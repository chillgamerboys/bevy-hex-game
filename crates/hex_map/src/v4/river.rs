//! Grand-only steady travelling highlights on unchanged authored liquid prisms.
use bevy::{
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::ShaderRef,
};
use hex_world_contracts::{ChunkPackage, LiquidKind, VoxelRun, WorldHex};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Style {
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
    liquid_style(liquid.kind, liquid.top, liquid.downstream.first().copied())
}

/// One style selection for exact detailed and distant authored liquid faces.
pub(crate) fn liquid_style(
    kind: LiquidKind,
    top: i32,
    downstream: Option<hex_world_contracts::VoxelPosition>,
) -> Option<Style> {
    let downstream = downstream?;
    match kind {
        LiquidKind::Standing => None,
        LiquidKind::Waterfall => Some(Style::Fall),
        LiquidKind::Directed => Some(if i64::from(top) - 1 - i64::from(downstream.level) >= 2 {
            Style::Rapid
        } else {
            Style::Current
        }),
    }
}

#[derive(Clone, Copy, ShaderType)]
struct Parameters {
    phase_foam_fall: Vec4,
    // Horizontal authored direction, continuous 3D chart origin phase, reserved.
    direction: Vec4,
}

#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub(crate) struct Extension {
    #[uniform(100)]
    parameters: Parameters,
}
impl MaterialExtension for Extension {
    fn fragment_shader() -> ShaderRef {
        "shaders/grand_river.wgsl".into()
    }
}
pub(crate) type RiverMaterial = ExtendedMaterial<StandardMaterial, Extension>;

#[derive(Resource, Default)]
pub(crate) struct Enabled;

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

#[expect(
    clippy::cast_possible_truncation,
    reason = "Absolute authored coordinates are reduced to one wavelength in f64 before f32 uniforms."
)]
pub(crate) fn set_origin(
    material: &mut RiverMaterial,
    origin: super::RenderOrigin,
    level_height: f32,
) {
    let [x, z] = hex_schematic::v4::northern::world_xz(origin.column);
    let [dx, dz] = hex_schematic::v4::grand::RIVER_PHASE_DIRECTION;
    // One chart covers caps and sides. Separate horizontal and height phases
    // would make the same lip/base position change phase with its face normal.
    let chart =
        x * f64::from(dx) + z * f64::from(dz) - f64::from(origin.level) * f64::from(level_height);
    material.extension.parameters.direction.z = (chart / 6.0).rem_euclid(1.0) as f32;
}

