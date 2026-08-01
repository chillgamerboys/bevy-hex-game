# Gameplay testing contract

This document defines how gameplay-owned behavior is tested. It is a contract, not a
catalog of every test. The goal is to choose the cheapest authoritative oracle for
each concern and reserve rendered frames for facts only a renderer or a person can
judge.

This partition does not own `hex_map`, the map review runner, Forest or Waterfall
walks, perception/map stress suites, or their acceptance criteria. Cross-owner
scenario binaries continue to run in the residual workspace suite. The unchanged
`hex_map` corpus runs beside it in the world-owned map shard whenever that residual
concern is selected.

## Concern partitions

| Concern | Owns | Allowed dependencies | Oracle | Local command | Ordinary budget |
|---|---|---|---|---|---:|
| Pure rules | Value objects, lattice transformations, compact AI policy | Owning crate dependencies only | Return values and immutable state | `python3 tools/gameplay_scope.py run rules` | 60 s total |
| ECS contracts | Focused commands, effects, movement, turns, occupancy and Channel seams | `hex_test_support`, then the owning gameplay crate dependencies | Components, resources, messages and exact positions | `python3 tools/gameplay_scope.py run contracts` | 60 s per test |
| Deterministic simulation | Multi-turn composition, tempo profiles, 3v3/6v6, canonical summaries and bounded no-progress | `hex_combat_core` over `hex_core` + `hex_lattice`; never Bevy App, `hex_test_support`, renderer, viewport, wall clock, asset server, ECS entity, perception implementation, or map generator | Full `CombatRunSnapshot` equality across two runs plus named metric assertions | `python3 tools/gameplay_scope.py run simulation` | 60 s |
| Game/UI behavior | Pure Combat Lab/Creator transitions plus Bevy wiring, re-entry and drawer lifecycle | `hex_gameplay_model` for state/launch/navigation truth; `hex_ui` for rendering; `hex_game` with default-off `test-support` only for Bevy lifecycle | Pure state equality, `GameplayStateSnapshot` for canonical facts, and `UiTreeSnapshot` for presentation structure | `python3 tools/gameplay_scope.py run app` | 60 s |
| Visual smoke | Layout, legibility, overlap, responsive composition and presentation regressions | Release-shaped game with `visual-walk`; no `dev` or `test-support` | Reviewed frames plus the human motion/feel walk | Run the one scoped gameplay walk through `/visual-walk` | At most 10 reviewed gameplay frames |
| Soak/performance | Long stalemates, stress corpora, bounded retention and performance | The scheduled stress workflow | Typed completion/timeout, fingerprints, timing and memory bounds | `.github/workflows/stress.yaml` | Scheduled/manual only |

The required gameplay CI job publishes separate JUnit and timing evidence for the
first four partitions. The residual workspace job keeps all other non-map packages
and cross-owner game/world contract binaries under the existing feature set, CI
profile, and timeout. The same residual decision also enables the separately
budgeted, unchanged `hex_map` test shard.

## Integration target topology

Expensive Bevy links are explicit Cargo targets rather than one binary per source
module. `hex_units/tests/contracts.rs` links every focused unit/ECS contract once;
`hex_combat/tests/contracts.rs` does the same for combat, while
`hex_combat_core/tests/simulation.rs` remains the sole multi-turn simulation target.
Their concern modules live below a directory with the target name so ownership stays
readable without adding another linker invocation.

The app partition runs `hex_gameplay_model` and `hex_ui` inline tests together with
`hex_game/tests/gameplay_app.rs`, the default-off gameplay-owned application target.
Before enabling `test-support`, it also compiles the default-feature `hex_game`
library-test target in package isolation. That preflight prevents workspace feature
unification from hiding a test-only field or import that the ordinary shipping-shaped
crate cannot compile.
`hex_game/tests/game_content_contracts.rs` is the separately selectable shared
shipped-content seam, and the `hex_game` library target retains inline
scenario/loading contracts that require private composition details. Those shared
targets stay in the residual gate. The three packages set `autotests = false` and
declare their integration targets explicitly, so adding a helper file cannot silently
create a new expensive binary or escape its concern selector.

## Scope selection

The concern filter is not the Cargo selector. Cargo packages, targets, and features
must be selected first so a rules-only change cannot compile the renderer or unrelated
owner packages and then merely filter their tests out.

`.config/gameplay-test-scopes.json` is the single machine-readable authority for
canonical concern commands and changed-path classification. Inspect the proposed
closure for a branch with:

```sh
python3 tools/gameplay_scope.py plan --base origin/dev --head HEAD
```

For a source PR inside a wave, substitute that PR's wave base, for example
`--base origin/wave/8-gameplay-foundation`. Scoping against `dev` from a cumulative
wave source would correctly select every earlier lane, but it would not be a useful
edit-loop scope.

The selector unions concerns across changed files. Shared vocabulary, validation
infrastructure, world-owned paths, an unknown path, an invalid manifest, or an empty
diff fail closed to the complete gate. A narrow rules graph additionally runs:

```sh
python3 tools/gameplay_scope.py check-graph rules
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

Pull-request CI applies the selector directly and publishes the decision plus
per-concern JUnit and timing evidence. Pushes to `dev` or `main` forcibly promote
the decision to the complete integration gate, regardless of changed paths. Final
wave/release candidates likewise run the complete gate before the exact-head manual
sign-off; unknown paths, invalid configuration, and empty diffs also fail closed to
that same result.

For a gameplay-only change that does not modify `hex_core`, shared application
composition, scenario/loading lifecycle, or a published world seam, V3/map corpora
are not a relevant PR oracle. Their source, selection, and acceptance criteria remain
unchanged and continue on their owning changes and broad gates.

## Shared support boundary

`hex_test_support` is test-only infrastructure. Its Cargo dependency ceiling is
Bevy, `hex_core`, and `hex_assets`. It may construct deterministic minimal apps,
advance fixed time, settle within a bound, publish synthetic shared surfaces, load
fixture palettes/settings/content, and observe shared positions/resources.

It must never depend on `hex_units`, `hex_combat`, `hex_game`, `hex_map`,
`hex_world`, or `hex_perception`. Such an edge would let the fixture reconstruct
private gameplay or world truth and turn a test helper into a second authority.
Owning tests add their system under test on top of the support app.

`hex_game` exposes `GameplayStateSnapshot` only behind `test-support`. It observes
canonical screen/phase/mode, turn and budgets, pending decisions, command state,
exact positions, lattice summaries, outcome, and report fingerprints. `hex_ui`
separately exposes `UiTreeSnapshot` for visible regions, focus/accessibility,
computed bounds/overflow, and action priority. Neither feature exposes mutable
screen state, and the shipping binary has no feature dependency on either harness.
`hex_game` also re-exports `HeadlessUiPlugin`, which installs the real presentation
schedules on a synthetic window without Winit or a renderer. Use it only with
`UiTreeSnapshot` presentation assertions; canonical behavior still comes from
`GameplayStateSnapshot` and the owning rules or simulation target.

`hex_gameplay_model` owns renderer-free Combat Lab and Creator transitions. It may
depend on `bevy_ecs` derive support and `hex_core`, but not on assets, combat, units,
game, map, world, perception, or the Bevy facade. It is the oracle for roster
editing, report selection/deletion, report presentation mode, exact Retry/Tune/Copy
routing, Creator navigation identity, and bounded edit history. Widget systems emit
typed actions into that model and apply effectful results; they do not duplicate
those decisions.

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

Screenshots answer presentation questions: whether controls fit, labels read, dense
rosters remain legible, drawers overlap, and a responsive surface adapts. They do not
prove occupancy, action accounting, tempo, determinism, state restoration, or report
identity.

A scoped gameplay acceptance run reviews no more than ten frames. Six deterministic
offscreen frames cover Creator validation, Combat Lab setup/deployment, aiming and a
disabled action, the live statistics drawer, and the 4K dense Compare report. Four
native macOS window-only frames cover cold Compact title, Standard settings,
fullscreen Auto gameplay, and a restarted 200% required decision. Default-off
presentation fixtures create visual state without solving combat. Before any
capture, the live `UiTreeSnapshot` oracle rejects zero-area targets, inherited
clipping, off-canvas placement, unreachable scroll content, overlap, missing labels,
invalid focus order, and targets below 44×44. The runner contains no combat-solving
verbs.

Forest, Waterfall, map review, and Alberto's map captures are outside this budget and
remain unchanged.

## Manual runtime sign-off

The automated walk, an agent-operated native runtime review, and a named human
playtest are distinct evidence. Gameplay runtime PRs record the human result in the
structured PR fields together with the full final head SHA, reviewer, date, and exact
route exercised. A later push makes that evidence stale and the required
`Current-head manual runtime sign-off` check fails until the new head is played.

Source-lane PRs targeting `wave/*` defer this gate because they are not independently
shippable runtime candidates. The combined wave PR targeting `dev` must carry the
named human sign-off for its exact final head; merging a lane into a wave never
inherits, substitutes for, or weakens that release gate.

Draft PRs may omit the sign-off while implementation is moving. A gameplay PR may not
be marked ready or merged with a placeholder, a blocked result, a different commit,
or an agent named as the human reviewer. A maintainer waiver describes an
infrastructure exception; it does not convert a known gameplay failure into a pass.

## Anti-patterns

- **Noun-only assertions:** proving an ID, label, node, fixture name, or tab exists
  without exercising the behavior it promises.
- **Screenshots as logic:** reading pixels or frame timing to infer movement budgets,
  exact occupancy, Channel actions, outcomes, determinism, or report fidelity.
- **Fixture-description drift:** prose claims a matrix or state the owned fixture
  does not construct and verify directly.
- **All-feature runtime pollution:** enabling inspector/review/test features in a
  visual run and treating the resulting UI as shipping evidence.
- **Frame-sensitive balance evidence:** using an input heuristic, viewport-dependent
  timing, animation settling, or wall-clock timeout as a combat balance benchmark.
- **Unbounded settling:** polling until a test hangs. Every settle/run has an explicit
  frame or turn bound and reports exhaustion as a typed failure/result.
