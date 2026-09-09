# Arena encounters: controls and tuning

Battle Mode puts continuous spell combat on Fort, Duel and Seven Regions.
Fort and Duel offer selectable enemy parties; choosing Shadow on Duel retains
the reference fight against the accepted bot. It does not use tactical turns, lattices or multiplayer.

## Start and controls

Choose **Battle Mode** on the ordinary Main Menu. It opens an isolated native
arena window, leaving the Main Menu available after you exit. On macOS and Windows
the Main Menu hides while the battle window is open; on Linux it remains visible
with its actions disabled. The ready screen offers **PLAY** and **SPECTATE BATTLE**,
map selection and party selection.

From a source checkout, open the same Fort-versus-Dragon ready screen directly:

    cargo battle

The configurable launcher has the same default:

    python3 tools/arena.py launch

For the original Shadow duel:

    python3 tools/arena.py launch --map duel

Run source builds through Cargo or this helper so the repository asset root is
applied. Check initial output for asset errors before calling a launch successful;
`Path not found` means the launch failed. The helper honors `CARGO_TARGET_DIR` and
also accepts an absolute `--target-dir` when a particular build cache is needed.

Choose a map on the start screen. Fort defaults to a Dragon encounter and also
offers ten Goblins, a Shaman with five Goblins, one Shadow, a Golem, four Ember
Wisps, or a Worm.
Duel offers the same **Enemy Party** choices and defaults to Shadow when launched directly.
Switching between Fort and Duel keeps your selected party; R restores it and returns
to the ready screen, where you can choose another. Seven Regions
contains 17 enemies in three independent parties: a Dragon at the mountain high
pass, a Shaman with five Goblins at the fort courtyard, and ten Goblins at the
cave entrance. Combat stays stopped until you press Enter or choose Start.
The launcher also accepts `--map seven-regions` or `--map duel`; Fort and Duel presets use
`--encounter dragon`, `goblins`, `shaman-party`, `shadow`, `golem`, `wisps-4` or `worm`.

| Input | Action |
|---|---|
| WASD / mouse | Move / look |
| Space / Shift | Jump / sprint |
| 1 / 2 / 3 | Select Shield / Fireball / Area Blast |
| Hold then release left mouse | Charge and cast; Area Blast has fixed strength |
| C | Switch first person / close third person |
| T | Toggle assistance for the selected projectile |
| Escape or Tab | Pause combat, release the cursor and open the menu |
| R | Restore the whole selected encounter and return to the ready screen |

Maximum projectile charge takes 0.75 seconds. Holding longer does not auto-fire.
A tap has one-third of the old reference range; full charge has 130%, measured for
a same-height 45-degree shot. Actual range follows aim, gravity and elevation.
Pausing, changing spells, focus loss, death and resetting cancel a charge.

The paused menu contains existing spell tuning, resume/reset, window mode and
quit. A win, death, draw or spectator timeout automatically opens the paused result
menu and releases the cursor. A completed round cannot resume; choose Reset Arena
to return to the ready screen or quit. There is no live HUD menu button. Menu
clicks never become casts.

## Watching monster battles

Choose **SPECTATE BATTLE** on the ready screen, then Fort or Duel and the two
team presets. Press Enter to begin. The observer has no character body, HP or spells.
The screen shows each team's surviving count, total HP, elapsed time and result.
A two-minute limit reports a timeout rather than declaring a winner.

    python3 tools/arena.py launch --spectator --map fort --team-a goblins --team-b shaman-party --seed 1

Team presets accept shadow, dragon, goblins, shaman-party, golem, worm, one goblin,
and Wisp groups of 1, 2, 4, 8 or 12. Their launcher names are `goblin`, `wisp`,
`wisps-2`, `wisps-4`, `wisps-8` and `wisps-12`. The seed
repeats the initial setup and decisions; changing the map or roster still changes
the match. `--tick-limit 14400` sets the 120 Hz simulation limit. Seven Regions
remains a player encounter map.

| Observer input | Action |
|---|---|
| Mouse | Look / orbit |
| WASD | Pan the orbit center or move the free camera |
| Q / E | Lower / raise camera |
| Shift | Faster camera travel |
| Mouse wheel | Orbit zoom |
| C | Switch orbit / free camera |
| Escape / Tab | Pause battle and release cursor |
| R | Restore the same teams, terrain and seed at the ready screen |

Cyan and amber clothing distinguish teams while creatures retain their species
shapes. The normal player HUD continues to hide enemy counts and positions.

For repeatable local calibration, commit a clean candidate and run the same real
world/simulation harness without rendering:

    python3 tools/arena_battles.py --map duel --seeds 8 --seconds 90 --output /absolute/new/battle-evidence

