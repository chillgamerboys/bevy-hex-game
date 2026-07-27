//! Showing a piece where it can go before it commits to going there.
//!
//! Clicking a tile used to move the player instantly, and refuse silently when it
//! could not: not your turn, no route, too far, nothing standable. Five different
//! reasons all looked identical from the outside — nothing happened. This module
//! makes the answer visible *before* the click, so a tile that cannot be reached is
//! simply not lit.
//!
//! Three things are drawn:
//!
//! | | |
//! |---|---|
//! | a ring | at the feet of the acting unit, or of the selection out of combat |
//! | a faint tint | over every surface within this turn's movement — combat only |
//! | a stronger tint | along the way to whatever the cursor is over |
//!
//! There is **no range tint while exploring**, because there is no budget there: every
//! connected surface is reachable, and a tint over the whole map says nothing.
//!
//! # One search, not one per tile
//!
//! Both tints come out of a single [`Reach`](crate::movement::Reach). Its keys are the
//! range and a walk backwards down its predecessors is the path, so hovering costs a
//! lookup rather than a search. What is expensive is
//! [`Footing::from_tiles`](crate::movement::Footing::from_tiles), which reads every
//! tile entity on the map — so the search is rebuilt when the *selection* changes, not
//! when the cursor does. Moving the mouse redraws; it does not re-solve.

use bevy::picking::events::{Out, Over, Pointer};
use bevy::picking::Pickable;
use bevy::prelude::*;

use hex_assets::{GameAssets, SubstanceTable};
use hex_core::{
    CameraFocusTarget, GameplaySetup, HexTile, Mode, PausableSystems, Screen, TilePos, Turn,
};

use crate::movement::{Body, Footing, Reach, Standing};
use crate::units::{Faction, Player, StandsOn, TileQuery};

/// Thickness of an overlay cap in world units. Thin enough to read as paint on the
/// ground rather than as a slab sitting on it.
const CAP_THICKNESS: f32 = 0.02;

/// How much of a tile's width the range tint covers, leaving a gap at the edges so
/// individual hexes stay legible instead of merging into one blob.
const RANGE_INSET: f32 = 0.86;

/// How much of a tile's width the path tint covers. Narrower than the range, so on a
/// tile carrying both the range still shows as a border around the path.
const PATH_INSET: f32 = 0.58;

/// How far above a surface the range cap floats.
const RANGE_LIFT: f32 = 0.01;

/// How far above a surface the path cap floats. Above [`RANGE_LIFT`] so the two never
/// contend for the same pixels where a path crosses its own range.
const PATH_LIFT: f32 = 0.05;

/// How far above a unit's feet the ring sits.
const RING_LIFT: f32 = 0.03;

/// Marks the unit whose movement is being previewed.
///
/// Separate from [`Player`] because "which piece is the interface talking about" is
/// not the same question as "which pieces does this human control", and a party of
/// more than one makes the difference load-bearing.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct Selected;

/// The surface under the cursor, or [`None`] when the cursor is off the map.
///
/// A resource rather than a component because it is a property of the pointer, not of
/// any tile, and because the tile it refers to is despawned and respawned wholesale
/// every time terrain is edited.
#[derive(Resource, Default, Debug)]
pub struct HoveredSurface(pub Option<TilePos>);

/// Marks a tile tinted as being within reach this turn.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct RangeOverlay;

/// Marks a tile tinted as lying on the way to what the cursor is over.
///
/// Distinct from [`RangeOverlay`] rather than one marker with a flag, because the two
/// answer different questions — "could I get there" and "how would I get there" — and
/// a test that cannot tell them apart cannot check that exploring draws the second
/// without the first.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct PathOverlay;

/// Marks a ring, and remembers the unit it belongs to.
///
/// Holding the owner rather than relying on the hierarchy keeps the reconcile a single
/// flat comparison against who the ring *should* be under.
#[derive(Component)]
pub struct UnitRing(Entity);

