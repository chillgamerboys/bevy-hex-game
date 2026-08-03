# Gameplay testing contract

This document defines how gameplay-owned behavior is tested. It is a contract, not a
catalog of every test. The goal is to choose the cheapest authoritative oracle for
each concern and reserve rendered frames for facts only a renderer or a person can
judge.

## Non-negotiable oracle boundary

Screenshots and rendered frames are valid evidence for static presentation: camera
framing and occlusion, UI hierarchy/layout/legibility/focus/contrast/reflow, and a
rendered map's visible geometry, materials, lighting, cutaways, seams, and
composition. Video and human checks are valid for camera motion, native-input
response, animation, control feel, and taste. A visual artifact may show how a state
already established by hooks is rendered, but it does not establish the state.

Screenshots, rendered frames, video, and human visual observation are never evidence
for gameplay or exact world logic when the claim can be represented by typed hooks,
components, resources, messages, logs, canonical snapshots, or deterministic
contracts. This rule applies even when a visual artifact appears to corroborate the
expected result: pixels cannot pass, strengthen, or substitute for a logical
assertion.

If the required hook does not exist, add the narrow renderer-free hook or contract.
Do not use visual observation to infer legality, occupancy, payment, damage, decisions,
settlement, authority release, turn order, persistence, launch identity, or
determinism. A static frame does not prove motion or control feel.

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
| Spell resolution | Impact/content admission, radial clipping consumers, area queue, paid batch correlation, authority hold, terrain phase order, unsupported settlement/adoption, and typed freeze | Pure/owning crates plus one renderer-free `hex_game` composition target over real map/units/perception/combat plugins; never `hex_ui`, `gameplay_app`, renderer, viewport, or UI snapshot support | Exact values, components, resources, messages, positions, correlation evidence, and bounded transaction state | `python3 tools/test_scope.py run spell_resolution_contracts` | 60 s per test |
| Deterministic simulation | Multi-turn composition, tempo profiles, 3v3/6v6, canonical summaries and bounded no-progress | `hex_combat_core` over `hex_core` + `hex_lattice`; never Bevy App, `hex_test_support`, renderer, viewport, wall clock, asset server, ECS entity, perception implementation, or map generator | Full `CombatRunSnapshot` equality across two runs plus named metric assertions | `python3 tools/test_scope.py run simulation` | 60 s |
| Game/UI behavior | Pure Main Menu, Campaign, Sandbox, Creator, guided deployment, HUD visibility/Main View, and input-binding transitions plus Bevy wiring, persistence, inspection, exact terrain placement, suppression, and outcome lifecycle | `hex_gameplay_model` for route/draft/deployment/launch/HUD truth; `hex_core` for stable input actions; `hex_ui` for rendering; `hex_game` with default-off `test-support` only for Bevy lifecycle | Pure state equality, `GameplayStateSnapshot` for authority facts and explicit presentation-adapter observations, and `UiTreeSnapshot` for presentation structure | `python3 tools/test_scope.py run app` | 60 s |
| Visual smoke | Layout, legibility, overlap, responsive composition and presentation regressions | Release-shaped game with `visual-walk`; no `dev` or `test-support` | Reviewed frames plus the human motion/feel walk | Run the one scoped gameplay walk through `/visual-walk` | At most 10 reviewed gameplay frames |
| Soak/performance | Long stalemates, stress corpora, bounded retention and performance | The scheduled stress workflow | Typed completion/timeout, fingerprints, timing and memory bounds | `.github/workflows/stress.yaml` | Scheduled/manual only |

The required gameplay CI job publishes separate JUnit and timing evidence for its
selected partitions, including the dedicated spell-resolution concern when applicable.
The residual workspace job keeps all other non-map packages
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
optional development controls and clock coverage without allowing those surfaces to
alter or mask the primary matrix.
`hex_game/tests/game_content_contracts.rs` is the separately selectable shared
shipped-content seam, and the `hex_game` library target retains inline
scenario/loading contracts that require private composition details. Those shared
targets stay in the residual gate. The three packages set `autotests = false` and
declare their integration targets explicitly, so adding a helper file cannot silently
create a new expensive binary or escape its concern selector.

