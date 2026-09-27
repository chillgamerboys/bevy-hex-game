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
    // Interior shore/bed refinements must not redefine the frozen coastline.
    finish_terrain_profiled(d, [x, z], h, false)
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
        let relief = if let Some(profile) = d.interior_profile.as_ref().filter(|_| i == 0) {
            // The western body uses the full authored footprint and a rounded
            // summit. It must not recover height by adding the apron twice.
            let radius = radius * f.core_setback / profile.western_radius_multiplier;
            profile.western_relief * smooth(1. - radius).powf(profile.western_power)
        } else {
            (height - f.apron_relief).max(0.) * clamp(1. - radius).powf(f.core_power)
        };
        core = core.max(relief);
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
            // Automatic peak links are lower saddles, as in the approved
            // coast/reference landform. Interpolating full summit height here
            // joins unequal peaks into one nearly level curtain wall.
            ridge(
                [a[0], a[1], a[2] * 0.72, 170.],
                [b[0], b[1], b[2] * 0.72, 170.],
            );
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
    let active_core = core * smooth(coast_distance / f.core_shore_blend);
    let height = d
        .interior_profile
        .as_ref()
        .map_or(low + active_core, |profile| {
            composed_relief(low, active_core, profile.join_width)
        });
    finish_terrain(d, [x, z], height)
}

/// Join the broad foot and upper body without spending the shore slope budget
/// once for each overlapping landform. The bounded blend is continuous and
/// vanishes where either component vanishes; its width is capped near the coast.
fn composed_relief(low: f64, core: f64, join_width: f64) -> f64 {
    let width = join_width.min(2. * low.min(core).max(0.));
    if width <= f64::EPSILON {
        return low.max(core);
    }
    low.max(core) + (width - (low - core).abs()).max(0.).powi(2) / (4. * width)
}

fn finish_terrain(d: &GrandGeographyDocument, [x, z]: [f64; 2], h: f64) -> f64 {
    finish_terrain_profiled(d, [x, z], h, true)
}

