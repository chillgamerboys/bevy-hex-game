use bevy::prelude::*;
use hex_core::{GameplayPhase, InputAction, PlayerSeat, UnitId};
use hex_gameplay_model::{
    CampaignSlotId, MainMenuRoute, MainViewDestination, SandboxCharacter, SandboxDeploymentSlot,
    SandboxDeploymentStage, SandboxRoute, SandboxSide, SandboxSlotIndex, SandboxStartBlocker,
};

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

/// Placement priority inside the contextual Action Bar.
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

/// Immutable per-surface projection for minimalist gameplay chrome.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameplayChromeView {
    /// Whether the Party surface participates in layout and interaction.
    pub party_shown: bool,
    /// Whether the combat-only Initiative surface participates.
    pub initiative_shown: bool,
    /// Whether the categorized Activity surface participates.
    pub activity_shown: bool,
    /// Whether the available-actions surface participates.
    pub action_bar_shown: bool,
    /// Typed Main View destination; required decisions cannot be dismissed.
    pub main_view: MainViewDestination,
    /// Existing terrain-health presentation gate retained without making it a HUD component.
    pub terrain_health_shown: bool,
    /// Whether terminal encounter presentation supersedes stale decisions.
    pub encounter_complete: bool,
}

impl Default for GameplayChromeView {
    fn default() -> Self {
        Self {
            party_shown: true,
            initiative_shown: false,
            activity_shown: false,
            action_bar_shown: true,
            main_view: MainViewDestination::Closed,
            terrain_health_shown: true,
            encounter_complete: false,
        }
    }
}

impl GameplayChromeView {
    /// Whether a command-modal decision must remain reachable.
    #[must_use]
    pub const fn decision_required(self) -> bool {
        matches!(self.main_view, MainViewDestination::RequiredDecision)
    }

    /// Whether any ordinary screen-space HUD component is currently visible.
    #[must_use]
    pub const fn any_ordinary_shown(self) -> bool {
        self.party_shown
            || self.initiative_shown
            || self.activity_shown
            || self.action_bar_shown
            || matches!(
                self.main_view,
                MainViewDestination::Character(_) | MainViewDestination::Formation
            )
    }
}

/// Category of one disclosure-frozen activity line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind {
    /// Combat command, consequence, or refusal.
    Combat,
    /// Exploration, party, or other high-level gameplay activity.
    Activity,
}

/// One disclosure-frozen gameplay-history line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityLogLineView {
    /// Stable category used by the selected tab.
    pub kind: ActivityKind,
    /// Player-facing event description.
    pub text: String,
    /// Whether the line receives danger emphasis in addition to its wording.
    pub danger: bool,
}

/// Activity-log tab selected by the application adapter.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ActivityTab {
    /// Combat and high-level activity in chronological order.
    #[default]
    All,
    /// Combat events only.
    Combat,
    /// Exploration and party activity only.
    Activity,
}

/// Immutable visible portion of gameplay history.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct ActivityLogView {
    /// Compact heading including the keyboard affordance.
    pub heading: String,
    /// Active typed category tab.
    pub tab: ActivityTab,
    /// Already-filtered visible lines in chronological order.
    pub lines: Vec<ActivityLogLineView>,
}

/// Typed activity-log actions emitted by presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityIntent {
    /// Select one stable category tab.
    SelectTab(ActivityTab),
}

/// Progress for a required damage/restoration lattice choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionChoiceView {
    /// Selected cell count.
    pub chosen: usize,
    /// Exact required count.
    pub owed: usize,
    /// Whether disabled cells are being restored rather than live cells disabled.
    pub restoring: bool,
    /// Current player binding for confirming the completed choice.
    pub confirm_shortcut: String,
}

/// Player-owned lattice panel projection.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnLatticeView {
    /// Semantic Main View role, already converted to player-facing text.
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
    /// Current player binding for confirming a cast.
    pub confirm_shortcut: String,
    /// Current player binding for cycling the target.
    pub next_target_shortcut: String,
    /// Current player binding for cancelling aiming.
    pub cancel_shortcut: String,
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
    /// Small, non-interactive lattice silhouette.
    pub cells: Vec<SandboxLatticeCellView>,
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
    /// Inspect one stable party slot without changing gameplay selection authority.
    ActivateMember(usize),
    /// Make one stable party slot authoritative for formation edits and Solo movement.
    SelectFormationMember(usize),
    /// Toggle group/solo movement.
    ToggleMovementMode,
    /// Select an authored formation preset.
    SelectPreset(String),
    /// Assign the selected unit to an authored relative slot.
    AssignSlot(hex_core::HexCoord),
    /// Rest through the canonical command funnel.
    Rest,
}

