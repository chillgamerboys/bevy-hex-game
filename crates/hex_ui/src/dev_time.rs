//! Development-only local presentation-time preview controls.

use bevy::input_focus::tab_navigation::TabIndex;
use bevy::prelude::*;
use bevy::ui_widgets::{
    Slider, SliderDragState, SliderPrecision, SliderRange, SliderStep, SliderThumb, SliderValue,
    TrackClick, ValueChange,
};
use hex_core::{AppSystems, Screen};

use crate::{
    blurb, fine, heading, layout::is_ultra_constrained, panel, theme::fixed_row_button,
    DevTimeIntent, DevTimeView, GameplayChromeView, HudElement, ResolvedUiMetrics, UiAssets,
    UiHudSetup, UiIntent, UiRegionRole, UiSystems, UiViewportClass, UiVisibilityRequirement,
    ACCENT, ACCENT_EDGE, EDGE,
};

const PRESET_CONTROLS: [(&str, &str, DevTimeIntent); 7] = [
    (
        "Dev Time Midnight",
        "Midnight",
        DevTimeIntent::SetPreviewHours(0.0),
    ),
    ("Dev Time Dawn", "Dawn", DevTimeIntent::SetPreviewHours(6.5)),
    (
        "Dev Time Noon",
        "Noon",
        DevTimeIntent::SetPreviewHours(12.0),
    ),
    (
        "Dev Time Golden Hour",
        "Golden Hour",
        DevTimeIntent::SetPreviewHours(16.5),
    ),
    (
        "Dev Time Sunset",
        "Sunset",
        DevTimeIntent::SetPreviewHours(18.5),
    ),
    (
        "Dev Time Night",
        "Night",
        DevTimeIntent::SetPreviewHours(20.0),
    ),
    ("Dev Time Reset", "Reset", DevTimeIntent::ResetPreview),
];

const SLIDER_MIN_HOURS: f32 = 0.0;
const SLIDER_MAX_HOURS: f32 = 23.75;
const SLIDER_STEP_HOURS: f32 = 0.25;
const SLIDER_THUMB_SIZE: f32 = 20.0;
const SLIDER_TRACK: Color = Color::srgba(1.0, 1.0, 1.0, 0.16);

#[derive(Component)]
struct DevTimePanel;

#[derive(Component)]
struct DevTimeHeading;

#[derive(Component)]
struct DevTimeStatus;

#[derive(Component, Default)]
struct DevTimeControls {
    available: bool,
}

#[derive(Component)]
struct DevTimePresets;

#[derive(Component)]
struct DevTimeSlider;

#[derive(Component)]
struct DevTimeSliderThumb;

#[derive(Component, Debug, Clone, Copy, PartialEq)]
struct DevTimeControl(DevTimeIntent);

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        OnEnter(Screen::Gameplay),
        spawn_panel.in_set(UiHudSetup::Tooling),
    )
    .add_systems(
        Update,
        (rebuild, reconcile_layout, update_slider_style)
            .chain()
            .in_set(UiSystems::Render)
            .run_if(in_state(Screen::Gameplay)),
    )
    .add_systems(
        Update,
        (
            consume_focused_slider_navigation
                .in_set(UiSystems::CaptureInput)
                .before(AppSystems::RecordInput),
            emit_preset_intents.in_set(UiSystems::EmitIntents),
        )
            .run_if(in_state(Screen::Gameplay)),
    )
    .add_observer(emit_slider_intent);
}

/// Keeps navigation owned by the focused native slider out of gameplay bindings.
///
/// Bevy's focused-input observer has already translated these raw key events into
/// `ValueChange` by `Update`. Resetting only the keys handled by a focused slider
/// here prevents a user-rebound gameplay or camera action from observing the same
/// press later in [`AppSystems::RecordInput`]. Unfocused input is untouched.
fn consume_focused_slider_navigation(
    focus: Res<bevy::input_focus::InputFocus>,
    sliders: Query<(), With<DevTimeSlider>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
) {
    if focus.get().is_none_or(|entity| !sliders.contains(entity)) {
        return;
    }
    for key in [
        KeyCode::ArrowLeft,
        KeyCode::ArrowRight,
        KeyCode::Home,
        KeyCode::End,
    ] {
        keys.reset(key);
    }
}

fn spawn_panel(
    mut commands: Commands,
    assets: Res<UiAssets>,
    metrics: Res<ResolvedUiMetrics>,
    chrome: Res<GameplayChromeView>,
    review: Option<Res<crate::review::UiReviewPresentation>>,
    regions: Query<(Entity, &UiRegionRole)>,
) {
    let Some(inspector) = regions
        .iter()
        .find_map(|(entity, role)| (*role == UiRegionRole::Inspector).then_some(entity))
    else {
        return;
    };
    let chrome = review.as_ref().map_or(*chrome, |review| {
        review.effective_chrome(*chrome, metrics.viewport)
    });
    let panel = commands
        .spawn((
            Name::new("Dev Time Panel"),
            DevTimePanel,
            HudElement,
            panel(),
            Pickable::IGNORE,
            GlobalZIndex(3),
        ))
        .insert(panel_node(*metrics, chrome.decision_required()))
        .with_children(|panel| {
            panel.spawn((DevTimeHeading, heading(&assets, "LOCAL VISUAL PREVIEW")));
            panel.spawn((
                DevTimeStatus,
                Node {
                    width: Val::Percent(100.0),
                    ..default()
                },
                blurb(&assets, "Checking cyclic time…"),
            ));
            panel.spawn((
                Name::new("Dev Time Controls"),
                DevTimeControls::default(),
                controls_node(*metrics),
                Pickable::IGNORE,
            ));
        })
        .id();
    commands.entity(inspector).add_child(panel);
}

fn panel_is_collapsed(metrics: ResolvedUiMetrics, decision_required: bool) -> bool {
    (is_ultra_constrained(metrics) && decision_required) || metrics.effective_size.y < 300.0
}

fn presets_are_hidden(metrics: ResolvedUiMetrics, decision_required: bool) -> bool {
    is_ultra_constrained(metrics)
        || (metrics.viewport == UiViewportClass::Compact && decision_required)
}

fn panel_node(metrics: ResolvedUiMetrics, decision_required: bool) -> Node {
    let mut node = Node {
        width: Val::Percent(100.0),
        flex_shrink: 0.0,
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(8.0),
        padding: UiRect::all(Val::Px(12.0)),
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(10.0)),
        ..default()
    };
    if metrics.viewport == UiViewportClass::Compact {
        node.row_gap = Val::Px(4.0);
        if is_ultra_constrained(metrics) {
            node.row_gap = Val::Px(2.0);
            node.padding = UiRect::all(Val::Px(4.0));
        } else {
            node.padding = UiRect::all(Val::Px(6.0));
        }
    }
    // Keep the scrubber available after presets collapse. Only hide the entire
    // secondary surface when even the essential 44px track cannot coexist with
    // a required gameplay decision or the effective canvas is exceptionally short.
    if panel_is_collapsed(metrics, decision_required) {
        node.display = Display::None;
    }
    node
}

fn controls_node(metrics: ResolvedUiMetrics) -> Node {
    let gap = if is_ultra_constrained(metrics) {
        2.0
    } else if metrics.viewport == UiViewportClass::Compact {
        4.0
    } else {
        6.0
    };
    Node {
        width: Val::Percent(100.0),
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(gap),
        ..default()
    }
}

