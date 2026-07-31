//! Algorithm-neutral enemy decision host.
//!
//! Combat owns legality. Algorithms receive an authorized observation and opaque keys
//! for the complete legal command set; their only authority is choosing one key. The
//! selected command then travels through the same queue and applier as player input.

use std::collections::{BTreeMap, BTreeSet};

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use hex_ai::{
    ActionKey, AiAlgorithm, AiAlgorithmId, AiAlliedUnit, AiCellKind, AiController,
    AiDecisionFailure, AiDecisionKind, AiDecisionTrace, AiEffectObservation, AiLatticeCell,
    AiLatticeObservation, AiObservation, AiObservedHostile, AiProfileId, AiSelection,
    AiSpellObservation, AiTraversalObservation, BaselineAlgorithm, CellChoiceFingerprint,
    CellChoiceSet, DecisionRequest, LegalActionFingerprint, LegalActionSet,
};
use hex_assets::{
    AiProfileCatalog, CastingAxis, CombatSettings, ContentIndex, Effect, ElementCatalog, SpellBook,
    SubstanceTable, TargetShape, Trajectory,
};
use hex_core::{
    Busy, CommandQueue, ControlOwner, GameCommand, GameplaySetup, GameplaySetupFailure,
    IssuedCommand, KnowledgeState, LatticeCoord, Mode, PausableSystems, PendingDecision, Screen,
    Sextant, TilePos, Turn, UnitId,
};
use hex_lattice::{castable, CellKind, LatticeSpec, LatticeState, LatticeStats};
use hex_perception::{FactionKnowledge, FactionMapKnowledge, SurfaceSnapshot};
use hex_units::{
    targeting, trajectory_destination, trajectory_is_clear, volumes, Body, Downed, Enemy, Faction,
    Footing, Player, Reach, StandsOn, TerrainOccupancy, UnitOccupancy, UnitRegistry,
};
use xxhash_rust::xxh3::xxh3_64;

use crate::{delivers_anything, FactionLatticeKnowledge, PersistentEffects, TurnOrder};

/// Registered mutable algorithm instances, scoped to one gameplay session.
#[derive(Resource)]
pub struct AiAlgorithmRegistry {
    algorithms: BTreeMap<AiAlgorithmId, Box<dyn AiAlgorithm>>,
}

impl Default for AiAlgorithmRegistry {
    fn default() -> Self {
        let mut registry = Self {
            algorithms: BTreeMap::new(),
        };
        registry.register(AiAlgorithmId("baseline-v1".to_owned()), BaselineAlgorithm);
        registry
    }
}

impl AiAlgorithmRegistry {
    /// Registers or replaces one implementation under a stable content id.
    pub fn register(
        &mut self,
        id: AiAlgorithmId,
        algorithm: impl AiAlgorithm,
    ) -> Option<Box<dyn AiAlgorithm>> {
        self.algorithms.insert(id, Box::new(algorithm))
    }

    fn get_mut(&mut self, id: &AiAlgorithmId) -> Option<&mut Box<dyn AiAlgorithm>> {
        self.algorithms.get_mut(id)
    }

    fn contains(&self, id: &AiAlgorithmId) -> bool {
        self.algorithms.contains_key(id)
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Decision traces retained for developer inspection during the current session.
#[derive(Resource, Debug, Default)]
pub struct AiDecisionTraces {
    /// Dispatches in command order.
    pub entries: Vec<AiDecisionTrace>,
    next_sequence: u64,
}

/// Maximum exact decision snapshots retained for live developer inspection.
pub const MAX_AI_DECISION_TRACES: usize = 64;

impl AiDecisionTraces {
    fn record(&mut self, mut trace: AiDecisionTrace) {
        trace.sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        while self.entries.len() >= MAX_AI_DECISION_TRACES {
            self.entries.remove(0);
        }
        self.entries.push(trace);
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.next_sequence = 0;
    }
}

type UnitQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static UnitId,
        &'static StandsOn,
        &'static Body,
        &'static Faction,
        Option<&'static Turn>,
        Has<Downed>,
        Has<Busy>,
        Option<&'static ControlOwner>,
        Option<&'static AiController>,
        Option<&'static LatticeSpec>,
        Option<&'static LatticeState>,
        Option<&'static LatticeStats>,
        Has<Enemy>,
    ),
>;

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<AiAlgorithmRegistry>()
        .init_resource::<AiDecisionTraces>()
        .add_systems(
            Update,
            (drive_ai, answer_headless_disable)
                .chain()
                .in_set(crate::CombatSystems::Act)
                .in_set(PausableSystems)
                .run_if(in_state(Mode::Combat)),
        )
        .add_systems(
            OnEnter(Screen::Gameplay),
            validate_controller_references.in_set(GameplaySetup::Perception),
        )
        .add_systems(OnExit(Screen::Gameplay), clear_session);
}

