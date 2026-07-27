# How the map works

The world is made of **voxels**: hex prisms stacked in columns, each one made of some
substance. This describes the model, the vocabulary that goes with it, and the rules
that everything else depends on.

If you only want to change how the terrain looks, [development/config.md](../development/config.md) is shorter.

## The pieces

**A coordinate** — `HexCoord` — is a hex on the flat plane, in cube coordinates.

**A level** is how far up. Level 0 is the bedrock floor.

**A position** — `TilePos { coord, level }` — is one voxel. This is how anything in the
world is addressed.

**Headroom** — `Headroom(Level)` — is how many clear voxels sit above a tile's surface.
Zero means it is buried inside a column.

**A substance** — `SubstanceId` — is what a voxel is made of: stone, dirt, grass, air.
The list lives in `assets/config/substances.ron`.

> **The vertical axis is always called `level`.** Never `z`. Cube coordinates already
> use `x`, `y` and `z`, and all three are horizontal — `HexCoord::z()` returns
> `-x - y`. Two different `z`s in one coordinate system produce bugs that are silent
> and geometric.

## The rules the rest of the game depends on

### Stacked surfaces are not connected

A piece on a bridge **cannot step down** to the ground beneath it. Getting down means
walking a ramp of adjacent surfaces that descends one level at a time, or using an
ability that explicitly bypasses the rule — a teleport, a tunnel.

This is a design decision, and it means a position is a `TilePos`, never a `HexCoord`.
There is one `Column` per coordinate, but separate material runs within it are
unrelated positions that happen to share a horizontal address. Only runs whose
substance is solid can become places to stand.

**Never key anything by `HexCoord` in a way that collapses a stack.** A
`HashMap<HexCoord, f32>` keeping only the highest surface silently makes every lower
surface unreachable, and a piece crossing a bridge teleports to the ground. That abstraction
existed briefly and was deleted rather than fixed — one that *can* express the
forbidden thing eventually will.

### One level is one step, when the body fits

Ordinary movement uses one complete predicate: `TraversalProfile::admits_transition`.
Both endpoints must be solid, have enough headroom, occupy adjacent `HexCoord`s, and
stay within the profile's climb and drop limits. The canonical walker is two levels
tall and can climb or drop one level.

Two individually standable endpoints still may not connect. The clear volumes above
them must overlap laterally for the body's full height:

```text
min(from.level + from.headroom, to.level + to.headroom)
    - max(from.level, to.level)
    >= levels_tall
```

For example, a one-level ramp with exactly two clear levels above each endpoint has
only a one-level shared aperture. A two-level walker cannot pass its lintel; the lower
endpoint needs a third clear level. Live movement, pathfinding, AI, and V2 validation
all use this same transition predicate. The position-only `admits_step` remains for
frozen V1 generator compatibility.

Because levels are integers, there is no epsilon or accumulated float error. This is
the concrete payoff for quantising the vertical axis.

### A surface has to have room above it

Every tile reports its **headroom**: how many clear voxels sit directly above it,
saturating at `MAX_HEADROOM`. Two things fall out of one number.

**Zero headroom means buried.** A column is several stacked runs — bedrock under dirt
under grass — and only the top of a contiguous stack is a surface. The rest are inside
the column and nothing can stand on them however solid they are.

**Small headroom means cramped.** A body's traversal profile declares how tall it is.
The canonical walker is exactly two levels tall, so a one-voxel gap under a bridge is
a crawlspace: passable to something smaller, a wall to an ordinary person. Terrain
being walkable is a property of the traversal profile, not of the terrain.

Only the map can measure this. A run knows its own extent but nothing about what is
stacked on it, so `hex_map` counts it at spawn and publishes it; gameplay cannot
work it out from spans.

> This is the map's half of a contract nothing else can check. Marking buried runs as
> having room put the player *inside* the terrain and left every route walking through
> the bedrock — and it rendered perfectly, with a clean log and a green test suite.

### Terrain is not guaranteed connected

There can be places you cannot walk to. `route` returns `Option`, and "no route exists"
is a real answer that callers handle rather than an error.

### Optional regions use exact surface metadata

A generator can publish an optional area through `SpecialMovementRegions`, which maps
each exact `TilePos` in the area to a `SpecialMovementRegion`. The resource is the sole
source of truth; tile entities do not duplicate this membership. This is generic
metadata: it does not name the recipe that created the area or decide whether flight,
swimming, tunnelling, or some future ability can enter it.

Membership is keyed by `TilePos`, not `HexCoord`, so a bridge and the ground beneath it
can belong to different regions. Region numbers are deterministic only within one
generated map; they are grouping labels, not persistent IDs to compare across maps or
seeds.

