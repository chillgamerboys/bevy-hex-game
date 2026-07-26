# hex_map — context for AI agents

You are working in `crates/hex_map/`. This file is read automatically on every turn
in this directory. Read it before changing anything.

## What this crate owns

Everything about the map:

| File | Holds |
|---|---|
| `src/voxel.rs` | Voxel storage — `VoxelMap`, `Column`, and the run-merging |
| `src/generator.rs` | Terrain height generation. `HeightMap`, `HeightGenerator`, Perlin |
| `src/grid.rs` | Building the world and turning it into tile entities |
| `src/settings.rs` | Designer-facing settings, loaded from `assets/config/world.ron` |

Plus `assets/config/world.ron` and `assets/config/substances.ron`, both edited by a
non-programmer.

**Read [`docs/MAP_MODEL.md`](../../docs/MAP_MODEL.md) before changing the model.** It
explains the voxel representation and the rules everything else depends on.

## Your compile-time blast radius is bounded, deliberately

**Nothing depends on `hex_map` except the binary.** `hex_core`, `hex_assets`,
`hex_world` and `hex_units` cannot see it. Cargo enforces this — a `use hex_map::`
in any of them fails to compile.

The consequence is that those crates cannot import map internals. It is still
possible to break their runtime behaviour by publishing a wrong `TilePos`, `HexSpan`
or `Headroom`, which is why the component contract and visual checks below matter.

## The one contract you must keep

The rest of the game learns about terrain **through components on tile entities**,
never by reading anything defined here:

```rust
commands.spawn((
    HexTile,       // marker
    hex_coord,     // HexCoord  — which hex
    tile_pos,      // TilePos   — the run's TOPMOST SOLID VOXEL, not its base
    span,          // HexSpan   — the run's world extent
    substance,     // SubstanceId
    headroom,      // Headroom  — clear voxels above the run, 0 if buried
    Mesh3d(...), MeshMaterial3d(...), Transform { ... },
));
```

`hex_units` queries `(&TilePos, &HexSpan, &SubstanceId, &Headroom)` with
`With<HexTile>`. That is the entire read interface, and `TerrainEdit` is the entire
write interface.

### `Headroom` is not optional, and it is yours to get right

**Only the map can measure it.** A run carries its own extent but knows nothing about
what is stacked on it, so gameplay cannot work this out — it has to be told.