/// Meshes and materials shared by every overlay.
///
/// Four materials and one mesh for the whole game, following the `MaterialCache`
/// precedent in `hex_map`: a highlight over sixty tiles must not be sixty materials.
#[derive(Resource)]
struct OverlayAssets {
    range: Handle<StandardMaterial>,
    path: Handle<StandardMaterial>,
    ring: Handle<Mesh>,
    player_ring: Handle<StandardMaterial>,
    enemy_ring: Handle<StandardMaterial>,
}

/// How many times the terrain has been rebuilt.
///
/// `apply_terrain_edits` despawns the **entire** grid and respawns it on any accepted
/// edit, so every tile entity gets a new id and the ground a route was found across may
/// no longer exist. Nothing about the *unit* changes when that happens, which is
/// exactly why a preview keyed only on the unit outlives the terrain it describes.
///
/// A counter rather than a comparison of the tiles themselves: the map publishes a few
/// thousand tile entities, and the question is only ever "is this the same terrain I
/// last looked at".
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerrainRevision(pub u64);

/// What a [`Reach`] was computed for.
///
/// Two levels of absence, which are different things: no key at all means nothing is
/// being previewed — in combat, on somebody else's turn. A key with `budget: None`
/// means exploring, where movement is unlimited.
///
/// `terrain` is here because the other three can all be unchanged while the ground
/// underneath them is replaced.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct PreviewKey {
    unit: Entity,
    from: TilePos,
    budget: Option<u32>,
    terrain: u64,
}

/// The current search, and what is drawn from it.
#[derive(Resource, Default)]
struct MovementPreview {
    /// What `reach` was computed for, and the test for whether it is still valid.
    of: Option<PreviewKey>,
    reach: Option<Reach>,
    /// The hovered surface the drawn overlays reflect.
    shown: Option<TilePos>,
}

/// Registers selection, the ring, and the movement overlays.
pub fn plugin(app: &mut App) {
    app.register_type::<Selected>()
        .register_type::<CameraFocusTarget>()
        .register_type::<RangeOverlay>()
        .register_type::<PathOverlay>()
        .init_resource::<HoveredSurface>()
        .init_resource::<MovementPreview>()
        .init_resource::<TerrainRevision>()
        .add_systems(
            OnEnter(Screen::Gameplay),
            create_overlay_assets.in_set(GameplaySetup::Resources),
        )
        .add_systems(OnExit(Screen::Gameplay), clear_overlays)
        // Deliberately **not** pausable, and ordered before the overlays that read it.
        // `apply_terrain_edits` is in no set at all, so it runs while the game is
        // paused; a tracker that stopped with everything else would miss the rebuild
        // and never see it again — `Added` is a change tick, not a queue.
        .add_systems(Update, track_terrain_changes)
        .add_systems(
            Update,
            (
                select_a_player,
                reconcile_camera_focus_target,
                reconcile_rings,
                redraw_overlays,
            )
                .chain()
                .in_set(PausableSystems)
                .after(track_terrain_changes)
                .after(crate::movement::MovementSystems::Reconcile),
        )
        // Observers are global and fire in every state, including the title screen.
        // These two touch only `HoveredSurface`, which is initialised at startup and
        // therefore always present — Bevy validates system parameters *before* the
        // body runs, so an `Option` would be required for anything gameplay-scoped.
        .add_observer(on_tile_hovered)
        .add_observer(on_tile_unhovered);
}

/// Counts a rebuild whenever tile entities appear or disappear.
///
/// Keyed on the tiles themselves rather than on [`TerrainEdit`](hex_core::TerrainEdit),
/// which is a *request*: the map rejects edits below the floor, no-ops, and anything
/// non-diggable, and only then rebuilds. Reading the message would invalidate on edits
/// that changed nothing, and — worse for a test — the message is registered by
/// `hex_map`, so an app without the map does not have it at all.
///
/// This over-counts instead: the initial spawn and every screen entry look like a
/// rebuild. That costs one search, and erring toward too many rebuilds is the right
/// direction when the alternative is highlighting ground that no longer exists.
fn track_terrain_changes(
    mut revision: ResMut<TerrainRevision>,
    added: Query<(), Added<HexTile>>,
    mut removed: RemovedComponents<HexTile>,
) {
    // `read().count()` drains. Leaving the removals unread would replay them on the
    // next frame and count one teardown twice.
    let gone = removed.read().count();
    if gone > 0 || !added.is_empty() {
        revision.0 = revision.0.wrapping_add(1);
    }
}

