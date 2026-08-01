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
playable Hills map below a higher, multi-band flight-gated upper network. Mountains
covers most of the map with sharp frozen massifs, deliberate cliffs, a
high-pass/low-bypass route pair, snow caps, and a peak-fed river and fall. Caves
places a varied rocky surface above a two-wide entrance and a dense,
height-validated underground chamber network with exact opaque cutaway roofs.

V3 now has ten complete recipe variants: Hills, Sky Islands, Mountains, Caves,
Waterfall, Forest, Fort, Volcano, Deep Forest, and Prairie. Ring7 places its fixed
seven-recipe roster in one connected radius-33 world. Ring19 powers the selectable
**Two Rings** map: a radius-55, 9,241-column world with 19 fixed regions, 42
reciprocal seams, 30 outer boundary sides, and a physical ordinary-walker graph that
keeps all regions reachable after any one seam is removed. Its three mountain-fed
water branches meet in central Hills before flowing through downstream Hills and an
outlet Waterfall; the western Volcano owns a separate lava outlet. Single and Ring7
retain their 4-bit patch namespace, while Ring19 uses 5 patch bits so slots 16–18
remain collision-free.

Waterfall authors deterministic directed liquid topology from calm inlet through
rapids, a contiguous thirteen-level fall, plunge basin, outlet, redundant land
routes, and sparse vegetation; an opaque animated renderer consumes the same exact
flow facts. Forest plans rolling terrain and clearings, places its denser woodland,
then bends a mostly two-wide road around exact authored tree footprints with short
one-wide constraints and a three-cell prairie taper. Deep Forest extends blocking
woodland across a complete patch around a winding trail and three clearings; Prairie
ships the complementary tree-free grassland. Volcano replaces the old volcanic-Hills
geometry behind the stable scenario name with a crater massif, descending lava, and
an elevated stair-served bridge.

Small broadleaf, tall narrow, and seven-root old-growth trees vary the canopy and
height profile; most prairie surfaces carry nonblocking authored grass tufts. Object
ids, exact rotations, and rotated blocker footprints are fingerprinted before
routing. Map validation, movement previews, click routing, command validation,
spawning, review relocation, enemy pathfinding, terrain-edit protection, and the
object renderer consume the same world-owned projection. Native V3 Caves plans the
rocky exterior and stacked underground network together: six through twelve chambers
on three `+0/+2/+4` floor tiers, one-level two-wide connectors, exact cutaway roofs,
sparse authored moss and lichen, and deterministic Bright gameplay lights that cover
the required network while leaving optional branches dark.

Two Rings is mechanically selectable and covered by deterministic generation,
spawning, regeneration, and re-entry checks. Its mandatory human visual and play
approval remains pending, so the development wave is not yet a release candidate.

Authoritative spatial perception now runs headlessly every gameplay frame.
`hex_world` publishes a renderer-independent Bright or Dim exterior tier;
`hex_perception` derives exact exterior/interior domains, maximum-tier public local
lights, pooled faction sight, and independent faction memory over stacked `TilePos`
surfaces. Unknown, Remembered, and Observed terrain snapshots do not leak hidden
edits, unseen units disappear immediately, and the faction-generic traversal
projection is rebuilt from the same knowledge. Downed units can remain visible but
cannot provide sight, and changing `Downed` republishes observation in the same frame.
Three validated hot-reloadable sight profiles live in `perception.ron`. V3 cave
sources publish fixed local gameplay lights directly into this headless pipeline.
World observation gates the gameplay-owned hostile lattice view, every cast anchor,
and AI identities, effects, turn order, traversal, and legal commands. AI can traverse
only Observed or Remembered terrain and cannot use Unknown truth. Fog/picking
presentation, unknown-frontier routing, engagement, ordinary-attack targeting, and
lost-contact search are not wired yet. Authored emissive cave crystals and restrained
physical point lights now present every fixed cave gameplay-light source without
becoming gameplay authority.

Fort adds the first complete V3 structure recipe and the canonical worked-stone
substance. A five-level, two-wide curtain surrounds a gravel courtyard and offset
keep, with six small accessible turrets, two lintelled gates, two broad stair
terraces, and alternating outer battlements. Exact graph validation proves that
closing both gates seals the courtyard, either gate independently reconnects it, and
every usable wall or tower surface remains ordinary-walker accessible.

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
hedge-mage, raider, and wolf parties. Ability Lab and Raider Mirror keep focused
ability and identity checks small inside Combat Lab's fixture selector.

