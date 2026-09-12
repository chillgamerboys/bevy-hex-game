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
the existing radius-12 and 8,192-voxel limits.

The north–south river reaches both map edges. Its water surface is level 34 with
12 occupied liquid levels. One 49-column-long, seven-column-wide timber bridge
crosses at level 48; water and air remain below its two-level deck. The eastern
massif has three limestone combat shelves at levels 80, 130 and 190, joined by
graded trails. Forest clearings and routes remain free of tree occupancy.

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
`compiled/content-verification.json`.

`check` independently checks emitted voxel connectivity, exact stock provenance,
deterministic generated files and pairwise root spacing. Stock exports reference
blueprint commit `18493cea8201d80c1d85f5aaf82ff85b0ea6c0ad`. If tree geometry is edited,
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
the base recipe uses basalt/limestone. Tree trunk/branch styles map to `timber`;
all three foliage styles map to solid `foliage`. Rendering uses the original
style colors and exact geometry; gameplay must not silently turn canopy into air.

Geometry validation is independent of static presentation, native motion and
performance review. A successful compiler or query receipt grants no aesthetic
or frame-rate approval.
