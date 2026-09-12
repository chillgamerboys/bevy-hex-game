//! Showing a piece where it can go before it commits to going there.
//!
//! Clicking a tile used to move the player instantly, and refuse silently when it
//! could not: not your turn, no route, too far, nothing standable. Five different
//! reasons all looked identical from the outside — nothing happened. This module
//! makes the answer visible *before* the click: affordable tiles show their route,
//! connected terrain beyond the budget gets a refusal ×, and terrain with no route
//! remains unlit.
//!
//! Four things are drawn:
//!
//! | | |
//! |---|---|
//! | a ring | at the feet of the acting unit, or of the selection out of combat |
//! | a faint tint | over every surface within this turn's movement — combat only |
//! | a stronger tint | along the way to whatever the cursor is over |
//! | a rose × | on a connected destination this turn cannot afford |
//!
//! There is **no range tint while exploring**, because there is no budget there: every
//! connected surface is reachable, and a tint over the whole map says nothing.
//!
//! # Cached searches, not one per tile
//!
//! Combat caches one unbounded, disclosure-safe [`Reach`](crate::movement::Reach).
//! Its exact costs are sliced by the current budget for range and affordable paths;
//! a higher cost supplies the coarse "connected but too far" cue. The search includes
//! allied and currently presentable hostile occupancy but excludes fogged hostiles,
//! preventing hidden actors from becoming a presentation oracle. Command validation
//! remains authoritative. Exploring uses the same search without a budget slice.
//! Either way, a hover remains a lookup rather than a search. What is expensive is
//! [`Footing::from_tiles`](crate::movement::Footing::from_tiles), which reads every
//! tile entity on the map — so the search is rebuilt when its preview key changes,
//! not when the cursor does. Moving the mouse redraws; it does not re-solve.

use bevy::light::NotShadowCaster;
use bevy::picking::events::{Move, Out, Over, Pointer};
use bevy::picking::Pickable;
use bevy::prelude::*;

use hex_assets::{GameAssets, SubstanceTable};
use hex_core::{
    Busy, CameraFocusTarget, GameplaySetup, GameplaySystems, HexTile, Mode, PausableSystems,
    PerceptionSystems, PresentationOcclusion, PresentationOcclusionReason, Screen,
    TargetReticleRequest, TerrainRenderBatch, TilePos, TraversalBlockers, Turn, UnitId,
    WorldMarkerSuppression,
};

use crate::movement::{Body, FootingCache, Reach, Standing};
use crate::units::{
    resolve_tile_pointer_target, MovingTo, Party, Player, StandsOn, TileQuery, UnitRegistry,
};
use crate::AuthoredObjectOccupancy;
use crate::Faction;
use crate::UnitOccupancy;

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

/// Scale of each reused target tick in the out-of-range ×.
///
/// The underlying cuboid is 0.34 × 0.025 × 0.12 world units. These factors make two
/// crossed strokes about 0.75 units long and 0.09 wide, plainly different from every
/// hex cap even without colour.
const OUT_OF_RANGE_STROKE_SCALE: Vec3 = Vec3::new(2.2, 1.0, 0.75);

/// Clearance above the terrain for the refusal ×.
///
/// Tactical shroud caps occupy the layer through `surface top + 0.10`. Keeping the
/// stroke's underside at `+0.12`, with a larger depth bias than the shroud, preserves
/// movement feedback on shaded destinations as well as ordinary ground.
const OUT_OF_RANGE_LIFT: f32 = 0.12;

/// Half the shared target-tick mesh's unscaled thickness.
const TARGET_TICK_HALF_THICKNESS: f32 = 0.0125;

/// How far above a unit's feet the ring sits.
const RING_LIFT: f32 = 0.03;

/// How far above a unit's feet the segmented target reticle sits.
const TARGET_RETICLE_LIFT: f32 = 0.07;

/// Radius of the four target-reticle ticks around a unit.
const TARGET_RETICLE_RADIUS: f32 = 0.94;

/// A four-tick reticle stays unmistakably distinct from the continuous acting ring.
const TARGET_RETICLE_PARTS: usize = 4;

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

/// Marks a hovered destination that is connected but beyond this turn's budget.
///
/// This is deliberately a two-stroke × on one target rather than a potentially
/// map-spanning continuation. The cue answers why the click would refuse without
/// creating hundreds of entities when the cursor crosses a distant tile, and its
/// shape remains meaningful without colour.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct OutOfRangeOverlay;

