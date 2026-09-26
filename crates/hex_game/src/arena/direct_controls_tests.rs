//! Direct native gestures preserve input ordering independently of physics cadence.
use super::*;
use bevy::input::ButtonState;

#[test]
fn restart_requires_shift_and_plain_r_preserves_the_active_or_paused_run() {
    for paused in [false, true] {
        for shift in [KeyCode::ShiftLeft, KeyCode::ShiftRight] {
            let (mut app, _) = ready(60);
            app.world_mut().resource_mut::<ViewState>().paused = paused;
            app.world_mut()
                .resource_mut::<ArenaSession>()
                .actors
                .first_mut()
                .expect("ready fixture has its player")
                .hp = 20.0;
            let generation = app.world().resource::<ArenaReset>().generation;
            tap_key(&mut app, KeyCode::KeyR);
            assert_eq!(app.world().resource::<ArenaReset>().generation, generation);
            assert!(app.world().resource::<ViewState>().started);
            assert!(
                app.world()
                    .resource::<ArenaSession>()
                    .actors
                    .first()
                    .expect("restart-preserving fixture has its player")
                    .hp
                    < 21.0
            );

            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(shift);
            tap_key(&mut app, KeyCode::KeyR);
            assert_eq!(
                app.world().resource::<ArenaReset>().generation,
                generation + 1
            );
            let state = app.world().resource::<ViewState>();
            assert!(!state.started && state.paused);
        }
    }
}

fn ready(hz: u32) -> (App, Entity) {
    let (mut app, window) = menu_app_at(hz);
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
    tap_key(&mut app, KeyCode::Enter);
    frame(&mut app);
    (app, window)
}

fn frame(app: &mut App) {
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear();
}

fn mouse(app: &mut App, button: MouseButton, down: bool) {
    let mut input = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
    if down {
        input.press(button);
    } else {
        input.release(button);
    }
}

fn charge(app: &App) -> Option<Spell> {
    app.world()
        .resource::<ArenaSession>()
        .actors
        .first()
        .and_then(|actor| actor.charge())
        .map(|charge| charge.spell)
}

fn casts(app: &App, spell: Spell) -> u32 {
    app.world()
        .resource::<ArenaSession>()
        .round_summary()
        .actors
        .first()
        .and_then(|actor| actor.casts.get(spell.index()))
        .copied()
        .unwrap_or_default()
}

#[test]
fn direct_mouse_buttons_hold_then_release_their_own_spell() {
    for (button, spell) in [
        (MouseButton::Left, Spell::Fireball),
        (MouseButton::Right, Spell::Shield),
    ] {
        let (mut app, _) = ready(60);
        mouse(&mut app, button, true);
        frame(&mut app);
        assert_eq!(charge(&app), Some(spell));
        assert_eq!(casts(&app, spell), 0);
        mouse(&mut app, button, false);
        frame(&mut app);
        assert!(charge(&app).is_none());
        assert_eq!(casts(&app, spell), 1);
        assert!(app
            .world()
            .resource::<ArenaSession>()
            .actors
            .first()
            .expect("human")
            .cooldowns
            .get(spell.index())
            .is_some_and(|cd| *cd > 0.0));
    }
}

#[test]
fn first_mouse_owner_ignores_other_press_and_release_until_a_fresh_gesture() {
    for (first, second, owner, other) in [
        (
            MouseButton::Left,
            MouseButton::Right,
            Spell::Fireball,
            Spell::Shield,
        ),
        (
            MouseButton::Right,
            MouseButton::Left,
            Spell::Shield,
            Spell::Fireball,
        ),
    ] {
        let (mut app, _) = ready(60);
        mouse(&mut app, first, true);
        frame(&mut app);
        mouse(&mut app, second, true);
        frame(&mut app);
        assert_eq!(charge(&app), Some(owner));
        mouse(&mut app, second, false);
        frame(&mut app);
        assert_eq!(charge(&app), Some(owner));
        assert_eq!(casts(&app, owner), 0);
        mouse(&mut app, first, false);
        frame(&mut app);
        assert_eq!(casts(&app, owner), 1);
        assert_eq!(casts(&app, other), 0);
        mouse(&mut app, second, true);
        frame(&mut app);
        assert_eq!(charge(&app), Some(other));
    }
}

