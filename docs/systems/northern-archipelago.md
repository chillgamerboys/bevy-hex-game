# Northern Archipelago

Northern Archipelago is a separate V4 exploration map. Forest–Massif remains available in Battle Mode. The new map has three distant island clusters and eleven islands, a dormant crater, snowy ridges, a sheltered timber settlement and a preserved submerged mountain range. There are no encounters, objectives or victory condition.

## Start and controls

Select Northern Archipelago on the Battle Mode start screen. The first selection prepares its immutable package asynchronously if no package was supplied. An explicit package can be selected with:

```sh
python3 tools/arena.py launch --map northern-archipelago --northern-world /absolute/package/directory --target-dir /absolute/shared/cargo-target
```

The helper invokes Cargo and preserves its asset root. Play begins on dry ground above the crater bay. Normal expedition movement, Fireball, Shield, High Jump and G gliding remain available. F opens/folds exploration flight; mouse and WASD steer, Space/Ctrl rise/descend, and Shift raises flight speed from 80 to 160 units/s. Flight is collision-aware and holds the last safe position while required terrain is loading. Exiting caps retained momentum at the glider limit. M toggles the cached north-up overview; the paused Map page supports a personal destination pin.

The latest movement tuning raises expedition starting walking speed to 5.90625 units/s and normal jump rise to 1.38 units (three-voxel ledges, below four). **Shift+R** or Restart in Esc restores the starting position, original terrain and exploration state. Plain R has no restart action. Pause preserves the run.

## Sailing, swimming and wind

**B** deploys or folds a portable sailboat beside clear water at least 0.7 units
deep. W steers toward the view and provides slow paddling; A/D turn, S brakes.
The sail adds propulsion along the wind and loses momentum across or against it.
Speed is capped at 24 units/s. Opening equipment supplies no momentum; the hull
and physical player sweep against solids and wait at unloaded boundaries. Spells
remain available aboard. F and High Jump fold the boat.

Swimming uses WASD, Space to rise and Ctrl to dive. The physical eye has a
**90-second oxygen reserve**; breathing replenishes it over six seconds without
healing. Empty oxygen costs 10 HP/s. The compact air indicator appears while
submerged or recovering. The third-person camera never determines breathing.

The prevailing wind is approximately 10 units/s with slow gusts. Glider lift,
stall and airspeed use velocity relative to that wind; collision and streaming
use actual ground velocity. G can open the glider on land without adding lift or
changing walking/jumping. Landing leaves the canopy open; water, blocking walls
and casting fold it.

**V** independently toggles a north-up wind instrument showing the direction the
wind blows toward and its current speed in units/s. It sits beside the minimap
when M is enabled, or in the upper-right corner otherwise. It uses the same wind
and simulation clock as sailing and gliding, and is available on land too.

## Water and residency

Three absolute-coordinate swells use amplitudes 2.1, 1.05 and 0.35 units, within a
3.5-unit displacement envelope. Their periods remain 18, 25 and 12 seconds.
Weak reflected waves near shores add interference; foam follows actual crests.
Cached bed, shelter and shore-anchor samples supply matching CPU/GPU height,
normal and vertical velocity. The boat, physical-eye breathing and rendering
share one pause/reset-aware simulation clock. Rivers and fountains retain zero
displacement. No currents, water-volume solver, gravity-driven liquid motion,
draining or refilling have been added. Water cannot be carved.

The world retains at most 512 fine source chunks, two source workers and two admitted products per pump. Body and predicted travel dependencies take priority over optional detail. At most 256 detailed terrain chunks are presented; coarse terrain silhouettes and the decorative ocean horizon remain visible at distance. Neither proxy geometry nor unloaded chunks grant collision clearance. Sparse carved cells survive retirement and reload, but not Restart.

## Developer review

Compile the source with `python3 tools/northern_package.py --output /new/package/directory --target-dir /absolute/pure/target`. The companion `northern-overview.ron` is validated against its manifest before it supplies map facts.

Use `python3 tools/northern_review.py --package /absolute/package/directory --target-dir /absolute/shared/cargo-target --label checkpoint-01` for six windowless composition captures. The helper requires a clean committed candidate by default, records source/package identities and refuses to overwrite an earlier pack. `--dirty-diagnostic` permits explicitly unapprovable scratch evidence.
An additional `--view northern-boat` stages the player at admitted water and sends
B through the normal controller, then freezes a close boat/HUD frame. Its receipt
records active boat state and simulation time; it is not evidence of native input
or travel feel.
Add `--navigation` with that boat view to display the map and wind instrument;
the receipt records both visibility flags. Keyboard behavior has separate tests.

A capture-only `--view northern-bay-flat` uses zero wave amplitudes with the same bay camera, bathymetry and depth absorption. `--settle-frames 244` adds a bounded settled sample for wave-cost comparisons (allowed range 4–600); receipts record the requested count. Update wall intervals include scheduler/render-submission waits and are not GPU or native frame timing.

Static captures establish visible geography, geometry, water boundaries and composition. Native motion and feel remain separate user checks: watch the bay swells, fly between clusters with a fast reversal, then walk through the settlement and enter/leave the water. See the [wave manifest](../planning/waves/northern-archipelago/manifest.md) for validation status and the remaining acceptance gates.
