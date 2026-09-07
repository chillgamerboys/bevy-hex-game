//! Bot decisions exercised against the same actors, collision, and spell path as play.

use super::*;
use crate::{ArenaSession, CommandsOut};
use hex_core::arena::ArenaMaterials;
use hex_core::{ElementId, HexCoord, SubstanceId, TerrainEdit, TilePos};

struct Fixture {
    session: ArenaSession,
    world: ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    materials: ArenaMaterials,
    tuning: ArenaTuning,
}

impl Fixture {
    fn new(distance: f32) -> Self {
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
            spawns: [
                Vec3::new(distance * 0.5, SKIN, 0.0),
                Vec3::new(-distance * 0.5, SKIN, 0.0),
            ],
        };
        let mut session = ArenaSession::default();
        session.reset(0, &world, geometry);
        session.bot.release_ticks = 0;
        session.bot.think_ticks = 0;
        Self {
            session,
            world,
            geometry,
            materials,
            tuning: ArenaTuning::default(),
        }
    }

    fn actor(&self, id: u8) -> &Actor {
        self.session
            .actors
            .iter()
            .find(|actor| actor.id == id)
            .expect("fixture actor")
    }

    fn actor_mut(&mut self, id: u8) -> &mut Actor {
        self.session
            .actors
            .iter_mut()
            .find(|actor| actor.id == id)
            .expect("fixture actor")
    }

    fn refresh(&mut self) {
        self.world.revision += 1;
        self.session.collision.refresh(&self.world, self.geometry);
    }

    fn decide(&mut self) -> ActorIntent {
        self.session.bot.intent(
            &self.session.actors,
            &self.session.projectiles,
            &self.session.collision,
            &self.world,
            self.geometry,
            &self.tuning,
        )
    }

    fn advance(&mut self) -> CommandsOut {
        self.session.advance(
            ActorIntent::default(),
            &self.world,
            self.geometry,
            self.materials,
            &self.tuning,
        )
    }
}

#[test]
fn low_arc_reaches_level_elevated_and_vertical_targets_at_the_tuned_speed() {
    let origin = Vec3::new(-3.0, 2.0, 5.0);
    for (speed, gravity, delta) in [
        (32.0, 12.0, Vec3::new(28.0, 0.0, 0.0)),
        (24.0, 20.0, Vec3::new(8.0, 4.0, 3.0)),
        (24.0, 20.0, Vec3::new(8.0, -4.0, -3.0)),
        (64.0, 2.0, Vec3::new(45.0, 8.0, 4.0)),
        (32.0, 12.0, Vec3::Y * 4.0),
        (32.0, 12.0, Vec3::NEG_Y * 4.0),
    ] {
        let tuning = ArenaTuning {
            projectile_speed: speed,
            projectile_gravity: gravity,
            ..Default::default()
        };
        let target = origin + delta;
        let (aim, time) = ballistic_aim(origin, target, &tuning).expect("reachable low arc");
        let velocity = aim * speed;
        let endpoint = origin + velocity * time - Vec3::Y * (0.5 * gravity * time * time);
        assert!(aim.is_finite() && time > 0.0 && time < 2.0);
        assert!((velocity.length() - speed).abs() < 0.001);
        assert!(
            endpoint.distance(target) < 0.001,
            "missed {target:?} with {tuning:?}"
        );
    }
}

#[test]
fn ballistic_aim_rejects_impossible_expired_and_nonfinite_requests() {
    let tuning = ArenaTuning::default();
    for target in [
        Vec3::ZERO,
        Vec3::Y * 100.0,
        Vec3::X * 200.0,
        Vec3::splat(f32::NAN),
        Vec3::splat(f32::INFINITY),
    ] {
        assert!(ballistic_aim(Vec3::ZERO, target, &tuning).is_none());
    }
    assert!(ballistic_aim(Vec3::splat(f32::NAN), Vec3::X, &tuning).is_none());
    for (speed, gravity) in [
        (0.0, 12.0),
        (-1.0, 12.0),
        (f32::NAN, 12.0),
        (f32::INFINITY, 12.0),
        (32.0, 0.0),
        (32.0, -1.0),
        (32.0, f32::NAN),
        (32.0, f32::INFINITY),
    ] {
        let invalid = ArenaTuning {
            projectile_speed: speed,
            projectile_gravity: gravity,
            ..Default::default()
        };
        assert!(ballistic_aim(Vec3::ZERO, Vec3::X, &invalid).is_none());
    }
    let long_flight = ArenaTuning {
        projectile_speed: 64.0,
        projectile_gravity: 2.0,
        ..Default::default()
    };
    assert!(ballistic_aim(Vec3::ZERO, Vec3::X * 700.0, &long_flight).is_none());
}

