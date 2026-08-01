use super::*;

#[test]
fn procedural_setup_publishes_validated_resources_and_exact_anchors() {
    let mut app = procedural_app();
    enter_gameplay(&mut app);

    assert!(app.world().contains_resource::<TerrainReady>());
    let report = app.world().resource::<GenerationReport>();
    assert_eq!(report.seed, 20_260_726);
    assert_eq!(report.candidates_evaluated, 8);
    assert!(!report.used_fallback, "{:?}", report.notes);

    let anchors = app.world().resource::<MapAnchors>();
    for name in [
        "party_start",
        "hostile_start",
        "conflict_center",
        "bridge",
        "alternate_crossing",
    ] {
        assert!(
            anchors.get(&MapAnchorId::from(name)).is_some(),
            "missing generated anchor {name}"
        );
    }
    assert_eq!(app.world().resource::<VoxelMap>().len(), 469);
    assert!(
        app.world().resource::<SpecialMovementRegions>().is_empty(),
        "the hills recipe does not introduce optional regions yet"
    );
    assert!(app.world().resource::<InteriorRegions>().is_empty());
}

#[test]
fn v3_frozen_hills_publishes_ice_without_an_optional_movement_region() {
    let mut app = v3_frozen_hills_app();
    enter_gameplay(&mut app);

    assert!(
        app.world().contains_resource::<TerrainReady>(),
        "setup failed: {:?}",
        app.world()
            .get_resource::<GameplaySetupFailure>()
            .map(|failure| failure.reason.as_str())
    );
    assert!(!app.world().contains_resource::<GameplaySetupFailure>());
    assert!(
        app.world().resource::<SpecialMovementRegions>().is_empty(),
        "Frozen Hills ice is presentation geometry, not optional traversal authority"
    );
    let ice = app
        .world()
        .resource::<SubstanceTable>()
        .id("ice")
        .expect("Frozen Hills fixture should register ice");
    assert!(
        app.world()
            .resource::<VoxelMap>()
            .columns()
            .any(|(_coord, column)| column.iter().any(|substance| substance == ice)),
        "Frozen Hills dropped its visible shoreline ice caps"
    );
}

