//! Full-scale, chunk-native Northern Archipelago authoring. No voxel expansion.
#![expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "The fixed radius700,1400-level package and bounded finite authored dimensions fit integer/f32 publications; quantization deliberately rounds voxel levels."
)]
mod objects;
#[cfg(test)]
mod tests;

use hex_world_contracts::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Fixed physical height per logical level.
pub const LEVEL_HEIGHT: f64 = 0.35;
/// Common exclusive sea surface level (140 world units).
pub const SEA_TOP: i32 = 400;
/// Exact axial region radius, approximately 2425 by 2100 world units.
pub const RADIUS: i64 = 700;
const REGION: &str = "northern";
const SPAWN_XZ: [f64; 2] = [-646.0, -100.0];
const BAY_XZ: [f64; 2] = [-563.0, -75.0];

#[derive(Clone, Copy)]
struct SettlementSite {
    name: &'static str,
    xz: [f64; 2],
    half_width: i64,
    half_length: i64,
    elevation: f64,
}
const BUILDING_SITES: [SettlementSite; 4] = [
    SettlementSite {
        name: "longhouse",
        xz: [-18.0, 570.0],
        half_width: 4,
        half_length: 10,
        elevation: 21.0,
    },
    SettlementSite {
        name: "cottage-west",
        xz: [-42.0, 590.0],
        half_width: 3,
        half_length: 4,
        elevation: 19.0,
    },
    SettlementSite {
        name: "cottage-east",
        xz: [24.0, 575.0],
        half_width: 3,
        half_length: 4,
        elevation: 23.0,
    },
    SettlementSite {
        name: "storehouse",
        xz: [15.0, 552.0],
        half_width: 2,
        half_length: 4,
        elevation: 25.0,
    },
];
const FIELD_SITE: SettlementSite = SettlementSite {
    name: "cultivation",
    xz: [0.0, 612.0],
    half_width: 5,
    half_length: 4,
    elevation: 16.0,
};

fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
fn bay_radius(x: f64, z: f64) -> f64 {
    let a = (x + 563.0) / 64.0;
    let b = (z + 72.0) / 60.0;
    a.hypot(b) * (1.0 + 0.08 * (b.atan2(a) * 3.0 + 1.0).sin())
}
fn bay_elevation(mut terrain: f64, x: f64, z: f64) -> f64 {
    let radius = bay_radius(x, z);
    if radius < 1.9 {
        let bed = 12.0 * (radius - 1.0);
        let weight = 1.0 - smoothstep((radius - 0.8) / 1.1);
        terrain = terrain.min(terrain + (bed - terrain) * weight);
    }
    // The broad outlet widens toward the sea, with unequal, sloping rocky arms.
    // Both its upstream end and its banks blend into the composed mountain.
    if (-125.0..65.0).contains(&z) {
        let center_x = -563.0 + 6.0 * ((z + 72.0) * 0.025).sin();
        let width = 31.0 + (z + 72.0).max(0.0) * 0.08;
        let radius = (x - center_x).abs() / width;
        let bed = -6.0 + 12.0 * radius.powi(2);
        let weight = (1.0 - smoothstep((radius - 0.65) / 0.9)) * smoothstep((z + 125.0) / 55.0);
        terrain = terrain.min(terrain + (bed - terrain) * weight);
    }
    terrain
}
fn settlement_elevation(terrain: f64, p: WorldHex, x: f64, z: f64) -> f64 {
    let radius = ((x + 6.0) / 100.0).hypot((z - 575.0) / 90.0);
    if radius >= 1.5 {
        return terrain;
    }
    let floor = 22.0 + 1.8 * ((x + 6.0) * 0.04).sin() + 1.2 * ((z - 575.0) * 0.06).sin();
    let weight = 1.0 - smoothstep((radius - 0.2) / 1.1);
    let valley = terrain.min(terrain + (floor - terrain) * weight);
    // Small exact foundations share the blueprint footprint. The intervening
    // ground stays rolling, and the valley fades into the existing island.
    let Some((site, distance)) = BUILDING_SITES
        .iter()
        .chain(std::iter::once(&FIELD_SITE))
        .map(|site| {
            let root = nearest_hex(site.xz[0], site.xz[1]);
            let dq = ((p.q - root.q).abs() - site.half_width).max(0) as f64;
            let dr = ((p.r - root.r).abs() - site.half_length).max(0) as f64;
            (site, dq.max(dr) * 1.5)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
    else {
        return valley;
    };
    let pad_weight = 1.0 - smoothstep(distance / 9.0);
    valley + (site.elevation - valley) * pad_weight
}

/// One source-authored island, independent of publication and storage chunks.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IslandSpec {
    /// Stable name.
    pub id: String,
    /// Cluster number, zero through two.
    pub cluster: u8,
    /// Absolute world X/Z center.
    pub center: [f64; 2],
    /// Elliptical shore radii before low-frequency irregularity.
    pub radii: [f64; 2],
    /// Rotation in radians.
    pub angle: f64,
    /// Approximate elevation above the mean sea.
    pub peak: f64,
    /// Dormant crater profile, only for the main island.
    pub crater: bool,
}
/// Runtime-loaded, compact deterministic authoring source.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NorthernSpec {
    /// Schema version.
    pub version: u32,
    /// Stable world ID.
    pub id: String,
    /// Stable variation seed.
    pub seed: u64,
    /// Eleven explicit island footprints.
    pub islands: Vec<IslandSpec>,
    /// False creates the full-size checkpoint with only the bay/settlement dressed.
    pub full_dressing: bool,
}
/// Coarse map and distant-terrain samples published once, outside residency.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NorthernOverview {
    /// Companion schema.
    pub version: u32,
    /// Exact source identity.
    pub source_fingerprint: u64,
    /// Exact package identity, assigned by the publishing tool.
    pub package_fingerprint: u64,
    /// World identity.
    pub world_id: String,
    /// Physical lattice radius.
    pub hex_radius: f32,
    /// Per-level world height.
    pub level_height: f32,
    /// Physical offset added to topmost occupied voxel levels.
    pub vertical_offset: f32,
    /// Inclusive finite axial radius.
    pub radius: u32,
    /// Inclusive logical lower and upper bounds.
    pub level_bounds: [i32; 2],
    /// Mean ocean height in world Y.
    pub sea_level: f32,
    /// World X/Z origin of the row-major grid; X changes fastest.
    pub origin_xz: [f32; 2],
    /// Uniform sample step in world units.
    pub spacing: f32,
    /// Samples in each row.
    pub width: u32,
    /// Sample rows.
    pub height: u32,
    /// Actual quantized solid upper-face height, including seabed (not water).
    pub bed_heights: Vec<f32>,
    /// Palette entry per sample; indexes `materials`.
    pub surface_materials: Vec<u16>,
    /// Corresponding authoring palette.
    pub materials: Vec<MaterialSpec>,
    /// Dry player feet at the supported bay observation point.
    pub player_spawn: [f32; 3],
    /// Additional exact review/structure anchors in world coordinates.
    pub anchors: BTreeMap<String, [f32; 3]>,
    /// All eleven authored island facts.
    pub islands: Vec<IslandSpec>,
    /// Exact total tree records.
    pub tree_count: usize,
    /// Exact building count, excluding field.
    pub building_count: usize,
}
/// A composed height and surface identity before sea fill.
#[derive(Clone, Copy, Debug)]
pub struct NorthernSurface {
    /// Topmost solid voxel level.
    pub level: i32,
    /// Registered material ID.
    pub material: &'static str,
}
/// Small immutable generator plus globally reserved object footprints.
pub struct NorthernCompiler {
    /// Accepted authoring source.
    pub source: NorthernSpec,
    /// Source serialization identity.
    pub source_fingerprint: u64,
    /// Explicit material registry.
    pub materials: Vec<MaterialSpec>,
    objects: BTreeMap<ChunkId, Vec<ObjectInstance>>,
    influences: BTreeMap<ChunkId, Vec<ObjectInfluence>>,
    anchors: Vec<WorldAnchor>,
    /// Number of deterministic authored tree blueprints.
    pub tree_count: usize,
}

