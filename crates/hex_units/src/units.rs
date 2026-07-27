//! The units on the map: what they are, and how they get placed.
//!
//! A unit is a [`Body`] standing on a surface, wearing a [`Faction`] so anything else
//! can tell friend from foe without naming concrete types. `Player` and `Enemy` are
//! markers on top of that, for the two things that currently exist.
//!
//! Spawning reads the active entry from `assets/config/scenarios.ron`, which is
//! deliberately the crudest thing that works: two coordinates. It exists so terrain
//! can be tried out without writing Rust, not because it is the encounter format the
//! game will ship.

use bevy::picking::events::{Click, Pointer};
use bevy::picking::Pickable;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use hex_anim::Transformation;
use hex_assets::{
    to_color, CubeCoord, GameAssets, PlayerSettings, ScenarioPlacement, ScenarioSettings,
    SubstanceTable,
};
use hex_core::{
    GameplaySetup, GameplaySetupFailure, Headroom, HexCoord, HexSpan, HexTile, MapAnchorId,
    MapAnchors, Mode, Pause, Screen, SubstanceId, TerrainReady, TilePos, TraversalProfile, Turn,
};

use crate::movement::{route, Body, Footing, MovementCrossings, Standing};
use crate::pathing::{reached_step_index, HexPathingLine};
use crate::selection::Selected;

/// Tiles as units see them.
///
/// Terrain is read off the entities rather than from a map resource, so this crate has
/// no dependency on `hex_map` at all. However the map is generated or stored, this
/// query keeps working.
///
/// [`Headroom`] comes along because standability depends on it, but the query does not
/// filter on it: what counts as enough room depends on the body asking, so the filter
/// belongs in [`Footing::from_tiles`] where the body is known.
pub(crate) type TileQuery<'w, 's> = Query<
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

/// Which surface a piece is **actually standing on**, right now.
///
/// A coordinate is not enough: surfaces stacked in one column are separate places,
/// so a piece on a bridge and a piece on the ground beneath it share a horizontal
/// address but not a location.
///
/// This used to be written the moment a move was *ordered*, which made it the
/// destination rather than the position for as long as the walk took. Everything that
/// asks where a unit is — engagement above all — was therefore reading the future.
/// A single click across the map started a fight instantly, ended one while the piece
/// was still walking away, and could cross straight through engaging distance without
/// ever noticing if the far end happened to be out of range. The committed route lives
/// in [`MovingTo`] now, and this means what it says.
#[derive(Component, Debug, Clone, Copy)]
pub struct StandsOn(pub Standing);

/// A walk in progress, and every surface it passes over.
///
/// Paired with a [`Transformation`]: that one animates, this one remembers what the
/// animation *means*. The whole path rather than just the endpoint, because a fight
/// starting mid-stride has to put the piece down on a real hex, and the animation
/// cannot say which surface each world-space waypoint represents.
///
/// The first entry is the surface left behind, so a path is always at least one long.
#[derive(Component, Debug, Clone)]
pub struct MovingTo {
    /// Surfaces walked over, starting where the walk began.
    pub path: Vec<Standing>,
    /// Speed captured when the route was committed.
    ///
    /// Settings can hot-reload while a piece is walking. Reconciliation must keep the
    /// schedule the animation was built with rather than silently adopting a new one.
    speed: f32,
    /// Index of the last route step published as [`StandsOn`].
    reconciled_step: usize,
}

impl MovingTo {
    /// Records a committed route using the same speed as its animation.
    #[must_use]
    pub fn new(path: Vec<Standing>, speed: f32) -> Self {
        Self {
            path,
            speed,
            reconciled_step: 0,
        }
    }

    fn reached_at(&self, elapsed: f64) -> Option<usize> {
        reached_step_index(&self.path, self.speed, elapsed)
    }
}

/// Requests that an in-flight walk stop at a particular completed waypoint.
///
/// Combat attaches this when a large animation tick crosses into engagement range and
/// then leaves it again. The request survives until `OnEnter(Mode::Combat)`, even when
/// the visual animation reached its destination and [`MovingTo`] was already removed.
#[derive(Component, Debug, Clone, Copy)]
pub struct StopMovingAt(Standing);

impl StopMovingAt {
    /// Stops the walk at `standing` when combat begins.
    #[must_use]
    pub const fn new(standing: Standing) -> Self {
        Self(standing)
    }
}

/// Which side a unit is on.
///
/// A component rather than a `Player`-or-not check, so "is this hostile to me" is one
/// comparison and does not have to enumerate every unit type that exists. Neutral
/// parties and enemies that turn on each other both fit without a new mechanism.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
            spawn_units
                .in_set(GameplaySetup::Actors)
                .run_if(resource_exists::<TerrainReady>),
        )
        .add_systems(OnExit(Screen::Gameplay), despawn_units)
        .add_observer(on_tile_clicked);
}

fn despawn_units(mut commands: Commands, units: Query<Entity, With<Faction>>) {
    for entity in &units {
        commands.entity(entity).despawn();
    }
}

