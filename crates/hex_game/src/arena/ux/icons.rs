//! Small code-authored, antialiased spell silhouettes; no font glyph dependency.
use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

fn inside(x: f32, y: f32, polygon: &[(f32, f32)]) -> bool {
    let mut hit = false;
    for (a, b) in polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
    {
        if (a.1 > y) != (b.1 > y) && x < (b.0 - a.0) * (y - a.1) / (b.1 - a.1) + a.0 {
            hit = !hit;
        }
    }
    hit
}
pub(in crate::arena) fn create(images: &mut Assets<Image>, slot: usize) -> Handle<Image> {
    let outline: &[(f32, f32)] = match slot {
        0 => &[
            (10., 7.),
            (54., 7.),
            (51., 37.),
            (44., 48.),
            (32., 58.),
            (20., 48.),
            (13., 37.),
        ],
        1 => &[
            (30., 3.),
            (29., 22.),
            (43., 12.),
            (46., 29.),
            (55., 36.),
            (56., 45.),
            (49., 56.),
            (31., 61.),
            (17., 54.),
            (10., 42.),
            (13., 30.),
            (21., 19.),
            (20., 38.),
            (27., 30.),
        ],
        _ => &[
            (32., 5.),
            (10., 28.),
            (24., 28.),
            (24., 48.),
            (40., 48.),
            (40., 28.),
            (54., 28.),
        ],
    };
    let inner: &[(f32, f32)] = match slot {
        0 => &[
            (17., 14.),
            (47., 14.),
            (44., 36.),
            (39., 43.),
            (32., 49.),
            (25., 43.),
            (20., 36.),
        ],
        1 => &[
            (34., 29.),
            (38., 42.),
            (44., 36.),
            (46., 47.),
            (39., 55.),
            (28., 55.),
            (23., 48.),
            (27., 40.),
        ],
        _ => &[],
    };
    let mut rgba = Vec::with_capacity(64 * 64 * 4);
    for y in 0_u16..64 {
        for x in 0_u16..64 {
            let mut sum = [0_u32; 4];
            for oy in [0.25, 0.75] {
                for ox in [0.25, 0.75] {
                    let (px, py) = (f32::from(x) + ox, f32::from(y) + oy);
                    let filled = inside(px, py, outline)
                        || (slot == 2 && (53.0..58.0).contains(&py) && (18.0..46.0).contains(&px));
                    let color = if !filled {
                        [0, 0, 0, 0]
                    } else if inside(px, py, inner) {
                        if slot == 0 {
                            [25, 74, 83, 255]
                        } else {
                            [255, 239, 163, 255]
                        }
                    } else {
                        match slot {
                            0 => [110, 230, 235, 255],
                            1 => [255, 148, 64, 255],
                            _ => [195, 194, 255, 255],
                        }
                    };
                    for (sum, value) in sum.iter_mut().zip(color) {
                        *sum += value;
                    }
                }
            }
            rgba.extend(sum.map(|value| u8::try_from(value / 4).unwrap_or(255)));
        }
    }
    images.add(Image::new(
        Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    ))
}
