# Foundation hardening

Durable handoff for the July 2026 correctness, scalability, lifecycle, and CI cleanup.
This record describes the implemented playable slice. It does not turn unresolved
design questions into code.

> **Integration status, 2026-07-29:** automated acceptance is complete for the
> focused implementation: targeted correctness gates, the complete local
> release-stress matrix, scripted visual walks, exact-final-tip ordinary triplicate,
> PR-stack propagation, strict GitHub checks, coverage, deep checks, and all three
> shipping platforms. PR #137 is the review boundary. Human presentation/play-feel
> approval remains a separate mandatory signoff.

## Reference and scope

The clean reference was `origin/dev` at `31d4e63`: formatting, dependency policy,
strict all-feature Clippy, docs, the shipping build, and 1,293 ordinary tests passed;
18 stress tests were ignored. The foundation branch later merged the then-current
`origin/dev` at `0b85ca3` without rewriting either history.

The hardening scope is the implemented 0.3 slice:

- faction-authorized AI and casting;
- coherent content loading and authored-lattice validation;
- bounded deterministic AI/combat diagnostics;
- measured AI, formation, perception, lifecycle, terrain, and generator behavior;
- repeatable stress and platform gates; and
- conflict-safe stabilization of the active Trova97 PR stack.

Rout and surrender, unit obstruction, multi-hex bodies, terrain magic,
obstruction-aware trajectories, fog rendering, and balance decisions are deliberate
exclusions. They remain roadmap work because their rules are unresolved, not because
the foundation silently chose defaults.

## Branch and commit matrix

No history was rewritten and no PR was merged by this hardening work. Shared fixes
remain on `status-report-request`; PR-specific fixes were pushed to the lowest
Trova97-owned branch that introduced the behavior.

### Foundation branch

| Commit | Change | Validation at this snapshot |
|---|---|---|
| `64c4b73` | coherent semantic content revisions, contiguous authored lattices, strict scenario fields, registered resolved lighting, and dark-handoff validation | content/load/config regressions green |
| `82a5c94` | faction-authorized AI/casting and same-frame ordering | AI, combat, casting, and hidden-spillover regressions green |
| `46412cb` | idle lifecycle guards, bounded diagnostics, stress and CI gates | focused lifecycle, terrain, generator, and soak gates green |
| `86b850e` | shared formation traversal projections | 124 ordinary formation/unit tests and release matrix green; 1 ignored |
| `7c7be20` | radius-40 and same-frame perception acceptance tests | focused perception matrix, idle, lifecycle, and Downed-order tests green |
| `2b0b0db` | merge current `origin/dev` into the foundation branch | focused post-merge reconciliation green |
| `2ffb45d` | faction-authorized combat-log and pulse disclosure | 7 focused readout tests and strict all-feature game Clippy green |
| `dc09492` | deterministic V2 Hills ordinary and release corpora | 134 ordinary semantic cases and 10,000-seed release corpus green |
| `aae0c94` | align the sky test fixture with interpolation-aware dark handoff | 12 sky and 22 settings tests, strict Clippy, and preliminary full workspace gate green |
| `ec887a9` | reconcile the implemented contracts, project status, and roadmap exclusions | Markdown links, diff checks, and focused code/doc gates green |
| `4b5203c` | close audit documentation gaps and repair strict rustdoc links | warnings-denied workspace rustdoc green |
| `d81adad` | execute isolated deep checks with one pinned nightly | isolated `hex_core` check, 117/1 scoped Miri, and Linux AddressSanitizer green |
| `f55b818` | keep scripted casts on authorized anchors and make Party Trial routing atomic | all seven release walks, 176/176 steps and 37/37 captures green |
| `1c14bcf` | keep branch-derived authored mountain peaks connected through protected carving | seed 6401 ordinary regression, fallback matrix, 390 `hex_map` tests, and corrected 30,000-map expanded corpus green |
| `d9fc93c` | select the Windows LLVM linker for the exact release package and remove its platform-only dead-code warning | exact Windows package passes in 22m07s; local and all other code-tip GitHub gates green |

