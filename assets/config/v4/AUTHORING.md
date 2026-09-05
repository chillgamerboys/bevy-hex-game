# Authoring and checking a V4 world

Run these commands from the repository root. Edit the RON sources in this directory;
compiled world packages, saved gameplay edits and captures belong in separate output
directories. The full fixtures remain radius 187: 105,469 columns for one region,
210,938 for two and 738,283 for seven. This workflow does not reduce their dimensions.

## Build the authoring tool once

```sh
python3 tools/world.py build
```

The wrapper records the compiler source and binary identities. Subsequent commands
load world RON at runtime and do not invoke Cargo. Editing these map files does not
invalidate the compiler build. Changing compiler/runtime contract code, dependencies
or the lockfile requires another explicit build; an identity mismatch is a refusal,
not permission to use a stale executable. Use `--target-dir DIRECTORY` before the
command when reusing an explicitly managed authoring build target.

## The normal edit, validate, compile and preview loop

Start by changing one named recipe operation in `rich-region.ron`, or an instance
placement/connection in `two-regions.ron` or `seven-regions.ron`. Preserve stable IDs
when changing the same feature: they participate in deterministic candidate streams
and cache dependencies. A recipe change affects each region using that recipe;
duplicate a recipe under a new ID when only one region should change.

```sh
python3 tools/world.py validate --source assets/config/v4/two-regions.ron
python3 tools/world.py compile --source assets/config/v4/two-regions.ron --output .context/v4/workspaces/two-regions
python3 tools/world.py inspect --package .context/v4/workspaces/two-regions
python3 tools/world.py preview --package .context/v4/workspaces/two-regions --output .context/v4/workspaces/two-regions/review.html
```

`validate` runs the actual compiler and final exact topology checks. It is more than
a RON/schema check. `compile` publishes an immutable revision beneath
`packages/<manifest-fingerprint>/` and atomically advances the workspace's
`current.ron`. Keep passing the stable workspace directory to preview and the
explorer; do not manually edit the pointer or generated chunks. A failed publication
leaves the last valid revision selected. Retained revisions provide exact review
and regression inputs. A workspace cannot switch to a different world ID.

Compilation also refreshes `review.html` and a compile receipt. The explicit preview
command is useful when producing another review location. The HTML uses summaries
and feature metadata, without loading detailed terrain. Use it for overall geography,
region placement and seam review; exact walking, cave clearance, water continuity and
edits require the compiler/runtime checks. A running explorer retains the package
revision it opened; restarting against the stable workspace opens its current
revision. This is not yet live source hot reload.

An existing immutable package directory containing `manifest.ron` remains readable,
but cannot be used as a writable workspace. Publish into a new workspace directory
when migrating earlier package outputs. Source changes and mutable gameplay saves
have different identities: use a fresh save after changing the base package unless
an explicit migration is provided. Never apply a saved delta to a mismatching base.

## Author geography, rules and hard constraints

Each `RegionSpec` chooses an exact origin, radius, rotation and recipe. Recipe
coordinates are local; connection flow endpoints are global. Storage uses global
16×16 axial chunks, which can contain several regions. Authoring region size is not
the streaming or encounter size.

Use named landforms and biome masks for broad terrain, then pools/channels, graded
routes, bridges and caves for intended structure. Features supply reusable voxel
occupancy, density and optional explicit roots. Stock exports retain their catalog
identity and blueprint provenance; `procedural/limestone-tower` is explicitly a new
prefab. Add a local hard override only where the authored surface height or material
must remain fixed. Observation anchors describe scenic positions without requiring
access or flattening the landscape.

Hard overrides and protected route ribbons are constraints. A later operator that
breaks them must fail with the conflicting IDs. Required hubs, cave entrances,
bridge endpoints and seam ports must connect through final stacked solid surfaces.
The ordinary walker needs two clear levels, at most one level of climb/drop, and
two levels of shared lateral aperture. Two adjacent floors can each be standable
yet have an impassable low lintel between them. Do not silence this failure by
switching the validator to a height-only check.

Declare one shared connection for every touching region pair. One resolver controls
both sides' ground/water datums and access. Directed seam water needs explicit
upstream/downstream endpoints; absent endpoints mean standing water. Keep air,
solid strata, water and object occupancy distinct. Unloaded runtime data is unknown,
not air or an implied traversable gap.

Current limits: finite hex-disk region footprints; radius 1024 per compile region;
a common ground datum and common water level at each seam; graded/falling water
inside regions; deterministic volume operations rather than hydraulic or erosion
simulation. Compilation still assembles and validates full output in memory. The
in-process cache reuses declared whole-region and geometry-before-features stages;
separate CLI invocations currently start clean. Fine per-operator invalidation,
on-disk stage caching and unbounded procedural generation remain future work.

