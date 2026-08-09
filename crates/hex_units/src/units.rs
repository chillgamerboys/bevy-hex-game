//! The units on the map: what they are, and how they get placed.
//!
//! A unit is a [`Body`] standing on a surface, wearing a [`Faction`] so anything else
//! can tell friend from foe without naming concrete types. `Player` and `Enemy` are
//! markers on top of that, for the two things that currently exist.
//!
//! Spawning reads the active `Encounter` — the roster the chosen scenario named —
//! and places every entry in it on a surface. One entry is one unit, so the loop is
//! roster-shaped rather than "the player and the enemy": the day an archetype carries a
//! lattice, that lookup goes in one place here instead of into per-unit spawn code.
//!
//! **Every rostered unit is placed, or setup fails with a reason.** A roster entry with
//! nowhere to stand is not a unit that quietly does not appear — that is the failure
//! mode this codebase is worst at seeing.

use bevy::ecs::system::SystemParam;
use bevy::picking::events::{Click, Pointer};
use bevy::picking::Pickable;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use hex_ai::{AiController, AiGroupId, AiProfileId};
use hex_anim::Transformation;
use hex_assets::{
    AiProfileCatalog, ArtPalette, CubeCoord, Encounter, EncounterPlacement, FormationCatalog,
    FormationCenter, GameAssets, LatticeLibrary, PlayerSettings, RosteredUnit, SubstanceTable,
};
use hex_lattice::LatticeState;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub use hex_core::Faction;
use hex_core::{
    CommandQueue, ControlOwner, GameCommand, GameplayPhase, GameplaySetup, GameplaySetupFailure,
    Headroom, HexCoord, HexSpan, HexTile, IssuedCommand, MapAnchorId, MapAnchors, Mode,
    PartyFormation, PartyMovementMode, Pause, PendingDecision, PresentationOcclusion, Screen,
    SubstanceId, TerrainReady, TilePos, TraversalBlockers, TraversalProfile, Turn, UnitId,
};

use crate::movement::{route_with_occupancy, Body, Footing, MovementCrossings, Reach, Standing};
use crate::pathing::{leg_duration, reached_step_index};
use crate::selection::Selected;
use crate::{
    formation_subset_anchor, plan_formation_subset_move_with_occupancy, FormationMember,
    FormationPlanError, UnitOccupancy,
};

const PLAYER_SWATCH_ID: &str = "unit/player";
const HOSTILE_SWATCH_ID: &str = "unit/hostile";

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

/// A domain walk in progress, and every surface it passes over.
///
/// A [`Transformation`] may mirror this route for presentation, but it does not
/// advance or complete the walk. The whole path rather than just the endpoint, because a fight
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
    /// Settings can hot-reload while a piece is walking. Reconciliation keeps the
    /// schedule committed with the route rather than silently adopting a new one.
    speed: f32,
    /// Elapsed domain time, independent of presentation-component lifetime.
    elapsed: f64,
    /// The first scheduled tick establishes the route epoch at zero, matching the
    /// generic animation driver without reading any presentation component.
    started: bool,
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
            elapsed: 0.0,
            started: false,
            reconciled_step: 0,
        }
    }

    /// Reconstructs an exact authoritative route clock on a replica.
    ///
    /// Returns `None` for an empty route, a non-finite clock/speed, negative elapsed
    /// time, or a reconciled index outside the route.
    #[must_use]
    pub fn from_authoritative_clock(
        path: Vec<Standing>,
        speed: f32,
        elapsed: f64,
        started: bool,
        reconciled_step: usize,
    ) -> Option<Self> {
        if path.is_empty()
            || !speed.is_finite()
            || !elapsed.is_finite()
            || elapsed < 0.0
            || reconciled_step >= path.len()
        {
            return None;
        }
        Some(Self {
            path,
            speed,
            elapsed,
            started,
            reconciled_step,
        })
    }

    /// Exact committed domain speed.
    #[must_use]
    pub const fn speed(&self) -> f32 {
        self.speed
    }

    /// Exact elapsed authoritative route time.
    #[must_use]
    pub const fn elapsed(&self) -> f64 {
        self.elapsed
    }

    /// Whether the route epoch has been established.
    #[must_use]
    pub const fn started(&self) -> bool {
        self.started
    }

    /// Last route step committed as the exact discrete position.
    #[must_use]
    pub const fn reconciled_step(&self) -> usize {
        self.reconciled_step
    }

    fn advance(&mut self, delta: f64) -> Option<usize> {
        if self.speed <= 0.0 {
            return self.path.len().checked_sub(1);
        }
        if !self.started {
            self.started = true;
            return reached_step_index(&self.path, self.speed, 0.0);
        }
        self.elapsed += delta.max(0.0);
        reached_step_index(&self.path, self.speed, self.elapsed)
    }

    fn complete(&self) -> bool {
        self.reconciled_step.saturating_add(1) >= self.path.len()
    }

    /// Returns exact route surfaces ordered by proximity to the domain clock's
    /// current instant.
    ///
    /// `StandsOn` deliberately trails interpolation until a whole leg completes. That
    /// is the truthful occupancy fact while walking, but it is not always the safest
    /// place to freeze a converging party: a follower may just have reached the
    /// leader's last published surface while the leader is already more than halfway
    /// to the next one. Combat entry needs the same nearest-whole-step decision the
    /// presentation used to provide, derived here from the authoritative route clock.
    fn stopping_candidates(&self) -> Vec<Standing> {
        if self.speed <= 0.0 {
            return self.path.iter().rev().copied().collect();
        }
        let mut endpoint_time = 0.0;
        let mut previous = None;
        let mut candidates = self
            .path
            .iter()
            .copied()
            .enumerate()
            .map(|(index, standing)| {
                if let Some(previous) = previous {
                    endpoint_time += leg_duration(previous, standing, self.speed);
                }
                previous = Some(standing);
                (standing, (endpoint_time - self.elapsed).abs(), index)
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|(_, distance_a, index_a), (_, distance_b, index_b)| {
            distance_a
                .total_cmp(distance_b)
                .then_with(|| index_a.cmp(index_b))
        });
        candidates
            .into_iter()
            .map(|(standing, _, _)| standing)
            .collect()
    }
}

