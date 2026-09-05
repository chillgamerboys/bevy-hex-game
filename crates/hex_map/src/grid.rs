//! Builds the voxel world, publishes logical tile entities, and batches terrain draws.
//!
//! Storage and generation are private to `hex_map`; rendered terrain reaches other
//! crates as entities carrying [`HexTile`](hex_core::HexTile),
//! [`HexCoord`](hex_core::HexCoord), a surface [`TilePos`](hex_core::TilePos),
//! [`RunBottom`](hex_core::RunBottom), [`HexSpan`](hex_core::HexSpan),
//! [`SubstanceId`](hex_core::SubstanceId), and [`Headroom`](hex_core::Headroom).
//! The substance table itself is shared through `hex_assets` because gameplay also
//! reads its behavior flags.
//!
//! Logical runs remain independent of the combined meshes that draw them. Keeping
//! that boundary narrow is what lets the map rebuild one render chunk without
//! touching gameplay. A richer map means producing different voxels in the terrain
//! builder; it does not change what a tile *is* to anyone else.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::Indices;
use bevy::picking::Pickable;
use bevy::render::render_resource::PrimitiveTopology;
use bevy::{ecs::system::SystemParam, prelude::*};

use hex_assets::{
    to_color, ElementCatalog, GameAssets, HexObjectRotation, ObjectBlueprint, RuntimeArtCatalog,
    SubstanceTable, TerrainDamageFile, TerrainDamageTable,
};
use hex_core::{
    AuthoritativeSystems, BiomeRegions, CutawayOccluder, DamagedVoxels, GameplayLight,
    GameplaySetup, GameplaySetupFailure, Headroom, HexCoord, HexGrid, HexSpan, HexTile,
    InteriorRegionId, InteriorRegions, MapAnchorId, MapAnchors, MapObservationAnchors, MapViewHint,
    PerceptionSystems, PresentationOcclusion, ResolvedMapSeed, ReviewCrystalLightProfile,
    ReviewEdgeTreatment, ReviewMaterialTreatment, RunBottom, Screen, SimulationRole,
    SpecialMovementRegions, SubstanceId, TerrainChunkRoot, TerrainEdit, TerrainImpact,
    TerrainImpactDisposition, TerrainImpactOutcome, TerrainImpactRejection, TerrainImpactResult,
    TerrainPickRun, TerrainReady, TerrainRenderBatch, TerrainSystems, TilePos, TraversalBlockers,
    TraversalProfile, TreeOccluder, MAX_TERRAIN_PICK_RUNS_PER_BATCH,
};
use hex_multiplayer::AuthorityBoundary;

use crate::crystal_render::{self, CrystalPresentationError};
use crate::feature_render::{self, FeaturePresentationError, GeneratedFeatureRoot};
use crate::liquid_render::{self, LiquidMaterial, LiquidPresentationError, LiquidVisualTime};
use crate::procedural;
use crate::procedural_v2;
use crate::procedural_v3;
use crate::procedural_v3::MapPresentationProjection;
use crate::settings::{MapSettings, TerrainSettings};
use crate::terrain::{build_non_procedural_map, TerrainPalette};
use crate::terrain_damage::TerrainDamageState;
use crate::voxel::{runs, terrain_chunk_coord, Column, SubstanceRun, TerrainChunkCoord, VoxelMap};
use crate::world_snapshot::{
    apply_world_delta_v1, export_from_parts, prepare_world_snapshot_v1,
    CampaignWorldRestoreOutcomeV2, CampaignWorldRestoreRefusalV2, CampaignWorldRestoreResultV2,
    CurrentWorldSnapshotV1, PendingCampaignWorldSnapshotV2, PreparedWorldSnapshotV1,
    WorldExportParts, WorldReplicationOutcomeV1, WorldReplicationRefusalV1,
    WorldReplicationRequestV1, WorldReplicationResultV1, WorldReplicationStateV1,
};
use crate::{
    CavesReportMetrics, DeepForestReportMetrics, ForestReportMetrics, FortReportMetrics,
    GenerationReport, MacroMetrics, MountainRangeMetrics, OceanArchipelagoMetrics,
    PrairieReportMetrics, ProceduralRecipeMetrics, Ring19Metrics, Ring7Metrics,
    SandyIsletsReportMetrics, VolcanoReportMetrics, WaterfallReportMetrics,
    WoodedIslandReportMetrics,
};

/// One claimed impact decision retained in exact incoming message order.
#[derive(Debug)]
enum PendingTerrainImpact {
    Apply(TerrainImpact),
    Reject {
        batch: hex_core::TerrainBatchId,
        reason: TerrainImpactRejection,
    },
}

/// Claimed impact decisions collected before direct edits are resolved.
#[derive(Resource, Debug, Default)]
struct PendingTerrainImpacts(Vec<PendingTerrainImpact>);

/// Direct edits claimed before the pausable mutation phase.
///
/// Messages live for only a bounded number of updates. Keeping an owned queue lets
/// a cast announced immediately before pausing survive until gameplay resumes
/// without changing the world against a stale perception frame.
#[derive(Resource, Debug, Default)]
struct PendingTerrainEdits(Vec<TerrainEdit>);

/// Map-owned truth awaiting publication into the reconnect cache.
///
/// Initial construction needs a complete export. Accepted edits retain their exact
/// changed coordinates so the existing cache can reproject only local consequences.
#[derive(Resource, Debug, Default)]
struct WorldSnapshotDirty {
    full_refresh: bool,
    changed_coords: BTreeSet<HexCoord>,
}

impl WorldSnapshotDirty {
    fn mark_full(&mut self) {
        self.full_refresh = true;
        self.changed_coords.clear();
    }

    fn mark_changed(&mut self, changed_coords: BTreeSet<HexCoord>) {
        if !self.full_refresh {
            self.changed_coords.extend(changed_coords);
        }
    }

    fn is_dirty(&self) -> bool {
        self.full_refresh || !self.changed_coords.is_empty()
    }

    fn clear(&mut self) {
        self.full_refresh = false;
        self.changed_coords.clear();
    }
}

/// A validated resource candidate waiting for the ordinary grid publication pass.
#[derive(Resource, Debug)]
struct PendingSnapshotGridBuild {
    ordered_results: Vec<WorldReplicationResultV1>,
    previous_sequence: Option<hex_multiplayer::AuthoritySequence>,
}

/// A validated Campaign world waiting for the terrain publication set to finish.
#[derive(Resource, Debug, Clone, Copy)]
struct PendingCampaignWorldPublication {
    public_fingerprint: hex_multiplayer::PublicWorldFingerprint,
}

/// Complete validated map resources waiting for their first grid publication.
///
/// This private marker separates semantic construction from presentation readiness.
/// [`TerrainReady`] is published only after [`build_grid`] has created every resident
/// chunk root and global presentation projection and the canonical world snapshot has
/// been published without error.
#[derive(Resource, Debug, Clone, Copy)]
struct PendingTerrainPublication;

/// A complete grid waiting for canonical snapshot publication and final readiness.
///
/// Keeping this distinct from [`PendingTerrainPublication`] prevents any observer from
/// seeing [`TerrainReady`] between presentation construction and snapshot publication.
#[derive(Resource, Debug, Clone, Copy)]
struct PendingInitialSnapshotPublication;

/// Presentation-free lifecycle owner for one chunk's logical terrain runs.
///
/// Logical [`HexTile`] entities are authoritative ECS facts, not scene nodes. Keeping
/// them below an inert owner preserves recursive chunk teardown without placing every
/// run in Bevy's transform or visibility propagation trees.
#[derive(Component, Debug)]
struct LogicalTerrainRuns;

const MAX_WORLD_REPLICATION_REQUESTS_PER_UPDATE: usize = 64;

