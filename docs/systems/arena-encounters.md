# Arena encounters: controls and tuning

Battle Mode defaults to Forest–Massif and also offers Fort, Duel and Seven Regions.
Fort and Duel offer selectable enemy parties; choosing Shadow on Duel retains
the reference fight against the accepted bot. It does not use tactical turns, lattices or multiplayer.

## Start and controls

Choose **Battle Mode** on the ordinary Main Menu. It opens an isolated native
arena window, leaving the Main Menu available after you exit. On macOS and Windows
the Main Menu hides while the battle window is open; on Linux it remains visible
with its actions disabled. The ready screen offers **PLAY** and **SPECTATE BATTLE**,
map selection and, on Fort/Duel, party selection. Forest–Massif uses its fixed player roster.

From a source checkout, open the same Forest–Massif ready screen directly:

    cargo battle

The configurable launcher has the same default:

    python3 tools/arena.py launch

For the original Shadow duel:

    python3 tools/arena.py launch --map duel

Run source builds through Cargo or this helper so the repository asset root is
applied. Check initial output for asset errors before calling a launch successful;
`Path not found` means the launch failed. The helper honors `CARGO_TARGET_DIR` and
also accepts an absolute `--target-dir` when a particular build cache is needed.

The implicit Forest selection uses the expedition package. On first launch,
Cargo/helper preparation builds the pure world compiler if needed and generates
the reviewed package; later launches reuse it. Python 3 is required for source
preparation. `HEX_FOREST_WORLD` or the helper's `--forest-world ABSOLUTE_PATH`
selects an already compiled package, including the older Forest map.

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
| WASD / mouse | Move / look (4.725 units/s in the expedition; 4.5 on legacy maps) |
| Space | Ordinary jump |
| E | Immediate High Jump; preserves a held projectile charge |
| Hold then release left mouse (LMB) | Charge and cast Fireball |
| Hold then release right mouse (RMB) | Charge and cast Shield |
| C | Switch first person / close third person |
| T | Toggle active projectile assistance; expedition Fireball guide requires the Wisp reward |
| G | Open/fold the expedition momentum glider while airborne |
| Escape or Tab | Pause combat, release the cursor and open the menu |
| R | Restore the whole selected encounter and return to the ready screen |
| M | Toggle the expedition minimap without pausing |
| F9 | Bookmark the current native recording |
| Cmd-Q (macOS) | Finalize any recording and quit |

Battle UI keeps the active screen to the reticle, Fireball/Shield/High Jump cards,
compact player HP and level, plus optional map/recording or glider indicators.
Cards distinguish ready, cooldown and held-charge states. Available upgrade points
turn the level indicator gold. Enemy health appears for one second above the body:
three white pips above two-thirds HP, two amber pips above one-third, otherwise one
red pip. The first visible damaging player hit announces health; later announcements
require a band change, including healing. Same-band hits never extend the display,
and occluded or off-screen bodies have no indicator. Esc pages hold Overview, Map, Upgrades,
Settings and Controls. Settings cycles interface size from 100% through 200%; menus
scroll with the wheel and arrows move keyboard focus. Presentation preferences
persist separately from the current run.

The expedition's north-up map remembers Dragons, Shadow, Troll, Golems and fountains
only after five consecutive visible observations at ten Hz. Discovery requires a
meaningful central view, within 120 units for Dragons, 60 for bosses/Golems, and 35
for a fountain's central water patch. Icons remember last-seen locations and
charged/spent fountain state; hidden enemies do not update their remembered position.
Click a discovered marker in Esc → Map to inspect it, or click terrain to set one
personal destination. M starts hidden each run; Restart clears all discoveries and
the destination. The expedition player has a 1.2-unit physical body and 1.02-unit
physical eyes, retaining the .25 radius and existing jump/step behavior.

