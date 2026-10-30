# forgejo-github-api

A stateless gateway that serves GitHub Enterprise API routes for Forgejo. It forwards each client's
Forgejo token to the upstream server and maps Forgejo responses to GitHub's JSON and GraphQL types.

G1 supports `gh auth login`, `gh api user`, and
`gh pr view N --json number,title,state,url,isDraft,headRefName,baseRefName` with gh 2.103.3.
It also supports the repository and branch-to-PR REST lookups Claude Code uses.
See [the design](docs/DESIGN.md) for later milestones.

Run with Rust 1.97.1:

```sh
FGA_FORGEJO_URL=http://127.0.0.1:3000 \
FGA_PUBLIC_URL=https://forge.example.com \
cargo run --locked -p fga-server
```

Mount `/api/v3/*` and `/api/graphql` on the Forgejo hostname through your reverse proxy. The gateway
listens on `127.0.0.1:8080` by default. Set `FGA_LISTEN` to change it. `FGA_GHES_VERSION` defaults
to `3.19.0`. For direct HTTPS, set both `FGA_TLS_CERT` and `FGA_TLS_KEY` to PEM file paths.
The upstream and public URLs must use HTTP or HTTPS and contain no credentials, query or fragment.
Upstream requests have a ten-second deadline and do not follow redirects.

Use a Forgejo token with `read:user`, `write:repository`, `write:issue`, and `read:organization`.
The gateway accepts `token` and `bearer` authorization schemes. It advertises the GitHub scopes
`repo, read:org, workflow` for gh login; Forgejo enforces the token's actual permissions.

The GraphQL subset contains `Query.viewer`, `Query.repository`, `Repository.pullRequest`,
`User.login`, and the eight core PR fields including the `id` gh adds. It supports schema
introspection and rejects unknown fields. It does not implement mutations or checks and reviews.
The [trimmed GitHub SDL](fixtures/github-g1.graphql) pins the source commit and checksum;
its contract test compares type kinds, field types, nullability, arguments and enum values.
The full schema download remains ignored under `.local/`.

The pull-list route filters `head=owner:branch` after reading Forgejo's pages, then applies GitHub
`page` and `per_page` pagination and emits `Link` headers. It compares owner names without case and
branch names with case. This first version reads all upstream pages per lookup, which costs more
requests on repositories with many PRs. It emits Forgejo's real web URLs, including `/pulls/N`.
Merged PRs have GraphQL state `MERGED`, REST state `closed`, and their upstream `merged_at`.

Run the gates from this checkout:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
scripts/e2e.sh
```

The e2e script requires Docker, starts Forgejo 15 with SQLite, creates a disposable account and token,
and seeds a private repository with normal and draft PRs. It builds a test-only image with the
checksum-verified official gh 2.103.3 Linux release for the container architecture. A self-signed
certificate lets gh use the gateway as `https://forge.test`. No host gh config or credentials enter
the containers. Results and gh's query capture go under `.local/e2e/`; the script removes its
containers, network, and certificate volume on exit. CI runs the same gates.

Related: [gh-forgejo-shim](https://github.com/dirtydishes/gh-forgejo-shim), a CLI alternative for
hosts where you cannot add gateway routes.
