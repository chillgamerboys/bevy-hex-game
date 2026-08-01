# Context for Claude Code

A hex-grid game on **Bevy 0.19**, organised as a multi-crate cargo workspace.

Read **[docs/architecture.md](docs/architecture.md)** first — it explains the crate
graph and, more usefully, the reasoning behind it. This file is the operational
summary.

## Tech stack

| Crate | Version | Notes |
|---|---|---|
| `bevy` | `0.19` | |
| `hexx` | `0.24` | **No Bevy features.** Pins only `glam`, so it can never gate a Bevy upgrade. `a_star` and `field_of_movement` are compiled in but **unusable** — both key on `Hex` alone, which cannot express two surfaces stacked at one coordinate. Pathfinding is `hex_units::movement::Reach`, over `TilePos` |
| `bevy-inspector-egui` | `0.37` | Targets bevy 0.19. Isolated in `hex_dev`, `dev` feature only |
| `ron` / `serde` | | Designer-facing settings |
| `xxhash-rust`, `rand` | | Terrain hashing |

Toolchain pinned to **Rust 1.97.1** in `rust-toolchain.toml`. Bevy 0.19's MSRV is
1.95, so this isn't optional — an older stable fails to build the dependency tree
with errors that don't obviously point at the toolchain.

## Build & run

```
cargo dev            # inspector + live asset reload
cargo run --release  # as it ships
cargo editor         # standalone Asset Workshop
```

**Always run through cargo.** `BEVY_ASSET_ROOT` is set in `.cargo/config.toml`
because in a workspace `CARGO_MANIFEST_DIR` is the *binary crate's* directory —
without it the game looks in `crates/hex_game/assets/`, finds nothing, and renders
a plain blue window with only `Path not found` in the log.

### Deterministic map review builds

`hex_game/src/review.rs` owns the exact-scenario launch and renderer-capture hooks.
They compile only with the default-off `map-review` feature, so the shipped release
binary ignores every `HEX_REVIEW_*` environment variable. The feature is separate
from `dev`: review packs exercise a release-shaped build without the inspector.

Launch one exact configured scenario seed for manual play:

```sh
HEX_REVIEW_SCENARIO="Procedural Hills" \
HEX_REVIEW_SEED=1592598566 \
cargo run --release -p hex_game --features map-review
```

This bypasses only the title-screen click. Loading, validation, terrain spawning, and
actor spawning still use the production path. Omit `HEX_REVIEW_SEED` to use the
scenario's configured seed; an override is valid only when that scenario declares
`generation_seed`.

Add a PNG path and camera view for a deterministic 1920x1080 renderer capture:

```sh
HEX_REVIEW_SCENARIO="Procedural Hills" \
HEX_REVIEW_SEED=1592598566 \
HEX_REVIEW_TIME=18.5 \
HEX_REVIEW_CAMERA=character \
HEX_REVIEW_CAPTURE=".context/procedural-maps/iteration-01/hero-default.png" \
HEX_REVIEW_VIEW=default \
cargo run --release -p hex_game --features map-review
```

`HEX_REVIEW_VIEW` accepts `default`, `rotated`, or `top-down` and requires
`HEX_REVIEW_CAPTURE`; omitting the view uses `default`. `HEX_REVIEW_CAMERA` accepts
`map` or `character` and also requires a capture. `HEX_REVIEW_TIME` accepts an hour in
`[0, 24)` and can be used with or without a capture, but the selected scenario must use
cyclic lighting. `HEX_REVIEW_LIQUID_PHASE` accepts any finite phase in seconds and
freezes liquid presentation there; captures default to `0.0`, while launches without a
capture keep live animation. `HEX_REVIEW_FOCUS_ANCHOR` relocates the selected actor to
an exact generated anchor before framing and requires a capture.
`HEX_REVIEW_CUTAWAY=full` exposes the selected cave interior for a review overview
while ordinary gameplay keeps its local six-hex opening; it also requires a capture.
`HEX_REVIEW_ILLUMINATION=overlay` adds exact Dark, Dim, and Bright cave-interior
gameplay-tier caps to a capture without changing physical lighting, perception, fog,
or picking.
The process exits after persisting the PNG. A frame that fails the visual-coverage
check still leaves its PNG at the requested path and exits with an error, so the
rejected output can be inspected.
Map-review builds keep their console on Windows because these diagnostics are part of
the tool.

### Scripted visual walks

