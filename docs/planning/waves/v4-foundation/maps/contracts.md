# Public V4 contract implementation brief

These types are new, pure shared vocabulary in `hex_world_contracts`. Read the wave
decisions first. If reality contradicts this brief, report the contradiction to the
coordinator; do not silently create a second wire vocabulary.

## Canonical geometry and packages

- `WorldHex { q: i64, r: i64 }`: axial coordinates; checked cube-distance,
  translation and 60-degree rotation. Never silently overflow.
- `ChunkId { q: i64, r: i64 }`: Euclidean division of WorldHex by CHUNK_SIZE=16.
- `VoxelPosition { column: WorldHex, level: i32 }`.
- `VoxelRun { bottom: i32, top: i32, material: String }`: exclusive top;
  canonical sorted disjoint non-air intervals, adjacent equal materials coalesced.
- `ColumnData { position: WorldHex, runs: Vec<VoxelRun> }`.
- `MaterialSpec { id: String, solid: bool, diggable: bool, color: [u8;4] }`.
- `FeatureSummary { id: String, region_id: String, kind: String,
  anchor: VoxelPosition, asset: Option<String> }`.
- `MapSummaryCell { position: WorldHex, level: i32, material: String,
  region_id: String }`.
- `RegionDescriptor { id: String, origin: WorldHex, radius: u32,
  source_fingerprint: u64 }`. Region footprints are exact integer hex disks in v4's
  first data format; content/feature masks inside a region remain independent.
- `ChunkPackage { schema_version: u32, world_id: String, coordinate: ChunkId,
  source_fingerprint: u64, columns: Vec<ColumnData>, features: Vec<FeatureSummary>,
  fingerprint: u64 }`. Sorted coordinate order. Global chunks can contain several
  region contributions. Never use region-local IDs as globally unique identities.
- `ChunkDescriptor { coordinate: ChunkId, fingerprint: u64, path: String }`.
- `BoundarySample { a: WorldHex, b: WorldHex, ground_level: i32,
  water_level: Option<i32>, required_access: bool }`.
- `BoundaryContract { id: String, region_a: String, region_b: String,
  samples: Vec<BoundarySample> }`.
- `WorldManifest { schema_version: u32, world_id: String, compiler_version: String,
  source_fingerprint: u64, materials: Vec<MaterialSpec>, regions: Vec<RegionDescriptor>,
  chunks: Vec<ChunkDescriptor>, boundaries: Vec<BoundaryContract>,
  summary: Vec<MapSummaryCell>, features: Vec<FeatureSummary>, fingerprint: u64 }`.
- `WorldPackage { manifest: WorldManifest, chunks: BTreeMap<ChunkId, ChunkPackage> }`.

Public validation rejects duplicate IDs/coordinates, malformed runs/materials,
noncanonical ordering, unknown schema versions, wrong-world chunks, wrong chunk
membership, invalid hashes and unsafe relative package paths. Hashes use canonical
serialization with the fingerprint field excluded/zeroed; use fixed little-endian
integers where directly hashing numeric values. No float serialization in authority.
`seal` canonicalizes and fingerprints; `validate` never silently repairs untrusted
input. Expose both operations plus `fingerprint(value)` with Result errors.

## Availability and edits

- `QueryResult<T>`: `Ready(T)`, `Unloaded(ChunkId)`, `OutsideWorld`.
- `Surface { position: VoxelPosition, material: String, headroom: Option<u32> }`:
  exact air clearance; None means unbounded sky, not unavailable or unknown.
- `WorldQuery`: `voxel(VoxelPosition) -> QueryResult<Option<String>>` (None is actual
  air), `surfaces(WorldHex) -> QueryResult<Vec<Surface>>` (every solid exposed stack),
  `revision(ChunkId) -> Option<u64>`.
- `ResidencyRequest { id: String, center: WorldHex, radius: u32,
  retention_radius: u32, priority: u8 }`: actor/operation interest, independent of turns.
- `VoxelEdit { position: VoxelPosition, material: Option<String> }`.
- `WorldEditTransaction { id: String, expected_revisions: BTreeMap<ChunkId,u64>,
  edits: Vec<VoxelEdit> }`.
- `WorldChange { transaction_id: String, revisions: BTreeMap<ChunkId,u64>,
  changed_columns: Vec<WorldHex> }`.

No Bevy resources or app scheduling in this crate. Tests must independently prove
negative coordinates, stacked intervals, duplicate rejection, hashing integrity and
malformed input rejection. Wire allocation limits are per package/operation rather
than a total-world column cap; bound byte reads in the filesystem adapter.
