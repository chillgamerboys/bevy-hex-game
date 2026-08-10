# Gameplay lighting

Gameplay illumination is a headless world fact. `IlluminationLevel` is ordered
`Dark < Dim < Bright`; ambient light and applicable `GameplayLight` sources resolve
the maximum tier at each exact surface. Physical point lights, emissive materials,
shadows, exposure, and rendered pixels communicate the result but never establish it.
The complete sight and knowledge contract remains in
[perception.md](perception.md).

Every source and surface belongs to the exterior or to one exact authored
`LightDomain`. A local source affects only its own domain. Range is inclusive and uses
the existing upper-dome rule: horizontal hex distance and upward level distance obey
`h² + u² <= radius²`, while downward distance is ignored. Local illumination remains
obstruction-agnostic in this slice; terrain and authored object occupancy instead gate
sight after the target's illumination chooses its sight radius.

## Crystal Mountain

The Crystal Mountain wave makes its tunnel and Crystal Ascent one Dark domain. The
foot and summit thresholds are exterior, while the lower Crystal aperture is an
internal opening. Exterior daylight therefore does not illuminate the route through
either opening under the authored-domain contract.

Nonblocking small crystals occupy alcoves no more than 24 centerline steps apart.
Each fixture publishes one Bright radius-4 and one Dim radius-18 gameplay source; only
the Bright member owns its visual object and presentation-only, non-shadow-casting
point light. Crystal Ascent retains its eighteen landing pairs and cathedral-heart
Bright-8/Dim-24 pair with four physical heart lights. Validation requires every
canonical tunnel, stair, and landing surface to resolve to at least Dim, while exact
Bright pools remain local to their fixtures. Optional recesses may remain Dark.

The renderer's illumination overlay is diagnostic presentation over
`ResolvedIllumination`; it never changes a source, domain, observation, fog, or
picking. Delivery status and capture requirements live in
[status.md](../planning/status.md).
