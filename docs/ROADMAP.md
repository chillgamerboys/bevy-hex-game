# Refactor roadmap

## Context

The Bevy 0.9 → 0.18 migration shipped as **PR #1**. The migration preserved many 0.9-era patterns verbatim, leaving the codebase littered with anachronisms (custom unix-time clock, `Box<dyn Trait>` components, commented-out feature toggles). Two user-visible bugs surfaced during the first run on Bevy 0.18 and have been fixed (orbit camera, lighting). A perf baseline measurement (PR #9) confirmed software-rendering FPS at ~11. Phase 3 perf work is partially done and currently blocked on WSL2 GPU passthrough; Phase 4 architecture rewrites are pending.

Scope (approved 2026-05-10): full rewrite. Touch every audit finding. Code + WSL2 environment investigation. Long-running `refactor` branch with separate small PRs ordered quick → intense. Final umbrella PR `refactor → main` when everything's verified.

## Strategy

- `refactor` branch off `main` accumulates all cleanup PRs.
- Every PR below is a separate short-lived branch off `refactor` with its own PR back into `refactor`.
- Merge with merge commits (`gh pr merge N --merge`), not squash, so per-PR history is preserved.
- Final umbrella PR opens once `cargo run --release` on `refactor` is verified end-to-end working with full quality.

## Phase 1 — Quick wins ✅ SHIPPED

- ✅ **PR 1.1** `chore/clippy-cleanup` — #4. Float precision, `*=` ops, `Default` impl for `TransformerSeries`.
- ✅ **PR 1.2** `chore/derive-eq-on-hexcoord` — #5. Removed manual `impl PartialEq`, added `Eq, PartialEq, Hash` derives (Hash is pre-work for Phase 3.4).
- ✅ **PR 1.3** `chore/remove-dead-generators-comments` — #6. Collapsed `init_height_map` to the single active `lowlands` line.
- ✅ **PR 1.4** `chore/strip-2d-world` — #3. Deleted `src/plugins/world_2d/` (never wired into `main.rs`).
- ✅ **PR 1.5** `docs/readme` — #2. Expanded README with build/run, controls, WSL2 caveat (Phase 3.0 content folded in).

## Phase 2 — Bug fixes ✅ MOSTLY SHIPPED

- ✅ **PR 2.1** `fix/orbit-camera` — #7. Right-drag orbit did nothing on Wayland because `MouseMotion` doesn't fire while a button is held. Switched to `CursorMoved` deltas with a `Local<Option<Vec2>>` baseline. **Do not reintroduce `MouseMotion` for drag.**
- ✅ **PR 2.2** `fix/lighting-lux-units` — #8. Rebalanced for Bevy 0.18 physical units. `SUN_INTENSITY: 50_000 → 10_000`, `SUN_AMBIENT_LIGHT: 1 → 80`, new `SKYBOX_BRIGHTNESS: 300`. All in `src/plugins/world_3d/config.rs`.
- ⏳ **PR 2.3** `fix/skybox-grain` — **pending, gated on GPU passthrough**. Cubemap looks noisy under llvmpipe; expected to disappear on real GPU. If it persists: likely missing mipmaps on the reinterpreted stacked-2D texture. Investigate by computing mipmaps after `reinterpret_stacked_2d_as_array` in `src/plugins/world_3d/camera.rs::reinterpret_skybox_when_loaded`, OR verify the sampler filter mode (`mipmap_filter: FilterMode::Linear`).

## Phase 3 — Performance

**Approach**: measure first, fix second. The baseline (Phase 3.1) showed ~11 FPS steady state on llvmpipe. WSL2 GPU passthrough fix is happening separately; once a real GPU is in play, most of Phase 3 may be unnecessary or its priority drops dramatically.

