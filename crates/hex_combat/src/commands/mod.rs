//! The applier: the one place a command becomes a change to the sim.
//!
//! Emitters — the click handler, the end-turn key, the AI — resolve *intent*
//! and push [`IssuedCommand`]s. This module drains the [`CommandQueue`] in
//! issue order, validates each command against the rules, and either applies
//! it or drops it with a logged reason. Nothing else mutates turn budgets,
//! starts walks, or lands strikes.
//!
//! # One verb, one file
//!
//! The drain loop below does the work every verb shares — resolving the unit,
//! checking the seat, logging a refusal — then dispatches to a module per verb.
//! A handler returns `Err(reason)` rather than logging and continuing, so the
//! loop owns the refusal path and each handler stays a straight-line function
//! that can be read on its own.
//!
//! That shape is load-bearing rather than tidiness: casting and defender choices
//! each became **a new file plus one match arm** instead of another hundred lines
//! inside one function. A verb
//! needing a fact the handlers lack adds a field to [`Verb`] instead of
//! changing every signature.
//!
//! # Where it runs
//!
//! [`CombatSystems::Apply`](crate::CombatSystems), between the AI's decision
//! and the turn advancing, inside `PausableSystems` — so a command issued just
//! before a pause waits in the queue rather than being lost, which is the whole
//! reason the funnel is a queue and not a `Message` (see
//! [`hex_core::commands`]).
//!
//! Despite the set's name, the applier is not combat-only: exploring moves are
//! commands too. Validation is what differs by [`Mode`] — free movement in
//! real time, turn ownership and budgets in combat.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use hex_anim::Transformation;
use hex_assets::{
    CombatSettings, ContentIndex, ContentTables, ElementCatalog, FormationCatalog, PlayerSettings,
    SpellBook, SubstanceTable,
};
use hex_core::{
    AppSystems, Busy, CommandQueue, ControlOwner, GameCommand, IssuedCommand, Mode, PartyFormation,
    PausableSystems, PendingDecision, Screen, TilePos, TraversalBlockers, Turn,
};
use hex_perception::FactionMapKnowledge;
use hex_units::{Body, Downed, Faction, MovingTo, Party, StandsOn, UnitRegistry};

use crate::outcomes::{CombatEvent, CommandRefusal};
use crate::turns::TurnOrder;

pub(crate) mod cast;
pub use cast::{delivers_anything, UNDELIVERABLE};
mod choose_disables;
mod choose_restores;
mod end_turn;
mod move_along;
mod move_party;
mod presentation;
mod rest;
mod strike;

/// Tiles, as the applier needs them to ground a commanded path.
type TileQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static TilePos,
        &'static hex_core::HexSpan,
        &'static hex_core::SubstanceId,
        &'static hex_core::Headroom,
    ),
    With<hex_core::HexTile>,
>;

/// Everything the applier reads or writes about one unit.
///
/// Standing and body are `Option` because not every verb needs them — ending a
/// turn moves nothing — and a test harness spawns only what its verbs read.
/// Each verb demands what it actually uses and drops the command otherwise.
type ActorQuery<'w, 's> = Query<
    'w,
    's,
    (
        Option<&'static StandsOn>,
        Option<&'static Body>,
        Option<&'static mut Turn>,
        Has<Busy>,
        Option<&'static ControlOwner>,
        Option<&'static Faction>,
        Has<Downed>,
    ),
>;

