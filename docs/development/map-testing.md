# Map testing contract

This document defines how `hex_map` behavior is tested without moving world authority
into shared fixtures. The map owns its substances, generator inputs, V1/V2/V3 plans,
publication assertions, edit/rebuild evidence, and visual acceptance. Shared test code
supplies only the neutral deterministic app shell.

## Concern partitions

| Concern | Owns | Oracle | Command | Ordinary budget |
|---|---|---|---|---:|
| Map unit | Voxel storage, settings, grid projections, and renderer-independent presentation calculations | Exact values, columns, resources, and publication inputs | `python3 tools/test_scope.py run map_unit` | 60 s total |
| Map generation | Frozen V1/V2 oracles, V3 semantic plans, validation, selection, fingerprints, and every 128-seed PR corpus | Deterministic plans, exact projections, fingerprints, named regressions, and fallback bounds | `python3 tools/test_scope.py run map_generation` | 60 s per test; 5 min total |
| Map contracts | Real-plugin construction, run publication, terrain edits, presentation entities, teardown, and re-entry | Components, resources, exact voxels, entity lifecycle, and regenerated state | `python3 tools/test_scope.py run map_contracts` | 60 s per test; 3 min total |
| Visual review | Geometry, materials, lighting, cutaways, composition, and motion | Existing map-review captures and human play | Existing map, Forest, and Waterfall routes | Existing criteria |
| Stress/performance | 10,000-seed corpora, reports, benchmarks, and localized edits | Release-mode diagnostics and explicit bounds | `.github/workflows/stress.yaml` | Scheduled/manual only |

The ordinary partitions retain every existing PR seed. Long scheduled gates are not
promoted to ordinary CI, and ordinary corpora are not shortened to manufacture a
timing pass.

## Target topology

`hex_map` declares `autotests = false` and one integration target,
`tests/contracts.rs`. Its modules group support fixtures, publication, terrain edits,
procedural publication, composed worlds, presentation, and lifecycle/re-entry while
linking Bevy once. Adding a helper file cannot silently create another test binary.

The target uses `hex_test_support::TestAppBuilder` for deterministic app construction,
plugin finalization, and state entry. Substance tables, runtime-art catalogs,
procedural settings, terrain edits, and acceptance assertions remain in `hex_map`.
`SyntheticArena` is consumer-owned evidence and must never replace the real map
publisher in this target.

The `map-test` Cargo profile keeps `hex_map` at optimization level 1 and reuses the
workspace's optimized dependency policy. This changes test execution only; release
and shipping profiles are unchanged.

## Selection and completeness

`.config/test-scopes.json` is the single authority shared with gameplay. Inspect a
branch with:

```sh
python3 tools/test_scope.py plan --base origin/dev --head HEAD
```

Map integration-test changes select contracts; procedural source changes select
generation plus contracts; grid/presentation source changes select unit plus
contracts; shared map foundations select all three. Shared `hex_test_support` changes
select both gameplay and map consumers. Other world crates, shared core/assets,
unknown paths, invalid configuration, and empty diffs fail closed. Pushes to `dev` or
`main` and final wave candidates force the complete gate.

The executable completeness guard is:

```sh
python3 tools/test_scope.py check-partitions map
```

It lists the exact package and targets through nextest, rejects overlap or omission,
and confirms ignored stress tests remain discoverable. CI publishes separate JUnit,
timing JSON, and logs for all three ordinary concerns.

## Evidence boundaries

- A screenshot cannot prove exact `TilePos`, `RunBottom`, `HexSpan`, `Headroom`,
  region membership, edit rebuilding, determinism, or teardown.
- A synthetic gameplay surface cannot prove the real map publisher.
- World floats, transforms, `level_height`, and saturated headroom cannot reconstruct
  voxel occupancy; publication tests assert the integer contract directly.
- Fixed-frame tests remain fixed when frame count is the invariant. Bounded settling
  is used only when convergence, rather than an exact frame, is the contract.
- Forest, Waterfall, map-review captures, and their acceptance criteria remain owned
  by the existing visual workflow and are not redefined by this partition.
