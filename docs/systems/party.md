# Party controls and formations

The player party is a stable, session-scoped roster of at most six `UnitId`s. Its
order is the interface contract: the strip and number keys `1` through `6` address
that order, never query or entity order. Selecting a member while exploring also
publishes that member's exact `TilePos` as the camera focus target.

Combat owns selection while a player unit has the turn. The acting player is selected
and focused automatically; number keys and the party strip cannot redirect commands
to another member. Outside combat, exactly one live player remains selected.

The party strip reports each member's stable id, archetype, live lattice cells, downed
state, selection, and whether that member occupies the formation anchor. It also owns
two explicit exploration movement modes:

- **Group** applies a destination to the whole formation atomically.
- **Solo** preserves the ordinary selected-unit `MoveAlong` behavior.

Formation presets are content in `assets/config/formations.ron`. Each has one to six
unique, connected axial slots and exactly one anchor. Compact, Column, and Wedge ship
as the initial set. Picking a preset assigns members in stable party order. Picking a
slot assigns the selected member; an occupied slot swaps its occupant into the
selected member's old slot. Any unassigned members fill remaining authored slots in
stable order.

`PartyFormation` contains only current-session choices: preset, assignments, facing,
and Group/Solo mode. Entering a new gameplay session resets it, and spawning a roster
selects Compact when available. Save/load may serialize the same vocabulary later,
but no formation choice persists between sessions today.

The miniature grid is an editor as well as a readout. A diamond marks the authored
anchor. Its slot positions are the actual axial offsets, so changing a preset's
content changes the displayed shape without a parallel UI layout.

## Atomic group traversal

A Group click first routes the assigned anchor exactly as an ordinary walker. For each
anchor segment, the planner derives its sextant and rotates the authored offsets from
their `A` orientation. Members are placed anchor-first and then in authored slot order.
Each member uses its own `Body` and `Footing`.

The candidate ladder is deterministic:

1. the member's ideal rotated slot, choosing the closest vertical surface;
2. an unused recent exact surface on the anchor corridor; then
3. another standable surface within the party's maximum compressed spread, ordered by
   distance to the ideal coordinate, vertical difference, and `TilePos`.

Every candidate must have a complete route from that member's prior planned surface.
Destinations are unique at every anchor segment. This lets a formation collapse into
a single-file bridge corridor and reclaim its ideal slots when open ground returns,
while a member that cannot remain within the safe compressed footprint rejects the
complete plan.

The emitter queues one `MoveParty` containing every exact `PartyPath`. The combat
command funnel independently regrounds all paths against live terrain and validates
the active anchor, party membership, complete member coverage, unique members, seats,
starts, traversability, busy state, and unique final destinations. It inserts no
movement component until every member passes. Member paths may cross because global
unit obstruction remains deferred, but they cannot finish together.

If combat begins during presentation, the existing movement interruption reconciles
every moving party member to its nearest whole route surface before turn construction.
Solo mode bypasses this planner and continues to emit one selected-unit `MoveAlong`.

## Exploration rest

The party strip exposes Rest and the `R` shortcut while Exploring. Both emit one
`GameCommand::Rest`; the applier validates that its issuer belongs to the party and
then recovers every roster member in stable order.

Rest restores every disabled lattice cell, removes `Downed`, fills live unlocked gems
to authored capacity, and removes persistent effects targeting party members. Active
enchantments, their locks, and locked mana remain intact. Enchantments already broken
by damage are not recreated. Positions, terrain, the encounter roster, and downed
hostiles are untouched, and one structured `Rested` event records each member's exact
cells and refilled mana.
