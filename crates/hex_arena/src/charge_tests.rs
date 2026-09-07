//! Charges exercise actual releases and flights, not just a presentation timer.

use super::*;
use hex_core::{ElementId, HexCoord, SubstanceId};

struct Fixture {
    session: ArenaSession,
    world: ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    materials: ArenaMaterials,
    tuning: ArenaTuning,
}

impl Fixture {
    fn new() -> Self {
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
            spawns: [Vec3::new(-6.0, SKIN, 0.0), Vec3::new(8.0, SKIN, 0.0)],
        };
        let mut session = ArenaSession {
            bot_enabled: false,
            ..Default::default()
        };
        session.reset(0, &world, geometry);
        Self {
            session,
            world,
            geometry,
            materials,
            tuning: ArenaTuning::default(),
        }
    }

    fn actor(&self) -> &Actor {
        self.session.actors.first().expect("human actor")
    }

    fn tick(&mut self, intent: ActorIntent) -> CommandsOut {
        self.session.advance(
            intent,
            &self.world,
            self.geometry,
            self.materials,
            &self.tuning,
        )
    }

    fn hold(&mut self, ticks: u32, spell: Spell, aim: Vec3) {
        for tick in 0..ticks {
            self.tick(ActorIntent {
                aim,
                selected: Some(spell),
                cast_pressed: tick == 0,
                cast_held: true,
                ..Default::default()
            });
        }
    }

    fn release(&mut self, aim: Vec3) -> CommandsOut {
        self.tick(ActorIntent {
            aim,
            cast_released: true,
            ..Default::default()
        })
    }
}

#[test]
fn tap_half_and_full_charge_match_reference_range_in_actual_flights() {
    let aim = (Vec3::X + Vec3::Y).normalize();
    for (ticks, ratio) in [(0, 1.0 / 3.0), (45, (1.0 / 3.0 + 1.3) * 0.5), (90, 1.3)] {
        let mut f = Fixture::new();
        if ticks == 0 {
            f.tick(ActorIntent {
                aim,
                cast_pressed: true,
                cast_released: true,
                ..Default::default()
            });
        } else {
            f.hold(ticks, Spell::Fireball, aim);
            assert!(f.session.projectiles.is_empty());
            assert!(f.actor().cooldowns.iter().all(|v| v.abs() < SKIN));
            f.release(aim);
        }
        let shot = f
            .session
            .projectiles
            .first()
            .expect("one released projectile");
        let origin = shot.position;
        let mut previous = origin;
        let mut range = None;
        for _ in 0..900 {
            f.tick(ActorIntent::default());
            let point = f
                .session
                .projectiles
                .first()
                .expect("flight survives until reference height")
                .position;
            if point.y <= origin.y && previous.y > origin.y {
                let fraction = (previous.y - origin.y) / (previous.y - point.y);
                range = Some(previous.lerp(point, fraction).x - origin.x);
                break;
            }
            previous = point;
        }
        let expected = f.tuning.projectile_speed.powi(2) / f.tuning.projectile_gravity * ratio;
        assert!((range.expect("descending crossing") - expected).abs() < 0.025);
        assert_eq!(f.session.projectiles.len(), 1);
    }
}

#[test]
fn maximum_charge_waits_for_release_then_uses_current_aim_once() {
    let mut f = Fixture::new();
    f.hold(300, Spell::Fireball, Vec3::Y);
    assert!(f.session.projectiles.is_empty());
    assert!(
        (f.actor().charge().expect("held spell").elapsed - f.tuning.charge_seconds).abs() < SKIN
    );
    assert!(f.actor().cooldowns.iter().all(|v| v.abs() < SKIN));
    f.release(Vec3::X);
    let shot = f.session.projectiles.first().expect("release fires");
    assert!(
        shot.velocity
            .distance(Vec3::X * f.tuning.launch_speed(f.tuning.charge_seconds))
            < 0.001
    );
    assert!(f.actor().charge().is_none());
    assert!((f.actor().cooldowns.get(1).copied().unwrap_or_default() - 1.25).abs() < SKIN);
    f.release(Vec3::X);
    assert_eq!(f.session.projectiles.len(), 1);
}

