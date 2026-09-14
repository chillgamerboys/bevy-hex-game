# Grand V3 map proxy checkpoint

- Status: `approved-historical-baseline; final implementation in progress`
- Branch: `wave/grand-v3-schematic`
- Implementation head: `03c3a0c0443bede6d0f136e6d09a9a4182e8fb40`
- Schematic dependency head: `36b67f349cca394fdd198636451eaac7f4b66ff9`
- Measurement date: 2026-08-25
- Host: Apple M1 Pro, 10 logical CPUs, 16 GiB RAM, macOS 26.5.2
- Toolchain: rustc 1.97.1 (`8bab26f4f`, LLVM 22.1.6)

This is the required representative proxy checkpoint, not the completed Grand V3 map.
It measures the exact radius-187 scale before expensive authored features and decoration.
The proxy includes intended coarse strata and height bands, sea extent, all resident
chunks, liquid surfaces, exact terrain-run presentation, party movement, perception,
camera obstruction geometry, localized terrain edits, snapshots, teardown, and re-entry.
It intentionally does not claim final route or hydrology validity:
`ordinary_route_validated` remains `false`.

Final hydrology, physical routes, bridges, Crystal Ascent, tunnel/interior content,
vegetation, objects, and lights remain paused until Alberto approves quantitative budgets
and the solid-terrain presentation direction. The map has not been silently reduced: pitch
22, radius 187, the dramatic vertical profile, and the complete 105,469-column footprint
are intact.

## Contract evidence

The compiler calls the approved Grand V3 revision-2 schematic generator once, then maps
all 217 coarse cells into exactly 105,469 fine columns. Every column has one stable biome
owner. Euclidean 16 by 16 axial partitioning produces exactly 444 nonempty resident chunks,
each with at most 256 columns. Chunk coordinates do not enter fingerprints, stable IDs,
anchors, snapshots, or network data.

Terrain storage is chunk-native while canonical whole-world `VoxelMap` iteration and the
public run tuple remain unchanged. Runtime setup uses a private pending-publication marker;
it publishes `TerrainReady` only after the `HexGrid`, all 444 `TerrainChunkRoot` children,
liquid and feature projections, global gameplay resources, and the current snapshot have
been built successfully. Malformed, missing, duplicate, orphaned, wrongly parented, or
unexpected chunk roots fail closed before an edit mutates authority.

Terrain edits retain the stable `HexGrid`, replace only affected chunk roots, revalidate
only affected coordinate columns, publish one world revision, and preserve unrelated
special/interior metadata. Teardown removes the grid, chunk roots, exact run presentation,
and the tested map-owned gameplay projections; gameplay re-entry regenerates the same
fingerprints and category counts. Fog lifecycle is covered by its focused production-plugin
suite rather than the reduced checkpoint harness.

The snapshot column bound is now 131,072 without changing the snapshot wire version. Old
snapshots are repartitioned internally and never serialize chunk metadata.

## Optimized headless comparison

The ignored release checkpoint exercised Grand V3 Baseline and Crystal Mountain from the
same optimized test binary. Each map was generated, fully published through
`TerrainReady`, exported, torn down, regenerated, and compared for exact map-owned re-entry
equivalence. These are one-host checkpoint observations; repeated probes inside each map
are reported as p95 where noted.

`GenerationReport.elapsed_micros` currently spans semantic compilation and voxel
materialization together. The runtime does not expose separate planner and materializer
timers, so the table labels this combined measurement rather than inventing a split.

| World/setup measure | Grand V3 proxy | Crystal Mountain | Grand / Crystal |
|---|---:|---:|---:|
| Initial setup through `TerrainReady` | 1.794 s | 7.435 s | 0.241x |
| Re-entry setup through `TerrainReady` | 1.778 s | 7.438 s | 0.238x |
| Combined semantic compile + voxel materialization | 1.137 s | 7.271 s | 0.156x |
| Fine columns | 105,469 | 18,019 | 5.853x |
| Resident chunks | 444 | 80 | 5.550x |
| Material runs | 421,876 | 62,175 | 6.785x |
| Terrain tile entities | 421,876 | 63,020 | 6.694x |
| Total headless ECS entities | 422,586 | 63,848 | 6.618x |
| Snapshot export | 59.971 ms | 38.523 ms | 1.556x |
| Snapshot encoding | 16.151 ms | 5.142 ms | 3.140x |
| Canonical snapshot bytes | 10,861,429 | 3,017,258 | 3.599x |

