//! Turns generated terrain into tile entities.
//!
//! This is the seam between the map and the rest of the game. Everything upstream
//! of tile spawning — the height map, the generators, the settings — is private to
//! this crate. Everything downstream sees only entities carrying a
//! [`HexCoord`](hex_core::HexCoord) and a [`HexSpan`](hex_core::HexSpan).
//!
//! Keeping that seam narrow is what lets the map be rebuilt without touching
//! gameplay. A richer map means producing different spans here; it does not mean
//! changing what a tile *is* to anyone else.

use bevy::prelude::*;

use hex_assets::{to_color, GameAssets};
use hex_core::{GameplaySetup, HexCoord, HexGrid, HexSpan, HexTile, Screen};

use crate::generator::{HeightMap, PerlinGenerator, PerlinStep};
use crate::settings::MapSettings;

/// Registers terrain construction and tile spawning.
pub fn plugin(app: &mut App) {
    app.register_type::<HexCoord>()
        .register_type::<HexGrid>()
        .register_type::<HexSpan>()
        .register_type::<HexTile>()
        // Split across two sets rather than chained locally: `hex_gameplay` spawns
        // the player into `Actors`, which must come after the tiles here, and a
        // local `.chain()` cannot order systems in another crate.
        .add_systems(
            OnEnter(Screen::Gameplay),
            init_height_map.in_set(GameplaySetup::Resources),
        )
        .add_systems(
            OnEnter(Screen::Gameplay),
            spawn_grid.in_set(GameplaySetup::Terrain),
        )
        .add_systems(OnExit(Screen::Gameplay), despawn_grid);
}

fn init_height_map(mut commands: Commands, settings: Res<MapSettings>) {
    let steps = settings
        .terrain
        .steps
        .iter()
        .map(|step| PerlinStep::new(step.x_freq, step.y_freq, step.magnitude))
        .collect();
    let generator = PerlinGenerator::new(steps, settings.terrain.seed);
    commands.insert_resource(HeightMap::new(
        generator,
        settings.grid_radius,
        settings.height_scale,
    ));
}

fn despawn_grid(mut commands: Commands, grids: Query<Entity, With<HexGrid>>) {
    for entity in &grids {
        commands.entity(entity).despawn();
    }
}

fn spawn_grid(
    mut commands: Commands,
    assets: Res<GameAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    height_map: Res<HeightMap>,
    settings: Res<MapSettings>,
) {
    let tile_material = materials.add(StandardMaterial::from(to_color(settings.tile_color)));
    let hex_tile_mesh = assets.hex_tile.clone();

    let mut tiles = Vec::new();
    for hex_coord in HexCoord::ORIGIN.within_radius(settings.grid_radius) {
        // Today's terrain is a height field, so every column rests on the ground.
        // A generator that returns several spans per coordinate — floating
        // platforms, overhangs — would push more entities here and nothing else in
        // the game would need to change.
        let span = HexSpan::from_ground(height_map.get_world_height(hex_coord));

        tiles.push(spawn_tile(
            hex_coord,
            span,
            &mut commands,
            &hex_tile_mesh,
            &tile_material,
        ));
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

/// Spawns one tile occupying `span`.
///
/// The mesh has its origin at its centre, so it is placed at the span's midpoint and
/// scaled to the span's height. That is what makes the entity's transform agree with
/// its `HexSpan` — an invariant gameplay relies on when it reads a tile's surface,
/// and one covered by a test.
fn spawn_tile(
    hex_coord: HexCoord,
    span: HexSpan,
    commands: &mut Commands,
    mesh: &Handle<Mesh>,
    material: &Handle<StandardMaterial>,
) -> Entity {
    let position = hex_coord.to_world(span.centre());
    let scale = Vec3::new(1., span.height(), 1.);

    commands
        .spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            Transform {
                translation: position,
                scale,
                ..default()
            },
            Name::new("HexTile"),
            HexTile,
            hex_coord,
            span,
        ))
        .id()
}
