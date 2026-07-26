# Context for Claude Code

A hex-grid game on **Bevy 0.19**, organised as a seven-crate cargo workspace.

Read **[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)** first — it explains the crate
graph and, more usefully, the reasoning behind it. This file is the operational
summary.

## Tech stack

| Crate | Version | Notes |
|---|---|---|
| `bevy` | `0.19` | |
| `hexx` | `0.24` | **No Bevy features.** Pins only `glam`, so it can never gate a Bevy upgrade. `a_star`, `field_of_view` and `field_of_movement` are compiled in but unused |
| `bevy-inspector-egui` | `0.37` | Targets bevy 0.19. Isolated in `hex_dev`, `dev` feature only |
| `ron` / `serde` | | Designer-facing settings |
| `xxhash-rust`, `rand` | | Terrain hashing |

Toolchain pinned to **Rust 1.97.1** in `rust-toolchain.toml`. Bevy 0.19's MSRV is
1.95, so this isn't optional — an older stable fails to build the dependency tree
with errors that don't obviously point at the toolchain.

## Build & run

```
cargo dev            # inspector + live asset reload
cargo run --release  # as it ships
```

**Always run through cargo.** `BEVY_ASSET_ROOT` is set in `.cargo/config.toml`
because in a workspace `CARGO_MANIFEST_DIR` is the *binary crate's* directory —
without it the game looks in `crates/hex_game/assets/`, finds nothing, and renders
a plain blue window with only `Path not found` in the log.

## Workspace

```
hex_core → hex_assets → {hex_map, hex_world, hex_gameplay, hex_dev} → hex_game
```

**`hex_map`, `hex_world` and `hex_gameplay` must not depend on each other.** Shared
types go in `hex_core`. Cargo enforces this; a violating `use` fails to compile.

**`hex_map` is a leaf** — nothing depends on it but the binary. It is owned by one
person, and the map reaches the rest of the game only through `HexCoord` and
`HexSpan` components on tile entities. See `crates/hex_map/CLAUDE.md`.

`hex_core` depends on Bevy sub-crates rather than the `bevy` facade, so it builds
and tests without a renderer. It holds the largest share of the test suite (34 of 94).

## Conventions

- **Modules expose `pub fn plugin(app: &mut App)`**, not a `Plugin` struct.
- **Each plugin registers its own reflected types.** Never a central list.
- **`AppSystems`** (`TickTimers → RecordInput → Update`) orders `Update`;
  **`GameplaySetup`** (`Resources → Terrain → Actors`) orders
  `OnEnter(Screen::Gameplay)`. Ordering across a crate boundary *must* use a shared
  set — `.chain()` cannot express it, and a local chain that looks correct will race.
  The set boundary also supplies a sync point: `Commands`-spawned entities are not
  queryable until the queue is applied, so `Actors` sees the tiles `Terrain` made.
- **A position is a voxel, not a coordinate.** `TilePos { coord, level }`. Stacked
  columns at one coordinate are not connected. Never key anything by `HexCoord` in a
  way that collapses a stack.
- **The vertical axis is `level`, never `z`** — cube coordinates already use `x`, `y`
  and `z`, and all three are horizontal.
- **A tile entity is a run of voxels, not one voxel**, and its `TilePos` is the run's
  topmost solid voxel. Interior voxels have no entity, which is why targeting is
  positional. See `docs/MAP_MODEL.md`.
- **A surface needs room above it.** Every tile carries `Headroom` — clear voxels above
  it, 0 when buried inside a column — and a `Body` may stand only where headroom is at
  least its `levels_tall` (2 for the player). Only the map can measure this, so it
  publishes it; gameplay cannot derive it from spans. `height` is reserved for terrain,
  which is why a body is `levels_tall`.
- **Screens tag entities with `DespawnOnExit(Screen::X)`**; one generic system
  clears them.
- **Speeds are world units per second**, driven by `Res<Time>`, never `SystemTime`.
- **Settings come from `assets/config/*.ron`.** Resources are absent until parsed
  rather than defaulted, so a bad file stalls loading instead of silently running
  with the wrong values.

## Bevy 0.19 specifics

Idioms that look right but aren't:

- `MessageReader<T>` / `MessageWriter<T>`, never `EventReader` (renamed in 0.17).
  **`AssetEvent<T>` is a `Message`** — read with `MessageReader`, not `add_observer`.
  `Pointer<Click>` is still an `Event` for observers.
