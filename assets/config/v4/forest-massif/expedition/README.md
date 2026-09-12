# Forest–Massif expedition content

This finite V4 world contains a forest, curved river, one arched bridge, a massif
with three encounter shelves, and a walled Shadow arena. Its world identity is
`forest-massif-expedition`, radius 187, with 105,469 terrain columns in 444 chunks.
The legacy Forest source and its compiled package remain separate.

The compiled content has 807 exact voxel trees: 700 smaller forest trees, 36
landmarks, one 60.2-unit Heart, and 70 trees around the mountains and lower slopes.
The smaller crowns start 3.85–4.55 world units above their roots. Ordinary landmark
clearings leave at least 9 world units beyond their root buttresses before smaller
trees begin; ancient clearings are wider. The Heart's radius 28 clearing connects
the largest camps and the Troll. Large trunks taper through staggered contours and gently curved centers, with
flared root fins and rising boughs exposed below unequal crown lobes;
visible voxel geometry and colliding occupied intervals match exactly.

Coverage is the union of compiled foliage columns over **all 47,743 western
forest land columns**, including camps, routes and large clearings. This is
**31,992 columns, 67.0088%**, without overlapping-canopy double counting. It exceeds
the 50% floor and 60% target. The original terrain proxy is retained as `terrain.ron`;
`world.ron` adds exact tree roots, small foundation overrides and 107 prop objects.
These include 10 bridge sections, 7 arena decorations, 6 fountain rims, 48 rock
formations and 36 bright crystal formations. The bridge keeps its half width 4 travel
ribbon within a half width 5 deck, including the side departure toward the Shadow.

The 14 Goblin camps use the locked 107-Goblin distribution. World metadata exposes
19 encounter sites, 42 traversable route segments and six fountain volumes.
Gameplay owns enemy profiles, XP, rewards and healing state; `arena-sites.ron`
contains only exact world support, clearance and water facts. The player starts
at bridge center `(0,0,58)`. Dragon shelves are at levels 80, 160 and 260. The Shadow
arena's radius 12 interior has tall walls and a permanently open decorated gate.

## Reproduce and verify

`cargo battle` and `python3 tools/arena.py launch` prepare this default package
on first use. The equivalent standalone preparation command is
`python3 tools/forest_package.py ensure`. It builds missing compiler prerequisites
in `target/v4-authoring`, separate from the app target, and reuses a complete
reviewed package without running Cargo or regenerating content. Explicit
`HEX_FOREST_WORLD` / `--forest-world` selections are left to runtime admission.

From the repository root, build the pure authoring CLI once into its own target:

```sh
python3 tools/world.py --target-dir target/world-authoring build
python3 tools/forest_package.py compile --target-dir target/world-authoring
python3 tools/forest_package.py verify --target-dir target/world-authoring
```

Only the first command invokes Cargo. `compile` defaults to this directory's
ignored `compiled/` package workspace. Both commands accept `--output PATH` and
`--scratch PATH` to preserve independent candidates. They compile the bare terrain,
obtain public terrain facts from `worldc survey`, reproduce exact placements,
check current and committed art bytes, and compare the generated source with the
reviewed source. Then they survey the final package and check every expected
object interval, 4,908 ground contacts, 8,002 gameplay support/clearance positions,
and all six water volumes. `compile` emits the strict `arena-sites.ron` companion
bound to the final manifest fingerprint; `verify` checks that companion without
changing it. Neither command parses private package storage.

Expected package fingerprint is `a538263d612e891f`. `generation.json` binds exact
artwork to commit `89940ef4cb040a30506be21206c31dbdabd3f087`.
`content-verification.json`, `compile-receipt.json` and `compiler-identity.json`
record the accepted authoring candidate's logical evidence and actual compiler
identity. The production adapter still performs its own support, route, water,
art and encounter admission checks. Windowless captures and native combat review
are separate from these authoring checks.

To revise geometry, edit the pure generator modules, compile/survey `terrain.ron`,
then run `tools/forest_finish.py --survey SURVEY.json --out-dir SCRATCH --write-assets`.
Commit changed exact assets before passing that commit via `--source-revision` to
emit a new complete source. Recompile and verify into a fresh candidate directory;
only then update this directory's source and generation record. Include every new
catalog blueprint in `hex_map::arena::worlds::load_art` and keep existing assets
available for the other map modes.
