# Small-geometry world-detail capture harness

> UNAPPROVABLE STRUCTURAL DRAFT — AESTHETIC REVIEW ONLY

`tools/world_detail_experiments.py` is the deterministic external driver for the 60-treatment study. It stages private asset copies, launches the genuine game through Cargo’s release-shaped `map-review` feature, and requires every `foo.png` to have the runtime-written sibling `foo.world-detail-report.json`. It never fabricates a score, winner, image, lifecycle certificate, or performance sample.

The complete study always has 665 logical slots. All 25 neutral, golden, and overcast shared controls are fresh omitted-profile renders from the same current-source provenance as the candidates. Before any study renderer launch, the harness validates a private clean-source `bc06a896…` baseline-oracle pack: its source and recipe, staged assets, producer, four primary controls, 22 clean-source stability repeats, exact file/directory inventory, and no-symlink property are hash-bound by the source-controlled `baseline-oracle-contract-v1.json`. The clean oracle is unchanged. A second source-controlled `control-equivalence-raster-contract-v1.json` hash-binds a private six-process v7c qualification pack and is scoped only to omitted-profile versus explicit-current controls. It adds exactly camera `14` pixel `(1438,273)` with exact endpoints `[164,95,66]` and `[169,133,125]` to that one comparison; every other pixel remains exact except the clean oracle’s pre-existing coordinates. No threshold or spatial expansion is allowed, and the supplemental coordinate is unavailable to clean-oracle and reproduction comparisons. The pack discloses that process-environment snapshots and standalone exit-code receipts were not retained; the six PNGs, reports and receipts, teardown reports, logs, and runtime data are nevertheless file-by-file hash-bound. Each of the four fresh neutral focused controls is a separate one-camera process, matching the oracle recipe; each also serves as the omitted-profile side of control verification and has a separate one-camera explicit-current process mate. The runner orders those eight processes before every treatment job. This preserves exactly four omitted and four explicit-current PNGs while eight distinct runtime receipts prove process isolation. The 611-PNG ceiling applies only to unique non-control treatment renders; the maximally distinct legitimate outcome needs 596. A separate complete-evidence ledger retains those treatments, 25 controls, four explicit-current checks, one deterministic reproduction, and four primary oracle images, for a maximum of 630 accounted PNGs. No blinded result is rejected or altered to manufacture reuse. The 22 oracle repeats and six control-equivalence qualification renders are separately disclosed pre-study renderer-qualification evidence rather than gallery slots. Motion is 22 paired comparisons, implemented as six shared control orbits plus 22 candidate orbits. Each orbit is one genuine v2 runtime launch with 90 captures along a deterministic 20-degree path, not interpolated frames.

Cloud projected coverage is the fraction of a deterministic 256×256 center-sample grid, clipped to the circular massif field, that lies inside the XZ projection of at least one actually emitted low-poly cloud puff. Each octahedral puff projects to its exact rotated-diamond silhouette; cluster-envelope disks do not count as cloud area. The field radius is `clamp(grid_radius × 0.52, 64, 120)`, and every non-control cloud treatment must measure within the exact absolute tolerance `0.01` of its 10%, 18%, or 28% target. Local-fog coverage is the measured fraction of a deterministic, coherent, nested 32×32 density footprint inside every eligible named-anchor volume; it is not rounded against the small anchor count. Neither value claims full-map area coverage.

W04/W05 evaluate the named water half-distance continuously for every exact liquid run and carry the resulting value multiplier in vertex colors inside chunk batches; they do not quantize depth or allocate per-cell materials. I06 uses the single shared ice material for both its solid 0.25-world-unit region and a continuous per-vertex alpha ramp across the exact final 0.10 world units; it has no feather bands or extra materials. C08 likewise uses one shared shadow material with continuous per-vertex alpha from the 20% cap through the complete 24-world-unit radial transition; there is no opacity cutoff or band palette. All of these vertex streams participate in the deterministic projection hashes and runtime vertex counts.

Wet-rim materials are shared per resolved visible shoreline substance, never per cell. In combined profiles, a shore newly covered by the active snow treatment uses the snow presentation as its value/roughness substrate; a removed snow cap resolves back to the underlying non-snow terrain substance and fails closed if that substrate is unavailable.

