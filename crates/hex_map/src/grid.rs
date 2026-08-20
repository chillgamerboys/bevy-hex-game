//! Builds the voxel world, and turns it into tile entities.
//!
//! Storage and generation are private to `hex_map`; rendered terrain reaches other
//! crates as entities carrying [`HexTile`](hex_core::HexTile),
//! [`HexCoord`](hex_core::HexCoord), a surface [`TilePos`](hex_core::TilePos),
//! [`RunBottom`](hex_core::RunBottom), [`HexSpan`](hex_core::HexSpan),
//! [`SubstanceId`](hex_core::SubstanceId), and [`Headroom`](hex_core::Headroom).
//! The substance table itself is shared through `hex_assets` because gameplay also
//! reads its behavior flags.
//!
//! Keeping that boundary narrow is what lets the map be rebuilt without touching
//! gameplay. A richer map means producing different voxels in the terrain builder;
//! it does not change what a tile *is* to anyone else.

use std::collections::BTreeSet;
use std::fmt;

use bevy::{ecs::system::SystemParam, prelude::*};

use hex_assets::{
    to_color, ElementCatalog, GameAssets, HexObjectRotation, ObjectBlueprint, RuntimeArtCatalog,
    SubstanceTable, TerrainDamageFile, TerrainDamageTable,
};
use hex_core::{
    AuthoritativeSystems, BiomeRegions, CutawayOccluder, DamagedVoxels, GameplayLight,
    GameplaySetup, GameplaySetupFailure, Headroom, HexCoord, HexGrid, HexSpan, HexTile,
    InteriorRegionId, InteriorRegions, MapAnchorId, MapAnchors, MapViewHint, PerceptionSystems,
    PresentationOcclusion, ResolvedMapSeed, RunBottom, Screen, SimulationRole,
    SpecialMovementRegions, SubstanceId, TerrainEdit, TerrainImpact, TerrainImpactDisposition,
    TerrainImpactOutcome, TerrainImpactRejection, TerrainImpactResult, TerrainReady,
    TerrainSystems, TilePos, TraversalBlockers, TraversalProfile, TreeOccluder,
};
use hex_multiplayer::AuthorityBoundary;

