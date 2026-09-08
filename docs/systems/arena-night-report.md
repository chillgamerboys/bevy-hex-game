# Arena overnight update

The local prototype has three maps: **Duel**, **Fort**, and **Seven Regions**. Fort offers a Dragon, five Goblins, a Shaman with three Goblins, a Shadow, a Golem, four Ember Wisps, or a Worm. Seven Regions has three separate encounters. The Worm passes its gameplay, real-map and application checks; native comparison and visual review are next.

Start with `python3 tools/arena.py launch`. Combat waits at the start screen until **Enter**. This remains a local game; multiplayer is deferred.

- **Play:** WASD and mouse; Space jumps, Shift sprints. Select Shield, Fireball or Area Blast with 1/2/3, then release left mouse to cast. Shield/Fireball charge for up to .75 seconds. C switches first/close-third person.
- **Spectate:** choose Fort or Duel and two teams. Available rosters include the original groups, Golem, one Goblin, and 1/2/4/8/12 Wisps; Worm is also available. WASD pans/moves, Q/E changes height, the wheel zooms, and C switches orbit/free camera.
- **Both modes:** Esc or Tab pauses and frees the mouse. R restores terrain and the same encounter at the ready screen. The paused menu has fullscreen and quit.

Machine battles provide rough tuning, with strong map and matchup differences. The Golem crushes Goblin groups but remains weak against the Shadow. Wisps are fragile alone; twelve beat the Shadow in **three of four initial Fort trials**, while four lost all four. These small automated samples do not predict human win rates. Rare Golem/Dragon simulation spikes remain.

**Remaining checks:** Worm native calibration and 13 fresh views; the repaired Wisp warning and 12 fresh views; final combined gates and the remaining world-load check. These are pending in this draft. Human control feel, animation and combat balance still need playtesting.

See the [controls guide](arena-encounters.md) and [validation record](arena-bestiary-validation.md) for details and historical evidence.
