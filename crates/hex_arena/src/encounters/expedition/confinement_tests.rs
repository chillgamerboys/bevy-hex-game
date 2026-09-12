use super::*;
use crate::hex_prisms::HexPrism;
use std::collections::BTreeSet;

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
    // A continuous floor beyond the admitted arena proves confinement without
    // relying on a wall collider or an accidental cliff outside the open gate.
    for coord in center.coord.within_radius(16) {
        view.voxels
            .insert(TilePos::new(coord, center.level), materials.stone);
    }
    view.revision += 1;
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    (session, view, geometry, materials, tuning)
}

fn inside(actor: &Actor, view: &ArenaTerrainView) -> bool {
    let region = &view
        .expedition
        .as_ref()
        .expect("sites")
        .encounters
        .get("mountain_shadow")
        .expect("arena")
        .deployment;
    let columns: BTreeSet<_> = region.surfaces.iter().map(|p| p.coord).collect();
    columns.contains(&HexCoord::from_world(actor.feet))
        && HexCoord::from_world(actor.feet)
            .within_radius(2)
            .into_iter()
            .filter(|coord| !columns.contains(coord))
            .all(|coord| {
                HexPrism::new(coord.to_world(-2.0), 4.0)
                    .expect("prism")
                    .distance(actor.feet.with_y(0.0))
                    + 0.00001
                    >= actor.dimensions.x * 0.5
            })
}

#[test]
fn shadow_stays_inside_an_open_arena_under_sustained_impulse_and_jumps() {
    let (mut session, view, geometry, materials, tuning) = open_gate();
    let shadow = session
        .actors
        .iter()
        .find(|a| a.expedition_role() == Some(ExpeditionRole::MountainShadow))
        .expect("Shadow")
        .id;
    let home = session
        .actors
        .iter()
        .find(|a| a.id == shadow)
        .expect("Shadow")
        .feet;
    let mut max_height = home.y;
    for tick in 0..180 {
        let actor = session
            .actors
            .iter_mut()
            .find(|a| a.id == shadow)
            .expect("Shadow");
        actor.body.impulse_velocity = Vec3::X * 90.0;
        if tick == 0 {
            actor.body.boost(2.0);
        }
        session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
        let actor = session
            .actors
            .iter()
            .find(|a| a.id == shadow)
            .expect("Shadow");
        assert!(
            inside(actor, &view),
            "escaped on tick {tick}: {:?}",
            actor.feet
        );
        assert!(shapes::clear(
            &session.collision,
            actor,
            actor.feet,
            actor.body_yaw
        ));
        max_height = max_height.max(actor.feet.y);
        assert!((actor.hp - 125.0).abs() < 0.001);
        assert!((actor.expedition_tuning(&tuning).fireball_damage - 30.0).abs() < 0.001);
    }
    assert!(
        max_height > home.y + 1.0,
        "confinement preserves vertical movement"
    );
    assert!(
        session
            .actors
            .iter()
            .find(|a| a.id == shadow)
            .expect("Shadow")
            .feet
            .x
            > home.x + 3.0
    );
    // The same footprint restriction does not bind the player crossing the gate.
    let player = session.actors.first_mut().expect("player");
    player.feet = home + Vec3::X * 10.0;
    player.previous_feet = player.feet;
    let outside = player.feet;
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    assert!(
        session
            .actors
            .first()
            .expect("player")
            .feet
            .with_y(0.0)
            .distance(outside.with_y(0.0))
            < 0.01
    );
}

#[test]
fn separation_and_large_displacements_cannot_push_shadow_through_the_boundary() {
    let (mut session, view, geometry, _, _) = open_gate();
    let shadow = session
        .actors
        .iter()
        .find(|a| a.expedition_role() == Some(ExpeditionRole::MountainShadow))
        .expect("Shadow")
        .id;
    let home = session
        .actors
        .iter()
        .find(|a| a.id == shadow)
        .expect("Shadow")
        .feet;
    session
        .actors
        .iter_mut()
        .find(|a| a.id == shadow)
        .expect("Shadow")
        .feet = home + Vec3::X * 100.0;
    session.confine_shadow(&view, geometry);
    let boundary = session
        .actors
        .iter()
        .find(|a| a.id == shadow)
        .expect("Shadow")
        .feet;
    assert!(inside(
        session
            .actors
            .iter()
            .find(|a| a.id == shadow)
            .expect("Shadow"),
        &view
    ));
    let player = session.actors.first_mut().expect("player");
    player.feet = boundary - Vec3::X * 0.1;
    player.previous_feet = player.feet;
    session.encounter.separation_stats = separate_many(&mut session.actors, &session.collision);
    session.confine_shadow(&view, geometry);
    let actor = session
        .actors
        .iter()
        .find(|a| a.id == shadow)
        .expect("Shadow");
    assert!(inside(actor, &view));
    assert!(shapes::clear(
        &session.collision,
        actor,
        actor.feet,
        actor.body_yaw
    ));
    assert!((actor.hp - 125.0).abs() < 0.001);
}
