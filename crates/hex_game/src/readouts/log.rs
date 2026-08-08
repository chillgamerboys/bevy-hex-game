//! Bounded combat history with disclosure frozen at ingestion time.

use std::{
    collections::{BTreeMap, VecDeque},
    ops::Deref,
    time::Duration,
};

use bevy::prelude::*;
use hex_assets::{ElementCatalog, SpellBook};
use hex_combat::{CombatEvent, CombatSystems, CommandRefusal, FactionLatticeKnowledge};
use hex_core::{AppSystems, GameCommand, LatticeCoord, Screen, TilePos, UnitId};
use hex_lattice::{CellKind, LatticeSpec};
use hex_perception::FactionMapKnowledge;
use hex_units::{Faction, Player, StandsOn, UnitRegistry};

use super::lattice::RetainedTarget;
use hex_ui::{
    ActivityKind, ActivityLogLineView, ActivityLogView, ActivityTab, TargetPulseView, UiIntent,
};

const CAPACITY: usize = 64;
const DRAWER_LINES: usize = 12;
const PULSE_SECONDS: f32 = 0.28;
#[derive(Debug, Clone, PartialEq)]
struct LogLine {
    text: String,
    danger: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct RecordedLine {
    sequence: u64,
    line: LogLine,
}

impl Deref for RecordedLine {
    type Target = LogLine;

    fn deref(&self) -> &Self::Target {
        &self.line
    }
}

/// Shared ordering across combat and high-level activity ingestion.
#[derive(Resource, Default)]
struct ActivityChronology {
    next: u64,
}

impl ActivityChronology {
    fn record(&mut self, line: LogLine) -> RecordedLine {
        let recorded = RecordedLine {
            sequence: self.next,
            line,
        };
        self.next = self.next.saturating_add(1);
        recorded
    }
}

#[derive(Resource, Default)]
struct CombatLog {
    lines: VecDeque<RecordedLine>,
    /// Damage outcomes follow a typed cause event. This freezes that stable cause by
    /// source/target pair until the resulting defender answer resolves.
    causes: BTreeMap<(UnitId, UnitId), DamageCause>,
    /// A cause whose authoritative source is not disclosed to the player.
    ///
    /// Keying only by the visible target preserves qualitative Burn de-duplication
    /// without retaining the hidden source identity inside presentation state.
    anonymous_causes: BTreeMap<UnitId, DamageCause>,
}

/// High-level non-combat history kept separate from combat disclosure machinery.
#[derive(Resource, Default)]
struct ActivityHistory {
    lines: VecDeque<RecordedLine>,
}

impl ActivityHistory {
    fn push(&mut self, line: LogLine, chronology: &mut ActivityChronology) {
        self.lines.push_back(chronology.record(line));
        while self.lines.len() > CAPACITY {
            self.lines.pop_front();
        }
    }
}

/// Typed high-level activity emitted by gameplay adapters.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActivityNotice(pub(crate) String);

#[derive(Debug, Clone, PartialEq, Eq)]
enum DamageCause {
    Spell(String),
    Burn,
}

impl CombatLog {
    fn push(&mut self, line: LogLine, chronology: &mut ActivityChronology) {
        self.lines.push_back(chronology.record(line));
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

    /// Advances the pulse whether or not its target is currently focused.
    ///
    /// Focus decides only whether the pulse is painted. Letting it decide whether the
    /// clock advances freezes an old hit when the player looks away and replays it the
    /// next time that target is selected.
    fn tick(&mut self, delta: Duration) -> bool {
        let live = self.timer.as_mut().is_some_and(|timer| {
            timer.tick(delta);
            !timer.is_finished()
        });
        if !live {
            self.target = None;
            self.timer = None;
        }
        live
    }
}

#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
struct SelectedActivityTab(ActivityTab);

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<CombatLog>()
        .init_resource::<ActivityHistory>()
        .init_resource::<ActivityChronology>()
        .init_resource::<DamagePulse>()
        .init_resource::<SelectedActivityTab>()
        .add_message::<ActivityNotice>()
        .add_systems(OnEnter(Screen::Gameplay), reset)
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
            (
                ingest_activity,
                select_tab.after(hex_ui::UiSystems::EmitIntents),
                publish_view,
                pulse_panel,
            )
                .chain()
                .after(ingest)
                .before(hex_ui::UiSystems::Render)
                .run_if(in_state(Screen::Gameplay)),
        );
}

