# Grand V3 visual experiments

`tools/visual_experiments.py` produces a deterministic, review-only comparison pack for
outdoor and indoor lighting, materials, tactical visibility, voxel aspect, and
world-palette and edge-response experiments. It never
edits `assets/`, never adds a candidate to a shipped scenario, and never publishes an
incomplete pack.

This is presentation evidence, not a world-logic oracle. Typed map tests remain the
authority for topology, heights, routes, illumination tiers, occupancy, and fingerprints.
The captures show how an already-established state renders.

## Canonical matrix

The strict registry is `tools/visual_experiments/profiles.json`. It fixes scenario
`Grand V3 Baseline`, seed `1592598566`, an eight-view screen matrix, and these twenty-four
one-factor profiles:

| Profile | Only changed axis |
|---|---|
| `e00-baseline` | Shipped state at clear noon (`12.0`) with tactical fog `current` |
| `l01-midnight` | Clear-cycle time `0.0` |
| `l02-dawn` | Clear-cycle time `6.5` |
| `l03-golden` | Clear-cycle time `16.5` |
| `l04-overcast` | Shipped static `config/lighting/overcast.ron` |
| `l05-soft-fill-noon` | Softer clear-noon key lighting and brighter fill |
| `l06-high-contrast-noon` | Stronger clear-noon directional contrast |
| `z01-haze-light` | Promoted light-haze baseline equivalence control |
| `z02-haze-medium` | Medium clear-noon atmospheric haze |
| `i01-crystal-tight` | Set every generated crystal point-light range to `3.0` |
| `i02-crystal-broad` | Set every generated crystal point-light range to `7.0` |
| `i03-heart-feature-shadow` | Enable shadow maps only for the Crystal heart light at local offset level `18` |
| `v01-fog-none` | Review-only tactical terrain shading `none` |
| `v02-fog-dimmed` | Review-only tactical terrain shading `dimmed` |
| `v03-fog-observed-only` | Review-only tactical approximation `observed-only` |
| `v04-fog-softened` | Review-only tactical terrain shading `softened` |
| `m01-matte-terrain` | Fully rough terrain while preserving current object materials |
| `m02-unified-matte` | Fully rough terrain and authored-object materials |
| `e01-micro-bevel-004` | Blend terrain and object normals toward their adjacent edges by `0.04` |
| `e02-micro-bevel-008` | Blend terrain and object normals toward their adjacent edges by `0.08` |
| `h01-flat-030` | `level_height: 0.30` |
| `h02-tall-055` | `level_height: 0.55` |
| `p01-muted-earth` | Complete muted-earth world palette |
| `p02-high-separation` | Promoted high-separation baseline equivalence control |

The two candidate palettes live under `tools/visual_experiments/palettes/`, outside
the shipped asset tree. They contain exactly the shipped swatch vocabulary and only
replace RGB values in the private staged copy.

Every profile captures the same screen views:

1. the unfocused, full-footprint top-down Map;
2. a free oblique Map view looking at the scenic Massif crest;
3. a free oblique Map view of the coast and river outlet at `grand_v3.coast`;
4. a free oblique Map view of the mountain-lake Garden island;
5. a reverse Character view across the treeline transition;
6. a rotated Character view at `grand_v3.waterfall_base`;
7. First Person at `grand_v3.tunnel_mouth`;
8. a normal-presentation Character view inside `crystal_ascent.bottom_chamber`.

Full-cutaway and illumination-overlay diagnostics remain available as separate map-review
evidence. They are deliberately excluded from the one-factor screen matrix so an overlay
cannot mask the indoor lighting and material differences that the chamber view is meant
to compare.

The registry declares two exact named capture sets. `screen` is all eight views and is
the default. `smoke` is the four-view full-footprint/highlands/waterfall/tunnel preflight.
A set may not silently reorder, duplicate, or name an unknown capture, and `screen` must
remain complete. Use smoke to reject a broken render cheaply; use screen before comparing
or selecting a look.

Capture rows can express either actor relocation with `focus_anchor` or review-only free
framing with paired `look_at_anchor` and three-component `look_at_offset`. They may also
request the existing exact `full` cutaway and `overlay` illumination diagnostics. The
schema rejects partial look-at pairs, actor/look-at mixtures, non-Map free cameras,
cutaways without a focused interior, and overlays without a full cutaway.

