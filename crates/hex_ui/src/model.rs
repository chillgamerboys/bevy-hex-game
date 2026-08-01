use bevy::prelude::*;
use hex_assets::Scenario;
use hex_core::{GameplayPhase, UnitId};

/// Whether an action can currently be taken, with the canonical refusal when it cannot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionAvailability {
    /// The action is currently legal.
    Enabled,
    /// The action is visible but cannot currently be taken.
    Disabled {
        /// Canonical, player-facing reason supplied by the application adapter.
        reason: String,
    },
}

/// Placement priority inside the persistent action rail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ActionPriority {
    /// Secondary convenience or inspection action.
    Secondary,
    /// Ordinary legal turn action.
    Primary,
    /// A blocking choice that must be resolved before play continues.
    Required,
}

/// Gameplay action identities understood by the application composition root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameplayAction {
    /// Restore mana through the canonical combat command funnel.
    Channel,
    /// Yield the active combat turn.
    EndTurn,
    /// Rest the exploring party.
    Rest,
    /// Toggle the pause overlay.
    Pause,
    /// Confirm the currently required lattice decision.
    ConfirmDecision,
}

/// One application-authorized action rendered by the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionAffordance {
    /// Typed action returned in [`UiIntent::Gameplay`].
    pub action: GameplayAction,
    /// Player-facing verb.
    pub label: String,
    /// Current binding rendered beside the verb, when one exists.
    pub shortcut: Option<String>,
    /// Canonical availability and refusal reason.
    pub availability: ActionAvailability,
    /// Visual and focus priority.
    pub priority: ActionPriority,
}

/// Immutable gameplay HUD projection supplied by `hex_game`.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct GameplayHudView {
    /// Current application phase.
    pub phase: GameplayPhase,
    /// Current actor, if the authoritative turn model names one.
    pub actor: Option<UnitId>,
    /// Player-facing actor label with disclosure already applied.
    pub actor_label: String,
    /// Current round label.
    pub round: String,
    /// Remaining movement budget.
    pub movement_remaining: u32,
    /// Whether the actor retains its action.
    pub action_remaining: bool,
    /// Guidance for the current required choice, if any.
    pub required_prompt: Option<String>,
    /// Current authorized actions.
    pub actions: Vec<ActionAffordance>,
}

/// Immutable visibility projection for gameplay chrome.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameplayChromeView {
    /// Whether ordinary HUD surfaces are shown.
    pub shown: bool,
    /// Whether a command-modal decision must remain reachable.
    pub decision_required: bool,
    /// Whether terminal encounter presentation supersedes stale decisions.
    pub encounter_complete: bool,
}

impl Default for GameplayChromeView {
    fn default() -> Self {
        Self {
            shown: true,
            decision_required: false,
            encounter_complete: false,
        }
    }
}

/// One disclosure-frozen combat history line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombatLogLineView {
    /// Player-facing event description.
    pub text: String,
    /// Whether the line receives danger emphasis in addition to its wording.
    pub danger: bool,
}

/// Immutable visible portion of the combat history.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct CombatLogView {
    /// Drawer/feed heading including its keyboard affordance.
    pub heading: String,
    /// Already-filtered visible lines in chronological order.
    pub lines: Vec<CombatLogLineView>,
}

/// Semantic unit-badge treatment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadgeKind {
    /// Current initiative actor.
    Acting,
    /// Current disclosed target.
    Target,
}

/// One screen-space identity badge projection.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitBadgeView {
    /// Canonical identity retained for immutable observation.
    pub unit: UnitId,
    /// Presentation role.
    pub kind: BadgeKind,
    /// Disclosure-safe player-facing label.
    pub label: String,
    /// Projected viewport anchor, or `None` when the unit is offscreen.
    pub anchor: Option<Vec2>,
}

/// Immutable acting and target badge projections.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct UnitBadgesView {
    /// Acting-unit badge.
    pub acting: Option<UnitBadgeView>,
    /// Target badge.
    pub target: Option<UnitBadgeView>,
}

/// Progress for a required damage/restoration lattice choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecisionChoiceView {
    /// Selected cell count.
    pub chosen: usize,
    /// Exact required count.
    pub owed: usize,
    /// Whether disabled cells are being restored rather than live cells disabled.
    pub restoring: bool,
}

/// Player-owned lattice panel projection.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnLatticeView {
    /// Semantic inspector role, already converted to player-facing text.
    pub heading: String,
    /// Disclosure-safe unit identity.
    pub identity: String,
    /// Renderable cell projections.
    pub cells: Vec<crate::LatticeCellView>,
    /// Required choice progress, when this lattice owns it.
    pub decision: Option<DecisionChoiceView>,
}

