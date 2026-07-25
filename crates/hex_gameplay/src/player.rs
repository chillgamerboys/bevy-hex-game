use bevy::picking::events::{Click, Pointer};
use bevy::picking::Pickable;
use bevy::prelude::*;

use hex_assets::{to_color, GameAssets, PlayerSettings};
use hex_core::{GameplaySetup, HexCoord, HexSpan, HexTile, Screen};

use crate::animation::Transformation;
use crate::pathing::{HexPathingLine, PathStep};

/// Tiles as gameplay sees them: a coordinate and the column it occupies.
///
/// Gameplay reads terrain off the entities rather than from a map resource, so it
/// has no dependency on `hex_map` at all. However the map is generated or stored,
/// this query keeps working.
type TileQuery<'w, 's> = Query<'w, 's, (&'static HexCoord, &'static HexSpan), With<HexTile>>;

/// Which column a piece is standing on.
///
/// A coordinate is not enough: columns stacked at one coordinate are separate
/// places, so a piece on a bridge and a piece on the ground beneath it share an
/// address but not a location. See the rule in [`hex_core::hex`].
#[derive(Component, Debug, Clone, Copy)]
pub struct StandsOn(pub PathStep);

/// Picks the route a piece takes between two columns.
///
/// **Placeholder.** It walks the straight line of coordinates between the two ends
/// and, at each, steps onto whichever column is closest in height to the one before.
/// That is enough to stop a piece teleporting between stacked columns, but it is not
/// a movement rule: it ignores how big a step is, so it will happily walk up a cliff.
///
/// Replacing this is movement design — step limits, stairs, and the abilities that
/// bypass them. `hexx`'s `a_star` is already compiled in and is the obvious basis.
fn route(from: PathStep, to: PathStep, tiles: &TileQuery) -> Vec<PathStep> {
    let mut steps = vec![from];

    for coord in from.coord.line_between(to.coord).into_iter().skip(1) {
        let previous_top = steps.last().map_or(from.span.top, |step| step.span.top);

        // Of the columns at this coordinate, take the one nearest in height to where
        // we just were. Never collapse the stack to "the highest".
        let nearest = tiles
            .iter()
            .filter(|(tile_coord, _)| **tile_coord == coord)
            .min_by(|(_, a), (_, b)| {
                let da = (a.top - previous_top).abs();
                let db = (b.top - previous_top).abs();
                da.total_cmp(&db)
            })
            .map(|(tile_coord, span)| PathStep {
                coord: *tile_coord,
                span: *span,
            });

        if let Some(step) = nearest {
            steps.push(step);
        }
    }

    steps
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
    players: Query<(Entity, &StandsOn), With<Player>>,
    settings: Option<Res<PlayerSettings>>,
) {
    let Some(settings) = settings else {
        return;
    };

    // The click identifies a tile *entity*, which resolves to one specific column
    // even where several share a coordinate. That is why picking is the right input
    // for this: it never has to guess which column was meant.
    let clicked = event.event_target();
    let Ok((coord, span)) = tiles.get(clicked) else {
        return;
    };
    let destination = PathStep {
        coord: *coord,
        span: *span,
    };

    for (entity, standing) in players.iter() {
        let steps = route(standing.0, destination, &tiles);
        let animation: Transformation = HexPathingLine::new(&steps, settings.speed).into();
        commands
            .entity(entity)
            .insert((animation, StandsOn(destination)));
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

    // Stand on the lowest column at the origin — the ground, rather than any bridge
    // built over it. Falls back to ground level if the map put nothing there, rather
    // than refusing to spawn a player.
    let coord = HexCoord::ORIGIN;
    let span = tiles
        .iter()
        .filter(|(tile_coord, _)| **tile_coord == coord)
        .map(|(_, span)| *span)
        .min_by(|a, b| a.top.total_cmp(&b.top))
        .unwrap_or_else(|| HexSpan::from_ground(f32::EPSILON));
    let standing = PathStep { coord, span };
    let position = standing.world_position();
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
            StandsOn(standing),
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
