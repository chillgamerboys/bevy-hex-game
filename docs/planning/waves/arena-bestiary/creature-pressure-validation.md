# Creature pressure follow-up — validation

Implementation, local checks, native captures and scoped static review are complete.
Remote CI and human playtest remain separate pending gates. This record covers the
2026-09-08 creature-pressure follow-up, not a fresh calibration of earlier bestiary
results. Delivery stays on draft PR #221; no merge is authorized.

## Source and verification

The combined library run used `01c431b259c5e04776b343067701450e32a165ef`.
The verified native/capture candidate is `217102c5202a9ba8b924a1d5d64bc21c56a9846d`.
The expanded-route/workload checkpoint is `71dc0f8544af9436c89c4479171315d821de3d57`.
The final source checkpoint is `9aa3e0c40f67444d759e4dfe1f49e61d0ee3af9d`;
its sole difference from `217102c` is the Stone Swipe review-camera framing.
Strict workspace lint and the native Swipe build/capture pass at that final source.
All other cited capture surfaces and runtime behavior are unchanged by this camera
branch. This delivery commit changes documentation only.

Local evidence lives under `.context/creature-pressure-checks/`. The final SHA-256
ledger `final-log-hashes.json` records current logs; earlier checkpoint ledgers are
historical and may refer to superseded logs. Command scopes below do not imply every
historical suite was rerun at the final head.

| Check | Result | Local evidence file |
|---|---|---|
| Arena library | 298 passed | `combined-libs-final.log`, at `01c431b` |
| Game library | 490 passed, 5 explicitly ignored | `game-final.log`, candidate `217102c` |
| Map library | 574 passed, 28 explicitly ignored | Same combined log |
| Real-map battle, encounter and route targets | 28 passed: 14 + 6 + 8 | `real-maps.log` |
| Explicit headless performance profiles | 2 passed, emitting 5 workload rows | `performance.log` |
| Capture-only stress override isolation | 1 passed | `stress-adapter.log` |
| Python launcher/capture guards | 40 passed | `python-final.log` |
| Strict workspace Clippy, all targets/features | Passed final rerun | `clippy-swipe-camera.log`, at `9aa3e0c` |
| Formatting | Passed | `format-final.log` |
| Dependencies, selector tests and deprecated UI terms | Passed | `deny.log`, `selector-final.log`, `ui-terms-final.log` |
| Markdown relative links | Passed | `links-final.log` |

Real-map tests ran with `cargo test -p hex_game --all-features --test arena_battles
--test arena_encounters --test arena_routes`. Profiles explicitly added `-- --ignored
--nocapture --test-threads=1` to the encounter target. Strict lint used
`cargo clippy --workspace --all-targets --all-features -- -D warnings`.
`remaining-receipts.json` retains the earlier failed Clippy attempt; the final
successful log supersedes that attempt, not the other command receipts.

Coverage includes unchanged Duel goldens, hidden-history equivalence, creature
attack caps and collision, real Worm conversion/emergence/Boulder/reset, larger
roster admission, whole-group completion, terminal cursor/menu state, cancellation,
selection-preserving restart and publication of the last terrain queue. The final
application rerun also proves ordinary Shield input triggers the actual Golem
Stone Swipe in Fort, and result captures keep the living human mesh hidden from
the first-person camera. Neither repair changes ordinary player combat rules.

## Current behavior

Goblins10 and Shaman-plus-five replace the smaller groups, with Seven Regions now
containing 17 enemies. Goblins spread approaches, make admitted high jumps and keep
escorts near the Shaman's nine-unit support aura. Dragons can make bounded swept
approach bursts; Wisps chip obstructing cover toward their own recent sighting.
Golem laser fire lasts four seconds with a 180-damage cap, tracks own sightings and
continues from the same target's last observed velocity after losing sight. A
separate frontal swipe clears obstructing terrain while excluding footing/protection.

An activated Worm privately senses map-wide hostile positions only while physically
buried. Above ground it requires actual sight; damage causes retraction and renewed
pursuit. Its underground approach/expose/attack/reposition cycle retains HP320,
Boulder70, correlated world acknowledgement and full swept-body admission.
Wins, defeat, draws and timeouts automatically pause and free the cursor; completed
rounds cannot resume, and reset preserves the selected setup.

## Bounded headless load measurement

