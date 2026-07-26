# The gameplay loop

What is built, what is a placeholder, and which unresolved design question each
placeholder is standing in for.

Read [DESIGN.md](DESIGN.md) for the game this is heading toward. **Most of it does not
exist yet**, and the gap is deliberate rather than a backlog.

## Two modes, one map

```
                  hostile within 4 hexes
    Exploring ─────────────────────────────► Combat
        ▲                                       │
        └───────────────────────────────────────┘
                  no hostile within 6 hexes
```

Like Baldur's Gate 3: real time when nothing is happening, turns when something is.
There is **one map** and one set of units either way — this is a change of tempo, not
a change of place.

`Mode` is a `SubStates` of `Screen::Gameplay`, alongside `Pause`, so "in combat on the
title screen" is unrepresentable rather than merely unlikely.

The two thresholds differ on purpose. Without a margin, a unit sitting exactly on the
boundary would flip in and out of combat every frame it drifted.

## A turn

One unit holds a `Turn` at a time. It carries a movement budget and whether the action
has been taken. Ending it passes the marker to the next unit in the order; running off
the end wraps and counts a round.

**A turn cannot end while its unit is still moving.** The removal of the
`Transformation` component is what "finished moving" means, and advancing before then
would cut the animation off and strand the piece between two hexes.

Keys: `SPACE` ends a turn. `ESC` and `BACKSPACE` were already taken by pause and
quit-to-title.

## What is provisional

Everything in this table is a guess standing in for a decision that
[DESIGN.md](DESIGN.md) explicitly has not taken. **Do not tune these into place** —
they are meant to be replaced.

| Thing | Now | What it is waiting for |
|---|---|---|
| **Initiative** | a number on a component, high to low, ties by entity index | Derived from lattice size, per the design — which also solves boss action economy by giving a large lattice several slots |
| **A turn** | 4 hexes of movement and one action | The action-economy question. The design's current preference is 1–2 hexes plus an action |
| **Damage** | none at all | Lattices. Damage disables lattice hexes, and there are no lattices |
| **Enemy behaviour** | close the distance, swing | Lattices to know what it can cast, hidden information to know what it knows, a rout threshold to know when to stop |
| **Engage range** | 4 hexes, 6 to disengage | Nothing in particular. It is a feel question and wants playing with |

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

## Vocabulary

**Lattice**, not "core". The design notes call a character's hex grid their core;
`hex_core` is already the crate every other crate depends on, and the collision would
be permanent and confusing in exactly the places it matters — an agent deciding which
crate a new type belongs in.

A crystal lattice is a structured arrangement of gems, which is what the thing is, and
it carries the connectedness that adjacency-based power depends on. It reads correctly
everywhere: a twelve-hex lattice, lattice damage, a lattice hex disabled.

**Nothing in the code implements a lattice yet.** The word is reserved so it is
settled before it is load-bearing.

## Where it lives

| Crate | Holds |
|---|---|
| `hex_core` | `Mode` and `Turn` — shared because `hex_combat` writes them and `hex_units` reads them, and neither can see the other |
| `hex_units` | Bodies, positions, factions, where a unit may step |
| `hex_combat` | The turn order, engagement, and the placeholder AI |
| `hex_anim` | Moving a transform over time. Knows nothing about any of the above |

`hex_combat` depends on `hex_units` because a turn order is a fact *about* units. That
is a layer, not a sibling — the rule keeping `hex_map`, `hex_world` and `hex_units`
independent of each other still holds.

## Trying it out

`assets/config/scenario.ron` places the player and the enemy. Move them onto whatever
part of a map is worth testing and press `BACKSPACE` then `ENTER` to rebuild. See
[CONTENT.md](CONTENT.md).

## Not built, and not next

Everything in [DESIGN.md](DESIGN.md)'s open questions, plus:

- **A pathfinder.** `route` walks a straight line and gives up when blocked, so an
  enemy behind a wall stands still. `hexx::a_star` is compiled in and unused.
- **Units obstructing each other.** Two units can occupy the same column. An
  occupancy map over unit positions would fix it and lives entirely in `hex_combat`.
- **Multi-hex bodies.** `Body` has room for a footprint; the rule for whether a wide
  body may straddle a one-level step has not been decided.
