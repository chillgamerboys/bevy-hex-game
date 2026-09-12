# Expedition resume checkpoint

The latest user amendments A1–A3 in `manifest.md` override the original locked decisions: 107 Goblins in fourteen camps (3,3,3,3,3,5,5,5,9,9,11,13,15,20), two Shamans, Troll, three Dragons and Shadow. Total 114 enemies / 115 actors. No enemy healing drops or player regeneration. Hidden finite fountains are the only healing source. Shadow grants +25 maximum HP with **no current-HP increase**. Troll grants +25 damage; the final Dragon unlocks explosions, both through milestone pickups.

## Preserved delivery

- Integration checkout: `/Users/alberto/Documents/Codex/2026-09-11/i-w/work/hex-expedition`, branch `wave/forest-expedition`.
- Last integrated code checkpoint: `5ddc5fb73027b90d742715e6a97e7532101cbfa2`. No remote writes or dev/main integration.
- Existing playable checkout remains `work/hex-forest` at `a872a57`; `outputs/Launch Forest Battle.command` still launches it. The expedition candidate is unfinished and must not replace that launcher before composed acceptance.
- Agent branches retain source work: `feat/expedition-world-foundation` at `43dceeb`, `feat/expedition-content` at `963d630`; their implementation commits have been integrated individually. Gameplay is integrated through `0d439414` (root `b8e7bec`). Its latest **unintegrated and uncompiled** fountain/snapshot checkpoint is `6a822d70f38cc3ae2056ab7db8a8117c730267ac` on `feat/expedition-gameplay` in `work/expedition-gameplay`.

## Current implementation

Passive named encounter/route/fountain contracts, strict manifest-bound `arena-sites.ron` loading, route clearance and real-water validation, exact tree grounding/edit guards, atomic arched bridge geometry, deterministic actor broadphase, 115-actor admission and role profiles, softer afternoon lighting and role palettes are present. The tree generator has thirteen presets with irregular tapered trunks and exact matching occupied voxels.

The radius-187 terrain proxy has 105469 columns, 444 chunks, fourteen camp pads, thirty graded route segments, six pools, three Dragon elevations and the Shadow arena opening. Compiled fingerprint: `484603000e2166fd`. Runtime package and companion are currently at:

`/Users/alberto/Documents/Codex/2026-09-11/i-w/work/expedition-content/.context/expedition-proxy/compiled`

This is a terrain-only proxy. Trees, rocks, crystals and finished structures are not populated. Compiler success is not evidence of gameplay admission, finished traversal or visual quality.

## Validation status

Lane evidence: 186 world tests pass (one pre-existing ignored release test); 350 arena tests pass (one manual benchmark ignored), strict arena Clippy passes; tree generator eight tests cover 39 preset/seed pairs; proxy twelve tests pass; bridge six tests pass including reversed and rotated arches and atomic rejection. A previous combined app/map checkpoint passed 108 game and 29 map tests before later site, roster and proxy integration.

Final pre-pause evidence:

- `bbcd24d` combined suite: **108 game tests passed / four ignored; 41 map tests passed / one ignored**. This includes all eleven site-validator and five companion-loader checks.
- Actual proxy initially rejected all spawning because gameplay expected `*_spring_*` names while the world published `*_fountain_*`. Root `b8e7bec` aligns admission/fixtures/documentation to the producer's canonical names; validators were not weakened.
- After that correction, the explicit production-loader fixture **passed with all 115 physically supported actors / 19 parties**, the exact role distribution, and identical reset roster. Setup 2922.95 ms, reset 860.48 ms; 120 idle bridge-start ticks p50 0.660 ms, p95 0.807 ms, max 1.000 ms. These are terrain-proxy simulation CPU measurements, not populated-forest/rally or FPS claims.
- Updated role XP and derived completion: **352 arena tests passed / one manual benchmark ignored**, including all-credit 327 XP → level 8 / 4 XP, uncredited/expired attribution, duplicates and final-kill settlement. Expedition deaths no longer grant automatic damage/explosion bonuses; legacy package tests pass.
- Combined Python authoring: **20 tests passed**. Cargo launcher/capture helper, including exact legacy/expedition roster validation and explicit `--forest-world` package selection: **44 tests passed**.
- Durable logs: `outputs/expedition-validation/proxy-bbcd24d.log` (failed), `proxy-b8e7bec.log` (passed), and `arena-accounting.log` (passed), under the projectless workspace root.
- No fresh expedition windowless captures or native playtest yet. The helper can now select the proxy explicitly, but its final expedition landmark matrix still needs expansion. Do not use old Forest images as new expedition evidence.
- Fountain/snapshot source commit `6a822d70` has only formatting/diff checks; its three new tests have **not run**, and compilation is **unverified**. Preserve that distinction until the reset continuation tests it. No Cargo process remains running at this checkpoint.

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