The Grand settings fingerprint is `9886354532181515839`; its schematic-aware semantic
fingerprint is `7416488859872558253`, map fingerprint is `15846338855474687564`, and
snapshot fingerprint is `9935533443869518182`. Re-entry reproduces them exactly. The proxy
publishes 15 review anchors, 3,393 special surfaces, 105,469 biome memberships, 34,182
liquid records, and 105,469 surface/illumination records. It initially observes and knows
3,118 player surfaces. It intentionally owns no final objects or lights.

## Interaction probes

The checkpoint repeats deterministic production geometry/solver operations after setup.
The isolated perception probe invokes the production resolver over published occupancy and
illumination. The camera build/rebuild probes exercise the production obstruction-index
geometry helper, not a scheduled camera plugin frame. The measured rebuild repeats a full
index rebuild; production incremental indexing has separate correctness coverage but is not
timed here. The footing probe uses the same
`FootingCache` implementation locally; selection and click systems have separate focused
coverage for cache sharing and invalidation. The reduced benchmark app does not install the
production fog or camera plugins, so these numbers are not mislabelled as whole-frame UI
timings.

| Interaction measure | Grand V3 proxy | Crystal Mountain | Grand / Crystal |
|---|---:|---:|---:|
| Cold footing projection | 9.759 ms | 3.189 ms | 3.060x |
| Footing-cache hit | 5.333 us | 11.791 us | 0.452x |
| Reach query p95 | 43.019 ms | 6.995 ms | 6.149x |
| Deterministic long A* p95 | 32.495 ms | 7.951 ms | 4.086x |
| Isolated perception p95 | 56.707 ms | 10.313 ms | 5.498x |
| Camera obstruction-index build | 32.861 ms | 4.455 ms | 7.375x |
| Full camera-index rebuild p95 | 34.858 ms | 5.263 ms | 6.623x |
| Camera obstruction query p95, 10,000 probes | 2.750 us | 2.667 us | 1.031x |
| One-column edit p95, 24 replacements | 192.377 ms | 65.555 ms | 2.934x |

The Grand reach probe finds 73,259 surfaces. Its long path targets
`TilePos((123, -187), 127)` at cost 434; A* worst-case time was 33.780 ms. Perception
worst-case time was 56.887 ms. Camera query worst-case time was 62.625 us. Every edit
replaced exactly one chunk root and caused exactly one perception rebuild, resolution, and
publication; edit worst-case time was 193.272 ms. Unchanged frames retain zero
recomputation through the existing focused idle gates.

## Renderer-shaped comparison

Warm optimized `map-review` launches generated deterministic 1920 by 1080 top-down
captures after the release binary was already compiled. Wall time includes generation,
renderer initialization, settle frames, readback, and PNG encoding; it is not frame time.
macOS `/usr/bin/time -l` does not expose a reliable steady-state footprint, so only maximum
RSS and peak memory footprint are reported.

| Warm renderer process | Grand V3 proxy | Crystal Mountain | Grand / Crystal |
|---|---:|---:|---:|
| Capture wall time | 10.60 s | 15.09 s | 0.702x |
| Maximum resident set size | 1,521,238,016 B | 859,160,576 B | 1.771x |
| Peak memory footprint | 2,549,385,832 B | 1,357,301,488 B | 1.878x |
| PNG size | 4.0 MiB | 2.0 MiB | 2.0x |

Captures are local review evidence at
`.context/grand-v3-proxy-final/grand-v3-warm.png` and
`.context/grand-v3-proxy-final/crystal-mountain-warm.png`.

The renderer currently has deterministic chunk roots but still presents solid terrain as
one PBR entity per exact material run. A direct “draw-visible solid chunk mesh” count is
therefore unavailable because combined solid meshes do not exist yet; the relevant current
cardinality is 421,876 terrain-run entities. Fog caps are merged by chunk and cutaway
region, while liquid caps are merged by chunk, role, and style; fall curtains remain exact.

## Proposed approval budgets