fn validate_controller_references(
    mut commands: Commands,
    profiles: Option<Res<AiProfileCatalog>>,
    algorithms: Res<AiAlgorithmRegistry>,
    controllers: Query<&AiController>,
) {
    let Some(profiles) = profiles else {
        return;
    };
    for controller in &controllers {
        let Some(profile) = profiles.get(&controller.profile) else {
            commands.insert_resource(GameplaySetupFailure::new(format!(
                "AI controller references missing profile {:?}.",
                controller.profile.0
            )));
            return;
        };
        if !algorithms.contains(&profile.algorithm) {
            commands.insert_resource(GameplaySetupFailure::new(format!(
                "AI profile {:?} references unregistered algorithm {:?}.",
                profile.id.0, profile.algorithm.0
            )));
            return;
        }
    }
}

/// Keeps lattice-only headless harnesses on the same command seam.
///
/// Runtime units always have the spatial/controller components consumed by
/// [`drive_ai`]. Focused damage tests intentionally do not; this fallback applies the
/// baseline's documented cell order only when the host could not enqueue an answer.
fn answer_headless_disable(
    pending: Res<PendingDecision>,
    registry: Res<UnitRegistry>,
    mut queue: ResMut<CommandQueue>,
    lattices: Query<(&LatticeSpec, &LatticeState, Has<Player>, &ControlOwner)>,
) {
    let PendingDecision::ChooseDisables { decider, count, .. } = *pending else {
        return;
    };
    if queue.holds_answer_for(decider) {
        return;
    }
    let Some(entity) = registry.entity_of(decider) else {
        return;
    };
    let Ok((spec, state, human, owner)) = lattices.get(entity) else {
        return;
    };
    if human {
        return;
    }
    let mut cells: Vec<(u8, u16, LatticeCoord)> = spec
        .cells()
        .filter(|(coord, _)| !state.is_disabled(*coord))
        .map(|(coord, kind)| {
            let rank = match kind {
                CellKind::Blank => 0,
                CellKind::Gem { .. } => 1,
                CellKind::Fusion { .. } => 2,
                CellKind::Spell { .. } => 3,
            };
            (rank, state.mana(coord), coord)
        })
        .collect();
    cells.sort_unstable();
    queue.push(IssuedCommand {
        seat: owner.0,
        command: GameCommand::ChooseDisables {
            unit: decider,
            cells: cells
                .into_iter()
                .take(usize::from(count))
                .map(|(_, _, coord)| coord)
                .collect(),
        },
    });
}

fn clear_session(
    mut algorithms: ResMut<AiAlgorithmRegistry>,
    mut traces: ResMut<AiDecisionTraces>,
) {
    algorithms.reset();
    traces.clear();
}

#[derive(SystemParam)]
struct AiWorld<'w> {
    profiles: Option<Res<'w, AiProfileCatalog>>,
    table: Option<Res<'w, SubstanceTable>>,
    spells: Option<Res<'w, SpellBook>>,
    content: Option<Res<'w, ContentIndex>>,
    elements: Option<Res<'w, ElementCatalog>>,
    combat: Option<Res<'w, CombatSettings>>,
    spatial: Option<Res<'w, FactionMapKnowledge>>,
    terrain: Option<Res<'w, TerrainOccupancy>>,
    knowledge: Res<'w, FactionLatticeKnowledge>,
    effects: Res<'w, PersistentEffects>,
}

