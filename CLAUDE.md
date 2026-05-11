# Context for Claude Code

A hex-grid game built on **Bevy 0.18**. Single binary (`cargo run --release`). Single 3D world (a 2D parallel world existed pre-refactor and was stripped). All gameplay code lives under `src/plugins/world_3d/`.

## Tech stack

| Crate | Version | Notes |
|---|---|---|
| `bevy` | `0.18` | |
| `bevy-inspector-egui` | `0.36` | Targets bevy 0.18 |
| `xxhash-rust` | `0.8` | xxh3 feature |
| `rand` | `0.8` | |

**Not used:** `bevy_mod_picking` — picking was upstreamed into Bevy as of 0.15. Use `bevy::picking::mesh_picking::MeshPickingPlugin` and `On<Pointer<Click>>` observers instead.

## Build & run

```
cargo run --release
```

`--release` matters. Dev profile builds game code at `opt-level = 1` (deps at 3), which is dramatically slower for anything per-frame. See `README.md` for full build instructions and the WSL2 GPU caveat.

## Bevy 0.18 conventions in this repo

These are the non-obvious migration choices a fresh session needs to respect — Bevy 0.9-era idioms will *look* like they'd work but will compile-error or behave wrongly:

- **Systems**: `add_systems(Startup|PreStartup|Update, ...)` everywhere. Never `add_system` / `add_startup_system` (gone since 0.10).
- **Spawning**: required-component tuples — `Camera3d`, `Mesh3d(handle)`, `MeshMaterial3d(handle)`, `Sprite::from_image(handle)`, `DirectionalLight { .. }`. **No legacy `*Bundle` types** (`Camera3dBundle`, `PbrBundle`, `SpriteBundle`, etc. were removed in 0.15).
- **Input resources**: `ButtonInput<KeyCode>` / `ButtonInput<MouseButton>`, never `Input<T>` (renamed in 0.12).
- **Buffered events**: `MessageReader<T>` / `MessageWriter<T>`, never `EventReader<T>` (renamed in 0.17). `Pointer<Click>` and friends are still `Event`s for observers.
- **Cursor delta math**: use `CursorMoved` + a `Local<Option<Vec2>>` baseline, **never `MouseMotion`**. Wayland (and therefore WSL2 WSLg) doesn't deliver `MouseMotion` while a mouse button is held. See `src/plugins/world_3d/camera.rs::orbit_camera` for the working pattern.
- **Inspector wiring**: `#[derive(Reflect)]` + `#[reflect(Component)]` on the type, `app.register_type::<T>()` to register. Never the old `#[derive(Inspectable)]` / `register_inspectable` (gone).
- **Picking**: `MeshPickingPlugin` added in `main.rs`. Meshes are pickable by default; tag entities you want to opt out with `Pickable::IGNORE`. Click handler is a global observer attached via `app.add_observer(on_tile_clicked)` taking `event: On<Pointer<Click>>`, with `event.event_target()` returning the clicked entity.
- **Light units**: physical units. `DirectionalLight::illuminance` in lux (~100_000 = noon sun, ~10_000 = overcast). `GlobalAmbientLight::brightness` in lux. `Skybox::brightness` in cd/m². All centralized as consts in `src/plugins/world_3d/config.rs`. **`GlobalAmbientLight` is a resource**, not the legacy `AmbientLight`.
- **Colors**: `Color::srgb(r, g, b)`, never `Color::rgb` (renamed in 0.14).
- **GLTF sub-asset loading**: `GltfAssetLabel::Primitive { mesh, primitive }.from_asset("path.glb")`, not the legacy `"path.glb#Mesh0/Primitive0"` string syntax (still works but the structured form is the modern idiom).
- **Plugin add**: `app.add_plugins(P)` for one or many. Never `add_plugin` (gone).

## Repo layout

```
src/
  main.rs                     DefaultPlugins + MeshPickingPlugin + diagnostics + World3dPlugins
  lib.rs / plugins.rs         re-exports
  plugins/world_3d.rs         PluginGroup
  plugins/world_3d/
    camera.rs                 PanOrbitCamera (right-drag), Skybox, CursorMoved orbit
    hex.rs                    HexCoord, HexGrid, HexTile, init_height_map
    hex/height_map.rs         Perlin / Flat / Rand generators (anachronistic — TBD refactor)
    player.rs                 Player + on_tile_clicked global observer
    sky.rs                    Sun (DirectionalLight) + GlobalAmbientLight + ClearColor
    transformation.rs         Box<dyn Transformer> animation driver (anachronistic — TBD refactor)
    debug.rs                  Inspector wiring (debug builds only, F12-gating TBD)
    config.rs                 All tunable constants
assets/
  meshes/{hex,pieces}.glb
  textures/sky_boxes/Ryfjallet_cubemap.png
  textures/sprites/{hex,hex_highlighted}.png
docs/
  ROADMAP.md                  Full phase-by-phase refactor plan
```

