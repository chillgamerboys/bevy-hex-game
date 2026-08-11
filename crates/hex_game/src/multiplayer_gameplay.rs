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
use hex_assets::{ArtPalette, GameAssets, PlayerSettings};
use hex_combat::{
    CombatEvent, CombatSystems, CommandRefusal, EncounterOutcome, EncounterResolution,
    FactionLatticeKnowledge, PersistentEffects, TurnOrder,
};
use hex_core::{
    Busy, CommandQueue, CommandRequestId, ControlOwner, Faction, GameCommand, GameplaySetup,
    HexSpan, HexTile, IssuedCommand, LocalGameCommandRequest, Mode, Pause, PendingDecision,
    PlayerSeat, Screen, SimulationRole, TilePos, Turn, UnitId,
};
use hex_lattice::LatticeState;
use hex_multiplayer::{
    ArchetypeIdentityV1, AuthenticatedCommandRequest, AuthorityBoundary,
    AuthorityCommandResolution, AuthoritySequence, BoundedVec, CommandOutcome,
    CommandRefusalReason, CommandSequencer, GameCommandRequest, LobbyPhase, MotionReplicaV1,
    ReplicaValidationError, SequencerError, SessionAdmissionAuthority, SessionOutcome,
    SessionReplica, SessionRuntimeSystems, UnitReplica, MAX_ROUTE_STEPS, MAX_SESSION_UNITS,
    MAX_UNIT_EFFECTS,
};
use hex_units::{
    spawn_replica_unit, Archetype, Downed, HexPathingLine, MovementSystems, MovingTo, Party,
    ReplicaUnitSpawn, StandsOn, UnitAllocator, UnitRegistry,
};

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

/// Validated reconnect baseline applied only after L3 restores the world snapshot.
#[derive(Message, Debug, Clone)]
pub(crate) struct ApplyReplicaBaseline {
    units: BTreeMap<UnitId, UnitReplica>,
    session: SessionReplica,
}

impl ApplyReplicaBaseline {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "L4 begins constructing typed baselines once its live-snapshot adapter lands"
        )
    )]
    pub(crate) fn new(
        units: impl IntoIterator<Item = UnitReplica>,
        session: SessionReplica,
    ) -> Result<Self, ReplicaBaselineError> {
        session
            .validate()
            .map_err(ReplicaBaselineError::InvalidSession)?;
        let mut canonical = BTreeMap::new();
        for unit in units {
            unit.validate()
                .map_err(|error| ReplicaBaselineError::InvalidUnit {
                    unit: unit.unit,
                    error,
                })?;
            let id = unit.unit;
            if canonical.insert(id, unit).is_some() {
                return Err(ReplicaBaselineError::DuplicateUnit(id));
            }
            if canonical.len() > MAX_SESSION_UNITS {
                return Err(ReplicaBaselineError::TooManyUnits);
            }
        }
        Ok(Self {
            units: canonical,
            session,
        })
    }
}

/// Why a locally received live-session baseline was not safe to apply.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "L4 exposes this typed refusal through its reconnect loading adapter"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReplicaBaselineError {
    InvalidSession(ReplicaValidationError),
    InvalidUnit {
        unit: UnitId,
        error: ReplicaValidationError,
    },
    DuplicateUnit(UnitId),
    TooManyUnits,
}

impl std::fmt::Display for ReplicaBaselineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSession(error) => write!(formatter, "invalid session baseline: {error}"),
            Self::InvalidUnit { unit, error } => {
                write!(formatter, "invalid unit baseline {unit:?}: {error}")
            }
            Self::DuplicateUnit(unit) => write!(formatter, "baseline repeats unit {unit:?}"),
            Self::TooManyUnits => formatter.write_str("baseline exceeds the session unit bound"),
        }
    }
}

impl std::error::Error for ReplicaBaselineError {}

