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
([ARCHITECTURE.md](../ARCHITECTURE.md#ownership-cuts-both-ways)): `map` rows
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
| Production hygiene | panic hook, log-to-file, version display, release diagnostics off, settings menu with persistence, audio facade, input map, app icon, signing and Steam depot lane, crash reporting | game |
| Named region tags | `regions:` in world files, published as `RegionTags` on tile entities — anti-magic fields, lit zones, any painted area | map |
| Run bottoms on tiles | publish `RunBottom(Level)` beside each tile's `TilePos` so line-of-sight and cover can see under bridges | map |
| Pre-spawn terrain edit replay | drain a `PendingTerrainEdits` resource after map build and before first spawn, so save-restore and authored pre-battle terrain cost zero respawns | map |
| Terrain snapshot | a name-keyed `VoxelMap` dump behind a request/response pair, making saves survive generator changes | map |
| Docs restructure | execute the kind-separated docs-tree spec: moves, splits, index, status doc, troubleshooting merge, skill repointing | docs |

## Sequencing

Genuine blockers only — everything else parallelizes:

- **Deterministic sim seams** → Command funnel → {Lattices wired, mid-combat saves}
- **Deterministic sim seams** → Save and load
- **Elements and spells** → Lattice engine wiring; **Lattice engine** → {archetypes in Lattices wired, Knowledge seam}
- **Docs restructure** is blocked on PR #52 merging (it moves five files #52 edits).
- Independent picks needing no funnel knowledge: Combat policy knobs, Elements
  and spells, Production hygiene items, and every `map` row.

The `map` rows are specified precisely, with fallbacks if deferred, in
[map-asks.md](map-asks.md) — two of the original six asks were already
delivered by PR #52 (seeds and generator versioning).

## The epics, in detail

### Deterministic sim seams

The cheapest-now, brutal-later foundations for saves, replays, and future
co-op. Four small PRs: serde derives on the hex_core vocabulary (`HexCoord`
via its constructor invariant, `TilePos`, `SubstanceId`, `TerrainEdit`,
`Turn`) plus `Body`/`Faction`, with round-trip tests and the `CubeCoord`
dedup; a `UnitId(u64)` component with a saved allocator and an
entity-registry, allocation in scenario spawn order; `TurnOrder` keyed by
`UnitId` with ties broken initiative-then-id (today's entity-index tie-break
is not stable across runs or saves); AI target and selection tie-breaks moved
to `UnitId`; a `SimSeeds` resource (world / ai-flavor / cosmetic — resolution
itself takes no RNG, by signature). Also `PlayerSeat`/`ControlOwner` (seat 0
everywhere today) and a `Party` roster resource — one field each, and they are
the entire future co-op ownership model.

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
design questions from [DESIGN.md](../DESIGN.md#open-questions) as policy
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

### Production hygiene

The commercial checklist, all independently landable: panic hook +
log-to-file (Windows release currently logs nowhere) + version display;
diagnostics logging off in release; settings menu backed by bevy_persistent
with window modes and volume placeholders; audio behind a small facade over
bevy_kira_audio (trim the unused bevy_audio feature then); input map
centralization (rebinding-ready); app icon; macOS codesign/notarize lane and
a Steam depot upload job on the existing release workflow; opt-in crash
reporting via sentry-rust-minidump with split debug symbols. Versions and
sources for every crate choice are in
[production-audit.md](production-audit.md).

### The map rows

Specified in [map-asks.md](map-asks.md), each with exact signatures,
publisher/consumer, tests, and a fallback if deferred — nothing on the
gameplay side blocks on them.

### Docs restructure

Execute the kind-separated docs-tree spec (index, `systems/`, `design/`,
`development/`, `planning/`, single-source troubleshooting and status docs,
skill repointing). Hard-blocked on PR #52, which edits five of the files it
moves. The spec lives outside the repo (it was PR #54, merged and then
reverted to keep `dev` clean while #52 is in flight); the plan for executing
it, including the amendments this file's existence introduces, is recorded
with the audit work.