- ✅ **PR 3.1** `perf/measure-baseline` — #9. Added `FrameTimeDiagnosticsPlugin`, `EntityCountDiagnosticsPlugin`, `LogDiagnosticsPlugin` in `main.rs`. Always-on (release-observable). Baseline on llvmpipe / WSL2: `~11.2 FPS avg, ~88 ms/frame, 1286 entities (1261 hex tiles + 25 other)`.
- ⏳ **PR 3.2** `perf/inspector-toggle` — pending. `bevy_inspector_egui::quick::WorldInspectorPlugin` reflects every entity every frame for the inspector UI even when the panel isn't visible. With 1286 entities that's measurable in debug builds. Replace with `DefaultInspectorConfigPlugin` + manual `bevy_inspector::ui_for_world` system gated on a `Res<InspectorOpen>` toggled by F12. Files: `src/plugins/world_3d/debug.rs`.
- ⏳ **PR 3.3** `perf/perlin-no-string-alloc` — pending. `PerlinGenerator::gradient` in `src/plugins/world_3d/hex/height_map.rs:143-147` allocates a `String` twice per gradient lookup via `vec.to_string()`. With 1261 tiles × 4 gradients × 2 = ~10k allocations at spawn (plus more during path generation). Replace with direct f32 byte hashing (`bytemuck::bytes_of(&vec)` or concatenated `to_ne_bytes`). Output must be bit-identical for same seed. One-shot spawn cost; doesn't affect steady-state FPS.
- ⏳ **PR 3.4** `perf/cached-height-map` — pending. `HeightMap::get_world_height` recomputes perlin each call: once per tile at spawn (1261), again for each waypoint in `HexPathingLine::new`. Precompute `HashMap<HexCoord, f32>` inside `HeightMap::new`. Hash derive on HexCoord landed in PR 1.2. Files: `src/plugins/world_3d/hex/height_map.rs`.
- ⏳ **PR 3.5** `perf/skybox-asset-event` — pending. `src/plugins/world_3d/camera.rs::reinterpret_skybox_when_loaded` runs every Update, querying for the `SkyboxNeedsReinterpret` marker. Even after the marker is removed the query still scans every frame. Replace with an `AssetEvent::LoadedWithDependencies` observer that fires once.
- ⏳ **PR 3.6** `perf/shadows-and-draws` — gated on baseline + GPU work. Experimentally measured (shadows off): `~22 FPS avg` vs. `~11 FPS` with shadows on — directional shadows over 1261 tiles cost ~half the frame time on llvmpipe. **On real GPU this is trivial work; don't disable shadows in production.** If perf is still bad post-GPU-fix, options are (a) single-cascade `CascadeShadowConfig` with tight `maximum_distance ≈ 50`, (b) lower shadow map size, (c) `ShadowFilteringMethod::Hardware2x2`. A `SUN_SHADOWS_ENABLED` const exists in a stashed branch and can be reintroduced as a config knob.

## Phase 4 — Architecture / anachronism rewrites

These touch many files and land last to avoid merge churn with Phase 3.

- ⏳ **PR 4.1** `refactor/shared-hexcoord-module` — pending. `HexCoord` and its methods (`to_world`, `from_world`, `from_floating`, `within_radius`, `line_between`, `distance`, `to_bytes`) currently live in `src/plugins/world_3d/hex.rs`. World-2D's parallel copy was deleted in PR 1.4, so there's no current duplication, but moving HexCoord to a top-level `src/hex/mod.rs` makes it reusable if another world ever appears and decouples it from the world_3d plugin tree. **Lower priority now that world_2d is gone — could defer.**
- ⏳ **PR 4.2** `refactor/res-time` — pending. `src/plugins/world_3d/transformation.rs:17-19` defines a wall-clock `now() -> f64` via `SystemTime::now()`. Animations should be frame-aware. Rewrite `Transformer::update(&self, transform: &mut Transform, time: f64)` to take elapsed seconds since the transformer was attached, fed by `Res<Time>` inside `transformation_driver`. `LinearMovement`, `HexPathingLine`, `TransformerSeries` all switch from absolute unix-ms to seconds-since-start.
- ⏳ **PR 4.3** `refactor/animation-via-component` — pending, largest. Replace the entire `Transformer` / `Box<dyn Transformer>` machinery in `src/plugins/world_3d/transformation.rs` with concrete `LinearMovement` component (no trait objects, no per-animation heap alloc). Series animations become a `Queue<LinearMovement>` component the system pops from. Alternative path is `bevy::animation::AnimationClip` + `AnimationPlayer` but that's overkill for current usage.
- ⏳ **PR 4.4** `refactor/picking-clean` — pending. `src/plugins/world_3d/player.rs::spawn_player` adds `Pickable::IGNORE` to both player meshes so clicks pass through to the tile beneath. The `on_tile_clicked` observer already filters by `Query<&HexCoord, With<HexTile>>` so non-tile clicks are ignored automatically. Try removing `Pickable::IGNORE`; if clicks on the player now break movement, revert. Cosmetic cleanup, small PR.
- ⏳ **PR 4.5** `refactor/plugin-group-cleanup` — pending. After 1.4 stripped world_2d, `World3dPlugins` is the only plugin group. Either keep for self-documentation or inline `add_plugins((CameraPlugin, HexPlugin, ...))` directly in `main.rs`. Pick whichever is clearer; minor.

## Phase 5 — Umbrella PR

- ⏳ **PR 5.1** `refactor → main` — pending. Opens after every PR above is merged into `refactor` AND the game runs at full quality and acceptable framerate on a real GPU. Body links each child PR. Merge with merge commit so the per-PR history lands on `main`.

## API mapping reference (Bevy 0.9 → 0.18)

Kept here so the answer is one search away when reading old gists or migrating other Bevy 0.9 code into this repo.