These are optimized native test-process `ArenaTick` wall times, excluding renderer,
application-frame work, synthetic placement-query cost and GPU cost. Each profile
runs 3,600 ticks, discards the
first 120 timing samples and reports 3,480 samples. Setup time is headless world
construction, not interactive loading time or a rendered frame-rate measurement.

The explicit synthetic fixture assigns all actors 100,000 HP and extends ground,
Shadow and Dragon home leashes to **150 units**. It places the human at repeatedly
revalidated, dry/body-clear/visible party targets every 72 ticks, retaining anchors
within ten units of home. Search, sight, activation, movement and attacks retain
their authored rules. This measures sustained work; it does **not** validate normal
home-return behavior, realistic player travel, balance or natural encounter duration.

| Workload | Living enemies | All active / measured | Setup ms | Tick p95 ms | p99 ms | Max ms | Publication ticks | Destroyed voxels |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Fort Dragon | 1 | 3,480 / 3,480 | 141.538 | 0.219 | 1.911 | 2.921 | 13 | 336 |
| Fort Goblins | 10 | 3,480 / 3,480 | 139.024 | 0.648 | 1.594 | 4.229 | 68 | 288 |
| Fort Shaman party | 6 | 3,480 / 3,480 | 136.943 | 0.423 | 2.862 | 6.393 | 43 | 362 |
| Fort Shadow | 1 | 3,480 / 3,480 | 136.982 | 0.077 | 0.161 | 0.487 | 19 | 397 |
| Seven Regions | 17 | 3,454 / 3,480 | 2,238.153 | 3.249 | 4.714 | 8.980 | 27 | 276 |

Every row exceeds the unchanged 2,400 all-active-sample guard, records real damage
outcomes and destruction, and reports zero invalid placement ticks. Seven Regions
retains all 17 enemies throughout; its all-active fraction is 99.253%. Its 27
publication samples have p95/p99/max of 5.853/7.867/8.320 ms, with 34 measured damage-
outcome ticks and 42 total terrain outcomes. Its overall 8.980 ms maximum exceeds
the 120 Hz budget of 8.333 ms. The summary does not retain the number of over-budget
samples, so no zero-spike or every-tick-budget claim is warranted.

Normal-home evidence is separate: the library regressions cover search expiry,
return, preserved HP and nearby-only Returning defense using ordinary tuning.
The route target replays frozen controller excursions for all 18 Fort profile
actors and all 17 Seven Regions actors, plus four human roundtrips, damaged-floor
travel and blocked-wall recovery. It uses no route search, teleport or jump/fly
shortcut in the final fixtures; these are movement contracts, not proof that
normal autonomous pursuit always chooses a successful route.

## Native load measurement

The clean `217102c` image-target build also ran three native synthetic workloads,
each with 3,480 post-warmup ticks. These use the explicit HP/leash overrides above.
All **10,440 measured ticks stayed below 8.333 ms**. This is a CPU schedule result,
not a rendered frame-rate guarantee.

| Native workload | Tick p95 / p99 / max ms | All-active ticks | Ready elapsed ms |
|---|---:|---:|---:|
| Fort Goblins10 | 0.800 / 2.484 / 6.974 | 3,480 / 3,480 | 624.122 |
| Fort Shaman + 5 | 0.468 / 2.795 / 5.249 | 3,480 / 3,480 | 636.416 |
| Seven Regions17 | 2.809 / 4.620 / 8.095 | 3,454 / 3,480 | 2,873.977 |

Readiness runs from view construction through four ready frames, excluding Cargo.
The Seven bootstrap tick was 8.909 ms, outside the post-warmup set. The earlier
headless maximum of 8.980 ms was not reproduced in the native measured set.
Publication-tick maxima were 4.065 / 1.474 / 6.427 ms respectively. The largest
overall ticks had no publication flag, so they cannot be attributed to destruction.

Measured application wall intervals had p95 around 19.6–20.6 ms. They include
fixture work, scheduling and render-submission waits, not isolated GPU/vsync cost.
An existing user game remained running on the same machine; no 60 FPS claim is made.
The Shaman and Seven fixtures retained their current human pose and suppressed the
scripted cast on 97 and four ticks where no new valid visit pose was found. These
are placement fallbacks, not evidence of an invalid teleport or a zero-fallback run.