/// Global picking observer: when any `HexTile` is clicked, animate the player over to
/// that tile, one hex at a time along the route the search found.
///
/// Only a [`Selected`] piece moves. With one player piece that piece is always the
/// selection, so this changes nothing today — but it is what makes the click
/// unambiguous once there is a party, and it ties the move to the same piece whose
/// range and path are being drawn.
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
    mut players: Query<
        (Entity, &StandsOn, &Body, Option<&mut Turn>),
        // Both filters are rules. `Transformation` covers the visible walk;
        // `MovingTo` also covers the deferred landing frame after an animation has
        // been removed but before its route has been reconciled. Accepting a click in
        // that gap would route from a stale `StandsOn` and overwrite the arrival.
        (
            With<Player>,
            With<Selected>,
            Without<Transformation>,
            Without<MovingTo>,
        ),
    >,
    settings: Option<Res<PlayerSettings>>,
    table: Option<Res<SubstanceTable>>,
    mode: Option<Res<State<Mode>>>,
    pause: Option<Res<State<Pause>>>,
) {
    let (Some(settings), Some(table)) = (settings, table) else {
        return;
    };

    // Paused means paused. `PausableSystems` gates *systems*, and this is a global
    // observer — it is not in that set and never was, so a click landing behind the
    // pause overlay would spend the turn and start a walk that then plays out the
    // moment the game resumes.
    if pause.is_some_and(|pause| pause.get().0) {
        return;
    }

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
        // `MovingTo`, not `StandsOn`. The piece has not gone anywhere yet — it has been
        // told where to go, and reconciliation advances the position as each leg lands.
        commands
            .entity(entity)
            .insert((animation, MovingTo::new(steps, settings.speed)));
    }
}

/// Keeps logical position aligned with the whole route steps already reached.
///
/// Registered by [`movement::plugin`](crate::movement::plugin) rather than here,
/// because it is bookkeeping every unit needs and nothing to do with spawning a
/// scenario. `hex_combat`'s tests want the former without the latter.
///
/// Updating on each completed leg is what lets engagement observe a route that enters
/// range and later leaves it again. Updating only at the final destination makes both
/// endpoints truthful while every point between them is invisible to gameplay.
///
/// The finished case is still reconciled from the **absence** of a
/// [`Transformation`] rather than `RemovedComponents`: ordered system sets apply the
/// driver's deferred removal before this runs, so the destination and route cleanup
/// land in the same frame.
pub(crate) fn reconcile_movement(
    mut commands: Commands,
    mut crossings: ResMut<MovementCrossings>,
    mut moving_units: Query<(
        Entity,
        &mut MovingTo,
        &mut StandsOn,
        Option<&Transformation>,
    )>,
) {
    crossings.clear();

    for (entity, mut moving, mut standing, animation) in &mut moving_units {
        let reached_index = animation
            .and_then(|animation| moving.reached_at(animation.elapsed()))
            .or_else(|| moving.path.len().checked_sub(1));

        if let Some(reached_index) = reached_index {
            let first_new = moving.reconciled_step.saturating_add(1);
            if first_new <= reached_index {
                for index in first_new..=reached_index {
                    if let Some(reached) = moving.path.get(index).copied() {
                        crossings.push(entity, reached);
                    }
                }

                if let Some(reached) = moving.path.get(reached_index).copied() {
                    if standing.0 != reached {
                        standing.0 = reached;
                    }
                    moving.reconciled_step = reached_index;
                }
            }
        }

        if animation.is_none() {
            commands.entity(entity).remove::<MovingTo>();
        }
    }
}

/// Stops a walk where it is when a fight starts.
///
/// Committing to a long walk and then being ambushed halfway should leave the piece
/// where the ambush happened, not deliver it to a destination chosen before anyone
/// knew there was a fight.
///
/// It snaps to the **nearest whole step** rather than to the exact interpolated point,
/// because a piece standing between two hexes is not a position the rest of the game
/// can express — every rule here is written in terms of a surface.
pub(crate) fn halt_on_combat(
    mut commands: Commands,
    mut walking: Query<
        (
            Entity,
            Option<&MovingTo>,
            &mut Transform,
            Option<&StopMovingAt>,
        ),
        Or<(With<MovingTo>, With<StopMovingAt>)>,
    >,
) {
    for (entity, moving, mut transform, requested) in &mut walking {
        let stopped = requested.map(|requested| requested.0).or_else(|| {
            moving.and_then(|moving| nearest_step(&moving.path, transform.translation))
        });
        let Some(stopped) = stopped else {
            continue;
        };
        transform.translation = stopped.world_position();
        commands
            .entity(entity)
            .insert(StandsOn(stopped))
            .remove::<MovingTo>()
            .remove::<StopMovingAt>()
            .remove::<Transformation>();
    }
}

/// The step in `path` closest to a world position.
///
/// `total_cmp` rather than `partial_cmp`: distances are never `NaN` here, and a
/// comparison that cannot fail needs no unwrap to explain away.
fn nearest_step(path: &[Standing], at: Vec3) -> Option<Standing> {
    path.iter()
        .min_by(|a, b| {
            a.world_position()
                .distance_squared(at)
                .total_cmp(&b.world_position().distance_squared(at))
        })
        .copied()
}