#[test]
fn v3_waterfall_publishes_exact_resources_and_report_identity() {
    let mut app = v3_waterfall_app();
    enter_gameplay(&mut app);

    assert!(
        app.world().contains_resource::<TerrainReady>(),
        "setup failed: {:?}",
        app.world()
            .get_resource::<GameplaySetupFailure>()
            .map(|failure| failure.reason.as_str())
    );
    assert!(!app.world().contains_resource::<GameplaySetupFailure>());
    let report = app.world().resource::<GenerationReport>();
    assert_eq!(report.generator_version, 3);
    assert_eq!(report.seed, 771_203_419);
    assert_eq!(report.candidates_evaluated, 8);
    assert_eq!(report.valid_candidates, 8);
    assert_eq!(report.selected_candidate, Some(4));
    assert!(!report.used_fallback);
    assert_eq!(report.repair_rounds, 0);
    assert!(report.repair_actions.is_empty());
    assert!(report.notes.is_empty());
    assert_eq!(report.settings_fingerprint, 5_082_310_489_405_017_929);
    assert_eq!(
        report.semantic_plan_fingerprint,
        Some(2_940_332_537_625_721_792)
    );
    assert_eq!(report.map_fingerprint, 18_345_439_249_093_579_610);
    assert_ne!(
        report.semantic_plan_fingerprint,
        Some(report.map_fingerprint),
        "semantic and materialized identities use independent domains"
    );
    assert_eq!(report.metrics.relief, 13);
    assert_eq!(report.metrics.critical_route_steps, 11);
    let Some(ProceduralRecipeMetrics::Waterfall(metrics)) = &report.recipe_metrics else {
        panic!("V3 Waterfall should publish exact recipe metrics");
    };
    assert_eq!(metrics.fall_height, 13);
    assert_eq!(metrics.fall_nodes, 3);
    assert_eq!(metrics.bypass_steps, 11);
    assert_eq!(metrics.alternate_bypass_steps, 13);
    assert_eq!(metrics.raised_terrain, 95);
    assert_eq!(report.metrics.alternate_detour_percent, 18);
    assert_eq!(metrics.water_nodes, report.metrics.barrier_cells);
    assert_eq!(metrics.ordinary_surfaces, report.metrics.reachable_surfaces);

    let anchors = app.world().resource::<MapAnchors>();
    let party = anchors
        .get(&MapAnchorId::from("party_start"))
        .expect("Waterfall should publish party_start");
    let hostile = anchors
        .get(&MapAnchorId::from("hostile_start"))
        .expect("Waterfall should publish hostile_start");
    assert_eq!(party.level - hostile.level, 11);
    for review_anchor in ["fall_overlook", "basin_overlook"] {
        assert!(
            anchors.get(&MapAnchorId::from(review_anchor)).is_some(),
            "missing Waterfall review anchor {review_anchor}"
        );
    }

    assert_eq!(app.world().resource::<VoxelMap>().len(), 469);
    assert_eq!(app.world().resource::<SpecialMovementRegions>().len(), 6);
    assert!(app.world().resource::<InteriorRegions>().is_empty());
    let blockers = app
        .world()
        .resource::<TraversalBlockers>()
        .iter()
        .collect::<BTreeSet<_>>();
    let roots = feature_roots(&mut app);
    let tree_roots = roots
        .iter()
        .filter_map(|(_entity, kind, position, _parent)| {
            (kind == "GeneratedTree").then_some(*position)
        })
        .collect::<BTreeSet<_>>();
    let grass_roots = roots
        .iter()
        .filter_map(|(_entity, kind, position, _parent)| {
            (kind == "GeneratedTallGrass").then_some(*position)
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(tree_roots.len(), 3);
    assert!(tree_roots.is_subset(&blockers));
    assert_eq!(blockers.len(), 3);
    assert!(grass_roots.is_disjoint(&blockers));
    assert_eq!(app.world().resource::<BiomeRegions>().len(), 475);
    assert!(app.world().resource::<MapViewHint>().is_valid());
}

#[test]
fn v3_forest_publishes_exact_features_blockers_and_routes() {
    let mut app = v3_forest_app();
    enter_gameplay(&mut app);

    assert!(
        app.world().contains_resource::<TerrainReady>(),
        "setup failed: {:?}",
        app.world()
            .get_resource::<GameplaySetupFailure>()
            .map(|failure| failure.reason.as_str())
    );
    assert!(!app.world().contains_resource::<GameplaySetupFailure>());
    let report = app.world().resource::<GenerationReport>().clone();
    assert_eq!(report.generator_version, 3);
    assert_eq!(report.seed, 381_654_729);
    assert_eq!(report.candidates_evaluated, 8);
    assert_eq!(report.valid_candidates, 5);
    assert!(!report.used_fallback);
    assert_eq!(report.repair_rounds, 0);
    assert!(report.repair_actions.is_empty());
    assert_eq!(report.notes.len(), 3);
    assert!(report
        .notes
        .iter()
        .all(|note| note.starts_with("candidate ")));
    assert_eq!(report.selected_candidate, Some(6));
    assert_eq!(report.settings_fingerprint, 2_658_105_648_444_344_100);
    assert_eq!(
        report.semantic_plan_fingerprint,
        Some(3_116_162_104_822_374_845)
    );
    assert_eq!(report.map_fingerprint, 18_084_914_740_711_593_486);
    let Some(ProceduralRecipeMetrics::Forest(metrics)) = &report.recipe_metrics else {
        panic!("V3 Forest should publish exact recipe metrics");
    };
    assert_eq!(metrics.clearing_count, 4);
    assert_eq!(metrics.relief, 4);
    assert_eq!(metrics.tree_roots, 53);
    assert!(metrics.old_growth_roots > 0);
    assert_eq!(
        metrics.old_growth_blocker_surfaces,
        metrics.old_growth_roots.saturating_mul(7)
    );
    assert_eq!(
        metrics.tree_blocker_surfaces,
        metrics
            .tree_roots
            .saturating_add(metrics.old_growth_roots.saturating_mul(6))
    );
    assert_eq!(metrics.tall_grass_roots, 155);
    assert!(metrics.tall_grass_roots.saturating_mul(2) > metrics.prairie_surfaces);
    assert_eq!(metrics.ordinary_surfaces, report.metrics.reachable_surfaces);
    assert_eq!(
        metrics.critical_route_steps,
        report.metrics.critical_route_steps
    );
    assert_eq!(
        metrics.spawn_height_difference,
        report.metrics.spawn_height_difference
    );
    assert_eq!(
        report.metrics.barrier_cells, 0,
        "distributed tree blockers are not a semantic hazard barrier"
    );
    assert_eq!(
        report.metrics.bank_high_ground_difference, 0,
        "Forest has woodland and prairie sides, not opposing river banks"
    );
    assert!(
        metrics.woodland_prairie_high_ground_difference <= metrics.relief,
        "side-specific high ground cannot exceed total ordinary relief"
    );

    let blockers: BTreeSet<_> = app.world().resource::<TraversalBlockers>().iter().collect();
    let roots = feature_roots(&mut app);
    let tree_roots: BTreeSet<_> = roots
        .iter()
        .filter_map(|(_entity, kind, position, _parent)| {
            (kind == "GeneratedTree").then_some(*position)
        })
        .collect();
    let grass_roots: BTreeSet<_> = roots
        .iter()
        .filter_map(|(_entity, kind, position, _parent)| {
            (kind == "GeneratedTallGrass").then_some(*position)
        })
        .collect();
    assert!(tree_roots.is_subset(&blockers));
    assert_eq!(
        blockers.len(),
        usize::try_from(metrics.tree_blocker_surfaces).unwrap_or(usize::MAX)
    );
    assert!(grass_roots.is_disjoint(&blockers));
    let canopy_roots: BTreeSet<_> = {
        let world = app.world_mut();
        let mut canopies = world.query::<(
            &CanopyOccluder,
            Option<&PresentationOcclusion>,
            Option<&HexTile>,
        )>();
        canopies
            .iter(world)
            .map(|(canopy, occlusion, tile)| {
                assert!(tile.is_none(), "a tree canopy became terrain footing");
                assert!(
                    occlusion.is_none_or(|occlusion| !occlusion.is_hidden()),
                    "a freshly spawned canopy carried a stale cutaway reason"
                );
                canopy.0
            })
            .collect()
    };
    assert_eq!(canopy_roots, tree_roots);
    assert_eq!(
        tree_roots.len(),
        usize::try_from(metrics.tree_roots).unwrap_or(usize::MAX)
    );
    assert_eq!(
        grass_roots.len(),
        usize::try_from(metrics.tall_grass_roots).unwrap_or(usize::MAX)
    );
    assert_eq!(app.world().resource::<VoxelMap>().len(), 469);
    assert_eq!(app.world().resource::<BiomeRegions>().len(), 469);
    assert!(app.world().resource::<SpecialMovementRegions>().is_empty());
    assert!(app.world().resource::<InteriorRegions>().is_empty());
    assert!(app.world().resource::<MapViewHint>().is_valid());
    for anchor in [
        "party_start",
        "hostile_start",
        "forest_clearing",
        "prairie_overlook",
    ] {
        assert!(
            app.world()
                .resource::<MapAnchors>()
                .get(&MapAnchorId::from(anchor))
                .is_some(),
            "missing Forest anchor {anchor}"
        );
    }
}

#[test]
fn v3_deep_forest_publishes_dense_trees_and_reenters_deterministically() {
    let mut app = v3_deep_forest_app();
    enter_gameplay(&mut app);

    assert!(
        app.world().contains_resource::<TerrainReady>(),
        "setup failed: {:?}",
        app.world()
            .get_resource::<GameplaySetupFailure>()
            .map(|failure| failure.reason.as_str())
    );
    assert!(!app.world().contains_resource::<GameplaySetupFailure>());
    let first_report = app.world().resource::<GenerationReport>().clone();
    assert_eq!(first_report.generator_version, 3);
    assert_eq!(first_report.seed, 1_592_598_566);
    assert_eq!(first_report.candidates_evaluated, 8);
    assert_eq!(first_report.valid_candidates, 8);
    assert_eq!(first_report.selected_candidate, Some(0));
    assert!(!first_report.used_fallback);
    assert_eq!(first_report.repair_rounds, 0);
    assert!(first_report.repair_actions.is_empty());
    assert_eq!(first_report.settings_fingerprint, 6_246_604_390_469_913_222);
    assert_eq!(
        first_report.semantic_plan_fingerprint,
        Some(1_319_216_151_194_471_912)
    );
    assert_eq!(first_report.map_fingerprint, 15_168_627_475_653_117_104);
    let Some(ProceduralRecipeMetrics::DeepForest(metrics)) = &first_report.recipe_metrics else {
        panic!("V3 Deep Forest should publish exact recipe metrics");
    };
    let metrics = *metrics;
    assert_eq!(metrics.tree_roots, 105);
    assert_eq!(metrics.tree_blocker_surfaces, 117);
    assert_eq!(metrics.blocker_coverage_percent, 29);
    assert_eq!(metrics.clearing_count, 3);
    assert_eq!(metrics.clearing_surfaces, 30);
    assert_eq!(metrics.protected_trail_surfaces, 38);
    assert_eq!(metrics.ordinary_surfaces, 352);
    assert_eq!(metrics.reachable_elevation_levels, 5);
    assert_eq!(metrics.relief, 4);
    assert_eq!(metrics.critical_route_steps, 27);
    assert_eq!(
        metrics.tree_blocker_surfaces,
        u32::try_from(app.world().resource::<TraversalBlockers>().len()).unwrap_or(u32::MAX)
    );
    assert!(app.world().resource::<SpecialMovementRegions>().is_empty());
    assert!(app.world().resource::<InteriorRegions>().is_empty());
    for anchor in ["party_start", "hostile_start", "deep_forest_clearing"] {
        assert!(
            app.world()
                .resource::<MapAnchors>()
                .get(&MapAnchorId::from(anchor))
                .is_some(),
            "missing Deep Forest anchor {anchor}"
        );
    }

    let first_roots = feature_roots(&mut app);
    assert_eq!(
        first_roots.len(),
        usize::try_from(metrics.tree_roots).unwrap_or(usize::MAX)
    );
    assert!(first_roots
        .iter()
        .all(|(_entity, kind, _position, _parent)| kind == "GeneratedTree"));
    let first_root_positions = first_roots
        .iter()
        .map(|(_entity, _kind, position, _parent)| *position)
        .collect::<BTreeSet<_>>();
    let first_blockers = app
        .world()
        .resource::<TraversalBlockers>()
        .iter()
        .collect::<BTreeSet<_>>();
    let first_biomes = app
        .world()
        .resource::<BiomeRegions>()
        .iter()
        .collect::<BTreeMap<_, _>>();
    let first_tile_count = tile_count(&mut app);

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();

    assert_eq!(tile_count(&mut app), 0);
    assert!(feature_roots(&mut app).is_empty());
    for absent in [
        app.world().contains_resource::<VoxelMap>(),
        app.world().contains_resource::<MapAnchors>(),
        app.world().contains_resource::<SpecialMovementRegions>(),
        app.world().contains_resource::<InteriorRegions>(),
        app.world().contains_resource::<TraversalBlockers>(),
        app.world().contains_resource::<BiomeRegions>(),
        app.world().contains_resource::<MapViewHint>(),
        app.world().contains_resource::<GenerationReport>(),
        app.world().contains_resource::<TerrainReady>(),
    ] {
        assert!(!absent);
    }

    enter_gameplay(&mut app);
    let second_report = app.world().resource::<GenerationReport>();
    assert_eq!(second_report.seed, first_report.seed);
    assert_eq!(
        second_report.selected_candidate,
        first_report.selected_candidate
    );
    assert_eq!(
        second_report.settings_fingerprint,
        first_report.settings_fingerprint
    );
    assert_eq!(
        second_report.semantic_plan_fingerprint,
        first_report.semantic_plan_fingerprint
    );
    assert_eq!(second_report.map_fingerprint, first_report.map_fingerprint);
    assert_eq!(second_report.metrics, first_report.metrics);
    assert_eq!(second_report.recipe_metrics, first_report.recipe_metrics);
    assert_eq!(tile_count(&mut app), first_tile_count);
    let second_roots = feature_roots(&mut app)
        .iter()
        .map(|(_entity, _kind, position, _parent)| *position)
        .collect::<BTreeSet<_>>();
    assert_eq!(second_roots, first_root_positions);
    assert_eq!(
        app.world()
            .resource::<TraversalBlockers>()
            .iter()
            .collect::<BTreeSet<_>>(),
        first_blockers
    );
    assert_eq!(
        app.world()
            .resource::<BiomeRegions>()
            .iter()
            .collect::<BTreeMap<_, _>>(),
        first_biomes
    );
}

#[test]
fn v3_fort_publishes_worked_stone_structures_and_access_metrics() {
    let mut app = v3_fort_app();
    enter_gameplay(&mut app);

    assert!(
        app.world().contains_resource::<TerrainReady>(),
        "setup failed: {:?}",
        app.world()
            .get_resource::<GameplaySetupFailure>()
            .map(|failure| failure.reason.as_str())
    );
    assert!(!app.world().contains_resource::<GameplaySetupFailure>());
    let report = app.world().resource::<GenerationReport>();
    assert_eq!(report.generator_version, 3);
    assert_eq!(report.seed, 640_367_719);
    assert_eq!(report.candidates_evaluated, 8);
    assert!(report.valid_candidates > 0);
    assert!(!report.used_fallback);
    assert_eq!(report.repair_rounds, 0);
    assert!(report.repair_actions.is_empty());
    let Some(ProceduralRecipeMetrics::Fort(metrics)) = &report.recipe_metrics else {
        panic!("V3 Fort should publish exact recipe metrics");
    };
    assert_eq!(metrics.gate_count, 2);
    assert_eq!(metrics.stair_count, 2);
    assert_eq!(metrics.tower_count, 6);
    assert_eq!(metrics.battlement_columns, 22);
    assert_eq!(metrics.independent_gate_routes, 2);
    assert_eq!(metrics.curtain_height, 5);
    assert_eq!(metrics.keep_height, 8);
    assert_eq!(metrics.ordinary_surfaces, report.metrics.reachable_surfaces);
    assert_eq!(
        metrics.critical_route_steps,
        report.metrics.critical_route_steps
    );

    let anchors = app.world().resource::<MapAnchors>();
    for name in [
        "party_start",
        "hostile_start",
        "fort_courtyard",
        "fort_wall_walk",
        "fort_keep",
    ] {
        assert!(
            anchors.get(&MapAnchorId::from(name)).is_some(),
            "missing Fort anchor {name}"
        );
    }
    let worked_stone = app
        .world()
        .resource::<SubstanceTable>()
        .id("worked_stone")
        .expect("Fort fixture should register worked stone");
    assert!(app
        .world()
        .resource::<VoxelMap>()
        .columns()
        .any(|(_coord, column)| column.iter().any(|substance| substance == worked_stone)));
    assert!(app.world().resource::<TraversalBlockers>().is_empty());
    assert!(app.world().resource::<InteriorRegions>().is_empty());
    // Compact turrets release six alternating outer-ring cells back to the
    // non-ordinary battlement pattern: 22 battlements plus seven keep-roof cells.
    assert_eq!(app.world().resource::<SpecialMovementRegions>().len(), 29);
    assert!(app.world().resource::<MapViewHint>().is_valid());
}

#[test]
fn v3_forest_grass_does_not_block_edits_and_retires_with_its_support() {
    let mut app = v3_forest_app();
    enter_gameplay(&mut app);

    let roots = feature_roots(&mut app);
    let grass_root = roots
        .iter()
        .find_map(|(_entity, kind, position, _parent)| {
            (kind == "GeneratedTallGrass").then_some(*position)
        })
        .expect("Forest should publish presentation-only grass");
    let tree_roots_before = roots
        .iter()
        .filter(|(_entity, kind, _position, _parent)| kind == "GeneratedTree")
        .count();
    let grass_roots_before = roots.len().saturating_sub(tree_roots_before);

    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: grass_root });
    app.update();
    app.update();

    let rebuilt = feature_roots(&mut app);
    assert!(
        rebuilt
            .iter()
            .all(|(_entity, _kind, position, _parent)| *position != grass_root),
        "grass presentation survived after its exact authored support was cleared"
    );
    assert_eq!(
        rebuilt
            .iter()
            .filter(|(_entity, kind, _position, _parent)| kind == "GeneratedTree")
            .count(),
        tree_roots_before,
        "editing non-blocking grass changed the blocking tree plan"
    );
    assert_eq!(
        rebuilt
            .iter()
            .filter(|(_entity, kind, _position, _parent)| kind == "GeneratedTallGrass")
            .count(),
        grass_roots_before.saturating_sub(1)
    );
}

