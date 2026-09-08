//! Frozen Ember parameters exercised by the shared production projectile sweep.
use super::*;
use crate::{CreatureAbility, ProjectileAppearance, Species};
use hex_core::{ElementId, SubstanceId, TerrainDamageKind};

fn fixture() -> (
    ArenaSession,
    ArenaTerrainView,
    ArenaVoxelGeometry,
    ArenaMaterials,
    CreatureProjectileSpec,
) {
    let geometry = ArenaVoxelGeometry {
        radius: 20,
        ..Default::default()
    };
    let view = ArenaTerrainView {
        voxels: HexCoord::ORIGIN
            .within_radius(20)
            .into_iter()
            .map(|coord| (TilePos::new(coord, 0), SubstanceId(1)))
            .collect(),
        ..Default::default()
    };
    let materials = ArenaMaterials {
        stone: SubstanceId(1),
        grass: SubstanceId(2),
        dirt: SubstanceId(3),
        bedrock: SubstanceId(4),
        fire: ElementId(9),
    };
    let tuning = ArenaTuning::default();
    let mut owner = Actor::spawn(7, Vec3::new(-8.0, 4.0, 0.0), Vec3::X);
    owner.configure_species(Species::Wisp, &tuning.encounters);
    owner.team = 42;
    let mut target = Actor::spawn(9, Vec3::new(8.0, 4.0, 0.0), Vec3::NEG_X);
    target.configure_species(Species::Goblin, &tuning.encounters);
    target.team = 7;
    let mut session = ArenaSession {
        actors: vec![owner, target],
        ..Default::default()
    };
    session.collision.refresh(&view, geometry);
    session.encounter.initialized = true;
    let spec = CreatureProjectileSpec {
        ability: CreatureAbility::WispEmber,
        appearance: ProjectileAppearance::Ember,
        speed: 32.0,
        gravity: 2.0,
        collision_radius: 0.06,
        splash_radius: 0.8,
        damage: 8.0,
        knockback: 1.5,
        terrain_kind: TerrainDamageKind::Elemental(materials.fire),
        terrain_power: 1,
    };
    (session, view, geometry, materials, spec)
}

#[test]
fn ember_forecast_matches_actual_sweep_and_does_not_count_as_hotbar_fireball() {
    let (mut session, view, geometry, materials, spec) = fixture();
    let owner = session.actors.first().expect("owner").clone();
    let target = session.actors.get(1).expect("target").clone();
    let (aim, _) = crate::bot::ballistic_aim_with_gravity(
        owner.eye(),
        target.center(),
        spec.gravity,
        spec.speed,
    )
    .expect("arc");
    let fact = ForecastBody {
        id: target.id,
        feet: target.feet,
        velocity: Vec3::ZERO,
        predict_seconds: 0.5,
        species: target.species,
        team: target.team,
        dimensions: target.dimensions,
        yaw: target.body_yaw,
        yaw_velocity: 0.0,
        prisms: None,
    };
    let forecast = forecast_creature_projectile(
        &owner,
        aim,
        spec,
        &[fact],
        &session.collision,
        &view,
        geometry,
    )
    .impact
    .expect("predicted hit");
    assert_eq!(forecast.actor, Some(target.id));
    session.release_creature_projectile(&owner, aim, spec);
    for _ in 0..120 {
        session.advance_projectiles(&view, geometry, materials, &mut CommandsOut::default());
        if session.projectiles.is_empty() {
            break;
        }
    }
    let effect = session.effects.last().expect("real impact");
    assert!(effect.center.distance(forecast.point) < SKIN);
    assert!(session.actors.get(1).expect("target").hp < target.hp - 6.0);
    let stats = session
        .encounter
        .stats
        .get(&owner.id)
        .expect("damage accounting");
    assert_eq!(stats.casts, [0; 3]);
    assert_eq!(stats.fireballs_resolved, 0);
}

