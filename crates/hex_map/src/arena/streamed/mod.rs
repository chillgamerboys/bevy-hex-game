//! Bounded V4 authority for the northern exploration map.
//! Only compact admitted runs cross into the continuous controller.
use super::{ArenaInbox, ArenaWorldState, Content};
use bevy::prelude::*;
use hex_core::arena::{
    ArenaActorStreamInterests, ArenaAvailability, ArenaMap, ArenaOverview, ArenaPackageIdentity,
    ArenaReset, ArenaResidency, ArenaSelection, ArenaSolidSpan, ArenaStaticSpan,
    ArenaStreamInterest, ArenaSystems, ArenaTerrainView, ArenaTick, ArenaVoxelGeometry,
};
use hex_core::{
    DamagedVoxels, HexCoord, SubstanceId, TerrainEdit, TerrainImpactOutcome,
    TerrainImpactRejection, TerrainImpactResult, TilePos,
};
use hex_schematic::v4::grand::GrandBiomeMap;
use hex_schematic::v4::northern::NorthernOverview;
use hex_world_contracts::{
    ChunkId, ResidencyRequest, VoxelEdit, VoxelPosition, WorldEditTransaction, WorldHex,
};
use hex_world_runtime::{
    FileChunkSource, FiniteWorldSession, IoLimits, RuntimeConfig, WorldRuntime,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
mod burrow;
pub mod checkpoint;
mod grand_forest;
mod grand_landmarks;
mod render;
#[cfg(test)]
mod tests;

pub(super) fn clear_render(world: &mut World) {
    render::clear(world);
}

/// Local world authority and sparse run-local edits; never retains the full fine map.
#[derive(Resource)]
pub struct StreamedArena {
    /// Bounded immutable source residency.
    pub runtime: WorldRuntime,
    /// Exact session mutation layer following resident source lifetimes.
    pub edits: FiniteWorldSession,
    /// Cached immutable map geography for ocean and overview presentation.
    pub overview: Arc<NorthernOverview>,
    biomes: Option<GrandBiomeMap>,
    generation: u64,
    projected: BTreeMap<ChunkId, u64>,
    interest_key: Option<Vec<WorldHex>>,
    next_transaction: u64,
    policies: BTreeMap<String, SubstanceId>,
    /// Most recent asynchronous source failure, surfaced in the host UI/log.
    pub failure: Option<String>,
    /// Last bounded residency/publication CPU cost, excluding asynchronous mesh work.
    pub publication_ms: f64,
    /// Peak resident count for the current run.
    pub peak_resident: usize,
}

impl StreamedArena {
    /// Compiler-authored entered biome; never depends on distant mesh residency.
    #[must_use]
    pub fn biome_at(&self, position: Vec3) -> Option<&'static str> {
        if !position.is_finite() {
            return None;
        }
        let p = world_hex(HexCoord::from_world(position));
        if p.checked_distance(WorldHex::new(0, 0)).ok()? > u64::from(self.overview.radius) {
            return None;
        }
        self.biomes
            .as_ref()?
            .label_at(position.to_array().map(f64::from))
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<ArenaStreamInterest>()
        .init_resource::<ArenaActorStreamInterests>()
        .add_systems(Update, pump.run_if(resource_exists::<StreamedArena>))
        .add_systems(
            ArenaTick,
            apply
                .in_set(ArenaSystems::ApplyTerrain)
                .run_if(resource_exists::<StreamedArena>),
        );
    app.add_systems(
        ArenaTick,
        burrow::apply
            .after(apply)
            .in_set(ArenaSystems::ApplyTerrain)
            .run_if(resource_exists::<StreamedArena>),
    );
    render::plugin(app);
}

/// Resolve only this map's selected immutable package.
#[must_use]
pub fn package_path() -> PathBuf {
    std::env::var_os("HEX_NORTHERN_WORLD")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            super::forest::asset_root().join("assets/config/v4/northern-archipelago/compiled")
        })
}

