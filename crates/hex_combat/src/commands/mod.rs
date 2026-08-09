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

use std::collections::{BTreeMap, BTreeSet};

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use hex_assets::{
    CombatSettings, ContentIndex, ContentTables, ElementCatalog, FormationCatalog, PlayerSettings,
    SpellBook, SubstanceTable,
};
use hex_core::{
    Busy, CommandQueue, ControlOwner, GameCommand, IssuedCommand, Mode, PartyFormation,
    PausableSystems, PendingDecision, Screen, TerrainEdit, TerrainImpact, TerrainImpactOutcome,
    TilePos, TraversalBlockers, Turn, UnitId,
};
use hex_perception::FactionMapKnowledge;
use hex_units::{
    Body, Downed, Faction, MovingTo, Party, StandsOn, TerrainOccupancy, UnitOccupancy, UnitRegistry,
};

use crate::ai::AiIssuedCommand;
use crate::outcomes::{CombatEvent, CommandRefusal};
use crate::turns::TurnOrder;

pub(crate) mod cast;
pub use cast::{delivers_anything, UNDELIVERABLE};
mod channel;
pub use channel::{channel_refusal, ChannelReadiness};
mod choose_disables;
mod choose_restores;
mod end_turn;
mod move_along;
mod move_party;
mod presentation;
mod rest;
mod spell_resolution;
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
    /// Private completion authority for area effects and terrain acknowledgements.
    resolution: &'a mut crate::SpellResolutionState,
    /// Stable names for per-element structured outcomes.
    elements: Option<&'a ElementCatalog>,
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
    /// Exact material occupancy projected from world-published run bounds.
    terrain: Option<&'a TerrainOccupancy>,
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
    /// Live exact surfaces plus every committed route in flight at drain start.
    occupancy: &'a UnitOccupancy,
    /// Units this drain already committed domain movement for. `Busy` lands via
    /// `Commands` and is not queryable until the next sync point, so within one
    /// drain this set is the truth.
    committed: &'a mut Vec<Entity>,
    /// Exact endpoints committed earlier in this same queue drain.
    reserved: &'a mut BTreeMap<UnitId, TilePos>,
    in_combat: bool,
}

/// Mutable resolution stores grouped to stay inside Bevy's system-parameter arity.
#[derive(SystemParam)]
struct ResolutionStores<'w> {
    pending: ResMut<'w, PendingDecision>,
    resolution: ResMut<'w, crate::SpellResolutionState>,
    effects: ResMut<'w, crate::effects::PersistentEffects>,
    knowledge: ResMut<'w, crate::knowledge::FactionLatticeKnowledge>,
    spatial: Option<Res<'w, FactionMapKnowledge>>,
    terrain: Option<Res<'w, TerrainOccupancy>>,
    terrain_edits: MessageWriter<'w, TerrainEdit>,
    terrain_impacts: MessageWriter<'w, TerrainImpact>,
    events: MessageWriter<'w, CombatEvent>,
    revivals: ResMut<'w, crate::turns::PendingRevivals>,
    summary: ResMut<'w, crate::CombatSummary>,
    authority: Option<ResMut<'w, crate::authority_host::CombatAuthority>>,
    rounds: MessageWriter<'w, hex_core::RoundElapsed>,
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
    ai_commands: Query<'w, 's, (), With<AiIssuedCommand>>,
    lattices: cast::LatticeQuery<'w, 's>,
    lattice_stats: Query<'w, 's, &'static hex_lattice::LatticeStats>,
    occupants: Query<
        'w,
        's,
        (
            &'static UnitId,
            &'static StandsOn,
            Option<&'static MovingTo>,
        ),
    >,
}

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<CommandQueue>()
        .init_resource::<Party>()
        .init_resource::<PartyFormation>()
        // A resource rather than a marker component since it carries a payload, so it
        // needs initialising as well as registering. Nothing sets it to anything but
        // `None` until the damage model lands.
        .init_resource::<PendingDecision>()
        .init_resource::<crate::SpellResolutionState>()
        .register_type::<GameCommand>()
        .register_type::<IssuedCommand>()
        .register_type::<Busy>()
        .register_type::<PendingDecision>()
        .add_message::<TerrainEdit>()
        .add_message::<TerrainImpact>()
        .add_message::<TerrainImpactOutcome>();
    spell_resolution::plugin(app);
    app.add_systems(
        Update,
        (apply_commands
            .in_set(crate::CombatSystems::Apply)
            .in_set(PausableSystems)
            .run_if(in_state(Screen::Gameplay)),),
    );
    // Unit ids reset between sessions, so a held-over command would name
    // somebody else's unit next launch.
    app.add_systems(OnExit(Screen::Gameplay), clear_session_state);
}

