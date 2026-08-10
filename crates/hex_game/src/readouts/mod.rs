//! Gameplay readouts: lattices, initiative, and the disclosed combat history.

use bevy::prelude::*;
use hex_combat::{CombatSystems, EncounterResolution};
use hex_core::{AppSystems, GameplayPhase, GameplaySetup, GameplaySystems, Mode, Screen, UnitId};
use hex_gameplay_model::{
    HudActionResult, HudComponent, HudContext, HudContextEligibility, HudState, HudViewportMode,
};
pub(crate) use hex_ui::UiHudSetup as HudSetup;
use hex_units::{Faction, Party, UnitRegistry};

mod context;
mod initiative;
mod lattice;
mod log;
mod spatial_feedback;

pub(crate) use context::{GameplayUiContext, TargetProvenance, UiUnitIdentity};
pub(crate) use lattice::DisableSelection;
pub(crate) use log::ActivityNotice;

/// Presentation-only unit identity inspected by Party, Initiative, and Main View.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HudInspection {
    pub(crate) subject: Option<UnitId>,
}

/// Canonical effective HUD context published before screen-level Escape handling.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GameplayHudContext(pub(crate) HudContext);

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<hex_core::InputBindings>();
    app.init_resource::<HudState>()
        .init_resource::<HudInspection>()
        .init_resource::<GameplayHudContext>()
        .configure_sets(
            Update,
            (
                GameplaySystems::Selection,
                GameplaySystems::Casting,
                GameplaySystems::UiContext,
                GameplaySystems::WorldFeedbackRequests,
                GameplaySystems::WorldFeedback,
            )
                .chain()
                .after(CombatSystems::Advance),
        )
        .configure_sets(
            OnEnter(Screen::Gameplay),
            (HudSetup::Frame, HudSetup::Panels)
                .chain()
                .in_set(GameplaySetup::View),
        )
        .add_plugins(context::plugin)
        .add_plugins(spatial_feedback::plugin)
        .add_plugins((lattice::plugin, initiative::plugin, log::plugin))
        .add_systems(OnEnter(Screen::Gameplay), reset_hud_runtime)
        // Deliberately outside `PausableSystems`: hiding chrome does not advance
        // the simulation and remains available while a decision is open.
        .add_systems(
            Update,
            handle_hud_shortcuts
                .run_if(resource_equals(GameplayPhase::Active))
                .in_set(AppSystems::RecordInput)
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(
            Update,
            handle_inspection_input
                .after(hex_ui::UiSystems::EmitIntents)
                .after(AppSystems::RecordInput)
                .before(AppSystems::Update)
                .before(hex_ui::UiSystems::Render)
                .run_if(resource_equals(GameplayPhase::Active))
                .run_if(in_state(Screen::Gameplay)),
        );
    add_chrome_publisher(app);
}

#[expect(
    clippy::too_many_arguments,
    reason = "inspection validates roster identity, disclosure, and responsive HUD context without mutating gameplay authority"
)]
fn handle_inspection_input(
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<hex_core::InputBindings>,
    metrics: Res<hex_ui::ResolvedUiMetrics>,
    phase: Res<GameplayPhase>,
    mode: Res<State<Mode>>,
    resolution: Option<Res<EncounterResolution>>,
    party: Res<Party>,
    registry: Res<UnitRegistry>,
    knowledge: Option<Res<hex_perception::FactionMapKnowledge>>,
    factions: Query<&Faction>,
    ui_context: Res<GameplayUiContext>,
    selection: Res<lattice::DisableSelection>,
    mut intents: MessageReader<hex_ui::UiIntent>,
    mut center_camera: MessageWriter<hex_core::CenterInspectionCamera>,
    mut hud: ResMut<HudState>,
    mut inspection: ResMut<HudInspection>,
) {
    let context = hud_context(
        *metrics,
        *phase,
        *mode.get(),
        resolution.as_deref(),
        !party.members.is_empty(),
        ui_context.acting.as_ref().map(|actor| actor.faction),
        false,
        selection.is_active(),
    );
    let mut requested = Vec::new();
    for action in hex_core::InputAction::PARTY_SLOTS {
        if bindings.just_pressed(&keys, action) {
            if let Some(unit) = action
                .party_slot_index()
                .and_then(|slot| party.members.get(slot))
            {
                requested.push(*unit);
            }
        }
    }
    for intent in intents.read() {
        match intent {
            hex_ui::UiIntent::Party(hex_ui::PartyIntent::ActivateMember(slot)) => {
                if let Some(unit) = party.members.get(*slot) {
                    requested.push(*unit);
                }
            }
            hex_ui::UiIntent::Initiative(hex_ui::InitiativeIntent::ActivateUnit(unit)) => {
                requested.push(*unit);
            }
            _ => {}
        }
    }
    for unit in requested {
        let Some(entity) = registry.entity_of(unit) else {
            continue;
        };
        let Ok(faction) = factions.get(entity) else {
            continue;
        };
        let disclosed = *faction == Faction::Player
            || knowledge
                .as_deref()
                .is_some_and(|knowledge| knowledge.faction(Faction::Player).unit(unit).is_some());
        if !disclosed {
            continue;
        }
        if inspection.subject == Some(unit) {
            let _ = hud.open_character(unit, context);
        } else {
            inspection.subject = Some(unit);
            let _ = hud.clear_active_surface();
            center_camera.write(hex_core::CenterInspectionCamera::new(unit));
        }
    }
}