fn drive_ai(
    turn_order: Res<TurnOrder>,
    unit_registry: Res<UnitRegistry>,
    pending: Res<PendingDecision>,
    mut queue: ResMut<CommandQueue>,
    mut algorithms: ResMut<AiAlgorithmRegistry>,
    mut traces: ResMut<AiDecisionTraces>,
    world: AiWorld,
    units: UnitQuery,
) {
    let Some(table) = world.table.as_deref() else {
        return;
    };
    let decision = match *pending {
        PendingDecision::ChooseDisables { decider, count, .. } => {
            Some((decider, AiDecisionKind::ChooseDisables, count, None))
        }
        PendingDecision::ChooseRestores {
            decider,
            target,
            count,
        } => Some((decider, AiDecisionKind::ChooseRestores, count, Some(target))),
        PendingDecision::None => turn_order
            .current()
            .map(|unit| (unit, AiDecisionKind::TurnAction, 0, None)),
    };
    let Some((actor_id, kind, count, restoration_target)) = decision else {
        return;
    };
    if queue.holds_command_for(actor_id) {
        return;
    }
    let Some(actor_entity) = unit_registry.entity_of(actor_id) else {
        return;
    };
    let Ok(actor) = units.get(actor_entity) else {
        return;
    };
    if !actor.13 || actor.7 {
        return;
    }
    if kind == AiDecisionKind::TurnAction && actor.5.is_none() {
        return;
    }

    let fallback_controller = AiController {
        profile: AiProfileId("baseline".to_owned()),
        group: None,
    };
    let controller = actor.9.unwrap_or(&fallback_controller);
    let algorithm_id = configured_algorithm(controller, world.profiles.as_deref());
    if !algorithms.contains(&algorithm_id) {
        warn!(
            "AI profile {:?} references unregistered algorithm {:?}; using baseline-v1",
            controller.profile.0, algorithm_id.0
        );
    }
    let dispatched_id = if algorithms.contains(&algorithm_id) {
        algorithm_id
    } else {
        AiAlgorithmId("baseline-v1".to_owned())
    };

    let spatial = world
        .spatial
        .as_deref()
        .map(|knowledge| knowledge.faction(*actor.4));
    let footing = spatial.map(|knowledge| authorized_footing(knowledge, table, *actor.3));
    let authorized = spatial
        .map(|knowledge| authorized_unit_ids(actor, &units, knowledge))
        .unwrap_or_default();
    let occupancy = UnitOccupancy::from_positions(
        units
            .iter()
            .filter(|unit| authorized.contains(unit.1))
            .map(|unit| (*unit.1, unit.2 .0.pos)),
    );
    let reach = footing
        .as_ref()
        .map(|footing| Reach::with_occupancy(actor.2 .0, footing, None, &occupancy, *actor.1));
    let (commands, cell_choices) = match kind {
        AiDecisionKind::TurnAction => {
            match (spatial, footing.as_ref(), reach.as_ref()) {
                (Some(spatial), Some(footing), Some(reach)) if spatial.unit(actor_id).is_some() => {
                    (
                        enumerate_turn_actions(
                            actor,
                            &units,
                            footing,
                            reach,
                            spatial,
                            world.spells.as_deref(),
                            world.content.as_deref(),
                            world.elements.as_deref(),
                            world.combat.as_deref(),
                            world.terrain.as_deref(),
                            table,
                        ),
                        None,
                    )
                }
                // Ending the turn is the only spatially neutral action. Keeping it
                // available avoids a deadlock while failing closed on movement,
                // strikes, casts, identities, and terrain when perception is absent.
                _ => (vec![GameCommand::EndTurn { unit: actor_id }], None),
            }
        }
        AiDecisionKind::ChooseDisables => (
            Vec::new(),
            cell_choice_set(
                actor_id,
                actor_id,
                actor.10,
                actor.11,
                count,
                AiDecisionKind::ChooseDisables,
            ),
        ),
        AiDecisionKind::ChooseRestores => (
            Vec::new(),
            restoration_target
                .and_then(|target| {
                    unit_registry
                        .entity_of(target)
                        .map(|entity| (target, entity))
                })
                .and_then(|(target, entity)| units.get(entity).ok().map(|unit| (target, unit)))
                .and_then(|(target, target_unit)| {
                    cell_choice_set(
                        actor_id,
                        target,
                        target_unit.10,
                        target_unit.11,
                        count,
                        AiDecisionKind::ChooseRestores,
                    )
                }),
        ),
    };
    if commands.is_empty() && cell_choices.is_none() {
        warn!("AI {actor_id:?} has no legal command for {kind:?}");
        return;
    }
    let legal_actions = legal_action_set(commands);
    let observation = build_observation(
        actor,
        &units,
        &turn_order,
        &world.knowledge,
        &world.effects,
        spatial,
        footing.as_ref(),
        reach.as_ref(),
        world.spells.as_deref(),
        world.content.as_deref(),
        world.elements.as_deref(),
    );
    let request = DecisionRequest {
        controller: actor.8.copied().unwrap_or_default().0,
        group: controller.group.clone(),
        kind,
        observation,
        legal_actions,
        cell_choices,
    };
    let selected = algorithms
        .get_mut(&dispatched_id)
        .map(|algorithm| algorithm.select(&request))
        .unwrap_or_else(|| {
            AiSelection::Action(ActionKey::from_parts(
                request.legal_actions.fingerprint(),
                u32::MAX,
            ))
        });
    let (command, failure) = match resolve_selection(&request, &selected) {
        Ok(command) => (Some(command), None),
        Err(failure) => (fallback_command(&request), Some(failure)),
    };
    traces.record(AiDecisionTrace {
        sequence: 0,
        profile: controller.profile.clone(),
        algorithm: dispatched_id,
        actor: actor_id,
        group: controller.group.clone(),
        kind,
        observation: request.observation.clone(),
        legal_actions: request.legal_actions.clone(),
        fingerprint: request.legal_actions.fingerprint(),
        cell_fingerprint: request
            .cell_choices
            .as_ref()
            .map(CellChoiceSet::fingerprint),
        selected,
        command: command.clone(),
        failure,
    });
    if let Some(failure) = failure {
        warn!("AI {actor_id:?} returned an invalid selection: {failure:?}");
    }
    if let Some(command) = command {
        queue.push(IssuedCommand {
            seat: request.controller,
            command,
        });
    }
}

