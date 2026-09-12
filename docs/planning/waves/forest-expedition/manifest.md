# Forest–Massif Expedition

Status: candidate implemented and statically reviewed; native feedback repairs in progress. Hourly continuation paused at the user’s request. Branch: `wave/forest-expedition`. Coordinator: root. Current resume authority: [resume.md](resume.md).
Local candidate based on validated `a872a57`; origin/dev at survey: `2795c75c7fb9dd61775708e98c341e0da1f33105`.
Epic: none. One outcome: a 45–60 minute Forest–Massif expedition with layered forest, 114 enemies, physical milestone pickups and finite healing.
User-approved isolation supersedes dev-first foundation/remote PR sequence. Preserve the playable `hex-forest` checkout. No dev/main merges, remote writes, unrelated branches, or investigation of the dismissed “run ended” report.

## Why this wave exists
World facts, gameplay rules and visible presentation must compose in one candidate. Establish shared facts first, then use disjoint world/gameplay/presentation lanes; the coordinator owns combination and validation.

## Locked decisions
1. Keep radius 187 and V4 authority, west forest/east massif, a gently curved central river and exactly one broad arched stone bridge with parapets, supports and open portal gates. Spawn the player on the bridge. Keep current water physics.
2. Rebuild tree distribution: approximately 36 large landmarks plus the unique 60-unit Heart, and initially about 700 smaller 6–12-unit trees. Large tiers 14–22/28–40. Smaller foliage generally begins 3.5–5 units up. Smallest landmark has about 9 units of clear walking beyond its trunk, increasing with tree size; Heart clearing fits three groups and Troll. Use irregular buttressed tapering trunks, fuller taller and narrower landmark crowns, exact matching voxel collision, and supported roots. Aim near 60% canopy with a hard 50% whole-forest floor including paths/clearings. CubeWorld giant trees are scale reference only, not copied art.
3. Add 4–10-unit forest hills, 2–4-unit gullies, local undulations, graded camps, indirect winding bridge-to-Heart paths with forks/reconnecting loops, narrow passages, broad clearings, rock formations and bright crystals. Preserve useful firing positions and local walking clearance, not globally empty understory.
4. All three Dragons exist from reset, with initial shelf levels 80/160/260, variable-width switchbacks, intermediate openings and resting shelves. Intended ascent passes the two lower encounters. Off-route slopes are steep and unpleasant but possible with repeated High Jump; no hard gate, locked spawn or defeat-triggered access. Trees surround foothills and thin up slopes toward snow.
5. A mountain side path leads to a decorated walled arena with Duel-sized radius-12 interior, tall walls/buttresses/ornaments and massive open gate. Its Shadow exists from reset, stays in the arena, has 125 HP and independent explosive Fireballs (30 damage, speed 45, gravity 12, cooldown .75), using existing Shadow movement/AI.
6. Fourteen camps contain **3,3,3,3,3,5,5,5,9,9,11,13,15,20 Goblins = 107**, incorporating A1 below. The first five camps contain 15 Baby Goblins (30 HP, walk/run 3/5, swipe 8, cooldown 1.4); 92 use the adult baseline. Two Shamans join the 13- and 15-Goblin camps, keeping the largest complete party at 20. Add one Troll, three Dragons and one Shadow: **114 enemies + player = 115 actors**.
7. Troll starts at 600 HP, about three player heights, slower movement, stronger melee and 35-damage Fireballs. Its radius-9 aura gives forest allies across parties +25% damage and existing support healing; ordinary Shaman aura stays party-local. First damaging player hit announces and rallies all surviving forest parties via authored routes. Keep ordinary perception, stable party ids, no duplicate orders; Troll death ends rally.
8. Start player at speed 45, gravity 12, contact damage 15, knockback 12, cooldown .5, explosions locked. Keep health/movement/charging/Shield/High Jump/controls and existing bankable upgrades. Gravity fixed. Freeze payload and mode at launch. Only player benefits from upgrades and rewards.
9. Rewards require proximity pickup: Troll gold orb grants a separate +25 damage (replaces all-Goblin-clear reward); last Dragon blue orb unlocks radius-2.5 explosions; Shadow violet orb grants **+25 maximum HP with zero current-HP increase**, incorporating A2–A3 below. Distinguish defeated, reward available and reward collected; apply each once, never alter in-flight shots. Milestone orbs require an unobstructed approach and a living player. **Enemies drop no health orbs and never heal the player. Fountains are the only healing source.**
10. Six glowing fountain pools (four forest, two mountain) heal up to 40 HP once, then stop glowing; full HP does not consume. Disable Forest player passive regeneration, retain enemy healing. XP immediately on credited kills: Goblin 1, Shaman 5, Dragon 20, Troll 50, Shadow 100; preserve 10-second knockback credit and once-only XP, ceil(10*1.5^(L-1)) level thresholds and rollover. All authored enemies dead marks victory with exploration and pickups still active. Restart restores terrain/enemies/stats/rewards/pools/XP; pause preserves, no persistence.
11. Use 15:00 light, softer direct sun, brighter ambient shadows, warmer highlights and restrained haze. Retain accepted V3 foliage/material/water visuals. Presentation consumes gameplay snapshots only.
12. User's new 3% remaining floor supersedes repository temporary 7%. Work in bounded units and check live usage; at floor checkpoint and activate hourly current-task heartbeat. Resume only after a live confirmed quota reset, not the clock alone. Only after game work and validation finish, with at least 20% remaining, review recent official OpenAI development/skills guidance and report prioritized improvements; no global instruction/config changes without a separate request.