/// Knowledge-gated hostile lattice state.
#[derive(Debug, Clone, PartialEq)]
pub enum TargetLatticeStateView {
    /// Existence is known but no cells are disclosed.
    Opaque,
    /// Disclosed cells plus a count of still-unknown cells.
    Known {
        /// Renderable disclosed cells.
        cells: Vec<crate::LatticeCellView>,
        /// Count of cells not disclosed by knowledge.
        unknown: Option<usize>,
    },
}

/// Hostile target lattice panel projection.
#[derive(Debug, Clone, PartialEq)]
pub struct TargetLatticeView {
    /// Target provenance, already converted to player-facing text.
    pub heading: String,
    /// Disclosure-safe target identity.
    pub identity: String,
    /// Knowledge-gated lattice state.
    pub state: TargetLatticeStateView,
}

/// Immutable gameplay lattice presentation.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct GameplayLatticesView {
    /// Selected/acting/deciding player lattice.
    pub own: Option<OwnLatticeView>,
    /// Retained hostile target lattice.
    pub target: Option<TargetLatticeView>,
}

/// Short-lived target-panel damage emphasis.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TargetPulseView(pub bool);

/// One spell action shown by the casting panel.
#[derive(Debug, Clone, PartialEq)]
pub struct CastingSpellView {
    /// Stable spell name used by the returned intent.
    pub name: String,
    /// Player-facing cost, range, and shape summary.
    pub cost: String,
    /// Canonical preflight refusal, when the lattice cannot pay.
    pub blocked: Option<String>,
    /// Element presentation tint.
    pub color: Color,
}

/// Current aim summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CastingAimView {
    /// Player-facing spell and resolved-volume summary.
    pub label: String,
    /// Whether confirm/next/cancel controls are currently legal to offer.
    pub controls_enabled: bool,
}

/// Mutually exclusive casting-panel presentation states.
#[derive(Debug, Clone, PartialEq)]
pub enum CastingPanelContentView {
    /// Status or refusal with optional turn controls.
    Message {
        /// Player-facing status.
        text: String,
        /// Whether Channel and End Turn remain available beside it.
        turn_controls: bool,
    },
    /// Required lattice choice.
    Decision {
        /// Player-facing owner/target context.
        prompt: String,
        /// Exact choice progress.
        choice: DecisionChoiceView,
    },
    /// Ordinary spells or an aim in flight.
    Spells {
        /// Panel-wide refusal that temporarily suspends every spell.
        unavailable: Option<String>,
        /// Stable spell actions.
        spells: Vec<CastingSpellView>,
        /// Active aim, when one replaces the spell list.
        aiming: Option<CastingAimView>,
    },
}

impl Default for CastingPanelContentView {
    fn default() -> Self {
        Self::Message {
            text: "no unit to cast from".to_owned(),
            turn_controls: true,
        }
    }
}

/// Immutable casting-panel projection.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct CastingPanelView {
    /// Whether combat mode makes the panel relevant.
    pub visible: bool,
    /// Complete panel content.
    pub content: CastingPanelContentView,
}

/// Typed casting actions emitted by presentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CastingIntent {
    /// Begin aiming the named projected spell.
    Begin(String),
    /// Confirm the current aim.
    Confirm,
    /// Cycle to the next legal target.
    NextTarget,
    /// Cancel the current aim.
    Cancel,
}

/// One stable party-member row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartyMemberView {
    /// Zero-based party slot returned by activation.
    pub slot: usize,
    /// Complete non-color status label.
    pub label: String,
    /// Whether this member owns the current turn.
    pub active: bool,
    /// Whether this member is selected for movement/formation work.
    pub selected: bool,
}

/// One authored slot in the selected formation preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormationSlotView {
    /// Canonical relative offset.
    pub offset: hex_core::HexCoord,
    /// Whether this is the formation anchor.
    pub anchor: bool,
}

/// Immutable party and formation presentation.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct PartyView {
    /// Existing party members in stable slot order.
    pub members: Vec<PartyMemberView>,
    /// Whether exploration-only formation controls are visible.
    pub formation_visible: bool,
    /// Current movement-mode label.
    pub movement_mode: String,
    /// Available preset names.
    pub presets: Vec<String>,
    /// Slots authored by the selected preset.
    pub slots: Vec<FormationSlotView>,
}

/// Typed party/formation actions emitted by presentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartyIntent {
    /// Select one stable party slot.
    SelectMember(usize),
    /// Toggle group/solo movement.
    ToggleMovementMode,
    /// Select an authored formation preset.
    SelectPreset(String),
    /// Assign the selected unit to an authored relative slot.
    AssignSlot(hex_core::HexCoord),
    /// Rest through the canonical command funnel.
    Rest,
}

