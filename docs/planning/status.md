# Status

**This is the doc that is allowed to be out of date, and the only one.** Everything
else under `docs/` describes a contract; this describes a moment. If it disagrees
with the code, the code is right and this needs an edit.

What is *planned* is [roadmap.md](roadmap.md). What the game is *for* is
[the design](../design/game.md). This is the gap between them.

## What is built

A playable skeleton. Workspace boundaries enforced by Cargo, CI on three platforms,
a strict lint wall, dependency auditing, a state machine, and a RON content pipeline
that refuses to start on a bad file rather than defaulting past it.

The world is a voxel map with substances, destruction, and a deterministic
procedural generator: seeded recipes with validated crossings, anchors that scenarios
place units on by name, and architecture probes for frozen, volcanic, and sky-island
terrain. The sky-island probe now preserves a complete playable Hills map below a
separate flight-gated upper network. Movement is level-based over stacked surfaces,
with body size decided by headroom and a breadth-first pathfinder that cannot collapse
a stack. A movement preview draws the reachable set and the route before a click commits
to either.
Combat has two tempos, a turn order, engagement with hysteresis, and surface-aware
targeting where height buys range.

There are still **no abilities and no lattices**. Bodies are one hex wide; there is
no footprint for anything larger, and units do not obstruct each other — so a route
may be drawn straight through another piece.

## What is provisional

Everything in this table is a guess standing in for a decision that
[the design](../design/game.md) explicitly has not taken. **Do not tune these into
place** — they are meant to be replaced.

| Thing | Now | What it is waiting for |
|---|---|---|
| **Initiative** | a number on a component, high to low, ties by entity index | Derived from lattice size, per the design — which also solves boss action economy by giving a large lattice several slots |
| **A turn** | 4 hexes of movement and one action | The action-economy question. The design's current preference is 1–2 hexes plus an action |
| **Damage** | none at all | Lattices. Damage disables lattice hexes, and there are no lattices |
| **Enemy behaviour** | close the distance, swing | Lattices to know what it can cast, hidden information to know what it knows, a rout threshold to know when to stop |
| **Engage range** | 4 hexes, 6 to disengage | Nothing in particular. It is a feel question and wants playing with |
| **What height is worth** | +1 hex of range per 5 levels above the target | Abilities. The rule is real but has exactly one caller — engagement — until there are spells with ranges to apply it to |
| **How the tints look** | pale warm white, 0.22 alpha for range and 0.6 for the route | Nothing but taste. The constants are at the top of `hex_units::selection`; change the numbers rather than the structure |

**No randomness** is *not* provisional. The design is explicit that uncertainty comes
from hidden information rather than dice, so the turn order is deterministic: ties
break by entity index, and the same units always produce the same order.

### Why there is no damage

Damage disables hexes in a lattice. With no lattices, any damage model would be a
second system invented to be thrown away — and worse, it would fix numbers the design
has deliberately left open (how many hexes a spell disables, how long a fight runs,
whether functional death arrives before zero).

An attack is currently an animation and a log line. That is enough to see a turn pass,
which is what the loop needed to exist at all.

## Not built, and not next

Everything in [the design](../design/game.md#open-questions)'s open questions, plus:

- **Terrain that costs something to cross.** `Reach` charges one per step, so the
  shortest route is the one taken and breadth-first order is enough to find it. Mud,
  ice or a climb would each need a priority queue, and none of them are designed.
- **Units obstructing each other.** Two units can occupy the same surface, and the
  pathfinder will happily route one straight through another. An occupancy map over
  unit positions would fix both and lives entirely in `hex_combat`.
- **A way out of a stalemate.** A melee-only enemy separated by terrain it cannot cross
  stays in the fight forever: `approach` finds no route, so it spends its turn doing
  nothing, every round. Height makes this easier to fall into, since a fight now starts
  from further away when one side is above the other. Nothing is stuck — the player can
  still walk out past the disengage margin — but the enemy should give up rather than
  wait to be left. That is the rout threshold the design names and the enemy-behaviour
  row above is waiting on.
- **Multi-hex bodies.** `Body` has room for a footprint; the rule for whether a wide
  body may straddle a one-level step has not been decided.

## Not yet done, at the toolchain level

- **`bevy_lint`** is wired (`cfg(bevy_lint)` is declared, the `register_tool`
  attribute is in place) but unusable: it supports Bevy 0.18 at most, and this is
  0.19. Adopting it later costs no source changes.
- **Bevy feature trimming.** `default-features = true` still. The `3d` collection
  would cut compile time and binary size but risks silently dropping capability.
- **The animation system** is still `Box<dyn Transformer>` trait objects, which is
  why `Transformation` cannot derive `Reflect` and is invisible in the inspector.
  It works and is correctly frame-timed; it is the most likely thing to be
  rewritten when real gameplay lands.

## The production gap

Nothing that makes this a product exists yet: no saves, no settings menu, no audio,
no input rebinding, no crash reporting, no log files, and no signing or store
packaging. The full checklist, with the evidence behind each line and the crate
choices for closing them, is [production-audit.md](production-audit.md); the
sequenced work is the production-hygiene epic in [roadmap.md](roadmap.md).
