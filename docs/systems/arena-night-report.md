# Arena overnight update

The local prototype now has **Duel, Fort and Seven Regions**, plus spectator battles between two selected teams. Fort offers Dragon, player-sized Goblins, Shaman with Goblins, Shadow, Golem, Ember Wisps and Worm. Seven Regions has three separate encounters. Multiplayer remains deferred.

Launch with `python3 tools/arena.py launch`, then press **Enter**. Play with WASD and mouse, Space to jump, Shift to sprint, 1/2/3 to select spells and left mouse to charge/release. C switches first/close-third person; Esc/Tab pauses; R restores the encounter and terrain. Spectator mode has its own movable camera.

The **Worm** moves just below the surface, leaves dirt behind, raises its head before attacking, and has **320 HP**; its Boulders deal up to **70 damage**. We fixed close-shot rejection by making Boulders harmless to their caster, and fixed lost head exposure after explosions removed its old supporting floor. Fireball self-damage and shallow burrow limits remain intact.

In the final 16 machine battles, Worm beat five Goblins **6/8** times and beat Shadow **0/8**: five losses and three unresolved Fort timeouts. Golem handles Goblin groups but remains weak against Shadow. Twelve Wisps beat Shadow **3/4** times in the initial small Fort trial. These results do not establish human win rates; rare Golem/Dragon simulation spikes remain.

The final repository checks, fresh Worm/Wisp screenshots and remaining map-load measurement are in progress. These are not yet marked passed.

Human camera/control feel, animation readability and combat balance still need playtesting.

Heavy destruction can still trap the creatures’ simple local movement. See the [controls guide](arena-encounters.md) and [validation record](arena-bestiary-validation.md).