The element wheel and spells now load as **validated content**: `elements.ron` (the
six-element wheel, opposition, and fusion recipes, checked acyclic and feedable) and
`spells.ron` (requirements as an element multiset with tier ≤ 6, casting and mana axes,
targeting, and a closed effect enum). A `ContentIndex` resolves every element and
substance name a spell references; a dangling reference is logged and the last valid
content kept. Canonical source fingerprints prevent that retained index or lattice
library from being paired with newer raw catalogs: Loading requires one
`AcceptedContentRevision` spanning elements, substances, spells, and lattices. A test
opens everything shipped so a broken reference cannot ship.
`ElementId` and `SpellId` are opaque `hex_core` ids assigned from sorted names, like
`SubstanceId`. A dev-feature content dump remains available for inspecting the resolved
spell list, while gameplay now consumes the same catalogs through its lattices and cast
panel. Every externally authored archetype must also form one contiguous lattice;
disconnected islands fail with the archetype named in the error.

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

**Channel is live.** An active, non-downed combatant can spend its one action to
restore each element by that unit's Channelling value, capped by Attunement capacity.
The lattice engine skips disabled and enchantment-locked cells in deterministic
element/coordinate order and reports only mana actually restored. Human input and
baseline AI use the same command/refusal/event seam; the summary attributes Channel
actions and restored mana under stable element names.

**And casting has an interface.** A spell panel lists what the acting unit inscribes,
each row carrying its live blocked reason from `castable` and, above the list, whichever
of the applier's own refusals is standing in the way — not this unit's turn, action
already spent, a decision still open. Choosing a spell starts aiming: every legal anchor
takes a clickable marker, `hex_units::volumes` resolves the shape, and the surfaces
inside that volume are painted in the spell's element colour. The anchor moves by
clicking a lit surface or by cycling the units in range; `ENTER` casts and `Q` puts the
spell down. Only *surfaces* are painted — gameplay cannot know how tall a level is in
world units — so the panel reports the whole voxel count beside the number it could
show. Preview, target cycling, AI enumeration, and the authoritative applier all
require the exact anchor to be Observed. An authorized area may still spill into
hidden space without revealing the result. The `1`-casts-something placeholder that
made the damage loop playable before any of this existed is gone.

Bodies are one hex wide; there is no footprint for anything larger. Exact `TilePos`
occupancy now makes those bodies real: movement preview, path construction, command
validation, party routes, baseline AI, encounter placement, and Combat Lab deployment
all prevent occupied endpoints and pass-through routes without collapsing stacked
elevations. In-flight paths reserve their surfaces, command refusals distinguish route
from endpoint conflicts, and downed bodies retain their surface for revival.

**Complete-party combat is live.** The stable party rail selects up to six members,
number keys and camera focus follow that roster, and combat hands selection to the
acting ally. Exploration can switch between Solo movement and atomic Group movement;
authored formations rotate by route segment, compress through the Crossing bottleneck,
and reform when space returns. Algorithm-neutral AI consumes canonical legal actions
through the same command funnel as the player. Exact-cell damage and restoration use
a compact fingerprinted eligible set instead of allocating every cell combination;
the host validates count, uniqueness, eligibility, and fingerprint before building
the same replayable command. Movement scoring shares one authorized graph, one actor
reach/predecessor projection, and one reverse distance map per live observed hostile.
Victory and Defeat retain the
battlefield, Retry rebuilds the same resolved seed, Renewal revives at the next round
boundary, and exploration Rest recovers the whole party. The tactical HUD keeps actor,
selected ally, decision owner, aimed target, and retained target as explicit roles.
Party Trial is the 3v3 integration and human regression fixture; Ability Lab and Raider
Mirror remain its focused automated companions behind stable fixture IDs.

