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

## Tactical HUD and role resolution

Gameplay UI is one responsive safe frame with 12px baseline viewport margins and 8px
baseline inter-panel gaps. Semantic scale and viewport class resolve the party,
Inspector, top-turn, and bottom-action regions; center bands consume the resolved
side insets, so no independently positioned panels compete for the same pixels. Most
read-only regions pass pointer input through to the world. The Inspector participates
in picking because it is the single scroll owner for lattice and secondary detail,
and opaque action/history panels catch input intended for their controls.

The HUD is mode-aware:

- Exploring shows the party rail, selected-ally lattice, formation editor, Group/Solo,
  presets, Rest, and the exploration hint. Combat-only spell actions are absent.
- A player turn shows the compact initiative rail, party rail, active ally lattice,
  movement and action budget, spells, and visible Channel and End Turn buttons.
- A hostile turn says `ENEMY TURN`, keeps a labeled `SELECTED ALLY` lattice for
  inspection, and replaces action buttons with `PLAYER COMMANDS LOCKED`.
- A damage or restoration decision replaces ordinary actions with its exact role,
  owner and target, quota, Clear, and Confirm. Hiding ordinary HUD chrome cannot hide
  these required controls.

`hex_game::readouts::GameplayUiContext` is the presentation-private projection that
keeps acting unit, selected ally, caster, decision owner, decision target, aimed
target, and retained target separate. No panel applies a precedence rule to guess
which unit is “mine.” Every binding selects one explicit role:
`ACTIVE ALLY`, `SELECTED ALLY`, `DAMAGE CHOICE`, `RESTORE TARGET`, `AIM TARGET`, or
`PINNED TARGET`. Identity lines always include faction and unit identity, such as
`ALLY 2 · RAIDER #1` or `HOSTILE · RAIDER #4`.

Only the acting unit and current target receive short world badges. A hostile target
is retained while an aim moves over empty terrain only as `PINNED TARGET`; it is
cleared when aim is cancelled, its caster or turn changes, combat exits, or the target
is no longer valid. Hostile lattice contents still pass exclusively through faction
knowledge.

The default combat feed is the latest three structured events. `L` opens and closes
the bounded full-history drawer; command refusals use high-priority styling and remain
in the same 64-event history as actions, Rest, revival, and encounter outcomes.

The projection asserts rather than conceals producer disagreements. On player turns,
actor, selected unit, and caster must agree. On hostile turns, player casting cannot
remain enabled. Decision headings must name owner and affected target. A violation is
a state-production defect to fix and cover, not a reason for the UI to choose another
precedence rule.

## A turn

One unit holds a `Turn` at a time. It carries a movement budget and whether the action
has been taken. Ending it passes the marker to the next unit in the order; running off
the end wraps and counts a round.

The serializable authority for those facts is `hex_combat_core::CombatState`. It
reduces frozen rules, roster, exact directed arena links, explicit faction
observation, and stable content names through one ordered command boundary. Its
canonical simulation requires no Bevy `App`, ECS schedule, renderer, viewport,
wall-clock settling, asset server, or map generator. Exact links are published input,
so the reducer never guesses connectivity from `HexCoord` and cannot collapse stacked
surfaces.

Wave 8's adapter boundary is fail-closed. Move, Strike, End Turn, Channel, and exact
disable choices reduce only in `CombatState`; ECS receives their projection. Cast and
restoration still resolve authored content through typed host adapters, then publish
the complete position/turn/downed/lattice/order/decision/revival projection
transactionally before another command may run. Missing authority is a typed refusal,
never permission to invoke a legacy mutator. AI and human input both remain command
producers; animation and UI are projections.

**A turn cannot end while its unit is still moving.** `MovingTo` owns a bounded
domain clock and exact surface path; `Busy` is its legality/turn gate. The movement
reconciler publishes whole crossed surfaces and clears both at the final surface.
`Transformation` mirrors that route for presentation only. Removing it early cannot
move or unlock the unit, and retaining a strike/cast animation cannot retain the turn.

