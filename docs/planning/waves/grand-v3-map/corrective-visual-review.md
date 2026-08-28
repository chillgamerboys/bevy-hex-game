# Grand V3 corrective visual-review manifest

This is the fail-closed pixel-review matrix for the highland, hydrology, Crystal, tunnel,
and ecology corrections prompted by the first complete Grand V3 play review. It supplements
the broad gameplay route in `walks/camera_grand_v3_baseline.ron`; it does not replace that
Map/Character/First Person traversal.

## Evidence identity and launch policy

- Scenario: `Grand V3 Baseline`
- Seed: `1_592_598_566`
- Capture size: the repository's fixed 1920 by 1080 review target
- Output directory: `.context/map-review/<exact-short-head>-grand-v3-corrective/`
- Approval capture begins only after the candidate is committed and `git status --porcelain
  --untracked-files=all` is empty. Record the full `git rev-parse HEAD` value and refuse to
  reuse an existing exact-head output directory. Dirty-worktree renders are scratch evidence,
  not this approval pack.
- Every capture must be regenerated after the final relevant terrain, liquid, prop,
  vegetation, or presentation change.
- Routine captures and the motion route are windowless. A native visible launch is reserved
  for an explicitly requested human play review.
- The complete-footprint frames must show every world boundary with margin. A cropped frame
  fails even if every requested local frame exists.
- Free-camera look-at frames are feature-composition evidence. They do not replace the
  shipped Character and First Person frames named below.

Use this release-shaped base command once per capture row:

```bash
CARGO_INCREMENTAL=0 \
CARGO_PROFILE_RELEASE_DEBUG=0 \
HEX_REVIEW_SCENARIO='Grand V3 Baseline' \
HEX_REVIEW_SEED=1592598566 \
HEX_REVIEW_TIME=12 \
HEX_REVIEW_CAPTURE="$PWD/.context/map-review/<exact-short-head>-grand-v3-corrective/<capture-id>.png" \
cargo run --release -p hex_game --features 'test-support map-review'
```

Set only the row-specific environment values in addition to that base command. Clear
`HEX_REVIEW_FOCUS_ANCHOR`, `HEX_REVIEW_LOOK_AT_ANCHOR`,
`HEX_REVIEW_LOOK_AT_OFFSET`, and `HEX_REVIEW_CHARACTER_RADIUS_SCALE` between rows.

## Required review anchors

The following existing anchors are sufficient and must keep their exact meanings:

- `grand_v3.waterfall_profile`: dry review surface nearest the first real plunge edge;
- `grand_v3.waterfall_base` and `grand_v3.waterfall_crown`;
- `grand_v3.tunnel_mouth` and `grand_v3.tunnel_midpoint`;
- `grand_v3.mountain_lake`, `grand_v3.peak_saddle`, and `grand_v3.massif`;
- `grand_v3.frozen_woods`, `grand_v3.crystal_summit`, `grand_v3.frozen_exit`, and
  `crystal_ascent.upper_exit`. `grand_v3.frozen_exit` is the exact outer centerline
  endpoint of the protected four-wide summit-to-woods connection.

Generation publishes and validates these additional stable review anchors; the matrix remains
incomplete until their captures pass:

- `grand_v3.crystal_mantle_overlook`: an ordinary, valley-side surface outside the Crystal
  site from which the corrected mantle hides the upper Ascent;
- `grand_v3.river_bend`: an ordinary dry bank beside the hero seed's strongest authored
  centerline turn, not merely the nearest bridge or lake cell;
- `grand_v3.lake_island`: one exact rendered surface in the scenic Garden-like mountain-lake
  island footprint;
- `grand_v3.treeline_transition`: an ordinary snowy mountain-base surface that frames a
  vegetated downhill neighbor and a treeless uphill slope;
- `grand_v3.peak_ridge_overlook`: an ordinary ledge that frames both connected peak chains
  and the waterfall opening; the existing saddle alone proves traversal, not the complete
  ridge composition;
