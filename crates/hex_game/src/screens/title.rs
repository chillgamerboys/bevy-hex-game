//! Main menu: pick a scenario and start it.
//!
//! One button per scenario in `assets/config/scenarios.ron`, so adding a map to the
//! menu is a content change rather than a code one.
//!
//! # The list is rebuilt, not spawned once
//!
//! `Screen::Title` is reached on a **wall-clock timer** from the splash screen, not on
//! a load gate, so it is genuinely reachable before `scenarios.ron` has finished
//! parsing. A system that spawned the buttons on entry would show an empty menu on a
//! cold disk and never correct itself. Rebuilding when the library appears or changes
//! covers that, hot reload, and nothing else needing to know the difference.
//!
//! For the same reason the library is taken as `Option<Res<_>>` everywhere here. A
//! plain `Res<T>` on a resource that does not exist yet is a panic, and this project
//! has already shipped that crash once on this very screen.

use bevy::prelude::*;
use hex_assets::{ScenarioLibrary, SelectedScenario};
use hex_core::Screen;

use crate::menus::widgets::{blurb, button, label, LABEL, MUTED};

use super::{despawn_screen, screen_root};

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Screen::Title), spawn_title);
    app.add_systems(
        Update,
        (rebuild_scenario_list, start_chosen_scenario, handle_input)
            .run_if(in_state(Screen::Title)),
    );
    app.add_systems(OnExit(Screen::Title), despawn_screen(Screen::Title));
}

/// The node the scenario buttons hang off.
#[derive(Component)]
struct ScenarioList;

/// A button that starts the scenario at this index.
#[derive(Component)]
struct StartsScenario(usize);

/// The line that stands in for the list until the library has loaded.
#[derive(Component)]
struct ListPlaceholder;

fn spawn_title(mut commands: Commands) {
    commands
        .spawn(screen_root(Screen::Title, "Title Screen"))
        .with_children(|parent| {
            parent.spawn((
                Text::new("hex game"),
                TextFont::from_font_size(56.0),
                TextColor(LABEL),
            ));
            parent
                .spawn((
                    Name::new("Scenario List"),
                    ScenarioList,
                    Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(10.0),
                        align_items: AlignItems::Center,
                        ..default()
                    },
                ))
                .with_children(|list| {
                    list.spawn((
                        ListPlaceholder,
                        Text::new("loading scenarios..."),
                        TextFont::from_font_size(16.0),
                        TextColor(MUTED),
                    ));
                });
            parent.spawn((
                Text::new("click a scenario to play    -    ESC to quit"),
                TextFont::from_font_size(14.0),
                TextColor(MUTED),
            ));
        });
}

/// Fills the list once the library is known, and again if it changes on disk.
fn rebuild_scenario_list(
    mut commands: Commands,
    library: Option<Res<ScenarioLibrary>>,
    lists: Query<Entity, With<ScenarioList>>,
    placeholders: Query<Entity, With<ListPlaceholder>>,
    existing: Query<Entity, With<StartsScenario>>,
) {
    let Some(library) = library else { return };
    let Ok(list) = lists.single() else { return };

    // **Reconciled from what is on screen, not from a change event.** The first version
    // rebuilt only when the library was added or changed, which is true exactly once
    // per run — so coming back from gameplay spawned a fresh, empty list that nothing
    // ever filled, and the menu sat on "loading scenarios…" for the rest of the
    // session. Asking "are the buttons missing?" covers screen re-entry, a library that
    // arrives late, and a hot reload, without needing to tell them apart.
    //
    // The guard still matters: without it this would rebuild every frame and a button
    // would never hold a hover long enough to show one.
    if !library.is_changed() && !existing.is_empty() {
        return;
    }

    for stale in existing.iter().chain(placeholders.iter()) {
        commands.entity(stale).despawn();
    }

    for (index, scenario) in library.scenarios.iter().enumerate() {
        let entry = commands
            .spawn((button("Scenario"), StartsScenario(index)))
            .with_children(|entry| {
                entry.spawn(label(scenario.name.clone()));
                entry.spawn(blurb(scenario.blurb.clone()));
            })
            .id();
        commands.entity(list).add_child(entry);
    }
}

