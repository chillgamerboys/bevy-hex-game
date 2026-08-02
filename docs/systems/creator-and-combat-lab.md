# Creator and Combat Lab

Wave 6 replaces the old title-screen Lattice Demo with direct **Character Creator**,
**Spell Creator**, and **Combat Lab** primary routes. Character and spell libraries
retain independent screens and workspaces, and the character workspace links directly
to spell management when an inscription needs work. The lattice mechanics sandbox
still exists, but only as a local test launched from the Creator. Ability Lab and
Raider Mirror retain stable fixtures inside Combat Lab and also appear as focused
entries in the separate Demos catalog.

This document owns the creation-library, creator, sandbox, deployment, fixture, and
launch-snapshot contracts. `hex_assets` owns serializable content, `hex_combat` owns
the fail-closed deployability decision, and `hex_game` owns persistence and UI.

Wave 7 adds a versioned `CombatRulesProfile` without changing that ownership. Shipped
values are copied from validated `combat.ron`; Tactical two-step changes only movement
per turn; Custom edits remain inside published numeric bounds. Version 2 mirrors every
shipping authority input: seven bounded numeric fields plus typed initiative,
action-economy, channelling, and rout policies. Those policies preserve the
implemented fixed initiative, movement-plus-one-action, burst-only, and
fight-to-the-end algorithms. Loading validates and installs an effective
session-local copy before gameplay, Retry retains it exactly, and leaving the Lab
restores the authored settings. Invalid profiles fail closed and never edit
`combat.ron`.

One versioned `CombatLabReport` combines that frozen profile with map and resolved
seed, accepted content fingerprint, ordered rosters, exact `TilePos` deployment,
Sandbox or stable fixture origin, an outcome or explicit command/turn/no-progress/manual
stop, and the gameplay-owned `CombatSummary`. Version 2 migrates version 1 outcome
reports. Summary and report fingerprinting fail closed on serialization error rather
than emitting a plausible sentinel hash. Saved entries add bounded editable labels
and notes. Local report history is a separate bounded schema from Creator creations
and Continue; fixed fixtures never consult it.

## Saved creation library

`creations.ron` is one versioned local library containing custom characters and
spells. IDs are monotonically allocated and stable across rename or modification.
The file is replaced atomically; an invalid in-memory update or failed write restores
the previous library. A corrupt or future-version file is preserved and reported
instead of being partially loaded.

Saving is explicit. A saved record can be reopened, modified in place, renamed,
duplicated, or deleted after confirmation. Incomplete characters and invalid spells
may be saved, but their diagnostics remain visible. A spell referenced by a saved
character cannot be deleted; the refusal names every dependent character. Changing a
referenced Ready spell into a Draft is allowed and leaves its characters intact but
Map-blocked.

Packaged content uses the same blueprint and validation path but is outside the local
file:

- `HumanTemplate` records are immutable, visible, and duplicable.
- `AutomationFixture` records are immutable and accessible only through fixed
  fixtures. Local edits can neither shadow nor mutate them.

Display name is the only authored character identity. Runtime archetype keys remain
stable, while an optional display-name override supplies the custom name to gameplay.

## Spell Creator

A custom spell has one to six gem requirements, fixed mana, no co-casting, and no
line-of-sight dependency. It targets Self or one unit at a configured range. The
effect list is ordered, permits repetition, and has no editor count limit.

The delivered behavior set is intentionally narrow:

- non-targeted Disable;
- Burn;
- Restore, including revival through the combat system;
- Reveal;
- a positive defensive Enchantment value.

Any other mechanic fails closed before map gameplay. Shared schema checks derive the
Draft/Ready label; `hex_combat::creator_spell_deployability` is the authoritative
combat compatibility check.

## Character Creator

The character editor is a scrollable contiguous axial lattice. It exposes neighbor
add slots, a cell inspector, gem/fusion/spell/blank choices, stats and validation,
manual attunement and channelling, and session undo/redo. The schema requires an
origin, connectivity, valid content names and coordinates, and no more than 64 cells.