### Active PR stack

| Dependency edge or PR | Pushed head | Result at this snapshot |
|---|---:|---|
| `#116 → #125` | `2086ba8 → 3b0e49d` | exact ancestry; all six GitHub jobs green |
| `#119 → #124` | `47ce2e3 → eaacc81` | exact ancestry; all six GitHub jobs green |
| #120, `feat/v3-fort` | `fd9cbe3` | conflicts resolved against its actual base; mergeable; all six GitHub checks green |
| #122, `feat/v3-caves-lights` | `6d7cff1` | conflicts resolved against its actual base; mergeable; all six GitHub checks green |
| `#120/#122 → #126` | `25e0005` | both ancestry merges pushed; mergeable; all six GitHub checks green |
| `#126 → #129` | `c07b105` | broad V3 suite green; mergeable; all six GitHub checks green |
| `#126 → #132` | `f455367` | Caves/crystal tests green; mergeable; all six GitHub checks green |
| `#126 → #130` | `70a7c32` | Caves seed-445 golden retained; mergeable; all six GitHub checks green |
| `#130 → #131` | `2dc4e99` | propagated; mergeable; all six GitHub checks green |
| `#131 → #133`, `feat/v3-ring7-runtime` | `9dd03e9` | contains #131 and Forest fix `12a98c2`; mergeable; all six GitHub checks green; broad V3 175/7 ignored and focused gates green |

Every propagated worktree was clean and its local head exactly matched its remote
head after the final push.

The #133 Forest failure was not “fixed” by changing the expected
`left: 7, right: 4`. Generated output was bisected to the first behavioral divergence
and the stitched Forest planner was isolated there.

After the scope freeze, colleague-owned draft PR #136 (“Wave 6: Creator and Combat
Lab”) appeared from head `56367cd`. It was inspected read-only and is not part of this
integration graph. Five platform/quick jobs pass, while Clippy/tests fail because
`review.rs` does not handle the newly added `Screen::CharacterCreator` and
`Screen::CombatLab` variants. The owner must repair that draft before it is eligible
to merge; no commits, pushes, or comments were made on it here.

### PR #135

PR #135 is colleague-owned and was tested as evidence only. No commits, pushes,
comments, or branch mutations were made. Its final inspected head is `09fd513`;
GitHub checks and every final-head local gate are green: format, dependency policy,
strict all-feature Clippy, all-feature ordinary tests and doctests, warnings-denied
rustdoc, and the exact release game build. The release link emitted no macOS
`__eh_frame` warning.

One P2 risk remains: `save.rs::scenario_digest` hashes compile-time
`include_str!` bytes, while a development session may run a hot-reloaded accepted
semantic content revision. A resume compatibility digest can therefore describe
different content from the runtime that produced the save. This does not block the
foundation PR, and it must be fixed on the owning save branch rather than smuggled
into the evidence-only review.

## Correctness contracts now enforced

### Knowledge and casting

`FactionMapKnowledge::local_map_knowledge(faction)` is the only terrain projection an
AI decision receives. Observed and Remembered surfaces may contribute traversal;
Unknown surfaces do not. Hostile identities, positions, effects, and turn-order
entries require current observation. A downed hostile is not an offensive movement
goal.

Visibility and sight contribution are distinct. A downed unit may remain visible,
but `ObservedUnit::provides_sight` is false and adding or removing `Downed` invalidates
perception. The authorization-critical prefix of the live update order is:

`PublishKnowledge` → combat spatial-knowledge synchronization →
`CombatSystems::Act` → `CombatSystems::Apply`; normal combat processing then continues
through `Resolve` and `Advance`.

Cast preview, unit target cycling, AI enumeration, and the authoritative applier all
require an Observed exact anchor. Remembered and Unknown anchors fail. Once an
Observed anchor resolves, an area may spill into hidden space; the simulation applies
the result without promoting knowledge or disclosing the hidden outcome. Player-facing
logs and pulses apply the same faction-generic spatial authority: hidden hostile
outcomes are suppressed, while an outcome against an allied unit remains visible with
an unauthorized hostile source rendered as `Unknown source`. Authoritative combat
events, summaries, and opt-in tooling transcripts remain complete.

