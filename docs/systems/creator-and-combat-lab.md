# Creator and Combat Lab

Wave 6 replaces the old title-screen Lattice Demo with three vertically stacked entries
in the existing **Demos** column: **Character Creator**, **Spell Creator**, and
**Combat Lab**. Character and spell libraries have independent screens and workspaces;
the character workspace links directly to spell management when an inscription needs
work. The lattice
mechanics sandbox still exists, but only as a local test launched from the Creator.
Ability Lab and Raider Mirror are fixtures selected inside Combat Lab rather than
separate title cards.

This document owns the creation-library, creator, sandbox, deployment, fixture, and
launch-snapshot contracts. `hex_assets` owns serializable content, `hex_combat` owns
the fail-closed deployability decision, and `hex_game` owns persistence and UI.

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

Combat Lab has two tabs.

### Sandbox

Sandbox offers all thirteen distinct shipped environments through the versioned
`combat_lab_maps.ron` catalog: Flat Arena, The Crossing, Procedural Hills, Rolling
Hills, Frozen Hills, Volcanic Hills, Sky Islands, Mountains, Caves, Waterfall, and
Forest, Fort, and Seven Regions. Duplicate scenario uses of Flat Arena and The
Crossing do not create duplicate map choices. Each stable map record names its
scenario, deterministic seed contract, renderer-generated preview, tactical
description and tags, and separate Player and Hostile deployment regions. A region
resolves from an authored cube coordinate or named map anchor plus a bounded footing
radius. Both rosters are
fully editable and ordered, with one to six units per side. Choices come from packaged
templates or saved Map-ready characters. Player units are human controlled; hostile
units use the shipped `baseline-v1` AI. The setup and deployment are transient.

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
Occupied or invalid surfaces are rejected. The tester can select an earlier roster
entry, Undo, Clear either side, or deterministically Auto-place nearest legal unused
surfaces. Start Combat remains disabled until every unit has a unique valid placement.

Start Combat repositions the frozen roster onto those exact surfaces without
regenerating terrain, then enters `Active`. Retry retains the content snapshot, map
seed, roster order, and resolved surfaces. Fixed fixtures bypass Deployment because
their encounter placements are immutable.

### Fixed fixtures

One searchable selector owns the stable fixture IDs:

| ID | Purpose |
|---|---|
| `ability-lab` | aiming, reveal, friendly damage, restore, downing, revival |
| `raider-mirror` | same-archetype identity and defensive enchantments |
| `creator-spell-matrix` | packaged creator Disable, Burn, Reveal, Restore, defense |
| `creator-roster-matrix` | packaged creator rosters, ordering, selection, multi-unit combat |

Automated walks launch by ID, never by list position. Creator-format fixture records,
roster, AI, map, seed, and placements are immutable and never consult the local
creation library.

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
