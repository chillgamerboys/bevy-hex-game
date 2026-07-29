# Combat

The turn loop as it is built: the two tempos, what a turn costs, how a move is
committed, and what elevation buys.

Read [the design](../design/game.md) for the game this is heading toward. The playable
combat core exists, while party-scale behavior, outcomes, and several policy choices
remain deliberately open. Which of the numbers here are placeholders, and what each
is waiting for, is [planning/status.md](../planning/status.md).

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

Keys: `SPACE` ends the current player turn and is ignored while a hostile owns the
turn. `ESC` and `BACKSPACE` were already taken by pause and quit-to-title.

### A defender choice is command-modal, not Pause

When damage opens `PendingDecision::ChooseDisables`, the command applier rejects every
simulation command except a `ChooseDisables` answer for that exact defender. Input
emitters also stop producing movement, casts, end-turn commands, and AI actions while
the decision is open, keeping the refusal log quiet during ordinary play.

A `Player` defender uses the own-lattice panel: only live cells are buttons, additional
picks stop at the owed quota, Clear removes the local selection, and Confirm or `ENTER`
emits the answer. If every live cell is owed, all are preselected but confirmation is
still explicit. Non-player defenders use the deterministic policy and issue the answer
under their own `ControlOwner`.

This is not the `Pause` state. Camera and ordinary UI keep running. `H` hides ordinary
readouts but deliberately leaves an active decision lattice and its controls visible.

Every accepted outcome and refusal is also a public, serde-capable `CombatEvent` or
`CommandRefusal`. Those contracts use stable unit ids, spell names, positions, and exact
lattice-coordinate lists—never session-local spell ids or formatted presentation
strings. The combat log applies faction disclosure when it ingests each event, so later
divination cannot rewrite what an older line was allowed to reveal.

The one-decision seam is also an authoring boundary: a spell may contain at most one
non-targeted `DisableHexes` effect, because two would otherwise overwrite each other's
defender choice. Damage commands aimed at an already downed unit are refused before
spending action or mana; non-damaging inspection such as Reveal remains legal because
the retained lattice is the future restoration target.

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

**The rules engine exists** as `hex_lattice`, knowledge *about* a lattice exists as the
store below, and **units now carry one**: `spawn_unit` looks the archetype up in
`lattices.ron` and attaches a `LatticeSpec`, a `LatticeState` and the `LatticeStats` that
say what its gems hold. An enemy's lattice is its entire stat block, so that one lookup
is what makes a wolf a wolf.

## What a faction knows

There are **two knowledge channels**, and conflating them is the mistake this
section exists to prevent.

| Channel | Answers | Owner |
|---|---|---|
| **Spatial perception** | *Where is that unit, and can I see it* | world owner, `hex_perception` |
| **Lattice knowledge** | *What do I know about that unit's lattice* | `hex_combat::knowledge` |

**Seeing a unit reveals nothing about its gems, its fusions, or what it can cast.**
Observing establishes a position and permits targeting; divination is what reveals
contents. `FactionLatticeKnowledge::view(viewer, subject)` is **the** read path for the
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

### What observation publishes

`FactionMapKnowledge` is the sole authority for whether the subject is currently
observed. A gameplay-owned adapter copies only existence and faction into the lattice
read seam; it does not rederive sight from combat entities. Losing observation hides
the entire lattice view immediately. Any unexpired divination facts remain stored on
their independent clock and become readable again only if the subject is re-observed.

**Shape and capacity are hidden information too.** Until divination learns capacity,
`known_capacity()` and `unknown_count()` both return `None`; the target panel says only
`lattice unknown`.

### Wired presentation and Reveal

Units carry lattices. `lattices.ron` holds the archetypes, `spawn_units` attaches
a `LatticeSpec`, `LatticeState` and `LatticeStats` keyed by the unit's
`Archetype`, and the adapter consumes world perception every frame in gameplay.
`view()` returns base visibility only for currently observed subjects.

The target panel reads no hostile `LatticeSpec` or `LatticeState`; it projects only
`FactionLatticeKnowledge::view`. A complete Reveal (the shipped Scrying Eye) learns capacity
and every cell. While it lasts, already-divined cells refresh mana and disabled state
from live truth without resetting their expiry. Tier one lasts through the current
partial round and the next complete round, expiring at the following rollover.

