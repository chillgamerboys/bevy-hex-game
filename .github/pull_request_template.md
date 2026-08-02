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
- [ ] Structural UI oracle and scoped Bevy image-target walk green, or N/A because no app/UI/rendered runtime concern is selected

### Manual runtime sign-off

<!--
Gameplay runtime changes may be marked ready only after a named human runs the
release-shaped build at the exact final PR head. Any later push invalidates this
sign-off. Agent-operated Bevy frame review and visual-walk evidence are useful but do not
replace the named human gate. Source lanes targeting wave/* defer this evidence to
the combined wave PR into dev; do not copy a sign-off from a lane or older wave head.
-->

- Agent-operated Bevy visual review:

Manual runtime result: <PASS, BLOCKED, or N/A>
Manual runtime commit: <full 40-character PR head SHA or N/A>
Manual runtime reviewer: <named human or N/A>
Manual runtime date: <YYYY-MM-DD or N/A>
Manual runtime route: <affected scenarios and failure paths exercised, or why N/A>
Manual runtime findings/waiver: <none, findings, explicit maintainer waiver, or N/A>

<!--
For UI work, record all applicable checkpoints explicitly: 1280x720, 1920x1080,
and 3840x2160 under Auto and 200% scale; Main Menu; all three Campaign card states;
Sandbox Overview, Map Browser/Detail, Party, Enemies, Character Picker, deployment,
minimal outcome and Retry Exact; Tools and typed Creator returns; restart persistence;
movement, casting, Channel, blocked actions, required lattice decision, HUD hiding,
and pause.
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
gates on the human box, never the automated one.

If the change touches rendering, movement, persistence, or state transitions, walk
it at the exact candidate head: splash -> Main Menu -> Campaign slot 1 -> Party
Trial, orbit, move, pause, save with F5, return to Campaign, Continue slot 1, then
persist one Settings change across restart. Separately traverse Sandbox map selection,
both rosters, deployment, outcome, Retry Exact, Return to Sandbox, and a Tools-origin
Creator return.
-->

## Boundaries

- [ ] I stayed inside my crate, **or** I have said below why a shared crate needed changing

<!--
`hex_core` and `hex_game` are shared. Changing them is fine and sometimes
necessary — it is worth a sentence so the people who depend on them know.
-->
