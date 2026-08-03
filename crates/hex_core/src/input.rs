//! Central names, metadata, and configurable keyboard bindings for player intent.

use std::collections::BTreeMap;
use std::fmt;

use bevy_ecs::prelude::Resource;
use bevy_input::{keyboard::KeyCode, ButtonInput};
use serde::{Deserialize, Serialize};

/// Stable player actions, independent from their current key chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum InputAction {
    /// Leave the current menu.
    Cancel,
    /// Pause or resume gameplay.
    Pause,
    /// Return from gameplay to the Main Menu.
    ReturnTitle,
    /// Confirm the current gameplay decision or cast.
    Confirm,
    /// Cancel spell aiming.
    CancelCast,
    /// Cycle the aimed unit.
    NextTarget,
    /// End the acting unit's turn.
    EndTurn,
    /// Recover the party while exploring.
    Rest,
    /// Suppress or restore the ordinary HUD as a group.
    ToggleHud,
    /// Toggle or summon the Party surface.
    ToggleParty,
    /// Toggle or summon the Initiative surface.
    ToggleInitiative,
    /// Toggle or summon the Activity surface.
    ToggleActivity,
    /// Toggle or summon the Action Bar.
    ToggleActionBar,
    /// Open the inspected character in the Main View.
    OpenCharacterView,
    /// Open formation controls in the Main View.
    OpenFormationView,
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
    /// Activate Party slot 1.
    PartySlot1,
    /// Activate Party slot 2.
    PartySlot2,
    /// Activate Party slot 3.
    PartySlot3,
    /// Activate Party slot 4.
    PartySlot4,
    /// Activate Party slot 5.
    PartySlot5,
    /// Activate Party slot 6.
    PartySlot6,
    /// Development-only knowledge reveal.
    RevealAll,
}

impl InputAction {
    /// All serializable actions in their stable presentation order.
    ///
    /// Consumers which dispatch or present actions must use
    /// [`InputActionInventory::active`] instead. This complete vocabulary also
    /// contains actions which are absent from the shipping product graph.
    pub const ALL: [Self; 28] = [
        Self::Cancel,
        Self::Pause,
        Self::ReturnTitle,
        Self::Confirm,
        Self::CancelCast,
        Self::NextTarget,
        Self::EndTurn,
        Self::Rest,
        Self::ToggleHud,
        Self::ToggleParty,
        Self::ToggleInitiative,
        Self::ToggleActivity,
        Self::ToggleActionBar,
        Self::OpenCharacterView,
        Self::OpenFormationView,
        Self::ToggleCamera,
        Self::Save,
        Self::CameraForward,
        Self::CameraBackward,
        Self::CameraLeft,
        Self::CameraRight,
        Self::PartySlot1,
        Self::PartySlot2,
        Self::PartySlot3,
        Self::PartySlot4,
        Self::PartySlot5,
        Self::PartySlot6,
        Self::RevealAll,
    ];

    /// The six stable Party-slot actions in roster order.
    pub const PARTY_SLOTS: [Self; 6] = [
        Self::PartySlot1,
        Self::PartySlot2,
        Self::PartySlot3,
        Self::PartySlot4,
        Self::PartySlot5,
        Self::PartySlot6,
    ];

    /// Static presentation, context, and default-binding metadata.
    #[must_use]
    pub const fn metadata(self) -> InputActionMetadata {
        match self {
            Self::Cancel => InputActionMetadata::fixed(
                "Back",
                InputCategory::System,
                InputContext::Menu,
                KeyCode::Escape,
            ),
            Self::Pause => InputActionMetadata::rebindable(
                "Pause",
                InputCategory::System,
                InputContext::Gameplay,
                KeyCode::Escape,
            ),
            Self::ReturnTitle => InputActionMetadata::rebindable(
                "Return to Main Menu",
                InputCategory::System,
                InputContext::Gameplay,
                KeyCode::Backspace,
            ),
            Self::Confirm => InputActionMetadata::rebindable(
                "Confirm Decision",
                InputCategory::Gameplay,
                InputContext::Gameplay,
                KeyCode::Enter,
            ),
            Self::CancelCast => InputActionMetadata::rebindable(
                "Cancel Aiming",
                InputCategory::Gameplay,
                InputContext::Gameplay,
                KeyCode::KeyQ,
            ),
            Self::NextTarget => InputActionMetadata::rebindable(
                "Next Target",
                InputCategory::Gameplay,
                InputContext::Gameplay,
                KeyCode::Tab,
            ),
            Self::EndTurn => InputActionMetadata::rebindable(
                "End Turn",
                InputCategory::Gameplay,
                InputContext::Gameplay,
                KeyCode::Space,
            ),
            Self::Rest => InputActionMetadata::rebindable(
                "Rest",
                InputCategory::Gameplay,
                InputContext::Gameplay,
                KeyCode::KeyR,
            ),
            Self::ToggleHud => InputActionMetadata::rebindable(
                "Hide / Restore HUD",
                InputCategory::Interface,
                InputContext::Gameplay,
                KeyCode::KeyH,
            ),
            Self::ToggleParty => InputActionMetadata::rebindable(
                "Party",
                InputCategory::Interface,
                InputContext::Gameplay,
                KeyCode::KeyP,
            ),
            Self::ToggleInitiative => InputActionMetadata::rebindable(
                "Initiative",
                InputCategory::Interface,
                InputContext::Gameplay,
                KeyCode::KeyI,
            ),
            Self::ToggleActivity => InputActionMetadata::rebindable(
                "Activity",
                InputCategory::Interface,
                InputContext::Gameplay,
                KeyCode::KeyL,
            ),
            Self::ToggleActionBar => InputActionMetadata::rebindable(
                "Action Bar",
                InputCategory::Interface,
                InputContext::Gameplay,
                KeyCode::KeyB,
            ),
            Self::OpenCharacterView => InputActionMetadata::rebindable(
                "View Character",
                InputCategory::MainView,
                InputContext::Gameplay,
                KeyCode::KeyV,
            ),
            Self::OpenFormationView => InputActionMetadata::rebindable(
                "View Formation",
                InputCategory::MainView,
                InputContext::Gameplay,
                KeyCode::KeyF,
            ),
            Self::ToggleCamera => InputActionMetadata::rebindable(
                "Map / Character Camera",
                InputCategory::Camera,
                InputContext::Gameplay,
                KeyCode::KeyC,
            ),
            Self::Save => InputActionMetadata::rebindable(
                "Save Campaign",
                InputCategory::Gameplay,
                InputContext::Gameplay,
                KeyCode::F5,
            ),
            Self::CameraForward => InputActionMetadata::rebindable(
                "Camera Forward",
                InputCategory::Camera,
                InputContext::Gameplay,
                KeyCode::KeyW,
            ),
            Self::CameraBackward => InputActionMetadata::rebindable(
                "Camera Backward",
                InputCategory::Camera,
                InputContext::Gameplay,
                KeyCode::KeyS,
            ),
            Self::CameraLeft => InputActionMetadata::rebindable(
                "Camera Left",
                InputCategory::Camera,
                InputContext::Gameplay,
                KeyCode::KeyA,
            ),
            Self::CameraRight => InputActionMetadata::rebindable(
                "Camera Right",
                InputCategory::Camera,
                InputContext::Gameplay,
                KeyCode::KeyD,
            ),
            Self::PartySlot1 => party_slot_metadata("Party Slot 1", KeyCode::Digit1),
            Self::PartySlot2 => party_slot_metadata("Party Slot 2", KeyCode::Digit2),
            Self::PartySlot3 => party_slot_metadata("Party Slot 3", KeyCode::Digit3),
            Self::PartySlot4 => party_slot_metadata("Party Slot 4", KeyCode::Digit4),
            Self::PartySlot5 => party_slot_metadata("Party Slot 5", KeyCode::Digit5),
            Self::PartySlot6 => party_slot_metadata("Party Slot 6", KeyCode::Digit6),
            Self::RevealAll => InputActionMetadata::rebindable(
                "Reveal Knowledge (Development)",
                InputCategory::System,
                InputContext::Gameplay,
                KeyCode::KeyK,
            ),
        }
    }

