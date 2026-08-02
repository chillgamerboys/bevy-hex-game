# Gameplay testing contract

This document defines how gameplay-owned behavior is tested. It is a contract, not a
catalog of every test. The goal is to choose the cheapest authoritative oracle for
each concern and reserve rendered frames for facts only a renderer or a person can
judge.

This partition does not own `hex_map`, the map review runner, Forest or Waterfall
walks, perception/map stress suites, or their acceptance criteria. The complementary
[map testing contract](map-testing.md) keeps those oracles world-owned while both
owners use the same fail-closed scope selector and evidence format. Cross-owner
scenario binaries continue to run in the residual workspace suite.

## Concern partitions

| Concern | Owns | Allowed dependencies | Oracle | Local command | Ordinary budget |
|---|---|---|---|---|---:|
| Pure rules | Value objects, lattice transformations, compact AI policy | Owning crate dependencies only | Return values and immutable state | `python3 tools/test_scope.py run rules` | 60 s total |
| ECS contracts | Focused commands, effects, movement, turns, occupancy and Channel seams | `hex_test_support`, then the owning gameplay crate dependencies | Components, resources, messages and exact positions | `python3 tools/test_scope.py run contracts` | 60 s per test |
| Deterministic simulation | Multi-turn composition, tempo profiles, 3v3/6v6, canonical summaries and bounded no-progress | `hex_combat_core` over `hex_core` + `hex_lattice`; never Bevy App, `hex_test_support`, renderer, viewport, wall clock, asset server, ECS entity, perception implementation, or map generator | Full `CombatRunSnapshot` equality across two runs plus named metric assertions | `python3 tools/test_scope.py run simulation` | 60 s |
| Game/UI behavior | Pure Main Menu, Campaign, Sandbox, Creator, and guided deployment transitions plus Bevy wiring, persistence, re-entry, exact terrain placement, HUD suppression, and outcome lifecycle | `hex_gameplay_model` for route/draft/deployment/launch truth; `hex_ui` for rendering; `hex_game` with default-off `test-support` only for Bevy lifecycle | Pure state equality, `GameplayStateSnapshot` for authority facts and explicit presentation-adapter observations, and `UiTreeSnapshot` for presentation structure | `python3 tools/test_scope.py run app` | 60 s |
| Visual smoke | Layout, legibility, overlap, responsive composition and presentation regressions | Release-shaped game with `visual-walk`; no `dev` or `test-support` | Reviewed frames plus the human motion/feel walk | Run the one scoped gameplay walk through `/visual-walk` | At most 10 reviewed gameplay frames |
| Soak/performance | Long stalemates, stress corpora, bounded retention and performance | The scheduled stress workflow | Typed completion/timeout, fingerprints, timing and memory bounds | `.github/workflows/stress.yaml` | Scheduled/manual only |

The required gameplay CI job publishes separate JUnit and timing evidence for the
first four partitions. The residual workspace job keeps all other non-map packages
and cross-owner game/world contract binaries under the existing feature set, CI
profile, and timeout. Map unit, generation, and publication evidence runs through
its independently selected world-owned partitions.

## Integration target topology

Expensive Bevy links are explicit Cargo targets rather than one binary per source
module. `hex_units/tests/contracts.rs` links every focused unit/ECS contract once;
`hex_combat/tests/contracts.rs` does the same for combat, while
`hex_combat_core/tests/simulation.rs` remains the sole multi-turn simulation target.
Their concern modules live below a directory with the target name so ownership stays
readable without adding another linker invocation.

The app partition runs `hex_gameplay_model` and `hex_ui` inline tests together with
`hex_game/tests/gameplay_app.rs`, the default-off gameplay-owned application target.
Its primary run enables only `hex_game/test-support`; `hex_ui/dev-tools` is absent, so
the exhaustive structural matrix has the same runtime composition as shipping plus
immutable test observation. Before that run, the partition compiles the
default-feature `hex_game` library-test target in package isolation. That preflight
prevents workspace feature unification from hiding a test-only field or import that
the ordinary shipping-shaped crate cannot compile. A focused postflight runs only the
development-time UI tests with `hex_ui`'s `dev-tools,test-support` features. This keeps
optional inspector/clock coverage without allowing those surfaces to alter or mask
the primary matrix.
`hex_game/tests/game_content_contracts.rs` is the separately selectable shared
shipped-content seam, and the `hex_game` library target retains inline
scenario/loading contracts that require private composition details. Those shared
targets stay in the residual gate. The three packages set `autotests = false` and
declare their integration targets explicitly, so adding a helper file cannot silently
create a new expensive binary or escape its concern selector.

