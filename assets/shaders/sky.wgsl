// Procedural sky: a blue vertical gradient with static hexagonal clouds.
// Rendered on the inside of a large sphere ("sky dome") that follows the camera,
// so `dir` depends only on view orientation — the sky is stable while panning and
// only re-orients while orbiting.

#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings::view

struct SkyParams {
    horizon_color: vec3<f32>,
    cloud_coverage: f32,
    zenith_color: vec3<f32>,
    hex_scale: f32,
    cloud_color: vec3<f32>,
    cloud_softness: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> sky: SkyParams;

fn hash21(p: vec2<f32>) -> f32 {
    let n = dot(p, vec2<f32>(127.1, 311.7));
    return fract(sin(n) * 43758.5453123);
}

// pointy-top axial hex helpers, size 1 (redblobgames)
fn axial_from_point(p: vec2<f32>) -> vec2<f32> {
    let q = 0.5773502692 * p.x - 0.3333333333 * p.y;
    let r = 0.6666666667 * p.y;
    return vec2<f32>(q, r);
}
fn point_from_axial(h: vec2<f32>) -> vec2<f32> {
    let x = 1.7320508076 * (h.x + 0.5 * h.y);
    let y = 1.5 * h.y;
    return vec2<f32>(x, y);
}
fn hex_round(h: vec2<f32>) -> vec2<f32> {
    let x = h.x;
    let z = h.y;
    let y = -x - z;
    var rx = round(x);
    var ry = round(y);
    var rz = round(z);
    let dx = abs(rx - x);
    let dy = abs(ry - y);
    let dz = abs(rz - z);
    if (dx > dy && dx > dz) {
        rx = -ry - rz;
    } else if (dy > dz) {
        ry = -rx - rz;
    } else {
        rz = -rx - ry;
    }
    return vec2<f32>(rx, rz);
}
// hexagonal distance field: ~0 at centre, grows to the flat edges
fn hex_dist(p: vec2<f32>) -> f32 {
    let q = abs(p);
    return max(q.x, dot(q, vec2<f32>(0.5, 0.8660254)));
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let dir = normalize(in.world_position.xyz - view.world_position.xyz);

    // vertical gradient (horizon -> zenith)
    let t = clamp(dir.y, 0.0, 1.0);
    var color = mix(sky.horizon_color, sky.zenith_color, t);

    // Azimuthal-equidistant projection: the radius on the hex plane is the angle
    // away from the zenith, so a hex covers the same angular size straight up as it
    // does near the horizon. A plain `dir.xz / dir.y` (gnomonic) projection instead
    // stretches cells to infinity as the view approaches the horizon.
    let theta = acos(clamp(dir.y, -1.0, 1.0));
    let azim = atan2(dir.z, dir.x);
    let p = vec2<f32>(cos(azim), sin(azim)) * theta * sky.hex_scale;

    let cell = hex_round(axial_from_point(p));
    let centre = point_from_axial(cell);
    let local = p - centre;

    let present = step(1.0 - sky.cloud_coverage, hash21(cell));
    let edge = 1.0 - smoothstep(0.8 - sky.cloud_softness, 0.8 + sky.cloud_softness, hex_dist(local));
    let cloud = present * edge;

    color = mix(color, sky.cloud_color, cloud);
    return vec4<f32>(color, 1.0);
}