Machine compile time is not active authoring time. **Active authoring hours remain
unmeasured.** Record time spent designing, editing, diagnosing and reviewing maps
separately before claiming that the workflow saves a measured number of human hours.

## Windowless acceptance walks

The four scripts in `walks/` use the actual explorer movement, step, edit and save
requests. They never teleport actors. Each `MoveTo` stays within the local query's
64-column radius; the longest leg below is 36 columns. The same scripts apply to
the checked two- and seven-region fixtures, whose first two party IDs are
`party/region-0` and `party/region-1`.

Use `--capture` for every automated launch. It selects the windowless image target;
`--walk` is rejected without it. Launch through Cargo so the repository's asset
root is applied. A source-only RON edit does not rebuild unchanged Rust game code.
Use the coordinated existing Cargo target when one is already assigned, rather
than starting another engine build target. Capture paths must be new.

After compiling the two-region workspace above, choose a fresh run directory:

```sh
world_workspace=.context/v4/workspaces/two-regions
walk_run=.context/v4/walks/run-001
mkdir -p "$walk_run"

cargo run --release -p hex_game --features v4-world --bin hex_v4 -- --world "$world_workspace" --focus 184,-88,30 --radius 16 --frames 4800 --walk assets/config/v4/walks/seam-reversal.ron --capture "$walk_run/seam-reversal.png"

cargo run --release -p hex_game --features v4-world --bin hex_v4 -- --world "$world_workspace" --focus 184,-88,30 --radius 16 --frames 4800 --walk assets/config/v4/walks/party-independence.ron --capture "$walk_run/party-independence.png"

cargo run --release -p hex_game --features v4-world --bin hex_v4 -- --world "$world_workspace" --focus 184,-88,30 --radius 16 --frames 14000 --save "$walk_run/save" --walk assets/config/v4/walks/edit-residency.ron --capture "$walk_run/edit-residency.png"

cargo run --release -p hex_game --features v4-world --bin hex_v4 -- --world "$world_workspace" --radius 16 --frames 4800 --save "$walk_run/save" --walk assets/config/v4/walks/edit-residency-resume.ron --capture "$walk_run/edit-residency-resume.png"
```

The first edit run requires a new save directory. The resume run is a **second
process using that same save**, without `--focus`: its initial positions must come
from the checkpoint. It also uses `StepOnce` before setting step mode, checking that
the saved mode was restored. Choose a different run directory for another pair.
For seven regions, compile `seven-regions.ron` into its own workspace and substitute
that workspace in the same commands. Do not share saves across the two world IDs.

| Script | Typed acceptance witness |
| --- | --- |
| `seam-reversal.ron` | Walk from region 0 into region 1 and across the q=192 chunk boundary; issue a new goal one update after `StepOnce` starts a reversing leg; settle at the new goal and return. |
| `party-independence.ron` | Party 0 stops after its first turn step with one step queued while distant party 1 reaches its continuous destination; then both finish their routes. |
| `edit-residency.ron` | Remove exactly `(182,-88,30)`, retain soil at level 29, save, walk far enough to observe chunk `(11,-6)` unloaded, return and read the same edit after reload; save actor position and step mode. |
| `edit-residency-resume.ron` | A fresh process restores the saved actor positions, mode and exact voxel edit, walks through the ordinary controller and saves again. |

The launch focus is `(184,-88,30)`. Required dry boundary samples include
`(187,-90,30) → (188,-90,30)`. The edit is in chunk `(11,-6)`, whose q coordinates
are at most 191. The excursion endpoint `(250,-120,30)` is therefore at least 59
columns from every column in that chunk, beyond the explorer's 48-column retention
radius when `--radius 16` is used. The other party stays near `(375,-187,40)`.
Increasing radius can invalidate this unload witness; do not change it casually.

These locations were selected from exact compiled columns, object occupancy and
boundary metadata in package fingerprints `2be53b2a89e3877b` (two regions) and
`7b460087b405a3a3` (seven regions). Independent bounded integer path queries found
the same leg lengths and supports in both packages, with at most 256 settled
surfaces per queried leg. This records fixture selection, **not an executed Bevy
acceptance pass**. Re-run the scripts after relevant source or controller changes.

Capture JSON records script identity/fingerprint, typed commands, exact actor
supports/motion/queued steps, observed chunk revisions and save/checkpoint counters.
Completion must come from successful exact movement observations; waiting alone
does not pass. Static PNGs still require visual inspection. Automated typed motion
does not clear `HUMAN-MOTION-PENDING`, camera feel, popping or translucent effects.
Package metadata also does not establish complete stock-asset styling or domain
lighting in the renderer; retain those presentation gates until they are exercised.