#[derive(Debug)]
struct CombatStop {
    entity: Entity,
    unit: Option<UnitId>,
    standing: Standing,
    moving: Option<MovingTo>,
    requested: Option<Standing>,
}

impl CombatStop {
    fn candidates(&self) -> Vec<Standing> {
        let mut candidates = Vec::new();
        if let Some(requested) = self.requested {
            candidates.push(requested);
        }
        if let Some(moving) = &self.moving {
            for candidate in moving.stopping_candidates() {
                if !candidates.contains(&candidate) {
                    candidates.push(candidate);
                }
            }
        }
        if !candidates.contains(&self.standing) {
            candidates.push(self.standing);
        }
        candidates
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

/// Marks the piece the player controls.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct Player;

/// Marks a unit that fights the player.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct Enemy;

/// Allocates stable unit identities in scenario spawn order.
///
/// Never reuses an id within a session; reset when gameplay exits so the same
/// scenario launch always deals the same ids — which is what lets a replay or
/// a save name units without caring which `Entity` they landed on this run.
///
/// A future load path must restore this counter alongside the restored ids,
/// or fresh deals can collide with loaded ones.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct UnitAllocator {
    next: u64,
}

impl UnitAllocator {
    /// Deals the next id. Ids are dense from zero in spawn order.
    pub fn allocate(&mut self) -> UnitId {
        let id = UnitId(self.next);
        self.next += 1;
        id
    }
}

/// Resolves between stable [`UnitId`]s and the entities carrying them.
///
/// The sim's resources and decisions key on [`UnitId`]; systems that need to
/// touch the actual ECS entity (insert a `Turn`, play an animation) resolve it
/// here. Torn down with the units so a stale entry cannot outlive its entity.
#[derive(Resource, Debug, Default)]
pub struct UnitRegistry {
    by_id: BTreeMap<UnitId, Entity>,
    ids: BTreeMap<Entity, UnitId>,
}

impl UnitRegistry {
    /// Records a unit's identity. Test spawners call this directly.
    pub fn register(&mut self, id: UnitId, entity: Entity) {
        self.by_id.insert(id, entity);
        self.ids.insert(entity, id);
    }

    /// Every registered unit, in id order.
    ///
    /// Ordered because callers scan it to answer sim questions — "who is standing
    /// here" — and a scan whose order came from a hash would make the answer depend on
    /// insertion history rather than on the map.
    pub fn iter(&self) -> impl Iterator<Item = (UnitId, Entity)> + '_ {
        self.by_id.iter().map(|(&id, &entity)| (id, entity))
    }

    /// The entity registered for `id`, if any.
    ///
    /// The registry has no liveness knowledge, and deliberately does not need any:
    /// death is a [`Downed`] marker rather than a despawn, so an entity here is always
    /// a real one. A future path that *does* despawn units must unregister them, or
    /// this will serve a dead entity.
    #[must_use]
    pub fn entity_of(&self, id: UnitId) -> Option<Entity> {
        self.by_id.get(&id).copied()
    }

    /// The stable id of `entity`, if it is a registered unit.
    #[must_use]
    pub fn id_of(&self, entity: Entity) -> Option<UnitId> {
        self.ids.get(&entity).copied()
    }

    fn clear(&mut self) {
        self.by_id.clear();
        self.ids.clear();
    }
}

/// The player's roster, by stable id.
///
/// One field, and it is the future party system: everything that needs "the
/// player's units" as a set — co-op seat assignment, saves, the party UI —
/// reads this rather than querying markers.
#[derive(Resource, Debug, Default)]
pub struct Party {
    /// Player-controlled units in spawn order.
    pub members: Vec<UnitId>,
}