### Content and authored lattices

Element, spell, substance, content-index, lattice-file, and lattice-library semantics
have deterministic canonical fingerprints. `ContentIndex::matches_sources` and
`LatticeLibrary::matches_sources` distinguish a retained last-valid resource from the
current raw inputs.

`AcceptedContentRevision` is published only when every raw, direct, and derived layer
describes one semantic revision. Loading gates on that marker and rechecks the
sources; Bevy resource presence and settled change ticks cannot admit a mixed
revision. Invalid cross-references keep last-valid data available but keep Loading
closed until repair or revert.

Every lattice entering through authored `lattices.ron` resolution must form one
contiguous hex arrangement. A disconnected lattice is rejected with its archetype in
the error. Shipped valid content is unchanged.

Scenario files reject unknown fields, so a misspelled authored option cannot be
silently ignored. `ResolvedLighting` is registered for reflection, and dark-light
handoff validation evaluates interpolated state at the actual zero-elevation
transition rather than relying only on authored endpoints.

### Compact choices and bounded records

Ordinary AI turn commands remain a fingerprinted canonical `LegalActionSet`.
Disable/restore requests instead carry a compact `CellChoiceSet`: subject, exact
quota, canonical eligible cells, and fingerprint. The host validates fingerprint,
count, uniqueness, and eligibility before constructing the existing replayable
command. It does not allocate every `n choose k` combination.

Detailed AI inspection retains the latest 64 traces. `CombatSummary` records complete
totals and rolling fingerprints while retaining at most 4,096 detailed events and
4,096 detailed AI decisions. `CombatTranscriptRecorder` is an explicit opt-in
unbounded recorder for tests and tooling. Existing serialized summaries retain
compatibility decoding.

## Measured validation

All timings below are release-mode measurements on the primary Mac unless stated
otherwise. They are regression evidence, not portable promises about another
machine.

The final inventory discovers 1,363 tests: 1,338 ordinary tests in the complete
all-feature workspace gate and 25 explicitly ignored stress/benchmark entries.

### AI decision matrix

Each cell ran 100 deterministic decisions. All fingerprints matched.

| Radius | Teams | p95 | Worst |
|---:|---:|---:|---:|
| 40 | 1v1 | 0.691 ms | 0.714 ms |
| 40 | 3v3 | 0.785 ms | 0.870 ms |
| 40 | 6v6 | 0.894 ms | 0.899 ms |

The radius-12 and radius-20 cells also passed the same fingerprint and budget gate.
The radius-40 result is far below the required p95 50 ms and worst 100 ms. One
authorized traversal graph, one actor reach/predecessor projection, and one reverse
distance map per live observed hostile replace BFS-per-move-per-hostile scoring.

### Six-member formation matrix

Each cell ran 100 times and preserved the exact expected route.

| Fixture | Steps | p95 | Worst |
|---|---:|---:|---:|
| Open | 10 | 29.625 µs | 33.958 µs |
| Open | 50 | 126.167 µs | 130.917 µs |
| Open | 100 | 175.334 µs | 190.000 µs |
| Stacked | 10 | 18.542 µs | 34.250 µs |
| Stacked | 50 | 84.167 µs | 95.958 µs |
| Stacked | 100 | 143.667 µs | 156.208 µs |
| Narrow | 10 | 30.625 µs | 36.833 µs |
| Narrow | 50 | 84.042 µs | 97.250 µs |
| Narrow | 100 | 161.916 µs | 169.708 µs |
| Blocked | 10 | 39.708 µs | 45.917 µs |
| Blocked | 50 | 89.625 µs | 98.000 µs |
| Blocked | 100 | 156.959 µs | 175.125 µs |

Formation planning now shares the terrain footing and computes one
reach/predecessor projection per member instead of routing independently to every
fallback candidate. All cells are far below the p95 16.7 ms and worst 50 ms gates.

