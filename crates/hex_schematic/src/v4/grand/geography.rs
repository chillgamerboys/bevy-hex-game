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
    pub(super) foothills: Foothills,
    #[serde(default)]
    pub(super) interior_profile: Option<InteriorProfile>,
    #[serde(default)]
    pub(super) mountain_composition: Option<MountainComposition>,
    #[serde(default)]
    pub(super) upper_mountain_bodies: Option<UpperMountainBodies>,
    #[serde(default)]
    pub(super) mountain_envelope: Option<MountainEnvelope>,
    pub(super) massif: [f64; 5],
    pub(super) headland: [f64; 5],
    pub(super) peaks: Vec<[f64; 5]>,
    pub(super) ridge_links: Vec<[usize; 2]>,
    pub(super) upper_lake: Lake,
    #[serde(default)]
    pub(super) upper_lake_shore: Option<LakeShore>,
    pub(super) garden: Ellipse,
    #[serde(default)]
    pub(super) garden_access: Option<GardenAccess>,
    pub(super) falls: Watercourse,
    pub(super) lower_lake: Lake,
    #[serde(default)]
    pub(super) lower_lake_shore: Option<LakeShore>,
    pub(super) river: Watercourse,
    #[serde(default)]
    pub(super) ordinary_channel_banks: Option<ChannelBanks>,
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
    pub(super) site_blends: Vec<[f64; 5]>,
    pub(super) frozen_landing: Landing,
    pub(super) rooms: Vec<Room>,
    pub(super) layer_routes: BTreeMap<String, LayerRoute>,
    pub(super) sailing_start: [f64; 2],
    pub(super) sailing_start_bay: [f64; 2],
    pub(super) low_hills: Vec<[f64; 5]>,
    pub(super) valley_bowl: ValleyBowl,
    pub(super) caldera: Caldera,
    pub(super) fountain_basin: Lake,
    pub(super) fountain_rill: Watercourse,
    pub(super) review_cameras: BTreeMap<String, Camera>,
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
shape!(Foothills {
    radius_multiplier: f64,
    minimum_radius: f64,
    apron_relief: f64,
    core_setback: f64,
    core_power: f64,
    shore_grade: f64,
    core_shore_blend: f64,
    base_noise: f64,
    basin_south_blend: [f64; 2],
    shore_headlands: Vec<[f64; 5]>,
    coast_noise_fade: [f64; 2]
});
shape!(InteriorProfile {
    join_width: f64,
    western_relief: f64,
    western_radius_multiplier: f64,
    western_power: f64
});
shape!(MountainComposition {
    crystal_rim_height: [f64; 2],
    frozen_support_width: [f64; 2],
    frozen_verge: f64,
    frozen_undulation: f64,
    frozen_toe_rise: f64,
    peak_rounding_radius: f64,
    phase: f64
});
// Absolute sea-relative heights; these bodies do not subtract the apron.
shape!(UpperMountainBodies {
    height_transition: [f64; 2],
    maximum_uplift: f64,
    edge_blend_fraction: f64,
    bodies: Vec<UpperMountainBody>
});
// Each spine node is [model east, model north, absolute crest height, width].
shape!(UpperMountainBody {name: String, spine: Vec<[f64; 4]>});
// Shared landform before lake, ascent and cave carving. Broad spines are
// absolute sea-relative relief, not extra height added once per landmark.
shape!(MountainEnvelope {shore_blend: f64, bodies: Vec<EnvelopeBody>});
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EnvelopeBody {
    pub name: String,
    pub spine: Vec<[f64; 4]>,
    pub lower_slope_power: f64,
    /// Only the inner crest; zero retains the pointed profile.
    #[serde(default)]
    pub crest_rounding_radius: f64,
}
shape!(Coast {center:[f64;2],radii:[f64;2],phase:f64,coves:Vec<[f64;4]>});
shape!(Ellipse {
    center: [f64; 2],
    radii: [f64; 2]
});
shape!(GardenAccess {
    points: Vec<[f64; 3]>,
    width: f64,
    bank_blend: f64,
    wet_fan_radius: f64,
    wet_edge_depth: f64
});
shape!(Lake {
    center: [f64; 2],
    radii: [f64; 2],
    level: f64,
    phase: f64
});
shape!(Watercourse {width:f64,points:Vec<[f64;3]>});
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LakeShore {
    pub shelf_width: [f64; 2],
    pub outer_blend: [f64; 2],
    pub shelf_level: f64,
    pub shelf_grade: f64,
    pub phase: f64,
    #[serde(default)]
    pub wet_width: Option<[f64; 2]>,
    #[serde(default)]
    pub wet_edge_depth: Option<f64>,
}
shape!(ChannelBanks {
    max_longitudinal_grade: f64,
    plunge_buffer: f64,
    dry_lip: f64,
    shelf_power: f64,
    dry_outer_blend: Option<f64>,
    dry_reach_support: Option<DryReachSupport>
});
shape!(DryReachSupport {
    sea_fade_height: f64,
    endpoint_blend: f64,
    maximum_outer_rise: f64
});
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
    outer_apothem: f64,
    expected_columns: usize,
    stair_radius: f64,
    turns: f64,
    width: f64,
    riser: f64,
    temple_radius: f64
});
shape!(FrozenRoute {width:f64,forest_half_width:f64,points:Vec<[f64;3]>});
shape!(VolcanoRoute {width:f64,local_points:Vec<[f64;3]>});
shape!(FrameSpec {origin:[f64;2],angle:f64,layer:SupportLayer,floor:Option<f64>});

