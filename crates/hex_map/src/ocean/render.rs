use super::{mesh, OceanBathymetry, OceanFrame, OceanNearBoundary, OceanSurfaceProfile};
use bevy::{
    asset::RenderAssetUsages,
    camera::visibility::NoFrustumCulling,
    light::NotShadowCaster,
    material::OpaqueRendererMethod,
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::{AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat},
    shader::ShaderRef,
};

#[derive(Clone, Debug, ShaderType)]
struct OceanParams {
    water: Vec4,
    bath: Vec4,
    wave0: Vec4,
    wave1: Vec4,
    wave2: Vec4,
    periods: Vec4,
    phase_offsets: Vec4,
    shallow: Vec4,
    deep: Vec4,
}
#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
struct OceanExtension {
    #[uniform(100)]
    params: OceanParams,
    #[texture(
        101,
        sample_type = "float",
        filterable = false,
        visibility(vertex, fragment)
    )]
    bathymetry: Handle<Image>,
}
impl MaterialExtension for OceanExtension {
    fn vertex_shader() -> ShaderRef {
        "shaders/ocean.wgsl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/ocean.wgsl".into()
    }
}
type OceanMaterial = ExtendedMaterial<StandardMaterial, OceanExtension>;

/// Typed preparation/publication diagnostics; pixels and native motion are separate.
#[derive(Resource, Debug, Default)]
pub struct OceanRenderStatus {
    /// Current requested profile/grid has been admitted to presentation.
    pub ready: bool,
    /// Fixed surface topology vertex count.
    pub surface_vertices: usize,
    /// Current exact local boundary vertex count.
    pub boundary_vertices: usize,
    /// Accepted bathymetry revision.
    pub bathymetry_revision: Option<u64>,
    /// Invalid requested inputs; no partial invalid material is published.
    pub error: Option<String>,
}

#[derive(Resource, Default)]
struct Cache {
    profile: Option<OceanSurfaceProfile>,
    bed_revision: Option<u64>,
    boundary_revision: Option<u64>,
    material: Option<Handle<OceanMaterial>>,
    image: Option<Handle<Image>>,
    surface: Option<Entity>,
    boundary: Option<(Entity, Handle<Mesh>)>,
}
#[derive(Component)]
struct Surface;

/// Installs disposable ocean rendering; the host owns frame/profile/grid resources.
/// Call once during application composition, then update `OceanFrame` before
/// `PostUpdate`. No scene is spawned until the first valid enabled frame.
pub fn install(app: &mut App) {
    app.add_plugins(MaterialPlugin::<OceanMaterial>::default())
        .init_resource::<OceanSurfaceProfile>()
        .init_resource::<OceanBathymetry>()
        .init_resource::<OceanFrame>()
        .init_resource::<OceanNearBoundary>()
        .init_resource::<OceanRenderStatus>()
        .init_resource::<Cache>()
        .add_systems(
            PostUpdate,
            update.before(bevy::transform::TransformSystems::Propagate),
        );
}

fn parameters(profile: &OceanSurfaceProfile, bed: &OceanBathymetry, phase: f32) -> OceanParams {
    let waves = profile.waves.map(|wave| {
        let direction = wave.direction.normalize();
        Vec4::new(
            direction.x,
            direction.y,
            wave.amplitude,
            std::f32::consts::TAU / wave.wavelength,
        )
    });
    let [wave0, wave1, wave2] = waves;
    let [a, b, c] = profile.waves;
    OceanParams {
        water: Vec4::new(
            profile.mean_sea_level,
            phase,
            profile.shore_depth,
            mesh::HORIZON_RADIUS,
        ),
        bath: Vec4::new(bed.origin_xz.x, bed.origin_xz.y, bed.spacing, 0.0),
        wave0,
        wave1,
        wave2,
        periods: Vec4::new(
            std::f32::consts::TAU / a.period,
            std::f32::consts::TAU / b.period,
            std::f32::consts::TAU / c.period,
            0.0,
        ),
        phase_offsets: Vec4::new(a.phase_radians, b.phase_radians, c.phase_radians, 0.0),
        shallow: profile.shallow_color,
        deep: profile.deep_color,
    }
}

