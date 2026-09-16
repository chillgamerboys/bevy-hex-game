# Natural wind field: Wave Lab candidate

The Wave Lab opens with **Field** wind. F4 retains Calm, Steady, Strong, Gusts,
Turning and the old synthetic Shelter comparison. Ocean now uses Field wind too.
The fixture remains seven radius-12 biomes.

Field starts from 9 u/s toward east. Its prevailing heading changes slowly within
25 degrees, with smooth local veering up to another 20 degrees. Related gusts
vary speed by up to 45%, with slower spatial bands adding up to 12%. Two moving
curl scales produce broad bends and smaller eddies. The smaller eddies grow and
subside on a smooth 29-second envelope; combined swirl is capped at 85% of the
altitude-adjusted mean speed. Exposed mean speed grows smoothly to 1.8 times at
24 units above sea level. Low-altitude gust periods are 8.3 and 11 seconds;
high-altitude periods are 3.7 and 5.3 seconds. Total speed is capped at 25 u/s.
These are game tuning choices, not a fluid simulation.

The immutable lab sampler owns a compact snapshot of published solid spans,
including objects and shield-wall terrain edits. Three upwind probe paths extend
48 units. Interpolated occupancy, soft vertical edges and distance decay produce
shelter approaching 20% of exposed strength. Separate occupied intervals preserve
openings. Terrain revision changes republish the sampler; removed walls therefore
stop sheltering. Shelter follows the unsheltered local flow direction, including local veering
and curls. Gust coordinates stay in the fixed prevailing frame to prevent
artificially fast changes far from the world origin.

Boat, glider and displays all query `OceanEnvironmentView::wind_at` using the
unwrapped completed simulation clock. The boat's propulsion and the glider's 65%
multiplier are unchanged. There is no vertical wind or walking force. Pause holds
the clock; F6/F7 only freeze/reset wave phase. Wind does not drive waves yet.

**V** toggles the local instrument and a depth-tested arrow grid. Three layers sit
at sea level +2, +14 and +30, each a 7×7 grid at 8-unit spacing. Occupied positions
are omitted. Arrows show environmental wind before the glider multiplier: cyan to
yellow and short to long correspond to 0–25 u/s. Samples refresh at 10 Hz and
vectors interpolate between refreshes. Hiding the display clears samples and
performs no visualization wind queries. Terrain edits refresh immediately.

Capture example (use a fresh committed candidate and output directory):

```sh
python3 tools/water_lab_review.py --wind field --show-wind \
  --view overview --view shore --view glider-third --settle-frames 180 \
  --target-dir /absolute/shared/cargo-target --output /absolute/fresh/output
```

Omit `--show-wind` for the timing control. Receipts include sampled positions,
velocities, terrain revision, simulation time and generation. App-frame timing
summaries exclude the loading frames before render readiness; they are not GPU
benchmarks or native FPS guarantees.

Native acceptance route: enable V, sail from exposed water into the hill's lee;
place a shield wall upwind, compare behind/above it, then destroy it. Use F11 to
start gliding from the hill, compare the three layers, and check controllability.
F4 Steady provides the previous constant-wind reference. Pause/resume and F6/F7
should confirm the clocks remain independent. The user accepted the lab and Ocean
wind iterations through gameplay head `83b409782ce3dd130231dad5f84eeae64a0fd45f`.
This route remains available for regressions; full combined runtime sign-off stays
with the parent delivery stack.

## Ocean adoption

Ocean now publishes the same natural field, with its own sea level as the altitude
origin. Its immutable cover snapshot refreshes on terrain publications and movement
between 16-unit cells; only spans within 112 units are retained. Bathymetry is shared
across those refreshes. Waves, boat propulsion, and the 65% glider multiplier retain
the accepted tuning. Other maps keep their previous behavior.

V also shows the field in Ocean. Near sea level the layers use sea +2/+14/+30;
farther up they follow camera altitude so the summit field remains visible.

`python3 tools/arena.py launch --map northern-archipelago --start summit-glider`
requests the summit neighborhood, refines the overview peak using admitted solid
columns, and stages a body-clear glide five units above it at 12 u/s toward the
shore. The ready screen keeps the player paused until Play/Enter. Progress is
preserved; failed admission cannot move the player into unloaded terrain.