On macOS 15+, Esc provides Start/Stop Recording and Open Recordings. The bundled
helper requests screen-capture permission on the first Record action, captures only
the requesting arena window (HUD and menus included), and records video without
microphone or system audio. MP4 clips are H.264, SDR, 30 fps, at most 1080p, saved to
`~/Movies/Hex Game/Recordings/` alongside timestamped `.events.jsonl` metadata and F9
bookmarks. Red REC appears only after the native start callback. Pause, death and
Restart remain in the same clip. Esc Quit, the window close button and Cmd-Q wait
for finalization; forced termination, Dock Quit or logout may leave an explicitly
partial file. Other platforms keep gameplay available and show recording unavailable.
Full recorder status appears in Overview. Native implementation details are in
[the recorder guide](../../crates/hex_game/native/recorder/README.md).

Maximum projectile charge takes 0.75 seconds. Holding longer does not auto-fire.
A tap has one-third of the old reference range; full charge has 130%, measured for
a same-height 45-degree shot. Actual range follows aim, gravity and elevation.
The first mouse button pressed owns the charge. Pressing the other button during
that hold neither switches spells nor queues another cast; release and press it
again to begin a new gesture. Only the actively charging spell is highlighted.
Pausing, focus loss, death and resetting cancel a charge.
Legacy Human and Shadow use 4.5 units/s; the expedition player uses 4.725. Shift does not add sprinting.
Pressing **E** does not cancel charging: you can jump high while preparing or releasing
Shield or Fireball. High Jump replaces Area Blast. It is an immediate upward boost,
usable on the ground or in the air, with **four world units** of rise from rest and a
**seven-second cooldown**. Ceilings still block it. It deals no damage or terrain
damage. Holding E does not repeat, and pressing during cooldown does not queue a
future jump. Pausing, focus loss and reset discard pending presses and require a
fresh press afterward.

The paused menu contains existing spell tuning, resume/reset, window mode and
quit. A win, death, draw or spectator timeout automatically opens the paused result
menu and releases the cursor. A completed Fort/Duel/Seven Regions round cannot resume; choose RESTART
to return to the ready screen or quit. There is no live HUD menu button. Menu
clicks never become casts.

On the older maps, the same menu adjusts **High Jump height** from **2–8 units** in 0.5-unit steps and
its cooldown from **0.5–20 seconds**. **Shadow reaction** defaults to **150 ms** and
can be changed from **Off (0 ms) to 500 ms** in 50 ms steps. This is the delay after
acquiring or reacquiring sight; sight sampling can add up to 100 ms. It does not
change charge speed or the existing post-cast gap. **Shadow escape** toggles local
walking/jumping recovery from holes. Settings apply on resume and survive round
reset and map changes within the session; reopening the game loads configured defaults.

## Forest–Massif progression

The expedition package uses a radius-187 V4 world with 14 forest camps, three
mountain Dragons, a Troll beneath the central giant tree, and a Shadow in a walled
arena. The player starts on the arched bridge across the curved river. The camps
contain `3,3,3,3,3,5,5,5,9,9,11,13,15,20` Goblins; the first five are babies.
Two Shamans join the 13- and 15-Goblin camps. The southern mountain-side plain adds
three independent Golems and Wisp packs of 3, 3 and 4. There are 127 enemies/128 actors
in total.
Terrain loads at runtime and the ready screen waits for its rendering publication.

Fireball starts with 15 contact damage, 45 base launch speed, 12 projectile gravity,
12 knockback and a 0.5-second cooldown. It stops at the first valid collision and
only hurts the struck target. Defeating the Troll leaves a gold reward sphere
that adds 25 to base damage before purchased damage multipliers. Defeating all three
Dragons leaves a blue sphere that unlocks radius-2.5 explosions. The Shadow leaves
a violet sphere granting 25 maximum HP **without healing current HP**. Clearing all
ten Wisps drops a reward granting +15 base Fireball speed and a charging-only aim
guide, ending at first collision. Clearing all three Golems drops a reward granting
+20 base Shield speed and +2 columns/+2 levels to Shield dimensions. Walk near a sphere
with a clear approach to collect it. Every reward is once-only; clearing an area does
not award its bonus until collection. Each shot freezes speed, damage, geometry and
impact mode at launch. Enemy spells have independent tuning.

