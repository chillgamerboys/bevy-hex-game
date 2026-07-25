use bevy::picking::events::{Click, Pointer};
use bevy::picking::Pickable;
use bevy::prelude::*;

use hex_assets::{to_color, GameAssets, PlayerSettings};
use hex_core::{GameplaySetup, HexCoord, HexSpan, HexTile, Screen};

use crate::animation::Transformation;
use crate::pathing::{HexPathingLine, SurfaceHeights};

/// Tiles as gameplay sees them: a coordinate and the column it occupies.
///
/// Gameplay reads terrain off the entities rather than from a map resource, so it
/// has no dependency on `hex_map` at all. However the map is generated or stored,
/// this query keeps working.
type TileQuery<'w, 's> = Query<'w, 's, (&'static HexCoord, &'static HexSpan), With<HexTile>>;

/// Surface height of every tile, keyed by coordinate.
///
/// Where several columns share a coordinate — a bridge over ground — this keeps the
/// highest. That is a placeholder, not a design: choosing which surface a piece
/// belongs on is a movement question, and belongs with whoever defines movement.
fn surface_heights(tiles: &TileQuery) -> SurfaceHeights {
    let mut surfaces = SurfaceHeights::default();
    for (coord, span) in tiles.iter() {
        surfaces
            .entry(*coord)
            .and_modify(|top| *top = top.max(span.top))
            .or_insert(span.top);
    }
    surfaces
}

pub fn plugin(app: &mut App) {
    app.register_type::<Player>()
        // `Actors` runs after `Terrain`, where `hex_map` spawns the tiles this
        // system queries to find the surface to stand on. The set boundary also
        // provides the sync point that makes those tiles queryable at all —
        // `Commands`-spawned entities are invisible until the queue is applied.
        .add_systems(
            OnEnter(Screen::Gameplay),
            spawn_player.in_set(GameplaySetup::Actors),
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
/// `PlayerSettings` is taken as an `Option` because observers are global and fire on
/// every click, including clicks on menus, where settings-derived resources may be
/// absent. A plain `Res<T>` panics there — Bevy validates system parameters *before*
/// the body runs, so the "is this a tile?" check below never gets the chance to
/// reject it.
fn on_tile_clicked(
    event: On<Pointer<Click>>,
    mut commands: Commands,
    tiles: TileQuery,
    player_query: Query<(Entity, &Transform), With<Player>>,
    settings: Option<Res<PlayerSettings>>,
) {
    let Some(settings) = settings else {
        return;
    };
    let clicked = event.event_target();
    let Ok((tile_coord, _)) = tiles.get(clicked) else {
        return;
    };

    let surfaces = surface_heights(&tiles);

    for (entity, transform) in player_query.iter() {
        let animation: Transformation = HexPathingLine::new(
            HexCoord::from_world(transform.translation),
            *tile_coord,
            settings.speed,
            &surfaces,
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
    tiles: TileQuery,
    settings: Res<PlayerSettings>,
) {
    let material = materials.add(StandardMaterial::from(to_color(settings.color)));

    // Stand on whatever the map put at the origin. Falls back to ground level if
    // nothing is there, rather than refusing to spawn a player.
    let coord = HexCoord::ORIGIN;
    let surface = surface_heights(&tiles).get(&coord).copied().unwrap_or(0.0);
    let position = coord.to_world(surface);
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
