//! Shared Golem shape/recipe contracts, before runtime geometry is admitted.

use super::*;

#[test]
fn golem_projection_is_seven_native_prisms_with_fixed_body_orientation() {
    let tuning = EncounterTuning::default();
    let mut actor = Actor::spawn(7, Vec3::new(2.0, 3.2, 4.0), Vec3::X);
    actor.configure_species(Species::Golem, &tuning);
    let prisms: Vec<_> = actor.body_hex_prisms().collect();
    assert_eq!(prisms.len(), 7);
    let expected = HexCoord::ORIGIN.within_radius(1);
    for prism in &prisms {
        assert!(prism.offset.y.abs() < SKIN && (prism.height - 2.0).abs() < SKIN);
        assert!(expected
            .iter()
            .any(|hex| hex.to_world(0.0).distance(prism.offset) < SKIN));
    }
    for (index, prism) in prisms.iter().enumerate() {
        assert!(prisms
            .iter()
            .skip(index + 1)
            .all(|other| prism.offset.distance(other.offset) > 1.7));
    }
    assert!(prisms.first().expect("center first").offset.length() < SKIN);
    assert!(
        (actor.body_dimensions() - Vec3::new(hex_core::config::HEX_SMALL_DIAMETER * 3.0, 2.0, 5.0))
            .length()
            < SKIN
    );
    actor.aim = Vec3::Z;
    assert!(
        actor
            .body_rotation()
            .angle_between(bevy_math::Quat::IDENTITY)
            < SKIN
    );
    assert!((actor.max_hp - tuning.golem_hp).abs() < SKIN && actor.beam().is_none());
    for species in [
        Species::Human,
        Species::Shadow,
        Species::Dragon,
        Species::Goblin,
        Species::Shaman,
    ] {
        let mut old = Actor::spawn(1, Vec3::ZERO, Vec3::X);
        old.configure_species(species, &tuning);
        assert_eq!(old.body_hex_prisms().count(), 0);
    }
    assert_eq!(CreatureAbility::Aura.index(), 6);
    assert_eq!(CreatureAbility::GolemSlam.index(), 7);
    assert_eq!(
        CreatureAbility::GolemLaser.index() + 1,
        CREATURE_ABILITY_COUNT
    );
}

#[test]
fn incomplete_golem_has_a_typed_atomic_refusal_in_both_control_modes() {
    let (mut session, view, geometry, materials, tuning) = fixture(ArenaEncounter::Dragon);
    for control in [ArenaControl::Spectator, ArenaControl::Player] {
        let setup = if control == ArenaControl::Spectator {
            ArenaBattleSetup::spectator(BattlePreset::Golem, BattlePreset::Shadow, 1)
        } else {
            ArenaBattleSetup {
                player_recipe: Some(BattlePreset::Golem),
                ..Default::default()
            }
        };
        assert_eq!(
            setup.validate_for(ArenaMap::Fort),
            Err(BattleSetupError::CreatureNotReady)
        );
        session.reset_with_setup(2, &view, geometry, &setup);
        let before = session.tick;
        session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
        assert!(session.is_finished() && session.actors.is_empty());
        assert_eq!(session.tick, before);
        assert!(matches!(
            session.battle_result,
            Some(BattleResult::InvalidSetup(_))
        ));
    }
}

#[test]
fn fort_player_override_is_reset_owned_and_none_restores_the_world_recipe() {
    let (mut session, view, geometry, materials, tuning) = fixture(ArenaEncounter::Dragon);
    let setup = ArenaBattleSetup {
        player_recipe: Some(BattlePreset::Goblins),
        ..Default::default()
    };
    assert_eq!(
        setup.validate_for(ArenaMap::Duel),
        Err(BattleSetupError::PlayerRecipeMap)
    );
    session.reset_with_setup(2, &view, geometry, &setup);
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    assert_eq!(session.actors.len(), 6);
    assert!(session
        .actors
        .iter()
        .skip(1)
        .all(|a| a.species == Species::Goblin));
    assert_eq!(view.selection.encounter, ArenaEncounter::Dragon);
    session.reset_with_setup(3, &view, geometry, &ArenaBattleSetup::default());
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    assert_eq!(session.actors.len(), 2);
    assert_eq!(
        session.actors.get(1).expect("world recipe").species,
        Species::Dragon
    );
    let serialized = ron::to_string(&setup).expect("setup roundtrip");
    assert_eq!(
        ron::from_str::<ArenaBattleSetup>(&serialized).expect("decode"),
        setup
    );
    let legacy = "(control:Player,rosters:[],seed:1,tick_limit:None)";
    assert!(ron::from_str::<ArenaBattleSetup>(legacy)
        .expect("old request")
        .player_recipe
        .is_none());
    assert_eq!(BattlePreset::ALL.len(), 5);
    assert!(!BattlePreset::ORIGINAL.contains(&BattlePreset::Golem));
}
