---
name: merge-pr
description: "Merge an exact audited PR head with a merge commit, verify the resulting base SHA, safely remove only an unused remote feature/wave branch, and reconcile an existing Linear issue when its full scope landed."
---

# Merge a pull request

This command is destructive. Invoke it only after the user has authorized the merge.
It never switches branches or rewrites history.

## Exact-head gate

Fetch the PR's number, state, draft flag, base, head branch, head SHA, mergeability,
review decision, and checks. Allowed shapes are feature→`dev`, source→`wave/*`,
wave→`dev`, and `dev`→`main` only through `/promote`.

Read `/tmp/audit-pr-receipt-<PR>.json` and require all of the following:

- valid JSON with `schema_version: 4` and `overall_status: green`;
- matching PR number, full head SHA, and base branch;
- review and validation status `pass`;
- visual value `pass` or `not_applicable`;
- manual runtime value `PASS` or `N/A`;
- local `HEAD`, the pushed PR head, and the receipt head are identical; and
- the worktree is clean.

Missing, malformed, failed, or SHA/base-mismatched evidence is a hard stop. Re-run
`/audit-pr`; do not reconstruct a receipt by hand.

Also require the PR open and non-draft, mergeable, required review satisfied, and all
required checks complete and green for that exact SHA. Infrastructure timeouts remain
failures unless the repository's documented maintainer-waiver process explicitly
applies; no warning path may bypass a substantive failure.

## Merge and verify

Capture the pre-merge `origin/<base>` SHA, then merge with exact-head protection:

```sh
gh pr merge <PR> --merge --match-head-commit <full-head-sha>
```

Do not squash, rebase, force-push, or enable auto-merge as a substitute for waiting.
Fetch the base and PR again. Require state `MERGED`, a recorded merge commit, the
audited head to be an ancestor of `origin/<base>`, and the fetched base to contain the
merge result. Report the pre/post base SHAs and merge SHA. A wave landing additionally
requires the combined head rather than any source-lane receipt.

## Linear and cleanup

Follow [delivery-state.md](../../../docs/development/delivery-state.md). If one linked
issue's complete promised outcome has now landed on `dev`, invoke `/update-linear` to
verify and move it to the live `Done` equivalent. A source PR entering a wave, a
partial epic, or a symptom-level fix remains non-terminal. Linear failure is reported
but never changes a valid merge result.

Before deleting the remote head, verify it is neither `dev` nor `main`, is not a base
for any open PR, and is not a still-needed source lane. Then delete only that explicit
remote ref and fetch with prune. Never delete or switch the local Conductor branch;
its upstream may show `[gone]` until the workspace is retired.

For an already-merged PR, perform verification, reconciliation, and safe cleanup
idempotently without issuing a second merge.

## Report

Report PR URL, audited head, merge SHA, resulting exact base SHA, check/runtime
evidence, remote-branch cleanup, post-merge CI status, and Linear changes or precise
recommendations left unapplied.
