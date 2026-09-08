# Spectator battles and bestiary validation

Status: original-group calibration is recorded below. Golem integration is active;
Ember Wisp and Worm remain queued in that order. The [local wave](../planning/waves/arena-bestiary/manifest.md)
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

## Attack repairs and repeated native comparison

Gameplay checkpoint `543ceab` repairs start-of-step caster clearance, positional
Shaman aim spread, exposed finite-cone contacts, and physically aligned defensive
Dragon breath with visible-target tracking. All 155 focused gameplay tests pass,
including the unchanged Duel goldens; strict scoped arena lint passes. The first
run had one incorrect new negative fixture: a 3.55-unit cone could legitimately
reach an exposed capsule flank. The corrected fixture and its geometric reason
are retained; no production range was changed to satisfy that test.

Harness checkpoint `9e83d30` adds accepted-session setup checks and separate terminal
publication timing. Three real-world battle integration tests and strict application
lint pass. Eleven Python receipt guards pass. The historical 96 outcomes also pass
the stronger setup/member/result checks; their previously documented final-flush
timing limitation remains. A Git commit during the integration test changed source
bookkeeping only; the retained content-provenance receipt verifies unchanged files.

Clean combined `b7c5d60` repeats the same native 96-round comparison at unchanged
numeric values:

| Pair | First wins | Second wins | Timeouts |
|---|---:|---:|---:|
| Shadow / Dragon | 16 | 0 | 0 |
| Shadow / 5 Goblins | 6 | 8 | 2 |
| Shadow / Shaman party | 10 | 2 | 4 |
| Dragon / 5 Goblins | 0 | 16 | 0 |
| Dragon / Shaman party | 0 | 16 | 0 |
| 5 Goblins / Shaman party | 3 | 13 | 0 |

Shaman self-damage falls from 554 to **zero** across its 48 rounds, with ordinary
caster damage rules intact. Maximum living-tick CPU is 5.492 ms; maximum separately
measured terminal publication is .087 ms. The Dragon remains too weak (93 total
damage in its 16 Shadow rounds), so this is a repaired baseline, not completed
calibration. The next bounded trial raises only Dragon HP to 160 and breath range
to 6, and improves its opportunity to turn toward a nearby attacker. Shaman support
will begin when allies actually engage and remain within the existing aura range.
Goblins, human spells and Shadow policy remain unchanged.

Retained task evidence: `outputs/arena-original-native-02/{receipt.json,battles.log}`,
`outputs/arena-original-native-summary-02.json`, the `work/creature-combat-repair-*`
logs, and `work/arena-battle-integration-01` / `work/arena-battle-app-clippy-01`.

## Original-group calibration checkpoint

Clean `27157af` finishes this bounded tuning pass. Dragon HP is 220, breath damage
45 and range 6; the remaining Dragon values retain their original hypotheses.
Goblins remain player-sized with their original HP and attack numbers. Human spells,
charging and accepted Shadow policy remain unchanged. Shaman timing and statistics
are unchanged; its aura now waits for eligible engagement and its positioning follows
its frontline. Ground creatures can take verified ordinary-controller descents into
craters, and spectator Dragons search a bounded set of public deployment waypoints.
All 160 focused gameplay tests and strict arena lint pass, including the retained Duel
fixtures. The new search equivalence test varies a hidden enemy without changing decisions.

The 120-round native comparison uses seeds 1–20, both side/initiative orders and a
90-second limit. Each pair has 40 rounds:

| Shadow versus | Shadow wins | Creature wins | Timeouts |
|---|---:|---:|---:|
| Dragon | 23 | 15 | 2 |
| 5 Goblins | 14 | 24 | 2 |
| Shaman + 3 Goblins | 13 | 13 | 14 |

A separate, untuned holdout uses seeds 101–105, ten rounds per pair:

| Shadow versus | Shadow wins | Creature wins | Timeouts |
|---|---:|---:|---:|
| Dragon | 7 | 2 | 1 |
| 5 Goblins | 3 | 7 | 0 |
| Shaman + 3 Goblins | 5 | 1 | 4 |

This is rough machine calibration, not statistical equivalence or a human win-rate
claim. Shaman matchups still have substantial stalemates after cover and terrain
changes; timeouts are never counted as wins. The holdout remains reported rather than
being folded back into tuning. No original group is tuned further in this pass.

