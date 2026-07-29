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
procedural generator: seeded recipes with validated crossings, anchors that encounters
place units on by name, architecture probes for frozen and volcanic Hills, and
dedicated Sky Islands, Mountains, and Caves biomes. Sky Islands preserves a complete
playable Hills map below a high flight-gated upper network. Mountains covers most of
the map with sharp frozen massifs, deliberate cliffs, and a high-pass/low-bypass route
pair without introducing a river. Caves places a varied rocky surface above a
two-wide entrance and a dense, height-validated underground chamber network with
exact opaque cutaway roofs.

V3 now has its first two complete recipe lanes. Waterfall authors deterministic
directed liquid topology from calm inlet through rapids, a contiguous fall, plunge
basin, outlet, and redundant land routes; an opaque animated renderer consumes the
same exact flow facts. Forest plans rolling terrain and clearings, places its denser
woodland, then bends a mostly two-wide road around those exact roots with short
one-wide constraints and a three-cell prairie taper. Most prairie surfaces carry tall
grass, while a few renderer-private tall exemplars vary the shared low-poly tree
silhouette without claiming future multi-voxel occupancy. Tree roots are exact map
blockers and tall grass is presentation-only. Map validation, movement previews,
click routing, command validation, spawning, review relocation, and enemy pathfinding
all consume the same exact blocker projection through the gameplay-owned adapter that
has now passed review and is live on `dev`.

Authoritative spatial perception now runs headlessly every gameplay frame.
`hex_world` publishes a renderer-independent Bright or Dim exterior tier;
`hex_perception` derives exact exterior/interior domains, maximum-tier public local
lights, pooled faction sight, and independent faction memory over stacked `TilePos`
surfaces. Unknown, Remembered, and Observed terrain snapshots do not leak hidden
edits, unseen units disappear immediately, and the player-side traversal projection
is rebuilt from the same knowledge. Three validated hot-reloadable sight profiles
live in `perception.ron`. World observation already gates the gameplay-owned hostile
lattice-knowledge view. Fog/picking presentation, generated cave lamps/crystals,
unknown-frontier routing, and engagement/targeting/AI consumers are not wired yet.

Movement is level-based over stacked surfaces, with body size decided by headroom and
a breadth-first pathfinder that cannot collapse a stack. A movement preview draws the
reachable set and the route before a click commits to either.
Combat has two tempos, a turn order, engagement with hysteresis, and surface-aware
targeting where height buys range. Its tuning values are designer-facing knobs in
`assets/config/combat.ron`. Player and AI intent flows through one **command funnel**:
clicks, the end-turn key, and the AI emit `GameCommand`s into a queue, and a single
applier in `hex_combat` validates each against seat, turn, reach, and budget before
applying it. Passive effects and derived consequences such as downing run at their
own deterministic schedule points. The queue is consumed rather than persisted;
recording its command stream is future replay work.

Who stands on a map is an **encounter**: `assets/config/encounters/*.ron`, a roster of
units per side, each naming an archetype and one placement — an authored coordinate, a
generated anchor, or a formation that spreads a group over the surfaces walkable from one
centre. A scenario names its encounter by path exactly as it names its world and its sky,
so several scenarios share one file, and every rostered unit is either placed or setup
fails naming the entry and the reason. It replaced a two-coordinate scaffold that could
express one player and one enemy and nothing else. **The archetype is looked up in
`lattices.ron`**, so a roster line is most of what a unit is. The shipped encounters are
no longer limited to one unit a side. Party Trial fields matching three-member
hedge-mage, raider, and wolf parties, while Ability Lab and Raider Mirror keep focused
ability and identity checks small.

The element wheel and spells now load as **validated content**: `elements.ron` (the
six-element wheel, opposition, and fusion recipes, checked acyclic and feedable) and
`spells.ron` (requirements as an element multiset with tier ≤ 6, casting and mana axes,
targeting, and a closed effect enum). A `ContentIndex` resolves every element and
substance name a spell references; a dangling reference is logged and the last valid
content kept, and a test opens everything shipped so a broken reference cannot ship.
`ElementId` and `SpellId` are opaque `hex_core` ids assigned from sorted names, like
`SubstanceId`. A dev-feature content dump remains available for inspecting the resolved
spell list, while gameplay now consumes the same catalogs through its lattices and cast
panel.

