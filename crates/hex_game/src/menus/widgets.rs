//! Buttons, panels, type, and the colours they are made of.
//!
//! The first interactive UI in this project — everything before it was static text
//! read with the keyboard. So this is less a widget library than a decision about what
//! one looks like, and the next menu should reach for these rather than inventing a
//! second set.
//!
//! # One place for the design system
//!
//! The palette, the five-step type scale, and the two vendored fonts all live here.
//! Every screen used to pick its own `TextFont::from_font_size(N)` and its own grey
//! literal — thirteen distinct sizes had accumulated by the time the first visual
//! walk photographed the result. Text helpers now take [`UiAssets`], so a screen
//! cannot accidentally fall back to the engine's placeholder font.
//!
//! # Hover is a system, not an observer
//!
//! [`Interaction`] is maintained by Bevy's UI picking, and reading it with a
//! `Changed<Interaction>` filter costs nothing on the frames where nothing moved. The
//! alternative — a `Pointer<Over>` observer per button — would fire globally and has
//! already caused one crash in this codebase by running in states it did not expect.

use bevy::prelude::*;

/// Background of a button at rest.
const RESTING: Color = Color::srgba(1.0, 1.0, 1.0, 0.08);

/// Background under the cursor. Deliberately a lift in brightness rather than a hue
/// change, so it reads the same against any screen behind it.
const HOVERED: Color = Color::srgba(1.0, 1.0, 1.0, 0.16);

/// Background while held down.
const PRESSED: Color = Color::srgba(1.0, 0.94, 0.75, 0.28);

/// Text on a button.
pub const LABEL: Color = Color::srgb(0.94, 0.94, 0.95);

/// Secondary text — the line under a heading.
pub const MUTED: Color = Color::srgba(0.88, 0.89, 0.92, 0.62);

/// The one warm accent: headings and the edge of anything primary.
pub const ACCENT: Color = Color::srgb(0.93, 0.79, 0.46);

/// The accent at edge strength, for borders that should glow rather than shout.
pub const ACCENT_EDGE: Color = Color::srgba(0.93, 0.79, 0.46, 0.4);

/// The hairline that gives a resting button its shape.
pub const EDGE: Color = Color::srgba(1.0, 1.0, 1.0, 0.13);

/// Panel fill — the gameplay HUD's proven dark surface, generalised.
pub const PANEL_BG: Color = Color::srgba(0.02, 0.03, 0.045, 0.72);

/// Display size — screen titles, in the display face.
pub const DISPLAY_SIZE: f32 = 46.0;

/// Section headings inside a screen or panel.
pub const TITLE_SIZE: f32 = 19.0;

/// Font size for a button's main label and body text.
pub const LABEL_SIZE: f32 = 16.0;

/// Font size for supporting text.
pub const BLURB_SIZE: f32 = 13.0;

/// The smallest legible line — seeds, costs, log entries.
pub const FINE_SIZE: f32 = 11.0;

/// The vendored fonts and shared UI art, loaded once at startup.
///
/// Cinzel carries display text (engraved, arcane — the game's voice); Inter
/// carries everything a player has to actually read. Both are OFL, vendored
/// under `assets/fonts/` with their licenses.
#[derive(Resource, Clone)]
pub struct UiAssets {
    /// Display face for titles and section headings.
    pub display: Handle<Font>,
    /// Text face for labels, body, and fine print.
    pub body: Handle<Font>,
    /// A white pointy-top hexagon, tinted per use.
    pub hex_cell: Handle<Image>,
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(PreStartup, load_ui_assets);
    // Not gated on any state: buttons belong to whatever screen spawned them, and that
    // screen despawns them on exit. A run condition here would only add a way for the
    // colours to get stuck.
    app.add_systems(Update, paint_interactions);
}

fn load_ui_assets(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(UiAssets {
        display: asset_server.load("fonts/Cinzel.ttf"),
        body: asset_server.load("fonts/Inter.ttf"),
        hex_cell: asset_server.load("ui/hex-cell.png"),
    });
}

/// A screen title in the display face.
#[must_use]
pub fn display(assets: &UiAssets, text: impl Into<String>) -> impl Bundle {
    (
        Text::new(text),
        TextFont {
            font: assets.display.clone().into(),
            ..TextFont::from_font_size(DISPLAY_SIZE)
        },
        TextColor(LABEL),
    )
}

