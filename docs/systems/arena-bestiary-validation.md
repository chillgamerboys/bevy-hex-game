# Spectator battles and bestiary validation

Status: original-group calibration in progress. Golem, Ember Wisp and Worm remain
queued in that order. The [local wave](../planning/waves/arena-bestiary/manifest.md)
and [approved requirements](../planning/waves/arena-bestiary/plan.md) govern this work.
The accepted human/Shadow reference remains `127d1ce`; no remote merge is authorized.
The original map/creature milestone has [separate evidence](arena-encounters-validation.md).

## Spectator checkpoint

Combined source `90e2e4d9a6196dc28c25246b593981111047e02f` includes world deployment
`56db778`, gameplay teams/observations `469cac0`, observer presentation `6881eb2`,
and the real-world battle harness. Results at this checkpoint:

- 13 world arena tests and scoped strict world lint passed.
- 136 gameplay tests and strict arena lint passed, including three frozen Duel
  fixtures, arbitrary teams, hidden-history isolation, source identity after death,
  whole-team results, timeout admission, and monster actor zero.
- 60 application arena tests passed, with two explicit capture/performance tests
  ignored. Strict application lint and eight launcher argument guards passed.
- Two battle integration tests passed: all 32 original map/roster deployment
  combinations, reset-time setup changes, and return to ordinary Duel.
- Native `dev,arena-prototype` build passed in 4m 30s.

The complete repository-selected gate, final bestiary native build, expanded
capacity timing and final presentation matrix remain pending.

## Actual matchup pilot

The first pilot uses ordinary actors, abilities, health and terrain publication at
120 Hz. One seed, six distinct roster pairings and both side/actor-order assignments
produce 12 rounds on Duel, with a 60-second bound. It ran under the CI profile;
its CPU numbers are diagnostic, not native performance evidence.

| Pair | First wins | Second wins | Timeouts |
|---|---:|---:|---:|
| Shadow / Dragon | 2 | 0 | 0 |
| Shadow / 5 Goblins | 1 | 1 | 0 |
| Shadow / Shaman party | 0 | 0 | 2 |
| Dragon / 5 Goblins | 1 | 0 | 1 |
| Dragon / Shaman party | 0 | 0 | 2 |
| 5 Goblins / Shaman party | 1 | 1 | 0 |

This is a defect-finding pilot, not a balance result. Dragons dealt no damage in
the two Shadow rounds. The five timeouts and traces exposed runaway retreat goals,
unsafe edge pursuit, crater-lip recovery gaps, covered Shaman standoff, and an
eye-versus-center visibility mismatch. Fixing those precedes numeric calibration;
the original enemy HP, damage and cooldown hypotheses are still unchanged.

Test-only checkpoint `4af96a4` adds optional half-second `ARENA_BATTLE_TRACE` records
of physical actors, valid volume, dated party knowledge, attacks, charging and
statistics. Eight 30-second diagnostic rounds passed their completion bound. These
traces are local diagnostics; they are never supplied to creature decision code or
the ordinary human HUD.

Retained task evidence: `outputs/spectator-matchup-smoke-ci-01.json`,
`work/spectator-matchup-smoke-ci-01.log`,
`work/spectator-battle-diagnostic-ci-01.log`, and
`taskwork/spectator-timeout-diagnosis-initial.md`.

## Native smoke measurements

At `90e2e4d`, two windowless ordinary seeded battles ran serially, with no synthetic
health or input and no concurrent build. Both ended naturally before 30 seconds.
First 120 samples are excluded. These short rounds do not establish worst-case
capacity or replace the all-ten-enemy Seven Regions stress measurement.

| Map / roster | Samples | Tick p95 / p99 / max (ms) | Publication max (ms) | Render ready (ms) |
|---|---:|---:|---:|---:|
| Duel, Shadow / Dragon | 502 | .420 / 3.201 / 3.519 | 3.215 | 490.8 |
| Fort, Goblins / Shaman party | 603 | .484 / .850 / 1.467 | 1.467 | 638.1 |

No sampled simulation tick exceeded 8.333 ms. Main application frame interval p99
was 20.743 ms on Duel and 20.661 ms on Fort. Those start-to-start wall intervals
include render-submission waits; they are not GPU duration or vsync FPS.
Full receipts and setup identities are in the ignored exact-head
`90e2e4d...-spectator-performance-01` pack and task
`outputs/spectator-native-performance-90e2e4d.json`.

## Static review and remaining experience checks

The clean `90e2e4d` matrix contains ten observer views plus three ordinary player
controls, all at 1600 by 900. Menus, team accounting, camera mode labels, timeout
versus winner text, whole-map framing and the ordinary human HUD are readable.
Whole-map camera distances make creature/team detail too small for that criterion;
closer views and useful initial observer framing are being added. Do not treat the
original overview pack as complete model-detail approval.

Human native camera motion, collision feel, creature animation, telegraph readability
in motion, and 20–30 Fort encounters remain **HUMAN-MOTION-PENDING**. Paired machine
matchups and future holdout seeds cannot establish human win rates.