/// Registers world construction and tile spawning.
pub fn plugin(app: &mut App) {
    liquid_render::plugin(app);
    app.register_type::<HexCoord>()
        .register_type::<HexGrid>()
        .register_type::<TerrainChunkRoot>()
        .register_type::<HexSpan>()
        .register_type::<HexTile>()
        .register_type::<SubstanceId>()
        .register_type::<TilePos>()
        .register_type::<RunBottom>()
        .register_type::<Headroom>()
        .register_type::<InteriorRegionId>()
        .register_type::<CutawayOccluder>()
        .register_type::<TreeOccluder>()
        .register_type::<PresentationOcclusion>()
        .register_type::<GameplayLight>()
        .register_type::<TerrainReady>()
        .register_type::<DamagedVoxels>()
        .register_type::<TerrainImpact>()
        .register_type::<TerrainImpactOutcome>()
        .register_type::<GenerationReport>()
        .register_type::<ProceduralRecipeMetrics>()
        .register_type::<WaterfallReportMetrics>()
        .register_type::<ForestReportMetrics>()
        .register_type::<FortReportMetrics>()
        .register_type::<CavesReportMetrics>()
        .register_type::<Ring7Metrics>()
        .register_type::<VolcanoReportMetrics>()
        .register_type::<DeepForestReportMetrics>()
        .register_type::<PrairieReportMetrics>()
        .register_type::<Ring19Metrics>()
        .register_type::<MacroMetrics>()
        .register_type::<MountainRangeMetrics>()
        .register_type::<SandyIsletsReportMetrics>()
        .register_type::<WoodedIslandReportMetrics>()
        .register_type::<OceanArchipelagoMetrics>()
        .register_type::<crate::procedural::GrandV3Metrics>()
        .init_resource::<MaterialCache>()
        .init_resource::<TerrainMeshCache>()
        .init_resource::<DamagedVoxels>()
        .init_resource::<TerrainDamageState>()
        .init_resource::<PendingTerrainEdits>()
        .init_resource::<PendingTerrainImpacts>()
        .init_resource::<WorldSnapshotDirty>()
        .init_resource::<WorldReplicationStateV1>()
        .init_resource::<AuthorityBoundary>()
        .init_resource::<SimulationRole>()
        .add_message::<TerrainEdit>()
        .add_message::<TerrainImpact>()
        .add_message::<TerrainImpactOutcome>()
        .add_message::<WorldReplicationRequestV1>()
        .add_message::<WorldReplicationResultV1>()
        .configure_sets(
            Update,
            AuthoritativeSystems.run_if(resource_equals(SimulationRole::Authority)),
        )
        // Only ApplyWorld has participants today. The empty downstream sets reserve
        // the cross-crate protocol without moving gameplay behavior in this change.
        .configure_sets(
            Update,
            (
                TerrainSystems::ApplyWorld,
                TerrainSystems::RefreshProjections,
                TerrainSystems::ReconcileActors,
                TerrainSystems::ConsumeOutcomes,
            )
                .chain()
                .before(PerceptionSystems::ResolveIllumination),
        )
        // Split across two sets rather than chained locally: `hex_units` spawns
        // the player into `Actors`, which must come after the tiles here, and a
        // local `.chain()` cannot order systems in another crate.
        .add_systems(
            OnEnter(Screen::Gameplay),
            generate_world.in_set(GameplaySetup::Resources),
        )
        .add_systems(
            OnEnter(Screen::Gameplay),
            (
                spawn_grid.run_if(resource_exists::<PendingTerrainPublication>),
                publish_current_world_snapshot
                    .run_if(resource_exists::<PendingInitialSnapshotPublication>),
                finalize_initial_terrain_publication
                    .run_if(resource_exists::<PendingInitialSnapshotPublication>),
            )
                .chain()
                .in_set(GameplaySetup::Terrain),
        )
        .add_systems(
            Update,
            (collect_terrain_edits, collect_terrain_impacts)
                .in_set(TerrainSystems::ApplyWorld)
                .in_set(AuthoritativeSystems)
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(
            Update,
            (
                apply_terrain_changes.run_if(terrain_world_available),
                reject_pending_impacts_without_world.run_if(not(terrain_world_available)),
            )
                .chain()
                .in_set(TerrainSystems::ApplyWorld)
                .in_set(AuthoritativeSystems)
                .in_set(hex_core::PausableSystems)
                .after(collect_terrain_edits)
                .after(collect_terrain_impacts)
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(
            Update,
            publish_current_world_snapshot
                .after(apply_terrain_changes)
                .after(reject_pending_impacts_without_world)
                .run_if(in_state(Screen::Gameplay))
                .run_if(resource_exists::<TerrainReady>),
        )
        .add_systems(
            Update,
            (
                apply_world_replication_requests,
                spawn_imported_snapshot_grid,
            )
                .chain()
                .in_set(TerrainSystems::ApplyWorld)
                .after(publish_current_world_snapshot)
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(OnExit(Screen::Gameplay), teardown_map);
}

fn terrain_world_available(
    terrain_ready: Option<Res<TerrainReady>>,
    map: Option<Res<VoxelMap>>,
) -> bool {
    terrain_ready.is_some() && map.is_some()
}

fn generate_world(
    mut commands: Commands,
    settings: Res<MapSettings>,
    table: Res<SubstanceTable>,
    art_catalog: Option<Res<RuntimeArtCatalog>>,
    mut pending_campaign: Option<ResMut<PendingCampaignWorldSnapshotV2>>,
    resolved_seed: Option<Res<ResolvedMapSeed>>,
    mut damage_state: ResMut<TerrainDamageState>,
    mut damaged_voxels: ResMut<DamagedVoxels>,
    mut pending_edits: ResMut<PendingTerrainEdits>,
    mut pending_impacts: ResMut<PendingTerrainImpacts>,
    mut edits: ResMut<Messages<TerrainEdit>>,
    mut impacts: ResMut<Messages<TerrainImpact>>,
    mut outcomes: ResMut<Messages<TerrainImpactOutcome>>,
    mut snapshot_dirty: ResMut<WorldSnapshotDirty>,
    mut replication_state: ResMut<WorldReplicationStateV1>,
) {
    damage_state.reset(&mut damaged_voxels);
    pending_edits.0.clear();
    pending_impacts.0.clear();
    edits.clear();
    impacts.clear();
    outcomes.clear();
    snapshot_dirty.mark_full();
    replication_state.set_last_applied_sequence(None);
    commands.remove_resource::<GameplaySetupFailure>();
    commands.remove_resource::<TerrainReady>();
    commands.remove_resource::<VoxelMap>();
    commands.remove_resource::<GenerationReport>();
    commands.remove_resource::<MapPresentationProjection>();
    liquid_render::clear_material_cache(&mut commands);
    commands.remove_resource::<MapAnchors>();
    commands.remove_resource::<MapObservationAnchors>();
    commands.remove_resource::<SpecialMovementRegions>();
    commands.remove_resource::<InteriorRegions>();
    commands.remove_resource::<TraversalBlockers>();
    commands.remove_resource::<BiomeRegions>();
    commands.remove_resource::<MapViewHint>();
    commands.remove_resource::<CurrentWorldSnapshotV1>();
    commands.remove_resource::<PendingSnapshotGridBuild>();
    commands.remove_resource::<PendingCampaignWorldPublication>();
    commands.remove_resource::<PendingTerrainPublication>();
    commands.remove_resource::<PendingInitialSnapshotPublication>();
    commands.remove_resource::<CampaignWorldRestoreResultV2>();

    if let Some(pending_campaign) = pending_campaign.as_deref_mut() {
        let candidate = pending_campaign.take();
        commands.remove_resource::<PendingCampaignWorldSnapshotV2>();
        let Some(candidate) = candidate else {
            refuse_campaign_world_restore(
                &mut commands,
                CampaignWorldRestoreRefusalV2::MissingSnapshot,
            );
            return;
        };
        let prepared =
            match prepare_world_snapshot_v1(candidate, &table, &settings, art_catalog.as_deref()) {
                Ok(prepared) => prepared,
                Err(error) => {
                    refuse_campaign_world_restore(
                        &mut commands,
                        CampaignWorldRestoreRefusalV2::InvalidSnapshot(error),
                    );
                    return;
                }
            };
        stage_campaign_world_restore(
            &mut commands,
            prepared,
            &mut damage_state,
            &mut damaged_voxels,
            &mut snapshot_dirty,
        );
        return;
    }

    let palette = match TerrainPalette::for_terrain(&table, &settings.terrain) {
        Ok(palette) => palette,
        Err(error) => {
            error!("cannot build terrain: {error}");
            commands.insert_resource(GameplaySetupFailure::new(format!(
                "The selected terrain cannot be built: {error}"
            )));
            return;
        }
    };

    let TerrainSettings::Procedural(procedural_settings) = &settings.terrain else {
        let Some(map) = build_non_procedural_map(&settings, &palette) else {
            error!("non-procedural terrain did not produce an authored map");
            commands.insert_resource(GameplaySetupFailure::new(
                "The selected authored terrain did not produce a map.",
            ));
            return;
        };
        commands.insert_resource(map);
        commands.insert_resource(MapAnchors::new());
        commands.insert_resource(MapObservationAnchors::new());
        commands.insert_resource(SpecialMovementRegions::new());
        commands.insert_resource(InteriorRegions::new());
        commands.insert_resource(PendingTerrainPublication);
        return;
    };

    let Some(seed) = resolved_seed else {
        error!("procedural terrain requires a resolved scenario seed");
        commands.insert_resource(GameplaySetupFailure::new(
            "The selected procedural terrain has no resolved generation seed.",
        ));
        return;
    };
    match procedural_settings {
        crate::settings::ProceduralSettings::V1(v1) => {
            let generated = procedural::build(
                settings.grid_radius,
                v1,
                seed.0,
                &palette,
                TraversalProfile::WALKER,
                &|substance| table.is_solid(substance),
            );
            let anchors: MapAnchors = generated
                .anchors
                .iter()
                .map(|(name, pos)| (MapAnchorId::from(name), pos))
                .collect();
            if generated.validated {
                info!(
                    "generated procedural map seed={} candidate={:?} fingerprint={} in {}us",
                    generated.report.seed,
                    generated.report.selected_candidate,
                    generated.report.map_fingerprint,
                    generated.report.elapsed_micros
                );
                commands.insert_resource(generated.special_regions);
                commands.insert_resource(InteriorRegions::new());
                commands.insert_resource(PendingTerrainPublication);
            } else {
                error!(
                    "procedural map and canonical fallback failed validation: {:?}",
                    generated.report.notes
                );
                commands.insert_resource(GameplaySetupFailure::new(
                    "Procedural generation and its canonical fallback both failed validation.",
                ));
            }
            commands.insert_resource(generated.map);
            commands.insert_resource(anchors);
            commands.insert_resource(MapObservationAnchors::new());
            commands.insert_resource(generated.report);
        }
        crate::settings::ProceduralSettings::V2(v2) => {
            let generated = match procedural_v2::build(
                settings.grid_radius,
                settings.level_height,
                v2,
                seed.0,
                &palette,
                &|substance| table.is_solid(substance),
            ) {
                Ok(generated) => generated,
                Err(error) => {
                    error!("cannot build procedural V2 terrain: {error}");
                    commands.insert_resource(GameplaySetupFailure::new(format!(
                        "The selected procedural terrain cannot be built: {error}."
                    )));
                    return;
                }
            };
            info!(
                "generated procedural V2 map seed={} candidate={:?} fingerprint={} in {}us",
                generated.report.seed,
                generated.report.selected_candidate,
                generated.report.map_fingerprint,
                generated.report.elapsed_micros
            );
            commands.insert_resource(generated.map);
            commands.insert_resource(generated.anchors);
            commands.insert_resource(MapObservationAnchors::new());
            commands.insert_resource(generated.special_regions);
            commands.insert_resource(generated.interiors);
            commands.insert_resource(generated.view_hint);
            commands.insert_resource(generated.report);
            commands.insert_resource(PendingTerrainPublication);
        }
        crate::settings::ProceduralSettings::V3(v3) => {
            let generated = match procedural_v3::build(
                settings.grid_radius,
                settings.level_height,
                v3,
                seed.0,
                &palette,
                &|substance| table.is_solid(substance),
                art_catalog.as_deref(),
            ) {
                Ok(generated) => generated,
                Err(error) => {
                    error!("cannot build procedural V3 terrain: {error}");
                    commands.insert_resource(GameplaySetupFailure::new(format!(
                        "The selected procedural terrain cannot be built: {error}."
                    )));
                    return;
                }
            };
            info!(
                "generated procedural V3 map seed={} candidate={:?} fingerprint={} in {}us",
                generated.report.seed,
                generated.report.selected_candidate,
                generated.report.map_fingerprint,
                generated.report.elapsed_micros
            );
            commands.insert_resource(generated.map);
            commands.insert_resource(generated.anchors);
            commands.insert_resource(generated.observation_anchors);
            commands.insert_resource(generated.special_regions);
            commands.insert_resource(generated.interiors);
            commands.insert_resource(generated.blockers);
            commands.insert_resource(generated.biome_regions);
            commands.insert_resource(generated.view_hint);
            commands.insert_resource(generated.presentation);
            commands.insert_resource(generated.report);
            commands.insert_resource(PendingTerrainPublication);
        }
    }
}

fn stage_campaign_world_restore(
    commands: &mut Commands,
    prepared: PreparedWorldSnapshotV1,
    damage_state: &mut TerrainDamageState,
    damaged_voxels: &mut DamagedVoxels,
    snapshot_dirty: &mut WorldSnapshotDirty,
) {
    let PreparedWorldSnapshotV1 {
        snapshot,
        map,
        damage,
        anchors,
        interiors,
        special_regions,
        biome_regions,
        blockers,
        view_hint,
        presentation,
    } = prepared;
    let public_fingerprint = snapshot.public_fingerprint;

    damage_state.restore(damage, damaged_voxels);
    snapshot_dirty.clear();
    commands.insert_resource(map);
    commands.insert_resource(anchors);
    commands.insert_resource(MapObservationAnchors::new());
    commands.insert_resource(interiors);
    commands.insert_resource(special_regions);
    commands.insert_resource(biome_regions);
    commands.insert_resource(blockers);
    if let Some(view_hint) = view_hint {
        commands.insert_resource(view_hint);
    }
    if let Some(presentation) = presentation {
        commands.insert_resource(presentation);
    }
    commands.insert_resource(CurrentWorldSnapshotV1::new(snapshot));
    commands.insert_resource(PendingCampaignWorldPublication { public_fingerprint });
    commands.insert_resource(PendingTerrainPublication);
}

fn refuse_campaign_world_restore(commands: &mut Commands, reason: CampaignWorldRestoreRefusalV2) {
    let description = match &reason {
        CampaignWorldRestoreRefusalV2::MissingSnapshot => {
            "the pending Campaign world was already consumed".to_owned()
        }
        CampaignWorldRestoreRefusalV2::InvalidSnapshot(error) => error.to_string(),
        CampaignWorldRestoreRefusalV2::PresentationFailed(error) => error.clone(),
    };
    error!("cannot restore Campaign world: {description}");
    commands.insert_resource(CampaignWorldRestoreResultV2 {
        outcome: CampaignWorldRestoreOutcomeV2::Refused(reason),
    });
    commands.insert_resource(GameplaySetupFailure::new(format!(
        "The saved Campaign world is incompatible: {description}."
    )));
}

fn teardown_map(
    mut commands: Commands,
    grids: Query<Entity, With<HexGrid>>,
    mut damage_state: ResMut<TerrainDamageState>,
    mut damaged_voxels: ResMut<DamagedVoxels>,
    mut pending_edits: ResMut<PendingTerrainEdits>,
    mut pending_impacts: ResMut<PendingTerrainImpacts>,
    mut edits: ResMut<Messages<TerrainEdit>>,
    mut impacts: ResMut<Messages<TerrainImpact>>,
    mut outcomes: ResMut<Messages<TerrainImpactOutcome>>,
    mut snapshot_dirty: ResMut<WorldSnapshotDirty>,
    mut replication_state: ResMut<WorldReplicationStateV1>,
    mut terrain_meshes: ResMut<TerrainMeshCache>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    for entity in &grids {
        commands.entity(entity).despawn();
    }
    damage_state.reset(&mut damaged_voxels);
    pending_edits.0.clear();
    pending_impacts.0.clear();
    edits.clear();
    impacts.clear();
    outcomes.clear();
    snapshot_dirty.clear();
    replication_state.set_last_applied_sequence(None);
    commands.remove_resource::<VoxelMap>();
    commands.remove_resource::<MapAnchors>();
    commands.remove_resource::<MapObservationAnchors>();
    commands.remove_resource::<SpecialMovementRegions>();
    commands.remove_resource::<InteriorRegions>();
    commands.remove_resource::<TraversalBlockers>();
    commands.remove_resource::<BiomeRegions>();
    commands.remove_resource::<MapViewHint>();
    commands.remove_resource::<GenerationReport>();
    commands.remove_resource::<MapPresentationProjection>();
    commands.remove_resource::<CurrentWorldSnapshotV1>();
    commands.remove_resource::<PendingSnapshotGridBuild>();
    commands.remove_resource::<PendingCampaignWorldPublication>();
    commands.remove_resource::<PendingTerrainPublication>();
    commands.remove_resource::<PendingInitialSnapshotPublication>();
    commands.remove_resource::<CampaignWorldRestoreResultV2>();
    liquid_render::clear_material_cache(&mut commands);
    terrain_meshes.reset_for_world(&mut meshes);
    commands.remove_resource::<TerrainReady>();
}

/// Publishes one lightweight logical entity per contiguous run of substance and
/// bounded combined render meshes per resident chunk.
///
/// Logical run entities retain gameplay's exact public tuple. They deliberately carry
/// no scene transform, visibility, picking, mesh, or material components: rendering
/// and traversing hundreds of thousands of independent scene entities was the
/// radius-187 bottleneck. Chunk children combine those prisms by substance and
/// cutaway ownership while [`TerrainRenderBatch`] maps pointer hits back to the same
/// logical run entities.
fn spawn_grid(
    mut commands: Commands,
    assets: Res<GameAssets>,
    mut presentation_assets: MapPresentationAssets,
    map: Res<VoxelMap>,
    table: Res<SubstanceTable>,
    settings: Res<MapSettings>,
    liquid_visual_time: Res<LiquidVisualTime>,
    interiors: Option<Res<InteriorRegions>>,
    presentation: Option<Res<MapPresentationProjection>>,
    art_catalog: Option<Res<RuntimeArtCatalog>>,
    material_treatment: Option<Res<ReviewMaterialTreatment>>,
    edge_treatment: Option<Res<ReviewEdgeTreatment>>,
    crystal_light_profile: Option<Res<ReviewCrystalLightProfile>>,
    pending_campaign: Option<Res<PendingCampaignWorldPublication>>,
    mut damage_state: ResMut<TerrainDamageState>,
    mut damaged_voxels: ResMut<DamagedVoxels>,
) {
    let built = build_grid(
        &mut commands,
        &assets,
        &mut presentation_assets.terrain_materials,
        &mut presentation_assets.terrain_meshes,
        &mut presentation_assets.materials,
        &mut presentation_assets.meshes,
        &mut presentation_assets.liquid_materials,
        &map,
        &table,
        &settings,
        liquid_visual_time.phase_seconds(),
        interiors.as_deref(),
        presentation.as_deref(),
        art_catalog.as_deref(),
        material_treatment.as_deref().copied().unwrap_or_default(),
        edge_treatment.as_deref().copied().unwrap_or_default(),
        crystal_light_profile
            .as_deref()
            .copied()
            .unwrap_or_default(),
    );
    match built {
        Ok(()) => {
            commands.remove_resource::<PendingTerrainPublication>();
            commands.insert_resource(PendingInitialSnapshotPublication);
        }
        Err(error) => {
            commands.remove_resource::<PendingTerrainPublication>();
            commands.remove_resource::<PendingInitialSnapshotPublication>();
            fail_presentation_setup(&mut commands, &error);
            if pending_campaign.is_some() {
                discard_staged_campaign_world(
                    &mut commands,
                    &mut damage_state,
                    &mut damaged_voxels,
                );
                refuse_campaign_world_restore(
                    &mut commands,
                    CampaignWorldRestoreRefusalV2::PresentationFailed(error.to_string()),
                );
                commands.remove_resource::<PendingCampaignWorldPublication>();
            }
        }
    }
}

/// Publishes initial terrain readiness only after the canonical snapshot exists.
///
/// This system is chained after [`publish_current_world_snapshot`]. Deferred commands
/// from that system are applied before this one runs, so a missing snapshot is an exact
/// setup failure and no downstream gameplay system can observe partial readiness.
fn finalize_initial_terrain_publication(
    mut commands: Commands,
    current: Option<Res<CurrentWorldSnapshotV1>>,
    pending_campaign: Option<Res<PendingCampaignWorldPublication>>,
    mut damage_state: ResMut<TerrainDamageState>,
    mut damaged_voxels: ResMut<DamagedVoxels>,
    mut next_screen: ResMut<NextState<Screen>>,
) {
    commands.remove_resource::<PendingInitialSnapshotPublication>();

    let Some(current) = current else {
        if pending_campaign.is_some() {
            discard_staged_campaign_world(&mut commands, &mut damage_state, &mut damaged_voxels);
            refuse_campaign_world_restore(
                &mut commands,
                CampaignWorldRestoreRefusalV2::PresentationFailed(
                    "canonical world snapshot publication failed".to_owned(),
                ),
            );
            commands.remove_resource::<PendingCampaignWorldPublication>();
        } else {
            fail_presentation_setup(
                &mut commands,
                &MapPresentationError::SnapshotResourcesMissing,
            );
        }
        next_screen.set(Screen::Title);
        return;
    };

    if let Some(pending_campaign) = pending_campaign {
        if current.fingerprint() != pending_campaign.public_fingerprint {
            discard_staged_campaign_world(&mut commands, &mut damage_state, &mut damaged_voxels);
            refuse_campaign_world_restore(
                &mut commands,
                CampaignWorldRestoreRefusalV2::PresentationFailed(
                    "canonical world snapshot fingerprint changed during publication".to_owned(),
                ),
            );
            commands.remove_resource::<PendingCampaignWorldPublication>();
            next_screen.set(Screen::Title);
            return;
        }
        commands.insert_resource(CampaignWorldRestoreResultV2 {
            outcome: CampaignWorldRestoreOutcomeV2::Applied {
                public_fingerprint: pending_campaign.public_fingerprint,
            },
        });
        commands.remove_resource::<PendingCampaignWorldPublication>();
    }

    commands.insert_resource(TerrainReady);
}

fn discard_staged_campaign_world(
    commands: &mut Commands,
    damage_state: &mut TerrainDamageState,
    damaged_voxels: &mut DamagedVoxels,
) {
    damage_state.reset(damaged_voxels);
    commands.remove_resource::<VoxelMap>();
    commands.remove_resource::<MapAnchors>();
    commands.remove_resource::<MapObservationAnchors>();
    commands.remove_resource::<SpecialMovementRegions>();
    commands.remove_resource::<InteriorRegions>();
    commands.remove_resource::<TraversalBlockers>();
    commands.remove_resource::<BiomeRegions>();
    commands.remove_resource::<MapViewHint>();
    commands.remove_resource::<MapPresentationProjection>();
    commands.remove_resource::<CurrentWorldSnapshotV1>();
    commands.remove_resource::<PendingTerrainPublication>();
    commands.remove_resource::<PendingInitialSnapshotPublication>();
    commands.remove_resource::<TerrainReady>();
    liquid_render::clear_material_cache(commands);
}

/// Spawns the grid entities. Shared by first construction and by rebuilds after an
/// edit, so the two cannot drift apart.
fn build_grid(
    commands: &mut Commands,
    assets: &GameAssets,
    palette_materials: &mut MaterialCache,
    terrain_meshes: &mut TerrainMeshCache,
    materials: &mut Assets<StandardMaterial>,
    meshes: &mut Assets<Mesh>,
    liquid_materials: &mut Assets<LiquidMaterial>,
    map: &VoxelMap,
    table: &SubstanceTable,
    settings: &MapSettings,
    liquid_phase_seconds: f32,
    interiors: Option<&InteriorRegions>,
    presentation: Option<&MapPresentationProjection>,
    art_catalog: Option<&RuntimeArtCatalog>,
    material_treatment: ReviewMaterialTreatment,
    edge_treatment: ReviewEdgeTreatment,
    crystal_light_profile: ReviewCrystalLightProfile,
) -> Result<(), MapPresentationError> {
    // Keep construction ordered behind the accepted shared asset set even though
    // terrain geometry is now baked directly rather than instancing `hex.glb`.
    let _accepted_hex_mesh = assets.hex_tile.id();
    // Crystal asset resolution happens before any presentation entities are
    // queued, so a missing or incompatible dependency cannot leave a partial map.
    let prepared_crystals =
        crystal_render::prepare_presentations(settings.level_height, presentation, art_catalog)
            .map_err(MapPresentationError::Crystal)?;
    // Substance ids are stable, but their accepted palette colours may have been
    // replaced since the previous world was presented. A full publication is the
    // lifecycle boundary at which every terrain material must be resolved again.
    // Local chunk edits deliberately keep using the refreshed cache below.
    palette_materials.reset_for_world(materials, material_treatment);
    terrain_meshes.reset_for_world(meshes);
    let mut children = liquid_render::spawn_presentations(
        commands,
        meshes,
        liquid_materials,
        map,
        table,
        settings.level_height,
        liquid_phase_seconds,
        presentation,
    )
    .map_err(MapPresentationError::Liquid)?;
    children.extend(
        feature_render::spawn_presentations(commands, settings.level_height, presentation)
            .map_err(MapPresentationError::Feature)?,
    );
    children.extend(crystal_render::spawn_prepared(
        commands,
        prepared_crystals,
        crystal_light_profile,
    ));
    children.extend(spawn_gameplay_lights(commands, presentation));

    let grid = commands
        .spawn((
            Transform::default(),
            Visibility::default(),
            Name::new("HexGrid"),
            HexGrid,
        ))
        .id();
    let mut chunk_roots = BTreeMap::new();
    for chunk in map.chunk_coords() {
        let root = spawn_terrain_chunk(
            commands,
            palette_materials,
            terrain_meshes,
            materials,
            meshes,
            map,
            table,
            settings,
            interiors,
            chunk,
            material_treatment,
            edge_treatment,
        )?;
        chunk_roots.insert(chunk, root);
    }
    debug_assert_eq!(chunk_roots.len(), map.chunk_count());
    commands
        .entity(grid)
        .add_children(&chunk_roots.values().copied().collect::<Vec<_>>());

    commands.entity(grid).add_children(&children);
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "chunk projection requires the same explicit terrain inputs as whole-grid publication"
)]
fn spawn_terrain_chunk(
    commands: &mut Commands,
    palette_materials: &mut MaterialCache,
    terrain_meshes: &mut TerrainMeshCache,
    materials: &mut Assets<StandardMaterial>,
    meshes: &mut Assets<Mesh>,
    map: &VoxelMap,
    table: &SubstanceTable,
    settings: &MapSettings,
    interiors: Option<&InteriorRegions>,
    chunk: crate::voxel::TerrainChunkCoord,
    material_treatment: ReviewMaterialTreatment,
    edge_treatment: ReviewEdgeTreatment,
) -> Result<Entity, MapPresentationError> {
    let root = commands
        .spawn((
            Transform::default(),
            Visibility::default(),
            Name::new(format!("TerrainChunk[{},{}]", chunk.q, chunk.r)),
            TerrainChunkRoot {
                q: chunk.q,
                r: chunk.r,
            },
        ))
        .id();
    let logical_root = commands
        .spawn((
            Name::new(format!("LogicalTerrainRuns[{},{}]", chunk.q, chunk.r)),
            LogicalTerrainRuns,
        ))
        .id();
    let chunk_columns = map.columns_in_chunk(chunk).collect::<Vec<_>>();
    let mut relevant_coords = BTreeSet::new();
    for (coord, _column) in &chunk_columns {
        relevant_coords.insert(*coord);
        relevant_coords.extend(coord.neighbors());
    }
    let projected_columns = relevant_coords
        .into_iter()
        .filter_map(|coord| {
            map.column(coord)
                .map(|column| (coord, projected_runs(coord, column, interiors)))
        })
        .collect::<BTreeMap<_, _>>();

    let mut logical_children = Vec::new();
    let mut rendered_children = vec![logical_root];
    let mut batches = BTreeMap::<TerrainBatchKey, Vec<PendingTerrainRun>>::new();
    for (coord, column) in chunk_columns {
        for projected in projected_columns.get(&coord).into_iter().flatten().copied() {
            let run = projected.run;
            let span = span_for(run.bottom, run.top, settings.level_height);

            // Only the map can measure this: a run knows its own extent but nothing
            // about what is stacked on it. Zero means buried, and nothing can stand
            // on a buried run however solid it is.
            let headroom = column.headroom_above(run.top);
            // The run's topmost material voxel. Gameplay combines this position with
            // the substance's `solid` flag before treating it as footing. Tagging the
            // base instead would force gameplay to know the level height to work the
            // surface out, putting a dependency on the map straight back into movement.
            // Voxels inside the run are addressed by `TilePos`, not by this entity.
            let position = TilePos::new(coord, run.top - 1);
            let mut tile = commands.spawn((
                HexTile,
                coord,
                span,
                run.substance,
                position,
                RunBottom(run.bottom),
                headroom,
            ));
            if let Some(region) = projected.cutaway {
                tile.insert((CutawayOccluder(region), PresentationOcclusion::default()));
            }
            let entity = tile.id();
            logical_children.push(entity);
            batches
                .entry(TerrainBatchKey {
                    substance: run.substance,
                    cutaway: projected.cutaway,
                })
                .or_default()
                .push(PendingTerrainRun {
                    entity,
                    position,
                    span,
                    bottom: run.bottom,
                    top: run.top,
                    cutaway: projected.cutaway,
                });
        }
    }

    let chunk_marker = TerrainChunkRoot {
        q: chunk.q,
        r: chunk.r,
    };
    let mut chunk_mesh_handles = Vec::new();
    for (key, runs) in batches {
        let material =
            palette_materials.get_or_create(key.substance, table, material_treatment, materials);
        for (partition, runs) in runs.chunks(MAX_TERRAIN_PICK_RUNS_PER_BATCH).enumerate() {
            let mesh = combined_terrain_mesh_with_edge(
                runs,
                &projected_columns,
                settings.level_height,
                edge_treatment,
            )
            .map_err(MapPresentationError::TerrainMesh)?;
            let mesh = meshes.add(mesh);
            chunk_mesh_handles.push(mesh.clone());
            let lookup = runs
                .iter()
                .map(|run| TerrainPickRun::new(run.entity, run.position, run.span))
                .collect();
            let mut batch = commands.spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material.clone()),
                Transform::default(),
                Visibility::Inherited,
                Pickable::default(),
                TerrainRenderBatch::new(chunk_marker, key.substance, lookup),
                Name::new(format!(
                    "TerrainBatch[{},{},{}:{}]",
                    chunk.q, chunk.r, key.substance.0, partition
                )),
            ));
            if let Some(region) = key.cutaway {
                batch.insert((CutawayOccluder(region), PresentationOcclusion::default()));
            }
            rendered_children.push(batch.id());
        }
    }
    terrain_meshes.replace_chunk(chunk, chunk_mesh_handles, meshes);
    commands
        .entity(logical_root)
        .add_children(&logical_children);
    commands.entity(root).add_children(&rendered_children);
    Ok(root)
}