PR #180 added one other explicit target,
`hex_game/tests/spell_resolution.rs`, solely for the cross-crate spell transaction.
It builds a minimal deterministic gameplay state with the real map, units, perception,
and combat plugins. It does not install `AppPlugin`, a renderer, viewport,
`hex_ui::UiPlugin`, `HeadlessUiPlugin`, or test-support UI. Its tiny authored fixture
proves phase composition and exact authority state; it is not a procedural V3 seed
corpus or a presentation test. Because `hex_game` disables automatic integration-test
discovery, the wave registered this target explicitly and routed it only to
`spell_resolution_contracts`.

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

### Spell-resolution wave waiver

PR #180 used an explicit one-wave maintainer waiver for a gameplay-only change
whose authorities are narrower than the repository's ordinary final-wave gate. Its
automated gameplay evidence is exactly:

```sh
python3 tools/test_scope.py run trajectory_contracts
python3 tools/test_scope.py run spell_resolution_contracts
```

`trajectory_contracts` proves the shared supercover, canonical radial clipping,
endpoint behavior, `None` preservation, conservative grazes, vertical/stacked cases,
and full-truth-versus-faction-known privacy. `spell_resolution_contracts` proves:

- Impact schema, element resolution, fingerprinting, Fireball content, payment and
  refusal boundaries;
- authored-effect/stable-`UnitId` area Disable/Burn, friendly fire, queued defender
  answers, and the independent combat-authority hold;
- monotonic multi-batch allocation, out-of-order valid answers, all valid rejections,
  correlation freeze, teardown/re-entry, and no optimistic release;
- refreshed occupancy/movement, deterministic simultaneous settlement, authority
  adoption, no-landing freeze, and the reserved terrain-phase order;
- the ten pure `hex_core::terrain_impact` contracts plus the real map producer seams
  `terrain_protocol_orders_reserved_phases_before_perception` and
  `overkill_is_capped_and_empty_voxels_report_no_material`; and
- the explicit renderer-free `hex_game/tests/spell_resolution.rs` composition target.

The two named map tests exercise the #175 producer and shared schedule directly; they
do not select the map-generation corpus. Existing non-UI regression closures for
spell validation/fingerprints, cast payment/refusal/construction, command/authority/
turn behavior, and occupancy/movement also belong in
`spell_resolution_contracts`. A focused selector regression may run when the waiver
manifest or routing changes, but it does not authorize any broader application or UI
partition.

Before executing that concern, the selector lists and compares all three exact
partitions against the reviewed identities: 56 domain tests, two real-map seam tests,
and seven renderer-free game-consumer/composition tests. A renamed, removed, newly
captured, or zero-match filter fails the concern instead of silently shrinking or
widening its evidence.

The following omissions are **WAIVED**, not passed, green, N/A, or silently skipped:

| Omitted gate | Why it cannot exercise this wave's changed authority |
|---|---|
| `hex_ui` and `hex_game/tests/gameplay_app.rs` | The wave changes two thin gameplay consumers—Creator deployability and semantic casting-preview clipping—but no UI model, widget, layout, focus, persistence, or broad application lifecycle authority. The content/trajectory contracts and renderer-free composition target cover those policies directly |
| UI snapshots and the automated visual walk | The preview's gameplay voxel set changes, but widget/layout/rendering mechanics do not. Trajectory and composition hooks prove the set and downstream state directly; screenshots and visual walks have no authority over batch correlation, stable ordering, settlement, or release |
| Deterministic combat simulation | The renderer-free reducer cannot execute the ECS/world impact and settlement adapter; focused authority-hold tests cover the reducer seam it does own |
| V3/procedural map corpora | The wave changes no generator, world content, G/H schema, or map implementation; two exact real-map producer tests cover the consumed seam |
| Residual workspace corpus | Its unrelated owner/application binaries cannot compile or exercise the changed transaction authority |

Format, dependency policy, strict workspace Clippy, warnings-denied docs, and the
default-feature shipping release build remain required non-test checks. The selector
must apply this exception only when an explicit waiver manifest is itself in the diff
and every changed path matches its allow-list. Unknown paths, invalid configuration,
an empty diff, and ordinary future changes remain fail-closed. The same declaration
may route only PR #180 from `wave/spell-resolution` to `dev` and the exact #180 merge
diff pushed to `dev`; it does not apply to any `main`-target PR, any push to `main`, or
a later unrelated push.

