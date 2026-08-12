//! Complete public-world round-trip and ordered-delta contracts.

use super::*;
use hex_core::SimulationRole;
use hex_map::{
    diff_world_snapshots_v1, export_world_snapshot_v1, fingerprint_world_snapshot_v1,
    validate_world_snapshot_v1_against_content, CurrentWorldSnapshotV1, WorldReplicationOutcomeV1,
    WorldReplicationRefusalV1, WorldReplicationRequestV1, WorldReplicationResultV1,
    WorldSnapshotError,
};
use hex_multiplayer::{
    AuthorityBoundary, AuthoritySequence, BoundedText, BoundedVec, InteriorSurfaceSnapshotV1,
    WorldAnchorSnapshotV1, WorldDamageSnapshotV1, WorldSnapshotV1, MAX_WORLD_DELTA_OPERATIONS,
    MAX_WORLD_PROJECTION_ENTRIES,
};

type TileTuple = (TilePos, RunBottom, u32, u32, SubstanceId, Headroom);

fn tile_tuples(app: &mut App) -> Vec<TileTuple> {
    let world = app.world_mut();
    let mut query = world
        .query_filtered::<(&TilePos, &RunBottom, &HexSpan, &SubstanceId, &Headroom), With<HexTile>>(
        );
    let mut tuples = query
        .iter(world)
        .map(|(position, bottom, span, substance, headroom)| {
            (
                *position,
                *bottom,
                span.bottom.to_bits(),
                span.top.to_bits(),
                *substance,
                *headroom,
            )
        })
        .collect::<Vec<_>>();
    tuples.sort_by_key(|tuple| tuple.0);
    tuples
}

fn drain_replication_results(app: &mut App) -> Vec<WorldReplicationResultV1> {
    app.world_mut()
        .resource_mut::<Messages<WorldReplicationResultV1>>()
        .drain()
        .collect()
}

#[expect(
    clippy::expect_used,
    reason = "the malformed-snapshot fixtures must still have a valid canonical fingerprint"
)]
fn refresh_fingerprint(snapshot: &mut WorldSnapshotV1) {
    snapshot.public_fingerprint = fingerprint_world_snapshot_v1(snapshot)
        .expect("structurally valid content fixture should fingerprint");
}

#[expect(
    clippy::expect_used,
    reason = "malicious content fixtures must remain valid bounded wire collections"
)]
fn replace_first_run_substance(snapshot: &mut WorldSnapshotV1, name: &str) {
    let mut columns = snapshot.columns.clone().into_vec();
    let column = columns
        .first_mut()
        .expect("snapshot fixture should contain one column");
    let mut runs = column.runs.clone().into_vec();
    runs.first_mut()
        .expect("snapshot fixture should contain one material run")
        .substance = BoundedText::new(name).expect("fixture substance identity should fit");
    column.runs = BoundedVec::new(runs).expect("fixture runs should remain bounded");
    snapshot.columns = BoundedVec::new(columns).expect("fixture columns should remain bounded");
}

#[expect(
    clippy::expect_used,
    reason = "the malicious object fixture must remain a valid bounded wire collection"
)]
fn replace_first_object_identity(snapshot: &mut WorldSnapshotV1, identity: &str) {
    let mut objects = snapshot.objects.clone().into_vec();
    objects
        .first_mut()
        .expect("snapshot fixture should contain one presentation object")
        .asset_identity =
        BoundedText::new(identity).expect("fixture object identity should remain bounded");
    snapshot.objects = BoundedVec::new(objects).expect("fixture objects should remain bounded");
}

