# When something looks wrong

**Several failure modes here produce no log output at all.** A clean log is not
evidence that a change worked — look at the window. This is the single list of
symptoms; the copies that used to live in `CLAUDE.md`, the architecture doc and the
config guide had already drifted apart from each other.

Symptoms specific to the map's internals — a tile in the wrong place, a piece sunk
into terrain, a dig that removes the wrong thing — are in
[`crates/hex_map/CLAUDE.md`](../../crates/hex_map/CLAUDE.md), next to the code that
causes them.

## The window is wrong

| Symptom | Cause |
|---|---|
| Plain blue window | Assets not found: Bevy fell back to `ClearColor` with no meshes. Run through `cargo`, never the binary directly, and check `BEVY_ASSET_ROOT` in `.cargo/config.toml` |
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

**It returned to the title screen with a notice.** Terrain generation or actor
spawning failed after loading succeeded — the reason is on screen and in the log.

## Editing settings

**A change had no effect.** Check that you saved the file, and that you are running
`cargo dev` rather than `cargo run --release`: only the dev build watches files.
Some values are only read when the world is built — press `BACKSPACE` and pick the
scenario again. Which values those are is the hot-reload table in
[config.md](config.md).

**You want to undo everything.** These files are tracked in git:

```sh
git checkout assets/config/
```

## And when the tests are green anyway

Every serious bug in this codebase so far was found by a person looking at the
window, and the worst of them were green across the compiler, clippy, the whole test
suite and CI. The headless tests cannot see a black sky, a gap between tiles, or a
piece standing inside a column. That is why `main` moves only by promotion, after
someone has played the build.
