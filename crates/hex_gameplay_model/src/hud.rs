//! Renderer-free state and transition policy for the gameplay HUD.
//!
//! Saved component preferences, transient presentation, and blocking Main View
//! decisions are deliberately separate. A responsive reflow or phase-level
//! suppression can therefore change what is currently presented without rewriting
//! the player's persisted choices.

use bevy_ecs::prelude::Resource;
use hex_core::UnitId;
use serde::{Deserialize, Serialize};

/// One independently configurable ordinary HUD component.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HudComponent {
    /// Ordered player-party summary.
    Party,
    /// Current combat turn order.
    Initiative,
    /// Bounded activity and combat event history.
    Activity,
    /// Currently available gameplay actions.
    ActionBar,
}

impl HudComponent {
    /// Every ordinary HUD component in stable presentation order.
    pub const ALL: [Self; 4] = [
        Self::Party,
        Self::Initiative,
        Self::Activity,
        Self::ActionBar,
    ];
}

/// Persisted per-component HUD choices.
///
/// Context eligibility remains separate: keeping Initiative enabled here does not
/// make it appear outside combat, and keeping Action Bar enabled does not invent an
/// action when none is available.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(default)]
pub struct HudComponentPreferences {
    /// Whether Party should appear when the current context admits it.
    pub party: bool,
    /// Whether Initiative should appear when the current context admits it.
    pub initiative: bool,
    /// Whether Activity should appear when the current context admits it.
    pub activity: bool,
    /// Whether Action Bar should appear when the current context admits it.
    pub action_bar: bool,
}

impl Default for HudComponentPreferences {
    fn default() -> Self {
        Self {
            party: true,
            initiative: true,
            activity: false,
            action_bar: true,
        }
    }
}

impl HudComponentPreferences {
    /// Whether one component is enabled in the persisted preference value.
    #[must_use]
    pub const fn is_enabled(self, component: HudComponent) -> bool {
        match component {
            HudComponent::Party => self.party,
            HudComponent::Initiative => self.initiative,
            HudComponent::Activity => self.activity,
            HudComponent::ActionBar => self.action_bar,
        }
    }

    /// Sets one component preference, returning whether the persisted value changed.
    pub fn set_enabled(&mut self, component: HudComponent, enabled: bool) -> bool {
        let target = match component {
            HudComponent::Party => &mut self.party,
            HudComponent::Initiative => &mut self.initiative,
            HudComponent::Activity => &mut self.activity,
            HudComponent::ActionBar => &mut self.action_bar,
        };
        if *target == enabled {
            return false;
        }
        *target = enabled;
        true
    }

    /// Toggles and returns the new persisted value for one component.
    pub fn toggle(&mut self, component: HudComponent) -> bool {
        let enabled = !self.is_enabled(component);
        self.set_enabled(component, enabled);
        enabled
    }
}

/// Context-owned eligibility for ordinary HUD components.
///
/// The game adapter publishes these facts. The model never infers combat phase,
/// available actions, roster membership, or disclosure from presentation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HudContextEligibility {
    /// Whether Party has meaningful content in this context.
    pub party: bool,
    /// Whether Initiative has meaningful, disclosed content in this context.
    pub initiative: bool,
    /// Whether Activity has meaningful content in this context.
    pub activity: bool,
    /// Whether Action Bar has at least one contextual action to present.
    pub action_bar: bool,
}

impl Default for HudContextEligibility {
    fn default() -> Self {
        Self::all()
    }
}

impl HudContextEligibility {
    /// Admits every component.
    #[must_use]
    pub const fn all() -> Self {
        Self {
            party: true,
            initiative: true,
            activity: true,
            action_bar: true,
        }
    }

    /// Admits no ordinary component.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            party: false,
            initiative: false,
            activity: false,
            action_bar: false,
        }
    }

    /// Whether the current canonical context admits one component.
    #[must_use]
    pub const fn is_eligible(self, component: HudComponent) -> bool {
        match component {
            HudComponent::Party => self.party,
            HudComponent::Initiative => self.initiative,
            HudComponent::Activity => self.activity,
            HudComponent::ActionBar => self.action_bar,
        }
    }
}

