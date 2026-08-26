# Hex Development Problem-Solving Casebook

## Scope and current truth

This casebook distills the complete **Plan hex game visibility** development task
(`019fd5b0-1cfb-7c53-bf05-20c2d5b9db49`) from its initial visibility design through the
active Grand V3 compiler wave. It records both successful and unsuccessful approaches so
that later agents can reuse the procedures without inheriting false confidence from the
original narration.

- **Observation cutoff:** `2026-08-25 23:20:53 PDT` (`wait_threads` cursor
  `ce1699c9-6148-4697-a1e0-685fbf23c6ab:1`, followed by a newest-page refresh).
- **Coverage:** five history pages, 43 unique human turn IDs, and 60 human-authored
  messages. Environment-context injections are excluded. Seven turns contain multiple
  mid-response steering messages: T02, T17, T20, T21, T22, T28, and T43.
- **Source checkout at observation:**
  `/Users/alberto/Documents/Codex/2026-08-05/this/work/crystal-ascent`, branch
  `wave/grand-v3-schematic`, committed checkpoint `dcf917536aaca7e43b9eee35e75141eb442a9c3f`,
  with extensive tracked and untracked Grand V3 work in progress.
- **Latest committed checkpoint:** the full-scale Grand V3 proxy exists at radius 187 with
  105,469 columns and 444 resident chunks. It reached `TerrainReady` in 1.794 seconds and
  measured 1.52 GB maximum RSS, but perception p95 was 56.7 ms against a 50 ms target and
  local-edit p95 was 192.4 ms against a 100 ms target.
- **Last attributable detailed active-state gate:**
  `CARGO_INCREMENTAL=0 HEX_GRAND_PROFILE=1 cargo test -p hex_map --test schematic_compile public_exact_schematic_boundary_compiles_publishes_and_exports -- --exact --nocapture --test-threads=1`
  failed after 5.33 seconds. The exact error was: `ordinary cell 19 has no dry same-region
  connector to the foothill network (498 candidates, lower/upper attempts [0, 24], 24 paths
  reached the network)`. Earlier attempts failed for cells 63 and 92. This is a verified
  failure of that exact dirty state, not a final judgment on the unfinished feature. The
  task made further route changes afterward. At the final refresh it was still in progress
  with 348 summarized items; its newest task snapshot exposed a later failed command marker
  (`exec-621df9d5-4104-47f4-84e4-4cf9c08f`) but not enough output to classify that command.
  Therefore cell 19 is the last detailed gate, not a claim about the newest dirty files.
- **Active-turn warning:** every statement about T43 is provisional through the cutoff.
  A temporary 8.7-second end-to-end compile succeeded earlier in T43, but subsequent
  stronger invariants and route changes exposed new failures. Later exact-state evidence
  governs current truth.

Proof dispositions in this document are `verified-success`, `verified-failure`,
`provisional`, and `superseded`. Evidence strength is tracked separately as `acceptance`,
`direct`, `diagnostic`, `trace`, or `inference`. A user correction or an independent
reference oracle outranks an assistant's success summary; a still image cannot prove
motion; a self-consistent generated fixture cannot validate its own transcription.

## Turn and steering coverage

Each row covers one stable human turn. Semicolon-separated numbered clauses represent
distinct human messages within that turn, so the message counts sum to 60.

