//! Headless behavior contracts for game-layer composition and Combat Lab surfaces.
//!
//! This is deliberately one integration binary: linking Bevy once is expensive,
//! while these assertions share the same immutable game-layer observation API.

use bevy::{prelude::*, window::WindowPlugin};
use hex_anim::Transformation;
use hex_assets::{
    ArtPalette, CameraSettings, CombatRulesProfile, CombatSettings, GameAssets, SubstanceFile,
    SubstanceTable,
};
use hex_combat::{
    CombatSummary, DeliveredEffectKind, EncounterOutcome, EncounterResolution, UnitCombatSummary,
};
use hex_core::{
    CameraFocusTarget, GameplayPhase, Headroom, HexCoord, HexSpan, HexTile, PresentationOcclusion,
    PresentationOcclusionReason, RunBottom, SubstanceId, TerrainEdit, TilePos, Turn, UnitId,
};
use hex_game::combat_reports::{
    CombatLabReport, CombatLabReportController, CombatLabReportDeployment, CombatLabReportId,
    CombatLabReportMap, CombatLabReportOrigin, CombatLabReportRosterEntry, CombatLabReportRosters,
    CombatLabReportTermination, SavedCombatLabReport,
};
use hex_game::test_support::{
    fixture_filter_snapshot, gameplay_state_snapshot, live_statistics_snapshot,
    live_statistics_visible, production_lab_ui_transition_snapshot, report_transition_snapshot,
    sandbox_reentry_snapshot, tempo_movement_matrix, wave_seven_fixtures, ReportMode,
};
use hex_map::{MapSettings, PerlinSettings, TerrainSettings};
use hex_test_support::{enter_gameplay, TestAppBuilder};
use hex_units::{HexPathingLine, Standing, StandsOn};
use hex_world::{CameraMode, PanOrbitCamera};

fn position(q: i32, r: i32, level: i32) -> TilePos {
    TilePos::new(HexCoord::from_axial(q, r), level)
}

fn sample_report(rounds: u32, player_id: u64, hostile_id: u64) -> CombatLabReport {
    let shipped = CombatSettings::default();
    let mut summary = CombatSummary::default();
    summary.rounds = rounds;
    summary.turns = rounds.saturating_mul(3);
    summary.successful_commands = 11;
    summary.refused_commands = 2;
    summary.movement_distance = 7;
    summary.movement_budget_used = 7;
    summary.channels = 1;
    summary.raw_disables = 4;
    summary.prevented_disables = 1;
    summary.applied_disables = 3;
    summary.no_progress_max = 2;
    summary.outcome = Some(EncounterOutcome::Victory);
    summary.casts_by_spell.insert("Spark".to_owned(), 1);
    summary
        .delivered_effects
        .insert(DeliveredEffectKind::Disable, 3);
    summary.channelled_mana.insert("Ember".to_owned(), 2);
    summary.units.insert(
        UnitId(player_id),
        UnitCombatSummary {
            turns: rounds,
            movement_distance: 7,
            channels: 1,
            ..Default::default()
        },
    );
    let result = CombatLabReport::new(
        CombatRulesProfile::shipped(&shipped),
        CombatLabReportOrigin::Sandbox,
        CombatLabReportMap {
            catalog_id: "flat-arena".to_owned(),
            scenario: "Ability Lab".to_owned(),
            resolved_seed: None,
        },
        77,
        CombatLabReportRosters {
            players: vec![CombatLabReportRosterEntry {
                unit_id: player_id,
                archetype: "hedge-mage".to_owned(),
                display_name: "Hedge Mage".to_owned(),
                controller: CombatLabReportController::Human,
            }],
            hostiles: vec![CombatLabReportRosterEntry {
                unit_id: hostile_id,
                archetype: "raider".to_owned(),
                display_name: "Raider".to_owned(),
                controller: CombatLabReportController::BaselineAi,
            }],
        },
        CombatLabReportDeployment {
            players: vec![position(-2, 1, 1)],
            hostiles: vec![position(3, -1, 2)],
        },
        CombatLabReportTermination::Outcome(EncounterOutcome::Victory),
        summary,
    );
    match result {
        Ok(report) => report,
        Err(_) => std::process::abort(),
    }
}

