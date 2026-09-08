//! Golem presentation consumes physical snapshots and ordinary menu/reset/input.
use super::*;
use bevy::ecs::system::RunSystemOnce;
use bevy::mesh::VertexAttributeValues;
use hex_arena::{ArenaBattleSetup, ArenaControl, BattlePreset, Species};

#[test]
fn golem_launch_is_an_explicit_fort_player_override_or_observer_roster() {
    let selection = launch_selection(Some("fort"), Some("golem")).expect("Fort recipe");
    assert_eq!(selection.encounter, ArenaEncounter::Dragon);
    let mut player = ArenaBattleSetup::default();
    apply_player_recipe(&mut player, selection, Some("golem")).expect("Golem enabled");
    assert_eq!(player.player_recipe, Some(BattlePreset::Golem));
    for map in ["duel", "seven-regions"] {
        assert!(launch_selection(Some(map), Some("golem")).is_err());
    }
    let mut observer = spectator::launch_setup(
        ArenaMap::Duel,
        true,
        Some("golem"),
        Some("shadow"),
        Some("7"),
        Some("720"),
    )
    .expect("Duel Golem spectator roster");
    assert_eq!(
        spectator::preset_for(&observer, 0),
        Some(BattlePreset::Golem)
    );
    assert!(observer.player_recipe.is_none());
    assert!(apply_player_recipe(&mut observer, selection, Some("golem")).is_err());
}

#[test]
fn golem_menu_restart_preserves_override_and_original_selection_clears_it() {
    let (mut fixture, _) = menu_app();
    press_action(&mut fixture, hud::Action::Map(ArenaMap::Fort));
    press_action(&mut fixture, hud::Action::Encounter(ArenaEncounter::Dragon));
    let before = fixture.world().resource::<ArenaReset>().generation;
    press_action(&mut fixture, hud::Action::PlayerRecipe(BattlePreset::Golem));
    assert_eq!(
        fixture.world().resource::<ArenaReset>().generation,
        before + 1
    );
    let accepted = fixture
        .world()
        .resource::<ArenaSession>()
        .accepted_battle_setup()
        .clone();
    assert_eq!(accepted.player_recipe, Some(BattlePreset::Golem));
    assert!(fixture
        .world()
        .resource::<ArenaSession>()
        .actors
        .iter()
        .any(|a| a.species == Species::Golem));
    press_action(&mut fixture, hud::Action::Start);
    // Choices cannot change a running encounter.
    press_action(&mut fixture, hud::Action::Encounter(ArenaEncounter::Dragon));
    assert_eq!(*fixture.world().resource::<ArenaBattleSetup>(), accepted);
    tap_key(&mut fixture, KeyCode::KeyR);
    assert_eq!(
        *fixture
            .world()
            .resource::<ArenaSession>()
            .accepted_battle_setup(),
        accepted
    );
    // The underlying world recipe is already Dragon, but clearing the override
    // still needs a reset rather than treating this as a no-op selection.
    let generation = fixture.world().resource::<ArenaReset>().generation;
    press_action(&mut fixture, hud::Action::Encounter(ArenaEncounter::Dragon));
    assert_eq!(
        fixture.world().resource::<ArenaReset>().generation,
        generation + 1
    );
    assert!(fixture
        .world()
        .resource::<ArenaBattleSetup>()
        .player_recipe
        .is_none());
    assert!(fixture
        .world()
        .resource::<ArenaSession>()
        .actors
        .iter()
        .any(|a| a.species == Species::Dragon));
    assert!(fixture
        .world()
        .resource::<ArenaSession>()
        .actors
        .iter()
        .all(|a| a.species != Species::Golem));
    for leave in [
        hud::Action::Map(ArenaMap::Duel),
        hud::Action::Control(ArenaControl::Spectator),
    ] {
        press_action(&mut fixture, hud::Action::Map(ArenaMap::Fort));
        press_action(&mut fixture, hud::Action::PlayerRecipe(BattlePreset::Golem));
        press_action(&mut fixture, leave);
        assert!(fixture
            .world()
            .resource::<ArenaBattleSetup>()
            .player_recipe
            .is_none());
    }
}

