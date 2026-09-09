//! Bounded Worm schema, observation and complete physical admission contracts.

use super::*;
use crate::collision::SKIN;
use crate::spells::ForecastBody;
use crate::targeting::ObservedTarget;
use hex_core::arena::{ArenaMap, ArenaSelection};
use hex_core::{ElementId, SubstanceId};

fn worm(count: u8) -> Actor {
    let tuning = EncounterTuning {
        worm_segments: count,
        ..Default::default()
    };
    let mut actor = Actor::spawn(7, Vec3::new(2.0, 3.2, 4.0), Vec3::NEG_Z);
    actor.configure_species(Species::Worm, &tuning);
    actor
}

#[test]
fn worm_profiles_preserve_head_order_and_publish_the_offset_union_center() {
    for count in [4, 6] {
        let actor = worm(count);
        let parts: Vec<_> = actor.body_hex_prisms().collect();
        assert_eq!(parts.len(), usize::from(count));
        assert!(parts.first().expect("head").offset.length() < SKIN);
        for pair in parts.windows(2) {
            let [front, back] = pair else {
                panic!("component pair")
            };
            assert!(
                (front.offset.distance(back.offset) - hex_core::config::HEX_SMALL_DIAMETER).abs()
                    < SKIN
            );
            assert!(back.offset.z > front.offset.z);
        }
        assert!(parts.iter().all(|p| (p.height - 0.4).abs() < SKIN));
        assert!(actor.center().z > actor.feet.z + 2.5);
        assert!(actor.eye().distance(actor.feet + Vec3::Y * 0.2) < SKIN);
        assert!(
            actor
                .body_rotation()
                .angle_between(bevy_math::Quat::IDENTITY)
                < SKIN
        );
        assert_eq!(
            actor
                .body_prism_snapshot()
                .expect("copied geometry")
                .iter()
                .len(),
            parts.len()
        );
        assert!(actor
            .previous_body_hex_prisms()
            .zip(parts)
            .all(|(a, b)| a.offset == b.offset));
        assert!(!actor.worm().expect("phase projection").exposed);
        assert!((actor.hp - 320.0).abs() < SKIN);
    }
    let ordinary = Actor::spawn(3, Vec3::new(-0.0, 1.2, 2.3), Vec3::X);
    let expected = ordinary.feet + Vec3::Y * (BODY_HEIGHT * 0.5);
    assert_eq!(
        ordinary.center().to_array().map(f32::to_bits),
        expected.to_array().map(f32::to_bits)
    );
    assert!(ordinary.worm().is_none() && ordinary.body_prism_snapshot().is_none());
}

#[test]
fn copied_emerged_geometry_drives_center_and_distance_without_live_worm_lookup() {
    let mut live = worm(4);
    let mut parts: Vec<_> = live.body_hex_prisms().collect();
    for (index, part) in parts.iter_mut().enumerate() {
        part.offset.y = 1.2 - f32::from(u8::try_from(index).expect("bounded")) * 0.4;
    }
    let copied = BodyPrismSnapshot::try_from_parts(&parts).expect("finite geometry");
    live.set_observed_prisms(copied).expect("valid Worm copy");
    let fact = ForecastBody {
        id: live.id,
        feet: live.feet,
        velocity: Vec3::X * 2.0,
        predict_seconds: 0.5,
        species: Species::Worm,
        team: live.team,
        dimensions: live.dimensions,
        yaw: 0.0,
        yaw_velocity: 5.0,
        prisms: live.body_prism_snapshot(),
    };
    let observed = ObservedTarget {
        body: fact,
        tick: 10,
        sight_point: live.eye(),
    };
    assert!(fact.center().distance(live.center()) < SKIN);
    assert!((live.eye().y - live.feet.y - 1.4).abs() < SKIN);
    assert!((fact.center().y - live.feet.y - 0.8).abs() < SKIN);
    let tail = live.feet + parts.last().expect("tail").offset + Vec3::Y * 0.2;
    assert!(observed.distance(tail, 0.0) < SKIN);
    let forecast = fact.actor_at(0.5).expect("copied forecast body");
    assert!(forecast.center().distance(fact.center() + Vec3::X) < SKIN);
    assert!(forecast
        .body_hex_prisms()
        .zip(copied.iter())
        .all(|(a, b)| a.offset == b.offset));
    assert!(forecast
        .previous_body_hex_prisms()
        .zip(copied.iter())
        .all(|(a, b)| a.offset == b.offset));

    // A later unobserved shape change must not alter either copied endpoint.
    live.feet += Vec3::splat(40.0);
    live.set_observed_prisms(
        crate::worm_body::WormBodyState::straight(6, 1.0)
            .expect("new pose")
            .current,
    )
    .expect("new body");
    assert!(observed.distance(tail, 0.0) < SKIN);
    assert!(
        fact.reconstruct()
            .expect("dated copy")
            .center()
            .distance(observed.center())
            < SKIN
    );
    assert!(ForecastBody {
        prisms: None,
        ..fact
    }
    .reconstruct()
    .is_none());
    let bad_height = BodyPrismSnapshot::try_from_parts(
        &[BodyHexPrism {
            offset: Vec3::ZERO,
            height: 0.8,
        }; 4],
    )
    .expect("generic finite snapshot");
    assert!(ForecastBody {
        prisms: Some(bad_height),
        ..fact
    }
    .reconstruct()
    .is_none());
}

