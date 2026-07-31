use bevy::prelude::*;
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
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum UiIntent {
    /// Activate one application-authorized gameplay action.
    Gameplay(GameplayAction),
    /// Navigate back through the current screen's canonical route.
    Back,
    /// Cycle one Settings value.
    AdjustSetting(UiSetting),
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
