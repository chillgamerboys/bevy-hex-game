// Opaque animated liquid overlay. Extending StandardMaterial retains Bevy's
// forward PBR lighting, shadows, fog, exposure, and tonemapping.

#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
}

struct LiquidMaterialParams {
    // xy: downstream UV velocity, z: deterministic phase, w: UV scale.
    flow_phase_scale: vec4<f32>,
    // x: highlight, y: foam, z: roughness reduction, w: cross-wave frequency.
    modulation: vec4<f32>,
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

    // Mesh UV +V is authored downstream. Cap entities rotate that axis toward
    // their exact successor; curtain +V runs from the lip to the landing.
    let uv_scale = max(liquid.flow_phase_scale.w, 0.0001);
    let scaled_uv = in.uv * uv_scale;
    let advected_uv =
        scaled_uv - liquid.flow_phase_scale.xy * liquid.flow_phase_scale.z;

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
    let ripple = clamp(crest * 0.78 + secondary_wave * 0.22, 0.0, 1.0);

    let highlight_strength = max(liquid.modulation.x, 0.0);
    let foam_strength = clamp(liquid.modulation.y, 0.0, 1.0);
    let roughness_reduction = max(liquid.modulation.z, 0.0);

    var liquid_color =
        pbr_input.material.base_color.rgb * (1.0 + ripple * highlight_strength);
    let foam_mask = smoothstep(
        0.82 - analytic_width,
        0.96 + analytic_width,
        primary_wave,
    ) * foam_strength;
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
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material,
        pbr_input.material.base_color,
    );

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    out.color.a = 1.0;
    return out;
}