These are proposed checkpoint budgets, not silently accepted requirements. “Pass” means
the undecorated proxy is already within the proposal. Two latency targets deliberately
remain red because approving the architecture should not normalize visible stutter.

| Measure | Proposed budget | Proxy | Checkpoint state |
|---|---:|---:|---|
| Setup through `TerrainReady` | <= 5 s | 1.794 s | pass |
| Combined compile + materialization | <= 2.5 s | 1.137 s | pass |
| Re-entry setup | <= 5 s | 1.778 s | pass |
| Maximum RSS | <= 2.0 GB | 1.521 GB | pass |
| Peak memory footprint | <= 3.5 GB | 2.549 GB | pass, limited headroom |
| Canonical snapshot | <= 16 MiB | 10.36 MiB | pass |
| Snapshot export / encode | <= 100 / 50 ms | 59.971 / 16.151 ms | pass |
| Cold footing / cache hit | <= 20 ms / 100 us | 9.759 ms / 5.333 us | pass |
| Reach p95 | <= 50 ms | 43.019 ms | pass |
| Long A* p95 | <= 50 ms | 32.495 ms | pass |
| Isolated perception p95 | <= 50 ms | 56.707 ms | **miss** |
| Camera index build / repeated full-rebuild p95 | <= 50 / 50 ms | 32.861 / 34.858 ms | pass |
| Camera query p95 | <= 5 us | 2.750 us | pass |
| One-column edit p95 | <= 100 ms | 192.377 ms | **miss** |

## Decision recorded

Generation at exact scale is viable for this simple proxy, but comparison with the much
more authored Crystal Mountain does not prove that final content is free. Presentation
cardinality is already the dominant architectural risk: 421,876 solid terrain entities and
a 2.55 GB peak footprint exist before trees, bridges, Crystal Ascent, tunnel/interior data,
physical lights, or final hydrology.

The recommended next lane is a separately reviewable combined-solid-terrain mesh per chunk
with exact coordinate/run picking. `VoxelMap`, resident chunk authority, canonical runs,
snapshots, terrain edits, occupancy, LOS, fog, liquids, saves, and semantic identities stay
exact. Only solid presentation and picking change; one affected chunk rebuilds bounded mesh
batches instead of retaining one renderer entity per material run. That migration should
reduce edit and memory pressure. Perception is independent of render-entity cardinality and
requires its own optimization and remeasurement lane to reach 50 ms p95. Both missed
latency targets remain final acceptance gates while content proceeds in parallel.

The checkpoint asked for both:

1. combined solid chunk meshes plus exact picking as the next implementation lane; and
2. the proposed budgets above, including retaining the two currently missed latency targets
   as optimization requirements rather than loosening them.

Alberto approved both decisions on 2026-08-25. The exact scale and proposed budgets remain
fixed, combined solid chunk meshes with exact picking are the approved presentation lane,
and the 50 ms perception plus 100 ms local-edit targets remain requirements rather than
being relaxed. Final implementation and evidence are tracked by
[the delivery manifest](manifest.md); this document remains the immutable before-state for
remeasurement.

## Verification

- `cargo fmt --all -- --check`
- `cargo check --release -p hex_game --tests --features test-support`
- `cargo clippy --release -p hex_game --tests --features test-support -- -D warnings`
- `hex_map` release contract evidence: 98 passed, 1 ignored benchmark
- Grand V3 settings, fingerprint, compiler, height, seam, chunk, and exact-plan tests: passed
- Public exact-schematic compile/export integration: passed
- Movement and pathfinding: 24 passed, 1 ignored benchmark
- Perception: 65 passed, 3 ignored benchmarks
- Camera obstruction index: 61 passed, 1 ignored benchmark
- Fog presentation: 12 passed
- Liquid presentation: 18 passed plus waterfall and lifecycle contracts
- Chunk-local edit and unrelated-metadata locality regressions: passed
- Post-readiness-order release Grand/Crystal proxy checkpoint: passed
- Warm renderer captures: both completed successfully

The full workspace/CI-equivalent gate was deliberately deferred at this checkpoint because
it could not answer the product and architecture choice. It remains a required final-world
gate once the approved mesh, optimization, and content lanes stabilize.
