//! Bounded death recovery against admitted collision and marine facts.
use super::{GrandState, RespawnStage};
use crate::{encounters, shapes, Actor, ArenaSession, SKIN};
use bevy_math::Vec3;
use hex_core::arena::{ArenaAvailability, ArenaTerrainView, ArenaVoxelGeometry};
use hex_core::{HexCoord, TilePos};

// The world guarantees a 16-hex neighborhood for every actor interest. A
// 12-hex search plus a two-hex body/query halo stays inside that publication.
const SEARCHES: [(u32, f32); 2] = [(6, 4.0), (12, 24.0)];

enum Ground {
    Waiting,
    Safe(Vec3),
    Unusable,
}

impl ArenaSession {
    pub(super) fn recover_grand_player(
        &mut self,
        grand: &mut GrandState,
        player_index: usize,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
    ) -> bool {
        if grand.respawn_stage == RespawnStage::Failed {
            self.notice = "No safe ground remains near your shrine or starting beach. Progress is retained; start a New Run to play again.".into();
            return false;
        }
        let desired = match grand.respawn_stage {
            RespawnStage::Shrine => grand
                .last_shrine
                .and_then(|s| world.anchors.get(s.anchor()).copied())
                .unwrap_or(grand.start),
            RespawnStage::Start | RespawnStage::Failed => grand.start,
        };
        grand.respawn_interest = Some(desired);
        let Some(player) = self.actors.get(player_index) else {
            return false;
        };
        match self.grand_respawn_ground(player, desired, world, geometry) {
            Ground::Waiting => {
                self.notice = "Loading safe ground for your return…".into();
                false
            }
            Ground::Unusable => {
                if grand.respawn_stage == RespawnStage::Shrine && grand.last_shrine.is_some() {
                    grand.respawn_stage = RespawnStage::Start;
                    grand.respawn_interest = Some(grand.start);
                    self.notice =
                        "Your shrine has no safe ground nearby. Returning to the starting beach…"
                            .into();
                } else {
                    grand.respawn_stage = RespawnStage::Failed;
                    self.notice = "No safe ground remains near your shrine or starting beach. Progress is retained; start a New Run to play again.".into();
                }
                false
            }
            Ground::Safe(feet) => {
                let max_hp = player.max_hp;
                let mut recovered = Actor::spawn(0, feet, player.aim);
                recovered.configure_expedition_player();
                recovered.max_hp = max_hp;
                recovered.hp = max_hp;
                recovered.free_flight = None;
                recovered.marine = Some(Default::default());
                if let Some(marine) = &mut recovered.marine {
                    marine.lab = true;
                    marine.glider_wind_scale = 0.65;
                }
                if let Some(slot) = self.actors.get_mut(player_index) {
                    *slot = recovered;
                }
                self.notice = if grand.respawn_stage == RespawnStage::Start || grand.last_shrine.is_none() {
                    "Returned to the starting beach. Progress and the changed world are retained."
                } else {
                    "Returned to safe ground near your last shrine. Progress and the changed world are retained."
                }.into();
                grand.respawn_interest = None;
                grand.respawn_stage = RespawnStage::Shrine;
                grand.deaths = grand.deaths.saturating_add(1);
                self.outcome = None;
                true
            }
        }
    }

    fn grand_respawn_ground(
        &self,
        actor: &Actor,
        desired: Vec3,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
    ) -> Ground {
        // The compact publication omits the implicit ocean; do not admit its
        // seabed before the authoritative water sampler is installed.
        if world.residency.is_some() && self.ocean_environment.is_none() {
            return Ground::Waiting;
        }
        let origin = HexCoord::from_world(desired);
        for (radius, max_height_delta) in SEARCHES {
            if world.residency.as_ref().is_some_and(|residency| {
                origin
                    .within_radius(radius + 2)
                    .into_iter()
                    .any(|coord| residency.at(coord, geometry) == ArenaAvailability::Unloaded)
            }) {
                return Ground::Waiting;
            }
            let mut candidates = Vec::new();
            for coord in origin.within_radius(radius) {
                if !geometry.contains_column(coord)
                    || world.residency.as_ref().is_some_and(|residency| {
                        residency.at(coord, geometry) != ArenaAvailability::Ready
                    })
                {
                    continue;
                }
                if let Some(runs) = world.columns.get(&coord) {
                    candidates.extend(runs.iter().map(|run| {
                        coord.to_world(geometry.top(TilePos::new(coord, run.top_level)) + SKIN)
                    }));
                } else {
                    candidates.extend(
                        world
                            .voxels
                            .range(TilePos::new(coord, i32::MIN)..=TilePos::new(coord, i32::MAX))
                            .map(|(pos, _)| coord.to_world(geometry.top(*pos) + SKIN)),
                    );
                }
            }
            candidates.retain(|feet| (feet.y - desired.y).abs() <= max_height_delta);
            candidates.sort_by(|a, b| {
                a.distance_squared(desired)
                    .total_cmp(&b.distance_squared(desired))
            });
            for feet in candidates {
                let mut candidate = actor.clone();
                candidate.feet = feet;
                if shapes::clear(&self.collision, &candidate, feet, candidate.body_yaw)
                    && shapes::ground(&self.collision, &candidate, feet, SKIN * 8.0).is_some()
                    && encounters::dry(&candidate, world, geometry)
                    && self.ocean_environment.as_ref().is_none_or(|environment| {
                        crate::marine::MarineWorld {
                            terrain: world,
                            geometry,
                            environment: Some(environment),
                            time: self.ocean_time(),
                        }
                        .dry_support(
                            feet,
                            candidate.dimensions.x.max(candidate.dimensions.z) * 0.5,
                        )
                    })
                    && self
                        .actors
                        .iter()
                        .filter(|other| other.hp > 0.0)
                        .all(|other| encounters::body_overlap(&candidate, other).is_none())
                {
                    return Ground::Safe(feet);
                }
            }
        }
        Ground::Unusable
    }
}
