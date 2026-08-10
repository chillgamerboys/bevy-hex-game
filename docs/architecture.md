# Architecture

How the code is organised, and — more importantly — **why**, so the reasons survive
contact with the next change.

## The crate graph

```
hex_core → hex_assets → {hex_map, hex_world, hex_units → hex_combat} → hex_game
hex_core → hex_assets → hex_objects ───────────────────────────────→ hex_game
hex_core → hex_ai → {hex_assets, hex_units, hex_combat}   (contracts, controllers, host)
{hex_core, hex_lattice} → hex_combat_core → hex_combat   (pure combat authority)
{bevy_ecs, hex_core} → hex_gameplay_model → hex_game  (pure screen behavior)
{Bevy, hex_core, hex_assets, hex_gameplay_model} → hex_ui → hex_game  (runtime presentation)
hex_core → {hex_assets, hex_units} → hex_perception → {hex_combat, hex_game}
hex_core → hex_lattice → {hex_assets, hex_units, hex_combat}   (the pure rules engine)
hex_core → hex_anim ─────────────────────→ hex_units
{hex_core, hex_lattice, Replicon, Aeronet} → hex_multiplayer ───────→ hex_game
{Bevy, bevy-inspector-egui} → hex_dev ──────────────────────────────→ hex_game
{Bevy, bevy_egui, hex_core, hex_assets} → hex_editor  (standalone tool)
{Bevy, hex_core} → hex_test_app → hex_test_support  (test-only app mechanics)
{Bevy, hex_core, hex_assets} ───→ hex_test_support  (test-only shared fixtures)
```

An arrow means "may depend on". **Cargo enforces this.** A `use` that crosses the
graph the wrong way does not compile — it is not a convention anyone has to
remember or review for.

That matters more here than in most projects, because a good deal of the code will
be written by AI agents. An agent that *can* import across a boundary eventually
will, and no amount of documentation prevents it. A compiler error does.

| Crate | Holds | Depends on | Owner |
|---|---|---|---|
| `hex_core` | Hex coordinates, voxel positions, factions, exact occupancy, substances, headroom, terrain edits/impacts/outcomes, app states, ordering sets, lattice ids | Bevy sub-crates only — no renderer | gameplay |
| `hex_lattice` | **The lattice**: gems, fusions, spells, mana, disables, enchantments — the game's core rules, as a pure engine | `hex_core` | gameplay |
| `hex_ai` | Authorized observations, canonical legal-action requests, profile/controller identities, and replaceable algorithm traits; no legality or simulation mutation | `hex_core`, Bevy sub-crates | gameplay |
| `hex_combat_core` | Frozen combat inputs, serializable state, the command reducer, typed outcomes, canonical snapshots and bounded simulation | `hex_core`, `hex_lattice`, `bevy_ecs` derive support only | gameplay |
| `hex_gameplay_model` | Pure Main Menu, Campaign, Sandbox, and Creator routes; bounded slot identities; draft edits; launch blockers; and edit history | `hex_core`, `bevy_ecs` derive support only | gameplay |
| `hex_ui` | Runtime UI rendering, immutable presentation models, typed UI intentions, responsive scale, semantic styling, focus/accessibility, and presentation-only observations | Bevy, `hex_core`, `hex_assets`, `hex_gameplay_model`; never gameplay/world implementations | shared presentation |
| `hex_assets` | Generic asset loading plus domain-owned RON schema and settings modules | `hex_core`, `hex_lattice` | loader infrastructure: gameplay; each schema/settings module and its content: that domain's owner |
| `hex_objects` | Palette-backed rendering of static authored voxel objects and isolated per-tree fade materials | `hex_core`, `hex_assets` | shared presentation |
| `hex_map` | **The map**: voxel storage, terrain generation, tile spawning, map settings | `hex_core`, `hex_assets` | world |
| `hex_world` | Sky, collision-aware camera presentation, tree obstruction, and review-only cutaways | `hex_core`, `hex_assets` | world |
| `hex_anim` | Moving a transform over time. Knows nothing about hexes | `hex_core` | gameplay |
| `hex_units` | Units and their lattices, AI-controller attachment, picking, pathfinding, body size, and the movement preview | `hex_core`, `hex_ai`, `hex_assets`, `hex_anim`, `hex_lattice` | gameplay |
| `hex_perception` | Authoritative illumination, faction sight, and remembered map knowledge | `hex_core`, `hex_assets`, `hex_units` | world |
| `hex_combat` | The loop: modes, turn order, algorithm-neutral AI host and legal-action enumeration, persistent effects, and faction lattice knowledge | `hex_core`, `hex_ai`, `hex_assets`, `hex_anim`, `hex_units`, `hex_lattice`, `hex_perception` | gameplay |
| `hex_multiplayer` | Transport-neutral protocol, bounded wire containers, custom-admission vocabulary, lobby/manifest contracts, disclosure-safe replicas, and default-off Replicon/Aeronet composition | `hex_core`, `hex_lattice`, Bevy app/ECS sub-crates, Replicon, Aeronet; never map/unit/combat/perception implementations | shared infrastructure |
| `hex_dev` | World inspector. Behind the `dev` feature | Bevy, `bevy-inspector-egui` | gameplay |
| `hex_game` | Thin executable library and composition root: observes authority, builds immutable UI view models, applies typed intents, and wires plugins | all runtime crates | shared |
| `hex_editor` | Standalone palette, voxel-style, and object authoring; validated explicit writes, untracked recovery, and deterministic review packs | Bevy, `bevy_egui`, `hex_core`, `hex_assets` | shared tooling |
| `hex_test_app` | Capability-based deterministic Bevy app construction, plugin finalization, bounded settling, and shared state entry; no fixtures or owner implementation | Bevy, `hex_core` | shared testing |
| `hex_test_support` | Test-only deterministic app setup plus consumer-side synthetic exact-surface facts and fixture assets; no gameplay or world implementation | Bevy, `hex_core`, `hex_assets`, `hex_test_app` | gameplay testing; neutral app shell is shared across owners |

