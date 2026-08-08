use bevy::input_focus::InputFocus;
use bevy::prelude::*;
use bevy::ui::InteractionDisabled;
use hex_core::Screen;

use crate::{
    blurb, brand_logo, button, despawn_screen, display, heading, label, panel, row_button,
    screen_root, screen_title, DespawnOnExit, GameplayAction, PauseView, ResolvedUiMetrics,
    SettingsIntent, SettingsModalView, SettingsTab, UiAssets, UiIntent, UiSettingsView, UiSystems,
    UiViewportClass,
};

#[derive(Component)]
struct SettingsControl(SettingsIntent);

#[derive(Component)]
struct SettingNotice;

#[derive(Component)]
struct SettingsBack;

#[derive(Component)]
struct SettingsSurface;

#[derive(Component, Debug, Clone, PartialEq, Eq)]
struct RenderedSettingsContent {
    tab: SettingsTab,
    rows: Vec<crate::UiSettingRow>,
    bindings: Vec<crate::UiBindingRow>,
    can_restore_all: bool,
}

impl From<&UiSettingsView> for RenderedSettingsContent {
    fn from(view: &UiSettingsView) -> Self {
        Self {
            tab: view.tab,
            rows: view.rows.clone(),
            bindings: view.bindings.clone(),
            can_restore_all: view.can_restore_all,
        }
    }
}

#[derive(Component)]
struct SettingsRoot;

#[derive(Component)]
struct SettingsBindingRow;

#[derive(Component)]
struct SettingsModalRoot;

#[derive(Component)]
struct PauseOverlay;

#[derive(Component)]
struct PauseHint;

#[derive(Component)]
struct PauseNotice;

#[derive(Component)]
struct ResumeControl;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Screen::Splash), spawn_splash)
        .add_systems(OnExit(Screen::Splash), despawn_screen(Screen::Splash))
        .add_systems(OnEnter(Screen::Loading), spawn_loading)
        .add_systems(OnExit(Screen::Loading), despawn_screen(Screen::Loading))
        .add_systems(OnEnter(Screen::Settings), spawn_settings)
        .add_systems(
            Update,
            (
                (refresh_settings, apply_settings_layout)
                    .chain()
                    .in_set(UiSystems::Render),
                handle_settings_controls.in_set(UiSystems::EmitIntents),
            )
                .run_if(in_state(Screen::Settings)),
        )
        .add_systems(OnExit(Screen::Settings), despawn_screen(Screen::Settings));
    app.add_systems(OnEnter(hex_core::Pause(true)), spawn_pause)
        .add_systems(
            Update,
            (
                refresh_pause.in_set(UiSystems::Render),
                handle_pause_controls.in_set(UiSystems::EmitIntents),
            )
                .run_if(in_state(hex_core::Pause(true))),
        )
        .add_systems(OnExit(hex_core::Pause(true)), despawn_pause);
}

fn spawn_pause(mut commands: Commands, assets: Res<UiAssets>, view: Res<PauseView>) {
    commands
        .spawn((crate::overlay_root("Pause Menu"), PauseOverlay))
        .with_children(|root| {
            root.spawn(screen_title(&assets, "Paused"));
            root.spawn((PauseHint, blurb(&assets, view.hint.clone())));
            root.spawn((
                PauseNotice,
                blurb(&assets, view.notice.clone().unwrap_or_default()),
            ));
            root.spawn((button("Resume"), ResumeControl))
                .with_children(|resume| {
                    resume.spawn(label(&assets, "Resume"));
                });
        });
}

fn handle_pause_controls(
    controls: Query<&Interaction, (Changed<Interaction>, With<ResumeControl>)>,
    mut intents: MessageWriter<UiIntent>,
) {
    for interaction in &controls {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::Gameplay(GameplayAction::Pause));
        }
    }
}

fn refresh_pause(
    view: Res<PauseView>,
    mut hints: Query<&mut Text, (With<PauseHint>, Without<PauseNotice>)>,
    mut notices: Query<&mut Text, (With<PauseNotice>, Without<PauseHint>)>,
) {
    if !view.is_changed() {
        return;
    }
    for mut hint in &mut hints {
        hint.0.clone_from(&view.hint);
    }
    for mut notice in &mut notices {
        notice.0 = view.notice.clone().unwrap_or_default();
    }
}

