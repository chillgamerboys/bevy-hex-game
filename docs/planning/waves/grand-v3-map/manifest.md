# Grand V3 map delivery manifest

- Status: `integration-and-validation-in-progress`
- Delivery PR: [#219](https://github.com/chillgamerboys/bevy-hex-game/pull/219)
- Integration candidate: `fix/hex-six-map-fixes`
- Branch: `wave/grand-v3-schematic`
- Fixed scenario: `Grand V3 Baseline`
- Fixed world seed: `1_592_598_566`
- Current template: Grand V3 revision 3
- Current fine scale: pitch 22, radius 187, level height 0.35
- Footprint: 105,469 columns in 444 resident 16 by 16 axial chunks

This manifest retains the original planner, proxy and final-content checkpoint evidence.
Checked historical items do not certify the current combined PR head or delivery to `dev`.
The current gate below owns the recovered fixes, accepted visual promotion and grounded
exploration. Earlier measurements and approvals remain scoped to their recorded sources.

## Current combined delivery gate

- [ ] Record the exact geometry candidate, walking handoff and combined source identity.
- [ ] Resolve all seven entries in [the recovered-fixes ledger](../../hex-six-map-fixes.md)
  with final geometry evidence or the approved concrete deferral reason.
- [ ] Preserve the independently captured original seed-zero and hero natural passes.
- [ ] Pass strict current-source generation, retained corpora and the selector-chosen
  combined gate, without structural draft or weakened geometry/majority contracts.
- [ ] Inspect fresh changed-region and full-map captures and separate water animation.
- [ ] Complete combined walking/controller/camera/lifecycle checks and record the exact
  scope of native motion and play evidence.
- [ ] Verify both same-map Cargo launchers and record final PR checks.
- [ ] Verify the exact merge on `dev` and preserve the dependent V4 branch ancestry.

## Approved foundation

- [x] Revision-2 schematic matches the source drawing cell for cell.
- [x] Runtime generation selects one schematic plan and does not repeat the legacy
  eight-candidate voxel search.
- [x] The exact radius, pitch, vertical profile, and resident-chunk scale are approved.
- [x] Proxy setup, memory, snapshot, path, camera, perception, and edit measurements are
  recorded in [proxy-checkpoint.md](proxy-checkpoint.md).
- [x] Combined solid chunk meshes with exact run picking are the approved presentation
  direction.
- [x] Perception p95 at 50 ms and one-column edit p95 at 100 ms remain hard optimization
  targets.

## Compiler and content

- [x] Prove exact 217-cell ownership, 105,469-column coverage, and 444 resident chunks
  after final content is applied.
- [x] Prove the global-height-field seam bound across 8,437 ordinary, unlocked,
  non-route cross-owner neighbor pairs, with a scale-relative coverage floor.
- [ ] Complete the multi-angle pixel review for visible coarse-cell seams; the typed height
  bound is necessary but cannot establish presentation by itself.
- [x] Prove the mountain-lake → waterfall → valley-lake → river → sea graph is acyclic,
  descending, three lanes wide, recessed below adjacent banks, and has exactly one outlet.
- [x] Prove exactly two two-wide worked-stone river bridges and no unintended ordinary
  water crossing.
- [x] Prove one connected ordinary graph with a representative hub in every ordinary
  schematic cell.
- [x] Admit 256 schematic seeds with exactly two declared lower-to-upper routes and all
  three seeded natural-pass width buckets (`3`, `4`, and `5`).
- [x] Prove the final natural pass retains its exact seeded physical width, admits the
  separator-safe single-detour/pass-tail fallback, and leaves no third materialized
  lower-to-upper route across reference, zero, hero, and maximum seeds.
- [x] Prove exact Crystal Ascent heart occupancy, tunnel handoff, unified interior, Bright
  fixture coverage, authored-object LOS/traversal blocking, fog/cutaway composition, and
  teardown/re-entry. The compile-time geometry and support-skirt checks remain separate
  map-owned validators.
- [x] Execute the exact vegetation-density and full-clearance contract for all five
  density bands and exclusions covering water, routes, bridges, architecture, anchors,
  lights, blockers, Crystal terrain, and sight-critical clearings.
- [x] Re-run the hero export contract proving stable review anchors for every required
  coast, hydrology, alpine, tunnel, and Crystal review site.
- [x] Re-run the complete Crystal compile-time geometry, support-skirt, and tunnel-light
  coverage validators after the final pass repair.

## Runtime and budgets

- [x] Remeasure setup, compile/materialization, resident/peak memory, render entities and
  batches, snapshot export/encoding, movement, A*, perception, camera queries, local edits,
  teardown, and re-entry against Crystal Mountain. Results are recorded in
  [final-runtime-checkpoint.md](final-runtime-checkpoint.md).
- [ ] Meet every approved proxy budget without changing world scale or fidelity. All
  deterministic headless latency and snapshot budgets pass, including perception p95
  ≤ 50 ms and local-edit p95 ≤ 100 ms; full-renderer RSS and peak-footprint misses are
  isolated to transient terrain publication/extraction and require optimization plus
  remeasurement.
- [x] Prove chunk-local edits replace only affected render roots and preserve exact picking,
  occupancy, LOS, fog, liquids, saves, and semantic identity.
- [x] Preserve zero idle recomputation for 10,000 unchanged Grand V3 runtime updates,
  including stable terrain/fog batch identities and mesh counts.
- [x] Pass the 256-seed schematic/mask/route/hydrology topology-admission corpus.
- [x] Pass the reference, zero, hero, and maximum-seed complete-world gates.
- [x] Pass the ignored 32-seed full-world release corpus. The final run completed all
  32 fully materialized worlds in 64.65 s; seed 14 is retained as a non-ignored regression
  for review anchors that must not promote a disconnected incidental cap into walker intent.
- [ ] Pass the selector-chosen CI-equivalent gate.

## Historical typed evidence

The following commands passed at their original checkpoints. They require fresh receipts
on the combined source before the current delivery gate is complete. The accepted-source
diagnostic reproduced strict-admission failures; these historical results do not certify
the recovered candidate:

```text
CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_DEBUG=0 cargo test --release -p hex_map --test schematic_compile public_schematic_fine_topology_admits_256_seeds -- --exact --nocapture --test-threads=1
CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_DEBUG=0 cargo test --release -p hex_map --lib procedural_v3::schematic::tests::final_content_publishes_exact_bridges_lights_hubs_and_grounded_vegetation -- --exact --nocapture --test-threads=1
CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_DEBUG=0 cargo test --release -p hex_map --test schematic_compile complete_world -- --nocapture --test-threads=1
CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_DEBUG=0 cargo test --release -p hex_map --test schematic_compile public_generated_hero_schematic_compiles_publishes_and_exports -- --exact --nocapture --test-threads=1
CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_DEBUG=0 cargo test --release -p hex_map --lib procedural_v3::schematic::tests::final_schematic_vegetation_meets_density_and_full_clearance_contract -- --exact --nocapture --test-threads=1
CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_DEBUG=0 cargo test --release -p hex_map --lib procedural_v3::schematic::tests::ordinary_unlocked_patch_boundaries_have_no_coarse_height_step -- --exact --nocapture --test-threads=1
CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_DEBUG=0 cargo test --release -p hex_game --features test-support --lib scenarios::tests::grand_v3_edit_contract::one_dry_grand_v3_edit_is_chunk_local_and_preserves_composed_runtime_contracts -- --exact --nocapture --test-threads=1
CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_DEBUG=0 cargo test --release -p hex_game --features test-support --lib scenarios::tests::grand_v3_crystal_contract::grand_v3_crystal_contract_rebuilds_identically_across_the_complete_lifecycle -- --exact --nocapture --test-threads=1
CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_DEBUG=0 cargo test --release -p hex_game --features test-support --lib scenarios::tests::grand_v3_idle_contract::grand_v3_ten_thousand_idle_frames_reuse_every_runtime_projection -- --ignored --exact --nocapture --test-threads=1
CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_DEBUG=0 cargo test --release -p hex_game --features test-support --lib scenarios::tests::grand_v3_headless_runtime_gate_compares_to_crystal_mountain -- --ignored --exact --nocapture --test-threads=1
CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_DEBUG=0 cargo test --release -p hex_map --test schematic_compile grand_v3_full_world_release_corpus_compiles_32_seeds -- --ignored --exact --nocapture --test-threads=1
```

## Publication and review

- [x] Scenario, world, encounter, and Sandbox catalog identities reserve the fixed baseline.
- [x] Record the fail-closed corrective pixel matrix and motion route in
  [corrective-visual-review.md](corrective-visual-review.md), including the exact new review
  anchors required for Crystal enclosure, river meander, Garden island, snowline/treeline,
  peak chains, and the centered massif crest.
- [ ] Generate the real `ui/sandbox/grand-v3-baseline.png` from the completed world; never
  publish an undecorated proxy image in that slot.
- [x] Add the seed-exact Grand V3 route to `walks/camera_routes.ron` and a complete
  Map/Third/First Person traversal script.
- [ ] Pair selected schematic SVGs with compiled top-down images.
- [ ] Capture coast, river, both bridges, valley lake, waterfall base/crown, mountain lake,
  frozen woods, natural pass, massif, peak saddle, tunnel mouth, Crystal chamber/ascent/
  summit, illumination, and cutaway evidence.
- [ ] Complete native camera-motion and gameplay review.
- [ ] Receive named human visual/play approval before publishing the final delivery.
