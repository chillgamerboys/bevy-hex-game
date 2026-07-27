# Combat

The turn loop as it is built: the two tempos, what a turn costs, how a move is
committed, and what elevation buys.

Read [the design](../design/game.md) for the game this is heading toward — **most of
it does not exist yet**, and the gap is deliberate rather than a backlog. Which of
the numbers here are placeholders, and what each is waiting for, is
[planning/status.md](../planning/status.md).

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

## Saying no out loud

Clicking a tile can fail for five different reasons — not your turn, nothing standable
there, no route, further than the movement left, not in gameplay at all — and every one
of them used to look the same from outside: nothing happened. A rule was
indistinguishable from a bug, which is a bad property for a game to have and a worse
one for a prototype nobody has a manual for.

So the answer is drawn **before** the click rather than inferred after it:

| | |
|---|---|
| **a ring** | at the feet of whoever is acting — the selection, out of combat |
| **a faint tint** | over every surface this turn's movement can pay for |
| **a stronger tint** | along the route to whatever the cursor is over |

A tile that cannot be reached is simply not lit, and hovering it draws no route. The
HUD carries the same fact as a number — `your turn, 4 to move` — so the tint can be
checked against something rather than merely trusted.

**There is no range tint while exploring.** Movement is unlimited there, so every
connected surface qualifies and a tint over the whole map would say nothing. The route
preview still draws, which is the half that carries information.

Both tints come out of **one** search per selection, not one per hovered tile:
`Reach`'s keys are the range and a walk back down its predecessors is the route. What
costs something is rebuilding `Footing`, which reads every tile entity on the map — so
that happens when the selection or its position changes, never when the cursor moves.

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

Each entry in `assets/config/scenarios.ron` places its player and enemy. Move them onto
whatever part of that scenario's map is worth testing, press `BACKSPACE`, then click
the scenario to rebuild. See [development/config.md](../development/config.md).

## Where a unit is, and where it is going

Two different facts, and they used to be one component. `StandsOn` is the surface a
piece is **actually on**; `MovingTo` carries the route it has committed to walking.

Conflating them meant everything that asked where a unit was got the answer it would
have *eventually*. A click across the map started a fight instantly at the far end of
the route, ended one while the piece was still walking away, and could pass straight
through engaging distance without noticing whenever the destination happened to be out
of range.

**A fight starting mid-walk stops the walk.** The piece is put down on the nearest
whole step of its route — never between two hexes, because a position between surfaces
is not something any other rule can express. Committing to a long walk and then being
ambushed halfway should leave you where the ambush happened, not deliver you somewhere
chosen before anyone knew there was a fight.

The route is kept as the whole path rather than just its endpoint precisely so that
logical position can advance at each completed leg and interruption has somewhere real
to put the piece. `HexPathingLine` and `MovingTo` share cumulative, world-space leg
durations, so climbs take their actual 3D travel time while every waypoint still maps
back to its surface.

## The high ground

Elevation is an **advantage, not a separation**. A unit gains one hex of range for
every 5 levels it stands above its target, and the unit below gains nothing back.

That asymmetry is the whole mechanic, and it is why there is no "distance between two
surfaces" anywhere in the code. There cannot be one: the answer depends on who is
asking. `hex_units::targeting` exposes `in_reach(from, to, range)` instead, and
engagement is its first caller — a fight starts when *either* side can reach the other,
because being shot at without being able to shoot back is still a fight.

**Melee is exempt.** A spell has *range* and gains from height; a fist has *reach* and
does not, or an attacker five levels up would acquire a two-hex punch. Swinging stays
an adjacent-surface step that the attacker's `TraversalProfile` admits in both
directions. Requiring both directions keeps melee symmetric even if a future profile
can drop farther than it climbs.

**Two units at one coordinate are not far apart**, however tall the column between
them. Horizontal separation is genuinely zero and someone directly overhead can act on
you. The stacking rule governs where you can *walk*; it does not make people invisible.

That last one was raised in review as a collapsed stack and kept anyway, deliberately —
see [architecture.md](../architecture.md#ownership-cuts-both-ways) for why a design call
inside this crate is settled by its owner. If it turns out to play badly, the thing to
change is this rule, not the reading of it: both readings were defensible and the
argument is recorded on PR #46 rather than lost.
