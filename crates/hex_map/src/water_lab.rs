//! Small built-in water fixture and its world-owned stepped surface.

mod render;

use std::collections::{BTreeMap, BTreeSet};

use crate::{Column, VoxelMap};
use bevy::prelude::*;
use hex_assets::SubstanceTable;
use hex_core::arena::ArenaTerrainView;
use hex_core::arena::{ArenaMaterials, ArenaVoxelGeometry};
use hex_core::ocean::{
    OceanEnvironmentSampler, OceanSimulationTime, OceanSurfaceSample, OceanWaterColumn,
    OceanWindProfile,
};
use hex_core::water_lab::{LabWave, LabWind, WaterLabSettings};
use hex_core::{HexCoord, TilePos};

/// Fixture's mean water level, in world units.
pub const SEA_LEVEL: f32 = 8.0;
const SEA_VOXEL: i32 = 20;
const CENTERS: [(i32, i32); 7] = [
    (0, 0),
    (25, -12),
    (12, 13),
    (-13, 25),
    (-25, 12),
    (-12, -13),
    (13, -25),
];

/// Exact finite footprint; no bounding-disk fill or decorative-ocean fallback.
#[must_use]
pub fn contains(coord: HexCoord) -> bool {
    CENTERS.into_iter().any(|(q, r)| {
        let dq = coord.x() - q;
        let dr = coord.y() - r;
        dq.abs().max(dr.abs()).max((dq + dr).abs()) <= 12
    })
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "The bounded fixture quantizes a finite terrain height to voxel levels."
)]
fn bed_level(at: Vec2) -> i32 {
    let island = 16.0 - at.length();
    let cove = (at - Vec2::new(11.0, -6.0)).length() - 8.0;
    let land = island.min(cove);
    let headland = if land > 1.0 {
        (1.0 - ((at - Vec2::new(-8.0, -9.0)).length() - 2.0).max(0.0) / 7.0).clamp(0.0, 1.0) * 48.0
    } else {
        0.0
    };
    let terrain = SEA_VOXEL + (land * 0.8 + headland).floor() as i32;
    // A broad inlet with a submerged bed, tapering toward a sheltered far end.
    let bank = ((at.y - 3.0).abs() / 5.2).clamp(0.0, 1.0);
    let channel_depth = (3.6 - (at.x + 16.0).clamp(0.0, 32.0) * 0.09) * (1.0 - bank * bank);
    let channel = if bank < 1.0 && at.x.abs() < 18.0 {
        SEA_VOXEL - (channel_depth / 0.4).ceil() as i32
    } else {
        terrain
    };
    terrain.min(channel).clamp(0, 85)
}

pub(crate) type Fixture = (
    VoxelMap,
    ArenaVoxelGeometry,
    BTreeMap<String, Vec3>,
    crate::procedural_v3::MapPresentationProjection,
);

pub(crate) fn build(
    materials: ArenaMaterials,
    substances: &SubstanceTable,
) -> Result<Fixture, String> {
    let water = substances
        .id("water")
        .ok_or("Water lab requires the water substance")?;
    let sand = substances.id("sand").unwrap_or(materials.dirt);
    let geometry = ArenaVoxelGeometry {
        radius: 37,
        ..default()
    };
    let mut map = VoxelMap::new();
    let cells: BTreeSet<_> = CENTERS
        .into_iter()
        .flat_map(|(q, r)| HexCoord::from_axial(q, r).within_radius(12))
        .collect();
    for coord in cells {
        let point = coord.to_world(0.0);
        let bed = bed_level(Vec2::new(point.x, point.z));
        let mut column = Column::filled(materials.stone, bed + 1);
        column.set(0, materials.bedrock);
        column.set(
            bed,
            if bed > SEA_VOXEL + 1 {
                materials.grass
            } else {
                sand
            },
        );
        for level in bed + 1..=SEA_VOXEL {
            column.set(level, water);
        }
        map.insert_column(coord, column);
    }
    let shore_coord = HexCoord::from_axial(-7, 7);
    let shore = shore_coord.to_world(geometry.top(TilePos::new(
        shore_coord,
        map.surface(shore_coord).ok_or("Missing lab beach")?,
    )));
    let water_start = HexCoord::from_axial(12, 0).to_world(SEA_LEVEL);
    let launch_coord = HexCoord::from_axial(-2, -6);
    let launch = launch_coord.to_world(
        geometry.top(TilePos::new(
            launch_coord,
            map.surface(launch_coord).ok_or("Missing launch hill")?,
        )) + 2.0,
    );
    let anchors = BTreeMap::from([
        ("party_start".into(), shore),
        ("hostile_start".into(), shore + Vec3::X * 2.0),
        ("player_look_at".into(), water_start),
        ("water_lab_shore".into(), shore),
        ("water_lab_swim".into(), water_start),
        ("water_lab_glider".into(), launch),
    ]);
    Ok((map, geometry, anchors, default()))
}

