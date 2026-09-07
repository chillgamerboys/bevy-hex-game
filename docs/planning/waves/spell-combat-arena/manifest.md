# Spell Combat Arena — local experiment

Status: combined locally; integration validation in progress. Branch: experiment/spell-combat-arena. Base: dev 495a73dcbe7edbab6d993867d91b15979fa6ce81. Coordinator: root. Ticket: null (Linear reauthentication required).

Outcome: native offline first-person spell duel against one disposable bot. The user explicitly chose a local experiment branch until playtesting, so the shared foundation and lanes stay local; no remote PR/merge or production promotion is part of this experiment.

## Locked decisions

1. First person default; C toggles tightly constrained third person. WASD, mouse look, Space jump, Shift sprint, 1/2/3 select, left click casts, T toggles selected projectile preview, Escape pauses, R fully resets.
2. Shield is a physical ballistic seed, cover-only stone wall, persists until destroyed. Revalidate complete supported footprint including both continuous bodies before creation. Invalid hits/footprints fizzle and consume cooldown.
3. Fireball is ballistic, detonates once at earliest body/terrain hit, damages its caster. Area Blast is self-centered and excludes its caster. Radial damage and knockback ignore cover. All explosive damage uses world-space spheres.
4. HP100; shield/fireball/area cooldowns5/1.25/7 seconds; max actor damage0/35/45; terrain power2. Projectile launch speed32, gravity12. Three independently chosen size presets: shield3x4/5x5/7x6 at thickness1; fireball radius1.5/2.5/3.5; blast radius2.5/4/5.5. Standard default.
5. Accepted M01 walk3.5/run7/body2 levels/radius.25/step1/jump3.25/gravity17.333334/coyote and buffer.1; no flight, no automatic terrain recovery teleport for ordinary falling. External impulse velocity survives input.
6. Radius12, .4 level height. Two elevated platforms with ramps, cover, destructible soil, protected bedrock. No V4, multiplayer, progression, or visual-lab imports.

## Foundation and public interfaces

`hex_core::arena` defines ArenaTick, ArenaSystems::{ApplyTerrain,PublishTerrain,Simulate}, ArenaVoxelGeometry, ArenaTerrainView (complete sorted occupancy, revision, two spawn positions), ArenaMaterials (catalog identities), ArenaReset (generation). Only world writes terrain facts. Voxel top is level*.4 and bottom is (level-1)*.4 in this isolated producer.

World owns existing TerrainEdit and TerrainImpact processing and exact TerrainImpactOutcome. Gameplay reads the public terrain view and emits those messages; it must not access VoxelMap. New arena producer is isolated inside hex_map and reuses production private VoxelMap and TerrainDamageState; it avoids importing the pending large-map branch. Gameplay has its own real continuous actors and never uses the inspection ghost/tactical authority. Presentation reads immutable simulation state and submits ActorIntent.

## Dispatch queue

```yaml
lanes:
  - id: L1
    title: Arena terrain producer
    order: orders/L1-world.md
    ticket: null
    authority: world
    builder: worker
    branch: experiment/arena-world
    owns: [crates/hex_map/src/arena.rs, crates/hex_map/src/arena/, "crates/hex_map/src/lib.rs: arena module export", "crates/hex_map/Cargo.toml: arena feature and ron dependency"]
    dispatch_blockers: []
    merge_blockers: []
    fences: []
    selector: {concerns: [map_unit, map_contracts, clippy, docs, shipping], full: true}
    evidence: static-presentation
    sizing: {model: inherited, effort: inherited}
    state: dispatched
    pr: null
  - id: L2
    title: Continuous actors and spell simulation
    order: orders/L2-gameplay.md
    ticket: null
    authority: gameplay
    builder: worker
    branch: experiment/arena-gameplay
    owns: [crates/hex_arena/]
    dispatch_blockers: []
    merge_blockers: [L1]
    fences: []
    selector: {concerns: [contracts, simulation, clippy, docs], full: true}
    evidence: motion-or-feel
    sizing: {model: inherited, effort: inherited}
    state: dispatched
    pr: null
  - id: L3
    title: Native launch, input, camera, HUD, capture and integration
    order: orders/L3-integration.md
    ticket: null
    authority: shared
    builder: worker
    branch: experiment/spell-combat-arena
    owns: [crates/hex_game/, Cargo.toml, Cargo.lock, assets/config/arena.ron, tools/arena.py, docs/planning/waves/spell-combat-arena/, docs/architecture.md, docs/contracts.md, docs/planning/status.md]
    dispatch_blockers: []
    merge_blockers: [L1, L2]
    fences: []
    selector: {concerns: [app, clippy, docs, shipping], full: true}
    evidence: motion-or-feel
    sizing: {model: inherited, effort: inherited}
    state: dispatched
    pr: null
```

## Territory and integration

Current upstream dev remains495a73d. Open PR219 includes unrelated GrandV3/grounded work; PR220 is V4; PR196 is tactical lattice work. All stay outside this candidate. The active visual-lab checkout is read-only source for M01 controller/collision; selectively port those files rather than merging its branch. Root writes foundation core geometry and shared manifest. Worker branches own only their declared crates; root integrates additive commits. No published branches are rebased.

## Acceptance

At 120Hz apply/reset world, publish/flush, refresh collision, simulate actors and casts, then next tick settles effects. Windowless static review: whole-footprint overview, first person, close third person, shield/impact sizes, HUD/tuning, at least two cover angles. Typed tests: round-trip geometry, true radial selection, nearest swept collision, continuous movement and impulses, cooldown and click edges, self effects, terrain damage/outcome, valid/invalid shields, immediate next-tick collision, reset/focus/KO. Native feel remains human pending. Focused lane tests first; combined selector and lint/shipping gates once. Record known baseline failures separately. Inspect every rendered frame before delivering.

## Stop conditions and close-out

No deletion of source/saves/review evidence; disk cleanup authorized separately by user and receipted in task outputs. No native focus-stealing review. Missing authoritative fact requires a shared-contract correction, never private world reconstruction. Finish with committed local prototype, launcher, controls/tuning guide, and exact-head validation report. No automatic merge.

## Local checkpoints

- World producer committed and integrated; eight focused world tests passed.
- Gameplay committed and integrated; first focused run passed 26 arena tests and 124 core tests. Additional round-ending and race regressions are being added before combined validation.
- Native composition, input, two cameras, HUD, tuning, windowless capture matrix, and combined lifecycle tests are authored. Combined native build and render inspection remain pending.
- Obsolete generated caches were removed under the user's explicit cleanup instruction; the output receipt records 14,178,758,656 physical bytes recovered. Source checkouts and review evidence were preserved.
- Hourly overnight follow-up is attached to this task until delivery or September 7, 08:00 America/Los_Angeles.
