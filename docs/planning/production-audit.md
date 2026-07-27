# Production-readiness audit — July 2026

> **A dated snapshot, deliberately frozen.** Everything below was true of
> `dev` at `afe9b79` (plus the then-open PR #52) on **2026-07-26**, and is
> not updated as the code moves — that is what makes it citable. The living
> plan derived from it is [roadmap.md](roadmap.md); the cross-owner asks are
> [boundary.md](boundary.md) (called `map-asks.md` when this was written).
> Mechanical doc tooling must never edit this file.

**Question asked:** what has to change, architecturally and operationally,
for this codebase to become the commercial game
[the design](../design/game.md) describes — a Steam release with BG3-style
co-op planned later?

**Scope agreed with both constraints in force:** multiplayer is
planned-but-later, so the audit recommends cheap seams now (command-driven,
deterministic, serializable simulation) and no netcode; the bar is a
commercial Steam release (saves, settings, audio, packaging, crash
handling); the map crate is its owner's — map-side needs are written asks
with fallbacks, never code from this side; and the design's open questions
(initiative, action economy, permadeath, boss slots, rout, fight length,
element list, map shape) must **stay open**, expressed as data-driven policy
knobs rather than settled by architecture.

**Method:** three parallel repo explorations (contracts, runtime flow,
infrastructure), first-hand reads of `hex_core` and every doc, two
independently-produced architecture designs (simulation core; content
pipeline and map asks) reconciled where they differed, ecosystem research
with crate versions verified against crates.io/GitHub as of July 2026, and a
full multi-agent review of PR #52 (posted on the PR).

---

## Verdict

**A genuinely production-grade skeleton with the entire game still ahead of
it.** The boundaries, tests, lints, and pipelines are better than most
shipped indie code. But the system the design calls the game — the lattice —
does not exist; nothing is serializable; input mutates the world directly
with no seam for replay, saves, or a second player; and turn order is not
reproducible across runs. The design's own choices (integer positions, flat
disable counts, no resolution randomness) make a deterministic, serializable
core unusually cheap **if it is built in now** rather than retrofitted under
a shipped save format.

## What is strong (keep, and build on)

- **Cargo-enforced crate boundaries.** `hex_map`/`hex_world`/`hex_units`
  cannot import each other; the map reaches gameplay only through six
  components and one message. `hex_units`' integration tests spawn their own
  fake terrain — proof the contract is real.
- **The lint wall.** `unwrap`/`panic!`/indexing/`#[allow]` denied,
  `unsafe` forbidden, `missing_docs` denied. Calibrated for AI-agent
  contributions and enforced in CI.
- **Honest headless tests** (226 tracked at audit time), including
  regression tests verified by re-introducing their bugs, and
  every-file-a-scenario-names-must-parse tests.
- **The RON settings pipeline**: generic loader, absent-until-parsed,
  last-valid-on-bad-hot-reload, `validate()`-in-`Deserialize`, runtime file
  choice. This is the pattern the whole content plan extends.
- **CI**: fmt, clippy `-D warnings`, tests, doc `-D warnings`, cargo-deny,
  three-platform builds, markdown link check, and a tag-triggered release
  workflow already staging binary+assets archives for four targets.
- **State machine**: `Pause` and `Mode` as `SubStates` of
  `Screen::Gameplay` — illegal states unrepresentable; Loading blocks until
  every settings file parsed.
- Tuned profiles (release thin-LTO; a fast CI profile), a license
  allow-list, and — since the skills import — a receipted PR pipeline.

## Architecture gaps

1. **The lattice doesn't exist.** Gems, fusions, spells, mana, disables,
   enchantments, channeling — no code. An attack is a lunge animation plus a
   log line, deliberately (the repo refuses to invent a throwaway damage
   system). Everything below is shaped so this can be built without
   settling the open design questions.
2. **No command funnel.** `on_tile_clicked` deducts movement and inserts
   animation components directly; the AI does the same; a turn cannot end
   while a `Transformation` (an *animation crate* component) exists. Fine
   for a prototype; hostile to replay, netcode, save-mid-combat, and AI
   forward-simulation.
3. **Determinism hazards.** Turn-order ties break by entity index; entity
   allocation order depends on `HashMap` iteration during tile spawn; there
   is no stable unit identity; Perlin `seed: None` worlds are
   unreproducible. Same units, different order across sessions — poison for
   saves and co-op. (PR #52 fixed the seed story for procedural worlds:
   `ResolvedMapSeed`, per-scenario seeds, `generator_version`.)