/// Immutable snapshot shared by contact queries and the finite water renderer.
#[derive(Debug, Clone)]
pub struct LabSurface {
    /// Explicit comparison choices and environmental clock.
    pub settings: WaterLabSettings,
    /// Height of one whole voxel, obtained from world geometry.
    pub level_height: f32,
    columns: BTreeMap<HexCoord, WaveColumn>,
}

#[derive(Debug, Clone, Copy)]
struct WaveColumn {
    water: OceanWaterColumn,
    // Integrated optical distance and dissipated amplitude, for both wave trains.
    travel: Vec2,
    gain: Vec2,
}

impl LabSurface {
    /// Publish current exact solid beds, including the intermittently flooded shore.
    /// The fixture has one exposed bed per column; this is not a stacked-ocean rule.
    #[must_use]
    pub fn new(
        view: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        settings: WaterLabSettings,
    ) -> Self {
        let mut beds = BTreeMap::new();
        if let Some(water) = view.liquids.first().map(|span| span.substance) {
            for (coord, spans) in &view.columns {
                if let Some(top) = spans.iter().map(|span| span.top_level).max() {
                    beds.insert(
                        *coord,
                        OceanWaterColumn {
                            mean_height: SEA_LEVEL,
                            bed_height: geometry.top(TilePos::new(*coord, top)),
                            water_id: water,
                        },
                    );
                }
            }
        }
        Self::from_beds(beds, geometry.level_height, settings)
    }

    fn from_beds(
        beds: BTreeMap<HexCoord, OceanWaterColumn>,
        level_height: f32,
        settings: WaterLabSettings,
    ) -> Self {
        let columns = beds
            .iter()
            .map(|(coord, water)| {
                let p = coord.to_world(0.0);
                let at = Vec2::new(p.x, p.z);
                let (travel_x, gain_x) = propagation(at, Vec2::X, *water, &beds);
                let (travel_cross, gain_cross) =
                    propagation(at, Vec2::new(0.5, 0.866_025_4), *water, &beds);
                (
                    *coord,
                    WaveColumn {
                        water: *water,
                        travel: Vec2::new(travel_x, travel_cross),
                        gain: Vec2::new(gain_x, gain_cross),
                    },
                )
            })
            .collect();
        Self {
            settings,
            level_height,
            columns,
        }
    }

    /// Quantized water level. A level at/below the solid bed is dry, never clamped up.
    #[must_use]
    pub fn height(&self, coord: HexCoord, seconds: f32, column: OceanWaterColumn) -> f32 {
        let p = coord.to_world(0.0);
        let fallback = WaveColumn {
            water: column,
            travel: Vec2::new(p.x, p.x * 0.5 + p.z * 0.866_025_4),
            gain: Vec2::ONE,
        };
        let profile = self.columns.get(&coord).unwrap_or(&fallback);
        let phase = self.settings.phase(seconds);
        let (amplitude, wavelength, period) = wave_parameters(self.settings.wave);
        let primary =
            (std::f32::consts::TAU * (profile.travel.x / wavelength - phase / period)).sin();
        let cross = if self.settings.wave == LabWave::Crossing {
            0.4 * profile.gain.y
                * (std::f32::consts::TAU * (profile.travel.y / 18.0 - phase / 6.0)).sin()
        } else {
            0.0
        };
        let displacement = amplitude * profile.gain.x * primary + cross;
        column.mean_height + (displacement / self.level_height).round() * self.level_height
    }
}