/// Outcome actions whose effects remain application-owned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeAction {
    /// Continue after victory.
    Continue,
    /// Retry the active Campaign scenario.
    Retry,
    /// Retry the exact frozen Sandbox launch.
    RetryExact,
    /// Reopen the host-owned assignment lobby after a multiplayer outcome.
    ReturnToLobby,
    /// Close the host-owned multiplayer session for every peer.
    CloseSession,
    /// Leave a multiplayer session without asserting a host transition.
    LeaveSession,
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

/// Immutable encounter outcome presentation.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct OutcomeView {
    /// Whether an encounter outcome currently blocks gameplay.
    pub visible: bool,
    /// Outcome heading.
    pub title: String,
    /// Short outcome guidance.
    pub detail: String,
    /// Ordered footer actions.
    pub actions: Vec<OutcomeActionView>,
}

/// Typed outcome controls emitted by presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeIntent {
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

/// One authored spell animation as the VFX tuner's spell list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VfxTunerSpellView {
    /// Spell name, the key into `spell_animations.ron`.
    pub name: String,
    /// Short motion/style summary, e.g. "Beam · Spark".
    pub summary: String,
    /// Whether this is the spell currently being tuned.
    pub selected: bool,
}

/// Which tunable parameter a VFX tuner row edits.
///
/// Deliberately a flat enum rather than a reflection path: the tuner is a fixed set
/// of authored knobs, and naming them as data keeps `hex_ui` free of any dependency
/// on `hex_assets`, which owns the struct these map onto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VfxTunerField {
    /// The motion archetype: instant flash, beam, or travelling projectile.
    Motion,
    /// The particle look: spark or flame.
    Style,
    /// The archetype's leading timing — hold, flash, or travel seconds.
    TimingPrimary,
    /// Seconds the impact burst is held. Absent for an instant flash.
    TimingImpact,
    /// A beam or arc line's cross-section width, independent of particle size.
    BeamThickness,
    /// How far an arc's path wanders off the straight line to its target.
    ArcDisplacement,
    /// How many times an arc's path is subdivided, i.e. how fine its crackle is.
    ArcSubdivisions,
    /// How many forked branches split off an arc.
    ArcBranches,
    /// Whether a projectile trails particles across its flight.
    Trail,
    /// How many particles the effect spawns.
    ParticleCount,
    /// World units/second particles leave the emitter at.
    ParticleSpeed,
    /// Seconds an individual particle lives.
    ParticleLifetime,
    /// Individual particle size, and a beam's thickness.
    Scale,
    /// Radius of the ball particles spawn inside and fly out from.
    Spread,
    /// Whether an explicit color replaces the spell's flavor-element tint.
    ColorOverride,
    /// Red channel of the color override.
    ColorRed,
    /// Green channel of the color override.
    ColorGreen,
    /// Blue channel of the color override.
    ColorBlue,
}

/// How a VFX tuner row is operated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VfxTunerControl {
    /// A decrement/increment pair around a numeric readout.
    Nudge,
    /// A single button advancing to the next value in a closed set.
    Cycle,
}

/// One tunable parameter row in the VFX tuner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VfxTunerRowView {
    /// The parameter this row edits.
    pub field: VfxTunerField,
    /// Player-facing parameter name.
    pub label: String,
    /// Formatted current value.
    pub value: String,
    /// How the row is operated.
    pub control: VfxTunerControl,
}

/// Immutable spell VFX tuner presentation.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct VfxTunerView {
    /// Whether authored animation content has loaded.
    pub ready: bool,
    /// Every authored spell animation, in stable name order.
    pub spells: Vec<VfxTunerSpellView>,
    /// Tunable parameters of the selected spell, empty when none is selected.
    pub rows: Vec<VfxTunerRowView>,
    /// Result of the most recent save, or other transient notice.
    pub status: Option<String>,
    /// Whether the live values differ from what is on disk.
    pub dirty: bool,
}

