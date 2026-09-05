use super::*;
use hex_core::SightBand;
use hex_world_contracts::{
    AnchorRole, ChunkDescriptor, ChunkSemantics, ColumnData, InteriorSpan, MaterialSpec,
    ObjectInstance, RegionDescriptor, ResidencyRequest, VoxelEdit, VoxelRun, WorldAnchor,
    WorldEditTransaction, WorldManifest, WorldPackage, SCHEMA_VERSION,
};
use hex_world_runtime::{MemoryChunkSource, RuntimeConfig, WorldRuntime};
use std::{
    thread,
    time::{Duration, Instant},
};

fn point(q: i64, r: i64) -> WorldHex {
    WorldHex::new(q, r)
}
fn voxel(column: WorldHex, level: i32) -> VoxelPosition {
    VoxelPosition { column, level }
}
fn run(bottom: i32, top: i32) -> VoxelRun {
    VoxelRun {
        bottom,
        top,
        material: "stone".into(),
    }
}
fn world(regions: &[(WorldHex, u32)], level: i32) -> WorldPackage {
    let mut chunks: BTreeMap<ChunkId, ChunkPackage> = BTreeMap::new();
    let mut descriptors = Vec::new();
    for (index, (origin, radius)) in regions.iter().enumerate() {
        descriptors.push(RegionDescriptor {
            id: format!("region-{index:04}"),
            origin: *origin,
            radius: *radius,
            source_fingerprint: 1,
        });
        let extent = i64::from(*radius);
        for q in -extent..=extent {
            for r in -extent..=extent {
                if q.abs().max(r.abs()).max((q + r).abs()) > extent {
                    continue;
                }
                let position = origin.checked_add(point(q, r)).expect("fixture coordinate");
                chunks
                    .entry(position.chunk())
                    .or_insert_with(|| ChunkPackage {
                        schema_version: SCHEMA_VERSION,
                        world_id: "perception-world".into(),
                        coordinate: position.chunk(),
                        source_fingerprint: 1,
                        columns: Vec::new(),
                        features: Vec::new(),
                        semantics: ChunkSemantics::default(),
                        fingerprint: 0,
                    })
                    .columns
                    .push(ColumnData {
                        position,
                        runs: vec![run(level - 3, level + 1)],
                    });
            }
        }
    }
    let catalogue = chunks
        .keys()
        .map(|coordinate| ChunkDescriptor {
            coordinate: *coordinate,
            fingerprint: 0,
            path: format!("chunks/{}_{}.ron", coordinate.q, coordinate.r),
        })
        .collect();
    let mut package = WorldPackage {
        manifest: WorldManifest {
            schema_version: SCHEMA_VERSION,
            world_id: "perception-world".into(),
            compiler_version: "perception-fixture".into(),
            source_fingerprint: 1,
            materials: vec![MaterialSpec {
                id: "stone".into(),
                solid: true,
                diggable: true,
                color: [120, 120, 120, 255],
            }],
            regions: descriptors,
            chunks: catalogue,
            boundaries: Vec::new(),
            summary: Vec::new(),
            features: Vec::new(),
            fingerprint: 0,
        },
        chunks,
    };
    package.seal().expect("world fixture");
    package
}
fn request(id: &str, at: WorldHex, level: i32, radius: u32) -> ObserverRequest {
    ObserverRequest {
        id: id.into(),
        principal: format!("party-{id}"),
        position: voxel(at, level),
        profile: SightProfile {
            bright: SightBand::new(radius),
            dim: SightBand::new(radius),
            dark: SightBand::new(radius),
        },
        exterior: ExteriorIllumination::new(IlluminationLevel::Bright),
    }
}
fn fixture(
    package: WorldPackage,
    requests: &[ObserverRequest],
    config: PerceptionConfig,
) -> (WorldRuntime, PerceptionWorld) {
    let index = Arc::new(ManifestIndex::new(Arc::new(package.manifest.clone())).expect("index"));
    let mut projection = PerceptionWorld::new(index, config).expect("projection");
    let mut runtime = WorldRuntime::new(
        Arc::new(MemoryChunkSource::new(package).expect("source")),
        RuntimeConfig::default(),
    )
    .expect("runtime");
    runtime
        .set_interests(
            requests
                .iter()
                .map(|request| ResidencyRequest {
                    id: request.id.clone(),
                    center: request.position.column,
                    radius: radius(request.profile) + 1,
                    retention_radius: radius(request.profile) + 1,
                    priority: 1,
                })
                .collect(),
        )
        .expect("interests");
    flush(&mut runtime, &mut projection);
    (runtime, projection)
}
fn flush(runtime: &mut WorldRuntime, projection: &mut PerceptionWorld) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let update = runtime.pump();
        assert!(
            update.failures.is_empty(),
            "source failures: {:?}",
            update.failures
        );
        for chunk in update.removed {
            projection.remove(chunk);
        }
        for chunk in update.loaded.into_iter().chain(update.changed) {
            projection
                .publish(chunk.package, chunk.revision)
                .expect("source publication");
        }
        if runtime.counts().in_flight_jobs == 0 && runtime.counts().queued_chunks == 0 {
            break;
        }
        assert!(Instant::now() < deadline, "bounded test load deadline");
        thread::sleep(Duration::from_millis(1));
    }
}
fn ready(result: ObservationResult) -> Arc<ObserverFacts> {
    match result {
        ObservationResult::Ready(facts) => facts,
        other => {
            assert!(
                matches!(other, ObservationResult::Ready(_)),
                "expected ready, got {other:?}"
            );
            unreachable!()
        }
    }
}
fn column_mut(package: &mut WorldPackage, at: WorldHex) -> &mut ColumnData {
    package
        .chunks
        .get_mut(&at.chunk())
        .expect("chunk")
        .columns
        .iter_mut()
        .find(|column| column.position == at)
        .expect("column")
}
fn add_object(package: &mut WorldPackage, root: WorldHex, column: WorldHex, bottom: i32, top: i32) {
    package
        .chunks
        .get_mut(&root.chunk())
        .expect("root")
        .semantics
        .objects
        .push(ObjectInstance {
            id: "pillar".into(),
            region_id: "region-0000".into(),
            asset: "pillar".into(),
            origin: voxel(root, 0),
            rotation: 0,
            occupancy: vec![ColumnData {
                position: column,
                runs: vec![run(bottom, top)],
            }],
        });
}

