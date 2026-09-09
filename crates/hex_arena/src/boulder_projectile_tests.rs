//! Caster immunity follows the released Boulder, never the current actor species.
use super::*;
use crate::{CreatureAbility, ProjectileAppearance, Species};
use hex_core::{ElementId, SubstanceId, TerrainDamageKind};

fn returning_source(source: Option<CreatureAbility>) -> (Actor, Actor, Actor) {
    let tuning = ArenaTuning::default();
    let geometry = ArenaVoxelGeometry::default();
    let view = ArenaTerrainView::default();
    let materials = ArenaMaterials {
        stone: SubstanceId(1),
        grass: SubstanceId(2),
        dirt: SubstanceId(3),
        bedrock: SubstanceId(4),
        fire: ElementId(1),
    };
    let mut owner = Actor::spawn(7, Vec3::new(-8.0, 4.0, 0.0), Vec3::X);
    owner.configure_species(
        if source == Some(CreatureAbility::WispEmber) {
            Species::Wisp
        } else {
            Species::Worm
        },
        &tuning.encounters,
    );
    owner.team = 42;
    let mut session = ArenaSession {
        actors: vec![owner.clone()],
        ..Default::default()
    };
    session.collision.refresh(&view, geometry);
    if let Some(ability) = source {
        let c = &tuning.encounters;
        let spec = CreatureProjectileSpec {
            ability,
            appearance: if ability == CreatureAbility::WormBoulder {
                ProjectileAppearance::Boulder
            } else {
                ProjectileAppearance::Ember
            },
            speed: c.worm_boulder_speed,
            gravity: c.worm_boulder_gravity,
            collision_radius: c.worm_boulder_collision_radius,
            splash_radius: c.worm_boulder_radius,
            damage: c.worm_boulder_damage,
            knockback: c.worm_boulder_knockback,
            terrain_kind: TerrainDamageKind::Physical,
            terrain_power: c.worm_boulder_terrain_power,
        };
        session.release_creature_projectile(&owner, Vec3::X, spec);
    } else {
        session.release(
            owner.id,
            Spell::Fireball,
            &tuning,
            tuning.projectile_speed,
            &view,
            geometry,
            materials,
            &mut CommandsOut::default(),
        );
    }
    for _ in 0..60 {
        session.advance_projectiles(&view, geometry, materials, &mut CommandsOut::default());
        if session
            .projectiles
            .first()
            .is_some_and(|shot| shot.owner_cleared)
        {
            break;
        }
    }
    let shot = session.projectiles.first().expect("outgoing projectile");
    assert!(shot.owner_cleared);
    assert_eq!(shot.source_ability(), source);
    assert_eq!(shot.source_team(), 42);
    let impact = shot.position;
    assert!(impact.distance(owner.eye()) > 1.0);
    // Move the caster into its genuine, already-cleared projectile. Replace its
    // live shape and allegiance to ensure neither supplies the frozen policy.
    let mut moved_owner = Actor::spawn(7, impact - Vec3::Y * 0.4, Vec3::X);
    moved_owner.team = 7;
    let mut hostile = Actor::spawn(9, moved_owner.feet + Vec3::Z * 0.6, Vec3::NEG_X);
    hostile.team = 7;
    let mut ally = Actor::spawn(8, moved_owner.feet - Vec3::Z * 0.6, Vec3::X);
    ally.team = 42;
    session.actors = vec![moved_owner, hostile, ally];
    session.advance_projectiles(&view, geometry, materials, &mut CommandsOut::default());
    assert!(
        session.projectiles.is_empty(),
        "owner body remains a real swept contact"
    );
    assert!(
        session
            .effects
            .last()
            .expect("radial impact")
            .center
            .distance(impact)
            < SKIN
    );
    let mut actors = session.actors.into_iter();
    (
        actors.next().expect("owner"),
        actors.next().expect("hostile"),
        actors.next().expect("ally"),
    )
}

#[test]
fn boulder_excludes_frozen_owner_from_hp_and_knockback_after_motion_and_shape_change() {
    let (owner, hostile, ally) = returning_source(Some(CreatureAbility::WormBoulder));
    assert_eq!(owner.hp.to_bits(), owner.max_hp.to_bits());
    assert!(owner.body.impulse_velocity.length() < SKIN);
    assert!(hostile.hp < hostile.max_hp && hostile.body.impulse_velocity.length() > 1.0);
    assert_eq!(ally.hp.to_bits(), ally.max_hp.to_bits());
    assert!(ally.body.impulse_velocity.length() < SKIN);
}

#[test]
fn ordinary_fireball_from_a_worm_still_damages_and_pushes_its_caster() {
    let (owner, hostile, ally) = returning_source(None);
    assert!(owner.hp < owner.max_hp && owner.body.impulse_velocity.length() > 1.0);
    assert!(hostile.hp < hostile.max_hp);
    assert_eq!(ally.hp.to_bits(), ally.max_hp.to_bits());
    assert!(ally.body.impulse_velocity.length() < SKIN);
}

#[test]
fn ember_source_does_not_inherit_boulder_caster_immunity() {
    let (owner, _, _) = returning_source(Some(CreatureAbility::WispEmber));
    assert!(owner.hp < owner.max_hp && owner.body.impulse_velocity.length() > 1.0);
}
