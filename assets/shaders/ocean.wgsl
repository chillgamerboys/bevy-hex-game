// Visual swells only. CPU equations live in hex_map::ocean::sample_surface.
#import bevy_pbr::{
    forward_io::{Vertex, VertexOutput, FragmentOutput},
    mesh_functions,
    mesh_view_bindings::view,
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
    near: vec4<f32>,
    effects: vec4<f32>,
    shallow: vec4<f32>,
    deep: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> ocean: OceanParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var beds: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var near_water: texture_2d<f32>;

struct OceanBed { bed: vec3<f32>, shelter: vec3<f32>, anchor: vec2<f32>, anchor_dx: vec2<f32>, anchor_dz: vec2<f32>, shore_distance: vec3<f32>, }

fn shore_distance(at: vec2<f32>, anchor: vec2<f32>) -> vec3<f32> {
    let offset = at-anchor;
    let distance = length(offset);
    return vec3<f32>(distance, offset/max(distance, 0.00001));
}

// Height and its exact bilinear X/Z derivatives. Manual loads avoid requiring
// filterable RGBA32Float support and share the CPU grid interpolation precisely.
fn bed_sample(at: vec2<f32>) -> OceanBed {
    let dimensions = textureDimensions(beds);
    let position = (at - ocean.bath.xy) / ocean.bath.z;
    let maximum = vec2<f32>(dimensions - vec2<u32>(1u));
    if any(position < vec2<f32>(0.0)) || any(position > maximum) {
        return OceanBed(vec3<f32>(-140.0, 0.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), vec2<f32>(-100000.0), vec2<f32>(0.0), vec2<f32>(0.0), vec3<f32>(100000.0, 0.0, 0.0));
    }
    let cell = min(vec2<i32>(floor(position)), vec2<i32>(dimensions) - vec2<i32>(2));
    let t = position - vec2<f32>(cell);
    let a = textureLoad(beds, cell, 0);
    let b = textureLoad(beds, cell + vec2<i32>(1, 0), 0);
    let c = textureLoad(beds, cell + vec2<i32>(0, 1), 0);
    let d = textureLoad(beds, cell + vec2<i32>(1, 1), 0);
    let row0 = mix(a, b, t.x);
    let row1 = mix(c, d, t.x);
    let value = mix(row0, row1, t.y);
    let dx = mix(b - a, d - c, t.y) / ocean.bath.z;
    let dz = (row1 - row0) / ocean.bath.z;
    // Distances to the four real anchors remain conservative across unrelated
    // coasts. Distance to a blended anchor invents offshore reflection bands.
    let sa = shore_distance(at, a.zw);
    let sb = shore_distance(at, b.zw);
    let sc = shore_distance(at, c.zw);
    let sd = shore_distance(at, d.zw);
    let srow0 = mix(sa, sb, t.x);
    let srow1 = mix(sc, sd, t.x);
    let svalue = mix(srow0, srow1, t.y);
    let shore = vec3<f32>(svalue.x, svalue.y + mix(sb.x-sa.x, sd.x-sc.x, t.y)/ocean.bath.z, svalue.z + (srow1.x-srow0.x)/ocean.bath.z);
    return OceanBed(vec3<f32>(value.x, dx.x, dz.x), vec3<f32>(value.y, dx.y, dz.y), value.zw, dx.zw, dz.zw, shore);
}

