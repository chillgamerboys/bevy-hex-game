# Context for Claude Code

A hex-grid game on **Bevy 0.19**, organised as a six-crate cargo workspace.

Read **[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)** first — it explains the crate
graph and, more usefully, the reasoning behind it. This file is the operational
summary.

## Tech stack

| Crate | Version | Notes |
|---|---|---|
| `bevy` | `0.19` | |
| `hexx` | `0.24` | **No Bevy features.** Pins only `glam`, so it can never gate a Bevy upgrade |
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
and tests without a renderer. It holds the test suite (17 tests).

## Conventions

- **Modules expose `pub fn plugin(app: &mut App)`**, not a `Plugin` struct.
- **Each plugin registers its own reflected types.** Never a central list.
- **`AppSystems`** (`TickTimers → RecordInput → Update`) orders `Update`;
  **`GameplaySetup`** (`Resources → Terrain → Actors`) orders
  `OnEnter(Screen::Gameplay)`. Ordering across a crate boundary *must* use a shared
  set — `.chain()` cannot express it, and a local chain that looks correct will race.
  The set boundary also supplies a sync point: `Commands`-spawned entities are not
  queryable until the queue is applied, so `Actors` sees the tiles `Terrain` made.
- **A position is a tile, not a coordinate.** Stacked columns at one coordinate are
  not connected. Never key a map by `HexCoord` in a way that collapses a stack.
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

- Long-running **`refactor`** branch off `main`; work targets `refactor`.
- Prefixes: `chore/`, `fix/`, `perf/`, `feat/`, `docs/`.
- `refactor/*` names are usable again now the `refactor` branch is gone; a git ref
  can't be both a file and a directory, so they clashed while it existed.
- Merge with merge commits (`gh pr merge N --merge`), not squash.
- CI runs fmt, clippy, tests, `cargo deny`, and builds on all three platforms.

## Current state

Runs on macOS/Metal, ~1647 entities in gameplay. Bevy 0.19, Rust 1.97.1. macOS is
the primary dev machine; the WSL2 setup in the README belongs to another
contributor and still works.

Structurally complete as a skeleton: workspace boundaries, CI, linting,
dependency auditing, a state machine, a RON content pipeline, and the first tests.
Gameplay itself is still the 2022 prototype — a grid, a camera, and a piece that
walks between tiles. The design doc is what comes next.

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
