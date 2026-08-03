# Spell resolution gameplay wave

Status: **approved; implementation in progress**. The draft PR remains the live wave
tracker, and each reviewed semantic checkpoint is pushed to it.

Base: `origin/dev @ 6cb749adc5168e4480d1f4efedba8097f49bf64d`

Integration branch: `wave/spell-resolution`

Integration owner: Shravan / gameplay

This plan applies the repository's
[parallel-development rules](../development/parallel-development.md) to the remaining
gameplay half of terrain magic. It is a single wave because clipping, area resolution,
terrain acknowledgment, settlement, and command release share one runtime transaction;
reviewing any leaf as a completed feature would be misleading.

## Landed foundation

- PR #162 (merge `84cbd409`) delivered exact caster-to-anchor `Direct`,
  authored-rise `Arc`, and `None` trajectories over one symmetric integer
  supercover.
- PR #174 (merge `2f48cdcb`) superseded the stale PR #173 camera prerequisite.
- PR #175 (merge `70a212bb`) delivered the world-owned terrain durability resolver:
  fixed toughness, protected ordered impact resolution, applied/rejected outcomes,
  sparse health, and terrain rebuild consequences.
- PR #176 landed independently and no longer overlaps this work.
- PR #178 (merge `6cb749ad`) delivered exhaustive `TerrainImpactOutcome` validation, configured
  `TerrainSystems::{ApplyWorld, RefreshProjections, ReconcileActors, ConsumeOutcomes}`
  ordering vocabulary, and the 61-test non-UI trajectory/volume concern.

The live [G/H contracts](../contracts.md) are sufficient. This wave requires no new
`hex_core` schema, no `hex_map` implementation change, and no Alberto/world
implementation lane. The accepted settlement policy replaces the earlier support
reservation proposal. A breached cave retains authored Interior membership and gains
no dynamic daylight in this slice.

## Draft candidate ledger

The following reviewed foundations are on the local wave candidate. They remain draft
evidence rather than delivered `dev` behavior:

- `ae17f54` adds the pure combat-authority hold that prevents an intermediate area
  answer from advancing the turn or settling an outcome;
- `9dc9555` moves exact occupancy and route reconciliation into
  `TerrainSystems::RefreshProjections` and adds the deterministic unsupported-actor
  landing planner;
- `ecc58fc` adds validated `Impact { element, power }` content, makes Fireball the
  first Fire/power-2 consumer, and removes its deferred `Displace`; and
- `6286ca8` adds canonical radial effect-volume clipping over the existing symmetric
  supercover, with separate full-truth and faction-known projections.

At the `6286ca8` helper checkpoint, the paid runtime transaction, area queue, terrain
outcome consumer, ECS settlement commit, narrow scope wiring, and composition target
remained integration-owner work on the same draft PR. Later #180 checkpoints compose
those pieces under the contract below; the PR's exact head is the implementation-state
authority. No checkpoint in this ledger is a final green gate or manual runtime
sign-off.

## Combined outcome

A legal area elemental cast will:

1. resolve one canonical effect volume and obstruction-clip it without changing the
   already-live caster-to-anchor trajectory;
2. snapshot exact occupants and apply supported unit effects in authored-effect then
   stable-`UnitId` order, including caster, allies, and enemies;
3. pay once and publish exact `TerrainImpact` batches;
4. keep command and turn authority pending while the world applies or rejects them;
5. refresh occupancy and movement, settle unsupported actors deterministically, and
   adopt the settled positions into combat authority;
6. validate every matching answer and resume only after terrain, unit decisions,
   settlement, and authority adoption are all complete.

The first shipped content consumer is Fireball with
`Impact(element: "Fire", power: 2)`. Its still-deferred `Displace` effect is removed
from the shipped spell instead of advertising a partially delivered effect. A
Creator-authored full Fire ring can inscribe that Fireball and launch it through the
existing Creator -> Sandbox route; packaged archetypes and scenarios remain unchanged.

## Pinned gameplay policy

### Effect-volume clipping