The waiver is invalid as soon as the candidate changes UI models, widgets, layout,
rendering, `hex_game` lifecycle outside the two exact thin consumers and dedicated
headless adapter, `hex_map` implementation, the G/H schema or world response policy,
procedural content, or any behavior the two named concerns cannot exercise.
Invalidation restores the complete ordinary gate; it is not permission to expand the
allow-list after behavior has expanded.

This wave has no changed presentation, native-input, motion, or feel claim. Its final
candidate therefore used a verified-maintainer exact-head N/A sign-off naming the
renderer-free hook closure and the reason no visual oracle applied. A Creator →
Sandbox launch may remain a diagnostic, but neither that route nor a screenshot is
acceptance evidence for the logic proved by the contracts.

Pull-request CI applies the selector directly and publishes the decision plus timing
evidence and JUnit for each nextest-backed concern. Pushes to `dev` or `main`
ordinarily promote the decision to the complete integration gate regardless of changed
paths. The explicit, allow-listed one-wave exception above applied to PR #180 and its
exact merge diff on `dev`; `main` promotions and later unrelated pushes remain
complete. Unknown paths, invalid configuration, and empty diffs still fail closed. A
waived gate never contributes green evidence or substitutes for the exact-head
verified sign-off classification.

GitHub deliberately enters each required job even when all of that job's expensive
steps are conditional, so a green job shell can mean only that scope detection and the
non-waived checks completed. The `test-scope-decision` artifact is authoritative: its
omitted partitions remain **WAIVED**. The verified-maintainer N/A was recorded against
the exact final head before merge; neither state was converted to a pass by a green
shell.

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
frozen launch/retry identity, effective HUD presentation, typed Main View, and the
disclosure-authorized inspection subject. Its explicitly
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

`hex_gameplay_model` owns renderer-free Main Menu, Campaign, Sandbox, Creator, and HUD
transitions. It may depend on `bevy_ecs` derive support and `hex_core`, but not on
assets, combat, units, game, map, world, perception, or the Bevy facade. It is the
oracle for route and Back behavior, pending/committed map and resolved-seed edits,
fixed six-slot roster identity/order/duplicates, launch-blocker priority, guided
Party-then-Enemies deployment order, exact placement occupancy, reselection, Undo and
Review, exact Retry identity, Campaign slot identity, typed Creator destinations,
bounded edit history, component preference/eligibility/master/phase resolution,
Compact transient surfaces, and forced Main View ownership. Widget systems emit typed
actions into that model and apply effectful results; they do not duplicate those
decisions.

The application adapter proves that one ordinary `HexTile` click outside the catalog's
hidden staging regions is accepted when canonical walker footing admits its exact
`TilePos`, while invalid footing and occupied surfaces refuse without advancing.
Headless presentation evidence separately proves that Deployment removes every
ordinary HUD surface from layout, focus, scrolling, and picking and leaves the compact
task card reachable. The scoped visual walk places at least one Party and one Enemy
character before capturing Review; pixels never substitute for the typed placement or
frozen-launch assertions.

HUD adapter evidence exhausts saved preferences, contextual eligibility, master and
phase suppression, Standard/Compact behavior, temporary summons, and required
decisions without using pixels. It drives configurable bindings through the real
highest-priority capture seam, including Escape cancellation, Swap/Cancel conflicts,
conflict-safe row restore after a swap, focused Enter/Space refusal, confirmed Restore
All, shipping/development action-inventory separation, and schema-v3 restart.
Cross-build restart coverage also proves a development-only binding is deterministically
rehomed when a shipping edit later occupies its chord without rewriting player actions.
Inspection cases prove
first activation centers one disclosed subject, repeated activation opens Character
Main View, Character camera follows, and selection, turn, caster, command ownership,
formation, and unobserved hostile location remain unchanged.

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

Screenshots answer static presentation questions: whether camera framing and
occlusion look right; whether controls fit, labels read, dense rosters remain legible,
regions overlap, focus remains visible, and a responsive surface adapts; and whether
rendered-map geometry, materials, lighting, cutaways, seams, and composition look
right. Video and human checks answer camera-motion, native-input, animation,
control-feel, and taste questions. A visual artifact can judge the rendering of a
hook-established state, but it does not prove occupancy, action accounting, tempo,
determinism, state restoration, Campaign persistence, or launch/retry identity.

