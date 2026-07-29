# Architecture

How the code is organised, and — more importantly — **why**, so the reasons survive
contact with the next change.

## The crate graph

```
hex_core → hex_assets → {hex_map, hex_world, hex_units → hex_combat} → hex_game
hex_core → hex_units → hex_perception → hex_combat  (planned)
hex_core → hex_lattice → {hex_assets, hex_units, hex_combat}   (the pure rules engine)
hex_core → hex_anim ─────────────────────→ hex_units
{Bevy, bevy-inspector-egui} → hex_dev ──────────────────────────────→ hex_game
{Bevy, bevy_egui, hex_core, hex_assets} → hex_editor  (standalone tool)
```

An arrow means "may depend on". **Cargo enforces this.** A `use` that crosses the
graph the wrong way does not compile — it is not a convention anyone has to
remember or review for.

That matters more here than in most projects, because a good deal of the code will
be written by AI agents. An agent that *can* import across a boundary eventually
will, and no amount of documentation prevents it. A compiler error does.

| Crate | Holds | Depends on | Owner |
|---|---|---|---|
| `hex_core` | Hex coordinates, voxel positions, substances, headroom, terrain edits, app states, ordering sets, lattice ids | Bevy sub-crates only — no renderer | gameplay |
| `hex_lattice` | **The lattice**: gems, fusions, spells, mana, disables, enchantments — the game's core rules, as a pure engine | `hex_core` | gameplay |
| `hex_assets` | Generic asset loading plus domain-owned RON schema and settings modules | `hex_core`, `hex_lattice` | loader infrastructure: gameplay; each schema/settings module and its content: that domain's owner |
| `hex_map` | **The map**: voxel storage, terrain generation, tile spawning, map settings | `hex_core`, `hex_assets` | world |
| `hex_world` | Sky, camera, and presentation cutaways | `hex_core`, `hex_assets` | world |
| `hex_anim` | Moving a transform over time. Knows nothing about hexes | `hex_core` | gameplay |
| `hex_units` | Units and their lattices, picking, pathfinding, body size, and the movement preview | `hex_core`, `hex_assets`, `hex_anim`, `hex_lattice` | gameplay |
| `hex_perception` | **Planned:** authoritative illumination, faction sight, and map knowledge | `hex_core`, `hex_units` | world |
| `hex_combat` | The loop: modes, turn order, the placeholder AI, faction knowledge | `hex_core`, `hex_assets`, `hex_anim`, `hex_units`, `hex_lattice` | gameplay |
| `hex_dev` | World inspector. Behind the `dev` feature | Bevy, `bevy-inspector-egui` | gameplay |
| `hex_game` | The binary: app setup, screens, menus, wiring | all of the above | shared |
| `hex_editor` | Standalone palette, voxel-style, and object authoring; validated explicit writes, untracked recovery, and deterministic review packs | Bevy, `bevy_egui`, `hex_core`, `hex_assets` | shared tooling |

`hex_editor` is not a game screen and does not depend on runtime world or gameplay
crates. Reusable art schemas and validation live in `hex_assets`; the editor owns only
authoring workflow, crash recovery, review presentation, and filesystem side effects.
Recovery and review output stay untracked under `.context/asset-workshop/`, while
explicit saves are the only operations that change `assets/art/`. The canonical
palette and object contracts are described in
[design/visual-language.md](design/visual-language.md), and the operational workflow
is in [systems/asset-workshop.md](systems/asset-workshop.md).

### `hex_map` is a leaf, on purpose

Nothing depends on it except the binary. It is owned by one person, and bounding the
compile-time blast radius is what makes that ownership manageable: gameplay, camera,
sky, screens and menus cannot import map internals.

The boundary does not make malformed output harmless. Those crates consume the
components the map publishes, so a wrong `TilePos`, `HexSpan` or `Headroom` can still
break movement or presentation. Cargo protects the dependency graph; tests and visual
review protect the component contract.

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

