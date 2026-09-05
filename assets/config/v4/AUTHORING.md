# Authoring and checking a V4 world

Run these commands from the repository root. Edit the RON sources in this directory;
compiled world packages, saved gameplay edits and captures belong in separate output
directories. The full fixtures remain radius 187: 105,469 columns for one region,
210,938 for two and 738,283 for seven. This workflow does not reduce their dimensions.

The seven-region fixture uses four recipes: `caldera` for regions 0/1,
`ochre-dunes` for 2/5, `pine-uplands` for 3/6 and `frost-spires` for 4. These vary
landforms, surface materials, caves and feature populations; they are not seven
rotations of one terrain recipe. Regions 0/1 retain the shared seam-walk corridor.

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
python3 tools/world.py probe --package .context/v4/workspaces/two-regions --at 187,-90
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

The compiler is serial; there is no configurable compiler worker count to test.
The complete manifest and its spatial index remain in memory; terrain, saved edit
bodies and nearby fine knowledge are paged. Default file limits are 64 MiB for a
manifest/save head and 8 MiB for a chunk or transaction. These finite metadata and
operation limits are not an infinite-world implementation.

## Measured authoring loop

The compiler/3 run at Git HEAD `dcf665826b37cc59c97bdda5bf46dbfba009fb20`
produced these exact fixture identities:

| Source | Columns | Package fingerprint | Full compile command wall |
| --- | ---: | --- | ---: |
| `rich-region.ron` | 105,469 | `c3975dd9eedad127` | 3.961 s |
| `two-regions.ron` | 210,938 | `892b69b7d372d08f` | 6.734 s |
| `seven-regions.ron` | 738,283 | `bec7b11980145a09` | 23.524 s |

Twenty successive ordinary source edits matched clean compilation exactly:
incremental strict compile p50 **0.768 s**, p95 **0.782 s**. The 21 snapshots include
the baseline; changes comprise seven feature-density, seven landform-height and
six biome-material edits. These are one-region, in-process cache measurements,
not end-to-end publish/preview latency. The full compile wall times above include
wrapper identity checks, reading, compilation, validation, publication and preview.
OS caches were uncontrolled; neither figure claims a cold-storage benchmark.

The run's compiler source SHA-256 is
`7a42efde52af877b9fba2712b09587b076f1f9a16be1d6c70b64974cfe11dfd3`;
its binary SHA-256 is
`6265e7abb8707fce896caf1b5a0861b7db75ff0095c9b46668dfb12e5ecc9610`.
Detailed receipts and identity sidecars are retained in the coordinator's
`work/v4-final-measurements/`, summarized by `measurement-summary.md` there.
These fingerprints identify the measured inputs, not future edits to these files.

Machine compile time is not active authoring time. **Active authoring hours remain
unmeasured**, including the four-hour authoring target. Record design, editing,
diagnosis and review separately. Renderer frame time, RSS and human motion review
are also separate acceptance gates.

## Windowless acceptance walks

The scripts in `walks/` drive the ordinary actor-owned movement, step, edit and save
requests. They never teleport actors. `party-view-rebase.ron` additionally switches
the detailed view between distant moving parties and requires three origin rebases.
Wait budgets count uncapped render updates, so the fixtures deliberately allow
substantial headroom; timeout errors include elapsed time and actual actor state. Every `MoveTo` stays within the local query's
64-column radius. Party identities are **`party/0`, `party/1`, ...**, independent
of region ownership. `--parties N` requests exactly N actors (1..7): declared region
entries first, then deterministic farthest usable gameplay/transit anchors. If the
source has too few safe anchors, startup fails and the author must add them.

Use the strict review driver below after committing a clean candidate. It invokes
`cargo run --locked --release -p hex_game --features v4-world --bin hex_v4` with an
explicit `--capture`; no native window opens. It verifies exact source bytes before
and after the run, actual package identity, typed work and PNG coverage. The supplied
`--output` directory **must not exist**; do not create it with `mkdir` first.
`--target-dir DIRECTORY` reuses the coordinated engine target. `--profile map-test`
and `--dirty-diagnostic` are explicitly unapprovable diagnostics.

