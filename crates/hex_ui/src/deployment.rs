//! Guided Sandbox deployment presentation.

use bevy::input_focus::tab_navigation::TabGroup;
use bevy::prelude::*;
use hex_core::Screen;
use hex_gameplay_model::{SandboxDeploymentSlot, SandboxDeploymentStage, SandboxSide};

use crate::{
    blurb, compact_glyph_role, fine, fixed_row_button, heading, label,
    layout::is_ultra_constrained, row_button, DeploymentIntent, DeploymentQueueEntryView,
    DeploymentView, DespawnOnExit, ResolvedUiMetrics, UiAssets, UiIntent, UiSystems,
    UiViewportClass, ACCENT, EDGE, LABEL, PANEL_BG,
};

#[derive(Component)]
struct DeploymentRoot;

#[derive(Component)]
struct DeploymentCurrent;

#[derive(Component)]
struct DeploymentQueue;

#[derive(Component)]
struct DeploymentActions;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        (
            (render, apply_layout).chain().in_set(UiSystems::Render),
            emit_actions.in_set(UiSystems::EmitIntents),
        )
            .run_if(in_state(Screen::Gameplay)),
    );
}

fn render(
    mut commands: Commands,
    roots: Query<Entity, With<DeploymentRoot>>,
    view: Res<DeploymentView>,
    assets: Res<UiAssets>,
) {
    if !view.is_changed() {
        return;
    }
    for root in &roots {
        commands.entity(root).despawn();
    }
    if !view.active {
        return;
    }

    commands
        .spawn((
            Name::new("Sandbox Deployment HUD"),
            DeploymentRoot,
            TabGroup {
                order: 20,
                modal: true,
            },
            crate::focus::ModalFocusScope,
            DespawnOnExit(Screen::Gameplay),
            // Only the painted card blocks the world. The rest of the canvas
            // remains an ordinary terrain-picking and camera lane.
            Pickable {
                should_block_lower: true,
                is_hoverable: false,
            },
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(16.0),
                top: Val::Px(16.0),
                width: Val::Px(430.0),
                padding: UiRect::all(Val::Px(14.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(10.0),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.015, 0.022, 0.035, 0.94)),
            BorderColor::all(Color::srgba(0.93, 0.79, 0.46, 0.62)),
            GlobalZIndex(11),
        ))
        .with_children(|card| {
            card.spawn(heading(&assets, format!("DEPLOY · {}", view.map_name)));
            spawn_current(card, &assets, &view);
            card.spawn(blurb(&assets, view.notice.clone()));
            spawn_queue(card, &assets, &view.queue);
            spawn_actions(card, &assets, &view);
        });
}

fn spawn_current(card: &mut ChildSpawnerCommands, assets: &UiAssets, view: &DeploymentView) {
    card.spawn((
        Name::new("Deployment Current Character"),
        DeploymentCurrent,
        Node {
            width: Val::Percent(100.0),
            min_width: Val::Px(0.0),
            padding: UiRect::all(Val::Px(10.0)),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(4.0),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(7.0)),
            ..default()
        },
        BackgroundColor(PANEL_BG),
        BorderColor::all(EDGE),
        Pickable::IGNORE,
    ))
    .with_children(|current| match view.stage {
        Some(SandboxDeploymentStage::Placing(slot)) => {
            let entry = view.queue.iter().find(|entry| entry.slot == slot);
            let name = entry.map_or("Character", |entry| entry.name.as_str());
            let (step, total) = deployment_progress(&view.queue, slot);
            current.spawn(heading(assets, format!("{slot} · {name}")));
            current.spawn(fine(
                assets,
                format!("CHARACTER {step} OF {total} · click any valid map surface"),
            ));
        }
        Some(SandboxDeploymentStage::Review) => {
            current.spawn(heading(assets, "Review deployment"));
            current.spawn(fine(
                assets,
                "All characters are placed. Reposition one or start combat.",
            ));
        }
        None => {
            current.spawn(heading(assets, "Preparing deployment"));
            current.spawn(fine(assets, "Waiting for the frozen roster."));
        }
    });
}

fn deployment_progress(
    queue: &[DeploymentQueueEntryView],
    slot: SandboxDeploymentSlot,
) -> (usize, usize) {
    let step = queue
        .iter()
        .position(|entry| entry.slot == slot)
        .map_or(1, |index| index + 1);
    (step, queue.len().max(1))
}

