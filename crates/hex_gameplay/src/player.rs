use bevy::picking::events::{Click, Pointer};
use bevy::picking::Pickable;
use bevy::prelude::*;

use hex_assets::{to_color, GameAssets, PlayerSettings};
use hex_core::{GameplaySetup, HeightMap, HexCoord, HexTile, Screen};

use crate::animation::Transformation;
use crate::pathing::HexPathingLine;

pub fn plugin(app: &mut App) {
    app.register_type::<Player>()
        // `GameplaySetup::Entities` runs after `GameplaySetup::Resources`, which is
        // where `hex_world` inserts the height map this system reads. Ordering has
        // to be expressed through a shared set because the two systems live in
        // different crates and would otherwise race.
        .add_systems(
            OnEnter(Screen::Gameplay),
            spawn_player.in_set(GameplaySetup::Entities),
        )
        .add_systems(OnExit(Screen::Gameplay), despawn_player)
        .add_observer(on_tile_clicked);
}

fn despawn_player(mut commands: Commands, players: Query<Entity, With<Player>>) {
    for entity in &players {
        commands.entity(entity).despawn();
    }
}

/// Global picking observer: when any `HexTile` is clicked, animate the player
/// over to that tile along a hex-by-hex straight line.
///
/// `HeightMap` is taken as an `Option` because observers are global and fire on
/// every click, including clicks on menus, where the map does not exist. A plain
/// `Res<HeightMap>` panics there — parameter validation runs *before* the body, so
/// the "is this a tile?" check below never gets the chance to reject it.
fn on_tile_clicked(
    event: On<Pointer<Click>>,
    mut commands: Commands,
    tile_query: Query<&HexCoord, With<HexTile>>,
    player_query: Query<(Entity, &Transform), With<Player>>,
    height_map: Option<Res<HeightMap>>,
    settings: Option<Res<PlayerSettings>>,
) {
    let (Some(height_map), Some(settings)) = (height_map, settings) else {
        return;
    };
    let clicked = event.event_target();
    let Ok(tile_coord) = tile_query.get(clicked) else {
        return;
    };

    for (entity, transform) in player_query.iter() {
        let animation: Transformation = HexPathingLine::new(
            HexCoord::from_world(transform.translation),
            *tile_coord,
            settings.speed,
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
    assets: Res<GameAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    height_map: Res<HeightMap>,
    settings: Res<PlayerSettings>,
) {
    let material = materials.add(StandardMaterial::from(to_color(settings.color)));

    let coord = HexCoord::ORIGIN;
    let position = coord.to_world(Some(&height_map));
    let scale = Vec3::splat(settings.scale);

    let [mesh_a, mesh_b] = assets.player_pieces.clone();

    let child_transform = Transform {
        // Offsets the mesh so its origin sits on the tile centre.
        translation: Vec3::new(-settings.scale, -settings.scale, -10. * settings.scale),
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
