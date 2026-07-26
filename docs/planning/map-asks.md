# Map contract asks

Written for the map's owner. Each entry is an **ask, not a demand**: a
precise spec (types, publisher/consumer, tests) plus a fallback the gameplay
side ships with if the ask is deferred — so nothing here blocks anyone.
Contract vocabulary lands in `hex_core`/`hex_assets` (gameplay-side work);
the map-side work is stated per ask. Background and evidence:
[production-audit.md](production-audit.md); the corresponding roadmap rows:
[roadmap.md](roadmap.md).

The framing follows
[ARCHITECTURE.md](../ARCHITECTURE.md#ownership-cuts-both-ways): design
inside the map crate is yours; these asks only extend the published
component/message contract, and each one says exactly where the boundary
sits.

## Already delivered by PR #52 — nothing left to ask

- **Seed contract** (was ask E): `ResolvedMapSeed`, per-scenario
  `generation_seed`, session rerolls, seed snapshotted at scenario click.
- **Generator versioning** (was ask D3): `MapSettings.generator_version`,
  mixed into the seed stream. One save-relevant consequence worth agreeing
  on: a version bump intentionally re-terraforms same-seed worlds, so
  regen-based save restoration is version-fragile *by design* — which is
  what makes D2 below the primary save format rather than an optimization.
- **Shared vocabulary the asks below build on**: `TraversalProfile`/
  `TraversalProfiles`, `MapAnchorId`/`MapAnchors`,
  `SpecialMovementRegion(s)`, `TerrainReady`, `ScenarioPlacement::Anchor`,
  and the snow/ice/basalt/lava substances.

## A′ — Movement classes (now via traversal profiles)

**Need** ([DESIGN.md](../DESIGN.md#map)): swamp passable only to some
units, lava only to flying ones, water to swimmers.

**Shape after PR #52**: future movement modes become additional
`TraversalProfileId`s beside `WALKER`, and substances gain footing tags —
a gameplay-side field on `Substance` in `hex_assets`, serde-defaulted so
`substances.ron` keeps parsing:

```rust
/// Movement modes that may treat this substance as footing.
/// Default: ["ground"] when solid, [] otherwise.
#[serde(default)] pub footing_for: Vec<String>,
```

`Footing` then composes profile × substance class. `SpecialMovementRegions`
is already the map-side hook for fly/teleport-only areas.

**Map work: none now.** The eventual asks are content (swamp/lava entries in
`substances.ron` — proposals can come from the gameplay side; the file is
yours) and agreement on `SpecialMovementRegion` semantics when the first
ability that enters one lands. **Fallback: not needed — ships without map
work.**

## B — Named regions (anti-magic fields, lit zones)

**Need**: painted areas that are part of a terrain's identity — an
anti-magic field where evocations fail, a lit courtyard feeding visibility.
Distinct from `SpecialMovementRegion`, which is deliberately opaque and
unstable across maps; these are **named**, content-addressable regions the
two mechanisms should be coordinated with rather than duplicated.

**Ask**: `world.ron` grows an optional authored section (shape yours —
coordinate lists or center+radius, validated in `MapSettings::validate` like
everything else there), and at spawn the map tags member tiles with a
component defined in `hex_core` (gameplay-side):

```rust
/// Names of authored world regions this tile belongs to.
#[derive(Component, Reflect, Debug, Clone, Default)]
pub struct RegionTags(pub Vec<String>);
```

**Publisher**: map, at tile spawn. **Consumers**: casting rules in
hex_combat (a `region_rules` table in `combat.ron` maps names to
deny-evocation/deny-enchantment), visibility later. One mechanism, many
painted uses.

**Fallback if deferred**: encounter files carry the same shape as a
gameplay-owned overlay keyed by `HexCoord` — works, just makes anti-magic
encounter data instead of world data.

## C — Run bottoms (exact line-of-sight and cover)

**Need**: `needs_los` spells and cover want column occupancy. Tiles publish
their run's top (`TilePos`) and world extent (`HexSpan`) but not the run's
bottom **level**, and gameplay must not divide by `level_height` to recover
it (that reintroduces the dependency the split exists to prevent);
`Headroom` saturates, so occupancy can't be reconstructed exactly.

**Ask**: one more component on the existing spawn bundle (type in
`hex_core`, gameplay-side):

```rust
/// The run's lowest material voxel. Its topmost is the entity's TilePos.
#[derive(Component, Reflect, Debug, Copy, Clone, PartialEq, Eq)]
pub struct RunBottom(pub Level);
```

You already hold both bounds when merging runs in the spawn pass — this is
one insert plus one line in the spawning test.

**Fallback if deferred**: a documented approximation — a sight line is
blocked iff some intervening column's highest run top reaches it. Wrong only
for shooting *under* bridges and overhangs; `needs_los` content ships with
that caveat.

## D1 — Pre-spawn terrain edit replay

**Need**: save restoration (and encounter-authored terrain — a pre-broken
bridge, a dug trench) replays recorded `TerrainEdit`s onto a freshly
generated world. Sending them as ordinary messages works today but costs one
visible full-grid respawn after first spawn.

**Ask**: a resource defined in `hex_core` (gameplay-side), drained by the
map right after the voxel map is built and **before** tiles first spawn:

```rust
/// Terrain edits to apply to a freshly generated world before tiles spawn.
/// Written by save-restore and encounters; drained by the map during setup.
#[derive(Resource, Debug, Default, Clone)]
pub struct PendingTerrainEdits(pub Vec<TerrainEdit>);
```

Replay then costs zero respawns, independent of (and compatible with) any
later per-column re-mesh optimization for live edits. Validation is the same
as the message path, so rejected edits re-reject identically.

**Fallback if deferred**: replay as ordinary `TerrainEdit` messages on the
first gameplay frame — correct, with one respawn flash.

## D2 — Terrain snapshot (the generator-proof save format)

**Need**: saves must not depend on generator code never changing. Any
generator tweak — and, by design, any `generator_version` bump — relocates
same-seed terrain, silently invalidating regen+replay restoration. A dump is
immune.

**Ask**: a request/response pair. Types in `hex_core` (gameplay-side);
serialization of the voxel map and consuming a provided snapshot instead of
generating are map-side:

```rust
/// Gameplay requests; the map answers by inserting the resource.
#[derive(Message, Debug, Clone, Copy)]
pub struct TerrainSnapshotRequest;

/// A generator-independent dump. Substances BY NAME — ids are session-local
/// (the table assigns them from sorted names), so a saved id is meaningless.
#[derive(Resource, Debug, Clone)]
pub struct TerrainSnapshot {
    pub names: Vec<String>,                 // index -> substance name
    pub columns: Vec<(HexCoord, Vec<u8>)>,  // per column, per level, index into names
}
```

On load, gameplay inserts the snapshot (same shape) and the map consumes it
during setup *instead of* generating. Size is trivial — a radius-12 world is
roughly 15 KB before compression. This makes the dump the primary save
format; seeded regen + edit replay becomes an optimization rather than a
correctness requirement.

**Fallback until it lands**: saves record `(seed, generator_version)` and,
on mismatch at load, offer "restart this area" instead of drifting silently.
Gameplay-side tests refuse to mark a world savable unless its seed is
explicit.

## F — Deliberate non-asks

- **Streaming / bigger maps**: nothing now. The map shape question (open
  world vs hub vs chapters) is deliberately open, and chunking would settle
  it by accident. Multiple world files per scenario already work; the
  contract keeps `VoxelMap` private, so a streamed rewrite later changes no
  consumer. The only request: keep it that way, and share the practical
  `grid_radius` ceiling you observe so validation can state it.
- **Unit obstruction / occupancy**: gameplay-side (hex_combat), not a map
  concern.
- **Anchor constants**: raised in the PR #52 review rather than here — if
  well-known anchor ids (`party_start`, `bridge`, …) become consts on
  `MapAnchorId`, scenarios and encounters can reference every anchor the
  generator already publishes.
