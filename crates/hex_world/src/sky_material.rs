//! The procedural sky: a blue gradient with static hexagonal clouds, drawn on the
//! inside of a camera-following dome by a custom material. See `assets/shaders/sky.wgsl`.

use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

/// GPU-side sky parameters.
///
/// Field order and types must match `struct SkyParams` in `assets/shaders/sky.wgsl`
/// exactly — `encase` lays out `Vec3 + f32` pairs to match the WGSL `vec3 + f32`.
#[derive(Clone, ShaderType)]
pub(crate) struct SkyParams {
    pub horizon_color: Vec3,
    pub cloud_coverage: f32,
    pub zenith_color: Vec3,
    pub hex_scale: f32,
    pub cloud_color: Vec3,
    pub cloud_softness: f32,
}

/// Material that renders the procedural sky onto the dome.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
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
}
