# How the map works

The world is made of **voxels**: hex prisms stacked in columns, each one made of some
substance. This describes the model, the vocabulary that goes with it, and the rules
that everything else depends on.

If you only want to change how the terrain looks, [CONTENT.md](CONTENT.md) is shorter.

## The pieces

**A coordinate** — `HexCoord` — is a hex on the flat plane, in cube coordinates.

**A level** is how far up. Level 0 is the bedrock floor.

**A position** — `TilePos { coord, level }` — is one voxel. This is how anything in the
world is addressed.

**A substance** — `SubstanceId` — is what a voxel is made of: stone, dirt, grass, air.
The list lives in `assets/config/substances.ron`.

> **The vertical axis is always called `level`.** Never `z`. Cube coordinates already
> use `x`, `y` and `z`, and all three are horizontal — `HexCoord::z()` returns
> `-x - y`. Two different `z`s in one coordinate system produce bugs that are silent
> and geometric.

## The rules the rest of the game depends on

### Stacked columns are not connected

A piece on a bridge **cannot step down** to the ground beneath it. Getting down means
walking a ramp of adjacent columns that descends one level at a time, or using an
ability that explicitly bypasses the rule — a teleport, a tunnel.

This is a design decision, and it means a position is a `TilePos`, never a `HexCoord`.
Two columns sharing a coordinate are unrelated places that happen to share an address.

**Never key anything by `HexCoord` in a way that collapses a stack.** A
`HashMap<HexCoord, f32>` keeping "the highest column" silently makes every lower column
unreachable, and a piece crossing a bridge teleports to the ground. That abstraction
existed briefly and was deleted rather than fixed — one that *can* express the
forbidden thing eventually will.

### One level is one step

A step is legal when the destination is an adjacent column and its surface is within
**one level**. Because levels are integers, that is `step.abs() <= 1` — no epsilon, no
accumulated float error. This is the concrete payoff for quantising the vertical axis.

### Terrain is not guaranteed connected

There can be places you cannot walk to. `route` returns `Option`, and "no route exists"
is a real answer that callers handle rather than an error.

### Bedrock is not diggable

It is the floor of the world. Every column has at least one level above it, so a column
of bare bedrock — a permanent hole nothing could dig through — cannot be generated.

## How it is stored

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

**One entity per voxel would be about 25,000 entities.** Instead the spawn pass merges
vertical runs of the same substance into a single prism, so a fifteen-level stone column
is one entity. Measured: **3,481 entities at 60 FPS**.

Two consequences:

- **A tile entity is a run, not a voxel.** Its `HexSpan` covers the whole run.
- **Interior voxels have no entity.** The rock two levels inside a cliff — exactly what
  a tunnelling spell targets — is addressable only by `TilePos`. This is why targeting
  is positional rather than entity-based.

A tile is tagged with the `TilePos` of its **topmost solid voxel**: the thing a piece
standing there stands on. Tagging the base instead would force gameplay to know the
level height to work the surface out, which would put a dependency on the map back into
movement.

## What each crate sees

```
hex_core     TilePos, HexSpan, SubstanceId, TerrainEdit — the vocabulary
hex_assets   the substance table
hex_map      voxel storage, generation, rendering — nothing else can see this
hex_gameplay reads tiles; cannot see hex_map
```

The map talks to the rest of the game **only through components on tile entities**:

```rust
(HexTile, HexCoord, TilePos, HexSpan, SubstanceId, Mesh3d, ...)
```

`hex_gameplay` queries those. It never reads `VoxelMap` or any generator, so terrain
storage and generation can be replaced wholesale — chunked, streamed, generated
differently — without anything else noticing.

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
| A tile's `TilePos` is its **surface** | the topmost solid voxel, not the base |
| Air is never spawned | so an air-filled cave is a gap between two entities |
| A tile's transform must agree with its span | otherwise pieces float or sink, and **nothing errors** |
| Clearing a one-voxel run **removes** an entity | only clearing the middle of a taller run adds one |
| Digging above the top does nothing | there is nothing there to remove |
| Building above the top leaves a gap | that is how a floating platform is made |

## What is deliberately not decided

- **What a spell does.** `TerrainEdit` can express digging and building; which spells
  exist, what they cost, and what they target is game design.
- **Pathfinding.** `route` walks a straight line and gives up when blocked.
  `hexx::a_star`, `field_of_view` and `field_of_movement` are already compiled in and
  are the obvious basis, once there is a movement-cost model.
- **Anything about turns.** There is no turn system yet.
- **Whether stacked columns ever connect.** Teleport and tunnel are named in the design
  but not implemented. When they are, they belong in `hex_gameplay` as explicit
  exceptions to the step rule, not as changes to it.