- Reuse the exact direct supercover; do not add another ray algorithm.
- `Direct` and `Arc` keep their existing caster-to-anchor legality, then clip each
  candidate along the selected-anchor-to-candidate supercover.
- Exclude both radial endpoints. The anchor and the wall/candidate voxel remain
  hittable; only intermediate material clips voxels behind it.
- `Trajectory::None` preserves the raw canonical volume byte-for-byte.
- Reject noncanonical input. Never sort, deduplicate, normalize, repair, or add voxels.
- Authority uses complete `TerrainOccupancy`. Preview and AI use
  `KnownTerrainOccupancy`, so hidden blockers never change faction-facing choices.

### Area occupants and hidden information

- Snapshot exact `StandsOn` occupants when the cast commits, before terrain settlement.
- Process effects in authored order and occupants in stable `UnitId` order. Apply each
  effect to each body at most once; never filter by faction.
- This wave delivers area `DisableHexes` and `Burn`. It queues one defender choice at a
  time behind the existing public `PendingDecision`.
- A selected downed damage target retains the current pre-payment refusal. Incidental
  downed spill targets are skipped without revealing their presence.
- Area `RestoreHexes` and `Reveal` remain fail-closed. A hidden Restore would expose a
  target lattice through the caster's exact-cell choice, while hidden Reveal conflicts
  with the current observed-subject rule. Neither policy should be invented inside the
  implementation.
- The existing gameplay deployability gate may expose shipped Fireball to a
  Creator-authored full Fire ring after area damage lands. Packaged archetypes and
  scenario balance stay unchanged. This changes the thin Creator eligibility consumer,
  not its UI model, widgets, layout, or rendering.

### Terrain transaction and failure

- Add gameplay-owned `Effect::Impact { element: String, power: u8 }`. Reject blank or
  unknown element names and zero power before gameplay; never infer an impact element
  from gem requirements.
- Permit repeated Impact effects. Preflight enough checked, monotonic, session-local
  batch IDs before payment and correlate every exact `TerrainBatchId -> TerrainImpact`.
  Each batch resolves independently, and all must finish before cast release.
- A valid `Applied` or `Rejected` answer—including `TerrainUnavailable`—retains payment
  and completes that batch.
- Unknown, duplicate, mismatched, or structurally inconsistent answers retain typed
  correlation evidence and freeze resolution. There is no timeout or optimistic
  release.
- Pending state, queued decisions, the batch allocator, and fatal evidence reset on
  gameplay-screen teardown, not ordinary combat exit. Pause retains them.

### Unsupported actors

After refreshed terrain publication, settle unsupported units in stable `UnitId`
order:

1. highest legal, unoccupied support strictly below in the same column;
2. otherwise lateral legal surfaces ordered by
   `(hex_distance, abs_level_difference, is_higher, TilePos)`;
3. body/headroom/traversal rules, exact blockers, current occupancy, and earlier
   reservations all apply;
4. cancel stale route, `Busy`, and transformation state and update `StandsOn`,
   `Transform`, occupancy, and combat authority together;
5. falling costs no health, movement, action, or turn ownership;
6. no landing yields a typed fatal/frozen diagnostic—never air standing or despawn.

## Integration topology

Only the integration owner edits `crates/hex_combat/src/commands/cast.rs`,
`crates/hex_combat/src/commands/mod.rs`, the combat-authority hold seam,
`.config/test-scopes.json`, CI topology, and shared delivery docs.

| Lane | Branch | Base/dependency | Owned result |
|---|---|---|---|
| Transaction foundation | `wave/spell-resolution` | exact wave base | one private pending transaction, queue, modal gate, explicit combat-authority hold, and turn/exit gate |
| Effect-volume clipping | `feat/spell-volume-clipping` | exact wave base; parallel | pure clipping helpers and contracts; no cast hot-file edits |
| Terrain reconciliation | `feat/terrain-reconciliation` | exact wave base; parallel | occupancy/movement phase placement and pure deterministic landing planner |
| Impact content | `feat/spell-impact-content` | exact wave base; integrated as `ecc58fc` | Impact schema/validation/fingerprint, Fireball content, and Creator deployability policy; no runtime hot-file edits |
| Area + terrain runtime | `wave/spell-resolution` | combined checkpoint | area queue, paid batch ledger, outcome consumer, settlement and authority adoption |
| Composition | `wave/spell-resolution` | all lanes | dedicated headless composition target, exact scope routing, docs, and delivery reconciliation |