The sibling default-off `visual-walk` feature drives the whole game through a RON
step list — screens, button clicks by `Name`, keys, scenario launches — and
photographs each step, so an agent can *look* at the frames (`/visual-walk` in the
skill pipeline reads them; audit-pr runs it as Step 2.5):

```sh
HEX_WALK_SCRIPT=walks/gameplay_ui.ron \
HEX_WALK_OUT=.context/visual-walks/local \
cargo run -p hex_game --features visual-walk
```

Exit code is the mechanical verdict: any stalled step, structural UI failure, or
black frame fails the run. The scoped gameplay route contains six deterministic
offscreen frames plus four native macOS checkpoints, never more than ten reviewed
images. It reviews hierarchy, layout, focus, legibility, and responsive composition
only; gameplay correctness is proved by canonical state snapshots in the
rules/contracts/simulation/app partitions. Offscreen capture redirects the camera
because the primary window is unreadable on macOS/Metal. The native wrapper
`tools/run_gameplay_ui_native_review_macos.sh` instead preserves the real window and
captures its exact CoreGraphics window ID for Retina/fullscreen/restart evidence.

## Workspace

```
hex_core → hex_assets → {hex_map, hex_world, hex_units → hex_combat} → hex_game
hex_core → hex_assets → hex_objects ───────────────────────────────→ hex_game
{Bevy, bevy_egui, hex_core, hex_assets} → hex_editor  (standalone tool)
hex_core → hex_ai → {hex_assets, hex_units, hex_combat}   (contracts, controllers, host)
{hex_core, hex_lattice} → hex_combat_core → hex_combat   (pure combat authority)
{bevy_ecs, hex_core} → hex_gameplay_model → hex_game  (pure screen behavior)
{Bevy, hex_core, hex_assets, hex_gameplay_model} → hex_ui → hex_game  (runtime presentation)
hex_core → {hex_assets, hex_units} → hex_perception → {hex_combat, hex_game}
hex_core → hex_lattice → {hex_assets, hex_units, hex_combat}   (pure rules engine)
hex_core → hex_anim ─────────────────────→ hex_units
{Bevy, inspector} → hex_dev ────────────────────────────────────────→ hex_game
```

**`hex_lattice` is the game's pure rules engine** — the lattice: gems, fusions,
spells, mana, disables, enchantments. Built like `hex_core` (Bevy sub-crates only, no
`App`, no plugin, no renderer), it depends only on `hex_core` and settles none of the
design's open questions. `hex_assets` resolves authored content into it, `hex_units`
carries per-unit lattice state, and `hex_combat` drives casts, disables, and decisions.
See `crates/hex_lattice`.

**`hex_gameplay_model` is pure screen behavior** — Combat Lab editing, report
selection and launch routing, plus Creator navigation and edit history. It does not
depend on assets, combat, units, the game binary, or renderer. `hex_game` adapts typed
model transitions to Bevy resources, persistence, and navigation instead of owning a
second copy of those decisions.

**`hex_map`, `hex_world` and `hex_units` must not depend on each other.** Shared
types go in `hex_core`. Cargo enforces this; a violating `use` fails to compile.

`hex_objects` is the renderer for static Workshop-authored objects. Producers publish
the shared `hex_assets::ObjectInstance` contract; they do not depend on the renderer,
and the renderer does not project gameplay blockers.

**`hex_perception`** owns authoritative illumination, faction sight, and map
knowledge. It depends on `hex_units` to observe unit positions.
`hex_units` will consume only the compact `LocalMapKnowledge` projection in `hex_core`
for player movement, while `hex_combat` consumes faction-generic projections and the
richer current-observation API to gate gameplay-owned lattice knowledge, cast anchors,
and AI. Neither gameplay crate may import map-generator internals.

**Two owners, two roles.** The **world owner** has `hex_map`, `hex_world`,
`hex_perception`, their schema/settings modules in `hex_assets`, and map/perception
content (world files, `substances.ron`, lighting profiles, `perception.ron`).
The **gameplay owner** has `hex_core`, `hex_units`, `hex_combat_core`, `hex_combat`, `hex_lattice`,
`hex_anim`, generic `hex_assets` loader infrastructure, and gameplay schema/settings
modules and content (`combat.ron`, `spells.ron`, `elements.ron`). `hex_game` is shared;
`hex_objects` and `hex_editor` are shared presentation/tooling with no gameplay
authority. Every fact that crosses between the owners, and whether it is live, agreed,
reserved, or still an ask, is `docs/contracts.md`; the open asks are
`docs/planning/boundary.md`.