/// Resolves a coordinate written in a settings file, falling back to the map centre.
///
/// Both failures are a designer's typo rather than a bug, so both say so in the log
/// and carry on. Refusing to start would leave someone staring at a loading screen
/// with no idea which of two numbers was wrong.
fn coord_from(setting: CubeCoord, unit: &str) -> HexCoord {
    HexCoord::try_new_cubic(setting.x, setting.y, setting.z).unwrap_or_else(|| {
        warn!(
            "scenarios.ron: {unit} is at ({}, {}, {}), which does not sum to zero — \
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
    anchors: Option<Res<MapAnchors>>,
) {
    // Both units share a body for now. When lattices land, size becomes a property of
    // the unit rather than a global setting, and this is where that starts.
    let body = Body::new(TraversalProfile::WALKER);
    let footing = Footing::from_tiles(tiles.iter(), &table, body);

    let player_material = materials.add(StandardMaterial::from(to_color(settings.color)));
    // Hostile pieces are a colder colour, which is the only way to tell them apart
    // until they have their own meshes.
    let enemy_material = materials.add(StandardMaterial::from(Color::srgb(0.25, 0.45, 0.9)));

    let anchors = anchors.as_deref();
    let player = placement_from(&scenario.player, "player", anchors).and_then(|placement| {
        spawn_unit(
            &mut commands,
            &assets,
            UnitSpawn {
                placement,
                faction: Faction::Player,
                material: player_material,
                name: "Player",
                settings: &settings,
                body,
            },
            &footing,
        )
    });
    if let Err(reason) = player {
        error!("{reason}");
        commands.insert_resource(GameplaySetupFailure::new(reason));
        return;
    }

    let enemy = placement_from(&scenario.enemy, "enemy", anchors).and_then(|placement| {
        spawn_unit(
            &mut commands,
            &assets,
            UnitSpawn {
                placement,
                faction: Faction::Hostile,
                material: enemy_material,
                name: "Enemy",
                settings: &settings,
                body,
            },
            &footing,
        )
    });
    if let Err(reason) = enemy {
        error!("{reason}");
        commands.insert_resource(GameplaySetupFailure::new(reason));
    }
}

/// A placement resolved as far as scenario settings permit.
enum ResolvedPlacement {
    /// Authored placements choose the lowest fitting surface at this coordinate.
    Fixed(HexCoord),
    /// Generated anchors identify one exact surface, including its level.
    Anchor { id: MapAnchorId, pos: TilePos },
}

/// Resolves a scenario placement without silently substituting generated anchors.
fn placement_from(
    setting: &ScenarioPlacement,
    unit: &str,
    anchors: Option<&MapAnchors>,
) -> Result<ResolvedPlacement, String> {
    match setting {
        ScenarioPlacement::Fixed(coord) => Ok(ResolvedPlacement::Fixed(coord_from(*coord, unit))),
        ScenarioPlacement::Anchor(name) => {
            let id = MapAnchorId::from(name.as_str());
            let Some(anchors) = anchors else {
                return Err(format!(
                    "The {unit} uses map anchor \"{id}\", but the active map published no anchors."
                ));
            };
            let Some(pos) = anchors.get(&id) else {
                return Err(format!("The {unit} uses missing map anchor \"{id}\"."));
            };
            Ok(ResolvedPlacement::Anchor { id, pos })
        }
    }
}

/// Everything that differs between one unit and the next.
///
/// Grouped into a struct because the alternative is an eight-argument function where
/// two of the arguments are `&str` and easy to swap by accident.
struct UnitSpawn<'a> {
    placement: ResolvedPlacement,
    faction: Faction,
    material: Handle<StandardMaterial>,
    name: &'static str,
    settings: &'a PlayerSettings,
    body: Body,
}

fn spawn_unit(
    commands: &mut Commands,
    assets: &GameAssets,
    spawn: UnitSpawn,
    footing: &Footing,
) -> Result<(), String> {
    let standing = match spawn.placement {
        // Stand on the lowest surface at an authored coordinate that this body fits
        // on: the ground, rather than any bridge built over it. Preserve the existing
        // authored-map fallback for a designer typo.
        ResolvedPlacement::Fixed(coord) => footing.ground(coord).unwrap_or_else(|| {
            warn!(
                "scenarios.ron: nothing at {:?} that the {} can stand on — \
                 using the centre of the map instead",
                coord, spawn.name
            );
            footing.ground(HexCoord::ORIGIN).unwrap_or(Standing {
                pos: TilePos::new(HexCoord::ORIGIN, 0),
                span: HexSpan::new(0.0, f32::EPSILON),
            })
        }),
        // A generated anchor promises one exact surface. Falling back to the lowest
        // surface or the origin would hide a generator/validation defect and may put
        // the unit on the ground beneath a bridge.
        ResolvedPlacement::Anchor { id, pos } => {
            let Some(standing) = footing.at(pos) else {
                return Err(format!(
                    "Map anchor \"{id}\" for the {} points to {pos:?}, which its body cannot \
                     stand on.",
                    spawn.name
                ));
            };
            standing
        }
    };

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
    Ok(())
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
