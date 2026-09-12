//! Independent contract oracles for geometry, packages, availability and semantics.
#![expect(
    clippy::expect_used,
    reason = "Assertions unwrap only test-controlled fixture construction."
)]

use hex_world_contracts::*;
use std::collections::BTreeMap;

fn run(bottom: i32, top: i32, material: &str) -> VoxelRun {
    VoxelRun {
        bottom,
        top,
        material: material.into(),
    }
}

fn column(position: WorldHex) -> ColumnData {
    ColumnData {
        position,
        runs: vec![run(-4, 1, "stone")],
    }
}

fn materials() -> Vec<MaterialSpec> {
    vec![
        MaterialSpec {
            id: "stone".into(),
            solid: true,
            diggable: true,
            color: [100, 110, 120, 255],
        },
        MaterialSpec {
            id: "water".into(),
            solid: false,
            diggable: false,
            color: [0, 80, 220, 180],
        },
    ]
}

fn world(origin: WorldHex, radius: u32) -> WorldPackage {
    let mut chunks: BTreeMap<ChunkId, ChunkPackage> = BTreeMap::new();
    let extent = i64::from(radius);
    for q in -extent..=extent {
        for r in -extent..=extent {
            // Independent disk predicate for these small fixtures.
            if q.abs().max(r.abs()).max((q + r).abs()) > extent {
                continue;
            }
            let position = origin
                .checked_add(WorldHex::new(q, r))
                .expect("small fixture coordinate");
            chunks
                .entry(position.chunk())
                .or_insert_with(|| ChunkPackage {
                    schema_version: SCHEMA_VERSION,
                    world_id: "world".into(),
                    coordinate: position.chunk(),
                    source_fingerprint: 12,
                    columns: Vec::new(),
                    features: Vec::new(),
                    semantics: ChunkSemantics::default(),
                    fingerprint: 0,
                })
                .columns
                .push(column(position));
        }
    }
    let descriptors = chunks
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
            world_id: "world".into(),
            compiler_version: "fixture-v1".into(),
            source_fingerprint: 12,
            materials: materials(),
            regions: vec![RegionDescriptor {
                id: "region".into(),
                origin,
                radius,
                source_fingerprint: 10,
            }],
            chunks: descriptors,
            boundaries: Vec::new(),
            summary: Vec::new(),
            features: Vec::new(),
            fingerprint: 0,
        },
        chunks,
    };
    package.seal().expect("valid fixture");
    package
}

#[test]
fn negative_chunks_use_euclidean_not_truncating_division() {
    for (position, chunk, local) in [
        (WorldHex::new(-1, -1), ChunkId { q: -1, r: -1 }, (15, 15)),
        (WorldHex::new(-16, -17), ChunkId { q: -1, r: -2 }, (0, 15)),
        (WorldHex::new(16, 0), ChunkId { q: 1, r: 0 }, (0, 0)),
    ] {
        assert_eq!(position.chunk(), chunk);
        assert_eq!(position.local(), local);
        assert_eq!(
            chunk
                .origin()
                .expect("valid chunk")
                .checked_add(WorldHex::new(local.0, local.1))
                .expect("valid offset"),
            position
        );
    }
}

#[test]
fn distances_and_six_rotations_preserve_exact_geometry() {
    let point = WorldHex::new(3, -5);
    let expected = [
        WorldHex::new(3, -5),
        WorldHex::new(5, -2),
        WorldHex::new(2, 3),
        WorldHex::new(-3, 5),
        WorldHex::new(-5, 2),
        WorldHex::new(-2, -3),
    ];
    for (turns, target) in (0_u8..6).zip(expected) {
        assert_eq!(point.rotate_60(turns).expect("rotation"), target);
        assert_eq!(
            target
                .checked_distance(WorldHex::default())
                .expect("distance"),
            5
        );
    }
    assert_eq!(point.rotate_60(6).expect("full rotation"), point);
    assert_eq!(
        WorldHex::new(-3, 2)
            .checked_distance(WorldHex::new(4, -1))
            .expect("distance"),
        7
    );
}

#[test]
fn extreme_geometry_never_silently_wraps() {
    assert!(WorldHex::new(i64::MAX, 0)
        .checked_add(WorldHex::new(1, 0))
        .is_err());
    assert!(WorldHex::new(i64::MIN, 0).rotate_60(3).is_err());
    assert!(ChunkId { q: i64::MAX, r: 0 }.origin().is_err());
    assert!(WorldHex::new(i64::MIN, i64::MIN)
        .checked_distance(WorldHex::new(i64::MAX, i64::MAX))
        .is_err());
    assert_eq!(
        WorldHex::new(i64::MIN, i64::MIN)
            .rotate_60(6)
            .expect("identity remains representable"),
        WorldHex::new(i64::MIN, i64::MIN)
    );
}

#[test]
fn sealing_coalesces_adjacent_runs_without_collapsing_stacks() {
    let mut data = ColumnData {
        position: WorldHex::default(),
        runs: vec![run(8, 10, "stone"), run(-2, 0, "stone"), run(0, 2, "stone")],
    };
    assert!(data.validate().is_err());
    data.seal().expect("valid intervals");
    assert_eq!(data.runs, vec![run(-2, 2, "stone"), run(8, 10, "stone")]);
    assert_eq!(data.material_at(2), None);
    assert_eq!(data.material_at(8), Some("stone"));
    let surfaces = data.surfaces(&materials()).expect("registered materials");
    assert_eq!(
        surfaces
            .iter()
            .map(|s| (s.position.level, s.headroom))
            .collect::<Vec<_>>(),
        vec![(1, Some(6)), (9, None)]
    );
}