`hex_assets` ownership follows the concern, not the directory. Generic loader traits,
load tracking, registration patterns, and cross-domain reference machinery stay with
the gameplay owner. A domain owner may change its own schema types, validation,
settings resources, content, and routine registration without a permanent loader
bottleneck. Generic loader behavior and cross-domain contracts still require their
owner's review; crate boundaries do not change.

**`hex_map` is a leaf** — nothing depends on it but the binary. It is owned by one
person, and the map reaches the rest of the game only through `HexTile`, `HexCoord`,
surface `TilePos`, `RunBottom`, `HexSpan`, `SubstanceId` and `Headroom` components on tile
entities. See `crates/hex_map/CLAUDE.md`. Cargo isolates the implementation, but
malformed components can still break gameplay at runtime.

**Ownership cuts both ways.** `hex_units` and `hex_combat` belong to the other person,
and a review comment on a *design* question inside someone else's crate is an argument
rather than a veto — the owner decides, writes down why, and moves. Contract bugs and
broken boundaries are the exception and should block. See
`docs/architecture.md#ownership-cuts-both-ways`.

**Delivery state has several projections.** Before planning from old tickets or
calling work complete, compare the implementation with status/design/roadmap docs,
GitHub, and Linear when it is connected. Linear is strongly recommended for
cross-owner visibility but never blocks a contribution from an owner who does not use
it. The tool-neutral contract is `docs/development/delivery-state.md`; Codex uses
`$reconcile-delivery-state`.

`hex_core` depends on Bevy sub-crates rather than the `bevy` facade, so it builds
and tests without a renderer. It holds the largest share of the test suite.

## Conventions

- **Subsystem modules expose `pub fn plugin(app: &mut App)`**, not a `Plugin`
  struct. Support modules such as generators do not need one.
- **Each plugin registers the reflected types it owns.** `hex_core` has no plugin,
  so the runtime plugin that introduces one of its shared types registers it.
- **`AppSystems`** (`TickTimers → RecordInput → Update`) orders systems that opt
  into those global `Update` phases; self-contained state/UI systems can run outside.
  **`PausableSystems`** gates gameplay work behind `Pause(false)`;
  **`GameplaySetup`** (`Resources → Terrain → Actors → Restore → Perception → View
  → Finalize`) orders `OnEnter(Screen::Gameplay)`. Ordering across a crate boundary
  *must* use a shared set — `.chain()` cannot express it, and a local chain that
  looks correct will race. The set boundary also supplies a sync point:
  `Commands`-spawned entities are not queryable until the queue is applied, so
  `Actors` sees the tiles `Terrain` made, `Restore` sees the scenario roster,
  `Perception` sees restored actors, `View` sees the completed projection, and
  `Finalize` sees the required actors.
- **`PerceptionSystems`** (`PublishAmbient → ResolveIllumination →
  ResolveObservation → PublishKnowledge → ApplyPresentation`) orders both initial
  perception and later updates. Authored lighting publishes
  `ExteriorIllumination`; gameplay never samples renderer lights or pixels.
- **Same-frame combat knowledge** is ordered `PublishKnowledge → combat spatial
  knowledge synchronization → CombatSystems::Act → Apply → Resolve → Advance`.
  Casting and AI must use that publication; neither preview nor a legal-action request
  authorizes a later command by itself.
- **A position is a voxel, not a coordinate.** `TilePos { coord, level }`. Separate
  surfaces in one coordinate's column are not connected. Never key anything by
  `HexCoord` in a way that collapses a stack.
- **The vertical axis is `level`, never `z`** — cube coordinates already use `x`, `y`
  and `z`, and all three are horizontal.
- **A tile entity is a run of voxels, not one voxel**, and its `TilePos` is the run's
  topmost material voxel while `RunBottom` is its lowest. Its substance determines
  whether that position is solid footing. Interior voxels have no entity, which is why
  targeting is positional. See `docs/systems/map.md`.
- **A surface needs room above it.** Every tile carries `Headroom` — clear voxels above
  it, 0 when buried inside a column — and a `Body` may stand only where headroom admits
  its traversal profile. The canonical walker is exactly 2 levels tall and may climb
  or drop 1. Only the map can measure headroom, so it publishes it; gameplay cannot
  derive it from spans.
