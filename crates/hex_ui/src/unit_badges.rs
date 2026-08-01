//! Screen-space unit badges from projected, disclosure-safe anchors.

use bevy::prelude::*;
use hex_core::Screen;

use crate::{
    body_text_role, responsive_control_role, BadgeKind, DespawnOnExit, HudElement, UiAssets,
    UiSystems, UnitBadgeView, UnitBadgesView, ACCENT, BLURB_SIZE, DANGER, LABEL, PANEL_BG,
};

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct BadgeRoot(BadgeKind);

#[derive(Component)]
struct BadgeLabel;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Screen::Gameplay), spawn_badges)
        .add_systems(
            PostUpdate,
            render_badges
                .in_set(UiSystems::Render)
                .run_if(in_state(Screen::Gameplay)),
        );
}

fn spawn_badges(mut commands: Commands, assets: Res<UiAssets>) {
    spawn_badge(&mut commands, &assets, BadgeKind::Acting);
    spawn_badge(&mut commands, &assets, BadgeKind::Target);
}

fn spawn_badge(commands: &mut Commands, assets: &UiAssets, kind: BadgeKind) {
    let (name, accent) = match kind {
        BadgeKind::Acting => ("Acting Unit Badge", ACCENT),
        BadgeKind::Target => ("Target Unit Badge", DANGER),
    };
    commands
        .spawn((
            Name::new(name),
            BadgeRoot(kind),
            HudElement,
            responsive_control_role(),
            Node {
                display: Display::None,
                position_type: PositionType::Absolute,
                width: Val::Px(240.0),
                min_height: Val::Px(44.0),
                margin: UiRect::left(Val::Px(-120.0)),
                padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)),
                justify_content: JustifyContent::Center,
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            },
            BorderColor::all(accent),
            BackgroundColor(PANEL_BG),
            GlobalZIndex(5),
            Pickable::IGNORE,
            DespawnOnExit(Screen::Gameplay),
        ))
        .with_child((
            BadgeLabel,
            Text::new(""),
            body_text_role(),
            TextFont {
                font: assets.body.clone().into(),
                ..TextFont::from_font_size(BLURB_SIZE)
            },
            TextColor(LABEL),
            Pickable::IGNORE,
        ));
}

fn render_badges(
    view: Res<UnitBadgesView>,
    mut roots: Query<(&BadgeRoot, &mut Node, &Children)>,
    mut labels: Query<&mut Text, With<BadgeLabel>>,
) {
    if !view.is_changed() {
        return;
    }
    for (root, mut node, children) in &mut roots {
        let badge = match root.0 {
            BadgeKind::Acting => view.acting.as_ref(),
            BadgeKind::Target => view.target.as_ref(),
        };
        let Some(UnitBadgeView { label, anchor, .. }) = badge else {
            node.display = Display::None;
            continue;
        };
        let Some(anchor) = anchor else {
            node.display = Display::None;
            continue;
        };
        node.display = Display::Flex;
        node.left = Val::Px(anchor.x);
        node.top = Val::Px(badge_top(root.0, *anchor, &view));
        for child in children.iter() {
            if let Ok(mut text) = labels.get_mut(child) {
                text.0.clone_from(label);
            }
        }
    }
}

fn badge_top(kind: BadgeKind, anchor: Vec2, view: &UnitBadgesView) -> f32 {
    match kind {
        BadgeKind::Acting => anchor.y - 48.0,
        BadgeKind::Target => {
            let base = anchor.y - 96.0;
            let Some(acting) = view.acting.as_ref().and_then(|badge| badge.anchor) else {
                return base;
            };
            if (acting.x - anchor.x).abs() < 240.0 {
                base.min(acting.y - 48.0 - 64.0)
            } else {
                base
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::UnitId;

    #[test]
    fn badge_view_keeps_identity_separate_from_disclosed_label() {
        let badge = UnitBadgeView {
            unit: UnitId(0),
            kind: BadgeKind::Acting,
            label: "ACTIVE · ALLY 1".to_owned(),
            anchor: Some(Vec2::new(320.0, 240.0)),
        };
        assert_eq!(badge.unit, UnitId(0));
        assert!(badge.label.starts_with("ACTIVE"));
    }

    #[test]
    fn nearby_target_badge_stacks_above_the_actor() {
        let view = UnitBadgesView {
            acting: Some(UnitBadgeView {
                unit: UnitId(0),
                kind: BadgeKind::Acting,
                label: "ACTIVE".to_owned(),
                anchor: Some(Vec2::new(500.0, 400.0)),
            }),
            target: None,
        };
        let target_top = badge_top(BadgeKind::Target, Vec2::new(520.0, 420.0), &view);
        assert!(target_top <= 288.0);
    }
}
