//! Real setup and presentation checks; no native window or synthetic creature pose.
use super::*;
use bevy::ecs::system::RunSystemOnce;
use bevy::light::NotShadowCaster;
use hex_arena::{ArenaBattleSetup, ArenaControl, BattlePreset, ProjectileAppearance, Species};

fn observer() -> App {
    let (mut fixture, _) = menu_app();
    fixture.insert_resource(ArenaBattleSetup::spectator(
        BattlePreset::Wisp,
        BattlePreset::Goblin,
        1,
    ));
    fixture.world_mut().resource_mut::<ArenaReset>().generation += 1;
    fixture.update();
    fixture
}

#[test]
fn wisp_recipes_use_fort_override_and_reset_to_originals_cleanly() {
    for preset in BattlePreset::WISP_SWARMS
        .into_iter()
        .chain([BattlePreset::Goblin])
    {
        let selection =
            launch_selection(Some("fort"), Some(preset.slug())).expect("Fort explicit count");
        let mut setup = ArenaBattleSetup::default();
        apply_player_recipe(&mut setup, selection, Some(preset.slug()))
            .expect("ready creature override");
        assert_eq!(setup.player_recipe, Some(preset));
        for map in ["duel", "seven-regions"] {
            assert!(launch_selection(Some(map), Some(preset.slug())).is_err());
        }
    }
    let (mut fixture, _) = menu_app();
    press_action(&mut fixture, hud::Action::Map(ArenaMap::Fort));
    press_action(
        &mut fixture,
        hud::Action::PlayerRecipe(BattlePreset::Wisps4),
    );
    assert_eq!(
        fixture
            .world()
            .resource::<ArenaSession>()
            .actors
            .iter()
            .filter(|a| a.species == Species::Wisp)
            .count(),
        4
    );
    press_action(&mut fixture, hud::Action::Start);
    tap_key(&mut fixture, KeyCode::KeyR);
    assert_eq!(
        fixture
            .world()
            .resource::<ArenaSession>()
            .accepted_battle_setup()
            .player_recipe,
        Some(BattlePreset::Wisps4)
    );
    press_action(&mut fixture, hud::Action::Control(ArenaControl::Spectator));
    assert!(fixture
        .world()
        .resource::<ArenaBattleSetup>()
        .player_recipe
        .is_none());
    press_action(&mut fixture, hud::Action::Control(ArenaControl::Player));
    press_action(&mut fixture, hud::Action::Encounter(ArenaEncounter::Dragon));
    assert!(fixture
        .world()
        .resource::<ArenaSession>()
        .actors
        .iter()
        .all(|a| a.species != Species::Wisp));
    for preset in BattlePreset::WISP_SWARMS {
        let setup = spectator::launch_setup(
            ArenaMap::Duel,
            true,
            Some(preset.slug()),
            Some("goblin"),
            Some("3"),
            Some("720"),
        )
        .expect("observer explicit swarm");
        assert_eq!(spectator::preset_for(&setup, 0), Some(preset));
    }
}

#[test]
fn wisp_actor_zero_is_one_visible_fixed_native_prism_without_fake_shadow() {
    let mut fixture = observer();
    let (feet, height) = {
        let session = fixture.world().resource::<ArenaSession>();
        assert!(session.human_actor_id().is_none());
        let actor = session.actors.first().expect("actor zero");
        assert_eq!(actor.species, Species::Wisp);
        assert!(actor.flying && !actor.grounded);
        assert!(actor
            .body_dimensions()
            .abs_diff_eq(Vec3::new(3.0_f32.sqrt(), 0.4, 2.0), 0.0001));
        assert!(actor.eye().abs_diff_eq(actor.center(), 0.0001));
        assert!(wisp::phase_actor(session, "encounter-wisp-body").is_some_and(|a| a.id == 0));
        (actor.feet, actor.body_dimensions().y)
    };
    fixture
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    fixture
        .world_mut()
        .run_system_once(golem::setup)
        .expect("Golem cached assets");
    fixture
        .world_mut()
        .run_system_once(wisp::setup)
        .expect("Wisp cached assets");
    fixture
        .world_mut()
        .run_system_once(worm::setup)
        .expect("Worm cached visuals");
    fixture
        .world_mut()
        .run_system_once(presentation::actors)
        .expect("actual body models");
    let models = fixture
        .world_mut()
        .query_filtered::<(
            &Transform,
            &ChildOf,
            &MeshMaterial3d<StandardMaterial>,
            Has<NotShadowCaster>,
        ), With<wisp::WispPrism>>()
        .iter(fixture.world())
        .map(|(t, p, material, no_shadow)| (*t, p.parent(), material.0.clone(), no_shadow))
        .collect::<Vec<_>>();
    assert_eq!(models.len(), 1);
    let (part, parent, material, no_shadow) = models.first().expect("single Wisp model");
    assert!(*no_shadow);
    let body = fixture
        .world()
        .resource::<Assets<StandardMaterial>>()
        .get(material)
        .expect("cached Wisp material");
    assert!(body.unlit && body.cull_mode.is_none());
    assert_eq!(body.alpha_mode, AlphaMode::Blend);
    assert!(part.scale.abs_diff_eq(Vec3::new(1.0, height, 1.0), 0.0001));
    assert!(part.translation.abs_diff_eq(Vec3::Y * height * 0.5, 0.0001));
    let root = fixture
        .world()
        .get::<Transform>(*parent)
        .expect("root transform");
    assert!(root.translation.abs_diff_eq(feet, 0.0001));
    assert!(root.rotation.abs_diff_eq(Quat::IDENTITY, 0.0001));
    assert_eq!(
        *fixture
            .world()
            .get::<Visibility>(*parent)
            .expect("visibility"),
        Visibility::Visible
    );
}