- **Screens tag entities with `DespawnOnExit(Screen::X)`**; one generic system
  clears them.
- **Speeds are world units per second**, driven by `Res<Time>`, never `SystemTime`.
- **Settings come from `assets/config/*.ron`.** On initial load, resources are
  absent until parsed rather than defaulted, so a bad file stalls loading. After
  that, a failed hot reload retains the last valid value and reports the error.
  Elements, substances, spells, and lattices additionally require one matching
  `AcceptedContentRevision`; resource presence or a settled Bevy change tick cannot
  admit mixed source revisions.

## Bevy 0.19 specifics

Idioms that look right but aren't:

- `MessageReader<T>` / `MessageWriter<T>`, never `EventReader` (renamed in 0.17).
  **`AssetEvent<T>` is a `Message`** — read with `MessageReader`, not `add_observer`.
  `Pointer<Click>` is still an `Event` for observers.
- Required-component tuples (`Camera3d`, `Mesh3d`, `MeshMaterial3d`). No `*Bundle`.
- `ButtonInput<T>`, never `Input<T>`. `Color::srgb`, never `Color::rgb`.
- `GlobalAmbientLight` is a resource, not `AmbientLight` (which is a per-camera
  *component* in 0.19).
- Physical light units: illuminance in lux, `EnvironmentMapLight::intensity` in cd/m².
- **Cursor deltas via `CursorMoved`, never `MouseMotion`** — Wayland/WSLg does not
  deliver `MouseMotion` while a button is held. See `camera.rs::orbit_camera`.

### 0.18 → 0.19 deltas hit during the upgrade

Two of these aren't in the official migration guide:

- `DirectionalLight::shadows_enabled` → **`shadow_maps_enabled`**. *(Undocumented.)*
- `Assets::get_mut` returns an `AssetMut` wrapper, not `&mut A`. Bindings need
  `mut`, and you can't read the value inside the argument list of a method that
  mutably borrows it. *(Undocumented.)*
- `AssetLoader` implementations need `TypePath`.
- **`StandardMaterial::from(Color)` infers `AlphaMode::Blend` when alpha < 1; a struct
  literal does not.** It leaves `Opaque`, which discards the alpha and renders a solid
  object with no warning at all. Anything translucent must set `alpha_mode` explicitly.

Resources-as-components doesn't bite here because no type derives both `Resource`
and `Component`, and every query names concrete components.

## Traps

Several failure modes produce **no log output**. A clean log is not evidence a
change worked — look at the window. The sharpest three:

- **Plain blue window** — assets not found (see "Always run through cargo").
- **Black sky** — the sky shader failed to load, or the dome was culled.
- **Appears frozen** — it is paused. The overlay exists because this was
  indistinguishable from a hang.

Full list, including the map-specific ones:
[docs/development/troubleshooting.md](docs/development/troubleshooting.md).

**Observers are global.** They fire in every state. One touching a gameplay-only
resource must take `Option<Res<T>>` — Bevy validates parameters *before* the body
runs, so an internal guard won't save it. This caused a real crash on the title
screen.

## Branch & PR workflow

**Everything lands on `dev`. Nothing is merged straight to `main`.**

```
feat/whatever  ──PR──►  dev  ──PR──►  main
feat/ticket    ──PR──►  wave/N-name  ──one walked PR──►  dev
```

`dev` is permanent — it is the integration branch, not a release branch that gets
cleaned up. Standalone work PRs straight onto it; **related work with shared contracts,
hot files, or one meaningful runtime checkpoint goes through a short-lived
`wave/*` branch**. Source branches are work lanes and need leaf PRs only when focused
review is useful. The combined wave gets the full audit and human walk, then lands on
`dev` in one merge and is deleted (never `dev`). See
[parallel development](docs/development/parallel-development.md) for the topology
decision table and reconciliation rules.

```sh
gh pr create --base dev          # standalone work
gh pr create --base wave/N-name  # a ticket PR joining its wave
```

`main` only ever moves by merging `dev` into it, as a deliberate promotion once the
work there has been played and looked at. That gap is the point: **CI cannot see a
black sky, a gap between tiles, or a piece sunk into the terrain**, and every serious
bug in this codebase so far was found by a person clicking. `dev` is where things are
allowed to be wrong.

- Prefixes: `chore/`, `fix/`, `perf/`, `feat/`, `docs/`, `refactor/`.
- `refactor/*` names are usable again now the `refactor` branch is gone; a git ref
  can't be both a file and a directory, so they clashed while it existed.