It still depends only on `hex_core`, and three crates now declare an edge to it — drawn
ahead of the code so the damage-loop PRs stop contending over the same manifests. Each
edge is for a different half of the job. **`hex_assets`** will implement the engine's
content lookup traits over `elements.ron`/`spells.ron` and turn authored lattices into a
`LatticeSpec`, so the engine reads content without knowing what a file is. **`hex_units`**
will carry the result: a unit's spec, state and stats go on at spawn, keyed by its
archetype. **`hex_combat`** drives it — casting through the command funnel, damage
through `apply_disables`, and the defender-chooses decision the engine deliberately
refuses to own; today it reads the lattice types only in `knowledge.rs`.

Drawing an edge early costs something worth naming: the compiler stops being the review
signal for that boundary, since anything in those crates can now reach the engine. The
trade is deliberate and temporary — the alternative was three PRs each editing the same
two `Cargo.toml` files. Like the map, the engine is one person's, and its contract is
the types it exposes.

### Ownership cuts both ways

Two roles, named so the arrangement survives a change of people:

| Role | Owns |
|---|---|
| **World owner** | `hex_map`, `hex_world` (sky, camera, cutaway), the planned `hex_perception`, world/perception schema and settings modules in `hex_assets`, and their content: world files, `substances.ron`, lighting profiles, the future `perception.ron` and terrain-response table |
| **Gameplay owner** | `hex_core`, `hex_units`, `hex_combat`, `hex_lattice`, `hex_anim`, `hex_dev`, generic `hex_assets` loader infrastructure, and gameplay schema/settings modules and content: `combat.ron`, `spells.ron`, `elements.ron` |

`hex_game` is **shared** — it is wiring, screens, scenarios and review tooling, and
whoever needs a change makes it. `scenario.rs` and `scenarios.ron` sit in the same
shared middle, flagged to the other side when a change touches their domain.

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

The map reaches the rest of the game **only through components**. Tiles are spawned
carrying a `HexTile` marker, `HexCoord`, `TilePos`, `HexSpan`, `SubstanceId` and
`Headroom`; `hex_units` queries those. Nothing outside `hex_map` references
`VoxelMap` or any generator, so terrain storage and generation can be replaced
wholesale without anyone noticing.

`Headroom` is on that list because only the map can measure it: a run carries its own
extent but knows nothing about what is stacked on it, so gameplay cannot tell a surface
from the inside of a column — let alone whether a body fits in the space above one.

Writing goes the other way, through the `TerrainEdit` message — gameplay cannot call
into the map, so a spell that digs or builds requests it and the map applies it. The
planned second write path, `TerrainImpact`, keeps the same direction and hands the map
even more authority: gameplay announces which voxels an elemental effect reaches, and
the map decides what each material does about it ([systems/casting.md](systems/casting.md)).

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

The planned `hex_perception` crate follows the same rule. It may depend on
`hex_units` to observe unit positions, but it cannot expose map internals back to
units. `hex_units` reads only the `LocalMapKnowledge` projection in `hex_core`;
`hex_combat` may depend on the richer perception API for detection, targeting, and
last-known-position behavior. A lighting-profile adapter publishes the core
`ExteriorIllumination` projection before perception runs; it does not expose
`hex_world` renderer state to perception. Physical lights and rendered fog are
presentation. Neither is the authoritative gameplay visibility calculation.

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
- **`GameplaySetup`** — `Resources → Terrain → Actors → Perception → View →
  Finalize`, for `OnEnter(Screen::Gameplay)`. `Perception` derives initial knowledge
  only after terrain and actors are queryable; `View` applies generated framing and
  presentation only after that projection exists.
- **`PerceptionSystems`** — `PublishAmbient → ResolveIllumination →
  ResolveObservation → PublishKnowledge → ApplyPresentation`, nested inside
  `GameplaySetup::Perception` on entry and `AppSystems::Update` thereafter. The first
  phase is the cross-owner hand-off from authored lighting, not a renderer query.

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

The names carry that: `Terrain` is the map, `Actors` are the things standing on it.
The old `Resources → Entities` gave nowhere to say "entities that depend on other
entities", which is why the mistake was easy to make.

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
Splash ──► Title ──► Loading ──► Gameplay
              ▲                     │
              └──── BACKSPACE ──────┘
                                      └── Pause (sub-state of Gameplay)