| Turn | Stable turn UUID | Human messages | Intent, correction, or decision |
|---|---|---:|---|
| T01 | `019fd5b0-1d65-7722-a5d4-9d114c16c891` | 1 | Plan light- and obstruction-aware visibility after inspecting existing repository contracts. |
| T02 | `019fd5d9-50d9-7861-bef9-03d9fcd91490` | 2 | (1) Implement the approved tactical-shroud plan; (2) continue autonomously during an eight-hour absence unless completely blocked. |
| T03 | `019fd791-08a2-7b73-b2d3-26f525b44246` | 1 | Open the implemented game for direct inspection. |
| T04 | `019fd792-6a7f-7d73-ae39-f8019645334f` | 1 | Diagnose missing text shown in a screenshot and persist the prevention rule. |
| T05 | `019fda59-97d3-7a32-b6eb-ed27e35a7a6f` | 1 | Reconsider the visibility rule because close steps hid nearby tiles; research and plan a less brittle alternative. |
| T06 | `019fda7d-ffc5-75e1-9711-cea0516c74c2` | 1 | Implement the approved character-volume seven-ray visibility bundle. |
| T07 | `019fdcdd-a1c2-7ae2-a0c0-851cd2490537` | 1 | Open the game to inspect the revised shading. |
| T08 | `019fdce8-7f08-7090-9f57-9620461d3046` | 1 | Report that the visible result still failed the pillar test and require correction. |
| T09 | `019fdd62-b04a-70d1-ac50-5fdd9ba422b3` | 1 | Accept the correction and publish the accumulated work as PRs. |
| T10 | `019fddac-2319-78c1-9d4d-77493824e82f` | 1 | Plan a first-person view alongside map and third-person views. |
| T11 | `019fddc8-c643-7a03-820e-4363578afaf2` | 1 | Select the recommended A/A/A camera, controls, and lens choices. |
| T12 | `019fddd5-889f-7913-b551-03cd2ad09788` | 1 | Implement the approved first-person plan. |
| T13 | `019fdeaa-935e-7482-919c-d20be75573f6` | 1 | Open the first-person implementation for direct play. |
| T14 | `019fdeb1-1090-7c31-a12a-db9b153b6840` | 1 | Question why partially visible first-person tiles could still carry tactical shading. |
| T15 | `019fdf2b-830e-7cf1-98c1-3f41d03f6ce2` | 1 | Accept the first-person behavior and request a PR. |
| T16 | `019fe1b9-4fe6-7bf2-8321-7b087433fba2` | 1 | Design Crystal Ascent from three sketches as a tall authored biome and future map component. |
| T17 | `019fe1db-27fe-7bd0-a63d-ac757962f910` | 2 | (1) Implement the approved Crystal Ascent plan; (2) reach a safe checkpoint before connectivity was lost. |
| T18 | `019fe20f-f543-7500-8bb0-194471b27c9a` | 1 | Investigate the disk-space crisis, especially a 36+ GB first-person worktree, and remove only safe material. |
| T19 | `019fe216-4326-7400-9b72-3dee73357bf8` | 1 | Resume Crystal Ascent development after capacity recovery. |
| T20 | `019fe247-6923-7f42-82a8-66a13b81acb2` | 2 | (1) Resume work; (2) find another safe stopping point before the user went offline. |
| T21 | `019fe313-bcc4-7cc1-a8a2-c21d54f39fd6` | 3 | (1) Continue; (2) open the game while work continued; (3) stop safely within five minutes. |
| T22 | `019fe6d4-cccb-7662-a36e-4b94456f55f3` | 3 | (1) Restart work; (2) show the biome; (3) refine the pyramid-like crystal, thin stairs, and wide summit aperture. |
| T23 | `019fea62-d03d-74c1-a5f6-f4d2c6b49c89` | 1 | Open the refined Crystal Ascent for inspection. |
| T24 | `019feb7f-7a6d-76b3-86c0-5316ee584b37` | 1 | Accept the biome and request its PR. |
| T25 | `019feb93-244b-7332-87e8-95fd7bb6c771` | 1 | Place Crystal Ascent inside a mountain/wooded-valley map connected by a tunnel beneath multiple biomes. |
| T26 | `019feb93-b6e7-7d10-ab74-e09b767b9644` | 1 | Require planning first because a feature had never crossed multiple biomes. |
| T27 | `019feba8-9a0a-7ec2-912b-ca8e7417212d` | 1 | Implement the approved Crystal Mountain and cross-biome tunnel plan. |
| T28 | `01a01dd1-a5a6-7e23-a62b-338b09f24ba0` | 5 | (1) Continue; (2) report progress and next steps; (3) finish Crystal Mountain, then create desert and island suites while the user was offline; (4) give a quick status; (5) finish and produce a visual approval report, then open the game. |
| T29 | `01a02355-1e7f-78e1-9587-aac4730e2770` | 1 | Reopen the accidentally closed game. |
| T30 | `01a02365-d53f-7781-8fe7-27ed0f096191` | 1 | Report a void gap, crystal flicker, flat oasis water, and weak island relief; require verification, fixes, and earlier detection. |
| T31 | `01a02549-e704-7740-b60d-3203c18b260c` | 1 | Continue the corrective pass. |
| T32 | `01a0260d-eeeb-7372-b5a5-80f794496c92` | 1 | Accept the corrections and publish the map-suite PRs. |
| T33 | `01a03465-ad14-7053-ab1f-2f5aa29ba07f` | 1 | Plan the schematic-authoring process for the largest V3 map before voxel generation. |
| T34 | `01a034a9-83ba-7280-af6e-09e011aa1f44` | 1 | Implement the approved Grand V3 schematic planner. |
| T35 | `01a035ea-279f-70d0-b374-6e7c400ee837` | 1 | Show a render of the claimed manual trace for each seed as a sanity check. |
| T36 | `01a035ec-264d-7421-942d-497bb471e110` | 1 | Point out that the source and generated images disagreed on the lake, massif, islands, and most geography. |
| T37 | `01a0362b-5abe-76b3-abf7-94cf456793ab` | 1 | Define uncolored hexes as outside the radius-eight map. |
| T38 | `01a0362d-c563-7692-b3fa-99756e794942` | 1 | Require an exact cell-type reproduction of the source before further generation work. |
| T39 | `01a03659-7f5c-7630-925b-a852d6eb04a4` | 1 | Approve the exact transcription and identify the generated peak positions as wrong. |
| T40 | `01a03944-fbc1-77f0-b3f5-0a1bbf92aa9f` | 1 | Plan how valid schematics should compile into basic playable maps. |
| T41 | `01a039a0-30e4-7330-99b0-f597c4de7754` | 1 | Correct the planning process because the agent had stopped asking material questions. |
| T42 | `01a039ea-931a-73d0-9680-3042e2f8cad9` | 1 | Implement the approved Grand V3 schematic-to-map compiler plan. |
| T43 | `01a03b42-3ba3-71c1-a2e6-12b2d79e990f` | 7 | (1) Approve the full-scale proxy architecture and budgets; (2) request a quick progress report; (3) ask for current screenshots; (4) reject a cropped screenshot presented as the map; (5) require a reusable render-inspection skill; (6) request an eight-hour work plan; (7) prioritize a playable baseline, then controlled visual experiments. |

## Evidence register

