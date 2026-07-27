# Roadmap

The epics between this skeleton and a shippable game, as rows `/seed-tickets`
can turn into Linear `HEX-*` tickets. Where each epic came from, and the
evidence behind it, is [production-audit.md](production-audit.md) — a dated
snapshot that does not change. This file is the living plan: rows get claimed,
split, and retired.

**How this file works.** Each row of the table below becomes one Linear ticket
when someone runs `/seed-tickets`, which writes an HTML-comment marker back
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

## Upcoming

| Epic | Scope | Owner |
|---|---|---|
| Deterministic sim seams | serde across the hex_core domain vocabulary; a stable `UnitId` with allocator and registry; every sim tie-break moved off entity ids; a `SimSeeds` resource | core | <!-- linear: HEX-6 owner: shravan-kumaran -->
| Command funnel | `GameCommand` vocabulary and queue in hex_core; one validating applier in hex_combat; click, SPACE, and the AI become emitters; a `Busy` gate replaces animation-component gating | combat | <!-- linear: HEX-11 owner: shravan-kumaran -->
| Combat policy knobs | `combat.ron`: engage/disengage ranges, movement budget, height bonus, and the open design questions as named policy enums that parse but reject-with-reason until built | combat | <!-- linear: HEX-10 owner: shravan-kumaran -->
| Elements and spells as content | `elements.ron` (wheel, opposition, fusion recipes) and `spells.ron` (closed-enum effects, targeting specs); a cross-file `ContentIndex` with load-time and test-time reference checks | assets | <!-- linear: HEX-7 owner: shravan-kumaran -->
| Lattice engine | new pure `hex_lattice` crate: inscription/state split, `castable()`, disable and enchantment bookkeeping, channeling; the property-test suite | combat | <!-- linear: HEX-8 owner: shravan-kumaran -->
| Lattices wired into the game | units spawn with archetype lattices from `lattices.ron`; a first `Cast`; damage, death, and the defender-chooses decision flow; a HUD readout of live/disabled hexes | combat | <!-- linear: HEX-12 owner: shravan-kumaran -->
| Run bottoms on tiles | publish `RunBottom(Level)` beside every run entity's `TilePos`, including stacked runs; accepted prerequisite to wave 3 terrain casting | map |
| Terrain magic | after boundary asks G/H/L are agreed: canonical exact-voxel `TerrainImpact` announcements using runtime `ElementId`, map-approved conjuration through `TerrainEdit::Set`, 3D volume shapes, the casting legality ladder, and deterministic `TerrainImpactOutcome` consumption; feature destruction remains deferred | combat | <!-- linear: HEX-19 owner: shravan-kumaran -->
| Persistent effects | `{source, target, payload, start, end}` in hex_core with a hex_combat runtime; rounds and enchantment-bound end conditions; `Burn` and damage-over-time become payloads | combat | <!-- linear: HEX-20 owner: shravan-kumaran -->
| Casting UX | shape previews under the cursor, blocked-reason surfacing from `castable()`, target cycling, and cast presentation per element | units | <!-- linear: HEX-21 owner: shravan-kumaran -->
| Outcome flow | victory, defeat and rout screens; what happens after a fight ends; returning to the world | game | <!-- linear: HEX-22 owner: shravan-kumaran -->
| Combat readability | initiative order display, a live/disabled lattice readout beyond a count, and a combat log | game | <!-- linear: HEX-23 owner: shravan-kumaran -->
| Trajectories and lingering effects | obstruction-aware spell trajectories once `RunBottom` and line-of-sight land, authored `Path` shapes, area-lingering zones, and dispel | combat | <!-- linear: HEX-24 owner: shravan-kumaran -->
| Magic outside combat | casting in real time, which first requires an answer to out-of-combat mana regeneration | combat | <!-- linear: HEX-25 owner: shravan-kumaran -->
| Channelling and co-casting | the always-available channel action, and rituals — which wait on the initiative question being settled | combat | <!-- linear: HEX-26 owner: shravan-kumaran -->
| Encounters | `encounters/*.ron`: rosters by archetype, spawn zones, anchor placements, a formation anchor; retires the two-coordinate scenario scaffold | game | <!-- linear: HEX-14 owner: shravan-kumaran -->
| Save and load | versioned `SaveFile` snapshot of domain state; the terrain edit log; restore through the existing Loading flow; then mid-combat saves | game | <!-- linear: HEX-15 owner: shravan-kumaran -->
| Knowledge and divination seam | `FactionKnowledge` with a `view()` accessor and round-based decay; UI and AI read hostile lattices only through it | combat | <!-- linear: HEX-13 owner: shravan-kumaran -->
| Ship-hygiene basics | panic hook, log-to-file, version display in the title, diagnostics logging off in release | game | <!-- linear: HEX-9 owner: shravan-kumaran -->
| Settings menu, persistence, and audio | in-game options backed by bevy_persistent; window modes; input-map centralization; an audio facade over bevy_kira_audio with volume buses | game | <!-- linear: HEX-16 owner: shravan-kumaran -->
| Steam packaging and crash reporting | app icon, macOS codesign/notarize lane, Steam depot upload on the release workflow, split debug symbols, opt-in crash reporting via sentry-rust-minidump | game | <!-- linear: HEX-17 owner: shravan-kumaran -->
| Engine upkeep | the one budgeted Bevy 0.20 upgrade (~Q4 2026) plus the feature trim, landed together in a quiet window before any release | game | <!-- linear: HEX-18 owner: shravan-kumaran -->
| V3 procedural foundation | `generator_version: 3`; private `GeneratedWorldPlan`; `Single` and radius-33 `Ring7` layouts; patch masks, edge contracts, named streams, validation, scoring, repair, fallback, and diagnostics | map |
| Steady-state liquids and Waterfall | directed water topology; still/current/rapid/fall rendering; elevated inlet, rapids, fall, basin, outlet, and ordinary-walker bypass | map |
| Authoritative spatial perception | new headless `hex_perception`: validated `perception.ron`, illumination domains, pooled faction sight, Unknown/Remembered/Observed knowledge, deterministic visibility, and cave-local lights | perception |
| Perception presentation | faction fog, remembered rendering, picking gates, and composition with cave/canopy cutaways | perception |
| Movement and combat perception adapters | unknown-route restriction; detection, engagement, targeting, AI, and one-round last-known-position behavior in isolated owner-reviewed PRs | units/combat |
| Surface features and Forest | deterministic low-poly trees and grass, exact root blockers, protected routes, clearings, prairie, and composable canopy cutaway | map |
| Structures and Fort | worked-stone walls, towers, gates, keep, wall walks, stairs, battlements, and validated defensive circulation | map |
| Seven-region composition | one radius-33 world: central Hills, then Mountains, Waterfall, Forest, Fort, Caves, and Sky Islands clockwise; global routes, elevation seams, and hydrology before patch interiors | map |
| V3 recipe migration and legacy removal | rebuild every active V1/V2 recipe and scenario in V3; approve replacement corpora; then remove both legacy parsers, generators, assets, and runtime tests | map |
| Named rule regions | revisit a content-addressable exact-surface overlay when the first region-sensitive spell lands; do not combine biome identity, lighting, and anti-magic into generic tile tags | map/combat |
| Pre-spawn terrain edit replay | drain a `PendingTerrainEdits` resource after map build and before first spawn, so save-restore and authored pre-battle terrain cost zero respawns | map |
| Terrain snapshot | a name-keyed `VoxelMap` dump behind a request/response pair, making saves survive generator changes | map |