fn configured_algorithm(
    controller: &AiController,
    profiles: Option<&AiProfileCatalog>,
) -> AiAlgorithmId {
    profiles
        .and_then(|catalog| catalog.get(&controller.profile))
        .map_or_else(
            || AiAlgorithmId("baseline-v1".to_owned()),
            |profile| profile.algorithm.clone(),
        )
}

fn authorized_footing(knowledge: &FactionKnowledge, table: &SubstanceTable, body: Body) -> Footing {
    let surfaces: Vec<SurfaceSnapshot> = knowledge
        .surfaces()
        .map(|(_, known)| known.snapshot())
        .filter(|surface| !surface.blocked)
        .collect();
    Footing::from_tiles(
        surfaces.iter().map(|surface| {
            (
                &surface.pos,
                &surface.span,
                &surface.substance,
                &surface.headroom,
            )
        }),
        table,
        body,
        None,
    )
}

fn authorized_unit_ids(
    actor: (
        Entity,
        &UnitId,
        &StandsOn,
        &Body,
        &Faction,
        Option<&Turn>,
        bool,
        bool,
        Option<&ControlOwner>,
        Option<&AiController>,
        Option<&LatticeSpec>,
        Option<&LatticeState>,
        Option<&LatticeStats>,
        bool,
    ),
    units: &UnitQuery,
    spatial: &FactionKnowledge,
) -> BTreeSet<UnitId> {
    let observed_hostiles = spatial
        .units()
        .filter(|(_, unit)| actor.4.is_hostile_to(unit.faction))
        .map(|(id, _)| id);
    units
        .iter()
        .filter(|unit| *unit.4 == *actor.4)
        .map(|unit| *unit.1)
        .chain(observed_hostiles)
        .collect()
}

fn resolve_selection(
    request: &DecisionRequest,
    selected: &AiSelection,
) -> Result<GameCommand, AiDecisionFailure> {
    match (request.kind, selected) {
        (AiDecisionKind::TurnAction, AiSelection::Action(key)) => {
            if key.fingerprint() != request.legal_actions.fingerprint() {
                return Err(AiDecisionFailure::StaleFingerprint);
            }
            request
                .legal_actions
                .resolve(*key)
                .map(|action| action.command.clone())
                .ok_or(AiDecisionFailure::UnknownAction)
        }
        (
            AiDecisionKind::ChooseDisables | AiDecisionKind::ChooseRestores,
            AiSelection::Cells(selection),
        ) => {
            let choices = request
                .cell_choices
                .as_ref()
                .ok_or(AiDecisionFailure::WrongSelectionKind)?;
            choices.validate(selection)?;
            Ok(match request.kind {
                AiDecisionKind::ChooseDisables => GameCommand::ChooseDisables {
                    unit: request.observation.actor.unit,
                    cells: selection.cells.clone(),
                },
                AiDecisionKind::ChooseRestores => GameCommand::ChooseRestores {
                    unit: request.observation.actor.unit,
                    target: choices.subject(),
                    cells: selection.cells.clone(),
                },
                AiDecisionKind::TurnAction => unreachable!("covered by the outer match"),
            })
        }
        _ => Err(AiDecisionFailure::WrongSelectionKind),
    }
}

fn fallback_command(request: &DecisionRequest) -> Option<GameCommand> {
    match request.kind {
        AiDecisionKind::TurnAction => request
            .legal_actions
            .actions()
            .iter()
            .find(|action| matches!(action.command, GameCommand::EndTurn { .. }))
            .or_else(|| request.legal_actions.actions().first())
            .map(|action| action.command.clone()),
        AiDecisionKind::ChooseDisables | AiDecisionKind::ChooseRestores => {
            let choices = request.cell_choices.as_ref()?;
            let cells = choices
                .eligible()
                .iter()
                .copied()
                .take(usize::from(choices.count()))
                .collect();
            Some(match request.kind {
                AiDecisionKind::ChooseDisables => GameCommand::ChooseDisables {
                    unit: request.observation.actor.unit,
                    cells,
                },
                AiDecisionKind::ChooseRestores => GameCommand::ChooseRestores {
                    unit: request.observation.actor.unit,
                    target: choices.subject(),
                    cells,
                },
                AiDecisionKind::TurnAction => unreachable!("covered by the outer match"),
            })
        }
    }
}

