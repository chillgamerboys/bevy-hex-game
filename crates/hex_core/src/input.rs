//! Central names for player intent and their fixed pre-alpha keys.

use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use bevy_input::{keyboard::KeyCode, ButtonInput};

/// Stable player actions, independent from their current key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum InputAction {
    /// Leave the current menu or close the application.
    Cancel,
    /// Pause or resume gameplay.
    Pause,
    /// Return from gameplay to the title.
    ReturnTitle,
    /// Confirm the current modal choice or cast.
    Confirm,
    /// Cancel spell aiming.
    CancelCast,
    /// Cycle the aimed unit.
    NextTarget,
    /// End the acting unit's turn.
    EndTurn,
    /// Recover the party while exploring.
    Rest,
    /// Toggle ordinary HUD panels.
    ToggleHud,
    /// Toggle the full combat log.
    ToggleLog,
    /// Toggle close/map camera framing.
    ToggleCamera,
    /// Save a quiescent exploration state.
    Save,
    /// Move the camera forward.
    CameraForward,
    /// Move the camera backward.
    CameraBackward,
    /// Move the camera left.
    CameraLeft,
    /// Move the camera right.
    CameraRight,
    /// Development-only knowledge reveal.
    RevealAll,
}

/// Fixed default key map. Wave 5 centralizes intent but deliberately adds no rebinding UI.
#[derive(Resource, Debug, Clone)]
pub struct InputBindings {
    keys: BTreeMap<InputAction, KeyCode>,
    party: [KeyCode; 6],
}

impl Default for InputBindings {
    fn default() -> Self {
        Self {
            keys: BTreeMap::from([
                (InputAction::Cancel, KeyCode::Escape),
                (InputAction::Pause, KeyCode::Escape),
                (InputAction::ReturnTitle, KeyCode::Backspace),
                (InputAction::Confirm, KeyCode::Enter),
                (InputAction::CancelCast, KeyCode::KeyQ),
                (InputAction::NextTarget, KeyCode::Tab),
                (InputAction::EndTurn, KeyCode::Space),
                (InputAction::Rest, KeyCode::KeyR),
                (InputAction::ToggleHud, KeyCode::KeyH),
                (InputAction::ToggleLog, KeyCode::KeyL),
                (InputAction::ToggleCamera, KeyCode::KeyC),
                (InputAction::Save, KeyCode::F5),
                (InputAction::CameraForward, KeyCode::KeyW),
                (InputAction::CameraBackward, KeyCode::KeyS),
                (InputAction::CameraLeft, KeyCode::KeyA),
                (InputAction::CameraRight, KeyCode::KeyD),
                (InputAction::RevealAll, KeyCode::KeyK),
            ]),
            party: [
                KeyCode::Digit1,
                KeyCode::Digit2,
                KeyCode::Digit3,
                KeyCode::Digit4,
                KeyCode::Digit5,
                KeyCode::Digit6,
            ],
        }
    }
}

impl InputBindings {
    /// Key currently assigned to `action`.
    #[must_use]
    pub fn key(&self, action: InputAction) -> Option<KeyCode> {
        self.keys.get(&action).copied()
    }

    /// Whether an action's key was newly pressed.
    #[must_use]
    pub fn just_pressed(&self, input: &ButtonInput<KeyCode>, action: InputAction) -> bool {
        self.key(action).is_some_and(|key| input.just_pressed(key))
    }

    /// Whether an action's key is held.
    #[must_use]
    pub fn pressed(&self, input: &ButtonInput<KeyCode>, action: InputAction) -> bool {
        self.key(action).is_some_and(|key| input.pressed(key))
    }

    /// Newly pressed party-member slot, if any.
    #[must_use]
    pub fn pressed_party_member(&self, input: &ButtonInput<KeyCode>) -> Option<usize> {
        self.party.iter().position(|key| input.just_pressed(*key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_expose_actions_and_party_slots() {
        let bindings = InputBindings::default();
        assert_eq!(bindings.key(InputAction::EndTurn), Some(KeyCode::Space));

        let mut input = ButtonInput::default();
        input.press(KeyCode::Digit4);
        assert_eq!(bindings.pressed_party_member(&input), Some(3));
    }
}
