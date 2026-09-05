//! Geographic summary image and party markers; no fine terrain reads or simulation.

use std::collections::BTreeMap;

use bevy::asset::RenderAssetUsages;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use hex_world_contracts::{WorldHex, WorldManifest};

use super::{Options, ResidentWorld, Session};

const SIZE: u32 = 1024;

#[derive(Resource, Default)]
pub(super) struct AtlasState {
    pub visible: bool,
    zoom: f32,
    pan: Vec2,
    projection: Option<AtlasProjection>,
}

#[derive(Component)]
pub(super) struct AtlasRoot;
#[derive(Component)]
pub(super) struct AtlasImage;
#[derive(Component)]
pub(super) struct AtlasMarker(usize);
#[derive(Component)]
pub(super) struct MiniMarker(usize);

#[derive(Clone)]
struct AtlasProjection {
    origin: WorldHex,
    min: (f64, f64),
    side: f64,
}

impl AtlasProjection {
    fn new(manifest: &WorldManifest) -> Option<Self> {
        let origin = manifest.regions.first()?.origin;
        let mut min = (f64::INFINITY, f64::INFINITY);
        let mut max = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for region in &manifest.regions {
            let (x, y) = local_xy(origin, region.origin);
            let radius = f64::from(region.radius) * 1.75;
            min = (min.0.min(x - radius), min.1.min(y - radius));
            max = (max.0.max(x + radius), max.1.max(y + radius));
        }
        let side = (max.0 - min.0).max(max.1 - min.1) * 1.08;
        Some(Self {
            origin,
            min: ((max.0 + min.0 - side) / 2.0, (max.1 + min.1 - side) / 2.0),
            side,
        })
    }

    fn relative(&self, column: WorldHex) -> Vec2 {
        let (x, y) = local_xy(self.origin, column);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "only normalized presentation positions become floats"
        )]
        Vec2::new(
            ((x - self.min.0) / self.side) as f32,
            ((y - self.min.1) / self.side) as f32,
        )
    }
}

fn local_xy(origin: WorldHex, position: WorldHex) -> (f64, f64) {
    #[expect(
        clippy::cast_precision_loss,
        reason = "subtract exact integer origin before converting geographic presentation offsets"
    )]
    let (q, r) = (
        (i128::from(position.q) - i128::from(origin.q)) as f64,
        (i128::from(position.r) - i128::from(origin.r)) as f64,
    );
    (3_f64.sqrt() * (q + r / 2.0), 1.5 * r)
}

fn image(manifest: &WorldManifest, projection: &AtlasProjection) -> Image {
    let colors = manifest
        .materials
        .iter()
        .map(|material| (material.id.as_str(), material.color))
        .collect::<BTreeMap<_, _>>();
    let mut image = Image::new_fill(
        Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[16, 27, 30, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    if let Some(data) = &mut image.data {
        for sample in &manifest.summary {
            let normalized = projection.relative(sample.position);
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_precision_loss,
                reason = "bounded raster dimensions and normalized geographic coordinates"
            )]
            let (x, y, radius) = (
                (normalized.x * SIZE as f32) as i32,
                (normalized.y * SIZE as f32) as i32,
                (12.0 / projection.side * f64::from(SIZE)).ceil() as i32,
            );
            let Some(color) = colors.get(sample.material.as_str()) else {
                continue;
            };
            let [red, green, blue, _alpha] = *color;
            let radius = radius.clamp(1, 1024);
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    let (px, py) = (x + dx, y + dy);
                    let (Ok(px), Ok(py)) = (u32::try_from(px), u32::try_from(py)) else {
                        continue;
                    };
                    if px >= SIZE || py >= SIZE {
                        continue;
                    }
                    let Some(index) = usize::try_from((py * SIZE + px) * 4).ok() else {
                        continue;
                    };
                    if let Some(pixel) = data.get_mut(index..index + 4) {
                        pixel.copy_from_slice(&[red, green, blue, 255]);
                    }
                }
            }
        }
    }
    image
}

