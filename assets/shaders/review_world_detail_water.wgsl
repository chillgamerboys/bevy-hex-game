// Disposable review-water material. Extending StandardMaterial retains Bevy's
// lighting, fog, attenuation, and medium-quality screen-space transmission.

#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings as view_bindings,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types,
}

#ifdef OIT_ENABLED
#import bevy_core_pipeline::oit::oit_draw
#endif

struct ReviewWaterMaterialParams {
    // Shipped liquid contract: xy flow, z phase, w UV scale.
    flow_phase_scale: vec4<f32>,
    // Shipped liquid contract: highlight, foam, roughness reduction, cross-wave frequency.
    modulation: vec4<f32>,
    // Shipped liquid contract: emission controls (zero for water).
    emission: vec4<f32>,
    // Current palette-backed foam colour in linear RGB.
    foam_color: vec4<f32>,
    // x: W06-only maximum refracted screen-UV offset; yzw reserved.
    refraction: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100)
var<uniform> review_water: ReviewWaterMaterialParams;

const TAU: f32 = 6.283185307179586;

fn transmission_uv(world_position: vec3<f32>) -> vec2<f32> {
    let clip = view_bindings::view.clip_from_world * vec4<f32>(world_position, 1.0);
    let clip_sign = select(-1.0, 1.0, clip.w >= 0.0);
    let safe_w = select(clip.w, clip_sign * 0.00001, abs(clip.w) < 0.00001);
    return (clip.xy / safe_w) * vec2<f32>(0.5, -0.5) + 0.5;
}

// Matches the refracted direction used by Bevy's screen-space transmission
// function. The caller can therefore limit the exact projected displacement
// before Bevy samples the transmission texture.
fn refracted_direction(N: vec3<f32>, V: vec3<f32>, ior: f32) -> vec3<f32> {
    let eta = 1.0 / max(ior, 0.0001);
    let I = -V;
    let NdotI = dot(N, I);
    let k = max(1.0 - eta * eta * (1.0 - NdotI * NdotI), 0.0);
    return eta * I - (eta * NdotI + sqrt(k)) * N;
}

fn projected_refraction_displacement(
    world_position: vec3<f32>,
    direction: vec3<f32>,
    thickness: f32,
) -> f32 {
    let entry_uv = transmission_uv(world_position);
    let exit_uv = transmission_uv(world_position + direction * thickness);
    return length(exit_uv - entry_uv);
}

fn capped_refraction_thickness(
    world_position: vec3<f32>,
    N: vec3<f32>,
    V: vec3<f32>,
    ior: f32,
    thickness: f32,
    maximum_uv: f32,
) -> f32 {
    let requested_thickness = max(thickness, 0.0);
    let cap = clamp(maximum_uv, 0.0, 0.015);
    if requested_thickness == 0.0 || cap == 0.0 {
        return 0.0;
    }
    let direction = refracted_direction(N, V, ior);
    if projected_refraction_displacement(
        world_position,
        direction,
        requested_thickness,
    ) <= cap {
        return requested_thickness;
    }

    // Projection is rational rather than linear in distance. A fixed-count
    // bisection is deterministic and leaves `low` on the admitted side, so the
    // central refracted ray cannot exceed the requested UV radius. W06 forces
    // zero transmission roughness below, so this is the only effective tap.
    var low = 0.0;
    var high = requested_thickness;
    for (var iteration: u32 = 0u; iteration < 12u; iteration += 1u) {
        let middle = 0.5 * (low + high);
        if projected_refraction_displacement(world_position, direction, middle) <= cap {
            low = middle;
        } else {
            high = middle;
        }
    }
    return low;
}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    // This is the ordinary liquid shader's exact surface/fall modulation. The
    // optics study changes only its named alpha, absorption, transmission,
    // roughness, reflectance, and refraction fields.
    let uv_scale = max(review_water.flow_phase_scale.w, 0.0001);
    let scaled_uv = in.uv * uv_scale;
    let advected_uv =
        scaled_uv - review_water.flow_phase_scale.xy * review_water.flow_phase_scale.z;
    let cross_frequency = max(review_water.modulation.w, 0.0001);
    let cross_wave = sin(TAU * advected_uv.x * cross_frequency);
    let primary_position = advected_uv.y + 0.13 * cross_wave;
    let primary_wave = 0.5 + 0.5 * sin(TAU * primary_position);
    let secondary_wave = 0.5 + 0.5 * sin(
        TAU * (
            scaled_uv.x * 1.37 -
            scaled_uv.y * 0.43 +
            review_water.flow_phase_scale.z * 0.025
        ),
    );
    let analytic_width = max(fwidth(primary_wave), 0.001);
    let crest = smoothstep(
        0.56 - analytic_width,
        0.78 + analytic_width,
        primary_wave,
    );
    let ripple = clamp(crest * 0.78 + secondary_wave * 0.22, 0.0, 1.0);
    let highlight_strength = max(review_water.modulation.x, 0.0);
    let foam_strength = clamp(review_water.modulation.y, 0.0, 1.0);
    let roughness_reduction = max(review_water.modulation.z, 0.0);
    var liquid_color =
        pbr_input.material.base_color.rgb * (1.0 + ripple * highlight_strength);
    let foam_mask = smoothstep(
        0.82 - analytic_width,
        0.96 + analytic_width,
        primary_wave,
    ) * foam_strength;
    liquid_color = mix(
        liquid_color,
        review_water.foam_color.rgb,
        clamp(foam_mask, 0.0, 0.72),
    );
    pbr_input.material.base_color = vec4<f32>(
        liquid_color,
        pbr_input.material.base_color.a,
    );
    pbr_input.material.perceptual_roughness = clamp(
        pbr_input.material.perceptual_roughness - ripple * roughness_reduction,
        0.089,
        1.0,
    );
    let emission_pulse = 0.5 + 0.5 * sin(
        TAU * review_water.flow_phase_scale.z * max(review_water.emission.z, 0.0),
    );
    let emission_strength =
        max(review_water.emission.x, 0.0) +
        max(review_water.emission.y, 0.0) * emission_pulse;
    pbr_input.material.emissive = vec4<f32>(
        liquid_color * emission_strength,
        1.0,
    );

    if pbr_input.material.specular_transmission > 0.0 {
        // W06 is deliberately a single central transmission ray. Bevy's rough
        // transmission taps can spread beyond the requested screen-UV bound,
        // while zero roughness makes the capped thickness below authoritative
        // for every effective transmission sample.
        pbr_input.material.perceptual_roughness = 0.0;
        pbr_input.material.thickness = capped_refraction_thickness(
            pbr_input.world_position.xyz,
            pbr_input.N,
            pbr_input.V,
            pbr_input.material.ior,
            pbr_input.material.thickness,
            review_water.refraction.x,
        );
    }

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