| Evidence | Observed claim | Source locator and exact state | Strength | Limitation |
|---|---|---|---|---|
| EV01 | Initial visibility implementation reported exact LOS, fog, performance, and idle-frame gates. | T02; commits `2f89d88` and `d759f5c`. | trace | Assistant-reported gates; later gameplay counterexamples narrowed the claim. |
| EV02 | Direct binary launch omitted Bevy's asset-root setup; Cargo launch restored text and assets. | T03–T04; fresh launch log; commit `89e26d7`. | direct, diagnostic | Proves the observed launch failure and recovery, not every future asset issue. |
| EV03 | Seven-ray character-volume implementation met reported focused and performance gates. | T05–T06; commit `5c590a3`. | trace | Pillar screenshot in T08 later showed the gameplay rule was still incomplete. |
| EV04 | Full-depth terrain runs incorrectly treated grounded steps as walls; the low-cover correction passed and the user said it worked. | T07–T09; screenshot correction; commit `d3ec9e7`. | acceptance, diagnostic | Acceptance applies to the tested visual behavior, not every LOS geometry. |
| EV05 | Visibility work was pushed as draft PRs #186–#189 with documented ordering. | T09. | direct | CI had started; not all human review gates were complete. |
| EV06 | First-person camera implementation and lifecycle gates were reported at a stable commit. | T10–T15; commit `15102e2`; PR #190. | trace, acceptance | Native feel was explicitly pending until the user accepted the visible behavior. |
| EV07 | Crystal Ascent reached a committed, focused-green but non-launch-ready checkpoint. | T16–T17; commit `4e3214d`. | direct | Full integration, release, and visual gates remained. |
| EV08 | About 55 GiB was recovered by deleting duplicated Cargo targets and four clean pushed worktrees; ignored captures in those worktrees were also lost. | T18; 2.7 GiB to 53 GiB free. | direct | The source branches survived, but local evidence deletion makes this a mixed recovery. |
| EV09 | Later checkpoint reporting “full geometry” was followed by an audit requiring exact four-wide transfers and radius-32 footprint fixes. | T20–T21; commits `b05b073`, `f69b7de`. | direct | Shows checkpoint safety and also why a status label is not final acceptance. |
| EV10 | Human visual review drove a taller irregular crystal, deep stair haunches, and a smaller oculus. | T22–T24; commit `e59cc24`; PR #197. | acceptance | Final merge state and all motion qualities were outside this episode. |
| EV11 | The overnight map suite reported 67 captures and green automated gates. | T28; clean head `09bc0eb`. | trace | Later live review found defects that the report did not expose. |
| EV12 | The user found a Crystal base void, moving crystal flicker, flat oasis water, and weak island relief. | T30 user correction. | acceptance, direct | Screenshot plus motion observation; exact defect cause required diagnosis. |
| EV13 | Corrective work added a base plinth, removed coplanar crystal faces, recessed oasis water, increased island relief, and hardened review. | T30–T32; commits `a6fd6e6`, `63aed36`; PRs #210–#213. | direct, acceptance | Native moving-camera flicker remained an explicit manual gate at publication. |
| EV14 | The first Grand schematic implementation passed 256- and 10,000-seed self-consistency corpora but used an approximate, mislabeled “manual trace.” | T33–T36; commit `2ec7fff`. | direct | Generated tests used the wrong template as their oracle. |
| EV15 | Side-by-side user comparison exposed the wrong massif, lake, islands, tunnel, and Crystal spacing. | T35–T38. | acceptance | The correction still required an exact transcription and approval. |
| EV16 | A literal 217-cell transcription was approved, then revision 2 corrected the peak chains and regenerated both corpora. | T38–T39. | acceptance, direct | Validates the radius-eight reference contract, not later voxel compilation. |
| EV17 | The user noticed that implementation planning had stopped asking material questions; the recovered plan added a full-scale proxy and approval checkpoint. | T40–T42. | acceptance | Process correction, not implementation proof. |
| EV18 | The full-scale proxy established real memory, timing, perception, and edit costs without shrinking the world. | T43; commits `03c3a0c`, `dcf9175`. | direct | Content and final rendering were intentionally incomplete. |
| EV19 | Stage profiling found a 37.4-second quadratic route-sealing scan; a coordinate index reduced it to 24.7 ms and exact compile reached topology validation in 3.55 seconds. | T43 active dirty state. | direct, diagnostic | Later edits changed the exact dirty state; the total compiler still missed its target. |
| EV20 | Parallel optimized builds repeatedly filled the disk; narrow incremental-cache cleanup restored room temporarily. | T43 active dirty state; reported 14 GiB artifacts and less than 1 GiB free. | direct, diagnostic | Capacity recovery did not eliminate the concurrency pattern that recreated pressure. |
| EV21 | A cropped proxy image was presented as a whole-map view and immediately rejected; the camera hint was then traced to a hand-tuned proxy pose. | T43 user correction and bounds diagnosis. | acceptance, diagnostic | A corrected full-map capture had not yet passed at the cutoff. |
| EV22 | Stronger fail-closed route validation successively exposed cells 63, 92, and 19. | T43; last attributable exact `schematic_compile` output quoted in Current truth. | direct | Further route edits followed; the final refresh showed a newer failed command marker without classifiable output. |

## Episode cards

### A01 — Establish the visibility contract (T01)

- **Goal and constraints:** Preserve the visible map, shade tiles outside allied sight, hide
  enemies there, combine target lighting with physical obstruction, use a head-height origin,
  and count exact tangencies as clear.
