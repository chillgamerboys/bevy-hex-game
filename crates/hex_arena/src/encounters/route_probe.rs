//! Test-support route diagnostics; admission checks match the boolean probe.

use super::*;

/// Exact failed controller state from an explicit synthetic route probe.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DryRouteProbeFailure {
    /// Failed admission check, not an inferred navigation cause.
    pub reason: &'static str,
    /// Zero-based waypoint being approached, or waypoint count at final settlement.
    pub waypoint_index: usize,
    /// Number of actual movement-controller ticks already simulated.
    pub ticks: usize,
    /// Current cloned body feet; absent if the requested actor was missing.
    pub feet: Option<[f32; 3]>,
    /// Feet immediately before the last simulated movement tick.
    pub before: Option<[f32; 3]>,
    /// Current requested waypoint, if one exists.
    pub target: Option<[f32; 3]>,
    /// Actual nearest ground found by the same body query within four units.
    pub ground: Option<[f32; 3]>,
    /// Complete body collision clearance at failure.
    pub clear: bool,
    /// Complete body liquid exclusion at failure.
    pub dry: bool,
    /// The movement controller's current grounded state.
    pub grounded: bool,
    /// Current vertical velocity of the cloned movement controller.
    pub vertical_velocity: Option<f32>,
}

impl ArenaSession {
    /// Clone an actor and continuously drive authored waypoints through the actual
    /// movement controller. At most 3,600 ticks and 32 waypoints; no live mutation.
    #[must_use]
    pub fn probe_dry_route(
        &self,
        id: ActorId,
        waypoints: &[Vec3],
        view: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
    ) -> bool {
        self.probe_dry_route_report(id, waypoints, view, geometry, tuning)
            .is_ok()
    }

