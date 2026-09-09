//! Prepared waiting and route recovery, using real collision and body rollouts.

use super::*;

fn covered_fixture(wide: bool) -> Fixture {
    let mut fixture = Fixture::new(14.0);
    fixture.tuning.bot.blind_fire_enabled = false;
    fixture.tuning.bot.ambush_chance = 0.0;
    fixture.actor_mut(1).cooldowns = [600.0; 3];
    fixture.advance();
    assert!(fixture.session.bot.debug().last_seen.is_some());
    let columns = if wide {
        HexCoord::ORIGIN
            .within_radius(12)
            .into_iter()
            .filter(|coord| coord.to_world(0.0).x.abs() < 0.9)
            .collect::<Vec<_>>()
    } else {
        vec![
            HexCoord::from_axial(0, -1),
            HexCoord::ORIGIN,
            HexCoord::from_axial(0, 1),
        ]
    };
    for coord in columns {
        for level in 1..=8 {
            fixture
                .world
                .voxels
                .insert(TilePos::new(coord, level), fixture.materials.stone);
        }
    }
    fixture.refresh();
    fixture.session.bot.sense_ticks = 0;
    fixture.session.bot.think_ticks = 0;
    fixture
}

fn assert_hidden(fixture: &Fixture) {
    assert!([fixture.actor(0).center(), fixture.actor(0).eye()]
        .into_iter()
        .all(|point| !visible_from(&fixture.session.collision, fixture.actor(1).eye(), point)));
}

fn establish_route(fixture: &mut Fixture) {
    for _ in 0..120 {
        fixture.advance();
        if !fixture.session.bot.route.points.is_empty() {
            return;
        }
    }
    panic!("cover must produce a real local route before testing its recovery");
}

#[test]
fn prepared_ambush_times_out_once_per_loss_episode_without_false_stuck_recovery() {
    let mut fixture = covered_fixture(true);
    fixture.tuning.bot.ambush_chance = 1.0;
    fixture.actor_mut(1).cooldowns = [600.0, 0.0, 600.0];
    for _ in 0..120 {
        fixture.advance();
        if fixture.session.bot.ambush_until > fixture.session.tick {
            break;
        }
    }
    let until = fixture.session.bot.ambush_until;
    let start_tick = fixture.session.tick;
    assert!(until > start_tick);
    assert_eq!(until - start_tick, u64::from(ticks(1.2)));
    assert!(fixture.actor(1).charge().is_some());
    assert!(!fixture.session.bot.route.points.is_empty());
    let feet = fixture.actor(1).feet;
    let side = fixture.session.bot.route.side;
    let episode = fixture.session.bot.ambush_episode;
    assert!(episode.is_some());
    while fixture.session.tick + 1 < until {
        assert_hidden(&fixture);
        fixture.advance();
        assert!(
            fixture.actor(1).feet.distance(feet) < 0.002,
            "the prepared wait must actually hold position"
        );
        assert!(
            !fixture.session.bot.route.points.is_empty(),
            "deliberate waiting must not count as stuck"
        );
        assert_eq!(fixture.session.bot.route.side, side);
        assert_eq!(fixture.session.bot.ambush_until, until);
        assert!(fixture.session.projectiles.is_empty());
    }
    for _ in 0..120 {
        assert_hidden(&fixture);
        fixture.advance();
        assert_eq!(fixture.session.bot.ambush_episode, episode);
        assert_eq!(
            fixture.session.bot.ambush_until, until,
            "the same unseen episode cannot start another wait"
        );
        assert!(fixture.session.projectiles.is_empty());
    }
    assert!(fixture.session.tick >= until);
    assert!(
        fixture.actor(1).feet.distance(feet) > 1.0,
        "the bot must resume its route after the bounded wait"
    );
    assert!(fixture.actor(1).grounded);
}

#[test]
fn terrain_revision_and_same_revision_knockback_discard_a_route_before_next_movement() {
    for remove_support in [true, false] {
        let mut fixture = covered_fixture(false);
        establish_route(&mut fixture);
        let old_revision = fixture.session.bot.route.revision;
        fixture.session.bot.think_ticks = 600; // Exercise the per-tick invalidation, not a new decision.
        if remove_support {
            let under = HexCoord::from_world(fixture.actor(1).feet);
            for coord in under.within_radius(1) {
                fixture.world.voxels.remove(&TilePos::new(coord, 0));
            }
            fixture.refresh();
            assert_ne!(old_revision, fixture.session.collision.revision);
        } else {
            fixture.actor_mut(1).body.impulse_velocity = Vec3::Z * 2.0;
            assert_eq!(old_revision, fixture.session.collision.revision);
        }
        fixture.advance();
        assert!(
            fixture.session.bot.route.points.is_empty(),
            "old waypoints must not survive a changed physical start or support map"
        );
        assert!(fixture.actor(1).feet.is_finite());
        assert!(fixture
            .session
            .collision
            .clear(fixture.actor(1).feet, BODY_HEIGHT, BODY_RADIUS));
        if remove_support {
            assert!(!fixture.actor(1).grounded);
        } else {
            assert!(
                fixture.actor(1).impulse_velocity().length() > 1.0,
                "the test must apply real external momentum"
            );
        }
    }
}

#[test]
fn stalled_route_switches_side_then_backs_off_without_exceeding_the_rollout_budget() {
    let fixture = covered_fixture(false);
    let actor = fixture.actor(1).clone();
    let target = fixture.actor(0).center();
    let collision = &fixture.session.collision;
    let tuning = &fixture.tuning.bot;
    let mut route = Route::default();
    let first = 100;
    let work = route.plan(&actor, target, collision, first, tuning, 1);
    assert!((1..=480).contains(&work));
    let original_side = route.side;
    assert!(route.travel(&actor, first, tuning).is_some());
    // A movement request that made no physical progress is distinct from the
    // deliberate hold path tested above; it must eventually abandon this side.
    let stalled = first + u64::from(ticks(tuning.stuck_seconds));
    assert!(route.travel(&actor, stalled, tuning).is_none());
    assert!(route.points.is_empty());
    assert_eq!(route.side, -original_side);
    assert_eq!(route.plan(&actor, target, collision, stalled, tuning, 1), 0);
    let retry = first + u64::from(ticks(tuning.flank_replan_seconds));
    let work = route.plan(&actor, target, collision, retry, tuning, 1);
    assert!((1..=480).contains(&work));
    assert_eq!(route.side, -original_side);
    let second_stall = retry + u64::from(ticks(tuning.stuck_seconds));
    assert!(route.travel(&actor, second_stall, tuning).is_none());
    assert_eq!(
        route.plan(&actor, target, collision, second_stall + 119, tuning, 1),
        0,
        "two failed sides require the one-second recovery gap"
    );
    let work = route.plan(&actor, target, collision, second_stall + 120, tuning, 1);
    assert!(
        (1..=480).contains(&work),
        "planning must become eligible again after the bounded backoff"
    );
}
