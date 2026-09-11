//! Recovery through the same controller and High Jump impulse used by human input.

use super::*;
use hex_core::SubstanceId;

fn crater(height: i32) -> (Actor, ArenaTerrainView, CollisionWorld, ArenaVoxelGeometry) {
    let geometry = ArenaVoxelGeometry::default();
    let mut world = ArenaTerrainView {
        revision: 1,
        ..Default::default()
    };
    for coord in HexCoord::ORIGIN.within_radius(12) {
        let top = if coord.distance(HexCoord::ORIGIN) <= 1 {
            0
        } else {
            height
        };
        for level in 0..=top {
            world
                .voxels
                .insert(TilePos::new(coord, level), SubstanceId(1));
        }
    }
    let mut collision = CollisionWorld::default();
    collision.refresh(&world, geometry);
    let mut bot = Actor::spawn(1, Vec3::Y * SKIN, Vec3::NEG_Z);
    crate::motion::tick(
        &mut bot,
        Vec3::ZERO,
        false,
        false,
        false,
        &collision,
        &ArenaTuning::default().encounters,
    );
    (bot, world, collision, geometry)
}

fn apply_movement(
    bot: &mut Actor,
    intent: ActorIntent,
    collision: &CollisionWorld,
    tuning: &ArenaTuning,
) {
    bot.aim = intent.aim;
    let forward = intent.aim.with_y(0.0).normalize_or(Vec3::NEG_Z);
    let direction = forward.cross(Vec3::Y) * intent.movement.x + forward * intent.movement.y;
    if intent.high_jump {
        bot.body.boost(tuning.high_jump_height);
        bot.grounded = false;
        if let Some(cooldown) = bot.cooldowns.get_mut(Spell::HighJump.index()) {
            *cooldown = tuning.high_jump_cooldown;
        }
    }
    crate::motion::tick(
        bot,
        direction,
        intent.run,
        intent.jump,
        false,
        collision,
        &tuning.encounters,
    );
}

#[test]
fn shadow_escape_normal_and_high_jumps_land_without_overshooting() {
    for (height, wants_high) in [(3, false), (8, true)] {
        let (mut bot, world, collision, geometry) = crater(height);
        let tuning = ArenaTuning::default();
        let mut recovery = EscapeRecovery::default();
        let mut normal = false;
        let mut high = false;
        let mut airborne_steer = false;
        let mut max_work = 0;
        let mut max_time = std::time::Duration::ZERO;
        for tick in 0..360 {
            let before = recovery.rollout_ticks;
            let started = std::time::Instant::now();
            let intent = recovery.intent(&bot, &collision, &world, geometry, &tuning, tick);
            max_time = max_time.max(started.elapsed());
            max_work = max_work.max(recovery.rollout_ticks - before);
            let action = intent.unwrap_or_else(|| neutral(&bot));
            normal |= action.jump;
            high |= action.high_jump;
            airborne_steer |= !bot.grounded && action.movement.length_squared() > 0.1;
            assert!(!action.cast_pressed && !action.cast_released && action.selected.is_none());
            apply_movement(&mut bot, action, &collision, &tuning);
            if recovery.escapes > 0 {
                break;
            }
        }
        eprintln!("escape height={height} max_intent={max_time:?} max_rollout={max_work}");
        assert!(max_work <= 240, "one two-second candidate per tick");
        assert_eq!(high, wants_high, "{recovery:?} feet={:?}", bot.feet);
        assert!(normal || high);
        assert!(
            airborne_steer && recovery.escapes == 1 && bot.grounded,
            "{recovery:?} feet={:?}",
            bot.feet
        );
        assert!(
            bot.feet.y >= f32::from(u8::try_from(height).expect("fixture height")) * 0.4 - 0.01
        );
    }
}

#[test]
fn shadow_escape_prefers_supported_ground_opening_to_jump() {
    let (bot, mut world, mut collision, geometry) = crater(8);
    world
        .voxels
        .retain(|pos, _| pos.level == 0 || !(pos.coord.x() > 0 && pos.coord.y() == 0));
    world.revision += 1;
    collision.refresh(&world, geometry);
    let tuning = ArenaTuning::default();
    let mut recovery = EscapeRecovery::default();
    let route = recovery
        .test_route(&bot, 0, None, &collision, &world, geometry, &tuning)
        .expect("dry ground opening");
    assert!(!route.jump && !route.boost);
}

#[test]
fn shadow_escape_open_ground_and_disabled_baseline_never_interrupted() {
    let (bot, world, collision, geometry) = crater(0);
    let mut recovery = EscapeRecovery::default();
    let mut tuning = ArenaTuning::default();
    for tick in 0..180 {
        assert!(recovery
            .intent(&bot, &collision, &world, geometry, &tuning, tick)
            .is_none());
    }
    let (bot, world, collision, geometry) = crater(8);
    tuning.bot.escape.enabled = false;
    for tick in 0..180 {
        assert!(recovery
            .intent(&bot, &collision, &world, geometry, &tuning, tick)
            .is_none());
    }
}

