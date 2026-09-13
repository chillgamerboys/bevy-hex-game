# Northern Archipelago

Northern Archipelago is a separate V4 exploration map. Forest–Massif remains available in Battle Mode. The new map has three distant island clusters and eleven islands, a dormant crater, snowy ridges, a sheltered timber settlement and a preserved submerged mountain range. There are no encounters, objectives or victory condition.

## Start and controls

Select Northern Archipelago on the Battle Mode start screen. The first selection prepares its immutable package asynchronously if no package was supplied. An explicit package can be selected with:

```sh
python3 tools/arena.py launch --map northern-archipelago --northern-world /absolute/package/directory --target-dir /absolute/shared/cargo-target
```

The helper invokes Cargo and preserves its asset root. Play begins on dry ground above the crater bay. Normal expedition movement, Fireball, Shield, High Jump and G gliding remain available. F opens/folds exploration flight; mouse and WASD steer, Space/Ctrl rise/descend, and Shift raises flight speed from 80 to 160 units/s. Flight is collision-aware and holds the last safe position while required terrain is loading. Exiting caps retained momentum at the glider limit. M toggles the cached north-up overview; the paused Map page supports a personal destination pin.

The latest movement tuning raises expedition starting walking speed to 5.90625 units/s and normal jump rise to 1.38 units (three-voxel ledges, below four). Restart restores the starting position, original terrain and exploration state. Pause preserves them.

## Water and residency

Ocean swells affect rendering and camera tint only. Three absolute-coordinate waves have a combined amplitude bound of two units and fade into shallow shoreline water. Rivers and fountains retain zero displacement. No currents, buoyancy, gravity-driven liquid motion, draining or refilling have been added. Water cannot be carved.

The world retains at most 512 fine source chunks, two source workers and two admitted products per pump. Body and predicted travel dependencies take priority over optional detail. At most 256 detailed terrain chunks are presented; coarse terrain silhouettes and the decorative ocean horizon remain visible at distance. Neither proxy geometry nor unloaded chunks grant collision clearance. Sparse carved cells survive retirement and reload, but not Restart.

## Developer review

Compile the source with `python3 tools/northern_package.py --output /new/package/directory --target-dir /absolute/pure/target`. The companion `northern-overview.ron` is validated against its manifest before it supplies map facts.

Use `python3 tools/northern_review.py --package /absolute/package/directory --target-dir /absolute/shared/cargo-target --label checkpoint-01` for six windowless composition captures. The helper requires a clean committed candidate by default, records source/package identities and refuses to overwrite an earlier pack. `--dirty-diagnostic` permits explicitly unapprovable scratch evidence.

Static captures establish visible geography, geometry, water boundaries and composition. Native motion and feel remain separate user checks: watch the bay swells, fly between clusters with a fast reversal, then walk through the settlement and enter/leave the water. See the [wave manifest](../planning/waves/northern-archipelago/manifest.md) for validation status and the remaining acceptance gates.