/// Responsive presentation mode relevant to HUD interaction policy.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum HudViewportMode {
    /// Standard and Wide layouts may show several ordinary components together.
    #[default]
    Standard,
    /// Compact starts map-only and opens at most one temporary task surface.
    Compact,
}

/// Immutable context used to project and transition the HUD state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HudContext {
    /// Responsive mode selected by the presentation adapter.
    pub viewport: HudViewportMode,
    /// Canonical contextual eligibility for each ordinary component.
    pub eligibility: HudContextEligibility,
    /// Whether the current gameplay phase suppresses ordinary HUD presentation.
    pub phase_suppressed: bool,
    /// Whether Formation is a meaningful Main View destination in this context.
    pub formation_available: bool,
}

impl Default for HudContext {
    fn default() -> Self {
        Self::standard(HudContextEligibility::all())
    }
}

impl HudContext {
    /// Builds an unsuppressed Standard/Wide context.
    #[must_use]
    pub const fn standard(eligibility: HudContextEligibility) -> Self {
        Self {
            viewport: HudViewportMode::Standard,
            eligibility,
            phase_suppressed: false,
            formation_available: true,
        }
    }

    /// Builds an unsuppressed Compact context.
    #[must_use]
    pub const fn compact(eligibility: HudContextEligibility) -> Self {
        Self {
            viewport: HudViewportMode::Compact,
            eligibility,
            phase_suppressed: false,
            formation_available: true,
        }
    }

    /// Returns the same context with phase-level suppression selected explicitly.
    #[must_use]
    pub const fn with_phase_suppressed(mut self, suppressed: bool) -> Self {
        self.phase_suppressed = suppressed;
        self
    }

    /// Returns the same context with Formation eligibility selected explicitly.
    #[must_use]
    pub const fn with_formation_available(mut self, available: bool) -> Self {
        self.formation_available = available;
        self
    }
}

/// Typed content hosted by the contextual Main View.
#[derive(Serialize, Deserialize, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum MainViewDestination {
    /// No ordinary Main View is open.
    #[default]
    Closed,
    /// Inspect one stable unit without changing gameplay selection or command authority.
    Character(UnitId),
    /// Inspect or edit the current party formation.
    Formation,
    /// A blocking gameplay decision whose view cannot be dismissed or replaced.
    RequiredDecision,
}

/// One temporary surface summoned while the ordinary HUD is unavailable.
///
/// Compact mode always uses this single-slot task surface. Master-hidden
/// Standard/Wide mode uses the same vocabulary so an explicit shortcut reveals only
/// its requested content while the rest of the HUD remains hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HudTransientSurface {
    /// One ordinary HUD component.
    Component(HudComponent),
    /// Character inspection for one stable unit.
    Character(UnitId),
    /// Formation inspection or editing.
    Formation,
}

/// Whether a HUD action changed runtime-only state or persisted preferences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HudActionResult {
    /// The action was already satisfied or refused by current context.
    NoChange,
    /// Only transient runtime presentation changed.
    RuntimeChanged,
    /// A persisted per-component preference changed.
    PreferencesChanged,
}

/// Renderer-free authority for HUD visibility and Main View transitions.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct HudState {
    preferences: HudComponentPreferences,
    master_suppressed: bool,
    main_view: MainViewDestination,
    transient: Option<HudTransientSurface>,
}

impl Default for HudState {
    fn default() -> Self {
        Self::new(HudComponentPreferences::default())
    }
}

impl HudState {
    /// Creates runtime HUD state from one persisted preference value.
    #[must_use]
    pub const fn new(preferences: HudComponentPreferences) -> Self {
        Self {
            preferences,
            master_suppressed: false,
            main_view: MainViewDestination::Closed,
            transient: None,
        }
    }

    /// Current persisted per-component choices.
    #[must_use]
    pub const fn preferences(&self) -> HudComponentPreferences {
        self.preferences
    }

    /// Replaces persisted choices after loading or resetting preferences.
    ///
    /// Runtime master, transient, and Main View state remain unchanged.
    pub fn replace_preferences(&mut self, preferences: HudComponentPreferences) {
        self.preferences = preferences;
    }