Source branches target the wave or remain source-only. One final wave PR targets
`dev`; there is no leaf PR per helper by default.

## Exact runtime order

```text
frame N Combat Apply: commit/pay once, queue unit work, emit TerrainImpact

frame N+1 TerrainSystems::ApplyWorld
  -> TerrainSystems::RefreshProjections
       -> TerrainOccupancySystems::Publish
       -> MovementSystems::Reconcile
       -> stage only whether a consistent first answer is Applied (not completion)
  -> TerrainSystems::ReconcileActors
       -> for Applied work, plan every landing and validate future authority first
       -> atomically cancel invalid routes, settle actors, and adopt exact positions
  -> TerrainSystems::ConsumeOutcomes
       -> validate and correlate answers
       -> release only when every obligation is complete
  -> perception through PublishKnowledge
  -> Combat Act -> Apply -> Resolve -> Advance
```

Replace the current occupancy-after-`PublishKnowledge` edge; do not supplement it and
create a cycle. Do not add `Combat Apply -> ApplyWorld` or
`ConsumeOutcomes -> Act`. Normal actions, turn advance, disengagement, and combat exit
all remain gated while a transaction or fatal resolution state is active.
Valid rejected answers never invoke settlement; the early Applied staging reader owns
no completion authority and exists only to keep rejection and settlement failure paths
disjoint.

## Narrow non-UI verification

The automated test evidence is limited to authorities that can exercise these changes.
It does **not** run `hex_ui`, `hex_game/tests/gameplay_app.rs`, UI snapshots, visual
walks, deterministic combat simulation, procedural-generation corpora, or the residual
workspace corpus.

- Clipping: extend and run `python3 tools/test_scope.py run trajectory_contracts`.
  Cover malformed input, `None`, endpoints, wall shadows, conservative grazes,
  vertical/stacked cases, canonical preservation, and knowledge-vs-authority privacy.
- Resolution: add `python3 tools/test_scope.py run spell_resolution_contracts` with an
  exact nextest filter and JUnit. It includes the new impact/content, area queue,
  settlement, pending/outcome, authority-hold, phase-order, and composition coverage;
  existing non-UI spell validation/fingerprint, cast/payment/refusal/construction,
  command/authority/turn, and occupancy/movement regression closures; the ten
  `hex_core::terrain_impact` tests; and these two real-map producer seams:
  `terrain_protocol_orders_reserved_phases_before_perception` and
  `overkill_is_capped_and_empty_voxels_report_no_material`.
  Before execution, an exact nextest-list contract pins 56 domain identities, those
  two map identities, and seven renderer-free game-consumer/composition identities so
  filter drift cannot reduce evidence silently.
- Composition: add explicit `hex_game/tests/spell_resolution.rs` using minimal state
  and the real map/units/perception/combat plugins. It installs no `AppPlugin`,
  renderer, viewport, `hex_ui::UiPlugin`, or test-support UI and uses a tiny authored
  fixture rather than a V3 seed corpus. Because `hex_game` has `autotests = false`,
  register the target explicitly in its manifest and route both files to this concern.
- Authority: focused renderer-free reducer/host tests prove a held area transaction
  cannot advance the turn or settle the encounter between defender answers, and that
  release resumes exactly once. This is intentionally included even though the broad
  deterministic-simulation partition remains omitted.
- Non-test checks remain format, dependency policy, strict workspace Clippy,
  warnings-denied docs, and the default-feature shipping release build.
- Final runtime evidence is one named human Creator -> Sandbox Fireball pass on the
  exact wave head. It is not a UI/visual test; no screenshot review is required.

