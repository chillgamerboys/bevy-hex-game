//! Grand's full sevenfold mainland, compiled a single compact chunk at a time.
#![expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Finite authored geometry is bounded by radius900 and1600levels; rounding is the voxelization contract."
)]
mod biomes;
mod dressing;
pub use biomes::GrandBiomeMap;
mod sites;
#[cfg(test)]
mod tests;
use super::northern::{nearest_hex, world_xz, IslandSpec, NorthernOverview};
use hex_world_contracts::*;
use serde::{Deserialize, Serialize};
pub use sites::GrandSites;
use std::collections::{BTreeMap, VecDeque};
/// Finite ocean envelope, independent of the measured mainland area.
pub const RADIUS: i64 = 900;
/// World units per voxel level.
pub const LEVEL_HEIGHT: f64 = 0.35;
/// Exclusive ocean surface level.
pub const SEA_TOP: i32 = 400;
/// Inclusive vertical storage bound.
pub const MAX_LEVEL: i32 = 1600;
const GRID: usize = 1101;
const OFFSET: i64 = 550;
const DIRS: [(i64, i64); 6] = [(1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1)];
/// Small measured authoring input. Rows are (r, first q, last q), inclusive.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrandSpec {
    /// Source schema version.
    pub version: u32,
    /// Whether globally reserved landmarks and vegetation are included.
    pub full_dressing: bool,
    /// Stable immutable world identity.
    pub id: String,
    /// Deterministic variation seed.
    pub seed: u64,
    /// Measured canonical coast-enclosed mainland area.
    pub canonical_mainland_columns: usize,
    /// Original embedded Crystal Ascent footprint, measured separately.
    pub canonical_crystal_columns: usize,
    /// Inclusive canonical row spans of the enlarged mainland footprint.
    pub mainland_rows: Vec<(i64, i64, i64)>,
}
/// Composed exact terrain and optional inland-water upper surface.
#[derive(Clone, Copy, Debug)]
pub struct GrandSurface {
    /// Topmost solid voxel level.
    pub level: i32,
    /// Stable surface material name.
    pub material: &'static str,
    /// Exclusive inland liquid surface when present.
    pub water: Option<i32>,
}
/// Pure chunk authoring with small global footprint and object reservations.
pub struct GrandCompiler {
    /// Validated authoring source.
    pub source: GrandSpec,
    /// Canonical authoring identity.
    pub source_fingerprint: u64,
    /// Stable material policies and presentation colors.
    pub materials: Vec<MaterialSpec>,
    /// Exact enlarged coast-enclosed area in hex columns.
    pub mainland_columns: usize,
    /// Exact independently enlarged Crystal footprint in hex columns.
    pub crystal_columns: usize,
    /// Number of globally reserved tree objects.
    pub tree_count: usize,
    coast: Vec<u16>,
    offshore_distance: Vec<u16>,
    crystal: std::collections::BTreeSet<WorldHex>,
    anchors: Vec<WorldAnchor>,
    objects: BTreeMap<ChunkId, Vec<ObjectInstance>>,
    influences: BTreeMap<ChunkId, Vec<ObjectInfluence>>,
}
fn index(p: WorldHex) -> Option<usize> {
    let q = usize::try_from(p.q + OFFSET).ok()?;
    let r = usize::try_from(p.r + OFFSET).ok()?;
    (q < GRID && r < GRID).then_some(r * GRID + q)
}
fn smooth(t: f64) -> f64 {
    let t = t.clamp(0., 1.);
    t * t * (3. - 2. * t)
}
fn gaussian(x: f64, z: f64, cx: f64, cz: f64, rx: f64, rz: f64) -> f64 {
    (-((x - cx) / rx).powi(2) - ((z - cz) / rz).powi(2)).exp()
}
fn run(bottom: i32, top: i32, material: &str) -> VoxelRun {
    VoxelRun {
        bottom,
        top,
        material: material.into(),
    }
}
fn cut(runs: &mut Vec<VoxelRun>, bottom: i32, top: i32) {
    *runs = std::mem::take(runs)
        .into_iter()
        .flat_map(|r| {
            let mut out = Vec::with_capacity(2);
            if r.bottom < bottom {
                out.push(run(r.bottom, r.top.min(bottom), &r.material));
            }
            if r.top > top {
                out.push(run(r.bottom.max(top), r.top, &r.material));
            }
            out
        })
        .filter(|r| r.bottom < r.top)
        .collect();
}
impl GrandCompiler {
    /// Validate measured area, build coastal-distance field and reserve exact objects.
    pub fn new(source: GrandSpec) -> Result<Self, ContractError> {
        if source.version != 1
            || source.id != "grand-v4"
            || source.canonical_mainland_columns != 93326
            || source.canonical_crystal_columns != 3169
        {
            return Err(ContractError::new(
                "grand",
                "wrong canonical geometry or identity",
            ));
        }
        let mut coast = vec![0_u16; GRID * GRID];
        let mut count = 0;
        for &(r, a, b) in &source.mainland_rows {
            if a > b {
                return Err(ContractError::new("grand", "inverted footprint row"));
            }
            for q in a..=b {
                let i = index(WorldHex::new(q, r)).ok_or_else(|| {
                    ContractError::new("grand", "footprint outside measured bounds")
                })?;
                if coast[i] != 0 {
                    return Err(ContractError::new("grand", "overlapping footprint rows"));
                }
                coast[i] = u16::MAX;
                count += 1;
            }
        }
        if count != source.canonical_mainland_columns * 7 {
            return Err(ContractError::new(
                "grand",
                "mainland must be exactly seven times canonical area",
            ));
        }
        let mut queue = VecDeque::new();
        for r in -OFFSET..=OFFSET {
            for q in -OFFSET..=OFFSET {
                let p = WorldHex::new(q, r);
                let i = index(p).unwrap();
                if coast[i] != 0
                    && DIRS.iter().any(|(a, b)| {
                        index(WorldHex::new(q + a, r + b)).is_none_or(|j| coast[j] == 0)
                    })
                {
                    coast[i] = 1;
                    queue.push_back(p);
                }
            }
        }
        while let Some(p) = queue.pop_front() {
            let depth = coast[index(p).unwrap()];
            for (a, b) in DIRS {
                let n = WorldHex::new(p.q + a, p.r + b);
                if let Some(i) = index(n) {
                    if coast[i] == u16::MAX {
                        coast[i] = depth + 1;
                        queue.push_back(n);
                    }
                }
            }
        }
        let mut offshore_distance = vec![u16::MAX; GRID * GRID];
        let mut offshore = VecDeque::new();
        for r in -OFFSET..=OFFSET {
            for q in -OFFSET..=OFFSET {
                let p = WorldHex::new(q, r);
                let i = index(p).unwrap();
                if coast[i] > 0 {
                    offshore_distance[i] = 0;
                    if coast[i] == 1 {
                        offshore.push_back(p);
                    }
                }
            }
        }
        while let Some(p) = offshore.pop_front() {
            let d = offshore_distance[index(p).unwrap()];
            if d >= 100 {
                continue;
            }
            for (a, b) in DIRS {
                let n = WorldHex::new(p.q + a, p.r + b);
                if let Some(i) = index(n) {
                    if offshore_distance[i] == u16::MAX {
                        offshore_distance[i] = d + 1;
                        offshore.push_back(n);
                    }
                }
            }
        }
        let root = nearest_hex(-201., -524.);
        let mut candidates = Vec::new();
        for q in -95_i64..=95 {
            for r in -95_i64..=95 {
                if q.abs().max(r.abs()).max((q + r).abs()) <= 95 {
                    candidates.push((q * q + q * r + r * r, q, r));
                }
            }
        }
        candidates.sort();
        let crystal = candidates
            .into_iter()
            .take(source.canonical_crystal_columns * 7)
            .map(|(_, q, r)| WorldHex::new(root.q + q, root.r + r))
            .collect();
        let source_fingerprint = hash_serializable(&source)?;
        let mut result = Self {
            source,
            source_fingerprint,
            materials: palette(),
            mainland_columns: count,
            crystal_columns: 22183,
            tree_count: 0,
            coast,
            offshore_distance,
            crystal,
            anchors: vec![],
            objects: BTreeMap::new(),
            influences: BTreeMap::new(),
        };
        result.anchors = result.make_anchors();
        let objects = if result.source.full_dressing {
            dressing::compose(&result)?
        } else {
            vec![]
        };
        result.tree_count = objects
            .iter()
            .filter(|o| o.asset.starts_with("plant/"))
            .count();
        for object in objects {
            for (chunk, influence) in object.influences()? {
                result.influences.entry(chunk).or_default().push(influence);
            }
            result
                .objects
                .entry(object.origin.column.chunk())
                .or_default()
                .push(object);
        }
        Ok(result)
    }
    /// Whether the column is inside the authored mainland footprint.
    pub fn mainland(&self, p: WorldHex) -> bool {
        index(p).is_some_and(|i| self.coast[i] > 0)
    }
    /// Quantized solid surface before exact caves and above-ground structures.
    pub fn surface(&self, p: WorldHex) -> GrandSurface {
        let [x, z] = world_xz(p);
        let depth = index(p).map_or(0, |i| self.coast[i]);
        let mut h = if depth > 0 {
            let base = 480.
                + 45. * gaussian(x, z, -60., 125., 220., 180.)
                + 14. * (x * 0.006).sin() * (z * 0.004).cos()
                + 9. * (x * 0.019 + z * 0.009).sin();
            let mountains = 530. * gaussian(x, z, -410., -420., 250., 225.)
                + 240. * gaussian(x, z, -400., -570., 150., 125.);
            let saddle = 290. * gaussian(x, z, 20., -350., 520., 160.);
            let eastern = 430. * gaussian(x, z, 320., -490., 240., 180.);
            401. + (base + mountains + saddle + eastern - 401.) * smooth(f64::from(depth) / 28.)
        } else {
            let distance = index(p).map_or(100, |i| self.offshore_distance[i]).min(100);
            400. - 3. * f64::from(distance)
                + 10. * (x * 0.003 + z * 0.005).sin() * smooth(f64::from(distance) / 30.)
        };
        let mut material = if h > 1100. {
            "snow"
        } else if h > 750. {
            "stone"
        } else if depth < 8 {
            "sand"
        } else {
            "moss"
        };
        let mut water = None;
        // Intimate garden and headwater lake retain human scale within the enlarged land.
        let garden = ((x - 275.) / 43.).hypot((z + 490.) / 35.);
        if garden < 3.5 {
            h += (900. - h) * (1. - smooth((garden - 0.7) / 2.8));
            material = "moss";
        }
        let lake_angle = (z + 470.).atan2(x - 330.);
        let lake = ((x - 330.) / 34.).hypot((z + 470.) / 27.)
            / (1. + 0.10 * (lake_angle * 3. + 0.4).sin() + 0.05 * (lake_angle * 5.).cos());
        if lake < 2.5 {
            h += (if lake < 1. {
                875. + 8. * lake.powi(2)
            } else {
                905.
            } - h)
                * (1. - smooth((lake - 1.) / 1.5));
            if lake < 1. {
                water = Some(900);
                material = "sand";
            }
        }
        // The same authored centerline composes both banks and liquid, avoiding
        // free-standing water walls where natural terrain lies below river level.
        if (-450. ..=-140.).contains(&z) {
            let cx = headwater_center(z);
            let d = (x - cx).abs();
            let width = 13. + 2.5 * ((z + 450.) / 37.).sin();
            let top = if z < -370. {
                900
            } else if z < -275. {
                790
            } else if z < -170. {
                680
            } else {
                615
            };
            if d < width + 76. {
                let bank = f64::from(top + 15);
                h += (bank - h) * (1. - smooth((d - width - 6.) / 70.));
            }
            if d < width + 6. {
                h = f64::from(top - 12) + (d / (width + 6.)).powi(4) * 27.;
                if d < width {
                    water = Some(top);
                    material = "stone";
                }
            }
            for (fall, upper, lower) in [(-370., 900, 790), (-275., 790, 680), (-170., 680, 615)] {
                if (z - fall).abs() < 2.2 && d < width {
                    h = f64::from(lower - 12);
                    water = Some(upper);
                }
            }
        }
        let valley_angle = (z + 115.).atan2(x - 405.);
        let valley = ((x - 405.) / 70.).hypot((z + 115.) / 60.)
            / (1.
                + 0.14 * (valley_angle * 3. + 0.3).sin()
                + 0.07 * (valley_angle * 5. - 0.6).cos());
        if valley < 2.4 {
            let bed = if valley < 1. {
                580. + 12. * valley.powi(2)
            } else {
                625.
            };
            h += (bed - h) * (1. - smooth((valley - 1.) / 1.4));
            if valley < 1. {
                water = Some(615);
                material = "sand";
            }
        }
        // Gradual river descends into its bay, with broad concave banks.
        if (-65. ..=650.).contains(&z) {
            let t = ((z + 60.) / 525.).clamp(0., 1.);
            let cx = river_center(z);
            let d = (x - cx).abs();
            let width = 10. + 12. * t + 3. * (t * std::f64::consts::PI * 5.).sin();
            let bank_width = 65. + 15. * (t * std::f64::consts::PI * 3.).cos();
            let top = (615. - 215. * t.powf(0.60)).round() as i32;
            if d < width + bank_width && depth > 0 {
                let bank = f64::from(top + 12);
                h += (bank - h) * (1. - smooth((d - width) / bank_width));
            }
            if d < width && depth > 0 {
                h = f64::from(top - 10) + 8. * (d / width).powi(2);
                water = Some(top.max(400));
                material = "sand";
            }
        }
        // Broad independent Crystal footprint: winding ascending terraces plus snowy crown.
        if self.crystal.contains(&p) {
            let dx = x + 201.;
            let dz = z + 524.;
            let radius = dx.hypot(dz);
            let angle = dz.atan2(dx);
            let phase = (angle + std::f64::consts::PI) / (2. * std::f64::consts::PI);
            let rings = (145. - radius).max(0.) / 25.;
            let elevation = 680. + (rings.floor() + phase) * 77.;
            let weight = smooth((151. - radius) / 30.);
            h += (h.max(elevation.min(1170.)) - h) * weight;
            material = if h > 1070. { "snow" } else { "slate" };
        }
        // Broad massif shoulders already cover the library. Entrances alone
        // taper down to exact dry floor supports; no corridor-shaped surface ridges.
        if (x - 310.).abs() < 24. && (z + 348.).abs() < 14. {
            let d = (x - 310.).abs().max((z + 348.).abs() * 1.5);
            h += (721. - h) * (1. - smooth((d - 8.) / 16.));
            water = None;
        }
        // The Shadow tunnel passes seventy units below the library where their
        // horizontal projections cross, retaining two independent ceilings.
        if (x + 105.).abs() < 24. && (-150. ..=10.).contains(&z) {
            let weight = 1. - smooth(((x + 105.).abs() - 9.) / 15.);
            let along = smooth((z + 150.) / 160.);
            let target = 521. + (h - 521.) * along;
            h += (target - h) * weight;
        }
        // Open south entrance to the temple below the World Tree roots.
        if (x + 60.).abs() < 14. && (188. ..=225.).contains(&z) {
            let target = 471. + ((z - 190.) / 35.).clamp(0., 1.) * 40.;
            let weight = 1. - smooth(((x + 60.).abs() - 6.) / 8.);
            h += (target - h) * weight;
        }
        // Offshore volcanic cone, separate from mainland area accounting.
        let vr = ((x + 1180.) / 86.).hypot((z - 450.) / 77.);
        if vr < 1.35 {
            let cone = 400. + 260. * (1. - vr).max(0.).powf(0.7);
            let crater = if vr < 0.24 { 555. + vr * 60. } else { cone };
            h = h.max(crater);
            material = "basalt";
            if vr > 0.94 {
                material = "sand";
            }
        }
        // Small precise supported shrine/encounter aprons blend into surrounding terrain.
        for site in sites::PADS {
            let d = (x - site.x).hypot(z - site.z);
            if water.is_some() && d > site.radius {
                continue;
            }
            if d < site.radius + 32. {
                let weight = 1. - smooth((d - site.radius) / 32.);
                h += (f64::from(site.level + 1) - h) * weight;
                material = site.material;
                water = None;
            }
        }
        // The garden pool drains by a narrow, gently curved rill into the lake.
        if (276. ..=306.).contains(&x) {
            let center = -474. + 4. * ((x - 276.) / 30.) + 1.5 * ((x - 276.) / 9.).sin();
            if (z - center).abs() < 2.2 {
                h = 895.;
                water = Some(900);
                material = "stone";
            }
        }
        let fountain = nearest_hex(276., -474.);
        if p.checked_distance(fountain).is_ok_and(|d| d <= 3) {
            h = 894.;
            water = Some(900);
            material = "stone";
        }
        // Only the explicit offshore island can add land outside the measured
        // mainland footprint. Local aprons never alter coastline area.
        if depth == 0 && vr >= 1. {
            water = None;
            let distance = index(p).map_or(100, |i| self.offshore_distance[i]).min(100);
            h = 400. - 3. * f64::from(distance);
            material = "sand";
        }
        if depth > 0 {
            if let Some((floor, _)) = library_cavity(p) {
                h = h.max(f64::from(floor + 1));
            }
        }
        GrandSurface {
            level: h.floor().clamp(3., 1500.) as i32 - 1,
            material,
            water,
        }
    }
    /// Exact clear interval [floor+1, ceiling), inclusive support under actors.
    pub fn cavity(&self, p: WorldHex) -> Option<(i32, i32)> {
        let [x, z] = world_xz(p);
        if let Some(interval) = library_cavity(p) {
            return Some(interval);
        }
        if (x + 105.).abs() < 9. && (-408. ..=-150.).contains(&z) {
            return Some((520, 585));
        }
        None
    }
    /// Compile one exact column with carved interiors and optional liquid.
    pub fn column(&self, p: WorldHex) -> (ColumnData, Option<LiquidColumn>) {
        let s = self.surface(p);
        let top = s.level + 1;
        let mut runs = vec![
            run(0, 1, "bedrock"),
            run(1, (top - 4).max(1), "stone"),
            run(
                (top - 4).max(1),
                top - 1,
                if s.material == "moss" {
                    "soil"
                } else {
                    "slate"
                },
            ),
            run(top - 1, top, s.material),
        ];
        runs.retain(|r| r.bottom < r.top);
        if let Some((floor, ceiling)) = library_cavity(p) {
            cut(&mut runs, floor + 1, ceiling);
        }
        let [x, z] = world_xz(p);
        if (x + 105.).abs() < 9. && (-408. ..=-150.).contains(&z) {
            cut(&mut runs, 521, 585);
        }
        let liquid = super::fill_sea_column(
            p,
            &mut runs,
            s.water.unwrap_or(SEA_TOP) - 1,
            "water",
            if s.water.is_some() {
                "grand/river"
            } else {
                "grand/ocean"
            },
        )
        .expect("bounded finite sea fill");
        (ColumnData { position: p, runs }, liquid)
    }
    /// Iterate finite candidate chunk coordinates in canonical order.
    pub fn chunk_ids(&self) -> impl Iterator<Item = ChunkId> {
        let lo = (-RADIUS).div_euclid(CHUNK_SIZE);
        let hi = RADIUS.div_euclid(CHUNK_SIZE);
        (lo..=hi).flat_map(move |q| (lo..=hi).map(move |r| ChunkId { q, r }))
    }
    /// Compile one sealed compact chunk and its exact object influences.
    pub fn chunk(&self, id: ChunkId) -> Result<Option<ChunkPackage>, ContractError> {
        let origin = id.origin()?;
        let mut columns = vec![];
        let mut liquids = vec![];
        for q in 0..CHUNK_SIZE {
            for r in 0..CHUNK_SIZE {
                let p = WorldHex::new(origin.q + q, origin.r + r);
                if p.checked_distance(WorldHex::new(0, 0))? > RADIUS as u64 {
                    continue;
                }
                let (c, l) = self.column(p);
                columns.push(c);
                if let Some(l) = l {
                    liquids.push(l);
                }
            }
        }
        if columns.is_empty() {
            return Ok(None);
        }
        let object_influences = self.influences.get(&id).cloned().unwrap_or_default();
        let occupancy = union_object_occupancy(&object_influences)?;
        let objects = self.objects.get(&id).cloned().unwrap_or_default();
        let features = objects
            .iter()
            .map(|o| FeatureSummary {
                id: o.id.clone(),
                region_id: "grand".into(),
                kind: if o.asset.starts_with("plant/") {
                    "tree"
                } else {
                    "structure"
                }
                .into(),
                anchor: o.origin,
                asset: Some(o.asset.clone()),
            })
            .collect();
        let anchors = self
            .anchors
            .iter()
            .filter(|a| a.position.column.chunk() == id)
            .cloned()
            .collect();
        let mut chunk = ChunkPackage {
            schema_version: SCHEMA_VERSION,
            world_id: self.source.id.clone(),
            coordinate: id,
            source_fingerprint: self.source_fingerprint,
            columns,
            features,
            semantics: ChunkSemantics {
                objects,
                object_influences,
                occupancy,
                liquids,
                anchors,
                ..Default::default()
            },
            fingerprint: 0,
        };
        chunk.seal()?;
        Ok(Some(chunk))
    }
    /// Create the manifest; the writer adds descriptors and seals it.
    pub fn manifest(&self) -> WorldManifest {
        WorldManifest {
            schema_version: SCHEMA_VERSION,
            world_id: self.source.id.clone(),
            compiler_version: "hex-grand/1".into(),
            source_fingerprint: self.source_fingerprint,
            materials: self.materials.clone(),
            regions: vec![RegionDescriptor {
                id: "grand".into(),
                origin: WorldHex::new(0, 0),
                radius: RADIUS as u32,
                source_fingerprint: self.source_fingerprint,
            }],
            chunks: vec![],
            boundaries: vec![],
            summary: vec![],
            features: vec![],
            fingerprint: 0,
        }
    }
    /// Sample actual quantized relief and named observation locations.
    pub fn overview(&self) -> NorthernOverview {
        let origin_xz = [-1564., -1356.];
        let (width, height) = (783, 679);
        let spacing = 4.;
        let mut bed_heights = Vec::with_capacity(width * height);
        let mut surface_materials = Vec::with_capacity(width * height);
        for row in 0..height {
            for col in 0..width {
                let s = self.surface(nearest_hex(
                    origin_xz[0] + col as f64 * spacing,
                    origin_xz[1] + row as f64 * spacing,
                ));
                bed_heights.push(((s.level + 1) as f64 * LEVEL_HEIGHT) as f32);
                surface_materials.push(
                    self.materials
                        .iter()
                        .position(|m| {
                            m.id == if s.water.is_some() {
                                "water"
                            } else {
                                s.material
                            }
                        })
                        .unwrap() as u16,
                );
            }
        }
        let anchors: BTreeMap<_, _> = self
            .anchors
            .iter()
            .map(|a| {
                let [x, z] = world_xz(a.position.column);
                (
                    a.id.rsplit('/').next().unwrap().into(),
                    [
                        x as f32,
                        (f64::from(a.position.level + 1) * LEVEL_HEIGHT) as f32,
                        z as f32,
                    ],
                )
            })
            .collect();
        NorthernOverview {
            version: 1,
            source_fingerprint: self.source_fingerprint,
            package_fingerprint: 0,
            world_id: self.source.id.clone(),
            hex_radius: 1.,
            level_height: 0.35,
            vertical_offset: 0.35,
            radius: RADIUS as u32,
            level_bounds: [0, MAX_LEVEL],
            sea_level: 140.,
            origin_xz: origin_xz.map(|x| x as f32),
            spacing: spacing as f32,
            width: width as u32,
            height: height as u32,
            bed_heights,
            surface_materials,
            materials: self.materials.clone(),
            player_spawn: anchors["party_start"],
            anchors,
            islands: vec![IslandSpec {
                id: "fire-volcano".into(),
                cluster: 0,
                center: [-1180., 450.],
                radii: [86., 77.],
                angle: 0.,
                peak: 91.,
                crater: true,
            }],
            tree_count: self.tree_count,
            building_count: self
                .objects
                .values()
                .flatten()
                .filter(|object| !object.asset.starts_with("plant/"))
                .count(),
        }
    }
}
fn palette() -> Vec<MaterialSpec> {
    let mut v: Vec<_> = [
        ("bedrock", [54, 64, 76, 255]),
        ("stone", [115, 124, 134, 255]),
        // An opaque carved flame marker, using the existing fitted-masonry policy.
        ("worked_stone", [248, 132, 48, 255]),
        ("reinforced_stone", [115, 124, 134, 255]),
        ("basalt", [62, 53, 57, 255]),
        ("slate", [83, 99, 123, 255]),
        ("soil", [87, 75, 58, 255]),
        ("dirt", [87, 75, 58, 255]),
        ("moss", [66, 110, 70, 255]),
        ("snow", [219, 235, 245, 255]),
        ("sand", [184, 169, 129, 255]),
        ("timber", [108, 73, 45, 255]),
        ("foliage", [34, 94, 61, 255]),
        ("crystal", [83, 157, 204, 255]),
        ("water", [37, 104, 150, 180]),
    ]
    .into_iter()
    .map(|(id, color)| MaterialSpec {
        id: id.into(),
        solid: id != "water",
        diggable: id != "water" && id != "bedrock",
        color,
    })
    .collect();
    v.sort_by(|a, b| a.id.cmp(&b.id));
    v
}

