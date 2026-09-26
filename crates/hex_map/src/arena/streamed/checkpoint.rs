//! Atomic session owner codec. Restoration stages all sparse truth before adoption.
use super::*;
use hex_core::TerrainVoxelHealth;
use hex_world_runtime::{
    CancellationToken, ErrorKind, FiniteChunkCheckpoint, FiniteSessionHeader, OwnerRecord,
    RuntimeError, RuntimeResult, SessionCheckpoint,
};
use serde::{Deserialize, Serialize};
const OWNER: &str = "world";
const FORMAT: &str = "grand-map-v1";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    version: u32,
    finite: FiniteSessionHeader,
    next_transaction: u64,
    burrow_sequences: BTreeMap<u32, u64>,
    consumed_batches: Vec<u64>,
    damage_chunks: Vec<ChunkId>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DamageCell {
    position: VoxelPosition,
    remaining: u8,
    maximum: u8,
}
fn error(message: impl ToString) -> RuntimeError {
    RuntimeError {
        kind: ErrorKind::InvalidData,
        message: message.to_string(),
    }
}
fn encode(key: String, value: &impl Serialize) -> RuntimeResult<OwnerRecord> {
    Ok(OwnerRecord {
        owner: OWNER.into(),
        key,
        format: FORMAT.into(),
        bytes: ron::to_string(value).map_err(error)?.into_bytes(),
    })
}
fn read<T: serde::de::DeserializeOwned>(
    snapshot: &SessionCheckpoint,
    key: &str,
) -> RuntimeResult<T> {
    let bytes = snapshot
        .record(OWNER, key, FORMAT)?
        .ok_or_else(|| error(format!("missing map checkpoint {key}")))?;
    ron::de::from_bytes(&bytes).map_err(error)
}
fn finite_key(c: ChunkId) -> String {
    format!("finite/{}/{}", c.q, c.r)
}
fn damage_key(c: ChunkId) -> String {
    format!("damage/{}/{}", c.q, c.r)
}
/// Export owner records at a quiescent gameplay boundary, one finite partition at a time.
pub fn export_records(
    world: &World,
) -> Result<impl Iterator<Item = RuntimeResult<OwnerRecord>> + '_, String> {
    let state = world
        .get_resource::<StreamedArena>()
        .ok_or("no streamed world to save")?;
    let damage = world
        .get_resource::<DamagedVoxels>()
        .ok_or("no terrain damage projection")?;
    let mut pages: BTreeMap<ChunkId, Vec<DamageCell>> = BTreeMap::new();
    for (pos, health) in damage.iter() {
        let position = VoxelPosition {
            column: world_hex(pos.coord),
            level: pos.level,
        };
        pages
            .entry(position.column.chunk())
            .or_default()
            .push(DamageCell {
                position,
                remaining: health.remaining,
                maximum: health.maximum,
            });
    }
    let header = Header {
        version: 1,
        finite: state.edits.checkpoint_header(),
        next_transaction: state.next_transaction,
        burrow_sequences: world.resource::<ArenaWorldState>().burrow_sequences.clone(),
        consumed_batches: world
            .resource::<crate::terrain_damage::TerrainDamageState>()
            .checkpoint_batches(),
        damage_chunks: pages.keys().copied().collect(),
    };
    let header = encode("header".into(), &header).map_err(|e| e.to_string())?;
    let finite = state.edits.checkpoint_partitions().map(|part| {
        let part = part?;
        encode(finite_key(part.coordinate), &part)
    });
    let damage = pages.into_iter().map(|(id, mut cells)| {
        cells.sort_by_key(|c| c.position);
        encode(damage_key(id), &cells)
    });
    Ok(std::iter::once(Ok(header)).chain(finite).chain(damage))
}
/// Completely validated sparse replacement. It holds no extra nonresident terrain bytes.
pub struct StagedMapCheckpoint {
    session: FiniteWorldSession,
    header: Header,
    damage: Vec<(TilePos, TerrainVoxelHealth)>,
}
/// Validate source, all finite overlays, material toughness and counters without mutation.
pub fn stage_restore(
    world: &World,
    snapshot: &SessionCheckpoint,
) -> Result<StagedMapCheckpoint, String> {
    stage(world, snapshot).map_err(|e| e.to_string())
}
fn stage(world: &World, snapshot: &SessionCheckpoint) -> RuntimeResult<StagedMapCheckpoint> {
    let state = world
        .get_resource::<StreamedArena>()
        .ok_or_else(|| error("no streamed world"))?;
    let geometry = *world.resource::<ArenaVoxelGeometry>();
    let header: Header = read(snapshot, "header")?;
    if header.version != 1
        || snapshot.identity().manifest_fingerprint != state.runtime.manifest().fingerprint
        || snapshot.identity().world_id != state.runtime.manifest().world_id
        || header.next_transaction == u64::MAX
        || header
            .damage_chunks
            .windows(2)
            .any(|p| p.first() >= p.get(1))
        || header.consumed_batches.len() > 1_048_576
        || header
            .consumed_batches
            .windows(2)
            .any(|p| p.first() >= p.get(1))
    {
        return Err(error("map checkpoint identity or metadata is invalid"));
    }
    let mut expected: BTreeSet<_> = header
        .finite
        .edited_chunks
        .iter()
        .map(|c| finite_key(*c))
        .chain(header.damage_chunks.iter().map(|c| damage_key(*c)))
        .collect();
    expected.insert("header".into());
    let actual: BTreeSet<_> = snapshot
        .records()
        .iter()
        .filter(|r| r.owner == OWNER)
        .map(|r| r.key.clone())
        .collect();
    if actual != expected {
        return Err(error("map checkpoint partition set differs from header"));
    }
    let partitions = header
        .finite
        .edited_chunks
        .iter()
        .map(|id| read::<FiniteChunkCheckpoint>(snapshot, &finite_key(*id)));
    let session = FiniteWorldSession::restore_checkpoint(
        &state.runtime,
        geometry.min_level,
        geometry.max_level,
        &header.finite,
        partitions,
        &CancellationToken::default(),
    )?;
    let source = FileChunkSource::open_workspace(
        package_path_for(world.resource::<ArenaSelection>().map),
        IoLimits::default(),
    )?;
    if source.manifest().fingerprint != state.runtime.manifest().fingerprint {
        return Err(error("source package changed while staging resume"));
    }
    let substances = world.resource::<hex_assets::SubstanceTable>();
    let mut damage = vec![];
    for &id in &header.damage_chunks {
        let cells: Vec<DamageCell> = read(snapshot, &damage_key(id))?;
        if cells.is_empty()
            || cells.len() > 1_048_576
            || cells
                .windows(2)
                .any(|p| p.first().map(|c| c.position) >= p.get(1).map(|c| c.position))
        {
            return Err(error("noncanonical damage partition"));
        }
        let chunk = source.load_chunk(id)?;
        let partition: Option<FiniteChunkCheckpoint> =
            if header.finite.edited_chunks.binary_search(&id).is_ok() {
                Some(read(snapshot, &finite_key(id))?)
            } else {
                None
            };
        for cell in cells {
            let p = cell.position;
            if p.column.chunk() != id
                || p.level < geometry.min_level
                || p.level > geometry.max_level
            {
                return Err(error(
                    "damage position outside its partition or vertical bounds",
                ));
            }
            let terrain = partition
                .as_ref()
                .and_then(|part| {
                    part.terrain_edits
                        .binary_search_by_key(&p, |e| e.position)
                        .ok()
                        .and_then(|i| part.terrain_edits.get(i))
                        .map(|edit| edit.material.as_deref())
                })
                .unwrap_or_else(|| {
                    chunk
                        .columns
                        .iter()
                        .find(|c| c.position == p.column)
                        .and_then(|c| c.material_at(p.level))
                });
            let removed = partition
                .as_ref()
                .is_some_and(|part| part.object_removed.binary_search(&p).is_ok());
            let name = terrain
                .or_else(|| {
                    (!removed)
                        .then(|| {
                            chunk
                                .semantics
                                .occupancy
                                .iter()
                                .find(|c| c.position == p.column)
                                .and_then(|c| c.material_at(p.level))
                        })
                        .flatten()
                })
                .ok_or_else(|| error("partial health refers to removed or empty voxel"))?;
            let material = super::super::forest::material_id(name, substances).map_err(error)?;
            if substances.toughness(material) != Some(cell.maximum) {
                return Err(error(
                    "saved partial health toughness differs from material",
                ));
            }
            let health = TerrainVoxelHealth::new(cell.remaining, cell.maximum)
                .filter(|h| h.is_damaged())
                .ok_or_else(|| error("invalid partial health"))?;
            let coord = local(p.column).ok_or_else(|| error("damage coordinate overflow"))?;
            damage.push((TilePos::new(coord, p.level), health));
        }
    }
    Ok(StagedMapCheckpoint {
        session,
        header,
        damage,
    })
}
/// Adopt a validated replacement without advancing/resetting gameplay generation.
/// The application stays paused until `restore_ready` and its own body checks pass.
pub fn commit_staged(
    world: &mut World,
    staged: StagedMapCheckpoint,
    player_interest: Vec3,
) -> Result<(), String> {
    if !player_interest.is_finite() {
        return Err("resume player interest is nonfinite".into());
    }
    let geometry = *world.resource::<ArenaVoxelGeometry>();
    let center = world_hex(HexCoord::from_world(player_interest));
    if center
        .checked_distance(WorldHex::new(0, 0))
        .map_err(|e| e.to_string())?
        > u64::from(geometry.radius)
    {
        return Err("resume player is outside world".into());
    }
    // Validate request before replacing authoritative state; set_interests is transactional.
    world
        .resource_mut::<StreamedArena>()
        .runtime
        .set_interests(vec![ResidencyRequest {
            id: "resume-critical".into(),
            center,
            radius: 20,
            retention_radius: 20,
            priority: 255,
        }])
        .map_err(|e| e.to_string())?;
    render::clear(world);
    world.resource_scope(|world, mut state: Mut<StreamedArena>| {
        state.edits = staged.session;
        state.next_transaction = staged.header.next_transaction;
        state.projected.clear();
        state.interest_key = None;
        state.failure = None;
        let view = &mut *world.resource_mut::<ArenaTerrainView>();
        view.columns.clear();
        view.object_columns.clear();
        view.liquids.clear();
        view.static_spans.clear();
        if let Some(r) = &mut view.residency {
            r.ready.clear();
        }
        publish(&mut state, view, &geometry);
    });
    world.resource_scope(
        |world, mut state: Mut<crate::terrain_damage::TerrainDamageState>| {
            state.restore(staged.damage, &mut world.resource_mut::<DamagedVoxels>());
            state.restore_checkpoint_batches(staged.header.consumed_batches);
        },
    );
    world.resource_mut::<ArenaWorldState>().burrow_sequences = staged.header.burrow_sequences;
    *world.resource_mut::<ArenaStreamInterest>() = ArenaStreamInterest {
        position: player_interest,
        velocity: Vec3::ZERO,
    };
    let mut inbox = world.resource_mut::<ArenaInbox>();
    inbox.edits.clear();
    inbox.impacts.clear();
    inbox.burrows.clear();
    Ok(())
}
/// True only when the exact critical collision neighborhood has been published.
#[must_use]
pub fn restore_ready(world: &World, position: Vec3) -> bool {
    if !position.is_finite() {
        return false;
    }
    let Some(view) = world.get_resource::<ArenaTerrainView>() else {
        return false;
    };
    HexCoord::from_world(position)
        .within_radius(3)
        .into_iter()
        .all(|c| {
            view.residency.as_ref().is_some_and(|r| {
                r.at(c, *world.resource::<ArenaVoxelGeometry>()) == ArenaAvailability::Ready
            })
        })
}
