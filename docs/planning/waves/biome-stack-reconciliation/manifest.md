# Biome stack delivery reconciliation

- Status: integrating
- Cutoff: 2026-09-02 America/Los_Angeles
- Original stack base: `fc55bd5a1c3c0181b6506d5ac59e1189d287838a`
- Post-#218 stack-refresh base: `495a73dcbe7edbab6d993867d91b15979fa6ce81`
- Merge target: `dev` only
- Required order: #210 → #211 → #212 → #213

This is a delivery-state record for a user-authorized, pre-existing sequential PR stack.
This manifest does not create an aggregate `wave/*` branch: it records the existing GitHub
topology, where #210 targets `dev` and #211–#213 target their immediate parent branches.
Changing that topology before each parent lands would make the history and review state less
truthful. The source waves retain and update their own manifests on their branches.

## Outcome and exclusions

Deliver Crystal Mountain, Arid/Desert Oasis Rings, Coastal/Ocean Archipelagoes, and the
corrective biome-feedback pass to `dev` in strict dependency order with exact-head automated
and human evidence.

Grand V3, Garden, the Outpost redesign, route reauthoring, time-cycle work,
subtle-geometry experiments, generic review tooling, and teammate PR #196 are excluded from
this stack. Their readiness is recorded below so excluded work cannot be mistaken for
delivered work.

## Locked delivery decisions

1. Land #210, then #211, then #212, then #213; never merge a child before its parent is
   verified on `dev`.
2. Use merge commits, never squash, rebase, or force-push, so exact source heads remain
   reachable from `dev`.
3. Retarget each child to `dev` only after its parent merge is verified, then merge current
   `dev` into the child additively.
4. A draft-green manual-sign-off check is a deferral, not named-human evidence.
5. Consume the identity-based selector delivered by #215. Preserve its exhaustive,
   disjoint ordinary-test preflight and add each new required ignored-test identity in the
   candidate that introduces it; never restore numeric partition counts.
6. Keep parent branches until every child and downstream Grand branch is retargeted and the
   exact ancestry is verified.

## Live stack