#[expect(
    clippy::expect_used,
    reason = "invalid tracked content should fail the composition fixture immediately"
)]
fn camera_terrain_substances() -> SubstanceTable {
    let palette: ArtPalette = ron::from_str(include_str!("../../../assets/art/palette.ron"))
        .expect("the tracked art palette should parse");
    let substances: SubstanceFile =
        ron::from_str(include_str!("../../../assets/config/substances.ron"))
            .expect("the tracked substance catalog should parse");
    SubstanceTable::from_file(&substances, &palette)
        .expect("the tracked substances should resolve through the tracked palette")
}

#[expect(
    clippy::expect_used,
    reason = "invalid tracked camera settings should fail the composition fixture immediately"
)]
fn camera_terrain_settings() -> CameraSettings {
    let settings: CameraSettings = ron::from_str(include_str!("../../../assets/config/camera.ron"))
        .expect("the tracked camera settings should parse");
    settings
        .validate()
        .expect("the radius-only camera fixture should remain valid");
    settings
}

fn camera_terrain_app() -> App {
    let mut builder = TestAppBuilder::new();
    builder
        .app_mut()
        .add_plugins(WindowPlugin {
            primary_window: None,
            ..default()
        })
        .add_plugins(bevy::transform::TransformPlugin)
        .insert_resource(GameAssets {
            hex_tile: Handle::default(),
            player_pieces: [Handle::default(), Handle::default()],
        })
        .insert_resource(camera_terrain_substances())
        .insert_resource(MapSettings {
            grid_radius: 6,
            level_height: 0.4,
            terrain: TerrainSettings::Perlin(PerlinSettings {
                seed: Some(20_260_801),
                // An empty octave list is the generator's deterministic flat-ground
                // case. It still travels through real map generation and publication.
                steps: Vec::new(),
            }),
        })
        .insert_resource(camera_terrain_settings())
        .add_plugins((
            hex_anim::plugin,
            hex_map::grid::plugin,
            hex_world::test_support::headless_camera_plugin,
        ));
    builder.build()
}

#[expect(
    clippy::expect_used,
    reason = "a missing real map surface is the contract failure this assertion helper reports"
)]
fn exposed_surface(app: &mut App, coord: HexCoord) -> (TilePos, HexSpan) {
    let world = app.world_mut();
    let mut tiles = world.query_filtered::<(&TilePos, &HexSpan, &Headroom), With<HexTile>>();
    tiles
        .iter(world)
        .filter(|(position, _, headroom)| position.coord == coord && headroom.0 > 0)
        .map(|(position, span, _)| (*position, *span))
        .max_by_key(|(position, _)| *position)
        .expect("flat generated terrain should publish one exposed run per coordinate")
}

fn published_run(
    app: &mut App,
    coord: HexCoord,
    bottom: i32,
    top: i32,
    substance: SubstanceId,
) -> Option<HexSpan> {
    let world = app.world_mut();
    let mut tiles =
        world.query_filtered::<(&TilePos, &RunBottom, &HexSpan, &SubstanceId), With<HexTile>>();
    tiles
        .iter(world)
        .find(|(position, run_bottom, _, material)| {
            position.coord == coord
                && position.level == top
                && run_bottom.0 == bottom
                && **material == substance
        })
        .map(|(_, _, span, _)| *span)
}

#[expect(
    clippy::expect_used,
    reason = "invalid production camera cardinality should fail the composition test immediately"
)]
fn camera_pose(app: &mut App) -> (Transform, Vec3, f32) {
    let world = app.world_mut();
    let mut cameras = world.query::<(&Transform, &PanOrbitCamera)>();
    let (transform, camera) = cameras
        .single(world)
        .expect("the production camera plugin should spawn one orbit camera");
    (*transform, camera.focus, camera.radius)
}