Count the clear voxels directly above the run's top, saturating at `MAX_HEADROOM`
(above a column's top the air is unbounded, so an uncapped count would not terminate):

```rust
let headroom = headroom_above(column, run.top);   // run.top is exclusive
```

Two things depend on it:

- **Zero means buried.** A run with something solid directly on top is inside a
  column, not a surface, and nothing can stand on it however solid it is.
- **Small means cramped.** A character is 2 levels tall by default
  (`levels_tall` in `player.ron`), so a one-voxel gap under a bridge is a wall to it
  and a corridor to something shorter.

Getting this wrong is the worst class of bug in this codebase — it renders perfectly
and errors nowhere. Publishing headroom for every run as if it were exposed put the
player *inside* the terrain and left every route walking through the bedrock, arriving
nowhere. It shipped green across clippy, the whole test suite and every CI check.

**So: however you generate, store, or stream the map, spawn tiles carrying those
components and everything keeps working.** Replace `VoxelMap` wholesale if you want
to — nothing outside this crate references it.

### `TilePos` is the surface, not the base

A tile entity covers a **run** of voxels. Its `TilePos` must be the topmost solid one,
because that is what a piece standing there stands on. Tagging the base would force
gameplay to know `level_height` to work the surface out, which puts a dependency on
this crate straight back into movement — the exact thing the split prevents.

### Storage is not rendering

One entity per voxel would be ~25,000 at radius 20. The spawn pass merges vertical runs
of the same substance into one prism; measured, that is 3,400–4,100 entities at 60 FPS,
varying with the terrain seed.

Interior voxels therefore have **no entity**. That is why targeting is positional.

### The transform must agree with the span

A tile's `Transform` has to match the rendered run described by its span:

- `translation.y == span.centre()`
- `scale.y == span.height()`

Gameplay reads `span.top` to place a piece on a surface. If the transform disagrees,
pieces float or sink and **nothing errors** — the tiles still render. There is a test
for this (`tests/spawning.rs::every_tile_transform_matches_its_span`); keep it
passing.

## Voxels, columns, and the rule about them

A column is a list of substances indexed by level, from the bedrock floor up. Anything
above the top is air; air *inside* is stored explicitly, and that is what a cave is.

- Ground: levels 0..n of solid substance
- A floating platform: solid voxels with air beneath them
- A bridge over ground: one column, solid low down, air, then solid again — which the
  spawn pass renders as **two entities**

**Surfaces stacked within the same column are not connected.** A piece on a bridge
cannot step down to the ground beneath it; reaching it means a ramp across adjacent
coordinates descending a level at a time, or an ability that explicitly bypasses the
rule.

> **Never key anything by `HexCoord` in a way that collapses a stack.** Keeping only
> the highest surface silently makes every lower one unreachable, and a piece crossing a
> bridge teleports to the ground.

`TilePos::neighbours` deliberately excludes above and below. `TilePos::level_step_to`
gives the level difference a step rule compares against.

## Invariants you must not break

The tests enforce these; they are here so you know *why*.

- **Generated terrain is solid from level 0 up.** Digging needs something to dig
  through, and a column starting above ground is a hole nothing can stand in.
- **Every column has at least one level above bedrock.** Bedrock is not diggable, so
  bare bedrock is a permanent hole.
- **Terrain edits preserve non-diggable voxels.** `Clear`, setting air, or replacing
  bedrock with another substance must all be rejected.
- **A tile's transform agrees with its span.** Otherwise pieces float or sink and
  *nothing errors*.
- **Air is never spawned as a prism.**
- **A buried run reports zero headroom.** Anything else makes gameplay treat the
  inside of a column as a place to stand.

## Rules that will block your commit

CI runs `cargo clippy --workspace --all-targets --all-features -- -D warnings`.

**`#[allow(...)]` is banned.** If you cannot satisfy a lint, you must write:

```rust
#[expect(clippy::some_lint, reason = "why this is genuinely fine here")]
```

Silencing a warning without a reason is not available to you. This is deliberate.

Also denied: `unwrap()`, `panic!`, `todo!`, `unimplemented!`, slice indexing
(`v[0]` — use `.get()`, `.first()`, or destructure), `dbg!`, `println!` (use Bevy's
`info!`/`warn!`), comparing floats with `==`, and any undocumented public item.

Tests may unwrap, expect, panic, debug and print because failure is their job.
Slice indexing and the other restriction lints remain denied there.

## Scheduling: where your systems go

Systems that build the world run on `OnEnter(Screen::Gameplay)`, in one of:

```rust
GameplaySetup::Resources   // generate and insert VoxelMap
GameplaySetup::Terrain     // spawn tiles — needs Resources to have run
GameplaySetup::Actors      // hex_units's, not yours; needs tiles to exist
```

**Do not put tile spawning outside `Terrain`.** Systems in one `OnEnter` schedule run
in *unspecified order* unless a set says otherwise, and the set boundary is also what
supplies the sync point — entities created via `Commands` are not queryable until the
queue is applied. Both halves matter; ordering alone is not enough. This has caused a
real bug (the player spawned before the tiles existed and sank into the ground).

Clean up on `OnExit(Screen::Gameplay)`. There is a test that nothing leaks.

## Things that fail silently here

A clean log is not evidence a change worked. **Look at the window.**

| Symptom | Cause |
|---|---|
| Plain blue window | Assets not found — run through `cargo`, never the binary directly |
| Tiles in the wrong place, no error | Transform disagrees with the span |
| Stuck on "loading…" during initial startup | `world.ron` failed to parse. The terminal names the line |
| Terrain differs every run | `seed: None` in `world.ron`. Set a number to reproduce a map |
| Tile scaled to nothing | A zero-height span. `HexSpan::new` refuses these; check you used it |
| Digging removes an entity instead of adding one | The run was one voxel tall. Only clearing the *middle* of a taller run splits it |
| A piece floats above or sinks into terrain | The tile's `TilePos` is its base rather than its surface |
| A piece stands *inside* a column, and clicking does nothing | Buried runs were given non-zero `Headroom`, so gameplay took the bedrock for a surface |
| A piece refuses to walk somewhere that looks fine | Its `Headroom` is below the body's `levels_tall`. Check what is above it |

## Working here

```sh
cargo dev                      # run with inspector and live asset reload
cargo test -p hex_map          # fast; no GPU needed
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Editing `assets/config/world.ron` while `cargo dev` is running reloads it, but the
world is only rebuilt on entering gameplay — press `BACKSPACE` then `ENTER` to see
terrain changes.

### Putting pieces on your terrain

`assets/config/scenario.ron` holds two cube coordinates: where the player starts and
where the enemy starts. Move them onto the part of the map you want to try out, then
`BACKSPACE`/`ENTER` to rebuild.

This is the fastest way to answer questions the tests cannot: is that ridge actually
climbable, can a piece cross the bridge, is the far side of the valley reachable at
all. A coordinate that does not sum to zero, or that has no standable ground, falls
back to the centre of the map and says so in the terminal rather than failing to
start.

`HeightGenerator` implementations **must be pure**: the same coordinate must always
give the same height. Results are cached, so an impure generator produces terrain
that changes depending on what has been looked at.

## Before you finish

1. `cargo clippy --workspace --all-targets --all-features -- -D warnings`
2. `cargo test --workspace`
3. **Run the game and look at it.** Every bug found in this codebase so far was found
   by a human looking at the window, not by CI.

## Where your work goes

**Branch off `dev`, and open your PR against `dev`. Never against `main`.**

```sh
git checkout dev && git pull
git checkout -b feat/your-thing
gh pr create --base dev        # <- the --base matters
```

`main` moves only when `dev` is promoted into it, after someone has played the game.
This is not ceremony: **CI cannot see anything**. A black sky, a gap between every
tile, a piece standing inside the terrain — all three have shipped here, green across
clippy, the whole test suite and every CI check. `dev` is where work is allowed to be
wrong until a person has looked at it.

`dev` is permanent. Delete your feature branch after it merges; never delete `dev`.

## Further reading

- [`docs/ONBOARDING.md`](../../docs/ONBOARDING.md) — start here if the vocabulary is new
- [`docs/CONTENT.md`](../../docs/CONTENT.md) — editing settings without code
- [`docs/ARCHITECTURE.md`](../../docs/ARCHITECTURE.md) — the whole crate graph and why
- [`CONTRIBUTING.md`](../../CONTRIBUTING.md) — house style
