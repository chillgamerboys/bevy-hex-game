# Perception

The contract for deciding what a faction can see, what it remembers, and what the
renderer may disclose. The shared vocabulary and setup ordering were established
before the runtime system on purpose: map, unit, combat, and presentation work can
compile against one boundary without reaching into one another's crates.

> **Status:** authoritative illumination, obstruction-aware pooled faction sight,
> Unknown/Remembered/Observed map knowledge, and the live-map tactical shroud are
> live.
> V3 Caves publishes fixed local gameplay lights into that live pipeline.
> Casting anchors, hostile lattice disclosure, and AI observation/traversal now
> consume that authority. Authored cave crystals and restrained physical lights
> present those sources without becoming gameplay authority. Unknown-frontier
> movement, engagement, and lost-contact search remain later isolated work.

## Four facts, not one

"Visible" is too overloaded to be a useful domain word. Perception keeps four facts
separate:

1. **Illumination** is an objective property of an exact place in the world.
2. **Sight** asks whether one faction currently observes that place.
3. **Knowledge** is what that faction is allowed to know about it now.
4. **Presentation** turns that knowledge into meshes, overlays, picking, and UI.

A physically bright pixel does not make a tile observed. A remembered tile may be
drawn without being observed. Hiding a cave roof for an explicit review capture does
not illuminate the cave below it; ordinary gameplay keeps cave roofs intact. No
gameplay rule may infer knowledge from Bevy's `Visibility` or the PBR light list.

Enemy lattice knowledge is a fifth, independent information channel. Observing an
enemy establishes its position; it does not reveal its lattice or intent. Divination
owns those facts. The live combat adapter reads faction knowledge, never Bevy's
`Visibility` or the PBR light list.

## Illumination

`IlluminationLevel` is ordered from `Dark` through `Dim` to `Bright`. The effective
level at an exact `TilePos` is the greatest level supplied by its ambient domain and
any applicable `GameplayLight`.

| Source | Gameplay illumination |
|---|---|
| Static profile or sun-key cycle exterior | `Bright` |
| Moonlit exterior | `Dim` |
| Cave or tunnel interior | `Dark` |
| Local gameplay light | Its authored level inside its authored radius |

Scenario time remains fixed. A lighting-profile adapter publishes the core
`ExteriorIllumination` resource from validated authored settings: static profiles and
a cycle whose active key is the sun publish `Bright`; a moon key publishes `Dim`.
It runs in `PerceptionSystems::PublishAmbient` before exact illumination is resolved,
both on gameplay entry and when the development scrubber changes. Missing initial
publication is a setup failure, and an invalid hot reload retains the last valid
projection.

The adapter may resolve gameplay and renderer outputs alongside each other, but one
does not feed back into the other. Physical illuminance, shadows, exposure, emissive
materials, fog, and sampled pixels are presentation only. They never establish
gameplay illumination.

Every place and local light belongs to a `LightDomain`: the exterior or one exact
interior region. A light's domain is derived from its exact current position rather
than cached, so a future carried light can cross an entrance. A light affects only
positions in its current domain and inside its inclusive upper-dome radius. Let `h`
be horizontal hex distance and `u = max(target_level - source_level, 0)`; a source
reaches the target when `h² + u² <= radius²`. Vertical distance below a source is
ignored, so the volume is a downward cylinder with a grid-space spherical half-dome
above it. This prevents a cave lamp from shining into another authored domain without
requiring line-of-sight. Inside one domain, local light is deliberately
obstruction-agnostic: walls, corners, units, props, and shadows do not block its
illumination projection.

Gameplay lights are public world facts. The same lamp, crystal, or future carried
light illuminates a place for every faction; there are no faction-private light
volumes. Overlapping sources take the maximum level rather than adding brightness.

V3 Caves places fixed Bright sources with radii from four through seven. They cover
the entrance, required route, and critical chambers while preserving dark optional
branches. Authored emissive crystal meshes and restrained physical point lights
communicate the rule at those exact sites, but they never implement gameplay
illumination.

## Sight

A `SightProfile` maps each illumination level to one grid-space radius. The initial
ordinary profile is:

| Target illumination | Radius |
|---|---:|
| `Bright` | 36 |
| `Dim` | 12 |
| `Dark` | 1 |

The target's effective illumination selects the radius. Range is measured from the
center of the observer's second body voxel above its support surface—the head—to the
target surface's top-face center. The same exact upper-dome predicate as local light
applies, using fixed-sixth coordinates so the half-level target face is exact. A target
at or below the eye pays only horizontal distance; upward distance combines with
horizontal distance by the squared rule. Radius-one Dark sight is immediate awareness
in absolute darkness, not emitted light.

