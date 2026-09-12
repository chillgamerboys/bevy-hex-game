# hex_map — context for AI agents

You are working in `crates/hex_map/`. This file is read automatically on every turn
in this directory. Read it before changing anything.

## What this crate owns

Everything about the map:

| File | Holds |
|---|---|
| `src/voxel.rs` | Voxel storage — `VoxelMap`, `Column`, and the run-merging |
| `src/generator.rs` | The optional Perlin height field |
| `src/terrain.rs` | Pure Showcase and Perlin construction |
| `src/procedural.rs` / `src/procedural_v2/` | Frozen V1/V2 procedural oracles while V3 recipes migrate |
| `src/procedural_v3/` | Private semantic plans, patches, recipes, validation, selection, fingerprints, fallback, and exact projections |
| `src/grid.rs` | Map lifecycle, tile entities, rendering, and terrain edits |
| `src/settings.rs` | Validated designer-facing settings from a world file, e.g. `assets/config/world.ron` |

Plus the world schema/content under `assets/config/world.ron`,
`assets/config/worlds/`, and `assets/config/substances.ron`, all editable without
Rust.

**Read [`docs/systems/map.md`](../../docs/systems/map.md) before changing the model.** It
explains the voxel representation and the rules everything else depends on.

## Your compile-time blast radius is bounded, deliberately

**Nothing depends on `hex_map` except the binary.** `hex_core`, `hex_assets`,
`hex_world` and `hex_units` cannot see it. Cargo enforces this — a `use hex_map::`
in any of them fails to compile.

The consequence is that those crates cannot import map internals. It is still
possible to break their runtime behaviour by publishing a wrong `TilePos`, `HexSpan`
or `Headroom`, which is why the component contract and visual checks below matter.

## The one contract you must keep

The rest of the game learns about terrain through shared components on tile entities
plus exact shared resources; it never reads map-private storage or plans:

```rust
commands.spawn((
    HexTile,       // marker
    hex_coord,     // HexCoord  — which hex
    tile_pos,      // TilePos   — the run's TOPMOST MATERIAL VOXEL, not its base
    run_bottom,    // RunBottom — the run's LOWEST MATERIAL VOXEL
    span,          // HexSpan   — the run's world extent
    substance,     // SubstanceId
    headroom,      // Headroom  — clear voxels above the run, 0 if buried
    // Deliberately no scene, visibility, mesh, material, or picking components.
));
```

Gameplay may query `(&TilePos, &RunBottom, &HexSpan, &SubstanceId, &Headroom)` with
`With<HexTile>` and consumes exact projections such as `TraversalBlockers`. Disposable
`TerrainRenderBatch` entities own transforms, visibility, meshes, materials, and picking;
their pick metadata resolves a hit back to the exact logical run.
`MapAnchors`, `BiomeRegions`, `InteriorRegions`, and view hints use the same shared,
stack-safe pattern. `TerrainEdit` and `TerrainImpact` are the two live write
interfaces: direct edits replace or clear material, while impacts resolve numeric
toughness and publish one ordered `TerrainImpactOutcome` per batch. `DamagedVoxels` is
the exact partial-health projection, not a visibility grant. PR #180 adds the live
paid spell emitter plus gameplay-owned correlation, settlement, and outcome consumer.

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
- **Small means cramped.** The canonical walker is exactly 2 levels tall, so a
  one-voxel gap under a bridge is a wall to it and a corridor to something shorter.

Getting this wrong is the worst class of bug in this codebase — it renders perfectly
and errors nowhere. Publishing headroom for every run as if it were exposed put the
player *inside* the terrain and left every route walking through the bedrock, arriving
nowhere. It shipped green across clippy, the whole test suite and every CI check.

**So: however you generate, store, or stream the map, spawn tiles carrying those
components and everything keeps working.** Replace `VoxelMap` wholesale if you want
to — nothing outside this crate references it.

### `TilePos` is the surface, not the base

A tile entity covers a **run** of one non-air substance. Its `TilePos` must be that
run's topmost material voxel. Gameplay combines the position with the substance's
`solid` flag before treating it as footing. Tagging the base would force gameplay to
know `level_height` to work the surface out, which puts a dependency on this crate
straight back into movement — the exact thing the split prevents.

`RunBottom` publishes the same run's lowest material voxel as an integer `Level`.
Together those two components make the inclusive voxel bounds exact without gameplay
reconstructing a level from `HexSpan`, world transforms, `level_height`, or saturated
`Headroom`.

### Storage is not rendering

One entity per voxel would be tens of thousands on a deep map. The spawn pass merges
vertical runs of the same substance into lightweight logical entities, then groups
bounded sets of those runs into disposable render batches. Logical run count stays
proportional to material bands rather than depth without placing those authoritative
facts in Bevy's transform, visibility, culling, or picking graphs.

Interior voxels therefore have **no entity**. That is why targeting is positional.

### Rendered geometry must agree with the logical span

A logical tile has no `Transform`; its exact world-space run is reconstructed from its
coordinate and span:

- horizontal centre: `HexCoord::to_world(span.centre())`
- vertical extent: `span.bottom..span.top`

Gameplay reads the logical components, while `TerrainRenderBatch` builds vertices from
that same geometry and owns the scene transform and visibility. If the batch geometry
disagrees with the logical span, pieces float or sink and **nothing errors** — the
terrain still renders. The presentation contract tests prove logical runs remain
scene-free and that render batches cover each run exactly once; keep them passing.

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