## Scripted movement steps

The default-off `visual-walk` runner may exercise ordinary map movement without a
gameplay bypass. `ClickTile(q, r)` is valid only when that coordinate has exactly one
exposed (`Headroom > 0`) surface. A stacked coordinate must use
`ClickTile(q, r, level: Some(level))`; missing or duplicate exact surfaces fail the
walk. `ClickAnchor(name, expected)` resolves the generated map's published anchor and
requires it to equal the authored exact `TilePos` before emitting that same primary
pointer click; an anchor move therefore invalidates stale evidence instead of silently
reviewing a new route. `AwaitPartyIdle(max_frames)` waits on public party, registry,
command-queue, `Busy`, and `MovingTo` facts. Its bound must be positive and exhaustion
is a failing exit. After that wait,
`AssertSelectedAt(expected: (q: ..., r: ..., level: ...))` requires exactly one
selected unit, its authoritative `StandsOn`, and `CameraFocusTarget.surface` to match
the expected `TilePos`. An ignored or interrupted pointer click therefore cannot
qualify later captures as route evidence.

Idleness alone is not proof that a pointer request was accepted: an ignored click can
also leave the party idle. Route evidence therefore follows every movement and
`AwaitPartyIdle` with `AssertSelectedAt(expected)`, which checks both the selected
unit's authoritative `StandsOn` and the exact `CameraFocusTarget.surface` before any
destination capture. Large composite walks may restart the same exact seed and use
stale-checked intermediate destinations when ordinary combat would otherwise interrupt
a route; they still may not suppress combat. Five grouped Two Rings walks cover one
ordinary-network destination in all 19 regions. The Sky Islands case intentionally
stops at its grounded bridge because upper surfaces require flight.

`OrbitCamera(yaw_turns, pitch_fraction)` injects a bounded, multi-frame held-right-
button cursor drag. It never writes `PanOrbitCamera` or Character collision state.
Each gesture is limited to half a yaw turn and one quarter-turn of pitch, and the
ordinary camera system remains responsible for Character pitch clamping and desired-
pose authorship. `walks/camera_routes.ron` contains exactly one seed-pinned case for
every selectable Map scenario, with exact destinations and the azimuths that must be
reviewed. Scripted destinations must be members of that manifest. These steps provide
route evidence only when a map owner has authored and validated the waypoint sequence;
a capture-only script is not a traversal test, and no script may teleport, suppress
combat, fake flight reachability, or bypass pathfinding.

## Scope selection

The concern filter is not the Cargo selector. Cargo packages, targets, and features
must be selected first so a rules-only change cannot compile the renderer or unrelated
owner packages and then merely filter their tests out.

`.config/test-scopes.json` is the single machine-readable authority for
canonical concern commands and changed-path classification. Inspect the proposed
closure for a branch with:

```sh
python3 tools/test_scope.py plan --base origin/dev --head HEAD
```

For a source PR inside a wave, substitute that PR's wave base, for example
`--base origin/wave/8-gameplay-foundation`. Scoping against `dev` from a cumulative
wave source would correctly select every earlier lane, but it would not be a useful
edit-loop scope.

The selector unions concerns across changed files. A classified shared contract uses
the smallest producer/consumer closure that can compile or exercise its authority;
unclassified shared vocabulary, selector-command or CI-topology changes, unclassified
world-owned paths, an unknown path, an invalid manifest, or an empty diff still fail
closed to the complete gate. The combined `.config/test-scopes.json` file contains
both routing and executable commands, so every change to it stays full; selector
regression-test-only changes run the dedicated `selector` concern. A narrow rules graph
additionally runs:

```sh
python3 tools/test_scope.py check-graph rules
```

That guard rejects a workspace dependency edge outside `hex_core`, `hex_lattice`, and
`hex_ai` instead of letting renderer or application dependencies silently erode the
partition.

