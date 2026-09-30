//! Small compiler-authored labels, independent of distant terrain disclosure.
use super::*;
/// Immutable geometric regions bound to the exact package and source.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrandBiomeMap {
    /// Companion schema version.
    pub version: u32,
    /// Canonical generator input identity.
    pub source_fingerprint: u64,
    /// Sealed package identity.
    pub package_fingerprint: u64,
    /// Exact mainland row spans, including inland water.
    pub mainland_rows: Vec<(i64, i64, i64)>,
    /// Exact independently enlarged Crystal footprint row spans.
    pub crystal_rows: Vec<(i64, i64, i64)>,
    /// Ordered semantic volumes derived from the canonical geography (version2).
    #[serde(default)]
    regions: Vec<BiomeRegion>,
}
const MAX_REGIONS: usize = 128;
const MAX_REGION_POINTS: usize = 4096;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
enum Label {
    Library,
    RootTemple,
    Shadow,
    Volcano,
    Garden,
    Frozen,
    Crystal,
    UpperLake,
    LowerLake,
    Falls,
    River,
    Rainforest,
    Forest,
    Massif,
    Highlands,
}
impl Label {
    fn text(self) -> &'static str {
        match self {
            Self::Library => "Hidden Library",
            Self::RootTemple => "Root Temple",
            Self::Shadow => "Shadow Tunnel",
            Self::Volcano => "Volcanic Island",
            Self::Garden => "Garden of Beginnings",
            Self::Frozen => "Frozen Woods",
            Self::Crystal => "Crystal Ascent",
            Self::UpperLake => "Mountain Lake",
            Self::LowerLake => "Valley Lake",
            Self::Falls => "Waterfall Gorge",
            Self::River => "River Valley",
            Self::Rainforest => "Rainforest",
            Self::Forest => "Forest",
            Self::Massif => "Western Massif",
            Self::Highlands => "Eastern Highlands",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum RegionShape {
    Ellipse {
        center: [f64; 3],
        radii: [f64; 2],
        phase: Option<f64>,
    },
    Room {
        center: [f64; 3],
        half: [f64; 2],
    },
    Hex {
        center: [f64; 3],
        apothem: f64,
    },
    Corridor {
        points: Vec<[f64; 3]>,
        radius: f64,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BiomeRegion {
    label: Label,
    shape: RegionShape,
    /// Half-open offsets above the shape datum (interpolated along corridors).
    height: Option<[f64; 2]>,
}
impl BiomeRegion {
    fn contains(&self, [x, y, z]: [f64; 3]) -> bool {
        let (inside, datum) = match &self.shape {
            RegionShape::Ellipse {
                center: [cx, cy, cz],
                radii,
                phase,
            } => {
                let r = phase.map_or_else(
                    || geography::ellipse([x, z], [*cx, *cz], *radii),
                    // Authored north becomes negative runtime Z; preserve both
                    // harmonics of the source outline through that reflection.
                    |phase| geography::irregular([x, -z], [*cx, -*cz], *radii, phase),
                );
                (r <= 1., *cy)
            }
            RegionShape::Room {
                center: [cx, cy, cz],
                half: [rx, rz],
            } => ((x - cx).abs() <= *rx && (z - cz).abs() <= *rz, *cy),
            RegionShape::Hex {
                center: [cx, cy, cz],
                apothem,
            } => {
                let dx = x - cx;
                let dz = z - cz;
                (
                    dx.abs()
                        .max((0.5 * dx + 0.866_025_403_784 * dz).abs())
                        .max((0.5 * dx - 0.866_025_403_784 * dz).abs())
                        <= *apothem,
                    *cy,
                )
            }
            RegionShape::Corridor { points, radius } => {
                // Multi-turn stairs can overlap in XZ. Match any segment at the
                // correct height, rather than only the first/nearest turn.
                return points.windows(2).any(|pair| {
                    let (Some([ax, ay, az]), Some([bx, by, bz])) = (pair.first(), pair.get(1))
                    else {
                        return false;
                    };
                    let (distance, t) = geography::segment([x, z], [*ax, *az], [*bx, *bz]);
                    distance <= *radius && self.in_height(y, ay + t * (by - ay))
                });
            }
        };
        inside && self.in_height(y, datum)
    }
    fn in_height(&self, y: f64, datum: f64) -> bool {
        self.height
            .is_none_or(|[bottom, top]| y >= datum + bottom && y < datum + top)
    }
}

fn authored_regions(g: &GrandGeography) -> Vec<BiomeRegion> {
    let Some(d) = &g.document else {
        return Vec::new();
    };
    let point = |[x, y, z]: [f64; 3]| {
        let [x, z] = g.world_xz([x, z]);
        [x, f64::from(g.top_level(y)) * LEVEL_HEIGHT, z]
    };
    let mut regions = Vec::new();
    // The open well owns the space above its floor. A tunnel's nominal height
    // envelope can overlap that space as it approaches from underneath, but
    // must not rename the Earth temple or the bottom of Crystal Ascent.
    // Stop at the upper landing so the Frozen Woods arrival keeps its label.
    let [x, z] = d.ascent.center;
    regions.push(BiomeRegion {
        label: Label::Crystal,
        shape: RegionShape::Hex {
            center: point([x, d.ascent.base, z]),
            apothem: g.length(d.ascent.well_apothem),
        },
        height: Some([
            0.,
            f64::from(g.top_level(d.ascent.top) - g.top_level(d.ascent.base)) * LEVEL_HEIGHT,
        ]),
    });
    let label = |layer| match layer {
        SupportLayer::LibraryLower | SupportLayer::LibraryUpper => Label::Library,
        SupportLayer::RootTemple => Label::RootTemple,
        SupportLayer::Shadow => Label::Shadow,
        SupportLayer::CrystalFloor => Label::Crystal,
        SupportLayer::Exterior => Label::Highlands,
    };
    // Caves precede surface labels, and retain their real vertical separation.
    for room in &d.rooms {
        if let Some(frame) = d.frames.get(&room.frame) {
            if let Some(floor) = frame.floor {
                let [x, z] = frame.origin;
                let [rx, rz] = room.half_extents;
                regions.push(BiomeRegion {
                    label: label(frame.layer),
                    shape: RegionShape::Room {
                        center: point([x, floor, z]),
                        half: [g.length(rx), g.length(rz)],
                    },
                    height: Some([0., room.height * d.transform.vertical_scale]),
                });
            }
        }
    }
    let mut corridor = |label, points: &[[f64; 3]], radius, height| {
        regions.push(BiomeRegion {
            label,
            shape: RegionShape::Corridor {
                points: points.iter().copied().map(point).collect(),
                radius: g.length(radius),
            },
            height,
        })
    };
    for route in d.layer_routes.values() {
        corridor(
            label(route.layer),
            &route.points,
            route.width * 0.5,
            Some([0., 36. * LEVEL_HEIGHT]),
        );
    }
    corridor(
        Label::Shadow,
        &d.shadow_route,
        8.,
        Some([0., 36. * LEVEL_HEIGHT]),
    );
    let [vx, vz] = d.volcano.center;
    let volcano_route: Vec<_> = d
        .volcano_route
        .local_points
        .iter()
        .map(|&[x, y, z]| [vx + x, y, vz + z])
        .collect();
    corridor(
        Label::Volcano,
        &volcano_route,
        d.volcano_route.width * 0.5,
        None,
    );
    corridor(
        Label::Frozen,
        &d.frozen_route.points,
        d.frozen_route.forest_half_width,
        Some([-8., 80.]),
    );
    corridor(
        Label::Falls,
        &d.falls.points,
        d.falls.width * 0.5 + 20.,
        None,
    );
    corridor(
        Label::River,
        &d.river.points,
        d.river.width * 0.5 + 20.,
        None,
    );
    let [x, z] = d.ascent.center;
    regions.push(BiomeRegion {
        label: Label::Crystal,
        shape: RegionShape::Hex {
            center: point([x, d.ascent.base, z]),
            apothem: g.length(d.ascent.outer_apothem),
        },
        height: None,
    });
    let mut ellipse = |label: Label, [x, z]: [f64; 2], [rx, rz]: [f64; 2], phase: Option<f64>| {
        regions.push(BiomeRegion {
            label,
            shape: RegionShape::Ellipse {
                center: point([x, 0., z]),
                radii: [g.length(rx), g.length(rz)],
                phase,
            },
            height: None,
        })
    };
    // Island must precede lake. Volcano precedes the mainland-only sea fallback.
    ellipse(Label::Garden, d.garden.center, d.garden.radii, None);
    ellipse(
        Label::UpperLake,
        d.upper_lake.center,
        d.upper_lake.radii,
        Some(d.upper_lake.phase),
    );
    ellipse(
        Label::LowerLake,
        d.lower_lake.center,
        d.lower_lake.radii,
        Some(d.lower_lake.phase),
    );
    ellipse(
        Label::Volcano,
        d.volcano.center,
        d.volcano.radii,
        Some(d.volcano.phase),
    );
    let [rx, rz] = d.forest.radii;
    ellipse(
        Label::Rainforest,
        d.forest.center,
        [rx * 0.55, rz * 0.55],
        None,
    );
    ellipse(Label::Forest, d.forest.center, d.forest.radii, None);
    let [x, z, _, rx, rz] = d.massif;
    ellipse(Label::Massif, [x, z], [rx, rz], None);
    for &[x, z, _, rx, rz] in &d.peaks {
        ellipse(Label::Highlands, [x, z], [rx, rz], None);
    }
    regions
}
fn contains(rows: &[(i64, i64, i64)], p: WorldHex) -> bool {
    let first = rows.partition_point(|(r, _, _)| *r < p.r);
    rows.iter()
        .skip(first)
        .take_while(|(r, _, _)| *r == p.r)
        .any(|(_, a, b)| (*a..=*b).contains(&p.q))
}
/// Same forest envelope used to place tree roots and describe entered regions.
pub(super) fn forest_extent(x: f64, z: f64) -> f64 {
    ((x + 20.) / 480.).hypot((z - 190.) / 360.)
}
impl GrandBiomeMap {
    /// Validate the bounded label companion against the admitted finite world.
    /// Package/source fingerprints are checked by the filesystem-owning caller.
    pub fn validate_in_bounds(&self, radius: u32, levels: [i32; 2]) -> Result<(), ContractError> {
        let invalid =
            || ContractError::new("grand.biomes", "invalid schema, regions or finite bounds");
        let [bottom, top] = levels;
        if !(1..=2).contains(&self.version)
            || radius == 0
            || radius > 4096
            || bottom >= top
            || bottom < -16384
            || top > 16384
            || self.mainland_rows.len() > 10000
            || self.crystal_rows.len() > 1000
            || self.regions.len() > MAX_REGIONS
            || (self.version == 1 && !self.regions.is_empty())
            || (self.version == 2 && self.regions.is_empty())
        {
            return Err(invalid());
        }
        for rows in [&self.mainland_rows, &self.crystal_rows] {
            let mut previous = None;
            for &(r, first, last) in rows {
                if first > last
                    || previous.is_some_and(|(pr, end)| r < pr || (r == pr && first <= end))
                    || [first, last].into_iter().any(|q| {
                        !WorldHex::new(q, r)
                            .checked_distance(WorldHex::new(0, 0))
                            .is_ok_and(|d| d <= u64::from(radius))
                    })
                {
                    return Err(invalid());
                }
                previous = Some((r, last));
            }
        }
        let mut point_count = 0;
        for region in &self.regions {
            if region
                .height
                .is_some_and(|[a, b]| !a.is_finite() || !b.is_finite() || a >= b)
            {
                return Err(invalid());
            }
            let (points, reach): (&[[f64; 3]], f64) = match &region.shape {
                RegionShape::Ellipse {
                    center,
                    radii: [rx, rz],
                    phase,
                } => {
                    if !rx.is_finite()
                        || !rz.is_finite()
                        || *rx <= 0.
                        || *rz <= 0.
                        || phase.is_some_and(|p| !p.is_finite())
                    {
                        return Err(invalid());
                    }
                    (
                        std::slice::from_ref(center),
                        rx.max(*rz) * if phase.is_some() { 1.12 } else { 1. },
                    )
                }
                RegionShape::Room {
                    center,
                    half: [rx, rz],
                } => {
                    if !rx.is_finite() || !rz.is_finite() || *rx <= 0. || *rz <= 0. {
                        return Err(invalid());
                    }
                    (std::slice::from_ref(center), rx.hypot(*rz))
                }
                RegionShape::Hex { center, apothem } => {
                    (std::slice::from_ref(center), apothem * 2. / 3_f64.sqrt())
                }
                RegionShape::Corridor { points, radius } => {
                    if !(2..=256).contains(&points.len()) {
                        return Err(invalid());
                    }
                    (points, *radius)
                }
            };
            point_count += points.len();
            if point_count > MAX_REGION_POINTS || !reach.is_finite() || reach <= 0. {
                return Err(invalid());
            }
            for &[x, y, z] in points {
                if ![x, y, z].into_iter().all(f64::is_finite)
                    || x.abs() > 8192.
                    || z.abs() > 8192.
                    || y < f64::from(bottom) * LEVEL_HEIGHT
                    || y > f64::from(top + 1) * LEVEL_HEIGHT
                {
                    return Err(invalid());
                }
                let distance = nearest_hex(x, z).checked_distance(WorldHex::new(0, 0))?;
                // The inverse hex projection changes by at most distance/1.5.
                if distance as f64 + reach / 1.5 > f64::from(radius) + 1. {
                    return Err(invalid());
                }
                if region.height.is_some_and(|[a, b]| {
                    y + a < f64::from(bottom) * LEVEL_HEIGHT
                        || y + b > f64::from(top + 1) * LEVEL_HEIGHT
                }) {
                    return Err(invalid());
                }
            }
        }
        Ok(())
    }
    /// Resolve the entered authored region, including vertically separated caves.
    #[must_use]
    pub fn label_at(&self, position: [f64; 3]) -> Option<&'static str> {
        let [x, y, z] = position;
        if !position.iter().all(|v| v.is_finite()) {
            return None;
        }
        if !(1..=2).contains(&self.version) {
            return None;
        }
        let p = nearest_hex(x, z);
        if self.version == 2 {
            if let Some(region) = self.regions.iter().find(|region| region.contains(position)) {
                return Some(region.label.text());
            }
            return Some(if !contains(&self.mainland_rows, p) {
                "Open Sea"
            } else if DIRS.iter().any(|(q, r)| {
                !contains(
                    &self.mainland_rows,
                    WorldHex::new(p.q + q * 12, p.r + r * 12),
                )
            }) {
                "Coastal Beach"
            } else {
                "Rolling Lowlands"
            });
        }
        let level = y / LEVEL_HEIGHT;
        if let Some((floor, ceiling)) = library_cavity(p) {
            if level >= f64::from(floor) && level < f64::from(ceiling) {
                return Some(if z > 0. {
                    "Root Temple"
                } else {
                    "Hidden Library"
                });
            }
        }
        if let Some((floor, ceiling)) = shadow_cavity(p) {
            if (f64::from(floor)..f64::from(ceiling)).contains(&level) {
                return Some("Shadow Tunnel");
            }
        }
        if ((x + 1180.) / 86.).hypot((z - 450.) / 77.) <= 1. {
            return Some("Volcanic Island");
        }
        if !contains(&self.mainland_rows, p) {
            return Some("Open Sea");
        }
        if contains(&self.crystal_rows, p) {
            return Some(if z < -590. {
                "Frozen Woods"
            } else {
                "Crystal Ascent"
            });
        }
        if ((x - 275.) / 43.).hypot((z + 490.) / 35.) < 1.5 {
            return Some("Garden of Beginnings");
        }
        if ((x - 330.) / 34.).hypot((z + 470.) / 27.) < 1.35 {
            return Some("Mountain Lake");
        }
        if (-450. ..=-140.).contains(&z) && (x - (330. + (z + 450.) * 0.23)).abs() < 90. {
            return Some("Waterfall Gorge");
        }
        if ((x - 405.) / 63.).hypot((z + 115.) / 62.) < 2.4 {
            return Some("Valley Lake");
        }
        if (-60. ..=490.).contains(&z) {
            let t = ((z + 60.) / 525.).clamp(0., 1.);
            let cx = 400. - 701. * t + 28. * (t * std::f64::consts::PI * 2.).sin();
            if (x - cx).abs() < 90. + 12. * t {
                return Some("River Valley");
            }
        }
        if DIRS.iter().any(|(q, r)| {
            !contains(
                &self.mainland_rows,
                WorldHex::new(p.q + q * 12, p.r + r * 12),
            )
        }) {
            return Some("Coastal Beach");
        }
        let forest = forest_extent(x, z);
        if forest < 1. {
            return Some(if forest < 0.55 {
                "Rainforest"
            } else {
                "Forest"
            });
        }
        Some(if z < -210. {
            if x < 0. {
                "Western Massif"
            } else {
                "Eastern Highlands"
            }
        } else {
            "Rolling Lowlands"
        })
    }
}
impl GrandCompiler {
    /// Generate the small immutable biome companion; no terrain columns are retained.
    pub fn biomes(&self, package_fingerprint: u64) -> GrandBiomeMap {
        let mut grouped: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
        for p in &self.crystal {
            grouped.entry(p.r).or_default().push(p.q);
        }
        let mut rows = vec![];
        for (r, mut qs) in grouped {
            qs.sort();
            let mut qs = qs.into_iter();
            let Some(mut start) = qs.next() else { continue };
            let mut last = start;
            for q in qs {
                if q != last + 1 {
                    rows.push((r, start, last));
                    start = q;
                }
                last = q;
            }
            rows.push((r, start, last));
        }
        GrandBiomeMap {
            version: if self.geography.document.is_some() {
                2
            } else {
                1
            },
            regions: authored_regions(&self.geography),
            source_fingerprint: self.source_fingerprint,
            package_fingerprint,
            mainland_rows: self.source.mainland_rows.clone(),
            crystal_rows: rows,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    fn fixture() -> Result<(GrandGeography, GrandBiomeMap), Box<dyn Error>> {
        let document = serde_json::from_str(include_str!(
            "../../../../../assets/config/v4/grand-v4/geography-r02.json"
        ))?;
        let g = GrandGeography::new(document)?;
        let map = GrandBiomeMap {
            version: 2,
            source_fingerprint: 42,
            package_fingerprint: 43,
            mainland_rows: Vec::new(),
            crystal_rows: Vec::new(),
            regions: authored_regions(&g),
        };
        Ok((g, map))
    }
    fn position(g: &GrandGeography, [x, y, z]: [f64; 3]) -> [f64; 3] {
        let [x, z] = g.world_xz([x, z]);
        [x, f64::from(g.top_level(y)) * LEVEL_HEIGHT + 0.7, z]
    }

    #[test]
    fn canonical_landmarks_and_ascent_use_the_new_coordinate_frame() {
        let (g, map) = fixture().expect("canonical biome fixture");
        let d = g.document.as_ref().expect("missing canonical document");
        let [x, z] = d.garden.center;
        assert_eq!(
            map.label_at(position(&g, [x, d.upper_lake.level + 7., z])),
            Some("Garden of Beginnings")
        );
        let [x, z] = d.volcano.center;
        assert_eq!(
            map.label_at(position(&g, [x, 150., z])),
            Some("Volcanic Island")
        );
        let [x, z] = d.ascent.center;
        assert_eq!(
            map.label_at(position(&g, [x, d.ascent.base, z])),
            Some("Crystal Ascent")
        );
        let (distance, tunnel_floor, _) = geography::route_distance([x, z], &d.shadow_route);
        assert!(distance < 8. && tunnel_floor < d.ascent.base);
        assert_eq!(
            map.label_at(position(&g, [x, tunnel_floor, z])),
            Some("Shadow Tunnel"),
            "the approach below the well keeps its underground label"
        );
        let annulus = (d.ascent.well_apothem + d.ascent.outer_apothem) * 0.5;
        assert_eq!(
            map.label_at(position(&g, [x + annulus, d.ascent.base, z])),
            Some("Crystal Ascent"),
            "the full feature includes its supporting annulus outside the open well"
        );
        let end = d
            .frozen_route
            .points
            .last()
            .copied()
            .expect("missing Frozen shore");
        assert_eq!(map.label_at(position(&g, end)), Some("Frozen Woods"));
        // The overhead Frozen exit must not rename the bottom of the open well.
        let [x, _, z] = d
            .frozen_route
            .points
            .first()
            .copied()
            .expect("missing Frozen exit");
        assert_eq!(
            map.label_at(position(&g, [x, d.ascent.base, z])),
            Some("Crystal Ascent")
        );
        map.validate_in_bounds(1052, [0, 2100])
            .expect("canonical regions fit the finite world");
        let serialized = ron::to_string(&map).expect("serialize canonical regions");
        assert!(
            serialized.len() < 131072,
            "label regions remain a small companion"
        );
        let decoded: GrandBiomeMap = ron::from_str(&serialized).expect("decode canonical regions");
        decoded
            .validate_in_bounds(1052, [0, 2100])
            .expect("decoded regions fit the finite world");
        let [x, z] = d.garden.center;
        assert_eq!(
            decoded.label_at(position(&g, [x, d.upper_lake.level + 7., z])),
            Some("Garden of Beginnings")
        );
    }

    #[test]
    fn stacked_rooms_and_multiturn_routes_preserve_vertical_labels() {
        let (g, map) = fixture().expect("canonical biome fixture");
        let d = g.document.as_ref().expect("missing canonical document");
        for key in ["library_lower", "library_upper", "root_temple"] {
            let frame = d.frames.get(key).expect("missing authored room");
            let [x, z] = frame.origin;
            let floor = frame.floor.expect("missing room floor");
            let expected = if key == "root_temple" {
                "Root Temple"
            } else {
                "Hidden Library"
            };
            assert_eq!(map.label_at(position(&g, [x, floor, z])), Some(expected));
        }
        let room = d.frames.get("library_upper").expect("missing upper room");
        let [x, z] = room.origin;
        assert_ne!(
            map.label_at(position(&g, [x, 250., z])),
            Some("Hidden Library")
        );
        let route = d
            .layer_routes
            .get("library_upper")
            .expect("missing layered route");
        for &p in &route.points {
            assert_eq!(
                map.label_at(position(&g, p)),
                Some("Hidden Library"),
                "every real stair turn is labelled"
            );
        }
        let start = d
            .shadow_route
            .first()
            .copied()
            .expect("missing Shadow route");
        assert_eq!(map.label_at(position(&g, start)), Some("Shadow Tunnel"));
    }

    #[test]
    fn companion_validation_rejects_nonfinite_unbounded_and_ambiguous_data() {
        let (_, map) = fixture().expect("canonical biome fixture");
        let mut bad = map.clone();
        bad.version = 3;
        assert!(bad.validate_in_bounds(1052, [0, 2100]).is_err());
        bad = map.clone();
        bad.regions = vec![map.regions.first().expect("missing regions").clone(); MAX_REGIONS + 1];
        assert!(bad.validate_in_bounds(1052, [0, 2100]).is_err());
        bad = map.clone();
        bad.regions.first_mut().expect("missing region").height = Some([0., f64::NAN]);
        assert!(bad.validate_in_bounds(1052, [0, 2100]).is_err());
        bad = map.clone();
        bad.mainland_rows = vec![(0, 0, 10), (0, 5, 15)];
        assert!(bad.validate_in_bounds(1052, [0, 2100]).is_err());
        assert!(map.validate_in_bounds(64, [0, 2100]).is_err());
        assert!(map.validate_in_bounds(1052, [0, 500]).is_err());
        assert_eq!(map.label_at([f64::NAN, 0., 0.]), None);
    }

    #[test]
    fn legacy_companions_keep_legacy_labels_and_cannot_inject_new_regions() {
        let mut map: GrandBiomeMap = ron::from_str(
            "(version:1,source_fingerprint:1,package_fingerprint:2,mainland_rows:[],crystal_rows:[])",
        ).expect("decode legacy regions");
        map.validate_in_bounds(900, [0, 1600])
            .expect("legacy regions fit the legacy envelope");
        assert_eq!(map.label_at([-1180., 200., 450.]), Some("Volcanic Island"));
        assert_eq!(map.label_at([0., 140., 0.]), Some("Open Sea"));
        map.regions = fixture().expect("canonical biome fixture").1.regions;
        assert!(map.validate_in_bounds(1052, [0, 2100]).is_err());
    }
}
