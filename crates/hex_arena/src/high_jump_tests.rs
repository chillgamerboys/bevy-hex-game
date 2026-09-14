//! Shared boost authority: motion, admission and independent projectile charging.
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
    let world = ArenaTerrainView {
        revision: 1,
        voxels: HexCoord::ORIGIN
            .within_radius(12)
            .into_iter()
            .map(|coord| (TilePos::new(coord, 0), materials.stone))
            .collect(),
        spawns: [Vec3::Y * SKIN, Vec3::new(15.0, SKIN, 0.0)],
        ..Default::default()
    };
    let mut session = ArenaSession {
        bot_enabled: false,
        ..Default::default()
    };
    session.reset(0, &world, geometry);
    (session, world, geometry, materials, ArenaTuning::default())
}

#[test]
fn high_jump_reaches_configured_height_using_shared_gravity() {
    for height in [2.0, 4.0, 8.0] {
        let (mut session, world, geometry, materials, mut tuning) = fixture();
        tuning.high_jump_height = height;
        let mut maximum = 0.0_f32;
        for tick in 0..300 {
            let out = session.advance(
                ActorIntent {
                    high_jump: tick == 0,
                    jump: tick <= 1,
                    ..Default::default()
                },
                &world,
                geometry,
                materials,
                &tuning,
            );
            assert!(out.impacts.is_empty() && out.edits.is_empty());
            let human = session.actors.first().expect("human");
            maximum = maximum.max(human.feet.y);
            assert_eq!(human.selected, Spell::Fireball);
            assert!((human.hp - 100.0).abs() < SKIN);
        }
        assert!(
            (maximum - height).abs() < 0.003,
            "height={height} apex={maximum}"
        );
        assert_eq!(
            session
                .round_summary()
                .actors
                .first()
                .expect("human stats")
                .casts,
            [0, 0, 1]
        );
    }
}

#[test]
fn high_jump_reverses_falls_preserves_fast_ascent_and_horizontal_knockback() {
    for vertical in [-30.0, 0.0, 20.0] {
        let (mut session, _, _, _, tuning) = fixture();
        for actor in &mut session.actors {
            actor.body.vertical_velocity = vertical;
            actor.body.impulse_velocity = Vec3::new(3.0, -2.0, 4.0);
            assert!(
                actor.high_jump(&tuning),
                "both human and Shadow share admission"
            );
            assert!(actor.body.vertical_velocity > 11.0);
            if vertical > 0.0 {
                assert!((actor.body.vertical_velocity - 18.0).abs() < SKIN);
            }
            assert_eq!(actor.body.impulse_velocity, Vec3::new(3.0, 0.0, 4.0));
            assert!(!actor.grounded);
            assert!(!actor.high_jump(&tuning), "cooldown prevents stacking");
            *actor.cooldowns.get_mut(2).expect("High Jump slot") = 0.0;
            actor.body.vertical_velocity = -5.0;
            assert!(
                actor.high_jump(&tuning),
                "no extra airborne-use restriction"
            );
        }
    }
}

#[test]
fn high_jump_and_projectile_release_share_a_tick_without_changing_selection() {
    for spell in [Spell::Shield, Spell::Fireball] {
        let (mut session, world, geometry, materials, tuning) = fixture();
        for tick in 0..30 {
            session.advance(
                ActorIntent {
                    selected: Some(spell),
                    cast_pressed: tick == 0,
                    cast_held: true,
                    aim: Vec3::Y,
                    ..Default::default()
                },
                &world,
                geometry,
                materials,
                &tuning,
            );
        }
        let out = session.advance(
            ActorIntent {
                high_jump: true,
                cast_released: true,
                aim: Vec3::Y,
                ..Default::default()
            },
            &world,
            geometry,
            materials,
            &tuning,
        );
        assert!(out.impacts.is_empty() && out.edits.is_empty());
        let actor = session.actors.first().expect("human");
        assert_eq!(actor.selected, spell);
        assert!(actor.feet.y > 0.09, "boost precedes movement");
        assert!(actor.charge().is_none());
        let shot = session
            .projectiles
            .first()
            .expect("same-tick ordinary projectile");
        assert_eq!(shot.spell, spell);
        assert!(shot.position.distance(actor.eye()) < SKIN);
        assert!(actor.cooldowns.get(spell.index()).is_some_and(|v| *v > 0.0));
        assert!(actor.cooldowns.get(2).is_some_and(|v| *v > 0.0));
    }
}

#[test]
fn high_jump_ceiling_blocks_motion_but_spends_cooldown_and_never_changes_terrain() {
    let (mut session, mut world, geometry, materials, tuning) = fixture();
    for coord in HexCoord::ORIGIN.within_radius(3) {
        world.voxels.insert(TilePos::new(coord, 4), materials.stone);
    }
    world.revision += 1;
    let original = world.voxels.clone();
    for tick in 0..120 {
        let out = session.advance(
            ActorIntent {
                high_jump: tick == 0,
                ..Default::default()
            },
            &world,
            geometry,
            materials,
            &tuning,
        );
        assert!(out.impacts.is_empty() && out.edits.is_empty());
        assert!(session.actors.first().expect("human").feet.y < 0.401);
    }
    assert_eq!(world.voxels, original);
    assert!(session
        .actors
        .first()
        .expect("human")
        .cooldowns
        .get(2)
        .is_some_and(|v| *v > 5.9));
}

#[test]
fn observed_high_jump_velocity_is_not_flattened_to_walk_speed() {
    let velocity = targeting::observed_velocity(Vec3::new(7.0, 16.0, 0.0));
    assert_eq!(velocity, Vec3::new(7.0, 16.0, 0.0));
}