/// Typed spell VFX tuner controls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VfxTunerIntent {
    /// Tune a different spell.
    Select(String),
    /// Step the field's value down.
    Decrement(VfxTunerField),
    /// Step the field's value up.
    Increment(VfxTunerField),
    /// Advance the field to the next value in its closed set.
    Cycle(VfxTunerField),
    /// Replay the selected spell's cast on the preview dummies.
    Play,
    /// Write the live values back to `spell_animations.ron`.
    Save,
    /// Discard live edits and reload the values on disk.
    Revert,
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
    /// Application-owned destination label shown when leaving the library hub.
    pub hub_exit_label: String,
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
            hub_exit_label: "Back to Tools".to_owned(),
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
    /// Select exact occupied-unit touch or ordinary ranged targeting.
    SetTouch(bool),
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
    /// Open Sandbox with the saved character in Party slot 1.
    OpenInSandbox,
    /// Request or confirm full library reset.
    ResetLibrary,
    /// Replace one draft name.
    SetName(CreatorNameField, String),
}

/// One catalog map projected for Sandbox selection or confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxMapView {
    /// Stable catalog identity.
    pub id: String,
    /// Player-facing name.
    pub name: String,
    /// Authored tactical description.
    pub description: String,
    /// Existing renderer-generated preview asset path.
    pub preview: String,
    /// Resolved launch seed, or `None` for authored terrain.
    pub resolved_seed: Option<u64>,
    /// Whether this pending generated choice may receive another seed.
    pub can_regenerate: bool,
}

/// One renderer-ready character identity and lattice summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxCharacterView {
    /// Stable packaged or local identity returned by typed intents.
    pub character: SandboxCharacter<hex_assets::CustomCharacterId>,
    /// Player-facing character name.
    pub name: String,
    /// Compact lattice presentation.
    pub lattice: String,
    /// Renderer-neutral authored lattice cells.
    pub cells: Vec<SandboxLatticeCellView>,
    /// Canonical Map-ready refusal, when this character cannot launch.
    pub blocked: Option<String>,
    /// Whether this character owns the picker preview.
    pub selected: bool,
}

/// Semantic cell treatment for a Sandbox character preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxLatticeCellKind {
    /// Basic elemental mana cell.
    Gem,
    /// Higher-order fusion cell.
    Fusion,
    /// Castable spell inscription.
    Spell,
    /// Structural durability cell.
    Blank,
}

/// One renderer-neutral cell in a Sandbox character preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxLatticeCellView {
    /// Axial q coordinate.
    pub q: i32,
    /// Axial r coordinate.
    pub r: i32,
    /// Compact glyph.
    pub label: String,
    /// Semantic color role.
    pub kind: SandboxLatticeCellKind,
}

/// One of the fixed six roster slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxRosterSlotView {
    /// Exact side-local slot identity.
    pub slot: SandboxSlotIndex,
    /// Occupant presentation, when the slot is not sparse.
    pub character: Option<SandboxCharacterView>,
}

/// Immutable route-specific Sandbox presentation.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct SandboxView {
    /// Whether the coarse Sandbox screen is active.
    pub active: bool,
    /// Renderer-free child route.
    pub route: SandboxRoute,
    /// Committed map summary used by the overview and launch.
    pub map: Option<SandboxMapView>,
    /// Pending, cancelable map summary used only by map detail.
    pub pending_map: Option<SandboxMapView>,
    /// Canonical catalog rows in stable authored order.
    pub maps: Vec<SandboxMapView>,
    /// Fixed Party slots.
    pub party: Vec<SandboxRosterSlotView>,
    /// Fixed Enemy slots.
    pub enemies: Vec<SandboxRosterSlotView>,
    /// Character rows available to the shared picker.
    pub characters: Vec<SandboxCharacterView>,
    /// Currently previewed character, without roster mutation.
    pub preview: Option<SandboxCharacterView>,
    /// Centralized typed launch refusal.
    pub start_blocker: Option<SandboxStartBlocker>,
    /// Supplemental application-owned status, when useful.
    pub notice: Option<String>,
}

impl Default for SandboxView {
    fn default() -> Self {
        let empty_slots = || {
            SandboxSlotIndex::ALL
                .into_iter()
                .map(|slot| SandboxRosterSlotView {
                    slot,
                    character: None,
                })
                .collect()
        };
        Self {
            active: false,
            route: SandboxRoute::Overview,
            map: None,
            pending_map: None,
            maps: Vec::new(),
            party: empty_slots(),
            enemies: empty_slots(),
            characters: Vec::new(),
            preview: None,
            start_blocker: Some(SandboxStartBlocker::MapsLoading),
            notice: None,
        }
    }
}

