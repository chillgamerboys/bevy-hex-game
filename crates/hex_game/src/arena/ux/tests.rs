//! Renderer-free integration checks through actual Bevy layout, input and UI systems.
#![cfg(feature = "test-support")]

use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::mouse::MouseButtonInput;
use bevy::input::ButtonState;
use bevy::window::{CursorOptions, WindowMode};
use hex_arena::{ActorIntent, ArenaBattleSetup, ArenaInput, Spell};
use hex_core::arena::{ArenaMap, ArenaSelection, ArenaTick};
use hex_ui::test_support::HeadlessUiPlugin;

use super::*;

fn settle(app: &mut App) {
    for _ in 0..6 {
        app.update();
    }
}

fn app(width: u32, height: u32, scale: f32) -> App {
    let mut app = App::new();
    app.add_plugins(HeadlessUiPlugin::new(width, height))
        .insert_resource(ViewState {
            started: false,
            paused: true,
            capture: None,
            ..default()
        })
        .init_resource::<ArenaSession>()
        .init_resource::<ArenaTuning>()
        .init_resource::<ArenaInput>()
        .init_resource::<ArenaBattleSetup>()
        .init_resource::<ArenaReset>()
        .init_resource::<ArenaSelection>()
        .init_resource::<ArenaTerrainView>()
        .init_resource::<ArenaVoxelGeometry>()
        .init_resource::<ArenaOverview>()
        .add_systems(Startup, hud::setup)
        .add_systems(
            Update,
            (
                super::super::input,
                keyboard,
                controls,
                hud::buttons,
                hud::update,
            )
                .chain(),
        );
    install(&mut app);
    settle(&mut app);
    app.world_mut().resource_mut::<UxState>().scale = scale;
    let window = window(&mut app);
    app.world_mut()
        .get_mut::<Window>(window)
        .expect("window")
        .focused = true;
    if app.world().get::<CursorOptions>(window).is_none() {
        app.world_mut()
            .entity_mut(window)
            .insert(CursorOptions::default());
    }
    settle(&mut app);
    app
}

fn window(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(app.world())
        .expect("one synthetic window")
}

fn rect(world: &World, entity: Entity) -> Rect {
    let node = world.get::<ComputedNode>(entity).expect("computed UI node");
    let transform = world
        .get::<UiGlobalTransform>(entity)
        .expect("computed transform");
    Rect::from_center_size(transform.affine().translation, node.size())
}

fn in_panel<T: Component>(world: &World, mut entity: Entity) -> bool {
    loop {
        if world.get::<T>(entity).is_some() {
            return true;
        }
        let Some(parent) = world.get::<ChildOf>(entity) else {
            return false;
        };
        entity = parent.parent();
    }
}

fn button(app: &mut App, matches: impl Fn(&hud::Action) -> bool, paused: bool) -> Entity {
    app.world_mut()
        .query::<(Entity, &hud::Action)>()
        .iter(app.world())
        .find_map(|(entity, action)| {
            (matches(action)
                && if paused {
                    in_panel::<hud::PausePanel>(app.world(), entity)
                } else {
                    in_panel::<hud::StartPanel>(app.world(), entity)
                })
            .then_some(entity)
        })
        .expect("authored button")
}

fn click(app: &mut App, entity: Entity) {
    let position = rect(app.world(), entity).center();
    click_at(app, position);
}

fn click_at(app: &mut App, position: Vec2) {
    let window = window(app);
    app.world_mut()
        .get_mut::<Window>(window)
        .expect("window")
        .set_physical_cursor_position(Some(position.into()));
    app.world_mut().write_message(MouseButtonInput {
        button: MouseButton::Left,
        state: ButtonState::Pressed,
        window,
    });
    app.update();
    app.world_mut().write_message(MouseButtonInput {
        button: MouseButton::Left,
        state: ButtonState::Released,
        window,
    });
    app.update();
}

fn key(app: &mut App, code: KeyCode) {
    let window = window(app);
    for state in [ButtonState::Pressed, ButtonState::Released] {
        app.world_mut().write_message(KeyboardInput {
            key_code: code,
            logical_key: match code {
                KeyCode::Enter => Key::Enter,
                KeyCode::ArrowDown => Key::ArrowDown,
                KeyCode::Escape => Key::Escape,
                _ => Key::Character("m".into()),
            },
            state,
            text: None,
            repeat: false,
            window,
        });
        app.update();
    }
}

