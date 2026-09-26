# Grand V4

The immutable world is generated from `world.ron` by the chunk compiler in
`crates/hex_schematic/src/v4/grand`. Generated packages are ignored by Git.

## Reproduce the package

Run `python3 tools/grand_package.py ensure --target-dir PATH`, using the existing
Cargo target directory for `PATH`. The default output is this directory's
`compiled/`. Set `HEX_GRAND_WORLD` to select a package elsewhere. For a deliberate
new revision, use `compile --output PATH`; existing packages are never overwritten.
A supplied `--worldc PATH` uses that executable without starting Cargo.

`--plain` retains the same full terrain, caves and water while omitting objects.
`python3 tools/grand_review.py --package PATH --output PATH.png` plots the actual
compiled terrain relief. This is a composition review, not a game screenshot;
objects, cave interiors, water shaders and gameplay require native validation.

## Measurements and geography

`python3 tools/grand_measure.py --write` independently reproduces canonical Grand
V3 column ownership from `grand-v3-template.ron`: radius 187, coarse pitch 22,
nearest axial hex distance with CellId tie break. The coast-enclosed mainland
includes inland lakes and their LakeIsland, and excludes offshore SeaIslands and
open ocean. It measures **93,326 columns**, rather than the full 105,469-column disk.
The enlarged mainland is one connected, hole-free footprint of **653,282 columns**,
exactly seven times that area. `measurement.json` records the canonical source hash.

Crystal Ascent is independently **22,183 columns**, seven times the embedded
Grand footprint's radius-32 disk (3,169 columns). It does not use the standalone
Crystal map's larger radius and does not receive the mainland scale a second time.
The finite ocean envelope has radius 900 and 2,432,701 total columns in 9,804 chunks.
Ocean volume is independent of the mainland-area measurement.

The source defines the beach start, five shrines, 14 encounter sites, the forest
and World Tree, a temple below its roots, a fort, the mountain garden and fountain,
a continuous descending watercourse through falls and valley lake into the bay,
Crystal terraces and frozen woods, the library stair route inside the mountain,
a separate uniform Shadow tunnel, and the offshore volcanic island. Site facts
carry exact supporting voxels and package identity in `arena-sites.ron`.

The offshore observation anchors define a clear 796.08-world-unit sea segment.
The independent unupgraded controller reference travels 793.9485 units in 45 seconds
from rest with a constant 9-unit/s tailwind. This establishes the layout scale;
actual travel with spatial wind and waves is a separate native measurement.

## Runtime and persistence

Terrain and object source use compact runs, compiled and admitted one chunk at a
time. The runtime retains at most 512 source chunks and 256 detailed render chunks.
Grand publishes exact loaded collision, bounded actor interests, and a compact
package-bound biome companion. The World Tree spans 21 chunks and has a stable
`grand/world-tree` identity; distant presentation is separate from seabed authority.

Map checkpoint records preserve sparse edits, partial voxel health, transaction
counters, impact batch counters and per-actor burrow sequences. An owned sparse
snapshot moves encoding and partition writes to the save worker. Restore validates
all saved partitions against immutable package bytes before adopting the new
session. The application restores gameplay and waits for exact local collision.

Focused compiler tests cover measured areas, shrine/encounter support, the full
408-step library route, separate overlapping tunnel voids, fountain water, beach
height, descending water continuity and a clear offshore sailing segment. The
opt-in `actual_grand_fresh_burrow_and_owned_checkpoint_round_trip` map test requires
`HEX_GRAND_WORLD` and checks compact terrain conversion and background checkpoint
round-trip against the real package.