/// Forgets everything naming a unit, on the way out of a session.
///
/// Both of these outlive the screen — that is what being a resource means, and it is
/// exactly the property a per-entity marker did not have. Unit ids restart each
/// session, so a queued command or an unanswered decision held across one names
/// somebody else's unit next launch: the queue would apply to a stranger, and the
/// decision would park resolution forever on an answer nobody can give.
fn clear_session_state(
    mut queue: ResMut<CommandQueue>,
    mut pending: ResMut<PendingDecision>,
    mut resolution: ResMut<crate::SpellResolutionState>,
) {
    queue.clear();
    *pending = PendingDecision::None;
    resolution.reset();
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
    mut turn_order: ResMut<TurnOrder>,
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
    let mut reserved = BTreeMap::new();
    let mut consumed_ai_issuers = BTreeSet::new();
    let mut emitted: Vec<CombatEvent> = Vec::new();
    let in_combat = *mode.get() == Mode::Combat;
    let occupancy = UnitOccupancy::from_positions(units.occupants.iter().flat_map(
        |(unit, standing, moving)| {
            std::iter::once((*unit, standing.0.pos)).chain(
                moving
                    .into_iter()
                    .flat_map(|moving| moving.path.iter())
                    .map(|step| (*unit, step.pos)),
            )
        },
    ));

    if in_combat {
        if let Some(authority) = stores.authority.as_deref_mut() {
            if authority.adapter_pending() {
                let adopted = adopt_authority_projection(
                    &mut authority.state,
                    &turn_order,
                    &stores.pending,
                    &stores.revivals,
                    &registry,
                    &units.actors,
                    &units.lattices,
                );
                assert!(
                    adopted.is_ok(),
                    "content adapter could not publish its combat projection: {adopted:?}"
                );
                authority.finish_adapter_adoption();
            }
            let projected = project_authority_state(
                &authority.state,
                &mut commands,
                &mut turn_order,
                &mut stores.pending,
                &mut stores.revivals,
                &registry,
                &units.actors,
                &units.lattices,
            );
            assert!(
                projected.is_ok(),
                "combat authority produced an invalid ECS projection: {projected:?}"
            );
            authority.drain_events(&mut emitted);
            let mut elapsed = Vec::new();
            authority.drain_rounds(&mut elapsed);
            stores.rounds.write_batch(elapsed);
        }
    }

    if in_combat
        && matches!(
            stores.resolution.status(),
            crate::SpellResolutionStatus::Pending { .. }
        )
        && !stores.pending.is_open()
        && stores.resolution.has_unit_work()
    {
        let progressed = spell_resolution::pump_unit_work(
            &mut stores.resolution,
            &mut stores.pending,
            &mut stores.effects,
            turn_order.round,
            &registry,
            &units.actors,
            &mut units.lattices,
            &mut emitted,
        );
        if progressed {
            let Some(authority) = stores.authority.as_deref_mut() else {
                stores
                    .resolution
                    .freeze(crate::SpellResolutionFailure::AuthorityUnavailable {
                        reason: "combat authority is absent while advancing unit effects"
                            .to_owned(),
                    });
                stores.events.write_batch(emitted);
                return;
            };
            if !authority.state.external_resolution_is_held() {
                stores
                    .resolution
                    .freeze(crate::SpellResolutionFailure::AuthorityUnavailable {
                        reason: "combat authority lost the spell-resolution hold".to_owned(),
                    });
                stores.events.write_batch(emitted);
                return;
            }
            if let Err(reason) = adopt_authority_projection(
                &mut authority.state,
                &turn_order,
                &stores.pending,
                &stores.revivals,
                &registry,
                &units.actors,
                &units.lattices,
            ) {
                stores
                    .resolution
                    .freeze(crate::SpellResolutionFailure::AuthorityUnavailable { reason });
                stores.events.write_batch(emitted);
                return;
            }
        }
    }

    while let Some(issued) = queue.pop() {
        let unit = issued.command.unit();
        let ai_issuer = registry.entity_of(unit).filter(|entity| {
            units.ai_commands.contains(*entity) && consumed_ai_issuers.insert(*entity)
        });
        if let Some(entity) = ai_issuer {
            commands.entity(entity).remove::<AiIssuedCommand>();
        }
        if matches!(
            stores.resolution.status(),
            crate::SpellResolutionStatus::Frozen(_)
        ) {
            let refusal = CommandRefusal::Busy;
            if let Some(authority) = stores.authority.as_deref_mut() {
                authority
                    .state
                    .record_adapter_refusal(issued.clone(), refusal.clone());
                let mut discarded = Vec::new();
                authority.drain_events(&mut discarded);
            }
            drop_command(&mut emitted, &issued, refusal);
            continue;
        }
        if in_combat && stores.authority.is_none() {
            let refusal = CommandRefusal::MissingCombatData {
                data: crate::CombatData::AuthorityState,
            };
            drop_command(&mut emitted, &issued, refusal);
            continue;
        }
        if in_combat
            && stores
                .authority
                .as_deref()
                .is_some_and(|authority| authority.handles(&issued.command))
        {
            let observed = issued.clone();
            let recorded = issued.command.clone();
            let answered_spell_decision =
                matches!(
                    stores.resolution.status(),
                    crate::SpellResolutionStatus::Pending { .. }
                ) && matches!(&observed.command, GameCommand::ChooseDisables { .. });
            let Some(authority) = stores.authority.as_deref_mut() else {
                drop_command(
                    &mut emitted,
                    &issued,
                    CommandRefusal::MissingCombatData {
                        data: crate::CombatData::AuthorityState,
                    },
                );
                continue;
            };
            let outcome = authority.state.apply(issued);
            if let Err(refusal) = outcome {
                if let Some(authority) = stores.authority.as_deref_mut() {
                    authority.drain_events(&mut emitted);
                }
                warn!("command dropped ({refusal:?}): {observed:?}");
                continue;
            }

            let entity = registry.entity_of(unit);
            assert!(
                entity.is_some(),
                "authority accepted a unit absent from the ECS registry"
            );
            let Some(entity) = entity else {
                continue;
            };
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
                resolution: &mut stores.resolution,
                elements: elements.as_deref(),
                effects: &mut stores.effects,
                knowledge: &mut stores.knowledge,
                spatial: stores.spatial.as_deref(),
                terrain: stores.terrain.as_deref(),
                events: &mut emitted,
                revivals: &mut stores.revivals,
                combat: combat.as_deref(),
                party: &party_stores.party,
                formation: &mut party_stores.formation,
                formations: party_stores.formations.as_deref(),
                blockers: blockers.as_deref(),
                occupancy: &occupancy,
                committed: &mut committed,
                reserved: &mut reserved,
                in_combat,
            };
            let projection = match &observed.command {
                GameCommand::MoveAlong { path, .. } => move_along::project(
                    &mut verb,
                    &mut commands,
                    &tiles,
                    &mut units.actors,
                    unit,
                    entity,
                    path,
                ),
                GameCommand::Strike { target, .. } => {
                    strike::project(&verb, &mut commands, &units.actors, unit, entity, *target)
                }
                _ => Ok(()),
            };
            assert!(
                projection.is_ok(),
                "authority-approved command could not be projected: {projection:?}"
            );
            if let Some(authority) = stores.authority.as_deref_mut() {
                let projected = project_authority_state(
                    &authority.state,
                    &mut commands,
                    &mut turn_order,
                    &mut stores.pending,
                    &mut stores.revivals,
                    &registry,
                    &units.actors,
                    &units.lattices,
                );
                assert!(
                    projected.is_ok(),
                    "combat authority produced an invalid ECS projection: {projected:?}"
                );
                authority.drain_events(&mut emitted);
                let mut elapsed = Vec::new();
                authority.drain_rounds(&mut elapsed);
                stores.rounds.write_batch(elapsed);
            }
            stores.summary.record_command(&recorded);
            if answered_spell_decision {
                // The pure answer may have downed the defender through deferred ECS
                // projection. Let that projection settle before the next snapshotted
                // occupant is inspected or another queued command is considered.
                break;
            }
            continue;
        }

        if let Some(refusal) = modal_refusal(&stores.pending, &stores.resolution, &issued.command) {
            record_adapter_refusal(
                stores.authority.as_deref_mut(),
                in_combat,
                &issued,
                &refusal,
            );
            drop_command(&mut emitted, &issued, refusal);
            continue;
        }
        let Some(entity) = registry.entity_of(unit) else {
            record_adapter_refusal(
                stores.authority.as_deref_mut(),
                in_combat,
                &issued,
                &CommandRefusal::UnknownUnit,
            );
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
            let refusal = CommandRefusal::WrongSeat {
                issued_by: issued.seat,
                owned_by: owner.0,
            };
            record_adapter_refusal(
                stores.authority.as_deref_mut(),
                in_combat,
                &issued,
                &refusal,
            );
            drop_command(&mut emitted, &issued, refusal);
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
            resolution: &mut stores.resolution,
            elements: elements.as_deref(),
            effects: &mut stores.effects,
            knowledge: &mut stores.knowledge,
            spatial: stores.spatial.as_deref(),
            terrain: stores.terrain.as_deref(),
            events: &mut emitted,
            revivals: &mut stores.revivals,
            combat: combat.as_deref(),
            party: &party_stores.party,
            formation: &mut party_stores.formation,
            formations: party_stores.formations.as_deref(),
            blockers: blockers.as_deref(),
            occupancy: &occupancy,
            committed: &mut committed,
            reserved: &mut reserved,
            in_combat,
        };

        let observed = issued.clone();
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
                &mut stores.terrain_edits,
                &mut stores.terrain_impacts,
                &mut commands,
                &tiles,
                &mut units.actors,
                &mut units.lattices,
                unit,
                entity,
                spell,
                target,
                facing,
            ),
            GameCommand::Channel { .. } => channel::apply(
                &mut verb,
                &mut units.actors,
                &mut units.lattices,
                &units.lattice_stats,
                unit,
                entity,
            ),
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
            let end_hidden_blocked_hostile_turn =
                end_ai_turn_after_hidden_blocker(&observed.command, &refusal, ai_issuer.is_some());
            if in_combat {
                if let Some(authority) = stores.authority.as_deref_mut() {
                    if matches!(&observed.command, GameCommand::Cast { .. })
                        && stores.resolution.is_blocking()
                        && !authority.state.external_resolution_is_held()
                    {
                        if let Err(reason) = authority.state.begin_external_resolution() {
                            stores.resolution.freeze(
                                crate::SpellResolutionFailure::AuthorityUnavailable { reason },
                            );
                        }
                    }
                    authority
                        .state
                        .record_adapter_refusal(observed.clone(), refusal.clone());
                    let mut discarded = Vec::new();
                    authority.drain_events(&mut discarded);
                }
            }
            drop_command(&mut emitted, &issued, refusal);
            if end_hidden_blocked_hostile_turn {
                // The AI may not inspect authoritative material hidden from its
                // faction just to predict this refusal. Ending the automated turn
                // is the fail-closed fallback: it reveals no blocker and prevents
                // the same knowledge-valid but authority-blocked cast from parking
                // combat forever.
                queue.push(IssuedCommand {
                    seat: observed.seat,
                    command: GameCommand::EndTurn { unit },
                });
            }
        } else {
            let mut needs_settled_adoption = false;
            let mut authority_ready = true;
            if in_combat {
                if let Some(authority) = stores.authority.as_deref_mut() {
                    if matches!(&observed.command, GameCommand::Cast { .. })
                        && stores.resolution.is_blocking()
                    {
                        if let Err(reason) = authority.state.begin_external_resolution() {
                            stores.resolution.freeze(
                                crate::SpellResolutionFailure::AuthorityUnavailable { reason },
                            );
                            authority_ready = false;
                        }
                    }
                    needs_settled_adoption =
                        matches!(&observed.command, GameCommand::ChooseRestores { .. });
                    if authority_ready && !needs_settled_adoption {
                        let adopted = adopt_authority_projection(
                            &mut authority.state,
                            &turn_order,
                            &stores.pending,
                            &stores.revivals,
                            &registry,
                            &units.actors,
                            &units.lattices,
                        );
                        assert!(
                            adopted.is_ok(),
                            "content adapter could not publish its combat projection: {adopted:?}"
                        );
                    }
                    authority.state.record_adapter_success(observed);
                    if needs_settled_adoption {
                        authority.mark_adapter_pending();
                    }
                }
            }
            stores.summary.record_command(&recorded);
            if needs_settled_adoption || !authority_ready {
                // Restoration removes `Downed` through deferred commands, while a
                // failed external hold must freeze before anything else interleaves.
                // Settle and validate the complete projection before reducing a later
                // queued command against the canonical authority.
                break;
            }
        }
    }
    if in_combat {
        if let Some(authority) = stores.authority.as_deref_mut() {
            authority.state.settle_outcome();
            authority.drain_events(&mut emitted);
            let mut elapsed = Vec::new();
            authority.drain_rounds(&mut elapsed);
            stores.rounds.write_batch(elapsed);
        }
    }
    if let Some(authority) = stores.authority.as_deref() {
        project_authority_reveals(&authority.state, &emitted, &mut stores.knowledge);
    }
    stores.events.write_batch(emitted);
}

