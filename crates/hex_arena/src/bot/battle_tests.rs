//! Independent regressions for disclosed multi-body Shadow decisions.

use super::*;
use crate::{EncounterTuning, Species};

const CASTER: u8 = 7;
const TARGET: u8 = 8;

fn profile(id: u8, team: u8, species: Species, feet: Vec3) -> Actor {
    let tuning = EncounterTuning::default();
    let mut actor = Actor::spawn(id, feet, Vec3::NEG_X);
    actor.species = species;
    actor.team = team;
    actor.dimensions = match species {
        Species::Dragon => Vec3::new(
            tuning.dragon_width,
            tuning.dragon_height,
            tuning.dragon_length,
        ),
        Species::Goblin => Vec3::new(
            tuning.goblin_radius * 2.0,
            tuning.goblin_height,
            tuning.goblin_radius * 2.0,
        ),
        _ => actor.dimensions,
    };
    actor
}

fn battle_fixture(species: Species) -> (Fixture, Bot) {
    let mut fixture = Fixture::new(12.0);
    fixture.session.actors = vec![
        profile(CASTER, 7, Species::Shadow, Vec3::new(-6.0, SKIN, 0.0)),
        profile(TARGET, 42, species, Vec3::new(6.0, SKIN, 0.0)),
    ];
    fixture.tuning.bot.aim_error = 0.0;
    fixture.tuning.bot.ambush_chance = 0.0;
    fixture.tuning.bot.flank_enabled = false;
    let mut bot = Bot::with_seed(0x5eed);
    bot.release_ticks = 0;
    bot.think_ticks = 0;
    (fixture, bot)
}

fn decide_battle(fixture: &Fixture, bot: &mut Bot, tick: u64) -> ActorIntent {
    bot.sense_ticks = 0;
    bot.intent_battle(
        CASTER,
        Vec3::new(6.0, SKIN, 0.0),
        &fixture.session.actors,
        &fixture.session.projectiles,
        &fixture.session.collision,
        &fixture.world,
        fixture.geometry,
        &fixture.tuning,
        &[],
        tick,
    )
}

fn arm_full_charge(fixture: &mut Fixture, bot: &mut Bot) {
    let tuning = fixture.tuning.clone();
    let actor = fixture.actor_mut(CASTER);
    actor.selected = Spell::Fireball;
    assert!(actor
        .casting(
            ActorIntent {
                cast_pressed: true,
                cast_held: true,
                ..Default::default()
            },
            &tuning,
        )
        .is_none());
    for _ in 0..120 {
        assert!(actor
            .casting(
                ActorIntent {
                    cast_held: true,
                    ..Default::default()
                },
                &tuning,
            )
            .is_none());
    }
    assert_eq!(
        actor
            .charge()
            .expect("ordinary held charge")
            .elapsed
            .to_bits(),
        tuning.charge_seconds.to_bits()
    );
    bot.charging_fireball = true;
    bot.release_ticks = 0;
    bot.think_ticks = 0;
}

#[test]
fn charged_shadow_release_damages_real_low_and_capsule_profiles() {
    for species in [Species::Dragon, Species::Goblin, Species::Shaman] {
        let (mut fixture, mut bot) = battle_fixture(species);
        // Establish knowledge without assuming the optimizer must choose a hold
        // at this range. The controlled hold still uses the real casting input.
        fixture.actor_mut(CASTER).cooldowns = [600.0; 3];
        decide_battle(&fixture, &mut bot, 1);
        let observed = bot
            .battle
            .as_ref()
            .expect("battle sensing")
            .target
            .expect("visible target");
        assert_eq!(observed.body.species, species);
        assert_eq!(observed.body.dimensions, fixture.actor(TARGET).dimensions);
        fixture.actor_mut(CASTER).cooldowns = [0.0; 3];
        arm_full_charge(&mut fixture, &mut bot);
        let intent = decide_battle(&fixture, &mut bot, 122);
        assert!(
            intent.cast_released,
            "full charged shot must be admitted against {species:?}"
        );
        let tuning = fixture.tuning.clone();
        let actor = fixture.actor_mut(CASTER);
        actor.aim = intent.aim;
        actor.selected = intent.selected.expect("fireball selection");
        let (spell, speed) = actor
            .casting(intent, &tuning)
            .expect("ordinary charged release");
        assert_eq!(spell, Spell::Fireball);
        assert!((speed - tuning.launch_speed(tuning.charge_seconds)).abs() < SKIN);
        assert!(actor.charge().is_none());
        let mut out = CommandsOut::default();
        fixture.session.release(
            CASTER,
            spell,
            &tuning,
            speed,
            &fixture.world,
            fixture.geometry,
            fixture.materials,
            &mut out,
        );
        for _ in 0..240 {
            fixture.session.advance_projectiles(
                &fixture.world,
                fixture.geometry,
                fixture.materials,
                &mut out,
            );
            if fixture.session.projectiles.is_empty() {
                break;
            }
        }
        assert!(fixture.session.projectiles.is_empty());
        assert!(
            fixture.actor(TARGET).hp <= 100.0 - tuning.fireball_damage * 0.4,
            "admitted shot must deliver useful damage to the actual {species:?} shape"
        );
        assert_eq!(fixture.actor(CASTER).hp.to_bits(), 100.0_f32.to_bits());
        assert_eq!(fixture.session.effects.len(), 1, "one actual impact");
    }
}

