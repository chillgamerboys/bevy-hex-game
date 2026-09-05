# V4 reusable world foundation

## Header

- Status: integrating
- Wave branch: `wave/v4-foundation`
- Verified `origin/dev`: `495a73dcbe7edbab6d993867d91b15979fa6ce81`
- Selected V3 reference: `bc06a8969532b807ec677928eee304bc28399386` (PR #219)
- Coordinator: Codex, current V4 task
- Epic: none; Linear currently requires reauthentication.
- Outcome: data-authored regions compiled into independently resident, editable,
  persistent world chunks, with seamless full-scale composition and gameplay-neutral
  queries consumed by an actual Bevy runtime.
- Exclusions: infinite geography generation, V3 save compatibility, a second complete
  gameplay mode, and detailed combat joining/merging implementation.

## Why this wave exists

The user authorized the complete V4 foundation plan. Compiler, residency, tooling and
runtime presentation share package/query contracts and one combined acceptance path.
They cannot be reviewed as unrelated features. The selected Grand candidate is a
fast-forward descendant of the verified dev revision; it remains an open, imperfect
reference rather than an assertion of green CI. No source from the active dirty visual
experiment is included implicitly.

## Locked decisions

1. Keep exact stacked terrain and existing geometric predicates. Rendering is not authority.
2. Ordinary new maps are runtime-loaded data; compiler changes are needed only for new capabilities.
3. Authoring sources, immutable compiled bases, and runtime modifications are separate records.
4. Regions, storage chunks, render batches, simulation interests, and encounters are distinct.
5. Keep 16 by 16 axial storage chunks initially. A global chunk may contain contributions from multiple regions.
6. Unloaded and outside-world queries never report available air. Publication is revision-checked and atomic.
7. World edits and residency never wait for an unrelated party's turn or pending combat decision.
8. Use fresh V4 formats; keep V3 on its reference branch and through independent fixtures.
9. One shared boundary plan controls both neighbors. Required access and scenic policy remain distinct.
10. The renderer and world map use separate products. Gameplay disclosure is independent of visual residency.
11. The map platform supports turn-based consumers and future free movement; detailed combat is a parallel workstream.
12. All automated rendering stays windowless; no visible game is launched without the user's explicit request.

## Shared foundation

`hex_world_contracts` is a new renderer-free shared data and query boundary. It does
not depend on generator implementations, Bevy, existing save/protocol code, or combat.
`hex_schematic::v4` owns authored source and compilation; `hex_world_runtime` owns
resident world authority and filesystem persistence. Gameplay may consume only public
contracts; the integration binary wires providers. `hex_map` remains the Bevy world
publication owner.

The user explicitly authorized these cross-owner contracts. Commit them separately on
the wave before integrating dependent behavior. Remote dev landing remains the normal
reviewed wave operation; no protected branch is changed during implementation.

## Dispatch queue

```yaml
lanes:
  - id: L1
    title: Shared world contracts
    order: orders/L1-contracts.md
    ticket: null
    authority: shared
    builder: worker
    branch: feat/v4-contracts
    owns: [crates/hex_world_contracts]
    dispatch_blockers: []
    merge_blockers: []
    fences: []
    selector: {concerns: [residual, clippy, docs, shipping], full: true}
    evidence: logic-only
    sizing: {model: inherited, effort: inherited}
    state: merged-to-wave
    pr: null
  - id: L2
    title: Runtime-loaded region authoring and compiler
    order: orders/L2-authoring.md
    ticket: null
    authority: world
    builder: worker
    branch: feat/v4-authoring
    owns: [crates/hex_schematic/src/v4, assets/config/v4]
    dispatch_blockers: [L1 contract API available]
    merge_blockers: [L1]
    fences: []
    selector: {concerns: [map_generation, residual, clippy, docs], full: false}
    evidence: logic-only
    sizing: {model: inherited, effort: inherited}
    state: merged-to-wave
    pr: null
  - id: L3
    title: Residency, queries, edits and persistence
    order: orders/L3-runtime.md
    ticket: null
    authority: world
    builder: worker
    branch: feat/v4-world-runtime
    owns: [crates/hex_world_runtime]
    dispatch_blockers: [L1 contract API available]
    merge_blockers: [L1]
    fences: []
    selector: {concerns: [residual, clippy, docs, shipping], full: true}
    evidence: logic-only
    sizing: {model: inherited, effort: inherited}
    state: merged-to-wave
    pr: null
  - id: L4
    title: Resident terrain presentation adapter
    order: orders/L4-presentation.md
    ticket: null
    authority: world
    builder: worker
    branch: feat/v4-presentation
    owns: [crates/hex_map/src/v4, crates/hex_map/src/grid.rs, crates/hex_map/src/lib.rs, crates/hex_map/Cargo.toml]
    dispatch_blockers: []
    merge_blockers: [L1]
    fences: []
    selector: {concerns: [map_contracts, clippy, docs], full: false}
    evidence: static-presentation
    sizing: {model: inherited, effort: inherited}
    state: merged-to-wave
    pr: null
  - id: L5
    title: Resident stock object presentation
    order: orders/L5-objects.md
    ticket: null
    authority: shared
    builder: worker
    branch: feat/v4-object-presentation
    owns: [crates/hex_objects/src/v4]
    dispatch_blockers: []
    merge_blockers: [L1, L3]
    fences: []
    selector: {concerns: [residual, clippy, docs], full: true}
    evidence: static-presentation
    sizing: {model: inherited, effort: inherited}
    state: merged-to-wave
    pr: null
  - id: L6
    title: Regional sight and illumination
    order: orders/L6-perception.md
    ticket: null
    authority: world
    builder: worker
    branch: feat/v4-perception
    owns: [crates/hex_perception/src/v4.rs, crates/hex_perception/src/v4]
    dispatch_blockers: []
    merge_blockers: [L1, L3]
    fences: []
    selector: {concerns: [residual, clippy, docs], full: true}
    evidence: logic-only
    sizing: {model: inherited, effort: inherited}
    state: merged-to-wave
    pr: null
  - id: L7
    title: Persistent object transactions
    order: orders/L7-object-edits.md
    ticket: null
    authority: world
    builder: worker
    branch: feat/v4-object-edits
    owns: [crates/hex_world_runtime/src/object_edits.rs, crates/hex_world_runtime/tests/runtime/object_edits.rs]
    dispatch_blockers: []
    merge_blockers: [L1, L3]
    fences: []
    selector: {concerns: [residual, clippy, docs], full: true}
    evidence: logic-only
    sizing: {model: inherited, effort: inherited}
    state: merged-to-wave
    pr: null
```

## Additional integration work

- L1 followup: a retained catalogue index and one shared exact lateral-aperture
  predicate; summary pitch is an explicit schema constant.
- L2 primary compiler integrated; required-route aperture regression repaired.
- L3 followup: bounded historical transaction bodies, immutable authoring revisions
  behind an atomic current pointer, principal-specific knowledge partitions and
  disclosure-only reconnect batches.
- L4 primary map presentation integrated; combined windowless verification pending.
- L5: **shared presentation authority only**, `feat/v4-object-presentation`, owns new
  `crates/hex_objects/src/v4/`, its module export and dependency wiring. Reuse the
  existing asset baker and styles through typed resident ownership. Dependencies:
  shared object semantics available for dispatch, L1 and atomic root integration
  required before promotion. Unknown foreign-root art remains an explicit proxy.
  Builder: history_review. State: merged-to-wave. Static and lifecycle evidence required.
- Coordinator: runtime source CLI, atomic receipts, a separate V4 explorer, atlas,
  selected resident-art integration, fresh gameplay checkpoints and combined gates.
- Script harness: generation_architecture owns only new
  `crates/hex_game/src/v4/walk.rs` on `feat/v4-walk`. This is shared integration,
  separate from L2 world authority. It drives existing commands and inspects typed
  state, never teleports actors to manufacture a crossing. Root owns wiring/fixtures.

## Follow-up integration lanes

- L6 — world perception: `hex_perception::v4` and the shared cached sight helper.
  Builder runtime_scaling; source commit integrated as `c6c7df4`. Exact light
  influence projection was integrated separately in shared contracts. Local facts,
  explicit unavailable dependencies, and visible remembered-support removal reuse
  the existing sight kernel. Whole owner suite: 79 pass, 3 manual benchmarks ignored;
  combined game integration remains a separate gate.
- Knowledge consumer — shared game integration: generation_architecture owns only
  new `hex_game/src/v4/knowledge.rs`; root owns scheduling, atlas and capture wiring.
  Principal memory is independent of renderer visibility, with bounded nearby fine
  data and background partition persistence. No encounter scheduler is included.
- L7 — runtime object edits: history_review owns separate shared-contract and world
  runtime commits in `feat/v4-object-edits`. Identity-tagged clipped influence records
  preserve overlapping occupancy; complete before/after objects and exact old/new
  dependency revisions stage atomically. Runtime additions use a reserved
  transaction-derived namespace, with no mutable global object registry. Root owns
  compiler capability bump, game controls, presentation transitions and captures.
- Loopback acceptance tool — runtime_scaling owns only new
  `hex_world_tool/src/replication_benchmark.rs`. Separate sender/receiver processes
  exercise durable partition delivery and restart/replay over bounded loopback
  frames. This validates the protocol seam; it does not install production online
  authentication or a V4 multiplayer lobby. Root owns CLI dispatch.
- Review driver — integrated `26ff9e1`; generation_architecture authored
  `tools/v4_review.py`. Exact checkout/package/walk provenance, bounded timeout,
  independent PNG checks and eleven driver regressions. Mechanical capture completion
  cannot grant visual or native-motion approval.

## Ownership map

L1 owns only its new crate. L2 owns V4 authoring modules and V4 fixture content. L3
owns only its new runtime crate. The coordinator owns workspace wiring, shared
documentation, supported tools, combined tests and application integration. L4 owns
the isolated map presentation adapter and necessary internal grid extraction. Shared existing
files are edited by the coordinator only. Workers use separate source worktrees;
their source commits are inspected and integrated additively.

## Territory

- PR #219: Grand compiler/render/scenario changes; selected exact head is the reference.
- PRs #210–213: preceding Crystal/desert/island/biome stack; their intended combined
  content is already in the selected reference and is not independently re-merged.
- PR #196: lattice fusion; unrelated gameplay territory, not modified by this wave.
- Active subtle-visual task: dirty review/world-detail modules, based on bc06a89;
  intentionally remains separate pending selection of treatments.
- GitHub fetch encountered an unresolved-delta pack error. Verified dev through the
  GitHub API, then obtained that exact existing ref from the local source repository.
  No source checkout or remote branch was changed.

## Integration order

Commit shared contracts first, then compile and runtime lanes can work independently.
Integrate compiler and runtime, followed by tools and real Bevy publication. Run one
serialized expensive build/capture lane; pure library checks can run independently.

## Combined acceptance

- Runtime source edit without relinking the compiler; precise conflict diagnostics.
- Full radius-187 region with distinct geography, stacked cave/bridge geometry,
  directed and standing water, required routes and optional/repeated features.
- Two-region seam geometry and water continuity; seven-region full-cardinality corpus.
- Stable identities under placement, order and worker permutations.
- Incremental packages equivalent to clean compilation.
- Exact availability queries, stale-job rejection, pins, bounded activation,
  stream-out/in edits, atomic save/reload and local idempotent deltas.
- Turn-based and continuous-motion queries use the same occupancy authority.
- Actual windowless Bevy terrain publication, exact picking, local-origin geometry,
  teardown/re-entry, and map-summary display.
- Measurements identify source SHA, input fingerprints, executed work counts, cold/warm
  compilation, streaming/mesh work, memory and remaining red acceptance gates.
- Static render inspection is separate from logical verification. Native motion and
  human aesthetic approval remain honestly unproved until performed.

## Stop conditions

Do not reduce footprint, remove required routes, turn an unavailable query into air,
weaken a validator, or call a headless package a playable runtime merely to pass a gate.
Repair exact failures and preserve their evidence. Do not edit other active checkouts.

## Injection log

- 2026-09-04: initial full implementation authorization. Subsequent entries below
  refine ownership and acceptance within that scope.
- 2026-09-04: shared contracts integrated as `bcb3e2b`; 22 contract tests and
  focused Clippy passed in the source lane. L4 dispatches after L1 frees its worker.

## Close-out

All implementation lanes are integrated; the combined runtime/capture gate is in
progress. The prebuilt authoring workflow, independent regions, local world authority,
actor-local motion, object edits, private exploration and partition persistence have
concrete consumers. Final measurements and review remain separate from implementation.
No protected branch has been changed. Detailed combat/online product integration,
human authoring time and native motion approval remain outside this mechanical gate.


## Reference checks and current integration checkpoint

The selected V3 PR #219 remained open at `bc06a89` when rechecked. Historical workflow
33837409054 contains these preexisting failures: UI catalog expected 26/actual 27;
Grand structural preview peak 58 has no exact patch-owned terrain summit;
`hex_core::presentation` float equality assertions fail strict Clippy; and a redundant
`SpecialMovementRegions` rustdoc link. Ubuntu/macOS shipping and domain coverage passed;
the Windows prime was cancelled. These are not V4 test results.

The first V4 windowless seam run completed 505 frames and three exact verified moves,
including a mid-step reversal and independent second party. It was a dirty map-test
capture of compiler/2, with 45 resident chunks, 21 rendered chunks and a two-job queue.
It is a diagnostic checkpoint, not final approval. The run caught and repaired the
headless window-exit default. Subsequent integration adds real uncapped frame timing,
settled samples, game-only RSS sampling, independent party IDs, private observation,
object transactions and visible landmark absence.

Pure follow-up checks passed 53 runtime tests and 14 compiler V4 tests. The final seven
fixture uses materially different recipe data; clean reordered builds and a clean
seam edit match cached output. The compiler is serial. Source-policy replacement now
requires fresh consumers when materials or shared boundaries change. Fine residency
is bounded; catalogue metadata and persistence heads still scale with history/world
metadata and require paging/compaction before V5-scale indefinite growth.


The compiler/3 measurement series records full wrapper publication times of 3.961,
6.734 and 23.524 seconds for one, two and seven regions. Twenty changed-source edits
all matched clean output (warm incremental p50 0.768 seconds; p95 0.782 seconds).
Fixed one-group and two-group terrain footprints peaked at 26 and 52 resident chunks
respectively in every catalogue. Catalogue-open cost grows with metadata. These are
terrain/compiler measurements with uncontrolled OS cache, not renderer FPS or active
human authoring hours.

The first water capture exposed redundant translucent walls at storage boundaries.
The repair uses immutable one-hex halos from actually published neighbors and serial
asynchronous presentation transactions. A target and at most six adjacent published
roots commit together after preflight, including safe restoration before unload.
Neighbor source/mask identity is nonrecursive, so remeshing does not invalidate the
whole view. This changes the maximum atomic mesh upload from two chunks to seven;
final renderer timings must include that cost. Real queue lifecycle regressions
exercise admission, retirement, stale intent and origin changes.