#[test]
fn both_actor_identities_reach_full_charge_in_exactly_ninety_fixed_ticks() {
    let tuning = ArenaTuning::default();
    assert!((tuning.charge_seconds - 0.75).abs() < SKIN);
    for id in [0, 1] {
        let mut actor = Actor::spawn(id, Vec3::ZERO, Vec3::X);
        for tick in 0..90 {
            assert!(actor
                .casting(
                    ActorIntent {
                        cast_pressed: tick == 0,
                        cast_held: true,
                        ..Default::default()
                    },
                    &tuning
                )
                .is_none());
            if tick == 88 {
                assert!(actor.charge().expect("held").elapsed < tuning.charge_seconds);
            }
        }
        assert!((actor.charge().expect("maximum").elapsed - tuning.charge_seconds).abs() < SKIN);
        let (_, speed) = actor
            .casting(
                ActorIntent {
                    cast_released: true,
                    ..Default::default()
                },
                &tuning,
            )
            .expect("release");
        assert!((speed - 32.0 * 1.3_f32.sqrt()).abs() < SKIN);
    }
}

#[test]
fn cooldown_press_does_not_arm_later_when_held_until_ready() {
    let mut f = Fixture::new();
    f.session.actors.first_mut().expect("human").cooldowns = [0.0, 0.2, 0.0];
    f.hold(90, Spell::Fireball, Vec3::Y);
    assert!(f.actor().charge().is_none());
    f.release(Vec3::Y);
    assert!(f.session.projectiles.is_empty());
    f.tick(ActorIntent {
        aim: Vec3::Y,
        cast_pressed: true,
        cast_released: true,
        ..Default::default()
    });
    assert_eq!(f.session.projectiles.len(), 1);
}

#[test]
fn cancellation_without_a_physics_tick_requires_fresh_press_and_spends_no_cooldown() {
    let mut f = Fixture::new();
    f.hold(45, Spell::Fireball, Vec3::Y);
    let tick = f.session.tick;
    f.session.cancel_charges();
    assert_eq!(f.session.tick, tick);
    assert!(f.actor().charge().is_none());
    for _ in 0..30 {
        f.tick(ActorIntent {
            cast_held: true,
            ..Default::default()
        });
    }
    f.release(Vec3::Y);
    assert!(f.session.projectiles.is_empty());
    assert!(f.actor().cooldowns.iter().all(|v| v.abs() < SKIN));
    f.hold(1, Spell::Fireball, Vec3::Y);
    f.release(Vec3::Y);
    assert_eq!(f.session.projectiles.len(), 1);
}

#[test]
fn switching_spell_cancels_the_hold_and_area_blast_still_requires_release() {
    let mut f = Fixture::new();
    f.hold(30, Spell::Fireball, Vec3::Y);
    f.tick(ActorIntent {
        selected: Some(Spell::AreaBlast),
        cast_held: true,
        ..Default::default()
    });
    assert!(f.actor().charge().is_none());
    assert!(f.release(Vec3::Y).impacts.is_empty());
    assert!(f.session.projectiles.is_empty());
    f.hold(240, Spell::AreaBlast, Vec3::Y);
    assert!(f.session.effects.is_empty());
    let out = f.release(Vec3::Y);
    assert_eq!(out.impacts.len(), 1);
    let effect = f.session.effects.first().expect("fixed blast effect");
    assert_eq!(effect.kind, Spell::AreaBlast);
    assert!((effect.radius - f.tuning.blast_radius()).abs() < SKIN);
    assert!((f.actor().hp - 100.0).abs() < SKIN);
    assert!((f.actor().cooldowns.get(2).copied().unwrap_or_default() - 7.0).abs() < SKIN);
}

#[test]
fn charged_preview_and_release_reach_the_same_impact() {
    let mut f = Fixture::new();
    let aim = Vec3::new(1.0, -0.15, 0.0).normalize();
    f.hold(75, Spell::Fireball, aim);
    let forecast = preview(&f.session, &f.world, &f.geometry, &f.tuning)
        .impact
        .expect("predicted ground hit");
    f.release(aim);
    for _ in 0..240 {
        f.tick(ActorIntent {
            aim,
            ..Default::default()
        });
        if let Some(effect) = f.session.effects.first() {
            assert!(effect.center.distance(forecast) < 0.001);
            return;
        }
    }
    panic!("charged shot must hit the predicted surface");
}

