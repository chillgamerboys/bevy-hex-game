# Map contract asks

Written for the map's owner. Each entry is an **ask, not a demand**: a
precise spec (types, publisher/consumer, tests) plus a fallback the gameplay
side ships with if the ask is deferred — so nothing here blocks anyone.
Contract vocabulary lands in `hex_core`/`hex_assets` (gameplay-side work);
the map-side work is stated per ask. Background and evidence:
[production-audit.md](production-audit.md); the corresponding roadmap rows:
[roadmap.md](roadmap.md).

The framing follows
[architecture.md](../architecture.md#ownership-cuts-both-ways): design
inside the map crate is yours; these asks only extend the published
component/message contract, and each one says exactly where the boundary
sits.

## V3 publication rule

V3 replaces the assumption that gameplay may need generic access to generator
semantics. `GeneratedWorldPlan`, patch masks, edge contracts, liquid graphs,
feature plans, structure plans, recipe names, and repair metadata remain private
to `hex_map`.

The contracts-first PR reserves the exact projections V3 may publish:

- `BiomeRegions` maps exact `TilePos` values to map-local `BiomeRegionId`s;
- `TraversalBlockers` names exact otherwise-standable surfaces occupied by
  generated features such as tree roots;
- a generated light is an entity with an exact `TilePos` and `GameplayLight`;
  its exterior or interior `LightDomain` is derived at use time;
- `PresentationOcclusion` composes independent fog, interior-cutaway, and
  canopy-cutaway reasons without making any one system the owner of Bevy
  `Visibility`.

These contracts carry consequences, not instructions. Gameplay can ask whether a
known surface is blocked or which biome region it belongs to, but it cannot ask the
map how that tree was sampled, which candidate produced the region, or how to repair
the liquid graph.

V3 implementation and delivery are specified in
[world-generation-v3.md](../systems/world-generation-v3.md). The map publishes
these resources only when the corresponding V3 layer lands; the contracts-first
PR itself changes no runtime behavior.

## Delivered by the procedural map pipeline — nothing left to ask

PR #52 landed on `dev` on 2026-07-26 and settled two of these asks outright:

- **Seed contract** (was ask E): `ResolvedMapSeed`, per-scenario
  `generation_seed`, session rerolls, seed snapshotted at scenario click.
- **Generator versioning** (was ask D3): `MapSettings.generator_version`,
  mixed into the seed stream. One save-relevant consequence worth agreeing
  on: a version bump intentionally re-terraforms same-seed worlds, so
  regen-based save restoration is version-fragile *by design* — which is
  what makes D2 below the primary save format rather than an optimization.
- **Shared vocabulary the asks below build on**: `TraversalProfile` (the
  standability and step predicates, shared by generated-map validation and
  live movement), `MapAnchorId`/`MapAnchors`, `SpecialMovementRegion(s)`,
  `TerrainReady`, `GameplaySetupFailure`, the terminal
  `GameplaySetup::Finalize` phase, `ScenarioPlacement::Anchor`, and the
  snow/ice/basalt/lava substances.

Those are historical V1/V2 contracts, not a promise to preserve either generator
indefinitely. V3 uses its own versioned streams while V1/V2 remain frozen review
oracles; both legacy implementations are removed after active scenarios migrate.
No production save format may depend on regenerating a V1 or V2 seed.

## A′ — Movement classes (now via traversal profiles)

**Need** ([the design](../design/game.md#map)): swamp passable only to some
units, lava only to flying ones, water to swimmers.

**Shape now that the pipeline has landed**: future movement modes become
additional `TraversalProfile` values beside the canonical walker, and
substances gain footing tags — a gameplay-side field on `Substance` in
`hex_assets`, optional so `substances.ron` keeps parsing unchanged:

```rust
/// Movement modes that may treat this substance as footing.
///
/// Absent in the file means "derive it": `["ground"]` when the substance is
/// solid, empty otherwise. `Option` rather than `#[serde(default)]` because a
/// field default cannot see `solid` — the fallback is applied in `validate()`,
/// alongside the rest of the substance-table checks, and callers read the
/// resolved value.
#[serde(default)] pub footing_for: Option<Vec<String>>,
```

`Footing` then composes profile × substance class. `SpecialMovementRegions`
is already the map-side hook for fly/teleport-only areas.

**Map work: none now.** The eventual asks are content (swamp/lava entries in
`substances.ron` — proposals can come from the gameplay side; the file is
yours) and agreement on `SpecialMovementRegion` semantics when the first
ability that enters one lands. **Fallback: not needed — ships without map
work.**

## B — Named rule regions (anti-magic fields)

The old proposal combined biome identity, lighting, and future anti-magic rules
in one `RegionTags` tile component. V3 deliberately does not implement that shape:

- biome identity uses exact `BiomeRegions`;
- illumination uses ambient domains and exact `GameplayLight` sources;
- optional movement regions retain their existing exact contract.

Anti-magic is still a valid future need, but its policy and overlap semantics are
not designed. Revisit a content-addressable rule-region resource when the first
region-sensitive spell is implemented. Do not overload `BiomeRegionId`, infer it
from materials, or add a generic “lit” tag in the meantime.

**Fallback if deferred**: the first encounter needing anti-magic may carry an
encounter-owned exact `TilePos` overlay. No current system depends on it.

## C — Run bottoms (future exact line-of-sight and cover)

**Need**: `needs_los` spells and cover want column occupancy. Tiles publish
their run's top (`TilePos`) and world extent (`HexSpan`) but not the run's
bottom **level**, and gameplay must not divide by `level_height` to recover
it (that reintroduces the dependency the split exists to prevent);
`Headroom` saturates, so occupancy can't be reconstructed exactly.

Initial spatial perception is deliberately obstruction-agnostic and does not need
this component. Gameplay lights are radial within one light domain; sight uses exact
horizontal and vertical bands. Add `RunBottom` only with the later obstruction-aware
line-of-sight or cover work, not as a prerequisite for V3 visibility.

**Ask**: one more component on the existing spawn bundle (type in
`hex_core`, gameplay-side):

```rust
/// The run's lowest material voxel. Its topmost is the entity's TilePos.
#[derive(Component, Reflect, Debug, Copy, Clone, PartialEq, Eq)]
#[reflect(Component)]
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
This fallback is development-only while V1/V2 are still present. Gameplay-side
tests refuse to mark a world savable unless its seed is explicit, and shipped
procedural saves wait for the generator-independent snapshot rather than extending
legacy generator lifetime.

## F — Deliberate non-asks

- **Streaming / chunks**: `Ring7` is one radius-33 map, not a streaming
  decision. Keep `VoxelMap` private so a later chunked rewrite changes no
  consumer. Record generation time, entity count, and perception recomputation
  for the composite before choosing a streaming model.
- **Unit obstruction / occupancy**: gameplay-side (hex_combat), not a map
  concern.
- **Anchor constants**: raised in the PR #52 review rather than here — if
  well-known anchor ids (`party_start`, `bridge`, …) become consts on
  `MapAnchorId`, scenarios and encounters can reference every anchor the
  generator already publishes.