#[test]
fn switching_to_new_target_cancels_armed_release_without_cross_id_velocity() {
    let (mut fixture, mut bot) = battle_fixture(Species::Goblin);
    fixture.actor_mut(CASTER).cooldowns = [600.0; 3];
    decide_battle(&fixture, &mut bot, 1);
    fixture.actor_mut(TARGET).feet.z += 0.3;
    decide_battle(&fixture, &mut bot, 13);
    assert!(
        bot.memory
            .expect("moving observed target")
            .velocity
            .length()
            > 1.0
    );
    fixture.actor_mut(CASTER).cooldowns = [0.0; 3];
    arm_full_charge(&mut fixture, &mut bot);
    fixture.actor_mut(TARGET).hp = 0.0;
    fixture
        .session
        .actors
        .push(profile(9, 42, Species::Dragon, Vec3::new(7.0, SKIN, -2.0)));

    let intent = decide_battle(&fixture, &mut bot, 25);
    assert!(
        !intent.cast_pressed && !intent.cast_held && !intent.cast_released,
        "changing an armed target requires a neutral cancellation sample"
    );
    let selected = bot
        .battle
        .as_ref()
        .expect("battle state")
        .target
        .expect("new target");
    assert_eq!(selected.body.id, 9);
    assert_eq!(selected.body.velocity, Vec3::ZERO);
    assert_eq!(selected.body.yaw_velocity.to_bits(), 0.0_f32.to_bits());
    assert_eq!(bot.memory.expect("new memory").velocity, Vec3::ZERO);
    assert!(!bot.charging_fireball);
    let tuning = fixture.tuning.clone();
    assert!(fixture.actor_mut(CASTER).casting(intent, &tuning).is_none());
    assert!(fixture.actor(CASTER).charge().is_none());
    assert_eq!(
        fixture.actor(CASTER).cooldowns.map(f32::to_bits),
        [0.0_f32.to_bits(); 3]
    );
    assert!(fixture.session.projectiles.is_empty());
}

#[test]
fn every_seen_hostile_shape_intercepts_before_selected_target_while_allies_pass() {
    let (mut fixture, mut bot) = battle_fixture(Species::Goblin);
    fixture.actor_mut(TARGET).feet = Vec3::new(3.0, SKIN, 0.0);
    fixture.actor_mut(TARGET).previous_feet = fixture.actor(TARGET).feet;
    let mut dragon = profile(9, 42, Species::Dragon, Vec3::new(3.0, SKIN, 1.5));
    dragon.body_yaw = 0.0;
    dragon.previous_yaw = 0.0;
    fixture.session.actors.push(dragon);
    fixture
        .session
        .actors
        .push(profile(10, 7, Species::Shadow, Vec3::new(-2.0, SKIN, 0.0)));
    fixture.actor_mut(CASTER).cooldowns = [600.0; 3];
    decide_battle(&fixture, &mut bot, 1);
    let battle = bot.battle.as_ref().expect("battle observations");
    assert_eq!(
        battle.target.expect("nearest center target").body.id,
        TARGET
    );
    let belief = bot
        .belief(true, &fixture.session.collision, &fixture.tuning)
        .expect("visible belief");
    let bodies = bot.forecast_bodies(belief, &fixture.tuning);
    assert_eq!(
        bodies.iter().map(|body| body.id).collect::<Vec<_>>(),
        vec![TARGET, 9]
    );
    let speed = fixture.tuning.launch_speed(fixture.tuning.charge_seconds);
    let (aim, _) = ballistic_aim(
        fixture.actor(CASTER).eye(),
        Vec3::new(3.0, SKIN + 0.2, 0.0),
        &fixture.tuning,
        speed,
    )
    .expect("low body shot");
    fixture.actor_mut(CASTER).aim = aim;
    let forecast = forecast_spell(
        fixture.actor(CASTER),
        &bodies,
        &fixture.session.collision,
        &fixture.world,
        fixture.geometry,
        &fixture.tuning,
        speed,
    )
    .impact
    .expect("nearest disclosed body contact");
    assert_eq!(
        forecast.actor,
        Some(9),
        "the long off-center Dragon intersects before the selected capsule"
    );
    let selected_only = bodies
        .iter()
        .copied()
        .filter(|body| body.id == TARGET)
        .collect::<Vec<_>>();
    let incomplete = forecast_spell(
        fixture.actor(CASTER),
        &selected_only,
        &fixture.session.collision,
        &fixture.world,
        fixture.geometry,
        &fixture.tuning,
        speed,
    )
    .impact
    .expect("selected body alone also hittable");
    assert_eq!(incomplete.actor, Some(TARGET));
    assert!(forecast.time < incomplete.time);

    let mut out = CommandsOut::default();
    fixture.session.release(
        CASTER,
        Spell::Fireball,
        &fixture.tuning,
        speed,
        &fixture.world,
        fixture.geometry,
        fixture.materials,
        &mut out,
    );
    for _ in 0..240 {
        fixture.session.advance_projectiles(
            &fixture.world,
            fixture.geometry,
            fixture.materials,
            &mut out,
        );
        if fixture.session.projectiles.is_empty() {
            break;
        }
    }
    let impact = fixture
        .session
        .effects
        .first()
        .expect("real projectile impact");
    assert!(
        impact.center.distance(forecast.point) < SKIN,
        "actual sweep must hit the same nearest shape as the disclosed forecast"
    );
    assert!(fixture.actor(9).hp < 100.0);
    assert_eq!(
        fixture.actor(10).hp.to_bits(),
        100.0_f32.to_bits(),
        "an intervening ally neither intercepts nor takes splash damage"
    );
}
