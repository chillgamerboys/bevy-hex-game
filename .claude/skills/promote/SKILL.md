---
name: promote
description: "Open and land the deliberate dev-to-main promotion after an exact-dev human playtest. Uses the normal audit receipt and merge-commit gate, never switches branches, and never deletes dev."
---

# Promote dev to main

Promotion is an explicit release-readiness decision, not routine feature delivery.

1. Fetch `origin/dev` and `origin/main`. Require a non-empty range, no unexpected
   ancestry problem, and no unresolved `dev` checks.
2. Require a named human to run the shipping build at the exact current `origin/dev`
   SHA and record date, platform, route, outcome, and findings. A screenshot-only or
   agent-only check is insufficient; promotion always includes experience, not a
   logic-only `N/A`.
3. In Conductor, never checkout, switch, create, or rename a branch. The workspace
   used for local validation must already be at the exact `origin/dev` commit. If not,
   stop and ask the user to use an appropriate workspace.
4. Open or update one PR with `--head dev --base main`, using the canonical
   [PR template](../../../.github/pull_request_template.md). Summarize the included
   merge commits and record the exact-dev human evidence. Do not bind one feature
   ticket to this aggregate promotion.
5. Run `/audit-pr` for that PR number at the exact `dev` head, then `/merge-pr` only
   after explicit user authorization. Promotion uses a merge commit and never deletes
   `dev`.
6. Fetch and verify `origin/main` contains the audited `dev` head and report the merge
   SHA. Reconcile repository/GitHub/Linear projections without treating promotion as
   completion evidence for unrelated residual work.

Do not tag or publish a release automatically. `/release` is a separate explicit step.
