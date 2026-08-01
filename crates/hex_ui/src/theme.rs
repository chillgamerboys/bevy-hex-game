use bevy::input_focus::tab_navigation::TabIndex;
use bevy::prelude::*;
use hex_assets::ElementCatalog;
use hex_core::ElementId;

const RESTING: Color = Color::srgba(1.0, 1.0, 1.0, 0.08);
const HOVERED: Color = Color::srgba(1.0, 1.0, 1.0, 0.16);
const PRESSED: Color = Color::srgba(1.0, 0.94, 0.75, 0.28);
/// Primary readable text.
pub const LABEL: Color = Color::srgb(0.96, 0.96, 0.97);
/// Supporting readable text.
pub const MUTED: Color = Color::srgb(0.76, 0.78, 0.82);
/// Warm primary accent.
pub const ACCENT: Color = Color::srgb(0.93, 0.79, 0.46);
/// Accent-strength border.
pub const ACCENT_EDGE: Color = Color::srgba(0.93, 0.79, 0.46, 0.55);
/// Errors and destructive outcomes.
pub const DANGER: Color = Color::srgb(0.94, 0.36, 0.30);
/// Resting control edge.
pub const EDGE: Color = Color::srgba(1.0, 1.0, 1.0, 0.20);
/// Opaque-enough backing for text over any world surface.
pub const PANEL_BG: Color = Color::srgba(0.02, 0.03, 0.045, 0.96);
/// Basic gem tint.
pub const GEM_COLOR: Color = Color::srgba(0.16, 0.45, 0.52, 0.92);
/// Fusion tint.
pub const FUSION_COLOR: Color = Color::srgba(0.42, 0.30, 0.62, 0.92);

/// Display title size at 100% scale.
pub const DISPLAY_SIZE: f32 = 48.0;
/// Top-level screen title size at 100% scale.
pub const SCREEN_TITLE_SIZE: f32 = 32.0;
/// Section heading size at 100% scale.
pub const TITLE_SIZE: f32 = 24.0;
/// Essential body and control size at 100% scale.
pub const LABEL_SIZE: f32 = 20.0;
/// Supporting text size at 100% scale.
pub const BLURB_SIZE: f32 = 18.0;
/// Optional metadata size at 100% scale.
pub const FINE_SIZE: f32 = 16.0;

/// Opts a control out of shared interaction paint because it owns state colors.
#[derive(Component)]
pub struct OwnColors;

/// Shared runtime fonts and UI art.
#[derive(Resource, Clone)]
pub struct UiAssets {
    /// Display face.
    pub display: Handle<Font>,
    /// Readable body face.
    pub body: Handle<Font>,
    /// Tintable pointy-top hexagon.
    pub hex_cell: Handle<Image>,
}

pub(super) fn plugin(app: &mut App) {
    let asset_server = app.world().resource::<AssetServer>();
    app.insert_resource(UiAssets {
        display: asset_server.load("fonts/Cinzel.ttf"),
        body: asset_server.load("fonts/Inter.ttf"),
        hex_cell: asset_server.load("ui/hex-cell.png"),
    });
    app.add_systems(Update, paint_interactions);
}

/// Resolves an element's presentation tint from authored wheel order.
#[must_use]
pub fn element_color(element: Option<ElementId>, elements: &ElementCatalog) -> Color {
    let Some(id) = element else { return GEM_COLOR };
    let wheel = elements.wheel();
    let Some(step) = wheel.iter().position(|candidate| *candidate == id) else {
        return FUSION_COLOR;
    };
    let step = u16::try_from(step).unwrap_or(0);
    let spokes = u16::try_from(wheel.len()).unwrap_or(1).max(1);
    let base = Hsla::from(GEM_COLOR);
    let hue = (base.hue + 360.0 * f32::from(step) / f32::from(spokes)).rem_euclid(360.0);
    Color::from(Hsla::new(hue, base.saturation, base.lightness, base.alpha))
}

