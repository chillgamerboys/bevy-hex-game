# Authored arena encounter validation

Status: combined implementation under validation. No native playtest or balance
claim is implied by these records. The accepted reference is local commit
`127d1ce2058de9ba79da9717b7e37df4b9913502` on
`experiment/spell-combat-arena`.

## Candidate and scope

The wave manifest is [arena-encounters](../planning/waves/arena-encounters/manifest.md).
The [approved plan](../planning/waves/arena-encounters/plan.md) and
[controls/tuning guide](arena-encounters.md) define the delivered scope.
Fort uses seed 640367719; Seven Regions uses seed 703700113. The original Duel
retains its original deterministic recipe. All changes remain local; no PR or
merge into dev is part of this delivery.

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
| Required combined gate | Repository selector from accepted base | Pending. |
| Native arena build | Cargo dev + arena-prototype features | Pending. |
| Fresh windowless render matrix | Full-resolution review and independent reviewer | Pending. |

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