| Order and PR | Published head and base | Local evidence | Hosted CI | Exact-head human | Delivered / remaining blocker |
|---|---|---|---|---|---|
| 1. [#210](https://github.com/chillgamerboys/bevy-hex-game/pull/210), `wave/crystal-mountain` | Original `74deb7f84d92e2088c63eafc1d5988c63171896d`; refresh base `495a73dc`; additive merge `2e175917` | `PENDING` on the final refreshed head; the old full gate and 28 captures are historical | `PENDING`; the old head was green except cancelled macOS | `PENDING`; the draft job was a deferral | No; base refresh and selector reconciliation are complete, but all-platform CI, refreshed captures/acceptance, and exact-head findings remain |
| 2. [#211](https://github.com/chillgamerboys/bevy-hex-game/pull/211), `wave/desert-biomes` | `441c22cc6968478993e920a1a575fa086edc05ee`; `wave/crystal-mountain` at `74deb7f` | Source records a local full gate and 22 frames | Historical Map partitions failed under the retired count check; macOS cancelled | Pending; source-lane job is a deferral | No; #210 must land, then retarget/refresh, reconcile, rerun, and record Desert/Oasis findings |
| 3. [#212](https://github.com/chillgamerboys/bevy-hex-game/pull/212), `wave/island-biomes` | `09bc0ebc28be9bb800dadb1f2e6d9a31f01cb3c8`; `wave/desert-biomes` at `441c22c` | Source records a local full gate and 17 captures | Historical published-head matrix green | Pending; source-lane job is a deferral | No; #211 must land, then retarget/refresh, reconcile, rerun, and record Island presentation/motion/control/play findings |
| 4. [#213](https://github.com/chillgamerboys/bevy-hex-game/pull/213), `fix/biome-feedback` | `63aed363e5ba394c4404e9b168967548960e851e`; `wave/island-biomes` at `09bc0ebc` | PR body records focused/full gates and 56 stills | Historical Map partitions failed under the retired count check | Native Crystal/Desert/Island flicker/motion/control findings pending | No; #212 must land, then retarget/refresh, preserve all three manifests, rerun, and record findings |

The refreshed #210 branch contains the recorded post-#218 `dev` base; #211–#213 do not,
and none of the four candidates is delivered. The original unique footprints were measured
against each immediate parent: 25 commits/52 files for #210,
3/61 for #211, 6/62 for #212, and 2/27 for #213. Shared hotspots include
`.config/test-scopes.json`, map and scenario definitions, save fixtures, procedural generators,
camera routes, status/roadmap documents, and the source-wave manifests. Later branches extend
their parents and must not replace parent semantics with obsolete snapshots.

## Enabling delivery ledger

These changes are delivered to `dev`. They make the stack refreshable; they do not deliver
any biome content.

| PR | Exact source head | Merge on `dev` | Delivered outcome |
|---|---|---|---|
| [#214](https://github.com/chillgamerboys/bevy-hex-game/pull/214) | `eb496a1574212d46c6a8f15f0e6904ba30e7dc4e` | `2d6cf1c6e6daac1bb5cec6d881afb3a093ee5cf6` | Established the first explicit stack ledger. Its exact-head checks were green; the subsequent `dev` run exposed the dependency failure repaired by #216. |
| [#216](https://github.com/chillgamerboys/bevy-hex-game/pull/216) | `2c31eb892095379677fdf9ade71a186ba5348c99` | `aaf2f7b7edc80122cb0e802a79d323e80c64dfd4` | Locked `chacha20` to the viable `0.10.2` graph and restored the 45-minute macOS shipping-build budget. Post-merge `dev` run `33694018317` passed. |
| [#215](https://github.com/chillgamerboys/bevy-hex-game/pull/215) | `6bba227c5c19f4edb316535fb3ea6061f93b84ce` | `a5eef67eb016f6b88bc2da425238dbb213b813e4` | Replaced partition counts with declared test identities and required ignored patterns; added selector preflight and fail-closed zero-test/ignored-only behavior. Post-merge `dev` run `33704463781` passed. |
| [#217](https://github.com/chillgamerboys/bevy-hex-game/pull/217) | `93bf6499b476f9c67431d8bf3c8370e24a012f35` | `4c97b75151b1a6f4e1ea1972976e1d9512ed8c45` | Published four isolated case-backed development workflows without their eleven Grand ancestors; exact-head checks and the logic-only manual-runtime `N/A` passed. Post-merge `dev` run `33708389913` was still running at this cutoff. |
| [#218](https://github.com/chillgamerboys/bevy-hex-game/pull/218) | `7808ce99157845cef7e0cc2f26f8ce9e420f5fbc` | `495a73dcbe7edbab6d993867d91b15979fa6ce81` | Reconciled status, roadmap, and wave records against repository and GitHub truth before the stack refresh began. |

## Evidence axes and downstream readiness

The [status](../../status.md), [roadmap](../../roadmap.md), this manifest, and GitHub
must keep four separate facts: what exists locally, what GitHub validated, what a named
human approved, and what is reachable from `origin/dev`.

| Axis | Meaning |
|---|---|
| Local evidence | A command, artifact, or finding recorded against one exact local head; it says nothing about GitHub or `dev` by itself |
| Hosted CI | GitHub's completed check rollup for the exact published PR head; an older head cannot validate a refresh |
| Human approval | Named-human findings tied to the exact candidate head; a successful draft/source-lane deferral job is not approval |
| Delivered | The exact PR head is reachable from `origin/dev` through its verified merge commit |

| Outcome | Local / attributable state | GitHub state | Exact-head human state | `dev` state and classification |
|---|---|---|---|---|
| #210 Crystal Mountain | Original committed head plus additive current-base merge `2e175917` | Draft PR; refreshed-head checks pending | Missing; draft job was a deferral | Absent; base-refreshed, still in progress |
| #211–#213 biome stack | Three fully committed published heads with explicit parent ancestry, not yet refreshed | Three draft PRs; displayed rollups are pre-refresh evidence | Missing; draft/source-lane jobs are deferrals | Absent; blocked behind #210 |
| Grand V3 committed checkpoints | Sixteen clean commits from `a065223` through `3a6e331`, already containing all four original PR heads | No Grand PR | Missing | Absent; mechanically packageable as three drafts only after #213 |
| Garden | Nine local commits above its published branch plus two dirty integration-golden edits | No PR | Earlier walk is stale | Absent; near-ready only after clean post-#213 reconstruction, current-base golden repairs, complete gate, and exact-head walk |
| Generic review tooling | Candidate hunks exist inside the inherited dirty Grand snapshot | No PR | Not applicable until clean extraction | Absent; split into provenance hardening and capture sequences |
| Grand structural review | Candidate hunks exist inside the inherited dirty Grand snapshot | No PR | Missing | Absent; reconstruct on the committed Grand baseline before active ports |
| Route revision 3 | Active task has an attributable evolving patch, but shares the inherited dirty snapshot | No PR | Missing | Absent; in progress after the structural baseline |
| Time cycle | Stable task-owned source freeze exists; shared files require hunk composition | No PR | Missing | Absent; paused behind accepted route revision 3 and split preview/content PRs |
| Subtle geometry | Experimental deltas exist, but its ownership manifest is incomplete and evidence inputs are not yet immutable | No PR | Missing | Absent; non-mergeable until one exact-head review boundary passes |
| Outpost | Rejected checkpoint `f4f0e4cfd1a33c977e68a39d28b2c49b90b5dea8` is preserved remotely as `archive/outpost-rejected-f4f0e4c` with no PR | Archive branch only | Rejected | Absent; replacement redesign has not started |

No clean local commit set may remain without a remote PR, an explicit WIP
branch, or an archive classification. A clean commit is not merge-ready when current-head
CI or required presentation evidence is absent.

## Source-manifest reconciliation

The Crystal, Desert, and Island manifests do not exist on current `dev`; they arrive with
their source branches and must be corrected there, not copied early into this manifest.

- #210 retains its integrating status after merging current `dev`, retains the delivered
  identity selector, declares exactly the following three Crystal required-ignored patterns,
  and resets current-head CI and human evidence to pending:
  `*procedural_v3::macro_world::tests::crystal_mountain_constructs_as_one_valid_world_in_all_six_global_rotations`,
  `*procedural_v3::macro_world::tests::crystal_mountain_release_corpus_validates_32_seeds`,
  and
  `*procedural_v3::macro_world::tests::crystal_mountain_generation_benchmark_p95_stays_within_existing_macro_budget`.
- #211 labels its old local/full-gate claims historical, changes to integrating, records the
  delivered #210 merge and current `dev` base, and restores review-ready only after its new
  exact-head gate.
- #212 applies the same four-axis reset after #211; its old hosted-green rollup is not copied
  as refreshed evidence.
- #213 has no independent manifest. Its conflict resolution preserves the refreshed Crystal,
  Desert, and Island manifests and reapplies only its additive corrective verification; it
  never selects a whole stale manifest side.
- Every refreshed branch keeps current canonical `status.md` and `roadmap.md`. Detailed
  source-branch test results belong in the owning manifest with their exact SHA, never as a
  delivered claim in status.

The already closed client-hosted-Sandbox and host-owned-Campaign manifests are dated
historical delivery records whose source and merge commits remain ancestors of current
`dev`; they require no rewrite for this stack.

## Remaining integration procedure

1. #210 base refresh is complete at additive merge `2e175917` without rewriting its
   published history. Rerun every platform, record exact-head Crystal findings, mark ready,
   audit, and merge with a GitHub merge commit.
2. Verify both #210's original and refreshed heads are ancestors of `origin/dev`.
3. Retarget #211 to `dev`, merge updated `dev` into it, reconcile its manifest, rerun under
   identity selection, obtain Desert/Oasis findings, audit, and merge.
4. Repeat the ancestry verification, retarget, refresh, full gate, and exact-head Island
   review for #212 even though its old head was green.
5. Repeat for #213 and complete the native Crystal/Desert/Island flicker route.
6. Retain every parent source branch until its child is retargeted and verified. After each
   merge, record the original head, refreshed head, merge commit, resulting `origin/dev`,
   hosted run, human reviewer/result, and remaining downstream blocker in this ledger.
7. Only after #213 is verified on `dev`, reconstruct the Grand planner, compiler, and final
   baseline drafts while preserving their original commits and merge ancestry.

## Downstream recovery order

1. Reconstruct `wave/grand-v3-schematic-refresh` at planner endpoint
   `36b67f349cca394fdd198636451eaac7f4b66ff9`, merge post-#213 `dev`; reconstruct
   `wave/grand-v3-proxy-compiler` at `dcf917536aaca7e43b9eee35e75141eb442a9c3f`,
   merge the refreshed planner; reconstruct `wave/grand-v3-final-baseline` at
   `3a6e3317e21371935a3ebf2a0044a0701f10b191`, merge the refreshed compiler. Open all
   three as stacked drafts and retain the final draft while renderer-memory or
   visual/full-selector/human gates remain open.
2. Rebuild Garden on post-#213 `dev` from its attributable commits. Re-derive its save and
   Sandbox expectations on the current catalog instead of importing either dirty local
   integration hunk.
3. Do not commit the inherited dirty Grand snapshot as a common baseline. Extract generic
   stale-output/provenance/disk-preflight reporting to
   `chore/review-provenance-hardening`, and JSON capture parsing/sequencing to
   `chore/review-capture-sequences`. Then compose Grand-only structural presentation work
   on `wave/grand-v3-structural-review`. Both generic PRs exclude time selection,
   fog/material/edge treatments, geometry effects, route changes, and Grand-only structural
   presentation.
4. Port active work in dependency order: route revision 3, time-cycle preview controls,
   time-cycle content rollout, then subtle geometry. Route owns the first baseline-world
   edit; exclude shared-transit residue and the earlier NO-GO patch, and require the strict
   reference, generated-hero, Crystal-mutation, disconnected-negative, publication,
   route-history, and structural gates. Time applies its height edit afterward and refreshes
   combined fingerprints once. The preview controls remain render-only and never mutate
   authoritative time, perception, save, or multiplayer state.
5. Subtle geometry must use a default-off `hex_map/map-review` feature, optional review-only
   dependencies, the generic capture sequence, and immutable checksummed camera inputs.
   Rerun after route landmark movement, revalidate physical-cloud finalists after time
   sky/cloud changes, then reimplement only approved winners on a clean branch.
6. Preserve rejected Outpost commit `f4f0e4cfd1a33c977e68a39d28b2c49b90b5dea8`
   only on `archive/outpost-rejected-f4f0e4c`, with no PR. Start the replacement from
   then-current `dev`; its sketch-authoritative repeated military geometry, consistent
   bands, elevated openings, and enclosed stair/camera solution precede full hardening.

Active route, time, and geometry source trees all inherit the dirty `3a6e331` snapshot.
Their branch labels do not establish ownership. Port only recorded task-owned files and
hunks into the canonical GitHub-backed repository; the route task's local `origin/dev` is
stale at `2795c75`, rather than GitHub `dev`, and must never be used as an integration
base. Do not delete, move, or prune
`crystal-ascent-aesthetic-snapshot`; it is the Git common directory for the active
small-geometry linked worktree. Cleanup is allowed only after that task's owned patch and
immutable evidence are preserved, followed by normal `git worktree remove` and then
`git worktree prune`.

## Acceptance and stop conditions

- Every PR diff is attributable to clean commits or recorded source hunks. Heavy Rust builds
  run serially across these worktrees under an explicit owner lease.
- Every refreshed head passes the complete selector-chosen CI-equivalent gate.
- Typed contracts prove generation, publication, regeneration, re-entry, routes, geometry,
  and fingerprints.
- Multi-capture coverage includes malformed JSON, duplicate paths, sequencing, watchdog
  failure, and successful completion.
- On one exact subtle-geometry head, require the ordinary/default `hex_map` build, the
  shipping-shaped `hex_game --no-default-features` build, the `hex_game --features
  map-review` build, and dependency inspection proving direct `serde_json` and `sha2` are
  absent without the feature. `hex_game/map-review` must activate `hex_map/map-review`.
- Fresh static review covers each affected biome and a named human records exact-head native
  camera motion, flicker, input/control feel, and play findings.
- Status, roadmap, and source manifests describe only residual work after each merge.
- Stop if a child does not contain its declared parent, GitHub cannot use a merge commit, an
  operation would require force-pushing, a transplant would resurrect obsolete ancestry, or
  a conflict changes world/gameplay authority instead of mechanically composing the stack.

Linear was unavailable at this cutoff. No ticket state is inferred from repository or
GitHub state, no ticket change is required to merge an otherwise valid PR, and a future
connected reconciliation should record only genuine residual product scope rather than one
ticket per lane.

## Close-out

After #213 lands, record all four merge commits and the resulting `origin/dev` SHA here,
retarget downstream consumers, reconcile the source manifests/status/roadmap, and only then
classify the stack as delivered.