A 36-round Fort spot corpus (three paired seeds for all six pairings) establishes a
strong map effect: Dragon beats Shadow, Goblins and Shaman party in all six rounds
per pairing; Goblins beat Shadow 5–1 and Shaman party 6–0; Shaman party beats Shadow
6–0. The close courtyard starts favor immediate area and melee pressure. These small
samples include repeated deterministic creature-only trajectories and must not be
read as independent random trials or a balanced tournament ranking.

Native timing includes ordinary world publication and a separately measured final
flush. Maximum living tick was 8.799 ms in the 120-round corpus, 7.477 ms in holdout,
and 5.066 ms in Fort. The first corpus therefore includes an over-budget sample;
final combined stress testing must check repeatability rather than claiming every
tick fits 120 Hz. Final native rendering and the expanded bestiary remain pending.

Retained task evidence: `outputs/arena-original-calibration-04`,
`outputs/arena-original-fort-05`, `outputs/arena-original-holdout-06`, their companion
summary JSON files, and `work/creature-descent-search-*` logs. All three battle receipts
are COMPLETE, source-frozen at `27157af`, and consume the authored arena configuration.

## Golem gameplay checkpoint

Runtime `2570dce` adds the fixed seven-hex, five-level body, 2-unit/second grounded
movement, spherical slam and visibly charged straight laser. Initial hypotheses
were HP160; slam35/radius6.928/windup.8s/cooldown5s/impulse5; laser45 total over1s,
minimum admission12, charge2s, final.35s locked, cooldown8s. The deliberate medium
range gap remains. The sphere can damage its own supporting terrain. Human and
Shadow values and policy remain unchanged.

The full arena suite passes199/199 and strict scoped lint passes. Coverage includes
actual compound movement, side-prism hits, concave boundaries, translating bodies,
forecast agreement, mixed separation, dry bounds, unsupported footing, finite cones,
windups/caps, allied passage, barrier/terrain ordering, death, and world-boundary
beam endpoints. The retained Duel fixtures pass. The beam follows its physical
mouth under knockback while retaining the locked direction.

Independent review repaired a translated shared-edge rounding crack, point-blank
laser terrain contact, and center-to-mouth elevation parallax; dedicated regressions
retain those cases. A decorative face mounting correction is being integrated
separately. The combined working tree then passes five real-world battle tests
(including Golem deployment on Fort/Duel, Fort's player start outside activation
range, and reset into Seven Regions), 69 application tests, strict application lint
and17 Python guards. The source-recording receipt confirms no source changes during
those checks. Decorative face mounting now uses separate plaques outside the
published body support with an open mouth corridor, checked across72 yaw directions.
This checkpoint precedes native matchup calibration below; Golem static review
remains pending.

Task logs: `work/golem-runtime-tests-03.log` and
`work/golem-runtime-clippy-03.log`, `work/golem-app-integration-01/receipt.json`,
and `work/golem-presentation-python-01.log`. Earlier failed test/lint logs are retained.

## Golem survival trial

The initial native corpus at `3710941`, with HP160, records no Golem wins in 64 Duel
rounds: 63 losses and one timeout across Shadow, Dragon, five Goblins and Shaman
party. The 24-round Fort spot also has no Golem wins, with three Dragon timeouts.
A first 35-damage slam leaves the Goblins at 15HP each; the observed Duel Goblin
rounds end around 7.2 seconds before a second slam. Against Shadow, Golem records
no damage or released lasers, so range and cover remain a separate limitation.

The isolated hypothesis doubles only Golem starting HP to 320 in the default
and authored configuration. Slam, laser, cooldowns, speed and the deliberate
medium-range gap stay unchanged. This tests whether the slow body survives long
enough for another attack. No route correction accompanies this health trial.

At clean `c1a6cf4`, the 32-round Duel comparison and 16-round Fort spot use identical
attacks and two side/initiative orders. The Golem now reliably reaches its second
slam against Goblins, while ranged movement and cover remain strong counters.
Keep HP320 as the initial playable hypothesis; do not remove the requested medium
range gap to force parity against the Shadow.

| Opponent | Duel: Golem wins / losses / timeouts | Fort: Golem wins / losses / timeouts |
| --- | ---: | ---: |
| Shadow | 0 / 8 / 0 | 0 / 4 / 0 |
| Dragon | 0 / 8 / 0 | 0 / 0 / 4 |
| Five Goblins | 8 / 0 / 0 | 4 / 0 / 0 |
| Shaman + three Goblins | 3 / 4 / 1 | 2 / 2 / 0 |