/// Typed Sandbox actions interpreted by the composition root.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub enum SandboxIntent {
    /// Navigate to the canonical parent route or destination.
    Back,
    /// Open the catalog browser without changing the committed map.
    OpenMapBrowser,
    /// Create a pending map choice and open its confirmation route.
    SelectMap(String),
    /// Replace only the pending generated seed.
    RegenerateMap,
    /// Commit the pending map choice.
    UseMap,
    /// Open one side's reusable fixed-slot roster route.
    OpenRoster(SandboxSide),
    /// Open the shared picker for one exact side and slot.
    OpenCharacterPicker {
        /// Party or Enemies.
        side: SandboxSide,
        /// Bounded side-local slot.
        slot: SandboxSlotIndex,
    },
    /// Preview a character without changing either roster.
    PreviewCharacter(SandboxCharacter<hex_assets::CustomCharacterId>),
    /// Apply the preview to the picker route's exact side and slot.
    UseCharacter,
    /// Clear one exact sparse roster slot.
    ClearSlot {
        /// Party or Enemies.
        side: SandboxSide,
        /// Bounded side-local slot.
        slot: SandboxSlotIndex,
    },
    /// Enter Character Creator with this picker as typed origin.
    CreateCharacter,
    /// Freeze and prepare the current draft for deployment.
    StartSandbox,
}

/// One immutable occupied slot in the guided Sandbox deployment queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentQueueEntryView {
    /// Exact sparse side/slot identity.
    pub slot: SandboxDeploymentSlot,
    /// Player-facing build name.
    pub name: String,
    /// Whether this entry owns the next valid terrain click.
    pub selected: bool,
    /// Whether this entry already owns one exact surface.
    pub placed: bool,
    /// Whether selecting this entry preserves stable guided order.
    pub selectable: bool,
}

/// Immutable Sandbox deployment presentation.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct DeploymentView {
    /// Whether a deployment session is active.
    pub active: bool,
    /// Packaged map display name.
    pub map_name: String,
    /// Current placement instruction or refusal.
    pub notice: String,
    /// Current guided placement or final review stage.
    pub stage: Option<SandboxDeploymentStage>,
    /// Stable Party-then-Enemies occupied-slot queue.
    pub queue: Vec<DeploymentQueueEntryView>,
    /// Whether the exact previous placement edit can be restored.
    pub can_undo: bool,
    /// Whether every exact placement passes the canonical start gate.
    pub complete: bool,
}

/// Typed deployment actions interpreted by the composition root.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeploymentIntent {
    /// Select one occupied sparse slot for placement or repositioning.
    SelectSlot(SandboxDeploymentSlot),
    /// Restore the last changed placement.
    Undo,
    /// Return to the Sandbox overview.
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
    /// Whether activating this row may issue a presentation-only inspection request.
    pub inspectable: bool,
}

/// Immutable initiative presentation supplied by the game adapter.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct InitiativeView {
    /// Player-facing heading such as “your turn” or “enemy turn”.
    pub heading: String,
    /// Stable combat order.
    pub entries: Vec<InitiativeEntryView>,
}

/// Typed initiative actions emitted by presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitiativeIntent {
    /// Inspect one disclosed unit without changing turn or selection authority.
    ActivateUnit(UnitId),
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
    /// Camera zoom speed.
    ZoomSpeed,
}

/// Direct Settings route selected by one persistent tab.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    /// Display, scale, and audio preferences.
    #[default]
    General,
    /// Unit, turn, save, and Party-slot bindings.
    Gameplay,
    /// Ordinary HUD visibility bindings.
    Interface,
    /// Typed central Main View bindings.
    MainView,
    /// Camera-mode and camera-movement bindings.
    Camera,
    /// Pause, navigation, and development bindings.
    System,
}

impl SettingsTab {
    /// Every direct tab in stable focus order.
    pub const ALL: [Self; 6] = [
        Self::General,
        Self::Gameplay,
        Self::Interface,
        Self::MainView,
        Self::Camera,
        Self::System,
    ];

    /// Player-facing tab label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Gameplay => "Gameplay",
            Self::Interface => "Interface",
            Self::MainView => "Main View",
            Self::Camera => "Camera",
            Self::System => "System",
        }
    }

    /// Input category represented by this tab, or `None` for General.
    #[must_use]
    pub const fn input_category(self) -> Option<hex_core::InputCategory> {
        match self {
            Self::General => None,
            Self::Gameplay => Some(hex_core::InputCategory::Gameplay),
            Self::Interface => Some(hex_core::InputCategory::Interface),
            Self::MainView => Some(hex_core::InputCategory::MainView),
            Self::Camera => Some(hex_core::InputCategory::Camera),
            Self::System => Some(hex_core::InputCategory::System),
        }
    }
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

