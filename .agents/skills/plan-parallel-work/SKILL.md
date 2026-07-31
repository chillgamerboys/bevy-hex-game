---
name: plan-parallel-work
description: Choose independent, stacked, or wave topology before parallel implementation. Use when dividing a feature among agents or owners, planning three or more related branches or PRs, coordinating shared contracts or hot files, or when the user asks how to avoid duplicate PR and CI work. Do not use for one clearly independent change.
---

# Plan Parallel Work

Choose the integration and review shape before creating implementation branches. The
output is an ownership-aware lane plan that minimizes duplicate work without weakening
the combined release gate.

## Read the repository contracts

Read `AGENTS.md`, `CLAUDE.md`, and
`docs/development/parallel-development.md`. Read `docs/architecture.md`,
`docs/contracts.md`, and `docs/planning/boundary.md` when more than one owner or a
shared crate is involved. Read the relevant system and design docs for the requested
behavior.

Strongly prefer `$reconcile-delivery-state` first when the plan begins from existing
tickets, roadmap state, or claimed delivered work. Linear is a soft coordination
signal: if unavailable, warn and continue from repository/GitHub evidence.

## 1. Map the outcome and concerns

State one shippable outcome and explicit exclusions. List the concerns needed to
reach it, then identify for each concern:

- its world, gameplay, or shared owner;
- expected crates, contracts, and hot files;
- strict dependencies;
- whether it is meaningful to review or ship alone; and
- the runtime paths that prove it works.

Record relevant ticket references and their verified current/residual scope when
Linear is available. For broad roadmap or architecture work, inspect every
non-completed Hex Game ticket rather than only the current branch's references. If
that inventory cannot run, put the skipped reconciliation in the handoff.

Group work by authority and shared concern, not merely by file or ticket. If two lanes
would implement the same authority, make that one concern with one owner.

## 2. Classify the topology

Use the repository decision table:

- **Independent:** each lane is shippable alone, has no unsettled shared contract,
  and has little risky overlap. Each branch and PR targets `dev`.
- **Stacked:** one small lane strictly depends on one parent and isolated child review
  remains useful. Keep stacks no deeper than two.
- **Wave:** choose this for three or more related lanes, a deeper stack, shared hot
  files or contracts, one meaningful combined runtime checkpoint, or substantial
  duplicate CI/reconciliation cost.

Explain the evidence for the classification. Do not choose a wave merely because work
is large; unrelated changes remain independent.

## 3. Resolve the foundation and ownership

When implementation needs new facts across the gameplay/world boundary, plan a small
behavior-neutral foundation first: docs, shared vocabulary, scheduling/readiness
contracts, and headless contract tests. Land it on `dev` before dependent lanes.

Name one integration owner for a wave. Lane owners write only to their source
branches; the integration owner writes to the shared wave. Keep world and gameplay
changes in identifiable commits even when the final PR contains both.

An unresolved cross-owner behavior choice is a stop condition. A known live contract
or a mechanical integration detail is not.

## 4. Set the review budget

Define focused lane checks and combined acceptance separately. Put composition,
regeneration, failure paths, return-to-title/re-entry, automated visual review, and
human play on the combined candidate. Full platform CI belongs on every independent
release unit or once on the wave; do not manufacture leaf PRs solely to repeat it.

## 5. Produce the plan

For independent or stacked work, return:

- topology and rationale;
- branch/base table with owner and dependencies;
- shared foundation, if any;
- review and validation per release unit; and
- retargeting or cleanup order.

For a wave, read `references/wave-manifest.md` and fill it in. Store it under
`.context/waves/<name>.md` when the task calls for implementation, or return it in the
response when the user asked only for a plan. Do not open PRs or create remote state
unless the user also asked to execute.

If execution is authorized and no semantic ambiguity remains, continue from the
approved or user-supplied plan without pausing merely to reconfirm its topology.

## Guardrails

- Do not create one PR per ticket or agent by default.
- Do not hide separate owners behind a generic “integration” label.
- Do not use a wave to combine unrelated work.
- Do not rebase or force-push published or shared branches.
- Do not claim leaf validation proves combined behavior.