#[test]
fn natural_wisp_windup_and_released_ember_drive_capture_and_frozen_visual_size() {
    let mut fixture = observer();
    fixture
        .world_mut()
        .resource_mut::<ArenaSession>()
        .bot_enabled = true;
    let mut windup = false;
    let mut ember = false;
    for _ in 0..600 {
        tick(&mut fixture);
        let session = fixture.world().resource::<ArenaSession>();
        windup |= wisp::phase_actor(session, "encounter-wisp-windup").is_some();
        ember |= wisp::phase_actor(session, "encounter-wisp-ember").is_some();
        if windup && ember {
            break;
        }
    }
    assert!(
        windup && ember,
        "ordinary Wisp/Goblin fixture must exercise natural windup and in-flight Ember"
    );
    let shots = fixture
        .world()
        .resource::<ArenaSession>()
        .projectiles
        .iter()
        .filter(|p| p.appearance() == ProjectileAppearance::Ember)
        .map(|p| (p.position, p.collision_radius()))
        .collect::<Vec<_>>();
    assert!(!shots.is_empty());
    fixture
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    fixture
        .world_mut()
        .run_system_once(wisp::setup)
        .expect("Wisp assets");
    fixture
        .world_mut()
        .run_system_once(worm::setup)
        .expect("Worm cached visuals");
    fixture
        .world_mut()
        .run_system_once(presentation::setup_effects)
        .expect("ordinary assets");
    fixture
        .world_mut()
        .run_system_once(presentation::solid_effects)
        .expect("frozen appearance renderer");
    let visuals = fixture
        .world_mut()
        .query_filtered::<(&Transform, Has<NotShadowCaster>), With<presentation::TransientEffect>>()
        .iter(fixture.world())
        .map(|(t, no_shadow)| (*t, no_shadow))
        .collect::<Vec<_>>();
    for (position, radius) in shots {
        assert!(visuals.iter().any(|(t, no_shadow)| *no_shadow
            && t.translation.abs_diff_eq(position, 0.0001)
            && t.scale.abs_diff_eq(Vec3::splat(radius), 0.0001)));
        assert!(visuals.iter().any(|(t, no_shadow)| *no_shadow
            && t.translation.abs_diff_eq(position, 0.0001)
            && t.scale.abs_diff_eq(Vec3::splat(radius * 1.8), 0.0001)));
    }
}

#[test]
fn dim_comparison_changes_only_explicit_capture_lighting() {
    for (capture, view, dim) in [
        (false, "encounter-wisp-dark", false),
        (true, "encounter-wisp-body", false),
        (true, "encounter-wisp-dark", true),
    ] {
        let mut app = App::new();
        app.insert_resource(ViewState {
            capture: capture.then(|| PathBuf::from("unused-test.png")),
            capture_view: view.into(),
            ..default()
        })
        .insert_resource(GlobalAmbientLight {
            brightness: 420.0,
            ..default()
        })
        .insert_resource(ClearColor(Color::srgb(0.1, 0.16, 0.22)));
        let sun = app
            .world_mut()
            .spawn(DirectionalLight {
                illuminance: 18_000.0,
                ..default()
            })
            .id();
        app.world_mut()
            .run_system_once(wisp::configure_capture_lighting)
            .expect("presentation-only lighting");
        let expected = if dim { 6.0 } else { 420.0 };
        assert!((app.world().resource::<GlobalAmbientLight>().brightness - expected).abs() < 0.001);
        let expected = if dim { 0.0 } else { 18_000.0 };
        assert!(
            (app.world()
                .get::<DirectionalLight>(sun)
                .expect("sun")
                .illuminance
                - expected)
                .abs()
                < 0.001
        );
    }
}