## Branch & PR workflow

- Long-running **`refactor`** branch off `main`. Active cleanup work targets `refactor`, not `main`.
- Each fix is its own small branch off `refactor` opening a PR back into `refactor`. Branch prefixes: `chore/`, `fix/`, `perf/`, `refactor/`, `docs/`.
- Merge with **merge commits** (`gh pr merge N --merge`), not squash, so per-PR history is preserved.
- Final umbrella PR `refactor` → `main` once the whole refactor is verified end-to-end. Not opened yet.

## Current state

The migration from Bevy 0.9 to 0.18 is **shipped**. The refactor is mid-stream:

- **Phase 1 (quick wins)** — fully shipped: clippy cleanup, Eq/Hash derives on HexCoord, dead-comment removal, world_2d strip, README expansion.
- **Phase 2 (bug fixes)** — fully shipped: orbit camera (CursorMoved fix for Wayland), lighting (lux unit rebalance).
- **Phase 3 (perf)** — partially shipped: baseline diagnostics (PR #9) confirm ~11 FPS on llvmpipe / WSL2 software rendering. Remaining: inspector toggle, perlin alloc fix, cached height map, skybox asset-event, shadows-and-draws investigation.
- **Phase 4 (architecture refactors)** — pending: shared HexCoord module, `Res<Time>` instead of `SystemTime`, replace `Box<dyn Transformer>` with concrete components, picking cleanup, plugin-group cleanup.
- **Phase 5 (umbrella PR)** — pending.

See **`docs/ROADMAP.md`** for the per-PR detail.

## Next pending PR

**PR 3.2 — `perf/inspector-toggle`**, OR if a real-GPU passthrough has just landed, the first task is to verify `cargo run --release` hits 60+ FPS with `SUN_SHADOWS_ENABLED = true` (see "Known issues" below) and revert any shadows-off stop-gap.

## Known issues / open threads

- **Skybox grain**: cubemap looks visibly noisy under llvmpipe. Expected to disappear on real GPU; if not, file `fix/skybox-grain` per the roadmap (likely missing mipmaps on the reinterpreted stacked-2D texture).
- **WSL2 GPU passthrough**: Ubuntu 22.04's stock Mesa ships no Vulkan ICD for NVIDIA and no Dozen (D3D12 translation). `wgpu` falls back to `llvmpipe` software rendering. Fix is moving to Ubuntu 24.04 in WSL (Mesa 24+ includes Dozen) or running natively on Windows. Diagnosed in conversation 2026-05-10.
- **Shadow stop-gap**: while running on llvmpipe, `DirectionalLight::shadows_enabled` was experimented set to `false` (doubles FPS, sacrifices visual depth). The current `refactor` HEAD still has `shadows_enabled: true`. Once GPU passthrough works, leave it `true`.
- **WSL2 + Wayland gotcha**: raw `MouseMotion` events are *not* delivered while a mouse button is held. We hit this on orbit-drag and fixed it by switching to `CursorMoved` deltas. Don't reintroduce `MouseMotion` for drag-style input.

## Process notes

- `cargo run --release` is the canonical way to run. `cargo run` (dev) is much slower and only useful for compile-time iteration.
- Diagnostics plugins (`FrameTimeDiagnosticsPlugin`, `EntityCountDiagnosticsPlugin`, `LogDiagnosticsPlugin`) are wired in `main.rs` and print FPS / entity count to stdout once per second. Always-on (not debug-gated) so release-build perf is observable without rebuilding.
- The inspector (`bevy_inspector_egui::quick::WorldInspectorPlugin`) is added in debug builds only. It traverses every entity every frame for reflection — with 1261 hex tiles + UI that's measurable overhead. Toggling it on F12 is a pending PR (3.2).
- Auto-merging small PRs into `refactor` is fine when the user has approved the line of work. Destructive operations (force push, deleting branches not owned by this session, unregistering WSL distros, etc.) still need explicit confirmation.
