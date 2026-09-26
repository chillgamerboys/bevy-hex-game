// Steady downstream shading only: exact authored prism heights never move.
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
}
#ifdef OIT_ENABLED
#import bevy_core_pipeline::oit::oit_draw
#endif

struct RiverParameters {
    // Shared saved-clock phase, foam strength, fall flag, wavelength.
    phase_foam_fall: vec4<f32>,
    // Authored chart direction, plus horizontal/fall phase at the render origin.
    direction: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> river: RiverParameters;

@fragment
fn fragment(input: VertexOutput, @builtin(front_facing) front: bool) -> FragmentOutput {
    var pbr = pbr_input_from_standard_material(input, front);
    let falling = river.phase_foam_fall.z > 0.5 && abs(input.world_normal.y) < 0.5;
    let chart = select(dot(input.world_position.xz, river.direction.xy), -input.world_position.y, falling);
    let origin_phase = select(river.direction.z, river.direction.w, falling);
    let phase = chart / river.phase_foam_fall.w + origin_phase - river.phase_foam_fall.x;
    let wave = 0.5 + 0.5 * sin(6.283185307 * phase);
    let width = max(fwidth(wave), 0.002);
    let crest = smoothstep(0.68 - width, 0.92 + width, wave);
    let foam = crest * river.phase_foam_fall.y;
    let highlight = pbr.material.base_color.rgb * (1.0 + 0.16 * crest);
    pbr.material.base_color = vec4<f32>(mix(highlight, vec3<f32>(0.75, 0.88, 0.86), foam), pbr.material.base_color.a);
    pbr.material.perceptual_roughness = 0.75 - 0.12 * crest;
    pbr.material.base_color = alpha_discard(pbr.material, pbr.material.base_color);
    var out: FragmentOutput;
    out.color = main_pass_post_lighting_processing(pbr, apply_pbr_lighting(pbr));
#ifdef OIT_ENABLED
    oit_draw(input.position, out.color);
    discard;
#endif
    return out;
}
