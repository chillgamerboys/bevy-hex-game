//! Continuous approved landforms, sampled at production hex resolution.
//! No 12-unit study-grid stair steps enter the playable terrain.
use super::geography::{GrandGeographyDocument, ellipse, irregular, route_distance, segment};
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
#[cfg(test)]
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
#[cfg(test)]
pub(super) fn foothill_region(d: &GrandGeographyDocument, point: [f64; 2]) -> bool {
    foothill_weight(d, point) > 0.01
}
/// Frozen continuous landform intersection defines the approved coast. The
/// spatial relief below changes its interior profile, not its positive domain.
pub(super) fn coast_reference(d: &GrandGeographyDocument, [x, z]: [f64; 2]) -> f64 {
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
    finish_terrain(d, [x, z], h)
}

/// Actual mountain interior: broad gentle aprons support smaller steep cores.
/// Coast distance comes from the same exact measured hex footprint used by the
/// compiler; cave cover, detailed columns and overview all call this sampler.
pub(super) fn mainland(d: &GrandGeographyDocument, [x, z]: [f64; 2], coast_distance: f64) -> f64 {
    if coast_distance <= 0. {
        return coast_reference(d, [x, z]);
    }
    let f = &d.foothills;
    let co = &d.coast;
    let r = ellipse([x, z], co.center, co.radii);
    let angle = (z - co.center[1]).atan2(x - co.center[0]);
    let shore = 1.
        + 0.055 * (5. * angle + co.phase).sin()
        + 0.035 * (9. * angle + 0.3).cos()
        + 0.025 * (13. * angle).sin();
    let [fade_start, fade_end] = f.coast_noise_fade;
    let coastal = 1. + (shore - 1.) * smooth((r - fade_start) / (fade_end - fade_start));
    let mut base = 42. * (coastal - r) + f.base_noise * noise(x, z);
    for &[cx, cz, rx, rz] in &co.coves {
        base -= 35. * (-2. * ellipse([x, z], [cx, cz], [rx, rz]).powi(2)).exp();
    }
    for &[cx, cz, height, rx, rz] in &d.low_hills {
        base += height * (-ellipse([x, z], [cx, cz], [rx, rz]).powi(2)).exp();
    }
    base = base.max((coast_distance * 0.07).min(12.));
    let mut apron: f64 = 0.;
    let mut core: f64 = 0.;
    for (i, &[cx, cz, height, rx, rz]) in std::iter::once(&d.massif).chain(&d.peaks).enumerate() {
        let radii = [
            (rx * f.radius_multiplier).max(f.minimum_radius),
            (rz * f.radius_multiplier).max(f.minimum_radius),
        ];
        apron = apron.max(f.apron_relief * smooth(1. - ellipse([x, z], [cx, cz], radii)));
        let angle = (z - cz).atan2(x - cx);
        let radius = ellipse([x, z], [cx, cz], [rx * f.core_setback, rz * f.core_setback])
            / (1. + 0.10 * (3. * angle + i as f64).sin() + 0.04 * (5. * angle).cos());
        core = core.max((height - f.apron_relief).max(0.) * clamp(1. - radius).powf(f.core_power));
    }
    let mut ridge = |a: [f64; 4], b: [f64; 4]| {
        let (distance, t) = segment([x, z], [a[0], a[1]], [b[0], b[1]]);
        let crest = a[2] * (1. - t) + b[2] * t;
        let width = a[3] * (1. - t) + b[3] * t;
        let radius = (width * f.radius_multiplier).max(f.minimum_radius);
        apron = apron.max(f.apron_relief * smooth(1. - distance / radius));
        core = core.max(
            (crest - f.apron_relief).max(0.)
                * clamp(1. - distance / (width * f.core_setback)).powf(f.core_power),
        );
    };
    for &[i, j] in &d.ridge_links {
        if let (Some(a), Some(b)) = (d.peaks.get(i), d.peaks.get(j)) {
            ridge([a[0], a[1], a[2], 170.], [b[0], b[1], b[2], 170.]);
        }
    }
    for path in &d.landform_ridges {
        for pair in path.windows(2) {
            if let (Some(&a), Some(&b)) = (pair.first(), pair.get(1)) {
                ridge(a, b);
            }
        }
    }
    // These are supporting mountain bodies, not isolated summit cones. The
    // basin backing fades before the lower-lake foot, leaving that broad
    // shoulder intact while joining the high shore to the enclosing peaks.
    let [cx, cz, height, rx, rz] = d.headland;
    let [south, north] = f.basin_south_blend;
    core = core.max(
        height
            * (-1.35 * ellipse([x, z], [cx, cz], [rx, rz]).powi(2)).exp()
            * smooth((z - south) / (north - south)),
    );
    for &[cx, cz, height, rx, rz] in &d.site_shoulders {
        core = core.max(height * (-1.35 * ellipse([x, z], [cx, cz], [rx, rz]).powi(2)).exp());
    }
    let mut grade = f.shore_grade;
    for &[cx, cz, extra, rx, rz] in &f.shore_headlands {
        grade += extra * (-2. * ellipse([x, z], [cx, cz], [rx, rz]).powi(2)).exp();
    }
    let low = base.max(apron).min(coast_distance * grade);
    let height = low + core * smooth(coast_distance / f.core_shore_blend);
    finish_terrain(d, [x, z], height)
}

