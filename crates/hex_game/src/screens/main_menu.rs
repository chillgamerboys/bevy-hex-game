//! Main Menu application adapters.
//!
//! Renderer-free route state lives in `hex_gameplay_model`; this module projects
//! persistence and setup failures into immutable UI views and performs typed effects.

use bevy::prelude::*;
use hex_assets::{ElementCatalog, LatticeLibrary};
use hex_core::{GameplaySetupFailure, InputAction, InputBindings, Screen};
use hex_gameplay_model::{CreatorEntry, CreatorOrigin, MainMenuModel, MainMenuRoute};
use hex_ui::{MainMenuIntent, MainMenuView, UiIntent, UiSystems};

use crate::save::CampaignStore;

use super::creator::CreatorEntryRequest;

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<InputBindings>()
        .init_resource::<MainMenuModel>()
        .add_systems(
            Update,
            handle_intents
                .after(UiSystems::EmitIntents)
                .run_if(in_state(Screen::Title)),
        )
        .add_systems(
            Update,
            (publish_view, handle_input).run_if(in_state(Screen::Title)),
        );
}

fn publish_view(
    model: Res<MainMenuModel>,
    campaigns: Res<CampaignStore>,
    lattices: Option<Res<LatticeLibrary>>,
    elements: Option<Res<ElementCatalog>>,
    failure: Option<Res<GameplaySetupFailure>>,
    #[cfg(feature = "arena-prototype")] battle: Option<Res<crate::battle_launcher::BattleLauncher>>,
    mut view: ResMut<MainMenuView>,
) {
    #[cfg(feature = "arena-prototype")]
    let (battle_running, battle_launch_error) = battle.as_deref().map_or((false, None), |battle| {
        (battle.running(), battle.error().map(str::to_owned))
    });
    #[cfg(not(feature = "arena-prototype"))]
    let (battle_running, battle_launch_error) = (false, None);
    let next = MainMenuView {
        route: model.route,
        setup_failure: failure.as_deref().map(|failure| failure.reason.clone()),
        campaign_slots: campaigns.slot_views(lattices.as_deref(), elements.as_deref()),
        battle_mode_available: cfg!(feature = "arena-prototype"),
        battle_running,
        battle_launch_error,
    };
    if *view != next {
        *view = next;
    }
}

fn handle_intents(
    mut intents: MessageReader<UiIntent>,
    mut model: ResMut<MainMenuModel>,
    mut commands: Commands,
    mut next: ResMut<NextState<Screen>>,
    #[cfg(feature = "arena-prototype")] battle: Option<Res<crate::battle_launcher::BattleLauncher>>,
) {
    #[cfg(feature = "arena-prototype")]
    if battle.as_deref().is_some_and(|battle| battle.running()) {
        intents.clear();
        return;
    }
    for intent in intents.read() {
        let UiIntent::MainMenu(intent) = intent else {
            continue;
        };
        match intent {
            MainMenuIntent::OpenBattleMode => {
                #[cfg(feature = "arena-prototype")]
                if model.route == MainMenuRoute::Root && battle.is_some() {
                    commands.insert_resource(crate::battle_launcher::BattleLaunchRequest);
                    // A failed spawn must not replay another queued menu click.
                    intents.clear();
                    break;
                }
            }
            MainMenuIntent::OpenCampaign => model.show(MainMenuRoute::Campaign),
            MainMenuIntent::OpenMultiplayer => {
                model.show(MainMenuRoute::Multiplayer);
                next.set(Screen::Multiplayer);
            }
            MainMenuIntent::OpenSandbox => {
                let _consumed = model.back();
                next.set(Screen::Sandbox);
            }
            MainMenuIntent::OpenTools => model.show(MainMenuRoute::Tools),
            MainMenuIntent::OpenSettings => {
                let _consumed = model.back();
                next.set(Screen::Settings);
            }
            MainMenuIntent::OpenCharacterCreator => {
                commands.insert_resource(CreatorEntryRequest(CreatorEntry::CharacterLibrary(
                    CreatorOrigin::Tools,
                )));
                next.set(Screen::CharacterCreator);
            }
            MainMenuIntent::OpenSpellCreator => {
                commands.insert_resource(CreatorEntryRequest(CreatorEntry::SpellLibrary(
                    CreatorOrigin::Tools,
                )));
                next.set(Screen::SpellCreator);
            }
            MainMenuIntent::OpenVfxTuner => next.set(Screen::VfxTuner),
            MainMenuIntent::Back => {
                let _consumed = model.back();
            }
            MainMenuIntent::NewCampaign(_) | MainMenuIntent::ContinueCampaign(_) => {
                // Campaign persistence consumes these through its own MessageReader.
            }
        }
    }
}

