# Casting

How a spell becomes a change to the world: what makes a cast legal, what shape it
affects, who decides what happens to the material inside that shape, and how effects
that outlive their turn are expressed.

> **Status:** this is the normative contract for wave 3's casting work. The pieces
> that exist today are marked **built**; everything else is a contract, not a
> description. Nothing in the shipped game casts a spell yet — `GameCommand::Cast`
> parses and is rejected with a reason.

Read [the design](../design/game.md) for the magic system this serves, and
[combat.md](combat.md) for the turn loop a cast happens inside.

## The one-sentence version

**Gameplay owns geometry; the world owns materiality.** A cast announces an exact set
of voxels, and what the material inside them does about it is the world owner's
decision.

That single line settles most of the questions below, and it is the reason casting can
be validated before it is announced: we always know *which* voxels, even when we do not
know what will become of them.

## Casting is committing

A cast that reaches the world is paid for. Mana is spent, the action is taken, and the
presentation plays — **whether or not anything changes**.

A fireball thrown at bedrock is a legal cast that scorches nothing. It is not a
refusal, not an error, and not a bug: the caster committed, and the mountain won. This
is deliberate, and it is what keeps gameplay from having to know which materials yield
to which forces — a table gameplay has no business owning.

Partial application is therefore normal. A blast across a hillside may clear the dirt
and leave the granite ridge running through it untouched.

## Two write paths

Terrain reaches the world through exactly two messages, and the split is not
arbitrary — it tracks whether there is a material with an opinion.

| Path | Message | Who decides the outcome |
|---|---|---|
| **Conjuration** | `TerrainEdit::Set` — **built** | Gameplay names the substance and the volume; the world validates placement |
| **Elemental effect** | `TerrainImpact { batch, volume, element, power }` — *contract* | The world owns the entire material response |

`batch` is the id the world echoes back in its acknowledgment, so an outcome can be
matched to the announcement that caused it. Both messages are specified in full in
[boundary.md](../planning/boundary.md) asks G and H.

**Destruction has a counterparty; creation does not.** When fire meets a voxel, that
voxel already has properties its author defined, so the response belongs to that
author. When a mage conjures, nothing is there to respond — the material *is* the
spell's identity. "Earthen Wall" summons dirt because that is what the spell is, and if
that lived in a world-side response table, the spell's identity would drift out of the
spell file.

So conjuration keeps naming substances in `spells.ron`, where
[`ContentIndex`](../../crates/hex_assets/src/content_index.rs) already validates the
cross-file reference (**built**), and elemental effects name only an element and a
power.

**`TerrainEdit::Clear` will not be used by spells.** Destruction flows through
`TerrainImpact` so the world owner arbitrates it; `Clear` remains for save restoration
and authored terrain, where the exact outcome is the point.

That is a change to shipped content, not just a rule: **"Stone Shaper" currently
carries `effects: [SetTerrain(substance: "stone"), ClearTerrain]`**, and its
`ClearTerrain` becomes an `Impact` when terrain magic lands. Whether `Effect::ClearTerrain`
survives as a variant at all is decided then — nothing else uses it.

Power will be an explicit content field — `Impact(element, power: N)`, a variant
`Effect` does not have yet — so designers tune it directly rather than having it
inferred from tier. Any defined substance is conjurable; balance lives in a spell's gem
cost and tier, not in a whitelist.

## Evocations shape the world; enchantments do not

- **Only evocations write terrain**, and every terrain edit is **permanent and
  atomic**. There is no conjuration ledger, no provenance tracking, and no un-build:
  conjured stone is simply stone, destroyed the way any stone is.
- **Enchantments never touch terrain.** Anything an enchantment manifests is an
  *entity* bound to its [`EnchantId`](../../crates/hex_core/src/lattice_ids.rs)
  (**built**), and enchantment start and break are announced like any other effect —
  when the enchantment breaks, whatever it manifested despawns with it.

This deletes an entire class of problem. The alternative — enchantment-conjured terrain
that must be un-built when the enchantment falls — needs a ledger, needs provenance,
and has to answer what happens when someone else reshapes those voxels in between.
Splitting by *what kind of thing is created* means the question never arises.

