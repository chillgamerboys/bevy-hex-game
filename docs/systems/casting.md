# Casting

How a spell becomes a change to the world: what makes a cast legal, what shape it
affects, who decides what happens to the material inside that shape, and how effects
that outlive their turn are expressed.

> **Status:** this is the normative contract for wave 3's casting work. Unit effects,
> Burn, Reveal, geometry, aiming, and the command path are built. Terrain announcements,
> obstruction, and spell-created illumination remain contracts.

Read [the design](../design/game.md) for the magic system this serves, and
[combat.md](combat.md) for the turn loop a cast happens inside.

## The one-sentence version

**Gameplay owns geometry; the world owns materiality.** A cast announces an exact set
of voxels, and what the material inside them does about it is the world owner's
decision.

That single line settles most of the questions below, and it is the reason casting can
be validated before it is announced: we always know *which* voxels, even when we do not
know what will become of them.

## Casting is committing — provisionally

The initial wave charges a cast that reaches the world: mana is spent, the action is
taken, and the presentation plays even when no material changes. **That payment policy
is provisional.** The binding boundary is that gameplay announces the effect and the
world decides the material response; playtesting may change when or how a fully
resisted cast is charged without moving that responsibility.

A fireball thrown at bedrock is a legal cast that scorches nothing. It is not a
map error, and gameplay still does not predict which materials yield. In the first
implementation it also costs the full cast. That last rule is a starting point to
test, not permanent game design.

Partial material application remains normal regardless of payment policy. A blast
across a hillside may clear the dirt and leave the granite ridge running through it
untouched.

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
inferred from tier. A conjuration may name only a substance the world marks
`conjurable`; existence alone is insufficient. The content loader validates that
cross-domain reference before the spell becomes available, as specified by
[boundary.md](../planning/boundary.md) ask L. This prevents ordinary spell content
from creating protected bedrock or static hazards merely because those substances
exist.

## Shaped terrain persists

**An evocation's terrain change is persistent.** It lasts for at least multiple turns
rather than disappearing with its casting animation. The initial implementation makes
every accepted voxel edit permanent: there is no conjuration ledger, provenance,
expiry, or automatic un-build, so conjured stone is simply stone until something
changes it again. Whole-cast batch atomicity is not implied; it needs an explicit
terrain-edit batch contract if the first multi-voxel implementation requires it.

The first implementation keeps enchantment manifestations as entities bound to their
[`EnchantId`](../../crates/hex_core/src/lattice_ids.rs) (**built**) rather than terrain,
and despawns them when the enchantment breaks. That is a provisional implementation
split, not a ruling that every future enchantment must avoid terrain.

Permanent initial edits avoid an immediate class of provenance problems. Expiring
terrain would need a ledger and an answer for what happens when another effect reshapes
the same voxels before expiry; that design may be added later without weakening the
multi-turn persistence rule.

Honest caveat: an entity-shaped barrier does not block movement, because units do not
obstruct each other yet ([status.md](../planning/status.md)). Terrain walls are the
blocking tool until occupancy exists.

## The legality ladder

Five rungs, checked in order. The emitter pre-filters what it can for click feel; the
applier in `hex_combat` is authoritative.

1. **Actor** — whose turn it is, seat ownership, not `Busy`, action available.
   **Built** — this is the command funnel's existing validation.
2. **Lattice** — [`castable()`](../../crates/hex_lattice/src/cast.rs) returns a
   `CastPlan` or a blocked reason the UI shows. **Built and wired**.
3. **Targeting** — the anchor is in range, the shape resolves to a voxel set, and the
   trajectory is clear. Range uses
   [`in_reach`](../../crates/hex_units/src/targeting.rs) (**built**), so **spells
   inherit high-ground-buys-range automatically** — the rule was written for this and
   has had exactly one consumer until now, engagement. Trajectory is deferred; see
   *Obstruction*.
4. **Unit interaction — provisional first-wave safety policy.** The current unit-effect
   applier reaches the unit on the anchor. Content therefore refuses a unit-affecting
   spell only when its resolved shape can contain more than one distinct voxel. Boundary
   shapes resolving to zero or one distinct voxel remain legal; genuinely area-shaped
   unit effects wait until resolution iterates every occupied voxel.
5. **Announce** — the surviving terrain volume goes to the world, which arbitrates.
   Terrain effects still fail closed as undeliverable rather than charging for no result.

Rungs 1–2 are gameplay's own state. Rung 4 is gameplay's knowledge too: **where
characters stand is ours**, so a cast interacts with units through legality, exactly as
movement does — a character cannot walk through a wall, and a wall cannot be conjured
through a character. Only rung 5 defers, because only material response is the world's.

### Observation

Every cast requires its exact positional anchor to be currently Observed by the acting
faction, whether that anchor identifies terrain or an observed unit
([perception.md](perception.md)). Remembered terrain is not sufficient, and Unknown
terrain exposes no targetable `TilePos`.

Once the Observed anchor resolves, an area shape may extend into Remembered or Unknown
positions. The effect still applies authoritatively to hidden terrain and units, but
its acknowledgments, animations, and combat-log entries are filtered through faction
knowledge and do not reveal hidden outcomes.

The rule is absolute, including for divination. That is the honest reading of the
design rather than an oversight: divination is the *lattice*-information channel, and
`Reveal` targets a unit you must already see in order to scry. A **spatial** divination
— "reveal what is over that ridge" — would need an exception, but no such spell is
designed, and Unknown positions are unpickable by design so there would be nothing to
name anyway. If one is ever specified, the exception is one condition in this ladder
and changes nothing about the boundary with the world owner.