New inscriptions may select shipped spells or saved Ready custom spells. Existing
references are retained if a spell later becomes Draft, with an actionable diagnostic.
A Map-ready character is saved, clean, schema-valid, contains an inscribed spell, and
resolves all referenced spells. Final launch resolution also proves the selected
lattice can freshly cast at least one inscription.

The local mechanics test may use unsaved character changes. It snapshots the current
lattice plus saved Ready spell definitions, and supports casting, channelling,
disable, restore, enchantment breakage, and reset without entering map gameplay.

## Combat Lab

Combat Lab has three tabs: Sandbox, Fixed Fixtures, and Saved Reports.

Its unattended workbench uses `hex_combat_core`, the same serializable reducer hosted
by live combat. A case freezes arena/observation facts, roster and lattices, active
single-target spell content, and the complete rules projection. Controllers are
either exact per-unit command scripts (including defender/restoration choices) or the
stable non-random baseline policy. Independent typed command, turn, and no-progress
bounds always produce an explicit termination. Cast payment, direct damage,
restoration, Burn ticks, downing, revival scheduling, and annihilation outcomes reduce
without a Bevy `App`, ECS schedule, renderer, clock, asset server, or map generator.
Exploration and unsupported terrain/area verbs remain named host adapters and cannot
silently count as simulation evidence.

### Sandbox

Sandbox offers all sixteen distinct shipped maps through the versioned
`combat_lab_maps.ron` catalog: Flat Arena, The Crossing, Procedural Hills, Rolling
Hills, Frozen Hills, Volcanic Hills, Sky Islands, Mountains, Caves, Waterfall,
Forest, Deep Forest, Prairie, Fort, Seven Regions, and Two Rings. Duplicate scenario
uses of Flat Arena and The Crossing do not create duplicate map choices. Each stable
map record names its scenario, deterministic seed contract, renderer-generated
preview, tactical description and tags, and separate Player and Hostile deployment
regions. A region resolves from an authored cube coordinate or named map anchor plus
a bounded footing radius. Both rosters are
fully editable and ordered, with one to six units per side. Choices come from packaged
templates or saved Map-ready characters. Player units are human controlled; hostile
units use the shipped `baseline-v1` AI. The setup and deployment are transient.

Sandbox setup is an explicit `Map → Rosters → Rules → Deploy` flow. The Rules step
offers Shipped, Tactical two-step, and Custom profiles. Its seven labelled steppers
consume the profile contract's descriptions and inclusive bounds, and every numeric
difference is also written as `CHANGED shipped → selected`; color is supplementary.
Reset restores the exact shipped profile. Loading validates and freezes the selected
profile without writing `combat.ron`.

`Test on Map` is enabled only for the current saved, clean, Map-ready character. It
opens this same Sandbox with that record in Player slot one. The tester still chooses
the map and completes both rosters.

### Deployment

Sandbox loading has an explicit `Preparing → Deployment → Active` phase boundary.
Preparing builds the frozen namespace and terrain. During Deployment the terrain and
camera run, while actors, AI, combat, casting, saves, and the ordinary gameplay HUD
remain hidden and inactive.

The map renders clickable world-space surface caps inside both authored regions.
Candidates use the shared walker `Footing`, exact headroom and blockers, and the live
terrain surface, so a click records the complete `TilePos`, including elevation.
Occupied or invalid surfaces are rejected. Placement proceeds in roster order; the
tester may directly select any roster row to reposition an earlier unit, Undo, Clear
either side, or deterministically Auto-place nearest legal unused surfaces. Direct
placement, auto-place, and the Start gate all consume canonical exact-surface
occupancy. Start Combat remains visibly disabled until every unit has a unique valid
placement.

Start Combat repositions the frozen roster onto those exact surfaces without
regenerating terrain, then enters `Active`. Retry retains the content snapshot, map
seed, roster order, and resolved surfaces. Fixed fixtures bypass Deployment because
their encounter placements are immutable.