- **Worked:** Repository-first inspection found perception, lighting, occupancy, fog, and
  knowledge contracts that could be extended instead of duplicated.
- **Failed or unproved:** This was a plan, so no implementation or gameplay claim was yet
  proved. The initial point-origin/corner model had not been tested against close steps.
- **Exposure:** Later T05 and T08 screenshots supplied production-shaped counterexamples.
- **Recovery:** The plan became a concrete exact-geometry implementation, then was revised
  when live geometry contradicted its assumptions.
- **Evidence:** T01 and the accepted plan beginning T02; EV01.
- **Outcome:** `provisional` as design; useful repository grounding.
- **Reusable rule:** Before designing a cross-system feature, trace existing ownership,
  invalidation, data, and presentation boundaries; record untested gameplay assumptions as
  hypotheses rather than requirements proved by construction.

### A02 — Implement tactical shroud without losing exactness (T02)

- **Goal and constraints:** Add 36/12/1 illumination-selected range, exact strict-interior
  terrain LOS, fog caps, hostile hiding, and same-frame invalidation while meeting runtime
  budgets.
- **Worked:** The implementation kept integer geometry, composable occlusion, fail-closed
  occupancy, range-first filtering, explicit benchmarks, and idle recomputation checks.
- **Failed or unproved:** Passing its own exact-head and performance tests did not prove that
  the chosen visibility abstraction felt correct around stairs and pillars.
- **Exposure:** T05 showed nearby shaded tiles; T08 showed the revised version still failed the
  pillar test.
- **Recovery:** Preserve the integration work but supersede the gameplay sampling and
  grounded-step interpretation in A04–A05.
- **Evidence:** EV01, later counterevidence EV03–EV04.
- **Outcome:** `superseded` for gameplay adequacy; reported performance/integration success is
  narrower than the original completion claim.
- **Reusable rule:** Separate implementation correctness, performance, and gameplay-rule
  adequacy. A green suite on synthetic geometry cannot close a feature whose acceptance is
  visual and spatial.

### A03 — Restore assets by launching through Cargo (T03–T04)

- **Goal and constraints:** Explain why UI outlines rendered but text and icons did not, then
  prevent recurrence.
- **Worked:** The log and launch path showed that executing `target/release/hex_game` directly
  bypassed `BEVY_ASSET_ROOT`. `cargo run --release -p hex_game` restored the asset context; the
  fresh log had no missing-asset errors.
- **Failed or unproved:** “Optimized binary exists” was incorrectly treated as equivalent to
  “production-shaped launch.”
- **Exposure:** The user's screenshot made the missing-text symptom undeniable.
- **Recovery:** Relaunch through Cargo, inspect the startup log, and add the exact symptom and
  launch rule to repository instructions in `89e26d7`.
- **Evidence:** EV02.
- **Outcome:** `verified-success` for this failure mode.
- **Reusable rule:** Launch Rust/Bevy review builds through the repository-supported Cargo
  entrypoint and verify startup logs before interpreting UI screenshots.

### A04 — Replace point vision with a character-volume ray bundle (T05–T06)

- **Goal and constraints:** Make nearby terrain more permissive without arbitrary radius
  exceptions or the cost/keyholes of fully permissive area-to-area sight.
- **Worked:** Research and domain comparison led to one center ray plus six paired body-corner
  rays, exact tangency semantics, range-first filtering, focused counterexamples, and measured
  budgets.
- **Failed or unproved:** The implementation retained an incorrect full-depth interpretation
  for some terrain runs. The seven-ray suite could still agree with the wrong blocker model.
- **Exposure:** The live pillar test in T08 contradicted the reported green state.
- **Recovery:** Keep the sampling bundle but repair grounded low-cover semantics in A05.
- **Evidence:** EV03 and counterevidence EV04.
- **Outcome:** `superseded` as a complete fix; retained as a useful component.
- **Reusable rule:** When changing a spatial rule, vary both sampling geometry and production
  blocker representation in fixtures. More rays do not correct a wrongly modeled obstacle.

### A05 — Let the pillar test outrank a green suite (T07–T09)

- **Goal and constraints:** Make grounded one-level steps behave as low cover while retaining
  two-level walls, cliffs, roofs, and decks as blockers.
- **Worked:** Diagnosis isolated fog LOS against full-depth terrain runs rather than lighting.
  A targeted correction passed focused and full gates, and the user explicitly confirmed the
  visible result.
- **Failed or unproved:** The preceding suites lacked the exact production-shaped pillar/step
  oracle and therefore certified the wrong abstraction.
- **Exposure:** User screenshot and direct “does not pass the pillar test” correction.
- **Recovery:** Encode the user-approved fixture as regression coverage and preserve the
  distinction between grounded low cover and true character-height blockers.
- **Evidence:** EV04; publication EV05.
- **Outcome:** `verified-success` for the pillar/step behavior.
- **Reusable rule:** An exact user-approved counterexample becomes a permanent oracle. Never
  argue from broader test counts against a narrower relevant failure.

### A06 — Publish a dependency-aware PR stack (T09)

- **Goal and constraints:** Deliver visibility and adjacent completed work without hiding
  overlaps or pending human gates.
- **Worked:** Four draft PRs were pushed, their order and additive overlaps were stated, local
  captures were excluded, and tickets linked to the PRs.
