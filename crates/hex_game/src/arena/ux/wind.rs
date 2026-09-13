//! Optional north-up wind instrument, consuming the same wind and clock as flight.
use bevy::prelude::*;
use hex_arena::ArenaSession;
use hex_core::{arena::ArenaOverview, ocean::OceanEnvironmentView};

use super::{hud, set_display, UxState, ViewState};

#[derive(Component)]
pub(super) struct WindPanel;
#[derive(Component)]
pub(super) struct WindArrow;
#[derive(Component)]
pub(super) struct WindDetails;

pub(in crate::arena) fn spawn(parent: &mut ChildSpawnerCommands) {
    parent
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(24),
                right: px(24),
                width: px(168),
                padding: UiRect::all(px(12)),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: px(6),
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::srgba(0.035, 0.07, 0.085, 0.96)),
            WindPanel,
            Name::new("Local wind instrument"),
        ))
        .with_children(|panel| {
            panel.spawn(hud::text("WIND  /  V", 20.0, Color::WHITE));
            panel.spawn(hud::text("NORTH UP", 14.0, Color::srgb(0.72, 0.82, 0.86)));
            panel.spawn((
                Node {
                    width: px(68),
                    height: px(68),
                    ..default()
                },
                hud::text("↑", 54.0, Color::srgb(0.50, 0.91, 1.0)),
                TextLayout::justify(Justify::Center),
                UiTransform::IDENTITY,
                WindArrow,
            ));
            panel.spawn((hud::text("", 20.0, Color::WHITE), WindDetails));
            panel.spawn(hud::text(
                "Blowing toward",
                14.0,
                Color::srgb(0.72, 0.82, 0.86),
            ));
        });
}

pub(super) fn present(
    ux: Res<UxState>,
    state: Res<ViewState>,
    session: Res<ArenaSession>,
    ocean: Option<Res<OceanEnvironmentView>>,
    overview: Option<Res<ArenaOverview>>,
    mut panels: Query<&mut Node, With<WindPanel>>,
    mut arrows: Query<&mut UiTransform, With<WindArrow>>,
    mut labels: Query<&mut Text, With<WindDetails>>,
) {
    let visible = ux.wind_visible
        && state.started
        && !state.paused
        && session.human_actor_id().is_some()
        && ocean.is_some();
    let beside_map = ux.map_visible
        && overview
            .as_ref()
            .is_some_and(|map| map.width > 0 && !map.rgba.is_empty());
    for mut panel in &mut panels {
        set_display(
            &mut panel,
            if visible {
                Display::Flex
            } else {
                Display::None
            },
        );
        let right = px(if beside_map { 336 } else { 24 });
        if panel.right != right {
            panel.right = right;
        }
    }
    if !visible {
        return;
    }
    let Some(ocean) = ocean else {
        return;
    };
    let velocity = ocean.wind.velocity_at(session.ocean_time());
    let heading = velocity.x.atan2(-velocity.y);
    for mut arrow in &mut arrows {
        arrow.set_if_neq(UiTransform::from_rotation(Rot2::radians(heading)));
    }
    let compass = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    let label = format!(
        "{} · {:.1} u/s",
        compass
            .get(super::direction_octant(velocity))
            .copied()
            .unwrap_or("N"),
        velocity.length()
    );
    for mut text in &mut labels {
        if text.0 != label {
            text.0.clone_from(&label);
        }
    }
}
