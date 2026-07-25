//! Spawns the hex grid: one parent entity owning a tile entity per coordinate.

use bevy::prelude::*;

use hex_assets::GameAssets;
use hex_core::config::HEX_GRID_RADIUS;
use hex_core::terrain::{HeightMap, PerlinGenerator};
use hex_core::{GameplaySetup, HexCoord, HexGrid, HexTile, Screen};

pub fn plugin(app: &mut App) {
    app.register_type::<HexCoord>()
        .register_type::<HexGrid>()
        .register_type::<HexTile>()
        // The height map has to exist before anything reads it, including
        // `hex_gameplay`'s player spawn — hence a shared set rather than a local
        // `.chain()`, which would only order these two.
        .add_systems(
            OnEnter(Screen::Gameplay),
            init_height_map.in_set(GameplaySetup::Resources),
        )
        .add_systems(
            OnEnter(Screen::Gameplay),
            spawn_grid.in_set(GameplaySetup::Entities),
        )
        .add_systems(OnExit(Screen::Gameplay), despawn_grid);
}

fn init_height_map(mut commands: Commands) {
    commands.insert_resource(HeightMap::new(PerlinGenerator::lowlands(None)));
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
) {
    let tile_material = materials.add(StandardMaterial::from(Color::srgb(1., 0.8, 0.8)));
    let hex_tile_mesh = assets.hex_tile.clone();

    let mut tiles = Vec::new();
    for hex_coord in HexCoord::ORIGIN.within_radius(HEX_GRID_RADIUS) {
        let tile = spawn_tile(
            hex_coord,
            &height_map,
            &mut commands,
            &hex_tile_mesh,
            &tile_material,
        );
        tiles.push(tile);
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

fn spawn_tile(
    hex_coord: HexCoord,
    height_map: &HeightMap,
    commands: &mut Commands,
    mesh: &Handle<Mesh>,
    material: &Handle<StandardMaterial>,
) -> Entity {
    let height = height_map.get_world_height(hex_coord);
    let mut position = hex_coord.to_world(None);
    position.y = height / 2.;
    let scale = Vec3::new(1., height, 1.);
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
        ))
        .id()
}
