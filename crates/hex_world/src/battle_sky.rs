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
    /// Water fog at the actual camera; None preserves the ordinary atmosphere.
    pub underwater_color: Option<Vec3>,
}
impl Default for BattleSkyFrame {
    fn default() -> Self {
        Self {
            enabled: false,
            center: Vec3::ZERO,
            sun_direction: Vec3::Y,
            cloud_phase: 0.0,
            underwater_color: None,
        }
    }
}

/// Map-selected sky palette and scale, separate from camera/time frame inputs.
#[derive(Resource, Debug, Clone, Copy)]
pub struct BattleSkyProfile {
    /// Linear-looking artistic horizon RGB, consumed by the procedural shader.
    pub horizon_color: Vec3,
    /// Zenith RGB.
    pub zenith_color: Vec3,
    /// Cloud RGB.
    pub cloud_color: Vec3,
    /// Fractional procedural cloud coverage.
    pub cloud_coverage: f32,
    /// Keep clouds above the horizon for open-ocean views; legacy maps may mirror them.
    pub upper_hemisphere_clouds: bool,
    /// Camera-centered dome radius; keep beyond the map's farthest visible terrain.
    pub dome_radius: f32,
}
impl Default for BattleSkyProfile {
    fn default() -> Self {
        Self {
            horizon_color: Vec3::new(0.12, 0.32, 0.68),
            zenith_color: Vec3::new(0.025, 0.14, 0.50),
            cloud_color: Vec3::new(0.60, 0.68, 0.77),
            cloud_coverage: 0.22,
            upper_hemisphere_clouds: false,
            dome_radius: 1000.0,
        }
    }
}
impl BattleSkyProfile {
    /// Cold clear late-afternoon atmosphere for the distant northern islands.
    #[must_use]
    pub fn northern() -> Self {
        Self {
            horizon_color: Vec3::new(0.25, 0.39, 0.57),
            zenith_color: Vec3::new(0.055, 0.18, 0.38),
            cloud_color: Vec3::new(0.65, 0.69, 0.74),
            cloud_coverage: 0.18,
            upper_hemisphere_clouds: true,
            dome_radius: 20_000.0,
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
        .init_resource::<BattleSkyProfile>()
        .add_systems(Startup, spawn)
        .add_systems(
            PostUpdate,
            update.before(bevy::transform::TransformSystems::Propagate),
        );
}
fn parameters(frame: &BattleSkyFrame, profile: &BattleSkyProfile) -> SkyParams {
    SkyParams {
        horizon_color: profile.horizon_color,
        zenith_color: profile.zenith_color,
        cloud_color: profile.cloud_color,
        cloud_coverage: profile.cloud_coverage,
        hex_scale: 28.0,
        cloud_softness: 0.32,
        cloud_roundness: 0.65,
        cloud_noise: 0.55,
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
        underwater_color: frame.underwater_color.unwrap_or(Vec3::ZERO),
        underwater_strength: f32::from(u8::from(frame.underwater_color.is_some())),
        upper_hemisphere_clouds: if profile.upper_hemisphere_clouds {
            1.0
        } else {
            0.0
        },
    }
}
fn spawn(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SkyMaterial>>,
    profile: Res<BattleSkyProfile>,
) {
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(1.0).mesh().uv(48, 32))),
        MeshMaterial3d(materials.add(SkyMaterial {
            params: parameters(&BattleSkyFrame::default(), &profile),
        })),
        Transform::from_scale(Vec3::splat(profile.dome_radius)),
        Visibility::Hidden,
        NotShadowCaster,
        Pickable::IGNORE,
        BattleSkyDome,
        Name::new("Expedition sky"),
    ));
}
fn update(
    frame: Res<BattleSkyFrame>,
    profile: Res<BattleSkyProfile>,
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
        transform.scale = Vec3::splat(profile.dome_radius);
        if let Some(mut material) = materials.get_mut(&handle.0) {
            material.params = parameters(&frame, &profile);
        }
    }
}
