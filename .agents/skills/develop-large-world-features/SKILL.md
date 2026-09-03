---
name: develop-large-world-features
description: Plan, implement, and checkpoint very large or cross-biome Hex Game world features without patch-order artifacts, topology drift, or silent scale reductions. Use for authored mountains, landmarks, long routes, global reservations, multi-chunk structures, full-scale proxies, or world changes whose generation, runtime publication, rendering, picking, perception, and performance must be proved separately.
---

# Develop Large World Features

Treat a large world feature as one authoritative composition with multiple evidence lanes,
not as decorative patches that happen to overlap.

Read [AGENTS.md](../../../AGENTS.md), [CLAUDE.md](../../../CLAUDE.md),
[the architecture contract](../../../docs/architecture.md),
[the cross-owner contracts](../../../docs/contracts.md), the relevant map/world design
documents, and the
[large-world cases](../../../docs/development/problem-solving-casebook.md). Use
`$plan-parallel-work` before splitting implementation across lanes.

## 1. Lock the authored contract and budgets

Record the exact source revision, coordinate register, footprint, scale, topology,
required routes, landmarks, vertical range, decoration level, and exclusions. If the
source has not been approved as an exact oracle, use `$trace-authored-map-reference`
first.

Make acceptance targets explicit before implementation:

- topology and connectivity invariants;
- deterministic seed and rotation behavior;
- terrain-ready and re-entry timing;
- resident entity, memory, and chunk budgets;
- snapshot/edit/picking/perception latency;
- required static views and native-motion checks.

Targets remain red until met or the user deliberately changes the product contract. Do
not quietly shrink the map, remove authored routes, reduce vertical drama, or relax a
validator to manufacture green results.

## 2. Classify the feature boundary

Choose the narrowest correct authority:

- **Patch-local:** output depends only on the patch plus stable neighboring inputs and can
  be generated independently in any order.
- **Global reservation:** one feature owns cells across patches or biomes and needs a
  single authoritative footprint.
- **Global composition:** routes, carves, supports, or finishing depend on the merged
  whole and must finalize atomically.

A named mountain spanning several biomes, a long authored route, or a structure whose
support/ownership is global is not patch-local merely because chunks publish it.

## 3. Compose once, publish locally

For a global feature, use this order:

1. Build or load the canonical whole-feature plan.
2. Reserve its authoritative footprint before ordinary local generation competes for it.
3. Let biome or patch recipes generate around the reservation using stable inputs.
4. Merge contributions into one semantic world plan.
5. Carve routes, resolve supports and face ownership, and finalize the feature once.
6. Partition the finalized result into chunk-native publication data.

Do not carve or finalize the same global feature independently inside each patch. Guard
against gaps down to the support/foundation boundary, coincident faces, duplicate cells,
and seams whose result changes with iteration order.

## 4. Keep transforms local and deterministic

Express shape logic, tie-breakers, region IDs, and authored route decisions in a local
feature frame. Transform to world coordinates only at the boundary. Test rotations and
translations explicitly; world-coordinate tie-breakers can preserve ordinary seed
determinism while silently breaking rotational equivalence.

For every 60-degree rotation, inverse-transform the completed semantic result back into the
canonical frame and compare it against the independent canonical oracle. Also permute patch
and chunk publication order; topology, ownership, IDs, routes, and fingerprints must remain
unchanged unless the contract explicitly versions them.

Hoist invariant routing, reservation, and candidate data out of per-cell evaluation. Cache
seed-invariant work and keep parallel evaluation deterministic.

## 5. Prove a full-scale proxy before final content

Publish the exact intended footprint and vertical scale with minimal decoration first. A
proxy checkpoint is meant to expose architecture and budget failures cheaply; it is not
permission to substitute a smaller world.

Run the production lifecycle, including loading/readiness, ordinary actor spawn, map and
camera entry, return-to-title or regeneration, snapshots, edits, picking, perception, and
chunk publication. Record exact-head measurements and entity/memory counts.

If scale passes topology but fails runtime budgets, preserve the approved contract and
pause final decoration. Propose an architecture correction—such as chunk meshes with
exact semantic picking or a separate perception optimization—rather than hiding the red
gate.

## 6. Validate independent lanes

Run and report each lane separately:

1. **Oracle:** exact authored membership and layers.
2. **Generation:** determinism, rotations, corpus coverage, topology, routes, supports,
   face ownership, and regeneration/re-entry.
3. **Runtime:** readiness, publication, snapshots, edits, picking, and perception.
4. **Performance:** terrain-ready, p95 or named latency gates, resident entities, memory,
   and build/runtime resource use.
5. **Presentation:** fresh exact-head matrix through `$inspect-game-renders`.
6. **Motion/feel:** native-camera motion and human checks where required.

One green lane cannot clear another. A still can expose a support gap but cannot prove a
flicker is absent. A self-consistent generator test cannot prove the authored map was
transcribed correctly.

## Checkpoint Rules

At each useful checkpoint record source revision, exact commit, changed contract, tests
with executed counts, performance numbers, capture provenance, and every red target.
Call the checkpoint `runtime-ready`, `proxy-only`, or `provisional` accurately. Resume
from the last explicit product decision after interruptions instead of silently making a
new one.

## Stop Conditions

- The authored source or coordinate register is disputed.
- Ownership crosses the world/gameplay boundary without an explicit shared contract.
- The global feature is being finalized patch by patch.
- A full-scale proxy exceeds a locked budget and the next step would add content anyway.
- A validator or required route is being weakened solely to pass the current output.
- The current exact-head gate fails. Preserve the failure and repair it before claiming
  the checkpoint complete.