/// What a verb handler may reach for while applying one command.
///
/// Plain data only. `Commands` and the queries stay separate parameters
/// because each Bevy system parameter carries its own `'w`/`'s`, and holding
/// them together in one struct would force those lifetimes to unify.
///
/// A struct rather than a long argument list, so a verb that needs a new fact
/// adds a field here instead of editing every handler's signature — which is
/// what keeps concurrent work on different verbs from colliding.
struct Verb<'a> {
    turn_order: &'a TurnOrder,
    registry: &'a UnitRegistry,
    settings: Option<&'a PlayerSettings>,
    table: Option<&'a SubstanceTable>,
    /// What a spell is called, and what it costs.
    spells: Option<&'a SpellBook>,
    /// The engine's content lookups, borrowed for the length of one drain.
    tables: Option<ContentTables<'a>>,
    /// The one open decision, if resolution is parked on somebody's answer.
    pending: &'a mut PendingDecision,
    /// The ledger of effects that outlast the action that caused them.
    ///
    /// Casting a burn needs the persistent-effect ledger. Keeping that fact here avoids
    /// a ninth argument on `cast::apply`.
    effects: &'a mut crate::effects::PersistentEffects,
    /// Knowledge written by divination effects after a cast resolves.
    knowledge: &'a mut crate::knowledge::FactionLatticeKnowledge,
    /// World-owned current and remembered spatial knowledge for both factions.
    ///
    /// Casting fails closed when this is absent; no command may infer observation
    /// directly from authoritative terrain or unit entities.
    spatial: Option<&'a FactionMapKnowledge>,
    /// Structured outcomes accumulated in command order for presentation consumers.
    events: &'a mut Vec<CombatEvent>,
    /// Restored units waiting for a round boundary before initiative.
    revivals: &'a mut crate::turns::PendingRevivals,
    /// Policy knobs: budgets, ranges, and what a strike costs.
    combat: Option<&'a CombatSettings>,
    party: &'a Party,
    formation: &'a mut PartyFormation,
    formations: Option<&'a FormationCatalog>,
    /// Exact world-space obstacles excluded from footing for movement and reach.
    blockers: Option<&'a TraversalBlockers>,
    /// Units this drain already committed presentation for. `Busy` lands via
    /// `Commands` and is not queryable until the next sync point, so within one
    /// drain this set is the truth.
    committed: &'a mut Vec<Entity>,
    in_combat: bool,
}

/// Mutable resolution stores grouped to stay inside Bevy's system-parameter arity.
#[derive(SystemParam)]
struct ResolutionStores<'w> {
    pending: ResMut<'w, PendingDecision>,
    effects: ResMut<'w, crate::effects::PersistentEffects>,
    knowledge: ResMut<'w, crate::knowledge::FactionLatticeKnowledge>,
    spatial: Option<Res<'w, FactionMapKnowledge>>,
    events: MessageWriter<'w, CombatEvent>,
    revivals: ResMut<'w, crate::turns::PendingRevivals>,
    summary: ResMut<'w, crate::CombatSummary>,
}

#[derive(SystemParam)]
struct PartyStores<'w> {
    party: Res<'w, Party>,
    formation: ResMut<'w, PartyFormation>,
    formations: Option<Res<'w, FormationCatalog>>,
}

#[derive(SystemParam)]
struct UnitStores<'w, 's> {
    actors: ActorQuery<'w, 's>,
    lattices: cast::LatticeQuery<'w, 's>,
    lattice_stats: Query<'w, 's, &'static hex_lattice::LatticeStats>,
}

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<CommandQueue>()
        .init_resource::<Party>()
        .init_resource::<PartyFormation>()
        // A resource rather than a marker component since it carries a payload, so it
        // needs initialising as well as registering. Nothing sets it to anything but
        // `None` until the damage model lands.
        .init_resource::<PendingDecision>()
        .register_type::<GameCommand>()
        .register_type::<IssuedCommand>()
        .register_type::<Busy>()
        .register_type::<PendingDecision>();
    app.add_systems(
        Update,
        (
            // Frees units whose walk or swing landed this frame, before anyone
            // decides or applies anything on their behalf.
            sync_busy
                .in_set(AppSystems::Update)
                .in_set(PausableSystems)
                .after(hex_units::MovementSystems::Reconcile)
                .before(crate::CombatSystems::Act)
                .run_if(in_state(Screen::Gameplay)),
            apply_commands
                .in_set(crate::CombatSystems::Apply)
                .in_set(PausableSystems)
                .run_if(in_state(Screen::Gameplay)),
        ),
    );
    // Unit ids reset between sessions, so a held-over command would name
    // somebody else's unit next launch.
    app.add_systems(OnExit(Screen::Gameplay), clear_session_state);
    // A decision open when a fight ends has nobody left to answer it: both answer paths
    // are combat-only, so it would park every later cast behind "a decision is still
    // open" for the rest of the session.
    app.add_systems(OnExit(Mode::Combat), clear_pending_decision);
}

/// Forgets everything naming a unit, on the way out of a session.
///
/// Both of these outlive the screen — that is what being a resource means, and it is
/// exactly the property a per-entity marker did not have. Unit ids restart each
/// session, so a queued command or an unanswered decision held across one names
/// somebody else's unit next launch: the queue would apply to a stranger, and the
/// decision would park resolution forever on an answer nobody can give.
fn clear_session_state(mut queue: ResMut<CommandQueue>, mut pending: ResMut<PendingDecision>) {
    queue.clear();
    *pending = PendingDecision::None;
}

/// Drops an unanswered decision when a fight ends.
fn clear_pending_decision(mut pending: ResMut<PendingDecision>) {
    if pending.is_open() {
        warn!("combat ended with a decision still open; dropping it");
        *pending = PendingDecision::None;
    }
}