fn presets_node(metrics: ResolvedUiMetrics, decision_required: bool) -> Node {
    let mut node = controls_node(metrics);
    node.flex_direction = FlexDirection::Row;
    node.flex_wrap = FlexWrap::Wrap;
    node.column_gap = node.row_gap;
    if presets_are_hidden(metrics, decision_required) {
        node.display = Display::None;
    }
    node
}

fn slider_node() -> Node {
    Node {
        width: Val::Percent(100.0),
        min_width: Val::Px(44.0),
        height: Val::Px(44.0),
        min_height: Val::Px(44.0),
        position_type: PositionType::Relative,
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Stretch,
        padding: UiRect::axes(Val::Px(4.0), Val::Px(0.0)),
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(6.0)),
        ..default()
    }
}

fn spawn_slider(parent: &mut ChildSpawnerCommands, hours: f32) {
    parent
        .spawn((
            Name::new("Dev Time Slider"),
            DevTimeSlider,
            Slider {
                track_click: TrackClick::Snap,
                ..default()
            },
            SliderValue(hours),
            SliderRange::new(SLIDER_MIN_HOURS, SLIDER_MAX_HOURS),
            SliderStep(SLIDER_STEP_HOURS),
            SliderPrecision(2),
            TabIndex(0),
            UiVisibilityRequirement::Scrollable,
            slider_node(),
            BorderColor::all(ACCENT_EDGE),
            BackgroundColor(Color::srgba(0.105, 0.115, 0.145, 0.98)),
        ))
        .insert(AccessibleLabel::new("Local visual preview time"))
        .with_children(|slider| {
            slider.spawn((
                Name::new("Dev Time Slider Track"),
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(6.0),
                    border_radius: BorderRadius::all(Val::Px(3.0)),
                    ..default()
                },
                BackgroundColor(SLIDER_TRACK),
                Pickable::IGNORE,
            ));
            slider
                .spawn((
                    Name::new("Dev Time Slider Thumb Lane"),
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(4.0),
                        right: Val::Px(SLIDER_THUMB_SIZE + 4.0),
                        top: Val::Px(0.0),
                        bottom: Val::Px(0.0),
                        ..default()
                    },
                    Pickable::IGNORE,
                ))
                .with_child((
                    Name::new("Dev Time Slider Thumb"),
                    DevTimeSliderThumb,
                    SliderThumb,
                    Node {
                        width: Val::Px(SLIDER_THUMB_SIZE),
                        height: Val::Px(SLIDER_THUMB_SIZE),
                        position_type: PositionType::Absolute,
                        left: Val::Percent(0.0),
                        top: Val::Px((44.0 - SLIDER_THUMB_SIZE) * 0.5),
                        border: UiRect::all(Val::Px(2.0)),
                        border_radius: BorderRadius::MAX,
                        ..default()
                    },
                    BorderColor::all(EDGE),
                    BackgroundColor(ACCENT),
                ));
        });
}

fn spawn_presets(parent: &mut ChildSpawnerCommands, assets: &UiAssets, metrics: ResolvedUiMetrics) {
    parent
        .spawn((
            Name::new("Dev Time Presets"),
            DevTimePresets,
            presets_node(metrics, false),
            Pickable::IGNORE,
        ))
        .with_children(|presets| {
            let (width, height) = control_size(metrics);
            for (name, label, intent) in PRESET_CONTROLS {
                presets
                    .spawn((
                        fixed_row_button(name, width, height),
                        UiVisibilityRequirement::Scrollable,
                        DevTimeControl(intent),
                    ))
                    .with_child(fine(assets, label));
            }
        });
}

fn reconcile_layout(
    metrics: Res<ResolvedUiMetrics>,
    chrome: Res<GameplayChromeView>,
    review: Option<Res<crate::review::UiReviewPresentation>>,
    added_panels: Query<(), Added<DevTimePanel>>,
    added_roots: Query<(), Added<DevTimeControls>>,
    added_presets: Query<(), Added<DevTimePresets>>,
    added_sliders: Query<(), Added<DevTimeSlider>>,
    added_controls: Query<(), Added<DevTimeControl>>,
    mut nodes: ParamSet<(
        Query<&mut Node, With<DevTimePanel>>,
        Query<&mut Node, With<DevTimeControls>>,
        Query<&mut Node, With<DevTimeHeading>>,
        Query<&mut Node, With<DevTimePresets>>,
        Query<&mut Node, With<DevTimeControl>>,
        Query<&mut Node, With<DevTimeSlider>>,
    )>,
) {
    if !metrics.is_changed()
        && !chrome.is_changed()
        && review.as_ref().is_none_or(|review| !review.is_changed())
        && added_panels.is_empty()
        && added_roots.is_empty()
        && added_presets.is_empty()
        && added_sliders.is_empty()
        && added_controls.is_empty()
    {
        return;
    }

    let chrome = review.as_ref().map_or(*chrome, |review| {
        review.effective_chrome(*chrome, metrics.viewport)
    });
    if let Ok(mut node) = nodes.p0().single_mut() {
        *node = panel_node(*metrics, chrome.decision_required());
    }
    if let Ok(mut node) = nodes.p1().single_mut() {
        *node = controls_node(*metrics);
    }
    if let Ok(mut node) = nodes.p2().single_mut() {
        node.display = if metrics.viewport == UiViewportClass::Compact {
            Display::None
        } else {
            Display::Flex
        };
    }
    if let Ok(mut node) = nodes.p3().single_mut() {
        *node = presets_node(*metrics, chrome.decision_required());
    }
    let (width, height) = control_size(*metrics);
    for mut node in &mut nodes.p4() {
        node.width = Val::Px(width);
        node.height = Val::Px(height);
        node.padding = if is_ultra_constrained(*metrics) {
            UiRect::axes(Val::Px(3.0), Val::Px(1.0))
        } else if metrics.viewport == UiViewportClass::Compact {
            UiRect::axes(Val::Px(4.0), Val::Px(2.0))
        } else {
            UiRect::axes(Val::Px(10.0), Val::Px(4.0))
        };
    }
    if let Ok(mut node) = nodes.p5().single_mut() {
        *node = slider_node();
    }
}

fn control_size(metrics: ResolvedUiMetrics) -> (f32, f32) {
    if is_ultra_constrained(metrics) && metrics.content_scale >= 1.5 {
        (172.0, 44.0)
    } else if is_ultra_constrained(metrics) {
        (82.0, 44.0)
    } else if metrics.viewport == UiViewportClass::Compact {
        (76.0, 44.0)
    } else {
        (96.0, 48.0)
    }
}

