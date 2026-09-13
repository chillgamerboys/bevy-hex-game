# Northern Archipelago implementation

Status: implementation in progress, no acceptance claim.
Base: user-selected expedition `13fdb5aca0948e6364aaa193a9bc7b846161460e`.
Candidate: `wave/northern-archipelago`. Local work only; no dev/main or remote writes.
Topology: one wave, because the new world, shared surface renderer and exploration controller have one meaningful combined runtime checkpoint. Root is the integration owner. Existing island-biomes PR212 is frozen V3; PR220 is the earlier V4 foundation already incorporated. Neither branch tip is imported. PR213/219 remain separate. Linear reconciliation unavailable; no tickets created.

## Locked decisions

The user's approved Northern Archipelago plan is authoritative: approximately2400 world units across, exactly11 islands in3 clusters separated by300–500 water units; crater~300 high; cold snowy Viking landscape;4 buildings and1 field; dry bay spawn. No enemies/objectives/victory, no new fluid physics. Normal expedition player and spells, G glider, F collision-aware freeflight80/160. Ocean waves amplitudes1.2/.6/.2, wavelengths110/180/60, periods18/25/12; shared static liquid levels and dynamic visual surface. Water is indestructible; all solids carve; unloaded is never air. Original Forest package and running game stay untouched. No native automation or launch without user request; user owns motion/feel checks.

## Foundation

`ArenaMap::NorthernArchipelago`, `ArenaMapCapabilities`, compact-run `solid_at`, `ArenaResidency` and `ArenaAvailability`, `ArenaStreamInterest` establish shared vocabulary. Ocean presentation profile is world-owned and published by the water lane. Current legacy maps retain full-publication behavior. New map never constructs a whole-world VoxelMap or expanded voxel BTreeMap. Resident max512, rendered max256,2 IO workers/products; async V4 preparation/halos; coarse distant terrain and ocean are not collision authority. Sparse session edits survive unload and clear Restart.

## Queue

```yaml
lanes:
  - id: L1
    title: Terrain authoring and package
    order: orders/L1.md
    ticket: null
    authority: world
    builder: worker
    branch: feat/northern-geography
    owns:
      - "crates/hex_schematic/src/v4/ (new ocean/northern authoring only)"
      - "crates/hex_world_tool/ (northern authoring command only)"
      - "assets/config/v4/northern-archipelago/"
      - "tools/northern_*.py"
    dispatch_blockers: []
    merge_blockers: []
    fences: []
    selector: {concerns: [map_unit, map_generation, map_contracts, residual, clippy, docs, shipping], full: true}
    evidence: static-presentation
    sizing: {model: inherited, effort: inherited}
    state: queued
    pr: null
  - id: L2
    title: Ocean surface presentation
    order: orders/L2.md
    ticket: null
    authority: world
    builder: worker
    branch: feat/northern-ocean
    owns:
      - "crates/hex_map/src/ocean/"
      - "crates/hex_map/src/liquid_render/ (shared surface conversion)"
      - "crates/hex_world/src/battle_sky.rs (profile configuration only)"
      - "assets/shaders/ocean.wgsl"
    dispatch_blockers: []
    merge_blockers: []
    fences: []
    selector: {concerns: [map_unit, map_generation, map_contracts, residual, clippy, docs, shipping], full: true}
    evidence: motion-or-feel
    sizing: {model: inherited, effort: inherited}
    state: queued
    pr: null
  - id: L3
    title: Exploration controller
    order: orders/L3.md
    ticket: null
    authority: gameplay
    builder: worker
    branch: feat/northern-flight
    owns:
      - "crates/hex_arena/"
      - "crates/hex_core/src/arena/exploration.rs (availability helpers only)"
    dispatch_blockers: []
    merge_blockers: []
    fences: []
    selector: {concerns: [map_unit, map_generation, map_contracts, residual, clippy, docs, shipping], full: true}
    evidence: motion-or-feel
    sizing: {model: inherited, effort: inherited}
    state: queued
    pr: null
  - id: L4
    title: Streamed world adapter
    order: orders/L4.md
    ticket: null
    authority: world
    builder: worker
    branch: wave/northern-archipelago
    owns:
      - "crates/hex_map/src/arena/streamed/"
      - "crates/hex_world_runtime/ (stream-safe carve overlay)"
      - "crates/hex_map/src/arena.rs (stream admission wiring)"
    dispatch_blockers: []
    merge_blockers: [L1]
    fences: []
    selector: {concerns: [map_unit, map_generation, map_contracts, residual, clippy, docs, shipping], full: true}
    evidence: motion-or-feel
    sizing: {model: inherited, effort: inherited}
    state: queued
    pr: null
  - id: L5
    title: Application composition
    order: orders/L5.md
    ticket: null
    authority: shared
    builder: worker
    branch: wave/northern-archipelago
    owns:
      - "crates/hex_game/"
      - "tools/arena.py"
      - "docs/planning/waves/northern-archipelago/"
    dispatch_blockers: []
    merge_blockers: [L1, L2, L3, L4]
    fences: []
    selector: {concerns: [map_unit, map_generation, map_contracts, residual, clippy, docs, shipping], full: true}
    evidence: motion-or-feel
    sizing: {model: inherited, effort: inherited}
    state: queued
    pr: null
```

## Acceptance

First prove full scale and final ocean mesh with all silhouettes, tallest peak and one dressed sector before further decoration. Exact island/cluster/shore/bed/spawn facts come from typed generation output. Streaming queries, carve reload/reset, movement boundaries, pause and legacy regression checks are logic evidence. Fresh windowless whole-map/bay/settlement/snow/waterline captures establish static visuals. User's three short checks establish waves, fast crossing/reversal and walking/water/freeflight feel. Measure3 circuits; no memory growth after warmup; bounded queues; streaming/publication target<2ms CPU p95, wave-on/off frame regression<10%. Repository-selected combined gate once assembled; inherited V3 lint remains explicit.
