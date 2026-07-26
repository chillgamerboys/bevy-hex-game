//! The units on the map: what they are, and how they get placed.
//!
//! A unit is a [`Body`] standing on a surface, wearing a [`Faction`] so anything else
//! can tell friend from foe without naming concrete types. `Player` and `Enemy` are
//! markers on top of that, for the two things that currently exist.
//!
//! Spawning reads `assets/config/scenario.ron`, which is deliberately the crudest
//! thing that works: two coordinates. It exists so terrain can be tried out without
//! writing Rust, not because it is the encounter format the game will ship.

use bevy::picking::events::{Click, Pointer};
use bevy::picking::Pickable;
use bevy::prelude::*;

use hex_anim::Transformation;
use hex_assets::{
    to_color, CubeCoord, GameAssets, PlayerSettings, ScenarioSettings, SubstanceTable,
};
use hex_core::{
    GameplaySetup, Headroom, HexCoord, HexSpan, HexTile, Mode, Screen, SubstanceId, TilePos, Turn,
};

use crate::movement::{route, Body, Footing, Standing};
use crate::pathing::HexPathingLine;

/// Tiles as units see them.
///
/// Terrain is read off the entities rather than from a map resource, so this crate has
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

/// Which surface a piece is standing on.
///
/// A coordinate is not enough: surfaces stacked in one column are separate places,
/// so a piece on a bridge and a piece on the ground beneath it share a horizontal
/// address but not a location.
#[derive(Component, Debug, Clone, Copy)]
pub struct StandsOn(pub Standing);

/// Which side a unit is on.
///
/// A component rather than a `Player`-or-not check, so "is this hostile to me" is one
/// comparison and does not have to enumerate every unit type that exists. Neutral
/// parties and enemies that turn on each other both fit without a new mechanism.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[reflect(Component)]
pub enum Faction {
    /// The party the player controls.
    Player,
    /// Everything that wants the party dead.
    Hostile,
}

impl Faction {
    /// Whether these two sides fight each other.
    ///
    /// Deliberately not `self != other`: a third neutral faction should be hostile to
    /// nobody, and writing the rule as inequality would make it hostile to everybody.
    #[must_use]
    pub fn is_hostile_to(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::Player, Self::Hostile) | (Self::Hostile, Self::Player)
        )
    }
}

/// Marks the piece the player controls.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct Player;

/// Marks a unit that fights the player.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct Enemy;

/// Registers the units, their spawning, and click-to-move.
pub fn plugin(app: &mut App) {
    app.register_type::<Player>()
        .register_type::<Enemy>()
        .register_type::<Faction>()
        // `Actors` runs after `Terrain`, where `hex_map` spawns the tiles this
        // system queries to find the surface to stand on. The set boundary also
        // provides the sync point that makes those tiles queryable at all —
        // `Commands`-spawned entities are invisible until the queue is applied.
        .add_systems(
            OnEnter(Screen::Gameplay),
            spawn_units.in_set(GameplaySetup::Actors),
        )
        .add_systems(OnExit(Screen::Gameplay), despawn_units)
        .add_observer(on_tile_clicked);
}