#[test]
fn v2_hills_setup_preserves_v1_map_identity_with_v2_report_identity() {
    let mut v1 = procedural_app();
    enter_gameplay(&mut v1);
    let v1_report = v1.world().resource::<GenerationReport>().clone();
    let v1_anchors: BTreeMap<String, TilePos> = v1
        .world()
        .resource::<MapAnchors>()
        .iter()
        .map(|(id, position)| (id.as_str().to_owned(), position))
        .collect();

    let mut v2 = v2_hills_app();
    enter_gameplay(&mut v2);

    assert!(v2.world().contains_resource::<TerrainReady>());
    assert!(!v2.world().contains_resource::<GameplaySetupFailure>());
    assert_eq!(v2.world().resource::<VoxelMap>().len(), 469);

    let report = v2.world().resource::<GenerationReport>();
    assert_eq!(report.generator_version, 2);
    assert_eq!(report.seed, 20_260_726);
    assert_eq!(report.candidates_evaluated, 8);
    assert_eq!(report.map_fingerprint, v1_report.map_fingerprint);
    assert_eq!(report.selected_candidate, v1_report.selected_candidate);
    assert_eq!(report.valid_candidates, v1_report.valid_candidates);
    assert_eq!(report.repair_actions, v1_report.repair_actions);
    assert_eq!(report.used_fallback, v1_report.used_fallback);
    assert_eq!(report.metrics, v1_report.metrics);
    assert_ne!(
        report.settings_fingerprint, v1_report.settings_fingerprint,
        "V2 output parity must not erase generator-version identity"
    );

    let anchors: BTreeMap<String, TilePos> = v2
        .world()
        .resource::<MapAnchors>()
        .iter()
        .map(|(id, position)| (id.as_str().to_owned(), position))
        .collect();
    assert_eq!(anchors, v1_anchors);
}