#[test]
fn wisp_stress_rejects_live_or_nonexact_setups_and_keeps_nominal_tuning() {
    let mut setup = ArenaBattleSetup::spectator(BattlePreset::Wisps12, BattlePreset::Wisps12, 1);
    setup.tick_limit = Some(wisp::STRESS_TICKS);
    for map in [ArenaMap::Duel, ArenaMap::Fort] {
        assert!(wisp::validate_stress_setup(true, "observer-wisp-stress", map, &setup).is_ok());
        assert!(wisp::validate_stress_setup(false, "observer-wisp-stress", map, &setup).is_err());
    }
    assert!(wisp::validate_stress_setup(
        true,
        "observer-wisp-stress",
        ArenaMap::SevenRegions,
        &setup
    )
    .is_err());
    let mut wrong = setup.clone();
    wrong.seed = 2;
    assert!(
        wisp::validate_stress_setup(true, "observer-wisp-stress", ArenaMap::Duel, &wrong).is_err()
    );
    wrong = setup.clone();
    wrong.tick_limit = Some(3600);
    assert!(
        wisp::validate_stress_setup(true, "observer-wisp-stress", ArenaMap::Duel, &wrong).is_err()
    );
    wrong = ArenaBattleSetup::spectator(BattlePreset::Wisps8, BattlePreset::Wisps12, 1);
    wrong.tick_limit = Some(wisp::STRESS_TICKS);
    assert!(
        wisp::validate_stress_setup(true, "observer-wisp-stress", ArenaMap::Duel, &wrong).is_err()
    );
    for nominal in [18.0, 30.0] {
        for (capture, view, expected) in [
            (false, "observer-wisp-stress", None),
            (true, "observer-performance", None),
            (true, "observer-wisp-stress", Some(nominal)),
        ] {
            let mut tuning = ArenaTuning::default();
            tuning.encounters.wisp_hp = nominal;
            assert_eq!(
                wisp::configure_stress_tuning(capture, view, &mut tuning),
                expected
            );
            assert!(tuning.validate().is_ok());
            let hp = if expected.is_some() {
                wisp::STRESS_HP
            } else {
                nominal
            };
            assert!((tuning.encounters.wisp_hp - hp).abs() < f32::EPSILON);
        }
    }
}

#[test]
fn wisp_stress_records_exact_living_ticks_then_separate_frozen_publication() {
    let (mut fixture, _) = menu_app();
    let mut setup = ArenaBattleSetup::spectator(BattlePreset::Wisps12, BattlePreset::Wisps12, 1);
    setup.tick_limit = Some(wisp::STRESS_TICKS);
    fixture.insert_resource(setup);
    let nominal = {
        let mut tuning = fixture.world_mut().resource_mut::<ArenaTuning>();
        wisp::configure_stress_tuning(true, "observer-wisp-stress", &mut tuning)
    };
    {
        let mut state = fixture.world_mut().resource_mut::<ViewState>();
        state.prepare_round();
        state.capture = Some(PathBuf::from("unused-stress-test.png"));
        state.capture_view = "observer-wisp-stress".into();
        state.capture_wisp_nominal_hp = nominal;
        state.begin_play();
    }
    fixture.world_mut().resource_mut::<ArenaReset>().generation += 1;
    for _ in 0..750 {
        fixture.update();
        if fixture
            .world()
            .resource::<ViewState>()
            .capture_event_frame
            .is_some()
        {
            break;
        }
    }
    let before = {
        let session = fixture.world().resource::<ArenaSession>();
        let summary = session
            .battle_summary()
            .expect("accepted spectator workload");
        assert_eq!(summary.result, Some(hex_arena::BattleResult::Timeout));
        assert_eq!(summary.ticks, wisp::STRESS_TICKS);
        assert_eq!(session.actors.len(), 24);
        assert!(session.human_actor_id().is_none());
        assert!(summary.teams.iter().all(|team| team.initial == 12
            && team.living == 12
            && (team.max_hp - 12_000.0).abs() < 0.01));
        session
            .actors
            .iter()
            .map(|a| (a.id, a.hp))
            .collect::<Vec<_>>()
    };
    {
        let state = fixture.world().resource::<ViewState>();
        assert_eq!(state.capture_wisp_ticks.len(), 1440);
        assert!(state
            .capture_wisp_ticks
            .iter()
            .enumerate()
            .all(|(index, row)| row.tick == index as u64 + 1));
        let flush = state
            .capture_wisp_terminal
            .as_ref()
            .expect("separate terminal flush");
        assert_eq!(flush.tick, wisp::STRESS_TICKS);
        assert!(flush.cpu_ms.is_finite() && flush.cpu_ms >= 0.0);
        // The final terminal publication is separate from the 1440 living ticks.
        assert_eq!(
            state.tick_times.last().expect("flush timing").0,
            wisp::STRESS_TICKS
        );
    }
    for _ in 0..4 {
        fixture.update();
    }
    let session = fixture.world().resource::<ArenaSession>();
    assert_eq!(session.tick, wisp::STRESS_TICKS);
    assert_eq!(
        before,
        session
            .actors
            .iter()
            .map(|a| (a.id, a.hp))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        fixture
            .world()
            .resource::<ViewState>()
            .capture_wisp_ticks
            .len(),
        1440
    );
}