1. Inspect and cherry-pick only `6a822d70f38cc3ae2056ab7db8a8117c730267ac` from the preserved gameplay branch. It adds the approved public `expedition_progress()` snapshot, frozen-roster milestone defeated flags, named pool cells and single-use exact capsule/current-water overlap healing. Milestone positions remain `None`, collection flags remain false. Run its new tests, the arena suite, strict Clippy, and the app proxy after integration; fix actual failures before more behavior work. Do not blindly merge the entire agent branch.
2. Finish gameplay-owned milestone pickup state and app presentation. Role XP, derived 109-minion/114-enemy accounting and victory are already integrated and tested. Preserve legacy maps; implement independent defeat/available/collected states, player-only bonuses and frozen launch-time payloads. Damage/explosions intentionally remain locked in the unfinished expedition until pickup authority is implemented.
3. Implement Troll ranged/aura/rally behavior using world route facts and normal perception; confine the Shadow to its arena. Test largest camp and full surviving-forest rally at 115 actors.
4. Place supported tree blueprints on final terrain, retain local landmark clearings and exact routes/camps, and prove at least 50% whole-forest canopy coverage. Add rock/crystal formations, mountain trees, fountain approaches and detailed bridge/arena structures. The CubeWorld reference image has not been successfully inspected; do not claim otherwise.
5. Present read-only progression snapshots, milestone spheres and fountain glow/consumption, update objectives, and test XP rollover/upgrades/reward orders/final kills/pause/victory/reset and Duel/Fort regressions. Expected total XP is 327, yielding level 8 with 4 XP carried forward.
6. Profile the actual populated 115-actor map, inspect fresh windowless full-map and ground views plus HUD, then complete the requested native aiming/traversal/combat review. Earlier CUA could not address the unbundled native game; that is a technical limitation, not completed feel validation. Launch through Cargo/helper, never a bare source binary.
7. Publish a new Cargo launcher only after the candidate is ready. After all game work and validation are complete, and only with at least 20% usage remaining, review recent official OpenAI development/skills guidance and report prioritized improvements. No global settings changes are authorized by that optional audit.

## Usage and automatic continuation

User's 3% floor overrides the repository's older 7% instruction. Check live `codex` account limits, not the separate Spark bucket. At the floor, finish the bounded checkpoint and configure an hourly heartbeat on this current task. While usage remains exhausted/unchanged, perform only the quota check and stay quiet. Resume development only after a confirmed fresh window/reset in live usage, not a predicted wall-clock deadline. No reset credits may be consumed.

Parked at **97% used / 3% remaining**, weekly reset timestamp **1789435563** (September 14, 2026 at 18:26:03 America/Los_Angeles). The user expected an earlier reset, so the heartbeat uses fresh tool evidence.

Hourly current-task heartbeat **`resume-forest-expedition-after-reset` is ACTIVE**, created through `automation_update` for task `01a0936b-7adc-7c31-95e6-391395cf9e86`. State file: `/Users/alberto/Documents/Codex/2026-09-11/i-w/outputs/expedition-reset-state.json`. While waiting, do only the quota check and stay quiet on unchanged state. On a confirmed reset, resume the above work, then checkpoint again at 3%. The local app and computer need to remain available for scheduled work. Pause this heartbeat when all authorized work, including the conditional guidance review if eligible, is handled.
