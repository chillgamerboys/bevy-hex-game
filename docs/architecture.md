# Architecture

How the code is organised, and — more importantly — **why**, so the reasons survive
contact with the next change.

## The crate graph

```
hex_core → hex_assets → {hex_map, hex_world, hex_units → hex_combat} → hex_game
hex_core → hex_anim ─────────────────────→ hex_units
{Bevy, bevy-inspector-egui} → hex_dev ──────────────────────────────→ hex_game
```

An arrow means "may depend on". **Cargo enforces this.** A `use` that crosses the
graph the wrong way does not compile — it is not a convention anyone has to
remember or review for.

That matters more here than in most projects, because a good deal of the code will
be written by AI agents. An agent that *can* import across a boundary eventually
will, and no amount of documentation prevents it. A compiler error does.

| Crate | Holds | Depends on |
|---|---|---|
| `hex_core` | Hex coordinates, voxel positions, substances, headroom, terrain edits, app states, ordering sets | Bevy sub-crates only — no renderer |
| `hex_assets` | Asset handles, load tracking, RON settings and their loader | `hex_core` |
| `hex_map` | **The map**: voxel storage, terrain generation, tile spawning, map settings | `hex_core`, `hex_assets` |
| `hex_world` | Sky and camera | `hex_core`, `hex_assets` |
| `hex_anim` | Moving a transform over time. Knows nothing about hexes | `hex_core` |
| `hex_units` | Units, picking, pathfinding, body size, and the movement preview | `hex_core`, `hex_assets`, `hex_anim` |
| `hex_combat` | The loop: modes, turn order, the placeholder AI | `hex_core`, `hex_assets`, `hex_anim`, `hex_units` |
| `hex_dev` | World inspector. Behind the `dev` feature | Bevy, `bevy-inspector-egui` |
| `hex_game` | The binary: app setup, screens, menus, wiring | all of the above |

### `hex_map` is a leaf, on purpose

Nothing depends on it except the binary. It is owned by one person, and bounding the
compile-time blast radius is what makes that ownership manageable: gameplay, camera,
sky, screens and menus cannot import map internals.

The boundary does not make malformed output harmless. Those crates consume the
components the map publishes, so a wrong `TilePos`, `HexSpan` or `Headroom` can still
break movement or presentation. Cargo protects the dependency graph; tests and visual
review protect the component contract.

### Ownership cuts both ways

The map is one person's; **`hex_units` and `hex_combat` are the other's**. The split is
not only about compile times — it is about who gets to decide.

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
into the map, so a spell that digs or builds requests it and the map applies it.

See [systems/map.md](systems/map.md) for the voxel model itself.

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
- **`GameplaySetup`** — `Resources → Terrain → Actors → Finalize`, for
  `OnEnter(Screen::Gameplay)`.

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
| Every frame | `camera.ron`, `display.ron`, all of `lighting.ron` | Immediate |
| At interaction | `player.ron` speed | The next movement started; an in-flight move keeps its speed |
| At spawn | `world.ron`, `substances.ron`, `player.ron` scale/colour | Next `OnEnter(Screen::Gameplay)` |

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

