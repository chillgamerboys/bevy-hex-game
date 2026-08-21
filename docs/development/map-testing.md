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
select both gameplay and map consumers. The combined shared terrain-impact source also
owns the damaged-health projection, so path-level changes remain full; the foundation
PR records a narrower exact validator/schedule/producer wedge as review evidence rather
than weakening that fail-closed route. Unclassified shared core/assets, other world
crates, selector-command or CI-topology changes, unknown paths, invalid configuration,
and empty diffs fail closed.
Pushes to `dev` or `main` and selector changes force the complete gate. Final wave
candidates run the selector-chosen gate over their exact combined diff.

The executable completeness guard is:

```sh
python3 tools/test_scope.py check-partitions map
```

It lists the exact package and targets through nextest, rejects overlap or omission,
and confirms ignored stress tests remain discoverable. CI publishes separate JUnit,
timing JSON, and logs for all three ordinary concerns.

## Evidence boundaries

- Screenshots and rendered frames are valid evidence for a rendered map's visible
  geometry, materials, lighting, cutaways, seams, composition, and static camera
  framing/occlusion. Video and human checks are valid for camera motion, native-input
  response, animation, control feel, and taste. A static screenshot does not prove
  motion or control feel.
- A visual artifact may show how exact map state already established by hooks is
  rendered, but screenshots, frames, video, and human observation must never prove or
  corroborate world logic that typed map hooks or contracts can express. Add a narrow
  hook instead of inferring state from pixels.
- In particular, a screenshot cannot prove exact `TilePos`, `RunBottom`, `HexSpan`,
  `Headroom`, region membership, edit rebuilding, determinism, or teardown.
- A synthetic gameplay surface cannot prove the real map publisher.
- World floats, transforms, `level_height`, and saturated headroom cannot reconstruct
  voxel occupancy; publication tests assert the integer contract directly.
- Fixed-frame tests remain fixed when frame count is the invariant. Bounded settling
  is used only when convergence, rather than an exact frame, is the contract.
- Forest, Waterfall, Arid, map-review captures, and their acceptance criteria remain
  owned by the existing visual workflow and are not redefined by this partition.
  Arid review must show all three Desert Transition bands, Desert Plain's open
  relief, both a dune crest and trough, and the oasis with both surrounding rings;
  typed map tests, not those frames, prove exact material coverage, one-level dune
  traversal, local-water isolation, date-palm blockers, seam redundancy, and
  reachability.
- Island review must show all five Sandy Islets and the playable primary route, the
  Wooded Island beach/interior/ridge progression, and Ocean Archipelagoes' open sea,
  three satellite clusters, landing, wooded heart, and only dry seam. Typed tests,
  not those frames, prove level-8 water continuity, exact component counts, two-column
  sand fringes, tree exclusions, ordinary reachability, scenic disconnection, and
  teardown/re-entry. No capture may be used as evidence that water is impassable.

## Visual-pack close-out

A successful visual-walk process proves that the route, renderer, and structural capture
oracles completed. It does **not** mean that somebody inspected the resulting pixels. At walk
startup, `review-index.md` is atomically replaced with an **INCOMPLETE — NOT REVIEWABLE**
marker. This invalidates any checked index left by an earlier run; if the process aborts, the
directory remains explicitly non-reviewable even when old PNGs are still present.

Only when the persisted capture count exactly matches the script's expected count does the
walk atomically replace that marker with a completed index. It records the run ID, script path,
planned and launched scenarios (including resolved seeds), and capture count, then embeds every
frame in script order. Every frame begins as `UNREVIEWED`; a reviewer must replace that result
with explicit `PASS` or `FAIL` and fill in its notes before curating a smaller hero set or calling
the candidate review-ready. Looking only at the images selected for a summary is not a review of
the pack.

Corrective packs require a second, fresh-eyes evidence pass after the primary full-resolution
inspection. Scan a contact sheet containing the complete capture set to expose inconsistent
framing or systematic artifacts, then challenge each PASS note against the pixels it actually
shows. Repeated generic notes are not acceptable when a frame is occluded, dominated by a
near-camera prop, or does not visibly support that criterion. Recapture such a frame or mark it
FAIL; capture-count completeness never upgrades weak evidence.

Authored and composed maps add two required stress checks:

- Inspect a character-height view across every landmark/biome seam and entrance apron, with
  enough exterior sky in frame for a missing foundation, skirt, or cutaway-owned run to be
  obvious. Crystal Mountain specifically includes the lower-entry apron and the complete base
  of Crystal Ascent in this check. Its authored stair annulus must also prove exact continuous
  occupancy from level 0 through the chamber datum; validating only the elevated treads is not
  sufficient foundation evidence.
- Orbit the native camera past repeated emissive, translucent, or animated props. Static frames
  cannot clear flicker or temporal visibility changes; the motion check remains a named-human
  gate even when every still is clean.

When visual review finds a defect, fix the renderer or authored geometry, add the narrowest typed
root-cause invariant available, and recapture the affected route. Old frames are not evidence for
the corrected candidate.