#[test]
fn simultaneous_snapshots_use_left_priority_but_native_events_preserve_right_first() {
    for ordered_events in [false, true] {
        let (mut app, window) = ready(60);
        mouse(&mut app, MouseButton::Right, true);
        mouse(&mut app, MouseButton::Left, true);
        if ordered_events {
            for button in [MouseButton::Right, MouseButton::Left] {
                app.world_mut().write_message(MouseButtonInput {
                    button,
                    state: ButtonState::Pressed,
                    window,
                });
            }
        }
        frame(&mut app);
        let expected = if ordered_events {
            Spell::Shield
        } else {
            Spell::Fireball
        };
        assert_eq!(charge(&app), Some(expected));
        for button in [MouseButton::Left, MouseButton::Right] {
            mouse(&mut app, button, false);
        }
        if ordered_events {
            for button in [MouseButton::Left, MouseButton::Right] {
                app.world_mut().write_message(MouseButtonInput {
                    button,
                    state: ButtonState::Released,
                    window,
                });
            }
        }
        frame(&mut app);
        assert_eq!(casts(&app, expected), 1);
        let other = if ordered_events {
            Spell::Fireball
        } else {
            Spell::Shield
        };
        assert_eq!(casts(&app, other), 0);
    }
}

#[test]
fn two_successive_taps_before_physics_remain_two_ordered_spells() {
    let (mut app, _) = ready(1920);
    app.world_mut().resource_mut::<ViewState>().accumulator = 0.0;
    let before = app.world().resource::<ArenaSession>().tick;
    for button in [MouseButton::Left, MouseButton::Right] {
        mouse(&mut app, button, true);
        frame(&mut app);
        mouse(&mut app, button, false);
        frame(&mut app);
    }
    assert_eq!(app.world().resource::<ArenaSession>().tick, before);
    for _ in 0..40 {
        frame(&mut app);
    }
    assert_eq!(casts(&app, Spell::Fireball), 1);
    assert_eq!(casts(&app, Spell::Shield), 1);
    assert!(charge(&app).is_none());
}

#[test]
fn held_fireball_starts_when_ready_and_keeps_gesture_ownership() {
    let (mut app, _) = ready(60);
    *app.world_mut()
        .resource_mut::<ArenaSession>()
        .actors
        .first_mut()
        .expect("human")
        .cooldowns
        .get_mut(1)
        .expect("Fireball slot") = 0.15;
    mouse(&mut app, MouseButton::Left, true);
    frame(&mut app);
    for _ in 0..15 {
        frame(&mut app);
    }
    assert_eq!(charge(&app), Some(Spell::Fireball));
    mouse(&mut app, MouseButton::Right, true);
    frame(&mut app);
    assert_eq!(charge(&app), Some(Spell::Fireball));
    mouse(&mut app, MouseButton::Left, false);
    frame(&mut app);
    for _ in 0..3 {
        frame(&mut app);
    }
    assert!(charge(&app).is_none());
    assert_eq!(casts(&app, Spell::Fireball), 1);
    assert_eq!(casts(&app, Spell::Shield), 0);
    mouse(&mut app, MouseButton::Right, false);
    frame(&mut app);
    mouse(&mut app, MouseButton::Right, true);
    frame(&mut app);
    assert_eq!(charge(&app), Some(Spell::Shield));
}

