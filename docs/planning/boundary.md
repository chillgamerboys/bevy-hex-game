# The boundary

Where the **world owner** and the **gameplay owner** meet. This records both accepted
cross-owner work and what each side is still asking of the other. An **ask** is not a
demand: it includes a precise spec (types, publisher/consumer, tests) and a fallback
the asking side ships with if it is deferred. An **accepted prerequisite** is
different: dependent work waits for it rather than silently shipping against an
approximation.

For the settled picture — which contracts are live, agreed, reserved, or still asked
for, and where each is specified — see [contracts.md](../contracts.md). This file is
the detail behind the *asked* rows and the record of what has already been answered.

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

## What the world side commits to

The same restraint applies in the other direction:

- **Publish consequences, never generator instructions.** Gameplay may consume exact
  surfaces, blockers, biome membership, illumination, and knowledge; it never needs a
  patch plan, feature candidate, repair action, or liquid graph.
- **Keep every spatial projection stack-safe.** Published facts are keyed by exact
  `TilePos` whenever level matters. A horizontal `HexCoord` must not collapse a bridge,
  cave floor, and ground surface into one answer.
- **Never hide gameplay policy in presentation.** A rendered object, semantic part,
  palette swatch, canopy, or material appearance does not become blocking, damaging,
  visible, or interactive by implication. Those are separate explicit contracts.
- **Never change gameplay-owned behavior without its owner's review.** A world PR may
  add isolated shared vocabulary first, but adapters in `hex_units` or `hex_combat`
  remain gameplay work.

## V3 publication rule

V3 replaces the assumption that gameplay may need generic access to generator
semantics. `GeneratedWorldPlan`, patch masks, edge contracts, liquid graphs,
feature plans, structure plans, recipe names, and repair metadata remain private
to `hex_map`.

The contracts-first PR introduced the exact projections V3 may publish:

- `BiomeRegions` maps exact `TilePos` values to map-local `BiomeRegionId`s;
- `TraversalBlockers` names exact otherwise-standable surfaces occupied by
  generated features such as tree roots;
- a generated light is an entity with an exact `TilePos` and `GameplayLight`;
  its exterior or interior `LightDomain` is derived at use time;
- `PresentationOcclusion` composes independent fog, explicit review-cutaway, and
  Character-camera proximity reasons; reason-producing systems never write Bevy
  `Visibility` independently, and one shared presentation pass applies their combined
  result. `TreeOccluder` and `TreeFadeAmount` carry whole-tree camera opacity without
  exposing feature plans.

These contracts carry consequences, not instructions. Gameplay can ask whether a
known surface is blocked or which biome region it belongs to, but it cannot ask the
map how that tree was sampled, which candidate produced the region, or how to repair
the liquid graph.

V3 implementation and delivery are specified in
[world-generation-v3.md](../systems/world-generation-v3.md). V3 now publishes biome
membership, feature blockers, and generated cave gameplay-light entities. Authored
crystal objects and restrained physical lights present those sources without changing
their headless authority.

## Shared presentation is a third role, not a loophole

`hex_objects` and `hex_editor` are shared presentation/tooling. They own how authored
objects are validated, edited, and drawn, but they own no world or gameplay semantics.
The split for a Forest tree is therefore explicit:

- Forest publishes its authored `ObjectInstance`, exact rotated roots in
  `TraversalBlockers`, and one stack-safe whole-tree identity;
- `hex_objects` renders the object, propagates the supplied root to every chunk, and
  preserves authored canopy membership as separate, currently unconsumed art
  metadata; it never derives traversal from object parts;
- gameplay consumes the blocker and knowledge projections, never the renderer.

This permits authored Forest visuals without moving generation into presentation or
letting a `root`/`trunk` label become an accidental collision contract. Spell-created
objects follow the same rule: gameplay may request a shared visual instance, while any
simulation effect remains in gameplay/world contracts.

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
  `GameplaySetup::Finalize` phase, `EncounterPlacement::Anchor` (called
  `ScenarioPlacement::Anchor` until HEX-14 replaced the two-coordinate scaffold
  with encounter rosters — same anchor mechanism, same guarantee), and the
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

