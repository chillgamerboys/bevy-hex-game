# Northern Archipelago authoring

`world.ron` is a runtime-loaded NorthernSpec source consumed by the pure V4
compiler. The finite radius is700 unit hexes: approximately2425×2100 world units,
with11 islands in3 clusters. Mean sea is140 world units; occupied voxel levels
use0.35 height and0.35 top-face offset. The submarine relief remains solid beneath
the sea; the sea fill adds only the open interval between the existing bed and
exclusive level400. All solid materials are diggable; water is not.

Build a new immutable package with:

```sh
python3 tools/northern_package.py --output /absolute/path/to/new-package
```

The writer seals and production-validates each complete chunk, retaining only
one terrain chunk at a time. It writes `manifest.ron`, content-addressed chunk
fingerprints, `compile-receipt.json` and `northern-overview.ron`. Runtime consumers
can use `FileChunkSource::open_workspace` directly on the output directory.
No whole-region VoxelMap or dense voxel dictionary is generated.

The overview uses a4-unit regular X/Z grid with X varying fastest. `bed_heights`
are quantized upper faces of actual solid beds, including underwater relief;
`surface_materials` indexes the sorted `materials` palette. The grid extends to
the bounding rectangle, so runtime uses the published axial radius to distinguish
outside-world corners. `player_spawn` is feet height on a reserved dry bay ledge.
`anchors` contains scenic and settlement locations; exact authority remains in
chunk semantics. Trees and buildings use complete exact package object occupancy,
including cross-chunk influences and real grounding contacts. Their stable asset
names request occupancy presentation, not absent external catalogue blueprints.

The bay banks and widening outlet blend into the mountain; the player starts on
the actual dry slope with a reserved view of the central water surface. The bay
observation anchor is at mean sea level. The settlement follows a blended valley,
with small exact foundations under individual buildings instead of a single flat
ellipse. Its cultivated field occupies the open southern approach near world
XZ `(0, 612)`. Non-crater angular relief fades at each island center to avoid
undefined-azimuth height spikes. These terrain corrections use compiler identity
`hex-northern/2` and require a newly compiled immutable package.

`full_dressing:true` is now the default: the full-scale terrain/water checkpoint
passed static review before expanding woodland to the other sheltered slopes. It
authors 255 trees with the same reserved structures and exact tree blueprints.
Setting false retains the earlier bay/settlement-only dressing for a diagnostic
checkpoint. The footprint and terrain never change with this option.

Rendering, ocean motion and runtime performance require the combined candidate's
separate review. Compiler success alone makes no visual or native acceptance claim.
