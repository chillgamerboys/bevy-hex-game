#![expect(
    clippy::indexing_slicing,
    reason = "Tests use exact independent fixture assertions and explicit failure points."
)]
use super::*;
use hex_world_contracts::*;
use std::{collections::BTreeMap, fs, path::PathBuf};

fn fixture(name: &str) -> WorldSpec {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/config/v4")
        .join(format!("{name}.ron"));
    parse_world(&fs::read_to_string(path).expect("runtime-loaded authored fixture"))
        .expect("valid source")
}
fn column(package: &WorldPackage, p: WorldHex) -> &ColumnData {
    package
        .chunks
        .get(&p.chunk())
        .expect("fixture invariant")
        .columns
        .iter()
        .find(|column| column.position == p)
        .expect("fixture invariant")
}
fn run(bottom: i32, top: i32, material: &str) -> VoxelRun {
    VoxelRun {
        bottom,
        top,
        material: material.into(),
    }
}

#[test]
fn interval_cuts_preserve_lower_and_upper_stacks() {
    let mut runs = vec![run(0, 1, "bedrock"), run(1, 20, "rock")];
    volume::replace(&mut runs, 5, 11, None).expect("fixture invariant");
    volume::insert(&mut runs, run(7, 9, "water")).expect("fixture invariant");
    assert_eq!(
        runs,
        vec![
            run(0, 1, "bedrock"),
            run(1, 5, "rock"),
            run(7, 9, "water"),
            run(11, 20, "rock")
        ]
    );
    assert_eq!(volume::material_at(&runs, 5), None);
    assert_eq!(volume::clear_above(&runs, 4), Some(2));
    assert!(volume::replace(&mut runs, 0, 1, None).is_err());
    assert!(volume::insert(&mut runs, run(8, 12, "deck")).is_err());
}

#[test]
fn integer_path_and_corridor_guarantees_are_coordinate_independent() {
    for end in [
        WorldHex::new(15, -8),
        WorldHex::new(-19, 10),
        WorldHex::new(0, 22),
    ] {
        let line = geometry::line(WorldHex::new(0, 0), end).expect("fixture invariant");
        assert_eq!(
            line.len() as u64,
            geometry::distance(WorldHex::new(0, 0), end).expect("fixture invariant") + 1
        );
        assert!(line
            .windows(2)
            .all(|pair| geometry::distance(pair[0], pair[1]).expect("fixture invariant") == 1));
        let mask = geometry::ribbon(&line, 2).expect("fixture invariant");
        let grade = geometry::grade(
            &mask,
            VoxelPosition {
                column: WorldHex::new(0, 0),
                level: 30,
            },
            VoxelPosition {
                column: end,
                level: 39,
            },
        )
        .expect("fixture invariant");
        assert_eq!(grade.get(&end), Some(&39));
        for (p, h) in &grade {
            for n in geometry::neighbors(*p) {
                if let Some(nh) = grade.get(&n) {
                    assert!(h.abs_diff(*nh) <= 1);
                }
            }
        }
    }
}

#[test]
fn rich_full_radius_region_has_independent_volume_and_metadata_witnesses() {
    let compiled = compile_world_cached(&fixture("rich-region"), None).expect("fixture invariant");
    assert_eq!(compiled.report.columns, 105_469);
    let package = &compiled.package;
    let lake = column(package, WorldHex::new(60, 30));
    assert_eq!(lake.material_at(30), Some("sand"));
    assert_eq!(lake.material_at(31), Some("water"));
    assert_eq!(lake.material_at(34), Some("water"));
    assert_eq!(lake.material_at(35), None);
    let cave = column(package, WorldHex::new(-65, 35));
    assert_eq!(cave.material_at(30), Some("limestone"));
    assert_eq!(cave.material_at(31), None);
    assert_eq!(cave.material_at(35), None);
    assert_eq!(cave.material_at(36), Some("limestone"));
    let bridge = column(package, WorldHex::new(51, -25));
    assert_eq!(bridge.material_at(47), Some("timber"));
    assert_eq!(bridge.material_at(48), Some("timber"));
    assert_eq!(bridge.material_at(46), None);
    assert!(compiled.report.liquid_columns > 1_300);
    assert!(compiled.report.interior_columns > 300);
    assert!(compiled.report.objects > 200);
    assert!(package.chunks.values().any(|chunk| chunk
        .semantics
        .liquids
        .iter()
        .any(|liquid| liquid.kind == LiquidKind::Waterfall)));
    assert!(package
        .chunks
        .values()
        .any(|chunk| !chunk.semantics.lights.is_empty()));
    assert!(package
        .chunks
        .values()
        .any(|chunk| !chunk.semantics.occupancy.is_empty()));
    assert!(package
        .chunks
        .values()
        .flat_map(|chunk| &chunk.columns)
        .flat_map(|column| &column.runs)
        .any(|run| run.top > 140));
    package.validate().expect("fixture invariant");
}