#[test]
fn occupied_nonsolid_intervals_do_not_become_air_clearance() {
    let data = ColumnData {
        position: WorldHex::default(),
        runs: vec![run(-2, 1, "stone"), run(1, 3, "water"), run(5, 6, "stone")],
    };
    let surfaces = data.surfaces(&materials()).expect("materials");
    assert_eq!(surfaces.len(), 1);
    assert_eq!(surfaces.first().expect("bridge").position.level, 5);
}

#[test]
fn invalid_runs_are_rejected_and_failed_seal_is_atomic() {
    for runs in [
        vec![run(0, 0, "stone")],
        vec![run(0, 3, "stone"), run(2, 4, "stone")],
        vec![run(0, 1, "air")],
    ] {
        let mut data = ColumnData {
            position: WorldHex::default(),
            runs,
        };
        let old = data.clone();
        assert!(data.validate().is_err());
        assert!(data.seal().is_err());
        assert_eq!(data, old);
    }
}

#[test]
fn fingerprint_excludes_its_own_field_but_detects_content_changes() {
    let mut package = world(WorldHex::default(), 1);
    let chunk = package.chunks.values_mut().next().expect("chunk");
    let before = fingerprint(chunk).expect("hash");
    chunk.fingerprint = 123;
    assert_eq!(fingerprint(chunk).expect("hash"), before);
    assert!(chunk.validate().is_err());
    chunk.seal().expect("restore");
    chunk
        .columns
        .first_mut()
        .expect("column")
        .runs
        .first_mut()
        .expect("run")
        .top = 2;
    assert!(chunk.validate().is_err());
    assert_ne!(fingerprint(chunk).expect("hash"), before);
}

#[test]
fn producer_order_does_not_change_canonical_package_identity() {
    let expected = world(WorldHex::new(15, 0), 2);
    let mut shuffled = expected.clone();
    shuffled.manifest.materials.reverse();
    shuffled.manifest.chunks.reverse();
    for chunk in shuffled.chunks.values_mut() {
        chunk.columns.reverse();
    }
    assert!(shuffled.validate().is_err());
    shuffled.seal().expect("canonicalize producer");
    assert_eq!(expected, shuffled);
    assert_eq!(
        fingerprint(&expected).expect("hash"),
        fingerprint(&shuffled).expect("hash")
    );
}

#[test]
fn duplicate_ids_columns_and_chunk_paths_cannot_be_sealed_away() {
    let mut duplicate_material = world(WorldHex::default(), 1);
    duplicate_material.manifest.materials.push(
        duplicate_material
            .manifest
            .materials
            .first()
            .expect("material")
            .clone(),
    );
    assert!(duplicate_material.seal().is_err());
    let mut duplicate_column = world(WorldHex::default(), 1);
    let chunk = duplicate_column.chunks.values_mut().next().expect("chunk");
    chunk
        .columns
        .push(chunk.columns.first().expect("column").clone());
    assert!(duplicate_column.seal().is_err());
    let mut duplicate_path = world(WorldHex::default(), 1);
    let path = duplicate_path
        .manifest
        .chunks
        .first()
        .expect("descriptor")
        .path
        .clone();
    duplicate_path
        .manifest
        .chunks
        .iter_mut()
        .for_each(|chunk| chunk.path = path.clone());
    assert!(duplicate_path.seal().is_err());
}

#[test]
fn paths_reject_cross_platform_traversal_and_aliases() {
    for path in [
        "",
        "/tmp/chunk",
        "../chunk",
        "a/../chunk",
        "a//chunk",
        "./chunk",
        "a\\chunk",
        "C:chunk",
        "a/ chunk",
        "a/chunk\n",
    ] {
        assert!(validate_package_path(path).is_err(), "accepted {path:?}");
    }
    assert!(validate_package_path("chunks/-1_2.ron").is_ok());
}

#[test]
fn wire_unknown_fields_and_duplicate_map_keys_are_rejected() {
    assert!(ron::from_str::<WorldHex>("(q:0,r:0,z:0)").is_err());
    let duplicate="(id:\"edit\",expected_revisions:{(q:0,r:0):1,(q:0,r:0):2},edits:[(position:(column:(q:0,r:0),level:0),material:None)])";
    assert!(ron::from_str::<WorldEditTransaction>(duplicate).is_err());
    let mut package = world(WorldHex::default(), 0);
    package.manifest.schema_version = 2;
    assert!(parse_ron::<WorldPackage>(&ron::ser::to_string(&package).expect("serialize")).is_err());
}

#[test]
fn complete_world_roundtrips_and_untrusted_hash_mutation_fails() {
    let package = world(WorldHex::new(-16, 15), 3);
    let wire = ron::ser::to_string(&package).expect("serialize");
    assert_eq!(
        parse_ron::<WorldPackage>(&wire).expect("validated roundtrip"),
        package
    );
    let changed = wire.replacen(
        "compiler_version:\"fixture-v1\"",
        "compiler_version:\"different\"",
        1,
    );
    assert_ne!(wire, changed);
    assert!(parse_ron::<WorldPackage>(&changed).is_err());
}

