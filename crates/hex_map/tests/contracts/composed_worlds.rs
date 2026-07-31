use super::*;

#[test]
fn ring7_recipe_metrics_are_public_reflected_and_exhaustive() {
    let recipe_metrics = ProceduralRecipeMetrics::Ring7(Ring7Metrics {
        ordinary_surfaces: 1,
        reachable_surfaces: 2,
        reachable_elevation_levels: 3,
        relief: 4,
        critical_route_steps: 5,
        macro_edges: 6,
        redundant_regions: 7,
        directed_liquid_seams: 8,
        liquid_cells: 9,
        feature_instances: 10,
        structures: 11,
        gameplay_lights: 12,
        interiors: 13,
    });
    let ProceduralRecipeMetrics::Ring7(Ring7Metrics {
        ordinary_surfaces,
        reachable_surfaces,
        reachable_elevation_levels,
        relief,
        critical_route_steps,
        macro_edges,
        redundant_regions,
        directed_liquid_seams,
        liquid_cells,
        feature_instances,
        structures,
        gameplay_lights,
        interiors,
    }) = recipe_metrics
    else {
        panic!("the Ring7 report must retain its exact aggregate metrics");
    };
    assert_eq!(liquid_cells, 9);
    assert_eq!(
        (
            ordinary_surfaces,
            reachable_surfaces,
            reachable_elevation_levels,
            relief,
            critical_route_steps,
            macro_edges,
            redundant_regions,
            directed_liquid_seams,
            feature_instances,
            structures,
            gameplay_lights,
            interiors,
        ),
        (1, 2, 3, 4, 5, 6, 7, 8, 10, 11, 12, 13)
    );

    let app = test_app();
    let registry = app.world().resource::<AppTypeRegistry>().read();
    for type_id in [
        TypeId::of::<ProceduralRecipeMetrics>(),
        TypeId::of::<Ring7Metrics>(),
    ] {
        assert!(
            registry.get(type_id).is_some(),
            "Ring7 report vocabulary is missing reflection registration"
        );
    }
}

#[test]
fn ring19_recipe_metrics_are_public_reflected_and_exhaustive() {
    let recipe_metrics = ProceduralRecipeMetrics::Ring19(Ring19Metrics {
        world_columns: 1,
        biome_regions: 2,
        reciprocal_seams: 3,
        boundary_sides: 4,
        ordinary_surfaces: 5,
        reachable_surfaces: 6,
        reachable_elevation_levels: 7,
        relief: 8,
        critical_route_steps: 9,
        macro_edges: 10,
        redundant_regions: 11,
        directed_liquid_seams: 12,
        boundary_liquid_outlets: 13,
        liquid_cells: 14,
        feature_instances: 15,
        structures: 16,
        gameplay_lights: 17,
        interiors: 18,
    });
    let ProceduralRecipeMetrics::Ring19(Ring19Metrics {
        world_columns,
        biome_regions,
        reciprocal_seams,
        boundary_sides,
        ordinary_surfaces,
        reachable_surfaces,
        reachable_elevation_levels,
        relief,
        critical_route_steps,
        macro_edges,
        redundant_regions,
        directed_liquid_seams,
        boundary_liquid_outlets,
        liquid_cells,
        feature_instances,
        structures,
        gameplay_lights,
        interiors,
    }) = recipe_metrics
    else {
        panic!("the Ring19 report must retain its exact aggregate metrics");
    };
    assert_eq!(
        (
            world_columns,
            biome_regions,
            reciprocal_seams,
            boundary_sides,
            ordinary_surfaces,
            reachable_surfaces,
            reachable_elevation_levels,
            relief,
            critical_route_steps,
            macro_edges,
            redundant_regions,
            directed_liquid_seams,
        ),
        (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12)
    );
    assert_eq!(
        (
            boundary_liquid_outlets,
            liquid_cells,
            feature_instances,
            structures,
            gameplay_lights,
            interiors,
        ),
        (13, 14, 15, 16, 17, 18)
    );

    let app = test_app();
    let registry = app.world().resource::<AppTypeRegistry>().read();
    for type_id in [
        TypeId::of::<ProceduralRecipeMetrics>(),
        TypeId::of::<Ring19Metrics>(),
    ] {
        assert!(
            registry.get(type_id).is_some(),
            "Ring19 report vocabulary is missing reflection registration"
        );
    }
}