/// An unlit, blended cap material.
///
/// `unlit` because a highlight that dims as the sun moves reads as a lighting fault
/// rather than as a highlight, and `depth_bias` because the cap sits a hair above a
/// tile top it is parallel to.
///
/// **`alpha_mode` is set explicitly and must stay that way.**
/// `StandardMaterial::from(Color)` infers `Blend` when the alpha is below one; a
/// struct literal like this one does not, and leaves it `Opaque` — which silently
/// discards the alpha and draws a solid slab over the terrain.
fn cap_material(color: Color, depth_bias: f32) -> StandardMaterial {
    StandardMaterial {
        base_color: color,
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        depth_bias,
        ..default()
    }
}

/// Builds the shared overlay meshes and materials, once per visit to gameplay.
fn create_overlay_assets(
    mut commands: Commands,
    existing: Option<Res<OverlayAssets>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if existing.is_some() {
        return;
    }

    // A pale warm white. Deliberately not either piece's colour: red is the player
    // and blue is the enemy, and a highlight that borrows one of them would read as
    // ownership rather than as reachable ground.
    let range = materials.add(cap_material(Color::srgba(1.0, 0.94, 0.75, 0.22), 1.0));
    let path = materials.add(cap_material(Color::srgba(1.0, 0.9, 0.55, 0.6), 2.0));

    // Sized inside a hex — the grid's circumradius is 1.0, so its inradius is about
    // 0.87 and a ring at 0.78 clears the edge on every side.
    let ring = meshes.add(Mesh::from(Torus::new(0.68, 0.78)));

    // Rings borrow their unit's colour, because a ring *is* about ownership — it says
    // whose turn it is.
    let player_ring = materials.add(cap_material(Color::srgba(1.0, 0.45, 0.4, 0.85), 1.0));
    let enemy_ring = materials.add(cap_material(Color::srgba(0.45, 0.65, 1.0, 0.85), 1.0));

    commands.insert_resource(OverlayAssets {
        range,
        path,
        ring,
        player_ring,
        enemy_ring,
    });
}

/// Keeps exactly one player piece selected.
///
/// The agreed behaviour is that the acting unit is selected automatically and a click
/// re-selects. With a single player piece those collapse into the same thing: there is
/// nothing else to select, so it is simply always the selection. This is where a party
/// of several would change, and the reason `Selected` exists as its own marker rather
/// than being inferred from [`Player`].
fn select_a_player(
    mut commands: Commands,
    unselected: Query<Entity, (With<Player>, Without<Selected>)>,
    selected: Query<(), With<Selected>>,
) {
    if !selected.is_empty() {
        return;
    }
    let Some(player) = unselected.iter().next() else {
        return;
    };
    commands.entity(player).insert(Selected);
}

/// Projects the authoritative unit selection into the shared camera vocabulary.
///
/// Reconciliation handles selection changes regardless of which future interaction
/// caused them and removes stale targets rather than leaving the camera attached to a
/// unit the interface is no longer controlling.
fn reconcile_camera_focus_target(
    mut commands: Commands,
    selected: Query<(Entity, &StandsOn), With<Selected>>,
    focused: Query<(Entity, &CameraFocusTarget)>,
) {
    for (entity, _) in &focused {
        if selected.get(entity).is_err() {
            commands.entity(entity).remove::<CameraFocusTarget>();
        }
    }

    for (entity, standing) in &selected {
        let wanted = CameraFocusTarget::new(standing.0.pos);
        let needs_update = match focused.get(entity) {
            Ok((_, current)) => *current != wanted,
            Err(_) => true,
        };
        if needs_update {
            commands.entity(entity).insert(wanted);
        }
    }
}