fn add_chrome_publisher(app: &mut App) {
    app.add_systems(
        Update,
        publish_hud_view
            .in_set(AppSystems::Update)
            .after(GameplaySystems::UiContext)
            .before(hex_ui::UiSystems::Render)
            .run_if(in_state(Screen::Gameplay)),
    );
}

fn reset_hud_runtime(
    mut hud: ResMut<HudState>,
    mut inspection: ResMut<HudInspection>,
    mut context: ResMut<GameplayHudContext>,
) {
    *hud = HudState::new(hud.preferences());
    *inspection = HudInspection::default();
    *context = GameplayHudContext::default();
}

#[expect(
    clippy::too_many_arguments,
    reason = "HUD shortcuts resolve typed state from independent gameplay and responsive contexts"
)]
fn handle_hud_shortcuts(
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<hex_core::InputBindings>,
    metrics: Res<hex_ui::ResolvedUiMetrics>,
    phase: Res<GameplayPhase>,
    mode: Res<State<Mode>>,
    resolution: Option<Res<EncounterResolution>>,
    party: Res<Party>,
    ui_context: Res<GameplayUiContext>,
    actions: Res<hex_ui::GameplayHudView>,
    selection: Res<lattice::DisableSelection>,
    mut hud: ResMut<HudState>,
    inspection: Res<HudInspection>,
    mut preferences: ResMut<crate::preferences::UserPreferences>,
    mut dirty: ResMut<crate::preferences::PreferencesDirty>,
) {
    let context = hud_context(
        *metrics,
        *phase,
        *mode.get(),
        resolution.as_deref(),
        !party.members.is_empty(),
        ui_context.acting.as_ref().map(|actor| actor.faction),
        !actions.actions.is_empty(),
        selection.is_active(),
    );
    if !context.phase_suppressed && bindings.just_pressed(&keys, hex_core::InputAction::ToggleHud) {
        let _ = hud.toggle_master();
    }
    let mut preferences_changed = false;
    for (action, component) in [
        (hex_core::InputAction::ToggleParty, HudComponent::Party),
        (
            hex_core::InputAction::ToggleInitiative,
            HudComponent::Initiative,
        ),
        (
            hex_core::InputAction::ToggleActivity,
            HudComponent::Activity,
        ),
        (
            hex_core::InputAction::ToggleActionBar,
            HudComponent::ActionBar,
        ),
    ] {
        if bindings.just_pressed(&keys, action) {
            preferences_changed |=
                hud.activate_component(component, context) == HudActionResult::PreferencesChanged;
        }
    }
    if bindings.just_pressed(&keys, hex_core::InputAction::OpenCharacterView) {
        let subject = inspection
            .subject
            .or_else(|| {
                ui_context
                    .selected_ally
                    .as_ref()
                    .map(|identity| identity.unit)
            })
            .or_else(|| party.members.first().copied());
        if let Some(unit) = subject {
            let _ = hud.open_character(unit, context);
        }
    }
    if bindings.just_pressed(&keys, hex_core::InputAction::OpenFormationView)
        && *mode.get() == Mode::Exploring
    {
        let _ = hud.open_formation(context);
    }
    if preferences_changed {
        preferences.hud_visibility = hud.preferences();
        dirty.0 = true;
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the pure HUD context intentionally receives each independent eligibility fact"
)]
fn hud_context(
    metrics: hex_ui::ResolvedUiMetrics,
    phase: GameplayPhase,
    mode: Mode,
    resolution: Option<&EncounterResolution>,
    has_party: bool,
    acting: Option<Faction>,
    has_actions: bool,
    decision_required: bool,
) -> HudContext {
    let encounter_complete = resolution.is_some_and(EncounterResolution::is_resolved);
    HudContext {
        viewport: if metrics.viewport == hex_ui::UiViewportClass::Compact {
            HudViewportMode::Compact
        } else {
            HudViewportMode::Standard
        },
        eligibility: HudContextEligibility {
            party: has_party,
            initiative: mode == Mode::Combat,
            activity: true,
            action_bar: has_actions
                && !decision_required
                && (mode == Mode::Exploring || acting == Some(Faction::Player)),
        },
        phase_suppressed: phase != GameplayPhase::Active || encounter_complete,
        formation_available: mode == Mode::Exploring,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "chrome is one atomic projection of state, responsive context, and gameplay eligibility"
)]
fn publish_hud_view(
    mut hud: ResMut<HudState>,
    metrics: Res<hex_ui::ResolvedUiMetrics>,
    phase: Res<GameplayPhase>,
    mode: Res<State<Mode>>,
    party: Res<Party>,
    ui_context: Res<GameplayUiContext>,
    actions: Res<hex_ui::GameplayHudView>,
    selection: Res<lattice::DisableSelection>,
    resolution: Option<Res<EncounterResolution>>,
    mut current_context: ResMut<GameplayHudContext>,
    mut view: ResMut<hex_ui::GameplayChromeView>,
) {
    let encounter_complete = resolution
        .as_deref()
        .is_some_and(EncounterResolution::is_resolved);
    let decision_required =
        *phase == GameplayPhase::Active && !encounter_complete && selection.is_active();
    if decision_required {
        let _ = hud.require_decision();
    } else {
        let _ = hud.resolve_required_decision();
    }
    let context = hud_context(
        *metrics,
        *phase,
        *mode.get(),
        resolution.as_deref(),
        !party.members.is_empty(),
        ui_context.acting.as_ref().map(|actor| actor.faction),
        !actions.actions.is_empty(),
        decision_required,
    );
    if current_context.0 != context {
        current_context.0 = context;
    }
    let _ = hud.reconcile_context(context);
    let next = hex_ui::GameplayChromeView {
        party_shown: hud.is_component_visible(HudComponent::Party, context),
        initiative_shown: hud.is_component_visible(HudComponent::Initiative, context),
        activity_shown: hud.is_component_visible(HudComponent::Activity, context),
        action_bar_shown: hud.is_component_visible(HudComponent::ActionBar, context),
        main_view: hud.effective_main_view(context),
        terrain_health_shown: !hud.master_suppressed() && !context.phase_suppressed,
        encounter_complete: resolution
            .as_deref()
            .is_some_and(|value| value.is_resolved()),
    };
    if *view != next {
        *view = next;
    }
}