/// Immutable keybinding row supplied by the Settings adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiBindingRow {
    /// Stable action returned by rebind and restore controls.
    pub action: InputAction,
    /// Player-facing action label.
    pub label: String,
    /// Current resolved chord label.
    pub chord: String,
    /// Whether capture is available for this action.
    pub rebindable: bool,
    /// Whether the current chord differs from the canonical default.
    pub overridden: bool,
}

/// Blocking Settings task presented above the tab content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsModalView {
    /// Capture the next non-modifier key chord for one action.
    Capture {
        /// Action being rebound.
        action: InputAction,
        /// Player-facing action label.
        label: String,
    },
    /// A captured chord collides with another action in the same context.
    Conflict {
        /// Action receiving the captured chord.
        requested: String,
        /// Existing action currently using the chord.
        existing: String,
        /// Player-facing captured chord.
        chord: String,
    },
    /// Confirm destructive restoration of every canonical default.
    ConfirmRestoreAll,
}

/// Immutable Settings screen projection.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct UiSettingsView {
    /// Active direct tab.
    pub tab: SettingsTab,
    /// Ordered General controls; empty on keybinding tabs.
    pub rows: Vec<UiSettingRow>,
    /// Ordered keybindings for the active category; empty on General.
    pub bindings: Vec<UiBindingRow>,
    /// Whether at least one persisted override can be restored.
    pub can_restore_all: bool,
    /// Highest-priority blocking Settings task.
    pub modal: Option<SettingsModalView>,
    /// Persistence or validation notice.
    pub notice: Option<String>,
}

/// Typed Settings actions emitted by presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsIntent {
    /// Select one direct Settings tab.
    SelectTab(SettingsTab),
    /// Cycle one General preference.
    Adjust(UiSetting),
    /// Begin capture for one rebindable action.
    BeginCapture(InputAction),
    /// Cancel active key capture.
    CancelCapture,
    /// Swap the requested and conflicting effective chords.
    SwapConflict,
    /// Keep both bindings unchanged and dismiss the conflict.
    CancelConflict,
    /// Restore one canonical action chord.
    RestoreBinding(InputAction),
    /// Open the Restore All confirmation.
    RequestRestoreAll,
    /// Restore every canonical action chord after confirmation.
    ConfirmRestoreAll,
    /// Dismiss the Restore All confirmation.
    CancelRestoreAll,
    /// Return to the Main Menu.
    Back,
}

/// Immutable pause overlay projection.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct PauseView {
    /// Available pause actions and bindings.
    pub hint: String,
    /// Save/resume notice.
    pub notice: Option<String>,
}

/// Compact character/lattice preview shown in one occupied Campaign slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignPartyMemberView {
    /// Player-facing character name.
    pub name: String,
    /// Existing lattice summary projected by the save adapter.
    pub lattice: String,
    /// Existing compact lattice shape shared with Sandbox character cards.
    pub cells: Vec<SandboxLatticeCellView>,
}

/// Immutable state of one of the three Campaign slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CampaignSlotStatusView {
    /// No manual save has been written to this slot.
    Empty,
    /// A compatible save can be continued.
    Available {
        /// Saved player party in stable unit order.
        party: Vec<CampaignPartyMemberView>,
        /// Accumulated active gameplay time.
        active_time: String,
    },
    /// Data exists but is corrupt or incompatible and must remain untouched.
    Invalid {
        /// Canonical refusal from persistence validation.
        reason: String,
    },
}

/// One immutable Campaign slot card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignSlotView {
    /// Exact persistence identity.
    pub slot: CampaignSlotId,
    /// Empty, resumable, or invalid presentation.
    pub status: CampaignSlotStatusView,
}

/// Immutable progress/refusal state for preparing one host-owned Campaign lobby.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MultiplayerCampaignHostView {
    /// Selected durable slot, if preparation has been attempted.
    pub slot: Option<CampaignSlotId>,
    /// Whether the app is transactionally restoring and validating the checkpoint.
    pub preparing: bool,
    /// Sanitized typed refusal copy; never a path or checkpoint detail.
    pub refusal: Option<String>,
}

/// Disclosure-safe Campaign save status rendered by multiplayer presentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MultiplayerCampaignSaveStatusView {
    /// The host accepted a save and is building the atomic checkpoint.
    Saving,
    /// The host atomically committed the checkpoint.
    Saved,
    /// The host refused or could not commit the save.
    Refused {
        /// Sanitized stable refusal copy.
        reason: String,
    },
}

