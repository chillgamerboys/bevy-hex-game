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
([architecture.md](../architecture.md#ownership-cuts-both-ways)): `map` rows
belong to the map's owner and are deliberately left unclaimed here; everything
else is the gameplay side. `docs` is whoever picks it up.

## Upcoming

| Epic | Scope | Owner |
|---|---|---|
| Deterministic sim seams | serde across the hex_core domain vocabulary; a stable `UnitId` with allocator and registry; every sim tie-break moved off entity ids; a `SimSeeds` resource | core |
| Command funnel | `GameCommand` vocabulary and queue in hex_core; one validating applier in hex_combat; click, SPACE, and the AI become emitters; a `Busy` gate replaces animation-component gating | combat |
| Combat policy knobs | `combat.ron`: engage/disengage ranges, movement budget, height bonus, and the open design questions as named policy enums that parse but reject-with-reason until built | combat |
| Elements and spells as content | `elements.ron` (wheel, opposition, fusion recipes) and `spells.ron` (closed-enum effects, targeting specs); a cross-file `ContentIndex` with load-time and test-time reference checks | assets |
| Lattice engine | new pure `hex_lattice` crate: inscription/state split, `castable()`, disable and enchantment bookkeeping, channeling; the property-test suite | combat |
| Lattices wired into the game | units spawn with archetype lattices from `lattices.ron`; a first `Cast`; damage, death, and the defender-chooses decision flow; a HUD readout of live/disabled hexes | combat |
| Encounters | `encounters/*.ron`: rosters by archetype, spawn zones, anchor placements, a formation anchor; retires the two-coordinate scenario scaffold | game |
| Save and load | versioned `SaveFile` snapshot of domain state; the terrain edit log; restore through the existing Loading flow; then mid-combat saves | game |
| Knowledge and divination seam | `FactionKnowledge` with a `view()` accessor and round-based decay; UI and AI read hostile lattices only through it | combat |
| Ship-hygiene basics | panic hook, log-to-file, version display in the title, diagnostics logging off in release | game |
| Settings menu, persistence, and audio | in-game options backed by bevy_persistent; window modes; input-map centralization; an audio facade over bevy_kira_audio with volume buses | game |
| Steam packaging and crash reporting | app icon, macOS codesign/notarize lane, Steam depot upload on the release workflow, split debug symbols, opt-in crash reporting via sentry-rust-minidump | game |
| Engine upkeep | the one budgeted Bevy 0.20 upgrade (~Q4 2026) plus the feature trim, landed together in a quiet window before any release | game |
| Named region tags | `regions:` in world files, published as `RegionTags` on tile entities — anti-magic fields, lit zones, any painted area | map |
| Run bottoms on tiles | publish `RunBottom(Level)` beside each tile's `TilePos` so line-of-sight and cover can see under bridges | map |
| Pre-spawn terrain edit replay | drain a `PendingTerrainEdits` resource after map build and before first spawn, so save-restore and authored pre-battle terrain cost zero respawns | map |
| Terrain snapshot | a name-keyed `VoxelMap` dump behind a request/response pair, making saves survive generator changes | map |

## Sequencing — the waves

Ordered by two forces: the critical path to damage existing
(**sim seams → command funnel → lattices wired**, with content and the engine
feeding in from the side), and what the map owner has in flight. PR #56
front-loads all of his shared-contract changes (the `GameplaySetup::View`
phase, `TraversalEndpoint`/`admits_transition`, `InteriorRegions`,
`MapViewHint`); the recipe PRs after it churn only `hex_map` internals — so
one merge, not his whole sequence, is the gate below.

- **Wave 1 — start now.** Lattice engine (started early — it is the long
  pole), Elements and spells, Ship-hygiene basics, and **most of**
  Deterministic sim seams (first: smallest, and it unblocks the funnel and
  saves). These live in new modules and in files #56 does not hold; the
  hex_core additions they make (`lattice_ids`, `ElementId`, `UnitId`) are
  new lines that merge cleanly past #56's `lib.rs` edits. Two slivers of
  the seams are the exception and wait for the gate: the serde derives on
  `Turn` and `Body` each edit one line of a file #56 is holding
  (`app.rs`, `movement.rs`).
- **Wave 2 — after #56 merges.** Combat policy knobs — its height-bonus
  parameterization rewrites the elevation tests, which #56 is extending
  right now — and Command funnel, which is sequenced here by dependency
  (it needs the seams' `UnitId`) and lands more cleanly once `app.rs` is
  quiet. `targeting.rs` is contested too, but by our own knobs ticket, not
  by #56 — the funnel and knobs should not run concurrently with each
  other in that file.
- **Wave 3 — the slice becomes a game.** Lattices wired (needs content +
  engine + funnel), Knowledge seam (needs the engine only), Save and load
  (needs the seams; opens the terrain-snapshot conversation below),
  Encounters (after the first V2 recipe lands, since it consumes anchors
  and `scenarios.rs`).
- **Wave 4 — productization, latest before the first external build.**
  Settings/persistence/audio, Steam packaging and crash reporting, Engine
  upkeep (pinned to the 0.20 release window).

Hard dependencies, for reference: sim seams → {funnel, saves}; elements →
engine wiring; engine → {lattices wired, knowledge}; funnel → lattices
wired.

**Standing toe-stepping rules.** Never touch `crates/hex_map/**`. While
#56 is open it holds, on the gameplay side: `hex_core/{app,lib,terrain,traversal}.rs`,
`hex_units/movement.rs` (+ its integration tests), `hex_combat/tests/elevation.rs`,
`hex_world/camera.rs`, `hex_game/{main,review,scenarios}.rs`. The working
rule: **adding new lines** to those files (a fresh `pub mod`, a new enum, a
new test fn) merges cleanly and is fine; **editing lines that exist** is
what hands the other person a rebase conflict — defer those edits until it
lands. System-ordering note that ages with the gate: `dev` today has the
four-phase `GameplaySetup`; #56 adds `View` between `Actors` and
`Finalize`, so anything written against the five-phase set compiles only
after it merges.

The `map` rows are specified precisely, with fallbacks if deferred, in
[map-asks.md](map-asks.md) — two further asks (the seed contract and
generator versioning) were answered outright by the procedural map pipeline
and are recorded there as settled. **The map rows are seeded and claimed by
the map's owner**, at his own pace around the V2 recipe sequence; the
gameplay side never marks them. The one with a gameplay-side clock is the
terrain snapshot: it decides the save format, so it wants a conversation
when Save and load starts — the seeded-regen fallback keeps that ticket
unblocked either way.

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

Two derives deliberately trail the rest: `Turn` and `Body` live on lines
inside files the open #56 is editing (`app.rs`, `movement.rs`), so their
serde attributes land as a follow-up commit once it merges — see the
toe-stepping rules above. Nothing downstream needs them before the save
work starts.

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
([map-asks.md](map-asks.md), ask D2) — which is the generator-change-proof
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
quiet window and one visual walk. Not before wave 3 lands — upgrading under
an in-flight system rewrite doubles the blast radius.

### The map rows

Specified in [map-asks.md](map-asks.md), each with exact signatures,
publisher/consumer, tests, and a fallback if deferred — nothing on the
gameplay side blocks on them.

### Where the rest of the documentation lives

The kind-separated docs tree — [the index](../README.md), `systems/`,
`design/`, `development/`, and this directory — was reorganised alongside
these planning docs. [status.md](status.md) is the one doc allowed to drift;
everything outside `planning/` describes contracts.
