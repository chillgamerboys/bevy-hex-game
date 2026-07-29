//! Builds the voxel world, and turns it into tile entities.
//!
//! Storage and generation are private to `hex_map`; rendered terrain reaches other
//! crates as entities carrying [`HexTile`](hex_core::HexTile),
//! [`HexCoord`](hex_core::HexCoord), a surface [`TilePos`](hex_core::TilePos),
//! [`HexSpan`](hex_core::HexSpan), [`SubstanceId`](hex_core::SubstanceId), and
//! [`Headroom`](hex_core::Headroom). The substance table itself is shared through
//! `hex_assets` because gameplay also reads its behavior flags.
//!
//! Keeping that boundary narrow is what lets the map be rebuilt without touching
//! gameplay. A richer map means producing different voxels in the terrain builder;
//! it does not change what a tile *is* to anyone else.

use std::collections::BTreeSet;
use std::fmt;

use bevy::{ecs::system::SystemParam, prelude::*};

use hex_assets::{to_color, GameAssets, RuntimeArtCatalog, SubstanceTable};
use hex_core::{
    BiomeRegions, CanopyOccluder, CutawayOccluder, GameplayLight, GameplaySetup,
    GameplaySetupFailure, Headroom, HexCoord, HexGrid, HexSpan, HexTile, InteriorRegionId,
    InteriorRegions, MapAnchorId, MapAnchors, MapViewHint, PerceptionSystems,
    PresentationOcclusion, ResolvedMapSeed, Screen, SpecialMovementRegions, SubstanceId,
    TerrainEdit, TerrainReady, TilePos, TraversalBlockers, TraversalProfile,
};

use crate::feature_render::{self, FeaturePresentationError};
use crate::liquid_render::{self, LiquidMaterial, LiquidPresentationError, LiquidVisualTime};
use crate::procedural;
use crate::procedural_v2;
use crate::procedural_v3;
use crate::procedural_v3::MapPresentationProjection;
use crate::settings::{MapSettings, TerrainSettings};
use crate::terrain::{build_non_procedural_map, TerrainPalette};
use crate::voxel::{runs, Column, SubstanceRun, VoxelMap};
use crate::{
    CavesReportMetrics, ForestReportMetrics, FortReportMetrics, GenerationReport,
    ProceduralRecipeMetrics, Ring7Metrics, WaterfallReportMetrics,
};

/// Registers world construction and tile spawning.
pub fn plugin(app: &mut App) {
    liquid_render::plugin(app);
    app.register_type::<HexCoord>()
        .register_type::<HexGrid>()
        .register_type::<HexSpan>()
        .register_type::<HexTile>()
        .register_type::<SubstanceId>()
        .register_type::<TilePos>()
        .register_type::<Headroom>()
        .register_type::<InteriorRegionId>()
        .register_type::<CutawayOccluder>()
        .register_type::<CanopyOccluder>()
        .register_type::<PresentationOcclusion>()
        .register_type::<GameplayLight>()
        .register_type::<TerrainReady>()
        .register_type::<GenerationReport>()
        .register_type::<ProceduralRecipeMetrics>()
        .register_type::<WaterfallReportMetrics>()
        .register_type::<ForestReportMetrics>()
        .register_type::<FortReportMetrics>()
        .register_type::<CavesReportMetrics>()
        .register_type::<Ring7Metrics>()
        .add_message::<TerrainEdit>()
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
            apply_terrain_edits
                .run_if(in_state(Screen::Gameplay))
                .run_if(resource_exists::<TerrainReady>)
                .before(PerceptionSystems::ResolveIllumination),
        )
        .add_systems(OnExit(Screen::Gameplay), teardown_map);
}

