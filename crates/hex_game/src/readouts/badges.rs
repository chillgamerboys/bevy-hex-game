//! Screen-space identity badges for the acting unit and current target.

use bevy::prelude::*;
use bevy::transform::TransformSystems;
use hex_core::{GameplaySystems, Screen, UnitId};
use hex_units::UnitRegistry;
use hex_world::PanOrbitCamera;

use crate::screens::DespawnOnExit;
use hex_ui::{UiAssets, ACCENT, DANGER, LABEL, PANEL_BG};

use super::{GameplayUiContext, HudElement};

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
enum BadgeRole {
    Acting,
    Target,
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct UnitBadge {
    unit: UnitId,
    role: BadgeRole,
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        rebuild
            .after(GameplaySystems::UiContext)
            .run_if(in_state(Screen::Gameplay)),
    )
    .add_systems(
        PostUpdate,
        follow_units
            .after(TransformSystems::Propagate)
            .run_if(in_state(Screen::Gameplay)),
    );
}

fn rebuild(
    mut commands: Commands,
    context: Res<GameplayUiContext>,
    existing: Query<Entity, With<UnitBadge>>,
    assets: Res<UiAssets>,
) {
    if !context.is_changed() {
        return;
    }
    for entity in &existing {
        commands.entity(entity).despawn();
    }

    if let Some(actor) = context.acting.as_ref() {
        let role = if actor.faction == hex_units::Faction::Player {
            "ACTIVE"
        } else {
            "ENEMY ACTING"
        };
        spawn_badge(
            &mut commands,
            &assets,
            actor.unit,
            BadgeRole::Acting,
            format!("{role} · {}", actor.label()),
        );
    }
    if let Some((provenance, target)) = context.target.as_ref() {
        spawn_badge(
            &mut commands,
            &assets,
            target.unit,
            BadgeRole::Target,
            format!("{} · {}", provenance.label(), target.label()),
        );
    }
}

fn spawn_badge(
    commands: &mut Commands,
    assets: &UiAssets,
    unit: UnitId,
    role: BadgeRole,
    label: String,
) {
    let accent = match role {
        BadgeRole::Acting => ACCENT,
        BadgeRole::Target => DANGER,
    };
    commands
        .spawn((
            Name::new(match role {
                BadgeRole::Acting => "Acting Unit Badge",
                BadgeRole::Target => "Target Unit Badge",
            }),
            UnitBadge { unit, role },
            HudElement,
            Node {
                display: Display::None,
                position_type: PositionType::Absolute,
                width: Val::Px(190.0),
                min_height: Val::Px(28.0),
                margin: UiRect::left(Val::Px(-95.0)),
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
            Text::new(label),
            TextFont {
                font: assets.body.clone().into(),
                ..TextFont::from_font_size(18.0)
            },
            TextColor(LABEL),
            Pickable::IGNORE,
        ));
}

fn follow_units(
    registry: Res<UnitRegistry>,
    units: Query<&GlobalTransform>,
    cameras: Query<(&Camera, &GlobalTransform), With<PanOrbitCamera>>,
    mut badges: Query<(&UnitBadge, &mut Node)>,
) {
    let Ok((camera, camera_transform)) = cameras.single() else {
        return;
    };
    for (badge, mut node) in &mut badges {
        let position = registry
            .entity_of(badge.unit)
            .and_then(|entity| units.get(entity).ok())
            .and_then(|transform| {
                camera
                    .world_to_viewport(camera_transform, transform.translation() + Vec3::Y)
                    .ok()
            });
        let Some(position) = position else {
            node.display = Display::None;
            continue;
        };
        node.display = Display::Flex;
        node.left = Val::Px(position.x);
        node.top = Val::Px(
            position.y
                - if badge.role == BadgeRole::Acting {
                    48.0
                } else {
                    16.0
                },
        );
    }
}