Ordinary traversal remains geometry-driven. Solidity, headroom, adjacency, climb, and
drop decide whether a walker can move; adding or removing a region tag does not change
those rules. Generation validates that tagged optional surfaces are outside the
ordinary network and that critical anchors are not tagged.

### Bedrock is not diggable

It is the floor of the world. `substances.ron` marks it non-diggable, and the map
rejects terrain edits that would replace or clear a non-diggable voxel. Every generated
column also has at least one level above it, so bare bedrock cannot become a permanent
hole in the walkable surface.

## How it is stored

### V2 plans occupied volumes first

`TerrainVolumePlan` is V2's recipe-independent semantic model before voxelization.
Every coordinate in the map footprint has one `VolumeColumn` containing ordered,
non-overlapping occupied intervals:

- `SolidMass` intervals support surfaces and may identify a cutaway interior.
- `NonSolidFill` intervals hold visible hazards such as water or lava but cannot
  support footing.
- Gaps between occupied intervals are implicit air.

This represents ground plus floating islands, cave floors plus roofs, bridges, and
hazards without recipe-specific storage exceptions. Validation rejects missing
columns, invalid or overlapping intervals, incompatible materials, and malformed
interior references before the plan becomes a `VoxelMap`.

Hills is the first shipped V2 recipe. Its compatibility adapter evaluates the frozen
V1 candidates unchanged, then losslessly lifts the selected map and its exact anchors,
optional regions, and tactical metadata into `TerrainVolumePlan` for final
materialization. Equivalent V1 and V2 Hills settings therefore select the same
candidate and retain the same map fingerprint. New recipes build the volume directly;
V1 remains loadable for saved-seed compatibility.

Layered Sky Islands consumes that finalized Hills selection before it samples any
`sky.*` stream. Eight native upper-layer candidates append floating solid masses and
two-wide metal lanes without changing a ground column prefix, ground surface, anchor,
interior, river crossing, or protected approach. The selected upper footprint is
15–25% of map columns, has at least eight empty levels below it, and is published as
one exact flight-gated `SpecialMovementRegion`. Its combined `MapViewHint` frames both
ground play and the upper network. The shipped eight-level-clearance path remains
frozen. Higher-clearance settings add independently varied island footprints,
walkable terraces, and tapered stone underbodies while preserving the configured
empty gap beneath the deepest point.

Mountains supports the frozen shipped ridge as well as expanded massifs. Expanded
candidates raise slightly more than half of the map above the base, use a meandering
edge-to-edge spine with multiple long branches, and distribute five through seven
non-collinear peaks across varied summit levels. The ordinary network keeps a
two-wide high pass and a separated two-wide low bypass; the rest of the range
deliberately retains cliff edges. It has no hazard fill or crossing material. Only
summit components that are actually disconnected under the shared walker predicate
are published as special-movement terrain, while the generated view looks across the
range so both routes remain legible.

Caves keeps a playable rocky surface above one rooted underground network. A two-wide
open entrance descends one level per row to six through twelve chambers joined by
two-wide critical corridors. The shipped six-through-eight-room path remains frozen.
Larger networks add walkable sloped corridors, selected loop connections, varied
floor and ceiling levels, larger chambers, and additional surface relief. Every
corridor retains at least three clear levels, every chamber at least four, and
cutaway roofs remain at least three solid levels thick. The party anchor names the
surface entrance while the hostile anchor names the deepest main chamber; both are
validated on the same exact walker graph as live movement.

Every exposed upward solid boundary has exactly one `SurfaceMetadata` entry keyed by
its full `TilePos`. It classifies that exact surface as ordinary, special-movement, or
non-standable and may associate it with an interior. Anchors also name exact
`TilePos`s, so stacked surfaces at one `HexCoord` never become interchangeable.

Each interior records its exact floor and entrance surfaces plus the air intervals
that must remain clear. Roof masses identify their `InteriorRegionId`; voxelization
publishes every exact authored roof voxel through `InteriorRegions`. The grid splits
its disposable material runs wherever cutaway membership changes and projects
`CutawayOccluder(region)` onto the resulting roof segments. Digging through a roof
therefore preserves both surviving fragments without transferring the tag to replacement
material. A cutaway tag does not remove or make terrain transparent, change voxel
storage, or change traversal. Cave presentation may hide tagged opaque roof segments
locally while leaving adjacent untagged columns visible as walls.

The plan also publishes a `MapViewHint` so camera setup can frame the generated
geometry after terrain and actors exist. V1 keeps its frozen single-height plan and
hashing behavior for compatibility; only V2 uses this volume contract.

### Runtime storage remains voxel columns

A column is a list of substances, indexed by level from the floor up. Anything above
the top is air, so empty sky costs nothing. Air *inside* a column is stored explicitly —
that is what a cave is.

