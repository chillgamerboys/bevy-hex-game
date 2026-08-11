# World and disclosure map

Refreshed against `origin/dev@1dca1065c7681737ce424fa187879ea31974e356`
and composed wave merge `e610e26c50398e43ff23bc4db0890ba7463f11ae` on 2026-08-10.
`WorldSnapshotV1` was ratified under the user's temporary world-owner delegation. If
refreshed source disagrees with a disposition below, L3 stops and the manifest is amended.

## Current terrain authority

`VoxelMap` is the world source of truth (`crates/hex_map/src/voxel.rs:156-247`). It is
deliberately private in meaning even though its Rust visibility permits map-local tests.
Outside consumers see only tile components and shared resources described in
`crates/hex_map/CLAUDE.md`.

The map lifecycle is centralized in `crates/hex_map/src/grid.rs`:

- plugin/system registration and resource setup: lines 70-155;
- generation and `TerrainReady` publication: lines 168-343;
- teardown of map and semantic resources: lines 346-382;
- run-based tile publication: line 385 onward;
- direct edits/impacts and rebuilt projections: later `TerrainSystems::ApplyWorld`
  regions in the same file.

`GenerationReport::map_fingerprint` is the existing generated-identity hook at
`crates/hex_map/src/procedural.rs:36-76`. It hashes initial generated state, but terrain
mutation at `crates/hex_map/src/grid.rs:805-1085` does not refresh it. It must not identify
the current world for reconnect or Campaign persistence.

## Public state that a complete snapshot must preserve

| Public fact | Current anchor | Snapshot disposition |
|---|---|---|
| Every voxel substance | `VoxelMap::columns`, `crates/hex_map/src/voxel.rs:156-247` | export stable-name compressed runs sorted by coordinate and level; import resolves names fail-closed and reconstructs every level including air gaps |
| Partial voxel damage | `DamagedVoxels`, `crates/hex_core/src/terrain_impact.rs:258-324` | include sorted exact health entries and validate each against imported material toughness |
| Spawn anchors | `MapAnchors`, `crates/hex_core/src/terrain.rs:57-109` | include stable name + exact `TilePos` |
| Interior floors/roofs | `InteriorRegions`, `crates/hex_core/src/terrain.rs:135-219` | include region ids, floor surfaces, and roof voxels |
| Special movement | `SpecialMovementRegions`, `crates/hex_core/src/terrain.rs:239-307` | include exact stack-safe surface membership |
| Biome membership | `BiomeRegions`, `crates/hex_core/src/spatial.rs:81-130` | include exact published regions used by semantic/presentation consumers |
| Traversal blockers | `TraversalBlockers`, `crates/hex_core/src/spatial.rs:19-73` | include exact stack-safe blocker projection; never infer it from rendering |
| View hint | `MapViewHint`, `crates/hex_core/src/terrain.rs:327-350` | preserve exact bit-pattern semantic framing input, not a client camera transform |
| Presentation semantics | `MapPresentationProjection`, `crates/hex_map/src/procedural_v3/materialize.rs:53-161` | export current stable asset placement/rotation/blocker/edit-protection consequences for features and crystals; exclude recipe plans and structures already represented by voxels |
| Liquid semantics | `MapPresentationProjection::liquids`, `crates/hex_map/src/procedural_v3/materialize.rs:53-75` | include stable material, exact voxel, flow class, and downstream position |
| Gameplay lights | `GameplayLight`, `crates/hex_core/src/perception.rs:346-365` | preserve exact world-owned light inputs; do not serialize renderer lights |
| Generation identity | `GenerationReport`, `crates/hex_map/src/procedural.rs:36-76` | manifest carries generator/settings/map identity; snapshot retains public identity needed to compare restore |
| Readiness | `TerrainReady` | never serialized as a fact; publish it only after full import validation and rebuilt projections succeed |