**Delivered**: material-sensitive trajectories, future sight, and cover want column
occupancy. Every material-run
entity now publishes its inclusive top (`TilePos`) and bottom (`RunBottom`) alongside
its world extent (`HexSpan`). Gameplay does not divide by `level_height` or infer from
the saturated `Headroom` clearance fact. The published bounds feed one gameplay-owned
exact occupancy projection and deterministic trajectory supercover; faction-facing
trajectory choices filter that geometry through authorized knowledge, while full truth
stays at command authority. Obstruction-aware sight remains later work and must reuse
the primitive rather than introduce a second ray.

Initial spatial perception is deliberately obstruction-agnostic and does not need
this component. Gameplay lights are radial within one light domain; sight uses exact
horizontal and vertical bands.

That reasoning holds for *sight*, but casting still needs the same datum, and for a
different reason. [casting.md](../systems/casting.md) validates a cast against the
voxels it would affect — is this voxel solid, is it empty enough to conjure into, is it
somebody's supporting surface — and none of those are answerable without exact
occupancy. Wave 3 deliberately shipped terrain effects fail-closed rather than
reconstructing it; `RunBottom` has now unblocked permanent construction and remains
the foundation for obstruction-aware trajectories and sight.

One component answers casting legality, conjuration placement, trajectory, cover, and
pathing alike, using the existing published-data pattern rather than a new API surface.

**Accepted and live contract**: one component extends the existing spawn bundle.
The type lives in gameplay-owned `hex_core`:

```rust
/// The run's lowest material voxel. Its topmost is the entity's TilePos.
#[derive(Component, Reflect, Debug, Copy, Clone, PartialEq, Eq)]
#[reflect(Component)]
pub struct RunBottom(pub Level);
```

You already hold both bounds when merging runs in the spawn pass. Every run entity,
including stacked runs under bridges, overhangs, and caves, carries it. Spawn-bundle
tests assert the exact inclusive bottom and top for each such run.

**Publication is live:** the shared type and map adapter land together while gameplay
consumers remain downstream. Terrain casting did wait for this contract; it does not
reconstruct occupancy or ship terrain effects that cannot distinguish rock from air.
Obstruction-aware sight may still use its independent approximation while its consumer
waits: a sight line is
blocked iff some intervening column's highest run top reaches it. Wrong only
for shooting *under* bridges and overhangs.

## G — Declarative terrain damage (accepted contract)

**Need**: a spell must not decide whether a material can be damaged or how much health
it has. Gameplay owns the exact volume and authored power; the world owns material
toughness, protection, accumulated damage, and destruction.

The shared announcement remains a second message beside `TerrainEdit`:

```rust
pub struct TerrainBatchId(pub u64);

pub struct TerrainImpact {
    pub batch: TerrainBatchId,
    pub volume: Vec<TilePos>,
    pub element: ElementId,
    pub power: u8,
}
```

The publisher allocates a session-unique batch id and emits a nonempty `volume` sorted
by `TilePos` with duplicates removed. `power` is authored and strictly positive.
`TerrainBatchId` and `ElementId` are transient handles, never authored or durable save
identities. Empty, noncanonical, zero-power, unknown-element, and reused-batch
announcements receive the explicit rejection in contract H rather than disappearing.

### Initial toughness and damage rule

`Substance::toughness: Option<u8>` is the voxel's maximum health. `None` means the
substance does not participate in damage; authored values are restricted to the fixed
initial scale `1`, `2`, `4`, and `8`:

| Maximum health | Initial substances |
|---:|---|
| 1 | grass, snow |
| 2 | dirt, gravel, ice |
| 4 | stone, basalt |
| 8 | worked stone, metal |
| none | air, water, lava, bedrock |

`assets/config/terrain_damage.ron` is a world-owned allow-list of stable
`(element_name, substance_name)` pairs. A listed pair permits damage; a missing pair
resists. The initial file lists every current element against every substance with a
numeric toughness, so the first slice tests the Boolean contract without pretending
to settle elemental balance. Validation rejects unknown names, duplicate pairs, and
pairs naming a substance without toughness. The resolved matrix participates in the
same coherent content revision and deterministic fingerprint as elements, substances,
spells, and lattices; a failed reload retains the last complete accepted revision.