fn open_page(app: &mut App, page: Page) {
    let entity = app
        .world_mut()
        .query::<(Entity, &UxAction)>()
        .iter(app.world())
        .find_map(|(entity, action)| {
            matches!(action, UxAction::Page(p) if *p == page).then_some(entity)
        })
        .expect("page button");
    click(app, entity);
    settle(app);
    assert_eq!(app.world().resource::<UxState>().page, page);
}

fn assert_visible(app: &mut App, entity: Entity, label: &str) {
    // Battle owns UiScale independently of the tactical ResolvedUiMetrics used
    // by ui_tree_snapshot. Comparing that snapshot's UI-logical coordinates to
    // tactical window-logical bounds falsely clips e.g. a physical (640, 656)
    // Start button: at Battle scale 2/3 it is reported as (960, 984). Keep every
    // quantity here in the same physical pixels as Bevy's actual layout/picking.
    let window = window(app);
    let viewport = app
        .world()
        .get::<Window>(window)
        .expect("window")
        .physical_size()
        .as_vec2();
    let world = app.world();
    let bounds = rect(world, entity);
    let ui_scale = world.resource::<UiScale>().0;
    let fits = |outer: Rect, inner: Rect| {
        inner.min.cmpge(outer.min - Vec2::splat(0.5)).all()
            && inner.max.cmple(outer.max + Vec2::splat(0.5)).all()
    };
    assert!(
        bounds.size().min_element() > 0.0 && fits(Rect::from_corners(Vec2::ZERO, viewport), bounds),
        "{label}: physical bounds {bounds:?}, viewport {viewport:?}, Battle UiScale {ui_scale}"
    );
    if let Some(clip) = world.get::<bevy::ui::CalculatedClip>(entity) {
        assert!(fits(clip.clip, bounds),
            "{label}: physical bounds {bounds:?} clipped by {:?}, viewport {viewport:?}, Battle UiScale {ui_scale}", clip.clip);
    }
    let mut current = Some(entity);
    while let Some(ancestor) = current {
        if let Some(node) = world.get::<Node>(ancestor) {
            assert!(
                node.display != Display::None,
                "{label}: hidden ancestor {ancestor:?}"
            );
            let area = rect(world, ancestor);
            if !node.overflow.x.is_visible() {
                assert!(bounds.min.x >= area.min.x - 0.5 && bounds.max.x <= area.max.x + 0.5,
                    "{label}: bounds {bounds:?} horizontally clipped by {ancestor:?} {area:?}, viewport {viewport:?}, Battle UiScale {ui_scale}");
            }
            if !node.overflow.y.is_visible() {
                assert!(bounds.min.y >= area.min.y - 0.5 && bounds.max.y <= area.max.y + 0.5,
                    "{label}: bounds {bounds:?} vertically clipped by {ancestor:?} {area:?}, viewport {viewport:?}, Battle UiScale {ui_scale}");
            }
        }
        if let Some(visibility) = world.get::<InheritedVisibility>(ancestor) {
            assert!(visibility.get(), "{label}: invisible ancestor {ancestor:?}");
        }
        current = world.get::<ChildOf>(ancestor).map(ChildOf::parent);
    }
}

#[test]
fn primary_actions_fit_all_window_sizes_and_scales_without_scrolling() {
    for (width, height) in [(1280, 720), (1600, 900), (1920, 1080)] {
        for scale in [1.0, 2.0] {
            let mut app = app(width, height, scale);
            let start = button(&mut app, |a| matches!(a, hud::Action::Start), false);
            let quit = button(&mut app, |a| matches!(a, hud::Action::Quit), false);
            assert_visible(&mut app, start, "Ready Start");
            assert_visible(&mut app, quit, "Ready Quit");
            app.world_mut().resource_mut::<ViewState>().started = true;
            settle(&mut app);
            for page in Page::ALL {
                app.world_mut().resource_mut::<UxState>().page = page;
                settle(&mut app);
                for (which, label) in [
                    (0, "Paused Resume"),
                    (1, "Paused Restart"),
                    (2, "Paused Quit"),
                ] {
                    let entity = button(
                        &mut app,
                        |a| {
                            matches!(
                                (which, a),
                                (0, hud::Action::Resume)
                                    | (1, hud::Action::Restart)
                                    | (2, hud::Action::Quit)
                            )
                        },
                        true,
                    );
                    assert_visible(&mut app, entity, label);
                }
                let resume = button(&mut app, |a| matches!(a, hud::Action::Resume), true);
                let restart = button(&mut app, |a| matches!(a, hud::Action::Restart), true);
                assert!(
                    rect(app.world(), resume).max.x <= rect(app.world(), restart).min.x + 0.1,
                    "separate fixed actions at {width}x{height}, scale {scale}"
                );
            }
        }
    }
}

