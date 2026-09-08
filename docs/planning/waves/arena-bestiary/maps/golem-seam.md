# Golem prism-union integration seam

Status: **DISPATCHED — shared foundation `ddb060d` passes focused checks; body and attack integration is pending.**

The original-group comparison checkpoint is complete and the coordinator has
dispatched this integration against the guarded shared foundation. Golem is
followed by Ember Wisp and then Worm. Root owns the shared foundation, integration,
and commits; gameplay owns shape and ability authority. See
[approved scope](../plan.md), [gameplay order](../orders/L2-gameplay.md), and the
separate queued [Worm world seam](burrow-seam.md).

## Minimal shared body projection

Add the following gameplay-owned public value and actor projection. These are
queued API names, not claims about the current implementation:

```rust
#[derive(Debug, Clone, Copy)]
pub struct BodyHexPrism {
    pub offset: Vec3,
    pub height: f32,
}

impl Actor {
    pub fn body_hex_prisms(&self) -> impl Iterator<Item = BodyHexPrism>;
}
```

Each prism is one native pointy hex with fixed circumradius 1. Its offset is
relative to the actor's feet, in world orientation; height is measured upward in
world units. Return an owned fixed seven-element array through
`array.into_iter().take(count)`: seven prisms for Golem, zero for existing species,
and later one for Wisp. Do not allocate a `Vec` or materialize a hex-neighborhood
collection at runtime. Keep the offset order stable: center followed by the six
horizontal neighbor centers, with every vertical offset zero.

For Golem all seven prism heights are 2.0: five levels on the accepted 0.4-unit
maps. Neighbor center offsets are `(sqrt(3), 0)`, `(sqrt(3)/2, 1.5)`,
`(-sqrt(3)/2, 1.5)`, `(-sqrt(3), 0)`, `(-sqrt(3)/2, -1.5)`, and
`(sqrt(3)/2, -1.5)` in X/Z, plus the center. Derive presentation and physical size
from this immutable shape. Its AABB dimensions are `(3*sqrt(3), 2, 5)` and its
horizontal bounding radius is `sqrt(7)`; these are broad bounds, not collision
volumes.

Physical Golem yaw remains zero. Face aim and attack direction are independent;
turning the face must not rotate the seven-hex footprint. Existing Human/Shadow
capsules and the Dragon's oriented box retain their current paths. Dispatch by
explicit species matches: an empty prism iterator does not authorize an unknown
future species to use a capsule fallback.

`ForecastBody` already retains species, dimensions, observed feet, and yaw. Its
existing copy is sufficient to reconstruct the immutable Golem shape. No mutable
world shape table, new per-voxel world fact, or presentation-owned collider is
required. Presentation consumes the same public prism projection.

## Reuse the collision kernel

For each Golem prism, use the existing `CollisionWorld::clear` and `sweep` kernel
at `feet + offset`, with height 2.0 and horizontal expansion equal to the native
hex apothem `sqrt(3)/2`. For two fixed, aligned native hexes, this produces the
exact summed extent along the kernel's three horizontal face normals. Aggregate
all-clear for occupancy and the earliest contact for a sweep. Preserve the
existing tangent and skin conventions.

The resulting union sweep lets `shapes::slide` and `shapes::ground` retain their
existing whole-body response. Grounded Golem movement uses the current
shape-aware slide, one-level step, gravity, and impulse sequence with Golem speed
and no jump or flight. Keep Dragon yaw/flight and the accepted M01 controller
separate; any extraction of shared grounded code must preserve their arithmetic
and behavior.

Never pass the Golem AABB half-width as a capsule radius. That would be about
2.598 for a body only 2.0 high, reversing the existing capsule clamp interval and
also filling the union's exterior recesses with invented collision.

## Complete query integration

The shape phase must finish every relevant query before Golem becomes selectable:

| Current path | Golem branch |
| --- | --- |
| `shapes::clear`, `sweep`, `slide`, `ground` | Seven fixed prisms; earliest actual contact and whole-body response. |
| `shapes::distance` and closest-body contact | Minimum exact point-to-prism distance, including native hex faces and vertical caps. |
| `shapes::voxel_overlap` | Any actual prism overlap, retaining tangent-free neighboring cells. |
| `shapes::exposed_cone_contact` | Test convex prism components separately and retain per-contact obstruction checks. |
| `spells::aim_from_camera` | Ray against the actual union, not the capsule fallback. |
| `spells::advance_shot` | Sweep relative to the translating union; choose the earliest prism contact and physical normal. |
| Forecast and owner clearance | Reuse the same species-aware distance and sweep branches from copied observations. |
| `encounters::body_overlap` / `separate_many` | Actual component overlap; separate the whole body through shape-aware sliding. |
| Dry checks and `steering::contained` | Cover the complete body using published liquids and bounds; preserve existing species behavior. |