#[expect(
    clippy::panic,
    reason = "the shared test helper reports the exact shipped fixture that violated setup or export"
)]
fn assert_round_trip(name: &str, mut app: App) {
    enter_gameplay(&mut app);
    if let Some(failure) = app.world().get_resource::<GameplaySetupFailure>() {
        panic!("{name} setup failed before export: {}", failure.reason);
    }
    let before = export_world_snapshot_v1(app.world())
        .unwrap_or_else(|error| panic!("{name} should export: {error}"));
    assert_eq!(
        app.world().resource::<CurrentWorldSnapshotV1>().snapshot(),
        &before,
        "{name} current cache drifted from live map truth"
    );
    let tile_tuples_before = tile_tuples(&mut app);

    app.world_mut()
        .write_message(WorldReplicationRequestV1::Restore {
            baseline_sequence: AuthoritySequence(11),
            snapshot: Box::new(before.clone()),
        });
    app.update();

    assert!(app.world().contains_resource::<TerrainReady>(), "{name}");
    let results = drain_replication_results(&mut app);
    assert_eq!(
        results,
        vec![WorldReplicationResultV1 {
            authority_sequence: AuthoritySequence(11),
            outcome: WorldReplicationOutcomeV1::Applied {
                public_fingerprint: before.public_fingerprint,
            },
        }],
        "{name} import did not publish one typed acceptance"
    );
    let after = export_world_snapshot_v1(app.world())
        .unwrap_or_else(|error| panic!("{name} should re-export: {error}"));
    assert_eq!(after, before, "{name} complete public world drifted");
    assert_eq!(
        tile_tuples(&mut app),
        tile_tuples_before,
        "{name} TilePos/RunBottom/HexSpan/SubstanceId/Headroom drifted"
    );
}

#[test]
fn perlin_v1_v2_v3_and_stacked_cave_worlds_round_trip_exactly() {
    let fixtures: [(&str, fn() -> App); 10] = [
        ("perlin", test_app),
        ("procedural-v1", procedural_app),
        ("procedural-v2", v2_hills_app),
        ("procedural-v3", v3_hills_app),
        ("v3-waterfall", v3_waterfall_app),
        ("v3-volcano", v3_volcano_app),
        ("v3-forest", v3_forest_app),
        ("v3-fort", v3_fort_app),
        ("stacked-caves", v3_caves_app),
        ("v3-crystal-ascent", v3_crystal_ascent_app),
    ];
    for (name, fixture) in fixtures {
        assert_round_trip(name, fixture());
    }
}

#[test]
fn shipped_projection_bound_is_derived_from_crystal_ascent_measurement() {
    let mut app = v3_crystal_ascent_app();
    enter_gameplay(&mut app);
    let snapshot = export_world_snapshot_v1(app.world())
        .unwrap_or_else(|error| panic!("Crystal Ascent should export: {error}"));
    let measured = snapshot.interior_roofs.len();

    assert_eq!(measured, 135_739, "shipped world measurement drifted");
    assert_eq!(
        MAX_WORLD_PROJECTION_ENTRIES,
        measured.saturating_mul(2).next_power_of_two()
    );
    assert_eq!(MAX_WORLD_DELTA_OPERATIONS, MAX_WORLD_PROJECTION_ENTRIES);
}

#[test]
fn liquid_metadata_requires_complete_run_level_topology() {
    let mut app = v3_waterfall_app();
    enter_gameplay(&mut app);
    let mut snapshot = export_world_snapshot_v1(app.world())
        .unwrap_or_else(|error| panic!("Waterfall should export: {error}"));
    let source = snapshot
        .liquids
        .iter()
        .find(|entry| entry.downstream.is_some())
        .cloned()
        .unwrap_or_else(|| panic!("Waterfall should contain a downstream liquid node"));
    let source_run = snapshot
        .columns
        .iter()
        .find(|column| column.coord == source.position.coord)
        .and_then(|column| {
            column.runs.iter().find(|run| {
                run.run_bottom <= source.position.level
                    && source.position.level <= run.position.level
            })
        })
        .cloned()
        .unwrap_or_else(|| panic!("liquid source should belong to one material run"));
    let missing_target = source
        .position
        .coord
        .neighbors()
        .into_iter()
        .map(|coord| TilePos::new(coord, 0))
        .find(|target| {
            !snapshot
                .liquids
                .iter()
                .any(|entry| entry.position == *target)
        })
        .unwrap_or_else(|| panic!("fixture should have an adjacent non-node target"));
    let mut liquids = snapshot.liquids.clone().into_vec();
    for entry in &mut liquids {
        if entry.position.coord == source_run.position.coord
            && source_run.run_bottom <= entry.position.level
            && entry.position.level <= source_run.position.level
        {
            entry.downstream = Some(missing_target);
        }
    }
    snapshot.liquids = BoundedVec::new(liquids)
        .unwrap_or_else(|error| panic!("modified liquid fixture should remain bounded: {error}"));
    snapshot.public_fingerprint = fingerprint_world_snapshot_v1(&snapshot)
        .unwrap_or_else(|error| panic!("modified fixture should remain structural: {error}"));

    let error = validate_world_snapshot_v1_against_content(
        &snapshot,
        app.world().resource::<SubstanceTable>(),
        app.world().resource::<MapSettings>(),
        app.world().get_resource::<RuntimeArtCatalog>(),
    )
    .expect_err("a missing downstream run must fail map-owned topology validation");
    assert!(matches!(
        error,
        WorldSnapshotError::PresentationMismatch(reason)
            if reason.contains("missing downstream node")
    ));
}