fn reset(
    mut log: ResMut<CombatLog>,
    mut activity: ResMut<ActivityHistory>,
    mut chronology: ResMut<ActivityChronology>,
    mut pulse: ResMut<DamagePulse>,
    mut tab: ResMut<SelectedActivityTab>,
) {
    *log = CombatLog::default();
    *activity = ActivityHistory::default();
    *chronology = ActivityChronology::default();
    activity.push(
        LogLine {
            text: "Entered the battlefield.".to_owned(),
            danger: false,
        },
        &mut chronology,
    );
    *pulse = DamagePulse::default();
    *tab = SelectedActivityTab::default();
}

fn ingest_activity(
    mut notices: MessageReader<ActivityNotice>,
    mut activity: ResMut<ActivityHistory>,
    mut chronology: ResMut<ActivityChronology>,
) {
    for notice in notices.read() {
        activity.push(
            LogLine {
                text: notice.0.clone(),
                danger: false,
            },
            &mut chronology,
        );
    }
}

fn select_tab(mut intents: MessageReader<UiIntent>, mut selected: ResMut<SelectedActivityTab>) {
    for intent in intents.read() {
        if let UiIntent::Activity(hex_ui::ActivityIntent::SelectTab(tab)) = intent {
            selected.0 = *tab;
        }
    }
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
    mut chronology: ResMut<ActivityChronology>,
    mut pulse: ResMut<DamagePulse>,
    registry: Res<UnitRegistry>,
    identities: IdentityQuery,
    spatial: Option<Res<FactionMapKnowledge>>,
    own_lattices: Query<(&UnitId, &LatticeSpec), With<Player>>,
    knowledge: Res<FactionLatticeKnowledge>,
    elements: Option<Res<ElementCatalog>>,
    spells: Option<Res<SpellBook>>,
) {
    for event in events.read() {
        if !event_is_disclosed(
            Faction::Player,
            event,
            &registry,
            &identities,
            spatial.as_deref(),
        ) {
            clear_resolved_cause(event, &mut log);
            continue;
        }
        remember_cause(
            Faction::Player,
            event,
            &mut log,
            &registry,
            &identities,
            spatial.as_deref(),
        );
        if let Some(target) = pulse_target(event) {
            pulse.start(target);
        }
        if let Some(line) = format_event(
            Faction::Player,
            event,
            &log,
            &registry,
            &identities,
            spatial.as_deref(),
            &own_lattices,
            &knowledge,
            elements.as_deref(),
            spells.as_deref(),
        ) {
            log.push(line, &mut chronology);
        }
        clear_resolved_cause(event, &mut log);
    }
}

fn clear_resolved_cause(event: &CombatEvent, log: &mut CombatLog) {
    if let CombatEvent::HexesDisabled { source, target, .. } = event {
        log.causes.remove(&(*source, *target));
        log.anonymous_causes.remove(target);
    }
}

