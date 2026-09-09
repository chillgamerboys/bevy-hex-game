# L3 — Observer controls creature effects and launcher

Read [approved scope](../plan.md), [manifest](../manifest.md), and [seams](../maps/seams.md). If reality disagrees with the map, escalate before crossing ownership.

1. Preserve accepted human controls, movement, spells and Shadow policy; Goblins match the player body.
2. Spectator has two autonomous monster teams and no dummy human; observations alone feed decisions, and arbitrary stable team IDs retain their meaning.
3. World publishes finite deployment surfaces; gameplay admits the entire roster atomically against actual body shapes, dry support and separation.
4. Finish and roughly calibrate original groups, then add Golem, Ember Wisp and Worm in that order; no Boar.
5. Golem uses a round seven-hex base and five levels of height, slow movement, spherical short slam and long charged laser with a medium-range gap.
6. Wisp is one voxel, slow flying, conspicuously glowing and long ranged; Worm burrows, converts eligible earth to dirt, and must expose its head before boulders.
7. Everything remains local on experiment/spell-combat-arena; no PR, merge, visible launch, multiplayer, adaptive difficulty or unrelated visual work.

Own only: crates/hex_game/src/arena/, tools/arena.py. Root alone stages and commits. No Cargo/capture/timing concurrency; claim the slot first.

Foundation must be committed before production work. Implement original spectator support first; later creature phases are explicitly dispatched after the previous phase is tested and calibrated. Keep focused tests meaningful and report failures, runtime limitations and exact evidence.