fn despawn_units(mut commands: Commands, units: Query<Entity, With<Faction>>) {
    for entity in &units {
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
    mut players: Query<(Entity, &StandsOn, &Body, Option<&mut Turn>), With<Player>>,
    settings: Option<Res<PlayerSettings>>,
    table: Option<Res<SubstanceTable>>,
    mode: Option<Res<State<Mode>>>,
) {
    let (Some(settings), Some(table)) = (settings, table) else {
        return;
    };

    // Every resource here is an `Option`. Observers are global: this one fires on the
    // title screen, in menus, and before anything has loaded. Bevy validates system
    // parameters *before* the body runs, so a plain `Res<T>` panics in those states
    // no matter what the body checks — which is a crash this codebase has already
    // shipped once.
    //
    // No mode at all means we are not in gameplay, so a click cannot be a move.
    let Some(mode) = mode else {
        return;
    };

    // The click identifies a tile *entity*, which resolves to one specific surface
    // even where several share a coordinate. Picking is the right input for exactly
    // that reason: it never has to guess which surface was meant.
    let clicked = event.event_target();
    let Ok((pos, _, _, _)) = tiles.get(clicked) else {
        return;
    };

    for (entity, standing, body, turn) in players.iter_mut() {
        // In combat a click is only a move if it is this unit's turn. Out of combat
        // everything moves freely — that is the whole difference between the modes.
        if *mode.get() == Mode::Combat && turn.is_none() {
            continue;
        }

        // Footing and the destination are resolved per body, because whether a surface
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

        // A route of N surfaces costs N-1 steps: the first entry is where the piece
        // already stands.
        let cost = u32::try_from(steps.len().saturating_sub(1)).unwrap_or(u32::MAX);
        if let Some(mut turn) = turn {
            if cost > turn.movement_left {
                // Too far for what is left of this turn. Refusing outright rather
                // than walking partway keeps the click meaning one thing.
                continue;
            }
            turn.movement_left -= cost;
        }

        let animation: Transformation = HexPathingLine::new(&steps, settings.speed).into();
        commands
            .entity(entity)
            .insert((animation, StandsOn(destination)));
    }
}

/// Resolves a coordinate written in a settings file, falling back to the map centre.
///
/// Both failures are a designer's typo rather than a bug, so both say so in the log
/// and carry on. Refusing to start would leave someone staring at a loading screen
/// with no idea which of two numbers was wrong.
fn coord_from(setting: CubeCoord, unit: &str) -> HexCoord {
    HexCoord::try_new_cubic(setting.x, setting.y, setting.z).unwrap_or_else(|| {
        warn!(
            "scenario.ron: {unit} is at ({}, {}, {}), which does not sum to zero — \
             using the centre of the map instead",
            setting.x, setting.y, setting.z
        );
        HexCoord::ORIGIN
    })
}

/// Places both units on the terrain.
///
/// Runs in `Actors`, after the map has built and flushed its tiles. Reading them any
/// earlier finds nothing and drops the units to ground level — a bug that renders
/// perfectly and reports nothing, which is why the set boundary exists.
fn spawn_units(
    mut commands: Commands,
    assets: Res<GameAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    tiles: TileQuery,
    table: Res<SubstanceTable>,
    settings: Res<PlayerSettings>,
    scenario: Res<ScenarioSettings>,
) {
    // Both units share a body for now. When lattices land, size becomes a property of
    // the unit rather than a global setting, and this is where that starts.
    let body = Body {
        levels_tall: settings.levels_tall,
    };
    let footing = Footing::from_tiles(tiles.iter(), &table, body);

    let player_material = materials.add(StandardMaterial::from(to_color(settings.color)));
    // Hostile pieces are a colder colour, which is the only way to tell them apart
    // until they have their own meshes.
    let enemy_material = materials.add(StandardMaterial::from(Color::srgb(0.25, 0.45, 0.9)));

    spawn_unit(
        &mut commands,
        &assets,
        UnitSpawn {
            coord: coord_from(scenario.player, "player"),
            faction: Faction::Player,
            material: player_material,
            name: "Player",
            settings: &settings,
            body,
        },
        &footing,
    );

    spawn_unit(
        &mut commands,
        &assets,
        UnitSpawn {
            coord: coord_from(scenario.enemy, "enemy"),
            faction: Faction::Hostile,
            material: enemy_material,
            name: "Enemy",
            settings: &settings,
            body,
        },
        &footing,
    );
}

/// Everything that differs between one unit and the next.
///
/// Grouped into a struct because the alternative is an eight-argument function where
/// two of the arguments are `&str` and easy to swap by accident.
struct UnitSpawn<'a> {
    coord: HexCoord,
    faction: Faction,
    material: Handle<StandardMaterial>,
    name: &'static str,
    settings: &'a PlayerSettings,
    body: Body,
}

fn spawn_unit(commands: &mut Commands, assets: &GameAssets, spawn: UnitSpawn, footing: &Footing) {
    // Stand on the lowest surface at the coordinate that this body fits on — the
    // ground, rather than any bridge built over it.
    let standing = footing.ground(spawn.coord).unwrap_or_else(|| {
        warn!(
            "scenario.ron: nothing at {:?} that the {} can stand on — \
             using the centre of the map instead",
            spawn.coord, spawn.name
        );
        footing.ground(HexCoord::ORIGIN).unwrap_or(Standing {
            pos: TilePos::new(HexCoord::ORIGIN, 0),
            span: HexSpan::new(0.0, f32::EPSILON),
        })
    });

    let scale = spawn.settings.scale;
    let [mesh_a, mesh_b] = assets.player_pieces.clone();

    let child_transform = Transform {
        // Offsets the mesh so its origin sits on the tile centre.
        translation: Vec3::new(-scale, -scale, -10. * scale),
        scale: Vec3::splat(scale),
        ..default()
    };

    let mut unit = commands.spawn((
        Transform::from_translation(standing.world_position()),
        Visibility::default(),
        StandsOn(standing),
        spawn.body,
        spawn.faction,
        Name::new(spawn.name),
    ));

    match spawn.faction {
        Faction::Player => unit.insert(Player),
        Faction::Hostile => unit.insert(Enemy),
    };

    unit.with_children(|parent| {
        // `Pickable::IGNORE` so clicks pass through to the tiles below. Without it a
        // unit standing between the cursor and the ground swallows the click and
        // movement silently stops working wherever a piece happens to be.
        parent.spawn((
            Mesh3d(mesh_a),
            MeshMaterial3d(spawn.material.clone()),
            child_transform,
            Pickable::IGNORE,
        ));
        parent.spawn((
            Mesh3d(mesh_b),
            MeshMaterial3d(spawn.material),
            child_transform,
            Pickable::IGNORE,
        ));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opposing_factions_are_hostile() {
        assert!(Faction::Player.is_hostile_to(Faction::Hostile));
        assert!(Faction::Hostile.is_hostile_to(Faction::Player));
    }

    /// The rule is deliberately not `self != other`. Writing it that way passes today
    /// and breaks the moment a third faction exists: a neutral bystander would come
    /// out hostile to everyone, including other bystanders.
    #[test]
    fn a_faction_is_not_hostile_to_itself() {
        assert!(!Faction::Player.is_hostile_to(Faction::Player));
        assert!(!Faction::Hostile.is_hostile_to(Faction::Hostile));
    }
}