fn wave_parameters(wave: LabWave) -> (f32, f32, f32) {
    match wave {
        LabWave::Flat => (0.0, 28.0, 9.0),
        LabWave::Gentle => (0.3, 20.0, 6.0),
        LabWave::Regular | LabWave::Crossing => (0.9, 28.0, 9.0),
        LabWave::Swell => (1.5, 44.0, 12.0),
        LabWave::Extreme => (4.8, 52.0, 18.0),
    }
}

fn propagation(
    at: Vec2,
    direction: Vec2,
    water: OceanWaterColumn,
    beds: &BTreeMap<HexCoord, OceanWaterColumn>,
) -> (f32, f32) {
    let depth = water.mean_height - water.bed_height;
    let shallow = (1.0 - depth / 6.0).clamp(0.0, 1.0);
    let mut travel = direction.dot(at);
    let mut loss = 0.0_f32;
    // Integrate fixed spatial paths once per terrain publication, not each frame.
    // Increased travel distance gives shorter wavelengths and lower phase speed.
    let mut distance = 1.0;
    while distance < direction.dot(at) + 70.0 {
        let upstream = at - direction * distance;
        let coord = HexCoord::from_world(Vec3::new(upstream.x, 0.0, upstream.y));
        if let Some(bed) = beds.get(&coord) {
            let d = bed.mean_height - bed.bed_height;
            travel += 2.0 * 0.7 * (1.0 - d / 6.0).clamp(0.0, 1.0);
            loss += 2.0 * (3.0 - d).max(0.0) * if d < -1.0 { 0.14 } else { 0.016 };
        }
        distance += 2.0;
    }
    let mut gain = (0.8 + 0.85 * shallow) * (-loss).exp();
    if (at.y - 3.0).abs() < 5.2 && (-18.0..18.0).contains(&at.x) {
        let progress = ((at.x + 16.0) / 32.0).clamp(0.0, 1.0);
        let bank = ((at.y - 3.0).abs() / 5.2).powi(2);
        gain *= (1.0 - progress * progress).powi(2) * (-progress * bank * 2.0).exp();
        travel += progress * progress * (3.0 + 8.0 * bank);
    }
    (travel, gain)
}

impl OceanEnvironmentSampler for LabSurface {
    fn inundation_column_at(&self, at: Vec2) -> Option<OceanWaterColumn> {
        if !at.is_finite() {
            return None;
        }
        self.columns
            .get(&HexCoord::from_world(Vec3::new(at.x, 0.0, at.y)))
            .map(|column| column.water)
    }

    fn surface_at(
        &self,
        at: Vec2,
        seconds: f32,
        column: OceanWaterColumn,
    ) -> Option<OceanSurfaceSample> {
        let coord = HexCoord::from_world(Vec3::new(at.x, 0.0, at.y));
        if !contains(coord) || !at.is_finite() || !seconds.is_finite() || self.level_height <= 0.0 {
            return None;
        }
        Some(OceanSurfaceSample {
            height: self.height(coord, seconds, column),
            normal: Vec3::Y,
            vertical_velocity: 0.0,
            mean_height: column.mean_height,
            bed_height: column.bed_height,
            water_id: column.water_id,
        })
    }

    fn wind_at(
        &self,
        position: Vec3,
        time: OceanSimulationTime,
        _profile: OceanWindProfile,
    ) -> Vec2 {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "Bounded periodic wind is evaluated in world f32 units."
        )]
        let t = time.seconds.rem_euclid(900.0) as f32;
        let changing = matches!(self.settings.wind, LabWind::Turning | LabWind::Shelter);
        let heading = if changing {
            20_f32.to_radians() * (t * std::f32::consts::TAU / 60.0).sin()
        } else {
            0.0
        };
        let speed = match self.settings.wind {
            LabWind::Calm => 0.0,
            LabWind::Steady => 9.0,
            LabWind::Strong => 20.0,
            _ => 9.0 + 3.0 * (t * std::f32::consts::TAU / 12.0).sin(),
        };
        let shelter = if self.settings.wind == LabWind::Shelter {
            let distance = (Vec2::new(position.x, position.z) - Vec2::new(11.0, -6.0)).length();
            let blend = ((distance - 4.0) / 10.0).clamp(0.0, 1.0);
            0.45 + 0.55 * blend * blend * (3.0 - 2.0 * blend)
        } else {
            1.0
        };
        Vec2::new(heading.cos(), heading.sin()) * speed * shelter
    }
}

