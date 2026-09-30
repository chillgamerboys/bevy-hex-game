# Quick combat fixes — 2026-09-11

Local branch `fix/arena-golem-worm-response`, continuing from response repair
`6518fd7`. Scope and usage guard approved by the user: start at 27% remaining,
check before tasks/checks, checkpoint and stop at 16%.

## Changes

- Shadow uses an observation-only human movement-controller forecast, capped at
  .5 seconds. Gravity, landing and ceilings replace constant upward extrapolation.
  Aim, projectile forecasts and splash admission share the path; release rechecks
  it. No speculative second jump, hidden input or changes to other opponents.
- Golem walks at 3.2 units/second in defaults and authored configuration. Its
  previously repaired obstacle swipe and all attacks/HP remain unchanged.
- Worm retracts before trying a full-body escape band up to eight levels deep.
  The existing swept earth-conversion/publication path admits every move. It seeks
  3.5 units of relocation and usable emergence, with at most four seconds of buried
  escape travel. Full-rise preflight avoids blocked emergence; existing directional
  detours gain space when a nearby player obstructs the head. Bedrock/protected
  ground can still force stationary counterfire.

## Focused validation

- Five Shadow jump tests pass: rising/apex/falling/landing, ceiling contact, no
  speculative repeat jump, human-only scope, and aim/forecast/live-sweep agreement.
- The existing identical-observed-history test passes despite divergent hidden
  human positions and velocities.
- 33 Golem tests pass, including actual 3.2-unit motion and previously repaired
  obstacle-clearing, protected terrain and movement through a published opening.
- 49 Worm tests pass: deep head/tail craters, relocation and counterfire, blocked
  roofs/bodies, protected-floor/bedrock refusal, hidden-information isolation,
  no above-ground crawling, and world publication/conversion contracts. The
  configuration test also rejects out-of-budget depth/time and nonfinite distance.
- Strict arena Clippy passes with all targets/features and `-D warnings`.
- Scoped Rust formatting and `git diff --check` pass.
- The strengthened actual-map application test passes on Fort and Duel (.71s).

The actual-map fixture removes four supporting levels beneath the tail after a
real Boulder release. It requires descent, relocation and exposure, then places
the synthetic human on dry, visible footing near the new location and requires
another projectile. Removed terrain must remain absent on both Fort and Duel.
The flat-world head/tail tests keep the human in place throughout recovery.

The first Fort run caught a blocked emergence after relocation; complete rise
preflight now rejects those locations. The second run completed escape/exposure
but did not shoot the original stationary human within 30 seconds. The final
fixture separates lawful counterfire on a visible target from that unresolved
map-routing case; it does not establish reliable reacquisition around Fort cover.
The Duel run also reproduced stopping beneath a player whose body blocked the
rise. The final candidate's existing-direction detour resolved that fixture.
The roof regression was corrected to refresh its published terrain query and
leave clearance above the existing body; its original roof intersected body skin.

Commands used the retained target cache, `CARGO_INCREMENTAL=0` and one build job:

```sh
cargo test -p hex_arena --all-features shadow_jump
cargo test -p hex_arena --all-features identical_observations_ignore_silent_hidden_positions_and_velocities
cargo test -p hex_arena --all-features golem
cargo test -p hex_arena --all-features worm
cargo test -p hex_arena --all-features worm_configuration
cargo test -p hex_game --all-features --test arena_battles player_worm_answers_a_nearby_visible_human_on_each_real_map
cargo clippy -p hex_arena --all-targets --all-features -- -D warnings
```

## Limits and playtest

Controls are unchanged. Launch using `python3 tools/arena.py launch --map fort`
from the repository (or the existing Battle button). Compare repeated jumps against
Shadow, Golem approach speed, and Worm head/tail craters after explosions.

The rigid Worm cannot solve every terrain shape. Deep destruction reaching bedrock,
protected objects and enclosed positions may still prevent escape. Independent
segment movement and general route planning are deferred. Finding a usable firing
angle after escaping remains a native playtest concern, especially on Fort.
No broad captures, balance tournaments or repeated
full-workspace checks were requested or run. Native control feel remains pending.

Final usage check: **25% remaining**. Work stayed above the approved 16% stop point.
