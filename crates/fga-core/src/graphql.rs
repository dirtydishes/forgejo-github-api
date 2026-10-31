use crate::forgejo;
use async_graphql::{EmptyMutation, EmptySubscription, ErrorExtensions, Schema};
mod checks;
mod connections;
mod pulls;
mod query;
mod types;
use checks::*;
use connections::*;
use pulls::*;
pub use query::{Query, Repository, SearchResultItem};
pub use types::*;

pub type GithubSchema = Schema<Query, EmptyMutation, EmptySubscription>;

pub fn schema() -> GithubSchema {
    Schema::build(Query, EmptyMutation, EmptySubscription)
        .data(crate::map::WipPrefixes::default())
        .finish()
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
