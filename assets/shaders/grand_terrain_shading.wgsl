// Grand diagnostic: fragment-only normal/color frequency filtering.
// The standard Bevy vertex shader, geometry, depth and shadow meshes are unchanged.
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::view,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}
struct Parameters { pixels: vec4<f32>, }
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> terrain_filter: Parameters;
fn pixels_between(a: vec3<f32>, b: vec3<f32>) -> f32 {
    let p=view.clip_from_world*vec4<f32>(a,1.0);
    let q=view.clip_from_world*vec4<f32>(b,1.0);
    if p.w <= 0.0 || q.w <= 0.0 { return 1000000.0; }
    return length((p.xy/p.w-q.xy/q.w)*0.5*view.viewport.zw);
}
@fragment
fn fragment(input: VertexOutput, @builtin(front_facing) front: bool) -> FragmentOutput {
    var original=input;
    // COLOR is a transport attribute, never a tint on original near shading.
    original.color=vec4<f32>(1.0);
    var pbr=pbr_input_from_standard_material(original,front);
    let p=input.world_position.xyz;
    // All three opposite pointy-hex corners: diagonal views cannot underestimate
    // the projected diameter by measuring only the world X/Z axes.
    let radius=terrain_filter.pixels.z*0.5;
    let a=vec3<f32>(0.0,0.0,radius);
    let b=vec3<f32>(0.8660254038*radius,0.0,0.5*radius);
    let c=vec3<f32>(0.8660254038*radius,0.0,-0.5*radius);
    let diameter=max(pixels_between(p-a,p+a),max(pixels_between(p-b,p+b),pixels_between(p-c,p+c)));
    let vertical=pixels_between(p,p+vec3<f32>(0.0,input.uv.x,0.0));
    let extent=max(diameter,vertical);
    let weight=input.color.a*(1.0-smoothstep(terrain_filter.pixels.x,terrain_filter.pixels.y,extent));
    if weight > 0.0 {
        let macro_normal=normalize(vec3<f32>(input.uv_b.x,1.0,input.uv_b.y));
        pbr.N=normalize(mix(pbr.N,macro_normal,weight));
        pbr.material.base_color=vec4<f32>(mix(pbr.material.base_color.rgb,input.color.rgb,weight),pbr.material.base_color.a);
    }
    var out: FragmentOutput;
    out.color=main_pass_post_lighting_processing(pbr,apply_pbr_lighting(pbr));
    return out;
}