The matrix intentionally avoids a full cross-product. After a human selects a promising
visibility, height, or palette treatment, add a separately reviewed interaction matrix
rather than changing the canonical one-factor profiles.

## Night aesthetic interaction sweep

The separate strict specification
`tools/visual_experiments/sweeps/night-aesthetic-v1.json` composes presentation axes
without weakening or editing the canonical twenty-four-profile registry. It is now
retained as **historical review provenance**: the selected high-separation palette and
light noon haze were promoted to the shipped baseline, so its former `pshipped` versus
`pseparate` and `z000` versus `z003` axes no longer produce distinct current renders.
Its golden-hour haze implementation also targeted the noon anchor. The tool therefore
allows validation and report inspection but refuses new broad, golden, or adaptive
captures from this specification. Create a new sweep against the promoted baseline for
future experiments.

Validate both historical and canonical contracts with:

```sh
python3 tools/visual_experiments.py validate
python3 tools/visual_experiments.py validate-sweep \
  --spec tools/visual_experiments/sweeps/night-aesthetic-v1.json
```

At the time of the original review, the `broad` tier described the Cartesian product
of three voxel heights (`0.30`, `0.35`, `0.40`), three noon light rigs (balanced,
soft-fill, high-contrast), three palettes (shipped, muted earth, high separation),
three haze states (`0`, `0.0003`, `0.0007`), and three normal responses (current,
`0.04`, `0.08`). It therefore contains exactly 243 stable recipe IDs (not 243 distinct
renders against today's promoted baseline). Contiguous one-based shards keep one height
per 81-look capture boundary. The optional `golden` tier replaces the three noon rigs
with one `16.5`-hour state and contains one additional 81-look shard.

The former capture command is shown only for provenance; it now fails before creating
output or invoking Cargo:

```sh
python3 tools/visual_experiments.py run-sweep \
  --tier broad \
  --shard 1 \
  --output-root /absolute/task/work/aesthetic-sweep \
  --allow-structural-draft \
  --dry-run
```

Historical outputs used the destination
`<output-root>/night-aesthetic-v1/<tier>/shard-NN`. Each shard is staged privately and
published with an atomic no-replace rename only after all 81 PNGs, sidecars, hashes,
and indexes validate. Repeating the exact command validates and resumes an already
published shard without building or recapturing. A changed spec, source hash, Git head,
or worktree digest refuses resume. Failed captures are retried once at identical
settings; a twice-failed shard remains unpublished.

Every look has a descriptive ID such as
`h030-lsoft-pearth-z003-e004`, a semantic SHA-256, the five fully resolved axes,
source/config hashes, seed, camera, start/completion timestamps, and retry state in its
sidecar. Light-rig and haze patches are merged only when their fields are disjoint.
Height, palette, and light changes occur only in the private staged asset tree; fog,
material, and edge choices use review-only runtime settings. The sweep parser also
recognizes `geometric-bevel-004` and `geometric-bevel-008` for a later finalist recipe,
while the broad factorial deliberately uses only normal-based edge treatments.

Source must remain frozen for a complete shard. The output root may be outside the
repository (the normal overnight-work location). A destination inside the repository
is accepted only below `.context/grand-v3-visual-experiments`, whose captures are
excluded from the worktree provenance digest.

After shards publish, create a mechanically validated blind score worksheet:

```sh
python3 tools/visual_experiments.py score-sweep \
  --manifest /absolute/task/work/aesthetic-sweep/night-aesthetic-v1/broad/shard-01/manifest.json \
  --manifest /absolute/task/work/aesthetic-sweep/night-aesthetic-v1/broad/shard-02/manifest.json \
  --manifest /absolute/task/work/aesthetic-sweep/night-aesthetic-v1/broad/shard-03/manifest.json \
  --output /absolute/task/work/aesthetic-sweep/broad-scorecard.json
```

The scorecard verifies every PNG and manifest hash, identifies exact duplicate images,
assigns stable blind IDs, and provides empty two-reviewer fields for the approved six
weighted criteria. It does not invent aesthetic scores. `report-sweep --selection ...`
accepts a strict selection JSON containing exactly twelve ranked winners and four unique
representatives, validates 1–5 scores and provenance against that scorecard, and emits a
Markdown summary to stdout or a new `--output` path. Existing scorecards and reports are
never replaced.

### Adaptive selection shards

Later funnel stages use a separate external selection JSON; they never add mixed profiles
to `profiles.json`. A selection names resolved broad or golden look IDs, an ordered set of
captures, a shard count, and exactly one of:

- `matrix`: a Cartesian product over material, tactical shading, Crystal light, and edge
  overrides for every selected base look;
- `recipes`: an explicit ordered list of base-look/override pairs for locked finalists.

This bounded semifinal example captures the highlands hero plus five additional canonical
views for each selected look:

```json
{
  "version": 1,
  "id": "night-semifinal-v1",
  "stage": "semifinal",
  "sweep_id": "night-aesthetic-v1",
  "shard_count": 3,
  "base_look_ids": [
    "h030-lsoft-pearth-z003-e004",
    "h035-lbalanced-pshipped-z000-ehard"
  ],
  "capture_ids": [
    "01-world-topdown",
    "02-highlands-oblique",
    "03-coast-river-outlet",
    "04-garden-island-oblique",
    "05-treeline-character",
    "06-waterfall-character"
  ],
  "matrix": {
    "material_treatment": ["current"],
    "fog_mode": ["current"],
    "crystal_light_profile": ["current"],
    "edge_treatment": ["inherit"]
  }
}
```

Validate and inspect an exact shard without capture, then remove `--dry-run` only after
the worktree is frozen:

```sh
python3 tools/visual_experiments.py validate-selection \
  --selection /absolute/task/work/night-semifinal-v1.json

python3 tools/visual_experiments.py run-selection \
  --selection /absolute/task/work/night-semifinal-v1.json \
  --shard 1 \
  --output-root /absolute/task/work/aesthetic-sweep \
  --allow-structural-draft \
  --dry-run
```

The four strict override fields support the planned interaction passes:

- materials: `current`, `matte-terrain`, `unified-matte`;
- tactical fog: `current`, `dimmed`;
- Crystal light: `current`, `i01-crystal-tight`, `i02-crystal-broad`,
  `i03-heart-feature-shadow`;
- edge treatment: `inherit`, `current`, both normal blends, and
  `geometric-bevel-004` / `geometric-bevel-008`.

For example, one base look crossed with the two fog values and four Crystal values
produces exactly eight tunnel/chamber recipes. The bevel pass crosses `inherit` with the
two geometric values. Explicit finalist recipes may lock any one combination per base
look without generating unwanted cross-products. Recipe IDs and hashes are derived from
the base-look identity plus all four overrides, so duplicate resolved recipes fail
validation.

An optional `camera_manifest` is a canonical relative path beside the selection JSON. Its
strict schema is `{ "version": 1, "id": "...", "captures": [...] }`, using the same
camera, view, focus/look-at, cutaway, and overlay fields as `profiles.json`. It is
authoritative for capture order. Reused canonical IDs must match their canonical
definitions exactly. A `finalist` selection that supplies a camera manifest must select
exactly seventeen unique views; this is the frozen final itinerary. A `motion-samples`
selection can use a supplied sequence of static orbit positions, but its index remains
`HUMAN-MOTION-PENDING`: stills cannot clear shimmer, popping, input, or motion quality.

Selection files and external camera manifests are hashed into every plan, manifest, and
sidecar. `run-selection` otherwise uses the same build-once, private asset staging,
retry-once, full-worktree source freeze, strict PNG validation, atomic publication, and
exact resume checks as `run-sweep`. Published selection manifests are also accepted by
`score-sweep`.

### Initial screening set

Use the named `initial` selection for the first bounded pass. It retains the noon
baseline and nine candidates: golden hour, soft-fill noon, medium haze, observed-only and
softened visibility, matte terrain, both voxel-height ratios, and muted earth. This is
the smallest canonical selection that covers outdoor light, atmosphere, tactical
visibility, materials, both sides of the voxel-height experiment, and palette without
mixing any two axes in one profile.

Run the four-view preflight first (40 renders), then the eight-view screen only after the
map and framing pass inspection:

```sh
python3 tools/visual_experiments.py run --profile-set initial --capture-set smoke --dry-run
python3 tools/visual_experiments.py run --profile-set initial --capture-set smoke
python3 tools/visual_experiments.py run --profile-set initial --capture-set screen
```

The dry-run plan and published root manifest both contain `comparison_report`: canonical
profile and capture IDs, ordered axes, render/comparison counts, a selection ID, and a
semantic SHA-256. The Markdown and HTML indexes repeat that hash so a screenshot report
cannot be mistaken for a different matrix with similar captions.

### Indoor crystal-light profiles

The approved tighter-light, broader-light, and selectively shadowed feature-light tests
are specified exactly in
`tools/visual_experiments/lighting/indoor-crystal-v1.json`. They are active strict
`indoor-lighting` profiles. The file also pins the shipped physical-light baseline:
`4,500 lm`, `4.5` range, `0.12` radius, with shadow maps and contact shadows disabled.
These candidates use a runtime presentation seam because the Crystal physical lights do
not read `assets/config/lighting.ron`.

The file pins three one-factor candidates:

| Candidate | Exact presentation change |
|---|---|
| `i01-crystal-tight` | Set every generated crystal point-light range to `3.0` |
| `i02-crystal-broad` | Set every generated crystal point-light range to `7.0` |
| `i03-heart-feature-shadow` | Enable shadow maps only for the Crystal heart light at local offset level `18`; retain disabled contact shadows |

Only those three profiles emit `HEX_REVIEW_CRYSTAL_LIGHT_PROFILE`; the baseline and every
other axis omit it and therefore retain the fixed shipped rig. The runtime parser accepts
only the three canonical IDs. It applies the selection while physical point-light
children are published and does not change authoritative `GameplayLight`, illumination
tiers, LOS, saves, map fingerprints, or networking. The spec, selected target, overrides,
and fully resolved target state are recorded in the plan, profile manifests, and capture
sidecars.

## Validate and inspect the plan

Validation reads the strict JSON schemas and the referenced sources. It rejects unknown
fields, unsafe paths, incomplete palettes, unknown or repeated tactical-fog modes, mixed
experiment axes, a non-static overcast file, drift from the canonical Grand
scenario/world/lighting/palette paths or seed, and drift from the shipped `0.35` baseline.
It writes nothing and does not invoke Cargo:

```sh
python3 tools/visual_experiments.py validate
```

Dry run also records exact Git/worktree provenance and prints the capture commands and
sanitized environment. It creates neither an asset stage nor an output directory:

```sh
python3 tools/visual_experiments.py run --dry-run

python3 tools/visual_experiments.py run \
  --profile h01-flat-030 \
  --profile h02-tall-055 \
  --capture-set smoke \
  --dry-run
```

Run the focused tool tests without building the game:

```sh
python3 -m unittest \
  tools/test_visual_experiments.py \
  tools/test_visual_experiment_sweeps.py \
  -v
```

## Capture a pack

Run all twenty-four profiles:

```sh
python3 tools/visual_experiments.py run
```

Or run an explicit subset:

```sh
python3 tools/visual_experiments.py run \
  --profile v02-fog-dimmed \
  --profile v04-fog-softened \
  --capture-set screen
```

`--profile` and `--profile-set` are mutually exclusive. Candidate-only explicit subsets
still add `e00-baseline` automatically.

Any candidate-only subset automatically adds `e00-baseline`. The published HTML must
always show the baseline immediately beside every candidate capture; the tool therefore
never publishes a candidate-only comparison pack.

While Grand V3 is temporarily blocked behind an unfinished structural validator, a
diagnostic-only run may explicitly add `--allow-structural-draft`. The runner otherwise
scrubs `HEX_GRAND_V3_STRUCTURAL_REVIEW_DRAFT` from its inherited environment, so a shell
cannot accidentally weaken a capture. Opt-in draft state is recorded in the plan, root
manifest, review-binary record, every tokenized capture environment, and both indexes;
the latter are visibly marked `UNAPPROVABLE STRUCTURAL DRAFT`. Such a pack may guide a
geometry repair but is never approval evidence and must be recaptured without the flag.

The default destination is beneath:

```text
.context/grand-v3-visual-experiments/<full-git-head>/<clean-or-dirty-digest>/seed-1592598566/
```

An explicit `--output` must remain below that experiment root. Existing destinations
are refused; there is no overwrite, resume, or partial-success mode. Each capture has a
30-minute default timeout, adjustable with `--timeout-seconds`. A run additionally has
an eight-hour default whole-matrix deadline (hard maximum twelve hours), an 8-GiB
unpublished-work cap (hard maximum 32 GiB), and a 20-GiB free-space reserve. Override
those bounded policies with `--total-timeout-seconds`, `--max-work-gib`, and
`--min-free-gib`; the harness checks them before the build, around asset staging, and
between captures. Exceeding any policy removes the unpublished stage and publishes
nothing.

The runner builds the release-shaped review executable exactly once:

```text
cargo build --release -p hex_game --features map-review --message-format=json-render-diagnostics
```

Cargo's artifact message must identify exactly one regular `hex_game` executable. The
harness records its SHA-256, invokes that exact path for every capture with an explicit
private `BEVY_ASSET_ROOT`, and re-hashes it before publication. Sidecars tokenize the
path as `$REVIEW_BINARY`; absolute build paths never enter the semantic pack.

Because every matrix entry supplies `HEX_REVIEW_CAPTURE`, the game uses its windowless
schedule runner and renders directly into the capture target. Routine matrices therefore
must not create, focus, or activate a native game window. The runner has no interactive
fallback; if a native window appears, stop the batch and treat its evidence as failed even
if PNG generation completes. A `map-review` launch without `HEX_REVIEW_CAPTURE` remains an
ordinary visible manual-play session. Visible gameplay and native motion review happen only
after an explicit human request.

It launches one fresh process group per capture. It removes inherited `HEX_*`, `BEVY_*`,
`WGPU_*`, `RUSTFLAGS`, compiler-wrapper, Cargo-profile, and target-specific overrides,
fixes liquid phase to `0.0`, supplies the exact camera/view/anchor, and assigns a unique
`HEX_GAME_DATA_DIR`. A timeout terminates and, if necessary, kills the complete Cargo/game
process group before staged files are removed.

## Isolation and publication

For each profile, the runner creates a fresh real copy of all `assets/` in a hidden
sibling work directory under `.context`. It rejects symlinks and special files, proves
that no staged file shares a device/inode pair with its source, applies only the expected
file mutation, verifies the complete source/stage hash diff, and makes the staged tree
read-only before launch.

The allowlist is intentionally narrow:

- level-height profiles change only the one `level_height` field in the staged Grand V3
  world;
- palette profiles change only RGB tuples in staged `art/palette.ron`;
- overcast adds the shipped static-lighting path only to the staged Grand V3 scenario;
- time profiles change no staged asset.

Staged assets and local game data are removed after each profile and are never included
in the review pack. Before parsing, after parsing, and immediately before publication,
the runner re-hashes the tracked asset tree, every experiment source, the Git head, and
the worktree content. A concurrent source change fails or retries before any capture can
claim mixed provenance.

The final directory is assembled beside its destination with an `INCOMPLETE — NOT
REVIEWABLE` marker. Only after every process exits successfully, every PNG is exactly
1920×1080, every PNG hash matches its sidecar, and all expected profile/capture pairs
exist does the runner replace the marker and perform an atomic no-replace directory
rename. PNG validation walks every chunk, verifies CRCs, decompresses the complete image,
and requires a canonical end marker; a header-only or truncated file cannot publish. Pack
validation also requires the exact profile/capture/log/sidecar file set and checks every
sidecar identity, capture axis, command, environment, profile state, and provenance
projection against the root manifest. The publication operation uses
`renamex_np(RENAME_EXCL)` on macOS and
`renameat2(RENAME_NOREPLACE)` on Linux; unsupported platforms fail closed. The final
path never exists after a failed run.

## Pack contents and provenance

Each successful pack contains:

```text
manifest.json
review-index.md
index.html
logs/build.log
profiles/<profile>/profile.json
profiles/<profile>/<capture>.png
profiles/<profile>/<capture>.manifest.json
profiles/<profile>/logs/<capture>.log
```

Semantic JSON files are canonical and omit timestamps, process IDs, timings, machine
names, absolute staging paths, and GPU identity. Each capture sidecar records:

- exact Git head, dirty-state flag, and deterministic worktree-content digest;
- hashes for the tool, registry, palette candidates, and relevant shipped inputs;
- scenario, seed, profile axis, resolved tactical-fog mode, material treatment, selected
  Crystal-light profile, level height, lighting source/time, and palette source;
- the exact staged-file allowlist with before/after hashes and the staged asset-tree hash;
- camera, view, focus/look-at framing, cutaway, illumination overlay, liquid phase,
  tokenized exact-binary command, and sanitized environment;
- PNG dimensions, relative path, and SHA-256.

The manifest and sidecars carry a strict `NOT-EMITTED` typed-runtime-report placeholder.
It makes the missing authority explicit and reserves a validated seam for a future Rust
map-review report without pretending that logs or pixels prove runtime facts.

The harness deliberately does **not** claim resolved runtime lighting, accepted art
catalog fingerprints, map fingerprints, or world-snapshot fingerprints. The current
`map-review` hook does not emit those typed values. A future Rust-side review report
would be required before they could truthfully enter the sidecars.

`index.html` starts with capture-first comparisons (the same view across every selected
profile), then provides axis-first sections for the baseline, lighting, haze,
indoor-lighting, visibility, materials, height, and palette. Every caption includes the
resolved tactical-fog mode, material treatment, and Crystal-light profile. Within every
candidate section, each view is a two-column pair with the baseline
on the left and candidate on the right. This keeps framing regressions visible and avoids
forcing reviewers to compare separated grids from memory. The Markdown index remains the
per-frame PASS/FAIL worksheet.

## Interpreting the axes

- Clear time changes both physical presentation and authoritative exterior illumination:
  Moon is gameplay Dim and Sun is gameplay Bright. Midnight therefore also changes fog
  and perception; it is not merely a color-grade comparison.
- Static overcast is supported without a runtime override by changing only the private
  staged scenario. It must not receive `HEX_REVIEW_TIME`.
- Tactical visibility is a separate review-only axis from atmospheric haze. Its four
  candidates emit exactly one of `none`, `dimmed`, `observed-only`, or `softened` through
  `HEX_REVIEW_FOG`, while the otherwise identical noon baseline explicitly emits
  `current`. The hook changes terrain-shroud presentation only; hostile concealment
  remains authoritative in every mode. The registry rejects unknown modes, `current` as
  a candidate, duplicate mode coverage, and profiles that mix visibility with lighting,
  haze, height, or palette changes.
- Material response is a review-only presentation axis selected through
  `HEX_REVIEW_MATERIAL`. `matte-terrain` forces terrain roughness to `1.0` while
  preserving authored objects; `unified-matte` applies the same roughness to both.
  Colors, emissive output, alpha, liquids, gameplay state, saves, and fingerprints are
  unchanged. The checked-in registry emits `current` explicitly for every other axis so
  baseline rendering is unchanged and inherited developer state cannot contaminate a
  comparison.
- Edge response is a review-only presentation axis selected through `HEX_REVIEW_EDGE`.
  `micro-bevel-004` and `micro-bevel-008` alter vertex normals only, using exact blend
  weights `0.04` and `0.08` for terrain and authored objects. They do not add geometry,
  change positions or indices, alter picking/collision, or enter saves and fingerprints.
  The checked-in baseline explicitly emits `current`, which preserves the existing hard
  normals byte-for-byte.
- Indoor Crystal lighting is a review-only presentation axis selected through
  `HEX_REVIEW_CRYSTAL_LIGHT_PROFILE`. The environment key is emitted only for the three
  active indoor candidates; all other profiles omit it and use the shipped rig. The tight
  and broad candidates change only point-light range. The feature-shadow candidate enables
  shadow maps only on the heart light at local offset level `18`; contact shadows remain
  disabled. These physical-light changes do not alter gameplay illumination.
- A `level_height` change requires regeneration. Horizontal hex circumradius remains the
  fixed `1.0`; the experiment is the ratio `level_height / radius`.
- Player scale and Character/First-Person focus heights remain fixed world-unit values.
  Height profiles therefore expose shipped-character compatibility as well as terrain
  proportion. Do not silently rescale the player or camera and call it the same factor.
- Palette changes require a fresh world. Live palette reload can update authored objects
  while existing terrain and units retain prior colors, producing a mixed invalid review.
- The candidate palette covers world art. Sky, fog/shroud, UI, and some VFX use separate
  colors and are not part of this axis.

## Required visual review

A mechanically complete pack starts every still as `UNREVIEWED`. Follow
`.agents/skills/inspect-game-renders/SKILL.md`: inspect every PNG at original resolution,
then scan `index.html` for systematic differences and ask a fresh-eyes reviewer to
challenge the verdicts.

Static frames cannot clear motion. Orbit and walk the water, crystals, tall terrain, and
chunk seams in both directions, watching for flicker, z-fighting, popping, holes, floating
features, incomplete footprint rendering, and camera collision. Until that happens the
pack remains `HUMAN-MOTION-PENDING`.