Range is only the cheap first gate. Every in-range target then receives one paired
seven-ray character-volume bundle; there is no separate near-field cutoff. Its center
ray runs from the observer's head center to the target top-face center. Its six
perimeter rays run from the six corners of the standing body's top face to the matching
canonical corners of the target top face. The target is observable when the center ray
is clear or at least three of the six paired perimeter rays are clear. One observer
must satisfy that whole threshold; a party cannot pool corner successes from opposite
sides of a wall.

Corners pair one-to-one rather than forming every source-to-target combination. The
aligned bundle models the hexagonal volume occupied by a standing character and
prevents a diagonal fan from inventing views around a pillar. The worst case remains
seven segment tests per observer-target pair.

Character LOS applies one observer-relative low-cover rule before tracing those
segments. For each compact material run whose exposed top is within one level above or
below the observer's support **and has material directly beneath it in that run**, only
that top voxel is omitted from sight obstruction. The rest of the run remains material,
so the rule clears ordinary grounded steps and nearby one-level ridges without making
their solid cores transparent. A disconnected one-voxel run keeps its complete volume
even inside that level band. Runs topped two or more levels away also retain their
complete volume; character-height walls and vertically remote roofs or decks therefore
remain blockers.

Only exact material runs in `TerrainOccupancy` block; liquids follow the same rule as
every other terrain material. A ray is blocked when its open segment crosses a
material voxel's open interior for nonzero length. Exact face, edge, corner, and
endpoint-only tangencies are clear. Units, trees, props, renderer meshes, shadows, and
opacity do not establish obstruction. This strict-interior policy shares the exact
rational kernel with casting while leaving casting's conservative closed-contact
`supercover` unchanged. The raw strict-interior segment query always intersects the
complete supplied runs and is source/destination symmetric. The low-cover projection
belongs only to standing-character LOS and is chosen from the observer's support
level, so observation from one surface to another is allowed to differ in the reverse
direction.

`LightDomain` remains an illumination-containment fact, not a sight boundary. Sight
may cross an exterior/interior boundary through a physically open cave mouth; a wall
or roof stops it through material occupancy instead.

Each faction pools the union of all its active characters' sight. Selection has no
effect: a six-character party knows everything any one member currently observes.
Illumination alone is insufficient; an allied observer must also be within the
applicable band. Visibility and the ability to observe are separate facts: a downed
unit may remain visible to another observer, but does not contribute sight itself.
Adding or removing `Downed` invalidates observation so this change is published in the
same frame.

Sight is computed on exact stacked surfaces. A bridge, cave floor, and ground surface
at one `HexCoord` remain different `TilePos` values and may have different domains,
illumination, and knowledge.

## Faction knowledge

`FactionMapKnowledge` owns separate authoritative `FactionKnowledge` slots for Player
and Hostile and never passes facts between them. It can build the compact
`LocalMapKnowledge` traversal projection for either faction; the player publication
is retained for the movement adapter, while AI asks explicitly for its controller's
faction. Richer terrain and unit memory remains owned by `hex_perception`.

| State | What the faction may know |
|---|---|
| `Unknown` | No terrain, surface, feature, unit, occupancy, or edit information |
| `Remembered` | The exact terrain snapshot from the last observation, but no units or later changes |
| `Observed` | Current exposed-surface snapshot, blocker state, and currently observed units |

The first observation changes Unknown to Observed. When sight leaves, the last
observed terrain snapshot becomes Remembered. Terrain edits and unit movement that
happen afterward do not update it. Re-observation replaces the snapshot with current
truth. Remembered state is therefore useful but never an oracle.

The blocker state of a static map feature may be remembered with its terrain. Units
and other transient objects disappear immediately when no longer observed. Whether
knowledge survives a saved game is deferred; the first implementation is
gameplay-session state.

### Exploring an unknown frontier

The movement adapter will use Observed and Remembered exact surfaces and the shared
traversal predicate. It may append at most one horizontally adjacent Unknown
coordinate to the end of an otherwise known route.

The shipped tactical-map presentation deliberately treats current terrain as public:
Unknown and Remembered surfaces remain visible, shaded, and pickable, including later
terrain edits. This is a presentation exception, not a knowledge promotion. Unit
identity, target legality, casting anchors, AI inputs, and other observation-only facts
continue to use faction knowledge. The pending movement adapter must reconcile its
older unknown-frontier design with this public-map rule rather than making the fog
adapter manufacture stale geometry.

The movement owner decides the action cost of a rejected exploration step when that
turn rule is implemented. Perception's contract is only that the preview and result
do not leak hidden geometry.

## Combat contact