/// Immutable live Combat Lab statistics presentation.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct LabStatisticsView {
    /// Whether the current gameplay run belongs to Combat Lab.
    pub present: bool,
    /// Whether the encounter is still active and the drawer may be shown.
    pub visible: bool,
    /// Whether the secondary statistics body is expanded.
    pub expanded: bool,
    /// Canonical, already-formatted combat summary.
    pub text: String,
}

impl Default for LabStatisticsView {
    fn default() -> Self {
        Self {
            present: false,
            visible: false,
            expanded: true,
            text: "Waiting for canonical combat statistics…".to_owned(),
        }
    }
}

/// Typed actions available from the live Combat Lab statistics drawer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabStatisticsIntent {
    /// Expand or collapse the secondary statistics body.
    Toggle,
    /// Freeze the current run as a manual-stop report.
    EndExperiment,
}

/// One selectable saved-report identity in Compare mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeCompareChoiceView {
    /// Stable report identity owned by the gameplay model.
    pub id: hex_gameplay_model::CombatLabReportId,
    /// Complete player-facing selector label.
    pub label: String,
    /// Whether this report is the current comparison target.
    pub selected: bool,
}

/// Outcome actions whose effects remain application-owned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeAction {
    /// Continue after victory.
    Continue,
    /// Retry the active non-Lab scenario.
    Retry,
    /// Retry the exact frozen Lab launch.
    RetryExact,
    /// Restore the frozen Lab run for tuning.
    TuneAgain,
    /// Copy a fixed fixture into the editable sandbox.
    CopyToSandbox,
    /// Persist the frozen report.
    SaveReport,
    /// Return to the session's owning screen.
    Return,
}

/// One ordered outcome-footer action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeActionView {
    /// Typed action returned to the application adapter.
    pub action: OutcomeAction,
    /// Player-facing verb.
    pub label: String,
}

/// Immutable encounter outcome and optional Combat Lab report presentation.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct OutcomeReportView {
    /// Whether an encounter outcome currently blocks gameplay.
    pub visible: bool,
    /// Outcome heading.
    pub title: String,
    /// Short outcome guidance.
    pub detail: String,
    /// Frozen run identity and fingerprint, when this is a Lab run.
    pub metadata: Option<String>,
    /// Active report mode.
    pub mode: hex_gameplay_model::ReportMode,
    /// Already-formatted, gameplay-owned report body.
    pub body: Option<String>,
    /// Independent saved-report choices.
    pub comparisons: Vec<OutcomeCompareChoiceView>,
    /// Ordered footer actions.
    pub actions: Vec<OutcomeActionView>,
}

impl Default for OutcomeReportView {
    fn default() -> Self {
        Self {
            visible: false,
            title: String::new(),
            detail: String::new(),
            metadata: None,
            mode: hex_gameplay_model::ReportMode::Overview,
            body: None,
            comparisons: Vec::new(),
            actions: Vec::new(),
        }
    }
}

/// Typed outcome/report controls emitted by presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeIntent {
    /// Select one report presentation mode.
    SelectMode(hex_gameplay_model::ReportMode),
    /// Select one saved report without changing the active mode implicitly.
    CompareWith(hex_gameplay_model::CombatLabReportId),
    /// Activate an application-owned outcome transition.
    Activate(OutcomeAction),
}

/// One spell row in the isolated Lattice Demo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatticeDemoSpellView {
    /// Canonical spell-cell coordinate returned by activation.
    pub coord: hex_core::LatticeCoord,
    /// Stable spell name used by accessibility and walk automation.
    pub name: String,
    /// Player-facing spell heading, including ritual status.
    pub headline: String,
    /// Casting-kind detail.
    pub kind: String,
    /// Total payable mana when the cast is legal.
    pub cost: Option<u32>,
    /// Canonical blocked reason when the cast is unavailable.
    pub blocked: Option<String>,
}

/// Immutable isolated lattice-rules presentation.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct LatticeDemoView {
    /// Whether content has produced a demo state.
    pub ready: bool,
    /// Fully disclosed projected lattice cells.
    pub cells: Vec<crate::LatticeCellView>,
    /// Stable spell action rows.
    pub spells: Vec<LatticeDemoSpellView>,
    /// Current mana/enchantment totals.
    pub totals: String,
    /// Bounded gameplay-owned event lines.
    pub log: Vec<String>,
}

/// Typed isolated Lattice Demo controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatticeDemoIntent {
    /// Strike or restore a projected cell, or cast its spell.
    ActivateCell(hex_core::LatticeCoord),
    /// Cast from the selected spell row.
    Cast(hex_core::LatticeCoord),
    /// Channel mana for the isolated lattice.
    EndTurn,
    /// Rebuild fresh battle state from the inscription.
    Reset,
}

