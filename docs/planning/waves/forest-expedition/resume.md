# Expedition resume checkpoint

The latest user amendments A1–A3 in `manifest.md` override the original locked decisions: 107 Goblins in fourteen camps (3,3,3,3,3,5,5,5,9,9,11,13,15,20), two Shamans, Troll, three Dragons and Shadow. Total 114 enemies / 115 actors. No enemy healing drops or player regeneration. Hidden finite fountains are the only healing source. Shadow grants +25 maximum HP with **no current-HP increase**. Troll grants +25 damage; the final Dragon unlocks explosions, both through milestone pickups.

## Preserved delivery

- Integration checkout: `/Users/alberto/Documents/Codex/2026-09-11/i-w/work/hex-expedition`, branch `wave/forest-expedition`.
- Last code checkpoint: `bbcd24d`. No remote writes or dev/main integration.
- Existing playable checkout remains `work/hex-forest` at `a872a57`; `outputs/Launch Forest Battle.command` still launches it. The expedition candidate is unfinished and must not replace that launcher before composed acceptance.
- Agent branches retain source work: `feat/expedition-world-foundation` at `43dceeb`, `feat/expedition-gameplay` at `c5a1a966`, `feat/expedition-content` at `963d630`. Their implementation commits have been integrated individually.

## Current implementation

Passive named encounter/route/fountain contracts, strict manifest-bound `arena-sites.ron` loading, route clearance and real-water validation, exact tree grounding/edit guards, atomic arched bridge geometry, deterministic actor broadphase, 115-actor admission and role profiles, softer afternoon lighting and role palettes are present. The tree generator has thirteen presets with irregular tapered trunks and exact matching occupied voxels.

The radius-187 terrain proxy has 105469 columns, 444 chunks, fourteen camp pads, thirty graded route segments, six pools, three Dragon elevations and the Shadow arena opening. Compiled fingerprint: `484603000e2166fd`. Runtime package and companion are currently at:

`/Users/alberto/Documents/Codex/2026-09-11/i-w/work/expedition-content/.context/expedition-proxy/compiled`

This is a terrain-only proxy. Trees, rocks, crystals and finished structures are not populated. Compiler success is not evidence of gameplay admission, finished traversal or visual quality.

## Validation status

Lane evidence: 186 world tests pass (one pre-existing ignored release test); 350 arena tests pass (one manual benchmark ignored), strict arena Clippy passes; tree generator eight tests cover 39 preset/seed pairs; proxy twelve tests pass; bridge six tests pass including reversed and rotated arches and atomic rejection. A previous combined app/map checkpoint passed 108 game and 29 map tests before later site, roster and proxy integration.

The `bbcd24d` combined map/game suite and explicit 115-actor production-loader proxy test are pending at this document's initial write. Do not infer a pass from the earlier suite or from an ignored test listing. Append their actual outcomes below before parking.

Builds sharing a target must be serialized. Same-version isolated worktrees previously reused stale Cargo artifacts: one invocation discovered zero tests, another linked incompatible grounding contracts. Touch the changed crate roots when switching source trees and require the intended positive test count. App target:

`/Users/alberto/Documents/Codex/2026-09-04/there-were-a-few-issues-i/work/cargo-target-explore`

Use `CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2`. The pure target is `/Users/alberto/Documents/Codex/2026-09-04/i/work/cargo-v4-pure`.

From the integration checkout, the combined check is:

```bash
touch crates/hex_core/src/lib.rs crates/hex_arena/src/lib.rs crates/hex_map/src/lib.rs crates/hex_game/src/lib.rs
CARGO_TARGET_DIR=/Users/alberto/Documents/Codex/2026-09-04/there-were-a-few-issues-i/work/cargo-target-explore CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test -p hex_map -p hex_game --features hex_game/dev,hex_game/test-support,hex_map/arena-prototype --lib arena::
```

Then explicitly run the ignored full-scale fixture:

```bash
HEX_FOREST_WORLD=/Users/alberto/Documents/Codex/2026-09-11/i-w/work/expedition-content/.context/expedition-proxy/compiled CARGO_TARGET_DIR=/Users/alberto/Documents/Codex/2026-09-04/there-were-a-few-issues-i/work/cargo-target-explore CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test -p hex_game --features dev,arena-prototype,test-support --lib arena::forest_tests::authored_expedition_proxy_has_115_supported_actors_and_resets -- --ignored --nocapture
```

Its receipt measures real supported spawning, reset and idle bridge-start simulation CPU. It does not measure a populated forest, full Troll rally or renderer FPS.

## Next work

1. Resolve any production-loader/physical-spawn proxy failures without weakening the validators.
2. Replace old hardcoded 22/3 automatic progression for expedition packages with authored-role XP and gameplay-owned milestone/fountain state. Preserve legacy maps. Implement independent defeat/available/collected states, player-only bonuses, launch-time payloads and exact reset. The current progression is still the old implementation and is not suitable for playing the expanded roster.
3. Implement Troll ranged/aura/rally behavior using world route facts and normal perception; confine the Shadow to its arena. Test largest camp and full surviving-forest rally at 115 actors.
4. Place supported tree blueprints on final terrain, retain local landmark clearings and exact routes/camps, and prove at least 50% whole-forest canopy coverage. Add rock/crystal formations, mountain trees, fountain approaches and detailed bridge/arena structures. The CubeWorld reference image has not been successfully inspected; do not claim otherwise.
5. Present read-only progression snapshots, milestone spheres and fountain glow/consumption, update objectives, and test XP rollover/upgrades/reward orders/final kills/pause/victory/reset and Duel/Fort regressions. Expected total XP is 327, yielding level 8 with 4 XP carried forward.
6. Profile the actual populated 115-actor map, inspect fresh windowless full-map and ground views plus HUD, then complete the requested native aiming/traversal/combat review. Earlier CUA could not address the unbundled native game; that is a technical limitation, not completed feel validation. Launch through Cargo/helper, never a bare source binary.
7. Publish a new Cargo launcher only after the candidate is ready. After all game work and validation are complete, and only with at least 20% usage remaining, review recent official OpenAI development/skills guidance and report prioritized improvements. No global settings changes are authorized by that optional audit.

## Usage and automatic continuation

User's 3% floor overrides the repository's older 7% instruction. Check live `codex` account limits, not the separate Spark bucket. At the floor, finish the bounded checkpoint and configure an hourly heartbeat on this current task. While usage remains exhausted/unchanged, perform only the quota check and stay quiet. Resume development only after a confirmed fresh window/reset in live usage, not a predicted wall-clock deadline. No reset credits may be consumed.

Last observed usage at this document's initial write: 96% used / 4% remaining; weekly reset timestamp 1789435563 (September 14, 2026 at 18:26:03 America/Los_Angeles). The user expected an earlier reset, so the heartbeat must use fresh tool evidence. No monitor has been created at this initial write; record its actual ID and state before claiming it is scheduled.