- **Failed or unproved:** “Mergeable” did not mean all CI and human review were complete; some
  presentation/play checks remained named.
- **Exposure:** No failure in this episode; the risk was premature readiness language.
- **Recovery:** Keep PRs draft and describe required ordering and outstanding gates.
- **Evidence:** EV05.
- **Outcome:** `verified-success` for publication mechanics; final landing remained
  `provisional`.
- **Reusable rule:** For stacked delivery, record exact reviewed SHA, target, dependency order,
  remote-head match, CI state, and human-only gates separately.

### A07 — Add first person as a presentational camera mode (T10–T15)

- **Goal and constraints:** Add Map → Third Person → First Person cycling while preserving
  tactical input, map-pose restoration, selection authority, fog, and character hiding.
- **Worked:** Material choices were asked explicitly; A/A/A fixed the interface before coding.
  The implementation added lifecycle, malformed-state, performance, and deterministic capture
  coverage at `15102e2`. The user inspected it and requested PR #190.
- **Failed or unproved:** Automated captures did not initially explain why visible physical
  fragments could belong to tactically shaded tiles. Native feel was not automated.
- **Exposure:** T14 screenshot and question about the expected subset relation.
- **Recovery:** Trace the two different predicates—depth-tested fragments versus tactical
  center/three-corner observation—and state the exact condition that would constitute a bug.
- **Evidence:** EV06.
- **Outcome:** `verified-success` by user acceptance for the shipped camera behavior; semantic
  communication remained a design consideration.
- **Reusable rule:** When two views expose the same world through different predicates, write
  their relationship explicitly and test it; do not infer subset equivalence from visual
  intuition.

### A08 — Preserve semantic distinctions instead of “fixing” partial visibility (T14–T15)

- **Goal and constraints:** Decide whether first-person visible fragments require a tile to be
  tactically unshaded.
- **Worked:** The investigation separated mesh visibility, top-cap coverage, light-selected
  range, LOS sampling, and eye origin. It avoided weakening enemy visibility to any single
  fragment.
- **Failed or unproved:** The translucent 84% lifted cap could still communicate partial
  visibility poorly; the explanation did not itself prove every pictured wedge's identity.
- **Exposure:** The user's direct screenshot and subset question.
- **Recovery:** Define a falsifiable bug rule and keep a possible partial-visibility
  presentation state separate from gameplay observation.
- **Evidence:** T14 explanation and T15 acceptance; EV06.
- **Outcome:** `verified-success` as an accepted semantic resolution, with presentation polish
  `provisional`.
- **Reusable rule:** Diagnose mismatched views by enumerating each predicate and authority.
  Change gameplay only when the authoritative condition—not merely a fragment—is violated.

### A09 — Build Crystal Ascent through explicit safe checkpoints (T16–T17)

- **Goal and constraints:** Create a 100–200-level deterministic authored landmark with exact
  stairs, lighting, occupancy, terminals, and later composition readiness.
- **Worked:** The plan defined geometry, routes, headroom, lights, blockers, rotations, and
  acceptance before implementation. A committed checkpoint survived imminent disconnection
  and was honestly labeled non-launch-ready.
- **Failed or unproved:** Disk exhaustion interrupted development, and focused tests did not
  establish runtime dispatch, full route fidelity, release performance, or visual quality.
- **Exposure:** The imminent offline deadline and capacity failure forced an explicit boundary.
- **Recovery:** Commit coherent source, record exact remaining work, and resume only after a
  separate capacity audit.
- **Evidence:** EV07.
- **Outcome:** `provisional`, but a verified safe checkpoint.
- **Reusable rule:** Before interruption, commit only coherent work, run the cheapest relevant
  gate, state what is not ready, and leave an exact restart note.

### A10 — Recover Cargo capacity without confusing source and artifacts (T18, T43)

- **Goal and constraints:** Recover disk while preserving active work, branches, PRs, task
  history, and evidence.
- **Worked:** Size inspection distinguished ~38 MiB of first-person source from ~31 GiB of its
  Cargo `target`, plus ~19 GiB in the active target. Removing derived outputs recovered about
  55 GiB; later narrow incremental-cache cleanup also restored working room.
- **Failed or unproved:** Four worktrees were removed and ignored visual captures inside them
  were lost. Parallel optimized builds later recreated 14 GiB of disposable artifacts and
  drove free space below 1 GiB. Cleanup treated the symptom more effectively than the build
  concurrency cause.
- **Exposure:** `df`/size inventory, repeated ENOSPC behavior, and the explicit admission that
  exact local captures were deleted.
- **Recovery:** Protect all dirty, untracked, ignored, and unpushed evidence; inventory every
  worktree and effective Cargo target; serialize expensive profiles; disable incremental work
  where appropriate; clean only exact inactive tagged cache paths; then rerun the identical
  verification rather than narrowing it.
- **Evidence:** EV08 and EV20.
- **Outcome:** `verified-success` for reclaimed bytes, `superseded` as a recommended end-to-end
  procedure because evidence was lost and pressure recurred.
- **Reusable rule:** Disk pressure never authorizes weaker verification. Audit first, preserve
  evidence, remove only exact reproducible Cargo artifacts, prevent duplicate builds, and
  resume the same gate.

### A11 — Iterate Crystal Ascent from contracts through visual taste (T19–T24)

- **Goal and constraints:** Complete the landmark, preserve work across interruptions, then
  refine its crystal, stairs, and aperture from direct review.
