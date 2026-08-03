# Roadmap

The remaining epics between the current build and a shippable game. The table under
**Upcoming** is the one ticket-seeding workflow can turn into Linear `HEX-*` tickets.
What is already built lives in [status.md](status.md); release history lives in the
root changelog. Where each epic came from, and the
evidence behind it, is [production-audit.md](production-audit.md) — a dated
snapshot that does not change. This file is the living plan: rows get claimed,
split, and retired.

**How this file works.** Each unmarked row of the Upcoming table becomes one Linear
ticket when someone runs the repository's ticket-seeding workflow (Claude exposes it
as `/seed-tickets`), which writes an HTML-comment marker back onto the row recording
the ticket and who claimed it. Never write those
markers by hand, and never take a row someone else's marker claims. Rows are
epics, not implementation plans. Planning starts by reconciling the live delivery
state: Codex uses `$reconcile-delivery-state` before `$plan-parallel-work`, while
Claude's `/plan-ticket` produces its plan from the per-epic sections below. Splitting
a row later is safe: seeding is create-only and idempotent.

The `Owner` column names the crate-ownership area
([architecture.md](../architecture.md#ownership-cuts-both-ways)). `map` rows
belong to the map's owner; `units` and `combat` rows belong to the gameplay
owner. `perception` is the new headless visibility boundary, but its adapters
still belong to the crate they change. `docs` is whoever picks it up.

## Upcoming

| Epic | Scope | Owner |
|---|---|---|
| Terrain magic | **Partial / In Progress:** permanent `Single`/`Column` stone evocation, the #175/#178 world resolver, answer schema, and ordering foundation, and #180's explicit Impact/Fireball content, paid monotonic `TerrainImpact` emission, matching-outcome consumption, fresh occupancy/movement ordering, deterministic unsupported-actor settlement, authority adoption, and typed frozen failure are live. Remaining HEX-19 work keeps authored Interior membership with no dynamic cave daylight and defers area construction, enchantment-bound terrain, fluid dynamics, feature destruction, and terrain save/restore persistence | combat | <!-- linear: HEX-19 owner: shravan-kumaran -->
| Trajectories and lingering effects | **Partial / In Progress:** #162's exact `Direct`/`Arc`/`None` material trajectories share one symmetric integer supercover; faction-facing preview, cycling, and AI use authorized knowledge while command authority retains full occupancy. #180 adds live radial per-voxel clipping and stable friendly-fire area Disable/Burn behind one held resolution transaction. Remaining HEX-24 work is area Restore/Reveal policy, area-lingering zones, dispel, and later sight reuse | combat | <!-- linear: HEX-24 owner: shravan-kumaran -->
| Magic outside combat | general real-time casting and its input model; Rest has moved into outcomes/recovery and does not settle this deferred question | combat | <!-- linear: HEX-25 owner: shravan-kumaran -->
| Co-casting and rituals | variable-mana group casting after Wave 7 supplies a real Channel action and evidence for initiative and action economy | combat | <!-- linear: HEX-26 owner: shravan-kumaran -->
| Engine upkeep | the one budgeted Bevy 0.20 upgrade (~Q4 2026) plus the feature trim, landed together in a quiet window before any release | game | <!-- linear: HEX-18 owner: shravan-kumaran -->
| Perception presentation | faction fog, remembered rendering, picking gates, and composition with explicit review-roof visibility and whole-tree opacity | perception |
| Scenario camera policy | decide when Map mode is appropriate and limit its availability by scenario now that the human-approved Character view is live | world / presentation |
| Remaining movement and combat perception adapters | unknown-route restriction; detection, engagement, ordinary-attack targeting, and one-round last-known-position behavior in isolated owner-reviewed PRs; AI and casting anchors are already live | units/combat |
| V1/V2 legacy removal | remove the frozen V1/V2 parsers, generators, assets, and runtime tests now that every active shipped procedural scenario resolves through V3 | map |
| Named rule regions | revisit a content-addressable exact-surface overlay when the first region-sensitive spell lands; do not combine biome identity, lighting, and anti-magic into generic tile tags | map/combat |
| Pre-spawn terrain edit replay | drain a `PendingTerrainEdits` resource after map build and before first spawn, so save-restore and authored pre-battle terrain cost zero respawns | map |
| Terrain snapshot | a name-keyed `VoxelMap` dump behind a request/response pair, making saves survive generator changes | map |

## Delivered

| Epic | Delivered |
|---|---|
| Run bottoms on tiles | Every material-run entity publishes exact inclusive integer bounds through `RunBottom` and `TilePos`, including stacked platform/cave runs and terrain-edit rebuilds |
| Casting UX | HEX-21 landed in Wave 3: cursor shape previews, blocked reasons, target cycling, and per-element cast presentation |
| Combat readability | HEX-23 landed in Wave 3: initiative order, detailed lattice panels, and the structured combat log |
| AI host | Wave 4: pure request/action contracts, authoritative canonical legal actions, profile/algorithm dispatch, encounter overrides, and deterministic `baseline-v1` |
| Knowledge-safe AI and casting | Foundation hardening: faction-authorized AI identities, terrain, traversal, turn/effect fields and legal commands; Observed-only cast anchors from preview through authoritative application |
| Persistent effects | HEX-20: `{source, target, payload, start, end}` vocabulary and combat runtime, including personal-turn Burn and enchantment-bound expiry <!-- linear: HEX-20 owner: shravan-kumaran --> |
| Party controls | Stable six-member roster and combat-owned acting selection; Party/Initiative activation is presentation-only inspection, while typed Formation Main View owns explicit movement-member selection, Group/Solo mode, and preset/member-slot editing |
| Formation traversal | Wave 4: per-segment sextant rotation, deterministic bottleneck compression/reformation, and all-or-nothing exact-path `MoveParty` validation |
| Outcomes and recovery | Wave 4: retained-world Victory/Defeat, exact same-seed Retry, caster-chosen Renewal restoration with next-round revival, and whole-party exploration Rest |
| Party-combat checkpoint | Wave 4: deterministic 3v3 Party Trial summary/replay, focused Ability Lab and Raider Mirror walks, and the completed human Crossing playtest |
| Campaign/Sandbox app shell | Main Menu with exactly Campaign, Sandbox, Tools, and Settings; three indexed Campaign slots; one persistent in-memory six-slot-per-side Sandbox; typed Creator destinations; Party Trial remains the canonical new Campaign |
| Campaign persistence | Three atomic, build/content-bound indexed records, safe manual saves from Campaign exploration, active-play time, invalid-record preservation, and one-time non-destructive import of the Wave 5 resume with semantic-drift records retained as Invalid <!-- linear: HEX-15 owner: shravan-kumaran --> |
| Settings and seams | Persistent display, semantic UI scale, volume, ordinary HUD visibility, and categorized keyboard overrides with explicit capture/conflict/restore behavior; empty music/SFX/UI buses remain ready for later content <!-- linear: HEX-16 owner: shravan-kumaran --> |
| Release artifact scaffold | Wave 5 / HEX-17: stable app identity, normalized packages, retained symbol material, and documented future credential slots with no live integrations <!-- linear: HEX-17 owner: shravan-kumaran --> |
| Creator foundation (Wave 6; player-facing organization superseded) | Versioned saved character/spell blueprints, immutable templates, Creator-local lattice tests, Map readiness, and frozen custom-content launches retained under typed Tools/Sandbox routing |
| Tactical integrity (Wave 7; player-facing organization superseded) | Exact-surface occupancy, Channel, deterministic summaries/simulations, and the measured decision to retain the shipped four-hex movement default remain; old tuning/report UI does not |
| Gameplay foundation and scoped validation | Wave 8: one pure serializable combat authority projected through ECS/animation/UI, renderer-free gameplay screen models, concern-specific integration targets, and fail-closed dependency-scoped validation with unchanged broad owner gates <!-- linear: HEX-28 owner: shravan-kumaran --> |
| Unified map validation | Map unit, deterministic generation, and real-plugin publication contracts now share the repository scope selector, one explicit integration target, optimized dependency execution, per-concern timing/JUnit evidence, and unchanged PR/stress/visual acceptance |
| V3 active recipe migration | Hills, Sky Islands, Mountains, Caves, Waterfall, Forest, Fort, Volcano, Deep Forest, Prairie, Shallow Sea, Beach, Shore, and Deep Mountain all use the V3 semantic pipeline in shipped scenarios |
| Forest authored objects | Forest publishes rotated vegetation `ObjectInstance`s and exact blockers; `hex_objects` propagates whole-tree camera roots and retains authored canopy masks as separate art metadata |
| Structures and Fort | V3 Fort ships worked-stone walls, towers, gates, keep, wall walks, stairs, battlements, and validated defensive circulation |
| Seven-region composition | Ring7 composes all seven V3 recipe variants in one connected radius-33 world with global routes, elevation seams, and hydrology |
| Expanded biome set | Volcano, Deep Forest, and Prairie add distinct crater/lava, full-woodland, and open-grassland recipes while stable scenario and object identities remain compatible |
| Nineteen-region composition | Ring19 composes the selectable radius-55 Two Rings map from 19 fixed logical regions, 42 physically redundant seams, one mountain-fed confluence/outlet water graph, and a separate volcano lava outlet |
| Macro Mountain Range | The selectable radius-77 / 18,019-column world composes 37 atomic cells into 30 logical biome instances with Macro-only adjacency validation, coastal and alpine recipes, one joined watershed, graded mountain tiers, a five-cell massif, generated framing, and a validated Shore-to-massif-base route |
| Cave lighting and presentation | V3 Caves publishes deterministic gameplay lights over required routes plus authored emissive crystals and restrained presentation-only physical lights |
| Character-first camera | Player-authoritative full-range look, radius-only terrain collision with stable recovery, near-character occlusion, whole-tree fading, ordinary opaque cave roofs, seed-exact multi-azimuth traversal over every selectable map and Two Rings region, and Alberto's 2026-08-01 native motion/readability approval |

## Sequencing — independent lanes behind one contract

The V3 program began with a small contract PR: documentation, shared
`hex_core` vocabulary, headless tests, and a reserved `GameplaySetup::Perception`
phase. It changed no behavior or existing `hex_units`/`hex_combat` systems; test
harnesses only mirror the expanded shared setup chain. Both implementation lanes
branched from updated `dev` after that contract merged.

**Map lane:** every active shipped procedural scenario now uses V3; `Ring7`, `Ring19`,
and `Macro` are live, and Two Rings and Mountain Range are selectable without
replacing Seven Regions. Mountain Range adds the authored coast-to-massif scenario
without changing the legacy layouts or their fingerprints.
`RunBottom` publication is live; frozen V1/V2 removal remains independent cleanup.
The map owner keeps semantic plans private and publishes exact shared consequences.
Recipe PRs do not edit gameplay-owned crates. Two Rings received the wave's final
human visual and play approval before landing.

**Perception lane:** knowledge-safe casting and AI plus cave gameplay-light and
physical presentation are live. Fog presentation → movement adapter →
engagement/ordinary-targeting/lost-contact adapters remain.
`hex_perception` may observe unit positions, while `hex_units` reads only
`LocalMapKnowledge` from `hex_core`. Every adapter that changes an owned crate is
isolated and reviewed by that owner.

Headless perception remains independent of the map lane. The completed biome,
vegetation, Ring7, Ring19, and Macro work does not depend on combat integration, and
Wave 7 does not reopen their private semantic plans.

Until topology-aware rebuilding exists, V3 authored liquid voxels and every lower
voxel in their columns remain protected as one atomic semantic dependency. The
`diggable` flag still governs legacy and non-topological liquids and is not a
substitute for this policy.
The initial cave-breach ruling deliberately keeps authored Interior membership after a
roof edit, so the chamber does not gain dynamic daylight. A later topology-aware model
must separately define aperture and domain semantics before reclassification is safe.

### The gameplay lane, in waves

The gameplay side delivers in **waves**: a short-lived `wave/N-*` branch collects a
group of ticket PRs in dependency order, the integrated candidate runs its
selector-chosen gate, and the whole wave lands on `dev` in one merge. Affected
presentation or experience gets one visual/human route; a logic-only wave uses
exact-head hook-backed evidence (CONTRIBUTING.md has the rules).

> **Historical organization:** The Wave 5–8 bullets and Wave 7/8 topology records
> below describe what those merged waves established at the time. The current
> Campaign/Sandbox cutover supersedes their title, resume, standalone browsing, Lab,
> fixture, rule-profile, statistics, and report UI claims. Their gameplay authority,
> exact occupancy, creation persistence, frozen-launch, simulation, and test-boundary
> provenance remains valid.

#### Historical Waves 3–8 delivery record (player-facing routes superseded)

- **Wave 3 — the slice becomes a game (delivered).** Lattices wired the damage loop
  from casts through disables and downing, alongside persistent effects, knowledge and
  divination, and authored encounters. Later work now consumes the live `RunBottom`
  projection for permanent construction and obstruction-aware trajectories, and the
  world side of terrain durability is live. Gameplay's elemental declaration, outcome
  consumption, refreshed occupancy/movement, and unsupported-actor settlement landed
  later in #180 rather than forming part of the original Wave 3 claim.
- **Wave 4 — complete party combat (delivered).** Algorithm-neutral AI hosting, party
  controls, formation traversal, outcomes, Renewal, Rest, and one integrated 3v3
  scenario through a mandatory human playtest checkpoint. Casting UX and combat
  readability already landed in Wave 3. General real-time casting, Channel,
  co-casting, initiative, action economy, and rout remain future gameplay decisions.
  Perception adapters and `RunBottom`-dependent obstruction/trajectory work were
  optional satellites, not retroactive Wave 4 gates. `RunBottom`-backed construction
  and trajectories subsequently landed; the remaining perception adapters stay
  Upcoming.
- **Wave 5 — pre-alpha continuity (delivered).** A stable app shell and default New Game,
  one disposable exploration-resume slot, persistent settings and audio/input seams,
  and release-artifact scaffolding. This wave gets ahead of productization without
  promising save compatibility or live storefront, signing, telemetry, or crash
  reporting. Engine upkeep remains parked for the Bevy 0.20 window and is not a Wave 5
  gate.
- **Wave 6 — creator and combat lab (delivered).** The primary title routes now expose
  separate Character Creator and Spell Creator workspaces plus Combat Lab. Local records have
  stable IDs, atomic persistence, Draft/Ready and Map-ready diagnostics,
  dependency-safe deletion, and immutable packaged templates. Sandbox builds ordered
  rosters on all seventeen distinct supported shipped maps, previews and describes
  each choice, resolves deployment, freezes content for Retry, and refuses resume
  writes. Fixed automated scenarios live behind one searchable stable-ID selector.
- **Wave 7 — tactical integrity and tempo (delivered).** Combat Lab is now the
  interactive rules-testing workspace. Exact `TilePos` occupancy makes positioning
  real, Channel closes the lattice resource loop, and frozen profiles, live telemetry,
  fixtures, and comparable reports make manually played action-economy trials
  reproducible. The renderer-free simulation target now consumes exact scripts or a
  deterministic baseline controller and proves supported casts, restoration,
  persistent effects, downing/revival, outcomes, and explicit command/turn/no-progress
  bounds. It remains regression evidence rather than a balance oracle. Versioned
  policy and report projections mirror shipping authority inputs, migrate v1 reports,
  record manual/bounded stops, and support annotated side-by-side experiments. The
  evidence gate retained the shipped
  four-hex movement default; the measurements and caveats are recorded in the
  [tempo decision audit](../development/wave-7-tempo-decision.md). Flat deterministic
  one-slot initiative remains the baseline; boss initiative, co-casting, rout,
  terrain magic, perception adapters, and campaign persistence remain outside the
  wave.
- **Wave 8 — gameplay foundation and scoped validation (delivered).** The production
  audit's organizing move is complete: one pure, integer-valued, serializable combat
  authority driven by validated commands, with ECS, animation, and UI as projections.
  Pure Combat Lab, Creator, report, and launch/retry models are split from Bevy
  wiring; expensive gameplay contracts are consolidated; and local/PR validation
  selects exact Cargo packages, targets, and features from one fail-closed concern
  map. Broad owner corpora remain unchanged and continue on their owning changes,
  `dev` pushes, schedules, and combined wave/release gates.

#### Wave 8 outcome and topology (historical; player-facing routes superseded)

Wave 8 is one gameplay/shared-integration wave rather than independent refactors.
The pure combat authority, ECS and animation adapters, gameplay screen models, and
test selector all change which layer owns truth and which dependency graph each test
must compile. No leaf is a release candidate on its own, and only the combined state
can prove that old gameplay behavior, app lifecycle, and scoped validation agree.

Its source PRs targeted `wave/8-gameplay-foundation` in this order:

1. **Test-scope foundation.** Record cold and warm compile/link/test baselines; add
   one machine-readable, fail-closed change-to-concern map; correct nextest commands
   so Cargo package/target/feature selection happens before test filtering; and emit
   shadow-mode scope decisions without skipping the existing full PR suite.
2. **Pure combat authority.** Introduce one serializable combat state and validated
   command reducer over frozen arena, roster, rules-profile, and observation inputs.
   Run it in temporary shadow against canonical Wave 7 snapshots; do not preserve a
   second simulator after equivalence.
3. **ECS and animation cutover.** Make ECS resources/components and animation consume
   projections from the gameplay authority. Domain movement/busy state, not
   `Transformation` presence or frame settling, governs legality and turn advance.
4. **Gameplay screen models.** Extract pure Combat Lab, Creator, report, comparison,
   Retry Exact, Tune & Run Again, fixture copy, and re-entry transitions from Bevy UI.
   Keep a small immutable headless app observation target for wiring and lifecycle.
5. **Test and app seam consolidation.** Collapse units tests into one contracts
   binary and combat tests into one contracts binary plus simulation. Separate
   gameplay-owned app behavior from the unchanged shared scenario/loading and
   map/world contract targets, proving old/new selection equivalence before removing
   duplication.
6. **Scope cutover and wave gate.** Enable required scoped PR jobs only after shadow
   evidence shows no missed failures; keep the full residual suite on `dev`; update
   local skills, docs, and PR evidence; then run the complete final wave checks,
   bounded presentation walk, adversarial diff review, and exact-head human sign-off.

Wave 8 does not tune balance, add mechanics or content, upgrade Bevy, or modify
`hex_map`, `hex_world`, `hex_perception`, `hex_game/src/review.rs`, map visual walks,
map/perception stress suites, or their acceptance criteria.

#### Wave 7 outcome and topology (historical; player-facing routes superseded)

Wave 7 landed as one gameplay wave because occupancy and Channel both change canonical
legal actions and AI, while the Lab UI, fixtures, reports, Retry, and automation prove
only their combined behavior. Its lanes entered the integration branch in this order:

1. **Measurement and profile foundation (`hex_combat`, `hex_assets`, `hex_game`).**
   Extend the gameplay-owned serializable combat summary, define a validated
   session-local rules profile, and let the shared game layer combine them with the
   frozen launch snapshot into one deterministic Lab report. Every report names its
   profile, map and seed, frozen content revision, roster order, resolved deployment,
   fixture or Sandbox origin, outcome, and summary fingerprint.
2. **Exact unit occupancy (`hex_units`, `hex_combat`).** One `TilePos` occupancy
   projection drives movement preview, path construction, command validation, party
   formation, AI legal actions, and deployment. Units may neither share an endpoint
   nor route through another body. Preview and authoritative refusal always agree.
3. **Channel action (`hex_lattice`, `hex_combat`, AI).** An active, non-downed unit
   without an open modal decision may spend its action to recover mana using the
   lattice's per-element Channelling values. Channel never repairs disabled cells,
   bypasses an enchantment lock, or grants a second action.
4. **Combat Lab tuning workspace (`hex_game`).** Sandbox becomes
   `Map → Rosters → Rules → Deploy`. Deployment roster rows become directly selectable
   for repositioning. A focused dashboard, outcome report, comparison view, and
   fixture-to-Sandbox copy flow share the same report projection.
5. **Fixtures and decision gate.** Immutable Occupancy Matrix, Channel Attrition, and
   Tempo Matrix fixtures run the same frozen setups across the shipped baseline, a
   two-step tactical profile, and bounded custom profiles. The resulting
   [tempo decision audit](../development/wave-7-tempo-decision.md) retains the shipped
   movement/action policy.

The Rules panel exposes only already-understood numeric seams:
movement per turn, strike disables, default initiative, engage range, disengage
margin, levels per bonus range, and Reveal duration. It also serializes the typed
fixed-initiative, movement-plus-one-action, burst-only channelling, and
fight-to-the-end policies without offering unbuilt variants. It offers **Shipped**, **Tactical two-step**, and
validated **Custom** profiles, shows every deviation from shipped values, and resets
without editing `combat.ron`. Initiative algorithms, multiple actions, Channel cost,
rout policy, and co-casting are not disguised as numeric switches before those
behaviors exist. The chosen profile is frozen into launch and Retry.

The Rules step presents preset cards above labelled steppers, a plain-language
description of each parameter's effect, a visible changed-from-shipped state, Reset,
and validation at the point of editing. Active Lab sessions add a collapsible
statistics drawer below the persistent lattice readout without replacing the
ordinary gameplay HUD. The Inspector is its single scroll owner at every scale;
statistics never appear without that lattice, and HUD/terminal transitions hide the
two together. Outcomes open a
full-screen report with **Overview**, **Units**, **Spells & Effects**, **Timeline**,
and **Compare** modes. Comparison shows both frozen profile/roster headers and
labelled numeric deltas; colour is never the only indication of improvement or
regression. Outcome actions distinguish **Retry Exact** from **Tune & Run Again**:
the former reuses the frozen launch unchanged, while the latter returns to Rules with
the same map, rosters, and still-valid resolved deployment before creating a new
report.

The live dashboard and post-combat report provide totals and per-unit breakdowns for
rounds and turns; movement distance and budget use; successful and refused commands;
casts by spell and delivered effect; Channel actions and mana restored by element;
strikes; raw, prevented, and applied disables; restorations, downings, and revivals;
idle turns; no-progress stretches; AI selections; and the final outcome. Reports are
versioned, bounded, explicitly saved local Lab data, separate from Creator records and
Continue. A user can compare two saved reports or delete them. Fixed fixtures never
read that local history, but their result can be copied into Sandbox with the exact
map, roster, placement, and profile for controlled experimentation.

Combined acceptance runs human and baseline-AI occupancy chokepoints, Channel under
attrition and enchantment locks, every numeric profile boundary, direct deployment
repositioning, fixture copying, report comparison/deletion, Retry, exit/re-entry, and
Creator-origin return routing. Automated fixture walks launch by stable ID and emit
the same deterministic report schema used by the UI. The final human pass compares
the shipped and two-step tempo profiles on Party Trial and at least one full six-unit
Sandbox roster before the default action economy changes.

The Wave 5 resume slot deliberately uses explicit seeded regeneration and refuses
generator/content drift. It is a development convenience, not the production save
format. Generator-independent terrain snapshots (boundary ask D2) and pre-spawn edit
replay (D1) remain prerequisites for durable saves, but do not block this scaffold.

The casting contract those waves implement — the announce model, the legality ladder,
volumes, and persistent effects — is [casting.md](../systems/casting.md).

The complete V3 map contract, fixed Ring7 and Ring19 rosters, authored Mountain Range
Macro composition, fingerprint policy, recipe stages, and removal gate live in
[world-generation-v3.md](../systems/world-generation-v3.md). Publication asks and
fallbacks in both directions are [boundary.md](boundary.md); what crosses the boundary
today, and its status, is [contracts.md](../contracts.md).

## The epics, in detail

### Pre-alpha app shell and default game

The responsive Main Menu keeps exactly Campaign, Sandbox, Tools, and Settings as its
primary routes. Campaign always shows three cards. Party Trial is the canonical new
Campaign and is bound to the selected empty slot. Sandbox owns the player-facing map
catalog, pending/committed generated seed, two ordered six-slot rosters, character
selection, deployment, and frozen launch. Tools owns the two Creators and a disabled
Map Creator marked Coming Soon.

Scenario definitions remain internal launch inputs. Ability Lab, Raider Mirror, and
other stable deterministic cases are available only through default-off test support.
Close Quarters and category-driven player navigation remain retired.

### Save and load

The app ships one hand-shaped, versioned, atomic `campaigns.ron` through
`crates/hex_game/src/save.rs` — domain state, not ECS reflection. It contains exactly
three explicit indexed records. Safe manual saving is limited to paused, quiescent
Campaign exploration and overwrites only the bound slot. Each occupied record stores
scenario/build/content/generator identity, party state, selected unit, formation, and
accumulated active-play milliseconds. Restore rides the existing Loading flow.
Corrupt or incompatible records are preserved and refused visibly rather than
partially loaded or treated as empty.

If `campaigns.ron` is absent, a valid legacy `resume.ron` is copied into slot 1 with
zero prior play time; invalid legacy data and its reason remain visible. The legacy
file is never overwritten or deleted. Combat state, durable cross-version
compatibility, deletion/overwrite UI, and a terrain edit log remain outside this
scaffold.

### Settings menu, persistence, and audio

The pre-alpha options surface persists display/window presentation, music/SFX/UI
volume values, ordinary HUD visibility, and keyboard overrides across sessions.
Keybindings are grouped into Gameplay, Interface, Main View, Camera, and System;
capture consumes the next non-modifier key, overlapping conflicts require explicit
Swap or Cancel, each row can restore its default, and Restore All requires
confirmation. Input actions remain centralized so gameplay systems do not own raw
keys. Audio sits behind music/SFX/UI buses ready for later content; Wave 5 did not
ship audio. The frozen production audit remains the research record, not a
requirement to adopt every integration now.

### Steam packaging and crash reporting

Wave 5 builds use an app identity and icon, normalized release artifact names and
layout, and retained debug symbols. Release documentation reserves the future
credential and configuration slots for signing, Steam upload, and crash reporting.
Live integrations, codesigning, notarization, upload, consent UI, and telemetry remain
later productization work.

### Engine upkeep

This is explicitly outside Wave 5. The audit budgets exactly one Bevy upgrade before
any release window: 0.20
(~Q4 2026, BSN asset files and assets-as-entities are the churn to watch),
landed together with the long-deferred feature trim (`default-features =
false` plus the collections actually used) so both risky changes share one
quiet window and one visual walk. Not while the command, perception, or V3
foundation contracts are moving — upgrading under an in-flight system rewrite
doubles the blast radius.

### V3 world program

V3 replaces the recipe-per-map assumption with a patch-capable semantic world plan.
`Single` keeps focused recipe iteration fast; Ring7 composes a central Hills region
and six fixed outer recipes inside one radius-33 footprint. Ring19 composes a centre,
six first-ring regions, and twelve second-ring regions inside one radius-55
footprint. Shared edges, routes, elevation datums, and hydrology are resolved before
patch interiors, so the system never tries to disguise incompatible maps with a
material blend.

The delivered Mountain Range map uses an authored `Macro` path: 37 atomic cells
cover one radius-77 footprint, while 30 named logical biome instances own connected
cell unions and receive one recipe invocation and region id each. Macro-only
allowed-by-default adjacency rules reject the initial coastal, Deep Mountain, and
Frozen–Volcanic mismatches. Standing-water joins keep the shared coast and sea still,
while directed tributaries remain explicit. Mountain Range uses that foundation for
coastal recipes, graded Alpine tiers, one five-cell massif, and a declared central
Shore-to-massif-base land route. Procedural placement, deep water, swimming, boats,
and playable terrain behind or atop the massif remain outside this milestone.

Waterfall establishes the liquid layer, Forest establishes surface features and
exact blockers, and Fort establishes structures and circulation. Deep Forest and
Prairie reuse one vegetation authority at opposite density extremes, while Volcano
owns the separate lava topology. Those recipes feed Ring7 and Ring19, while the
coastal and alpine recipes feed Macro; every active shipped recipe now uses the same
V3 semantic pipeline.
V1/V2 remain frozen development oracles only until replacement review passes;
they are removed rather than maintained as permanent compatibility paths.
The decision-complete contract is
[world-generation-v3.md](../systems/world-generation-v3.md).

### Spatial perception

Remaining perception work follows
[the perception contract](../systems/perception.md): presentation and each remaining
gameplay-owned adapter stay separate, while spatial observation and hidden lattice
contents remain distinct information channels. Casting and AI already use the live
faction authority; fog/picking, unknown-frontier movement, engagement, ordinary
attacks, and lost-contact search do not.

### The map rows

Specified in [boundary.md](boundary.md), each with exact signatures,
publisher/consumer, tests, and a fallback if deferred. Most gameplay work can proceed
independently. Exact run bounds are now published; terrain casting can consume them
alongside the accepted impact/outcome/conjuration contracts rather than reconstructing
world facts.

### Where the rest of the documentation lives

The kind-separated docs tree — [the index](../README.md), `systems/`,
`design/`, `development/`, and this directory — was reorganised alongside
these planning docs. [status.md](status.md) is the one doc allowed to drift;
everything outside `planning/` describes contracts.