    /// Zero-based Party slot for a Party-slot action.
    #[must_use]
    pub const fn party_slot_index(self) -> Option<usize> {
        match self {
            Self::PartySlot1 => Some(0),
            Self::PartySlot2 => Some(1),
            Self::PartySlot3 => Some(2),
            Self::PartySlot4 => Some(3),
            Self::PartySlot5 => Some(4),
            Self::PartySlot6 => Some(5),
            _ => None,
        }
    }

    const fn is_development_only(self) -> bool {
        matches!(self, Self::RevealAll)
    }

    const fn yields_to_focused_activation(self) -> bool {
        matches!(self, Self::Confirm | Self::NextTarget | Self::EndTurn)
    }
}

/// Compile-selected set of keyboard actions active in this product graph.
///
/// Development-only variants remain part of [`InputAction`]'s serialized
/// vocabulary so preferences can cross between development and shipping builds.
/// They participate in presentation and conflict detection only when the
/// `dev-input` feature is enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputActionInventory {
    include_development: bool,
}

impl InputActionInventory {
    /// Inventory active for the current compile-time feature graph.
    #[must_use]
    pub const fn active() -> Self {
        Self {
            include_development: cfg!(feature = "dev-input"),
        }
    }

    /// Whether this product graph contains an action.
    #[must_use]
    pub const fn contains(self, action: InputAction) -> bool {
        self.include_development || !action.is_development_only()
    }

    /// Iterate active actions in stable Settings presentation order.
    pub fn iter(self) -> impl Iterator<Item = InputAction> {
        InputAction::ALL
            .into_iter()
            .filter(move |action| self.contains(*action))
    }
}

const fn party_slot_metadata(label: &'static str, key: KeyCode) -> InputActionMetadata {
    InputActionMetadata::rebindable(label, InputCategory::Gameplay, InputContext::Gameplay, key)
}

/// Settings category for an input action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum InputCategory {
    /// Unit, turn, and exploration actions.
    Gameplay,
    /// Ordinary gameplay HUD surfaces.
    Interface,
    /// Typed content shown in the central Main View.
    MainView,
    /// Camera mode and movement.
    Camera,
    /// Application-level actions.
    System,
}

impl InputCategory {
    /// All categories in their Settings tab order.
    pub const ALL: [Self; 5] = [
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
            Self::Gameplay => "Gameplay",
            Self::Interface => "Interface",
            Self::MainView => "Main View",
            Self::Camera => "Camera",
            Self::System => "System",
        }
    }
}

/// Coarse dispatch context used to determine whether two bindings can collide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputContext {
    /// Main Menu and child routes, where fixed UI navigation owns focus semantics.
    Menu,
    /// Exploration, combat, deployment, and gameplay overlays.
    Gameplay,
}

impl InputContext {
    const fn overlaps(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::Menu, Self::Menu) | (Self::Gameplay, Self::Gameplay)
        )
    }
}

/// Static metadata used to render and validate one binding row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputActionMetadata {
    /// Player-facing action label.
    pub label: &'static str,
    /// Settings category containing this action.
    pub category: InputCategory,
    /// Dispatch context in which this action is active.
    pub context: InputContext,
    /// Whether Settings may persist an override for this action.
    pub rebindable: bool,
    /// Canonical binding used when no override exists.
    pub default_chord: KeyChord,
}