The union is nonconvex. A convex alternating-projection helper must operate on
one prism at a time, not on its AABB or on the entire union as if it were convex.
Likewise, body separation needs actual component pairs, with a maximum of 49
Golem/Golem prism pairs, using native hex normals and the other body's appropriate
box or capsule contact axes. An AABB is suitable for early rejection only. Keep
queries bounded and check their cost in the combined native performance gate.

Aim, collision, direct attacks, explosions, shield placement, and forecast tests
must all observe the same physical body. Existing barrier query masks remain
unchanged: barriers stop direct attacks but pass bodies, sight, and cameras.

## Support and deployment

Preserve the current runtime support meaning: the earliest downward collision
under any part of a clear body can support it. Reset-time deployment is stricter:
`deployment_pose` lowers the complete body slightly and requires every overlapping
supporting voxel to belong to the region at the exact selected level. These are
distinct existing rules; do not introduce a new full-footprint support requirement
on every movement tick.

The Golem `voxel_overlap` branch must not inherit the capsule branch's additional
`SKIN * 4` horizontal radius padding. Exact horizontal tangency to an adjacent
voxel is not overlap. Otherwise a centered seven-hex Golem would incorrectly
demand an extra support ring. The existing radius-four candidate scan in
`deployment_pose` is sufficient for this fixed footprint.

No world deployment publication change is expected. Existing typed world fixtures
establish seven same-level supporting surfaces per side, open sky, and no liquid,
static object, or protected support in those regions:

| Map | Preferred axial centers | Feet height before skin | Center separation |
| --- | --- | --- | --- |
| Duel | `(-6, 0)` and `(6, 0)` | 3.2 | About 20.785 |
| Fort | `(-4, 2)` and `(-2, -2)` | 6.4 | 6.0 |

One centered fixed Golem fits each region geometrically. Fort leaves a one-unit
gap between their five-unit Z extents. Actual Golem admission still needs the new
shape tests; existing world fixtures alone do not prove the unimplemented body
queries. Additional mixed or multiple-Golem rosters may fail atomically when
those finite surfaces cannot support the complete group. Seven Regions retains
its existing absence of spectator deployment.

For player play, keep the authored human start. Validate the Golem's complete
shape against the current enemy approach; if Fort's ordinary hostile start cannot
fit it, explicitly select a published courtyard preferred surface on the Golem
path. Do not alter the world recipe or use an unrestricted rooftop search as an
admission workaround. The Fort gate has four clear levels, 1.6 units, so a
two-unit-high Golem cannot pass it. Its courtyard spawn is useful combat space;
gate rejection is physical behavior, not a reason to enlarge the gate or teleport.

## Focused acceptance before ability calibration

1. The public projection yields exactly seven native prisms for Golem and zero for
   existing species, with stable offsets, five-level height, and no allocation.
2. Turning aim leaves physical yaw, footprint, and collision unchanged. Rendered
   face aim remains independent from the fixed body.
3. Terrain clear/sweep/ground queries respect every outer prism, exterior recess,
   floor, ceiling, one-level step, and unsupported edge.
4. Camera rays, moving projectile sweeps, owner clearance, explosions, and exposed
   cone contacts use the real union, including side prisms and concave boundaries.
5. Golem/capsule, Golem/Dragon, and Golem/Golem separation cannot overlap an ignored
   outer prism or push a body through terrain. Allied projectile policy stays intact.
6. One Golem per side admits on actual Duel and Fort publications with exactly
   seven support cells; blocked support, lost footing, or oversized mixed rosters
   fail atomically. Player Fort/Duel spawns are physically valid, and the low gate
   blocks Golem passage without trapping the human route.
7. Existing Human/Shadow goldens, Dragon geometry, world reset, dirty-column refresh,
   and missed-revision fallback remain valid.

Slam/laser timing, source identity, damage budgets, warning presentation, and
matchup calibration follow [the approved plan](../plan.md). The body and attack acceptance matrix remains pending; the shared foundation
alone does not establish completed Golem evidence.
