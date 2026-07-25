# Context for Claude Code

A hex-grid game built on **Bevy 0.19**. Single binary (`cargo run --release`). Single 3D world (a 2D parallel world existed pre-refactor and was stripped). All gameplay code lives under `src/plugins/world_3d/`.

## Tech stack

| Crate | Version | Notes |
|---|---|---|
| `bevy` | `0.19` | |
| `bevy-inspector-egui` | `0.37` | Targets bevy 0.19 (via `bevy_egui` 0.40 / `egui` 0.34) |
| `xxhash-rust` | `0.8` | xxh3 feature |
| `rand` | `0.8` | |

Toolchain is pinned to **Rust 1.97.1** in `rust-toolchain.toml`. Bevy 0.19's MSRV is 1.95, so this isn't optional — an older stable fails to build the dependency tree at all, with errors that don't obviously point at the toolchain.

**Not used:** `bevy_mod_picking` — picking was upstreamed into Bevy as of 0.15. Use `bevy::picking::mesh_picking::MeshPickingPlugin` and `On<Pointer<Click>>` observers instead.

## Build & run

```
cargo run --release
```

`--release` matters. Dev profile builds game code at `opt-level = 1` (deps at 3), which is dramatically slower for anything per-frame.

**Always run through cargo.** Bevy resolves the asset root from `CARGO_MANIFEST_DIR` when set and from the executable's directory otherwise, so `./target/release/magic_game` looks in `target/release/assets/` and finds nothing. The symptom is a blank blue window (`ClearColor` with no meshes) and `Path not found` lines in the log — it looks like a rendering bug and isn't one.

## Bevy 0.19 conventions in this repo

Non-obvious choices a fresh session needs to respect. Bevy 0.9-era idioms will *look* like they'd work but will compile-error or behave wrongly:

- **Systems**: `add_systems(Startup|PreStartup|Update, ...)` everywhere. Never `add_system` / `add_startup_system` (gone since 0.10).
- **Spawning**: required-component tuples — `Camera3d`, `Mesh3d(handle)`, `MeshMaterial3d(handle)`, `DirectionalLight { .. }`. **No legacy `*Bundle` types** (`Camera3dBundle`, `PbrBundle`, `SpriteBundle`, etc. were removed in 0.15).
- **Input resources**: `ButtonInput<KeyCode>` / `ButtonInput<MouseButton>`, never `Input<T>` (renamed in 0.12).
- **Buffered events are messages**: `MessageReader<T>` / `MessageWriter<T>`, never `EventReader<T>` (renamed in 0.17). This includes `AssetEvent<T>` — it is a `Message`, so it's read with `MessageReader`, *not* via `add_observer`. `Pointer<Click>` and friends are still `Event`s for observers.
- **Cursor delta math**: use `CursorMoved` + a `Local<Option<Vec2>>` baseline, **never `MouseMotion`**. Wayland (and therefore WSL2 WSLg) doesn't deliver `MouseMotion` while a mouse button is held. See `camera.rs::orbit_camera`.
- **Animation timing**: `Res<Time>`, never `SystemTime`. Transformers receive *seconds elapsed since their own `Transformation` component started*, never an absolute timestamp — see `transformation.rs`. Speeds in `config.rs` are world units per **second**.
- **Inspector wiring**: `#[derive(Reflect)]` + `#[reflect(Component)]` on the type, `app.register_type::<T>()` to register. Never the old `#[derive(Inspectable)]` / `register_inspectable` (gone).
- **Picking**: `MeshPickingPlugin` added in `main.rs`. Meshes are pickable by default; tag entities you want to opt out with `Pickable::IGNORE`. Click handler is a global observer via `app.add_observer(on_tile_clicked)` taking `event: On<Pointer<Click>>`, with `event.event_target()` returning the clicked entity.
- **Light units**: physical units. `DirectionalLight::illuminance` in lux (~100_000 = noon sun, ~10_000 = overcast). `GlobalAmbientLight::brightness` in lux. `Skybox::brightness` in cd/m². All centralized as consts in `config.rs`. **`GlobalAmbientLight` is a resource**, not the legacy `AmbientLight`.
- **Colors**: `Color::srgb(r, g, b)`, never `Color::rgb` (renamed in 0.14).
- **GLTF sub-asset loading**: `GltfAssetLabel::Primitive { mesh, primitive }.from_asset("path.glb")`, not the legacy `"path.glb#Mesh0/Primitive0"` string syntax.
- **Plugin add**: `app.add_plugins(P)` for one or many. Never `add_plugin` (gone).

### 0.18 → 0.19 deltas hit during the upgrade

Only three things in this codebase broke. Recorded because two of them aren't in the official migration guide:

- `Skybox::image` is now `Option<Handle<Image>>`.
- `DirectionalLight::shadows_enabled` → **`shadow_maps_enabled`**. *(Not in the migration guide.)*
- `Assets::get_mut` returns a change-detection `AssetMut` wrapper rather than `&mut A`. Bindings need `mut`, and you can't read from the value inside an argument list of a method that mutably borrows it — bind intermediates first. *(Not in the migration guide.)*

