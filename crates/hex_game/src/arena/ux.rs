//! Player-facing UI consumes gameplay observations and a cached world overview.
pub(super) mod icons;
mod performance;
#[cfg(test)]
mod tests;
use super::{hud, recording::Recorder, ArenaFrame, ViewState};
use bevy::asset::RenderAssetUsages;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::ui::RelativeCursorPosition;
use bevy::window::PrimaryWindow;
use hex_arena::{
    ArenaSession, ArenaTuning, LandmarkKind, PlayerObservation, SpellAvailabilityState,
};
use hex_core::arena::{ArenaOverview, ArenaReset, ArenaTerrainView, ArenaVoxelGeometry};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Page {
    #[default]
    Overview,
    Map,
    Upgrades,
    Settings,
    Controls,
}
impl Page {
    pub(super) const ALL: [Self; 5] = [
        Self::Overview,
        Self::Map,
        Self::Upgrades,
        Self::Settings,
        Self::Controls,
    ];
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Map => "Map",
            Self::Upgrades => "Upgrades",
            Self::Settings => "Settings",
            Self::Controls => "Controls",
        }
    }
}
#[derive(Component, Clone, Copy)]
pub(super) enum UxAction {
    Page(Page),
    Record,
    OpenFolder,
    Scale,
    ClearPin,
}
#[derive(Component)]
pub(super) enum UxLabel {
    Target,
    Notice,
    Recording,
    RecordButton,
    RecorderDetail,
    RecorderFull,
    Scale,
    MapSelection,
    Pin,
}
#[derive(Component)]
pub(super) struct PageBody(pub Page);
#[derive(Component)]
pub(super) struct MiniMap;
#[derive(Component)]
pub(super) struct MenuScroll;
#[derive(Component)]
pub(super) struct MenuScrollHint;
#[derive(Component)]
pub(super) struct MenuPanel;
#[derive(Component)]
pub(super) struct MenuHelper;
#[derive(Component)]
pub(super) struct SpellFill(pub usize);
#[derive(Component)]
pub(super) struct SpellIcon(pub usize);
#[derive(Component)]
pub(super) struct Reticle;
#[derive(Component)]
pub(super) struct MapCanvas {
    large: bool,
}
#[derive(Component)]
struct MapDot(usize);
#[derive(Serialize, Deserialize)]
struct Preferences {
    scale: f32,
}
#[derive(Resource)]
pub(super) struct UxState {
    page: Page,
    map_visible: bool,
    scale: f32,
    generation: u64,
    pin: Option<Vec2>,
    selected: String,
    selected_id: Option<String>,
    image: Option<Handle<Image>>,
    image_generation: Option<u64>,
    image_size: UVec2,
    hit_sequence: u64,
    flash: f32,
    keyboard_press: Option<Entity>,
    focus: Option<Entity>,
    pub(super) present_micros: f64,
    timing_start: Option<std::time::Instant>,
    timing_samples: std::collections::VecDeque<f64>,
    observation_micros: f64,
    notice: String,
    notice_remaining: f32,
}
impl Default for UxState {
    fn default() -> Self {
        Self {
            page: Page::Overview,
            map_visible: false,
            scale: 1.0,
            generation: 0,
            pin: None,
            selected: String::new(),
            selected_id: None,
            image: None,
            image_generation: None,
            image_size: UVec2::ZERO,
            hit_sequence: 0,
            flash: 0.0,
            keyboard_press: None,
            focus: None,
            present_micros: 0.0,
            timing_start: None,
            timing_samples: std::collections::VecDeque::with_capacity(256),
            observation_micros: 0.0,
            notice: String::new(),
            notice_remaining: 0.0,
        }
    }
}
impl UxState {
    pub(super) fn snapshot(&self) -> serde_json::Value {
        let mut samples: Vec<_> = self.timing_samples.iter().copied().collect();
        samples.sort_by(f64::total_cmp);
        serde_json::json!({"page":self.page.name(),"scale":self.scale,"map_visible":self.map_visible,"destination":self.pin.map(|value| value.to_array()),"selected_landmark":self.selected_id,"ui_cpu_samples":samples.len(),"ui_cpu_p95_micros":samples.get(samples.len().saturating_sub(1)*95/100)})
    }
    pub(super) fn has_menu_focus(&self) -> bool {
        self.focus.is_some()
    }
}

