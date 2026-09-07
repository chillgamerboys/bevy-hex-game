//! Hidden-information and navigation regressions through ordinary arena simulation.

use super::*;

fn add_divider(fixture: &mut Fixture) {
    add_divider_with_width(fixture, 1.8);
}

fn add_divider_with_width(fixture: &mut Fixture, half_width: f32) {
    // Reach both arena edges so these knowledge tests cannot accidentally reveal
    // the target while the bot is moving. Navigation gets its own short wall.
    for coord in HexCoord::ORIGIN.within_radius(fixture.geometry.radius) {
        if coord.to_world(0.0).x.abs() < half_width {
            for level in 1..=12 {
                fixture
                    .world
                    .voxels
                    .insert(TilePos::new(coord, level), fixture.materials.stone);
            }
        }
    }
    fixture.refresh();
}

fn target_hidden(fixture: &Fixture) -> bool {
    [fixture.actor(0).center(), fixture.actor(0).eye()]
        .into_iter()
        .all(|target| !visible_from(&fixture.session.collision, fixture.actor(1).eye(), target))
}

fn assert_same_bot(left: &Fixture, right: &Fixture) {
    let a = left.actor(1);
    let b = right.actor(1);
    assert_eq!(a.feet, b.feet, "hidden positions changed movement");
    assert_eq!(a.aim, b.aim, "hidden positions changed aim");
    assert_eq!(a.selected, b.selected);
    assert_eq!(a.charge(), b.charge());
    assert_eq!(a.cooldowns.map(f32::to_bits), b.cooldowns.map(f32::to_bits));
    assert_eq!(left.session.bot.seed, right.session.bot.seed);
    let a = left.session.bot.debug();
    let b = right.session.bot.debug();
    assert_eq!(a.mode, b.mode);
    assert_eq!(a.last_seen, b.last_seen);
    assert_eq!(a.memory_age, b.memory_age);
    assert_eq!(a.cue_position, b.cue_position);
    assert_eq!(a.charging, b.charging);
    assert_eq!(a.route_len, b.route_len);
    assert_eq!(a.flank_side, b.flank_side);
    assert_eq!(a.blind_shots, b.blind_shots);
    let shots = |fixture: &Fixture| {
        fixture
            .session
            .projectiles
            .iter()
            .filter(|shot| shot.owner == 1)
            .map(|shot| (shot.spell, shot.position, shot.velocity))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        shots(left),
        shots(right),
        "hidden bodies changed shot forecasts/releases"
    );
}

#[test]
fn identical_observations_ignore_silent_hidden_positions_and_velocities() {
    let mut left = Fixture::new(14.0);
    let mut right = Fixture::new(14.0);
    left.actor_mut(1).cooldowns = [600.0; 3];
    right.actor_mut(1).cooldowns = [600.0; 3];
    for _ in 0..12 {
        left.advance();
        right.advance();
        assert_same_bot(&left, &right);
    }
    assert!(left.session.bot.debug().last_seen.is_some());
    add_divider(&mut left);
    add_divider(&mut right);
    left.actor_mut(1).cooldowns = [0.0; 3];
    right.actor_mut(1).cooldowns = [0.0; 3];
    let mut saw_charge = false;
    for tick in 0..360 {
        // No release or impact cue accompanies these different hidden histories.
        // Previous feet also differ, detecting illicit hidden velocity forecasts.
        let offset = if tick % 2 == 0 { 0.5 } else { -0.5 };
        for (fixture, side) in [(&mut left, 1.0), (&mut right, -1.0)] {
            let human = fixture.actor_mut(0);
            human.previous_feet = Vec3::new(7.0, SKIN, side * 4.0);
            human.feet = Vec3::new(7.0, SKIN, side * (3.0 + offset));
            assert!(
                target_hidden(fixture),
                "fixture target became visible at {tick}"
            );
        }
        left.advance();
        right.advance();
        assert_same_bot(&left, &right);
        saw_charge |= left.actor(1).charge().is_some();
    }
    assert!(
        saw_charge,
        "the paired runs must exercise remembered-target precharging"
    );
}

#[test]
fn a_hidden_opponent_without_observation_or_cues_never_arms_blind_fire() {
    let mut fixture = Fixture::new(14.0);
    add_divider(&mut fixture);
    for tick in 0..720 {
        assert!(
            target_hidden(&fixture),
            "fixture target became visible at {tick}"
        );
        let out = fixture.advance();
        assert!(out.impacts.is_empty() && out.edits.is_empty());
        assert!(fixture.session.projectiles.is_empty());
        assert!(fixture.actor(1).charge().is_none());
        let debug = fixture.session.bot.debug();
        assert!(debug.last_seen.is_none() && debug.cue_position.is_none());
        assert_eq!(debug.blind_shots, 0);
    }
    assert_eq!(
        fixture
            .session
            .round_summary()
            .actors
            .get(1)
            .expect("bot stats")
            .casts,
        [0; 3]
    );
}

