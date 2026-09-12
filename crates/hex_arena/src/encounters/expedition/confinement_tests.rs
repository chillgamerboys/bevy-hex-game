use super::*;

fn open_gate() -> (
    ArenaSession,
    ArenaTerrainView,
    ArenaVoxelGeometry,
    ArenaMaterials,
    ArenaTuning,
) {
    let (mut session, mut view, geometry, materials, tuning) = fixture();
    let center = view
        .expedition
        .as_ref()
        .expect("sites")
        .encounters
        .get("mountain_shadow")
        .expect("arena")
        .deployment
        .preferred;
    // A continuous floor beyond the authored arena models an open/destroyed gate.
    for coord in center.coord.within_radius(16) {
        view.voxels
            .insert(TilePos::new(coord, center.level), materials.stone);
    }
    view.revision += 1;
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    (session, view, geometry, materials, tuning)
}

fn shadow_id(session: &ArenaSession) -> ActorId {
    session
        .actors
        .iter()
        .find(|a| a.expedition_role() == Some(ExpeditionRole::MountainShadow))
        .expect("Shadow")
        .id
}

#[test]
fn shadow_can_cross_a_destroyed_arena_boundary_by_physical_impulse() {
    let (mut session, view, geometry, materials, tuning) = open_gate();
    let id = shadow_id(&session);
    let actor = session
        .actors
        .iter_mut()
        .find(|a| a.id == id)
        .expect("Shadow");
    let home = actor.feet;
    actor.body.impulse_velocity = Vec3::X * 90.0;
    for _ in 0..30 {
        session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    }
    let actor = session.actors.iter().find(|a| a.id == id).expect("Shadow");
    assert!(
        actor.feet.x > home.x + 8.0,
        "no invisible rollback at former boundary"
    );
    assert!(shapes::clear(
        &session.collision,
        actor,
        actor.feet,
        actor.body_yaw
    ));
    assert!((actor.hp - 125.0).abs() < 0.001);
}

#[test]
fn displaced_shadow_returns_by_ordinary_continuous_ai_movement() {
    let (mut session, view, geometry, materials, tuning) = open_gate();
    let id = shadow_id(&session);
    let actor = session
        .actors
        .iter_mut()
        .find(|a| a.id == id)
        .expect("Shadow");
    let home = actor.feet;
    actor.feet += Vec3::X * 12.0;
    actor.previous_feet = actor.feet;
    let mut previous = actor.feet;
    let party = actor.party.expect("party");
    session
        .encounter
        .runtime
        .iter_mut()
        .find(|p| p.snapshot.id == party)
        .expect("runtime")
        .snapshot
        .phase = PartyPhase::Returning;
    session.bot_enabled = true;
    for _ in 0..480 {
        session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
        let actor = session.actors.iter().find(|a| a.id == id).expect("Shadow");
        assert!(
            previous.distance(actor.feet) < 0.5,
            "homing must not teleport"
        );
        previous = actor.feet;
    }
    assert!(previous.with_y(0.0).distance(home.with_y(0.0)) < 4.5);
}