- `grand_v3.massif_crest`: the exact exposed world-high surface selected by the centered
  massif field. It must not be rebound to a lower reachable review surface.

`grand_v3.crystal_mantle_overlook`, `grand_v3.river_bend`,
`grand_v3.treeline_transition`, and `grand_v3.peak_ridge_overlook` are gameplay
`MapAnchors`: their rows exercise shipped cameras and therefore require ordinary live
footing. `grand_v3.lake_island` and `grand_v3.massif_crest` are instead
`MapObservationAnchors`: they may be scenic or inaccessible and can be consumed only by
the review-only Map look-at camera, never scenario placement or actor relocation. Every
anchor must resolve to exactly one rendered `HexTile` surface.

## Static capture matrix

`look-at` rows use `HEX_REVIEW_CAMERA=map`, `HEX_REVIEW_LOOK_AT_ANCHOR`, and the stated
world-space `HEX_REVIEW_LOOK_AT_OFFSET`. `focus` rows use `HEX_REVIEW_FOCUS_ANCHOR`; a radius
scale is review-only and must not be presented as the shipped camera distance.

| Capture ID | Criterion | Mode and anchor | Camera / view | Extra |
|---|---|---|---|---|
| `01-grand-v3-corrective-complete-footprint-top-down` | Every boundary and chunk is present | overview | Map / `top-down` | none |
| `02-grand-v3-corrective-complete-footprint-oblique-a` | Whole high-to-low silhouette | overview | Map / `default` | none |
| `03-grand-v3-corrective-complete-footprint-oblique-b` | Opposite whole-world silhouette | overview | Map / `rear` | none |
| `04-grand-v3-corrective-crystal-hidden-valley-a` | Valley view hides upper Ascent behind mantle | focus `grand_v3.crystal_mantle_overlook` | Character / `default` | radius scale `10` |
| `05-grand-v3-corrective-crystal-hidden-valley-b` | Reverse azimuth exposes mantle thickness without a void halo | focus `grand_v3.crystal_mantle_overlook` | Character / `rear` | radius scale `10` |
| `06-grand-v3-corrective-crystal-hidden-valley-first-person` | Shipped eye-height view does not reveal an exposed tower | focus `grand_v3.crystal_mantle_overlook` | First Person / `default` | none |
| `07-grand-v3-corrective-waterfall-plunge-a` | Water visibly leaves a crown and falls rather than following a long ramp | look-at `grand_v3.waterfall_profile` | Map / `default` | offset `80,42,65` |
| `08-grand-v3-corrective-waterfall-plunge-b` | Opposite side shows a vertical face, clear base, and no rear void | look-at `grand_v3.waterfall_profile` | Map / `rear` | offset `80,42,65` |
| `09-grand-v3-corrective-waterfall-base-character` | Recessed receiving water and grounded banks at shipped distance | focus `grand_v3.waterfall_base` | Character / `counter-rotated` | none |
| `10-grand-v3-corrective-tunnel-mouth-a` | Exterior approach reads as concealed mountain terrain | look-at `grand_v3.tunnel_mouth` | Map / `default` | offset `34,18,42` |
| `11-grand-v3-corrective-tunnel-mouth-b` | Reverse side shows continuous cap material and no distinct roof line | look-at `grand_v3.tunnel_mouth` | Map / `rear` | offset `34,18,42` |
| `12-grand-v3-corrective-tunnel-mouth-character` | Entrance scale remains readable without becoming monumental | focus `grand_v3.tunnel_mouth` | Character / `rotated` | none |
| `13-grand-v3-corrective-tunnel-mouth-first-person` | Shipped eye height sees the four-wide opening and grounded ceiling | focus `grand_v3.tunnel_mouth` | First Person / `rotated` | none |
| `14-grand-v3-corrective-river-bend-top-down` | Centerline has an unmistakable bend and non-mechanical banks | look-at `grand_v3.river_bend` | Map / `top-down` | offset `1,90,1` |
| `15-grand-v3-corrective-river-bend-oblique` | Width modulation and one-voxel-recessed water remain legible at ground scale | focus `grand_v3.river_bend` | Character / `counter-rotated` | radius scale `6` |
| `16-grand-v3-corrective-lake-garden-island-a` | Island reads as a deliberate Garden landmark with columns and dense trees | look-at `grand_v3.lake_island` | Map / `default` | offset `58,38,62` |
| `17-grand-v3-corrective-lake-garden-island-b` | Reverse side proves grounded columns, canopy depth, and no floating props | look-at `grand_v3.lake_island` | Map / `rear` | offset `58,38,62` |
| `18-grand-v3-corrective-lake-island-shore-character` | Garden island appears unnatural against the snowy lake basin | focus `grand_v3.mountain_lake` | Character / `rotated` | radius scale `7` |
| `19-grand-v3-corrective-treeline-downhill` | Dense valley to hills to sparse mountain-base gradient is visible | focus `grand_v3.treeline_transition` | Character / `default` | radius scale `10` |
| `20-grand-v3-corrective-treeline-uphill` | Reverse azimuth shows snow increasing and trees ending before summit | focus `grand_v3.treeline_transition` | Character / `rear` | radius scale `10` |
| `21-grand-v3-corrective-treeline-first-person` | Individual high-elevation trees stop at a credible shipped-camera boundary | focus `grand_v3.treeline_transition` | First Person / `rear` | none |
| `22-grand-v3-corrective-peak-ridge-a` | First oblique rejects cylindrical benches and isolated pyramids | focus `grand_v3.peak_ridge_overlook` | Character / `default` | radius scale `12` |
| `23-grand-v3-corrective-peak-ridge-b` | Opposite oblique shows both irregular connected chains and the lake outlet opening | focus `grand_v3.peak_ridge_overlook` | Character / `rear` | radius scale `12` |
| `24-grand-v3-corrective-peak-saddle-first-person` | The authored saddle remains playable beneath treeless snowy summits | focus `grand_v3.peak_saddle` | First Person / `counter-rotated` | none |
| `25-grand-v3-corrective-massif-crest-a` | Exact crest is centered in the massif body, separated from Crystal, and above every peak | look-at `grand_v3.massif_crest` | Map / `default` | offset `110,76,115` |
| `26-grand-v3-corrective-massif-crest-b` | Reverse silhouette proves a connected, irregular mountain instead of a cone on a cylinder | look-at `grand_v3.massif_crest` | Map / `rear` | offset `110,76,115` |
| `27-grand-v3-corrective-crystal-exit-frozen-woods` | Crystal upper route opens directly into Frozen Woods | focus `crystal_ascent.upper_exit` | Character / `rotated` | radius scale `5` |
| `28-grand-v3-corrective-crystal-exit-first-person` | Shipped eye height sees the snowy woodland destination rather than a side opening | focus `crystal_ascent.upper_exit` | First Person / `rotated` | none |
| `29-grand-v3-corrective-full-cutaway` | One continuous tunnel/Crystal interior is exposed without floating trees or retained roof runs | focus `grand_v3.tunnel_midpoint` | Map / `top-down` | full cutaway |
| `30-grand-v3-corrective-illumination-overlay` | The same exact pose shows authoritative Dark/Dim/Bright coverage without changing geometry | focus `grand_v3.tunnel_midpoint` | Map / `top-down` | full cutaway + illumination overlay |

