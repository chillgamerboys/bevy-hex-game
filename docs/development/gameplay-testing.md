# Gameplay testing contract

This document defines how gameplay-owned behavior is tested. It is a contract, not a
catalog of every test. The goal is to choose the cheapest authoritative oracle for
each concern and reserve rendered frames for facts only a renderer or a person can
judge.

This partition does not own `hex_map`, the map review runner, Forest or Waterfall
walks, perception/map stress suites, or their acceptance criteria. Cross-owner
scenario binaries continue to run in the residual workspace suite.

## Concern partitions

| Concern | Owns | Allowed dependencies | Oracle | Local command | Ordinary budget |
|---|---|---|---|---|---:|
| Pure rules | Value objects, lattice transformations, compact AI policy | Owning crate dependencies only | Return values and immutable state | `cargo nextest run --workspace --all-features --cargo-profile ci --profile gameplay-rules` | 60 s total |
| ECS contracts | Focused commands, effects, movement, turns, occupancy and Channel seams | `hex_test_support`, then the owning gameplay crate dependencies | Components, resources, messages and exact positions | `cargo nextest run --workspace --all-features --cargo-profile ci --profile gameplay-contracts` | 60 s per test |
| Deterministic simulation | Multi-turn composition, tempo profiles, 3v3/6v6, canonical summaries and bounded no-progress | Combat dependencies plus `hex_test_support`; never a renderer, viewport, wall clock, or map generator | Full `CombatRunSnapshot` equality across two runs plus named metric assertions | `cargo nextest run -p hex_combat --test simulation --cargo-profile ci --profile gameplay-simulation` | 60 s |
| Game/UI behavior | Combat Lab report modes, comparison selection, re-entry, identity and drawer lifecycle | `hex_game` with default-off `test-support` | Immutable observation snapshots and projected text/state | `cargo nextest run -p hex_game --features test-support --test gameplay_app --cargo-profile ci --profile gameplay-app` | 60 s |
| Visual smoke | Layout, legibility, overlap, responsive composition and presentation regressions | Release-shaped game with `visual-walk`; no `dev` or `test-support` | Reviewed frames plus the human motion/feel walk | Run the one scoped gameplay walk through `/visual-walk` | At most 10 reviewed gameplay frames |
| Soak/performance | Long stalemates, stress corpora, bounded retention and performance | The scheduled stress workflow | Typed completion/timeout, fingerprints, timing and memory bounds | `.github/workflows/stress.yaml` | Scheduled/manual only |

The required gameplay CI job publishes separate JUnit and timing evidence for the
first four partitions. The residual workspace job keeps all other packages and the
unchanged map/game-world contract binaries under the existing feature set, CI
profile, and timeout.

## Shared support boundary

`hex_test_support` is test-only infrastructure. Its Cargo dependency ceiling is
Bevy, `hex_core`, and `hex_assets`. It may construct deterministic minimal apps,
advance fixed time, settle within a bound, publish synthetic shared surfaces, load
fixture palettes/settings/content, and observe shared positions/resources.

It must never depend on `hex_units`, `hex_combat`, `hex_game`, `hex_map`,
`hex_world`, or `hex_perception`. Such an edge would let the fixture reconstruct
private gameplay or world truth and turn a test helper into a second authority.
Owning tests add their system under test on top of the support app.

`hex_game` exposes its headless UI observation harness only behind `test-support`.
That feature returns immutable facts and formatted canonical projections; it does
not expose mutable screen resources. The shipping binary has no feature dependency
on the harness.

## Simulation evidence

One `CombatCase` freezes typed unit/controller inputs, profile, arena and run bounds.
One run produces a `CombatRunSnapshot` containing the canonical `CombatSummary`,
summary, command, and full-transcript fingerprints, typed outcome or bounded
no-progress termination, turn state, lattice summaries, and exact `TilePos`
positions.

Every acceptance case runs twice from fresh state and requires complete snapshot
equality before checking named metrics. A bounded no-progress result is data. It is
never silently converted to success, inferred from a frame, or described as a
terminal outcome.

## Visual evidence policy

Screenshots answer presentation questions: whether controls fit, labels read, dense
rosters remain legible, drawers overlap, and a responsive surface adapts. They do not
prove occupancy, action accounting, tempo, determinism, state restoration, or report
identity.

A scoped gameplay acceptance run reviews no more than ten frames. Capture both
resolutions only for surfaces whose responsive behavior is under review. Wave 7
allocates that budget to Rules, edited Deployment, live HUD/drawer, one dense report
mode, Compare, 6v6 readability, and the smallest number of responsive duplicates
needed to judge those surfaces.

Forest, Waterfall, map review, and Alberto's map captures are outside this budget and
remain unchanged.

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