#[test]
fn v3_ring7_materializes_complete_world_and_reenters_deterministically() {
    const RADIUS_33_COLUMNS: usize = 1 + 3 * 33 * 34;

    let mut app = v3_ring7_app();
    enter_gameplay(&mut app);

    assert!(
        app.world().contains_resource::<TerrainReady>(),
        "setup failed: {:?}",
        app.world()
            .get_resource::<GameplaySetupFailure>()
            .map(|failure| failure.reason.as_str())
    );
    assert!(!app.world().contains_resource::<GameplaySetupFailure>());
    assert_eq!(app.world().resource::<ResolvedMapSeed>().0, 703_700_113);
    assert_eq!(
        app.world().resource::<VoxelMap>().len(),
        RADIUS_33_COLUMNS,
        "Ring7 must materialize every horizontal column in its fixed radius-33 world"
    );
    let first_tile_count = tile_count(&mut app);
    assert!(
        first_tile_count > RADIUS_33_COLUMNS,
        "stacked caves and sky islands should materialize more footing than a flat radius-33 map"
    );

    let report = app.world().resource::<GenerationReport>().clone();
    assert_eq!(report.generator_version, 3);
    assert_eq!(report.seed, 703_700_113);
    assert_eq!(report.selected_candidate, Some(4));
    assert_eq!(report.candidates_evaluated, 8);
    assert_eq!(report.valid_candidates, 3);
    assert!(!report.used_fallback, "{:?}", report.notes);
    assert_eq!(report.repair_rounds, 0);
    assert!(report.repair_actions.is_empty());
    assert_eq!(
        report.settings_fingerprint, 11_463_780_561_406_126_783,
        "update only with an explicit shipped Ring7 settings-identity decision"
    );
    assert_eq!(
        report.semantic_plan_fingerprint,
        Some(17_137_489_855_939_949_303),
        "update only with an explicit shipped Ring7 semantic-plan decision"
    );
    assert_eq!(
        report.map_fingerprint, 14_774_674_416_521_441_907,
        "update only with an explicit shipped Ring7 materialized-map decision"
    );

    let Some(ProceduralRecipeMetrics::Ring7(metrics)) = report.recipe_metrics.as_ref() else {
        panic!("V3 Ring7 should publish exact whole-world metrics");
    };
    assert_eq!(
        metrics,
        &Ring7Metrics {
            ordinary_surfaces: 2_880,
            reachable_surfaces: 2_880,
            reachable_elevation_levels: 23,
            relief: 22,
            critical_route_steps: 28,
            macro_edges: 12,
            redundant_regions: 7,
            directed_liquid_seams: 2,
            liquid_cells: 227,
            feature_instances: 382,
            structures: 17,
            gameplay_lights: 4,
            interiors: 1,
        },
        "update only with an explicit shipped Ring7 aggregate-contract decision"
    );
    assert!(metrics.liquid_cells > 0);
    assert!(metrics.feature_instances > 0);
    assert!(metrics.structures > 0);
    assert!(metrics.gameplay_lights > 0);
    assert_eq!(metrics.interiors, 1);
    assert_eq!(
        report.metrics.reachable_surfaces,
        metrics.reachable_surfaces
    );
    assert_eq!(report.metrics.barrier_cells, metrics.liquid_cells);

    let anchors = app.world().resource::<MapAnchors>();
    for name in [
        "party_start",
        "hostile_start",
        "conflict_center",
        "center_party_start",
        "center_hostile_start",
        "center_conflict_center",
    ] {
        assert!(
            anchors.get(&MapAnchorId::from(name)).is_some(),
            "missing composed Ring7 anchor {name}"
        );
    }
    let first_anchors: BTreeMap<String, TilePos> = anchors
        .iter()
        .map(|(id, position)| (id.as_str().to_owned(), position))
        .collect();
    assert_eq!(
        first_anchors.get("party_start"),
        first_anchors.get("center_party_start"),
        "the canonical party alias should resolve to the center Hills anchor"
    );
    assert_eq!(
        first_anchors.get("hostile_start"),
        first_anchors.get("center_hostile_start"),
        "the canonical hostile alias should resolve to the center Hills anchor"
    );

    let first_biomes: BTreeMap<TilePos, BiomeRegionId> =
        app.world().resource::<BiomeRegions>().iter().collect();
    let represented_regions = first_biomes
        .values()
        .copied()
        .collect::<BTreeSet<BiomeRegionId>>();
    assert_eq!(
        represented_regions,
        (0..7).map(BiomeRegionId).collect(),
        "Ring7 should publish exact memberships for all seven patches"
    );
    assert!(first_biomes.len() >= RADIUS_33_COLUMNS);
    assert!(app.world().resource::<MapViewHint>().is_valid());

    let first_view = *app.world().resource::<MapViewHint>();
    let first_interior_floors: BTreeMap<TilePos, InteriorRegionId> = app
        .world()
        .resource::<InteriorRegions>()
        .surfaces()
        .collect();
    let first_lights: BTreeMap<TilePos, GameplayLight> = {
        let world = app.world_mut();
        let mut lights = world.query::<(&TilePos, &GameplayLight)>();
        lights
            .iter(world)
            .map(|(position, light)| (*position, *light))
            .collect()
    };
    assert!(!first_interior_floors.is_empty());
    assert_eq!(
        first_lights.len(),
        usize::try_from(metrics.gameplay_lights).unwrap_or(usize::MAX)
    );
    assert_eq!(
        first_interior_floors
            .values()
            .copied()
            .collect::<BTreeSet<_>>()
            .len(),
        usize::try_from(metrics.interiors).unwrap_or(usize::MAX)
    );
    assert!(
        first_lights
            .keys()
            .all(|position| first_interior_floors.contains_key(position)),
        "every Ring7 gameplay light should be rooted on an exact cave floor"
    );

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();

    assert_eq!(tile_count(&mut app), 0);
    assert!(!app.world().contains_resource::<VoxelMap>());
    assert!(!app.world().contains_resource::<MapAnchors>());
    assert!(!app.world().contains_resource::<BiomeRegions>());
    assert!(!app.world().contains_resource::<InteriorRegions>());
    assert!(!app.world().contains_resource::<GenerationReport>());
    assert!(!app.world().contains_resource::<TerrainReady>());
    let light_count_after_exit = {
        let world = app.world_mut();
        let mut lights = world.query::<&GameplayLight>();
        lights.iter(world).count()
    };
    assert_eq!(light_count_after_exit, 0);

    enter_gameplay(&mut app);

    assert!(app.world().contains_resource::<TerrainReady>());
    assert!(!app.world().contains_resource::<GameplaySetupFailure>());
    assert_eq!(tile_count(&mut app), first_tile_count);
    let second_report = app.world().resource::<GenerationReport>();
    assert_eq!(second_report.seed, report.seed);
    assert_eq!(second_report.selected_candidate, report.selected_candidate);
    assert_eq!(second_report.valid_candidates, report.valid_candidates);
    assert_eq!(
        second_report.settings_fingerprint,
        report.settings_fingerprint
    );
    assert_eq!(
        second_report.semantic_plan_fingerprint,
        report.semantic_plan_fingerprint
    );
    assert_eq!(second_report.map_fingerprint, report.map_fingerprint);
    assert_eq!(second_report.metrics, report.metrics);
    assert_eq!(second_report.recipe_metrics, report.recipe_metrics);
    assert_eq!(*app.world().resource::<MapViewHint>(), first_view);
    let second_anchors: BTreeMap<String, TilePos> = app
        .world()
        .resource::<MapAnchors>()
        .iter()
        .map(|(id, position)| (id.as_str().to_owned(), position))
        .collect();
    let second_biomes: BTreeMap<TilePos, BiomeRegionId> =
        app.world().resource::<BiomeRegions>().iter().collect();
    let second_interior_floors: BTreeMap<TilePos, InteriorRegionId> = app
        .world()
        .resource::<InteriorRegions>()
        .surfaces()
        .collect();
    let second_lights: BTreeMap<TilePos, GameplayLight> = {
        let world = app.world_mut();
        let mut lights = world.query::<(&TilePos, &GameplayLight)>();
        lights
            .iter(world)
            .map(|(position, light)| (*position, *light))
            .collect()
    };
    assert_eq!(second_anchors, first_anchors);
    assert_eq!(second_biomes, first_biomes);
    assert_eq!(second_interior_floors, first_interior_floors);
    assert_eq!(second_lights, first_lights);
}

