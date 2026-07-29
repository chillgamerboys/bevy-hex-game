//! Bounded combat history with disclosure frozen at ingestion time.

use std::collections::{BTreeMap, VecDeque};

use bevy::picking::Pickable;
use bevy::prelude::*;
use hex_assets::{ElementCatalog, SpellBook};
use hex_combat::{CombatEvent, CombatSystems, CommandRefusal, FactionKnowledge};
use hex_core::{AppSystems, GameCommand, LatticeCoord, Screen, TilePos, UnitId};
use hex_lattice::{CellKind, LatticeSpec};
use hex_units::{Faction, Player, StandsOn, UnitRegistry};

use super::lattice::{set_pulse_color, RetainedTarget, TargetPanel};
use crate::menus::widgets::{heading, panel, UiAssets, DANGER, FINE_SIZE, LABEL};
use crate::readouts::HudElement;
use crate::screens::DespawnOnExit;

const CAPACITY: usize = 64;
const VISIBLE_LINES: usize = 6;
const PULSE_SECONDS: f32 = 0.28;
const FRAME: Pickable = Pickable {
    should_block_lower: true,
    is_hoverable: false,
};

#[derive(Debug, Clone, PartialEq)]
struct LogLine {
    text: String,
    danger: bool,
}

#[derive(Resource, Default)]
struct CombatLog {
    lines: VecDeque<LogLine>,
    /// Damage outcomes follow a typed cause event. This freezes that stable cause by
    /// source/target pair until the resulting defender answer resolves.
    causes: BTreeMap<(UnitId, UnitId), DamageCause>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DamageCause {
    Spell(String),
    Burn,
}

impl CombatLog {
    fn push(&mut self, line: LogLine) {
        self.lines.push_back(line);
        while self.lines.len() > CAPACITY {
            self.lines.pop_front();
        }
    }
}

#[derive(Resource, Default)]
struct DamagePulse {
    target: Option<UnitId>,
    timer: Option<Timer>,
}

impl DamagePulse {
    fn start(&mut self, target: UnitId) {
        self.target = Some(target);
        self.timer = Some(Timer::from_seconds(PULSE_SECONDS, TimerMode::Once));
    }
}

#[derive(Component)]
struct LogBody;

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<CombatLog>()
        .init_resource::<DamagePulse>()
        .add_systems(OnEnter(Screen::Gameplay), (spawn_panel, reset))
        // Not pausable. Bevy messages age out after two frames, so pausing on
        // the resolution frame must not erase the outcome from history.
        .add_systems(
            Update,
            ingest
                .in_set(AppSystems::Update)
                .after(CombatSystems::Advance)
                .after(super::lattice::retain_target)
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(
            Update,
            (rebuild, pulse_panel)
                .chain()
                .after(ingest)
                .run_if(in_state(Screen::Gameplay)),
        );
}

fn spawn_panel(mut commands: Commands, assets: Res<UiAssets>) {
    commands
        .spawn((
            Name::new("Combat Log Panel"),
            HudElement,
            panel(),
            FRAME,
            DespawnOnExit(Screen::Gameplay),
        ))
        .insert(Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(66.0),
            left: Val::Px(310.0),
            width: Val::Px(470.0),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(Val::Px(12.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(10.0)),
            row_gap: Val::Px(4.0),
            ..default()
        })
        .with_children(|panel| {
            panel.spawn(heading(&assets, "combat log"));
            panel.spawn((
                Name::new("Combat Log Body"),
                LogBody,
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(3.0),
                    ..default()
                },
                Pickable::IGNORE,
            ));
        });
}

fn reset(mut log: ResMut<CombatLog>, mut pulse: ResMut<DamagePulse>) {
    *log = CombatLog::default();
    *pulse = DamagePulse::default();
}

type IdentityQuery<'w, 's> = Query<
    'w,
    's,
    (
        Option<&'static Name>,
        Option<&'static Faction>,
        Option<&'static StandsOn>,
    ),
>;

#[expect(
    clippy::too_many_arguments,
    reason = "formatting freezes names, factions, positions, own truth, and hostile knowledge"
)]
fn ingest(
    mut events: MessageReader<CombatEvent>,
    mut log: ResMut<CombatLog>,
    mut pulse: ResMut<DamagePulse>,
    registry: Res<UnitRegistry>,
    identities: IdentityQuery,
    own_lattices: Query<(&UnitId, &LatticeSpec), With<Player>>,
    knowledge: Res<FactionKnowledge>,
    elements: Option<Res<ElementCatalog>>,
    spells: Option<Res<SpellBook>>,
) {
    for event in events.read() {
        remember_cause(event, &mut log, &registry, &identities);
        if let Some(target) = pulse_target(event) {
            pulse.start(target);
        }
        if let Some(line) = format_event(
            event,
            &log,
            &registry,
            &identities,
            &own_lattices,
            &knowledge,
            elements.as_deref(),
            spells.as_deref(),
        ) {
            log.push(line);
        }
        if let CombatEvent::HexesDisabled { source, target, .. } = event {
            log.causes.remove(&(*source, *target));
        }
    }
}