#[test]
fn reset_and_knockout_clear_armed_spells() {
    let mut f = Fixture::new();
    f.hold(15, Spell::Fireball, Vec3::Y);
    f.session.reset(1, &f.world, f.geometry);
    assert!(f.actor().charge().is_none());
    f.hold(15, Spell::Shield, Vec3::Y);
    f.session.actors.first_mut().expect("human").hp = 0.0;
    f.tick(ActorIntent {
        cast_held: true,
        ..Default::default()
    });
    assert!(f.session.outcome.is_some());
    assert!(f
        .session
        .actors
        .iter()
        .all(|actor| actor.charge().is_none()));
    assert!(f.session.projectiles.is_empty());
}

#[test]
fn charge_tuning_rejects_invalid_values_and_reference_duration_is_consistent() {
    let tuning = ArenaTuning::default();
    assert!((tuning.reference_charge_seconds() - 0.5172414).abs() < 0.0001);
    assert!((tuning.launch_speed(tuning.reference_charge_seconds()) - 32.0).abs() < 0.001);
    for bad in [f32::NAN, f32::INFINITY, -1.0, 0.0] {
        assert!(ArenaTuning {
            charge_seconds: bad,
            ..tuning.clone()
        }
        .validate()
        .is_err());
        assert!(ArenaTuning {
            tap_range_multiplier: bad,
            ..tuning.clone()
        }
        .validate()
        .is_err());
        assert!(ArenaTuning {
            max_range_multiplier: bad,
            ..tuning.clone()
        }
        .validate()
        .is_err());
    }
    assert!(ArenaTuning {
        tap_range_multiplier: 2.0,
        ..tuning.clone()
    }
    .validate()
    .is_err());
    assert!(ArenaTuning {
        shield_push: f32::NAN,
        ..tuning.clone()
    }
    .validate()
    .is_err());
    assert!(ArenaTuning {
        shield_push: 0.0,
        ..tuning
    }
    .validate()
    .is_ok());
}

#[test]
fn lethal_incoming_fireball_cancels_queued_area_blast_without_spending_cooldown() {
    let mut f = Fixture::new();
    let human_feet = f.actor().feet;
    f.session.actors.first_mut().expect("human").hp = 1.0;
    {
        let bot = f
            .session
            .actors
            .iter_mut()
            .find(|actor| actor.id == 1)
            .expect("bot");
        // Outside fireball splash, but inside the human's Area Blast. An
        // incorrectly admitted retaliatory blast would turn this win into a draw.
        bot.feet = human_feet + Vec3::X * 3.5;
        bot.previous_feet = bot.feet;
        bot.aim = Vec3::NEG_X;
        bot.hp = 1.0;
    }
    f.hold(1, Spell::AreaBlast, Vec3::X);
    assert!(f.actor().charge().is_some());
    let mut launch = CommandsOut::default();
    f.session.release(
        1,
        Spell::Fireball,
        &f.tuning,
        f.tuning.projectile_speed,
        &f.world,
        f.geometry,
        f.materials,
        &mut launch,
    );
    assert!(launch.impacts.is_empty());
    let incoming_position = f.actor().eye() + Vec3::X * 0.4;
    let incoming = f
        .session
        .projectiles
        .first_mut()
        .expect("real bot projectile");
    incoming.position = incoming_position;
    incoming.previous_position = incoming_position;
    incoming.velocity = Vec3::NEG_X * f.tuning.projectile_speed;

    let out = f.release(Vec3::X);
    assert_eq!(f.session.outcome, Some(ArenaOutcome::Winner(1)));
    assert!(f.actor().hp <= 0.0);
    assert!(f.actor().charge().is_none());
    assert!(f
        .actor()
        .cooldowns
        .get(Spell::AreaBlast.index())
        .is_some_and(|value| value.abs() < SKIN));
    assert_eq!(
        out.impacts.len(),
        1,
        "only the incoming fireball may explode"
    );
    assert_eq!(f.session.effects.len(), 1);
    assert_eq!(
        f.session.effects.first().expect("incoming impact").kind,
        Spell::Fireball
    );
    let bot = f
        .session
        .actors
        .iter()
        .find(|actor| actor.id == 1)
        .expect("surviving bot");
    assert!((bot.hp - 1.0).abs() < SKIN);
    assert!(f.session.projectiles.is_empty());
}
