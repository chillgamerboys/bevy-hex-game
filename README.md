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

## WSL2 setup (NVIDIA / AMD / Intel GPU passthrough)

Bevy renders through `wgpu`. On WSL2 without a properly configured Vulkan path to the host GPU, `wgpu` falls back to **llvmpipe** (CPU software rendering), which produces single-digit FPS even in `--release`. The fix is **Mesa Dozen** — a Vulkan-over-D3D12 translation layer that routes Vulkan calls through `/usr/lib/wsl/lib/libd3d12.so` to the host GPU.

The pieces required:

1. **Latest GPU-vendor driver on the Windows host** (NVIDIA Game Ready / Studio, AMD Adrenalin, or Intel Arc). These drop `libd3d12.so`, `libdxcore.so`, and friends into `/usr/lib/wsl/lib/` on every WSL distro.
2. **Ubuntu 24.04 (noble) or newer** in WSL. Ubuntu 22.04 (jammy) is unsupported here: its stock Mesa lacks Dozen and the kisak-mesa PPA has dropped jammy. Install with `wsl --install -d Ubuntu-24.04` from PowerShell.
3. **Mesa with Dozen built in.** Ubuntu noble's stock `mesa-vulkan-drivers` package strips Dozen. Add the kisak-mesa PPA which ships a build that includes it:
   ```sh
   sudo add-apt-repository -y ppa:kisak/kisak-mesa
   sudo apt update
   sudo apt upgrade -y
   ```
4. **Verify.** `vulkaninfo --summary | sed -n '/Devices:/,$p'` should now list two devices — your host GPU (`deviceName = Microsoft Direct3D12 (NVIDIA GeForce RTX 3080)` or equivalent, `driverID = DRIVER_ID_MESA_DOZEN`) plus llvmpipe as a fallback. `ls /usr/share/vulkan/icd.d/` should include `dzn_icd.x86_64.json`.
5. **No further env vars required.** The code already passes `InstanceFlags::ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER` to wgpu (in `src/main.rs`), without which wgpu silently filters Dozen out and falls back to llvmpipe — Dozen self-reports as non-conformant and wgpu defaults to rejecting non-conformant adapters.

Expected outcome: `cargo run --release` shows `AdapterInfo { name: "Microsoft Direct3D12 (NVIDIA ...)", device_type: DiscreteGpu, ... backend: Vulkan }` in the startup log, and FPS lines reporting at the display refresh rate (60+ FPS, V-sync locked). Without these steps you'd see `AdapterInfo { name: "llvmpipe...", device_type: Cpu, ... }` and ~11 FPS.

If host-GPU passthrough is unavailable for your hardware, build and run the game natively on Windows or macOS instead.

### Migrating from an older WSL distro (e.g. Ubuntu 22.04)

If you're on Ubuntu 22.04 (jammy), Mesa Dozen isn't available — stock Mesa lacks it and the kisak-mesa PPA has dropped jammy. The path forward is moving to a fresh Ubuntu 24.04 distro. Old distro stays intact during the migration so you can verify the new one first and roll back if needed.

1. **From PowerShell on the Windows host**, install the new distro and confirm both are listed:
   ```powershell
   wsl --install -d Ubuntu-24.04
   wsl -l -v
   ```
   Note the **exact** name of the old distro from that list (often `Ubuntu`, `Ubuntu-22.04`, or similar) — you'll use it for unregistration later.

2. **Migrate personal data** from old → new. From inside the new 24.04 shell, your old distro's home dir is reachable through Windows at `\\wsl$\Ubuntu-22.04\home\<user>\` or via the path under `/mnt/wsl/instances/...`. What's worth carrying:
   - Any uncommitted/unpushed git work in repos outside `bevy-hex-game`. Push first, don't copy.
   - SSH keys (`~/.ssh/`) if you have any.
   - `~/.gitconfig` (small, can also recreate with `git config --global user.name/email`).
   - GPG keys (`~/.gnupg/`) if you use signed commits.
   - Any local files (notes, secrets) — `~/key.txt` was an example in our case.
   - `~/.config/gh` can be skipped — easier to just re-auth with `gh auth login`.
   - `.aws` / `.azure` / similar typically symlink to `/mnt/c/Users/.../...` — re-create the symlinks on the new side, no data copy needed.
   - Bash history (`.bash_history`) — optional.

3. **Set up the new 24.04 environment.** From inside the 24.04 shell:
   ```sh
   sudo apt update
   sudo apt install -y build-essential pkg-config libssl-dev git curl gh \
       libudev-dev libasound2-dev libwayland-dev libxkbcommon-dev \
       libx11-dev libxi-dev libxrandr-dev libxcursor-dev libxinerama-dev \
       mesa-vulkan-drivers vulkan-tools
   sudo add-apt-repository -y ppa:kisak/kisak-mesa
   sudo apt update && sudo apt upgrade -y
   vulkaninfo --summary | sed -n '/Devices:/,$p'
   ```
   The `vulkaninfo` output should now list both `Microsoft Direct3D12 (NVIDIA ...)` and `llvmpipe`. If only `llvmpipe` shows up, see the WSL2 setup section above for diagnostics.

4. **Install Rust and clone the repo fresh:**
   ```sh
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
   . "$HOME/.cargo/env"
   gh auth login   # GitHub → HTTPS → web browser
   git clone https://github.com/<your-fork-or-org>/bevy-hex-game ~/bevy-hex-game
   cd ~/bevy-hex-game && cargo run --release
   ```
   Expected: the startup log shows `AdapterInfo { name: "Microsoft Direct3D12 (NVIDIA ...)", device_type: DiscreteGpu, ... }` and `fps` lines around 60.

5. **Make 24.04 the default** (optional) so plain `wsl` opens it from now on:
   ```powershell
   wsl --set-default Ubuntu-24.04
   ```

6. **Once you're satisfied the new distro works** — game runs, you've got your personal data over, you can do everything you need — unregister the old distro from PowerShell:
   ```powershell
   wsl --unregister Ubuntu-22.04   # use the exact name from `wsl -l -v` in step 1
   ```
   **This is irreversible.** Anything left in the old distro disappears with it.

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
