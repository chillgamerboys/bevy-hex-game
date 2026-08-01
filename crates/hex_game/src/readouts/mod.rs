//! Gameplay readouts: lattices, initiative, and the disclosed combat history.

use bevy::prelude::*;
use hex_combat::{CombatSystems, EncounterResolution};
use hex_core::{AppSystems, GameplayPhase, GameplaySetup, GameplaySystems, Screen};
pub(crate) use hex_ui::{HudElement, UiHudSetup as HudSetup};

mod badges;
mod context;
mod initiative;
mod lattice;
mod log;

pub(crate) use context::{GameplayUiContext, UiUnitIdentity};
pub(crate) use lattice::DisableSelection;

/// Whether ordinary gameplay chrome is currently shown.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HudVisibility {
    pub(crate) shown: bool,
}

impl Default for HudVisibility {
    fn default() -> Self {
        Self { shown: true }
    }
}

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<hex_core::InputBindings>();
    app.init_resource::<HudVisibility>()
        .configure_sets(
            Update,
            (
                GameplaySystems::Selection,
                GameplaySystems::Casting,
                GameplaySystems::UiContext,
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
        .add_plugins(badges::plugin)
        .add_plugins((lattice::plugin, initiative::plugin, log::plugin))
        .add_systems(OnEnter(Screen::Gameplay), reset_hud)
        // Deliberately outside `PausableSystems`: hiding chrome does not advance
        // the simulation and remains available while a decision is open.
        .add_systems(
            Update,
            toggle_hud
                .run_if(resource_equals(GameplayPhase::Active))
                .in_set(AppSystems::RecordInput)
                .run_if(in_state(Screen::Gameplay)),
        );
    add_chrome_publisher(app);
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

fn reset_hud(mut hud: ResMut<HudVisibility>) {
    hud.shown = true;
}

fn toggle_hud(
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<hex_core::InputBindings>,
    mut hud: ResMut<HudVisibility>,
) {
    if bindings.just_pressed(&keys, hex_core::InputAction::ToggleHud) {
        hud.shown = !hud.shown;
    }
}

fn publish_hud_view(
    hud: Res<HudVisibility>,
    phase: Res<GameplayPhase>,
    selection: Res<lattice::DisableSelection>,
    resolution: Option<Res<EncounterResolution>>,
    mut view: ResMut<hex_ui::GameplayChromeView>,
) {
    let next = hex_ui::GameplayChromeView {
        shown: hud.shown && *phase == GameplayPhase::Active,
        decision_required: *phase == GameplayPhase::Active && selection.is_active(),
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

    #[test]
    fn read_only_hud_surfaces_pass_world_picks_through() {
        assert_eq!(hex_ui::READ_ONLY_HUD, Pickable::IGNORE);
    }

    #[test]
    fn hiding_the_hud_publishes_visibility_without_mutating_ui_nodes() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<HudVisibility>()
            .init_resource::<GameplayPhase>()
            .init_resource::<lattice::DisableSelection>()
            .init_resource::<hex_ui::GameplayChromeView>()
            .add_systems(Update, publish_hud_view);

        app.world_mut().resource_mut::<HudVisibility>().shown = false;
        app.update();

        assert!(!app.world().resource::<hex_ui::GameplayChromeView>().shown);
    }

    #[test]
    fn an_active_decision_lattice_stays_visible_when_the_hud_is_hidden() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<HudVisibility>()
            .init_resource::<GameplayPhase>()
            .init_resource::<lattice::DisableSelection>()
            .init_resource::<hex_ui::GameplayChromeView>()
            .add_systems(Update, publish_hud_view);
        app.world_mut().resource_mut::<HudVisibility>().shown = false;
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
        assert!(!view.shown);
        assert!(view.decision_required);
    }

    #[test]
    fn a_choice_activated_during_update_reaches_the_same_frame_render() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .init_state::<Screen>()
            .insert_state(Screen::Gameplay)
            .init_resource::<HudVisibility>()
            .init_resource::<GameplayPhase>()
            .init_resource::<lattice::DisableSelection>()
            .init_resource::<hex_ui::GameplayChromeView>()
            .init_resource::<RenderedChrome>()
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
            app.world().resource::<RenderedChrome>().0.decision_required,
            "a choice activated during gameplay reconciliation must suppress competing controls before UI rendering in the same frame"
        );
    }

    #[test]
    fn a_resolved_encounter_hides_stale_decision_surfaces() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<HudVisibility>()
            .init_resource::<GameplayPhase>()
            .init_resource::<lattice::DisableSelection>()
            .init_resource::<hex_ui::GameplayChromeView>()
            .insert_resource(EncounterResolution(Some(
                hex_combat::EncounterOutcome::Victory,
            )))
            .add_systems(Update, publish_hud_view);

        app.update();

        assert!(
            app.world()
                .resource::<hex_ui::GameplayChromeView>()
                .encounter_complete
        );
    }

    #[test]
    fn deployment_collapses_ordinary_chrome_without_changing_the_player_preference() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<HudVisibility>()
            .insert_resource(GameplayPhase::Deployment)
            .init_resource::<lattice::DisableSelection>()
            .init_resource::<hex_ui::GameplayChromeView>()
            .add_systems(Update, publish_hud_view);

        app.update();

        assert!(app.world().resource::<HudVisibility>().shown);
        assert!(!app.world().resource::<hex_ui::GameplayChromeView>().shown);
    }
}