/// Package selected by map identity; existing Northern path remains supported.
pub fn package_path_for(map: ArenaMap) -> PathBuf {
    if map == ArenaMap::GrandV4 {
        std::env::var_os("HEX_GRAND_WORLD")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                super::forest::asset_root().join("assets/config/v4/grand-v4/compiled")
            })
    } else {
        package_path()
    }
}
pub(super) fn initialize(world: &mut World, mut content: Content) -> Result<(), String> {
    let selection = *world.resource::<ArenaSelection>();
    let grand = selection.map == ArenaMap::GrandV4;
    let directory = package_path_for(selection.map);
    if grand {
        content.materials.reinforced_stone = content.substances.id("reinforced_stone");
    }
    let overview_name = if grand {
        "grand-overview.ron"
    } else {
        "northern-overview.ron"
    };
    let overview: NorthernOverview = ron::from_str(
        &std::fs::read_to_string(directory.join(overview_name)).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let overview = Arc::new(overview);
    let source = Arc::new(
        FileChunkSource::open_workspace(&directory, IoLimits::default())
            .map_err(|e| e.to_string())?,
    );
    validate_overview(&overview, source.manifest())?;
    let biomes = if grand {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(directory.join("grand-biomes.ron"))
            .map_err(|e| e.to_string())?
            .take(1_048_577)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 1_048_576 {
            return Err("Grand biome companion exceeds1MiB".into());
        }
        let map: GrandBiomeMap = ron::de::from_bytes(&bytes).map_err(|e| e.to_string())?;
        if map.version != 1
            || map.package_fingerprint != source.manifest().fingerprint
            || map.source_fingerprint != source.manifest().source_fingerprint
            || map.mainland_rows.len() > 10000
            || map.crystal_rows.len() > 1000
        {
            return Err("Grand biome companion identity/bounds mismatch".into());
        }
        for rows in [&map.mainland_rows, &map.crystal_rows] {
            if rows.windows(2).any(|pair| pair.first() >= pair.get(1))
                || rows.iter().any(|(r, a, b)| {
                    a > b
                        || r.unsigned_abs() > 900
                        || a.unsigned_abs() > 900
                        || b.unsigned_abs() > 900
                })
            {
                return Err("Grand biome rows are invalid".into());
            }
        }
        Some(map)
    } else {
        None
    };
    let mut runtime = WorldRuntime::new(
        source.clone(),
        RuntimeConfig {
            max_resident_chunks: 512,
            max_interests: if grand { 132 } else { 64 },
            max_in_flight_jobs: 2,
            max_publications_per_pump: 2,
            ..default()
        },
    )
    .map_err(|e| e.to_string())?;
    let spawn = Vec3::from_array(overview.player_spawn);
    let center = world_hex(HexCoord::from_world(spawn));
    runtime
        .set_interests(vec![ResidencyRequest {
            id: "player-initial".into(),
            center,
            radius: 6,
            retention_radius: 6,
            priority: 255,
        }])
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let result = runtime.pump();
        if let Some(error) = result.failures.first() {
            return Err(error.error.to_string());
        }
        if HexCoord::from_world(spawn)
            .within_radius(3)
            .into_iter()
            .all(|c| runtime.resident_chunk(world_hex(c).chunk()).is_some())
        {
            break;
        }
        if Instant::now() > deadline {
            return Err("Northern starting terrain did not become ready".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let anchor = runtime
        .resident_chunk(center.chunk())
        .and_then(|chunk| {
            chunk
                .package
                .semantics
                .anchors
                .iter()
                .find(|anchor| {
                    anchor.id
                        == if grand {
                            "grand/anchor/party_start"
                        } else {
                            "northern/anchor/party_start"
                        }
                        && anchor.role == hex_world_contracts::AnchorRole::Gameplay
                })
                .cloned()
        })
        .ok_or("Northern spawn has no matching supported gameplay anchor")?;
    let anchor_column =
        local(anchor.position.column).ok_or("Northern spawn coordinate overflow")?;
    let anchor_level = i16::try_from(anchor.position.level)
        .map_err(|error| format!("Northern spawn height exceeds its physical profile: {error}"))?;
    let anchor_feet =
        anchor_column.to_world((f32::from(anchor_level) + 1.0) * overview.level_height);
    if anchor_feet.distance(spawn) > 0.01 {
        return Err("Northern companion spawn differs from its exact package anchor".into());
    }
    let edits =
        FiniteWorldSession::streamed(&runtime, overview.level_bounds[0], overview.level_bounds[1])
            .map_err(|e| e.to_string())?;
    let generation = world.resource::<ArenaReset>().generation;
    let selection = *world.resource::<ArenaSelection>();
    let geometry = ArenaVoxelGeometry {
        radius: overview.radius,
        level_height: overview.level_height,
        vertical_offset: overview.vertical_offset,
        min_level: overview.level_bounds[0],
        max_level: overview.level_bounds[1],
    };
    let policies = runtime
        .manifest()
        .materials
        .iter()
        .map(|m| {
            let id = super::forest::material_id(&m.id, &content.substances)
                .unwrap_or(content.materials.stone);
            (m.id.clone(), id)
        })
        .collect();
    let catalogue = runtime
        .manifest()
        .chunks
        .iter()
        .filter_map(|c| pair(c.coordinate))
        .collect();
    let mut anchors: BTreeMap<_, _> = overview
        .anchors
        .iter()
        .map(|(name, point)| (name.clone(), Vec3::from_array(*point)))
        .collect();
    if let Some(bay) = anchors.get("bay").copied() {
        anchors.insert("player_look_at".into(), bay);
    }
    let companion = if grand {
        Some(super::forest::expedition_file::load(
            &directory,
            runtime.manifest(),
            geometry,
        )?)
    } else {
        None
    };
    let view = ArenaTerrainView {
        selection,
        anchors,
        spawns: [spawn, spawn],
        expedition: companion.as_ref().and_then(|c| c.sites.clone()),
        package_identity: Some(companion.map_or_else(
            || ArenaPackageIdentity {
                world_id: runtime.manifest().world_id.clone(),
                manifest_fingerprint: runtime.manifest().fingerprint,
                sites_fingerprint: None,
            },
            |c| c.identity,
        )),
        residency: Some(ArenaResidency {
            catalogue,
            ready: BTreeSet::new(),
        }),
        ..default()
    };
    world.insert_resource(StreamedArena {
        runtime,
        edits,
        overview: overview.clone(),
        biomes,
        generation,
        projected: BTreeMap::new(),
        interest_key: None,
        next_transaction: 0,
        policies,
        failure: None,
        publication_ms: 0.0,
        peak_resident: 0,
    });
    world.insert_resource(geometry);
    world.insert_resource(view);
    world
        .resource_mut::<crate::terrain_damage::TerrainDamageState>()
        .reset(&mut DamagedVoxels::default());
    world.insert_resource(DamagedVoxels::default());
    world.insert_resource(ArenaStreamInterest {
        position: spawn,
        velocity: Vec3::ZERO,
    });
    world.insert_resource(super::burrow::materials(&content.substances));
    world.insert_resource(content.substances);
    world.insert_resource(content.elements);
    world.insert_resource(content.damage);
    world.insert_resource(content.materials);
    world.insert_resource(content.art);
    world.insert_resource(ArenaWorldState {
        generation,
        ..default()
    });
    world.remove_resource::<crate::VoxelMap>();
    world.insert_resource(crate::procedural_v3::MapPresentationProjection::default());
    publish_overview(world, &overview, generation);
    world.resource_scope(|world, mut state: Mut<StreamedArena>| {
        let mut view = world.resource_mut::<ArenaTerrainView>();
        publish(&mut state, &mut view, &geometry);
    });
    Ok(())
}

fn validate_overview(
    overview: &NorthernOverview,
    manifest: &hex_world_contracts::WorldManifest,
) -> Result<(), String> {
    let grand = manifest.world_id == "grand-v4";
    let radius = if grand { 900 } else { 700 };
    let max_level = if grand { 1600 } else { 1400 };
    let extent_x = if grand { 1564.0 } else { 1216.0 };
    let extent_z = if grand { 1356.0 } else { 1056.0 };
    let max_height = if grand { 560.0 } else { 490.0 };
    let [region] = manifest.regions.as_slice() else {
        return Err("Northern package requires one finite region".into());
    };
    if overview.version != 1
        || overview.world_id != manifest.world_id
        || overview.package_fingerprint != manifest.fingerprint
        || overview.source_fingerprint != manifest.source_fingerprint
        || overview.materials != manifest.materials
        || region.origin != WorldHex::new(0, 0)
        || region.radius != overview.radius
        || overview.radius != radius
        || overview.level_bounds != [0, max_level]
        || overview.hex_radius.to_bits() != 1.0_f32.to_bits()
        || overview.level_height.to_bits() != 0.35_f32.to_bits()
        || overview.vertical_offset.to_bits() != 0.35_f32.to_bits()
        || overview.sea_level.to_bits() != 140.0_f32.to_bits()
    {
        return Err(
            "Northern overview identity, palette or geometry differs from its package/profile"
                .into(),
        );
    }
    if let Some(forest) = &overview.forest {
        if !grand {
            return Err("Forest companion is only supported for Grand".into());
        }
        forest
            .validate_catalog(&manifest.features)
            .map_err(|e| e.to_string())?;
    }
    let samples = overview
        .width
        .checked_mul(overview.height)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or("Northern overview dimensions overflow")?;
    if overview.width < 2
        || overview.height < 2
        || samples > 1_048_576
        || overview.player_spawn[0].abs() > extent_x
        || overview.player_spawn[2].abs() > extent_z
        || !(0.0..=max_height).contains(&overview.player_spawn[1])
        || overview.bed_heights.len() != samples
        || overview.surface_materials.len() != samples
        || !overview.spacing.is_finite()
        || !(0.5..=8.0).contains(&overview.spacing)
        || !overview
            .origin_xz
            .iter()
            .chain(&overview.player_spawn)
            .chain(overview.anchors.values().flatten())
            .all(|v| v.is_finite())
        || overview
            .bed_heights
            .iter()
            .any(|height| !height.is_finite() || !(0.0..=max_height).contains(height))
        || overview
            .surface_materials
            .iter()
            .any(|index| overview.materials.get(usize::from(*index)).is_none())
    {
        return Err(
            "Northern overview grid, palette index or observation position is invalid".into(),
        );
    }
    let [ox, oz] = overview.origin_xz;
    let width = u16::try_from(overview.width - 1)
        .map_err(|error| format!("Northern overview width exceeds its grid bound: {error}"))?;
    let height = u16::try_from(overview.height - 1)
        .map_err(|error| format!("Northern overview height exceeds its grid bound: {error}"))?;
    let end_x = ox + f32::from(width) * overview.spacing;
    let end_z = oz + f32::from(height) * overview.spacing;
    if ox > -extent_x + 3.0
        || oz > -extent_z + 5.0
        || end_x < extent_x - 3.0
        || end_z < extent_z - 5.0
        || !end_x.is_finite()
        || !end_z.is_finite()
        || world_hex(HexCoord::from_world(Vec3::from_array(
            overview.player_spawn,
        )))
        .checked_distance(WorldHex::new(0, 0))
        .map_err(|error| error.to_string())?
            > u64::from(radius)
    {
        return Err(
            "Northern overview does not cover the finite region or has an outside-world spawn"
                .into(),
        );
    }
    Ok(())
}

fn publish_overview(world: &mut World, overview: &NorthernOverview, generation: u64) {
    let width = overview.width;
    let height = overview.height;
    let water = overview
        .materials
        .iter()
        .find(|m| m.id == "water")
        .map_or([40, 95, 132, 255], |m| {
            [m.color[0], m.color[1], m.color[2], 255]
        });
    let rgba = overview
        .surface_materials
        .iter()
        .zip(&overview.bed_heights)
        .flat_map(|(index, bed)| {
            if *bed < overview.sea_level {
                return water;
            }
            overview
                .materials
                .get(usize::from(*index))
                .map_or([40, 95, 132, 255], |m| m.color)
        })
        .collect();
    let min = Vec2::from_array(overview.origin_xz);
    #[expect(clippy::cast_precision_loss, reason = "bounded overview dimensions")]
    let max = min
        + Vec2::new(
            width.saturating_sub(1) as f32,
            height.saturating_sub(1) as f32,
        ) * overview.spacing;
    world.insert_resource(ArenaOverview {
        generation,
        width,
        height,
        min,
        max,
        rgba,
    });
}
fn world_hex(c: HexCoord) -> WorldHex {
    WorldHex::new(i64::from(c.x()), i64::from(c.y()))
}
fn pair(c: ChunkId) -> Option<(i32, i32)> {
    Some((i32::try_from(c.q).ok()?, i32::try_from(c.r).ok()?))
}
fn local(c: WorldHex) -> Option<HexCoord> {
    Some(HexCoord::from_axial(
        i32::try_from(c.q).ok()?,
        i32::try_from(c.r).ok()?,
    ))
}
fn publish(state: &mut StreamedArena, view: &mut ArenaTerrainView, geometry: &ArenaVoxelGeometry) {
    let resident: BTreeSet<_> = state
        .runtime
        .resident_chunks()
        .map(|p| p.coordinate)
        .collect();
    let retired: Vec<_> = state
        .projected
        .keys()
        .filter(|c| !resident.contains(c))
        .copied()
        .collect();
    let changed: Vec<_> = state
        .runtime
        .resident_chunks()
        .filter(|p| {
            state.projected.get(&p.coordinate) != state.edits.revision(p.coordinate).as_ref()
        })
        .map(|p| p.coordinate)
        .collect();
    if retired.is_empty() && changed.is_empty() {
        return;
    }
    view.dirty_columns.clear();
    view.full_rebuild = state.projected.is_empty();
    for chunk in retired.into_iter().chain(changed.iter().copied()) {
        let origin = chunk.origin().unwrap_or_default();
        for q in 0..16 {
            for r in 0..16 {
                let Some(coord) = local(WorldHex::new(origin.q + q, origin.r + r)) else {
                    continue;
                };
                view.columns.remove(&coord);
                view.object_columns.remove(&coord);
                view.dirty_columns.insert(coord);
            }
        }
        view.liquids
            .retain(|run| world_hex(run.bottom.coord).chunk() != chunk);
        state.projected.remove(&chunk);
        if let Some(key) = pair(chunk) {
            if let Some(residency) = &mut view.residency {
                residency.ready.remove(&key);
            }
        }
    }
    // Material IDs use the arena's stable gameplay vocabulary, not the render palette order.
    let names: BTreeMap<_, _> = state
        .runtime
        .manifest()
        .materials
        .iter()
        .map(|m| (m.id.clone(), m.solid))
        .collect();
    for chunk in changed {
        let Some(product) = state.runtime.resident_chunk(chunk) else {
            continue;
        };
        for source in &product.package.columns {
            let Some(coord) = local(source.position) else {
                continue;
            };
            if !geometry.contains_column(coord) {
                continue;
            }
            if let Some(column) = state.edits.terrain_column(source.position) {
                for run in column.runs {
                    let span = ArenaSolidSpan {
                        bottom: TilePos::new(coord, run.bottom),
                        top_level: run.top - 1,
                        substance: state
                            .policies
                            .get(&run.material)
                            .copied()
                            .unwrap_or(SubstanceId::AIR),
                    };
                    if names.get(&run.material) == Some(&true) {
                        view.columns.entry(coord).or_default().push(span);
                    } else {
                        view.liquids.push(span);
                    }
                }
            }
            if let Some(column) = state.edits.object_column(source.position) {
                for run in column.runs {
                    view.object_columns
                        .entry(coord)
                        .or_default()
                        .push(ArenaSolidSpan {
                            bottom: TilePos::new(coord, run.bottom),
                            top_level: run.top - 1,
                            substance: state
                                .policies
                                .get(&run.material)
                                .copied()
                                .unwrap_or(SubstanceId::AIR),
                        });
                }
            }
        }
        state
            .projected
            .insert(chunk, state.edits.revision(chunk).unwrap_or(0));
        if let Some(key) = pair(chunk) {
            if let Some(residency) = &mut view.residency {
                residency.ready.insert(key);
            }
        }
    }
    view.liquids.sort_by_key(|span| span.bottom);
    view.static_spans = view
        .object_columns
        .values()
        .flatten()
        .map(|span| ArenaStaticSpan {
            bottom: span.bottom,
            top_level: span.top_level,
            blocks_movement: true,
            blocks_projectiles: true,
            blocks_sight: true,
        })
        .collect();
    view.revision = view.revision.wrapping_add(1);
}

// Rounding any cell in a 16×16 axial chunk to its centre can move the interest
// by sixteen hexes at the q/r corner. Preserve the intended sixteen-hex local
// actor envelope after quantization, including the diagonally previous chunk.
const ACTOR_INTEREST_RADIUS: u32 = 16 + 16;

fn interest_center(position: Vec3) -> WorldHex {
    let chunk = world_hex(HexCoord::from_world(position)).chunk();
    WorldHex::new(chunk.q * 16 + 8, chunk.r * 16 + 8)
}

fn pump(world: &mut World) {
    let started = Instant::now();
    let interest = *world.resource::<ArenaStreamInterest>();
    let geometry = *world.resource::<ArenaVoxelGeometry>();
    world.resource_scope(|world, mut state: Mut<StreamedArena>| {
        let center = interest_center(interest.position);
        let ahead = interest_center(interest.position + interest.velocity);
        let far = interest_center(interest.position + interest.velocity * 2.0);
        let mut actors: Vec<_> = world
            .resource::<ArenaActorStreamInterests>()
            .positions
            .iter()
            .filter(|p| p.is_finite())
            .map(|p| interest_center(*p))
            .collect();
        actors.sort();
        actors.dedup();
        actors.truncate(128);
        let mut key = vec![center, ahead, far];
        key.extend_from_slice(&actors);
        if state.interest_key.as_ref() != Some(&key) {
            let mut requests = vec![
                ResidencyRequest {
                    id: "player-body".into(),
                    center,
                    radius: 20,
                    retention_radius: 20,
                    priority: 255,
                },
                ResidencyRequest {
                    id: "terrain-detail".into(),
                    center,
                    // Match the ocean's stepped surface, retaining the existing
                    // 256 presentation / 512 source chunk caps.
                    radius: u32::from(crate::ocean::VOXEL_DETAIL_RADIUS) + 16,
                    retention_radius: u32::from(crate::ocean::VOXEL_DETAIL_RADIUS) + 48,
                    priority: 100,
                },
                ResidencyRequest {
                    id: "ahead".into(),
                    center: ahead,
                    radius: 32,
                    retention_radius: 32,
                    priority: 254,
                },
                ResidencyRequest {
                    id: "far".into(),
                    center: far,
                    radius: 32,
                    retention_radius: 32,
                    priority: 253,
                },
            ];
            requests.extend(
                actors
                    .iter()
                    .enumerate()
                    .map(|(i, center)| ResidencyRequest {
                        id: format!("actor-{i}"),
                        center: *center,
                        radius: ACTOR_INTEREST_RADIUS,
                        retention_radius: ACTOR_INTEREST_RADIUS,
                        priority: 252,
                    }),
            );
            // A distant respawn request must not fail merely because the
            // renderer is retaining an old detail fringe. Critical authority
            // remains bounded; trim disposable retention before refusing it.
            let result = state
                .runtime
                .set_interests(requests.clone())
                .or_else(|_| {
                    for request in &mut requests {
                        request.retention_radius = request.radius;
                    }
                    state.runtime.set_interests(requests.clone())
                })
                .or_else(|_| {
                    requests.retain(|request| request.id != "terrain-detail");
                    state.runtime.set_interests(requests)
                });
            match result {
                Ok(()) => {
                    state.interest_key = Some(key);
                    state.failure = None;
                }
                Err(e) => state.failure = Some(e.to_string()),
            }
        }
        let update = state.runtime.pump();
        if let Some(failure) = update.failures.first() {
            state.failure = Some(failure.error.to_string());
        }
        let StreamedArena { runtime, edits, .. } = &mut *state;
        edits.sync_residency(runtime);
        let mut view = world.resource_mut::<ArenaTerrainView>();
        publish(&mut state, &mut view, &geometry);
        state.peak_resident = state
            .peak_resident
            .max(state.runtime.resident_chunks().count());
        state.publication_ms = started.elapsed().as_secs_f64() * 1000.0;
    });
}

fn apply(world: &mut World) {
    let reset = *world.resource::<ArenaReset>();
    if world.resource::<StreamedArena>().generation != reset.generation {
        let content = match super::load_content() {
            Ok(c) => c,
            Err(e) => {
                error!("{e}");
                return;
            }
        };
        render::clear(world);
        world.resource_mut::<ArenaInbox>().edits.clear();
        world.resource_mut::<ArenaInbox>().impacts.clear();
        if let Err(e) = initialize(world, content) {
            error!("Northern restart: {e}");
        }
        return;
    }
    let (direct, impacts) = {
        let mut inbox = world.resource_mut::<ArenaInbox>();
        (
            std::mem::take(&mut inbox.edits),
            std::mem::take(&mut inbox.impacts),
        )
    };
    if direct.is_empty() && impacts.is_empty() {
        return;
    }
    let Some(mut view) = world.remove_resource::<ArenaTerrainView>() else {
        return;
    };
    let mut outcomes = Vec::new();
    world.resource_scope(|world, mut state: Mut<StreamedArena>| {
        let geometry = *world.resource::<ArenaVoxelGeometry>();
        let mut requests = Vec::new();
        let mut directly_changed = Vec::new();
        for edit in direct {
            let pos = edit.pos();
            if !ready_voxel(&view, geometry, pos) {
                continue;
            }
            let at = VoxelPosition {
                column: world_hex(pos.coord),
                level: pos.level,
            };
            if state.edits.terrain_at(at).is_some_and(|name| {
                state
                    .runtime
                    .manifest()
                    .materials
                    .iter()
                    .any(|m| m.id == name && !m.solid)
            }) {
                continue;
            }
            let material = match edit {
                TerrainEdit::Clear { .. } => None,
                TerrainEdit::Set { substance, .. } => {
                    let Some(name) = world
                        .resource::<hex_assets::SubstanceTable>()
                        .name(substance)
                    else {
                        continue;
                    };
                    if !state
                        .runtime
                        .manifest()
                        .materials
                        .iter()
                        .any(|m| m.id == name && m.solid)
                    {
                        continue;
                    }
                    Some(name.to_owned())
                }
            };
            requests.push(VoxelEdit {
                position: at,
                material,
            });
            directly_changed.push(pos);
        }
        if !requests.is_empty() {
            match commit(&mut state, requests) {
                Ok(()) => {
                    world.resource_scope(
                        |world, mut damage: Mut<crate::terrain_damage::TerrainDamageState>| {
                            let mut damaged = world.resource_mut::<DamagedVoxels>();
                            for position in directly_changed {
                                damage.forget_voxel(position, &mut damaged);
                            }
                        },
                    );
                    publish(&mut state, &mut view, &geometry);
                }
                Err(error) => {
                    error!("Northern direct transaction: {error}");
                    state.failure = Some(error);
                }
            }
        }
        world.resource_scope(
            |world, mut damage: Mut<crate::terrain_damage::TerrainDamageState>| {
                world.resource_scope(|world, mut damaged: Mut<DamagedVoxels>| {
                    for impact in impacts {
                        let rejection = if !damage.consume_batch(impact.batch) {
                            Some(TerrainImpactRejection::ReusedBatch)
                        } else if let Some(reason) = impact.structural_rejection() {
                            Some(reason)
                        } else if impact.kind.element().is_some_and(|element| {
                            world
                                .resource::<hex_assets::ElementCatalog>()
                                .name(element)
                                .is_none()
                        }) {
                            Some(TerrainImpactRejection::UnknownElement)
                        } else if impact.volume.len()
                            > hex_world_contracts::MAX_EDITS_PER_TRANSACTION
                            || impact
                                .volume
                                .iter()
                                .any(|p| !ready_voxel(&view, geometry, *p))
                        {
                            Some(TerrainImpactRejection::TerrainUnavailable)
                        } else {
                            None
                        };
                        if let Some(reason) = rejection {
                            outcomes.push(TerrainImpactOutcome {
                                batch: impact.batch,
                                result: TerrainImpactResult::Rejected(reason),
                            });
                            continue;
                        }
                        // Stage only this bounded impact's health. A failed voxel transaction
                        // must leave real HP and damage outcomes unchanged.
                        let materials: BTreeMap<_, _> = impact
                            .volume
                            .iter()
                            .map(|p| (*p, view.solid_at(*p).unwrap_or(SubstanceId::AIR)))
                            .collect();
                        let mut staged_damage =
                            crate::terrain_damage::TerrainDamageState::default();
                        let mut staged_projection = DamagedVoxels::default();
                        staged_damage.restore(
                            impact
                                .volume
                                .iter()
                                .filter_map(|p| damaged.get(*p).map(|health| (*p, health))),
                            &mut staged_projection,
                        );
                        let staged = staged_damage.apply_finite(
                            impact.clone(),
                            |p| materials.get(&p).copied().unwrap_or(SubstanceId::AIR),
                            world.resource::<hex_assets::SubstanceTable>(),
                            world.resource::<hex_assets::TerrainDamageTable>(),
                            &mut staged_projection,
                            |_| false,
                        );
                        let requests = staged
                            .destroyed
                            .into_iter()
                            .map(|p| VoxelEdit {
                                position: VoxelPosition {
                                    column: world_hex(p.coord),
                                    level: p.level,
                                },
                                material: None,
                            })
                            .collect();
                        if let Err(error) = commit(&mut state, requests) {
                            error!("Northern impact transaction: {error}");
                            state.failure = Some(error);
                            outcomes.push(TerrainImpactOutcome {
                                batch: impact.batch,
                                result: TerrainImpactResult::Rejected(
                                    TerrainImpactRejection::TerrainUnavailable,
                                ),
                            });
                            continue;
                        }
                        let applied = damage.apply_finite(
                            impact,
                            |p| materials.get(&p).copied().unwrap_or(SubstanceId::AIR),
                            world.resource::<hex_assets::SubstanceTable>(),
                            world.resource::<hex_assets::TerrainDamageTable>(),
                            &mut damaged,
                            |_| false,
                        );
                        outcomes.push(applied.outcome);
                        // The next impact observes preceding committed removals, never a stale body.
                        publish(&mut state, &mut view, &geometry);
                    }
                });
            },
        );
    });
    world.insert_resource(view);
    world
        .resource_mut::<Messages<TerrainImpactOutcome>>()
        .write_batch(outcomes);
}

fn ready_voxel(view: &ArenaTerrainView, geometry: ArenaVoxelGeometry, pos: TilePos) -> bool {
    geometry.contains_column(pos.coord)
        && (geometry.min_level..=geometry.max_level).contains(&pos.level)
        && view
            .residency
            .as_ref()
            .is_some_and(|r| r.at(pos.coord, geometry) == ArenaAvailability::Ready)
}

fn commit(state: &mut StreamedArena, edits: Vec<VoxelEdit>) -> Result<(), String> {
    if edits.is_empty() {
        return Ok(());
    }
    let edits: Vec<_> = edits
        .into_iter()
        .map(|e| (e.position, e))
        .collect::<BTreeMap<_, _>>()
        .into_values()
        .collect();
    let expected_revisions = edits
        .iter()
        .map(|e| {
            let chunk = e.position.column.chunk();
            state
                .edits
                .revision(chunk)
                .map(|revision| (chunk, revision))
                .ok_or("Northern transaction chunk is unavailable")
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let next = state
        .next_transaction
        .checked_add(1)
        .ok_or("Northern transaction counter exhausted")?;
    state
        .edits
        .apply_transaction(&WorldEditTransaction {
            id: format!("northern-{}-{next}", state.generation),
            expected_revisions,
            edits,
        })
        .map_err(|error| error.to_string())?;
    state.next_transaction = next;
    Ok(())
}