fn end_ai_turn_after_hidden_blocker(
    command: &GameCommand,
    refusal: &CommandRefusal,
    issued_by_ai: bool,
) -> bool {
    issued_by_ai
        && matches!(command, GameCommand::Cast { .. })
        && matches!(refusal, CommandRefusal::TrajectoryBlocked { .. })
}

fn project_authority_reveals(
    state: &hex_combat_core::CombatState,
    events: &[CombatEvent],
    knowledge: &mut crate::knowledge::FactionLatticeKnowledge,
) {
    for event in events {
        let CombatEvent::Revealed {
            viewer,
            subject,
            rounds,
            ..
        } = event
        else {
            continue;
        };
        let Some(lattice) = state
            .units
            .get(subject)
            .and_then(|unit| unit.lattice.as_ref())
        else {
            continue;
        };
        let _ = knowledge.reveal(
            *viewer,
            *subject,
            &lattice.spec,
            &lattice.state,
            hex_core::KnowledgeExpiry::Rounds(*rounds),
        );
    }
}

fn record_adapter_refusal(
    authority: Option<&mut crate::authority_host::CombatAuthority>,
    in_combat: bool,
    issued: &IssuedCommand,
    refusal: &CommandRefusal,
) {
    if !in_combat
        || authority
            .as_deref()
            .is_some_and(|authority| authority.handles(&issued.command))
    {
        return;
    }
    if let Some(authority) = authority {
        authority
            .state
            .record_adapter_refusal(issued.clone(), refusal.clone());
        let mut discarded = Vec::new();
        authority.drain_events(&mut discarded);
    }
}

