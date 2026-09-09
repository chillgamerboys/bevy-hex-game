//! Golem-only direct observations and stationary attack admission.

use super::*;

impl Brain {
    pub(super) fn golem_sight(
        &mut self,
        actor: &Actor,
        party: &PartyRuntime,
        actors: &[Actor],
        collision: &CollisionWorld,
        tuning: &ArenaTuning,
        tick: u64,
    ) -> Option<Knowledge> {
        self.golem_observation = None;
        if party.snapshot.phase == PartyPhase::Dormant {
            return None;
        }
        if self
            .battle_sense_tick
            .is_none_or(|previous| tick.saturating_sub(previous) >= 12)
        {
            self.battle_seen = targeting::observe(
                actor,
                actors,
                &self.battle_seen,
                collision,
                tick,
                tuning.bot.prediction_seconds,
            );
            self.battle_sense_tick = Some(tick);
        }
        // A running laser never switches to another creature's position or
        // velocity. Only a new cast may choose a different visible opponent.
        let target = self.active.as_ref().and_then(|cast| cast.laser_target());
        let observed = self.battle_seen.iter().copied().find(|seen| {
            tick.saturating_sub(seen.tick) < 12
                && target.is_none_or(|id| seen.body.id == id)
                && (party.battle_search.is_some() || seen.body.id == 0)
                && (party.snapshot.phase != PartyPhase::Returning
                    || seen.center().distance(actor.eye()) <= 6.0)
                && collision.sight_clear(actor.eye(), seen.sight_point)
        });
        self.golem_observation = observed;
        observed.map(|sample| Knowledge {
            point: sample.body.feet,
            velocity: sample.body.velocity,
            tick: sample.tick,
            direct: true,
            cue_kind: None,
            observed: Some(sample),
        })
    }

    pub(in crate::encounters) fn golem_observation(&self) -> Option<targeting::ObservedTarget> {
        self.golem_observation
    }

    pub(super) fn golem_intent(
        &mut self,
        actor: &Actor,
        party: &PartyRuntime,
        sight: Option<Knowledge>,
        goal: Vec3,
        collision: &CollisionWorld,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
        tick: u64,
        input: &mut ActorIntent,
    ) -> Option<Request> {
        let c = &tuning.encounters;
        if self
            .active
            .as_ref()
            .is_some_and(|cast| cast.preparing_laser())
            && sight.is_none()
        {
            // Full windup still requires own sight. Once firing, the finite cast
            // continues from its last observed velocity instead of party memory.
            self.active = None;
        }
        let desired = (goal - actor.feet).with_y(0.0);
        if actor.grounded
            && matches!(
                party.snapshot.phase,
                PartyPhase::Active | PartyPhase::Returning
            )
            && desired.length() > 0.35
            && self.steering.blocked_ticks(tick) >= 48
            && tick.is_multiple_of(12)
            && self.ready(CreatureAbility::GolemSwipe)
            && super::super::abilities::golem_swipe_blocked(
                actor,
                desired.normalize_or_zero(),
                collision,
                world,
                geometry,
            )
        {
            input.aim = desired.normalize_or(actor.aim);
            return Some(Request {
                kind: CreatureAbility::GolemSwipe,
                aim: input.aim,
            });
        }
        let observed = sight?.observed?;
        let point = observed.sight_point;
        let distance = observed.distance(actor.center(), 0.0);
        let bearing = point - (actor.feet + Vec3::Y * 1.4);
        let mouth = shapes::golem_mouth(actor, bearing);
        input.aim = (point - mouth).normalize_or(actor.aim);
        if distance <= c.golem_slam_range && self.ready(CreatureAbility::GolemSlam) {
            Some(Request {
                kind: CreatureAbility::GolemSlam,
                aim: input.aim,
            })
        } else if distance >= c.golem_laser_min_range
            && self.ready(CreatureAbility::GolemLaser)
            && collision.sight_clear(shapes::golem_mouth(actor, input.aim), point)
        {
            Some(Request {
                kind: CreatureAbility::GolemLaser,
                aim: input.aim,
            })
        } else {
            None
        }
    }
}
