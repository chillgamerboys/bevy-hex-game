use bevy::input_focus::tab_navigation::TabIndex;
use bevy::prelude::*;
use hex_assets::ElementCatalog;
use hex_core::ElementId;

use crate::element_visual::authored_element_tint;

// Keep controls visually inside the dark arcane surface instead of rendering
// large translucent grey slabs. Opaque resting paint also makes their contrast
// independent of the world or menu color behind them.
const RESTING: Color = Color::srgba(0.105, 0.115, 0.145, 0.98);
const HOVERED: Color = Color::srgba(0.16, 0.17, 0.20, 0.99);
const PRESSED: Color = Color::srgba(0.25, 0.22, 0.15, 0.99);
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
/// Generic gem tint and missing-element fallback.
pub const GEM_COLOR: Color = Color::srgba(0.16, 0.45, 0.52, 0.92);
/// Generic fusion tint and custom-fusion fallback.
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

#[derive(Component, Clone, Copy, Debug, PartialEq)]
enum SemanticText {
    Display,
    ScreenTitle,
    Heading,
    Body,
    /// Persistent gameplay chrome uses the smallest accessible hierarchy so
    /// world actors remain the dominant visual layer.
    HudHeading,
    HudBody,
    Supporting,
    Metadata,
    /// A redundant label constrained inside a diagram cell.
    ///
    /// These labels intentionally remain below the 18-pixel essential-text floor:
    /// lattice cells and miniature arena tokens expose their complete meaning through
    /// an accessible control label or adjacent body copy. They still grow with the
    /// moderated control scale so the accessibility preference is never ignored.
    CompactGlyph {
        base_size: f32,
    },
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
enum SemanticControl {
    /// Shared target geometry scales from the node's current authored dimensions.
    Responsive { applied_scale: f32 },
    /// The owning renderer resolves a tessellated or otherwise coupled geometry.
    OwnerResolved,
}

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
    /// Hex-cell wordmark used on branded menu surfaces.
    pub logo: Handle<Image>,
    /// Tintable pointy-top hexagon.
    pub hex_cell: Handle<Image>,
}

pub(super) fn plugin(app: &mut App) {
    let asset_server = app.world().resource::<AssetServer>();
    app.insert_resource(UiAssets {
        display: asset_server.load("fonts/Cinzel.ttf"),
        body: asset_server.load("fonts/Inter.ttf"),
        logo: asset_server.load("ui/hex-logo.png"),
        hex_cell: asset_server.load("ui/hex-cell.png"),
    });
    app.add_systems(Update, paint_interactions).add_systems(
        PostUpdate,
        apply_semantic_metrics
            .in_set(crate::scale::SemanticMetricsSystems::Apply)
            .before(bevy::ui::UiSystems::Prepare),
    );
}

pub(crate) fn brand_logo(assets: &UiAssets, width: f32) -> impl Bundle {
    (
        Name::new("Hex Logo"),
        AccessibleLabel::new("Hex"),
        ImageNode::new(assets.logo.clone()),
        Node {
            width: Val::Px(width),
            max_width: Val::Percent(92.0),
            aspect_ratio: Some(1290.0 / 480.0),
            flex_shrink: 1.0,
            ..default()
        },
        Pickable::IGNORE,
        // Bevy derives an image's accessibility label from descendant text in
        // PostUpdate. Keep a non-rendered name here so that sync cannot clear the
        // explicit label above after the image is first inserted.
        children![(
            Text::new("Hex"),
            Node {
                display: Display::None,
                ..default()
            },
        )],
    )
}

/// Resolves an element's presentation tint.
///
/// Canonical elements use the authored visual catalog shared with their icons. A
/// custom basic element falls back to a deterministic wheel hue, a custom fusion to
/// [`FUSION_COLOR`], and an absent element to [`GEM_COLOR`].
#[must_use]
pub fn element_color(element: Option<ElementId>, elements: &ElementCatalog) -> Color {
    let Some(id) = element else { return GEM_COLOR };
    if let Some(tint) = elements.name(id).and_then(authored_element_tint) {
        return tint;
    }
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
    text_bundle(
        assets.display.clone(),
        text,
        DISPLAY_SIZE,
        LABEL,
        SemanticText::Display,
    )
}