pub(super) fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    runtime: Res<ResidentWorld>,
    options: Res<Options>,
    session: Res<Session>,
    mut state: ResMut<AtlasState>,
) {
    let Some(projection) = AtlasProjection::new(runtime.0.manifest()) else {
        return;
    };
    let texture = images.add(image(runtime.0.manifest(), &projection));
    state.visible = options.view == "atlas";
    state.zoom = 1.0;
    state.projection = Some(projection);
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: px(190),
                height: px(190),
                right: px(18),
                bottom: px(18),
                border: UiRect::all(px(2)),
                overflow: Overflow::clip(),
                ..default()
            },
            BorderColor::all(Color::srgba(0.8, 0.85, 0.8, 0.6)),
            ImageNode::new(texture.clone()),
        ))
        .with_children(|parent| {
            for index in 0..session.actors.len() {
                parent.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        width: px(7),
                        height: px(7),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(1.0, 0.8, 0.3)),
                    MiniMarker(index),
                ));
            }
        });
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: percent(24),
                top: percent(10),
                width: percent(66),
                height: percent(82),
                overflow: Overflow::clip(),
                display: if state.visible {
                    Display::Flex
                } else {
                    Display::None
                },
                ..default()
            },
            BackgroundColor(Color::srgb(0.04, 0.07, 0.08)),
            AtlasRoot,
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    ImageNode::new(texture),
                    Node {
                        position_type: PositionType::Absolute,
                        width: px(720),
                        height: px(720),
                        ..default()
                    },
                    AtlasImage,
                ))
                .with_children(|image| {
                    for index in 0..session.actors.len() {
                        image.spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                width: px(10),
                                height: px(10),
                                ..default()
                            },
                            BackgroundColor(Color::srgb(1.0, 0.8, 0.3)),
                            AtlasMarker(index),
                        ));
                    }
                });
            parent.spawn((
                Text::new("World atlas   ·   Drag to pan   ·   Scroll to zoom   ·   M to close"),
                TextFont {
                    font_size: 17.0,
                    ..default()
                },
                TextColor(Color::WHITE),
                BackgroundColor(Color::srgba(0.025, 0.05, 0.06, 0.9)),
                Node {
                    position_type: PositionType::Absolute,
                    left: px(14),
                    top: px(14),
                    padding: UiRect::all(px(10)),
                    ..default()
                },
            ));
        });
}

pub(super) fn update(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    session: Res<Session>,
    mut state: ResMut<AtlasState>,
    mut queries: ParamSet<(
        Query<(&mut Node, &ComputedNode), With<AtlasRoot>>,
        Query<&mut Node, With<AtlasImage>>,
        Query<(&mut Node, &AtlasMarker)>,
        Query<(&mut Node, &MiniMarker)>,
    )>,
) {
    if keys.just_pressed(KeyCode::KeyM) {
        state.visible = !state.visible;
    }
    if state.visible {
        if mouse.pressed(MouseButton::Left) {
            state.pan += motion.delta;
        }
        state.zoom = (state.zoom * (scroll.delta.y * 0.12).exp()).clamp(0.5, 12.0);
    }
    let mut viewport = Vec2::splat(720.0);
    for (mut node, computed) in &mut queries.p0() {
        node.display = if state.visible {
            Display::Flex
        } else {
            Display::None
        };
        if computed.size().min_element() > 0.0 {
            viewport = computed.size() * computed.inverse_scale_factor();
        }
    }
    let size = viewport.min_element() * state.zoom;
    for mut node in &mut queries.p1() {
        node.width = px(size);
        node.height = px(size);
        node.left = px((viewport.x - size) / 2.0 + state.pan.x);
        node.top = px((viewport.y - size) / 2.0 + state.pan.y);
    }
    let Some(projection) = &state.projection else {
        return;
    };
    for (mut node, marker) in &mut queries.p2() {
        if let Some(actor) = session.actors.get(marker.0) {
            let point = projection.relative(actor.column) * size;
            node.left = px(point.x - 5.0);
            node.top = px(point.y - 5.0);
        }
    }
    for (mut node, marker) in &mut queries.p3() {
        if let Some(actor) = session.actors.get(marker.0) {
            let point = projection.relative(actor.column) * 190.0;
            node.left = px(point.x - 3.5);
            node.top = px(point.y - 3.5);
        }
    }
}