### Perception and lifecycle

- The radius-40 matrix covers 4,921 exact surfaces across light changes,
  active/inactive observers, memory, and re-observation.
- Ten thousand idle perception frames took 1.38 s and caused zero recomputations.
- Downing the sole sight provider republishes knowledge before same-frame AI; the
  hidden unit is absent from both identity and target fields.
- One hundred gameplay enter/exit cycles took 0.09 s and left exact expected
  entity/resource counts.
- Idle camera and cutaway tests, object reconciliation, terrain identity, and
  selection-overlay identity tests are green.

### Combat soak and diagnostic bounds

- Party Trial repeated 100 times in 8.619 s. Every run ended Defeat in round 10 with
  67 AI decisions; AI fingerprint `6188876318016585085`, event count 112, event
  fingerprint `3044013766241655007`.
- Two 10,000-turn deliberate stalemates completed in 4.266 s with no queue deadlock.
  Each reached round 5,000 with 5,000 AI decisions and fingerprint
  `4687009868250885990`.
- The stalemate retained exactly 64 detailed AI traces and 4,096 AI-summary details;
  complete totals and fingerprints continued advancing.

### Terrain and generation

- One hundred release terrain edits measured p95 2.283625 ms and worst 3.706459 ms.
  The current broad rebuild changed 189,850 entities in total, at most 1,899 per
  edit. Mesh assets plateaued `4 → 5 → 5`; material assets plateaued
  `7 → 14 → 14`.
- Because the terrain-edit timing gate passes comfortably, the broad rebuild is
  retained for now rather than accepting a riskier partial-column rewrite. Churn
  remains a measured P2 optimization opportunity.
- Seven ordinary generator corpora cover 923 deduplicated inputs. The original six
  cover 789 inputs; the V2 Hills addition contributes 134 semantic cases. Mountains
  seed 52 remains the only known ordinary fallback-pressure case and stays below 1%.
- The complete weekly/manual release matrix passed after promoting one newly
  discovered Mountains failure to an ordinary regression:

| Corpus | Valid generated worlds | Existing fallback gate | Test time | Command wall |
|---|---:|---:|---:|---:|
| V1 Hills | 10,000/10,000; 10,000 unique fingerprints | 0 | 62.706 s | 267.930 s cold |
| V2 Hills | 10,000/10,000; 10,000 unique fingerprints | 0 | 66.519 s | 68.279 s |
| V2 Caves | 10,000/10,000 | <1% | 72.16 s | 254.568 s including shared build wait |
| V2 Caves expanded | 10,000/10,000 | <1% | 82.64 s | 84.171 s |
| V2 Mountains | 10,000/10,000 | <1% | 114.01 s | 115.511 s |
| V2 Mountains expanded `(18,5)/(21,6)/(24,7)` | 30,000/30,000 | <1% for each 10,000-map setting | 634.43 s | 779.662 s including rebuild |
| V2 Sky | 10,000/10,000 | <1% | 141.97 s | 143.469 s |
| V3 Forest | 10,000/10,000 | <1% | 131.56 s | 312.10 s including shared build wait |
| V3 Waterfall | 10,000/10,000 | <1% | 164.06 s | 165.56 s |

The first expanded Mountains run failed after 16,401 valid maps at relief 21,
six peaks, seed 6401: a branch summit survived selection while its attachment prefix
crossed protected approach cells that carving later flattened. The peak selector now
requires the authored prefix through a branch-derived summit to survive protected and
route carving. Seed 6401 is an ordinary regression; the deterministic fallback is
directly tested for all three expanded setting pairs. The shipped Mountains
fingerprint is unchanged.

Passing tests that expose only a fallback assertion substantiate the bound but do not
emit the exact passing count; this is an observability limitation, not a missing gate.
Corpus timings are trends rather than hardware-sensitive PR failures.

### Toolchain and CI