fn spawn_gameplay_lights(
    commands: &mut Commands,
    presentation: Option<&MapPresentationProjection>,
) -> Vec<Entity> {
    presentation.map_or_else(Vec::new, |presentation| {
        presentation
            .lights()
            .values()
            .map(|light| {
                commands
                    .spawn((
                        Name::new("GeneratedGameplayLight"),
                        light.origin,
                        GameplayLight::new(light.level, light.radius),
                    ))
                    .id()
            })
            .collect()
    })
}

fn fail_presentation_setup(commands: &mut Commands, error: &MapPresentationError) {
    error!("cannot build map presentation: {error}");
    commands.remove_resource::<TerrainReady>();
    commands.insert_resource(GameplaySetupFailure::new(format!(
        "The selected terrain cannot be presented: {error}."
    )));
    liquid_render::clear_material_cache(commands);
}

#[derive(Debug)]
enum MapPresentationError {
    Liquid(LiquidPresentationError),
    Feature(FeaturePresentationError),
    Crystal(CrystalPresentationError),
    TerrainMesh(String),
    TerrainGridMissing,
    MultipleTerrainGrids,
    TerrainChunkTopology(String),
    SnapshotResourcesMissing,
}

#[derive(SystemParam)]
struct MapPresentationAssets<'w> {
    terrain_materials: ResMut<'w, MaterialCache>,
    terrain_meshes: ResMut<'w, TerrainMeshCache>,
    materials: ResMut<'w, Assets<StandardMaterial>>,
    meshes: ResMut<'w, Assets<Mesh>>,
    liquid_materials: ResMut<'w, Assets<LiquidMaterial>>,
}

/// Borrow-only world inputs for canonical reconnect publication.
#[derive(SystemParam)]
struct WorldSnapshotSources<'w> {
    map: Option<Res<'w, VoxelMap>>,
    table: Option<Res<'w, SubstanceTable>>,
    settings: Option<Res<'w, MapSettings>>,
    damage: Option<Res<'w, DamagedVoxels>>,
    anchors: Option<Res<'w, MapAnchors>>,
    interiors: Option<Res<'w, InteriorRegions>>,
    special_regions: Option<Res<'w, SpecialMovementRegions>>,
    biome_regions: Option<Res<'w, BiomeRegions>>,
    blockers: Option<Res<'w, TraversalBlockers>>,
    view_hint: Option<Res<'w, MapViewHint>>,
    presentation: Option<Res<'w, MapPresentationProjection>>,
    art_catalog: Option<Res<'w, RuntimeArtCatalog>>,
}

impl WorldSnapshotSources<'_> {
    fn parts(&self) -> Result<WorldExportParts<'_>, crate::WorldSnapshotError> {
        let map = self
            .map
            .as_deref()
            .ok_or(crate::WorldSnapshotError::WorldUnavailable("VoxelMap"))?;
        let table = self
            .table
            .as_deref()
            .ok_or(crate::WorldSnapshotError::WorldUnavailable(
                "SubstanceTable",
            ))?;
        let settings = self
            .settings
            .as_deref()
            .ok_or(crate::WorldSnapshotError::WorldUnavailable("MapSettings"))?;
        let damage = self
            .damage
            .as_deref()
            .ok_or(crate::WorldSnapshotError::WorldUnavailable("DamagedVoxels"))?;
        let anchors = self
            .anchors
            .as_deref()
            .ok_or(crate::WorldSnapshotError::WorldUnavailable("MapAnchors"))?;
        let interiors =
            self.interiors
                .as_deref()
                .ok_or(crate::WorldSnapshotError::WorldUnavailable(
                    "InteriorRegions",
                ))?;
        let special_regions =
            self.special_regions
                .as_deref()
                .ok_or(crate::WorldSnapshotError::WorldUnavailable(
                    "SpecialMovementRegions",
                ))?;
        Ok(WorldExportParts {
            map,
            table,
            settings,
            damage,
            anchors,
            interiors,
            special_regions,
            biome_regions: self.biome_regions.as_deref(),
            blockers: self.blockers.as_deref(),
            view_hint: self.view_hint.as_deref(),
            presentation: self.presentation.as_deref(),
            art_catalog: self.art_catalog.as_deref(),
        })
    }

    fn export(&self) -> Result<hex_multiplayer::WorldSnapshotV1, crate::WorldSnapshotError> {
        export_from_parts(self.parts()?)
    }
}

fn publish_current_world_snapshot(
    mut commands: Commands,
    sources: WorldSnapshotSources,
    mut current: Option<ResMut<CurrentWorldSnapshotV1>>,
    mut dirty: ResMut<WorldSnapshotDirty>,
    mut next_screen: ResMut<NextState<Screen>>,
) {
    if !dirty.is_dirty() {
        return;
    }

    let published = if dirty.full_refresh || current.is_none() {
        sources.export().map(|snapshot| {
            commands.insert_resource(CurrentWorldSnapshotV1::new(snapshot));
        })
    } else {
        current
            .as_deref_mut()
            .ok_or(crate::WorldSnapshotError::WorldUnavailable(
                "CurrentWorldSnapshotV1",
            ))
            .and_then(|current| {
                sources.parts().and_then(|parts| {
                    current.refresh_changed_coordinates(parts, &dirty.changed_coords)
                })
            })
    };
    match published {
        Ok(()) => dirty.clear(),
        Err(error) => {
            error!("cannot publish current world snapshot: {error}");
            commands.remove_resource::<CurrentWorldSnapshotV1>();
            commands.remove_resource::<TerrainReady>();
            commands.insert_resource(GameplaySetupFailure::new(format!(
                "The active terrain cannot be synchronized: {error}."
            )));
            next_screen.set(Screen::Title);
        }
    }
}

