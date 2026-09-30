# Combat, gliding and world readability injection

Approved September 12, 2026, against local candidate 47124d9. This is an injection into the existing isolated wave, not a new release or dev/main merge. The user explicitly authorized the complete implementation. The running native game remains user-owned; native UI/feel checks are handed to the user. Timer stays paused.

## Locked contracts

1. Gameplay owns effective rank-derived player stats, frozen projectile payloads, Dragon tier, glider velocity, health events, discovery and reward accounting. Presentation reads snapshots. Legacy spell presets stay separate.
2. World owns finite object/terrain carving and its revision. Immutable authored blueprints plus sparse removals produce identical rendered/collision/sight/placement occupancy. Only affected sections rebuild. Water, air and clouds remain unaffected; unsupported solids remain.
3. Passive cross-owner additions belong in an identifiable foundation commit before their producers/consumers. Existing TerrainImpact remains the command, ArenaTerrainView remains the collision publication. Source package stays immutable during runs.
4. Rank formulas: walking 4.725*1.10^r (4); damage (15+Troll25)*1.15^r (5); separate spell speeds (45+Wisp15/Golem20)*1.10^r (5); knockback 12*1.15^r (5); cooldowns base*.85^r (5); jump min(4*1.15^r,8) (5); unlocked radius 2.5+.15*r (5); Shield 5x5,6x5,6x6,7x6,7x7 (4), plus collected Golem2x2. Gravity12. One point per rank, unavailable spends nothing. Exact even widths.
5. Brief one-second health dots above actors: first visible damaging player hit, then only band changes including healing; same-band hits never refresh. Three white above2/3, two amber above1/3, one red otherwise. Hide occluded/offscreen, clear dead, delayed changed band once on reacquisition. Level/points beside HP, gold banked and level pulse. Cards Fireball/Shield/Jump, unchanged bindings.
6. Discovery five consecutive 10Hz samples, central LOS/projected size; Dragon120, Troll/Shadow/Golem60, fountain35. Fountain central water patch must be visible. Remember hidden facts without updates.
7. Dragons aggressive above1/3, flee at/below, heal restores aggression, flight6; visible chase beyond leash, bounded remembered search; physical reposition underfoot. Summit violet/icy profile330HP,60breath,65bite,7range,60degree cone; standards220/45/50/6/50. Effects match geometry.
8. Glider G toggle from start, velocity conserved at transitions, raw mouse steering,32 cap, pitch -60/+30, bounded turning; dive45 at20 ~+12 acceleration, climb30 ~-14, level-.6. Lift fades below8, clear descent near4, stall remains open. Swept collision folds; landing/water/casting folds. Repeated airborne E retains normal cooldown and requires manual G reopen. Pause preserves, death/reset clears. No hard summit gate.
9. New plain Golems (15,72),(5,115),(35,137) and Wisp packs3/3/4 (35,60),(20,95),(0,145). Reserve before vegetation, validate bodies, winding branch. 127 enemies/128 actors. Wisp3XP/Golem25XP. Final Wisp orb +15 base Fireball speed and charging-only first-collision guide; T toggles after collection, no projectile trails. Final Golem orb +20 base Shield speed,+2x2. No HP drops. Golem map markers. Once-only final death/reward/reset; reward support fallback to living player.
10. Closed transparent water volume, cull internal faces, preserve fountain identity/colors. Camera-based underwater world tint, UI unaffected. Sun and slow decorative clouds at fixed15:00. Subtle real voxel seams/strata and bounded corner shading explain crater depth, including new surfaces.

## Ownership and integration

Root is sole integration writer. Three isolated workers may run concurrently. Each authority gets identifiable commits. Shared hot files are composed by root; workers send exact interface changes before integration. If source disagrees with surveyed entrypoints, escalate before changing ownership.

- C1 gameplay: hex_arena progression, expedition roles/rewards, spells, Dragon AI and tuning. Owns lib.rs stat/tier/role additions. Excludes player_observation.rs, glider.rs, controller.rs and motion.rs. Effective APIs preserve old defaults; add explicit spell speed/size accessors for UI and simulation.
- C2 glider gameplay: new glider.rs plus controller.rs/motion.rs; owns lib.rs glider state/input/module regions only. Coordinate GroundProfile walking speed consumer with C1. No progression edits.
- C3 world: hex_world_contracts/runtime, hex_map and domain authoring/content. Shared hex_core DTO and hex_objects baking changes are separate passive/shared foundation commits, reviewed by root before world producer commit. No hex_arena/hex_game edits.
- C4 root gameplay observation and shared presentation: player_observation.rs, hex_game HUD/input/render integration and focused capture hooks; incorporates pending pointer regression/repair c20fd423/a0c23aa first. World sky adapter may be delegated after C1 completes.

C1/C2/C3 dispatch after this document; merge C1 before C2 hot-file integration, world passive contracts before C3 world behavior, C4 last per checkpoint. Runtime acceptance is combined. The already integrated observation optimization87d058f remains included. Legacy historical lanes and remote branches are retained.

## Evidence and checkpoints

C1/C2 are motion-or-feel, with focused pure state tests; C3/C4 also require fresh windowless pixel review. No native automation without a specific reported bug. User gets2-3 actions for checkpoint1 combat/UI, checkpoint2 movement/rewards, checkpoint3 world. Continue independent implementation while manual feedback is pending. Root owns APP Cargo target; PURE granted to one worker at a time, jobs2 incremental0, no duplicate target graphs. One selector-selected combined gate after assembly; inherited Clippy debt remains explicit. Actual128-actor CPU measurements and changed-region renders (including2/8 voxel craters) required, not old115-actor evidence.

## Survey / territory

Clean47124d9 source. Pending repair clean a0c23aa in ux-ui-tests; its only source overlap is root HUD layout. Existing observation87d058f integrated. Remote open PRs220(v4 foundation),219(Grand),213/212/211/210(world biomes),196(lattice) remain outside this isolated user-approved candidate; no merges or remote writes. Local gamePID1608 runtime70de964 is not the validation candidate. Linear connection previously unavailable; local UI drafts retain evidence.

## Status

Implementation dispatched after foundation record; no new features or tests claimed complete yet.