During a Lab encounter, a collapsible statistics drawer supplements rather than
replaces the ordinary HUD. At every viewport and semantic scale it follows the
persistent own/target lattice readout in the Inspector's single scroll flow;
statistics never render without an own lattice. Expanding it cannot hide, move, or
cover that readout, and mouse-wheel plus Tab/Shift-Tab navigation can reach every
secondary control. HUD hiding and terminal outcomes remove lattice and statistics
together. It reads
`CombatSummary` directly and labels rounds and
completed turns, current and maximum no-progress stretches, outcome,
successful/refused commands, AI choices, movement distance/budget, casts, Channel and
mana restored by element, strikes, idle turns, disable flow, restorations, downings,
and revivals. The same projection carries per-unit command, movement, spell, Channel,
disable, recovery, condition, idle, and AI totals; presentation never reconstructs
them from live entities. `End Experiment` freezes a manual-stop report and explicitly
saves it before returning to the Lab; serialization or storage failure leaves the
encounter in place and surfaces the error.

At combat start, the Lab freezes the accepted content revision, stable map and seed,
ordered roster/controller headers, and exact initial `TilePos` deployment. Outcome
combines those launch facts with the canonical summary into `CombatLabReport`; it
never substitutes final battlefield positions. The full-screen outcome surface
provides functional Overview, Units, Spells & Effects, Timeline, and Compare modes
plus Save Report, Retry Exact, and Tune & Run Again actions. Timeline reads the
bounded canonical event window and labels truncation. Compare explicitly selects a
saved report and shows signed numeric deltas against the current run. Tune returns to
Rules with the frozen map, ordered rosters, profile, and every still-valid exact
deployment surface retained.

Saved Reports lists only explicitly saved local reports and exposes a bounded label
and notes editor on each entry. The user selects the left
and right report independently; when they differ, the view shows frozen
rules/map/seed/content/roster/deployment headers, both stop reasons, and signed
aggregate, per-unit, spell, effect, and no-progress deltas. Deletion is a two-step
request/confirm action and affects only the selected local report.

### Fixed fixtures

One searchable selector owns the stable fixture IDs:

| ID | Purpose |
|---|---|
| `ability-lab` | aiming, reveal, friendly damage, restore, downing, revival |
| `raider-mirror` | same-archetype identity and defensive enchantments |
| `creator-spell-matrix` | packaged creator Disable, Burn, Reveal, Restore, defense |
| `creator-roster-matrix` | packaged creator rosters, ordering, selection, multi-unit combat |
| `occupancy-matrix` | human/AI chokepoints, endpoints, route reservations, stacked surfaces, interruption |
| `channel-attrition` | depleted/full mana, disabled cells, enchantment locks, repeated/AI Channel, downed refusal |
| `tempo-matrix` | repeated frozen Party Trial baseline under Shipped, Tactical two-step, and bounded Custom profiles |

Automated walks launch by ID, never by list position. Creator-format fixture records,
roster, AI, map, seed, and placements are immutable and never consult the local
creation library.

The three Wave 7 fixtures own launch inputs rather than merely renaming existing
scenario cards. Occupancy and Tempo materialize frozen 3v3 encounter overrides.
Channel Attrition materializes a 3v3 matrix with depleted human and AI casters, one
locked enchantment, one partly disabled body, one full-mana reference, and one fully
disabled body. Tempo exposes Shipped, Tactical two-step, and a validated Custom
three-step run over the same encounter. A fixed-fixture outcome offers Copy to
Sandbox, carrying its exact map, roster, initial placement, profile, and any frozen
creator content into the ordinary Rules and Deployment flow.

## Frozen launch and return routing

Every Creator/Combat Lab launch carries a frozen combined namespace, selected map,
resolved seed, ordered roster, optional exact deployment, return destination, and
retry contract. Loading installs the snapshot before actors are built. Retry reuses
the same active scenario, seed, encounter, and content even if the local library
changes meanwhile.

Combat Lab sessions never write Continue/resume state. The active Creator snapshot
and its shipped counterpart each bundle the raw `SpellFile` and `LatticeFile` with
their derived `SpellBook`, `ContentIndex`, and `LatticeLibrary`. Loading installs all
five before the normal content-readiness publisher creates
`AcceptedContentRevision`; it never manufactures or bypasses acceptance. Exiting
restores all five shipped resources and lets that same publisher accept the shipped
revision. Creator-origin sessions return to the Creator on setup back, pause exit, or
outcome exit; standalone Sandbox returns to Sandbox setup; fixture sessions return to
the fixture selector.