Spatial combat integration is partially live. Hostile lattice disclosure, cast
anchors, and every AI observation and traversal input use faction knowledge.
Engagement, lost-contact search, ordinary attack anchors, and unknown-frontier
movement remain pending and must use the same authority rather than introducing a
second visibility rule.

Observation gates the existing reach trigger; it does not replace it. Combat begins
when either faction currently observes a hostile **and** that hostile pair satisfies
the existing `engage_range` reach rule. Detection need not be mutual: an unseen
hostile that observes and reaches the party still changes the game to combat tempo.
Seeing a hostile beyond `engage_range` does not start combat by itself.

Every cast target anchor now requires its exact `TilePos` to be currently Observed by
the acting faction at command validation time. Preview, target cycling, AI
enumeration, and the authoritative applier all enforce the same rule. Remembered and
Unknown positions cannot anchor a cast. The same rule is binding but not yet wired for
ordinary attacks or future abilities. Movement destinations remain governed by the
separate remembered-route and one-step-frontier rule above.

A valid area effect may extend beyond the Observed area after its anchor resolves and
still affects hidden terrain and units. Its outcomes do not promote those positions to
Observed or reveal them through presentation or logs.

Current position knowledge does not grant lattice or intent knowledge. AI is subject
to its faction's knowledge rather than reading every hostile entity in the world.

The current `disengage_margin` remains the spatial hysteresis for hostiles that are
still observed: crossing `engage_range` starts combat, while retreating beyond
`engage_range + disengage_margin` may end it. Lost contact is a separate information
transition, not a replacement for that distance rule.

When all hostile contact is lost, combat records each faction's own last observed
hostile positions. The next normal round boundary begins one complete search round;
losing contact partway through the preceding round does not consume it. During the
search round, a faction may move toward only its own contact records. Those records
do not render or make a hidden unit targetable.

Reacquiring any hostile cancels the search and restores normal contact; the
independent reach and disengage-margin rules then apply normally. If no faction
re-establishes hostile contact by the end of the complete search round, combat ends.
Losing contact again later starts a new search on the same rule.

## Presentation without state collisions

The live fog adapter consumes player faction knowledge:

- current Unknown and Remembered terrain remains visible and pickable under one dark
  translucent exact-surface cap;
- Observed surfaces have no cap and render normally;
- a hostile root receives `PresentationOcclusionReason::Fog` unless the player
  currently observes that exact unit;
- hidden hostiles retain only the anonymous `Unobserved hostile` initiative slot
  during active combat, never their model, identity, location, inspection, targeting,
  health bar, or world marker.

Authoritative effects may change hidden terrain or units, but player-facing impact
presentation and combat logs filter every outcome through the receiving faction's
current knowledge. An acknowledgment may exist for simulation, replay, or saving
without disclosing its hidden position, material, resistance, occupancy, or damage.

Fog joins the explicit review-roof cutaway by contributing its own independent
occlusion reason to one composed result. Character-camera tree handling is a separate
renderer-neutral opacity request: it fades every chunk sharing one exact tree root and
never makes fogged content visible. No system may set `Visibility::Visible` to undo
another system's hide, or treat camera presentation as a knowledge change. Picking,
shadows, overlays, units, terrain, and props derive their final state from the same
composition.

`GameplaySetup::Perception` runs after actors exist and before generated view framing.
The `hex_perception` crate owns illumination, sight, faction knowledge, and
their authoritative queries. It may read units and shared map projections.
`hex_units` will consume only the compact `LocalMapKnowledge` projection in `hex_core`
for the pending movement adapter; `hex_combat` consumes the richer
current-observation API through gameplay-owned adapters for hostile lattice
knowledge, cast validation, and AI. Engagement and ordinary-attack adapters remain
pending and must use the same authority.

Later frames preserve the same authorization-critical ordering across crates:

`PublishKnowledge` → combat spatial-knowledge synchronization →
`CombatSystems::Act` → `CombatSystems::Apply`, followed by the normal `Resolve` and
`Advance` phases.

This is a semantic boundary and a Bevy synchronization boundary. A same-frame
position, observer, or `Downed` change is visible to AI before it selects a command,
and the applier validates against the publication for that frame.

The live ECS adapter caches each ordered stage. Terrain, substance, interior, or
blocker changes rebuild the exact `SurfaceSnapshots`; a `RunBottom`-only occupancy
change restarts at observation; ambient or local-light changes reuse the surface cache
and restart at illumination; unit positions and `perception.ron` changes restart at
observation. Exact occupancy is published after restore and before the first setup
observation, so missing or malformed material never produces a one-frame clear map.
Unchanged gameplay frames run only the change detector. Every update-stage system
belongs to `PausableSystems`, while the setup pass still resolves one complete initial
frame before view framing.
`PerceptionRuntimeStats` exposes the four recomputation counts in the development
inspector and headless benchmarks.

