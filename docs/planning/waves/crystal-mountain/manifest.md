# Crystal Mountain wave

- Status: `dispatching`
- Wave branch: `wave/crystal-mountain`
- Base: `origin/dev @ 9267d9f899a6caaec870980e10668c82cdcf1d06`
- Pending prerequisite carried by this branch: Crystal Ascent and First Person through
  `7da9cd29fe27ce81baa7c64304a82792647be6d5` (PR #197)
- Coordinator: Codex / world integration
- Epic: Crystal Mountain and cross-biome tunnel
- Shippable outcome: one selectable radius-77 mountain whose only ordinary foot-to-basin
  route is a lit, four-wide tunnel into the existing Crystal Ascent and out into a wooded
  level-150 summit basin enclosed by higher ridges.
- Exclusions: combat population, doors, branches, water features, destructible tunnel
  walls, save migration, alternate surface ascent, and new runtime movement/perception
  mechanics.

## Why this wave exists

The outcome crosses Macro layout settings, exact mask ownership, global volume composition,
interior/light-domain publication, presentation cutaway, and scenario/catalog wiring. Those
parts are independently testable but only meaningful as one composed route, so they ship as
one wave rather than unrelated PRs.

## Locked decisions

1. **D1:** "Crystal Ascent remains centered at the world origin, rotation zero, base level 6,
   and rise 144; its exact radius-32 site may borrow only the columns it needs from adjacent
   Macro masks."
2. **D2:** "The tunnel is the only ordinary foot-to-basin route: level 6, exactly four lanes,
   six clear levels, at least three solid roof levels, and a rough-hewn body transitioning to
   worked Gothic masonry for its final twelve centerline rows."
3. **D3:** "The tunnel and Crystal Ascent publish one Dark interior and one light domain; only
   the foot and summit thresholds are exterior entrances, and every required floor is at
   least Dim from paired nonblocking crystal lights."
4. **D4:** "The five consecutive upper radius-two cells are temperate Forest basin, the other
   radius-two cells are inner mountain wall, and all radius-three cells are outer slope and
   enclosing ridge; no alpine treeline removes temperate basin or Crystal crown trees."
5. **D5:** "Existing runtime traversal, occupancy, LOS, fog, authored-heart occupancy, camera,
   and save contracts remain authoritative; this wave adds generation and presentation data,
   not parallel gameplay mechanics."
6. **D6:** "Legacy Macro configuration and fingerprints remain byte-identical when the new
   defaulted walker, spanning-feature, and anchor-alias collections are empty."

## Shared foundation

- World authority: `MacroLayoutSettings`, `ResolvedLayoutPlan`, exact patch masks and seam
  contracts in `hex_map` remain the source of layout truth. L1 adds defaulted explicit walker,
  tunnel, alias, and resolved subsurface vocabulary without changing legacy behavior.
- World authority: `GeneratedWorldPlan`, `PlannedInterior`, planned lights, anchors, semantic
  volume, and feature ownership remain the runtime publication inputs. L2 composes the tunnel
  once after fragment merge and before final validation.
- Shared presentation: current cutaway and feature reconciliation consume the world-owned
  interior and roof facts. L3 adds no new visibility authority.
- Shared integration: scenario and Sandbox catalogs select the new world recipe. L4 adds no
  generation policy.
- Landing plan: L1 contracts merge first; L2 consumes them; L3 and L4 remain additive and
  merge after the generator contract is stable. The coordinator owns all shared hotspots and
  the final composed validation.

## Dispatch queue

```yaml
- id: L1
  title: Macro spanning-feature contracts
  order: orders/L1-macro-contracts.md
  ticket: null
  authority: world
  builder: worker
  branch: feat/crystal-mountain-contracts
  owns:
    - crates/hex_map/src/settings.rs (Macro settings vocabulary and validation)
    - crates/hex_map/src/procedural_v3/layout.rs (resolved walker/subsurface/alias contracts)
    - crates/hex_map/src/procedural_v3/fingerprint.rs (conditional Macro extension)
    - docs/planning/waves/crystal-mountain/manifest.md (L1 queue row only)
  dispatch_blockers: []
  merge_blockers: []
  fences:
    - path: crates/hex_map/src/procedural_v3/fingerprint.rs
      disposition: keep
      reason: old Mountain Range and Two Rings fingerprints must remain byte-identical
  selector: { concerns: [map_unit, map_generation, map_contracts], full: false }
  evidence: logic-only
  sizing: { model: inherited, effort: high }
  state: queued
  pr: null
- id: L2
  title: Crystal Mountain composition and tunnel
  order: orders/L2-world-composition.md
  ticket: null
  authority: world
  builder: "@codex"
  branch: wave/crystal-mountain
  owns:
    - crates/hex_map/src/procedural_v3/macro_world.rs (merge/finalize and global tunnel)
    - crates/hex_map/src/procedural_v3/macro_landform.rs (Crystal Mountain elevation policy)
    - crates/hex_map/src/procedural_v3/crystal_ascent.rs (Macro landmark validation seam only)
    - crates/hex_map/src/procedural_v3/mod.rs (Crystal Mountain dispatch/report regions)
    - docs/planning/waves/crystal-mountain/manifest.md (L2 queue row and integration ledger)
  dispatch_blockers: []
  merge_blockers: [L1]
  fences:
    - path: crates/hex_map/src/procedural_v3/macro_world.rs
      disposition: keep
      reason: existing Macro route, generation-budget, and full-world validators remain live
  selector: { concerns: [map_generation, map_contracts], full: false }
  evidence: static-presentation
  sizing: { model: inherited, effort: high }
  state: dispatched
  pr: null
- id: L3
  title: Interior cutaway and overlying-feature reconciliation
  order: orders/L3-presentation.md
  ticket: null
  authority: world
  builder: worker
  branch: feat/crystal-mountain-presentation
  owns:
    - crates/hex_world/src (Crystal Mountain cutaway and feature-lifecycle regions only)
    - crates/hex_map/src/procedural_v3/world.rs (presentation facts only)
    - docs/planning/waves/crystal-mountain/manifest.md (L3 queue row only)
  dispatch_blockers: []
  merge_blockers: [L2]
  fences:
    - path: crates/hex_world/src
      disposition: keep
      reason: Map remains opaque and gameplay camera behavior remains unchanged
  selector: { concerns: [world, visual_walk], full: false }
  evidence: static-presentation
  sizing: { model: inherited, effort: high }
  state: queued
  pr: null
- id: L4
  title: Crystal Mountain selection, review route, and docs
  order: orders/L4-content-docs.md
  ticket: null
  authority: shared
  builder: worker
  branch: feat/crystal-mountain-content
  owns:
    - assets/config/worlds/procedural-crystal-mountain.ron
    - assets/config/encounters/crystal-mountain-showcase.ron
    - assets/config/scenarios.ron (Crystal Mountain entry only)
    - assets/config/sandbox_maps.ron (Crystal Mountain entry only)
    - crates/hex_game/src/scenarios.rs (embedded Crystal Mountain path only)
    - crates/hex_game/src/walk.rs (Crystal Mountain review route only)
    - docs/development/config.md
    - docs/systems/world-generation-v3.md
    - docs/systems/interiors.md
    - docs/systems/lighting.md
    - docs/planning/status.md
    - docs/planning/roadmap.md
    - docs/planning/waves/crystal-mountain/manifest.md (L4 queue row only)
  dispatch_blockers: []
  merge_blockers: [L1, L2]
  fences: []
  selector: { concerns: [app, config, docs, visual_walk], full: true }
  evidence: motion-or-feel
  sizing: { model: inherited, effort: high }
  state: queued
  pr: null
```

## Ownership map

- L1 exclusively owns the serialized and resolved vocabulary. L2 consumes those types but
  does not edit their definitions or fingerprint encoding.
- L2 exclusively owns world construction, tunnel routing/carving, unified interiors/lights,
  route/path validators, and generation benchmarks.
- L3 exclusively owns review-cutaway presentation behavior and lifecycle tests. It may read
  semantic roof/interior facts but may not infer them from rendered entities.
- L4 exclusively owns selectable content, embedded paths, scripted review actions, and the
  listed documentation. It may not add generation policy in configuration consumers.
- `manifest.md` is the expected overlap: each worker changes only its queue row; the
  coordinator reconciles all rows after integrating commits.
- Hotspot rule: L2 refreshes after L1 before compiling. L3 refreshes after L2 before its
  final tests. L4 refreshes after the final config schema and generated anchor names exist.

## Territory

- PR #197 / `wave/crystal-ascent` owns the prerequisite Crystal Ascent and first-person
  implementation. This wave carries that exact head and does not rewrite its approved route.
- `origin/dev` contains no competing Crystal Mountain implementation at wave creation.
- Untracked `.context/` is scratch and outside every lane.

## Integration order

1. L1 contracts and legacy fingerprint closure.
2. L2 mask claim, terrain composition, tunnel, interior, lights, anchors, and validators.
3. L3 presentation reconciliation and lifecycle coverage.
4. L4 selectable content, visual walk, documentation, and benchmark records.
5. Coordinator aggregate review, selector plan, full CI-equivalent gate, captures, and human
   First/Third/Map camera review.

## Combined acceptance

- Load Crystal Mountain and publish exactly 18,019 disjoint radius-77 columns.
- Prove the exact seven-cell logical Crystal core, exact radius-32 authored claim, connected
  neighboring masks, five-cell high Forest basin, and enclosing inner/outer mountain ridges.
- Prove the only ordinary route is foot apron -> four-wide level-6 tunnel -> Crystal lower
  threshold -> approved 144-level ascent -> summit -> every basin section.
- Prove tunnel clearance/roof/seam lane pairs, unified Dark interior/domain, Dim route
  coverage, exact Bright pools, physical occupancy/LOS/fog, and no liquids or blockers.
- Regenerate representative seeds and all six rotations; teardown and re-enter gameplay and
  review cutaway without stale roofs, lights, features, anchors, or floating trees.
- Inspect Map, First Person, and Third Person frames for foot portal, natural tunnel, Gothic
  transition, chamber, ascent, summit, basin, ridge, lighting overlay, and review cutaway.
- Run selector-chosen CI, full candidate gate, radius-77 camera/perception budgets, 10,000
  idle-frame perception gate, and Macro generation/materialization/entity/memory comparisons.

## Stop conditions

- A required mask subtraction disconnects an adjacent biome.
- Tunnel routing requires an undeclared seam, surface bypass, bedrock breach, or a second
  runtime traversal/perception authority.
- Legacy Macro settings or fingerprints change with empty extensions.
- A presentation lane begins reconstructing world truth from rendering state.
- The prerequisite Crystal Ascent/first-person head changes incompatibly before this wave
  lands; refresh additively and rerun the exact route gates.

## Injection log

- 2026-08-10: initial four-lane implementation wave approved by the user.

## Close-out

Open one `wave/crystal-mountain -> dev` PR after combined acceptance and named human runtime
PASS. After landing, record the exact merge SHA, close superseded source PRs, update delivery
state, and remove transient `orders/` files in the close-out PR.
