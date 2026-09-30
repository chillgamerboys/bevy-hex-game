//! Idle travel avoids speculative probes, while real physics and spacing continue.
use super::*;

#[test]
fn arrived_goblin_skips_remote_revision_probes_but_keeps_crowd_spacing() {
    let (mut actor, target, mut party, mut view, geometry, mut collision, tuning) =
        scene(Species::Goblin);
    party.snapshot.phase = PartyPhase::Dormant;
    let mut brain = Brain::new(actor.id, actor.feet);
    brain.intent(
        &actor,
        &party,
        &[target.clone(), actor.clone()],
        &[],
        &[],
        &collision,
        &view,
        geometry,
        &tuning,
        1,
    );
    actor.feet = Vec3::from_array(brain.decision.as_ref().expect("home goal").goal) + Vec3::X * 0.2;
    let mut diagnostics = ArenaSession::default();
    diagnostics.set_cpu_profiling(true);
    for tick in 2..=3 {
        if tick == 3 {
            let remote = TilePos::new(HexCoord::from_axial(15, 0), 0);
            assert!(view.voxels.remove(&remote).is_some());
            view.revision += 1;
            collision.refresh(&view, geometry);
        }
        diagnostics.cpu.begin(view.revision);
        let scope = diagnostics.cpu.begin_brains();
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
        diagnostics.cpu.finish_brains(scope);
        diagnostics.cpu.finish(tick);
        let counters = diagnostics
            .cpu_profile()
            .expect("profiled idle tick")
            .steering;
        assert_eq!(counters.decisions, 0);
        assert_eq!(counters.walk_probe_steps, 0);
        assert!(intent.direction.length_squared() < 0.001);
        assert!(request.is_none());
    }
    // Arrival suppresses the requested home motion, not the independent spacing
    // response. Live inter-actor separation remains in the ordinary session tick.
    let mut ally = actor.clone();
    ally.id = 8;
    ally.feet += Vec3::X * (tuning.encounters.goblin_spacing * 0.5);
    let (intent, _) = brain.intent(
        &actor,
        &party,
        &[target, actor.clone(), ally],
        &[],
        &[],
        &collision,
        &view,
        geometry,
        &tuning,
        4,
    );
    assert!(
        intent.direction.x < -0.1,
        "a nearby ally still requests spacing"
    );
}

#[test]
fn arrived_goblin_settles_when_its_support_is_carved_away() {
    let (mut actor, target, mut party, mut view, geometry, mut collision, tuning) =
        scene(Species::Goblin);
    let geometry = ArenaVoxelGeometry {
        min_level: -10,
        ..geometry
    };
    party.snapshot.phase = PartyPhase::Dormant;
    let mut brain = Brain::new(actor.id, actor.feet);
    brain.intent(
        &actor,
        &party,
        &[target.clone(), actor.clone()],
        &[],
        &[],
        &collision,
        &view,
        geometry,
        &tuning,
        1,
    );
    actor.feet = Vec3::from_array(brain.decision.as_ref().expect("home goal").goal) + Vec3::X * 0.2;
    let before = actor.feet;
    for coord in HexCoord::from_world(actor.feet).within_radius(2) {
        view.voxels.remove(&TilePos::new(coord, 0));
        view.voxels.insert(TilePos::new(coord, -3), SubstanceId(1));
    }
    view.revision += 1;
    collision.refresh(&view, geometry);
    for tick in 2..=121 {
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
        assert!(request.is_none());
        assert!(intent.direction.length_squared() < 0.001);
        motion::tick(
            &mut actor,
            intent.direction,
            intent.input.run,
            intent.input.jump,
            intent.flight,
            &collision,
            &tuning.encounters,
        );
        if tick == 2 {
            assert!(
                actor.feet.y < before.y,
                "removed support falls in the same tick"
            );
            assert!(!actor.grounded);
        }
    }
    let lower_floor = geometry.top(TilePos::new(HexCoord::from_world(actor.feet), -3));
    assert!(actor.grounded);
    assert!((actor.feet.y - lower_floor - SKIN).abs() < SKIN * 2.0);
    assert!(actor.feet.with_y(0.0).distance(before.with_y(0.0)) < SKIN);
}