/// Mutable session-local state cleared atomically with a restored world.
#[derive(SystemParam)]
struct WorldImportRuntime<'w, 's> {
    grids: Query<'w, 's, Entity, With<HexGrid>>,
    damage_state: ResMut<'w, TerrainDamageState>,
    damaged_voxels: ResMut<'w, DamagedVoxels>,
    pending_edits: ResMut<'w, PendingTerrainEdits>,
    pending_impacts: ResMut<'w, PendingTerrainImpacts>,
    edits: ResMut<'w, Messages<TerrainEdit>>,
    impacts: ResMut<'w, Messages<TerrainImpact>>,
    terrain_outcomes: ResMut<'w, Messages<TerrainImpactOutcome>>,
    dirty: ResMut<'w, WorldSnapshotDirty>,
    replication_state: ResMut<'w, WorldReplicationStateV1>,
}

fn apply_world_replication_requests(
    mut commands: Commands,
    mut requests: MessageReader<WorldReplicationRequestV1>,
    mut results: MessageWriter<WorldReplicationResultV1>,
    boundary: Res<AuthorityBoundary>,
    current: Option<Res<CurrentWorldSnapshotV1>>,
    pending_build: Option<Res<PendingSnapshotGridBuild>>,
    table: Option<Res<SubstanceTable>>,
    settings: Option<Res<MapSettings>>,
    art_catalog: Option<Res<RuntimeArtCatalog>>,
    mut runtime: WorldImportRuntime,
) {
    let incoming = requests.read().cloned().collect::<Vec<_>>();
    if incoming.is_empty() {
        return;
    }
    if pending_build.is_some() {
        for request in incoming {
            results.write(refused_world_request(
                request.authority_sequence(),
                WorldReplicationRefusalV1::BoundaryBusy,
            ));
        }
        return;
    }

    let Some(table) = table.as_deref() else {
        refuse_unavailable_world_requests(incoming, "SubstanceTable", &mut results);
        return;
    };
    let Some(settings) = settings.as_deref() else {
        refuse_unavailable_world_requests(incoming, "MapSettings", &mut results);
        return;
    };

    let previous_sequence = runtime.replication_state.last_applied_sequence();
    let mut staged_sequence = previous_sequence;
    let mut staged_snapshot = current.as_deref().map(|current| current.snapshot().clone());
    let mut latest_prepared: Option<PreparedWorldSnapshotV1> = None;
    let mut ordered_results = Vec::new();

    for (index, request) in incoming.into_iter().enumerate() {
        let sequence = request.authority_sequence();
        if index >= MAX_WORLD_REPLICATION_REQUESTS_PER_UPDATE {
            ordered_results.push(refused_world_request(
                sequence,
                WorldReplicationRefusalV1::RequestBurstExceeded,
            ));
            continue;
        }
        let target_fingerprint = match &request {
            WorldReplicationRequestV1::Restore { snapshot, .. } => snapshot.public_fingerprint,
            WorldReplicationRequestV1::ApplyDelta(delta) => delta.target_fingerprint,
        };
        if let Some(last) = staged_sequence {
            if sequence < last {
                ordered_results.push(refused_world_request(
                    sequence,
                    WorldReplicationRefusalV1::StaleSequence,
                ));
                continue;
            }
            if sequence == last {
                let outcome = if staged_snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.public_fingerprint == target_fingerprint)
                {
                    WorldReplicationOutcomeV1::Duplicate {
                        public_fingerprint: target_fingerprint,
                    }
                } else {
                    WorldReplicationOutcomeV1::Refused(WorldReplicationRefusalV1::SequenceConflict)
                };
                ordered_results.push(WorldReplicationResultV1 {
                    authority_sequence: sequence,
                    outcome,
                });
                continue;
            }
        }
        if !boundary.is_quiescent() {
            ordered_results.push(refused_world_request(
                sequence,
                WorldReplicationRefusalV1::BoundaryBusy,
            ));
            continue;
        }

        let candidate = match request {
            WorldReplicationRequestV1::Restore { snapshot, .. } => Ok(*snapshot),
            WorldReplicationRequestV1::ApplyDelta(delta) => staged_snapshot
                .as_ref()
                .ok_or(WorldReplicationRefusalV1::MissingCurrentWorld)
                .and_then(|base| {
                    apply_world_delta_v1(base, &delta)
                        .map_err(WorldReplicationRefusalV1::InvalidSnapshot)
                }),
        };
        let candidate = match candidate {
            Ok(candidate) => candidate,
            Err(reason) => {
                ordered_results.push(refused_world_request(sequence, reason));
                continue;
            }
        };
        let prepared =
            match prepare_world_snapshot_v1(candidate, table, settings, art_catalog.as_deref()) {
                Ok(prepared) => prepared,
                Err(error) => {
                    ordered_results.push(refused_world_request(
                        sequence,
                        WorldReplicationRefusalV1::InvalidSnapshot(error),
                    ));
                    continue;
                }
            };
        staged_snapshot = Some(prepared.snapshot.clone());
        staged_sequence = Some(sequence);
        ordered_results.push(WorldReplicationResultV1 {
            authority_sequence: sequence,
            outcome: WorldReplicationOutcomeV1::Applied {
                public_fingerprint: prepared.snapshot.public_fingerprint,
            },
        });
        latest_prepared = Some(prepared);
    }

    let Some(prepared) = latest_prepared else {
        for result in ordered_results {
            results.write(result);
        }
        return;
    };
    commit_prepared_world_snapshot(
        &mut commands,
        prepared,
        ordered_results,
        previous_sequence,
        staged_sequence,
        &mut runtime,
    );
}

fn refuse_unavailable_world_requests(
    requests: Vec<WorldReplicationRequestV1>,
    resource: &'static str,
    results: &mut MessageWriter<WorldReplicationResultV1>,
) {
    for request in requests {
        results.write(refused_world_request(
            request.authority_sequence(),
            WorldReplicationRefusalV1::InvalidSnapshot(
                crate::WorldSnapshotError::WorldUnavailable(resource),
            ),
        ));
    }
}

fn refused_world_request(
    authority_sequence: hex_multiplayer::AuthoritySequence,
    reason: WorldReplicationRefusalV1,
) -> WorldReplicationResultV1 {
    WorldReplicationResultV1 {
        authority_sequence,
        outcome: WorldReplicationOutcomeV1::Refused(reason),
    }
}

fn commit_prepared_world_snapshot(
    commands: &mut Commands,
    prepared: PreparedWorldSnapshotV1,
    ordered_results: Vec<WorldReplicationResultV1>,
    previous_sequence: Option<hex_multiplayer::AuthoritySequence>,
    staged_sequence: Option<hex_multiplayer::AuthoritySequence>,
    runtime: &mut WorldImportRuntime,
) {
    let PreparedWorldSnapshotV1 {
        snapshot,
        map,
        damage,
        anchors,
        interiors,
        special_regions,
        biome_regions,
        blockers,
        view_hint,
        presentation,
    } = prepared;

    for entity in &runtime.grids {
        commands.entity(entity).despawn();
    }
    runtime
        .damage_state
        .restore(damage, &mut runtime.damaged_voxels);
    runtime.pending_edits.0.clear();
    runtime.pending_impacts.0.clear();
    runtime.edits.clear();
    runtime.impacts.clear();
    runtime.terrain_outcomes.clear();
    runtime.dirty.clear();
    runtime
        .replication_state
        .set_last_applied_sequence(staged_sequence);

    commands.remove_resource::<TerrainReady>();
    commands.remove_resource::<GameplaySetupFailure>();
    commands.remove_resource::<GenerationReport>();
    commands.remove_resource::<MapViewHint>();
    commands.remove_resource::<MapPresentationProjection>();
    liquid_render::clear_material_cache(commands);
    commands.insert_resource(map);
    commands.insert_resource(anchors);
    commands.insert_resource(MapObservationAnchors::new());
    commands.insert_resource(interiors);
    commands.insert_resource(special_regions);
    commands.insert_resource(biome_regions);
    commands.insert_resource(blockers);
    if let Some(view_hint) = view_hint {
        commands.insert_resource(view_hint);
    }
    if let Some(presentation) = presentation {
        commands.insert_resource(presentation);
    }
    commands.insert_resource(CurrentWorldSnapshotV1::new(snapshot));
    commands.insert_resource(PendingSnapshotGridBuild {
        ordered_results,
        previous_sequence,
    });
}

fn spawn_imported_snapshot_grid(
    mut commands: Commands,
    pending: Option<Res<PendingSnapshotGridBuild>>,
    mut results: MessageWriter<WorldReplicationResultV1>,
    assets: Res<GameAssets>,
    mut presentation_assets: MapPresentationAssets,
    map: Option<Res<VoxelMap>>,
    table: Option<Res<SubstanceTable>>,
    settings: Option<Res<MapSettings>>,
    liquid_visual_time: Res<LiquidVisualTime>,
    interiors: Option<Res<InteriorRegions>>,
    presentation: Option<Res<MapPresentationProjection>>,
    art_catalog: Option<Res<RuntimeArtCatalog>>,
    material_treatment: Option<Res<ReviewMaterialTreatment>>,
    edge_treatment: Option<Res<ReviewEdgeTreatment>>,
    crystal_light_profile: Option<Res<ReviewCrystalLightProfile>>,
    mut replication_state: ResMut<WorldReplicationStateV1>,
) {
    let Some(pending) = pending else {
        return;
    };
    let built = match (map.as_deref(), table.as_deref(), settings.as_deref()) {
        (Some(map), Some(table), Some(settings)) => build_grid(
            &mut commands,
            &assets,
            &mut presentation_assets.terrain_materials,
            &mut presentation_assets.terrain_meshes,
            &mut presentation_assets.materials,
            &mut presentation_assets.meshes,
            &mut presentation_assets.liquid_materials,
            map,
            table,
            settings,
            liquid_visual_time.phase_seconds(),
            interiors.as_deref(),
            presentation.as_deref(),
            art_catalog.as_deref(),
            material_treatment.as_deref().copied().unwrap_or_default(),
            edge_treatment.as_deref().copied().unwrap_or_default(),
            crystal_light_profile
                .as_deref()
                .copied()
                .unwrap_or_default(),
        ),
        _ => Err(MapPresentationError::SnapshotResourcesMissing),
    };
    match built {
        Ok(()) => {
            commands.insert_resource(TerrainReady);
            for result in &pending.ordered_results {
                results.write(result.clone());
            }
        }
        Err(error) => {
            let reason = error.to_string();
            fail_presentation_setup(&mut commands, &error);
            commands.remove_resource::<CurrentWorldSnapshotV1>();
            replication_state.set_last_applied_sequence(pending.previous_sequence);
            for result in &pending.ordered_results {
                if matches!(&result.outcome, WorldReplicationOutcomeV1::Applied { .. }) {
                    results.write(refused_world_request(
                        result.authority_sequence,
                        WorldReplicationRefusalV1::PresentationFailed(reason.clone()),
                    ));
                } else {
                    results.write(result.clone());
                }
            }
        }
    }
    commands.remove_resource::<PendingSnapshotGridBuild>();
}

impl fmt::Display for MapPresentationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Liquid(error) => write!(formatter, "liquid presentation failed: {error}"),
            Self::Feature(error) => write!(formatter, "feature presentation failed: {error}"),
            Self::Crystal(error) => write!(formatter, "crystal presentation failed: {error}"),
            Self::TerrainMesh(error) => write!(formatter, "terrain mesh batching failed: {error}"),
            Self::TerrainGridMissing => formatter.write_str("terrain grid root is missing"),
            Self::MultipleTerrainGrids => {
                formatter.write_str("more than one terrain grid root is active")
            }
            Self::TerrainChunkTopology(reason) => {
                write!(formatter, "terrain chunk topology is invalid: {reason}")
            }
            Self::SnapshotResourcesMissing => {
                formatter.write_str("snapshot presentation resources are missing")
            }
        }
    }
}

impl std::error::Error for MapPresentationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Liquid(error) => Some(error),
            Self::Feature(error) => Some(error),
            Self::Crystal(error) => Some(error),
            Self::TerrainMesh(_) => None,
            Self::TerrainGridMissing
            | Self::MultipleTerrainGrids
            | Self::TerrainChunkTopology(_)
            | Self::SnapshotResourcesMissing => None,
        }
    }
}

/// One material run split further wherever exact cutaway membership changes.
///
/// Rendered runs are disposable projections. Keeping cutaway ownership on exact
/// voxels lets this rebuild both fragments after digging through a roof and prevents
/// a replacement material from inheriting the old run's component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProjectedRun {
    pub(crate) run: SubstanceRun,
    pub(crate) cutaway: Option<InteriorRegionId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct TerrainBatchKey {
    substance: SubstanceId,
    cutaway: Option<InteriorRegionId>,
}

#[derive(Debug, Clone, Copy)]
struct PendingTerrainRun {
    entity: Entity,
    position: TilePos,
    span: HexSpan,
    bottom: hex_core::Level,
    top: hex_core::Level,
    cutaway: Option<InteriorRegionId>,
}

#[derive(Debug, Default)]
struct TerrainRawMesh {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
    edge_treatment: ReviewEdgeTreatment,
    geometric_bevel: Option<TerrainGeometricBevel>,
}

impl TerrainRawMesh {
    fn with_edge_treatment(
        edge_treatment: ReviewEdgeTreatment,
        level_height: f32,
    ) -> Result<Self, String> {
        Ok(Self {
            edge_treatment,
            geometric_bevel: TerrainGeometricBevel::resolve(edge_treatment, level_height)?,
            ..Self::default()
        })
    }

    fn vertex(&mut self, position: Vec3, normal: Vec3, uv: [f32; 2]) -> Result<u32, String> {
        let index = u32::try_from(self.positions.len())
            .map_err(|_error| "terrain mesh vertex index overflowed u32".to_owned())?;
        self.positions.push(position.to_array());
        self.normals.push(normal.to_array());
        self.uvs.push(uv);
        Ok(index)
    }

    fn cap(&mut self, coord: HexCoord, y: f32, top: bool) -> Result<(), String> {
        if let Some(bevel) = self.geometric_bevel {
            return self.geometric_cap(coord, y, top, bevel);
        }
        let centre = coord.to_world(y);
        let normal = if top { Vec3::Y } else { Vec3::NEG_Y };
        let centre_index = self.vertex(centre, normal, [0.5, 0.5])?;
        let mut corner_indices = [0u32; 6];
        for (slot, corner) in corner_indices.iter_mut().zip(terrain_hex_corners()) {
            let edge_direction = (normal + corner.normalize()).normalize();
            *slot = self.vertex(
                centre + corner,
                micro_bevel_normal(normal, edge_direction, self.edge_treatment),
                [0.5 + corner.x * 0.5, 0.5 + corner.z * 0.5],
            )?;
        }
        let mut emit_triangle = |current: u32, next: u32| {
            if top {
                self.indices.extend([centre_index, current, next]);
            } else {
                self.indices.extend([centre_index, next, current]);
            }
        };
        for pair in corner_indices.windows(2) {
            let [current, next] = pair else {
                return Err("terrain cap corner partition was malformed".to_owned());
            };
            emit_triangle(*current, *next);
        }
        let first = corner_indices
            .first()
            .copied()
            .ok_or_else(|| "terrain cap has no first corner".to_owned())?;
        let last = corner_indices
            .last()
            .copied()
            .ok_or_else(|| "terrain cap has no last corner".to_owned())?;
        emit_triangle(last, first);
        Ok(())
    }