#[test]
fn bot_fireball_actually_damages_stationary_targets_at_near_and_far_range() {
    for distance in [8.0, 28.0] {
        let mut fixture = Fixture::new(distance);
        fixture.advance();
        let shot = fixture
            .session
            .projectiles
            .first()
            .expect("bot launches fireball");
        assert_eq!(shot.owner, 1);
        assert_eq!(shot.spell, Spell::Fireball);
        assert!(shot.position.distance(fixture.actor(1).eye()) < SKIN);
        fixture.session.bot_enabled = false;
        let mut impacts = 0;
        for _ in 0..240 {
            impacts += fixture.advance().impacts.len();
            if fixture.actor(0).hp < 100.0 {
                break;
            }
        }
        assert!(
            fixture.actor(0).hp < 90.0,
            "bot missed target at {distance}"
        );
        assert!((fixture.actor(1).hp - 100.0).abs() < SKIN);
        assert!(fixture.session.projectiles.is_empty());
        assert_eq!(impacts, 1);
    }
}

#[test]
fn close_bot_uses_caster_safe_blast_and_does_not_replace_cooldown_with_suicidal_fire() {
    let mut fixture = Fixture::new(2.0);
    let first = fixture.advance();
    assert_eq!(fixture.actor(1).selected, Spell::AreaBlast);
    assert_eq!(first.impacts.len(), 1);
    assert!(fixture.actor(0).hp < 100.0);
    assert!((fixture.actor(1).hp - 100.0).abs() < SKIN);
    assert!(
        (fixture
            .actor(1)
            .cooldowns
            .get(2)
            .copied()
            .unwrap_or_default()
            - 7.0)
            .abs()
            < SKIN
    );
    assert!(fixture.session.projectiles.is_empty());
    fixture.session.bot.think_ticks = 0;
    fixture.session.bot.release_ticks = 0;
    let next = fixture.advance();
    assert!(next.impacts.is_empty() && fixture.session.projectiles.is_empty());
    assert!(fixture
        .actor(1)
        .cooldowns
        .get(1)
        .is_some_and(|cooldown| cooldown.abs() < SKIN));
}

#[test]
fn fully_hidden_target_does_not_update_aim_or_trigger_any_spell() {
    let mut fixture = Fixture::new(14.0);
    for coord in HexCoord::ORIGIN.within_radius(2) {
        for level in 1..=8 {
            fixture
                .world
                .voxels
                .insert(TilePos::new(coord, level), fixture.materials.stone);
        }
    }
    fixture.refresh();
    fixture.actor_mut(1).hp = 40.0; // Low HP must not reveal a hidden target through shield choice.
    let before = fixture.decide();
    assert!(!before.cast && before.selected.is_none());
    {
        let target = fixture.actor_mut(0);
        target.feet.z = 3.0;
        target.previous_feet = target.feet;
    }
    fixture.session.bot.think_ticks = 0;
    fixture.session.bot.release_ticks = 0;
    let hidden = fixture.decide();
    assert!(!hidden.cast && hidden.selected.is_none());
    assert!(hidden.aim.distance(before.aim) < SKIN);
    fixture.world.voxels.retain(|pos, _| pos.level == 0);
    fixture.refresh();
    fixture.actor_mut(1).hp = 100.0;
    fixture.session.bot.think_ticks = 0;
    let visible = fixture.decide();
    assert!(visible.cast);
    assert_eq!(visible.selected, Some(Spell::Fireball));
    assert!(visible.aim.distance(before.aim) > 0.05);
}

#[test]
fn defensive_shield_seed_produces_one_complete_supported_wall() {
    let mut fixture = Fixture::new(14.0);
    fixture.actor_mut(1).hp = 50.0;
    fixture.advance();
    let seed = fixture.session.projectiles.first().expect("defensive seed");
    assert_eq!(seed.owner, 1);
    assert_eq!(seed.spell, Spell::Shield);
    fixture.session.bot_enabled = false;
    let mut edits = Vec::new();
    for _ in 0..120 {
        edits.extend(fixture.advance().edits);
    }
    assert_eq!(fixture.session.shields_raised, 1);
    assert_eq!(edits.len(), 25);
    let mut positions = std::collections::BTreeSet::new();
    for edit in edits {
        let TerrainEdit::Set { pos, substance } = edit else {
            panic!("shield must set material")
        };
        assert_eq!(substance, fixture.materials.stone);
        assert!(!fixture.world.voxels.contains_key(&pos));
        positions.insert(pos);
    }
    assert_eq!(positions.len(), 25);
    assert!((fixture.actor(0).hp - 100.0).abs() < SKIN);
    assert!((fixture.actor(1).hp - 50.0).abs() < SKIN);
}

#[test]
fn unsupported_defensive_footprints_fall_back_without_spending_shield_cooldown() {
    let mut fixture = Fixture::new(14.0);
    fixture.actor_mut(1).hp = 50.0;
    fixture.world.voxels.retain(|pos, _| {
        let x = fixture.geometry.center(*pos).x;
        !(-5.7..=-1.0).contains(&x)
    });
    fixture.refresh();
    fixture.advance();
    assert_eq!(fixture.actor(1).selected, Spell::Fireball);
    assert!(fixture
        .actor(1)
        .cooldowns
        .first()
        .is_some_and(|cooldown| cooldown.abs() < SKIN));
    assert_eq!(
        fixture.session.projectiles.first().map(|shot| shot.spell),
        Some(Spell::Fireball)
    );
    assert!(fixture.session.pending_walls.is_empty());
    fixture.session.bot_enabled = false;
    for _ in 0..120 {
        fixture.advance();
        if fixture.actor(0).hp < 100.0 {
            break;
        }
    }
    assert!(fixture.actor(0).hp < 100.0);
    assert_eq!(fixture.session.shields_raised, 0);
}