fn preference_path() -> std::path::PathBuf {
    crate::storage::StoragePaths::default()
        .preferences
        .with_file_name("battle-ui.ron")
}
fn load(mut ux: ResMut<UxState>, state: Res<ViewState>) {
    if state.capture.is_some() {
        ux.page = match std::env::var("HEX_ARENA_UI_PAGE").ok().as_deref() {
            Some("map") => Page::Map,
            Some("upgrades") => Page::Upgrades,
            Some("settings") => Page::Settings,
            Some("controls") => Page::Controls,
            Some("overview") => Page::Overview,
            _ if state.capture_view == "tuning" => Page::Upgrades,
            _ => Page::Overview,
        };
        ux.map_visible = std::env::var("HEX_ARENA_UI_MAP").is_ok_and(|s| s == "1");
        ux.scale = std::env::var("HEX_ARENA_UI_SCALE")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .filter(|v| v.is_finite())
            .unwrap_or(1.0)
            .clamp(1.0, 2.0);
    }
    if state.capture.is_none() {
        if let Ok(p) = crate::storage::read(&preference_path())
            .and_then(|s| ron::from_str::<Preferences>(&s).map_err(std::io::Error::other))
        {
            if p.scale.is_finite() {
                ux.scale = p.scale.clamp(1.0, 2.0);
            }
        }
    }
}
pub(super) fn install(app: &mut App) {
    performance::install(app);
    app.init_resource::<UxState>()
        .add_systems(
            Update,
            begin_timing.before(hud::update).in_set(ArenaFrame::Present),
        )
        .add_systems(Startup, load)
        .add_systems(
            Update,
            observe.after(ArenaFrame::Tick).before(ArenaFrame::Present),
        )
        .add_systems(
            Update,
            (
                present_map,
                present_feedback,
                present_menus,
                reflow,
                present_scroll_hints,
                end_timing,
            )
                .chain()
                .after(hud::update)
                .in_set(ArenaFrame::Present),
        );
}
pub(super) fn spawn_map(parent: &mut ChildSpawnerCommands, large: bool, size: f32) {
    parent
        .spawn((
            Node {
                width: px(size),
                height: px(size),
                max_width: percent(100),
                flex_shrink: 0.0,
                overflow: Overflow::clip(),
                ..default()
            },
            ImageNode::default(),
            MapCanvas { large },
            Interaction::default(),
            RelativeCursorPosition::default(),
            Name::new(if large { "Expanded map" } else { "Minimap" }),
        ))
        .with_children(|map| {
            for index in 0..13 {
                map.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        display: Display::None,
                        margin: UiRect::axes(px(-9), px(-14)),
                        ..default()
                    },
                    hud::text("", 24.0, Color::WHITE),
                    TextShadow {
                        offset: Vec2::splat(1.5),
                        color: Color::BLACK,
                    },
                    MapDot(index),
                ));
            }
        });
}
#[expect(
    clippy::cast_possible_truncation,
    reason = "Finite camera directions are rounded to eight octants, then wrapped to 0..8."
)]
fn direction_octant(direction: Vec2) -> usize {
    ((direction.x.atan2(-direction.y) / std::f32::consts::FRAC_PI_4).round() as i32).rem_euclid(8)
        as usize
}
fn coordinates(overview: &ArenaOverview, position: Vec2) -> Vec2 {
    ((position - overview.min) / (overview.max - overview.min)).clamp(Vec2::ZERO, Vec2::ONE)
}