/// Immutable Main Menu hierarchy supplied by the composition root.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct MainMenuView {
    /// Current renderer-free child route.
    pub route: MainMenuRoute,
    /// Setup failure carried back from gameplay, if one exists.
    pub setup_failure: Option<String>,
    /// Exactly three Campaign slots in stable order.
    pub campaign_slots: Vec<CampaignSlotView>,
}

/// Sensitive text that may be rendered only on an explicit connection-code surface.
/// Ordinary `Debug` output is always redacted so intents and immutable views remain safe
/// to inspect in diagnostics.
#[derive(Default, Clone, PartialEq, Eq)]
pub struct SensitiveText(String);

impl SensitiveText {
    /// Wraps player-entered or explicitly shareable credential-bearing text.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrows the complete value for the explicit edit/copy/share surface only.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Whether no sensitive characters are present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for SensitiveText {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SensitiveText([REDACTED])")
    }
}

/// Presentation-only connection state for one stable human seat.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum MultiplayerSeatConnectionView {
    /// No admitted player owns the seat.
    #[default]
    Vacant,
    /// The admitted player is online.
    Connected,
    /// A disconnected player retains the seat for the displayed whole seconds.
    Reserved {
        /// Whole real-time seconds remaining before temporary host delegation.
        seconds: u32,
    },
    /// The reservation expired and host delegation is active.
    Delegated,
    /// The player reconnected and waits for a quiescent authority boundary.
    ReclaimPending,
}

/// One party assignment rendered inside a seat card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiplayerAssignmentView {
    /// Stable unit identity used only by typed assignment controls.
    pub unit: UnitId,
    /// Shipped player-facing character identity.
    pub label: String,
}

/// Immutable presentation of one of the six stable human seats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiplayerSeatView {
    /// Canonical human seat.
    pub seat: PlayerSeat,
    /// Current connection/reservation/delegation projection.
    pub connection: MultiplayerSeatConnectionView,
    /// Safe short player label; never a credential or transport entity id.
    pub player_label: Option<String>,
    /// Assigned shipped party members in stable manifest order.
    pub assignments: Vec<MultiplayerAssignmentView>,
    /// Guest readiness; host readiness is implicit.
    pub ready: bool,
    /// Whether this seat owns the local process.
    pub local: bool,
}

impl MultiplayerSeatView {
    /// Empty stable seat fixture.
    #[must_use]
    pub fn vacant(seat: PlayerSeat) -> Self {
        Self {
            seat,
            connection: MultiplayerSeatConnectionView::Vacant,
            player_label: None,
            assignments: Vec::new(),
            ready: false,
            local: false,
        }
    }
}

/// Disclosure-safe presentation of one mDNS-resolved open lobby on the current LAN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiplayerLanSessionView {
    /// Opaque DNS-SD identity returned unchanged by a typed join intent.
    pub service_id: String,
    /// Stable public session label; never a machine/user identity.
    pub label: String,
    /// Resolved public LAN endpoint selected by the discovery adapter.
    pub endpoint: String,
    /// Currently claimed human seats.
    pub claimed_seats: u8,
    /// Maximum human seats.
    pub seat_capacity: u8,
    /// Discovery-hint compatibility; final custom admission still checks exact identities.
    pub compatible: bool,
}

/// Immutable Direct Connect and lobby presentation.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct MultiplayerView {
    /// Renderer-free local route.
    pub route: hex_gameplay_model::MultiplayerRoute,
    /// Host/client role after starting a session.
    pub role: Option<hex_gameplay_model::MultiplayerRole>,
    /// Host-derived local seat after admission.
    pub local_seat: Option<PlayerSeat>,
    /// Editable advertised host name/IP. This is public endpoint data, not a secret.
    pub advertised_host: String,
    /// Editable UDP port text; the app adapter performs typed validation.
    pub advertised_port: String,
    /// Explicit host share surface; ordinary diagnostics redact it.
    pub share_code: Option<SensitiveText>,
    /// Explicit join-code editor; ordinary diagnostics redact it.
    pub join_code: SensitiveText,
    /// Whether a private reconnect credential is available in temporary storage.
    pub reconnect_available: bool,
    /// Whether this process is actively browsing the local multicast link.
    pub lan_searching: bool,
    /// Current same-network lobby projections in deterministic service order.
    pub lan_sessions: Vec<MultiplayerLanSessionView>,
    /// Whether this host session was explicitly opened for LAN discovery.
    pub lan_hosting: bool,
    /// Whether the open host lobby is currently being advertised successfully.
    pub lan_advertising: bool,
    /// Whether LAN advertisement stopped and can be explicitly retried.
    pub lan_advertisement_failed: bool,
    /// Exactly three host-owned Campaign slots in stable order.
    pub campaign_slots: Vec<CampaignSlotView>,
    /// Typed Campaign-lobby preparation state.
    pub campaign_host: MultiplayerCampaignHostView,
    /// Whether the current frozen session restores a Campaign checkpoint.
    pub campaign_session: bool,
    /// Latest ordered Campaign save status for the current session.
    pub campaign_save_status: Option<MultiplayerCampaignSaveStatusView>,
    /// Six stable seat cards in canonical order.
    pub seats: Vec<MultiplayerSeatView>,
    /// Scenario/map summary from the frozen manifest.
    pub launch_summary: Option<String>,
    /// Player-facing typed status/refusal/end copy.
    pub notice: Option<String>,
    /// Whether every canonical launch invariant currently passes.
    pub can_launch: bool,
    /// Visible reason Launch is disabled.
    pub launch_blocker: Option<String>,
    /// Remote-client Escape surface; never global pause.
    pub local_menu_open: bool,
}