#[test]
fn covered_memory_can_hold_full_charge_but_expiry_cancels_without_a_cast() {
    let mut fixture = Fixture::new(14.0);
    fixture.tuning.bot.blind_fire_enabled = false;
    fixture.tuning.bot.flank_enabled = false;
    fixture.tuning.bot.ambush_chance = 0.0;
    fixture.actor_mut(1).cooldowns = [600.0; 3];
    fixture.advance();
    assert!(fixture.session.bot.debug().last_seen.is_some());
    assert!(fixture.actor(1).charge().is_none());
    add_divider(&mut fixture);
    fixture.actor_mut(1).cooldowns = [0.0; 3];
    fixture.session.bot.think_ticks = 0;
    let mut saw_full_charge = false;
    for tick in 0..900 {
        assert!(
            target_hidden(&fixture),
            "fixture target became visible at {tick}"
        );
        let out = fixture.advance();
        assert!(out.impacts.is_empty() && out.edits.is_empty());
        assert!(fixture.session.projectiles.is_empty());
        if let Some(charge) = fixture.actor(1).charge() {
            assert_eq!(charge.spell, Spell::Fireball);
            saw_full_charge |= (charge.elapsed - fixture.tuning.charge_seconds).abs() < SKIN;
        }
        if tick == 600 {
            assert!(
                fixture.session.bot.debug().last_seen.is_some(),
                "direct sight memory lasts six seconds"
            );
        }
    }
    assert!(
        saw_full_charge,
        "a covered remembered target permits a capped precharge"
    );
    assert!(fixture.actor(1).charge().is_none());
    assert!(!fixture.session.bot.debug().charging);
    assert!(fixture.session.bot.debug().last_seen.is_none());
    assert!(fixture.bot_cooldown(Spell::Fireball).abs() < SKIN);
    assert_eq!(
        fixture
            .session
            .round_summary()
            .actors
            .get(1)
            .expect("bot stats")
            .casts,
        [0; 3]
    );
}

#[test]
fn hearing_only_precharges_but_never_attacks_without_a_direct_sighting() {
    let mut fixture = Fixture::new(14.0);
    fixture.tuning.bot.flank_enabled = false;
    fixture.tuning.bot.ambush_chance = 0.0;
    add_divider_with_width(&mut fixture, 0.9);
    assert!(target_hidden(&fixture));
    assert!(fixture.session.bot.debug().last_seen.is_none());
    // Even a cue whose terrain region can be splashed cannot authorize blind
    // fire without a prior direct sighting. It may authorize a covered precharge.
    fixture.session.combat_cue(
        0,
        Vec3::new(-0.9, 0.4, 0.0),
        crate::telemetry::CombatCueKind::Impact,
    );
    let mut saw_cue = false;
    let mut saw_charge = false;
    for tick in 0..720 {
        assert!(
            target_hidden(&fixture),
            "fixture target became visible at {tick}"
        );
        let out = fixture.advance();
        assert!(out.impacts.is_empty() && out.edits.is_empty());
        assert!(fixture.session.projectiles.is_empty());
        let debug = fixture.session.bot.debug();
        saw_cue |= debug.cue_position.is_some();
        assert!(
            debug.last_seen.is_none(),
            "a heard cue cannot become a sighting"
        );
        saw_charge |= fixture.actor(1).charge().is_some();
        assert_eq!(debug.blind_shots, 0);
        let casts = fixture
            .session
            .round_summary()
            .actors
            .get(1)
            .expect("bot stats")
            .casts;
        assert_eq!(casts, [0; 3]);
    }
    assert!(
        saw_cue && saw_charge,
        "an in-range discrete cue permits a covered precharge"
    );
    assert!(fixture.session.bot.debug().cue_position.is_none());
    assert!(fixture.actor(1).charge().is_none());
    assert!(fixture.bot_cooldown(Spell::Fireball).abs() < SKIN);
}