#[test]
fn resident_sight_matches_existing_exact_predicate_with_stacks_and_full_height_objects() {
    let origin = point(0, 0);
    let mut package = world(&[(origin, 6)], 0);
    column_mut(&mut package, point(1, 0)).runs = vec![run(-3, 2)];
    column_mut(&mut package, point(2, 0)).runs.push(run(3, 4));
    add_object(&mut package, point(0, 1), point(1, 1), 1, 3);
    package.seal().expect("mixed fixture");
    let observer = request("a", origin, 0, 4);
    let (runtime, mut projection) = fixture(
        package.clone(),
        std::slice::from_ref(&observer),
        PerceptionConfig::default(),
    );
    let facts = ready(
        projection
            .observe(&observer, &runtime)
            .expect("observation"),
    );
    let terrain = TerrainOccupancy::from_runs(
        package
            .chunks
            .values()
            .flat_map(|chunk| &chunk.columns)
            .flat_map(|column| {
                column.runs.iter().map(|run| {
                    (
                        TilePos::new(
                            HexCoord::from_axial(
                                i32::try_from(column.position.q).expect("small q"),
                                i32::try_from(column.position.r).expect("small r"),
                            ),
                            run.top - 1,
                        ),
                        RunBottom(run.bottom),
                    )
                })
            }),
    )
    .expect("legacy terrain");
    let objects = AuthoredObjectOccupancy::from_runs(
        package
            .chunks
            .values()
            .flat_map(|chunk| &chunk.semantics.occupancy)
            .flat_map(|column| {
                column.runs.iter().map(|run| AuthoredObjectVoxelRun {
                    top: TilePos::new(
                        HexCoord::from_axial(
                            i32::try_from(column.position.q).expect("q"),
                            i32::try_from(column.position.r).expect("r"),
                        ),
                        run.top - 1,
                    ),
                    bottom: run.bottom,
                })
            }),
    )
    .expect("legacy objects");
    let mut candidates = Vec::new();
    for column in package.chunks.values().flat_map(|chunk| &chunk.columns) {
        if origin.checked_distance(column.position).expect("distance") <= 4 {
            if let QueryResult::Ready(surfaces) = runtime.surfaces(column.position) {
                candidates.extend(surfaces);
            }
        }
    }
    let illumination = crate::ResolvedIllumination::try_resolve(
        candidates.iter().map(|surface| {
            (
                local_position(surface.position, observer.position).expect("local"),
                LightDomain::Exterior,
            )
        }),
        observer.exterior,
        &[],
    )
    .expect("legacy illumination");
    let expected = candidates
        .into_iter()
        .filter(|surface| {
            crate::can_observe_with_authored_objects(
                TilePos::new(HexCoord::ORIGIN, 0),
                local_position(surface.position, observer.position).expect("local"),
                &illumination,
                observer.profile,
                &terrain,
                &objects,
            )
        })
        .map(|surface| surface.position)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        facts
            .surfaces
            .iter()
            .map(|fact| fact.surface.position)
            .collect::<BTreeSet<_>>(),
        expected
    );
    assert!(
        facts
            .surfaces
            .iter()
            .any(|fact| fact.surface.position == voxel(point(2, 0), 3)),
        "upper stack identity survives"
    );
}

