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
}
fn contains(rows: &[(i64, i64, i64)], p: WorldHex) -> bool {
    let first = rows.partition_point(|(r, _, _)| *r < p.r);
    rows[first..]
        .iter()
        .take_while(|(r, _, _)| *r == p.r)
        .any(|(_, a, b)| (*a..=*b).contains(&p.q))
}
/// Same forest envelope used to place tree roots and describe entered regions.
pub(super) fn forest_extent(x: f64, z: f64) -> f64 {
    ((x + 20.) / 480.).hypot((z - 190.) / 360.)
}
impl GrandBiomeMap {
    /// Resolve the entered authored region, including vertically separated caves.
    #[must_use]
    pub fn label_at(&self, position: [f64; 3]) -> Option<&'static str> {
        let [x, y, z] = position;
        if !position.iter().all(|v| v.is_finite()) {
            return None;
        }
        let p = nearest_hex(x, z);
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
        if (x + 105.).abs() < 9. && (-408. ..=-150.).contains(&z) && (520. ..585.).contains(&level)
        {
            return Some("Shadow Tunnel");
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
            let mut start = qs[0];
            let mut last = start;
            for q in qs.into_iter().skip(1) {
                if q != last + 1 {
                    rows.push((r, start, last));
                    start = q;
                }
                last = q;
            }
            rows.push((r, start, last));
        }
        GrandBiomeMap {
            version: 1,
            source_fingerprint: self.source_fingerprint,
            package_fingerprint,
            mainland_rows: self.source.mainland_rows.clone(),
            crystal_rows: rows,
        }
    }
}
