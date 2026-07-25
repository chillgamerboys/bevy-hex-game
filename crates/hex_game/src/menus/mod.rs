//! Menus drawn over the top of a screen, rather than replacing it.
//!
//! Distinct from [`crate::screens`]: a screen owns the whole frame and controls
//! what state the game is in, whereas a menu overlays one. The pause menu leaves
//! the world rendered behind it.
//!
//! Scaffolding. Layout and transitions are real; the option lists wait for the
//! design doc.

use bevy::prelude::*;

mod pause;

pub(super) fn plugin(app: &mut App) {
    app.add_plugins(pause::plugin);
}

/// A full-screen overlay that dims whatever is behind it.
pub fn overlay_root(name: &'static str) -> impl Bundle {
    (
        Name::new(name),
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(12.0),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
        GlobalZIndex(1),
    )
}