The scope/profile/CI bootstrap itself ordinarily triggers the repository's fail-closed
full gate. The user's explicit instruction and implementation approval are the durable
one-wave maintainer waiver for this gameplay-only candidate. Its only gameplay test
concerns are `trajectory_contracts` and `spell_resolution_contracts`; a focused
selector regression may additionally prove the waiver manifest itself. The following
omissions must appear as **WAIVED**, never passed or green: `hex_ui`,
`hex_game/tests/gameplay_app.rs`, UI snapshots, the automated visual walk,
deterministic combat simulation, V3/procedural-generation corpora, and the residual
workspace corpus. Format, dependency policy, strict Clippy, warnings-denied docs, and
the shipping build remain non-test gates.

The exception is valid only while every changed behavior is exercised by those two
non-UI producer/consumer closures and every changed path is in the wave's exact-file
allow-list. This wave does change two thin gameplay consumers: Creator deployability
and the casting preview's semantic clipped voxel set. Their policy is covered by the
content/trajectory contracts, renderer-free composition target, and named manual
route; all automated UI/app tests remain waived. A UI model, widget, layout or
rendering change, a `hex_map` implementation or G/H contract change, an unclassified
path, or behavior those closures cannot exercise invalidates the waiver and restores
the ordinary fail-closed gate. It applies only to PR #180 from
`wave/spell-resolution` to `dev` and its exact reviewed merge diff pushed to `dev`,
never a `main`-target PR, push to `main`, or later unrelated push. The PR and canonical
testing policy publish the exact narrow JUnit replacements and the waiver reason.

GitHub still enters required job shells whose expensive steps are conditionally
omitted. A green shell reports successful routing and required non-waived checks, not
an omitted partition passing: the scope artifact continues to label those partitions
**WAIVED**, and the named exact-head runtime route remains **PENDING** until recorded.

## Combined acceptance

- Frame N pays/emits exactly once and prevents later command or turn interleaving.
- Terrain answers and at least three queued defender choices may finish in either order
  and converge to the same final state.
- Friendly fire reaches caster, allies, and enemies exactly once in stable order.
- A mixed material/air impact uses the real #175 resolver, refreshes occupancy, settles
  simultaneous unsupported actors around occupied candidates, and adopts exact
  positions before the next action.
- Applied, all valid rejections, out-of-order valid multi-batch answers, malformed or
  foreign answers, pause/resume, forced mode exit, regeneration, and gameplay
  teardown/re-entry are bounded and deterministic.
- No-landing and invalid-correlation paths remain visibly fatal/frozen; no stale batch,
  duplicate payment, silent despawn, air standing, or timeout release is accepted.

## Stop conditions

- Any required change to G/H fields, meanings, rejection precedence, toughness,
  protections, world content, or `hex_map` implementation.
- Any new gameplay-to-world fact, support reservations, dynamic cave daylight/interior
  reclassification, liquid mutation, or feature destruction.
- A schedule cycle, stale occupancy/position reaching a later command, or inability to
  adopt settlement into combat authority before answer release.
- A second lane implements its own clipping, pending queue, settlement, or completion
  authority, or edits an integration-owned hot file.
- Area Restore/Reveal would be admitted without a separately accepted hidden
  information/choice policy.
- UI model, widget, layout, rendering, or presentation behavior beyond the two thin
  Creator/preview gameplay consumers becomes necessary. That invalidates the non-UI
  waiver and requires a new scope decision.

## Delivery-state reconciliation

HEX-19 and HEX-24 remain partial / In Progress while PR #180 is a draft. Linear is a
soft coordination signal and is currently unavailable; the exact recommended update
is to record #162, #174 (superseding #173), #175, and #178 as delivered foundations,
link draft #180 as the active implementation wave, and retain the residuals listed
here. Final tracked [casting](../systems/casting.md), status, contracts, roadmap, and
[gameplay-testing](../development/gameplay-testing.md) corrections land on the wave
candidate before it is called complete. After merge, reconciliation must verify the
exact `dev` merge SHA before either ticket is reclassified.
