//! Grand's full sevenfold mainland, compiled a single compact chunk at a time.
#![expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "Finite authored geometry is bounded by radius1052 and2100levels; rounding is the voxelization contract."
)]
mod biomes;
mod dressing;
mod flow;
mod forest_overview;
mod geography;
mod ground_cover;
mod oracle;
mod r02;
mod r02_overview;
mod r02_sites;
pub use geography::{GrandGeography, GrandGeographyDocument};
use geography::{LandmarkFrame, SupportLayer};
mod library_finish;
pub use biomes::GrandBiomeMap;
pub use flow::RIVER_PHASE_DIRECTION;
mod sites;
mod terrain;
#[cfg(test)]
mod terrain_tests;
#[cfg(test)]
mod tests;
use super::northern::{IslandSpec, NorthernOverview, nearest_hex, world_xz};
use hex_world_contracts::*;
use serde::{Deserialize, Serialize};
pub use sites::GrandSites;
use std::collections::{BTreeMap, VecDeque};
/// Finite ocean envelope, independent of the measured mainland area.
pub const RADIUS: i64 = 1052;
/// World units per voxel level.
pub const LEVEL_HEIGHT: f64 = 0.35;
/// Exclusive ocean surface level.
pub const SEA_TOP: i32 = 400;
/// Inclusive vertical storage bound.
pub const MAX_LEVEL: i32 = 2100;
const GRID: usize = 2105;
const OFFSET: i64 = 1052;
const DIRS: [(i64, i64); 6] = [(1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1)];
/// Small measured authoring input. Rows are (r, first q, last q), inclusive.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrandSpec {
    /// Source schema version.
    pub version: u32,
    /// Whether globally reserved landmarks and vegetation are included.
    pub full_dressing: bool,
    /// Optional sibling JSON geography authoring document, resolved by the tool.
    #[serde(default)]
    pub geography: Option<String>,
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
    pub(super) geography: GrandGeography,
    /// Stable material policies and presentation colors.
    pub materials: Vec<MaterialSpec>,
    /// Exact enlarged coast-enclosed area in hex columns.
    pub mainland_columns: usize,
    /// Exact independently enlarged Crystal footprint in hex columns.
    pub crystal_columns: usize,
    /// Number of globally reserved tree objects.
    pub tree_count: usize,
    forest: Option<super::northern::forest::ForestOverview>,
    ground_cover: Option<super::northern::ground_cover::GroundCover>,
    inland: Option<super::northern::InlandWaterOverview>,
    layered: r02::Layered,
    r02_flow: BTreeMap<WorldHex, VoxelPosition>,
    coast: Vec<u16>,
    offshore_distance: Vec<u16>,
    crystal: std::collections::BTreeSet<WorldHex>,
    cave_cover: BTreeMap<WorldHex, i32>,
    graded_shoulders: BTreeMap<WorldHex, i32>,
    anchors: Vec<WorldAnchor>,
    objects: BTreeMap<ChunkId, Vec<ObjectInstance>>,
    influences: BTreeMap<ChunkId, Vec<ObjectInfluence>>,
}
fn index(p: WorldHex) -> Option<usize> {
    let q = usize::try_from(p.q + OFFSET).ok()?;
    let r = usize::try_from(p.r + OFFSET).ok()?;
    (q < GRID && r < GRID).then_some(r * GRID + q)
}
#[expect(
    clippy::expect_used,
    reason = "Both private distance grids are allocated at GRID squared before queries; index bounds use the same constant. An internal shape mismatch must fail."
)]
fn grid_value(grid: &[u16], p: WorldHex, outside: u16) -> u16 {
    index(p).map_or(outside, |i| {
        *grid
            .get(i)
            .expect("distance grid matches its bounded index")
    })
}
fn grid_cell(grid: &mut [u16], p: WorldHex) -> Result<&mut u16, ContractError> {
    index(p).and_then(|i| grid.get_mut(i)).ok_or_else(|| {
        ContractError::new("grand", "distance-grid coordinate outside measured bounds")
    })
}
fn smooth(t: f64) -> f64 {
    let t = t.clamp(0., 1.);
    t * t * (3. - 2. * t)
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
        if source.geography.is_some() {
            return Err(ContractError::new(
                "grand.geography",
                "resolve the named geography document before compilation",
            ));
        }
        Self::build(source, GrandGeography::legacy(), None)
    }
    /// Compile an explicitly resolved sibling geography document. The exact bytes
    /// and the validated typed value both participate in immutable source identity.
    pub fn with_geography(
        source: GrandSpec,
        document: GrandGeographyDocument,
        bytes: &[u8],
    ) -> Result<Self, ContractError> {
        let name = source
            .geography
            .as_deref()
            .ok_or_else(|| ContractError::new("grand.geography", "missing geography dependency"))?;
        if !name.ends_with(".json")
            || name.contains(['/', '\\'])
            || name == ".json"
            || name.contains("..")
        {
            return Err(ContractError::new(
                "grand.geography",
                "geography must be a single sibling JSON filename",
            ));
        }
        let geography = GrandGeography::new(document)?;
        Self::build(source, geography, Some(xxhash_rust::xxh3::xxh3_64(bytes)))
    }
    fn build(
        mut source: GrandSpec,
        geography: GrandGeography,
        geography_bytes: Option<u64>,
    ) -> Result<Self, ContractError> {
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
        if let Some(document) = &geography.document {
            for r in -OFFSET..=OFFSET {
                for q in -OFFSET..=OFFSET {
                    let p = WorldHex::new(q, r);
                    let point = geography.model_xz(p);
                    if !(-1200. ..=1200.).contains(&point[0])
                        || !(-1000. ..=1500.).contains(&point[1])
                    {
                        continue;
                    }
                    if oracle::coast_reference(document, point) > 0. {
                        *grid_cell(&mut coast, p)? = u16::MAX;
                    }
                }
            }
            // Only the connected mainland counts toward sevenfold area. Tiny
            // separate positive coastal islets retain their terrain separately.
            let seed = geography.world_hex([0., 100.]);
            let mut todo = VecDeque::from([seed]);
            *grid_cell(&mut coast, seed)? = 2;
            while let Some(p) = todo.pop_front() {
                count += 1;
                for (a, b) in DIRS {
                    let n = WorldHex::new(p.q + a, p.r + b);
                    if grid_value(&coast, n, 0) == u16::MAX {
                        *grid_cell(&mut coast, n)? = 2;
                        todo.push_back(n);
                    }
                }
            }
            for value in &mut coast {
                *value = if *value == 2 { u16::MAX } else { 0 };
            }
            source.mainland_rows.clear();
            for r in -OFFSET..=OFFSET {
                let mut begin = None;
                for q in -OFFSET..=OFFSET + 1 {
                    let inside = grid_value(&coast, WorldHex::new(q, r), 0) > 0;
                    match (begin, inside) {
                        (None, true) => begin = Some(q),
                        (Some(a), false) => {
                            source.mainland_rows.push((r, a, q - 1));
                            begin = None;
                        }
                        _ => {}
                    }
                }
            }
        } else {
            for &(r, a, b) in &source.mainland_rows {
                if a > b {
                    return Err(ContractError::new("grand", "inverted footprint row"));
                }
                for q in a..=b {
                    let cell = grid_cell(&mut coast, WorldHex::new(q, r))?;
                    if *cell != 0 {
                        return Err(ContractError::new("grand", "overlapping footprint rows"));
                    }
                    *cell = u16::MAX;
                    count += 1;
                }
            }
        }
        if count != source.canonical_mainland_columns * 7 {
            return Err(ContractError::new(
                "grand",
                format!(
                    "mainland must be exactly seven times canonical area: actual {count}, expected {}",
                    source.canonical_mainland_columns * 7
                ),
            ));
        }
        let mut queue = VecDeque::new();
        for r in -OFFSET..=OFFSET {
            for q in -OFFSET..=OFFSET {
                let p = WorldHex::new(q, r);
                if grid_value(&coast, p, 0) != 0
                    && DIRS
                        .iter()
                        .any(|(a, b)| grid_value(&coast, WorldHex::new(q + a, r + b), 0) == 0)
                {
                    *grid_cell(&mut coast, p)? = 1;
                    queue.push_back(p);
                }
            }
        }
        while let Some(p) = queue.pop_front() {
            let depth = grid_value(&coast, p, 0);
            for (a, b) in DIRS {
                let n = WorldHex::new(p.q + a, p.r + b);
                if index(n).is_some() {
                    let cell = grid_cell(&mut coast, n)?;
                    if *cell == u16::MAX {
                        *cell = depth + 1;
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
                let depth = grid_value(&coast, p, 0);
                if depth > 0 {
                    *grid_cell(&mut offshore_distance, p)? = 0;
                    if depth == 1 {
                        offshore.push_back(p);
                    }
                }
            }
        }
        while let Some(p) = offshore.pop_front() {
            let d = grid_value(&offshore_distance, p, 100);
            if d >= 100 {
                continue;
            }
            for (a, b) in DIRS {
                let n = WorldHex::new(p.q + a, p.r + b);
                if index(n).is_some() {
                    let cell = grid_cell(&mut offshore_distance, n)?;
                    if *cell == u16::MAX {
                        *cell = d + 1;
                        offshore.push_back(n);
                    }
                }
            }
        }
        let root = geography.document.as_ref().map_or_else(
            || nearest_hex(-201., -524.),
            |d| geography.world_hex(d.ascent.center),
        );
        let crystal: std::collections::BTreeSet<_> = if let Some(d) = &geography.document {
            let radius = (geography.length(d.ascent.outer_apothem) * 1.16 / 1.5).ceil() as i64 + 2;
            let mut footprint = std::collections::BTreeSet::new();
            for q in -radius..=radius {
                for r in -radius..=radius {
                    let p = WorldHex::new(root.q + q, root.r + r);
                    if oracle::crystal_distance(d, geography.model_xz(p)) < d.ascent.outer_apothem {
                        footprint.insert(p);
                    }
                }
            }
            footprint
        } else {
            let mut candidates = Vec::new();
            for q in -95_i64..=95 {
                for r in -95_i64..=95 {
                    if q.abs().max(r.abs()).max((q + r).abs()) <= 95 {
                        candidates.push((q * q + q * r + r * r, q, r));
                    }
                }
            }
            candidates.sort();
            candidates
                .into_iter()
                .take(source.canonical_crystal_columns * 7)
                .map(|(_, q, r)| WorldHex::new(root.q + q, root.r + r))
                .collect()
        };
        let crystal_columns = crystal.len();
        if geography
            .document
            .as_ref()
            .is_some_and(|d| crystal_columns != d.ascent.expected_columns)
        {
            return Err(ContractError::new(
                "grand.crystal",
                "actual outer feature polygon differs from authored measured count",
            ));
        }
        let source_fingerprint =
            hash_serializable(&(&source, geography_bytes, &geography.document))?;
        let layered = r02::Layered::compile(&geography, &coast)?;
        let revision02 = geography.document.is_some();
        let mut result = Self {
            source,
            source_fingerprint,
            geography,
            materials: palette(),
            mainland_columns: count,
            crystal_columns,
            tree_count: 0,
            forest: None,
            ground_cover: None,
            inland: None,
            layered,
            r02_flow: BTreeMap::new(),
            coast,
            offshore_distance,
            crystal,
            cave_cover: if revision02 {
                BTreeMap::new()
            } else {
                terrain::compile_cave_cover()
            },
            graded_shoulders: BTreeMap::new(),
            anchors: vec![],
            objects: BTreeMap::new(),
            influences: BTreeMap::new(),
        };
        result.r02_flow = result.compile_r02_flow()?;
        if !revision02 {
            result.graded_shoulders = terrain::compile_grades(&result)?;
        }
        result.anchors = if revision02 {
            result.r02_anchors()?
        } else {
            result.make_anchors()
        };
        let objects = if result.source.full_dressing {
            dressing::compose(&result)?
        } else {
            vec![]
        };
        if result.source.full_dressing {
            result.forest = Some(forest_overview::compile(&objects)?);
            result.ground_cover = Some(ground_cover::compile(&result, &objects)?);
        }
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
        if revision02 {
            result.inland = Some(result.r02_inland()?);
        }
        Ok(result)
    }
    /// Whether the column is inside the authored mainland footprint.
    pub fn mainland(&self, p: WorldHex) -> bool {
        grid_value(&self.coast, p, 0) > 0
    }
    /// Quantized solid surface before exact caves and above-ground structures.
    pub fn surface(&self, p: WorldHex) -> GrandSurface {
        if self.geography.document.is_some() {
            self.r02_surface(p)
        } else {
            terrain::surface(self, p)
        }
    }

    /// Exact clear interval [floor+1, ceiling), inclusive support under actors.
    pub fn cavity(&self, p: WorldHex) -> Option<(i32, i32)> {
        if let Some(interval) = library_cavity(p) {
            return Some(interval);
        }
        shadow_cavity(p)
    }
    /// Compile one exact column with carved interiors and optional liquid.
    pub fn column(&self, p: WorldHex) -> (ColumnData, Option<LiquidColumn>) {
        if self.geography.document.is_some() {
            return self.r02_column(p);
        }
        let (mut column, liquid) = self.column_without_library_finish(p);
        library_finish::floor(p, &mut column.runs);
        (column, liquid)
    }

    #[expect(
        clippy::expect_used,
        reason = "Authored finite columns are ordered, nonnegative compact intervals within MAX_LEVEL; sea filling cannot overflow those validated geometry bounds."
    )]
    fn column_without_library_finish(&self, p: WorldHex) -> (ColumnData, Option<LiquidColumn>) {
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
            let [x, z] = world_xz(p);
            if z > 0. {
                // Warm floor and a narrow contrasting center guide remain actual
                // supporting terrain, with no raised decorative obstacle.
                cut(&mut runs, floor - 1, floor + 1);
                runs.push(run(
                    floor - 1,
                    floor + 1,
                    if (x + 60.).abs() < 1.8 {
                        "slate"
                    } else {
                        "sand"
                    },
                ));
                runs.sort_by_key(|run| run.bottom);
            }
        }
        if let Some((floor, ceiling)) = shadow_cavity(p) {
            cut(&mut runs, floor + 1, ceiling);
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
                if let Some(mut l) = l {
                    self.direct_river(&mut l);
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
            compiler_version: "hex-grand/r02-1".into(),
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
    #[expect(
        clippy::expect_used,
        clippy::cast_sign_loss,
        reason = "Positive fixed overview extents produce nonnegative grid dimensions. The fixed compiler palette contains every surface material and make_anchors always publishes party_start; missing either is an authoring invariant failure."
    )]
    pub fn overview(&self) -> NorthernOverview {
        let spacing = 8.;
        let extent_x = (RADIUS as f64 + 0.5) * 3_f64.sqrt();
        let extent_z = RADIUS as f64 * 1.5 + 1.;
        let origin_xz = [-extent_x, -extent_z];
        let width = (extent_x * 2. / spacing).ceil() as usize + 1;
        let height = (extent_z * 2. / spacing).ceil() as usize + 1;
        let mut bed_heights = Vec::with_capacity(width * height);
        let mut surface_materials = Vec::with_capacity(width * height);
        for row in 0..height {
            for col in 0..width {
                let p = nearest_hex(
                    origin_xz[0] + col as f64 * spacing,
                    origin_xz[1] + row as f64 * spacing,
                );
                let (column, _) = self.column(p);
                let solid = column
                    .runs
                    .iter()
                    .filter(|r| r.material != "water")
                    .max_by_key(|r| r.top)
                    .expect("finite terrain has solid bedrock");
                bed_heights.push((f64::from(solid.top) * LEVEL_HEIGHT) as f32);
                surface_materials.push(
                    self.materials
                        .iter()
                        .position(|m| m.id == solid.material)
                        .expect("authored surface material is in the compiler palette")
                        as u16,
                );
            }
        }
        let anchors: BTreeMap<_, _> = self
            .anchors
            .iter()
            .map(|a| {
                let [x, z] = world_xz(a.position.column);
                (
                    a.id.rsplit_once('/')
                        .map_or(a.id.as_str(), |(_, name)| name)
                        .into(),
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
            player_spawn: *anchors
                .get("party_start")
                .expect("authored starting anchor"),
            anchors,
            islands: vec![IslandSpec {
                id: "fire-volcano".into(),
                cluster: 0,
                center: self
                    .geography
                    .document
                    .as_ref()
                    .map_or([-1180., 450.], |d| {
                        self.geography.world_xz(d.volcano.center)
                    }),
                radii: self.geography.document.as_ref().map_or([86., 77.], |d| {
                    d.volcano.radii.map(|r| self.geography.length(r))
                }),
                angle: 0.,
                peak: 91.,
                crater: true,
            }],
            tree_count: self.tree_count,
            forest: self.forest.clone(),
            ground_cover: self.ground_cover.clone(),
            inland_water: self.inland.clone(),
            review_cameras: self.r02_cameras(),
            building_count: self
                .objects
                .values()
                .flatten()
                .filter(|object| !object.asset.starts_with("plant/"))
                .count(),
        }
    }
}

/// The Shadow's original uniform bore ends at r=-272. A straight northern
/// stair then gains 131 levels over 140 adjacent rows and opens onto Crystal.
/// Every riser is at most one level (0.35u), within the ordinary walk controller.
fn shadow_cavity(p: WorldHex) -> Option<(i32, i32)> {
    let [x, z] = world_xz(p);
    if (x + 105.).abs() >= 9. || !(-618. ..=-150.).contains(&z) {
        return None;
    }
    let steps = (-272 - p.r).max(0);
    let floor = terrain::SHADOW_FLOOR + (steps * 131 / 140) as i32;
    Some((floor, floor + 65))
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
        // Library paving and existing piers; the shared policy aliases this
        // presentation material to ordinary stone, including durability.
        ("limestone", [194, 179, 148, 255]),
        ("soil", [87, 75, 58, 255]),
        ("dirt", [87, 75, 58, 255]),
        ("moss", [66, 110, 70, 255]),
        ("snow", [219, 235, 245, 255]),
        ("sand", [184, 169, 129, 255]),
        ("timber", [108, 73, 45, 255]),
        ("foliage", [20, 110, 20, 255]),
        ("foliage_dark", [8, 56, 15, 255]),
        ("foliage_light", [82, 173, 20, 255]),
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
        return Some((terrain::LIBRARY_FLOOR, terrain::LIBRARY_FLOOR + 36));
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
            let original_rise = prefix + (t * f64::from(steps)).round() as i32;
            let floor = terrain::LIBRARY_FLOOR + original_rise * 240 / 408;
            if d <= 25. && best.is_none_or(|(old, _)| d < old) {
                best = Some((d, floor));
            }
            prefix += steps;
        }
        if let Some((_, floor)) = best {
            return Some((floor, floor + 48));
        }
    }
    if (-440. ..=-310.).contains(&x) && (-392. ..=-305.).contains(&z) {
        return Some((terrain::LIBRARY_FLOOR, terrain::LIBRARY_FLOOR + 65));
    }
    if (-450. ..=-350.).contains(&x) && (-475. ..=-435.).contains(&z) {
        return Some((706, 768));
    }
    if (-438. ..=-365.).contains(&x) && (-555. ..=-515.).contains(&z) {
        return Some((824, 878));
    }
    // Temple below the roots, with a southern passage under the trunk.
    if ((x + 60.) / 26.).hypot((z - 125.) / 22.) < 1. {
        return Some((470, 503));
    }
    if (x + 60.).abs() < 7. && (130. ..=225.).contains(&z) {
        let floor = 470 + (p.r - 120).clamp(0, 30) as i32;
        return Some((floor, floor + 33));
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

/// Authored flowing corridors; receiving lake and ocean are separate terminals.
pub(super) fn river_channel(p: WorldHex) -> bool {
    let [x, z] = world_xz(p);
    let headwater = (-450. ..=-140.).contains(&z)
        && (x - headwater_center(z)).abs() < 13. + 2.5 * ((z + 450.) / 37.).sin();
    let t = ((z + 60.) / 525.).clamp(0., 1.);
    let lower = (-65. ..=465.).contains(&z)
        && (x - river_center(z)).abs() < 10. + 12. * t + 3. * (t * std::f64::consts::PI * 5.).sin();
    headwater || lower
}

/// A channel may discharge only to its actual lower lake or mean-sea terminus.
pub(super) fn river_receiver(p: WorldHex, top: i32) -> bool {
    if top == SEA_TOP {
        return true;
    }
    let [x, z] = world_xz(p);
    let angle = (z + 115.).atan2(x - 405.);
    let lake = ((x - 405.) / 70.).hypot((z + 115.) / 60.)
        / (1. + 0.14 * (angle * 3. + 0.3).sin() + 0.07 * (angle * 5. - 0.6).cos());
    top == terrain::VALLEY_TOP && lake < 1.
}