- Merge with merge commits (`gh pr merge N --merge`), not squash.
- Delete feature branches once merged. **Never delete `dev`.**
- CI runs fmt, clippy, tests, `cargo deny`, and builds on all three platforms for
  Rust-affecting PRs into `dev` as well as into `main`. Markdown-only changes skip
  the Rust jobs.

### Skill pipeline

Codex reads root [`AGENTS.md`](AGENTS.md) automatically and discovers repository
skills under `.agents/skills/`. Use `$plan-parallel-work` before dividing a related
outcome across lanes, and `$land-development-wave` to reconcile and land an existing
batch without multiplying release gates.

The PR lifecycle is driven by skills in `.claude/skills/`:
`/create-pr` → `/audit-pr` → `/merge-pr` for feature work into `dev`;
`/promote` for the deliberate dev→main hop, which gates on a human having
played the build; `/release` to bump `[workspace.package] version` and tag
`vX.Y.Z` (the tag triggers the release build). Commit subjects follow
Conventional Commits — `/release` computes the version bump from them.
`/audit-pr` writes `/tmp/audit-pr-receipt-<PR>.json`; `/merge-pr` refuses to
merge without a green receipt for the current HEAD.
Test tiers: `/test-quick` (fmt+clippy+tests) → `/test-local` (+deny, doc,
links) → `/test-full` (+ship build; the visual walk stays manual).
Gameplay and map tests are partitioned by concern in
[`docs/development/gameplay-testing.md`](docs/development/gameplay-testing.md) and
[`docs/development/map-testing.md`](docs/development/map-testing.md); logical combat
evidence comes from rules/contracts/simulation/app data, while map logic uses
unit/generation/publication data and retains its existing visual criteria.
Standalone audits: `/audit-diff`, `/audit-silent-failures`, `/update-docs`,
`/visual-walk` (the scripted capture walk — audit-pr's Step 2.5; the agent
reads the frames, and the human walk still owns motion and taste).
Tickets live in Linear (team HEX): `/plan-ticket` to start from one,
`/update-linear` to bind a PR, `/seed-tickets` to turn a roadmap into
tickets. Binding is encouraged, never required.

## Current state

Runs on macOS/Metal at 60 FPS, 3,400–4,100 entities in gameplay depending on the
terrain seed. Bevy 0.19 and Rust 1.97.1 are pinned. The test count is intentionally
not frozen here; the current foundation gate and its exact count are recorded in
[foundation-hardening.md](docs/planning/foundation-hardening.md). macOS is the primary
dev machine; the WSL2 setup in the README belongs to another contributor and still
works.

**What is built, what is a placeholder, and what each placeholder is waiting for
lives in [docs/planning/status.md](docs/planning/status.md)** — the one doc allowed
to be out of date. Everything else under `docs/` describes contracts.

## Constraints on how you write here

- **Lints are strict, deliberately.** `#[allow]` is banned — use
  `#[expect(lint, reason = "…")]`. `unwrap`, `panic!`, slice indexing, `dbg!`,
  `println!`, float `==` and undocumented public items are all denied. Tests may
  unwrap, expect, panic, debug and print; slice indexing and the other restrictions
  remain denied.
- **Headless integration tests** use capability-based app mechanics from
  `hex_test_app` and dependency-limited fixtures from `hex_test_support`, then live
  in their owning crate. Units and combat each
  expose one explicit `contracts` target; concern modules live beneath that target
  rather than creating another Bevy link. The single
  `hex_combat_core/tests/simulation.rs` target owns multi-turn composition, and the
  single `hex_game/tests/gameplay_app.rs` target owns gameplay UI behavior behind
  `test-support`. `game_content_contracts` and the library's private
  scenario/loading tests stay separately selectable in the residual shared seam.
  Map tests may reuse the neutral app shell while retaining their world-owned fixture
  data and acceptance criteria; they must not replace the map producer with a synthetic
  consumer arena. None can see anything visual — a black sky or a mistransformed tile
  still needs a human looking at the window.

**Gaps in the engine and the toolchain** — `bevy_lint` unusable at 0.19, Bevy
features untrimmed, animation still `Box<dyn Transformer>` — are recorded in
[docs/planning/status.md](docs/planning/status.md) with the rest of the status, so
there is one copy to keep current rather than three.