Enemies give XP and **never drop health**. The player does not regenerate HP.
Six hidden fountains are the only healing source: four in the forest and two in
the mountains. Enter their glowing water while wounded to recover up to 40 HP once;
the glow then fades, but the water remains. A full-health visit does not consume it.

Player kills award Goblin 1, Shaman 5, Wisp 3, Golem 25, Dragon 20, Troll 50 and Shadow 100 XP,
including attributed knockback deaths within ten seconds. Levels require 10, 15,
23, 34… additional XP; surplus carries forward. Each level banks one upgrade point.
Spend it with a beneficial **+** in the Esc menu. Unavailable and capped upgrades
are disabled; explosion radius remains locked until its reward and gravity stays 12.
The menu shows numerical before/after values, milestone progress and fountain rules.
Purchased ranks and collected base bonuses are separate, so purchase/reward order
produces identical final stats.

| Expedition upgrade | Per point | Rank cap |
|---|---|---:|
| Walking speed | ×1.10 from 4.725 |4|
| Fireball damage | ×1.15 on 15 plus collected Troll +25 |5|
| Fireball / Shield launch speed | Separate ×1.10 ranks on 45 plus each collected bonus |5 each|
| Fireball knockback | ×1.15 from 12 |5|
| Each spell cooldown | ×0.85 from its starting cooldown |5 each|
| High Jump height | ×1.15 from 4, capped at 8 |5|
| Unlocked explosion radius | +0.15 from 2.5 |5|
| Shield dimensions |5×5 →6×5 →6×6 →7×6 →7×7, then Golem +2×2 |4|

The glider is available immediately: press **G** airborne, and steer with mouse look.
It preserves momentum when opened or folded, gains speed in a dive, loses speed
quickly when climbing, and slowly slows in level flight. Its cap is 32 units/s and
steering pitch is 60° down to 30° up. Lift fades below 8 units/s; near 4 it descends
strongly but stays open so a dive can recover. Landing, water, blocking collisions,
starting Fireball/Shield charge, or High Jump fold it. Repeated airborne High Jump
remains available on its ordinary cooldown; reopening the glider requires G.
Pause preserves flight; death and Restart clear it.

The Troll casts fireballs and supports nearby forest minions. Damaging it calls all
surviving Goblin parties toward the ancient grove along authored paths; they still
need ordinary perception to find the player. Defeating the Troll ends the rally.
The Shadow uses ordinary AI homing; a destroyed wall or open gate allows real
movement and knockback beyond its original arena. All Dragons exist from the start;
the intended summit route passes the lower encounters, while off-route approaches
remain possible with High Jump and gliding. The summit Dragon has a distinct violet/icy
palette, 330 HP, 60 total breath damage, 65 bite damage, seven-unit breath range and a
60° cone. Ordinary Dragons retain 220 HP, 45/50 damage, six-unit breath and a 50° cone.

Defeating all 127 enemies marks victory and leaves exploration, casting and uncollected
rewards available. All credited kills yield 432 XP: level 8 with 109/171 XP and seven
upgrade points in total. Pause preserves the run. Restart restores terrain, objects,
enemies, initial
stats, level 1, zero XP, locked rewards and unused fountains. Runs do not persist across
launches. The older 26-actor Forest package remains compatible through `--forest-world`.
See [expedition content](../../assets/config/v4/forest-massif/expedition/README.md)
for reproducible package compilation and verification.

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
| Shadow |100| Charged Fireballs and Shield, configurable reaction delay, and shared High Jump for escaping holes. |
| Dragon |220; summit 330| Aggressive pursuit and mouth-origin breath/bite above one-third HP; escapes when critical. |
| Goblin |50| Spreading melee groups with high jumps and a short, telegraphed 12-damage swipe. |
| Shaman |60| Less aggressive Fireballs, permanent stone Shields and a timed healing/damage aura for its Goblins. |
| Golem |320| Slow seven-hex stone body, broad nearby slam, a sustained tracking laser and a frontal cover-clearing swipe. |
| Ember Wisp |30| Small glowing flyer with a long-range Ember shot; fragile alone and dangerous in groups. |
| Worm |320| Four long native hex segments, underground pursuit and an exposed-head Boulder followed by retraction and repositioning. |

