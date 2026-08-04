---
name: visual-walk
description: "Run and inspect the scoped Bevy image-target walk for affected static presentation. Structural and mechanical failures always fail; UI review findings block UI work. Never use frames as gameplay or world-logic evidence."
---

# Visual walk

Run this only when the diff affects UI, camera framing/occlusion, rendered-map
presentation, visual scripts, or another static presentation surface. Logic-only work
returns `not_applicable` with its typed hook closure.

## Evidence boundary

- Frames may judge layout, hierarchy, clipping, focus, labels, contrast, reflow,
  camera framing, visible geometry, materials, lighting, seams, and composition.
- Video or a named human may judge motion, native input, animation, control feel, and
  taste.
- Neither may prove or corroborate legality, occupancy, payment, damage, persistence,
  determinism, launch identity, or any other gameplay/world transition. Add a typed
  hook if that oracle is missing.

## Run

1. Ensure no operator-owned game process is running.
2. Build the release-shaped local walk harness:

   ```sh
   cargo build -p hex_game --features visual-walk --profile ci
   ```

3. Create a fresh output directory under `.context/visual-walks/` for the exact PR
   head. Run only the scripts relevant to the changed presentation, for example:

   ```sh
   HEX_WALK_SCRIPT=walks/gameplay_ui.ron \
   HEX_WALK_OUT=.context/visual-walks/pr-<N>-<short-sha> \
   cargo run -p hex_game --features visual-walk --profile ci
   ```

   Never point canonical review at the operator's normal application-data root. A
   nonzero exit, stalled typed step, rejected structural oracle, or black/missing
   rendered surface fails immediately.

4. Open every accepted PNG in script order. For each frame record `ok` or
   `{step, png_path, check, message}`, where `check` is `mechanical` or `review`.
   Do not inspect a frame rejected by the structural oracle.

## Verdict

- Structural or mechanical finding: `fail`.
- Review finding on an affected presentation surface: `fail`.
- No findings: `pass`.
- No affected static presentation: `not_applicable`, with authoritative hooks named.

Report the exact head, scripts, output directory, frame count, and per-frame verdict.
The applicable structured human sign-off in the PR template remains separate and is
invalidated by any later commit.
