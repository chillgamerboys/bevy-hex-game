---
name: visual-walk
description: Run the scripted visual walk — build with the visual-walk feature, drive the game through its screens from walks/*.ron, capture PNGs, then READ every frame and judge it. Structural and mechanical failures always fail; usability findings also block UI/presentation PRs. Step 2.5 of /audit-pr, receipt key 5_visual_walk. Local-only — CI has no GPU.
---

When invoked, follow these steps. The point of this skill is the part no
other gate does: **an agent actually looks at the game.**

## What this skill can and cannot judge

Two tiers, and the split is the contract:

- **Structural tier (hard fail):** before every capture, the live UI tree checks
  effective bounds, inherited clipping, scroll reachability, accessible labels,
  focus order, interactive overlap, and 44×44 targets. A nonzero node inside a
  clipped ancestor is not visible. Structural findings block capture and merge.
- **Mechanical tier (hard fail):** the walk stalls, a capture comes back black,
  the wrong screen is visible, a panel is entirely missing, or text renders as
  nothing. These block the merge exactly like a failing test.
- **Review tier:** hierarchy, cramped or dead space, visual contrast, and
  inconsistent styling that no geometry oracle can judge remain human-readable
  findings. They block UI/presentation PRs; other runtime changes may record them
  as warnings for the human.
- **Not covered:** motion (movement speed, animation feel), sub-pixel seams,
  and final taste. The human walk owns those — this skill narrows the
  human's job, it does not replace it.

## Step 0 — Applicability

Diffs with no runtime surface (docs, CI, pure data with no renderer path)
skip the walk: report `skipped — no runtime surface` and stop. The trigger
rule is the same one audit-diff uses for its visual flag: rendering, UI,
transforms, movement, screen/state transitions → walk.

## Step 1 — Build and run the walks

```bash
cargo build -p hex_game --features visual-walk --profile ci
OUT=.context/visual-walks/pr-<N>   # or /tmp for uncommitted work
HEX_WALK_SCRIPT=walks/gameplay_ui.ron HEX_WALK_OUT=$OUT cargo run -p hex_game --features visual-walk
```

The runner automatically assigns every configured walk a fresh disposable
application-data root beneath `OUT`, before preferences or resume data load. An
explicit `HEX_GAME_DATA_DIR` remains available for narrowly controlled fixtures,
but a canonical review must never point it at the operator's normal data root.

Run the scripts relevant to the diff. Gameplay UI uses the one bounded Bevy
image-target route; map/world routes retain their owned scripts and acceptance
criteria. Each run
must exit 0 — a nonzero exit is a mechanical `fail`; the process's own log
names the stalled step or black frame. NEVER run while the operator has a
game instance open (the two windows fight for nothing, but the operator's
session must not be disturbed — check first).

A walk drives the REAL binary through the REAL wiring: clicks are injected
`Interaction::Pressed` on named buttons, keys go through `ButtonInput`, and
scenario launches use the same bypass as map-review. If a script's click
target was renamed, fix the script in the same PR — scripts are part of the
UI's contract now.

## Step 2 — Read every frame

Open each PNG with the Read tool, in script order, and judge it against the
step's intent (the script comments say what each frame should show). For
each frame record: `ok`, or a finding
`{step, png_path, check: mechanical|review, message}`.

Do not review a PNG the structural oracle rejected. Checklist per accepted frame —
the known silent failure modes first (blue window,
black sky from the down-looking camera, missing HUD, pause overlay absent),
then the review tier (alignment, overflow, contrast, spacing, hierarchy).
Say what is GOOD too when it is — the operator uses this to calibrate trust.

## Step 3 — Report and receipt entry

Print a table: frame × verdict × finding. Then the receipt entry for
`/audit-pr` (this skill is its Step 2.5, key `5_visual_walk`):

- All frames ok → `"status": "pass"`, summary like
  `"10 Bevy image-target frames, 0 structural, 0 mechanical, 0 review findings"`.
- A review finding in a UI/presentation diff → `"status": "fail"` + the findings
  array. An ordinary non-UI runtime diff may report review findings as `warn`, but
  they never become green evidence for a UI change.
- Any mechanical failure → `"status": "fail"` + findings. This blocks
  `/merge-pr` like any failing step.

Findings shape: `{step, png_path, check, message}`.

The receipt entry also records `"review_policy": "blocking"` for UI/presentation
diffs, `"advisory"` for other runtime diffs, or `"not_applicable"` when skipped.
`warn` is valid only with the advisory policy; a blocking-policy review finding is
always `fail`.

## When NOT to invoke

- **In GitHub CI** — runners have no GPU; this is a local gate on the dev
  machine, like the rest of the audit chain's heavy steps.
- **Doc-only / no-runtime diffs** — that's Step 0's `skipped`.
- **As a substitute for the human walk.** The PR template keeps two boxes:
  the automated walk (this skill, auto-tickable from the receipt) and the
  human's "I ran the game and looked at it" — `/promote` still gates on the
  human one, always.

## Self-updating

- New screen or interaction worth photographing → extend `walks/*.ron` (and
  add a step comment saying what the frame should show).
- New mechanical or geometry check → add to the
  harness (`crates/hex_game/src/walk.rs`) first, then note it here.
- If the walk scripts' click names drift from the UI, the walk fails loudly —
  fix the script with the UI change, never delete the step.