// Same pointy-hex cube rounding as the authoritative world coordinate query.
fn exact_water(at: vec2<f32>) -> f32 {
    let q = 0.5773502692 * at.x - at.y / 3.0;
    let r = 2.0 * at.y / 3.0;
    let cube = vec3<f32>(q, -q-r, r);
    var rounded = round(cube);
    let error = abs(rounded - cube);
    if error.x > error.y && error.x > error.z { rounded.x = -rounded.y-rounded.z; }
    else if error.y > error.z { rounded.y = -rounded.x-rounded.z; }
    else { rounded.z = -rounded.x-rounded.y; }
    let cell = vec2<i32>(rounded.xz - ocean.near.xy);
    let size = vec2<i32>(textureDimensions(near_water));
    if any(cell < vec2<i32>(0)) || any(cell >= size) { return 0.0; }
    return textureLoad(near_water, cell, 0).r;
}
struct Reflection { weight: f32, gradient: vec2<f32>, anchor: vec2<f32>, anchor_dx: vec2<f32>, anchor_dz: vec2<f32>, }
fn shore_reflection(at: vec2<f32>, bed: OceanBed) -> Reflection {
    let distance = bed.shore_distance.x;
    let t = clamp((distance-2.0)/(ocean.effects.y-2.0), 0.0, 1.0);
    let strength = ocean.effects.x * ocean.effects.z;
    let weight = strength * (1.0-t*t*(3.0-2.0*t));
    return Reflection(weight, -strength*(6.0*t*(1.0-t)/(ocean.effects.y-2.0))*bed.shore_distance.yz, bed.anchor, bed.anchor_dx, bed.anchor_dz);
}
fn wave(at: vec2<f32>, specification: vec4<f32>, rate: f32, phase_offset: f32, reflection: Reflection) -> vec4<f32> {
    let phase = specification.w * dot(specification.xy, at) - ocean.water.y * rate + phase_offset;
    let incident = vec4<f32>(specification.z * sin(phase), specification.xy * (specification.z * specification.w * cos(phase)), -rate*specification.z*cos(phase));
    if reflection.weight <= 0.0 { return incident; }
    let reflected_phase = -specification.w*dot(specification.xy, at) + 2.0*specification.w*dot(specification.xy, reflection.anchor) - rate*ocean.water.y + phase_offset;
    let reflected_gradient = -specification.w*specification.xy + 2.0*specification.w*vec2<f32>(dot(specification.xy, reflection.anchor_dx), dot(specification.xy, reflection.anchor_dz));
    let reflected = vec4<f32>(specification.z*sin(reflected_phase), specification.z*cos(reflected_phase)*reflected_gradient, -rate*specification.z*cos(reflected_phase));
    let denominator = 1.0+reflection.weight;
    let combined = (incident+reflection.weight*reflected)/denominator;
    let correction = reflection.gradient*((reflected.x-incident.x)/(denominator*denominator));
    return vec4<f32>(combined.x, combined.yz+correction, combined.w);
}
// Displacement followed by X/Z derivative, including shore attenuation.
fn surface(at: vec2<f32>, sample: OceanBed) -> vec4<f32> {
    let bed = sample.bed;
    let depth = ocean.water.x - bed.x;
    let t = clamp(depth / ocean.water.z, 0.0, 1.0);
    let depth_weight = t * t * (3.0 - 2.0 * t);
    let weight = depth_weight * sample.shelter.x;
    let slope = -bed.yz * (6.0 * t * (1.0 - t) / ocean.water.z) * sample.shelter.x
        + sample.shelter.yz * depth_weight;
    let reflection = shore_reflection(at, sample);
    let waves = wave(at, ocean.wave0, ocean.periods.x, ocean.phase_offsets.x, reflection) + wave(at, ocean.wave1, ocean.periods.y, ocean.phase_offsets.y, reflection) + wave(at, ocean.wave2, ocean.periods.z, ocean.phase_offsets.z, reflection);
    return vec4<f32>(waves.x * weight, waves.yz * weight + slope * waves.x, waves.w*weight);
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
    let depth = ocean.water.x - bed.bed.x;
    let footprint = max(length(dpdx(input.world_position.xz)), length(dpdy(input.world_position.xz)));
    let normal_detail = 1.0 - smoothstep(4.0, 16.0, footprint);
    var shading = input;
    var crest = 0.0;
    var foam = 0.0;
    if input.world_normal.y > 0.5 {
        let swell = surface(input.world_position.xz, bed);
        shading.world_normal = normalize(vec3<f32>(-swell.y * normal_detail, 1.0, -swell.z * normal_detail));
        // Reuse the displaced, shore-attenuated swell rather than a separate
        // moving noise pattern. Zero-amplitude or flattened shoreline water has
        // neither highlights nor foam; no new texture reads or wave sampling.
        let amplitude = max(ocean.wave0.z + ocean.wave1.z + ocean.wave2.z, 0.0001);
        let depth_t = clamp(depth/ocean.water.z, 0.0, 1.0);
        let local_weight = depth_t*depth_t*(3.0-2.0*depth_t)*bed.shelter.x;
        let crest_height = max(swell.x / max(amplitude*local_weight, 0.1), 0.0);
        crest = smoothstep(0.35, 0.85, crest_height) * normal_detail;
        let shallow = 1.0 - smoothstep(ocean.water.z, ocean.water.z * 3.0, depth);
        foam = smoothstep(0.50, 0.85, crest_height) * mix(0.04, 0.09, shallow) * smoothstep(0.02,0.1,local_weight) * normal_detail;
    }
    var pbr = pbr_input_from_standard_material(shading, front);
    let color = mix(ocean.shallow, ocean.deep, clamp(depth / 40.0, 0.0, 1.0));
    // Restrained cool crest accents keep long swells readable without white
    // patches covering the sea. Preserve depth-driven alpha, especially shallows.
    let highlighted = color.rgb * (1.0 + 0.08 * crest);
    let surface_color = mix(highlighted, vec3<f32>(0.40, 0.65, 0.73), foam);
    // The decorative mesh is finite. Fade its far ring into the clear lower sky
    // so high-altitude views never reveal the camera-centered circular boundary.
    let camera_distance = length(input.world_position.xz - view.world_position.xz);
    let horizon = 1.0 - smoothstep(ocean.water.w * 0.67, ocean.water.w, camera_distance);
    pbr.material.base_color = vec4<f32>(surface_color, color.a * horizon);
    pbr.material.perceptual_roughness = 0.58 - 0.025 * crest;
    var out: FragmentOutput;
    out.color = main_pass_post_lighting_processing(pbr, apply_pbr_lighting(pbr));
    // StandardMaterial sampling uses implicit derivatives. Keep that work ahead
    // of our nonuniform discard so shoreline fragments do not make its texture
    // sampling violate WGSL derivative-uniformity requirements.
    // The exact local mask owns near wet/dry coverage. Coarse bathymetry only
    // attenuates waves: its different triangulation must not punch shore holes.
    // Outside the known window, opaque distant terrain clips the decorative sea.
    if input.world_normal.y > 0.5 && exact_water(input.world_position.xz) < -0.5 { discard; }
    // Closed water boundaries remain visible from outside, including carved
    // vessels, but their backfaces must not z-fight with the opaque seabed.
    if input.world_normal.y < 0.5 && !front { discard; }
#ifdef OIT_ENABLED
    oit_draw(input.position, out.color);
    discard;
#endif
    return out;
}
