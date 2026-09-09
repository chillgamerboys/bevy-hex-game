//! Player-facing Main Menu, Campaign slots, and Tools hierarchy.

use bevy::input_focus::InputFocus;
use bevy::prelude::*;
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::ScrollArea;
use hex_core::Screen;
use hex_gameplay_model::{CampaignSlotId, MainMenuRoute};

use crate::{
    blurb, brand_logo, button, despawn_screen, fine, fluid_button, heading, label, panel,
    screen_root, screen_title, CampaignSlotStatusView, MainMenuIntent, MainMenuView,
    ResolvedUiMetrics, UiAssets, UiIntent, UiSystems, UiViewportClass, UiVisibilityRequirement,
};

#[derive(Component)]
struct MainMenuSurface;

#[derive(Component)]
struct MainMenuControl(MainMenuIntent);

#[derive(Component)]
struct CampaignDeck;

#[derive(Component)]
struct BattleMenuActions;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Screen::Title), spawn)
        .add_systems(
            Update,
            (refresh, apply_layout)
                .chain()
                .in_set(UiSystems::Render)
                .run_if(in_state(Screen::Title)),
        )
        .add_systems(
            Update,
            emit_intents
                .in_set(UiSystems::EmitIntents)
                .run_if(in_state(Screen::Title)),
        )
        .add_systems(OnExit(Screen::Title), despawn_screen(Screen::Title));
}

fn spawn(mut commands: Commands, assets: Res<UiAssets>, view: Res<MainMenuView>) {
    commands
        .spawn((screen_root(Screen::Title, "Main Menu"), MainMenuSurface))
        .insert(Node {
            padding: UiRect::all(Val::Px(28.0)),
            justify_content: JustifyContent::FlexStart,
            ..crate::screen_root_node()
        })
        .with_children(|root| render(root, &assets, &view));
}

fn refresh(
    view: Res<MainMenuView>,
    assets: Res<UiAssets>,
    roots: Query<Entity, With<MainMenuSurface>>,
    parents: Query<&ChildOf>,
    names: Query<&Name>,
    mut focus: ResMut<InputFocus>,
    mut focus_refreshes: ResMut<crate::focus::FocusRefreshRequests>,
    mut commands: Commands,
) {
    if !view.is_changed() {
        return;
    }
    for root in &roots {
        crate::focus::begin_route_refresh(root, &mut focus, &parents, &names, &mut focus_refreshes);
        commands.entity(root).despawn_related::<Children>();
        commands
            .entity(root)
            .with_children(|root| render(root, &assets, &view));
    }
}

fn render(root: &mut ChildSpawnerCommands, assets: &UiAssets, view: &MainMenuView) {
    match view.route {
        MainMenuRoute::Root => render_root(root, assets, view),
        MainMenuRoute::Campaign => render_campaign(root, assets, view),
        MainMenuRoute::Multiplayer => render_root(root, assets, view),
        MainMenuRoute::Tools => render_tools(root, assets),
    }
}

fn battle_notice(
    root: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    name: &'static str,
    copy: &str,
) {
    // Resolve the line width on a parent before measuring text. Neither level
    // may shrink its height below the final wrapped glyph layout.
    root.spawn(Node {
        width: Val::Percent(94.0),
        max_width: Val::Px(900.0),
        flex_direction: FlexDirection::Column,
        flex_shrink: 0.0,
        ..default()
    })
    .with_child((
        Name::new(name),
        blurb(assets, copy),
        Node {
            width: Val::Percent(100.0),
            flex_shrink: 0.0,
            ..default()
        },
        TextLayout::justify(Justify::Center),
        UiVisibilityRequirement::Immediate,
        crate::UiTextMustFit,
    ));
}