The **Wave 5 pre-alpha app shell is live**. The title presents one responsive grid of
primary application routes, while a separate Scenarios screen groups development Maps
and focused Demos; New Game resolves the hidden Party Trial default, while Continue
restores one explicit save made from paused exploration. That atomic, build-bound
resume captures scenario/content identity, generator identity, party lattices, and
exploration positions, rejects corrupt or incompatible files visibly, and cannot be
written during combat. Settings atomically persists window, presentation, and volume
preferences, with fixed centralized input actions and audio buses as replaceable
seams. Builds also carry the Hex Game app identity/icon, normalized package layout,
and retained crash symbols. These are disposable continuity and artifact scaffolds,
not a durable save contract, rebinding UI, audio content, signing, storefront, crash
reporting, or telemetry.

The **Wave 6 Creator and Combat Lab are live as primary title routes**. One versioned
atomic creation library owns stable custom character and spell IDs, supports reopen,
modify, rename, duplicate, and confirmed deletion, and refuses removal of referenced
spells. Draft/Ready and Map-ready diagnostics fail closed through shared schema and
combat validation. Immutable human templates are duplicable; automation-only
creator-format records are isolated from local saves. Separate Character Creator and
Spell Creator navigation leads to focused libraries and workspaces; the character
workspace retains a direct spell-management link. The Character Creator provides a
contiguous 64-cell lattice editor, manual stats, undo/redo, and an unsaved local
mechanics test. The Spell Creator provides ordered repeatable effects, targeting,
range, requirements, and permanent Draft/Ready diagnostics.

Combat Lab owns transient Sandbox setup for all sixteen distinct supported shipped
maps, with deterministic renderer previews, tactical descriptions and tags, complete
ordered one-to-six rosters, deterministic deployment, and creator-origin return routing.
Its searchable fixture selector coexists with the focused Scenarios entries and adds
Lab-only packaged creator spell and roster matrices. Every launch freezes its content
namespace,
encounter, and seed for Retry, refuses Continue writes, and restores shipped runtime
content on exit.

The **Wave 7 tactical integrity and tuning workspace is live**. Exact-surface
occupancy is shared by previews, validation, AI, formation movement, encounter
placement, and deployment; Channel is a canonical one-action mana recovery path.
Combat Lab adds frozen Shipped, Tactical, and validated Custom rule profiles,
direct deployment repositioning, immutable occupancy/channel/tempo fixtures, a
canonical live dashboard, and versioned reports with functional Overview, Units,
Spells & Effects, Timeline, and independently selectable Compare modes. Retry Exact,
Tune & Run Again, and fixture Copy preserve the frozen map, ordered rosters, exact
deployment, and profile through re-entry. The bounded tempo audit retained the
shipped four-hex movement default; its measurements and limits are recorded in the
[Wave 7 decision audit](../development/wave-7-tempo-decision.md).

The **Combat Lab strategy-workbench completion is live on the Wave 8 foundation**.
`hex_combat_core` is the sole
renderer-free, serializable authority for the commands it reduces, exact positions,
turns, lattices, and transcripts. Bevy combat resources, unit movement, animation,
and UI are projections or validated content adapters over that authority rather than
parallel mutation paths. Supported active-combat single-target Cast, restoration,
Burn, downing, revival, and annihilation outcomes now reduce there; unsupported
terrain/area spells, exploration Rest, and party movement remain explicit adapters.

The bounded deterministic simulation target proves canonical state, occupancy,
turn/action accounting, profile propagation, fingerprints, spell/effect composition,
and typed command, turn, no-progress, or outcome termination. It consumes exact
per-unit scripts or a deterministic non-random baseline controller. This is a
regression workbench, not a claim that the baseline is optimal or that balance is fun.
Combat Rules and report schemas are version 2: the rules projection mirrors shipping
numeric and typed policy inputs, reports migrate version 1 outcomes, manual/bounded
stops no longer invent an outcome, and fingerprint errors fail closed. Saved reports
carry editable labels/notes and compare frozen inputs plus aggregate, per-unit, spell,
effect, and no-progress facts. Pure
`hex_gameplay_model` transitions own Combat Lab and Creator navigation, report
selection, Retry/Tune/Copy, re-entry identity, and edit history without exposing
mutable widget state.