4. **Nothing is serializable.** Zero serde on the domain vocabulary
   (`TilePos`, `HexCoord`, `SubstanceId`, `Turn`, `Faction`, `Body`). Saves,
   replays, and any network payload are blocked on this.
5. **No knowledge layer.** The design's hidden-information game (divination,
   decaying reveals, two-way cheap sight) has no seam: presentation would
   read enemy truth directly. The same missing indirection is the future
   server-side anti-cheat filter.
6. **Single-unit assumptions.** One player piece, one enemy; no roster, no
   party order, no notion of which seat controls which unit. Engagement and
   selection generalize, but the spawn scaffold does not.
7. **Provisional constants hardcoded.** Engage ranges, movement budget,
   initiative default, height bonus — all `const`, all flagged provisional
   in the code. The open design questions live in a compiled language.
8. **Scenario format is a scaffold** (two placements), acknowledged as such;
   PR #52 evolved it to `Fixed`/`Anchor` placements but not to encounters.

Known debt already flagged by the repo and confirmed: `hex_anim`'s
`Box<dyn Transformer>` (blocks `Reflect`; rewrite expected), no terrain move
costs, no unit obstruction (routes pass through pieces), no rout (stalemates
possible), `Body` footprint unbuilt, bevy_lint still unusable at 0.19.

## Production checklist (verified 2026-07-26)

| Piece | Status then |
|---|---|
| Save/load | Absent |
| Settings/options menu | Absent (RON files only; pause overlay is the only menu) |
| Audio | Absent entirely — no assets, no code; `bevy_audio` compiles in unused |
| Localization | Absent; strings hardcoded (design wants little/no dialogue → low priority, but UI strings exist) |
| Input rebinding / gamepad | Absent; keys hardcoded in systems |
| Window modes | Only `present_mode` via display.ron; no fullscreen toggle |
| App icon | Absent |
| Packaging | Release workflow archives 4 targets (good start); no signing/notarization, no Steam depot upload |
| Crash reporting | Absent (no panic hook; the lint wall is the only defense) |
| Logging | stdout only; Windows release has no console (`windows_subsystem="windows"`) → shipped logs go nowhere |
| Version display | Never surfaced in-game |
| Perf posture | Healthy at scale: shared meshes, per-substance materials, ~1–2.5k tile entities; terrain edit = full-grid respawn (flagged naive); diagnostics logging always on, release included |

## PR #52 addendum (reviewed in full; merged 2026-07-26)

The deterministic procedural map pipeline landed several things this audit
was about to ask for: `ResolvedMapSeed` + per-scenario `generation_seed` +
session rerolls (the seed contract), `generator_version` (versioning, as
data), `MapAnchors`/`MapAnchorId` (exact spawn surfaces), `TraversalProfile`
(shared standability/step predicates between validation and live movement),
`SpecialMovementRegion(s)`, a `TerrainReady` gate, and
`ScenarioPlacement::{Fixed,Anchor}`.

The review found one silent-failure UX regression (generation failure after
the loading gate strands the player in an empty world), two confirmed
generator bugs (a latent validation hang; a sky-island repair regression),
one future-armed contract drift (a directional melee predicate that would
have granted one-sided melee to the first profile able to drop farther than
it climbs), and a set of registration, doc, and simplification items — all
on the PR. The follow-up commit resolved them: `GameplaySetupFailure` plus a
terminal `GameplaySetup::Finalize` phase make a failed setup return to a
visible screen instead of an empty world; melee became symmetric
(`admits_step` in both directions); the `TraversalProfiles` registry and its
`GameplaySetup::Rules` phase were dropped in favour of the bare
`TraversalProfile` both sides already share; and the superseded
`TilePos::is_within_step_of` was removed rather than left as a second step
predicate.

Consequences absorbed into the plan: encounters should use anchor
placements; saves get seeds and versioning for free; the terrain-snapshot
ask is now clearly the primary save format (a `generator_version` bump
intentionally re-terraforms same-seed worlds, so regen-based saves are
version-fragile by design).

---

## The architecture: one organizing move