/// Registers the units, their spawning, and click-to-move.
pub fn plugin(app: &mut App) {
    app.register_type::<Player>()
        .register_type::<Enemy>()
        .register_type::<Archetype>()
        .register_type::<Downed>()
        .register_type::<Faction>()
        .register_type::<AiController>()
        // `hex_core` has no plugin, so the runtime plugin that introduces its
        // shared types registers them.
        .register_type::<UnitId>()
        .register_type::<ControlOwner>()
        .register_type::<PresentationOcclusion>()
        .register_type::<PartyFormation>()
        .register_type::<PartyMovementMode>()
        .init_resource::<UnitAllocator>()
        .init_resource::<UnitRegistry>()
        .init_resource::<Party>()
        .init_resource::<PartyFormation>()
        // The funnel's queue. Initialised here as well as by `hex_combat` so
        // the click emitter works in an app composing either crate alone.
        .init_resource::<CommandQueue>()
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

fn despawn_units(
    mut commands: Commands,
    units: Query<Entity, With<Faction>>,
    mut allocator: ResMut<UnitAllocator>,
    mut registry: ResMut<UnitRegistry>,
    mut party: ResMut<Party>,
    mut formation: ResMut<PartyFormation>,
) {
    for entity in &units {
        commands.entity(entity).despawn();
    }
    // Identity state resets with the units: the next launch deals ids from
    // zero again, so a scenario's ids are a function of the scenario alone.
    *allocator = UnitAllocator::default();
    registry.clear();
    party.members.clear();
    *formation = PartyFormation::default();
}

/// Global picking observer: when any `HexTile` is clicked, resolve the route and
/// **emit** a move command for the applier to validate and start.
///
/// An emitter, not an actor: the observer's job is turning a click into intent —
/// which piece, which surface, which route — and pushing it into the
/// [`CommandQueue`]. Spending the turn's budget and starting the walk belong to
/// the one applier in `hex_combat`, same as for the AI and any future input.
/// The checks below are click-UX, not rules: they decide whether this click
/// *means* anything, so an ordinary miss-click dies quietly here instead of as
/// a warned-about invalid command.
///
/// Only a [`Selected`] piece moves. With one player piece that piece is always the
/// selection, so this changes nothing today — but it is what makes the click
/// unambiguous once there is a party, and it ties the move to the same piece whose
/// range and path are being drawn.
fn on_tile_clicked(
    event: On<Pointer<Click>>,
    tiles: TileQuery,
    players: Query<
        (
            &UnitId,
            Option<&ControlOwner>,
            &StandsOn,
            &Body,
            Option<&Turn>,
        ),
        // `Busy` is the domain movement gate. Presentation may finish before or
        // after it without changing whether another command is legal.
        (
            With<Player>,
            With<Selected>,
            Without<hex_core::Busy>,
            Without<MovingTo>,
        ),
    >,
    party_players: Query<
        (
            &UnitId,
            Option<&ControlOwner>,
            &StandsOn,
            &Body,
            Has<hex_core::Busy>,
            Has<MovingTo>,
        ),
        With<Player>,
    >,
    positions: Query<(&UnitId, &StandsOn, Option<&MovingTo>)>,
    queue: Option<ResMut<CommandQueue>>,
    table: Option<Res<SubstanceTable>>,
    party: Option<Res<Party>>,
    formation: Option<Res<PartyFormation>>,
    formations: Option<Res<FormationCatalog>>,
    blockers: Option<Res<TraversalBlockers>>,
    mode: Option<Res<State<Mode>>>,
    pause: Option<Res<State<Pause>>>,
    phase: Option<Res<GameplayPhase>>,
    pending: Option<Res<PendingDecision>>,
) {
    if phase.is_some_and(|phase| *phase != GameplayPhase::Active) {
        return;
    }
    // Every resource here is an `Option`. Observers are global: this one fires on the
    // title screen, in menus, and before anything has loaded. Bevy validates system
    // parameters *before* the body runs, so a plain `Res<T>` panics in those states
    // no matter what the body checks — which is a crash this codebase has already
    // shipped once.
    let (Some(mut queue), Some(table)) = (queue, table) else {
        return;
    };

    // Paused means paused. `PausableSystems` gates *systems*, and this is a global
    // observer — it is not in that set and never was. The applier is paused too, so
    // an emitted command would not be *lost*, but it would sit in the queue and play
    // out the moment the game resumes — a click through the pause overlay must mean
    // nothing at all, not "something, later".
    if pause.is_some_and(|pause| pause.get().0) {
        return;
    }
    if pending.is_some_and(|decision| decision.is_open()) {
        return;
    }

    // No mode at all means we are not in gameplay, so a click cannot be a move.
    let Some(mode) = mode else {
        return;
    };
    let occupancy =
        UnitOccupancy::from_positions(positions.iter().flat_map(|(unit, on, moving)| {
            std::iter::once((*unit, on.0.pos)).chain(
                moving
                    .into_iter()
                    .flat_map(|moving| moving.path.iter())
                    .map(|step| (*unit, step.pos)),
            )
        }));

    // The click identifies a tile *entity*, which resolves to one specific surface
    // even where several share a coordinate. Picking is the right input for exactly
    // that reason: it never has to guess which surface was meant.
    let clicked = event.event_target();
    let Ok((pos, _, _, _)) = tiles.get(clicked) else {
        return;
    };

    if *mode.get() == Mode::Exploring
        && party
            .as_deref()
            .is_some_and(|party| party.members.len() > 1)
        && formation
            .as_deref()
            .is_some_and(|formation| formation.mode == PartyMovementMode::Group)
    {
        let (Some(party), Some(formation), Some(formations)) = (
            party.as_deref(),
            formation.as_deref(),
            formations.as_deref(),
        ) else {
            return;
        };
        let Some(preset) = formations.get(&formation.preset) else {
            return;
        };
        let Some((selected, owner, _, _, _turn)) = players.iter().next() else {
            return;
        };
        let issuing_seat = owner.copied().unwrap_or_default().0;
        let owned_members = party
            .members
            .iter()
            .copied()
            .filter(|member| {
                party_players
                    .iter()
                    .find(|(unit, _, _, _, _, _)| **unit == *member)
                    .is_some_and(|(_, owner, _, _, _, _)| {
                        owner.copied().unwrap_or_default().0 == issuing_seat
                    })
            })
            .collect::<Vec<_>>();
        if !owned_members.contains(selected) {
            return;
        }
        let Some(anchor) = formation_subset_anchor(preset, formation, &owned_members) else {
            return;
        };
        let Some((_, _, anchor_standing, anchor_body, _, _)) = party_players
            .iter()
            .find(|(unit, _, _, _, _, _)| **unit == anchor)
        else {
            return;
        };
        if queue.holds_command_for(anchor)
            || party_players
                .iter()
                .any(|(unit, owner, _, _, busy, moving)| {
                    owner.copied().unwrap_or_default().0 == issuing_seat
                        && owned_members.contains(unit)
                        && (busy || moving)
                })
        {
            return;
        }
        let anchor_footing = Arc::new(Footing::from_tiles(
            tiles.iter(),
            &table,
            *anchor_body,
            blockers.as_deref(),
        ));
        let Some(destination) = anchor_footing.at(*pos) else {
            return;
        };
        let external_occupancy = occupancy.without(owned_members.iter().copied());
        let Some(anchor_path) = route_with_occupancy(
            anchor_standing.0,
            destination,
            &anchor_footing,
            &external_occupancy,
            anchor,
        ) else {
            return;
        };
        if anchor_path.len() < 2 {
            return;
        }
        // Footing is body-profile specific, but members with the same body read the
        // same immutable terrain index. Today every shipped unit is a walker, so a
        // six-member move needs one index rather than six duplicate map projections;
        // retaining the profile-keyed cache keeps future heterogeneous parties valid.
        let mut footing_by_body = vec![(*anchor_body, Arc::clone(&anchor_footing))];
        let mut members = Vec::with_capacity(owned_members.len());
        for member in &owned_members {
            let Some((unit, _, standing, body, _, _)) = party_players
                .iter()
                .find(|(unit, _, _, _, _, _)| *unit == member)
            else {
                return;
            };
            let member_footing = if let Some((_, footing)) = footing_by_body
                .iter()
                .find(|(cached_body, _)| *cached_body == *body)
            {
                Arc::clone(footing)
            } else {
                let footing = Arc::new(Footing::from_tiles(
                    tiles.iter(),
                    &table,
                    *body,
                    blockers.as_deref(),
                ));
                footing_by_body.push((*body, Arc::clone(&footing)));
                footing
            };
            members.push(FormationMember {
                unit: *unit,
                standing: standing.0,
                footing: member_footing,
            });
        }
        match plan_formation_subset_move_with_occupancy(
            preset,
            formation,
            anchor,
            &anchor_path,
            members,
            &external_occupancy,
        ) {
            Ok(plan) => queue.push(IssuedCommand {
                seat: issuing_seat,
                command: GameCommand::MoveParty {
                    anchor,
                    paths: plan.paths,
                },
            }),
            Err(FormationPlanError::NoSafeSlot(member)) => {
                warn!("party move rejected: member {member:?} has no safe compressed slot");
            }
            Err(FormationPlanError::Occupied(block)) => {
                warn!("party move rejected: simultaneous routes conflict at {block:?}");
            }
            Err(FormationPlanError::InvalidFormation) => {
                warn!("party move rejected: runtime formation assignments are invalid");
            }
        }
        return;
    }

    for (unit, owner, standing, body, turn) in players.iter() {
        // In combat a click is only a move if it is this unit's turn. Out of combat
        // everything moves freely — that is the whole difference between the modes.
        if *mode.get() == Mode::Combat && turn.is_none() {
            continue;
        }

        // Two clicks in one frame are one intent. The mid-walk filters above
        // cannot catch the second one — the first command's animation lands
        // only at the applier's sync point — so fold it here rather than let
        // it reach the applier and die as a warned drop.
        if queue.holds_command_for(*unit) {
            continue;
        }

        // Footing and the destination are resolved per body, because whether a surface
        // can be stood on depends on who is asking — a crawlspace is footing for a
        // small creature and a wall for a large one. With one player this is the same
        // work as hoisting it out of the loop; with a mixed party it is the difference
        // between right and wrong.
        let footing = Footing::from_tiles(tiles.iter(), &table, *body, blockers.as_deref());
        let Some(destination) = footing.at(*pos) else {
            continue;
        };

        // No route is a legitimate answer: terrain is not guaranteed connected, and
        // a cliff, a gap, or a ceiling too low to fit under means the piece simply
        // does not move.
        let Some(steps) =
            route_with_occupancy(standing.0, destination, &footing, &occupancy, *unit)
        else {
            continue;
        };

        // A route of N surfaces costs N-1 steps: the first entry is where the piece
        // already stands. Too far for what is left of this turn means the click
        // means nothing — refusing here rather than emitting keeps a long-range
        // miss-click from being logged as an invalid command.
        let cost = u32::try_from(steps.len().saturating_sub(1)).unwrap_or(u32::MAX);
        if let Some(turn) = turn {
            if cost > turn.movement_left {
                continue;
            }
        }

        queue.push(IssuedCommand {
            seat: owner.copied().unwrap_or_default().0,
            command: GameCommand::MoveAlong {
                unit: *unit,
                path: steps.iter().map(|step| step.pos).collect(),
            },
        });
    }
}

/// Advances domain movement and publishes every exact route step already reached.
///
/// Registered by [`movement::plugin`](crate::movement::plugin) rather than here,
/// because it is bookkeeping every unit needs and nothing to do with spawning a
/// scenario. `hex_combat`'s tests want the former without the latter.
///
/// Updating on each completed leg is what lets engagement observe a route that enters
/// range and later leaves it again. Updating only at the final destination makes both
/// endpoints truthful while every point between them is invisible to gameplay.
///
/// Completion comes from this route's own bounded clock. Removing or retaining a
/// [`Transformation`] cannot move a unit, clear [`hex_core::Busy`], or advance a turn.
pub(crate) fn reconcile_movement(
    mut commands: Commands,
    time: Res<Time<Virtual>>,
    mut crossings: ResMut<MovementCrossings>,
    mut moving_units: Query<(Entity, Option<&UnitId>, &mut MovingTo, &mut StandsOn)>,
) {
    crossings.clear();

    for (entity, unit, mut moving, mut standing) in &mut moving_units {
        let reached_index = moving.advance(time.delta_secs_f64());

        if let Some(reached_index) = reached_index {
            let first_new = moving.reconciled_step.saturating_add(1);
            if first_new <= reached_index {
                for index in first_new..=reached_index {
                    if let Some(reached) = moving.path.get(index).copied() {
                        crossings.push(unit.copied(), entity, reached);
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

        if moving.complete() {
            commands
                .entity(entity)
                .remove::<(MovingTo, hex_core::Busy)>();
        }
    }

    crossings.sort_deterministic();
}

/// Stops a walk where it is when a fight starts.
///
/// Committing to a long walk and then being ambushed halfway should leave the piece
/// where the ambush happened, not deliver it to a destination chosen before anyone
/// knew there was a fight.
///
/// The engagement crossing wins for the unit that triggered combat. Other walkers
/// choose the closest unoccupied surface on their committed route, in stable-id order.
/// Formation routes may converge through the same chokepoint, so independently
/// choosing each nearest step can otherwise freeze two members on one exact surface.
/// Render interpolation never chooses the result; the transform merely snaps to the
/// domain-owned answer.
pub(crate) fn halt_on_combat(world: &mut World) {
    let mut occupied = {
        let mut positions = world.query::<(Entity, &StandsOn, Has<MovingTo>, Has<StopMovingAt>)>();
        positions
            .iter(world)
            .filter_map(|(_, standing, moving, requested)| {
                (!moving && !requested).then_some(standing.0.pos)
            })
            .collect::<BTreeSet<_>>()
    };
    let mut stops = {
        let mut walking = world.query_filtered::<(
            Entity,
            Option<&UnitId>,
            &StandsOn,
            Option<&MovingTo>,
            Option<&StopMovingAt>,
        ), Or<(With<MovingTo>, With<StopMovingAt>)>>();
        walking
            .iter(world)
            .map(|(entity, unit, standing, moving, requested)| CombatStop {
                entity,
                unit: unit.copied(),
                standing: standing.0,
                moving: moving.cloned(),
                requested: requested.map(|requested| requested.0),
            })
            .collect::<Vec<_>>()
    };
    stops.sort_by_key(|stop| {
        (
            stop.requested.is_none(),
            stop.unit.is_none(),
            stop.unit,
            stop.entity.to_bits(),
        )
    });

    for stop in stops {
        let stopped = stop
            .candidates()
            .into_iter()
            .find(|candidate| !occupied.contains(&candidate.pos))
            .unwrap_or_else(|| {
                error!(
                    "combat cannot freeze {:?} on a unique exact route surface",
                    stop.unit
                );
                stop.standing
            });
        occupied.insert(stopped.pos);
        if let Some(mut transform) = world.get_mut::<Transform>(stop.entity) {
            transform.translation = stopped.world_position();
        }
        world
            .entity_mut(stop.entity)
            .insert(StandsOn(stopped))
            .remove::<MovingTo>()
            .remove::<StopMovingAt>()
            .remove::<hex_core::Busy>()
            .remove::<Transformation>();
    }
}

/// What kind of unit this is, as its encounter rostered it.
///
/// The key an archetype's lattice is looked up by in `lattices.ron`, attached at spawn.
/// It resolves to no mesh and no body size: every unit is still drawn the same and walks
/// the same.
#[derive(Component, Reflect, Debug, Clone, PartialEq, Eq)]
#[reflect(Component)]
pub struct Archetype(pub String);

/// A unit whose lattice is entirely disabled: out of the fight, not out of the world.
///
/// **The provisional first implementation of death**, and provisional is the operative
/// word — the design leaves both functional death (a threshold before zero) and
/// permadeath open, and this settles neither. A downed unit leaves the turn order and is
/// retained with its lattice. Renewal can restore one or more cells, remove `Downed`,
/// and schedule it to rejoin initiative at the next round boundary; exploration Rest
/// also recovers downed party members.
///
/// A marker rather than a despawn, for two reasons. `UnitRegistry` has no `unregister`
/// and its own doc says death must add one or it will serve a dead entity — a marker
/// avoids needing it at all. The restoration flow also needs something to target:
/// a despawned unit cannot be brought back, so despawning would preclude that design
/// option.
///
/// Everything that decides who is *in* a fight filters on this: `engagement` and
/// `begin_combat` in `hex_combat`, the AI's target search, selection, and targeting. A
/// downed unit that kept its `Faction` unfiltered would keep the fight running forever
/// against somebody who cannot act.
#[derive(Component, Reflect, Debug, Default, Clone, Copy, PartialEq, Eq)]
#[reflect(Component)]
pub struct Downed;

/// Resolves a coordinate written in an encounter file.
///
/// A triple that does not sum to zero is not a hex at all. The encounter file's own
/// validation rejects it before it can be loaded, so reaching this means an encounter
/// built in Rust — and answering with the centre of the map would place a unit
/// somewhere nobody asked for, which is the whole failure mode this ticket removes.
fn coord_from(setting: CubeCoord) -> Result<HexCoord, String> {
    HexCoord::try_new_cubic(setting.x, setting.y, setting.z).ok_or_else(|| {
        format!(
            "is placed at ({}, {}, {}), whose components do not sum to zero and so are not a hex",
            setting.x, setting.y, setting.z
        )
    })
}

#[derive(SystemParam)]
struct SpawnContent<'w> {
    lattices: Option<Res<'w, LatticeLibrary>>,
    profiles: Option<Res<'w, AiProfileCatalog>>,
    formations: Option<Res<'w, FormationCatalog>>,
    anchors: Option<Res<'w, MapAnchors>>,
    blockers: Option<Res<'w, TraversalBlockers>>,
}

/// Places every rostered unit on the terrain.
///
/// Runs in `Actors`, after the map has built and flushed its tiles. Reading them any
/// earlier finds nothing and drops the units to ground level — a bug that renders
/// perfectly and reports nothing, which is why the set boundary exists.
///
/// Placement is resolved for the **whole roster before anything spawns**, so a roster
/// that cannot be placed leaves no half-built encounter standing on the map for the
/// frame it takes the failure to reach the title screen.
fn spawn_units(
    mut commands: Commands,
    assets: Res<GameAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    tiles: TileQuery,
    table: Res<SubstanceTable>,
    palette: Res<ArtPalette>,
    settings: Res<PlayerSettings>,
    encounter: Res<Encounter>,
    phase: Option<Res<GameplayPhase>>,
    // Absent until the element and spell catalogs resolve, which the loading gate now
    // waits on — so in practice it is here. Optional rather than required because a
    // headless test harness has no content, and demanding it would make every one of
    // them build a library to spawn a unit that does not cast.
    content: SpawnContent,
    mut allocator: ResMut<UnitAllocator>,
    mut registry: ResMut<UnitRegistry>,
    mut party: ResMut<Party>,
    mut formation: ResMut<PartyFormation>,
) {
    let mut identity = UnitIdentity {
        allocator: &mut allocator,
        registry: &mut registry,
        party: &mut party,
    };
    // Every unit shares a body for now. When traversal size becomes an archetype
    // property rather than a global setting, this is where that starts.
    let body = Body::new(TraversalProfile::WALKER);
    let footing = Footing::from_tiles(tiles.iter(), &table, body, content.blockers.as_deref());

    let allow_staging_overflow = phase.is_some_and(|phase| *phase == GameplayPhase::Preparing);
    let placements = match place_roster(
        &encounter,
        &footing,
        content.anchors.as_deref(),
        allow_staging_overflow,
    ) {
        Ok(placements) => placements,
        Err(reason) => {
            error!("{reason}");
            commands.insert_resource(GameplaySetupFailure::new(reason));
            return;
        }
    };

    // Resolve the complete authored presentation contract before allocating materials
    // or actors. A missing second swatch must not leave a player-only session behind.
    let (player_color, hostile_color) = match unit_colors(&palette) {
        Ok(colors) => colors,
        Err(reason) => {
            error!("{reason}");
            commands.insert_resource(GameplaySetupFailure::new(reason));
            return;
        }
    };
    let player_material = materials.add(StandardMaterial::from(player_color));
    let enemy_material = materials.add(StandardMaterial::from(hostile_color));

    // Declaration order, which is what makes the dealt ids a function of the encounter
    // rather than of this run.
    for (unit, standing) in placements {
        let faction = unit.faction;
        let lattice = lattice_for(content.lattices.as_deref(), unit.archetype);
        let controller = if faction == Faction::Hostile {
            let profile = unit
                .ai_profile
                .or_else(|| lattice.and_then(|archetype| archetype.ai_profile.as_deref()))
                .unwrap_or("baseline");
            let profile_id = AiProfileId(profile.to_owned());
            if content
                .profiles
                .as_deref()
                .is_some_and(|catalog| catalog.get(&profile_id).is_none())
            {
                let reason = format!(
                    "Encounter {:?}: hostile {:?} references missing AI profile {:?}.",
                    encounter.name, unit.archetype, profile
                );
                error!("{reason}");
                commands.insert_resource(GameplaySetupFailure::new(reason));
                return;
            }
            Some(AiController {
                profile: profile_id,
                group: unit.ai_group.map(|group| AiGroupId(group.to_owned())),
            })
        } else {
            None
        };
        spawn_unit(
            &mut commands,
            &assets,
            UnitSpawn {
                standing,
                faction,
                material: match faction {
                    Faction::Player => player_material.clone(),
                    Faction::Hostile => enemy_material.clone(),
                },
                archetype: unit.archetype,
                lattice,
                controller,
                settings: &settings,
                body,
            },
            &mut identity,
        );
    }
    if let Some(preset) = content
        .formations
        .as_deref()
        .and_then(|catalog| catalog.get("Compact").or_else(|| catalog.presets.first()))
    {
        formation.select_preset(preset, &identity.party.members);
    }
}

/// The archetype's lattice, warning once per unit if there is none.
///
/// A unit without a lattice is playable but inert — it cannot cast and nothing can
/// damage it — so this is exactly the case that must not pass quietly. It is a warning
/// rather than a setup failure because an encounter naming an undefined archetype should
/// still let the rest of the fight be looked at, which is more useful than a black
/// screen while content is being written.
fn lattice_for<'a>(
    library: Option<&'a LatticeLibrary>,
    archetype: &str,
) -> Option<&'a hex_assets::Archetype> {
    let Some(library) = library else {
        // Not a warning: no library at all means *every* unit on the field is inert, so
        // the fight cannot be won or lost by anybody. That is a broken build rather than
        // a content gap, and it is the one case here that is never a designer mid-edit.
        error!("no lattice library at all — every unit spawns unable to cast or be damaged");
        return None;
    };
    let found = library.get(archetype);
    if found.is_none() {
        warn!(
            "lattices.ron defines no {archetype:?}: it spawns inert — it cannot cast or be damaged"
        );
    }
    found
}

fn unit_colors(palette: &ArtPalette) -> Result<(Color, Color), String> {
    let required = |id| {
        palette
            .get_str(id)
            .map(|swatch| swatch.color().to_bevy_color())
            .ok_or_else(|| format!("art/palette.ron is missing required unit swatch \"{id}\"."))
    };
    Ok((required(PLAYER_SWATCH_ID)?, required(HOSTILE_SWATCH_ID)?))
}

/// The identity bookkeeping a spawn threads through: deal an id, record it,
/// and enrol player units in the party.
struct UnitIdentity<'a> {
    allocator: &'a mut UnitAllocator,
    registry: &'a mut UnitRegistry,
    party: &'a mut Party,
}

/// Resolves one surface for every rostered unit, or says which entry could not be.
///
/// **Exact placements are resolved first, formations second.** A `Fixed` coordinate or
/// an `Anchor` names one surface deliberately; a formation only wants *a* surface near
/// its centre. Resolving in that order means a formation flows around the sentry on the
/// bridge rather than taking his hex and pushing him off the map.
///
/// The returned order is the encounter's declaration order regardless, because that is
/// what the unit ids are dealt in.
fn place_roster<'a>(
    encounter: &'a Encounter,
    footing: &Footing,
    anchors: Option<&MapAnchors>,
    allow_staging_overflow: bool,
) -> Result<Vec<(RosteredUnit<'a>, Standing)>, String> {
    let units: Vec<RosteredUnit<'a>> = encounter.entries().collect();
    let mut resolved: Vec<Option<Standing>> = vec![None; units.len()];
    // One unit per surface. Two pieces on one voxel is not a position the rest of the
    // game can express, so it is a setup failure rather than a rendering curiosity.
    let mut occupancy = UnitOccupancy::default();

    for (index, unit) in units.iter().enumerate() {
        let standing = match unit.placement {
            EncounterPlacement::Surface(pos) => footing.at(*pos).ok_or_else(|| {
                format!(
                    "Encounter {:?}: {} is placed on exact surface {:?}, but that surface is not \
                     valid footing for its body.",
                    encounter.name,
                    describe(unit),
                    pos
                )
            })?,
            EncounterPlacement::Fixed(coord) => {
                authored_standing(*coord, footing, unit, encounter)?
            }
            EncounterPlacement::Anchor(name) => {
                anchored_standing(name, footing, anchors, unit, encounter)?
            }
            // Second pass: a formation needs to know which surfaces the exact
            // placements have already claimed.
            EncounterPlacement::Formation { .. } => continue,
        };
        let unit_id = UnitId(u64::try_from(index).unwrap_or(u64::MAX));
        if occupancy.is_occupied(standing.pos, Some(unit_id)) {
            return Err(format!(
                "Encounter {:?}: {} is placed on {:?}, where another unit already stands.",
                encounter.name,
                describe(unit),
                standing.pos
            ));
        }
        occupancy.relocate(unit_id, standing.pos);
        if let Some(slot) = resolved.get_mut(index) {
            *slot = Some(standing);
        }
    }

    // Surfaces per formation centre, computed once. Two rosters may share a centre, and
    // the flood fill is the expensive part of placement.
    let mut spreads: HashMap<(TilePos, u32), Vec<Standing>> = HashMap::default();
    let mut staging_spreads: HashMap<TilePos, Vec<Standing>> = HashMap::default();
    for (index, unit) in units.iter().enumerate() {
        let EncounterPlacement::Formation { center, spread } = unit.placement else {
            continue;
        };
        let middle = match center {
            FormationCenter::Fixed(coord) => authored_standing(*coord, footing, unit, encounter)?,
            FormationCenter::Anchor(name) => {
                anchored_standing(name, footing, anchors, unit, encounter)?
            }
        };
        let candidates = spreads
            .entry((middle.pos, *spread))
            .or_insert_with(|| formation_surfaces(middle, footing, Some(*spread)));

        let standing = candidates
            .iter()
            .find(|candidate| !occupancy.is_occupied(candidate.pos, None))
            .copied();
        let standing = standing.or_else(|| {
            allow_staging_overflow.then(|| {
                staging_spreads
                    .entry(middle.pos)
                    .or_insert_with(|| formation_surfaces(middle, footing, None))
                    .iter()
                    .find(|candidate| !occupancy.is_occupied(candidate.pos, None))
                    .copied()
            })?
        });
        let Some(standing) = standing else {
            let reason = if allow_staging_overflow {
                format!(
                    "Encounter {:?}: {} has no free staging surface reachable from formation \
                     centre {:?}. The terrain cannot stage the frozen deployment roster.",
                    encounter.name,
                    describe(unit),
                    middle.pos
                )
            } else {
                format!(
                    "Encounter {:?}: {} has no free surface within {spread} steps of its formation \
                     centre {:?}. The formation needs a wider spread, or the roster is larger than \
                     the ground it was given.",
                    encounter.name,
                    describe(unit),
                    middle.pos
                )
            };
            return Err(reason);
        };
        occupancy.relocate(
            UnitId(u64::try_from(index).unwrap_or(u64::MAX)),
            standing.pos,
        );
        if let Some(slot) = resolved.get_mut(index) {
            *slot = Some(standing);
        }
    }

    // Every entry, or a reason. A `None` here would be a bug in the two passes above
    // rather than a designer's mistake, so it is reported as what it is.
    units
        .into_iter()
        .zip(resolved)
        .map(|(unit, standing)| {
            standing.map(|standing| (unit, standing)).ok_or_else(|| {
                format!(
                    "Encounter {:?}: {} was never placed.",
                    encounter.name,
                    describe(&unit)
                )
            })
        })
        .collect()
}