impl Default for MultiplayerView {
    fn default() -> Self {
        Self {
            route: hex_gameplay_model::MultiplayerRoute::Home,
            role: None,
            local_seat: None,
            advertised_host: "127.0.0.1".to_owned(),
            advertised_port: "7777".to_owned(),
            share_code: None,
            join_code: SensitiveText::default(),
            reconnect_available: false,
            lan_searching: false,
            lan_sessions: Vec::new(),
            lan_hosting: false,
            lan_advertising: false,
            lan_advertisement_failed: false,
            campaign_slots: CampaignSlotId::ALL
                .into_iter()
                .map(|slot| CampaignSlotView {
                    slot,
                    status: CampaignSlotStatusView::Empty,
                })
                .collect(),
            campaign_host: MultiplayerCampaignHostView::default(),
            campaign_session: false,
            campaign_save_status: None,
            seats: (0_u8..=PlayerSeat::LAST_HUMAN.0)
                .filter_map(PlayerSeat::human)
                .map(MultiplayerSeatView::vacant)
                .collect(),
            launch_summary: None,
            notice: None,
            can_launch: false,
            launch_blocker: Some("Configure a shipped Sandbox encounter first.".to_owned()),
            local_menu_open: false,
        }
    }
}

/// Editable field identity on Direct Host/Join setup.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultiplayerTextField {
    /// Public advertised hostname or IP literal.
    AdvertisedHost,
    /// Public UDP port.
    AdvertisedPort,
    /// Credential-bearing `HEX1` join code.
    JoinCode,
}

/// Typed Multiplayer screen/local-menu intentions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MultiplayerIntent {
    /// Open the three-slot host-owned Campaign browser.
    OpenHostCampaign,
    /// Configure a shipped Sandbox whose open lobby is discoverable on this LAN.
    HostLanSandbox,
    /// Start same-network mDNS/DNS-SD browsing.
    OpenLanBrowser,
    /// Restart same-network browsing after a local-network change or failure.
    RefreshLanBrowser,
    /// Join one exact service identity returned by LAN discovery.
    JoinLanSession(String),
    /// Open Direct Host setup/help.
    OpenHostDirect,
    /// Open Direct Join code entry/help.
    OpenJoinDirect,
    /// Replace one editable setup field.
    SetText(MultiplayerTextField, SensitiveText),
    /// Configure a shipped encounter through the existing Sandbox/deployment flow.
    ConfigureSandbox,
    /// Prepare a new or resumable Campaign slot for a fresh assignment lobby.
    HostCampaign(CampaignSlotId),
    /// Copy the current host-issued private connection code to the system clipboard.
    CopyConnectionCode,
    /// Retry advertisement for the current explicitly open LAN host lobby.
    RetryLanAdvertisement,
    /// Start one explicit pinned direct connection.
    JoinDirect,
    /// Reconnect through the persisted pinned endpoint and private rotating credential.
    ReconnectDirect,
    /// Move one character to a claimed destination seat (host-only).
    AssignUnit {
        /// Stable shipped party-member identity.
        unit: UnitId,
        /// Claimed human seat that should receive the member.
        destination: PlayerSeat,
    },
    /// Remove one non-host seat from an open lobby (host-only).
    Kick(PlayerSeat),
    /// Toggle the authenticated guest's own ready state.
    SetReady(bool),
    /// Freeze admission and begin exact world verification (host-only).
    Launch,
    /// Retry the exact frozen encounter (host-only).
    RetryExact,
    /// Reopen assignment and clear readiness (host-only).
    ReturnToLobby,
    /// Close the host-owned session (host-only).
    CloseSession,
    /// Leave a connecting or admitted remote session.
    LeaveSession,
    /// Close the remote client's local, non-pausing Escape menu.
    ResumeLocal,
    /// Apply canonical route-aware Back/Escape behavior.
    Back,
}

