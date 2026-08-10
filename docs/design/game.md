# Hex — game design

The design this codebase is built toward. Kept in the repo because it is what
anyone, human or agent, needs in order to make a choice that fits the game rather
than merely compiles.

> **Status: a working document.** Large parts are unresolved and marked so. The
> [Open questions](#open-questions) section is not a backlog of things nobody got to —
> it is a list of decisions that have deliberately not been taken, and building past
> them would settle them by accident.

> **One renaming.** The notes below originally called a character's hex grid their
> **core**. In code and in the rest of the documentation it is a **lattice**, because
> `hex_core` is already a crate and the collision would be permanent. A crystal
> lattice is a structured arrangement of gems, which is what the thing is, and it
> carries the connectedness that adjacency-based power depends on. See
> [systems/combat.md](../systems/combat.md).

---

## Overview

Hex is a turn-based game where the world is defined by patterns of tessellated
hexagons. The map, characters, spells and elements are all represented by hex grids.
Each character or enemy is defined by a hex grid that determines what elements they
can use, how they can combine them, and how they power spells.

A party of up to six player characters travels an isometric hex grid, battling
enemies and completing quests. The loop should feel like Baldur's Gate 3 — real time
when characters are not engaged in combat or a timed action, turn-based when they
are. The differences: this is on a hex grid, and there is little to no dialogue. The
end state should feel like a retro indie game even with sophisticated mechanics and
multiplayer (people control a subset of characters, as in Baldur's Gate).

Most customisation comes from the **lattice** that defines a character or enemy. It
consists of gems holding elements, fusions that create higher-order elements, and
spells that consume them.

**There is no HP.** Damage disables hexes, which prevents casting and powering. The
current playable rule removes a fully disabled character from the turn order as
**downed** and leaves the lattice available for restoration. Whether downed becomes
functional death, permanent death, or a recoverable campaign state remains an
explicit design question below; the overview does not settle it by implication.

---

## Elements and spells

### The canonical elemental grid

The six basic elements follow this clockwise inner-ring order:

**Air · Fire · Metal · Earth · Life · Water**

Elements three steps apart are opposed: Air/Earth, Fire/Life, and Metal/Water. The
wheel defines and exposes those relationships, but the current combat rules do not
yet grant an opposition-triggered recharge or other opposition bonus.

The complete initial catalog has six direct pair fusions and six direct triple
fusions:

| Kind | Result | Direct basic inputs |
|---|---|---|
| Pair | Lightning | Air + Fire |
| Pair | Volcano | Fire + Metal |
| Pair | Crystal | Metal + Earth |
| Pair | Transmutation | Earth + Life |
| Pair | Divination | Life + Water |
| Pair | Illusion | Water + Air |
| Triple | Destruction | Air + Fire + Metal |
| Triple | Artifice | Fire + Metal + Earth |
| Triple | Necromancy | Metal + Earth + Life |
| Triple | Wild | Earth + Life + Water |
| Triple | Storm | Life + Water + Air |
| Triple | Space | Water + Air + Fire |

Every recipe in this catalog draws directly from two or three distinct basic
feeders. A triple is not a pair-plus-basic chain.

The Creator may present these elements as a radius-two chart with an empty centre.
Its inner ring is the basic wheel above; its outer ring, beginning along the
upper-left edge and proceeding clockwise, is Space, Lightning, Destruction, Volcano,
Artifice, Crystal, Necromancy, Transmutation, Wild, Divination, Storm, and Illusion.
That layout, its colours, and its vector glyphs are presentation aids. Stable names,
the wheel, and the authored recipes remain the gameplay authority.

### Gems and adjacency

Gems hold mana of a single element. Spells draw power from **adjacent** gems.
Adjacency is the entire power mechanism — there is no action at a distance within a
lattice.

A spell's tier is how many adjacent source cells it requires: a matching gem for a
basic spell or a matching fusion for a complex-school spell. Ember needs one adjacent
Fire gem. Fireball needs six: a full ring, the spell hex completely surrounded.

Every starter spell and first pair/triple showcase the design owner names for this
program is **tier 1**. A basic-school spell requires exactly one adjacent gem of that
school. A pair- or triple-school spell requires exactly one adjacent fusion cell of
that school; the fusion's two or three basic feeders build that source and are not
additional spell requirements. Higher-level spells use more adjacent source cells.
A lattice may still include independent one-gem funding groups so a sustained
enchantment does not lock the source needed by another tier-1 spell.

This makes layout a packing puzzle with hard geometric consequences:

- A six-gem spell costs seven hexes.
- **Two six-gem spells can never be adjacent.** Each would occupy one of the other's
  six slots, capping both at five.
- The tightest legal packing for two maxed spells is distance 2, sharing two gems:
  twelve hexes instead of fourteen.

**Shared gems compete for fuel.** A gem adjacent to two spells can power either, but
it is the same mana. Tight packing fields more spells per hex of capacity and
sustains fewer casts. Sprawl is less efficient and fires repeatedly.

That is also the resilience trade-off. Because damage disables hexes, a tightly
packed lattice is fragile — one shared gem going down silences two spells. Redundant
sprawl survives longer and does less. **This tension does the work a hit-point bar
does in other games.**

### Fusions

A fusion hex combines its adjacent feeders into one authored higher-order element.
Lightning, for example, requires adjacent Air and Fire gems; Space requires adjacent
Water, Air, and Fire gems.

- A fusion is a **build commitment** — its output is fixed when inscribed, not
  chosen dynamically from whichever neighbours are live.
- It costs a hex of capacity.
- Fusion hexes are prime disable targets. A fusion sits between its feeders and its
  consumers, so breaking any link kills everything downstream. Long chains are
  efficient and brittle — gem sharing, one level up.

Complex-element spells require **very few** fusions of that element, or their feeder
geometry becomes unbuildable. A six-source complex spell would need six fusions with
their own feeders, far beyond an ordinary lattice.

The first complex-school showcases therefore scale by **recipe complexity, not
volume**: each consumes one fusion source whose feeder geometry already carries the
school's complexity. Later higher-tier spells scale by volume through several
adjacent source cells and may add further recipe complexity deliberately. The
initial catalog stops at direct two- and three-basic recipes; a later chained recipe
would be a deliberate extension rather than an alternate way to build one of these
twelve elements.

### Evocation and enchantment

**Evocations** (fireball, ember) drain mana from adjacent gems and consume it. Their
cost is **throughput**, recoverable by channelling.

**Enchantments** (earthen wall, metal shield) tie mana up in a gem for as long as they
last. Their cost is **capacity** — that part of the lattice is spoken for. Some carry
an upkeep.

- When an enchantment breaks, its locked mana is consumed.
- **If a gem holding an enchantment is disabled, the enchantment breaks** and the mana
  is lost. Many enchantments are defensive precisely to prevent this and are
  themselves vulnerable to it. A hit that cracks a shield costs the shield, the mana
  funding it, and the hex.
- Enchantment-heavy builds want dedicated gems and sprawl; evocation-heavy builds want
  tight packing. Two archetypes falling out of the geometry.

### Binary casting and rituals

Most spells are **binary**. If fireball needs six adjacent fire gems and one is
disabled, fireball is offline. No degraded casting.

**Rituals** are the exception: variable mana for varied effect — flamethrower, wall of
fire. Multiple characters may cast together.

Rituals have a structural role beyond flexibility. Binary spells wink out one by one
as gems go down; a flamethrower simply gets weaker. **Late in a fight, rituals are
what a battered character has left.**

> *Naming note: variable cost and co-casting are separable axes and may not always
> co-occur. "Ritual" currently bundles both.*

---

## Characters and enemies

A lattice is a fixed number of contiguous hexagons, determined by three stats:

- **Capacity** — how many hexes the lattice has
- **Attunement** — which elements can be stored and cast, and how much
- **Channelling** — how much of each element a channel action recovers

On level-up a character raises one. The first attunement in a new element grants one
channelling in it for free; more requires further points.

---

## Combat

### Damage

There is no HP. Damage disables hexes.

- Spells disable a **flat count**, roughly 1–4. Ember disables 1 and burns for two
  affected-unit turns; fireball disables 3.
- **The defender chooses which hexes are disabled**, except for abilities that target
  hexes directly. Those are the counter to tight packing — a shared gem is a
  two-for-one kill and no defender surrenders one voluntarily. They should be rare and
  expensive.
- **All hexes are equally durable.** No per-hex stat; differentiation lives in
  enchantments.
- Damage type does not matter, except fire's burn.

### Nothing is aimed away from you

There is **no friendly-fire filter**. A spell may be aimed at anyone, and an area
effect touches everything inside its volume — allies, enemies, and the caster alike.
Healing an enemy is allowed and is your own fault; a fireball dropped on a melee is a
decision, not a mis-click the game will protect you from.

That is what makes area spells a real choice rather than free damage, and it is why
positioning is a defensive tool. How the rule is enforced — and the volume an area
effect actually covers — is [casting.md](../systems/casting.md).

### Defences subtract

Defensive enchantments reduce incoming disable counts by a flat amount. A metal shield
reducing by 1 turns fireball into a 2 and ember into nothing.

Flat subtraction gives threshold behaviour for free: small spells bounce off defended
targets, big spells punch through. Chip damage is worthless, heavy hits decisive — the
same categorical-not-incremental philosophy as spell tiers, emerging rather than
imposed. Stacked shields make a target impervious to small arms, which makes breaking
defences the correct opening move and gives rituals an obvious job.

### Burn

Fire's damage over time: one additional hex disabled at the start of the target's
turn, for some number of turns. Same currency as everything else, no second system.

**Burn is not reduced by armour.** Fire's identity is beating defences by ignoring
them rather than overpowering them.

### No randomness in resolution

**No to-hit rolls. No damage variance.** The game already has a better source of
uncertainty: enemy lattices are hidden. Combat should feel like chess with fog rather
than a slot machine — and when a player commits several actions to a ritual, the only
thing that can go wrong is something they could in principle have known.

### Recovery and death

- Hexes recover through healing spells or rest after combat. Canonical Heal is a
  tier-one Life spell: a fixed-mana Evocation that restores one chosen disabled cell
  on the caster or on an Observed occupied unit across one mutually traversable
  footing edge. It may heal a hostile only when that hostile's complete current
  lattice is known.
- The long-term consequence of total disablement is unresolved. Permanent death unless
  reversed remains one candidate; the prototype uses restoration-ready downing.
- **Proposed:** functional death arrives before zero. A character whose spell hexes
  are all offline can still channel but cannot act on the world. The threshold emerges
  from the mechanics rather than being imposed, makes the last few hexes a grace
  period rather than a slog, and gives enemies a legible rout condition.
- **Provisional first implementation:** a unit whose hexes are all disabled leaves the
  turn order and is **downed**, retaining its unit and lattice. Heal or Renewal restores
  chosen cells and returns the unit at the next round boundary; exploration Rest
  recovers the party immediately. This is a testable starting behavior, not the answer
  to functional death or the [permadeath question](#permadeath).
- **Ruled 2026-07-27: out-of-combat recovery is an explicit rest action.** Channelling
  is a per-turn model and has nothing to say about the time between fights, so the
  alternative was inventing a regeneration curve before there was a fight to pace it
  against. Rest doubles as a testing affordance: it is the shortest path from "a fight
  ended badly" back to "try that again". It does **not** settle whether casting is
  possible outside combat — that question stays open.
- **Ruled 2026-07-27: rout and surrender are deferred.** Both are named above as brakes
  on the negative spiral and as the thing that ends a fight before the slog, and both
  need a threshold number nobody can pick honestly yet. `rout_policy` stays an unbuilt
  knob in `combat.ron` that parses and fails with a reason. The known consequence is
  that a melee enemy which cannot reach the party never gives up, so a fight can
  stalemate — a recorded gap, not an oversight.

### Information and divination

Enemy lattices and intent are hidden by default. The Divination school owns spells
that reveal them.

Spatial perception and lattice knowledge are separate. Seeing an enemy establishes
where it is and permits targeting; it does not reveal the enemy's lattice or intent.
The sun, moon, caves, local lights, faction memory, and loss of contact follow the
[perception contract](../systems/perception.md). Divination changes the separate
lattice-information channel.

Observation is intended to gate combat without replacing distance: an observed
hostile pair must also satisfy `engage_range` to start combat. Casting anchors,
hostile identities, and AI already consume authoritative observation. The remaining
engagement, ordinary-attack, and one-round lost-contact adapters are not wired yet;
the prototype currently starts and ends combat from distance and
`disengage_margin` alone.

- **Scrying Eye belongs to Divination** and requires one adjacent Divination fusion
  source. Its current playable form targets one
  already-observed unit and writes a complete, bounded lattice projection. Keeping
  that feed readable and live after the subject leaves ordinary sight is later
  Divination work, not part of the elemental-grid foundation.
- What a later divination reveals may scale with tier: full or partial lattice, one
  enemy or several, and expiring versus sustained knowledge.
- Every faction has the same base spatial sight profile, pooled across its active
  characters; divination changes lattice knowledge, not that base sight rule.
- Illusion is a separate Water + Air school. Its first planned showcase spell,
  Invisibility, requires one adjacent Illusion fusion source. Spell-created
  illumination also belongs to Illusion, but no such
  spell ships in the foundation wave; the former Daylight spell is withdrawn rather
  than migrated into the new catalog.
- Consumables can eventually patch structural weaknesses such as missing Divination
  or revival at high cost.

### Enemies

**An enemy's lattice is its entire stat block.** There is no separate authoring
system. A wolf is four hexes with a bite. A raider is eight with a metal shield. A
hedge-mage is thirteen with direct Lightning and Divination fusions plus Scrying Eye.
Difficulty is the size and complexity of the drawing, and every enemy runs on the
player's rules.

This makes the information layer self-balancing. Small enemies are learnable — once
you know a wolf, divining one is a wasted action. Bosses and novel enemies stay
genuinely unknown, so Divination investment pays off exactly where it should.

Most enemies are weaker than playable characters and have a **surrender or fatality
mechanic**. With no dialogue, surrender must read through posture and animation.

### Bosses

The boss problem is action economy, not durability. Six characters against one boss is
a 6:1 action ratio; inflating a boss to capacity 40 produces a longer slog, not a
harder fight.

**Candidate solution:** if initiative derives from lattice size, a large lattice
naturally earns several slots in the turn order. The boss's size *is* its action
economy, and one mechanic solves both.

What makes it attractive:

- It **degrades as you damage it**. Every hex removed costs durability *and* actions,
  so boss fights accelerate: grinding while intact, collapsing once broken.
- It is **readable without text**. The initiative track is already on screen; the
  player watches the boss's slots disappear. A health bar nobody had to build.

Refinements it needs: everyone gets one slot baseline with extras at capacity
thresholds, or a four-hex wolf gets less than a full turn. Pairs well with
**segmented lattices** — semi-independent clusters, each worth a slot, so disabling a
segment removes an action and gives discrete phase transitions with no scripting.

Costs: independent tuning is lost (adding hexes makes a boss act more often — the
knobs are welded together), it needs a floor so the last phase is not a punching bag,
and multiple turns for one unit needs strong telegraphing or it reads as cheating.
**It only works if initiative derives from lattice size rather than from Air
specifically.** If Air-derived, a huge earth golem is slow — a fine fantasy, arguably
a better one, but then the boss mechanic applies unevenly.

---

## Map

An isometric hex grid. **One map**, used both for travel and for combat. Tiles have
properties and elevations:

- traversable by all units (grass, floor, stone) or only some — a swamp only for
  specific units or after a spell, lava only for flying units or after a spell
- gameplay illumination, separate from rendered light, feeding the
  [perception mechanic](../systems/perception.md)
- whether an evocation or enchantment can be cast there — most tiles allow
  evocations unless they have special properties like an anti-magic field; fewer
  allow enchantments, since a fixed stone wall cannot be cast on water

### Crystal Ascent landmark

Crystal Ascent is a vertical transition biome intended to become one landmark inside
a larger authored world. A monumental aperture enters at the bottom into an open,
dark chamber dominated by one large cyan crystal. The playable route does not cross
the central void: it coils upward in three broad circuits along the enclosing walls,
contracts inward like a Gothic cathedral, and emerges through an oculus into a wooded
summit clearing. Smaller landing crystals make the route readable without turning the
whole interior into daylight.

The cathedral heart is a tall, asymmetric prism rather than a concentric pyramid:
its fractured footing narrows quickly into a long irregular body and an off-centre
crown. The stair tops remain four hexes wide, but their carved worked-stone support
deepens across the route toward the wall instead of ending as a one-voxel shelf. The
summit opening is a compact radius-11 oculus.

The standalone scenario fixes the climb at 144 voxel levels while the authored recipe
accepts 100 through 200 for later composition. Four-wide stairs and landings support a
full party, ordinary transitions remain flat or one level, and each contraction
retains four independent transfer lanes. The lower exterior apron and upper terminal
remain opposite one another, with the Dark interior beginning just inside the lower
threshold. On a fresh showcase launch, the standard party occupies three exact cells
of that four-wide apron and faces inward; save restoration remains authoritative over
that staging. The cathedral heart is physical world volume:
characters cannot enter it and sight cannot pass through its interior. Small crystals
are presentation and illumination fixtures rather than blockers. Map, Third Person,
and First Person remain ordinary camera choices; no automatic gameplay cutaway is
introduced for the tower.

### Magic shapes the world; the world decides how

**Evocations make persistent terrain changes.** They last at least across multiple
turns rather than vanishing with the casting animation. The initial implementation
makes applied terrain edits permanent: conjured stone is simply stone until something
changes it again. It initially represents enchantment manifestations as bound
entities that vanish when the enchantment breaks, but that implementation split is
provisional.

**A cast announces; the world answers.** A spell says which voxels it reaches and what
kind of energy arrives there; what the *material* does about it is the world's own
rule. Fire on dirt and fire on granite may produce different outcomes that a fireball
does not need to predict. How the first wave charges a fully resisted cast is
provisional rather than a permanent design rule. Generated feature effects, including
burning trees and tall grass, are deferred.

Conjuration still names its material because that is part of the spell's identity, but
the world explicitly marks which substances are admissible for spell content. Merely
defining bedrock, water, or lava does not make it conjurable.

Every cast anchors on a currently Observed exact position. An area resolved from that
anchor may extend into hidden terrain and affect hidden units, including allies, but
its acknowledgments, presentation, and logs cannot reveal those hidden outcomes.

The full contract is [casting.md](../systems/casting.md).

Sight uses an asymmetric grid-space range volume. Horizontal distance and only the
upward part of vertical distance combine by the inclusive squared rule; downward
vertical distance is ignored. The result is a downward cylinder with a spherical
half-dome above it. Target illumination selects radius 36 in Bright conditions, 12 in
Dim conditions, and 1 in Dark conditions.

Characters travel in a formation. Once combat starts, controls switch to moving each
character independently.

Range alone does not establish observation. For every in-range target, exact terrain
material must leave clear either the character-head-center to target-top-center ray or
at least three of six rays from the matching corners of the standing body's top face
to the target top face. For character LOS only, the exposed top voxel of a material run
is low cover when that run tops out within one level of the observer's support and has
material directly beneath its top. Its deeper core remains solid. A disconnected
one-voxel platform, or a run topped farther away—including character-height walls and
vertically remote roofs or decks—keeps its full blocking volume. There is no separate
near-distance exception, and exact tangencies are clear. Because low cover is
classified relative to the observer, observation may differ in the reverse direction
even though the underlying exact segment test is symmetric. What the camera happens to
frame is presentation, not knowledge.
The tactical map intentionally keeps current terrain visible and pickable under a dark
shroud while withholding unobserved hostile units and observation-only cues.

Each tile type should be distinguishable by colour and design. A tile is a **3D prism
with a hex base**, so it has five coordinates: cube coordinates horizontally (see
<https://www.redblobgames.com/grids/hexagons/>), and an `h_min`/`h_max` for the lower
and upper extent.

Because there are heights, the player must be able to **rotate the view**.

---

## Open questions

### Negative spirals

**The biggest structural concern.** Losing a hex costs offence, defence, information
and potentially actions at once — four losses from one hit, and they multiply.
Combined with defender-chooses (early damage is nearly free because you dump junk
hexes; late damage is catastrophic because everything left is load-bearing) and
enchantments breaking on disable, combat may read as: nothing, nothing, nothing,
collapse.

Some of that is desirable — it makes breaking through defences the whole tactical
problem, and suits a game with no HP. But the accelerants stack, and permadeath is on
the other end.

Brakes already proposed by the design: rituals can function on a degraded lattice,
channelling can remain available, rout and surrender can end fights before the slog,
and healing can restore hexes mid-combat. Two are now playable: Channel spends one
action to recover live, unlocked gems, and Renewal can restore chosen disabled cells
and return a downed unit at the next round boundary. Rituals, rout, and surrender
remain deferred.

Additional candidates: desperation effects that strengthen as a lattice weakens, a
floor on boss action count, and cheap partial recovery as a standard action.

**Ruled 2026-07-27, updated 2026-07-30:** the initial missing brakes were deferred
because you cannot tune a spiral you have not felt. Channel and Renewal/Restore have
since landed as provisional playable brakes without deciding the remaining policy.
The loop now includes defender-chosen disables, downing, Burn, Reveal, recovery, and
the combat readouts. Whether it reads as *nothing, nothing, nothing, collapse* is a
deterministic-simulation and bounded Sandbox playtest question. Rituals, rout,
surrender, and any additional brake should answer that evidence rather than precede
it.

### Initiative

Unresolved and crucial.

The current prototype is a deterministic baseline: each active unit has one slot,
ordered by its authored initiative and then stable unit id. It has no roll, round
reroll, hold/delay action, or boss multi-slot rule. Default-off deterministic cases
and explicit design playtests should compare alternatives against that baseline
rather than treating it as the final design; ordinary Sandbox always uses shipped
rules.

- **One roll per combat, fixed.** Plannable, but a bad roll can kill a character under
  permadeath, and "improvable by air spells" has a timing problem — order locks before
  anything can be cast.
- **Derived from the lattice** (Air attunement, or total capacity). Makes turn order a
  build decision rather than a dice decision, enables the boss mechanic, gives Air a
  real identity as the tempo element.
- **Re-rolled each round.** Removes the ruined-by-one-roll problem and makes
  order-manipulation tactical, but makes co-casting a dice game.
- **Fixed order with a hold/delay action.** Gives co-casting an explicit sync tool.
  Pairs naturally with the derived option.

Co-casting raises the stakes: if order is unpredictable, a three-person ritual can be
broken by a well-placed hit before the third caster contributes.

### Action economy

The same question as initiative in disguise: how scarce is a turn?

The current prototype permits up to four movement steps plus one action. Wave 7's
historical two-step and bounded-Custom experiments retained the four-step shipping
value because the deterministic matrix proved profile fidelity but supplied no
balance preference. Optional profile injection remains test-only, while Sandbox
freezes the shipped value. That is an experimental baseline, not a settled answer;
see the [tempo decision audit](../development/wave-7-tempo-decision.md).

- **Strict one action** (move *or* cast *or* channel). Maximum tension, but movement
  effectively stops happening — which strands elevation, illumination and terrain.
- **Free small movement (1–2 hexes) plus one action.** *Current recommendation.*
  Preserves the scarcity that makes big spells categorical, keeps the map
  load-bearing, makes rituals affordable, least bookkeeping.
- **Move plus action**, channel as the action. Most legible, loses the cast-vs-move
  tension.
- **Action points** (~3/turn). Most flexible, but converts categorical spell tiers
  back into linear pricing.

Related: should channelling passively trickle at turn start, with the channel action
as a burst refill? That removes dead turns without removing the choice.

### Fight length

A capacity-14 character takes 5–7 landed hits to kill. Four enemies is 20+ damaging
actions. A party generating 3–4 damage actions per round puts a standard fight north
of 8 rounds, where Baldur's Gate 3 resolves in 3–5. Either fights are simply long
(defensible, harsher under permadeath), lattices are smaller than assumed, spells
disable more, or the rout threshold arrives well before zero.

### Permadeath

Tabled. Six characters with permanent death is severe for a campaign this length. Is
there a recruitable roster? Do you play down a member? Is a death a run-ender? How
accessible are revival spells — that number sets the whole difficulty tone.

### Map shape

Open world, hub-and-spoke, or chapters — undecided. The gameplay loop largely depends
on this.

### Ritual terminology

"Ritual" bundles two separable properties: variable mana cost and multi-caster
contribution. Decide whether every variable spell is co-castable, or whether these are
two tags that sometimes co-occur.

### Surrender consequences

What can the player do with a surrendered enemy? If killing them is free, surrender is
a victory banner. If sparing has a payoff — they flee, or something accrues across the
campaign — the choice has teeth. Probably a quest-design question.

### What a fight yields

Unanswered, and **ruled 2026-07-27 that it stays unanswered for now**: a fight ends in a
victory or defeat screen and a return to exploring, and yields nothing else. No loot, no
experience, no currency.

The ruling is about sequencing rather than taste. Rewards are the tightest coupling in a
campaign — they set progression pace, they decide whether an avoidable fight is worth
taking, and content starts depending on them the moment they exist. Inventing that
vocabulary before a fight is playable would be guessing at all three. The concrete
consequence is that the encounter schema carries **rosters and placement only**; adding
rewards later extends it without breaking any file, which is exactly the property that
makes waiting cheap.

Related, and equally open: whether a fight is avoidable at all, which is the same
question as [surrender consequences](#surrender-consequences) from the other end.

---

## Implementation status

Current implementation status, including provisional rules and known gaps, is
maintained in [planning/status.md](../planning/status.md).