#[test]
fn page_pointer_switches_one_body_and_keyboard_scroll_reaches_last_upgrade() {
    for (width, height) in [(1280, 720), (1600, 900), (1920, 1080)] {
        for scale in [1.0, 2.0] {
            let mut app = app(width, height, scale);
            app.world_mut().resource_mut::<ViewState>().started = true;
            settle(&mut app);
            open_page(&mut app, Page::Upgrades);
            let bodies = app
                .world_mut()
                .query::<(&PageBody, &ComputedNode)>()
                .iter(app.world())
                .filter(|(_, node)| node.size().min_element() > 0.0)
                .map(|(body, _)| body.0)
                .collect::<Vec<_>>();
            assert_eq!(bodies, vec![Page::Upgrades]);
            let target = button(
                &mut app,
                |a| matches!(a, hud::Action::Change(11, n) if *n > 0.0),
                true,
            );
            let mut reached = false;
            for _ in 0..120 {
                key(&mut app, KeyCode::ArrowDown);
                if app.world().resource::<UxState>().focus == Some(target) {
                    reached = true;
                    break;
                }
            }
            assert!(
                reached,
                "final upgrade reachable by keyboard at {width}x{height}, {scale}"
            );
            settle(&mut app);
            assert_visible(&mut app, target, "Focused final upgrade");
            open_page(&mut app, Page::Map);
            assert_eq!(
                app.world()
                    .get::<ComputedNode>(target)
                    .expect("button")
                    .size(),
                Vec2::ZERO
            );
        }
    }
}

#[test]
fn focused_ready_control_activates_with_enter_without_starting_combat() {
    let mut app = app(1280, 720, 2.0);
    let fullscreen = button(&mut app, |a| matches!(a, hud::Action::Fullscreen), false);
    app.world_mut().resource_mut::<UxState>().focus = Some(fullscreen);
    key(&mut app, KeyCode::Enter);
    assert!(!app.world().resource::<ViewState>().started);
    assert!(app.world().resource::<ViewState>().paused);
    let window = window(&mut app);
    assert!(app.world().get::<Window>(window).expect("window").mode != WindowMode::Windowed);
    app.world_mut().resource_mut::<UxState>().focus = None;
    key(&mut app, KeyCode::Enter);
    assert!(app.world().resource::<ViewState>().started);
    assert!(!app.world().resource::<ViewState>().paused);
}

#[test]
fn scrolled_settings_header_clicks_change_page_without_activating_clipped_controls() {
    for (width, height) in [(1280, 720), (1600, 900), (1920, 1080)] {
        for destination in [Page::Map, Page::Upgrades, Page::Controls] {
            let mut app = app(width, height, 2.0);
            {
                let mut state = app.world_mut().resource_mut::<ViewState>();
                state.started = true;
                // Keep accidental Scale activation from writing real preferences.
                // This harness has no render-capture systems installed.
                state.capture = Some("pointer-layout-only.png".into());
            }
            let detail = app
                .world_mut()
                .query::<(Entity, &UxLabel)>()
                .iter(app.world())
                .find_map(|(entity, label)| {
                    matches!(label, UxLabel::RecorderDetail).then_some(entity)
                })
                .expect("recording detail label");
            // Match the native one-line recorder footer without starting OS IPC.
            app.world_mut().entity_mut(detail).remove::<UxLabel>();
            app.world_mut().get_mut::<Text>(detail).expect("footer").0 =
                "Recording video · F9 adds a bookmark".into();
            settle(&mut app);
            open_page(&mut app, Page::Settings);
            let window = window(&mut app);
            app.world_mut().write_message(MouseWheel {
                unit: MouseScrollUnit::Line,
                x: 0.0,
                y: -100.0,
                phase: bevy::input::touch::TouchPhase::Moved,
                window,
            });
            settle(&mut app);
            assert!(
                app.world_mut()
                    .query_filtered::<&ScrollPosition, With<MenuScroll>>()
                    .iter(app.world())
                    .any(|scroll| scroll.0.y > 1.0),
                "Settings must actually be scrolled at {width}x{height}"
            );
            let tab = app
                .world_mut()
                .query::<(Entity, &UxAction)>()
                .iter(app.world())
                .find_map(|(entity, action)| {
                    matches!(action, UxAction::Page(page) if *page == destination).then_some(entity)
                })
                .expect("visible destination tab");
            assert_visible(&mut app, tab, "Header above the scrolled Settings body");
            click(&mut app, tab);
            settle(&mut app);
            let ux = app.world().resource::<UxState>();
            assert!((ux.scale - 2.0).abs() < f32::EPSILON,
                "{destination:?} header click activated clipped Scale control at {width}x{height}: {}", ux.scale);
            assert_eq!(
                ux.page, destination,
                "visible header tab owns its pointer click at {width}x{height}"
            );
        }
    }
}