The branch adds superseded-run cancellation, 10-minute lightweight jobs, 30-minute
build/test jobs, nextest timing and JUnit artifacts, exact
`cargo build --package hex_game --release` validation on Linux/macOS/Windows, and
LLVM coverage artifacts for the headless domain crates. Weekly/manual workflows own
release stress corpora, performance probes, Party Trial/stalemate soaks, scoped Miri,
and supported Linux AddressSanitizer checks.

The deep workflow uses `nightly-2026-07-29` explicitly despite the repository's
stable toolchain file. Scoped Miri passed 117 tests with one ignored in 5m38.209s;
the matching GitHub Miri and Linux AddressSanitizer jobs passed. An isolated
`cargo check --package hex_core` prevents workspace feature unification from hiding
missing direct features again.

The first cold Windows exact-release job reached `hex_game`'s final ThinLTO link but
the MSVC linker exceeded the 30-minute job ceiling. The corrected code tip selects the
runner's preinstalled MSVC-compatible LLVM linker for that exact Cargo profile and
records its version. This is a CI linker selection, not a weaker release build. The
exact package then passed in 22m07s; Linux passed in 7m40s and macOS in 10m18s.

GitHub cannot dispatch a newly introduced `workflow_dispatch` file until the workflow
exists on the default branch. Therefore PR #137's full generator/stress workflow is
locally reproduced by the complete release matrix above; its scheduled/manual GitHub
entry becomes dispatchable after merge. Ordinary PR CI still runs 128 deterministic
seeds per recipe plus every named regression seed.

The stale MPL allowance is removed. Bevy/platform duplicate dependencies remain
documented where they are intentional. No `__eh_frame` warning appeared in the full
Party Trial links, PR #135's exact release game build, or the foundation's local exact
release build.

### Final gate ledger

