# Casting

How a spell becomes a change to the world: what makes a cast legal, what shape it
affects, who decides what happens to the material inside that shape, and how effects
that outlive their turn are expressed.

> **Status:** this is the normative contract for the 0.3 casting slice. Unit effects,
> Burn, Reveal, geometry, aiming, the command path, exact material occupancy,
> permanent stone evocation construction, the world-owned terrain resolver, radial
> effect-volume clipping, area Disable/Burn, elemental impact publication, paid
> monotonic correlation, the combat-authority hold, and deterministic
> unsupported-actor settlement are live.
> Enchantment-bound terrain, spell-created illumination, area Restore/Reveal, and
> area-lingering zones remain later work.

One-shot wards also remain deferred: the schema can decode
`ModifyIncomingDisables`, but no runtime state owns that effect yet.

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

The initial policy charges a cast that reaches the world: mana is spent, the action is
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
| **Conjuration** | `TerrainEdit::Set` — **built for permanent evocations** | Gameplay names the substance and the volume; the world validates placement |
| **Elemental damage** | `TerrainImpact { batch, volume, element, power }` — **world receiver and gameplay emitter live** | The world owns toughness, protection, accumulated damage, and destruction |

`batch` is the session-unique id the world echoes in its applied or rejected answer, so
gameplay can match the result to the cast that caused it. Both messages are specified
in full in [boundary.md](../planning/boundary.md) G and H.

**Destruction has a counterparty; creation does not.** When fire meets a voxel, that
voxel already has properties its author defined, so the response belongs to that
author. When a mage conjures, nothing is there to respond — the material *is* the
spell's identity. `Stone Shaper` summons stone because that is what the spell is, and
moving that choice into the world-side damage table would let the spell's identity
drift out of its own file.

So conjuration keeps naming substances in `spells.ron`, where
[`ContentIndex`](../../crates/hex_assets/src/content_index.rs) already validates the
cross-file reference (**built**), and elemental effects name only an element and a
power.

**`TerrainEdit::Clear` will not be used by spells.** Destruction flows through
`TerrainImpact` so the world owner arbitrates it; `Clear` remains for save restoration
and authored terrain, where the exact outcome is the point.

That is a change to shipped content, not just a rule: **"Stone Shaper" now carries
only `SetTerrain(substance: "stone")`**. Spell-authored `ClearTerrain` was removed;
destruction becomes an `Impact` when terrain magic lands. The lower-level
`TerrainEdit::Clear` remains available for save restoration and authored terrain.

Power is an explicit content field — `Impact(element, power: N)` — so designers tune
it directly rather than having it inferred from tier. Content admission rejects blank
or unknown element names and zero power before a spell is admitted. A conjuration may
name only a substance the world marks
`conjurable`; existence alone is insufficient. The content loader validates that
cross-domain reference before the spell becomes available, as specified by
[boundary.md](../planning/boundary.md) L. This prevents ordinary spell content
from creating protected bedrock or static hazards merely because those substances
exist.

The first damage model is intentionally literal. `power` subtracts that many hit
points, capped at the voxel's remaining health. Material maximum health comes from the
fixed `1/2/4/8` toughness scale, and `terrain_damage.ron` contains only a Boolean
element × material allow-list. A missing pair resists; there are no multipliers,
thresholds, healing, replacement materials, or elemental transformations. Gameplay
does not duplicate either file and cannot predict the outcome before the world answers.

The neutral elemental-grid content lists each of the 18 canonical elements against
each of the ten toughness-bearing substances: **180 unique allowed pairs**. That
broad table proves coherent admission without pretending to be final balance. Its
expansion is content migration, not a new terrain-damage mechanic and not completion
of the residual HEX-19 work. Water, lava, air, and bedrock have no toughness; authored
liquid topology and the other map-owned protections continue to resist.

The first consumer is Fireball with `Impact(element: "Fire", power: 2)`. Its
previous `Displace` is removed rather than advertising forced movement the runtime
does not implement. A Creator-authored full Fire ring may inscribe it through the
existing Creator → Sandbox route; packaged archetypes and scenario balance are
unchanged. PR #180 updates the thin Creator deployability consumer and the casting
preview's semantic clipped voxel set, but adds no UI model, widget, layout, or rendering
behavior.

### World answer and gameplay completion are live

The map now answers every processed batch exactly once with
`TerrainImpactResult::Applied` or `::Rejected`. An applied answer has one ordered
`TerrainVoxelOutcome` per announced voxel: `NoMaterial`, `Resisted`, `Damaged`, or
`Destroyed`, with material and valid nonzero health before/after as the disposition
allows. A rejected answer carries one explicit reason and no voxel payload; it changes
nothing. Invalid input and unavailable terrain therefore cannot strand a correctly
implemented pending cast.