#[derive(Resource, Debug, Default)]
struct ReplicaBaselineState {
    active: Option<ApplyReplicaBaseline>,
    apply_active_once: bool,
    apply_all_network_once: bool,
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<LocalRequestIds>()
        .init_resource::<PendingAuthorityRequests>()
        .init_resource::<AuthorityReplicaEntities>()
        .init_resource::<BoundaryProjection>()
        .init_resource::<ReplicaBaselineState>()
        .add_message::<ApplyReplicaBaseline>()
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
                capture_replica_baseline,
                release_replica_baseline_when_caught_up,
                materialize_missing_unit_replicas,
                apply_effect_replicas,
                apply_unit_replicas,
                apply_session_replica,
                withdraw_unprojected_hostiles,
                finish_replica_baseline_transition,
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
                clear_replica_baseline,
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
        | CommandRefusal::TargetUnoccupied { .. }
        | CommandRefusal::TargetOutOfTouchReach { .. }
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
        | CommandRefusal::RestorationTarget { .. }
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
            &'static Archetype,
            &'static Faction,
            &'static StandsOn,
            Option<&'static MovingTo>,
            Option<&'static ControlOwner>,
            Has<Downed>,
            Option<&'static Turn>,
            Option<&'static LatticeState>,
        ),
    >,
    unit_replicas: Query<'w, 's, &'static UnitReplica>,
    session_replicas: Query<'w, 's, &'static SessionReplica>,
}