impl Default for MainMenuView {
    fn default() -> Self {
        Self {
            route: MainMenuRoute::Root,
            setup_failure: None,
            campaign_slots: CampaignSlotId::ALL
                .into_iter()
                .map(|slot| CampaignSlotView {
                    slot,
                    status: CampaignSlotStatusView::Empty,
                })
                .collect(),
        }
    }
}

/// Typed Main Menu and Campaign intentions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainMenuIntent {
    /// Open the three Campaign slots.
    OpenCampaign,
    /// Open client-hosted multiplayer.
    OpenMultiplayer,
    /// Enter the persistent Sandbox draft.
    OpenSandbox,
    /// Open the creator tools hierarchy.
    OpenTools,
    /// Open Settings.
    OpenSettings,
    /// Open Character Creator from Tools.
    OpenCharacterCreator,
    /// Open Spell Creator from Tools.
    OpenSpellCreator,
    /// Open the spell VFX tuner from Tools.
    OpenVfxTuner,
    /// Bind and launch the default campaign in one empty slot.
    NewCampaign(CampaignSlotId),
    /// Continue one exact occupied slot.
    ContinueCampaign(CampaignSlotId),
    /// Return from a child route to the root.
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
    /// Inspect one disclosed initiative entry.
    Initiative(InitiativeIntent),
    /// Select one gameplay-history category.
    Activity(ActivityIntent),
    /// Act on the encounter outcome.
    Outcome(OutcomeIntent),
    /// Act on the isolated Lattice Demo.
    LatticeDemo(LatticeDemoIntent),
    /// Tune or replay an authored spell animation.
    VfxTuner(VfxTunerIntent),
    /// Act on Character or Spell Creator presentation.
    Creator(CreatorIntent),
    /// Act on Sandbox composition.
    Sandbox(SandboxIntent),
    /// Act on the exact Sandbox deployment surface.
    Deployment(DeploymentIntent),
    /// Adjust the development-only cyclic map clock.
    #[cfg(feature = "dev-tools")]
    DevTime(DevTimeIntent),
    /// Adjust Settings, bindings, or the active Settings tab.
    Settings(SettingsIntent),
    /// Navigate back through the current screen's canonical route.
    Back,
    /// Navigate the Main Menu, Campaign, and Tools hierarchy.
    MainMenu(MainMenuIntent),
    /// Act on Direct Connect, lobby, reconnect, or the client-local menu.
    Multiplayer(MultiplayerIntent),
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

    #[test]
    fn settings_tabs_directly_cover_general_and_every_input_category() {
        assert_eq!(SettingsTab::ALL.first(), Some(&SettingsTab::General));
        let categories = SettingsTab::ALL
            .into_iter()
            .filter_map(SettingsTab::input_category)
            .collect::<Vec<_>>();
        assert_eq!(categories, hex_core::InputCategory::ALL);
    }

    #[test]
    fn sandbox_projection_keeps_six_slots_per_side() {
        let view = SandboxView::default();
        assert_eq!(view.party.len(), 6);
        assert_eq!(view.enemies.len(), 6);
        assert!(matches!(
            view.start_blocker,
            Some(SandboxStartBlocker::MapsLoading)
        ));
    }

    #[test]
    fn multiplayer_secrets_are_redacted_from_views_and_intents() {
        let secret = SensitiveText::new("HEX1.private-invite-material");
        assert_eq!(format!("{secret:?}"), "SensitiveText([REDACTED])");
        assert!(!format!("{secret:?}").contains(secret.expose()));

        let mut view = MultiplayerView {
            share_code: Some(secret.clone()),
            join_code: secret.clone(),
            ..Default::default()
        };
        assert!(!format!("{view:?}").contains(secret.expose()));

        let intent = MultiplayerIntent::SetText(MultiplayerTextField::JoinCode, secret.clone());
        assert!(!format!("{intent:?}").contains(secret.expose()));

        view.share_code = None;
        assert_eq!(view.seats.len(), PlayerSeat::HUMAN_COUNT);
        assert_eq!(view.campaign_slots.len(), CampaignSlotId::ALL.len());
        assert_eq!(
            view.seats.first().map(|seat| seat.seat),
            Some(PlayerSeat::HOST)
        );
    }
}
