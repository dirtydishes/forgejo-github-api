use axum::{Json, Router, extract::Query, http::HeaderMap, response::IntoResponse, routing::get};
use fga_core::forgejo::{Error, Forgejo};
use serde_json::{Value, json};
use std::collections::HashMap;

fn user() -> Value {
    json!({"id":1,"login":"e2e","full_name":"Test User","avatar_url":"https://forge.test/avatar/1"})
}
fn pr(number: i32, owner: &str, branch: &str) -> Value {
    json!({"number":number,"title":"test","state":"open","merged":false,"draft":false,"html_url":format!("https://forge.test/e2e/oracle/pulls/{number}"),"head":{"label":branch,"ref":branch,"sha":"abc","repo":{"owner":user_with_login(owner)}},"base":{"label":"main","ref":"main","sha":"def","repo":{"owner":user()}},"user":user(),"merged_at":null})
}
fn user_with_login(login: &str) -> Value {
    let mut value = user();
    value["login"] = json!(login);
    value
}

#[tokio::test]
async fn forwards_auth_and_filters_across_short_upstream_pages() {
    let upstream = Router::new()
        .route(
            "/api/v1/user",
            get(|headers: HeaderMap| async move {
                assert_eq!(headers["authorization"], "bearer disposable");
                Json(user())
            }),
        )
        .route(
            "/api/v1/repos/e2e/oracle/pulls",
            get(
                |headers: HeaderMap, Query(query): Query<HashMap<String, String>>| async move {
                    assert_eq!(headers["authorization"], "bearer disposable");
                    assert_eq!(query["state"], "all");
                    match query["page"].as_str() {
                        // A server can clamp limit below our requested 50; Link remains authoritative.
                        "1" => (
                            [("link", "<http://untrusted.invalid/steal>; rel=\"next\"")],
                            Json(json!([pr(4, "other", "s/pass"), null])),
                        )
                            .into_response(),
                        "2" => (
                            [("link", "<http://untrusted.invalid/steal>; rel=\"next\"")],
                            Json(json!([pr(3, "e2e", "s/other")])),
                        )
                            .into_response(),
                        "3" => Json(json!([pr(1, "E2E", "s/pass"), pr(2, "e2e", "s/pass")]))
                            .into_response(),
                        _ => panic!("unexpected page"),
                    }
                },
            ),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let forgejo = Forgejo::new(url).unwrap();
    assert!(matches!(
        forgejo.authenticate("basic invalid").await,
        Err(Error::Unauthorized)
    ));
    let session = forgejo.authenticate("bearer disposable").await.unwrap();
    let prs = session
        .pull_requests("e2e", "oracle", "all", Some("e2e:s/pass"))
        .await
        .unwrap();
    assert_eq!(prs.iter().map(|p| p.number).collect::<Vec<_>>(), [2, 1]);
    assert_eq!(prs[0].head.label(), "e2e:s/pass");
    task.abort();
}
