# V4 reusable world foundation

## Header

- Status: implementation
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
    state: integrated
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
    state: building
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
    state: building
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
    state: building
    pr: null
```

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

- 2026-09-04: initial implementation authorization; no later scope changes.
- 2026-09-04: shared contracts integrated as `bcb3e2b`; 22 contract tests and
  focused Clippy passed in the source lane. L4 dispatches after L1 frees its worker.

## Close-out

Pending implementation, combined review and ordinary wave PR. No merge is authorized
by a partial checkpoint. Record completed milestones and residual work here as it changes.
