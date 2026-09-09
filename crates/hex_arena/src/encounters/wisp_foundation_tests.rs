//! Stable Wisp schema and exact body projection.

use super::*;

#[test]
fn wisp_is_one_native_prism_with_a_low_core_and_fixed_physical_orientation() {
    let (_, mut view, geometry, _, tuning) = fixture(ArenaEncounter::Dragon);
    let mut wisp = Actor::spawn(7, Vec3::Y * SKIN, Vec3::X);
    wisp.configure_species(Species::Wisp, &tuning.encounters);
    let parts: Vec<_> = wisp.body_hex_prisms().collect();
    assert_eq!(parts.len(), 1);
    let part = parts.first().expect("one physical voxel");
    assert!(part.offset.length() < SKIN && (part.height - 0.4).abs() < SKIN);
    assert!(
        wisp.body_dimensions()
            .distance(Vec3::new(hex_core::config::HEX_SMALL_DIAMETER, 0.4, 2.0))
            < SKIN
    );
    assert!(wisp.eye().distance(wisp.center()) < SKIN);
    wisp.aim = Vec3::NEG_Z;
    assert!(
        wisp.body_rotation()
            .angle_between(bevy_math::Quat::IDENTITY)
            < SKIN
    );
    assert!((shapes::distance(Vec3::Y * 0.6, &wisp) - 0.2).abs() < SKIN * 2.0);
    view.voxels
        .insert(TilePos::new(HexCoord::ORIGIN, 2), SubstanceId(1));
    let mut collision = CollisionWorld::default();
    collision.refresh(&view, geometry);
    assert!(shapes::clear(&collision, &wisp, wisp.feet, 0.0));
    assert!(!shapes::clear(
        &collision,
        &wisp,
        wisp.feet + Vec3::Y * 0.01,
        0.0
    ));
    let human = Actor::spawn(0, wisp.feet, Vec3::X);
    assert!(!shapes::clear(&collision, &human, human.feet, 0.0));
    assert!(tuning.encounters.wisp_hp < tuning.encounters.goblin_hp);
    assert!(tuning.encounters.wisp_flight_speed < tuning.encounters.goblin_walk);
}

#[test]
fn wisp_recipes_preserve_original_groups_and_require_authored_deployment() {
    let (mut session, view, geometry, materials, tuning) = fixture(ArenaEncounter::Dragon);
    assert_eq!(
        BattlePreset::ORIGINAL,
        [
            BattlePreset::Shadow,
            BattlePreset::Dragon,
            BattlePreset::Goblins,
            BattlePreset::ShamanParty
        ]
    );
    assert_eq!(BattlePreset::Goblin.members(), vec![Species::Goblin]);
    for (preset, count) in BattlePreset::WISP_SWARMS.into_iter().zip([1, 2, 4, 8, 12]) {
        assert_eq!(preset.members(), vec![Species::Wisp; count]);
        assert_eq!(BattlePreset::from_slug(preset.slug()), Some(preset));
        for setup in [
            ArenaBattleSetup::spectator(preset, preset, 1),
            ArenaBattleSetup {
                player_recipe: Some(preset),
                ..Default::default()
            },
        ] {
            assert_eq!(setup.validate_for(ArenaMap::Fort), Ok(()));
            session.reset_with_setup(2, &view, geometry, &setup);
            session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
            assert!(session.is_finished() && session.actors.is_empty());
        }
    }
    assert_eq!(
        BattlePreset::PLAYER
            .iter()
            .filter(|p| BattlePreset::WISP_SWARMS.contains(p))
            .count(),
        1
    );
    assert_eq!(CreatureAbility::GolemLaser.index(), 8);
    assert_eq!(CreatureAbility::WispEmber.index(), 9);
    assert_eq!(CreatureAbility::WormBoulder.index(), 10);
    assert_eq!(CREATURE_ABILITY_COUNT, 12);
}

#[test]
fn ordinary_releases_publish_their_unchanged_radius_and_spell_appearance() {
    let (mut session, view, geometry, materials, tuning) = fixture(ArenaEncounter::Dragon);
    for (spell, appearance) in [
        (Spell::Shield, ProjectileAppearance::ShieldSeed),
        (Spell::Fireball, ProjectileAppearance::Fireball),
    ] {
        session.release(
            0,
            spell,
            &tuning,
            tuning.projectile_speed,
            &view,
            geometry,
            materials,
            &mut CommandsOut::default(),
        );
        let shot = session.projectiles.last().expect("ordinary real release");
        assert_eq!(shot.appearance(), appearance);
        assert_eq!(shot.spell, spell);
        assert!(shot.source_ability().is_none());
        assert!((shot.collision_radius() - 0.06).abs() < SKIN);
    }
}
