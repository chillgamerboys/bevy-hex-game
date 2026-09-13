// Visual swells only. CPU equations live in hex_map::ocean::sample_surface.
#import bevy_pbr::{
    forward_io::{Vertex, VertexOutput, FragmentOutput},
    mesh_functions,
    view_transformations::position_world_to_clip,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}
#ifdef OIT_ENABLED
#import bevy_core_pipeline::oit::oit_draw
#endif

struct OceanParams {
    water: vec4<f32>,
    bath: vec4<f32>,
    wave0: vec4<f32>,
    wave1: vec4<f32>,
    wave2: vec4<f32>,
    periods: vec4<f32>,
    phase_offsets: vec4<f32>,
    shallow: vec4<f32>,
    deep: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> ocean: OceanParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var beds: texture_2d<f32>;

// Height and its exact bilinear X/Z derivatives. Manual loads avoid requiring
// filterable R32Float support and share the CPU grid interpolation precisely.
fn bed_sample(at: vec2<f32>) -> vec3<f32> {
    let dimensions = textureDimensions(beds);
    let position = (at - ocean.bath.xy) / ocean.bath.z;
    let maximum = vec2<f32>(dimensions - vec2<u32>(1u));
    if any(position < vec2<f32>(0.0)) || any(position > maximum) {
        return vec3<f32>(-140.0, 0.0, 0.0);
    }
    let cell = min(vec2<i32>(floor(position)), vec2<i32>(dimensions) - vec2<i32>(2));
    let t = position - vec2<f32>(cell);
    let a = textureLoad(beds, cell, 0).r;
    let b = textureLoad(beds, cell + vec2<i32>(1, 0), 0).r;
    let c = textureLoad(beds, cell + vec2<i32>(0, 1), 0).r;
    let d = textureLoad(beds, cell + vec2<i32>(1, 1), 0).r;
    let row0 = mix(a, b, t.x);
    let row1 = mix(c, d, t.x);
    return vec3<f32>(mix(row0, row1, t.y), mix(b - a, d - c, t.y) / ocean.bath.z, (row1 - row0) / ocean.bath.z);
}
fn wave(at: vec2<f32>, specification: vec4<f32>, rate: f32, phase_offset: f32) -> vec3<f32> {
    let phase = specification.w * dot(specification.xy, at) - ocean.water.y * rate + phase_offset;
    return vec3<f32>(specification.z * sin(phase), specification.xy * (specification.z * specification.w * cos(phase)));
}
// Displacement followed by X/Z derivative, including shore attenuation.
fn surface(at: vec2<f32>, bed: vec3<f32>) -> vec3<f32> {
    let depth = ocean.water.x - bed.x;
    let t = clamp(depth / ocean.water.z, 0.0, 1.0);
    let weight = t * t * (3.0 - 2.0 * t);
    let slope = -bed.yz * (6.0 * t * (1.0 - t) / ocean.water.z);
    let waves = wave(at, ocean.wave0, ocean.periods.x, ocean.phase_offsets.x) + wave(at, ocean.wave1, ocean.periods.y, ocean.phase_offsets.y) + wave(at, ocean.wave2, ocean.periods.z, ocean.phase_offsets.z);
    return vec3<f32>(waves.x * weight, waves.yz * weight + slope * waves.x);
}
@vertex
fn vertex(input: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let transform = mesh_functions::get_world_from_local(input.instance_index);
    var world = mesh_functions::mesh_position_local_to_world(transform, vec4<f32>(input.position, 1.0));
    let bed = bed_sample(world.xz);
    let swell = surface(world.xz, bed);
    let normal = mesh_functions::mesh_normal_local_to_world(input.normal, input.instance_index);
    // Exact boundary upper vertices follow the same swell; bottoms stay fixed.
    if abs(world.y - ocean.water.x) < 0.005 { world.y += swell.x; }
    out.world_position = world;
    out.position = position_world_to_clip(world.xyz);
    out.world_normal = normal;
#ifdef VERTEX_UVS_A
    out.uv = input.uv;
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = input.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(input.instance_index, transform[3]);
#endif
    return out;
}
@fragment
fn fragment(input: VertexOutput, @builtin(front_facing) front: bool) -> FragmentOutput {
    let bed = bed_sample(input.world_position.xz);
    let depth = ocean.water.x - bed.x;
    var shading = input;
    if input.world_normal.y > 0.5 {
        let swell = surface(input.world_position.xz, bed);
        shading.world_normal = normalize(vec3<f32>(-swell.y, 1.0, -swell.z));
    }
    var pbr = pbr_input_from_standard_material(shading, front);
    let color = mix(ocean.shallow, ocean.deep, clamp(depth / 40.0, 0.0, 1.0));
    pbr.material.base_color = color;
    pbr.material.perceptual_roughness = 0.42;
    var out: FragmentOutput;
    out.color = main_pass_post_lighting_processing(pbr, apply_pbr_lighting(pbr));
    // StandardMaterial sampling uses implicit derivatives. Keep that work ahead
    // of our nonuniform discard so shoreline fragments do not make its texture
    // sampling violate WGSL derivative-uniformity requirements.
    if depth <= 0.0 && input.world_normal.y > 0.5 { discard; }
#ifdef OIT_ENABLED
    oit_draw(input.position, out.color);
    discard;
#endif
    return out;
}
