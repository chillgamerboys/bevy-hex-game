# magic-game

A hex-based magic game built with [Bevy](https://bevy.org/) 0.19.

![logo](readme_assets/game_logo.jpg)

## Build & run

```sh
cargo run --release
```

That's the whole setup on macOS and Windows. The repo pins its toolchain in `rust-toolchain.toml`, so rustup fetches the right compiler on first build — you don't need to match it by hand. A cold build takes 10–20 minutes; incremental builds after that are seconds.

Two things worth knowing:

**`--release` matters.** The dev profile compiles game code at `opt-level = 1` (dependencies at `3`), which is dramatically slower for anything per-frame. Use `cargo run` only when you're iterating on compile errors.

**Run it through cargo, not by invoking the binary.** Bevy resolves the asset root relative to `CARGO_MANIFEST_DIR` when that's set, and relative to the executable otherwise — so `./target/release/magic_game` looks in `target/release/assets/` and finds nothing. The symptom is a plain blue window: that's `ClearColor` with no meshes drawn, not a crash. If you ever ship a standalone binary, the `assets/` directory has to sit beside it.

## Controls

| Input | Action |
|---|---|
| Right-mouse drag | Orbit camera around focus |
| `W` `A` `S` `D` | Pan camera |
| Mouse wheel | Zoom |
| Left-click a hex tile | Animate the player to that tile |

## Diagnostics

`FrameTimeDiagnosticsPlugin`, `EntityCountDiagnosticsPlugin`, and `LogDiagnosticsPlugin` are wired in `main.rs` and print FPS and entity count to stdout once per second. They're always on rather than debug-gated, so release performance is observable without rebuilding.

The startup log also names the graphics adapter. On an M2 Max you should see:

```
AdapterInfo { name: "Apple M2 Max", device_type: IntegratedGpu, ..., backend: Metal }
```

If `device_type` ever reads `Cpu`, you're on a software rasteriser, and any framerate below ~15 FPS is explained by that alone.

## Project layout

```
src/
  main.rs                          # App entry point, plugin wiring, diagnostics
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
      debug.rs                     # Inspector wiring (debug builds only)
      config.rs                    # Hex / camera / sun constants
assets/
  meshes/        # hex.glb, pieces.glb
  textures/      # sprite hex tiles, sky_boxes/Ryfjallet_cubemap.png
docs/
  ROADMAP.md     # State of the codebase and open work
```

---

## Appendix: Linux and WSL2

Skip this entirely unless you're building on Linux under WSL2. Native Linux, macOS, and Windows need nothing here.

### System packages

```sh
sudo apt install -y build-essential pkg-config libssl-dev \
    libudev-dev libasound2-dev libwayland-dev libxkbcommon-dev \
    libx11-dev libxi-dev libxrandr-dev libxcursor-dev libxinerama-dev \
    mesa-vulkan-drivers vulkan-tools
```

### GPU passthrough

Bevy renders through `wgpu`. On WSL2 without a working Vulkan path to the host GPU, `wgpu` falls back to **llvmpipe** (CPU software rendering) — single-digit FPS even in `--release`. The fix is **Mesa Dozen**, a Vulkan-over-D3D12 translation layer that reaches the host GPU through `/usr/lib/wsl/lib/libd3d12.so`.

1. **Install the latest GPU-vendor driver on the Windows host** (NVIDIA Game Ready / Studio, AMD Adrenalin, or Intel Arc). These drop `libd3d12.so` and friends into `/usr/lib/wsl/lib/` on every WSL distro.
2. **Use Ubuntu 24.04 (noble) or newer.** Ubuntu 22.04 can't work: its stock Mesa lacks Dozen, and the kisak-mesa PPA has dropped jammy. Install with `wsl --install -d Ubuntu-24.04` from PowerShell.
3. **Get a Mesa build that includes Dozen.** Noble's stock `mesa-vulkan-drivers` strips it, so add the kisak-mesa PPA:
   ```sh
   sudo add-apt-repository -y ppa:kisak/kisak-mesa
   sudo apt update && sudo apt upgrade -y
   ```
4. **Verify.** `vulkaninfo --summary | sed -n '/Devices:/,$p'` should list your host GPU (`driverID = DRIVER_ID_MESA_DOZEN`) alongside llvmpipe, and `ls /usr/share/vulkan/icd.d/` should include `dzn_icd.x86_64.json`.

No environment variables are needed. `main.rs` already passes `InstanceFlags::ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER` to wgpu — Dozen self-reports as non-conformant, and without that flag wgpu silently filters it out and falls back to llvmpipe. **Don't remove that flag**; it's inert on every other platform.

Measured on an RTX 3080 host: 60 FPS V-sync locked via Dozen, versus ~11 FPS on llvmpipe.

### Wayland gotcha

Wayland — and therefore WSLg — does **not** deliver `MouseMotion` events while a mouse button is held. Drag-style input has to be built from `CursorMoved` deltas instead. See `orbit_camera` in `src/plugins/world_3d/camera.rs` for the working pattern, and don't reintroduce `MouseMotion` for dragging.