#[test]
fn v2_hills_publishes_geometry_derived_view_and_empty_interiors() {
    let mut app = v2_hills_app();
    enter_gameplay(&mut app);

    let view = *app.world().resource::<MapViewHint>();
    assert!(view.is_valid());
    assert_eq!(view, MapViewHint::new((0.0, 48.0, 42.0), (0.0, 6.0, 0.0)));
    assert!(
        app.world().resource::<InteriorRegions>().is_empty(),
        "Hills must publish explicit empty interior metadata"
    );
}

#[test]
fn sky_region_registry_contains_exact_generated_surfaces() {
    let mut app = sky_islands_app();
    enter_gameplay(&mut app);

    let expected: BTreeMap<TilePos, SpecialMovementRegion> = app
        .world()
        .resource::<SpecialMovementRegions>()
        .iter()
        .collect();
    assert!(!expected.is_empty());
    assert!(expected.keys().all(|position| !app
        .world()
        .resource::<VoxelMap>()
        .get(*position)
        .is_air()));
}

#[test]
fn v2_layered_sky_publishes_ground_anchors_upper_regions_and_combined_view() {
    let mut hills = v2_hills_app();
    enter_gameplay(&mut hills);
    let expected_anchors: BTreeMap<_, _> = hills
        .world()
        .resource::<MapAnchors>()
        .iter()
        .map(|(name, position)| (name.as_str().to_owned(), position))
        .collect();

    let mut app = v2_layered_sky_app();
    enter_gameplay(&mut app);

    assert!(app.world().contains_resource::<TerrainReady>());
    assert!(!app.world().contains_resource::<GameplaySetupFailure>());
    assert_eq!(app.world().resource::<VoxelMap>().len(), 469);
    assert!(app.world().resource::<MapViewHint>().is_valid());
    assert!(app.world().resource::<InteriorRegions>().is_empty());

    let report = app.world().resource::<GenerationReport>();
    assert_eq!(report.generator_version, 2);
    assert_eq!(report.candidates_evaluated, 8);
    let actual_anchors: BTreeMap<_, _> = app
        .world()
        .resource::<MapAnchors>()
        .iter()
        .map(|(name, position)| (name.as_str().to_owned(), position))
        .collect();
    assert_eq!(actual_anchors, expected_anchors);

    let regions = app.world().resource::<SpecialMovementRegions>();
    assert!(!regions.is_empty());
    assert!(regions.iter().all(|(position, _region)| !app
        .world()
        .resource::<VoxelMap>()
        .get(position)
        .is_air()));
}

