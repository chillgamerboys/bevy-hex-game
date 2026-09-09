# Wisp body, projectile and flight seam

Status: **Implemented — flight, layered deployment, Ember behavior and machine calibration are integrated. Native capacity was measured at `5f5afec`; a windup-clarity repair awaits refreshed static review on the current candidate.**
The main experiment checkout starts this phase from `c1a6cf4`, preserving Golem
HP320. A separate detached `hex-arena-review` checkout retains that exact Golem
candidate for native calibration and captures. Receipts identify the actual checkout
and commit; changing this main foundation does not update the frozen native run.
No Wisp flight, layered deployment, Ember constructor, attack, or brain is enabled.
All Wisp requests return a typed refusal until those authorities are integrated.

The coordinator has approved the names and initial values below. The grounded
Golem and every original species keep their current parameters and decision paths.
[The approved wave plan](../plan.md) separates behavior requirements from tuning.

## Public names and immutable shape

- Append `Species::Wisp` and `CreatureAbility::WispEmber` at index9; count becomes10.
  Preserve all old indices and `BattlePreset::ORIGINAL` exactly.
- Append recipes `Goblin`, `Wisp`, `Wisps2`, `Wisps4`, `Wisps8`, `Wisps12`.
  Slugs: `goblin`, `wisp`, `wisps-2`, `wisps-4`, `wisps-8`, `wisps-12`.
  `ALL` has11 recipes; `WISP_SWARMS` has the five Wisp recipes. A provisional
  `PLAYER` list contains the existing five recipes plus `Wisps4`; explicit recipe
  requests and observer mode retain every count. Calibration chooses the final
  one-menu swarm count; four is not a strength-equivalence claim.
- Wisp is exactly one fixed native hex prism, offsetZERO, height0.4; bounds
  `(sqrt(3),0.4,2)`, fixed physical yaw0. `body_hex_prisms()` returns one element,
  Golem still seven, original species zero. Never route this wide, short body
  through the capsule clamp. `Actor::eye()` is the glow core at `center()`.
- Reuse exact `hex_prisms` queries in shape clearance, sweep, ray, forecast,
  radial distance, cone contact, liquid tests, actor separation and containment.
  Append explicit species matches; no unknown-profile fallback.
- Public `ProjectileAppearance::{ShieldSeed,Fireball,Ember}` and readonly
  `Projectile::appearance()`, `collision_radius()`, `source_ability()` let the
  renderer and receipts consume frozen facts. No new `Spell` or human hotbar slot.
  Keep `VisualEffect.kind` as the existing radial cosmetic family; an Ember impact
  may reuse the small Fireball effect at its actual0.8 radius. A future Boulder
  appearance is appended only with Worm; do not expose a selectable absent attack.

## Config starting values

All are new EncounterTuning fields, serde-defaulted and authored in arena.ron:

| Name | Initial value | Meaning |
| --- | --- | --- |
| wisp_hp |18 | Weaker than the50HP Goblin |
| wisp_flight_speed |1.5 | All voluntary movement, no run mode |
| wisp_cruise_height |4.0 | Feet above the locally admitted dry supporting surface |
| wisp_layer_spacing |0.8 | 0.4 body plus0.4 open vertical gap |
| wisp_preferred_min |20 | Positioning preference, **not** a minimum firing range |
| wisp_preferred_max |32 | Preferred outer distance and maximum shot admission |
| wisp_ember_damage |8 | Maximum radial damage before ordinary distance falloff |
| wisp_ember_radius |0.8 | Small physical splash radius |
| wisp_ember_cooldown |2 | Cooldown of the creature ability |
| wisp_ember_windup |0.35 | Visible preparation before actual projectile release |
| wisp_ember_speed |32 | Ballistic launch speed |
| wisp_ember_gravity |2 | Gentle ballistic fall |
| wisp_ember_collision_radius |0.06 | Physical swept shot radius, same initial size as old shots |
| wisp_ember_knockback |1.5 | Proposed small impulse; coordinator may tune |
| wisp_ember_terrain_power |1 | Proposed weak Fire admission, world owns material/HP |