#[test]
fn wrong_world_membership_materials_and_missing_chunks_are_rejected() {
    let mut wrong_world = world(WorldHex::default(), 1);
    wrong_world
        .chunks
        .values_mut()
        .next()
        .expect("chunk")
        .world_id = "other".into();
    assert!(wrong_world.seal().is_err());
    let mut unknown = world(WorldHex::default(), 1);
    unknown
        .chunks
        .values_mut()
        .next()
        .expect("chunk")
        .columns
        .first_mut()
        .expect("column")
        .runs
        .first_mut()
        .expect("run")
        .material = "absent".into();
    assert!(unknown.seal().is_err());
    let mut missing = world(WorldHex::default(), 1);
    let key = *missing.chunks.keys().next().expect("key");
    missing.chunks.remove(&key);
    missing
        .manifest
        .chunks
        .retain(|chunk| chunk.coordinate != key);
    assert!(missing.seal().is_err());
    let mut wrong_member = world(WorldHex::default(), 0);
    wrong_member
        .chunks
        .values_mut()
        .next()
        .expect("chunk")
        .columns
        .first_mut()
        .expect("column")
        .position = WorldHex::new(16, 0);
    assert!(wrong_member.seal().is_err());
}

#[test]
fn interior_and_anchor_contracts_use_exact_air_not_a_capped_probe() {
    let mut package = world(WorldHex::default(), 0);
    let chunk = package.chunks.values_mut().next().expect("chunk");
    chunk
        .columns
        .first_mut()
        .expect("column")
        .runs
        .push(run(14, 18, "stone"));
    chunk.semantics.interiors.push(InteriorSpan {
        id: "cave".into(),
        column: WorldHex::default(),
        floor_level: 0,
        roof_bottom: 14,
        roof_top: 18,
        light_domain: "cave-light".into(),
    });
    chunk.semantics.anchors.push(WorldAnchor {
        id: "walk".into(),
        region_id: "region".into(),
        position: VoxelPosition::default(),
        role: AnchorRole::Gameplay,
    });
    chunk.semantics.lights.push(WorldLight {
        id: "lamp".into(),
        position: VoxelPosition {
            column: WorldHex::default(),
            level: 2,
        },
        domain: Some("cave-light".into()),
        bright_radius: 4,
        dim_radius: 18,
    });
    package.seal().expect("clear 13-level interior");
    let chunk = package.chunks.values_mut().next().expect("chunk");
    chunk
        .columns
        .first_mut()
        .expect("column")
        .runs
        .push(run(5, 6, "stone"));
    assert!(package.seal().is_err());
}

#[test]
fn observation_anchor_does_not_imply_walker_placement() {
    let mut package = world(WorldHex::default(), 0);
    let chunk = package.chunks.values_mut().next().expect("chunk");
    chunk.semantics.anchors.push(WorldAnchor {
        id: "scenic".into(),
        region_id: "region".into(),
        position: VoxelPosition {
            column: WorldHex::default(),
            level: 300,
        },
        role: AnchorRole::Observation,
    });
    package.seal().expect("scenic anchor can look at air");
    package
        .chunks
        .values_mut()
        .next()
        .expect("chunk")
        .semantics
        .anchors
        .first_mut()
        .expect("anchor")
        .role = AnchorRole::Gameplay;
    assert!(package.seal().is_err());
}

#[test]
fn object_root_and_full_cross_chunk_occupancy_survive_roundtrip() {
    let mut package = world(WorldHex::new(15, 0), 1);
    let owner = WorldHex::new(15, 0).chunk();
    package
        .chunks
        .get_mut(&owner)
        .expect("owner")
        .semantics
        .objects
        .push(ObjectInstance {
            id: "region/tree".into(),
            region_id: "region".into(),
            asset: "tree.oak".into(),
            origin: VoxelPosition {
                column: WorldHex::new(15, 0),
                level: 1,
            },
            rotation: 5,
            occupancy: vec![ColumnData {
                position: WorldHex::new(16, 0),
                runs: vec![run(2, 5, "stone")],
            }],
        });
    package.seal().expect("cross-chunk object");
    assert_eq!(
        package
            .chunks
            .get(&WorldHex::new(16, 0).chunk())
            .expect("foreign chunk")
            .semantics
            .occupancy,
        vec![ColumnData {
            position: WorldHex::new(16, 0),
            runs: vec![run(2, 5, "stone")]
        }]
    );
    let wire = ron::ser::to_string(&package).expect("serialize");
    assert_eq!(
        parse_ron::<WorldPackage>(&wire).expect("roundtrip"),
        package
    );
    package
        .chunks
        .get_mut(&owner)
        .expect("owner")
        .semantics
        .objects
        .first_mut()
        .expect("object")
        .occupancy
        .first_mut()
        .expect("column")
        .runs
        .first_mut()
        .expect("run")
        .material = "missing".into();
    assert!(package.seal().is_err());
}