Shadow attempts nearby walking, normal-jump and High Jump exits while preserving
its combat charge. Recovery is bounded and requires supported landings; it cannot
escape every deep or enclosed crater. See [arena bot behavior](arena-bot.md) for
observation limits and the comparison controls.

Goblins retain the legacy .8-unit body and .25-unit radius; the expedition player is taller. Groups approach
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
body clearance checks. Ordinary pursuit flight is six units/second. Taking damage
alone does not trigger retreat: Dragons escape at or below one-third HP and resume
aggression if healed above that threshold. Visible pursuit continues beyond the old
home leash; losing sight uses dated observations and a bounded search. When a target
is directly underneath, the Dragon repositions into a valid mouth-origin attack
rather than remaining stationary. It regenerates 3 HP/second after four seconds
undamaged, so chasing an escaping Dragon can prevent recovery.

The transparent Dragon panel lasts four seconds or until its 60 HP is depleted.
It blocks direct attacks from either direction but lets bodies and sight pass.
It has a 20-second cooldown. Explosions can splash through it, as they do through
ordinary stone cover.

Shaman support lasts five seconds after a visible cast, with a 12-second cooldown.
Visible party allies within nine units receive 3 HP/second and 25% extra actor
damage. It excludes the Shaman; leaving range or sight stops support, and killing
the Shaman ends the aura. Support does not stack or revive enemies.

The Golem has a seven-hex footprint and is five levels (2 units) tall. It walks at
3.2 units/second (below the player's 4.5-unit movement) and cannot jump or pass Fort's
low gate. Its .8-second slam windup
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
again. When a crater prevents ordinary shallow travel, the Worm retracts and tries
a deeper escape, at most eight levels below local terrain across its entire body.
It seeks at least 3.5 units of horizontal relocation and a valid shallow band before
emerging again. Buried escape travel is limited to four seconds; protected ground,
bedrock or no legal route can still force stationary counterfire. It never rebuilds
lost terrain or teleports. The optional six-segment body is refused where an authored spawn
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

Water remains physically non-solid with no swimming or drowning; required approaches
are initially dry. Expedition terrain and solid objects, including trees, bedrock,
bridge, arena and fountain vessels, support voxel carving. Only damaged cells vanish;
unsupported crowns and structures may remain suspended. Air, water and clouds are
unaffected. Carves update visible geometry, collision, sight and Shield placement at
one accepted world revision. Earned rewards relocate to surviving support, or near
the living player if no valid support remains. Initial package/spawn validation stays
strict; live destruction may invalidate routes. Ground enemies use bounded local
steering, so heavy destruction and unusual traps can defeat their routes.
The Forest–Massif expedition uses the accepted Grand visuals and has run-local
XP, upgrades and encounter rewards. Multiplayer and cross-launch progression saves
remain deferred.

## Deferred playtest issues — 2026-09-08

The player reports that Worms do not attack back and Golems fail to destroy
obstacles to continue chasing. Both are known unresolved issues, explicitly deferred
for this delivery. The final follow-up changes only Goblin pursuit; the described
Worm/Golem abilities above express their implemented intent, not successful playtest
acceptance.

### Response repair candidate — 2026-09-09

On `fix/arena-golem-worm-response`, Golems check locally obstructing destructible
terrain on the existing 10 Hz cadence without waiting for a complete movement
stall. Swiping still respects its cooldown, world damage admission and protected
terrain. Worms unable to finish diving after ground destruction retract and retry
stationary exposure after the existing surface interval, allowing sight-checked
counterfire. No above-ground movement or replacement terrain is granted. Travel
can still fail where no valid shallow band exists. These address specific causes
of the reported nonresponse; native playtest confirmation remains pending.

### Quick repair follow-up — 2026-09-11

The same local branch adds gravity-aware Shadow aim against jumping humans, raises
Golem movement to 3.2 units/second and tries the bounded deeper Worm escape above
before stationary fallback. Existing attacks and player controls are unchanged.
See the [focused validation report](../planning/waves/arena-bestiary/quick-combat-fixes.md).
