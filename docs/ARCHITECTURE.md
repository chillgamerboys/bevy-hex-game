# Architecture

How the code is organised, and — more importantly — **why**, so the reasons survive
contact with the next change.

## The crate graph

```
                    hex_core
                       │
                   hex_assets
                       │
      ┌────────────────┼────────────────┐
      │                │                │
  hex_world      hex_gameplay        hex_dev
      │                │                │
      └────────────────┼────────────────┘
                       │
                    hex_game
```

An arrow means "may depend on". **Cargo enforces this.** A `use` that crosses the
graph the wrong way does not compile — it is not a convention anyone has to
remember or review for.

That matters more here than in most projects, because a good deal of the code will
be written by AI agents. An agent that *can* import across a boundary eventually
will, and no amount of documentation prevents it. A compiler error does.

| Crate | Holds | Depends on |
|---|---|---|
| `hex_core` | Hex coordinates, terrain generation, app states, system-ordering sets, shared marker components | Bevy sub-crates only — no renderer |
| `hex_assets` | Asset handles, load tracking, RON settings and their loader | `hex_core` |
| `hex_world` | Presentation: grid spawning, terrain meshes, sky, camera | `hex_core`, `hex_assets` |
| `hex_gameplay` | Player, picking, movement, animation | `hex_core`, `hex_assets` |
| `hex_dev` | World inspector. Behind the `dev` feature | Bevy only |
| `hex_game` | The binary: app setup, screens, menus, wiring | all of the above |

### The rule that carries the most weight

**`hex_world` and `hex_gameplay` must never depend on each other.**

Presentation and rules are the two things that most want to reach into each other,
and the pair that becomes most painful to separate once they have. Before the
split, `player.rs` imported from three foreign modules to do its one job.

Anything they both need goes in `hex_core`. That is why `HexTile` and `HexGrid` —
which look like presentation concerns — live there: gameplay has to query tiles
without depending on how they are drawn.

### Why `hex_core` avoids the `bevy` facade

It depends on `bevy_ecs`, `bevy_math`, `bevy_platform`, `bevy_reflect` and
`bevy_state` individually rather than on `bevy`. That keeps a renderer out of the
domain crate, so its tests run fast and headless. `hex_core` is where the test
suite lives, and it should stay somewhere a test can run without a GPU.

## Conventions

### Modules expose `pub fn plugin(app: &mut App)`

Not `struct FooPlugin; impl Plugin for FooPlugin`. Same result, far less
boilerplate:

```rust
app.add_plugins((camera::plugin, grid::plugin, sky::plugin));
```

### Every plugin registers its own reflected types

`hex_dev` used to re-register five types their owning plugins already registered,
which meant adding any reflected component required editing an unrelated crate.
Register beside the type it belongs to.

### Ordering is declared, never inferred

Bevy runs systems in parallel and in unspecified order unless told otherwise. Two
sets make the intended order explicit:

- **`AppSystems`** — `TickTimers → RecordInput → Update`, for the `Update`
  schedule. Configured once in `main.rs`.
- **`GameplaySetup`** — `Resources → Entities`, for `OnEnter(Screen::Gameplay)`.

`GameplaySetup` exists because of a bug worth not repeating. `hex_world` inserts
`HeightMap`; `hex_gameplay` spawns the player that reads it. Both run in the same
`OnEnter` schedule, and the two crates cannot see each other, so `.chain()` could
not express the dependency. A local chain looked correct and raced in practice —
a nondeterministic panic that would most likely have shown up first on someone
else's machine, with a different core count.

**Ordering that spans a crate boundary has to go through a shared set in
`hex_core`.**

### Screens own their entities

Each screen tags what it spawns with `DespawnOnExit(Screen::X)`, and one generic
system clears them on exit. Teardown is not a per-screen checklist somebody
forgets to update.

## States

```
Splash ──► Title ──► Loading ──► Gameplay
                        ▲            │
                        └────────────┘
                                        └── Pause (sub-state of Gameplay)
```

`Pause` is a **sub-state** of `Gameplay`, so "paused on the title screen" is
unrepresentable rather than merely unlikely.

`Loading` is load-bearing, not decorative. It is what makes
`OnEnter(Screen::Gameplay)` a safe place to build the world: it blocks until both
the meshes and every settings file are present, so gameplay systems can take
`Res<WorldSettings>` rather than `Option<Res<…>>`.

### Observers are global — treat them that way

An observer registered with `app.add_observer` fires on **every** matching event,
in every state. `on_tile_clicked` took `Res<HeightMap>`, which only exists during
gameplay, so clicking the title screen panicked. Bevy validates system parameters
*before* running the body, so the observer's own "is this a tile?" guard never got
the chance to reject it.

**An observer that touches state-scoped resources must take them as `Option`.**

## Settings

Tunable values live in `assets/config/*.ron` and are editable without Rust. See
[CONTENT.md](CONTENT.md).

Settings resources are **absent** until their file parses, rather than falling
back to a default. A default that silently diverges from what someone wrote is
worse than a stall, because it looks like the edit did not work rather than like
an error.

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
| Every frame | `camera.ron`, `display.ron` | Immediate |
| At spawn | `world.ron`, `lighting.ron`, `player.ron` (size, colour) | Next `OnEnter(Screen::Gameplay)` |

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

## Things that fail silently

Several failure modes here produce no log output at all. A clean log is not
evidence that a change worked — **look at the window**.

| Symptom | Cause |
|---|---|
| Plain blue window | Assets not found. Bevy fell back to `ClearColor` with no meshes. Check `BEVY_ASSET_ROOT` in `.cargo/config.toml` |
| Black sky | The skybox `AssetEvent` was missed, so the PNG was never reinterpreted as a cubemap |
| Stuck on "loading…" | A RON file failed to parse, or an asset path is wrong |
| Movement looks wrong | A speed unit conversion. Speeds are world units per **second** |
| Game appears frozen | It is paused. The overlay exists precisely because this was indistinguishable from a hang |

## Testing

`hex_core` carries the suite, because it is pure and needs no GPU. 17 tests cover
coordinate round-tripping, the cube invariant, line drawing including the
degenerate zero-length case, radius tile counts, and terrain determinism.

Presentation and gameplay crates have no tests yet. Adding them means either a
headless `App` harness or extracting more logic down into `hex_core` — the latter
being generally the better answer.

## Not yet done

- **`bevy_lint`** is wired (`cfg(bevy_lint)` is declared, the `register_tool`
  attribute is in place) but unusable: it supports Bevy 0.18 at most, and this is
  0.19. Adopting it later costs no source changes.
- **Bevy feature trimming.** The `bevy` dependency still uses default features.
  Moving to `default-features = false` with the `3d` collection would cut compile
  time and binary size, but risks silently dropping capability, so it wants doing
  deliberately and verifying by running.
- **`missing_docs`** is set to `allow` in `[workspace.lints]`. Worth raising once
  the public API settles.
- **The animation system** is still `Box<dyn Transformer>` trait objects, which is
  why `Transformation` cannot derive `Reflect` and is invisible in the inspector.
  It works and is correctly frame-timed; it is the most likely thing to be
  rewritten when real gameplay lands.