The default six original matchups run each seed twice with opposing side/actor-order
assignments. Use `--matchups shadow:dragon,shadow:goblins` for a subset and
`--first-seed 101` for fresh holdout seeds. Native optimized development settings
are the default; `--profile ci` is for diagnostic outcomes, and `--trace` adds dated
decision records. CI or traced timings are not native performance measurements.
The launcher retains source identity, command, log hashes and every result, refusing
missing/duplicate rounds, invalid deployment and source changes. Timeouts remain
timeouts; these machine matches do not establish human win rates. Run one build,
capture or calibration job at a time.

## Encounters and recovery

A party wakes when any member sees you within 12 units, or when your attack damages
a member. Walls block sightings; shooting a hidden enemy provides a coarse cue,
not continuous knowledge of your position. Other parties must detect you independently.
You can trigger more than one party by moving carelessly. An activated Worm has
a separate underground sensing rule described below; it never shares that
information with other parties.

Goblins and Shamans search for four seconds and normally return beyond 18 units
from home. A party with living Goblins keeps chasing while it sees the player,
including when sight returns during the trip home;
Shadows use six seconds/24 units and Dragons eight seconds/32 units. Survivors
return without respawning or instantly healing. Activated Worms retain their own
pursuit through lost sight and ordinary party leash expiry. Destroyed terrain and cleared
parties remain changed until restart.

On Fort and Seven Regions, recover 2 HP/second after eight seconds without casting,
dealing damage or receiving damage, and three seconds unseen by active enemies.
Duel keeps its no-regeneration rule for every enemy party.

Defeat all parties to clear the map. Death ends the run. R restores the entire map
and roster, including barriers, cooldowns and party memories.

## Enemies

| Enemy | Starting HP | What to expect |
|---|---:|---|
| Shadow |100| The accepted charged-shot opponent, unchanged. |
| Dragon |220| Slow flight, fast ground pursuit, occasional approach bursts, fire breath and a heavy bite; retreats and shields after damage. |
| Goblin |50| Spreading melee groups with high jumps and a short, telegraphed 12-damage swipe. |
| Shaman |60| Less aggressive Fireballs, permanent stone Shields and a timed healing/damage aura for its Goblins. |
| Golem |320| Slow seven-hex stone body, broad nearby slam, a sustained tracking laser and a frontal cover-clearing swipe. |
| Ember Wisp |30| Small glowing flyer with a long-range Ember shot; fragile alone and dangerous in groups. |
| Worm |320| Four long native hex segments, underground pursuit and an exposed-head Boulder followed by retraction and repositioning. |

Goblins match the player body: .8 units tall with a .25-unit radius. Groups approach
from wider angles and keep space between bodies. While pursuing, they attempt
2.8-unit jumps at intervals of two to three seconds only when a swept route and
landing are admitted. Ground recovery can also use a checked descent or jump out
of a hole. Goblin escorts chase a visible or remembered target even beyond their
Shaman's aura; they regroup near the Shaman when they have no target. Losing sight
still uses the party's existing search and return rules.

The Dragon is deliberately low and long: .4 units high and about 3.5 long.
Its breath reaches six units and can deal 45 total damage across three pulses; a close
mouth bite deals 50. When above 60% HP, a Dragon under ranged pressure can burst
toward its own visible opponent from at least eight units away. The burst lasts
up to .75 seconds at 12 units/second, has an eight-second cooldown and uses normal
body clearance checks. Damage interrupts it for the existing retreat behavior.
It regenerates 3 HP/second after four seconds undamaged, so chasing a retreating
Dragon can prevent recovery.

The transparent Dragon panel lasts four seconds or until its 60 HP is depleted.
It blocks direct attacks from either direction but lets bodies and sight pass.
It has a 20-second cooldown. Explosions can splash through it, as they do through
ordinary stone cover.

Shaman support lasts five seconds after a visible cast, with a 12-second cooldown.
Visible party allies within nine units receive 3 HP/second and 25% extra actor
damage. It excludes the Shaman; leaving range or sight stops support, and killing
the Shaman ends the aura. Support does not stack or revive enemies.

The Golem has a seven-hex footprint and is five levels (2 units) tall. It walks at
2 units/second and cannot jump or pass Fort's low gate. Its .8-second slam windup
precedes a 35-damage sphere of about four horizontal hexes (6.93 units), with
knockback and terrain damage. Its laser charges for two seconds, then fires for
four seconds at up to 45 damage/second, capped at 180 damage per target for the
whole cast. Aim turns at a bounded 1.2 radians/second using the Golem's own sightings.
Losing sight during charge cancels it; losing sight after release continues along
the same target's last observed position and velocity, without reading its hidden
current location. The beam stops at the nearest obstacle or hostile body. Its
12-unit laser admission range leaves a gap beyond the slam; keep moving when a
beam is already active. Slam/laser cooldowns remain 5/8 seconds.