#[test]
fn forged_or_conflicting_object_projections_are_rejected() {
    let mut package = world(WorldHex::new(15, 0), 1);
    let root = VoxelPosition {
        column: WorldHex::new(15, 0),
        level: 1,
    };
    let object = ObjectInstance {
        id: "one".into(),
        region_id: "region".into(),
        asset: "tree".into(),
        origin: root,
        rotation: 0,
        occupancy: vec![ColumnData {
            position: WorldHex::new(16, 0),
            runs: vec![run(2, 5, "stone")],
        }],
    };
    package
        .chunks
        .get_mut(&root.column.chunk())
        .expect("root")
        .semantics
        .objects
        .push(object.clone());
    package.seal().expect("valid projection");
    let remote = package
        .chunks
        .get_mut(&WorldHex::new(16, 0).chunk())
        .expect("remote");
    remote.semantics.occupancy.clear();
    assert!(remote.seal().is_err());
    for descriptor in &mut package.manifest.chunks {
        descriptor.fingerprint = package
            .chunks
            .get(&descriptor.coordinate)
            .expect("chunk")
            .fingerprint;
    }
    package.manifest.seal().expect("consistent hashes");
    assert!(package.validate().is_err());
    package.seal().expect("producer regenerates projection");
    let mut conflict = object;
    conflict.id = "two".into();
    conflict
        .occupancy
        .first_mut()
        .expect("column")
        .runs
        .first_mut()
        .expect("run")
        .material = "water".into();
    package
        .chunks
        .get_mut(&root.column.chunk())
        .expect("root")
        .semantics
        .objects
        .push(conflict);
    assert!(package.seal().is_err());
}

#[test]
fn bridge_boundary_keeps_water_and_walkable_deck_at_distinct_levels() {
    let mut package = world(WorldHex::new(15, 0), 1);
    package.manifest.regions = vec![
        RegionDescriptor {
            id: "a".into(),
            origin: WorldHex::new(15, 0),
            radius: 0,
            source_fingerprint: 1,
        },
        RegionDescriptor {
            id: "b".into(),
            origin: WorldHex::new(16, 0),
            radius: 0,
            source_fingerprint: 2,
        },
    ];
    for chunk in package.chunks.values_mut() {
        chunk.columns.retain(|column| {
            [WorldHex::new(15, 0), WorldHex::new(16, 0)].contains(&column.position)
        });
        for column in &mut chunk.columns {
            column.runs.extend([run(1, 3, "water"), run(5, 6, "stone")]);
            chunk.semantics.liquids.push(LiquidColumn {
                column: column.position,
                bottom: 1,
                top: 3,
                kind: LiquidKind::Standing,
                body_id: "lake".into(),
                downstream: Vec::new(),
            });
        }
    }
    package.chunks.retain(|_, chunk| !chunk.columns.is_empty());
    package
        .manifest
        .chunks
        .retain(|descriptor| package.chunks.contains_key(&descriptor.coordinate));
    package.manifest.boundaries.push(BoundaryContract {
        id: "shared".into(),
        region_a: "a".into(),
        region_b: "b".into(),
        samples: vec![BoundarySample {
            a: WorldHex::new(15, 0),
            b: WorldHex::new(16, 0),
            ground_level: 5,
            water_level: Some(2),
            required_access: true,
        }],
    });
    package
        .seal()
        .expect("deck surface independent from water surface");
    package
        .manifest
        .boundaries
        .first_mut()
        .expect("boundary")
        .samples
        .first_mut()
        .expect("sample")
        .ground_level = 0;
    assert!(
        package.seal().is_err(),
        "water above bed cannot masquerade as walkable air"
    );
}

#[test]
fn liquid_links_resolve_exact_stack_endpoints_across_chunks() {
    let mut package = world(WorldHex::new(15, 0), 1);
    for (position, kind, target) in [
        (
            WorldHex::new(15, 0),
            LiquidKind::Directed,
            Some(WorldHex::new(16, 0)),
        ),
        (WorldHex::new(16, 0), LiquidKind::Standing, None),
    ] {
        let chunk = package.chunks.get_mut(&position.chunk()).expect("chunk");
        chunk
            .columns
            .iter_mut()
            .find(|column| column.position == position)
            .expect("column")
            .runs
            .push(run(1, 3, "water"));
        chunk.semantics.liquids.push(LiquidColumn {
            column: position,
            bottom: 1,
            top: 3,
            kind,
            body_id: "river".into(),
            downstream: target
                .into_iter()
                .map(|column| VoxelPosition { column, level: 2 })
                .collect(),
        });
    }
    package.seal().expect("valid cross-chunk downstream");
    let liquid = package
        .chunks
        .get_mut(&WorldHex::new(15, 0).chunk())
        .expect("chunk")
        .semantics
        .liquids
        .first_mut()
        .expect("liquid");
    liquid.downstream.first_mut().expect("endpoint").level = 1;
    assert!(package.seal().is_err());
}

#[test]
fn liquid_cycles_fail_even_when_every_local_link_is_valid() {
    let mut package = world(WorldHex::new(15, 0), 1);
    for (position, target) in [
        (WorldHex::new(15, 0), WorldHex::new(16, 0)),
        (WorldHex::new(16, 0), WorldHex::new(15, 0)),
    ] {
        let chunk = package.chunks.get_mut(&position.chunk()).expect("chunk");
        chunk
            .columns
            .iter_mut()
            .find(|column| column.position == position)
            .expect("column")
            .runs
            .push(run(1, 3, "water"));
        chunk.semantics.liquids.push(LiquidColumn {
            column: position,
            bottom: 1,
            top: 3,
            kind: LiquidKind::Directed,
            body_id: "loop".into(),
            downstream: vec![VoxelPosition {
                column: target,
                level: 2,
            }],
        });
    }
    assert!(package.seal().is_err());
}