#[test]
fn content_validation_rejects_unknown_material_air_and_unknown_object() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    let baseline = export_world_snapshot_v1(app.world()).expect("baseline should export");

    let mut unknown_material = baseline.clone();
    replace_first_run_substance(&mut unknown_material, "unknown/shipped-material");
    refresh_fingerprint(&mut unknown_material);
    assert!(matches!(
        validate_world_snapshot_v1_against_content(
            &unknown_material,
            app.world().resource::<SubstanceTable>(),
            app.world().resource::<MapSettings>(),
            app.world().get_resource::<RuntimeArtCatalog>(),
        ),
        Err(WorldSnapshotError::UnknownSubstance(name))
            if name == "unknown/shipped-material"
    ));

    let mut air_material = baseline;
    replace_first_run_substance(&mut air_material, "air");
    refresh_fingerprint(&mut air_material);
    assert!(matches!(
        validate_world_snapshot_v1_against_content(
            &air_material,
            app.world().resource::<SubstanceTable>(),
            app.world().resource::<MapSettings>(),
            app.world().get_resource::<RuntimeArtCatalog>(),
        ),
        Err(WorldSnapshotError::AirAsMaterial(name)) if name == "air"
    ));

    let mut object_app = v3_crystal_ascent_app();
    enter_gameplay(&mut object_app);
    let mut unknown_object =
        export_world_snapshot_v1(object_app.world()).expect("Crystal Ascent should export");
    replace_first_object_identity(&mut unknown_object, "prop/not-shipped");
    refresh_fingerprint(&mut unknown_object);
    assert!(matches!(
        validate_world_snapshot_v1_against_content(
            &unknown_object,
            object_app.world().resource::<SubstanceTable>(),
            object_app.world().resource::<MapSettings>(),
            object_app.world().get_resource::<RuntimeArtCatalog>(),
        ),
        Err(WorldSnapshotError::UnknownObject(name)) if name == "prop/not-shipped"
    ));
}

#[test]
fn malformed_restore_variants_refuse_without_touching_ready_world() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    let baseline = export_world_snapshot_v1(app.world()).expect("baseline should export");
    let surface = baseline
        .columns
        .first()
        .and_then(|column| column.runs.first())
        .expect("baseline should contain one surface")
        .position;

    let mut duplicate = baseline.clone();
    let duplicate_anchor = WorldAnchorSnapshotV1 {
        name: BoundedText::new("duplicate").expect("fixture identity should fit"),
        position: surface,
    };
    duplicate.anchors = BoundedVec::new(vec![duplicate_anchor.clone(), duplicate_anchor])
        .expect("duplicate fixture should remain within the collection bound");

    let mut impossible_health = baseline.clone();
    impossible_health.damage = BoundedVec::new(vec![WorldDamageSnapshotV1 {
        position: surface,
        remaining: 2,
        maximum: 2,
    }])
    .expect("damage fixture should remain within the collection bound");

    let mut dangling_region = baseline.clone();
    dangling_region.interior_surfaces = BoundedVec::new(vec![InteriorSurfaceSnapshotV1 {
        position: TilePos::new(HexCoord::from_axial(100, 0), 0),
        region: 9,
    }])
    .expect("region fixture should remain within the collection bound");

    let mut wrong_version = baseline.clone();
    wrong_version.version = wrong_version.version.saturating_add(1);

    for (index, malformed) in [duplicate, impossible_health, dangling_region, wrong_version]
        .into_iter()
        .enumerate()
    {
        let sequence = AuthoritySequence(50_u64.saturating_add(index as u64));
        app.world_mut()
            .write_message(WorldReplicationRequestV1::Restore {
                baseline_sequence: sequence,
                snapshot: Box::new(malformed),
            });
        app.update();

        assert!(app.world().contains_resource::<TerrainReady>());
        assert!(matches!(
            drain_replication_results(&mut app).as_slice(),
            [WorldReplicationResultV1 {
                authority_sequence,
                outcome: WorldReplicationOutcomeV1::Refused(
                    WorldReplicationRefusalV1::InvalidSnapshot(_)
                ),
            }] if *authority_sequence == sequence
        ));
        assert_eq!(
            export_world_snapshot_v1(app.world()).expect("ready world should remain exportable"),
            baseline
        );
    }
}