/// Integration publishes the completed simulation clock for this finite renderer.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct WaterLabFrame {
    /// Render only the selected lab world.
    pub enabled: bool,
    /// Completed simulation phase, identical to surface queries.
    pub seconds: f32,
}

/// Install finite opaque water rendering without a second terrain authority.
pub fn install(app: &mut App) {
    app.init_resource::<WaterLabFrame>()
        .init_resource::<WaterLabSettings>();
    render::install(app);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn seven_regions_are_bounded_connected_and_exactly_3283_columns() {
        let cells: BTreeSet<_> = CENTERS
            .into_iter()
            .flat_map(|(q, r)| HexCoord::from_axial(q, r).within_radius(12))
            .collect();
        assert_eq!(cells.len(), 3283);
        assert!(cells.iter().all(|coord| contains(*coord)));
        let mut reached = BTreeSet::from([HexCoord::ORIGIN]);
        let mut pending = vec![HexCoord::ORIGIN];
        while let Some(coord) = pending.pop() {
            for neighbour in coord.within_radius(1) {
                if cells.contains(&neighbour) && reached.insert(neighbour) {
                    pending.push(neighbour);
                }
            }
        }
        assert_eq!(reached, cells);
        assert!(!contains(HexCoord::from_axial(38, 0)));
    }
    #[test]
    fn stepped_sampler_is_hex_constant_frozen_and_never_below_bed() {
        let column = OceanWaterColumn {
            mean_height: SEA_LEVEL,
            bed_height: 1.0,
            water_id: hex_core::SubstanceId(3),
        };
        let mut sampler = LabSurface {
            settings: default(),
            level_height: 0.4,
            columns: BTreeMap::new(),
        };
        for time in [0.0, 0.1, 1.5, 5.9, 6.0, 899.99] {
            let a = sampler
                .surface_at(Vec2::new(24.0, 0.0), time, column)
                .unwrap();
            let b = sampler
                .surface_at(Vec2::new(24.1, 0.1), time, column)
                .unwrap();
            assert!((a.height - b.height).abs() < 0.00001);
            let levels = (a.height - SEA_LEVEL) / 0.4;
            assert!((levels - levels.round()).abs() < 0.00001);
            assert!(a.height > column.bed_height);
            assert_eq!(a.normal, Vec3::Y);
            assert!(a.vertical_velocity.abs() < f32::EPSILON);
        }
        sampler.settings.frozen_phase = Some(1.5);
        assert_eq!(
            sampler.surface_at(Vec2::new(24.0, 0.0), 0.0, column),
            sampler.surface_at(Vec2::new(24.0, 0.0), 700.0, column)
        );
    }
    #[test]
    fn local_wind_presets_share_exact_speed_and_blend_shelter() {
        let mut sampler = LabSurface {
            settings: default(),
            level_height: 0.4,
            columns: BTreeMap::new(),
        };
        let time = OceanSimulationTime::default();
        assert!((sampler.wind_at(Vec3::ZERO, time, default()).length() - 9.0).abs() < 0.00001);
        sampler.settings.wind = LabWind::Shelter;
        let sheltered = sampler
            .wind_at(Vec3::new(11.0, 8.0, -6.0), time, default())
            .length();
        let exposed = sampler
            .wind_at(Vec3::new(40.0, 8.0, 0.0), time, default())
            .length();
        assert!((sheltered / exposed - 0.45).abs() < 0.00001);
    }
}

#[cfg(test)]
mod revision_tests {
    use super::*;
    use hex_core::arena::ArenaAvailability;
    use hex_core::ocean::{OceanEnvironmentView, OceanSurfaceState};
    use std::sync::Arc;

