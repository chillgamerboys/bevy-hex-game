//! The procedural sky: a blue gradient with static hexagonal clouds, drawn on the
//! inside of a camera-following dome by a custom material. See `assets/shaders/sky.wgsl`.

use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

/// GPU-side sky parameters.
///
/// Field order and types must match `struct SkyParams` in `assets/shaders/sky.wgsl`
/// exactly — `encase` lays out `Vec3 + f32` pairs to match the WGSL `vec3 + f32`.
#[derive(Clone, ShaderType, Reflect)]
pub(crate) struct SkyParams {
    pub horizon_color: Vec3,
    pub cloud_coverage: f32,
    pub zenith_color: Vec3,
    pub hex_scale: f32,
    pub cloud_color: Vec3,
    pub cloud_softness: f32,
    pub cloud_roundness: f32,
    pub cloud_noise: f32,
    pub sun_direction: Vec3,
    pub celestial_bodies_enabled: f32,
    pub sun_disc_color: Vec3,
    pub sun_angular_radius_radians: f32,
    pub moon_direction: Vec3,
    pub moon_angular_radius_radians: f32,
    pub moon_disc_color: Vec3,
    pub sun_halo_width_radians: f32,
    pub lower_glow_direction: Vec3,
    pub moon_halo_width_radians: f32,
    pub lower_glow_color: Vec3,
    pub sun_halo_strength: f32,
    pub moon_halo_strength: f32,
    pub lower_glow_angular_radius_radians: f32,
    pub lower_glow_strength: f32,
    pub _padding: f32,
}

/// Material that renders the procedural sky onto the dome.
///
/// `TypePath` is not derived explicitly: `Reflect` generates one, and having both is a
/// conflicting-implementation error. `Asset` needs it as a supertrait either way.
#[derive(Asset, AsBindGroup, Clone, Reflect)]
pub(crate) struct SkyMaterial {
    #[uniform(0)]
    pub params: SkyParams,
}

impl Material for SkyMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/sky.wgsl".into()
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // The camera sits inside the dome, so render its inward-facing triangles.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// Registers the procedural sky material.
pub fn plugin(app: &mut App) {
    app.add_plugins(MaterialPlugin::<SkyMaterial>::default());
    // `register_asset_reflect` rather than `register_type`: the inspector lists assets
    // by filtering the registry for `ReflectAsset`, which only this adds. With plain
    // `register_type` the material is registered and still never appears.
    app.register_asset_reflect::<SkyMaterial>();
}

#[cfg(test)]
mod tests {
    #[test]
    fn celestial_bodies_clip_smoothly_and_clouds_composite_last() {
        let shader = include_str!("../../../assets/shaders/sky.wgsl");
        assert!(
            !shader.contains("body_dir.y <= 0.0"),
            "body centres must not be culled at the horizon"
        );
        assert!(shader.contains("body_elevation + outer_radius <= 0.0"));
        assert!(shader.contains("fwidth(view_dir.y)"));
        assert!(shader.contains("disc = disc * horizon"));
        assert!(shader.contains("halo = halo * horizon"));

        let moon = shader
            .find("sky.moon_direction")
            .expect("the moon should be composited");
        let clouds = shader
            .rfind("color = mix(color, sky.cloud_color")
            .expect("clouds should be composited");
        assert!(clouds > moon, "clouds must remain above celestial bodies");
    }
}