/// A section heading, in the display face at reading size, accent-coloured.
#[must_use]
pub fn heading(assets: &UiAssets, text: impl Into<String>) -> impl Bundle {
    (
        Text::new(text),
        TextFont {
            font: assets.display.clone().into(),
            ..TextFont::from_font_size(TITLE_SIZE)
        },
        TextColor(ACCENT),
    )
}

/// The label line inside a button, or body text.
#[must_use]
pub fn label(assets: &UiAssets, text: impl Into<String>) -> impl Bundle {
    (
        Text::new(text),
        TextFont {
            font: assets.body.clone().into(),
            ..TextFont::from_font_size(LABEL_SIZE)
        },
        TextColor(LABEL),
        // Text inside a button must not take the pointer, or the button never sees a
        // hover once the cursor is over its own words.
        Pickable::IGNORE,
    )
}

/// The supporting line inside a button, or secondary body text.
#[must_use]
pub fn blurb(assets: &UiAssets, text: impl Into<String>) -> impl Bundle {
    (
        Text::new(text),
        TextFont {
            font: assets.body.clone().into(),
            ..TextFont::from_font_size(BLURB_SIZE)
        },
        TextColor(MUTED),
        Pickable::IGNORE,
    )
}

/// The smallest text tier — seeds, costs, log lines.
#[must_use]
pub fn fine(assets: &UiAssets, text: impl Into<String>) -> impl Bundle {
    (
        Text::new(text),
        TextFont {
            font: assets.body.clone().into(),
            ..TextFont::from_font_size(FINE_SIZE)
        },
        TextColor(MUTED),
        Pickable::IGNORE,
    )
}

/// A clickable button with room for a label and a line of supporting text.
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
            border: UiRect::all(Val::Px(1.0)),
            // `border_radius` is a field on `Node` in Bevy 0.19, not the separate
            // `BorderRadius` component it used to be — that type still exists and is
            // still constructible, it simply is not a `Component` any more, so putting
            // one in a bundle fails with "is not a Bundle" rather than "not found".
            border_radius: BorderRadius::all(Val::Px(6.0)),
            ..default()
        },
        BorderColor::all(EDGE),
        BackgroundColor(RESTING),
    )
}

/// A compact button for a secondary command inside a row.
///
/// Fixed dimensions so its row cannot resize when a numeric label changes, and
/// short enough that a table of them keeps an even rhythm.
#[must_use]
pub fn small_button(name: &'static str) -> impl Bundle {
    (
        Name::new(name),
        Button,
        Node {
            width: Val::Px(132.0),
            height: Val::Px(46.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(1.0),
            padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(6.0)),
            ..default()
        },
        BorderColor::all(EDGE),
        BackgroundColor(RESTING),
    )
}

/// A framed dark panel — the surface screens arrange their content on.
///
/// The gameplay HUD proved this style: a dark, mostly opaque plate reads
/// clearly over any background the game happens to render.
#[must_use]
pub fn panel() -> impl Bundle {
    (
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(12.0),
            padding: UiRect::all(Val::Px(18.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(10.0)),
            ..default()
        },
        BorderColor::all(EDGE),
        BackgroundColor(PANEL_BG),
    )
}

/// A thin horizontal rule between sections.
#[must_use]
pub fn divider(width: f32) -> impl Bundle {
    (
        Node {
            width: Val::Px(width),
            height: Val::Px(1.0),
            margin: UiRect::vertical(Val::Px(6.0)),
            ..default()
        },
        BackgroundColor(EDGE),
    )
}

/// Opts a node out of [`paint_interactions`]'s shared palette.
///
/// The lattice demo paints its cells by *game state* — disabled, locked,
/// element — and the shared hover repaint would overwrite that on every
/// pointer move. A node carrying this marker owns its `BackgroundColor`
/// entirely and gives up hover feedback in exchange.
#[derive(Component)]
pub struct OwnColors;

/// Keeps every button's background in step with what the pointer is doing.
fn paint_interactions(
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, Without<OwnColors>),
    >,
) {
    for (interaction, mut background) in &mut buttons {
        background.0 = match interaction {
            Interaction::Pressed => PRESSED,
            Interaction::Hovered => HOVERED,
            Interaction::None => RESTING,
        };
    }
}