Keys: `SPACE` ends the current player turn and is ignored while a hostile owns the
turn. `ESC` and `BACKSPACE` were already taken by pause and quit-to-title.

### A defender choice is command-modal, not Pause

When damage opens `PendingDecision::ChooseDisables`, the command applier rejects every
simulation command except a `ChooseDisables` answer for that exact defender. Input
emitters also stop producing movement, casts, end-turn commands, and AI actions while
the decision is open, keeping the refusal log quiet during ordinary play.

A `Player` defender uses the explicitly labeled ally lattice: only live cells are
buttons, additional picks stop at the owed quota, and the bottom decision dock's Clear
and Confirm controls (or `ENTER`) answer it. If every live cell is owed, all are
preselected but confirmation is still explicit. Non-player defenders use the
deterministic policy and issue the answer under their own `ControlOwner`.

This is not the `Pause` state. Camera and ordinary UI keep running. `H` hides ordinary
readouts but deliberately leaves an active decision lattice and its controls visible.

Every accepted outcome and refusal is also a public, serde-capable `CombatEvent` or
`CommandRefusal`. Those contracts use stable unit ids, spell names, positions, and exact
lattice-coordinate lists—never session-local spell ids or formatted presentation
strings. The combat log applies faction disclosure when it ingests each event, so later
divination cannot rewrite what an older line was allowed to reveal.

The one-decision seam is also an authoring boundary: a spell may contain at most one
exact-cell decision effect across non-targeted `DisableHexes` and `RestoreHexes`,
because two would otherwise overwrite each other's choice. Damage commands aimed at an
already downed unit are refused before spending action or mana; non-damaging inspection
such as Reveal remains legal because the retained lattice is the restoration target.

### Terminal outcomes retain the battlefield

After commands apply, outstanding decisions settle, and newly spent lattices become
`Downed`, combat counts active factions. A surviving player side with no active
hostile is Victory; no active player is Defeat, including simultaneous elimination.
The result is emitted once and stored in `EncounterResolution`.

That resource gates the same `PausableSystems` set as ordinary pause, freezing
movement, casting, AI, effects, command application, and turns while the world remains
visible. Outcome UI runs outside the gate. Victory continues into Exploring. Defeat
can rebuild the retained scenario snapshot with its original resolved seed, or return
to the title screen.

### Revival waits for a round boundary

`RestoreHexes` opens `ChooseRestores` for the caster, naming the target and quota.
The answer is a command containing exact target cells; every coordinate must be a
distinct disabled cell on that target, and the answer count is the smaller of the
authored amount and the cells currently disabled. Restoring at least one cell removes
`Downed`, but the unit is held outside `TurnOrder` until the next wrap. At that boundary
it rejoins the initiative sort by initiative then stable `UnitId`.

### Channel closes the mana loop

`GameCommand::Channel` is a canonical combat action for an active, non-downed unit
whose one action remains. The command passes through the same modal, seat, turn,
busy, and action gates as casting and striking. It consumes exactly that action even
when every eligible gem is already full; it neither spends movement nor grants
another action.

The applier delegates the refill itself to `hex_lattice::channel`. For each element
in stable ID order, the unit's Channelling budget fills live unlocked gems in
`LatticeCoord` order up to their per-gem Attunement capacity. Disabled cells are not
repaired and enchantment locks are not bypassed. The returned per-element amounts are
resolved through the loaded element catalog before mutation and emitted as one
`CombatEvent::Channelled` under stable names. Missing or inconsistent catalog/lattice
facts fail closed.

The player action panel emits Channel through the command queue. Combat includes it
in the baseline AI's canonical legal-action set only when the actor carries the full
lattice/spec/stats contract; the deterministic baseline selects it when a live gem is
empty and no higher-priority restoration, reveal, damaging cast, enchantment, or
strike applies.