pub(crate) fn material(style: Style, color: Color, seconds: f64) -> RiverMaterial {
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
                    phase(if seconds.is_finite() { seconds } else { 0.0 }),
                    match style {
                        Style::Current => 0.07,
                        Style::Rapid => 0.22,
                        Style::Fall => 0.52,
                    },
                    0.0,
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
    use bevy::ecs::system::RunSystemOnce;
    use hex_core::ocean::OceanSimulationTime;
    #[test]
    fn shared_saved_clock_keeps_phase_after_sixty_hours_and_wraps_continuously() {
        assert_eq!(phase(216_000.25).to_bits(), phase(0.25).to_bits());
        assert!((phase(3.999_999) - phase(4.000_001)).abs() > 0.99);
        let sample =
            |s: f32, seconds: f64| ((s / 6.0 - phase(seconds)) * std::f32::consts::TAU).sin();
        assert!((sample(1.0, 3.999_999) - sample(1.0, 4.000_001)).abs() < 0.000_01);
        assert!((sample(1.0, 0.0) - sample(2.5, 1.0)).abs() < 0.000_01);
    }

    #[test]
    fn rebase_preserves_lip_and_base_phase_across_flow_styles() {
        // Package04's first fall had a 0.261-cycle cap/side discontinuity at
        // this shared lip. Exercise both joins, several points across the face,
        // all flow styles and independent horizontal/vertical render rebases.
        let [center_x, center_z] = hex_schematic::v4::northern::world_xz(WorldHex::new(330, -246));
        let [target_x, target_z] = hex_schematic::v4::northern::world_xz(WorldHex::new(329, -245));
        let lip_x = (center_x + target_x) * 0.5;
        let lip_z = (center_z + target_z) * 0.5;
        let level_height = 0.35_f32;
        for seconds in [0.0, 3.999_999, 4.000_001, 216_000.25] {
            for level in [700, 620] {
                for across in [-0.4, 0.0, 0.4] {
                    let point = bevy::math::DVec3::new(
                        lip_x + across,
                        f64::from(level) * f64::from(level_height),
                        lip_z,
                    );
                    let [dx, dz] = hex_schematic::v4::grand::RIVER_PHASE_DIRECTION;
                    let expected = (point.x * f64::from(dx) + point.z * f64::from(dz) - point.y)
                        / 6.0
                        - f64::from(phase(seconds));
                    for origin in [
                        super::super::RenderOrigin::default(),
                        super::super::RenderOrigin {
                            column: WorldHex::new(330, -246),
                            level: 600,
                        },
                        super::super::RenderOrigin {
                            column: WorldHex::new(21, -37),
                            level: -120,
                        },
                    ] {
                        let [x, z] = hex_schematic::v4::northern::world_xz(origin.column);
                        let local = (point
                            - bevy::math::DVec3::new(
                                x,
                                f64::from(origin.level) * f64::from(level_height),
                                z,
                            ))
                        .as_vec3();
                        for style in [Style::Current, Style::Rapid, Style::Fall] {
                            let mut material = material(style, Color::WHITE, seconds);
                            set_origin(&mut material, origin, level_height);
                            let actual = shader_phase(&material.extension.parameters, local);
                            let error = (f64::from(actual) - expected + 0.5).rem_euclid(1.0) - 0.5;
                            assert!(error.abs() < 0.000_02, "shared join phase: {error}");
                        }
                    }
                }
            }
        }
    }

    // CPU contract for the shader inputs; GPU compilation and temporal review
    // remain separate acceptance. This has no face-normal/style phase switch.
    fn shader_phase(parameters: &Parameters, point: Vec3) -> f32 {
        (point.x * parameters.direction.x + point.z * parameters.direction.y - point.y)
            / parameters.phase_foam_fall.w
            + parameters.direction.z
            - parameters.phase_foam_fall.x
    }

    #[test]
    fn continuous_chart_moves_downstream_and_down_falls_without_changing_speed() {
        let at = Vec3::new(358.1, 245.0, -368.25);
        let current = material(Style::Fall, Color::WHITE, 0.0);
        let later = material(Style::Fall, Color::WHITE, 1.0);
        let direction = current.extension.parameters.direction;
        let downstream = Vec3::new(direction.x, 0.0, direction.y);
        let start_phase = shader_phase(&current.extension.parameters, at);
        // A crest advances 1.5 chart units per second on both level water and
        // vertical falling water. Compare unreduced phase so direction matters.
        for travel in [downstream * 1.5, Vec3::NEG_Y * 1.5] {
            let advanced = shader_phase(&current.extension.parameters, at + travel);
            assert!(advanced > start_phase);
            assert!(
                (shader_phase(&later.extension.parameters, at + travel) - start_phase).abs()
                    < 0.000_02
            );
        }
        // Main-channel progression uses the full 3D chart. Physical drainage
        // also permits lateral flow at flat bank tips; those edges need not
        // follow this global highlight direction.
        let mut admitted_directions = 0;
        for (q, r) in [(1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1)] {
            let [x, z] = hex_schematic::v4::northern::world_xz(WorldHex::new(q, r));
            let step = bevy::math::DVec3::new(x, 0.0, z).as_vec3();
            if step.dot(downstream) <= 0.0 {
                continue;
            }
            admitted_directions += 1;
            for drop in [0.0, 0.35, 28.0] {
                assert!(
                    shader_phase(
                        &current.extension.parameters,
                        at + step + Vec3::NEG_Y * drop,
                    ) > start_phase
                );
            }
        }
        assert_eq!(admitted_directions, 3);
        // A real bend on revision02 falls flows one hex west and seven levels
        // down. Rejecting horizontal reversal alone incorrectly strands it.
        let west_drop = Vec3::new(-3.0_f32.sqrt(), -7.0 * 0.35, 0.0);
        assert!(shader_phase(&current.extension.parameters, at + west_drop) > start_phase);
        let shader = include_str!("../../../../assets/shaders/grand_river.wgsl");
        assert!(
            !shader.contains("input.world_normal"),
            "the phase field must not switch at a cap/side normal boundary"
        );
    }

    #[test]
    #[expect(
        clippy::panic_in_result_fn,
        reason = "Assertions are test oracles; Result propagates only fixture and Bevy system errors."
    )]
    fn saved_clock_drives_all_styles_independently_of_camera_and_rejects_nonfinite_time()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut world = World::new();
        world.init_resource::<Assets<RiverMaterial>>();
        let handles: Vec<_> = [Style::Current, Style::Rapid, Style::Fall]
            .into_iter()
            .map(|style| {
                world.resource_mut::<Assets<RiverMaterial>>().add(material(
                    style,
                    Color::WHITE,
                    0.0,
                ))
            })
            .collect();
        world.insert_resource(crate::ocean::OceanFrame {
            enabled: true,
            simulation_time: Some(OceanSimulationTime {
                generation: 7,
                seconds: 216_000.25,
            }),
            ..default()
        });
        for camera in [Vec3::ZERO, Vec3::new(500.0, 200.0, -900.0)] {
            world
                .resource_mut::<crate::ocean::OceanFrame>()
                .camera_position = camera;
            world
                .run_system_once(update)
                .map_err(|error| format!("river update: {error:?}"))?;
            for handle in &handles {
                let material = world
                    .resource::<Assets<RiverMaterial>>()
                    .get(handle)
                    .ok_or("material")?;
                assert_eq!(
                    material.extension.parameters.phase_foam_fall.x.to_bits(),
                    phase(0.25).to_bits()
                );
            }
        }
        world
            .resource_mut::<crate::ocean::OceanFrame>()
            .simulation_time = Some(OceanSimulationTime {
            generation: 8,
            seconds: f64::NAN,
        });
        world
            .run_system_once(update)
            .map_err(|error| format!("river update: {error:?}"))?;
        for handle in &handles {
            let material = world
                .resource::<Assets<RiverMaterial>>()
                .get(handle)
                .ok_or("material")?;
            assert_eq!(
                material.extension.parameters.phase_foam_fall.x.to_bits(),
                phase(0.25).to_bits()
            );
        }
        Ok(())
    }
}
