// Bevy Imports
use bevy::gltf::GltfAssetLabel;
use bevy::picking::Pickable;
use bevy::picking::events::{Click, Pointer};
use bevy::prelude::*;

use crate::plugins::world_3d::{
    transformation::{HexPathingLine, Transformation},
    config::{PLAYER_SCALE, PLAYER_SPEED},
    hex::{HexCoord, HexTile, height_map::HeightMap},
};

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Player>()
            .add_systems(Startup, spawn_player)
            .add_observer(on_tile_clicked);
    }
}

/// Global picking observer: when any `HexTile` is clicked, animate the player
/// over to that tile along a hex-by-hex straight line.
fn on_tile_clicked(
    event: On<Pointer<Click>>,
    mut commands: Commands,
    tile_query: Query<&HexCoord, With<HexTile>>,
    player_query: Query<(Entity, &Transform), With<Player>>,
    height_map: Res<HeightMap>,
) {
    let clicked = event.event_target();
    let Ok(tile_coord) = tile_query.get(clicked) else { return };

    for (entity, transform) in player_query.iter() {
        let animation: Transformation = HexPathingLine::new(
            HexCoord::from_world(transform.translation),
            *tile_coord,
            PLAYER_SPEED,
            &height_map,
        )
        .into();
        commands.entity(entity).insert(animation);
    }
}

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct Player;

fn spawn_player(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    height_map: Res<HeightMap>,
) {
    let material = materials.add(StandardMaterial::from(Color::srgb(1., 0.2, 0.2)));

    let coord = HexCoord(0, 0);
    let position = coord.to_world(Some(&height_map));
    let scale = Vec3::splat(PLAYER_SCALE);

    let mesh_a: Handle<Mesh> =
        asset_server.load(GltfAssetLabel::Primitive { mesh: 0, primitive: 0 }.from_asset("meshes/pieces.glb"));
    let mesh_b: Handle<Mesh> =
        asset_server.load(GltfAssetLabel::Primitive { mesh: 1, primitive: 0 }.from_asset("meshes/pieces.glb"));

    let child_transform = Transform {
        translation: Vec3::new(-PLAYER_SCALE, -PLAYER_SCALE, -10. * PLAYER_SCALE),
        scale,
        ..default()
    };

    commands
        .spawn((
            Transform::from_translation(position),
            Visibility::default(),
            Player,
            Name::new("Player"),
        ))
        .with_children(|parent| {
            // Mark player meshes as Pickable::IGNORE so clicks pass through to tiles below.
            parent.spawn((
                Mesh3d(mesh_a),
                MeshMaterial3d(material.clone()),
                child_transform,
                Pickable::IGNORE,
            ));
            parent.spawn((
                Mesh3d(mesh_b),
                MeshMaterial3d(material),
                child_transform,
                Pickable::IGNORE,
            ));
        });
}