/// Top-level screen title.
#[must_use]
pub fn screen_title(assets: &UiAssets, text: impl Into<String>) -> impl Bundle {
    text_bundle(
        assets.display.clone(),
        text,
        SCREEN_TITLE_SIZE,
        LABEL,
        SemanticText::ScreenTitle,
    )
}

/// Section heading.
#[must_use]
pub fn heading(assets: &UiAssets, text: impl Into<String>) -> impl Bundle {
    text_bundle(
        assets.display.clone(),
        text,
        TITLE_SIZE,
        ACCENT,
        SemanticText::Heading,
    )
}

/// Compact section heading for persistent gameplay chrome.
#[must_use]
pub(crate) fn hud_heading(assets: &UiAssets, text: impl Into<String>) -> impl Bundle {
    text_bundle(
        assets.display.clone(),
        text,
        20.0,
        ACCENT,
        SemanticText::HudHeading,
    )
}

/// Essential body or control text.
#[must_use]
pub fn label(assets: &UiAssets, text: impl Into<String>) -> impl Bundle {
    text_bundle(
        assets.body.clone(),
        text,
        LABEL_SIZE,
        LABEL,
        SemanticText::Body,
    )
}

/// Supporting text.
#[must_use]
pub fn blurb(assets: &UiAssets, text: impl Into<String>) -> impl Bundle {
    text_bundle(
        assets.body.clone(),
        text,
        BLURB_SIZE,
        MUTED,
        SemanticText::Supporting,
    )
}

/// Optional metadata text.
#[must_use]
pub fn fine(assets: &UiAssets, text: impl Into<String>) -> impl Bundle {
    text_bundle(
        assets.body.clone(),
        text,
        FINE_SIZE,
        MUTED,
        SemanticText::Metadata,
    )
}

fn text_bundle(
    font: Handle<Font>,
    text: impl Into<String>,
    size: f32,
    color: Color,
    role: SemanticText,
) -> impl Bundle {
    (
        role,
        Text::new(text),
        TextFont {
            font: font.into(),
            ..TextFont::from_font_size(size)
        },
        TextColor(color),
        Pickable::IGNORE,
    )
}

/// Applies essential body typography to a custom text entity.
#[must_use]
pub(crate) const fn body_text_role() -> impl Bundle {
    SemanticText::Body
}

/// Applies compact essential typography to persistent gameplay chrome.
#[must_use]
pub(crate) const fn hud_text_role() -> impl Bundle {
    SemanticText::HudBody
}

/// Applies supporting typography to a custom text entity.
#[must_use]
pub(crate) const fn supporting_text_role() -> impl Bundle {
    SemanticText::Supporting
}

/// Marks a compact diagram glyph that is redundant with an accessible label or copy.
#[must_use]
pub(crate) const fn compact_glyph_role(base_size: f32) -> impl Bundle {
    SemanticText::CompactGlyph { base_size }
}

/// Opts custom control geometry into moderated semantic target scaling.
#[must_use]
pub(crate) const fn responsive_control_role() -> impl Bundle {
    SemanticControl::Responsive { applied_scale: 1.0 }
}

/// Marks control geometry whose owner already scales the coupled layout as a unit.
#[must_use]
pub(crate) const fn owner_resolved_control_role() -> impl Bundle {
    SemanticControl::OwnerResolved
}

/// Standard menu action.
#[must_use]
pub fn button(name: impl Into<String>) -> impl Bundle {
    button_with_width(name, Val::Px(440.0), Val::Auto)
}

/// Menu action that fills a constrained parent such as a campaign card.
#[must_use]
pub(crate) fn fluid_button(name: impl Into<String>) -> impl Bundle {
    // `Percent(100)` resolves against a fixed-width parent's authored content width,
    // then adds this control's own padding. Let flex stretch own the final width so
    // the complete bordered target remains inside padded cards.
    button_with_width(name, Val::Auto, Val::Auto)
}

