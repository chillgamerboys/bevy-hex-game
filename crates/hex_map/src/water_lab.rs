//! Small built-in water fixture and its world-owned stepped surface.

mod render;

use std::collections::{BTreeMap, BTreeSet};

use crate::{Column, VoxelMap};
use bevy::prelude::*;
use hex_assets::SubstanceTable;
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
    let channel = (at.y - 3.0).abs() - 1.8;
    let land = island.min(cove).min(channel);
    let headland = if land > 2.0 {
        (1.0 - (at - Vec2::new(-7.0, -7.0)).length() / 5.0).max(0.0) * 12.0
    } else {
        0.0
    };
    (SEA_VOXEL + (land * 0.8 + headland).floor() as i32).clamp(2, 40)
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
    let shore_coord = HexCoord::from_axial(-7, 5);
    let shore = shore_coord.to_world(geometry.top(TilePos::new(
        shore_coord,
        map.surface(shore_coord).ok_or("Missing lab beach")?,
    )));
    let water_start = HexCoord::from_axial(12, 0).to_world(SEA_LEVEL);
    let anchors = BTreeMap::from([
        ("party_start".into(), shore),
        ("hostile_start".into(), shore + Vec3::X * 2.0),
        ("player_look_at".into(), water_start),
        ("water_lab_shore".into(), shore),
        ("water_lab_swim".into(), water_start),
        (
            "water_lab_glider".into(),
            HexCoord::from_axial(0, 18).to_world(SEA_LEVEL + 20.0),
        ),
    ]);
    Ok((map, geometry, anchors, default()))
}

/// Immutable snapshot shared by contact queries and the finite water renderer.
#[derive(Debug, Clone, Copy)]
pub struct LabSurface {
    /// Explicit comparison choices and wave clock.
    pub settings: WaterLabSettings,
    /// Height of one whole voxel, obtained from world geometry.
    pub level_height: f32,
}

impl LabSurface {
    /// Quantized top of the exact confirmed-wet hex column.
    #[must_use]
    pub fn height(self, coord: HexCoord, seconds: f32, column: OceanWaterColumn) -> f32 {
        let p = coord.to_world(0.0);
        let at = Vec2::new(p.x, p.z);
        let phase = self.settings.phase(seconds);
        let wave = |direction: Vec2, amplitude: f32, wavelength: f32, period: f32| {
            amplitude
                * (std::f32::consts::TAU * (direction.dot(at) / wavelength - phase / period)).sin()
        };
        let displacement = match self.settings.wave {
            LabWave::Flat => 0.0,
            LabWave::Regular => wave(Vec2::X, 0.8, 18.0, 6.0),
            LabWave::Swell => wave(Vec2::X, 1.2, 36.0, 12.0),
            LabWave::Crossing => {
                wave(Vec2::X, 0.8, 18.0, 6.0) + wave(Vec2::new(0.5, 0.866_025_4), 0.4, 12.0, 4.0)
            }
        };
        let depth = column.mean_height - column.bed_height;
        let shallow = (depth / 3.2).clamp(0.0, 1.0);
        let cove = ((at - Vec2::new(11.0, -6.0)).length() / 8.0).clamp(0.15, 1.0);
        let height = column.mean_height
            + (displacement * shallow * cove / self.level_height).round() * self.level_height;
        height.max(column.bed_height + self.level_height)
    }
}

impl OceanEnvironmentSampler for LabSurface {
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
            // Whole-voxel jumps are position changes, never launch impulses.
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