fn project_authority_state(
    state: &hex_combat_core::CombatState,
    commands: &mut Commands,
    order: &mut TurnOrder,
    pending: &mut PendingDecision,
    revivals: &mut crate::turns::PendingRevivals,
    registry: &UnitRegistry,
    actors: &ActorQuery,
    lattices: &cast::LatticeQuery,
) -> Result<(), String> {
    // Validate the complete projection before mutating any ECS fact or enqueueing a
    // deferred component write. A late missing unit/lattice must leave both sides of
    // the authority seam at the pre-release checkpoint.
    let mut targets = Vec::with_capacity(state.units.len());
    let mut claimed_entities = BTreeMap::new();
    for actor in state.units.values() {
        let entity = registry
            .entity_of(actor.id)
            .ok_or_else(|| format!("authority unit {:?} has no ECS entity", actor.id))?;
        if let Some(existing) = claimed_entities.insert(entity, actor.id) {
            return Err(format!(
                "authority units {existing:?} and {:?} share one ECS entity",
                actor.id,
            ));
        }
        let (standing, _, turn, busy, _, _, downed) = actors.get(entity).map_err(|error| {
            format!("authority unit {:?} is not projectable: {error}", actor.id)
        })?;
        if standing.map(|standing| standing.0.pos) != Some(actor.position) {
            return Err(format!(
                "authority position {:?} disagrees with {:?}",
                actor.position,
                standing.map(|standing| standing.0.pos)
            ));
        }
        let lattice_changed = match (&actor.lattice, lattices.get(entity).ok()) {
            (Some(expected), Some((spec, current))) if expected.spec == *spec => {
                expected.state != *current
            }
            (None, None) => false,
            _ => {
                return Err(format!(
                    "authority lattice shape for {:?} cannot be projected",
                    actor.id
                ));
            }
        };
        targets.push((entity, turn.copied(), busy, downed, lattice_changed));
    }

    order.project(&state.order, state.current(), state.round);
    *pending = state.pending.clone();
    revivals.project(&state.pending_revivals);
    for (actor, (entity, turn, busy, downed, lattice_changed)) in state.units.values().zip(targets)
    {
        match (actor.turn, turn) {
            (Some(expected), Some(current)) if expected == current => {}
            (Some(expected), _) => {
                commands.entity(entity).insert(expected);
            }
            (None, Some(_)) => {
                commands.entity(entity).remove::<Turn>();
            }
            (None, None) => {}
        }
        if actor.busy != busy {
            if actor.busy {
                commands.entity(entity).insert(Busy);
            } else {
                commands.entity(entity).remove::<Busy>();
            }
        }
        if actor.downed != downed {
            if actor.downed {
                commands.entity(entity).insert(Downed);
            } else {
                commands.entity(entity).remove::<Downed>();
            }
        }
        // Do not enqueue an already-matching mutable projection. An adapter command
        // may commit a newer turn or lattice later in this system; a stale deferred
        // insertion from the pre-command checkpoint must not overwrite that commit.
        if lattice_changed {
            if let Some(expected) = &actor.lattice {
                commands.entity(entity).insert(expected.state.clone());
            }
        }
    }
    Ok(())
}