fn overview() -> ArenaOverview {
    ArenaOverview {
        generation: 7,
        width: 4,
        height: 2,
        min: Vec2::new(-100.0, -50.0),
        max: Vec2::new(100.0, 50.0),
        rgba: vec![128; 4 * 2 * 4],
    }
}

#[test]
fn expanded_atlas_and_clear_destination_fit_every_window_and_scale() {
    for (width, height) in [(1280, 720), (1600, 900), (1920, 1080)] {
        for scale in [1.0, 2.0] {
            let mut app = app(width, height, scale);
            let mut atlas = overview();
            // Exercise the finite hex region's near-square world bounds, not
            // the much shallower 2:1 pointer fixture above.
            atlas.min = Vec2::new(-100.0, -86.6);
            atlas.max = -atlas.min;
            app.world_mut().insert_resource(atlas);
            app.world_mut().resource_mut::<ViewState>().started = true;
            app.world_mut().resource_mut::<UxState>().pin = Some(Vec2::ZERO);
            settle(&mut app);
            open_page(&mut app, Page::Map);
            let map = app
                .world_mut()
                .query::<(Entity, &MapCanvas)>()
                .iter(app.world())
                .find_map(|(entity, canvas)| canvas.large.then_some(entity))
                .expect("expanded atlas");
            let clear = app
                .world_mut()
                .query::<(Entity, &UxAction)>()
                .iter(app.world())
                .find_map(|(entity, action)| matches!(action, UxAction::ClearPin).then_some(entity))
                .expect("clear destination");
            assert_visible(&mut app, map, "Entire expanded atlas");
            assert_visible(&mut app, clear, "Clear destination beside atlas");
            let bounds = rect(app.world(), map);
            assert!(
                (bounds.height() / bounds.width() - 0.866).abs() < 0.01,
                "atlas preserves world aspect at {width}x{height}, scale {scale}: {bounds:?}"
            );
            click(&mut app, clear);
            assert!(
                app.world().resource::<UxState>().pin.is_none(),
                "visible destination control remains usable at {width}x{height}, scale {scale}"
            );
        }
    }
}

#[test]
fn overflowing_upgrade_page_has_a_visible_scroll_hint() {
    for (width, height) in [(1280, 720), (1600, 900), (1920, 1080)] {
        for scale in [1.0, 2.0] {
            let mut app = app(width, height, scale);
            app.world_mut().resource_mut::<ViewState>().started = true;
            settle(&mut app);
            open_page(&mut app, Page::Upgrades);
            let scroll = app
                .world_mut()
                .query_filtered::<Entity, With<MenuScroll>>()
                .iter(app.world())
                .find(|entity| in_panel::<hud::PausePanel>(app.world(), *entity))
                .expect("paused page scroll area");
            let node = app
                .world()
                .get::<ComputedNode>(scroll)
                .expect("scroll layout");
            assert!(
                node.content_size().y > node.size().y + 2.0,
                "fixture must contain hidden upgrade rows at {width}x{height}, scale {scale}"
            );
            let hint = app
                .world_mut()
                .query_filtered::<Entity, With<MenuScrollHint>>()
                .iter(app.world())
                .find(|entity| in_panel::<hud::PausePanel>(app.world(), *entity))
                .expect("paused scroll hint");
            assert_visible(&mut app, hint, "Upgrade scroll hint outside clipped rows");
            let label = &app.world().get::<Text>(hint).expect("scroll hint text").0;
            assert!(
                label.contains("Scroll") && label.contains('↓'),
                "hidden lower upgrades must advertise downward scrolling: {label:?}"
            );
        }
    }
}

