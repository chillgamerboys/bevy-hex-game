//! Gameplay-owned adapters between session infrastructure and authoritative ECS facts.
//!
//! `hex_multiplayer` deliberately cannot query units, combat, or perception. This module
//! sits in the composition root, where it may translate both directions without teaching
//! shared protocol code about private implementations.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_replicon::prelude::Replicated;
use hex_anim::Transformation;
use hex_combat::{
    CombatEvent, CombatSystems, CommandRefusal, EncounterOutcome, EncounterResolution,
    FactionLatticeKnowledge, PersistentEffects, TurnOrder,
};
use hex_core::{
    Busy, CommandQueue, CommandRequestId, ControlOwner, Faction, GameCommand, GameplaySetup,
    HexSpan, HexTile, IssuedCommand, LocalGameCommandRequest, Mode, Pause, PendingDecision,
    PlayerSeat, Screen, SimulationRole, TilePos, Turn, UnitId,
};
use hex_lattice::{LatticeSpec, LatticeState};
use hex_multiplayer::{
    AuthenticatedCommandRequest, AuthorityBoundary, AuthorityCommandResolution, BoundedVec,
    CommandOutcome, CommandRefusalReason, CommandSequencer, GameCommandRequest, LobbyPhase,
    MotionReplicaV1, SessionAdmissionAuthority, SessionOutcome, SessionReplica,
    SessionRuntimeSystems, UnitReplica, MAX_ROUTE_STEPS, MAX_SESSION_UNITS, MAX_UNIT_EFFECTS,
};
use hex_units::{Downed, HexPathingLine, MovementSystems, MovingTo, StandsOn, UnitRegistry};

#[derive(Resource, Debug)]
struct LocalRequestIds {
    last: u64,
}

impl Default for LocalRequestIds {
    fn default() -> Self {
        Self::for_process(hex_multiplayer::SessionPeerId::generate())
    }
}

impl LocalRequestIds {
    fn for_process(identity: hex_multiplayer::SessionPeerId) -> Self {
        let identity = identity.to_bytes();
        let mut epoch = [0_u8; 8];
        epoch.copy_from_slice(&identity[..8]);
        Self {
            // A fresh process must not restart at one against the host's retained
            // reconnect/idempotence cache. Reserving the top bit guarantees at least
            // half the u64 range remains for monotonic allocation.
            last: u64::from_be_bytes(epoch) & (u64::MAX >> 1),
        }
    }

    fn allocate(&mut self) -> Option<CommandRequestId> {
        self.last = self.last.checked_add(1)?;
        Some(CommandRequestId(self.last))
    }
}

#[derive(Debug, Clone)]
struct PendingAuthorityRequest {
    source_seat: PlayerSeat,
    request_id: CommandRequestId,
    issued: IssuedCommand,
}

#[derive(Resource, Debug, Default)]
struct PendingAuthorityRequests(VecDeque<PendingAuthorityRequest>);

#[derive(Resource, Debug, Default)]
struct AuthorityReplicaEntities {
    units: BTreeMap<UnitId, Entity>,
    session: Option<Entity>,
}

