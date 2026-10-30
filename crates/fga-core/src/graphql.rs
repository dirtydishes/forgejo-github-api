use crate::{
    forgejo::{self, Session},
    ids::node_id,
};
use async_graphql::{
    Context, EmptyMutation, EmptySubscription, Enum, ErrorExtensions, ID, Object, Scalar,
    ScalarType, Schema, SimpleObject, Value,
};

pub type GithubSchema = Schema<Query, EmptyMutation, EmptySubscription>;

pub fn schema() -> GithubSchema {
    Schema::build(Query, EmptyMutation, EmptySubscription).finish()
}

pub fn error(error: forgejo::Error) -> async_graphql::Error {
    let kind = match error {
        forgejo::Error::Unauthorized => "UNAUTHORIZED",
        forgejo::Error::NotFound => "NOT_FOUND",
        forgejo::Error::Forbidden => "FORBIDDEN",
        forgejo::Error::Timeout | forgejo::Error::Upstream => "INTERNAL",
    };
    async_graphql::Error::new(error.to_string())
        .extend_with(|_, extensions| extensions.set("type", kind))
}

pub struct Query;

#[Object]
impl Query {
    async fn viewer(&self, ctx: &Context<'_>) -> User {
        User {
            login: ctx.data_unchecked::<Session>().viewer.login.clone(),
        }
    }

    async fn repository(
        &self,
        ctx: &Context<'_>,
        owner: String,
        name: String,
    ) -> async_graphql::Result<Option<Repository>> {
        let session = ctx.data_unchecked::<Session>();
        let repo = session.repository(&owner, &name).await.map_err(error)?;
        Ok(Some(Repository {
            owner: repo.owner.login,
            name: repo.name,
        }))
    }
}

#[derive(SimpleObject)]
pub struct User {
    pub login: String,
}

pub struct Repository {
    owner: String,
    name: String,
}

#[Object]
impl Repository {
    async fn pull_request(
        &self,
        ctx: &Context<'_>,
        number: i32,
    ) -> async_graphql::Result<Option<PullRequest>> {
        let pr = ctx
            .data_unchecked::<Session>()
            .pull_request(&self.owner, &self.name, number)
            .await
            .map_err(error)?;
        let state = if pr.merged {
            PullRequestState::Merged
        } else if pr.state == "closed" {
            PullRequestState::Closed
        } else {
            PullRequestState::Open
        };
        let url = url::Url::parse(&pr.html_url).map_err(|_| error(forgejo::Error::Upstream))?;
        Ok(Some(PullRequest {
            id: ID(node_id(
                "PullRequest",
                &format!("{}/{}", self.owner, self.name),
                pr.number,
            )),
            number: pr.number,
            title: pr.title,
            state,
            url: Uri(url),
            is_draft: pr.draft,
            head_ref_name: pr.head.ref_name,
            base_ref_name: pr.base.ref_name,
        }))
    }
}

#[derive(Clone, Copy, Eq, PartialEq, Enum)]
pub enum PullRequestState {
    Open,
    Closed,
    Merged,
}

#[derive(SimpleObject)]
pub struct PullRequest {
    pub id: ID,
    pub number: i32,
    pub title: String,
    pub state: PullRequestState,
    pub url: Uri,
    pub is_draft: bool,
    pub head_ref_name: String,
    pub base_ref_name: String,
}

/// GitHub calls this scalar URI, not String or URL.
pub struct Uri(url::Url);

#[Scalar(name = "URI")]
impl ScalarType for Uri {
    fn parse(value: Value) -> async_graphql::InputValueResult<Self> {
        match value {
            Value::String(value) => Ok(Self(url::Url::parse(&value)?)),
            other => Err(async_graphql::InputValueError::expected_type(other)),
        }
    }

    fn to_value(&self) -> Value {
        Value::String(self.0.to_string())
    }
}