#[test]
fn wisp_windup_ring_is_local_depth_tested_and_absent_when_idle_or_dead() {
    fn redraw(
        mut commands: Commands,
        session: Res<ArenaSession>,
        assets: Res<wisp::WispVisualAssets>,
        old: Query<Entity, With<wisp::WispWindup>>,
    ) {
        for entity in &old {
            commands.entity(entity).despawn();
        }
        for actor in &session.actors {
            wisp::windup(&mut commands, actor, &assets);
        }
    }
    let mut fixture = observer();
    fixture
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    fixture
        .world_mut()
        .run_system_once(wisp::setup)
        .expect("cached Wisp assets");
    fixture
        .world_mut()
        .run_system_once(redraw)
        .expect("idle draw");
    assert_eq!(
        fixture
            .world_mut()
            .query_filtered::<Entity, With<wisp::WispWindup>>()
            .iter(fixture.world())
            .count(),
        0
    );
    fixture
        .world_mut()
        .resource_mut::<ArenaSession>()
        .bot_enabled = true;
    let mut reached = false;
    for _ in 0..600 {
        tick(&mut fixture);
        if wisp::phase_actor(
            fixture.world().resource::<ArenaSession>(),
            "encounter-wisp-windup",
        )
        .is_some()
        {
            reached = true;
            break;
        }
    }
    assert!(reached, "ordinary Wisp reaches a natural windup");
    let actor = fixture
        .world()
        .resource::<ArenaSession>()
        .actors
        .first()
        .expect("Wisp actor zero");
    let origin = actor.eye();
    let expected_radius = 0.35 + actor.attack_state().expect("windup").progress * 0.5;
    let initial_assets = fixture.world().resource::<Assets<Mesh>>().len();
    fixture
        .world_mut()
        .run_system_once(redraw)
        .expect("windup draw");
    let segments = fixture
        .world_mut()
        .query_filtered::<(
            &Transform,
            &MeshMaterial3d<StandardMaterial>,
            Has<NotShadowCaster>,
        ), With<wisp::WispWindup>>()
        .iter(fixture.world())
        .map(|(t, m, no_shadow)| (*t, m.0.clone(), no_shadow))
        .collect::<Vec<_>>();
    assert_eq!(segments.len(), 6);
    for (transform, material, no_shadow) in segments {
        assert!(no_shadow);
        assert!((transform.translation.y - origin.y - 0.16).abs() < 0.001);
        assert!((transform.scale.x - expected_radius).abs() < 0.001);
        assert!(transform.translation.distance(origin) < 1.0);
        let material = fixture
            .world()
            .resource::<Assets<StandardMaterial>>()
            .get(&material)
            .expect("warning material");
        assert!(material.unlit);
        assert_eq!(material.alpha_mode, AlphaMode::Opaque);
        assert_eq!(material.depth_bias.to_bits(), 0.0_f32.to_bits());
    }
    // Render-only lifetime check: a dead source must not keep its old warning.
    fixture
        .world_mut()
        .resource_mut::<ArenaSession>()
        .actors
        .first_mut()
        .expect("Wisp")
        .hp = 0.0;
    fixture
        .world_mut()
        .run_system_once(redraw)
        .expect("dead source draw");
    assert_eq!(
        fixture
            .world_mut()
            .query_filtered::<Entity, With<wisp::WispWindup>>()
            .iter(fixture.world())
            .count(),
        0
    );
    assert_eq!(
        fixture.world().resource::<Assets<Mesh>>().len(),
        initial_assets
    );
}