Validate finite positive values, HP through the existing health bound, ordered
preferred interval, shot radius <=0.25, layer spacing >0.4, terrain power1..10.
Knockback can be zero. Literal native body dimensions do not need a new mutable
shape table. No old Human/Shadow tuning value changes.

The accepted4.0 cruise height accounts for Goblin's1.3 jump and1.732 swipe reach;
3.0 would not establish the requested above-melee counter. Lower heights are
permitted when actual overhead clearance forces them, with physical vulnerability
retained. Flat-open-map counter tests must include the legal Goblin jump apex.

## Frozen projectile authority

The implementation phase uses this small frozen launch recipe:

```rust
struct CreatureProjectileSpec {
    ability: CreatureAbility,
    appearance: ProjectileAppearance,
    speed: f32,
    gravity: f32,
    collision_radius: f32,
    splash_radius: f32,
    damage: f32,
    knockback: f32,
    terrain_kind: TerrainDamageKind,
    terrain_power: u8,
}
```

This recipe is a shared implementation contract, not an unused installed type. At the first
released WispEmber pulse, copy actor ID/team and admitted aura multiplier, then
freeze appearance, source ability, speed/gravity, collision radius, splash radius,
damage, knockback, terrain kind and power into the existing Projectile. Existing
Actor.charge/hotbar state is untouched; Wisp uses the finite creature Cast phase.

Keep ordinary `projectile()` and its default values bit-equivalent. Extend private
ShotParameters by the missing frozen metadata; the existing `radius` is a splash
radius, **not** the currently hardcoded0.06 swept radius. Use the new swept radius
consistently in terrain sweep, body sweep and initial owner clearance. The existing
Shield contact-anchor helper retains its original fixed seed radius.

Add a constructor for a creature spec, and factor the existing forecast's copied
observed-body loop into a shared private routine. Both actual and forecast run
`advance_shot` with the same frozen recipe. Do not clone ArenaTuning and disguise
the Wisp as a player or Shaman merely to get gravity2/damage8.

TerrainDamageKind for an Ember is `Elemental(materials.fire)` captured at release.
Existing spell-only forecasts need no material admission; their ordinary payload
may retain its old world-Fire resolution while the creature spec supplies the
explicit frozen kind. Do not infer payload from the owner's species at impact.

Allies pass through the projectile and ignore splash; caster self-damage remains
possible after owner clearance. Impact on either team's transparent barrier uses
the ordinary direct-blocker rule. Radial splash continues through cover. Source
death cancels only an unreleased Cast; already released shots retain their payload.

The finite ability's existing first-pulse counter records WispEmber once. Its
constructor must not call `record_cast(owner,Spell::Fireball)`, nor classify Ember
hits as ordinary fireball-impact stats. Generic damage-dealt/self-damage stats and
source-team cues remain shared. New projectiles first advance on the following
tick, after the current release (existing chronology is retained).

## Finite layered deployment

Keep world-owned seven-surface regions unchanged. Ground bodies reserve first.
For each Wisp, enumerate at most14 candidates: lower layer first, preferred surface
then the existing deterministic surface order, then upper layer. Feet are
`top(surface)+SKIN+cruise_height+layer*layer_spacing`, layer0or1. This fits12 Wisps
per side without pretending that seven ground positions hold twelve bodies.

A candidate is admitted only when its exact supporting surface still exists,
the complete single-hex projection is horizontally above an approved surface,
full body volume is clear/dry/contained, vertical bounds admit it, and it does not
overlap any already reserved actor. A flying deployment does not assert grounded
support at feet: store flying=true, grounded=false and retain the chosen layer
in the brain's ordinary home/flight preference. Initializer must not overwrite
those flags with the existing unconditional ground initialization.

Player Fort recipes use the same explicit published pocket and bounded layers,
keeping the authored human start. No unrestricted rooftop search. A blocked layer,
low roof, missing support, liquid or mixed roster capacity shortfall produces an
atomic InvalidSetup. 12v12 remains inside the shared24-actor cap; custom requests
that do not fit the14 finite candidate slots fail rather than expanding the map.

