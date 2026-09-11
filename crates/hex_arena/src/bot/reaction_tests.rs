//! Acquisition delay through actual bot intents; simulation time owns the clock.
use super::*;

#[test]
fn acquisition_wait_blocks_prepared_fireball_until_exact_deadline() {
    for delay in [0.0, 0.15, 0.5] {
        let mut fixture = prepared();
        fixture.tuning.bot.acquisition_seconds = delay;
        let first = fixture.decide();
        let acquired = fixture.session.bot.acquired_at.expect("new sighting");
        assert_eq!(first.cast_released, delay == 0.0);
        if delay == 0.0 {
            continue;
        }
        for elapsed in 1..u64::from(ticks(delay)) {
            fixture.session.tick = acquired + elapsed;
            assert!(
                !fixture.decide().cast_released,
                "early release at {elapsed}"
            );
        }
        fixture.session.bot.think_ticks = 23;
        fixture.session.tick = acquired + u64::from(ticks(delay));
        let released = fixture.decide();
        assert!(released.cast_released);
        assert_eq!(released.selected, Some(Spell::Fireball));
        assert_eq!(fixture.session.bot.acquired_at, Some(acquired));
        assert_eq!(
            fixture.session.bot.release_ticks,
            ticks(fixture.tuning.bot.reaction_seconds)
        );
    }
}

#[test]
fn acquisition_menu_adjustment_uses_original_simulation_timestamp() {
    let mut fixture = prepared();
    fixture.tuning.bot.acquisition_seconds = 0.5;
    assert!(!fixture.decide().cast_released);
    fixture.session.tick = 12;
    assert!(!fixture.decide().cast_released);
    for _ in 0..60 {
        fixture.session.bot.cancel_charge();
        fixture.session.bot.update_acquisition(&fixture.tuning);
    }
    assert_eq!(
        fixture.session.bot.acquisition_wait_ticks, 48,
        "paused wall time does not advance tick"
    );
    fixture.tuning.bot.acquisition_seconds = 0.15;
    fixture.session.tick = 18;
    fixture.session.bot.think_ticks = 20;
    assert!(fixture.decide().cast_released);
    assert_eq!(fixture.session.bot.acquired_at, Some(0));
}

#[test]
fn acquisition_rechecks_prepared_fireball_and_restarts_only_after_lost_sight() {
    let mut fixture = Fixture::new(14.0);
    fixture.tuning.bot.acquisition_seconds = 0.15;
    fixture.tuning.bot.blind_fire_enabled = false;
    fixture.tuning.bot.flank_enabled = false;
    fixture.tuning.bot.ambush_chance = 0.0;
    fixture.actor_mut(1).cooldowns = [600.0; 3];
    fixture.advance();
    for coord in HexCoord::ORIGIN.within_radius(2) {
        for level in 1..=8 {
            fixture
                .world
                .voxels
                .insert(TilePos::new(coord, level), fixture.materials.stone);
        }
    }
    fixture.refresh();
    fixture.actor_mut(1).cooldowns = [0.0; 3];
    for _ in 0..130 {
        fixture.advance();
    }
    assert!(fixture.session.bot.observation.target.is_none());
    assert!(fixture.actor(1).charge().is_some());
    fixture.world.voxels.retain(|pos, _| pos.level == 0);
    fixture.refresh();
    fixture.session.bot.sense_ticks = 1;
    fixture.advance();
    let acquired = fixture.session.bot.acquired_at.expect("reappearance");
    assert!(fixture.session.projectiles.is_empty());
    for _ in 0..40 {
        let now = fixture.session.tick;
        fixture.advance();
        if let Some(shot) = fixture.session.projectiles.first() {
            assert_eq!(shot.owner, 1);
            assert!(fixture.session.bot.tick >= acquired + u64::from(ticks(0.15)));
            assert!(
                fixture.session.bot.tick <= acquired + u64::from(ticks(0.15)) + 1,
                "deadline must not wait for a movement decision: {now}"
            );
            return;
        }
    }
    panic!("prepared shot should release on acquisition deadline");
}

#[test]
fn acquisition_defensive_shield_can_interrupt_before_offensive_deadline() {
    let mut fixture = Fixture::new(14.0);
    fixture.tuning.bot.acquisition_seconds = 0.5;
    fixture.actor_mut(1).hp = 50.0;
    let intent = fixture.decide();
    assert!(intent.cast_released);
    assert_eq!(intent.selected, Some(Spell::Shield));
    assert!(!fixture.session.bot.acquisition_ready());
}

fn prepared() -> Fixture {
    let mut fixture = Fixture::new(14.0);
    let tuning = fixture.tuning.clone();
    for tick in 0..100 {
        fixture.actor_mut(1).casting(
            ActorIntent {
                selected: Some(Spell::Fireball),
                cast_pressed: tick == 0,
                cast_held: true,
                ..Default::default()
            },
            &tuning,
        );
    }
    fixture.session.bot.charging_fireball = true;
    fixture
}