fn remember_cause(
    event: &CombatEvent,
    log: &mut CombatLog,
    registry: &UnitRegistry,
    identities: &IdentityQuery,
) {
    match event {
        CombatEvent::Cast {
            caster,
            spell,
            target,
        } => {
            if let Some(subject) = unit_at(*target, registry, identities) {
                log.causes
                    .insert((*caster, subject), DamageCause::Spell(spell.clone()));
            }
        }
        CombatEvent::Strike { attacker, target } => {
            log.causes.remove(&(*attacker, *target));
        }
        CombatEvent::BurnTicked { source, target, .. } => {
            log.causes.insert((*source, *target), DamageCause::Burn);
        }
        _ => {}
    }
}

fn unit_at(target: TilePos, registry: &UnitRegistry, identities: &IdentityQuery) -> Option<UnitId> {
    registry.iter().find_map(|(unit, entity)| {
        identities
            .get(entity)
            .ok()
            .and_then(|(_, _, standing)| (standing?.0.pos == target).then_some(unit))
    })
}

fn pulse_target(event: &CombatEvent) -> Option<UnitId> {
    match event {
        CombatEvent::DecisionOpened { decider, .. } => Some(*decider),
        CombatEvent::DamagePrevented { target, .. }
        | CombatEvent::HexesDisabled { target, .. }
        | CombatEvent::BurnApplied { target, .. }
        | CombatEvent::BurnTicked { target, .. } => Some(*target),
        _ => None,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "a frozen disclosure snapshot needs every legal presentation source"
)]
fn format_event(
    event: &CombatEvent,
    log: &CombatLog,
    registry: &UnitRegistry,
    identities: &IdentityQuery,
    own_lattices: &Query<(&UnitId, &LatticeSpec), With<Player>>,
    knowledge: &FactionKnowledge,
    elements: Option<&ElementCatalog>,
    spells: Option<&SpellBook>,
) -> Option<LogLine> {
    let line = match event {
        CombatEvent::Cast { caster, spell, .. } => LogLine {
            text: format!("{} cast {spell}", unit_name(*caster, registry, identities)),
            danger: false,
        },
        CombatEvent::Strike { attacker, target } => LogLine {
            text: format!(
                "{} struck {}",
                unit_name(*attacker, registry, identities),
                unit_name(*target, registry, identities)
            ),
            danger: false,
        },
        // The eventual exact disable line is the durable result. Logging the
        // intermediate count would leak an opaque hostile's lattice damage.
        CombatEvent::DecisionOpened { .. } => return None,
        CombatEvent::DamagePrevented { source, target, .. } => LogLine {
            text: format!(
                "{}'s defence absorbed the hit from {}",
                unit_name(*target, registry, identities),
                cause_name(*source, *target, log, registry, identities)
            ),
            danger: false,
        },
        CombatEvent::HexesDisabled {
            source,
            target,
            cells,
        } => {
            let known = disclosed_cells(
                *target,
                cells,
                registry,
                identities,
                own_lattices,
                knowledge,
                elements,
                spells,
            );
            let actor = cause_name(*source, *target, log, registry, identities);
            let subject = unit_name(*target, registry, identities);
            let text = if known.is_empty() {
                // `BurnTicked` already emitted the legally safe qualitative line. An
                // opaque exact answer has no further information to add.
                if matches!(log.causes.get(&(*source, *target)), Some(DamageCause::Burn)) {
                    return None;
                }
                format!("{actor} damaged {subject}")
            } else if known.len() == cells.len() {
                format!("{actor} disabled {} on {subject}", known.join(", "))
            } else {
                format!("{actor} damaged {subject}, including {}", known.join(", "))
            };
            LogLine { text, danger: true }
        }
        CombatEvent::EnchantmentBroken {
            unit,
            spell,
            burned_mana,
            trigger,
        } => {
            let visible = disclosed_cells(
                *unit,
                &[*trigger],
                registry,
                identities,
                own_lattices,
                knowledge,
                elements,
                spells,
            );
            let text = if visible.is_empty() {
                format!(
                    "{}'s enchantment broke",
                    unit_name(*unit, registry, identities)
                )
            } else {
                let trigger_name = visible.first().map_or("known cell", String::as_str);
                format!(
                    "{}'s {} broke at {}; {burned_mana} mana burned",
                    unit_name(*unit, registry, identities),
                    spell.as_deref().unwrap_or("enchantment"),
                    trigger_name
                )
            };
            LogLine { text, danger: true }
        }
        CombatEvent::BurnApplied { source, target, .. } => LogLine {
            text: format!(
                "{} ignited {}",
                cause_name(*source, *target, log, registry, identities),
                unit_name(*target, registry, identities)
            ),
            danger: false,
        },
        CombatEvent::BurnTicked { target, .. } => LogLine {
            text: format!("Burn damaged {}", unit_name(*target, registry, identities)),
            danger: true,
        },
        CombatEvent::Revealed {
            viewer, subject, ..
        } => {
            if *viewer != Faction::Player {
                return None;
            }
            LogLine {
                text: format!(
                    "{}'s lattice was revealed",
                    unit_name(*subject, registry, identities)
                ),
                danger: false,
            }
        }
        CombatEvent::Downed { unit } => LogLine {
            text: format!("{} went down", unit_name(*unit, registry, identities)),
            danger: true,
        },
        CombatEvent::CommandRefused { command, refusal } => LogLine {
            text: format!(
                "{} could not {}: {}",
                unit_name(command.unit(), registry, identities),
                command_label(command),
                refusal_label(refusal)
            ),
            danger: true,
        },
    };
    Some(line)
}

