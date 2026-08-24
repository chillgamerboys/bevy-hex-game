# Grand V3 schematic planner wave

- Status: `review-ready`
- Wave branch: `wave/grand-v3-schematic`
- Base `origin/dev`: `fc55bd5a1c3c0181b6506d5ac59e1189d287838a`
- Required stacked dependency: biome feedback head
  `63aed363e5ba394c4404e9b168967548960e851e` from draft PRs #210–#213
- Coordinator: Codex / world integration
- Epic: Grand V3 schematic planner; no Linear ticket is assigned
- Shippable outcome: a pure deterministic radius-eight schematic library and CLI which
  emits strict RON plans, metrics, SVG diagnostics, and atomic multi-seed galleries.
- Exclusions: voxel terrain, a new runtime V3 layout, recipe compilation, final materials
  or palette, save migration, and automatic raster-sketch interpretation.

## Why this wave exists

The schema, solver, diagnostics, CLI, and corpus are useful only as one contract. The
future runtime map generator must consume the same deterministic plan that the offline
gallery validates, while the current V3 Macro and every shipped fingerprint remain
unchanged. Four disjoint world-authority lanes therefore assemble one review candidate.

## Locked decisions

1. **S1:** "A complete radius-eight schematic contains exactly 217 canonical cells;
   this phase plans cells and never chooses voxel radius, height, materials, or colors."
2. **S2:** "The peak ring, elevated mountain lake, lake island, frozen woods,
   waterfall, Crystal Ascent, and complete tunnel route use exact traced cells in every
   seed."
3. **S3:** "The coastline may move at most two cells; sea islands comprise two through
   six scenic groups of one through four cells; eligible woodland occupies thirty
   through eighty percent; and the valley lake contains three through seven cells."
4. **S4:** "One world seed deterministically generates one selected plan through named
   independent streams and 32 hard-valid candidate attempts, with a separately validated
   reference fallback and no invalid output."
5. **S5:** "The authoritative output is strict versioned RON. SVG and HTML are diagnostic
   projections, use a review-only palette, and never establish logical correctness."
6. **S6:** "This new pure crate is world-authority infrastructure intended for later V3
   runtime use; this wave does not modify `V3LayoutSettings` or existing fingerprints."

## Shared foundation

- `hex_schematic::SchematicCoord` supplies a checked, strict cube coordinate without
  importing gameplay or Bevy into the planning boundary.
- Existing V3 named-stream, validation, fingerprint, and fail-closed evidence practices
  are precedents only; the new library has no dependency on `hex_map` or Bevy rendering.
- The coordinator owns workspace registration, aggregate integration, selector planning,
  gallery review, and the final combined gate.

## Dispatch queue

```yaml
lanes:
  - id: L1
    title: Strict schematic contracts and traced Grand V3 template
    order: orders/L1-contracts-template.md
    ticket: null
    authority: world
    builder: worker
    branch: lane/schematic-contracts
    owns:
      - crates/hex_schematic/Cargo.toml
      - crates/hex_schematic/src/lib.rs
      - crates/hex_schematic/src/model.rs
      - crates/hex_schematic/src/template.rs
      - assets/config/schematics/grand-v3-template.ron
      - docs/planning/waves/grand-v3-schematic/manifest.md (L1 queue row only)
    dispatch_blockers: []
    merge_blockers: []
    fences: []
    selector: {concerns: [residual, clippy, docs], full: true}
    evidence: logic-only
    sizing: {model: inherited, effort: high}
    state: integrated
    pr: null

  - id: L2
    title: Deterministic generator, validator, metrics, and fingerprints
    order: orders/L2-generator-validator.md
    ticket: null
    authority: world
    builder: worker
    branch: lane/schematic-generator
    owns:
      - crates/hex_schematic/src/generator.rs
      - crates/hex_schematic/src/validate.rs
      - crates/hex_schematic/src/fingerprint.rs
      - crates/hex_schematic/src/metrics.rs
      - docs/planning/waves/grand-v3-schematic/manifest.md (L2 queue row only)
    dispatch_blockers: []
    merge_blockers: [L1]
    fences: []
    selector: {concerns: [residual, clippy, docs], full: true}
    evidence: logic-only
    sizing: {model: inherited, effort: high}
    state: integrated
    pr: null

  - id: L3
    title: CLI, SVG diagnostics, and atomic gallery publication
    order: orders/L3-cli-rendering.md
    ticket: null
    authority: world
    builder: worker
    branch: lane/schematic-cli
    owns:
      - crates/hex_schematic/src/main.rs
      - crates/hex_schematic/src/cli.rs
      - crates/hex_schematic/src/render.rs
      - docs/planning/waves/grand-v3-schematic/manifest.md (L3 queue row only)
    dispatch_blockers: []
    merge_blockers: [L1, L2]
    fences: []
    selector: {concerns: [residual, clippy, docs], full: true}
    evidence: static-presentation
    sizing: {model: inherited, effort: high}
    state: integrated
    pr: null

  - id: L4
    title: Corpus, performance, documentation, and approval pack
    order: orders/L4-acceptance.md
    ticket: null
    authority: world
    builder: worker
    branch: lane/schematic-acceptance
    owns:
      - crates/hex_schematic/tests/**
      - docs/systems/world-generation-v3.md (schematic planner section only)
      - docs/development/config.md (schematic planner section only)
      - docs/planning/status.md (schematic planner section only)
      - docs/planning/roadmap.md (schematic planner row only)
      - docs/planning/waves/grand-v3-schematic/manifest.md (L4 queue row and acceptance ledger)
    dispatch_blockers: []
    merge_blockers: [L1, L2, L3]
    fences: []
    selector: {concerns: [residual, clippy, docs], full: true}
    evidence: static-presentation
    sizing: {model: inherited, effort: high}
    state: integrated
    pr: null
```