For an allowed, unprotected voxel, effective damage is exactly `power`,
capped at its remaining health. A voxel with no sparse damage entry starts at its
material maximum. A positive remainder produces `Damaged`; zero destroys the voxel
and produces `Destroyed`. There are no thresholds, multipliers, healing, material
replacement, or elemental transformations in this slice.

The map processes direct `TerrainEdit`s before impacts, then impacts in message order;
later work observes all earlier changes. An accepted material-changing `Set` drops old
damage and gives the new material full health, while `Clear` drops it. A same-material
`Set` remains a no-op and does not heal. Partial damage changes no material and does
not require a terrain rebuild; any actual creation or destruction uses the existing
single consequence/rebuild path, at most once per update.

The existing protections remain authoritative. Non-diggable material, authored V3
liquid voxels and their protected lower supports, blocking feature roots, and generated
light protection resist without accumulating damage. Damage never redistributes a
liquid or affects a non-voxel feature.

Feature destruction is outside this contract. V3 trees and structures are semantic
instances rather than substance voxels and need their own occupancy, response, and
acknowledgment contract.

**Status**: the message vocabulary and this policy are reserved/agreed; spell emission,
world resolution, content, and outcome consumption are not live yet.

## H — Damage acknowledgment, health projection, and settlement (accepted contract)

The map produces exactly one answer for every impact it processes:

```rust
pub struct TerrainVoxelHealth {
    pub remaining: u8,
    pub maximum: u8,
}

pub enum TerrainImpactDisposition {
    NoMaterial,
    Resisted,
    Damaged,
    Destroyed,
}

pub struct TerrainVoxelOutcome {
    pub pos: TilePos,
    pub disposition: TerrainImpactDisposition,
    pub before: Option<SubstanceId>,
    pub after: Option<SubstanceId>,
    pub health_before: Option<TerrainVoxelHealth>,
    pub health_after: Option<TerrainVoxelHealth>,
}

pub enum TerrainImpactRejection {
    EmptyVolume,
    NonCanonicalVolume,
    ZeroPower,
    UnknownElement,
    ReusedBatch,
    TerrainUnavailable,
}

pub enum TerrainImpactResult {
    Applied(Vec<TerrainVoxelOutcome>),
    Rejected(TerrainImpactRejection),
}

pub struct TerrainImpactOutcome {
    pub batch: TerrainBatchId,
    pub result: TerrainImpactResult,
}
```

An applied result contains exactly one entry for every announced voxel, in the same
canonical order. `NoMaterial` is `None → None` with no health. `Resisted` preserves
one material; toughness-bearing material reports equal valid health before and after,
while material without toughness reports neither. `Damaged` preserves one material
and reports the same maximum with `0 < after.remaining < before.remaining`.
`Destroyed` is `Some → None`, reports valid `health_before`, and has no
`health_after`; zero is therefore never represented as live voxel health. Every
`TerrainVoxelHealth` satisfies `1 <= remaining <= maximum`.

A rejected result contains no per-voxel payload and mutates nothing. Validation uses
this fixed precedence: `ReusedBatch`, `EmptyVolume`, `NonCanonicalVolume`, `ZeroPower`,
`UnknownElement`, then `TerrainUnavailable`. The first processed use of a batch id
consumes it whether applied or rejected; a later use is therefore `ReusedBatch`.
`TerrainUnavailable` covers a missing/not-ready map or a missing coherent damage
catalog. Gameplay keeps a cast pending until it receives either answer, so rejection
cannot deadlock turn advancement.

### Partial-health projection and privacy

`DamagedVoxels` is a stack-safe `TilePos → TerrainVoxelHealth` projection containing
only voxels with partial health. `hex_map` is its sole publisher. Absence means full
health or no toughness; destruction and a material-changing direct edit remove the
entry. The projection is authoritative world truth, **not permission to reveal it**.

