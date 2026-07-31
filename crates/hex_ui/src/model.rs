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
    /// Open the pause overlay.
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

/// One immutable scenario card on the title screen.
#[derive(Debug, Clone)]
pub struct TitleScenarioView {
    /// Exact launch input represented by the card.
    pub scenario: Scenario,
    /// Session-resolved seed shown beside generated scenarios.
    pub resolved_seed: Option<u64>,
}

/// Immutable title-screen projection supplied by the composition root.
#[derive(Resource, Debug, Default, Clone)]
pub struct TitleView {
    /// Development scenarios in authored order. The renderer groups them by category.
    pub scenarios: Vec<TitleScenarioView>,
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
    /// Launch one visible development scenario.
    StartScenario(Scenario),
    /// Replace one generated scenario's session seed.
    RerollScenario(Scenario),
    /// Open character authoring.
    CharacterCreator,
    /// Open spell authoring.
    SpellCreator,
    /// Open Combat Lab.
    CombatLab,
    /// Open settings.
    Settings,
    /// Exit the application.
    Quit,
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
    /// Navigate back through the current screen's canonical route.
    Back,
    /// Cycle one Settings value.
    AdjustSetting(UiSetting),
    /// Activate a title-screen route or exact scenario card.
    Title(TitleIntent),
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