### Focused automation and Party Trial

`CombatSummary` is the session-scoped, serde-capable verification artifact. It records
the highest round reached; successful commands by stable unit and semantic kind; exact
AI dispatch traces (profile, algorithm, observation, canonical legal actions,
fingerprint, selection, and emitted command); aggregate moves, casts, strikes,
decisions, and explicit end turns; raw, prevented, and applied disables; restored
cells, revivals, and downings; the ordered structured event stream; and the final
outcome. Wave 7 extends that same authority with refused-command counts, movement
distance and budget use, casts by stable spell name, delivered-effect categories,
Channel actions, and mana restored by stable element name. A versioned deterministic
summary fingerprint covers the aggregates plus both bounded detail windows; their
rolling fingerprints continue to cover facts that aged out. It resets with the
gameplay session.

Typed gameplay tests deliberately do not use screenshots as their combat oracle:

- **Ability Lab** is a flat 2v1 with one player hedge-mage, one player wolf, and one
  hostile raider. Two allies are the minimum honest fixture for friendly damage,
  downing, Renewal, and next-round revival. The app/contracts partitions cover
  Scrying Eye, aim/pin/confirm, damage decisions, refusal history, and the wolf's
  no-spell turn through canonical state.
- **Raider Mirror** is a flat 1v1 with the same archetype on opposite factions. It is
  the focused state regression for a hostile raider ever being selected or presented
  as the active allied raider.

Both retain their typed fixture identities, but their behavior runs through the
concern partitions rather than frame-sensitive walk automation:

```sh
python3 tools/gameplay_scope.py run contracts
python3 tools/gameplay_scope.py run simulation
python3 tools/gameplay_scope.py run app
```

`walks/gameplay_ui.ron` may present these surfaces, but its frames prove only UI
composition and legibility.

The shipped **Party Trial** remains the full 3v3 integration and human test: matching
hedge-mage, raider, and wolf rosters approach the authored Crossing from opposite
banks. Its headless simulation replay still compares party routes, AI observations,
legal-action fingerprints, commands, events, turn order, positions, summary, and
outcome. The release playtest alone owns formation editing, bridge compression,
reformation, six-unit readability, and the interaction between terrain and combat.
The Wave 4 gate passed after that walk exercised Renewal on a downed ally and verified
next-round initiative return. Party Trial keeps the same coverage as the persistent
human regression walk for later waves.

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

Exploration Group movement applies that same contract to every player member. One
exact-path `MoveParty` is validated in full before any member receives presentation,
and interruption reconciles every in-flight member before initiative is built.
Rotation, bottleneck compression, reformation, and Solo behavior are specified in
[party.md](party.md).

### Bodies occupy exact surfaces

`hex_units::UnitOccupancy` is the one gameplay projection from stable `UnitId` to
exact `TilePos`. Elevation is identity: a body on the ground does not block the bridge
surface above it. The projection includes each unit's current `StandsOn` and every
surface reserved by an in-flight `MovingTo` route. A downed unit remains a body and
continues to own its surface so revival cannot create an overlap; despawn/removal
removes it with the entity.

`Reach`, click path construction, the movement preview, authoritative `MoveAlong`,
baseline AI legal actions and traversal, whole-party planning/application, encounter
placement, and Combat Lab deployment all consume this projection. A route may neither
finish on nor pass through another body. The command refusal preserves that
distinction as `OccupancyBlock::Destination` or `OccupancyBlock::Route`, including
the exact surface and stable occupant. Commands drained in one frame reserve their
endpoints before the next command validates, so deferred ECS insertion cannot create
a same-frame overlap.

Group movement excludes the moving party's own starting surfaces while retaining
external bodies, then continues to require unique member destinations. This preserves
the existing atomic compression/trailing behavior without allowing a formation to
route through a nonparty body or letting two members swap directly across one edge.

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