#[test]
fn v2_mountains_publishes_route_anchors_and_geometry_derived_view() {
    let mut app = v2_mountains_app();
    enter_gameplay(&mut app);

    assert!(
        app.world().contains_resource::<TerrainReady>(),
        "setup failed: {:?}",
        app.world()
            .get_resource::<GameplaySetupFailure>()
            .map(|failure| failure.reason.as_str())
    );
    assert!(!app.world().contains_resource::<GameplaySetupFailure>());
    assert_eq!(app.world().resource::<VoxelMap>().len(), 469);
    assert!(app.world().resource::<MapViewHint>().is_valid());
    assert!(app.world().resource::<InteriorRegions>().is_empty());

    let report = app.world().resource::<GenerationReport>();
    assert_eq!(report.generator_version, 2);
    assert_eq!(report.candidates_evaluated, 8);
    assert!(!report.used_fallback, "{:?}", report.notes);

    let anchors = app.world().resource::<MapAnchors>();
    for name in [
        "party_start",
        "hostile_start",
        "conflict_center",
        "high_pass",
        "low_bypass",
    ] {
        assert!(
            anchors.get(&MapAnchorId::from(name)).is_some(),
            "missing Mountains anchor {name}"
        );
    }

    let map = app.world().resource::<VoxelMap>();
    assert!(
        app.world()
            .resource::<SpecialMovementRegions>()
            .iter()
            .all(|(position, _)| !map.get(position).is_air()),
        "a summit region named an air surface"
    );
}