fn render_root(root: &mut ChildSpawnerCommands, assets: &UiAssets, view: &MainMenuView) {
    root.spawn(brand_logo(assets, 420.0));
    if let Some(reason) = &view.setup_failure {
        root.spawn(blurb(assets, reason.clone()));
    }
    if view.battle_running {
        battle_notice(
            root,
            assets,
            "Battle Mode Running",
            "Battle Mode is open. Close its window to return.",
        );
    }
    if let Some(reason) = &view.battle_launch_error {
        battle_notice(root, assets, "Battle Mode Launch Error", reason);
    }
    let mut actions = root.spawn((Name::new("Main Menu Actions"), panel()));
    if view.battle_mode_available {
        actions.insert(BattleMenuActions);
    }
    actions
        .insert(Node {
            width: Val::Px(480.0),
            max_width: Val::Percent(94.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Stretch,
            row_gap: Val::Px(12.0),
            padding: UiRect::all(Val::Px(22.0)),
            ..crate::panel_node()
        })
        .with_children(|actions| {
            let battle = view
                .battle_mode_available
                .then_some(("Battle Mode", MainMenuIntent::OpenBattleMode));
            for (name, intent) in [("Campaign", MainMenuIntent::OpenCampaign)]
                .into_iter()
                .chain(battle)
                .chain([
                    ("Sandbox", MainMenuIntent::OpenSandbox),
                    ("Multiplayer", MainMenuIntent::OpenMultiplayer),
                    ("Tools", MainMenuIntent::OpenTools),
                    ("Settings", MainMenuIntent::OpenSettings),
                ])
            {
                let mut control = if view.battle_mode_available {
                    actions.spawn(fluid_button(name))
                } else {
                    actions.spawn(button(name))
                };
                control.insert((MainMenuControl(intent), UiVisibilityRequirement::Immediate));
                if view.battle_running {
                    control.insert(InteractionDisabled);
                }
                control.with_children(|control| {
                    let mut text = control.spawn(label(assets, name));
                    if view.battle_running {
                        text.insert(TextColor(crate::theme::MUTED));
                    }
                });
            }
        });
    root.spawn(fine(assets, concat!("v", env!("CARGO_PKG_VERSION"))));
}

fn render_campaign(root: &mut ChildSpawnerCommands, assets: &UiAssets, view: &MainMenuView) {
    root.spawn((screen_title(assets, "Hex / Campaign"), crate::UiTextMustFit));
    root.spawn(blurb(
        assets,
        "Continue an occupied campaign or begin in an empty slot.",
    ));
    if let Some(reason) = &view.setup_failure {
        root.spawn((
            Name::new("Campaign Setup Failure"),
            blurb(assets, reason.clone()),
        ));
    }
    root.spawn((
        Name::new("Campaign Slot Viewport"),
        ScrollArea,
        ScrollPosition::default(),
        Node {
            width: Val::Percent(100.0),
            min_height: Val::Px(0.0),
            flex_grow: 1.0,
            overflow: Overflow::scroll_y(),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::FlexStart,
            align_items: AlignItems::Center,
            ..default()
        },
    ))
    .with_children(|viewport| {
        viewport
            .spawn((
                Name::new("Campaign Slots"),
                CampaignDeck,
                Node {
                    width: Val::Percent(96.0),
                    max_width: Val::Px(1_320.0),
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Stretch,
                    justify_content: JustifyContent::Center,
                    column_gap: Val::Px(18.0),
                    row_gap: Val::Px(18.0),
                    min_width: Val::Px(0.0),
                    flex_shrink: 0.0,
                    ..default()
                },
            ))
            .with_children(|cards| {
                for slot in &view.campaign_slots {
                    cards
                        .spawn((
                            Name::new(format!("Campaign Slot {}", slot.slot.number())),
                            panel(),
                        ))
                        .insert(Node {
                            width: Val::Px(390.0),
                            min_width: Val::Px(280.0),
                            min_height: Val::Px(310.0),
                            flex_grow: 1.0,
                            flex_shrink: 0.0,
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::Stretch,
                            row_gap: Val::Px(10.0),
                            ..crate::panel_node()
                        })
                        .with_children(|card| {
                            card.spawn(heading(
                                assets,
                                format!("Save Slot {}", slot.slot.number()),
                            ));
                            match &slot.status {
                                CampaignSlotStatusView::Empty => {
                                    card.spawn(blurb(assets, "Empty campaign slot"));
                                    campaign_menu_button(
                                        card,
                                        assets,
                                        "New Game",
                                        slot.slot,
                                        MainMenuIntent::NewCampaign(slot.slot),
                                        UiVisibilityRequirement::Scrollable,
                                    );
                                }
                                CampaignSlotStatusView::Available { party, active_time } => {
                                    card.spawn(label(
                                        assets,
                                        format!("Active gameplay · {active_time}"),
                                    ));
                                    for member in party {
                                        card.spawn(panel())
                                            .insert(Node {
                                                width: Val::Percent(100.0),
                                                padding: UiRect::all(Val::Px(10.0)),
                                                min_height: Val::Px(104.0),
                                                flex_direction: FlexDirection::Row,
                                                align_items: AlignItems::Center,
                                                column_gap: Val::Px(8.0),
                                                ..crate::panel_node()
                                            })
                                            .with_children(|preview| {
                                                crate::sandbox::spawn_mini_lattice(
                                                    preview,
                                                    assets,
                                                    &member.cells,
                                                );
                                                preview
                                                    .spawn(Node {
                                                        min_width: Val::Px(0.0),
                                                        flex_grow: 1.0,
                                                        flex_direction: FlexDirection::Column,
                                                        row_gap: Val::Px(3.0),
                                                        ..default()
                                                    })
                                                    .with_children(|copy| {
                                                        copy.spawn(label(
                                                            assets,
                                                            member.name.clone(),
                                                        ));
                                                        copy.spawn(fine(
                                                            assets,
                                                            member.lattice.clone(),
                                                        ));
                                                    });
                                            });
                                    }
                                    campaign_menu_button(
                                        card,
                                        assets,
                                        "Continue",
                                        slot.slot,
                                        MainMenuIntent::ContinueCampaign(slot.slot),
                                        UiVisibilityRequirement::Scrollable,
                                    );
                                }
                                CampaignSlotStatusView::Invalid { reason } => {
                                    card.spawn(blurb(assets, "Campaign unavailable"));
                                    card.spawn(fine(assets, reason.clone()));
                                }
                            }
                        });
                }
            });
    });
    menu_button(
        root,
        assets,
        "Back",
        MainMenuIntent::Back,
        UiVisibilityRequirement::Immediate,
    );
}

fn render_tools(root: &mut ChildSpawnerCommands, assets: &UiAssets) {
    root.spawn((screen_title(assets, "Hex / Tools"), crate::UiTextMustFit));
    root.spawn((Name::new("Tools List"), panel()))
        .insert(Node {
            width: Val::Px(620.0),
            max_width: Val::Percent(94.0),
            flex_grow: 1.0,
            max_height: Val::Px(520.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Stretch,
            row_gap: Val::Px(14.0),
            ..crate::panel_node()
        })
        .with_children(|tools| {
            let mut map = tools.spawn((
                button("Map Creator — Coming Soon"),
                InteractionDisabled,
                crate::UiVisibilityRequirement::Immediate,
            ));
            map.with_children(|control| {
                control.spawn(label(assets, "Map Creator"));
                control.spawn(fine(assets, "Coming Soon"));
            });
            menu_button(
                tools,
                assets,
                "Character Creator",
                MainMenuIntent::OpenCharacterCreator,
                UiVisibilityRequirement::Immediate,
            );
            menu_button(
                tools,
                assets,
                "Spell Creator",
                MainMenuIntent::OpenSpellCreator,
                UiVisibilityRequirement::Immediate,
            );
            menu_button(
                tools,
                assets,
                "VFX Tuner",
                MainMenuIntent::OpenVfxTuner,
                UiVisibilityRequirement::Immediate,
            );
        });
    menu_button(
        root,
        assets,
        "Back",
        MainMenuIntent::Back,
        UiVisibilityRequirement::Immediate,
    );
}

fn menu_button(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    name: &'static str,
    intent: MainMenuIntent,
    visibility: UiVisibilityRequirement,
) {
    let mut control = parent.spawn((button(name), MainMenuControl(intent), visibility));
    control.with_child(label(assets, name));
}

fn campaign_menu_button(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    label_text: &'static str,
    slot: CampaignSlotId,
    intent: MainMenuIntent,
    visibility: UiVisibilityRequirement,
) {
    let control_name = format!("{label_text} Save Slot {}", slot.number());
    let accessible_label = format!("{label_text}, Save Slot {}", slot.number());
    let mut control = parent.spawn((
        fluid_button(control_name),
        MainMenuControl(intent),
        visibility,
    ));
    control
        .insert(AccessibleLabel::new(accessible_label))
        // Player-facing copy remains the short action verb rendered on the card.
        .with_child(label(assets, label_text));
}

fn apply_layout(
    metrics: Res<ResolvedUiMetrics>,
    added: Query<(), Added<CampaignDeck>>,
    added_battle: Query<(), Added<BattleMenuActions>>,
    mut decks: Query<&mut Node, (With<CampaignDeck>, Without<BattleMenuActions>)>,
    mut battle_actions: Query<&mut Node, (With<BattleMenuActions>, Without<CampaignDeck>)>,
) {
    if !metrics.is_changed() && added.is_empty() && added_battle.is_empty() {
        return;
    }
    for mut node in &mut decks {
        let compact = metrics.viewport == UiViewportClass::Compact;
        node.flex_direction = if compact {
            FlexDirection::Column
        } else {
            FlexDirection::Row
        };
        node.align_items = if compact {
            AlignItems::Center
        } else {
            AlignItems::Stretch
        };
    }
    for mut node in &mut battle_actions {
        // Six large-text actions need fewer rows on the existing compact canvas.
        // Reading and keyboard order stay Campaign, Battle Mode, then the old routes.
        let compact = metrics.viewport == UiViewportClass::Compact;
        node.display = if compact {
            Display::Grid
        } else {
            Display::Flex
        };
        node.width = Val::Px(if compact { 900.0 } else { 480.0 });
        node.grid_template_columns = RepeatedGridTrack::flex(2, 1.0);
        node.column_gap = Val::Px(12.0);
        node.row_gap = Val::Px(if compact { 8.0 } else { 12.0 });
    }
}

fn emit_intents(
    view: Res<MainMenuView>,
    controls: Query<
        (&Interaction, &MainMenuControl),
        (Changed<Interaction>, Without<InteractionDisabled>),
    >,
    mut intents: MessageWriter<UiIntent>,
) {
    if view.battle_running {
        return;
    }
    for (interaction, control) in &controls {
        if *interaction == Interaction::Pressed
            && (control.0 != MainMenuIntent::OpenBattleMode || view.battle_mode_available)
        {
            intents.write(UiIntent::MainMenu(control.0));
        }
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "menu fixtures require the controls they render"
)]
mod tests {
    use super::*;

    fn menu_app(view: MainMenuView) -> App {
        let mut app = App::new();
        app.insert_resource(view)
            .insert_resource(UiAssets {
                display: default(),
                body: default(),
                logo: default(),
                hex_cell: default(),
            })
            .add_message::<UiIntent>()
            .add_systems(Startup, spawn)
            .add_systems(Update, emit_intents);
        app.update();
        app
    }

    fn controls(app: &mut App) -> Vec<(Entity, String)> {
        let world = app.world_mut();
        let mut parents = world.query::<(&Name, &Children)>();
        let children = parents
            .iter(world)
            .find_map(|(name, children)| {
                (name.as_str() == "Main Menu Actions").then(|| children.to_vec())
            })
            .expect("root actions");
        children
            .into_iter()
            .map(|entity| {
                (
                    entity,
                    world
                        .get::<Name>(entity)
                        .expect("named menu control")
                        .as_str()
                        .to_owned(),
                )
            })
            .collect()
    }

    fn press(app: &mut App, entity: Entity) {
        *app.world_mut()
            .get_mut::<Interaction>(entity)
            .expect("interactive button") = Interaction::Pressed;
    }

    fn drain(app: &mut App) -> Vec<MainMenuIntent> {
        app.world_mut()
            .resource_mut::<Messages<UiIntent>>()
            .drain()
            .filter_map(|intent| {
                if let UiIntent::MainMenu(intent) = intent {
                    Some(intent)
                } else {
                    None
                }
            })
            .collect()
    }

    #[test]
    fn battle_mode_is_optional_and_emits_only_its_available_enabled_intent() {
        let mut original = menu_app(MainMenuView::default());
        assert_eq!(
            controls(&mut original)
                .into_iter()
                .map(|(_, name)| name)
                .collect::<Vec<_>>(),
            ["Campaign", "Sandbox", "Multiplayer", "Tools", "Settings"]
        );
        let mut app = menu_app(MainMenuView {
            battle_mode_available: true,
            ..default()
        });
        let buttons = controls(&mut app);
        assert_eq!(
            buttons
                .iter()
                .map(|(_, name)| name.as_str())
                .collect::<Vec<_>>(),
            [
                "Campaign",
                "Battle Mode",
                "Sandbox",
                "Multiplayer",
                "Tools",
                "Settings"
            ]
        );
        let battle = buttons
            .iter()
            .find_map(|(entity, name)| (name == "Battle Mode").then_some(*entity))
            .expect("Battle Mode");
        press(&mut app, battle);
        app.update();
        assert_eq!(drain(&mut app), [MainMenuIntent::OpenBattleMode]);
        app.world_mut()
            .entity_mut(battle)
            .insert(InteractionDisabled);
        press(&mut app, battle);
        app.update();
        assert!(
            drain(&mut app).is_empty(),
            "disabled pointer interactions cannot leak intents"
        );
        app.world_mut()
            .entity_mut(battle)
            .remove::<InteractionDisabled>();
        app.world_mut()
            .resource_mut::<MainMenuView>()
            .battle_mode_available = false;
        press(&mut app, battle);
        app.update();
        assert!(
            drain(&mut app).is_empty(),
            "a stale button cannot bypass changed availability"
        );
    }

    #[test]
    fn battle_running_disables_every_root_action_and_blocks_pre_refresh_interactions() {
        let mut app = menu_app(MainMenuView {
            battle_mode_available: true,
            battle_running: true,
            ..default()
        });
        let buttons = controls(&mut app);
        assert_eq!(buttons.len(), 6);
        for (entity, _) in &buttons {
            assert!(app.world().get::<InteractionDisabled>(*entity).is_some());
            press(&mut app, *entity);
        }
        app.update();
        assert!(drain(&mut app).is_empty());
        let mut texts = app.world_mut().query::<&Text>();
        assert!(texts
            .iter(app.world())
            .any(|text| text.0 == "Battle Mode is open. Close its window to return."));

        let mut app = menu_app(MainMenuView {
            battle_mode_available: true,
            ..default()
        });
        app.world_mut()
            .resource_mut::<MainMenuView>()
            .battle_running = true;
        for (entity, _) in controls(&mut app) {
            // Intent emission runs before the view rebuild; even stale enabled
            // controls must not navigate away while the child is running.
            press(&mut app, entity);
        }
        app.update();
        assert!(drain(&mut app).is_empty());
    }

    #[cfg(feature = "test-support")]
    fn rendered_app(size: UVec2, scale: crate::UiScaleMode, view: MainMenuView) -> App {
        let mut app = App::new();
        app.add_plugins(crate::test_support::HeadlessUiPlugin::with_scale_factor(
            size.x, size.y, 1.0,
        ));
        app.insert_resource(crate::UiScalePreference(scale))
            .insert_resource(view);
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Title);
        settle(&mut app);
        app
    }

    #[cfg(feature = "test-support")]
    fn settle(app: &mut App) {
        for _ in 0..8 {
            app.update();
        }
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn battle_mode_actions_and_launch_error_fit_the_supported_layout_matrix() {
        use crate::{
            test_support::{ui_tree_snapshot, UiTaskCase},
            UiScaleMode,
        };
        for size in [
            UVec2::new(1280, 720),
            UVec2::new(1920, 1080),
            UVec2::new(3840, 2160),
        ] {
            for scale in [UiScaleMode::Auto, UiScaleMode::Percent200] {
                let mut app = rendered_app(
                    size,
                    scale,
                    MainMenuView {
                        battle_mode_available: true,
                        battle_launch_error: Some(
                            "Battle Mode could not open. Please try again.".to_owned(),
                        ),
                        ..default()
                    },
                );
                let snapshot = ui_tree_snapshot(app.world_mut());
                assert!(
                    snapshot.task_issues(UiTaskCase::MainMenu).is_empty(),
                    "{size:?} {scale:?}: {:?}",
                    snapshot.task_issues(UiTaskCase::MainMenu)
                );
                assert_eq!(
                    snapshot.focus_order,
                    [
                        "Campaign",
                        "Battle Mode",
                        "Sandbox",
                        "Multiplayer",
                        "Tools",
                        "Settings"
                    ]
                );
                let battle = snapshot
                    .nodes
                    .iter()
                    .find(|node| node.name == "Battle Mode")
                    .expect("Battle Mode observation");
                assert!(
                    battle.fully_visible
                        && battle.keyboard_reachable == Some(true)
                        && battle.meets_minimum_target == Some(true)
                );
                let error = snapshot
                    .nodes
                    .iter()
                    .find(|node| node.name == "Battle Mode Launch Error")
                    .expect("launch error observation");
                assert!(error.fully_visible && error.rendered_text_bounds.is_some());
            }
        }
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn battle_running_refresh_removes_focus_targets_and_restores_navigation_on_close() {
        use crate::{
            test_support::{ui_tree_snapshot, UiTaskCase},
            UiScaleMode,
        };
        let mut app = rendered_app(
            UVec2::new(1280, 720),
            UiScaleMode::Percent200,
            MainMenuView {
                battle_mode_available: true,
                ..default()
            },
        );
        let battle = controls(&mut app)
            .into_iter()
            .find_map(|(entity, name)| (name == "Battle Mode").then_some(entity))
            .expect("Battle Mode");
        *app.world_mut().resource_mut::<InputFocus>() = InputFocus::from_entity(battle);
        app.world_mut()
            .resource_mut::<MainMenuView>()
            .battle_running = true;
        settle(&mut app);
        let snapshot = ui_tree_snapshot(app.world_mut());
        assert!(
            snapshot.task_issues(UiTaskCase::MainMenu).is_empty(),
            "{:?}",
            snapshot.task_issues(UiTaskCase::MainMenu)
        );
        assert!(snapshot.focus_order.is_empty());
        let notice = snapshot
            .nodes
            .iter()
            .find(|node| node.name == "Battle Mode Running")
            .expect("running notice");
        assert!(notice.fully_visible && notice.rendered_text_bounds.is_some());
        assert!(controls(&mut app)
            .iter()
            .all(|(entity, _)| app.world().get::<InteractionDisabled>(*entity).is_some()));
        app.world_mut()
            .resource_mut::<MainMenuView>()
            .battle_running = false;
        settle(&mut app);
        let snapshot = ui_tree_snapshot(app.world_mut());
        assert_eq!(
            snapshot.focus_order,
            [
                "Campaign",
                "Battle Mode",
                "Sandbox",
                "Multiplayer",
                "Tools",
                "Settings"
            ]
        );
        assert!(!snapshot
            .nodes
            .iter()
            .any(|node| node.name == "Battle Mode Running"));
        assert!(controls(&mut app)
            .iter()
            .all(|(entity, _)| app.world().get::<InteractionDisabled>(*entity).is_none()));
    }
}