fn actual_camera_radius(transform: &Transform, focus: Vec3) -> f32 {
    transform.translation.distance(focus)
}

#[test]
fn terrain_edits_retract_and_restore_the_character_camera_in_projection_order() {
    let mut app = camera_terrain_app();
    enter_gameplay(&mut app);

    let origin = HexCoord::ORIGIN;
    let (support, support_span) = exposed_surface(&mut app, origin);
    app.world_mut().spawn((
        Transform::from_translation(origin.to_world(support_span.top)),
        CameraFocusTarget::new(support),
    ));

    let settings = app.world().resource::<CameraSettings>().clone();
    let focus = origin.to_world(support_span.top) + Vec3::Y * settings.character_focus_height;
    let pitch = settings.character_pitch * std::f32::consts::FRAC_PI_2;
    let direction = Vec3::new(0.0, pitch.sin(), pitch.cos());
    {
        let world = app.world_mut();
        let mut cameras = world.query::<(&mut Transform, &mut PanOrbitCamera)>();
        let (mut transform, mut camera) = cameras
            .single_mut(world)
            .expect("the production camera plugin should spawn one orbit camera");
        *transform = Transform::from_translation(focus + direction * settings.character_radius)
            .looking_at(focus, Vec3::Y);
        camera.focus = focus;
        camera.radius = settings.character_radius;
    }
    *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Character;
    app.update();

    assert_eq!(*app.world().resource::<CameraMode>(), CameraMode::Character);
    let (unobstructed_transform, focus, desired_radius) = camera_pose(&mut app);
    let unobstructed_radius = actual_camera_radius(&unobstructed_transform, focus);
    assert!(
        (unobstructed_radius - desired_radius).abs() < 1e-4,
        "flat public terrain should leave the desired Character radius clear"
    );

    // This coordinate is centred on the camera's preserved +z heading. The wall is
    // high enough to obstruct the exact player-authored boom, and it begins
    // directly above the real map-published surface rather than replacing a fixture.
    let wall_coord = HexCoord::from_axial(-1, 2);
    let (wall_support, _) = exposed_surface(&mut app, wall_coord);
    let wall_bottom = wall_support.level.saturating_add(1);
    let wall_top = wall_bottom.saturating_add(38);
    let stone = app
        .world()
        .resource::<SubstanceTable>()
        .id("stone")
        .expect("the tracked substance table should contain stone");
    for level in wall_bottom..=wall_top {
        app.world_mut().write_message(TerrainEdit::Set {
            pos: TilePos::new(wall_coord, level),
            substance: stone,
        });
    }

    // `hex_map` applies and republishes in Update. The camera refreshes that public
    // projection and follows in PostUpdate, so one frame must already be retracted.
    app.update();

    let wall_span = published_run(&mut app, wall_coord, wall_bottom, wall_top, stone)
        .expect("the real map edit should publish one exact contiguous stone run");
    assert!(wall_span.bottom <= support_span.top);
    assert!(wall_span.top > unobstructed_transform.translation.y);
    let (blocked_transform, blocked_focus, retained_radius) = camera_pose(&mut app);
    let blocked_radius = actual_camera_radius(&blocked_transform, blocked_focus);
    assert!(
        blocked_radius < desired_radius - 1e-4,
        "the newly published wall should retract the camera in the edit frame"
    );
    assert!((retained_radius - desired_radius).abs() < f32::EPSILON);
    assert!(
        blocked_transform
            .rotation
            .dot(unobstructed_transform.rotation)
            .abs()
            > 0.9999,
        "terrain collision must preserve the player-authored yaw and pitch"
    );

    for level in wall_bottom..=wall_top {
        app.world_mut().write_message(TerrainEdit::Clear {
            pos: TilePos::new(wall_coord, level),
        });
    }
    app.update();

    assert!(published_run(&mut app, wall_coord, wall_bottom, wall_top, stone).is_none());
    let (held_transform, held_focus, held_desired_radius) = camera_pose(&mut app);
    let held_radius = actual_camera_radius(&held_transform, held_focus);
    assert!(
        (held_radius - blocked_radius).abs() < 1e-4,
        "the first clear frame must respect the collision release delay"
    );
    assert!((held_desired_radius - desired_radius).abs() < f32::EPSILON);
    assert!(
        held_transform
            .rotation
            .dot(unobstructed_transform.rotation)
            .abs()
            > 0.9999
    );

    app.update();
    let (restoring_transform, restoring_focus, restoring_desired_radius) = camera_pose(&mut app);
    let restoring_radius = actual_camera_radius(&restoring_transform, restoring_focus);
    assert!(
        restoring_radius > held_radius + 1e-4,
        "restoration should begin only after continuous clearance reaches the delay"
    );
    assert!(restoring_radius - held_radius <= settings.character_restoration_speed * 0.1 + 1e-4);
    assert!(restoring_radius <= desired_radius);
    assert!((restoring_desired_radius - desired_radius).abs() < f32::EPSILON);
    assert!(
        restoring_transform
            .rotation
            .dot(unobstructed_transform.rotation)
            .abs()
            > 0.9999
    );

    let mut restored = false;
    let mut previous_radius = restoring_radius;
    for _ in 0..32 {
        app.update();
        let (transform, current_focus, current_desired_radius) = camera_pose(&mut app);
        let current_radius = actual_camera_radius(&transform, current_focus);
        assert!(current_radius + 1e-4 >= previous_radius);
        assert!(
            current_radius - previous_radius <= settings.character_restoration_speed * 0.1 + 1e-4
        );
        assert!(current_radius <= desired_radius + 1e-4);
        assert!(
            transform
                .rotation
                .dot(unobstructed_transform.rotation)
                .abs()
                > 0.9999
        );
        if (current_radius - desired_radius).abs() < 1e-4 {
            assert!((current_desired_radius - desired_radius).abs() < f32::EPSILON);
            restored = true;
            break;
        }
        previous_radius = current_radius;
    }
    assert!(
        restored,
        "the cleared camera should restore within 32 frames"
    );

    let settled = camera_pose(&mut app);
    app.update();
    let idle = camera_pose(&mut app);
    assert!(settled.0.translation.distance(idle.0.translation) < f32::EPSILON);
    assert!(settled.0.rotation.dot(idle.0.rotation).abs() > 0.9999);
    assert!((settled.1 - idle.1).length_squared() < f32::EPSILON);
    assert!((settled.2 - idle.2).abs() < f32::EPSILON);
}