#[test]
fn translating_huge_negative_world_coordinates_and_absolute_levels_preserves_observation() {
    let mut baseline = None;
    for (origin, level) in [
        (point(0, 0), 0),
        (point(-1, -1), -100),
        (point(1_000_000_000_015, -1_000_000_000_017), i32::MAX - 8),
        (point(-1_000_000_000_015, 1_000_000_000_017), i32::MIN + 8),
    ] {
        let mut package = world(&[(origin, 5)], level);
        if level > 0 {
            for column in package
                .chunks
                .values_mut()
                .flat_map(|chunk| &mut chunk.columns)
            {
                column.runs.first_mut().expect("run").bottom = i32::MIN;
            }
            package.seal().expect("deep compact interval");
        }
        let observer = request("a", origin, level, 3);
        let (runtime, mut projection) = fixture(
            package,
            std::slice::from_ref(&observer),
            PerceptionConfig::default(),
        );
        let facts = ready(
            projection
                .observe(&observer, &runtime)
                .expect("translated perception"),
        );
        let local = facts
            .surfaces
            .iter()
            .map(|fact| {
                (
                    i128::from(fact.surface.position.column.q) - i128::from(origin.q),
                    i128::from(fact.surface.position.column.r) - i128::from(origin.r),
                    i64::from(fact.surface.position.level) - i64::from(level),
                )
            })
            .collect::<BTreeSet<_>>();
        if let Some(expected) = &baseline {
            assert_eq!(&local, expected);
        } else {
            baseline = Some(local);
        }
    }
}

#[test]
fn unloaded_or_stale_dependencies_never_become_air_or_partial_observations() {
    let at = point(15, 1);
    let observer = request("a", at, 0, 4);
    let package = world(&[(at, 6)], 0);
    let (mut runtime, mut projection) = fixture(
        package.clone(),
        std::slice::from_ref(&observer),
        PerceptionConfig::default(),
    );
    let facts = ready(
        projection
            .observe(&observer, &runtime)
            .expect("initial complete"),
    );
    let removed = facts.dependencies.first().expect("dependency").coordinate;
    projection.remove(removed);
    assert!(
        matches!(projection.observe(&observer,&runtime).expect("pending publication"),ObservationResult::Pending(chunks) if chunks.contains(&removed))
    );
    let current = runtime.resident_chunk(removed).expect("resident product");
    projection
        .publish(Arc::clone(&current.package), current.revision)
        .expect("republish");
    ready(
        projection
            .observe(&observer, &runtime)
            .expect("complete again"),
    );
    runtime.set_interests(Vec::new()).expect("stream out");
    assert!(matches!(
        projection
            .observe(&observer, &runtime)
            .expect("query disappeared"),
        ObservationResult::Pending(_)
    ));
    assert!(matches!(
        projection
            .observe(&request("outside", point(9000, 9000), 0, 4), &runtime)
            .expect("outside observer"),
        ObservationResult::OutsideWorld
    ));
}