fn enumerate_turn_actions(
    actor: (
        Entity,
        &UnitId,
        &StandsOn,
        &Body,
        &Faction,
        Option<&Turn>,
        bool,
        bool,
        Option<&ControlOwner>,
        Option<&AiController>,
        Option<&LatticeSpec>,
        Option<&LatticeState>,
        Option<&LatticeStats>,
        bool,
    ),
    units: &UnitQuery,
    footing: &Footing,
    reach: &Reach,
    spatial: &FactionKnowledge,
    spells: Option<&SpellBook>,
    content: Option<&ContentIndex>,
    elements: Option<&ElementCatalog>,
    combat: Option<&CombatSettings>,
    terrain: Option<&TerrainOccupancy>,
    substances: &SubstanceTable,
) -> Vec<GameCommand> {
    let id = *actor.1;
    let Some(turn) = actor.5 else {
        return Vec::new();
    };
    let observed_hostiles: BTreeSet<UnitId> = spatial
        .units()
        .filter(|(_, unit)| actor.4.is_hostile_to(unit.faction))
        .filter_map(|(id, _)| {
            units
                .iter()
                .any(|candidate| *candidate.1 == id)
                .then_some(id)
        })
        .collect();
    let live_hostiles: BTreeSet<UnitId> = observed_hostiles
        .iter()
        .copied()
        .filter(|id| {
            units
                .iter()
                .find(|candidate| *candidate.1 == *id)
                .is_some_and(|candidate| !candidate.6)
        })
        .collect();
    let authorized_units: BTreeSet<UnitId> = units
        .iter()
        .filter(|unit| *unit.4 == *actor.4)
        .map(|unit| *unit.1)
        .chain(observed_hostiles.iter().copied())
        .collect();
    let mut commands = vec![GameCommand::EndTurn { unit: id }];
    if turn.acted {
        return commands;
    }
    if !actor.6 && actor.10.is_some() && actor.11.is_some() && actor.12.is_some() {
        commands.push(GameCommand::Channel { unit: id });
    }

    if turn.movement_left > 0 {
        for surface in reach.surfaces() {
            if surface.pos == actor.2 .0.pos
                || reach
                    .cost(surface.pos)
                    .is_none_or(|cost| cost > turn.movement_left)
            {
                continue;
            }
            if let Some(path) = reach.path_to(surface.pos) {
                commands.push(GameCommand::MoveAlong {
                    unit: id,
                    path: path.into_iter().map(|step| step.pos).collect(),
                });
            }
        }
    }

    for unit in units
        .iter()
        .filter(|unit| live_hostiles.contains(unit.1) && unit.0 != actor.0)
    {
        if footing.admits_step(actor.2 .0.pos, unit.2 .0.pos)
            && footing.admits_step(unit.2 .0.pos, actor.2 .0.pos)
        {
            commands.push(GameCommand::Strike {
                unit: id,
                target: *unit.1,
            });
        }
    }

    if let (Some(spec), Some(state), Some(book), Some(index), Some(elements)) =
        (actor.10, actor.11, spells, content, elements)
    {
        let tables = index.tables(elements);
        let anchors: Vec<TilePos> = spatial
            .surfaces()
            .filter(|(_, known)| known.state() == KnowledgeState::Observed)
            .map(|(position, _)| position)
            .collect();
        for (spell_id, name, spell) in book.iter() {
            if !delivers_anything(spell) {
                continue;
            }
            if !crate::commands::cast::terrain_creation_is_admitted(spell, substances) {
                continue;
            }
            let Some(cell) = crate::commands::cast::spell_cell(spec, state, spell_id) else {
                continue;
            };
            if castable(spec, state, cell, &tables).is_err() {
                continue;
            }
            let spell_anchors: Vec<TilePos> =
                if matches!(spell.targeting.shape, TargetShape::SelfCast) {
                    vec![actor.2 .0.pos]
                } else {
                    anchors
                        .iter()
                        .copied()
                        .filter(|target| {
                            targeting::in_reach(
                                actor.2 .0.pos,
                                *target,
                                u32::from(spell.targeting.range),
                                combat.map_or(5, |settings| settings.levels_per_bonus_range),
                            )
                        })
                        .collect()
                };
            let facings: &[Option<Sextant>] = if volumes::needs_facing(&spell.targeting.shape) {
                &[
                    Some(Sextant::A),
                    Some(Sextant::B),
                    Some(Sextant::C),
                    Some(Sextant::D),
                    Some(Sextant::E),
                    Some(Sextant::F),
                ]
            } else {
                &[None]
            };
            for target in spell_anchors {
                if !trajectory_available(
                    spell.targeting.trajectory,
                    actor.2 .0.pos,
                    target,
                    spell.effects.iter().any(|effect| {
                        matches!(effect, Effect::SetTerrain { .. } | Effect::SpawnWall { .. })
                    }),
                    terrain,
                ) {
                    continue;
                }
                if damages_downed(spell, target, units, &authorized_units) {
                    continue;
                }
                for &facing in facings {
                    if volumes::resolve(&spell.targeting.shape, actor.2 .0.pos, target, facing)
                        .is_some()
                    {
                        commands.push(GameCommand::Cast {
                            unit: id,
                            spell: name.to_owned(),
                            target,
                            facing,
                            mana: None,
                        });
                    }
                }
            }
        }
    }
    commands
}

fn trajectory_available(
    trajectory: Trajectory,
    standing: TilePos,
    target: TilePos,
    creates_terrain: bool,
    terrain: Option<&TerrainOccupancy>,
) -> bool {
    matches!(trajectory, Trajectory::None)
        || terrain.is_some_and(|terrain| {
            trajectory_is_clear(
                trajectory,
                standing.above(),
                trajectory_destination(target, creates_terrain),
                terrain,
            )
        })
}

