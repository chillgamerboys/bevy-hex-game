## What and why

<!-- The diff says what changed. Say why. -->

## Checks

- [ ] All relative links in tracked Markdown resolve
- [ ] `cargo fmt --all --check` (unless Markdown-only)
- [ ] `cargo deny check` (unless Markdown-only)
- [ ] `cargo clippy --workspace --all-targets --all-features --profile ci -- -D warnings` (unless Markdown-only)
- [ ] `python3 tools/test_scope.py plan --base origin/dev --head HEAD` recorded; every selected test concern passed
- [ ] Residual workspace tests and doctests (only when the scope decision selects `residual`)
- [ ] `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features` (unless Markdown-only)
- [ ] `cargo build --package hex_game --release` (when the scope decision selects `shipping`; CI builds it on all three platforms)
- [ ] Structural UI oracle and scoped Bevy image-target walk green, or exact-head N/A because the reviewed diff has no rendered runtime concern

### Manual runtime sign-off

<!--
Gameplay runtime-surface changes may be marked ready only after a named human runs the
release-shaped build at the exact final PR head. A named maintainer may instead record
an exact-head N/A waiver when the reviewed change has no rendered presentation,
navigation, movement, persistence, or visual-script surface. Any later push invalidates
either result. Agent-operated Bevy frame review and visual-walk evidence are useful but
do not replace the named human gate. Source lanes targeting wave/* defer this evidence
to the combined wave PR into dev; do not copy evidence from a lane or older wave head.
-->

- Agent-operated Bevy visual review:

Manual runtime result: <PASS, BLOCKED, or N/A>
Manual runtime commit: <full 40-character PR head SHA for PASS/waiver, or N/A only when no runtime path changed>
Manual runtime reviewer: <named human for PASS; @maintainer-login for waiver; N/A only when no runtime path changed>
Manual runtime date: <YYYY-MM-DD for PASS/waiver, or N/A only when no runtime path changed>
Manual runtime route: <affected route for PASS; exact non-rendered reason for waiver; N/A only when no runtime path changed>
Manual runtime findings/waiver: <findings or explicit maintainer waiver; N/A only when no runtime path changed>

<!--
For UI work, record all applicable checkpoints explicitly: 1280x720, 1920x1080,
and 3840x2160 under Auto and 200% scale; Main Menu; all three Campaign card states;
Sandbox Overview, Map Browser/Detail, Party, Enemies, Character Picker, deployment,
minimal outcome and Retry Exact; Tools and typed Creator returns; restart persistence;
movement, casting, Channel, blocked actions, and pause. HUD work additionally records
every default component and Main View shortcut, a custom visibility combination,
master-hidden one-surface summons, first/repeated Party and disclosed-Initiative
inspection in Map and Character camera modes, a blocking required decision,
deployment/outcome suppression, Compact map-only presentation, one keybinding conflict
resolved through Swap, and restart persistence of HUD preferences and keyboard
overrides. Confirm that hidden components leave no drawer, handle, tooltip, focusable
control, or hit region behind.
-->

### Evidence by concern

<!-- Record omitted concerns as N/A with the selector reason; do not run them merely to fill this list. -->

- Pure rules:
- ECS contracts:
- Deterministic simulation:
- Headless game/UI:
- Visual smoke (presentation only; reviewed gameplay frames, maximum 10):
- Scheduled/manual soak or performance (when applicable):

<!--
Screenshots are UI evidence only: layout, hierarchy, legibility, focus visibility,
contrast, responsive reflow, and presentation state. They are not evidence of
legality, budgets, decisions, damage, Channel, outcomes, persistence, deployment,
launch/retry identity, exact occupancy, tempo, or determinism. Cite canonical snapshots
and named metrics for those claims. A timeout or no-progress bound is a typed result,
not a pass.
-->

<!--
Those last two are different gates, not a formality and not each other's
substitute. Several failure modes here produce a clean log and a wrong window:
missing assets render a plain blue screen, a missed skybox event renders a black
sky, a wrong speed unit just looks slightly off, and a tile whose transform
disagrees with its span floats or sinks. All of them pass CI. The automated walk
(receipt key 5_visual_walk) catches the renders-nothing/renders-broken class and
lists layout findings; motion, feel, and taste still need human eyes — /promote
gates on the structured exact-head human PASS fields, never the automated tier.

If the change touches rendering, movement, persistence, or state transitions, walk
it at the exact candidate head: splash -> Main Menu -> Campaign slot 1 -> Party
Trial, orbit, move, pause, save with F5, return to Campaign, Continue slot 1, then
persist one volume, one HUD visibility preference, and one keyboard override across
restart. Separately traverse Sandbox map selection, both rosters, deployment, outcome,
Retry Exact, Return to Sandbox, and a Tools-origin Creator return. For HUD changes,
exercise the complete HUD checkpoint list above rather than treating a single hide/show
toggle as sign-off.
-->

## Boundaries

- [ ] I stayed inside my crate, **or** I have said below why a shared crate needed changing

<!--
`hex_core` and `hex_game` are shared. Changing them is fine and sometimes
necessary — it is worth a sentence so the people who depend on them know.
-->