#[test]
fn recent_direct_sighting_allows_one_blind_shot_and_cues_never_restore_the_budget() {
    let mut fixture = Fixture::new(14.0);
    fixture.tuning.bot.flank_enabled = false;
    fixture.tuning.bot.ambush_chance = 0.0;
    // Leave enough time for a prohibited repeat inside the 1.5-second sighting
    // window. These are existing spell settings, not a special bot shot mode.
    fixture.tuning.charge_seconds = 0.2;
    fixture.tuning.fireball_cooldown = 0.1;
    // A complete hex wall is too thick for Standard's required 40% splash.
    // Large admits useful damage through this wall without weakening admission.
    fixture.tuning.fireball_size = 2;
    let human = fixture.actor_mut(0);
    human.feet = Vec3::new(1.2, SKIN, 0.0);
    human.previous_feet = human.feet;
    fixture.actor_mut(1).cooldowns = [600.0; 3];
    fixture.advance();
    assert!(fixture.session.bot.debug().last_seen.is_some());
    assert!(fixture.session.projectiles.is_empty());
    let remembered_feet = fixture.actor(0).feet;
    add_divider_with_width(&mut fixture, 0.9);
    assert!(target_hidden(&fixture));
    assert!(fixture
        .session
        .collision
        .clear(fixture.actor(0).feet, BODY_HEIGHT, BODY_RADIUS));
    fixture.actor_mut(1).cooldowns = [0.0; 3];
    fixture.session.bot.think_ticks = 0;
    let mut first_release = None;
    let mut splashed_remembered_region = false;
    for tick in 0..900 {
        assert!(
            target_hidden(&fixture),
            "fixture target became visible at {tick}"
        );
        if first_release.is_some() && tick < 360 && tick % 12 == 0 {
            // Both fresh enemy events and the bot's own effects are forbidden
            // from replenishing the once-per-LOS-loss blind-shot permission.
            for owner in [0, 1] {
                fixture.session.combat_cue(
                    owner,
                    Vec3::new(-0.9, 0.4, 0.0),
                    crate::telemetry::CombatCueKind::Impact,
                );
            }
        }
        fixture.advance();
        splashed_remembered_region |= fixture.session.effects.iter().any(|effect| {
            effect.kind == Spell::Fireball
                && crate::spells::capsule_distance(effect.center, remembered_feet)
                    <= effect.radius * 0.6
        });
        let debug = fixture.session.bot.debug();
        assert!(debug.blind_shots <= 1);
        let casts = fixture
            .session
            .round_summary()
            .actors
            .get(1)
            .expect("bot stats")
            .casts;
        assert_eq!(casts.first().copied().unwrap_or_default(), 0);
        assert_eq!(casts.get(2).copied().unwrap_or_default(), 0);
        let fireballs = casts.get(1).copied().expect("Fireball slot");
        assert!(
            fireballs <= 1,
            "fresh cues must not authorize a second hidden attack"
        );
        if fireballs == 1 {
            first_release.get_or_insert(tick);
        }
    }
    assert!(
        first_release.is_some_and(|tick| tick < 180),
        "one safe shot must release within 1.5 seconds of direct sight"
    );
    assert!(
        splashed_remembered_region,
        "the actual terrain impact must yield at least 40% splash at the remembered region"
    );
    assert!(fixture
        .session
        .round_summary()
        .actors
        .get(1)
        .is_some_and(|stats| stats.damage_dealt >= fixture.tuning.fireball_damage * 0.4));
    assert_eq!(fixture.session.bot.debug().blind_shots, 1);
    assert!(fixture.session.bot.debug().last_seen.is_none());
    assert!(fixture.session.bot.debug().cue_position.is_none());
    assert!(fixture.actor(1).charge().is_none());
    assert!(fixture.bot_cooldown(Spell::Fireball).abs() < SKIN);
}

#[test]
fn real_body_routes_around_short_cover_without_destroying_it() {
    let mut fixture = Fixture::new(14.0);
    fixture.actor_mut(1).cooldowns = [600.0; 3];
    fixture.advance(); // Publish one genuine visible observation before cover appears.
    assert!(fixture.session.bot.debug().last_seen.is_some());
    for coord in [
        HexCoord::from_axial(0, -1),
        HexCoord::ORIGIN,
        HexCoord::from_axial(0, 1),
    ] {
        for level in 1..=8 {
            fixture
                .world
                .voxels
                .insert(TilePos::new(coord, level), fixture.materials.stone);
        }
    }
    fixture.refresh();
    assert!(target_hidden(&fixture));
    let original_world = fixture.world.voxels.clone();
    let start = fixture.actor(1).feet;
    let mut routed = false;
    let mut regained_sight = false;
    for _ in 0..960 {
        let out = fixture.advance();
        assert!(out.impacts.is_empty() && out.edits.is_empty());
        assert!(fixture.session.projectiles.is_empty());
        let actor = fixture.actor(1);
        assert!(fixture
            .session
            .collision
            .clear(actor.feet, BODY_HEIGHT, BODY_RADIUS));
        assert!(
            actor.feet.y.abs() < 0.05,
            "the route must walk around the tall wall"
        );
        assert!(actor.hp > 0.0);
        routed |= fixture.session.bot.debug().route_len > 0;
        if actor.feet.distance(start) > 2.0 && !target_hidden(&fixture) {
            regained_sight = true;
            break;
        }
    }
    assert!(routed, "blocked pursuit should produce a bounded route");
    assert!(
        regained_sight,
        "real movement must reach an unobstructed flank within eight seconds"
    );
    assert_eq!(fixture.world.voxels, original_world);
    assert_eq!(
        fixture
            .session
            .round_summary()
            .actors
            .get(1)
            .expect("bot stats")
            .casts,
        [0; 3]
    );
}
