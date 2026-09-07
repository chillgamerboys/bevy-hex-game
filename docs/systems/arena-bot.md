# Arena opponent

The local `arena-prototype` experiment uses one fixed-strength opponent. Both
actors share movement, charge, release, cooldown, damage, and terrain rules.
Maximum charge takes **0.75 seconds**. Tap and full launch speeds remain about
18.5 and 36.5 world units/second at the default reference speed of 32.

## Knowledge and attacks

Sight is omnidirectional but blocked by solid terrain. The bot samples sight and
visible projectile threats ten times per second, and chooses movement and behavior
five times per second. It remembers sightings for six seconds, extrapolating observed
movement for at most half a second. The estimate becomes less certain with age.
Nearby releases and impacts provide coarse, discrete cues within twelve world units.
A held charge is silent. An impact identifies a disturbance, not the shooter's
current location. Hidden actors are absent from the decision forecast's body list.

The bot evaluates charge strengths for useful, self-safe Fireballs and minimizes
remaining preparation plus projectile flight time. It keeps a charge through brief
concealment and movement, then rechecks the current trajectory before release.
Ordinary spell and cancellation authority admits every cast. Close encounters can
interrupt preparation for Area Blast or defensive Shield; the adapter cancels the
old hold before submitting a fresh press.

One blind Fireball is allowed per loss-of-sight episode, during the first 1.5 seconds
after a sighting. Its forecast must hit terrain close enough to splash the uncertain
remembered region. Heard-only contact does not authorize blind fire. Own cues do not
restore the allowance. Some concealment episodes instead trigger a 1.2-second
prepared wait at cover, followed by a peek. Reappearance and immediate danger
interrupt waiting.

## Positioning and limits

The preferred fighting distance is six to ten world units, adjusted for configured
self-splash safety. The bot strafes, approaches, backs away, and sprints on safe
open routes. Short left/right routes around blocking cover use clones of the real
movement controller. Planning is bounded to two two-second walking rollouts, at
most twice per second. Committed sides and stuck recovery prevent constant
left/right switching. Unsupported paths are rejected; terrain changes and knockback
invalidate stale routes.

This is local steering, not a navigation mesh or general planner. Deeply concave
cover, extensive destruction, and complex stacked obstacles can defeat it.
Shield placement remains a forecast: moving bodies and destruction can change
which blocks actually form. Nearby splash can still cross a Shield.

Behavior defaults live in the `bot` block of `assets/config/arena.ron`; existing
paused spell controls remain unchanged. There is no adaptive difficulty, extra
bot damage, privileged charging, or hidden-player indicator.

## Evidence and playtesting

`ArenaSession::bot_debug()` exposes remembered information, decisions, and bounded
planning counters. `round_summary()` records actual releases and HP damage; the
native game logs a compact summary when a round ends. Fireball outcome counts
separate resolved explosions from useful hits (at least 40% of maximum damage to
the living opponent before capping by remaining HP). Shots still flying or expired
are excluded from the resolved denominator. Reset clears these counters.

The default-off `hex_arena/test-support` feature retains the previous bot solely
for paired comparisons. `hex_game/test-support` forwards it only when the optional
arena dependency is enabled. Both comparison bots receive identical current spell
tuning; the baseline keeps its original fixed reference-speed charge policy.
Application evaluation runs the real world terrain publication and damage path.
Its static target, peeker, rusher, and strafing scripts share their policies across
both variants. The peeker uses ordinary movement and jumping to recover from
reachable destruction pits; an inescapable crater remains a recorded limitation.
Per-row reports retain release holds, visible-response observations, round results,
final decision state, terrain revisions, and slow-tick planning counters. CI timing
uses unoptimized arena code; native capture timing measures the launcher profile.

For a fresh windowless combat/performance sample, run
`python3 tools/arena.py capture --bot-review --output /absolute/new/directory`.
This records live bot decisions, casting, and destruction in both playable cameras.
Use `--charge-review` separately to inspect the faster meter and release prompts.

Synthetic opponents check responses and regressions, not human difficulty. Start
with twenty to thirty native rounds, then repeat after practice. A useful route is
to precharge behind cover, repeat the same peek, then change corners or approach
quietly from another direction. Track wins and whether a loss felt avoidable.
The intended bot win rates are 70–80% against a beginner and 20–30% after training;
they require human playtesting and are not automatically enforced.

The design borrows perceived memory and committed actions from
[Halo 2's AI design](https://www.gamedeveloper.com/programming/gdc-2005-proceeding-handling-complexity-in-the-i-halo-2-i-ai),
cover-peeking and last-seen suppression from
[Halo's behavior list](https://learn.microsoft.com/en-us/halo-master-chief-collection/h2/ai/aibehaviorlist),
and projectile prediction with separate attack safety from
[Quake III's bot source](https://raw.githubusercontent.com/id-Software/Quake-III-Arena/master/code/game/ai_dmq3.c).
The charge planner and numerical defaults are this experiment's own hypotheses.
