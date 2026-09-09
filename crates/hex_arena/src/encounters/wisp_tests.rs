//! Real session admission, windup and flying-swarm behavior.
use super::*;
use hex_core::arena::ArenaDeploymentRegion;

fn deployed(
    left: BattlePreset,
    right: BattlePreset,
) -> (
    ArenaSession,
    ArenaTerrainView,
    ArenaVoxelGeometry,
    ArenaMaterials,
    ArenaTuning,
) {
    let (mut session, mut view, geometry, materials, tuning) = fixture(ArenaEncounter::Dragon);
    view.battle_deployment = Some([-6, 6].map(|q| {
        let preferred = TilePos::new(HexCoord::from_axial(q, 0), 0);
        ArenaDeploymentRegion {
            preferred,
            surfaces: preferred
                .coord
                .within_radius(1)
                .into_iter()
                .map(|coord| TilePos::new(coord, 0))
                .collect(),
        }
    }));
    let setup = ArenaBattleSetup::spectator(left, right, 1);
    session.reset_with_setup(9, &view, geometry, &setup);
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    assert!(!session.is_finished(), "{:?}", session.battle_summary());
    (session, view, geometry, materials, tuning)
}

#[test]
fn all_swarm_counts_reserve_two_layers_and_replay_reset_exactly() {
    for preset in BattlePreset::WISP_SWARMS {
        let (mut session, view, geometry, materials, tuning) = deployed(preset, preset);
        let before: Vec<_> = session
            .actors
            .iter()
            .map(|a| (a.id, a.feet, a.flight_layer()))
            .collect();
        for actor in &session.actors {
            let layer = actor.flight_layer().expect("reserved layer");
            assert!((actor.feet.y - (SKIN + 4.0 + f32::from(layer) * 0.8)).abs() < SKIN);
            assert!(actor.flying && !actor.grounded);
            assert!(session.actor_pose_valid(actor.id, &view, geometry));
            assert!(session
                .actors
                .iter()
                .filter(|other| other.id != actor.id)
                .all(|other| body_overlap(actor, other).is_none()));
        }
        let setup = session.accepted_battle_setup().clone();
        session.reset_with_setup(10, &view, geometry, &setup);
        session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
        let after: Vec<_> = session
            .actors
            .iter()
            .map(|a| (a.id, a.feet, a.flight_layer()))
            .collect();
        assert_eq!(before, after);
    }
}

#[test]
fn obstructed_flight_corridors_refuse_the_whole_swarm_without_rooftop_fallback() {
    let (mut session, mut view, geometry, materials, tuning) =
        deployed(BattlePreset::Wisps12, BattlePreset::Wisp);
    let region = view
        .battle_deployment
        .as_ref()
        .expect("regions")
        .first()
        .expect("left");
    for surface in &region.surfaces {
        view.voxels
            .insert(TilePos::new(surface.coord, 4), materials.stone);
    }
    view.revision += 1;
    let setup = session.accepted_battle_setup().clone();
    session.reset_with_setup(10, &view, geometry, &setup);
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    assert!(session.is_finished() && session.actors.is_empty());
    assert!(matches!(
        session.battle_summary().expect("summary").result,
        Some(BattleResult::InvalidSetup(_))
    ));
}

#[test]
fn a_close_goblin_below_is_a_legal_target_and_the_wisp_stays_above_melee() {
    let (mut session, view, geometry, materials, tuning) =
        deployed(BattlePreset::Wisp, BattlePreset::Goblin);
    pose(&mut session, 0, Vec3::new(0.0, 4.0, 0.0), Vec3::NEG_Y);
    pose(&mut session, 1, Vec3::ZERO, Vec3::Y);
    session.bot_enabled = true;
    let before = session.actors.get(1).expect("goblin").hp;
    let mut windup = false;
    let mut fresh = false;
    for _ in 0..240 {
        session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
        windup |= session
            .actors
            .first()
            .and_then(Actor::attack_state)
            .is_some_and(|a| {
                a.kind == CreatureAbility::WispEmber && a.phase == AttackPhase::Windup
            });
        fresh |= session
            .projectiles
            .iter()
            .any(|p| p.source_ability() == Some(CreatureAbility::WispEmber) && p.age.abs() < SKIN);
    }
    let wisp = session.actors.first().expect("wisp");
    let goblin = session.actors.get(1).expect("goblin");
    assert!(
        windup && fresh,
        "normal windup and next-tick shot chronology"
    );
    assert!(goblin.hp < before - 4.0, "close shot must actually damage");
    assert!((wisp.hp - wisp.max_hp).abs() < SKIN && wisp.feet.y > 3.9);
    let count = session
        .encounter
        .ability_counts
        .get(&0)
        .expect("release count");
    assert_eq!(
        *count
            .get(CreatureAbility::WispEmber.index())
            .expect("ember"),
        1
    );
}