- Required-component tuples (`Camera3d`, `Mesh3d`, `MeshMaterial3d`). No `*Bundle`.
- `ButtonInput<T>`, never `Input<T>`. `Color::srgb`, never `Color::rgb`.
- `GlobalAmbientLight` is a resource, not `AmbientLight`.
- Physical light units: illuminance in lux, `Skybox::brightness` in cd/m².
- **Cursor deltas via `CursorMoved`, never `MouseMotion`** — Wayland/WSLg does not
  deliver `MouseMotion` while a button is held. See `camera.rs::orbit_camera`.

### 0.18 → 0.19 deltas hit during the upgrade

Two of these aren't in the official migration guide:

- `Skybox::image` is now `Option<Handle<Image>>`.
- `DirectionalLight::shadows_enabled` → **`shadow_maps_enabled`**. *(Undocumented.)*
- `Assets::get_mut` returns an `AssetMut` wrapper, not `&mut A`. Bindings need
  `mut`, and you can't read the value inside the argument list of a method that
  mutably borrows it. *(Undocumented.)*
- `AssetLoader` implementations need `TypePath`.

Resources-as-components doesn't bite here because no type derives both `Resource`
and `Component`, and every query names concrete components.

## Traps

Several failure modes produce **no log output**. A clean log is not evidence a
change worked — look at the window.

| Symptom | Cause |
|---|---|
| Plain blue window | Assets not found (see "Always run through cargo") |
| Black sky | Skybox `AssetEvent` missed; PNG never reinterpreted as a cubemap |
| Stuck on "loading…" | A RON file failed to parse, or an asset path is wrong |
| Appears frozen | It's paused. The overlay exists because this was indistinguishable from a hang |

**Observers are global.** They fire in every state. One touching a gameplay-only
resource must take `Option<Res<T>>` — Bevy validates parameters *before* the body
runs, so an internal guard won't save it. This caused a real crash on the title
screen.

## Branch & PR workflow

**Everything targets `dev`. Nothing is merged straight to `main`.**

```
feat/whatever  ──PR──►  dev  ──PR──►  main
```

`dev` is permanent — it is the integration branch, not a release branch that gets
cleaned up. Open every PR against it:

```sh
gh pr create --base dev
```

`main` only ever moves by merging `dev` into it, as a deliberate promotion once the
work there has been played and looked at. That gap is the point: **CI cannot see a
black sky, a gap between tiles, or a piece sunk into the terrain**, and every serious
bug in this codebase so far was found by a person clicking. `dev` is where things are
allowed to be wrong.

- Prefixes: `chore/`, `fix/`, `perf/`, `feat/`, `docs/`.
- `refactor/*` names are usable again now the `refactor` branch is gone; a git ref
  can't be both a file and a directory, so they clashed while it existed.
- Merge with merge commits (`gh pr merge N --merge`), not squash.
- Delete feature branches once merged. **Never delete `dev`.**
- CI runs fmt, clippy, tests, `cargo deny`, and builds on all three platforms — on
  PRs into `dev` as well as into `main`.

## Current state

Runs on macOS/Metal at 60 FPS, 3,400–4,100 entities in gameplay depending on the
terrain seed. Bevy 0.19, Rust 1.97.1, 94 tests. macOS is the primary dev machine; the
WSL2 setup in the README belongs to another contributor and still works.

Structurally complete as a skeleton: workspace boundaries, CI, linting, dependency
auditing, a state machine, a RON content pipeline, a voxel map with substances and
destruction, level-based movement, and body size via headroom.

There is still no turn system, no abilities, and no pathfinder — `route` walks a
straight line and gives up when blocked. Bodies are one hex wide; there is no
footprint for anything larger, and units do not obstruct each other.

## Known gaps

- **`bevy_lint` is wired but unusable** — supports Bevy 0.18 at most. Adopting it
  later costs no source changes.
- **Bevy features untrimmed.** Still `default-features = true`. The `3d` collection
  would cut compile time and binary size but risks silently dropping capability.
- **Lints are strict, deliberately.** `#[allow]` is banned — use
  `#[expect(lint, reason = "…")]`. `unwrap`, `panic!`, slice indexing, `dbg!`,
  `println!`, float `==` and undocumented public items are all denied. Restriction
  lints are relaxed in `#[test]` functions.
- **Animation is still `Box<dyn Transformer>`**, which is why `Transformation`
  can't derive `Reflect` and is invisible in the inspector. Most likely thing to be
  rewritten when gameplay lands.
- **Headless integration tests** live in `crates/hex_map/tests/` and
  `crates/hex_gameplay/tests/`. They cannot see anything visual — a black sky or a
  mistransformed tile still needs a human looking at the window.