These are matchup-dependent machine results, not a claim that the Golem already
matches the Shadow's human challenge. Creature-only seeds often repeat an identical
trajectory. Fort's Dragon/Golem timeouts remain unfinished fights, not draws or wins.

A retained seed1 trace of both HP160 side orders establishes the initial Shadow
counter: both Golems start charging at tick1, then actual authored cover blocks
sight at109/121 before the198-tick lock. The admitted8-second cooldown prevents
another laser before death. Most subsequent poses are clear and supported;
Fireball impulses repeatedly oppose the2-unit/second approach. The late self-slam
removes footing but the body lands and resumes movement. Those traces do not
establish a collision bug as the cause of the initial losses.

Native timing exposes repeatable Dragon/Golem spikes, typically18–30 ticks above
8.333ms per Duel round, with maxima around11ms and a few terrain-publication spikes.
The Fort maximum is12.022ms. These are an open performance defect under investigation,
not a passed120Hz capacity claim. Sampling-profiler rounds are diagnostic and must
not replace uninstrumented timing evidence.

Retained task evidence: `outputs/arena-golem-calibration-01`,
`outputs/arena-golem-fort-02`, `outputs/arena-golem-trace-03` and its independent
diagnosis, `outputs/arena-golem-calibration-04`, `outputs/arena-golem-fort-05`, and
their summary JSON files. The latter two are source-frozen in the small detached
review checkout, allowing unrelated Wisp work to proceed on the local experiment
branch. Arena tests pass199 after the HP-only change; capture guards pass17 after
the natural laser review opponent changes from Shadow to Dragon.

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

## Wisp focused integration checkpoint

The guarded foundation `93a7735` now has a complete one-prism flight body, finite
two-layer deployment, observed-target Ember windup and frozen creature projectile
payload. Twelve-per-team real-world fixtures fit both Duel and Fort without
shrinking bodies or bypassing ceilings. A translated shared-edge precision case
initially rejected legal adjacent layers; the corrected prism overlap admits
tangency while retaining real penetration tests.

Gameplay tests pass225 and strict arena lint passes. The source-stable application
receipt passes7 actual-world battle tests,73 application tests and strict game
all-target/all-feature lint. Python launcher/capture guards pass23. These are
focused checks, before the final workspace gate. The first application attempt
failed to compile a new test's nonexistent `alive()` helper; the corrected test
uses authoritative positive HP and the complete retry passes. Failure logs remain.

The candidate also gives the observer footer a dark backing and adds a pale inner
core to the unchanged Golem laser beam. The prior12-frame Golem pack established
body/framing but exposed those readability defects; fresh static approval remains
pending. Wisp native paired calibration and its12-frame static matrix are next.
No subjective motion or balance approval is claimed.

Task evidence: `work/wisp-runtime-tests-02.log`,
`work/wisp-runtime-clippy-02.log`, `work/wisp-app-integration-02/receipt.json`,
and `taskwork/wisp-presentation/python-guards-01.json`.

## Golem static repairs and query optimization follow-up

The fresh `dbf3fc6...-arena-golem-v2-dragon-phases` matrix passes independent and
coordinator static review: all12 original1600×900 frames first, then the contact
sheet. The backed observer footer reads over pale ground and the pale active core
remains distinct within Dragon breath from both directions. This closes the two
prior static readability findings. It does not establish animation or native feel.

The collision cache candidate `1ae71e9` reduced the old eight-round Dragon/Golem
corpus from201 over-budget ticks to4, maximum11.054→9.570ms. The later combined
array/basis query refactor in `4081f08` passes227 arena tests, including bitwise
old-query oracles. Eight unprofiled native rounds at `dbf3fc6` preserve actor poses,
HP, setup, outcomes and original statistics exactly (the appended Wisp counter
is zero), but still record5 over-budget ticks and maximum9.373ms. Do not claim
that this second change fixed the remaining spikes. The first profiler capture
identified collision queries under steering; it does not justify a new geometry
or behavior rewrite without fresh attribution.

Task evidence: `work/dragon-box-query-checks-01/receipt.json`,
`outputs/arena-golem-performance-07`, `outputs/arena-golem-performance-08`,
`outputs/arena-golem-cache-comparison-07.json` and
`outputs/arena-golem-box-comparison-08.json`.

## Wisp opening-volley investigation

The first native48-round Duel corpus at `dbf3fc6` uses four seeds and both sides.
A single Wisp beats a single Goblin8/8 without receiving damage. Against Shadow,
Wisp groups1/2/4/12 lose all8 matches; eight Wisps lose7 with1 timeout. Twelve
Wisps release172 Embers across their8 rounds but deal only8.54 total damage.
This establishes a weak opening profile, not successful swarm calibration.

