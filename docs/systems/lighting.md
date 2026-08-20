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

The Crystal Mountain candidate makes its roofed tunnel and Crystal Ascent one Dark
domain. The eight open foot-apron floors remain exterior; the first roofed four-wide
foot threshold and the four-wide summit threshold are entrance floors inside the
unified domain. The lower Crystal aperture is an internal opening. Exterior daylight
therefore stops at the domain boundary and does not illuminate the route through
either threshold under the authored-domain contract.

The tunnel planner samples its representative centerline every four steps, then fits
one nonblocking small crystal into a deterministic adjacent alcove for each sample.
The shipped 63-step route resolves sixteen fixtures, keeping the established
4,500-lumen, 4.5-range physical pools visually continuous along the passage without
changing authoritative illumination. Every fixture publishes one Bright radius-4 and one Dim radius-18 gameplay
source under Macro's world-owned namespace; only the Bright member owns its visual
object and presentation-only, non-shadow-casting point light. Crystal Ascent retains
its eighteen landing pairs and cathedral-heart Bright-8/Dim-24 pair with four physical
heart lights after its interior id is unified with the tunnel. Validation uses exact
upper-dome range and requires every canonical tunnel, stair, and landing surface to
resolve to at least Dim, while exact Bright pools remain local to their fixtures.
Optional recesses may remain Dark.

The renderer's illumination overlay is diagnostic presentation over
`ResolvedIllumination`; it never changes a source, domain, observation, fog, or
picking. Delivery status and capture requirements live in
[status.md](../planning/status.md).