fn adopt_authority_projection(
    state: &mut hex_combat_core::CombatState,
    order: &TurnOrder,
    pending: &PendingDecision,
    revivals: &crate::turns::PendingRevivals,
    registry: &UnitRegistry,
    actors: &ActorQuery,
    lattices: &cast::LatticeQuery,
) -> Result<(), String> {
    let mut projection = Vec::with_capacity(state.units.len());
    for actor in state.units.values() {
        let entity = registry
            .entity_of(actor.id)
            .ok_or_else(|| format!("authority unit {:?} has no ECS entity", actor.id))?;
        let (standing, _, turn, busy, _, _, downed) = actors
            .get(entity)
            .map_err(|error| format!("adapter unit {:?} is unavailable: {error}", actor.id))?;
        let position = standing
            .map(|standing| standing.0.pos)
            .ok_or_else(|| format!("adapter unit {:?} has no exact position", actor.id))?;
        let lattice = lattices
            .get(entity)
            .ok()
            .map(|(_, lattice)| lattice.clone());
        projection.push(hex_combat_core::CombatUnitProjection {
            id: actor.id,
            position,
            turn: turn.copied(),
            busy,
            downed,
            lattice,
        });
    }
    state.adopt_projection(
        order.order().to_vec(),
        order.current(),
        order.round,
        pending.clone(),
        revivals.snapshot(),
        projection,
    )
}