#[test]
fn source_validation_and_hard_override_conflicts_are_explicit() {
    let mut source = fixture("rich-region");
    source.regions[0].radius = 186;
    // A legitimate smaller source remains legitimate: there is no Grand singleton guard.
    assert!(validate_source(&source).is_ok());
    source.regions[0].rotation = 6;
    assert!(validate_source(&source).is_err());
    source.regions[0].rotation = 0;
    source
        .recipes
        .get_mut("caldera")
        .expect("fixture invariant")
        .overrides
        .push(OverrideSpec {
            id: "keep-lake-dry".into(),
            mask: DiskMask {
                center: WorldHex::new(60, 30),
                radius: 0,
            },
            surface_level: Some(45),
            material: None,
        });
    let error = compile_world(&source).unwrap_err().to_string();
    assert!(
        error.contains("keep-lake-dry")
            && error.contains("blue-reservoir")
            && error.contains("basin"),
        "{error}"
    );
}

#[test]
fn two_regions_seal_shared_walking_and_water_seams() {
    let source = fixture("two-regions");
    let compiled = compile_world_cached(&source, None).expect("fixture invariant");
    assert_eq!(compiled.report.columns, 210_938);
    assert_eq!(compiled.package.manifest.boundaries.len(), 1);
    let boundary = &compiled.package.manifest.boundaries[0];
    assert!(boundary.samples.len() > 300);
    assert!(boundary
        .samples
        .iter()
        .any(|sample| sample.water_level == Some(34)));
    assert!(boundary.samples.iter().any(|sample| sample.required_access));
    let region0 = &compiled.package.manifest.regions[0];
    let region1 = &compiled.package.manifest.regions[1];
    assert!(compiled
        .package
        .chunks
        .values()
        .any(|chunk| chunk.columns.iter().any(|column| region0
            .contains(column.position)
            .expect("fixture invariant"))
            && chunk.columns.iter().any(|column| region1
                .contains(column.position)
                .expect("fixture invariant"))));
    compiled.package.validate().expect("fixture invariant");
}

#[test]
fn feature_edit_reuses_geometry_and_only_affected_region_package_inputs() {
    let source = fixture("two-regions");
    let first = compile_world_cached(&source, None).expect("fixture invariant");
    let mut edit = source.clone();
    let mut recipe = edit
        .recipes
        .get("caldera")
        .expect("fixture invariant")
        .clone();
    recipe.features[0].density += 7;
    edit.recipes.insert("edited".into(), recipe);
    edit.regions[0].recipe = "edited".into();
    let cached = compile_world_cached(&edit, Some(&first)).expect("fixture invariant");
    let clean = compile_world(&edit).expect("fixture invariant");
    assert_eq!(cached.package, clean);
    assert_eq!(cached.report.regions_reused, 1);
    // Selecting a different recipe key intentionally changes the placement input;
    // editing the same recipe's feature stage alone is separately measured below.
    let mut same = source.clone();
    same.recipes
        .get_mut("caldera")
        .expect("fixture invariant")
        .features[0]
        .density += 7;
    let cached_same = compile_world_cached(&same, Some(&first)).expect("fixture invariant");
    assert!(cached_same
        .report
        .stages
        .iter()
        .filter(|stage| stage.stage == "geometry")
        .all(|stage| stage.reused));
    let remote = &clean.manifest.regions[1];
    let unchanged = first
        .package
        .chunks
        .iter()
        .filter(|(_, chunk)| {
            chunk
                .columns
                .iter()
                .all(|column| remote.contains(column.position).expect("fixture invariant"))
        })
        .collect::<BTreeMap<_, _>>();
    assert!(!unchanged.is_empty());
    for (coordinate, chunk) in unchanged {
        assert_eq!(
            chunk.fingerprint,
            clean
                .chunks
                .get(coordinate)
                .expect("fixture invariant")
                .fingerprint
        );
    }
}

