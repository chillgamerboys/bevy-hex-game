//! Spawns the hex grid: one parent entity owning a tile entity per coordinate.

use bevy::prelude::*;

use hex_assets::{to_color, GameAssets, WorldSettings};
use hex_core::terrain::{HeightMap, PerlinGenerator, PerlinStep};
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

fn init_height_map(mut commands: Commands, settings: Res<WorldSettings>) {
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
    settings: Res<WorldSettings>,
) {
    let tile_material = materials.add(StandardMaterial::from(to_color(settings.tile_color)));
    let hex_tile_mesh = assets.hex_tile.clone();

    let mut tiles = Vec::new();
    for hex_coord in HexCoord::ORIGIN.within_radius(settings.grid_radius) {
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