A cast must still anchor on a currently Observed position — the rule is
[absolute, including for divination](casting.md#observation), because Reveal targets a
unit you must already see. `RevealAll` remains a separate live developer override, not
knowledge written into the store.

The dev reveal-all toggle is `K`, behind the `dev` feature: the shipped build has
no key that exposes hidden information, since hidden information is this game's
source of uncertainty rather than dice.

## Effects that outlast their cast

Damage over time, enchantment upkeep and decaying divination are the same system wearing
three hats, so [casting.md](casting.md#persistent-effects) builds it once, around one
shape:

```
{ source, target, payload, start, end }
```

The vocabulary is `hex_core::effects`; the runtime is `hex_combat::effects`; lattice
payloads go through `hex_lattice`'s existing functions. Today the only payload is
`Burn`, and it is a payload rather than a special case — what burning means can be
redefined without touching the framework.

### Two tick points, because tick point is per payload

| Hook | When | What ticks there |
|---|---|---|
| `tick_turn_effects` | start of the acting unit's turn, before `CombatSystems::Act` | personal payloads — today, `Burn` |
| `expire_round_effects` | on `RoundElapsed`, after `CombatSystems::Advance` | end conditions that can only come due on a round boundary |

**Burn is personal.** It ticks at the start of the *affected unit's* turn, not at the
round boundary — the design words fire's damage over time that way, and a round-boundary
burn would hit a unit that had just acted and one that had not at the same moment.

The tick is driven by each newly added `Turn`. A turn is many frames long, so anything
keyed on "the acting unit is burning" would fire every frame and empty a lattice in
about a second. Conversely, a real same-round handoff that adds another `Turn` is another
tick; round number is not a substitute for turns taken.

### Burn ignores armour, but not the defender

Two halves that are easy to conflate, and the design settles both.

A due burn **skips `resolve_incoming`**, the flat subtraction that defensive
enchantments apply. Fire's identity is beating defences by ignoring them rather than
overpowering them, so a shield that turns an ember into nothing does not slow a fire.

A due burn **still parks `PendingDecision::ChooseDisables`**, exactly as a spell's
damage does. Damage names a count; the defender picks which hexes. A fight replays by
re-running its commands, so a choice made inside the runtime and never written down
would be re-derived on replay rather than replayed — and burn would become the one
damage source a fight could not reproduce.

The seam holds one decision at a time, so a tick that comes due while another decision
is open **queues** rather than skipping itself or overwriting the open one. Both
alternatives lose damage silently.

### One countdown, and it is the ledger's

A burn is entirely a ledger entry — source, start round, end condition, and
`PersistentEffect::ticks`, the count of personal ticks that have fired. `is_live` is a
total function of the record; nothing else is consulted and nothing has to agree.

It was briefly built the other way, with a `Vec<Burn>` inside `LatticeState` ticked by the
rules engine, and the seam was wrong in both directions. A burn has a *source* and the
lattice has no vocabulary for one, so attribution lived in the ledger regardless and the
two stores described a single fact between them. The engine's counter also advanced per
engine call rather than per the target's turn — the tick point this document specifies —
which a sandbox with no turn order could drive at all. `hex_lattice` now holds hexes,
mana and enchantments; fire is none of those.

`ticks` counts up rather than down for the same reason `start` does: a number that only
increases cannot be double-decremented by a repeated frame, and comparing it against the
end condition is a total function of two facts.

The ledger is cleared on leaving gameplay, because unit ids restart each session and
nothing ever drains an effect: an inherited burn would tick on a stranger forever. A
fight *ending* drops only what could not be delivered — nothing in the design puts a fire
out because the party walked away from it.

### What this does not settle

Burn is a named accelerant of the design's negative spiral, and the brakes are
[explicitly deferred](../design/game.md#recovery-and-death). This adds the accelerant and
no brake, on purpose: **initiative**, **action economy**, **fight length**, **permadeath**
and **functional death** are all still open, and how much a burn should hurt is a feel
question nobody has played with yet.

## Where it lives

| Crate | Holds |
|---|---|
| `hex_core` | `Mode`, `Turn` and `RoundElapsed` — shared because `hex_combat` writes them and `hex_units` reads them, and neither can see the other. Also `KnowledgeSource` / `KnowledgeExpiry`, which both knowledge channels need, and the `{source, target, payload, start, end}` persistent-effect vocabulary |
| `hex_units` | Bodies, positions, factions, where a unit may step |
| `hex_combat` | The turn order, engagement, the placeholder AI, the lattice-knowledge store, and the persistent-effect runtime |
| `hex_lattice` | The pure rules engine for castability, mana, fusions, enchantments, and disables. `hex_combat` drives it and owns turns, effects, defender decisions, and knowledge around it |
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