`hex_editor` is not a game screen and does not depend on runtime world or gameplay
crates. Reusable art schemas and validation live in `hex_assets`; the editor owns only
authoring workflow, crash recovery, review presentation, and filesystem side effects.
Recovery and review output stay untracked under `.context/asset-workshop/`, while
explicit saves are the only operations that change `assets/art/`. The canonical
palette and object contracts are described in
[design/visual-language.md](design/visual-language.md), and the operational workflow
is in [systems/asset-workshop.md](systems/asset-workshop.md).

### `hex_multiplayer` is a shared protocol boundary

Multiplayer is a server-authoritative listen-host projection, not a second simulator.
`GameCommandRequest` contains only a request id and `GameCommand`; an authenticated
connection lookup supplies its seat and temporary delegation before the existing
authority reducer sees an `IssuedCommand`. The host retains AI, combat truth, world
mutation, admission, global pause, and persistence. In particular, `CombatState` never
crosses the network boundary.

Lobby mutation follows the same rule. `ClientLobbyRequest` can only set the authenticated
guest's readiness or leave; it has no seat field. Assignment, kick, launch, retry,
return-to-lobby, and close use `HostSessionControlRequest`, which is a trusted local Bevy
message and is deliberately absent from protocol registration. Both paths converge on the
one `SessionAdmissionAuthority`, return a typed `SessionControlResult`, and publish its
canonical `LobbySnapshot`.

The shared crate owns stable data and transport registration, while each domain owns its
adapter. Gameplay publishes authorized `UnitReplica`/`SessionReplica` values. The world
owner alone exports/imports the ratified generator-neutral `WorldSnapshotV1`, computes
`PublicWorldFingerprintV1`, and transactionally derives/applies `WorldDeltaV1`.
Perception alone exports/imports `PlayerKnowledgeSnapshotV1` and decides which hostile
projections exist; networking applies that authorized view and cannot represent or
reconstruct private generator plans, hostile knowledge, or `CombatState`.

`MultiplayerPlugin` installs custom-auth Replicon, Aeronet adapters, and WebTransport
capability in one deterministic registration order. It does not spawn an endpoint or
open a socket. Offline play defaults to `SimulationRole::Authority`; a remote client
must explicitly select `Replica`. Direct Connect and a later Steam transport share the
same messages, manifests, snapshots, seat checks, and saves.

Direct transport pins SHA-256 of the exact certificate `SubjectPublicKeyInfo` through the
project-owned `SpkiPinVerifier`. It retains certificate validity/lifetime, P-256 key, and
TLS handshake-signature checks; the production-unsafe disable-validation path is never
used. This preserves the connection-code contract despite `wtransport 0.6.1`'s safe
convenience verifier hashing complete leaf-certificate DER instead.

