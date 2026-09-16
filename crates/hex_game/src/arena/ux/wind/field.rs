//! Bounded, depth-tested display of the exact environmental wind queries.
use bevy::prelude::*;
use hex_arena::ArenaSession;
use hex_core::{
    arena::{ArenaTerrainView, ArenaVoxelGeometry},
    ocean::{OceanEnvironmentView, OceanSimulationTime},
    water_lab::WATER_LAB_ID,
};
use hex_map::water_lab::SEA_LEVEL;

use super::super::UxState;
use crate::arena::{ArenaCamera, ArenaFrame, ViewState};

#[derive(Default, Reflect, GizmoConfigGroup)]
struct WindGizmos;

#[derive(Clone)]
struct Sample {
    position: Vec3,
    previous: Vec2,
    velocity: Vec2,
}

#[derive(Resource, Default)]
pub(in crate::arena) struct FieldDisplay {
    enabled: bool,
    samples: Vec<Sample>,
    sampled_at: Option<OceanSimulationTime>,
    terrain_revision: u64,
    center: Vec2,
    height_origin: Option<f32>,
}

impl FieldDisplay {
    pub(in crate::arena) fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "enabled": self.enabled, "terrain_revision": self.terrain_revision,
            "simulation_time": self.sampled_at.map(|t| t.seconds),
            "generation": self.sampled_at.map(|t| t.generation),
            "center": self.center.to_array(), "sample_hz": 10,
            "speed_scale_u_s": [0,25], "height_origin": self.height_origin.unwrap_or(SEA_LEVEL), "layer_offsets": [2,14,30],
            "samples": self.samples.iter().map(|s| serde_json::json!({
                "position":s.position.to_array(), "velocity":s.velocity.to_array()
            })).collect::<Vec<_>>()
        })
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "Interpolation is bounded to [0,1]."
    )]
    fn blend(&self, time: OceanSimulationTime) -> f32 {
        self.sampled_at.map_or(1.0, |last| {
            ((time.seconds - last.seconds) / 0.1).clamp(0.0, 1.0) as f32
        })
    }

    fn refresh(
        &mut self,
        enabled: bool,
        center: Vec3,
        time: OceanSimulationTime,
        environment: &OceanEnvironmentView,
        view: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
    ) {
        self.enabled = enabled;
        if !enabled {
            self.samples.clear();
            self.sampled_at = None;
            return;
        }
        let center = (Vec2::new(center.x, center.z) / 8.0).round() * 8.0;
        let reset = self
            .sampled_at
            .is_none_or(|last| last.generation != time.generation || time.seconds < last.seconds);
        // Position changes wait for the next 10Hz refresh. Terrain edits and
        // generation changes must remove obsolete/inside-solid arrows immediately.
        if !reset
            && self.terrain_revision == view.revision
            && self
                .sampled_at
                .is_some_and(|last| time.seconds - last.seconds < 0.1 - 1e-8)
        {
            return;
        }
        let blend = self.blend(time);
        let mut samples = Vec::with_capacity(147);
        for height in [2.0, 14.0, 30.0] {
            for x in -3..=3_i16 {
                for z in -3..=3_i16 {
                    let position = Vec3::new(
                        center.x + f32::from(x) * 8.0,
                        self.height_origin.unwrap_or(SEA_LEVEL) + height,
                        center.y + f32::from(z) * 8.0,
                    );
                    if inside(position, view, geometry) {
                        continue;
                    }
                    let velocity = environment.wind_at(position, time);
                    let previous = if reset {
                        velocity
                    } else {
                        self.samples
                            .iter()
                            .find(|s| s.position == position)
                            .map_or(velocity, |s| s.previous.lerp(s.velocity, blend))
                    };
                    samples.push(Sample {
                        position,
                        previous,
                        velocity,
                    });
                }
            }
        }
        self.samples = samples;
        self.sampled_at = Some(time);
        self.terrain_revision = view.revision;
        self.center = center;
    }
}

fn inside(position: Vec3, view: &ArenaTerrainView, geometry: ArenaVoxelGeometry) -> bool {
    geometry.voxel_at(position).is_some_and(|pos| {
        view.solid_at(pos).is_some()
            || view.static_spans.iter().any(|span| {
                span.blocks_movement
                    && span.bottom.coord == pos.coord
                    && (span.bottom.level..=span.top_level).contains(&pos.level)
            })
    })
}

pub(in crate::arena) fn install(app: &mut App) {
    app.init_gizmo_group::<WindGizmos>();
    let mut store = app.world_mut().resource_mut::<GizmoConfigStore>();
    let (config, _) = store.config_mut::<WindGizmos>();
    config.depth_bias = 0.0;
    config.line.width = 2.5;
    app.init_resource::<FieldDisplay>().add_systems(
        Update,
        present
            .in_set(ArenaFrame::Present)
            .after(super::present)
            .after(crate::arena::water_lab::LabPresentation),
    );
}