Gameplay validation is split by oracle into pure rules, focused ECS contracts,
deterministic simulation, and model/headless-app partitions. One fail-closed concern
map selects exact packages, targets, and features for narrow pull requests. Map
validation uses the same authority for unit, deterministic generation, and real-plugin
publication contracts, with all PR seeds preserved under an optimized test-only
profile. Unknown paths, shared core/assets, other world crates, or validation
infrastructure promote to the complete gate. The residual workspace corpus still runs
on its owning changes, `dev` pushes, schedules, and combined wave/release candidates.
Screenshots remain presentation evidence only; the dependency ceilings, commands,
budgets, and anti-patterns are recorded in the
[gameplay](../development/gameplay-testing.md) and
[map](../development/map-testing.md) testing contracts.

The **knowledge seam is live** as `hex_combat::knowledge`:
`FactionLatticeKnowledge::view` is the one read path for a hostile lattice.
World-owned `FactionMapKnowledge` gates which subjects currently exist to each viewer;
the gameplay adapter publishes only existence and faction, while capacity and cells
remain opaque until Reveal.
Scrying Eye writes a complete, expiring projection whose known cells refresh from live
mana and disabled state without extending its lifetime. The HUD renders that projection,
retains a valid aimed hostile, and freezes legal disclosure when each typed combat event
enters the bounded log. The dev reveal-all toggle remains `K` under the `dev` feature.

Around the game sits its own verification tooling. The Creator's **local lattice
test** isolates the magic ruleset and shared lattice renderer from a full fight. A
default-off
**`visual-walk`** build drives the whole game through scripted RON walks — screens,
named UI clicks, exact stack-safe terrain clicks, bounded party-idle waits, keys, and
scenario launches — photographing every step through an offscreen render target so
an agent can read the frames; `/audit-pr` runs it as a
structural and mechanical gate, with usability findings also blocking changes to UI
or presentation. New Game reaches the 3v3 Party Trial in one click, while fixture walks
launch Ability Lab and Raider Mirror by stable ID. The menus wear vendored
Cinzel/Inter type over a
design-token widget set; scenarios carry optional per-scenario lighting, and cyclic
time-of-day is available to those that opt in. The title screen shows the workspace
version, sessions write a `hex_game.log` beside the executable (fresh per launch),
and a panic hook puts the last words in it.

The 2026-07-29 foundation inventory contains 1,363 tests: 1,338 ordinary tests in the
complete all-feature workspace gate and 25 explicitly ignored stress/benchmark
entries. The exact list, measurements, branch matrix, and exclusions are recorded in
[foundation-hardening.md](foundation-hardening.md) rather than repeated as a brittle
project-wide constant.

The 2026-07-30 dev-integrated biome-wave checkpoint extends that complete gate to
1,583 passing ordinary tests and 32 deliberate ignored stress/benchmark entries.
Its release-mode Ring19 generation gate measures Two Rings at 3.250 seconds p95
against Seven Regions at 1.234 seconds p95, or 2.63× inside the 3.5× budget.
Automated final-SHA captures and the mandatory human visual/play review remain
separate release gates.

The standalone **Asset Workshop** is available through `cargo editor`. It loads the
canonical palette, voxel-style, and object catalogs, starts with an unsaved
calibration object, and provides palette/style editing plus hex-voxel object authoring
with semantic parts, masks, level slicing, deterministic preview rigs, camera
controls, grouped undo/redo, explicit validated saves, external-change guards, and
untracked crash recovery. A clean saved object can export a deterministic ten-view
review pack, contact sheet, and semantic report under `.context/asset-workshop/`.

The runtime resolves that complete art graph atomically and retains its last valid
revision across a bad hot reload. `hex_objects` renders static instances from cached
mesh chunks using the game prism and exact palette-backed material modes. Production
review exemplars cover nine-, sixteen-, and twenty-one-level trees, their snowy
variants, a nonblocking grass tuft and snowy variant, cave moss and lichen, and three
nonblocking emissive crystal silhouettes. Terrain substances, liquids, construction
metal, and unit presentation resolve exact palette swatches. Forest and Deep Forest
publish generated vegetation as shared `ObjectInstance`s while retaining exact
rotated blockers and stack-safe tree roots. Character mode fades an entire obstructing
tree through isolated per-tree material clones; authored canopy masks remain art
metadata. Prairie publishes nonblocking grass.
Caves publishes authored crystal `ObjectInstance`s with presentation-only
point-light children at its gameplay-light sites.