use crate::crystal_render::{self, CrystalPresentationError};
use crate::feature_render::{self, FeaturePresentationError};
use crate::liquid_render::{self, LiquidMaterial, LiquidPresentationError, LiquidVisualTime};
use crate::procedural;
use crate::procedural_v2;
use crate::procedural_v3;
use crate::procedural_v3::MapPresentationProjection;
use crate::settings::{MapSettings, TerrainSettings};
use crate::terrain::{build_non_procedural_map, TerrainPalette};
use crate::terrain_damage::TerrainDamageState;
use crate::voxel::{runs, Column, SubstanceRun, VoxelMap};
use crate::world_snapshot::{
    apply_world_delta_v1, export_from_parts, prepare_world_snapshot_v1,
    CampaignWorldRestoreOutcomeV2, CampaignWorldRestoreRefusalV2, CampaignWorldRestoreResultV2,
    CurrentWorldSnapshotV1, PendingCampaignWorldSnapshotV2, PreparedWorldSnapshotV1,
    WorldExportParts, WorldReplicationOutcomeV1, WorldReplicationRefusalV1,
    WorldReplicationRequestV1, WorldReplicationResultV1, WorldReplicationStateV1,
};
use crate::{
    CavesReportMetrics, DeepForestReportMetrics, ForestReportMetrics, FortReportMetrics,
    GenerationReport, MacroMetrics, MountainRangeMetrics, PrairieReportMetrics,
    ProceduralRecipeMetrics, Ring19Metrics, Ring7Metrics, VolcanoReportMetrics,
    WaterfallReportMetrics,
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

/// Whether map-owned truth changed since the current reconnect cache was published.
#[derive(Resource, Debug, Default)]
struct WorldSnapshotDirty(bool);

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

const MAX_WORLD_REPLICATION_REQUESTS_PER_UPDATE: usize = 64;

/// Registers world construction and tile spawning.
pub fn plugin(app: &mut App) {
    liquid_render::plugin(app);
    app.register_type::<HexCoord>()
        .register_type::<HexGrid>()
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
            spawn_grid
                .in_set(GameplaySetup::Terrain)
                .run_if(resource_exists::<TerrainReady>),
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
            OnEnter(Screen::Gameplay),
            publish_current_world_snapshot
                .in_set(GameplaySetup::Terrain)
                .after(spawn_grid)
                .run_if(resource_exists::<TerrainReady>),
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
    snapshot_dirty.0 = true;
    replication_state.set_last_applied_sequence(None);
    commands.remove_resource::<GameplaySetupFailure>();
    commands.remove_resource::<TerrainReady>();
    commands.remove_resource::<VoxelMap>();
    commands.remove_resource::<GenerationReport>();
    commands.remove_resource::<MapPresentationProjection>();
    liquid_render::clear_material_cache(&mut commands);
    commands.remove_resource::<MapAnchors>();
    commands.remove_resource::<SpecialMovementRegions>();
    commands.remove_resource::<InteriorRegions>();
    commands.remove_resource::<TraversalBlockers>();
    commands.remove_resource::<BiomeRegions>();
    commands.remove_resource::<MapViewHint>();
    commands.remove_resource::<CurrentWorldSnapshotV1>();
    commands.remove_resource::<PendingSnapshotGridBuild>();
    commands.remove_resource::<PendingCampaignWorldPublication>();
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
        commands.insert_resource(SpecialMovementRegions::new());
        commands.insert_resource(InteriorRegions::new());
        commands.insert_resource(TerrainReady);
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
                commands.insert_resource(TerrainReady);
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
            commands.insert_resource(generated.special_regions);
            commands.insert_resource(generated.interiors);
            commands.insert_resource(generated.view_hint);
            commands.insert_resource(generated.report);
            commands.insert_resource(TerrainReady);
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
            commands.insert_resource(generated.special_regions);
            commands.insert_resource(generated.interiors);
            commands.insert_resource(generated.blockers);
            commands.insert_resource(generated.biome_regions);
            commands.insert_resource(generated.view_hint);
            commands.insert_resource(generated.presentation);
            commands.insert_resource(generated.report);
            commands.insert_resource(TerrainReady);
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
    snapshot_dirty.0 = false;
    commands.insert_resource(map);
    commands.insert_resource(anchors);
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
    commands.insert_resource(TerrainReady);
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
    snapshot_dirty.0 = false;
    replication_state.set_last_applied_sequence(None);
    commands.remove_resource::<VoxelMap>();
    commands.remove_resource::<MapAnchors>();
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
    commands.remove_resource::<CampaignWorldRestoreResultV2>();
    liquid_render::clear_material_cache(&mut commands);
    commands.remove_resource::<TerrainReady>();
}

/// Spawns one entity per contiguous run of substance.
///
/// **Voxel storage does not mean voxel rendering.** One entity per voxel at radius 20
/// with bedrock depth would be tens of thousands; merging vertical runs of the same
/// substance keeps it to a handful per column. It is also why targeting has to be
/// positional — a voxel inside a run has no entity of its own.
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
    pending_campaign: Option<Res<PendingCampaignWorldPublication>>,
    mut damage_state: ResMut<TerrainDamageState>,
    mut damaged_voxels: ResMut<DamagedVoxels>,
) {
    let built = build_grid(
        &mut commands,
        &assets,
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
    );
    match built {
        Ok(()) => {
            if let Some(pending_campaign) = pending_campaign {
                commands.insert_resource(CampaignWorldRestoreResultV2 {
                    outcome: CampaignWorldRestoreOutcomeV2::Applied {
                        public_fingerprint: pending_campaign.public_fingerprint,
                    },
                });
                commands.remove_resource::<PendingCampaignWorldPublication>();
            }
        }
        Err(error) => {
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

fn discard_staged_campaign_world(
    commands: &mut Commands,
    damage_state: &mut TerrainDamageState,
    damaged_voxels: &mut DamagedVoxels,
) {
    damage_state.reset(damaged_voxels);
    commands.remove_resource::<VoxelMap>();
    commands.remove_resource::<MapAnchors>();
    commands.remove_resource::<SpecialMovementRegions>();
    commands.remove_resource::<InteriorRegions>();
    commands.remove_resource::<TraversalBlockers>();
    commands.remove_resource::<BiomeRegions>();
    commands.remove_resource::<MapViewHint>();
    commands.remove_resource::<MapPresentationProjection>();
    commands.remove_resource::<CurrentWorldSnapshotV1>();
    commands.remove_resource::<TerrainReady>();
    liquid_render::clear_material_cache(commands);
}

/// Spawns the grid entities. Shared by first construction and by rebuilds after an
/// edit, so the two cannot drift apart.
fn build_grid(
    commands: &mut Commands,
    assets: &GameAssets,
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
) -> Result<(), MapPresentationError> {
    let mesh = assets.hex_tile.clone();
    let mut palette_materials = MaterialCache::default();
    // Crystal asset resolution happens before any presentation entities are
    // queued, so a missing or incompatible dependency cannot leave a partial map.
    let prepared_crystals =
        crystal_render::prepare_presentations(settings.level_height, presentation, art_catalog)
            .map_err(MapPresentationError::Crystal)?;
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
    children.extend(crystal_render::spawn_prepared(commands, prepared_crystals));
    children.extend(spawn_gameplay_lights(commands, presentation));

    for (coord, column) in map.columns() {
        for projected in projected_runs(coord, column, interiors) {
            let run = projected.run;
            let material = palette_materials.get_or_create(run.substance, table, materials);
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
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material),
                Transform {
                    translation: coord.to_world(span.centre()),
                    scale: Vec3::new(1., span.height(), 1.),
                    ..default()
                },
                // A roof run can be hidden independently of the grid that owns it.
                Visibility::Inherited,
                Name::new("HexTile"),
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
            children.push(tile.id());
        }
    }

    commands
        .spawn((
            Transform::default(),
            Visibility::default(),
            Name::new("HexGrid"),
            HexGrid,
        ))
        .add_children(&children);
    Ok(())
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
    SnapshotResourcesMissing,
}

#[derive(SystemParam)]
struct MapPresentationAssets<'w> {
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
    fn export(&self) -> Result<hex_multiplayer::WorldSnapshotV1, crate::WorldSnapshotError> {
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
        export_from_parts(WorldExportParts {
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
}

fn publish_current_world_snapshot(
    mut commands: Commands,
    sources: WorldSnapshotSources,
    mut dirty: ResMut<WorldSnapshotDirty>,
    mut next_screen: ResMut<NextState<Screen>>,
) {
    if !dirty.0 {
        return;
    }
    match sources.export() {
        Ok(snapshot) => {
            commands.insert_resource(CurrentWorldSnapshotV1::new(snapshot));
            dirty.0 = false;
        }
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
    runtime.dirty.0 = false;
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
    mut replication_state: ResMut<WorldReplicationStateV1>,
) {
    let Some(pending) = pending else {
        return;
    };
    let built = match (map.as_deref(), table.as_deref(), settings.as_deref()) {
        (Some(map), Some(table), Some(settings)) => build_grid(
            &mut commands,
            &assets,
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
            Self::SnapshotResourcesMissing => None,
        }
    }
}

/// One material run split further wherever exact cutaway membership changes.
///
/// Rendered runs are disposable projections. Keeping cutaway ownership on exact
/// voxels lets this rebuild both fragments after digging through a roof and prevents
/// a replacement material from inheriting the old run's component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProjectedRun {
    run: SubstanceRun,
    cutaway: Option<InteriorRegionId>,
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
#[derive(Default)]
struct MaterialCache {
    by_substance: Vec<(SubstanceId, Handle<StandardMaterial>)>,
}

impl MaterialCache {
    fn get_or_create(
        &mut self,
        substance: SubstanceId,
        table: &SubstanceTable,
        materials: &mut Assets<StandardMaterial>,
    ) -> Handle<StandardMaterial> {
        if let Some((_, handle)) = self.by_substance.iter().find(|(id, _)| *id == substance) {
            return handle.clone();
        }

        // Bright magenta makes an unknown id visibly distinct from a lighting fault.
        let color = table.get(substance).map_or((1.0, 0.0, 1.0), |s| s.color);
        let handle = materials.add(StandardMaterial::from(to_color(color)));
        self.by_substance.push((substance, handle.clone()));
        handle
    }
}

/// Optional V3 exact-position consequences maintained after terrain edits.
#[derive(SystemParam)]
struct EditableSpatialConsequences<'w> {
    biome_regions: Option<ResMut<'w, BiomeRegions>>,
    blockers: Option<ResMut<'w, TraversalBlockers>>,
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

/// Applies direct terrain edits first, then admitted impacts, and rebuilds once.
///
/// Naive on purpose: any material change respawns the whole grid. Correct, obviously
/// so, and fast enough at this scale. Partial health never enters this consequence
/// path. Re-meshing only affected columns remains a private future optimisation.
fn apply_terrain_changes(
    mut commands: Commands,
    mutation: TerrainMutation,
    mut map: ResMut<VoxelMap>,
    grids: Query<Entity, With<HexGrid>>,
    assets: Res<GameAssets>,
    mut presentation_assets: MapPresentationAssets,
    table: Res<SubstanceTable>,
    damage_table: Option<Res<TerrainDamageTable>>,
    settings: Res<MapSettings>,
    liquid_visual_time: Res<LiquidVisualTime>,
    art_catalog: Option<Res<RuntimeArtCatalog>>,
    mut special_regions: ResMut<SpecialMovementRegions>,
    mut interiors: Option<ResMut<InteriorRegions>>,
    mut spatial: EditableSpatialConsequences,
    mut presentation: Option<ResMut<MapPresentationProjection>>,
    mut next_screen: ResMut<NextState<Screen>>,
) {
    let TerrainMutation {
        mut edits,
        mut outcomes,
        mut pending,
        mut damage_state,
        mut damaged_voxels,
        mut snapshot_dirty,
    } = mutation;
    let mut changed = false;
    let mut snapshot_changed = false;
    let mut changed_coords = BTreeSet::new();
    for edit in edits.0.drain(..) {
        let semantic_projection_protected = presentation.as_deref().is_some_and(|projection| {
            projection.protects_liquid_edit(edit.pos())
                || projection.protects_feature_edit(edit.pos())
                || projection.protects_light_edit(edit.pos())
        });
        if apply_terrain_edit(&mut map, &table, &edit, semantic_projection_protected) {
            damage_state.forget_voxel(edit.pos(), &mut damaged_voxels);
            changed = true;
            snapshot_changed = true;
            changed_coords.insert(edit.pos().coord);
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
        snapshot_changed |= matches!(
            &resolved.outcome.result,
            TerrainImpactResult::Applied(voxels)
                if voxels.iter().any(|voxel| matches!(
                    voxel.disposition,
                    TerrainImpactDisposition::Damaged | TerrainImpactDisposition::Destroyed
                ))
        );
        for position in &resolved.destroyed {
            changed = true;
            changed_coords.insert(position.coord);
            if let Some(interiors) = interiors.as_deref_mut() {
                interiors.remove_roof_voxel(*position);
            }
        }
        outcomes.write(resolved.outcome);
    }

    if snapshot_changed {
        snapshot_dirty.0 = true;
    }

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
            let Some(column) = map.column(position.coord) else {
                return false;
            };
            TraversalProfile::WALKER.admits_surface(
                table.is_solid(column.get(position.level)),
                column.headroom_above(position.level.saturating_add(1)),
            )
        });
        interiors.retain_roof_voxels(|position, _| table.is_solid(map.get(position)));
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

    let rebuilt = build_grid(
        &mut commands,
        &assets,
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
    );
    match rebuilt {
        Ok(()) => {
            for entity in &grids {
                commands.entity(entity).despawn();
            }
        }
        Err(error) => {
            fail_presentation_setup(&mut commands, &error);
            next_screen.set(Screen::Title);
        }
    }
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
        let swatch_id = SwatchId::new("test/gray").expect("the fixture swatch id should be valid");
        let swatch = PaletteSwatch::new(
            "Test Gray",
            SrgbColor::new(0.5, 0.5, 0.5).expect("the fixture color should be valid"),
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
