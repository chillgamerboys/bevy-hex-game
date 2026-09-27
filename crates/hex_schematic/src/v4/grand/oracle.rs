//! Continuous approved landforms, sampled at production hex resolution.
//! No 12-unit study-grid stair steps enter the playable terrain.
use super::geography::{ellipse, irregular, route_distance, segment, GrandGeographyDocument};
fn clamp(x: f64) -> f64 {
    x.clamp(0., 1.)
}
fn smooth(x: f64) -> f64 {
    let t = clamp(x);
    t * t * (3. - 2. * t)
}
fn noise(x: f64, z: f64) -> f64 {
    (x * 0.017 + z * 0.005).sin() * (z * 0.022 - x * 0.003).sin()
        + 0.45 * (x * 0.034 - z * 0.013).sin()
        + 0.3 * (z * 0.051 + x * 0.025).cos()
}
/// Signed continuous mainland relief; positive land is measured before carving.
pub(super) fn mainland(d: &GrandGeographyDocument, [x, z]: [f64; 2]) -> f64 {
    let co = &d.coast;
    let r = ellipse([x, z], co.center, co.radii);
    let a = (z - co.center[1]).atan2(x);
    let shore = 1.
        + 0.055 * (5. * a + co.phase).sin()
        + 0.035 * (9. * a + 0.3).cos()
        + 0.025 * (13. * a).sin();
    let mut h = 42. * (shore - r) + 2.6 * noise(x, z);
    for &[cx, cz, rx, rz] in &co.coves {
        h -= 35. * (-2. * ellipse([x, z], [cx, cz], [rx, rz]).powi(2)).exp();
    }
    let land = clamp((shore - r) * 7.);
    let [cx, cz, ph, rx, rz] = d.massif;
    let mut high = ph * clamp(1. - ellipse([x, z], [cx, cz], [rx, rz])).powf(1.35);
    let [cx, cz, ph, rx, rz] = d.headland;
    high = high.max(ph * (-ellipse([x, z], [cx, cz], [rx, rz]).powi(2) * 1.35).exp());
    for (i, &[px, pz, ph, rx, rz]) in d.peaks.iter().enumerate() {
        let a = (z - pz).atan2(x - px);
        let rr = ellipse([x, z], [px, pz], [rx, rz])
            / (1. + 0.13 * (3. * a + i as f64).sin() + 0.05 * (5. * a).cos());
        high = high.max(ph * clamp(1. - rr * 0.65).powf(1.65));
    }
    for &[i, j] in &d.ridge_links {
        if let (Some(a), Some(b)) = (d.peaks.get(i), d.peaks.get(j)) {
            let (dist, t) = segment([x, z], [a[0], a[1]], [b[0], b[1]]);
            let crest = (a[2] * (1. - t) + b[2] * t) * 0.72;
            high = high.max(crest * clamp(1. - dist / 170.).powf(1.4));
        }
    }
    let hills = d
        .low_hills
        .iter()
        .map(|&[cx, cz, ph, rx, rz]| ph * (-ellipse([x, z], [cx, cz], [rx, rz]).powi(2)).exp())
        .sum::<f64>();
    for &[cx, cz, ph, rx, rz] in &d.site_shoulders {
        high = high.max(ph * clamp(1. - ellipse([x, z], [cx, cz], [rx, rz]) * 0.70).powf(1.6));
    }
    for path in &d.landform_ridges {
        for pair in path.windows(2) {
            let (Some(a), Some(b)) = (pair.first(), pair.get(1)) else {
                continue;
            };
            let (dist, t) = segment([x, z], [a[0], a[1]], [b[0], b[1]]);
            let crest = a[2] * (1. - t) + b[2] * t;
            let width = a[3] * (1. - t) + b[3] * t;
            high = high.max(crest * clamp(1. - dist / width).powf(1.4));
        }
    }
    h += high + land * hills;
    let c = d.lower_lake.center;
    let blend = clamp(
        1. - ellipse(
            [x, z],
            [
                c[0] + d.valley_bowl.offset[0],
                c[1] + d.valley_bowl.offset[1],
            ],
            d.valley_bowl.radii,
        ),
    ) * 0.8;
    let low = d.valley_bowl.level + 3. * (x * 0.009).sin() * (z * 0.012).cos();
    h = h * (1. - blend) + low * blend;
    for lake in [&d.upper_lake, &d.lower_lake] {
        let r = irregular([x, z], lake.center, lake.radii, lake.phase);
        if r < 1.24 {
            let t = smooth((r - 0.84) / 0.4);
            let bed = lake.level - 9. + 4. * r * r;
            h = h * t + bed * (1. - t);
        }
    }
    let gr = ellipse([x, z], d.garden.center, d.garden.radii);
    if gr < 1. {
        h = h.max(d.upper_lake.level + 5. + 3. * clamp(1. - gr));
    }
    for channel in [&d.falls, &d.river] {
        let (dist, target, _) = route_distance([x, z], &channel.points);
        let bank = channel.width * 0.5 + 20.;
        if dist < bank {
            let t = smooth((dist - channel.width * 0.40) / (bank - channel.width * 0.40));
            let cut = target - 3. + 2. * t;
            h = h.min(h * t + cut * (1. - t));
        }
    }
    // These are the approved broad shoulders, not the open Crystal well floor.
    for &[cx, cz, level, rx, rz] in &d.site_blends {
        let rr = ellipse([x, z], [cx, cz], [rx, rz]);
        let blend = clamp((1.4 - rr) / 0.6);
        h = h * (1. - blend) + (level + 1.5 * noise(x, z)) * blend;
    }
    let fr = &d.frozen_route;
    let (dist, y, _) = route_distance([x, z], &fr.points);
    let blend = smooth((fr.forest_half_width - dist) / 43.)
        * clamp(
            (irregular(
                [x, z],
                d.upper_lake.center,
                d.upper_lake.radii,
                d.upper_lake.phase,
            ) - 0.98)
                / 0.13,
        );
    h = h * (1. - blend) + (y - 2.) * blend;
    let landing = &d.frozen_landing;
    let sr = ellipse([x, z], landing.center, landing.radii);
    let blend = smooth((1.5 - sr) / 0.5);
    h = h * (1. - blend) + landing.height * blend;
    let a = &d.ascent;
    let dx = x - a.center[0];
    let dz = z - a.center[1];
    let hr = dx
        .abs()
        .max((0.5 * dx + 0.866025403784 * dz).abs())
        .max((0.5 * dx - 0.866025403784 * dz).abs());
    if hr < a.well_apothem {
        h = a.base;
    }
    if dist < fr.width * 0.5 + 13.
        && z > a.center[1] + 85.
        && z < a.center[1] + 160.
        && x < a.center[0] + 40.
    {
        h = h.min(y - 3.);
    }
    h
}
pub(super) fn volcano_points(d: &GrandGeographyDocument) -> Vec<[f64; 3]> {
    let [cx, cz] = d.volcano.center;
    d.volcano_route
        .local_points
        .iter()
        .map(|&[x, y, z]| [cx + x, y, cz + z])
        .collect()
}
pub(super) fn volcano(d: &GrandGeographyDocument, point: [f64; 2]) -> f64 {
    let v = &d.volcano;
    let r = irregular(point, v.center, v.radii, v.phase);
    if r >= 1.28 {
        return -1000.;
    }
    let mut h = v.height * clamp(1. - r).powf(1.45) - 5. + 16. * clamp(1. - r / 0.95);
    let cr = ellipse(point, v.center, d.caldera.radii);
    let rim =
        185. + 22. * ((point[1] - v.center[1]).atan2(point[0] - v.center[0]) * 3. + 0.6).sin();
    if cr < 1.3 {
        h = h.max(rim * clamp((1.3 - cr) / 0.3));
    }
    if cr < 0.72 {
        h = d.caldera.floor + 3. * cr;
    }
    let (dist, route, _) = route_distance(point, &volcano_points(d));
    let blend = smooth((61. - dist) / 36.);
    h = h * (1. - blend) + (route - 2.) * blend;
    if cr < 0.72 && dist > 22. {
        h = d.caldera.floor + 3. * cr;
    }
    h
}
/// Actual liquid datum within the continuous basin/channel masks.
pub(super) fn water(d: &GrandGeographyDocument, point: [f64; 2], ground: f64) -> Option<f64> {
    let mut level = None;
    for lake in [&d.upper_lake, &d.lower_lake] {
        if irregular(point, lake.center, lake.radii, lake.phase) < 1. && ground < lake.level {
            level = Some(lake.level);
        }
    }
    for c in [&d.falls, &d.river] {
        let (dist, y, _) = route_distance(point, &c.points);
        if dist < c.width * 0.5 && ground < y {
            level = Some(y);
        }
    }
    level
}
