//! Grand-only steady travelling highlights on unchanged authored liquid prisms.
use bevy::{
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::ShaderRef,
};
use hex_world_contracts::{ChunkPackage, LiquidKind, VoxelRun, WorldHex};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Style {
    Current,
    Rapid,
    Fall,
}

pub(super) fn style(package: &ChunkPackage, position: WorldHex, run: &VoxelRun) -> Option<Style> {
    if package.world_id != "grand-v4" || run.material != "water" {
        return None;
    }
    let liquid = package.semantics.liquids.iter().find(|liquid| {
        liquid.column == position && liquid.bottom == run.bottom && liquid.top == run.top
    })?;
    let downstream = liquid.downstream.first()?;
    match liquid.kind {
        LiquidKind::Standing => None,
        LiquidKind::Waterfall => Some(Style::Fall),
        LiquidKind::Directed => Some(if liquid.top - 1 - downstream.level >= 2 {
            Style::Rapid
        } else {
            Style::Current
        }),
    }
}

#[derive(Clone, Copy, ShaderType)]
struct Parameters {
    phase_foam_fall: Vec4,
    direction: Vec4,
}

#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub(super) struct Extension {
    #[uniform(100)]
    parameters: Parameters,
}
impl MaterialExtension for Extension {
    fn fragment_shader() -> ShaderRef {
        "shaders/grand_river.wgsl".into()
    }
}
pub(super) type RiverMaterial = ExtendedMaterial<StandardMaterial, Extension>;

#[derive(Resource, Default)]
pub(super) struct Enabled;

pub(crate) fn install(app: &mut App) {
    app.add_plugins(MaterialPlugin::<RiverMaterial>::default())
        .init_resource::<Enabled>()
        .add_systems(PostUpdate, update);
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "The f64 saved clock is reduced to one bounded four-second phase before casting."
)]
fn phase(seconds: f64) -> f32 {
    (seconds.rem_euclid(4.0) / 4.0) as f32
}

pub(super) fn material(style: Style, color: Color, seconds: f64) -> RiverMaterial {
    let [x, z] = hex_schematic::v4::grand::RIVER_PHASE_DIRECTION;
    RiverMaterial {
        base: StandardMaterial {
            base_color: color,
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 0.75,
            ..default()
        },
        extension: Extension {
            parameters: Parameters {
                phase_foam_fall: Vec4::new(
                    phase(seconds),
                    match style {
                        Style::Current => 0.07,
                        Style::Rapid => 0.22,
                        Style::Fall => 0.52,
                    },
                    if style == Style::Fall { 1.0 } else { 0.0 },
                    6.0,
                ),
                direction: Vec4::new(x, z, 0.0, 0.0),
            },
        },
    }
}

fn update(
    frame: Option<Res<crate::ocean::OceanFrame>>,
    mut materials: ResMut<Assets<RiverMaterial>>,
) {
    let Some(frame) = frame.filter(|frame| frame.enabled) else {
        return;
    };
    let time = frame.time();
    if !time.seconds.is_finite() {
        return;
    }
    let phase = phase(time.seconds);
    for (_, material) in materials.iter_mut() {
        material.extension.parameters.phase_foam_fall.x = phase;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_saved_clock_keeps_phase_after_sixty_hours_and_wraps_continuously() {
        assert_eq!(phase(216_000.25).to_bits(), phase(0.25).to_bits());
        assert!((phase(3.999_999) - phase(4.000_001)).abs() > 0.99);
        let sample =
            |s: f32, seconds: f64| ((s / 6.0 - phase(seconds)) * std::f32::consts::TAU).sin();
        assert!((sample(1.0, 3.999_999) - sample(1.0, 4.000_001)).abs() < 0.000_01);
        assert!((sample(1.0, 0.0) - sample(2.5, 1.0)).abs() < 0.000_01);
    }
}