**Damage exists.** The lattice engine (`hex_lattice`) is joined to the game at last:
`lattices.ron` authors the three archetypes the design names — a wolf of four hexes and
a bite, a raider of eight around a metal shield, a hedge-mage of thirteen with the
roster's only fusion chain and Scrying Eye — and units spawn carrying them, keyed by the
archetype their encounter rostered. A cast goes through the command funnel and the
legality ladder, and drains the lattice that paid for it. Damage names a count; **the defender
chooses which hexes go down**, answering through a `ChooseDisables` command so the choice
is replayable rather than made inside the applier. A unit whose every hex is disabled
leaves the turn order and is **downed** — retained with its lattice rather than
despawned. Renewal restores chosen cells, removes `Downed`, and returns the unit at the
next round boundary; exploration Rest recovers the party immediately. A strike deals
damage the same way, through the same decision.

**And casting has an interface.** A spell panel lists what the acting unit inscribes,
each row carrying its live blocked reason from `castable` and, above the list, whichever
of the applier's own refusals is standing in the way — not this unit's turn, action
already spent, a decision still open. Choosing a spell starts aiming: every legal anchor
takes a clickable marker, `hex_units::volumes` resolves the shape, and the surfaces
inside that volume are painted in the spell's element colour. The anchor moves by
clicking a lit surface or by cycling the units in range; `ENTER` casts and `Q` puts the
spell down. Only *surfaces* are painted — gameplay cannot know how tall a level is in
world units — so the panel reports the whole voxel count beside the number it could
show. The `1`-casts-something placeholder that made the damage loop playable before any
of this existed is gone.

Bodies are one hex wide; there is no footprint for anything larger, and units do not
obstruct each other — so a route may be drawn straight through another piece.

**Complete-party combat is live.** The stable party rail selects up to six members,
number keys and camera focus follow that roster, and combat hands selection to the
acting ally. Exploration can switch between Solo movement and atomic Group movement;
authored formations rotate by route segment, compress through the Crossing bottleneck,
and reform when space returns. Algorithm-neutral AI consumes canonical legal actions
through the same command funnel as the player. Victory and Defeat retain the
battlefield, Retry rebuilds the same resolved seed, Renewal revives at the next round
boundary, and exploration Rest recovers the whole party. The tactical HUD keeps actor,
selected ally, decision owner, aimed target, and retained target as explicit roles.
Party Trial is the 3v3 integration and human regression fixture; Ability Lab and Raider
Mirror remain its focused automated companions.

The **knowledge seam is live** as `hex_combat::knowledge`:
`FactionLatticeKnowledge::view` is the one read path for a hostile lattice.
World-owned `FactionMapKnowledge` gates which subjects currently exist to each viewer;
the gameplay adapter publishes only existence and faction, while capacity and cells
remain opaque until Reveal.
Scrying Eye writes a complete, expiring projection whose known cells refresh from live
mana and disabled state without extending its lifetime. The HUD renders that projection,
retains a valid aimed hostile, and freezes legal disclosure when each typed combat event
enters the bounded log. The dev reveal-all toggle remains `K` under the `dev` feature.

Around the game sits its own verification tooling. A **lattice-demo screen** on the
title menu isolates the magic ruleset and shared lattice renderer from a full fight. A
default-off
**`visual-walk`** build drives the whole game through scripted RON walks — screens,
clicks by `Name`, keys, scenario launches — photographing every step through an
offscreen render target so an agent can read the frames; `/audit-pr` runs it as a
mechanical gate, and the *Close Quarters* scenario exists so a walk (or a person)
reaches combat in one click. The menus wear vendored Cinzel/Inter type over a
design-token widget set; scenarios carry optional per-scenario lighting, and cyclic
time-of-day is available to those that opt in. The title screen shows the workspace
version, sessions write a `hex_game.log` beside the executable (fresh per launch),
and a panic hook puts the last words in it.

The standalone **Asset Workshop** is available through `cargo editor`. It loads the
canonical palette, voxel-style, and object catalogs, starts with an unsaved
calibration object, and provides palette/style editing plus hex-voxel object authoring
with semantic parts, masks, level slicing, deterministic preview rigs, camera
controls, grouped undo/redo, explicit validated saves, external-change guards, and
untracked crash recovery. A clean saved object can export a deterministic ten-view
review pack, contact sheet, and semantic report under `.context/asset-workshop/`.

The runtime resolves that complete art graph atomically and retains its last valid
revision across a bad hot reload. `hex_objects` renders static instances from cached
mesh chunks using the game prism and exact palette-backed material modes. The first
production exemplar is the six-level `plant/small-broadleaf`. Terrain substances,
liquids, construction metal, and unit presentation also resolve exact palette
swatches. Forest still presents its generated temporary vegetation directly; adapting
those placements to authored object instances and procedural plant synthesis have not
landed yet.

## What is provisional

Everything in this table is a guess standing in for a decision that
[the design](../design/game.md) explicitly has not taken. **Do not tune these into
place** — they are meant to be replaced.

