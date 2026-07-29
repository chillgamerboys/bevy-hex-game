# Roadmap

The remaining epics between the current build and a shippable game. The table under
**Upcoming** is the one `/seed-tickets` can turn into Linear `HEX-*` tickets.
What is already built lives in [status.md](status.md); release history lives in the
root changelog. Where each epic came from, and the
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

## Upcoming

| Epic | Scope | Owner |
|---|---|---|
| Complete-party combat integration | Finish party selection, deterministic AI hosting, atomic formation traversal, and multi-party combat behavior | units/combat |
| Run bottoms on tiles | Publish `RunBottom(Level)` beside every run entity's `TilePos`, including stacked runs; accepted prerequisite to terrain casting and obstruction-aware trajectories | map |
| Terrain magic | after boundary asks G/H/L are agreed: canonical exact-voxel `TerrainImpact` announcements using runtime `ElementId`, map-approved conjuration through `TerrainEdit::Set`, 3D volume shapes, the casting legality ladder, and deterministic `TerrainImpactOutcome` consumption; feature destruction remains deferred | combat | <!-- linear: HEX-19 owner: shravan-kumaran -->
| Persistent effects | `{source, target, payload, start, end}` in hex_core with a hex_combat runtime; rounds and enchantment-bound end conditions; `Burn` and damage-over-time become payloads | combat | <!-- linear: HEX-20 owner: shravan-kumaran -->
| Party-combat playtest checkpoint | deterministic 3v3 Party Trial summary/replay gate, focused flat ability/identity walks, and a mandatory human Crossing walk before Part 2 planning | game/docs |
| Trajectories and lingering effects | obstruction, area-unit resolution, area-lingering zones, and dispel; obstruction remains a `RunBottom`/line-of-sight satellite rather than a Wave 4 Part 1 gate | combat | <!-- linear: HEX-24 owner: shravan-kumaran -->
| Magic outside combat | general real-time casting and its input model; Rest has moved into outcomes/recovery and does not settle this deferred question | combat | <!-- linear: HEX-25 owner: shravan-kumaran -->
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

## Delivered

| Epic | Delivered |
|---|---|
| Casting UX | HEX-21 landed in Wave 3: cursor shape previews, blocked reasons, target cycling, and per-element cast presentation |
| Combat readability | HEX-23 landed in Wave 3: initiative order, detailed lattice panels, and the structured combat log |
| AI host | Wave 4 Part 1: pure request/action contracts, authoritative canonical legal actions, profile/algorithm dispatch, encounter overrides, and deterministic `baseline-v1` |
| Party controls | Wave 4 Part 1: stable six-member strip and number-key selection, camera focus, combat-owned acting selection, Group/Solo mode, and preset/member-slot editing |
| Formation traversal | Wave 4 Part 1: per-segment sextant rotation, deterministic bottleneck compression/reformation, and all-or-nothing exact-path `MoveParty` validation |
| Outcomes and recovery | Wave 4 Part 1: retained-world Victory/Defeat, exact same-seed Retry, caster-chosen Renewal restoration with next-round revival, and whole-party exploration Rest |

## Sequencing — independent lanes behind one contract

The V3 program began with a small contract PR: documentation, shared
`hex_core` vocabulary, headless tests, and a reserved `GameplaySetup::Perception`
phase. It changed no behavior or existing `hex_units`/`hex_combat` systems; test
harnesses only mirror the expanded shared setup chain. Both implementation lanes
branched from updated `dev` after that contract merged.

**Map lane:** Fort → `Ring7` → remaining recipe migration → V1/V2 removal.
The map owner keeps semantic plans private and publishes exact shared consequences.
Recipe PRs do not edit gameplay-owned crates.

**Perception lane:** fog presentation and cave lights → movement adapter →
engagement/targeting/AI adapters. `hex_perception` may observe unit positions, while
`hex_units` reads
only `LocalMapKnowledge` from `hex_core`. Every adapter that changes an owned
crate is isolated and reviewed by that owner.

Headless perception remains independent of the map lane. Forest and Fort do not
depend on combat integration. `Ring7` waits for Waterfall, Forest, and Fort semantic
plans; V3 migration waits for the composite contracts but not for final combat
tuning.

Until topology-aware rebuilding exists, V3 authored liquid voxels and every lower
voxel in their columns remain protected as one atomic semantic dependency. The
`diggable` flag still governs legacy and non-topological liquids and is not a
substitute for this policy.
Dynamic cave-breach illumination remains unresolved: terrain edits do not reclassify
an entire chamber until aperture and domain semantics are agreed.

### The gameplay lane, in waves

The gameplay side delivers in **waves**: a short-lived `wave/N-*` branch collects a
group of ticket PRs in dependency order, a human walks the integrated build once, and
the whole wave lands on `dev` in one merge (CONTRIBUTING.md has the rules).

- **Wave 3 — the slice becomes a game.** Lattices wired (the damage loop: cast,
  disables, downed state), Terrain magic, Persistent effects, Knowledge and divination,
  Encounters. `RunBottom` lands before Terrain magic starts, and Terrain magic starts
  only after the declarative impact, outcome, and conjuration-admission asks G/H/L have
  an agreed shape. Other wave work need not wait for those boundary contracts. Damage
  exists at the end of it.
- **Wave 4 Part 1 — complete party combat.** Algorithm-neutral AI hosting, party
  controls, formation traversal, outcomes, Renewal, Rest, and one integrated 3v3
  scenario through a mandatory human playtest checkpoint. Casting UX and combat
  readability already landed in Wave 3. General real-time casting, Channel,
  co-casting, initiative, action economy, and rout remain Part 2 decisions.
  Perception adapters and `RunBottom`-dependent obstruction/trajectory work are
  optional satellites, not Part 1 gates.
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

Remaining perception work follows
[the perception contract](../systems/perception.md): presentation and each
gameplay-owned adapter stay separate, while spatial observation and hidden lattice
contents remain distinct information channels.

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
