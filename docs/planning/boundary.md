# The boundary

Where the **world owner** and the **gameplay owner** meet, and what each is still
asking of the other. Every entry is an **ask, not a demand**: a precise spec (types,
publisher/consumer, tests) plus a fallback the asking side ships with if it is
deferred — so nothing here blocks anyone.

For the settled picture — which contracts are live, reserved, or still asked for, and
where each is specified — see [contracts.md](../contracts.md). This file is the
detail behind the *asked* rows and the record of what has already been answered.

Background and evidence: [production-audit.md](production-audit.md); the corresponding
roadmap rows: [roadmap.md](roadmap.md).

The framing follows
[architecture.md](../architecture.md#ownership-cuts-both-ways): design inside a crate
belongs to its owner; these asks only extend the published component/message contract,
and each one says exactly where the boundary sits.

Formerly `map-asks.md`, when the world side was one crate. It now spans generation,
presentation, and perception, and the asks run in both directions.

## What the gameplay side commits to

The asks below are one half of the boundary. The other half is what gameplay promises
never to do, so the world owner can build against it without checking:

- **Never sample the renderer for a gameplay fact.** Physical lights, shadows,
  exposure, emissive materials, fog, and pixels are presentation. Gameplay illumination
  comes from the published projection or it does not exist.
- **Never import map-generator internals.** Plans, masks, edge contracts, liquid
  graphs, candidates, and repair metadata are private, and gameplay does not want them.
- **Never reconstruct world units.** No dividing by `level_height`, no inferring a
  run's extent from its `HexSpan`. Where gameplay needs a level, it asks for a level —
  which is why ask C exists rather than a workaround.
- **Never change an owned crate's behavior without its owner's review**, and land a
  shared-type change in its own commit before either side depends on it.

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

## C — Run bottoms (exact occupancy: casting legality, line-of-sight, cover)

**Need**: `needs_los` spells and cover want column occupancy. Tiles publish
their run's top (`TilePos`) and world extent (`HexSpan`) but not the run's
bottom **level**, and gameplay must not divide by `level_height` to recover
it (that reintroduces the dependency the split exists to prevent);
`Headroom` saturates, so occupancy can't be reconstructed exactly.

Initial spatial perception is deliberately obstruction-agnostic and does not need
this component. Gameplay lights are radial within one light domain; sight uses exact
horizontal and vertical bands.

> **Gameplay-side counter-note (please reconsider the deferral).** That reasoning holds
> for *sight*, but casting needs the same datum sooner, and for a different reason.
> [casting.md](../systems/casting.md) validates a cast against the voxels it would
> affect — is this voxel solid, is it empty enough to conjure into, is it somebody's
> supporting surface — and none of those are answerable without exact occupancy. That
> lands in **wave 3**, ahead of obstruction-aware sight.
>
> One component answers casting legality, conjuration placement, trajectory, cover, and
> pathing alike, using the existing published-data pattern rather than a new API
> surface. If it slips, the fallback below degrades sight gracefully but leaves casting
> legality with no honest implementation — the gameplay side would ship terrain magic
> that cannot tell rock from air.

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

## G — Declarative terrain impact (spells announce, the world arbitrates)

**Need**: a fireball should not have to know whether stone yields to fire. Today a
spell would have to say `TerrainEdit::Clear`, which is gameplay deciding an outcome for
a material it does not own — and it means every new material you add (worked stone,
ice, tree roots) needs a corresponding gameplay change to interact sensibly.

**Ask**: a second message beside `TerrainEdit`, type in `hex_core` (gameplay-side):

```rust
/// Identifies one announcement so its outcome can be matched to it (ask H).
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TerrainBatchId(pub u64);

/// An energetic effect announced over an exact voxel volume. The map decides
/// what each material does about it — including nothing.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct TerrainImpact {
    /// Dealt by gameplay; echoed back by the outcome message.
    pub batch: TerrainBatchId,
    /// Exact voxels the effect reaches. Gameplay owns this geometry.
    pub volume: Vec<TilePos>,
    /// Element name, resolved against `elements.ron`.
    pub element: String,
    /// Authored strength, from the spell's own content.
    pub power: u8,
}
```

**The map side is a response table** — (element, power, substance) → outcome — as
map-domain content you own and tune. `hex_map` already depends on `hex_assets`, so
reading element content needs no new crate edge.

This is what lets you decide, entirely on your side and at your own pace, whether fire
clears a tree, whether worked stone resists Earth magic, and what happens when a spell
hits water. Gameplay never encodes material physics; it only ever says *which voxels*
and *what kind of energy*.

The full model, including the invariant that **gameplay owns geometry and the world
owns materiality**, is [casting.md](../systems/casting.md).

**Fallback if deferred**: spells keep using `TerrainEdit::Set`/`::Clear` with outcomes
chosen gameplay-side, and terrain magic ships ignorant of materials — fireballs that
delete granite, and no interaction at all with trees or structures.

## H — Terrain edit acknowledgment

**Need**: under ask G the world decides outcomes, so gameplay does not know what
happened. Without an answer it cannot show the difference between shattered and
scorched, cannot log it, cannot record it in a save, and can never express a
conditional effect ("if the wall falls…"). This is the *only* channel back.

**Ask**: one message written after a batch is applied, type in `hex_core`:

```rust
/// What each announced voxel actually became.
#[derive(Message, Debug, Clone)]
pub struct TerrainEditOutcome {
    /// Correlates with the announcement that caused it.
    pub batch: TerrainBatchId,
    /// One entry per announced voxel: unchanged entries included.
    pub voxels: Vec<(TilePos, SubstanceId, SubstanceId)>, // pos, before, after
}
```

Unchanged voxels matter as much as changed ones — "the bedrock resisted" is the
outcome a player needs to see.

**Correlation**: `TerrainImpact` carries the `TerrainBatchId` gameplay dealt it (ask G)
and the outcome echoes it. `TerrainEdit` predates this and has no such field — if
conjuration outcomes should be correlated too, it needs the same addition, and that is
part of this ask rather than something assumed. If you would rather not touch
`TerrainEdit`, gameplay can correlate conjuration by voxel alone, since a unit may
only commit one cast at a time.

**Fallback if deferred**: presentation plays only the spell's own animation and never
reflects the world's answer; the save log records intent rather than result (see D2's
interaction — a replayed impact then depends on the response table not changing).

