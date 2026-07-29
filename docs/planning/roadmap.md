# Roadmap

The delivered and remaining epics between this playable slice and a shippable game.
Rows under **Upcoming** are the ones `/seed-tickets` can turn into Linear `HEX-*`
tickets. Where each epic came from, and the
evidence behind it, is [production-audit.md](production-audit.md) — a dated
snapshot that does not change. This file is the living plan: rows get claimed,
split, and retired.

**How this file works.** Each unmarked row of the Upcoming table becomes one Linear
ticket when someone runs `/seed-tickets`, which writes an HTML-comment marker back
onto the row recording the ticket and who claimed it. Never write those
markers by hand, and never take a row someone else's marker claims. Rows are
epics, not implementation plans — `/plan-ticket` produces the plan when work
starts, using the per-epic sections below as raw material. Splitting a row
later is safe: seeding is create-only and idempotent.

The `Owner` column names the crate-ownership area
([architecture.md](../architecture.md#ownership-cuts-both-ways)). `map` rows
belong to the map's owner; `units` and `combat` rows belong to the gameplay
owner. `perception` is the new headless visibility boundary, but its adapters
still belong to the crate they change. `docs` is whoever picks it up.

## Delivered

These rows are on `dev`. Their detail remains below because it records the intended
shape and the decisions the implementation now embodies; they are no longer work to
seed or claim.

| Epic | Delivered scope | Owner |
|---|---|---|
| Deterministic sim seams | Stable ids, deterministic tie-breaks, serializable domain vocabulary, seats, parties, and separated simulation seeds | core | <!-- linear: HEX-6 owner: shravan-kumaran -->
| Command funnel | One validated command queue and applier for movement, strikes, casts, decisions, and turn advancement | combat | <!-- linear: HEX-11 owner: shravan-kumaran -->
| Combat policy knobs | Validated `combat.ron` values and explicit unbuilt-policy refusals | combat | <!-- linear: HEX-10 owner: shravan-kumaran -->
| Elements and spells as content | Validated element, spell, and cross-domain content catalogs | assets | <!-- linear: HEX-7 owner: shravan-kumaran -->
| Lattice engine | Pure deterministic `hex_lattice` rules engine and property suite | combat | <!-- linear: HEX-8 owner: shravan-kumaran -->
| Lattices wired into the game | Archetype lattices, casting, defender choices, disables, downing, and HUD integration | combat | <!-- linear: HEX-12 owner: shravan-kumaran -->
| Persistent effects | Shared effect vocabulary and the combat runtime, with playable Burn | combat | <!-- linear: HEX-20 owner: shravan-kumaran -->
| Casting UX | Live blocked reasons, target cycling, shape previews, and element presentation | units | <!-- linear: HEX-21 owner: shravan-kumaran -->
| Combat readability | Initiative order, knowledge-safe lattice readouts, damage cues, and a disclosure-frozen combat log | game | <!-- linear: HEX-23 owner: shravan-kumaran -->
| Encounters | Validated rosters, archetypes, exact/anchor/formation placement, and shared scenario references | game | <!-- linear: HEX-14 owner: shravan-kumaran -->
| Knowledge and divination seam | `FactionLatticeKnowledge`, expiring Reveal, and world-observation gating | combat | <!-- linear: HEX-13 owner: shravan-kumaran -->
| Ship-hygiene basics | Panic hook, per-session log, title version, and release diagnostics policy | game | <!-- linear: HEX-9 owner: shravan-kumaran -->
| V3 procedural foundation | Private semantic plans, patch contracts, named streams, fingerprints, selection, fallback, and exact projections | map |
| Steady-state liquids and Waterfall | Directed topology, opaque animated flow, the complete Waterfall recipe, and conservative edit admission | map |
| Surface features and Forest | Forest recipe, exact tree-root blockers, protected routes, clearings, prairie, and canopy cutaway | map |
| Headless spatial perception | Authoritative illumination, faction sight, remembered terrain, and the gameplay-owned lattice-knowledge adapter | perception/combat |
| Authored object rendering | Atomic palette/style/object catalogs, Asset Workshop authoring, and render-only `ObjectInstance` consumption | shared presentation |

## Upcoming

| Epic | Scope | Owner |
|---|---|---|
| Complete-party combat integration | Integrate the active Wave 4 party-selection, deterministic AI-host, and atomic formation-traversal work into `dev`, then finish multi-party combat behavior | units/combat |
| Run bottoms on tiles | Publish `RunBottom(Level)` beside every run entity's `TilePos`, including stacked runs; accepted prerequisite to terrain casting and obstruction-aware trajectories | map |
| Terrain magic | after boundary asks G/H/L are agreed: canonical exact-voxel `TerrainImpact` announcements using runtime `ElementId`, map-approved conjuration through `TerrainEdit::Set`, 3D volume shapes, the casting legality ladder, and deterministic `TerrainImpactOutcome` consumption; feature destruction remains deferred | combat | <!-- linear: HEX-19 owner: shravan-kumaran -->
| Outcome flow | victory, defeat and rout screens; what happens after a fight ends; returning to the world | game | <!-- linear: HEX-22 owner: shravan-kumaran -->
| Trajectories and lingering effects | obstruction-aware spell trajectories once `RunBottom` and line-of-sight land, authored `Path` shapes, area-lingering zones, and dispel | combat | <!-- linear: HEX-24 owner: shravan-kumaran -->
| Magic outside combat | casting in real time; rest has settled recovery, but exploration needs its own input, time-cost, and interruption rules | combat | <!-- linear: HEX-25 owner: shravan-kumaran -->
| Channelling and co-casting | the always-available channel action, and rituals — which wait on the initiative question being settled | combat | <!-- linear: HEX-26 owner: shravan-kumaran -->
| Save and load | versioned `SaveFile` snapshot of domain state; the terrain edit log; restore through the existing Loading flow; then mid-combat saves | game | <!-- linear: HEX-15 owner: shravan-kumaran -->
| Settings menu, persistence, and audio | in-game options backed by bevy_persistent; window modes; input-map centralization; an audio facade over bevy_kira_audio with volume buses | game | <!-- linear: HEX-16 owner: shravan-kumaran -->
| Steam packaging and crash reporting | app icon, macOS codesign/notarize lane, Steam depot upload on the release workflow, split debug symbols, opt-in crash reporting via sentry-rust-minidump | game | <!-- linear: HEX-17 owner: shravan-kumaran -->
| Engine upkeep | the one budgeted Bevy 0.20 upgrade (~Q4 2026) plus the feature trim, landed together in a quiet window before any release | game | <!-- linear: HEX-18 owner: shravan-kumaran -->
| Cave lighting retrofit | generated public lamps/crystals, deterministic gameplay-light placement over the required cave route and critical chambers, and matching emissive presentation | map/perception |
| Perception presentation | faction fog, remembered rendering, picking gates, and composition with cave/canopy cutaways | perception |
| Movement and combat perception adapters | unknown-route restriction; detection, engagement, targeting, AI, and one-round last-known-position behavior in isolated owner-reviewed PRs | units/combat |
| Forest authored-object adoption | World-side Forest placements publish shared `ObjectInstance`s while blockers remain a separate exact projection; procedural plant synthesis follows only after exemplar review | map/shared presentation |
| Structures and Fort | worked-stone walls, towers, gates, keep, wall walks, stairs, battlements, and validated defensive circulation | map |
| Seven-region composition | one radius-33 world: central Hills, then Mountains, Waterfall, Forest, Fort, Caves, and Sky Islands clockwise; global routes, elevation seams, and hydrology before patch interiors | map |
| V3 recipe migration and legacy removal | rebuild every active V1/V2 recipe and scenario in V3; approve replacement corpora; then remove both legacy parsers, generators, assets, and runtime tests | map |
| Named rule regions | revisit a content-addressable exact-surface overlay when the first region-sensitive spell lands; do not combine biome identity, lighting, and anti-magic into generic tile tags | map/combat |
| Pre-spawn terrain edit replay | drain a `PendingTerrainEdits` resource after map build and before first spawn, so save-restore and authored pre-battle terrain cost zero respawns | map |
| Terrain snapshot | a name-keyed `VoxelMap` dump behind a request/response pair, making saves survive generator changes | map |

## Sequencing — independent lanes behind one contract

The V3 program began with a small contract PR: documentation, shared
`hex_core` vocabulary, headless tests, and a reserved `GameplaySetup::Perception`
phase. It changed no behavior or existing `hex_units`/`hex_combat` systems; test
harnesses only mirror the expanded shared setup chain. Both implementation lanes
branched from updated `dev` after that contract merged.

**Map lane:** V3 foundation → liquid topology → opaque renderer → Waterfall →
Forest are delivered. Fort → `Ring7` → remaining recipe migration → V1/V2 removal
remain. The map
owner keeps semantic plans private and publishes exact shared consequences.
Recipe PRs do not edit gameplay-owned crates.

**Perception lane:** headless illumination and faction knowledge (delivered) → fog
presentation and cave lights → movement adapter → engagement/targeting/AI
adapters. `hex_perception` may observe unit positions, while `hex_units` reads
only `LocalMapKnowledge` from `hex_core`. Every adapter that changes an owned
crate is isolated and reviewed by that owner.

Headless perception remains independent of the map lane. Forest and Fort do not
depend on combat integration. `Ring7` waits for Waterfall, Forest, and Fort semantic
plans; V3 migration waits for the composite contracts but not for final combat
tuning.

The first liquid implementation also records an explicit terrain-edit policy for
support removal and stale flow topology. Until topology-aware rebuilding exists, V3
authored liquid voxels and every lower voxel in their columns are protected as one
atomic semantic dependency. The map-private exact classifier and runtime admission
are live with Waterfall. The `diggable` flag still
governs legacy and non-topological liquids and is not a substitute for this policy.
Dynamic cave-breach illumination remains unresolved: terrain edits do not reclassify
an entire chamber until aperture and domain semantics are agreed.

The pre-existing gameplay critical path remains independent: sim seams →
funnel → lattices wired, with element content and the lattice engine feeding
it from the side. Spatial map knowledge does not replace the lattice-specific
Knowledge and divination seam: the former answers which world entities a
faction currently observes, while the latter answers what that faction knows
about an observed enemy's lattice.

### The gameplay lane, in waves

The gameplay side delivers in **waves**: a short-lived `wave/N-*` branch collects a
group of ticket PRs in dependency order, a human walks the integrated build once, and
the whole wave lands on `dev` in one merge (CONTRIBUTING.md has the rules). Waves 1
through 3 are done. Content, the lattice engine, sim seams, combat knobs, the command
funnel, ship hygiene, encounters, damage, casting UX, persistent effects, combat
readability, and lattice knowledge are on `dev`.

- **Wave 3 — delivered.** The slice became playable: lattices, damage, defender
  decisions, Burn, Reveal, encounters, casting UX, initiative/lattice readouts, and
  the combat log. Terrain magic did not masquerade as complete: its shared vocabulary
  and geometry landed, while actual world announcements still wait on `RunBottom` and
  the G/H/L boundary.
- **Wave 4 — active, not yet on `dev`.** Its integration branch already contains
  the deterministic AI host, six-member party controls, and atomic formation
  traversal. The remaining lane covers outcome flow, multi-party combat behavior,
  trajectories and lingering zones, magic outside combat, channelling/co-casting,
  and the gameplay-owned movement/engagement/targeting/AI perception adapters.
- **Wave 5 — productization.** Save and load, Settings/persistence/audio, Steam
  packaging and crash reporting, Engine upkeep (pinned to the Bevy 0.20 window).

Save and load sits in wave 5 rather than wave 3 deliberately. A production save may not
depend on regenerating a legacy seed, which makes the terrain snapshot (boundary ask
D2) and pre-spawn replay (D1) prerequisites — so saves wait for the world lane rather
than blocking on it.

The casting contract those waves implement — the announce model, the legality ladder,
volumes, and persistent effects — is [casting.md](../systems/casting.md).

The complete V3 map contract, fixed `Ring7` roster, fingerprint policy, recipe
stages, and removal gate live in
[world-generation-v3.md](../systems/world-generation-v3.md). Publication asks and
fallbacks in both directions are [boundary.md](boundary.md); what crosses the boundary
today, and its status, is [contracts.md](../contracts.md).

## The epics, in detail

### Deterministic sim seams

The cheapest-now, brutal-later foundations for saves, replays, and future
co-op. Four small PRs: serde derives on the hex_core vocabulary (`HexCoord`
via its constructor invariant, `TilePos`, `SubstanceId`, `TerrainEdit`) and
`Faction`, with round-trip tests and the `CubeCoord` dedup; a `UnitId(u64)`
component with a saved allocator and an entity-registry, allocation in
scenario spawn order; `TurnOrder` keyed by `UnitId` with ties broken
initiative-then-id (today's entity-index tie-break is not stable across
runs or saves); AI target and selection tie-breaks moved to `UnitId`; a
`SimSeeds` resource (world / ai-flavor / cosmetic — resolution itself takes
no RNG, by signature). Also `PlayerSeat`/`ControlOwner` (seat 0 everywhere
today) and a `Party` roster resource — one field each, and they are the
entire future co-op ownership model.

The `Turn` and `Body` serde derives were once slated to trail (their lines
sat in files #56 held), but they landed with the first serde PR after it
merged — nothing trails. The remainder of this epic (`UnitId`, tie-breaks,
`SimSeeds`, and the seat/party fields) shipped in wave 2.

### Command funnel

Everything that changes sim state now becomes a `GameCommand` (`MoveAlong`,
`Strike`, `EndTurn`, `Cast`, `ChooseDisables`; `Channel` remains reserved) pushed
onto a `CommandQueue` resource and applied at one schedule point in `hex_combat`:
validate (whose turn, seat ownership, `Reach`-checked path, `castable()`) → apply
(the only sim mutation site) → project (animation,
overlays). `on_tile_clicked`, the SPACE handler, and the AI keep their logic
but end by emitting instead of mutating. A `Busy` component is the
"still presenting" gate so turn logic stops depending on hex_anim's
`Transformation` component. The drained queue is the replay log, the save
adjunct, and the future network payload. The standing rule that landed with it:
never key a sim decision on entity order or query iteration order.

### Combat policy knobs

The provisional constants (`ENGAGE_RANGE`, `DISENGAGE_MARGIN`,
`MOVEMENT_PER_TURN`, default initiative, `LEVELS_PER_BONUS_RANGE`) moved into
`combat.ron` via the existing loader traits, alongside the deliberately-open
design questions from [the design](../design/game.md#open-questions) as policy
enums whose variants are the doc's own options (initiative source, action
economy, channeling trickle, rout). That move is complete. Unimplemented variants
parse but fail the loading screen with a reason naming what they wait on — flipping
a built playtest option is a file edit, and nothing gets settled by accident.

### Elements and spells as content

`elements.ron`: the six-element wheel (opposition is index arithmetic over
the wheel array), higher-order elements, and fusion recipes, validated
acyclic and feedable; `ElementId` assigned from sorted names exactly like
`SubstanceId` (ids never appear in files or saves — names do).
`spells.ron`: requirements as an element multiset (tier ≤ 6), casting axis
(evocation / enchantment-with-upkeep), mana axis (fixed / variable),
co-castable flag — "ritual" becomes the name for variable + co-castable, per
the design's own note. Effects are a closed enum of primitives
(disable / burn / restore / modify-incoming / reveal / illuminate /
terrain edits by substance name / displace) — no scripting engine; the lint
wall and validate-at-parse are the argument. A `ContentIndex` resolves every
cross-file name and fails loudly, plus a hex_game test module that opens
everything shipped.

### Lattice engine

The game's core system as a pure rules crate, `hex_core → hex_lattice`,
proven headless before any wiring: `LatticeSpec` (the inscription — also the
serde format `lattices.ron` and the future in-game editor share) vs
`LatticeState` (mana, disabled set, enchantment locks — all integer,
all BTree-ordered; persistent effects deliberately live in combat); `castable()`
returning either a `CastPlan` (the exact
gem-to-requirement assignment) or a reason the UI can show; `apply_cast`,
`apply_disables` (breaking enchantments burns their locked mana),
deterministic `channel`. Property tests: two tier-6 spells can never be
adjacent, disabling a locked gem kills its enchantment, fusion chains die
downstream, serde round-trips are identity.

### Lattices wired into the game

The moment damage arrived. Units spawn with archetype lattices
(`lattices.ron`: wolf, raider, hedge-mage authored as cube-coordinate
entries); `Cast` joins the deliberately retained melee `Strike` in the funnel;
disables flow through a `PendingDecision::ChooseDisables` suspension
point (defender-chooses is a protocol fact — an AI auto-policy answers it
today, another human answers it in co-op later); all hexes disabled means downed,
which leaves the turn order but retains the unit and lattice;
the HUD shows the live/disabled state. Fight
length, initiative source, and rout stay knobs — nothing here settles them.

### Encounters

The two-coordinate scenario scaffold is replaced by encounter files chosen per
scenario the same way worlds and lighting already are: a party anchor plus
formation offsets, named spawn zones with deterministic fill, a roster of
archetype references. `Anchor("name")` placements use PR #52's `MapAnchors`
mechanism alongside zones and exact coordinates. Later additions (triggers, quests)
extend the schema without breaking it.

### Save and load

A hand-shaped, versioned serde `SaveFile` in `hex_game/src/save/` — domain
snapshot, not ECS reflection (the ecosystem consensus; see the audit's
research section). Contents: scenario reference, world seed + settings digest
+ the terrain-edit log (substances by name), content digests for legible
drift refusal, units (id, seat, faction, `TilePos`, body, lattice trio,
initiative), optional combat state including any pending decision, knowledge,
campaign flags. Restore rides the existing Loading flow. World restoration is
seeded-regen + edit replay until the map-side terrain snapshot lands
([boundary.md](boundary.md), ask D2) — which is the generator-change-proof
primary format. Floats never enter a save: positions are `TilePos`, spans are
re-derived.

### Knowledge and divination seam

Hidden information is the game's uncertainty mechanism, and one accessor is
both the feature and the future anti-cheat filter: `FactionLatticeKnowledge` maps
(viewer faction, subject) to what has been revealed and until when; UI and AI
read hostile lattices only through `view()`; a decay system ticks reveals at
round ends. Ships with a dev reveal-all toggle and an opaque "lattice unknown"
readout from base visibility; capacity is itself divination-gated.

### Ship-hygiene basics

The smallest production ticket became the pipeline warm-up: a panic hook,
per-session log file (including Windows releases whose console is disabled), the
workspace version on the title screen, and diagnostics logging disabled in release.

### Settings menu, persistence, and audio

The player-facing options surface: an in-game settings menu whose values
persist across sessions via bevy_persistent; window modes (fullscreen
toggle, resolution) beside the existing `present_mode`; input-map
centralization so keys stop being hardcoded in systems (rebinding-ready,
not yet rebindable); and audio behind a small facade over bevy_kira_audio
with music/SFX/UI volume buses wired to the menu — trim the unused
`bevy_audio` feature in the same change. Versions and sources for every
crate choice are in [production-audit.md](production-audit.md).

### Steam packaging and crash reporting

The ship lane: an app icon; a macOS codesign/notarize lane (arm64 — Rosetta
retires before any plausible release window); a Steam depot upload job
stacked on the existing tag-triggered release workflow; split debug symbols
retained from release builds; and opt-in crash reporting via
sentry-rust-minidump. Independently landable pieces — the audit's research
section carries the reasoning per pick.

### Engine upkeep

The audit budgets exactly one Bevy upgrade before any release window: 0.20
(~Q4 2026, BSN asset files and assets-as-entities are the churn to watch),
landed together with the long-deferred feature trim (`default-features =
false` plus the collections actually used) so both risky changes share one
quiet window and one visual walk. Not while the command, perception, or V3
foundation contracts are moving — upgrading under an in-flight system rewrite
doubles the blast radius.

### V3 world program

V3 replaces the recipe-per-map assumption with a patch-capable semantic
world plan. `Single` keeps focused recipe iteration fast; `Ring7` composes a
central Hills region and six fixed outer recipes inside one radius-33
footprint. Shared edges, routes, elevation datums, and hydrology are resolved
before patch interiors, so the system never tries to disguise incompatible
maps with a material blend.

Waterfall establishes the liquid layer, Forest establishes surface features
and exact blockers, and Fort establishes structures and circulation. They
feed `Ring7`, then every existing recipe moves to the same V3 pipeline.
V1/V2 remain frozen development oracles only until replacement review passes;
they are removed rather than maintained as permanent compatibility paths.
The decision-complete contract is
[world-generation-v3.md](../systems/world-generation-v3.md).

### Spatial perception

Physical scene lighting remains presentation. A headless `hex_perception`
crate deterministically combines exterior illumination, interior darkness,
public local lights, unit positions, and per-faction knowledge. It owns
Unknown/Remembered/Observed state and publishes the smaller
`LocalMapKnowledge` view for the pending movement adapter.

Fog rendering, unknown exploration, engagement, targeting, AI, and
last-known-position behavior remain separate pending adapters now that the headless
rules are live. The existing Knowledge and divination epic remains responsible
for hidden lattice contents; it consumes spatial observation rather than
duplicating it.

### The map rows

Specified in [boundary.md](boundary.md), each with exact signatures,
publisher/consumer, tests, and a fallback if deferred. Most gameplay work can proceed
independently, but terrain casting deliberately blocks on `RunBottom` and the accepted
impact/outcome/conjuration contracts rather than reconstructing world facts.

### Where the rest of the documentation lives

The kind-separated docs tree — [the index](../README.md), `systems/`,
`design/`, `development/`, and this directory — was reorganised alongside
these planning docs. [status.md](status.md) is the one doc allowed to drift;
everything outside `planning/` describes contracts.