Every concrete host run has a random `SessionInstanceId`. Reconnect persistence binds
that id to the endpoint, SPKI pin, exact verified certificate expiry, seat/player
identity, and rotating credential. Only a matching typed closure, expiry, or successful
replacement can remove it; an unrelated failed endpoint never consumes recoverable state.

### `hex_map` is a leaf, on purpose

Nothing depends on it except the binary. It is owned by one person, and bounding the
compile-time blast radius is what makes that ownership manageable: gameplay, camera,
sky, screens and menus cannot import map internals.

The boundary does not make malformed output harmless. Those crates consume the
components the map publishes, so a wrong `TilePos`, `RunBottom`, `HexSpan` or
`Headroom` can still break movement or presentation. Cargo protects the dependency
graph; tests and visual review protect the component contract.

### `hex_lattice` is the rules engine, built like `hex_core`

The lattice — the game's core system: gems holding element mana, fusions combining
them, spells powered by adjacency, damage that disables hexes rather than subtracting
hit points — is a pure, headless, deterministic, serializable rules crate, built like
`hex_core`: Bevy sub-crates only, no `App`, no plugin, no renderer, so its property
suite (the geometric theorems: two tier-6 spells can never be adjacent, fusion chains
die downstream, a disabled locked gem breaks its enchantment, serde round-trips are
identity) runs headless in milliseconds. Every field is an integer and every
container a `BTreeMap`/`BTreeSet`, so determinism is a property of the types. It
settles none of [the design's open questions](design/game.md#open-questions) —
initiative, action economy, fight length, the functional-death threshold — it exposes
primitives and leaves the policy to the crates above it.

It still depends only on `hex_core`, and three crates consume it for distinct reasons.
**`hex_assets`** implements the engine's content lookup traits over
`elements.ron`/`spells.ron` and turns authored lattices into a `LatticeSpec`, so the
engine reads content without knowing what a file is. **`hex_units`** carries the
result: a unit's spec, state, and stats are attached at spawn, keyed by its archetype.
**`hex_combat`** drives it — casting through the command funnel, damage through
`apply_disables`, defender-owned disable choices, persistent effects, and the
knowledge seam the engine deliberately refuses to own.

### `hex_combat_core` is the authority, not a second simulator

Combat truth has a renderer-free home above the lattice engine. `CombatState` accepts
only frozen rules, stable roster records, exact arena links, explicit faction
observation, and ordered `IssuedCommand`s. It contains integer-valued/domain state,
ordered collections, and stable IDs—never an `Entity`, transform, viewport, clock,
asset server, or map-generator type. A refusal is transactional and produces the same
typed `CombatEvent` vocabulary as a successful transition.

`hex_combat` is the Bevy host: it resolves live published contracts into frozen input,
feeds human and AI commands to the authority, and projects state/events to ECS,
animation, summaries, and UI. Reducer-covered verbs mutate only `CombatState`.
The pure workbench freezes supported active-combat Cast and restoration facts and
reduces them, persistent effects, downing, revival, and outcomes without Bevy. The
live ECS path still resolves Cast and restoration through explicit content adapters:
each publishes a complete projection back through transactional exact-roster
validation before any later command may reduce. Exploration Rest, party movement, and
unsupported terrain/area spell effects remain outside the pure reducer. Missing
authority refuses every combat command; there is no legacy fallback or retained
shadow simulator.

Movement completion is an explicit domain projection, not an animation query.
`MovingTo` advances from the pausable virtual clock, publishes exact `TilePos`
crossings, and clears the shared `Busy` gate when it reaches its bound. Generic
`Transformation` components may start, finish, or be torn down independently; no
legality, logical position, AI decision, or turn-order system queries their presence.

Drawing an edge costs something worth naming: the compiler stops being the review
signal for that boundary, since anything in those crates can now reach the engine. The
trade is deliberate — the compiler cannot distinguish an intended consumer from an
accidental import once the edge exists. Like the map, the engine is one person's, and
its contract is the types it exposes.

Party state follows the same split. `hex_core` owns the serializable formation
vocabulary and session resource, `hex_assets` loads and validates named presets,
`hex_units` owns roster identity and selection/focus projection, and `hex_game` renders
and edits those facts. The UI never invents an entity ordering: keys and strip slots
follow the `Party` resource's stable `UnitId` order. See
[systems/party.md](systems/party.md).

### Ownership cuts both ways

Two roles, named so the arrangement survives a change of people:

| Role | Owns |
|---|---|
| **World owner** | `hex_map`, `hex_world` (sky, camera, cutaway), `hex_perception`, world/perception schema and settings modules in `hex_assets`, and their content: world files, `substances.ron`, lighting profiles, `perception.ron`, and the `terrain_damage.ron` allow-list |
| **Gameplay owner** | `hex_core`, `hex_units`, `hex_combat`, `hex_lattice`, `hex_anim`, `hex_dev`, generic `hex_assets` loader infrastructure, and gameplay schema/settings modules and content: `combat.ron`, `spells.ron`, `elements.ron` |

`hex_game` is **shared** — it is wiring, screens, scenarios and review tooling, and
whoever needs a change makes it. `hex_multiplayer` is also shared, with a stricter
dependency ceiling: it owns protocol/session contracts but no gameplay or world truth.
`scenario.rs` and `scenarios.ron` sit in the same shared middle, flagged to the other
side when a change touches their domain.

`hex_ui` is also shared presentation, but its dependency ceiling is strict. Domain
facts flow into it as immutable view models and player actions flow out as typed
intent. The complete contract, responsive model, and testing oracle are in
[systems/ui.md](systems/ui.md).

`hex_assets` is split by concern rather than guarded as one person's directory.
Generic mechanisms — loader traits, load tracking, common registration patterns, and
cross-domain reference infrastructure — remain gameplay-owned. A domain's schema
types, validation, settings resources, and matching RON content belong to that
domain's owner. The world owner may therefore add or change world/perception schemas
and perform their routine exports and registration without waiting on a permanent
loader gate. A change to the generic loading mechanism or to a cross-domain contract
still requires the owning review; placing domain code in `hex_assets` does not waive
the crate graph or the contract-first process.

Authored-art schemas and `assets/art/` are a shared visual-content contract: either
owner may add assets, while schema changes receive both reviews. Runtime adapters stay
with the crate that draws or consumes them.

`hex_objects` consumes the shared `ObjectInstance` request and renders it without
learning who requested it. Map generation, spell presentation, and future prop systems
can therefore publish an exact object id, origin voxel, level height, and rotation
without depending on the renderer. The renderer never publishes traversal blockers or
interprets object semantic parts as gameplay policy.

Where the two meet is [contracts.md](contracts.md); what each is still asking of the
other is [planning/boundary.md](planning/boundary.md).

The split is not only about compile times — it is about who gets to decide.

Review across that line is welcome and has caught real bugs in both directions. But a
comment on a *design* question inside somebody else's crate is an argument, not a veto:
whether height should help or hinder, what starts a fight, what a turn costs. The owner
answers it, writes down why, and moves. Blocking on agreement about taste would stall
work neither person is responsible for.

That has already happened once and is worth knowing about, because the code now
deliberately does **not** do what a blocking review comment asked. Engagement keeps two
units at one coordinate in the same fight however tall the column between them — see
[systems/combat.md](systems/combat.md#the-high-ground) for the reasoning. The reviewer
read it as a collapsed stack; it is the high ground working. Both readings are
defensible, and the deciding vote went to the crate's owner rather than to whoever
commented last.

**Contract bugs are the exception.** A wrong component on a tile entity, a broken
boundary, a crash — those are not taste and either owner should block on them.

The map reaches the rest of the game only through shared `hex_core` components and
resources. Tiles carry `HexTile`, `HexCoord`, `TilePos`, `RunBottom`, `HexSpan`,
`SubstanceId`, and `Headroom`; exact resources publish anchors, interiors, blockers,
biome membership, and view hints. Nothing outside `hex_map` references `VoxelMap` or
generator internals, so terrain storage and generation can be replaced wholesale
without anyone noticing.

`Headroom` is on that list because only the map can measure it: a run carries its own
extent but knows nothing about what is stacked on it, so gameplay cannot tell a surface
from the inside of a column — let alone whether a body fits in the space above one.

Writing goes the other way, through shared messages — gameplay cannot call into the
map. Live stone construction uses `TerrainEdit::Set`. The map-side receiver for the
second path, `TerrainImpact`, is also live and keeps toughness and damage policy in the
world: gameplay announces which voxels an elemental effect reaches and its power; the
map accumulates material health, destroys voxels at zero, and answers through
`TerrainImpactOutcome`; gameplay correlates that answer before releasing its pending
authority ([systems/casting.md](systems/casting.md)).

See [systems/map.md](systems/map.md) for the voxel model itself. V3's private
semantic plan and its exact published projections are specified in
[systems/world-generation-v3.md](systems/world-generation-v3.md).

`hex_units`'s integration tests spawn their own stand-in terrain, which is the
clearest available demonstration that the separation is real.

### The rule that carries the most weight

**`hex_world`, `hex_units` and `hex_map` must never depend on each other.**

Presentation, rules and content are the three things that most want to reach into
each other, and the ones that become most painful to separate once they have. Before
the split, `player.rs` imported from three foreign modules to do its one job.

Anything they share goes in `hex_core`. That is why `HexTile`, `HexGrid` and
`HexSpan` — which look like presentation concerns — live there: gameplay has to
query tiles without depending on how they are generated or drawn.

The `hex_perception` crate follows the same rule. It depends on `hex_units` only to
snapshot stable unit identities, factions, and exact standing positions, and on
`hex_assets` for validated sight settings and substance solidity. It cannot expose map
internals back to units. `hex_units` will read only the `LocalMapKnowledge` projection
in `hex_core` for the pending player-movement adapter; `hex_combat` consumes
faction-generic traversal projections and the richer current-observation API to gate
hostile identities, cast anchors, AI inputs, and gameplay-owned lattice knowledge.
Combat retains divination facts on their own expiry clock, but never decides that a
world unit is visible. A lighting-profile adapter
publishes the core `ExteriorIllumination` projection before perception runs; it does
not expose `hex_world` renderer state to perception. Physical lights and rendered fog
are presentation. Neither is the authoritative gameplay visibility calculation.

## Positions are voxels, not coordinates

A unit is not *at* `HexCoord(3, -1)`. It is on a **specific voxel** there —
`TilePos { coord, level }`. One `Column` owns that coordinate, but separate material
runs within it are unrelated positions that happen to share a horizontal address.
Only solid substances provide places to stand.

The vertical axis is called **`level`**, never `z`: cube coordinates already use `x`,
`y` and `z` and all three are horizontal.

> **Surfaces stacked at the same coordinate are not connected.** A unit on a bridge
> cannot step down to the ground beneath it. Reaching it means a ramp or spiral of
> adjacent surfaces descending gradually, or an ability that explicitly bypasses the
> rule — teleporting, tunnelling.

This is a game-design decision, and it decides a type. The practical consequence:
**never key a map by `HexCoord` in a way that collapses a stack.** A
`HashMap<HexCoord, f32>` keeping only the highest surface silently makes every lower
surface unreachable, and a unit crossing a bridge teleports to the ground.

That exact abstraction existed briefly during the refactor and was deleted rather
than fixed — an abstraction that *can* express the forbidden thing eventually will.

A step is one level, so the rule is an integer comparison rather than a float
epsilon — which is the concrete payoff for quantising the vertical axis. Which
abilities ignore the rule is movement design and lives in `hex_units`.

### Why `hex_core` avoids the `bevy` facade

It depends on `bevy_ecs`, `bevy_math`, `bevy_platform`, `bevy_reflect` and
`bevy_state` individually rather than on `bevy`. That keeps a renderer out of the
domain crate, so its tests run fast and headless. Pure domain coverage belongs there;
asset, map and gameplay tests live with the behavior they exercise and also run
without a GPU.

## Conventions

### Subsystems expose `pub fn plugin(app: &mut App)`

Composing modules use a function rather than `struct FooPlugin; impl Plugin for
FooPlugin`. Support modules such as generators do not need a plugin. Same result,
far less boilerplate:

```rust
app.add_plugins((camera::plugin, grid::plugin, sky::plugin));
```

### Reflection registration stays beside the composing plugin

`hex_dev` used to re-register five types the runtime plugins already registered,
which meant adding any reflected component required editing an unrelated crate.
Register a crate-owned type in that crate's plugin.

`hex_core` is the deliberate exception: it has no `App` and no root plugin, so the
runtime plugin that introduces a shared type to the composed app registers it.
For example, `hex_map::grid::plugin` registers the shared tile vocabulary it spawns.

### Ordering is declared, never inferred

Bevy runs systems in parallel and in unspecified order unless told otherwise. Shared
sets make the ordering that crosses crate boundaries explicit:

- **`AppSystems`** — `TickTimers → RecordInput → Update`, for the `Update`
  schedule. Systems opt into these phases when they participate in that ordering;
  state transitions and self-contained UI/presentation systems may run outside them.
- **`PausableSystems`** — gates gameplay work such as movement animation behind
  `Pause(false)`.
- **`GameplaySetup`** — `Resources → Terrain → Actors → Restore → Perception →
  View → Finalize`, for `OnEnter(Screen::Gameplay)`. `Restore` applies a validated
  bound Campaign slot after the scenario terrain and roster exist; `Perception` then
  derives initial knowledge from the restored actors, and `View` applies generated
  framing and presentation only after that projection exists.
- **`PerceptionSystems`** — `PublishAmbient → ResolveIllumination →
  ResolveObservation → PublishKnowledge → ApplyPresentation`, nested inside
  `GameplaySetup::Perception` on entry and `AppSystems::Update` thereafter. The first
  phase is the cross-owner hand-off from authored lighting, not a renderer query.
- **`TerrainSystems`** — `ApplyWorld → RefreshProjections → ReconcileActors →
  ConsumeOutcomes`, configured before illumination and later perception. Map-owned
  `ApplyWorld` applies impacts and publishes rebuilt facts/outcomes;
  `RefreshProjections` republishes occupancy and reconciles movement;
  `ReconcileActors` deterministically settles or adopts unsupported actors; and
  `ConsumeOutcomes` validates the matching batch before releasing gameplay authority.
- **`PresentationSystems`** — `ResolveCameraOcclusion → ApplyMaterials →
  ApplyVisibility`, in `PostUpdate` after final transforms. World presentation
  publishes whole-tree opacity, the object renderer owns isolated material clones,
  and fog/review visibility remains composable.

`GameplaySetup` exists because of two bugs worth not repeating.

The first: `hex_map` builds the world; `hex_units` spawns the player that
stands on the tiles built from it. Both run in the same `OnEnter` schedule, and the
crates cannot see each other, so `.chain()` could not express the dependency. A local
chain looked correct and raced in practice — a nondeterministic panic that would most
likely have appeared first on someone else's machine with a different core count.

The second is subtler and produced a visible bug: the player spawned at ground level
and **sank into the terrain**. It read tile entities in the same set that created
them, and **entities spawned via `Commands` are not queryable until the queue is
applied**. Ordering alone would not have fixed it — a set boundary also supplies a
sync point, which is what makes the tiles *visible* rather than merely earlier.

The names carry that: `Terrain` is the map, `Actors` are the things standing on it,
and `Restore` may replace those actors' session state before any observer derives
knowledge from them. The old `Resources → Entities` gave nowhere to say "entities
that depend on other entities", which is why the mistake was easy to make.

**Ordering that spans a crate boundary has to go through a shared set in
`hex_core`.**

### Screens own their entities

Each screen tags what it spawns with `DespawnOnExit(Screen::X)`, and one generic
system clears them on exit. Teardown is not a per-screen checklist somebody
forgets to update.

> **Presentation lives next door.** How the sky is actually drawn — the dome, the
> shader, and the four non-obvious choices inside it — is
> [systems/sky.md](systems/sky.md).

## States

```
Splash ──► Title (Main Menu host) ◄──────────────► Settings
              │  ├──► Campaign (three slots) ───────────┐
              │  ├──► Sandbox ──► deployment ───────────┤
              │  └──► Tools ──► Character/Spell Creator │
              │                    └──► local test       │
              │                                          ▼
              └──────────────────────────────────────► Loading ──► Gameplay
                                                                   │
                                                                   ├── BACKSPACE ──► typed owner
                                                                   └── Pause
```

`Screen::Title` remains the internal coarse state that hosts the player-facing Main
Menu, Campaign cards, and Tools page. `Pause` is a **sub-state** of `Gameplay`, so
"paused on the Main Menu" is unrepresentable rather than merely unlikely.

`Loading` is load-bearing, not decorative. It is what makes
`OnEnter(Screen::Gameplay)` a safe place to build the world: it blocks until every
settings file has parsed, the derived `SubstanceTable` exists, and every asset handle
has reached a terminal state. Gameplay systems can therefore take resources such as
`Res<MapSettings>` rather than `Option<Res<…>>`.

Campaign New Game and Continue deliberately share that path. New Game binds the
canonical Party Trial to one selected empty `CampaignSlotId`; it does not occupy the
slot until the first safe manual save. Continue validates that explicit slot and
stages its scenario, party, selection, formation, and play time for Restore. Empty,
corrupt, or incompatible records stay on Campaign with a visible reason instead of
constructing a partial session. `campaigns.ron` always projects exactly three indexed
records. A valid legacy `resume.ron` is migrated once into slot 1 without changing the
legacy file.

The session provenance carries `Campaign(slot)`, `Sandbox`, or `TestFixture`. Only an
exact Campaign slot is save-eligible. Its accumulated milliseconds advance only while
Gameplay is active, unpaused, and non-terminal; Loading, Main Menu and child pages,
deployment, pause, outcomes, Sandbox, and tests contribute no time.

Creator and Sandbox launches also share Loading. They install one frozen
shipped-plus-custom spell/content/lattice namespace before terrain and actors enter
Gameplay. `SandboxLaunchSnapshot` carries the exact map, resolved seed, ordered
rosters, accepted content revision, shipped combat rules, and eventual deployment,
so Retry Exact cannot observe later map, draft, or local-library edits. These sessions
refuse Campaign writes and restore the shipped namespace when they return.

An asset failure is terminal too. The asset server already reports it, and treating
failure as "still loading" would turn a visible missing-asset problem into a permanent
loading screen. That is why a bad mesh can still reach gameplay and produce the
documented plain-blue fallback.

### Observers are global — treat them that way

An observer registered with `app.add_observer` fires on **every** matching event,
in every state. `on_tile_clicked` took `Res<HeightMap>`, which only exists during
gameplay, so clicking the Main Menu panicked. Bevy validates system parameters
*before* running the body, so the observer's own "is this a tile?" guard never got
the chance to reject it.

**An observer that touches state-scoped resources must take them as `Option`.**

## Authored settings and local preferences

Tunable values live in `assets/config/*.ron` and are editable without Rust. See
[development/config.md](development/config.md).

On initial load, settings resources are **absent** until their file parses rather
than falling back to a default. A default that silently diverges from what someone
wrote is worse than a stall. After a valid resource exists, a failed hot reload keeps
that last valid value active while the asset server reports the error.

The Settings screen writes a separate, atomic local-preferences file outside
`assets/`. A valid local preference overrides the authored display default; a missing
file leaves the authored value in force, and corrupt preferences are rejected visibly
before falling back. Local preferences are user state, not hot-reloaded project
content.

`InputBindings` centralizes stable input actions, canonical defaults, categories, and
context-aware keyboard overrides. A compile-selected `InputActionInventory` excludes
development-only actions from shipping presentation and conflict validation while
retaining their serialized overrides for a later development build. If shipping edits
later occupy that tooling chord, development startup rehomes only the tooling action to
a deterministic free modified chord and atomically persists the repaired preferences.
Settings captures
one non-modifier key, resolves overlapping-context conflicts through explicit Swap or
Cancel, and persists only overrides in preferences schema v3. Row restore is an atomic
binding edit: if another row owns the canonical chord it opens the same explicit
conflict flow instead of creating a duplicate. Fixed Tab and Escape UI navigation stay
outside that remapping surface. Enter and Space may bind only to the gameplay actions
whose handlers explicitly yield to a focused control, preventing one press from also
dispatching an unrelated gameplay action. `AudioBusVolumes` and the audio facade
similarly reserve music, SFX, and UI seams without requiring Wave 5 to ship audio
content.

**Hex geometry constants deliberately stayed in Rust.** `HEX_INNER_RADIUS` and its
derivations in `hex_core::config` describe the dimensions of `hex.glb`. Editing
`0.88` without editing the mesh does not make tiles bigger — it makes them overlap
or leaves gaps, with nothing reported. A value someone can change should be one
they can change *safely*.

### Hot reload is partial, by construction

With the `dev` feature on, every settings file is watched and its resource is
re-inserted on change. Whether that is *visible* depends on when the value is read:

| Read | Files | Effect |
|---|---|---|
| Every frame | `camera.ron`, all of `lighting.ron`, the session `TimeOfDay` resource | Immediate |
| Until a local preference is saved | `display.ron` | Immediate; afterward the user's persisted presentation choice wins |
| At interaction | `player.ron` speed | The next movement started; an in-flight move keeps its speed |
| On coherent art-graph reload | `palette.ron`, `voxel_styles.ron`, `object_catalog.ron`, `objects/*.ron` | Existing authored object instances rebuild; a bad graph retains the last valid revision |
| At spawn | `world.ron`, `substances.ron`, `palette.ron` substance/unit swatches, `player.ron` scale | Next `OnEnter(Screen::Gameplay)` |

`lighting.ron` used to be split across the first and last rows: the sky shader read its
values every frame, but the sun and ambient were only applied on
`OnEnter(Screen::Gameplay)`, so tuning a light angle meant a round trip through the
Main Menu. `reload_lighting` now re-applies them on change, which is what makes the
lighting worth exposing at all — the values below are only useful if you can see them
move.

Returning to the Main Menu and re-entering rebuilds the world in under a second, so
this is a mild inconvenience rather than a gap. Regenerating terrain in place on
change would be a real improvement for anyone tuning it, and is a fair follow-up.

### Frame presentation appears unchanged on macOS

Measured, not assumed. Editing the active value through Settings, or editing
`display.ron` before a local preference exists, re-applies the setting — confirmed by
instrumenting the system that writes it — but the frame rate stays pinned to the
display refresh either way. macOS composites every windowed app and vsyncs it.

This also explains frame rates varying between 60 and 120 across runs with no code
change: ProMotion adapts on its own, and none of it was ours to control. The
setting is real on Windows and Linux.

## When it fails silently

Several presentation failures here produce no log output at all, and a clean log is
not evidence that the window is correct. The list of visual symptoms and their causes
is [development/troubleshooting.md](development/troubleshooting.md). Inspecting those
symptoms never substitutes for typed gameplay or world evidence.

## Testing

Testing is partitioned by the authority needed for the claim. The complete matrices,
commands, dependency ceilings, budgets, and anti-patterns are the
[gameplay](development/gameplay-testing.md) and
[map](development/map-testing.md) testing contracts.

Screenshots and rendered frames prove static presentation: camera framing/occlusion,
UI hierarchy/layout/legibility/focus/contrast/reflow, and rendered-map geometry,
materials, lighting, cutaways, seams, and composition. Video and human checks prove
camera motion, native-input response, animation, control feel, and taste. A visual
artifact may show how hook-established state is rendered, but whenever hooks,
components, resources, messages, logs, canonical snapshots, or deterministic
contracts can express gameplay or exact world state, those typed oracles are
mandatory; if one is missing, add it instead of inferring logic from pixels.

**Pure unit tests** live beside behavior throughout the workspace and do not need a GPU:
coordinate round-tripping, the cube invariant, lattice properties, content validation,
object meshing, perception, voxel columns and run-merging, substance id assignment,
and movement rules — including that a two-level body is refused a one-voxel crawlspace
a one-level body walks into.

**Focused ECS contracts** run a deterministic headless `App` and inspect components,
resources, messages and exact positions. Owning tests may reuse the neutral app shell
from dependency-limited `hex_test_support`. Gameplay consumer tests may also build
synthetic shared facts there; map tests retain their own world-owned fixtures and
acceptance criteria and must exercise the real map publisher. Separate asset
integration tests parse the GLB directly to verify mesh geometry.

**Composition** has one `hex_combat_core` simulation target and one `hex_game` headless
app/UI target. A simulation compares complete canonical snapshots from two fresh runs.
Rendered frames review presentation only; they are not a combat oracle.

Gameplay screen behavior that does not need a widget tree lives in
`hex_gameplay_model`. `hex_game` translates clicks and Bevy state changes into typed
model actions, then performs the resulting filesystem, resource, and navigation
effects. Sandbox pending/committed maps, resolved seeds, six-slot roster order and
duplicates, blocker priority, exact Retry identity, Campaign slot identities, typed
Creator returns, and bounded undo/redo therefore run without `App`, assets, renderer,
viewport, or screen internals. The headless app target retains only wiring and
lifecycle claims that require Bevy.

Together the focused contracts cover tile counts, that a tile's transform agrees with its `HexSpan`, headroom
under open sky and beneath platforms, clean teardown and re-entry, and three specific
regressions: the player must spawn *on* the surface, clicking before settings load
must not panic, and a buried run must never be standable.

### A test you have not seen fail is not evidence

Every regression test here was verified by **reintroducing its bug**, and that habit
has paid for itself twice:

- The crash test initially **passed** with the bug restored, because the shared
  harness inserted the very resource whose absence caused the crash.
- The buried-run bug shipped past a green suite because the fake terrain spawned
  **one** tile per coordinate. Every tile was trivially a surface, so a bug that
  confuses a buried run for a surface had nothing to bite on.

Both are the same failure: a fixture too simple to express the thing being tested.
When adding a test, make the fixture resemble what the real map produces — stacked
runs, varying headroom — or it will report a safety it does not provide.

**These are headless.** A black sky, a wrong colour, or a mesh at the wrong scale
still only show up by looking at the window.

## What is not done yet

Engine and toolchain gaps — `bevy_lint`, Bevy feature trimming, the animation
rewrite — live with the rest of the status in
[planning/status.md](planning/status.md), which is the one doc allowed to lag.
