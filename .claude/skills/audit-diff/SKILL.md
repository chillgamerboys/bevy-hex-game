---
name: audit-diff
description: "Review the current PR diff for correctness, including silent failures, ownership boundaries, edge cases, Bevy ordering, evidence quality, and configuration traceability. Findings only; never edits the candidate."
---

# Audit the diff

Use this for a standalone review or as the review phase of `/audit-pr`.

## Scope

1. Resolve the actual PR base with `gh pr view --json number,baseRefName,title,body`.
   Without a PR, default to `origin/dev` and say so.
2. Fetch the base, then inspect the committed range with
   `git diff origin/<base>...HEAD`. Also inspect `git diff --cached`, `git diff`, and
   every nonignored untracked file. Keep local-only changes visibly separate from the
   pushed PR range; `/audit-pr` already requires them to be absent. Never audit a wave
   source against `dev` when its PR targets `wave/*`.
3. Read the affected crate's `CLAUDE.md` and the contracts named by root `CLAUDE.md`.
   Use [parallel-development.md](../../../docs/development/parallel-development.md)
   for topology and [delivery-state.md](../../../docs/development/delivery-state.md)
   for projection reconciliation. When the PR targets `wave/*`, also read its lane entry
   and work order under `docs/planning/waves/<slug>/`; the lane field table is in
   [wave-protocol.md](../../../docs/development/wave-protocol.md).
4. Split independent review surfaces among at most three fresh-eyes agents when that
   materially improves coverage. Agents report findings; they do not edit files.

## Review lenses

Walk every applicable lens. Report `file:line`, a short excerpt, impact, and
`SHIP-BLOCKER` or `NON-BLOCKER`.

1. **Silent failure and error semantics.** Inspect changed error paths and sibling
   uses for swallowed parse/IO errors, bare `.ok()`, empty `Err` arms, `if let Ok`
   without justified rejection handling, startup log-and-continue, bare `#[ignore]`,
   vacuous lint reasons, weak `expect` invariants, and broad automation suppression.
   Tests may use panic-shaped assertions; startup/content loading must still fail loud.
2. **Single authority and ownership.** Reject duplicated constants, parallel models,
   facts reconstructed from another owner's private implementation, dependency-ceiling
   violations, and crate-local ordering that should be a shared contract.
3. **Edge values.** Exercise the input domain relevant to the change: empty and
   singleton collections, zero and maximum bounded identities, invalid RON, first
   frame/zero duration, negative hex coordinates, stacked surfaces, level zero,
   buried headroom, overflow, and non-finite values.
4. **Round trips and determinism.** Trace serialization, migration, save/reload,
   seed/content identity, retries, ordered collections, and configuration defaults
   through every producer and consumer. Refusal must be transactional and preserve
   invalid user data where the contract requires it.
5. **Compiles-but-wrong APIs.** Check current Bevy traps from root `CLAUDE.md`, notably
   messages versus events, global observers, `Option<Res<_>>` for state-scoped
   resources, translucent material alpha, cursor input, `AssetMut`, and stack-safe
   `TilePos` rather than `HexCoord`-only authority.
6. **Commands and schedules.** Look for deferred-command reads without a sync point,
   cross-crate ordering without shared sets, gameplay work outside pause gates, and
   teardown/re-entry races.
7. **Test altitude and evidence.** Pure rules need pure assertions; scheduling and
   lifecycle need focused headless contracts; composition needs canonical snapshots.
   Frames judge static presentation and humans judge motion/feel. Neither can prove or
   corroborate gameplay or exact world logic.
8. **Feature, configuration, and persistence traceability.** Follow each changed
   feature, setting, preference, schema field, environment variable, and path through
   default-build and feature-gated code. Missing/invalid initial content must not
   silently default. Shipping must not accidentally gain test/dev behavior.
9. **Lane ownership**, when the PR is a wave lane. Compare every changed path against the
   lane's declared `owns` list. A path outside it is a `SHIP-BLOCKER` even when the change
   itself is correct: it is invisible to the coordinator's disjointness union and to every
   sibling's additive refresh, so the first sign of it is a conflict or a silent revert on
   the wave.
   A change that crosses the world/gameplay authority boundary is a `SHIP-BLOCKER`
   unconditionally. Read what the lane *removed*, not only what it added — a builder on a
   stale base can rewrite a shared file wholesale and revert a deliberate default inside
   it, which looks additive in every summary.

For documentation-only diffs, replace the code-specific checks with: automation
contracts, links and fragments, claims against executable reality, and single-source
consistency. Do not mutate documentation merely to make the audit green.

## Lightweight silent-failure scan

Use searches only to seed inspection; context determines whether a match is a bug.
Search changed Rust and nearby modules for:

```sh
grep -RInE '\.unwrap_or_default\(\)|\.unwrap_or\(|\.ok\(\);|Err\(_\).*=>|if let Ok\(' crates --include='*.rs'
grep -RInE '#\[ignore\][[:space:]]*$|reason[[:space:]]*=[[:space:]]*"(TODO|temp|fixme|for now|)"' crates --include='*.rs'
grep -RInE '\|\| true' .github scripts 2>/dev/null
```

Do not dump repository-wide matches into the report. Report only investigated issues
that intersect the changed authority or reveal the same changed bug pattern nearby.

## Verification and result

When standalone, invoke `/test-quick` after the read-only review. When called from
`/audit-pr`, reuse `/test-full` evidence instead of running validation twice.

Return:

```text
audit-diff — PR #<number> against <base>
SHIP-BLOCKERS: <count>
NON-BLOCKERS: <count>
Verification: <focused result or supplied by audit-pr>
Findings:
- path:line — excerpt — impact — severity
```

An empty findings list must state which lenses were applicable. This skill never fixes,
commits, pushes, edits tickets, or appends an audit log.