## Ownership map and integration order

- L1 owns wire shape and the hand-traced reference, not generation policy.
- L2 consumes L1 and owns every logical acceptance decision and metric.
- L3 projects validated outputs and may not infer or weaken L2 validity.
- L4 owns black-box acceptance and documentation; it does not duplicate private solver
  predicates.
- Every lane owns only its queue row. The coordinator resolves the manifest, root
  `Cargo.toml`, `Cargo.lock`, module exports, and composed documentation at integration.
- L1 lands first; L2 and L3 may build concurrently against its declared interfaces; L4
  closes after the composed CLI is runnable.

## Combined acceptance

- Strict template/plan round trips, canonical geometry, fixed claims, networks,
  determinism, stream independence, fallback behavior, and renderer projections pass.
- The CI corpus covers 256 seeds; the ignored release corpus covers 10,000 seeds and
  records validity, fallback, uniqueness, and diversity distributions.
- Release generation p95 remains below 50 ms per plan and peak process memory below
  64 MiB on the acceptance runner.
- A labelled reference trace, one reference plan, and a twelve-seed gallery are inspected
  at full resolution and as a complete contact sheet. Pixels judge only the diagnostic
  projection; typed plans and validators own all logical claims.
- The final selector-chosen CI-equivalent gate runs once on the exact combined head.

## Acceptance ledger

| Evidence | Result |
|---|---|
| Strict package tests | 30 library, 16 binary, and 12 black-box acceptance tests pass; the release-only corpus is intentionally ignored by the ordinary package run |
| Normal corpus | 256/256 valid, non-fallback, semantically unique plans; every required bounded region varies |
| Release corpus | 10,000/10,000 valid, zero fallback, 10,000 unique fingerprints, every island-count and island-size bucket represented, adjacent-seed diversity threshold met, generation p95 below 50 ms |
| Memory | Linux `VmHWM < 64 MiB` remains enforced by the scheduled stress workflow; macOS local runs cannot publish that metric |
| Static presentation | labelled grid, canonical reference, twelve seed variants, authorship diagnostics, and hydrology overlays generated and inspected at original resolution and as a self-contained contact sheet |
| Dependency boundary | normal tree contains only `atomicwrites`, `ron`, `rustix`, `serde`, and `xxhash-rust`; no Bevy, gameplay, `hex_core`, or `hex_map` dependency |
| Repository integration | selector-chosen CI-equivalent gate pending on the documentation-complete combined head |

## Stop conditions

- Any dependency on `hex_map`, a renderer, gameplay, or runtime map state.
- Any change to existing V3 layout/settings/fingerprint bytes.
- A generator output which bypasses validation or silently emits fallback as a normal
  candidate.
- An unannotated lane overlap or a new product decision not covered by S1–S6.
- Landing to `dev` before draft PRs #210–#213 and this wave's own approval are complete.

## Injection log

- The final contract audit separated deliberate canonical reference artifacts from genuine
  exhausted-candidate fallbacks, hardened fixed-overlay ownership and scalar provenance,
  and moved hydrology before island/vegetation resolution so later named streams cannot
  affect candidate eligibility or selection.
- Static review found externally embedded gallery panels and overly dense cell labels.
  The contact sheet now renders all twelve maps directly, composites carry one semantic
  abbreviation per cell, and diagnostics retain coordinates and authorship detail.
- Directory publication now uses an atomic no-replace rename and has a deterministic
  late-created-empty-destination race regression.

## Close-out

Implementation, package verification, both deterministic corpora, and static gallery review
are complete. Pending the selector-chosen repository gate, named-human approval,
publication, and delivery to `dev` after the prerequisite corrective stack lands.