fn despawn_pause(mut commands: Commands, overlays: Query<Entity, With<PauseOverlay>>) {
    for entity in &overlays {
        commands.entity(entity).despawn();
    }
}

fn spawn_splash(mut commands: Commands, assets: Res<UiAssets>) {
    commands
        .spawn(screen_root(Screen::Splash, "Splash Screen"))
        .with_children(|root| {
            root.spawn(brand_logo(&assets, 620.0));
            root.spawn(blurb(&assets, "A deterministic elemental tactics game"));
        });
}

fn spawn_loading(mut commands: Commands, assets: Res<UiAssets>) {
    commands
        .spawn(screen_root(Screen::Loading, "Loading Screen"))
        .with_children(|root| {
            root.spawn(display(&assets, "Preparing the battlefield"));
            root.spawn(blurb(
                &assets,
                "Loading and validating content before play begins…",
            ));
        });
}

fn spawn_settings(mut commands: Commands, assets: Res<UiAssets>, view: Res<UiSettingsView>) {
    commands
        .spawn((
            screen_root(Screen::Settings, "Settings Screen"),
            SettingsRoot,
            bevy::ui_widgets::ScrollArea,
            ScrollPosition::default(),
        ))
        .with_children(|root| {
            root.spawn(screen_title(&assets, "Settings"));
            root.spawn(blurb(
                &assets,
                "Display, audio, interface, gameplay, and camera controls.",
            ));
            root.spawn((
                button("Back"),
                SettingsBack,
                SettingsControl(SettingsIntent::Back),
                crate::UiVisibilityRequirement::Immediate,
            ))
            .with_child(label(&assets, "Back"));
            root.spawn((
                panel(),
                SettingsSurface,
                RenderedSettingsContent::from(view.as_ref()),
                bevy::ui_widgets::ScrollArea,
                ScrollPosition::default(),
            ))
            .insert(Node {
                width: Val::Px(860.0),
                max_width: Val::Percent(94.0),
                max_height: Val::Percent(78.0),
                padding: UiRect::all(Val::Px(18.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(10.0),
                overflow: Overflow::scroll_y(),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(10.0)),
                ..default()
            })
            .with_children(|surface| {
                spawn_settings_content(surface, &assets, &view);
            });
            root.spawn((
                SettingNotice,
                blurb(&assets, view.notice.clone().unwrap_or_default()),
            ));
        });
    spawn_settings_modal(&mut commands, &assets, view.modal.as_ref());
}

fn apply_settings_layout(
    mut commands: Commands,
    metrics: Res<ResolvedUiMetrics>,
    added: Query<(), Added<SettingsSurface>>,
    added_rows: Query<(), Added<SettingsBindingRow>>,
    mut roots: Query<&mut Node, (With<SettingsRoot>, Without<SettingsSurface>)>,
    mut surfaces: Query<(Entity, &mut Node), (With<SettingsSurface>, Without<SettingsRoot>)>,
    mut binding_rows: Query<
        &mut Node,
        (
            With<SettingsBindingRow>,
            Without<SettingsRoot>,
            Without<SettingsSurface>,
        ),
    >,
    mut backs: Query<
        &mut Node,
        (
            With<SettingsBack>,
            Without<SettingsRoot>,
            Without<SettingsSurface>,
            Without<SettingsBindingRow>,
        ),
    >,
) {
    if !metrics.is_changed() && added.is_empty() && added_rows.is_empty() {
        return;
    }
    let compact = metrics.viewport == UiViewportClass::Compact;
    for mut node in &mut roots {
        // A vertically overflowing flex column cannot scroll to content placed
        // above its origin by Center alignment. Compact owns the scroll route,
        // so anchor its first setting at the start edge.
        node.justify_content = if compact {
            JustifyContent::FlexStart
        } else {
            JustifyContent::Center
        };
        node.overflow = if compact {
            Overflow::scroll_y()
        } else {
            Overflow::clip_y()
        };
    }
    for (entity, mut node) in &mut surfaces {
        node.flex_shrink = if compact { 0.0 } else { 1.0 };
        node.max_height = if compact {
            Val::Auto
        } else {
            Val::Percent(78.0)
        };
        node.overflow = if compact {
            Overflow::visible()
        } else {
            Overflow::scroll_y()
        };
        if compact {
            // Compact owns one continuous page scroll. Bevy's ScrollArea
            // consumes wheel input before it checks whether this surface can
            // scroll, so an idle nested owner would strand the lower rows.
            commands
                .entity(entity)
                .remove::<bevy::ui_widgets::ScrollArea>()
                .insert(ScrollPosition::default());
        } else {
            commands
                .entity(entity)
                .insert((bevy::ui_widgets::ScrollArea, ScrollPosition::default()));
        }
    }
    for mut node in &mut binding_rows {
        node.flex_direction = if compact {
            FlexDirection::Column
        } else {
            FlexDirection::Row
        };
        node.align_items = if compact {
            AlignItems::Stretch
        } else {
            AlignItems::Center
        };
    }
    for mut node in &mut backs {
        node.position_type = if compact {
            PositionType::Relative
        } else {
            PositionType::Absolute
        };
        node.top = if compact { Val::Auto } else { Val::Px(12.0) };
        node.right = if compact { Val::Auto } else { Val::Px(12.0) };
        node.width = Val::Px(240.0);
        node.max_width = if compact {
            Val::Percent(100.0)
        } else {
            Val::Percent(40.0)
        };
        node.min_width = Val::Px(0.0);
        node.align_self = if compact {
            AlignSelf::FlexEnd
        } else {
            AlignSelf::Auto
        };
    }
}

fn refresh_settings(
    view: Res<UiSettingsView>,
    assets: Res<UiAssets>,
    mut commands: Commands,
    mut surfaces: Query<(Entity, &mut RenderedSettingsContent), With<SettingsSurface>>,
    modal_roots: Query<Entity, With<SettingsModalRoot>>,
    mut focus: ResMut<InputFocus>,
    parents: Query<&ChildOf>,
    names: Query<&Name>,
    mut focus_refreshes: ResMut<crate::focus::FocusRefreshRequests>,
    mut notices: Query<&mut Text, With<SettingNotice>>,
    mut returning_control: Local<Option<String>>,
) {
    if !view.is_changed() {
        return;
    }
    if let Some(modal) = view.modal.as_ref() {
        *returning_control = Some(match modal {
            SettingsModalView::Capture { action, .. } => {
                format!("Rebind {}", action.metadata().label)
            }
            SettingsModalView::Conflict { requested, .. } => {
                format!("Rebind {requested}")
            }
            SettingsModalView::ConfirmRestoreAll => "Restore All Keybindings".to_owned(),
        });
    }
    let returning_name = if view.modal.is_none() {
        returning_control.take()
    } else {
        None
    };
    let next_content = RenderedSettingsContent::from(view.as_ref());
    for (surface, mut rendered) in &mut surfaces {
        if *rendered == next_content {
            continue;
        }
        if let Some(name) = returning_name.clone() {
            crate::focus::request_route_focus(
                surface,
                Some(name),
                &mut focus,
                &mut focus_refreshes,
            );
        } else {
            crate::focus::begin_route_refresh(
                surface,
                &mut focus,
                &parents,
                &names,
                &mut focus_refreshes,
            );
        }
        commands.entity(surface).despawn_related::<Children>();
        commands
            .entity(surface)
            .with_children(|surface| spawn_settings_content(surface, &assets, &view));
        *rendered = next_content.clone();
    }
    for modal in &modal_roots {
        commands.entity(modal).despawn();
    }
    spawn_settings_modal(&mut commands, &assets, view.modal.as_ref());
    for mut notice in &mut notices {
        notice.0 = view.notice.clone().unwrap_or_default();
    }
}

fn spawn_settings_content(
    surface: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    view: &UiSettingsView,
) {
    surface
        .spawn(Node {
            width: Val::Percent(100.0),
            max_width: Val::Px(820.0),
            display: Display::Flex,
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            column_gap: Val::Px(8.0),
            row_gap: Val::Px(8.0),
            ..default()
        })
        .with_children(|tabs| {
            for tab in SettingsTab::ALL {
                let selected = tab == view.tab;
                tabs.spawn((
                    row_button(
                        format!("Settings Tab {}", tab.label()),
                        if tab == SettingsTab::MainView {
                            132.0
                        } else {
                            112.0
                        },
                    ),
                    SettingsControl(SettingsIntent::SelectTab(tab)),
                    crate::UiVisibilityRequirement::Immediate,
                ))
                .with_children(|button| {
                    button.spawn(label(
                        assets,
                        if selected {
                            format!("{} · Selected", tab.label())
                        } else {
                            tab.label().to_owned()
                        },
                    ));
                });
            }
        });
    surface.spawn(heading(assets, view.tab.label()));
    if !view.rows.is_empty() {
        spawn_general_settings(surface, assets, view);
    }
    if view.tab.input_category().is_some() {
        spawn_binding_settings(surface, assets, view);
    }
}

fn spawn_general_settings(
    surface: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    view: &UiSettingsView,
) {
    surface.spawn(blurb(
        assets,
        "Display and audio changes preview and save immediately.",
    ));
    for row in &view.rows {
        surface
            .spawn((
                button(format!("Setting {:?}", row.setting)),
                SettingsControl(SettingsIntent::Adjust(row.setting)),
                crate::UiVisibilityRequirement::Scrollable,
            ))
            .with_child(label(assets, format!("{} · {}", row.label, row.value)));
    }
}

fn spawn_binding_settings(
    surface: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    view: &UiSettingsView,
) {
    surface.spawn(blurb(
        assets,
        "Select a binding, then press one key with optional modifiers. Escape cancels capture.",
    ));
    for row in &view.bindings {
        spawn_binding_row(surface, assets, row);
    }
    let mut restore = surface.spawn((
        button("Restore All Keybindings"),
        SettingsControl(SettingsIntent::RequestRestoreAll),
        crate::UiVisibilityRequirement::Scrollable,
    ));
    if !view.can_restore_all {
        restore.insert(InteractionDisabled);
    }
    restore.with_child(label(
        assets,
        if view.can_restore_all {
            "Restore All…"
        } else {
            "All bindings use defaults"
        },
    ));
}

fn spawn_binding_row(
    surface: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    row: &crate::UiBindingRow,
) {
    surface
        .spawn((
            Name::new(format!("Binding Row {}", row.label)),
            SettingsBindingRow,
            Node {
                width: Val::Percent(100.0),
                max_width: Val::Px(820.0),
                min_height: Val::Px(56.0),
                padding: UiRect::all(Val::Px(8.0)),
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                row_gap: Val::Px(8.0),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(crate::PANEL_BG),
            BorderColor::all(crate::EDGE),
        ))
        .with_children(|binding| {
            binding
                .spawn(Node {
                    flex_grow: 1.0,
                    min_width: Val::Px(180.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(3.0),
                    ..default()
                })
                .with_children(|copy| {
                    copy.spawn(label(assets, row.label.clone()));
                    copy.spawn(blurb(
                        assets,
                        if row.rebindable {
                            format!("Current · {}", row.chord)
                        } else {
                            format!("Fixed navigation · {}", row.chord)
                        },
                    ));
                });
            if !row.rebindable {
                return;
            }
            binding
                .spawn((
                    row_button(format!("Rebind {}", row.label), 150.0),
                    SettingsControl(SettingsIntent::BeginCapture(row.action)),
                    crate::UiVisibilityRequirement::Scrollable,
                ))
                .with_child(label(assets, format!("Rebind · {}", row.chord)));
            let mut restore = binding.spawn((
                row_button(format!("Restore {}", row.label), 116.0),
                SettingsControl(SettingsIntent::RestoreBinding(row.action)),
                crate::UiVisibilityRequirement::Scrollable,
            ));
            if !row.overridden {
                restore.insert(InteractionDisabled);
            }
            restore.with_child(label(
                assets,
                if row.overridden { "Restore" } else { "Default" },
            ));
        });
}

fn spawn_settings_modal(
    commands: &mut Commands,
    assets: &UiAssets,
    modal: Option<&SettingsModalView>,
) {
    let Some(modal) = modal else { return };
    commands
        .spawn((
            crate::overlay_root("Settings Modal"),
            SettingsModalRoot,
            DespawnOnExit(Screen::Settings),
        ))
        .with_children(|overlay| {
            overlay
                .spawn(panel())
                .insert(Node {
                    width: Val::Px(520.0),
                    max_width: Val::Percent(92.0),
                    padding: UiRect::all(Val::Px(20.0)),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(12.0),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(8.0)),
                    ..default()
                })
                .with_children(|card| match modal {
                    SettingsModalView::Capture { label: action, .. } => {
                        card.spawn(heading(assets, "Press a key"));
                        card.spawn(blurb(
                            assets,
                            format!("New binding for {action}. Modifiers may be held."),
                        ));
                        card.spawn(blurb(
                            assets,
                            "Pure modifier keys are ignored. Escape cancels.",
                        ));
                        card.spawn((
                            button("Cancel Key Capture"),
                            SettingsControl(SettingsIntent::CancelCapture),
                            crate::UiVisibilityRequirement::Immediate,
                        ))
                        .with_child(label(assets, "Cancel"));
                    }
                    SettingsModalView::Conflict {
                        requested,
                        existing,
                        chord,
                    } => {
                        card.spawn(heading(assets, "Binding conflict"));
                        card.spawn(blurb(
                            assets,
                            format!("{chord} is assigned to {existing}. Swap it with {requested}?"),
                        ));
                        card.spawn((
                            button("Swap Conflicting Bindings"),
                            SettingsControl(SettingsIntent::SwapConflict),
                            crate::UiVisibilityRequirement::Immediate,
                        ))
                        .with_child(label(assets, "Swap"));
                        card.spawn((
                            button("Cancel Binding Conflict"),
                            SettingsControl(SettingsIntent::CancelConflict),
                            crate::UiVisibilityRequirement::Immediate,
                        ))
                        .with_child(label(assets, "Cancel"));
                    }
                    SettingsModalView::ConfirmRestoreAll => {
                        card.spawn(heading(assets, "Restore all bindings?"));
                        card.spawn(blurb(
                            assets,
                            "Every binding shown in Settings will return to its canonical default.",
                        ));
                        card.spawn((
                            button("Confirm Restore All Keybindings"),
                            SettingsControl(SettingsIntent::ConfirmRestoreAll),
                            crate::UiVisibilityRequirement::Immediate,
                        ))
                        .with_child(label(assets, "Restore All"));
                        card.spawn((
                            button("Cancel Restore All Keybindings"),
                            SettingsControl(SettingsIntent::CancelRestoreAll),
                            crate::UiVisibilityRequirement::Immediate,
                        ))
                        .with_child(label(assets, "Cancel"));
                    }
                });
        });
}

fn handle_settings_controls(
    controls: Query<(&Interaction, &SettingsControl), Changed<Interaction>>,
    mut intents: MessageWriter<UiIntent>,
) {
    for (interaction, control) in &controls {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::Settings(control.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_entities_are_owned_by_their_screen() {
        let mut world = World::new();
        let entity = world
            .spawn(screen_root(Screen::Loading, "Loading Screen"))
            .id();
        assert_eq!(
            world
                .entity(entity)
                .get::<crate::DespawnOnExit>()
                .map(|tag| tag.0),
            Some(Screen::Loading)
        );
    }

    #[cfg(feature = "test-support")]
    mod focus_regressions {
        use bevy::input_focus::{tab_navigation::TabIndex, FocusCause, InputFocus};

        use super::*;
        use crate::test_support::HeadlessUiPlugin;

        fn binding_row(
            action: hex_core::InputAction,
            chord: &str,
            overridden: bool,
        ) -> crate::UiBindingRow {
            crate::UiBindingRow {
                action,
                label: action.metadata().label.to_owned(),
                chord: chord.to_owned(),
                rebindable: true,
                overridden,
            }
        }

        fn settings_view(
            bindings: Vec<crate::UiBindingRow>,
            can_restore_all: bool,
            modal: Option<SettingsModalView>,
        ) -> UiSettingsView {
            UiSettingsView {
                tab: SettingsTab::Interface,
                rows: Vec::new(),
                bindings,
                can_restore_all,
                modal,
                notice: None,
            }
        }

        fn settle(app: &mut App) {
            for _ in 0..4 {
                app.update();
            }
        }

        fn settings_app(view: UiSettingsView) -> App {
            let mut app = App::new();
            app.add_plugins(HeadlessUiPlugin::default());
            *app.world_mut().resource_mut::<UiSettingsView>() = view;
            app.world_mut()
                .resource_mut::<NextState<Screen>>()
                .set(Screen::Settings);
            settle(&mut app);
            app
        }

        fn named_entity(world: &mut World, wanted: &str) -> Entity {
            let mut names = world.query::<(Entity, &Name)>();
            names
                .iter(world)
                .find_map(|(entity, name)| (name.as_str() == wanted).then_some(entity))
                .unwrap_or_else(|| panic!("missing named UI entity {wanted}"))
        }

        fn focus_named(app: &mut App, name: &str) -> Entity {
            let entity = named_entity(app.world_mut(), name);
            app.world_mut()
                .resource_mut::<InputFocus>()
                .set(entity, FocusCause::Navigated);
            entity
        }

        fn replace_view(app: &mut App, view: UiSettingsView) {
            *app.world_mut().resource_mut::<UiSettingsView>() = view;
            settle(app);
        }

        fn assert_live_reachable_focus(app: &mut App) -> (Entity, String) {
            let focused = app
                .world()
                .resource::<InputFocus>()
                .get()
                .expect("Settings should retain a focused control");
            assert!(app.world().get_entity(focused).is_ok());
            assert!(app
                .world()
                .get::<TabIndex>(focused)
                .is_some_and(|index| index.0 >= 0));

            let mut current = Some(focused);
            while let Some(entity) = current {
                assert!(app.world().get::<InteractionDisabled>(entity).is_none());
                assert!(!app
                    .world()
                    .get::<Visibility>(entity)
                    .is_some_and(|visibility| *visibility == Visibility::Hidden));
                assert!(!app
                    .world()
                    .get::<Node>(entity)
                    .is_some_and(|node| node.display == Display::None));
                current = app.world().get::<ChildOf>(entity).map(ChildOf::parent);
            }

            let name = app
                .world()
                .get::<Name>(focused)
                .expect("focused Settings controls have stable names")
                .as_str()
                .to_owned();
            (focused, name)
        }

        #[test]
        fn restore_conflict_swap_returns_focus_to_the_live_binding_row() {
            let swapped = vec![
                binding_row(hex_core::InputAction::ToggleParty, "I", true),
                binding_row(hex_core::InputAction::ToggleInitiative, "P", true),
            ];
            let mut app = settings_app(settings_view(swapped.clone(), true, None));
            let stale_restore = focus_named(&mut app, "Restore Party");

            replace_view(
                &mut app,
                settings_view(
                    swapped,
                    true,
                    Some(SettingsModalView::Conflict {
                        requested: "Party".to_owned(),
                        existing: "Initiative".to_owned(),
                        chord: "P".to_owned(),
                    }),
                ),
            );
            assert_eq!(
                assert_live_reachable_focus(&mut app).1,
                "Swap Conflicting Bindings"
            );

            replace_view(
                &mut app,
                settings_view(
                    vec![
                        binding_row(hex_core::InputAction::ToggleParty, "P", false),
                        binding_row(hex_core::InputAction::ToggleInitiative, "I", false),
                    ],
                    false,
                    None,
                ),
            );

            assert!(app.world().get_entity(stale_restore).is_err());
            assert_eq!(assert_live_reachable_focus(&mut app).1, "Rebind Party");
        }

        #[test]
        fn confirmed_restore_all_falls_back_to_a_live_settings_control() {
            let overridden = vec![binding_row(hex_core::InputAction::ToggleParty, "Y", true)];
            let mut app = settings_app(settings_view(overridden.clone(), true, None));
            let stale_restore_all = focus_named(&mut app, "Restore All Keybindings");

            replace_view(
                &mut app,
                settings_view(overridden, true, Some(SettingsModalView::ConfirmRestoreAll)),
            );
            assert_eq!(
                assert_live_reachable_focus(&mut app).1,
                "Confirm Restore All Keybindings"
            );

            replace_view(
                &mut app,
                settings_view(
                    vec![binding_row(hex_core::InputAction::ToggleParty, "P", false)],
                    false,
                    None,
                ),
            );

            assert!(app.world().get_entity(stale_restore_all).is_err());
            assert_eq!(
                assert_live_reachable_focus(&mut app).1,
                "Settings Tab General"
            );
        }
    }
}