/// Horizontal lattice conversion matching the shared unit-circumradius contract.
pub fn world_xz(p: WorldHex) -> [f64; 2] {
    [
        3_f64.sqrt() * (p.q as f64 + p.r as f64 * 0.5),
        p.r as f64 * 1.5,
    ]
}
/// Nearest exact hex column to an authored absolute world location.
pub fn nearest_hex(x: f64, z: f64) -> WorldHex {
    let q = x / 3_f64.sqrt() - z / 3.0;
    let r = z / 1.5;
    let mut rq = q.round();
    let mut rr = r.round();
    let rs = (-q - r).round();
    let dq = (rq - q).abs();
    let dr = (rr - r).abs();
    let ds = (rs + q + r).abs();
    if dq > dr && dq > ds {
        rq = -rr - rs;
    } else if dr > ds {
        rr = -rq - rs;
    }
    WorldHex::new(rq as i64, rr as i64)
}
fn noise(x: f64, z: f64) -> f64 {
    (x * 0.037 + z * 0.013).sin() * 0.5
        + (x * 0.019 - z * 0.043).sin() * 0.3
        + (x * 0.091 + z * 0.076).sin() * 0.2
}
impl IslandSpec {
    /// Irregular normalized radius and azimuth in the island's own frame.
    pub fn normalized(&self, x: f64, z: f64) -> (f64, f64) {
        let [cx, cz] = self.center;
        let [rx, rz] = self.radii;
        let dx = x - cx;
        let dz = z - cz;
        let a = (dx * self.angle.cos() + dz * self.angle.sin()) / rx;
        let b = (-dx * self.angle.sin() + dz * self.angle.cos()) / rz;
        let theta = b.atan2(a);
        let variation = 1.0 + 0.085 * (theta * 3.0 + 1.3).sin() + 0.055 * (theta * 5.0 - 0.4).cos();
        (a.hypot(b) / variation, theta)
    }
    fn elevation(&self, x: f64, z: f64) -> f64 {
        let (s, angle) = self.normalized(x, z);
        if s >= 1.0 {
            return -((s - 1.0) * 85.0).min(140.0);
        }
        let relief = noise(x, z) * 12.0 * (1.0 - s);
        let shape = if self.crater {
            let ring = (-((s - 0.29) / 0.20).powi(2)).exp();
            (1.0 - s).powf(0.75) * (0.41 + 0.7544 * ring) * (1.0 + 0.09 * (angle * 4.0).sin())
        } else {
            // Angular variation must vanish at the origin: azimuth is undefined
            // there and otherwise creates abrupt needle-like height jumps.
            let angular_weight = smoothstep(s / 0.22);
            (1.0 - s).powf(0.90) * (1.0 + 0.10 * angular_weight * (angle * 3.0 + s * 9.0).sin())
        };
        self.peak * shape + relief
    }
}
impl NorthernSpec {
    /// Validate source bounds and the locked complete island roster.
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.version != 1 || self.id.is_empty() || self.islands.len() != 11 {
            return Err(ContractError::new(
                "northern",
                "version1 and exactly11 islands required",
            ));
        }
        let mut counts = [0; 3];
        let mut ids = std::collections::BTreeSet::new();
        for island in &self.islands {
            let Some(count) = counts.get_mut(usize::from(island.cluster)) else {
                return Err(ContractError::new("island", "unknown cluster"));
            };
            *count += 1;
            if !ids.insert(&island.id)
                || !island
                    .center
                    .iter()
                    .chain(&island.radii)
                    .chain([&island.angle, &island.peak])
                    .all(|v| v.is_finite())
                || island.radii.iter().any(|r| *r < 10.0 || *r > 300.0)
                || !(10.0..=350.0).contains(&island.peak)
            {
                return Err(ContractError::new(
                    "island",
                    "invalid dimensions or duplicate identity",
                ));
            }
        }
        if counts != [4, 4, 3] || self.islands.iter().filter(|i| i.crater).count() != 1 {
            return Err(ContractError::new(
                "northern",
                "requires4/4/3 islands and one dormant crater",
            ));
        }
        Ok(())
    }
    /// Composed topmost solid level; broad submarine ridges precede sea fill.
    pub fn surface(&self, p: WorldHex) -> NorthernSurface {
        let [x, z] = world_xz(p);
        let mut relative = -110.0
            + 17.0 * (x * 0.0031 + z * 0.0013).sin()
            + 12.0 * (x * 0.0017 - z * 0.0043).cos();
        let mut closest: Option<(&IslandSpec, f64)> = None;
        for island in &self.islands {
            let (s, _) = island.normalized(x, z);
            if closest.is_none_or(|(_, old)| s < old) {
                closest = Some((island, s));
            }
            relative = relative.max(island.elevation(x, z));
        }
        let bay = bay_radius(x, z);
        relative = bay_elevation(relative, x, z);
        relative = settlement_elevation(relative, p, x, z);
        let rockness = noise(x + 38.0, z - 14.0);
        let material = if relative > 210.0 {
            "snow"
        } else if relative < -1.0 {
            if rockness > 0.2 { "slate" } else { "stone" }
        } else if relative < 4.0 && bay < 1.3 {
            "sand"
        } else if relative < 130.0 && rockness > -0.05 {
            "moss"
        } else if closest.is_some_and(|(i, s)| i.crater && s < 0.45) {
            "basalt"
        } else if rockness < -0.15 {
            "slate"
        } else {
            "stone"
        };
        NorthernSurface {
            level: (((140.0 + relative) / LEVEL_HEIGHT).floor() as i32).max(3) - 1,
            material,
        }
    }
}
fn palette() -> Vec<MaterialSpec> {
    let mut out: Vec<_> = [
        ("bedrock", [54, 64, 76, 255]),
        ("basalt", [63, 70, 83, 255]),
        ("stone", [115, 124, 134, 255]),
        ("slate", [87, 105, 122, 255]),
        ("soil", [87, 75, 58, 255]),
        ("moss", [76, 102, 80, 255]),
        ("snow", [216, 229, 236, 255]),
        ("sand", [157, 150, 132, 255]),
        ("timber", [104, 78, 57, 255]),
        ("foliage", [52, 84, 75, 255]),
        ("roof", [69, 78, 83, 255]),
        ("crop", [142, 143, 87, 255]),
        ("water", [37, 104, 150, 180]),
    ]
    .into_iter()
    .map(|(id, color)| MaterialSpec {
        id: id.into(),
        solid: id != "water",
        diggable: id != "water",
        color,
    })
    .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}
