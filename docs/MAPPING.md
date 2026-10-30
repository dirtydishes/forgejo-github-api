# Field mapping: Forgejo → GitHub (A3)

Status: v0, 2026-10-30 (Opus). The gateway (`fga-core::map`) and gh-forgejo-shim share this spec.

**Sources:**
- GitHub's public GraphQL schema (`schema.docs.graphql`, fetched 2026-10-30).
- `cli/cli` at `v2.103.3`.
- GitHub REST docs.
- Forgejo 15 swagger (`/swagger.v1.json`).

Rules marked **decision** are ours, because GitHub doesn't document the behavior. Each can be revisited
when a live comparison against the GitHub twin disagrees.

## What clients actually read

| Field | gh | Claude | ChatGPT |
|---|---|---|---|
| `statusCheckRollup.contexts` (per check) | yes: `pr checks`, `pr view`, `pr status` | REST check data (TBD) | yes |
| `statusCheckRollup.state` (rollup) | **no**, never read | likely | yes |
| `mergeStateStatus` | `pr merge` (blocks on DIRTY/BLOCKED/BEHIND; merges now only on CLEAN/HAS_HOOKS/UNSTABLE), `pr status` (BEHIND) | ? | yes (merge button) |
| `mergeable` | `pr status` (UNKNOWN → "status unknown"), `pr update-branch` | ? | yes |
| `reviewDecision` | `pr status`, `pr view` labels | ? | yes |

**gh always sends introspection** (`__type(name:"PullRequest")`, `StatusCheckRollupContextConnection`,
`WorkflowRun`) and picks fields from the answer. The schema subset therefore has to be *honest*: list
only the fields we serve.

## 1. `PullRequest.state`, `isDraft`, `closed`, `merged`

| GitHub | Forgejo |
|---|---|
| `MERGED` | `merged == true` |
| `CLOSED` | `state == "closed" && !merged` |
| `OPEN` | `state == "open"` |
| `isDraft` | `draft` (Forgejo derives it from the WIP title prefix) |
| REST `state` | `open`/`closed`. Merged PRs are `closed`, with `merged: true` and `merged_at` set |

## 2. `mergeable`

| GitHub | Forgejo |
|---|---|
| `MERGEABLE` | open, and `mergeable == true` |
| `CONFLICTING` | open, `mergeable == false`, and the PR's status reports a conflict |
| `UNKNOWN` | Forgejo is still checking, or the PR is closed or merged |

- **Decision:** gh never retries, so the gateway doesn't return UNKNOWN on its own initiative. If
  Forgejo reports "checking", it re-reads once after about 300 ms within the request deadline, then gives
  up and returns UNKNOWN.
- **To verify in G2:** whether Forgejo's API distinguishes "checking" from "conflict" when
  `mergeable == false`. The swagger PR object has only the boolean, so it may need the
  `pulls/{n}.json` web endpoint or the PR's `status` field, if Forgejo returns one undocumented.

## 3. Check contexts (`statusCheckRollup.contexts`)

Forgejo has commit statuses only (`GET repos/{o}/{r}/commits/{sha}/statuses`, states `pending`,
`success`, `error`, `failure`, `warning`). Forgejo Actions jobs report through them, with context
`"<workflow> / <job> (<event>)"` and `target_url` pointing to the Actions run.

- **Decision: Actions statuses become `CheckRun`; everything else becomes `StatusContext`.** gh, ChatGPT
  and Claude render CheckRuns with workflow and event information. Identify an Actions status by its
  `target_url` pointing to `/{o}/{r}/actions/runs/` and its context matching `^(.+) / (.+) \((.+)\)$`.
  Then:
  - `name` ← job, `checkSuite.workflowRun.workflow.name` ← workflow, `.event` ← event.
  - `detailsUrl` ← `target_url`. Forgejo returns it **relative** (`/o/r/actions/runs/5/jobs/1`), so prefix the forge's public web base. `startedAt` ← `created_at`, `completedAt` ← `updated_at` when terminal.
  - `status`/`conclusion`:

    | Forgejo | `status` | `conclusion` |
    |---|---|---|
    | pending, description "Waiting to run" | `QUEUED` | null |
    | pending, other descriptions ("Has started running") | `IN_PROGRESS` | null |
    | success | `COMPLETED` | `SUCCESS` |
    | failure | `COMPLETED` | `FAILURE` |
    | error | `COMPLETED` | `FAILURE` (gh: fail) |
    | warning | `COMPLETED` | `NEUTRAL` (gh: skipping, i.e. not a failure and not pending) |

    Skipped Actions jobs report as `success` with a description such as "Has been skipped". If the
    description says it was skipped, return `SKIPPED`.
- **Non-Actions statuses** become `StatusContext {context, state, targetUrl, description, createdAt}`,
  with `state` mapped pending→PENDING, success→SUCCESS, error→ERROR, failure→FAILURE and
  warning→**SUCCESS**. The last is a decision: StatusState has no "warning", and gh would bucket an
  unknown value as pending, which means exit 8 forever.
- **Latest per context only.** Forgejo returns status history, so keep the newest `updated_at` per
  `context`. gh dedupes too, but the rollup state must not count stale entries.