/// Creator workspace selected beneath the Character/Spell library route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CreatorWorkspace {
    /// Library and packaged-template overview.
    #[default]
    Hub,
    /// Character lattice/stat editor.
    Character,
    /// Spell requirements/effects editor.
    Spell,
}

/// Effect template added by one Creator action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreatorEffectKind {
    /// Disable lattice cells.
    Disable,
    /// Apply a damage-over-time effect.
    Burn,
    /// Restore disabled lattice cells.
    Restore,
    /// Reveal hidden information.
    Reveal,
}

/// Editable Creator text field.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreatorNameField {
    /// Character display name.
    Character,
    /// Spell display name.
    Spell,
}

/// Immutable persistence facts shown by the Creator.
#[derive(Debug, Clone, Default)]
pub struct CreatorLibraryView {
    /// Saved character and spell records.
    pub file: hex_assets::CreationLibraryFile,
    /// Last persistence error, when present.
    pub error: Option<String>,
}

/// Immutable Creator presentation snapshot.
#[derive(Resource, Debug, Clone)]
pub struct CreatorScreenView {
    /// Whether either Creator route is active.
    pub active: bool,
    /// Exact screen that owns this projection.
    pub screen: hex_core::Screen,
    /// Character or Spell library surface.
    pub tab: hex_gameplay_model::CreatorSurface,
    /// Library hub or focused editor.
    pub workspace: CreatorWorkspace,
    /// Current character draft.
    pub character: Option<hex_assets::SavedCharacter>,
    /// Current spell draft.
    pub spell: Option<hex_assets::SavedSpell>,
    /// Inspected lattice cell.
    pub selected_cell: Option<hex_core::LatticeCoord>,
    /// Active lattice authoring tool.
    pub active_tool: Option<hex_assets::CreationCellKind>,
    /// Whether the erase tool is active.
    pub erase_tool: bool,
    /// Discrete lattice zoom setting.
    pub zoom_step: i8,
    /// Whether the character draft differs from persistence.
    pub character_dirty: bool,
    /// Whether the spell draft differs from persistence.
    pub spell_dirty: bool,
    /// Player-facing validation or persistence notice.
    pub notice: String,
    /// Whether destructive record deletion awaits confirmation.
    pub confirm_delete: bool,
    /// Whether library reset awaits confirmation.
    pub confirm_reset: bool,
    /// Frozen saved-library presentation.
    pub library: CreatorLibraryView,
    /// Accepted element catalog.
    pub elements: Option<hex_assets::ElementCatalog>,
    /// Accepted combined spell catalog.
    pub spell_book: Option<hex_assets::SpellBook>,
    /// Accepted shipped spell source.
    pub spell_file: Option<hex_assets::SpellFile>,
    /// Accepted lattice source.
    pub lattice_file: Option<hex_assets::LatticeFile>,
    /// Packaged Creator templates.
    pub presets: Option<hex_assets::CreationPresetCatalog>,
    /// Canonical character validation issues.
    pub character_issues: Vec<String>,
    /// Canonical spell validation issues.
    pub spell_issues: Vec<String>,
    /// Shipped spells admitted by combat deployability checks.
    pub deployable_shipped_spells: Vec<String>,
    /// Custom spells admitted by validation and deployability checks.
    pub deployable_custom_spells: Vec<hex_assets::CustomSpellId>,
}

impl Default for CreatorScreenView {
    fn default() -> Self {
        Self {
            active: false,
            screen: hex_core::Screen::CharacterCreator,
            tab: hex_gameplay_model::CreatorSurface::Characters,
            workspace: CreatorWorkspace::Hub,
            character: None,
            spell: None,
            selected_cell: None,
            active_tool: None,
            erase_tool: false,
            zoom_step: 0,
            character_dirty: false,
            spell_dirty: false,
            notice: String::new(),
            confirm_delete: false,
            confirm_reset: false,
            library: CreatorLibraryView::default(),
            elements: None,
            spell_book: None,
            spell_file: None,
            lattice_file: None,
            presets: None,
            character_issues: Vec::new(),
            spell_issues: Vec::new(),
            deployable_shipped_spells: Vec::new(),
            deployable_custom_spells: Vec::new(),
        }
    }
}