fn damages_downed(
    spell: &hex_assets::Spell,
    target: TilePos,
    units: &UnitQuery,
    authorized: &BTreeSet<UnitId>,
) -> bool {
    spell.effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::DisableHexes {
                targeted: false,
                ..
            } | Effect::Burn { .. }
        )
    }) && units
        .iter()
        .any(|unit| authorized.contains(unit.1) && unit.6 && unit.2 .0.pos == target)
}

fn cell_choice_set(
    decider: UnitId,
    subject: UnitId,
    spec: Option<&LatticeSpec>,
    state: Option<&LatticeState>,
    count: u16,
    kind: AiDecisionKind,
) -> Option<CellChoiceSet> {
    let (Some(spec), Some(state)) = (spec, state) else {
        return None;
    };
    let restoring = kind == AiDecisionKind::ChooseRestores;
    let cells: Vec<LatticeCoord> = spec
        .cells()
        .filter(|(coord, _)| state.is_disabled(*coord) == restoring)
        .map(|(coord, _)| coord)
        .collect();
    let owed = count.min(u16::try_from(cells.len()).unwrap_or(u16::MAX));
    let mut bytes = Vec::with_capacity(32usize.saturating_add(cells.len().saturating_mul(8)));
    bytes.push(match kind {
        AiDecisionKind::ChooseDisables => 0,
        AiDecisionKind::ChooseRestores => 1,
        AiDecisionKind::TurnAction => return None,
    });
    bytes.extend_from_slice(&decider.0.to_le_bytes());
    bytes.extend_from_slice(&subject.0.to_le_bytes());
    bytes.extend_from_slice(&owed.to_le_bytes());
    for cell in &cells {
        bytes.extend_from_slice(&cell.q().to_le_bytes());
        bytes.extend_from_slice(&cell.r().to_le_bytes());
    }
    Some(CellChoiceSet::from_cells(
        CellChoiceFingerprint(xxh3_64(&bytes)),
        subject,
        owed,
        cells,
    ))
}