Honest caveat: an entity-shaped barrier does not block movement, because units do not
obstruct each other yet ([status.md](../planning/status.md)). Terrain walls are the
blocking tool until occupancy exists.

## The legality ladder

Five rungs, checked in order. The emitter pre-filters what it can for click feel; the
applier in `hex_combat` is authoritative.

1. **Actor** — whose turn it is, seat ownership, not `Busy`, action available.
   **Built** — this is the command funnel's existing validation.
2. **Lattice** — [`castable()`](../../crates/hex_lattice/src/cast.rs) returns a
   `CastPlan` or a blocked reason the UI can show. **Built** (unwired).
3. **Targeting** — the anchor is in range, the shape resolves to a voxel set, and the
   trajectory is clear. Range uses
   [`in_reach`](../../crates/hex_units/src/targeting.rs) (**built**), so **spells
   inherit high-ground-buys-range automatically** — the rule was written for this and
   has had exactly one consumer until now, engagement. Trajectory is deferred; see
   *Obstruction*.
4. **Unit interaction** — **terrain-creation voxels that intersect a unit's body are
   illegal.** A single-target shaping spell aimed at an occupied hex is refused *before
   mana moves*; an area spell drops only the conflicting *terrain* voxels and still
   applies its other effects, because a fireball must stay castable in a melee.

   Also refused: edits to any voxel that is a unit's supporting surface. **Nobody can
   be entombed or undermined.** That one clause deletes falling rules, post-edit
   footing reconciliation, and a class of races with the world's re-meshing — all of
   which can be relaxed deliberately later if undermining becomes a mechanic worth
   designing.
5. **Announce** — the surviving volume goes to the world, which arbitrates.

Rungs 1–2 are gameplay's own state. Rung 4 is gameplay's knowledge too: **where
characters stand is ours**, so a cast interacts with units through legality, exactly as
movement does — a character cannot walk through a wall, and a wall cannot be conjured
through a character. Only rung 5 defers, because only material response is the world's.

### Observation

A unit-directed cast requires the acting faction to observe its target
([perception.md](perception.md)). The rung is written now as one function that returns
`true` — which is **not a stub but the truth**, since no fog exists yet and every unit
genuinely is observed by everyone. It becomes a real query when `hex_perception` lands,
and that is a one-function change inside `hex_combat`.

### Occupancy

Rungs 4 and 5 both need to know which voxels are solid, and gameplay cannot currently
answer that: tiles publish each run's *top* (`TilePos`) but not its *bottom*, and
`Headroom` saturates. `RunBottom(Level)` is the missing datum — see
[boundary.md](../planning/boundary.md) ask C. It is the keystone for casting legality,
conjuration placement, trajectory, cover, and pathing alike.

## Volumes

Every effect resolves to a **3D voxel volume**, because a world with bridges, cave
floors and sky islands has no flat answer to "what did the blast touch".

**The metric is grid-space**: horizontal hex distance and vertical level distance count
equally. A world-space sphere would require `level_height`, and gameplay is forbidden
from knowing it — that is precisely the dependency the crate split exists to prevent.
So a radius-3 sphere reaches three hexes out and three levels up or down, and looks
slightly squashed on screen. That is the correct trade.

| Shape | Volume |
|---|---|
| `Self` | the caster's own voxel |
| `Single` | the anchor voxel |
| `Sphere(radius)` | grid-space ball around the anchor |
| `Column(height)` | the anchor and the voxels above it |
| `Line(length, width)` | from the caster toward the facing |
| `Cone(length, spread)` | widening from the caster toward the facing |
| `Path(offsets)` | an authored offset list, rotated to the facing |

`Path` rotates in sextants — 60° steps are exact on cube coordinates, so an authored
pattern keeps its shape in all six directions.

**One volume affects everything inside it**: units, the terrain announcement, and
features alike. A blast that reaches a bridge deck hits whoever is standing on it.
Vertical reach is the entire point of describing volumes rather than footprints.

Conjured walls are **2 voxels tall**. The canonical walker is 2 tall and climbs 1, so a
1-voxel wall is a step, not a wall.