fn library_cavity(p: WorldHex) -> Option<(i32, i32)> {
    let [x, z] = world_xz(p);
    // Waterfall gallery, lower grand hall, perpendicular ascending grand stair.
    if (x + 40.).abs() < 365. && (z + 348.).abs() < 8. {
        return Some((720, 768));
    }
    // Stair centerline follows exact neighbouring hexes. One level per step
    // keeps its 0.35-unit risers within the ordinary 0.4-unit controller step.
    if (-460. ..=-275.).contains(&x) && (-578. ..=-355.).contains(&z) {
        const PATH: [(i64, i64); 12] = [
            (-119, -247),
            (-111, -263),
            (-42, -263),
            (-28, -290),
            (-106, -290),
            (-93, -317),
            (-26, -317),
            (-13, -343),
            (-68, -343),
            (-55, -370),
            (-46, -370),
            (-42, -377),
        ];
        let mut prefix = 0_i32;
        let mut best: Option<(f64, i32)> = None;
        for pair in PATH.windows(2) {
            let [(aq, ar), (bq, br)] = pair else {
                continue;
            };
            let dq = (bq - aq) as f64;
            let dr = (br - ar) as f64;
            let pq = (p.q - aq) as f64;
            let pr = (p.r - ar) as f64;
            let metric = dq * dq + dq * dr + dr * dr;
            let t = ((pq * dq + (pq * dr + pr * dq) * 0.5 + pr * dr) / metric).clamp(0., 1.);
            let eq = pq - t * dq;
            let er = pr - t * dr;
            let d = eq * eq + eq * er + er * er;
            let steps = (bq - aq)
                .abs()
                .max((br - ar).abs())
                .max((bq + br - aq - ar).abs()) as i32;
            let floor = 720 + prefix + (t * f64::from(steps)).round() as i32;
            if d <= 25. && best.is_none_or(|(old, _)| d < old) {
                best = Some((d, floor));
            }
            prefix += steps;
        }
        if let Some((_, floor)) = best {
            return Some((floor, floor + 64));
        }
    }
    if (-440. ..=-310.).contains(&x) && (-392. ..=-305.).contains(&z) {
        return Some((720, 825));
    }
    if (-450. ..=-350.).contains(&x) && (-475. ..=-435.).contains(&z) {
        return Some((900, 1000));
    }
    if (-438. ..=-365.).contains(&x) && (-555. ..=-515.).contains(&z) {
        return Some((1100, 1190));
    }
    // Temple below the roots, with a southern passage under the trunk.
    if ((x + 60.) / 26.).hypot((z - 125.) / 22.) < 1.
        || (x + 60.).abs() < 7. && (130. ..=196.).contains(&z)
    {
        return Some((470, 503));
    }
    None
}

fn headwater_center(z: f64) -> f64 {
    let t = ((z + 450.) / 310.).clamp(0., 1.);
    330. + (z + 450.) * 0.23 + 9. * (t * std::f64::consts::TAU).sin()
}
fn river_center(z: f64) -> f64 {
    let t = ((z + 60.) / 525.).clamp(0., 1.);
    400. - 701. * t
        + 28. * (t * std::f64::consts::TAU).sin()
        + 13. * (t * std::f64::consts::TAU * 2.).sin()
}