Changed-path closures follow executable dependency direction, not broad gameplay
proximity. For example, unit, animation, AI-adapter, and ECS-combat changes cannot
affect the renderer-free `hex_combat_core` simulation and therefore do not rerun it.
Asset and animation changes do select the residual partition because their owning
inline tests live outside the four explicit gameplay targets. A concern may be
omitted only when it cannot compile or exercise the changed authority.

The terrain-resolution foundation used this explicit review wedge:

```sh
cargo test -p hex_core --lib terrain_impact::tests::
cargo test -p hex_map --test contracts \
  terrain_damage::terrain_protocol_orders_reserved_phases_before_perception -- --exact
cargo test -p hex_map --test contracts \
  terrain_damage::overkill_is_capped_and_empty_voxels_report_no_material -- --exact
```

The first command exhausts the pure answer schema. The two headless map tests prove
the installed `ApplyWorld → RefreshProjections → ReconcileActors → ConsumeOutcomes →
perception` order and the real mixed material/air producer seam. They do not initialize
`hex_game`, `hex_ui`, a renderer, or a map-generation corpus.

The combined `hex_core::terrain_impact` file also owns the damaged-voxel projection
consumed by map and application code, so future whole-file changes remain fail-closed
until those authorities are split by file. That prevents a path-level exemption from
silently skipping a real consumer.

Changes confined to `hex_units::trajectories` or `hex_units::volumes` select the
dedicated `trajectory_contracts` concern. Its one nextest profile runs the pure
trajectory and volume unit modules; the dedicated volume contracts; two direct
creation-resolution units; and the direct AI-legality, authoritative command, and game
casting consumers. The current closure is 61 tests across `hex_units`, `hex_combat`,
and the `hex_game` library:

```sh
python3 tools/test_scope.py run trajectory_contracts
```

It emits JUnit at `target/nextest/gameplay-trajectory/junit.xml`. It does not select
`hex_ui`, the `gameplay_app` target, deterministic combat simulation, map generation,
or residual workspace tests. Changes to broader unit or casting authority continue to
use their broader producer/consumer closures.

Pull-request CI applies the selector directly and publishes the decision plus timing
evidence and JUnit for each nextest-backed concern. Pushes to `dev` or `main` forcibly
promote the decision to the complete integration gate, regardless of changed paths. Final
wave/release candidates likewise run the complete gate before the exact-head manual
sign-off; unknown paths, invalid configuration, and empty diffs also fail closed to
that same result.

For a gameplay-only change that does not modify `hex_core`, shared application
composition, scenario/loading lifecycle, or a published world seam, V3/map corpora
are not a relevant PR oracle. Their source, selection, and acceptance criteria remain
unchanged and continue on their owning changes and broad gates.

## Shared support boundary

`hex_test_app` is the lowest test-only infrastructure tier. Its Cargo dependency
ceiling is Bevy and `hex_core`; callers explicitly opt into assets, states, input,
shared schedules, and deterministic time so absence tests do not receive hidden
capabilities. It owns plugin finalization, bounded settling, and state-entry mechanics,
but no fixture or owner implementation.

`hex_test_support` builds on that neutral tier. Its Cargo dependency ceiling adds
`hex_assets`; it may publish synthetic shared surfaces, load fixture
palettes/settings/content, and observe shared positions/resources. Its existing
`TestAppBuilder` remains the complete compatibility shell for gameplay and map
contracts.

The deterministic app shell, plugin finalization, and state-transition helpers are
neutral infrastructure available to owning tests on either side of the gameplay/world
boundary. Domain fixtures and acceptance criteria stay with their owner. In
particular, `hex_map` publication tests may reuse the app shell but must exercise the
real map plugin; using `SyntheticArena` there would replace the publisher under test
with consumer-authored facts and invalidate the evidence.

It must never depend on `hex_units`, `hex_combat`, `hex_game`, `hex_map`,
`hex_world`, or `hex_perception`. Such an edge would let the fixture reconstruct
private gameplay or world truth and turn a test helper into a second authority.
Owning tests add their system under test on top of the support app.