| Thing | Now | What it is waiting for |
|---|---|---|
| **Initiative** | a number on a component, high to low, ties by stable `UnitId` | The initiative question; derived-from-lattice is one candidate and could also address boss action economy |
| **A turn** | 4 hexes of movement and one action | The action-economy question. The design's current preference is 1–2 hexes plus an action |
| **Damage** | disables lattice hexes; a player defender chooses and confirms live cells in the HUD, while non-player defenders use a deterministic cheapest-first policy | The fight-length question — how many hexes a spell should take is a feel question nobody has played with yet |
| **Enemy behaviour** | close the distance, swing | A rout threshold to know when to stop, and a reason to cast. Units carry lattices now, so the AI *could* read `view()` and choose a spell; it still only strikes, which is the placeholder it always was |
| **Engage range** | 4 hexes, 6 to disengage; perception will gate the reach trigger on observation | The numbers remain a feel question. The disengage margin stays spatial hysteresis; the separate lost-contact rule searches for one round |
| **What height is worth** | +1 hex of range per 5 levels above the target | The value remains provisional; engagement and spell targeting now share the rule |
| **How the tints look** | pale warm white, 0.22 alpha for range and 0.6 for the route | Nothing but taste. The constants are at the top of `hex_units::selection`; change the numbers rather than the structure |

**No randomness** is *not* provisional. The design is explicit that uncertainty comes
from hidden information rather than dice, so the turn order is deterministic: ties
break by the stable `UnitId` dealt at spawn, and the same units always
produce the same order across runs and saves.

### What damage does not settle

It disables hexes and it can put a unit down, and that is deliberately as far as it
goes. **Downed is provisional**: the design leaves both functional death — a threshold
arriving before zero — and permadeath open, and a unit whose lattice is spent simply
leaves the turn order while retaining its lattice for restoration. Renewal can
reactivate it for the next round, and exploration Rest recovers it. How many hexes a
spell disables, how long a fight runs, and what a strike costs are all knobs rather
than answers; `strike_disables`
sits in `combat.ron` beside the rest precisely so it can be moved without touching code.
Further damage against an already downed target is refused before spending the action
or mana, while non-damaging inspection such as Reveal can still reach the retained
lattice.

One thing a landed cast still cannot do: reach terrain, because rungs 4 and 5 of the
ladder wait on `RunBottom` from the world lane. It refuses by name rather than silently
doing nothing.