/// Typed Creator actions interpreted by the application composition root.
#[derive(Component, Debug, Clone, PartialEq)]
pub enum CreatorIntent {
    /// Navigate to the canonical previous Creator destination.
    Back,
    /// Open the Spell Creator from a character workspace.
    OpenSpellCreator,
    /// Create a blank character draft.
    NewCharacter,
    /// Create a blank spell draft.
    NewSpell,
    /// Open one saved character.
    SelectCharacter(hex_assets::CustomCharacterId),
    /// Open one saved spell.
    SelectSpell(hex_assets::CustomSpellId),
    /// Duplicate the active character.
    DuplicateCharacter,
    /// Duplicate the active spell.
    DuplicateSpell,
    /// Duplicate one packaged character template.
    DuplicatePackagedCharacter(String),
    /// Duplicate one packaged spell template.
    DuplicatePackagedSpell(String),
    /// Validate and persist the character draft.
    SaveCharacter,
    /// Validate and persist the spell draft.
    SaveSpell,
    /// Request or confirm character deletion.
    DeleteCharacter,
    /// Request or confirm spell deletion.
    DeleteSpell,
    /// Inspect one authored lattice cell.
    SelectCell(hex_core::LatticeCoord),
    /// Add the active tool at a lattice coordinate.
    AddCell(hex_core::LatticeCoord),
    /// Select the inspection tool.
    InspectTool,
    /// Select one lattice authoring tool.
    ChooseTool(hex_assets::CreationCellKind),
    /// Select the erase tool.
    ChooseErase,
    /// Adjust lattice zoom by a discrete delta.
    Zoom(i8),
    /// Reset lattice zoom to fit.
    FitLattice,
    /// Remove the selected lattice cell.
    RemoveCell,
    /// Adjust one character mana stat.
    AdjustStat {
        /// Stable element name.
        element: String,
        /// Channelling when true, capacity otherwise.
        channelling: bool,
        /// Signed discrete adjustment.
        delta: i8,
    },
    /// Add a spell requirement.
    AddRequirement,
    /// Remove one spell requirement.
    RemoveRequirement(usize),
    /// Cycle one requirement's element.
    CycleRequirement(usize),
    /// Adjust one requirement's mana.
    AdjustRequirement(usize, i8),
    /// Select enchantment or evocation casting.
    SetEnchantment(bool),
    /// Select single-target or self-cast targeting.
    SetSingleTarget(bool),
    /// Adjust spell range.
    AdjustRange(i8),
    /// Adjust enchantment defense.
    AdjustDefense(i8),
    /// Add one effect template.
    AddEffect(CreatorEffectKind),
    /// Remove one ordered effect.
    RemoveEffect(usize),
    /// Move one ordered effect.
    MoveEffect(usize, i8),
    /// Adjust one effect magnitude.
    AdjustEffect(usize, i8),
    /// Undo the latest draft edit.
    Undo,
    /// Redo the latest undone edit.
    Redo,
    /// Restore the persisted draft.
    DiscardChanges,
    /// Open the isolated lattice test with the unsaved draft.
    LocalTest,
    /// Open Combat Lab with the saved character.
    TestOnMap,
    /// Request or confirm full library reset.
    ResetLibrary,
    /// Replace one draft name.
    SetName(CreatorNameField, String),
}

/// Rules profile used for one fixed-fixture launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatLabRulesVariant {
    /// Authored shipping rules.
    Shipped,
    /// Authored tactical two-step preset.
    TacticalTwoStep,
    /// Custom three-step movement profile.
    CustomThreeStep,
}

/// Editable saved-report annotation field.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatLabReportField {
    /// Short report label.
    Label(hex_gameplay_model::CombatLabReportId),
    /// Longer free-form notes.
    Notes(hex_gameplay_model::CombatLabReportId),
}

/// One frozen report card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombatLabReportCardView {
    /// Stable local report identity.
    pub id: hex_gameplay_model::CombatLabReportId,
    /// Termination heading.
    pub heading: String,
    /// Editable label.
    pub label: String,
    /// Editable notes.
    pub notes: String,
    /// Frozen launch identity and fingerprint.
    pub metadata: String,
    /// Canonical summary metrics.
    pub summary: String,
    /// Whether selected on the left comparison axis.
    pub left_selected: bool,
    /// Whether selected on the right comparison axis.
    pub right_selected: bool,
    /// Whether destructive confirmation is open.
    pub pending_delete: bool,
}

/// Frozen comparison presentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombatLabComparisonView {
    /// Comparison heading naming both stable IDs.
    pub heading: String,
    /// Frozen launch headers.
    pub frozen: String,
    /// Canonical metric deltas.
    pub deltas: String,
}

/// Saved-report surface projection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CombatLabReportsView {
    /// Last report persistence error.
    pub error: Option<String>,
    /// Saved reports in stable local order.
    pub reports: Vec<CombatLabReportCardView>,
    /// Independent two-report comparison.
    pub comparison: Option<CombatLabComparisonView>,
}