#[test]
fn v3_caves_publish_exact_interiors_lights_anchors_and_cutaway_roofs() {
    let mut app = v3_caves_app();
    enter_gameplay(&mut app);

    assert!(
        app.world().contains_resource::<TerrainReady>(),
        "setup failed: {:?}",
        app.world()
            .get_resource::<GameplaySetupFailure>()
            .map(|failure| failure.reason.as_str())
    );
    assert!(!app.world().contains_resource::<GameplaySetupFailure>());
    assert_eq!(app.world().resource::<VoxelMap>().len(), 469);
    assert!(app.world().resource::<MapViewHint>().is_valid());
    assert!(
        app.world().resource::<SpecialMovementRegions>().is_empty(),
        "the critical cave network should remain ordinarily walkable"
    );

    let report = app.world().resource::<GenerationReport>();
    assert_eq!(report.generator_version, 3);
    assert_eq!(report.seed, 736_283_041);
    assert_eq!(report.candidates_evaluated, 8);
    assert!(!report.used_fallback, "{:?}", report.notes);
    let Some(ProceduralRecipeMetrics::Caves(CavesReportMetrics {
        chamber_count,
        gameplay_lights,
        moss_roots,
        lichen_roots,
        vegetation_visual_voxels,
        optional_dark_floors,
        minimum_roof_thickness,
        ..
    })) = report.recipe_metrics
    else {
        panic!("V3 Caves should publish exact recipe metrics");
    };
    assert_eq!(chamber_count, 12);
    assert_eq!(moss_roots, 2);
    assert_eq!(lichen_roots, 2);
    assert_eq!(vegetation_visual_voxels, 14);
    assert!(gameplay_lights > 0);
    assert!(optional_dark_floors > 0);
    assert!(minimum_roof_thickness >= 3);
    assert!(app.world().resource::<TraversalBlockers>().is_empty());
    let vegetation = cave_vegetation_instances(&mut app);
    assert_eq!(vegetation.len(), 4);
    assert_eq!(
        vegetation
            .values()
            .filter(|(object, _rotation)| object == "prop/cave-moss")
            .count(),
        2
    );
    assert_eq!(
        vegetation
            .values()
            .filter(|(object, _rotation)| object == "prop/cave-lichen")
            .count(),
        2
    );
    assert!(
        vegetation.values().all(|(_object, rotation)| *rotation < 6),
        "cave vegetation published an invalid authored rotation"
    );

    let anchors = app.world().resource::<MapAnchors>();
    for name in [
        "party_start",
        "hostile_start",
        "conflict_center",
        "cave_entrance",
        "deep_chamber",
    ] {
        assert!(
            anchors.get(&MapAnchorId::from(name)).is_some(),
            "missing Caves anchor {name}"
        );
    }

    let interiors = app.world().resource::<InteriorRegions>();
    let floors: Vec<_> = interiors.surfaces().collect();
    let roofs: Vec<_> = interiors.roof_voxels().collect();
    assert!(
        !floors.is_empty(),
        "Caves published no exact interior floors"
    );
    assert!(
        !roofs.is_empty(),
        "Caves published no exact cutaway roof voxels"
    );

    let map = app.world().resource::<VoxelMap>();
    let table = app.world().resource::<SubstanceTable>();
    assert!(floors.iter().all(|(position, _region)| {
        map.column(position.coord).is_some_and(|column| {
            table.is_solid(map.get(*position))
                && column.headroom_above(position.level.saturating_add(1)).0 >= 2
        })
    }));
    assert!(
        roofs
            .iter()
            .all(|(position, _region)| table.is_solid(map.get(*position))),
        "cutaway metadata named a non-solid roof voxel"
    );

    let projected_cutaways = {
        let world = app.world_mut();
        let mut cutaways =
            world.query_filtered::<Option<&PresentationOcclusion>, With<CutawayOccluder>>();
        cutaways
            .iter(world)
            .inspect(|occlusion| {
                assert!(
                    occlusion.is_some(),
                    "a projected cave roof cannot participate in composed cutaway"
                );
            })
            .count()
    };
    assert!(
        projected_cutaways > 0,
        "exact roof voxels did not project onto rendered runs"
    );
    let generated_lights = {
        let world = app.world_mut();
        let mut lights = world.query::<(&TilePos, &GameplayLight)>();
        lights
            .iter(world)
            .inspect(|(position, light)| {
                assert!((4..=7).contains(&light.radius));
                assert!(
                    floors.iter().any(|(floor, _region)| floor == *position),
                    "generated gameplay light is not rooted on an interior floor"
                );
            })
            .count()
    };
    assert_eq!(
        generated_lights,
        usize::try_from(gameplay_lights).unwrap_or(usize::MAX)
    );
}