#[test]
fn separated_observers_cache_independently_and_ignore_dormant_world_growth() {
    let a = point(0, 0);
    let b = point(1000, 1000);
    let mut regions = vec![(a, 5), (b, 5)];
    regions.extend((0..1000).map(|index| (point(10_000 + i64::from(index) * 32, 50_000), 0)));
    let package = world(&regions, 0);
    let qa = request("a", a, 0, 3);
    let qb = request("b", b, 0, 3);
    let (mut runtime, mut projection) = fixture(
        package.clone(),
        &[qa.clone(), qb.clone()],
        PerceptionConfig::default(),
    );
    let first_a = ready(projection.observe(&qa, &runtime).expect("a"));
    let first_b = ready(projection.observe(&qb, &runtime).expect("b"));
    assert_eq!(first_a.inspected_columns, first_b.inspected_columns);
    assert!(Arc::ptr_eq(
        &first_a,
        &ready(projection.observe(&qa, &runtime).expect("cached a"))
    ));
    let changed = a.checked_add(point(1, 0)).expect("adjacent");
    runtime
        .apply_transaction(&WorldEditTransaction {
            id: "local-edit".into(),
            expected_revisions: BTreeMap::from([(changed.chunk(), 0)]),
            edits: vec![VoxelEdit {
                position: voxel(changed, 0),
                material: None,
            }],
        })
        .expect("local terrain edit");
    flush(&mut runtime, &mut projection);
    assert_eq!(projection.counts().cached_observers, 1);
    assert!(Arc::ptr_eq(
        &first_b,
        &ready(
            projection
                .observe(&qb, &runtime)
                .expect("unrelated b cache")
        )
    ));
    let next_a = ready(projection.observe(&qa, &runtime).expect("revised a"));
    assert!(!Arc::ptr_eq(&first_a, &next_a));
    assert!(next_a
        .surfaces
        .iter()
        .any(|fact| fact.surface.position == voxel(changed, -1) && fact.world_revision == 1));
    assert_eq!(first_a.principal, "party-a");
    assert_eq!(first_b.principal, "party-b");
    assert!(runtime.counts().resident_chunks < 20);
    let dormant = package
        .chunks
        .values()
        .find(|chunk| chunk.coordinate.q > 100)
        .expect("dormant chunk")
        .clone();
    projection
        .publish(Arc::new(dormant), 0)
        .expect("unrelated publication");
    assert!(Arc::ptr_eq(
        &first_b,
        &ready(projection.observe(&qb, &runtime).expect("still cached b"))
    ));
}

#[test]
fn influencing_light_survives_its_owner_chunk_being_unloaded() {
    let observer_at = point(20, 5);
    let light_at = point(15, 5);
    let mut package = world(&[(point(18, 5), 12)], 0);
    package
        .chunks
        .get_mut(&light_at.chunk())
        .expect("owner")
        .semantics
        .lights
        .push(WorldLight {
            id: "remote-lamp".into(),
            position: voxel(light_at, 1),
            domain: None,
            bright_radius: 10,
            dim_radius: 12,
        });
    package.seal().expect("light projections");
    let mut observer = request("a", observer_at, 0, 2);
    observer.profile.dim = SightBand::new(1);
    observer.profile.dark = SightBand::new(0);
    observer.exterior = ExteriorIllumination::new(IlluminationLevel::Dark);
    let (runtime, mut projection) = fixture(
        package,
        std::slice::from_ref(&observer),
        PerceptionConfig::default(),
    );
    assert_eq!(
        runtime.revision(light_at.chunk()),
        None,
        "root owner remains unloaded"
    );
    let facts = ready(
        projection
            .observe(&observer, &runtime)
            .expect("projected illumination"),
    );
    assert!(facts
        .surfaces
        .iter()
        .any(|fact| fact.surface.position == voxel(point(22, 5), 0)
            && fact.illumination == IlluminationLevel::Bright));
}

#[test]
fn interior_light_domains_cover_object_supports_and_do_not_leak_exterior_light() {
    let mut package = world(&[(point(0, 0), 6)], 0);
    for chunk in package.chunks.values_mut() {
        for column in &mut chunk.columns {
            if column.position.q <= 0 {
                column.runs.push(run(5, 7));
                chunk.semantics.interiors.push(InteriorSpan {
                    id: "room".into(),
                    column: column.position,
                    floor_level: 0,
                    roof_bottom: 5,
                    roof_top: 7,
                    light_domain: "cave".into(),
                });
            }
        }
    }
    let lamp_at = point(-1, 0);
    package
        .chunks
        .get_mut(&lamp_at.chunk())
        .expect("owner")
        .semantics
        .lights
        .push(WorldLight {
            id: "interior-lamp".into(),
            position: voxel(lamp_at, 1),
            domain: Some("cave".into()),
            bright_radius: 5,
            dim_radius: 5,
        });
    add_object(&mut package, point(-3, 0), point(-2, 0), 1, 2);
    package.seal().expect("interior projections");
    let mut observer = request("a", lamp_at, 0, 3);
    observer.profile.dark = SightBand::new(0);
    observer.exterior = ExteriorIllumination::new(IlluminationLevel::Dark);
    let (runtime, mut projection) = fixture(
        package,
        std::slice::from_ref(&observer),
        PerceptionConfig::default(),
    );
    let facts = ready(
        projection
            .observe(&observer, &runtime)
            .expect("interior observations"),
    );
    assert!(facts
        .surfaces
        .iter()
        .any(|fact| fact.surface.position == voxel(point(-2, 0), 1)
            && fact.illumination == IlluminationLevel::Bright));
    assert!(
        !facts
            .surfaces
            .iter()
            .any(|fact| fact.surface.position.column.q > 0),
        "interior light does not illuminate exterior floor"
    );
}