/// Starts whichever scenario was clicked.
fn start_chosen_scenario(
    clicked: Query<(&Interaction, &StartsScenario), Changed<Interaction>>,
    mut selected: ResMut<SelectedScenario>,
    mut next: ResMut<NextState<Screen>>,
) {
    for (interaction, starts) in &clicked {
        if *interaction == Interaction::Pressed {
            selected.0 = starts.0;
            next.set(Screen::Loading);
        }
    }
}

fn handle_input(keys: Res<ButtonInput<KeyCode>>, mut exit: MessageWriter<AppExit>) {
    if keys.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
    }
}

#[cfg(test)]
mod tests {
    use bevy::state::app::StatesPlugin;
    use bevy::MinimalPlugins;
    use hex_assets::{CubeCoord, Scenario, ScenarioSettings};

    use super::*;

    fn at(x: i32, y: i32, z: i32) -> CubeCoord {
        CubeCoord { x, y, z }
    }

    fn library() -> ScenarioLibrary {
        ScenarioLibrary {
            scenarios: vec![Scenario {
                name: "First".to_owned(),
                blurb: "A map.".to_owned(),
                world: "config/world.ron".to_owned(),
                units: ScenarioSettings {
                    player: at(0, 0, 0),
                    enemy: at(1, -1, 0),
                },
            }],
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        // `InputPlugin` because `handle_input` reads `ButtonInput<KeyCode>`, which
        // `MinimalPlugins` does not provide -- and a missing system parameter is a
        // panic rather than a skipped system.
        app.add_plugins((MinimalPlugins, StatesPlugin, bevy::input::InputPlugin));
        app.init_state::<Screen>();
        app.init_resource::<SelectedScenario>();
        app.insert_resource(library());
        app.add_plugins(super::plugin);
        app
    }

    fn go_to(app: &mut App, screen: Screen) {
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(screen);
        app.update();
        app.update();
    }

    fn buttons(app: &mut App) -> usize {
        let mut query = app
            .world_mut()
            .query_filtered::<Entity, With<StartsScenario>>();
        query.iter(app.world()).count()
    }

    /// The menu still has its scenarios when you come back to it.
    ///
    /// Reported from play: quitting to the title left it stuck on "loading scenarios…"
    /// for the rest of the session. The list was rebuilt only when the library was
    /// *added or changed*, which happens exactly once per run — so the second visit
    /// spawned a fresh, empty list and nothing ever filled it.
    ///
    /// The fix is to reconcile from what is on screen rather than from a change event,
    /// which is the same lesson the turn ring taught: state, not edges.
    #[test]
    fn returning_to_the_title_screen_rebuilds_the_list() {
        let mut app = test_app();

        go_to(&mut app, Screen::Title);
        assert_eq!(
            buttons(&mut app),
            1,
            "the first visit should list a scenario"
        );

        go_to(&mut app, Screen::Gameplay);
        assert_eq!(
            buttons(&mut app),
            0,
            "leaving the title screen should take its buttons with it"
        );

        go_to(&mut app, Screen::Title);
        assert_eq!(
            buttons(&mut app),
            1,
            "coming back left the menu empty — the list was never repopulated"
        );
    }

    /// A library that arrives after the screen does still gets listed.
    ///
    /// `Screen::Title` is reached on a wall-clock timer rather than a load gate, so on
    /// a cold disk the menu really can exist before `scenarios.ron` has parsed.
    #[test]
    fn a_library_that_arrives_late_still_gets_listed() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin, bevy::input::InputPlugin));
        app.init_state::<Screen>();
        app.init_resource::<SelectedScenario>();
        app.add_plugins(super::plugin);

        go_to(&mut app, Screen::Title);
        assert_eq!(buttons(&mut app), 0, "nothing to list yet");

        app.insert_resource(library());
        app.update();
        app.update();

        assert_eq!(buttons(&mut app), 1, "the library arrived and was ignored");
    }
}
