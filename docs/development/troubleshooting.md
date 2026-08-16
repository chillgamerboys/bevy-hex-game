# When something looks wrong

**Several presentation failures here produce no log output at all.** A clean log is
not evidence that the window is correct, so inspect it for these visual symptoms.
Screenshots/frames are valid for static camera, UI, and rendered-map symptoms;
video/human checks are valid for motion, input response, control feel, and taste. That
inspection is never gameplay or exact world logic evidence when typed hooks, state,
messages, logs, snapshots, or deterministic contracts can prove the claim. This is
the single symptom list; the copies that used to live in `CLAUDE.md`, the architecture
doc and the config guide had already drifted apart from each other.

Symptoms specific to the map's internals — a tile in the wrong place, a piece sunk
into terrain, a dig that removes the wrong thing — are in
[`crates/hex_map/CLAUDE.md`](../../crates/hex_map/CLAUDE.md), next to the code that
causes them.

## The window is wrong

| Symptom | Cause |
|---|---|
| Plain blue window | Assets not found: Bevy fell back to `ClearColor` with no meshes. Run through `cargo`, never the binary directly, and check `BEVY_ASSET_ROOT` in `.cargo/config.toml` |
| UI panels and button outlines render, but text and icons are absent | The source-tree binary was launched directly and is searching under `target/*/assets`; the log will report missing fonts followed by other assets. Stop it and launch from the repository root with `cargo dev` or `cargo run --release -p hex_game` |
| Black sky | The sky shader failed to load, or the dome was culled — check `shaders/sky.wgsl` and that `SkyMaterial::specialize` sets `cull_mode = None` |
| Clouds smeared into streaks | A sky-projection singularity. Check the mirroring in `sky.wgsl`, and verify from the *gameplay* camera — it looks down, so it sees the half of the sky a level screenshot never shows |
| Terrain looks flat and washed out | Fill light competing with the sun. The terrain has no texture, so shadows are the only thing giving it shape; see `lighting.ron` |
| A piece floats above or sinks into the terrain | A tile's transform disagrees with its span, or its `TilePos` is the run's base rather than its surface. See the map crate's own list |
| Movement looks wrong | A speed unit conversion. Speeds are world units per **second**, driven by `Res<Time>` |
| The game appears frozen | It is paused. The overlay exists precisely because this was indistinguishable from a hang |

## It will not start

**Stuck on "loading…" during initial startup.** A RON settings file failed to parse —
for terrain, whichever world the chosen scenario names. The terminal names the file,
the line and the column; the usual cause is a missing comma.

This is deliberate. The game refuses to start without one valid value for every
setting, because a default that silently diverges from what someone wrote is worse
than a stall. Once a valid value exists, a failed hot reload keeps it and reports the
error instead — fix the file and save it again.

**It returned to the Main Menu with a notice.** Terrain generation or actor spawning
failed after loading succeeded — the reason is on screen and in the log. A launch
owned by Creator returns through Sandbox instead so its retained Creator route remains
available.

**A Campaign card is Invalid or Continue is unavailable.** The card preserves the
record and explains the explicit slot, schema, build, scenario/content digest, or
generator mismatch. There is no in-product overwrite or delete action for an Invalid
record: reopen it with the compatible build/content, or restore a known-good
`campaigns.ron` backup. If `campaigns.ron` is absent, startup tries the legacy
`resume.ron` once: a valid record becomes slot 1, while invalid data and the legacy file
are preserved. Set `HEX_GAME_DATA_DIR` to an isolated directory when testing this flow.

**Start Sandbox is unavailable.** Resolve the first reason shown: wait for maps,
choose an available map, add at least one character to each side, then repair the
first Party or Enemy slot that is not Map-ready. Regeneration changes a pending seed;
Back intentionally discards it and Use Map commits it.

**Settings reported that defaults were restored.** `preferences.ron` was corrupt or
from an incompatible version. The game keeps running with safe defaults and reports
the problem instead of partially applying the file. The exact platform directories
and override are in [config.md](config.md#local-settings-are-not-authored-config).

**The log says an open combat decision was dropped after leaving a scenario.** This is
intentional teardown when `BACKSPACE` or another state change exits combat while a
defender-choice prompt is open. The decision names session-local units and must not
survive into the next scenario. The same warning without an explicit screen/combat
exit is a bug.

**A remote Direct guest cannot join.** Use the
[Tailscale remote-playtest runbook](remote-multiplayer-testing.md#diagnose-a-failed-connection)
when that private test network is in use. First prove the guest can reach the shared
host with `tailscale ping`; then verify that the issued `HEX1` code advertises the
host's Tailscale address, that the selected UDP port and local firewall permit inbound
traffic, and that both processes run the exact candidate. Never work around a typed
protocol, build, content, certificate, or map mismatch.

## Editing settings

**A change had no effect.** Check that you saved the file, and that you are running
`cargo dev` rather than `cargo run --release`: only the dev build watches files.
Some values are only read when the world is built — press `BACKSPACE` and relaunch
from Campaign, Sandbox, or a test-support request. Which values those are is the
hot-reload table in [config.md](config.md).

**You want to undo everything.** These files are tracked in git:

```sh
git checkout assets/config/
```

## Asset Workshop

The complete authoring workflow and controls are in
[the Asset Workshop contract](../systems/asset-workshop.md).

**The editor cannot find a project.** Run `cargo editor` from inside a checkout, or
pass its root explicitly with `cargo editor -- --project-root /path/to/repository`.
The root must contain `assets/art/palette.ron`, `assets/art/voxel_styles.ron`, and
`assets/art/object_catalog.ron`.

**Save is disabled for an object.** Calibration and newly created objects need Save As
before ordinary Save has a tracked destination. A saved object must also satisfy its
category, connectivity, style-reference, bounds, origin, and mask contracts. The
object inspector reports the current intrinsic validation error.

**Every tracked write is blocked.** The toolbar distinguishes an external-file change
from a recovery conflict. Reload accepts the current disk files as the new baseline
and discards local drafts. Save As can preserve an object under a new id, but it does
not silently resolve dirty shared palette or style catalogs. External object and
manifest additions are reloaded and merged when Save As can prove their graph is
coherent.

**A recovery prompt will not go away.** The Workshop deliberately leaves an invalid
or unknown-version recovery file untouched and pauses autosave. Discard it explicitly
only after deciding it contains no work to preserve. Recovery files are untracked at
`.context/asset-workshop/recovery/`; tracked art is never repaired or overwritten by
the prompt.

**Review is unavailable.** Review export requires a saved, clean object and a valid
palette-style-object dependency graph. Save or reload first, then resolve the
validation error shown in the inspector. Output is untracked under
`.context/asset-workshop/reviews/`.

## And when the tests are green anyway

Every serious bug in this codebase so far was found by a person looking at the
window, and the worst of them were green across the compiler, clippy, the whole test
suite and CI. The headless tests cannot see a black sky, a gap between tiles, or a
piece standing inside a column. That is why `main` moves only by promotion, after
someone has played the build.