#[test]
fn finite_world_holes_are_private_sight_barriers_and_never_support_facts() {
    let a = point(0, 0);
    let b = point(4, 0);
    let observer = request("a", a, 0, 6);
    let (runtime, mut projection) = fixture(
        world(&[(a, 0), (b, 0)], 0),
        std::slice::from_ref(&observer),
        PerceptionConfig::default(),
    );
    let facts = ready(
        projection
            .observe(&observer, &runtime)
            .expect("world edge observation"),
    );
    assert_eq!(
        facts
            .surfaces
            .iter()
            .map(|fact| fact.surface.position)
            .collect::<Vec<_>>(),
        vec![voxel(a, 0)]
    );
    assert_eq!(
        runtime.voxel(voxel(point(1, 0), 0)),
        QueryResult::OutsideWorld
    );
}

#[test]
fn sight_budgets_and_unrepresentable_target_ranges_fail_explicitly() {
    let at = point(0, 0);
    let observer = request("a", at, 0, 2);
    let (runtime, mut projection) = fixture(
        world(&[(at, 4)], 0),
        std::slice::from_ref(&observer),
        PerceptionConfig {
            max_runs_per_observer: 1,
            ..PerceptionConfig::default()
        },
    );
    assert!(matches!(
        projection.observe(&observer, &runtime),
        Err(Error::Limit(_))
    ));
    assert_eq!(projection.counts().cached_observers, 0);
    assert!(matches!(
        projection.required_chunks(&request("huge", at, 0, u32::MAX)),
        Err(Error::Limit(_))
    ));
    let level = i32::MAX - 8;
    let mut package = world(&[(at, 0)], level);
    column_mut(&mut package, at)
        .runs
        .insert(0, run(i32::MIN + 2, i32::MIN + 4));
    package.seal().expect("exact distant stacks");
    let observer = request("far-stacks", at, level, 0);
    let (runtime, mut projection) = fixture(
        package,
        std::slice::from_ref(&observer),
        PerceptionConfig::default(),
    );
    assert!(matches!(
        projection.observe(&observer, &runtime),
        Err(Error::Limit(_))
    ));
}

#[test]
fn landmark_observation_is_exact_and_residency_does_not_disclose_hidden_features() {
    let origin = point(0, 0);
    let mut package = world(&[(origin, 6)], 0);
    let near = FeatureSummary {
        id: "seen".into(),
        region_id: "region-0000".into(),
        kind: "landmark".into(),
        anchor: voxel(point(1, 0), 0),
        asset: None,
    };
    let far = FeatureSummary {
        id: "hidden".into(),
        region_id: "region-0000".into(),
        kind: "landmark".into(),
        anchor: voxel(point(5, 0), 0),
        asset: None,
    };
    for feature in [near.clone(), far] {
        package.manifest.features.push(feature.clone());
        package
            .chunks
            .get_mut(&feature.anchor.column.chunk())
            .expect("owner")
            .features
            .push(feature);
    }
    // An observation anchor does not itself grant a visible feature or walker support.
    package
        .chunks
        .get_mut(&origin.chunk())
        .expect("owner")
        .semantics
        .anchors
        .push(WorldAnchor {
            id: "scenic".into(),
            region_id: "region-0000".into(),
            position: voxel(origin, 10),
            role: AnchorRole::Observation,
        });
    package.seal().expect("features");
    let observer = request("a", origin, 0, 3);
    let (runtime, mut projection) = fixture(
        package,
        std::slice::from_ref(&observer),
        PerceptionConfig::default(),
    );
    let facts = ready(
        projection
            .observe(&observer, &runtime)
            .expect("landmark observations"),
    );
    assert_eq!(
        facts
            .landmarks
            .iter()
            .map(|fact| &fact.feature)
            .collect::<Vec<_>>(),
        vec![&near]
    );
}