fn rebuild(
    mut commands: Commands,
    view: Res<DevTimeView>,
    mut statuses: Query<(&mut Text, Ref<DevTimeStatus>)>,
    mut controls: Query<(Entity, &mut DevTimeControls)>,
    sliders: Query<(Entity, &SliderValue), With<DevTimeSlider>>,
    assets: Res<UiAssets>,
    metrics: Res<ResolvedUiMetrics>,
) {
    let Ok((mut status, status_marker)) = statuses.single_mut() else {
        return;
    };
    let Ok((controls_entity, mut controls)) = controls.single_mut() else {
        return;
    };
    let controls_added = controls.is_added();
    if !view.is_changed() && !metrics.is_changed() && !status_marker.is_added() && !controls_added {
        return;
    }

    let available = matches!(view.as_ref(), DevTimeView::Available { .. });
    if controls_added || controls.available != available {
        commands
            .entity(controls_entity)
            .despawn_related::<Children>();
        if let DevTimeView::Available {
            game_hours,
            preview_hours,
        } = view.as_ref()
        {
            let hours = slider_display_hours(preview_hours.unwrap_or(*game_hours));
            commands.entity(controls_entity).with_children(|controls| {
                spawn_slider(controls, hours);
                spawn_presets(controls, &assets, *metrics);
            });
        }
        controls.available = available;
    }

    match view.as_ref() {
        DevTimeView::Available {
            game_hours,
            preview_hours,
        } => {
            let displayed = slider_display_hours(preview_hours.unwrap_or(*game_hours));
            if let Ok((entity, value)) = sliders.single() {
                if value.0.to_bits() != displayed.to_bits() {
                    commands.entity(entity).insert(SliderValue(displayed));
                }
            }
            **status = match preview_hours {
                Some(hours) => format!(
                    "LOCAL VISUAL PREVIEW · {}\nGAME · {}",
                    format_clock(*hours),
                    format_clock(*game_hours)
                ),
                None => format!(
                    "LOCAL VISUAL PREVIEW · OFF\nGAME · {}",
                    format_clock(*game_hours)
                ),
            };
        }
        DevTimeView::Unavailable { reason } => {
            **status = format!("LOCAL VISUAL PREVIEW · UNAVAILABLE\n{reason}");
        }
    }
}

fn slider_display_hours(hours: f32) -> f32 {
    hours.clamp(SLIDER_MIN_HOURS, SLIDER_MAX_HOURS)
}

fn quantize_slider_hours(hours: f32) -> f32 {
    let bounded = slider_display_hours(hours);
    ((bounded / SLIDER_STEP_HOURS).round() * SLIDER_STEP_HOURS)
        .clamp(SLIDER_MIN_HOURS, SLIDER_MAX_HOURS)
}

fn format_clock(hours: f32) -> String {
    let total_minutes = (hours.rem_euclid(24.0) * 60.0)
        .round()
        .rem_euclid(24.0 * 60.0);
    let clock_hours = (total_minutes / 60.0).floor();
    let clock_minutes = total_minutes.rem_euclid(60.0);
    format!("{clock_hours:02.0}:{clock_minutes:02.0}")
}

fn emit_preset_intents(
    view: Res<DevTimeView>,
    controls: Query<(&Interaction, &DevTimeControl), Changed<Interaction>>,
    mut intents: MessageWriter<UiIntent>,
) {
    if !matches!(view.as_ref(), DevTimeView::Available { .. }) {
        return;
    }
    for (interaction, control) in &controls {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::DevTime(control.0));
        }
    }
}

fn emit_slider_intent(
    value_change: On<ValueChange<f32>>,
    view: Res<DevTimeView>,
    sliders: Query<(), With<DevTimeSlider>>,
    mut intents: MessageWriter<UiIntent>,
) {
    if sliders.contains(value_change.source)
        && matches!(view.as_ref(), DevTimeView::Available { .. })
        && value_change.value.is_finite()
    {
        intents.write(UiIntent::DevTime(DevTimeIntent::SetPreviewHours(
            quantize_slider_hours(value_change.value),
        )));
    }
}

