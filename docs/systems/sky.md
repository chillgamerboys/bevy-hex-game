# The sky

The sky is neither a cubemap nor Bevy's `Atmosphere`. It is a custom `Material`
(`hex_world::sky_material::SkyMaterial`) whose fragment shader
(`assets/shaders/sky.wgsl`) computes a colour per pixel from the view direction: a
vertical horizon→zenith gradient, optional celestial bodies and halos, and static
hexagonal clouds.

It renders on the inside of a large inverted sphere — the *sky dome* — spawned at
`Startup` beside the camera. `SkyMaterial::specialize` sets `cull_mode = None` so
the dome draws from within, and `follow_camera` pins the dome's translation to the
camera every frame. Because the camera stays permanently at the dome's centre, the
sky depends only on view *orientation*: clouds stay fixed on the celestial dome
while panning and re-orient only while orbiting. The dome radius (500) is inside the
camera's far plane and well outside the terrain and max zoom, and it is a
`NotShadowCaster` — a 500-unit sphere would otherwise shadow the whole map.

Choices worth knowing:

- **Custom shader over `Atmosphere`.** Bevy 0.19's first-party atmospheric
  scattering draws a physically-accurate clear sky but cannot draw clouds, and it
  forces `hdr` + tonemapping on the camera, which would recolour the *entire* scene.
  A dome shader keeps the change contained to the sky.
- **Azimuthal-equidistant cloud projection.** Cloud cells are placed by the angle
  *away from the zenith*, so a hex keeps the same angular size straight up as it does
  near the horizon. The obvious `dir.xz / dir.y` (gnomonic) projection stretches
  cells toward infinity near the horizon — it renders, and looks wrong, with no
  error in the log.
- **The lower hemisphere is mirrored onto the upper one** (`acos(abs(dir.y))`). The
  projection has a second singularity at straight *down*, where cells smear into long
  radial streaks. That sounds ignorable and is not: the gameplay camera looks down at
  the map, so most of the sky on screen is *below* the horizon — the broken region is
  the one you actually see. Sky-only screenshots aimed up or level never show it, which
  is exactly how it shipped unnoticed the first time.
- **The cloud field is a density, not a per-cell mask.** Each pixel sums a soft bump
  from its hex cell *and its six neighbours*, then thresholds; that is what lets
  adjacent clouds merge with no seam (a single-cell mask left a visible gap because
  the fill stopped short of the shared edge). `cloud_roundness` blends the cell shape
  hexagon→disc, and an fbm built on the shader's one `hash21` breaks the edges up.
- **Anti-aliasing is analytic.** The cloud edge is a `smoothstep` whose width comes
  from `fwidth()` of the density, so it stays ~1px crisp at any zoom or view angle.
  This matters because MSAA (Bevy's default 4x) only smooths *geometry* edges, not a
  colour discontinuity computed inside the fragment shader, and there is no
  post-process AA in the project — a fixed-width edge shimmered and read as
  low-resolution.
- **The sun and moon are shader discs, not scene meshes.** Their authored angular
  sizes stay constant at every camera radius. The renderer derives the light ray and
  body direction from one resolved vector, keeps the moon opposite the sun, clips
  discs at the true horizon, and uses the same analytic derivative approach on their
  edges. Clouds are evaluated last so they can cover a body naturally.
- **Sunset glow is local in azimuth.** A mirrored patch on the lower dome carries the
  low sun's colour into the downward map view. It is intentionally restrained and
  directional: tinting the whole lower dome produced a flat terracotta surround.

`LightingSettings` resolves either its legacy static values or a cycle keyframe pair
into `ResolvedLighting`. `TimeOfDay` is a reflected gameplay-session resource; changing
it does not advance a simulation clock, it simply resolves another deterministic
frame. `apply_sky_material` pushes that same frame into the dome while the lighting
system applies it to the single shadow-casting key light, exposure, ambient fill,
environment fill, and fog. See [development/config.md](../development/config.md) for
the authored controls.

The default-off development UI may adjust an existing cyclic `TimeOfDay` in half-hour
steps or select midnight, dawn, noon, and dusk. Its adapter runs before lighting
resolution, so physical lighting and the renderer-independent exterior illumination
projection change in the same frame. It never inserts a missing clock, changes a
static profile, persists a preference, or ships in the default release build.