#[test]
fn transactions_preserve_stack_identity_and_require_complete_revisions() {
    let mut transaction = WorldEditTransaction {
        id: "tx".into(),
        expected_revisions: BTreeMap::from([(ChunkId { q: 0, r: 0 }, 4)]),
        edits: vec![
            VoxelEdit {
                position: VoxelPosition::default(),
                material: None,
            },
            VoxelEdit {
                position: VoxelPosition {
                    column: WorldHex::default(),
                    level: 3,
                },
                material: Some("stone".into()),
            },
        ],
    };
    transaction.validate().expect("two distinct stack edits");
    transaction
        .edits
        .push(transaction.edits.first().expect("edit").clone());
    assert!(transaction.validate().is_err());
    transaction.edits.pop();
    transaction.expected_revisions.clear();
    assert!(transaction.validate().is_err());
}

#[test]
fn residency_hysteresis_and_availability_are_explicit() {
    let request = ResidencyRequest {
        id: "actor".into(),
        center: WorldHex::default(),
        radius: 4,
        retention_radius: 3,
        priority: 0,
    };
    assert!(request.validate().is_err());
    let air: QueryResult<Option<String>> = QueryResult::Ready(None);
    assert_ne!(air, QueryResult::Unloaded(ChunkId::default()));
    assert_ne!(air, QueryResult::OutsideWorld);
}

#[test]
fn manifest_catalogue_covers_every_intersecting_chunk_and_no_extra_chunks() {
    let package = world(WorldHex::new(-1, 15), 19);
    let mut missing = package.manifest.clone();
    missing.chunks.pop().expect("nonempty catalogue");
    missing.fingerprint = fingerprint(&missing).expect("fresh checksum");
    assert!(missing.validate().is_err());
    let mut empty = package.manifest.clone();
    empty.chunks.clear();
    empty.fingerprint = fingerprint(&empty).expect("fresh checksum");
    assert!(empty.validate().is_err());
    let mut extra = package.manifest.clone();
    extra.chunks.push(ChunkDescriptor {
        coordinate: ChunkId { q: 999, r: 999 },
        fingerprint: 12,
        path: "chunks/outside.ron".into(),
    });
    extra.fingerprint = fingerprint(&extra).expect("fresh checksum");
    assert!(extra.validate().is_err());
}

#[test]
fn chunk_rectangle_intersection_matches_independent_fine_hex_disk_oracle() {
    for origin in [
        WorldHex::new(0, 0),
        WorldHex::new(-17, 15),
        WorldHex::new((1_i64 << 54) + 15, -31),
    ] {
        for radius in [0, 1, 15, 16, 31] {
            // world() builds descriptor keys by enumerating exact fine columns;
            // production validation instead intersects integer chunk rectangles.
            let package = world(origin, radius);
            package
                .manifest
                .validate()
                .expect("independent disk catalogue");
        }
    }
    let mut overlapping = world(WorldHex::new(2, 2), 2).manifest;
    let mut duplicate_footprint = overlapping.regions.first().expect("region").clone();
    duplicate_footprint.id = "region-overlap".into();
    overlapping.regions.push(duplicate_footprint);
    overlapping
        .seal()
        .expect("overlapping footprints use the union, not summed counts");
}

#[test]
fn huge_declared_region_with_tiny_catalogue_is_rejected_without_enumeration() {
    let mut manifest = world(WorldHex::new(0, 0), 0).manifest;
    manifest.regions.first_mut().expect("region").radius = u32::MAX;
    manifest.fingerprint = fingerprint(&manifest).expect("fresh checksum");
    let error = manifest
        .validate()
        .expect_err("tiny catalogue cannot cover huge disk");
    assert_eq!(error.context, "chunks");
    assert!(error.message.contains("cannot cover"));
}

#[test]
fn manifest_index_footprint_matches_independent_fine_disk_membership() {
    let origin = WorldHex::new(-17, 15);
    let package = world(origin, 19);
    let shared = std::sync::Arc::new(package.manifest);
    let index = ManifestIndex::new(shared.clone()).expect("validated spatial index");
    assert!(std::ptr::eq(index.manifest(), shared.as_ref()));
    for q in -25_i64..=25 {
        for r in -25_i64..=25 {
            let position = origin
                .checked_add(WorldHex::new(q, r))
                .expect("small offset");
            let expected = q.abs().max(r.abs()).max((q + r).abs()) <= 19;
            assert_eq!(
                index.contains(position).expect("indexed membership"),
                expected
            );
        }
    }
    assert_eq!(
        index.candidate_region_count(WorldHex::new(1000, 1000).chunk()),
        0
    );
    assert!(index
        .contains(WorldHex::new(i64::MAX, i64::MAX))
        .is_ok_and(|inside| !inside));
}

