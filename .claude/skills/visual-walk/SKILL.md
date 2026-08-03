---
name: visual-walk
description: Run the presentation-only scripted visual walk — build with the visual-walk feature, drive the game through its screens from walks/*.ron, capture PNGs, then READ every frame and judge rendered output only. Structural and mechanical presentation failures always fail; usability findings also block UI/presentation PRs. Step 2.5 of /audit-pr, receipt key 5_visual_walk. Local-only — CI has no GPU.
---

When invoked, follow these steps. The point of this skill is the part no
other gate does: **an agent actually looks at rendered presentation.**

## What this skill can and cannot judge

Two tiers, and the split is the contract:

Screenshots and rendered frames may judge static camera framing/occlusion, UI
hierarchy/layout/legibility/focus/contrast/reflow, and rendered-map geometry,
materials, lighting, cutaways, visible seams, and composition. Video and human checks
may judge camera motion, native-input response, animation, control feel, and taste. A
visual artifact may show how hook-established state is rendered, but screenshots,
frames, video, and human observation never prove, corroborate, or strengthen
gameplay/world logic that typed hooks, state, messages, logs, snapshots, or
deterministic contracts can express. If the needed hook is missing, add it instead of
inferring state from pixels or frame timing.

- **Structural tier (hard fail):** before every capture, the live UI tree checks
  effective bounds, inherited clipping, scroll reachability, accessible labels,
  focus order, interactive overlap, and 44×44 targets. A nonzero node inside a
  clipped ancestor is not visible. Structural findings block capture and merge.
- **Mechanical tier (hard fail):** a typed walk step stalls, a capture comes back
  black, the wrong rendered surface is visible, a panel is entirely missing, or text
  renders as nothing. These are presentation failures and block the merge exactly like
  a failing presentation test; they do not establish underlying gameplay state.
- **Review tier:** hierarchy, cramped or dead space, visual contrast, and
  inconsistent styling that no geometry oracle can judge remain human-readable
  findings. They block UI/presentation PRs; other runtime changes may record them
  as warnings for the human.
- **Not covered by frames:** any gameplay/world logic, plus camera/movement/animation
  motion, control feel, native-input response, and final taste. Typed hooks own logic;
  video and the human walk own the experiential remainder. This skill narrows the
  human's presentation job; it does not replace it.

## Step 0 — Applicability

Diffs with no affected presentation or experiential surface skip the walk: report
`skipped — no affected presentation surface` and stop, even when renderer-free runtime
logic changed. Trigger on rendering, UI presentation, native-input experience, motion,
seams, or visual scripts—not on a gameplay/world state transition already proved by
hooks.

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

Each `Capture` owns a fresh shared 3D/UI image target and waits four complete render
frames before screenshotting. The tooling UI camera tracks the 3D camera's MSAA so
OIT tree fading cannot leave world pixels stale while UI pixels continue updating.
Identical frames across movement or orbit are therefore a rendered-response failure
unless the script explicitly establishes that no visible output should change. Typed
state assertions, never image differences, prove whether movement or orbit occurred.

A walk drives the real binary through real wiring: named UI clicks are injected
as `Interaction::Pressed`; `ClickTile` emits the ordinary primary `Pointer<Click>`
after resolving one exact exposed surface; `ClickAnchor` first checks the published
anchor against its authored exact position and then emits that same pointer click;
`OrbitCamera` uses bounded cursor messages while the ordinary right mouse button is
held; keys go through `ButtonInput`; and scenario launches use the same bypass as
map-review. Route scripts may not mutate camera/unit state, teleport, suppress combat,
or fake flight reachability. Typed DSL assertions and exit status may prove route/state
facts; the captures cannot. If a script's click target or anchor moved, fix and
re-review the script in the same PR — scripts are part of the UI's contract now.

## Step 2 — Read every frame

Open each PNG with the Read tool, in script order, and judge it only against the
step's presentation intent (the script comments say what each frame should render).
Comments must not ask a frame to prove gameplay/world logic; add a typed assertion for
that claim. For
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

- All frames visually ok → presentation-only `"status": "pass"`, summary like
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
- **No affected presentation/experiential surface** — that's Step 0's `skipped`, even
  for a renderer-free runtime-logic change.
- **As gameplay/world logic evidence.** Use or add typed hooks/contracts instead.
- **As a substitute for applicable human presentation review.** The PR template keeps
  automated visual evidence separate from structured exact-head human presentation
  evidence. A logic-only feature or wave may carry a verified-maintainer N/A naming
  its hooks; `/promote` still gates on the human presentation PASS, always.

## Self-updating

- New screen or interaction with presentation worth photographing → extend
  `walks/*.ron` (and add a step comment saying what the frame should render).
- New mechanical or geometry check → add to the
  harness (`crates/hex_game/src/walk.rs`) first, then note it here.
- If the walk scripts' click names drift from the UI, the walk fails loudly —
  fix the script with the UI change, never delete the step.
