//! Authored benches, gentle lowland envelopes and connected mountain shoulders.
//!
//! Grand V3 supplies the geography; Forest Expedition supplies the low-relief
//! walking scale. Heights are composed once before caves and voxel publication.
//! The broad lowlands use maxima/minima of bounded slopes, rather than adding
//! overlapping hills whose combined gradient traps the ordinary controller.
use super::*;

pub(super) const SHADOW_FLOOR: i32 = 460;
pub(super) const LIBRARY_FLOOR: i32 = 600;
pub(super) const LAKE_TOP: i32 = 700;
pub(super) const VALLEY_TOP: i32 = 460;

pub(super) fn library_portal(p: WorldHex) -> bool {
    let [x, z] = world_xz(p);
    z > 0. || x > 250. || (x + 400.).hypot(z + 565.) < 24.
}

/// A point on an authored broad shelf or ridge: x, z, exclusive solid height.
type Pin = [f64; 3];

fn nearest_grade(x: f64, z: f64, path: &[Pin]) -> Option<(f64, f64)> {
    path.windows(2)
        .filter_map(|pair| {
            let [a, b] = pair else { return None };
            let [ax, az, ah] = *a;
            let [bx, bz, bh] = *b;
            let dx = bx - ax;
            let dz = bz - az;
            let length2 = dx * dx + dz * dz;
            let t = (((x - ax) * dx + (z - az) * dz) / length2).clamp(0., 1.);
            Some(((x - ax - t * dx).hypot(z - az - t * dz), ah + t * (bh - ah)))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
}

fn hill(x: f64, z: f64, center: Pin, bench: f64, slope: f64) -> f64 {
    let [cx, cz, top] = center;
    top - ((x - cx).hypot(z - cz) - bench).max(0.) * slope
}

fn shelf(h: f64, x: f64, z: f64, path: &[Pin], half_width: f64, shoulder: f64) -> f64 {
    let Some((distance, grade)) = nearest_grade(x, z, path) else {
        return h;
    };
    if distance > half_width + shoulder {
        return h;
    }
    // A generous flat travel surface, then a continuous supporting shoulder.
    // At the outer edge the original relief returns without a cylindrical stamp.
    let d = (distance - half_width).max(0.);
    let target = h.clamp(grade - d * 0.55, grade + d * 0.55);
    target + (h - target) * smooth(d / shoulder)
}

const WESTERN_RIDGE: &[Pin] = &[
    [-535., -300., 650.],
    [-490., -410., 750.],
    [-440., -500., 850.],
    [-410., -585., 902.],
    [-320., -670., 826.],
];
// These are broad landform terraces, not the sole valid strips through lowlands.
const CRYSTAL_SHOULDER: &[Pin] = &[
    [-105., -618., 592.],
    [10., -665., 646.],
    [55., -535., 704.],
    [-45., -450., 760.],
    [-179., -518., 801.],
    [-260., -625., 830.],
    [-400., -610., 860.],
    [-400., -565., 841.],
];
const WESTERN_ASCENT: &[Pin] = &[
    [-170., -115., 505.],
    [-400., -190., 554.],
    [-555., -300., 630.],
    [-550., -430., 716.],
    [-525., -565., 800.],
    [-485., -610., 842.],
    [-400., -610., 860.],
    [-400., -565., 841.],
];
const GARDEN_ASCENT: &[Pin] = &[
    [405., -110., 462.],
    [505., -185., 510.],
    [500., -310., 566.],
    [425., -405., 620.],
    [470., -525., 676.],
    [330., -535., 701.],
    [275., -490., 701.],
];
const VOLCANO_ASCENT: &[Pin] = &[
    [-1099., 465., 401.],
    [-1118., 475., 411.157],
    [-1131., 502., 425.333],
    [-1184., 510., 450.690],
    [-1238., 484., 479.042],
    [-1240., 431., 504.132],
    [-1207., 395., 527.235],
    [-1161., 406., 549.609],
    [-1145., 439., 566.959],
    [-1170., 455., 581.],
];

fn lowlands(x: f64, z: f64) -> f64 {
    let mut h = 414. + (465. - z).clamp(0., 850.) * 0.035;
    for (pin, bench) in [
        ([-60., 125., 522.], 22.),
        ([-60., 225., 501.], 20.),
        ([-240., 220., 477.], 30.),
        ([110., 255., 478.], 26.),
        ([-355., 35., 467.], 35.),
        ([135., 60., 488.], 28.),
        ([-185., -80., 501.], 34.),
        ([265., 180., 460.], 25.),
    ] {
        h = h.max(hill(x, z, pin, bench, 0.36));
    }
    h
}

fn main_body(x: f64, z: f64, depth: u16) -> f64 {
    let mut h = lowlands(x, z);
    // A broad north-facing tilted bench supports the whole highland ensemble.
    // It is intentionally planar in its main direction, with distinct crowns
    // above it; the north is no longer two overlapping spherical hills.
    h = h.max(490. + (-z - 130.).clamp(0., 580.) * 0.55);
    let gallery_shoulder =
        650. - ((z + 348.).abs() - 45.).max(0.) * 0.4 - ((x + 40.).abs() - 360.).max(0.) * 0.3;
    h = h.max(gallery_shoulder);
    let lower_hall_body =
        690. - ((x + 375.).abs() - 100.).max(0.) * 0.5 - ((z + 348.).abs() - 70.).max(0.) * 0.5;
    h = h.max(lower_hall_body);
    // The western body is a connected sequence of broad inclined shelves.
    // Its overburden spans the full library width, rather than lifting only
    // stair columns until the underground switchbacks print onto the surface.
    const MASSIF_CONTOURS: &[(f64, f64)] = &[
        (-300., 640.),
        (-370., 710.),
        (-435., 790.),
        (-480., 846.),
        (-530., 898.),
        (-550., 900.),
        (-567., 848.),
        (-650., 810.),
    ];
    for pair in MASSIF_CONTOURS.windows(2) {
        let [(za, a), (zb, b)] = pair else { continue };
        if (*zb..=*za).contains(&z) {
            let t = (z - za) / (zb - za);
            let top = a + t * (b - a);
            h = h.max(top - ((x + 370.).abs() - 108.).max(0.) * 0.72);
            break;
        }
    }
    if let Some((distance, crest)) = nearest_grade(x, z, WESTERN_RIDGE) {
        h = h.max(crest - distance * 1.35);
        h = h.max(crest.min(765.) - distance * 0.48);
    }
    // Garden and lake occupy a generous highland bench, with a long approach.
    let garden_distance = ((x - 295.).abs() - 82.)
        .max(0.)
        .hypot(((z + 492.).abs() - 53.).max(0.));
    h = h.max(701. - garden_distance * 0.46);
    // Separate sharp northern crowns, confined well beyond ordinary hills.
    for (pin, bench) in [
        ([-540., -590., 902.], 5.),
        ([-470., -685., 928.], 4.),
        ([-325., -695., 866.], 6.),
    ] {
        h = h.max(hill(x, z, pin, bench, 2.5));
    }
    // The entire southern shoreline uses a one-level-per-hex beach envelope.
    // Northern mountain cliffs retain enough body to enclose the library;
    // forcing the lowland beach envelope there exposes the buried stair roofs.
    let beach = h.min(400. + f64::from(depth));
    let mountain_coast = 400. + (h - 400.) * smooth(f64::from(depth) / 12.);
    beach + (mountain_coast - beach) * smooth((-z - 180.) / 180.)
}

fn watercourse(
    p: WorldHex,
    h: &mut f64,
    water: &mut Option<i32>,
    material: &mut &'static str,
    x: f64,
    z: f64,
) {
    let angle = (z + 470.).atan2(x - 330.);
    let lake = ((x - 330.) / 34.).hypot((z + 470.) / 27.)
        / (1. + 0.10 * (angle * 3. + 0.4).sin() + 0.05 * (angle * 5.).cos());
    if lake < 1. {
        *h = f64::from(LAKE_TOP - 10) + 8. * lake.powi(3);
        *water = Some(LAKE_TOP);
        *material = "sand";
    } else if lake < 3. {
        *h = h.min(f64::from(LAKE_TOP + 1) + (lake - 1.) * 27. * 0.4);
    }
    if (-450. ..=-140.).contains(&z) {
        let d = (x - headwater_center(z)).abs();
        let width = 13. + 2.5 * ((z + 450.) / 37.).sin();
        let top = headwater_top(z);
        if river_channel(p) {
            *h = f64::from(top - 5) + 3. * (d / width).powi(3);
            *water = Some(top);
            *material = "stone";
        } else if d < width + 38. {
            // Narrow stream shelves end before the broad highland approach;
            // named falls may be steep without turning a whole hillside into a wall.
            let bank = f64::from(top + 1) + (d - width) * 0.45;
            *h += (bank - *h) * (1. - smooth((d - width - 8.) / 30.));
        }
        for (fall, upper, lower) in [(-370., 700, 620), (-275., 620, 540), (-170., 540, 460)] {
            if (z - fall).abs() < 2.2 && d < width {
                *h = f64::from(lower - 5);
                *water = Some(upper);
            }
        }
    }
    let angle = (z + 115.).atan2(x - 405.);
    let lake = ((x - 405.) / 70.).hypot((z + 115.) / 60.)
        / (1. + 0.14 * (angle * 3. + 0.3).sin() + 0.07 * (angle * 5. - 0.6).cos());
    if river_receiver(p, VALLEY_TOP) {
        *h = f64::from(VALLEY_TOP - 9) + 7. * lake.powi(3);
        *water = Some(VALLEY_TOP);
        *material = "sand";
    } else {
        // A whole valley envelope has no artificial outer mask cliff. Gentle
        // banks open broadly south/east/west; the northern side joins the
        // mountain shoulder on a higher gradient, with its authored ascent.
        let north = (-z - 175.).max(0.);
        let lateral = ((x - 405.).abs() - 70.).max(0.);
        let south = (z + 55.).max(0.);
        let bank = f64::from(VALLEY_TOP + 1) + lateral.hypot(south) * 0.35 + north * 0.8;
        *h = h.min(bank);
    }
    if (-65. ..=650.).contains(&z) {
        let t = ((z + 60.) / 525.).clamp(0., 1.);
        let d = (x - river_center(z)).abs();
        let width = river_width(t);
        let top = river_top(t);
        // Valley slopes are bounded across their whole width; no sudden blend
        // back to a higher hill immediately outside a narrow river corridor.
        *h = h.min(f64::from(top + 1) + (d - width).max(0.) * 0.35);
        if d < width {
            *h = f64::from(top - 5) + 3. * (d / width).powi(3);
            *water = Some(top);
            *material = "sand";
        }
    }
}

pub(super) fn headwater_top(z: f64) -> i32 {
    if z < -370. {
        700
    } else if z < -275. {
        620
    } else if z < -170. {
        540
    } else {
        VALLEY_TOP
    }
}
pub(super) fn river_top(t: f64) -> i32 {
    (f64::from(VALLEY_TOP) - f64::from(VALLEY_TOP - SEA_TOP) * t.clamp(0., 1.)).round() as i32
}
fn river_width(t: f64) -> f64 {
    10. + 12. * t + 3. * (t * std::f64::consts::PI * 5.).sin()
}

pub(super) fn surface(g: &GrandCompiler, p: WorldHex) -> GrandSurface {
    let [x, z] = world_xz(p);
    let depth = grid_value(&g.coast, p, 0);
    let offshore = f64::from(grid_value(&g.offshore_distance, p, 100).min(100));
    let mut h = if depth > 0 {
        main_body(x, z, depth)
    } else {
        400. - offshore
    };
    let mut water = None;
    let mut material = if depth < 10 {
        "sand"
    } else if z < -220. {
        "stone"
    } else {
        "moss"
    };
    if depth > 0 {
        for (path, width, shoulder) in [
            (WESTERN_ASCENT, 20., 42.),
            (GARDEN_ASCENT, 18., 38.),
            (CRYSTAL_SHOULDER, 20., 44.),
        ] {
            h = shelf(h, x, z, path, width, shoulder);
        }
        watercourse(p, &mut h, &mut water, &mut material, x, z);
        if g.crystal.contains(&p) {
            material = "slate";
        }
        if h > 852. {
            material = "snow";
        }
        // An open terrace reaches the library gallery behind the middle fall.
        h = shelf(
            h,
            x,
            z,
            &[[305., -348., 601.], [282., -348., 601.]],
            9.,
            22.,
        );
        // Preserve the horizontal underground routes while adapting their datum.
        if (x + 105.).abs() < 38. && (-150. ..=-70.).contains(&z) {
            let d = (x + 105.).abs();
            let target = f64::from(SHADOW_FLOOR + 1) + (z + 150.).max(0.) * 0.4;
            h += (target - h) * (1. - smooth((d - 10.) / 28.));
        }
    }
    let vr = ((x + 1180.) / 86.).hypot((z - 450.) / 77.);
    if vr < 1. {
        // Broad basal apron, offset higher cone and a real winding ascent.
        h = 400. + (1. - vr) * 100.;
        h = h.max(hill(x, z, [-1190., 440., 610.], 12., 2.6));
        material = if vr > 0.85 { "sand" } else { "basalt" };
    }
    for site in sites::PADS {
        let distance = (x - site.x).hypot(z - site.z);
        if water.is_some() && distance > site.radius {
            continue;
        }
        if distance < site.radius + 48. {
            let target = f64::from(site.level + 1);
            let d = (distance - site.radius).max(0.);
            let bounded = h.clamp(target - d * 0.5, target + d * 0.5);
            h = bounded + (h - bounded) * smooth(d / 48.);
            material = site.material;
            water = None;
        }
    }
    let angle = (z - 472.).atan2(x + 1195.);
    let crater = ((x + 1195.) / 20.).hypot((z - 472.) / 16.) / (1. + 0.08 * (angle * 3.).sin());
    let clear_pad = sites::PADS
        .iter()
        .filter(|site| site.x < -1000.)
        .all(|site| (x - site.x).hypot(z - site.z) > site.radius + 4.);
    if crater < 1.5 && clear_pad {
        let target = if crater < 1. {
            511. + 77. * crater.powi(6)
        } else {
            620. + 8. * (angle * 5.).sin()
        };
        h += (target - h) * (1. - smooth((crater - 1.) / 0.5));
        material = "basalt";
        water = None;
    }
    // Finalize the connected ascent after summit aprons: their broad support
    // envelopes must not lift the low beach back into a wall.
    if vr < 1. {
        h = shelf(h, x, z, VOLCANO_ASCENT, 7., 14.);
        // A landing beach is an area, not merely the first point of the ascent.
        // Its shallow plane reaches inland across several player-width rows.
        if x > -1133. && (z - 465.).abs() < 23. {
            h = 401. + (-1099. - x) * 0.42 + ((z - 465.).abs() - 14.).max(0.) * 0.2;
        }
    }
    if (276. ..=306.).contains(&x) {
        let center = -474. + 4. * ((x - 276.) / 30.) + 1.5 * ((x - 276.) / 9.).sin();
        if (z - center).abs() < 2.2 {
            h = 695.;
            water = Some(LAKE_TOP);
            material = "stone";
        }
    }
    if p.checked_distance(nearest_hex(276., -474.))
        .is_ok_and(|d| d <= 3)
    {
        h = 694.;
        water = Some(LAKE_TOP);
        material = "stone";
    }
    if depth == 0 && vr >= 1. {
        h = (400. - offshore).max(400. + (1. - vr) * 30.);
        water = None;
        material = "sand";
    }
    if depth > 0 {
        if let Some((floor, ceiling)) = library_cavity(p) {
            // Broad exterior shoulders and shrine aprons cannot shave through
            // the reserved interior ceiling. Only the named mouths may open it.
            h = h.max(f64::from(if library_portal(p) {
                floor + 1
            } else {
                ceiling + 8
            }));
        }
        if let Some((floor, _)) = shadow_cavity(p) {
            h = h.max(f64::from(floor + 1));
        }
        if ((x + 105.) / 7.).hypot((z + 618.) / 4.5) < 1. {
            h = 592.;
            material = "slate";
        }
        if (x + 60.).abs() < 7. && (180. ..=225.).contains(&z) {
            h = f64::from(471 + (p.r - 120).clamp(0, 30) as i32);
            material = "sand";
        }
    }
    GrandSurface {
        level: h.floor().clamp(3., 1500.) as i32 - 1,
        material,
        water,
    }
}
