//! Algorithm-neutral enemy decision host.
//!
//! Combat owns legality. Algorithms receive an authorized observation and opaque keys
//! for the complete legal command set; their only authority is choosing one key. The
//! selected command then travels through the same queue and applier as player input.

use std::collections::BTreeMap;

use bevy::prelude::*;
use hex_ai::{
    ActionKey, AiAlgorithm, AiAlgorithmId, AiAlliedUnit, AiCellKind, AiController,
    AiDecisionFailure, AiDecisionKind, AiDecisionTrace, AiEffectObservation, AiLatticeCell,
    AiLatticeObservation, AiObservation, AiObservedHostile, AiProfileId, AiSpellObservation,
    AiTraversalObservation, BaselineAlgorithm, DecisionRequest, LegalActionFingerprint,
    LegalActionSet,
};
use hex_assets::{
    AiProfileCatalog, CastingAxis, CombatSettings, ContentIndex, Effect, ElementCatalog, SpellBook,
    SubstanceTable, TargetShape,
};
use hex_core::{
    Busy, CommandQueue, ControlOwner, GameCommand, GameplaySetup, GameplaySetupFailure, Headroom,
    HexSpan, HexTile, IssuedCommand, KnowledgeState, LatticeCoord, Mode, PausableSystems,
    PendingDecision, Screen, Sextant, SubstanceId, TilePos, Turn, UnitId,
};
use hex_lattice::{castable, CellKind, LatticeSpec, LatticeState};
use hex_units::{
    targeting, volumes, Body, Downed, Enemy, Faction, Footing, Player, Reach, StandsOn,
    UnitRegistry,
};
use xxhash_rust::xxh3::xxh3_64;

use crate::{delivers_anything, FactionKnowledge, PersistentEffects, TurnOrder};

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
}

type TileQuery<'w, 's> = Query<
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
    traces.entries.clear();
}