The two seed1 traces show all12 Wisps releasing at tick42 toward a then-stationary
observed Shadow. Their18.3–21.9-unit shots take about.57–.68 seconds. The Shadow
resumes strafing and moves about four units sideways before those impacts. The
shared trajectory is consistent with the admitted observation; these traces do
not show a collision or hidden-information defect.

The next isolated trial spreads each party's initial Wisp admissions over.6s,
starting only at its first eligible own sight. One Wisp keeps zero delay; later
cooldowns remain relative to each actual release. HP18, damage8, cooldown2,
windup.35, speed32 and splash.8 stay unchanged. If delivery remains weak, a
separate launch-speed trial will shorten the already observed shot flight.

Retained evidence: `outputs/arena-wisp-calibration-01` and
`outputs/arena-wisp-trace-02`. Traced timings are diagnostic only.

## Wisp initial calibrated profile

Keep HP30, Ember speed128, initial volley spread.6s; all other initial Wisp
values remain unchanged. Individual HP remains below Goblin50, damage8 below
Swipe12, and cooldown2 slower than Swipe1.2. The isolated trials distinguish
changes: staggering alone improved little; speed64 raised twelve-Wisp damage
to23.9 per Duel round, speed128 to48.1, and HP30 to57.5. The health trial helps
against incidental splash while a full35-damage Fireball still kills one Wisp.

The final48-round Duel corpus uses four seeds/both sides at frozen `8fa9640`
(main tuning `f0ef3fa`). The16-round Fort spot uses two seeds/both sides.

| Wisp group | Duel: Wisp wins / losses / timeouts | Fort: Wisp wins / losses / timeouts |
| --- | ---: | ---: |
| One, versus Shadow |0 /8 /0|Not sampled|
| Two, versus Shadow |0 /8 /0|Not sampled|
| Four, versus Shadow |0 /7 /1|0 /4 /0|
| Eight, versus Shadow |0 /8 /0|0 /4 /0|
| Twelve, versus Shadow |0 /7 /1|3 /1 /0|
| One, versus one Goblin |8 /0 /0|4 /0 /0|

The accepted Shadow remains a strong AoE/dodging counter in Duel. Fort's large
swarm results establish the intended scaling across this small machine corpus,
not a human win-rate estimate or a claim that four Wisps equal a Shadow. Keep
four as the accessible initial Fort wave and all five sizes in spectator mode.
Final Duel/Fort measured maxima are4.616/4.115ms with no over-budget combat ticks.
Larger sustained synthetic load, fresh Wisp pixels and the final workspace gate
remain separate checks.

All230 arena tests pass after the separate speed and HP trials. Earlier full
strict arena lint passes with the opening stagger; final strict combined lint
will cover the numeric-only follow-ups. Retained source-matched task evidence:
`outputs/arena-wisp-stagger-03`, `outputs/arena-wisp-speed64-04`,
`outputs/arena-wisp-speed128-05`, `outputs/arena-wisp-hp30-06`,
`outputs/arena-wisp-fort-07` and their companion summaries.

## Worm guarded foundation and world conversion

The guarded `dccfddb` foundation passes236 arena tests and strict arena lint,
including six snapshot/profile/admission contracts. Actual Worm setups remain
refused until body movement and attacks are integrated. The first compile exposed
a temporary-iterator lifetime in the new body snapshot accessor; the local-count
repair passes without changing the accepted Duel golden fixture.

The world conversion lane passes24 feature-enabled arena map tests (11 new) and
strict map/assets all-target/all-feature lint. The real ledger tests retain grass
1/1→dirt1/2, stone1/4→dirt1/2 and stone3/4→dirt2/2; repeat dirt and air remain
unchanged. Mixed blocked volumes reject atomically; extreme axial coordinates
reject using widened bounds arithmetic. Distinct sources may reuse their own
sequence without duplicate conversion or healing. Paused requests persist and
reset clears inbox, outcomes, sequences and converted materials. Authored
Fort/Duel publish finite elongated pockets with four-cell runs; actual Worm body
deployment and locomotion remain later gameplay/application checks.

Task evidence: `work/worm-foundation-checks-01/receipt.json`,
`work/worm-world-tests-01.log`, `work/worm-world-clippy-01.log`, and
`work/worm-world-checks-01.json`. The world checks run independently of active
changes in gameplay-owned files; they are not the final combined candidate gate.
