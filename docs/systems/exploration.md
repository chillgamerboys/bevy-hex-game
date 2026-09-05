# Grand V3 walk/fly exploration

Development builds include an **Explore Grand V3 Map** title-screen shortcut. It
uses the ordinary Sandbox catalog and loading pipeline with one disposable player,
then keeps `GameplayPhase::Preparing` for the complete session. Tactical occupancy,
turns, combat, spells and saved character positions do not follow this test pawn.
The environment entry point remains `HEX_FLY_MAP=<sandbox-map-id>`; both entry points
start in fly mode for compatibility. Press **F** to walk or resume flying.

| Input | Action |
|---|---|
| WASD (effective camera movement bindings) | Camera-relative movement |
| Shift + movement | Run while walking |
| Space | Jump, with 100 ms buffering and coyote time |
| F | Walk/fly toggle, including while airborne |
| Right-drag / wheel | Look / requested zoom |
| Backspace (effective ReturnTitle binding) | Return to title |

F and Space are exploration-owned edges. Movement preserves its held state with
Shift and does not issue tactical commands. A focused UI control receives normal
keyboard input while the body continues falling with neutral intent. Window blur
or pause suspends simulation without accumulating catch-up time. Held Space never
repeats a jump. The fixed controller runs at 120 Hz; a single frame contributes at
most 250 ms, so resuming after a stall cannot cause an unbounded catch-up loop.

## Grounded motion

WASD follows camera yaw even at vertical pitch poles. Walls slide, floors support,
ceilings stop upward motion, and leaving a ledge starts a fall. A bounded up/across/
down sweep climbs a one-level step only when the complete body has clearance.
Jump height, step height, wading depth and body height scale with the loaded map's
level height (Grand V3 is 0.35 world units). Tuning is validated on load/hot reload in
[exploration.ron](../../assets/config/exploration.ron).

Water and lava are liquid volumes, never platforms. One-level wading is permitted;
walking into deeper liquid is rejected. Falling into deep liquid or below the map
returns to a still-valid last grounded position, then the original spawn if needed.
The shore guard applies near the water surface: high bridge and cliff edges still
allow ordinary falling over deep water, followed by recovery on entering it.
If both supports have changed, exploration enables fly instead of repeatedly
teleporting into invalid geometry. Falls do no damage. Swimming is deferred.

Fly remains noclip and preserves its existing full-pitch movement at 25 world units
per second. Switching to walk in clear air begins a fall from that exact position;
switching inside solid geometry is rejected with a control hint. Every successful
switch clears vertical motion and jump timing, preserving position, player-authored
rotation and requested zoom.

## Collision and presentation boundary

The dev-only collision adapter consumes public `HexTile`, `TilePos`, `HexSpan` and
`SubstanceId` facts plus validated `ObjectInstance` / `RuntimeArtCatalog` geometry.
It retains all stacked runs, applies object pivots and six-way rotations exactly,
and updates only changed sources and their spatial buckets. Catalogue changes
invalidate the projection; geometry errors disable grounded simulation until fixed.
It never reads private voxel storage or writes gameplay blocker projections.

Solid substances, tree trunks/branches/canopies, crystals and structural props
collide even when presentation hides them. Air and shader clouds do not. Grass
tufts, snowy grass tufts, cave moss and cave lichen are explicitly soft dressing;
solid terrain grass remains a supporting surface. Cosmetic opacity and tactical
blocker footprints are not collision classifiers.

The swept body conservatively expands the six hex faces by its configured radius
and the vertical interval by its height. Nearby columns along each sweep provide
candidates. A separate camera probe retracts the boom while preserving authored
rotation and desired zoom; stable-clearance delay governs outward restoration.
The probe/focus stay inside the body's clear envelope, and close views use the
existing composable character-occlusion reason. Fly retains camera noclip.

Exploration disables tactical terrain shroud while its pawn moves independently
of gameplay perception, and restores the previous presentation mode on exit.
Windowless scripts temporarily run exploration at real simulation speed and restore
their previous clock speed when the session ends.
Automated image captures omit the native world inspector, whose renderer requires
a Winit surface. Ordinary development launches retain the inspector and its input
ownership checks.

## Verification and future controls

Pure controller/geometry tests and Bevy integration tests in `hex_game::fly` cover
motion, collision, gravity, input ownership, mode switches and source invalidation.
Runtime screenshots assess presentation only; controller state is the logical
oracle. Native motion/control feel requires a Grand V3 playtest of running, jumping,
ledge falls, wall sliding, tight camera passages and repeated walk/fly switching.

The [windowless exploration script](../../walks/grand_v3_exploration.ron) runs with
`HEX_WALK_SCRIPT=walks/grand_v3_exploration.ron`, a fresh `HEX_WALK_OUT` directory,
and `cargo run -p hex_game --features dev,visual-walk`. It observes mode, grounding,
displacement and re-entry through typed state, and captures both camera azimuths.
The script is static presentation evidence; its captures do not establish motion feel.
Its explicit 180-second gameplay-loading allowance accommodates unoptimized CI
world construction; it does not change native loading behavior or performance gates.

Later, compare the retained right-drag controls against captured-mouse third-person
look, including cursor release, camera comfort, movement direction and quick switching.
No mouse capture is introduced in this version.
