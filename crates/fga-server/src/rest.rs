use crate::{AppState, rest_error};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use fga_core::{forgejo::Session, map};
use serde::Deserialize;
use serde_json::json;

pub async fn root(State(state): State<AppState>) -> impl IntoResponse {
    let public = state.public_url.as_str().trim_end_matches('/');
    (
        [
            ("x-oauth-scopes", "repo, read:org, workflow"),
            ("x-github-media-type", "github.v3; format=json"),
        ],
        Json(
            json!({"current_user_url": format!("{public}/api/v3/user"), "repository_url": format!("{public}/api/v3/repos/{{owner}}/{{repo}}") }),
        ),
    )
}

pub async fn meta(State(state): State<AppState>) -> impl IntoResponse {
    Json(json!({"installed_version": state.ghes_version}))
}

pub async fn user(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
) -> impl IntoResponse {
    Json(map::user(&session.viewer, &state.public_url))
}

pub async fn repository(
    Extension(session): Extension<Session>,
    Path((owner, name)): Path<(String, String)>,
) -> Response {
    let result = async {
        let repo = session.repository(&owner, &name).await?;
        let owner_type = session.owner_type(&repo.owner.login).await?;
        Ok(Json(map::repository(&repo, owner_type)).into_response())
    }
    .await;
    result.unwrap_or_else(rest_error)
}

#[derive(Default, Deserialize)]
pub struct PullsQuery {
    head: Option<String>,
    state: Option<String>,
    per_page: Option<usize>,
    page: Option<usize>,
}

pub async fn pulls(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path((owner, name)): Path<(String, String)>,
    query: Result<Query<PullsQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let Ok(Query(query)) = query else {
        return invalid_query();
    };
    let filter = query.state.as_deref().unwrap_or("open");
    let per_page = query.per_page.unwrap_or(30).clamp(1, 100);
    let page = query.page.unwrap_or(1);
    if !matches!(filter, "open" | "closed" | "all") || page == 0 {
        return invalid_query();
    }
    let pulls = match session
        .pull_requests(&owner, &name, filter, query.head.as_deref())
        .await
    {
        Ok(pulls) => pulls,
        Err(error) => return rest_error(error),
    };
    let start = (page - 1).saturating_mul(per_page);
    let body: Vec<_> = pulls
        .iter()
        .skip(start)
        .take(per_page)
        .map(map::pull_request)
        .collect();
    let last = pulls.len().div_ceil(per_page).max(1);
    let mut headers = HeaderMap::new();
    if last > 1 {
        let mut url = state.public_url.clone();
        url.path_segments_mut()
            .expect("validated base URL")
            .pop_if_empty()
            .extend(["api", "v3", "repos", &owner, &name, "pulls"]);
        let mut links = Vec::new();
        for (rel, number) in [
            ("next", page.checked_add(1).filter(|_| page < last)),
            ("prev", page.checked_sub(1).filter(|n| *n > 0)),
            ("last", Some(last)),
            ("first", Some(1)),
        ] {
            if let Some(number) = number {
                url.set_query(None);
                url.query_pairs_mut()
                    .append_pair("state", filter)
                    .append_pair("per_page", &per_page.to_string())
                    .append_pair("page", &number.to_string());
                if let Some(head) = &query.head {
                    url.query_pairs_mut().append_pair("head", head);
                }
                links.push(format!("<{url}>; rel=\"{rel}\""));
            }
        }
        headers.insert(
            "link",
            links.join(", ").parse().expect("URL is a valid header"),
        );
    }
    (headers, Json(body)).into_response()
}

fn invalid_query() -> Response {
    (
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({"message": "Validation Failed"})),
    )
        .into_response()
}