    fn geometric_cap(
        &mut self,
        coord: HexCoord,
        y: f32,
        top: bool,
        bevel: TerrainGeometricBevel,
    ) -> Result<(), String> {
        let centre = coord.to_world(y);
        let normal = if top { Vec3::Y } else { Vec3::NEG_Y };
        let centre_index = self.vertex(centre, normal, [0.5, 0.5])?;
        let mut corner_indices = [0u32; 6];
        for (slot, corner) in corner_indices
            .iter_mut()
            .zip(terrain_hex_corners().map(|corner| bevel.inset_corner(corner)))
        {
            *slot = self.vertex(centre + corner, normal, terrain_cap_uv(corner))?;
        }
        let mut emit_triangle = |current: u32, next: u32| {
            if top {
                self.indices.extend([centre_index, current, next]);
            } else {
                self.indices.extend([centre_index, next, current]);
            }
        };
        for pair in corner_indices.windows(2) {
            let [current, next] = pair else {
                return Err("terrain cap corner partition was malformed".to_owned());
            };
            emit_triangle(*current, *next);
        }
        let first = corner_indices
            .first()
            .copied()
            .ok_or_else(|| "terrain cap has no first corner".to_owned())?;
        let last = corner_indices
            .last()
            .copied()
            .ok_or_else(|| "terrain cap has no last corner".to_owned())?;
        emit_triangle(last, first);

        let outer_y = if top {
            y - bevel.depth
        } else {
            y + bevel.depth
        };
        let outer_centre = coord.to_world(outer_y);
        for [outer_first, outer_second] in terrain_hex_ring_edges() {
            let inner_first = bevel.inset_corner(outer_first);
            let inner_second = bevel.inset_corner(outer_second);
            let outer_first_position = outer_centre + outer_first;
            let outer_second_position = outer_centre + outer_second;
            let inner_first_position = centre + inner_first;
            let inner_second_position = centre + inner_second;
            if top {
                self.quad(
                    [
                        outer_first_position,
                        outer_second_position,
                        inner_second_position,
                        inner_first_position,
                    ],
                    [
                        terrain_cap_uv(outer_first),
                        terrain_cap_uv(outer_second),
                        terrain_cap_uv(inner_second),
                        terrain_cap_uv(inner_first),
                    ],
                )?;
            } else {
                self.quad(
                    [
                        outer_first_position,
                        inner_first_position,
                        inner_second_position,
                        outer_second_position,
                    ],
                    [
                        terrain_cap_uv(outer_first),
                        terrain_cap_uv(inner_first),
                        terrain_cap_uv(inner_second),
                        terrain_cap_uv(outer_second),
                    ],
                )?;
            }
        }
        Ok(())
    }

    fn quad(&mut self, positions: [Vec3; 4], uvs: [[f32; 2]; 4]) -> Result<(), String> {
        let [first, second, third, fourth] = positions;
        let [first_uv, second_uv, third_uv, fourth_uv] = uvs;
        let cross = (second - first).cross(third - first);
        if !cross.is_finite() || cross.length_squared() <= f32::EPSILON {
            return Err("terrain bevel produced a degenerate face".to_owned());
        }
        let normal = cross.normalize();
        let a = self.vertex(first, normal, first_uv)?;
        let b = self.vertex(second, normal, second_uv)?;
        let c = self.vertex(third, normal, third_uv)?;
        let d = self.vertex(fourth, normal, fourth_uv)?;
        self.indices.extend([a, b, c, a, c, d]);
        Ok(())
    }

    fn side(
        &mut self,
        coord: HexCoord,
        [first, second]: [Vec3; 2],
        normal: Vec3,
        bottom: f32,
        top: f32,
        trim_bottom: bool,
        trim_top: bool,
    ) -> Result<(), String> {
        let (bottom, top) = self.geometric_bevel.map_or((bottom, top), |bevel| {
            (
                if trim_bottom {
                    bottom + bevel.depth
                } else {
                    bottom
                },
                if trim_top { top - bevel.depth } else { top },
            )
        });
        if top - bottom <= f32::EPSILON {
            return Err("terrain bevel consumed an exposed side interval".to_owned());
        }
        let base = coord.to_world(0.0);
        let first_bottom = base + first + Vec3::Y * bottom;
        let second_bottom = base + second + Vec3::Y * bottom;
        let second_top = base + second + Vec3::Y * top;
        let first_top = base + first + Vec3::Y * top;
        let first_horizontal = first.normalize();
        let second_horizontal = second.normalize();
        let first_bottom_normal = micro_bevel_normal(
            normal,
            (first_horizontal + Vec3::NEG_Y).normalize(),
            self.edge_treatment,
        );
        let second_bottom_normal = micro_bevel_normal(
            normal,
            (second_horizontal + Vec3::NEG_Y).normalize(),
            self.edge_treatment,
        );
        let second_top_normal = micro_bevel_normal(
            normal,
            (second_horizontal + Vec3::Y).normalize(),
            self.edge_treatment,
        );
        let first_top_normal = micro_bevel_normal(
            normal,
            (first_horizontal + Vec3::Y).normalize(),
            self.edge_treatment,
        );
        let a = self.vertex(first_bottom, first_bottom_normal, [0.0, bottom])?;
        let b = self.vertex(second_bottom, second_bottom_normal, [1.0, bottom])?;
        let c = self.vertex(second_top, second_top_normal, [1.0, top])?;
        let d = self.vertex(first_top, first_top_normal, [0.0, top])?;
        self.indices.extend([a, b, c, a, c, d]);
        Ok(())
    }

    fn into_mesh(self) -> Result<Mesh, String> {
        if self.positions.is_empty() || self.indices.is_empty() {
            return Err("terrain batch produced no visible geometry".to_owned());
        }
        if !self
            .positions
            .iter()
            .flatten()
            .chain(self.normals.iter().flatten())
            .chain(self.uvs.iter().flatten())
            .all(|component| component.is_finite())
        {
            return Err("terrain batch produced non-finite geometry".to_owned());
        }
        Ok(Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs)
        .with_inserted_indices(Indices::U32(self.indices)))
    }
}

#[derive(Debug, Clone, Copy)]
struct TerrainGeometricBevel {
    inset: f32,
    depth: f32,
}

impl TerrainGeometricBevel {
    fn resolve(treatment: ReviewEdgeTreatment, level_height: f32) -> Result<Option<Self>, String> {
        let Some(fraction) = treatment.geometric_bevel_fraction() else {
            return Ok(None);
        };
        if !level_height.is_finite() || level_height <= 0.0 {
            return Err(
                "geometric terrain bevel requires a finite positive level height".to_owned(),
            );
        }
        let inset = fraction * hex_core::config::HEX_CIRCUMRADIUS;
        // A quarter-level cap keeps top and bottom chamfers separated and leaves a
        // positive side face even for the smallest one-level exposed interval.
        let depth = inset.min(level_height * 0.25);
        if !inset.is_finite()
            || !depth.is_finite()
            || inset <= 0.0
            || inset >= hex_core::config::HEX_CIRCUMRADIUS
            || depth <= 0.0
        {
            return Err("geometric terrain bevel resolved outside the voxel bounds".to_owned());
        }
        Ok(Some(Self { inset, depth }))
    }

    fn inset_corner(self, corner: Vec3) -> Vec3 {
        let scale =
            (hex_core::config::HEX_CIRCUMRADIUS - self.inset) / hex_core::config::HEX_CIRCUMRADIUS;
        corner * scale
    }
}

fn micro_bevel_normal(
    face_normal: Vec3,
    edge_direction: Vec3,
    treatment: ReviewEdgeTreatment,
) -> Vec3 {
    let blend = treatment.normal_blend();
    if blend == 0.0 {
        return face_normal;
    }
    face_normal.lerp(edge_direction, blend).normalize()
}

fn terrain_hex_corners() -> [Vec3; 6] {
    let radius = hex_core::config::HEX_CIRCUMRADIUS;
    let inradius = 0.5 * hex_core::config::HEX_SMALL_DIAMETER;
    [
        Vec3::new(0.0, 0.0, -radius),
        Vec3::new(-inradius, 0.0, -0.5 * radius),
        Vec3::new(-inradius, 0.0, 0.5 * radius),
        Vec3::new(0.0, 0.0, radius),
        Vec3::new(inradius, 0.0, 0.5 * radius),
        Vec3::new(inradius, 0.0, -0.5 * radius),
    ]
}

fn terrain_hex_sides() -> [([Vec3; 2], Vec3); 6] {
    let [north, north_west, south_west, south, south_east, north_east] = terrain_hex_corners();
    [
        ([south_east, north_east], Vec3::X),
        ([south, south_east], Vec3::new(0.5, 0.0, 0.866_025_4)),
        ([south_west, south], Vec3::new(-0.5, 0.0, 0.866_025_4)),
        ([north_west, south_west], Vec3::NEG_X),
        ([north, north_west], Vec3::new(-0.5, 0.0, -0.866_025_4)),
        ([north_east, north], Vec3::new(0.5, 0.0, -0.866_025_4)),
    ]
}

fn terrain_hex_ring_edges() -> [[Vec3; 2]; 6] {
    let [north, north_west, south_west, south, south_east, north_east] = terrain_hex_corners();
    [
        [north, north_west],
        [north_west, south_west],
        [south_west, south],
        [south, south_east],
        [south_east, north_east],
        [north_east, north],
    ]
}

fn terrain_cap_uv(corner: Vec3) -> [f32; 2] {
    [0.5 + corner.x * 0.5, 0.5 + corner.z * 0.5]
}

fn owner_at(
    projected: Option<&[ProjectedRun]>,
    level: hex_core::Level,
    owner: Option<InteriorRegionId>,
) -> bool {
    projected.is_some_and(|runs| {
        runs.iter()
            .any(|run| run.cutaway == owner && run.run.bottom <= level && level < run.run.top)
    })
}

fn exposed_intervals(
    bottom: hex_core::Level,
    top: hex_core::Level,
    neighbour: Option<&[ProjectedRun]>,
    owner: Option<InteriorRegionId>,
) -> Vec<(hex_core::Level, hex_core::Level)> {
    let mut cursor = bottom;
    let mut exposed = Vec::new();
    let occluders = neighbour
        .into_iter()
        .flatten()
        .filter(|run| run.cutaway == owner && run.run.top > bottom && run.run.bottom < top);
    for occluder in occluders {
        let occluder_bottom = occluder.run.bottom.max(bottom);
        let occluder_top = occluder.run.top.min(top);
        if cursor < occluder_bottom {
            exposed.push((cursor, occluder_bottom));
        }
        cursor = cursor.max(occluder_top);
        if cursor >= top {
            break;
        }
    }
    if cursor < top {
        exposed.push((cursor, top));
    }
    exposed
}

#[cfg(test)]
fn combined_terrain_mesh(
    runs: &[PendingTerrainRun],
    projected_columns: &BTreeMap<HexCoord, Vec<ProjectedRun>>,
    level_height: f32,
) -> Result<Mesh, String> {
    combined_terrain_mesh_with_edge(
        runs,
        projected_columns,
        level_height,
        ReviewEdgeTreatment::Current,
    )
}

fn combined_terrain_mesh_with_edge(
    runs: &[PendingTerrainRun],
    projected_columns: &BTreeMap<HexCoord, Vec<ProjectedRun>>,
    level_height: f32,
    edge_treatment: ReviewEdgeTreatment,
) -> Result<Mesh, String> {
    terrain_mesh_from_runs(
        runs.iter().map(|run| TerrainMeshRun {
            position: run.position,
            span: run.span,
            bottom: run.bottom,
            top: run.top,
            cutaway: run.cutaway,
        }),
        projected_columns,
        level_height,
        edge_treatment,
        false,
    )
}

/// Geometry-only input shared by the legacy and resident publication adapters.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TerrainMeshRun {
    pub(crate) position: TilePos,
    pub(crate) span: HexSpan,
    pub(crate) bottom: i32,
    pub(crate) top: i32,
    pub(crate) cutaway: Option<InteriorRegionId>,
}

/// Uses the same cap, side-culling, normals, winding and batched mesh engine as V3.
/// Columns contain exactly one source chunk, so boundaries remain closed even when
/// the render origin is not aligned with the storage lattice. Negative bottom
/// levels are valid; the legacy bedrock-floor convention does not apply here.
pub(crate) fn resident_terrain_mesh(
    runs: &[TerrainMeshRun],
    columns: &BTreeMap<HexCoord, Vec<ProjectedRun>>,
    level_height: f32,
) -> Result<Mesh, String> {
    terrain_mesh_from_runs(
        runs.iter().copied(),
        columns,
        level_height,
        ReviewEdgeTreatment::Current,
        true,
    )
}

fn terrain_mesh_from_runs(
    runs: impl IntoIterator<Item = TerrainMeshRun>,
    projected_columns: &BTreeMap<HexCoord, Vec<ProjectedRun>>,
    level_height: f32,
    edge_treatment: ReviewEdgeTreatment,
    resident: bool,
) -> Result<Mesh, String> {
    let mut combined = TerrainRawMesh::with_edge_treatment(edge_treatment, level_height)?;
    for run in runs {
        // Retaining each run's top cap preserves material boundaries and guarantees
        // every logical run has one exact pick surface in its bounded batch. Buried
        // caps remain depth-occluded; edits rebuild the chunk before exposure.
        combined.cap(run.position.coord, run.span.top, true)?;
        let bottom_exposed = (resident || run.bottom > 0)
            && run.bottom.checked_sub(1).is_none_or(|below| {
                !owner_at(
                    projected_columns
                        .get(&run.position.coord)
                        .map(Vec::as_slice),
                    below,
                    run.cutaway,
                )
            });
        if bottom_exposed {
            combined.cap(run.position.coord, run.span.bottom, false)?;
        }
        for (neighbour, (side_corners, side_normal)) in run
            .position
            .coord
            .neighbors()
            .into_iter()
            .zip(terrain_hex_sides())
        {
            // Resident chunk meshes own their seam walls permanently. Depending on
            // another chunk's columns here would force an otherwise local edit to
            // replace neighbouring roots just to repair one culled face.
            let neighbour_runs = if resident
                || terrain_chunk_coord(neighbour) == terrain_chunk_coord(run.position.coord)
            {
                projected_columns.get(&neighbour).map(Vec::as_slice)
            } else {
                None
            };
            for (bottom, top) in exposed_intervals(run.bottom, run.top, neighbour_runs, run.cutaway)
            {
                let trim_bottom = bottom_exposed && bottom == run.bottom;
                let trim_top = top == run.top;
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "legacy levels and checked resident-local levels are within f32 exact integer range"
                )]
                let (bottom, top) = (bottom as f32 * level_height, top as f32 * level_height);
                combined.side(
                    run.position.coord,
                    side_corners,
                    side_normal,
                    bottom,
                    top,
                    trim_bottom,
                    trim_top,
                )?;
            }
        }
    }
    combined.into_mesh()
}