```
level 5   air          ← above the top, not stored
level 4   grass
level 3   dirt
level 2   air          ← a cave, stored explicitly
level 1   stone
level 0   bedrock
```

**Run-length storage was considered and rejected.** Compressing a uniform column to one
entry saves memory, but destruction is the common operation: as flat voxels it is a
single assignment, where a run model has to split an entry, preserve substance on both
sides, and merge neighbours back when they match. At this scale the memory difference is
under a megabyte and the correctness difference is what matters.

## Storage is not rendering

This is the part worth understanding before changing anything.

**One entity per voxel would be tens of thousands of entities on a deep map.** Instead
the spawn pass merges vertical runs of the same substance into a single prism, so a
fifteen-level stone column is one entity. The rendered entity count therefore follows
the number of substance bands rather than the number of stored voxels.

Two consequences:

- **A tile entity is a run, not a voxel.** Its `HexSpan` covers the whole run.
- **Interior voxels have no entity.** The rock two levels inside a cliff — exactly what
  a tunnelling spell targets — is addressable only by `TilePos`. This is why targeting
  is positional rather than entity-based.

A tile is tagged with the `TilePos` of its run's **topmost material voxel**. Gameplay
then combines that position with the substance's `solid` flag before treating it as
footing; a water run is rendered but is not standable. Tagging the base instead would
force gameplay to know the level height to work the surface out, which would put a
dependency on the map back into movement.

## What each crate sees

```
hex_core     HexTile, HexCoord, TilePos, HexSpan, SubstanceId, Headroom,
             TraversalEndpoint, TraversalProfile, SpecialMovementRegion,
             SpecialMovementRegions, InteriorRegionId, InteriorRegions,
             CutawayOccluder, MapViewHint, TerrainEdit
             — the shared vocabulary
hex_assets   the substance table
hex_map      voxel storage, generation, rendering — nothing else can see this
hex_units reads tiles; cannot see hex_map
```

The map exposes rendered footing through components on tile entities:

```rust
(HexTile, HexCoord, TilePos, HexSpan, SubstanceId, Headroom, Mesh3d, ...)
```

Exact optional-region memberships live in the `SpecialMovementRegions` resource keyed
by `TilePos`; they are not duplicated on tile entities. Exact interior floors and
cutaway roof voxels likewise live in `InteriorRegions`; only rendered segments projected
from those roof voxels receive the `CutawayOccluder` component needed by live
presentation queries. `hex_units` queries the footing components. It never reads
`VoxelMap` or any generator, so terrain storage and generation can be replaced wholesale
— chunked, streamed, generated differently — without anything else noticing.

**Writing** goes the other way, through a message:

```rust
TerrainEdit::Set { pos, substance }
TerrainEdit::Clear { pos }
```

Gameplay cannot call into `hex_map`, so a spell that digs or builds writes one of these
and the map applies it. That is the whole write path.

## Things that are true and easy to forget

| | |
|---|---|
| A tile entity covers a **run**, not a voxel | its `HexSpan` may be many levels tall |
| A tile's `TilePos` is its **run surface** | the topmost material voxel, not the base |
| Headroom of 0 means **buried** | solid, but inside a column and not standable |
| A one-voxel gap under a bridge is **not** a corridor | a 2-level body does not fit; a 1-level one does |
| Two standable endpoints do not guarantee a step | the shared lateral aperture can still be too short |
| Cutaway metadata names exact opaque roof voxels | rendering projects them onto disposable run segments |
| Air is never spawned | so an air-filled cave is a gap between two entities |
| A tile's transform must agree with its span | otherwise pieces float or sink, and **nothing errors** |
| Clearing a one-voxel run **removes** an entity | only clearing the middle of a taller run adds one |
| Digging above the top does nothing | there is nothing there to remove |
| Building above the top leaves a gap | that is how a floating platform is made |

## What is deliberately not decided

- **What a spell does.** `TerrainEdit` can express digging and building; which spells
  exist, what they cost, and what they target is game design.
- **What a step costs.** `hex_units::movement::Reach` searches over `TilePos` and
  charges one per step, so the map's substances do not yet affect how far a piece
  gets. **`hexx::a_star` cannot supply that model**, despite being compiled in: it
  keys on `Hex` alone, so it cannot tell a bridge from the ground beneath it.
- **Whether terrain takes a turn to change.** A turn order exists now — see
  [systems/combat.md](combat.md) — but `TerrainEdit` is applied the moment it
  arrives and costs nobody anything. Whether digging is an action is a design question.
- **Whether stacked surfaces ever connect.** Teleport and tunnel are named in the design
  but not implemented. When they are, they belong in `hex_units` as explicit
  exceptions to the step rule, not as changes to it.