#[test]
fn remembered_visible_destruction_invalidates_only_the_missing_exact_support() {
    let observer = request("a", point(0, 0), 0, 4);
    let erased = voxel(point(0, 1), 0);
    let surviving = voxel(point(0, 2), 0);
    let (mut runtime, mut projection) = fixture(
        world(&[(observer.position.column, 6)], 0),
        std::slice::from_ref(&observer),
        PerceptionConfig::default(),
    );
    let initial = ready(
        projection
            .observe(&observer, &runtime)
            .expect("initial sight"),
    );
    assert!(initial
        .surfaces
        .iter()
        .any(|fact| fact.surface.position == erased));
    runtime
        .apply_transaction(&WorldEditTransaction {
            id: "visible-destruction".into(),
            expected_revisions: BTreeMap::from([(erased.column.chunk(), 0)]),
            edits: vec![VoxelEdit {
                position: erased,
                material: None,
            }],
        })
        .expect("destroy visible support");
    flush(&mut runtime, &mut projection);
    let without_memory = ready(
        projection
            .observe(&observer, &runtime)
            .expect("current sight"),
    );
    assert!(without_memory.invalidated_surfaces.is_empty());
    let with_memory = ready(
        projection
            .observe_with_memory(&observer, &[erased, surviving], &runtime)
            .expect("memory-aware sight"),
    );
    assert!(!Arc::ptr_eq(&without_memory, &with_memory));
    assert_eq!(with_memory.invalidated_surfaces, vec![erased]);
    assert!(with_memory
        .surfaces
        .iter()
        .any(|fact| fact.surface.position == voxel(erased.column, -1)));
    assert!(with_memory
        .surfaces
        .iter()
        .any(|fact| fact.surface.position == surviving));
    let cached = ready(
        projection
            .observe_with_memory(&observer, &[surviving, erased, erased], &runtime)
            .expect("canonical memory cache"),
    );
    assert!(Arc::ptr_eq(&cached, &with_memory));
    let cleared = ready(
        projection
            .observe(&observer, &runtime)
            .expect("memory removed"),
    );
    assert!(cleared.invalidated_surfaces.is_empty());
    assert!(!Arc::ptr_eq(&cached, &cleared));
}

#[test]
fn hidden_destroyed_or_pending_remembered_supports_never_erase_memory() {
    let observer = request("a", point(0, 0), 0, 4);
    let hidden = voxel(point(3, 0), 0);
    let mut package = world(&[(observer.position.column, 7)], 0);
    add_object(&mut package, point(1, 0), point(1, 0), 1, 8);
    package.seal().expect("opaque object fixture");
    let (mut runtime, mut projection) = fixture(
        package,
        std::slice::from_ref(&observer),
        PerceptionConfig::default(),
    );
    let before = ready(
        projection
            .observe(&observer, &runtime)
            .expect("blocked sight"),
    );
    assert!(!before
        .surfaces
        .iter()
        .any(|fact| fact.surface.position == hidden));
    runtime
        .apply_transaction(&WorldEditTransaction {
            id: "hidden-destruction".into(),
            expected_revisions: BTreeMap::from([(hidden.column.chunk(), 0)]),
            edits: vec![VoxelEdit {
                position: hidden,
                material: None,
            }],
        })
        .expect("destroy hidden support");
    flush(&mut runtime, &mut projection);
    let memory = [
        hidden,
        voxel(point(6, 0), 99),
        voxel(point(i64::MIN, i64::MAX), i32::MAX),
    ];
    let after = ready(
        projection
            .observe_with_memory(&observer, &memory, &runtime)
            .expect("hidden and distant memories retained"),
    );
    assert!(after.invalidated_surfaces.is_empty());
    projection.remove(hidden.column.chunk());
    assert!(matches!(
        projection
            .observe_with_memory(&observer, &memory, &runtime)
            .expect("missing source"),
        ObservationResult::Pending(_)
    ));
    let source = runtime
        .resident_chunk(hidden.column.chunk())
        .expect("still resident authority");
    projection
        .publish(source.package, source.revision)
        .expect("republish");
    runtime.set_interests(Vec::new()).expect("unload authority");
    assert!(matches!(
        projection
            .observe_with_memory(&observer, &memory, &runtime)
            .expect("missing authority"),
        ObservationResult::Pending(_)
    ));
}

#[test]
fn remembered_support_input_is_bounded_before_copying_or_hashing() {
    let observer = request("a", point(0, 0), 0, 2);
    let (runtime, mut projection) = fixture(
        world(&[(observer.position.column, 4)], 0),
        std::slice::from_ref(&observer),
        PerceptionConfig {
            max_remembered_positions: 1,
            ..PerceptionConfig::default()
        },
    );
    let before = projection.counts();
    assert!(matches!(
        projection.observe_with_memory(&observer, &[observer.position; 2], &runtime),
        Err(Error::Limit(_))
    ));
    assert_eq!(projection.counts(), before);
}