/// Keeps [`Busy`] equal to "presentation in flight".
///
/// The applier inserts [`Busy`] eagerly when it commits an animation; this
/// system is the other half, removing it once both the [`Transformation`] and
/// the [`MovingTo`] it stood for are gone — and re-asserting it for any
/// presentation that arrived outside the funnel, so the marker can be trusted
/// wherever it is read.
fn sync_busy(
    mut commands: Commands,
    units: Query<
        (Entity, Has<Transformation>, Has<MovingTo>, Has<Busy>),
        Or<(With<Busy>, With<Transformation>, With<MovingTo>)>,
    >,
) {
    for (entity, animating, walking, busy) in &units {
        let active = animating || walking;
        if active && !busy {
            commands.entity(entity).insert(Busy);
        } else if !active && busy {
            commands.entity(entity).remove::<Busy>();
        }
    }
}

/// Drains the queue: validate, apply, project.
///
/// Commands apply in issue order, whole-queue-per-run. A dropped command is a
/// `warn!` with its reason — a drop is an emitter bug, a verb that is not
/// built yet, or input that lost a race with the frame it landed on (a key or
/// click arriving exactly as the mode flipped or a turn passed). All of them
/// deserve a line: the first two are defects, and the third explains itself.
fn apply_commands(
    mut commands: Commands,
    mut queue: ResMut<CommandQueue>,
    mode: Res<State<Mode>>,
    turn_order: Res<TurnOrder>,
    registry: Res<UnitRegistry>,
    settings: Option<Res<PlayerSettings>>,
    table: Option<Res<SubstanceTable>>,
    spells: Option<Res<SpellBook>>,
    content: Option<Res<ContentIndex>>,
    elements: Option<Res<ElementCatalog>>,
    mut stores: ResolutionStores,
    combat: Option<Res<CombatSettings>>,
    blockers: Option<Res<TraversalBlockers>>,
    tiles: TileQuery,
    mut units: UnitStores,
    mut party_stores: PartyStores,
) {
    let mut committed: Vec<Entity> = Vec::new();
    let mut emitted: Vec<CombatEvent> = Vec::new();
    let in_combat = *mode.get() == Mode::Combat;

    while let Some(issued) = queue.pop() {
        if let Some(refusal) = modal_refusal(&stores.pending, &issued.command) {
            drop_command(&mut emitted, &issued, refusal);
            continue;
        }
        let unit = issued.command.unit();
        let Some(entity) = registry.entity_of(unit) else {
            drop_command(&mut emitted, &issued, CommandRefusal::UnknownUnit);
            continue;
        };

        // Seat validation. Units without an owner belong to seat 0 — "the only
        // session there is" — matching how they are spawned; the check grows
        // teeth the moment a second seat exists.
        let owner = units
            .actors
            .get(entity)
            .ok()
            .and_then(|(_, _, _, _, owner, _, _)| owner.copied())
            .unwrap_or_default();
        if owner.0 != issued.seat {
            drop_command(
                &mut emitted,
                &issued,
                CommandRefusal::WrongSeat {
                    issued_by: issued.seat,
                    owned_by: owner.0,
                },
            );
            continue;
        }

        let mut verb = Verb {
            turn_order: &turn_order,
            registry: &registry,
            settings: settings.as_deref(),
            table: table.as_deref(),
            spells: spells.as_deref(),
            tables: content
                .as_deref()
                .zip(elements.as_deref())
                .map(|(index, elements)| index.tables(elements)),
            pending: &mut stores.pending,
            effects: &mut stores.effects,
            knowledge: &mut stores.knowledge,
            spatial: stores.spatial.as_deref(),
            events: &mut emitted,
            revivals: &mut stores.revivals,
            combat: combat.as_deref(),
            party: &party_stores.party,
            formation: &mut party_stores.formation,
            formations: party_stores.formations.as_deref(),
            blockers: blockers.as_deref(),
            committed: &mut committed,
            in_combat,
        };

        let recorded = issued.command.clone();
        let outcome = match issued.command {
            GameCommand::MoveAlong { ref path, .. } => move_along::apply(
                &mut verb,
                &mut commands,
                &tiles,
                &mut units.actors,
                unit,
                entity,
                path,
            ),
            GameCommand::MoveParty { ref paths, .. } => move_party::apply(
                &mut verb,
                &mut commands,
                &tiles,
                &mut units.actors,
                issued.seat,
                unit,
                paths,
            ),
            GameCommand::Strike { target, .. } => strike::apply(
                &mut verb,
                &mut commands,
                &tiles,
                &mut units.actors,
                &mut units.lattices,
                unit,
                entity,
                target,
            ),
            GameCommand::EndTurn { .. } => {
                end_turn::apply(&mut verb, &mut units.actors, unit, entity)
            }
            GameCommand::Cast {
                ref spell,
                target,
                facing,
                ..
            } => cast::apply(
                &mut verb,
                &mut commands,
                &mut units.actors,
                &mut units.lattices,
                unit,
                entity,
                spell,
                target,
                facing,
            ),
            GameCommand::Channel { .. } => Err(CommandRefusal::ChannelUnavailable),
            GameCommand::ChooseDisables { ref cells, .. } => {
                choose_disables::apply(&mut verb, &mut units.lattices, unit, entity, cells)
            }
            GameCommand::ChooseRestores {
                target, ref cells, ..
            } => choose_restores::apply(
                &mut verb,
                &mut commands,
                &mut units.actors,
                &mut units.lattices,
                unit,
                target,
                cells,
            ),
            GameCommand::Rest { .. } => rest::apply(
                &mut verb,
                &mut commands,
                &mut units.lattices,
                &units.lattice_stats,
                unit,
            ),
        };

        if let Err(refusal) = outcome {
            drop_command(&mut emitted, &issued, refusal);
        } else {
            stores.summary.record_command(&recorded);
        }
    }
    stores.events.write_batch(emitted);
}

