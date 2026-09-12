//! Reusable procedural-sky presentation without tactical camera or lighting plugins.
use crate::sky_material::{SkyMaterial, SkyParams};
use bevy::light::NotShadowCaster;
use bevy::pbr::MaterialPlugin;
use bevy::prelude::*;

/// Presentation facts for a camera-centered sky with a fixed sun and moving clouds.
#[derive(Resource, Debug, Clone, Copy)]
pub struct BattleSkyFrame {
    /// False outside the environment that requests this sky.
    pub enabled: bool,
    /// Actual camera position; this never grants collision or observation facts.
    pub center: Vec3,
    /// Normalized direction toward the visible sun, matching the direct light.
    pub sun_direction: Vec3,
    /// Cloud animation seconds; a constant freezes deterministic review captures.
    pub cloud_phase: f32,
}
impl Default for BattleSkyFrame {
    fn default() -> Self {
        Self {
            enabled: false,
            center: Vec3::ZERO,
            sun_direction: Vec3::Y,
            cloud_phase: 0.0,
        }
    }
}
#[derive(Component)]
struct BattleSkyDome;

/// Installs only the procedural sky material and its disposable dome.
pub fn install(app: &mut App) {
    if !app.is_plugin_added::<MaterialPlugin<SkyMaterial>>() {
        crate::sky_material::plugin(app);
    }
    app.init_resource::<BattleSkyFrame>()
        .add_systems(Startup, spawn)
        .add_systems(
            PostUpdate,
            update.before(bevy::transform::TransformSystems::Propagate),
        );
}
fn parameters(frame: &BattleSkyFrame) -> SkyParams {
    SkyParams {
        horizon_color: Vec3::new(0.35, 0.48, 0.63),
        zenith_color: Vec3::new(0.11, 0.27, 0.48),
        cloud_color: Vec3::new(0.78, 0.83, 0.90),
        cloud_coverage: 0.38,
        hex_scale: 10.0,
        cloud_softness: 0.20,
        cloud_roundness: 0.65,
        cloud_noise: 0.30,
        sun_direction: frame.sun_direction.normalize_or(Vec3::Y),
        celestial_bodies_enabled: 1.0,
        sun_disc_color: Vec3::new(1.0, 0.87, 0.61),
        sun_angular_radius_radians: 0.015,
        sun_halo_width_radians: 0.065,
        sun_halo_strength: 0.20,
        moon_direction: Vec3::NEG_Y,
        moon_disc_color: Vec3::ZERO,
        moon_angular_radius_radians: 0.0,
        moon_halo_width_radians: 0.0,
        moon_halo_strength: 0.0,
        lower_glow_direction: Vec3::NEG_Y,
        lower_glow_color: Vec3::ZERO,
        lower_glow_angular_radius_radians: 0.0,
        lower_glow_strength: 0.0,
        cloud_phase_seconds: frame.cloud_phase,
    }
}
fn spawn(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SkyMaterial>>,
) {
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(1.0).mesh().uv(48, 32))),
        MeshMaterial3d(materials.add(SkyMaterial {
            params: parameters(&BattleSkyFrame::default()),
        })),
        Transform::from_scale(Vec3::splat(1000.0)),
        Visibility::Hidden,
        NotShadowCaster,
        Pickable::IGNORE,
        BattleSkyDome,
        Name::new("Expedition sky"),
    ));
}
fn update(
    frame: Res<BattleSkyFrame>,
    mut domes: Query<
        (
            &mut Transform,
            &mut Visibility,
            &MeshMaterial3d<SkyMaterial>,
        ),
        With<BattleSkyDome>,
    >,
    mut materials: ResMut<Assets<SkyMaterial>>,
) {
    for (mut transform, mut visibility, handle) in &mut domes {
        *visibility = if frame.enabled {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        transform.translation = frame.center;
        if let Some(material) = materials.get_mut(&handle.0) {
            material.params = parameters(&frame);
        }
    }
}
