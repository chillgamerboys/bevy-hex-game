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
- [ ] Structural UI oracle: green when selected, or selector-N/A when unselected
- [ ] Scoped Bevy image-target walk: green for affected static presentation, or exact-head N/A because the reviewed diff has no affected presentation concern

### Manual runtime sign-off

<!--
A named human PASS is required only when the changed surface includes rendered
presentation, native input, motion, control feel, seams, or taste. A logic-only runtime
change backed by typed hooks, state, messages, logs, snapshots, or deterministic
contracts records an exact-head verified-maintainer N/A instead. Screenshots/frames
judge static camera, UI, and rendered-map presentation; video/human checks judge
motion, input response, control feel, and taste. None proves or corroborates
gameplay/world logic; add a missing hook rather than infer state from pixels. Any later
push invalidates either classification. Source lanes targeting wave/* defer it to the
combined wave PR into dev; do not copy evidence from a lane or older wave head.
-->

- Agent-operated Bevy visual review:

Manual runtime result: <PASS, BLOCKED, or N/A>
Manual runtime commit: <full 40-character PR head SHA for PASS or N/A waiver>
Manual runtime reviewer: <named human for PASS; @maintainer-login for N/A waiver>
Manual runtime date: <YYYY-MM-DD for PASS or N/A waiver>
Manual runtime route: <affected presentation/experiential route for PASS; authoritative hook closure and no-visual reason for N/A>
Manual runtime findings/waiver: <presentation findings for PASS; explicit logic-only maintainer waiver naming hooks/contracts for N/A>

<!--
For UI work, record all applicable checkpoints explicitly: 1280x720, 1920x1080,
and 3840x2160 under Auto and 200% scale; Main Menu; all three Campaign card states;
Sandbox Overview, Map Browser/Detail, Party, Enemies, Character Picker, deployment,
minimal outcome and Retry Exact; Tools and typed Creator returns; post-restart
presentation of hook-proven state;
movement, casting, Channel, blocked actions, and pause. HUD work additionally records
every default component and Main View shortcut, a custom visibility combination,
master-hidden one-surface summons, first/repeated Party and disclosed-Initiative
inspection in Map and Character camera modes, a blocking required decision,
deployment/outcome suppression, Compact map-only presentation, one keybinding conflict
resolved through Swap, and post-restart presentation of hook-proven HUD preferences
and keyboard overrides. Confirm that hidden components leave no drawer, handle,
tooltip, focusable control, or hit region behind. These are presentation and
control-feel checkpoints;
typed hooks and canonical snapshots, never observation of the route, prove every
underlying state transition and persistence claim.
-->

### Evidence by concern

<!-- Record selector-omitted concerns as N/A with the selector reason. Do not run an unselected concern merely to fill this list. -->

- Pure rules:
- ECS contracts:
- Deterministic simulation:
- Headless game/UI:
- Visual smoke (presentation only; reviewed gameplay frames, maximum 10):
- Scheduled/manual soak or performance (when applicable):

<!--
Screenshots and rendered frames prove static presentation: camera framing/occlusion,
UI layout/hierarchy/legibility/focus/contrast/reflow, and rendered-map geometry,
materials, lighting, cutaways, seams, and composition. Video and human checks prove
camera motion, native-input response, animation, control feel, and taste. These may
judge how a hook-established state is rendered, but must never prove, corroborate, or
strengthen gameplay/world logic. Add a missing hook instead of inferring legality,
budgets, decisions, damage, Channel, outcomes, persistence, deployment, launch/retry
identity, exact occupancy, tempo, determinism, or any other state transition from
pixels or frame timing. A timeout or no-progress bound is a typed result, not a pass.
-->

<!--
Those last two are different gates, not a formality and not each other's
substitute. Several presentation failures produce a clean log and a wrong window:
missing assets render a plain blue screen, a missed skybox event renders a black sky,
native text can be unreadable, and motion can feel wrong. The automated walk (receipt
key 5_visual_walk) catches the renders-nothing/renders-broken class and lists layout
findings; motion, feel, and taste still need human eyes — /promote retains its
structured exact-head human presentation PASS gate, never the automated tier.

If the change affects presentation, native input, motion, control feel, seams, or
taste, walk the affected route at the exact candidate head. Exercise the complete HUD
checkpoint list for HUD work rather than treating a single hide/show toggle as visual
sign-off. Do not record route observation as gameplay/world logic evidence; cite the
typed hooks or contracts that prove each underlying state transition separately.
-->

## Boundaries

- [ ] I stayed inside my crate, **or** I have said below why a shared crate needed changing

<!--
`hex_core` and `hex_game` are shared. Changing them is fine and sometimes
necessary — it is worth a sentence so the people who depend on them know.
-->