`hex_game` exposes `GameplayStateSnapshot` only behind `test-support`. It observes
canonical screen/phase/mode, session provenance, turn and budgets, pending decisions,
command state, exact positions, lattice summaries, `CombatSummary`, terminal outcome,
and frozen launch/retry identity. Its explicitly
named `presented_actions` field mirrors `GameplayHudView` only for application-adapter
parity; it is not a legal-action oracle and cannot replace owning command/contract
tests. `hex_ui` separately exposes `UiTreeSnapshot` for visible regions,
focus/accessibility, computed bounds/overflow, and action priority. Neither feature
exposes mutable screen state, and the shipping binary has no feature dependency on
either harness.
`hex_game` also re-exports `HeadlessUiPlugin`, which installs the real presentation
schedules on a synthetic window without Winit or a renderer. Use it only with
`UiTreeSnapshot` presentation assertions; canonical behavior still comes from the
authority-backed fields in `GameplayStateSnapshot` and the owning rules, contracts,
or simulation target.

`hex_game/test-support` additionally owns typed internal-scenario and
deterministic-fixture launch requests, optional combat-rule-profile injection, and
observation snapshots. Stable IDs such as `ability-lab`, `raider-mirror`, and
`tempo-matrix` are resolved through the same definitions simulations consume. None
of those request types, manifests, or injected rules joins the shipping plugin graph.

`hex_gameplay_model` owns renderer-free Main Menu, Campaign, Sandbox, and Creator
transitions. It may depend on `bevy_ecs` derive support and `hex_core`, but not on
assets, combat, units, game, map, world, perception, or the Bevy facade. It is the
oracle for route and Back behavior, pending/committed map and resolved-seed edits,
fixed six-slot roster identity/order/duplicates, launch-blocker priority, guided
Party-then-Enemies deployment order, exact placement occupancy, reselection, Undo and
Review, exact Retry identity, Campaign slot identity, typed Creator destinations, and
bounded edit history. Widget systems emit typed actions into that model and apply
effectful results; they do not duplicate those decisions.

The application adapter proves that one ordinary `HexTile` click outside the catalog's
hidden staging regions is accepted when canonical walker footing admits its exact
`TilePos`, while invalid footing and occupied surfaces refuse without advancing.
Headless presentation evidence separately proves that Deployment removes every
ordinary HUD surface from layout, focus, scrolling, and picking and leaves the compact
task card reachable. The scoped visual walk places at least one Party and one Enemy
character before capturing Review; pixels never substitute for the typed placement or
frozen-launch assertions.

### Campaign persistence evidence

Persistence tests use an isolated `HEX_GAME_DATA_DIR` and exercise all three explicit
slot IDs, mixed Empty/Available/Invalid projection, `UnitId(0)`, atomic replacement of
only the bound slot, compatible restore, corrupt/future/build/content/generator
refusal, and valid and invalid legacy migration. They verify that neither migration
nor a later save changes or deletes `resume.ron`.

Time tests drive the application clock and prove accumulation only while a
Campaign-origin session is Gameplay, active, unpaused, and non-terminal. Loading,
Main Menu, Campaign pages, pause, deployment, outcomes, Sandbox, and deterministic
test cases must produce a zero delta. A separate access-recording compatibility
sentinel creates an existing `combat-reports.ron`, exercises legacy migration,
requires zero read/write operations against that path, then requires byte-for-byte
identity and unchanged existence. The default-build terminology gate and test-only
storage path prove that shipping runtime code has no report-history reader, writer,
or deletion path.

Report-history and comparison regressions are replaced by assertions over canonical
`CombatSummary`, complete deterministic run snapshots, frozen launch identity, exact
retry identity, and terminal outcomes.

## Simulation evidence

One `CombatCase` freezes typed unit/controller inputs, profile, explicit arena links,
world-published observation, stable content names, active-combat spell facts, and
independent command/turn/no-progress bounds. One run produces a
`CombatRunSnapshot` containing canonical metrics and complete state plus state,
command, and full-transcript fingerprints, typed outcome or exact bound termination,
turn state, lattice summaries, and exact `TilePos`
positions.

Every acceptance case runs twice from fresh state and requires complete snapshot
equality before checking named metrics. A bounded no-progress result is data. It is
never silently converted to success, inferred from a frame, or described as a
terminal outcome.

