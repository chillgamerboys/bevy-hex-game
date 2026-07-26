//! Builds the voxel world, and turns it into tile entities.
//!
//! Storage and generation are private to `hex_map`; rendered terrain reaches other
//! crates as entities carrying [`HexTile`](hex_core::HexTile),
//! [`HexCoord`](hex_core::HexCoord), a surface [`TilePos`](hex_core::TilePos),
//! [`HexSpan`](hex_core::HexSpan), [`SubstanceId`](hex_core::SubstanceId), and
//! [`Headroom`](hex_core::Headroom). The substance table itself is shared through
//! `hex_assets` because gameplay also reads its behavior flags.
//!
//! Keeping that seam narrow is what lets the map be rebuilt without touching
//! gameplay. A richer map means producing different voxels here; it does not change
//! what a tile *is* to anyone else.

use bevy::prelude::*;

use hex_assets::{to_color, GameAssets, SubstanceTable};
use hex_core::{
    GameplaySetup, Headroom, HexCoord, HexGrid, HexSpan, HexTile, Level, Screen, SubstanceId,
    TerrainEdit, TilePos, MAX_HEADROOM,
};

use crate::generator::{HeightMap, PerlinGenerator, PerlinStep};
use crate::settings::MapSettings;
use crate::voxel::{runs, Column, VoxelMap};

/// Registers world construction and tile spawning.
pub fn plugin(app: &mut App) {
    app.register_type::<HexCoord>()
        .register_type::<HexGrid>()
        .register_type::<HexSpan>()
        .register_type::<HexTile>()
        .register_type::<SubstanceId>()
        .register_type::<TilePos>()
        .register_type::<Headroom>()
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
            spawn_grid.in_set(GameplaySetup::Terrain),
        )
        .add_systems(
            Update,
            apply_terrain_edits.run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(OnExit(Screen::Gameplay), teardown_map);
}

/// Which substances the generator lays down, resolved once from the table.
///
/// Names are looked up rather than hardcoded as ids, because ids come from the
/// substance file and would change if it did. Missing names fall back to air, which
/// renders as nothing — a visible, harmless failure rather than a wrong-looking world.
struct Palette {
    bedrock: SubstanceId,
    stone: SubstanceId,
    dirt: SubstanceId,
    grass: SubstanceId,
}

impl Palette {
    fn from_table(table: &SubstanceTable) -> Self {
        let id = |name: &str| table.id(name).unwrap_or(SubstanceId::AIR);
        Self {
            bedrock: id("bedrock"),
            stone: id("stone"),
            dirt: id("dirt"),
            grass: id("grass"),
        }
    }
}

/// Fills every column from the bedrock floor up to its generated surface height.
///
/// Solid all the way down is what gives digging something to work through. The
/// banding — bedrock, stone, dirt, a single layer of grass on top — is deliberately
/// simple: it exists to prove the model renders and to be replaced.
fn generate_world(mut commands: Commands, settings: Res<MapSettings>, table: Res<SubstanceTable>) {
    let palette = Palette::from_table(&table);

    let steps = settings
        .terrain
        .steps
        .iter()
        .map(|step| PerlinStep::new(step.x_freq, step.y_freq, step.magnitude))
        .collect();
    let generator = PerlinGenerator::new(steps, settings.terrain.seed);
    let heights = HeightMap::new(generator, settings.grid_radius);

    let mut map = VoxelMap::new();
    for coord in HexCoord::ORIGIN.within_radius(settings.grid_radius) {
        map.insert_column(coord, column_for(heights.surface_level(coord), &palette));
    }

    commands.insert_resource(map);
}

/// The strata of one column, given the level of its topmost solid voxel.
fn column_for(surface: hex_core::Level, palette: &Palette) -> Column {
    let mut column = Column::new();
    for level in 0..=surface {
        let substance = if level == 0 {
            palette.bedrock
        } else if level == surface {
            palette.grass
        } else if level + 2 >= surface {
            palette.dirt
        } else {
            palette.stone
        };
        column.set(level, substance);
    }
    column
}

fn teardown_map(mut commands: Commands, grids: Query<Entity, With<HexGrid>>) {
    for entity in &grids {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<VoxelMap>();
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
) {
    build_grid(
        &mut commands,
        &assets,
        &mut materials,
        &map,
        &table,
        &settings,
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
) {
    let mesh = assets.hex_tile.clone();
    let mut palette_materials = MaterialCache::default();

    let mut tiles = Vec::new();
    for (coord, column) in map.columns() {
        for run in runs(column) {
            let material = palette_materials.get_or_create(run.substance, table, materials);
            let span = span_for(run.bottom, run.top, settings.level_height);

            // Only the map can measure this: a run knows its own extent but nothing
            // about what is stacked on it. Zero means buried, and nothing can stand
            // on a buried run however solid it is.
            let headroom = headroom_above(column, run.top);

            tiles.push(
                commands
                    .spawn((
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
                        // The run's **topmost solid voxel** — the thing something
                        // standing here is standing on. Tagging the base instead
                        // would force gameplay to know the level height to work the
                        // surface out, putting a dependency on the map straight back
                        // into movement. Voxels inside the run are addressed by
                        // `TilePos`, not by this entity.
                        TilePos::new(coord, run.top - 1),
                        Headroom(headroom),
                    ))
                    .id(),
            );
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

/// Clear voxels starting at `from`, saturating at [`MAX_HEADROOM`].
///
/// `from` is a run's exclusive `top`, so it names the voxel directly above the run's
/// topmost solid one — the first place a body standing here would put its feet' worth
/// of air. A solid voxel there means the run is buried and the answer is zero.
///
/// Saturating matters: above a column's top the air is unbounded, so counting to the
/// first solid voxel would never terminate. [`Column::get`] returns air for anything
/// out of range, which is what makes this loop safe without bounds checks.
fn headroom_above(column: &Column, from: Level) -> Level {
    (0..MAX_HEADROOM)
        .take_while(|offset| column.get(from + offset).is_air())
        .count()
        .try_into()
        .unwrap_or(MAX_HEADROOM)
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
) {
    let mut changed = false;
    for edit in edits.read() {
        changed |= apply_terrain_edit(&mut map, &table, edit);
    }
    if !changed {
        return;
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