fn button_with_width(name: impl Into<String>, width: Val, max_width: Val) -> impl Bundle {
    let name = name.into();
    (
        Name::new(name.clone()),
        AccessibleLabel::new(name),
        Button,
        TabIndex(0),
        crate::DefaultImmediateControl,
        responsive_control_role(),
        Node {
            width,
            max_width,
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

/// Compact control whose exact dimensions are already resolved by its owner.
///
/// Unlike semantic row buttons, this does not scale its box a second time.
pub(crate) fn fixed_row_button(name: impl Into<String>, width: f32, height: f32) -> impl Bundle {
    let name = name.into();
    (
        Name::new(name.clone()),
        AccessibleLabel::new(name),
        Button,
        TabIndex(0),
        crate::DefaultImmediateControl,
        owner_resolved_control_role(),
        Node {
            width: Val::Px(width),
            min_width: Val::Px(44.0),
            height: Val::Px(height),
            min_height: Val::Px(44.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(2.0),
            padding: UiRect::axes(Val::Px(4.0), Val::Px(2.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(6.0)),
            ..default()
        },
        BorderColor::all(EDGE),
        BackgroundColor(RESTING),
    )
}

fn row_button_with_height(name: impl Into<String>, width: f32, height: f32) -> impl Bundle {
    let name = name.into();
    (
        Name::new(name.clone()),
        AccessibleLabel::new(name),
        Button,
        TabIndex(0),
        crate::DefaultImmediateControl,
        responsive_control_role(),
        Node {
            // The requested width is the compact baseline, not a hard text
            // clipping boundary. Semantic type can grow independently of
            // control spacing, so the box must be allowed to grow with its
            // label while flex parents retain responsibility for wrapping.
            width: Val::Auto,
            min_width: Val::Px(width.max(44.0)),
            height: Val::Auto,
            min_height: Val::Px(height.max(44.0)),
            flex_shrink: 0.0,
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

fn apply_semantic_metrics(
    metrics: Res<crate::ResolvedUiMetrics>,
    mut text: Query<(Ref<SemanticText>, Mut<TextFont>)>,
    mut controls: Query<(Mut<Node>, Mut<SemanticControl>)>,
) {
    let metrics_changed = metrics.is_changed();
    for (role, mut font) in &mut text {
        if !metrics_changed && !role.is_changed() && !font.is_changed() {
            continue;
        }
        let size = match *role {
            SemanticText::Display => DISPLAY_SIZE * metrics.heading_scale,
            SemanticText::ScreenTitle => SCREEN_TITLE_SIZE * metrics.heading_scale,
            SemanticText::Heading => TITLE_SIZE * metrics.heading_scale,
            SemanticText::Body => (LABEL_SIZE * metrics.content_scale).max(18.0),
            SemanticText::HudHeading => (20.0 * metrics.heading_scale).max(18.0),
            SemanticText::HudBody => (18.0 * metrics.content_scale).max(18.0),
            SemanticText::Supporting => (BLURB_SIZE * metrics.content_scale).max(18.0),
            SemanticText::Metadata => FINE_SIZE * metrics.content_scale,
            SemanticText::CompactGlyph { base_size } => base_size * metrics.control_scale.max(1.0),
        };
        let wanted = FontSize::Px(size);
        if font.font_size != wanted {
            font.font_size = wanted;
        }
    }
    let next_scale = metrics.control_scale.max(1.0);
    for (mut node, mut control) in &mut controls {
        if !metrics_changed && !control.is_changed() && !node.is_changed() {
            continue;
        }
        let SemanticControl::Responsive { applied_scale } = *control else {
            continue;
        };
        let previous_scale = applied_scale.max(1.0);
        let ratio = next_scale / previous_scale;
        let min_width = match node.min_width {
            Val::Px(current) => Val::Px((current * ratio).max(44.0 * next_scale)),
            other => other,
        };
        let min_height = match node.min_height {
            Val::Px(current) => Val::Px((current * ratio).max(44.0 * next_scale)),
            _ => Val::Px(44.0 * next_scale),
        };
        if node.min_width != min_width {
            node.min_width = min_width;
        }
        if node.min_height != min_height {
            node.min_height = min_height;
        }
        if let Val::Px(current) = node.height {
            let height = Val::Px(current * ratio);
            if node.height != height {
                node.height = height;
            }
        }
        if let SemanticControl::Responsive { applied_scale } = &mut *control {
            if (*applied_scale - next_scale).abs() > f32::EPSILON {
                *applied_scale = next_scale;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(mode: crate::UiScaleMode) -> crate::ResolvedUiMetrics {
        crate::resolve_ui_metrics(Vec2::new(1920.0, 1080.0), mode)
    }

    #[test]
    fn semantic_metrics_resize_actual_custom_text_and_node_components() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(metrics(crate::UiScaleMode::Percent100))
            .add_systems(Update, apply_semantic_metrics);
        let text = app
            .world_mut()
            .spawn((body_text_role(), TextFont::from_font_size(18.0)))
            .id();
        let control = app
            .world_mut()
            .spawn((
                responsive_control_role(),
                Node {
                    min_height: Val::Px(58.0),
                    ..default()
                },
            ))
            .id();

        app.update();
        assert_eq!(
            app.world().get::<TextFont>(text).map(|font| font.font_size),
            Some(FontSize::Px(20.0))
        );
        assert_eq!(
            app.world().get::<Node>(control).map(|node| node.min_height),
            Some(Val::Px(58.0))
        );

        *app.world_mut().resource_mut::<crate::ResolvedUiMetrics>() =
            metrics(crate::UiScaleMode::Percent200);
        app.update();
        assert_eq!(
            app.world().get::<TextFont>(text).map(|font| font.font_size),
            Some(FontSize::Px(40.0))
        );
        assert_eq!(
            app.world().get::<Node>(control).map(|node| node.min_height),
            Some(Val::Px(87.0))
        );
    }

    #[test]
    fn widgets_added_after_a_scale_change_receive_current_metrics() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(metrics(crate::UiScaleMode::Percent200))
            .add_systems(Update, apply_semantic_metrics);
        app.update();

        let text = app
            .world_mut()
            .spawn((body_text_role(), TextFont::from_font_size(18.0)))
            .id();
        let control = app
            .world_mut()
            .spawn((
                responsive_control_role(),
                Node {
                    min_height: Val::Px(44.0),
                    ..default()
                },
            ))
            .id();
        app.update();

        assert_eq!(
            app.world().get::<TextFont>(text).map(|font| font.font_size),
            Some(FontSize::Px(40.0))
        );
        assert_eq!(
            app.world().get::<Node>(control).map(|node| node.min_height),
            Some(Val::Px(66.0))
        );
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn brand_logo_keeps_its_spoken_name_after_image_accessibility_sync() {
        let mut app = App::new();
        app.add_plugins(crate::test_support::HeadlessUiPlugin::new(1280, 720));
        let assets = app.world().resource::<UiAssets>().clone();
        let logo = app.world_mut().spawn(brand_logo(&assets, 420.0)).id();

        for _ in 0..4 {
            app.update();
        }

        let accessibility = app
            .world()
            .get::<bevy::a11y::AccessibilityNode>(logo)
            .expect("the logo image must participate in the accessibility tree");
        assert_eq!(accessibility.label(), Some("Hex"));
    }

    #[derive(Resource, Default)]
    struct SemanticWriteCount(usize);

    fn count_semantic_writes(
        fonts: Query<(), (Changed<TextFont>, With<SemanticText>)>,
        nodes: Query<(), (Changed<Node>, With<SemanticControl>)>,
        mut count: ResMut<SemanticWriteCount>,
    ) {
        let writes = fonts.iter().count() + nodes.iter().count();
        if writes > 0 {
            count.0 += writes;
        }
    }

    #[test]
    fn ten_thousand_stable_updates_do_not_rewrite_semantic_components() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(metrics(crate::UiScaleMode::Percent200))
            .init_resource::<SemanticWriteCount>()
            .add_systems(
                Update,
                (
                    apply_semantic_metrics,
                    count_semantic_writes.after(apply_semantic_metrics),
                ),
            );
        app.world_mut()
            .spawn((body_text_role(), TextFont::from_font_size(18.0)));
        app.world_mut().spawn((
            responsive_control_role(),
            Node {
                min_height: Val::Px(48.0),
                ..default()
            },
        ));
        app.update();
        app.world_mut().resource_mut::<SemanticWriteCount>().0 = 0;

        for _ in 0..10_000 {
            app.update();
        }

        assert_eq!(app.world().resource::<SemanticWriteCount>().0, 0);
    }

    #[cfg(feature = "test-support")]
    #[derive(Resource, Default)]
    struct RenderSpawnedWidget {
        control: Option<Entity>,
        text: Option<Entity>,
    }

    #[cfg(feature = "test-support")]
    fn spawn_semantic_widget_during_render(
        mut commands: Commands,
        mut spawned: ResMut<RenderSpawnedWidget>,
    ) {
        if spawned.control.is_some() {
            return;
        }
        let text = commands
            .spawn((
                Text::new("Same-frame action"),
                body_text_role(),
                TextFont::from_font_size(18.0),
            ))
            .id();
        let control = commands
            .spawn((
                Name::new("Same-frame semantic control"),
                Button,
                responsive_control_role(),
                Node {
                    width: Val::Px(240.0),
                    min_height: Val::Px(48.0),
                    ..default()
                },
            ))
            .add_child(text)
            .id();
        spawned.control = Some(control);
        spawned.text = Some(text);
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn update_spawned_widgets_scale_before_their_first_layout() {
        let mut app = App::new();
        app.add_plugins(crate::test_support::HeadlessUiPlugin::new(1920, 1080))
            .insert_resource(crate::UiScalePreference(crate::UiScaleMode::Percent200))
            .init_resource::<RenderSpawnedWidget>()
            .add_systems(
                Update,
                spawn_semantic_widget_during_render.in_set(crate::UiSystems::Render),
            );

        app.update();

        let spawned = app.world().resource::<RenderSpawnedWidget>();
        let control = spawned
            .control
            .expect("render system must spawn its control");
        let text = spawned.text.expect("render system must spawn its text");
        assert_eq!(
            app.world().get::<TextFont>(text).map(|font| font.font_size),
            Some(FontSize::Px(40.0))
        );
        assert_eq!(
            app.world().get::<Node>(control).map(|node| node.min_height),
            Some(Val::Px(72.0))
        );
        assert!(
            app.world()
                .get::<ComputedNode>(control)
                .is_some_and(|node| node.size().y >= 72.0),
            "the first computed layout must use the scaled control geometry"
        );
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn semantic_scale_changes_a_custom_controls_computed_size() {
        let mut app = App::new();
        app.add_plugins(crate::test_support::HeadlessUiPlugin::new(1920, 1080));
        app.world_mut()
            .insert_resource(crate::UiScalePreference(crate::UiScaleMode::Percent100));
        let control = app
            .world_mut()
            .spawn((
                Name::new("Representative Custom Control"),
                Button,
                responsive_control_role(),
                Node {
                    width: Val::Px(240.0),
                    min_height: Val::Px(48.0),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
            ))
            .id();
        let text = app
            .world_mut()
            .spawn((
                Text::new("Custom action"),
                body_text_role(),
                TextFont::from_font_size(18.0),
            ))
            .id();
        app.world_mut().entity_mut(control).add_child(text);

        for _ in 0..4 {
            app.update();
        }
        let baseline = app
            .world()
            .get::<ComputedNode>(control)
            .map(ComputedNode::size)
            .expect("custom control must participate in UI layout");
        assert_eq!(
            app.world().get::<TextFont>(text).map(|font| font.font_size),
            Some(FontSize::Px(20.0))
        );

        app.world_mut()
            .insert_resource(crate::UiScalePreference(crate::UiScaleMode::Percent200));
        for _ in 0..4 {
            app.update();
        }
        let enlarged = app
            .world()
            .get::<ComputedNode>(control)
            .map(ComputedNode::size)
            .expect("custom control must remain in UI layout");
        assert_eq!(
            app.world().get::<Node>(control).map(|node| node.min_height),
            Some(Val::Px(72.0))
        );
        assert_eq!(
            app.world().get::<TextFont>(text).map(|font| font.font_size),
            Some(FontSize::Px(40.0))
        );
        assert!(
            enlarged.y > baseline.y,
            "computed control height must grow: {baseline:?} -> {enlarged:?}"
        );
    }
}
