---
name: plan-epic
description: Decompose a wave into lanes with disjoint crate authority and file ownership, sequence them with separate dispatch and merge blockers, and commit the wave manifest and work orders under docs/planning/waves/. Use once $plan-parallel-work has classified the work as a wave with about three or more parallelizable lanes. Do not use for a single ticket, a genuine dependency chain, or a wave that is already dispatching; Linear is a soft signal and never blocks planning.
---

# Plan Epic

Read
[`docs/development/wave-protocol.md`](../../../docs/development/wave-protocol.md)
as the complete canonical policy: the artifact layout, the lane field table, the ownership
algebra, the sequencing rules, and the merge order. Do not restate the field table here or
in the manifest's prose — it is versioned in that document, and four copies drift.

Read `AGENTS.md`, `CLAUDE.md`, and `docs/development/parallel-development.md` first if the
work has not yet been classified. `$plan-parallel-work` owns the independent/stacked/wave
choice; this skill starts once the answer is a wave. Read `docs/architecture.md`,
`docs/contracts.md`, and `docs/planning/boundary.md` when more than one owner is involved.

## Produce, in this order

1. **Survey and bank the maps.** Write the exploration down under
   `docs/planning/waves/<slug>/maps/`, with `file:line` anchors and per-symbol
   dispositions. A map that lives only in a session dies with it. Every order citing a map
   carries the rule: if reality disagrees with the map, that is an escalation, not a
   judgment call.

2. **Sweep territory.** `gh pr list --state open` across all authors and
   `git fetch origin --prune`, reading the new-branch lines rather than discarding them.
   Record one row per PR or branch with a measured footprint —
   `git diff --stat origin/dev...origin/<branch>`. A shared file is not a blocker by
   itself; the relationship is the point. The world owner lands map PRs on `dev`
   continuously, so this table goes stale in hours.

3. **Decompose by authority first, then by file.** One lane sits inside one crate
   authority — world, gameplay, or shared. **A lane crossing the world/gameplay boundary is
   a stop condition**: re-cut the seam, or plan a behavior-neutral foundation lane that
   lands on `dev` first. Where a file must be shared, name the regions, the composed
   end-state, and the hotspot rule. There is no third option. Tag every lane's builder —
   `worker`, or `@<login>` for a lane a human builds themselves, which is territory and
   never a dispatch.

4. **Sequence with two separate fields.** `dispatch_blockers` is what must be true to
   START; `merge_blockers` is what must LAND first. Most lanes have merge blockers and an
   empty `dispatch_blockers`. One ordered list collapses the two and turns a merge chain
   into a work chain. If the lanes genuinely form a chain, say so and stop — that is
   stacked work.

5. **Record the selector consequence and the evidence class per lane.** Read
   `.config/test-scopes.json`: a lane touching a path that promotes to the complete gate
   pays for it, and three such lanes are usually one foundation lane plus two. Classify
   each lane `logic-only`, `static-presentation`, or `motion-or-feel`; a motion-or-feel
   claim cannot be proven by a lane and defers to the combined candidate.

6. **Resolve ambiguity into numbered locked decisions**, quoted verbatim into every order
   they bind. Decisions are amendable, never edited. A decision that is not yours to
   make — most often a behavior choice inside the other owner's crates — gets tagged with
   its owner and routed through a review draft, not guessed.

7. **Commit the manifest and orders** under `docs/planning/waves/<slug>/`, on a branch off
   `origin/dev`, through the ordinary PR path. `.context/` is untracked scratch and is
   invisible to anyone who checks the branch out, so a wave plan stored there cannot be read
   by the people who have to build it.

## Linear

Reconcile existing `HEX-N` through `$reconcile-delivery-state`. This skill may create the
child issues of an approved manifest — it is one of two workflows authorized to create,
because it owns both a parent and its own deduplication (see
`docs/development/delivery-state.md`). Resolve the team, parent, and workflow state from
live connector data on every run; never embed a workspace UUID, and never create the epic
itself.

If Linear is unavailable, warn visibly, key every lane on its lane id, put the recommended
child set in the handoff, and continue. Linear never blocks planning or dispatch.

## Capability boundary

Executing the wave with parallel agents is Claude-side: `/dispatch` runs isolated worktrees
and `/inject` mutates a running wave. Both depend on harness worktree isolation and agent
messaging that Codex does not have, which is why there is no `$dispatch`.

A Codex coordinator lands lanes by hand, following `wave-protocol.md` §6 for the merge
order and the composed-tree check, §8 when the wave is being assembled from branches that
already exist, and §9 for landing and close-out. Do not half-execute a wave: without the slot audit
and the territory sweep, parallel lanes stop being tracked and the ownership union stops
being true.

## Guardrails

- Do not create one lane per ticket by default. Lanes are units of ownership.
- Do not use a wave to combine unrelated work.
- Do not decompose a dependency chain into a wave to make it look parallel.
- Do not open PRs or create remote state unless execution was also requested.
- Do not re-plan over a wave that is already dispatching; that is an injection.
