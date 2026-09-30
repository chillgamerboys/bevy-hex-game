# Forest–Massif Battle world

This is an ordinary runtime-loaded V4 `WorldSpec`, with one radius-187 region
(`forest`), 105,469 terrain columns and 444 storage chunks. World identity is
`forest-massif-battle`, seed `20260911`. The source is `world.ron`; generated
immutable packages belong under ignored `compiled/`, never in the source recipe.

The west has broadleaf outskirts, a pine interior and connected ancient mosswood
groves. Trees increase from 6.3–9.8 to 14–21.7 to 28–39.9 world units, with minimum
root separations of 10, 18 and 30 units. The central Heart Tree is 60.2 units tall
(172 exact .35-unit voxel levels) and reserves 48 units from other roots. The ten
catalog blueprints contain actual occupied voxels, not render-only scaling. They
require the shared blueprint height limit of 192 levels while remaining within
the shared radius-32 and 65,536-voxel limits. Crown radii are 4, 7, 13 and
24 hex columns; all 275 original trunk positions remain fixed. Overlapping crown
footprints use disjoint height bands, never overlapping solid voxels.

The north–south river reaches both map edges. Its water surface is level 34 with
12 occupied liquid levels. One 49-column-long, seven-column-wide stone bridge
crosses at level 48; water and air remain below its two-level deck. The eastern
massif has three limestone combat shelves at levels 80, 130 and 190, joined by
graded trails. Its main body and crown relief are 20% taller, with offset ridge
centers, two carved valleys and four companion peaks to the north, east,
southeast and south. Shelf heights and every original route grade are unchanged.
Forest clearings and routes remain free of ground tree occupancy; the explicit
`overhead_clearance: Some(8)` authoring policy allows crowns at least eight clear
levels (2.8 world units) overhead while refusing actual object voxel overlaps.

The reproducible forest planting/coverage domain is q in [-171,19], r in
[-122,162], axial hex distance at most 171, and q+r/2 < -28: 30,291 land columns,
including all camps, trails and clearings within it. It covers the three planted
forest biomes; it is not the whole western grass border. `authoring.json` reports
actual union coverage from exact foliage voxels: 20,957 / 30,291 = 69.1856%,
with no overlap-area double counting. It also records the secondary coverage of
all western land at q+r/2 < -28 within radius187, including unplanted border:
21,856 / 42,141 = 51.8640%.
`trunk-layout.json` preserves the accepted horizontal root layout. The generator
independently rejects all exact voxel overlaps and requires primary coverage >=50%.

## Build and verify

From the repository root:

```sh
python3 tools/world.py build
python3 tools/forest_world.py generate
python3 tools/forest_world.py check
python3 tools/forest_world.py compile
python3 tools/forest_world.py verify-package
```

Only the first command invokes Cargo. Pass `--target-dir PATH` to both tools to
reuse an explicitly managed compiler target. `compile` and `verify-package`
accept `--output PATH` to use a different stable package workspace. The compiler
validates final stacked geometry and protected constraints before publishing.
`verify-package` uses the verified compiler's real runtime query command for all
375 river centerline rows, every named supported anchor and the complete Heart
Tree occupancy. It requires a single seven-row bridge crossing and writes
`compiled/content-verification.json`. It also queries every tree root and unions
the actual compiled foliage columns, verifies both coverage domains and rejects
any occupied voxel overlap; the receipt is `compiled/canopy-verification.json`.

`check` independently checks emitted voxel connectivity, exact stock provenance,
deterministic generated files and pairwise root spacing. Stock exports reference
blueprint commit `9233a82e9a80aa32c4d2856dde6c021b89006245`. If tree geometry is edited,
commit the blueprints first, update `BLUEPRINT_SOURCE_REV` in the authoring helper,
and regenerate the source so provenance remains truthful. Ordinary map root,
biome, route, landmark or material changes do not need a compiler rebuild.

## Runtime contract

The package workspace is `assets/config/v4/forest-massif/compiled`. The runtime
must read its `current.ron` and immutable package products through the V4 source
adapter. `authoring.json` records the exact source anchors, tree sizes and roots.
Authored anchors are named `forest/anchor/<name>`; gameplay may expose the final
name after stripping that prefix.

| Anchor | q | r | support level |
| --- | ---: | ---: | ---: |
| party_start | -94 | 125 | 40 |
| hostile_start / forest_outer_a | -109 | 100 | 40 |
| forest_outer_b | -72 | 84 | 40 |
| forest_middle | -92 | 18 | 40 |
| forest_deep_a | -130 | 55 | 40 |
| forest_deep_b | -110 | -25 | 40 |
| ancient_tree | -132 | 4 | 40 |
| bridge_west | -24 | 0 | 48 |
| bridge_east | 24 | 0 | 48 |
| dragon_lower | 68 | 25 | 80 |
| dragon_middle | 118 | -25 | 130 |
| dragon_upper | 113 | -77 | 190 |

`ancient_tree` is a scenic ground marker east of the tree root at `(-143,5,40)`;
it does not spawn a boss. The other named anchors publish supported gameplay
placements. Enemy counts, parties, health, AI and progression remain gameplay
authority and are not duplicated in the map source.

All materials except `water` are solid; `bedrock` and `water` are not diggable.
The terrain registry includes `stone` for gameplay-created shields even though
the base terrain uses basalt/limestone and the bridge uses stone. Tree trunk/branch styles map to `timber`;
all three foliage styles map to solid `foliage`. Rendering uses the original
style colors and exact geometry; gameplay must not silently turn canopy into air.

Geometry validation is independent of static presentation, native motion and
performance review. A successful compiler or query receipt grants no aesthetic
or frame-rate approval.
