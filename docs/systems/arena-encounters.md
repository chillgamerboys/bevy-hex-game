# Arena encounters: controls and tuning

This local experiment puts continuous spell combat on Fort and Seven Regions.
The original Duel remains available as the reference fight against the accepted
Shadow Player. It does not use tactical turns, lattices or multiplayer.

## Start and controls

Run the local launcher from the repository:

    python3 tools/arena.py launch

Choose a map on the start screen. Fort defaults to a Dragon encounter and also
offers five Goblins, a Shaman with three Goblins, or one Shadow. Seven Regions
contains three separate parties at the mountain high pass, fort courtyard and
cave entrance. Combat stays stopped until you press Enter or choose Start.
The launcher also accepts `--map seven-regions` or `--map duel`; Fort presets use
`--encounter dragon`, `goblins`, `shaman-party` or `shadow`.

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
quit. There is no live HUD menu button. Menu clicks never become casts.

## Watching monster battles

Choose **SPECTATE BATTLE** on the ready screen, then Fort or Duel and the two
team presets. Press Enter to begin. The observer has no character body, HP or spells.
The screen shows each team's surviving count, total HP, elapsed time and result.
A two-minute limit reports a timeout rather than declaring a winner.

    python3 tools/arena.py launch --spectator --map fort --team-a goblins --team-b shaman-party --seed 1

Team presets currently accept shadow, dragon, goblins and shaman-party. The seed
repeats the initial setup and decisions; changing the map or roster still changes
the match. `--tick-limit 14400` sets the120 Hz simulation limit. Seven Regions
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

## Encounters and recovery

A party wakes when any member sees you within 12 units, or when your attack damages
a member. Walls block sightings; shooting a hidden enemy provides a coarse cue,
not continuous knowledge of your position. Other parties must detect you independently.
You can trigger more than one party by moving carelessly.

Goblins and Shamans search for four seconds and pursue up to 18 units from home;
Shadows use six seconds/24 units and Dragons eight seconds/32 units. Survivors
return without respawning or instantly healing. Destroyed terrain and cleared
parties remain changed until restart.

On Fort and Seven Regions, recover 2 HP/second after eight seconds without casting,
dealing damage or receiving damage, and three seconds unseen by active enemies.
The original Duel keeps its existing no-regeneration rule.

Defeat all parties to clear the map. Death ends the run. R restores the entire map
and roster, including barriers, cooldowns and party memories.

## Enemies

| Enemy | Initial HP | What to expect |
|---|---:|---|
| Shadow |100| The accepted charged-shot opponent, unchanged. |
| Dragon |100| Slow flight, fast ground pursuit, fire breath and a heavy bite; retreats and shields after damage. |
| Goblin |50| Fast swarming with a short, telegraphed 12-damage swipe. |
| Shaman |60| Less aggressive Fireballs, permanent stone Shields and a timed healing/damage aura for its Goblins. |

Goblins match the player body: .8 units tall with a .25-unit radius.

The Dragon is deliberately low and long: .4 units high and about 3.5 long.
Its breath can deal 35 total damage across three pulses; a close mouth bite deals 50.
It regenerates 3 HP/second after four seconds undamaged, so chasing a retreating
Dragon can prevent recovery.

The transparent Dragon panel lasts four seconds or until its 60 HP is depleted.
It blocks direct attacks from either direction but lets bodies and sight pass.
It has a 20-second cooldown. Explosions can splash through it, as they do through
ordinary stone cover.

Shaman support lasts five seconds after a visible cast, with a 12-second cooldown.
Visible party allies within six units receive 3 HP/second and 25% extra actor
damage. It excludes the Shaman; leaving range or sight stops support, and killing
the Shaman ends the aura. Support does not stack or revive enemies.

Enemy attacks spare allied actors, and enemy projectiles pass through them.
Barriers still intercept allied attacks. Fireballs can hurt their own caster.
Bites, swipes and breath damage the terrain they contact; cover stops their direct
reach. Existing explosion damage and knockback ignore cover.

## Tuning and playtest

Player spell controls remain in the paused menu. Creature numbers and encounter
behavior live in the encounters block of assets/config/arena.ron; there is no
separate settings screen. Close and relaunch after editing this configuration
file; R resets the current run using the values already loaded. Paused spell
tuning applies within the current application.

Initial equivalence targets are one Shadow, one Dragon, five Goblins, or one
Shaman with three Goblins. These are hypotheses, not measured human win rates.
For the first 20–30 Fort encounters, note preset, win/loss, remaining HP and the
main cause of damage. Compare Dragon pursuit/escape, Goblin crowd pressure and
how much prioritizing the Shaman changes the fight.

Water remains non-solid with no swimming or drowning; required approaches are dry.
Static authored map objects remain indestructible. Ground enemies use bounded
local steering, so heavy destruction and unusual traps can defeat their routes.
Grand V3, multiplayer, progression and persistent saves are deferred.