fn projected_runs(
    coord: HexCoord,
    column: &Column,
    interiors: Option<&InteriorRegions>,
) -> Vec<ProjectedRun> {
    let material_runs = runs(column);
    let Some(interiors) = interiors.filter(|interiors| interiors.has_roof_voxels()) else {
        return material_runs
            .into_iter()
            .map(|run| ProjectedRun { run, cutaway: None })
            .collect();
    };

    let mut projected = Vec::new();
    for material_run in material_runs {
        let mut bottom = material_run.bottom;
        let mut cutaway = interiors.roof_region(TilePos::new(coord, bottom));
        for level in material_run.bottom.saturating_add(1)..material_run.top {
            let next = interiors.roof_region(TilePos::new(coord, level));
            if next == cutaway {
                continue;
            }
            projected.push(ProjectedRun {
                run: SubstanceRun {
                    bottom,
                    top: level,
                    substance: material_run.substance,
                },
                cutaway,
            });
            bottom = level;
            cutaway = next;
        }
        projected.push(ProjectedRun {
            run: SubstanceRun {
                bottom,
                top: material_run.top,
                substance: material_run.substance,
            },
            cutaway,
        });
    }
    projected
}

/// World-space extent of a run of levels.
fn span_for(bottom: hex_core::Level, top: hex_core::Level, level_height: f32) -> HexSpan {
    #[expect(
        clippy::cast_precision_loss,
        reason = "levels are small integers, exact in f32 far beyond any playable depth"
    )]
    HexSpan::new(bottom as f32 * level_height, top as f32 * level_height)
}

/// One material per substance, created on demand.
///
/// Without this every run would allocate its own `StandardMaterial`, so a world of a
/// few thousand runs would hold a few thousand identical materials and defeat any
/// chance of batching.
#[derive(Resource, Default)]
struct MaterialCache {
    treatment: ReviewMaterialTreatment,
    by_substance: Vec<(SubstanceId, Handle<StandardMaterial>)>,
}

/// Owns generated combined meshes so chunk replacement and teardown remove assets
/// as well as entities. Otherwise repeated digging would leak one mesh allocation per
/// retired batch even though its chunk root had been despawned.
#[derive(Resource, Default)]
struct TerrainMeshCache {
    by_chunk: BTreeMap<TerrainChunkCoord, Vec<Handle<Mesh>>>,
}

impl TerrainMeshCache {
    fn reset_for_world(&mut self, meshes: &mut Assets<Mesh>) {
        for handle in self.by_chunk.values().flatten() {
            let _removed = meshes.remove(handle.id());
        }
        self.by_chunk.clear();
    }

    fn replace_chunk(
        &mut self,
        chunk: TerrainChunkCoord,
        handles: Vec<Handle<Mesh>>,
        meshes: &mut Assets<Mesh>,
    ) {
        if let Some(retired) = self.by_chunk.insert(chunk, handles) {
            for handle in retired {
                let _removed = meshes.remove(handle.id());
            }
        }
    }
}

impl MaterialCache {
    fn reset_for_world(
        &mut self,
        materials: &mut Assets<StandardMaterial>,
        treatment: ReviewMaterialTreatment,
    ) {
        for (_substance, handle) in self.by_substance.drain(..) {
            let _removed = materials.remove(handle.id());
        }
        self.treatment = treatment;
    }

    fn get_or_create(
        &mut self,
        substance: SubstanceId,
        table: &SubstanceTable,
        treatment: ReviewMaterialTreatment,
        materials: &mut Assets<StandardMaterial>,
    ) -> Handle<StandardMaterial> {
        if self.treatment != treatment {
            self.reset_for_world(materials, treatment);
        }
        if let Some((_, handle)) = self.by_substance.iter().find(|(id, _)| *id == substance) {
            return handle.clone();
        }

        // Bright magenta makes an unknown id visibly distinct from a lighting fault.
        let color = table.get(substance).map_or((1.0, 0.0, 1.0), |s| s.color);
        let handle = materials.add(terrain_material(to_color(color), treatment));
        self.by_substance.push((substance, handle.clone()));
        handle
    }
}

fn terrain_material(color: Color, treatment: ReviewMaterialTreatment) -> StandardMaterial {
    let mut material = StandardMaterial::from(color);
    if treatment.applies_to_terrain() {
        material.perceptual_roughness = 1.0;
        material.metallic = 0.0;
    }
    material
}

/// Optional V3 exact-position consequences maintained after terrain edits.
#[derive(SystemParam)]
struct EditableSpatialConsequences<'w> {
    biome_regions: Option<ResMut<'w, BiomeRegions>>,
    blockers: Option<ResMut<'w, TraversalBlockers>>,
}

/// Presentation identity queried and repaired alongside one authoritative edit batch.
#[derive(SystemParam)]
struct EditableTerrainPresentation<'w, 's> {
    grids: Query<'w, 's, Entity, With<HexGrid>>,
    chunk_roots: Query<'w, 's, (Entity, &'static TerrainChunkRoot, Option<&'static ChildOf>)>,
    feature_roots: Query<'w, 's, (Entity, &'static GeneratedFeatureRoot, &'static ChildOf)>,
    next_screen: ResMut<'w, NextState<Screen>>,
}

/// Mutable terrain-impact resources grouped to stay within Bevy's function-system
/// parameter arity while keeping every authority explicit.
#[derive(SystemParam)]
struct TerrainMutation<'w> {
    edits: ResMut<'w, PendingTerrainEdits>,
    outcomes: MessageWriter<'w, TerrainImpactOutcome>,
    pending: ResMut<'w, PendingTerrainImpacts>,
    damage_state: ResMut<'w, TerrainDamageState>,
    damaged_voxels: ResMut<'w, DamagedVoxels>,
    snapshot_dirty: ResMut<'w, WorldSnapshotDirty>,
}

/// Claims direct edits outside the pausable mutation phase so they cannot age out.
fn collect_terrain_edits(
    mut edits: MessageReader<TerrainEdit>,
    mut pending: ResMut<PendingTerrainEdits>,
) {
    pending.0.extend(edits.read().cloned());
}

/// Reprojects an accepted cave prop using the feature renderer's exact origin convention.
fn project_cave_vegetation_cells(
    root: TilePos,
    rotation: HexObjectRotation,
    blueprint: &ObjectBlueprint,
) -> Option<BTreeSet<TilePos>> {
    let visual_origin_level = root.level.checked_add(1)?;
    let mut cells = BTreeSet::new();
    for placement in &blueprint.placements {
        let rotated = rotation.rotate_voxel(placement.position, blueprint.origin)?;
        let delta_q = rotated.q.checked_sub(blueprint.origin.q)?;
        let delta_r = rotated.r.checked_sub(blueprint.origin.r)?;
        let coord = HexCoord::from_axial(
            root.coord.x().checked_add(delta_q)?,
            root.coord.y().checked_add(delta_r)?,
        );
        let relative_level = rotated.level.checked_sub(blueprint.origin.level)?;
        let level = visual_origin_level.checked_add(relative_level)?;
        if !cells.insert(TilePos::new(coord, level)) {
            return None;
        }
    }
    (cells.len() == blueprint.placements.len()).then_some(cells)
}

/// Validates and claims every announced batch exactly once before map mutation.
///
/// This reader runs even while terrain is unavailable so malformed or early casts
/// receive an explicit rejection instead of aging out of Bevy's message buffers. The
/// accepted queue is drained after direct edits in the same ordered terrain phase.
fn collect_terrain_impacts(
    mut impacts: MessageReader<TerrainImpact>,
    mut pending: ResMut<PendingTerrainImpacts>,
    mut damage_state: ResMut<TerrainDamageState>,
    terrain_ready: Option<Res<TerrainReady>>,
    map: Option<Res<VoxelMap>>,
    substances: Option<Res<SubstanceTable>>,
    elements: Option<Res<ElementCatalog>>,
    damage_file: Option<Res<TerrainDamageFile>>,
    damage_table: Option<Res<TerrainDamageTable>>,
) {
    let coherent_damage_content = match (
        damage_file.as_deref(),
        damage_table.as_deref(),
        elements.as_deref(),
        substances.as_deref(),
    ) {
        (Some(file), Some(table), Some(elements), Some(substances)) => {
            table.matches_sources(file, elements, substances)
        }
        _ => false,
    };
    let terrain_available = terrain_ready.is_some() && map.is_some() && coherent_damage_content;

    for impact in impacts.read() {
        let rejection = if !damage_state.consume_batch(impact.batch) {
            Some(TerrainImpactRejection::ReusedBatch)
        } else if let Some(reason) = impact.structural_rejection() {
            Some(reason)
        } else if elements
            .as_deref()
            .is_some_and(|catalog| catalog.name(impact.element).is_none())
        {
            Some(TerrainImpactRejection::UnknownElement)
        } else if !terrain_available {
            Some(TerrainImpactRejection::TerrainUnavailable)
        } else {
            None
        };

        if let Some(reason) = rejection {
            pending.0.push(PendingTerrainImpact::Reject {
                batch: impact.batch,
                reason,
            });
        } else {
            pending.0.push(PendingTerrainImpact::Apply(impact.clone()));
        }
    }
}

fn rejected_terrain_outcome(
    batch: hex_core::TerrainBatchId,
    reason: TerrainImpactRejection,
) -> TerrainImpactOutcome {
    TerrainImpactOutcome {
        batch,
        result: TerrainImpactResult::Rejected(reason),
    }
}

/// Completes queued rejections when no mutable terrain world can run this frame.
fn reject_pending_impacts_without_world(
    mut pending: ResMut<PendingTerrainImpacts>,
    mut outcomes: MessageWriter<TerrainImpactOutcome>,
) {
    for decision in pending.0.drain(..) {
        let outcome = match decision {
            PendingTerrainImpact::Reject { batch, reason } => {
                rejected_terrain_outcome(batch, reason)
            }
            PendingTerrainImpact::Apply(impact) => {
                rejected_terrain_outcome(impact.batch, TerrainImpactRejection::TerrainUnavailable)
            }
        };
        outcomes.write(outcome);
    }
}

/// Applies direct terrain edits first, then atomically replaces affected chunk roots.
///
/// Partial health never enters this consequence path. Unchanged chunks and global
/// authored presentation entities retain their entity identities across an edit.
fn apply_terrain_changes(
    mut commands: Commands,
    mutation: TerrainMutation,
    mut map: ResMut<VoxelMap>,
    mut terrain_presentation: EditableTerrainPresentation,
    assets: Res<GameAssets>,
    mut presentation_assets: MapPresentationAssets,
    table: Res<SubstanceTable>,
    damage_table: Option<Res<TerrainDamageTable>>,
    settings: Res<MapSettings>,
    art_catalog: Option<Res<RuntimeArtCatalog>>,
    material_treatment: Option<Res<ReviewMaterialTreatment>>,
    edge_treatment: Option<Res<ReviewEdgeTreatment>>,
    mut special_regions: ResMut<SpecialMovementRegions>,
    mut interiors: Option<ResMut<InteriorRegions>>,
    mut spatial: EditableSpatialConsequences,
    mut presentation: Option<ResMut<MapPresentationProjection>>,
) {
    let TerrainMutation {
        mut edits,
        mut outcomes,
        mut pending,
        mut damage_state,
        mut damaged_voxels,
        mut snapshot_dirty,
    } = mutation;
    if edits.0.is_empty() && pending.0.is_empty() {
        return;
    }
    let (grid, existing_chunk_roots) = match validated_chunk_roots(
        &map,
        &terrain_presentation.grids,
        &terrain_presentation.chunk_roots,
    ) {
        Ok(topology) => topology,
        Err(error) => {
            fail_presentation_setup(&mut commands, &error);
            terrain_presentation.next_screen.set(Screen::Title);
            return;
        }
    };
    let mut changed = false;
    let mut changed_coords = BTreeSet::new();
    let mut snapshot_changed_coords = BTreeSet::new();
    for edit in edits.0.drain(..) {
        let semantic_projection_protected = presentation.as_deref().is_some_and(|projection| {
            projection.protects_liquid_edit(edit.pos())
                || projection.protects_feature_edit(edit.pos())
                || projection.protects_light_edit(edit.pos())
        });
        if apply_terrain_edit(&mut map, &table, &edit, semantic_projection_protected) {
            damage_state.forget_voxel(edit.pos(), &mut damaged_voxels);
            changed = true;
            changed_coords.insert(edit.pos().coord);
            snapshot_changed_coords.insert(edit.pos().coord);
            if let Some(interiors) = interiors.as_deref_mut() {
                // A replacement is new material, not part of the authored roof even
                // when it remains solid. Removing only this voxel keeps both original
                // fragments available for exact re-projection.
                interiors.remove_roof_voxel(edit.pos());
            }
        }
    }

    for decision in pending.0.drain(..) {
        let impact = match decision {
            PendingTerrainImpact::Apply(impact) => impact,
            PendingTerrainImpact::Reject { batch, reason } => {
                outcomes.write(rejected_terrain_outcome(batch, reason));
                continue;
            }
        };
        let Some(damage_table) = damage_table.as_deref() else {
            outcomes.write(rejected_terrain_outcome(
                impact.batch,
                TerrainImpactRejection::TerrainUnavailable,
            ));
            continue;
        };
        let resolved = damage_state.apply(
            impact,
            &mut map,
            &table,
            damage_table,
            &mut damaged_voxels,
            |position| {
                presentation.as_deref().is_some_and(|projection| {
                    projection.protects_liquid_edit(position)
                        || projection.protects_feature_edit(position)
                        || projection.protects_light_edit(position)
                })
            },
        );
        if let TerrainImpactResult::Applied(voxels) = &resolved.outcome.result {
            snapshot_changed_coords.extend(voxels.iter().filter_map(|voxel| {
                matches!(
                    voxel.disposition,
                    TerrainImpactDisposition::Damaged | TerrainImpactDisposition::Destroyed
                )
                .then_some(voxel.pos.coord)
            }));
        }
        for position in &resolved.destroyed {
            changed = true;
            changed_coords.insert(position.coord);
            if let Some(interiors) = interiors.as_deref_mut() {
                interiors.remove_roof_voxel(*position);
            }
        }
        outcomes.write(resolved.outcome);
    }

    snapshot_dirty.mark_changed(snapshot_changed_coords);

    if !changed {
        return;
    }

    if let Some(presentation) = presentation.as_deref_mut() {
        presentation.retain_features(|feature| match feature.kind {
            procedural_v3::FeatureKind::Tree => true,
            procedural_v3::FeatureKind::TallGrass => {
                if !changed_coords.contains(&feature.root.coord) {
                    return true;
                }
                let Some(column) = map.column(feature.root.coord) else {
                    return false;
                };
                TraversalProfile::WALKER.admits_surface(
                    table.is_solid(column.get(feature.root.level)),
                    column.headroom_above(feature.root.level.saturating_add(1)),
                )
            }
            procedural_v3::FeatureKind::CaveVegetation => {
                let Some(blueprint) = art_catalog
                    .as_deref()
                    .and_then(|catalog| catalog.object(&feature.object_id))
                else {
                    return false;
                };
                let Some(visual_cells) =
                    project_cave_vegetation_cells(feature.root, feature.rotation, blueprint)
                else {
                    return false;
                };
                if !changed_coords.contains(&feature.root.coord)
                    && visual_cells
                        .iter()
                        .all(|position| !changed_coords.contains(&position.coord))
                {
                    return true;
                }
                visual_cells.iter().all(|visual| {
                    let Some(column) = map.column(visual.coord) else {
                        return false;
                    };
                    TraversalProfile::WALKER.admits_surface(
                        table.is_solid(column.get(feature.root.level)),
                        column.headroom_above(feature.root.level.saturating_add(1)),
                    ) && map.get(*visual).is_air()
                })
            }
        });
    }

    special_regions.retain(|position, _| {
        if !changed_coords.contains(&position.coord) {
            return true;
        }
        let Some(column) = map.column(position.coord) else {
            return false;
        };
        TraversalProfile::WALKER.admits_surface(
            table.is_solid(column.get(position.level)),
            column.headroom_above(position.level.saturating_add(1)),
        )
    });
    if let Some(interiors) = interiors.as_deref_mut() {
        interiors.retain_surfaces(|position, _| {
            if !changed_coords.contains(&position.coord) {
                return true;
            }
            let Some(column) = map.column(position.coord) else {
                return false;
            };
            TraversalProfile::WALKER.admits_surface(
                table.is_solid(column.get(position.level)),
                column.headroom_above(position.level.saturating_add(1)),
            )
        });
        interiors.retain_roof_voxels(|position, _| {
            !changed_coords.contains(&position.coord) || table.is_solid(map.get(position))
        });
    }
    if let Some(biome_regions) = spatial.biome_regions.as_deref_mut() {
        reproject_biome_surfaces(
            &map,
            &table,
            &changed_coords,
            biome_regions,
            presentation.as_deref(),
        );
    }
    if let Some(blockers) = spatial.blockers.as_deref_mut() {
        retain_valid_blockers(&map, &table, &changed_coords, blockers);
    }

    let affected_chunks = changed_coords
        .iter()
        .copied()
        .map(terrain_chunk_coord)
        .collect::<BTreeSet<_>>();
    let _accepted_hex_mesh = assets.hex_tile.id();
    let mut replacements = Vec::with_capacity(affected_chunks.len());
    for chunk in affected_chunks.iter().copied() {
        match spawn_terrain_chunk(
            &mut commands,
            &mut presentation_assets.terrain_materials,
            &mut presentation_assets.terrain_meshes,
            &mut presentation_assets.materials,
            &mut presentation_assets.meshes,
            &map,
            &table,
            &settings,
            interiors.as_deref(),
            chunk,
            material_treatment.as_deref().copied().unwrap_or_default(),
            edge_treatment.as_deref().copied().unwrap_or_default(),
        ) {
            Ok(root) => replacements.push(root),
            Err(error) => {
                fail_presentation_setup(&mut commands, &error);
                terrain_presentation.next_screen.set(Screen::Title);
                return;
            }
        }
    }
    commands.entity(grid).add_children(&replacements);
    for chunk in &affected_chunks {
        if let Some(entity) = existing_chunk_roots.get(chunk) {
            commands.entity(*entity).despawn();
        }
    }
    for (entity, root, parent) in &terrain_presentation.feature_roots {
        if parent.parent() == grid
            && presentation
                .as_deref()
                .is_none_or(|projection| !projection.features().contains_key(&root.id))
        {
            commands.entity(entity).despawn();
        }
    }
}

