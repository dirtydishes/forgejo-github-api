use reqwest::{Client, StatusCode, header::AUTHORIZATION};
use serde::{Deserialize, de::DeserializeOwned};
use std::time::Duration;
use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Bad credentials")]
    Unauthorized,
    #[error("Not Found")]
    NotFound,
    #[error("Resource not accessible by token")]
    Forbidden,
    #[error("Forgejo request timed out")]
    Timeout,
    #[error("Forgejo returned an invalid response")]
    Upstream,
}

#[derive(Clone)]
pub struct Forgejo {
    client: Client,
    base: Url,
}

/// A token lives only for the duration of its incoming request.
#[derive(Clone)]
pub struct Session {
    pub forgejo: Forgejo,
    authorization: String,
    pub viewer: User,
}

impl Forgejo {
    pub fn new(base: Url) -> Result<Self, reqwest::Error> {
        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            base,
        })
    }

    pub async fn authenticate(&self, authorization: &str) -> Result<Session, Error> {
        let (scheme, token) = authorization.split_once(' ').ok_or(Error::Unauthorized)?;
        if !(scheme.eq_ignore_ascii_case("token") || scheme.eq_ignore_ascii_case("bearer"))
            || token.is_empty()
            || token.chars().any(char::is_whitespace)
        {
            return Err(Error::Unauthorized);
        }
        let viewer = self.get(&["user"], &[], authorization).await?.0;
        Ok(Session {
            forgejo: self.clone(),
            authorization: authorization.to_owned(),
            viewer,
        })
    }

    async fn get<T: DeserializeOwned>(
        &self,
        segments: &[&str],
        query: &[(&str, String)],
        authorization: &str,
    ) -> Result<(T, bool), Error> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|_| Error::Upstream)?
            .pop_if_empty()
            .extend(["api", "v1"])
            .extend(segments);
        let response = self
            .client
            .get(url)
            .query(query)
            .header(AUTHORIZATION, authorization)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    Error::Timeout
                } else {
                    Error::Upstream
                }
            })?;
        match response.status() {
            StatusCode::UNAUTHORIZED => return Err(Error::Unauthorized),
            StatusCode::FORBIDDEN => return Err(Error::Forbidden),
            StatusCode::NOT_FOUND => return Err(Error::NotFound),
            status if !status.is_success() => return Err(Error::Upstream),
            _ => {}
        }
        // Follow only the page number, never an upstream-provided URL with our token.
        let has_next = response
            .headers()
            .get(reqwest::header::LINK)
            .and_then(|h| h.to_str().ok())
            .is_some_and(|h| h.split(',').any(|link| link.contains("rel=\"next\"")));
        let value = response.json().await.map_err(|_| Error::Upstream)?;
        Ok((value, has_next))
    }
}

impl Session {
    pub async fn repository(&self, owner: &str, name: &str) -> Result<Repository, Error> {
        Ok(self
            .forgejo
            .get(&["repos", owner, name], &[], &self.authorization)
            .await?
            .0)
    }

    /// Forgejo's repository owner is a User-shaped object even for organizations.
    pub async fn owner_type(&self, owner: &str) -> Result<&'static str, Error> {
        match self
            .forgejo
            .get::<serde_json::Value>(&["orgs", owner], &[], &self.authorization)
            .await
        {
            Ok(_) => Ok("Organization"),
            Err(Error::NotFound) => Ok("User"),
            Err(error) => Err(error),
        }
    }

    pub async fn pull_request(
        &self,
        owner: &str,
        name: &str,
        number: i32,
    ) -> Result<PullRequest, Error> {
        Ok(self
            .forgejo
            .get(
                &["repos", owner, name, "pulls", &number.to_string()],
                &[],
                &self.authorization,
            )
            .await?
            .0)
    }

    /// Filter after traversing upstream pages: a match may occur past the first page.
    /// GitHub pagination applies to the filtered set, not Forgejo's unfiltered pages.
    pub async fn pull_requests(
        &self,
        owner: &str,
        name: &str,
        state: &str,
        head: Option<&str>,
    ) -> Result<Vec<PullRequest>, Error> {
        let mut pulls = Vec::new();
        let mut page = 1_u64;
        loop {
            let (batch, has_next): (Vec<Option<PullRequest>>, _) = self
                .forgejo
                .get(
                    &["repos", owner, name, "pulls"],
                    &[
                        ("state", state.to_owned()),
                        ("limit", "50".into()),
                        ("page", page.to_string()),
                    ],
                    &self.authorization,
                )
                .await?;
            let count = batch.len();
            pulls.extend(batch.into_iter().flatten().filter(|pr| {
                let state_matches =
                    state == "all" || (if pr.merged { "closed" } else { &pr.state }) == state;
                state_matches && head.is_none_or(|filter| pr.head.matches(filter))
            }));
            if !has_next && count < 50 {
                break;
            }
            page += 1;
        }
        pulls.sort_by_key(|pr| std::cmp::Reverse(pr.number));
        Ok(pulls)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct User {
    pub id: i64,
    pub login: String,
    pub full_name: Option<String>,
    pub html_url: Option<String>,
    pub avatar_url: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Repository {
    pub id: i64,
    pub name: String,
    pub full_name: String,
    pub owner: User,
    pub private: bool,
    pub html_url: String,
    pub default_branch: String,
    pub fork: bool,
    pub archived: bool,
    pub permissions: Option<Permissions>,
}

#[derive(Clone, Debug, Deserialize, serde::Serialize)]
pub struct Permissions {
    pub admin: bool,
    pub push: bool,
    pub pull: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BranchRepository {
    pub owner: User,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Branch {
    pub label: String,
    #[serde(rename = "ref")]
    pub ref_name: String,
    pub sha: String,
    pub repo: Option<BranchRepository>,
}

impl Branch {
    pub fn label(&self) -> String {
        match &self.repo {
            Some(repo) => format!("{}:{}", repo.owner.login, self.ref_name),
            None => self.label.clone(),
        }
    }

    fn matches(&self, filter: &str) -> bool {
        match filter.split_once(':') {
            Some((owner, branch)) => {
                self.label()
                    .split_once(':')
                    .is_some_and(|(login, _)| login.eq_ignore_ascii_case(owner))
                    && self.ref_name == branch
            }
            None => self.ref_name == filter,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct PullRequest {
    pub number: i32,
    pub title: String,
    pub state: String,
    pub merged: bool,
    pub draft: bool,
    pub html_url: String,
    pub head: Branch,
    pub base: Branch,
    pub user: User,
    pub merged_at: Option<String>,
}