#[test]
fn v3_ring19_materializes_complete_world_and_reenters_deterministically() {
    const RADIUS_55_COLUMNS: usize = 1 + 3 * 55 * 56;

    let mut app = v3_ring19_app();
    enter_gameplay(&mut app);

    assert!(
        app.world().contains_resource::<TerrainReady>(),
        "setup failed: {:?}",
        app.world()
            .get_resource::<GameplaySetupFailure>()
            .map(|failure| failure.reason.as_str())
    );
    assert!(!app.world().contains_resource::<GameplaySetupFailure>());
    assert_eq!(app.world().resource::<ResolvedMapSeed>().0, 1_592_598_566);
    assert_eq!(
        app.world().resource::<VoxelMap>().len(),
        RADIUS_55_COLUMNS,
        "Ring19 must materialize every horizontal column in its fixed radius-55 world"
    );
    let first_tile_count = tile_count(&mut app);
    assert!(
        first_tile_count > RADIUS_55_COLUMNS,
        "Ring19 caves, structures, and sky islands should materialize stacked voxel runs"
    );

    let report = app.world().resource::<GenerationReport>().clone();
    assert_eq!(report.generator_version, 3);
    assert_eq!(report.seed, 1_592_598_566);
    assert_eq!(report.selected_candidate, Some(0));
    assert_eq!(report.candidates_evaluated, 8);
    assert_eq!(report.valid_candidates, 1);
    assert!(!report.used_fallback, "{:?}", report.notes);
    assert_eq!(report.repair_rounds, 0);
    assert!(report.repair_actions.is_empty());
    assert_eq!(
        report.settings_fingerprint, 2_347_243_186_379_186_390,
        "update only with an explicit shipped Ring19 settings-identity decision"
    );
    assert_eq!(
        report.semantic_plan_fingerprint,
        Some(3_259_497_139_560_498_268),
        "update only with an explicit shipped Ring19 semantic-plan decision"
    );
    assert_eq!(
        report.map_fingerprint, 13_502_613_458_185_406_509,
        "update only with an explicit shipped Ring19 materialized-map decision"
    );

    let Some(ProceduralRecipeMetrics::Ring19(metrics)) = report.recipe_metrics.as_ref() else {
        panic!("V3 Ring19 should publish exact whole-world metrics");
    };
    assert_eq!(
        metrics,
        &Ring19Metrics {
            world_columns: 9_241,
            biome_regions: 19,
            reciprocal_seams: 42,
            boundary_sides: 30,
            ordinary_surfaces: 7_222,
            reachable_surfaces: 7_146,
            reachable_elevation_levels: 26,
            relief: 25,
            critical_route_steps: 49,
            macro_edges: 42,
            redundant_regions: 19,
            directed_liquid_seams: 8,
            boundary_liquid_outlets: 2,
            liquid_cells: 744,
            feature_instances: 1_353,
            structures: 23,
            gameplay_lights: 5,
            interiors: 1,
        },
        "update only with an explicit shipped Ring19 aggregate-contract decision"
    );
    assert!(metrics.liquid_cells > 0);
    assert!(metrics.feature_instances > 0);
    assert!(metrics.structures > 0);
    assert!(metrics.gameplay_lights > 0);
    assert!(metrics.interiors > 0);
    assert_eq!(
        report.metrics.reachable_surfaces,
        metrics.reachable_surfaces
    );
    assert_eq!(report.metrics.barrier_cells, metrics.liquid_cells);

    let anchors = app.world().resource::<MapAnchors>();
    for name in ["party_start", "hostile_start", "conflict_center"] {
        assert!(
            anchors.get(&MapAnchorId::from(name)).is_some(),
            "missing canonical Ring19 anchor {name}"
        );
    }
    let first_anchors: BTreeMap<String, TilePos> = anchors
        .iter()
        .map(|(id, position)| (id.as_str().to_owned(), position))
        .collect();
    let first_biomes: BTreeMap<TilePos, BiomeRegionId> =
        app.world().resource::<BiomeRegions>().iter().collect();
    assert_eq!(
        first_biomes
            .values()
            .copied()
            .collect::<BTreeSet<BiomeRegionId>>(),
        (0..19).map(BiomeRegionId).collect(),
        "Ring19 should publish exact memberships for all nineteen patches"
    );
    assert!(first_biomes.len() >= RADIUS_55_COLUMNS);

    let first_blockers = app
        .world()
        .resource::<TraversalBlockers>()
        .iter()
        .collect::<BTreeSet<_>>();
    let first_special_regions: BTreeMap<TilePos, SpecialMovementRegion> = app
        .world()
        .resource::<SpecialMovementRegions>()
        .iter()
        .collect();
    let first_interior_floors: BTreeMap<TilePos, InteriorRegionId> = app
        .world()
        .resource::<InteriorRegions>()
        .surfaces()
        .collect();
    let first_interior_roofs: BTreeMap<TilePos, InteriorRegionId> = app
        .world()
        .resource::<InteriorRegions>()
        .roof_voxels()
        .collect();
    let first_lights: BTreeMap<TilePos, GameplayLight> = {
        let world = app.world_mut();
        let mut lights = world.query::<(&TilePos, &GameplayLight)>();
        lights
            .iter(world)
            .map(|(position, light)| (*position, *light))
            .collect()
    };
    let first_objects = object_instance_snapshot(&mut app);
    let first_view = *app.world().resource::<MapViewHint>();
    assert!(first_view.is_valid());
    assert!(!first_blockers.is_empty());
    assert!(!first_special_regions.is_empty());
    assert!(!first_interior_floors.is_empty());
    assert!(!first_interior_roofs.is_empty());
    assert!(!first_objects.is_empty());
    assert_eq!(
        first_lights.len(),
        usize::try_from(metrics.gameplay_lights).unwrap_or(usize::MAX)
    );
    assert_eq!(
        first_interior_floors
            .values()
            .copied()
            .collect::<BTreeSet<_>>()
            .len(),
        usize::try_from(metrics.interiors).unwrap_or(usize::MAX)
    );
    assert!(
        first_lights
            .keys()
            .all(|position| first_interior_floors.contains_key(position)),
        "every Ring19 gameplay light should be rooted on an exact cave floor"
    );

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();

    assert_eq!(tile_count(&mut app), 0);
    assert_eq!(
        app.world_mut()
            .query_filtered::<Entity, With<HexGrid>>()
            .iter(app.world())
            .count(),
        0
    );
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
    let gameplay_light_count_after_exit = {
        let world = app.world_mut();
        let mut lights = world.query::<&GameplayLight>();
        lights.iter(world).count()
    };
    assert_eq!(gameplay_light_count_after_exit, 0);
    assert!(object_instance_snapshot(&mut app).is_empty());

    enter_gameplay(&mut app);

    assert!(app.world().contains_resource::<TerrainReady>());
    assert!(!app.world().contains_resource::<GameplaySetupFailure>());
    assert_eq!(tile_count(&mut app), first_tile_count);
    let second_report = app.world().resource::<GenerationReport>();
    assert_eq!(second_report.generator_version, report.generator_version);
    assert_eq!(second_report.seed, report.seed);
    assert_eq!(second_report.selected_candidate, report.selected_candidate);
    assert_eq!(
        second_report.candidates_evaluated,
        report.candidates_evaluated
    );
    assert_eq!(second_report.valid_candidates, report.valid_candidates);
    assert_eq!(second_report.repair_rounds, report.repair_rounds);
    assert_eq!(second_report.repair_actions, report.repair_actions);
    assert_eq!(second_report.used_fallback, report.used_fallback);
    assert_eq!(
        second_report.settings_fingerprint,
        report.settings_fingerprint
    );
    assert_eq!(
        second_report.semantic_plan_fingerprint,
        report.semantic_plan_fingerprint
    );
    assert_eq!(second_report.map_fingerprint, report.map_fingerprint);
    assert_eq!(second_report.metrics, report.metrics);
    assert_eq!(second_report.recipe_metrics, report.recipe_metrics);
    assert_eq!(second_report.notes, report.notes);
    assert_eq!(*app.world().resource::<MapViewHint>(), first_view);

    let second_anchors: BTreeMap<String, TilePos> = app
        .world()
        .resource::<MapAnchors>()
        .iter()
        .map(|(id, position)| (id.as_str().to_owned(), position))
        .collect();
    let second_biomes: BTreeMap<TilePos, BiomeRegionId> =
        app.world().resource::<BiomeRegions>().iter().collect();
    let second_blockers = app
        .world()
        .resource::<TraversalBlockers>()
        .iter()
        .collect::<BTreeSet<_>>();
    let second_special_regions: BTreeMap<TilePos, SpecialMovementRegion> = app
        .world()
        .resource::<SpecialMovementRegions>()
        .iter()
        .collect();
    let second_interior_floors: BTreeMap<TilePos, InteriorRegionId> = app
        .world()
        .resource::<InteriorRegions>()
        .surfaces()
        .collect();
    let second_interior_roofs: BTreeMap<TilePos, InteriorRegionId> = app
        .world()
        .resource::<InteriorRegions>()
        .roof_voxels()
        .collect();
    let second_lights: BTreeMap<TilePos, GameplayLight> = {
        let world = app.world_mut();
        let mut lights = world.query::<(&TilePos, &GameplayLight)>();
        lights
            .iter(world)
            .map(|(position, light)| (*position, *light))
            .collect()
    };
    let second_objects = object_instance_snapshot(&mut app);
    assert_eq!(second_anchors, first_anchors);
    assert_eq!(second_biomes, first_biomes);
    assert_eq!(second_blockers, first_blockers);
    assert_eq!(second_special_regions, first_special_regions);
    assert_eq!(second_interior_floors, first_interior_floors);
    assert_eq!(second_interior_roofs, first_interior_roofs);
    assert_eq!(second_lights, first_lights);
    assert_eq!(second_objects, first_objects);
}
