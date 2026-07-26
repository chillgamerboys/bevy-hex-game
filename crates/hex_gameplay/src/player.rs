use bevy::picking::events::{Click, Pointer};
use bevy::picking::Pickable;
use bevy::prelude::*;

use hex_assets::{to_color, GameAssets, PlayerSettings, SubstanceTable};
use hex_core::{GameplaySetup, Headroom, HexCoord, HexSpan, HexTile, Screen, SubstanceId, TilePos};

use hex_anim::Transformation;

use crate::movement::{route, Body, Footing, Standing};
use crate::pathing::HexPathingLine;

/// Tiles as gameplay sees them.
///
/// Terrain is read off the entities rather than from a map resource, so gameplay has
/// no dependency on `hex_map` at all. However the map is generated or stored, this
/// query keeps working.
///
/// [`Headroom`] comes along because standability depends on it, but the query does not
/// filter on it: what counts as enough room depends on the body asking, so the filter
/// belongs in [`Footing::from_tiles`] where the body is known.
type TileQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static TilePos,
        &'static HexSpan,
        &'static SubstanceId,
        &'static Headroom,
    ),
    With<HexTile>,
>;

/// Which column a piece is standing on.
///
/// A coordinate is not enough: columns stacked at one coordinate are separate places,
/// so a piece on a bridge and a piece on the ground beneath it share an address but
/// not a location.
#[derive(Component, Debug, Clone, Copy)]
pub struct StandsOn(pub Standing);

/// Registers the player, its spawning, and click-to-move.
pub fn plugin(app: &mut App) {
    app.register_type::<Player>()
        .register_type::<Body>()
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
    players: Query<(Entity, &StandsOn, &Body), With<Player>>,
    settings: Option<Res<PlayerSettings>>,
    table: Option<Res<SubstanceTable>>,
) {
    let (Some(settings), Some(table)) = (settings, table) else {
        return;
    };

    // The click identifies a tile *entity*, which resolves to one specific column
    // even where several share a coordinate. Picking is the right input for exactly
    // that reason: it never has to guess which column was meant.
    let clicked = event.event_target();
    let Ok((pos, _, _, _)) = tiles.get(clicked) else {
        return;
    };

    for (entity, standing, body) in players.iter() {
        // Footing and the destination are resolved per body, because whether a column
        // can be stood on depends on who is asking — a crawlspace is footing for a
        // small creature and a wall for a large one. With one player this is the same
        // work as hoisting it out of the loop; with a mixed party it is the difference
        // between right and wrong.
        let footing = Footing::from_tiles(tiles.iter(), &table, *body);
        let Some(destination) = footing.at(*pos) else {
            continue;
        };

        // No route is a legitimate answer: terrain is not guaranteed connected, and
        // a cliff, a gap, or a ceiling too low to fit under means the piece simply
        // does not move.
        let Some(steps) = route(standing.0, destination, &footing) else {
            continue;
        };
        let animation: Transformation = HexPathingLine::new(&steps, settings.speed).into();
        commands
            .entity(entity)
            .insert((animation, StandsOn(destination)));
    }
}

/// Marks the piece the player controls.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct Player;

fn spawn_player(
    mut commands: Commands,
    assets: Res<GameAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    tiles: TileQuery,
    table: Res<SubstanceTable>,
    settings: Res<PlayerSettings>,
) {
    let material = materials.add(StandardMaterial::from(to_color(settings.color)));

    let body = Body {
        levels_tall: settings.levels_tall,
    };

    // Stand on the lowest column at the origin that this body fits on — the ground,
    // rather than any bridge built over it.
    let coord = HexCoord::ORIGIN;
    let footing = Footing::from_tiles(tiles.iter(), &table, body);
    let standing = footing.ground(coord).unwrap_or(Standing {
        pos: TilePos::new(coord, 0),
        span: HexSpan::new(0.0, f32::EPSILON),
    });
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
            body,
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