/// How a roster entry is named in a message a designer has to act on.
fn describe(unit: &RosteredUnit<'_>) -> String {
    format!("the {} {:?}", unit.faction.label(), unit.archetype)
}

/// The lowest surface at an authored coordinate that this body fits on.
///
/// The ground, rather than any bridge built over it — an authored coordinate is written
/// on a map whose landmarks do not move, so the ambiguity a stacked column introduces is
/// resolved downwards, where the designer was looking.
fn authored_standing(
    coord: CubeCoord,
    footing: &Footing,
    unit: &RosteredUnit<'_>,
    encounter: &Encounter,
) -> Result<Standing, String> {
    let described = describe(unit);
    let coord = coord_from(coord)
        .map_err(|reason| format!("Encounter {:?}: {described} {reason}.", encounter.name))?;
    footing.ground(coord).ok_or_else(|| {
        format!(
            "Encounter {:?}: {described} is placed at {coord:?}, where nothing can be stood on.",
            encounter.name
        )
    })
}

/// The exact surface a generated anchor published.
///
/// Never a nearby surface and never the origin: an anchor promises one voxel, and
/// substituting another would hide a generator or validation defect — and could put the
/// unit on the ground *beneath* the bridge the anchor named.
fn anchored_standing(
    name: &str,
    footing: &Footing,
    anchors: Option<&MapAnchors>,
    unit: &RosteredUnit<'_>,
    encounter: &Encounter,
) -> Result<Standing, String> {
    let described = describe(unit);
    let id = MapAnchorId::from(name);
    let Some(anchors) = anchors else {
        return Err(format!(
            "Encounter {:?}: {described} uses map anchor \"{id}\", but the active map published \
             no anchors.",
            encounter.name
        ));
    };
    let Some(pos) = anchors.get(&id) else {
        return Err(format!(
            "Encounter {:?}: {described} uses missing map anchor \"{id}\".",
            encounter.name
        ));
    };
    footing.at(pos).ok_or_else(|| {
        format!(
            "Encounter {:?}: map anchor \"{id}\" for {described} points to {pos:?}, which its \
             body cannot stand on.",
            encounter.name
        )
    })
}