#[test]
fn production_animation_walk_preserves_the_ordinary_character_camera_for_120_frames() {
    let mut app = camera_terrain_app();
    enter_gameplay(&mut app);

    let (start_pos, start_span) = exposed_surface(&mut app, HexCoord::ORIGIN);
    let (destination_pos, destination_span) = exposed_surface(&mut app, HexCoord::from_axial(1, 0));
    let start = Standing {
        pos: start_pos,
        span: start_span,
    };
    let destination = Standing {
        pos: destination_pos,
        span: destination_span,
    };
    let animation: Transformation = HexPathingLine::new(&[start, destination], 0.1).into();
    let target = app
        .world_mut()
        .spawn((
            Transform::from_translation(start.world_position()),
            CameraFocusTarget::new(start.pos),
            StandsOn(start),
            animation,
        ))
        .id();

    let settings = app.world().resource::<CameraSettings>().clone();
    let authored_rotation =
        Quat::from_rotation_x(-settings.character_pitch * std::f32::consts::FRAC_PI_2);
    let initial_focus = start.world_position() + Vec3::Y * settings.character_focus_height;
    {
        let world = app.world_mut();
        let mut cameras = world.query::<(&mut Transform, &mut PanOrbitCamera)>();
        let (mut transform, mut camera) = cameras
            .single_mut(world)
            .expect("the production camera plugin should spawn one orbit camera");
        transform.rotation = authored_rotation;
        transform.translation =
            initial_focus + authored_rotation * Vec3::Z * settings.character_radius;
        camera.focus = initial_focus;
        camera.radius = settings.character_radius;
    }
    *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Character;

    // The animation driver's first frame establishes time zero. Every subsequent
    // update advances the real path animation in Update, then the production camera
    // follows that final Transform in PostUpdate.
    app.update();
    let mut previous_target = app
        .world()
        .entity(target)
        .get::<Transform>()
        .expect("the moving focus target should retain its transform")
        .translation;
    let mut previous_camera = camera_pose(&mut app).0.translation;

    for frame in 0..120 {
        app.update();

        let target_translation = app
            .world()
            .entity(target)
            .get::<Transform>()
            .expect("the moving focus target should retain its transform")
            .translation;
        let target_delta = target_translation - previous_target;
        assert!(
            target_delta.length() > 1e-6,
            "production animation stopped before sampled frame {frame}"
        );

        let (transform, focus, desired_radius) = camera_pose(&mut app);
        let expected_focus = target_translation + Vec3::Y * settings.character_focus_height;
        let actual_radius = actual_camera_radius(&transform, focus);
        assert!(
            focus.distance(expected_focus) < 1e-5,
            "frame {frame} followed a stale target position"
        );
        assert!(
            transform.rotation.dot(authored_rotation).abs() > 0.999999,
            "frame {frame} changed the player-authored rotation"
        );
        assert!(
            (desired_radius - settings.character_radius).abs() < f32::EPSILON,
            "frame {frame} changed the player-authored zoom"
        );
        assert!(
            (actual_radius - desired_radius).abs() < 1e-5,
            "frame {frame} retracted the camera on ordinary published terrain"
        );
        assert!(
            transform
                .translation
                .distance(expected_focus + authored_rotation * Vec3::Z * desired_radius)
                < 1e-5,
            "frame {frame} changed the ordinary player-authored composition"
        );
        assert!(
            ((transform.translation - previous_camera) - target_delta).length() < 1e-5,
            "frame {frame} introduced camera motion beyond the same-frame target delta"
        );

        previous_target = target_translation;
        previous_camera = transform.translation;
    }
}

