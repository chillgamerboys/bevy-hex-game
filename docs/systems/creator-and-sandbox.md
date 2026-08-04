# Creator and Sandbox

The player-facing shell has two different jobs: **Campaign** continues durable play,
while **Sandbox** assembles a temporary encounter. Sandbox is the only shipping route
for choosing a map, building either roster, selecting characters, deploying them, and
launching an experiment. Direct scenario browsing, alternate combat-rule profiles,
deterministic test cases, live telemetry, and report history are not player-facing
routes.

This document owns creation-library behavior, Creator navigation, Sandbox setup,
deployment, frozen launches, and return routing. `hex_assets` owns serializable
content and the Sandbox map catalog, `hex_combat` owns fail-closed Map readiness, the
renderer-free `hex_gameplay_model` owns routes and edits, and `hex_game` owns
persistence and runtime adaptation. `hex_ui` receives immutable views and emits typed
intents; it owns none of those facts.

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

Packaged shipping content uses the same blueprint and validation path but is outside
the local file. Human templates are immutable, visible, and duplicable. Exact
automation-only Creator records are retained in `hex_game` testdata and compiled only
with deterministic test support; they are absent from `creation_presets.ron`, so local
edits can neither shadow nor mutate them. Display name is the authored character
identity, while a stable runtime archetype key and optional display-name override
carry it into play.

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
combat-compatibility check.

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

The local mechanics test may use unsaved changes. It snapshots the current lattice
plus saved Ready spell definitions and supports casting, channelling, disable,
restore, enchantment breakage, and reset without entering map gameplay. Its internal
`Screen::LatticeDemo` route is not exposed as a Main Menu action.

### Creator destinations

Creator navigation carries a typed origin and return destination; no return boolean
reconstructs history. Character Creator and Spell Creator entered from Tools return
to Tools. Creating a character from a Sandbox picker returns to that exact side and
slot, highlights the newly saved character, and does not apply it automatically.

**Open in Sandbox** replaces the old map-test action. It is enabled only for a saved,
clean, Map-ready character. It preserves the current Sandbox map and Enemies, puts
the character in Party slot 1, clears Party slots 2–6, and opens the Party selector.
Leaving that typed Creator-origin flow returns to the Creator.

## Sandbox model

`SandboxModel` is the sole setup authority. Its route is one of Overview, Map
Browser, Map Detail, a Party or Enemies roster, or a character picker carrying an
exact side and slot. `SandboxDraft` contains one committed map and two ordered arrays
of exactly six bounded slots. Empty interior slots and duplicate characters are
valid. Launch flattens occupied slots in stable slot order.

The initial draft is Flat Arena, one Hedge Mage in Party slot 1, and one Raider in
Enemies slot 1. The draft remains in memory across child routes, return from gameplay,
Main Menu exit and re-entry, and Creator excursions.

Character selection is a preview until **Use Character** commits it. Back cancels the
preview. Party and Enemies use the same slot implementation; typed side/slot identity
prevents a selection from being applied to the wrong roster.

Start refusal is centralized and always chooses the first applicable reason:

1. `Sandbox maps are still loading.`
2. `Choose a map.`
3. `The selected map is unavailable.`
4. `Add at least one Party character.`
5. `Add at least one Enemy character.`
6. `{Party|Enemies} slot N is not Map-ready: {reason}` for the first failing slot.

The draft has no rule-profile field. Every normal Sandbox launch freezes the shipped
combat rules loaded from `combat.ron`.

### Map selection

`sandbox_maps.ron` is the exclusive player-facing map catalog. Its stable IDs,
scenario names, authored seeds, and Party/Enemy staging metadata are compatibility
contracts. The two regions stage hidden actors deterministically while terrain loads;
they do not constrain manual placement. Selecting a row creates a pending map choice.
Regenerate changes only that pending generated seed; authored maps expose no
regeneration. **Use Map** commits the pending choice, while Back discards it. **Create
New Map** is visible but disabled with **Coming Soon**.

The catalog currently resolves Flat Arena, The Crossing, Procedural Hills, Rolling
Hills, Frozen Hills, Volcanic Hills, Sky Islands, Mountains, Caves, Waterfall,
Forest, Deep Forest, Prairie, Fort, Seven Regions, Two Rings, and Mountain Range.
Duplicate internal scenario uses do not create duplicate choices.

## Deployment and frozen launch

Sandbox loading has an explicit `Preparing → Deployment → Active` phase boundary.
Preparing installs the frozen content and terrain. During Deployment the terrain and
camera run while actors, AI, combat, casting, campaign saving, and the ordinary
gameplay HUD remain inactive. Phase-level suppression removes every ordinary HUD
surface, including the Action Bar, from layout, focus, scrolling, and picking without
changing the player's stored HUD preference. One compact modal task card remains over
the interactive map.

Occupied sparse slots form one stable queue: Party slots first, then Enemy slots, each
in original slot order. The current character owns the next primary terrain click.
`hex_game` validates the clicked exact `TilePos` against the canonical walker footing,
including solidity, headroom, traversal blockers, and elevation. Any legal,
unoccupied surface on the map may be chosen; catalog staging regions impose no
manual-placement boundary. Invalid or occupied surfaces produce a visible typed
refusal without changing the placement or queue position.

A successful click paints an exact placement token and advances to the next character.
Future unplaced entries remain disclosed but disabled until the stable queue reaches
them. The player may select an earlier placed character to reposition it or use
**Undo** to restore the previous exact edit. Undo returns to Review whenever that
restoration is complete. The final placement enters **Review**, where the compact task
surface offers **Undo**, **Return to Sandbox**, and **Start Combat**. There are no
shipping clear-side or automatic-placement actions. Start remains refused outside
Review or unless every occupied slot owns one unique valid surface.

Start freezes a `SandboxLaunchSnapshot`: exact map and resolved seed, flattened
ordered rosters, accepted content revision, shipped rules, and eventual deployment.
Loading and **Retry Exact** consume that same snapshot. Retry therefore cannot observe
later map regeneration, draft edits, creation-library changes, or different surfaces.
Leaving restores the shipped content namespace.

Session provenance is typed as Campaign with its exact slot, Sandbox, or TestFixture.
Only Campaign provenance is save-eligible. A terminal Sandbox encounter exposes only
Victory or Defeat, **Retry Exact**, and **Return to Sandbox**.

## Internal scenario and test-support contracts

`Scenario`, `ScenarioLibrary`, `ScenarioToLoad`, and loading validation remain the
internal world + lighting + encounter launch contract. Campaign, Sandbox, saves,
retry, deterministic review, and tests all use it. `ScenarioCategory` remains inert
compatibility metadata while legacy resumes can still refer to the current
`scenarios.ron`; it does not drive a shipping route.

Deterministic fixtures such as `ability-lab`, `raider-mirror`, and `tempo-matrix`
retain their stable IDs behind the default-off `test-support` feature. Typed fixture
launch requests may inject a rules profile for tests and simulations, but the
shipping plugin graph and UI cannot reach that capability. Tests observe canonical
`CombatSummary`, deterministic run snapshots, launch/retry identity, and terminal
outcomes instead of product report history.

`combat-reports.ron` is obsolete product data. The application never reads, writes,
migrates, or deletes it; an existing file is deliberately left untouched.
