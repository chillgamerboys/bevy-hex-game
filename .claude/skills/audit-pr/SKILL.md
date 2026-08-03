---
name: audit-pr
description: "Run the exact-head merge audit for the current PR: soft Linear reconciliation, diff review, candidate validation, applicable visual review, and structured runtime evidence. Writes the receipt consumed by /merge-pr and never edits the candidate."
---

# Audit a pull request

This is the ready-to-merge gate. It is read-only except for its temporary receipt.

## Preflight

1. Require an open PR for the current branch and fetch its number, title, body, base,
   head branch, and `headRefOid` with `gh pr view`.
2. Fetch the base. Require local `HEAD` to equal the PR head exactly and require a clean
   worktree. A local-only or unpushed commit invalidates the audit.
3. Resolve the receipt path as `/tmp/audit-pr-receipt-<PR>.json`. Write a failed
   receipt if any later required phase fails.

## Linear reconciliation (soft)

Search the branch and PR body for `HEX-N`. When a Linear connector is available,
fetch the issue by identifier and confirm it belongs to the Hex Game team. Compare its
acceptance scope and state with the PR and
[delivery-state.md](../../../docs/development/delivery-state.md).

- Valid link: record the identifier, URL, and current state.
- No link or unavailable connector: record a visible warning and continue.
- Stale, duplicate, or already-terminal scope: record the mismatch for follow-up.

Linear is never a merge gate. Do not create a ticket here. New UI observations belong
through the repository's canonical `linear-ui-bug-intake` workflow; existing issue
linkage or state correction belongs to `/update-linear`.

## Required phases

Run in order and stop after the first failure:

1. `/audit-diff` — all applicable lenses; any ship blocker fails the audit.
2. `/test-full` — the selector-chosen candidate gate from `CONTRIBUTING.md`, including
   one `/visual-walk` invocation when static presentation is affected.
3. Validate the structured `Manual runtime sign-off` fields already present in the PR
   template. Presentation/experience work requires a named-human `PASS`; logic-only
   work requires a verified-maintainer `N/A` naming the typed hook closure. The full
   recorded SHA must equal the PR head. `BLOCKED`, placeholders, stale SHAs, or agent
   self-signoff fail the gate.

Do not rerun visual review, run a second validation tier, mutate documentation, append
audit records, or silently downgrade a failed check. The template and the gameplay/map
testing contracts own the detailed evidence policy.

## Receipt contract

Write schema 4 JSON after every audit attempt:

```json
{
  "schema_version": 4,
  "pr_number": 179,
  "head_sha": "40-character sha",
  "base_branch": "dev",
  "completed_at": "ISO-8601 UTC",
  "overall_status": "green",
  "linear": {
    "status": "linked",
    "issue": "HEX-42",
    "summary": "In Progress; scope agrees"
  },
  "review": {
    "status": "pass",
    "findings": []
  },
  "validation": {
    "status": "pass",
    "summary": "selector-chosen candidate gate passed",
    "visual": "pass",
    "manual_runtime": "PASS"
  }
}
```

Allowed `overall_status`, review status, and validation status values are `green` or
`failed` / `pass` or `fail` as shown. `linear.status` is `linked`, `unlinked`, or
`unavailable` and never changes `overall_status`. `validation.visual` is `pass` or
`not_applicable`; `manual_runtime` is `PASS` or `N/A` only on green receipts.

Capture the local SHA after all checks; because this skill does not edit the candidate,
it must still equal the PR head. If receipt writing fails, report that loudly:
`/merge-pr` must refuse a missing or malformed receipt.

## Report

Report the exact PR/head/base, Linear warning or link, review findings, selected
concerns and non-test gates, visual classification, structured runtime classification,
and receipt path. Never describe an unselected concern as executed or passed.
