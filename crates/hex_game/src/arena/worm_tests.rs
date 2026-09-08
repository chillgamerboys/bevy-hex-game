//! Application projections only; authoritative burrowing and attacks have separate tests.
use super::*;
use bevy::ecs::system::RunSystemOnce;
use bevy::light::NotShadowCaster;
use hex_arena::{ArenaBattleSetup, BattlePreset, ProjectileAppearance, Species};

fn observer() -> App {
    let (mut app, _) = menu_app();
    app.insert_resource(ArenaBattleSetup::spectator(
        BattlePreset::Worm,
        BattlePreset::Goblins,
        1,
    ));
    app.world_mut().resource_mut::<ArenaReset>().generation += 1;
    app.update();
    assert!(app
        .world()
        .resource::<ArenaSession>()
        .actors
        .first()
        .is_some_and(|actor| actor.id == 0 && actor.species == Species::Worm));
    app.init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    app.world_mut()
        .run_system_once(worm::setup)
        .expect("cached Worm visuals");
    app
}

fn spawn_projection(
    mut commands: Commands,
    session: Res<ArenaSession>,
    assets: Res<worm::WormVisualAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some(actor) = session.actors.first() else {
        return;
    };
    let accent = materials.add(StandardMaterial::from(Color::srgb(0.12, 0.72, 0.84)));
    commands
        .spawn((
            Transform::from_translation(actor.feet).with_rotation(actor.body_rotation()),
            Visibility::Inherited,
        ))
        .with_children(|body| worm::spawn_body(body, actor, &assets, accent));
}