impl InputActionMetadata {
    const fn rebindable(
        label: &'static str,
        category: InputCategory,
        context: InputContext,
        key: KeyCode,
    ) -> Self {
        Self {
            label,
            category,
            context,
            rebindable: true,
            default_chord: KeyChord::plain(key),
        }
    }

    const fn fixed(
        label: &'static str,
        category: InputCategory,
        context: InputContext,
        key: KeyCode,
    ) -> Self {
        Self {
            label,
            category,
            context,
            rebindable: false,
            default_chord: KeyChord::plain(key),
        }
    }
}

/// Modifier keys held alongside the one primary key in a chord.
#[derive(Serialize, Deserialize, Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct KeyModifiers {
    /// Either Control key is held.
    #[serde(default)]
    pub control: bool,
    /// Either Alt/Option key is held.
    #[serde(default)]
    pub alt: bool,
    /// Either Shift key is held.
    #[serde(default)]
    pub shift: bool,
    /// Either Super/Command/Windows key is held.
    #[serde(default)]
    pub super_key: bool,
}

impl KeyModifiers {
    /// Snapshot the standard modifier state from current keyboard input.
    #[must_use]
    pub fn from_input(input: &ButtonInput<KeyCode>) -> Self {
        Self {
            control: either_pressed(input, KeyCode::ControlLeft, KeyCode::ControlRight),
            alt: either_pressed(input, KeyCode::AltLeft, KeyCode::AltRight),
            shift: either_pressed(input, KeyCode::ShiftLeft, KeyCode::ShiftRight),
            super_key: either_pressed(input, KeyCode::SuperLeft, KeyCode::SuperRight),
        }
    }

    fn matches(self, input: &ButtonInput<KeyCode>) -> bool {
        self == Self::from_input(input)
    }
}

fn either_pressed(input: &ButtonInput<KeyCode>, left: KeyCode, right: KeyCode) -> bool {
    input.pressed(left) || input.pressed(right)
}

/// One serializable keyboard key plus optional standard modifiers.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct KeyChord {
    /// Physical keyboard key.
    pub key: KeyCode,
    /// Modifiers which must be held exactly when the key is pressed.
    #[serde(default)]
    pub modifiers: KeyModifiers,
}

impl KeyChord {
    /// Construct a chord, refusing a pure modifier as its primary key.
    #[must_use]
    pub const fn new(key: KeyCode, modifiers: KeyModifiers) -> Option<Self> {
        if is_modifier_key(key) {
            None
        } else {
            Some(Self { key, modifiers })
        }
    }

    /// Construct a chord with no modifiers.
    #[must_use]
    pub const fn plain(key: KeyCode) -> Self {
        Self {
            key,
            modifiers: KeyModifiers {
                control: false,
                alt: false,
                shift: false,
                super_key: false,
            },
        }
    }

    /// Compact player-facing chord label.
    #[must_use]
    pub fn label(self) -> String {
        let mut parts = Vec::with_capacity(5);
        if self.modifiers.control {
            parts.push("Ctrl".to_owned());
        }
        if self.modifiers.alt {
            parts.push("Alt".to_owned());
        }
        if self.modifiers.shift {
            parts.push("Shift".to_owned());
        }
        if self.modifiers.super_key {
            parts.push("Super".to_owned());
        }
        parts.push(key_label(self.key));
        parts.join("+")
    }

    fn just_pressed(self, input: &ButtonInput<KeyCode>) -> bool {
        input.just_pressed(self.key) && self.modifiers.matches(input)
    }

    fn pressed(self, input: &ButtonInput<KeyCode>) -> bool {
        input.pressed(self.key) && self.modifiers.matches(input)
    }

    const fn valid(self) -> bool {
        !is_modifier_key(self.key)
    }
}

fn key_label(key: KeyCode) -> String {
    match key {
        KeyCode::Space => "Space".to_owned(),
        KeyCode::Enter => "Enter".to_owned(),
        KeyCode::NumpadEnter => "Numpad Enter".to_owned(),
        KeyCode::Escape => "Esc".to_owned(),
        KeyCode::Backspace => "Backspace".to_owned(),
        KeyCode::Tab => "Tab".to_owned(),
        KeyCode::ArrowUp => "Up".to_owned(),
        KeyCode::ArrowDown => "Down".to_owned(),
        KeyCode::ArrowLeft => "Left".to_owned(),
        KeyCode::ArrowRight => "Right".to_owned(),
        KeyCode::KeyA => "A".to_owned(),
        KeyCode::KeyB => "B".to_owned(),
        KeyCode::KeyC => "C".to_owned(),
        KeyCode::KeyD => "D".to_owned(),
        KeyCode::KeyE => "E".to_owned(),
        KeyCode::KeyF => "F".to_owned(),
        KeyCode::KeyG => "G".to_owned(),
        KeyCode::KeyH => "H".to_owned(),
        KeyCode::KeyI => "I".to_owned(),
        KeyCode::KeyJ => "J".to_owned(),
        KeyCode::KeyK => "K".to_owned(),
        KeyCode::KeyL => "L".to_owned(),
        KeyCode::KeyM => "M".to_owned(),
        KeyCode::KeyN => "N".to_owned(),
        KeyCode::KeyO => "O".to_owned(),
        KeyCode::KeyP => "P".to_owned(),
        KeyCode::KeyQ => "Q".to_owned(),
        KeyCode::KeyR => "R".to_owned(),
        KeyCode::KeyS => "S".to_owned(),
        KeyCode::KeyT => "T".to_owned(),
        KeyCode::KeyU => "U".to_owned(),
        KeyCode::KeyV => "V".to_owned(),
        KeyCode::KeyW => "W".to_owned(),
        KeyCode::KeyX => "X".to_owned(),
        KeyCode::KeyY => "Y".to_owned(),
        KeyCode::KeyZ => "Z".to_owned(),
        KeyCode::Digit0 => "0".to_owned(),
        KeyCode::Digit1 => "1".to_owned(),
        KeyCode::Digit2 => "2".to_owned(),
        KeyCode::Digit3 => "3".to_owned(),
        KeyCode::Digit4 => "4".to_owned(),
        KeyCode::Digit5 => "5".to_owned(),
        KeyCode::Digit6 => "6".to_owned(),
        KeyCode::Digit7 => "7".to_owned(),
        KeyCode::Digit8 => "8".to_owned(),
        KeyCode::Digit9 => "9".to_owned(),
        other => format!("{other:?}"),
    }
}

