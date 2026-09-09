# Creature pressure follow-up

Authorized by the user's 2026-09-08 playtest feedback. Continue the existing
arena-bestiary wave and PR #221; no merge is authorized. The accepted human
controls, Shadow policy and world mutation ownership stay intact. Earlier
calibration describes earlier rosters and behavior, not this tuning pass.

## Outcome and agreed boundaries

- Wisps attack obstructing cover toward their own recent last sighting, then seek
  another angle. Never read a hidden target's current position for this behavior.
- Healthy Dragons occasionally make a swept approach burst toward a disclosed
  distant opponent. Existing attacks, collision and damaged retreat still apply.
- All Goblin recipes contain ten; all Shaman recipes contain a Shaman and five
  Goblins, including Seven Regions (17 enemies total). Use wider approach angles,
  periodic supported high jumps and hole recovery. Escorts stay near their Shaman;
  support radius starts at nine world units. Health and ordinary attack damage stay.
- Golem beam lasts four seconds, with 180 maximum total damage (45 per second),
  existing two-second charge and eight-second cooldown. Bounded turning follows
  own sightings. During lost sight it extrapolates that same target's last observed
  velocity for the rest of the cast. Add a separate short frontal terrain-clearing
  swipe for lack of movement progress; exclude its footing and protected terrain.
- An activated, physically buried Worm may know hostile positions across the map.
  This explicit species exception never supplies information to other enemies.
  Above ground it can aim attacks only from actual sight. Hostile damage refreshes
  pursuit and triggers retraction; travel is underground, with exposed attack and
  burrow/reposition cycles. Keep terrain acknowledgement and collision admission.

## Integration and ownership

Root is the integration owner and sole Cargo/native-build owner. A temporary local
worktree on `experiment/arena-creature-pressure` allows gameplay work while the
Battle Mode entry is captured from a clean candidate. Its commits will be applied
additively to `experiment/spell-combat-arena`; there is one combined PR to `dev`.

- Worm lane: `encounters/worm.rs`, Worm tests, and narrow Worm observation,
  dispatch and damage hooks in `encounters.rs`. Root changes roster construction
  before this lane begins. No new world mutation or generic sensing API.
- Golem lane: `encounters/abilities.rs`, `encounters/golem.rs`, new Golem brain
  helper and tests, and creature ability enum/snapshots. Coordinate shared Brain
  hooks with its sole editor; do not edit `brain.rs` independently.
- Creature brain lane: `encounters/brain.rs`, Wisp helpers/tests, local steering,
  movement/controller profiles and Dragon/Goblin/Shaman tests. Preserve the
  default human/Shadow movement values. Coordinate the Golem helper call and the
  Wisp release helper signature before Golem's ability edits.
- Root: authored/default tuning and validation, roster recipes, world/application
  composition checks, launcher receipts and presentation adapters. No lane runs
  Cargo or launches a window. Tests run after integration into the retained build
  checkout so the duplicate worktree does not create a second build cache.

## Required evidence

Focused species regressions cover hidden-information boundaries, cooldown/damage
caps, swept movement and terrain ordering. Actual-map checks admit the larger
rosters, verify attacks, reset, independent parties and bounded all-active work.
Run the repository-selected combined gate and strict workspace lint. Capture
changed ability and menu presentation windowlessly at a clean committed revision;
native feel, animation and balance remain human playtest judgments.


## World deployment prerequisite

The ten-Goblin recipe requires more supporting cells than the original seven-cell
battle pockets. The bounded world lane owns `hex_map/src/arena/worlds.rs` and its
world tests for this prerequisite. Ordinary Duel and Fort deployment now publishes
only existing open, dry, unreserved ground within radius two of the same preferred
supporting voxel. Every pocket must retain its original seven cells and contain
10–19 surfaces. Fort uses the elongated pocket's signed axial-r half rule, so the
two teams' surface sets remain disjoint.

Terrain generation, adventure anchors, world spawns, vertical mapping and the
separate elongated-body fit check are unchanged. Gameplay still admits full live
bodies atomically; the larger surface publication alone is not roster-admission
evidence. Wisp formation remains on the nearest seven surfaces and its existing
two flight layers. World tests cover bounds, dry/unreserved surface identity,
deterministic publication, preservation of the original cells and refusal of an
insufficient pocket. Root runs the actual-map roster composition checks.