Cliff value and strata treatments use lit, opaque, coplanar shells at the ordinary terrain roughness of 0.5. Their sRGB colors are the resolved surface-substrate color multiplied by the requested value, with combined value/strata sides partitioned into non-overlapping regions and materials shared across chunks per substrate and treatment; cliff-only profiles therefore require neither OIT nor a depth texture. Shore/pool foam resolves the staged `liquid/foam` swatch and ice fringes resolve the staged `ice` substance color, so settings that do not name a color preserve the current palette and fail closed if it is unavailable.

## Adaptive capture and blinded review

Start with the unresolved neutral screen:

```sh
python3 tools/world_detail_experiments.py scaffold \
  --root /ABSOLUTE/OUTPUT/plan-hex-small-geometry-aesthetic-report-2026-09-02
python3 tools/world_detail_experiments.py run \
  --plan /ABSOLUTE/OUTPUT/plan-hex-small-geometry-aesthetic-report-2026-09-02/capture-plan.json \
  --work-root /ABSOLUTE/WORK/world-detail-runtime
python3 tools/world_detail_experiments.py collect-metrics --plan CAPTURE_PLAN --output METRICS_JSON
python3 tools/world_detail_experiments.py collect-performance --plan CAPTURE_PLAN --output PERFORMANCE_JSON
```

Materialize a public packet containing only opaque randomized codes and a private unblind map outside that packet. Generate two forms, have the reviewers work independently, then derive the next adaptive stage:

```sh
python3 tools/world_detail_experiments.py build-review-packet \
  --plan CAPTURE_PLAN --packet-root PUBLIC_PACKET_DIR --unblind-map PRIVATE_UNBLIND_JSON
python3 tools/world_detail_experiments.py review-template \
  --packet PUBLIC_PACKET_DIR/packet.json --reviewer-id reviewer-a --output REVIEW_A_JSON
python3 tools/world_detail_experiments.py review-template \
  --packet PUBLIC_PACKET_DIR/packet.json --reviewer-id reviewer-b --output REVIEW_B_JSON
python3 tools/world_detail_experiments.py derive-selection \
  --plan CAPTURE_PLAN --packet PUBLIC_PACKET_DIR/packet.json --unblind-map PRIVATE_UNBLIND_JSON \
  --review REVIEW_A_JSON --review REVIEW_B_JSON \
  --metrics METRICS_JSON --performance PERFORMANCE_JSON \
  --output SELECTION_JSON --audit-output DERIVATION_JSON
```

Repeat `scaffold`, `run`, packet creation, review, and derivation after each adaptive stage. Keep the canonical plan at the final required output root from the first scaffold onward, keep raw captures and capture state in fixed disjoint work paths, and use a fresh packet directory outside the publication root for every review iteration. Never relocate the output root after a packet has bound its paths. Neutral evidence promotes two finalists per family. Golden and overcast evidence chooses provisional ladder inputs. Both reviewers then score the fixed neutral interaction ladders; a failed ladder step causally vetoes its introduced family (vegetation and cliffs are coupled at their joint step). Final still evidence resolves provisional atomic and combination looks.

The final still packet includes every one of the 153 neutral final-17 atomic-decision frames (nine families × 17 views), not just combination frames. A no-change motion decision may be scaffolded into `MOTION_GATED_FINALISTS_DERIVED` in the same evidence lineage: private still codes bind the immutable review-material digest, private motion codes bind the immutable visual motion-plan digest, and published motion labels bind source provenance plus that visual digest, so adding findings, pass/fail results, or review links cannot relabel already-reviewed media. If motion replaces any atomic or combination profile, the derivation status becomes `MOTION_RECAPTURE_REREVIEW_REPERF_REQUIRED`; preserve that entire lineage as immutable provisional/failed evidence and start a wholly fresh output, raw-capture, work, and packet lineage. The current v1 state model does not support validated in-place superseded-job rollover. In the fresh lineage, regenerate and review the changed stills, score-leader, motion clips, metrics, and runtime performance before finalization. Profile hashes remain embedded in adaptive still/motion artifact paths so replacement evidence cannot silently reuse stale pixels. `README-INCOMPLETE.md` marks each in-progress root until finalization.

Motion is mandatory before recommendations can finalize:

```sh
python3 tools/world_detail_experiments.py run --plan PRE_MOTION_PLAN --work-root WORK_ROOT --include-motion
python3 tools/world_detail_experiments.py finalize-motion --plan PRE_MOTION_PLAN --clip CLIP_ID
# Repeat finalize-motion for all 22 clip IDs.
python3 tools/world_detail_experiments.py build-motion-review-packet \
  --plan PRE_MOTION_PLAN --packet-root PUBLIC_MOTION_PACKET_DIR \
  --unblind-map PRIVATE_MOTION_UNBLIND_JSON
python3 tools/world_detail_experiments.py review-template \
  --packet PUBLIC_MOTION_PACKET_DIR/packet.json --reviewer-id reviewer-a --output MOTION_REVIEW_A_JSON
python3 tools/world_detail_experiments.py review-template \
  --packet PUBLIC_MOTION_PACKET_DIR/packet.json --reviewer-id reviewer-b --output MOTION_REVIEW_B_JSON
python3 tools/world_detail_experiments.py derive-selection \
  --plan PRE_MOTION_PLAN --packet PUBLIC_STILL_PACKET_JSON --unblind-map PRIVATE_STILL_UNBLIND_JSON \
  --review REVIEW_A_JSON --review REVIEW_B_JSON \
  --motion-review MOTION_REVIEW_A_JSON --motion-review MOTION_REVIEW_B_JSON \
  --motion-packet PUBLIC_MOTION_PACKET_DIR/packet.json \
  --motion-unblind-map PRIVATE_MOTION_UNBLIND_JSON \
  --metrics METRICS_JSON --performance PERFORMANCE_JSON \
  --output FINAL_SELECTION_JSON --audit-output FINAL_DERIVATION_JSON
```

Reviewer files are strict: their rating rows contain only opaque `RV-…` codes and six integer 1–5 categories. The packet and private map are separately hash-bound. The two reviewer IDs must remain the same for still and motion review. Missing or failing candidates resolve to explicit `control/no change`; the harness never invents a replacement score.

Every combined look inherits the final-17 veto of each family it keeps active, including restrained and expressive as well as score-leader. Publication also parses the Atomic recommendations and Combined recommendations table rows against the exact derived decisions, and rejects unresolved `{{…}}` tokens or missing exact H2 report sections.

## Genuine lifecycle and performance evidence

The runtime writes the lifecycle certificate. Python only launches one genuine process and validates the result:

```sh
python3 tools/world_detail_experiments.py run-lifecycle \
  --plan /ABSOLUTE/PATH/capture-plan.json \
  --certificate /ABSOLUTE/PATH/lifecycle-certificate.json \
  --work-root /ABSOLUTE/PATH/world-detail-lifecycle-runtime \
  --timeout-seconds 14400
```

The process performs 100 retained-profile projection teardown/reapply cycles and writes the certificate only after all restoration, zero-allocation, hash-chain, and authority checks pass. Each hash-chained cycle exposes exact-zero counters for renderer-owned fog images and for missing terrain-material, liquid-visibility, and vegetation-scale restoration targets. Ordinary per-capture sidecars are first persisted as incomplete evidence, then atomically finalized only after teardown verifies zero remaining review allocations, target removal, and restoration of camera, OIT, transmission, depth, and volumetric state. The 100-cycle certificate remains the independent repeated-lifecycle proof.

Every automated process gets a fresh 64-hex launch nonce. Runtime-written receipts bind that nonce, nonzero process ID, executable hash, source-provenance hash, exact runtime capture-plan bytes, and profile hash. The runner first scrubs inherited `HEX_*` state, then explicitly injects `HEX_GRAND_V3_STRUCTURAL_REVIEW_DRAFT=1` for both still and lifecycle launches; the map-review-only flag is recorded in plan provenance, the capture contract, attempt evidence, and lifecycle launch evidence. It is not a casual opt-out: the report remains an unapprovable structural draft while Grand V3 is validator-blocked. Resumed capture state and the lifecycle launch ledger must agree with those receipts; a pre-existing certificate or output is rejected. The complete still plan also schedules one independent rerun of score-leader at neutral camera `02` (explicit-current if the leader is control) and requires exact equality at every raster-stable pixel plus exact projection hashes, effect validation, and stable world/report state across the two fresh processes. Camera `02`'s one clean-source-proven ambiguous shared-vertex coordinate is the only spatial exception; treatment-specific endpoint RGB tuples are recorded exactly, with no color or distance threshold. Control-to-oracle and omitted-to-explicit-current comparisons remain stricter: any differing tuple at an enumerated coordinate must be one of the exact clean-source-observed values. Raw PNG equality is recorded when it happens but is not falsely claimed as universally satisfiable on this Metal baseline.