#[test]
fn dormant_catalogue_growth_does_not_expand_active_chunk_candidate_work() {
    let mut package = world(WorldHex::new(0, 0), 1);
    let active = WorldHex::new(0, 0).chunk();
    let live_feature = FeatureSummary {
        id: "live-feature".into(),
        region_id: "region".into(),
        kind: "landmark".into(),
        anchor: VoxelPosition {
            column: WorldHex::new(0, 0),
            level: 0,
        },
        asset: None,
    };
    package.manifest.features.push(live_feature.clone());
    package
        .chunks
        .get_mut(&active)
        .expect("active chunk")
        .features
        .push(live_feature);
    package.seal().expect("live feature");
    for number in 1..=10_000 {
        let origin = WorldHex::new(i64::from(number) * 1000, 0);
        let region_id = format!("dormant-{number:04}");
        package.manifest.regions.push(RegionDescriptor {
            id: region_id.clone(),
            origin,
            radius: 0,
            source_fingerprint: 1,
        });
        package.manifest.chunks.push(ChunkDescriptor {
            coordinate: origin.chunk(),
            fingerprint: 1,
            path: format!("chunks/dormant-{number}.ron"),
        });
        package.manifest.features.push(FeatureSummary {
            id: format!("dormant-feature-{number:04}"),
            region_id,
            kind: "landmark".into(),
            anchor: VoxelPosition {
                column: origin,
                level: 0,
            },
            asset: None,
        });
        package.manifest.materials.push(MaterialSpec {
            id: format!("dormant-material-{number:04}"),
            solid: false,
            diggable: false,
            color: [1, 2, 3, 255],
        });
    }
    package
        .manifest
        .seal()
        .expect("complete distant metadata catalogue");
    let index =
        ManifestIndex::new(std::sync::Arc::new(package.manifest.clone())).expect("expanded index");
    assert_eq!(index.candidate_region_count(active), 1);
    assert_eq!(index.manifest().regions.len(), 10_001);
    assert!(index.feature("dormant-feature-10000").is_some());
    assert!(index.material("dormant-material-10000").is_ok());
    assert!(index.region("dormant-10000").is_ok());
    let chunk = package.chunks.get(&active).expect("active package");
    chunk
        .validate_with_index(&index)
        .expect("unchanged active footprint");
    let mut missing = chunk.clone();
    missing.columns.pop().expect("one required column");
    missing.seal().expect("locally canonical missing footprint");
    assert!(missing.validate_with_index(&index).is_err());
    let mut forged = chunk.clone();
    forged.features.first_mut().expect("live feature").kind = "forged".into();
    forged.seal().expect("locally canonical forged feature");
    assert!(forged.validate_with_index(&index).is_err());
    let mut revised = chunk.clone();
    revised
        .columns
        .last_mut()
        .expect("column")
        .runs
        .push(run(3, 4, "stone"));
    revised.seal().expect("revised payload");
    assert_ne!(revised.fingerprint, chunk.fingerprint);
    revised
        .validate_with_index(&index)
        .expect("edited revisions need no immutable base hash match");
}

#[test]
fn manifest_index_rejects_untrusted_hash_and_unindexed_empty_chunk() {
    let package = world(WorldHex::new(0, 0), 1);
    let mut corrupt = package.manifest.clone();
    corrupt.fingerprint ^= 1;
    assert!(ManifestIndex::new(std::sync::Arc::new(corrupt)).is_err());
    let index = ManifestIndex::new(std::sync::Arc::new(package.manifest)).expect("index");
    let mut empty = ChunkPackage {
        schema_version: SCHEMA_VERSION,
        world_id: "world".into(),
        coordinate: ChunkId { q: 100, r: 100 },
        source_fingerprint: 12,
        columns: Vec::new(),
        features: Vec::new(),
        semantics: ChunkSemantics::default(),
        fingerprint: 0,
    };
    empty.seal().expect("local empty chunk shape");
    assert!(empty.validate_with_index(&index).is_err());
}

fn light(at: WorldHex, radius: u32) -> WorldLight {
    WorldLight {
        id: "local-lamp".into(),
        position: VoxelPosition {
            column: at,
            level: 1,
        },
        domain: None,
        bright_radius: radius / 2,
        dim_radius: radius,
    }
}
fn seal_hashes_only(package: &mut WorldPackage) {
    for chunk in package.chunks.values_mut() {
        chunk.seal().expect("local shape");
    }
    for descriptor in &mut package.manifest.chunks {
        descriptor.fingerprint = package
            .chunks
            .get(&descriptor.coordinate)
            .expect("chunk")
            .fingerprint;
    }
    package.manifest.seal().expect("manifest hashes");
}