#[test]
fn non_solid_terrain_and_object_volumes_match_air_in_the_exact_sight_projection() {
    let observer = request("a", point(0, 0), 0, 4);
    let target = voxel(point(3, 0), 0);
    for object in [false, true] {
        let mut observations = Vec::new();
        for solid in [None, Some(false), Some(true)] {
            let mut package = world(&[(observer.position.column, 6)], 0);
            if let Some(solid) = solid {
                package.manifest.materials.push(MaterialSpec {
                    id: "volume".into(),
                    solid,
                    diggable: true,
                    color: [0, 0, 255, 255],
                });
                if object {
                    add_object(&mut package, point(1, 0), point(1, 0), 1, 8);
                    package
                        .chunks
                        .get_mut(&point(1, 0).chunk())
                        .expect("object chunk")
                        .semantics
                        .objects
                        .first_mut()
                        .expect("object")
                        .occupancy
                        .first_mut()
                        .expect("occupancy column")
                        .runs
                        .first_mut()
                        .expect("occupancy run")
                        .material = "volume".into();
                } else {
                    column_mut(&mut package, point(1, 0)).runs.push(VoxelRun {
                        bottom: 1,
                        top: 8,
                        material: "volume".into(),
                    });
                }
                package.seal().expect("material policy fixture");
            }
            let (runtime, mut projection) = fixture(
                package,
                std::slice::from_ref(&observer),
                PerceptionConfig::default(),
            );
            let facts = ready(
                projection
                    .observe(&observer, &runtime)
                    .expect("volume sight"),
            );
            // The volume column itself has different candidate support/air-clearance facts.
            // Compare every unchanged target around and behind it to isolate the LOS policy.
            observations.push(
                facts
                    .surfaces
                    .iter()
                    .map(|fact| fact.surface.position)
                    .filter(|position| position.column != point(1, 0))
                    .collect::<BTreeSet<_>>(),
            );
        }
        let [air, non_solid, solid]: [BTreeSet<VoxelPosition>; 3] = observations
            .try_into()
            .expect("air, non-solid and solid fixture results");
        assert_eq!(
            air, non_solid,
            "non-solid volume must behave like air for sight; object={object}"
        );
        assert!(air.contains(&target));
        assert!(
            !solid.contains(&target),
            "solid volume must block the same exact ray; object={object}"
        );
    }
}

fn asset_landmark(package: &mut WorldPackage, at: WorldHex) -> (FeatureSummary, ObjectInstance) {
    let feature = FeatureSummary {
        id: "tree".into(),
        region_id: "region-0000".into(),
        kind: "landmark".into(),
        anchor: voxel(at, 0),
        asset: Some("test-tree".into()),
    };
    let object = ObjectInstance {
        id: feature.id.clone(),
        region_id: feature.region_id.clone(),
        asset: feature.asset.clone().expect("asset"),
        origin: voxel(at, 1),
        rotation: 0,
        occupancy: vec![ColumnData {
            position: at,
            runs: vec![run(1, 3)],
        }],
    };
    package.manifest.features.push(feature.clone());
    let chunk = package.chunks.get_mut(&at.chunk()).expect("owner");
    chunk.features.push(feature.clone());
    chunk.semantics.objects.push(object.clone());
    (feature, object)
}

fn remove_landmark_object(runtime: &mut WorldRuntime, object: ObjectInstance) {
    let expected_revisions = object
        .dependency_chunks()
        .expect("dependencies")
        .into_iter()
        .map(|coordinate| (coordinate, runtime.revision(coordinate).expect("resident")))
        .collect();
    runtime
        .apply_object_transaction(&hex_world_contracts::WorldObjectEditTransaction {
            id: "remove-tree".into(),
            expected_revisions,
            edits: vec![hex_world_contracts::ObjectEdit {
                before: Some(object),
                after: None,
            }],
        })
        .expect("object removal");
}