#[test]
fn frozen_ember_keeps_gravity_team_damage_and_appearance_after_owner_death() {
    let (mut session, view, geometry, materials, mut spec) = fixture();
    let owner = session.actors.first().expect("owner").clone();
    let target = session.actors.get(1).expect("target").clone();
    spec.damage *= 1.25;
    let (aim, _) = crate::bot::ballistic_aim_with_gravity(
        owner.eye(),
        target.center(),
        spec.gravity,
        spec.speed,
    )
    .expect("arc");
    session.release_creature_projectile(&owner, aim, spec);
    let shot = session.projectiles.first().expect("shot");
    assert_eq!(shot.appearance(), ProjectileAppearance::Ember);
    assert_eq!(shot.source_ability(), Some(CreatureAbility::WispEmber));
    assert_eq!(shot.source_team(), 42);
    let start = shot.position;
    let velocity = shot.velocity;
    session.actors.first_mut().expect("owner").hp = 0.0;
    session.actors.first_mut().expect("owner").team = 7;
    session.advance_projectiles(&view, geometry, materials, &mut CommandsOut::default());
    let shot = session.projectiles.first().expect("moving shot");
    assert!(
        shot.position
            .distance(start + velocity * STEP - Vec3::Y * STEP * STEP)
            < SKIN
    );
    for _ in 0..120 {
        session.advance_projectiles(&view, geometry, materials, &mut CommandsOut::default());
    }
    assert!(session.actors.get(1).expect("target").hp < target.hp - 8.0);
}

#[test]
fn ember_passes_allied_prism_and_splash_cannot_hurt_or_push_it() {
    let (mut session, view, geometry, materials, spec) = fixture();
    let owner = session.actors.first().expect("owner").clone();
    let target = session.actors.get(1).expect("target").clone();
    let mut ally = owner.clone();
    ally.id = 8;
    ally.feet = Vec3::new(0.0, 4.0, 0.0);
    ally.previous_feet = ally.feet;
    let before = ally.hp;
    session.actors.push(ally);
    let (aim, _) = crate::bot::ballistic_aim_with_gravity(
        owner.eye(),
        target.center(),
        spec.gravity,
        spec.speed,
    )
    .expect("arc");
    session.release_creature_projectile(&owner, aim, spec);
    for _ in 0..120 {
        session.advance_projectiles(&view, geometry, materials, &mut CommandsOut::default());
    }
    assert!(session.actors.get(1).expect("hostile").hp < target.hp);
    let ally = session.actors.last().expect("ally");
    assert!((ally.hp - before).abs() < SKIN && ally.body.impulse_velocity.length() < SKIN);
    // Move a newly released genuine shot into the allied body: its splash still
    // ignores that ally, while caster self-damage remains ordinary radial damage.
    session.actors.last_mut().expect("ally").feet = owner.feet;
    session.explode(
        owner.center(),
        owner.id,
        owner.team,
        Spell::Fireball,
        spec.splash_radius,
        spec.damage,
        spec.knockback,
        spec.terrain_power,
        Some(spec.terrain_kind),
        false,
        &view,
        geometry,
        materials,
        &mut CommandsOut::default(),
    );
    assert!(session.actors.first().expect("caster").hp < owner.hp);
    assert!((session.actors.last().expect("ally").hp - before).abs() < SKIN);
}

#[test]
fn ember_terrain_admission_uses_the_release_element_even_if_material_mapping_changes() {
    let (mut session, view, geometry, mut materials, spec) = fixture();
    let mut owner = session.actors.first().expect("owner").clone();
    owner.feet = Vec3::new(-8.0, 1.0, 0.0);
    session.actors.first_mut().expect("owner").feet = owner.feet;
    session.actors.first_mut().expect("owner").previous_feet = owner.feet;
    session.release_creature_projectile(&owner, Vec3::new(1.0, -0.5, 0.0).normalize(), spec);
    materials.fire = ElementId(99);
    let mut out = CommandsOut::default();
    for _ in 0..120 {
        session.advance_projectiles(&view, geometry, materials, &mut out);
    }
    assert!(!out.impacts.is_empty());
    assert!(out
        .impacts
        .iter()
        .all(|i| i.kind == spec.terrain_kind && i.power == spec.terrain_power));
}