The first capture attempt may tune an offset only when the feature is clipped or buried.
Retain the same distance for the paired azimuth and record the replacement value in the
final review index; do not accept a poor frame merely to preserve this initial offset.

## Motion route

After the static rows pass, revalidate every `expected` position in the broad
`walks/camera_grand_v3_baseline.ron` itinerary and `walks/camera_routes.ron`, refresh any stale
value, then run the dedicated temporal route:

```bash
CARGO_INCREMENTAL=0 \
CARGO_PROFILE_RELEASE_DEBUG=0 \
HEX_WALK_SCRIPT="$PWD/walks/camera_grand_v3_corrective_motion.ron" \
HEX_WALK_OUT="$PWD/.context/map-review/<exact-short-head>-grand-v3-corrective-motion" \
cargo run --release -p hex_game --features visual-walk
```

The windowless runner advances simulation with a manual 60 Hz timestep. Setup and uncaptured
route tails run at 12x, while every `CaptureWhileMoving` interval runs at 1x; synchronous PNG
encoding therefore cannot move the actor farther between two requested frames. The script
publishes exactly 184 indexed frames over twelve fail-closed sequences:

1. `grand_v3.valley_lake` to `grand_v3.waterfall_base` and back: inspect the recessed lower
   approach and receiving-water edges. This route must never continue through the plunge.