/// Immutable Combat Lab setup presentation.
#[derive(Resource, Debug, Clone)]
pub struct CombatLabScreenView {
    /// Whether the Combat Lab screen is active.
    pub active: bool,
    /// Active top-level Lab surface.
    pub tab: hex_gameplay_model::LabTab,
    /// Active Sandbox step.
    pub sandbox_step: hex_gameplay_model::SandboxStep,
    /// Selected packaged map ID.
    pub map: String,
    /// Ordered player roster.
    pub players: Vec<hex_gameplay_model::RosterChoice<hex_assets::CustomCharacterId>>,
    /// Ordered hostile roster.
    pub hostiles: Vec<hex_gameplay_model::RosterChoice<hex_assets::CustomCharacterId>>,
    /// Fixture search query.
    pub fixture_filter: String,
    /// Player-facing setup notice.
    pub notice: String,
    /// Selected combat rules.
    pub rules: Option<hex_assets::CombatRulesProfile>,
    /// Report awaiting destructive confirmation.
    pub pending_report_delete: Option<hex_gameplay_model::CombatLabReportId>,
    /// Saved Creator content.
    pub library: CreatorLibraryView,
    /// Accepted element catalog.
    pub elements: Option<hex_assets::ElementCatalog>,
    /// Accepted spell catalog.
    pub spells: Option<hex_assets::SpellBook>,
    /// Packaged Creator content.
    pub presets: Option<hex_assets::CreationPresetCatalog>,
    /// Packaged Combat Lab map catalog.
    pub maps: Option<hex_assets::CombatLabMapCatalog>,
    /// Authored combat settings.
    pub combat: Option<hex_assets::CombatSettings>,
    /// Choices admitted by the canonical map-readiness oracle.
    pub map_ready_choices: Vec<hex_gameplay_model::RosterChoice<hex_assets::CustomCharacterId>>,
    /// Frozen saved-report presentation.
    pub reports: CombatLabReportsView,
}

impl Default for CombatLabScreenView {
    fn default() -> Self {
        Self {
            active: false,
            tab: hex_gameplay_model::LabTab::Sandbox,
            sandbox_step: hex_gameplay_model::SandboxStep::Map,
            map: String::new(),
            players: Vec::new(),
            hostiles: Vec::new(),
            fixture_filter: String::new(),
            notice: String::new(),
            rules: None,
            pending_report_delete: None,
            library: CreatorLibraryView::default(),
            elements: None,
            spells: None,
            presets: None,
            maps: None,
            combat: None,
            map_ready_choices: Vec::new(),
            reports: CombatLabReportsView::default(),
        }
    }
}

/// Typed Combat Lab setup actions interpreted by the composition root.
#[derive(Component, Debug, Clone, PartialEq)]
pub enum CombatLabIntent {
    /// Select a top-level Lab surface.
    Tab(hex_gameplay_model::LabTab),
    /// Return to the title screen.
    Back,
    /// Select one Sandbox step.
    ShowSandboxStep(hex_gameplay_model::SandboxStep),
    /// Select a packaged map.
    SelectMap(String),
    /// Add a packaged player template.
    AddPlayerTemplate(String),
    /// Add a packaged hostile template.
    AddHostileTemplate(String),
    /// Add a saved player character.
    AddPlayerCustom(hex_assets::CustomCharacterId),
    /// Add a saved hostile character.
    AddHostileCustom(hex_assets::CustomCharacterId),
    /// Remove an ordered player entry.
    RemovePlayer(usize),
    /// Remove an ordered hostile entry.
    RemoveHostile(usize),
    /// Move an ordered player entry.
    MovePlayer(usize, i8),
    /// Move an ordered hostile entry.
    MoveHostile(usize, i8),
    /// Open one blocked saved character in Creator.
    EditCustom(hex_assets::CustomCharacterId),
    /// Select an authored rules preset.
    SelectRulesPreset(hex_assets::CombatRulesPreset),
    /// Adjust one custom rules field.
    AdjustRule(hex_assets::CombatRuleField, i8),
    /// Restore shipped rules.
    ResetRules,
    /// Load terrain and enter deployment.
    PrepareDeployment,
    /// Launch one deterministic fixture.
    StartFixture(String, CombatLabRulesVariant),
    /// Select the left report comparison.
    SelectCompareLeft(hex_gameplay_model::CombatLabReportId),
    /// Select the right report comparison.
    SelectCompareRight(hex_gameplay_model::CombatLabReportId),
    /// Open report-delete confirmation.
    RequestReportDelete(hex_gameplay_model::CombatLabReportId),
    /// Delete one confirmed report.
    ConfirmReportDelete(hex_gameplay_model::CombatLabReportId),
    /// Close report-delete confirmation.
    CancelReportDelete,
    /// Replace the fixture search query.
    SetFixtureFilter(String),
    /// Replace one saved-report annotation field.
    SetReportField(CombatLabReportField, String),
}

/// One immutable roster row in the Combat Lab deployment HUD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentRosterEntryView {
    /// Stable side-local row index.
    pub index: usize,
    /// Player-facing build name.
    pub name: String,
    /// Whether this row owns the next surface click.
    pub selected: bool,
    /// Exact chosen surface rendered as a disclosed label.
    pub position: Option<hex_core::TilePos>,
}