/// The surfaces a formation may use, nearest first.
///
/// Walkable from the centre rather than merely near it: a flood fill through
/// [`Reach`] uses the same footing and traversal rules movement does, so a formation
/// never spreads across a chasm, onto a ledge the body cannot climb, or under a ceiling
/// it does not fit beneath.
///
/// Sorted explicitly. [`Reach`] is a hash map, so its iteration order is not a promise,
/// and a formation that dealt its surfaces in a different order between runs would make
/// the same encounter on the same seed a different fight.
fn formation_surfaces(center: Standing, footing: &Footing, spread: Option<u32>) -> Vec<Standing> {
    let reach = Reach::from(center, footing, spread);
    let mut surfaces: Vec<Standing> = reach.surfaces().collect();
    surfaces.sort_by_key(|surface| {
        (
            reach.cost(surface.pos).unwrap_or(u32::MAX),
            surface.pos.coord,
            surface.pos.level,
        )
    });
    surfaces
}

/// Everything that differs between one unit and the next.
///
/// Grouped into a struct because the alternative is an eight-argument function where
/// two of the arguments are strings and easy to swap by accident.
struct UnitSpawn<'a> {
    standing: Standing,
    faction: Faction,
    material: Handle<StandardMaterial>,
    archetype: &'a str,
    /// The lattice this archetype names, if the library resolved one.
    ///
    /// `Option` because the library is absent until content resolves, and because an
    /// encounter may name an archetype `lattices.ron` does not define. Neither is fatal:
    /// a unit with no lattice stands, walks and strikes exactly as every unit did before
    /// this — it simply cannot cast and cannot be damaged. That is the honest fallback,
    /// and `spawn_units` warns when it happens so it is not a silent one.
    lattice: Option<&'a hex_assets::Archetype>,
    controller: Option<AiController>,
    settings: &'a PlayerSettings,
    body: Body,
}

