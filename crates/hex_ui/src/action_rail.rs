use bevy::input_focus::tab_navigation::TabGroup;
use bevy::prelude::*;
use bevy::ui_widgets::ScrollArea;
use hex_core::Screen;

use crate::{
    blurb, fine, heading, layout::is_ultra_constrained, row_button, ActionAvailability,
    DespawnOnExit, GameplayAction, GameplayHudView, LatticeIntent, ResolvedUiMetrics, UiAssets,
    UiIntent, UiViewportClass, ACCENT, EDGE, PANEL_BG,
};

#[derive(Component)]
pub(crate) struct ActionRail;

#[derive(Component)]
enum ActionRailCopy {
    Heading,
    Summary,
    Prompt,
}

#[derive(Component)]
struct ActionRailActions;

#[derive(Component)]
struct ActionRailKey(crate::GameplayAction);

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Screen::Gameplay), spawn_action_rail)
        .add_systems(
            Update,
            (
                refresh_action_rail.in_set(crate::UiSystems::Render),
                handle_action_rail.in_set(crate::UiSystems::EmitIntents),
            )
                .run_if(in_state(Screen::Gameplay)),
        );
}

fn spawn_action_rail(mut commands: Commands, assets: Res<UiAssets>) {
    commands
        .spawn((
            Name::new("Primary Action Rail"),
            ActionRail,
            ScrollArea,
            ScrollPosition::default(),
            TabGroup::new(10),
            DespawnOnExit(Screen::Gameplay),
            action_rail_node(UiViewportClass::Standard),
            BorderColor::all(ACCENT),
            BackgroundColor(PANEL_BG),
            GlobalZIndex(4),
        ))
        .with_children(|rail| {
            rail.spawn((ActionRailCopy::Heading, heading(&assets, "Now")));
            rail.spawn((
                ActionRailCopy::Summary,
                blurb(&assets, "Preparing actions…"),
            ));
            rail.spawn((ActionRailCopy::Prompt, blurb(&assets, "")));
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
    mut rails: Query<
        (Entity, &mut Node, &mut BorderColor),
        (With<ActionRail>, Without<ActionRailCopy>),
    >,
    mut copy: Query<
        (&ActionRailCopy, &mut Text, &mut Node),
        (With<ActionRailCopy>, Without<ActionRail>),
    >,
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
        apply_action_rail_layout(
            *metrics,
            &mut node,
            view.phase == hex_core::GameplayPhase::Deployment && view.actions.is_empty(),
        );
        *border = BorderColor::all(if view.required_prompt.is_some() {
            ACCENT
        } else {
            EDGE
        });
    }
    for (kind, mut text, mut node) in &mut copy {
        node.display = if is_ultra_constrained(*metrics)
            && matches!(kind, ActionRailCopy::Heading | ActionRailCopy::Prompt)
        {
            Display::None
        } else {
            Display::Flex
        };
        match kind {
            ActionRailCopy::Heading => {}
            ActionRailCopy::Summary => {
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
            ActionRailCopy::Prompt => {
                text.0 = view.required_prompt.clone().unwrap_or_else(|| {
                    "Choose an available action; unavailable actions explain why.".to_owned()
                });
            }
        }
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

fn action_rail_node(viewport: UiViewportClass) -> Node {
    let mut node = Node {
        position_type: PositionType::Absolute,
        min_height: Val::Px(116.0),
        padding: UiRect::axes(Val::Px(18.0), Val::Px(12.0)),
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(8.0),
        border: UiRect::all(Val::Px(2.0)),
        border_radius: BorderRadius::all(Val::Px(10.0)),
        ..default()
    };
    apply_action_rail_insets(viewport, &mut node);
    node
}

fn apply_action_rail_layout(metrics: ResolvedUiMetrics, node: &mut Node, minimal_deployment: bool) {
    apply_action_rail_insets(metrics.viewport, node);
    if is_ultra_constrained(metrics) {
        node.top = Val::Px(8.0);
        node.bottom = Val::Auto;
        node.padding = UiRect::axes(Val::Px(10.0), Val::Px(6.0));
        node.row_gap = Val::Px(4.0);
        node.min_height = Val::Px(if minimal_deployment { 48.0 } else { 64.0 });
        node.height = Val::Px(if minimal_deployment { 48.0 } else { 64.0 });
        node.overflow = if minimal_deployment {
            Overflow::default()
        } else {
            Overflow::scroll_y()
        };
    } else {
        node.top = Val::Auto;
        node.padding = UiRect::axes(Val::Px(18.0), Val::Px(12.0));
        node.row_gap = Val::Px(8.0);
        node.min_height = Val::Px(116.0);
        node.height = Val::Auto;
        node.overflow = Overflow::default();
    }
}

fn apply_action_rail_insets(viewport: UiViewportClass, node: &mut Node) {
    let (left, right, bottom) = match viewport {
        UiViewportClass::Compact => (12.0, 12.0, 12.0),
        UiViewportClass::Standard => (244.0, 320.0, 12.0),
        UiViewportClass::Wide => (280.0, 360.0, 16.0),
    };
    node.left = Val::Px(left);
    node.right = Val::Px(right);
    node.bottom = Val::Px(bottom);
}

fn handle_action_rail(
    clicks: Query<(&Interaction, &ActionRailKey), Changed<Interaction>>,
    mut intents: MessageWriter<UiIntent>,
) {
    for (interaction, action) in &clicks {
        if *interaction == Interaction::Pressed {
            intents.write(action_intent(action.0));
        }
    }
}

fn action_intent(action: GameplayAction) -> UiIntent {
    match action {
        GameplayAction::ConfirmDecision => UiIntent::Lattice(LatticeIntent::ConfirmDecision),
        action => UiIntent::Gameplay(action),
    }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::{Val, Vec2};

    use crate::{resolve_ui_metrics, UiScaleMode};

    use super::*;

    #[test]
    fn required_priority_is_reserved_for_blocking_choices() {
        assert!(crate::ActionPriority::Required > crate::ActionPriority::Primary);
    }

    #[test]
    fn rail_confirmation_uses_the_canonical_lattice_intent() {
        assert!(matches!(
            action_intent(GameplayAction::ConfirmDecision),
            UiIntent::Lattice(LatticeIntent::ConfirmDecision)
        ));
        assert!(matches!(
            action_intent(GameplayAction::EndTurn),
            UiIntent::Gameplay(GameplayAction::EndTurn)
        ));
    }

    #[test]
    fn required_rail_remains_reachable_across_the_structural_matrix() {
        for logical_size in [
            Vec2::new(960.0, 540.0),
            Vec2::new(1280.0, 720.0),
            Vec2::new(1920.0, 1080.0),
            Vec2::new(2560.0, 1440.0),
            Vec2::new(3840.0, 2160.0),
        ] {
            for mode in [UiScaleMode::Auto, UiScaleMode::Percent200] {
                let metrics = resolve_ui_metrics(logical_size, mode);
                let node = action_rail_node(metrics.viewport);
                let Val::Px(left) = node.left else {
                    panic!("the required rail needs a bounded left inset");
                };
                let Val::Px(right) = node.right else {
                    panic!("the required rail needs a bounded right inset");
                };
                let Val::Px(bottom) = node.bottom else {
                    panic!("the required rail needs a bounded bottom inset");
                };
                let Val::Px(min_height) = node.min_height else {
                    panic!("the required rail needs a bounded minimum height");
                };
                let available_width = metrics.effective_size.x - left - right;
                assert!(
                    available_width >= 156.0,
                    "a required action and its reason must fit at {logical_size:?} in {mode:?}"
                );
                assert!(
                    bottom + min_height <= metrics.effective_size.y,
                    "the required rail must remain on-canvas at {logical_size:?} in {mode:?}"
                );
            }
        }
    }
}