## Sequencing — independent lanes behind one contract

The V3 program begins with a small contract PR: documentation, shared
`hex_core` vocabulary, headless tests, and a reserved `GameplaySetup::Perception`
phase. It changes no behavior or existing `hex_units`/`hex_combat` systems; test
harnesses only mirror the expanded shared setup chain. Both implementation lanes
branch from updated `dev` only after that contract merges.

**Map lane:** V3 foundation → liquid topology → opaque renderer → Waterfall →
Forest → Fort → `Ring7` → remaining recipe migration → V1/V2 removal. The map
owner keeps semantic plans private and publishes exact shared consequences.
Recipe PRs do not edit gameplay-owned crates.

**Perception lane:** headless illumination and faction knowledge → fog
presentation and cave lights → movement adapter → engagement/targeting/AI
adapters. `hex_perception` may observe unit positions, while `hex_units` reads
only `LocalMapKnowledge` from `hex_core`. Every adapter that changes an owned
crate is isolated and reviewed by that owner.

Waterfall and headless perception can run concurrently after the foundation
contract. Forest and Fort do not depend on combat integration. `Ring7` waits
for Waterfall, Forest, and Fort semantic plans; V3 migration waits for the
composite contracts but not for final combat tuning.

The first liquid implementation also records an explicit terrain-edit policy for
support removal and stale flow topology. The `diggable` flag governs only direct
edits to a liquid voxel; it is not a substitute for that policy. Dynamic cave-breach
illumination remains unresolved: terrain edits do not reclassify an entire chamber
until aperture and domain semantics are agreed.

The pre-existing gameplay critical path remains independent: sim seams →
funnel → lattices wired, with element content and the lattice engine feeding
it from the side. Spatial map knowledge does not replace the lattice-specific
Knowledge and divination seam: the former answers which world entities a
faction currently observes, while the latter answers what that faction knows
about an observed enemy's lattice.

### The gameplay lane, in waves

The gameplay side delivers in **waves**: a short-lived `wave/N-*` branch collects a
group of ticket PRs in dependency order, a human walks the integrated build once, and
the whole wave lands on `dev` in one merge (CONTRIBUTING.md has the rules). Waves 1 and
2 are done — content, the lattice engine, sim seams, combat knobs, the command funnel,
and ship hygiene are on `dev`.

