# Perception

The contract for deciding what a faction can see, what it remembers, and what the
renderer may disclose. The shared vocabulary and setup ordering were established
before the runtime system on purpose: map, unit, combat, and presentation work can
compile against one boundary without reaching into one another's crates.

> **Status:** authoritative headless illumination, pooled faction sight, and
> Unknown/Remembered/Observed map knowledge are live in `hex_perception`.
> V3 Caves publishes fixed local gameplay lights into that live pipeline.
> Casting anchors, hostile lattice disclosure, and AI observation/traversal now
> consume that authority. Fog/picking presentation, visible crystal and
> physical-light presentation,
> unknown-frontier movement, engagement, and lost-contact search remain later
> isolated work.

## Four facts, not one

"Visible" is too overloaded to be a useful domain word. Perception keeps four facts
separate:

1. **Illumination** is an objective property of an exact place in the world.
2. **Sight** asks whether one faction currently observes that place.
3. **Knowledge** is what that faction is allowed to know about it now.
4. **Presentation** turns that knowledge into meshes, overlays, picking, and UI.

A physically bright pixel does not make a tile observed. A remembered tile may be
drawn without being observed. Hiding a cave roof for the camera does not illuminate
the cave below it. No gameplay rule may infer knowledge from Bevy's `Visibility` or
the PBR light list.

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
positions in its current domain and inside both its inclusive horizontal and vertical
radius. This prevents a cave lamp from shining through its roof and daylight from
filling a tunnel, without requiring line-of-sight. Inside one domain, local light is
deliberately radial: walls, corners, units, props, and shadows do not obstruct it.

Gameplay lights are public world facts. The same lamp, crystal, or future carried
light illuminates a place for every faction; there are no faction-private light
volumes. Overlapping sources take the maximum level rather than adding brightness.

V3 Caves places fixed Bright sources with radii from four through seven. They cover
the entrance, required route, and critical chambers while preserving dark optional
branches. Authored emissive crystal meshes and restrained physical lights will
communicate the rule after the object-renderer stack lands, but they never implement
gameplay illumination.

## Sight

A `SightProfile` maps each illumination level to a horizontal and vertical
`SightBand`. The initial ordinary profile is:

| Target illumination | Horizontal band | Vertical band |
|---|---:|---:|
| `Bright` | 36 | 36 |
| `Dim` | 12 | 12 |
| `Dark` | 1 | 1 |

The target's effective illumination selects the band. An observer sees a target
surface only when its exact `TilePos` falls within both limits. Horizontal distance
is cube-coordinate distance; vertical distance is the absolute level difference.
Observer and target must belong to the same light domain. This milestone does not
trace intervening terrain or sight through entrances between domains.

Using the same generated spatial partition for initial sight is deliberately coarse:
it prevents an exterior observer from seeing a lamp-lit chamber through an opaque
roof, but also prevents looking across an open cave threshold until the observer
crosses it. Portal-aware cross-domain sight is a later, separately reviewed
extension. `LightDomain` still does not turn illumination into sight; it is only an
eligibility boundary applied before the independent sight-band test.

Bright and Dim sight gain one extra horizontal hex for every four complete levels
the target is below the observer, capped at six extra hexes. Looking uphill grants
nothing. Dark sight never gains an elevation bonus. Radius-one Dark sight is the
character's immediate awareness in absolute darkness, not emitted light.

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

Once fog and picking land, Unknown terrain will remain unpickable. Presentation will
supply a generic frontier affordance rather than exposing a hidden tile entity, level,
material, headroom, or passability. Execution resolves the attempted final step
against the authoritative map. A rejected step leaves the unit at the known frontier
and must not disclose which hidden condition rejected it. A planner never searches
through several Unknown coordinates or uses failed probes to reveal an alternate
route.

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

The fog adapter will consume faction knowledge:

- Unknown places are featureless, unpickable, and disclose no underlying geometry.
- Remembered terrain is visually distinct from current observation and cannot show
  units or unseen changes.
- Observed places render current world state.

Authoritative effects may change hidden terrain or units, but player-facing impact
presentation and combat logs filter every outcome through the receiving faction's
current knowledge. An acknowledgment may exist for simulation, replay, or saving
without disclosing its hidden position, material, resistance, occupancy, or damage.

Fog will join the live cave-roof and canopy cutaways by contributing its own
independent occlusion reason to one composed result. No system may set
`Visibility::Visible` to undo another system's hide, or treat a camera cutaway as a
knowledge change. Picking, shadows, overlays, units, terrain, and props derive their
final state from the same composition.

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
blocker changes rebuild the exact `SurfaceSnapshots`; ambient or local-light changes
reuse that surface cache and restart at illumination; unit positions and
`perception.ron` changes restart at observation. Unchanged gameplay frames run only
the change detector. Every update-stage system belongs to `PausableSystems`, while
the setup pass still resolves one complete initial frame before view framing.
`PerceptionRuntimeStats` exposes the four recomputation counts in the development
inspector and headless benchmarks.

`hex_perception` imports only Bevy application, ECS, state, logging, and reflection
subcrates directly. Its current `hex_assets` and `hex_units` dependencies still bring
the Bevy facade transitively for their own behavior. “Headless” therefore means that
authoritative perception is renderer-independent; it does not claim that the whole
transitive build graph is renderer-free yet.

## Verification gate

Headless tests must cover static, sun-key, moon-key, and dark ambient resolution;
light-domain containment; inclusive local-light radii; maximum-tier overlap; pooled
party sight; downhill caps; exact stacked surfaces; and the alternative `24/8/1` and
`18/6/1` review profiles.

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
edits. Fog recomputation benchmarks and the visual review captures are deferred
until the fog/picking presentation adapter exists: the headless milestone has no
renderer output capable of showing Unknown, Remembered, or Observed state. That
presentation PR must capture one seed and azimuth at noon, moonlight, Remembered fog,
cave darkness, and local-light states in both map and character cameras. Those
captures must show no hidden-change leakage, black-but-playable terrain, or
disagreement between gameplay knowledge and picking.

## Deferred deliberately

- obstruction-aware line-of-sight, cover, and physical-shadow gameplay
- automatically advancing time and gameplay effects beyond exterior illumination
- carried, destructible, extinguishable, faction-private, and spell-created lights
- stealth, concealment, hearing, and hidden-unit detection
- spatial divination that reveals unknown terrain; lattice divination and its
  persistent knowledge are live in `hex_combat`
- cross-domain sight through entrances and other portals
- saved-game persistence for remembered terrain

These are extensions of the boundary, not reasons to bypass it. In particular,
future obstruction-aware sight replaces the radial acceptance test; it does not
change the meanings of illumination, knowledge, or presentation.

## Primary precedents

The separation is informed by mature implementations, not copied from any one of
them:

- [NetHack's vision interface](https://github.com/NetHack/NetHack/blob/NetHack-3.7/include/vision.h)
  keeps sight calculation separate from map display and memory.
- [OpenRA's shroud implementation](https://github.com/OpenRA/OpenRA/blob/bleed/OpenRA.Game/Traits/Player/Shroud.cs)
  distinguishes explored cells from cells currently visible to a player.
- [Freeciv's server-side map knowledge](https://github.com/freeciv/freeciv/blob/main/server/maphand.c)
  updates each player's remembered map without exposing current world truth.

Those sources justify the boundary. The numeric bands, public light domains,
elevation rule, and one-round search above are Hex's own authored rules.