```

`Pause` is a **sub-state** of `Gameplay`, so "paused on the title screen" is
unrepresentable rather than merely unlikely.

`Loading` is load-bearing, not decorative. It is what makes
`OnEnter(Screen::Gameplay)` a safe place to build the world: it blocks until every
settings file has parsed, the derived `SubstanceTable` exists, and every asset handle
has reached a terminal state. Gameplay systems can therefore take resources such as
`Res<MapSettings>` rather than `Option<Res<…>>`.

An asset failure is terminal too. The asset server already reports it, and treating
failure as "still loading" would turn a visible missing-asset problem into a permanent
loading screen. That is why a bad mesh can still reach gameplay and produce the
documented plain-blue fallback.

### Observers are global — treat them that way

An observer registered with `app.add_observer` fires on **every** matching event,
in every state. `on_tile_clicked` took `Res<HeightMap>`, which only exists during
gameplay, so clicking the title screen panicked. Bevy validates system parameters
*before* running the body, so the observer's own "is this a tile?" guard never got
the chance to reject it.

**An observer that touches state-scoped resources must take them as `Option`.**

## Settings

Tunable values live in `assets/config/*.ron` and are editable without Rust. See
[development/config.md](development/config.md).

On initial load, settings resources are **absent** until their file parses rather
than falling back to a default. A default that silently diverges from what someone
wrote is worse than a stall. After a valid resource exists, a failed hot reload keeps
that last valid value active while the asset server reports the error.

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
| Every frame | `camera.ron`, `display.ron`, all of `lighting.ron`, the session `TimeOfDay` resource | Immediate |
| At interaction | `player.ron` speed | The next movement started; an in-flight move keeps its speed |
| At spawn | `world.ron`, `substances.ron`, `palette.ron` substance/unit swatches, `player.ron` scale | Next `OnEnter(Screen::Gameplay)` |

`lighting.ron` used to be split across the first and last rows: the sky shader read its
values every frame, but the sun and ambient were only applied on
`OnEnter(Screen::Gameplay)`, so tuning a light angle meant a round trip through the
title screen. `reload_lighting` now re-applies them on change, which is what makes the
lighting worth exposing at all — the values below are only useful if you can see them
move.

Returning to the title and re-entering rebuilds the world in under a second, so
this is a mild inconvenience rather than a gap. Regenerating terrain in place on
change would be a real improvement for anyone tuning it, and is a fair follow-up.

### `present_mode` does nothing on macOS

Measured, not assumed. Editing `display.ron` reloads the file and re-applies the
setting — confirmed by instrumenting the system that writes it — but the frame rate
stays pinned to the display refresh either way. macOS composites every windowed app
and vsyncs it.

This also explains frame rates varying between 60 and 120 across runs with no code
change: ProMotion adapts on its own, and none of it was ours to control. The
setting is real on Windows and Linux.

## When it fails silently

Several failure modes here produce no log output at all, and a clean log is not
evidence that a change worked. The list of symptoms and their causes is
[development/troubleshooting.md](development/troubleshooting.md); the habit it
asks for is looking at the window.

## Testing

Two complementary layers across the workspace.

**Unit tests** live in `hex_core`, `hex_map`, `hex_assets` and `hex_units`, none of
which need a GPU: coordinate round-tripping, the cube invariant, line drawing including
the degenerate zero-length case, span geometry, voxel columns and run-merging, substance
id assignment, and the movement rules — including that a two-level body is refused a
one-voxel crawlspace a one-level body walks into.

**ECS integration tests** run a headless `App` with `MinimalPlugins` and inspect the
world afterwards — `crates/hex_map/tests/`, `crates/hex_units/tests/` and
`crates/hex_combat/tests/`. Separate asset integration tests parse the GLB directly to
verify mesh geometry. They exist because every bug found in this codebase was found by
a person clicking, and the worst of them were green across compiler, clippy, unit
tests and CI.

They cover tile counts, that a tile's transform agrees with its `HexSpan`, headroom
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