fn handle_input(
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<InputBindings>,
    mut model: ResMut<MainMenuModel>,
    mut exit: MessageWriter<AppExit>,
    #[cfg(feature = "arena-prototype")] battle: Option<Res<crate::battle_launcher::BattleLauncher>>,
) {
    #[cfg(feature = "arena-prototype")]
    if battle.as_deref().is_some_and(|battle| battle.running()) {
        return;
    }
    if !bindings.just_pressed(&keys, InputAction::Cancel) {
        return;
    }
    if model.route == MainMenuRoute::Root {
        exit.write(AppExit::Success);
    } else {
        let _consumed = model.back();
    }
}

#[cfg(test)]
mod tests {
    use bevy::state::app::StatesPlugin;
    use bevy::MinimalPlugins;

    use super::*;

    fn navigation_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .insert_state(Screen::Title)
            .init_resource::<MainMenuModel>()
            .add_message::<UiIntent>()
            .add_systems(Update, handle_intents);
        app
    }

    #[cfg(feature = "arena-prototype")]
    #[test]
    fn battle_request_is_root_only_and_keeps_the_parent_at_title() {
        let mut app = navigation_app();
        app.init_resource::<crate::battle_launcher::BattleLauncher>();
        app.world_mut()
            .resource_mut::<MainMenuModel>()
            .show(MainMenuRoute::Tools);
        app.world_mut()
            .write_message(UiIntent::MainMenu(MainMenuIntent::OpenBattleMode));
        app.update();
        assert!(!app
            .world()
            .contains_resource::<crate::battle_launcher::BattleLaunchRequest>());

        app.world_mut()
            .resource_mut::<MainMenuModel>()
            .show(MainMenuRoute::Root);
        app.world_mut()
            .write_message(UiIntent::MainMenu(MainMenuIntent::OpenBattleMode));
        app.world_mut()
            .write_message(UiIntent::MainMenu(MainMenuIntent::OpenSandbox));
        app.update();
        app.update();
        assert!(app
            .world()
            .contains_resource::<crate::battle_launcher::BattleLaunchRequest>());
        assert_eq!(
            app.world().resource::<MainMenuModel>().route,
            MainMenuRoute::Root
        );
        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Title
        );
    }

    #[test]
    fn unavailable_battle_request_does_not_navigate() {
        let mut app = navigation_app();
        app.world_mut()
            .write_message(UiIntent::MainMenu(MainMenuIntent::OpenBattleMode));
        app.update();
        app.update();
        assert_eq!(
            app.world().resource::<MainMenuModel>().route,
            MainMenuRoute::Root
        );
        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Title
        );
        #[cfg(feature = "arena-prototype")]
        assert!(!app
            .world()
            .contains_resource::<crate::battle_launcher::BattleLaunchRequest>());
    }

    #[test]
    fn cold_main_menu_intents_drive_real_tools_back_and_sandbox_state_transitions() {
        let mut app = navigation_app();
        app.world_mut()
            .write_message(UiIntent::MainMenu(MainMenuIntent::OpenTools));
        app.update();
        assert_eq!(
            app.world().resource::<MainMenuModel>().route,
            MainMenuRoute::Tools
        );

        app.world_mut()
            .write_message(UiIntent::MainMenu(MainMenuIntent::Back));
        app.update();
        assert_eq!(
            app.world().resource::<MainMenuModel>().route,
            MainMenuRoute::Root
        );

        app.world_mut()
            .write_message(UiIntent::MainMenu(MainMenuIntent::OpenSandbox));
        app.update();
        app.update();
        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Sandbox
        );
    }

    #[test]
    fn multiplayer_route_uses_its_own_screen() {
        let mut app = navigation_app();
        app.world_mut()
            .write_message(UiIntent::MainMenu(MainMenuIntent::OpenMultiplayer));
        app.update();
        app.update();

        assert_eq!(
            app.world().resource::<MainMenuModel>().route,
            MainMenuRoute::Multiplayer
        );
        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Multiplayer
        );
    }
}