/// Puts a ring at the feet of the unit the interface is currently about.
///
/// **In combat that is whoever holds a [`Turn`]; out of combat it is the selection.**
/// Keying it on `Turn` alone was the first attempt and left exploring with no ring at
/// all — there is no turn out there, so nothing was ever marked and the piece you
/// control looked no different from anything else on the map.
///
/// A child of the unit, so it rides the walk animation without any work: `hex_anim`
/// drives the root transform and the ring comes along.
///
/// **Reconciled from state rather than driven by `Added` and `RemovedComponents`.**
/// `advance_turn` takes the marker off one unit and puts it on the next in the same
/// system on the same frame. A reconcile does not care in which order those two facts
/// land; a pair of event readers does.
fn reconcile_rings(
    mut commands: Commands,
    overlays: Option<Res<OverlayAssets>>,
    acting: Query<(Entity, &Faction), With<Turn>>,
    selected: Query<(Entity, &Faction), With<Selected>>,
    rings: Query<(Entity, &UnitRing)>,
) {
    let Some(overlays) = overlays else {
        return;
    };

    // Acting first: during combat the selection is still sitting on the player, and
    // ringing it as well would say two units are up at once.
    let wanted = acting.iter().next().or_else(|| selected.iter().next());

    for (ring, owner) in &rings {
        if wanted.is_none_or(|(unit, _)| unit != owner.0) {
            // `try_` because the owner may have been despawned earlier this frame —
            // on leaving the screen — which takes its children with it.
            commands.entity(ring).try_despawn();
        }
    }

    let Some((unit, faction)) = wanted else {
        return;
    };
    if rings.iter().any(|(_, owner)| owner.0 == unit) {
        return;
    }

    let material = match faction {
        Faction::Player => overlays.player_ring.clone(),
        Faction::Hostile => overlays.enemy_ring.clone(),
    };

    commands.entity(unit).with_children(|parent| {
        parent.spawn((
            Mesh3d(overlays.ring.clone()),
            MeshMaterial3d(material),
            Transform::from_xyz(0.0, RING_LIFT, 0.0),
            // Without this the ring swallows clicks on the tile its unit stands on,
            // which is the bug `Pickable::IGNORE` on the piece already exists to
            // avoid.
            Pickable::IGNORE,
            UnitRing(unit),
            Name::new("UnitRing"),
        ));
    });
}

/// Records the surface under the cursor.
fn on_tile_hovered(
    event: On<Pointer<Over>>,
    tiles: TileQuery,
    mut hovered: ResMut<HoveredSurface>,
) {
    let Ok((pos, _, _, _)) = tiles.get(event.event_target()) else {
        return;
    };
    hovered.0 = Some(*pos);
}

/// Forgets it again, unless the cursor has already moved on to another tile.
fn on_tile_unhovered(
    event: On<Pointer<Out>>,
    tiles: TileQuery,
    mut hovered: ResMut<HoveredSurface>,
) {
    let Ok((pos, _, _, _)) = tiles.get(event.event_target()) else {
        return;
    };
    // `Over` for the tile being entered can arrive before `Out` for the one being
    // left. Clearing unconditionally would then erase a hover that had already moved
    // on, and the path would flicker out every time the cursor crossed a tile edge.
    if hovered.0 == Some(*pos) {
        hovered.0 = None;
    }
}