/// Display title.
#[must_use]
pub fn display(assets: &UiAssets, text: impl Into<String>) -> impl Bundle {
    text_bundle(assets.display.clone(), text, DISPLAY_SIZE, LABEL)
}

/// Top-level screen title.
#[must_use]
pub fn screen_title(assets: &UiAssets, text: impl Into<String>) -> impl Bundle {
    text_bundle(assets.display.clone(), text, SCREEN_TITLE_SIZE, LABEL)
}

/// Section heading.
#[must_use]
pub fn heading(assets: &UiAssets, text: impl Into<String>) -> impl Bundle {
    text_bundle(assets.display.clone(), text, TITLE_SIZE, ACCENT)
}

/// Essential body or control text.
#[must_use]
pub fn label(assets: &UiAssets, text: impl Into<String>) -> impl Bundle {
    text_bundle(assets.body.clone(), text, LABEL_SIZE, LABEL)
}

/// Supporting text.
#[must_use]
pub fn blurb(assets: &UiAssets, text: impl Into<String>) -> impl Bundle {
    text_bundle(assets.body.clone(), text, BLURB_SIZE, MUTED)
}

/// Optional metadata text.
#[must_use]
pub fn fine(assets: &UiAssets, text: impl Into<String>) -> impl Bundle {
    text_bundle(assets.body.clone(), text, FINE_SIZE, MUTED)
}

fn text_bundle(
    font: Handle<Font>,
    text: impl Into<String>,
    size: f32,
    color: Color,
) -> impl Bundle {
    (
        Text::new(text),
        TextFont {
            font: font.into(),
            ..TextFont::from_font_size(size)
        },
        TextColor(color),
        Pickable::IGNORE,
    )
}

/// Standard full-width menu action.
#[must_use]
pub fn button(name: impl Into<String>) -> impl Bundle {
    let name = name.into();
    (
        Name::new(name.clone()),
        AccessibleLabel::new(name),
        Button,
        TabIndex(0),
        Node {
            width: Val::Px(440.0),
            min_height: Val::Px(48.0),
            padding: UiRect::axes(Val::Px(20.0), Val::Px(12.0)),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(4.0),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(6.0)),
            ..default()
        },
        BorderColor::all(EDGE),
        BackgroundColor(RESTING),
    )
}

/// Standard width of a compact row action.
pub const SMALL_BUTTON_WIDTH: f32 = 144.0;

/// Compact row action.
#[must_use]
pub fn small_button(name: impl Into<String>) -> impl Bundle {
    row_button(name, SMALL_BUTTON_WIDTH)
}

/// Row action with a caller-selected width and shared minimum height.
#[must_use]
pub fn row_button(name: impl Into<String>, width: f32) -> impl Bundle {
    row_button_with_height(name, width, 48.0)
}

/// Row action sized for a label plus wrapped supporting text.
#[must_use]
pub fn stacked_row_button(name: impl Into<String>, width: f32) -> impl Bundle {
    row_button_with_height(name, width, 74.0)
}

fn row_button_with_height(name: impl Into<String>, width: f32, height: f32) -> impl Bundle {
    let name = name.into();
    (
        Name::new(name.clone()),
        AccessibleLabel::new(name),
        Button,
        TabIndex(0),
        Node {
            width: Val::Px(width),
            min_width: Val::Px(44.0),
            height: Val::Px(height),
            min_height: Val::Px(44.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(2.0),
            padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(6.0)),
            ..default()
        },
        BorderColor::all(EDGE),
        BackgroundColor(RESTING),
    )
}

/// Standard dark presentation panel.
#[must_use]
pub fn panel() -> impl Bundle {
    (
        panel_node(),
        BorderColor::all(EDGE),
        BackgroundColor(PANEL_BG),
    )
}

/// Layout portion of [`panel`].
#[must_use]
pub fn panel_node() -> Node {
    Node {
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(12.0),
        padding: UiRect::all(Val::Px(18.0)),
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(10.0)),
        ..default()
    }
}

/// Thin horizontal rule.
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