    /// Whether the transient master hide is suppressing the ordinary HUD.
    #[must_use]
    pub const fn master_suppressed(&self) -> bool {
        self.master_suppressed
    }

    /// Stored ordinary or required Main View state before contextual projection.
    #[must_use]
    pub const fn stored_main_view(&self) -> MainViewDestination {
        self.main_view
    }

    /// Stored temporary task surface before contextual projection.
    #[must_use]
    pub const fn raw_transient(&self) -> Option<HudTransientSurface> {
        self.transient
    }

    /// Toggles transient master suppression without changing saved preferences.
    ///
    /// Any one-off summon closes at the master transition boundary. Restoring the
    /// master therefore returns to the exact stored component combination.
    pub fn toggle_master(&mut self) -> HudActionResult {
        self.master_suppressed = !self.master_suppressed;
        self.transient = None;
        HudActionResult::RuntimeChanged
    }

    /// Handles one ordinary component shortcut.
    ///
    /// Standard/Wide toggles the persisted preference. Compact or master-hidden
    /// presentation instead toggles one temporary surface. A different temporary
    /// shortcut replaces the previous one without changing any preference.
    pub fn activate_component(
        &mut self,
        component: HudComponent,
        context: HudContext,
    ) -> HudActionResult {
        if self.decision_required()
            || context.phase_suppressed
            || !context.eligibility.is_eligible(component)
        {
            return HudActionResult::NoChange;
        }

        let requested = HudTransientSurface::Component(component);
        if self.master_suppressed
            || context.viewport == HudViewportMode::Compact
            || self.transient.is_some()
        {
            return self.toggle_transient(requested);
        }

        self.preferences.toggle(component);
        HudActionResult::PreferencesChanged
    }

    /// Opens Character content for one stable unit without changing gameplay authority.
    pub fn open_character(&mut self, unit: UnitId, context: HudContext) -> HudActionResult {
        self.open_main_view(
            HudTransientSurface::Character(unit),
            MainViewDestination::Character(unit),
            context,
        )
    }

    /// Opens Formation content without changing gameplay authority.
    pub fn open_formation(&mut self, context: HudContext) -> HudActionResult {
        if !context.formation_available {
            return HudActionResult::NoChange;
        }
        self.open_main_view(
            HudTransientSurface::Formation,
            MainViewDestination::Formation,
            context,
        )
    }

    /// Closes an effectively visible temporary surface or ordinary Main View.
    ///
    /// Required decisions and stored destinations hidden by responsive, master, or
    /// phase suppression are deliberately non-dismissible through this Escape path.
    pub fn close_active_surface(&mut self, context: HudContext) -> HudActionResult {
        if self.decision_required() {
            return HudActionResult::NoChange;
        }
        if self.effective_transient(context).is_some() {
            self.transient = None;
            return HudActionResult::RuntimeChanged;
        }
        if !matches!(
            self.effective_main_view(context),
            MainViewDestination::Character(_) | MainViewDestination::Formation
        ) {
            return HudActionResult::NoChange;
        }
        self.main_view = MainViewDestination::Closed;
        HudActionResult::RuntimeChanged
    }

    /// Clears any ordinary route for an explicit inspection-context change.
    ///
    /// Unlike Escape, this intentionally clears stored state even when presentation
    /// policy currently hides it, so restoring the HUD cannot reveal a stale subject.
    pub fn clear_active_surface(&mut self) -> HudActionResult {
        if self.decision_required() {
            return HudActionResult::NoChange;
        }
        if self.transient.take().is_some() {
            return HudActionResult::RuntimeChanged;
        }
        if self.main_view == MainViewDestination::Closed {
            return HudActionResult::NoChange;
        }
        self.main_view = MainViewDestination::Closed;
        HudActionResult::RuntimeChanged
    }