#[test]
fn support_limited_upward_look_hides_and_restores_the_real_focus_target() {
    let mut app = camera_terrain_app();
    enter_gameplay(&mut app);

    let origin = HexCoord::ORIGIN;
    let (support, support_span) = exposed_surface(&mut app, origin);
    let target = app
        .world_mut()
        .spawn((
            Transform::from_translation(origin.to_world(support_span.top)),
            Visibility::Inherited,
            PresentationOcclusion::default(),
            CameraFocusTarget::new(support),
        ))
        .id();
    let settings = app.world().resource::<CameraSettings>().clone();
    let focus = origin.to_world(support_span.top) + Vec3::Y * settings.character_focus_height;
    let straight_up = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
    {
        let world = app.world_mut();
        let mut cameras = world.query::<(&mut Transform, &mut PanOrbitCamera)>();
        let (mut transform, mut camera) = cameras
            .single_mut(world)
            .expect("the production plugin should own one camera");
        transform.rotation = straight_up;
        transform.translation = focus + straight_up * Vec3::Z * settings.character_radius;
        camera.focus = focus;
        camera.radius = settings.character_radius;
    }
    *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Character;

    app.update();

    let (retracted, retracted_focus, desired_radius) = camera_pose(&mut app);
    assert!(actual_camera_radius(&retracted, retracted_focus) <= 1e-4);
    assert!((desired_radius - settings.character_radius).abs() < f32::EPSILON);
    assert!(retracted.rotation.dot(straight_up).abs() > 0.9999);
    let hidden = app.world().entity(target);
    assert!(hidden
        .get::<PresentationOcclusion>()
        .expect("the unit should retain composable visibility ownership")
        .contains(PresentationOcclusionReason::CharacterCameraProximity));
    assert_eq!(hidden.get::<Visibility>(), Some(&Visibility::Hidden));

    let ordinary_look =
        Quat::from_rotation_x(-settings.character_pitch * std::f32::consts::FRAC_PI_2);
    {
        let world = app.world_mut();
        let mut cameras = world.query_filtered::<&mut Transform, With<PanOrbitCamera>>();
        cameras
            .single_mut(world)
            .expect("the orbit camera should remain queryable")
            .rotation = ordinary_look;
    }
    for _ in 0..4 {
        app.update();
    }

    let restored = app.world().entity(target);
    assert_eq!(restored.get::<Visibility>(), Some(&Visibility::Inherited));
    assert!(!restored
        .get::<PresentationOcclusion>()
        .expect("the unit should retain its reason set after recovery")
        .contains(PresentationOcclusionReason::CharacterCameraProximity));
    let (restored_camera, restored_focus, _) = camera_pose(&mut app);
    assert!(actual_camera_radius(&restored_camera, restored_focus) > 1.2);
    assert!(restored_camera.rotation.dot(ordinary_look).abs() > 0.9999);
}

