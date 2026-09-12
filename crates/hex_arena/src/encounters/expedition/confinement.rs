//! Actor-specific physical arena boundary, derived from the admitted site.

use super::*;
use crate::hex_prisms::HexPrism;
use hex_core::arena::ArenaDeploymentRegion;

#[derive(Debug)]
pub(in crate::encounters) struct ShadowArena {
    actor: ActorId,
    region: ArenaDeploymentRegion,
    columns: BTreeSet<HexCoord>,
    boundary: Vec<HexPrism>,
    accepted: Vec3,
}

impl ShadowArena {
    pub(super) fn new(sites: &ArenaExpeditionSites, actors: &[Actor]) -> Option<Self> {
        let actor = actors
            .iter()
            .find(|a| a.expedition_role() == Some(ExpeditionRole::MountainShadow))?;
        let region = sites.encounters.get("mountain_shadow")?.deployment.clone();
        let columns: BTreeSet<_> = region.surfaces.iter().map(|p| p.coord).collect();
        let outside: BTreeSet<_> = columns
            .iter()
            .flat_map(|coord| coord.within_radius(1))
            .filter(|coord| !columns.contains(coord))
            .collect();
        let boundary = outside
            .into_iter()
            .filter_map(|coord| HexPrism::new(coord.to_world(-2.0), 4.0))
            .collect();
        Some(Self {
            actor: actor.id,
            region,
            columns,
            boundary,
            accepted: actor.feet,
        })
    }

    fn contains(&self, actor: &Actor, feet: Vec3) -> bool {
        feet.is_finite()
            && self.columns.contains(&HexCoord::from_world(feet))
            && self
                .boundary
                .iter()
                .all(|hex| hex.distance(feet.with_y(0.0)) >= actor.dimensions.x * 0.5)
    }

    pub(in crate::encounters) fn constrain(
        &mut self,
        actor: &mut Actor,
        collision: &CollisionWorld,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
    ) {
        if actor.id != self.actor || actor.hp <= 0.0 {
            return;
        }
        let proposed = actor.feet;
        let from = self.accepted;
        let delta = (proposed - from).with_y(0.0);
        let radius = actor.dimensions.x * 0.5;
        let mut fraction = 1.0_f32;
        if proposed.is_finite() && self.contains(actor, from) {
            for hex in &self.boundary {
                if let Some(hit) = hex.sweep_sphere(from.with_y(0.0), delta, radius + SKIN) {
                    if hit.normal.dot(delta) < 0.0 {
                        fraction = fraction.min(hit.fraction);
                    }
                }
            }
        } else {
            fraction = 0.0;
        }
        if fraction < 1.0 {
            let retreat = SKIN / delta.length().max(SKIN);
            actor.feet = (from + delta * (fraction - retreat).max(0.0)).with_y(proposed.y);
            actor.body.impulse_velocity.x = 0.0;
            actor.body.impulse_velocity.z = 0.0;
        }
        if !self.contains(actor, actor.feet)
            || !shapes::clear(collision, actor, actor.feet, actor.body_yaw)
        {
            actor.feet = from;
            if !self.contains(actor, from) || !shapes::clear(collision, actor, from, actor.body_yaw)
            {
                // A newly published Shield can invalidate the previous position.
                // Recovery stays inside the same admitted region and uses the
                // production support/body checks, never the world outside the gate.
                if let Some(feet) = battle_runtime::deployment_pose(
                    actor,
                    &self.region,
                    &[],
                    collision,
                    world,
                    geometry,
                )
                .filter(|feet| self.contains(actor, *feet))
                {
                    actor.feet = feet;
                }
            }
            actor.body.impulse_velocity = Vec3::ZERO;
            actor.body.vertical_velocity = 0.0;
        }
        if self.contains(actor, actor.feet)
            && shapes::clear(collision, actor, actor.feet, actor.body_yaw)
        {
            self.accepted = actor.feet;
        }
    }
}

impl ArenaSession {
    pub(in crate::encounters) fn confine_shadow(
        &mut self,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
    ) {
        let Some(arena) = self.encounter.shadow_arena.as_mut() else {
            return;
        };
        if let Some(actor) = self.actors.iter_mut().find(|a| a.id == arena.actor) {
            arena.constrain(actor, &self.collision, world, geometry);
        }
    }
}