/// Marks a ring, and remembers the unit it belongs to.
///
/// Holding the owner rather than relying on the hierarchy keeps the reconcile a single
/// flat comparison against who the ring *should* be under.
#[derive(Component)]
pub struct UnitRing(Entity);

/// One of the four world-space ticks marking a disclosure-authorized target.
///
/// Like [`UnitRing`], retaining the owner makes reconciliation independent of
/// hierarchy traversal. Each tick remains a child of the unit so composable unit
/// visibility and animation apply automatically.
#[derive(Component)]
pub struct TargetReticle(Entity);

/// Meshes and materials shared by every overlay.
///
/// Six materials and two meshes for the whole game, following the `MaterialCache`
/// precedent in `hex_map`: a highlight over sixty tiles must not be sixty materials.
#[derive(Resource)]
struct OverlayAssets {
    range: Handle<StandardMaterial>,
    path: Handle<StandardMaterial>,
    out_of_range: Handle<StandardMaterial>,
    ring: Handle<Mesh>,
    target_tick: Handle<Mesh>,
    player_ring: Handle<StandardMaterial>,
    enemy_ring: Handle<StandardMaterial>,
    target_reticle: Handle<StandardMaterial>,
}

/// How many times the terrain's logical run projection has been rebuilt.
///
/// `apply_terrain_edits` atomically replaces each affected chunk. Every tile entity
/// inside those chunks receives a new id, so the ground a route crossed may no longer
/// exist even though the rest of the grid remains stable. Nothing about the *unit*
/// changes when that happens, which is exactly why a preview keyed only on the unit
/// outlives the terrain it describes.
///
/// A counter rather than a comparison of the tiles themselves: the map publishes a few
/// thousand tile entities, and the question is only ever "is this the same terrain I
/// last looked at".
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerrainRevision(pub u64);

/// What the disclosure-safe [`Reach`] was computed for.
///
/// `terrain` is here because the other fields can all be unchanged while the ground
/// underneath them is replaced.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct PreviewKey {
    unit: Entity,
    from: TilePos,
    terrain: u64,
    disclosure: u64,
    authored_objects: u64,
}

/// The current search, and what is drawn from it.
#[derive(Resource, Default)]
struct MovementPreview {
    /// What the cached search was computed for, and whether it is still valid.
    of: Option<PreviewKey>,
    /// One unbounded reach over facts presentable to the selected faction.
    reach: Option<Reach>,
    /// Affordable surfaces sliced from [`Self::reach`] for the current combat budget.
    affordable: Vec<Standing>,
    /// The budget [`Self::affordable`] reflects. `None` means exploring or no preview.
    range_for: Option<u32>,
    /// The hovered surface the drawn overlays reflect.
    shown: Option<TilePos>,
}

/// Registers selection, the ring, and the movement overlays.
pub fn plugin(app: &mut App) {
    app.register_type::<Selected>()
        .register_type::<CameraFocusTarget>()
        .register_type::<TargetReticleRequest>()
        .register_type::<WorldMarkerSuppression>()
        .register_type::<RangeOverlay>()
        .register_type::<PathOverlay>()
        .register_type::<OutOfRangeOverlay>()
        .init_resource::<HoveredSurface>()
        .init_resource::<MovementPreview>()
        .init_resource::<TerrainRevision>()
        .init_resource::<WorldMarkerSuppression>()
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
                reconcile_selection,
                reconcile_camera_focus_target,
                redraw_overlays,
            )
                .chain()
                .in_set(PausableSystems)
                .in_set(GameplaySystems::Selection)
                .after(track_terrain_changes)
                .after(PerceptionSystems::ApplyPresentation)
                .after(crate::movement::MovementSystems::Reconcile),
        )
        .add_systems(
            Update,
            (reconcile_rings, reconcile_target_reticles)
                .chain()
                .in_set(PausableSystems)
                .in_set(GameplaySystems::WorldFeedback),
        )
        // Observers are global and fire in every state, including the title screen.
        // These three touch only `HoveredSurface`, which is initialised at startup and
        // therefore always present — Bevy validates system parameters *before* the
        // body runs, so an `Option` would be required for anything gameplay-scoped.
        .add_observer(on_tile_hovered)
        .add_observer(on_tile_moved)
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
    // A rose-magenta refusal cue, rather than either faction's red/blue primary.
    // The crossed-stroke geometry carries the same meaning without relying on hue.
    let out_of_range = materials.add(cap_material(Color::srgba(0.95, 0.30, 0.62, 0.9), 9.0));

    // Sized inside a hex — the grid's circumradius is 1.0, so its inradius is about
    // 0.87 and a ring at 0.78 clears the edge on every side.
    let ring = meshes.add(Mesh::from(Torus::new(0.68, 0.78)));
    // Four copies form the target reticle, and two crossed copies form the
    // out-of-range ×. Neither can be confused with the continuous acting ring.
    let target_tick = meshes.add(Mesh::from(Cuboid::new(0.34, 0.025, 0.12)));

    // Rings borrow their unit's colour, because a ring *is* about ownership — it says
    // whose turn it is.
    let player_ring = materials.add(cap_material(Color::srgba(1.0, 0.45, 0.4, 0.85), 1.0));
    let enemy_ring = materials.add(cap_material(Color::srgba(0.45, 0.65, 1.0, 0.85), 1.0));
    let target_reticle = materials.add(cap_material(Color::srgba(1.0, 0.82, 0.34, 0.92), 3.0));

    commands.insert_resource(OverlayAssets {
        range,
        path,
        out_of_range,
        ring,
        target_tick,
        player_ring,
        enemy_ring,
        target_reticle,
    });
}

