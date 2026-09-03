# Authored interiors

An interior is exact world metadata, not a visual effect or a material convention.
`InteriorRegions` maps stack-safe floor and entrance `TilePos` values plus exact roof
voxels to one `InteriorRegionId`. The map publishes that resource; perception derives
ambient light domains from it, while world presentation projects roof voxels onto
disposable rendered runs for an explicit review cutaway. Movement and line of sight
continue to use ordinary terrain and occupancy.

Ordinary gameplay never removes an interior roof. A review-only full cutaway may hide
the roof of the interior occupied by the selected character, but it does not edit the
voxel map, create headroom, admit daylight, reveal enemies, or alter picking. Fog and
cutaway visibility compose as independent occlusion reasons. Features whose support
lies on hidden roof runs follow that same review visibility so trees cannot remain
floating above an exposed passage. Teardown, regeneration, and gameplay re-entry must
remove and rebuild both roof and feature presentation from current published facts.

## Crystal Mountain

Crystal Mountain joins the complete tunnel and Crystal Ascent into one
authored interior and therefore one Dark light domain. Exactly eight surfaces are
registered as entrances: the first roofed four-wide foot threshold and the four-wide
summit threshold. These threshold surfaces remain floors of the unified interior. The
eight-wide boundary apron before the foot threshold is exterior, and Crystal Ascent's
lower aperture becomes an internal connection: crossing it does not change domain or
split the review cutaway.

The level-6 tunnel records every roofed floor, roof voxel, and cutaway owner after it
is carved once across the merged Macro volume. All tunnel route floors except the
eight exterior-apron cells carry the unified interior id. The carve preserves six
clear levels above each of its four lanes and at least three solid roof levels, and
the exact declared seam pairs are the only ordinary openings across crossed biome
boundaries. The surface biome above the tunnel remains intact and retains its own
biome identity; there is no tunnel biome. Changing camera mode does not change any of
these facts. Map view keeps the complete mountain roof opaque, while the explicit full
review cutaway may expose the continuous foot-to-summit route.

The delivery state for this contract is recorded in
[status.md](../planning/status.md); the exact generation stages are specified in
[world-generation-v3.md](world-generation-v3.md).
