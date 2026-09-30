# Ocean clock and wind response

`OceanSurfaceProfile::regular_voxels` remains the accepted, fixed sea state.
The host can opt a world into wind response with
`regular_voxels(sea_level, voxel_height).with_wind_response(wind_profile)`.
Use the same `OceanWindProfile` as the world's `OceanEnvironmentView`.
Changing heading rotates the fixed wave train once when publishing the profile;
time-varying veering changes component energy rather than rotating spatial phases.

For an opted-in world:

- Publish `OceanFrame.simulation_time = Some(time)` from the completed gameplay
  tick. A frozen capture must publish its frozen time here too.
- Use `sample_surface_at_time` / `sample_local_surface_at_time` for camera queries.
- `OceanEnvironmentView::sample` passes that same unwrapped clock through
  `OceanEnvironmentSampler::surface_at_time` for gameplay contact.
- Keep tick-derived time in the gameplay checkpoint; wind response has no
  independent mutable accumulator to save or reset.

The original phase-only functions remain available. `OceanFrame.time()` falls
back to its original `phase_seconds` when `simulation_time` is absent. Wind
periods do not divide 900 seconds, so a wind-enabled host must supply the full
clock. Shader uniforms contain bounded phases reduced from that clock in f64.

The 12-second analytical response follows the natural field's broad heading and
speed modes, with a spatial 41-second wind band. It models sea-state energy,
not every instantaneous air eddy. Waves sample only immutable global bathymetry
and shoreline shelter. Actor-local wind cover snapshots and the moving render
mesh never influence surface contact.

The accepted shallow-water steepening remains, followed by dissipation in the
last 0.8 world units of depth and reduced energy in sheltered shores. The exact
local wet mask still controls coverage: this is refinement of existing wet
shoreline water, not run-up onto originally dry land or fluid-volume simulation.
No controller, boat influence multiplier, or authoritative water interval changes.
Admitted inland water whose mean differs from ocean sea level uses its exact flat
local mean, matching the ordinary liquid mesh; ocean waves are not projected onto
elevated lakes or rivers. Column identity, bounds and finite-grid checks still apply.

## Acceptance handoff

Logic tests cover shared full time, deterministic pause/resume, continuity of
wind energy across the carrier wrap and long sessions, camera/contact agreement,
wind strength, shoreline attenuation, and packed CPU/GPU equation agreement.
These do not establish rendered appearance or boat feel.

Before enabling a shipped profile, capture a clean composed commit through the
windowless review harness, using the same seed, clock and camera for baseline and
candidate. Include a whole-world overview, a wet bay close view from two azimuths,
boat-level and underwater views, and a frame sequence for shore approach/retreat
and camera recentering. Inspect boundary coverage, opaque steps, sheltered versus
open-water motion, shader errors and frame-time cost. Record the wind profile and
unwrapped clock alongside the existing bounded phase in capture metadata. Native
boat control feel remains a human-motion check; still images cannot clear it.
