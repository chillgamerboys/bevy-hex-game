# Expedition gameplay continuation

Checkpoint: `c5a1a966ed0b19e5e0c869a4d20b38f23a74e958` on
`feat/expedition-gameplay`. This is a resume design, not a completed feature report.
The manifest's amendments A1–A3 supersede its original roster and healing decisions.

## Resumed implementation status (2026-09-12)

The old checkpoint and next-slice list below are historical design/provenance. Root now contains fountains and public snapshots, settled proximity/LOS milestone pickups, Troll ranged attack/aura/route rally, physical Shadow arena confinement, and app presentation. Arena tests pass369cases with oneignored at `48b4692`, including isolated-crown settlement and Shadow max-HP/fountain capacity coverage. All populated content is integrated through `24bb0d9`; its first composed app run exposed an unsorted asset catalog, now repaired with a focused runtime rerun pending. A read-only audit found displaced rally survivors initially rejoining at their old camp; a bounded world-aware forward re-entry correction is in progress in the gameplay lane. Actual populated-map profiling and visual/native acceptance remain pending. Read `resume.md` and current source before dispatching work.

## Implemented checkpoint

- `ExpeditionRole` exposes gameplay-owned authored identity through
  `Actor::expedition_role()` while retaining existing `Species` compatibility.
- Complete admission is atomic when `ArenaTerrainView.expedition` is present.
  Missing/extra encounter identities, missing fountains, and insufficient body
  deployment reject the whole roster. The player remains at the supported bridge
  spawn and outside initial activation range.
- Baby movement/melee, Troll physical dimensions/melee, and Mountain Shadow
  independent spell tuning feed actual controllers, launch origins, forecasts,
  cooldowns and frozen projectile payloads. Ordinary enemy profiles are unchanged.
- Forest player passive regeneration is disabled, including the legacy Forest
  selection. Enemy regeneration and Shaman support healing remain intact.
- The absence of expedition metadata retains the legacy 26-actor spawn path.
- Evidence: 350 `hex_arena` tests passed, one manual CPU sample ignored; strict
  all-target Clippy passed after retaining integer-conversion error context.

The subsequent accounting correction registers authored roles alongside species,
awards Troll/Shadow 50/100 XP, derives the 109-minion and 114-enemy completion
conditions from the accepted roster, and prevents automatic milestone bonuses in
expedition mode. Legacy packages retain automatic clear rewards. Focused tests
cover 327 XP, final/duplicate/uncredited deaths, the attribution cutoff and reset;
the coordinator must run these new tests after integration because this lane did
not take the shared Cargo target during the root build.

**Do not enable the new package as a complete experience yet.** Milestone pickups,
fountain consumption, their public snapshots, Troll ranged/aura/rally AI and strict
Shadow arena confinement remain unfinished. Expedition milestone bonuses remain
locked until pickup authority is implemented.

## Fountain/snapshot checkpoint at the 3% floor

Source now adds `ArenaSession::expedition_progress()` and the approved snapshot,
reward, milestone and fountain types. It registers admitted pool cells and consumes
one use for up to40HP only when a living wounded player overlaps the exact pool
volume and currently published liquid. FullHP preserves the use; reset recreates it.
Milestone positions remain None and collected remains false until pickup authority
is implemented. Three focused tests cover water/body-volume checks, healing limits,
death/reset and frozen-roster snapshot facts. **This follow-up has not been compiled
or run:** quota reached3% after formatting/diff checks, and no Cargo job was started.
The coordinator must validate it before treating this source checkpoint as working.
Reward pickups, Troll ranged/aura/rally, Shadow confinement and all HUD/pool visual
wiring remain unfinished. Earlier sections describe the last validated baseline.

## Locked roster, rewards and healing

Fourteen Goblin camp counts are
`3,3,3,3,3,5,5,5,9,9,11,13,15,20`: **107 Goblins**.
The first five groups contain 15 Baby Goblins; the other 92 are adults. Two Shamans
join the 13- and 15-Goblin groups. The largest complete party therefore remains 20.
Add one Troll, three Dragons and one Mountain Shadow: **114 enemies + player =
115 actors**. The forest's **109 minions** are the Goblins and Shamans; the Troll
is a separate milestone and is excluded from that minion count.

Gameplay owns the exact encounter names `forest_camp_01` through
`forest_camp_14`, `forest_troll`, `dragon_lower`, `dragon_middle`, `dragon_upper`
and `mountain_shadow`. The six fountains are `forest_fountain_01` through
`forest_fountain_04` and `mountain_fountain_01` through `mountain_fountain_02`.

| Defeat condition | Proximity-pickup reward |
| --- | --- |
| Troll defeated | Separate permanent +25 Fireball damage, preserving upgrades/caps |
| All three Dragons defeated | Unlock standard radius-2.5 explosions and radius upgrades |
| Mountain Shadow defeated | +25 maximum HP; **current HP does not change** |

There is no reward for clearing all forest minions. There are **no enemy health
drops** and no ordinary health-orb infrastructure to build. Hidden fountains
distributed across the map are the player's only healing source. Each heals up to
40 HP once when a living wounded player enters its published liquid volume, then
loses its glow. At full HP the fountain remains unconsumed. Preserve the water and
its existing physics after consumption. Preserve enemy healing and other maps.

Kill XP is immediate and independent of collecting reward orbs: Goblin 1, Shaman 5,
Dragon 20, Troll 50, Mountain Shadow 100. All credited kills yield **327 XP**.
The existing `ceil(10 * 1.5^(L-1))` thresholds total 323 XP through level 8, so this
ends at **level 8 with 4 XP** and seven earned upgrade points before spending.
Retain surplus rollover, once-only XP and the existing 1,200-tick/10-second
attribution window for knockback deaths. Defeat milestones must also track deaths
without player kill credit.