#[test]
fn recording_badge_stays_above_menu_headings_at_every_supported_scale() {
    for (width, height) in [(1280, 720), (1600, 900), (1920, 1080)] {
        for scale in [1.0, 2.0] {
            for started in [false, true] {
                let mut app = app(width, height, scale);
                app.world_mut().resource_mut::<ViewState>().started = started;
                settle(&mut app);
                let badge = app
                    .world_mut()
                    .query::<(Entity, &UxLabel)>()
                    .iter(app.world())
                    .find_map(|(entity, label)| {
                        matches!(label, UxLabel::Recording).then_some(entity)
                    })
                    .expect("recording badge");
                app.world_mut()
                    .get_mut::<Node>(badge)
                    .expect("badge node")
                    .display = Display::Flex;
                app.world_mut()
                    .get_mut::<Text>(badge)
                    .expect("badge text")
                    .0 = "● REC 12:34".into();
                // Shape the actual recording label without installing or starting
                // an OS recorder in a deterministic layout regression.
                for _ in 0..6 {
                    app.world_mut().run_schedule(PostUpdate);
                }
                let heading = app
                    .world_mut()
                    .query::<(Entity, &Text)>()
                    .iter(app.world())
                    .find_map(|(entity, text)| {
                        (if started {
                            text.0.starts_with("PAUSED /")
                        } else {
                            text.0 == "BATTLE MODE"
                        })
                        .then_some(entity)
                    })
                    .expect("visible menu heading");
                assert_visible(&mut app, badge, "Recording badge");
                assert!(
                    rect(app.world(), badge).max.y < rect(app.world(), heading).min.y,
                    "REC overlaps heading at {width}x{height}, scale {scale}, started {started}"
                );
            }
        }
    }
}

#[test]
fn expedition_health_glyphs_fit_inside_the_combat_pill() {
    for (width, height) in [(1280, 720), (1600, 900), (1920, 1080)] {
        for scale in [1.0, 2.0] {
            let mut app = app(width, height, scale);
            app.world_mut().resource_mut::<ViewState>().begin_play();
            settle(&mut app);
            let health = app
                .world_mut()
                .query::<(Entity, &hud::Label)>()
                .iter(app.world())
                .find_map(|(entity, label)| {
                    (matches!(label, hud::Label::Health)
                        && in_panel::<hud::CombatHud>(app.world(), entity))
                    .then_some(entity)
                })
                .expect("combat health pill");
            app.world_mut()
                .get_mut::<Text>(health)
                .expect("health text")
                .0 = "100 / 100 HP".into();
            // Run actual text shaping/layout without the synthetic empty
            // session replacing the expedition string in the Update adapter.
            for _ in 0..6 {
                app.world_mut().run_schedule(PostUpdate);
            }
            assert_visible(&mut app, health, "Combat health pill");
            let world = app.world();
            let bounds = rect(world, health);
            let node = world.get::<ComputedNode>(health).expect("health layout");
            let transform = world
                .get::<UiGlobalTransform>(health)
                .expect("health transform");
            let layout = world
                .get::<bevy::text::TextLayoutInfo>(health)
                .expect("shaped health text");
            assert!(
                !layout.glyphs.is_empty(),
                "real health glyphs must be shaped"
            );
            let local_to_world = bevy::math::Affine2::from(*transform)
                * bevy::math::Affine2::from_translation(node.content_box().min);
            for glyph in &layout.glyphs {
                let half = glyph.atlas_info.rect.size() * 0.5;
                for corner in [
                    Vec2::new(-half.x, -half.y),
                    Vec2::new(half.x, -half.y),
                    half,
                    Vec2::new(-half.x, half.y),
                ] {
                    let point = local_to_world.transform_point2(glyph.position + corner);
                    assert!(
                        point.cmpge(bounds.min - Vec2::splat(0.5)).all()
                            && point.cmple(bounds.max + Vec2::splat(0.5)).all(),
                        "health glyph at {point:?} escapes pill {bounds:?} at {width}x{height}, scale {scale}"
                    );
                }
            }
        }
    }
}