/// Whether the subject exposed by this presentation event is currently authorized.
///
/// Own identities are durable knowledge and do not depend on the current sight map.
/// Every other subject must be in the viewer's current unit projection. Hidden sources
/// do not suppress outcomes on an authorized subject; source formatting redacts them
/// separately. Missing perception or identity data therefore hides a hostile outcome
/// instead of falling back to authoritative registry truth.
fn event_is_disclosed(
    viewer: Faction,
    event: &CombatEvent,
    registry: &UnitRegistry,
    identities: &IdentityQuery,
    spatial: Option<&FactionMapKnowledge>,
) -> bool {
    let unit = |id| unit_is_disclosed(viewer, id, registry, identities, spatial);
    match event {
        CombatEvent::TurnAdvanced { unit: actor, .. } => unit(*actor),
        CombatEvent::Cast { caster, .. } => unit(*caster),
        CombatEvent::Channelled {
            unit: channeler, ..
        } => unit(*channeler),
        CombatEvent::Strike { target, .. } => unit(*target),
        CombatEvent::DecisionOpened { decider, .. } => unit(*decider),
        CombatEvent::DamagePrevented { target, .. }
        | CombatEvent::HexesDisabled { target, .. }
        | CombatEvent::BurnApplied { target, .. }
        | CombatEvent::BurnTicked { target, .. } => unit(*target),
        CombatEvent::EnchantmentBroken { unit: target, .. }
        | CombatEvent::Downed { unit: target }
        | CombatEvent::Revived { unit: target, .. }
        | CombatEvent::Rested { unit: target, .. } => unit(*target),
        CombatEvent::Revealed {
            viewer: recipient,
            subject,
            ..
        } => *recipient == viewer && unit(*subject),
        CombatEvent::PartyMoved { anchor, paths } => {
            unit(*anchor) && paths.iter().all(|path| unit(path.member))
        }
        CombatEvent::HexesRestored { target, .. } => unit(*target),
        CombatEvent::EncounterResolved { .. } => true,
        CombatEvent::CommandRefused { command, .. } => unit(command.unit()),
    }
}

fn unit_is_disclosed(
    viewer: Faction,
    unit: UnitId,
    registry: &UnitRegistry,
    identities: &IdentityQuery,
    spatial: Option<&FactionMapKnowledge>,
) -> bool {
    let Some(faction) = registry
        .entity_of(unit)
        .and_then(|entity| identities.get(entity).ok())
        .and_then(|(_, faction, _)| faction.copied())
    else {
        return false;
    };
    faction == viewer
        || spatial.is_some_and(|knowledge| {
            knowledge
                .faction(viewer)
                .unit(unit)
                .is_some_and(|observed| observed.faction == faction)
        })
}