## Approved public snapshot seam

Keep the existing `ProgressSnapshot` field layout source-compatible. Add a
gameplay-owned, read-only `ExpeditionSnapshot`, returned only for an admitted
expedition player run:

```text
ExpeditionSnapshot
  enemies_total = 114
  enemies_defeated
  forest_total = 109                 // minions, excluding Troll
  forest_defeated
  dragons_defeated
  milestones: [MilestoneSnapshot; 3]
  fountains: Vec<FountainSnapshot>

ExpeditionReward = TrollDamage | DragonExplosions | ShadowVitality

MilestoneSnapshot
  reward: ExpeditionReward
  defeated: bool                    // all three for DragonExplosions
  available_position: Option<position>
  collected: bool

FountainSnapshot
  name: stable world identity
  consumed: bool
```

The exact Rust position representation can follow existing snapshot conventions.
Only actual dropped reward orbs have public positions. Fountain snapshots carry
identity and consumption, **not coordinates or hidden-location hints for the HUD**.
World geometry stays in `ArenaTerrainView.expedition`; presentation joins those
public world facts to consumption for pool appearance. Presentation owns colors,
sphere size and effects, and does not mutate gameplay state.

`ProgressSnapshot.forest_defeated` and `forest_cleared` must derive from registered
minions rather than a hardcoded 22 in the new mode. `damage_bonus` changes only
after Troll pickup; `explosions_unlocked` only after final-Dragon pickup.
`completed` means all registered authored enemies are dead, independently of
uncollected rewards. Keep legacy-package behavior isolated until migration is
explicit; do not spawn the expanded roster in its older empty geography.

## Next bounded gameplay slices

1. The registered role/species roster, role XP, derived totals and once-only defeat
   ledger are now present. Add a separate reward-collection ledger and initialize
   expedition fountain identities from the admitted world snapshot. Publish the
   complete read-only snapshot above without changing the existing progress layout.
2. Reconcile deaths, grant credited XP immediately, and create only the three
   milestone reward orbs. The final Dragon death creates one explosion orb.
   Collection requires a living nearby player and an unobstructed approach/ray;
   it never grants a second death award. If a death location is inaccessible,
   select a reachable location using public world geometry/accepted encounter
   support, without reconstructing private map facts. Freeze each shot's damage
   and contact/explosion mode at launch as the existing projectile code does.
3. Detect physical player overlap with each fountain's exact published water
   cells using `ArenaVoxelGeometry` and existing body queries. Heal at most 40,
   spend once only when wounded, and publish consumption without changing water
   terrain. Shadow collection increases only `max_hp`, never `hp`.
4. Preserve pause state. Reset must clear pickups, consumption, defeat/credit
   ledgers, XP, upgrades and bonuses along with normal terrain/enemy restoration.
   Final-kill XP and reward availability settle before outcome checks. Victory
   leaves casting, exploration and uncollected pickups available until Restart;
   player death must not be undone by a fountain or reward.
5. Add Troll Shaman-like ranged casting and forest-wide radius-9 support across
   party ids, with ordinary Shamans remaining party-local. Troll remains a real
   Goblin-family body with role-aware abilities. On first damaging player hit,
   announce and route all surviving forest groups toward the Troll using authored
   routes, preserving ordinary player perception. Prevent duplicate orders;
   Troll death ends rally. No Dragons/Shadow in Troll support or rally.
6. Confine Mountain Shadow movement to its authored arena. The current nine-unit
   home leash is only an AI preference and is **not** proof of physical confinement.
   Keep all three Dragons and Shadow present from reset. No hard summit gate or
   defeat-triggered spawn/access contract.

The world route contract is being strengthened in a separate lane with mandatory
`clearance_levels`. After integrating that foundation, include the field in route
fixtures; a camp's rally-entry node must lie on its published deployment surfaces.
Empty-route roster fixtures currently construct no route values and need no field
migration. Actual actor movement/pose validation remains gameplay authority.

## Acceptance and integration

- Test both objective orders, uncredited deaths, duplicate notifications, exact
  thresholds, final-kill awards, XP rollover, all-player-credit 327 XP, upgrade
  restrictions and damage-cap stacking.
- Test no automatic milestone award before pickup; blocked/distant/dead-player
  pickup rejection; all three orb kinds; unchanged current HP for Shadow; no
  enemy health drops; fountain overlap, partial heal, full-HP preservation,
  single consumption, pause and full reset.
- Test launch-time payload freezing across reward collection and enemy profile
  independence from player upgrades. Keep Duel/Fort and legacy Forest fixtures.
- Test real Troll attacks/support/rally and actual Shadow confinement. Profile
  the eventual real 115-actor authored map in exploration, largest-camp combat
  and full rally. Isolated broadphase samples are not full-map/FPS evidence.
- Root integrates gameplay, world publication and presentation before final
  windowless captures and native playtesting. Do not claim a working HUD, visible
  orbs or depleted-pool appearance until that app wiring is implemented and checked.

Use small tested commits. The user-approved **3% remaining floor** overrides the
repository's old 7% rule. Check live usage between slices; at the floor checkpoint
and hand control back to the coordinator's hourly reset heartbeat. Do not consume
reset credits. Resume only after the live account quota confirms its reset.

No Cargo jobs are running from this worktree at this checkpoint. Coordinate the
shared app target with root before tests; use `CARGO_INCREMENTAL=0` and
`CARGO_BUILD_JOBS=2`. Touch the appropriate crate roots before switching worktrees
on that shared target to avoid the observed stale-fingerprint reuse. Do not run
full app builds from this lane without coordinator scheduling.
