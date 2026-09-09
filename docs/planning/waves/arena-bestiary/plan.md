# Approved overnight arena continuation

Local branch: experiment/spell-combat-arena. Accepted human/Shadow reference:
127d1ce2058de9ba79da9717b7e37df4b9913502. Initial encounter code checkpoint:
27338deefcf12fdebba573aa2604b7fcb8f6c9cd. The user approved implementation and
subsequent corrections on 2026-09-07, then went offline for approximately ten hours.

## Execution order

1. Finish the original Dragon/Goblin/Shaman/Fort/Seven Regions milestone, including
   the corrected player-sized Goblin body, meaningful native captures and load measurements.
2. Commit a gameplay-owned spectator contract, then implement observed hostile targeting,
   two monster teams, observer UI/cameras and deterministic battle summaries.
3. Calibrate Dragon, five Goblins, and Shaman + three Goblins against the unchanged Shadow.
4. Add and calibrate Golem, then Ember Wisp, then Worm. No Charging Boar.
5. Run the repository-selected combined gate, strict workspace lint, native build,
   performance and fresh independent static review on the combined final candidate.
   Deliver local commits, launcher, guide, and evidence/limitations report.

All work stays local. No visible native game unless the user asks to play, no remote
publish/merge, no other checkout changes, no unapproved cache or evidence deletion.
Human controls, three spells, movement and accepted Shadow policy stay unchanged.
Automated monster battles establish relative machine matchups, not human win rates.

## User-locked corrections

- Goblins match the player's height and width, including hitboxes: 0.8 tall,
  0.25 radius. Their original HP/move/ability hypotheses remain the calibration starting point.
- Golem is slower and physically larger than the player, with a round seven-hex
  base and five voxel levels of height. The user explicitly clarified this is the
  Golem, not the Goblins.
- Golem has a short-range spherical slam of about four voxels radius and a long-range
  straight laser; no medium-range attack. Laser charges visibly for a long time,
  fires very quickly and lasts about one second. One Golem approximately equals one Shadow.
- Wisp flies slowly, occupies one voxel, glows conspicuously including darkness,
  attacks at very long range, and has weaker stats than a Goblin. Flight can let one
  defeat a melee Goblin. Increasing swarm numbers must create substantial pressure.
- Worm replaces the Boar. Four to six voxels long, travels one or two levels below
  the surface, converts blocks in front to dirt, is hidden by opaque earth, and
  cannot attack underground. Head must emerge by at least one level before throwing
  a damaging knockback boulder. It should beat a Goblin group but be dodgeable by faster players.
- The apparent phrase "cannot shoot" after emergence was read as "can shoot"; the
  user then approved the rest and asked to start.

## Working numeric assumptions, adjustable during calibration

Hex levels are 0.4 units high; horizontal neighbor centers are about 1.732 units apart.
Four voxels of slam radius means four horizontal hex steps, approximately 6.93 world
units, resolved as a physical sphere. The Golem body height is 2.0 units.

Golem initial HP160, speed2.0. Slam damage35, windup0.8s, cooldown5s, impulse5,
terrain power2. Laser admission starts at12 units, leaving a deliberate band beyond
slam reach with no attack. Charge2s, aim locks for the final0.35s, fixed firing line
for1s, damage capped45 per actor per cast, cooldown8s. Beam width and lock delay are
tuning values. Its ray extends to the actual world boundary without an artificial
combat range cap and stops at the nearest direct-attack obstruction. A narrow
warning line and increasing glow communicate charge; no enemy-revealing HUD marker.
The locked firing direction follows the physical mouth if knockback moves the body;
it is not a detached world-space gun. Laser terrain damage is elemental Fire power2,
admitted once per voxel per complete cast rather than on every simulation tick.
The45 actor-damage cap is distributed over the active second. Death cancels the
remaining beam. These conventions are shared before the Golem phase begins.

Wisp initial HP18, flight speed1.5, one horizontal hex by one level body. Long-range
engagement20–32 units after normal party activation. Ember shot initial damage8,
small splash0.8, cooldown2s, visible windup0.35s, speed32 and gravity2. Test swarms
of1/2/4/8/12 and report the observed Shadow crossover instead of asserting the superseded
two-Wisp equivalence. Opaque geometry still blocks sight and direct flight paths.