fn remember_cause(
    viewer: Faction,
    event: &CombatEvent,
    log: &mut CombatLog,
    registry: &UnitRegistry,
    identities: &IdentityQuery,
    spatial: Option<&FactionMapKnowledge>,
) {
    match event {
        CombatEvent::Cast {
            caster,
            spell,
            target,
        } => {
            if let Some(subject) = unit_at(*target, registry, identities).filter(|subject| {
                unit_is_disclosed(viewer, *subject, registry, identities, spatial)
            }) {
                log.anonymous_causes.remove(&subject);
                log.causes
                    .insert((*caster, subject), DamageCause::Spell(spell.clone()));
            }
        }
        CombatEvent::Strike { attacker, target } => {
            log.causes.remove(&(*attacker, *target));
            log.anonymous_causes.remove(target);
        }
        CombatEvent::BurnTicked { source, target, .. } => {
            if unit_is_disclosed(viewer, *source, registry, identities, spatial) {
                log.anonymous_causes.remove(target);
                log.causes.insert((*source, *target), DamageCause::Burn);
            } else {
                log.anonymous_causes.insert(*target, DamageCause::Burn);
            }
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
    viewer: Faction,
    event: &CombatEvent,
    log: &CombatLog,
    registry: &UnitRegistry,
    identities: &IdentityQuery,
    spatial: Option<&FactionMapKnowledge>,
    own_lattices: &Query<(&UnitId, &LatticeSpec), With<Player>>,
    knowledge: &FactionLatticeKnowledge,
    elements: Option<&ElementCatalog>,
    spells: Option<&SpellBook>,
) -> Option<LogLine> {
    let line = match event {
        // Turn boundaries are canonical report facts, but the ordinary combat log
        // already communicates initiative and should not grow one line per turn.
        CombatEvent::TurnAdvanced { .. } => return None,
        CombatEvent::Cast { caster, spell, .. } => LogLine {
            text: format!("{} cast {spell}", unit_name(*caster, registry, identities)),
            danger: false,
        },
        CombatEvent::Channelled { unit, restored } => {
            let detail = restored
                .iter()
                .map(|(element, amount)| format!("{element} +{amount}"))
                .collect::<Vec<_>>()
                .join(", ");
            let suffix = if detail.is_empty() {
                "no mana restored".to_owned()
            } else {
                detail
            };
            LogLine {
                text: format!(
                    "{} channelled ({suffix})",
                    unit_name(*unit, registry, identities)
                ),
                danger: false,
            }
        }
        CombatEvent::Strike { attacker, target } => LogLine {
            text: format!(
                "{} struck {}",
                source_name(viewer, *attacker, registry, identities, spatial),
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
                cause_name(viewer, *source, *target, log, registry, identities, spatial)
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
            let actor = cause_name(viewer, *source, *target, log, registry, identities, spatial);
            let subject = unit_name(*target, registry, identities);
            let text = if known.is_empty() {
                // `BurnTicked` already emitted the legally safe qualitative line. An
                // opaque exact answer has no further information to add.
                if matches!(damage_cause(log, *source, *target), Some(DamageCause::Burn)) {
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
                cause_name(viewer, *source, *target, log, registry, identities, spatial),
                unit_name(*target, registry, identities)
            ),
            danger: false,
        },
        CombatEvent::BurnTicked { target, .. } => LogLine {
            text: format!("Burn damaged {}", unit_name(*target, registry, identities)),
            danger: true,
        },
        CombatEvent::Revealed {
            viewer: recipient,
            subject,
            ..
        } => {
            if *recipient != viewer {
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
        CombatEvent::PartyMoved { paths, .. } => LogLine {
            text: format!("Party moved in formation ({} members)", paths.len()),
            danger: false,
        },
        CombatEvent::HexesRestored { target, cells, .. } => LogLine {
            text: format!(
                "{} restored {} lattice cells",
                unit_name(*target, registry, identities),
                cells.len()
            ),
            danger: false,
        },
        CombatEvent::Revived { unit, .. } => LogLine {
            text: format!("{} was revived", unit_name(*unit, registry, identities)),
            danger: false,
        },
        CombatEvent::Rested { unit, .. } => LogLine {
            text: format!(
                "{} recovered during rest",
                unit_name(*unit, registry, identities)
            ),
            danger: false,
        },
        CombatEvent::EncounterResolved { outcome } => LogLine {
            text: format!("Encounter resolved: {outcome:?}"),
            danger: false,
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
    viewer: Faction,
    source: UnitId,
    target: UnitId,
    log: &CombatLog,
    registry: &UnitRegistry,
    identities: &IdentityQuery,
    spatial: Option<&FactionMapKnowledge>,
) -> String {
    match damage_cause(log, source, target) {
        Some(DamageCause::Spell(spell)) => spell.clone(),
        Some(DamageCause::Burn) => "Burn".to_owned(),
        None => source_name(viewer, source, registry, identities, spatial),
    }
}

fn damage_cause(log: &CombatLog, source: UnitId, target: UnitId) -> Option<&DamageCause> {
    log.causes
        .get(&(source, target))
        .or_else(|| log.anonymous_causes.get(&target))
}

fn source_name(
    viewer: Faction,
    source: UnitId,
    registry: &UnitRegistry,
    identities: &IdentityQuery,
    spatial: Option<&FactionMapKnowledge>,
) -> String {
    if unit_is_disclosed(viewer, source, registry, identities, spatial) {
        unit_name(source, registry, identities)
    } else {
        "Unknown source".to_owned()
    }
}

fn disclosed_cells(
    target: UnitId,
    cells: &[LatticeCoord],
    registry: &UnitRegistry,
    identities: &IdentityQuery,
    own_lattices: &Query<(&UnitId, &LatticeSpec), With<Player>>,
    knowledge: &FactionLatticeKnowledge,
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
        GameCommand::MoveParty { .. } => "move the party",
        GameCommand::Strike { .. } => "strike",
        GameCommand::EndTurn { .. } => "end the turn",
        GameCommand::Cast { .. } => "cast",
        GameCommand::Channel { .. } => "channel",
        GameCommand::ChooseDisables { .. } => "choose damaged cells",
        GameCommand::ChooseRestores { .. } => "choose restored cells",
        GameCommand::Rest { .. } => "rest",
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
        CommandRefusal::Occupied { block } => match block {
            hex_units::OccupancyBlock::Destination { .. } => "destination occupied",
            hex_units::OccupancyBlock::Route { .. } => "route occupied",
        },
        CommandRefusal::MovementBudgetExceeded { .. } => "not enough movement",
        CommandRefusal::UnknownTarget { .. } => "unknown target",
        CommandRefusal::TargetDowned { .. } => "target is already down",
        CommandRefusal::TargetNotHostile { .. } => "target is not hostile",
        CommandRefusal::TargetOutOfMeleeReach { .. } => "target is out of reach",
        CommandRefusal::TargetUnoccupied { .. } => "target has no unit",
        CommandRefusal::TargetOutOfTouchReach { .. } => "target is out of touch reach",
        CommandRefusal::NoTurn => "no active turn",
        CommandRefusal::ActionAlreadySpent => "action already spent",
        CommandRefusal::ActingUnitDowned { .. } => "acting unit is down",
        CommandRefusal::UnknownSpell { .. } => "unknown spell",
        CommandRefusal::MissingSpellDefinition { .. } => "spell definition unavailable",
        CommandRefusal::UndeliverableSpell { .. } => "spell is not implemented",
        CommandRefusal::MissingFacing { .. } => "direction required",
        CommandRefusal::TargetOutOfRange { .. } => "target is out of range",
        CommandRefusal::ShapeUnresolved { .. } => "shape could not resolve",
        CommandRefusal::TargetUnobserved { .. } => "target is unobserved",
        CommandRefusal::TrajectoryBlocked { .. } => "spell trajectory is blocked",
        CommandRefusal::TerrainCreationBlocked { .. } => "terrain creation is blocked",
        CommandRefusal::SpellNotInscribed { .. } => "spell is not inscribed",
        CommandRefusal::CastBlocked { .. } => "lattice cannot pay",
        CommandRefusal::CastPlanStale { .. } => "lattice changed",
        CommandRefusal::PartyMovementUnavailable => "party movement is unavailable",
        CommandRefusal::RestorationUnavailable => "restoration is unavailable",
        CommandRefusal::RestUnavailable => "rest is unavailable",
        CommandRefusal::PartyMove { .. } => "party move is invalid",
        CommandRefusal::Restoration { .. } => "restoration answer is invalid",
        CommandRefusal::RestorationTarget { .. } => "restoration target is invalid",
        CommandRefusal::RestExploringOnly => "rest is exploration only",
        CommandRefusal::EncounterResolved { .. } => "the encounter is resolved",
        CommandRefusal::NoPendingDecision => "no decision is open",
        CommandRefusal::WrongDecisionUnit { .. } => "another unit must decide",
        CommandRefusal::WrongDisableCount { .. } => "wrong number of cells",
        CommandRefusal::CellOutsideLattice { .. } => "cell is outside the lattice",
        CommandRefusal::DuplicateCell { .. } => "cell was chosen twice",
        CommandRefusal::CellAlreadyDisabled { .. } => "cell is already disabled",
    }
}

fn publish_view(
    log: Res<CombatLog>,
    activity: Res<ActivityHistory>,
    selected: Res<SelectedActivityTab>,
    mut view: ResMut<ActivityLogView>,
) {
    if !log.is_changed() && !activity.is_changed() && !selected.is_changed() {
        return;
    }
    let mut lines = match selected.0 {
        ActivityTab::All => {
            let mut combined = activity
                .lines
                .iter()
                .map(|line| (ActivityKind::Activity, line))
                .chain(log.lines.iter().map(|line| (ActivityKind::Combat, line)))
                .collect::<Vec<_>>();
            combined.sort_by_key(|(_, line)| line.sequence);
            combined
        }
        ActivityTab::Combat => log
            .lines
            .iter()
            .map(|line| (ActivityKind::Combat, line))
            .collect(),
        ActivityTab::Activity => activity
            .lines
            .iter()
            .map(|line| (ActivityKind::Activity, line))
            .collect(),
    };
    let keep_from = lines.len().saturating_sub(DRAWER_LINES);
    lines.drain(0..keep_from);
    let count = match selected.0 {
        ActivityTab::All => log.lines.len() + activity.lines.len(),
        ActivityTab::Combat => log.lines.len(),
        ActivityTab::Activity => activity.lines.len(),
    };
    let next = ActivityLogView {
        heading: format!("ACTIVITY · {count} EVENTS"),
        tab: selected.0,
        lines: lines
            .into_iter()
            .map(|(kind, line)| ActivityLogLineView {
                kind,
                text: line.text.clone(),
                danger: line.danger,
            })
            .collect(),
    };
    if *view != next {
        *view = next;
    }
}

#[cfg(test)]
fn visible_lines(log: &CombatLog, visible: usize) -> Vec<&LogLine> {
    log.lines
        .iter()
        .rev()
        .take(visible)
        .map(|recorded| &recorded.line)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn pulse_panel(
    time: Res<Time>,
    focus: Res<RetainedTarget>,
    mut pulse: ResMut<DamagePulse>,
    mut view: ResMut<TargetPulseView>,
) {
    let live = pulse.tick(time.delta());
    let next = TargetPulseView(live && pulse.target == focus.unit);
    if *view != next {
        *view = next;
    }
}

#[cfg(test)]
mod tests {
    use bevy::MinimalPlugins;
    use hex_combat::{BaseVisibility, KnownCell};
    use hex_core::{
        ElementId, Headroom, HexCoord, HexSpan, KnowledgeExpiry, KnowledgeSource, LightDomain,
        SubstanceId,
    };
    use hex_perception::{
        apply_observations, FactionObservation, FactionObservations, ObservedUnit, SurfaceSnapshot,
        SurfaceSnapshots,
    };

    use super::*;

    fn knowledge_observing(viewer: Faction, unit: UnitId, faction: Faction) -> FactionMapKnowledge {
        let pos = TilePos::new(HexCoord::ORIGIN, 1);
        let current = SurfaceSnapshots::try_from_iter([SurfaceSnapshot {
            pos,
            span: HexSpan::new(0.0, 1.0),
            substance: SubstanceId(0),
            headroom: Headroom(2),
            is_solid: true,
            blocked: false,
            domain: LightDomain::Exterior,
        }])
        .expect("the fixture has one unique surface");
        let mut observation = FactionObservation::new();
        observation.insert_surface(pos);
        observation
            .try_insert_unit(ObservedUnit {
                id: unit,
                faction,
                pos,
                provides_sight: true,
            })
            .expect("the fixture has one unique unit");
        let observations = FactionObservations::with_faction(viewer, observation);
        let mut knowledge = FactionMapKnowledge::new();
        apply_observations(&mut knowledge, &current, &observations);
        knowledge
    }

    #[test]
    fn the_log_keeps_sixty_four_entries() {
        let mut log = CombatLog::default();
        let mut chronology = ActivityChronology::default();
        for number in 0..80 {
            log.push(
                LogLine {
                    text: number.to_string(),
                    danger: false,
                },
                &mut chronology,
            );
        }
        assert_eq!(log.lines.len(), CAPACITY);
        assert_eq!(log.lines.front().map(|line| line.text.as_str()), Some("16"));
        assert_eq!(log.lines.back().map(|line| line.text.as_str()), Some("79"));
    }

    #[test]
    fn the_feed_is_three_lines_without_discarding_drawer_history() {
        let mut log = CombatLog::default();
        let mut chronology = ActivityChronology::default();
        for number in 0..8 {
            log.push(
                LogLine {
                    text: number.to_string(),
                    danger: number == 6,
                },
                &mut chronology,
            );
        }

        let feed: Vec<_> = visible_lines(&log, 3)
            .iter()
            .map(|line| line.text.as_str())
            .collect();
        let drawer: Vec<_> = visible_lines(&log, DRAWER_LINES)
            .iter()
            .map(|line| line.text.as_str())
            .collect();
        assert_eq!(feed, ["5", "6", "7"]);
        assert_eq!(drawer, ["0", "1", "2", "3", "4", "5", "6", "7"]);
        assert!(log.lines.get(6).is_some_and(|line| line.danger));
        assert_eq!(log.lines.len(), 8, "opening the drawer is non-destructive");
    }

    #[test]
    fn all_tab_keeps_late_activity_after_dense_combat_history() {
        let mut chronology = ActivityChronology::default();
        let mut activity = ActivityHistory::default();
        let mut log = CombatLog::default();
        activity.push(
            LogLine {
                text: "entered".to_owned(),
                danger: false,
            },
            &mut chronology,
        );
        for number in 0..12 {
            log.push(
                LogLine {
                    text: format!("combat {number}"),
                    danger: false,
                },
                &mut chronology,
            );
        }
        activity.push(
            LogLine {
                text: "formation changed".to_owned(),
                danger: false,
            },
            &mut chronology,
        );

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(log)
            .insert_resource(activity)
            .init_resource::<SelectedActivityTab>()
            .init_resource::<ActivityLogView>()
            .add_systems(Update, publish_view);
        app.update();

        let view = app.world().resource::<ActivityLogView>();
        assert_eq!(view.lines.len(), DRAWER_LINES);
        assert_eq!(
            view.lines
                .last()
                .map(|line| (line.kind, line.text.as_str())),
            Some((ActivityKind::Activity, "formation changed"))
        );
        assert_eq!(
            view.lines
                .iter()
                .filter(|line| line.text == "formation changed")
                .count(),
            1
        );
    }

    #[test]
    fn a_pulse_expires_while_the_player_is_focused_elsewhere() {
        let target = UnitId(1);
        let mut pulse = DamagePulse::default();
        pulse.start(target);

        let focused_elsewhere = Some(UnitId(2));
        let live = pulse.tick(Duration::from_secs_f32(PULSE_SECONDS + 0.01));
        assert!(!live, "elapsed time must expire an unfocused pulse");
        assert_ne!(pulse.target, focused_elsewhere);
        assert_eq!(
            pulse.target, None,
            "refocusing the old target later must not replay the pulse"
        );
    }

    #[test]
    fn disclosure_is_frozen_and_partial_knowledge_names_only_known_cells() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<CombatEvent>()
            .init_resource::<CombatLog>()
            .init_resource::<ActivityChronology>()
            .init_resource::<DamagePulse>()
            .init_resource::<UnitRegistry>()
            .init_resource::<FactionLatticeKnowledge>()
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
            .resource_mut::<FactionLatticeKnowledge>()
            .observe_base(
                Faction::Player,
                target,
                BaseVisibility {
                    faction: Faction::Hostile,
                },
            );
        app.insert_resource(knowledge_observing(
            Faction::Player,
            target,
            Faction::Hostile,
        ));

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

        assert!(app
            .world_mut()
            .resource_mut::<FactionLatticeKnowledge>()
            .learn(
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
            .init_resource::<ActivityChronology>()
            .init_resource::<DamagePulse>()
            .init_resource::<UnitRegistry>()
            .init_resource::<FactionLatticeKnowledge>()
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
            .resource_mut::<FactionLatticeKnowledge>()
            .observe_base(
                Faction::Player,
                target,
                BaseVisibility {
                    faction: Faction::Hostile,
                },
            );
        app.insert_resource(knowledge_observing(
            Faction::Player,
            target,
            Faction::Hostile,
        ));
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
        assert!(log.anonymous_causes.is_empty());
    }

    #[test]
    fn hidden_hostile_outcomes_create_neither_log_lines_nor_pulses() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<CombatEvent>()
            .init_resource::<CombatLog>()
            .init_resource::<ActivityChronology>()
            .init_resource::<DamagePulse>()
            .init_resource::<UnitRegistry>()
            .init_resource::<FactionLatticeKnowledge>()
            .insert_resource(FactionMapKnowledge::new())
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

        let outcomes = || {
            [
                CombatEvent::BurnApplied {
                    source,
                    target,
                    turns: 2,
                },
                CombatEvent::HexesDisabled {
                    source,
                    target,
                    cells: vec![LatticeCoord::ORIGIN],
                },
                CombatEvent::BurnTicked {
                    source,
                    target,
                    count: 1,
                },
                CombatEvent::Downed { unit: target },
            ]
        };
        for event in outcomes() {
            app.world_mut()
                .resource_mut::<Messages<CombatEvent>>()
                .write(event);
        }
        app.update();

        assert!(
            app.world().resource::<CombatLog>().lines.is_empty(),
            "authoritative hidden spillover outcomes must not enter player history"
        );
        let pulse = app.world().resource::<DamagePulse>();
        assert_eq!(pulse.target, None);
        assert_eq!(pulse.timer, None);
        assert!(
            app.world().resource::<CombatLog>().causes.is_empty()
                && app
                    .world()
                    .resource::<CombatLog>()
                    .anonymous_causes
                    .is_empty(),
            "presentation bookkeeping must not retain hidden identities either"
        );

        app.insert_resource(knowledge_observing(
            Faction::Player,
            target,
            Faction::Hostile,
        ));
        for event in outcomes() {
            app.world_mut()
                .resource_mut::<Messages<CombatEvent>>()
                .write(event);
        }
        app.update();

        let log = app.world().resource::<CombatLog>();
        let lines: Vec<_> = log.lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(
            lines,
            [
                "hedge-mage #1 ignited wolf #9",
                "hedge-mage #1 damaged wolf #9",
                "Burn damaged wolf #9",
                "wolf #9 went down",
            ],
            "the same outcomes become presentable while the hostile is observed"
        );
        let pulse = app.world().resource::<DamagePulse>();
        assert_eq!(pulse.target, Some(target));
        assert!(pulse.timer.is_some());
    }

    #[test]
    fn a_hidden_source_is_redacted_without_hiding_player_owned_outcomes() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<CombatEvent>()
            .init_resource::<CombatLog>()
            .init_resource::<ActivityChronology>()
            .init_resource::<DamagePulse>()
            .init_resource::<UnitRegistry>()
            .init_resource::<FactionLatticeKnowledge>()
            .insert_resource(FactionMapKnowledge::new())
            .add_systems(Update, ingest);

        let source = UnitId(9);
        let target = UnitId(1);
        let source_entity = app
            .world_mut()
            .spawn((source, Name::new("hidden pyromancer #9"), Faction::Hostile))
            .id();
        let target_entity = app
            .world_mut()
            .spawn((target, Name::new("hero #1"), Faction::Player, Player))
            .id();
        {
            let mut registry = app.world_mut().resource_mut::<UnitRegistry>();
            registry.register(source, source_entity);
            registry.register(target, target_entity);
        }

        let events = [
            CombatEvent::Cast {
                caster: source,
                spell: "Wildfire".to_owned(),
                target: TilePos::new(HexCoord::ORIGIN, 1),
            },
            CombatEvent::BurnApplied {
                source,
                target,
                turns: 2,
            },
            CombatEvent::BurnTicked {
                source,
                target,
                count: 1,
            },
            CombatEvent::HexesDisabled {
                source,
                target,
                cells: vec![LatticeCoord::ORIGIN],
            },
            CombatEvent::Downed { unit: target },
        ];
        for event in events {
            app.world_mut()
                .resource_mut::<Messages<CombatEvent>>()
                .write(event);
        }
        app.update();

        let log = app.world().resource::<CombatLog>();
        let lines: Vec<_> = log.lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(
            lines,
            [
                "Unknown source ignited hero #1",
                "Burn damaged hero #1",
                "hero #1 went down",
            ],
            "own outcomes stay visible but neither the hidden cast nor source identity leaks"
        );
        assert!(
            lines.iter().all(|line| !line.contains("pyromancer")),
            "{lines:?}"
        );
        assert!(
            log.causes.is_empty() && log.anonymous_causes.is_empty(),
            "the disable resolution must clear both disclosed and anonymous attribution"
        );
        let pulse = app.world().resource::<DamagePulse>();
        assert_eq!(pulse.target, Some(target));
        assert!(pulse.timer.is_some());
    }
}