#[test]
fn fixture_search_filters_in_place_and_clear_restores_every_card() {
    let snapshot = fixture_filter_snapshot("tempo");
    assert_eq!(snapshot.visible, vec!["tempo-matrix"]);
    assert_eq!(snapshot.visible_after_clear.len(), 7);
    assert!(
        snapshot.input_survived,
        "filtering must not rebuild and replace the focused editable entity"
    );
}

#[test]
fn report_modes_and_comparison_identity_follow_real_typed_application_transitions() {
    let current = sample_report(8, 0, 1);
    let older = sample_report(2, 2, 3);
    let selected = sample_report(6, 4, 5);
    let saved = [
        SavedCombatLabReport {
            id: CombatLabReportId(17),
            label: "older".to_owned(),
            notes: String::new(),
            report: older,
        },
        SavedCombatLabReport {
            id: CombatLabReportId(23),
            label: "selected".to_owned(),
            notes: String::new(),
            report: selected,
        },
    ];

    let transition = report_transition_snapshot(&current, &saved, CombatLabReportId(17))
        .expect("completed reports must drive the production report adapter");
    assert_eq!(
        transition
            .modes
            .iter()
            .map(|projection| projection.mode)
            .collect::<Vec<_>>(),
        vec![
            ReportMode::Overview,
            ReportMode::Units,
            ReportMode::SpellsEffects,
            ReportMode::Timeline,
            ReportMode::Compare,
        ],
        "each typed intent must settle through the model and game adapter"
    );
    let current_fingerprint = current
        .fingerprint()
        .expect("current fixture must have a canonical fingerprint");
    assert!(transition
        .modes
        .iter()
        .all(|projection| projection.current_fingerprint == current_fingerprint));
    assert!(transition
        .modes
        .iter()
        .all(|projection| projection.units == [UnitId(0), UnitId(1)]));
    let Some(comparison) = transition
        .modes
        .last()
        .and_then(|projection| projection.comparison)
    else {
        panic!("Compare must preserve the independently selected report");
    };
    assert_eq!(comparison.0, CombatLabReportId(17));
    assert_eq!(
        comparison.1,
        saved
            .first()
            .expect("comparison fixture exists")
            .report
            .fingerprint()
            .expect("comparison fixture must have a canonical fingerprint")
    );
    assert_eq!(transition.after_compare.mode, ReportMode::Overview);
    assert_eq!(
        transition.after_compare.comparison,
        Some(comparison),
        "leaving Compare must preserve the independently selected canonical report"
    );
    assert_eq!(
        transition.after_compare.current_fingerprint, current_fingerprint,
        "presentation transitions must not replace the retained current report"
    );
}