/// While resolution is waiting on a defender, the answer is the whole command
/// vocabulary. Nothing else may interleave with the lattice state it settles.
fn modal_refusal(pending: &PendingDecision, command: &GameCommand) -> Option<CommandRefusal> {
    let decider = match *pending {
        PendingDecision::None => return None,
        PendingDecision::ChooseDisables { decider, .. }
        | PendingDecision::ChooseRestores { decider, .. } => decider,
    };
    match (pending, command) {
        (PendingDecision::ChooseDisables { .. }, GameCommand::ChooseDisables { unit, .. })
        | (PendingDecision::ChooseRestores { .. }, GameCommand::ChooseRestores { unit, .. })
            if *unit == decider =>
        {
            None
        }
        (PendingDecision::ChooseDisables { .. }, GameCommand::ChooseDisables { .. })
        | (PendingDecision::ChooseRestores { .. }, GameCommand::ChooseRestores { .. }) => {
            Some(CommandRefusal::WrongDecisionUnit { expected: decider })
        }
        _ => Some(CommandRefusal::DecisionPending { decider }),
    }
}

/// Says exactly what was refused and why, once per drop.
fn drop_command(events: &mut Vec<CombatEvent>, issued: &IssuedCommand, refusal: CommandRefusal) {
    warn!("command dropped ({refusal:?}): {issued:?}");
    events.push(CombatEvent::CommandRefused {
        command: issued.command.clone(),
        refusal,
    });
}

#[cfg(test)]
mod tests {
    use hex_core::{LatticeCoord, UnitId};

    use super::*;

    #[test]
    fn a_disable_decision_is_modal_for_every_other_command() {
        let decider = UnitId(4);
        let pending = PendingDecision::ChooseDisables {
            decider,
            count: 1,
            source: UnitId(2),
        };
        let blocked = [
            GameCommand::MoveAlong {
                unit: decider,
                path: vec![TilePos::ORIGIN],
            },
            GameCommand::Strike {
                unit: decider,
                target: UnitId(8),
            },
            GameCommand::EndTurn { unit: decider },
            GameCommand::Cast {
                unit: decider,
                spell: "Ember".to_owned(),
                target: TilePos::ORIGIN,
                facing: None,
                mana: None,
            },
            GameCommand::Channel { unit: decider },
        ];
        for command in &blocked {
            assert_eq!(
                modal_refusal(&pending, command),
                Some(CommandRefusal::DecisionPending { decider }),
                "{command:?} escaped the modal gate"
            );
        }

        assert_eq!(
            modal_refusal(
                &pending,
                &GameCommand::ChooseDisables {
                    unit: UnitId(9),
                    cells: vec![LatticeCoord::ORIGIN],
                }
            ),
            Some(CommandRefusal::WrongDecisionUnit { expected: decider })
        );
        assert_eq!(
            modal_refusal(
                &pending,
                &GameCommand::ChooseDisables {
                    unit: decider,
                    cells: vec![LatticeCoord::ORIGIN],
                }
            ),
            None,
            "the exact matching answer is the one legal command"
        );
    }
}
