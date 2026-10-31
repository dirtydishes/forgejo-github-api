use super::*;
use crate::{
    forgejo::{self, Session},
    ids,
};
use async_graphql::{Context, Enum, ID, InputObject, Interface, Object, Union};

pub struct Query;
#[Object]
impl Query {
    async fn viewer(&self, ctx: &Context<'_>) -> User {
        User(ctx.data_unchecked::<Session>().viewer.clone())
    }
    async fn repository(
        &self,
        ctx: &Context<'_>,
        owner: String,
        name: String,
    ) -> async_graphql::Result<Option<Repository>> {
        let source = ctx
            .data_unchecked::<Session>()
            .repository(&owner, &name)
            .await
            .map_err(error)?;
        Ok(Some(Repository { source }))
    }
    async fn node(&self, ctx: &Context<'_>, id: ID) -> async_graphql::Result<Option<Node>> {
        let Some(id) = ids::decode(&id).filter(|id| id.kind == "PullRequest") else {
            return Ok(None);
        };
        match ctx
            .data_unchecked::<Session>()
            .pull_request(&id.owner, &id.repository, id.number)
            .await
        {
            Ok(source) => Ok(Some(
                PullRequest::new(source, &id.owner, &id.repository).into(),
            )),
            Err(forgejo::Error::NotFound) => Ok(None),
            Err(e) => Err(error(e)),
        }
    }
    /// Only the repo-scoped author/review-request searches used by gh pr status.
    async fn search(
        &self,
        ctx: &Context<'_>,
        query: String,
        r#type: SearchType,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<SearchResultItemConnection> {
        if r#type != SearchType::Issue {
            return Err("only PR status searches are supported".into());
        }
        let mut repository = None;
        let mut author = None;
        let mut reviewer = None;
        let mut open = false;
        for term in query.split_whitespace() {
            if let Some(value) = term.strip_prefix("repo:") {
                repository = value.split_once('/');
            } else if let Some(value) = term.strip_prefix("author:") {
                author = Some(value);
            } else if let Some(value) = term.strip_prefix("review-requested:") {
                reviewer = Some(value);
            } else if term == "state:open" {
                open = true;
            } else if term != "is:pr" {
                return Err("unsupported PR status search qualifier".into());
            }
        }
        let (owner, repo) = repository.ok_or("PR status search requires repo qualifier")?;
        if !open || author.is_none() == reviewer.is_none() {
            return Err("unsupported PR status search".into());
        }
        let pulls = ctx
            .data_unchecked::<Session>()
            .pull_requests(owner, repo, "open", None)
            .await
            .map_err(error)?;
        let nodes = pulls
            .into_iter()
            .filter(|pr| {
                author.is_none_or(|login| pr.user.login.eq_ignore_ascii_case(login))
                    && reviewer.is_none_or(|login| {
                        pr.requested_reviewers.as_ref().is_some_and(|users| {
                            users.iter().any(|u| u.login.eq_ignore_ascii_case(login))
                        })
                    })
            })
            .map(|source| PullRequest::new(source, owner, repo))
            .collect();
        let p = page(nodes, first, None, after, None)?;
        Ok(SearchResultItemConnection {
            issue_count: p.total,
            page_info: p.info,
            edges: Some(
                p.nodes
                    .into_iter()
                    .map(|pr| {
                        Some(SearchResultItemEdge {
                            node: Some(pr.into()),
                        })
                    })
                    .collect(),
            ),
        })
    }
}
#[derive(Interface)]
#[graphql(field(name = "id", ty = "ID"))]
pub enum Node {
    PullRequest(PullRequest),
}
#[derive(Union)]
pub enum SearchResultItem {
    PullRequest(PullRequest),
}
#[derive(Clone, Copy, Eq, PartialEq, Enum)]
pub enum SearchType {
    Discussion,
    Issue,
    Repository,
    User,
}
#[derive(Clone, Copy, Eq, PartialEq, Enum)]
pub enum OrderDirection {
    Asc,
    Desc,
}
#[derive(Clone, Copy, Eq, PartialEq, Enum)]
pub enum IssueOrderField {
    Comments,
    CreatedAt,
    UpdatedAt,
}
#[derive(InputObject)]
pub struct IssueOrder {
    pub field: IssueOrderField,
    pub direction: OrderDirection,
}

pub struct Repository {
    pub source: forgejo::Repository,
}
#[Object]
impl Repository {
    async fn id(&self) -> ID {
        ID(ids::node_id(
            "Repository",
            &self.source.full_name,
            self.source.id,
        ))
    }
    async fn name(&self) -> &str {
        &self.source.name
    }
    async fn name_with_owner(&self) -> &str {
        &self.source.full_name
    }
    async fn url(&self) -> Uri {
        Uri(self.source.html_url.clone())
    }
    async fn owner(&self, ctx: &Context<'_>) -> async_graphql::Result<RepositoryOwner> {
        let kind = ctx
            .data_unchecked::<Session>()
            .owner_type(&self.source.owner.login)
            .await
            .map_err(error)?;
        Ok(if kind == "Organization" {
            Organization::from(self.source.owner.clone()).into()
        } else {
            User(self.source.owner.clone()).into()
        })
    }
    async fn default_branch_ref(&self) -> Option<Ref> {
        Some(Ref {
            name: self.source.default_branch.clone(),
            branch_protection_rule: None,
        })
    }
    async fn pull_request(
        &self,
        ctx: &Context<'_>,
        number: i32,
    ) -> async_graphql::Result<Option<PullRequest>> {
        let source = ctx
            .data_unchecked::<Session>()
            .pull_request(&self.source.owner.login, &self.source.name, number)
            .await
            .map_err(error)?;
        Ok(Some(PullRequest::new(
            source,
            &self.source.owner.login,
            &self.source.name,
        )))
    }
    async fn pull_requests(
        &self,
        ctx: &Context<'_>,
        head_ref_name: Option<String>,
        first: Option<i32>,
        after: Option<String>,
        order_by: Option<IssueOrder>,
    ) -> async_graphql::Result<PullRequestConnection> {
        if order_by.as_ref().is_some_and(|o| {
            o.field != IssueOrderField::CreatedAt || o.direction != OrderDirection::Desc
        }) {
            return Err("only CREATED_AT DESC ordering is supported".into());
        }
        let owner = &self.source.owner.login;
        let pulls = ctx
            .data_unchecked::<Session>()
            .pull_requests(owner, &self.source.name, "all", head_ref_name.as_deref())
            .await
            .map_err(error)?;
        Ok(page(
            pulls
                .into_iter()
                .map(|source| PullRequest::new(source, owner, &self.source.name))
                .collect(),
            first,
            None,
            after,
            None,
        )?
        .into())
    }
}