#[test]
fn restore_waits_for_a_quiescent_authority_boundary() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    let baseline = export_world_snapshot_v1(app.world()).expect("baseline should export");
    app.world_mut()
        .resource_mut::<AuthorityBoundary>()
        .begin_movement();
    app.world_mut()
        .write_message(WorldReplicationRequestV1::Restore {
            baseline_sequence: AuthoritySequence(60),
            snapshot: Box::new(baseline.clone()),
        });
    app.update();

    assert!(matches!(
        drain_replication_results(&mut app).as_slice(),
        [WorldReplicationResultV1 {
            authority_sequence: AuthoritySequence(60),
            outcome: WorldReplicationOutcomeV1::Refused(WorldReplicationRefusalV1::BoundaryBusy),
        }]
    ));
    assert_eq!(
        export_world_snapshot_v1(app.world()).expect("busy world should remain ready"),
        baseline
    );
    app.world_mut()
        .resource_mut::<AuthorityBoundary>()
        .finish_movement()
        .expect("test movement boundary should balance");
}

#[test]
fn mutated_and_partially_damaged_world_restores_exact_state() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    let (coord, run) = diggable_run(&app, 3).expect("fixture should have deep diggable terrain");
    let cleared = TilePos::new(coord, run.top - 1);
    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: cleared });
    app.update();
    app.update();

    let damaged = TilePos::new(coord, run.top - 2);
    let substance = app.world().resource::<VoxelMap>().get(damaged);
    let maximum = app
        .world()
        .resource::<SubstanceTable>()
        .toughness(substance)
        .expect("selected terrain should have toughness");
    let remaining = maximum.saturating_sub(1).max(1);
    let health = TerrainVoxelHealth::new(remaining, maximum).expect("partial health fixture");
    app.world_mut()
        .resource_mut::<DamagedVoxels>()
        .publish(damaged, health);

    let snapshot = export_world_snapshot_v1(app.world()).expect("mutated world should export");
    assert!(snapshot
        .damage
        .iter()
        .any(|entry| entry.position == damaged));
    app.world_mut()
        .write_message(WorldReplicationRequestV1::Restore {
            baseline_sequence: AuthoritySequence(21),
            snapshot: Box::new(snapshot.clone()),
        });
    app.update();

    assert_eq!(
        export_world_snapshot_v1(app.world()).expect("restored world should export"),
        snapshot
    );
    assert_eq!(
        app.world().resource::<DamagedVoxels>().get(damaged),
        Some(health)
    );
    assert!(app.world().resource::<VoxelMap>().get(cleared).is_air());
}

#[test]
fn replica_generates_for_verification_but_cannot_mutate_world() {
    let mut app = test_app();
    app.insert_resource(SimulationRole::Replica);
    enter_gameplay(&mut app);
    let baseline = export_world_snapshot_v1(app.world()).expect("replica should generate locally");
    assert_eq!(
        app.world()
            .resource::<CurrentWorldSnapshotV1>()
            .fingerprint(),
        baseline.public_fingerprint
    );
    let (coord, run) = diggable_run(&app, 2).expect("fixture should have diggable terrain");
    let target = TilePos::new(coord, run.top - 1);
    let substance = app.world().resource::<VoxelMap>().get(target);

    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: target });
    app.update();
    app.update();

    assert_eq!(app.world().resource::<VoxelMap>().get(target), substance);
    assert_eq!(
        export_world_snapshot_v1(app.world()).expect("replica world should remain exportable"),
        baseline
    );
}

