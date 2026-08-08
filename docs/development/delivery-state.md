# Delivery-state reconciliation

Implementation, repository documents, GitHub, and Linear describe one delivery from
different angles. They should agree, but Linear remains a strongly recommended
coordination tool rather than a merge gate: an owner who does not use it must not be
blocked from contributing.

## Authorities

| Projection | Owns |
|---|---|
| Code and executable tests | What behavior actually exists |
| `docs/planning/status.md` | Current implementation and known limitations |
| `docs/planning/roadmap.md` | Upcoming and delivered outcomes |
| `docs/design/game.md` | Current provisional rules and unresolved product decisions |
| Contracts, boundary, and system docs | Cross-owner facts and production paths |
| GitHub | Review unit, exact head, checks, and merge provenance |
| Linear, when used | Work state, residual scope, owner, and product-facing history |

A PR merge does not automatically prove an epic complete. Conversely, an old open
ticket does not prove its promised behavior is still absent. Reconcile against
executable evidence.

## Planning gate

Before planning from existing work:

1. fetch current `origin/dev`;
2. read referenced `HEX-N` tickets in full when Linear is connected;
3. compare their descriptions and blockers with status, roadmap, design, and
   contracts;
4. inspect the code/tests for claims that may already have landed; and
5. during broad roadmap or architecture work, list all non-completed Hex Game tickets
   when available.

Correct stale state before using it to define a new branch. Administrative onboarding
issues, duplicates, and superseded tickets should not remain mixed with product work.
If Linear is unavailable, continue from repository and GitHub evidence, emit a soft
warning, and name the recommended ticket changes in the handoff.

## Delivery gate

Before a feature PR or wave is declared ready:

- current implementation status is documented honestly;
- roadmap rows and available ticket descriptions name only residual work;
- design candidates are not presented as shipped policy;
- contract status matches the publishing and consuming code;
- the PR names relevant tickets when they exist; and
- partial epics remain open even when a leaf PR merges.

Documentation corrections belong in the candidate. A connected Linear workflow
should retire a ticket only after its complete promised outcome lands on `dev`;
missing Linear access does not block the merge. Retirement means `Done` unless the
free-workspace deletion policy below applies.

After merge:

1. verify the merge commit on `origin/dev`;
2. re-fetch affected Linear tickets when available;
3. retire fully delivered tickets using the retention policy below;
4. rewrite partial tickets around their remaining acceptance work;
5. cancel or deduplicate obsolete administrative work; and
6. perform a contradiction pass across every available projection.

The terminal check is:

- no accessible active ticket claims a documented delivered baseline is missing;
- no accessible Done ticket contains unshipped acceptance work;
- no Upcoming roadmap row is already completely live; and
- no status/design claim exceeds executable evidence.

Use `$reconcile-delivery-state` for this workflow. If Linear is unavailable, report a
visible warning and a concrete update list rather than silently treating repository
state as a proxy.

## Free-workspace issue budget and retention

The repository is the durable delivery record; Linear is the small coordination view. The
free workspace has a finite issue budget, so do not mirror the wave queue into Linear:

- the committed wave manifest owns lane ids, state, blockers, ownership, and PRs;
- `/plan-epic` and `/inject` reuse a relevant existing `HEX-N` when one exists and otherwise
  leave `ticket: null`; they never create one child per lane;
- a lane without a ticket is ordinary and mergeable, and its audit records `unlinked`; and
- incidental debt is recorded in the manifest or handoff instead of becoming an issue by
  default. A reproducible UI defect remains the one creation exception because
  `$linear-ui-bug-intake` owns its parent and duplicate search.

Completed issues owned by this workflow are deleted after delivery when all of these are
true:

1. the complete promised outcome is verified on `origin/dev`;
2. the identifier, title, outcome, exact `dev` SHA, and delivery PR are recorded durably in
   the wave manifest, PR, or applicable repository status document;
3. no residual acceptance item, active child, dependency, regression investigation, or
   other-owner coordination need remains; and
4. the issue is re-fetched immediately before deletion and the deletion is verified.

Do not delete a partial epic, an active duplicate target, an issue owned by the other owner,
or the current UI bug-bash parent. Move those to the truthful live state and preserve them.
If the connector cannot delete, report the exact manual deletion instead of pretending that
`Done` frees the issue budget. Deletion follows Linear's recoverability window and can become
permanent, so the durable repository record must exist first.

Every skill other than `$linear-ui-bug-intake` reconciles and links existing issues and
never creates. Missing Linear access produces a visible warning and the wave proceeds.

## UI bug intake

Initial UI defect capture is deliberately smaller than delivery reconciliation. Use
`$linear-ui-bug-intake` to reproduce one independently fixable root cause, search for
an existing issue, and either update it or create one child under the current HUD/UI
bug-bash parent. Record the route, build identity, viewport/scale, reproduction steps,
expected and actual behavior, severity rationale, and durable evidence. Re-fetch the
issue after writing it. Do not implement the fix as part of intake.

Resolve the Linear team, labels, workflow states, members, and parent by live
name or identifier on every run. Repository instructions may publish human-readable
defaults such as `HEX-67`, `Backlog`, `Bug`, and the assignee, but skills must not
embed workspace UUIDs or assume a state name is accepted where an API requires a
state identifier. If Linear is unavailable, preserve the complete proposed issue in
the handoff and continue other repository work.

## Optional Linear setup for Codex

Linear is useful for cross-owner visibility. A contributor who wants it can connect
the hosted MCP server once:

```sh
codex mcp add linear --url https://mcp.linear.app/mcp
codex mcp login linear
```

Complete OAuth in the browser, verify with `codex mcp list`, then start a new Codex
session so the Linear tools load. Setup is optional and must not hold up a valid PR.