## Amendments (append-only)
- A1, user 2026-09-11 follow-up: replace Decision 6's nine-camp Goblin counts with fourteen camps **3,3,3,3,3,5,5,5,9,9,11,13,15,20** = **107 Goblins**. Retain two Shamans, Troll, three Dragons and Shadow, so target **114 enemies + player = 115 actors**. Initial placement assumption: first five three-Goblin camps are babies (15); put Shamans with the 13- and 15-Goblin groups, leaving the largest complete party at 20. These placement details may be refined without changing the requested counts.
- A2, user 2026-09-11 follow-up: enemies drop **no health**, only kill XP plus the retained non-healing Troll/Dragon milestones. Remove all ordinary health-orb drops. **Hidden fountains distributed across the whole map are the only healing source.** The previously chosen Shadow +25 maximum-HP/25-heal reward is suspended pending the user's answer about keeping capacity only versus XP only; no Shadow healing may be implemented. Fountain pools remain finite 40-HP uses, unconsumed at full HP; Forest player passive regeneration stays disabled.

- A3, user 2026-09-11 clarification: retain the Shadow milestone orb granting **+25 maximum HP only**, with **zero current-HP increase**. Fountains remain the only source of healing. This resolves A2’s pending Shadow decision.

- A4, user 2026-09-12 native feedback: increase the expedition player's starting movement speed by **5%**, from **4.5 to 4.725 units/s**, without changing projectile speed, enemy movement or legacy maps. Restart restores the same bonus without stacking it. Native defeat→Restart feedback also authorizes correcting the ready-menu transition that placed Quit beneath the former Restart button.
- A5, user 2026-09-12: stop the timer. The hourly follow-up is **PAUSED**; do not reactivate it without a new request. This supersedes Decision 12's automatic-continuation instruction, while retaining the requested conditional OpenAI guidance review.

## Shared foundation
World authority owns named supported encounter areas, rally route graph and fountain volumes, exact object occupancy, root support footprints, bounded edit protections and map vertical bounds. Gameplay owns profiles, spawning/pose acceptance, spatial actor broadphase, progress/pickup/fountain state, upgrade spending and read-only HUD snapshots. Shared core holds passive data types only; app adapts publications. Existing ArenaTerrainView/Geometry/Systems remain the transport.
Root first adds backward-compatible optional authored-site contracts and records validation expectations. L1 establishes world admission/publication agreement for editable air beneath foliage, explicit root/buttress foundations and conservative legacy packages. L2 implements deterministic actor broadphase independently before population expands. L3/L4/L5 begin behavior work only after these facts are fixed.

The queue below records initial dispatch and ownership, not final delivery status. The latest integrated and pending source checkpoints are recorded in `resume.md`; no lane constitutes a delivered expanded map yet.