Character camera mode keeps player-authored yaw and desired zoom while a conservative
probe retracts against the public stacked-terrain projection and, only when necessary,
searches upward first, then toward the horizon for low-ceiling clearance. Distance and
pitch restore smoothly. Ordinary
gameplay keeps cave roofs intact, while explicit map-review capture may still request a
complete interior cutaway. Automated geometry, lifecycle, idle-churn, and release
performance gates are live. Seed-exact multi-azimuth walks now exercise ordinary
pointer movement to a proved destination on every standalone selectable map and every
Two Rings region; final human motion/readability approval remains pending. Map mode
remains available without a scenario restriction.

## What is provisional

Everything in this table is a guess standing in for a decision that
[the design](../design/game.md) explicitly has not taken. **Do not tune these into
place** — they are meant to be replaced.

| Thing | Now | What it is waiting for |
|---|---|---|
| **Initiative** | a number on a component, high to low, ties by stable `UnitId` | The initiative question; derived-from-lattice is one candidate and could also address boss action economy |
| **A turn** | 4 hexes of movement and one action; retained after the Wave 7 bounded tempo audit | Broader human playtesting and future initiative/action-economy work; Tactical two-step lengthened the fixed 3v3 fixture without a clear compensating benefit |
| **Damage** | disables lattice hexes; a player defender chooses and confirms live cells in the HUD, while non-player defenders use a deterministic cheapest-first policy | The fight-length question — how many hexes a spell should take is a feel question nobody has played with yet |
| **Enemy behaviour** | deterministic `baseline-v1`: revive, reveal, direct-damage cast, self-enchant, strike, then approach an observed live hostile | A rout threshold to know when to stop and a broader tactical policy; this remains a deliberately small baseline rather than a balance decision |
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

Permanent construction now reaches terrain through exact inclusive `TilePos` and
`RunBottom` occupancy. Evocations using `Single` or `Column` publish atomic
`TerrainEdit::Set` batches for map-approved conjurable substances. Hidden material or
units suppress an unsafe batch without changing acceptance or payment; elemental
terrain announcements and response outcomes remain downstream.

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

- **Trajectories are obstruction-aware; effect volumes are not clipped.** `Direct`
  and authored-rise `Arc` casts test exact material occupancy with one
  direction-symmetric integer supercover, while `None` deliberately bypasses it.
  Authoritative casting uses complete `RunBottom` occupancy; preview anchors, target
  cycling, and AI use only explicitly Observed material positions, so Unknown terrain
  cannot change faction-facing choices. Authored range and arc rise are technically
  capped at 16. A sphere may still include rock and the chamber beyond it after its
  anchor is reached; per-voxel volume clipping and obstruction-aware sight remain
  later work.
- **A breached cave roof will not admit daylight.** Terrain edits already keep the
  interior *roof* projection current, but interior **membership** is never re-derived,
  so a chamber you blow open still counts as inside. Live perception therefore
  continues to classify the chamber as Interior and does not admit daylight
  ([boundary.md](boundary.md) ask I).
- **Casting is provisionally combat-only.** Recovery between fights is intended to be
  a rest action, but real-time casting still needs an interaction and rest flow.
  **Rituals remain deferred** — `co_castable` parses and labels rituals in the demo,
  but has no mechanical effect.
- **Paid-on-resistance is provisional.** The first wave charges mana and the action
  after a legal announcement even if every material resists.
- **No-undermining is provisional.** Permanent evocation construction checks its
  complete volume and emits no edits when it intersects existing material, a unit body,
  or a unit's supporting surface. The cast remains accepted and paid so hidden blockers
  are not an oracle. Destructive terrain impacts still wait for falling and footing
  reconciliation.
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
  it; the terrain side separately waits on the gameplay announcement and legality
  adapters now that exact run bounds are published.
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

Most of what makes this a product does not exist yet: no durable saves, audio content,
input rebinding, signing, or store packaging. Wave 5 now provides one atomic,
build-bound exploration resume, a persistent settings menu, centralized fixed input
actions, empty audio buses, normalized release artifacts, and retained symbol
material. The first hygiene
slice has landed — a per-session log file beside the executable, a panic hook that
writes into it, and the version on the title screen — but full crash *reporting*
(symbolication, upload, a dialog) has not. These replaceable seams do not close the
production gap or promise compatibility. The full checklist and evidence remain
frozen in [production-audit.md](production-audit.md); the sequenced scaffold is in
[roadmap.md](roadmap.md).