**A cast can now outlast itself.** `Burn` runs through the persistent-effect runtime
(`hex_combat::effects`, vocabulary in `hex_core::effects`): a cast books a countdown in
the effect ledger, and one hex goes down at the start of each of that target's own turns.
The countdown lives **only** there. An earlier shape parked a `Vec<Burn>` inside
`LatticeState` and it was pulled back out before anything persisted it — a burn has a
source the lattice has no vocabulary for, and a tick point a rules engine with no turn
order cannot see. The two settled rules hold — the tick point is **personal, not the round
boundary**, and burn **ignores armour** while still going through the defender's choice,
so the nondeterministic choice is captured as a replayable command. No replay log is
persisted yet. What that does *not* settle is anything about the negative spiral it
accelerates: fight length, functional death, and the brakes the design names (rout,
surrender) are all still deferred, and burn deliberately ships without one. See
[systems/combat.md](../systems/combat.md#effects-that-outlast-their-cast).

## Not built, and not next

Everything in [the design](../design/game.md#open-questions)'s open questions, plus:

- **Terrain that costs something to cross.** `Reach` charges one per step, so the
  shortest route is the one taken and breadth-first order is enough to find it. Mud,
  ice or a climb would each need a priority queue, and none of them are designed.
- **Units obstructing each other.** Two units can occupy the same surface, and the
  pathfinder will happily route one straight through another. An occupancy map over
  unit positions would fix both and lives entirely in `hex_combat`. Encounter placement
  is the one exception: a roster never *starts* two units on one voxel, because
  placement tracks the surfaces it has already used.
- **A way out of a stalemate.** A melee-only enemy separated by terrain it cannot cross
  stays in the fight forever: `approach` finds no route, so it spends its turn doing
  nothing, every round. Height makes this easier to fall into, since a fight now starts
  from further away when one side is above the other. Nothing is stuck — the player can
  still walk out past the disengage margin — but the enemy should give up rather than
  wait to be left. That is the rout threshold the design names and the enemy-behaviour
  row above is waiting on. **Rout was deferred deliberately on 2026-07-27**, not
  overlooked: the threshold is a number nobody can pick honestly before fights have been
  played. `rout_policy` stays an unbuilt knob, and this stalemate is the known cost of
  waiting.
- **Multi-hex bodies.** `Body` has room for a footprint; the rule for whether a wide
  body may straddle a one-level step has not been decided.

## Casting: the playable slice and its remaining boundary

[casting.md](../systems/casting.md) records both the built 0.3 path and the terrain
contract still ahead. `GameCommand::Cast` is authoritative, pays through the acting
lattice, emits typed outcomes, and applies the implemented single-target unit effects:
direct disables, Burn, and Reveal. The panel and aiming flow described above are the
ordinary player path into that command.

**The shape vocabulary resolves to exact voxels**
(`hex_units::volumes`): `SelfCast`, `Single`, `Sphere`, `Column`, `Line`, `Cone` and
`Path`, over `TilePos` in the grid-space metric where hexes and levels count equally,
handing back the sorted, deduplicated form an announcement requires. `spells.ron`'s
`TargetShape` carries the matching extents — `Blast` is now `Sphere(radius: N)` — and
validation caps them. The casting preview resolves the aimed shape and paints every
surface in it, and the cast applier refuses a shape that cannot resolve. What is still
missing is the wider half: no cast announces a terrain volume, no unit effect iterates
every occupant of a multi-voxel volume, and no volume clips itself to obstruction.
Those are the rest of terrain magic.

The binding parts are:

- **Friendly fire remains enabled.** Unit effects include allies, enemies, and the
  caster whenever the resolved volume includes them.
- **Every positional anchor must be Observed.** An area may extend into Remembered or
  Unknown positions, but presentation and logs do not reveal hidden impact outcomes.
- **Evocation terrain persists for multiple turns.** The initial implementation makes
  applied terrain edits permanent rather than keeping an expiry ledger.
- **Generated feature effects are deferred.** Trees, tall grass, and other feature
  entities ignore impacts until a feature-response and outcome contract lands.
- **Conjuration is map-approved content.** A spell may name only a substance marked
  `conjurable`; the generic `TerrainEdit::Set` path remains available for authored
  restoration and other non-spell uses.

The first implementation also ships with explicit limitations:

- **Volumes are geometric, not obstruction-aware.** A sphere next to a cave
  wall fills voxels inside the rock and the chamber beyond it. Clipping waits on the
  same line-of-sight work that `RunBottom` ([boundary.md](boundary.md) ask C) unlocks,
  and `needs_los` on spell content is parsed but unenforced until then.
- **A breached cave roof will not admit daylight.** Terrain edits already keep the
  interior *roof* projection current, but interior **membership** is never re-derived,
  so a chamber you blow open still counts as inside. Live perception therefore
  continues to classify the chamber as Interior and does not admit daylight
  ([boundary.md](boundary.md) ask I).
- **Casting is provisionally combat-only.** Recovery between fights is intended to be
  a rest action, but real-time casting still needs an interaction and rest flow.
  **Channelling and rituals are deferred** — `co_castable` parses and labels rituals
  in the demo, but has no mechanical effect.
- **Paid-on-resistance is provisional.** The first wave charges mana and the action
  after a legal announcement even if every material resists.
- **No-undermining is provisional.** The first wave rejects terrain creation through
  a unit and edits to its supporting surface until falling and footing reconciliation
  exist.
- **Downed-first death is provisional.** A fully disabled unit initially leaves the
  turn order and retains its lattice. Renewal restores it into the next round and Rest
  recovers it after combat; functional death and permadeath remain open.
- **A unit effect reaches the unit on the anchor, not everyone in the volume — and an
  area spell is therefore refused at load.** `volumes::resolve` produces the full voxel
  list and the preview paints it, but `DisableHexes` and `Burn` both apply to whoever
  stands on the target voxel. The friendly-fire contract above is unchanged and
  unweakened — nothing filters by faction — but a fireball *would* damage one unit
  rather than every unit inside it.

  Rather than ship that as a silent lie, `lattices.ron` **rejects an inscribed spell
  whose shape covers more than the anchor and whose effects reach units**
  (`LatticeError::AreaEffectUnapplied`). The interface can only paint what a lattice can
  cast, so the preview cannot promise what the applier will not deliver. The refusal
  lifts the day the applier iterates the volume and queues one decision per unit inside
  it; it is the same seam `RunBottom` and the announce path close for terrain.
- **Burn attributes one source per tick.** Several burns on one target come due as a
  single count and therefore a single decision, which has room for one `source`. The
  earliest-lit fire fills it. The rules never read `source`, so the imprecision is
  confined to the combat log.

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
(symbolication, upload, a dialog) has not. Wave 5 adds disposable pre-alpha continuity
and replaceable app-shell, settings, audio/input, and artifact seams; it does not close
the production gap or promise compatibility. The full checklist and evidence remain
frozen in [production-audit.md](production-audit.md); the sequenced scaffold is in
[roadmap.md](roadmap.md).