fn spawn_queue(
    card: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    queue: &[DeploymentQueueEntryView],
) {
    card.spawn(fine(assets, "PLACEMENT ORDER · PARTY THEN ENEMIES"));
    card.spawn((
        Name::new("Deployment Queue"),
        DeploymentQueue,
        Node {
            width: Val::Percent(100.0),
            min_width: Val::Px(0.0),
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            align_items: AlignItems::Center,
            column_gap: Val::Px(6.0),
            row_gap: Val::Px(6.0),
            ..default()
        },
    ))
    .with_children(|controls| {
        for entry in queue {
            spawn_queue_control(controls, assets, entry);
        }
    });
}

fn spawn_queue_control(
    controls: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    entry: &DeploymentQueueEntryView,
) {
    let short_side = match entry.slot.side {
        SandboxSide::Party => "P",
        SandboxSide::Enemies => "E",
    };
    let status = if entry.selected {
        "NOW"
    } else if entry.placed {
        "SET"
    } else if entry.selectable {
        "NEXT"
    } else {
        "WAIT"
    };
    let spoken_status = if entry.selected {
        "current placement"
    } else if entry.placed {
        "placed"
    } else if entry.selectable {
        "next in order"
    } else {
        "waiting for earlier characters"
    };
    let control_name = deployment_slot_name(entry.slot);
    let accessible =
        AccessibleLabel::new(format!("{}, {}, {spoken_status}", entry.slot, entry.name));
    let mut control = if entry.selectable {
        let mut control = controls.spawn((
            fixed_row_button(control_name, 56.0, 56.0),
            crate::UiVisibilityRequirement::Immediate,
            DeploymentIntent::SelectSlot(entry.slot),
        ));
        control
            .insert(accessible)
            .insert(BorderColor::all(if entry.selected { ACCENT } else { EDGE }));
        control
    } else {
        controls.spawn((
            Name::new(control_name),
            accessible,
            Node {
                width: Val::Px(56.0),
                height: Val::Px(56.0),
                min_width: Val::Px(56.0),
                min_height: Val::Px(56.0),
                flex_shrink: 0.0,
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            },
            BorderColor::all(EDGE),
            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.025)),
            Pickable::IGNORE,
        ))
    };
    control.with_children(|control| {
        control.spawn((
            Text::new(format!("{short_side}{}", entry.slot.slot.number())),
            compact_glyph_role(14.0),
            TextFont {
                font: assets.body.clone().into(),
                ..TextFont::from_font_size(14.0)
            },
            TextColor(LABEL),
            Pickable::IGNORE,
        ));
        control.spawn((
            Text::new(status),
            compact_glyph_role(9.0),
            TextFont {
                font: assets.body.clone().into(),
                ..TextFont::from_font_size(9.0)
            },
            TextColor(if entry.selected { ACCENT } else { LABEL }),
            Pickable::IGNORE,
        ));
    });
}

fn deployment_slot_name(slot: SandboxDeploymentSlot) -> String {
    format!("Deployment {} slot {}", slot.side, slot.slot)
}

fn spawn_actions(card: &mut ChildSpawnerCommands, assets: &UiAssets, view: &DeploymentView) {
    card.spawn((
        DeploymentActions,
        Node {
            width: Val::Percent(100.0),
            min_width: Val::Px(0.0),
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            justify_content: JustifyContent::FlexEnd,
            align_items: AlignItems::Center,
            column_gap: Val::Px(7.0),
            row_gap: Val::Px(7.0),
            ..default()
        },
    ))
    .with_children(|actions| {
        if view.can_undo {
            deployment_button(actions, assets, "Undo", 90.0, DeploymentIntent::Undo);
        }
        deployment_button(
            actions,
            assets,
            "Return to Sandbox",
            180.0,
            DeploymentIntent::Back,
        );
        if view.complete && view.stage == Some(SandboxDeploymentStage::Review) {
            deployment_button(
                actions,
                assets,
                "Start Combat",
                150.0,
                DeploymentIntent::StartCombat,
            );
        }
    });
}

fn deployment_button(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    text: &'static str,
    width: f32,
    action: DeploymentIntent,
) {
    parent
        .spawn((
            row_button(text, width),
            crate::UiVisibilityRequirement::Immediate,
            action,
        ))
        .with_child(label(assets, text));
}

fn emit_actions(
    clicked: Query<(&Interaction, &DeploymentIntent), Changed<Interaction>>,
    mut intents: MessageWriter<UiIntent>,
) {
    for (interaction, action) in &clicked {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::Deployment(*action));
        }
    }
}