**The simulation becomes a pure, integer-valued, serializable core driven
exclusively through a validated command queue applied at one schedule
point; ECS entities, animation, and overlays become projections of it.**
Saves, co-op seams, replay, AI forward-simulation, and knowledge filtering
are corollaries of that one move, not separate systems.

The ten decisions, condensed. Type sketches are directional, not final.

### 1. A pure `hex_lattice` crate

The rules engine for the game's core system, positioned
`hex_core → hex_lattice → hex_assets → …`, built like `hex_core`: bevy_ecs +
bevy_reflect + serde + hexx, no `App`, no plugin, no renderer — the property
suite runs in milliseconds.

```rust
pub struct Lattice { cells: BTreeMap<LatticeCoord, CellKind> }   // the inscription
pub enum CellKind { Gem { element: ElementId }, Fusion { output: ElementId },
                    Spell { spell: SpellId }, Blank }
pub struct LatticeState {                                        // the battle-mutable half
    mana: BTreeMap<LatticeCoord, u16>,
    disabled: BTreeSet<LatticeCoord>,
    locks: BTreeMap<LatticeCoord, EnchantId>,   // disabling a locked gem breaks its enchantment
    enchantments: BTreeMap<EnchantId, ActiveEnchantment>,
    burns: Vec<Burn>,
}
pub fn castable(..) -> Result<CastPlan, CastBlocked>;  // ONE legality fn: preview, applier, AI
pub fn apply_disables(..) -> Vec<BrokenEnchantment>;
```

The inscription/state split gives level-up and save different lifetimes from
combat, and cloning `LatticeState` (small integer BTrees) is the AI's
forward-simulation primitive. `CastBlocked` is the "saying no out loud"
vocabulary; `CastPlan` (the exact gem-to-requirement assignment) is what
makes preview, application, and AI agree to the mana point. Everything is
integers — the crate simply defines no float fields, which is stronger than
a lint.

### 2. Vocabulary additions to `hex_core`

`LatticeCoord` (character-local hex; deliberately **no** world conversion —
the two spaces must not be confusable), `ElementId(u16)` and `SpellId`
(opaque, `SubstanceId`-style, table-assigned from sorted names; the wheel,
opposition, and fusion recipes are *data* — opposition is index arithmetic
over the wheel array, and no code ever matches on a specific element),
`UnitId(u64)` + allocator + registry, `PlayerSeat`/`ControlOwner` (seat 0
today; the entire co-op ownership model later), the command types below, a
`Busy` component (sim-side "still presenting" gate that replaces
`Has<Transformation>` checks and shrinks hex_anim's blast radius ahead of
its expected rewrite), `SimSystems::{Emit, Apply}`, and serde derives across
the domain vocabulary.

### 3. The command funnel

```rust
pub enum GameCommand {
    MoveAlong { unit: UnitId, path: Vec<TilePos> },
    Cast      { unit: UnitId, spell_cell: LatticeCoord, target: CastTarget },
    Channel   { unit: UnitId },
    ChooseDisables { unit: UnitId, cells: Vec<LatticeCoord> },
    EndTurn   { unit: UnitId },
    Strike    { unit: UnitId, target: UnitId },   // placeholder melee; dies when spells land
}
pub struct IssuedCommand { pub seat: PlayerSeat, pub command: GameCommand }
#[derive(Resource)] pub struct CommandQueue(VecDeque<IssuedCommand>);
#[derive(Resource)] pub enum PendingDecision { None,
    ChooseDisables { decider: UnitId, count: u8, from: UnitId } }
```

A queued **resource**, not a Message: one consumer at one schedule point,
drain order is the determinism contract, and the drained log is the replay
file and the future network payload for free. One applier in `hex_combat`
validates (turn, seat ownership, `Reach`-checked path, `castable()`,
decision-matches-pending) → applies (the only sim mutation site) → projects
(animation, overlays). `on_tile_clicked`, SPACE, and the AI keep their
logic and become emitters — the moment the AI and a human are
indistinguishable to the sim. `PendingDecision` is the defender-chooses
suspension point: an auto-policy answers it today, another player answers it
in co-op — the cheapest multiplayer seam in this plan.

### 4. Determinism fixes