fn validated_chunk_roots(
    map: &VoxelMap,
    grids: &Query<Entity, With<HexGrid>>,
    roots: &Query<(Entity, &TerrainChunkRoot, Option<&ChildOf>)>,
) -> Result<(Entity, BTreeMap<TerrainChunkCoord, Entity>), MapPresentationError> {
    let mut grids = grids.iter();
    let Some(grid) = grids.next() else {
        return Err(MapPresentationError::TerrainGridMissing);
    };
    if grids.next().is_some() {
        return Err(MapPresentationError::MultipleTerrainGrids);
    }

    let expected = map.chunk_coords().collect::<BTreeSet<_>>();
    let mut actual = BTreeMap::new();
    for (entity, root, parent) in roots {
        let Some(parent) = parent else {
            return Err(MapPresentationError::TerrainChunkTopology(format!(
                "chunk root [{},{}] is orphaned",
                root.q, root.r
            )));
        };
        if parent.parent() != grid {
            return Err(MapPresentationError::TerrainChunkTopology(format!(
                "chunk root [{},{}] belongs to another parent",
                root.q, root.r
            )));
        }
        let chunk = TerrainChunkCoord {
            q: root.q,
            r: root.r,
        };
        if !expected.contains(&chunk) {
            return Err(MapPresentationError::TerrainChunkTopology(format!(
                "grid owns unexpected chunk [{},{}]",
                chunk.q, chunk.r
            )));
        }
        if actual.insert(chunk, entity).is_some() {
            return Err(MapPresentationError::TerrainChunkTopology(format!(
                "grid owns duplicate chunk [{},{}]",
                chunk.q, chunk.r
            )));
        }
    }
    if actual.len() != expected.len() {
        let missing = expected
            .iter()
            .find(|chunk| !actual.contains_key(chunk))
            .copied();
        return Err(MapPresentationError::TerrainChunkTopology(
            missing.map_or_else(
                || "grid chunk count disagrees with resident voxel storage".to_owned(),
                |chunk| format!("grid is missing chunk [{},{}]", chunk.q, chunk.r),
            ),
        ));
    }
    Ok((grid, actual))
}

/// Rebuilds exact biome membership for every edited column.
///
/// A biome region belongs to the generated patch, not to one immutable top voxel.
/// Clearing a surface therefore transfers that region to newly exposed solid runs,
/// while placing terrain removes entries that became buried. Stacked surfaces retain
/// independent identities by inheriting the closest prior exact surface.
fn reproject_biome_surfaces(
    map: &VoxelMap,
    table: &SubstanceTable,
    changed_coords: &BTreeSet<HexCoord>,
    biome_regions: &mut BiomeRegions,
    presentation: Option<&MapPresentationProjection>,
) {
    for coord in changed_coords {
        let previous: Vec<_> = biome_regions
            .iter()
            .filter(|(position, _region)| position.coord == *coord)
            .collect();
        if previous.is_empty() {
            continue;
        }

        for (position, _region) in &previous {
            let _removed = biome_regions.remove(*position);
        }

        let Some(column) = map.column(*coord) else {
            continue;
        };
        for level in 0..column.top() {
            let position = TilePos::new(*coord, level);
            if !table.is_solid(column.get(level)) {
                continue;
            }
            let above = TilePos::new(*coord, level.saturating_add(1));
            let exposed_to_air = column.get(above.level).is_air();
            let supports_authored_liquid =
                presentation.is_some_and(|projection| projection.contains_liquid(above));
            if !exposed_to_air && !supports_authored_liquid {
                continue;
            }

            let inherited = previous
                .iter()
                .min_by_key(|(source, region)| {
                    (source.level.abs_diff(level), source.level, *region)
                })
                .map(|(_source, region)| *region);
            if let Some(region) = inherited {
                let _replaced = biome_regions.insert(position, region);
            }
        }
    }
}

/// Removes feature blockers whose exact footing was destroyed or buried.
///
/// Newly exposed surfaces remain unblocked: a blocker represents a generated
/// feature at one exact `TilePos`, not a property inherited by the whole column.
fn retain_valid_blockers(
    map: &VoxelMap,
    table: &SubstanceTable,
    changed_coords: &BTreeSet<HexCoord>,
    blockers: &mut TraversalBlockers,
) {
    let removed: Vec<_> = blockers
        .iter()
        .filter(|position| changed_coords.contains(&position.coord))
        .filter(|position| {
            let Some(column) = map.column(position.coord) else {
                return true;
            };
            !TraversalProfile::WALKER.admits_surface(
                table.is_solid(column.get(position.level)),
                column.headroom_above(position.level.saturating_add(1)),
            )
        })
        .collect();
    for position in removed {
        let _removed = blockers.remove(position);
    }
}

