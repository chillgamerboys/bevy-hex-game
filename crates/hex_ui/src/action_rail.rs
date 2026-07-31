use bevy::prelude::*;
use hex_core::Screen;

use crate::{
    blurb, fine, heading, row_button, ActionAvailability, DespawnOnExit, GameplayHudView,
    ResolvedUiMetrics, UiAssets, UiIntent, UiViewportClass, ACCENT, EDGE, PANEL_BG,
};

#[derive(Component)]
pub(crate) struct ActionRail;

#[derive(Component)]
struct ActionRailSummary;

#[derive(Component)]
struct ActionRailPrompt;

#[derive(Component)]
struct ActionRailActions;

#[derive(Component)]
struct ActionRailKey(crate::GameplayAction);

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Screen::Gameplay), spawn_action_rail)
        .add_systems(
            Update,
            (refresh_action_rail, handle_action_rail).run_if(in_state(Screen::Gameplay)),
        );
}

fn spawn_action_rail(mut commands: Commands, assets: Res<UiAssets>) {
    commands
        .spawn((
            Name::new("Primary Action Rail"),
            ActionRail,
            DespawnOnExit(Screen::Gameplay),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(244.0),
                right: Val::Px(320.0),
                bottom: Val::Px(12.0),
                min_height: Val::Px(116.0),
                padding: UiRect::axes(Val::Px(18.0), Val::Px(12.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::all(Val::Px(10.0)),
                ..default()
            },
            BorderColor::all(ACCENT),
            BackgroundColor(PANEL_BG),
            GlobalZIndex(4),
        ))
        .with_children(|rail| {
            rail.spawn(heading(&assets, "Now"));
            rail.spawn((ActionRailSummary, blurb(&assets, "Preparing actions…")));
            rail.spawn((ActionRailPrompt, blurb(&assets, "")));
            rail.spawn((
                Name::new("Primary Action Rail Controls"),
                ActionRailActions,
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    column_gap: Val::Px(8.0),
                    row_gap: Val::Px(6.0),
                    ..default()
                },
            ));
        });
}

fn refresh_action_rail(
    view: Res<GameplayHudView>,
    review: Option<Res<crate::review::UiReviewPresentation>>,
    metrics: Res<ResolvedUiMetrics>,
    assets: Res<UiAssets>,
    mut commands: Commands,
    mut rails: Query<(Entity, &mut Node, &mut BorderColor), With<ActionRail>>,
    mut summaries: Query<&mut Text, With<ActionRailSummary>>,
    mut prompts: Query<&mut Text, (With<ActionRailPrompt>, Without<ActionRailSummary>)>,
    actions: Query<Entity, With<ActionRailActions>>,
) {
    let review_changed = review.as_ref().is_some_and(|review| review.is_changed());
    if !view.is_changed() && !review_changed && !metrics.is_changed() {
        return;
    }
    let view = review
        .as_ref()
        .and_then(|review| review.hud.as_ref())
        .unwrap_or(view.as_ref());
    if let Ok((_, mut node, mut border)) = rails.single_mut() {
        match metrics.viewport {
            UiViewportClass::Compact => {
                node.left = Val::Px(12.0);
                node.right = Val::Px(12.0);
                node.bottom = Val::Px(12.0);
            }
            UiViewportClass::Standard => {
                node.left = Val::Px(244.0);
                node.right = Val::Px(320.0);
                node.bottom = Val::Px(12.0);
            }
            UiViewportClass::Wide => {
                node.left = Val::Px(280.0);
                node.right = Val::Px(360.0);
                node.bottom = Val::Px(16.0);
            }
        }
        *border = BorderColor::all(if view.required_prompt.is_some() {
            ACCENT
        } else {
            EDGE
        });
    }
    if let Ok(mut text) = summaries.single_mut() {
        text.0 = format!(
            "{} · {} · Move {} · Action {}",
            view.round,
            view.actor_label,
            view.movement_remaining,
            if view.action_remaining {
                "ready"
            } else {
                "spent"
            }
        );
    }
    if let Ok(mut text) = prompts.single_mut() {
        text.0 = view.required_prompt.clone().unwrap_or_else(|| {
            "Choose an available action; unavailable actions explain why.".to_owned()
        });
    }
    let Ok(action_root) = actions.single() else {
        return;
    };
    commands.entity(action_root).despawn_related::<Children>();
    commands.entity(action_root).with_children(|root| {
        let mut offered = view.actions.clone();
        offered.sort_by_key(|action| std::cmp::Reverse(action.priority));
        for action in offered {
            let name = format!("Action Rail {}", action.label);
            match action.availability {
                ActionAvailability::Enabled => {
                    root.spawn((row_button(name, 156.0), ActionRailKey(action.action)))
                        .with_children(|button| {
                            button.spawn(blurb(&assets, action.label));
                            if let Some(shortcut) = action.shortcut {
                                button.spawn(fine(&assets, shortcut));
                            }
                        });
                }
                ActionAvailability::Disabled { reason } => {
                    root.spawn((
                        Name::new(name),
                        Node {
                            width: Val::Px(156.0),
                            min_height: Val::Px(48.0),
                            padding: UiRect::axes(Val::Px(10.0), Val::Px(5.0)),
                            flex_direction: FlexDirection::Column,
                            justify_content: JustifyContent::Center,
                            border: UiRect::all(Val::Px(1.0)),
                            border_radius: BorderRadius::all(Val::Px(6.0)),
                            ..default()
                        },
                        BorderColor::all(EDGE),
                        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.035)),
                    ))
                    .with_children(|disabled| {
                        disabled.spawn(blurb(&assets, action.label));
                        disabled.spawn(fine(&assets, format!("Unavailable · {reason}")));
                    });
                }
            }
        }
    });
}

fn handle_action_rail(
    clicks: Query<(&Interaction, &ActionRailKey), Changed<Interaction>>,
    mut intents: MessageWriter<UiIntent>,
) {
    for (interaction, action) in &clicks {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::Gameplay(action.0));
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn required_priority_is_reserved_for_blocking_choices() {
        assert!(crate::ActionPriority::Required > crate::ActionPriority::Primary);
    }
}
