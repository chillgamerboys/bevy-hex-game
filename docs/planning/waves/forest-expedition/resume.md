# Expedition resume checkpoint

Active continuation, September 12, 2026. **Keep implementing until complete or live Codex quota reaches 3% remaining.** The reset was confirmed; `waiting_for_reset=false`. Latest usage check: 84% remaining. Earlier checkpoints are preserved in Git history rather than repeated as current instructions here.

## Candidate and authoritative requirements

- Integration checkout: `/Users/alberto/Documents/Codex/2026-09-11/i-w/work/hex-expedition`, branch `wave/forest-expedition`. Integrated runtime head before the current presentation repair is **2040abe**. No remote writes or dev/main merges. Preserve source branches and the old playable `work/hex-forest` checkout/launcher until composed acceptance.
- [manifest.md](manifest.md), amendments A1–A3, govern the expansion. **107 Goblins in 14 camps: 3,3,3,3,3,5,5,5,9,9,11,13,15,20.** Two Shamans join the 13/15 camps. First five camps contain 15 Baby Goblins. Troll, three Dragons and Shadow bring the total to **114 enemies / 115 actors, 19 parties**.
- **Fountains are the only healing source.** Six hidden single-use 40-HP pools, not consumed at full HP. No enemy HP drops or passive player regeneration. Shadow violet orb adds **25 maximum HP and zero current HP**. Troll gold orb adds 25 damage; final-Dragon blue orb unlocks explosions. Collection requires proximity and line of sight. Gameplay owns once-only XP, defeat/available/collected state, frozen projectile payloads, upgrades, victory exploration and reset.
- Start on bridge with 45 launch speed, 12 projectile gravity, 15 contact damage, 12 knockback, .5 cooldown, explosions locked. Enemy spells remain independent. All credited kills give 327 XP: level 8 with 4 XP carried and seven bankable points.
- All requested content/gameplay exists in source. Static presentation repairs, populated CPU evaluation, remaining combined checks, final windowless review and native acceptance are still open. Do not describe the map as delivered.

## Current package

`assets/config/v4/forest-massif/expedition/compiled`, fingerprint **a538263d612e891f**. Root reproduction passes: 105469 columns / 444 chunks, 914 objects (807 trees + 107 props), 4908 ground contacts, 8002 support/clearance positions, 42 routes, 19 encounters, six fountains. Tree count includes 700 understory, 36 landmarks, the Heart and 70 mountain trees. Whole-forest canopy **31992/47743 = 67.0088%**, including routes and clearings. Seven large-tree blueprints changed; the other 56 catalog objects, including all understory and props, remain byte-identical. Root and independent reviewer have inspected runtime tree pixels on this package. Cube World scale reference was inspected and is recorded in [render-review.md](render-review.md).

## Verified evidence

Logs and JSON receipts live at `/Users/alberto/Documents/Codex/2026-09-11/i-w/outputs/expedition-validation`.

- At `50e9e60`: **382 arena tests PASS / one ignored; 220 asset tests PASS; strict arena Clippy PASS.** `arena-art-a538-complete.log`, `clippy-a538-arena.log`.
- At `50e9e60`: **119 game tests PASS / seven ignored; 46 map tests PASS / one ignored.** `composed-a538-normal.log`. The arena/art parts of that filtered invocation ran zero tests and are not their evidence.
- At `50e9e60`, actual a538 package: **all four ignored expedition fixtures PASS**, `populated-a538-profile.log`. Exact 115 supported actors / 19 parties and reset; ordinary-tick reward/fountain readiness; all **408/408 segmented route traversals, 9360 waypoint visits**, both directions for player/adult Goblin/Shaman. These are production-controller collision/support checks with synthetic standing starts, not continuous crowd/native movement proof.
- Python arena tooling **51 PASS**, `python-arena-final-integration.log`; separate authoring/tree tests **54 PASS**. First-launch package locking/atomic publication subset **20 PASS**. Source `785434d` serializes first launches; `cfc4d09` removes old Duel/Fort render entities when entering Forest, covered by the combined map suite.
- CPU baseline on actual a538: idle bridge p95 .770 ms; camp p95 **39.482 ms**, rally p95 **20.673 ms**. Brains dominate spikes. Movement/separation p95 about 1 ms. Receipts `expedition_active_receipt-a538.json`, `expedition_proxy_receipt-a538.json`. The latter's PROXY text is stale: the command loaded actual a538. These measure ArenaTick CPU, never renderer/GPU/FPS/native.
- Cache integrated as `73dd277` + `2040abe`, from performance agent `135d04b` + `1e12163`: exact ordered short Movement candidate spans cached only within one brain phase, capped at 128 entries/8192 spans; no decisions, clearances, hits, barriers, long rays or cross-tick terrain state cached. **386 arena tests PASS / one ignored**, default and test-support strict Clippy PASS in agent PURE target, including ordered oracle and 720-tick cached/uncached trajectories. Root combined actual CPU rerun remains pending.
- Full map Clippy retains inherited Grand/V3 debt (517 library / 829 test findings); it is not a passed merge gate. Current scoped game Clippy and explicit `arena_encounters`, `arena_routes`, `arena_battles` integration suites remain pending.