## Dispatch queue
```yaml
lanes:
  - id: L1
    title: World support and edit foundation
    order: orders/L1.md
    ticket: null
    authority: world
    builder: worker
    branch: feat/expedition-world-foundation
    owns: ['crates/hex_world_contracts', 'crates/hex_world_runtime', 'crates/hex_schematic', 'crates/hex_map/src/arena/forest.rs', 'docs/planning/waves/forest-expedition/manifest.md (own queue row)']
    dispatch_blockers: []
    merge_blockers: []
    fences: []
    selector: {concerns: [combined_gate], full: true}
    evidence: logic-only
    sizing: {model: inherited, effort: inherited}
    state: queued
    pr: null
  - id: L2
    title: Actor broadphase and expedition gameplay
    order: orders/L2.md
    ticket: null
    authority: gameplay
    builder: worker
    branch: feat/expedition-gameplay
    owns: ['crates/hex_arena', 'docs/planning/waves/forest-expedition/manifest.md (own queue row)']
    dispatch_blockers: []
    merge_blockers: []
    fences: []
    selector: {concerns: [combined_gate], full: true}
    evidence: logic-only
    sizing: {model: inherited, effort: inherited}
    state: queued
    pr: null
  - id: L3
    title: Expedition geography and content
    order: orders/L3.md
    ticket: null
    authority: world
    builder: worker
    branch: feat/expedition-tree-silhouettes
    owns: ['assets/config/v4/forest-massif', 'assets/art/objects/plant/forest-*', 'assets/art/object_catalog.ron', 'tools/forest_world.py', 'tools/forest_trees.py', 'tools/test_forest_trees.py', 'docs/planning/waves/forest-expedition/manifest.md (own queue row)']
    dispatch_blockers: ['shared site contract fixed', 'L1 no overlapping work']
    merge_blockers: ['L1']
    fences: ['Large-tree silhouette revision a538263d612e891f compiles: 807 trees,107 props,4908 ground contacts,8002 site supports,67.0088% whole-forest canopy; six understory assets unchanged. Fresh coordinator art/runtime admission and visual/native review remain pending.']
    selector: {concerns: [combined_gate], full: true}
    evidence: logic-only
    sizing: {model: inherited, effort: inherited}
    state: in_progress
    pr: null
  - id: L4
    title: World publication adapters
    order: orders/L4.md
    ticket: null
    authority: world
    builder: worker
    branch: feat/expedition-adapter
    owns: ['crates/hex_map/src/arena', 'docs/planning/waves/forest-expedition/manifest.md (own queue row)']
    dispatch_blockers: ['shared site contract fixed', 'L1 no overlapping work']
    merge_blockers: ['L1', 'L3']
    fences: []
    selector: {concerns: [combined_gate], full: true}
    evidence: logic-only
    sizing: {model: inherited, effort: inherited}
    state: queued
    pr: null
  - id: L5
    title: Expedition HUD and presentation
    order: orders/L5.md
    ticket: null
    authority: shared
    builder: worker
    branch: feat/expedition-presentation
    owns: ['crates/hex_game/src/arena', 'tools/arena.py', 'docs/planning/waves/forest-expedition/manifest.md (own queue row)']
    dispatch_blockers: ['gameplay read-only snapshot fixed']
    merge_blockers: ['L2', 'L4']
    fences: []
    selector: {concerns: [combined_gate], full: true}
    evidence: motion-or-feel
    sizing: {model: inherited, effort: inherited}
    state: queued
    pr: null
```

## Ownership map
L1 owns support metadata/compiler/runtime validation and forest edit-protection publication only. Root owns operators.rs::bridge plus bridge_tests.rs; L1 must leave those regions untouched. L1 additionally owns only mechanical grounding: None literal additions in hex_game/src/v4/{walk,art,object_edit,mod,knowledge}.rs, hex_map/src/v4/tests.rs, hex_perception/src/v4/tests.rs and hex_objects/src/v4/tests.rs, in a separate compatibility commit. Root owns the passive site contract and hex_map/src/arena/expedition.rs validator plus its post-compact_static call; no overlap with L1 protection logic. L4 starts after L1 and owns other adapter regions. L2 exclusively owns hex_arena gameplay. L3 owns content/blueprints/catalog/tool generation. L5 owns arena app views/materials/effects/captures. Root owns shared core vocabulary, manifest outside lane rows, integration wiring/docs and combined tests. Shared manifest rows are additive; never resolve another lane's row in isolation. No concurrent source writes in the same worktree. Pure builds and app builds use separate targets; app builds serialize.