/// Applies a changed edit unless it is below the floor, non-diggable, or liquid-protected.
fn apply_terrain_edit(
    map: &mut VoxelMap,
    table: &SubstanceTable,
    edit: &TerrainEdit,
    liquid_protected: bool,
) -> bool {
    let pos = edit.pos();
    if pos.level < 0 {
        return false;
    }

    let current = map.get(pos);
    let replacement = match *edit {
        TerrainEdit::Set { substance, .. } => substance,
        TerrainEdit::Clear { .. } => SubstanceId::AIR,
    };

    if current == replacement
        || liquid_protected
        || (!current.is_air() && !table.is_diggable(current))
    {
        return false;
    }

    map.set(pos, replacement);
    true
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use bevy::platform::collections::HashMap;
    use hex_assets::{ArtPalette, PaletteSwatch, SrgbColor, Substance, SubstanceFile, SwatchId};

    use super::*;

    fn spatial_test_table() -> SubstanceTable {
        spatial_test_table_with_color(0.5, 0.5, 0.5)
    }

    fn spatial_test_table_with_color(red: f32, green: f32, blue: f32) -> SubstanceTable {
        let swatch_id = SwatchId::new("test/gray").expect("the fixture swatch id should be valid");
        let swatch = PaletteSwatch::new(
            "Test Gray",
            SrgbColor::new(red, green, blue).expect("the fixture color should be valid"),
            BTreeSet::from(["test".to_owned()]),
        )
        .expect("the fixture swatch should be valid");
        let palette = ArtPalette::new(BTreeMap::from([(swatch_id.clone(), swatch)]))
            .expect("the fixture palette should be valid");
        let substances = HashMap::from_iter([
            ("air".to_owned(), Substance::invisible(false, false)),
            (
                "stone".to_owned(),
                Substance::from_swatch(swatch_id.clone(), true, true),
            ),
            (
                "water".to_owned(),
                Substance::from_swatch(swatch_id, false, true),
            ),
        ]);
        SubstanceTable::from_file(&SubstanceFile { substances }, &palette)
            .expect("the fixture substances should resolve through their palette")
    }

    #[test]
    fn full_world_material_refresh_drops_stale_palette_handles() {
        let first_table = spatial_test_table_with_color(0.25, 0.5, 0.75);
        let second_table = spatial_test_table_with_color(0.8, 0.2, 0.1);
        let stone = first_table.id("stone").expect("stone fixture");
        assert_eq!(second_table.id("stone"), Some(stone));

        let mut cache = MaterialCache::default();
        let mut materials = Assets::<StandardMaterial>::default();
        let first = cache.get_or_create(
            stone,
            &first_table,
            ReviewMaterialTreatment::Current,
            &mut materials,
        );
        let first_color = materials
            .get(&first)
            .expect("the first cached material should exist")
            .base_color;

        cache.reset_for_world(&mut materials, ReviewMaterialTreatment::Current);
        assert!(
            materials.get(&first).is_none(),
            "the retired world's material remained resident"
        );

        let second = cache.get_or_create(
            stone,
            &second_table,
            ReviewMaterialTreatment::Current,
            &mut materials,
        );
        let second_color = materials
            .get(&second)
            .expect("the refreshed material should exist")
            .base_color;
        assert_ne!(
            first_color, second_color,
            "the stable substance id reused its stale palette colour"
        );
    }

    #[test]
    fn review_material_treatment_changes_only_terrain_roughness() {
        let expected = StandardMaterial::from(Color::srgb(0.2, 0.4, 0.7));
        let current =
            terrain_material(Color::srgb(0.2, 0.4, 0.7), ReviewMaterialTreatment::Current);
        assert_eq!(current.base_color, expected.base_color);
        assert_eq!(current.perceptual_roughness, expected.perceptual_roughness);
        assert_eq!(current.metallic, expected.metallic);

        for treatment in [
            ReviewMaterialTreatment::MatteTerrain,
            ReviewMaterialTreatment::UnifiedMatte,
        ] {
            let matte = terrain_material(Color::srgb(0.2, 0.4, 0.7), treatment);
            assert_eq!(matte.base_color, expected.base_color);
            assert_eq!(matte.perceptual_roughness, 1.0);
            assert_eq!(matte.metallic, 0.0);
        }
    }

    #[test]
    fn terrain_material_cache_replaces_assets_when_review_treatment_changes() {
        let table = spatial_test_table_with_color(0.25, 0.5, 0.75);
        let stone = table.id("stone").expect("stone fixture");
        let mut cache = MaterialCache::default();
        let mut materials = Assets::<StandardMaterial>::default();

        let current = cache.get_or_create(
            stone,
            &table,
            ReviewMaterialTreatment::Current,
            &mut materials,
        );
        let matte = cache.get_or_create(
            stone,
            &table,
            ReviewMaterialTreatment::MatteTerrain,
            &mut materials,
        );

        assert!(materials.get(&current).is_none());
        assert_eq!(
            materials
                .get(&matte)
                .expect("matte replacement remains resident")
                .perceptual_roughness,
            1.0,
        );
    }

    #[test]
    fn micro_bevel_treatment_changes_only_terrain_normals_and_scales_exactly() {
        let coord = HexCoord::ORIGIN;
        let substance = SubstanceId(1);
        let projected = BTreeMap::from([(
            coord,
            vec![ProjectedRun {
                run: SubstanceRun {
                    bottom: 0,
                    top: 1,
                    substance,
                },
                cutaway: None,
            }],
        )]);
        let pending = PendingTerrainRun {
            entity: Entity::from_raw_u32(1).expect("fixture entity"),
            position: TilePos::new(coord, 0),
            span: HexSpan::new(0.0, 0.4),
            bottom: 0,
            top: 1,
            cutaway: None,
        };
        let current = combined_terrain_mesh_with_edge(
            &[pending],
            &projected,
            0.4,
            ReviewEdgeTreatment::Current,
        )
        .expect("current terrain mesh should build");
        let subtle = combined_terrain_mesh_with_edge(
            &[pending],
            &projected,
            0.4,
            ReviewEdgeTreatment::MicroBevel04,
        )
        .expect("0.04 terrain mesh should build");
        let strong = combined_terrain_mesh_with_edge(
            &[pending],
            &projected,
            0.4,
            ReviewEdgeTreatment::MicroBevel08,
        )
        .expect("0.08 terrain mesh should build");

        let float3 = |mesh: &Mesh, attribute| {
            let Some(bevy::mesh::VertexAttributeValues::Float32x3(values)) =
                mesh.attribute(attribute)
            else {
                unreachable!("terrain fixture needs Float32x3 attributes")
            };
            values.clone()
        };
        let positions = float3(&current, Mesh::ATTRIBUTE_POSITION);
        assert_eq!(float3(&subtle, Mesh::ATTRIBUTE_POSITION), positions);
        assert_eq!(float3(&strong, Mesh::ATTRIBUTE_POSITION), positions);
        let indices = |mesh: &Mesh| {
            mesh.indices()
                .expect("terrain fixture remains indexed")
                .iter()
                .collect::<Vec<_>>()
        };
        assert_eq!(indices(&subtle), indices(&current));
        assert_eq!(indices(&strong), indices(&current));

        let shipped = float3(&current, Mesh::ATTRIBUTE_NORMAL);
        let subtle_normals = float3(&subtle, Mesh::ATTRIBUTE_NORMAL);
        let strong_normals = float3(&strong, Mesh::ATTRIBUTE_NORMAL);
        let delta = |normals: &[[f32; 3]]| {
            normals
                .iter()
                .zip(&shipped)
                .map(|(actual, baseline)| Vec3::from(*actual).distance(Vec3::from(*baseline)))
                .sum::<f32>()
        };
        assert!(delta(&subtle_normals) > 0.0);
        assert!(delta(&strong_normals) > delta(&subtle_normals));
        for normal in subtle_normals.iter().chain(&strong_normals) {
            assert!((Vec3::from(*normal).length() - 1.0).abs() < 1.0e-5);
        }
    }

    #[test]
    fn geometric_bevel_treatment_adds_finite_bounded_chamfers_with_exact_cap_insets() {
        let coord = HexCoord::ORIGIN;
        let substance = SubstanceId(1);
        let projected = BTreeMap::from([(
            coord,
            vec![ProjectedRun {
                run: SubstanceRun {
                    bottom: 1,
                    top: 2,
                    substance,
                },
                cutaway: None,
            }],
        )]);
        let pending = PendingTerrainRun {
            entity: Entity::from_raw_u32(1).expect("fixture entity"),
            position: TilePos::new(coord, 1),
            span: HexSpan::new(0.4, 0.8),
            bottom: 1,
            top: 2,
            cutaway: None,
        };
        let current = combined_terrain_mesh_with_edge(
            &[pending],
            &projected,
            0.4,
            ReviewEdgeTreatment::Current,
        )
        .expect("current terrain mesh should build");
        let subtle = combined_terrain_mesh_with_edge(
            &[pending],
            &projected,
            0.4,
            ReviewEdgeTreatment::GeometricBevel04,
        )
        .expect("0.04 geometric terrain mesh should build");
        let strong = combined_terrain_mesh_with_edge(
            &[pending],
            &projected,
            0.4,
            ReviewEdgeTreatment::GeometricBevel08,
        )
        .expect("0.08 geometric terrain mesh should build");

        let float3 = |mesh: &Mesh, attribute| {
            let Some(bevy::mesh::VertexAttributeValues::Float32x3(values)) =
                mesh.attribute(attribute)
            else {
                unreachable!("terrain fixture needs Float32x3 attributes")
            };
            values.clone()
        };
        let bounds = |positions: &[[f32; 3]]| {
            positions.iter().fold(
                ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]),
                |(mut minimum, mut maximum), position| {
                    for (value, (minimum, maximum)) in position
                        .iter()
                        .zip(minimum.iter_mut().zip(maximum.iter_mut()))
                    {
                        *minimum = (*minimum).min(*value);
                        *maximum = (*maximum).max(*value);
                    }
                    (minimum, maximum)
                },
            )
        };
        let current_positions = float3(&current, Mesh::ATTRIBUTE_POSITION);
        let current_bounds = bounds(&current_positions);
        let current_indices = current
            .indices()
            .expect("current terrain fixture remains indexed")
            .len();

        for (mesh, expected_radius) in [
            (&subtle, hex_core::config::HEX_CIRCUMRADIUS - 0.04),
            (&strong, hex_core::config::HEX_CIRCUMRADIUS - 0.08),
        ] {
            let positions = float3(mesh, Mesh::ATTRIBUTE_POSITION);
            let normals = float3(mesh, Mesh::ATTRIBUTE_NORMAL);
            assert!(
                positions
                    .iter()
                    .flatten()
                    .all(|component| component.is_finite()),
                "geometric bevel emitted a non-finite position"
            );
            assert!(
                normals
                    .iter()
                    .flatten()
                    .all(|component| component.is_finite()),
                "geometric bevel emitted a non-finite normal"
            );
            for (position, normal) in positions.iter().zip(&normals) {
                let [x, _, z] = *position;
                let normal = Vec3::from(*normal);
                assert!((normal.length() - 1.0).abs() < 1.0e-5);
                assert!(
                    Vec2::new(normal.x, normal.z).dot(Vec2::new(x, z)) >= -1.0e-6,
                    "geometric bevel face normal points into the voxel"
                );
            }
            assert_eq!(bounds(&positions), current_bounds);
            assert!(
                mesh.indices()
                    .expect("geometric terrain fixture remains indexed")
                    .len()
                    > current_indices
            );

            let top_radius = positions
                .iter()
                .filter_map(|position| {
                    let [x, y, z] = *position;
                    ((y - 0.8).abs() < 1.0e-6).then_some(Vec2::new(x, z).length())
                })
                .fold(0.0_f32, f32::max);
            let bottom_radius = positions
                .iter()
                .filter_map(|position| {
                    let [x, y, z] = *position;
                    ((y - 0.4).abs() < 1.0e-6).then_some(Vec2::new(x, z).length())
                })
                .fold(0.0_f32, f32::max);
            assert!((top_radius - expected_radius).abs() < 1.0e-6);
            assert!((bottom_radius - expected_radius).abs() < 1.0e-6);

            let indices = mesh
                .indices()
                .expect("geometric terrain fixture remains indexed")
                .iter()
                .collect::<Vec<_>>();
            for triangle in indices.chunks_exact(3) {
                let [first, second, third] = triangle else {
                    unreachable!("triangle chunk is exact")
                };
                let first = Vec3::from(
                    *positions
                        .get(*first)
                        .expect("first bevel index stays in bounds"),
                );
                let second = Vec3::from(
                    *positions
                        .get(*second)
                        .expect("second bevel index stays in bounds"),
                );
                let third = Vec3::from(
                    *positions
                        .get(*third)
                        .expect("third bevel index stays in bounds"),
                );
                assert!(
                    (second - first).cross(third - first).length_squared() > f32::EPSILON,
                    "geometric bevel emitted a degenerate triangle"
                );
            }
        }
    }

    #[test]
    fn geometric_bevel_rejects_invalid_vertical_scale() {
        assert!(TerrainRawMesh::with_edge_treatment(
            ReviewEdgeTreatment::GeometricBevel04,
            f32::NAN,
        )
        .is_err());
        assert!(
            TerrainRawMesh::with_edge_treatment(ReviewEdgeTreatment::GeometricBevel08, 0.0)
                .is_err()
        );
        assert!(
            TerrainRawMesh::with_edge_treatment(ReviewEdgeTreatment::Current, f32::NAN).is_ok(),
            "the shipped path must retain its prior validation boundary"
        );
    }

    #[test]
    fn projected_runs_split_at_every_exact_roof_boundary() {
        let coord = HexCoord::ORIGIN;
        let stone = SubstanceId(1);
        let column = Column::filled(stone, 6);
        let lower = InteriorRegionId(2);
        let upper = InteriorRegionId(7);
        let mut interiors = InteriorRegions::new();
        for level in 1..3 {
            interiors.insert_roof_voxel(TilePos::new(coord, level), lower);
        }
        interiors.insert_roof_voxel(TilePos::new(coord, 4), upper);

        assert_eq!(
            projected_runs(coord, &column, Some(&interiors)),
            vec![
                ProjectedRun {
                    run: SubstanceRun {
                        bottom: 0,
                        top: 1,
                        substance: stone,
                    },
                    cutaway: None,
                },
                ProjectedRun {
                    run: SubstanceRun {
                        bottom: 1,
                        top: 3,
                        substance: stone,
                    },
                    cutaway: Some(lower),
                },
                ProjectedRun {
                    run: SubstanceRun {
                        bottom: 3,
                        top: 4,
                        substance: stone,
                    },
                    cutaway: None,
                },
                ProjectedRun {
                    run: SubstanceRun {
                        bottom: 4,
                        top: 5,
                        substance: stone,
                    },
                    cutaway: Some(upper),
                },
                ProjectedRun {
                    run: SubstanceRun {
                        bottom: 5,
                        top: 6,
                        substance: stone,
                    },
                    cutaway: None,
                },
            ]
        );
    }

    #[test]
    fn combined_mesh_culls_shared_sides_but_keeps_cutaway_boundaries() {
        let origin = HexCoord::ORIGIN;
        let [east, _south_east, _south_west, west_across_chunk_seam, _north_west, _north_east] =
            origin.neighbors();
        let substance = SubstanceId(1);
        let current = ProjectedRun {
            run: SubstanceRun {
                bottom: 0,
                top: 1,
                substance,
            },
            cutaway: None,
        };
        let pending = PendingTerrainRun {
            entity: Entity::from_raw_u32(1).expect("fixture entity"),
            position: TilePos::new(origin, 0),
            span: HexSpan::new(0.0, 1.0),
            bottom: 0,
            top: 1,
            cutaway: None,
        };
        let same_owner = BTreeMap::from([(origin, vec![current]), (east, vec![current])]);
        let culled = combined_terrain_mesh(&[pending], &same_owner, 1.0)
            .expect("same-owner neighbours should mesh");
        assert_eq!(culled.count_vertices(), 7 + 5 * 4);
        assert_eq!(
            culled.indices().expect("indexed terrain mesh").len(),
            18 + 5 * 6
        );

        let other_region = ProjectedRun {
            cutaway: Some(InteriorRegionId(9)),
            ..current
        };
        let different_owner = BTreeMap::from([(origin, vec![current]), (east, vec![other_region])]);
        let retained = combined_terrain_mesh(&[pending], &different_owner, 1.0)
            .expect("different cutaway owners should retain their boundary");
        assert_eq!(retained.count_vertices(), 7 + 6 * 4);
        assert_eq!(
            retained.indices().expect("indexed terrain mesh").len(),
            18 + 6 * 6
        );

        let cross_chunk = BTreeMap::from([
            (origin, vec![current]),
            (west_across_chunk_seam, vec![current]),
        ]);
        let retained_seam = combined_terrain_mesh(&[pending], &cross_chunk, 1.0)
            .expect("chunk seam walls should remain independent");
        assert_eq!(retained_seam.count_vertices(), 7 + 6 * 4);
        assert_eq!(
            retained_seam.indices().expect("indexed terrain mesh").len(),
            18 + 6 * 6
        );
    }

    #[test]
    fn combined_mesh_side_order_matches_canonical_hex_neighbours() {
        let origin = HexCoord::ORIGIN;
        let centre = origin.to_world(0.0);
        let [north, north_west, _south_west, _south, _south_east, _north_east] =
            terrain_hex_corners();
        let inradius = 0.5 * hex_core::config::HEX_SMALL_DIAMETER;
        for (side, (neighbour, ([first, second], normal))) in origin
            .neighbors()
            .into_iter()
            .zip(terrain_hex_sides())
            .enumerate()
        {
            let direction = (neighbour.to_world(0.0) - centre).normalize();
            assert!(
                normal.dot(direction) > 1.0 - 1.0e-5,
                "side {side} normal {normal:?} disagrees with neighbour {neighbour:?}"
            );
            let midpoint = (first + second) * 0.5;
            assert!(
                (midpoint.dot(normal) - inradius).abs() < 1.0e-5,
                "side {side} corner pair does not lie on its outward face"
            );
            assert!(
                (second - first).cross(Vec3::Y).dot(normal) > 0.0,
                "side {side} triangle winding points inward"
            );
        }
        assert!(
            north.cross(north_west).dot(Vec3::Y) > 0.0,
            "top-cap triangle winding points downward"
        );
    }

    #[test]
    fn biome_membership_follows_buried_and_reexposed_surfaces() {
        let table = spatial_test_table();
        let stone = table.id("stone").expect("stone fixture");
        let coord = HexCoord::ORIGIN;
        let changed = BTreeSet::from([coord]);
        let region = hex_core::BiomeRegionId(7);
        let mut map = VoxelMap::new();
        map.insert_column(coord, Column::filled(stone, 3));
        let mut regions = BiomeRegions::new();
        let _previous = regions.insert(TilePos::new(coord, 2), region);

        map.set(TilePos::new(coord, 3), stone);
        reproject_biome_surfaces(&map, &table, &changed, &mut regions, None);
        assert_eq!(regions.get(TilePos::new(coord, 2)), None);
        assert_eq!(regions.get(TilePos::new(coord, 3)), Some(region));

        map.set(TilePos::new(coord, 3), SubstanceId::AIR);
        reproject_biome_surfaces(&map, &table, &changed, &mut regions, None);
        assert_eq!(regions.get(TilePos::new(coord, 3)), None);
        assert_eq!(regions.get(TilePos::new(coord, 2)), Some(region));
    }

    #[test]
    fn stacked_biome_surfaces_inherit_the_nearest_exact_region() {
        let table = spatial_test_table();
        let stone = table.id("stone").expect("stone fixture");
        let coord = HexCoord::ORIGIN;
        let changed = BTreeSet::from([coord]);
        let mut column = Column::filled(stone, 3);
        column.set(5, stone);
        let mut map = VoxelMap::new();
        map.insert_column(coord, column);
        let lower_region = hex_core::BiomeRegionId(2);
        let upper_region = hex_core::BiomeRegionId(9);
        let mut regions = BiomeRegions::new();
        let _previous = regions.insert(TilePos::new(coord, 2), lower_region);
        let _previous = regions.insert(TilePos::new(coord, 5), upper_region);

        map.set(TilePos::new(coord, 2), SubstanceId::AIR);
        reproject_biome_surfaces(&map, &table, &changed, &mut regions, None);

        assert_eq!(
            regions.get(TilePos::new(coord, 1)),
            Some(lower_region),
            "the newly exposed lower run inherits its own stacked region"
        );
        assert_eq!(
            regions.get(TilePos::new(coord, 5)),
            Some(upper_region),
            "the independent upper surface retains its exact region"
        );
    }

    #[test]
    fn feature_blockers_are_removed_when_their_footing_is_buried() {
        let table = spatial_test_table();
        let stone = table.id("stone").expect("stone fixture");
        let coord = HexCoord::ORIGIN;
        let root = TilePos::new(coord, 2);
        let mut map = VoxelMap::new();
        map.insert_column(coord, Column::filled(stone, 4));
        let mut blockers = TraversalBlockers::new();
        assert!(blockers.insert(root));

        retain_valid_blockers(&map, &table, &BTreeSet::from([coord]), &mut blockers);

        assert!(!blockers.contains(root));
    }
}