This boundary is absolute whenever a typed gameplay hook can express the claim. A
frame may help navigate or diagnose a presentation symptom, but it must not be cited
as corroborating logical evidence. Add a missing hook instead.

A scoped gameplay acceptance run reviews at most ten deterministic Bevy image-target
frames: minimal Exploration, player turn, hostile turn, Character Main View, Required
Decision, aiming/Action Bar, Activity, custom visibility, master-hidden Required
Decision, and one targeted Compact or 4K/200% duplicate.
Default-off presentation fixtures create visual state without solving combat.
Before any capture, the live `UiTreeSnapshot` oracle rejects zero-area targets, inherited
clipping, off-canvas placement, unreachable scroll content, overlap, missing labels,
invalid focus order, and targets below 44×44. Authored presentation fixtures apply
named composition contracts, including independently visible Standard/Wide
components, at most one typed Main View, Compact map-only presentation, exactly one
temporary full-screen Compact task, and forced decision ownership. The same contract
drives real wheel and Tab/Shift-Tab input through the declared scroll owner. The
scoped gameplay script uses no combat-solving steps; generic world-owned walks retain
their existing driver verbs.

Forest, Waterfall, map review, and Alberto's map captures are outside this budget and
remain unchanged.

## Manual runtime sign-off

The automated Bevy visual walk, an agent frame review, video, and a named human
playtest are distinct presentation/experiential evidence. Frames judge static camera,
UI, and rendered-map presentation; video and the human route judge motion, input,
control feel, and taste. None proves gameplay or exact world logic that typed hooks can
express. Gameplay runtime-surface PRs record the human PASS in the structured PR fields
together with the full final head SHA, reviewer, date, and exact route exercised. When
a candidate has no changed presentation, native-input, motion, feel, or visual-script
claim, a maintainer records an exact-head N/A waiver even if the renderer-free runtime
logic changed. The waiver names that maintainer's GitHub login and the authoritative
hook closure; the workflow requires the same maintain/admin account to trigger the
check. A later push makes either form of evidence stale. On a draft, the `Current-head
manual runtime sign-off` workflow may remain green because enforcement is deferred;
that is not a PASS or waiver. The required check must validate the new head before the
PR can leave draft.

Source-lane PRs targeting `wave/*` defer this gate because they are not independently
shippable runtime candidates. The combined wave PR targeting `dev` must carry the
correct exact-head classification: a named human PASS for affected presentation or
experiential surfaces, otherwise a verified-maintainer N/A naming the logic hooks.
Merging a lane into a wave never inherits an earlier classification.

Draft PRs may omit the sign-off while implementation is moving. A gameplay PR may not
be marked ready or merged with a placeholder, a blocked result, a different commit, or
an agent named as the human reviewer. An N/A waiver is available only when no changed
claim requires visual or experiential judgment and only to a verified maintainer; it
does not convert a known gameplay failure into a pass. Release promotions still
require the named-human presentation PASS over the complete shipped build.

HUD sign-off includes every default shortcut, a custom visibility combination,
master-hidden one-surface summons, Map centering and Character follow, a blocking
decision, deployment/outcome suppression, Compact map-only presentation, one binding
conflict resolved through Swap, and the post-restart presentation of hook-proven
preference state. The reviewer confirms that ordinary hidden components leave no
drawer, handle, tooltip, or hit region behind; typed restart contracts prove exactly
what persisted.

## Anti-patterns

- **Noun-only assertions:** proving an ID, label, node, deterministic case name, or route exists
  without exercising the behavior it promises.
- **Screenshots as logic:** using pixels, rendered frames, video, or human visual
  observation to infer or corroborate any claim available through gameplay hooks,
  including movement budgets, exact occupancy, Channel actions, outcomes,
  determinism, or launch identity. Add the missing hook instead.
- **Fixture-description drift:** prose claims a matrix or state the owned deterministic case
  does not construct and verify directly.
- **All-feature runtime pollution:** enabling inspector/review/test features in a
  visual run and treating the resulting UI as shipping evidence.
- **Frame-sensitive balance evidence:** using an input heuristic, viewport-dependent
  timing, animation settling, or wall-clock timeout as a combat balance benchmark.
- **Unbounded settling:** polling until a test hangs. Every settle/run has an explicit
  frame or turn bound and reports exhaustion as a typed failure/result.