fn current_occupancy(base: &UnitOccupancy, reserved: &BTreeMap<UnitId, TilePos>) -> UnitOccupancy {
    let mut occupancy = base.clone();
    for (&unit, &destination) in reserved {
        occupancy.relocate(unit, destination);
    }
    occupancy
}

/// While resolution is waiting on a defender, the answer is the whole command
/// vocabulary. Nothing else may interleave with the lattice state it settles.
fn modal_refusal(
    pending: &PendingDecision,
    resolution: &crate::SpellResolutionState,
    command: &GameCommand,
) -> Option<CommandRefusal> {
    if matches!(resolution.status(), crate::SpellResolutionStatus::Frozen(_))
        || (resolution.is_blocking() && !pending.is_open())
    {
        return Some(CommandRefusal::Busy);
    }
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
                modal_refusal(&pending, &crate::SpellResolutionState::default(), command),
                Some(CommandRefusal::DecisionPending { decider }),
                "{command:?} escaped the modal gate"
            );
        }

        assert_eq!(
            modal_refusal(
                &pending,
                &crate::SpellResolutionState::default(),
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
                &crate::SpellResolutionState::default(),
                &GameCommand::ChooseDisables {
                    unit: decider,
                    cells: vec![LatticeCoord::ORIGIN],
                }
            ),
            None,
            "the exact matching answer is the one legal command"
        );
    }

    #[test]
    fn an_external_spell_transaction_blocks_ordinary_commands_between_answers() {
        let mut resolution = crate::SpellResolutionState::default();
        resolution
            .begin(
                UnitId(2),
                vec![crate::spell_resolution::UnitResolution::Burn {
                    source: UnitId(2),
                    target: UnitId(5),
                    turns: 1,
                }],
                Vec::new(),
            )
            .expect("fixture transaction starts");
        let command = GameCommand::EndTurn { unit: UnitId(2) };
        assert_eq!(
            modal_refusal(&PendingDecision::None, &resolution, &command),
            Some(CommandRefusal::Busy)
        );

        let pending = PendingDecision::ChooseDisables {
            decider: UnitId(5),
            count: 1,
            source: UnitId(2),
        };
        let answer = GameCommand::ChooseDisables {
            unit: UnitId(5),
            cells: vec![LatticeCoord::ORIGIN],
        };
        assert_eq!(modal_refusal(&pending, &resolution, &answer), None);
    }

    #[test]
    fn hidden_blocker_fallback_applies_only_to_ai_issued_casts() {
        let cast = GameCommand::Cast {
            unit: UnitId(7),
            spell: "Ember".to_owned(),
            target: TilePos::ORIGIN,
            facing: None,
            mana: None,
        };
        let blocked = CommandRefusal::TrajectoryBlocked {
            spell: "Ember".to_owned(),
        };

        assert!(end_ai_turn_after_hidden_blocker(&cast, &blocked, true));
        assert!(!end_ai_turn_after_hidden_blocker(&cast, &blocked, false));
        assert!(!end_ai_turn_after_hidden_blocker(
            &GameCommand::EndTurn { unit: UnitId(7) },
            &blocked,
            true,
        ));
    }
}
