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
