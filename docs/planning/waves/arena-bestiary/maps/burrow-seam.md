# Worm terrain conversion seam

Status: **World conversion implemented at `aaecaf3`; Worm gameplay and application admission are enabled with focused composition checks passing.**

Original-group, Golem and Ember Wisp calibration checkpoints precede this phase.
Shared vocabulary landed at `59eef79`; the guarded body foundation at `dccfddb`
passes 236 arena tests. World conversion passes 24 arena map tests and strict
map/assets lint. Root owns the shared foundation and commits; the
world lane owns mutation and publication, and gameplay owns the body, movement,
observation, and attack state. See [approved scope](../plan.md) and
[world order](../orders/L1-world.md).

## Existing authority and the damage hazard

`hex_map::arena` applies direct edits and terrain impacts before publishing the
authoritative `ArenaTerrainView`. Its private `TerrainDamageState` stores sparse
remaining HP; a missing entry means full HP for the voxel's current substance.
`TerrainEdit::Set` changes material and calls `forget_voxel`, so it cannot implement
burrowing without repairing some damaged cells. `TerrainImpact` removes HP or
destroys material and likewise does not express this conversion.

Current authored maximum HP is grass/snow 1, dirt/gravel/sand/ice 2,
stone/basalt 4, and worked stone/metal 8. Bedrock and liquids have no damage HP.
Conversion must use actual current ledger HP, including damage resolved earlier in
the same tick, rather than a gameplay estimate or the requested snapshot's HP.

## Narrow shared request and outcome

The shared foundation publishes this arena-specific vocabulary:

```rust
ArenaBurrowRequest {
    generation: u64,
    actor: u8,
    sequence: u64,
    volume: Vec<TilePos>,
}

ArenaBurrowOutcome {
    generation: u64,
    actor: u8,
    sequence: u64,
    result: ArenaBurrowResult,
}

ArenaBurrowResult::Accepted { changed: Vec<ArenaBurrowChange> }
ArenaBurrowResult::Rejected { position: Option<TilePos>, reason }

ArenaBurrowChange {
    position: TilePos,
    before: SubstanceId,
    health_before: TerrainVoxelHealth,
    health_after: TerrainVoxelHealth,
}
```

Gameplay supplies the exact swept body/contact cells, including air, strictly
sorted and deduplicated. The initial per-request cap is 64 cells. The replacement
is always the world's published dirt; callers cannot supply material or HP.
`Accepted` acknowledges the entire announced volume, with `changed` containing
only material conversions in canonical order. Air and existing dirt need no
change entry. Outcomes preserve the request identity so gameplay can discard an
obsolete movement proposal without interpreting another Worm's acknowledgement.

Admission is atomic. An empty, noncanonical, oversized, stale-generation, or reused
request is rejected. If any announced cell is outside world bounds, protected,
liquid, static occupancy, or an ineligible solid, reject the complete volume with
the first canonical offending position. No partial conversion becomes permission
to move through a rejected part of the body. Use a highest-consumed sequence per
actor and generation, including rejected attempts, instead of an unbounded ledger
of every movement step. Gameplay emits at most one pending request per living
Worm; processing remains bounded by the accepted roster and per-request cap.

## Published world facts

Publish `ArenaBurrowMaterials` containing the material IDs admitted by the world
from `solid && diggable && toughness.is_some()`. With the current catalog this
admits the ten HP-bearing materials above, including worked stone and metal. This
policy is independent of elemental and physical damage admission. The destination
is the existing `ArenaMaterials.dirt`; the producer validates that it is an
admitted solid with valid toughness.

Reuse `ArenaVoxelGeometry` bounds and offset, `ArenaTerrainView.edit_protected`,
`liquids`, and `static_spans`. Gameplay must consume these facts rather than
reconstruct authored-object or liquid rules. Test interval overlap at the exact
voxel levels, including protected cells that currently contain air.

The current static publisher omits object cells that are neither opaque nor
movement blocking. For the Worm's prohibition on crossing authored tree/crystal
objects, publish those remaining cells too, retaining their ordinary query masks
as false. Burrowing treats every published static span as blocked regardless of
movement, attack, or sight flags. This adds complete object occupancy facts without
changing the existing masks for ordinary actors, projectiles, or cameras. Existing
decorative grass/moss exclusions remain outside this authored static-object rule.

## Exact material and HP transition

Keep a narrowly named private `convert_to_dirt` operation beside the world ledger.
For each admitted non-dirt solid, obtain current remaining HP before changing its
material, then calculate:

```text
new_remaining = min(old_remaining, dirt_maximum)
```