#[test]
fn v3_waterfall_spawns_caps_and_a_non_shadowing_fall_curtain() {
    let mut app = v3_waterfall_app();
    enter_gameplay(&mut app);

    let world = app.world_mut();
    let mut query = world.query::<(
        &Name,
        &ChildOf,
        &Pickable,
        Option<&NotShadowCaster>,
        Option<&HexTile>,
    )>();
    let mut caps = 0;
    let mut curtains = 0;
    for (name, _parent, pickable, no_shadow, tile) in query.iter(world) {
        if !matches!(name.as_str(), "LiquidCap" | "LiquidFallCurtain") {
            continue;
        }
        assert_eq!(*pickable, Pickable::IGNORE);
        assert!(no_shadow.is_some());
        assert!(tile.is_none());
        match name.as_str() {
            "LiquidCap" => caps += 1,
            "LiquidFallCurtain" => curtains += 1,
            _ => unreachable!(),
        }
    }
    assert!(caps > 30, "every Waterfall liquid run should receive a cap");
    assert_eq!(
        curtains, 1,
        "the three adjacent fall lanes share one water curtain mesh"
    );
}

#[test]
fn v3_volcano_materializes_and_reenters_with_exact_lava_and_report_state() {
    let mut app = v3_volcano_app();
    enter_gameplay(&mut app);

    assert!(
        app.world().contains_resource::<TerrainReady>(),
        "setup failed: {:?}",
        app.world()
            .get_resource::<GameplaySetupFailure>()
            .map(|failure| failure.reason.as_str())
    );
    assert!(!app.world().contains_resource::<GameplaySetupFailure>());
    let first_report = app.world().resource::<GenerationReport>().clone();
    let Some(ProceduralRecipeMetrics::Volcano(metrics)) = first_report.recipe_metrics.as_ref()
    else {
        panic!("V3 Volcano should publish exact recipe metrics");
    };
    assert_eq!(metrics.summit_relief, 20);
    assert!((20..=30).contains(&metrics.massif_coverage_percent));
    assert!(metrics.fall_nodes >= 3);
    assert!(metrics.maximum_fall_height >= 2);
    assert_eq!(metrics.bridge_surfaces, 6);
    assert!(metrics.bridge_clearance >= 4);
    assert!(!first_report.used_fallback, "{:?}", first_report.notes);

    let first_anchors: BTreeMap<String, TilePos> = app
        .world()
        .resource::<MapAnchors>()
        .iter()
        .map(|(id, position)| (id.as_str().to_owned(), position))
        .collect();
    for required in [
        "party_start",
        "hostile_start",
        "conflict_center",
        "bridge",
        "crater_overlook",
    ] {
        assert!(first_anchors.contains_key(required), "missing {required}");
    }
    let first_tile_count = tile_count(&mut app);
    let first_presentations = liquid_presentations(&mut app);
    let first_caps = first_presentations
        .iter()
        .filter(|(entity, _, _)| {
            app.world()
                .get::<Name>(*entity)
                .is_some_and(|name| name.as_str() == "LiquidCap")
        })
        .count();
    let first_curtains = first_presentations.len().saturating_sub(first_caps);
    assert!(first_caps >= metrics.lava_nodes as usize);
    assert_eq!(
        first_curtains, 1,
        "all adjacent lava falls should share one curtain mesh"
    );

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();
    assert_eq!(tile_count(&mut app), 0);
    assert!(liquid_presentations(&mut app).is_empty());
    assert!(!app.world().contains_resource::<GenerationReport>());
    assert!(!app.world().contains_resource::<TerrainReady>());

    enter_gameplay(&mut app);
    assert_eq!(tile_count(&mut app), first_tile_count);
    assert_eq!(
        liquid_presentations(&mut app).len(),
        first_presentations.len()
    );
    let second_report = app.world().resource::<GenerationReport>();
    assert_eq!(
        second_report.settings_fingerprint,
        first_report.settings_fingerprint
    );
    assert_eq!(
        second_report.semantic_plan_fingerprint,
        first_report.semantic_plan_fingerprint
    );
    assert_eq!(second_report.map_fingerprint, first_report.map_fingerprint);
    assert_eq!(second_report.recipe_metrics, first_report.recipe_metrics);
    assert_eq!(
        app.world()
            .resource::<MapAnchors>()
            .iter()
            .map(|(id, position)| (id.as_str().to_owned(), position))
            .collect::<BTreeMap<_, _>>(),
        first_anchors
    );
}

#[test]
fn sky_regions_reenter_with_the_same_exact_memberships() {
    let mut app = sky_islands_app();
    enter_gameplay(&mut app);
    let first: BTreeMap<TilePos, SpecialMovementRegion> = app
        .world()
        .resource::<SpecialMovementRegions>()
        .iter()
        .collect();
    assert!(!first.is_empty());

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();
    assert!(!app.world().contains_resource::<SpecialMovementRegions>());

    enter_gameplay(&mut app);
    let second: BTreeMap<TilePos, SpecialMovementRegion> = app
        .world()
        .resource::<SpecialMovementRegions>()
        .iter()
        .collect();

    assert_eq!(second, first);
}
