# Faster walking and direct spell controls — 2026-09-11

Local continuation from `68df079` on `fix/arena-golem-worm-response`.

## Changes

Human and Shadow move at 4.5 world units/second for walking and run requests.
Shift has no gameplay speed effect. Other creatures retain their movement
profiles, and spectator Shift-fast camera controls remain unchanged. No stamina.

Hold/release LMB for Fireball or RMB for Shield. The first button pressed owns
the gesture through release, including a press rejected by cooldown. The other
button must be released and freshly pressed to start another gesture. Ordered
native events preserve first-press order; unordered snapshots prefer Fireball.
A bounded private queue carries spell-tagged press/release edges into successive
120 Hz ticks, preserving rapid taps and release-time aim. Actor charging and
cooldown authority are unchanged.

E activates High Jump without cancelling charge; Space remains ordinary jump.
The old 1/2/3 player bindings and Shift sprint input are removed. Pause, focus
loss, reset and death clear pending gestures. After menu transitions, both mouse
buttons must be released before fresh gameplay gestures are admitted.

T follows the current gesture, otherwise the last-used spell, initially Fireball.
Shield assistance starts on and Fireball assistance starts off. The HUD labels
both mouse buttons and E, highlighting only an authoritative active charge.

## Validation

Arena authority: 327 tests passed, including cardinal/diagonal movement, identical
walk/run requests, preserved creature overrides, observation forecasting, normal
jump exits, High Jump escape/recovery, and the existing combat regressions.
Application compilation, strict arena lint (all targets/features, warnings denied),
workspace formatting and whitespace checks passed.

Application gate: **107 passed, zero failed, two existing long-running tests
ignored**. This includes the eight direct-control fixtures, quick clicks between
physics ticks, release-time aim, first/third-person casting, E during charge and
release, cooldown admission, pause/focus/reset/death cancellation, menu mouse
quarantine, removed number keys and Shift speed, assistance toggling, computed HUD
and menu layout, Fort/Duel map regressions and spectator input. The existing short
paired-bot smoke check passed for the current Shadow; it does not establish human
win rates. Native binary execution and visual inspection were not repeated.

Commands use the existing shared Cargo target with one job and incremental
compilation disabled:

```text
cargo test -p hex_arena --lib -- --nocapture
cargo check -p hex_game --lib --features test-support
cargo test -p hex_game --lib --features test-support arena:: -- --nocapture
cargo clippy -p hex_arena --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
```

Two historical test assumptions needed correction. The render-rate walk fixture
now clears bootstrap remainder and supplies exactly one second at each cadence;
it requires 120 ticks, one cast, and matching positions within 0.001 units. Its
previous nominal frame durations differed by one or two simulation ticks.

The frozen comparison bot from `8d20e13` walks behind cover before firing in one
10-second stationary-target scenario at the new speed (121 visible ticks, no
casts). This remains a recorded comparison outcome, rather than a requirement on
that historical brain. The shipped Shadow retains the assertion that it releases
against every stationary-target seed; runtime behavior was not relaxed.

Native playtesting remains necessary for the faster walking, mouse gesture feel,
and practical Shadow crater coverage. Windowless checks do not establish visual
quality or subjective difficulty. No balance tournament, remote publication or
merge is part of this change.

Usage checks remained at **19% remaining** at completion, above the requested
16% checkpoint threshold. This delivery is committed locally only.
