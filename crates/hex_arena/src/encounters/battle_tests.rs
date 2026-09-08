//! Spectator contracts exercised through the ordinary session and projectile steps.

use super::*;
use hex_core::arena::ArenaDeploymentRegion;

fn battle(
    left: BattlePreset,
    right: BattlePreset,
) -> (
    ArenaSession,
    ArenaTerrainView,
    ArenaVoxelGeometry,
    ArenaMaterials,
    ArenaTuning,
) {
    let (mut session, mut world, geometry, materials, tuning) = fixture(ArenaEncounter::Dragon);
    world.battle_deployment = Some([-4, 4].map(|q| {
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
    let mut setup = ArenaBattleSetup::spectator(left, right, 0x9182_7314_5566_1234);
    for (roster, team) in setup.rosters.iter_mut().zip([7, 42]) {
        roster.team = team;
    }
    session.reset_with_setup(2, &world, geometry, &setup);
    session.advance(ActorIntent::default(), &world, geometry, materials, &tuning);
    assert!(
        !session.is_finished(),
        "admission: {:?}",
        session.battle_summary()
    );
    (session, world, geometry, materials, tuning)
}

#[test]
fn all_original_spectator_rosters_admit_real_monster_zero_and_complete_dry_bodies() {
    for left in BattlePreset::ORIGINAL {
        for right in BattlePreset::ORIGINAL {
            let (session, world, geometry, _, _) = battle(left, right);
            assert_eq!(session.human_actor_id(), None);
            assert!(session.outcome.is_none());
            assert_eq!(
                session.actors.first().expect("monster zero").species,
                *left.members().first().expect("preset")
            );
            assert!(session
                .actors
                .iter()
                .all(|a| a.species != Species::Human
                    && session.actor_pose_valid(a.id, &world, geometry)));
            assert!(session
                .parties()
                .iter()
                .all(|p| p.phase == PartyPhase::Active));
            for (index, actor) in session.actors.iter().enumerate() {
                assert!(session
                    .actors
                    .iter()
                    .skip(index + 1)
                    .all(|other| body_overlap(actor, other).is_none()));
            }
            let summary = session.battle_summary().expect("spectator");
            assert_eq!(
                summary
                    .teams
                    .iter()
                    .map(|team| (team.team, team.initial, team.living))
                    .collect::<Vec<_>>(),
                vec![
                    (7, left.members().len(), left.members().len()),
                    (42, right.members().len(), right.members().len())
                ]
            );
        }
    }
}

#[test]
fn failed_deployment_is_atomic_and_cannot_spawn_on_unrelated_surfaces() {
    let (mut session, mut world, geometry, materials, tuning) =
        battle(BattlePreset::Shadow, BattlePreset::Goblins);
    let setup = session.accepted_battle_setup().clone();
    let regions = world.battle_deployment.as_mut().expect("regions");
    let region = regions.get_mut(1).expect("right");
    region.surfaces = [region.preferred].into_iter().collect();
    session.reset_with_setup(3, &world, geometry, &setup);
    session.advance(ActorIntent::default(), &world, geometry, materials, &tuning);
    assert!(session.actors.is_empty());
    assert!(session.is_finished());
    assert!(matches!(
        session.battle_summary().expect("result").result,
        Some(BattleResult::InvalidSetup(_))
    ));
    assert!(session.outcome.is_none());
    let before = session.tick;
    ticks(&mut session, 30, &world, geometry, materials, &tuning);
    assert_eq!(before, session.tick);
}

#[test]
fn accepted_setup_and_observer_input_do_not_control_monster_zero() {
    let (mut session, world, geometry, materials, tuning) =
        battle(BattlePreset::Shadow, BattlePreset::Goblins);
    let accepted = session.accepted_battle_setup().clone();
    let mut request = accepted.clone();
    request.seed = 12;
    request.control = ArenaControl::Player;
    let before = session.actors.first().expect("zero").feet;
    let out = session.advance(
        ActorIntent {
            movement: bevy_math::Vec2::ONE,
            aim: Vec3::X,
            selected: Some(Spell::AreaBlast),
            cast_pressed: true,
            cast_released: true,
            ..Default::default()
        },
        &world,
        geometry,
        materials,
        &tuning,
    );
    assert!(out.impacts.is_empty());
    assert!(session.projectiles.is_empty());
    assert!(session.actors.first().expect("zero").feet.distance(before) < 0.001);
    assert_eq!(session.accepted_battle_setup(), &accepted);
    assert!(crate::preview(&session, &world, &geometry, &tuning)
        .points
        .is_empty());
    session.reset_with_setup(3, &world, geometry, &request);
    assert_eq!(session.human_actor_id(), Some(0));
    assert!(session.battle_summary().is_none());
    assert_eq!(
        session.actors.first().expect("human").species,
        Species::Human
    );
}

#[test]
fn team_result_waits_for_all_members_and_distinguishes_draw_from_timeout() {
    let (mut session, world, geometry, materials, tuning) =
        battle(BattlePreset::Shadow, BattlePreset::Goblins);
    for actor in session.actors.iter_mut().filter(|a| a.team == 42).take(4) {
        actor.hp = 0.0;
    }
    ticks(&mut session, 1, &world, geometry, materials, &tuning);
    assert!(!session.is_finished());
    for actor in session.actors.iter_mut().filter(|a| a.team == 42) {
        actor.hp = 0.0;
    }
    ticks(&mut session, 1, &world, geometry, materials, &tuning);
    assert_eq!(
        session.battle_summary().expect("result").result,
        Some(BattleResult::TeamWinner(7))
    );
    let initial = session
        .battle_summary()
        .expect("summary")
        .teams
        .get(1)
        .expect("team")
        .max_hp;
    assert!((initial - 250.0).abs() < 0.001);
    let setup = session.accepted_battle_setup().clone();
    session.reset_with_setup(3, &world, geometry, &setup);
    ticks(&mut session, 1, &world, geometry, materials, &tuning);
    for actor in &mut session.actors {
        actor.hp = 0.0;
    }
    ticks(&mut session, 1, &world, geometry, materials, &tuning);
    assert_eq!(
        session.battle_summary().expect("draw").result,
        Some(BattleResult::Draw)
    );
    let mut limited = setup;
    limited.tick_limit = Some(3);
    session.reset_with_setup(4, &world, geometry, &limited);
    ticks(&mut session, 3, &world, geometry, materials, &tuning);
    assert_eq!(
        session.battle_summary().expect("timeout").result,
        Some(BattleResult::Timeout)
    );
    assert!(session.actors.iter().all(|a| a.hp > 0.0));
}

#[test]
fn spectator_zero_receives_no_player_regeneration() {
    let (mut session, world, geometry, materials, tuning) =
        battle(BattlePreset::Goblins, BattlePreset::Goblins);
    session.actors.first_mut().expect("zero").hp = 20.0;
    ticks(&mut session, 1200, &world, geometry, materials, &tuning);
    assert!((session.actors.first().expect("zero").hp - 20.0).abs() < 0.001);
    assert!(session
        .parties()
        .iter()
        .all(|p| p.phase == PartyPhase::Active));
}

#[test]
fn accepted_seed_replays_brains_and_reset_removes_previous_battle_effects() {
    let (mut session, world, geometry, materials, tuning) =
        battle(BattlePreset::Shadow, BattlePreset::ShamanParty);
    let setup = session.accepted_battle_setup().clone();
    session.bot_enabled = true;
    let mut snapshots = Vec::new();
    for generation in [3, 4] {
        session.reset_with_setup(generation, &world, geometry, &setup);
        ticks(&mut session, 180, &world, geometry, materials, &tuning);
        snapshots.push(format!(
            "{:?}|{:?}|{:?}|{:?}",
            session.actors,
            session.projectiles,
            session.encounter.brains,
            session.battle_summary()
        ));
    }
    assert_eq!(snapshots.first(), snapshots.get(1));
    session.reset_with_setup(5, &world, geometry, &setup);
    assert!(session.projectiles.is_empty());
    assert!(session.effects.is_empty());
    assert!(session.combat_cues.is_empty());
    assert!(session.pending_impacts.is_empty());
    assert!(session.pending_walls.is_empty());
    assert!(session.barriers().is_empty());
    assert!(session.auras().is_empty());
    assert_eq!(session.tick, 0);
}

#[test]
fn released_projectile_and_impact_cue_keep_source_team_after_owner_removal() {
    let (mut session, world, geometry, materials, tuning) =
        battle(BattlePreset::Goblins, BattlePreset::Shadow);
    for actor in &mut session.actors {
        actor.feet.z += 14.0;
        actor.previous_feet = actor.feet;
    }
    pose(&mut session, 0, Vec3::new(-5.0, 0.0, 0.0), Vec3::X);
    pose(&mut session, 1, Vec3::new(-2.0, 0.0, 0.0), Vec3::X);
    pose(&mut session, 5, Vec3::new(2.0, 0.0, 0.0), Vec3::NEG_X);
    let mut out = CommandsOut::default();
    session.release(
        0,
        Spell::Fireball,
        &tuning,
        32.0,
        &world,
        geometry,
        materials,
        &mut out,
    );
    let source = session.projectiles.first().expect("released").source_team();
    assert_eq!(source, 7);
    session.actors.retain(|actor| actor.id != 0);
    let ally_hp = session.actors.iter().find(|a| a.id == 1).expect("ally").hp;
    for _ in 0..60 {
        session.advance_projectiles(&world, geometry, materials, &mut out);
    }
    assert!(session.projectiles.is_empty());
    assert!((session.actors.iter().find(|a| a.id == 1).expect("ally").hp - ally_hp).abs() < 0.001);
    assert!(
        session
            .actors
            .iter()
            .find(|a| a.id == 5)
            .expect("hostile")
            .hp
            < 100.0
    );
    let cue = session.combat_cues.last().expect("impact cue");
    assert_eq!(cue.team, 7);
    assert_eq!(cue.kind, CombatCueKind::Impact);
}

#[test]
fn observed_shapes_and_velocities_preserve_identity_without_live_motion_access() {
    let (mut session, world, geometry, _, tuning) =
        battle(BattlePreset::Shadow, BattlePreset::Dragon);
    let observer = session.actors.first().expect("shadow").clone();
    let old = targeting::observe(&observer, &session.actors, &[], &session.collision, 1, 0.5);
    let seen = old.first().expect("dragon visible");
    assert_eq!(seen.body.species, Species::Dragon);
    assert!((seen.body.dimensions.y - 0.4).abs() < 0.001);
    let target = session.actors.get_mut(1).expect("dragon");
    target.feet.z += 0.1;
    target.body.impulse_velocity = Vec3::splat(100.0);
    let now = targeting::observe(
        &observer,
        &session.actors,
        &old,
        &session.collision,
        13,
        0.5,
    );
    assert!((now.first().expect("seen").body.velocity.z - 1.0).abs() < 0.001);
    let mut caster = observer;
    caster.selected = Spell::Fireball;
    caster.aim = (now.first().expect("seen").center() - caster.eye()).normalize();
    let bodies = now.iter().map(|o| o.body).collect::<Vec<_>>();
    let forecast = spells::forecast_spell(
        &caster,
        &bodies,
        &session.collision,
        &world,
        geometry,
        &tuning,
        32.0,
    );
    assert!(forecast.impact.is_some());
}

#[test]
fn hostile_sensing_and_shadow_history_are_identical_for_divergent_hidden_positions() {
    let (mut left, mut world, geometry, _, tuning) =
        battle(BattlePreset::Shadow, BattlePreset::Dragon);
    let mut right_actors = left.actors.clone();
    let mut left_bot = Bot::with_seed(19);
    let mut right_bot = Bot::with_seed(19);
    let search = Vec3::X * 7.0;
    for tick in 1..=13 {
        let a = left_bot.intent_battle(
            0,
            search,
            &left.actors,
            &[],
            &left.collision,
            &world,
            geometry,
            &tuning,
            &[],
            tick,
        );
        let b = right_bot.intent_battle(
            0,
            search,
            &right_actors,
            &[],
            &left.collision,
            &world,
            geometry,
            &tuning,
            &[],
            tick,
        );
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
    }
    for r in -20..=20 {
        for level in 1..=20 {
            world.voxels.insert(
                TilePos::new(HexCoord::from_axial(0, r), level),
                SubstanceId(1),
            );
        }
    }
    world.revision += 1;
    left.collision.refresh(&world, geometry);
    right_actors.get_mut(1).expect("hidden target").feet += Vec3::new(3.0, 1.0, 2.0);
    for tick in 14..=180 {
        let a = left_bot.intent_battle(
            0,
            search,
            &left.actors,
            &[],
            &left.collision,
            &world,
            geometry,
            &tuning,
            &[],
            tick,
        );
        let b = right_bot.intent_battle(
            0,
            search,
            &right_actors,
            &[],
            &left.collision,
            &world,
            geometry,
            &tuning,
            &[],
            tick,
        );
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
        assert_eq!(format!("{left_bot:?}"), format!("{right_bot:?}"));
    }
}

#[test]
fn battle_cues_are_frozen_hostile_events_shared_only_with_the_hearing_party() {
    let (mut session, mut world, geometry, materials, tuning) =
        battle(BattlePreset::Goblins, BattlePreset::Shadow);
    let mut setup = session.accepted_battle_setup().clone();
    setup.rosters.first_mut().expect("team").parties =
        vec![vec![Species::Goblin], vec![Species::Goblin]];
    session.reset_with_setup(3, &world, geometry, &setup);
    ticks(&mut session, 1, &world, geometry, materials, &tuning);
    pose(&mut session, 0, Vec3::new(-4.0, 0.0, 0.0), Vec3::X);
    pose(&mut session, 1, Vec3::new(-4.0, 0.0, 14.0), Vec3::X);
    pose(&mut session, 2, Vec3::new(5.0, 0.0, 0.0), Vec3::NEG_X);
    for r in -20..=20 {
        for level in 1..=15 {
            world.voxels.insert(
                TilePos::new(HexCoord::from_axial(0, r), level),
                SubstanceId(1),
            );
        }
    }
    world.revision += 1;
    session.collision.refresh(&world, geometry);
    session.combat_cue_from(2, 42, Vec3::new(-1.0, 0.0, 0.0), CombatCueKind::Release);
    session.combat_cue_from(0, 7, Vec3::new(-4.0, 0.0, 14.0), CombatCueKind::Impact);
    ticks(&mut session, 24, &world, geometry, materials, &tuning);
    let knowledge = session.party_knowledge();
    assert_eq!(
        knowledge.first().expect("hearing party").source,
        "heard-release"
    );
    assert_eq!(knowledge.get(1).expect("remote party").source, "none");
    assert!(session
        .parties()
        .iter()
        .all(|p| p.phase == PartyPhase::Active));
    let remembered_tick = knowledge.first().expect("hearing party").tick;
    ticks(&mut session, 24, &world, geometry, materials, &tuning);
    assert_eq!(
        session.party_knowledge().first().expect("memory").tick,
        remembered_tick
    );
}

#[test]
fn goblin_attacks_the_reachable_front_of_a_long_dragon_body() {
    let (mut session, world, geometry, materials, tuning) =
        battle(BattlePreset::Goblins, BattlePreset::Dragon);
    pose(&mut session, 0, Vec3::new(-2.1, 0.0, 0.0), Vec3::X);
    pose(&mut session, 5, Vec3::ZERO, Vec3::NEG_X);
    session.encounter.brains.retain(|id, _| *id == 0);
    let goblin = session.actors.first().expect("goblin");
    let party = session.encounter.runtime.first().expect("party");
    let (_, request) = session.encounter.brains.get_mut(&0).expect("brain").intent(
        goblin,
        party,
        &session.actors,
        &[],
        &[],
        &session.collision,
        &world,
        geometry,
        &tuning,
        session.tick + 1,
    );
    assert_eq!(
        request
            .expect("reachable body admits windup immediately")
            .kind,
        CreatureAbility::Swipe
    );
    session.bot_enabled = true;
    ticks(&mut session, 50, &world, geometry, materials, &tuning);
    assert!(
        session
            .actors
            .iter()
            .find(|a| a.id == 5)
            .expect("long target")
            .hp
            < tuning.encounters.dragon_hp
    );
    assert!(
        session
            .encounter_stats()
            .iter()
            .find(|a| a.id == 0)
            .expect("goblin")
            .abilities
            .get(4)
            .copied()
            .unwrap_or(0)
            > 0
    );
}