All examples require **600 consecutive fully settled updates** after loading,
meshing, motion, edits and knowledge persistence finish. Headless updates run without
a fixed sleep, so the overall `--frames` deadline is not seconds of elapsed time.
The longer deadlines below permit asynchronous work and do not relax script checks.
The game accepts settle counts 12..10,000 below the overall 1..100,000 frame deadline.
Use `python3 tools/v4_review.py --help` for the supported flags.

After compiling the two-region workspace above, use fresh output/save paths:

```sh
python3 tools/v4_review.py --package .context/v4/workspaces/two-regions --output .context/v4/reviews/seam-001 --name seam-reversal --focus 184,-88,30 --view orbit --radius 16 --parties 2 --frames 40000 --settle-frames 600 --walk assets/config/v4/walks/seam-reversal.ron

python3 tools/v4_review.py --package .context/v4/workspaces/two-regions --output .context/v4/reviews/parties-001 --name party-independence --focus 184,-88,30 --radius 16 --parties 2 --frames 40000 --settle-frames 600 --walk assets/config/v4/walks/party-independence.ron

python3 tools/v4_review.py --package .context/v4/workspaces/two-regions --output .context/v4/reviews/edit-001 --name edit-residency --focus 184,-88,30 --radius 16 --parties 2 --frames 40000 --settle-frames 600 --save .context/v4/saves/edit-001 --walk assets/config/v4/walks/edit-residency.ron

python3 tools/v4_review.py --package .context/v4/workspaces/two-regions --output .context/v4/reviews/resume-001 --name edit-resume --radius 16 --parties 2 --frames 40000 --settle-frames 600 --save .context/v4/saves/edit-001 --walk assets/config/v4/walks/edit-residency-resume.ron
```

The first edit run requires a fresh save. Resume is a **second process using that
same save**, with **no `--focus`**: exact actor supports and step mode must come from
the checkpoint. It uses `StepOnce` before setting step mode to check restoration.
For seven regions, compile `seven-regions.ron` into its own workspace and substitute
that workspace. Never share saves across different base fingerprints or world IDs.

| Script | Intended typed witness; execution still required |
| --- | --- |
| `seam-reversal.ron` | Cross the region seam and q=192 storage boundary, reverse an active step, settle at the new goal and return. |
| `party-independence.ron` | Party 0 waits after one turn step while distant party 1 reaches its continuous destination; both then finish their routes. |
| `edit-residency.ron` | Remove `(182,-88,30)`, retain soil below, save, witness chunk `(11,-6)` unloaded, return and read the edit after reload; save actor support and step mode. |
| `edit-residency-resume.ron` | Fresh-process restoration of actor support/mode and exact voxel edit, followed by ordinary walking and another save. |

The seam launch focus is `(184,-88,30)`; required dry seam samples include
`(187,-90,30) → (188,-90,30)`. The terrain edit's chunk ends at q=191. Its excursion
to `(250,-120,30)` is at least 59 columns from that chunk; the other party remains
near `(375,-187,40)`. At radius16, the surrounding retention radius is48 and ordinary
sight needs36 plus a one-column fringe. Route-directed adaptive prefetch uses measured
preparation latency/backlog to prioritize at most16 upcoming exact route steps;
its request has radius8 and retention16. A completed route drops that request.
Increasing the view radius or leaving another actor/operation nearby can defeat an
unload witness, so the scripts require a typed unloaded observation rather than
assuming that walking some distance was sufficient.

## Remove a stock object, reload it and restart

Compile `rich-region.ron` into a separate stable workspace, then run both commands
in sequence with a new save and distinct new outputs:

```sh
python3 tools/world.py compile --source assets/config/v4/rich-region.ron --output .context/v4/workspaces/one-region

python3 tools/v4_review.py --package .context/v4/workspaces/one-region --output .context/v4/reviews/object-001 --name object-residency --focus=-72,20,40 --view orbit --radius 16 --parties 1 --frames 65000 --settle-frames 600 --save .context/v4/saves/object-001 --walk assets/config/v4/walks/object-residency.ron

python3 tools/v4_review.py --package .context/v4/workspaces/one-region --output .context/v4/reviews/object-resume-001 --name object-resume --view orbit --radius 16 --parties 1 --frames 65000 --settle-frames 600 --save .context/v4/saves/object-001 --walk assets/config/v4/walks/object-residency-resume.ron
```

Use the equals form for a negative `--focus` so Python's argument parser treats the
entire coordinate as its value. The driver passes the correctly separated game
arguments to Cargo. The restart command deliberately omits focus.

The target is the real stock `plant/tall-narrow` instance
`region-0/mountain-pines/q-77-r16`. Final compiler/3 probes establish its root
`(-77,16)`, grass support50, timber `[51,63)` and foliage `[63,67)`. Neighbor
`(-77,15)` has a projected foliage contribution `[62,65)` in a different storage
chunk. `object-residency.ron` observes both, removes the exact named instance from
voxel `(-77,16,51)`, confirms both influences gone and grass50 retained, saves, walks
the clear support40 corridor to `(5,20)`, witnesses root chunk `(-5,1)` unloaded,
and returns to confirm the removal survived reload. The longest leg is32 columns.
The second script confirms removal after a fresh-process restore and performs two
short real moves before saving again. These probes and scripts specify acceptance;
**they are not a claim that a capture has executed or passed**.

For an explicitly requested interactive session, hold **D and click** to remove the
exact object at the clicked voxel; clicking terrain with D requests a voxel removal.
The clicked stock part's ID wins when overlapping objects are present, otherwise a
stable first exact influence is selected. **Escape cancels** a pending object edit.
The owner checks current source/revisions, loads and pins every affected chunk, and
protects current and next-step actor bodies/supports. `RemoveObject` in scripts uses
the same typed request; `WaitObject` needs a resident chunk and a settled successful
command. Unloaded data never counts as evidence of absence.

## Knowledge, presentation and remaining review

Terrain/object edits are partitioned changes over an immutable base. Actor support,
mode and selection are checkpointed separately. Knowledge persists automatically
through a bounded background queue under `--save/knowledge`; without a save path it
uses a transient store. Exact changed partitions use compare-and-write acknowledgments;
unchanged observations do not require rewriting pages. Explicit save/capture completion
waits for all active actors to have a complete observation and for pending knowledge
writes to finish. Failed persistence is reported rather than treated as completion.

The map/minimap's base geography is a public authoring summary. Exploration and known
landmarks are private overlays from the **selected principal's actual sight**, not
renderer residency or another party. Fine memories outside the active neighborhood
page out; compact discovery remains. Dormant landmark detail may be marked incomplete
until its partition is read. An unseen removed object stays remembered until exact
sight proves it absent; deleting it in world authority does not leak that fact to an
unobserving party. This explorer does not claim full hidden-terrain fog presentation.

Known stock assets use their existing material/style baker and clipped resident art.
Unresolved assets or footprints that cannot be matched retain explicit exact voxel
proxies and appear in unresolved-art receipt fields; the procedural limestone tower
is not presented as reused stock art. Proxy availability proves occupancy continuity,
not visual fidelity. Renderer assets, local-origin camera positions and mesh publication
have their own lifecycles and checks, separate from world authority.

The driver records package/source identity, script commands and supports, pending
operations, revisions/save counts, resident/rendered/art/knowledge work and actual
settled frame samples. Static PNGs still need visual inspection. Mechanical success
cannot establish native input feel, camera motion, popping, transparent/emissive
stability, 60 FPS or aesthetic approval. Keep **HUMAN-MOTION-PENDING** and unreviewed
presentation status until those checks are actually performed. This authoring guide
does not add production online gameplay, encounter/combat merging, an infinite
catalogue, hydraulic simulation or a universal procedural generator.