#[test]
fn map_pointer_uses_world_bounds_and_empty_overview_clears_old_texture() {
    let mut app = app(1600, 900, 1.0);
    app.world_mut().insert_resource(overview());
    app.world_mut().resource_mut::<ArenaVoxelGeometry>().radius = 40;
    app.world_mut().resource_mut::<ViewState>().started = true;
    settle(&mut app);
    open_page(&mut app, Page::Map);
    let map = app
        .world_mut()
        .query::<(Entity, &MapCanvas)>()
        .iter(app.world())
        .find_map(|(entity, canvas)| canvas.large.then_some(entity))
        .expect("large map");
    let bounds = rect(app.world(), map);
    click_at(&mut app, bounds.min + bounds.size() * Vec2::new(0.25, 0.25));
    let pin = app.world().resource::<UxState>().pin.expect("personal pin");
    assert!(
        pin.distance(Vec2::new(-50.0, -25.0)) < 1.0,
        "north-up quarter click: {pin}"
    );
    click_at(&mut app, bounds.min + bounds.size() * Vec2::splat(0.01));
    assert_eq!(
        app.world().resource::<UxState>().pin,
        Some(pin),
        "outside-world click preserves the existing destination"
    );
    let old = app
        .world()
        .resource::<UxState>()
        .image
        .clone()
        .expect("overview texture");
    let bytes = vec![240; 4 * 2 * 4];
    app.world_mut().resource_mut::<ArenaOverview>().rgba = bytes.clone();
    settle(&mut app);
    let updated = app
        .world()
        .resource::<UxState>()
        .image
        .clone()
        .expect("refreshed texture");
    assert_ne!(
        old.id(),
        updated.id(),
        "same reset generation accepts changed world publication"
    );
    assert_eq!(
        app.world()
            .resource::<Assets<Image>>()
            .get(&updated)
            .expect("image")
            .data
            .as_deref(),
        Some(bytes.as_slice())
    );
    app.world_mut().insert_resource(ArenaOverview::default());
    settle(&mut app);
    assert!(app.world().resource::<UxState>().image.is_none());
    assert!(app
        .world()
        .resource::<Assets<Image>>()
        .get(&updated)
        .is_none());
    assert_eq!(
        app.world().get::<Node>(map).expect("map node").display,
        Display::None
    );
}

fn charged_session(spell: Spell, ticks: u32) -> ArenaSession {
    let mut fixture = App::new();
    fixture.add_plugins((MinimalPlugins, hex_map::arena::plugin, hex_arena::plugin));
    fixture.update();
    fixture.world_mut().run_schedule(ArenaTick);
    fixture
        .world_mut()
        .resource_mut::<ArenaSession>()
        .bot_enabled = false;
    fixture.world_mut().resource_mut::<ArenaInput>().human = ActorIntent {
        selected: Some(spell),
        cast_pressed: true,
        cast_held: true,
        ..default()
    };
    for _ in 0..ticks {
        fixture.world_mut().run_schedule(ArenaTick);
    }
    fixture
        .world_mut()
        .remove_resource::<ArenaSession>()
        .expect("authoritative session")
}

#[test]
fn spell_cards_render_actual_charge_cooldown_and_ready_state_below_reticle() {
    for spell in [Spell::Shield, Spell::Fireball] {
        let session = charged_session(spell, 60);
        assert!(session
            .actors
            .first()
            .and_then(|actor| actor.charge())
            .is_some());
        let mut app = app(1280, 720, 2.0);
        app.world_mut().insert_resource(session);
        app.world_mut().resource_mut::<ViewState>().begin_play();
        settle(&mut app);
        let feedback = app
            .world()
            .resource::<ArenaSession>()
            .combat_feedback(app.world().resource::<ArenaTuning>());
        let authoritative = feedback
            .spells
            .iter()
            .find(|s| s.spell == spell)
            .expect("spell");
        assert_eq!(authoritative.state, SpellAvailabilityState::Charging);
        let card_index = if spell == Spell::Shield { 0 } else { 1 };
        let fill = app
            .world_mut()
            .query::<(Entity, &SpellFill)>()
            .iter(app.world())
            .find_map(|(entity, fill)| (fill.0 == card_index).then_some(entity))
            .expect("charge fill");
        let parent = app
            .world()
            .get::<ChildOf>(fill)
            .expect("track parent")
            .parent();
        let ratio = rect(app.world(), fill).width() / rect(app.world(), parent).width();
        assert!((ratio - authoritative.charge_fraction).abs() < 0.015);
        let cards = app
            .world_mut()
            .query::<(Entity, &hud::SpellCard)>()
            .iter(app.world())
            .map(|(entity, _)| entity)
            .collect::<Vec<_>>();
        assert_eq!(cards.len(), 3);
        for (index, entity) in cards.into_iter().enumerate() {
            assert_visible(&mut app, entity, &format!("Combat spell card {index}"));
            assert!(
                rect(app.world(), entity).min.y > 360.0,
                "spell bar below center reticle"
            );
        }
        {
            let mut session = app.world_mut().resource_mut::<ArenaSession>();
            session.cancel_charges();
            session.actors.first_mut().expect("player").cooldowns = [0.5, 0.5, 0.5];
        }
        settle(&mut app);
        let cooldown = app
            .world()
            .resource::<ArenaSession>()
            .combat_feedback(app.world().resource::<ArenaTuning>());
        assert!(cooldown
            .spells
            .iter()
            .all(|s| s.state == SpellAvailabilityState::CoolingDown));
        for (entity, fill) in app
            .world_mut()
            .query::<(Entity, &SpellFill)>()
            .iter(app.world())
        {
            let spell = cooldown.spells.get(fill.0).expect("known spell slot");
            let track = app
                .world()
                .get::<ChildOf>(entity)
                .expect("fill track")
                .parent();
            let fraction = rect(app.world(), entity).width() / rect(app.world(), track).width();
            let expected = (1.0 - spell.cooldown_remaining / spell.cooldown_total).clamp(0.0, 1.0);
            assert!(
                (fraction - expected).abs() < 0.015,
                "cooldown must use accepted spell timing"
            );
        }
        app.world_mut()
            .resource_mut::<ArenaSession>()
            .actors
            .first_mut()
            .expect("player")
            .cooldowns = [0.0; 3];
        settle(&mut app);
        let ready = app
            .world()
            .resource::<ArenaSession>()
            .combat_feedback(app.world().resource::<ArenaTuning>());
        assert!(ready
            .spells
            .iter()
            .any(|s| s.spell == Spell::HighJump && s.state == SpellAvailabilityState::Ready));
        let labels = app
            .world_mut()
            .query::<(&hud::Label, &Text)>()
            .iter(app.world())
            .filter_map(|(label, text)| {
                matches!(label, hud::Label::Spell(2)).then_some(text.0.clone())
            })
            .collect::<Vec<_>>();
        assert!(
            labels.iter().any(|text| text.contains("READY")),
            "ready text reflects gameplay: {labels:?}"
        );
    }
}

