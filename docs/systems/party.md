# Party controls and formations

The player party is a stable, session-scoped roster of at most six `UnitId`s. Its
order is the interface contract: the Party component and number keys `1` through `6`
address that order, never query or entity order.

Combat owns selection while a player unit has the turn. The acting player is selected
and focused automatically. HUD inspection is a separate presentation fact and never
redirects selection, the acting unit, caster, command owner, or formation anchor.
Outside combat, exactly one live player remains selected by gameplay authority.

## Party UI contract

Party order is presentation identity, not merely input order. Every ally heading starts
with `ALLY n` from the stable `Party.members` slot and then names the archetype and
session unit id. Initiative changes, selection changes, and matching hostile
archetypes never change that slot.

On Standard/Wide, Party is visible by default as one compact ordinary HUD component;
`P` toggles its persisted preference. On Compact it contributes no collapsed rail or
handle: `P` opens one temporary full-screen Party task and the same key or Escape
closes it. While master-hidden, `P` summons Party alone without changing the saved
preference.

Each card reports stable slot and identity, downed/ready state, gameplay selection,
and formation-anchor membership. It includes a small non-interactive lattice
silhouette for recognition, never the readable lattice or cell controls. Readable
character detail belongs to `MainViewDestination::Character`.

The first activation of a card or its number key publishes a presentation-only
inspection subject and one-shot Map-camera center request. Activating that same member
again opens its Character Main View. Character camera mode may follow the inspected
subject, but neither activation changes `Selected`, `Turn`, casting, command ownership,
or formation assignment.

Formation editing is a separate Main View destination opened with `F` during
Exploration. It keeps Group/Solo, presets, and the assignment grid together
instead of embedding an always-visible editor beside the map. The two movement modes
remain:

- **Group** applies a destination to the whole formation atomically.
- **Solo** preserves the ordinary selected-unit `MoveAlong` behavior.

Formation presets are content in `assets/config/formations.ron`. Each has one to six
unique, connected axial slots and exactly one anchor. Compact, Column, and Wedge ship
as the initial set. Picking a preset assigns members in stable party order. Picking a
slot assigns the selected member; an occupied slot swaps its occupant into the
selected member's old slot. Any unassigned members fill remaining authored slots in
stable order.

`PartyFormation` contains the preset, assignments, facing, and Group/Solo mode.
Entering an ordinary new Campaign resets it and spawning a roster selects Compact
when available. Its bound Campaign record serializes this same vocabulary and
restores it only when the explicit slot, build, content, roster, and terrain contract
still match; a new Campaign never inherits another slot's formation.

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

The eligible Action Bar exposes Rest and the `R` shortcut while Exploring. Both emit one
`GameCommand::Rest`; the applier validates that its issuer belongs to the party and
then recovers every roster member in stable order.

Rest restores every disabled lattice cell, removes `Downed`, fills live unlocked gems
to authored capacity, and removes persistent effects targeting party members. Active
enchantments, their locks, and locked mana remain intact. Enchantments already broken
by damage are not recreated. Positions, terrain, the encounter roster, and downed
hostiles are untouched, and one structured `Rested` event records each member's exact
cells and refilled mana.

## Test boundary

Party Trial on the authored Crossing is the human integration test for the full
three-member rail, formation editing, compression, reformation, terrain entry into
combat, and six-unit initiative readability. Focused automated combat checks use the
flat Ability Lab and Raider Mirror definitions only through default-off test support.
This keeps a spell, decision, or identity regression from being hidden behind bridge
routing or unrelated AI turns; the Crossing remains responsible only for party
dynamics that smaller deterministic cases cannot represent.