#[test]
fn ordered_delta_reaches_target_and_same_sequence_is_idempotent() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    let base = export_world_snapshot_v1(app.world()).expect("base should export");
    let (coord, run) = diggable_run(&app, 2).expect("fixture should have diggable terrain");
    app.world_mut().write_message(TerrainEdit::Clear {
        pos: TilePos::new(coord, run.top - 1),
    });
    app.update();
    app.update();
    let target = export_world_snapshot_v1(app.world()).expect("target should export");
    let delta = diff_world_snapshots_v1(&base, &target, AuthoritySequence(32))
        .expect("world snapshots should diff");

    app.world_mut()
        .write_message(WorldReplicationRequestV1::Restore {
            baseline_sequence: AuthoritySequence(31),
            snapshot: Box::new(base),
        });
    app.update();
    let _restore = drain_replication_results(&mut app);
    app.world_mut()
        .write_message(WorldReplicationRequestV1::ApplyDelta(delta.clone()));
    app.update();
    assert_eq!(
        export_world_snapshot_v1(app.world()).expect("delta target should export"),
        target
    );
    assert!(matches!(
        drain_replication_results(&mut app).as_slice(),
        [WorldReplicationResultV1 {
            outcome: WorldReplicationOutcomeV1::Applied { .. },
            ..
        }]
    ));

    let grid = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("delta should publish one grid");
    app.world_mut()
        .write_message(WorldReplicationRequestV1::ApplyDelta(delta));
    app.update();
    let duplicate_grid = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("duplicate should retain one grid");
    assert_eq!(duplicate_grid, grid, "duplicate delta rebuilt the world");
    assert!(matches!(
        drain_replication_results(&mut app).as_slice(),
        [WorldReplicationResultV1 {
            outcome: WorldReplicationOutcomeV1::Duplicate { .. },
            ..
        }]
    ));
}

#[test]
fn mixed_restore_batch_reports_results_in_request_order() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    let base = export_world_snapshot_v1(app.world()).expect("base should export");
    let (coord, run) = diggable_run(&app, 2).expect("fixture should have diggable terrain");
    app.world_mut().write_message(TerrainEdit::Clear {
        pos: TilePos::new(coord, run.top - 1),
    });
    app.update();
    app.update();
    let target = export_world_snapshot_v1(app.world()).expect("target should export");
    let delta = diff_world_snapshots_v1(&base, &target, AuthoritySequence(72))
        .expect("world snapshots should diff");

    app.world_mut()
        .write_message(WorldReplicationRequestV1::Restore {
            baseline_sequence: AuthoritySequence(71),
            snapshot: Box::new(base),
        });
    app.update();
    let _restore = drain_replication_results(&mut app);

    let mut malformed = target.clone();
    malformed.public_fingerprint.0 ^= 1;
    app.world_mut()
        .write_message(WorldReplicationRequestV1::ApplyDelta(delta));
    app.world_mut()
        .write_message(WorldReplicationRequestV1::Restore {
            baseline_sequence: AuthoritySequence(73),
            snapshot: Box::new(malformed),
        });
    app.update();

    let results = drain_replication_results(&mut app);
    assert!(matches!(
        results.as_slice(),
        [
            WorldReplicationResultV1 {
                authority_sequence: AuthoritySequence(72),
                outcome: WorldReplicationOutcomeV1::Applied { .. },
            },
            WorldReplicationResultV1 {
                authority_sequence: AuthoritySequence(73),
                outcome: WorldReplicationOutcomeV1::Refused(
                    WorldReplicationRefusalV1::InvalidSnapshot(_)
                ),
            },
        ]
    ));
    assert_eq!(
        export_world_snapshot_v1(app.world()).expect("accepted target should remain ready"),
        target
    );
}

#[test]
fn wrong_fingerprint_refuses_without_touching_the_ready_world() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    let before = export_world_snapshot_v1(app.world()).expect("world should export");
    let tuples = tile_tuples(&mut app);
    let mut malformed = before.clone();
    malformed.public_fingerprint.0 ^= 1;
    app.world_mut()
        .write_message(WorldReplicationRequestV1::Restore {
            baseline_sequence: AuthoritySequence(41),
            snapshot: Box::new(malformed),
        });
    app.update();

    assert!(app.world().contains_resource::<TerrainReady>());
    assert_eq!(
        export_world_snapshot_v1(app.world()).expect("prior world should remain"),
        before
    );
    assert_eq!(tile_tuples(&mut app), tuples);
    assert!(matches!(
        drain_replication_results(&mut app).as_slice(),
        [WorldReplicationResultV1 {
            outcome: WorldReplicationOutcomeV1::Refused(
                WorldReplicationRefusalV1::InvalidSnapshot(_)
            ),
            ..
        }]
    ));
}