/// Keeps exactly one player selected and forces the acting player during combat.
fn reconcile_selection(
    mut commands: Commands,
    mode: Option<Res<State<Mode>>>,
    party: Res<Party>,
    registry: Res<UnitRegistry>,
    players: Query<(Entity, &UnitId, Has<Selected>, Has<Turn>), With<Player>>,
    selected_non_players: Query<Entity, (With<Selected>, Without<Player>)>,
) {
    let forced = mode
        .as_deref()
        .filter(|mode| *mode.get() == Mode::Combat)
        .and_then(|_| {
            players
                .iter()
                .find_map(|(entity, _, _, acting)| acting.then_some(entity))
        });
    let existing = players
        .iter()
        .filter_map(|(entity, _, selected, _)| selected.then_some(entity))
        .next();
    let wanted = forced.or(existing).or_else(|| {
        party
            .members
            .iter()
            .find_map(|member| registry.entity_of(*member))
    });
    for entity in &selected_non_players {
        commands.entity(entity).remove::<Selected>();
    }
    for (entity, _, selected, _) in &players {
        if Some(entity) == wanted {
            if !selected {
                commands.entity(entity).insert(Selected);
            }
        } else if selected {
            commands.entity(entity).remove::<Selected>();
        }
    }
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
    suppression: Res<WorldMarkerSuppression>,
    acting: Query<(Entity, &Faction), With<Turn>>,
    selected: Query<(Entity, &Faction), With<Selected>>,
    rings: Query<(Entity, &UnitRing)>,
) {
    let Some(overlays) = overlays else {
        return;
    };

    // Acting first: during combat the selection is still sitting on the player, and
    // ringing it as well would say two units are up at once. Phase suppression
    // deliberately removes both options without mutating either gameplay marker.
    let wanted = if suppression.is_suppressed() {
        None
    } else {
        acting.iter().next().or_else(|| selected.iter().next())
    };

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
            Visibility::Inherited,
            // Without this the ring swallows clicks on the tile its unit stands on,
            // which is the bug `Pickable::IGNORE` on the piece already exists to
            // avoid.
            Pickable::IGNORE,
            UnitRing(unit),
            Name::new("UnitRing"),
        ));
    });
}