The shared presentation adapter may draw a health bar only for an entry that is also a
current exposed top surface, currently `Observed` by the Player faction, and visible
after ordinary cutaway/tile and camera culling. Remembered, Unknown, buried, internal,
side-only, full-health, and destroyed voxels expose no bar. Bars are small depth-tested,
camera-facing world billboards so terrain can occlude them normally. Logs and spell
presentation independently filter outcomes through faction knowledge.

### Cross-owner ordering and unsupported actors

`TerrainSystems::{ApplyWorld, ReconcileActors}` reserves the cross-crate update order.
`ApplyWorld` applies edits/impacts, rebuilds material consequences, publishes outcomes,
and flushes rebuilt tile facts. `ReconcileActors` then refreshes gameplay occupancy and
settles unsupported actors before illumination, observation, knowledge publication,
or another combat action. An impact emitted during combat apply on frame N therefore
settles at the next `ApplyWorld`; combat does not advance past its pending batch before
the outcome and actor reconciliation complete.

Gameplay owns settlement because the map never reads units. After destruction, every
unit whose exact support is no longer legal is handled in stable `UnitId` order:

1. Select the highest legal, unoccupied surface strictly below the old support in the
   same column.
2. If none exists, consider lateral legal surfaces in the deterministic tuple
   `(hex_distance, absolute_level_difference, is_higher, TilePos)`, where lower or
   same-level candidates sort before higher candidates at equal distance/difference.
   Higher surfaces are allowed.
3. Apply the body's headroom/traversal rules, exact blockers, current unit occupancy,
   and destinations reserved for earlier units in this same pass.
4. Cancel stale movement/animation and update `StandsOn`, transform, occupancy, and
   combat authority together. Falling deals no damage, spends no movement/action, and
   does not change turn ownership.
5. If no landing exists anywhere, stop combat with a fatal settlement diagnostic;
   never leave the unit on air or silently despawn it.

`TerrainEdit` still has no batch acknowledgment. Conjuration correlation is not
inferred from voxel position; it receives a separate contract if one is needed.

**Status**: the shared vocabulary/order is reserved and the behavior is agreed; map,
presentation, and gameplay producers/consumers remain pending.

## I — Interior domains after edits (initial ruling)

Material changes rebuild the same terrain, headroom, blocker, illumination,
observation, and knowledge projections used for an originally generated map. Existing
`InteriorRegions` roof voxels stay current as pieces are removed.

Interior **membership**, however, remains authored V3 metadata and is not re-derived.
A breached cave therefore stays in its authored Interior light domain and does not gain
new daylight in this initial implementation. No aperture-size, connectivity,
local-daylight, or repaired-roof rule is implied. That more dynamic model is separate
future work rather than a condition on terrain damage.

## J — Sight tunables as settings

**Need**: `SightProfile::DEFAULT` hardcodes the 36/12/1 bands and the downhill rule in
`hex_core`. Every other tunable in the game lives in `assets/config/*.ron`, validated
at load and hot-reloadable, which is what makes playtesting a file edit.

**Delivered**: `perception.ron` provides three validated, hot-reloadable profiles on
the same loader pattern as `combat.ron`. The world owner owns the values and the
gameplay owner reviews any shared loader-infrastructure change. **The numbers stay
yours** — this is about where they live, not what they are.

Note also that sight and spell range deliberately use *different* elevation rules:
sight gains one hex per four levels capped at six, spell range gains one per five,
uncapped. Sight is not reach, and they should be tuned apart.

`SightProfile::DEFAULT` remains the headless-test compatibility fallback; gameplay
uses the validated active profile.

## K — Liquid edit policy (accepted conservative admission rule)

`substances.ron` currently marks **water and lava as `diggable: true`**. That flag
allows a direct edit of a liquid voxel; setting it to `false` would reject that direct
edit. It does **not** protect a diggable support voxel beneath the liquid and does not
repair V3's private steady-state flow topology after either edit. Merely flipping the
flag therefore cannot prevent hanging liquid or stale current/fall metadata.

**Accepted policy**: until a topology-aware rebuild exists, `hex_map` must reject
changes to authored V3 liquid voxels and every lower voxel in the same column while a
retained authored liquid run remains above. Its private classifier is keyed by exact
`TilePos` and returns all affected stacked runs. Rejection is atomic: neither
occupancy nor current/fall metadata changes, and liquid never redistributes.