The observation rung returns `true` until fog exists because every current target
genuinely is Observed. It becomes a real query when `hex_perception` lands, and that is
a one-function change inside `hex_combat`.

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

The shape vocabulary resolves to exact voxel sets in
[`hex_units::volumes`](../../crates/hex_units/src/volumes.rs) (**built**), and
`TargetShape` in `spells.ron` names the same seven (**built**). Preview and unit-effect
cardinality validation consume the result today; terrain announcement does not.

| Shape | Volume |
|---|---|
| `SelfCast` | the caster's own voxel |
| `Single` | the anchor voxel |
| `Sphere(radius)` | grid-space ball around the anchor: `radius` hexes out **and** `radius` levels either way |
| `Column(height)` | the anchor and the voxels above it, `height` counting the anchor |
| `Line(length, width)` | from the caster toward the facing; `width` is a half-thickness in hexes, `0` being a single file |
| `Cone(length, spread)` | widening from the caster toward the facing; `spread` is 60° sectors each side, `1` being the familiar cone and `3` a full disc |
| `Path(offsets)` | an authored offset list, rotated to the facing |

Three rulings the table cannot carry:

- **`Line` and `Cone` start one hex out.** The caster's own voxel is never in its own
  line or cone, including when a thickened line's near end would otherwise round back
  over it. `SelfCast` is how a spell reaches the caster.
- **`Path` hangs on the anchor, not the caster.** A wall is authored where it is
  built, and the anchor is the one thing every cast names.
- **`Column` and `Path` are the only shapes with authored vertical extent.** `Line`
  and `Cone` are planar at the caster's level; a spell wanting a wall of flame needs a
  `Path`.

`Path` rotates in sextants — 60° steps are exact on cube coordinates, so an authored
pattern keeps its shape in all six directions. The rotation is about the vertical
axis, so an offset's `level` survives it untouched and a staircase rotates into a
staircase.

Resolvers hand back volumes already in `TerrainImpact`'s canonical sorted,
deduplicated form, so a `Sphere` and a `Column` that overlap name each shared voxel
once. Degenerate extents are total rather than special-cased: radius 0 is the anchor
alone, and a zero-height column, a zero-length line or cone, and an empty path are all
the empty volume. Content validation refuses to author most of those, but the geometry
does not depend on it having done so.

The binding contract is that **one volume eventually affects every unit and terrain
voxel inside it**, including allies, enemies, and the caster. The current unit-effect
implementation intentionally accepts only resolved cardinality zero or one, because it
still applies to the anchor's occupant. That fail-closed content guard prevents a
multi-voxel preview from promising area damage the applier cannot yet deliver.

Initial conjured walls are **2 voxels tall**. The canonical walker is 2 tall and climbs
1, so a 1-voxel wall is a step rather than a useful first implementation.

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

`Sextant` is the built six-value direction type in `hex_core`. It names the fixed
directions returned by `HexCoord::neighbors()`, so a `Path` rotation is expressed in the
domain rather than as a bare index.

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

- **End conditions in wave 3**: after N affected-unit turns, after N rounds, or bound
  to an enchantment (ending when it breaks). Area-lingering zones and dispel effects
  come later.
- **Tick point is per payload.** Some effects are personal and tick at the start of the
  affected unit's turn — which is exactly how the design words fire's damage over time —
  and some are global and tick at round boundaries.

`Burn` is a payload, not a special case. What burning *means* can be redefined freely
without touching the framework, which is the point of having one.

## Rulings worth writing down

- **There is no ally/enemy targeting filter, and there will not be one.** You may heal
  an enemy and immolate a friend. Multi-voxel unit effects are fail-closed until the
  applier can honor the eventual every-occupant contract.
- **Combat-only casting is provisional in wave 3.** Shaping terrain out of combat is
  attractive, and the mana half of that question now has an answer: recovery between
  fights is an explicit **rest action** (ruled 2026-07-27 — see
  [design/game.md](../design/game.md#recovery-and-death)), because channelling's
  per-turn model has nothing to say about the time between fights. What has no answer is
  the *input* half: casting in real time wants a different interaction model than a turn
  does. That is what magic outside combat still waits on, and it is a separate epic.
- **Channelling and rituals are deferred.** `co_castable` parses and feeds
  `Spell::is_ritual()` (**built**, and read today only by the lattice demo and the dev
  content dump); it has no mechanical effect. Co-casting is entangled with the
  unresolved initiative question.
- **Downed-first death is provisional.** Wave 3 initially removes a fully disabled
  unit from the turn order and leaves it revivable by a restoring spell. Functional
  death and permadeath remain separate design decisions.
- **`Reveal` is live; `Illuminate` still rejects with a reason.** Reveal writes a
  complete tier-bounded view through the knowledge seam. Spell-created lights still
  wait on the perception lane and must not silently do nothing.
- **Generated features are unaffected initially.** Destructible trees, tall grass,
  and other feature effects wait on an explicit world response and outcome contract.

## Verification gate

Legality tests cover each live rung independently and in combination: a cast refused
for lack of mana must not consume the action; every target anchor must be Observed; a
shape may spill into hidden positions without exposing them; and every zero/one-versus-
many resolved-cardinality boundary is pinned.

Volume tests pin the grid-space metric on stacked surfaces — a sphere centred on a
bridge deck must reach the ground below it exactly when the level distance says so, and
`Path` rotation must produce congruent shapes in all six sextants.

Replay tests extend the funnel's existing determinism test to casts, including variable
mana and facing — the same sequence applied twice must land the same world.