## Static and native review

- First full 26-frame old `ccc5847` / f06 pack: independently inspected **13 PASS / 10 FAIL / 3 BLOCKED**, static FAIL. Keep it historical.
- Repair subset: exact clean **cd6e2c08a258de5870e33b3d78ac296ba1aac531** / a538, 14 frames, mechanically complete, all originals + contact sheet inspected by root and independent `actor_broadphase`. **11 PASS / 3 FAIL**. Trees, brighter forest shadows, full overview, Heart, arena and gate now pass static inspection. Remaining issues: reverse bridge foreground trees obscure span; charged fountain tint/glow too weak; mountain fountain camera crops basin and lacks approach context.
- Pack: `.context/expedition-repair-first/cd6e2c08a258de5870e33b3d78ac296ba1aac531-forest-expedition-v2-rewards-focused`, independent `independent-review-actor-broadphase.{md,json}` and contact sheet. These are diagnostic subset results, not final 26-view acceptance.
- Current root repair changes only presentation/capture code: stronger translucent turquoise cap and drifting glimmers hidden under the consumed pool parent; bridge camera moved into river center; mountain pool composition uses published approach with higher fallback. Charged/spent forest views share a camera. New actual-package test covers charged children, consumption visibility and reset. **Not compiled or rendered yet.** Existing lighting is 15:00 / ambient1100 / directional6000 and already passed static inspection.
- Native CUA still reports **Mac locked**. A concise async request to unlock it is pending; no answer yet, no bypass. User already authorized the native playtest. Continue independent work. Native aiming/traversal/combat readability and motion remain HUMAN-MOTION-PENDING.

## Immediate next work and target ownership

1. Performance agent is profiling actual a538 on its clean **1e12163** worktree (`work/expedition-performance`; runtime-equivalent to root2040abe, docs differ). It exclusively owns APP_TARGET until released. Outputs are `cache-a538-*`. Compare exact steering counters against the baseline before claiming a behavior-preserving speedup. All 14 forest parties move, but 109 travellers remaining after a 21-second fixture is not evidence of arrival or failure.
2. Root finishes the current presentation repair and its test; actor agent reviews source read-only. Do not build against APP_TARGET until performance agent releases it.
3. Run scoped game Clippy, meaningful presentation test and remaining legacy integration checks; fix real failures. Capture a small repair subset if needed, inspect original pixels, then produce a fresh full 26-view pack from one clean committed source/package and inspect every original plus contact sheet independently.
4. Complete native route after manual unlock and successful addressing of the real Cargo-launched candidate. Repoint Cargo launcher/play guide only after composed acceptance. Do not substitute old binary or count screenshot checks as gameplay proof.
5. Once all game work and validation are complete, with at least 20% quota remaining, do the requested recent official OpenAI skills/development-guidance audit. This is not eligible yet. No global settings/instruction changes are authorized.

APP_TARGET: `/Users/alberto/Documents/Codex/2026-09-04/there-were-a-few-issues-i/work/cargo-target-explore`.
PURE_TARGET: `/Users/alberto/Documents/Codex/2026-09-04/i/work/cargo-v4-pure` (free).
Use `CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2`. Serialize shared targets. When switching source worktrees, refresh changed crate roots and require positive expected test counts; stale cross-worktree artifacts previously gave zero tests or incompatible rlibs. Never execute a bare source binary; Cargo supplies asset roots.

## Quota and automatic continuation

Hourly heartbeat **resume-forest-expedition-after-reset** remains ACTIVE on task `01a0936b-7adc-7c31-95e6-391395cf9e86`. State is `/Users/alberto/Documents/Codex/2026-09-11/i-w/outputs/expedition-reset-state.json`; confirmed reset window ends at Unix1789806214. Use live `codex` limits, not Spark. At 3% remaining, finish a bounded checkpoint, update state and wait quietly for a confirmed restored quota window; check hourly. Never consume reset credits. Pause heartbeat only when all authorized work is complete.
