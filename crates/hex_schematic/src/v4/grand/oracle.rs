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
/// Shared actual hexagonal Crystal feature boundary, including its enclosing rock.
pub(super) fn crystal_distance(d: &GrandGeographyDocument, point: [f64; 2]) -> f64 {
    let dx = point[0] - d.ascent.center[0];
    let dz = point[1] - d.ascent.center[1];
    dx.abs()
        .max((0.5 * dx + 0.866025403784 * dz).abs())
        .max((0.5 * dx - 0.866025403784 * dz).abs())
}
/// Broad source-owned mountain apron. It is shared with surface acceptance;
/// this mask does not select particular routes or omit awkward joins.
pub(super) fn foothill_weight(d: &GrandGeographyDocument, point: [f64; 2]) -> f64 {
    let f = &d.foothills;
    let mut apron: f64 = 0.;
    for &[cx, cz, _, rx, rz] in std::iter::once(&d.massif)
        .chain(std::iter::once(&d.headland))
        .chain(&d.peaks)
        .chain(&d.site_shoulders)
    {
        let r = ellipse(
            point,
            [cx, cz],
            [
                (rx * f.radius_multiplier).max(f.minimum_radius),
                (rz * f.radius_multiplier).max(f.minimum_radius),
            ],
        );
        apron = apron.max(f.apron_relief * smooth(1. - r));
    }
    for path in &d.landform_ridges {
        for pair in path.windows(2) {
            let (Some(a), Some(b)) = (pair.first(), pair.get(1)) else {
                continue;
            };
            let (distance, t) = segment(point, [a[0], a[1]], [b[0], b[1]]);
            let radius = ((a[3] * (1. - t) + b[3] * t) * f.radius_multiplier).max(f.minimum_radius);
            apron = apron.max(f.apron_relief * smooth(1. - distance / radius));
        }
    }
    smooth(apron / (f.apron_relief * 0.35))
}
/// Complete broad base envelope; water and cave overlaps are reported separately
/// by final-column surveys rather than removed from this authoring region.
pub(super) fn foothill_region(d: &GrandGeographyDocument, point: [f64; 2]) -> bool {
    foothill_weight(d, point) > 0.01
}
fn foothill_height(d: &GrandGeographyDocument, point: [f64; 2], h: f64) -> f64 {
    let f = &d.foothills;
    if h <= f.base_level || h >= f.restored_height || !foothill_region(d, point) {
        return h;
    }
    let delta = h - f.base_level;
    // The exponential toe keeps both value and first derivative continuous.
    let lower = f.base_level
        + delta * f.compression
        + (1. - f.compression) * f.toe_blend_height * (1. - (-delta / f.toe_blend_height).exp());
    let restore = smooth((h - f.compressed_height) / (f.restored_height - f.compressed_height));
    let target = lower + (h - lower) * restore;
    h + (target - h) * foothill_weight(d, point)
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
    // Angular coastline detail belongs near the shore. Carrying it to r=0
    // produces a finite directional jump and a radial fan in the valley.
    let fade = d.foothills.coast_noise_fade;
    let coastal_shore = 1. + (shore - 1.) * smooth((r - fade[0]) / (fade[1] - fade[0]));
    let mut h = 42. * (coastal_shore - r) + 2.6 * noise(x, z);
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
    // These are the approved broad shoulders, not the open Crystal well floor.
    for &[cx, cz, level, rx, rz] in &d.site_blends {
        let rr = ellipse([x, z], [cx, cz], [rx, rz]);
        let blend = clamp((1.4 - rr) / 0.6);
        h = h * (1. - blend) + (level + 1.5 * noise(x, z)) * blend;
    }
    // The whole ascent includes its enclosing rock. This source-owned envelope
    // blends into the mountain shoulder; the smaller well is carved below.
    let ascent = &d.ascent;
    let distance = crystal_distance(d, [x, z]);
    if distance >= ascent.well_apothem && distance < ascent.outer_apothem {
        let weight = smooth(
            (ascent.outer_apothem - distance) / (ascent.outer_apothem - ascent.well_apothem),
        );
        let crest = ascent.top + 12. + 5. * noise(x, z);
        h = h.max(h * (1. - weight) + crest * weight);
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
    // Apply the broad lower profile after all authored shoulder joins. The
    // positive low coast and elevations above the restoration datum are exact.
    h = foothill_height(d, [x, z], h);
    for lake in [&d.upper_lake, &d.lower_lake] {
        let r = irregular([x, z], lake.center, lake.radii, lake.phase);
        if r < 1. {
            // Shallow wet shelf reaches the actual bank. The dry collar above
            // the water datum encloses every production-resolution shoreline.
            let edge = smooth((r - 0.82) / 0.18);
            h = lake.level - 7. + 6. * edge;
        } else if r < 1.4 {
            let bank = lake.level + 1.;
            let blend = smooth((1.4 - r) / 0.3);
            h = h.max(bank * blend + h * (1. - blend));
        }
    }
    let gr = ellipse([x, z], d.garden.center, d.garden.radii);
    if gr < 1. {
        h = h.max(d.upper_lake.level + 5. + 3. * clamp(1. - gr));
    }
    for channel in [&d.falls, &d.river] {
        let (dist, target, _) = route_distance([x, z], &channel.points);
        let half = channel.width * 0.5;
        let bank = half + 20.;
        if dist < half {
            // A channel has an authored supported bed even where the previous
            // lowland was lower than it. Merely taking min leaves deep gaps.
            h = target - 3.;
        } else if dist < bank {
            let blend = smooth((bank - dist) / 20.);
            let retained_bank = target + 1.;
            h = h * (1. - blend) + retained_bank * blend;
        }
    }
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
    let rim = d.caldera.rim
        + d.caldera.amplitude
            * ((point[1] - v.center[1]).atan2(point[0] - v.center[0]) * 3. + 0.6).sin();
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

#[cfg(test)]
mod profile_tests {
    use super::*;

    fn document() -> GrandGeographyDocument {
        serde_json::from_str(include_str!(
            "../../../../../assets/config/v4/grand-v4/geography-r02.json"
        ))
        .expect("canonical geography")
    }

    #[test]
    fn broad_foothill_curve_preserves_datums_and_has_smooth_joins() {
        let d = document();
        let p = [d.massif[0], d.massif[1]];
        let f = &d.foothills;
        for height in [-10., 0., f.base_level, f.restored_height, 425.] {
            assert!((foothill_height(&d, p, height) - height).abs() < 1e-9);
        }
        for height in [f.base_level, f.compressed_height, f.restored_height] {
            let step = 0.0001;
            let center = foothill_height(&d, p, height);
            let left = (center - foothill_height(&d, p, height - step)) / step;
            let right = (foothill_height(&d, p, height + step) - center) / step;
            assert!(
                (left - right).abs() < 0.001,
                "slope join at {height}: {left}/{right}"
            );
        }
        let mut previous = foothill_height(&d, p, 0.);
        for level in 1..=500_u16 {
            let current = foothill_height(&d, p, f64::from(level));
            assert!(current > previous, "height transfer folds at {level}");
            previous = current;
        }
        assert!(foothill_height(&d, p, f.compressed_height) < f.apron_relief);
    }

    #[test]
    fn coast_center_has_no_angular_height_discontinuity() {
        let d = document();
        let [x, z] = d.coast.center;
        let center = mainland(&d, [x, z]);
        for direction in 0..32_u16 {
            let angle = f64::from(direction) * std::f64::consts::TAU / 32.;
            let height = mainland(&d, [x + angle.cos() * 0.001, z + angle.sin() * 0.001]);
            assert!(
                (height - center).abs() < 0.01,
                "directional jump: {height} vs {center}"
            );
        }
    }

    #[test]
    fn main_plunge_has_wet_supported_receiving_neighbors() {
        let d = document();
        let t = d.transform;
        let mut receiving = 0;
        for [q, r] in [[744, -175], [744, -174], [745, -175], [743, -174]] {
            let p = hex_world_contracts::WorldHex::new(q, r);
            let [x, z] = super::super::world_xz(p);
            let point = [
                (x - t.translation[0]) / t.horizontal_scale,
                -(z - t.translation[1]) / t.horizontal_scale,
            ];
            let bed = mainland(&d, point);
            if let Some(level) = water(&d, point, bed) {
                assert!(bed < level);
                receiving += 1;
            }
        }
        assert_eq!(
            receiving, 4,
            "plunge outlet must not terminate above dry neighbor columns"
        );
    }
}