The round-trip oracle is a new domain-separated `PublicWorldFingerprintV1` over all rows above plus
the tile publication tuple `(TilePos, RunBottom, HexSpan, SubstanceId, Headroom)` in stable
order. It must not compare only columns or renderer transforms. Actor footing is checked by
the gameplay consumer after import using the ordinary `Footing`/occupancy contracts.

V3's private `MapPresentationProjection` contains liquid, crystal, vegetation, gameplay-
light, terrain-edit-protection, and feature-retention consequences
(`crates/hex_map/src/procedural_v3/materialize.rs:53-161`). Its initial complete
fingerprint inventory is at `materialize.rs:597-652`. The 2026-08-10 amendment ratified
stable generator-neutral
presentation consequences in `WorldSnapshotV1`; regeneration from `SessionManifestV1` is
explicitly rejected for both reconnect and Campaign restore.

## `WorldSnapshotV1` shape

The shared wire type contains a version tag and bounded vectors of stable domain values;
it never exposes `VoxelMap`, generator plans, Bevy `Entity`, handles, meshes, materials, or
private recipe metadata. The world-owned exporter/importer is the only implementation that
may query `hex_map` storage.

Import is transactional and must also hydrate the private authoritative terrain-damage
ledger at `crates/hex_map/src/terrain_damage.rs:19-136`; restoring only public
`DamagedVoxels` would incorrectly give a damaged voxel full health on the next impact.

Import proceeds as follows:

1. reject version, size, duplicate/unsorted positions, invalid substance names, invalid
   health, dangling region data, or mismatched manifest before mutation;
2. construct a complete candidate `VoxelMap` and semantic resources off to the side;
3. tear down the old projection at an authority boundary;
4. install candidate resources and run the ordinary grid publication path;
5. publish `TerrainReady` only after the complete public fingerprint matches the snapshot.

Static launch does not send this snapshot. Peers generate from `SessionManifestV1` and
compare the same public fingerprint. Snapshot import is for restart-capable reconnect and
later Campaign persistence.

## Disclosure

`DamagedVoxels`, terrain outcomes, `CombatState`, and hostile lattice state are world/sim
truth, not permission to disclose. `hex_perception` publishes faction observation and
`LocalMapKnowledge`; Replicon visibility consumes the existing shared player-faction view.
It must withdraw hostile entities/components when observation no longer permits them and
must never infer visibility from renderer `Visibility`.

All co-op humans share the player-faction knowledge view. Per-seat visibility is therefore
not a fog split, but authorization still filters session secrets, reconnect credentials,
private lobby data, and hostile lattice facts. The host's full snapshot is projected into
a client-authorized `LiveSessionSnapshotV1`; it is never serialized wholesale.

Player knowledge is stateful: remembered surfaces retain their last-seen state after a
hidden mutation and cannot be re-derived from current terrain plus current sight. Reconnect
therefore carries a separate authorized `PlayerKnowledgeSnapshotV1` in
`LiveSessionSnapshotV1`, sourced from `crates/hex_perception/src/snapshots.rs:20-120` and
`crates/hex_perception/src/knowledge.rs:20-218`. It includes only the shared player-faction
view. Derived caches such as current surface snapshots, `LocalMapKnowledge`, and occupancy
are rebuilt; hostile-faction knowledge is never serialized to clients.

Export/import also waits for empty private terrain edit/impact queues. The foundation needs
a narrow world readiness/boundary hook because no public contract currently exposes that
quiescence.

## Territory

- #186 visibility is represented on `dev` by `3f2f6dc4`.
- #187's reserved surface-feature vocabulary is represented by `0e14e89d`; it is excluded
  from snapshot state until a live producer exists.
- #190's unique first-person/cutaway work is represented through composed `32577c26` and
  delivery reconciliation `1dca1065`.

All former world territory blockers are clear. A new snapshot implementation in `hex_map`
still does not authorize changes to gameplay-owned footing, occupancy, or combat state.
