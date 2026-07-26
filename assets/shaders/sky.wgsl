// Procedural sky: a blue vertical gradient with soft hexagonal clouds.
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
    // Field order and 16-byte pairing must match `SkyParams` in sky_material.rs.
    cloud_roundness: f32,
    cloud_noise: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> sky: SkyParams;

fn hash21(p: vec2<f32>) -> f32 {
    let n = dot(p, vec2<f32>(127.1, 311.7));
    return fract(sin(n) * 43758.5453123);
}

// Value noise + 3-octave fbm, built on `hash21`, used to break up cloud edges.
fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}
fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var amp = 0.5;
    var freq = 1.0;
    for (var i = 0; i < 3; i = i + 1) {
        v = v + amp * value_noise(p * freq);
        freq = freq * 2.0;
        amp = amp * 0.5;
    }
    return v;
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

// Density contributed by one hex cell at point `p`. A present cell (its hash clears
// the coverage threshold) adds a soft bump centred on the cell; the bump reaches past
// the cell's apothem (~0.87) so adjacent present cells overlap into one blob instead
// of leaving a seam. `cloud_roundness` blends the shape from a hexagon toward a disc.
fn cloud_bump(p: vec2<f32>, cell: vec2<f32>) -> f32 {
    let present = step(1.0 - sky.cloud_coverage, hash21(cell));
    let local = p - point_from_axial(cell);
    let d = mix(hex_dist(local), length(local), sky.cloud_roundness);
    // Per-cell size jitter so the field is not a uniform tiling.
    let r = 0.9 + 0.3 * hash21(cell + vec2<f32>(19.0, 7.0));
    let bump = 1.0 - smoothstep(r - 0.45, r + 0.35, d);
    return present * bump;
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

    // Accumulate cloud density over the cell the pixel is in and its six neighbours,
    // so a cloud spanning several present cells is one continuous mass.
    let cell = hex_round(axial_from_point(p));
    var offsets = array<vec2<f32>, 7>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(-1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
    );
    var density = 0.0;
    for (var i = 0; i < 7; i = i + 1) {
        density = density + cloud_bump(p, cell + offsets[i]);
    }

    // Perturb the density with fbm so the boundary is fluffy, not a clean polygon.
    density = density + (fbm(p * 0.7) - 0.5) * sky.cloud_noise;

    // Threshold at 0.5 with a screen-space-derivative width, so the edge stays ~1px
    // crisp at any zoom or view angle instead of aliasing. `cloud_softness` adds
    // artistic softening on top of that analytic width.
    let w = max(fwidth(density), 0.001) + sky.cloud_softness;
    let mask = smoothstep(0.5 - w, 0.5 + w, density);

    color = mix(color, sky.cloud_color, clamp(mask, 0.0, 1.0));
    return vec4<f32>(color, 1.0);
}
