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

The next integrated checkpoint `41e860e` contains bounded dry/support-aware creature
steering and crater recovery, fixed Dragon retreat destinations, and admitted
eye/center visibility (`002dc6b`). It preserves the accepted Shadow policy and all
initial creature numbers. Focused gameplay checks pass 146/146; the closer observer
camera/footer candidate passes 64 application checks with two explicit capture
tests ignored. Scoped strict gameplay and application lint pass.

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

## Optimized comparison before numeric calibration

Clean `41e860e19c79c968190412a5dab0469f4bacd6d5` ran 96 actual Duel battles: eight
seeds, six pairings, both side/actor-order assignments and a 90-second limit. The
optimized native test harness reads the same `assets/config/arena.ron` as the app;
the receipt confirms it matches the unchanged defaults. Invalid/incomplete rounds
are not admitted. These are simulation measurements without a renderer.

| Pair | First wins | Second wins | Timeouts |
|---|---:|---:|---:|
| Shadow / Dragon | 14 | 0 | 2 |
| Shadow / 5 Goblins | 6 | 8 | 2 |
| Shadow / Shaman party | 13 | 1 | 2 |
| Dragon / 5 Goblins | 8 | 8 | 0 |
| Dragon / Shaman party | 0 | 16 | 0 |
| 5 Goblins / Shaman party | 9 | 6 | 1 |

The maximum measured tick was 5.943 ms, with none above 8.333 ms. Final winning-tick
terrain requests were not separately flushed/measured by this harness version;
terminal publication timing remains an explicit evidence gap being repaired.
The 96-row receipt's setup/result fields were produced by the real harness; a
review found the Python validator should cross-check more of those fields rather
than only pair coverage. No incorrect actual setup was established by that review.

These are still defect-finding results. The Dragon delivered only 35 total damage
over its 16 Shadow rounds. Shaman self-damage totals 554 across its 48 rounds.
Five Goblins are close enough for the initial rough target; their stats stay fixed.
Dragon/Goblin decisions do not consume random values, so their eight seeds repeat
the same two side-dependent trajectories. Their apparent 50% is not eight
independent balance samples. Fort comparisons supply a separate terrain layout.

Test-only `9dbabf5` adds exact release/self-hit transitions. In the retained seed-8
Shaman-left/Goblins-right trace, a Fireball released at tick 295 detonates at its
previous position at tick 297 while its caster moves away. Owner-clearance admission
tests the current caster pose, but the subsequent sweep tests the previous pose;
the just-admitted shot still overlaps that previous capsule. This identifies a
false self-hit at the start of the sweep. Repair must preserve legitimate returning
projectile hits and splash self-damage. Separately, Shaman spread is mistakenly
added to a normalized direction while the accepted Shadow uses a positional error;
the configured two-times multiplier should retain the same units. An independent
geometric fixture also exposes cone attacks rejecting an exposed body flank after
one nearer contact is obstructed. Repairs and repeated comparisons are pending.

Retained task evidence: `outputs/arena-original-native-01/{receipt.json,battles.log}`,
`outputs/arena-original-native-summary-01.json`,
`outputs/arena-self-hit-trace-01/{receipt.json,battles.log}`, and
`outputs/shaman-self-hit-transitions-01.json`. Traced timing is diagnostic only.

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

The fresh `41e860e...-spectator-close-02` pack contains fourteen 1600 by 900 raw
views, including close Fort/Duel pairs at opposite azimuths. The coordinator and
independent reviewer inspected every original before the contact sheet: scoped
static PASS. Close views distinguish both team colors and show the player-sized
Goblins, Shaman, Shadow and long low Dragon. The footer is centered and padded.
Menus, HP labels, opaque terrain and transparent panels are readable. Rendered
result labels match the typed receipts: Fort Team 2 at tick 505, Duel Team 1 at
tick 619. Raw images, original receipts, contact sheet and both reviews remain in
the ignored exact-source pack. This closes the original framing gap.

Human native camera motion, collision feel, creature animation, telegraph readability
in motion, and 20–30 Fort encounters remain **HUMAN-MOTION-PENDING**. Paired machine
matchups and future holdout seeds cannot establish human win rates.
