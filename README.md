# magic-game

A hex-based magic game built with [Bevy](https://bevy.org/) 0.19.

![logo](readme_assets/game_logo.jpg)

## Build & run

```sh
cargo dev            # with the world inspector and live asset reload
cargo run --release  # as it ships
```

That's the whole setup on macOS and Windows. Linux needs the system packages in
the [appendix](#appendix-linux-and-wsl2). The repo pins its toolchain in
`rust-toolchain.toml`, so rustup fetches the right compiler on first build — you
don't need to match it by hand. A cold build takes 10–20 minutes; incremental
builds after that are seconds.

Two things worth knowing:

**`--release` matters for playing.** The dev profile compiles game code at `opt-level = 1` (dependencies at `3`), which is dramatically slower for anything per-frame. `cargo dev` is for iterating; `--release` is for playing.

**Run it through cargo, not by invoking the binary.** Bevy resolves its asset root from `BEVY_ASSET_ROOT` (set for you in `.cargo/config.toml`), then `CARGO_MANIFEST_DIR`, then the executable's own directory. Run `./target/release/hex_game` directly and it looks in `target/release/assets/`, finds nothing, and renders a plain blue window — `ClearColor` with no meshes, not a crash. If you ever ship a standalone binary, `assets/` has to sit beside it.

## Controls

| Input | Action |
|---|---|
| Right-mouse drag | Orbit camera around focus |
| `W` `A` `S` `D` | Pan camera |
| Mouse wheel | Zoom |
| Left-click a hex tile | Animate the player to that tile |
| `ESC` | Pause (or quit, on the title screen) |
| `BACKSPACE` | Return to the title screen |
| `ENTER` | Start the game, from the title screen |

## Diagnostics

`FrameTimeDiagnosticsPlugin`, `EntityCountDiagnosticsPlugin`, and `LogDiagnosticsPlugin` are wired in `main.rs` and print FPS and entity count to stdout once per second. They're always on rather than debug-gated, so release performance is observable without rebuilding.

The startup log also names the graphics adapter. On an M2 Max you should see:

```
AdapterInfo { name: "Apple M2 Max", device_type: IntegratedGpu, ..., backend: Metal }
```

If `device_type` ever reads `Cpu`, you're on a software rasteriser, and any framerate below ~15 FPS is explained by that alone.

## Project layout

A cargo workspace. Cargo enforces the dependency direction, so a module cannot
reach across a boundary it should not — see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
for the reasoning.

```
crates/
  hex_core/       # coordinates, voxel vocabulary, app states, shared components
  hex_assets/     # asset handles, RON settings and their loader
  hex_map/        # the map: voxels, terrain, tile spawning, map settings
  hex_world/      # sky and camera
  hex_gameplay/   # player, picking, movement, animation
  hex_dev/        # world inspector (dev feature only)
  hex_game/       # the binary: app setup, screens, menus
assets/
  config/         # designer-editable settings -- see docs/CONTENT.md
  meshes/         # hex.glb, pieces.glb
  textures/       # sky_boxes/Ryfjallet_cubemap.png
```

## Documentation

Start with the row that describes you.

| I want to… | Read |
|---|---|
| Change how the game looks or feels, without code | [docs/CONTENT.md](docs/CONTENT.md) |
| Work on the map, and I'm new here | [docs/ONBOARDING.md](docs/ONBOARDING.md) |
| Work on the map, and I'm an AI agent | [crates/hex_map/CLAUDE.md](crates/hex_map/CLAUDE.md) |
| Understand how the map works | [docs/MAP_MODEL.md](docs/MAP_MODEL.md) |
| Understand why the project is shaped this way | [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) |
| Contribute code anywhere | [CONTRIBUTING.md](CONTRIBUTING.md) |

## Contributing, in one line

**Branch off `dev`, and open your PR against `dev`** — `gh pr create --base dev`.
`main` moves only when `dev` is promoted into it, after someone has played the game.
See [CONTRIBUTING.md](CONTRIBUTING.md) for why.

---

## Appendix: Linux and WSL2

Linux needs the system packages below. The GPU-passthrough notes after that apply
only to WSL2; native Linux can skip them.

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

Wayland — and therefore WSLg — does **not** deliver `MouseMotion` events while a
mouse button is held. Drag-style input has to be built from `CursorMoved` deltas
instead. See `orbit_camera` in
[`crates/hex_world/src/camera.rs`](crates/hex_world/src/camera.rs) for the working
pattern, and don't reintroduce `MouseMotion` for dragging.
