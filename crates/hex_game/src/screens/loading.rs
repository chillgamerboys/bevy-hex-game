//! Waits for assets before letting gameplay spawn.
//!
//! This screen is what makes `OnEnter(Screen::Gameplay)` a safe place to build the
//! world. Before it existed, the grid spawned in `PreStartup` and the player in
//! `Startup`, and the only thing stopping the player system from reading a
//! resource that did not exist yet was the gap between those two schedules —
//! undocumented, unenforced, and one refactor away from a panic.

use bevy::prelude::*;
use hex_assets::{GameAssets, SettingsRegistry, SubstanceFile, SubstanceTable};
use hex_core::Screen;

use super::{despawn_screen, screen_root};
use crate::menus::widgets::UiAssets;
use crate::scenarios::ScenarioContractStatus;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Screen::Loading), spawn_loading);
    app.add_systems(
        PostUpdate,
        (
            crate::scenarios::validate_loaded_scenario,
            enter_gameplay_when_ready,
        )
            .chain()
            .run_if(in_state(Screen::Loading)),
    );
    app.add_systems(OnExit(Screen::Loading), despawn_screen(Screen::Loading));
}

fn spawn_loading(mut commands: Commands, assets: Res<UiAssets>) {
    commands
        .spawn(screen_root(Screen::Loading, "Loading Screen"))
        .with_children(|parent| {
            parent.spawn((
                Text::new("loading..."),
                TextFont {
                    font: assets.body.clone().into(),
                    ..TextFont::from_font_size(24.0)
                },
                TextColor(Color::srgb(0.8, 0.8, 0.8)),
            ));
        });
}

/// Gameplay may only start once asset handles are terminal and every settings file
/// and the derived substance table are present.
///
/// Asking the registry rather than listing settings types keeps this screen ignorant
/// of which crates define what — `MapSettings` lives in `hex_map`, and naming it here
/// would put a dependency on the map into the binary's screen code.
///
/// The registry tracks the raw `SubstanceFile`; `SubstanceTable` is built from it by
/// a separate `Update` system. This check runs in `PostUpdate`, after deferred
/// resource insertions, and waits until the file is unchanged from its point of view.
/// That prevents an existing table from satisfying the check during a hot reload.
fn enter_gameplay_when_ready(
    assets: Res<GameAssets>,
    asset_server: Res<AssetServer>,
    settings: Res<SettingsRegistry>,
    substance_file: Option<Res<SubstanceFile>>,
    substances: Option<Res<SubstanceTable>>,
    scenario_contract: Option<Res<ScenarioContractStatus>>,
    mut next: ResMut<NextState<Screen>>,
) {
    let substances_are_current = substance_file
        .as_ref()
        .is_some_and(|file| !file.is_changed())
        && substances.is_some();

    let scenario_is_valid = scenario_contract
        .as_deref()
        .is_some_and(|status| *status == ScenarioContractStatus::Ready);

    if assets.is_ready(&asset_server)
        && settings.all_loaded()
        && substances_are_current
        && scenario_is_valid
    {
        next.set(Screen::Gameplay);
    }
}

#[cfg(test)]
mod tests {
    use bevy::asset::AssetPlugin;
    use bevy::platform::collections::HashMap;
    use bevy::state::app::StatesPlugin;
    use hex_assets::Substance;

    use super::*;

    fn substance_file(name: &str) -> SubstanceFile {
        let mut substances = HashMap::default();
        for (entry, solid, diggable) in [("air", false, false), (name, true, true)] {
            substances.insert(
                entry.to_owned(),
                Substance {
                    color: (0.5, 0.5, 0.5),
                    solid,
                    diggable,
                },
            );
        }
        SubstanceFile { substances }
    }

    fn queue_replacement(mut commands: Commands) {
        commands.insert_resource(substance_file("clay"));
    }

    /// A settings replacement queued in `Update` must be visible before readiness is
    /// evaluated, or gameplay can start with the previous derived table.
    #[test]
    fn hot_reload_waits_for_the_substance_table_to_catch_up() {
        let original = substance_file("stone");
        let original_table = SubstanceTable::from_file(&original);

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), StatesPlugin));
        app.insert_state(Screen::Loading);
        app.insert_resource(UiAssets {
            display: Handle::default(),
            body: Handle::default(),
            hex_cell: Handle::default(),
        });
        app.init_resource::<SettingsRegistry>();
        app.insert_resource(GameAssets {
            hex_tile: Handle::default(),
            player_pieces: [Handle::default(), Handle::default()],
        });
        app.insert_resource(ScenarioContractStatus::Ready);
        app.insert_resource(original);
        plugin(&mut app);

        // Prime this system's change detection without a table, so it cannot enter
        // gameplay yet.
        app.update();
        assert!(matches!(
            app.world().resource::<NextState<Screen>>(),
            &NextState::Unchanged
        ));

        app.insert_resource(original_table);
        app.add_systems(Update, queue_replacement);
        app.update();

        let world = app.world();
        assert!(
            world
                .resource::<SubstanceFile>()
                .substances
                .contains_key("clay"),
            "the replacement file should have been applied"
        );
        assert!(
            world.resource::<SubstanceTable>().id("clay").is_none(),
            "the table should still represent the previous file in this frame"
        );
        assert!(
            matches!(world.resource::<NextState<Screen>>(), &NextState::Unchanged),
            "gameplay must wait for the derived table to catch up"
        );
    }
}