    /// Closes Character content only when it belongs to the specified stale subject.
    ///
    /// Disclosure loss is not a generic Back action: an unrelated Compact Party or
    /// Activity task opened after inspection must remain owned by the player.
    pub fn close_character(&mut self, unit: UnitId) -> HudActionResult {
        if self.decision_required() {
            return HudActionResult::NoChange;
        }
        let mut changed = false;
        if self.transient == Some(HudTransientSurface::Character(unit)) {
            self.transient = None;
            changed = true;
        }
        if self.main_view == MainViewDestination::Character(unit) {
            self.main_view = MainViewDestination::Closed;
            changed = true;
        }
        if changed {
            HudActionResult::RuntimeChanged
        } else {
            HudActionResult::NoChange
        }
    }

    /// Forces the required-decision Main View and clears any competing temporary task.
    pub fn require_decision(&mut self) -> HudActionResult {
        if self.decision_required() {
            return HudActionResult::NoChange;
        }
        self.main_view = MainViewDestination::RequiredDecision;
        self.transient = None;
        HudActionResult::RuntimeChanged
    }

    /// Resolves the forced Main View through its dedicated authoritative path.
    pub fn resolve_required_decision(&mut self) -> HudActionResult {
        if !self.decision_required() {
            return HudActionResult::NoChange;
        }
        self.main_view = MainViewDestination::Closed;
        HudActionResult::RuntimeChanged
    }

    /// Removes runtime-only presentation that is invalid in the current context.
    ///
    /// Persisted component choices are never rewritten. This prevents a hidden
    /// Compact task from swallowing Escape or resurrecting after its mode becomes
    /// eligible again, and prevents Formation from leaving a blank Main View when
    /// exploration ends.
    pub fn reconcile_context(&mut self, context: HudContext) -> HudActionResult {
        if self.decision_required() {
            return HudActionResult::NoChange;
        }

        let main_invalid = context.phase_suppressed
            || matches!(self.main_view, MainViewDestination::Formation)
                && !context.formation_available;
        let transient_invalid = self.transient.is_some_and(|surface| {
            context.phase_suppressed
                || matches!(surface, HudTransientSurface::Formation) && !context.formation_available
                || matches!(surface, HudTransientSurface::Component(component)
                    if !context.eligibility.is_eligible(component))
                || context.viewport == HudViewportMode::Standard && !self.master_suppressed
        });

        if main_invalid {
            self.main_view = MainViewDestination::Closed;
        }
        if transient_invalid {
            self.transient = None;
        }
        if main_invalid || transient_invalid {
            HudActionResult::RuntimeChanged
        } else {
            HudActionResult::NoChange
        }
    }

    /// Whether one ordinary component is effectively visible in this context.
    #[must_use]
    pub fn is_component_visible(&self, component: HudComponent, context: HudContext) -> bool {
        if context.phase_suppressed {
            return false;
        }
        if let Some(transient) = self.transient {
            return transient == HudTransientSurface::Component(component)
                && context.eligibility.is_eligible(component);
        }
        if self.master_suppressed || context.viewport == HudViewportMode::Compact {
            return false;
        }
        self.preferences.is_enabled(component) && context.eligibility.is_eligible(component)
    }

    /// Effective typed Main View after transient, responsive, master, and phase policy.
    ///
    /// Required decisions project above every suppression layer.
    #[must_use]
    pub fn effective_main_view(&self, context: HudContext) -> MainViewDestination {
        if self.decision_required() {
            return MainViewDestination::RequiredDecision;
        }
        if context.phase_suppressed {
            return MainViewDestination::Closed;
        }
        if let Some(transient) = self.transient {
            return match transient {
                HudTransientSurface::Component(_) => MainViewDestination::Closed,
                HudTransientSurface::Character(unit) => MainViewDestination::Character(unit),
                HudTransientSurface::Formation if context.formation_available => {
                    MainViewDestination::Formation
                }
                HudTransientSurface::Formation => MainViewDestination::Closed,
            };
        }
        if self.master_suppressed
            || context.viewport == HudViewportMode::Compact
            || (self.main_view == MainViewDestination::Formation && !context.formation_available)
        {
            MainViewDestination::Closed
        } else {
            self.main_view
        }
    }