The immutable performance pack is under
`.context/creature-pressure-captures-v2/performance/217102c5202a9ba8b924a1d5d64bc21c56a9846d-arena-performance-v2-synthetic-extended-leashes-focused/`.
`performance-review.md` and `performance-metrics.json` retain independent hash
verification, nearest-rank quantiles, timing boundaries and publication samples.
The pack's three PNGs are measurement attachments, not static approval frames.

## Native static review

Root and an independent reviewer inspected originals and contact sheets. Clean
source, phase/state hooks, commands and per-artifact hashes are retained in each
pack. Stills assess presentation only; tests establish the underlying gameplay.

| Surface | Source | Frames | Static result |
|---|---|---:|---|
| Main Menu Battle Mode, normal/compact/200% scale | `216b969` | 3 | Pass: text, layout and complete actions |
| Terminal win/defeat menu | `217102c` | 2 | Pass: readable actions, coherent first-person background |
| Golem charge and beam, two angles | `217102c` | 4 | Scoped pass: charge and beam readable; breath reduces contrast |
| Fort/Seven maps, Goblins, Shaman and aura | `217102c` | 9 | Map/ready UI pass; close creature/aura views have contrast and terrain-occlusion limits |
| Observer timeout menu | `217102c` | 1 | Pass: complete result menu and disabled resume |
| Golem Stone Swipe windup/active | `9aa3e0c` | 2 | Scoped pass: attack face, marks and contact opening visible; small windup marks |

Main Menu files are in `.context/visual-walks/216b969cca27eb8b0609270db8338c9c0c49688f-battle-entry-v1/`.
The `217102c` packs live under `.context/creature-pressure-captures-v2/`, grouped as
`terminal`, `golem-beam`, `encounters` and `observer-result`, with source-prefixed
matrix directories. Final Swipe files live in
`.context/creature-pressure-swipe-v3/9aa3e0c40f67444d759e4dfe1f49e61d0ee3af9d-arena-golem-v3-pressure-phases-focused/`.
Independent notes remain beside immutable receipts; the automated receipt's
`UNREVIEWED` field is not rewritten to impersonate human/agent review.
`capture-index.json` in the local check directory records pack and PNG hashes.

The first terminal win frame exposed the human mesh; the first Swipe recipe failed
to elicit the attack, and the next camera hid its effects. These failed/superseded
packs remain in place. Fixes use normal Shield input and a capture-only camera;
they do not force an attack, alter normal cameras or weaken encounter rules.
The terminal win/defeat fixtures inject explicit knockout state, whereas the observer
result reaches the ordinary six-second timeout. Neither establishes human win rates.

## CI and remaining limits

[CI run 34315874637](https://github.com/chillgamerboys/bevy-hex-game/actions/runs/34315874637)
for `217102c` passed strict CI-profile Clippy/ordinary tests, gameplay/map partitions,
coverage, documentation, formatting/dependencies and Linux/macOS shipping builds.
Windows was still building at this report checkpoint. The previous Linux lint
failure was repaired with a target-scoped expectation preserving platform defaults.
The complete selector-chosen gate runs again for the pushed delivery head; its live
status belongs to [PR #221](https://github.com/chillgamerboys/bevy-hex-game/pull/221).
No final-head green-CI or merge-readiness claim is made before it completes.

A Worm whose destroyed footprint admits no common shallow band can retract its
head and remain unable to travel. It cannot invent support, deepen its burrow or
crawl above ground to bypass that refusal. Heavy destruction can defeat bounded
local routes. Close Goblin/Shaman silhouettes and parts of the aura remain hard to
read behind opaque terrain; the map overview and menu passes do not approve every
close creature view. Human control feel, moving attack recognition, window behavior
and revised balance remain **pending playtest**.

Earlier Worm6/8 against five Goblins, Wisp12 Fort3/4 and Golem win rates retain their
original source/roster context. They are not new pressure-pass matchup results;
timeouts remain unresolved, and static pixels do not establish motion or input feel.

Local receipt ledger SHA-256: `02c21641b5c9d8a0fbabf44067d3077dbb125f8d769950d2b7a5e1041a8e6920`.
Capture index SHA-256: `1ffecede4a0fbc5a8c1285168731363cd0bbf4db73e00d1b779b89ee4470a4e2`.