fn spawn_unit(
    commands: &mut Commands,
    assets: &GameAssets,
    spawn: UnitSpawn,
    identity: &mut UnitIdentity,
) {
    let standing = spawn.standing;
    let scale = spawn.settings.scale;
    let [mesh_a, mesh_b] = assets.player_pieces.clone();

    let child_transform = Transform {
        // Offsets the mesh so its origin sits on the tile centre.
        translation: Vec3::new(-scale, -scale, -10. * scale),
        scale: Vec3::splat(scale),
        ..default()
    };

    let id = identity.allocator.allocate();
    let mut unit = commands.spawn((
        Transform::from_translation(standing.world_position()),
        Visibility::default(),
        PresentationOcclusion::default(),
        StandsOn(standing),
        spawn.body,
        spawn.faction,
        id,
        match spawn.faction {
            Faction::Player => ControlOwner::default(),
            Faction::Hostile => ControlOwner(hex_core::PlayerSeat::AI),
        },
        Archetype(spawn.archetype.to_owned()),
        // Archetype plus the stable id, so two wolves are distinguishable in the
        // inspector and each one's name matches what the log calls it.
        Name::new(format!("{} #{}", spawn.archetype, id.0)),
    ));

    // The archetype seam, and the whole reason `Archetype` went on at spawn time: one
    // lookup here rather than per-unit stat code everywhere. The three ride together
    // because they are meaningless apart — a spec with no stats has gems that hold
    // nothing, and a state built against a different spec addresses cells that are not
    // there.
    if let Some(lattice) = spawn.lattice {
        unit.insert((
            lattice.spec.clone(),
            LatticeState::new(&lattice.spec, &lattice.stats),
            lattice.stats.clone(),
        ));
    }
    if let Some(controller) = spawn.controller {
        unit.insert(controller);
    }

    match spawn.faction {
        Faction::Player => unit.insert(Player),
        Faction::Hostile => unit.insert(Enemy),
    };
    identity.registry.register(id, unit.id());
    if spawn.faction == Faction::Player {
        identity.party.members.push(id);
    }

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

    fn standing(q: i32, r: i32) -> Standing {
        Standing {
            pos: TilePos::new(HexCoord::from_axial(q, r), 1),
            span: HexSpan::from_ground(1.0),
        }
    }

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

    #[test]
    fn combat_freezes_converging_routes_on_unique_domain_surfaces() {
        let trigger_start = standing(-1, 0);
        let shared = standing(0, 0);
        let trigger_next = standing(1, 0);
        let follower_start = standing(0, 1);
        let speed = 1.0;
        let elapsed = leg_duration(trigger_start, shared, speed);

        let mut trigger_route = MovingTo::new(vec![trigger_start, shared, trigger_next], speed);
        trigger_route.started = true;
        trigger_route.elapsed = elapsed;
        trigger_route.reconciled_step = 1;
        let mut follower_route = MovingTo::new(vec![follower_start, shared, trigger_next], speed);
        follower_route.started = true;
        follower_route.elapsed = elapsed;
        follower_route.reconciled_step = 1;

        let mut world = World::new();
        let trigger = world
            .spawn((
                UnitId(0),
                StandsOn(shared),
                trigger_route,
                StopMovingAt::new(shared),
                hex_core::Busy,
                Transform::default(),
            ))
            .id();
        let follower = world
            .spawn((
                UnitId(1),
                StandsOn(shared),
                follower_route,
                hex_core::Busy,
                Transform::default(),
            ))
            .id();

        halt_on_combat(&mut world);

        let trigger_stop = world.get::<StandsOn>(trigger).expect("trigger remains").0;
        let follower_stop = world.get::<StandsOn>(follower).expect("follower remains").0;
        assert_eq!(
            trigger_stop, shared,
            "the exact engagement crossing has priority"
        );
        assert_ne!(
            follower_stop, shared,
            "the follower must choose another committed route surface"
        );
        assert!(
            [follower_start, trigger_next].contains(&follower_stop),
            "the collision resolver moved the follower off its committed route"
        );
        for entity in [trigger, follower] {
            assert!(world.get::<MovingTo>(entity).is_none());
            assert!(world.get::<hex_core::Busy>(entity).is_none());
            assert!(world.get::<Transformation>(entity).is_none());
        }
    }
}