fn publish_authority_replicas(
    mut commands: Commands,
    authority: Option<Res<SessionAdmissionAuthority>>,
    mut sequencer: ResMut<CommandSequencer>,
    mode: Res<State<Mode>>,
    pause: Option<Res<State<Pause>>>,
    order: Res<TurnOrder>,
    pending: Res<PendingDecision>,
    resolution: Res<EncounterResolution>,
    effects: Res<PersistentEffects>,
    lattice_knowledge: Res<FactionLatticeKnowledge>,
    mut entities: ResMut<AuthorityReplicaEntities>,
    sources: AuthorityProjectionSources,
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
    let mut desired_units = BTreeMap::new();
    let mut projected_units = sources.units.iter().collect::<Vec<_>>();
    projected_units.sort_by_key(|(unit, ..)| **unit);

    for (unit, archetype, faction, standing, moving, owner, downed, turn, lattice) in
        projected_units
    {
        if *faction == Faction::Hostile && lattice_knowledge.view(Faction::Player, *unit).is_none()
        {
            continue;
        }
        let Some(replica) = unit_replica(
            *unit,
            archetype,
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
        desired_units.insert(*unit, replica);
    }

    let stale = entities
        .units
        .keys()
        .copied()
        .filter(|unit| !desired_units.contains_key(unit))
        .collect::<Vec<_>>();
    let initiative = match BoundedVec::<_, MAX_SESSION_UNITS>::new(order.order().to_vec()) {
        Ok(initiative) => initiative,
        Err(_) => {
            error!("initiative exceeded the multiplayer session bound");
            return;
        }
    };
    let mut session = SessionReplica {
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

    let units_changed = !stale.is_empty()
        || desired_units.iter().any(|(unit, desired)| {
            entities
                .units
                .get(unit)
                .and_then(|entity| sources.unit_replicas.get(*entity).ok())
                .is_none_or(|current| current != desired)
        });
    let current_session = entities
        .session
        .and_then(|entity| sources.session_replicas.get(entity).ok());
    let session_facts_changed =
        current_session.is_none_or(|current| !session_replica_facts_match(current, &session));
    let domain_changed = units_changed || session_facts_changed;
    let Ok(sequence) = authority_projection_sequence(
        &mut sequencer,
        current_session.map(|current| current.authority_sequence),
        domain_changed,
    ) else {
        error!("authority sequence could not represent the next projection boundary");
        return;
    };
    session.authority_sequence = sequence;

    for (unit, replica) in desired_units {
        match entities.units.get(&unit).copied() {
            Some(entity) => {
                if sources
                    .unit_replicas
                    .get(entity)
                    .is_ok_and(|current| current != &replica)
                {
                    commands.entity(entity).insert(replica);
                }
            }
            None => {
                let entity = commands.spawn((Replicated, replica)).id();
                entities.units.insert(unit, entity);
            }
        }
    }
    for unit in stale {
        if let Some(entity) = entities.units.remove(&unit) {
            commands.entity(entity).despawn();
        }
    }

    match entities.session {
        Some(entity) => {
            if sources
                .session_replicas
                .get(entity)
                .is_ok_and(|current| current != &session)
            {
                commands.entity(entity).insert(session);
            }
        }
        None => {
            entities.session = Some(commands.spawn((Replicated, session)).id());
        }
    }
}

fn session_replica_facts_match(current: &SessionReplica, desired: &SessionReplica) -> bool {
    let mut comparable = desired.clone();
    comparable.authority_sequence = current.authority_sequence;
    current == &comparable
}

fn authority_projection_sequence(
    sequencer: &mut CommandSequencer,
    published: Option<AuthoritySequence>,
    domain_changed: bool,
) -> Result<AuthoritySequence, ProjectionSequenceError> {
    let Some(published) = published else {
        return Ok(sequencer.last_sequence());
    };
    if sequencer.last_sequence() < published {
        return Err(ProjectionSequenceError::PublishedAhead);
    }
    if domain_changed && sequencer.last_sequence() == published {
        sequencer
            .advance_system_boundary()
            .map_err(ProjectionSequenceError::Sequencer)
    } else {
        // A command result may already have allocated the sequence for these facts.
        // Reusing it keeps CommandResult and its first projection on one boundary.
        Ok(sequencer.last_sequence())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProjectionSequenceError {
    PublishedAhead,
    Sequencer(SequencerError),
}

#[expect(
    clippy::too_many_arguments,
    reason = "a unit replica is the deliberate flat boundary across these exact facts"
)]
fn unit_replica(
    unit: UnitId,
    archetype: &Archetype,
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
        archetype: ArchetypeIdentityV1::new(archetype.0.clone()).ok()?,
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

fn capture_replica_baseline(
    mut baselines: MessageReader<ApplyReplicaBaseline>,
    mut state: ResMut<ReplicaBaselineState>,
) {
    for baseline in baselines.read() {
        state.active = Some(baseline.clone());
        state.apply_active_once = true;
        state.apply_all_network_once = false;
    }
}

fn release_replica_baseline_when_caught_up(
    network: Query<&SessionReplica>,
    mut state: ResMut<ReplicaBaselineState>,
) {
    let Some(baseline_sequence) = state
        .active
        .as_ref()
        .map(|baseline| baseline.session.authority_sequence)
    else {
        return;
    };
    let caught_up = network.iter().any(|replica| {
        replica.validate().is_ok() && replica.authority_sequence >= baseline_sequence
    });
    if caught_up {
        state.active = None;
        state.apply_active_once = false;
        state.apply_all_network_once = true;
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "replica materialization explicitly joins presentation assets, world surfaces, and stable unit identity"
)]
fn materialize_missing_unit_replicas(
    mut commands: Commands,
    assets: Res<GameAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    palette: Res<ArtPalette>,
    settings: Res<PlayerSettings>,
    mut allocator: ResMut<UnitAllocator>,
    mut registry: ResMut<UnitRegistry>,
    mut party: ResMut<Party>,
    state: Res<ReplicaBaselineState>,
    network: Query<&UnitReplica>,
    tiles: Query<(&TilePos, &HexSpan), With<HexTile>>,
) {
    let surfaces = tiles
        .iter()
        .map(|(position, span)| (*position, *span))
        .collect::<BTreeMap<_, _>>();
    let projected = state.active.as_ref().map_or_else(
        || network.iter().collect::<Vec<_>>(),
        |baseline| baseline.units.values().collect::<Vec<_>>(),
    );
    for replica in projected {
        if let Err(error) = replica.validate() {
            error!("refusing to materialize invalid unit replica: {error}");
            continue;
        }
        if registry.entity_of(replica.unit).is_some() {
            continue;
        }
        let Some(span) = surfaces.get(&replica.position).copied() else {
            error!(
                "unit replica names an absent map surface: {:?}",
                replica.position
            );
            continue;
        };
        let spawn = ReplicaUnitSpawn {
            id: replica.unit,
            standing: hex_units::Standing {
                pos: replica.position,
                span,
            },
            faction: replica.faction,
            owner: replica.owner,
            archetype: replica.archetype.as_str(),
            lattice: replica.lattice.clone(),
        };
        if let Err(error) = spawn_replica_unit(
            &mut commands,
            &assets,
            &mut materials,
            &palette,
            &settings,
            &mut allocator,
            &mut registry,
            &mut party,
            spawn,
        ) {
            error!("could not materialize disclosed unit replica: {error}");
        }
    }
}

fn apply_unit_replicas(
    mut commands: Commands,
    registry: Res<UnitRegistry>,
    state: Res<ReplicaBaselineState>,
    network: Query<&UnitReplica>,
    changed: Query<&UnitReplica, Changed<UnitReplica>>,
    tiles: Query<(&TilePos, &HexSpan), With<HexTile>>,
    mut units: Query<(
        Entity,
        &Faction,
        &Archetype,
        &mut StandsOn,
        &mut ControlOwner,
        &mut Transform,
        Option<&mut LatticeState>,
    )>,
) {
    let replicas = if let Some(baseline) = &state.active {
        if !state.apply_active_once {
            return;
        }
        baseline.units.values().cloned().collect::<Vec<_>>()
    } else if state.apply_all_network_once {
        network.iter().cloned().collect::<Vec<_>>()
    } else {
        changed.iter().cloned().collect::<Vec<_>>()
    };
    let surfaces = tiles
        .iter()
        .map(|(position, span)| (*position, *span))
        .collect::<BTreeMap<_, _>>();
    for replica in replicas {
        if let Err(error) = replica.validate() {
            error!("refusing invalid unit replica: {error}");
            continue;
        }
        let Some(entity) = registry.entity_of(replica.unit) else {
            continue;
        };
        let Ok((entity, faction, archetype, mut standing, mut owner, mut transform, lattice)) =
            units.get_mut(entity)
        else {
            continue;
        };
        if *faction != replica.faction || archetype.0 != replica.archetype.as_str() {
            error!(
                "refusing replica identity change for stable unit {:?}",
                replica.unit
            );
            continue;
        }
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
                commands.entity(entity).remove::<LatticeState>();
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
    network: Query<&UnitReplica>,
    mut removed: RemovedComponents<UnitReplica>,
    state: Res<ReplicaBaselineState>,
    mut effects: ResMut<PersistentEffects>,
) {
    let replica_removed = removed.read().next().is_some();
    let apply_baseline = state.active.is_some() && state.apply_active_once;
    let apply_network = state.active.is_none()
        && (state.apply_all_network_once || !changed.is_empty() || replica_removed);
    if !apply_baseline && !apply_network {
        return;
    }
    let mut projected = Vec::new();
    let replicas = state.active.as_ref().map_or_else(
        || network.iter().collect::<Vec<_>>(),
        |baseline| baseline.units.values().collect::<Vec<_>>(),
    );
    for replica in replicas {
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
    network: Query<&SessionReplica>,
    changed: Query<&SessionReplica, Changed<SessionReplica>>,
    state: Res<ReplicaBaselineState>,
    mode: Option<ResMut<NextState<Mode>>>,
    pause: Option<ResMut<NextState<Pause>>>,
    mut order: ResMut<TurnOrder>,
    mut pending: ResMut<PendingDecision>,
    mut resolution: ResMut<EncounterResolution>,
) {
    let replica = if let Some(baseline) = &state.active {
        state.apply_active_once.then(|| baseline.session.clone())
    } else if state.apply_all_network_once {
        newest_valid_session(network.iter())
    } else {
        newest_valid_session(changed.iter())
    };
    let Some(replica) = replica else {
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

fn newest_valid_session<'a>(
    replicas: impl Iterator<Item = &'a SessionReplica>,
) -> Option<SessionReplica> {
    replicas
        .filter_map(|replica| match replica.validate() {
            Ok(()) => Some(replica.clone()),
            Err(error) => {
                error!("refusing invalid session replica: {error}");
                None
            }
        })
        .max_by_key(|replica| replica.authority_sequence)
}

fn withdraw_unprojected_hostiles(
    mut commands: Commands,
    state: Res<ReplicaBaselineState>,
    network: Query<&UnitReplica>,
    mut registry: ResMut<UnitRegistry>,
    units: Query<(Entity, &UnitId, &Faction)>,
) {
    let replicas = state.active.as_ref().map_or_else(
        || network.iter().collect::<Vec<_>>(),
        |baseline| baseline.units.values().collect::<Vec<_>>(),
    );
    let projected = replicas
        .into_iter()
        .filter(|replica| replica.validate().is_ok())
        .map(|replica| replica.unit)
        .collect::<BTreeSet<_>>();
    for (entity, unit, faction) in &units {
        if *faction == Faction::Hostile && !projected.contains(unit) {
            let registered = registry.unregister(*unit);
            if registered.is_some_and(|registered| registered != entity) {
                error!("unit registry disagreed with hostile replica withdrawal for {unit:?}");
            }
            commands.entity(entity).despawn();
        }
    }
}

fn finish_replica_baseline_transition(mut state: ResMut<ReplicaBaselineState>) {
    if state.apply_active_once || state.apply_all_network_once {
        state.apply_active_once = false;
        state.apply_all_network_once = false;
    }
}

fn clear_replica_baseline(mut state: ResMut<ReplicaBaselineState>) {
    *state = ReplicaBaselineState::default();
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
    use hex_assets::{PaletteSwatch, SrgbColor, SwatchId};
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

    fn session_replica(sequence: u64) -> SessionReplica {
        SessionReplica {
            authority_sequence: AuthoritySequence(sequence),
            mode: Mode::Exploring,
            pause: Pause(false),
            initiative: BoundedVec::default(),
            active_turn: None,
            round: 0,
            pending_decision: PendingDecision::default(),
            outcome: None,
        }
    }

    fn bounded_unit(unit: UnitId, faction: Faction) -> UnitReplica {
        UnitReplica {
            unit,
            archetype: ArchetypeIdentityV1::new(match faction {
                Faction::Player => "player",
                Faction::Hostile => "wolf",
            })
            .expect("test archetype should be bounded"),
            faction,
            position: TilePos::ORIGIN,
            motion: None,
            owner: ControlOwner(match faction {
                Faction::Player => PlayerSeat::HOST,
                Faction::Hostile => PlayerSeat::AI,
            }),
            lattice: None,
            downed: false,
            turn: None,
            effects: BoundedVec::default(),
        }
    }

    #[test]
    fn projection_sequences_advance_for_system_facts_and_reuse_human_results() {
        let mut system = CommandSequencer::default();
        assert_eq!(
            authority_projection_sequence(&mut system, Some(AuthoritySequence(0)), true),
            Ok(AuthoritySequence(1))
        );

        let mut human = CommandSequencer::default();
        let request = CommandRequestId(7);
        assert!(human.begin(PlayerSeat::HOST, request).is_ok());
        let result = human
            .finish(PlayerSeat::HOST, request, CommandOutcome::Accepted)
            .expect("begun command should finalize");
        assert_eq!(result.authority_sequence, AuthoritySequence(1));
        assert_eq!(
            authority_projection_sequence(&mut human, Some(AuthoritySequence(0)), true),
            Ok(AuthoritySequence(1)),
            "the projection must share the accepted command boundary"
        );
    }

    #[test]
    fn reconnect_baseline_is_canonical_and_held_until_network_catches_up() {
        let unit = bounded_unit(UnitId(4), Faction::Player);
        assert!(matches!(
            ApplyReplicaBaseline::new([unit.clone(), unit], session_replica(5),),
            Err(ReplicaBaselineError::DuplicateUnit(UnitId(4)))
        ));

        let baseline = ApplyReplicaBaseline::new(
            [bounded_unit(UnitId(4), Faction::Player)],
            session_replica(5),
        )
        .expect("valid reconnect baseline should canonicalize");
        let mut app = App::new();
        app.init_resource::<ReplicaBaselineState>()
            .add_message::<ApplyReplicaBaseline>()
            .add_systems(
                Update,
                (
                    capture_replica_baseline,
                    release_replica_baseline_when_caught_up,
                )
                    .chain(),
            );
        app.world_mut()
            .resource_mut::<Messages<ApplyReplicaBaseline>>()
            .write(baseline);
        app.update();
        assert!(app
            .world()
            .resource::<ReplicaBaselineState>()
            .active
            .is_some());

        let session_entity = app.world_mut().spawn(session_replica(4)).id();
        app.update();
        assert!(app
            .world()
            .resource::<ReplicaBaselineState>()
            .active
            .is_some());

        app.world_mut()
            .entity_mut(session_entity)
            .insert(session_replica(5));
        app.update();
        let state = app.world().resource::<ReplicaBaselineState>();
        assert!(state.active.is_none());
        assert!(state.apply_all_network_once);
    }

    #[test]
    fn disclosed_hostile_materializes_and_withdraws_as_a_complete_actor() {
        let swatch_id =
            SwatchId::new("unit/hostile").expect("test hostile swatch identity should be valid");
        let swatch = PaletteSwatch::new(
            "Hostile",
            SrgbColor::new(0.8, 0.2, 0.2).expect("test color should be valid"),
            BTreeSet::from(["test".to_owned()]),
        )
        .expect("test hostile swatch should be valid");
        let palette = ArtPalette::new(BTreeMap::from([(swatch_id, swatch)]))
            .expect("test palette should be valid");
        let mut app = App::new();
        app.insert_resource(GameAssets {
            hex_tile: Handle::default(),
            player_pieces: [Handle::default(), Handle::default()],
        })
        .insert_resource(Assets::<StandardMaterial>::default())
        .insert_resource(palette)
        .insert_resource(PlayerSettings {
            scale: 1.0,
            speed: 1.0,
        })
        .init_resource::<UnitAllocator>()
        .init_resource::<UnitRegistry>()
        .init_resource::<Party>()
        .init_resource::<ReplicaBaselineState>()
        .add_systems(
            Update,
            (
                materialize_missing_unit_replicas,
                withdraw_unprojected_hostiles,
            )
                .chain(),
        );
        app.world_mut()
            .spawn((HexTile, TilePos::ORIGIN, HexSpan::new(0.0, 1.0)));
        let projection = app
            .world_mut()
            .spawn(bounded_unit(UnitId(42), Faction::Hostile))
            .id();

        app.update();

        let actor = app
            .world()
            .resource::<UnitRegistry>()
            .entity_of(UnitId(42))
            .expect("disclosed hostile should materialize");
        assert_eq!(app.world().get::<Faction>(actor), Some(&Faction::Hostile));
        assert_eq!(
            app.world()
                .get::<Archetype>(actor)
                .map(|value| value.0.as_str()),
            Some("wolf")
        );
        assert!(app.world().get::<LatticeState>(actor).is_none());

        app.world_mut().entity_mut(projection).despawn();
        app.update();

        assert!(app
            .world()
            .resource::<UnitRegistry>()
            .entity_of(UnitId(42))
            .is_none());
        assert!(app.world().get_entity(actor).is_err());
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
            .init_resource::<ReplicaBaselineState>()
            .add_systems(Update, apply_effect_replicas);
        let replica = app
            .world_mut()
            .spawn(UnitReplica {
                unit: UnitId(3),
                archetype: ArchetypeIdentityV1::new("player")
                    .expect("test archetype should be bounded"),
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
            disclosure_safe_refusal(&CommandRefusal::TargetUnoccupied {
                target: TilePos::ORIGIN,
            }),
            CommandRefusalReason::InvalidTarget
        );
        assert_eq!(
            disclosure_safe_refusal(&CommandRefusal::TargetOutOfTouchReach { target: UnitId(42) }),
            CommandRefusalReason::InvalidTarget
        );
        assert_eq!(
            disclosure_safe_refusal(&CommandRefusal::RestorationTarget {
                reason: hex_combat::RestorationTargetRefusal::IncompleteHostileKnowledge {
                    target: UnitId(42),
                },
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
            &Archetype("wolf".to_owned()),
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
