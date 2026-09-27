# Grand V4

`world.ron` selects the versioned `geography-r02.json` authoring document. The
compiler in `crates/hex_schematic/src/v4/grand` consumes its terrain controls,
water profile, landmark frames, cave envelopes, routes and review cameras.
Generated packages are immutable and ignored by Git. The approved design and
calibration are preserved in [the design record](../../../../docs/planning/waves/grand-v4/approved-r02/provenance.json).

## Reproduce the package

Run `python3 tools/grand_package.py compile --output PATH --target-dir TARGET`,
using a fresh package directory and the existing shared Cargo target. Coordinate
heavy builds against that target. `ensure` can reuse an exactly matching package;
existing packages are never overwritten. `HEX_GRAND_WORLD` selects the package at
runtime. A supplied `--worldc PATH` remains explicitly unverified provenance.

`--plain` retains the terrain, caves and water while omitting object dressing.
The exact geographic source bytes are included in the authoring identity in both
modes. `tools/grand_capture.py --plain` captures actual plain game terrain;
omitting that option requires the dressed package. `tools/grand_review.py` only
plots compiled relief and is not a game screenshot.

## Measurements and geography

`python3 tools/grand_measure.py --write` measures the original Grand V3 mainland
and updates only `measurement.json`. It never changes the authored world. Original
ownership uses radius 187, coarse pitch 22, nearest axial distance and CellId tie
breaking. The coast-enclosed mainland includes its inland lakes and garden island,
while excluding open ocean and offshore islands: **93,326 columns**, rather than
the 105,469-column full disk.

The production mainland target is **653,282 connected columns**. Independent
revision02 calibration samples the approved continuous terrain on the actual hex
lattice and counts its largest connected mainland component, excluding a small
detached islet. It requires no arbitrary coastline trimming. Horizontal model scale
is 0.8838784715190828; vertical scale stays 1. The compiler independently checks
its result, recorded in each package's `compile-receipt.json`.

Crystal Ascent independently reserves **22,183 columns**, seven times the original
embedded Grand footprint of 3,169 columns. Its climb occupies that larger mountain
feature; the central open shaft is not its entire footprint. The complete finite
ocean envelope has radius 1052 and 3,323,269 columns. Ocean area is separate from
the mainland measurement.

One connected mountain complex encloses the hidden upper lake and garden island.
The Earth temple is at the bottom of Crystal Ascent; the climb reaches the Frozen
Woods corridor and lake shore. A narrow outlet feeds a dominant near-vertical fall
and connected shorter drops into the valley lake. The nearby World Tree, river
outlet, small huts and irregular clearings form one composition. There is no fort.
The waterfall cave reaches the branching lower library, while the straight Shadow
route stays separate. The southern bay and distant volcano retain distinct roles;
the volcano has a landing and an ordinary walking route to its crater.

The approved geometric shore separation is approximately 800 world units. Actual
unupgraded sailing is measured from both the accessible western mainland shore
(`sailing_start`) and the farther starting bay (`sailing_start_bay`). The 45-second
reference must not be mistaken for an already measured bay crossing.

Five shrines and fourteen encounter sites retain stable authored identities. Site
facts publish exact supporting voxels and package identity in `arena-sites.ron`.
Shrine sculptures use opaque voxel materials and add no incidental hazard.

## Runtime and persistence

Terrain and objects use compact runs, compiled and admitted one chunk at a time.
Runtime residency stays bounded at 512 source chunks and 256 detailed chunks.
Package bounds govern admission rather than the previous radius-900 envelope.
The World Tree uses one stable `grand/world-tree` object with taper, exposed
branches, roots and an asymmetric deep crown. Its presentation limits are 20,000
columns, 25,000 runs and 750,000 indexed vertices; record actual package counts
before making performance claims. Ordinary forest recipes reuse the later V4
Dragon/Goblin expedition's trees and reserve playable approaches.

Distant inland water publishes exact faces and surrounding terrain from the same
columns as detailed geometry, including cave surfaces and a nonrendered neighbor
collar. This preserves the waterfall profile across render distances. Persistent
edits suppress stale distant faces, and detailed terrain replaces them atomically.
These presentation facts do not create collision or pin source chunks. Ocean,
boats and swimmers retain the shared environment and accepted movement behavior.

Checkpoints preserve sparse edits, partial voxel health, transaction counters,
impact batches and actor burrow sequences. Restore validates all partitions and
exact package/content identities before adopting a staged session. Old packages
and incompatible saves remain preserved. Full acceptance combines measured terrain,
actual movement/restart checks, fresh static and temporal game renders, and the
native play/performance route described in [the candidate guide](../../../../docs/development/grand-v4.md).
