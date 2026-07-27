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

use bevy::prelude::*;

use hex_assets::{to_color, GameAssets, SubstanceTable};
use hex_core::{
    CutawayOccluder, GameplaySetup, GameplaySetupFailure, Headroom, HexCoord, HexGrid, HexSpan,
    HexTile, InteriorRegionId, InteriorRegions, MapAnchorId, MapAnchors, MapViewHint,
    ResolvedMapSeed, Screen, SpecialMovementRegions, SubstanceId, TerrainEdit, TerrainReady,
    TilePos, TraversalProfile,
};

use crate::procedural;
use crate::procedural_v2;
use crate::settings::{MapSettings, TerrainSettings};
use crate::terrain::{build_non_procedural_map, TerrainPalette};
use crate::voxel::{runs, Column, SubstanceRun, VoxelMap};
use crate::GenerationReport;

/// Registers world construction and tile spawning.
pub fn plugin(app: &mut App) {
    app.register_type::<HexCoord>()
        .register_type::<HexGrid>()
        .register_type::<HexSpan>()
        .register_type::<HexTile>()
        .register_type::<SubstanceId>()
        .register_type::<TilePos>()
        .register_type::<Headroom>()
        .register_type::<InteriorRegionId>()
        .register_type::<CutawayOccluder>()
        .register_type::<TerrainReady>()
        .register_type::<GenerationReport>()
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
                .run_if(resource_exists::<TerrainReady>),
        )
        .add_systems(OnExit(Screen::Gameplay), teardown_map);
}

fn generate_world(
    mut commands: Commands,
    settings: Res<MapSettings>,
    table: Res<SubstanceTable>,
    resolved_seed: Option<Res<ResolvedMapSeed>>,
) {
    commands.remove_resource::<GameplaySetupFailure>();
    commands.remove_resource::<TerrainReady>();
    commands.remove_resource::<GenerationReport>();
    commands.remove_resource::<MapAnchors>();
    commands.remove_resource::<SpecialMovementRegions>();
    commands.remove_resource::<InteriorRegions>();
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
    let generated = match procedural_settings {
        crate::settings::ProceduralSettings::V1(v1) => procedural::build(
            settings.grid_radius,
            v1,
            seed.0,
            &palette,
            TraversalProfile::WALKER,
            &|substance| table.is_solid(substance),
        ),
        crate::settings::ProceduralSettings::V2(v2) => {
            let reason = match procedural_v2::ensure_recipe_available(v2) {
                Ok(()) => "procedural V2 recipe has no generation runner".to_owned(),
                Err(error) => error.to_string(),
            };
            error!("cannot build procedural V2 terrain: {reason}");
            commands.insert_resource(GameplaySetupFailure::new(format!(
                "The selected procedural terrain cannot be built: {reason}."
            )));
            return;
        }
    };
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

fn teardown_map(mut commands: Commands, grids: Query<Entity, With<HexGrid>>) {
    for entity in &grids {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<VoxelMap>();
    commands.remove_resource::<MapAnchors>();
    commands.remove_resource::<SpecialMovementRegions>();
    commands.remove_resource::<InteriorRegions>();
    commands.remove_resource::<MapViewHint>();
    commands.remove_resource::<GenerationReport>();
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
    mut materials: ResMut<Assets<StandardMaterial>>,
    map: Res<VoxelMap>,
    table: Res<SubstanceTable>,
    settings: Res<MapSettings>,
    interiors: Option<Res<InteriorRegions>>,
) {
    build_grid(
        &mut commands,
        &assets,
        &mut materials,
        &map,
        &table,
        &settings,
        interiors.as_deref(),
    );
}

/// Spawns the grid entities. Shared by first construction and by rebuilds after an
/// edit, so the two cannot drift apart.
fn build_grid(
    commands: &mut Commands,
    assets: &GameAssets,
    materials: &mut Assets<StandardMaterial>,
    map: &VoxelMap,
    table: &SubstanceTable,
    settings: &MapSettings,
    interiors: Option<&InteriorRegions>,
) {
    let mesh = assets.hex_tile.clone();
    let mut palette_materials = MaterialCache::default();

    let mut tiles = Vec::new();
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
                tile.insert(CutawayOccluder(region));
            }
            tiles.push(tile.id());
        }
    }

    commands
        .spawn((
            Transform::default(),
            Visibility::default(),
            Name::new("HexGrid"),
            HexGrid,
        ))
        .add_children(&tiles);
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
    mut materials: ResMut<Assets<StandardMaterial>>,
    table: Res<SubstanceTable>,
    settings: Res<MapSettings>,
    mut special_regions: ResMut<SpecialMovementRegions>,
    mut interiors: Option<ResMut<InteriorRegions>>,
) {
    let mut changed = false;
    for edit in edits.read() {
        if apply_terrain_edit(&mut map, &table, edit) {
            changed = true;
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

    for entity in &grids {
        commands.entity(entity).despawn();
    }
    build_grid(
        &mut commands,
        &assets,
        &mut materials,
        &map,
        &table,
        &settings,
        interiors.as_deref(),
    );
}

/// Applies a changed edit at a nonnegative level unless it replaces a non-diggable voxel.
fn apply_terrain_edit(map: &mut VoxelMap, table: &SubstanceTable, edit: &TerrainEdit) -> bool {
    let pos = edit.pos();
    if pos.level < 0 {
        return false;
    }

    let current = map.get(pos);
    let replacement = match *edit {
        TerrainEdit::Set { substance, .. } => substance,
        TerrainEdit::Clear { .. } => SubstanceId::AIR,
    };

    if current == replacement || (!current.is_air() && !table.is_diggable(current)) {
        return false;
    }

    map.set(pos, replacement);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