When blocked while trying to move, the Golem can make a separate three-unit frontal
swipe after a .35-second windup, with a 2.5-second cooldown. It deals up to 25 actor
damage and applies terrain power 8 to obstructing cover, excluding its footing and
protected cells. The spherical slam can still remove its own footing; the swipe
does not make all terrain traps escapable.

Each Wisp occupies one native hex and one .4-unit level. It flies at 1.5 units/second,
roughly four units above admitted dry ground, with two altitude layers for larger
swarms. It prefers 20–32 units of separation, but can attack a nearby visible
opponent below it. Its Ember has a .35-second windup and two-second cooldown,
dealing up to 8 damage with a .8-unit splash radius. Embers travel at 128 units/second;
the opening shots of a group spread across .6 seconds. After losing sight, a Wisp
can spend up to two shots on intervening cover toward its own last sighting, held
for at most four seconds, while seeking another angle. It does not follow the
hidden opponent's current position. Walls still block sight and shots; body glow
does not reveal the Wisp through terrain. Four Wisps remain the default player
preset. The earlier twelve-Wisp group won three of four initial Fort trials against
Shadow; that small result predates this cover-pressure change and is not a current
balance estimate.

The Worm moves at 2.2 units/second, one or two voxel levels below the local surface.
After normal sight or damage activates it, a physically buried Worm senses hostile
positions across the map and keeps chasing through lost sight. This is a Worm-only
exception: it supplies no target information to its allies, does not wake a dormant
Worm by itself, and ends as the Worm starts emerging. Above ground, Boulder aim
requires actual sight. Hostile damage refreshes pursuit and triggers retraction,
canceling an unreleased shot.

The cycle is underground approach, physical head emergence, one attack attempt,
then burrow and reposition. Voluntary horizontal movement is confined to buried
travel; head transitions and swept knockback remain physical. The head must be fully
clear and at least one level above current support before firing. Boulder damage
remains 70, splash radius 2.5, impulse 8, windup .8 seconds and cooldown 3.5 seconds.
Its own Boulder causes it no damage or knockback. Opaque ground hides the body;
explosions still splash through cover.

Burrowing converts eligible earth to dirt while preserving remaining block HP,
and movement waits for the world's publication before checking the entire body
again. Destroyed support can prevent a valid shallow route. The Worm may safely
retract its head and remain unable to travel rather than cross an invalid depth or
invent support. The optional six-segment body is refused where an authored spawn
pocket is too small; bodies are never compressed to fit.

Enemy attacks spare allied actors, and enemy projectiles pass through them.
Barriers still intercept allied attacks. Fireballs can hurt their own caster.
Boulders spare the Worm that launched them, allowing close shots at swarming enemies.
Bites, swipes and breath damage the terrain they contact; cover stops their direct
reach. Existing explosion damage and knockback ignore cover.

## Tuning and playtest

Player spell controls remain in the paused menu. Creature numbers and encounter
behavior live in the encounters block of assets/config/arena.ron; there is no
separate settings screen. Close and relaunch after editing this configuration
file; R resets the current run using the values already loaded. Paused spell
tuning applies within the current application.

The current pressure pass uses one Shadow, one Dragon, ten Goblins, or one Shaman
with five Goblins. Earlier machine comparisons used five Goblins and a Shaman with
three; their win rates do not establish balance for the larger groups or the new
behaviors. See the [pressure validation record](../planning/waves/arena-bestiary/creature-pressure-validation.md)
for current checks and the [historical calibration record](arena-bestiary-validation.md)
for earlier results. Human win rates and control feel require playtesting.
For the first 20–30 Fort encounters, note preset, win/loss, remaining HP and the
main cause of damage. Compare Dragon pursuit/escape, Goblin crowd pressure and
how much prioritizing the Shaman changes the fight.

Water remains non-solid with no swimming or drowning; required approaches are dry.
Static authored map objects remain indestructible. Ground enemies use bounded
local steering, so heavy destruction and unusual traps can defeat their routes.
Grand V3, multiplayer, progression and persistent saves are deferred.

## Deferred playtest issues — 2026-09-08

The player reports that Worms do not attack back and Golems fail to destroy
obstacles to continue chasing. Both are known unresolved issues, explicitly deferred
for this delivery. The final follow-up changes only Goblin pursuit; the described
Worm/Golem abilities above express their implemented intent, not successful playtest
acceptance.