## Territory
Read-only GitHub sweep 2026-09-11: #220 V4 on #219 Grand, #219 Grand on dev, #213 biome feedback on #212 islands, #212 on #211 desert, #211 on #210 mountain, #210 and #196 lattice on dev. These are existing donor/unrelated work; no remote changes. Refreshed local origin. Measured old candidate versus origin/dev: 719 files, 372983 insertions, 5381 deletions (large donor history, not this revision's diff). This wave has zero initial difference from a872a57. Old content/gameplay/UI worktrees are preserved. No Linear inventory in this local delivery; no ticket writes or reconciliation claims.

## Integration order
Root commits shared passive contracts; L1 and L2's broadphase can run independently. Merge L1 then world content/publication; merge L2 gameplay; finish presentation on combined snapshots. Maintain current supported baseline until package regeneration and composed acceptance succeed. One coordinator writes integration branch. Keep source branches.

## Combined acceptance
Deterministic terrain/curved continuous river/exactly one bridge; complete graded route ribbons and support/body-clearance validation; exact 115 roster and camp totals; final rotated voxel intersections/root support and full-forest canopy >=50%; Heart clearing and landmarks. Shield under canopies, cross-chunk edits, roots/anchors/objects protected, atomic rejection/reset. Profile exploration/largest camp/full rally with actual 115 actors (CPU claims distinct from FPS). Test baby/Troll profiles, aura/rally, no enemy upgrades, pickup/progression orders, XP credit/rollover, final kill/duplicates/in-flight payloads, full-HP orbs/pools, pause/victory/reset. Regression Duel/Fort. Windowless ground-level forest, full-footprint, bridge, mountain openings, Shadow arena, orb/pool and HUD captures inspected individually. Native aiming/traversal/combat/readability pass remains required; record unverified controls honestly if CUA cannot address the native game. Cargo-based launcher/guide and outputs evidence. Full selector-chosen merge gate before future dev integration; local candidate reports exact checks only.

## Stop conditions
Unknown owner facts, object/support policy mismatch, malformed anchors/routes, overlapping ownership, unable-to-validate destructive change. At live <=3% remaining pause new development, gather all agent checkpoints, commit durable state and start hourly reset heartbeat. Do not consume reset credits. No automatic claim that the reset happened at its predicted time.

## Injection log
2026-09-11: A1/A2 bank the user’s recovered camp distribution and fountains-only healing corrections.
2026-09-11: approved expedition plan, user choices and conditional OpenAI guidance review banked from planning conversation.

## Close-out
In progress. Current playable launcher still targets old validated build. All expanded map/gameplay/presentation and combined validation remain to be completed; no new visuals delivered yet.

## Checkpoint 2026-09-12, foundation combined

Candidate `b3623ff` combines passive expedition sites, deterministic actor broadphase, exact tapered tree generator, explicit V4 grounding with matching under-canopy edit guards, atomic arched bridge cross-sections and provisional softer 15:00 lighting. None of the expanded map is published to the user's launcher yet.

Focused evidence: world lane 186 tests pass with one pre-existing ignored release test; broadphase 344 arena tests pass and one manual benchmark passes; tree generator 8 tests/39 preset-seed pairs; final bridge6tests pass including reversal, six orientations, conflicting retrace and atomic refusal. Initial bridge reversal test correctly failed due directional integer truncation; bridge-only weighted interpolation fixed it without changing channels. Root combined map/game tests are compiling. Site review identified route clearance/aperture, camp-entry geometry and fake-liquid acceptance gaps; L4 follow-up fixes them before product integration.

Shared Cargo targets reused stale dependency artifacts across isolated source worktrees. One bridge invocation discovered zero tests; another reused new world contracts against old compiler source and failed a field initializer. Neither counts as evidence. Touch changed crate roots on checkout switches, serialize builds sharing a target, and require positive expected test counts. Rebuilt combined compiler executed all6bridge tests successfully.

Next: finish/test115actor roster profiles, harden site validation, compile exact-radius terrain/path proxy, integrate tree distribution/grounding and canopy target, then milestones/fountains/Troll rally and presentation. Current source branches retain work; no remote changes. Quota at6%remaining; user's3%floor applies. No reset monitor created yet because active development has not reached checkpoint floor. Optional OpenAI practice audit remains conditional on game completion and >=20%remaining after reset.

