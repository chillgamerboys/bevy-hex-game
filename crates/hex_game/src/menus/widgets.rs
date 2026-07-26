//! Buttons, and the colours they are made of.
//!
//! The first interactive UI in this project — everything before it was static text
//! read with the keyboard. So this is less a widget library than a decision about what
//! one looks like, and the next menu should reach for these rather than inventing a
//! second set.
//!
//! # One place for the colours
//!
//! Every screen so far picks its own `TextFont::from_font_size(N)` and its own grey
//! literal, which is already tedious to keep consistent and would be worse with states
//! to match. The palette lives here instead.
//!
//! # Hover is a system, not an observer
//!
//! [`Interaction`] is maintained by Bevy's UI picking, and reading it with a
//! `Changed<Interaction>` filter costs nothing on the frames where nothing moved. The
//! alternative — a `Pointer<Over>` observer per button — would fire globally and has
//! already caused one crash in this codebase by running in states it did not expect.

use bevy::prelude::*;

/// Background of a button at rest.
const RESTING: Color = Color::srgba(1.0, 1.0, 1.0, 0.06);

/// Background under the cursor. Deliberately a lift in brightness rather than a hue
/// change, so it reads the same against any screen behind it.
const HOVERED: Color = Color::srgba(1.0, 1.0, 1.0, 0.16);

/// Background while held down.
const PRESSED: Color = Color::srgba(1.0, 0.94, 0.75, 0.28);

/// Text on a button.
pub const LABEL: Color = Color::srgb(0.95, 0.95, 0.95);

/// Secondary text — the line under a heading.
pub const MUTED: Color = Color::srgba(0.9, 0.9, 0.9, 0.65);

/// Font size for a button's main label.
pub const LABEL_SIZE: f32 = 22.0;

/// Font size for supporting text.
pub const BLURB_SIZE: f32 = 14.0;

pub(super) fn plugin(app: &mut App) {
    // Not gated on any state: buttons belong to whatever screen spawned them, and that
    // screen despawns them on exit. A run condition here would only add a way for the
    // colours to get stuck.
    app.add_systems(Update, paint_interactions);
}

/// A clickable button with a label and a line of supporting text.
///
/// Returns the bundle rather than spawning, so the caller decides the parent and adds
/// its own marker component — which is how the click is identified later.
#[must_use]
pub fn button(name: &'static str) -> impl Bundle {
    (
        Name::new(name),
        Button,
        Node {
            width: Val::Px(420.0),
            padding: UiRect::axes(Val::Px(20.0), Val::Px(12.0)),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(4.0),
            // `border_radius` is a field on `Node` in Bevy 0.19, not the separate
            // `BorderRadius` component it used to be — that type still exists and is
            // still constructible, it simply is not a `Component` any more, so putting
            // one in a bundle fails with "is not a Bundle" rather than "not found".
            border_radius: BorderRadius::all(Val::Px(6.0)),
            ..default()
        },
        BackgroundColor(RESTING),
    )
}

/// The label line inside a button.
#[must_use]
pub fn label(text: impl Into<String>) -> impl Bundle {
    (
        Text::new(text),
        TextFont::from_font_size(LABEL_SIZE),
        TextColor(LABEL),
        // Text inside a button must not take the pointer, or the button never sees a
        // hover once the cursor is over its own words.
        Pickable::IGNORE,
    )
}

/// The supporting line inside a button.
#[must_use]
pub fn blurb(text: impl Into<String>) -> impl Bundle {
    (
        Text::new(text),
        TextFont::from_font_size(BLURB_SIZE),
        TextColor(MUTED),
        Pickable::IGNORE,
    )
}

/// Keeps every button's background in step with what the pointer is doing.
fn paint_interactions(
    mut buttons: Query<(&Interaction, &mut BackgroundColor), Changed<Interaction>>,
) {
    for (interaction, mut background) in &mut buttons {
        background.0 = match interaction {
            Interaction::Pressed => PRESSED,
            Interaction::Hovered => HOVERED,
            Interaction::None => RESTING,
        };
    }
}