| Gate | Command / evidence | Duration | Result |
|---|---|---:|---|
| Format | `cargo fmt --all --check` | 2.468 s | pass |
| Dependency policy | `cargo deny check` | 5.336 s | pass; only documented Bevy/platform duplicates |
| Strict workspace lint | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 5.613 s warm | pass |
| Documentation | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps` | 8.338 s warm | pass |
| Shipping build | `cargo build --package hex_game --release` | 219.162 s | pass; no warning and no `__eh_frame` diagnostic |
| Package isolation | `cargo check --package hex_core` | 0.898 s warm | pass |
| Ordinary triplicate on the settled code tip | `cargo test --workspace --all-features` | 73.204 / 37.951 / 37.050 s | each pass: 1,338 ordinary, 25 ignored |
| Scoped interpreter | pinned-nightly Miri over `hex_core`, `hex_lattice`, and `hex_ai` | 338.209 s | 117 pass, 1 ignored |
| Release stress | nine generator workflow entries, AI/formation/perception/terrain probes, Party Trial 100×, and two 10,000-turn stalemates | generator timings above; soaks 8.619/4.266 s | pass after promoting Mountains seed 6401 |
| Scripted visual walks | seven release-shaped scripts at 1280×720 | 271 s release build; walk steps watchdog-bounded | 176/176 steps, 37/37 captures, all non-black and inspected |
| PR #137 code-tip GitHub gates | CI run `30490536448`; deep run `30490535873` | packages: Linux 7m40s, macOS 10m18s, Windows 22m07s | quick, strict tests/docs, coverage/timing artifacts, three exact packages, pinned Miri, and Linux ASan pass |

The final Markdown-only handoff commit is followed by the same complete ordinary
triplicate and all PR checks without changing tracked files. Their exact-tip results
are recorded in the PR closeout because recording them in this file would itself
create a new untested tip.

## Project-goal divergence

The current code supports the intended foundation: one deterministic world for
exploration and combat, lattices as bodies and spell engines, damage as disabled
cells, incomplete information instead of dice, party formations, and authorized AI
running through the same command funnel as the player.

It is not yet the complete game described by the design:

- terrain spells resolve geometry but do not yet announce material impacts;
- obstruction/line of sight, cover, and unit occupancy are absent;
- bodies occupy one surface even though the data model anticipates footprints;
- combat has no designed rout, surrender, or unreachable-stalemate policy;
- faction knowledge is authoritative headlessly, but fog/picking presentation is not
  implemented;
- initiative, action economy, fight length, death, and balance remain provisional;
- real-time casting, channelling/co-casting, durable saves, audio content, input
  rebinding, signing, storefronts, and telemetry remain product work; and
- V3 composition and legacy-generator removal continue on the active map PR stack.

These are explicit roadmap gaps. Foundation code must continue to fail closed or
expose a named provisional knob; it must not fill them with accidental policy.

## Required visual and play-feel signoff

Automation can reject a stalled or black scripted frame, but it cannot approve
composition, motion, readability, or fun. Before the foundation PR is eligible to
merge, a human must walk the final release-shaped integration tip and record the
artifacts and verdicts:

- [ ] Title, Settings, New Game, Continue, pause, back navigation, and Loading
  transitions render correctly with no stale screen entities.
- [ ] Lattice Demo shows adjacency, disabled cells, enchantment breakage, and readable
  element/status colors.
- [ ] Ability Lab shows Observed-only cast anchors; Remembered/Unknown anchors cannot
  be previewed, cycled, or confirmed.
- [ ] A legal area cast visibly reaches its Observed anchor without revealing hidden
  spillover outcomes in overlays or the combat log.
- [ ] Party Trial completes the exploration-to-combat handoff; the six-member rail,
  initiative, selection ownership, exact-cell decisions, downing, Renewal, and outcome
  screens remain readable.
- [ ] Baseline AI movement and casting look intentional and never visibly react to a
  hidden unit.
- [ ] Open, stacked, narrow, and blocked formation movement has no overlap, snap,
  route discontinuity, or unacceptable hitch.
- [ ] Procedural Hills, Waterfall, Forest, Caves, and every V3 showcase present on the
  final integrated branch have intact seams, blockers, liquids, objects, lighting, and
  cutaways.
- [ ] Camera pan/orbit/zoom, close-character handoff, radius-only collision restoration,
  whole-tree fade transitions, and the dark-lighting handoff have no frame pop or
  idle drift.
- [ ] One ordinary play session confirms movement speed, turn cadence, cast feedback,
  camera feel, and UI density are acceptable as provisional values.

The scripted `menus`, `gameplay`, `ability_lab`, `raider_mirror`, `waterfall`,
`forest`, and Party Trial walks provide repeatable screenshots. The human verdict is a
separate mandatory line item.

## Ranked deferred backlog

1. **P2 — resume digest authority:** make PR #135 save compatibility hash the accepted
   runtime semantic revision, or explicitly forbid saving after content divergence.
2. **P2 — terrain edit churn:** keep the passing regression benchmark; implement
   affected-column and neighboring-seam rebuilds only when the measured entity churn
   becomes an interactive or memory problem.
3. **P2 — remaining perception adapters:** unknown-frontier movement, engagement,
   ordinary attack targeting, and lost-contact search must consume the same faction
   authority already used by AI/casting.
4. **P3 — active-branch Fort stress:** retain the proven Forest feature contract and
   run the Fort fixed release corpus on its owning active branch before that branch
   joins `dev`; Fort is not part of the foundation tip's generator inventory.
5. **P3 — stress observability:** emit exact passing fallback counts and fingerprint
   diversity from Caves, Mountains, Sky, Forest, and Waterfall corpora rather than
   exposing only their enforced `<1%` assertions.
6. **P3 — intentional dependency/platform debt:** revisit documented Bevy/platform
   duplicates and feature trimming during the budgeted Bevy 0.20 window; do not churn
   them without an actionable build or binary-size result.
7. **Design backlog, unranked as defects:** rout/surrender, unit obstruction,
   multi-hex occupancy, terrain magic, fog presentation, and balance need explicit
   design decisions and their own acceptance plans.

There are no known unresolved P0 or P1 defects in the focused implementation at this
snapshot. The mandatory human visual/play-feel verdict and the ranked P2/P3 items
above are not represented as complete.
