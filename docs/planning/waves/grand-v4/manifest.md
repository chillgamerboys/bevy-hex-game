# Grand V4 integration wave

Status: dispatching
Wave: `wave/grand-v4`
Verified remote dev: `bb556963632de933b44fb75b1d306aca79258cef`
Playable integration base: `873a2c37eeb8eda23c11d398ce7d68d7965a64db`
Coordinator: root Codex agent. Epic: none; Linear reauthentication required.
Outcome: full-scale Grand remake with shrine progression, combat/marine traversal and durable arena resume.
Exclusions: new species/boss variants, advanced elemental control, tactical/network changes, V3 save compatibility.

## Why a wave
World, gameplay and shared app must be reviewed together; existing #222/#223 form the playable base. No individual leaf proves the expedition or save composition.

## Locked decisions
1. Grand V4 is a new selectable streamed expedition based on playable 873a2c37; preserve all existing maps.
2. Mainland coastline-enclosed area including inland lakes is seven times canonical Grand V3 mainland, excluding ocean/offshore islands. Crystal Ascent is independently seven times original footprint, never scaled twice.
3. Preserve approved relative layout, intimate garden/lake, connected waterfall/library, separate straight Shadow tunnel, broad forest/world tree and offshore volcano calibrated to 45 seconds favorable unupgraded sailing.
4. Five one-time cumulative shrines: Fire damage/size/explosions; Air projectile/glider; Water swim/boat; Earth new player structures/run and never HP; Plant size/jump. Existing XP remains. Shadow grants teleport instead of Forest HP reward.
5. Teleport is X, visible supported ground within 12 units, 6-second cooldown, gaps allowed, walls/unloaded/occupied destinations refused without cooldown.
6. One dedicated atomic resume slot preserves combat, movement, progression, enemies, clocks, sparse destruction and partial damage. Death respawns at last shrine/start retaining progress. Explicit New Run resets.
7. Existing species/population budget first. New bosses, scent/group ecology and advanced elemental spells are deferred.
8. Accepted sailing/boat/swim/glider baseline is protected; natural wind influence on ordinary gliding stays 65 percent. Water evolution is compared independently.
9. Automated rendering is windowless. Logical claims require typed tests; native feel and 60 FPS remain explicitly unverified until measured.

## Shared foundation
Existing ArenaTick, exact TilePos, ArenaExpeditionSites, ArenaAvailability, finite V4 packages and atomic owner attachments remain authority. Gameplay owns ActorId widening and Grand progression/checkpoint. World owns Grand package/sites, finite overlay codecs and partial damage. Coordinator owns shared map-kind declaration and application adapters. New additive vocabulary lands in wave before consumers; no old-map semantics are changed merely to admit Grand.

## Dispatch queue
```yaml
- id: L1
  title: World content and publication
  order: orders/L1.md
  ticket: null
  authority: world
  builder: worker
  branch: feat/grand-v4-world
  owns:
    - "crates/hex_schematic/**"
    - "crates/hex_map/** (except water-field files explicitly coordinated)"
    - "assets/config/v4/grand-v4/**"
    - "tools/grand_*"
    - "assets/art/** (new Grand assets only)"
    - "docs/planning/waves/grand-v4/manifest.md (L1 state/pr only)"
  dispatch_blockers: []
  merge_blockers: []
  fences: []
  selector: {concerns: [full], full: true}
  evidence: motion-or-feel
  sizing: {model: inherited, effort: inherited}
  state: dispatched
  pr: null
- id: L2
  title: Gameplay progression and checkpoint
  order: orders/L2.md
  ticket: null
  authority: gameplay
  builder: worker
  branch: feat/grand-v4-gameplay
  owns:
    - "crates/hex_arena/**"
    - "assets/config/arena/grand-v4.ron"
    - "docs/planning/waves/grand-v4/manifest.md (L2 state/pr only)"
  dispatch_blockers: []
  merge_blockers: []
  fences: []
  selector: {concerns: [full], full: true}
  evidence: motion-or-feel
  sizing: {model: inherited, effort: inherited}
  state: dispatched
  pr: null
- id: L3
  title: Finite persistence and water field
  order: orders/L3.md
  ticket: null
  authority: world
  builder: worker
  branch: feat/grand-v4-runtime
  owns:
    - "crates/hex_world_runtime/**"
    - "crates/hex_world_contracts/**"
    - "docs/planning/waves/grand-v4/manifest.md (L3 state/pr only)"
  dispatch_blockers: []
  merge_blockers: []
  fences: []
  selector: {concerns: [full], full: true}
  evidence: motion-or-feel
  sizing: {model: inherited, effort: inherited}
  state: dispatched
  pr: null
```

## Ownership and hotspots
Lane paths above are exclusive. Root owns hex_core shared arena vocabulary, hex_game, tools/arena.py, shared docs, shader composition and Cargo coordination. Request an explicit seam before touching another lane. All lane commits remain identifiable. No worker stages another worktree. Manifest updates limited to own row; coordinator resolves composition.

## Territory
#223 feat/water-lab -> #222 wave/northern-archipelago -> dev. #220/#219 and #210-213 are historical overlapping ancestors; preserve and do not merge tips blindly. #196 lattice fusion is separate and untouched. Base was clean. GitHub fetch on Sep25 failed with unresolved pack deltas; use pinned available revisions and live GitHub metadata until repaired. Actual GitHub remote is github; origin is an old local checkout.

## Integration order
Source lanes start together. Root lands vocabulary first, then coherent world/runtime/gameplay commits, then shared app adapters, checkpoint UI and water presentation. Shared heavy Cargo target is serialized by root; workers may run pure Python or lightweight tests only with agreed target. Final candidate targets dev, no force-push or automatic old PR closure.

## Combined acceptance
Full-size measurable terrain and independent source scale; all five shrine orders/idempotent reward; valid/refused teleport; old maps unchanged; staged actor admission over loaded terrain; land/boat/glider/combat save restart and partial damage; interrupted save preserves old checkpoint; death/New Run; unload/reload and repeated circuits; windowless full map and named landmark frames; native movement/30-minute profiling remains separate. Focused iteration then full selector gate; failures are recorded, never relabeled green.

## Stop conditions
Resolve source conflicts, invalid shared facts, actual budget failures and failing exact checkpoints before declaring completion. Preserve approved size and behavior. Missing Linear does not block work.

## Injection log
- Sep25: user approved implementation; three isolated source lanes and root integration started.

## Close-out
Pending; no claim of runtime, CI, visuals, native performance, save acceptance or merge yet.