- **`isRequired(pullRequestId:)`** is true when the base branch's protection has `enable_status_check`
  and the context matches one of `status_check_contexts` (glob).
- **Rollup `state`:** apply GitHub's documented combined-status rule to the mapped contexts:
  - any FAILURE or ERROR (and CheckRun conclusions FAILURE, TIMED_OUT, CANCELLED, ACTION_REQUIRED) → `FAILURE`;
  - else any pending (CheckRun not COMPLETED, StatusContext PENDING) → `PENDING`;
  - else, if there are contexts → `SUCCESS`;
  - **no contexts → `statusCheckRollup: null`**. gh then prints "no checks reported" (exit 1), matching
    the GitHub oracle for `s/nochecks`.

## 4. `reviewDecision`

**Decision:** null unless the base branch protection has `required_approvals > 0` or
`block_on_rejected_reviews`. This matches the commonly observed GitHub behavior, which is undocumented.
When protection applies:

1. Take the reviews (`pulls/{n}/reviews`), drop `dismissed`, drop `stale` if `dismiss_stale_approvals`
   is set, and drop state COMMENT and PENDING. Keep the **latest per user**.
2. If any is REQUEST_CHANGES and `block_on_rejected_reviews` is set (or `official` is true) → `CHANGES_REQUESTED`.
3. Else, if the count of APPROVED (official only, when `enable_approvals_whitelist` is set) ≥
   `required_approvals` → `APPROVED`.
4. Else → `REVIEW_REQUIRED`.

`latestReviews` is the latest non-pending review per user, in GitHub's `PullRequestReview` shape:
`state` maps APPROVED → APPROVED, REQUEST_CHANGES → CHANGES_REQUESTED, COMMENT → COMMENTED, and dismissed
reviews → DISMISSED.

## 5. `mergeStateStatus`

**Decision:** evaluate in this order and return the first that applies (open PRs only; closed or
merged → `UNKNOWN`):

1. `mergeable == UNKNOWN` → `UNKNOWN`.
2. `mergeable == CONFLICTING` → `DIRTY`.
3. `isDraft` → `DRAFT`. The value is deprecated but still in the schema, and gh doesn't block on it
   client-side. Forgejo refuses to merge WIP PRs, so the server's error surfaces as GitHub's would.
4. Protection `block_on_outdated_branch` and the head doesn't contain the base → `BEHIND`. Compute with
   `GET repos/{o}/{r}/compare/{base}...{head}` and check that `behind_by > 0`; skip the check if the
   endpoint is missing.
5. Protection requirements unmet → `BLOCKED`. That means either `reviewDecision` ∈
   {REVIEW_REQUIRED, CHANGES_REQUESTED}, or required status contexts that are missing, pending or
   failing.
6. Any context non-passing (rollup ≠ SUCCESS, and not null) → `UNSTABLE`.
7. Otherwise → `CLEAN`.

Never emit `HAS_HOOKS`. `isMergeQueueEnabled` is always false. `viewerCanMergeAsAdmin` ←
`apply_to_admins == false` and the viewer is a repo admin (G3).

## 6. Merge and auto-merge (G3)

| gh / GitHub | Forgejo |
|---|---|
| `mergePullRequest(pullRequestId, mergeMethod, commitHeadline, commitBody, expectedHeadOid)` | `POST pulls/{n}/merge {Do: merge\|squash\|rebase, MergeTitleField, MergeMessageField, head_commit_id}` |
| head mismatch | Forgejo returns 409. The gateway returns the GraphQL error `Head branch was modified. Review and try the merge again.` with `type: "UNPROCESSABLE"`. The exact GitHub text is unconfirmed: capture it from the GitHub twin in G3 |
| `enablePullRequestAutoMerge` | `POST pulls/{n}/merge {..., merge_when_checks_succeed: true}` |
| `disablePullRequestAutoMerge` | `DELETE pulls/{n}/merge` (cancel scheduled merge) |
| `autoMergeRequest` | **Open:** Forgejo's API has no "get scheduled merge". Candidates are the PR timeline (scheduled-merge comment type) or the issue comments API. Until verified, return `null`. G3 must solve this, because ChatGPT shows auto-merge state |
| REST `PUT pulls/{n}/merge` | The same mapping (not needed by any captured client yet) |

## 7. IDs

Node IDs are opaque base64 strings of `fga:<Type>:<owner>/<repo>:<number|id>`, per DESIGN.md.
`databaseId` holds the Forgejo numeric ID. `fullDatabaseId` is the same value as a string.

## Open items to verify against the live twin (G2/G3)

1. **Forgejo "checking" vs "conflict"** for `mergeable == false`.
2. ~~Actions status context format~~: **verified 2026-10-30** on the twin, for example `ci / lint (push)`.
   The history keeps several rows per context, and `target_url` is relative
   (`/dirtydishes/gfj-oracle/actions/runs/5/jobs/1`).
3. **Skipped-job reporting** in Forgejo Actions.
4. **Scheduled auto-merge discoverability** (`autoMergeRequest`).
5. **GitHub's exact head-mismatch error text** (capture from the GitHub twin).
6. **`reviewDecision` null rule**, compared against the GitHub twin with and without branch protection.