const fn is_modifier_key(key: KeyCode) -> bool {
    matches!(
        key,
        KeyCode::AltLeft
            | KeyCode::AltRight
            | KeyCode::ControlLeft
            | KeyCode::ControlRight
            | KeyCode::ShiftLeft
            | KeyCode::ShiftRight
            | KeyCode::SuperLeft
            | KeyCode::SuperRight
    )
}

/// Persisted deviations from the canonical action defaults.
#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, Eq)]
#[serde(transparent)]
pub struct InputBindingOverrides(BTreeMap<InputAction, KeyChord>);

impl InputBindingOverrides {
    /// Whether no action differs from its default chord.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether any override belongs to the active product graph.
    ///
    /// A shipping build can therefore retain a serialized development override
    /// without exposing a phantom Restore All affordance.
    #[must_use]
    pub fn has_active_overrides(&self) -> bool {
        let inventory = InputActionInventory::active();
        self.0.keys().any(|action| inventory.contains(*action))
    }

    /// Rehomes development-only bindings which collide after crossing product graphs.
    ///
    /// Shipping builds deliberately neither reserve chords for inactive tooling nor
    /// discard its serialized preferences. When those preferences are opened by a
    /// development build again, this gives the tooling action a deterministic free
    /// fallback rather than rejecting unrelated player settings.
    ///
    /// # Errors
    ///
    /// Returns the original conflict if no fallback chord can be found. The override
    /// map remains unchanged on failure.
    pub fn reconcile_active_development_conflicts(&mut self) -> Result<bool, BindingEditError> {
        let inventory = InputActionInventory::active();
        if !inventory.include_development {
            return Ok(false);
        }

        let mut repaired = self.clone();
        let mut changed = false;
        for action in InputAction::ALL
            .into_iter()
            .filter(|action| action.is_development_only())
        {
            let bindings = InputBindings::from_overrides(repaired.clone());
            let requested = bindings.chord(action);
            let Some(conflict) = bindings.conflict_for(action, requested) else {
                continue;
            };
            let Some(fallback) = development_fallback_chords()
                .find(|chord| bindings.conflict_for(action, *chord).is_none())
            else {
                return Err(BindingEditError::Conflict(conflict));
            };
            repaired.store(action, fallback);
            changed = true;
        }
        if changed {
            *self = repaired;
        }
        Ok(changed)
    }

    /// Persisted override for one action, if present.
    #[must_use]
    pub fn get(&self, action: InputAction) -> Option<KeyChord> {
        self.0.get(&action).copied()
    }

    /// Iterate persisted overrides in stable action order.
    pub fn iter(&self) -> impl Iterator<Item = (InputAction, KeyChord)> + '_ {
        self.0.iter().map(|(action, chord)| (*action, *chord))
    }

    /// Validate fixed actions, primary keys, and overlapping-context conflicts.
    ///
    /// # Errors
    ///
    /// Returns the first deterministic binding error.
    pub fn validate(&self) -> Result<(), BindingEditError> {
        let inventory = InputActionInventory::active();
        let bindings = InputBindings {
            overrides: self.clone(),
        };
        for (action, chord) in self
            .iter()
            .filter(|(action, _)| inventory.contains(*action))
        {
            if !action.metadata().rebindable {
                return Err(BindingEditError::FixedAction(action));
            }
            if !chord.valid() {
                return Err(BindingEditError::ModifierOnly(action));
            }
            validate_gameplay_escape_reservation(action, chord)?;
            validate_focus_activation_reservation(action, chord)?;
        }
        for action in inventory.iter() {
            if let Some(conflict) = bindings.conflict_for(action, bindings.chord(action)) {
                if action < conflict.existing {
                    return Err(BindingEditError::Conflict(conflict));
                }
            }
        }
        Ok(())
    }

    fn store(&mut self, action: InputAction, chord: KeyChord) {
        if chord == action.metadata().default_chord {
            self.0.remove(&action);
        } else {
            self.0.insert(action, chord);
        }
    }

    fn clear_active(&mut self) {
        let inventory = InputActionInventory::active();
        self.0.retain(|action, _| !inventory.contains(*action));
    }
}

fn development_fallback_chords() -> impl Iterator<Item = KeyChord> {
    const KEYS: [KeyCode; 13] = [
        KeyCode::KeyK,
        KeyCode::F1,
        KeyCode::F2,
        KeyCode::F3,
        KeyCode::F4,
        KeyCode::F5,
        KeyCode::F6,
        KeyCode::F7,
        KeyCode::F8,
        KeyCode::F9,
        KeyCode::F10,
        KeyCode::F11,
        KeyCode::F12,
    ];
    const MODIFIERS: [KeyModifiers; 4] = [
        KeyModifiers {
            control: false,
            alt: false,
            shift: true,
            super_key: false,
        },
        KeyModifiers {
            control: true,
            alt: false,
            shift: false,
            super_key: false,
        },
        KeyModifiers {
            control: false,
            alt: true,
            shift: false,
            super_key: false,
        },
        KeyModifiers {
            control: false,
            alt: false,
            shift: false,
            super_key: true,
        },
    ];

    MODIFIERS
        .into_iter()
        .flat_map(|modifiers| KEYS.into_iter().map(move |key| KeyChord { key, modifiers }))
}