Unaffected despite 0.19's two big internal reworks (resources-as-components, render-graph-as-systems): picking observers, `MessageReader`, `GlobalAmbientLight`, diagnostics plugins, `WgpuSettings`/`InstanceFlags`, `register_type`, `PluginGroup` wiring. Resources-as-components doesn't bite because no type here derives both `Resource` and `Component`, and every query names concrete components (so none can collide with resources).

**Entity counts jumped ~1286 → ~1647 across the upgrade.** The grid is still 1261 tiles (3r²+3r+1 at radius 20); the extra ~360 are resources, which 0.19 now stores as components on dedicated entities. Not a leak — but it means `EntityCountDiagnosticsPlugin` readings aren't comparable to any pre-0.19 baseline.

## Repo layout

```
src/
  main.rs                     DefaultPlugins + MeshPickingPlugin + diagnostics + World3dPlugins
  lib.rs / plugins.rs         re-exports
  plugins/world_3d.rs         PluginGroup
  plugins/world_3d/
    camera.rs                 PanOrbitCamera (right-drag), Skybox, CursorMoved orbit
    hex.rs                    HexCoord, HexGrid, HexTile, init_height_map
    hex/height_map.rs         Perlin / Flat / Rand generators, precomputed cache
    player.rs                 Player + on_tile_clicked global observer
    sky.rs                    Sun (DirectionalLight) + GlobalAmbientLight + ClearColor
    transformation.rs         Box<dyn Transformer> animation driver, Res<Time> based
    debug.rs                  Inspector wiring (debug builds only)
    config.rs                 All tunable constants
rust-toolchain.toml           Pins Rust 1.97.1
assets/
  meshes/{hex,pieces}.glb
  textures/sky_boxes/Ryfjallet_cubemap.png
  textures/sprites/{hex,hex_highlighted}.png
docs/
  ROADMAP.md                  State of the codebase and open work
```

## Branch & PR workflow

- Long-running **`refactor`** branch off `main`. Cleanup work targets `refactor`, not `main`.
- Each fix is its own small branch off `refactor` opening a PR back into `refactor`. Branch prefixes: `chore/`, `fix/`, `perf/`, `refactor/`, `docs/`.
- **`refactor/*` branch names are unusable** while a branch named `refactor` exists — git refs can't be both a file and a directory, so `git checkout -b refactor/foo` fails outright. Use another prefix.
- Merge with **merge commits** (`gh pr merge N --merge`), not squash, so per-PR history is preserved.

## Current state

Runs at **~120 FPS on an M2 Max via Metal**, 1647 entities, no warnings in the log. Bevy 0.19, Rust 1.97.1. macOS is the primary development machine; the WSL2 setup documented in the README belongs to another contributor and still works.

The Bevy 0.9 → 0.18 → 0.19 migration is complete and verified end-to-end: terrain renders with height variation, skybox is clean (no grain — that was a software-rendering artifact), shadows cast, right-drag orbits, WASD pans, scroll zooms, click-to-move animates hex-by-hex.

See **`docs/ROADMAP.md`** for what shipped and what's genuinely open.

## Known issues / open threads

- **The animation system is still trait-object based.** `transformation.rs` uses `Box<dyn Transformer>` with a `TransformerSeries` queue. It works and is now correctly frame-timed, but it's the most likely thing to get rewritten once real gameplay lands.
- **No tests.** Deliberate so far. Worth revisiting now that this is a base rather than a migration target.
- **`HexCoord` lives inside the `world_3d` plugin tree** (`plugins/world_3d/hex.rs`) rather than at the top level. Fine today; would want moving if a second world ever appears.
- **`PerlinGenerator::gradient` allocates two `String`s per lookup** via `vec.to_string()`. Cheap to fix with byte hashing, but doing so changes terrain output for a given seed, so it wasn't bundled with the other perf work.
- **Dozen non-conformance**: `main.rs` passes `InstanceFlags::ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER` through `RenderPlugin`'s `WgpuSettings`. Don't remove it — without that flag wgpu silently picks llvmpipe over the Dozen-wrapped GPU on WSL2. No effect on macOS/Metal, native Linux, or Windows.

## Process notes

- `cargo run --release` is the canonical way to run. `cargo run` (dev) is much slower and only useful for compile-time iteration.
- The inspector (`bevy_inspector_egui::quick::WorldInspectorPlugin`) is added in debug builds only. It reflects every entity every frame even when the panel is hidden; with 1647 entities that's measurable in debug builds, though invisible in release.
- Auto-merging small PRs into `refactor` is fine when the user has approved the line of work. Destructive operations (force push, deleting branches not owned by this session, unregistering WSL distros) still need explicit confirmation.
- Some failures here are **silent and visual**: a missed skybox `AssetEvent` renders a black sky with nothing in the log, and a wrong speed-unit conversion just looks off. Log-clean does not mean correct — get eyes on the window before claiming a rendering or animation change works.