    fn fixture() -> LabSurface {
        let beds = CENTERS
            .into_iter()
            .flat_map(|(q, r)| HexCoord::from_axial(q, r).within_radius(12))
            .map(|coord| {
                let at = coord.to_world(0.0);
                (
                    coord,
                    OceanWaterColumn {
                        mean_height: SEA_LEVEL,
                        bed_height: bed_level(Vec2::new(at.x, at.z)) as f32 * 0.4,
                        water_id: hex_core::SubstanceId(3),
                    },
                )
            })
            .collect();
        LabSurface::from_beds(beds, 0.4, default())
    }
    #[test]
    fn real_beach_alternates_wet_and_dry_and_presets_span_gentle_to_extreme() {
        let lab = fixture();
        let env = OceanEnvironmentView {
            package_fingerprint: 1,
            sampler: Arc::new(lab.clone()),
            wind: default(),
        };
        let mut alternating = 0;
        for (coord, profile) in &lab.columns {
            if !(8.0..=8.8).contains(&profile.water.bed_height) {
                continue;
            }
            let p = coord.to_world(0.0);
            let mut wet = false;
            let mut dry = false;
            for tick in 0..64 {
                let time = OceanSimulationTime {
                    seconds: f64::from(tick) * 0.125,
                    ..default()
                };
                match env.sample(Vec2::new(p.x, p.z), time, ArenaAvailability::Ready, None) {
                    OceanSurfaceState::ReadyWet(_) => wet = true,
                    OceanSurfaceState::ReadyDry => dry = true,
                    state => panic!("Unexpected shoreline state: {state:?}"),
                }
            }
            alternating += usize::from(wet && dry);
        }
        assert!(
            alternating >= 12,
            "only {alternating} columns wash and reveal"
        );
        let coord = HexCoord::from_world(Vec3::new(-32.0, 0.0, -12.0));
        let column = lab.columns.get(&coord).unwrap().water;
        for (wave, min_range, max_range) in [
            (LabWave::Flat, 0.0, 0.001),
            (LabWave::Gentle, 0.4, 0.81),
            (LabWave::Extreme, 7.0, 16.0),
        ] {
            let mut sampled = lab.clone();
            sampled.settings.wave = wave;
            let heights: Vec<_> = (0..128)
                .map(|i| sampled.height(coord, i as f32 * 0.125, column))
                .collect();
            let range = heights.iter().copied().fold(f32::NEG_INFINITY, f32::max)
                - heights.iter().copied().fold(f32::INFINITY, f32::min);
            assert!(
                (min_range..=max_range).contains(&range),
                "{wave:?}: {range}"
            );
        }
    }
    #[test]
    fn channel_front_spans_multiple_columns_and_dissipates_toward_end_and_banks() {
        let lab = fixture();
        let probe = |x, z| {
            *lab.columns
                .get(&HexCoord::from_world(Vec3::new(x, 0.0, z)))
                .unwrap()
        };
        let inlet = probe(-14.0, 3.0);
        let middle = probe(-3.0, 3.0);
        let end = probe(14.0, 3.0);
        assert!(inlet.gain.x > 0.7, "entry {}", inlet.gain.x);
        assert!(inlet.gain.x > middle.gain.x && middle.gain.x > end.gain.x);
        assert!(end.gain.x < 0.05);
        assert!(probe(-3.0, 6.0).gain.x < middle.gain.x);
        let offshore_spacing = probe(-28.0, 3.0).travel.x - probe(-32.0, 3.0).travel.x;
        let bank_spacing = probe(-1.0, 6.0).travel.x - probe(-5.0, 6.0).travel.x;
        assert!(bank_spacing > offshore_spacing);
        let rising = lab
            .columns
            .iter()
            .filter(|(coord, profile)| {
                let p = coord.to_world(0.0);
                (-15.0..-10.0).contains(&p.x)
                    && (p.z - 3.0).abs() < 4.0
                    && profile.gain.x * 0.9 > 0.4
            })
            .count();
        assert!(rising >= 5, "channel wave reduced to {rising} columns");
        let launch = HexCoord::from_axial(-2, -6).to_world(0.0);
        assert!(bed_level(Vec2::new(launch.x, launch.z)) as f32 * 0.4 >= SEA_LEVEL + 18.0);
    }
}