fn cause_name(
    source: UnitId,
    target: UnitId,
    log: &CombatLog,
    registry: &UnitRegistry,
    identities: &IdentityQuery,
) -> String {
    match log.causes.get(&(source, target)) {
        Some(DamageCause::Spell(spell)) => spell.clone(),
        Some(DamageCause::Burn) => "Burn".to_owned(),
        None => unit_name(source, registry, identities),
    }
}

fn disclosed_cells(
    target: UnitId,
    cells: &[LatticeCoord],
    registry: &UnitRegistry,
    identities: &IdentityQuery,
    own_lattices: &Query<(&UnitId, &LatticeSpec), With<Player>>,
    knowledge: &FactionKnowledge,
    elements: Option<&ElementCatalog>,
    spells: Option<&SpellBook>,
) -> Vec<String> {
    let faction = registry
        .entity_of(target)
        .and_then(|entity| identities.get(entity).ok())
        .and_then(|(_, faction, _)| faction.copied());
    cells
        .iter()
        .filter_map(|coord| {
            let kind = match faction {
                Some(Faction::Player) => own_lattices
                    .iter()
                    .find(|(unit, _)| **unit == target)
                    .and_then(|(_, spec)| spec.get(*coord)),
                Some(Faction::Hostile) => knowledge
                    .view(Faction::Player, target)
                    .and_then(|known| known.cell(*coord))
                    .map(|cell| cell.kind),
                None => None,
            }?;
            Some(cell_name(*coord, kind, elements, spells))
        })
        .collect()
}

fn cell_name(
    coord: LatticeCoord,
    kind: CellKind,
    elements: Option<&ElementCatalog>,
    spells: Option<&SpellBook>,
) -> String {
    let identity = match kind {
        CellKind::Gem { element } => elements
            .and_then(|catalog| catalog.name(element))
            .map_or_else(|| "gem".to_owned(), |name| format!("{name} gem")),
        CellKind::Fusion { output } => elements
            .and_then(|catalog| catalog.name(output))
            .map_or_else(|| "fusion".to_owned(), |name| format!("{name} fusion")),
        CellKind::Spell { spell } => spells
            .and_then(|book| book.name(spell))
            .unwrap_or("spell")
            .to_owned(),
        CellKind::Blank => "blank".to_owned(),
    };
    format!("{identity} ({}, {})", coord.q(), coord.r())
}

fn unit_name(unit: UnitId, registry: &UnitRegistry, identities: &IdentityQuery) -> String {
    registry
        .entity_of(unit)
        .and_then(|entity| identities.get(entity).ok())
        .and_then(|(name, _, _)| name)
        .map_or_else(
            || format!("unit #{}", unit.0),
            |name| name.as_str().to_owned(),
        )
}