`TurnOrder` becomes `Vec<UnitId>` with initiative-then-id ties; AI target
and selection ties move to `UnitId`; a `SimSeeds` resource holds the only
seeds (resolution takes no RNG *by signature* — the design's no-randomness
rule, structurally enforced); saves store `TilePos` only and re-derive
spans, so floats never enter a save. Walk interruption stays frame-timed —
accepted, and noted as the one thing a lockstep model (not planned) would
reopen. Standing rule now in force: **never key a sim decision on entity
order, entity bits, or query iteration order.**

### 5. Saves: a hand-shaped, versioned snapshot

`SaveFile` in `hex_game/src/save/` (no new crate — one consumer): a version
header read before the body (two-stage deserialize; migrations are
value-to-value functions), scenario reference, world = seed + settings
digest + the `TerrainEdit` log with substances **by name**, content digests
(xxh3 of the RON files → a legible refusal instead of silent drift), units
(id, seat, faction, `TilePos`, body, lattice trio, initiative), optional
combat state (order by id, round, turn, `PendingDecision`), knowledge,
campaign flags. Loading rides the existing Loading-screen flow
(`SaveToRestore` beside `ScenarioToLoad`) so every settings gate holds.
World restoration: seeded regen + edit replay now; the map-owned
`TerrainSnapshot` ([boundary.md](boundary.md) D2) as the generator-proof
primary format when it lands.

### 6. The knowledge seam

```rust
#[derive(Resource)] pub struct FactionKnowledge(BTreeMap<(Faction, UnitId), LatticeKnowledge>);
impl FactionKnowledge {
    /// THE accessor. UI and AI read enemy lattices through here or not at all.
    pub fn view(&self, viewer: Faction, subject: UnitId) -> Option<&LatticeKnowledge>;
}
```

Divination writes into it; decay ticks at round ends; base visibility
(faction, capacity, position) is always available. The reading rule is
established while it costs nothing — and a per-seat filtered snapshot is
exactly the server's anti-cheat view later.

### 7. Party prep (prep only)

`Party { members: Vec<UnitId> }` (ordered — the roster and the spawn/save
order), `ControlOwner(PlayerSeat)` on player units, and the scenario scaffold
generalized to a placement list. Formation travel is deliberately deferred;
when built, followers emit `MoveAlong` through the same funnel.

### 8. Content pipeline extension

All new files follow the existing pattern (validate-in-`Deserialize`,
absent-until-parsed, last-valid-on-bad-reload; names in files and saves, ids
session-local): `elements.ron` (wheel + elements + fusion recipes, DAG
validation), `spells.ron` (requirements multiset ≤ 6; casting axis
evocation/enchantment-with-upkeep; mana axis fixed/variable; `co_castable` —
"ritual" = variable + co-castable, separating the axes per the design's own
naming note; `TargetingSpec { range, shape, needs_los }` reusing
`hex_units::targeting`'s height-advantage geometry), `lattices.ron` (enemy
archetypes as cube-coordinate entries — the file **is** `LatticeSpec`'s
serde format, so the future in-game lattice editor round-trips for free; the
editor never rewrites hand-commented shipped files), `combat.ron` (policy
knobs), `progression.ron`, `encounters/*.ron`.

**Effects are a closed enum of primitives** — `DisableHexes{count,targeted}`,
`Burn`, `RestoreHexes`, `ModifyIncomingDisables`, `Reveal`, `Illuminate`,
`SetTerrain`/`ClearTerrain`/`SpawnWall` (substance by name), `Displace` — and
deliberately not a scripting engine: the lint wall exists to make runtime
failure unrepresentable and a script interpreter manufactures it; a closed
vocabulary can be bounds-validated at parse; the no-randomness and no-HP
rules stay structural; and for two people, every mechanic should have been
designed and reviewed. Extension cost is one variant + one match arm — the
compiler is the checklist. Cross-file integrity: a `ContentIndex` resource
(rebuilt only outside Gameplay, like `SubstanceTable`) plus a hex_game test
module that opens everything shipped and validates every reference.

### 9. UI direction