#[test]
#[ignore = "requires HEX_FOREST_WORLD pointing at the actual compiled expedition"]
fn actual_expedition_m_toggles_map_without_pausing_and_restart_clears_pin() {
    let mut fixture = App::new();
    fixture
        .add_plugins(MinimalPlugins)
        .insert_resource(ArenaSelection {
            map: ArenaMap::ForestMassif,
            ..default()
        })
        .add_plugins((hex_map::arena::plugin, hex_arena::plugin));
    fixture.update();
    fixture.world_mut().run_schedule(ArenaTick);
    assert!(fixture
        .world()
        .resource::<ArenaSession>()
        .expedition_progress()
        .is_some());
    let mut app = app(1600, 900, 1.0);
    app.world_mut().insert_resource(
        fixture
            .world_mut()
            .remove_resource::<ArenaSession>()
            .expect("actual session"),
    );
    app.world_mut().insert_resource(
        fixture
            .world_mut()
            .remove_resource::<ArenaOverview>()
            .expect("actual atlas"),
    );
    app.world_mut().resource_mut::<ViewState>().begin_play();
    settle(&mut app);
    assert!(!app.world().resource::<UxState>().map_visible);
    key(&mut app, KeyCode::KeyM);
    assert!(app.world().resource::<UxState>().map_visible);
    assert!(!app.world().resource::<ViewState>().paused);
    key(&mut app, KeyCode::KeyM);
    assert!(!app.world().resource::<UxState>().map_visible);
    assert!(!app.world().resource::<ViewState>().paused);
    app.world_mut().resource_mut::<UxState>().pin = Some(Vec2::ONE);
    app.world_mut().resource_mut::<ArenaReset>().generation += 1;
    settle(&mut app);
    assert!(app.world().resource::<UxState>().pin.is_none());
}

#[derive(Debug)]
struct WindOnlyFixture;
impl hex_core::ocean::OceanEnvironmentSampler for WindOnlyFixture {
    fn surface_at(
        &self,
        _: Vec2,
        _: f32,
        _: hex_core::ocean::OceanWaterColumn,
    ) -> Option<hex_core::ocean::OceanSurfaceSample> {
        None // This UI test never asks a fixture to authorize water or movement.
    }
}

