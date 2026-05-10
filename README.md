# magic-game

A hex-based magic game built with [Bevy](https://bevy.org/) 0.18.

![logo](readme_assets/game_logo.jpg)

## Build & run

```sh
cargo run --release
```

The `--release` flag matters. The dev profile compiles game code at `opt-level = 1` (deps at `3`), which is 5–30× slower than release for everything except the engine itself.

## Controls

| Input | Action |
|---|---|
| Right-mouse drag | Orbit camera around focus |
| `W` `A` `S` `D` | Pan camera |
| Mouse wheel | Zoom |
| Left-click a hex tile | Animate the player to that tile |
| `F12` | Toggle the world inspector (debug builds only) |

## WSL2 performance note

Bevy renders through `wgpu`. On WSL2 without GPU passthrough configured, `wgpu` falls back to **llvmpipe** (CPU software rendering), which produces single-digit FPS even in `--release`.

To get a usable framerate on WSL2:

1. Install the latest GPU-vendor driver on the **Windows host** — NVIDIA Game Ready / Studio, AMD Adrenalin, or Intel Arc — all of these expose the host GPU into WSL.
2. `wsl --shutdown` from PowerShell, then reopen the WSL terminal.
3. Verify with `vulkaninfo --summary`. You want to see your host GPU listed, not `llvmpipe`.
4. If `wgpu` still picks the wrong backend, override with `WGPU_BACKEND=vulkan cargo run --release`.

If host-GPU passthrough is unavailable, build and run the game natively on Windows or macOS instead.

## Project layout

```
src/
  main.rs                          # App entry point
  lib.rs                           # pub mod plugins
  plugins.rs                       # pub mod world_3d
  plugins/
    world_3d.rs                    # PluginGroup wiring
    world_3d/
      camera.rs                    # PanOrbitCamera + Skybox
      hex.rs                       # HexCoord, HexGrid, HexTile
      hex/height_map.rs            # Perlin terrain generation
      player.rs                    # Player + click-to-move observer
      sky.rs                       # Sun (DirectionalLight) + ambient
      transformation.rs            # Animation driver
      debug.rs                     # Inspector wiring
      config.rs                    # Hex / camera / sun constants
assets/
  meshes/        # hex.glb, pieces.glb
  textures/      # sprite hex tiles, sky_boxes/Ryfjallet_cubemap.png
```
