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
separate flight-gated upper network. The Mountains probe adds sharp frozen ridges,
deliberate cliffs, and a high-pass/low-bypass route pair without introducing a river.
The Caves probe adds a playable rocky surface above a two-wide entrance and a
height-validated underground chamber network with exact opaque cutaway roofs.
Movement is level-based over stacked surfaces, with body size decided by headroom and
a breadth-first pathfinder that cannot collapse a stack. A movement preview draws the
reachable set and the route before a click commits to either.
Combat has two tempos, a turn order, engagement with hysteresis, and surface-aware
targeting where height buys range. Its tuning values are designer-facing knobs in
`assets/config/combat.ron`, and every sim mutation flows through one **command
funnel**: clicks, the end-turn key, and the AI emit `GameCommand`s into a queue,
and a single applier in `hex_combat` validates each against seat, turn, reach, and
budget before anything moves — which is what makes an input log a replay.

The element wheel and spells now load as **validated content**: `elements.ron` (the
six-element wheel, opposition, and fusion recipes, checked acyclic and feedable) and
`spells.ron` (requirements as an element multiset with tier ≤ 6, casting and mana axes,
targeting, and a closed effect enum). A `ContentIndex` resolves every element and
substance name a spell references; a dangling reference is logged and the last valid
content kept, and a test opens everything shipped so a broken reference cannot ship.
`ElementId` and `SpellId` are opaque `hex_core` ids assigned from sorted names, like
`SubstanceId`. A dev-feature stub logs the resolved spell list to prove the pipeline
end to end.

The **lattice engine also exists** as a pure crate (`hex_lattice`): casting
(`castable` → a `CastPlan` or a blocked reason), applying casts, disables that break
enchantments and burn their locked mana, channelling, and a property suite carrying
the design's geometric theorems — all headless. Content and engine are **not yet
joined**: the `FusionTable`/`SpellTable` wiring and everything that spawns units with
lattices, casts in-game, or deals damage is the "lattices wired" work (HEX-12). Bodies
are one hex wide; there is no footprint for anything larger, and units do not obstruct
each other — so a route may be drawn straight through another piece.

Around the game sits its own verification tooling. A **lattice-demo screen** on the
title menu exercises the magic ruleset by hand ahead of HEX-12. A default-off
**`visual-walk`** build drives the whole game through scripted RON walks — screens,
clicks by `Name`, keys, scenario launches — photographing every step through an
offscreen render target so an agent can read the frames; `/audit-pr` runs it as a
mechanical gate, and the *Close Quarters* scenario exists so a walk (or a person)
reaches combat in one click. The menus wear vendored Cinzel/Inter type over a
design-token widget set; scenarios carry optional per-scenario lighting, and cyclic
time-of-day is available to those that opt in. The title screen shows the workspace
version, sessions write a `hex_game.log` beside the executable (fresh per launch),
and a panic hook puts the last words in it.

## What is provisional

Everything in this table is a guess standing in for a decision that
[the design](../design/game.md) explicitly has not taken. **Do not tune these into
place** — they are meant to be replaced.

| Thing | Now | What it is waiting for |
|---|---|---|
| **Initiative** | a number on a component, high to low, ties by stable `UnitId` | Derived from lattice size, per the design — which also solves boss action economy by giving a large lattice several slots |
| **A turn** | 4 hexes of movement and one action | The action-economy question. The design's current preference is 1–2 hexes plus an action |
| **Damage** | none at all | Lattices *wired into units*. The engine (`hex_lattice`) exists and is property-tested; what is missing is spawning units with lattices and routing damage through `apply_disables` |
| **Enemy behaviour** | close the distance, swing | Lattices to know what it can cast, hidden information to know what it knows, a rout threshold to know when to stop |
| **Engage range** | 4 hexes, 6 to disengage | Nothing in particular. It is a feel question and wants playing with |
| **What height is worth** | +1 hex of range per 5 levels above the target | Abilities. The rule is real but has exactly one caller — engagement — until there are spells with ranges to apply it to |
| **How the tints look** | pale warm white, 0.22 alpha for range and 0.6 for the route | Nothing but taste. The constants are at the top of `hex_units::selection`; change the numbers rather than the structure |

**No randomness** is *not* provisional. The design is explicit that uncertainty comes
from hidden information rather than dice, so the turn order is deterministic: ties
break by the stable `UnitId` dealt at spawn, and the same units always
produce the same order across runs and saves.

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

Most of what makes this a product does not exist yet: no saves, no settings menu,
no audio, no input rebinding, and no signing or store packaging. The first hygiene
slice has landed — a per-session log file beside the executable, a panic hook that
writes into it, and the version on the title screen — but full crash *reporting*
(symbolication, upload, a dialog) has not. The full checklist, with the evidence
behind each line and the crate choices for closing them, is
[production-audit.md](production-audit.md); the sequenced work is the
production-hygiene epic in [roadmap.md](roadmap.md).
