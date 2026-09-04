#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
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
        .unwrap()
        .columns
        .iter()
        .find(|column| column.position == p)
        .unwrap()
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
    volume::replace(&mut runs, 5, 11, None).unwrap();
    volume::insert(&mut runs, run(7, 9, "water")).unwrap();
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
        let line = geometry::line(WorldHex::new(0, 0), end).unwrap();
        assert_eq!(
            line.len() as u64,
            geometry::distance(WorldHex::new(0, 0), end).unwrap() + 1
        );
        assert!(line
            .windows(2)
            .all(|pair| geometry::distance(pair[0], pair[1]).unwrap() == 1));
        let mask = geometry::ribbon(&line, 2).unwrap();
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
        .unwrap();
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
    let compiled = compile_world_cached(&fixture("rich-region"), None).unwrap();
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
    package.validate().unwrap();
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
        .unwrap()
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
    let compiled = compile_world_cached(&source, None).unwrap();
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
    assert!(compiled.package.chunks.values().any(|chunk| chunk
        .columns
        .iter()
        .any(|column| region0.contains(column.position).unwrap())
        && chunk
            .columns
            .iter()
            .any(|column| region1.contains(column.position).unwrap())));
    compiled.package.validate().unwrap();
}

#[test]
fn feature_edit_reuses_geometry_and_only_affected_region_package_inputs() {
    let source = fixture("two-regions");
    let first = compile_world_cached(&source, None).unwrap();
    let mut edit = source.clone();
    let mut recipe = edit.recipes.get("caldera").unwrap().clone();
    recipe.features[0].density += 7;
    edit.recipes.insert("edited".into(), recipe);
    edit.regions[0].recipe = "edited".into();
    let cached = compile_world_cached(&edit, Some(&first)).unwrap();
    let clean = compile_world(&edit).unwrap();
    assert_eq!(cached.package, clean);
    assert_eq!(cached.report.regions_reused, 1);
    // Selecting a different recipe key intentionally changes the placement input;
    // editing the same recipe's feature stage alone is separately measured below.
    let mut same = source.clone();
    same.recipes.get_mut("caldera").unwrap().features[0].density += 7;
    let cached_same = compile_world_cached(&same, Some(&first)).unwrap();
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
                .all(|column| remote.contains(column.position).unwrap())
        })
        .collect::<BTreeMap<_, _>>();
    assert!(!unchanged.is_empty());
    for (coordinate, chunk) in unchanged {
        assert_eq!(
            chunk.fingerprint,
            clean.chunks.get(coordinate).unwrap().fingerprint
        );
    }
}

#[test]
fn seven_complete_regions_remain_exact_and_source_order_independent() {
    let source = fixture("seven-regions");
    let first = compile_world_cached(&source, None).unwrap();
    assert_eq!(first.report.columns, 738_283);
    assert_eq!(first.package.manifest.regions.len(), 7);
    assert_eq!(first.package.manifest.boundaries.len(), 12);
    let mut reordered = source;
    reordered.regions.reverse();
    reordered.connections.reverse();
    reordered.materials.reverse();
    let second = compile_world_cached(&reordered, Some(&first)).unwrap();
    assert_eq!(second.report.regions_reused, 7);
    assert_eq!(first.package, second.package);
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
    let source = fixture("rich-region");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let catalog: SourceCatalog =
        ron::from_str(&fs::read_to_string(root.join("assets/art/object_catalog.ron")).unwrap())
            .unwrap();
    let mut exports = 0;
    for feature in &source.recipes.get("caldera").unwrap().features {
        let Some(provenance) = &feature.provenance else {
            assert!(feature.asset.starts_with("procedural/"));
            continue;
        };
        assert!(catalog.objects.contains(&feature.asset));
        assert_eq!(
            provenance.source_revision,
            "bc06a8969532b807ec677928eee304bc28399386"
        );
        let original: SourceBlueprint =
            ron::from_str(&fs::read_to_string(root.join(&provenance.source_path)).unwrap())
                .unwrap();
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
                        .unwrap()
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
    }
    assert_eq!(exports, 3);
}

#[test]
fn declared_directed_current_reaches_exact_opposite_region_endpoint() {
    let source = fixture("two-regions");
    let package = compile_world(&source).unwrap();
    let water = source.connections[0].water.as_ref().unwrap();
    let flow = water.flow.as_ref().unwrap();
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
        let liquid = liquids.get(&p).unwrap();
        let Some(next) = liquid.downstream.first() else {
            break;
        };
        let first = &package.manifest.regions[0];
        crossed |= first.contains(p.column).unwrap() != first.contains(next.column).unwrap();
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