fn command_label(command: &GameCommand) -> &'static str {
    match command {
        GameCommand::MoveAlong { .. } => "move",
        GameCommand::Strike { .. } => "strike",
        GameCommand::EndTurn { .. } => "end the turn",
        GameCommand::Cast { .. } => "cast",
        GameCommand::Channel { .. } => "channel",
        GameCommand::ChooseDisables { .. } => "choose damaged cells",
    }
}

fn refusal_label(refusal: &CommandRefusal) -> &'static str {
    match refusal {
        CommandRefusal::UnknownUnit => "unknown unit",
        CommandRefusal::WrongSeat { .. } => "wrong controller",
        CommandRefusal::CombatOnly => "combat only",
        CommandRefusal::NotCurrentTurn { .. } => "not the current turn",
        CommandRefusal::DecisionPending { .. } => "a decision is open",
        CommandRefusal::MissingCombatData { .. } => "combat data unavailable",
        CommandRefusal::MissingUnitData { .. } => "unit data unavailable",
        CommandRefusal::Busy => "still busy",
        CommandRefusal::InvalidPath => "invalid path",
        CommandRefusal::MovementBudgetExceeded { .. } => "not enough movement",
        CommandRefusal::UnknownTarget { .. } => "unknown target",
        CommandRefusal::TargetNotHostile { .. } => "target is not hostile",
        CommandRefusal::TargetOutOfMeleeReach { .. } => "target is out of reach",
        CommandRefusal::NoTurn => "no active turn",
        CommandRefusal::ActionAlreadySpent => "action already spent",
        CommandRefusal::UnknownSpell { .. } => "unknown spell",
        CommandRefusal::MissingSpellDefinition { .. } => "spell definition unavailable",
        CommandRefusal::UndeliverableSpell { .. } => "spell is not implemented",
        CommandRefusal::MissingFacing { .. } => "direction required",
        CommandRefusal::TargetOutOfRange { .. } => "target is out of range",
        CommandRefusal::ShapeUnresolved { .. } => "shape could not resolve",
        CommandRefusal::TargetUnobserved { .. } => "target is unobserved",
        CommandRefusal::SpellNotInscribed { .. } => "spell is not inscribed",
        CommandRefusal::CastBlocked { .. } => "lattice cannot pay",
        CommandRefusal::CastPlanStale { .. } => "lattice changed",
        CommandRefusal::ChannelUnavailable => "channelling is unavailable",
        CommandRefusal::NoPendingDecision => "no decision is open",
        CommandRefusal::WrongDecisionUnit { .. } => "another unit must decide",
        CommandRefusal::WrongDisableCount { .. } => "wrong number of cells",
        CommandRefusal::CellOutsideLattice { .. } => "cell is outside the lattice",
        CommandRefusal::DuplicateCell { .. } => "cell was chosen twice",
        CommandRefusal::CellAlreadyDisabled { .. } => "cell is already disabled",
    }
}

fn rebuild(
    mut commands: Commands,
    log: Res<CombatLog>,
    bodies: Query<Entity, With<LogBody>>,
    assets: Res<UiAssets>,
) {
    if !log.is_changed() {
        return;
    }
    let Ok(body) = bodies.single() else { return };
    commands.entity(body).despawn_related::<Children>();
    commands.entity(body).with_children(|rows| {
        for line in log
            .lines
            .iter()
            .rev()
            .take(VISIBLE_LINES)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            rows.spawn((
                Text::new(line.text.clone()),
                TextFont {
                    font: assets.body.clone().into(),
                    ..TextFont::from_font_size(FINE_SIZE)
                },
                TextColor(if line.danger { DANGER } else { LABEL }),
                Pickable::IGNORE,
            ));
        }
    });
}

fn pulse_panel(
    time: Res<Time>,
    focus: Res<RetainedTarget>,
    mut pulse: ResMut<DamagePulse>,
    mut panels: Query<&mut BackgroundColor, With<TargetPanel>>,
) {
    let active = if pulse.target == focus.0 {
        pulse.timer.as_mut().is_some_and(|timer| {
            timer.tick(time.delta());
            !timer.is_finished()
        })
    } else {
        false
    };
    set_pulse_color(active, &mut panels);
}

#[cfg(test)]
mod tests {
    use bevy::MinimalPlugins;
    use hex_combat::{BaseVisibility, KnownCell};
    use hex_core::{ElementId, KnowledgeExpiry, KnowledgeSource};

    use super::*;

    #[test]
    fn the_log_keeps_sixty_four_entries() {
        let mut log = CombatLog::default();
        for number in 0..80 {
            log.push(LogLine {
                text: number.to_string(),
                danger: false,
            });
        }
        assert_eq!(log.lines.len(), CAPACITY);
        assert_eq!(log.lines.front().map(|line| line.text.as_str()), Some("16"));
        assert_eq!(log.lines.back().map(|line| line.text.as_str()), Some("79"));
    }