#[test]
fn live_drawer_uses_the_canonical_summary_and_has_a_bounded_lifecycle() {
    let report = sample_report(4, 0, 1);
    let snapshot = live_statistics_snapshot(&report.summary);
    assert_eq!(snapshot.rounds, 4);
    assert_eq!(snapshot.turns, 12);
    assert_eq!(snapshot.successful_commands, 11);
    assert_eq!(snapshot.refused_commands, 2);
    assert_eq!(snapshot.movement_distance, 7);
    assert_eq!(snapshot.movement_budget_used, 7);
    assert_eq!(snapshot.channels, 1);
    assert_eq!(snapshot.disables, (4, 1, 3));
    assert_eq!(snapshot.channelled_mana.get("Ember"), Some(&2));
    assert_eq!(snapshot.units, [UnitId(0)]);

    assert!(live_statistics_visible(
        GameplayPhase::Active,
        Some(EncounterResolution(None))
    ));
    assert!(!live_statistics_visible(
        GameplayPhase::Deployment,
        Some(EncounterResolution(None))
    ));
    assert!(!live_statistics_visible(
        GameplayPhase::Active,
        Some(EncounterResolution(Some(EncounterOutcome::Victory)))
    ));
}

#[test]
fn production_lab_toggle_keeps_statistics_below_the_lattice_at_retina_size() {
    let transition = production_lab_ui_transition_snapshot();
    assert_eq!(
        transition.published_own_lattice_cells, 7,
        "the production lattice publisher must project the canonical player lattice"
    );
    assert!(
        transition.expanded_state,
        "the typed production intent must toggle application state"
    );
    assert_eq!(
        transition.expanded.metrics.logical_size,
        bevy::prelude::Vec2::new(1291.0, 721.0),
        "the contract must use Bevy client pixels, excluding native title-bar chrome"
    );
    assert_eq!(
        transition.expanded.metrics.viewport,
        hex_ui::UiViewportClass::Compact
    );
    assert!(
        transition.collapsed.layout_issues().is_empty(),
        "the collapsed production adapter tree must satisfy the structural oracle: {:?}",
        transition.collapsed.layout_issues()
    );
    assert!(
        transition.expanded.layout_issues().is_empty(),
        "the expanded production adapter tree must satisfy the structural oracle: {:?}",
        transition.expanded.layout_issues()
    );

    fn node<'a>(
        snapshot: &'a hex_ui::test_support::UiTreeSnapshot,
        name: &str,
    ) -> &'a hex_ui::test_support::UiNodeObservation {
        snapshot
            .nodes
            .iter()
            .find(|node| node.name == name)
            .unwrap_or_else(|| panic!("production tree is missing {name:?}"))
    }
    let collapsed_lattice = node(&transition.collapsed, "Lattice Readout Stack");
    let expanded_lattice = node(&transition.expanded, "Lattice Readout Stack");
    let drawer = node(&transition.expanded, "Combat Lab Live Statistics Drawer");
    assert_eq!(
        expanded_lattice.parent_name.as_deref(),
        Some("Inspector HUD Region")
    );
    assert_eq!(drawer.parent_name.as_deref(), Some("Inspector HUD Region"));
    assert!(
        expanded_lattice.center.y + expanded_lattice.size.y * 0.5
            <= drawer.center.y - drawer.size.y * 0.5 + 0.5,
        "statistics must follow the persistent lattice in the same scroll flow: lattice={expanded_lattice:?}, drawer={drawer:?}"
    );
    assert!(
        (expanded_lattice.center - collapsed_lattice.center)
            .abs()
            .max_element()
            <= 0.5
            && (expanded_lattice.size - collapsed_lattice.size)
                .abs()
                .max_element()
                <= 0.5,
        "expanding through the production intent must not move or resize the lattice: collapsed={collapsed_lattice:?}, expanded={expanded_lattice:?}"
    );
}