fn texture(bed: &OceanBathymetry) -> Image {
    Image::new(
        Extent3d {
            width: bed.width,
            height: bed.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        bed.bed_heights
            .iter()
            .flat_map(|height| height.to_le_bytes())
            .collect(),
        TextureFormat::R32Float,
        RenderAssetUsages::default(),
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "One renderer publication owns atomic profile, mesh, material and texture admission."
)]
fn update(
    mut commands: Commands,
    frame: Res<OceanFrame>,
    profile: Res<OceanSurfaceProfile>,
    bed: Res<OceanBathymetry>,
    boundary: Res<OceanNearBoundary>,
    mut cache: ResMut<Cache>,
    mut status: ResMut<OceanRenderStatus>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<OceanMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut surfaces: Query<&mut Transform, With<Surface>>,
) {
    for entity in cache
        .surface
        .into_iter()
        .chain(cache.boundary.as_ref().map(|(entity, _)| *entity))
    {
        commands.entity(entity).insert(if frame.enabled {
            Visibility::Visible
        } else {
            Visibility::Hidden
        });
    }
    if !frame.enabled {
        return;
    }
    if !frame.camera_position.is_finite()
        || !frame.phase_seconds.is_finite()
        || !profile.is_valid()
        || (cache.bed_revision != Some(bed.revision) && !bed.is_valid())
    {
        status.ready = false;
        status.error = Some(
            "Ocean requires finite profile, camera, phase and a complete bounded bathymetry grid."
                .into(),
        );
        for entity in cache
            .surface
            .into_iter()
            .chain(cache.boundary.as_ref().map(|(entity, _)| *entity))
        {
            commands.entity(entity).insert(Visibility::Hidden);
        }
        return;
    }
    let changed =
        cache.profile.as_ref() != Some(&profile) || cache.bed_revision != Some(bed.revision);
    if changed {
        let image = images.add(texture(&bed));
        if let Some(old) = cache.image.replace(image.clone()) {
            images.remove(old.id());
        }
        if let Some(handle) = &cache.material {
            if let Some(mut material) = materials.get_mut(handle) {
                material.extension.bathymetry = image;
            }
        } else {
            cache.material = Some(materials.add(OceanMaterial {
                base: StandardMaterial {
                    base_color: Color::WHITE,
                    alpha_mode: AlphaMode::Blend,
                    perceptual_roughness: 0.42,
                    reflectance: 0.35,
                    cull_mode: None,
                    double_sided: true,
                    opaque_render_method: OpaqueRendererMethod::Forward,
                    ..default()
                },
                extension: OceanExtension {
                    params: parameters(&profile, &bed, frame.phase_seconds),
                    bathymetry: image,
                },
            }));
        }
        cache.profile = Some(profile.clone());
        cache.bed_revision = Some(bed.revision);
        status.bathymetry_revision = Some(bed.revision);
    }
    let Some(material) = cache.material.clone() else {
        return;
    };
    if let Some(mut value) = materials.get_mut(&material) {
        value.extension.params = parameters(&profile, &bed, frame.phase_seconds);
    }
    let position = Vec3::new(
        frame.camera_position.x,
        profile.mean_sea_level,
        frame.camera_position.z,
    );
    if cache.surface.is_none() {
        let mesh = mesh::surface_mesh();
        status.surface_vertices = mesh.count_vertices();
        cache.surface = Some(
            commands
                .spawn((
                    Mesh3d(meshes.add(mesh)),
                    MeshMaterial3d(material.clone()),
                    Transform::from_translation(position),
                    NoFrustumCulling,
                    NotShadowCaster,
                    Pickable::IGNORE,
                    Surface,
                    Name::new("Ocean swells"),
                ))
                .id(),
        );
    }
    for mut transform in &mut surfaces {
        transform.translation = position;
    }
    if cache.boundary_revision != Some(boundary.revision) {
        if let Some(mesh) = boundary.build() {
            if let Some((entity, old)) = cache.boundary.take() {
                commands.entity(entity).despawn();
                meshes.remove(old.id());
            }
            status.boundary_vertices = mesh.count_vertices();
            if status.boundary_vertices > 0 {
                let handle = meshes.add(mesh);
                let entity = commands
                    .spawn((
                        Mesh3d(handle.clone()),
                        MeshMaterial3d(material),
                        Transform::default(),
                        NoFrustumCulling,
                        NotShadowCaster,
                        Pickable::IGNORE,
                        Name::new("Ocean exact water boundary"),
                    ))
                    .id();
                cache.boundary = Some((entity, handle));
            }
            cache.boundary_revision = Some(boundary.revision);
        } else {
            status.ready = false;
            status.error = Some(
                "Ocean local boundary exceeds its interval bound or contains an invalid height."
                    .into(),
            );
            return;
        }
    }
    status.ready = true;
    status.error = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_offsets_agree_between_cpu_sampling_and_gpu_uniforms() {
        let mut profile = OceanSurfaceProfile::default();
        let bed = OceanBathymetry::default();
        for (at, seconds) in [(Vec2::ZERO, 0.0), (Vec2::new(17.0, -29.0), 7.0)] {
            let uniforms = parameters(&profile, &bed, seconds);
            assert!((uniforms.phase_offsets - Vec4::new(0.0, 1.3, 2.4, 0.0)).length() < 0.00001);
            let specifications = [uniforms.wave0, uniforms.wave1, uniforms.wave2];
            let mut height = profile.mean_sea_level;
            let mut gradient = Vec2::ZERO;
            for (index, specification) in specifications.into_iter().enumerate() {
                let direction = Vec2::new(specification.x, specification.y);
                let phase = specification.w * direction.dot(at)
                    - uniforms.water.y * uniforms.periods[index]
                    + uniforms.phase_offsets[index];
                height += specification.z * phase.sin();
                gradient += direction * (specification.z * specification.w * phase.cos());
            }
            let cpu = super::super::sample_surface(&profile, &bed, at, seconds).unwrap();
            let normal = Vec3::new(-gradient.x, 1.0, -gradient.y).normalize();
            assert!((cpu.height - height).abs() < 0.00001);
            assert!((cpu.normal - normal).length() < 0.00001);
        }
        profile.waves[1].phase_radians = f32::NAN;
        assert!(!profile.is_valid());
        assert!(super::super::sample_surface(&profile, &bed, Vec2::ZERO, 0.0).is_none());
    }
}
