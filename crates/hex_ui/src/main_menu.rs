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
        MainMenuRoute::Tools => render_tools(root, assets),
    }
}

fn render_root(root: &mut ChildSpawnerCommands, assets: &UiAssets, view: &MainMenuView) {
    root.spawn(brand_logo(assets, 420.0));
    if let Some(reason) = &view.setup_failure {
        root.spawn(blurb(assets, reason.clone()));
    }
    root.spawn((Name::new("Main Menu Actions"), panel()))
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
            for (name, intent) in [
                ("Campaign", MainMenuIntent::OpenCampaign),
                ("Sandbox", MainMenuIntent::OpenSandbox),
                ("Tools", MainMenuIntent::OpenTools),
                ("Settings", MainMenuIntent::OpenSettings),
            ] {
                menu_button(
                    actions,
                    assets,
                    name,
                    intent,
                    UiVisibilityRequirement::Immediate,
                );
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
    mut decks: Query<&mut Node, With<CampaignDeck>>,
) {
    if !metrics.is_changed() && added.is_empty() {
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
}

fn emit_intents(
    controls: Query<(&Interaction, &MainMenuControl), Changed<Interaction>>,
    mut intents: MessageWriter<UiIntent>,
) {
    for (interaction, control) in &controls {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::MainMenu(control.0));
        }
    }
}