#[test]
fn light_influence_projection_matches_independent_exact_horizontal_footprint() {
    for origin in [
        WorldHex::new(-1, -1),
        WorldHex::new(1_000_000_000_015, -1_000_000_000_001),
    ] {
        let mut package = world(origin, 20);
        let root = light(origin, 17);
        package
            .chunks
            .get_mut(&origin.chunk())
            .expect("owner")
            .semantics
            .lights
            .push(root.clone());
        package.seal().expect("complete light projection");
        let mut remote_count = 0;
        for (coordinate, chunk) in &package.chunks {
            let expected = chunk.columns.iter().any(|column| {
                let q = i128::from(column.position.q) - i128::from(origin.q);
                let r = i128::from(column.position.r) - i128::from(origin.r);
                q.abs().max(r.abs()).max((q + r).abs()) <= 17
            });
            assert_eq!(
                chunk.semantics.light_influences,
                if expected {
                    vec![root.clone()]
                } else {
                    Vec::new()
                }
            );
            if *coordinate != origin.chunk() && expected {
                remote_count += 1;
                assert!(
                    chunk.semantics.lights.is_empty(),
                    "projection does not create another root owner"
                );
            }
        }
        assert!(remote_count > 0);
        assert_eq!(
            package
                .chunks
                .values()
                .map(|chunk| chunk.semantics.lights.len())
                .sum::<usize>(),
            1
        );
        let index =
            ManifestIndex::new(std::sync::Arc::new(package.manifest.clone())).expect("index");
        let remote = package
            .chunks
            .iter()
            .find(|(coordinate, chunk)| {
                **coordinate != origin.chunk() && !chunk.semantics.light_influences.is_empty()
            })
            .map(|(_, chunk)| chunk.clone())
            .expect("remote product");
        drop(package);
        remote
            .validate_with_index(&index)
            .expect("local light admission with no owner chunk body");
    }
}

#[test]
fn missing_forged_or_duplicate_light_influences_are_rejected_at_their_trust_boundary() {
    let origin = WorldHex::new(15, 0);
    let remote = WorldHex::new(16, 0).chunk();
    let mut package = world(origin, 3);
    package
        .chunks
        .get_mut(&origin.chunk())
        .expect("owner")
        .semantics
        .lights
        .push(light(origin, 3));
    package.seal().expect("source");
    package
        .chunks
        .get_mut(&remote)
        .expect("recipient")
        .semantics
        .light_influences
        .clear();
    seal_hashes_only(&mut package);
    assert!(
        package.validate().is_err(),
        "whole-package validation proves complete influence coverage"
    );
    package.seal().expect("producer repairs missing projection");
    package
        .chunks
        .get_mut(&remote)
        .expect("recipient")
        .semantics
        .light_influences
        .first_mut()
        .expect("influence")
        .bright_radius = 0;
    seal_hashes_only(&mut package);
    let index =
        ManifestIndex::new(std::sync::Arc::new(package.manifest.clone())).expect("manifest index");
    package
        .chunks
        .get(&remote)
        .expect("recipient")
        .validate_with_index(&index)
        .expect("local shape cannot authenticate a foreign root by itself");
    assert!(
        package.validate().is_err(),
        "root registry catches forged foreign light despite recomputed local hashes"
    );
    package.seal().expect("producer repairs forged record");
    let owner = package.chunks.get_mut(&origin.chunk()).expect("owner");
    owner
        .semantics
        .light_influences
        .first_mut()
        .expect("root influence")
        .bright_radius = 0;
    owner.seal().expect("well shaped");
    assert!(
        owner.validate_with_index(&index).is_err(),
        "local root must match exactly"
    );
    package.seal().expect("producer repairs owner record");
    let recipient = package.chunks.get_mut(&remote).expect("recipient");
    let duplicate = recipient
        .semantics
        .light_influences
        .first()
        .expect("influence")
        .clone();
    recipient.semantics.light_influences.push(duplicate);
    assert!(recipient.seal().is_err());
}

#[test]
fn malformed_light_radius_fails_before_expansion_and_leaves_source_unchanged() {
    let origin = WorldHex::new(-1_000_000_000_001, 1_000_000_000_015);
    let mut package = world(origin, 0);
    let huge = light(origin, u32::MAX);
    let error = huge.validate().expect_err("bounded per-light expansion");
    assert_eq!(error.context, "light.influence");
    package
        .chunks
        .get_mut(&origin.chunk())
        .expect("owner")
        .semantics
        .lights
        .push(huge);
    let before = package.clone();
    assert!(package.seal().is_err());
    assert_eq!(package, before, "producer failure is atomic");
    let mut inverted = light(origin, 1);
    inverted.bright_radius = 2;
    assert!(inverted.validate().is_err());
}

#[test]
fn zero_radius_light_is_clipped_to_its_exact_column_and_outside_sources_are_rejected() {
    let origin = WorldHex::new(15, -1);
    let mut package = world(origin, 1);
    package
        .chunks
        .get_mut(&origin.chunk())
        .expect("owner")
        .semantics
        .lights
        .push(light(origin, 0));
    package.seal().expect("point influence");
    assert_eq!(
        package
            .chunks
            .values()
            .filter(|chunk| !chunk.semantics.light_influences.is_empty())
            .count(),
        1
    );
    let index = ManifestIndex::new(std::sync::Arc::new(package.manifest.clone())).expect("index");
    let mut owner = package.chunks.get(&origin.chunk()).expect("owner").clone();
    owner.semantics.light_influences = vec![light(WorldHex::new(999, 999), 0)];
    owner.seal().expect("bounded shape");
    assert!(owner.validate_with_index(&index).is_err());
}

fn spanning_object(id: &str) -> ObjectInstance {
    ObjectInstance {
        id: id.into(),
        region_id: "region".into(),
        asset: "tree".into(),
        origin: VoxelPosition {
            column: WorldHex::new(15, 0),
            level: 1,
        },
        rotation: 0,
        occupancy: vec![
            ColumnData {
                position: WorldHex::new(15, 0),
                runs: vec![run(1, 4, "stone")],
            },
            ColumnData {
                position: WorldHex::new(16, 0),
                runs: vec![run(2, 5, "stone")],
            },
        ],
    }
}

