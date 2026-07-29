# Foundation hardening

Durable handoff for the July 2026 correctness, scalability, lifecycle, and CI cleanup.
This record describes the implemented playable slice. It does not turn unresolved
design questions into code.

> **Integration status, 2026-07-29:** the focused fixes and their targeted stress
> gates are green and PR-stack propagation is settled. The three consecutive final
> ordinary gates, final release/doc/static gates, scripted visual captures, and GitHub
> platform checks are still in progress. Do not read this intermediate record as final
> signoff.

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

The final inventory discovers 1,362 tests: 1,337 ordinary tests in the complete
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
- Seven release generator corpora now cover 923 deduplicated ordinary inputs and
  pass. The original six covered 789 inputs in 15.13 s of test execution
  (145.09 s including the first ThinLTO build); Mountains seed 52 was their only
  fallback-pressure case and remained below 1%. The added V2 Hills corpus contributes
  134 semantic cases.
- A release V2 Hills run over 10,000 seeds produced zero invalid worlds, zero
  fallbacks, and 10,000 unique fingerprints. Test execution took 63.594 s; the
  first ThinLTO-inclusive command took 388.90 s.
- The full 10,000-seed generator corpora are weekly/manual release jobs. They enforce
  validity and each generator's existing fallback bound; their timings are trends,
  not hardware-sensitive PR failures.

### Toolchain and CI

The branch adds superseded-run cancellation, 10-minute lightweight jobs, 30-minute
build/test jobs, nextest timing and JUnit artifacts, exact
`cargo build --package hex_game --release` validation on Linux/macOS/Windows, and
LLVM coverage artifacts for the headless domain crates. Weekly/manual workflows own
release stress corpora, performance probes, Party Trial/stalemate soaks, scoped Miri,
and supported Linux AddressSanitizer checks.

The stale MPL allowance is removed. Bevy/platform duplicate dependencies remain
documented where they are intentional. No `__eh_frame` warning appeared in the full
Party Trial links or PR #135's exact release game build. The foundation tip still
receives its own exact shipping build in the final gate.

### Final gate ledger

| Gate | Status |
|---|---|
| Focused AI/combat/perception/content/formation/lifecycle regressions | green |
| Strict Clippy for the directly affected crates | green |
| Full workspace format, deny, all-target/all-feature Clippy, warnings-denied docs, exact release game build | pending final integration tip |
| Complete ordinary workspace gate, three consecutive runs | pending final integration tip |
| Scripted walks and frame inspection | pending |
| Foundation PR GitHub CI, coverage, platform packaging, stress, and deep checks | pending PR publication |

The final handoff must replace every pending row with a command, duration, and result.
No final “solid foundation” claim is valid while one remains pending.

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
- [ ] Camera pan/orbit/zoom, close-character handoff, cutaway interpolation, and the
  dark-lighting handoff have no frame pop or idle drift.
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
4. **P3 — V3 stress completion:** retain the proven Forest feature contract and run
   the full release generator corpora, including the Fort fixed corpus intentionally
   skipped by the propagated branch's broad ordinary gate.
5. **P3 — intentional dependency/platform debt:** revisit documented Bevy/platform
   duplicates and feature trimming during the budgeted Bevy 0.20 window; do not churn
   them without an actionable build or binary-size result.
6. **Design backlog, unranked as defects:** rout/surrender, unit obstruction,
   multi-hex occupancy, terrain magic, fog presentation, and balance need explicit
   design decisions and their own acceptance plans.

There are no known unresolved P0 or P1 defects in the focused implementation at this
snapshot. That statement remains conditional on the pending final gate ledger.
