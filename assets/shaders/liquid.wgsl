// Animated original water voxels and opaque lava overlays. Extending
// StandardMaterial retains forward PBR lighting, shadows, fog, and tonemapping.

#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types,
}

#ifdef OIT_ENABLED
#import bevy_core_pipeline::oit::oit_draw
#endif

struct LiquidMaterialParams {
    // xy: downstream UV velocity, z: deterministic phase, w: UV scale.
    flow_phase_scale: vec4<f32>,
    // x: highlight, y: foam, z: roughness reduction, w: cross-wave frequency.
    modulation: vec4<f32>,
    // x: base emission, y: pulse amplitude, z: pulse rate, w: reserved.
    emission: vec4<f32>,
    // Canonical palette-backed water-foam colour in linear RGB.
    foam_color: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100)
var<uniform> liquid: LiquidMaterialParams;

const TAU: f32 = 6.283185307179586;

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    // Lava keeps its existing cap/curtain UV chart and material controls.
    let uv_scale = max(liquid.flow_phase_scale.w, 0.0001);
    var scaled_uv = in.uv * uv_scale;
    var flow_velocity = liquid.flow_phase_scale.xy;
    var highlight_strength = max(liquid.modulation.x, 0.0);
    var foam_strength = clamp(liquid.modulation.y, 0.0, 1.0);
    var roughness_reduction = max(liquid.modulation.z, 0.0);

#ifdef VERTEX_UVS_B
    // Original water terrain batches carry one constant [angle, flow code] per
    // run in UV1. Absolute coordinates avoid restarting waves at every hex or
    // chunk, while the published downstream angle controls visible advection.
    let downstream = vec2<f32>(cos(in.uv_b.x), sin(in.uv_b.x));
    let across = vec2<f32>(-downstream.y, downstream.x);
    let world_plane = in.world_position.xz;
    let top_face = step(0.5, in.world_normal.y);
    let downstream_face = smoothstep(
        0.9,
        0.98,
        dot(in.world_normal.xz, downstream),
    );
    scaled_uv = vec2<f32>(
        dot(world_plane, across),
        mix(-in.world_position.y, dot(world_plane, downstream), top_face),
    ) * uv_scale;
    let moving = step(0.5, in.uv_b.y);
    let rapid = step(1.5, in.uv_b.y);
    let falling = step(2.5, in.uv_b.y);
    // These cycle rates all meet the material's 400-second phase wrap exactly.
    flow_velocity = vec2<f32>(0.0, 0.15 * moving + 0.50 * rapid + 0.20 * falling);
    // Level Current and Still water share a restrained blue modulation. A
    // Current row can continue into the sea without painting its outline white.
    highlight_strength = 0.035 + 0.035 * rapid + 0.025 * falling;
    roughness_reduction = 0.02;
    // Only Rapid/Fall tops and their authored downstream faces receive foam;
    // level currents and the other prism sides retain their water colour.
    foam_strength =
        (0.14 * rapid + 0.06 * falling) *
        max(top_face, downstream_face);
#endif

    let advected_uv =
        scaled_uv - flow_velocity * liquid.flow_phase_scale.z;

    let cross_frequency = max(liquid.modulation.w, 0.0001);
    let cross_wave = sin(TAU * advected_uv.x * cross_frequency);
    let primary_position = advected_uv.y + 0.13 * cross_wave;
    let primary_wave = 0.5 + 0.5 * sin(TAU * primary_position);
    let secondary_wave = 0.5 + 0.5 * sin(
        TAU * (
            scaled_uv.x * 1.37 -
            scaled_uv.y * 0.43 +
            liquid.flow_phase_scale.z * 0.025
        ),
    );

    let analytic_width = max(fwidth(primary_wave), 0.001);
    let crest = smoothstep(
        0.56 - analytic_width,
        0.78 + analytic_width,
        primary_wave,
    );
    var ripple = clamp(crest * 0.78 + secondary_wave * 0.22, 0.0, 1.0);
#ifdef VERTEX_UVS_B
    // Broad blue ripples keep level flow visible without a repeated bright comb.
    ripple = mix(primary_wave * 0.35 + secondary_wave * 0.65, ripple, rapid);
#endif

    var liquid_color =
        pbr_input.material.base_color.rgb * (1.0 + ripple * highlight_strength);
    var foam_mask = smoothstep(
        0.82 - analytic_width,
        0.96 + analytic_width,
        primary_wave,
    ) * foam_strength;
#ifdef VERTEX_UVS_B
    // Two oblique packet fields interrupt crests into separated patches. Both
    // travel with the same downstream coordinates as the crests. Their 1/4 and
    // 1/2 longitudinal frequencies complete whole cycles at the 400-second wrap.
    let along_packet = 0.5 + 0.5 * sin(TAU * (
        advected_uv.y * 0.25 + advected_uv.x * 0.37
    ));
    let across_packet = 0.5 + 0.5 * sin(TAU * (
        advected_uv.x * 0.71 - advected_uv.y * 0.50
    ));
    foam_mask *= smoothstep(0.38, 0.78, along_packet * across_packet);
#endif
    liquid_color = mix(
        liquid_color,
        liquid.foam_color.rgb,
        clamp(foam_mask, 0.0, 0.72),
    );

    pbr_input.material.base_color = vec4<f32>(
        liquid_color,
        pbr_input.material.base_color.a,
    );
    pbr_input.material.perceptual_roughness = clamp(
        pbr_input.material.perceptual_roughness -
            ripple * roughness_reduction,
        0.089,
        1.0,
    );
    let emission_pulse = 0.5 + 0.5 * sin(
        TAU * liquid.flow_phase_scale.z * max(liquid.emission.z, 0.0),
    );
    let emission_strength =
        max(liquid.emission.x, 0.0) +
        max(liquid.emission.y, 0.0) * emission_pulse;
    pbr_input.material.emissive = vec4<f32>(
        liquid_color * emission_strength,
        1.0,
    );
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material,
        pbr_input.material.base_color,
    );

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);

#ifdef OIT_ENABLED
    let alpha_mode =
        pbr_input.material.flags & pbr_types::STANDARD_MATERIAL_FLAGS_ALPHA_MODE_RESERVED_BITS;
    if alpha_mode != pbr_types::STANDARD_MATERIAL_FLAGS_ALPHA_MODE_OPAQUE {
        oit_draw(in.position, out.color);
        discard;
    }
#endif

    return out;
}