`hex_perception` imports only Bevy application, ECS, state, logging, and reflection
subcrates directly. Its current `hex_assets` and `hex_units` dependencies still bring
the Bevy facade transitively for their own behavior. “Headless” therefore means that
authoritative perception is renderer-independent; it does not claim that the whole
transitive build graph is renderer-free yet.

## Verification gate

Headless tests must cover static, sun-key, moon-key, and dark ambient resolution;
light-domain containment; inclusive upper-dome local-light boundaries; maximum-tier
overlap; pooled party sight; unchanged head-to-top range; the paired center plus six
matching-corner LOS bundle; its observer-local three-of-six threshold without
cross-pairs or cross-observer pooling; every radius-eight ground target around an
isolated ten-level pillar; full-run one-level ridges immediately before aligned and
off-axis targets in every rotation; stepped near-field relief; the corresponding
two-level walls; disconnected one-voxel runs both inside and outside the observer's
level band; direction-symmetric raw segment intersection and intentionally
observer-relative character visibility; exact tangencies; roofs, air gaps, stacked
surfaces, open domain thresholds; and the alternative `24/8/1` and `18/6/1` review
profiles.

Visual review is presentation evidence, not a substitute for those typed contracts.
Every behavior-named capture must record its scenario, seed, time/light setting,
camera mode, and exact anchor. Byte-identical captures cannot be cited as evidence for
different cases unless that equality is the expected result and is stated explicitly.

Knowledge tests prove Unknown contains no snapshot, Remembered retains the exact
last-seen terrain and blockers, unseen terrain edits and feature changes do not leak,
units vanish outside observation, and re-observation replaces stale facts. The
radius-40 matrix covers 4,921 exact surfaces across active/inactive observers, light
changes, memory, and re-observation. Ten thousand unchanged gameplay frames produce
zero recomputations. Movement adapter tests must eventually cover the one-step Unknown
frontier without exposing hidden level, headroom, material, or rejection reason.

The live casting and AI tests require an Observed exact anchor, reject Remembered and
Unknown anchors, allow area spillover into hidden positions without disclosure, and
prove same-frame loss of the sole sight provider removes hidden identity and targets
before AI decides. Engagement adapters still require observation-gated reach,
asymmetric detection, independent disengage-margin and lost-contact behavior, full
one-round search, reacquisition, and search-expiry tests before they change live
rules. Gameplay teardown and re-entry tests run 100 cycles and require the exact
expected entity and resource counts after each exit.

Benchmarks record faction-knowledge recomputation after unit movement and terrain
edits, retain the radius-40 release p95 below 50 ms and dense-wall six-observer p95
below 150 ms, keep the LOS maximum at seven segment tests per observer-target pair,
and prove 10,000 unchanged frames perform no downstream recomputation. Fog checks
bound overlays to one per shaded surface and one shared mesh/material. Visual review
captures one seed and azimuth at noon, moonlight, darkness, wall occlusion, and an open
cave threshold in Map, Third Person, and First Person. The chosen cap renderer shades top
surfaces; complete cliff-side and tall-prop darkening remains a future full-scene
renderer concern.

## Deferred deliberately

- cover and physical-shadow gameplay
- automatically advancing time and gameplay effects beyond exterior illumination
- carried, destructible, extinguishable, faction-private, and spell-created lights
- stealth, concealment, hearing, and hidden-unit detection
- spatial divination that reveals unknown terrain; Divination's current
  observed-subject, bounded lattice Reveal is live in `hex_combat`, while Scrying
  Eye's proposed readable off-sight live feed remains later work
- semantic prop, vegetation, and unit sight obstruction
- full-scene fog shading, soft edges, and fades
- saved-game persistence for remembered terrain

These are extensions of the boundary, not reasons to bypass it.

## Primary precedents

The separation is informed by mature implementations, not copied from any one of
them:

- [NetHack's vision interface](https://github.com/NetHack/NetHack/blob/NetHack-3.7/include/vision.h)
  keeps sight calculation separate from map display and memory.
- [OpenRA's shroud implementation](https://github.com/OpenRA/OpenRA/blob/bleed/OpenRA.Game/Traits/Player/Shroud.cs)
  distinguishes explored cells from cells currently visible to a player.
- [Freeciv's server-side map knowledge](https://github.com/freeciv/freeciv/blob/main/server/maphand.c)
  updates each player's remembered map without exposing current world truth.

Those sources justify the boundary. The numeric radii, upper-dome rule, exact
strict-interior samples, public light domains, and one-round search above are Hex's own
authored rules.
