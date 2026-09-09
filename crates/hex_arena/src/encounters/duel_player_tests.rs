//! Explicit Duel parties preserve the old Shadow entry and map-specific rules.
use super::*;
use hex_core::arena::ArenaDeploymentRegion;

fn duel_fixture() -> (
    ArenaSession,
    ArenaTerrainView,
    ArenaVoxelGeometry,
    ArenaMaterials,
    ArenaTuning,
) {
    let (mut session, mut world, geometry, materials, tuning) = fixture(ArenaEncounter::Dragon);
    world.selection.map = ArenaMap::Duel;
    let regions = [-6, 6].map(|q| {
        let preferred = TilePos::new(HexCoord::from_axial(q, 0), 0);
        ArenaDeploymentRegion {
            preferred,
            surfaces: preferred
                .coord
                .within_radius(2)
                .into_iter()
                .map(|coord| TilePos::new(coord, 0))
                .collect(),
        }
    });
    world.spawns = regions.each_ref().map(|region| {
        region
            .preferred
            .coord
            .to_world(geometry.top(region.preferred) + SKIN)
    });
    world.battle_deployment = Some(regions);
    session.reset_with_setup(1, &world, geometry, &ArenaBattleSetup::default());
    (session, world, geometry, materials, tuning)
}

#[test]
fn explicit_player_recipes_validate_on_duel_and_fort_but_not_seven_regions() {
    for recipe in BattlePreset::ALL {
        let setup = ArenaBattleSetup {
            player_recipe: Some(recipe),
            ..Default::default()
        };
        assert!(setup.validate_for(ArenaMap::Duel).is_ok());
        assert!(setup.validate_for(ArenaMap::Fort).is_ok());
        assert_eq!(
            setup.validate_for(ArenaMap::SevenRegions),
            Err(BattleSetupError::PlayerRecipeMap)
        );
        assert_eq!(
            ArenaBattleSetup {
                control: ArenaControl::Spectator,
                ..setup
            }
            .validate_for(ArenaMap::Duel),
            Err(BattleSetupError::PlayerRecipeInSpectator)
        );
    }
}

#[test]
fn explicit_shadow_and_default_duel_retain_identical_live_policy_and_round_stats() {
    let (mut legacy, world, geometry, materials, tuning) = duel_fixture();
    let mut explicit = ArenaSession::default();
    explicit.reset_with_setup(
        1,
        &world,
        geometry,
        &ArenaBattleSetup {
            player_recipe: Some(BattlePreset::Shadow),
            seed: 919,
            ..Default::default()
        },
    );
    legacy.bot_enabled = true;
    explicit.bot_enabled = true;
    assert_eq!(world.selection.encounter, ArenaEncounter::Dragon);
    for step in 0..720 {
        let input = ActorIntent {
            aim: Vec3::X,
            movement: bevy_math::Vec2::new(if step < 240 { 0.25 } else { -0.25 }, 0.0),
            selected: Some(Spell::Fireball),
            cast_pressed: step % 180 == 0,
            cast_released: step % 180 == 30,
            cast_held: step % 180 < 30,
            ..Default::default()
        };
        let old_commands = legacy.advance(input, &world, geometry, materials, &tuning);
        let explicit_commands = explicit.advance(input, &world, geometry, materials, &tuning);
        assert_eq!(old_commands.impacts.len(), explicit_commands.impacts.len());
        assert_eq!(legacy.outcome, explicit.outcome);
        assert_eq!(legacy.tick, explicit.tick);
        assert_eq!(
            format!("{:?}", legacy.actors),
            format!("{:?}", explicit.actors)
        );
        assert_eq!(
            format!("{:?}", legacy.projectiles),
            format!("{:?}", explicit.projectiles)
        );
    }
    assert!(!legacy.encounter_summary().enabled && !explicit.encounter_summary().enabled);
    assert_eq!(
        ron::to_string(&legacy.round_summary()).expect("legacy stats"),
        ron::to_string(&explicit.round_summary()).expect("explicit stats")
    );
}