fn finish_terrain(d: &GrandGeographyDocument, [x, z]: [f64; 2], mut h: f64) -> f64 {
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
        } else if dist < bank
            && ![&d.upper_lake, &d.lower_lake]
                .into_iter()
                .any(|lake| irregular([x, z], lake.center, lake.radii, lake.phase) < 1.)
        {
            // A bank must not form a ring dam across the lake at an intake or
            // receiving pool. Gorge reaches also retain a full dry collar:
            // their valley floor can be far below the authored water datum.
            let collar = if target > d.lower_lake.level {
                channel.width * 0.12
            } else {
                0.
            };
            let blend = smooth((bank - dist) / (20. - collar));
            let mut retained_bank = target + 1.;
            // On a short descending reach the nearest centerline datum can be
            // below the water immediately upstream across a hex edge. Retain
            // that adjacent water too; this changes the supporting bank only.
            let scale = d.transform.horizontal_scale;
            for [dx, dz] in [
                [3_f64.sqrt(), 0.],
                [-3_f64.sqrt(), 0.],
                [3_f64.sqrt() * 0.5, 1.5],
                [-3_f64.sqrt() * 0.5, 1.5],
                [3_f64.sqrt() * 0.5, -1.5],
                [-3_f64.sqrt() * 0.5, -1.5],
            ] {
                let (neighbor_distance, neighbor_level, _) =
                    route_distance([x + dx / scale, z + dz / scale], &channel.points);
                if neighbor_distance < half {
                    retained_bank = retained_bank.max(neighbor_level + 0.35);
                }
            }
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
    fn spatial_aprons_keep_the_coast_domain_and_lowland_foot() {
        let d = document();
        assert!(foothill_region(&d, [d.massif[0], d.massif[1]]));
        // Reshaping inside the measured coast cannot create an offshore shelf.
        for p in [[-1100., 0.], [1000., 700.], [0., -650.]] {
            assert!((mainland(&d, p, 0.) - coast_reference(&d, p)).abs() < 1e-9);
        }
        // The basin-support body starts above the broad lower-lake approach.
        let p = [315., 350.];
        let mut without_basin = d.clone();
        without_basin.headland[2] = 0.;
        assert!((mainland(&d, p, 200.) - mainland(&without_basin, p, 200.)).abs() < 1e-9);
    }

    #[test]
    fn coast_center_has_no_angular_height_discontinuity() {
        let d = document();
        let [x, z] = d.coast.center;
        let center = mainland(&d, [x, z], 350.);
        for direction in 0..32_u16 {
            let angle = f64::from(direction) * std::f64::consts::TAU / 32.;
            let height = mainland(&d, [x + angle.cos() * 0.001, z + angle.sin() * 0.001], 350.);
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
            let bed = mainland(&d, point, 200.);
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
