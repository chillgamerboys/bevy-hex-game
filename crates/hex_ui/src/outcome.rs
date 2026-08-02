//! Minimal encounter-outcome presentation.

use bevy::prelude::*;
use hex_core::Screen;

use crate::{
    blurb, heading, overlay_root, row_button, DespawnOnExit, OutcomeAction, OutcomeIntent,
    OutcomeView, UiAssets, UiIntent, UiSystems,
};

const OUTCOME_PANEL_BG: Color = Color::srgb(0.02, 0.03, 0.045);
// Terminal encounter decisions supersede deployment if their lifecycle briefly
// overlaps during the transition out of placement.
const OUTCOME_MODAL_Z: i32 = 12;

#[derive(Component)]
struct OutcomeRoot;

#[derive(Component, Debug, Clone, Copy)]
struct Control(OutcomeIntent);

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Screen::Gameplay), spawn)
        .add_systems(
            Update,
            (
                render.in_set(UiSystems::Render),
                emit_intents.in_set(UiSystems::EmitIntents),
            )
                .run_if(in_state(Screen::Gameplay)),
        );
}

fn spawn(mut commands: Commands) {
    commands
        .spawn((
            overlay_root("Encounter Outcome Modal"),
            OutcomeRoot,
            DespawnOnExit(Screen::Gameplay),
            Visibility::Hidden,
        ))
        .insert(GlobalZIndex(OUTCOME_MODAL_Z));
}

fn render(
    mut commands: Commands,
    view: Res<OutcomeView>,
    review: Option<Res<crate::review::UiReviewPresentation>>,
    roots: Query<Entity, With<OutcomeRoot>>,
    assets: Res<UiAssets>,
) {
    let review_changed = review.as_ref().is_some_and(|review| review.is_changed());
    if !view.is_changed() && !review_changed {
        return;
    }
    let view = review
        .as_ref()
        .and_then(|review| review.outcome.as_ref())
        .unwrap_or(view.as_ref());
    for root in &roots {
        commands.entity(root).despawn_related::<Children>();
        commands.entity(root).insert(if view.visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if !view.visible {
            continue;
        }
        commands.entity(root).with_children(|overlay| {
            overlay
                .spawn((
                    Name::new("Encounter Outcome Panel"),
                    Node {
                        width: Val::Px(520.0),
                        max_width: Val::Percent(90.0),
                        max_height: Val::Percent(90.0),
                        padding: UiRect::all(Val::Px(28.0)),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: Val::Px(16.0),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(10.0)),
                        ..default()
                    },
                    BorderColor::all(Color::srgba(0.93, 0.79, 0.46, 0.5)),
                    BackgroundColor(OUTCOME_PANEL_BG),
                ))
                .with_children(|panel| {
                    panel.spawn(heading(&assets, view.title.clone()));
                    panel.spawn(blurb(&assets, view.detail.clone()));
                    spawn_actions(panel, &assets, &view.actions);
                });
        });
    }
}

fn spawn_actions(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    actions: &[crate::OutcomeActionView],
) {
    parent
        .spawn((
            Name::new("Outcome Actions"),
            Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(10.0),
                row_gap: Val::Px(8.0),
                flex_wrap: FlexWrap::Wrap,
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|buttons| {
            for action in actions {
                buttons
                    .spawn((
                        row_button(action.label.clone(), action_width(action.action)),
                        Control(OutcomeIntent::Activate(action.action)),
                    ))
                    .with_child(blurb(assets, action.label.clone()));
            }
        });
}

fn action_width(action: OutcomeAction) -> f32 {
    match action {
        OutcomeAction::Continue | OutcomeAction::Retry | OutcomeAction::RetryExact => 150.0,
        OutcomeAction::Return => 180.0,
    }
}

fn emit_intents(
    clicked: Query<(&Interaction, &Control), Changed<Interaction>>,
    mut intents: MessageWriter<UiIntent>,
) {
    for (interaction, control) in &clicked {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::Outcome(control.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn outcome_actions_keep_clear_primary_and_return_targets() {
        assert!((action_width(OutcomeAction::Continue) - 150.0).abs() < f32::EPSILON);
        assert!((action_width(OutcomeAction::RetryExact) - 150.0).abs() < f32::EPSILON);
        assert!((action_width(OutcomeAction::Return) - 180.0).abs() < f32::EPSILON);
    }

    #[test]
    fn outcome_surface_is_opaque_over_live_gameplay() {
        assert!((OUTCOME_PANEL_BG.to_srgba().alpha - 1.0).abs() < f32::EPSILON);
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn outcome_modal_hands_off_retains_and_activates_keyboard_focus() {
        let mut app = App::new();
        app.add_plugins(crate::test_support::HeadlessUiPlugin::new(1280, 720));
        app.init_resource::<IntentLog>()
            .add_systems(Last, record_intents);
        app.world_mut().insert_resource(OutcomeView {
            visible: true,
            title: "VICTORY".to_owned(),
            detail: "The Enemy roster can no longer continue.".to_owned(),
            actions: vec![
                crate::OutcomeActionView {
                    action: OutcomeAction::RetryExact,
                    label: "Retry Exact".to_owned(),
                },
                crate::OutcomeActionView {
                    action: OutcomeAction::Return,
                    label: "Return to Sandbox".to_owned(),
                },
            ],
        });
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        for _ in 0..8 {
            app.update();
        }

        assert_eq!(focused_name(&app), Some("Retry Exact"));
        press_key(&mut app, KeyCode::Tab, Key::Tab);
        assert_eq!(
            focused_name(&app),
            Some("Return to Sandbox"),
            "real Tab input must remain trapped in the visible outcome modal"
        );

        // Outcome copy can refresh while the modal remains visible. Rebuilding its
        // descendants must preserve the exact action selected by the keyboard.
        app.world_mut()
            .resource_mut::<OutcomeView>()
            .detail
            .push_str(" Review the exact launch.");
        app.update();
        assert_eq!(focused_name(&app), Some("Return to Sandbox"));

        press_key(&mut app, KeyCode::Enter, Key::Enter);
        assert!(app.world().resource::<IntentLog>().0.iter().any(|intent| {
            matches!(
                intent,
                UiIntent::Outcome(OutcomeIntent::Activate(OutcomeAction::Return))
            )
        }));
    }
}
