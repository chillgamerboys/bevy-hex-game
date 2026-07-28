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
//! That shape is load-bearing rather than tidiness: wave 3 adds casting,
//! terrain impact and persistent effects, and each becomes **a new file plus
//! one match arm** instead of another hundred lines inside one function. A verb
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

use bevy::prelude::*;

use hex_anim::Transformation;
use hex_assets::{
    CombatSettings, ContentIndex, ContentTables, ElementCatalog, PlayerSettings, SpellBook,
    SubstanceTable,
};
use hex_core::{
    AppSystems, Busy, CommandQueue, ControlOwner, GameCommand, IssuedCommand, Mode,
    PausableSystems, PendingDecision, Screen, TilePos, Turn,
};
use hex_units::{Body, Faction, MovingTo, StandsOn, UnitRegistry};

use crate::turns::TurnOrder;

mod cast;
mod choose_disables;
mod end_turn;
mod move_along;
mod presentation;
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
    /// The field this struct's docs promised: persistent effects were named as one of
    /// wave 3's additions, and casting a burn is a verb needing a fact the handlers
    /// lacked. One field here rather than a ninth argument on `cast::apply`.
    effects: &'a mut crate::effects::PersistentEffects,
    /// Policy knobs: budgets, ranges, and what a strike costs.
    combat: Option<&'a CombatSettings>,
    /// Units this drain already committed presentation for. `Busy` lands via
    /// `Commands` and is not queryable until the next sync point, so within one
    /// drain this set is the truth.
    committed: &'a mut Vec<Entity>,
    in_combat: bool,
}

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<CommandQueue>()
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
    // A decision open when a fight ends has nobody left to answer it: the auto-policy
    // runs only in combat, so it would park every later cast behind "a decision is
    // still open" for the rest of the session.
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
    mut pending: ResMut<PendingDecision>,
    mut effects: ResMut<crate::effects::PersistentEffects>,
    combat: Option<Res<CombatSettings>>,
    tiles: TileQuery,
    mut actors: ActorQuery,
    mut lattices: cast::LatticeQuery,
) {
    let mut committed: Vec<Entity> = Vec::new();
    let in_combat = *mode.get() == Mode::Combat;

    while let Some(issued) = queue.pop() {
        let unit = issued.command.unit();
        let Some(entity) = registry.entity_of(unit) else {
            drop_command(&issued, "no such unit");
            continue;
        };

        // Seat validation. Units without an owner belong to seat 0 — "the only
        // session there is" — matching how they are spawned; the check grows
        // teeth the moment a second seat exists.
        let owner = actors
            .get(entity)
            .ok()
            .and_then(|(_, _, _, _, owner, _)| owner.copied())
            .unwrap_or_default();
        if owner.0 != issued.seat {
            drop_command(&issued, "issued by a seat that does not own the unit");
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
            pending: &mut pending,
            effects: &mut effects,
            combat: combat.as_deref(),
            committed: &mut committed,
            in_combat,
        };

        let outcome = match issued.command {
            GameCommand::MoveAlong { ref path, .. } => move_along::apply(
                &mut verb,
                &mut commands,
                &tiles,
                &mut actors,
                unit,
                entity,
                path,
            ),
            GameCommand::Strike { target, .. } => strike::apply(
                &mut verb,
                &mut commands,
                &tiles,
                &mut actors,
                &mut lattices,
                unit,
                entity,
                target,
            ),
            GameCommand::EndTurn { .. } => end_turn::apply(&mut verb, &mut actors, unit, entity),
            GameCommand::Cast {
                ref spell,
                target,
                facing,
                ..
            } => cast::apply(
                &mut verb,
                &mut commands,
                &mut actors,
                &mut lattices,
                unit,
                entity,
                spell,
                target,
                facing,
            ),
            GameCommand::Channel { .. } => {
                Err("channelling waits on the initiative question being settled")
            }
            GameCommand::ChooseDisables { ref cells, .. } => {
                choose_disables::apply(&mut verb, &mut lattices, unit, entity, cells)
            }
        };

        if let Err(reason) = outcome {
            drop_command(&issued, reason);
        }
    }
}

/// Says exactly what was refused and why, once per drop.
fn drop_command(issued: &IssuedCommand, reason: &str) {
    warn!("command dropped ({reason}): {issued:?}");
}