fn apply_layout(
    metrics: Res<ResolvedUiMetrics>,
    added_roots: Query<(), Added<DeploymentRoot>>,
    mut roots: Query<&mut Node, With<DeploymentRoot>>,
    mut current: Query<
        &mut Node,
        (
            With<DeploymentCurrent>,
            Without<DeploymentRoot>,
            Without<DeploymentQueue>,
            Without<DeploymentActions>,
        ),
    >,
    mut queue: Query<
        &mut Node,
        (
            With<DeploymentQueue>,
            Without<DeploymentRoot>,
            Without<DeploymentCurrent>,
            Without<DeploymentActions>,
        ),
    >,
    mut actions: Query<
        &mut Node,
        (
            With<DeploymentActions>,
            Without<DeploymentRoot>,
            Without<DeploymentCurrent>,
            Without<DeploymentQueue>,
        ),
    >,
) {
    if !metrics.is_changed() && added_roots.is_empty() {
        return;
    }

    let compact = metrics.viewport == UiViewportClass::Compact;
    let ultra_constrained = is_ultra_constrained(*metrics);
    let canvas_inset = if ultra_constrained { 8.0 } else { 14.0 };
    let desired_width = if compact { 430.0 } else { 470.0 } * metrics.control_scale;
    // At every breakpoint the deployment card owns less than half the canvas.
    // The remaining lane belongs to terrain picking and camera navigation.
    let width = desired_width.min(metrics.logical_size.x * 0.46);
    for mut node in &mut roots {
        node.left = Val::Px(canvas_inset);
        node.top = Val::Px(canvas_inset);
        node.width = Val::Px(width);
        node.max_height = Val::Px((metrics.logical_size.y - canvas_inset * 2.0).max(0.0));
        node.padding = UiRect::all(Val::Px(12.0 * metrics.spacing_scale));
        node.row_gap = Val::Px(8.0 * metrics.spacing_scale);
        node.overflow = Overflow::clip();
    }
    for mut node in &mut current {
        node.padding = UiRect::all(Val::Px(9.0 * metrics.spacing_scale));
        node.row_gap = Val::Px(3.0 * metrics.spacing_scale);
    }
    for mut node in &mut queue {
        node.column_gap = Val::Px(5.0 * metrics.spacing_scale);
        node.row_gap = Val::Px(5.0 * metrics.spacing_scale);
    }
    for mut node in &mut actions {
        node.column_gap = Val::Px(6.0 * metrics.spacing_scale);
        node.row_gap = Val::Px(6.0 * metrics.spacing_scale);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_gameplay_model::SandboxSlotIndex;

    #[cfg(feature = "test-support")]
    use bevy::{
        input::{
            keyboard::{Key, KeyboardInput},
            ButtonState,
        },
        input_focus::InputFocus,
        window::PrimaryWindow,
    };

    #[cfg(feature = "test-support")]
    #[derive(Resource, Default)]
    struct IntentLog(Vec<UiIntent>);

    #[cfg(feature = "test-support")]
    fn record_intents(mut intents: MessageReader<UiIntent>, mut log: ResMut<IntentLog>) {
        log.0.extend(intents.read().cloned());
    }

    #[cfg(feature = "test-support")]
    fn press_key(app: &mut App, key_code: KeyCode, logical_key: Key) {
        let window = app
            .world_mut()
            .query_filtered::<Entity, With<PrimaryWindow>>()
            .single(app.world())
            .expect("the headless UI owns one primary window");
        for state in [ButtonState::Pressed, ButtonState::Released] {
            app.world_mut().write_message(KeyboardInput {
                key_code,
                logical_key: logical_key.clone(),
                state,
                text: None,
                repeat: false,
                window,
            });
            app.update();
        }
    }

    #[cfg(feature = "test-support")]
    fn focused_name(app: &App) -> Option<&str> {
        app.world()
            .resource::<InputFocus>()
            .get()
            .and_then(|entity| app.world().get::<Name>(entity))
            .map(Name::as_str)
    }

    fn assets() -> UiAssets {
        UiAssets {
            display: Handle::default(),
            body: Handle::default(),
            hex_cell: Handle::default(),
        }
    }

    fn slot(side: SandboxSide, slot: SandboxSlotIndex) -> SandboxDeploymentSlot {
        SandboxDeploymentSlot::new(side, slot)
    }

    fn queue_entry(
        side: SandboxSide,
        slot_index: SandboxSlotIndex,
        name: impl Into<String>,
        selected: bool,
        placed: bool,
    ) -> DeploymentQueueEntryView {
        DeploymentQueueEntryView {
            slot: slot(side, slot_index),
            name: name.into(),
            selected,
            placed,
            selectable: selected || placed,
        }
    }

    fn placing_view() -> DeploymentView {
        let party = slot(SandboxSide::Party, SandboxSlotIndex::One);
        DeploymentView {
            active: true,
            map_name: "Flat Arena".to_owned(),
            notice: "Click any valid map surface.".to_owned(),
            stage: Some(SandboxDeploymentStage::Placing(party)),
            queue: vec![
                queue_entry(
                    SandboxSide::Party,
                    SandboxSlotIndex::One,
                    "Hedge Mage",
                    true,
                    false,
                ),
                queue_entry(
                    SandboxSide::Enemies,
                    SandboxSlotIndex::One,
                    "Raider",
                    false,
                    false,
                ),
            ],
            can_undo: false,
            complete: false,
        }
    }

    #[cfg(feature = "test-support")]
    fn dense_queue() -> Vec<DeploymentQueueEntryView> {
        SandboxSide::ALL
            .into_iter()
            .flat_map(|side| {
                SandboxSlotIndex::ALL.into_iter().map(move |slot_index| {
                    queue_entry(
                        side,
                        slot_index,
                        format!("{side} Character {slot_index}"),
                        side == SandboxSide::Party && slot_index == SandboxSlotIndex::One,
                        false,
                    )
                })
            })
            .collect()
    }

    #[test]
    fn start_combat_exists_only_for_a_complete_review_projection() {
        let mut app = App::new();
        app.insert_resource(assets())
            .insert_resource(placing_view())
            .add_systems(Update, render);
        app.update();
        let mut actions = app.world_mut().query::<&DeploymentIntent>();
        assert!(!actions
            .iter(app.world())
            .any(|action| *action == DeploymentIntent::StartCombat));

        app.world_mut().resource_mut::<DeploymentView>().complete = true;
        app.update();
        let mut actions = app.world_mut().query::<&DeploymentIntent>();
        assert!(
            !actions
                .iter(app.world())
                .any(|action| *action == DeploymentIntent::StartCombat),
            "a complete repositioning step is not the final review"
        );

        app.world_mut().resource_mut::<DeploymentView>().stage =
            Some(SandboxDeploymentStage::Review);
        app.update();
        let mut actions = app.world_mut().query::<&DeploymentIntent>();
        assert!(actions
            .iter(app.world())
            .any(|action| *action == DeploymentIntent::StartCombat));
    }

    #[test]
    fn bulk_deployment_actions_are_not_rendered() {
        let mut app = App::new();
        app.insert_resource(assets())
            .insert_resource(placing_view())
            .add_systems(Update, render);
        app.update();

        let names = app
            .world_mut()
            .query::<&Name>()
            .iter(app.world())
            .map(|name| name.as_str().to_owned())
            .collect::<Vec<_>>();
        assert!(!names.iter().any(|name| matches!(
            name.as_str(),
            "Clear Party" | "Clear Enemies" | "Deterministic Auto-place"
        )));
    }

    #[test]
    fn future_slots_are_visible_without_an_enabled_selection_intent() {
        let mut app = App::new();
        app.insert_resource(assets())
            .insert_resource(placing_view())
            .add_systems(Update, render);
        app.update();

        let intents = app
            .world_mut()
            .query::<&DeploymentIntent>()
            .iter(app.world())
            .copied()
            .collect::<Vec<_>>();
        assert!(intents.contains(&DeploymentIntent::SelectSlot(slot(
            SandboxSide::Party,
            SandboxSlotIndex::One
        ))));
        assert!(!intents.contains(&DeploymentIntent::SelectSlot(slot(
            SandboxSide::Enemies,
            SandboxSlotIndex::One
        ))));
        assert!(app
            .world_mut()
            .query::<&Name>()
            .iter(app.world())
            .any(|name| name.as_str() == "Deployment Enemies slot 1"));
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn deployment_modal_traps_retains_and_activates_keyboard_focus() {
        let mut app = App::new();
        app.add_plugins(crate::test_support::HeadlessUiPlugin::new(1280, 720));
        app.init_resource::<IntentLog>()
            .add_systems(Last, record_intents);
        let mut view = placing_view();
        view.can_undo = true;
        app.world_mut().insert_resource(view);
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        for _ in 0..8 {
            app.update();
        }

        assert_eq!(focused_name(&app), Some("Deployment Party slot 1"));
        press_key(&mut app, KeyCode::Tab, Key::Tab);
        assert_eq!(
            focused_name(&app),
            Some("Undo"),
            "a future Enemy slot must not enter focus before the Party is placed"
        );

        // Guided progression redraws the card. Stable sparse-slot names retain
        // the exact keyboard identity while selection advances.
        {
            let mut view = app.world_mut().resource_mut::<DeploymentView>();
            view.stage = Some(SandboxDeploymentStage::Placing(slot(
                SandboxSide::Enemies,
                SandboxSlotIndex::One,
            )));
            for entry in &mut view.queue {
                entry.selected = entry.slot.side == SandboxSide::Enemies;
                if entry.slot.side == SandboxSide::Party {
                    entry.placed = true;
                }
                entry.selectable = entry.selected || entry.placed;
            }
        }
        app.update();
        assert_eq!(focused_name(&app), Some("Undo"));
        let enemy = app
            .world_mut()
            .query::<(Entity, &Name)>()
            .iter(app.world())
            .find_map(|(entity, name)| {
                (name.as_str() == "Deployment Enemies slot 1").then_some(entity)
            })
            .expect("the newly current Enemy slot must become an enabled control");
        app.insert_resource(InputFocus::from_entity(enemy));
        app.update();
        assert_eq!(focused_name(&app), Some("Deployment Enemies slot 1"));

        press_key(&mut app, KeyCode::Tab, Key::Tab);
        assert_eq!(focused_name(&app), Some("Undo"));
        press_key(&mut app, KeyCode::Enter, Key::Enter);
        assert!(app
            .world()
            .resource::<IntentLog>()
            .0
            .iter()
            .any(|intent| { matches!(intent, UiIntent::Deployment(DeploymentIntent::Undo)) }));
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn compact_deployment_blocks_only_its_card_and_preserves_the_world_lane() {
        let mut app = App::new();
        app.add_plugins(crate::test_support::HeadlessUiPlugin::new(1284, 744));
        let mut view = placing_view();
        view.queue = dense_queue();
        view.can_undo = true;
        app.world_mut().insert_resource(view);
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        for _ in 0..8 {
            app.update();
        }

        let root = app
            .world_mut()
            .query_filtered::<Entity, With<DeploymentRoot>>()
            .single(app.world())
            .expect("deployment must have one root");
        let root_size = app
            .world()
            .get::<ComputedNode>(root)
            .map(|node| node.size() * node.inverse_scale_factor)
            .expect("deployment root must be laid out");
        let canvas = Vec2::new(1284.0, 744.0);
        assert!(
            root_size.x <= canvas.x * 0.5,
            "the compact card must leave at least half the map horizontally visible: {root_size:?}"
        );
        assert!(
            root_size.x * root_size.y <= canvas.x * canvas.y * 0.45,
            "the deployment card must not cover most of the interactive map: {root_size:?}"
        );
        assert_eq!(
            app.world().get::<Pickable>(root),
            Some(&Pickable {
                should_block_lower: true,
                is_hoverable: false,
            }),
            "only the painted card blocks lower map picks"
        );

        let snapshot = crate::test_support::ui_tree_snapshot(app.world_mut());
        let issues = snapshot.task_issues(crate::test_support::UiTaskCase::DeploymentIncomplete);
        assert!(
            issues.is_empty(),
            "the guided card must retain its immediate controls: {issues:#?}"
        );
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn enlarged_queue_stays_visible_while_future_slots_remain_out_of_focus() {
        let mut app = App::new();
        app.add_plugins(crate::test_support::HeadlessUiPlugin::new(1280, 720));
        app.world_mut()
            .insert_resource(crate::UiScalePreference(crate::UiScaleMode::Percent200));
        app.world_mut().insert_resource(DeploymentView {
            active: true,
            map_name: "Stacked Surface Arena".to_owned(),
            notice: "Click any valid map surface.".to_owned(),
            stage: Some(SandboxDeploymentStage::Placing(slot(
                SandboxSide::Party,
                SandboxSlotIndex::One,
            ))),
            queue: dense_queue(),
            can_undo: true,
            complete: false,
        });
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        for _ in 0..8 {
            app.update();
        }

        let snapshot = crate::test_support::ui_tree_snapshot(app.world_mut());
        let issues = snapshot.task_issues(crate::test_support::UiTaskCase::DeploymentIncomplete);
        assert!(
            issues.is_empty(),
            "the complete guided queue must fit without a roster scroll: {issues:#?}"
        );
        let last = snapshot
            .nodes
            .iter()
            .find(|node| node.name == "Deployment Enemies slot 6")
            .expect("the exact sparse Enemy slot must be presented");
        assert!(
            last.fully_visible
                && !last.in_focus_order
                && last.accessible_label.as_deref().is_some_and(|label| {
                    label.contains("waiting for earlier characters")
                }),
            "the final future slot must stay visible and disclosed without bypassing order: {last:?}"
        );
        assert!(app
            .world_mut()
            .query_filtered::<&bevy::ui_widgets::ScrollArea, With<DeploymentQueue>>()
            .iter(app.world())
            .next()
            .is_none());
    }
}