/// Reconciles one four-tick, world-space reticle from a disclosure-authorized request.
///
/// Exactly one identity-consistent request is required. Multiple requests fail closed
/// instead of choosing by query iteration order. Every rendered tick is a child of
/// the unit, uses ordinary depth testing, casts no shadow, and ignores picking.
fn reconcile_target_reticles(
    mut commands: Commands,
    overlays: Option<Res<OverlayAssets>>,
    suppression: Res<WorldMarkerSuppression>,
    requests: Query<(Entity, &UnitId, &TargetReticleRequest)>,
    reticles: Query<(Entity, &TargetReticle)>,
) {
    let Some(overlays) = overlays else {
        return;
    };

    let wanted = if suppression.is_suppressed() {
        None
    } else {
        let mut valid = requests
            .iter()
            .filter_map(|(entity, unit, request)| (*unit == request.unit).then_some(entity));
        let first = valid.next();
        if first.is_some() && valid.next().is_none() {
            first
        } else {
            None
        }
    };

    let mut wanted_parts = 0usize;
    for (reticle, owner) in &reticles {
        if Some(owner.0) == wanted {
            wanted_parts += 1;
        } else {
            // The owning unit may already have been removed earlier in the frame.
            commands.entity(reticle).try_despawn();
        }
    }

    let Some(unit) = wanted else {
        return;
    };
    if wanted_parts == TARGET_RETICLE_PARTS {
        return;
    }
    if wanted_parts > 0 {
        for (reticle, owner) in &reticles {
            if owner.0 == unit {
                commands.entity(reticle).try_despawn();
            }
        }
    }

    commands.entity(unit).with_children(|parent| {
        for yaw in [
            0.0,
            std::f32::consts::FRAC_PI_2,
            std::f32::consts::PI,
            -std::f32::consts::FRAC_PI_2,
        ] {
            let direction = Quat::from_rotation_y(yaw) * Vec3::Z;
            parent.spawn((
                Mesh3d(overlays.target_tick.clone()),
                MeshMaterial3d(overlays.target_reticle.clone()),
                Transform::from_translation(
                    direction * TARGET_RETICLE_RADIUS + Vec3::Y * TARGET_RETICLE_LIFT,
                )
                .with_rotation(Quat::from_rotation_y(yaw)),
                Visibility::Inherited,
                Pickable::IGNORE,
                NotShadowCaster,
                TargetReticle(unit),
                Name::new("TargetReticle"),
            ));
        }
    });
}

/// Records the surface under the cursor.
fn on_tile_hovered(
    event: On<Pointer<Over>>,
    tiles: TileQuery,
    terrain_batches: Query<&TerrainRenderBatch>,
    mut hovered: ResMut<HoveredSurface>,
) {
    let Some(target) = resolve_tile_pointer_target(
        event.event_target(),
        event.event.hit.position,
        event.event.hit.normal,
        &tiles,
        &terrain_batches,
    ) else {
        return;
    };
    let Ok((pos, _, _, _)) = tiles.get(target) else {
        return;
    };
    hovered.0 = Some(*pos);
}

/// Updates exact hover while the pointer crosses logical hexes inside one combined
/// mesh. Such crossings do not produce `Out`/`Over` because the picked batch entity
/// itself did not change.
fn on_tile_moved(
    event: On<Pointer<Move>>,
    tiles: TileQuery,
    terrain_batches: Query<&TerrainRenderBatch>,
    mut hovered: ResMut<HoveredSurface>,
) {
    let Some(target) = resolve_tile_pointer_target(
        event.event_target(),
        event.event.hit.position,
        event.event.hit.normal,
        &tiles,
        &terrain_batches,
    ) else {
        return;
    };
    let Ok((pos, _, _, _)) = tiles.get(target) else {
        return;
    };
    hovered.0 = Some(*pos);
}