Write dirt and update the private ledger and `DamagedVoxels` together. Publish a
partial-health entry when `new_remaining < dirt_maximum`; otherwise remove its
sparse entry. Do not erase health before reading it. Existing dirt is a true
no-op, preserving both material and HP. Air stays air and is never filled.

| Before | After | Required consequence |
| --- | --- | --- |
| Grass 1/1 | Dirt 1/2 | Publish partial dirt HP even though grass was untouched. |
| Stone 1/4 | Dirt 1/2 | Preserve its one remaining HP. |
| Stone 3/4 | Dirt 2/2 | Cap at dirt's maximum; never increase remaining HP. |
| Dirt 1/2 | Dirt 1/2 | Do not reset or republish health as full. |

This operation neither creates occupied space nor excavates a tunnel. Dirt stays
solid and opaque for other actors and all ordinary world queries, and conversion
cannot refill a hole or trap another actor by adding terrain.

## Tick ordering, publication, and lifecycle

Resolve conversions after existing direct edits and impacts in `ApplyTerrain`,
before `PublishTerrain` and `Simulate`. Validate the live map and current ledger at
resolution; an old preview is not admission. Mark only columns containing actual
material changes and reuse incremental material/collision/render publication.
An all-air or existing-dirt acceptance needs no terrain revision. Multiple changes
in one tick retain the existing single combined publication behavior.

Deliver the matching outcome with that publication before gameplay advances the
pending step. A successful request does not authorize later movement through a
different material: the actual swept body must be rechecked against the latest
published dirt, bounds, protection, static occupancy, liquids, and live bodies.
Movement through newly contacted non-dirt solids waits for acknowledgement.
Already admitted dirt can be traversed without repeatedly rewriting it.

Preserve queued messages across pauses using the existing arena inbox pattern.
On reset, clear requests, outcomes, pending movement, and sequence state with the
world/session generation. Restore original materials and damage with the selected
world snapshot. Death stops new requests and movement; already converted dirt
persists until reset. A changed pose or obsolete proposal cannot use an old
acknowledgement to teleport or bypass current collision checks.

## Underground and emerged body rules

Gameplay's special burrow query may phase only through currently published dirt.
It must keep complete-body bounds, protected terrain, liquid, authored-object,
and live-body checks, including the swept volume when turning. Other species and
ordinary queries retain their accepted collision rules.

Maintain the authored one-to-two-level travel depth and keep earth opaque. The
Worm cannot acquire hidden targets from live poses while underground; it retains
only actual sightings and must surface to look. The actual head/mouth must be
clear of the current surface by at least one voxel level before boulder windup and
again before release. Terrain arriving during windup can therefore cancel release.
No underground movement or HP indicators are added to human play. Released shots
retain their source identity under the existing projectile policy.

A Boulder excludes its frozen caster ID from radial damage and knockback, allowing
useful close shots against clustered melee enemies. Its ordinary swept body,
terrain, and barrier contacts still determine the impact, and shot admission still
requires an observed target and useful damage. This exception is specific to the
frozen WormBoulder source ability; Fireball and Wisp Ember retain caster damage.

## Focused validation required at dispatch

1. Exercise every HP transition above through the real ledger, followed by ordinary
   damage resolution; include already damaged dirt and repeated conversion.
2. Reject a mixed volume containing one bedrock, protected, static, liquid, or
   out-of-bounds cell with no partial mutation. Include overlapping static/liquid
   occupancy and protected air.
3. Keep air empty and never recreate a cell destroyed by an earlier impact.
4. Resolve earlier damage and shield edits against a pending request; revalidate a
   changed actor pose before using an accepted outcome.
5. Cover duplicate and stale identities, bounded volumes, pause retention, death,
   selected-world reset, and clearing old outcomes.
6. Verify exact dirty columns, unchanged publication on no-op conversion, actual
   material rendering, and collision refresh after a missed revision.
7. Sweep the complete four- and six-hex Worm bodies against thin protected/static
   obstacles, liquids, world edges, and other actors; no center-only admission.
8. Reject underground and partially emerged attacks, accept a sufficiently exposed
   head, and cancel a release when terrain covers the mouth during windup.

Typed tests establish mutation and movement legality. Fresh raw renders establish
opaque hiding, exposed-head presentation, and persistent dirt appearance; motion
and control feel remain separate human or video evidence. Focused world, geometry,
gameplay and actual-world application tests have run; exact counts and source
identities are recorded in
[bestiary validation](../../../../systems/arena-bestiary-validation.md).
Native Worm captures and final candidate checks remain in progress.