#[test]
fn worm_setup_is_typed_and_atomically_refused_until_runtime_admission_exists() {
    assert_eq!(CreatureAbility::WormBoulder.index(), 10);
    assert_eq!(CREATURE_ABILITY_COUNT, 12);
    assert_eq!(BattlePreset::Worm.members(), vec![Species::Worm]);
    assert_eq!(BattlePreset::from_slug("worm"), Some(BattlePreset::Worm));
    assert_eq!(BattlePreset::ORIGINAL.len(), 4);
    assert_eq!(BattlePreset::WISP_SWARMS.len(), 5);
    for preset in BattlePreset::ALL {
        assert!(ArenaBattleSetup::spectator(preset, BattlePreset::Shadow, 2)
            .validate_for(ArenaMap::Fort)
            .is_ok());
    }
    let view = ArenaTerrainView {
        selection: ArenaSelection {
            map: ArenaMap::Fort,
            ..Default::default()
        },
        ..Default::default()
    };
    let geometry = ArenaVoxelGeometry::default();
    let materials = ArenaMaterials {
        stone: SubstanceId(1),
        grass: SubstanceId(2),
        dirt: SubstanceId(3),
        bedrock: SubstanceId(4),
        fire: ElementId(1),
    };
    for setup in [
        ArenaBattleSetup::spectator(BattlePreset::Worm, BattlePreset::Shadow, 2),
        ArenaBattleSetup {
            player_recipe: Some(BattlePreset::Worm),
            ..Default::default()
        },
    ] {
        assert!(setup.validate_for(ArenaMap::Fort).is_ok());
        let mut session = ArenaSession::default();
        session.reset_with_setup(2, &view, geometry, &setup);
        session.advance(
            ActorIntent::default(),
            &view,
            geometry,
            materials,
            &ArenaTuning::default(),
        );
        assert!(
            session.is_finished() && session.actors.is_empty() && session.projectiles.is_empty()
        );
        assert!(matches!(
            session.battle_result,
            Some(BattleResult::InvalidSetup(_))
        ));
    }
}

#[test]
fn worm_configuration_rejects_unsupported_length_depth_and_nonfinite_boulders() {
    let valid = EncounterTuning::default();
    assert!(valid.validate().is_ok());
    assert!(EncounterTuning {
        worm_segments: 6,
        ..valid.clone()
    }
    .validate()
    .is_ok());
    for invalid in [
        EncounterTuning {
            worm_segments: 5,
            ..valid.clone()
        },
        EncounterTuning {
            worm_depth_levels: 3,
            ..valid.clone()
        },
        EncounterTuning {
            worm_boulder_gravity: f32::NAN,
            ..valid.clone()
        },
        EncounterTuning {
            worm_boulder_collision_radius: 0.6,
            ..valid.clone()
        },
        EncounterTuning {
            worm_boulder_terrain_power: 0,
            ..valid.clone()
        },
    ] {
        assert!(invalid.validate().is_err());
    }
}