### Obstruction

**Volumes are geometric in wave 3** — a sphere next to a cave wall fills voxels inside
the rock and the chamber beyond it. This is wrong, it is documented as wrong, and it is
bounded: obstruction-aware clipping arrives with the same line-of-sight work that
`RunBottom` unlocks, and `needs_los` on `TargetingSpec` (**built**, parsed) is unenforced
until then.

When that lands, sight and spell trajectories should share **one** raycast primitive.
Two independently-written line algorithms that disagree about grazing a corner is a bug
nobody will find for months.

## The command

Casting goes through the funnel like every other intent ([combat.md](combat.md)):

```rust
Cast {
    unit: UnitId,
    spell: String,            // by name — ids are session-local
    target: TilePos,          // one positional anchor, always
    facing: Option<Sextant>,  // Line, Cone, and Path need orientation
    mana: Option<u16>,        // the choice a Variable-mana spell requires
}
```

`Sextant` is a six-value direction type `hex_core` **does not have yet** and this
contract introduces. Today the only notion of direction is the fixed order of
`HexCoord::neighbors()`, which returns `[Self; 6]` — the new type names those six
positions so a `Path` shape's rotation is expressed in the domain rather than as a
bare index.

A unit target resolves to the voxel that unit stands on, so there is one target
vocabulary rather than two — which is forced anyway, since interior voxels have no
entity and targeting is therefore positional ([map.md](map.md)).

The payload will grow. It grows through **optional fields with serde defaults, or new
command variants** — never through speculative fields added early, because the command
log is the replay log and every field is a permanent save commitment.

## Persistent effects

Damage over time, enchantment upkeep, and decaying divination are the same system wearing
three hats, so they are built once:

```
{ source, target, payload, start, end }
```

Vocabulary in `hex_core`, runtime in `hex_combat`, lattice payloads applied through
`hex_lattice`'s existing functions — the same split the command funnel uses.

- **End conditions in wave 3**: after N rounds, or bound to an enchantment (ending when
  it breaks). Area-lingering zones and dispel effects come later.
- **Tick point is per payload.** Some effects are personal and tick at the start of the
  affected unit's turn — which is exactly how the design words fire's damage over time —
  and some are global and tick at round boundaries.

`Burn` is a payload, not a special case. What burning *means* can be redefined freely
without touching the framework, which is the point of having one.

## Rulings worth writing down

- **There is no ally/enemy targeting filter, and there will not be one.** You may heal
  an enemy and immolate a friend. Area effects hit everyone inside the volume, always.
- **Casting is combat-only** in wave 3. Shaping terrain out of combat is attractive and
  waits on an answer to out-of-combat mana regeneration, which channelling's per-turn
  model does not provide.
- **Channelling and rituals are deferred.** `co_castable` parses and feeds
  `Spell::is_ritual()` (**built**, and read today only by the lattice demo and the dev
  content dump); it has no mechanical effect. Co-casting is entangled with the
  unresolved initiative question.
- **Death** removes a unit from the turn order and leaves it downed, revivable by a
  restoring spell. Permadeath is a separate decision.
- **`Reveal` and `Illuminate` reject with a reason** naming what they wait on: the
  knowledge seam, and spell-created lights in the perception lane. They are in the
  shipped roster and must not silently do nothing.

## Verification gate

Legality tests cover each rung independently and in combination: a cast refused for
lack of mana must not also consume the action; a shaping cast onto an occupied hex must
refuse before mana moves; an area cast in a melee must still apply its damage while
dropping conflicting terrain voxels; an undermining edit must be refused.

Volume tests pin the grid-space metric on stacked surfaces — a sphere centred on a
bridge deck must reach the ground below it exactly when the level distance says so, and
`Path` rotation must produce congruent shapes in all six sextants.

Announcement tests prove the contract's honesty: a cast at undiggable material spends
mana and changes nothing, and the acknowledgment reports it unchanged rather than
silently succeeding.

Replay tests extend the funnel's existing determinism test to casts, including variable
mana and facing — the same sequence applied twice must land the same world.