#[test]
fn own_last_sight_admits_only_two_real_cover_shots_then_expires() {
    let (mut session, mut view, geometry, materials, tuning) =
        deployed(BattlePreset::Wisp, BattlePreset::Goblin);
    session.bot_enabled = true;
    for _ in 0..8 {
        session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    }
    assert!(session
        .actors
        .first()
        .and_then(Actor::attack_state)
        .is_some());
    for q in -1..=1 {
        for r in -20..=20 {
            for level in 1..=24 {
                view.voxels.insert(
                    TilePos::new(HexCoord::from_axial(q, r), level),
                    materials.stone,
                );
            }
        }
    }
    view.revision += 1;
    let mut impacts = Vec::new();
    for _ in 0..650 {
        impacts.extend(
            session
                .advance(ActorIntent::default(), &view, geometry, materials, &tuning)
                .impacts,
        );
    }
    assert!(
        impacts.iter().any(|impact| matches!(
            impact.kind,
            hex_core::TerrainDamageKind::Elemental(_)
        ) && impact
            .volume
            .iter()
            .any(|pos| pos.level > 0 && view.voxels.contains_key(pos))),
        "a real Ember must contact the obstructing terrain"
    );
    assert!(session.projectiles.is_empty());
    assert_eq!(
        session
            .encounter
            .ability_counts
            .get(&0)
            .and_then(|counts| counts.get(CreatureAbility::WispEmber.index()))
            .copied(),
        Some(2)
    );
    assert!(session
        .actors
        .first()
        .and_then(Actor::attack_state)
        .is_none());
}

#[test]
fn a_live_twelve_wisp_swarm_keeps_distinct_layers_and_clear_bodies() {
    let (mut session, view, geometry, materials, mut tuning) =
        deployed(BattlePreset::Wisps12, BattlePreset::Shadow);
    tuning.encounters.wisp_ember_cooldown = 20.0;
    for actor in &mut session.actors {
        actor.hp = 1000.0;
        actor.max_hp = 1000.0;
    }
    // Disable opposing attacks without changing the swarm's ordinary AI/motion.
    if let Some(shadow) = session.actors.last_mut() {
        shadow.cooldowns = [1000.0; 3];
    }
    session.bot_enabled = true;
    for _ in 0..240 {
        session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
        for actor in session.actors.iter().filter(|a| a.species == Species::Wisp) {
            let expected = 4.0 + f32::from(actor.flight_layer().expect("layer")) * 0.8;
            assert!(
                (actor.feet.y - expected).abs() < 0.1,
                "layer drift: {:?}",
                actor.feet
            );
            assert!(session.actor_volume_valid(actor.id, &view, geometry));
            assert!(
                session
                    .actors
                    .iter()
                    .filter(|other| other.id != actor.id)
                    .all(|other| body_overlap(actor, other).is_none()),
                "swarm overlap actor {}",
                actor.id
            );
        }
    }
}

#[test]
fn high_support_cannot_deploy_a_complete_wisp_above_the_published_vertical_ceiling() {
    let (mut session, mut view, geometry, materials, tuning) =
        deployed(BattlePreset::Wisp, BattlePreset::Wisp);
    for region in view.battle_deployment.as_mut().expect("regions") {
        region.preferred.level = geometry.max_level;
        region.surfaces = region
            .surfaces
            .iter()
            .map(|pos| TilePos::new(pos.coord, geometry.max_level))
            .collect();
        for surface in &region.surfaces {
            view.voxels.insert(*surface, materials.stone);
        }
    }
    view.revision += 1;
    let setup = session.accepted_battle_setup().clone();
    session.reset_with_setup(10, &view, geometry, &setup);
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    assert!(session.actors.is_empty() && session.is_finished());
    assert!(matches!(
        session.battle_summary().expect("summary").result,
        Some(BattleResult::InvalidSetup(_))
    ));
}