- **Worked:** Repeated checkpoints kept runtime, LOS, fog, lifecycle, geometry, and camera work
  recoverable. User feedback produced a 30-level irregular prism, 2/4/6/8-deep stair haunches,
  and an oculus reduction from radius 12 to 11 before PR #197.
- **Failed or unproved:** An intermediate “full geometry” report preceded an audit that still
  required four-wide circuit-transfer and radius-32 footprint corrections. Visual taste could
  not be inferred from geometric contract tests.
- **Exposure:** Later internal audit plus the user's live screenshot feedback.
- **Recovery:** Keep intermediate status scoped, turn feedback into exact geometry changes and
  regression contracts, recapture, and wait for user approval before publication.
- **Evidence:** EV09–EV10.
- **Outcome:** `verified-success` for the user-reviewed authored biome and PR state; intermediate
  completion claims are `superseded`.
- **Reusable rule:** Report “green” by subsystem and exact SHA. For authored assets, geometry
  tests prove safety while user-visible captures prove shape and taste; both are required.

### A12 — Compose a cross-biome tunnel as a world-owned feature (T25–T28)

- **Goal and constraints:** Pass one four-wide tunnel beneath several independently generated
  mountain biomes into Crystal Ascent while preserving ownership, water, lighting, routes, and
  final topology.
- **Worked:** The plan established the durable sequence: reserve the global passage and claims,
  generate local biome content, merge fragments, carve the tunnel once across the combined
  volume, and run one final validation. Typed anchors, unified interior ownership, and exact
  route/light contracts prevented local generators from silently overwriting the feature.
- **Failed or unproved:** The overnight delivery and its 67 captures appeared complete but did
  not expose a void halo, moving crystal flicker, or all presentation defects.
- **Exposure:** T30 direct live review.
- **Recovery:** Keep the composition architecture, add support/flicker corrections and stronger
  visual gates in A13, and never use the initial report as final evidence.
- **Evidence:** EV11–EV13.
- **Outcome:** `verified-success` for the architectural pattern; initial visual completion is
  `superseded`.
- **Reusable rule:** Cross-region features need global reservation, local generation, atomic
  carve/finalization, then combined-world validation. No local biome owns a world-spanning
  invariant.

### A13 — Upgrade visual review after holes and flicker escaped (T29–T32)

- **Goal and constraints:** Fix the user-reported map defects and change the workflow so they
  are found before approval.
- **Worked:** Diagnosis tied the base gap to missing support, flicker to coplanar faces, oasis
  flatness to unrecessed water, and island weakness to insufficient relief. Exact mesh tests,
  a support plinth, new captures, a full contact sheet, and a fresh-eyes review improved both
  code and evidence.
- **Failed or unproved:** The original 67-image report favored static coverage and did not
  establish full-resolution inspection or motion quality. Even after fixes, native
  camera-motion flicker remained an explicit manual gate.
- **Exposure:** The user's visual and motion observations in T30.
- **Recovery:** Invalidate stale images after material changes; inspect full-resolution stills
  from multiple modes and azimuths; add a separate orbit/motion pass; retain manual gates that
  stills cannot close.
- **Evidence:** EV11–EV13.
- **Outcome:** `verified-success` for static corrected geometry and user acceptance; motion was
  still `provisional` at PR publication.
- **Reusable rule:** Static stills, contact sheets, and motion are complementary evidence. Never
  infer “no flicker” from screenshots.

### A14 — Learn that self-consistency does not validate a traced reference (T33–T36)

- **Goal and constraints:** Convert a hand-drawn radius-eight schematic into a deterministic,
  constrained generator with locked, bounded, and seeded regions.
- **Worked:** The implementation produced a pure planner, stable fingerprints, provenance,
  candidate scoring, and large deterministic corpora with no invalid outputs or fallbacks.
- **Failed or unproved:** The source template was an approximate reinterpretation mislabeled as
  a manual trace. The generated plans and tests all agreed with the same wrong oracle, so 256-
  and 10,000-seed green corpora proved internal consistency only.
- **Exposure:** Side-by-side user comparison showed the massif and lake were swapped, Crystal
  spacing was wrong, the tunnel direction was guessed, and islands were approximate.
- **Recovery:** Admit the evidence failure, freeze generation changes, and require a literal
  coordinate-registered transcription before updating rules.
- **Evidence:** EV14–EV15.
- **Outcome:** `superseded` for reference fidelity; reusable planner mechanics survived.
- **Reusable rule:** A generated fixture cannot be its own reference oracle. Validate the input
  transcription independently before spending effort on downstream validity and scale tests.

### A15 — Trace and approve the exact 217-cell oracle (T36–T39)

- **Goal and constraints:** Make every output hex map to the same semantic cell type as the
  user's drawing; blank cells are outside the map.
- **Worked:** A literal 217-cell transcription was shown before planner mutation. The user
  approved it, which made the later peak mismatch observable. Revision 2 then locked two
  six-cell peak chains, the waterfall opening, frozen-shore contact, and exact reference map.
- **Failed or unproved:** Earlier visual similarity, a blank labeled grid, and deterministic
  outputs had been treated as trace evidence when no registered overlay existed.
- **Exposure:** The user's exact cell-by-cell acceptance and subsequent peak correction.
- **Recovery:** Version the approved oracle, regenerate masks and constraints from it, and rerun
  both small and release corpora while keeping the trace itself unchanged.