/// Immutable Combat Lab deployment presentation.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct DeploymentView {
    /// Whether a deployment session is active.
    pub active: bool,
    /// Packaged map display name.
    pub map_name: String,
    /// Current placement instruction or refusal.
    pub notice: String,
    /// Ordered player roster.
    pub players: Vec<DeploymentRosterEntryView>,
    /// Ordered hostile roster.
    pub hostiles: Vec<DeploymentRosterEntryView>,
    /// Whether every exact placement passes the canonical start gate.
    pub complete: bool,
}

/// Typed deployment actions interpreted by the composition root.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeploymentIntent {
    /// Select a side-local roster row.
    Select {
        /// Player or hostile side.
        player: bool,
        /// Side-local roster index.
        index: usize,
    },
    /// Restore the last changed placement.
    Undo,
    /// Clear every player placement.
    ClearPlayer,
    /// Clear every hostile placement.
    ClearHostile,
    /// Apply deterministic canonical placement order.
    AutoPlace,
    /// Return to Combat Lab rules.
    Back,
    /// Confirm the complete exact deployment.
    StartCombat,
}

/// Typed gameplay-lattice inputs emitted by presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatticeIntent {
    /// Toggle one actionable canonical cell.
    ToggleCell(hex_core::LatticeCoord),
    /// Clear the current local choice.
    ClearDecision,
    /// Submit the current exact choice through the command funnel.
    ConfirmDecision,
}

/// Disclosed side label used by the initiative renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitiativeSide {
    /// Player-controlled combatant.
    Ally,
    /// Hostile combatant.
    Hostile,
}

/// One immutable initiative row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitiativeEntryView {
    /// Stable canonical unit identity used only for observation names.
    pub unit: UnitId,
    /// Already-disclosed player-facing name.
    pub name: String,
    /// Disclosed faction side.
    pub side: InitiativeSide,
    /// Whether this is the current actor.
    pub current: bool,
}

/// Immutable initiative presentation supplied by the game adapter.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct InitiativeView {
    /// Player-facing heading such as “your turn” or “enemy turn”.
    pub heading: String,
    /// Stable combat order.
    pub entries: Vec<InitiativeEntryView>,
}

/// Development-only projection of the current cyclic map time.
#[cfg(feature = "dev-tools")]
#[derive(Resource, Debug, Clone, PartialEq)]
pub enum DevTimeView {
    /// Cyclic time can be adjusted for the current map.
    Available {
        /// Current hour in the scenario's `[0, 24)` cycle.
        hours: f32,
    },
    /// The current lighting profile does not expose a cyclic clock.
    Unavailable {
        /// Player-facing explanation supplied by the application adapter.
        reason: String,
    },
}

#[cfg(feature = "dev-tools")]
impl Default for DevTimeView {
    fn default() -> Self {
        Self::Unavailable {
            reason: "Cyclic time is unavailable for this map.".to_owned(),
        }
    }
}

/// Development-only time controls emitted by presentation.
#[cfg(feature = "dev-tools")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevTimeIntent {
    /// Move the cyclic clock back by thirty minutes.
    PreviousHalfHour,
    /// Move the cyclic clock forward by thirty minutes.
    NextHalfHour,
    /// Set the cyclic clock to midnight.
    Midnight,
    /// Set the cyclic clock to dawn.
    Dawn,
    /// Set the cyclic clock to noon.
    Noon,
    /// Set the cyclic clock to dusk.
    Dusk,
}

/// One configurable setting rendered by the Settings screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiSetting {
    /// Borderless fullscreen.
    Fullscreen,
    /// Windowed resolution.
    WindowSize,
    /// Present mode.
    Presentation,
    /// Global UI scale.
    UiScale,
    /// Master volume.
    MasterVolume,
    /// Music volume.
    MusicVolume,
    /// Effects volume.
    EffectsVolume,
    /// UI volume.
    UiVolume,
}

/// Immutable label and current value for one setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiSettingRow {
    /// Typed setting identity returned by interaction.
    pub setting: UiSetting,
    /// Nearby player-facing label.
    pub label: String,
    /// Current player-facing value.
    pub value: String,
}

/// Immutable Settings screen projection.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct UiSettingsView {
    /// Ordered controls.
    pub rows: Vec<UiSettingRow>,
    /// Persistence or validation notice.
    pub notice: Option<String>,
}

/// Immutable pause overlay projection.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct PauseView {
    /// Available pause actions and bindings.
    pub hint: String,
    /// Save/resume notice.
    pub notice: Option<String>,
}

