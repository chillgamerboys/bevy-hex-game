# Grand V3 final-content runtime checkpoint

- Status: `headless budgets passing; renderer memory, visual, and full-selector gates remain`
- Branch: `wave/grand-v3-schematic`
- Measurement date: 2026-08-27
- Scenario: `Grand V3 Baseline`
- Seed: `1_592_598_566`
- Host: Apple M1 Pro, 16 GiB RAM, macOS
- Build: release, incremental compilation disabled, release debug data disabled

This is the final-content remeasurement required by the approved proxy checkpoint. It
includes the completed hydrology, routes, bridges, vegetation, tunnel, Crystal Ascent,
interiors, authored objects, lights, combined terrain batches, exact picking, fog, and
runtime projections. It does not replace the historical proxy measurements in
[proxy-checkpoint.md](proxy-checkpoint.md).

The optimized deterministic comparison passed every approved latency and snapshot-size
budget. Values are one-host observations; repeated interaction probes report p95.

| Measure | Grand V3 final | Crystal Mountain | Approved Grand budget | Result |
|---|---:|---:|---:|---|
| Initial setup through `TerrainReady` | 3.210 s | 6.378 s | 5 s | pass |
| Re-entry setup through `TerrainReady` | 3.045 s | 6.422 s | 5 s | pass |
| Semantic compile + voxel materialization | 2.099 s | 6.183 s | 2.5 s | pass |
| Re-entry compile + materialization | 1.960 s | 6.223 s | 2.5 s | pass |
| Snapshot export | 91.110 ms | 36.959 ms | 100 ms | pass |
| Snapshot encoding | 18.247 ms | 4.445 ms | 50 ms | pass |
| Canonical snapshot | 12,550,364 B | 3,017,258 B | 16 MiB | pass |
| Cold footing projection | 13.676 ms | 2.072 ms | 20 ms | pass |
| Footing-cache hit | 35.375 us | 11.542 us | 100 us | pass |
| Reach query p95 | 35.211 ms | 6.239 ms | 50 ms | pass |
| Deterministic long A* p95 | 39.083 ms | 4.716 ms | 50 ms | pass |
| Perception resolver p95 | 25.535 ms | 5.700 ms | 50 ms | pass |
| Camera obstruction-index build | 31.798 ms | 4.426 ms | 50 ms | pass |
| Camera obstruction-index rebuild p95 | 33.694 ms | 4.636 ms | 50 ms | pass |
| Camera obstruction query p95 | 3.042 us | 2.625 us | 5 us | pass |
| One-column edit p95 | 77.638 ms | 22.452 ms | 100 ms | pass |

The final map publishes 105,469 columns, 425,443 material runs, 444 resident chunks,
426,891 logical terrain-run records, 2,147 combined solid render batches, 1,305 authored
objects, 58 gameplay lights, 32 physical point lights, and 431,162 headless ECS entities.
Every measured edit replaced exactly one chunk root and published exactly one coordinated
terrain, illumination, observation, and knowledge update.

Three independent strict samples after the corrective art pass measured initial builds at
2.099 s, 2.055 s, and 2.047 s, with re-entry builds at 1.960 s, 1.981 s, and 1.970 s.
The worst sample therefore retains about 401 ms of the unchanged 2.5 s budget. The recovered
margin comes from exact semantic-preserving work: radius-32 Crystal-disk distances use their
closed form, peak-foothill search stops after the first viable equal-gap group, corrective
review selections are sealed once and locally revalidated, and coast connectivity is admitted
constructively rather than replaying two redundant final whole-world floods. The profiled
hero build reduced the peak-foothill stage from about 187 ms to 5.7 ms, corrective validation
from about 211 ms to 48.6 ms, and coast construction from about 186 ms to 76.5 ms. Exact hero
fingerprints remained unchanged.

The full renderer was also exercised through the windowless 1920 by 1080 review path.
Maximum RSS was 3,835,658,240 bytes against the approved 2.0 GB target, and the macOS
peak-memory-footprint counter was 7,630,740,032 bytes against the approved 3.5 GB target.
Both are **misses**.

A second windowless run sampled the same Cargo/game process RSS throughout generation,
publication, settling, and capture. The process held roughly 0.28--0.47 GB while generating,
rose to about 0.79 GB when the compiled world was ready for publication, and peaked at
2,940,192 KiB (about 2.94 GB) while terrain meshes were being extracted and uploaded. It
then fell through about 1.72 GB to a settled 0.50--0.72 GB. Requesting and completing the
1920 by 1080 PNG raised that settled process to about 0.78 GB. The earlier whole-process
maximum is therefore not permanent world residency or primarily PNG readback: the actionable
high-water phase is the transient terrain publication/extraction wave. It still exceeds the
approved 2.0 GB maximum-RSS target, so the renderer memory gate remains open.

The retained mesh audit points first to reducing duplicate main-world/render-world mesh
storage during publication, avoiding presentation caps for buried terrain runs, and making
fog caps top-only rather than full thin prisms. Those are optimization candidates, not yet
passing evidence; scale and fidelity remain unchanged until a measured fix lands.

Stable hero identities at this checkpoint are:

- schematic fingerprint: `0x8d2c_feac_6856_09ec`
- compiled semantic fingerprint: `0x617e_db14_f9e5_72b1`
- map fingerprint: `0x1e41_dc04_f9da_c0df`
- snapshot fingerprint: `0xc994_a636_2a71_1576`

## Exact gate

```text
CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_DEBUG=0 cargo test --release -p hex_game --features test-support --lib scenarios::tests::grand_v3_headless_runtime_gate_compares_to_crystal_mountain -- --ignored --exact --nocapture --test-threads=1
```

The final 32-seed fully materialized release corpus also passes in 64.65 s. The remaining
delivery gates are renderer publication-memory optimization, the complete offscreen visual
matrix and motion review, the selector-generated CI-equivalent gate, and named human
visual/play approval.