#[cfg(test)]
mod tests {
    use bevy::state::app::StatesPlugin;
    use bevy::MinimalPlugins;

    use super::*;

    #[derive(Resource, Default)]
    struct RenderedChrome(hex_ui::GameplayChromeView);

    fn activate_required_choice(mut selection: ResMut<lattice::DisableSelection>) {
        selection.decision = Some(lattice::DisableDecision {
            decider: hex_core::UnitId(3),
            target: hex_core::UnitId(3),
            owed: 1,
            restoring: false,
            live: vec![hex_core::LatticeCoord::ORIGIN],
        });
    }

    fn reconcile_ui_context() {}

    fn capture_rendered_chrome(
        view: Res<hex_ui::GameplayChromeView>,
        mut rendered: ResMut<RenderedChrome>,
    ) {
        rendered.0 = *view;
    }

    fn chrome_app(phase: GameplayPhase) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .init_state::<Screen>()
            .insert_state(Screen::Gameplay)
            .add_sub_state::<Mode>()
            .insert_resource(phase)
            .init_resource::<HudState>()
            .init_resource::<hex_ui::ResolvedUiMetrics>()
            .insert_resource(Party {
                members: vec![UnitId(3)],
            })
            .init_resource::<GameplayUiContext>()
            .init_resource::<hex_ui::GameplayHudView>()
            .init_resource::<lattice::DisableSelection>()
            .init_resource::<GameplayHudContext>()
            .init_resource::<hex_ui::GameplayChromeView>();
        app
    }

    #[test]
    fn read_only_hud_surfaces_pass_world_picks_through() {
        assert_eq!(hex_ui::READ_ONLY_HUD, Pickable::IGNORE);
    }

    #[test]
    fn master_hiding_publishes_no_ordinary_or_terrain_chrome_without_changing_preferences() {
        let mut app = chrome_app(GameplayPhase::Active);
        app.add_systems(Update, publish_hud_view);
        let preferences = app.world().resource::<HudState>().preferences();
        app.world_mut().resource_mut::<HudState>().toggle_master();
        app.update();

        let view = app.world().resource::<hex_ui::GameplayChromeView>();
        assert!(!view.any_ordinary_shown());
        assert!(!view.terrain_health_shown);
        assert_eq!(
            app.world().resource::<HudState>().preferences(),
            preferences
        );
    }

    #[test]
    fn an_active_decision_lattice_stays_visible_when_the_hud_is_hidden() {
        let mut app = chrome_app(GameplayPhase::Active);
        app.add_systems(Update, publish_hud_view);
        app.world_mut().resource_mut::<HudState>().toggle_master();
        app.world_mut()
            .resource_mut::<lattice::DisableSelection>()
            .decision = Some(lattice::DisableDecision {
            decider: hex_core::UnitId(3),
            target: hex_core::UnitId(3),
            owed: 1,
            restoring: false,
            live: vec![hex_core::LatticeCoord::ORIGIN],
        });
        app.update();

        let view = app.world().resource::<hex_ui::GameplayChromeView>();
        assert!(!view.any_ordinary_shown());
        assert!(view.decision_required());
        assert_eq!(
            view.main_view,
            hex_gameplay_model::MainViewDestination::RequiredDecision
        );
    }

    #[test]
    fn a_choice_activated_during_update_reaches_the_same_frame_render() {
        let mut app = chrome_app(GameplayPhase::Active);
        app.init_resource::<RenderedChrome>()
            .configure_sets(
                Update,
                (AppSystems::RecordInput, AppSystems::Update).chain(),
            )
            .configure_sets(
                Update,
                GameplaySystems::UiContext.in_set(AppSystems::Update),
            )
            .add_systems(
                Update,
                activate_required_choice
                    .in_set(AppSystems::Update)
                    .before(GameplaySystems::UiContext),
            )
            .add_systems(
                Update,
                reconcile_ui_context.in_set(GameplaySystems::UiContext),
            );
        add_chrome_publisher(&mut app);
        app.add_systems(
            Update,
            capture_rendered_chrome.in_set(hex_ui::UiSystems::Render),
        );

        app.update();

        assert!(
            app.world().resource::<RenderedChrome>().0.decision_required(),
            "a choice activated during gameplay reconciliation must suppress competing controls before UI rendering in the same frame"
        );
    }

    #[test]
    fn a_resolved_encounter_hides_stale_decision_surfaces() {
        let mut app = chrome_app(GameplayPhase::Active);
        app.insert_resource(EncounterResolution(Some(
            hex_combat::EncounterOutcome::Victory,
        )))
        .add_systems(Update, publish_hud_view);

        app.update();

        let view = app.world().resource::<hex_ui::GameplayChromeView>();
        assert!(view.encounter_complete);
        assert!(!view.any_ordinary_shown());
        assert!(!view.terrain_health_shown);
    }

    #[test]
    fn a_resolved_outcome_refuses_hidden_master_toggle_input() {
        let mut app = chrome_app(GameplayPhase::Active);
        app.insert_resource(EncounterResolution(Some(
            hex_combat::EncounterOutcome::Victory,
        )))
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<hex_core::InputBindings>()
        .init_resource::<HudInspection>()
        .init_resource::<crate::preferences::UserPreferences>()
        .init_resource::<crate::preferences::PreferencesDirty>()
        .add_systems(Update, handle_hud_shortcuts);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyH);

        app.update();

        assert!(!app.world().resource::<HudState>().master_suppressed());
    }

    #[test]
    fn deployment_collapses_ordinary_chrome_without_changing_the_player_preference() {
        let mut app = chrome_app(GameplayPhase::Deployment);
        app.add_systems(Update, publish_hud_view);
        let preferences = app.world().resource::<HudState>().preferences();

        app.update();

        let view = app.world().resource::<hex_ui::GameplayChromeView>();
        assert!(!view.any_ordinary_shown());
        assert!(!view.terrain_health_shown);
        assert_eq!(
            app.world().resource::<HudState>().preferences(),
            preferences
        );
    }
}