/// Redraws the tints when what they show has changed, and not otherwise.
///
/// Every resource here is an `Option` for the same reason the click observer's are:
/// this runs inside gameplay, but `GameplaySetup::Resources` and the asset load are
/// what put them there, and a plain `Res<T>` would make the system's first frame a
/// question of ordering rather than of logic.
fn redraw_overlays(
    mut commands: Commands,
    mut preview: ResMut<MovementPreview>,
    hovered: Res<HoveredSurface>,
    assets: Option<Res<GameAssets>>,
    overlays: Option<Res<OverlayAssets>>,
    table: Option<Res<SubstanceTable>>,
    mode: Option<Res<State<Mode>>>,
    revision: Res<TerrainRevision>,
    tiles: TileQuery,
    selected: Query<(Entity, &StandsOn, &Body, Option<&Turn>), With<Selected>>,
    drawn: Query<Entity, DrawnOverlays>,
) {
    let (Some(assets), Some(overlays), Some(table), Some(mode)) = (assets, overlays, table, mode)
    else {
        return;
    };

    let selection = selected.iter().next();
    let key = selection.and_then(|(unit, standing, _, turn)| {
        let budget = match (mode.get(), turn) {
            // Somebody else's turn. Tinting a range the piece cannot use this turn
            // would promise a move that would then be refused.
            (Mode::Combat, None) => return None,
            (Mode::Combat, Some(turn)) => Some(turn.movement_left),
            // Exploring has no budget at all, which is why there is no range tint.
            (Mode::Exploring, _) => None,
        };
        Some(PreviewKey {
            unit,
            from: standing.0.pos,
            budget,
            terrain: revision.0,
        })
    });

    if key == preview.of && hovered.0 == preview.shown {
        return;
    }

    // The search depends on where the piece stands and what it has left, never on
    // where the cursor is. `Footing::from_tiles` reads every tile entity on the map —
    // around four thousand of them — so rebuilding it per mouse-move would cost
    // hundreds of thousands of reads a second to arrive at the same answer.
    if key != preview.of {
        preview.reach = selection.and_then(|(_, standing, body, _)| {
            let key = key?;
            let footing = Footing::from_tiles(tiles.iter(), &table, *body);
            Some(Reach::from(standing.0, &footing, key.budget))
        });
        preview.of = key;
    }
    preview.shown = hovered.0;

    for overlay in &drawn {
        commands.entity(overlay).despawn();
    }

    let Some(reach) = preview.reach.as_ref() else {
        return;
    };
    let Some(key) = key else {
        return;
    };

    if key.budget.is_some() {
        for standing in reach.surfaces() {
            if standing.pos == key.from {
                continue;
            }
            let tint = cap(&assets, &overlays.range, standing, RANGE_INSET, RANGE_LIFT);
            commands.spawn((tint, RangeOverlay, Name::new("RangeOverlay")));
        }
    }

    let Some(target) = hovered.0 else {
        return;
    };
    let Some(path) = reach.path_to(target) else {
        return;
    };
    // Skipping the first entry: it is the surface the piece already stands on, and
    // tinting it would make a legal move look like it started one hex early.
    for standing in path.iter().skip(1) {
        let tint = cap(&assets, &overlays.path, *standing, PATH_INSET, PATH_LIFT);
        commands.spawn((tint, PathOverlay, Name::new("PathOverlay")));
    }
}

/// Everything drawn on the ground by this module, for clearing in one pass.
type DrawnOverlays = Or<(With<RangeOverlay>, With<PathOverlay>)>;

/// One tinted cap, sitting on a surface.
///
/// The tile mesh is origin-centred and exactly one unit tall, so scaling Y by the
/// thickness and translating to `top + lift + thickness / 2` puts its underside at
/// `top + lift`. Building a [`HexSpan`](hex_core::HexSpan) for it would be the obvious
/// thing and is wrong: `HexSpan::new` asserts that the top is above the bottom, and a
/// cap this thin is exactly the degenerate case it refuses.
fn cap(
    assets: &GameAssets,
    material: &Handle<StandardMaterial>,
    standing: Standing,
    inset: f32,
    lift: f32,
) -> impl Bundle {
    (
        Mesh3d(assets.hex_tile.clone()),
        MeshMaterial3d(material.clone()),
        Transform {
            translation: standing
                .pos
                .coord
                .to_world(standing.span.top + lift + CAP_THICKNESS * 0.5),
            scale: Vec3::new(inset, CAP_THICKNESS, inset),
            ..default()
        },
        // Caps sit between the camera and the tile they mark. Without this they eat
        // the clicks and hovers that drive movement, and the feature breaks the exact
        // thing it exists to explain.
        Pickable::IGNORE,
        // Which surface this is marking. Keyed on the position and never on the tile
        // entity: `apply_terrain_edits` despawns and respawns the entire grid on any
        // accepted edit, so every tile `Entity` id is invalidated by a single dig.
        standing.pos,
    )
}