#[test]
fn retry_copy_and_tune_restore_exact_frozen_launch_identity_once() {
    let report = sample_report(8, 0, 1);
    let expected_profile = report.profile.preset;
    let expected_players = report.deployment.players.clone();
    let expected_hostiles = report.deployment.hostiles.clone();

    let restored = sandbox_reentry_snapshot(report);
    assert_eq!(restored.map, "flat-arena");
    assert_eq!(restored.preset, expected_profile);
    assert_eq!(restored.players, ["hedge-mage"]);
    assert_eq!(restored.hostiles, ["raider"]);
    assert_eq!(restored.player_positions, expected_players);
    assert_eq!(restored.hostile_positions, expected_hostiles);
    assert!(
        restored.request_consumed,
        "re-entry handoffs must be one-shot so cold launch cannot inherit stale identity"
    );
}

#[test]
fn canonical_zero_unit_id_is_valid_while_duplicate_nonzero_ids_are_rejected() {
    let shipped = CombatSettings::default();
    let zero = sample_report(1, 0, 1);
    assert!(zero.validate(&shipped).is_ok(), "UnitId(0) is canonical");

    let mut duplicate = sample_report(1, 7, 8);
    let Some(hostile) = duplicate.rosters.hostiles.first_mut() else {
        panic!("sample report has one hostile");
    };
    hostile.unit_id = 7;
    assert!(duplicate
        .validate(&shipped)
        .is_err_and(|error| error.contains("unique")));
}

#[test]
fn wave_seven_fixtures_own_three_by_three_rosters_and_all_tempo_profiles() {
    let fixtures = wave_seven_fixtures();
    assert_eq!(
        fixtures
            .iter()
            .map(|fixture| fixture.id)
            .collect::<Vec<_>>(),
        ["occupancy-matrix", "channel-attrition", "tempo-matrix"]
    );
    assert!(fixtures
        .iter()
        .all(|fixture| fixture.players == 3 && fixture.hostiles == 3));
    assert_eq!(tempo_movement_matrix(&CombatSettings::default()), [4, 2, 3]);
}

#[test]
fn gameplay_snapshot_reads_exact_canonical_position_and_budget_without_rendering() {
    let mut world = World::new();
    world.insert_resource(GameplayPhase::Active);
    world.spawn((
        UnitId(0),
        StandsOn(Standing {
            pos: position(-3, 2, 4),
            span: HexSpan::new(3.0, 4.0),
        }),
        Turn {
            movement_left: 2,
            acted: true,
        },
    ));

    let snapshot = gameplay_state_snapshot(&mut world);
    assert_eq!(snapshot.phase, Some(GameplayPhase::Active));
    assert_eq!(snapshot.units.len(), 1);
    let Some(unit) = snapshot.units.first() else {
        panic!("the canonical unit must be observable");
    };
    assert_eq!(unit.id, UnitId(0));
    assert_eq!(unit.position, position(-3, 2, 4));
    assert_eq!(unit.turn.map(|turn| turn.movement_left), Some(2));
    assert_eq!(unit.turn.map(|turn| turn.acted), Some(true));
    assert!(snapshot.presented_actions.is_empty());
}

#[test]
fn gameplay_snapshot_names_hud_actions_as_a_presentation_projection() {
    let mut world = World::new();
    let presented = hex_ui::ActionAffordance {
        action: hex_ui::GameplayAction::EndTurn,
        label: "End turn".to_owned(),
        shortcut: Some("Space".to_owned()),
        availability: hex_ui::ActionAvailability::Enabled,
        priority: hex_ui::ActionPriority::Primary,
    };
    world.insert_resource(hex_ui::GameplayHudView {
        phase: GameplayPhase::Active,
        actor: Some(UnitId(0)),
        actor_label: "Hedge Mage".to_owned(),
        round: "Round 1".to_owned(),
        movement_remaining: 2,
        action_remaining: true,
        required_prompt: None,
        actions: vec![presented.clone()],
    });

    let snapshot = gameplay_state_snapshot(&mut world);
    assert_eq!(snapshot.presented_actions, vec![presented]);
}