/// Forgets it again, unless the cursor has already moved on to another tile.
fn on_tile_unhovered(
    event: On<Pointer<Out>>,
    tiles: TileQuery,
    terrain_batches: Query<&TerrainRenderBatch>,
    mut hovered: ResMut<HoveredSurface>,
) {
    let current = hovered.0;
    let resolved_position = resolve_tile_pointer_target(
        event.event_target(),
        event.event.hit.position,
        event.event.hit.normal,
        &tiles,
        &terrain_batches,
    )
    .and_then(|target| tiles.get(target).ok())
    .map(|(position, _, _, _)| *position);
    let current_belongs_to_departing_batch = current.is_some_and(|position| {
        terrain_batches
            .get(event.event_target())
            .is_ok_and(|batch| batch.contains_position(position))
    });
    // `Over` for the tile being entered can arrive before `Out` for the one being
    // left. Clearing unconditionally would then erase a hover that had already moved
    // on, and the path would flicker out every time the cursor crossed a batch edge.
    // `Out` may carry no hit coordinates, so batch membership is the exact fallback
    // when the current hover still belongs to the departing render batch.
    if current == resolved_position || current_belongs_to_departing_batch {
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
    mut footing_cache: ResMut<FootingCache>,
    hovered: Res<HoveredSurface>,
    assets: Option<Res<GameAssets>>,
    overlays: Option<Res<OverlayAssets>>,
    table: Option<Res<SubstanceTable>>,
    mode: Option<Res<State<Mode>>>,
    revision: Res<TerrainRevision>,
    blockers: Option<Res<TraversalBlockers>>,
    authored_objects: Option<Res<AuthoredObjectOccupancy>>,
    tiles: TileQuery,
    selected: Query<
        (Entity, &UnitId, &Faction, &StandsOn, &Body, Option<&Turn>),
        (With<Selected>, Without<Busy>, Without<MovingTo>),
    >,
    selected_body_changes: Query<(), (With<Selected>, Changed<Body>)>,
    positions: Query<(
        &UnitId,
        &Faction,
        &StandsOn,
        Option<&MovingTo>,
        Option<&PresentationOcclusion>,
    )>,
    drawn: Query<Entity, DrawnOverlays>,
) {
    let (Some(assets), Some(overlays), Some(table), Some(mode), Some(authored_objects)) =
        (assets, overlays, table, mode, authored_objects)
    else {
        for overlay in &drawn {
            commands.entity(overlay).despawn();
        }
        *preview = MovementPreview::default();
        return;
    };

    let selection = selected.iter().next();
    let viewer_faction = selection.map(|(_, _, faction, _, _, _)| *faction);
    let footing_changed = table.is_changed()
        || blockers
            .as_ref()
            .is_some_and(|blockers| blockers.is_changed())
        || !selected_body_changes.is_empty();
    let disclosed_occupancy = UnitOccupancy::from_positions(
        positions
            .iter()
            .filter(|(_, faction, _, _, occlusion)| {
                viewer_faction.is_some_and(|viewer| {
                    **faction == viewer
                        || !occlusion.is_some_and(|occlusion| {
                            occlusion.contains(PresentationOcclusionReason::Fog)
                        })
                })
            })
            .flat_map(|(unit, _, on, moving, _)| {
                std::iter::once((*unit, on.0.pos)).chain(
                    moving
                        .into_iter()
                        .flat_map(|moving| moving.path.iter())
                        .map(|step| (*unit, step.pos)),
                )
            }),
    );
    let request = selection.and_then(|(entity, _, _, standing, _, turn)| {
        let budget = match (mode.get(), turn) {
            // Somebody else's turn. Tinting a range the piece cannot use this turn
            // would promise a move that would then be refused.
            (Mode::Combat, None) => return None,
            (Mode::Combat, Some(turn)) => Some(turn.movement_left),
            // Exploring has no budget at all, which is why there is no range tint.
            (Mode::Exploring, _) => None,
        };
        Some((
            PreviewKey {
                unit: entity,
                from: standing.0.pos,
                terrain: revision.0,
                disclosure: disclosed_occupancy.fingerprint(),
                authored_objects: authored_objects.fingerprint(),
            },
            budget,
        ))
    });
    let key = request.map(|(key, _)| key);
    let budget = request.and_then(|(_, budget)| budget);

    if key == preview.of
        && budget == preview.range_for
        && hovered.0 == preview.shown
        && !footing_changed
    {
        return;
    }

    // The search depends on where the piece stands, not where the cursor is.
    // `Footing::from_tiles` reads every tile entity on the map — tens of thousands on
    // the largest current worlds — so rebuild it only when the graph key is stale.
    // Budget changes merely re-slice cached costs, and hidden occupancy is absent from
    // the key entirely.
    let reach_dirty = key != preview.of || footing_changed;
    if reach_dirty {
        preview.reach = if let (Some(_), Some((_, unit, _, standing, body, _))) = (key, selection) {
            let footing = footing_cache.get_or_build(
                revision.0,
                tiles.iter(),
                &table,
                *body,
                blockers.as_deref(),
                &authored_objects,
            );
            Some(Reach::with_occupancy(
                standing.0,
                footing.as_ref(),
                None,
                &disclosed_occupancy,
                *unit,
            ))
        } else {
            None
        };
        preview.of = key;
    }

    if reach_dirty || budget != preview.range_for {
        let mut affordable = match (preview.reach.as_ref(), key, budget) {
            (Some(reach), Some(key), Some(budget)) => reach
                .surfaces()
                .filter(|standing| {
                    standing.pos != key.from
                        && reach.cost(standing.pos).is_some_and(|cost| cost <= budget)
                })
                .collect(),
            _ => Vec::new(),
        };
        affordable.sort_by_key(|standing: &Standing| standing.pos);
        preview.affordable = affordable;
        preview.range_for = budget;
    }
    preview.shown = hovered.0;

    for overlay in &drawn {
        commands.entity(overlay).despawn();
    }

    let Some(reach) = preview.reach.as_ref() else {
        return;
    };
    if key.is_none() {
        return;
    }

    if budget.is_some() {
        for standing in preview.affordable.iter().copied() {
            let tint = cap(&assets, &overlays.range, standing, RANGE_INSET, RANGE_LIFT);
            commands.spawn((tint, RangeOverlay, Name::new("RangeOverlay")));
        }
    }

    let Some(target) = hovered.0 else {
        return;
    };
    let Some(cost) = reach.cost(target) else {
        return;
    };
    if budget.is_some_and(|budget| cost > budget) {
        let Some(destination) = reach.standing(target) else {
            return;
        };
        spawn_out_of_range_glyph(&mut commands, &overlays, destination);
        return;
    }
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
type DrawnOverlays = Or<(
    With<RangeOverlay>,
    With<PathOverlay>,
    With<OutOfRangeOverlay>,
)>;

/// Draws one colour-independent × over a connected destination beyond the budget.
fn spawn_out_of_range_glyph(commands: &mut Commands, overlays: &OverlayAssets, standing: Standing) {
    let translation = standing
        .pos
        .coord
        .to_world(standing.span.top + OUT_OF_RANGE_LIFT + TARGET_TICK_HALF_THICKNESS);
    for yaw in [std::f32::consts::FRAC_PI_4, -std::f32::consts::FRAC_PI_4] {
        commands.spawn((
            Mesh3d(overlays.target_tick.clone()),
            MeshMaterial3d(overlays.out_of_range.clone()),
            Transform {
                translation,
                rotation: Quat::from_rotation_y(yaw),
                scale: OUT_OF_RANGE_STROKE_SCALE,
            },
            Visibility::Inherited,
            Pickable::IGNORE,
            NotShadowCaster,
            standing.pos,
            OutOfRangeOverlay,
            Name::new("OutOfRangeOverlay"),
        ));
    }
}

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
        // entity: `apply_terrain_edits` replaces the affected chunk on an accepted
        // edit, invalidating every tile `Entity` id in that chunk.
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
    markers: Query<Entity, Or<(With<UnitRing>, With<TargetReticle>)>>,
    mut preview: ResMut<MovementPreview>,
    mut hovered: ResMut<HoveredSurface>,
    mut suppression: ResMut<WorldMarkerSuppression>,
) {
    for overlay in &drawn {
        commands.entity(overlay).despawn();
    }
    for marker in &markers {
        commands.entity(marker).try_despawn();
    }
    *preview = MovementPreview::default();
    hovered.0 = None;
    *suppression = WorldMarkerSuppression::default();
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::{HexCoord, HexSpan};
    use hex_test_app::HeadlessAppBuilder;

    #[test]
    #[expect(
        clippy::expect_used,
        reason = "an explicit fixture window always normalizes for pointer events"
    )]
    fn batch_out_without_hit_coordinates_clears_only_its_exact_hover() {
        let mut app = App::new();
        app.init_resource::<HoveredSurface>()
            .add_observer(on_tile_unhovered);
        let position = TilePos::new(HexCoord::ORIGIN, 3);
        let logical = app.world_mut().spawn_empty().id();
        let batch = app
            .world_mut()
            .spawn(TerrainRenderBatch::new(
                hex_core::TerrainChunkRoot { q: 0, r: 0 },
                hex_core::SubstanceId(1),
                vec![hex_core::TerrainPickRun::new(
                    logical,
                    position,
                    HexSpan::new(0.0, 1.6),
                )],
            ))
            .id();
        let window = app.world_mut().spawn(Window::default()).id();
        let target = bevy::window::WindowRef::Entity(window)
            .normalize(Some(window))
            .expect("the fixture window should normalize");
        let location = bevy::picking::pointer::Location {
            target: bevy::camera::NormalizedRenderTarget::Window(target),
            position: Vec2::ZERO,
        };
        let out = || Out {
            hit: bevy::picking::backend::HitData::new(batch, 0.0, None, None),
        };

        app.world_mut().resource_mut::<HoveredSurface>().0 = Some(position);
        app.world_mut().trigger(Pointer::new(
            bevy::picking::pointer::PointerId::Mouse,
            location.clone(),
            out(),
            batch,
        ));
        assert_eq!(app.world().resource::<HoveredSurface>().0, None);

        let newer_hover = TilePos::new(HexCoord::from_axial(1, 0), 3);
        app.world_mut().resource_mut::<HoveredSurface>().0 = Some(newer_hover);
        app.world_mut().trigger(Pointer::new(
            bevy::picking::pointer::PointerId::Mouse,
            location,
            out(),
            batch,
        ));
        assert_eq!(
            app.world().resource::<HoveredSurface>().0,
            Some(newer_hover),
            "a late Out from the previous batch erased a newer exact hover"
        );
    }

    fn standing_at(position: TilePos) -> StandsOn {
        StandsOn(Standing {
            pos: position,
            span: HexSpan::new(0.0, 1.0),
        })
    }

    fn marker_overlay_assets() -> (OverlayAssets, Handle<Mesh>, Handle<Mesh>) {
        let mut meshes = Assets::<Mesh>::default();
        let ring = meshes.add(Mesh::from(Torus::new(0.68, 0.78)));
        let target_tick = meshes.add(Mesh::from(Cuboid::new(0.34, 0.025, 0.12)));
        let mut materials = Assets::<StandardMaterial>::default();
        let range = materials.add(StandardMaterial::default());
        let path = materials.add(StandardMaterial::default());
        let out_of_range = materials.add(StandardMaterial::default());
        let player_ring = materials.add(StandardMaterial::default());
        let enemy_ring = materials.add(StandardMaterial::default());
        let target_reticle = materials.add(StandardMaterial::default());

        (
            OverlayAssets {
                range,
                path,
                out_of_range,
                ring: ring.clone(),
                target_tick: target_tick.clone(),
                player_ring,
                enemy_ring,
                target_reticle,
            },
            ring,
            target_tick,
        )
    }

    fn marker_app() -> (App, Entity, Handle<Mesh>, Handle<Mesh>) {
        let mut builder = HeadlessAppBuilder::new().with_minimal_plugins();
        let (overlays, ring_mesh, reticle_mesh) = marker_overlay_assets();
        builder
            .app_mut()
            .insert_resource(overlays)
            .init_resource::<WorldMarkerSuppression>()
            .configure_sets(
                Update,
                (
                    GameplaySystems::WorldFeedbackRequests,
                    GameplaySystems::WorldFeedback,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (reconcile_rings, reconcile_target_reticles)
                    .chain()
                    .in_set(GameplaySystems::WorldFeedback),
            );
        let unit = UnitId(12);
        let entity = builder
            .app_mut()
            .world_mut()
            .spawn((
                unit,
                Faction::Player,
                Selected,
                TargetReticleRequest::new(unit),
                Transform::default(),
                Visibility::Hidden,
            ))
            .id();
        (builder.build(), entity, ring_mesh, reticle_mesh)
    }

    #[test]
    fn camera_focus_target_tracks_selection_and_exact_surface_changes() {
        let mut builder = HeadlessAppBuilder::new();
        builder
            .app_mut()
            .add_systems(Update, reconcile_camera_focus_target);
        let mut app = builder.build();

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
        let mut builder = HeadlessAppBuilder::new();
        builder.app_mut().add_systems(
            Update,
            (reconcile_selection, reconcile_camera_focus_target).chain(),
        );
        let mut app = builder.build();
        let surface = TilePos::new(HexCoord::from_axial(1, -1), 3);
        let id = UnitId(3);
        let player = app
            .world_mut()
            .spawn((Player, id, standing_at(surface)))
            .id();
        let mut registry = UnitRegistry::default();
        registry.register(id, player);
        app.insert_resource(registry);
        app.insert_resource(Party { members: vec![id] });

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

    #[test]
    fn combat_forces_selection_to_the_acting_player() {
        let mut builder = HeadlessAppBuilder::new()
            .with_minimal_plugins()
            .with_state_plugin();
        builder
            .app_mut()
            .init_state::<Mode>()
            .add_systems(Update, reconcile_selection);
        let mut app = builder.build();
        let first_id = UnitId(4);
        let second_id = UnitId(8);
        let first = app.world_mut().spawn((Player, first_id, Selected)).id();
        let second = app
            .world_mut()
            .spawn((
                Player,
                second_id,
                Turn {
                    movement_left: 3,
                    acted: false,
                },
            ))
            .id();
        let mut registry = UnitRegistry::default();
        registry.register(first_id, first);
        registry.register(second_id, second);
        app.insert_resource(registry);
        app.insert_resource(Party {
            members: vec![first_id, second_id],
        });
        app.world_mut()
            .resource_mut::<NextState<Mode>>()
            .set(Mode::Combat);

        app.update();
        app.update();

        assert!(!app.world().entity(first).contains::<Selected>());
        assert!(app.world().entity(second).contains::<Selected>());
    }

    #[test]
    fn target_reticle_is_shape_distinct_depth_tested_and_non_pickable() {
        let (mut app, unit, ring_mesh, reticle_mesh) = marker_app();

        app.update();

        let mut reticles =
            app.world_mut()
                .query::<(&TargetReticle, &Mesh3d, &Pickable, &ChildOf, &Visibility)>();
        let parts = reticles
            .iter(app.world())
            .map(|(reticle, mesh, pickable, parent, visibility)| {
                assert_eq!(reticle.0, unit);
                assert_eq!(mesh.0, reticle_mesh);
                assert_eq!(*pickable, Pickable::IGNORE);
                assert_eq!(parent.parent(), unit);
                assert_eq!(*visibility, Visibility::Inherited);
            })
            .count();
        assert_eq!(parts, TARGET_RETICLE_PARTS);

        let mut rings = app
            .world_mut()
            .query::<(&UnitRing, &Mesh3d, &Pickable, &ChildOf)>();
        let (ring, mesh, pickable, parent) = rings
            .single(app.world())
            .expect("the selected unit should keep one continuous acting ring");
        assert_eq!(ring.0, unit);
        assert_eq!(mesh.0, ring_mesh);
        assert_ne!(mesh.0, reticle_mesh);
        assert_eq!(*pickable, Pickable::IGNORE);
        assert_eq!(parent.parent(), unit);
    }

    #[test]
    fn target_reticle_and_ring_clear_under_phase_suppression_without_mutating_requests() {
        let (mut app, unit, _, _) = marker_app();
        app.update();

        app.world_mut()
            .resource_mut::<WorldMarkerSuppression>()
            .set(true);
        app.update();

        let mut rings = app.world_mut().query_filtered::<Entity, With<UnitRing>>();
        assert_eq!(rings.iter(app.world()).count(), 0);
        let mut reticles = app
            .world_mut()
            .query_filtered::<Entity, With<TargetReticle>>();
        assert_eq!(reticles.iter(app.world()).count(), 0);
        let unit_entity = app.world().entity(unit);
        assert!(unit_entity.contains::<Selected>());
        assert!(unit_entity.contains::<TargetReticleRequest>());

        app.world_mut()
            .resource_mut::<WorldMarkerSuppression>()
            .set(false);
        app.update();
        assert_eq!(rings.iter(app.world()).count(), 1);
        assert_eq!(reticles.iter(app.world()).count(), TARGET_RETICLE_PARTS);

        app.world_mut()
            .entity_mut(unit)
            .remove::<TargetReticleRequest>();
        app.update();
        assert_eq!(reticles.iter(app.world()).count(), 0);
        assert_eq!(rings.iter(app.world()).count(), 1);
    }

    #[test]
    fn published_suppression_clears_world_markers_in_the_same_frame() {
        fn publish_suppression(mut suppression: ResMut<WorldMarkerSuppression>) {
            suppression.set(true);
        }

        let (mut app, _, _, _) = marker_app();
        app.add_systems(
            Update,
            publish_suppression.in_set(GameplaySystems::WorldFeedbackRequests),
        );
        app.update();

        let mut rings = app.world_mut().query_filtered::<Entity, With<UnitRing>>();
        let mut reticles = app
            .world_mut()
            .query_filtered::<Entity, With<TargetReticle>>();
        assert_eq!(rings.iter(app.world()).count(), 0);
        assert_eq!(reticles.iter(app.world()).count(), 0);
    }
}