HUD and menus stay vanilla bevy_ui. The lattice inspector/builder — the
signature UI challenge — is world-space hex meshes driven by the existing
bevy_picking observer stack (the same machinery as tile clicks), with egui
acceptable for editor-grade panels. No third-party retained-UI framework
(the ecosystem's least durable dependency class — see research), no feathers
yet (experimental, editor-styled).

### 10. Crates *not* created

No `hex_ui`, no `hex_save`: boundaries that isolate nothing for two people.
Revisit when the lattice-builder screen or a server binary exists.

---

## Ecosystem decisions (verified July 2026)

| Need | Decision | Why |
|---|---|---|
| Saves | Hand-rolled serde domain snapshot; **bevy_persistent 0.11** for settings persistence | Community consensus is model-not-World; bevy_save is dead (stuck on Bevy 0.16); moonshine-save 0.7 works but overlaps what the command-driven sim provides |
| Future co-op | **bevy_replicon 0.41** (server-authoritative; ships a turn-based example) when netcode lands | Turn-based needs an intent seam + server validation, not rollback-grade float determinism; lightyear only if real-time exploration ever needs prediction |
| UI | Vanilla bevy_ui + world-space picking; egui (bevy_egui 0.40) legitimate for panels | sickle_ui dead, cobweb_ui unmaintained (Jan 2026); feathers experimental. Shipped Bevy games use first-party primitives |
| Audio | **bevy_kira_audio 0.26** behind a small facade | The shipped-game default; channels are the bus/volume story a settings menu needs; facade keeps the future Firewheel migration a leaf change |
| Steam | **game-ci/steam-deploy** on the existing release workflow; **bevy-steamworks 0.17** only when Steam features are wanted | Templates cover GitHub Releases/itch; nothing Bevy-side does depots |
| macOS | Ship arm64/universal; `.app` bundle + Developer ID signing with "Disable Library Validation"; notarize for outside-Steam | Rosetta 2 is being retired (gone for most apps by macOS 28 / 2027) |
| Crash reporting | **sentry-rust-minidump 0.16** on EmbarkStudios crash-handling; decide split debug symbols in the release profile early | The settled Rust-game stack; out-of-process minidumps |
| Engine | Stay 0.19 (current, June 2026); budget one upgrade (~0.20, Q4 2026) before any release window | BSN shipped but `.bsn` assets deferred → the RON pipeline is not obsolete; bevy_lint still 0.18-max |

Meta-lesson from shipped Bevy games (Tiny Glade, Settletopia, Times of
Progress): the pattern that ships is an engine-agnostic simulation core with
Bevy as the presentation shell — exactly the organizing move above. The
external tax is ~2 breaking engine upgrades per year and ~1 dead dependency
per year; the current posture (hexx + inspector as the only third-party
crates) is correct, and every addition above is deliberately small and
fenced.

## Testing and determinism strategy

- **The property suite in `hex_lattice`** is the cheapest test surface in
  the workspace (no `App`, no fixtures) and carries the design's geometric
  theorems: packing limits, enchantment breakage, fusion chain death,
  channel/cast conservation, serde round-trip identity.
- **The command log is the regression harness**: same commands, same order,
  same state — replayable headlessly. The existing hex_combat loop tests are
  the funnel-refactor's safety net.
- **The content test module** (in hex_game, the only crate that sees
  everything) opens every shipped file and validates every cross-file
  reference — a typo is a red test, not a runtime surprise.
- **Determinism rules**: sim state in BTree containers or sorted iteration;
  ids not entities; seeds only from `SimSeeds`; floats confined to
  presentation.
- **The window stays load-bearing.** Headless tests cannot see a black sky
  or a sunken piece; every serious bug here was found by a person looking.
  The visual walk remains part of every gameplay PR.

## Risks

- **Bevy 0.20 (~Q4 2026)**: BSN asset files and assets-as-entities are the
  churn to watch; one upgrade is budgeted before any release window.
- **Defender-chooses UX** may outgrow a single `PendingDecision` slot
  (simultaneous burns, co-cast joins, reactions) — the type is an enum
  behind a resource so growth is additive; revisit if design lands on
  interrupt-style reactions.
- **Segmented boss lattices** (semi-independent clusters, each worth a turn
  slot) would pressure the flat cell store; the inscription/state split
  survives it, and the property suite makes that refactor safe later.
- **Content renaming vs saves**: names-in-saves plus digests handle drift,
  but a wholesale renaming/localization pass would invalidate saves — the
  escape hatch is a stable `key` field per RON entry, provisioned as a save
  format v2.
- **Generator evolution vs saves** is real by design (`generator_version`
  re-terraforms same-seed worlds): the terrain snapshot ask (D2) is the
  answer; until then saves record seed + version and refuse legibly.
