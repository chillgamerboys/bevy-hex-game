# L3 order — World replication and disclosure

## Objective

Implement the world-owned complete export/import contract, current-world fingerprint,
terrain deltas, reconnect restoration, static-generation verification, and player-faction
disclosure projection. Only `hex_map` may inspect world storage.

The user ratified `WorldSnapshotV1` under the temporary world-owner delegation on
2026-08-10 and every L3 dispatch blocker is now true. Read `crates/hex_map/CLAUDE.md`,
`docs/systems/map.md`, the manifest amendment, and maps. A disagreement is an escalation,
not a judgment call.

## Locked decisions (verbatim)

- “Multiplayer supports only shipped content with an exact accepted-content fingerprint.
  Custom Creator content and host content transfer are rejected in this epic.”
- “The listen host owns simulation, AI, world mutation, admission, global pause, and saves.
  Remote clients submit intents and apply disclosure-safe authoritative projections; there
  is no lockstep, rollback, prediction, host migration, or dedicated server in this epic.”
- “`CombatState` remains host-only. Clients receive exact authorized unit/session
  projections and the existing shared player-faction knowledge view, never undisclosed
  hostile lattice facts.”
- “Every peer generates the static map locally from the frozen `SessionManifestV1`, reports
  the complete public map fingerprint, and activates only after exact agreement. Reconnect
  uses a bounded host snapshot plus deltas newer than its baseline sequence.”
- “Serialized commands are capped at 64 KiB; decoded strings, vectors, paths, and domain
  values are validated; request bursts are rate limited; and snapshot allocation is capped
  before deserialization.”

## Banked maps

- `../maps/world-and-disclosure.md`
- `../maps/authority-and-command.md`
- `../maps/territory.md`

## Owned territory

Only L3 paths/regions and its manifest row. Do not edit gameplay footing, occupancy,
combat, `hex_multiplayer`, root composition, or client UI. Shared DTO changes require a
foundation amendment and owner review, not a lane-local protocol edit.

## Required implementation

1. Add deterministic, bounded `export_world_snapshot_v1` that serializes all material
   voxels by stable substance name, partial damage, anchors, interior/special/biome regions,
   traversal blockers, gameplay lights, view/presentation semantic projections,
   and every public fact required to reproduce the same `TerrainReady` world. Export
   current stable asset identity/placement/rotation/blocker/edit-protection consequences
   for features and crystals plus liquid flow/downstream state; never regenerate these
   from private plans during reconnect or Campaign restore.
2. Add a canonical current-public-world fingerprint independent of generation-time
   `GenerationReport`. It changes after edits/damage and covers both storage and complete
   public semantic projections.
3. Implement transactional import: prevalidate complete bounded DTO, resolve stable names,
   build candidate state, replace at an authority boundary, reuse ordinary tile publication,
   hydrate both `DamagedVoxels` and the private remaining-health ledger, verify fingerprint,
   then publish `TerrainReady`. Any failure leaves the prior world or a clean not-ready
   state; never a partial ready world.
4. Prove all tile publication tuples (`TilePos`, `RunBottom`, `HexSpan`, `SubstanceId`,
   `Headroom`) are identical after round trip. Compare damaged voxels, anchors/regions,
   blockers, knowledge inputs, and actor footing through typed hooks.
5. Diff two canonical snapshots at a quiescent authority boundary into ordered
   `WorldDeltaV1` upserts/removals. Apply against the required base fingerprint in a
   candidate state, verify the target fingerprint, commit transactionally, and treat an
   already-applied authority sequence as an idempotent success.
6. For initial launch, compare the locally generated current-public-world fingerprint with
   the frozen manifest/report. Do not send terrain when exact generation succeeds.
7. For reconnect, export only at the shared authority boundary, project the client-
   authorized world/session view, attach baseline sequence, then let the client apply only
   newer ordered deltas.
8. Integrate the existing shared player-faction knowledge view with Replicon visibility.
   Withdraw no-longer-observed hostile entities/components; never use renderer visibility
   or `DamagedVoxels` as a disclosure grant. Export remembered player knowledge separately
   as the authorized `PlayerKnowledgeSnapshotV1`; rebuild derived current caches and never
   export hostile-faction knowledge.
9. Leave map generator plans, patch masks, recipe identities, `PlannedStructure` when its
   only consequence is in voxels, meshes/materials, entity ids, cameras, transport state,
   and hostile knowledge out of the snapshot. Do not serialize the reserved #187 surface
   feature vocabulary until a live producer exists.

## Required evidence

- Export → teardown → import over representative authored/V1/V2/V3 worlds and a mutated
  terrain fixture; exact complete fingerprint equality every time.
- Damage, split/merged runs, caves/stacked surfaces, headroom, anchors, interiors, special
  movement, biomes, blockers, semantic presentation, and actor footing assertions.
- Reject unknown/air-as-material names, duplicates, noncanonical ordering, impossible
  health, dangling regions, oversized counts/strings/allocation, wrong version/fingerprint,
  and truncated input without panic.
- Initial host/client generation verification accepts exact match and refuses mismatches
  before `GameplayPhase::Active`.
- Reconnect baseline plus later deltas reaches exactly the current host public fingerprint.
- Disclosure tests observe, withdraw, and re-observe a hostile while proving private
  lattice/combat state is absent.
- Map partition checks and selector-chosen complete gate. Static frames inspect only the
  affected restored rendering; typed hooks prove logic.

## Handoff

Update only the L3 manifest row to `in-review`. Record the ratifying world owner/date,
snapshot limits, fingerprint schema/version, all round-trip fixtures, and static frame paths.
The coordinator alone records `merged-to-wave`.
