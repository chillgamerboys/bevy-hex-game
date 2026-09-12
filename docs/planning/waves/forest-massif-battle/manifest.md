# Forest–Massif Battle

Status: integrated local candidate; logic validated; final native visual review in progress. Coordinator: root. Branch: wave/forest-massif-battle.
User-approved local implementation; no dev/main merge or remote publication in this delivery.
Source: battle 99d56c7 and V4 45f689b, common base 495a73d, composed in 98f68fb.
The explicit user source checkpoints supersede the ordinary dev-first landing sequence for this local experiment. Shared changes remain reviewable in the candidate.

## Locked decisions
1. Radius 187, west broadleaf/pine/ancient forest, one north–south river and one central bridge, east massif.
2. Five forest parties 2,3,4,5+Shaman,6+Shaman; three individual Dragons; 26 actors including Human.
3. Player starts with 45 speed, 12 gravity, 15 contact Fireball damage, 12 knockback, .5 cooldown. Enemy tuning remains baseline.
4. All 22 forest deaths award +25 damage once; all three Dragons unlock radius 2.5 explosions once. Freeze projectile mode/damage at release.
5. XP 1/5/20; additional threshold ceil(10*1.5^(level-1)); one beneficial upgrade per level. Restart resets all. Clear victory permits exploration.
6. World authority owns V4 data, material edits, exact occupancy and anchors; gameplay owns encounters/progression; UI only consumes these.

## Shared foundation
ArenaMap::ForestMassif is the shared selector. ArenaTerrainView/ArenaVoxelGeometry/ArenaSystems remain the world-to-gameplay contracts. Geometry max_level is authoritative. V4 package remains runtime terrain authority through world-owned adapter.
Gameplay publishes ProgressSnapshot, UpgradeStat, progress(), player_tuning(base), spend_upgrade(stat), is_forest_run(), completed_run(). Frozen FireballMode belongs to gameplay.
Named anchors: party_start, hostile_start, forest_outer_a, forest_outer_b, forest_middle, forest_deep_a, forest_deep_b, dragon_lower, dragon_middle, dragon_upper, ancient_tree, bridge_west, bridge_east; world publishes region-local names under a stable forest prefix where needed, adapter strips prefix.

## Dispatch queue
```yaml
lanes:
  - id: L1
    title: Forest content
    authority: world
    builder: worker
    branch: feat/forest-world-content
    owns: [assets/config/v4/forest-massif, assets/art/objects/plant/forest-*, assets/art/object_catalog.ron, tools/forest_world.py]
    dispatch_blockers: []
    merge_blockers: []
    evidence: static-presentation
    state: integrated
    ticket: null
    pr: null
  - id: L2
    title: Encounters and progression
    authority: gameplay
    builder: worker
    branch: feat/forest-progression
    owns: [crates/hex_arena]
    dispatch_blockers: []
    merge_blockers: []
    evidence: logic-only
    state: integrated
    ticket: null
    pr: null
  - id: L3
    title: Menu and presentation
    authority: shared
    builder: worker
    branch: feat/forest-battle-ui
    owns: [crates/hex_game/src/arena, crates/hex_game/src/screens/battle_launch.rs, tools/arena.py]
    dispatch_blockers: []
    merge_blockers: [L2]
    evidence: motion-or-feel
    state: integrated
    ticket: null
    pr: null
```

## Ownership and integration
Root owns hex_core, world adapter in hex_map/arena, shared manifests/Cargo, docs and final combination. Worker worktrees are isolated. Content and gameplay start independently; UI compiles after gameplay interface integration. Root merges content, gameplay, then UI. No unrelated changes to donor checkouts.

## Territory
Read-only GitHub inventory: #220 V4 foundation stacks on #219 Grand; both are source donors. Older biome #210–213 and lattice #196 remain separate. No remote PR edits. Linear reconciliation is unavailable in this local delivery; tickets are not required.

## Validation
Focused world compile/geometry/roster tests; contact-only vs explosion, reward, XP, spend/reset tests; unchanged enemy and Duel/Fort checks; app input/menu/reset checks; full map/forest/bridge/massif windowless captures; actual runtime performance. Source launch through Cargo with asset-log validation. Automated native controls and rendering checks are followed by the user’s subjective combat-feel review. Full selector checks are required before a future dev merge; this local delivery reports exact checks run.

## Integrated evidence

The runtime-loaded stone-bridge package is `987d396f3972feb5`. Source generation, exact tree spacing/height/stock checks and 388 runtime column probes pass. The source has 105,469 columns, 444 storage chunks, 275 exact-voxel trees and thirteen named gameplay anchors.

Gameplay lane validation passed all 334 `hex_arena` library tests, eight final focused progression/contact/liquid checks, and strict arena Clippy. Python arena helper checks passed; combined game, runtime terrain edits, environment captures and native playtest remain the coordinator’s final gate. The ignored combined fixture measures ArenaTick CPU time only, with synthetic player camp visits; it makes no renderer or FPS claim.

Combined validation: 108 arena app tests pass; actual V4 world edit roundtrip passes; full authored spawn/contact-kill/reset fixture passes with 26 actors and eight parties. The bounded three-second CPU fixture recorded all eight parties active for all 360 measured ticks (median 2.867 ms; p95 7.637 ms; no renderer/FPS claim). Eighteen land routes passed 750 runtime column probes; grades remain at most .35 units per hex, below .4 automatic step. All eleven final progression tests pass, including both clear orders, prior purchases plus the 25-point reward beyond the normal cap, protected-tree contacts, and launch-time payloads. Contact hit presentation now has a separate spark kind with no explosion sphere. Final windowless captures and native controls review follow.

## Wider crowns and mountain revision

The user's follow-up keeps the combat and encounter contracts and asks for at
least 50% forest canopy coverage, a taller irregular massif and surrounding
mountains. The content lane additionally owns narrow hex_schematic reservation
and interval-admission changes and their focused tests; the presentation lane owns
hex_objects hidden-cell baking and Forest capture readiness. Root owns the shared
radius-32 / 65,536-cell blueprint bounds and updated catalog fingerprints. Crown
coverage includes the forest mask's camps, paths and clearings; exact vertical
occupancy must remain disjoint. Anchors, trunk centers and gameplay remain fixed.

The shared blueprint boundary suite passes all 15 tests. Revised content compile,
traversal, combined spawn/reset and windowless review are in progress. Native
interactive validation is blocked by the locked Mac; no native feel claim is made.