fn finish_terrain_profiled(
    d: &GrandGeographyDocument,
    [x, z]: [f64; 2],
    mut h: f64,
    profiles: bool,
) -> f64 {
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
    if profiles {
        h = lake_shore(d, [x, z], h);
    }
    let gr = ellipse([x, z], d.garden.center, d.garden.radii);
    if gr < 1. {
        h = h.max(d.upper_lake.level + 5. + 3. * clamp(1. - gr));
    }
    if profiles {
        if let Some(access) = &d.garden_access {
            let (distance, y, _) = route_distance([x, z], &access.points);
            if gr < 1. {
                // A supported cut into the island approaches the unchanged
                // court along a curve. The layered open treads publish its
                // final ordinary risers; its sides meet the garden soil.
                let weight =
                    smooth((access.width * 0.5 + access.bank_blend - distance) / access.bank_blend);
                h = h * (1. - weight) + (y - 0.35) * weight;
            } else if let Some(start) = access.points.first() {
                let distance = (x - start[0]).hypot(z - start[2]);
                if distance < access.wet_fan_radius
                    && irregular(
                        [x, z],
                        d.upper_lake.center,
                        d.upper_lake.radii,
                        d.upper_lake.phase,
                    ) < 1.
                {
                    // This fan remains submerged and entirely inside the lake;
                    // it creates a swim-to-walk margin, never a shore bridge.
                    let depth = access.wet_edge_depth
                        + (7. - access.wet_edge_depth)
                            * smooth(distance / access.wet_fan_radius).powi(2);
                    h = h.max(d.upper_lake.level - depth);
                }
            }
        }
    }
    for channel in [&d.falls, &d.river] {
        let (dist, target, _) = route_distance([x, z], &channel.points);
        let half = channel.width * 0.5;
        let bank = half + 20.;
        let ordinary = if profiles {
            ordinary_bank_weight(d, [x, z], channel)
        } else {
            0.
        };
        // A sea-level outfall has lateral banks, not a circular levee around
        // its terminal cap. Preserve submerged natural ground beyond the last
        // cross-section so the authored river can join actual ocean columns.
        let open_sea_mouth = channel
            .points
            .last()
            .zip(channel.points.iter().rev().nth(1))
            .is_some_and(|(end, previous)| {
                end[1] <= 0.
                    && h < 0.
                    && (x - end[0]) * (end[0] - previous[0]) + (z - end[2]) * (end[2] - previous[2])
                        >= 0.
            });
        if dist < half {
            // A channel has an authored supported bed even where the previous
            // lowland was lower than it. Merely taking min leaves deep gaps.
            let shelf_depth = d.ordinary_channel_banks.as_ref().map_or(3., |b| {
                0.35 + 2.65 * clamp(1. - dist / half).powf(b.shelf_power)
            });
            h = target - (3. * (1. - ordinary) + shelf_depth * ordinary);
        } else if dist < bank
            && !open_sea_mouth
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
            let lip = d.ordinary_channel_banks.as_ref().map_or(1., |b| b.dry_lip);
            let mut retained_bank = target + 1. - ordinary * (1. - lip);
            // On a short descending reach the nearest centerline datum can be
            // below the water immediately upstream across a hex edge. Retain
            // that adjacent water too; this changes the supporting bank only.
            let scale = d.transform.horizontal_scale;
            let mut adjacent_water: Option<f64> = None;
            for [dx, dz] in [
                [3_f64.sqrt(), 0.],
                [-3_f64.sqrt(), 0.],
                [3_f64.sqrt() * 0.5, 1.5],
                [-3_f64.sqrt() * 0.5, 1.5],
                [3_f64.sqrt() * 0.5, -1.5],
                [-3_f64.sqrt() * 0.5, -1.5],
            ] {
                let neighbor = [x + dx / scale, z + dz / scale];
                let (neighbor_distance, neighbor_level, _) =
                    route_distance(neighbor, &channel.points);
                if neighbor_distance < half {
                    retained_bank = retained_bank.max(neighbor_level + 0.35 * (1. - ordinary));
                    adjacent_water =
                        Some(adjacent_water.map_or(neighbor_level, |old| old.max(neighbor_level)));
                }
                if profiles {
                    // A descending outlet also borders the receiving/source
                    // basin. Its dry side must contain that neighbouring lake
                    // datum, not only the lower nearest channel sample.
                    for lake in [&d.upper_lake, &d.lower_lake] {
                        if irregular(neighbor, lake.center, lake.radii, lake.phase) < 1. {
                            retained_bank = retained_bank.max(lake.level);
                            adjacent_water =
                                Some(adjacent_water.map_or(lake.level, |old| old.max(lake.level)));
                        }
                    }
                }
            }
            h = h * (1. - blend) + retained_bank * blend;
            if profiles {
                // The transition may approach lower natural ground, but the
                // first dry column still contains its actual wet neighbour.
                // Blending this bound away can cross a voxel rounding boundary.
                if let Some(level) = adjacent_water {
                    h = h.max(level);
                }
            }
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

/// Irregular wet/dry margins join their own basin to the surrounding land.
/// The upper shore preserves the exact Frozen arrival; the lower shore has no
/// mountain-route exception and joins the broadly traversable valley floor.
fn lake_shore(d: &GrandGeographyDocument, [x, z]: [f64; 2], h: f64) -> f64 {
    let h = d
        .lower_lake_shore
        .as_ref()
        .map_or(h, |s| lake_bank(&d.lower_lake, s, [x, z], h));
    let Some(s) = &d.upper_lake_shore else {
        return h;
    };
    let shore = lake_bank(&d.upper_lake, s, [x, z], h);
    // Preserve the exact upper stair/forest arrival and its dry landing.
    let (frozen_distance, _, _) = route_distance([x, z], &d.frozen_route.points);
    let route_weight = smooth((frozen_distance - d.frozen_route.width * 0.5 - 5.) / 15.);
    let landing = ellipse([x, z], d.frozen_landing.center, d.frozen_landing.radii);
    let weight = route_weight * smooth((landing - 1.) / 0.5);
    h * (1. - weight) + shore * weight
}

fn lake_bank(
    lake: &super::geography::Lake,
    s: &super::geography::LakeShore,
    [x, z]: [f64; 2],
    h: f64,
) -> f64 {
    let r = irregular([x, z], lake.center, lake.radii, lake.phase);
    if r < 1. && s.wet_width.is_none() {
        return h;
    }
    let dx = x - lake.center[0];
    let dz = z - lake.center[1];
    let angle = (dz / lake.radii[1]).atan2(dx / lake.radii[0]);
    let distance = if r > 1e-9 {
        (r - 1.) * dx.hypot(dz) / r
    } else {
        -lake.radii[0].min(lake.radii[1])
    };
    let variation =
        clamp(0.5 + 0.3 * (2. * angle + s.phase).sin() + 0.2 * (3. * angle - 0.7).cos());
    if distance < 0. {
        let Some((width, edge_depth)) = s.wet_width.zip(s.wet_edge_depth) else {
            return h;
        };
        let width = width[0] + (width[1] - width[0]) * variation;
        // Keep a broad, shallow outer bed before descending into the basin.
        // Squaring the smooth ramp keeps the near-surface gradient gentle;
        // deeper underwater relief need not become a walking staircase.
        let depth = edge_depth + (7. - edge_depth) * smooth(-distance / width).powi(2);
        return lake.level - depth;
    }
    let shelf = s.shelf_width[0] + (s.shelf_width[1] - s.shelf_width[0]) * variation;
    let outer = s.outer_blend[0] + (s.outer_blend[1] - s.outer_blend[0]) * (1. - variation);
    if distance >= shelf + outer {
        return h;
    }
    let target = s.shelf_level + s.shelf_grade * distance.min(shelf);
    let blend = smooth((distance - shelf) / outer);
    target * (1. - blend) + h * blend
}

/// Ordinary channel sides have shallow shelves. Steep waterfall segments and
/// the short region around their ends keep their authored plunge profiles.
fn ordinary_bank_weight(
    d: &GrandGeographyDocument,
    point: [f64; 2],
    channel: &super::geography::Watercourse,
) -> f64 {
    let Some(profile) = &d.ordinary_channel_banks else {
        return 0.;
    };
    let mut closest = f64::INFINITY;
    let mut ordinary = false;
    for pair in channel.points.windows(2) {
        let (Some(a), Some(b)) = (pair.first(), pair.get(1)) else {
            continue;
        };
        let (distance, _) = segment(point, [a[0], a[2]], [b[0], b[2]]);
        let grade = (b[1] - a[1]).abs() / (b[0] - a[0]).hypot(b[2] - a[2]);
        if distance < closest {
            closest = distance;
            ordinary = grade <= profile.max_longitudinal_grade;
        }
    }
    if ordinary {
        let mouth = channel.points.last().map_or(1., |end| {
            if end[1] <= 0. {
                smooth(
                    ((point[0] - end[0]).hypot(point[1] - end[2]) - channel.width * 0.5)
                        / profile.plunge_buffer,
                )
            } else {
                1.
            }
        });
        // A neighbouring plunge must not turn an ordinary reach's shallow
        // side into a submerged wall. The actual steep segment retains its
        // full depth below; regular receiving pools retain a deep centre and
        // shallow lateral margins. Only the ocean terminal keeps its buffer.
        mouth
    } else {
        0.
    }
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
    fn upper_lake_shore_joins_high_ground_without_moving_frozen_arrival() {
        let d = document();
        let lake = &d.upper_lake;
        let mut checked = 0;
        for i in 0..128_u16 {
            let angle = f64::from(i) * std::f64::consts::TAU / 128.;
            let edge =
                1. + 0.08 * (3. * angle + lake.phase).sin() + 0.04 * (5. * angle - 0.7).cos();
            let point = [
                lake.center[0] + lake.radii[0] * angle.cos() * edge * 1.01,
                lake.center[1] + lake.radii[1] * angle.sin() * edge * 1.01,
            ];
            let (distance, _, _) = route_distance(point, &d.frozen_route.points);
            if distance < d.frozen_route.width * 0.5 + 20.
                || ellipse(point, d.frozen_landing.center, d.frozen_landing.radii) < 1.5
            {
                continue;
            }
            let height = lake_shore(&d, point, 300.);
            assert!((206. ..208.).contains(&height), "shore {point:?}: {height}");
            checked += 1;
        }
        assert!(checked > 90, "survey must cover most of the actual shore");
        let point = d.frozen_landing.center;
        assert!((lake_shore(&d, point, 206.) - 206.).abs() < 1e-9);
        assert!((lake_shore(&d, lake.center, 198.) - 198.).abs() < 1e-9);
    }

    #[test]
    fn ordinary_channel_shelves_keep_deep_center_and_intentional_plunges() {
        let d = document();
        let channel = &d.falls;
        let a = channel
            .points
            .iter()
            .rev()
            .nth(1)
            .expect("last reach start");
        let b = channel.points.last().expect("last reach end");
        let length = (b[0] - a[0]).hypot(b[2] - a[2]);
        let normal = [-(b[2] - a[2]) / length, (b[0] - a[0]) / length];
        let center = [(a[0] + b[0]) * 0.5, (a[2] + b[2]) * 0.5];
        let target = (a[1] + b[1]) * 0.5;
        assert!((ordinary_bank_weight(&d, center, channel) - 1.).abs() < 1e-9);
        assert!((finish_terrain(&d, center, 60.) - (target - 3.)).abs() < 1e-9);
        for side in [-1., 1.] {
            let offset = channel.width * 0.5 - 2.;
            let point = [
                center[0] + side * normal[0] * offset,
                center[1] + side * normal[1] * offset,
            ];
            let ground = finish_terrain(&d, point, 60.);
            let expected = target - 0.35 - 2.65 * (2. / (channel.width * 0.5)).powi(2);
            assert!(
                (ground - expected).abs() < 1e-9,
                "wet shelf {point:?}: {ground}"
            );
            let offset = channel.width * 0.5 + 0.5;
            let point = [
                center[0] + side * normal[0] * offset,
                center[1] + side * normal[1] * offset,
            ];
            let ground = finish_terrain(&d, point, 60.);
            assert!(
                (target..target + 0.3).contains(&ground),
                "dry lip {point:?}: {ground}"
            );
        }
        let mut old = d.clone();
        old.ordinary_channel_banks = None;
        for pair in channel.points.windows(2) {
            let (Some(a), Some(b)) = (pair.first(), pair.get(1)) else {
                continue;
            };
            if (b[1] - a[1]).abs() / (b[0] - a[0]).hypot(b[2] - a[2]) <= 0.25 {
                continue;
            }
            let point = [(a[0] + b[0]) * 0.5, (a[2] + b[2]) * 0.5];
            assert!(ordinary_bank_weight(&d, point, channel).abs() < 1e-9);
            assert!(
                (finish_terrain(&d, point, 60.) - finish_terrain(&old, point, 60.)).abs() < 1e-9
            );
        }
    }

    #[test]
    fn lower_lake_margin_keeps_water_outline_and_varied_shore_widths() {
        let d = document();
        let lake = &d.lower_lake;
        let shore = d.lower_lake_shore.as_ref().expect("lower lake shore");
        assert!((lake_bank(lake, shore, lake.center, 90.) - (lake.level - 7.)).abs() < 1e-9);
        let mut inner_heights = Vec::new();
        for i in 0_u16..24 {
            let angle = f64::from(i) * std::f64::consts::TAU / 24.;
            let outline =
                1. + 0.08 * (3. * angle + lake.phase).sin() + 0.04 * (5. * angle - 0.7).cos();
            let radial = [lake.radii[0] * angle.cos(), lake.radii[1] * angle.sin()];
            let radius = radial[0].hypot(radial[1]);
            let at = |distance: f64| {
                [
                    lake.center[0] + radial[0] * (outline + distance / radius),
                    lake.center[1] + radial[1] * (outline + distance / radius),
                ]
            };
            let wet = lake_bank(lake, shore, at(-1.), 90.);
            let dry = lake_bank(lake, shore, at(1.), 90.);
            assert!(wet < lake.level && dry >= lake.level);
            assert!(
                dry - wet < 0.4,
                "near-shore profile at {angle}: {wet}->{dry}"
            );
            assert!((lake_bank(lake, shore, at(140.), 90.) - 90.).abs() < 1e-9);
            inner_heights.push(lake_bank(lake, shore, at(-20.), 90.));
        }
        let minimum = inner_heights.iter().copied().fold(f64::INFINITY, f64::min);
        let maximum = inner_heights
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(
            maximum - minimum > 0.5,
            "the wet shelf must vary around the basin"
        );
    }

    #[test]
    fn emitted_shores_remove_observed_pool_and_lake_lips() {
        use hex_world_contracts::WorldHex;
        let g = super::super::tests::compiler(false);
        // The first pair stopped the unchanged mixed traversal in plain03.
        // The second is the measured northwestern lower-lake mask cliff.
        // These regressions keep actual column authority; swim/ground handoff
        // and complete traversal are verified by the separate app harness.
        for [wet, dry] in [[[666, -90], [667, -90]], [[541, -38], [540, -38]]] {
            let (wet_column, liquid) = g.column(WorldHex::new(wet[0], wet[1]));
            let liquid = liquid.expect("lake or regular receiving-pool water");
            let wet_top = wet_column
                .runs
                .iter()
                .filter(|run| run.material != "water")
                .map(|run| run.top)
                .max()
                .expect("submerged support");
            let (dry_column, dry_liquid) = g.column(WorldHex::new(dry[0], dry[1]));
            assert!(dry_liquid.is_none(), "shore must stay dry");
            let dry_top = dry_column
                .runs
                .iter()
                .map(|run| run.top)
                .max()
                .expect("dry support");
            assert!(dry_top >= liquid.top, "shore still contains adjacent water");
            assert!(
                dry_top.abs_diff(wet_top) <= 1,
                "observed bank {wet:?}->{dry:?}: {wet_top}->{dry_top}"
            );
            eprintln!(
                "SHORE_REPAIR {wet:?}->{dry:?}: bed={wet_top} water={} dry={dry_top}",
                liquid.top
            );
        }
        // The lake's outgoing river used to leave dry banks below the adjacent
        // basin water because it considered only the descending river datum.
        for [wet, dry] in [[[513, 55], [512, 56]], [[526, 59], [525, 60]]] {
            let liquid = g
                .column(WorldHex::new(wet[0], wet[1]))
                .1
                .expect("lake outlet");
            let (column, water) = g.column(WorldHex::new(dry[0], dry[1]));
            assert!(water.is_none(), "existing outlet bank remains dry");
            let top = column
                .runs
                .iter()
                .map(|run| run.top)
                .max()
                .expect("outlet bank");
            assert!(
                top >= liquid.top,
                "dry outlet bank {dry:?} leaks lake water"
            );
        }
    }

    #[test]
    fn bank_profiles_preserve_actual_ocean_outfall_without_new_dry_land() {
        use hex_world_contracts::WorldHex;
        let g = super::super::tests::compiler(false);
        let d = g.geography.document.as_ref().expect("geography");
        let mut old = d.clone();
        old.upper_lake_shore = None;
        old.ordinary_channel_banks = None;
        let end = d.river.points.last().expect("river mouth");
        let center = g.geography.world_hex([end[0], end[2]]);
        let mut positive_bed_changes = 0;
        let mut changed_dry_columns = 0;
        let mut changed_columns = 0;
        for dq in -70_i64..=70 {
            for dr in -70_i64..=70 {
                let p = WorldHex::new(center.q + dq, center.r + dr);
                let point = g.geography.model_xz(p);
                let distance = f64::from(super::super::grid_value(&g.coast, p, 0)) * 1.5;
                let before = mainland(&old, point, distance);
                let before_top = g.geography.top_level(before).max(2);
                let before_water = water(&old, point, before)
                    .map_or(super::super::SEA_TOP, |y| g.geography.top_level(y));
                let (column, liquid) = g.column(p);
                let top = column
                    .runs
                    .iter()
                    .filter(|r| r.material != "water")
                    .map(|r| r.top)
                    .max()
                    .expect("sea bed");
                changed_columns += usize::from(top != before_top);
                positive_bed_changes += usize::from(
                    (top > super::super::SEA_TOP) != (before_top > super::super::SEA_TOP),
                );
                let before_dry = before_top > super::super::SEA_TOP && before_top >= before_water;
                let now_dry = top > super::super::SEA_TOP && liquid.is_none();
                changed_dry_columns += usize::from(now_dry != before_dry);
                assert!(
                    !now_dry || before_dry,
                    "bank profile created dry land at {p:?}"
                );
            }
        }
        for [q, r] in [[233, 419], [232, 420], [231, 420]] {
            let liquid = g.column(WorldHex::new(q, r)).1.expect("receiving ocean");
            assert_eq!(liquid.body_id, "grand/ocean");
            assert_eq!(liquid.top, super::super::SEA_TOP);
        }
        eprintln!(
            "mouth audit: changed solid columns={changed_columns}, changed positive-bed membership={positive_bed_changes}, changed dry membership={changed_dry_columns}; new dry columns=0"
        );
    }

    #[test]
    fn bank_profiles_report_actual_shallow_entry_bands() {
        use hex_world_contracts::WorldHex;
        use std::collections::{BTreeMap, BTreeSet};
        let g = super::super::tests::compiler(false);
        let d = g.geography.document.as_ref().expect("geography");
        let neighbors = [[1, 0], [0, 1], [-1, 1], [-1, 0], [0, -1], [1, -1]];
        for (name, channel) in [("falls", &d.falls), ("river", &d.river)] {
            let mut band = BTreeSet::new();
            for pair in channel.points.windows(2) {
                let (Some(a), Some(b)) = (pair.first(), pair.get(1)) else {
                    continue;
                };
                let a = g.geography.world_hex([a[0], a[2]]);
                let b = g.geography.world_hex([b[0], b[2]]);
                for q in a.q.min(b.q) - 16..=a.q.max(b.q) + 16 {
                    for r in a.r.min(b.r) - 16..=a.r.max(b.r) + 16 {
                        let p = WorldHex::new(q, r);
                        let point = g.geography.model_xz(p);
                        let (distance, _, _) = route_distance(point, &channel.points);
                        if (distance - channel.width * 0.5).abs() <= 5.
                            && ordinary_bank_weight(d, point, channel) > 0.999
                            && [&d.upper_lake, &d.lower_lake]
                                .iter()
                                .all(|l| irregular(point, l.center, l.radii, l.phase) >= 1.)
                        {
                            band.insert(p);
                        }
                    }
                }
            }
            let mut cache = BTreeMap::new();
            for &p in &band {
                for [dq, dr] in neighbors.into_iter().chain([[0, 0]]) {
                    let n = WorldHex::new(p.q + dq, p.r + dr);
                    cache.entry(n).or_insert_with(|| {
                        let (column, liquid) = g.column(n);
                        let top = column
                            .runs
                            .iter()
                            .filter(|r| r.material != "water")
                            .map(|r| r.top)
                            .max()
                            .expect("channel terrain");
                        (top, liquid)
                    });
                }
            }
            let mut entry = BTreeMap::<i32, usize>::new();
            let mut shallow = BTreeMap::<i32, usize>::new();
            let mut examples = Vec::new();
            for &p in &band {
                let (top, liquid) = cache.get(&p).expect("cached band");
                for [dq, dr] in neighbors {
                    let n = WorldHex::new(p.q + dq, p.r + dr);
                    let (other_top, other) = cache.get(&n).expect("cached neighbor");
                    if let Some(wet) = other {
                        if wet.top - wet.bottom > 4 {
                            continue;
                        }
                        if liquid.is_none() {
                            let delta = (*top - wet.bottom).abs();
                            *entry.entry(delta).or_default() += 1;
                            if delta > 1 && examples.len() < 4 {
                                examples.push((p, n, *top, wet.bottom, wet.top));
                            }
                        } else if liquid.as_ref().is_some_and(|l| l.top - l.bottom <= 4) {
                            *shallow.entry((*top - *other_top).abs()).or_default() += 1;
                        }
                    }
                }
            }
            assert!(
                !entry.is_empty() && !shallow.is_empty(),
                "complete regular {name} banks must be surveyed"
            );
            eprintln!(
                "bank audit {name}: band={} dry-to-shallow level differences={entry:?}; shallow-band differences={shallow:?}; >1level examples={examples:?}",
                band.len()
            );
        }
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
    fn composed_aprons_remove_the_observed_lower_foot_step_barriers() {
        use hex_world_contracts::WorldHex;
        let g = super::super::tests::compiler(false);
        // These exact dry edges stopped the production controller on plain02.
        // This is a terrain regression; movement still needs the app harness.
        for [a, b] in [
            [[185, -60], [186, -60]],
            [[164, -100], [165, -100]],
            [[165, -100], [166, -100]],
        ] {
            let tops = [a, b].map(|[q, r]| {
                let (column, liquid) = g.column(WorldHex::new(q, r));
                assert!(liquid.is_none(), "observed dry foothill became water");
                column.runs.iter().map(|r| r.top).max().expect("dry ground")
            });
            let [left, right] = tops;
            assert!(
                left.abs_diff(right) <= 1,
                "lower foot {a:?}->{b:?}: {tops:?}"
            );
            eprintln!("COMPOSED_FOOT {a:?}->{b:?} {tops:?}");
        }
    }

    #[test]
    fn automatic_peak_links_keep_saddles_below_adjacent_summits() {
        let g = super::super::tests::compiler(false);
        let d = g.geography.document.as_ref().expect("geography");
        let top = |point| {
            g.column(g.geography.world_hex(point))
                .0
                .runs
                .into_iter()
                .filter(|run| run.material != "water")
                .map(|run| run.top)
                .max()
                .expect("solid mountain")
        };
        for &[i, j] in &d.ridge_links {
            let a = d.peaks.get(i).expect("admitted peak link");
            let b = d.peaks.get(j).expect("admitted peak link");
            let left = top([a[0], a[1]]);
            let right = top([b[0], b[1]]);
            let saddle = top([(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]);
            assert!(
                saddle < left.min(right),
                "automatic ridge {i}->{j} joins summit levels {left}/{right} into a wall at {saddle}"
            );
        }
    }

    #[test]
    fn mountain_lake_is_screened_from_authored_valley_eyes() {
        use super::super::{nearest_hex, world_xz, LEVEL_HEIGHT};
        let g = super::super::tests::compiler(false);
        let d = g.geography.document.as_ref().expect("geography");
        let center = g.geography.world_hex(d.upper_lake.center);
        let water_top = g.geography.top_level(d.upper_lake.level);
        let mut targets = Vec::new();
        // Sample actual liquid columns across the basin, including its banks.
        // These are terrain sight lines, not a claim about native camera feel.
        for q in -120_i64..=120 {
            for r in -120_i64..=120 {
                let p = hex_world_contracts::WorldHex::new(center.q + q, center.r + r);
                if p.q.rem_euclid(8) != 0
                    || p.r.rem_euclid(8) != 0
                    || irregular(
                        g.geography.model_xz(p),
                        d.upper_lake.center,
                        d.upper_lake.radii,
                        d.upper_lake.phase,
                    ) >= 1.
                {
                    continue;
                }
                if g.column(p).1.is_some_and(|water| water.top == water_top) {
                    targets.push(world_xz(p));
                }
            }
        }
        assert!(
            targets.len() > 100,
            "screening must cover the lake, not one point"
        );
        for id in [
            "grand-valley-tree-bank",
            "grand-valley-lake-bank",
            "grand-valley-waterfall-approach",
        ] {
            let camera = d.review_cameras.get(id).expect("authored valley eye");
            let eye_column = g.geography.world_hex([camera.eye[0], camera.eye[2]]);
            let eye_top = g
                .column(eye_column)
                .0
                .runs
                .into_iter()
                .filter(|run| run.material != "water")
                .map(|run| run.top)
                .max()
                .expect("dry bank");
            let [ex, ez] = world_xz(eye_column);
            // An eye above the ordinary player's actual eye is conservative.
            let ey = f64::from(eye_top) * LEVEL_HEIGHT + 1.7;
            let water_y = f64::from(water_top) * LEVEL_HEIGHT;
            for &[tx, tz] in &targets {
                let blocked = (1..192_u16).any(|step| {
                    let t = f64::from(step) / 192.;
                    let p = nearest_hex(ex + (tx - ex) * t, ez + (tz - ez) * t);
                    let level = (ey + (water_y - ey) * t) / LEVEL_HEIGHT;
                    g.column(p).0.runs.iter().any(|run| {
                        run.material != "water"
                            && f64::from(run.bottom) <= level
                            && level < f64::from(run.top)
                    })
                });
                assert!(blocked, "lake water exposed from {id} toward [{tx},{tz}]");
            }
        }
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
    fn sea_level_outfall_keeps_its_ocean_side_open() {
        let d = document();
        // Final emitted river cap and its three seaward neighbors. These cells
        // previously surrounded the river's sea-level pool with a dry bank.
        for [q, r] in [[233, 419], [232, 420], [231, 420]] {
            let [x, z] = super::super::world_xz(hex_world_contracts::WorldHex::new(q, r));
            let point = [
                (x - d.transform.translation[0]) / d.transform.horizontal_scale,
                -(z - d.transform.translation[1]) / d.transform.horizontal_scale,
            ];
            let bed = mainland(&d, point, 0.);
            assert!(bed < 0., "ocean mouth blocked at {point:?}: {bed}");
            assert!(
                water(&d, point, bed).is_none(),
                "receiving water must be ocean"
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