    /// Effective temporary task surface, or `None` while phase-suppressed or required.
    #[must_use]
    pub fn effective_transient(&self, context: HudContext) -> Option<HudTransientSurface> {
        if self.decision_required() || context.phase_suppressed {
            return None;
        }
        match self.transient {
            Some(HudTransientSurface::Component(component))
                if !context.eligibility.is_eligible(component) =>
            {
                None
            }
            Some(HudTransientSurface::Formation) if !context.formation_available => None,
            transient => transient,
        }
    }

    fn decision_required(&self) -> bool {
        self.main_view == MainViewDestination::RequiredDecision
    }

    fn open_main_view(
        &mut self,
        transient: HudTransientSurface,
        destination: MainViewDestination,
        context: HudContext,
    ) -> HudActionResult {
        if self.decision_required() || context.phase_suppressed {
            return HudActionResult::NoChange;
        }
        if self.master_suppressed
            || context.viewport == HudViewportMode::Compact
            || self.transient.is_some()
        {
            return self.toggle_transient(transient);
        }
        if self.main_view == destination {
            return HudActionResult::NoChange;
        }
        self.main_view = destination;
        HudActionResult::RuntimeChanged
    }

    fn toggle_transient(&mut self, requested: HudTransientSurface) -> HudActionResult {
        if self.transient == Some(requested) {
            self.transient = None;
        } else {
            self.transient = Some(requested);
        }
        HudActionResult::RuntimeChanged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preferences_with(component: HudComponent, enabled: bool) -> HudComponentPreferences {
        let mut preferences = HudComponentPreferences {
            party: false,
            initiative: false,
            activity: false,
            action_bar: false,
        };
        preferences.set_enabled(component, enabled);
        preferences
    }

    fn eligibility_with(component: HudComponent, eligible: bool) -> HudContextEligibility {
        let mut eligibility = HudContextEligibility::none();
        match component {
            HudComponent::Party => eligibility.party = eligible,
            HudComponent::Initiative => eligibility.initiative = eligible,
            HudComponent::Activity => eligibility.activity = eligible,
            HudComponent::ActionBar => eligibility.action_bar = eligible,
        }
        eligibility
    }

    #[test]
    fn default_preferences_match_the_minimal_standard_hud() {
        let preferences = HudComponentPreferences::default();
        assert!(preferences.party);
        assert!(preferences.initiative);
        assert!(!preferences.activity);
        assert!(preferences.action_bar);
    }

    #[test]
    fn ordinary_visibility_truth_table_is_exhaustive() {
        for component in HudComponent::ALL {
            for preference in [false, true] {
                for eligible in [false, true] {
                    for master_suppressed in [false, true] {
                        for phase_suppressed in [false, true] {
                            let mut state = HudState::new(preferences_with(component, preference));
                            if master_suppressed {
                                state.toggle_master();
                            }
                            let context =
                                HudContext::standard(eligibility_with(component, eligible))
                                    .with_phase_suppressed(phase_suppressed);
                            assert_eq!(
                                state.is_component_visible(component, context),
                                preference
                                    && eligible
                                    && !master_suppressed
                                    && !phase_suppressed,
                                "component={component:?}, preference={preference}, eligible={eligible}, master={master_suppressed}, phase={phase_suppressed}",
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn standard_shortcuts_change_only_the_requested_persisted_preference() {
        let context = HudContext::standard(HudContextEligibility::all());
        let mut state = HudState::default();
        for component in HudComponent::ALL {
            let before = state.preferences();
            assert_eq!(
                state.activate_component(component, context),
                HudActionResult::PreferencesChanged
            );
            let after = state.preferences();
            for candidate in HudComponent::ALL {
                assert_eq!(
                    after.is_enabled(candidate),
                    if candidate == component {
                        !before.is_enabled(candidate)
                    } else {
                        before.is_enabled(candidate)
                    }
                );
            }
        }
    }

    #[test]
    fn master_suppression_restores_exact_preferences_after_one_off_summon() {
        let preferences = HudComponentPreferences {
            party: true,
            initiative: false,
            activity: true,
            action_bar: false,
        };
        let context = HudContext::standard(HudContextEligibility::all());
        let mut state = HudState::new(preferences);

        state.toggle_master();
        assert!(HudComponent::ALL
            .into_iter()
            .all(|component| !state.is_component_visible(component, context)));
        assert_eq!(
            state.activate_component(HudComponent::ActionBar, context),
            HudActionResult::RuntimeChanged
        );
        assert!(state.is_component_visible(HudComponent::ActionBar, context));
        assert!(HudComponent::ALL
            .into_iter()
            .all(|component| component == HudComponent::ActionBar
                || !state.is_component_visible(component, context)));
        assert_eq!(state.preferences(), preferences);

        state.toggle_master();
        assert_eq!(state.raw_transient(), None);
        assert_eq!(state.preferences(), preferences);
        for component in HudComponent::ALL {
            assert_eq!(
                state.is_component_visible(component, context),
                preferences.is_enabled(component)
            );
        }
    }

    #[test]
    fn compact_uses_one_temporary_surface_without_mutating_preferences() {
        let context = HudContext::compact(HudContextEligibility::all());
        let mut state = HudState::default();
        let preferences = state.preferences();
        assert!(HudComponent::ALL
            .into_iter()
            .all(|component| !state.is_component_visible(component, context)));

        assert_eq!(
            state.activate_component(HudComponent::Party, context),
            HudActionResult::RuntimeChanged
        );
        assert_eq!(
            state.effective_transient(context),
            Some(HudTransientSurface::Component(HudComponent::Party))
        );
        assert!(state.is_component_visible(HudComponent::Party, context));

        assert_eq!(
            state.open_character(UnitId(42), context),
            HudActionResult::RuntimeChanged
        );
        assert_eq!(
            state.effective_transient(context),
            Some(HudTransientSurface::Character(UnitId(42)))
        );
        assert!(HudComponent::ALL
            .into_iter()
            .all(|component| !state.is_component_visible(component, context)));
        assert_eq!(
            state.effective_main_view(context),
            MainViewDestination::Character(UnitId(42))
        );

        assert_eq!(
            state.close_active_surface(context),
            HudActionResult::RuntimeChanged
        );
        assert_eq!(state.effective_transient(context), None);
        assert_eq!(state.preferences(), preferences);
    }

    #[test]
    fn same_temporary_shortcut_closes_and_a_different_one_replaces() {
        let context = HudContext::compact(HudContextEligibility::all());
        let mut state = HudState::default();
        let preferences = state.preferences();

        state.activate_component(HudComponent::Party, context);
        state.activate_component(HudComponent::Activity, context);
        assert_eq!(
            state.raw_transient(),
            Some(HudTransientSurface::Component(HudComponent::Activity))
        );
        state.activate_component(HudComponent::Activity, context);
        assert_eq!(state.raw_transient(), None);
        assert_eq!(state.preferences(), preferences);
    }

    #[test]
    fn stale_character_closure_never_dismisses_an_unrelated_task() {
        let context = HudContext::compact(HudContextEligibility::all());
        let mut state = HudState::default();
        state.open_character(UnitId(7), context);
        assert_eq!(state.close_character(UnitId(8)), HudActionResult::NoChange);
        assert_eq!(
            state.raw_transient(),
            Some(HudTransientSurface::Character(UnitId(7)))
        );
        assert_eq!(
            state.close_character(UnitId(7)),
            HudActionResult::RuntimeChanged
        );

        state.activate_component(HudComponent::Party, context);
        assert_eq!(state.close_character(UnitId(7)), HudActionResult::NoChange);
        assert_eq!(
            state.raw_transient(),
            Some(HudTransientSurface::Component(HudComponent::Party))
        );
    }

    #[test]
    fn stale_character_closure_clears_stored_and_compact_routes_together() {
        let standard = HudContext::standard(HudContextEligibility::all());
        let compact = HudContext::compact(HudContextEligibility::all());
        let unit = UnitId(7);
        let mut state = HudState::default();

        state.open_character(unit, standard);
        state.open_character(unit, compact);
        assert_eq!(
            state.stored_main_view(),
            MainViewDestination::Character(unit)
        );
        assert_eq!(
            state.raw_transient(),
            Some(HudTransientSurface::Character(unit))
        );

        assert_eq!(state.close_character(unit), HudActionResult::RuntimeChanged);
        assert_eq!(state.stored_main_view(), MainViewDestination::Closed);
        assert_eq!(state.raw_transient(), None);
        assert_eq!(
            state.effective_main_view(standard),
            MainViewDestination::Closed
        );
    }

    #[test]
    fn reconciliation_prunes_invisible_transients_before_escape_can_consume_them() {
        let combat = HudContext::compact(HudContextEligibility::all());
        let exploration = HudContext::compact(HudContextEligibility {
            initiative: false,
            ..HudContextEligibility::all()
        });
        let mut state = HudState::default();
        state.activate_component(HudComponent::Initiative, combat);

        assert_eq!(
            state.reconcile_context(exploration),
            HudActionResult::RuntimeChanged
        );
        assert_eq!(state.raw_transient(), None);
        assert_eq!(
            state.close_active_surface(exploration),
            HudActionResult::NoChange
        );
    }

    #[test]
    fn leaving_compact_closes_its_task_but_master_hidden_summons_remain_owned() {
        let compact = HudContext::compact(HudContextEligibility::all());
        let standard = HudContext::standard(HudContextEligibility::all());
        let mut state = HudState::default();
        state.activate_component(HudComponent::Party, compact);
        state.reconcile_context(standard);
        assert_eq!(state.raw_transient(), None);

        state.toggle_master();
        state.activate_component(HudComponent::Party, standard);
        assert_eq!(state.reconcile_context(standard), HudActionResult::NoChange);
        assert_eq!(
            state.raw_transient(),
            Some(HudTransientSurface::Component(HudComponent::Party))
        );
    }

    #[test]
    fn formation_cannot_leave_a_blank_main_view_after_exploration() {
        let exploration = HudContext::standard(HudContextEligibility::all());
        let combat = exploration.with_formation_available(false);
        let mut state = HudState::default();
        state.open_formation(exploration);

        assert_eq!(
            state.effective_main_view(combat),
            MainViewDestination::Closed
        );
        assert_eq!(
            state.reconcile_context(combat),
            HudActionResult::RuntimeChanged
        );
        assert_eq!(state.stored_main_view(), MainViewDestination::Closed);
        assert_eq!(state.open_formation(combat), HudActionResult::NoChange);
    }

    #[test]
    fn phase_suppression_changes_projection_without_mutating_preferences() {
        let normal = HudContext::standard(HudContextEligibility::all());
        let suppressed = normal.with_phase_suppressed(true);
        let mut state = HudState::default();
        let preferences = state.preferences();
        assert!(state.is_component_visible(HudComponent::Party, normal));
        assert!(HudComponent::ALL
            .into_iter()
            .all(|component| !state.is_component_visible(component, suppressed)));
        assert_eq!(
            state.activate_component(HudComponent::Party, suppressed),
            HudActionResult::NoChange
        );
        assert_eq!(state.preferences(), preferences);
        assert!(state.is_component_visible(HudComponent::Party, normal));

        state.toggle_master();
        state.activate_component(HudComponent::Activity, normal);
        assert_eq!(state.effective_transient(suppressed), None);
        assert_eq!(
            state.effective_transient(normal),
            Some(HudTransientSurface::Component(HudComponent::Activity))
        );
        assert_eq!(state.preferences(), preferences);
    }

    #[test]
    fn required_decision_projects_above_every_suppression_and_cannot_be_replaced() {
        let normal = HudContext::standard(HudContextEligibility::all());
        let compact_suppressed =
            HudContext::compact(HudContextEligibility::none()).with_phase_suppressed(true);
        let mut state = HudState::default();
        state.open_character(UnitId(7), normal);
        assert_eq!(state.require_decision(), HudActionResult::RuntimeChanged);
        state.toggle_master();

        assert_eq!(
            state.effective_main_view(compact_suppressed),
            MainViewDestination::RequiredDecision
        );
        assert_eq!(
            state.activate_component(HudComponent::Party, normal),
            HudActionResult::NoChange
        );
        assert_eq!(
            state.open_character(UnitId(9), normal),
            HudActionResult::NoChange
        );
        assert_eq!(state.open_formation(normal), HudActionResult::NoChange);
        assert_eq!(
            state.close_active_surface(compact_suppressed),
            HudActionResult::NoChange
        );
        assert_eq!(state.raw_transient(), None);
        assert_eq!(
            state.stored_main_view(),
            MainViewDestination::RequiredDecision
        );

        assert_eq!(
            state.resolve_required_decision(),
            HudActionResult::RuntimeChanged
        );
        assert_eq!(
            state.effective_main_view(normal),
            MainViewDestination::Closed
        );
        assert_eq!(state.resolve_required_decision(), HudActionResult::NoChange);
    }

    #[test]
    fn main_view_shortcuts_open_typed_destinations_instead_of_generic_toggles() {
        let context = HudContext::standard(HudContextEligibility::all());
        let mut state = HudState::default();
        assert_eq!(
            state.open_character(UnitId(11), context),
            HudActionResult::RuntimeChanged
        );
        assert_eq!(
            state.effective_main_view(context),
            MainViewDestination::Character(UnitId(11))
        );
        assert_eq!(
            state.open_character(UnitId(11), context),
            HudActionResult::NoChange
        );
        assert_eq!(
            state.open_formation(context),
            HudActionResult::RuntimeChanged
        );
        assert_eq!(
            state.effective_main_view(context),
            MainViewDestination::Formation
        );
        assert_eq!(state.open_formation(context), HudActionResult::NoChange);
        assert_eq!(
            state.close_active_surface(context),
            HudActionResult::RuntimeChanged
        );
        assert_eq!(
            state.effective_main_view(context),
            MainViewDestination::Closed
        );
    }

    #[test]
    fn master_hidden_main_view_summon_is_temporary_and_preserves_stored_destination() {
        let context = HudContext::standard(HudContextEligibility::all());
        let mut state = HudState::default();
        state.open_formation(context);
        state.toggle_master();
        assert_eq!(
            state.effective_main_view(context),
            MainViewDestination::Closed
        );

        state.open_character(UnitId(23), context);
        assert_eq!(
            state.effective_main_view(context),
            MainViewDestination::Character(UnitId(23))
        );
        state.open_character(UnitId(23), context);
        assert_eq!(
            state.effective_main_view(context),
            MainViewDestination::Closed
        );

        state.toggle_master();
        assert_eq!(
            state.effective_main_view(context),
            MainViewDestination::Formation
        );
    }

    #[test]
    fn escape_does_not_discard_a_master_hidden_main_view() {
        let context = HudContext::standard(HudContextEligibility::all());
        let mut state = HudState::default();
        state.open_formation(context);
        state.toggle_master();

        assert_eq!(
            state.close_active_surface(context),
            HudActionResult::NoChange
        );
        state.toggle_master();
        assert_eq!(
            state.effective_main_view(context),
            MainViewDestination::Formation
        );
    }

    #[test]
    fn escape_does_not_discard_a_main_view_hidden_by_compact_projection() {
        let standard = HudContext::standard(HudContextEligibility::all());
        let compact = HudContext::compact(HudContextEligibility::all());
        let unit = UnitId(31);
        let mut state = HudState::default();
        state.open_character(unit, standard);

        assert_eq!(
            state.close_active_surface(compact),
            HudActionResult::NoChange
        );
        assert_eq!(
            state.effective_main_view(standard),
            MainViewDestination::Character(unit)
        );
    }

    #[test]
    fn ineligible_component_cannot_be_toggled_or_summoned() {
        let context = HudContext::standard(HudContextEligibility::none());
        let mut state = HudState::default();
        let preferences = state.preferences();
        assert_eq!(
            state.activate_component(HudComponent::Party, context),
            HudActionResult::NoChange
        );
        state.toggle_master();
        assert_eq!(
            state.activate_component(HudComponent::Party, context),
            HudActionResult::NoChange
        );
        assert_eq!(state.preferences(), preferences);
        assert_eq!(state.raw_transient(), None);
    }
}