## Checkpoint 2026-09-12, 3% pause

Integrated code `5ddc5fb` includes the complete 115-actor proxy admission, canonical fountain names, role XP and derived victory, and explicit Cargo helper package selection. Combined game/map tests pass (108/41), arena tests pass (352), Python authoring/helper tests pass (20/44). The actual compiled proxy now passes supported spawning and reset for all 115 actors / 19 parties; idle simulation p95 is 0.807 ms, without any populated-forest/FPS claim. First production admission exposed and then corrected the spring/fountain ID mismatch. The tested proxy has no trees or finished decorations and is not the new playable delivery.

Gameplay fountain/snapshot source `6a822d70` is preserved separately, clean but **uncompiled, untested and not integrated**. Begin the reset continuation there. Milestone pickups, Troll ranged/aura/rally, Shadow confinement, final vegetation/structures, HUD effects, full-map combat profiling, fresh windowless captures and native feel validation remain pending. The playable `hex-forest` launcher is unchanged.

Hourly heartbeat `resume-forest-expedition-after-reset` is ACTIVE on this task. It waits quietly for a live confirmed reset, then resumes the authorized work; it does not spend reset credits. Full source locations, evidence, remaining steps and usage state are in [resume.md](resume.md).

## Resumed 2026-09-12

The hourly check confirmed a fresh weekly window with 100% available. State now has `waiting_for_reset=false`. Integrated fountain/snapshot source as `d8958a8`; 355 arena tests pass, one manual benchmark ignored. Gameplay continues milestone pickups and Troll/Shadow AI, content continues supported vegetation/global reservations, and structures continue bridge/arena/rock/crystal/fountain architecture in isolated source branches. Root owns app HUD/effects and combined checks. Explicit fixed feature rotation, public compiled-world survey and bridge travel-strip reservation are small world foundations being coordinated by the content owner before final placement. The old playable launcher remains unchanged until final acceptance.


## Checkpoint 2026-09-12, resumed gameplay complete in source

Root `e864034` includes all planned gameplay systems and their read-only presentation seam. Focused arena suite: 367 passed / one ignored at `4e94638`; combined map/game tests are in progress. The exact 115-actor camp/rally CPU fixture and 21-view map matrix are authored but not run on final content. A narrow reward-settlement correction remains to integrate. Content composition now targets 807 trees with preliminary 69.45% canopy before final compilation and structure reservations. Actual compiled geometry, full map performance, reviewed pixels and native play remain acceptance gates. No delivery launcher replacement or complete/visual-ready claim.


## Active integration checkpoint — September 12, 02:50 PT

Source through `ce155de` contains the full populated expedition, default package
preparation, milestone/healing/progression presentation, physical Shadow bounds,
and bounded forward rally re-entry after kiting or combat. The exact reproduced
package remains `f06ba29a0bdfa9b0`: 914 objects, 807 trees, 68.9567% whole-forest canopy,
42 routes, 19 encounters and 6 fountains. The requested Cube World tree scale reference
has now been inspected and is recorded in `render-review.md`.

Production art admission initially failed on an unsorted generated catalog; the
catalog and generator were repaired without relaxing validation. The complete
63-object/14-style art test passes, and all 25 legacy object fingerprints are
unchanged. Map regressions pass 45 tests / 1 ignored. Arena regressions pass 374 tests /
1 ignored, including four rally re-entry cases and an actual enemy shot comparison
after all player milestone pickups/upgrades; scoped strict arena Clippy passes.
Python content tests pass 52 cases and arena helpers 51 cases. Fresh launch preparation
reproduced the complete package, then reused 453 file hashes unchanged without a
compiler cache in 0.120 seconds. The lazy ready-screen Forest selection path is the last
source slice in progress before the combined app check.

Full populated 115-actor admission/reset, camp/rally CPU and route-controller
fixtures, the fresh 26-frame windowless review and native play remain pending.
macOS native access currently reports a locked Mac; no bypass is permitted.
The original user launcher still opens the preserved earlier map. Broad inherited
Grand/V3 map Clippy debt is not a passed full merge gate. No remote changes or
dev/main integration have occurred. Live quota is 92% remaining; user's 3% floor and
conditional post-game OpenAI audit still apply.