Runtime performance uses the final 60 frames of the first 90-frame settle interval, after 30 warm-up frames, with nearest-rank p95. The stable sample is cached across later cameras or motion frames in the same process sequence; lifecycle reentry resets it. Resident presentation bytes cover unique live mesh buffers, relevant material allocations, non-capture image mip payloads, and review entity/component/name payloads. The final leader is matched against control and must remain within 15% for both p95 frame time and resident bytes.

## Publication

Raw, unwatermarked metric pixels stay in the external work root. Every PNG beneath the final output root is a visibly warning-labeled derivative. The gallery embeds its synchronized data and opens directly under `file://`; `gallery/data.json` remains a separate audit artifact. Every runtime sidecar is also mirrored byte-for-byte under `runtime-reports/`.

All shared controls are current-run captures with runtime sidecars and receipts and one pinned executable digest. The four neutral focused controls are captured by four fresh processes and aggregated only after all four one-camera records validate. They are compared with the clean `bc06` source oracle using exact decoded RGB equality outside eight individually enumerated pixels across all four views; at those coordinates only RGB tuples observed in repeated clean-source runs are accepted. Their four explicit-current verification mates likewise use four separate processes. There is no SSIM, DeltaE, spatial, or color-distance tolerance. The September-2 aesthetic report is preserved only as historical research, camera-contract, and rejected-bevel evidence: its README and manifest are copied into `provenance/prior-aesthetic-report/` and checked against their pinned hashes. Its older-source pixels are never used as this study's oracle.

After motion-gated selection and all captures validate, write the evidence derivatives, author the narrative/workbook, and install them through their strict contracts:

For every publication/finalization command below, `FINAL_PLAN` must be the regular, non-symlinked `OUTPUT_ROOT/capture-plan.json`; an equivalent copy elsewhere is rejected.

```sh
python3 tools/world_detail_experiments.py build-review-derivatives \
  --plan FINAL_PLAN --review REVIEW_A_JSON --review REVIEW_B_JSON \
  --metrics METRICS_JSON --performance PERFORMANCE_JSON
python3 tools/world_detail_experiments.py install-narrative --input AUTHORED_README --root OUTPUT_ROOT
python3 tools/world_detail_experiments.py install-workbook \
  --input FORMULA_WORKBOOK --root OUTPUT_ROOT --review-results OUTPUT_ROOT/review.json
python3 tools/world_detail_experiments.py build-sheets --plan FINAL_PLAN
python3 tools/world_detail_experiments.py finalize-manifest \
  --plan FINAL_PLAN --state CAPTURE_STATE --lifecycle LIFECYCLE_CERTIFICATE \
  --review REVIEW_A_JSON --review REVIEW_B_JSON \
  --metrics METRICS_JSON --performance PERFORMANCE_JSON
python3 tools/world_detail_experiments.py validate-publication --plan FINAL_PLAN
```

The narrative must substantively cover research, recommendations, rejected settings, interaction findings, limitations, and implementation cost; cite all eight prescribed research sources; describe all nine families and actual decisions; retain the bevel-rejection findings; and link the evidence artifacts. The XLSX must have relationship-resolved `Ratings`, `Rankings`, `Performance`, and `Recommendations` sheets, formulas with the required dependency direction, automatic full recalculation, the warning on every sheet, all 60 profile IDs, actual decisions, and the canonical-object SHA-256 of the review results. That workbook binding is intentionally distinct from the byte SHA-256 of the pretty-printed `review.json` file.

Runtime report validation additionally checks the sampled union of the exact rendered cloud-puff XZ silhouettes over the deterministic massif field, with tolerance fixed at `0.01`; exact ceiling-ranked ice edge selection; and sorted named waterfall landing evidence for spray treatments. Motion packet paths, visible labels, PNG metadata, and encoded frames expose only opaque review codes; a private hash-bound map performs the later unblinding. Final validation re-encodes every MP4 and strip from its exact planned frame hashes and verifies 90 frames at 30 fps with the exact three-second duration.

Run the static harness checks without Rust/Cargo:

```sh
python3 -m py_compile tools/world_detail_experiments.py tools/world_detail_experiments/test_harness.py
python3 tools/world_detail_experiments.py self-check
python3 tools/world_detail_experiments/test_harness.py -v
```

The ordinary planning camera manifests under `specs/` are immutable semantic copies. `camera-provenance-v1.json` binds each copy to an available absolute upstream path, exact raw hashes, and a canonical semantic hash before planning proceeds. Camera provenance remains planning authority; it is not a source of control pixels.
