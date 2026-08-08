---
name: reconcile-delivery-state
description: Reconcile shipped implementation, repository status/design/roadmap documents, GitHub PR state, and—when available—Linear HEX tickets. Use before calling a PR or wave complete, after landing work on dev, when planning from existing tickets, or when those delivery projections may disagree. Do not use for initial UI bug capture; use linear-ui-bug-intake. Linear is strongly recommended but remains a soft coordination signal, never a merge gate.
---

# Reconcile Delivery State

Treat code, repository documents, GitHub, and Linear as projections of one delivery.
Verify the projections that are available; do not infer one from another.

## Read the delivery contract

Read `AGENTS.md`, `CLAUDE.md`, and
`docs/development/delivery-state.md`. Read the affected status, roadmap, design,
contract, boundary, and system docs. Fetch current `origin/dev` and inspect the actual
PR/merge state.

Strongly prefer connected Linear access. If it is unavailable, complete the
repository/GitHub reconciliation, emit a visible soft warning, and provide the exact
tickets and recommended updates for someone who uses Linear. Never block an otherwise
valid merge solely because Linear is unavailable or unused by an owner.

## 1. Build an evidence ledger

Identify the outcomes in scope from the diff, PR body, wave manifest, roadmap markers,
and any `HEX-N` references. For each outcome record:

- implementation and tests that prove what is live;
- current PR/merge SHA and target branch;
- `status.md`, roadmap, design, contract, and system-doc claims;
- Linear title, description, state, and relations when accessible; and
- residual work that remains after this delivery.

For roadmap, architecture, or broad hygiene work, also inspect every non-completed Hex
Game ticket when Linear is available. This catches old tickets whose implementation
landed through a different PR or wave. If Linear is unavailable, say that this
workspace-wide check was skipped rather than pretending the named branch tickets were
the complete backlog.

Use Linear's live team/status queries instead of copying workflow-state IDs into this
skill. Re-fetch an issue before modifying it.

Initial defect capture is outside this skill. Route UI observations through
`$linear-ui-bug-intake`; reconcile them here only when implementation begins, enters
review, lands, becomes obsolete, or changes residual scope.

## 2. Classify each outcome

Choose exactly one:

- **Delivered:** the promised behavior is live on `dev`.
- **Partial:** a coherent portion is live, but named acceptance work remains.
- **Planned:** no implementation is live and the current scope is accurate.
- **Obsolete:** superseded, duplicate, administrative, or no longer intended.

PR state is evidence, not the classification. A merged leaf does not complete a
partial epic, and an open ticket does not prove its behavior is absent.

## 3. Correct repository documents

- `docs/planning/status.md` states only what is live and names limitations.
- `docs/planning/roadmap.md` keeps unfinished outcomes in Upcoming and records
  delivered outcomes without duplicate active rows.
- `docs/design/game.md` separates current provisional rules from open decisions.
- `docs/contracts.md` and `docs/planning/boundary.md` reflect actual cross-owner
  agreement and publication state.
- System docs describe the production path rather than a removed placeholder.

Do not weaken acceptance criteria to make old evidence appear sufficient. If code and
docs disagree, inspect runtime/tests and fix the false projection.

## 4. Advise or correct Linear

Mutate Linear only when access exists and the user authorized ticket reconciliation.

- **Delivered:** recommend/set Done or delete under the repository's free-workspace
  retention policy, and replace stale claims that the baseline is missing.
- **Partial:** recommend/retain In Review or Backlog, rewrite the description around
  exact residual scope, and link the delivery PR/SHA.
- **Planned:** retain Backlog/Todo and remove obsolete blockers or shipped subclaims.
- **Obsolete:** recommend/set Canceled or Duplicate with the reason.

Do not create tickets for incidental chores merely to satisfy traceability. Do not retire a
ticket when any acceptance item in its current scope remains. Before deleting, require the
durable identifier/title/outcome/dev-SHA/PR record and every safety condition in
`delivery-state.md`; re-fetch immediately before the mutation and verify it afterward. If
the owner does not use Linear, put the recommended changes in the PR/handoff instead.

## 5. Verify available projections

Validate repository edits with `git diff --check`, Markdown-link checks, and the
repository's changed-path selector. When Linear was modified, re-fetch every retained issue
and assert its returned state, title, and residual description; verify deleted issues
through the connector's returned deleted/recently-deleted state or an immediate explicit
not-found response. A timeout or connector error is not deletion evidence.

Perform a contradiction pass:

- no accessible active ticket says a documented delivered baseline is absent;
- no accessible Done ticket still contains unshipped acceptance work;
- no roadmap Upcoming row is already fully delivered;
- no status/design claim exceeds executable evidence; and
- every cross-owner blocker names the live agreed contract.

Before merge, include doc corrections in the candidate. After the merge reaches
`dev`, strongly recommend the terminal Linear update, but do not make it a merge
condition.

## Report

Return:

- PR and merge SHA or the exact blocker;
- tickets corrected, deleted, or recommended for correction/deletion;
- documents corrected;
- explicit residual work and owner; and
- a soft warning naming any Linear state that could not be inspected.