shape!(Landing {
    center: [f64; 2],
    radii: [f64; 2],
    height: f64
});
shape!(Room {
    frame: String,
    half_extents: [f64; 2],
    height: f64
});
shape!(LayerRoute {from:String,to:String,points:Vec<[f64;3]>,width:f64,layer:SupportLayer,open_ends:[bool;2]});
shape!(Camera {eye:[f64;3],target:[f64;3],interest:[f64;3],horizontal_span:Option<f64>,ground:bool});

shape!(ValleyBowl {
    offset: [f64; 2],
    radii: [f64; 2],
    level: f64
});
shape!(Caldera {
    radii: [f64; 2],
    floor: f64,
    rim: f64,
    amplitude: f64
});

impl GrandGeographyDocument {
    fn validate_dimensions(&self) -> Result<(), ContractError> {
        let bounded = |n: usize, min, max| n >= min && n <= max;
        let radii = |r: [f64; 2]| r.iter().all(|v| v.is_finite() && (1. ..=1500.).contains(v));
        let position = |p: [f64; 2]| p.iter().all(|v| v.is_finite() && v.abs() <= 4000.);
        let path = |p: &[[f64; 3]]| {
            bounded(p.len(), 2, 256)
                && p.iter().all(|v| {
                    position([v[0], v[2]]) && v[1].is_finite() && (-100. ..=560.).contains(&v[1])
                })
                && p.windows(2).all(|s| {
                    s.first()
                        .zip(s.get(1))
                        .is_some_and(|(a, b)| (a[0] - b[0]).hypot(a[2] - b[2]) > 0.01)
                })
        };
        if self.upper_mountain_bodies.as_ref().is_some_and(|profile| {
            !(120. ..=170.).contains(&profile.height_transition[0])
                || !(profile.height_transition[0] + 30. ..=230.)
                    .contains(&profile.height_transition[1])
                || !(0. ..=50.).contains(&profile.maximum_uplift)
                || !(0.1..=0.4).contains(&profile.edge_blend_fraction)
                || !bounded(profile.bodies.len(), 1, 3)
                || profile.bodies.iter().any(|body| {
                    body.name.is_empty()
                        || body.name.len() > 64
                        || !bounded(body.spine.len(), 2, 8)
                        || body.spine.iter().any(|node| {
                            !position([node[0], node[1]])
                                || !(self.upper_lake.level..=360.).contains(&node[2])
                                || !(40. ..=160.).contains(&node[3])
                        })
                        || body.spine.windows(2).any(|nodes| {
                            nodes
                                .first()
                                .zip(nodes.get(1))
                                .is_none_or(|(a, b)| (a[0] - b[0]).hypot(a[1] - b[1]) < 1.)
                        })
                })
        }) {
            return Err(ContractError::new(
                "grand.upper_mountains",
                "invalid bounded upper-rock bodies",
            ));
        }
        if self.mountain_envelope.as_ref().is_some_and(|profile| {
            !(100. ..=240.).contains(&profile.shore_blend)
                || !bounded(profile.bodies.len(), 2, 8)
                || profile.bodies.iter().any(|body| {
                    !(1. ..=3.).contains(&body.lower_slope_power)
                        || !(0. ..=0.45).contains(&body.crest_rounding_radius)
                        || body.name.is_empty()
                        || body.name.len() > 64
                        || !bounded(body.spine.len(), 2, 12)
                        || body.spine.iter().any(|node| {
                            !position([node[0], node[1]])
                                || !(70. ..=450.).contains(&node[2])
                                || !(160. ..=700.).contains(&node[3])
                        })
                        || body.spine.windows(2).any(|nodes| {
                            nodes
                                .first()
                                .zip(nodes.get(1))
                                .is_none_or(|(a, b)| (a[0] - b[0]).hypot(a[1] - b[1]) < 1.)
                        })
                })
        }) {
            return Err(ContractError::new(
                "grand.mountain_envelope",
                "invalid connected mountain envelope",
            ));
        }
        let f = &self.foothills;
        if self.interior_profile.as_ref().is_some_and(|p| {
            !(1. ..=60.).contains(&p.join_width)
                || !(150. ..=450.).contains(&p.western_relief)
                || !(0.75..=1.5).contains(&p.western_radius_multiplier)
                || !(1. ..=4.).contains(&p.western_power)
        }) || self.mountain_composition.as_ref().is_some_and(|m| {
            !(self.ascent.top..=self.ascent.top + 25.).contains(&m.crystal_rim_height[0])
                || !(m.crystal_rim_height[0]..=self.ascent.top + 50.)
                    .contains(&m.crystal_rim_height[1])
                || !(self.frozen_route.forest_half_width + 20. ..=180.)
                    .contains(&m.frozen_support_width[0])
                || !(m.frozen_support_width[0]..=240.).contains(&m.frozen_support_width[1])
                || !(4. ..=20.).contains(&m.frozen_verge)
                || !(self.frozen_route.width * 0.5 + m.frozen_verge + 1. ..=180.)
                    .contains(&self.frozen_route.forest_half_width)
                || !(0. ..=15.).contains(&m.frozen_undulation)
                || !(0. ..=8.).contains(&m.frozen_toe_rise)
                || !(0.35..=0.7).contains(&m.peak_rounding_radius)
                || !m.phase.is_finite()
        }) || !(1. ..=3.).contains(&f.radius_multiplier)
            || [
                (self.upper_lake_shore.as_ref(), &self.upper_lake),
                (self.lower_lake_shore.as_ref(), &self.lower_lake),
            ]
            .into_iter()
            .any(|(shore, lake)| {
                shore.is_some_and(|s| {
                    !(2. ..=24.).contains(&s.shelf_width[0])
                        || !(s.shelf_width[0]..=32.).contains(&s.shelf_width[1])
                        || !(12. ..=80.).contains(&s.outer_blend[0])
                        || !(s.outer_blend[0]..=100.).contains(&s.outer_blend[1])
                        || !(lake.level..=lake.level + 2.).contains(&s.shelf_level)
                        || !(0. ..=0.2).contains(&s.shelf_grade)
                        || !s.phase.is_finite()
                        || s.wet_width.is_some() != s.wet_edge_depth.is_some()
                        || s.wet_width.is_some_and(|width| {
                            !(12. ..=60.).contains(&width[0])
                                || !(width[0]..=80.).contains(&width[1])
                        })
                        || s.wet_edge_depth
                            .is_some_and(|depth| !(0.35..=1.).contains(&depth))
                })
            })
            || self.ordinary_channel_banks.as_ref().is_some_and(|b| {
                !(0.05..=0.4).contains(&b.max_longitudinal_grade)
                    || !(4. ..=30.).contains(&b.plunge_buffer)
                    || !(0. ..=1.).contains(&b.dry_lip)
                    || !(1.5..=3.).contains(&b.shelf_power)
                    || b.dry_outer_blend
                        .is_some_and(|width| !(40. ..=160.).contains(&width))
                    || b.dry_reach_support.as_ref().is_some_and(|s| {
                        b.dry_outer_blend.is_none()
                            || !(4. ..=20.).contains(&s.sea_fade_height)
                            || !(12. ..=40.).contains(&s.endpoint_blend)
                            || !(1. ..=24.).contains(&s.maximum_outer_rise)
                    })
            })
            || self.garden_access.as_ref().is_some_and(|a| {
                !path(&a.points)
                    || a.points.len() > 16
                    || !(4. ..=10.).contains(&a.width)
                    || !(4. ..=16.).contains(&a.bank_blend)
                    || !(12. ..=32.).contains(&a.wet_fan_radius)
                    || !(0.35..=0.7).contains(&a.wet_edge_depth)
                    || a.points.iter().any(|p| {
                        ellipse([p[0], p[2]], self.garden.center, self.garden.radii) > 1.
                            || !(self.upper_lake.level..=self.upper_lake.level + 10.)
                                .contains(&p[1])
                    })
            })
            || !(100. ..=1000.).contains(&f.minimum_radius)
            || !(1. ..=150.).contains(&f.apron_relief)
            || !(0.25..=1.).contains(&f.core_setback)
            || !(1. ..=4.).contains(&f.core_power)
            || !(0.05..=0.5).contains(&f.shore_grade)
            || !(20. ..=200.).contains(&f.core_shore_blend)
            || !(0. ..=3.).contains(&f.base_noise)
            || !(f.basin_south_blend[0] + 1. ..=1200.).contains(&f.basin_south_blend[1])
            || !f.basin_south_blend[0].is_finite()
            || f.shore_headlands.len() > 8
            || f.shore_headlands.iter().any(|v| {
                !position([v[0], v[1]]) || !(0. ..=0.6).contains(&v[2]) || !radii([v[3], v[4]])
            })
            || !(0. ..f.coast_noise_fade[1]).contains(&f.coast_noise_fade[0])
            || !(0.1..=0.8).contains(&f.coast_noise_fade[1])
            || !bounded(self.frames.len(), 1, 128)
            || !bounded(self.peaks.len(), 1, 32)
            || self.ridge_links.len() > 64
            || self.site_shoulders.len() > 32
            || self.site_blends.len() > 32
            || self.low_hills.len() > 32
            || self.coast.coves.len() > 32
            || self.landform_ridges.len() > 16
            || self
                .landform_ridges
                .iter()
                .any(|p| !bounded(p.len(), 2, 64))
            || !(60. ..=350.).contains(&self.tree.height)
            || self.rooms.len() > 16
            || self.layer_routes.len() > 32
            || self.review_cameras.len() > 64
            || ![
                self.coast.radii,
                self.upper_lake.radii,
                self.lower_lake.radii,
                self.garden.radii,
                self.forest.radii,
                self.volcano.radii,
                self.frozen_landing.radii,
                self.caldera.radii,
                self.valley_bowl.radii,
                self.fountain_basin.radii,
            ]
            .into_iter()
            .all(radii)
            || self
                .ridge_links
                .iter()
                .any(|p| p.iter().any(|i| *i >= self.peaks.len()))
            || self
                .peaks
                .iter()
                .chain(&self.site_shoulders)
                .chain(&self.site_blends)
                .chain(&self.low_hills)
                .any(|p| {
                    !position([p[0], p[1]]) || !(0. ..=560.).contains(&p[2]) || !radii([p[3], p[4]])
                })
            || ![&self.falls, &self.river, &self.fountain_rill]
                .into_iter()
                .all(|c| path(&c.points) && (1. ..=80.).contains(&c.width))
            || !path(&self.shadow_route)
            || !path(&self.frozen_route.points)
            || !path(&self.volcano_route.local_points)
            || !(1000..=50000).contains(&self.ascent.expected_columns)
            || !(5. ..=40.).contains(&self.ascent.width)
            || !(20. ..=160.).contains(&self.ascent.well_apothem)
            || !(self.ascent.well_apothem + 8. ..=200.).contains(&self.ascent.outer_apothem)
            || !(10. ..self.ascent.well_apothem).contains(&self.ascent.stair_radius)
            || !(self.ascent.base..=400.).contains(&self.ascent.top)
            || self
                .layer_routes
                .values()
                .any(|r| !path(&r.points) || !(3. ..=48.).contains(&r.width))
            || self.frames.values().any(|f| {
                !position(f.origin)
                    || !f.angle.is_finite()
                    || f.floor
                        .is_some_and(|h| !h.is_finite() || !(0. ..=560.).contains(&h))
            })
            || self.rooms.iter().any(|r| {
                !self.frames.contains_key(&r.frame)
                    || !radii(r.half_extents)
                    || r.half_extents.iter().any(|v| *v > 100.)
                    || !(3. ..=40.).contains(&r.height)
            })
            || self.review_cameras.values().any(|c| {
                !c.eye
                    .iter()
                    .chain(&c.target)
                    .chain(&c.interest)
                    .all(|v| v.is_finite() && v.abs() <= 10000.)
                    || c.eye
                        .iter()
                        .zip(c.target)
                        .map(|(a, b)| (a - b).powi(2))
                        .sum::<f64>()
                        <= f64::EPSILON
                    || c.horizontal_span
                        .is_some_and(|s| !s.is_finite() || !(1. ..=10000.).contains(&s))
                    || (c.ground && c.horizontal_span.is_some())
            })
        {
            return Err(ContractError::new(
                "grand.geography",
                "invalid or over-budget authored dimensions, routes, frames or cameras",
            ));
        }
        Ok(())
    }
}

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
        document.validate_dimensions()?;
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
        // Tree height is an authored physical asset dimension. Terrain relief
        // scaling must not squash this accepted crown while leaving its width
        // and all ordinary trees unchanged.
        let height = self.document.as_ref().map_or(235.451, |d| d.tree.height);
        // Measured outer-lobe union of approved seed712, not the nominal lobe radius.
        TreeDimensions {
            crown_radii: [166.744 * s, 150.877 * s],
            root_reach: 100. * s,
            height,
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
        if frame.layer != SupportLayer::Exterior && frame.floor.is_none() {
            return Err(ContractError::new(
                "grand.geography",
                "named interior layer needs an explicit floor",
            ));
        }
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
        if self.r02_reserved(p, bottom, top) {
            return true;
        }
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
