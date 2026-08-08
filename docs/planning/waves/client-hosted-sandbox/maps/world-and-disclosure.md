# World and disclosure map

Banked against `origin/dev@92662d456746506093e8de61f54f1d619085e1fe`.
`WorldSnapshotV1` is a world-owned public contract. If the world owner does not ratify
this map or refreshed source disagrees, L3 is blocked.

## Current terrain authority

`VoxelMap` is the world source of truth (`crates/hex_map/src/voxel.rs:147-216`). It is
deliberately private in meaning even though its Rust visibility permits map-local tests.
Outside consumers see only tile components and shared resources described in
`crates/hex_map/CLAUDE.md`.

The map lifecycle is centralized in `crates/hex_map/src/grid.rs`:

- plugin/system registration and resource setup: lines 70-155;
- generation and `TerrainReady` publication: lines 160-343;
- teardown of map and semantic resources: lines 346-377;
- run-based tile publication: lines 379 onward;
- direct edits/impacts and rebuilt projections: later `TerrainSystems::ApplyWorld`
  regions in the same file.

`GenerationReport::map_fingerprint` is the existing generated-identity hook at
`crates/hex_map/src/procedural.rs:36-76`. It hashes initial generated state, but terrain
mutation at `crates/hex_map/src/grid.rs:800-1000` does not refresh it. It must not identify
the current world for reconnect or Campaign persistence.

## Public state that a complete snapshot must preserve

| Public fact | Current anchor | Snapshot disposition |
|---|---|---|
| Every voxel substance | `VoxelMap::columns`, `crates/hex_map/src/voxel.rs:147-197` | export stable substance names, sorted by coordinate then level; import resolves names fail-closed through the accepted table |
| Partial voxel damage | `DamagedVoxels`, `crates/hex_core/src/terrain_impact.rs:258-324` | include sorted exact health entries and validate each against imported material toughness |
| Spawn anchors | `MapAnchors`, `crates/hex_core/src/terrain.rs:57-109` | include stable name + exact `TilePos` |
| Interior floors/roofs | `InteriorRegions`, `crates/hex_core/src/terrain.rs:111-219` | include region ids, floor surfaces, and roof voxels |
| Special movement | `SpecialMovementRegions`, `crates/hex_core/src/terrain.rs:221 onward` | include exact stack-safe surface membership |
| Biome membership | `BiomeRegions`, `crates/hex_core/src/spatial.rs` | include exact published regions used by semantic/presentation consumers |
| Traversal blockers | `TraversalBlockers`, `crates/hex_core/src/spatial.rs` | include exact stack-safe blocker projection; never infer it from rendering |
| View hint | `MapViewHint`, `crates/hex_core/src/terrain.rs` | preserve semantic framing input, not a client camera transform |
| Presentation semantics | `MapPresentationProjection`, `crates/hex_map/src/procedural_v3/mod.rs` | preserve only semantic projection required to rebuild identical public tile/object/cutaway output |
| Gameplay lights | `GameplayLight`, `crates/hex_core/src/perception.rs:228-265` | preserve exact world-owned light inputs; do not serialize renderer lights |
| Generation identity | `GenerationReport`, `crates/hex_map/src/procedural.rs:36-76` | manifest carries generator/settings/map identity; snapshot retains public identity needed to compare restore |
| Readiness | `TerrainReady` | never serialized as a fact; publish it only after full import validation and rebuilt projections succeed |

The round-trip oracle is a new domain-separated `PublicWorldFingerprintV1` over all rows above plus
the tile publication tuple `(TilePos, RunBottom, HexSpan, SubstanceId, Headroom)` in stable
order. It must not compare only columns or renderer transforms. Actor footing is checked by
the gameplay consumer after import using the ordinary `Footing`/occupancy contracts.

V3's private `MapPresentationProjection` contains liquid, crystal, vegetation, gameplay-
light, terrain-edit-protection, and feature-retention consequences
(`crates/hex_map/src/procedural_v3/materialize.rs:48-161`). Its initial complete
fingerprint inventory is at `materialize.rs:597-652`. The world owner must choose and
ratify one of two contracts before L3 starts:

1. **Recommended:** promote stable generator-neutral presentation consequences into
   `WorldSnapshotV1`, and let `hex_map` reconstruct its private projection on import.
2. Regenerate presentation consequences from `SessionManifestV1`. This is adequate for
   direct reconnect but does **not** satisfy the complete generator-independent Campaign
   snapshot promised by the epic.

That is a semantic choice and current stop condition, not an encoding detail.

## `WorldSnapshotV1` shape

The shared wire type contains a version tag and bounded vectors of stable domain values;
it never exposes `VoxelMap`, generator plans, Bevy `Entity`, handles, meshes, materials, or
private recipe metadata. The world-owned exporter/importer is the only implementation that
may query `hex_map` storage.

Import is transactional and must also hydrate the private authoritative terrain-damage
ledger at `crates/hex_map/src/terrain_damage.rs:18-143`; restoring only public
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
`LiveSessionSnapshotV1`, sourced from `crates/hex_perception/src/snapshots.rs:13-116` and
`crates/hex_perception/src/knowledge.rs:15-218`. It includes only the shared player-faction
view. Derived caches such as current surface snapshots, `LocalMapKnowledge`, and occupancy
are rebuilt; hostile-faction knowledge is never serialized to clients.

Export/import also waits for empty private terrain edit/impact queues. The foundation needs
a narrow world readiness/boundary hook because no public contract currently exposes that
quiescence.

## Territory

- #186 changes perception/core/save contracts used by disclosure and snapshot restore.
- #187 adds a new public surface-feature contract that may become part of the complete
  public-world fingerprint.
- #190 is stacked on #186 and changes world camera/cutaway plus the same perception seam.

L3 waits for those PRs or an exact remap and for explicit world-owner agreement. A new
snapshot implementation in `hex_map` does not authorize changes to gameplay-owned footing,
occupancy, or combat state.
