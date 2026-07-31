## What and why

<!-- The diff says what changed. Say why. -->

## Checks

- [ ] All relative links in tracked Markdown resolve
- [ ] `cargo fmt --all --check` (unless Markdown-only)
- [ ] `cargo deny check` (unless Markdown-only)
- [ ] `cargo clippy --workspace --all-targets --all-features --profile ci -- -D warnings` (unless Markdown-only)
- [ ] Gameplay rules, contracts, simulation, and app partitions (report each concern below)
- [ ] Residual workspace tests and doctests (unless Markdown-only)
- [ ] `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features` (unless Markdown-only)
- [ ] `cargo build --workspace --profile ci` (unless Markdown-only; CI builds all three platforms)
- [ ] Automated visual walk green — `/visual-walk` captured the screens and the agent read every frame

### Manual runtime sign-off

<!--
Gameplay runtime changes may be marked ready only after a named human runs the
release-shaped build at the exact final PR head. Any later push invalidates this
sign-off. Agent-operated native review and visual-walk evidence are useful but do not
replace the named human gate. Source lanes targeting wave/* defer this evidence to
the combined wave PR into dev; do not copy a sign-off from a lane or older wave head.
-->

- Agent-operated native review:

Manual runtime result: <PASS or BLOCKED>
Manual runtime commit: <full 40-character PR head SHA>
Manual runtime reviewer: <named human>
Manual runtime date: <YYYY-MM-DD>
Manual runtime route: <affected scenarios and failure paths exercised>
Manual runtime findings/waiver: <none, findings, or explicit maintainer waiver>

### Evidence by concern

- Pure rules:
- ECS contracts:
- Deterministic simulation:
- Headless game/UI:
- Visual smoke (presentation only; reviewed gameplay frames, maximum 10):
- Scheduled/manual soak or performance (when applicable):

<!--
Screenshots are not evidence of exact occupancy, action accounting, tempo,
determinism, report identity, or state restoration. Cite canonical snapshots and
named metrics for those claims. A timeout or no-progress bound is a typed result,
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

If the change touches rendering, movement, persistence, or state transitions, walk it:
splash -> title -> New Game -> Party Trial, orbit, move the party, ESC to pause,
save with F5, BACKSPACE to the title, Continue, then open Settings and persist one
change across restart. Launch an affected Map or focused Demo separately.
-->

## Boundaries

- [ ] I stayed inside my crate, **or** I have said below why a shared crate needed changing

<!--
`hex_core` and `hex_game` are shared. Changing them is fine and sometimes
necessary — it is worth a sentence so the people who depend on them know.
-->