`ControllerInput::Scripted` consumes exact replayable commands and fails when a
per-unit script is exhausted or names another unit. `ControllerInput::Baseline` is a
stable non-random reference policy that answers lattice decisions, casts supported
spells, strikes, advances over explicit links, and yields. The reducer owns cast
payment, direct disables, restoration, Burn, downing, revival scheduling, and
outcomes, so the partition proves their composition as well as deterministic
state/turn, occupancy, profile, fingerprint, and bound behavior. It remains a
regression oracle, not evidence that the baseline policy is strategically optimal or
that balance is fun.

## Visual evidence policy

The complete runtime task inventory and its full headless matrix are defined in
[Runtime UI verification](ui-verification.md). Every populated interactive task runs
all required logical viewport and semantic-scale combinations in one linked binary;
separate mapping cases prove that device pixels do not alter logical layout. A visual
route samples that contract and does not define the set of UI paths that exist.

Screenshots answer presentation questions: whether controls fit, labels read, dense
rosters remain legible, regions overlap, focus remains visible, and a responsive
surface adapts. They do not prove occupancy, action accounting, tempo, determinism,
state restoration, Campaign persistence, or launch/retry identity.

A scoped gameplay acceptance run reviews at most ten deterministic Bevy image-target
frames: Main Menu, Campaign, Sandbox Overview, Map Browser, one Map Detail, Party,
Enemies, Character Picker, Tools, and one targeted Compact or 4K duplicate.
Default-off presentation fixtures create visual state without solving combat.
Before any capture, the live `UiTreeSnapshot` oracle rejects zero-area targets, inherited
clipping, off-canvas placement, unreachable scroll content, overlap, missing labels,
invalid focus order, and targets below 44×44. Authored presentation fixtures apply
named composition contracts, including horizontal Standard/Wide pages, 2×3 rosters,
Compact stacking, and one scrollable roster column. The same contract drives real
wheel and Tab/Shift-Tab input through the declared scroll owner. The scoped gameplay
script uses no combat-solving steps; generic world-owned walks retain their existing
driver verbs.

Forest, Waterfall, map review, and Alberto's map captures are outside this budget and
remain unchanged.

## Manual runtime sign-off

The automated Bevy visual walk, an agent frame review, and a named human playtest are
distinct evidence. Gameplay runtime-surface PRs record the human PASS in the structured
PR fields together with the full final head SHA, reviewer, date, and exact route
exercised. When a conservative runtime classification has no rendered presentation,
navigation, movement, persistence, or visual-script surface, a maintainer may instead
record an exact-head N/A waiver. The waiver names that maintainer's GitHub login and its
specific reason; the workflow requires the same maintain/admin account to trigger the
check. A later push makes either form of evidence stale. On a draft, the
`Current-head manual runtime sign-off` workflow may remain green because enforcement
is deferred; that is not a PASS or waiver. The required check must validate the new
head before the PR can leave draft.

Source-lane PRs targeting `wave/*` defer this gate because they are not independently
shippable runtime candidates. The combined wave PR targeting `dev` must carry the
named human sign-off for its exact final head; merging a lane into a wave never
inherits, substitutes for, or weakens that release gate.

Draft PRs may omit the sign-off while implementation is moving. A gameplay PR may not
be marked ready or merged with a placeholder, a blocked result, a different commit, or
an agent named as the human reviewer. An N/A waiver is available only for a
non-rendered change and only to a verified maintainer; it does not convert a known
gameplay failure into a pass. Combined wave PRs and release promotions still require
the named-human PASS.

## Anti-patterns

- **Noun-only assertions:** proving an ID, label, node, deterministic case name, or route exists
  without exercising the behavior it promises.
- **Screenshots as logic:** reading pixels or frame timing to infer movement budgets,
  exact occupancy, Channel actions, outcomes, determinism, or launch identity.
- **Fixture-description drift:** prose claims a matrix or state the owned deterministic case
  does not construct and verify directly.
- **All-feature runtime pollution:** enabling inspector/review/test features in a
  visual run and treating the resulting UI as shipping evidence.
- **Frame-sensitive balance evidence:** using an input heuristic, viewport-dependent
  timing, animation settling, or wall-clock timeout as a combat balance benchmark.
- **Unbounded settling:** polling until a test hangs. Every settle/run has an explicit
  frame or turn bound and reports exhaustion as a typed failure/result.