The same classifier applies to `TerrainImpact`: authored liquid voxels and their
protected lower supports report `Resisted` and never acquire partial health. Water and
lava have no toughness in the initial damage model. Breaking a nearby ordinary voxel
does not trigger flow, refill, current propagation, or any other fluid simulation.

This does not make water or lava globally non-diggable. Legacy and non-topological
liquids remain governed by their existing `diggable` material behavior. The exact
classifier and conservative runtime admission are live for authored V3 liquid
topology. Topology-aware clearing or rebuilding may replace this conservative rule
later, but must update occupancy and all derived flow metadata in one operation.

## L — Conjurable substance admission

**Need**: spell content names the substance it creates, but existence is not permission.
Treating every defined substance as conjurable would let an otherwise valid spell
create protected bedrock, static water, or lava merely because the world needs those
materials for generation.

**Live contract**: the world-owned `conjurable` policy in substance content is checked
by the gameplay-owned content loader for every construction spell reference. The
initial file marks only `stone` conjurable; every other current substance, including
bedrock, air, water, and lava, is not. Ordinary construction also requires its complete
creation volume to be empty before it emits `TerrainEdit::Set`, so this slice creates
stone in air and never replaces an existing material.

This is a spell-content admission rule, not a new meaning for the low-level
`TerrainEdit::Set` message. Save restoration and authored terrain may still need to set
any valid substance. Runtime cast legality consumes only spell content that already
passed the cross-domain check.

**Accepted by the gameplay owner**, including the initial exclusions. This supersedes
an earlier gameplay-side ruling that any defined substance was conjurable and balance
would live in a spell's cost and tier. That ruling was aimed at *balance* whitelists —
gating an interesting material because it is strong — and this is a different concern:
world integrity. Conjured bedrock would be an indestructible wall; conjured liquid
creates the hanging-water problem ask K has not solved. The palette stays the world
owner's to widen, and gating a material purely because it is powerful remains a balance
decision that belongs in cost and tier.

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
production save may depend on regenerating a V1/V2 seed still makes D2 a
*prerequisite* for durable saves rather than an optimization — and D1 a prerequisite
for restoring an edited world. Wave 5's pre-alpha resume slot is a deliberately
disposable exception: it records an explicit seed, generator version, and content
digests, then refuses drift instead of migrating or silently rebuilding a different
world ([roadmap.md](roadmap.md)). It never claims production compatibility and does
not save combat. D1 and D2 therefore remain asked without blocking that scaffold.
When they land, contract H's outcome log is what makes a replayed impact reproducible
without pinning the damage table's version.

## F — Deliberate non-asks

- **Streaming / chunks**: Ring7 and Ring19 are finite maps, not a streaming
  decision. Ring7 remains one radius-33 world; Ring19 is one radius-55,
  9,241-column world with 19 regions, 42 internal seams, and 30 outer boundary
  sides. Keep `VoxelMap` private so a later chunked rewrite changes no consumer.
  Record generation time, entity count, and perception recomputation for both
  composites before choosing a streaming model.
- **Unit obstruction / occupancy**: gameplay-side (hex_combat), not a map
  concern.
- **Anchor constants**: raised in the PR #52 review rather than here — if
  well-known anchor ids (`party_start`, `bridge`, …) become consts on
  `MapAnchorId`, scenarios and encounters can reference every anchor the
  generator already publishes.
- **Destructible features (trees, structures)**: deliberately not folded into contract G.
  `TerrainImpact` covers material voxels; V3 features are semantic instances with
  separate blocker/canopy projections, and authored object parts explicitly carry no
  gameplay meaning. The first feature-damaging spell therefore needs its own exact
  occupancy, response, and acknowledgment contract. Until that need is scheduled,
  fireballs leave forests and structures standing, as [status.md](status.md) records.
- **A callable query API for terrain**: deliberately not asked for. `hex_map` is a leaf
  and should stay one; gameplay computing `Footing`, occupancy, and trajectories from
  published components is the same pattern that already works for movement. Ask C is
  the one datum missing from it.
