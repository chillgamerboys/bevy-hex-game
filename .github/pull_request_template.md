## What and why

<!-- The diff says what changed. Say why. -->

## Checks

- [ ] All relative links in tracked Markdown resolve
- [ ] `cargo fmt --all --check` (unless Markdown-only)
- [ ] `cargo deny check` (unless Markdown-only)
- [ ] `cargo clippy --workspace --all-targets --all-features --profile ci -- -D warnings` (unless Markdown-only)
- [ ] `cargo test --workspace --all-features --profile ci` (unless Markdown-only)
- [ ] `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features` (unless Markdown-only)
- [ ] `cargo build --workspace --profile ci` (unless Markdown-only; CI builds all three platforms)
- [ ] Automated visual walk green — `/visual-walk` captured the screens and the agent read every frame
- [ ] **A human ran the game and looked at it**

<!--
Those last two are different gates, not a formality and not each other's
substitute. Several failure modes here produce a clean log and a wrong window:
missing assets render a plain blue screen, a missed skybox event renders a black
sky, a wrong speed unit just looks slightly off, and a tile whose transform
disagrees with its span floats or sinks. All of them pass CI. The automated walk
(receipt key 5_visual_walk) catches the renders-nothing/renders-broken class and
lists layout findings; motion, feel, and taste still need human eyes — /promote
gates on the human box, never the automated one.

If the change touches rendering, movement, or state transitions, walk it:
splash -> title -> click a scenario -> gameplay, orbit, click a tile, click the tile
you are standing on, ESC to pause and resume, BACKSPACE to the title, click a scenario
to rebuild.
-->

## Boundaries

- [ ] I stayed inside my crate, **or** I have said below why a shared crate needed changing

<!--
`hex_core` and `hex_game` are shared. Changing them is fine and sometimes
necessary — it is worth a sentence so the people who depend on them know.
-->
