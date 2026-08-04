---
name: test-full
description: "Run the merge-candidate gate selected for the exact PR diff, including links, non-test checks, test concerns, shipping build, and presentation applicability. Used by /audit-pr; stop on the first substantive failure."
---

# Candidate validation

This is the only pre-merge validation tier. Resolve and fetch the actual PR base, then
follow the complete **Before opening a PR** procedure in
[`CONTRIBUTING.md`](../../../CONTRIBUTING.md#before-opening-a-pr) exactly.

## Required sequence

1. Require the local commit to equal the pushed PR head when a PR exists.
2. Run the tracked Markdown relative-link check.
3. Run the fail-closed scope plan against the exact PR base and head, and record it.
4. For Rust-affecting work, run format and dependency policy. Run Clippy, selected
   test concerns, residual doctests, warnings-denied docs, graph/partition checks, and
   the default-feature shipping release build exactly when the plan selects them.
5. Never convert a green job shell or timeout into evidence for a concern that did not
   execute.
6. Classify presentation applicability using the evidence boundary in the PR template
   and gameplay/map testing contracts. If static presentation changed, invoke
   `/visual-walk`. If motion, native input, control feel, or taste changed, require the
   structured exact-head human route in the PR body as well. Logic-only work records
   the exact typed hook closure for `N/A`.

Do not run an unconditional workspace or renderer suite to fill a checklist. Do not
use frames or human observation to prove or corroborate gameplay/world state. Add a
narrow typed hook when the required logical oracle is missing.

## Result

Return the exact head/base, selector decision, each selected gate and concern,
visual applicability, and first failure or all-green result. When invoked by
`/audit-pr`, return structured findings as `{suite, test, message}`; use `build`,
`deny`, `docs`, or `links` as the suite for non-test failures.