| Bevy 0.9 | Bevy 0.18 |
|---|---|
| `add_plugin(P)` / `add_plugins((A, B))` | `add_plugins(P)` / `add_plugins((A, B))` (unified) |
| `add_startup_system(f)` | `add_systems(Startup, f)` |
| `add_startup_system_to_stage(StartupStage::PreStartup, f)` | `add_systems(PreStartup, f)` |
| `add_system(f)` | `add_systems(Update, f)` |
| `Res<Input<T>>` | `Res<ButtonInput<T>>` |
| `Res<Windows>` + `.get_primary()` | `Query<&Window, With<PrimaryWindow>>` + `.single()` |
| `EventReader::iter()` | `MessageReader::read()` (also rename of the type) |
| `time.delta_seconds()` | `time.delta_secs()` |
| `Camera3dBundle { ... }` | `(Camera3d::default(), Transform, ...)` |
| `Camera2dBundle::default()` | `Camera2d` |
| `PbrBundle { mesh, material, transform }` | `(Mesh3d(mesh), MeshMaterial3d(material), transform)` |
| `MaterialMeshBundle::<M>` | `(Mesh3d, MeshMaterial3d::<M>)` |
| `SpriteBundle { texture, transform }` | `(Sprite::from_image(texture), transform)` |
| `DirectionalLightBundle { directional_light, transform }` | `(DirectionalLight { .. }, transform)` |
| `SpatialBundle::default()` | `(Transform::default(), Visibility::default())` |
| `shape::Cube { size }` | `Cuboid::new(size, size, size)` |
| `#[derive(TypeUuid)] #[uuid="..."]` for assets | `#[derive(Asset, TypePath)]` |
| `#[derive(Inspectable)]` | `#[derive(Reflect)]` + `#[reflect(Component)]` |
| `app.register_inspectable::<T>()` | `app.register_type::<T>()` |
| `bevy_mod_picking::DefaultPickingPlugins` | `bevy::picking::mesh_picking::MeshPickingPlugin` |
| `PickableBundle::default()` | (nothing — meshes pickable by default; opt out with `Pickable::IGNORE`) |
| `PickingCameraBundle::default()` | (nothing — automatic) |
| `EventReader<PickingEvent>` + `SelectionEvent::JustSelected` | `commands.entity(e).observe(\|t: On<Pointer<Click>>\| ...)` or `app.add_observer(...)` |
| `AmbientLight { .. }` resource | `GlobalAmbientLight { .. }` resource |
| `Color::rgb(...)` | `Color::srgb(...)` |
| `MouseMotion` | `CursorMoved` (when you need drag deltas — Wayland-friendly) |

## Critical files (across the roadmap)

- `Cargo.toml` — diagnostics features for Phase 3.x.
- `src/main.rs` — Phase 3.x plugin wiring.
- `src/plugins/world_3d/camera.rs` — Phases 3.5, 3.6.
- `src/plugins/world_3d/hex.rs` — Phase 4.1.
- `src/plugins/world_3d/hex/height_map.rs` — Phases 3.3, 3.4.
- `src/plugins/world_3d/player.rs` — Phase 4.4.
- `src/plugins/world_3d/sky.rs` — Phase 3.6 (shadow knob).
- `src/plugins/world_3d/transformation.rs` — Phases 4.2, 4.3.
- `src/plugins/world_3d/debug.rs` — Phase 3.2.
- `src/plugins/world_3d/config.rs` — Phase 3.6 (shadow const).
- New `src/hex/mod.rs` — Phase 4.1.

## Verification (post-roadmap)

End-to-end manual checklist run after the final PR opens:

- `cargo build --release` clean. `cargo clippy --all-targets -- -D warnings` clean.
- Frame time at steady state ≥ 60 FPS on a real GPU.
- Hex grid renders with height variation; sun direction unchanged; skybox visible without grain.
- **Right-drag rotates the camera**; WASD pans; scroll zooms.
- Clicking a hex tile moves the player along the hex path animation; path math uses `Res<Time>` not `SystemTime`.
- F12 toggles the inspector (Phase 3.2); FPS recovers when hidden.
- No `unwrap()` / `.expect()` outside genuine "this cannot fail at runtime" spots.
- No commented-out alternative code blocks remain in source files.

## What this roadmap does NOT do

- Rewrite the rendering pipeline (custom shaders / GI / etc.). Bevy 0.18's defaults are fine.
- Replace the perlin generator with a different terrain algorithm. Perf-fixed (Phase 3.3, 3.4) but kept algorithmically.
- Introduce ECS-state-machine crates or external animation libraries.
- Add tests. (No tests exist today; adding test infrastructure is its own decision the user can take after this lands.)
- Mid-refactor distro / OS migrations. Environment work (WSL2 GPU passthrough, switching to Ubuntu 24.04) is tracked separately in the README and in conversation, not as PRs in this roadmap.
