//! Geographic summary image and party markers; no fine terrain reads or simulation.

use std::collections::BTreeMap;

use bevy::asset::RenderAssetUsages;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use hex_world_contracts::{WorldHex, WorldManifest};

use super::{
    knowledge::{PrincipalView, WorldKnowledge},
    Options, ResidentWorld, Session,
};

const SIZE: u32 = 1024;

#[derive(Resource, Default)]
pub(super) struct AtlasState {
    pub visible: bool,
    zoom: f32,
    pan: Vec2,
    projection: Option<AtlasProjection>,
    base: Option<Image>,
    texture: Option<Handle<Image>>,
    knowledge_stamp: Option<(String, u64)>,
}

#[derive(Component)]
pub(super) struct AtlasRoot;
#[derive(Component)]
pub(super) struct AtlasImage;
#[derive(Component)]
pub(super) struct AtlasMarker(usize);
#[derive(Component)]
pub(super) struct MiniMarker(usize);

#[derive(Component)]
pub(super) struct AtlasLegend;

/// Filter only IDs already present in this principal's memory. This lookup does
/// not turn the public registry into a list of discovered features.
pub(super) fn is_summary_landmark(manifest: &WorldManifest, id: &str) -> bool {
    manifest
        .features
        .binary_search_by(|feature| feature.id.as_str().cmp(id))
        .ok()
        .and_then(|index| manifest.features.get(index))
        .is_some_and(|feature| {
            matches!(
                feature.kind.as_str(),
                "entry" | "transit" | "gameplay-anchor" | "observation" | "ruin"
            )
        })
}

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
                (hex_world_contracts::SUMMARY_SAMPLE_PITCH as f64 / projection.side
                    * f64::from(SIZE))
                .ceil() as i32,
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

fn private_overlay(
    mut image: Image,
    manifest: &WorldManifest,
    projection: &AtlasProjection,
    view: &PrincipalView,
) -> Image {
    let Some(data) = &mut image.data else {
        return image;
    };
    // Compact discovery masks are permitted to span the known map. No dormant
    // fine terrain or another principal's memory is loaded for this overlay.
    for chunk in view.discovered_chunks() {
        let Ok(origin) = chunk.origin() else {
            continue;
        };
        for q in 0..16 {
            for r in 0..16 {
                let Ok(column) = origin.checked_add(WorldHex::new(q, r)) else {
                    continue;
                };
                if view.discovered(column) {
                    mark_pixel(
                        data,
                        projection.relative(column),
                        0,
                        [65, 192, 194, 255],
                        true,
                    );
                }
            }
        }
    }
    for landmark in view
        .landmarks
        .values()
        .filter(|landmark| is_summary_landmark(manifest, &landmark.id))
    {
        mark_pixel(
            data,
            projection.relative(landmark.position.column),
            2,
            [247, 234, 154, 255],
            false,
        );
    }
    image
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "only normalized atlas presentation coordinates become bounded pixel indices"
)]
fn mark_pixel(data: &mut [u8], point: Vec2, radius: i32, color: [u8; 4], blend: bool) {
    if !point.is_finite() {
        return;
    }
    let x = (point.x * SIZE as f32) as i32;
    let y = (point.y * SIZE as f32) as i32;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let (Ok(px), Ok(py)) = (u32::try_from(x + dx), u32::try_from(y + dy)) else {
                continue;
            };
            if px >= SIZE || py >= SIZE {
                continue;
            }
            let Ok(index) = usize::try_from((py * SIZE + px) * 4) else {
                continue;
            };
            if let Some(pixel) = data.get_mut(index..index + 4) {
                for (value, tint) in pixel.iter_mut().zip(color) {
                    *value = if blend {
                        u8::try_from((u16::from(*value) + u16::from(tint)) / 2).unwrap_or(255)
                    } else {
                        tint
                    };
                }
            }
        }
    }
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
    let base = image(runtime.0.manifest(), &projection);
    let texture = images.add(base.clone());
    state.base = Some(base);
    state.texture = Some(texture.clone());
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
                Text::new("World geography | Drag: pan | Scroll: zoom | M: close\nPrivate exploration loading..."),
                TextFont {
                    font_size: FontSize::Px(17.0),
                    ..default()
                },
                TextColor(Color::WHITE),
                BackgroundColor(Color::srgba(0.025, 0.05, 0.06, 0.9)),
                AtlasLegend,
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
    runtime: Res<ResidentWorld>,
    knowledge: Res<WorldKnowledge>,
    mut images: ResMut<Assets<Image>>,
    mut legend: Query<&mut Text, With<AtlasLegend>>,
    mut state: ResMut<AtlasState>,
    mut queries: ParamSet<(
        Query<(&mut Node, &ComputedNode), With<AtlasRoot>>,
        Query<&mut Node, With<AtlasImage>>,
        Query<(&mut Node, &AtlasMarker)>,
        Query<(&mut Node, &MiniMarker)>,
    )>,
) {
    let selected = knowledge.selected(&session);
    let stamp = selected.map(|view| (view.principal.clone(), view.revision));
    if state.knowledge_stamp != stamp {
        if let (Some(base), Some(texture), Some(projection)) =
            (&state.base, &state.texture, &state.projection)
        {
            let updated = selected.map_or_else(
                || base.clone(),
                |view| private_overlay(base.clone(), runtime.0.manifest(), projection, view),
            );
            if let Some(image) = images.get_mut(texture) {
                *image = updated;
            }
        }
        state.knowledge_stamp = stamp;
    }
    if let Ok(mut text) = legend.single_mut() {
        **text = selected.map_or_else(|| "World geography | Private exploration loading...".into(), |view| {
            let landmarks = view.landmarks.values().filter(|landmark| is_summary_landmark(runtime.0.manifest(), &landmark.id)).count();
            format!("World geography | Drag: pan | Scroll: zoom | M: close\n{}: {} explored columns | {} known landmarks{}\nCyan: private exploration | Yellow: known landmarks", view.principal, view.discovered_column_count(), landmarks, if view.landmark_catalogue_complete { "" } else { " (nearby restore)" })
        });
    }
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
        node.display = if marker.0 == session.selected {
            Display::Flex
        } else {
            Display::None
        };
        if let Some(actor) = session.actors.get(marker.0) {
            let point = projection.relative(actor.column) * size;
            node.left = px(point.x - 5.0);
            node.top = px(point.y - 5.0);
        }
    }
    for (mut node, marker) in &mut queries.p3() {
        node.display = if marker.0 == session.selected {
            Display::Flex
        } else {
            Display::None
        };
        if let Some(actor) = session.actors.get(marker.0) {
            let point = projection.relative(actor.column) * 190.0;
            node.left = px(point.x - 3.5);
            node.top = px(point.y - 3.5);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exploration_tint_preserves_opaque_target_and_outside_markers_do_not_write() {
        let mut data = vec![100; usize::try_from(SIZE * SIZE * 4).expect("bounded atlas")];
        for pixel in data.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[100, 100, 100, 255]);
        }
        mark_pixel(&mut data, Vec2::splat(0.5), 0, [60, 200, 220, 255], true);
        assert!(data.chunks_exact(4).all(|pixel| pixel.last() == Some(&255)));
        assert!(data
            .chunks_exact(4)
            .any(|pixel| pixel == [80, 150, 160, 255]));
        let before = data.clone();
        mark_pixel(&mut data, Vec2::splat(-10.0), 2, [255; 4], false);
        mark_pixel(&mut data, Vec2::splat(f32::NAN), 2, [255; 4], false);
        assert_eq!(before, data);
    }
}