#[test]
fn identity_projection_is_clipped_and_preserves_overlapping_contributors() {
    let a = spanning_object("a");
    let b = spanning_object("b");
    for (coordinate, influence) in a.influences().expect("single-pass projection") {
        assert_eq!(
            a.influence(coordinate).expect("single clip"),
            Some(influence)
        );
    }
    let coordinate = WorldHex::new(16, 0).chunk();
    let a = a
        .influence(coordinate)
        .expect("projection")
        .expect("foreign contribution");
    let b = b
        .influence(coordinate)
        .expect("projection")
        .expect("foreign contribution");
    assert_eq!(a.occupancy.len(), 1);
    assert_eq!(a.occupancy, b.occupancy);
    assert_ne!(a.source_fingerprint, b.source_fingerprint);
    assert_eq!(
        union_object_occupancy(&[a.clone(), b.clone()]).expect("union"),
        a.occupancy
    );
    assert_eq!(
        union_object_occupancy(std::slice::from_ref(&b)).expect("survivor"),
        a.occupancy
    );
    let mut conflicting = b;
    conflicting
        .occupancy
        .first_mut()
        .expect("column")
        .runs
        .first_mut()
        .expect("run")
        .material = "water".into();
    assert!(union_object_occupancy(&[a, conflicting]).is_err());
}

#[test]
fn source_seal_rejects_reserved_identity_but_runtime_chunk_admission_accepts_it() {
    let mut package = world(WorldHex::new(15, 0), 1);
    let object = spanning_object(&runtime_object_id("add-tree", 0).expect("allocated"));
    package
        .chunks
        .get_mut(&object.origin.column.chunk())
        .expect("owner")
        .semantics
        .objects
        .push(object.clone());
    assert!(package.seal().is_err());
    for coordinate in object.dependency_chunks().expect("dependencies") {
        let chunk = package.chunks.get_mut(&coordinate).expect("chunk");
        chunk.semantics.object_influences = vec![object
            .influence(coordinate)
            .expect("projection")
            .expect("member")];
        chunk.semantics.occupancy =
            union_object_occupancy(&chunk.semantics.object_influences).expect("union");
        chunk.seal().expect("runtime package");
        chunk
            .validate_against_manifest(&package.manifest)
            .expect("runtime admission permits allocated identity");
    }
}

#[test]
fn clipped_object_forgery_fails_local_or_complete_projection_validation() {
    let mut package = world(WorldHex::new(15, 0), 1);
    let object = spanning_object("tree");
    package
        .chunks
        .get_mut(&object.origin.column.chunk())
        .expect("owner")
        .semantics
        .objects
        .push(object.clone());
    package.seal().expect("source");
    let foreign = WorldHex::new(16, 0).chunk();
    let chunk = package.chunks.get_mut(&foreign).expect("foreign");
    chunk
        .semantics
        .object_influences
        .first_mut()
        .expect("identity")
        .source_fingerprint ^= 1;
    chunk
        .seal()
        .expect("foreign full hash is checked at world boundary");
    package
        .manifest
        .chunks
        .iter_mut()
        .find(|row| row.coordinate == foreign)
        .expect("descriptor")
        .fingerprint = chunk.fingerprint;
    package.manifest.seal().expect("manifest");
    assert!(package.validate().is_err());
    let root = package
        .chunks
        .get_mut(&object.origin.column.chunk())
        .expect("root");
    root.semantics
        .object_influences
        .first_mut()
        .expect("identity")
        .source_fingerprint ^= 1;
    root.seal().expect("structural seal");
    assert!(root.validate_against_manifest(&package.manifest).is_err());
}

#[test]
fn object_transactions_require_canonical_exact_dependencies_and_allocated_new_ids() {
    let object = spanning_object("tree");
    let mut transaction = WorldObjectEditTransaction {
        id: "remove".into(),
        expected_revisions: object
            .dependency_chunks()
            .expect("dependencies")
            .into_iter()
            .map(|chunk| (chunk, 3))
            .collect(),
        edits: vec![ObjectEdit {
            before: Some(object.clone()),
            after: None,
        }],
    };
    transaction.validate().expect("removal");
    let saved = transaction.expected_revisions.clone();
    transaction.expected_revisions.pop_last();
    assert!(transaction.validate().is_err());
    transaction.expected_revisions = saved;
    transaction
        .edits
        .push(transaction.edits.first().expect("edit").clone());
    assert!(transaction.validate().is_err());
    transaction.edits = vec![ObjectEdit {
        before: None,
        after: Some(object),
    }];
    assert!(transaction.validate().is_err());
    transaction
        .edits
        .first_mut()
        .expect("edit")
        .after
        .as_mut()
        .expect("after")
        .id = runtime_object_id("remove", 0).expect("id");
    transaction.validate().expect("allocated addition");
    let wire = ron::ser::to_string(&transaction).expect("wire");
    let decoded: WorldObjectEditTransaction = ron::from_str(&wire).expect("shape");
    decoded.validate().expect("command");
    assert_eq!(decoded, transaction);
    transaction
        .edits
        .first_mut()
        .expect("edit")
        .after
        .as_mut()
        .expect("after")
        .id = runtime_object_id("other-command", 0).expect("id");
    assert!(transaction.validate().is_err());
}
