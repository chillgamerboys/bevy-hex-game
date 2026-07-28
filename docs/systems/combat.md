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

**The rules engine exists** as `hex_lattice`, and knowledge *about* a lattice
exists as the store below. **No unit carries one yet** — that is HEX-12 — so the
word is settled and the engine proven before either becomes load-bearing.

## What a faction knows

There are **two knowledge channels**, and conflating them is the mistake this
section exists to prevent.

| Channel | Answers | Owner |
|---|---|---|
| **Spatial perception** | *Where is that unit, and can I see it* | world owner, future `hex_perception` |
| **Lattice knowledge** | *What do I know about that unit's lattice* | `hex_combat::knowledge` |

**Seeing a unit reveals nothing about its gems, its fusions, or what it can cast.**
Observing establishes a position and permits targeting; divination is what reveals
contents. `FactionKnowledge::view(viewer, subject)` is **the** read path for the
second question — the AI included — and reading a hostile `LatticeState` directly
is a bug.

### Why the store is not keyed on visibility

The obvious implementation, *remember what a faction can currently see*, is the
one that has to be thrown away. Divination exists precisely to reveal what a
faction cannot see, so a fact may arrive from a cast with a lifetime of its own —
"revealed information decays or is one-time, unless the divination is an
enchantment" ([design/game.md](../design/game.md)).

So each revealed cell carries a `KnowledgeSource` (`Observation` or `Divination`)
and a `KnowledgeExpiry`, both in `hex_core::perception` because both channels need
the same words. An observation-sourced fact and a divination-sourced one differ in
nothing but those tags: the store does not care how a fact arrived, only that it
says so and says when it lapses.

`KnowledgeExpiry::Rounds(n)` counts down at each rollover; `Rounds(0)` **is** the
design's one-time reveal, so there is no separate variant that would decay
identically and drift. `Sustained` never decays on its own — an enchantment-backed
divination holds knowledge that way, and its writer owns the lifetime.

Decay reads `RoundElapsed` and is ordered `.after(CombatSystems::Advance)`, which
writes it. A local `.chain()` would look correct and race.

### What is public

A lattice's **shape** is public and its **contents** are not. Capacity is apparent
from looking at a character, so base visibility — faction and cell count — is
available with no reveal at all. That is what makes the "unknown lattice, N hexes"
readout honest rather than a placeholder.

### Wired, and what is still missing

Units carry lattices. `lattices.ron` holds the archetypes, `spawn_units` attaches
a `LatticeSpec`, `LatticeState` and `LatticeStats` keyed by the unit's
`Archetype`, and the publishing systems that matched nothing now populate the
store every frame in gameplay. `view()` returns real base visibility.

Two things it does not yet do. **Nothing draws a hostile lattice** — the store
fills, the dev reveal-all toggle fills it further, and no UI renders either, so
the readout in the HUD is your own party's hex count rather than anything about
the enemy. And **nothing writes divination-sourced knowledge**, because casting
does not resolve yet; `Reveal` is still refused with a reason.

`Reveal` (the shipped "Scrying Eye") reaches the store through the cast path,
which HEX-12 also lands. A cast must still anchor on a currently Observed
position — the rule is [absolute, including for divination](casting.md#observation),
because `Reveal` targets a unit you must already see.

The dev reveal-all toggle is `K`, behind the `dev` feature: the shipped build has
no key that exposes hidden information, since hidden information is this game's
source of uncertainty rather than dice.

## Where it lives

| Crate | Holds |
|---|---|
| `hex_core` | `Mode`, `Turn` and `RoundElapsed` — shared because `hex_combat` writes them and `hex_units` reads them, and neither can see the other. Also `KnowledgeSource` / `KnowledgeExpiry`, which both knowledge channels need |
| `hex_units` | Bodies, positions, factions, where a unit may step |
| `hex_combat` | The turn order, engagement, the placeholder AI, and the lattice-knowledge store |
| `hex_lattice` | The pure rules engine. `hex_combat` depends on it because knowledge *of* a lattice needs the lattice vocabulary — and because `hex_core → hex_lattice` is the dependency direction, so the store cannot live in `hex_core` |
| `hex_anim` | Moving a transform over time. Knows nothing about any of the above |

`hex_combat` depends on `hex_units` because a turn order is a fact *about* units. That
is a layer, not a sibling — the rule keeping `hex_map`, `hex_world` and `hex_units`
independent of each other still holds.

## Trying it out

Each entry in `assets/config/scenarios.ron` names an encounter file under
`assets/config/encounters/`, and that file holds the roster: who is on the map, by
archetype, and where each unit starts. Move them onto whatever part of that scenario's
map is worth testing, press `BACKSPACE`, then click the scenario to rebuild. Adding a
second unit to a side is one line in the roster. See
[development/config.md](../development/config.md).

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