#[test]
fn seven_complete_regions_remain_exact_and_source_order_independent() {
    let source = fixture("seven-regions");
    let first = compile_world_cached(&source, None).expect("fixture invariant");
    assert_eq!(first.report.columns, 738_283);
    assert_eq!(first.package.manifest.regions.len(), 7);
    assert_eq!(first.package.manifest.boundaries.len(), 12);
    let two = fixture("two-regions");
    assert_eq!(&source.regions[..2], two.regions.as_slice());
    assert_eq!(source.recipes.get("caldera"), two.recipes.get("caldera"));
    assert_eq!(source.recipes.len(), 4);
    let later_recipes = source
        .regions
        .iter()
        .skip(2)
        .map(|region| region.recipe.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(later_recipes.len(), 3);
    let mut reordered = source;
    reordered.regions.reverse();
    reordered.connections.reverse();
    reordered.materials.reverse();
    let second = compile_world_cached(&reordered, Some(&first)).expect("fixture invariant");
    assert_eq!(second.report.regions_reused, 7);
    assert_eq!(first.package, second.package);
    drop(second);
    // Compilation is currently serial. This proves input-order determinism across
    // independent cold compiles; it does not claim worker-count determinism.
    let clean_reordered = compile_world(&reordered).expect("cold reordered compile");
    assert_eq!(first.package, clean_reordered);
    drop(clean_reordered);
    let mut seam_edit = reordered;
    seam_edit
        .connections
        .first_mut()
        .expect("one seam")
        .water
        .as_mut()
        .expect("water crossing")
        .half_width += 1;
    let edited = compile_world_cached(&seam_edit, Some(&first)).expect("local boundary recompile");
    assert_eq!(
        edited.report.regions_reused, 5,
        "one shared seam invalidates exactly its two region dependencies"
    );
    let clean_seam_edit = compile_world(&seam_edit).expect("cold edited seam compile");
    assert_eq!(edited.package, clean_seam_edit);
}

#[test]
fn exported_stock_prefabs_match_real_catalog_blueprint_voxels() {
    #[derive(serde::Deserialize)]
    struct SourceVoxel {
        q: i64,
        r: i64,
        level: i32,
    }
    #[derive(serde::Deserialize)]
    struct SourcePlacement {
        position: SourceVoxel,
        style: String,
    }
    #[derive(serde::Deserialize)]
    struct SourceBlueprint {
        id: String,
        origin: SourceVoxel,
        placements: Vec<SourcePlacement>,
    }
    #[derive(serde::Deserialize)]
    struct SourceCatalog {
        objects: Vec<String>,
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let catalog: SourceCatalog = ron::from_str(
        &fs::read_to_string(root.join("assets/art/object_catalog.ron")).expect("fixture invariant"),
    )
    .expect("fixture invariant");
    for name in ["rich-region", "two-regions", "seven-regions"] {
        let source = fixture(name);
        let mut exports = 0;
        let mut assets = std::collections::BTreeSet::new();
        for recipe in source.recipes.values() {
            for feature in &recipe.features {
                let Some(provenance) = &feature.provenance else {
                    assert!(feature.asset.starts_with("procedural/"));
                    continue;
                };
                assert!(catalog.objects.contains(&feature.asset));
                assert_eq!(
                    provenance.source_revision,
                    "bc06a8969532b807ec677928eee304bc28399386"
                );
                let original: SourceBlueprint = ron::from_str(
                    &fs::read_to_string(root.join(&provenance.source_path))
                        .expect("fixture invariant"),
                )
                .expect("fixture invariant");
                assert_eq!(original.id, feature.asset);
                let expected: BTreeMap<_, _> = original
                    .placements
                    .into_iter()
                    .map(|placement| {
                        (
                            (
                                placement.position.q - original.origin.q,
                                placement.position.r - original.origin.r,
                                placement.position.level - original.origin.level,
                            ),
                            provenance
                                .style_materials
                                .get(&placement.style)
                                .expect("fixture invariant")
                                .clone(),
                        )
                    })
                    .collect();
                let actual: BTreeMap<_, _> = feature
                    .voxels
                    .iter()
                    .flat_map(|voxel| {
                        (voxel.bottom..voxel.top).map(move |level| {
                            (
                                (voxel.offset.q, voxel.offset.r, level),
                                voxel.material.clone(),
                            )
                        })
                    })
                    .collect();
                assert_eq!(actual, expected);
                exports += 1;
                assets.insert(feature.asset.clone());
            }
        }
        assert!(exports >= 3, "{name} must retain its stock export corpus");
        assert_eq!(
            assets.len(),
            3,
            "all stock blueprint types must be checked for {name}"
        );
    }
}

#[test]
fn declared_directed_current_reaches_exact_opposite_region_endpoint() {
    let source = fixture("two-regions");
    let package = compile_world(&source).expect("fixture invariant");
    let water = source.connections[0]
        .water
        .as_ref()
        .expect("fixture invariant");
    let flow = water.flow.as_ref().expect("fixture invariant");
    let liquids: BTreeMap<_, _> = package
        .chunks
        .values()
        .flat_map(|chunk| &chunk.semantics.liquids)
        .map(|liquid| {
            (
                VoxelPosition {
                    column: liquid.column,
                    level: liquid.top - 1,
                },
                liquid,
            )
        })
        .collect();
    let mut p = VoxelPosition {
        column: flow.upstream,
        level: water.level,
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut crossed = false;
    loop {
        assert!(seen.insert(p));
        let liquid = liquids.get(&p).expect("fixture invariant");
        let Some(next) = liquid.downstream.first() else {
            break;
        };
        let first = &package.manifest.regions[0];
        crossed |= first.contains(p.column).expect("fixture invariant")
            != first.contains(next.column).expect("fixture invariant");
        p = *next;
    }
    assert_eq!(
        p,
        VoxelPosition {
            column: flow.downstream,
            level: water.level
        }
    );
    assert!(crossed);
}

#[test]
fn malformed_runtime_source_never_silently_repairs_duplicate_or_overflowing_input() {
    let source = fixture("rich-region");
    let recipe = ron::ser::to_string(&source.recipes["caldera"]).expect("recipe serialization");
    let serialized = ron::ser::to_string(&source).expect("source serialization");
    let one = format!("recipes:{{\"caldera\":{recipe}}}");
    let duplicate = format!("recipes:{{\"caldera\":{recipe},\"caldera\":{recipe}}}");
    assert!(serialized.contains(&one));
    let error = parse_world(&serialized.replace(&one, &duplicate))
        .expect_err("duplicate map must fail")
        .to_string();
    assert!(error.contains("duplicate authoring map key"));
    let mut huge = source.clone();
    huge.recipes.get_mut("caldera").expect("recipe").biomes[0].mask = DiskMask {
        center: WorldHex::new(i64::MAX, i64::MAX),
        radius: u32::MAX,
    };
    assert!(validate_source(&huge).is_err());
    let mut overlap = source;
    let feature = &mut overlap.recipes.get_mut("caldera").expect("recipe").features[0];
    feature.voxels.push(feature.voxels[0].clone());
    assert!(validate_source(&overlap)
        .expect_err("overlapping prefab")
        .to_string()
        .contains("invalid prefab prototype"));
    let mut seams = fixture("two-regions");
    seams.connections.clear();
    assert!(validate_source(&seams)
        .expect_err("touching regions need seam authority")
        .to_string()
        .contains("shared connection"));
}

#[test]
fn adding_an_upper_vault_preserves_an_existing_lower_cave() {
    let source = fixture("rich-region");
    let mut recipe = source.recipes["caldera"].clone();
    recipe.landforms.clear();
    recipe.biomes.clear();
    recipe.overrides.clear();
    let region = RegionSpec {
        id: "stacked".into(),
        recipe: "caldera".into(),
        origin: WorldHex::new(0, 0),
        radius: 5,
        rotation: 0,
    };
    let mut build = operators::base(&region, &recipe, 0).expect("small terrain");
    let mut cave = CaveSpec {
        id: "lower".into(),
        rooms: vec![DiskMask {
            center: WorldHex::new(0, 0),
            radius: 2,
        }],
        path: vec![WorldHex::new(0, 0), WorldHex::new(1, 0)],
        half_width: 0,
        floor_level: 10,
        clearance: 4,
        roof_thickness: 3,
        material: "limestone".into(),
        entrances: vec![],
        light_spacing: 0,
    };
    operators::cave(&mut build, &recipe, "stacked", &cave).expect("lower cave");
    cave.id = "upper".into();
    cave.floor_level = 50;
    operators::cave(&mut build, &recipe, "stacked", &cave).expect("upper vault");
    let runs = &build.columns[&WorldHex::new(0, 0)];
    for (level, material) in [
        (10, Some("limestone")),
        (11, None),
        (14, None),
        (15, Some("limestone")),
        (50, Some("limestone")),
        (51, None),
        (54, None),
        (55, Some("limestone")),
    ] {
        assert_eq!(
            volume::material_at(runs, level),
            material,
            "exact stacked level {level}"
        );
    }
}

#[test]
fn later_crossing_route_cannot_bury_an_earlier_protected_ribbon() {
    let source = fixture("rich-region");
    let mut recipe = source.recipes["caldera"].clone();
    recipe.landforms.clear();
    recipe.biomes.clear();
    recipe.overrides.clear();
    let region = RegionSpec {
        id: "crossing".into(),
        recipe: "caldera".into(),
        origin: WorldHex::new(0, 0),
        radius: 6,
        rotation: 0,
    };
    let mut build = operators::base(&region, &recipe, 0).expect("flat terrain");
    let road = RouteSpec {
        id: "lower-road".into(),
        points: vec![
            GradePoint {
                column: WorldHex::new(-4, 0),
                level: 40,
            },
            GradePoint {
                column: WorldHex::new(4, 0),
                level: 40,
            },
        ],
        half_width: 0,
        shoulder_width: 0,
        material: "gravel".into(),
    };
    operators::route(&mut build, &recipe, &road).expect("first road");
    let crossing = RouteSpec {
        id: "raised-crossing".into(),
        points: vec![
            GradePoint {
                column: WorldHex::new(0, -4),
                level: 50,
            },
            GradePoint {
                column: WorldHex::new(0, 4),
                level: 50,
            },
        ],
        half_width: 0,
        shoulder_width: 0,
        material: "gravel".into(),
    };
    operators::route(&mut build, &recipe, &crossing)
        .expect("second road applies before constraint verification");
    // Both earlier endpoint surfaces still exist. The interior intersection is
    // nevertheless no longer the authored walking ribbon and must fail.
    for pin in &road.points {
        assert_eq!(operators::terrain(&build, pin.column).expect("pin").0, 40);
    }
    let error = operators::check_constraints(&build, &recipe, &crossing.id)
        .expect_err("buried route interior");
    assert!(
        error.contains("lower-road")
            && error.contains("raised-crossing")
            && error.contains("headroom")
    );
}

fn low_roof_steps() -> (WorldSpec, RegionRecipe, operators::RegionBuild) {
    let source = fixture("rich-region");
    let mut recipe = source.recipes["caldera"].clone();
    recipe.routes.clear();
    recipe.bridges.clear();
    recipe.overrides.clear();
    recipe.hub = GradePoint {
        column: WorldHex::new(0, 0),
        level: 10,
    };
    let mut build = operators::RegionBuild::default();
    // Both floors have exactly two air voxels above them. Only one level of
    // those air volumes overlaps laterally across the one-level step.
    build.columns.insert(
        WorldHex::new(0, 0),
        vec![run(0, 11, "limestone"), run(13, 14, "limestone")],
    );
    build.columns.insert(
        WorldHex::new(1, 0),
        vec![run(0, 12, "limestone"), run(14, 15, "limestone")],
    );
    (source, recipe, build)
}

#[test]
fn access_rejects_individually_standable_steps_under_a_low_lintel() {
    let (source, mut recipe, mut build) = low_roof_steps();
    for reverse in [false, true] {
        let (start, goal) = if reverse {
            ((WorldHex::new(1, 0), 11), (WorldHex::new(0, 0), 10))
        } else {
            ((WorldHex::new(0, 0), 10), (WorldHex::new(1, 0), 11))
        };
        recipe.hub = GradePoint {
            column: start.0,
            level: start.1,
        };
        build.semantics.anchors = vec![WorldAnchor {
            id: "required-low-roof-exit".into(),
            region_id: "steps".into(),
            position: VoxelPosition {
                column: goal.0,
                level: goal.1,
            },
            role: AnchorRole::Transit,
        }];
        let error = operators::validate_access(&build, &recipe, &source.materials)
            .expect_err("one-level aperture blocks a two-level walker in both directions");
        assert!(error.contains("required-low-roof-exit"), "{error}");
    }
    // One additional clear voxel above the lower floor opens exactly the
    // required two-level aperture. Nothing else in the topology changes.
    build.columns.insert(
        WorldHex::new(0, 0),
        vec![run(0, 11, "limestone"), run(14, 15, "limestone")],
    );
    operators::validate_access(&build, &recipe, &source.materials)
        .expect("raising the lower lintel restores the ordinary route");
}

#[test]
fn later_low_lintel_reports_the_invalidating_operator_and_protected_route() {
    let (_, recipe, mut build) = low_roof_steps();
    build.routes.insert(
        "required-stairway".into(),
        BTreeMap::from([(WorldHex::new(0, 0), 10), (WorldHex::new(1, 0), 11)]),
    );
    let error = operators::check_constraints(&build, &recipe, "new-vault-lintel")
        .expect_err("per-cell headroom alone does not preserve a graded route");
    assert!(
        error.contains("new-vault-lintel")
            && error.contains("required-stairway")
            && error.contains("lateral aperture"),
        "{error}"
    );
    build.columns.insert(
        WorldHex::new(0, 0),
        vec![run(0, 11, "limestone"), run(14, 15, "limestone")],
    );
    operators::check_constraints(&build, &recipe, "raised-vault-lintel")
        .expect("exactly two shared clear levels preserve the protected route");
}