#[test]
fn visible_target_beyond_low_overhang_does_not_provoke_nearby_self_splash() {
    let mut fixture = Fixture::new(20.0);
    fixture.world.voxels.insert(
        TilePos::new(HexCoord::from_axial(-5, 0), 3),
        fixture.materials.stone,
    );
    fixture.refresh();
    assert!(visible_from(
        &fixture.session.collision,
        fixture.actor(1).eye(),
        fixture.actor(0).center()
    ));
    let intent = fixture.decide();
    assert!(
        !intent.cast && intent.selected.is_none(),
        "nearby ballistic obstruction should suppress the shot"
    );
}

#[test]
fn reset_replays_the_same_seeded_bot_movement_and_cast_sequence() {
    let mut fixture = Fixture::new(14.0);
    fixture.session.bot = Bot::default();
    let record = |fixture: &mut Fixture| {
        (0..120)
            .map(|_| {
                fixture.advance();
                let actor = fixture.actor(1);
                (
                    actor.feet,
                    actor.aim,
                    actor.selected,
                    fixture
                        .session
                        .projectiles
                        .iter()
                        .map(|shot| (shot.owner, shot.spell, shot.position))
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>()
    };
    let first = record(&mut fixture);
    fixture.session.reset(1, &fixture.world, fixture.geometry);
    let second = record(&mut fixture);
    assert_eq!(first, second);
}

#[test]
fn bot_leads_and_damages_a_target_that_keeps_sprinting_sideways() {
    let mut fixture = Fixture::new(12.0);
    let moving = ActorIntent {
        aim: Vec3::X,
        movement: Vec2::X,
        run: true,
        ..Default::default()
    };
    let advance_moving = |fixture: &mut Fixture| {
        fixture.session.advance(
            moving,
            &fixture.world,
            fixture.geometry,
            fixture.materials,
            &fixture.tuning,
        )
    };
    // Establish an observed velocity through real movement before the bot thinks.
    fixture.session.bot_enabled = false;
    advance_moving(&mut fixture);
    fixture.session.bot_enabled = true;
    advance_moving(&mut fixture);
    let direct = (fixture.actor(0).center() - fixture.actor(1).eye()).normalize();
    let shot = fixture
        .session
        .projectiles
        .iter()
        .find(|shot| shot.owner == 1)
        .expect("bot releases a fireball at the moving target");
    assert_eq!(shot.spell, Spell::Fireball);
    assert!(
        shot.velocity.z > direct.z * fixture.tuning.projectile_speed + 2.0,
        "the shot must lead the observed sideways motion"
    );
    fixture.session.bot_enabled = false;
    for _ in 0..120 {
        advance_moving(&mut fixture);
        if fixture.actor(0).hp < 100.0 || fixture.session.projectiles.is_empty() {
            break;
        }
    }
    assert!(
        fixture.actor(0).feet.z > 1.0,
        "target must keep moving during flight"
    );
    assert!(
        fixture.actor(0).hp < 100.0,
        "the led fireball must damage the continuously moving target"
    );
    assert!((fixture.actor(1).hp - 100.0).abs() < SKIN);
}

#[test]
fn healthy_bot_shields_against_approaching_fireball_but_not_one_moving_away() {
    for approaching in [true, false] {
        let mut fixture = Fixture::new(10.0);
        fixture.session.bot_enabled = false;
        fixture.session.advance(
            ActorIntent {
                aim: if approaching { Vec3::NEG_X } else { Vec3::X },
                cast: true,
                selected: Some(Spell::Fireball),
                ..Default::default()
            },
            &fixture.world,
            fixture.geometry,
            fixture.materials,
            &fixture.tuning,
        );
        let incoming = fixture.session.projectiles.first().expect("human fireball");
        assert_eq!(incoming.owner, 0);
        assert!(incoming.position.distance(fixture.actor(1).center()) < 14.0);
        fixture.session.bot_enabled = true;
        fixture.advance();
        let response = fixture
            .session
            .projectiles
            .iter()
            .find(|shot| shot.owner == 1)
            .expect("healthy bot responds to the visible combat situation");
        assert_eq!(
            response.spell,
            if approaching {
                Spell::Shield
            } else {
                Spell::Fireball
            }
        );
        assert_eq!(fixture.actor(1).selected, response.spell);
        assert!(
            (fixture.actor(1).hp - 100.0).abs() < SKIN,
            "shield choice must come from the approaching projectile, not lost HP"
        );
        let shield_cooldown = fixture
            .actor(1)
            .cooldowns
            .first()
            .copied()
            .unwrap_or_default();
        assert_eq!(shield_cooldown > 0.0, approaching);
    }
}