- **Evidence:** EV15–EV16.
- **Outcome:** `verified-success` for the schematic reference contract.
- **Reusable rule:** Use the sequence reference → exact trace → human approval → implementation
  → independent comparison. Never revise the oracle silently to make generated outputs pass.

### A16 — Recover a planning conversation and add a budget checkpoint (T40–T42)

- **Goal and constraints:** Plan a radius-187 voxel compiler while preserving old maps,
  fingerprints, cameras, editing, saves, LOS, and full intended scale.
- **Worked:** After the user's process correction, planning resumed and locked a chunk-native
  full-scale proxy, exact compiler boundary, global height field, 105,469-column footprint, and
  a mandatory performance approval before decoration. The plan explicitly prohibited silent
  shrinking.
- **Failed or unproved:** The agent initially stopped asking material implementation questions,
  which risked encoding unapproved architecture and budgets.
- **Exposure:** Direct user steer in T41.
- **Recovery:** Resume intent chat, surface high-impact tradeoffs, and make the proxy checkpoint
  an interface between feasibility and final content.
- **Evidence:** EV17 and the accepted T42 plan.
- **Outcome:** `verified-success` as an accepted decision-complete plan.
- **Reusable rule:** For unprecedented scale, ask until ownership, topology, performance
  budgets, fallback policy, and approval checkpoints are explicit. Do not convert silence into
  consent.

### A17 — Measure the real full-scale proxy before decorating (early T43)

- **Goal and constraints:** Test the approved radius, pitch, chunk count, entity count, memory,
  generation, perception, and edit architecture without reducing fidelity.
- **Worked:** The proxy used the full 105,469 columns and 444 chunks, producing concrete
  `TerrainReady`, RSS, perception, and edit measurements. It stopped before final content and
  requested architecture/budget approval instead of hiding two misses.
- **Failed or unproved:** Perception and local edits exceeded their targets; a proxy render was
  not yet final-world evidence.
- **Exposure:** Direct release-shaped budget measurements.
- **Recovery:** The user approved combined solid chunk meshes plus exact picking while retaining
  the original performance requirements; perception stayed a separate optimization lane.
- **Evidence:** EV18.
- **Outcome:** `verified-success` as a feasibility checkpoint, not as feature completion.
- **Reusable rule:** Build the cheapest production-scale proxy that preserves cardinality and
  architecture. Measure before decoration, publish misses numerically, and require approval for
  any architectural response.

### A18 — Keep the live Grand compiler fail-closed (active T43)

- **Goal and constraints:** Complete chunk rendering/picking, perception indexing, hydrology,
  bridges, tunnel, Crystal integration, ordinary routes, captures, and a playable baseline
  without shrinking the world or weakening gates.
- **Worked:** Parallel lanes uncovered coarse coastlines, hard-coded passes, light/route
  projection errors, and tunnel-contract gaps before review. Profiling replaced a 37.4-second
  quadratic scan with a 24.7 ms indexed pass. Exact validators now cover three-lane hydrology,
  two bridges, four-wide tunnel body, clearance, roof thickness, materials, unified interior,
  route cuts, and bounds-derived camera framing.
- **Failed or unproved:** Parallel optimized builds repeatedly exhausted disk. A cropped proxy
  was shown as if it covered the whole map and was withdrawn after user correction. An
  8.7-second exact compile was only temporary; later strengthening exposed cells 63, 92, and
  currently 19. No fresh full-map multi-angle/motion pack or playable candidate had passed at
  the cutoff.
- **Exposure:** Stage timings, exact compiler failures, user rejection of the screenshot, and
  capacity measurements.
- **Recovery:** Preserve every failure as a targeted production-shaped regression, optimize
  measured stages rather than guesses, derive framing from exact bounds, use the render skill,
  serialize expensive builds, and keep final verification unchanged.
- **Evidence:** EV19–EV22.
- **Outcome:** the last attributable route gate is `verified-failure`; the newer dirty state
  and overall episode are `provisional through 2026-08-25 23:20:53 PDT`.
- **Reusable rule:** In a live large-world integration, each stronger validator may supersede a
  prior green claim. Report the newest exact failing gate, never the most encouraging earlier
  checkpoint.

## Cross-cutting worked-versus-failed patterns

| Worked | Failed or misled | Durable interpretation |
|---|---|---|
| Exact user-approved reference artifacts | Approximate trace plus self-consistent generator tests | Freeze and approve an independent oracle before generation or corpus testing. |
| Production-shaped pillar, step, and exact-world fixtures | Convenient synthetic geometry that shared implementation assumptions | Promote every relevant counterexample into a regression fixture. |
| Repository-supported Cargo launch plus fresh startup-log inspection | Running a compiled Bevy binary directly | Reproduce the actual asset/runtime environment before diagnosing presentation. |
| Global reservation, local generation, one atomic carve/finalize | Letting each biome independently approximate a spanning feature | World-spanning topology needs one world-owned authority and combined validation. |
| Full-scale proxy with explicit numerical approval gate | Late performance discovery or silent map shrinking | Preserve scale, measure early, and separate architecture approval from content completion. |
| Stage profiling and coordinate indexes | Guessing at the expensive phase or adding more search breadth | Instrument first; optimize the measured complexity without weakening invariants. |
| Exact SHAs, commands, test counts, profile/features, and outputs | “All tests pass” without proving which tests or state ran | Every success claim must identify state and gate; zero-match filters are not passes. |
| Safe committed checkpoints with explicit remaining work | Calling a focused-green subsystem “complete” | Checkpoint status and proof disposition are separate fields. |
| Full-resolution multi-angle stills plus motion and human review | Cropped, stale, single-angle, or static-only evidence | Match evidence to claim: stills for static layout, motion for flicker/popping, humans for taste. |
| Read-only inventory and serialized build strategy | Deleting whole worktrees or evidence and then recreating parallel caches | Recover capacity without touching source/evidence, then prevent the workload pattern that caused pressure. |
| Fail-closed typed topology and geometry validators | Screenshot plausibility and endpoint/label checks | Graph, ownership, clearance, flow, and route-cut properties must be proved directly. |
| Preserving user steers and later reclassification | Rewriting history into a smooth success narrative | Keep original claim, counterevidence, recovery, and successor linked. |

