//! One source-owned coordinate frame for the approved revision-02 geography.
//! Model local coordinates are east/north; runtime north is negative Z.
use super::{nearest_hex, world_xz, GrandCompiler, LEVEL_HEIGHT, SEA_TOP};
use hex_world_contracts::{ContractError, VoxelPosition, WorldHex};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Strict, versioned authoring document loaded by the filesystem-owning tool.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrandGeographyDocument {
    /// Schema version.
    pub version: u32,
    /// Frozen approved model identity, distinct from production validation.
    pub approval: String,
    /// Exact calibration method/status for the production transform.
    pub calibration: String,
    pub(super) transform: GeographyTransform,
    pub(super) coast: Coast,
    pub(super) massif: [f64; 5],
    pub(super) headland: [f64; 5],
    pub(super) peaks: Vec<[f64; 5]>,
    pub(super) ridge_links: Vec<[usize; 2]>,
    pub(super) upper_lake: Lake,
    pub(super) garden: Ellipse,
    pub(super) falls: Watercourse,
    pub(super) lower_lake: Lake,
    pub(super) river: Watercourse,
    pub(super) tree: Tree,
    pub(super) forest: Forest,
    pub(super) volcano: Volcano,
    pub(super) site_shoulders: Vec<[f64; 5]>,
    pub(super) landform_ridges: Vec<Vec<[f64; 4]>>,
    pub(super) ascent: Ascent,
    pub(super) frozen_route: FrozenRoute,
    pub(super) volcano_route: VolcanoRoute,
    pub(super) shadow_route: Vec<[f64; 3]>,
    pub(super) library_concept: Vec<[f64; 3]>,
    pub(super) frames: BTreeMap<String, FrameSpec>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GeographyTransform {
    pub horizontal_scale: f64,
    pub vertical_scale: f64,
    pub translation: [f64; 2],
    pub sea_top: i32,
}
macro_rules! shape {
    ($name:ident {$($field:ident:$ty:ty),*$(,)?}) => {
        #[derive(Clone, Debug, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(super) struct $name {$(pub $field:$ty),*}
    };
}
shape!(Coast {center:[f64;2],radii:[f64;2],phase:f64,coves:Vec<[f64;4]>});
shape!(Ellipse {
    center: [f64; 2],
    radii: [f64; 2]
});
shape!(Lake {
    center: [f64; 2],
    radii: [f64; 2],
    level: f64,
    phase: f64
});
shape!(Watercourse {width:f64,points:Vec<[f64;3]>});
shape!(Tree {
    center: [f64; 2],
    height: f64,
    crown_radius: f64
});
shape!(Forest {
    center: [f64; 2],
    radii: [f64; 2],
    seed: u64
});
shape!(Volcano {
    center: [f64; 2],
    radii: [f64; 2],
    height: f64,
    phase: f64
});
shape!(Ascent {
    center: [f64; 2],
    base: f64,
    top: f64,
    well_apothem: f64,
    stair_radius: f64,
    turns: f64,
    width: f64,
    riser: f64,
    temple_radius: f64
});
shape!(FrozenRoute {width:f64,forest_half_width:f64,points:Vec<[f64;3]>});
shape!(VolcanoRoute {width:f64,local_points:Vec<[f64;3]>});
shape!(FrameSpec {origin:[f64;2],angle:f64,layer:SupportLayer,floor:Option<f64>});

/// Named stacked terrain layer; an interior never means the topmost roof.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) enum SupportLayer {
    Exterior,
    CrystalFloor,
    LibraryLower,
    LibraryUpper,
    Shadow,
    RootTemple,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GrandRegion {
    Mainland,
    CrystalAscent,
    FrozenWoods,
    Forest,
    GardenIsland,
    UpperLake,
    ValleyLake,
    Volcano,
}
/// Final runtime bounds, including unequal outer crown lobes.
#[derive(Clone, Copy, Debug)]
pub(super) struct TreeDimensions {
    pub crown_radii: [f64; 2],
    pub root_reach: f64,
    pub height: f64,
}
#[derive(Clone, Debug)]
pub(super) struct LandmarkFrame {
    pub layer: SupportLayer,
    pub(super) floor: Option<f64>,
    origin: [f64; 2],
    angle: f64,
    transform: GeographyTransform,
}
impl LandmarkFrame {
    pub fn xz(&self, local: [f64; 2]) -> [f64; 2] {
        let (sin, cos) = self.angle.sin_cos();
        let [x, z] = [
            self.origin[0] + cos * local[0] - sin * local[1],
            self.origin[1] + sin * local[0] + cos * local[1],
        ];
        [
            self.transform.translation[0] + x * self.transform.horizontal_scale,
            self.transform.translation[1] - z * self.transform.horizontal_scale,
        ]
    }
    pub fn hex(&self, local: [f64; 2]) -> WorldHex {
        let [x, z] = self.xz(local);
        nearest_hex(x, z)
    }
    pub fn local(&self, p: WorldHex) -> [f64; 2] {
        let [x, z] = world_xz(p);
        let s = self.transform.horizontal_scale;
        let x = (x - self.transform.translation[0]) / s - self.origin[0];
        let z = -(z - self.transform.translation[1]) / s - self.origin[1];
        let (sin, cos) = self.angle.sin_cos();
        [cos * x + sin * z, -sin * x + cos * z]
    }
    pub fn length(&self, model_units: f64) -> f64 {
        model_units * self.transform.horizontal_scale
    }
}
/// Validated source geometry. Legacy callers retain the prior producer explicitly.
#[derive(Clone, Debug)]
pub struct GrandGeography {
    pub(super) document: Option<GrandGeographyDocument>,
}
impl GrandGeography {
    pub(super) fn legacy() -> Self {
        Self { document: None }
    }
    /// Validate the typed document before any geometry or allocations are derived.
    pub fn new(document: GrandGeographyDocument) -> Result<Self, ContractError> {
        let t = document.transform;
        if document.version != 1
            || !document.approval.starts_with("shared-topology-r02/")
            || !(0.1..=4.).contains(&t.horizontal_scale)
            || !(0.1..=4.).contains(&t.vertical_scale)
            || t.sea_top != SEA_TOP
            || !t.translation.iter().all(|v| v.is_finite())
            || document.frames.is_empty()
            || document.peaks.is_empty()
        {
            return Err(ContractError::new(
                "grand.geography",
                "invalid revision-02 transform or identity",
            ));
        }
        let bytes = ron::to_string(&document)
            .map_err(|e| ContractError::new("grand.geography", e.to_string()))?;
        if bytes.contains("NaN") || bytes.contains("inf") {
            return Err(ContractError::new("grand.geography", "nonfinite geometry"));
        }
        Ok(Self {
            document: Some(document),
        })
    }
    fn transform(&self) -> GeographyTransform {
        self.document.as_ref().map_or(
            GeographyTransform {
                horizontal_scale: 1.,
                vertical_scale: 1.,
                translation: [0., 0.],
                sea_top: SEA_TOP,
            },
            |d| d.transform,
        )
    }
    pub(super) fn model_xz(&self, p: WorldHex) -> [f64; 2] {
        let [x, z] = world_xz(p);
        let t = self.transform();
        [
            (x - t.translation[0]) / t.horizontal_scale,
            -(z - t.translation[1]) / t.horizontal_scale,
        ]
    }
    pub(super) fn world_xz(&self, model: [f64; 2]) -> [f64; 2] {
        let t = self.transform();
        [
            t.translation[0] + model[0] * t.horizontal_scale,
            t.translation[1] - model[1] * t.horizontal_scale,
        ]
    }
    pub(super) fn world_hex(&self, model: [f64; 2]) -> WorldHex {
        let [x, z] = self.world_xz(model);
        nearest_hex(x, z)
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "Validated model height is rounded once to the production level grid."
    )]
    pub(super) fn top_level(&self, model_y: f64) -> i32 {
        let t = self.transform();
        t.sea_top + (model_y * t.vertical_scale / LEVEL_HEIGHT).round() as i32
    }
    pub(super) fn length(&self, model_units: f64) -> f64 {
        model_units * self.transform().horizontal_scale
    }
    pub(super) fn frame(&self, id: &str) -> Result<LandmarkFrame, ContractError> {
        let d = self.document.as_ref().ok_or_else(|| {
            ContractError::new(
                "grand.geography",
                "legacy compiler has no revision-02 frames",
            )
        })?;
        let f = d.frames.get(id).ok_or_else(|| {
            ContractError::new("grand.geography", format!("missing landmark frame {id}"))
        })?;
        Ok(LandmarkFrame {
            layer: f.layer,
            floor: f.floor,
            origin: f.origin,
            angle: f.angle,
            transform: d.transform,
        })
    }
    pub(super) fn region_contains(&self, region: GrandRegion, p: WorldHex) -> bool {
        let Some(d) = &self.document else {
            return false;
        };
        let point = self.model_xz(p);
        match region {
            GrandRegion::Mainland => ellipse(point, d.coast.center, d.coast.radii) < 1.2,
            GrandRegion::CrystalAscent => ellipse(point, d.ascent.center, [145., 145.]) < 1.,
            GrandRegion::FrozenWoods => {
                route_distance(point, &d.frozen_route.points).0 < d.frozen_route.forest_half_width
            }
            GrandRegion::Forest => self.forest_density(p) > 0.,
            GrandRegion::GardenIsland => ellipse(point, d.garden.center, d.garden.radii) < 1.,
            GrandRegion::UpperLake => {
                irregular(
                    point,
                    d.upper_lake.center,
                    d.upper_lake.radii,
                    d.upper_lake.phase,
                ) < 1.
            }
            GrandRegion::ValleyLake => {
                irregular(
                    point,
                    d.lower_lake.center,
                    d.lower_lake.radii,
                    d.lower_lake.phase,
                ) < 1.
            }
            GrandRegion::Volcano => {
                irregular(point, d.volcano.center, d.volcano.radii, d.volcano.phase) < 1.2
            }
        }
    }
    pub(super) fn forest_density(&self, p: WorldHex) -> f64 {
        let Some(d) = &self.document else {
            return 0.;
        };
        let [x, z] = self.model_xz(p);
        let r = ellipse([x, z], d.forest.center, d.forest.radii);
        ((1.08 - r) * 5.).clamp(0., 1.)
            * (0.72 + 0.20 * (x * 0.014 + z * 0.019).sin()).clamp(0., 1.)
    }
    pub(super) fn frozen_planting_weight(&self, p: WorldHex) -> f64 {
        let Some(d) = &self.document else {
            return 0.;
        };
        let (distance, _, _) = route_distance(self.model_xz(p), &d.frozen_route.points);
        if distance < d.frozen_route.width * 0.5 + 8. {
            0.
        } else {
            ((d.frozen_route.forest_half_width - distance) / 20.).clamp(0., 1.)
        }
    }
    pub(super) fn tree_dimensions(&self) -> TreeDimensions {
        let s = self.transform().horizontal_scale;
        let y = self.transform().vertical_scale;
        // Measured outer-lobe union of approved seed712, not the nominal lobe radius.
        TreeDimensions {
            crown_radii: [166.744 * s, 150.877 * s],
            root_reach: 100. * s,
            height: 235.451 * y,
        }
    }
}
pub(super) fn ellipse([x, z]: [f64; 2], c: [f64; 2], r: [f64; 2]) -> f64 {
    ((x - c[0]) / r[0]).hypot((z - c[1]) / r[1])
}
pub(super) fn irregular([x, z]: [f64; 2], c: [f64; 2], r: [f64; 2], phase: f64) -> f64 {
    let dx = (x - c[0]) / r[0];
    let dz = (z - c[1]) / r[1];
    let a = dz.atan2(dx);
    dx.hypot(dz) / (1. + 0.08 * (3. * a + phase).sin() + 0.04 * (5. * a - 0.7).cos())
}
pub(super) fn segment(point: [f64; 2], a: [f64; 2], b: [f64; 2]) -> (f64, f64) {
    let dx = b[0] - a[0];
    let dz = b[1] - a[1];
    let t = (((point[0] - a[0]) * dx + (point[1] - a[1]) * dz) / (dx * dx + dz * dz).max(1e-12))
        .clamp(0., 1.);
    (
        (point[0] - a[0] - t * dx).hypot(point[1] - a[1] - t * dz),
        t,
    )
}
pub(super) fn route_distance(point: [f64; 2], route: &[[f64; 3]]) -> (f64, f64, f64) {
    let mut best = (f64::INFINITY, 0., 0.);
    let mut progress = 0.;
    for pair in route.windows(2) {
        let (Some(a), Some(b)) = (pair.first(), pair.get(1)) else {
            continue;
        };
        let (distance, t) = segment(point, [a[0], a[2]], [b[0], b[2]]);
        let length = (b[0] - a[0]).hypot(b[2] - a[2]);
        if distance < best.0 {
            best = (distance, a[1] + t * (b[1] - a[1]), progress + t * length);
        }
        progress += length;
    }
    best
}
impl GrandCompiler {
    pub(super) fn support_at(
        &self,
        frame: &LandmarkFrame,
        local: [f64; 2],
    ) -> Result<VoxelPosition, ContractError> {
        let p = frame.hex(local);
        let (c, _) = self.column(p);
        let expected = frame.floor.map(|y| self.geography.top_level(y));
        c.runs
            .iter()
            .filter(|r| r.material != "water")
            .map(|r| r.top - 1)
            .filter(|level| expected.is_none_or(|top| (*level + 1 - top).abs() < 12))
            .max()
            .map(|level| VoxelPosition { column: p, level })
            .ok_or_else(|| {
                ContractError::new("grand.geography", "named layer has no supporting terrain")
            })
    }
    pub(super) fn reserved_interval(&self, p: WorldHex, bottom: i32, top: i32) -> bool {
        let Some(d) = &self.geography.document else {
            return false;
        };
        let point = self.geography.model_xz(p);
        for route in [&d.frozen_route.points, &d.shadow_route] {
            let (distance, y, _) = route_distance(point, route);
            let floor = self.geography.top_level(y);
            if distance < d.frozen_route.width * 0.5 + 3. && bottom < floor + 12 && top > floor {
                return true;
            }
        }
        d.frames.iter().any(|(id, f)| {
            let radius = if id.starts_with("shrine_") {
                5.
            } else if id.starts_with("camp_") {
                15.
            } else {
                return false;
            };
            if ellipse(point, f.origin, [radius, radius]) >= 1. {
                return false;
            }
            let floor = f.floor.map_or_else(
                || self.surface(p).level + 1,
                |y| self.geography.top_level(y),
            );
            bottom < floor + 12 && top > floor
        })
    }
}