/// One existing action that would collide with a requested binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindingConflict {
    /// Action receiving the requested chord.
    pub requested: InputAction,
    /// Existing action already using that chord in an overlapping context.
    pub existing: InputAction,
    /// Chord shared by the two actions.
    pub chord: KeyChord,
}

/// Refusal from a binding edit or persisted-override validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingEditError {
    /// The action is not compiled into this product graph.
    UnavailableAction(InputAction),
    /// Fixed navigation actions cannot be overridden.
    FixedAction(InputAction),
    /// A standard modifier cannot be the primary key.
    ModifierOnly(InputAction),
    /// Escape remains reserved for closing HUD tasks and the Pause fallback.
    ReservedGameplayEscape(InputAction),
    /// Tab navigation and Enter/Space activation belong to focused UI controls.
    ReservedFocusedActivation(InputAction),
    /// Another action already uses the chord in an overlapping context.
    Conflict(BindingConflict),
}

impl fmt::Display for BindingEditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnavailableAction(action) => {
                write!(formatter, "{} is unavailable", action.metadata().label)
            }
            Self::FixedAction(action) => {
                write!(formatter, "{} is fixed", action.metadata().label)
            }
            Self::ModifierOnly(action) => write!(
                formatter,
                "{} requires a non-modifier key",
                action.metadata().label
            ),
            Self::ReservedGameplayEscape(action) => write!(
                formatter,
                "Esc is reserved for closing HUD tasks and cannot be assigned to {}",
                action.metadata().label
            ),
            Self::ReservedFocusedActivation(action) => write!(
                formatter,
                "Tab, Enter, and Space are reserved for focused controls and cannot be assigned to {}",
                action.metadata().label
            ),
            Self::Conflict(conflict) => write!(
                formatter,
                "{} is already assigned to {}",
                conflict.chord.label(),
                conflict.existing.metadata().label
            ),
        }
    }
}

impl std::error::Error for BindingEditError {}

/// Successful result of restoring one action's canonical chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingRestoreOutcome {
    /// The action already used its canonical chord.
    Unchanged,
    /// A persisted override was removed.
    Restored,
}

/// Resolved default-plus-override keyboard bindings.
#[derive(Resource, Debug, Clone, Default)]
pub struct InputBindings {
    overrides: InputBindingOverrides,
}

impl InputBindings {
    /// Resolve canonical defaults with the supplied persisted overrides.
    #[must_use]
    pub const fn from_overrides(overrides: InputBindingOverrides) -> Self {
        Self { overrides }
    }

    /// The effective chord currently assigned to `action`.
    #[must_use]
    pub fn chord(&self, action: InputAction) -> KeyChord {
        self.overrides
            .get(action)
            .unwrap_or(action.metadata().default_chord)
    }

    /// Persistable deviations from canonical defaults.
    #[must_use]
    pub const fn overrides(&self) -> &InputBindingOverrides {
        &self.overrides
    }

    /// Whether an action's complete chord was newly pressed.
    #[must_use]
    pub fn just_pressed(&self, input: &ButtonInput<KeyCode>, action: InputAction) -> bool {
        self.chord(action).just_pressed(input)
    }

    /// Whether an action's complete chord is held.
    #[must_use]
    pub fn pressed(&self, input: &ButtonInput<KeyCode>, action: InputAction) -> bool {
        self.chord(action).pressed(input)
    }

    /// Newly pressed Party-member slot, if any.
    #[must_use]
    pub fn pressed_party_member(&self, input: &ButtonInput<KeyCode>) -> Option<usize> {
        InputAction::PARTY_SLOTS
            .into_iter()
            .find(|action| self.just_pressed(input, *action))
            .and_then(InputAction::party_slot_index)
    }

    /// Find the first action that would collide with `requested` and `chord`.
    #[must_use]
    pub fn conflict_for(&self, requested: InputAction, chord: KeyChord) -> Option<BindingConflict> {
        self.conflict_for_except(requested, chord, None)
    }

    /// Assign one chord, storing it only when it differs from the default.
    ///
    /// # Errors
    ///
    /// Refuses unavailable or fixed actions, modifier-only chords, reserved UI
    /// activation keys, or an overlapping conflict.
    pub fn assign(&mut self, action: InputAction, chord: KeyChord) -> Result<(), BindingEditError> {
        validate_editable_chord(action, chord)?;
        if let Some(conflict) = self.conflict_for(action, chord) {
            return Err(BindingEditError::Conflict(conflict));
        }
        self.overrides.store(action, chord);
        Ok(())
    }

    /// Swap the effective chords of two rebindable actions.
    ///
    /// # Errors
    ///
    /// Refuses unavailable or fixed actions, modifier-only or reserved chords, or
    /// any third-action conflict created by the swap.
    pub fn swap(
        &mut self,
        first: InputAction,
        second: InputAction,
    ) -> Result<(), BindingEditError> {
        if first == second {
            return Ok(());
        }
        let first_chord = self.chord(first);
        let second_chord = self.chord(second);
        validate_editable_chord(first, second_chord)?;
        validate_editable_chord(second, first_chord)?;
        if let Some(conflict) = self.conflict_for_except(first, second_chord, Some(second)) {
            return Err(BindingEditError::Conflict(conflict));
        }
        if let Some(conflict) = self.conflict_for_except(second, first_chord, Some(first)) {
            return Err(BindingEditError::Conflict(conflict));
        }
        self.overrides.store(first, second_chord);
        self.overrides.store(second, first_chord);
        Ok(())
    }