#[expect(
    clippy::too_many_arguments,
    reason = "the host projects independent authoritative stores into a pure request"
)]
fn drive_ai(
    turn_order: Res<TurnOrder>,
    unit_registry: Res<UnitRegistry>,
    pending: Res<PendingDecision>,
    mut queue: ResMut<CommandQueue>,
    mut algorithms: ResMut<AiAlgorithmRegistry>,
    mut traces: ResMut<AiDecisionTraces>,
    profiles: Option<Res<AiProfileCatalog>>,
    table: Option<Res<SubstanceTable>>,
    spells: Option<Res<SpellBook>>,
    content: Option<Res<ContentIndex>>,
    elements: Option<Res<ElementCatalog>>,
    combat: Option<Res<CombatSettings>>,
    knowledge: Res<FactionKnowledge>,
    effects: Res<PersistentEffects>,
    tiles: TileQuery,
    units: UnitQuery,
) {
    let Some(table) = table else {
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
    if !actor.12 || actor.7 {
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
    let algorithm_id = configured_algorithm(controller, profiles.as_deref());
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

    let footing = Footing::from_tiles(tiles.iter(), &table, *actor.3);
    let commands = match kind {
        AiDecisionKind::TurnAction => enumerate_turn_actions(
            actor,
            &units,
            &footing,
            &tiles,
            spells.as_deref(),
            content.as_deref(),
            elements.as_deref(),
            combat.as_deref(),
        ),
        AiDecisionKind::ChooseDisables => {
            enumerate_cell_choices(actor_id, actor.10, actor.11, count, false, None)
        }
        AiDecisionKind::ChooseRestores => restoration_target
            .and_then(|target| {
                unit_registry
                    .entity_of(target)
                    .map(|entity| (target, entity))
            })
            .and_then(|(target, entity)| units.get(entity).ok().map(|unit| (target, unit)))
            .map_or_else(Vec::new, |(target, target_unit)| {
                enumerate_cell_choices(
                    actor_id,
                    target_unit.10,
                    target_unit.11,
                    count,
                    true,
                    Some(target),
                )
            }),
    };
    if commands.is_empty() {
        warn!("AI {actor_id:?} has no legal command for {kind:?}");
        return;
    }
    let legal_actions = legal_action_set(commands);
    let observation = build_observation(
        actor,
        &units,
        &turn_order,
        &knowledge,
        &effects,
        &footing,
        spells.as_deref(),
        content.as_deref(),
        elements.as_deref(),
    );
    let request = DecisionRequest {
        controller: actor.8.copied().unwrap_or_default().0,
        group: controller.group.clone(),
        kind,
        observation,
        legal_actions,
    };
    let selected = algorithms
        .get_mut(&dispatched_id)
        .map(|algorithm| algorithm.select(&request))
        .unwrap_or_else(|| ActionKey::from_parts(request.legal_actions.fingerprint(), u32::MAX));
    let failure = if selected.fingerprint() != request.legal_actions.fingerprint() {
        Some(AiDecisionFailure::StaleFingerprint)
    } else if request.legal_actions.resolve(selected).is_none() {
        Some(AiDecisionFailure::UnknownAction)
    } else {
        None
    };
    let command = request
        .legal_actions
        .resolve(selected)
        .or_else(|| {
            request
                .legal_actions
                .actions()
                .iter()
                .find(|action| matches!(action.command, GameCommand::EndTurn { .. }))
        })
        .or_else(|| request.legal_actions.actions().first())
        .map(|action| action.command.clone());
    traces.entries.push(AiDecisionTrace {
        profile: controller.profile.clone(),
        algorithm: dispatched_id,
        actor: actor_id,
        group: controller.group.clone(),
        kind,
        fingerprint: request.legal_actions.fingerprint(),
        selected,
        failure,
    });
    if let Some(failure) = failure {
        warn!("AI {actor_id:?} returned an invalid action key: {failure:?}");
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
        bool,
    ),
    units: &UnitQuery,
    footing: &Footing,
    tiles: &TileQuery,
    spells: Option<&SpellBook>,
    content: Option<&ContentIndex>,
    elements: Option<&ElementCatalog>,
    combat: Option<&CombatSettings>,
) -> Vec<GameCommand> {
    let id = *actor.1;
    let Some(turn) = actor.5 else {
        return Vec::new();
    };
    let mut commands = vec![GameCommand::EndTurn { unit: id }];
    if turn.acted {
        return commands;
    }

    if turn.movement_left > 0 {
        let reach = Reach::from(actor.2 .0, footing, Some(turn.movement_left));
        for surface in reach.surfaces() {
            if surface.pos == actor.2 .0.pos || occupied(surface.pos, units, Some(id)) {
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
        .filter(|unit| !unit.6 && actor.4.is_hostile_to(*unit.4) && unit.0 != actor.0)
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
        let mut anchors: Vec<TilePos> = tiles.iter().map(|(pos, ..)| *pos).collect();
        anchors.sort_unstable();
        anchors.dedup();
        for (spell_id, name, spell) in book.iter() {
            if !delivers_anything(spell) {
                continue;
            }
            let Some(cell) = spec.cells().find_map(|(coord, kind)| {
                matches!(kind, CellKind::Spell { spell: found } if found == spell_id)
                    .then_some(coord)
            }) else {
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
                if damages_downed(spell, target, units) {
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

fn occupied(pos: TilePos, units: &UnitQuery, except: Option<UnitId>) -> bool {
    units
        .iter()
        .any(|unit| Some(*unit.1) != except && unit.2 .0.pos == pos)
}

fn damages_downed(spell: &hex_assets::Spell, target: TilePos, units: &UnitQuery) -> bool {
    spell.effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::DisableHexes {
                targeted: false,
                ..
            } | Effect::Burn { .. }
        )
    }) && units.iter().any(|unit| unit.6 && unit.2 .0.pos == target)
}

fn enumerate_cell_choices(
    decider: UnitId,
    spec: Option<&LatticeSpec>,
    state: Option<&LatticeState>,
    count: u16,
    restoring: bool,
    target: Option<UnitId>,
) -> Vec<GameCommand> {
    let (Some(spec), Some(state)) = (spec, state) else {
        return Vec::new();
    };
    let cells: Vec<LatticeCoord> = spec
        .cells()
        .filter(|(coord, _)| state.is_disabled(*coord) == restoring)
        .map(|(coord, _)| coord)
        .collect();
    let take = usize::from(count).min(cells.len());
    let mut combinations = Vec::new();
    combinations_of(&cells, take, 0, &mut Vec::new(), &mut combinations);
    combinations
        .into_iter()
        .map(|cells| {
            if restoring {
                GameCommand::ChooseRestores {
                    unit: decider,
                    target: target.unwrap_or(decider),
                    cells,
                }
            } else {
                GameCommand::ChooseDisables {
                    unit: decider,
                    cells,
                }
            }
        })
        .collect()
}

fn combinations_of(
    cells: &[LatticeCoord],
    take: usize,
    start: usize,
    current: &mut Vec<LatticeCoord>,
    output: &mut Vec<Vec<LatticeCoord>>,
) {
    if current.len() == take {
        output.push(current.clone());
        return;
    }
    for index in start..cells.len() {
        let Some(&cell) = cells.get(index) else {
            continue;
        };
        current.push(cell);
        combinations_of(cells, take, index.saturating_add(1), current, output);
        let _ = current.pop();
    }
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
        bool,
    ),
    units: &UnitQuery,
    turn_order: &TurnOrder,
    knowledge: &FactionKnowledge,
    effects: &PersistentEffects,
    footing: &Footing,
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
    let mut hostiles: Vec<AiObservedHostile> = units
        .iter()
        .filter(|unit| actor.4.is_hostile_to(*unit.4))
        .map(|unit| AiObservedHostile {
            unit: *unit.1,
            position: unit.2 .0.pos,
            lattice: known_lattice(knowledge, *actor.4, *unit.1),
        })
        .collect();
    hostiles.sort_by_key(|hostile| hostile.unit);
    let mut effect_observations: Vec<AiEffectObservation> = effects
        .iter()
        .map(|(_, effect)| AiEffectObservation {
            source: effect.source,
            target: effect.target,
            payload: effect.payload,
        })
        .collect();
    effect_observations.sort_by_key(|effect| (effect.source, effect.target));
    let reach = Reach::from(actor.2 .0, footing, None);
    let mut traversal: Vec<AiTraversalObservation> = reach
        .surfaces()
        .map(|standing| {
            let mut neighbors: Vec<TilePos> = standing
                .pos
                .coord
                .neighbors()
                .into_iter()
                .flat_map(|coord| footing.steps_from(standing, coord))
                .map(|next| next.pos)
                .filter(|position| reach.cost(*position).is_some())
                .collect();
            neighbors.sort_unstable();
            neighbors.dedup();
            AiTraversalObservation {
                position: standing.pos,
                knowledge: KnowledgeState::Observed,
                standable: true,
                neighbors,
            }
        })
        .collect();
    traversal.sort_by_key(|surface| surface.position);
    AiObservation {
        actor: allied(actor, spells, content, elements),
        allies,
        hostiles,
        turn_order: turn_order.order().to_vec(),
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
    knowledge: &FactionKnowledge,
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
        GameCommand::EndTurn { unit } => format!("3:{:010}", unit.0),
        GameCommand::ChooseDisables { unit, cells } => {
            format!("4:{:010}:{cells:?}", unit.0)
        }
        GameCommand::ChooseRestores {
            unit,
            target,
            cells,
        } => format!("5:{:010}:{:010}:{cells:?}", unit.0, target.0),
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
    use hex_core::HexCoord;

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
    fn combinations_are_complete_and_stable() {
        let cells = [
            LatticeCoord::ORIGIN,
            LatticeCoord::new(1, 0),
            LatticeCoord::new(0, 1),
        ];
        let mut found = Vec::new();
        combinations_of(&cells, 2, 0, &mut Vec::new(), &mut found);
        assert_eq!(found.len(), 3);
        assert_eq!(found.first(), Some(&vec![cells[0], cells[1]]));
    }

    #[test]
    fn semantic_tile_key_distinguishes_surfaces() {
        let low = TilePos::new(HexCoord::ORIGIN, 1);
        let high = TilePos::new(HexCoord::ORIGIN, 2);
        assert_ne!(tile_key(&low), tile_key(&high));
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