#[test]
fn paused_mouse_quarantine_waits_for_both_buttons_to_be_released() {
    let (mut app, _) = ready(60);
    mouse(&mut app, MouseButton::Left, true);
    frame(&mut app);
    tap_key(&mut app, KeyCode::Escape);
    mouse(&mut app, MouseButton::Right, true);
    frame(&mut app);
    tap_key(&mut app, KeyCode::Escape);
    assert!(charge(&app).is_none());
    mouse(&mut app, MouseButton::Left, false);
    frame(&mut app);
    assert!(app.world().resource::<ViewState>().suppress_click);
    assert!(charge(&app).is_none());
    mouse(&mut app, MouseButton::Right, false);
    frame(&mut app);
    assert!(!app.world().resource::<ViewState>().suppress_click);
    assert_eq!(casts(&app, Spell::Fireball), 0);
    assert_eq!(casts(&app, Spell::Shield), 0);
    mouse(&mut app, MouseButton::Right, true);
    frame(&mut app);
    assert_eq!(charge(&app), Some(Spell::Shield));
}

#[test]
fn removed_number_keys_preserve_charge_and_e_is_the_high_jump_key() {
    let (mut app, _) = ready(60);
    mouse(&mut app, MouseButton::Left, true);
    frame(&mut app);
    for key in [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3] {
        tap_key(&mut app, key);
    }
    assert_eq!(charge(&app), Some(Spell::Fireball));
    assert_eq!(casts(&app, Spell::HighJump), 0);
    tap_key(&mut app, KeyCode::KeyE);
    assert_eq!(casts(&app, Spell::HighJump), 1);
    assert_eq!(charge(&app), Some(Spell::Fireball));
}

#[test]
fn native_shift_does_not_change_the_four_point_five_unit_movement_speed() {
    for shift in [false, true] {
        let (mut app, _) = ready(60);
        let start = app
            .world()
            .resource::<ArenaSession>()
            .actors
            .first()
            .expect("human")
            .feet;
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.press(KeyCode::KeyW);
            if shift {
                keys.press(KeyCode::ShiftLeft);
            }
        }
        for _ in 0..60 {
            frame(&mut app);
        }
        let actor = app
            .world()
            .resource::<ArenaSession>()
            .actors
            .first()
            .expect("human");
        let distance = actor.feet.with_y(0.0).distance(start.with_y(0.0));
        assert!((distance - 4.5).abs() < 0.05, "shift={shift}: {distance}");
        assert!(!app.world().resource::<ArenaInput>().human.run);
    }
}

fn grand_streaming_controls() -> App {
    use hex_core::arena::{ArenaExpeditionSites, ArenaMaterials, ArenaResidency, ArenaSolidSpan};
    use hex_core::{ElementId, HexCoord, SubstanceId, TilePos};

    let geometry = ArenaVoxelGeometry {
        radius: 32,
        ..default()
    };
    let chunks = (-2..=2)
        .flat_map(|q| (-2..=2).map(move |r| (q, r)))
        .collect::<std::collections::BTreeSet<_>>();
    let terrain = ArenaTerrainView {
        revision: 1,
        selection: ArenaSelection {
            map: ArenaMap::GrandV4,
            ..default()
        },
        spawns: [Vec3::Y * 0.0001, Vec3::X * 8.0],
        columns: HexCoord::ORIGIN
            .within_radius(32)
            .into_iter()
            .map(|coord| {
                (
                    coord,
                    vec![ArenaSolidSpan {
                        bottom: TilePos::new(coord, 0),
                        top_level: 0,
                        substance: SubstanceId(1),
                    }],
                )
            })
            .collect(),
        residency: Some(ArenaResidency {
            catalogue: chunks.clone(),
            ready: chunks,
        }),
        expedition: Some(ArenaExpeditionSites::default()),
        ..default()
    };
    let mut builder = HeadlessAppBuilder::new()
        .with_minimal_plugins()
        .with_fixed_step(std::time::Duration::from_secs_f64(1.0 / 60.0));
    builder
        .app_mut()
        .insert_resource(terrain)
        .insert_resource(geometry)
        .insert_resource(ArenaMaterials {
            stone: SubstanceId(1),
            reinforced_stone: None,
            bedrock: SubstanceId(2),
            grass: SubstanceId(3),
            dirt: SubstanceId(4),
            fire: ElementId(1),
        })
        .init_resource::<ArenaReset>()
        .insert_resource(ViewState {
            started: true,
            paused: false,
            capture: None,
            ..default()
        })
        .add_plugins(hex_arena::plugin)
        .add_systems(Update, drive_simulation);
    let mut app = builder.build();
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
    app.update();
    app.update();
    let session = app.world().resource::<ArenaSession>();
    assert!(
        session.is_grand_run() && session.tick > 0,
        "{}",
        session.notice
    );
    app
}