#[derive(Resource, Debug, Default)]
struct BoundaryProjection {
    queue_nonempty: bool,
    decision_open: bool,
    movements: usize,
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<LocalRequestIds>()
        .init_resource::<PendingAuthorityRequests>()
        .init_resource::<AuthorityReplicaEntities>()
        .init_resource::<BoundaryProjection>()
        .add_systems(
            PreUpdate,
            enqueue_authenticated_commands
                .after(SessionRuntimeSystems::CommandIngress)
                .run_if(resource_equals(SimulationRole::Authority)),
        )
        .add_systems(
            Update,
            route_direct_human_commands
                .after(CombatSystems::Act)
                .before(CombatSystems::Apply)
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(
            Update,
            resolve_consumed_requests
                .after(CombatSystems::Apply)
                .before(CombatSystems::Resolve)
                .run_if(resource_equals(SimulationRole::Authority)),
        )
        .add_systems(
            Update,
            track_authority_boundary
                .after(CombatSystems::Apply)
                .after(MovementSystems::Reconcile)
                .before(SessionRuntimeSystems::Boundaries)
                .run_if(resource_equals(SimulationRole::Authority)),
        )
        .add_systems(
            Update,
            publish_authority_replicas
                .after(CombatSystems::Advance)
                .run_if(in_state(Screen::Gameplay))
                .run_if(resource_equals(SimulationRole::Authority)),
        )
        .add_systems(
            Update,
            (
                apply_effect_replicas,
                apply_unit_replicas,
                apply_session_replica,
                conceal_unprojected_hostiles,
            )
                .chain()
                .run_if(resource_equals(SimulationRole::Replica)),
        )
        .add_systems(
            OnEnter(Screen::Gameplay),
            apply_lobby_assignments
                .in_set(GameplaySetup::Restore)
                .run_if(resource_equals(SimulationRole::Authority)),
        )
        .add_systems(
            OnExit(Screen::Gameplay),
            (
                close_gameplay_authority_requests,
                despawn_authority_replicas,
            ),
        );
}

/// Converts every legacy direct human command before authority reduction.
///
/// The seat carried by `IssuedCommand` is intentionally discarded. Local authority
/// derives the host seat; a remote process sends only request id plus command.
fn route_direct_human_commands(
    role: Res<SimulationRole>,
    mut ids: ResMut<LocalRequestIds>,
    mut queue: ResMut<CommandQueue>,
    mut local: MessageWriter<LocalGameCommandRequest>,
    mut wire: MessageWriter<GameCommandRequest>,
) {
    for command in queue.take_direct_human_commands() {
        let Some(request_id) = ids.allocate() else {
            error!("local multiplayer request id exhausted; command refused before ingress");
            continue;
        };
        match *role {
            SimulationRole::Authority => {
                local.write(LocalGameCommandRequest {
                    request_id,
                    command,
                });
            }
            SimulationRole::Replica => {
                wire.write(GameCommandRequest {
                    request_id,
                    command,
                });
            }
        }
    }
}

fn enqueue_authenticated_commands(
    mut requests: MessageReader<AuthenticatedCommandRequest>,
    authority: Option<Res<SessionAdmissionAuthority>>,
    mut queue: ResMut<CommandQueue>,
    mut pending: ResMut<PendingAuthorityRequests>,
    mut resolutions: MessageWriter<AuthorityCommandResolution>,
) {
    for request in requests.read() {
        if !request.source_seat.is_human() {
            resolutions.write(refused_resolution(
                request,
                CommandRefusalReason::NotAuthorized,
            ));
            continue;
        }
        if authority
            .as_ref()
            .is_some_and(|authority| authority.lobby().snapshot().phase != LobbyPhase::Active)
        {
            resolutions.write(refused_resolution(request, CommandRefusalReason::WrongMode));
            continue;
        }

        let effective_seat =
            effective_command_seat(request.source_seat, &request.command, authority.as_deref());
        let issued = IssuedCommand {
            seat: effective_seat,
            command: request.command.clone(),
        };
        // One reducer pass drains the whole queue. Keeping at most one request for
        // an acting unit makes every refusal event correlate exactly to one pending
        // request, including two clients racing to command the same canonical unit.
        if queue.holds_command_for(request.command.unit()) {
            resolutions.write(refused_resolution(request, CommandRefusalReason::Busy));
            continue;
        }
        queue.push_authenticated(issued.clone(), request.source_seat, request.request_id);
        pending.0.push_back(PendingAuthorityRequest {
            source_seat: request.source_seat,
            request_id: request.request_id,
            issued,
        });
    }
}

fn refused_resolution(
    request: &AuthenticatedCommandRequest,
    reason: CommandRefusalReason,
) -> AuthorityCommandResolution {
    AuthorityCommandResolution {
        source_seat: request.source_seat,
        request_id: request.request_id,
        outcome: CommandOutcome::Refused(reason),
    }
}

fn effective_command_seat(
    source: PlayerSeat,
    command: &GameCommand,
    authority: Option<&SessionAdmissionAuthority>,
) -> PlayerSeat {
    if source != PlayerSeat::HOST {
        return source;
    }
    let Some(authority) = authority else {
        return source;
    };
    let assigned = authority
        .lobby()
        .snapshot()
        .seats
        .iter()
        .find(|seat| seat.assigned_units.contains(&command.unit()))
        .map(|seat| seat.seat);
    match assigned {
        Some(seat) if seat == PlayerSeat::HOST || authority.lobby().host_can_delegate(seat) => seat,
        _ => source,
    }
}

fn resolve_consumed_requests(
    queue: Res<CommandQueue>,
    mut pending: ResMut<PendingAuthorityRequests>,
    mut events: MessageReader<CombatEvent>,
    mut resolutions: MessageWriter<AuthorityCommandResolution>,
) {
    let mut refusals = events
        .read()
        .filter_map(|event| match event {
            CombatEvent::CommandRefused { command, refusal } => {
                Some((command.clone(), refusal.clone()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut remaining = VecDeque::new();

    while let Some(request) = pending.0.pop_front() {
        if queue.contains_authenticated(request.source_seat, request.request_id) {
            remaining.push_back(request);
            continue;
        }
        let refusal = refusals
            .iter()
            .position(|(command, _)| command == &request.issued.command)
            .map(|index| refusals.remove(index).1);
        let outcome = refusal.map_or(CommandOutcome::Accepted, |refusal| {
            CommandOutcome::Refused(disclosure_safe_refusal(&refusal))
        });
        resolutions.write(AuthorityCommandResolution {
            source_seat: request.source_seat,
            request_id: request.request_id,
            outcome,
        });
    }
    pending.0 = remaining;
}

fn disclosure_safe_refusal(refusal: &CommandRefusal) -> CommandRefusalReason {
    match refusal {
        CommandRefusal::UnknownUnit | CommandRefusal::MissingUnitData { .. } => {
            CommandRefusalReason::UnknownUnit
        }
        CommandRefusal::WrongSeat { .. } => CommandRefusalReason::WrongSeat,
        CommandRefusal::CombatOnly
        | CommandRefusal::PartyMovementUnavailable
        | CommandRefusal::RestExploringOnly
        | CommandRefusal::EncounterResolved { .. } => CommandRefusalReason::WrongMode,
        CommandRefusal::NotCurrentTurn { .. } | CommandRefusal::NoTurn => {
            CommandRefusalReason::NotCurrentTurn
        }
        CommandRefusal::DecisionPending { .. } => CommandRefusalReason::DecisionPending,
        CommandRefusal::Busy
        | CommandRefusal::MissingCombatData { .. }
        | CommandRefusal::ActionAlreadySpent => CommandRefusalReason::Busy,
        CommandRefusal::InvalidPath
        | CommandRefusal::MovementBudgetExceeded { .. }
        | CommandRefusal::PartyMove { .. } => CommandRefusalReason::InvalidPath,
        CommandRefusal::Occupied { .. } => CommandRefusalReason::Occupied,
        CommandRefusal::UnknownTarget { .. }
        | CommandRefusal::TargetDowned { .. }
        | CommandRefusal::TargetNotHostile { .. }
        | CommandRefusal::TargetOutOfMeleeReach { .. }
        | CommandRefusal::ActingUnitDowned { .. }
        | CommandRefusal::UnknownSpell { .. }
        | CommandRefusal::MissingSpellDefinition { .. }
        | CommandRefusal::UndeliverableSpell { .. }
        | CommandRefusal::MissingFacing { .. }
        | CommandRefusal::TargetOutOfRange { .. }
        | CommandRefusal::ShapeUnresolved { .. }
        | CommandRefusal::TargetUnobserved { .. }
        | CommandRefusal::TrajectoryBlocked { .. }
        | CommandRefusal::TerrainCreationBlocked { .. }
        | CommandRefusal::SpellNotInscribed { .. }
        | CommandRefusal::CastBlocked { .. }
        | CommandRefusal::CastPlanStale { .. }
        | CommandRefusal::RestorationUnavailable
        | CommandRefusal::RestUnavailable => CommandRefusalReason::InvalidTarget,
        CommandRefusal::Restoration { .. }
        | CommandRefusal::NoPendingDecision
        | CommandRefusal::WrongDecisionUnit { .. }
        | CommandRefusal::WrongDisableCount { .. }
        | CommandRefusal::CellOutsideLattice { .. }
        | CommandRefusal::DuplicateCell { .. }
        | CommandRefusal::CellAlreadyDisabled { .. } => CommandRefusalReason::InvalidChoice,
    }
}

fn apply_lobby_assignments(
    mut commands: Commands,
    authority: Option<Res<SessionAdmissionAuthority>>,
    registry: Res<UnitRegistry>,
) {
    let Some(authority) = authority else {
        return;
    };
    for seat in &authority.lobby().snapshot().seats {
        for &unit in seat.assigned_units.as_slice() {
            let Some(entity) = registry.entity_of(unit) else {
                error!("lobby assignment names missing party unit {unit:?}");
                continue;
            };
            commands.entity(entity).insert(ControlOwner(seat.seat));
        }
    }
}

fn track_authority_boundary(
    queue: Res<CommandQueue>,
    decision: Res<PendingDecision>,
    moving: Query<(), With<MovingTo>>,
    mut projected: ResMut<BoundaryProjection>,
    mut boundary: ResMut<AuthorityBoundary>,
) {
    let queue_nonempty = !queue.is_empty();
    if queue_nonempty != projected.queue_nonempty {
        if queue_nonempty {
            boundary.begin_command();
        } else if boundary.finish_command().is_err() {
            warn!("authority queue boundary was not balanced");
        }
        projected.queue_nonempty = queue_nonempty;
    }

    let decision_open = decision.is_open();
    if decision_open != projected.decision_open {
        if decision_open {
            boundary.begin_decision();
        } else if boundary.finish_decision().is_err() {
            warn!("authority decision boundary was not balanced");
        }
        projected.decision_open = decision_open;
    }

    let movements = moving.iter().count();
    while projected.movements < movements {
        boundary.begin_movement();
        projected.movements = projected.movements.saturating_add(1);
    }
    while projected.movements > movements {
        if boundary.finish_movement().is_err() {
            warn!("authority movement boundary was not balanced");
            projected.movements = movements;
            break;
        }
        projected.movements = projected.movements.saturating_sub(1);
    }
}

#[derive(SystemParam)]
struct AuthorityProjectionSources<'w, 's> {
    units: Query<
        'w,
        's,
        (
            &'static UnitId,
            &'static Faction,
            &'static StandsOn,
            Option<&'static MovingTo>,
            Option<&'static ControlOwner>,
            Has<Downed>,
            Option<&'static Turn>,
            Option<&'static LatticeState>,
        ),
    >,
    unit_replicas: Query<'w, 's, &'static mut UnitReplica>,
    session_replicas: Query<'w, 's, &'static mut SessionReplica>,
}

fn publish_authority_replicas(
    mut commands: Commands,
    authority: Option<Res<SessionAdmissionAuthority>>,
    sequencer: Res<CommandSequencer>,
    mode: Res<State<Mode>>,
    pause: Option<Res<State<Pause>>>,
    order: Res<TurnOrder>,
    pending: Res<PendingDecision>,
    resolution: Res<EncounterResolution>,
    effects: Res<PersistentEffects>,
    lattice_knowledge: Res<FactionLatticeKnowledge>,
    mut entities: ResMut<AuthorityReplicaEntities>,
    mut sources: AuthorityProjectionSources,
) {
    if authority.is_none() {
        return;
    }
    if (0..=PlayerSeat::LAST_HUMAN.0).any(|seat| sequencer.in_flight_for(PlayerSeat(seat)) > 0) {
        // The reducer writes its typed resolution during Update and L1 assigns the
        // result sequence during the following PreUpdate. Holding the component delta
        // for that boundary keeps the projection and CommandResult on one sequence.
        return;
    }
    let mut visible = BTreeSet::new();
    let mut projected_units = sources.units.iter().collect::<Vec<_>>();
    projected_units.sort_by_key(|(unit, ..)| **unit);

    for (unit, faction, standing, moving, owner, downed, turn, lattice) in projected_units {
        if *faction == Faction::Hostile && lattice_knowledge.view(Faction::Player, *unit).is_none()
        {
            continue;
        }
        let Some(replica) = unit_replica(
            *unit,
            *faction,
            standing,
            moving,
            owner.copied().unwrap_or_else(|| match faction {
                Faction::Player => ControlOwner::default(),
                Faction::Hostile => ControlOwner(PlayerSeat::AI),
            }),
            downed,
            turn.copied(),
            lattice,
            &effects,
        ) else {
            error!("unit {unit:?} exceeded a multiplayer projection bound");
            continue;
        };
        visible.insert(*unit);
        match entities.units.get(unit).copied() {
            Some(entity) => {
                if let Ok(mut current) = sources.unit_replicas.get_mut(entity) {
                    if *current != replica {
                        *current = replica;
                    }
                }
            }
            None => {
                let entity = commands.spawn((Replicated, replica)).id();
                entities.units.insert(*unit, entity);
            }
        }
    }

    let stale = entities
        .units
        .keys()
        .copied()
        .filter(|unit| !visible.contains(unit))
        .collect::<Vec<_>>();
    for unit in stale {
        if let Some(entity) = entities.units.remove(&unit) {
            commands.entity(entity).despawn();
        }
    }

    let initiative = match BoundedVec::<_, MAX_SESSION_UNITS>::new(order.order().to_vec()) {
        Ok(initiative) => initiative,
        Err(_) => {
            error!("initiative exceeded the multiplayer session bound");
            return;
        }
    };
    let session = SessionReplica {
        authority_sequence: sequencer.last_sequence(),
        mode: *mode.get(),
        pause: pause.as_deref().map_or(Pause(false), |pause| *pause.get()),
        initiative,
        active_turn: order.current(),
        round: order.round,
        pending_decision: pending.clone(),
        outcome: resolution.outcome().map(|outcome| match outcome {
            EncounterOutcome::Victory => SessionOutcome::Victory,
            EncounterOutcome::Defeat => SessionOutcome::Defeat,
        }),
    };
    match entities.session {
        Some(entity) => {
            if let Ok(mut current) = sources.session_replicas.get_mut(entity) {
                if *current != session {
                    *current = session;
                }
            }
        }
        None => {
            entities.session = Some(commands.spawn((Replicated, session)).id());
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "a unit replica is the deliberate flat boundary across these exact facts"
)]
fn unit_replica(
    unit: UnitId,
    faction: Faction,
    standing: &StandsOn,
    moving: Option<&MovingTo>,
    owner: ControlOwner,
    downed: bool,
    turn: Option<Turn>,
    lattice: Option<&LatticeState>,
    effects: &PersistentEffects,
) -> Option<UnitReplica> {
    let motion = moving
        .map(|moving| {
            let route = BoundedVec::<_, MAX_ROUTE_STEPS>::new(
                moving.path.iter().map(|standing| standing.pos).collect(),
            )?;
            let reconciled_step =
                u32::try_from(moving.reconciled_step()).map_err(|_conversion_error| {
                    hex_multiplayer::BoundError::TooManyItems {
                        maximum: MAX_ROUTE_STEPS,
                        actual: moving.reconciled_step(),
                    }
                })?;
            Ok::<_, hex_multiplayer::BoundError>(MotionReplicaV1 {
                route,
                speed_bits: moving.speed().to_bits(),
                elapsed_bits: moving.elapsed().to_bits(),
                started: moving.started(),
                reconciled_step,
            })
        })
        .transpose()
        .ok()?;
    let effects = BoundedVec::<_, MAX_UNIT_EFFECTS>::new(
        effects.on(unit).map(|(_, effect)| *effect).collect(),
    )
    .ok()?;
    Some(UnitReplica {
        unit,
        faction,
        position: standing.0.pos,
        motion,
        owner,
        // Player lattices are owned facts. Hostile truth is never serialized here:
        // L3 projects the shared player-faction knowledge view instead.
        lattice: (faction == Faction::Player)
            .then(|| lattice.cloned())
            .flatten(),
        downed,
        turn,
        effects,
    })
}

fn apply_unit_replicas(
    mut commands: Commands,
    registry: Res<UnitRegistry>,
    replicas: Query<&UnitReplica, Changed<UnitReplica>>,
    tiles: Query<(&TilePos, &HexSpan), With<HexTile>>,
    mut units: Query<(
        Entity,
        &mut StandsOn,
        &mut ControlOwner,
        &mut Transform,
        Option<&mut LatticeState>,
    )>,
) {
    let surfaces = tiles
        .iter()
        .map(|(position, span)| (*position, *span))
        .collect::<BTreeMap<_, _>>();
    for replica in &replicas {
        if let Err(error) = replica.validate() {
            error!("refusing invalid unit replica: {error}");
            continue;
        }
        let Some(entity) = registry.entity_of(replica.unit) else {
            continue;
        };
        let Ok((entity, mut standing, mut owner, mut transform, lattice)) = units.get_mut(entity)
        else {
            continue;
        };
        let Some(span) = surfaces.get(&replica.position).copied() else {
            error!(
                "unit replica names an absent map surface: {:?}",
                replica.position
            );
            continue;
        };
        standing.0 = hex_units::Standing {
            pos: replica.position,
            span,
        };
        *owner = replica.owner;

        match (&replica.lattice, lattice) {
            (Some(expected), Some(mut current)) => current.clone_from(expected),
            (Some(expected), None) => {
                commands.entity(entity).insert(expected.clone());
            }
            (None, Some(_)) if replica.faction == Faction::Hostile => {
                commands
                    .entity(entity)
                    .remove::<(LatticeSpec, LatticeState)>();
            }
            (None, _) => {}
        }
        if replica.downed {
            commands.entity(entity).insert(Downed);
        } else {
            commands.entity(entity).remove::<Downed>();
        }
        if let Some(turn) = replica.turn {
            commands.entity(entity).insert(turn);
        } else {
            commands.entity(entity).remove::<Turn>();
        }

        if let Some(motion) = &replica.motion {
            let route = motion
                .route
                .as_slice()
                .iter()
                .map(|position| {
                    surfaces
                        .get(position)
                        .copied()
                        .map(|span| hex_units::Standing {
                            pos: *position,
                            span,
                        })
                })
                .collect::<Option<Vec<_>>>();
            let Some(route) = route else {
                error!("unit motion replica contains an absent map surface");
                continue;
            };
            let Ok(reconciled_step) = usize::try_from(motion.reconciled_step) else {
                continue;
            };
            let Some(moving) = MovingTo::from_authoritative_clock(
                route.clone(),
                motion.speed(),
                motion.elapsed(),
                motion.started,
                reconciled_step,
            ) else {
                continue;
            };
            let mut animation: Transformation = HexPathingLine::new(&route, motion.speed()).into();
            if !animation.synchronize_clock(motion.elapsed(), motion.started) {
                continue;
            }
            animation.update(&mut transform, motion.elapsed());
            commands.entity(entity).insert((moving, animation, Busy));
        } else {
            transform.translation = standing.0.world_position();
            commands
                .entity(entity)
                .remove::<(MovingTo, Transformation, Busy)>();
        }
        commands.entity(entity).insert(Visibility::Inherited);
    }
}

fn apply_effect_replicas(
    changed: Query<(), Changed<UnitReplica>>,
    replicas: Query<&UnitReplica>,
    mut removed: RemovedComponents<UnitReplica>,
    mut effects: ResMut<PersistentEffects>,
) {
    let replica_removed = removed.read().next().is_some();
    if changed.is_empty() && !replica_removed {
        return;
    }
    let mut projected = Vec::new();
    for replica in &replicas {
        if let Err(error) = replica.validate() {
            error!("refusing effects from invalid unit replica: {error}");
            return;
        }
        for (ordinal, effect) in replica.effects.as_slice().iter().enumerate() {
            if effect.target != replica.unit {
                error!(
                    "refusing mismatched effect target {:?} on unit replica {:?}",
                    effect.target, replica.unit
                );
                return;
            }
            projected.push((replica.unit, ordinal, *effect));
        }
    }
    projected.sort_by_key(|(unit, ordinal, _)| (*unit, *ordinal));
    effects.replace_replica(projected.into_iter().map(|(_, _, effect)| effect));
}

fn apply_session_replica(
    replicas: Query<&SessionReplica, Changed<SessionReplica>>,
    mode: Option<ResMut<NextState<Mode>>>,
    pause: Option<ResMut<NextState<Pause>>>,
    mut order: ResMut<TurnOrder>,
    mut pending: ResMut<PendingDecision>,
    mut resolution: ResMut<EncounterResolution>,
) {
    let Some(replica) = replicas.iter().next() else {
        return;
    };
    if let Err(error) = replica.validate() {
        error!("refusing invalid session replica: {error}");
        return;
    }
    if let Some(mut mode) = mode {
        mode.set(replica.mode);
    }
    if let Some(mut pause) = pause {
        pause.set(replica.pause);
    }
    order.apply_replica(
        replica.initiative.as_slice(),
        replica.active_turn,
        replica.round,
    );
    pending.clone_from(&replica.pending_decision);
    resolution.0 = replica.outcome.map(|outcome| match outcome {
        SessionOutcome::Victory => EncounterOutcome::Victory,
        SessionOutcome::Defeat => EncounterOutcome::Defeat,
    });
}

fn conceal_unprojected_hostiles(
    mut commands: Commands,
    replicas: Query<&UnitReplica>,
    units: Query<(Entity, &UnitId, &Faction)>,
) {
    let projected = replicas
        .iter()
        .map(|replica| replica.unit)
        .collect::<BTreeSet<_>>();
    for (entity, unit, faction) in &units {
        if *faction == Faction::Hostile && !projected.contains(unit) {
            commands
                .entity(entity)
                .remove::<(LatticeSpec, LatticeState)>()
                .insert(Visibility::Hidden);
        }
    }
}

fn close_gameplay_authority_requests(
    mut pending: ResMut<PendingAuthorityRequests>,
    mut projected: ResMut<BoundaryProjection>,
    mut boundary: ResMut<AuthorityBoundary>,
    mut resolutions: MessageWriter<AuthorityCommandResolution>,
) {
    for request in pending.0.drain(..) {
        resolutions.write(AuthorityCommandResolution {
            source_seat: request.source_seat,
            request_id: request.request_id,
            outcome: CommandOutcome::Refused(CommandRefusalReason::WrongMode),
        });
    }
    if projected.queue_nonempty && boundary.finish_command().is_err() {
        warn!("gameplay teardown found an unbalanced authority queue boundary");
    }
    if projected.decision_open && boundary.finish_decision().is_err() {
        warn!("gameplay teardown found an unbalanced authority decision boundary");
    }
    for _ in 0..projected.movements {
        if boundary.finish_movement().is_err() {
            warn!("gameplay teardown found an unbalanced authority movement boundary");
            break;
        }
    }
    *projected = BoundaryProjection::default();
}

fn despawn_authority_replicas(
    mut commands: Commands,
    mut entities: ResMut<AuthorityReplicaEntities>,
) {
    for entity in entities.units.values().copied() {
        commands.entity(entity).despawn();
    }
    entities.units.clear();
    if let Some(entity) = entities.session.take() {
        commands.entity(entity).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::{EffectEnd, EffectPayload, PersistentEffect};

    fn router_app(role: SimulationRole) -> App {
        let mut app = App::new();
        app.insert_resource(role)
            .init_resource::<CommandQueue>()
            .insert_resource(LocalRequestIds { last: 0 })
            .add_message::<LocalGameCommandRequest>()
            .add_message::<GameCommandRequest>()
            .add_systems(Update, route_direct_human_commands);
        app
    }

    #[test]
    fn authority_input_discards_the_emitter_claimed_seat() {
        let mut app = router_app(SimulationRole::Authority);
        let command = GameCommand::EndTurn { unit: UnitId(7) };
        app.world_mut()
            .resource_mut::<CommandQueue>()
            .push(IssuedCommand {
                seat: PlayerSeat(5),
                command: command.clone(),
            });

        app.update();

        let requests = app
            .world_mut()
            .resource_mut::<Messages<LocalGameCommandRequest>>()
            .drain()
            .collect::<Vec<_>>();
        assert_eq!(
            requests,
            vec![LocalGameCommandRequest {
                request_id: CommandRequestId(1),
                command,
            }]
        );
        assert!(app.world().resource::<CommandQueue>().is_empty());
    }

    #[test]
    fn replica_input_emits_only_the_seatless_wire_request() {
        let mut app = router_app(SimulationRole::Replica);
        let command = GameCommand::EndTurn { unit: UnitId(8) };
        app.world_mut()
            .resource_mut::<CommandQueue>()
            .push(IssuedCommand {
                seat: PlayerSeat(4),
                command: command.clone(),
            });

        app.update();

        let requests = app
            .world_mut()
            .resource_mut::<Messages<GameCommandRequest>>()
            .drain()
            .collect::<Vec<_>>();
        assert_eq!(
            requests,
            vec![GameCommandRequest {
                request_id: CommandRequestId(1),
                command,
            }]
        );
        assert!(app
            .world_mut()
            .resource_mut::<Messages<LocalGameCommandRequest>>()
            .drain()
            .next()
            .is_none());
    }

    #[test]
    fn offline_listen_host_uses_the_session_ingress_without_a_socket() {
        let mut app = App::new();
        app.add_plugins((
            bevy::time::TimePlugin,
            bevy::state::app::StatesPlugin,
            hex_multiplayer::MultiplayerPlugin,
        ))
        .init_resource::<CommandQueue>()
        .insert_resource(LocalRequestIds { last: 0 })
        .init_resource::<PendingAuthorityRequests>()
        .add_systems(Update, route_direct_human_commands)
        .add_systems(
            PreUpdate,
            enqueue_authenticated_commands.after(SessionRuntimeSystems::CommandIngress),
        );
        app.finish();
        app.cleanup();
        let command = GameCommand::EndTurn { unit: UnitId(10) };
        app.world_mut()
            .resource_mut::<CommandQueue>()
            .push(IssuedCommand {
                seat: PlayerSeat(5),
                command: command.clone(),
            });

        app.update();
        app.update();

        let queue = app.world().resource::<CommandQueue>();
        assert!(queue.contains_authenticated(PlayerSeat::HOST, CommandRequestId(1)));
        let applied = app.world_mut().resource_mut::<CommandQueue>().pop();
        assert_eq!(
            applied,
            Some(IssuedCommand {
                seat: PlayerSeat::HOST,
                command,
            }),
            "L1 derives the offline host seat after L2 discards the emitter's claim"
        );
    }

    #[test]
    fn authenticated_ingress_rejects_the_reserved_ai_seat() {
        let mut app = App::new();
        app.init_resource::<CommandQueue>()
            .init_resource::<PendingAuthorityRequests>()
            .add_message::<AuthenticatedCommandRequest>()
            .add_message::<AuthorityCommandResolution>()
            .add_systems(Update, enqueue_authenticated_commands);
        app.world_mut()
            .resource_mut::<Messages<AuthenticatedCommandRequest>>()
            .write(AuthenticatedCommandRequest {
                source_seat: PlayerSeat::AI,
                player_identity: hex_multiplayer::SessionPeerId::from_bytes([7; 16]),
                request_id: CommandRequestId(9),
                command: GameCommand::EndTurn { unit: UnitId(9) },
            });

        app.update();

        let resolutions = app
            .world_mut()
            .resource_mut::<Messages<AuthorityCommandResolution>>()
            .drain()
            .collect::<Vec<_>>();
        assert_eq!(
            resolutions,
            vec![AuthorityCommandResolution {
                source_seat: PlayerSeat::AI,
                request_id: CommandRequestId(9),
                outcome: CommandOutcome::Refused(CommandRefusalReason::NotAuthorized),
            }]
        );
        assert!(app.world().resource::<CommandQueue>().is_empty());
    }

    #[test]
    fn authenticated_race_for_one_unit_is_refused_before_reducer_correlation() {
        let mut app = App::new();
        app.init_resource::<CommandQueue>()
            .init_resource::<PendingAuthorityRequests>()
            .add_message::<AuthenticatedCommandRequest>()
            .add_message::<AuthorityCommandResolution>()
            .add_systems(Update, enqueue_authenticated_commands);
        let command = GameCommand::EndTurn { unit: UnitId(9) };
        for (seat, request_id) in [(PlayerSeat(1), 11), (PlayerSeat(2), 12)] {
            app.world_mut()
                .resource_mut::<Messages<AuthenticatedCommandRequest>>()
                .write(AuthenticatedCommandRequest {
                    source_seat: seat,
                    player_identity: hex_multiplayer::SessionPeerId::from_bytes([seat.0; 16]),
                    request_id: CommandRequestId(request_id),
                    command: command.clone(),
                });
        }

        app.update();

        assert!(app
            .world()
            .resource::<CommandQueue>()
            .contains_authenticated(PlayerSeat(1), CommandRequestId(11)));
        assert!(!app
            .world()
            .resource::<CommandQueue>()
            .contains_authenticated(PlayerSeat(2), CommandRequestId(12)));
        assert_eq!(
            app.world().resource::<PendingAuthorityRequests>().0.len(),
            1
        );
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<AuthorityCommandResolution>>()
                .drain()
                .collect::<Vec<_>>(),
            vec![AuthorityCommandResolution {
                source_seat: PlayerSeat(2),
                request_id: CommandRequestId(12),
                outcome: CommandOutcome::Refused(CommandRefusalReason::Busy),
            }]
        );
    }

    #[test]
    fn consumed_authenticated_request_is_correlated_to_one_result() {
        let mut app = App::new();
        app.init_resource::<CommandQueue>()
            .init_resource::<PendingAuthorityRequests>()
            .add_message::<CombatEvent>()
            .add_message::<AuthorityCommandResolution>()
            .add_systems(Update, resolve_consumed_requests);
        let issued = IssuedCommand {
            seat: PlayerSeat(2),
            command: GameCommand::EndTurn { unit: UnitId(12) },
        };
        app.world_mut()
            .resource_mut::<CommandQueue>()
            .push_authenticated(issued.clone(), PlayerSeat(2), CommandRequestId(41));
        app.world_mut()
            .resource_mut::<PendingAuthorityRequests>()
            .0
            .push_back(PendingAuthorityRequest {
                source_seat: PlayerSeat(2),
                request_id: CommandRequestId(41),
                issued,
            });
        let consumed = app.world_mut().resource_mut::<CommandQueue>().pop();
        assert!(consumed.is_some());

        app.update();

        let resolutions = app
            .world_mut()
            .resource_mut::<Messages<AuthorityCommandResolution>>()
            .drain()
            .collect::<Vec<_>>();
        assert_eq!(
            resolutions,
            vec![AuthorityCommandResolution {
                source_seat: PlayerSeat(2),
                request_id: CommandRequestId(41),
                outcome: CommandOutcome::Accepted,
            }]
        );
    }

    #[test]
    fn replica_effect_ledger_is_replaced_and_withdrawn_exactly() {
        let effect = PersistentEffect {
            source: UnitId(2),
            target: UnitId(3),
            payload: EffectPayload::Burn,
            start: 4,
            end: EffectEnd::AfterTurns(2),
            ticks: 1,
        };
        let bounded = BoundedVec::new(vec![effect]);
        assert!(bounded.is_ok());
        let mut app = App::new();
        app.init_resource::<PersistentEffects>()
            .add_systems(Update, apply_effect_replicas);
        let replica = app
            .world_mut()
            .spawn(UnitReplica {
                unit: UnitId(3),
                faction: Faction::Player,
                position: TilePos::ORIGIN,
                motion: None,
                owner: ControlOwner(PlayerSeat::HOST),
                lattice: None,
                downed: false,
                turn: None,
                effects: bounded.unwrap_or_default(),
            })
            .id();

        app.update();

        assert_eq!(
            app.world()
                .resource::<PersistentEffects>()
                .iter()
                .map(|(_, effect)| *effect)
                .collect::<Vec<_>>(),
            vec![effect]
        );

        app.world_mut().entity_mut(replica).despawn();
        app.update();
        assert!(app.world().resource::<PersistentEffects>().is_empty());
    }

    #[test]
    fn host_delegation_changes_only_the_effective_reducer_seat() {
        // The complete lobby transition is covered in `hex_multiplayer`; this pure
        // fallback proves offline input never invents a different canonical seat.
        assert_eq!(
            effective_command_seat(
                PlayerSeat::HOST,
                &GameCommand::EndTurn { unit: UnitId(2) },
                None,
            ),
            PlayerSeat::HOST
        );
    }

    #[test]
    fn gameplay_teardown_never_reuses_local_request_ids() {
        let mut app = App::new();
        app.insert_resource(LocalRequestIds { last: 0 })
            .init_resource::<PendingAuthorityRequests>()
            .init_resource::<BoundaryProjection>()
            .init_resource::<AuthorityBoundary>()
            .add_message::<AuthorityCommandResolution>()
            .add_systems(Update, close_gameplay_authority_requests);
        assert_eq!(
            app.world_mut().resource_mut::<LocalRequestIds>().allocate(),
            Some(CommandRequestId(1))
        );

        app.update();

        assert_eq!(
            app.world_mut().resource_mut::<LocalRequestIds>().allocate(),
            Some(CommandRequestId(2)),
            "returning through a lobby must not collide with the sequencer cache"
        );
    }

    #[test]
    fn restarted_process_uses_a_distinct_monotonic_request_epoch() {
        let mut first =
            LocalRequestIds::for_process(hex_multiplayer::SessionPeerId::from_bytes([0x11; 16]));
        let mut restarted =
            LocalRequestIds::for_process(hex_multiplayer::SessionPeerId::from_bytes([0x22; 16]));

        let first_request = first.allocate().expect("the first epoch has capacity");
        let next_request = first.allocate().expect("the first epoch stays monotonic");
        let restarted_request = restarted
            .allocate()
            .expect("the restarted epoch has capacity");

        assert_eq!(next_request.0, first_request.0 + 1);
        assert_ne!(restarted_request, first_request);
        assert!(first_request.0 <= (u64::MAX >> 1) + 1);
        assert!(restarted_request.0 <= (u64::MAX >> 1) + 1);
    }

    #[test]
    fn refusal_mapping_never_exposes_private_target_details() {
        assert_eq!(
            disclosure_safe_refusal(&CommandRefusal::TargetUnobserved {
                spell: "Scrying Eye".to_owned(),
                target: TilePos::ORIGIN,
            }),
            CommandRefusalReason::InvalidTarget
        );
        assert_eq!(
            disclosure_safe_refusal(&CommandRefusal::WrongSeat {
                issued_by: PlayerSeat(2),
                owned_by: PlayerSeat::AI,
            }),
            CommandRefusalReason::WrongSeat
        );
    }

    #[test]
    fn hostile_unit_projection_never_contains_authority_lattice_truth() {
        let effects = PersistentEffects::default();
        let standing = StandsOn(hex_units::Standing {
            pos: TilePos::ORIGIN,
            span: HexSpan::new(0.0, 1.0),
        });
        let replica = unit_replica(
            UnitId(9),
            Faction::Hostile,
            &standing,
            None,
            ControlOwner(PlayerSeat::AI),
            false,
            None,
            Some(&LatticeState::default()),
            &effects,
        )
        .expect("bounded hostile projection should build");
        assert!(replica.lattice.is_none());
        assert_eq!(replica.owner.0, PlayerSeat::AI);
    }
}