    /// Restore one action's canonical default without stealing another chord.
    ///
    /// # Errors
    ///
    /// Refuses an unavailable or fixed action, or an overlapping conflict. The
    /// override map is left byte-for-byte unchanged on refusal.
    pub fn restore(
        &mut self,
        action: InputAction,
    ) -> Result<BindingRestoreOutcome, BindingEditError> {
        let default = action.metadata().default_chord;
        validate_editable_chord(action, default)?;
        if let Some(conflict) = self.conflict_for(action, default) {
            return Err(BindingEditError::Conflict(conflict));
        }
        let outcome = if self.overrides.get(action).is_some() {
            BindingRestoreOutcome::Restored
        } else {
            BindingRestoreOutcome::Unchanged
        };
        self.overrides.store(action, default);
        Ok(outcome)
    }

    /// Restore every canonical default.
    pub fn restore_all(&mut self) {
        self.overrides.clear_active();
    }

    fn conflict_for_except(
        &self,
        requested: InputAction,
        chord: KeyChord,
        except: Option<InputAction>,
    ) -> Option<BindingConflict> {
        let requested_context = requested.metadata().context;
        InputActionInventory::active().iter().find_map(|existing| {
            (existing != requested
                && Some(existing) != except
                && requested_context.overlaps(existing.metadata().context)
                && self.chord(existing) == chord)
                .then_some(BindingConflict {
                    requested,
                    existing,
                    chord,
                })
        })
    }
}

fn validate_editable_chord(action: InputAction, chord: KeyChord) -> Result<(), BindingEditError> {
    if !InputActionInventory::active().contains(action) {
        return Err(BindingEditError::UnavailableAction(action));
    }
    if !action.metadata().rebindable {
        return Err(BindingEditError::FixedAction(action));
    }
    if !chord.valid() {
        return Err(BindingEditError::ModifierOnly(action));
    }
    validate_gameplay_escape_reservation(action, chord)?;
    validate_focus_activation_reservation(action, chord)?;
    Ok(())
}

fn validate_gameplay_escape_reservation(
    action: InputAction,
    chord: KeyChord,
) -> Result<(), BindingEditError> {
    if chord.key == KeyCode::Escape && action != InputAction::Pause {
        return Err(BindingEditError::ReservedGameplayEscape(action));
    }
    Ok(())
}

