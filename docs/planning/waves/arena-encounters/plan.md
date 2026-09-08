# Approved encounter implementation specification

Approved by Alberto on 2026-09-07. Values below are initial fixed tuning hypotheses. Preserve the accepted Shadow challenge and Duel behavior from 127d1ce.

## Playable loop and maps

Start screen adds Duel, Fort and Seven Regions. App defaults Fort/Dragon; core defaults Duel for fixtures. Fort presets: one Dragon, five Goblins, one Shaman + three Goblins, one Shadow. Seven Regions: Dragon at mountains_high_pass, Shaman+3 Goblins at fort_fort_courtyard, five Goblins at caves_cave_entrance. Accepted seeds: Fort 640367719, Seven Regions 703700113. Derive safe dry spawns near anchors matching actual bodies and connected approaches; human starts beyond every party's 12-unit activation distance.
Defeat every party to win; player death always ends run. Restart restores selected recipe, terrain, all actors, abilities, cues and parties. Dead parties do not respawn.
Map-only human regeneration: 2 HP/s after 8 s without casting, dealing or receiving damage and 3 s unseen by active enemies. Original Duel no regeneration. Water unchanged nonsolid, submerged-bed walking; no swimming, drowning or recovery teleport. Required routes dry.
Grand V3, multiplayer, progression, loot, saving and other maps deferred.

## Profiles

World units/second; vertical voxel .4, horizontal hex ~1.75. All new values in arena configuration.
Shadow: unchanged 100 HP, current M01, three spells and accepted bot. Encounter wrapper may activate/return; Duel decisions, timing, seed and physics unchanged.
Dragon: 100 HP, ground 7.5, flight 3, cruise 2 above ground with clearance. Literal body .4 high x 3.5 long x one hex wide; collision matches elongated body, wings decorative. Fire cone: 35 total HP over .75 s in three pulses, range 4, full angle 50 degrees, cooldown 3 s. Bite: 50 HP, one horizontal hex reach from mouth, .25 s windup, 1.8 s cooldown. Prefer landing to pursue; can breathe airborne. Slow flight patrol/search, simple observed-target approach/obstacle avoidance. No Shadow tactics. Retreat 4 s after damage, or 8 s below half HP; new damage refreshes retreat. Regenerate 3 HP/s after 4 s undamaged.
Dragon temporary barrier: fixed 2.5 units ahead of mouth toward perceived threat; two hexes wide, four levels high, 60 HP, 4 s lifetime, 20 s cooldown; air placement allowed. Transparent LOS; actors/cameras pass. All direct attacks both directions collide and damage it, including allies. Explosions remain unoccluded. Gameplay-owned dynamic object, never terrain deletion on expiry.
Goblin: 50 HP, walk 3.5/run 6. Swipe 12 HP, one hex reach, 90-degree forward arc, .25 s windup, 1.2 s cooldown. Local pursuit/separation/varied approach angles; no advanced Shadow brain.
Shaman: 60 HP, walk 3.5/run 5. Existing Fireball 35 maximum HP, 2.5 s cooldown, .52 s charge, twice Shadow aim spread, .35 s reaction. Recheck visibility/self-splash safety before release. Existing permanent Shield with 8 s cooldown when threatened/recently damaged; no Blast. Stay behind party.
Aura: .5 s cast, 6-unit radius, 5 s duration, 12 s cooldown. Heal 3 HP/s and increase actor damage 25% (not terrain). Alive own-party allies only, exclude caster, require LOS. No stacking; leaving range/LOS loses benefit; caster death ends aura. Cast for an injured eligible ally or at least two engaged allies. Freeze projectile damage multiplier at release; never mutate shared player tuning.

## Shared attacks and parties

No allied actor HP damage or knockback. Enemy projectiles including Shield seeds skip allied bodies; preserve caster clearance/self-hit/self-splash. Exclude friendly projectiles/cues from hostile threat memory. Attacks freeze owner ID/team/multiplier even after death. Dead actors cannot release queued abilities, heal or revive.
Terrain/barriers block bites/swipes/cone. Contacted exposed voxels receive damage; actors/cells behind do not. Terrain power: 1 swipe, 2 bite, 2 per complete breath. Existing elemental spells unchanged. Each swing hits actor/voxel at most once; three breath pulses with per-cast actor and voxel caps. No persistent burning or additional melee knockback.
Party states Dormant/Active/Returning/Cleared. Sight activation at 10 Hz when any member sees human within 12 units, omnidirectional and terrain blocked. Positive player damage wakes party immediately with coarse observed cue, never hidden current player position. Share observations within party only; no automatic alert between parties. Dormant physics/damage and local patrol continue.
Home leash/search after LOS loss: Goblin/Shaman 18 units/4 s; Shadow 24/6; Dragon 32/8. Search expiry or home limit crossing triggers return. Returning enemies can defend locally without extending pursuit. Preserve HP; no respawn/full heal. Ground controllers use bounded local rollouts with melee/ranged goals and staggered decisions/routes. No general navigation or stuck teleport. Required routes dry.

## Foundation and delivery

Retain ActorIntent, 120 Hz charging and world publication ordering. Stable IDs/species/team/party/max HP/body/per-actor brains, multi-body separation, victory and telemetry. Previous/current poses and shapes in sweeps, forecasts, shield clearance; swept flight and safe turning with independent knockback. Terrain/Barrier/Actor earliest-hit distinction. Forecasts never receive hidden live actors.
World publishes geometry offset/bounds/anchors/solid runs/dirty columns/static query spans/liquids/edit protection. Duel upper face level*.4; real maps (level+1)*.4. TerrainImpact kind Elemental/Physical; explicit physical material admission, existing protected materials and elemental behavior unchanged. Dirty-column refresh before next physics; bounded blast selection.
Simple distinct voxel creatures, mouth origins, windups, breath, barrier, aura, cleared-party count. No hidden-enemy indicators or opaque-wall fading. Static authored objects remain indestructible with conservative protected support; existing voxel material HP retained.
Tests: Duel regression; multiple bodies, rotation, flight/ceilings/landing/knockback; ranges/windups/cooldowns/caps/terrain/occlusion/friendly pass/self-damage; barrier first hit/single detonation/splash/expiry; aura/regeneration/death; party wake/hidden-information/return/independence. Dry continuous-controller routes all three encounters and back; destruction/blocking; pause/focus/KO/victory/reset/map switching/reentry.
Measure loading/frame percentiles/destruction/120 Hz budget with all ten enemies active. Focused tests, selector combined gate, strict workspace lint, native build, fresh windowless captures. Deliver local commits, launcher, controls/config guide and validation report. Human balance/feel remains pending 20–30 encounters by Alberto. No automatic PR/dev merge or unrelated visual work.
