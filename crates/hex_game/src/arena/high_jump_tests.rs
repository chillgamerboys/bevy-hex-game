//! Native key edges, independent charging, and direct High Jump captures.
use super::*;

fn ready() -> App {
    let (mut app, _) = menu_app();
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
    tap_key(&mut app, KeyCode::Enter);
    app.update();
    app
}

fn human(app: &App) -> &hex_arena::Actor {
    app.world()
        .resource::<ArenaSession>()
        .actors
        .first()
        .expect("human")
}

#[test]
fn high_jump_key_preserves_selected_fireball_and_held_charge() {
    let mut app = ready();
    tap_key(&mut app, KeyCode::Digit2);
    charge_with_mouse(&mut app);
    let before = human(&app).charge().expect("charging").elapsed;
    tap_key(&mut app, KeyCode::Digit3);
    let actor = human(&app);
    assert_eq!(actor.selected, Spell::Fireball);
    assert!(actor.charge().is_some_and(|charge| charge.elapsed > before));
    assert!(actor
        .cooldowns
        .get(Spell::HighJump.index())
        .is_some_and(|cd| *cd > 6.9));
    assert!(app
        .world()
        .resource::<ArenaSession>()
        .projectiles
        .is_empty());
    assert!(actor.feet.y > 3.2);
}

#[test]
fn high_jump_press_between_ticks_is_consumed_once_without_mouse_input() {
    let (mut app, _) = menu_app_at(480);
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
    {
        let mut state = app.world_mut().resource_mut::<ViewState>();
        state.begin_play();
        state.suppress_high_jump = false;
    }
    let before = app.world().resource::<ArenaSession>().tick;
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Digit3);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear();
    assert_eq!(app.world().resource::<ArenaSession>().tick, before);
    assert!(app.world().resource::<ArenaInput>().human.high_jump);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::Digit3);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear();
    for _ in 0..3 {
        app.update();
    }
    assert!(!app.world().resource::<ArenaInput>().human.high_jump);
    assert!(human(&app).cooldowns.get(2).is_some_and(|cd| *cd > 6.9));
    assert!(human(&app).charge().is_none());
    assert!(app
        .world()
        .resource::<ArenaSession>()
        .projectiles
        .is_empty());
}

#[test]
fn high_jump_and_fireball_release_share_a_frame_without_cancelling_each_other() {
    let mut app = ready();
    charge_with_mouse(&mut app);
    let eye_before = human(&app).eye();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Digit3);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(MouseButton::Left);
    app.update();
    assert!(human(&app).charge().is_none());
    assert_eq!(human(&app).selected, Spell::Fireball);
    assert!(human(&app).cooldowns.get(2).is_some_and(|cd| *cd > 6.9));
    assert!(app
        .world()
        .resource::<ArenaSession>()
        .projectiles
        .iter()
        .any(|projectile| {
            projectile.spell == Spell::Fireball && projectile.position.y > eye_before.y
        }));
}

#[test]
fn high_jump_holding_during_cooldown_does_not_buffer_or_repeat() {
    let mut app = ready();
    *app.world_mut()
        .resource_mut::<ArenaSession>()
        .actors
        .first_mut()
        .expect("human")
        .cooldowns
        .get_mut(2)
        .expect("High Jump slot") = 0.2;
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Digit3);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear();
    for _ in 0..60 {
        app.update();
    }
    assert!(human(&app).cooldowns.get(2).is_some_and(|cd| *cd <= 0.0));
    assert!((human(&app).feet.y - 3.2).abs() < 0.01);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::Digit3);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear();
    tap_key(&mut app, KeyCode::Digit3);
    assert!(human(&app).cooldowns.get(2).is_some_and(|cd| *cd > 6.9));
}

#[test]
fn high_jump_held_across_pause_focus_and_reset_requires_a_fresh_press() {
    for cancel in ["pause", "focus", "reset"] {
        let mut app = ready();
        match cancel {
            "focus" => {
                let mut windows = app.world_mut().query::<&mut Window>();
                windows.single_mut(app.world_mut()).expect("window").focused = false;
                app.update();
            }
            "reset" => tap_key(&mut app, KeyCode::KeyR),
            _ => tap_key(&mut app, KeyCode::Escape),
        }
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Digit3);
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
        if cancel == "focus" {
            let mut windows = app.world_mut().query::<&mut Window>();
            windows.single_mut(app.world_mut()).expect("window").focused = true;
        }
        tap_key(
            &mut app,
            if cancel == "reset" {
                KeyCode::Enter
            } else {
                KeyCode::Escape
            },
        );
        for _ in 0..4 {
            app.update();
        }
        assert!(!app.world().resource::<ArenaInput>().human.high_jump);
        assert!(
            human(&app).cooldowns.get(2).is_some_and(|cd| *cd <= 0.0),
            "{cancel}"
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::Digit3);
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
        tap_key(&mut app, KeyCode::Digit3);
        assert!(
            human(&app).cooldowns.get(2).is_some_and(|cd| *cd > 6.9),
            "{cancel}"
        );
    }
}

#[test]
fn high_jump_capture_uses_the_instant_key_edge_and_shows_no_charge() {
    let mut app = app(60);
    let view = "high-jump-first";
    let start = human(&app).feet;
    for frame in 1..=capture_frame_index(view) {
        let sample = capture_intent(frame, view, app.world().resource::<ArenaTuning>(), Vec3::X);
        assert!(!sample.cast_pressed && !sample.cast_released && !sample.cast_held);
        assert!(sample.selected.is_none());
        app.world_mut().resource_mut::<ArenaInput>().human = sample;
        app.update();
    }
    assert!(human(&app).feet.y > start.y + 1.0);
    assert!(human(&app).charge().is_none());
    assert!(human(&app).cooldowns.get(2).is_some_and(|cd| *cd > 6.0));
    assert!(app
        .world()
        .resource::<ArenaSession>()
        .projectiles
        .is_empty());
}

#[test]
fn high_jump_height_and_cooldown_follow_physics_at_different_render_rates() {
    for hz in [30, 60, 144, 480] {
        let mut app = app(hz);
        let floor = human(&app).feet.y;
        app.world_mut().resource_mut::<ArenaInput>().human.high_jump = true;
        let mut highest = floor;
        for _ in 0..hz * 2 {
            app.update();
            highest = highest.max(human(&app).feet.y);
        }
        assert!(
            (highest - floor - 4.0).abs() < 0.01,
            "hz={hz}, apex={highest}"
        );
        assert!(
            human(&app)
                .cooldowns
                .get(2)
                .is_some_and(|cd| (4.99..5.02).contains(cd)),
            "hz={hz}"
        );
        assert!(!app.world().resource::<ArenaInput>().human.high_jump);
    }
}