fn update_slider_style(
    sliders: Query<
        (Entity, &SliderValue, &SliderRange),
        (
            With<DevTimeSlider>,
            Or<(
                Changed<SliderValue>,
                Changed<SliderRange>,
                Changed<SliderDragState>,
            )>,
        ),
    >,
    children: Query<&Children>,
    mut thumbs: Query<&mut Node, (With<DevTimeSliderThumb>, Without<DevTimeSlider>)>,
) {
    for (slider, value, range) in &sliders {
        for descendant in children.iter_descendants(slider) {
            if let Ok(mut node) = thumbs.get_mut(descendant) {
                node.left = Val::Percent(range.thumb_position(value.0) * 100.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::SystemState;
    #[cfg(feature = "test-support")]
    use bevy::input::keyboard::{Key, KeyCode, KeyboardInput};
    #[cfg(feature = "test-support")]
    use bevy::input::ButtonState;
    use bevy::input_focus::tab_navigation::{NavAction, TabGroup, TabIndex, TabNavigation};
    use bevy::input_focus::InputFocus;
    #[cfg(feature = "test-support")]
    use bevy::input_focus::{FocusCause, InputFocusVisible};
    #[cfg(feature = "test-support")]
    use bevy::picking::{
        backend::HitData,
        events::{Drag, DragEnd, DragStart, Pointer},
        pointer::{Location, PointerButton, PointerId},
    };
    #[cfg(feature = "test-support")]
    use bevy::window::PrimaryWindow;

    #[cfg(feature = "test-support")]
    #[derive(Resource, Default)]
    struct RequiredChoiceTransition(bool);

    #[cfg(feature = "test-support")]
    fn character_main_view_chrome() -> GameplayChromeView {
        GameplayChromeView {
            party_shown: false,
            initiative_shown: false,
            activity_shown: false,
            action_bar_shown: false,
            main_view: hex_gameplay_model::MainViewDestination::Character(hex_core::UnitId(0)),
            terrain_health_shown: true,
            encounter_complete: false,
        }
    }

    #[cfg(feature = "test-support")]
    fn activate_required_choice_for_render(
        mut transition: ResMut<RequiredChoiceTransition>,
        mut hud: ResMut<crate::GameplayHudView>,
        mut chrome: ResMut<GameplayChromeView>,
    ) {
        if !transition.0 {
            return;
        }
        transition.0 = false;
        *hud = required_hud();
        *chrome = GameplayChromeView {
            party_shown: false,
            initiative_shown: false,
            activity_shown: false,
            action_bar_shown: false,
            main_view: hex_gameplay_model::MainViewDestination::RequiredDecision,
            terrain_health_shown: false,
            encounter_complete: false,
        };
    }

    #[derive(Resource, Default)]
    struct Received(Vec<DevTimeIntent>);

    #[cfg(feature = "test-support")]
    #[derive(Resource, Default)]
    struct GameplayNavigationObserved(bool);

    fn receive(mut intents: MessageReader<UiIntent>, mut received: ResMut<Received>) {
        for intent in intents.read() {
            if let UiIntent::DevTime(intent) = intent {
                received.0.push(*intent);
            }
        }
    }

    fn assert_hours_eq(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= f32::EPSILON,
            "expected {expected} hours, got {actual}"
        );
    }

    #[cfg(feature = "test-support")]
    fn observe_gameplay_navigation_keys(
        keys: Res<ButtonInput<KeyCode>>,
        mut observed: ResMut<GameplayNavigationObserved>,
    ) {
        observed.0 |= keys.any_pressed([
            KeyCode::ArrowLeft,
            KeyCode::ArrowRight,
            KeyCode::Home,
            KeyCode::End,
        ]) || keys.any_just_pressed([
            KeyCode::ArrowLeft,
            KeyCode::ArrowRight,
            KeyCode::Home,
            KeyCode::End,
        ]);
    }

    #[test]
    fn presets_cover_the_review_hours_and_reset_with_stable_names() {
        assert_eq!(
            PRESET_CONTROLS,
            [
                (
                    "Dev Time Midnight",
                    "Midnight",
                    DevTimeIntent::SetPreviewHours(0.0),
                ),
                ("Dev Time Dawn", "Dawn", DevTimeIntent::SetPreviewHours(6.5),),
                (
                    "Dev Time Noon",
                    "Noon",
                    DevTimeIntent::SetPreviewHours(12.0),
                ),
                (
                    "Dev Time Golden Hour",
                    "Golden Hour",
                    DevTimeIntent::SetPreviewHours(16.5),
                ),
                (
                    "Dev Time Sunset",
                    "Sunset",
                    DevTimeIntent::SetPreviewHours(18.5),
                ),
                (
                    "Dev Time Night",
                    "Night",
                    DevTimeIntent::SetPreviewHours(20.0),
                ),
                ("Dev Time Reset", "Reset", DevTimeIntent::ResetPreview),
            ]
        );
    }

    #[test]
    fn readout_formats_quarter_hours_as_clock_time() {
        assert_eq!(format_clock(0.0), "00:00");
        assert_eq!(format_clock(6.5), "06:30");
        assert_eq!(format_clock(16.75), "16:45");
        assert_eq!(format_clock(23.75), "23:45");
    }

    #[test]
    fn slider_quantization_clamps_endpoints_without_wrapping() {
        assert_hours_eq(quantize_slider_hours(-0.01), 0.0);
        assert_hours_eq(quantize_slider_hours(0.0), 0.0);
        assert_hours_eq(quantize_slider_hours(23.75), 23.75);
        assert_hours_eq(quantize_slider_hours(24.0), 23.75);
    }

    #[test]
    fn value_change_observer_snaps_to_quarters_and_clamps_without_wrapping() {
        let mut app = App::new();
        app.add_message::<UiIntent>()
            .init_resource::<Received>()
            .insert_resource(DevTimeView::Available {
                game_hours: 12.0,
                preview_hours: None,
            })
            .add_observer(emit_slider_intent)
            .add_systems(Update, receive);
        let slider = app.world_mut().spawn(DevTimeSlider).id();

        for (value, is_final) in [
            (-2.0, false),
            (0.124, false),
            (0.125, false),
            (16.625, false),
            (23.9, false),
            (26.0, true),
            (f32::NAN, true),
        ] {
            app.world_mut().trigger(ValueChange {
                source: slider,
                value,
                is_final,
            });
        }
        app.update();

        assert_eq!(
            app.world().resource::<Received>().0,
            [
                DevTimeIntent::SetPreviewHours(0.0),
                DevTimeIntent::SetPreviewHours(0.0),
                DevTimeIntent::SetPreviewHours(0.25),
                DevTimeIntent::SetPreviewHours(16.75),
                DevTimeIntent::SetPreviewHours(23.75),
                DevTimeIntent::SetPreviewHours(23.75),
            ]
        );
        assert!(app.world().resource::<Received>().0.iter().all(|intent| {
            matches!(
                intent,
                DevTimeIntent::SetPreviewHours(hours)
                    if hours.is_finite()
                        && (SLIDER_MIN_HOURS..=SLIDER_MAX_HOURS).contains(hours)
                        && (hours * 4.0).fract() == 0.0
            )
        }));
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn native_pointer_drag_emits_controlled_slider_intents() {
        let mut app = App::new();
        app.add_plugins(crate::test_support::HeadlessUiPlugin::new(1280, 720))
            .init_resource::<Received>()
            .add_systems(Update, receive.after(UiSystems::EmitIntents));
        app.world_mut().insert_resource(DevTimeView::Available {
            game_hours: 12.0,
            preview_hours: None,
        });
        app.world_mut()
            .insert_resource(character_main_view_chrome());
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        for _ in 0..4 {
            app.update();
        }
        let slider = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<Entity, With<DevTimeSlider>>();
            query.single(world).expect("preview slider")
        };
        let window = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<Entity, With<PrimaryWindow>>();
            query.single(world).expect("primary window")
        };
        let target = bevy::window::WindowRef::Entity(window)
            .normalize(Some(window))
            .expect("explicit primary window must normalize");
        let location = Location {
            target: bevy::camera::NormalizedRenderTarget::Window(target),
            position: Vec2::ZERO,
        };

        app.world_mut().trigger(Pointer::new(
            PointerId::Mouse,
            location.clone(),
            DragStart {
                button: PointerButton::Primary,
                hit: HitData::new(slider, 0.0, None, None),
            },
            slider,
        ));
        app.world_mut().trigger(Pointer::new(
            PointerId::Mouse,
            location.clone(),
            Drag {
                button: PointerButton::Primary,
                distance: Vec2::new(80.0, 0.0),
                delta: Vec2::new(80.0, 0.0),
            },
            slider,
        ));
        app.update();
        app.world_mut().trigger(Pointer::new(
            PointerId::Mouse,
            location,
            DragEnd {
                button: PointerButton::Primary,
                distance: Vec2::new(80.0, 0.0),
            },
            slider,
        ));
        app.update();

        let received = &app.world().resource::<Received>().0;
        assert_eq!(
            received.len(),
            2,
            "drag and release must each publish a value"
        );
        assert!(received.iter().all(|intent| matches!(
            intent,
            DevTimeIntent::SetPreviewHours(hours)
                if *hours > 12.0
                    && *hours <= 23.75
                    && (*hours * 4.0).fract() == 0.0
        )));
        assert_eq!(
            app.world().get::<SliderValue>(slider),
            Some(&SliderValue(12.0)),
            "the native pointer path must remain controlled by DevTimeView"
        );
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn native_keyboard_navigation_is_slider_only_while_focused() {
        let mut app = App::new();
        app.add_plugins(crate::test_support::HeadlessUiPlugin::new(1280, 720))
            .init_resource::<Received>()
            .init_resource::<GameplayNavigationObserved>()
            .add_systems(Update, receive.after(UiSystems::EmitIntents))
            .add_systems(
                Update,
                observe_gameplay_navigation_keys.in_set(AppSystems::RecordInput),
            );
        app.world_mut().insert_resource(DevTimeView::Available {
            game_hours: 12.0,
            preview_hours: None,
        });
        app.world_mut()
            .insert_resource(character_main_view_chrome());
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        for _ in 0..4 {
            app.update();
        }
        let slider = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<Entity, With<DevTimeSlider>>();
            query.single(world).expect("preview slider")
        };
        let window = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<Entity, With<PrimaryWindow>>();
            query.single(world).expect("primary window")
        };
        app.insert_resource(InputFocus::from_entity(slider));

        for (key_code, logical_key) in [
            (KeyCode::ArrowLeft, Key::ArrowLeft),
            (KeyCode::ArrowRight, Key::ArrowRight),
            (KeyCode::Home, Key::Home),
            (KeyCode::End, Key::End),
        ] {
            for state in [ButtonState::Pressed, ButtonState::Released] {
                app.world_mut().write_message(KeyboardInput {
                    key_code,
                    logical_key: logical_key.clone(),
                    state,
                    text: None,
                    repeat: false,
                    window,
                });
                app.update();
                assert!(
                    !app.world().resource::<ButtonInput<KeyCode>>().any_pressed([
                        KeyCode::ArrowLeft,
                        KeyCode::ArrowRight,
                        KeyCode::Home,
                        KeyCode::End,
                    ]),
                    "focused slider navigation must be consumed before gameplay input"
                );
            }
            assert_eq!(app.world().resource::<InputFocus>().get(), Some(slider));
        }

        assert_eq!(
            app.world().resource::<Received>().0,
            [
                DevTimeIntent::SetPreviewHours(11.75),
                DevTimeIntent::SetPreviewHours(12.25),
                DevTimeIntent::SetPreviewHours(0.0),
                DevTimeIntent::SetPreviewHours(23.75),
            ]
        );
        assert_eq!(
            app.world().get::<SliderValue>(slider),
            Some(&SliderValue(12.0)),
            "the widget remains controlled until the adapter publishes its view"
        );
        assert!(
            !app.world().resource::<GameplayNavigationObserved>().0,
            "focused slider keys must not reach an AppSystems::RecordInput consumer"
        );

        *app.world_mut().resource_mut::<DevTimeView>() = DevTimeView::Available {
            game_hours: 12.0,
            preview_hours: Some(SLIDER_MIN_HOURS),
        };
        app.update();
        app.update();
        app.world_mut().resource_mut::<Received>().0.clear();
        for state in [ButtonState::Pressed, ButtonState::Released] {
            app.world_mut().write_message(KeyboardInput {
                key_code: KeyCode::ArrowLeft,
                logical_key: Key::ArrowLeft,
                state,
                text: None,
                repeat: false,
                window,
            });
            app.update();
        }
        assert_eq!(
            app.world().resource::<Received>().0,
            [DevTimeIntent::SetPreviewHours(SLIDER_MIN_HOURS)],
            "ArrowLeft at midnight must clamp instead of wrapping"
        );

        *app.world_mut().resource_mut::<DevTimeView>() = DevTimeView::Available {
            game_hours: 12.0,
            preview_hours: Some(SLIDER_MAX_HOURS),
        };
        app.update();
        app.update();
        app.world_mut().resource_mut::<Received>().0.clear();
        for state in [ButtonState::Pressed, ButtonState::Released] {
            app.world_mut().write_message(KeyboardInput {
                key_code: KeyCode::ArrowRight,
                logical_key: Key::ArrowRight,
                state,
                text: None,
                repeat: false,
                window,
            });
            app.update();
        }
        assert_eq!(
            app.world().resource::<Received>().0,
            [DevTimeIntent::SetPreviewHours(SLIDER_MAX_HOURS)],
            "ArrowRight at 23:45 must clamp instead of wrapping"
        );
        assert!(
            !app.world().resource::<GameplayNavigationObserved>().0,
            "focused endpoint keys must remain isolated from gameplay input"
        );

        app.world_mut().resource_mut::<InputFocus>().clear();
        app.world_mut().write_message(KeyboardInput {
            key_code: KeyCode::ArrowLeft,
            logical_key: Key::ArrowLeft,
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
            window,
        });
        app.update();

        assert!(
            app.world().resource::<GameplayNavigationObserved>().0,
            "the same navigation key must remain available when the slider is not focused"
        );
        assert!(
            app.world()
                .resource::<ButtonInput<KeyCode>>()
                .pressed(KeyCode::ArrowLeft),
            "unfocused raw input must not be consumed"
        );
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn focused_slider_has_visible_outline_and_controlled_accessible_value() {
        let mut app = App::new();
        app.add_plugins(crate::test_support::HeadlessUiPlugin::new(1280, 720));
        app.world_mut().insert_resource(DevTimeView::Available {
            game_hours: 12.0,
            preview_hours: None,
        });
        app.world_mut()
            .insert_resource(character_main_view_chrome());
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        for _ in 0..4 {
            app.update();
        }
        let slider = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<Entity, With<DevTimeSlider>>();
            query.single(world).expect("preview slider")
        };

        app.world_mut().resource_mut::<InputFocusVisible>().0 = true;
        app.world_mut()
            .resource_mut::<InputFocus>()
            .set(slider, FocusCause::Navigated);
        app.update();

        assert_eq!(app.world().resource::<InputFocus>().get(), Some(slider));
        assert!(
            app.world().get::<Outline>(slider).is_some(),
            "keyboard focus must be visibly outlined"
        );
        let accessibility = app
            .world()
            .get::<bevy::a11y::AccessibilityNode>(slider)
            .expect("native slider accessibility node");
        assert_eq!(format!("{:?}", accessibility.role()), "Slider");
        assert_eq!(accessibility.numeric_value(), Some(12.0));

        *app.world_mut().resource_mut::<DevTimeView>() = DevTimeView::Available {
            game_hours: 12.0,
            preview_hours: Some(16.75),
        };
        app.update();
        app.update();

        assert_eq!(
            app.world().get::<SliderValue>(slider),
            Some(&SliderValue(16.75))
        );
        assert_eq!(
            app.world()
                .get::<bevy::a11y::AccessibilityNode>(slider)
                .and_then(|node| node.numeric_value()),
            Some(16.75),
            "the accessible value must follow the adapter-published controlled view"
        );
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(slider));

        app.world_mut().resource_mut::<InputFocusVisible>().0 = false;
        app.update();
        assert!(
            app.world().get::<Outline>(slider).is_none(),
            "the keyboard-only outline must clear when focus visibility is disabled"
        );
        assert_eq!(
            app.world().get::<SliderValue>(slider),
            Some(&SliderValue(16.75))
        );
    }

    #[test]
    fn preset_and_reset_presses_each_emit_one_typed_intent() {
        let mut app = App::new();
        app.add_message::<UiIntent>()
            .init_resource::<Received>()
            .insert_resource(DevTimeView::Available {
                game_hours: 12.0,
                preview_hours: None,
            })
            .add_systems(Update, (emit_preset_intents, receive).chain());
        let preset = app
            .world_mut()
            .spawn((
                Interaction::None,
                DevTimeControl(DevTimeIntent::SetPreviewHours(16.5)),
            ))
            .id();
        let reset = app
            .world_mut()
            .spawn((
                Interaction::None,
                DevTimeControl(DevTimeIntent::ResetPreview),
            ))
            .id();

        app.update();
        assert!(app.world().resource::<Received>().0.is_empty());
        *app.world_mut().get_mut::<Interaction>(preset).unwrap() = Interaction::Pressed;
        app.update();
        *app.world_mut().get_mut::<Interaction>(preset).unwrap() = Interaction::None;
        *app.world_mut().get_mut::<Interaction>(reset).unwrap() = Interaction::Pressed;
        app.update();
        app.update();

        assert_eq!(
            app.world().resource::<Received>().0,
            [
                DevTimeIntent::SetPreviewHours(16.5),
                DevTimeIntent::ResetPreview,
            ]
        );
    }

    #[test]
    fn unavailable_removes_controls_and_blocks_a_stale_press() {
        let mut app = App::new();
        app.add_message::<UiIntent>()
            .init_resource::<Received>()
            .init_resource::<ResolvedUiMetrics>()
            .insert_resource(DevTimeView::Available {
                game_hours: 12.0,
                preview_hours: None,
            })
            .insert_resource(UiAssets {
                display: Handle::default(),
                body: Handle::default(),
                logo: Handle::default(),
                hex_cell: Handle::default(),
            })
            .add_systems(Update, (rebuild, emit_preset_intents, receive).chain());
        app.world_mut()
            .spawn((DevTimeStatus, Text::new("Waiting…")));
        app.world_mut()
            .spawn((DevTimeControls::default(), Node::default()));

        app.update();
        let controls = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<Entity, With<DevTimeControl>>();
            query.iter(world).collect::<Vec<_>>()
        };
        assert_eq!(controls.len(), 7);
        let stale = controls
            .first()
            .copied()
            .expect("available time must create controls");
        *app.world_mut().get_mut::<Interaction>(stale).unwrap() = Interaction::Pressed;
        *app.world_mut().resource_mut::<DevTimeView>() = DevTimeView::Unavailable {
            reason: "Static lighting profile".to_owned(),
        };

        app.update();

        let control_count = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<Entity, With<DevTimeControl>>();
            query.iter(world).count()
        };
        assert_eq!(control_count, 0);
        assert!(app.world().resource::<Received>().0.is_empty());
        let view = DevTimeView::default();
        assert!(matches!(
            view,
            DevTimeView::Unavailable { ref reason } if !reason.is_empty()
        ));
    }

    #[test]
    fn controls_are_focusable_accessible_inspector_descendants() {
        let mut app = App::new();
        app.add_plugins(bevy::ui_widgets::SliderPlugin)
            .insert_resource(DevTimeView::Available {
                game_hours: 12.0,
                preview_hours: None,
            })
            .init_resource::<GameplayChromeView>()
            .insert_resource(crate::resolve_ui_metrics(
                Vec2::new(1920.0, 1080.0),
                crate::UiScaleMode::Auto,
            ))
            .insert_resource(UiAssets {
                display: Handle::default(),
                body: Handle::default(),
                logo: Handle::default(),
                hex_cell: Handle::default(),
            })
            .add_systems(Update, (spawn_panel, rebuild).chain());
        let gameplay_group = app
            .world_mut()
            .spawn((Name::new("Gameplay HUD Safe Frame"), TabGroup::new(0)))
            .id();
        let inspector = app
            .world_mut()
            .spawn((Name::new("Main View HUD Region"), UiRegionRole::Inspector))
            .id();
        app.world_mut()
            .entity_mut(gameplay_group)
            .add_child(inspector);

        app.update();

        let controls = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<(
                Entity,
                &Name,
                &TabIndex,
                &AccessibleLabel,
                &UiVisibilityRequirement,
            ), With<DevTimeControl>>();
            query
                .iter(world)
                .map(|(entity, name, index, label, requirement)| {
                    (
                        entity,
                        name.as_str().to_owned(),
                        index.0,
                        label.0.clone(),
                        *requirement,
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(controls.len(), 7);
        for (entity, name, index, label, requirement) in controls {
            assert!(name.starts_with("Dev Time "));
            assert_eq!(index, 0);
            assert!(!label.is_empty());
            assert_eq!(requirement, UiVisibilityRequirement::Scrollable);
            assert!(has_ancestor(app.world(), entity, inspector));
            assert!(has_ancestor(app.world(), entity, gameplay_group));
        }
        let (
            slider,
            index,
            label,
            range,
            step,
            node,
            requirement,
            role,
            value,
            min,
            max,
            value_step,
        ) = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<(
                Entity,
                &TabIndex,
                &AccessibleLabel,
                &SliderRange,
                &SliderStep,
                &Node,
                &UiVisibilityRequirement,
                &bevy::a11y::AccessibilityNode,
            ), With<DevTimeSlider>>();
            query
                .iter(world)
                .next()
                .map(
                    |(entity, index, label, range, step, node, requirement, accessibility)| {
                        (
                            entity,
                            index.0,
                            label.0.clone(),
                            *range,
                            step.0,
                            node.clone(),
                            *requirement,
                            format!("{:?}", accessibility.role()),
                            accessibility.numeric_value(),
                            accessibility.min_numeric_value(),
                            accessibility.max_numeric_value(),
                            accessibility.numeric_value_step(),
                        )
                    },
                )
                .expect("available time must expose one slider")
        };
        assert_eq!(index, 0);
        assert_eq!(label, "Local visual preview time");
        assert_hours_eq(range.start(), 0.0);
        assert_hours_eq(range.end(), 23.75);
        assert_hours_eq(step, 0.25);
        assert!(matches!(node.min_height, Val::Px(height) if height >= 44.0));
        assert_eq!(requirement, UiVisibilityRequirement::Scrollable);
        assert_eq!(role, "Slider");
        assert_eq!(value, Some(12.0));
        assert_eq!(min, Some(0.0));
        assert_eq!(max, Some(23.75));
        assert_eq!(value_step, Some(0.25));
        assert!(has_ancestor(app.world(), slider, inspector));
        assert!(has_ancestor(app.world(), slider, gameplay_group));

        let expected = std::iter::once("Dev Time Slider".to_owned())
            .chain(
                PRESET_CONTROLS
                    .iter()
                    .map(|(name, _, _)| (*name).to_owned()),
            )
            .collect::<Vec<_>>();
        let (forward, previous_from_first) = {
            let world = app.world_mut();
            let mut navigation_state: SystemState<TabNavigation> = SystemState::new(world);
            let navigation = navigation_state
                .get(world)
                .expect("the gameplay tab group must be structurally valid");
            let mut focus = InputFocus::default();
            let mut entities = Vec::new();
            for _ in 0..=expected.len() {
                let next = navigation
                    .navigate(&focus, NavAction::Next)
                    .expect("every development control must be tabbable");
                entities.push(next);
                focus = InputFocus::from_entity(next);
            }
            let previous = navigation
                .navigate(&InputFocus::from_entity(slider), NavAction::Previous)
                .expect("reverse navigation must wrap from the slider");
            let names = entities
                .into_iter()
                .map(|entity| {
                    world
                        .get::<Name>(entity)
                        .expect("focusable development control must have a name")
                        .as_str()
                        .to_owned()
                })
                .collect::<Vec<_>>();
            let previous = world
                .get::<Name>(previous)
                .expect("wrapped development control must have a name")
                .as_str()
                .to_owned();
            (names, previous)
        };
        assert_eq!(forward.get(..expected.len()), Some(expected.as_slice()));
        assert_eq!(forward.last(), expected.first());
        assert_eq!(previous_from_first, "Dev Time Reset");
    }

    #[test]
    fn compact_panel_stays_in_the_scrollable_inspector_without_recreating_controls() {
        let mut app = App::new();
        app.insert_resource(DevTimeView::Available {
            game_hours: 12.0,
            preview_hours: None,
        })
        .init_resource::<GameplayChromeView>()
        .insert_resource(crate::resolve_ui_metrics(
            Vec2::new(1280.0, 720.0),
            crate::UiScaleMode::Auto,
        ))
        .insert_resource(UiAssets {
            display: Handle::default(),
            body: Handle::default(),
            logo: Handle::default(),
            hex_cell: Handle::default(),
        })
        .add_systems(Startup, spawn_panel)
        .add_systems(Update, (rebuild, reconcile_layout).chain());
        let frame = app
            .world_mut()
            .spawn((Name::new("Gameplay HUD Safe Frame"), TabGroup::new(0)))
            .id();
        let inspector = app
            .world_mut()
            .spawn((Name::new("Main View HUD Region"), UiRegionRole::Inspector))
            .id();
        app.world_mut().entity_mut(frame).add_child(inspector);

        app.update();

        let (panel, parent, position, width) = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<(Entity, &ChildOf, &Node), With<DevTimePanel>>();
            query
                .iter(world)
                .next()
                .map(|(entity, parent, node)| {
                    (entity, parent.parent(), node.position_type, node.width)
                })
                .expect("the compact development panel must exist")
        };
        assert_eq!(parent, inspector);
        assert_eq!(position, PositionType::Relative);
        assert_eq!(width, Val::Percent(100.0));
        let control_sizes = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<&Node, With<DevTimeControl>>();
            query
                .iter(world)
                .map(|node| (node.width, node.height))
                .collect::<Vec<_>>()
        };
        assert_eq!(control_sizes.len(), 7);
        assert!(
            control_sizes
                .iter()
                .all(|size| *size == (Val::Px(76.0), Val::Px(44.0))),
            "new compact controls must use their final size in the first rendered frame"
        );

        let mut before = control_entities(app.world_mut());
        before.sort_by_key(|entity| entity.to_bits());
        assert_eq!(before.len(), 7);
        let focused = before
            .first()
            .copied()
            .expect("compact time controls must be reachable");
        app.insert_resource(InputFocus::from_entity(focused));

        *app.world_mut().resource_mut::<ResolvedUiMetrics>() =
            crate::resolve_ui_metrics(Vec2::new(1920.0, 1080.0), crate::UiScaleMode::Auto);
        app.update();

        assert_eq!(
            app.world().get::<ChildOf>(panel).map(ChildOf::parent),
            Some(inspector)
        );
        let mut after = control_entities(app.world_mut());
        after.sort_by_key(|entity| entity.to_bits());
        assert_eq!(after, before);
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(focused));
    }

    #[test]
    fn ultra_constrained_controls_reflow_inside_the_inspector() {
        let metrics = crate::resolve_ui_metrics(Vec2::new(960.0, 540.0), crate::UiScaleMode::Auto);
        assert_eq!(metrics.viewport, UiViewportClass::Compact);
        let panel = panel_node(metrics, false);
        let (control_width, control_height) = control_size(metrics);
        assert_eq!(panel.position_type, PositionType::Relative);
        assert_eq!(panel.width, Val::Percent(100.0));
        assert!(control_height >= 44.0);
        assert!(control_width * 3.0 + 16.0 <= crate::layout::inspector_width(metrics));
    }

    #[test]
    fn common_two_hundred_percent_canvas_keeps_slider_and_hides_presets_first() {
        let metrics =
            crate::resolve_ui_metrics(Vec2::new(1280.0, 720.0), crate::UiScaleMode::Percent200);
        let panel = panel_node(metrics, false);
        assert_eq!(panel.position_type, PositionType::Relative);
        assert_eq!(panel.width, Val::Percent(100.0));
        assert_eq!(panel.display, Display::Flex);
        assert_eq!(presets_node(metrics, false).display, Display::None);
        assert_eq!(slider_node().display, Display::Flex);
        let (control_width, control_height) = control_size(metrics);
        assert!(control_width * 2.0 + 12.0 <= crate::layout::inspector_width(metrics));
        assert!(control_height >= 44.0);
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn compact_panel_and_controls_fit_without_covering_primary_gameplay_surfaces() {
        for logical_size in [
            UVec2::new(960, 540),
            UVec2::new(1280, 720),
            UVec2::new(1920, 1080),
        ] {
            for mode in [crate::UiScaleMode::Auto, crate::UiScaleMode::Percent200] {
                let mut app = App::new();
                app.add_plugins(crate::test_support::HeadlessUiPlugin::new(
                    logical_size.x,
                    logical_size.y,
                ));
                app.world_mut()
                    .insert_resource(crate::UiScalePreference(mode));
                app.world_mut().insert_resource(DevTimeView::Available {
                    game_hours: 12.0,
                    preview_hours: None,
                });
                app.world_mut()
                    .insert_resource(character_main_view_chrome());
                app.world_mut()
                    .resource_mut::<NextState<Screen>>()
                    .set(Screen::Gameplay);
                app.update();
                let expected_size = control_size(*app.world().resource::<ResolvedUiMetrics>());
                let first_frame_sizes = {
                    let world = app.world_mut();
                    let mut query = world.query_filtered::<&Node, With<DevTimeControl>>();
                    query
                        .iter(world)
                        .map(|node| (node.width, node.height))
                        .collect::<Vec<_>>()
                };
                assert_eq!(first_frame_sizes.len(), 7);
                assert!(first_frame_sizes
                    .iter()
                    .all(|size| { *size == (Val::Px(expected_size.0), Val::Px(expected_size.1)) }));
                for _ in 0..7 {
                    app.update();
                }

                let snapshot = crate::test_support::ui_tree_snapshot(app.world_mut());
                if snapshot.metrics.viewport != UiViewportClass::Compact {
                    continue;
                }
                let Some(panel) = snapshot
                    .nodes
                    .iter()
                    .find(|node| node.name == "Dev Time Panel")
                else {
                    assert!(
                        panel_is_collapsed(snapshot.metrics, false),
                        "the development panel may collapse only when secondary tooling cannot fit at {logical_size:?} in {mode:?}: {:?}",
                        snapshot.metrics
                    );
                    continue;
                };
                assert!(panel.size.cmpgt(Vec2::ZERO).all());
                assert!(
                    !panel.overflows,
                    "the panel must fit at {logical_size:?} in {mode:?}: {panel:?}"
                );
                let panel_min = panel.center - panel.size * 0.5;
                let panel_max = panel.center + panel.size * 0.5;
                assert!(panel_min.cmpge(Vec2::ZERO).all());
                assert!(
                    panel_max.cmple(snapshot.metrics.logical_size).all(),
                    "the panel must remain on canvas at {logical_size:?} in {mode:?}: panel={panel:?}, metrics={:?}",
                    snapshot.metrics
                );

                let presets_hidden = presets_are_hidden(snapshot.metrics, false);
                for control_name in PRESET_CONTROLS.map(|(name, _, _)| name) {
                    if presets_hidden {
                        assert!(snapshot.nodes.iter().all(|node| node.name != control_name));
                        continue;
                    }
                    let control = snapshot
                        .nodes
                        .iter()
                        .find(|node| node.name == control_name)
                        .unwrap_or_else(|| {
                            panic!(
                                "{control_name:?} must be visible at {logical_size:?} in {mode:?}"
                            )
                        });
                    assert!(control.size.cmpgt(Vec2::ZERO).all());
                    assert!(
                        !control.overflows,
                        "{control_name:?} must fit at {logical_size:?} in {mode:?}: {control:?}"
                    );
                    let control_min = control.center - control.size * 0.5;
                    let control_max = control.center + control.size * 0.5;
                    assert!(control_min.cmpge(panel_min).all());
                    assert!(control_max.cmple(panel_max).all());
                }
                let slider = snapshot
                    .nodes
                    .iter()
                    .find(|node| node.name == "Dev Time Slider")
                    .expect("the slider must remain visible before the presets");
                assert!(slider.size.cmpge(Vec2::new(44.0, 44.0)).all());
                let slider_min = slider.center - slider.size * 0.5;
                let slider_max = slider.center + slider.size * 0.5;
                assert!(slider_min.cmpge(panel_min).all());
                assert!(slider_max.cmple(panel_max).all());

                for primary_name in [
                    "Party HUD Region",
                    "Initiative HUD Region",
                    "Action Bar HUD Region",
                    "Casting Panel",
                    "Action Bar",
                ] {
                    let Some(primary) =
                        snapshot.nodes.iter().find(|node| node.name == primary_name)
                    else {
                        continue;
                    };
                    assert!(
                        !overlaps(panel, primary),
                        "the development panel must not cover {primary_name:?} at {logical_size:?} in {mode:?}: panel={panel:?}, primary={primary:?}"
                    );
                }
            }
        }
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn required_choice_hides_ultra_constrained_controls_and_preserves_their_entities() {
        let mut app = App::new();
        app.add_plugins(crate::test_support::HeadlessUiPlugin::new(1280, 720))
            .init_resource::<RequiredChoiceTransition>()
            .add_systems(
                Update,
                activate_required_choice_for_render.in_set(AppSystems::Update),
            );
        app.world_mut()
            .insert_resource(crate::UiScalePreference(crate::UiScaleMode::Percent200));
        app.world_mut().insert_resource(DevTimeView::Available {
            game_hours: 12.0,
            preview_hours: None,
        });
        app.world_mut()
            .insert_resource(character_main_view_chrome());
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        for _ in 0..8 {
            app.update();
        }

        let mut before = control_entities(app.world_mut());
        before.sort_by_key(|entity| entity.to_bits());
        assert_eq!(before.len(), 7);
        let slider = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<Entity, With<DevTimeSlider>>();
            query
                .single(world)
                .expect("available time must expose a slider")
        };
        assert_eq!(
            app.world().get::<TabIndex>(slider),
            Some(&TabIndex(0)),
            "visible slider must begin in the native focus order"
        );
        app.insert_resource(InputFocus::from_entity(slider));
        app.world_mut().resource_mut::<RequiredChoiceTransition>().0 = true;
        app.update();

        let required = crate::test_support::ui_tree_snapshot(app.world_mut());
        assert!(required
            .nodes
            .iter()
            .all(|node| node.name != "Dev Time Panel"));
        assert!(PRESET_CONTROLS
            .iter()
            .all(|(name, _, _)| required.nodes.iter().all(|node| node.name != *name)));
        assert_eq!(app.world().resource::<InputFocus>().get(), None);
        assert_eq!(
            app.world().get::<TabIndex>(slider),
            Some(&TabIndex(-1)),
            "a hidden native slider must leave Bevy's tab order"
        );
        let required_surface = required
            .nodes
            .iter()
            .find(|node| node.name == "Own Lattice Panel")
            .expect("the required Main View must remain visible");
        assert!(required_surface.size.cmpgt(Vec2::ZERO).all());

        *app.world_mut().resource_mut::<GameplayChromeView>() = character_main_view_chrome();
        app.world_mut()
            .insert_resource(crate::UiScalePreference(crate::UiScaleMode::Auto));
        app.insert_resource(crate::GameplayHudView::default());
        for _ in 0..4 {
            app.update();
        }

        let restored = crate::test_support::ui_tree_snapshot(app.world_mut());
        assert!(restored
            .nodes
            .iter()
            .any(|node| node.name == "Dev Time Panel"));
        assert_eq!(
            app.world().get::<TabIndex>(slider),
            Some(&TabIndex(0)),
            "restoring the panel must restore the slider's authored focus order"
        );
        let mut after = control_entities(app.world_mut());
        after.sort_by_key(|entity| entity.to_bits());
        assert_eq!(after, before);
    }

    #[test]
    fn unchanged_available_view_rebuilds_recreated_gameplay_panel() {
        let mut app = App::new();
        app.insert_resource(DevTimeView::Available {
            game_hours: 6.0,
            preview_hours: None,
        })
        .init_resource::<ResolvedUiMetrics>()
        .insert_resource(UiAssets {
            display: Handle::default(),
            body: Handle::default(),
            logo: Handle::default(),
            hex_cell: Handle::default(),
        })
        .add_systems(Update, rebuild);
        let first_status = app
            .world_mut()
            .spawn((DevTimeStatus, Text::new("Checking cyclic time…")))
            .id();
        let first_controls = app
            .world_mut()
            .spawn((DevTimeControls::default(), Node::default()))
            .id();

        app.update();
        assert_eq!(control_entities(app.world_mut()).len(), 7);

        for entity in control_entities(app.world_mut()) {
            assert!(app.world_mut().despawn(entity));
        }
        assert!(app.world_mut().despawn(first_controls));
        assert!(app.world_mut().despawn(first_status));
        let second_status = app
            .world_mut()
            .spawn((DevTimeStatus, Text::new("Checking cyclic time…")))
            .id();
        app.world_mut()
            .spawn((DevTimeControls::default(), Node::default()));

        app.update();

        assert_eq!(control_entities(app.world_mut()).len(), 7);
        assert_eq!(
            app.world()
                .get::<Text>(second_status)
                .map(|text| text.as_str()),
            Some("LOCAL VISUAL PREVIEW · OFF\nGAME · 06:00")
        );
    }

    #[test]
    fn available_hour_updates_preserve_control_entities_and_focus() {
        let mut app = App::new();
        app.insert_resource(DevTimeView::Available {
            game_hours: 12.0,
            preview_hours: None,
        })
        .init_resource::<ResolvedUiMetrics>()
        .insert_resource(UiAssets {
            display: Handle::default(),
            body: Handle::default(),
            logo: Handle::default(),
            hex_cell: Handle::default(),
        })
        .add_systems(Update, rebuild);
        app.world_mut()
            .spawn((DevTimeStatus, Text::new("Checking cyclic time…")));
        app.world_mut()
            .spawn((DevTimeControls::default(), Node::default()));
        app.update();
        let mut before = control_entities(app.world_mut());
        before.sort_by_key(|entity| entity.to_bits());
        let focused = before
            .first()
            .copied()
            .expect("available time must create controls");
        app.insert_resource(InputFocus::from_entity(focused));

        *app.world_mut().resource_mut::<DevTimeView>() = DevTimeView::Available {
            game_hours: 12.0,
            preview_hours: Some(12.5),
        };
        app.update();

        let mut after = control_entities(app.world_mut());
        after.sort_by_key(|entity| entity.to_bits());
        assert_eq!(after, before);
        assert!(app.world().get_entity(focused).is_ok());
        assert_eq!(
            app.world().resource::<InputFocus>().get(),
            Some(focused),
            "updating the time must preserve keyboard focus"
        );
    }

    fn control_entities(world: &mut World) -> Vec<Entity> {
        let mut query = world.query_filtered::<Entity, With<DevTimeControl>>();
        query.iter(world).collect()
    }

    fn has_ancestor(world: &World, mut entity: Entity, wanted: Entity) -> bool {
        while let Some(parent) = world.get::<ChildOf>(entity) {
            entity = parent.parent();
            if entity == wanted {
                return true;
            }
        }
        false
    }

    #[cfg(feature = "test-support")]
    fn overlaps(
        left: &crate::test_support::UiNodeObservation,
        right: &crate::test_support::UiNodeObservation,
    ) -> bool {
        let left_min = left.center - left.size * 0.5;
        let left_max = left.center + left.size * 0.5;
        let right_min = right.center - right.size * 0.5;
        let right_max = right.center + right.size * 0.5;
        left_min.x < right_max.x
            && left_max.x > right_min.x
            && left_min.y < right_max.y
            && left_max.y > right_min.y
    }

    #[cfg(feature = "test-support")]
    fn required_hud() -> crate::GameplayHudView {
        let disabled = |reason: &str| crate::ActionAvailability::Disabled {
            reason: reason.to_owned(),
        };
        crate::GameplayHudView {
            phase: hex_core::GameplayPhase::Active,
            actor: Some(hex_core::UnitId(0)),
            actor_label: "Hedge Mage".to_owned(),
            round: "Round 1".to_owned(),
            movement_remaining: 2,
            action_remaining: true,
            required_prompt: Some("Choose the required cells in the lattice".to_owned()),
            actions: vec![
                crate::ActionAffordance {
                    action: crate::GameplayAction::ConfirmDecision,
                    label: "Confirm choice".to_owned(),
                    shortcut: Some("Enter".to_owned()),
                    availability: disabled("Choose the required cells in the lattice"),
                    priority: crate::ActionPriority::Required,
                },
                crate::ActionAffordance {
                    action: crate::GameplayAction::Channel,
                    label: "Channel".to_owned(),
                    shortcut: None,
                    availability: disabled("Resolve the required choice first"),
                    priority: crate::ActionPriority::Primary,
                },
                crate::ActionAffordance {
                    action: crate::GameplayAction::EndTurn,
                    label: "End turn".to_owned(),
                    shortcut: Some("Space".to_owned()),
                    availability: disabled("Resolve the required choice first"),
                    priority: crate::ActionPriority::Primary,
                },
            ],
        }
    }
}