Worm initial length4 hexes (configuration can compare6), one-hex width, one-level
body, burrow speed2.2 and depth1–2 levels. Initial HP140. It periodically surfaces to
look, retains only actual sightings while underground, then surfaces for a telegraphed
boulder. Initial windup0.8s, cooldown3.5s, launch speed18/gravity12, direct damage50,
small impact splash and strong horizontal knockback for grouped melee enemies.
Tuning decides the final splash/retreat cadence. No attack is admitted while the
mouth remains covered, and already released shots keep their source identity.

## Spectator contracts and behavior

ArenaSelection continues to own only the world recipe. A gameplay resource owns
Player/Spectator mode, two team rosters/parties, deterministic seed and optional tick
limit. No dummy human in spectator mode; actor0 may be a real monster. Validate
teams, species, bounded counts and safe spawns before play. Begin with the four
original rosters; add each new roster only after its species exists.

Fort is the initial real-map battle. Retain the original Duel arena as a neutral
calibration layout where needed, without changing its ordinary human mode. Seven
Regions remains the original three-party human map. Roster placement uses actual
footprints and continuous dry approaches; invalid placement is an error, never a loss.

Only sensing reads live hostiles. Dated observations carry target ID, team, body
profile, observed pose/velocity and time. Target changes clear old target-specific
history. Nearest visible opponents and stable tie breaks suffice; no hidden current
pose is supplied to decisions or forecasts. Party support remains same-party.
Battle parties start active; searching may use the public opposing deployment area,
but home-return/leash cannot end the match. Preserve original player encounter policies.

Public optional human ID, terminal/finished state and team summary drive UI. Summary
distinguishes winning team, simultaneous draw, timeout and invalid setup. Spectator
has no health/damage/collision body. Orbit/free camera, start/pause/reset/focus-loss
and team-colored actors are observer presentation. Normal player HUD gains no hidden
enemy information. The launcher and receipts retain seed, setup, timing and outcomes.

## Calibration and validation

Freeze meaningful deterministic Duel goldens before target generalization. Test
nonstandard team IDs, monster actor0, mixed body targets, target switches, hidden-state
equivalence, ally passage after source death, aura membership, whole-team outcomes
and reset. Calibrate using real ArenaTick/world publication, ordinary HP and brains,
fresh terrain each match, bounded90-second fixtures, and both side/ID initiative orders.

Use an initial8-seed paired smoke corpus for the six original roster pairings, then
roughly20 paired seeds against Shadow for calibration and a small fresh holdout.
Record wins/draws/timeouts, survival HP, duration, damage/ability use and CPU percentiles.
Aim roughly40–60% against Shadow for the original equivalent rosters and Golem.
Wisp/Worm follow the user's distinct matchup goals; natural counters are permitted.
Adjust new enemy values first; repair attacks, movement and stalls rather than compensating
for defects with HP. Do not tune the accepted Shadow or human abilities.

Worm conversion remains world-owned, bounded to contacted cells, cannot edit protected
bedrock/static objects/liquids or trap other actors, and must not repair existing damage.
Surface and underground body queries, exposed-head aiming, terrain conversion, persistent
dirt, death and reset require focused tests. Existing unoccluded explosion splash remains
consistent. No underground movement/HP indicators in human play.

Each new species receives typed attack/body/death tests and native model/windup/effect
captures before the next species begins. Full-resolution independent review and a
contact sheet establish only static presentation. User motion, feel and20–30 human
encounters remain explicitly pending.

## Coordination

Root integrates, commits, owns shared contracts, tournament tooling and delivery docs.
Gameplay lane owns hex_arena and arena.ron. Presentation lane owns hex_game arena UI,
models/captures and tools/arena.py. World lane owns world-owned transformation/publication
when the Worm step is admitted. Record the in-repository wave manifest before spectator
implementation begins. Expensive Cargo/capture/timing jobs are serialized and use warm
caches with jobs1/incremental0; preserve at least0.75GiB free and retained evidence.
