use base64::{Engine, engine::general_purpose::STANDARD};

/// Opaque, stable IDs; no local database or GitHub numeric IDs are involved.
pub fn node_id(kind: &str, repository: &str, id: impl std::fmt::Display) -> String {
    STANDARD.encode(format!("fga:{kind}:{repository}:{id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_includes_kind_repository_and_number() {
        assert_eq!(
            STANDARD
                .decode(node_id("PullRequest", "e2e/oracle", 1))
                .unwrap(),
            b"fga:PullRequest:e2e/oracle:1"
        );
        assert_ne!(
            node_id("PullRequest", "a/r", 1),
            node_id("PullRequest", "b/r", 1)
        );
    }
}
