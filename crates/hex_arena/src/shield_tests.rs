//! Shield actor hits and published terrain admission through the session spell path.

use super::*;
use hex_core::{ElementId, HexCoord, SubstanceId};

fn fixture() -> (
    ArenaSession,
    ArenaTerrainView,
    ArenaVoxelGeometry,
    ArenaMaterials,
    ArenaTuning,
) {
    let geometry = ArenaVoxelGeometry::default();
    let materials = ArenaMaterials {
        stone: SubstanceId(1),
        bedrock: SubstanceId(2),
        grass: SubstanceId(3),
        dirt: SubstanceId(4),
        fire: ElementId(1),
    };
    let view = ArenaTerrainView {
        revision: 1,
        voxels: HexCoord::ORIGIN
            .within_radius(12)
            .into_iter()
            .map(|coord| (TilePos::new(coord, 0), materials.stone))
            .collect(),
        spawns: [Vec3::new(-4.0, SKIN, 0.0), Vec3::new(2.0, SKIN, 0.0)],
        ..Default::default()
    };
    let mut session = ArenaSession {
        bot_enabled: false,
        ..Default::default()
    };
    session.reset(0, &view, geometry);
    (session, view, geometry, materials, ArenaTuning::default())
}

#[test]
fn shield_actor_hit_adds_one_frozen_horizontal_nudge_without_damage_or_teleport() {
    let (mut session, view, geometry, materials, mut tuning) = fixture();
    let feet: Vec<_> = session.actors.iter().map(|actor| actor.feet).collect();
    let mut emitted = CommandsOut::default();
    session.release(
        0,
        Spell::Shield,
        &tuning,
        32.0,
        &view,
        geometry,
        materials,
        &mut emitted,
    );
    tuning.shield_push = 9.0;
    for _ in 0..90 {
        session.advance_projectiles(&view, geometry, materials, &mut emitted);
    }
    assert!(session.projectiles.is_empty());
    assert_eq!(session.pending_walls.len(), 1);
    assert!(emitted.impacts.is_empty());
    assert!(session
        .actors
        .iter()
        .all(|actor| (actor.hp - 100.0).abs() < 0.001));
    assert_eq!(
        session
            .actors
            .iter()
            .map(|actor| actor.feet)
            .collect::<Vec<_>>(),
        feet
    );
    let human = session
        .actors
        .iter()
        .find(|actor| actor.id == 0)
        .expect("caster");
    let target = session
        .actors
        .iter()
        .find(|actor| actor.id == 1)
        .expect("struck actor");
    assert_eq!(human.impulse_velocity(), Vec3::ZERO);
    assert!(target.impulse_velocity().distance(Vec3::X * 2.0) < 0.001);
    assert!(
        target
            .impulse_velocity()
            .distance(Vec3::X * tuning.shield_push)
            > 1.0,
        "the in-flight shield retains its launch-time push tuning"
    );
}

#[test]
fn actor_hit_forms_available_cells_even_when_the_target_never_moves() {
    let (mut session, view, geometry, materials, tuning) = fixture();
    let mut emitted = CommandsOut::default();
    session.release(
        0,
        Spell::Shield,
        &tuning,
        32.0,
        &view,
        geometry,
        materials,
        &mut emitted,
    );
    for _ in 0..90 {
        session.advance_projectiles(&view, geometry, materials, &mut emitted);
        // Deliberately do not advance actors: model a target unable to move away.
        session.advance_walls(&view, geometry, materials, &mut emitted);
    }
    assert_eq!(session.shields_raised, 1);
    assert!(!emitted.edits.is_empty() && emitted.edits.len() < 25);
    assert!(emitted.impacts.is_empty());
    for edit in &emitted.edits {
        assert!(!view.voxels.contains_key(&edit.pos()));
        assert!(session
            .actors
            .iter()
            .all(|actor| !collision::voxel_overlaps_body(
                edit.pos(),
                geometry,
                actor.feet,
                BODY_HEIGHT,
                BODY_RADIUS + SKIN * 4.0,
            )));
    }
}

#[test]
fn no_room_notice_expires_after_two_seconds_without_erasing_a_newer_notice() {
    let (mut session, view, geometry, materials, tuning) = fixture();
    session.shield_no_room_notice();
    for _ in 0..239 {
        session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    }
    assert_eq!(session.notice, "No room for new shield blocks");
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    assert!(session.notice.is_empty());
    session.shield_no_room_notice();
    session.notice = "Fireball is cooling down.".into();
    for _ in 0..240 {
        session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    }
    assert_eq!(session.notice, "Fireball is cooling down.");
}