#[test]
fn golem_observer_actor_zero_renders_seven_native_columns_at_authoritative_pose() {
    let (mut fixture, _) = menu_app();
    fixture.insert_resource(ArenaBattleSetup::spectator(
        BattlePreset::Golem,
        BattlePreset::Shadow,
        1,
    ));
    fixture.world_mut().resource_mut::<ArenaReset>().generation += 1;
    fixture.update();
    assert!(fixture
        .world()
        .resource::<ArenaSession>()
        .human_actor_id()
        .is_none());
    {
        let mut session = fixture.world_mut().resource_mut::<ArenaSession>();
        let actor = session.actors.first_mut().expect("Golem actor zero");
        let body_before = actor.body_dimensions();
        // All six notches, their vertices, and intermediate yaw values. These
        // points independently describe each native hex's visible outer surface.
        for angle in 0..72u16 {
            let yaw = f32::from(angle) * std::f32::consts::TAU / 72.0;
            let direction = Vec3::new(yaw.sin(), 0.0, yaw.cos());
            actor.aim = direction;
            let mouth = actor.eye();
            let face = golem::decorative_mount(actor, mouth, direction);
            for prism in actor.body_hex_prisms() {
                for corner in 0..6u16 {
                    let theta = f32::from(corner) * std::f32::consts::TAU / 6.0;
                    let point =
                        actor.feet + prism.offset + Vec3::new(theta.sin(), 1.4, theta.cos());
                    // The back of a 0.06-deep eye surround must remain in front
                    // of every body surface when viewed along the face direction.
                    assert!((face - direction * 0.03 - point).dot(direction) > 0.039);
                }
            }
            assert_eq!(
                actor.eye(),
                mouth,
                "decoration cannot relocate the launch origin"
            );
            assert_eq!(actor.body_dimensions(), body_before);
            assert_eq!(actor.body_rotation(), Quat::IDENTITY);
        }
    }
    let (feet, prisms) = {
        let session = fixture.world().resource::<ArenaSession>();
        let actor = session
            .actors
            .first()
            .expect("ordinary observer actor zero");
        assert_eq!(actor.species, Species::Golem);
        (actor.feet, actor.body_hex_prisms().collect::<Vec<_>>())
    };
    fixture
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    fixture
        .world_mut()
        .run_system_once(golem::setup)
        .expect("cached visuals");
    fixture
        .world_mut()
        .run_system_once(wisp::setup)
        .expect("Wisp cached visuals");
    fixture
        .world_mut()
        .run_system_once(presentation::actors)
        .expect("models");
    let columns = fixture
        .world_mut()
        .query_filtered::<(&Transform, &Mesh3d, &ChildOf), With<golem::GolemPrism>>()
        .iter(fixture.world())
        .map(|(t, mesh, parent)| (*t, mesh.0.clone(), parent.parent()))
        .collect::<Vec<_>>();
    assert_eq!(columns.len(), 7);
    let meshes = fixture.world().resource::<Assets<Mesh>>();
    for (column, mesh, parent) in columns {
        assert!(prisms.iter().any(|p| {
            column
                .translation
                .abs_diff_eq(p.offset + Vec3::Y * p.height * 0.5, 0.0001)
                && column
                    .scale
                    .abs_diff_eq(Vec3::new(1.0, p.height, 1.0), 0.0001)
        }));
        let root = fixture
            .world()
            .get::<Transform>(parent)
            .expect("model root");
        assert!(root.translation.abs_diff_eq(feet, 0.0001));
        assert!(root.rotation.abs_diff_eq(Quat::IDENTITY, 0.0001));
        assert_eq!(
            *fixture
                .world()
                .get::<Visibility>(parent)
                .expect("root visibility"),
            Visibility::Visible
        );
        let mesh = meshes.get(&mesh).expect("native prism mesh");
        let vertices = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|positions| match positions {
                VertexAttributeValues::Float32x3(vertices) => Some(vertices),
                _ => None,
            })
            .expect("native float positions");
        let (low, high) = vertices.iter().fold(
            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(low, high), v| {
                let v = Vec3::from_array(*v);
                (low.min(v), high.max(v))
            },
        );
        assert!((high - low).abs_diff_eq(Vec3::new(3.0_f32.sqrt(), 1.0, 2.0), 0.0001));
    }
}
