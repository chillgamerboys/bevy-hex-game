# High Jump and Shadow reaction experiment — 2026-09-11

Local candidate on `fix/arena-golem-worm-response`, preserving accepted repairs
from `e0b2be9`. This supersedes the unfinished Fireball excavation experiment.
The temporary terrain-damage projection foundation is removed; world mutation
and collision-publication APIs match the accepted build.

## Playable changes

Press **3** for High Jump without selecting a different spell or interrupting a
held Shield/Fireball charge. Default rise is four world units, cooldown seven
seconds. The same boost works during ascent or descent, preserving horizontal
momentum and faster existing ascent. Space remains the ordinary jump. A fresh
press is required; cooldown presses, menu input and held keys cannot queue boosts.
Ceilings block movement and still consume an admitted activation's cooldown.

Esc/Tab opens existing tuning: jump height 2–8 units, jump cooldown 0.5–20 seconds,
Shadow acquisition reaction Off–500 ms (150 ms default), and escape assistance
On/Off. Settings survive round reset and map changes in this application session.
The reaction delay applies after new contact, reacquisition and target changes;
precharging overlaps it. Shield and High Jump remain immediately available.

Shadow recovery tries walking, ordinary jumping, then High Jump toward a supported
landing. Six directions, six-unit radius and two-second controller rollouts bound
the search; at most one candidate is evaluated per tick. Recovery preserves combat
charging and never creates or damages terrain. Failed searches are suppressed
until local terrain changes or the actor leaves that location. Golem's radial
slam remains a separate creature effect.

## Validation

Passed focused arena authority checks: **326 tests**, including shared boost
admission, eight recovery fixtures, acquisition timing, gravity-aware observation,
hidden-information isolation, and prior Golem/Worm regressions. Strict arena lint
passes with all targets/features and warnings denied. Workspace formatting and
whitespace checks pass.

Application checks pass: **99 passed, 2 existing ignored**. Coverage includes
seven High Jump native-input/render-rate tests (30/60/144/480 Hz), reaction menu
Off/150/500 ms and reset/map retention, computed menu layout at 1280×720 and
1600×900, pause/focus/death handling, terrain-publication ordering and Fort/Duel
regressions. The gate also ran its existing paired scripted-bot smoke comparison;
it does not establish human difficulty or win rates.
The native game binary compiled during the application test build; macOS emitted
a large unwind-table linker warning. Native interactive play and visual
inspection are still pending.

Crater fixture maximum recovery-intent time was 0.41 ms for normal jumping and
0.65 ms for High Jump, below the 8.33 ms simulation budget. These are local
synthetic CPU samples, not a full native-frame performance claim. No candidate
exceeded the 240-controller-tick bound. Failed searches retry when jump cooldown
becomes ready or the configured height changes, without repeatedly scanning an
unchanged impossible route.

Commands use the repository's shared Cargo target, one build job and incremental
compilation disabled:

```text
cargo test -p hex_arena --lib -- --nocapture
cargo clippy -p hex_arena --all-targets --all-features -- -D warnings
cargo test -p hex_game --lib --features test-support arena:: -- --nocapture
cargo fmt --all -- --check
git diff --check
```

Three historical full-combat snapshots from `27338de` predated the accepted
gravity-aware aim repair and Area Blast removal. Their artifact remains unchanged.
The harness now checks historical initial spawn/body/HP contracts, exact repeated
current-runtime replay, and stationary, sprinting and covered-precharge behavior.
It no longer claims byte-for-byte equivalence to that historical combat behavior.
Radial damage/publication fixtures now issue explicit effects or real Fireballs;
they no longer depend on a removed player ability.

Usage checks reported **20% remaining** at delivery, above the requested 16%
checkpoint threshold. Code and this report are committed locally; no remote merge
or publication was performed.

## Remaining human checks

Native playtesting should compare reaction Off/150/500 ms, charge-and-boost peeks,
jump height/cooldown, and escapes from irregular craters. Local ledge searches are
bounded and do not guarantee escape from every depression, ceiling or damaged map.
Broad captures, calibration tournaments and repeated full-workspace gates are
outside this usage-limited change. No native visual or subjective balance claim
is made by the windowless application tests.
