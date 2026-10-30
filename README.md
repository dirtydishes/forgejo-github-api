# forgejo-github-api

A GitHub-Enterprise-compatible API (`/api/v3` REST and `/api/graphql`) for Forgejo. Mount it on your
Forgejo hostname, and tools built for GitHub work against your forge: the `gh` CLI, Claude Code, Claude
desktop, and ChatGPT's coding agent. They treat it as a GitHub Enterprise host.

**Status:** design stage. See [docs/DESIGN.md](docs/DESIGN.md).

Related: [gh-forgejo-shim](https://github.com/dirtydishes/gh-forgejo-shim), a CLI-level shim for setups
where you can't add a route to the forge.