/// Clears everything on leaving gameplay.
///
/// Overlays are plain world entities rather than children of anything that gets torn
/// down, so nothing else would take them with it.
fn clear_overlays(
    mut commands: Commands,
    drawn: Query<Entity, DrawnOverlays>,
    mut preview: ResMut<MovementPreview>,
    mut hovered: ResMut<HoveredSurface>,
) {
    for overlay in &drawn {
        commands.entity(overlay).despawn();
    }
    *preview = MovementPreview::default();
    hovered.0 = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::{HexCoord, HexSpan};

    fn standing_at(position: TilePos) -> StandsOn {
        StandsOn(Standing {
            pos: position,
            span: HexSpan::new(0.0, 1.0),
        })
    }

    #[test]
    fn camera_focus_target_tracks_selection_and_exact_surface_changes() {
        let mut app = App::new();
        app.add_systems(Update, reconcile_camera_focus_target);

        let first_surface = TilePos::new(HexCoord::from_axial(2, -1), 7);
        let second_surface = TilePos::new(HexCoord::from_axial(-3, 2), 4);
        let first = app
            .world_mut()
            .spawn((Selected, standing_at(first_surface)))
            .id();
        let second = app
            .world_mut()
            .spawn((
                CameraFocusTarget::new(TilePos::ORIGIN),
                standing_at(second_surface),
            ))
            .id();

        app.update();

        assert_eq!(
            app.world()
                .entity(first)
                .get::<CameraFocusTarget>()
                .map(|target| target.surface),
            Some(first_surface),
            "the selected unit should publish its exact surface"
        );
        assert!(
            !app.world().entity(second).contains::<CameraFocusTarget>(),
            "a target without selection should be removed"
        );

        app.world_mut().entity_mut(first).remove::<Selected>();
        app.world_mut().entity_mut(second).insert(Selected);
        app.update();

        assert!(
            !app.world().entity(first).contains::<CameraFocusTarget>(),
            "the old selection should stop being the camera target"
        );
        assert!(
            app.world().entity(second).contains::<CameraFocusTarget>(),
            "the new selection should become the camera target"
        );
        assert_eq!(
            app.world()
                .entity(second)
                .get::<CameraFocusTarget>()
                .map(|target| target.surface),
            Some(second_surface)
        );

        let moved_surface = TilePos::new(HexCoord::from_axial(-2, 2), 5);
        app.world_mut()
            .entity_mut(second)
            .get_mut::<StandsOn>()
            .expect("the selected fixture should be standing")
            .0 = standing_at(moved_surface).0;
        app.update();

        assert_eq!(
            app.world()
                .entity(second)
                .get::<CameraFocusTarget>()
                .map(|target| target.surface),
            Some(moved_surface),
            "the focus projection should follow logical movement without reselection"
        );
    }

    #[test]
    fn automatic_selection_publishes_focus_in_the_same_update() {
        let mut app = App::new();
        app.add_systems(
            Update,
            (select_a_player, reconcile_camera_focus_target).chain(),
        );
        let surface = TilePos::new(HexCoord::from_axial(1, -1), 3);
        let player = app.world_mut().spawn((Player, standing_at(surface))).id();

        app.update();

        assert!(app.world().entity(player).contains::<Selected>());
        assert_eq!(
            app.world()
                .entity(player)
                .get::<CameraFocusTarget>()
                .map(|target| target.surface),
            Some(surface),
            "the chained reconciliation should observe deferred selection commands"
        );
    }
}