#[expect(
    clippy::too_many_arguments,
    reason = "Display reads authoritative wind, terrain, camera and UI without owning simulation."
)]
fn present(
    ux: Res<UxState>,
    state: Res<ViewState>,
    session: Res<ArenaSession>,
    environment: Option<Res<OceanEnvironmentView>>,
    view: Res<ArenaTerrainView>,
    geometry: Res<ArenaVoxelGeometry>,
    ocean_profile: Res<hex_map::ocean::OceanSurfaceProfile>,
    cameras: Query<&Transform, With<ArenaCamera>>,
    mut display: ResMut<FieldDisplay>,
    mut gizmos: Gizmos<WindGizmos>,
) {
    let Some(environment) = environment.filter(|env| {
        env.package_fingerprint == WATER_LAB_ID
            || view.selection.map == hex_core::arena::ArenaMap::NorthernArchipelago
    }) else {
        display.enabled = false;
        display.samples.clear();
        display.sampled_at = None;
        return;
    };
    let center = if state.capture_view == "water-lab-overview" && state.capture.is_some() {
        Vec3::ZERO // Review camera sees the whole fixture from outside it.
    } else {
        cameras.single().map_or(Vec3::ZERO, |pose| pose.translation)
    };
    display.height_origin = Some(if environment.package_fingerprint == WATER_LAB_ID {
        SEA_LEVEL
    } else {
        ocean_profile
            .mean_sea_level
            .max((center.y / 12.0).floor() * 12.0 - 14.0)
    });
    let time = session.ocean_time();
    display.refresh(
        ux.wind_visible && state.started,
        center,
        time,
        &environment,
        &view,
        *geometry,
    );
    let blend = display.blend(time);
    for sample in &display.samples {
        let velocity = sample.previous.lerp(sample.velocity, blend);
        let speed = velocity.length();
        if speed < 0.05 {
            continue;
        }
        let fraction = (speed / 25.0).clamp(0.0, 1.0);
        let color = Color::srgb(0.05 + 0.95 * fraction, 0.9, 1.0 - 0.95 * fraction);
        let direction = Vec3::new(velocity.x, 0.0, velocity.y).normalize_or_zero();
        let length = 0.35 + 5.0 * fraction;
        let end = sample.position + direction * length;
        let wing = direction.cross(Vec3::Y) * length * 0.18;
        let neck = end - direction * length * 0.28;
        gizmos.line(sample.position, end, color);
        gizmos.line(end, neck + wing, color);
        gizmos.line(end, neck - wing, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::ocean::{
        OceanEnvironmentSampler, OceanSurfaceSample, OceanWaterColumn, OceanWindProfile,
    };
    use hex_core::{arena::ArenaStaticSpan, HexCoord, TilePos};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[derive(Debug)]
    struct Counter(Arc<AtomicUsize>);
    impl OceanEnvironmentSampler for Counter {
        fn wind_at(&self, p: Vec3, _: OceanSimulationTime, _: OceanWindProfile) -> Vec2 {
            self.0.fetch_add(1, Ordering::Relaxed);
            Vec2::new(p.y * 0.1, p.x * 0.1)
        }
        fn surface_at(&self, _: Vec2, _: f32, _: OceanWaterColumn) -> Option<OceanSurfaceSample> {
            None
        }
    }
    #[test]
    fn hidden_is_free_samples_are_shared_bounded_and_pause_stops_refresh() {
        let count = Arc::new(AtomicUsize::new(0));
        let env = OceanEnvironmentView {
            package_fingerprint: WATER_LAB_ID,
            sampler: Arc::new(Counter(count.clone())),
            wind: OceanWindProfile::default(),
        };
        let mut display = FieldDisplay::default();
        let mut view = ArenaTerrainView::default();
        let geometry = ArenaVoxelGeometry::default();
        let mut time = OceanSimulationTime::default();
        display.refresh(false, Vec3::ZERO, time, &env, &view, geometry);
        assert_eq!(count.load(Ordering::Relaxed), 0);
        display.refresh(true, Vec3::ZERO, time, &env, &view, geometry);
        assert_eq!(display.samples.len(), 147);
        for s in &display.samples {
            assert_eq!(
                s.velocity,
                Vec2::new(s.position.y * 0.1, s.position.x * 0.1)
            );
        }
        for seconds in [0.0, 0.01, 0.099] {
            time.seconds = seconds;
            display.refresh(true, Vec3::ZERO, time, &env, &view, geometry);
        }
        assert_eq!(count.load(Ordering::Relaxed), 147);
        time.seconds = 0.1;
        display.refresh(true, Vec3::ZERO, time, &env, &view, geometry);
        assert_eq!(count.load(Ordering::Relaxed), 294);
        view.static_spans.push(ArenaStaticSpan {
            bottom: TilePos::new(HexCoord::ORIGIN, 20),
            top_level: 30,
            blocks_movement: true,
            blocks_sight: true,
            blocks_projectiles: true,
        });
        view.revision += 1;
        display.refresh(true, Vec3::ZERO, time, &env, &view, geometry);
        assert_eq!(display.samples.len(), 146);
        assert!(!display
            .samples
            .iter()
            .any(|s| s.position == Vec3::new(0.0, 10.0, 0.0)));
        let before = count.load(Ordering::Relaxed);
        display.refresh(false, Vec3::ZERO, time, &env, &view, geometry);
        assert_eq!(count.load(Ordering::Relaxed), before);
        assert!(display.samples.is_empty());
    }
}
