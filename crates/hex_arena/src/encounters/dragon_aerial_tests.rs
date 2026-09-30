//! Composed high-altitude target admission and production Dragon motion.
use super::*;

#[test]
fn expedition_dragon_takes_off_and_attacks_a_high_gliding_target() {
    let (mut actor, mut target, party, view, geometry, collision, tuning) = scene(Species::Dragon);
    actor.configure_expedition(ExpeditionRole::Dragon, &tuning.encounters);
    target.configure_expedition_player();
    target.feet.y = 25.0;
    target.glider.open = true;
    let mut brain = Brain::new(actor.id, actor.feet);
    let mut attacked = false;
    let mut highest = actor.feet.y;
    for tick in 1..1200 {
        let (intent, request) = brain.intent(
            &actor,
            &party,
            &[target.clone(), actor.clone()],
            &[],
            &[],
            &collision,
            &view,
            geometry,
            &tuning,
            tick,
        );
        actor.aim = intent.input.aim;
        assert!(brain.decision.as_ref().expect("decision").retreat_seconds <= 0.0);
        if let Some(request) = request.filter(|request| {
            matches!(
                request.kind,
                CreatureAbility::FireCone | CreatureAbility::Bite
            )
        }) {
            let (range, angle) = if request.kind == CreatureAbility::FireCone {
                (
                    tuning.encounters.breath_range,
                    tuning.encounters.breath_angle,
                )
            } else {
                (
                    tuning.encounters.bite_range + 0.2,
                    tuning.encounters.bite_angle,
                )
            };
            assert!(
                crate::shapes::distance(actor.eye(), &target) <= range + SKIN,
                "attack still requires real mouth-to-body reach"
            );
            assert!(
                (actor.body_rotation() * Vec3::NEG_Z)
                    .dot(request.aim.with_y(0.0).normalize_or(Vec3::NEG_Z))
                    >= (angle.to_radians() * 0.5).cos() - SKIN
            );
            assert!(collision.sight_clear(actor.eye(), target.center()));
            attacked = true;
            break;
        }
        motion::tick_with_lunge(
            &mut actor,
            intent.direction,
            true,
            intent.input.jump,
            intent.flight,
            intent.lunge,
            &collision,
            &tuning.encounters,
        );
        highest = highest.max(actor.feet.y);
        assert!(steering::contained(&actor, geometry));
        assert!(crate::shapes::clear(
            &collision,
            &actor,
            actor.feet,
            actor.body_yaw
        ));
    }
    assert!(
        highest > 12.0,
        "pursuit must cross the old near-ground ceiling"
    );
    assert!(
        attacked,
        "feet={:?}; decision={:?}",
        actor.feet, brain.decision
    );
}

#[test]
fn aerial_goals_preserve_full_body_clearance_bounds_and_legacy_cruise() {
    let (mut actor, _, _, mut view, geometry, mut collision, tuning) = scene(Species::Dragon);
    let desired = Vec3::new(10.0, 25.0, 0.0);
    assert!(
        steering::pursuit_flight_goal(
            &actor,
            desired,
            &collision,
            &view,
            geometry,
            &tuning.encounters
        )
        .is_none(),
        "legacy Dragon retains its low-flight policy"
    );
    actor.configure_expedition(ExpeditionRole::Dragon, &tuning.encounters);
    let accepted = steering::pursuit_flight_goal(
        &actor,
        desired,
        &collision,
        &view,
        geometry,
        &tuning.encounters,
    )
    .expect("clear high pursuit pose");
    assert!((accepted.y + actor.dimensions.y * 0.5 - desired.y).abs() < SKIN);
    let ceiling = geometry.top(TilePos::new(HexCoord::ORIGIN, geometry.max_level));
    for invalid in [desired.with_y(ceiling + 5.0), desired + Vec3::X * 100.0] {
        assert!(steering::pursuit_flight_goal(
            &actor,
            invalid,
            &collision,
            &view,
            geometry,
            &tuning.encounters
        )
        .is_none());
    }
    // A central obstacle invalidates the complete Dragon body even though the
    // requested center and most of the surrounding aerial volume remain clear.
    let blocked = TilePos::new(HexCoord::from_world(accepted), 64);
    view.voxels.insert(blocked, SubstanceId(1));
    view.revision += 1;
    collision.refresh(&view, geometry);
    if let Some(adjusted) = steering::pursuit_flight_goal(
        &actor,
        desired,
        &collision,
        &view,
        geometry,
        &tuning.encounters,
    ) {
        assert!(adjusted.distance(accepted) > SKIN);
        assert!(crate::shapes::clear(
            &collision,
            &actor,
            adjusted,
            actor.body_yaw
        ));
    }
}

#[test]
fn airborne_pursuit_uses_remembered_height_without_tracking_hidden_motion() {
    let (mut actor, mut target, mut party, mut view, geometry, mut collision, tuning) =
        scene(Species::Dragon);
    actor.configure_expedition(ExpeditionRole::Dragon, &tuning.encounters);
    actor.feet.y = 25.0;
    actor.flying = true;
    target.configure_expedition_player();
    target.feet.y = 25.0;
    let remembered = target.feet;
    party.knowledge = Some(Knowledge {
        point: remembered,
        velocity: Vec3::ZERO,
        tick: 30,
        direct: true,
        cue_kind: None,
        observed: None,
    });
    for r in -18..=18 {
        for level in 1..=geometry.max_level {
            view.voxels.insert(
                TilePos::new(HexCoord::from_axial(0, r), level),
                SubstanceId(1),
            );
        }
    }
    view.revision += 1;
    collision.refresh(&view, geometry);
    let mut moved = target.clone();
    moved.feet += Vec3::new(0.0, 8.0, 5.0);
    let mut decisions = Vec::new();
    for hidden in [target, moved] {
        assert!(!collision.sight_clear(actor.eye(), hidden.center()));
        let mut brain = Brain::new(actor.id, actor.feet);
        let (_, request) = brain.intent(
            &actor,
            &party,
            &[hidden, actor.clone()],
            &[],
            &[],
            &collision,
            &view,
            geometry,
            &tuning,
            31,
        );
        assert!(request.is_none(), "memory alone cannot admit an attack");
        let decision = brain.decision.expect("remembered pursuit");
        assert!(!decision.own_sight);
        assert!(Vec3::from_array(decision.goal).y > 20.0);
        decisions.push((decision.goal, decision.direction, decision.observation_tick));
    }
    assert_eq!(decisions.first(), decisions.get(1));
}
