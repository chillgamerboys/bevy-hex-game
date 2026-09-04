# V4 runtime authoring sources

These RON files are editable inputs to `hex_schematic::v4::parse_world` and
`compile_world`; none is compiled into the application. A source change needs a
new compile, not a Rust edit or application rebuild. `worldc` owns file loading,
package publication, preview and measurement. The compiler only returns memory.

`rich-region.ron` describes a materially different Grand-sized caldera landscape:
105,469 exact columns, alpine ridge, basalt mesa, dunes, hollow, layered substrate,
reservoir and directed falling river, a separate bridge deck, underground gallery
with explicit floor/air/roof and light domains, protected roads, local hard platforms,
and reusable trees, grass, crystal and procedural ruin voxel patterns. It is a new generic
fixture, not a claim to regenerate the selected V3 Grand reference.

`two-regions.ron` and `seven-regions.ron` place full radius-187 recipe instances with
rotated local frames. They use 16-by-16 **global** storage chunks. The seven-region
fixture contains 738,283 columns and 12 shared region boundaries. Individual chunks
can contain contributions from two or three authoring regions. Region IDs, recipe
IDs and chunk coordinates serve different purposes.

Authoring is geography plus rules plus exact constraints:

1. Place independently named region recipes and their exact world origins/rotations.
2. Specify bounded landforms and biome masks. Landforms add tapered integer fields;
   surface zones resolve explicit priority then stable ID.
3. Author pools, downhill channel controls, roads, bridge controls and cave rooms.
   `falls_after` indexes control segments; the segment's complete drop occurs at its
   final edge. Every resulting directed water ribbon is checked for drainage.
4. Supply reusable object occupancy and local feature densities. Candidate streams
   depend on stable IDs/seed/coordinates; there is no global top-K vegetation cap.
5. Add named hard height/material overrides. They establish ground constraints;
   every modifying stage checks them and reports both conflicting operator IDs.
   Named route ribbons likewise retain their complete exact support and headroom,
   even when an alternate route could still connect their endpoints.
   Soft banks/shoulders avoid hard constraints. An authored core intersection fails.
6. Declare one connection for every touching region pair. One global resolver supplies
   both neighbors' ground/water datums and walking ports. Boundary water may have
   explicit global upstream/downstream endpoints for an acyclic current across the
   seam; without endpoints it produces standing water.

Every recipe hub, cave entrance, required seam port and bridge endpoint must be
reachable by a two-level walker with one-level steps using the final exact stacked
terrain and object occupancy. This is an actual reachability check, not heightmap
sampling. Liquid, interior, light, anchor and object metadata remain gameplay data.

Known first-schema limits are explicit. Region footprints are finite hex disks.
Boundary ground uses one common solid datum across each crossing, so a natural
sloping shoreline needs a richer per-side boundary profile later. Directed seam
water currently has a common surface level; graded/falling segments are authored
inside regions. Basin/channel overlap requires matching water surfaces. Channels,
bridges and rooms are generic deterministic interval operations, not a hydraulic
simulation, erosion model or full structure grammar. Thin cave cover becomes a
constructed vault with the specified solid roof; hard terrain constraints still
win. Authoring validates complete regions in memory; runtime chunk residency is a
separate concern. The per-region compiler bound is radius 1024, not a total-world cap.

The optional compile artifact cache declares compiler version, material registry,
world identity/seed, region placement, recipe content and resolved shared seams.
It reuses whole region outputs and reuses pre-decoration geometry for feature-only
changes. It never reuses mutable runtime edits. Global package assembly and
validation always run. Fine per-operator invalidation and on-disk stage caches are
future work; current reports distinguish actual execution from reuse honestly.

Stock geometry provenance is carried in each exported rule: `plant/tall-narrow`,
`prop/crystal-spire` and `prop/grass-tuft` are verified entries of
`assets/art/object_catalog.ron`. Their exact blueprint placements are exported from
selected revision `bc06a8969532b807ec677928eee304bc28399386`; an independent fixture
test compares every occupied voxel against those source files. Explicit style to
world-material mappings preserve geometry while choosing V4 material policy. This
is **not** a claim to preserve the old two-dimensional `blocker_footprint` policy or
all visual-style/canopy metadata: stock rendering still consumes the original art
catalog. `procedural/limestone-tower` is explicitly a new interval prefab and is not
represented as an existing stock asset.

Region hubs publish `entry` feature summaries for safe metadata-only bootstrap.
Explicit Observation anchors are available for scenic reviews without authorizing
actor placement or forcing landscape changes. These summaries are world data, not
a gameplay visibility/disclosure grant.