    #[test]
    fn disclosure_is_frozen_and_partial_knowledge_names_only_known_cells() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<CombatEvent>()
            .init_resource::<CombatLog>()
            .init_resource::<DamagePulse>()
            .init_resource::<UnitRegistry>()
            .init_resource::<FactionKnowledge>()
            .add_systems(Update, ingest);

        let source = UnitId(1);
        let target = UnitId(9);
        let source_entity = app
            .world_mut()
            .spawn((source, Name::new("Ember"), Faction::Player, Player))
            .id();
        let target_entity = app
            .world_mut()
            .spawn((target, Name::new("wolf #9"), Faction::Hostile))
            .id();
        {
            let mut registry = app.world_mut().resource_mut::<UnitRegistry>();
            registry.register(source, source_entity);
            registry.register(target, target_entity);
        }
        app.world_mut()
            .resource_mut::<FactionKnowledge>()
            .observe_base(
                Faction::Player,
                target,
                BaseVisibility {
                    faction: Faction::Hostile,
                },
            );

        let known = LatticeCoord::new(0, 0);
        let hidden = LatticeCoord::new(1, 0);
        let event = CombatEvent::HexesDisabled {
            source,
            target,
            cells: vec![known, hidden],
        };
        app.world_mut()
            .resource_mut::<Messages<CombatEvent>>()
            .write(event.clone());
        app.update();

        let opaque_line = app
            .world()
            .resource::<CombatLog>()
            .lines
            .back()
            .map(|line| line.text.clone());
        assert_eq!(opaque_line.as_deref(), Some("Ember damaged wolf #9"));

        assert!(app.world_mut().resource_mut::<FactionKnowledge>().learn(
            Faction::Player,
            target,
            known,
            KnownCell {
                kind: CellKind::Gem {
                    element: ElementId(0),
                },
                mana: Some(2),
                disabled: false,
                source: KnowledgeSource::Divination,
                expiry: KnowledgeExpiry::Sustained,
            },
        ));
        app.world_mut()
            .resource_mut::<Messages<CombatEvent>>()
            .write(event);
        app.update();

        let log = app.world().resource::<CombatLog>();
        assert_eq!(
            log.lines.front().map(|line| line.text.as_str()),
            Some("Ember damaged wolf #9"),
            "later divination must not rewrite the first line"
        );
        let partial = log
            .lines
            .back()
            .map(|line| line.text.as_str())
            .unwrap_or("");
        assert!(partial.contains("(0, 0)"), "{partial}");
        assert!(!partial.contains("(1, 0)"), "{partial}");
    }

    #[test]
    fn burn_replaces_a_stale_spell_cause_without_duplicating_hidden_damage() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<CombatEvent>()
            .init_resource::<CombatLog>()
            .init_resource::<DamagePulse>()
            .init_resource::<UnitRegistry>()
            .init_resource::<FactionKnowledge>()
            .add_systems(Update, ingest);

        let source = UnitId(1);
        let target = UnitId(9);
        let source_entity = app
            .world_mut()
            .spawn((source, Name::new("hedge-mage #1"), Faction::Player, Player))
            .id();
        let target_entity = app
            .world_mut()
            .spawn((target, Name::new("wolf #9"), Faction::Hostile))
            .id();
        {
            let mut registry = app.world_mut().resource_mut::<UnitRegistry>();
            registry.register(source, source_entity);
            registry.register(target, target_entity);
        }
        app.world_mut()
            .resource_mut::<FactionKnowledge>()
            .observe_base(
                Faction::Player,
                target,
                BaseVisibility {
                    faction: Faction::Hostile,
                },
            );
        app.world_mut().resource_mut::<CombatLog>().causes.insert(
            (source, target),
            DamageCause::Spell("Scrying Eye".to_owned()),
        );

        let mut events = app.world_mut().resource_mut::<Messages<CombatEvent>>();
        events.write(CombatEvent::BurnTicked {
            source,
            target,
            count: 1,
        });
        events.write(CombatEvent::HexesDisabled {
            source,
            target,
            cells: vec![LatticeCoord::ORIGIN],
        });
        app.update();

        let log = app.world().resource::<CombatLog>();
        let lines: Vec<_> = log.lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(lines, ["Burn damaged wolf #9"]);
        assert!(
            !log.causes.contains_key(&(source, target)),
            "the resolved pair must not leak into a later unrelated hit"
        );
    }
}
