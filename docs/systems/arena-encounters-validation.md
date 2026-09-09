# Authored arena encounter validation

Status: combined implementation and automated validation are complete. All 29
combined checks passed at `25fa64d`; final static reviews and native measurements
are recorded in the [bestiary close-out](arena-bestiary-validation.md). Human
playtesting remains pending. The accepted reference is local commit
`127d1ce2058de9ba79da9717b7e37df4b9913502` on
`experiment/spell-combat-arena`.

## Candidate and scope

The wave manifest is [arena-encounters](../planning/waves/arena-encounters/manifest.md).
The [approved plan](../planning/waves/arena-encounters/plan.md) and
[controls/tuning guide](arena-encounters.md) define the delivered scope.
Fort uses seed 640367719; Seven Regions uses seed 703700113. The original Duel
retains its original deterministic recipe. The user subsequently authorized a
combined draft PR to `dev`; merging remains a separate decision.

## Focused checkpoints

| Checkpoint | Evidence | Result |
|---|---|---|
| Foundation `ad70deb` | hex_core and hex_arena library tests | 126 core and 82 accepted arena tests passed. |
| Foundation `ad70deb` | Strict hex_core all-target/all-feature Clippy | Passed. |
| World `e253af2` | 12 authored-arena map tests | Passed: recipe bounds/offsets, anchors, static props, physical damage, reset and dirty publication. |
| World `0c1b104` | 8 terrain-damage schema tests; strict hex_map/hex_assets Clippy | Passed, including explicit physical admission and versioned fingerprints. |
| Gameplay `578b3cf` | hex_arena CI library suite | 111 passed, including all 82 accepted tests and party cue/return regressions. |
| Presentation `067aa55` | hex_game arena CI library suite | 44 passed, 2 explicit manual/performance fixtures ignored; 326 unrelated tests filtered. |
| Composition `42ae1d9` | hex_game arena_encounters CI target | 4 passed; 2 explicit timing fixtures ignored during correctness checks. |
| Travel `42ae1d9` | hex_game arena_routes CI target | 8 passed: four human roundtrips, enemy ground loops, physical damage and wall stop/clear/retry. |
| Required combined gate `25fa64d` | Complete repository selector closure from accepted base | All 29 checks passed; see the bestiary close-out for source identity and evidence. |
| Native arena build `f12b329` | Cargo dev + arena-prototype, task work/encounter-native-build-03.log | Passed (9m 01s rebuild after shared contract addition). |
| Windowless visual checkpoints | 27 views at3046779 plus6 affected VFX and2 Duel views atf12b329 | Full-resolution coordinator/independent review; aura defect repaired. Final combined bestiary matrix still pending. |

The world test suite's aggregate runtime is not a map loading benchmark. Native
render review establishes static appearance only; it cannot establish attacks,
party activation, collision, or control feel.

## Performance protocol

Measure separately:

- Headless setup, and ArenaTick CPU wall-time percentiles, on both real maps.
- Sustained synthetic Seven Regions pressure with all ten enemies active, with
  explicit sample counts for all-party activity and terrain publications.
- Damage outcome and destruction samples, rather than an idle-only timing claim.
- Native windowless startup/render-ready elapsed and real frame intervals. Fixed
  simulation deltas are not rendered-frame measurements, GPU timings or vsync FPS.

The synthetic stress fixture grants additional life and repositions its human
input source around parties to sustain load. It is intentionally unsuitable for
balance, encounter accessibility or normal player movement claims. Continuous
controller route tests supply separate accessibility evidence.

## Native review matrix

Static inspection covers complete Fort and Seven Regions footprints from two
azimuths, first/third person, the three Seven Regions encounter landmarks,
creature models and rear silhouettes, attack preparation/breath/swipe, Shaman
charging/aura, and transparent Dragon panels from both sides. The original Duel
remains a separate control/occlusion regression matrix. Captures freeze liquid
phase at zero and require render readiness plus four completed frames.

The separate native performance matrix uses the same explicitly synthetic
party-visiting workload as the headless fixture. It cannot serve as a static
approval pack or as evidence of normal player movement or balance.

## Remaining human review

Run 20–30 Fort encounters across Dragon, Goblins, Shaman party and Shadow. Compare
challenge, damage sources, surviving HP, retreat/recovery opportunities, readable
windups, and the practical value of targeting the Shaman first. Play Seven Regions
for independent activation and the experience of traveling between fights.

Human testing is still required for native mouse feel, flying creature motion,
melee readability, frame pacing in an ordinary visible window, and subjective
balance. Automated correctness and synthetic performance cannot establish the
requested enemy-strength equivalence.


## Original creature milestone before spectator work

At `f12b329f5bf2e1445d7f28050b7b1190dbe33c31`, all122 hex_arena library
checks pass, including three frozen Duel replay fixtures and three battle setup
contracts. Strict arena lint and scoped integration lint pass. Latest application
suite has51 passing arena tests, including nonphysical VFX shadow admission.
Goblins now use the player's .8-unit height and .25-unit radius; the seven-hex/five-level
body request belongs to the future Golem.

The full27-view3046779 matrix revealed a Shaman aura shadow/readability defect.
Repair798a898 makes nonphysical effects non-shadow-casting and lowers aura fill;
f12b329 recaptures six affected VFX entries and both Duel cameras. Aura colors and
supported actors are now readable. The primary Dragon barrier camera is still
partially keep-occluded; its reverse shows all four panel edges and full transparency.
The Duel cameras review HUD/opaque crater surfaces, not a visible opponent or native
input feel. Full combined static review and human motion remain separate final gates.
Evidence: local ignored `.context/visual-walks/30467790...-encounters-02/` and
`.context/visual-walks/f12b329...-milestone-03/`, with per-file hashes and review notes.

The first native stress fixture failed the sustained-activity criterion (2054 ticks)
because unchecked side poses became hidden after excavation, and .1-second visits
interrupted ranged preparation. This failure is retained. Refinement4f305ec uses
1.2-second visits with bounded supported, dry, body-clear, visible target positions;
normal creature policy/cooldowns stay unchanged. Actual Dragon and Shaman attacks
now execute. Logical workload evidence is under task outputs/arena-stress-stimulus-refinement.

### Native synthetic timings at f12b329

Each run is3600 simulation ticks; first120 excluded. The explicit extra-life,
party-visiting fixture is not normal movement or balance evidence. No concurrent
build/timing job ran during measurement. Values are CPU milliseconds.

| Workload | All-active samples | Tick p95 / p99 / max | Publication max | Render-ready milliseconds |
|---|---:|---:|---:|---:|
| Fort Dragon |3480|.121 /3.460 /4.501|3.831|624.7|
| Fort Goblins |3480|.325 /.743 /1.497|1.497|626.6|
| Fort Shaman party |3480|.300 /.527 /1.380|1.380|635.3|
| Fort Shadow |3480|.086 /.162 /.276|.261|619.4|
| Seven Regions, all10 |3310|1.243 /5.101 /8.287|8.287|2780.1|

No measured simulation tick exceeded8.333ms. Seven publication margin is narrow and
must be remeasured on the final expanded candidate. Native main-loop frame intervals
p99 were21.06–21.75ms in these windowless fixtures; these are not GPU durations or
vsync FPS. Measured post-warmup destruction ranged48–277voxels, with5–67 publication
ticks per case; Seven recorded240 destroyed voxels across49 publication ticks.
Full machine summary: task outputs/encounter-native-performance-f12b329.json.
Separate native-profile headless timing and the full repository-selected combined
gate remain pending final candidate closure; CI fixture runtimes are not native performance.
