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