/// One immutable scenario card in the development catalog.
#[derive(Debug, Clone)]
pub struct TitleScenarioView {
    /// Exact launch input represented by the card.
    pub scenario: Scenario,
    /// Session-resolved seed shown beside generated scenarios.
    pub resolved_seed: Option<u64>,
}

/// Immutable development-scenario catalog supplied by the composition root.
#[derive(Resource, Debug, Default, Clone)]
pub struct ScenarioBrowserView {
    /// Visible Maps and Demos in authored order.
    pub scenarios: Vec<TitleScenarioView>,
}

/// Immutable title-screen projection supplied by the composition root.
#[derive(Resource, Debug, Default, Clone)]
pub struct TitleView {
    /// Setup failure carried back from gameplay, if one exists.
    pub setup_failure: Option<String>,
}

/// Independent Continue affordance supplied by the save adapter.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct ResumeView {
    /// Whether Continue may be activated.
    pub available: bool,
    /// Visible status or refusal reason attached to Continue.
    pub message: String,
}

impl Default for ResumeView {
    fn default() -> Self {
        Self {
            available: false,
            message: "No exploration resume has been saved.".to_owned(),
        }
    }
}

/// Title-screen intents. Scenario intents retain the exact card snapshot that was
/// clicked so a same-frame content hot reload cannot reinterpret the action.
#[derive(Debug, Clone)]
pub enum TitleIntent {
    /// Resume the save adapter's current slot.
    Continue,
    /// Launch the independently configured default game.
    NewGame,
    /// Open the shared Creator hub.
    Creators,
    /// Open Combat Lab.
    CombatLab,
    /// Open the development Map and Demo catalog.
    Scenarios,
    /// Open settings.
    Settings,
    /// Exit the application.
    Quit,
}

/// Typed intentions emitted by the development scenario catalog.
#[derive(Debug, Clone)]
pub enum ScenarioBrowserIntent {
    /// Launch the exact visible scenario snapshot.
    Start(Scenario),
    /// Replace one generated scenario's session seed.
    Reroll(Scenario),
    /// Return to the title.
    Back,
}

impl Default for PauseView {
    fn default() -> Self {
        Self {
            hint: "Esc to resume".to_owned(),
            notice: None,
        }
    }
}

impl Default for GameplayHudView {
    fn default() -> Self {
        Self {
            phase: GameplayPhase::Active,
            actor: None,
            actor_label: "No active unit".to_owned(),
            round: "Exploring".to_owned(),
            movement_remaining: 0,
            action_remaining: false,
            required_prompt: None,
            actions: Vec::new(),
        }
    }
}

/// Typed intentions emitted by presentation and handled by `hex_game`.
#[derive(Message, Debug, Clone)]
pub enum UiIntent {
    /// Activate one application-authorized gameplay action.
    Gameplay(GameplayAction),
    /// Act on a required lattice choice.
    Lattice(LatticeIntent),
    /// Act on the current casting projection.
    Casting(CastingIntent),
    /// Act on party selection or formation configuration.
    Party(PartyIntent),
    /// Act on the live Combat Lab statistics drawer.
    LabStatistics(LabStatisticsIntent),
    /// Act on the encounter outcome or frozen Combat Lab report.
    Outcome(OutcomeIntent),
    /// Act on the isolated Lattice Demo.
    LatticeDemo(LatticeDemoIntent),
    /// Act on Character or Spell Creator presentation.
    Creator(CreatorIntent),
    /// Act on Combat Lab setup or saved reports.
    CombatLab(CombatLabIntent),
    /// Act on the exact Combat Lab deployment surface.
    Deployment(DeploymentIntent),
    /// Adjust the development-only cyclic map clock.
    #[cfg(feature = "dev-tools")]
    DevTime(DevTimeIntent),
    /// Navigate back through the current screen's canonical route.
    Back,
    /// Cycle one Settings value.
    AdjustSetting(UiSetting),
    /// Activate a title-screen route or exact scenario card.
    Title(TitleIntent),
    /// Act on the development scenario catalog.
    Scenarios(ScenarioBrowserIntent),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_actions_sort_after_ordinary_actions() {
        assert!(ActionPriority::Required > ActionPriority::Primary);
        assert!(ActionPriority::Primary > ActionPriority::Secondary);
    }

    #[test]
    fn disabled_actions_require_a_visible_reason() {
        let action = ActionAffordance {
            action: GameplayAction::Channel,
            label: "Cast".to_owned(),
            shortcut: Some("C".to_owned()),
            availability: ActionAvailability::Disabled {
                reason: "No mana".to_owned(),
            },
            priority: ActionPriority::Primary,
        };
        let ActionAvailability::Disabled { reason } = action.availability else {
            return;
        };
        assert!(!reason.trim().is_empty());
    }
}