- **Every ground foundation is solid from level 0 to its surface.** Intentional
  features may then add non-solid water or a floating bridge with air beneath it.
- **Every column has at least one level above bedrock.** Bedrock is not diggable, so
  bare bedrock is a permanent hole.
- **Terrain edits preserve non-diggable voxels.** `Clear`, setting air, or replacing
  bedrock with another substance must all be rejected.
- **Rendered batch geometry agrees with each logical tile's coordinate and span.**
  Otherwise pieces float or sink and *nothing errors*.
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
GameplaySetup::Actors      // hex_units', not yours; needs tiles to exist
GameplaySetup::Perception  // illumination and knowledge; needs actors to exist
GameplaySetup::View        // frame the completed geometry and actors
GameplaySetup::Finalize    // hex_game verifies terrain and required actors
```

**Do not put tile spawning outside `Terrain`.** Systems in one `OnEnter` schedule run
in *unspecified order* unless a set says otherwise, and the set boundary is also what
supplies the sync point — entities created via `Commands` are not queryable until the
queue is applied. Both halves matter; ordering alone is not enough. This has caused a
real bug (the player spawned before the tiles existed and sank into the ground).

Clean up on `OnExit(Screen::Gameplay)`. There is a test that nothing leaks.

The terrain-durability contract configures the reserved update protocol
`TerrainSystems::ApplyWorld → TerrainSystems::RefreshProjections →
TerrainSystems::ReconcileActors → TerrainSystems::ConsumeOutcomes` before perception.
`ApplyWorld` remains the map-owned phase that flushes rebuilt tile facts and
outcomes. PR #180 fills the gameplay-owned refresh, deterministic actor settlement,
authority adoption, and matching-outcome consumption phases under cross-crate ordering
contracts.

## Things that fail silently here

A clean log is not evidence that presentation is correct, so inspect static frames for
camera framing/occlusion and rendered geometry, materials, lighting, cutaways, seams,
and composition; use video/human checks for camera motion, input response, control
feel, and taste. Those artifacts may show how hook-established state is rendered but
never prove map or gameplay logic when typed hooks or contracts can assert it; add the
missing hook instead.

| Symptom | Cause |
|---|---|
| Plain blue window | Assets not found — run through `cargo`, never the binary directly |
| Tiles in the wrong place, no error | Render-batch geometry disagrees with the logical coordinate or span |
| Stuck on "loading…" during initial startup | A settings file failed to parse -- for terrain, whichever world the chosen scenario names. The terminal names the file and line |
| Perlin terrain differs every run | Its preset has `seed: None`. Set a number to reproduce it |
| Tile scaled to nothing | A zero-height span. `HexSpan::new` refuses these; check you used it |
| Digging removes an entity instead of adding one | The run was one voxel tall. Only clearing the *middle* of a taller run splits it |
| A piece floats above or sinks into terrain | The tile's `TilePos` is its base rather than its surface |
| A piece stands *inside* a column, and clicking does nothing | Buried runs were given non-zero `Headroom`, so gameplay took the bedrock for a surface |
| A piece refuses to walk somewhere that looks fine | Its `Headroom` is below the body's traversal-profile height. Check what is above it |

## Working here

```sh
cargo dev                      # run with inspector and live asset reload
python3 tools/test_scope.py run map_unit
python3 tools/test_scope.py run map_generation
python3 tools/test_scope.py run map_contracts
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Editing `assets/config/world.ron` while `cargo dev` is running reloads it, but the
world is only rebuilt on entering gameplay — press `BACKSPACE`, then click its
scenario to see terrain changes.

`terrain::build_non_procedural_map` and the versioned procedural builders are pure:
settings and their explicit generation inputs go in, and a complete semantic plan or
`VoxelMap` comes out. Keep ECS resources, commands, and rendering out of them.

`HeightGenerator` implementations used by the optional Perlin preset must also be
pure. Results are cached, so an impure generator produces terrain that changes
depending on what has been looked at.

## Before you finish

1. `cargo clippy --workspace --all-targets --all-features -- -D warnings`
2. `python3 tools/test_scope.py check-partitions map`
3. Run the selected map concerns from `python3 tools/test_scope.py plan`.
4. Inspect the game only for affected geometry, materials, lighting, composition, or
   motion. Logical map claims must already be proved through typed contracts.

## Where your work goes

**Branch off `dev`, and open your PR against `dev`. Never against `main`.**

```sh
git checkout dev && git pull
git checkout -b feat/your-thing
gh pr create --base dev        # <- the --base matters
```

`main` moves only when `dev` is promoted into it, after someone has played the game.
This is not ceremony: typed hooks cannot judge a black sky, a gap between every tile,
or native presentation quality. Those visual claims need human eyes; map and gameplay
logic still require typed evidence.

`dev` is permanent. Delete your feature branch after it merges; never delete `dev`.

## Further reading

- [`docs/development/onboarding.md`](../../docs/development/onboarding.md) — start here if the vocabulary is new
- [`docs/development/config.md`](../../docs/development/config.md) — editing settings without code
- [`docs/architecture.md`](../../docs/architecture.md) — the whole crate graph and why
- [`CONTRIBUTING.md`](../../CONTRIBUTING.md) — house style