## I — Rebuild interior and domain metadata after edits

**Need**: breach a cave roof with a spell and the chamber below is open to the sky, but
nothing says so. To be precise about what already happens: `apply_terrain_edits`
(`crates/hex_map/src/grid.rs`) does maintain `InteriorRegions` across an edit by
calling `remove_roof_voxel` for each applied edit — so the *roof* projection stays
current. What is never re-derived is interior **membership**: the chamber's surfaces
keep the region they were generated with. `LightDomain` has no producer at all yet, so
there is nothing to correct there today — it is a reserved type waiting on the
perception lane.

The consequence still stands: once perception derives domains from that membership, a
chamber you blow open reads as interior and dark forever.

**Ask**: when V3's post-edit rebuild of private derived layers lands
([world-generation-v3.md](../systems/world-generation-v3.md) already reserves the right
to do this), please include interior and ambient-domain metadata in what it rebuilds.

**Fallback until then**: documented in [status.md](status.md) — a breached roof does
not admit daylight, and gameplay will not pretend otherwise.

## J — Sight tunables as settings

**Need**: `SightProfile::DEFAULT` hardcodes the 36/12/1 bands and the downhill rule in
`hex_core`. Every other tunable in the game lives in `assets/config/*.ron`, validated
at load and hot-reloadable, which is what makes playtesting a file edit.

**Ask**: a `perception.ron` when the perception system lands, on the same loader
pattern as `combat.ron`. **The numbers stay yours** — this is about where they live,
not what they are.

Note also that sight and spell range deliberately use *different* elevation rules:
sight gains one hex per four levels capped at six, spell range gains one per five,
uncapped. Sight is not reach, and they should be tuned apart.

**Fallback if deferred**: the constants stay in code and a tuning pass means a rebuild.

## K — Liquid diggability (a flag to check, not an ask for work)

`substances.ron` currently marks **water and lava as `diggable: true`**. Combined with
terrain magic that means a spell can clear the ground beneath a lake, and V3's water
"does not redistribute after terrain edits" — so the result is water hanging in the
air.

Not a request for hydraulics. Just a flag worth setting deliberately rather than
discovering: if liquids should be uneditable, the existing `diggable` mechanism already
says so.

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

**Scheduling note (gameplay side agrees, and has moved to suit).** Your rule that no
production save may depend on regenerating a V1/V2 seed makes D2 a *prerequisite* for
shipping saves rather than an optimization — and D1 a prerequisite for restoring an
edited world. Save and load has accordingly moved out of wave 3 and into wave 5
([roadmap.md](roadmap.md)), so this can land on your clock rather than blocking on it.
When it does land, ask H's outcome log is what makes a replayed impact reproducible
without pinning the response table's version.

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
- **Destructible features (trees, structures)**: *not* a separate ask. Ask G's
  announcement covers the voxels a feature occupies, so whether a tree burns or a wall
  falls is a row in your response table, decided whenever you want to decide it. The
  gameplay side ships with fireballs that leave forests standing and says so in
  [status.md](status.md) until you say otherwise.
- **A callable query API for terrain**: deliberately not asked for. `hex_map` is a leaf
  and should stay one; gameplay computing `Footing`, occupancy, and trajectories from
  published components is the same pattern that already works for movement. Ask C is
  the one datum missing from it.