    /// Run the same bounded controller probe and report its exact first failure.
    /// Diagnostics do not skip points, relax support, or change live actor state.
    pub fn probe_dry_route_report(
        &self,
        id: ActorId,
        waypoints: &[Vec3],
        view: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
    ) -> Result<(), DryRouteProbeFailure> {
        let actor = self.actors.iter().find(|actor| actor.id == id);
        let failure =
            |reason, waypoint_index, ticks, actor: Option<&Actor>, before| DryRouteProbeFailure {
                reason,
                waypoint_index,
                ticks,
                feet: actor.map(|actor| actor.feet.to_array()),
                before,
                target: waypoints.get(waypoint_index).map(|point| point.to_array()),
                ground: actor.and_then(|actor| {
                    shapes::ground(&self.collision, actor, actor.feet, 4.0)
                        .map(|feet| feet.to_array())
                }),
                clear: actor.is_some_and(|actor| {
                    shapes::clear(&self.collision, actor, actor.feet, actor.body_yaw)
                }),
                dry: actor.is_some_and(|actor| dry(actor, view, geometry)),
                grounded: actor.is_some_and(|actor| actor.grounded),
                vertical_velocity: actor.map(|actor| actor.body.vertical_velocity),
            };
        if waypoints.len() > 32 {
            return Err(failure("waypoint limit", 0, 0, actor, None));
        }
        let Some(mut actor) = actor.cloned() else {
            return Err(failure("actor missing", 0, 0, None, None));
        };
        let invalid_pose = |actor: &Actor, before: Vec3| {
            if !shapes::clear(&self.collision, actor, actor.feet, actor.body_yaw) {
                Some("body collision")
            } else if !dry(actor, view, geometry) {
                Some("liquid overlap")
            } else if actor.feet.y < before.y - 0.45 {
                Some("per-tick drop")
            } else if shapes::ground(&self.collision, actor, actor.feet, 0.45).is_none() {
                Some("support beyond 0.45")
            } else {
                None
            }
        };
        let mut ticks = 0;
        let mut before = actor.feet;
        for (index, point) in waypoints.iter().enumerate() {
            while actor.feet.with_y(0.0).distance(point.with_y(0.0)) > 0.3 {
                if ticks >= 3600 {
                    return Err(failure(
                        "tick limit",
                        index,
                        ticks,
                        Some(&actor),
                        Some(before.to_array()),
                    ));
                }
                before = actor.feet;
                let direction = (point - actor.feet).with_y(0.0).normalize_or_zero();
                motion::tick(
                    &mut actor,
                    direction,
                    true,
                    false,
                    false,
                    &self.collision,
                    &tuning.encounters,
                );
                ticks += 1;
                if let Some(reason) = invalid_pose(&actor, before) {
                    return Err(failure(
                        reason,
                        index,
                        ticks,
                        Some(&actor),
                        Some(before.to_array()),
                    ));
                }
            }
            if (actor.feet.y - point.y).abs() > 0.45 {
                return Err(failure(
                    "waypoint altitude",
                    index,
                    ticks,
                    Some(&actor),
                    Some(before.to_array()),
                ));
            }
        }
        // A bounded slice can finish over a descending tread before gravity
        // reaches its floor. Hold input, advance the actual controller, and keep
        // every physical check and the same total tick budget while it lands.
        while !actor.grounded || shapes::ground(&self.collision, &actor, actor.feet, 0.05).is_none()
        {
            if ticks >= 3600 {
                return Err(failure(
                    "tick limit during final settlement",
                    waypoints.len(),
                    ticks,
                    Some(&actor),
                    Some(before.to_array()),
                ));
            }
            before = actor.feet;
            motion::tick(
                &mut actor,
                Vec3::ZERO,
                true,
                false,
                false,
                &self.collision,
                &tuning.encounters,
            );
            ticks += 1;
            if let Some(reason) = invalid_pose(&actor, before) {
                return Err(failure(
                    reason,
                    waypoints.len(),
                    ticks,
                    Some(&actor),
                    Some(before.to_array()),
                ));
            }
            if waypoints.last().is_some_and(|point| {
                actor.feet.with_y(0.0).distance(point.with_y(0.0)) > 0.3
                    || (actor.feet.y - point.y).abs() > 0.45
            }) {
                return Err(failure(
                    "left final waypoint during settlement",
                    waypoints.len(),
                    ticks,
                    Some(&actor),
                    Some(before.to_array()),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::SubstanceId;

    fn descending_tread(
        species: Species,
    ) -> (
        ArenaSession,
        ArenaTerrainView,
        ArenaVoxelGeometry,
        Vec<Vec3>,
    ) {
        let geometry = ArenaVoxelGeometry {
            level_height: 0.35,
            ..Default::default()
        };
        let mut view = ArenaTerrainView::default();
        for coord in HexCoord::ORIGIN.within_radius(4) {
            let level = i32::from(coord.x <= 0);
            view.voxels
                .insert(TilePos::new(coord, level), SubstanceId(1));
        }
        let start = HexCoord::ORIGIN.to_world(0.35 + SKIN);
        let end = HexCoord::from_axial(1, 0).to_world(SKIN);
        let mut actor = Actor::spawn(0, start, Vec3::X);
        actor.configure_species(species, &ArenaTuning::default().encounters);
        actor.body.grounded = true;
        actor.grounded = true;
        let mut session = ArenaSession {
            actors: vec![actor],
            ..Default::default()
        };
        session.collision.refresh(&view, geometry);
        (session, view, geometry, vec![start, end])
    }

    #[test]
    fn downhill_endpoint_waits_for_actual_neutral_controller_landing() {
        for species in [Species::Human, Species::Goblin, Species::Shaman] {
            let (session, view, geometry, points) = descending_tread(species);
            let tuning = ArenaTuning::default();
            let mut moving = session.actors.first().expect("actor").clone();
            let target = *points.last().expect("target");
            for _ in 0..120 {
                if moving.feet.with_y(0.0).distance(target.with_y(0.0)) <= 0.3 {
                    break;
                }
                motion::tick(
                    &mut moving,
                    Vec3::X,
                    true,
                    false,
                    false,
                    &session.collision,
                    &tuning.encounters,
                );
            }
            assert!(moving.feet.with_y(0.0).distance(target.with_y(0.0)) <= 0.3);
            assert!(!moving.grounded);
            assert!(shapes::ground(&session.collision, &moving, moving.feet, 0.45).is_some());
            assert!(
                shapes::ground(&session.collision, &moving, moving.feet, 0.05).is_none(),
                "fixture must reproduce the admitted but not-yet-landed endpoint"
            );
            let result = session.probe_dry_route_report(0, &points, &view, geometry, &tuning);
            assert!(
                result.is_ok(),
                "neutral landing failed for {species:?}: {result:?}"
            );
            assert_eq!(
                session.actors.first().expect("live actor").feet,
                *points.first().expect("start")
            );
        }
    }

    #[test]
    fn endpoint_settlement_does_not_accept_missing_support_or_excess_waypoints() {
        let (mut session, mut view, geometry, points) = descending_tread(Species::Human);
        view.voxels.retain(|pos, _| pos.coord.x <= 0);
        view.revision += 1;
        session.collision.refresh(&view, geometry);
        let result =
            session.probe_dry_route_report(0, &points, &view, geometry, &ArenaTuning::default());
        assert_eq!(
            result.expect_err("gap must remain invalid").reason,
            "support beyond 0.45"
        );
        let long = vec![*points.first().expect("start"); 33];
        assert_eq!(
            session
                .probe_dry_route_report(0, &long, &view, geometry, &ArenaTuning::default())
                .expect_err("limit is unchanged")
                .reason,
            "waypoint limit"
        );
    }
}