2. `grand_v3.mountain_lake` to `grand_v3.waterfall_crown` and back: inspect the upper lake
   shore and lip independently. Keeping the upper and lower pairs separate prevents the
   rejected waterfall ramp—or an unrelated natural-pass detour—from masquerading as evidence.
3. `grand_v3.tunnel_mouth` to `grand_v3.tunnel_midpoint` and back: watch the cap seam,
   repeated tunnel crystals, fog, and entrance occlusion frame by frame.
4. `grand_v3.ascent_threshold` to `crystal_ascent.bottom_chamber` and back: watch the Crystal
   foundation and emissive fixtures for holes, coplanar flicker, and visibility oscillation.
5. `crystal_ascent.corner_landing` to `crystal_ascent.upper_contraction` and back: inspect the
   carved stair supports, wall clearances, and shroud at changing heights.
6. `crystal_ascent.upper_exit` to `grand_v3.frozen_exit` and back: prove the corrected snowy
   woodland connection at close range.

The cadence is distance-calibrated against the shipped `5.0` world-units/second player and the
`HEX_SMALL_DIAMETER` flat-step lower bound. Elevation changes and path detours can only make a
route longer. Each final request retains more than eight 60 Hz updates of conservative arrival
margin, while each direction spans at least half the direct endpoint-duration bound. The paired
reverse leg therefore samples the other end instead of duplicating only the route's first second;
the indexed frames must still confirm that an unexpected detour did not leave the middle
unreviewed.

| Bidirectional pair | Minimum flat-route frames | Frames/request | Requests/direction | Final requested frame |
|---|---:|---:|---:|---:|
| valley lake ↔ waterfall base | `685.89` | `38` | `16` | `608` |
| mountain lake ↔ waterfall crown | `457.26` | `26` | `16` | `416` |
| tunnel mouth ↔ midpoint | `1621.20` | `28` | `32` | `896` |
| Ascent threshold ↔ bottom chamber | `332.55` | `24` | `12` | `288` |
| corner landing ↔ upper contraction | `228.63` | `16` | `12` | `192` |
| Crystal upper exit ↔ Frozen exit | `62.35` | `12` | `4` | `48` |

The runner rejects missing/stale UI state, an unavailable selected actor or render target,
early movement completion, black/write failures, duplicate callbacks, filename collisions, and
interrupted runs. A fully written sequence remains provisional until its contiguous idle wait
and exact arrival proof succeed; failure at either stage removes the sequence and leaves
`review-index.md` incomplete. Inspect every indexed frame. If they cannot establish temporal behavior, report
`HUMAN-MOTION-PENDING`; do not infer a pass from clean endpoints or a zero exit code.

## Acceptance record

The final review index must record exact Git head, commands, output directory, resolved
anchor positions, any tuned offsets, and one `PASS`, `FAIL`, or `BLOCKED` verdict per capture
ID. Inspect every full-resolution artifact once, then scan a complete contact sheet; a
fresh-eyes reviewer who did not implement the correction must challenge the recorded verdicts.
The mechanically completed index remains `UNREVIEWED` until those two inspection passes are
recorded. It must separately record the motion method and verdict. Any cropped overview, exposed
Crystal tower, ramp waterfall, conspicuous tunnel cap line, straight river, generic lake
island, trees at summit, isolated frozen biome, pyramid/cylinder peak, off-center or lower
massif crest, hole, floating prop, or temporal flicker fails this corrective review.
