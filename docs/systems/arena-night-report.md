# Arena update

The **Worm is implemented** alongside Dragon, player-sized Goblins, Shaman, Shadow, Golem and Ember Wisps. Fort has selectable encounters, Seven Regions has three separate parties, and spectator mode lets two monster teams fight. The combined experiment is prepared for draft PR review; human playtesting remains pending.

The Worm has four segments, burrows just below the surface and converts eligible earth to dirt. It must expose its head before firing a damaging, knockback Boulder; its head turns gold while charging. It has **320 HP** and Boulders deal up to **70 damage**. Close shots now work without hurting the Worm, and cratered heads can find their surviving floor and continue attacking. Ordinary Fireball self-damage is preserved.

In the final sixteen machine battles, Worm beat five Goblins **6/8** times and beat Shadow **0/8**: five losses and three unresolved Fort timeouts. Golem beats Goblin groups but remains weak against Shadow. Twelve Wisps beat Shadow **3/4** times in a small Fort trial. These are rough machine comparisons, not human win rates.

Fresh Worm/Wisp images and the native build pass. All **29 combined checks pass**, including strict workspace lint, documentation and the shipping build. Seven Regions sustained all ten active enemies with real terrain destruction; its slowest measured simulation tick was **6.66 ms**, below the 8.33 ms budget. This synthetic check does not measure interactive FPS.

To try Worm, run `python3 tools/arena.py launch --map fort --encounter worm` from the repository and press **Enter**. **Esc/Tab** pauses, **R** resets, and the existing movement/charge controls remain. Choose **Spectate Battle** on the ready screen to watch two monster teams.

Human controls, animation readability and balance need playtesting. Heavy destruction can still strand simple local movement; rare Golem/Dragon CPU spikes remain. See the [controls guide](arena-encounters.md) and [detailed validation](arena-bestiary-validation.md).
