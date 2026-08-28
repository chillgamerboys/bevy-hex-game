# Grand V3 visual experiments

`tools/visual_experiments.py` produces a deterministic, review-only comparison pack for
lighting, voxel aspect, and world-palette experiments. It never edits `assets/`, never
adds a candidate to a shipped scenario, and never publishes an incomplete pack.

This is presentation evidence, not a world-logic oracle. Typed map tests remain the
authority for topology, heights, routes, illumination tiers, occupancy, and fingerprints.
The captures show how an already-established state renders.

## Canonical matrix

The strict registry is `tools/visual_experiments/profiles.json`. It fixes scenario
`Grand V3 Baseline`, seed `1592598566`, a four-view capture matrix, and these nine
one-factor profiles:

| Profile | Only changed axis |
|---|---|
| `e00-baseline` | Shipped palette and level height at clear noon (`12.0`) |
| `l01-midnight` | Clear-cycle time `0.0` |
| `l02-dawn` | Clear-cycle time `6.5` |
| `l03-golden` | Clear-cycle time `16.5` |
| `l04-overcast` | Shipped static `config/lighting/overcast.ron` |
| `h01-flat-030` | `level_height: 0.30` |
| `h02-tall-055` | `level_height: 0.55` |
| `p01-muted-earth` | Complete muted-earth world palette |
| `p02-high-separation` | Complete high-separation world palette |

The two candidate palettes live under `tools/visual_experiments/palettes/`, outside
the shipped asset tree. They contain exactly the shipped swatch vocabulary and only
replace RGB values in the private staged copy.

Every profile captures the same views:

1. the unfocused, full-footprint top-down Map;
2. an oblique Map view focused on `grand_v3.massif`;
3. a rotated Character view at `grand_v3.waterfall_base`;
4. First Person at `grand_v3.tunnel_mouth`.

The matrix intentionally avoids a full cross-product. After a human selects a promising
height or palette, add a separately reviewed interaction matrix rather than changing the
canonical one-factor profiles.

## Validate and inspect the plan

Validation reads the strict JSON schemas and the referenced sources. It rejects unknown
fields, unsafe paths, incomplete palettes, mixed experiment axes, a non-static overcast
file, drift from the canonical Grand scenario/world/lighting/palette paths or seed, and
drift from the shipped `0.4` baseline. It writes nothing and does not invoke Cargo:

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
  --dry-run
```

Run the focused tool tests without building the game:

```sh
python3 -m unittest tools/test_visual_experiments.py -v
```

## Capture a pack

Run all nine profiles:

```sh
python3 tools/visual_experiments.py run
```

Or run an explicit subset:

```sh
python3 tools/visual_experiments.py run \
  --profile e00-baseline \
  --profile p01-muted-earth
```

The default destination is beneath:

```text
.context/grand-v3-visual-experiments/<full-git-head>/<clean-or-dirty-digest>/seed-1592598566/
```

An explicit `--output` must remain below that experiment root. Existing destinations
are refused; there is no overwrite, resume, or partial-success mode. Each capture has a
30-minute default timeout, adjustable with `--timeout-seconds`.

The runner invokes only the source-build review command:

```text
cargo run --release -p hex_game --features map-review
```

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
profiles/<profile>/profile.json
profiles/<profile>/<capture>.png
profiles/<profile>/<capture>.manifest.json
profiles/<profile>/logs/<capture>.log
```

Semantic JSON files are canonical and omit timestamps, process IDs, timings, machine
names, absolute staging paths, and GPU identity. Each capture sidecar records:

- exact Git head, dirty-state flag, and deterministic worktree-content digest;
- hashes for the tool, registry, palette candidates, and relevant shipped inputs;
- scenario, seed, profile axis, level height, lighting source/time, and palette source;
- the exact staged-file allowlist with before/after hashes and the staged asset-tree hash;
- camera, view, focus anchor, liquid phase, command, and sanitized environment;
- PNG dimensions, relative path, and SHA-256.

The harness deliberately does **not** claim resolved runtime lighting, accepted art
catalog fingerprints, map fingerprints, or world-snapshot fingerprints. The current
`map-review` hook does not emit those typed values. A future Rust-side review report
would be required before they could truthfully enter the sidecars.

## Interpreting the axes

- Clear time changes both physical presentation and authoritative exterior illumination:
  Moon is gameplay Dim and Sun is gameplay Bright. Midnight therefore also changes fog
  and perception; it is not merely a color-grade comparison.
- Static overcast is supported without a runtime override by changing only the private
  staged scenario. It must not receive `HEX_REVIEW_TIME`.
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