#[test]
fn asset_landmarks_follow_live_objects_and_only_known_visible_absence_is_disclosed() {
    let observer = request("a", point(0, 0), 0, 4);
    let mut package = world(&[(point(0, 0), 6)], 0);
    let (feature, object) = asset_landmark(&mut package, point(1, 0));
    let mut semantic = feature.clone();
    semantic.id = "semantic-anchor".into();
    semantic.asset = None;
    package.manifest.features.push(semantic.clone());
    package
        .chunks
        .get_mut(&semantic.anchor.column.chunk())
        .expect("owner")
        .features
        .push(semantic.clone());
    package.seal().expect("asset fixture");
    let (mut runtime, mut projection) = fixture(
        package,
        std::slice::from_ref(&observer),
        PerceptionConfig::default(),
    );
    let initial = ready(
        projection
            .observe(&observer, &runtime)
            .expect("visible object"),
    );
    assert!(initial.landmarks.iter().any(|fact| fact.feature == feature));
    remove_landmark_object(&mut runtime, object);
    flush(&mut runtime, &mut projection);
    let current = ready(
        projection
            .observe(&observer, &runtime)
            .expect("current absence"),
    );
    assert_eq!(
        current
            .landmarks
            .iter()
            .map(|fact| &fact.feature)
            .collect::<Vec<_>>(),
        vec![&semantic]
    );
    assert!(
        current.invalidated_landmarks.is_empty(),
        "never disclose an unremembered deleted source ID"
    );
    let memory = RememberedLandmark {
        id: feature.id.clone(),
        position: feature.anchor,
    };
    let known = ready(
        projection
            .observe_with_landmark_memory(&observer, &[], std::slice::from_ref(&memory), &runtime)
            .expect("known absence"),
    );
    assert_eq!(
        known.invalidated_landmarks,
        vec![InvalidatedLandmark {
            id: feature.id.clone(),
            position: feature.anchor,
            world_revision: 1,
        }]
    );
    assert!(!Arc::ptr_eq(&known, &current));
    let cached = ready(
        projection
            .observe_with_landmark_memory(&observer, &[], &[memory.clone(), memory], &runtime)
            .expect("canonical duplicate memory"),
    );
    assert!(Arc::ptr_eq(&known, &cached));
    assert!(ready(
        projection
            .observe(&observer, &runtime)
            .expect("memory cleared")
    )
    .invalidated_landmarks
    .is_empty());
}

#[test]
fn hidden_removed_landmark_is_remembered_until_visible_and_pending_never_proves_absence() {
    let clear_observer = request("a", point(3, -1), 0, 5);
    let hidden_observer = request("a", point(0, 0), 0, 5);
    let mut package = world(&[(point(0, 0), 7)], 0);
    let (feature, object) = asset_landmark(&mut package, point(3, 0));
    add_object(&mut package, point(1, 0), point(1, 0), 1, 8);
    package.seal().expect("occluding object fixture");
    let (mut runtime, mut projection) = fixture(
        package,
        std::slice::from_ref(&clear_observer),
        PerceptionConfig::default(),
    );
    let before = ready(
        projection
            .observe(&clear_observer, &runtime)
            .expect("initial view"),
    );
    assert!(before.landmarks.iter().any(|fact| fact.feature == feature));
    let memory = [RememberedLandmark {
        id: feature.id.clone(),
        position: feature.anchor,
    }];
    remove_landmark_object(&mut runtime, object);
    flush(&mut runtime, &mut projection);
    let hidden = ready(
        projection
            .observe_with_landmark_memory(&hidden_observer, &[], &memory, &runtime)
            .expect("hidden deletion"),
    );
    assert!(hidden.landmarks.is_empty());
    assert!(hidden.invalidated_landmarks.is_empty());
    projection.remove(feature.anchor.column.chunk());
    assert!(matches!(
        projection
            .observe_with_landmark_memory(&clear_observer, &[], &memory, &runtime)
            .expect("unavailable"),
        ObservationResult::Pending(_)
    ));
    let product = runtime
        .resident_chunk(feature.anchor.column.chunk())
        .expect("authority source");
    projection
        .publish(product.package, product.revision)
        .expect("restore source");
    let revealed = ready(
        projection
            .observe_with_landmark_memory(&clear_observer, &[], &memory, &runtime)
            .expect("visible absence"),
    );
    assert_eq!(revealed.invalidated_landmarks.len(), 1);
    assert_eq!(
        revealed
            .invalidated_landmarks
            .first()
            .expect("one invalidated landmark")
            .id,
        feature.id
    );
}

#[test]
fn remembered_landmarks_are_bounded_and_bound_to_exact_registered_anchors() {
    let observer = request("a", point(0, 0), 0, 4);
    let mut package = world(&[(point(0, 0), 6)], 0);
    let (feature, _) = asset_landmark(&mut package, point(1, 0));
    package.seal().expect("fixture");
    let (runtime, mut projection) = fixture(
        package,
        std::slice::from_ref(&observer),
        PerceptionConfig {
            max_landmarks_per_observer: 1,
            ..PerceptionConfig::default()
        },
    );
    let memory = RememberedLandmark {
        id: feature.id,
        position: feature.anchor,
    };
    assert!(matches!(
        projection.observe_with_landmark_memory(
            &observer,
            &[],
            &[memory.clone(), memory.clone()],
            &runtime
        ),
        Err(Error::Limit(_))
    ));
    assert!(matches!(
        projection.observe_with_landmark_memory(
            &observer,
            &[],
            &[RememberedLandmark {
                position: voxel(point(2, 0), 0),
                ..memory
            }],
            &runtime
        ),
        Err(Error::Invalid(_))
    ));
}