- **Wave 3 — the slice becomes a game.** Lattices wired (the damage loop: cast,
  disables, downed state), Terrain magic, Persistent effects, Knowledge and divination,
  Encounters. `RunBottom` lands before Terrain magic starts, and Terrain magic starts
  only after the declarative impact, outcome, and conjuration-admission asks G/H/L have
  an agreed shape. Other wave work need not wait for those boundary contracts. Damage
  exists at the end of it.
- **Wave 4 — combat feel and casting UX.** Casting UX, Outcome flow, Combat
  readability, Trajectories and lingering effects, Magic outside combat, Channelling
  and co-casting — plus **Movement and combat perception adapters**, the gameplay half
  of the perception lane, which is planned rather than scheduled: it starts when
  `hex_perception` lands, on the world lane's clock.
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
`SimSeeds`, the seat/party fields) ships as wave 2's opening PR.

### Command funnel

Everything that changes sim state becomes a `GameCommand` (`MoveAlong`,
`Strike`, `EndTurn`, later `Cast`/`Channel`/`ChooseDisables`) pushed onto a
`CommandQueue` resource and applied at one schedule point in hex_combat:
validate (whose turn, seat ownership, `Reach`-checked path, later
`castable()`) → apply (the only sim mutation site) → project (animation,
overlays). `on_tile_clicked`, the SPACE handler, and the AI keep their logic
but end by emitting instead of mutating. A `Busy` component becomes the
"still presenting" gate so turn logic stops depending on hex_anim's
`Transformation` component. The drained queue is the replay log, the save
adjunct, and the future network payload. Standing rule that lands with it:
never key a sim decision on entity order or query iteration order.

### Combat policy knobs

Move the provisional constants (`ENGAGE_RANGE`, `DISENGAGE_MARGIN`,
`MOVEMENT_PER_TURN`, default initiative, `LEVELS_PER_BONUS_RANGE`) into
`combat.ron` via the existing loader traits, and express the deliberately-open
design questions from [the design](../design/game.md#open-questions) as policy
enums whose variants are the doc's own options (initiative source, action
economy, channeling trickle, rout). Unimplemented variants parse but fail the
loading screen with a reason naming what they wait on — flipping a playtest
option becomes a file edit, and nothing gets settled by accident.

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
`LatticeState` (mana, disabled set, enchantment locks, burns — all integer,
all BTree-ordered); `castable()` returning either a `CastPlan` (the exact
gem-to-requirement assignment) or a reason the UI can show; `apply_cast`,
`apply_disables` (breaking enchantments burns their locked mana),
deterministic `channel`. Property tests: two tier-6 spells can never be
adjacent, disabling a locked gem kills its enchantment, fusion chains die
downstream, serde round-trips are identity.

### Lattices wired into the game

The moment damage exists. Units spawn with archetype lattices
(`lattices.ron`: wolf, raider, hedge-mage authored as cube-coordinate
entries); the placeholder `Strike` becomes an ember-grade `Cast` through the
funnel; disables flow through a `PendingDecision::ChooseDisables` suspension
point (defender-chooses is a protocol fact — an AI auto-policy answers it
today, another human answers it in co-op later); death = all hexes disabled →
leaves the turn order; the HUD shows your own live/disabled count. Fight
length, initiative source, and rout stay knobs — nothing here settles them.

### Encounters

Replace the two-coordinate scenario scaffold with encounter files chosen per
scenario the same way worlds and lighting already are: a party anchor plus
formation offsets, named spawn zones with deterministic fill, a roster of
archetype references. Should support `Anchor("name")` placements — PR #52's
`MapAnchors` mechanism — alongside zones. Later additions (triggers, quests)
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
both the feature and the future anti-cheat filter: `FactionKnowledge` maps
(viewer faction, subject) to what has been revealed and until when; UI and AI
read hostile lattices only through `view()`; a decay system ticks reveals at
round ends. Ships with a dev reveal-all toggle and a v1 "unknown lattice,
N hexes" readout from base visibility.

### Ship-hygiene basics

The smallest production ticket and a good pipeline warm-up: a panic hook,
log-to-file (the Windows release currently logs nowhere — its console is
disabled and stdout goes with it), the workspace version displayed on the
title screen, and the always-on diagnostics logging turned off in release.
Nothing here depends on anything.

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
`LocalMapKnowledge` view consumed by movement.

Fog rendering, unknown exploration, engagement, targeting, AI, and
last-known-position behavior arrive as separate adapters after the headless
rules pass. The existing Knowledge and divination epic remains responsible
for hidden lattice contents; it consumes spatial observation rather than
duplicating it.

### The map rows

Specified in [boundary.md](boundary.md), each with exact signatures,
publisher/consumer, tests, and a fallback if deferred — nothing on the
gameplay side blocks on them.

### Where the rest of the documentation lives

The kind-separated docs tree — [the index](../README.md), `systems/`,
`design/`, `development/`, and this directory — was reorganised alongside
these planning docs. [status.md](status.md) is the one doc allowed to drift;
everything outside `planning/` describes contracts.