The gameplay emitter and consumer keep the cast pending from its one payment and
emission until every area decision, matching terrain answer, actor settlement, and
authority adoption finishes. It preflights checked, session-local, monotonic batch ids
before payment, records one exact
`TerrainBatchId → TerrainImpact` obligation per authored Impact, and accepts valid
answers in any order. `Applied` and every structurally valid `Rejected` answer,
including `TerrainUnavailable`, retain payment and complete their batch. Unknown,
duplicate, reused, mismatched, or structurally inconsistent answers preserve typed
correlation evidence and freeze the transaction; there is no timeout or optimistic
release.

The implementation wires the configured `TerrainSystems` protocol as
`ApplyWorld → RefreshProjections → ReconcileActors → ConsumeOutcomes` before
perception and later combat. `RefreshProjections` republishes exact terrain occupancy
and reconciles ordinary movement. A separate outcome reader stages only whether a
structurally consistent first answer is `Applied`; it does not complete or correlate
the batch. This makes `ReconcileActors` run settlement only for material work the
world actually applied, so a valid `TerrainUnavailable` or other rejection cannot
fail against an irrelevant support projection. `ReconcileActors` processes unsupported
actors in stable `UnitId` order, reserves earlier destinations, validates the complete
future authority projection before any ECS write, then atomically cancels stale
route/Busy/transformation state, commits `StandsOn` and `Transform`, and adopts the
exact result into combat authority. Landing first chooses the highest legal unoccupied support
strictly below in the same column; otherwise it uses the lateral ordering pinned in
[boundary H](../planning/boundary.md#cross-owner-ordering-and-unsupported-actors).
Falling costs no health, movement, action, or turn. No legal landing freezes with a
typed diagnostic rather than leaving an actor in air or despawning it.

The authority hold is independent of the one public `PendingDecision`. It therefore
survives between defender answers and while terrain is unresolved, blocking ordinary
commands, turn advance, disengagement, and outcome settlement. `ConsumeOutcomes`
remains the only phase that validates/correlates completion and releases it exactly
once, only after all obligations and any required settlement/adoption have completed.
Pending state, queued work, batch allocation, and fatal evidence survive
pause and ordinary combat-mode exit, then reset on gameplay-screen teardown.

An answer is authoritative simulation truth, not permission to show hidden terrain.
Faction-facing animation and logs filter its entries through observation. The separate
`DamagedVoxels` projection permits a depth-tested health bar only for currently
Observed, exposed, visible partial-health surfaces; it does not turn hidden impact
results into knowledge.

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

The shipped `Earthen Wall` is an evocation and uses the exact two-voxel permanent
stone-construction adapter. Stone is the only currently conjurable substance and the
complete creation volume must be air. Content admission rejects terrain creation on
enchantments; enchantment-bound terrain still waits for provenance and removal.

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
   inherit high-ground-buys-range automatically** from the same rule engagement uses.
   Direct, authored-rise Arc, and None trajectory checks are built and wired; see
   *Obstruction*. Radial per-voxel clipping runs after the anchor remains legal.
4. **Unit interaction — exact commit snapshot.** The implementation snapshots every
   exact `StandsOn` occupant in the clipped volume when the cast commits, then resolves
   authored effects and stable `UnitId`s in that order without a faction filter. It
   delivers area `DisableHexes` and `Burn`, one public defender decision at a time.
   Area `RestoreHexes` and `Reveal` remain fail-closed because their hidden-information
   and choice policy is not settled.
5. **Announce** — a legal permanent construction volume emits exact
   `TerrainEdit::Set` messages (**built**). Paid `TerrainImpact` publication and the
   pending-answer transaction are built and wired.

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

This is live at every current entry point. The aiming preview and target cycle expose
only Observed anchors, AI legal-action enumeration receives only authorized anchors,
and the authoritative command applier checks observation again at application time.
The repeated check is intentional: a preview or AI request is not authority after
knowledge changes.

The rule is absolute, including for divination. That is the honest reading of the
design rather than an oversight: divination is the *lattice*-information channel, and
`Reveal` targets a unit you must already see in order to scry. A **spatial** divination
— "reveal what is over that ridge" — would need an exception, but no such spell is
designed, and Unknown positions are unpickable by design so there would be nothing to
name anyway. If one is ever specified, the exception is one condition in this ladder
and changes nothing about the boundary with the world owner.

### Occupancy

Tiles publish each run's inclusive top (`TilePos`) and bottom
(`RunBottom(Level)`), including every stacked run; `Headroom` remains a separate
saturated clearance fact. `hex_units::TerrainOccupancy` now compacts those exact
inclusive bounds, preserving real air gaps between stacked runs without reconstructing
occupancy from rendered spans or world units. Construction legality consumes that
projection, as do the live trajectory checks; cover and pathing remain downstream
consumers.

### Construction placement

The selected `TilePos` remains the currently Observed authorization and range anchor.
Permanent construction begins at `target.above()`, and the authored shape resolves
from there. `Single` therefore creates one voxel above the selected surface, while
`Column(height: 2)` creates two complete voxels above it. Selecting a lower surface
under a bridge or overhang never jumps to the column's highest run.

The authoritative applier checks the complete creation volume before emitting any
edit. Existing material, a unit-support surface, or a unit-body intersection suppresses
the whole edit batch. Hidden truth cannot become a refusal or payment oracle: the cast
is accepted and paid in exactly the same way as a clear placement, while authority
withholds the unsafe edits. `Headroom` is not used to infer air. The world still
validates each low-level edit against its private material and topology policy.

For the initial slice, this means exactly **stone into air**. `stone` is the only
current substance admitted by `conjurable`, every accepted voxel starts at stone's
full toughness, and neither a same-material placement nor another existing material is
used as a repair path.

Initial spell content carries exactly one construction effect, cannot mix construction
with non-construction effects, requires `Evocation`, and uses only fully vertical
`Single` or `Column` shapes. Positive-radius and authored spillover construction waits
for a faction-authorized empty-volume contract. This avoids silently applying two
different materials or exposing hidden blockers through placement legality.

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

The binding contract is that **one clipped volume affects every supported unit and
terrain voxel inside it**, including allies, enemies, and the caster. The live
implementation honors that contract for `DisableHexes`, `Burn`, and `Impact`: it
snapshots exact occupants at commit, processes effects in authored order and occupants
in stable `UnitId` order, and reaches each body at most once per effect. A selected
downed damage target retains the current pre-payment refusal; an incidental downed
spill target is skipped without becoming an information oracle.

Area `RestoreHexes` stays closed because choosing exact cells on a hidden target would
expose its lattice. Area `Reveal` stays closed because the live divination policy
requires an observed subject. The implementation does not infer either policy merely to
make the generic volume loop total.

Initial conjured walls are **2 voxels tall**. The canonical walker is 2 tall and climbs
1, so a 1-voxel wall is a step rather than a useful first implementation.

The surface-only renderer cannot draw free-standing air voxels without a world-owned
presentation height contract. The aiming panel reports the exact translated voxel
count and keeps the selected support cap visible; it does not infer `level_height` or
fabricate world-space occupancy to paint the air volume.

### Obstruction

`TargetingSpec::trajectory` is a closed vocabulary:

- `Direct` follows a straight centre-to-centre segment;
- `Arc { rise }` follows two segments through a deterministic apex exactly `rise`
  integer levels above the higher endpoint;
- `None` deliberately ignores material obstruction.

Direct and arc traversal uses one direction-symmetric integer 3D supercover in
`hex_units`. Every closed voxel prism the segment touches counts, including exact
face, edge, and corner grazes. The source and destination endpoints are excluded; all
other touched material blocks. The source is the caster's lowest body voxel
(`standing.above()`). An ordinary spell ends in the body/air voxel above the selected
surface; otherwise a level shot across flat ground would enter the floor before it
arrived. Terrain construction instead ends at the selected material surface because
that surface authorizes the separate placement volume above it. Occupancy comes only
from the exact `RunBottom` projection — never `HexSpan`, transforms, `level_height`,
or saturated `Headroom`. A blocked cast exposes only a generic refusal because the
obstruction itself may be hidden.

Authoritative casting checks the trajectory after observation and before payment.
Faction-facing preview anchors, target cycling, and AI legal-action enumeration use
the same geometry over a separate authorized projection containing only exact material
surface positions that are currently Observed. Remembered or Unknown material cannot
change those choices; authority may still refuse against full physical truth. The
RecordInput target cycle intentionally uses the last published faction knowledge after
a same-frame edit, then redraws after the next knowledge publication. Authored target
range and `Arc.rise` are both capped at 16 as a technical traversal guardrail.

The canonical effect volume clips after the cast reaches its selected anchor. `Direct`
and `Arc` both spread radially from that anchor to each candidate over the same direct
symmetric supercover; they do not introduce an arc-shaped radial algorithm. Both
radial endpoints are excluded, so the anchor and candidate material remain hittable
while intermediate material removes only voxels behind it.
`Trajectory::None` returns the raw canonical volume byte-for-byte. A noncanonical
volume is rejected rather than sorted, deduplicated, normalized, or repaired.

Authority clips against complete `TerrainOccupancy`; preview and AI clip against
`KnownTerrainOccupancy`. Hidden blockers therefore cannot change faction-facing
choices even though full physical truth may remove a candidate at application.
Obstruction-aware sight, when it lands, must reuse this supercover rather than grow an
independently rounded ray.

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
wire format is a future replay/save commitment. The live queue is consumed; no replay
log is persisted yet.

## Persistent effects

Damage over time, enchantment upkeep, and decaying divination are the same system wearing
three hats, so they are built once:

```
{ source, target, payload, start, end }
```

Vocabulary in `hex_core`, runtime in `hex_combat`, lattice payloads applied through
`hex_lattice`'s existing functions — the same split the command funnel uses.

- **End conditions in 0.3**: after N affected-unit turns, after N rounds, or bound
  to an enchantment (ending when it breaks). Area-lingering zones and dispel effects
  come later.
- **Tick point is per payload.** Some effects are personal and tick at the start of the
  affected unit's turn — which is exactly how the design words fire's damage over time —
  and some are global and tick at round boundaries.

`Burn` is a payload, not a special case. What burning *means* can be redefined freely
without touching the framework, which is the point of having one.

## Rulings worth writing down

- **There is no ally/enemy targeting filter, and there will not be one.** You may heal
  an enemy and immolate a friend. Area Disable/Burn honors this across caster, allies,
  and enemies; unsupported area Restore/Reveal remain fail-closed for their information
  policy, not because of faction.
- **Combat-only casting is provisional in 0.3.** Shaping terrain out of combat is
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
- **Downed-first death is provisional.** Version 0.3 removes a fully disabled
  unit from the turn order and leaves it revivable by a restoring spell. Functional
  death and permadeath remain separate design decisions. Further damaging casts refuse
  a downed target before payment, while Reveal may still inspect its retained lattice.
- **Renewal is an exact caster choice.** `RestoreHexes` parks
  `PendingDecision::ChooseRestores`; a player caster selects disabled cells on the
  target lattice, while a non-player caster uses its registered deterministic
  algorithm. The answer remains a replayable `ChooseRestores` command rather than an
  internal healing policy. Shipped Renewal carries only `RestoreHexes(count: 2)`;
  its former `ModifyIncomingDisables` entry was removed because one-shot wards have
  no delivered runtime lifecycle.
- **Only one exact-cell choice is public at a time.** Content validation still prevents
  incompatible choice-producing effects, while the implementation may queue several
  area Disable recipients behind the existing `PendingDecision`. It publishes the next
  stable-`UnitId` decision only after the previous answer is adopted, and the separate
  authority hold prevents the cleared public slot from advancing the turn early.
- **Single-target `Reveal` is live; `Illuminate` still rejects with a reason.** The
  E0 content assigns Scrying Eye to Divination while retaining the current
  observed-subject, complete tier-bounded view through the knowledge seam. A
  continuous off-sight live feed is separate later Divination work. Spell-created
  illumination belongs to Illusion, still waits on the perception lane, and must not
  silently do nothing; the former Daylight spell is not part of the canonical
  migrated content.
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

Terrain-durability content and map tests pin the fixed toughness scale, Boolean matrix
validation, coherent reloads, canonical-volume rejection, one answer per batch, exact
ordered outcomes, direct power subtraction, protected topology, sparse exact-voxel
health, and map rebuild lifecycle. Presentation tests pin observation, darkness,
burial, composed visibility, grid replacement, and cleanup. Pure contract tests
exhaustively reject mismatched batches/positions, schema-invalid material/health
transitions, and incompatible applied/rejected answers. Substance-catalog
correspondence remains map/content-owned. The gameplay consumer wedge covers
`ApplyWorld → RefreshProjections → ReconcileActors → ConsumeOutcomes → perception
→ later combat`. Its settlement fixtures include stacked supports, simultaneous falls,
occupied candidates, lateral higher-ground fallback, insertion-order independence,
and typed no-landing freeze.

The lasting automated evidence for the delivered wave uses the ordinary fail-closed
concern graph: trajectory and volume rules run in `trajectory_contracts`; rules, ECS,
map seams, and application consumers run in their owning concerns; and the dedicated
renderer-free `hex_game/tests/spell_resolution.rs` composition target runs as the
`contracts` postflight. That target installs no renderer or UI and proves only the real
map/units/perception/combat protocol. The temporary delivery-only routing used while
PR #180 was in review is retired; see [gameplay
testing](../development/gameplay-testing.md#spell-resolution-evidence-after-pr-180).

Gameplay hooks, typed events, canonical snapshots, and renderer-free contracts are the
authoritative evidence for casting logic. Screenshots may judge static camera/UI/map
presentation of a hook-established casting state, and video/human checks may judge
motion and control feel. None may be collected, requested, or cited as evidence of
casting legality, payment, settlement, authority release, or turn resumption when
hooks can prove the claim; they neither satisfy nor supplement a gameplay-logic gate.

Replay tests extend the funnel's existing determinism test to casts, including variable
mana and facing — the same sequence applied twice must land the same world.