## Promoted skill map

| Skill or pointer | Trigger and scope | Supporting episodes | Required workflow and proof | Home |
|---|---|---|---|---|
| `distill-development-task` | Analyze another active or completed Codex development task and derive reusable practices. | A01–A18 | Read every page and human steer; correlate claims with exact state; preserve invalidations; classify proof and cause independently; refresh an active source immediately before cutoff. Proof is a complete turn/message ledger, evidence register, and contradiction queue. | User-global observer skill. Read-only by default. |
| `trace-authored-map-reference` | A map, layout, pixel sketch, or diagram must become semantic grid data. | A14–A15 | Register the grid; transcribe every cell; treat blanks explicitly; render the trace alone; obtain human approval; version/fingerprint it; only then generate variants and compare against the frozen oracle. Stop if orientation or outside-map meaning is ambiguous. | Repository-local skill. |
| `develop-large-world-features` | A feature crosses biomes/patches or a world is materially larger than prior production maps. | A12, A16–A18 | Assign global authority; reserve claims; generate local content; merge and carve atomically; validate topology; build a full-cardinality proxy; measure and obtain budget approval; add decoration only after the checkpoint. Proof includes exact-world and corpus gates plus release-shaped budgets. | Repository-local skill. |
| `recover-cargo-capacity` | Cargo tests/builds are threatened by ENOSPC, duplicated targets, or worktree cache pressure. | A10, A18 | Audit bytes/inodes, every worktree, dirty/untracked/ignored/unpushed state, effective target roots, active Cargo processes, and exact `CACHEDIR.TAG`; select the smallest inactive reproducible artifact; obtain authorization; revalidate path identity; clean only the exact target; rerun the identical gate. Never delete source/evidence or weaken verification. | Repository-local skill with a read-only audit script. |
| `inspect-game-renders` | Material map/render changes produce a new candidate or a claimed visual fix. | A03, A05, A07–A08, A11, A13, A18 | Launch through Cargo; record exact SHA/dirty state; invalidate stale captures; fit a complete overview from bounds; capture multiple modes/azimuths and targeted details; inspect full resolution and contact sheet; run an independent motion/orbit pass; retain explicit human taste gates. | Repository-local skill. |
| Thin instruction pointers | Any agent begins map, render, Cargo recovery, or retrospective work. | All | `AGENTS.md` and `CLAUDE.md` should route to canonical skills and this casebook without duplicating their bodies. Claude-specific adapters should remain thin and point to the repository skill. | Repository root plus thin Claude adapters. |

The repository skills above are deliberately focused. The visibility radii, Crystal geometry,
specific PR order, individual cell IDs, and current Grand compiler heuristics remain product
contracts or case evidence rather than reusable skills. The durable procedures are oracle
tracing, global feature integration and scale checkpoints, safe Cargo recovery, visual
inspection, and read-only retrospective distillation.

## Active T43 provisional caveat and open claims

T43 must not be summarized as “the Grand map is complete.” At the cutoff:

1. The last attributable exact compiler test failed at natural-pass/ordinary-route
   construction for cell 19. Previous cell-63 and cell-92 failures and the temporary
   8.7-second green compile are useful attempt history, not current success. Further route
   edits and a newer unclassified failed command marker mean the newest dirty state has no
   verified disposition yet.
2. The full-scale proxy is a verified architecture checkpoint, but its perception and local-edit
   budgets missed and its screenshot predated final hydrology, tunnel, vegetation, and Crystal
   integration.
3. The cropped proxy screenshot was explicitly rejected as a complete overview. No later
   bounds-fitted overview, full multi-angle pack, or motion review had passed.
4. The `inspect-game-renders` skill was drafted during T43, but its validation and real use on a
   fresh coherent build must be checked separately.
5. Repeated Cargo pressure remains a workflow risk until builds are coordinated and the
   read-only capacity audit confirms protected work and adequate bytes/inodes.
6. Runtime chunking, exact picking, perception, lifecycle, snapshot, corpus, CI-equivalent, and
   playable-candidate gates were not complete at the cutoff.

When refreshing this casebook, append new evidence rather than rewriting these facts. Update the
cutoff, current dirty/committed state, latest exact command and output, T43 message count if new
steers arrived, and A18's disposition. Promote T43 to `verified-success` only after the exact
compiler, required corpus, approved performance budgets, fresh full-resolution stills, native
motion review, playable launch, and final repository gate all pass on the same identified state.