#[test]
fn shadow_escape_floor_circling_does_not_reset_trap_detection() {
    let (mut bot, world, collision, geometry) = crater(8);
    let mut recovery = EscapeRecovery::default();
    let tuning = ArenaTuning::default();
    for tick in 0..55 {
        bot.feet.x = if tick % 2 == 0 { 0.1 } else { -0.1 };
        recovery.intent(&bot, &collision, &world, geometry, &tuning, tick);
    }
    assert_eq!(recovery.attempts, 1);
}

#[test]
fn shadow_escape_airborne_rescue_requires_valid_landing() {
    let (mut bot, world, collision, geometry) = crater(8);
    bot.feet.y = 1.0;
    bot.grounded = false;
    bot.body.grounded = false;
    bot.body.vertical_velocity = -2.0;
    let tuning = ArenaTuning::default();
    let mut recovery = EscapeRecovery::default();
    let mut boosted = false;
    for tick in 0..240 {
        let intent = recovery
            .intent(&bot, &collision, &world, geometry, &tuning, tick)
            .unwrap_or_else(|| neutral(&bot));
        boosted |= intent.high_jump;
        apply_movement(&mut bot, intent, &collision, &tuning);
        if recovery.escapes > 0 {
            break;
        }
    }
    assert!(
        boosted && recovery.escapes == 1 && bot.grounded,
        "{recovery:?} {bot:?}"
    );
}

#[test]
fn shadow_escape_refuses_low_ceiling_and_cooldown_without_mutation() {
    let (mut bot, mut world, mut collision, geometry) = crater(8);
    let tuning = ArenaTuning::default();
    let mut recovery = EscapeRecovery::default();
    let rims = nearby_rims(&bot, bot.feet, &collision, &world, geometry, 6.0);
    if let Some(slot) = bot.cooldowns.get_mut(Spell::HighJump.index()) {
        *slot = 3.0;
    }
    for index in 12..18 {
        assert!(recovery
            .test_route(
                &bot,
                index,
                rims.get(index % 6).copied().flatten(),
                &collision,
                &world,
                geometry,
                &tuning
            )
            .is_none());
    }
    if let Some(slot) = bot.cooldowns.get_mut(Spell::HighJump.index()) {
        *slot = 0.0;
    }
    for coord in HexCoord::ORIGIN.within_radius(1) {
        world.voxels.insert(TilePos::new(coord, 5), SubstanceId(1));
    }
    world.revision += 1;
    collision.refresh(&world, geometry);
    let unchanged = world.voxels.clone();
    for index in 6..18 {
        assert!(recovery
            .test_route(
                &bot,
                index,
                rims.get(index % 6).copied().flatten(),
                &collision,
                &world,
                geometry,
                &tuning
            )
            .is_none());
    }
    assert_eq!(world.voxels, unchanged);
}

#[test]
fn shadow_escape_replans_after_knockback_and_terrain_changes() {
    let (mut bot, mut world, mut collision, geometry) = crater(8);
    let tuning = ArenaTuning::default();
    let mut recovery = EscapeRecovery::default();
    let rim = nearby_rims(&bot, bot.feet, &collision, &world, geometry, 6.0)
        .into_iter()
        .flatten()
        .next()
        .expect("rim");
    let route = recovery
        .test_route(&bot, 12, Some(rim), &collision, &world, geometry, &tuning)
        .expect("boost route");
    recovery.started = Some(0);
    let intent = recovery.advance_route(&bot, route, &collision, &world, geometry, &tuning);
    apply_movement(&mut bot, intent, &collision, &tuning);
    bot.body.impulse_velocity += Vec3::X * 2.0;
    world
        .voxels
        .insert(TilePos::new(HexCoord::from_axial(3, 0), 12), SubstanceId(1));
    world.revision += 1;
    collision.refresh(&world, geometry);
    recovery.intent(&bot, &collision, &world, geometry, &tuning, 1);
    assert!(recovery.route.is_none());
    recovery.cancel_charge();
    assert!(recovery.route.is_none() && recovery.search.is_none());
}

#[test]
fn shadow_escape_retries_when_high_jump_becomes_ready_or_height_changes() {
    for cooldown_blocked in [true, false] {
        let (mut bot, world, collision, geometry) = crater(8);
        let mut tuning = ArenaTuning::default();
        let mut recovery = EscapeRecovery::default();
        if cooldown_blocked {
            *bot.cooldowns
                .get_mut(Spell::HighJump.index())
                .expect("High Jump slot") = 1.0;
        } else {
            tuning.high_jump_height = 1.0;
        }
        for tick in 0..240 {
            let intent = recovery.intent(&bot, &collision, &world, geometry, &tuning, tick);
            assert!(intent.is_none_or(|intent| !intent.high_jump));
        }
        assert_eq!(recovery.attempts, 1, "unchanged failure stays suppressed");
        *bot.cooldowns
            .get_mut(Spell::HighJump.index())
            .expect("High Jump slot") = 0.0;
        tuning.high_jump_height = 4.0;
        let mut boosted = false;
        for tick in 240..360 {
            let intent = recovery.intent(&bot, &collision, &world, geometry, &tuning, tick);
            if intent.is_some_and(|intent| intent.high_jump) {
                boosted = true;
                break;
            }
        }
        assert!(
            boosted,
            "capability change permits a fresh validated escape"
        );
        assert_eq!(recovery.attempts, 2);
    }
}