fn prism_rows(app: &mut App) -> Vec<(Entity, Transform, Handle<Mesh>, Entity, Visibility)> {
    let mut rows = app
        .world_mut()
        .query_filtered::<(
            &worm::WormPart,
            Entity,
            &Transform,
            &Mesh3d,
            &ChildOf,
            &Visibility,
        ), With<worm::WormPrism>>()
        .iter(app.world())
        .map(|(part, entity, transform, mesh, parent, visibility)| {
            (
                part.segment_index()
                    .expect("physical part has a stable segment index"),
                (
                    entity,
                    *transform,
                    mesh.0.clone(),
                    parent.parent(),
                    *visibility,
                ),
            )
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|(index, _)| *index);
    rows.into_iter().map(|(_, row)| row).collect()
}

#[test]
fn worm_current_head_first_offsets_update_existing_native_segment_meshes() {
    let mut app = observer();
    app.world_mut()
        .run_system_once(spawn_projection)
        .expect("body projection");
    let initial = prism_rows(&mut app);
    assert_eq!(initial.len(), 4);
    let initial_offsets = app
        .world()
        .resource::<ArenaSession>()
        .actors
        .first()
        .expect("Worm actor 0")
        .body_hex_prisms()
        .map(|p| p.offset)
        .collect::<Vec<_>>();
    assert!(initial.iter().all(|(_, _, mesh, _, _)| initial
        .first()
        .is_some_and(|(_, _, first, _, _)| mesh == first)));
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = true;
    let mut changed = false;
    for _ in 0..120 {
        tick(&mut app);
        let actor = app
            .world()
            .resource::<ArenaSession>()
            .actors
            .first()
            .expect("stable Worm identity");
        changed = actor
            .body_hex_prisms()
            .map(|p| p.offset)
            .collect::<Vec<_>>()
            != initial_offsets;
        if changed {
            break;
        }
    }
    assert!(
        changed,
        "ordinary physical emergence must produce a dynamic segment offset"
    );
    let mesh_count = app.world().resource::<Assets<Mesh>>().len();
    let material_count = app.world().resource::<Assets<StandardMaterial>>().len();
    app.world_mut()
        .run_system_once(worm::update_parts)
        .expect("update physical segment projection");
    let updated = prism_rows(&mut app);
    let actor = app
        .world()
        .resource::<ArenaSession>()
        .actors
        .first()
        .expect("Worm actor 0");
    let expected = actor.body_hex_prisms().collect::<Vec<_>>();
    assert_eq!(updated.len(), expected.len());
    assert_eq!(actor.body_rotation(), Quat::IDENTITY);
    assert_eq!(app.world().resource::<Assets<Mesh>>().len(), mesh_count);
    assert_eq!(
        app.world().resource::<Assets<StandardMaterial>>().len(),
        material_count
    );
    for (index, (entity, transform, mesh, parent, visibility)) in updated.iter().enumerate() {
        let part = expected
            .get(index)
            .expect("one current component for every stable index");
        let old = initial
            .get(index)
            .expect("one prior model for every stable index");
        assert_eq!((*entity, mesh, *parent), (old.0, &old.2, old.3));
        assert_eq!(
            *visibility,
            Visibility::Inherited,
            "body hiding comes from opaque earth, never phase"
        );
        assert!(transform
            .translation
            .abs_diff_eq(part.offset + Vec3::Y * (part.height * 0.5), 0.0001));
        assert!(transform
            .scale
            .abs_diff_eq(Vec3::new(1.0, part.height, 1.0), 0.0001));
    }
    let head = expected.first().expect("head-first geometry");
    assert!(actor.eye().abs_diff_eq(
        actor.feet + head.offset + Vec3::Y * (head.height * 0.5),
        0.0001
    ));
    // A redraw while simulation is paused changes neither identity nor current pose.
    app.world_mut()
        .run_system_once(worm::update_parts)
        .expect("paused projection");
    assert_eq!(updated, prism_rows(&mut app));
}

fn project_windup(
    mut commands: Commands,
    session: Res<ArenaSession>,
    assets: Res<worm::WormVisualAssets>,
) {
    for actor in session
        .actors
        .iter()
        .filter(|actor| actor.species == Species::Worm)
    {
        worm::windup(&mut commands, actor, &assets);
    }
}

fn project_boulders(
    mut commands: Commands,
    session: Res<ArenaSession>,
    assets: Res<worm::WormVisualAssets>,
) {
    for shot in session
        .projectiles
        .iter()
        .filter(|shot| shot.appearance() == ProjectileAppearance::Boulder)
    {
        worm::boulder(&mut commands, shot, &assets);
    }
}

#[test]
fn worm_admitted_windup_and_owner_independent_boulder_keep_physical_snapshot_poses() {
    let mut app = observer();
    app.world_mut()
        .run_system_once(spawn_projection)
        .expect("head projection");
    let idle = head_material(&mut app);
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = true;
    let mut windup_seen = false;
    let mut released = false;
    for _ in 0..1200 {
        tick(&mut app);
        if !windup_seen
            && worm::phase_actor(
                app.world().resource::<ArenaSession>(),
                "encounter-worm-windup",
            )
            .is_some()
        {
            app.world_mut()
                .run_system_once(worm::update_parts)
                .expect("windup head");
            let warning = head_material(&mut app);
            assert_ne!(
                warning, idle,
                "real Windup visibly changes the existing head material"
            );
            let material = app
                .world()
                .resource::<Assets<StandardMaterial>>()
                .get(&warning)
                .expect("cached warning");
            assert!(material.unlit);
            assert_eq!(material.alpha_mode, AlphaMode::Opaque);
            assert_eq!(material.depth_bias.to_bits(), 0.0_f32.to_bits());
            // Death during the actual windup must restore the ordinary material.
            let hp = app
                .world()
                .resource::<ArenaSession>()
                .actors
                .first()
                .expect("Worm")
                .hp;
            app.world_mut()
                .resource_mut::<ArenaSession>()
                .actors
                .first_mut()
                .expect("Worm")
                .hp = 0.0;
            app.world_mut()
                .run_system_once(worm::update_parts)
                .expect("dead head");
            assert_eq!(head_material(&mut app), idle);
            app.world_mut()
                .resource_mut::<ArenaSession>()
                .actors
                .first_mut()
                .expect("Worm")
                .hp = hp;
            app.world_mut()
                .run_system_once(project_windup)
                .expect("real admitted windup projection");
            let cues = app
                .world_mut()
                .query_filtered::<(
                    &Transform,
                    &MeshMaterial3d<StandardMaterial>,
                    Has<NotShadowCaster>,
                ), With<worm::WormWindup>>()
                .iter(app.world())
                .map(|(t, m, no_shadow)| (*t, m.0.clone(), no_shadow))
                .collect::<Vec<_>>();
            let actor = app
                .world()
                .resource::<ArenaSession>()
                .actors
                .first()
                .expect("Worm");
            let attack = actor.attack_state().expect("current Windup");
            let direction = attack.direction.normalize_or(Vec3::NEG_Z);
            let length = attack.range.clamp(0.0, 2.5);
            assert_eq!(cues.len(), 1);
            let (pose, material, no_shadow) = cues.first().expect("one short direction cue");
            assert!(*no_shadow);
            assert!(pose
                .translation
                .abs_diff_eq(attack.origin + direction * (length * 0.5), 0.0001));
            let material = app
                .world()
                .resource::<Assets<StandardMaterial>>()
                .get(material)
                .expect("warning material");
            assert_eq!(material.alpha_mode, AlphaMode::Opaque);
            windup_seen = true;
        }
        released = worm::phase_actor(
            app.world().resource::<ArenaSession>(),
            "encounter-worm-boulder",
        )
        .is_some();
        if released {
            break;
        }
    }
    assert!(
        windup_seen && released,
        "ordinary Worm/Goblins must exercise admitted windup and a released Boulder"
    );
    app.world_mut()
        .run_system_once(worm::update_parts)
        .expect("released head");
    assert_eq!(
        head_material(&mut app),
        idle,
        "release restores the ordinary head"
    );
    let shots = app
        .world()
        .resource::<ArenaSession>()
        .projectiles
        .iter()
        .filter(|shot| shot.appearance() == ProjectileAppearance::Boulder)
        .map(|shot| {
            (
                shot.position,
                shot.collision_radius(),
                shot.source_ability(),
            )
        })
        .collect::<Vec<_>>();
    assert!(!shots.is_empty());
    // Only source HP is changed in this rendering test; do not advance physics or modify the shot.
    if let Some(actor) = app
        .world_mut()
        .resource_mut::<ArenaSession>()
        .actors
        .first_mut()
    {
        actor.hp = 0.0;
    }
    app.world_mut()
        .run_system_once(project_boulders)
        .expect("frozen projectile projection after owner death");
    let visuals = app
        .world_mut()
        .query_filtered::<(
            &Transform,
            &MeshMaterial3d<StandardMaterial>,
            Has<NotShadowCaster>,
        ), With<worm::BoulderVisual>>()
        .iter(app.world())
        .map(|(t, m, no_shadow)| (*t, m.0.clone(), no_shadow))
        .collect::<Vec<_>>();
    assert_eq!(visuals.len(), shots.len());
    for (position, radius, source) in shots {
        assert_eq!(source, Some(hex_arena::CreatureAbility::WormBoulder));
        assert!(visuals.iter().any(|(pose, _, no_shadow)| *no_shadow
            && pose.translation.abs_diff_eq(position, 0.0001)
            && pose.scale.abs_diff_eq(Vec3::splat(radius), 0.0001)));
    }
    assert!(visuals.iter().all(|(_, handle, _)| app
        .world()
        .resource::<Assets<StandardMaterial>>()
        .get(handle)
        .is_some_and(|m| m.alpha_mode == AlphaMode::Opaque)));
}

fn head_material(app: &mut App) -> Handle<StandardMaterial> {
    app.world_mut()
        .query::<(&worm::WormPart, &MeshMaterial3d<StandardMaterial>)>()
        .iter(app.world())
        .find_map(|(part, material)| {
            (part.segment_index() == Some(0)).then_some(material.0.clone())
        })
        .expect("physical head material")
}

#[test]
fn conversion_capture_waits_for_a_correlated_exposed_top_without_rewriting_the_world() {
    let mut fixture = observer();
    fixture
        .init_resource::<worm_capture::Evidence>()
        .add_systems(
            Update,
            (worm_capture::observe, worm_capture::progress)
                .chain()
                .after(drive_simulation),
        );
    {
        let mut state = fixture.world_mut().resource_mut::<ViewState>();
        state.capture = Some(PathBuf::from("unused-test.png"));
        state.capture_view = "encounter-worm-converted-earth".into();
        state.begin_play();
    }
    let mut reached = false;
    for _ in 0..1800 {
        fixture.update();
        if fixture
            .world()
            .resource::<ViewState>()
            .capture_event_frame
            .is_some()
        {
            reached = true;
            break;
        }
    }
    assert!(
        reached,
        "ordinary Duel conversion must expose a real changed dirt top"
    );
    let world = fixture.world();
    let receipt = world.resource::<worm_capture::Evidence>().receipt(
        world.resource::<ArenaTerrainView>(),
        world.resource::<ArenaReset>(),
        world.resource::<hex_core::arena::ArenaMaterials>(),
        world.resource::<hex_core::DamagedVoxels>(),
    );
    let selected = receipt
        .pointer("/exposed_surface/position")
        .expect("selected top");
    let changed = receipt
        .pointer("/conversion/changed")
        .and_then(serde_json::Value::as_array)
        .expect("original correlated changes");
    assert!(changed
        .iter()
        .any(|change| change.get("position") == Some(selected)));
    assert_eq!(
        receipt.pointer("/exposed_surface/revision"),
        receipt.get("current_revision")
    );
    let position: hex_core::TilePos =
        serde_json::from_value(selected.clone()).expect("typed voxel");
    assert!(!world
        .resource::<ArenaTerrainView>()
        .voxels
        .keys()
        .any(|p| p.coord == position.coord && p.level > position.level));
    let geometry = *world.resource::<ArenaVoxelGeometry>();
    let surface = geometry.center(position).with_y(geometry.top(position));
    for actor in &world.resource::<ArenaSession>().actors {
        for part in actor.body_hex_prisms() {
            let bottom = actor.feet + part.offset;
            if (bottom.x - surface.x).abs() < 1.8 && (bottom.z - surface.z).abs() < 2.05 {
                // Actual child stripe: center0.525, height0.045 in the native
                // unit prism, then scaled by the physical segment height.
                let stripe_top = bottom.y + part.height * (0.5 + 0.525 + 0.045 * 0.5);
                assert!(
                    stripe_top < surface.y,
                    "capture must wait until the actual surface accent clears the converted top"
                );
            }
        }
    }
    assert!(
        receipt
            .pointer("/exposed_surface/frame")
            .and_then(serde_json::Value::as_u64)
            > receipt
                .pointer("/conversion/frame")
                .and_then(serde_json::Value::as_u64),
        "wait past the first conversion while the body still covers its surface"
    );
    let frozen = combat_snapshot(&fixture);
    fixture.update();
    assert_eq!(combat_snapshot(&fixture), frozen);
}

#[test]
fn worm_player_recipe_and_reset_return_to_ready_without_leaking_into_original_modes() {
    use hex_arena::ArenaControl;
    let selection = launch_selection(Some("fort"), Some("worm")).expect("Fort Worm override");
    let mut setup = ArenaBattleSetup::default();
    apply_player_recipe(&mut setup, selection, Some("worm")).expect("Worm override");
    assert_eq!(setup.player_recipe, Some(BattlePreset::Worm));
    for map in ["duel", "seven-regions"] {
        assert!(launch_selection(Some(map), Some("worm")).is_err());
    }
    let (mut fixture, _) = menu_app();
    press_action(&mut fixture, hud::Action::Map(ArenaMap::Fort));
    press_action(&mut fixture, hud::Action::PlayerRecipe(BattlePreset::Worm));
    assert!(fixture
        .world()
        .resource::<ArenaSession>()
        .actors
        .iter()
        .any(|actor| actor.species == Species::Worm));
    tap_key(&mut fixture, KeyCode::Enter);
    tap_key(&mut fixture, KeyCode::KeyR);
    assert!(!fixture.world().resource::<ViewState>().started);
    assert!(fixture.world().resource::<ViewState>().paused);
    assert_eq!(
        fixture
            .world()
            .resource::<ArenaSession>()
            .accepted_battle_setup()
            .player_recipe,
        Some(BattlePreset::Worm)
    );
    press_action(&mut fixture, hud::Action::Control(ArenaControl::Spectator));
    assert_eq!(
        fixture.world().resource::<ArenaSession>().human_actor_id(),
        None
    );
    press_action(&mut fixture, hud::Action::Control(ArenaControl::Player));
    press_action(&mut fixture, hud::Action::Map(ArenaMap::Duel));
    let session = fixture.world().resource::<ArenaSession>();
    assert_eq!(session.accepted_battle_setup().player_recipe, None);
    assert!(!session
        .actors
        .iter()
        .any(|actor| actor.species == Species::Worm));
}

#[test]
fn worm_capture_observes_real_conversion_then_uses_r_to_restore_a_frozen_ready_round() {
    let (mut fixture, _) = menu_app();
    fixture.insert_resource(ArenaBattleSetup::spectator(
        BattlePreset::Worm,
        BattlePreset::Goblins,
        1,
    ));
    fixture.world_mut().resource_mut::<ArenaReset>().generation += 1;
    fixture.update();
    fixture
        .init_resource::<worm_capture::Evidence>()
        .add_systems(PreUpdate, worm_capture::inject_reset_key.before(input))
        .add_systems(
            Update,
            (worm_capture::observe, worm_capture::progress)
                .chain()
                .after(drive_simulation),
        );
    {
        let mut state = fixture.world_mut().resource_mut::<ViewState>();
        state.capture = Some(PathBuf::from("unused-test.png"));
        state.capture_view = "encounter-worm-reset".into();
        state.begin_play();
    }
    let generation = fixture.world().resource::<ArenaReset>().generation;
    let mut reached = false;
    for _ in 0..1800 {
        fixture.update();
        let state = fixture.world().resource::<ViewState>();
        if state.capture_event_frame.is_some() && !state.started {
            reached = true;
            break;
        }
    }
    assert!(
        reached,
        "natural Worm conversion must reach the ordinary R-key reset receipt"
    );
    let receipt = {
        let world = fixture.world();
        world.resource::<worm_capture::Evidence>().receipt(
            world.resource::<ArenaTerrainView>(),
            world.resource::<ArenaReset>(),
            world.resource::<hex_core::arena::ArenaMaterials>(),
            world.resource::<hex_core::DamagedVoxels>(),
        )
    };
    assert_eq!(
        fixture.world().resource::<ArenaReset>().generation,
        generation + 1
    );
    assert_eq!(
        receipt
            .pointer("/reset_key/key")
            .and_then(serde_json::Value::as_str),
        Some("R")
    );
    let changed = receipt
        .pointer("/conversion/changed")
        .and_then(serde_json::Value::as_array)
        .expect("acknowledged actual changes");
    assert!(!changed.is_empty());
    assert!(changed.len() <= 64);
    let current = receipt
        .get("current_cells")
        .and_then(serde_json::Value::as_array)
        .expect("published reset materials");
    for (before, after) in changed.iter().zip(current) {
        assert_eq!(before.get("position"), after.get("position"));
        assert_eq!(before.get("before"), after.get("material"));
        assert!(after
            .get("published_health")
            .is_some_and(serde_json::Value::is_null));
    }
    let frozen = combat_snapshot(&fixture);
    for _ in 0..5 {
        fixture.update();
    }
    assert_eq!(
        combat_snapshot(&fixture),
        frozen,
        "capture completion keeps ready combat frozen"
    );
}

#[test]
fn natural_worm_travel_capture_requires_published_earth_at_its_actual_head() {
    let mut fixture = observer();
    fixture
        .world_mut()
        .resource_mut::<ArenaSession>()
        .bot_enabled = true;
    let mut reached = false;
    for _ in 0..1800 {
        tick(&mut fixture);
        let session = fixture.world().resource::<ArenaSession>();
        let terrain = fixture.world().resource::<ArenaTerrainView>();
        let geometry = *fixture.world().resource::<ArenaVoxelGeometry>();
        if worm::terrain_phase_ready(session, "encounter-worm-buried", terrain, geometry) {
            let actor = worm::phase_actor(session, "encounter-worm-buried")
                .expect("actual travelling Worm");
            assert!(!actor.worm().expect("physical state").exposed);
            assert_eq!(
                actor
                    .worm()
                    .expect("physical state")
                    .head_clearance
                    .to_bits(),
                0.0_f32.to_bits(),
            );
            let head_cell = geometry.voxel_at(actor.eye()).expect("physical head voxel");
            assert!(terrain
                .voxels
                .get(&head_cell)
                .is_some_and(|material| !material.is_air()));
            let mut missing_earth = terrain.clone();
            missing_earth.voxels.remove(&head_cell);
            assert!(
                !worm::terrain_phase_ready(
                    session,
                    "encounter-worm-buried",
                    &missing_earth,
                    geometry
                ),
                "Travel alone cannot authorize the buried capture"
            );
            reached = true;
            break;
        }
        if session.is_finished() {
            break;
        }
    }
    assert!(
        reached,
        "ordinary Worm/Goblins round reaches real buried travel before ending"
    );
}