fn validate_focus_activation_reservation(
    action: InputAction,
    chord: KeyChord,
) -> Result<(), BindingEditError> {
    if matches!(chord.key, KeyCode::Tab | KeyCode::Enter | KeyCode::Space)
        && !action.yields_to_focused_activation()
    {
        return Err(BindingEditError::ReservedFocusedActivation(action));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_preserve_camera_end_turn_and_recognizable_hud_keys() {
        let bindings = InputBindings::default();
        assert_eq!(
            bindings.chord(InputAction::ToggleCamera),
            KeyChord::plain(KeyCode::KeyC)
        );
        assert_eq!(
            bindings.chord(InputAction::EndTurn),
            KeyChord::plain(KeyCode::Space)
        );
        assert_eq!(
            bindings.chord(InputAction::ToggleHud),
            KeyChord::plain(KeyCode::KeyH)
        );
        assert_eq!(
            bindings.chord(InputAction::ToggleParty),
            KeyChord::plain(KeyCode::KeyP)
        );
        assert_eq!(
            bindings.chord(InputAction::ToggleInitiative),
            KeyChord::plain(KeyCode::KeyI)
        );
        assert_eq!(
            bindings.chord(InputAction::ToggleActivity),
            KeyChord::plain(KeyCode::KeyL)
        );
        assert_eq!(
            bindings.chord(InputAction::ToggleActionBar),
            KeyChord::plain(KeyCode::KeyB)
        );
        assert_eq!(
            bindings.chord(InputAction::OpenCharacterView),
            KeyChord::plain(KeyCode::KeyV)
        );
        assert_eq!(
            bindings.chord(InputAction::OpenFormationView),
            KeyChord::plain(KeyCode::KeyF)
        );
    }

    #[test]
    fn party_slots_are_ordinary_actions_in_stable_order() {
        let bindings = InputBindings::default();
        let mut input = ButtonInput::default();
        input.press(KeyCode::Digit4);
        assert_eq!(bindings.pressed_party_member(&input), Some(3));
        assert_eq!(InputAction::PartySlot4.party_slot_index(), Some(3));
        assert_eq!(InputAction::ToggleParty.party_slot_index(), None);
    }

    #[test]
    fn a_chord_requires_its_exact_modifiers() {
        let mut bindings = InputBindings::default();
        let chord = KeyChord::new(
            KeyCode::KeyP,
            KeyModifiers {
                shift: true,
                ..KeyModifiers::default()
            },
        )
        .expect("non-modifier key makes a chord");
        bindings
            .assign(InputAction::ToggleParty, chord)
            .expect("unused chord is assignable");

        let mut input = ButtonInput::default();
        input.press(KeyCode::KeyP);
        assert!(!bindings.just_pressed(&input, InputAction::ToggleParty));
        input.press(KeyCode::ShiftLeft);
        assert!(bindings.just_pressed(&input, InputAction::ToggleParty));
        assert_eq!(chord.label(), "Shift+P");
    }

    #[test]
    fn visually_distinct_keys_have_distinct_labels() {
        assert_eq!(KeyChord::plain(KeyCode::Enter).label(), "Enter");
        assert_eq!(
            KeyChord::plain(KeyCode::NumpadEnter).label(),
            "Numpad Enter"
        );
    }

    #[test]
    fn overlapping_context_conflicts_and_exclusive_contexts_do_not() {
        let bindings = InputBindings::default();
        let conflict = bindings
            .conflict_for(InputAction::ToggleParty, KeyChord::plain(KeyCode::KeyI))
            .expect("two gameplay actions overlap");
        assert_eq!(conflict.existing, InputAction::ToggleInitiative);

        assert_eq!(
            bindings.conflict_for(InputAction::Pause, KeyChord::plain(KeyCode::Escape)),
            None,
            "fixed menu Back and gameplay Pause are context-exclusive"
        );
    }

    #[test]
    fn assignment_persists_only_an_override_and_restore_removes_it() {
        let mut bindings = InputBindings::default();
        bindings
            .assign(InputAction::ToggleParty, KeyChord::plain(KeyCode::KeyY))
            .expect("unused chord is assignable");
        assert_eq!(bindings.overrides().iter().count(), 1);
        assert_eq!(bindings.chord(InputAction::ToggleParty).key, KeyCode::KeyY);

        assert_eq!(
            bindings.restore(InputAction::ToggleParty),
            Ok(BindingRestoreOutcome::Restored)
        );
        assert!(bindings.overrides().is_empty());
        assert_eq!(bindings.chord(InputAction::ToggleParty).key, KeyCode::KeyP);
        assert_eq!(
            bindings.restore(InputAction::ToggleParty),
            Ok(BindingRestoreOutcome::Unchanged)
        );
    }

    #[test]
    fn conflict_can_be_resolved_by_swapping_bindings() {
        let mut bindings = InputBindings::default();
        let conflict = bindings
            .assign(InputAction::ToggleParty, KeyChord::plain(KeyCode::KeyI))
            .expect_err("initiative already owns I");
        assert_eq!(
            conflict,
            BindingEditError::Conflict(BindingConflict {
                requested: InputAction::ToggleParty,
                existing: InputAction::ToggleInitiative,
                chord: KeyChord::plain(KeyCode::KeyI),
            })
        );

        bindings
            .swap(InputAction::ToggleParty, InputAction::ToggleInitiative)
            .expect("two ordinary HUD bindings can swap");
        assert_eq!(bindings.chord(InputAction::ToggleParty).key, KeyCode::KeyI);
        assert_eq!(
            bindings.chord(InputAction::ToggleInitiative).key,
            KeyCode::KeyP
        );
        assert_eq!(bindings.overrides().iter().count(), 2);
    }

    #[test]
    fn restoring_one_side_of_a_swap_is_conflict_safe_and_atomic() {
        let mut bindings = InputBindings::default();
        bindings
            .swap(InputAction::ToggleParty, InputAction::ToggleInitiative)
            .expect("two ordinary HUD bindings can swap");
        let before = bindings.overrides().clone();

        let conflict = bindings
            .restore(InputAction::ToggleParty)
            .expect_err("initiative still owns Party's canonical P chord");
        assert_eq!(
            conflict,
            BindingEditError::Conflict(BindingConflict {
                requested: InputAction::ToggleParty,
                existing: InputAction::ToggleInitiative,
                chord: KeyChord::plain(KeyCode::KeyP),
            })
        );
        assert_eq!(bindings.overrides(), &before, "restore refusal is atomic");
        bindings
            .overrides()
            .validate()
            .expect("refused restore preserves valid persisted preferences");

        bindings
            .swap(InputAction::ToggleParty, InputAction::ToggleInitiative)
            .expect("the explicit conflict swap restores both canonical chords");
        assert!(bindings.overrides().is_empty());
    }

    #[test]
    fn focused_activation_keys_are_limited_to_focus_aware_gameplay_actions() {
        let mut bindings = InputBindings::default();
        let before = bindings.overrides().clone();
        for key in [KeyCode::Tab, KeyCode::Enter, KeyCode::Space] {
            assert_eq!(
                bindings.assign(InputAction::ToggleParty, KeyChord::plain(key)),
                Err(BindingEditError::ReservedFocusedActivation(
                    InputAction::ToggleParty
                ))
            );
        }
        assert_eq!(bindings.overrides(), &before);
        assert_eq!(
            bindings.swap(InputAction::ToggleParty, InputAction::Confirm),
            Err(BindingEditError::ReservedFocusedActivation(
                InputAction::ToggleParty
            ))
        );
        assert_eq!(bindings.overrides(), &before, "refused swap is atomic");
        assert_eq!(
            bindings.swap(InputAction::ToggleParty, InputAction::NextTarget),
            Err(BindingEditError::ReservedFocusedActivation(
                InputAction::ToggleParty
            ))
        );
        assert_eq!(bindings.overrides(), &before, "refused Tab swap is atomic");

        bindings
            .swap(InputAction::Confirm, InputAction::EndTurn)
            .expect("both actions yield Enter and Space to a focused control");
        assert_eq!(bindings.chord(InputAction::Confirm).key, KeyCode::Space);
        assert_eq!(bindings.chord(InputAction::EndTurn).key, KeyCode::Enter);
        bindings
            .swap(InputAction::Confirm, InputAction::NextTarget)
            .expect("focus-aware actions may exchange Enter and Tab");
        assert_eq!(bindings.chord(InputAction::Confirm).key, KeyCode::Tab);
        assert_eq!(bindings.chord(InputAction::NextTarget).key, KeyCode::Space);
    }

    #[test]
    fn persisted_tab_override_for_a_non_focus_aware_action_is_refused() {
        let overrides = InputBindingOverrides(BTreeMap::from([(
            InputAction::ToggleParty,
            KeyChord::plain(KeyCode::Tab),
        )]));

        assert_eq!(
            overrides.validate(),
            Err(BindingEditError::ReservedFocusedActivation(
                InputAction::ToggleParty
            ))
        );
    }

    #[cfg(not(feature = "dev-input"))]
    #[test]
    fn shipping_inventory_neither_reserves_nor_surfaces_development_actions() {
        let overrides = InputBindingOverrides(BTreeMap::from([(
            InputAction::RevealAll,
            KeyChord::plain(KeyCode::KeyP),
        )]));
        overrides
            .validate()
            .expect("shipping validation ignores an inactive development override");
        assert!(!InputActionInventory::active().contains(InputAction::RevealAll));
        assert!(!overrides.has_active_overrides());

        let mut bindings = InputBindings::from_overrides(overrides.clone());
        bindings
            .assign(InputAction::ToggleParty, KeyChord::plain(KeyCode::KeyK))
            .expect("shipping does not reserve Reveal All's canonical chord");
        bindings.restore_all();
        assert_eq!(bindings.overrides(), &overrides);
        let mut preserved = overrides.clone();
        assert_eq!(
            preserved.reconcile_active_development_conflicts(),
            Ok(false)
        );
        assert_eq!(preserved, overrides);
        assert_eq!(
            bindings.assign(InputAction::RevealAll, KeyChord::plain(KeyCode::KeyY)),
            Err(BindingEditError::UnavailableAction(InputAction::RevealAll))
        );
    }

    #[cfg(feature = "dev-input")]
    #[test]
    fn development_inventory_detects_reveal_all_collisions() {
        assert!(InputActionInventory::active().contains(InputAction::RevealAll));
        let mut bindings = InputBindings::default();
        assert_eq!(
            bindings.conflict_for(InputAction::ToggleParty, KeyChord::plain(KeyCode::KeyK)),
            Some(BindingConflict {
                requested: InputAction::ToggleParty,
                existing: InputAction::RevealAll,
                chord: KeyChord::plain(KeyCode::KeyK),
            })
        );
        assert!(matches!(
            bindings.assign(InputAction::ToggleParty, KeyChord::plain(KeyCode::KeyK)),
            Err(BindingEditError::Conflict(BindingConflict {
                existing: InputAction::RevealAll,
                ..
            }))
        ));

        let persisted = InputBindingOverrides(BTreeMap::from([(
            InputAction::RevealAll,
            KeyChord::plain(KeyCode::KeyP),
        )]));
        assert!(matches!(
            persisted.validate(),
            Err(BindingEditError::Conflict(BindingConflict {
                requested: InputAction::ToggleParty,
                existing: InputAction::RevealAll,
                ..
            }))
        ));
    }

    #[cfg(feature = "dev-input")]
    #[test]
    fn development_inventory_repairs_cross_build_conflicts_without_losing_player_bindings() {
        for mut overrides in [
            InputBindingOverrides(BTreeMap::from([(
                InputAction::RevealAll,
                KeyChord::plain(KeyCode::KeyP),
            )])),
            InputBindingOverrides(BTreeMap::from([(
                InputAction::ToggleParty,
                KeyChord::plain(KeyCode::KeyK),
            )])),
        ] {
            let player_binding = overrides.get(InputAction::ToggleParty);
            assert!(overrides.validate().is_err());
            assert_eq!(overrides.reconcile_active_development_conflicts(), Ok(true));
            overrides
                .validate()
                .expect("development tooling is moved to a free fallback chord");
            assert_eq!(overrides.get(InputAction::ToggleParty), player_binding);
            assert_eq!(
                InputBindings::from_overrides(overrides).chord(InputAction::RevealAll),
                KeyChord {
                    key: KeyCode::KeyK,
                    modifiers: KeyModifiers {
                        shift: true,
                        ..KeyModifiers::default()
                    },
                }
            );
        }
    }

    #[test]
    fn overrides_round_trip_and_validate() {
        let mut bindings = InputBindings::default();
        bindings
            .assign(
                InputAction::OpenCharacterView,
                KeyChord::new(
                    KeyCode::KeyC,
                    KeyModifiers {
                        control: true,
                        ..KeyModifiers::default()
                    },
                )
                .expect("modified C is a chord"),
            )
            .expect("modified C does not conflict with plain C");
        let encoded = serde_json::to_string(bindings.overrides()).expect("overrides encode");
        let decoded: InputBindingOverrides =
            serde_json::from_str(&encoded).expect("overrides decode");
        decoded
            .validate()
            .expect("round-tripped overrides are valid");
        assert_eq!(decoded, *bindings.overrides());
    }

    #[test]
    fn persisted_overlapping_overrides_are_rejected() {
        let overrides = InputBindingOverrides(BTreeMap::from([
            (InputAction::ToggleParty, KeyChord::plain(KeyCode::KeyY)),
            (
                InputAction::ToggleInitiative,
                KeyChord::plain(KeyCode::KeyY),
            ),
        ]));
        assert!(matches!(
            overrides.validate(),
            Err(BindingEditError::Conflict(_))
        ));
    }

    #[test]
    fn modifier_only_primary_keys_and_fixed_navigation_are_refused() {
        assert_eq!(
            KeyChord::new(KeyCode::ShiftLeft, KeyModifiers::default()),
            None
        );

        let mut bindings = InputBindings::default();
        assert_eq!(
            bindings.assign(InputAction::Cancel, KeyChord::plain(KeyCode::KeyX)),
            Err(BindingEditError::FixedAction(InputAction::Cancel))
        );
    }

    #[test]
    fn gameplay_escape_cannot_be_assigned_or_swapped_away_from_its_close_precedence() {
        let mut bindings = InputBindings::default();
        assert_eq!(
            bindings.assign(InputAction::ToggleParty, KeyChord::plain(KeyCode::Escape)),
            Err(BindingEditError::ReservedGameplayEscape(
                InputAction::ToggleParty
            ))
        );
        assert_eq!(
            bindings.swap(InputAction::Pause, InputAction::ToggleParty),
            Err(BindingEditError::ReservedGameplayEscape(
                InputAction::ToggleParty
            ))
        );

        let persisted = InputBindingOverrides(BTreeMap::from([(
            InputAction::ToggleParty,
            KeyChord::new(
                KeyCode::Escape,
                KeyModifiers {
                    shift: true,
                    ..KeyModifiers::default()
                },
            )
            .expect("modified Escape is syntactically a chord"),
        )]));
        assert_eq!(
            persisted.validate(),
            Err(BindingEditError::ReservedGameplayEscape(
                InputAction::ToggleParty
            ))
        );
    }
}