fn set_grand_controls_ready(app: &mut App, ready: bool) {
    let mut terrain = app.world_mut().resource_mut::<ArenaTerrainView>();
    let residency = terrain.residency.as_mut().expect("finite fixture");
    residency.ready = if ready {
        residency.catalogue.clone()
    } else {
        default()
    };
    terrain.revision += 1;
    terrain.full_rebuild = true;
}

#[test]
fn grand_streaming_wait_preserves_restored_release_and_fresh_press_until_consumed() {
    for restored_release in [false, true] {
        let mut app = grand_streaming_controls();
        let mut mouse = ButtonInput::<MouseButton>::default();
        if restored_release {
            app.world_mut().resource_mut::<ArenaInput>().human = ActorIntent {
                aim: Vec3::X,
                selected: Some(Spell::Fireball),
                cast_pressed: true,
                cast_held: true,
                ..default()
            };
            tick(&mut app);
            assert_eq!(charge(&app), Some(Spell::Fireball));
            let identity = hex_arena::GrandCheckpointIdentity {
                world_id: "controls-fixture".into(),
                content_revision: "streaming-input".into(),
            };
            let bytes = app
                .world()
                .resource::<ArenaSession>()
                .encode_grand_checkpoint(&identity)
                .expect("armed checkpoint");
            let restored = ArenaSession::decode_grand_checkpoint(
                &bytes,
                &identity,
                app.world().resource::<ArenaTerrainView>(),
                *app.world().resource::<ArenaVoxelGeometry>(),
                app.world().resource::<ArenaReset>().generation,
            )
            .expect("restored armed checkpoint");
            app.world_mut().insert_resource(restored);
            app.world_mut()
                .resource_mut::<ViewState>()
                .casts
                .resume_existing(Spell::Fireball, false, Vec3::X);
        } else {
            mouse.press(MouseButton::Left);
            app.world_mut()
                .resource_mut::<ViewState>()
                .casts
                .observe(&mouse, &[], Vec3::X);
            mouse.clear();
        }
        set_grand_controls_ready(&mut app, false);
        let waiting_tick = app.world().resource::<ArenaSession>().tick;
        for _ in 0..3 {
            app.update();
            assert_eq!(app.world().resource::<ArenaSession>().tick, waiting_tick);
            assert_eq!(casts(&app, Spell::Fireball), 0);
            let mut queued = ActorIntent::default();
            app.world().resource::<ViewState>().casts.write(&mut queued);
            assert_eq!(queued.cast_pressed, !restored_release);
            assert_eq!(queued.cast_released, restored_release);
        }
        set_grand_controls_ready(&mut app, true);
        app.update();
        assert!(app.world().resource::<ArenaSession>().tick > waiting_tick);
        let mut consumed = ActorIntent::default();
        app.world()
            .resource::<ViewState>()
            .casts
            .write(&mut consumed);
        assert!(!consumed.cast_pressed && !consumed.cast_released);
        if !restored_release {
            assert_eq!(charge(&app), Some(Spell::Fireball));
            assert_eq!(casts(&app, Spell::Fireball), 0);
            mouse.release(MouseButton::Left);
            app.world_mut()
                .resource_mut::<ViewState>()
                .casts
                .observe(&mouse, &[], Vec3::X);
        }
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(casts(&app, Spell::Fireball), 1);
        assert!(charge(&app).is_none());
    }
}