pub(super) fn controls(
    state: Res<ViewState>,
    keys: Res<ButtonInput<KeyCode>>,
    mut ux: ResMut<UxState>,
    actions: Query<(&Interaction, &UxAction), Changed<Interaction>>,
    mut recorder: Option<ResMut<Recorder>>,
    reset: Res<ArenaReset>,
    overview: Option<Res<ArenaOverview>>,
    geometry: Res<ArenaVoxelGeometry>,
    session: Res<ArenaSession>,
    maps: Query<(&Interaction, &MapCanvas, &RelativeCursorPosition), Changed<Interaction>>,
    mut wheels: MessageReader<MouseWheel>,
    mut scrolls: Query<(&mut ScrollPosition, &ComputedNode), With<MenuScroll>>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    if ux.generation != reset.generation {
        ux.generation = reset.generation;
        ux.pin = None;
        ux.selected.clear();
        ux.selected_id = None;
        if state.capture.is_none() {
            ux.map_visible = false;
        }
        if state.capture.is_none() {
            ux.page = Page::Overview;
        }
        ux.focus = None;
    }
    if windows.iter().any(|w| !w.focused) {
        wheels.clear();
        return;
    }
    if !state.paused
        && state.started
        && keys.just_pressed(KeyCode::KeyM)
        && session.expedition_progress().is_some()
    {
        ux.map_visible = !ux.map_visible;
    }
    if !state.paused {
        wheels.clear();
        ux.focus = None;
        return;
    }
    for (interaction, action) in &actions {
        if *interaction == Interaction::Pressed {
            match *action {
                UxAction::Page(page) => {
                    ux.page = page;
                    ux.focus = None;
                    for (mut scroll, _) in &mut scrolls {
                        scroll.0.y = 0.0;
                    }
                }
                UxAction::Record => {
                    if let Some(r) = recorder.as_mut() {
                        r.request_toggle();
                    }
                }
                UxAction::OpenFolder => {
                    if let Some(r) = recorder.as_mut() {
                        r.request_open_folder();
                    }
                }
                UxAction::ClearPin => {
                    ux.pin = None;
                    ux.selected.clear();
                    ux.selected_id = None;
                }
                UxAction::Scale => {
                    ux.scale = if ux.scale < 1.24 {
                        1.25
                    } else if ux.scale < 1.49 {
                        1.5
                    } else if ux.scale < 1.99 {
                        2.0
                    } else {
                        1.0
                    };
                    if state.capture.is_none() {
                        if let Ok(s) = ron::to_string(&Preferences { scale: ux.scale }) {
                            if let Err(e) = crate::storage::write_atomic(&preference_path(), &s) {
                                warn!("Could not save Battle UI size: {e}");
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some(overview) = overview.filter(|o| !o.rgba.is_empty()) {
        for (interaction, canvas, cursor) in &maps {
            if !canvas.large
                || *interaction != Interaction::Pressed
                || !cursor.cursor_over
                || ux.page != Page::Map
            {
                continue;
            }
            if let Some(normalized) = cursor.normalized {
                let fraction = (normalized + Vec2::splat(0.5)).clamp(Vec2::ZERO, Vec2::ONE);
                let markers = session.discovered_landmarks();
                let selected = markers
                    .iter()
                    .filter(|m| coordinates(&overview, m.position.xz()).distance(fraction) < 0.04)
                    .min_by(|a, b| {
                        coordinates(&overview, a.position.xz())
                            .distance_squared(fraction)
                            .total_cmp(
                                &coordinates(&overview, b.position.xz()).distance_squared(fraction),
                            )
                    });
                if let Some(m) = selected {
                    ux.selected_id = Some(m.id.clone());
                    ux.selected = format!(
                        "{}: {}",
                        landmark_name(m.kind),
                        if m.kind == LandmarkKind::Fountain {
                            if m.consumed {
                                "spent"
                            } else {
                                "charged — heals up to 40 HP"
                            }
                        } else if m.defeated {
                            "defeated"
                        } else {
                            "last seen here"
                        }
                    );
                    ux.pin = Some(m.position.xz());
                } else {
                    let position = overview.min + fraction * (overview.max - overview.min);
                    if geometry
                        .voxel_at(Vec3::new(position.x, 0.0, position.y))
                        .is_some()
                    {
                        ux.pin = Some(position);
                        ux.selected_id = None;
                        ux.selected = "Personal destination placed".into();
                    }
                }
            }
        }
    }
    let dy = wheels
        .read()
        .map(|w| {
            -w.y * if w.unit == MouseScrollUnit::Line {
                48.0
            } else {
                1.0
            }
        })
        .sum::<f32>();
    if dy.abs() <= f32::EPSILON {
        return;
    }
    for (mut scroll, node) in &mut scrolls {
        if node.size().y > 0.0 {
            let max =
                (node.content_size().y - node.size().y).max(0.0) * node.inverse_scale_factor();
            scroll.0.y = (scroll.0.y + dy).clamp(0.0, max);
        }
    }
}
pub(super) fn keyboard(
    state: Res<ViewState>,
    keys: Res<ButtonInput<KeyCode>>,
    mut ux: ResMut<UxState>,
    mut buttons: Query<(Entity, &mut Interaction, &ComputedNode, &UiGlobalTransform), With<Button>>,
    parents: Query<&ChildOf>,
    mut scrolls: Query<(&mut ScrollPosition, &ComputedNode, &UiGlobalTransform), With<MenuScroll>>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    if let Some(entity) = ux.keyboard_press.take() {
        if let Ok((_, mut interaction, _, _)) = buttons.get_mut(entity) {
            *interaction = Interaction::None;
        }
    }
    if !state.paused || windows.iter().any(|w| !w.focused) {
        return;
    }
    let step = if keys.just_pressed(KeyCode::ArrowDown) || keys.just_pressed(KeyCode::ArrowRight) {
        1
    } else if keys.just_pressed(KeyCode::ArrowUp) || keys.just_pressed(KeyCode::ArrowLeft) {
        -1
    } else {
        0
    };
    if step != 0 {
        let mut visible = buttons
            .iter()
            .filter(|(_, _, n, _)| n.size().min_element() > 0.0)
            .map(|(e, _, _, t)| (e, t.affine().translation))
            .collect::<Vec<_>>();
        visible.sort_by(|a, b| a.1.y.total_cmp(&b.1.y).then(a.1.x.total_cmp(&b.1.x)));
        if !visible.is_empty() {
            let i = ux.focus.and_then(|e| visible.iter().position(|v| v.0 == e));
            let next = match (i, step) {
                (Some(i), 1) => (i + 1) % visible.len(),
                (Some(i), _) => (i + visible.len() - 1) % visible.len(),
                (None, 1) => 0,
                _ => visible.len() - 1,
            };
            let Some((focused, _)) = visible.get(next).copied() else {
                return;
            };
            ux.focus = Some(focused);
            if let Ok((_, _, child, transform)) = buttons.get(focused) {
                let child_rect =
                    Rect::from_center_size(transform.affine().translation, child.size());
                let mut ancestor = focused;
                while let Ok(parent) = parents.get(ancestor) {
                    ancestor = parent.parent();
                    if let Ok((mut scroll, node, transform)) = scrolls.get_mut(ancestor) {
                        let area =
                            Rect::from_center_size(transform.affine().translation, node.size());
                        let delta = if child_rect.min.y < area.min.y {
                            child_rect.min.y - area.min.y
                        } else if child_rect.max.y > area.max.y {
                            child_rect.max.y - area.max.y
                        } else {
                            0.0
                        };
                        let max = (node.content_size().y - node.size().y).max(0.0)
                            * node.inverse_scale_factor();
                        scroll.0.y =
                            (scroll.0.y + delta * node.inverse_scale_factor()).clamp(0.0, max);
                        break;
                    }
                }
            }
        }
    }
    if keys.just_pressed(KeyCode::Enter) {
        if let Some(e) = ux.focus {
            if let Ok((_, mut interaction, n, _)) = buttons.get_mut(e) {
                if n.size().min_element() > 0.0 {
                    *interaction = Interaction::Pressed;
                    ux.keyboard_press = Some(e);
                }
            }
        }
    }
}
fn observe(
    mut session: ResMut<ArenaSession>,
    mut ux: ResMut<UxState>,
    world: Res<ArenaTerrainView>,
    geometry: Res<ArenaVoxelGeometry>,
    state: Res<ViewState>,
    time: Res<Time>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    let start = std::time::Instant::now();
    let Some(actor) = session
        .human_actor_id()
        .and_then(|id| session.actors.iter().find(|a| a.id == id))
    else {
        return;
    };
    let direction = super::aim(&state);
    let origin = super::camera_origin(&session, &state, actor.eye(), direction);
    let (aspect, height) = windows.single().map_or((16.0 / 9.0, 900.0), |w| {
        (
            w.width() / w.height().max(1.0),
            w.height() * w.resolution.scale_factor(),
        )
    });
    session.observe_player(
        &world,
        *geometry,
        PlayerObservation {
            active: state.started
                && !state.paused
                && !state.external_camera()
                && (state.capture.is_some() || windows.iter().all(|w| w.focused)),
            origin,
            direction,
            vertical_fov: 75.0_f32.to_radians(),
            aspect,
            viewport_height: height,
        },
        time.delta_secs(),
    );
    ux.observation_micros = start.elapsed().as_secs_f64() * 1e6;
}
fn landmark_name(kind: LandmarkKind) -> &'static str {
    match kind {
        LandmarkKind::Dragon => "Dragon",
        LandmarkKind::Shadow => "Shadow",
        LandmarkKind::Troll => "Troll",
        LandmarkKind::Fountain => "Fountain",
    }
}
fn present_map(
    state: Res<ViewState>,
    mut ux: ResMut<UxState>,
    overview: Option<Res<ArenaOverview>>,
    session: Res<ArenaSession>,
    mut images: ResMut<Assets<Image>>,
    scrolls: Query<&ComputedNode, With<MenuScroll>>,
    mut canvases: Query<
        (&MapCanvas, &mut ImageNode, &mut Node),
        (Without<MapDot>, Without<MiniMap>),
    >,
    mut dots: Query<(&MapDot, &mut Node, &mut Text, &mut TextColor), Without<MapCanvas>>,
    mut minis: Query<&mut Node, (With<MiniMap>, Without<MapDot>, Without<MapCanvas>)>,
) {
    let start = std::time::Instant::now();
    let valid = overview
        .as_ref()
        .is_some_and(|o| o.width > 0 && !o.rgba.is_empty());
    for mut node in &mut minis {
        set_display(
            &mut node,
            if valid && ux.map_visible && !state.paused {
                Display::Flex
            } else {
                Display::None
            },
        );
    }
    let Some(overview) = overview.filter(|o| !o.rgba.is_empty()) else {
        for (_, mut image, mut node) in &mut canvases {
            set_display(&mut node, Display::None);
            if image.image != Handle::default() {
                image.image = Handle::default();
            }
        }
        if let Some(handle) = ux.image.take() {
            images.remove(handle.id());
        }
        ux.image_generation = None;
        for (_, mut node, _, _) in &mut dots {
            set_display(&mut node, Display::None);
        }
        return;
    };
    for (_, _, mut node) in &mut canvases {
        set_display(&mut node, Display::Flex);
    }
    if overview.is_changed()
        || ux.image_generation != Some(overview.generation)
        || ux.image_size != UVec2::new(overview.width, overview.height)
    {
        let image = Image::new(
            Extent3d {
                width: overview.width,
                height: overview.height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            overview.rgba.clone(),
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        if let Some(handle) = ux.image.take() {
            images.remove(handle.id());
        }
        let handle = images.add(image);
        for (_, mut image_node, _) in &mut canvases {
            image_node.image = handle.clone();
        }
        ux.image = Some(handle);
        ux.image_generation = Some(overview.generation);
        ux.image_size = UVec2::new(overview.width, overview.height);
    }
    let viewport = scrolls
        .iter()
        .map(|node| node.size() * node.inverse_scale_factor())
        .filter(|size| size.min_element() > 0.0)
        .max_by(|a, b| a.y.total_cmp(&b.y))
        .unwrap_or(Vec2::new(1000.0, 400.0));
    let aspect = (overview.max.y - overview.min.y) / (overview.max.x - overview.min.x);
    for (canvas, _, mut node) in &mut canvases {
        let width = if canvas.large {
            480.0_f32
                .min(viewport.x * 0.48)
                .min((viewport.y - 4.0).max(1.0) / aspect)
        } else {
            280.0
        };
        if node.width != px(width) {
            node.width = px(width);
        }
        if node.height != px(width * aspect) {
            node.height = px(width * aspect);
        }
    }
    let player = session
        .human_actor_id()
        .and_then(|id| session.actors.iter().find(|a| a.id == id));
    let mut markers = Vec::with_capacity(13);
    if let Some(player) = player {
        let d = super::aim(&state);
        let arrows = ["↑", "↗", "→", "↘", "↓", "↙", "←", "↖"];
        let index = direction_octant(d.xz());
        markers.push(Some((
            player.feet.xz(),
            arrows.get(index).copied().unwrap_or("↑"),
            Color::WHITE,
        )));
    } else {
        markers.push(None);
    }
    markers.push(ux.pin.map(|p| (p, "◆", Color::srgb(1.0, 0.84, 0.35))));
    for m in session.discovered_landmarks() {
        let (glyph, color) = if m.kind == LandmarkKind::Fountain {
            if m.consumed {
                ("○", Color::srgb(0.85, 0.9, 0.96))
            } else {
                ("+", Color::srgb(0.4, 1.0, 0.87))
            }
        } else if m.defeated {
            ("×", Color::srgb(0.8, 0.8, 0.85))
        } else {
            (
                match m.kind {
                    LandmarkKind::Dragon => "D",
                    LandmarkKind::Shadow => "S",
                    LandmarkKind::Troll => "T",
                    LandmarkKind::Fountain => "+",
                },
                Color::srgb(1.0, 0.69, 0.44),
            )
        };
        markers.push(Some((m.position.xz(), glyph, color)));
    }
    for (dot, mut node, mut text, mut color) in &mut dots {
        if let Some(Some((position, glyph, c))) = markers.get(dot.0) {
            let f = coordinates(&overview, *position);
            let left = percent(f.x * 100.0);
            let top = percent(f.y * 100.0);
            if node.left != left {
                node.left = left;
            }
            if node.top != top {
                node.top = top;
            }
            set_display(&mut node, Display::Flex);
            if text.0 != *glyph {
                text.0 = (*glyph).into();
            }
            color.set_if_neq(TextColor(*c));
        } else {
            set_display(&mut node, Display::None);
        }
    }
    ux.present_micros = start.elapsed().as_secs_f64() * 1e6;
}
fn present_feedback(
    session: Res<ArenaSession>,
    tuning: Res<ArenaTuning>,
    time: Res<Time>,
    mut ux: ResMut<UxState>,
    mut fills: Query<(&SpellFill, &mut Node, &mut BackgroundColor)>,
    mut icons: Query<(&SpellIcon, &mut ImageNode), Without<Reticle>>,
    mut reticles: Query<&mut TextColor, (With<Reticle>, Without<SpellIcon>)>,
) {
    ux.notice_remaining = (ux.notice_remaining - time.delta_secs()).max(0.0);
    if ux.notice != session.notice {
        ux.notice.clone_from(&session.notice);
        ux.notice_remaining = if ["reward:", "fountain", "Troll calls", "Level "]
            .iter()
            .any(|needle| session.notice.contains(needle))
        {
            3.0
        } else {
            0.0
        };
    }
    let feedback = session.combat_feedback(&tuning);
    if let Some(hit) = feedback.hit {
        if hit.sequence != ux.hit_sequence {
            ux.hit_sequence = hit.sequence;
            ux.flash = 0.16;
        }
    }
    ux.flash = (ux.flash - time.delta_secs()).max(0.0);
    for mut c in &mut reticles {
        c.set_if_neq(TextColor(if ux.flash > 0.0 {
            Color::srgb(1.0, 0.8, 0.3)
        } else {
            Color::WHITE
        }));
    }
    for (fill, mut node, mut color) in &mut fills {
        let Some(s) = feedback.spells.get(fill.0) else {
            continue;
        };
        let (fraction, c) = match s.state {
            SpellAvailabilityState::Ready => (1.0, Color::srgb(0.4, 0.9, 0.8)),
            SpellAvailabilityState::CoolingDown => (
                (1.0 - s.cooldown_remaining / s.cooldown_total.max(0.001)).clamp(0.0, 1.0),
                Color::srgb(0.5, 0.65, 0.8),
            ),
            SpellAvailabilityState::Charging => (s.charge_fraction, Color::srgb(1.0, 0.75, 0.32)),
            SpellAvailabilityState::Unavailable => (0.0, Color::srgb(0.35, 0.4, 0.46)),
        };
        let width = percent(fraction * 100.0);
        if node.width != width {
            node.width = width;
        }
        color.set_if_neq(BackgroundColor(c));
    }
    for (icon, mut c) in &mut icons {
        let next = if feedback
            .spells
            .get(icon.0)
            .is_some_and(|spell| spell.state == SpellAvailabilityState::CoolingDown)
        {
            Color::srgb(0.5, 0.56, 0.62)
        } else {
            Color::WHITE
        };
        if c.color != next {
            c.color = next;
        }
    }
}
fn present_menus(
    ux: Res<UxState>,
    state: Res<ViewState>,
    session: Res<ArenaSession>,
    tuning: Res<ArenaTuning>,
    recorder: Option<Res<Recorder>>,
    mut labels: Query<(&UxLabel, &mut Text, &mut Node), Without<PageBody>>,
    mut pages: Query<(&PageBody, &mut Node), Without<UxLabel>>,
    mut scale: ResMut<UiScale>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut borders: Query<(Entity, &mut BorderColor, Option<&UxAction>), With<Button>>,
) {
    let height = windows.single().map_or(1080.0, Window::height);
    let next_scale = (height / 1080.0).clamp(2.0 / 3.0, 2.0) * ux.scale;
    if scale.0.to_bits() != next_scale.to_bits() {
        scale.0 = next_scale;
    }
    for (page, mut node) in &mut pages {
        set_display(
            &mut node,
            if ux.page == page.0 {
                Display::Flex
            } else {
                Display::None
            },
        );
    }
    let feedback = session.combat_feedback(&tuning);
    for (label, mut text, mut node) in &mut labels {
        let next = match label {
            UxLabel::Notice => {
                set_display(
                    &mut node,
                    if ux.notice_remaining > 0.0 {
                        Display::Flex
                    } else {
                        Display::None
                    },
                );
                ux.notice.clone()
            }
            UxLabel::Target => feedback.target.as_ref().map_or(String::new(), |t| {
                let (pips, condition) = match t.health_pips {
                    3 => ("● ● ●", "Healthy"),
                    2 => ("● ● ○", "Wounded"),
                    _ => ("● ○ ○", "Critical"),
                };
                let name = match t.role {
                    Some(hex_arena::ExpeditionRole::BabyGoblin) => "Baby goblin".into(),
                    Some(hex_arena::ExpeditionRole::Troll) => "Troll".into(),
                    _ => format!("{:?}", t.species),
                };
                format!("{name}  {pips}  {condition}")
            }),
            UxLabel::Recording => {
                set_display(
                    &mut node,
                    if recorder.as_ref().is_some_and(|r| r.is_recording()) {
                        Display::Flex
                    } else {
                        Display::None
                    },
                );
                recorder.as_ref().map_or(String::new(), |r| {
                    let secs = std::time::Duration::from_secs_f64(r.elapsed_seconds()).as_secs();
                    format!("● REC  {:02}:{:02}", secs / 60, secs % 60)
                })
            }
            UxLabel::RecordButton => {
                recorder
                    .as_ref()
                    .map_or("RECORDING UNAVAILABLE".into(), |r| {
                        if r.is_starting() {
                            "STARTING…".into()
                        } else if r.is_finalizing() {
                            "SAVING…".into()
                        } else if r.is_recording() {
                            "STOP RECORDING".into()
                        } else if r.can_record() {
                            "START RECORDING".into()
                        } else {
                            "RECORDING UNAVAILABLE".into()
                        }
                    })
            }
            UxLabel::RecorderDetail => recorder.as_ref().map_or(String::new(), |r| {
                let status = r.status_text();
                if status.chars().count() > 85 {
                    "Recording details are in Overview.".into()
                } else {
                    status.into()
                }
            }),
            UxLabel::RecorderFull => recorder
                .as_ref()
                .map_or(String::new(), |r| r.status_text().into()),
            UxLabel::Scale => format!("Interface size: {:.0}%", ux.scale * 100.0),
            UxLabel::MapSelection => {
                if let Some(m) = ux.selected_id.as_ref().and_then(|id| {
                    session
                        .discovered_landmarks()
                        .into_iter()
                        .find(|m| &m.id == id)
                }) {
                    format!(
                        "{}: {}",
                        landmark_name(m.kind),
                        if m.kind == LandmarkKind::Fountain {
                            if m.consumed {
                                "spent"
                            } else {
                                "charged — heals up to 40 HP"
                            }
                        } else if m.defeated {
                            "defeated"
                        } else {
                            "last seen here"
                        }
                    )
                } else if ux.selected.is_empty() {
                    "Look at landmarks to discover them.".into()
                } else {
                    ux.selected.clone()
                }
            }
            UxLabel::Pin => ux
                .pin
                .and_then(|p| {
                    session
                        .human_actor_id()
                        .and_then(|id| session.actors.iter().find(|a| a.id == id))
                        .map(|a| {
                            let delta = p - a.feet.xz();
                            let index = direction_octant(delta);
                            format!(
                                "◆ {}  {:.0} units",
                                ["N", "NE", "E", "SE", "S", "SW", "W", "NW"]
                                    .get(index)
                                    .copied()
                                    .unwrap_or("N"),
                                delta.length()
                            )
                        })
                })
                .unwrap_or_default(),
        };
        if text.0 != next {
            text.0 = next;
        }
    }
    for (entity, mut border, action) in &mut borders {
        if ux.focus == Some(entity) && state.paused {
            border.set_if_neq(BorderColor::all(Color::srgb(1.0, 0.8, 0.3)));
        } else if action
            .is_some_and(|action| matches!(action,UxAction::Page(page) if *page == ux.page))
        {
            border.set_if_neq(BorderColor::all(Color::srgb(0.4, 0.9, 0.8)));
        } else {
            border.set_if_neq(BorderColor::all(Color::NONE));
        }
    }
}

fn reflow(
    ux: Res<UxState>,
    mut panels: Query<&mut Node, (With<MenuPanel>, Without<MenuHelper>)>,
    mut helpers: Query<&mut Node, (With<MenuHelper>, Without<MenuPanel>)>,
) {
    let compact = ux.scale >= 1.75;
    for mut node in &mut panels {
        let padding = UiRect::all(px(if compact { 16 } else { 24 }));
        let gap = px(if compact { 8 } else { 14 });
        if node.padding != padding {
            node.padding = padding;
        }
        if node.row_gap != gap {
            node.row_gap = gap;
        }
    }
    for mut node in &mut helpers {
        set_display(
            &mut node,
            if compact {
                Display::None
            } else {
                Display::Flex
            },
        );
    }
}
fn present_scroll_hints(
    scrolls: Query<(&ComputedNode, &ScrollPosition, &ChildOf), With<MenuScroll>>,
    mut hints: Query<(&ChildOf, &mut Node, &mut Text), With<MenuScrollHint>>,
) {
    for (parent, mut node, mut text) in &mut hints {
        let scroll = scrolls
            .iter()
            .find(|(_, _, owner)| owner.parent() == parent.parent());
        let label = scroll.map_or("", |(computed, position, _)| {
            let maximum = (computed.content_size().y - computed.size().y).max(0.0)
                * computed.inverse_scale_factor();
            if maximum <= 2.0 || computed.size().y <= 0.0 {
                ""
            } else if position.0.y <= 2.0 {
                "Scroll for more ↓"
            } else if position.0.y >= maximum - 2.0 {
                "Scroll back ↑"
            } else {
                "Scroll ↑ ↓"
            }
        });
        set_display(
            &mut node,
            if label.is_empty() {
                Display::None
            } else {
                Display::Flex
            },
        );
        if text.0 != label {
            text.0 = label.into();
        }
    }
}
fn begin_timing(mut ux: ResMut<UxState>) {
    ux.timing_start = Some(std::time::Instant::now());
}
fn end_timing(mut ux: ResMut<UxState>) {
    if let Some(start) = ux.timing_start.take() {
        let micros = start.elapsed().as_secs_f64() * 1e6 + ux.observation_micros;
        ux.present_micros = micros;
        if ux.timing_samples.len() == 256 {
            ux.timing_samples.pop_front();
        }
        ux.timing_samples.push_back(micros);
    }
}

/// Avoid invalidating Bevy layout for unchanged visibility.
pub(super) fn set_display(node: &mut Mut<Node>, display: Display) {
    if node.display != display {
        node.display = display;
    }
}