fn generate_world(
    mut commands: Commands,
    settings: Res<MapSettings>,
    table: Res<SubstanceTable>,
    art_catalog: Option<Res<RuntimeArtCatalog>>,
    resolved_seed: Option<Res<ResolvedMapSeed>>,
) {
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

fn teardown_map(mut commands: Commands, grids: Query<Entity, With<HexGrid>>) {
    for entity in &grids {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<VoxelMap>();
    commands.remove_resource::<MapAnchors>();
    commands.remove_resource::<SpecialMovementRegions>();
    commands.remove_resource::<InteriorRegions>();
    commands.remove_resource::<TraversalBlockers>();
    commands.remove_resource::<BiomeRegions>();
    commands.remove_resource::<MapViewHint>();
    commands.remove_resource::<GenerationReport>();
    commands.remove_resource::<MapPresentationProjection>();
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
) {
    if let Err(error) = build_grid(
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
    ) {
        fail_presentation_setup(&mut commands, &error);
    }
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
) -> Result<(), MapPresentationError> {
    let mesh = assets.hex_tile.clone();
    let mut palette_materials = MaterialCache::default();
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
                Name::new("HexTile"),
                HexTile,
                coord,
                span,
                run.substance,
                position,
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
}

#[derive(SystemParam)]
struct MapPresentationAssets<'w> {
    materials: ResMut<'w, Assets<StandardMaterial>>,
    meshes: ResMut<'w, Assets<Mesh>>,
    liquid_materials: ResMut<'w, Assets<LiquidMaterial>>,
}

impl fmt::Display for MapPresentationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Liquid(error) => write!(formatter, "liquid presentation failed: {error}"),
            Self::Feature(error) => write!(formatter, "feature presentation failed: {error}"),
        }
    }
}

impl std::error::Error for MapPresentationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Liquid(error) => Some(error),
            Self::Feature(error) => Some(error),
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

/// Applies terrain edits requested by gameplay, then rebuilds what changed.
///
/// Naive on purpose: any edit respawns the whole grid. Correct, obviously so, and
/// fast enough at this scale. Re-meshing only the affected columns is the first
/// optimisation worth making, and it is a change entirely inside this crate.
fn apply_terrain_edits(
    mut commands: Commands,
    mut edits: MessageReader<TerrainEdit>,
    mut map: ResMut<VoxelMap>,
    grids: Query<Entity, With<HexGrid>>,
    assets: Res<GameAssets>,
    mut presentation_assets: MapPresentationAssets,
    table: Res<SubstanceTable>,
    settings: Res<MapSettings>,
    liquid_visual_time: Res<LiquidVisualTime>,
    mut special_regions: ResMut<SpecialMovementRegions>,
    mut interiors: Option<ResMut<InteriorRegions>>,
    mut spatial: EditableSpatialConsequences,
    mut presentation: Option<ResMut<MapPresentationProjection>>,
    mut next_screen: ResMut<NextState<Screen>>,
) {
    let mut changed = false;
    let mut changed_coords = BTreeSet::new();
    for edit in edits.read() {
        let semantic_projection_protected = presentation.as_deref().is_some_and(|projection| {
            projection.protects_liquid_edit(edit.pos())
                || projection.protects_feature_edit(edit.pos())
                || projection.protects_light_edit(edit.pos())
        });
        if apply_terrain_edit(&mut map, &table, edit, semantic_projection_protected) {
            changed = true;
            changed_coords.insert(edit.pos().coord);
            if let Some(interiors) = interiors.as_deref_mut() {
                // A replacement is new material, not part of the authored roof even
                // when it remains solid. Removing only this voxel keeps both original
                // fragments available for exact re-projection.
                interiors.remove_roof_voxel(edit.pos());
            }
        }
    }
    if !changed {
        return;
    }

    if let Some(presentation) = presentation.as_deref_mut() {
        presentation.retain_features(|feature| {
            if feature.kind != procedural_v3::FeatureKind::TallGrass
                || !changed_coords.contains(&feature.root.coord)
            {
                return true;
            }
            let Some(column) = map.column(feature.root.coord) else {
                return false;
            };
            TraversalProfile::WALKER.admits_surface(
                table.is_solid(column.get(feature.root.level)),
                column.headroom_above(feature.root.level.saturating_add(1)),
            )
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