#[expect(
    clippy::too_many_arguments,
    reason = "projection reads each independent authorized domain store exactly once"
)]
fn build_observation(
    actor: (
        Entity,
        &UnitId,
        &StandsOn,
        &Body,
        &Faction,
        Option<&Turn>,
        bool,
        bool,
        Option<&ControlOwner>,
        Option<&AiController>,
        Option<&LatticeSpec>,
        Option<&LatticeState>,
        Option<&LatticeStats>,
        bool,
    ),
    units: &UnitQuery,
    turn_order: &TurnOrder,
    knowledge: &FactionLatticeKnowledge,
    effects: &PersistentEffects,
    spatial: Option<&FactionKnowledge>,
    footing: Option<&Footing>,
    reach: Option<&Reach>,
    spells: Option<&SpellBook>,
    content: Option<&ContentIndex>,
    elements: Option<&ElementCatalog>,
) -> AiObservation {
    let mut allies: Vec<AiAlliedUnit> = units
        .iter()
        .filter(|unit| unit.0 != actor.0 && *unit.4 == *actor.4)
        .map(|unit| allied(unit, spells, content, elements))
        .collect();
    allies.sort_by_key(|ally| ally.unit);
    let mut hostiles: Vec<AiObservedHostile> = spatial
        .into_iter()
        .flat_map(FactionKnowledge::units)
        .filter(|(_, observed)| actor.4.is_hostile_to(observed.faction))
        .filter_map(|(id, observed)| {
            let unit = units.iter().find(|unit| *unit.1 == id)?;
            Some(AiObservedHostile {
                unit: id,
                position: observed.pos,
                downed: unit.6,
                lattice: known_lattice(knowledge, *actor.4, id),
            })
        })
        .collect();
    hostiles.sort_by_key(|hostile| hostile.unit);
    let known_units: BTreeSet<UnitId> = std::iter::once(*actor.1)
        .chain(allies.iter().map(|ally| ally.unit))
        .chain(hostiles.iter().map(|hostile| hostile.unit))
        .collect();
    let mut effect_observations: Vec<AiEffectObservation> = effects
        .iter()
        .filter(|(_, effect)| {
            known_units.contains(&effect.source) && known_units.contains(&effect.target)
        })
        .map(|(_, effect)| AiEffectObservation {
            source: effect.source,
            target: effect.target,
            payload: effect.payload,
        })
        .collect();
    effect_observations.sort_by_key(|effect| (effect.source, effect.target));
    let mut traversal: Vec<AiTraversalObservation> = spatial
        .zip(footing)
        .zip(reach)
        .into_iter()
        .flat_map(|((spatial, footing), reach)| {
            footing
                .standings()
                .into_iter()
                .map(|standing| {
                    let mut neighbors: Vec<TilePos> = standing
                        .pos
                        .coord
                        .neighbors()
                        .into_iter()
                        .flat_map(|coord| footing.steps_from(standing, coord))
                        .map(|next| next.pos)
                        .collect();
                    neighbors.sort_unstable();
                    neighbors.dedup();
                    AiTraversalObservation {
                        position: standing.pos,
                        knowledge: spatial.state(standing.pos),
                        standable: reach.cost(standing.pos).is_some(),
                        neighbors,
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect();
    traversal.sort_by_key(|surface| surface.position);
    AiObservation {
        actor: allied(actor, spells, content, elements),
        allies,
        hostiles,
        turn_order: turn_order
            .order()
            .iter()
            .copied()
            .filter(|unit| known_units.contains(unit))
            .collect(),
        round: turn_order.round,
        effects: effect_observations,
        traversal,
    }
}

fn allied(
    unit: (
        Entity,
        &UnitId,
        &StandsOn,
        &Body,
        &Faction,
        Option<&Turn>,
        bool,
        bool,
        Option<&ControlOwner>,
        Option<&AiController>,
        Option<&LatticeSpec>,
        Option<&LatticeState>,
        Option<&LatticeStats>,
        bool,
    ),
    spells: Option<&SpellBook>,
    content: Option<&ContentIndex>,
    elements: Option<&ElementCatalog>,
) -> AiAlliedUnit {
    let lattice = full_lattice(unit.10, unit.11);
    let mut spell_observations = Vec::new();
    if let (Some(spec), Some(state), Some(book), Some(index), Some(elements)) =
        (unit.10, unit.11, spells, content, elements)
    {
        let tables = index.tables(elements);
        for (spell_id, name, spell) in book.iter() {
            let inscribed = spec.cells().any(
                |(_, kind)| matches!(kind, CellKind::Spell { spell: found } if found == spell_id),
            );
            if !inscribed {
                continue;
            }
            let direct_disables = spell.effects.iter().fold(0u16, |total, effect| {
                total.saturating_add(match effect {
                    Effect::DisableHexes {
                        count,
                        targeted: false,
                    } => u16::from(*count),
                    _ => 0,
                })
            });
            let self_enchantment = matches!(spell.casting, CastingAxis::Enchantment { .. })
                && matches!(spell.targeting.shape, TargetShape::SelfCast);
            let enchantment_active = state
                .active_enchantments()
                .any(|(_, enchantment)| enchantment.spell == spell_id);
            let castable_now = spec.cells().any(|(coord, kind)| {
                matches!(kind, CellKind::Spell { spell: found } if found == spell_id)
                    && castable(spec, state, coord, &tables).is_ok()
            });
            if castable_now {
                spell_observations.push(AiSpellObservation {
                    name: name.to_owned(),
                    direct_disables,
                    single_target: matches!(spell.targeting.shape, TargetShape::Single),
                    self_enchantment,
                    enchantment_active,
                });
            }
        }
    }
    spell_observations.sort_by(|left, right| left.name.cmp(&right.name));
    AiAlliedUnit {
        unit: *unit.1,
        position: unit.2 .0.pos,
        downed: unit.6,
        lattice,
        spells: spell_observations,
    }
}

fn full_lattice(spec: Option<&LatticeSpec>, state: Option<&LatticeState>) -> AiLatticeObservation {
    let (Some(spec), Some(state)) = (spec, state) else {
        return AiLatticeObservation {
            capacity: None,
            cells: Vec::new(),
        };
    };
    AiLatticeObservation {
        capacity: u16::try_from(spec.cells().count()).ok(),
        cells: spec
            .cells()
            .map(|(coord, kind)| AiLatticeCell {
                coord,
                kind: Some(cell_kind(kind)),
                disabled: Some(state.is_disabled(coord)),
                mana: matches!(kind, CellKind::Gem { .. }).then(|| state.mana(coord)),
            })
            .collect(),
    }
}

fn known_lattice(
    knowledge: &FactionLatticeKnowledge,
    viewer: Faction,
    subject: UnitId,
) -> AiLatticeObservation {
    let Some(view) = knowledge.view(viewer, subject) else {
        return AiLatticeObservation {
            capacity: None,
            cells: Vec::new(),
        };
    };
    AiLatticeObservation {
        capacity: view
            .known_capacity()
            .and_then(|capacity| u16::try_from(capacity).ok()),
        cells: view
            .cells()
            .map(|(coord, known)| AiLatticeCell {
                coord,
                kind: Some(cell_kind(known.kind)),
                disabled: Some(known.disabled),
                mana: known.mana,
            })
            .collect(),
    }
}

const fn cell_kind(kind: CellKind) -> AiCellKind {
    match kind {
        CellKind::Blank => AiCellKind::Blank,
        CellKind::Gem { .. } => AiCellKind::Gem,
        CellKind::Fusion { .. } => AiCellKind::Fusion,
        CellKind::Spell { .. } => AiCellKind::Spell,
    }
}

fn legal_action_set(mut commands: Vec<GameCommand>) -> LegalActionSet {
    commands.sort_by_cached_key(command_semantic_key);
    commands.dedup();
    let mut bytes = Vec::new();
    for command in &commands {
        let key = command_semantic_key(command);
        bytes.extend_from_slice(key.as_bytes());
        bytes.push(0xff);
    }
    LegalActionSet::from_canonical_commands(LegalActionFingerprint(xxh3_64(&bytes)), commands)
}

fn command_semantic_key(command: &GameCommand) -> String {
    match command {
        GameCommand::MoveAlong { unit, path } => format!(
            "0:{:010}:{}",
            unit.0,
            path.iter().map(tile_key).collect::<Vec<_>>().join(";")
        ),
        GameCommand::Strike { unit, target } => format!("1:{:010}:{:010}", unit.0, target.0),
        GameCommand::Cast {
            unit,
            spell,
            target,
            facing,
            mana,
        } => format!(
            "2:{:010}:{spell}:{}:{facing:?}:{mana:?}",
            unit.0,
            tile_key(target)
        ),
        GameCommand::Channel { unit } => format!("3:{:010}", unit.0),
        GameCommand::EndTurn { unit } => format!("4:{:010}", unit.0),
        GameCommand::ChooseDisables { unit, cells } => {
            format!("5:{:010}:{cells:?}", unit.0)
        }
        GameCommand::ChooseRestores {
            unit,
            target,
            cells,
        } => format!("6:{:010}:{:010}:{cells:?}", unit.0, target.0),
        other => format!("9:{other:?}"),
    }
}

fn tile_key(pos: &TilePos) -> String {
    format!(
        "{:+011},{:+011},{:+011}",
        pos.coord.x(),
        pos.coord.y(),
        pos.level
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_ai::{AiGroupId, AiProfile};
    use hex_core::{HexCoord, RunBottom};

    #[test]
    fn legal_sets_ignore_input_order() {
        let commands = vec![
            GameCommand::EndTurn { unit: UnitId(2) },
            GameCommand::Strike {
                unit: UnitId(2),
                target: UnitId(1),
            },
        ];
        let mut reversed = commands.clone();
        reversed.reverse();
        assert_eq!(legal_action_set(commands), legal_action_set(reversed));
    }

    #[test]
    fn compact_cell_choice_set_is_canonical_and_exact() {
        let cells = vec![
            LatticeCoord::ORIGIN,
            LatticeCoord::new(1, 0),
            LatticeCoord::new(0, 1),
        ];
        let choices =
            CellChoiceSet::from_cells(CellChoiceFingerprint(7), UnitId(1), 2, cells.clone());
        assert_eq!(choices.count(), 2);
        assert_eq!(choices.eligible().len(), 3);
        assert_eq!(
            choices.eligible(),
            &[
                LatticeCoord::ORIGIN,
                LatticeCoord::new(0, 1),
                LatticeCoord::new(1, 0),
            ]
        );
    }

    #[test]
    fn semantic_tile_key_distinguishes_surfaces() {
        let low = TilePos::new(HexCoord::ORIGIN, 1);
        let high = TilePos::new(HexCoord::ORIGIN, 2);
        assert_ne!(tile_key(&low), tile_key(&high));
    }

    #[test]
    fn ai_legality_filters_material_trajectories_with_the_shared_voxel_rule() {
        let standing = TilePos::new(HexCoord::ORIGIN, 1);
        let target = TilePos::new(HexCoord::from_axial(3, 0), 1);
        let blocker = TilePos::new(HexCoord::from_axial(1, 0), 2);
        let terrain =
            TerrainOccupancy::from_runs([(blocker, RunBottom(blocker.level))]).expect("wall");

        assert!(!trajectory_available(
            Trajectory::Direct,
            standing,
            target,
            false,
            Some(&terrain),
        ));
        assert!(trajectory_available(
            Trajectory::Arc { rise: 3 },
            standing,
            target,
            false,
            Some(&terrain),
        ));
        assert!(trajectory_available(
            Trajectory::None,
            standing,
            target,
            false,
            None,
        ));
        assert!(
            !trajectory_available(Trajectory::Direct, standing, target, false, None),
            "material-sensitive AI casts fail closed without exact occupancy"
        );
    }

    #[test]
    fn encounter_controller_override_dispatches_its_profile_algorithm() {
        let profiles = AiProfileCatalog {
            profiles: vec![
                AiProfile {
                    id: AiProfileId("guard".to_owned()),
                    algorithm: AiAlgorithmId("first".to_owned()),
                },
                AiProfile {
                    id: AiProfileId("raider".to_owned()),
                    algorithm: AiAlgorithmId("last".to_owned()),
                },
            ],
        };
        let controller = AiController {
            profile: AiProfileId("raider".to_owned()),
            group: Some(AiGroupId("west".to_owned())),
        };
        assert_eq!(
            configured_algorithm(&controller, Some(&profiles)),
            AiAlgorithmId("last".to_owned())
        );
    }
}