## Small observation-limited flight and attack path

Use existing party activation, sight memory/cue isolation, search expiry and
Returning behavior. Spectator search may use the public opposing deployment.
Only the sensing facade sees current opposing actors; copied observations enter
flight goals, ballistic lead and forecasts. Same-team positions may be used for
physical spacing, without importing allies' hidden target truth.

The20–32 band is a movement preference. Shoot any own-visible forecast-safe target
within32, including a Goblin directly below. A hard20 minimum would create an
unattackable pursuer under a slower Wisp and contradict the requested counter.
Use the existing ballistic solver with explicit speed32/gravity2, including its
vertical trajectory support. Lead only the copied same-ID velocity for at most
the existing prediction window. Revalidate own sight, actual release origin and
forecast usefulness/self-splash safety at the .35s release. Missing sight cancels
unreleased preparation; there is no Wisp blind fire in this phase.

For movement, retain each actor's admitted layer offset above the local dry floor.
Prefer the middle of the20–32 band, adjusted by the actor's existing side from the
observed target. Test a fixed small fan [0,±0.5,±1.0] at preferred range; use a
clearance-admitted closer location when the finite arena cannot fit that distance.
While already near the outer boundary, hold or move along a safe tangent instead
of seeking an impossible far point. Avoid same-team volumes with a bounded local
repulsion/fan choice; do not reset every actor to one shared home height or point.

Reuse the existing12-tick production-body rollout and per-actor decision staggering;
new Wisp motion is fixed-yaw, full-prism flight at1.5 with separately damped impulse.
No gravity on voluntary hovering, but knockback remains real and cannot tunnel
through terrain or other actors. The existing outside-flyer inward recovery rule
must apply to the Wisp too. Windup can pause voluntary flight while impulse still
moves it; release recomputes aim from the new core. No global pathfinding or full
new navigation framework is required.

## Landing order and focused acceptance

1. Validate and commit the schema/config/read-only projectile projection, with typed Wisp
   admission refusal until actual shape, flight and shot paths are complete.
   The guarded foundation does not include those mandatory runtime hooks.
2. Validate the included one-prism dispatch against kernel fixtures and freeze creature projectile payloads.
   Preserve old Duel golden snapshots exactly; never regenerate expectations.
3. Add bounded layered admission plus normal Wisp flight, then WispEmber ability
   and sensing-only brain branch. Remove refusal only for the complete path.
4. Run focused tests: body dimensions/roof/notch/query agreement;12v12 distinct
   clear flying admission on both maps; layer-blocked atomic failure; ordinary
   flight preserves spacing; old body flags/poses unchanged; actual stationary,
   moving and directly-below hits with gravity2; lost-sight/windup/source-death;
   fixed team/aura/terrain payload after release; ally passage and genuine caster
   self-hit; physical impulse and outside-boundary reentry; legal Goblin jump+swipe
   cannot reach the open-map cruise body but Ember can damage the Goblin.
5. Then app/native evidence and1/2/4/8/12-Wisp crossover against unchanged Shadow,
   with both sides/seeds and genuine loss/timeout accounting. One-Wisp-v-one-Goblin
   is a correctness/control fixture. No Wisp equivalence is promised before data.

Wisp test, build, calibration and static-review checkpoints are recorded in
[bestiary validation](../../../../systems/arena-bestiary-validation.md).

## Historical guarded foundation boundary

The initial guarded foundation added species, counters, recipes, schema and
safe prism-query dispatch while temporarily refusing Wisp setup. That was
a deliberate integration boundary, not current behavior. Flight, layered
spawning, Ember actions, brain and application projections were integrated
and validated in the subsequent recorded checkpoints.

Ballistics should use an explicit-gravity helper behind the existing
`ballistic_aim(origin,target,tuning,speed)` wrapper. The wrapper supplies its current
projectile_gravity unchanged; Wisp supplies2.0. This avoids cloning player tuning
or introducing divergent ballistic math. The existing quadratic formula handles
a directly below target without a special horizontal-distance division.
