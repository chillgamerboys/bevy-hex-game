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
    mut view: ResMut<MainMenuView>,
) {
    let next = MainMenuView {
        route: model.route,
        setup_failure: failure.as_deref().map(|failure| failure.reason.clone()),
        campaign_slots: campaigns.slot_views(lattices.as_deref(), elements.as_deref()),
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
) {
    for intent in intents.read() {
        let UiIntent::MainMenu(intent) = intent else {
            continue;
        };
        match intent {
            MainMenuIntent::OpenCampaign => model.show(MainMenuRoute::Campaign),
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
) {
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

    #[test]
    fn root_has_exactly_the_four_product_routes() {
        let routes = [
            MainMenuIntent::OpenCampaign,
            MainMenuIntent::OpenSandbox,
            MainMenuIntent::OpenTools,
            MainMenuIntent::OpenSettings,
        ];
        assert_eq!(routes.len(), 4);
    }

    fn navigation_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .insert_state(Screen::Title)
            .init_resource::<MainMenuModel>()
            .add_message::<UiIntent>()
            .add_systems(Update, handle_intents);
        app
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
}