#[test]
fn duel_party_deployment_refuses_missing_player_support_or_partial_enemy_roster_atomically() {
    let (mut session, world, geometry, materials, tuning) = duel_fixture();
    let setup = ArenaBattleSetup {
        player_recipe: Some(BattlePreset::Goblins),
        ..Default::default()
    };
    for bad_side in [0, 1] {
        let mut incomplete = world.clone();
        let region = incomplete
            .battle_deployment
            .as_mut()
            .expect("regions")
            .get_mut(bad_side)
            .expect("side");
        region.surfaces.clear();
        if bad_side == 1 {
            // Exactly one valid body can be placed, but the requested ten may
            // never become a partially playable roster.
            region.surfaces.insert(region.preferred);
        }
        session.reset_with_setup(2, &incomplete, geometry, &setup);
        session.advance(
            ActorIntent::default(),
            &incomplete,
            geometry,
            materials,
            &tuning,
        );
        assert!(matches!(
            session.battle_result,
            Some(BattleResult::InvalidSetup(_))
        ));
        assert!(session.actors.is_empty() && session.parties().is_empty());
        assert!(session.projectiles.is_empty() && session.encounter.brains.is_empty());
        session.reset_with_setup(3, &world, geometry, &ArenaBattleSetup::default());
        session.advance(ActorIntent::default(), &world, geometry, materials, &tuning);
        assert!(!session.is_finished() && !session.encounter_summary().enabled);
        assert_eq!(session.actors.len(), 2);
        assert_eq!(
            session.actors.get(1).expect("original opponent").species,
            Species::Shadow
        );
    }
}

#[test]
fn custom_duel_has_no_safe_human_regeneration_and_preserves_full_party_victory_reset() {
    let (mut session, world, geometry, materials, tuning) = duel_fixture();
    let setup = ArenaBattleSetup {
        player_recipe: Some(BattlePreset::Goblins),
        ..Default::default()
    };
    session.reset_with_setup(2, &world, geometry, &setup);
    session.advance(ActorIntent::default(), &world, geometry, materials, &tuning);
    assert!(session.encounter_summary().enabled && !session.is_finished());
    assert_eq!(session.actors.len(), 11);
    session.actors.first_mut().expect("human").hp = 50.0;
    ticks(&mut session, 1200, &world, geometry, materials, &tuning);
    assert_eq!(
        session.actors.first().expect("human").hp.to_bits(),
        50.0_f32.to_bits()
    );
    session.advance(
        ActorIntent {
            selected: Some(Spell::Shield),
            cast_pressed: true,
            cast_held: true,
            ..Default::default()
        },
        &world,
        geometry,
        materials,
        &tuning,
    );
    assert!(session.actors.first().expect("human").charge().is_some());
    for actor in session.actors.iter_mut().filter(|a| a.id > 0 && a.id < 10) {
        actor.hp = 0.0;
    }
    session.advance(ActorIntent::default(), &world, geometry, materials, &tuning);
    assert!(
        !session.is_finished(),
        "one surviving Goblin still contests victory"
    );
    session.actors.last_mut().expect("last enemy").hp = 0.0;
    session.advance(ActorIntent::default(), &world, geometry, materials, &tuning);
    assert_eq!(session.outcome, Some(ArenaOutcome::Winner(0)));
    assert!(session.actors.iter().all(|a| a.charge().is_none()));
    session.reset_with_setup(3, &world, geometry, &setup);
    session.advance(ActorIntent::default(), &world, geometry, materials, &tuning);
    assert_eq!(session.actors.len(), 11);
    assert!(session
        .actors
        .iter()
        .all(|a| a.hp > 0.0 && a.charge().is_none()));
    assert!(!session.is_finished() && session.projectiles.is_empty());
    session.reset_with_setup(4, &world, geometry, &ArenaBattleSetup::default());
    session.advance(ActorIntent::default(), &world, geometry, materials, &tuning);
    assert_eq!(session.actors.len(), 2);
    assert!(!session.encounter_summary().enabled);
    assert_eq!(
        session.actors.get(1).expect("legacy Shadow").species,
        Species::Shadow
    );
}
