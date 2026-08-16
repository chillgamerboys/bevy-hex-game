# Set up and run Hex

The canonical source-build instructions for Hex. This gets a first-time contributor
from a checkout to a running game; contribution policy and required checks remain in
[CONTRIBUTING.md](../../CONTRIBUTING.md).

## Prerequisites

Install Rust through [rustup](https://rustup.rs/). The repository pins Rust in
`rust-toolchain.toml`, so rustup fetches the correct compiler on the first Cargo
command rather than requiring you to match it by hand.

macOS and Windows need no additional project-specific system packages. Linux needs
the packages in [Linux and WSL2](#linux-and-wsl2) before Bevy can build.

A cold build compiles the game engine and commonly takes 10–20 minutes. Incremental
builds after that usually take seconds.

## Build and run

Run both forms from the repository root:

```sh
cargo dev            # world inspector and live asset reload
cargo run --release  # optimized, without development tools
```

Use `cargo dev` while changing code or content. Use the release build when judging
gameplay or performance: the development profile optimizes game code less
aggressively, which is noticeable in work that runs every frame.

Always launch through Cargo rather than invoking `target/release/hex_game` directly.
The repository sets `BEVY_ASSET_ROOT` in `.cargo/config.toml`; a bare executable
otherwise looks for `assets/` beside itself. When it finds no assets, the process
does not crash—it renders a plain blue window with no meshes.

Packaged standalone builds are different: their `assets/` directory is deliberately
staged beside the executable.

## Asset Workshop

The standalone [Asset Workshop](../systems/asset-workshop.md) edits the canonical
palette, voxel styles, and object blueprints:

```sh
cargo editor
```

It discovers the repository by walking upward from the current directory. When
launching it from elsewhere, pass the repository explicitly:

```sh
cargo editor -- --project-root /path/to/bevy-hex-game
```

## First-run smoke test

The Main Menu should show exactly **Campaign**, **Sandbox**, **Multiplayer**, **Tools**,
and **Settings**. On a fresh data directory, Campaign must show exactly three empty
indexed cards. Tools must show Character Creator, Spell Creator, and a disabled Map
Creator labelled Coming Soon.

Open Campaign, choose **New Game** on slot 1, and confirm that the canonical Party
Trial resolves through Loading. You should see the Crossing's hex-prism terrain, sky,
a three-member player party, and a matching hostile party approaching from the
opposite bank. The slot remains empty until the first normal manual save.

Open Sandbox and confirm the default draft: Flat Arena, Hedge Mage in Party slot 1,
Raider in Enemies slot 1, and five empty slots on each side. In Map Browser, selecting
a generated map creates a pending seed; Back discards it, while Use Map commits it.
Authored maps must not show Regenerate. Open both roster routes and confirm the same
six-slot interaction is used for Party and Enemies.

| Default input | Action |
|---|---|
| Right-mouse drag | Look around the current camera focus; First Person keeps the cursor visible |
| `W` `A` `S` `D` | Pan the camera in Map mode; First Person remains click-to-move |
| Mouse wheel | Zoom in Map and Third Person; First Person keeps a fixed eye |
| `C` | Cycle Map → Third Person → First Person → Map |
| Hover a hex tile | Preview the reachable area and route |
| Left-click a hex tile | Move the piece along that route |
| Click a spell row, then a lit target | Aim a cast |
| `Tab` / `Enter` / `Q` | Cycle aimed units / confirm the cast or decision / cancel aiming |
| `Space` | End the current player turn; hostile turns cannot be skipped |
| `1`–`6` or a Party card | First activation inspects and centers that member; repeated activation opens Character Main View |
| `H` | Hide or restore all ordinary HUD components without changing saved preferences |
| `P` / `I` / `L` / `B` | Toggle or temporarily summon Party / Initiative / Activity / Action Bar |
| `V` / `F` | Open Character / Formation in the contextual Main View |
| Formation Main View | Switch Group/Solo movement, select a formation, and edit assignments |
| `R` | Recover the party while exploring |
| `F5` while paused in Campaign exploration | Atomically replace the bound Campaign slot |
| Click lattice cells, then `Enter` | Choose and confirm which cells incoming damage disables; required decisions cannot close first |
| `Escape` | Pause, leave a menu, cancel key capture, or close an ordinary Compact task as context allows |
| `Backspace` | Return to the owning Creator, Sandbox route, or Main Menu |
| Campaign New Game | Bind the canonical Party Trial to the selected empty slot without occupying it yet |
| Campaign Continue | Restore the compatible selected slot through Loading |

Inspect different allies, move the group, orbit the camera, pause, press `F5`, return
to Campaign, and Continue slot 1. Exercise every HUD shortcut, including a
master-hidden one-surface summon and Compact map-only state. Then open Settings,
change one volume, one HUD preference, and one keyboard binding; return to the Main
Menu and restart. A representative walk should also enter combat, exercise the
mid-combat/Sandbox save refusal, and inspect required-decision presentation while
ordinary HUD components are hidden.

This human route judges asset/native rendering, camera motion, input response, layout,
control feel, and post-restart presentation. Typed hooks and canonical snapshots—not
observation of the route—prove gameplay selection, positions, formation, active-play
time, exact persistence, save refusal, teardown, storage, and world reconstruction.

## Diagnostics

The game prints frame time, FPS, and entity count to the terminal once per second.
The startup log also names the graphics adapter. On Apple Silicon, for example, it
should resemble:

```text
AdapterInfo { name: "Apple M2 Max", device_type: IntegratedGpu, ..., backend: Metal }
```

If `device_type` reads `Cpu`, rendering is using a software rasterizer. That alone
can explain a frame rate below roughly 15 FPS. See
[troubleshooting.md](troubleshooting.md) for silent asset, shader, settings, and
presentation failures.

## Linux and WSL2

### System packages

Ubuntu and Debian-family systems need:

```sh
sudo apt install -y build-essential pkg-config libssl-dev \
    libudev-dev libasound2-dev libwayland-dev libxkbcommon-dev \
    libx11-dev libxi-dev libxrandr-dev libxcursor-dev libxinerama-dev \
    mesa-vulkan-drivers vulkan-tools
```

Native Linux can stop here. The remaining notes apply only to WSL2.

### WSL2 GPU passthrough

Bevy renders through `wgpu`. Without a Vulkan path to the Windows host GPU, WSL2
falls back to **llvmpipe**, a CPU software renderer that produces single-digit or
low-double-digit frame rates.

The working path is Mesa **Dozen**, a Vulkan-over-D3D12 translation layer:

1. Install the latest driver for the GPU on the Windows host. NVIDIA, AMD, and Intel
   drivers provide the D3D12 libraries under `/usr/lib/wsl/lib/`.
2. Use Ubuntu 24.04 or newer. Ubuntu 22.04's available Mesa path does not provide the
   required Dozen setup.
3. Install a Mesa build that includes Dozen. On Ubuntu 24.04, add the kisak-mesa PPA:

   ```sh
   sudo add-apt-repository -y ppa:kisak/kisak-mesa
   sudo apt update
   sudo apt upgrade -y
   ```

4. Verify the adapter:

   ```sh
   vulkaninfo --summary | sed -n '/Devices:/,$p'
   ls /usr/share/vulkan/icd.d/
   ```

   The first command should list the host GPU with
   `driverID = DRIVER_ID_MESA_DOZEN`; the second should include
   `dzn_icd.x86_64.json`.

No environment variables are required. The game already enables wgpu's underlying
non-compliant-adapter flag because Dozen self-reports as non-conformant; the flag is
inert on other platforms. A measured RTX 3080 host reached its 60 FPS V-sync limit
through Dozen, compared with roughly 11 FPS through llvmpipe.

## Where to go next

- Change designer-facing values through [config.md](config.md).
- Diagnose a wrong or silent window with
  [troubleshooting.md](troubleshooting.md).
- Run Direct multiplayer between remote testers through the temporary
  [Tailscale playtest procedure](remote-multiplayer-testing.md).
- Learn the project boundaries in [architecture.md](../architecture.md).
- Read the contribution workflow and required checks in
  [CONTRIBUTING.md](../../CONTRIBUTING.md).
- Browse every design, system, development, and planning document in the
  [documentation index](../README.md).