impl NorthernCompiler {
    /// Prepare deterministic object reservations without generating global terrain columns.
    pub fn new(source: NorthernSpec) -> Result<Self, ContractError> {
        source.validate()?;
        let source_fingerprint = hash_serializable(&source)?;
        let (objects, tree_count) = objects::compose(&source)?;
        let mut roots: BTreeMap<ChunkId, Vec<ObjectInstance>> = BTreeMap::new();
        let mut influences: BTreeMap<ChunkId, Vec<ObjectInfluence>> = BTreeMap::new();
        for object in objects {
            for (chunk, influence) in object.influences()? {
                influences.entry(chunk).or_default().push(influence);
            }
            roots
                .entry(object.origin.column.chunk())
                .or_default()
                .push(object);
        }
        let anchors = [
            ("party_start", SPAWN_XZ[0], SPAWN_XZ[1]),
            ("bay", BAY_XZ[0], BAY_XZ[1]),
            ("settlement", 0.0, 575.0),
            ("crater", -540.0, -250.0),
            ("old_mountains", 350.0, -300.0),
        ]
        .into_iter()
        .map(|(id, x, z)| {
            let p = nearest_hex(x, z);
            WorldAnchor {
                id: format!("northern/anchor/{id}"),
                region_id: REGION.into(),
                position: VoxelPosition {
                    column: p,
                    level: if id == "bay" {
                        SEA_TOP - 1
                    } else {
                        source.surface(p).level
                    },
                },
                role: if id == "party_start" || id == "settlement" {
                    AnchorRole::Gameplay
                } else {
                    AnchorRole::Observation
                },
            }
        })
        .collect();
        Ok(Self {
            source,
            source_fingerprint,
            materials: palette(),
            objects: roots,
            influences,
            anchors,
            tree_count,
        })
    }
    /// All candidate chunk IDs in canonical order; edge chunks can be empty.
    pub fn chunk_ids(&self) -> impl Iterator<Item = ChunkId> {
        let low = (-RADIUS).div_euclid(CHUNK_SIZE);
        let high = RADIUS.div_euclid(CHUNK_SIZE);
        (low..=high).flat_map(move |q| (low..=high).map(move |r| ChunkId { q, r }))
    }
    /// Compile and seal one complete chunk, with exact cross-chunk object influences.
    pub fn chunk(&self, id: ChunkId) -> Result<Option<ChunkPackage>, ContractError> {
        let origin = id.origin()?;
        let mut columns = Vec::new();
        let mut liquids = Vec::new();
        for dq in 0..CHUNK_SIZE {
            for dr in 0..CHUNK_SIZE {
                let p = origin.checked_add(WorldHex::new(dq, dr))?;
                if p.checked_distance(WorldHex::new(0, 0))? > RADIUS as u64 {
                    continue;
                }
                let surface = self.source.surface(p);
                let top = surface.level + 1;
                let soil = (top - 4).max(1);
                let mut runs = vec![
                    VoxelRun {
                        bottom: 0,
                        top: 1,
                        material: "bedrock".into(),
                    },
                    VoxelRun {
                        bottom: 1,
                        top: soil,
                        material: "stone".into(),
                    },
                    VoxelRun {
                        bottom: soil,
                        top: top - 1,
                        material: if surface.material == "moss" {
                            "soil"
                        } else {
                            "slate"
                        }
                        .into(),
                    },
                    VoxelRun {
                        bottom: top - 1,
                        top,
                        material: surface.material.into(),
                    },
                ];
                runs.retain(|run| run.bottom < run.top);
                if let Some(liquid) =
                    super::fill_sea_column(p, &mut runs, SEA_TOP - 1, "water", "northern/ocean")?
                {
                    liquids.push(liquid);
                }
                columns.push(ColumnData { position: p, runs });
            }
        }
        if columns.is_empty() {
            return Ok(None);
        }
        let object_influences = self.influences.get(&id).cloned().unwrap_or_default();
        let occupancy = union_object_occupancy(&object_influences)?;
        let objects = self.objects.get(&id).cloned().unwrap_or_default();
        let anchors = self
            .anchors
            .iter()
            .filter(|a| a.position.column.chunk() == id)
            .cloned()
            .collect();
        let features = objects
            .iter()
            .map(|o| FeatureSummary {
                id: o.id.clone(),
                region_id: REGION.into(),
                kind: if o.asset.starts_with("plant/") {
                    "tree"
                } else {
                    "building"
                }
                .into(),
                anchor: o.origin,
                asset: Some(o.asset.clone()),
            })
            .collect();
        let mut chunk = ChunkPackage {
            schema_version: SCHEMA_VERSION,
            world_id: self.source.id.clone(),
            coordinate: id,
            source_fingerprint: self.source_fingerprint,
            columns,
            features,
            semantics: ChunkSemantics {
                object_influences,
                occupancy,
                objects,
                liquids,
                anchors,
                ..Default::default()
            },
            fingerprint: 0,
        };
        chunk.seal()?;
        Ok(Some(chunk))
    }
    /// Unsealed independent manifest; the writer fills chunk descriptors then seals it.
    pub fn manifest(&self) -> WorldManifest {
        WorldManifest {
            schema_version: SCHEMA_VERSION,
            world_id: self.source.id.clone(),
            compiler_version: "hex-northern/2".into(),
            source_fingerprint: self.source_fingerprint,
            materials: self.materials.clone(),
            regions: vec![RegionDescriptor {
                id: REGION.into(),
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
    /// Cached regular grid, actual quantized relief and supported observation locations.
    pub fn overview(&self) -> NorthernOverview {
        let origin_xz = [-1216.0, -1056.0];
        let spacing = 4.0;
        let width = 609;
        let height = 529;
        let mut bed_heights = Vec::with_capacity(width * height);
        let mut surface_materials = Vec::with_capacity(width * height);
        for row in 0..height {
            for col in 0..width {
                let p = nearest_hex(
                    origin_xz[0] + col as f64 * spacing,
                    origin_xz[1] + row as f64 * spacing,
                );
                let surface = self.source.surface(p);
                bed_heights.push(((surface.level + 1) as f64 * LEVEL_HEIGHT) as f32);
                surface_materials.push(
                    self.materials
                        .iter()
                        .position(|m| m.id == surface.material)
                        .unwrap_or(0) as u16,
                );
            }
        }
        let anchors: BTreeMap<_, _> = self
            .anchors
            .iter()
            .map(|a| {
                let [x, z] = world_xz(a.position.column);
                (
                    a.id.rsplit('/').next().unwrap_or(&a.id).into(),
                    [
                        x as f32,
                        ((a.position.level + 1) as f64 * LEVEL_HEIGHT) as f32,
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
            hex_radius: 1.0,
            level_height: LEVEL_HEIGHT as f32,
            vertical_offset: LEVEL_HEIGHT as f32,
            radius: RADIUS as u32,
            level_bounds: [0, 1400],
            sea_level: 140.0,
            origin_xz: origin_xz.map(|v| v as f32),
            spacing: spacing as f32,
            width: width as u32,
            height: height as u32,
            bed_heights,
            surface_materials,
            materials: self.materials.clone(),
            player_spawn: anchors.get("party_start").copied().unwrap_or([0.0; 3]),
            anchors,
            islands: self.source.islands.clone(),
            tree_count: self.tree_count,
            building_count: 4,
        }
    }
}