#[test]
fn exploration_navigation_keys_use_published_overview_and_wind_without_progression() {
    for (width, height, scale) in [(1280, 720, 2.0), (1600, 900, 1.0), (1920, 1080, 1.0)] {
        let mut app = app(width, height, scale);
        app.world_mut().resource_mut::<ArenaSelection>().map = ArenaMap::NorthernArchipelago;
        app.world_mut().insert_resource(ArenaOverview {
            width: 2,
            height: 2,
            min: Vec2::splat(-100.0),
            max: Vec2::splat(100.0),
            rgba: vec![128; 16],
            ..default()
        });
        app.world_mut()
            .insert_resource(hex_core::ocean::OceanEnvironmentView {
                package_fingerprint: 1,
                sampler: std::sync::Arc::new(WindOnlyFixture),
                wind: hex_core::ocean::OceanWindProfile {
                    heading_radians: std::f32::consts::FRAC_PI_2,
                    speed: 10.0,
                },
            });
        assert!(app.world().resource::<ArenaSession>().progress().is_none());
        app.world_mut().resource_mut::<ViewState>().begin_play();
        settle(&mut app);
        key(&mut app, KeyCode::KeyM);
        assert!(app.world().resource::<UxState>().map_visible);
        assert!(!app.world().resource::<ViewState>().paused);
        key(&mut app, KeyCode::KeyV);
        assert!(app.world().resource::<UxState>().wind_visible);
        settle(&mut app);
        let map = app
            .world_mut()
            .query_filtered::<Entity, With<MiniMap>>()
            .single(app.world())
            .expect("minimap");
        let wind = app
            .world_mut()
            .query_filtered::<Entity, With<super::wind::WindPanel>>()
            .single(app.world())
            .expect("wind instrument");
        assert_visible(&mut app, map, "Exploration minimap");
        assert_visible(&mut app, wind, "Wind beside minimap");
        assert!(rect(app.world(), wind).max.x < rect(app.world(), map).min.x);
        let arrow = app
            .world_mut()
            .query_filtered::<&UiTransform, With<super::wind::WindArrow>>()
            .single(app.world())
            .expect("wind arrow");
        assert!(
            (arrow.rotation * Vec2::NEG_Y - Vec2::X).length() < 1e-5,
            "eastward wind points right when looking north"
        );
        let text = app
            .world_mut()
            .query_filtered::<&Text, With<super::wind::WindDetails>>()
            .single(app.world())
            .expect("wind speed");
        assert!(
            text.0.starts_with("E · 10.4"),
            "shared initial gust: {}",
            text.0
        );
        // Look changes must rotate the arrow even while the player and wind stand still.
        for third_person in [false, true] {
            for (yaw, expected) in [
                (-std::f32::consts::FRAC_PI_2, Vec2::NEG_Y), // Looking east: ahead.
                (std::f32::consts::FRAC_PI_2, Vec2::Y),      // Looking west: behind.
                (std::f32::consts::PI, Vec2::NEG_X),         // Looking south: left.
                (std::f32::consts::TAU, Vec2::X),            // Full turn: right again.
            ] {
                {
                    let mut view = app.world_mut().resource_mut::<ViewState>();
                    view.yaw = yaw;
                    view.pitch = 0.7;
                    view.third_person = third_person;
                }
                settle(&mut app);
                let arrow = app
                    .world_mut()
                    .query_filtered::<&UiTransform, With<super::wind::WindArrow>>()
                    .single(app.world())
                    .expect("view-relative wind arrow");
                assert!(
                    (arrow.rotation * Vec2::NEG_Y - expected).length() < 1e-5,
                    "wind direction relative to look: yaw={yaw}, third_person={third_person}"
                );
            }
        }
        key(&mut app, KeyCode::KeyM);
        assert!(!app.world().resource::<UxState>().map_visible);
        assert!(app.world().resource::<UxState>().wind_visible);
        assert_eq!(
            app.world().get::<Node>(wind).expect("wind node").right,
            px(24)
        );
        key(&mut app, KeyCode::Escape);
        key(&mut app, KeyCode::KeyV);
        assert!(
            app.world().resource::<UxState>().wind_visible,
            "pause ignores V"
        );
        assert_eq!(
            app.world().get::<Node>(wind).expect("wind node").display,
            Display::None
        );
        app.world_mut().resource_mut::<ArenaReset>().generation += 1;
        settle(&mut app);
        assert!(!app.world().resource::<UxState>().wind_visible);
        assert!(!app.world().resource::<UxState>().map_visible);
    }
}

#[test]
fn navigation_shortcuts_ignore_unavailable_overview_and_wind() {
    let mut app = app(1600, 900, 1.0);
    app.world_mut().resource_mut::<ViewState>().begin_play();
    key(&mut app, KeyCode::KeyM);
    key(&mut app, KeyCode::KeyV);
    assert!(!app.world().resource::<UxState>().map_visible);
    assert!(!app.world().resource::<UxState>().wind_visible);
}
