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
fn generic_macro_metrics_are_public_reflected_and_exhaustive() {
    let metrics = MacroMetrics {
        world_columns: 1,
        macro_cells: 2,
        biome_regions: 3,
        reciprocal_seams: 4,
        outer_macro_sides: 5,
        ordinary_surfaces: 6,
        reachable_surfaces: 7,
        reachable_elevation_levels: 8,
        relief: 9,
        critical_route_steps: 10,
        standing_water_seams: 11,
        directed_liquid_seams: 12,
        liquid_cells: 13,
    };
    let ProceduralRecipeMetrics::Macro(reflected) = ProceduralRecipeMetrics::Macro(metrics) else {
        panic!("the generic Macro report must retain its exact aggregate metrics");
    };
    assert_eq!(reflected, metrics);

    let app = test_app();
    let registry = app.world().resource::<AppTypeRegistry>().read();
    assert!(registry.get(TypeId::of::<MacroMetrics>()).is_some());
}

#[test]
fn focused_island_metrics_are_public_reflected_and_exhaustive() {
    let sandy = SandyIsletsReportMetrics {
        world_columns: 1,
        land_surfaces: 2,
        water_cells: 3,
        land_components: 4,
        primary_reachable_surfaces: 5,
        sand_fringe_surfaces: 6,
        reachable_elevation_levels: 7,
        relief: 8,
        critical_route_steps: 9,
    };
    let wooded = WoodedIslandReportMetrics {
        world_columns: 10,
        land_surfaces: 11,
        water_cells: 12,
        sand_fringe_surfaces: 13,
        grass_interior_surfaces: 14,
        tree_roots: 15,
        reachable_surfaces: 16,
        reachable_elevation_levels: 17,
        relief: 18,
        critical_route_steps: 19,
    };
    let ProceduralRecipeMetrics::SandyIslets(reflected_sandy) =
        ProceduralRecipeMetrics::SandyIslets(sandy)
    else {
        panic!("the Sandy Islets report must retain its exact aggregate metrics");
    };
    let ProceduralRecipeMetrics::WoodedIsland(reflected_wooded) =
        ProceduralRecipeMetrics::WoodedIsland(wooded)
    else {
        panic!("the Wooded Island report must retain its exact aggregate metrics");
    };
    assert_eq!(reflected_sandy, sandy);
    assert_eq!(reflected_wooded, wooded);

    let app = test_app();
    let registry = app.world().resource::<AppTypeRegistry>().read();
    for type_id in [
        TypeId::of::<SandyIsletsReportMetrics>(),
        TypeId::of::<WoodedIslandReportMetrics>(),
    ] {
        assert!(registry.get(type_id).is_some());
    }
}

#[test]
fn ocean_archipelago_metrics_are_public_reflected_and_exhaustive() {
    let metrics = OceanArchipelagoMetrics {
        world_columns: 1,
        macro_cells: 2,
        biome_regions: 3,
        standing_water_seams: 4,
        liquid_cells: 5,
        dry_components: 6,
        scenic_dry_components: 7,
        ordinary_surfaces: 8,
        reachable_surfaces: 9,
        critical_route_steps: 10,
        shoreline_surfaces: 11,
        tree_roots: 12,
    };
    let ProceduralRecipeMetrics::OceanArchipelago(reflected) =
        ProceduralRecipeMetrics::OceanArchipelago(metrics)
    else {
        panic!("the Ocean Archipelagoes report must retain its exact aggregate metrics");
    };
    assert_eq!(reflected, metrics);

    let app = test_app();
    let registry = app.world().resource::<AppTypeRegistry>().read();
    assert!(registry
        .get(TypeId::of::<OceanArchipelagoMetrics>())
        .is_some());
}

#[test]
fn mountain_range_metrics_are_public_reflected_and_exhaustive() {
    let recipe_metrics = ProceduralRecipeMetrics::MountainRange(MountainRangeMetrics {
        world_columns: 1,
        macro_cells: 2,
        biome_regions: 3,
        reciprocal_seams: 4,
        outer_macro_sides: 5,
        ordinary_surfaces: 6,
        reachable_surfaces: 7,
        reachable_elevation_levels: 8,
        relief: 9,
        critical_route_steps: 10,
        standing_water_seams: 11,
        directed_liquid_seams: 12,
        liquid_cells: 13,
        summit_level: 14,
        high_massif_surfaces: 15,
    });
    let ProceduralRecipeMetrics::MountainRange(MountainRangeMetrics {
        world_columns,
        macro_cells,
        biome_regions,
        reciprocal_seams,
        outer_macro_sides,
        ordinary_surfaces,
        reachable_surfaces,
        reachable_elevation_levels,
        relief,
        critical_route_steps,
        standing_water_seams,
        directed_liquid_seams,
        liquid_cells,
        summit_level,
        high_massif_surfaces,
    }) = recipe_metrics
    else {
        panic!("the Mountain Range report must retain its exact aggregate metrics");
    };
    assert_eq!(
        (
            world_columns,
            macro_cells,
            biome_regions,
            reciprocal_seams,
            outer_macro_sides,
            ordinary_surfaces,
            reachable_surfaces,
            reachable_elevation_levels,
            relief,
            critical_route_steps,
        ),
        (1, 2, 3, 4, 5, 6, 7, 8, 9, 10)
    );
    assert_eq!(
        (
            standing_water_seams,
            directed_liquid_seams,
            liquid_cells,
            summit_level,
            high_massif_surfaces,
        ),
        (11, 12, 13, 14, 15)
    );

    let app = test_app();
    let registry = app.world().resource::<AppTypeRegistry>().read();
    for type_id in [
        TypeId::of::<ProceduralRecipeMetrics>(),
        TypeId::of::<MountainRangeMetrics>(),
    ] {
        assert!(
            registry.get(type_id).is_some(),
            "Mountain Range report vocabulary is missing reflection registration"
        );
    }
}

#[test]
fn mountain_range_materializes_the_authored_macro_world() {
    const RADIUS_77_COLUMNS: usize = 1 + 3 * 77 * 78;

    let mut app = v3_mountain_range_app();
    enter_gameplay(&mut app);

    assert!(
        app.world().contains_resource::<TerrainReady>(),
        "setup failed: {:?}",
        app.world()
            .get_resource::<GameplaySetupFailure>()
            .map(|failure| failure.reason.as_str())
    );
    assert_eq!(app.world().resource::<VoxelMap>().len(), RADIUS_77_COLUMNS);

    let report = app.world().resource::<GenerationReport>().clone();
    assert_eq!(report.generator_version, 3);
    assert_eq!(report.seed, 129_704_046);
    assert_eq!(
        report.settings_fingerprint, 2_843_243_527_997_079_402,
        "update only with an explicit shipped Mountain Range settings-identity decision"
    );
    assert_eq!(
        report.semantic_plan_fingerprint,
        Some(347_825_722_077_974_933),
        "update only with an explicit shipped Mountain Range semantic-plan decision"
    );
    assert_eq!(
        report.map_fingerprint, 4_089_854_997_773_874_143,
        "update only with an explicit shipped Mountain Range materialized-map decision"
    );
    let Some(ProceduralRecipeMetrics::MountainRange(metrics)) = report.recipe_metrics.as_ref()
    else {
        panic!("Mountain Range should publish its exact aggregate metrics");
    };
    assert_eq!(
        metrics,
        &MountainRangeMetrics {
            world_columns: 18_019,
            macro_cells: 37,
            biome_regions: 30,
            reciprocal_seams: 74,
            outer_macro_sides: 42,
            ordinary_surfaces: 11_858,
            reachable_surfaces: 2_482,
            reachable_elevation_levels: 51,
            relief: 92,
            critical_route_steps: 99,
            standing_water_seams: 9,
            directed_liquid_seams: 6,
            liquid_cells: 3_579,
            summit_level: 96,
            high_massif_surfaces: 1_053,
        },
        "update only with an explicit shipped Mountain Range aggregate-contract decision"
    );

    let represented_regions = app
        .world()
        .resource::<BiomeRegions>()
        .iter()
        .map(|(_, region)| region)
        .collect::<BTreeSet<_>>();
    assert_eq!(represented_regions, (0..30).map(BiomeRegionId).collect());
    let expected_anchors = [
        (
            "party_start",
            TilePos::new(HexCoord::new_cubic(-36, 27, 9), 13),
        ),
        (
            "hostile_start",
            TilePos::new(HexCoord::new_cubic(-8, 13, -5), 20),
        ),
        (
            "coast_review",
            TilePos::new(HexCoord::new_cubic(-52, 25, 27), 12),
        ),
        (
            "beach_review",
            TilePos::new(HexCoord::new_cubic(-46, -31, 77), 10),
        ),
        (
            "inland_review",
            TilePos::new(HexCoord::new_cubic(-9, 14, -5), 20),
        ),
        (
            "foothill_review",
            TilePos::new(HexCoord::new_cubic(-7, 13, -6), 20),
        ),
        (
            "massif_front_review",
            TilePos::new(HexCoord::new_cubic(31, 5, -36), 34),
        ),
        (
            "deep_mountain_review",
            TilePos::new(HexCoord::new_cubic(54, 5, -59), 48),
        ),
        (
            "deep_mountain_base",
            TilePos::new(HexCoord::new_cubic(53, 6, -59), 48),
        ),
    ];
    let first_anchors = app
        .world()
        .resource::<MapAnchors>()
        .iter()
        .map(|(id, position)| (id.as_str().to_owned(), position))
        .collect::<BTreeMap<_, _>>();
    for (name, expected) in expected_anchors {
        assert_eq!(
            first_anchors.get(name),
            Some(&expected),
            "Mountain Range anchor {name} drifted"
        );
    }
    let first_view = *app.world().resource::<MapViewHint>();
    assert!(first_view.is_valid());
    let (generated_maximum, horizontal_span) = {
        let map = app.world().resource::<VoxelMap>();
        let generated_maximum = map
            .columns()
            .filter_map(|(_, column)| column.surface())
            .max()
            .expect("Mountain Range should have generated terrain");
        let bounds = map.columns().map(|(coord, _)| coord.to_world(0.0)).fold(
            None::<(f32, f32, f32, f32)>,
            |bounds, point| match bounds {
                None => Some((point.x, point.x, point.z, point.z)),
                Some((min_x, max_x, min_z, max_z)) => Some((
                    min_x.min(point.x),
                    max_x.max(point.x),
                    min_z.min(point.z),
                    max_z.max(point.z),
                )),
            },
        );
        let (min_x, max_x, min_z, max_z) = bounds.expect("Mountain Range has footprint bounds");
        (generated_maximum, (max_x - min_x).hypot(max_z - min_z))
    };
    assert_eq!(generated_maximum, metrics.summit_level);
    let eye = Vec3::from(first_view.eye);
    let focus = Vec3::from(first_view.focus);
    let generated_height = f32::from(
        i16::try_from(generated_maximum).expect("Mountain Range elevation fits camera math"),
    ) * 0.4;
    assert!((focus.y - generated_height * 0.36).abs() < 1e-4);
    let derived_frame = (eye.z - focus.z).abs() / 0.82;
    assert!(derived_frame + 1e-3 >= horizontal_span * 0.78);
    let camera: CameraSettings =
        ron::from_str(include_str!("../../../../assets/config/camera.ron"))
            .expect("shipped camera settings should parse");
    assert!(
        eye.distance(focus) * 1.1 > camera.max_zoom,
        "Mountain Range hint must extend the ordinary Map zoom ceiling"
    );

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();
    assert!(!app.world().contains_resource::<VoxelMap>());
    assert!(!app.world().contains_resource::<MapAnchors>());
    assert!(!app.world().contains_resource::<BiomeRegions>());
    assert!(!app.world().contains_resource::<GenerationReport>());
    assert!(!app.world().contains_resource::<TerrainReady>());

    enter_gameplay(&mut app);
    assert!(app.world().contains_resource::<TerrainReady>());
    assert!(!app.world().contains_resource::<GameplaySetupFailure>());
    let second_report = app.world().resource::<GenerationReport>();
    assert_eq!(
        second_report.settings_fingerprint,
        report.settings_fingerprint
    );
    assert_eq!(
        second_report.semantic_plan_fingerprint,
        report.semantic_plan_fingerprint
    );
    assert_eq!(second_report.map_fingerprint, report.map_fingerprint);
    assert_eq!(second_report.recipe_metrics, report.recipe_metrics);
    assert_eq!(*app.world().resource::<MapViewHint>(), first_view);
    let second_anchors = app
        .world()
        .resource::<MapAnchors>()
        .iter()
        .map(|(id, position)| (id.as_str().to_owned(), position))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(second_anchors, first_anchors);
}

#[expect(
    clippy::expect_used,
    reason = "the tracked Crystal Mountain review world is a compile-time integration fixture"
)]
fn v3_crystal_mountain_app() -> App {
    let mut app = test_app();
    let settings: MapSettings = ron::from_str(include_str!(
        "../../../../assets/config/worlds/procedural-crystal-mountain.ron"
    ))
    .expect("tracked Crystal Mountain settings should parse");
    app.insert_resource(settings);
    app.insert_resource(ResolvedMapSeed(1_592_598_566));
    app.insert_resource(runtime_art_catalog());
    app
}

#[test]
fn crystal_mountain_materializes_and_reenters_with_exact_runtime_projections() {
    const RADIUS_77_COLUMNS: usize = 1 + 3 * 77 * 78;

    let mut app = v3_crystal_mountain_app();
    enter_gameplay(&mut app);

    assert!(
        app.world().contains_resource::<TerrainReady>(),
        "Crystal Mountain setup failed: {:?}",
        app.world()
            .get_resource::<GameplaySetupFailure>()
            .map(|failure| failure.reason.as_str())
    );
    assert_eq!(app.world().resource::<VoxelMap>().len(), RADIUS_77_COLUMNS);
    let first_tile_count = tile_count(&mut app);
    assert!(first_tile_count >= RADIUS_77_COLUMNS);

    let report = app.world().resource::<GenerationReport>().clone();
    assert_eq!(report.generator_version, 3);
    assert_eq!(report.seed, 1_592_598_566);
    let Some(ProceduralRecipeMetrics::Macro(metrics)) = report.recipe_metrics.as_ref() else {
        panic!(
            "Crystal Mountain should publish generic Macro metrics, got {:?}",
            report.recipe_metrics
        );
    };
    assert_eq!(metrics.world_columns, 18_019);
    assert_eq!(metrics.macro_cells, 37);
    assert_eq!(metrics.biome_regions, 4);
    assert!(metrics.ordinary_surfaces > 0);
    assert!(metrics.reachable_surfaces > 0);
    assert!(metrics.critical_route_steps > 144);
    assert_eq!(
        report.metrics.critical_route_steps,
        metrics.critical_route_steps
    );

    let first_anchors = app
        .world()
        .resource::<MapAnchors>()
        .iter()
        .map(|(id, position)| (id.as_str().to_owned(), position))
        .collect::<BTreeMap<_, _>>();
    for required in [
        "party_start",
        "crystal_mountain.foot_apron",
        "crystal_mountain.tunnel_mouth",
        "crystal_mountain.midpoint",
        "crystal_mountain.gothic_transition",
        "crystal_mountain.ascent_threshold",
        "crystal_mountain.summit_exit",
        "crystal_mountain.basin_clearing",
        "crystal_mountain.ridge",
    ] {
        assert!(
            first_anchors.contains_key(required),
            "Crystal Mountain omitted anchor {required:?}"
        );
    }

    let first_biomes: BTreeMap<TilePos, BiomeRegionId> =
        app.world().resource::<BiomeRegions>().iter().collect();
    assert_eq!(
        first_biomes
            .values()
            .copied()
            .collect::<BTreeSet<BiomeRegionId>>()
            .len(),
        4,
        "Crystal Mountain should retain four logical biome owners"
    );
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
    assert!(!first_interior_floors.is_empty());
    assert!(!first_interior_roofs.is_empty());
    assert_eq!(
        first_interior_floors
            .values()
            .copied()
            .collect::<BTreeSet<_>>()
            .len(),
        1,
        "the tunnel and Crystal Ascent must materialize as one interior"
    );
    assert_eq!(
        first_interior_roofs
            .values()
            .copied()
            .collect::<BTreeSet<_>>()
            .len(),
        1,
        "the tunnel and Crystal Ascent roof must share that interior"
    );
    let first_lights: BTreeMap<TilePos, GameplayLight> = {
        let world = app.world_mut();
        let mut lights = world.query::<(&TilePos, &GameplayLight)>();
        lights
            .iter(world)
            .map(|(position, light)| (*position, *light))
            .collect()
    };
    let first_objects = object_instance_snapshot(&mut app);
    let first_blockers = app
        .world()
        .resource::<TraversalBlockers>()
        .iter()
        .collect::<BTreeSet<_>>();
    let first_view = *app.world().resource::<MapViewHint>();
    assert!(first_view.is_valid());
    assert!(!first_lights.is_empty());
    assert!(!first_objects.is_empty());
    assert!(!first_blockers.is_empty());

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();

    assert_eq!(tile_count(&mut app), 0);
    for absent in [
        app.world().contains_resource::<VoxelMap>(),
        app.world().contains_resource::<MapAnchors>(),
        app.world().contains_resource::<InteriorRegions>(),
        app.world().contains_resource::<TraversalBlockers>(),
        app.world().contains_resource::<BiomeRegions>(),
        app.world().contains_resource::<GenerationReport>(),
        app.world().contains_resource::<TerrainReady>(),
    ] {
        assert!(!absent);
    }
    assert!(object_instance_snapshot(&mut app).is_empty());
    let lights_after_exit = {
        let world = app.world_mut();
        let mut lights = world.query::<&GameplayLight>();
        lights.iter(world).count()
    };
    assert_eq!(lights_after_exit, 0);

    enter_gameplay(&mut app);
    assert!(app.world().contains_resource::<TerrainReady>());
    assert!(!app.world().contains_resource::<GameplaySetupFailure>());
    assert_eq!(tile_count(&mut app), first_tile_count);
    let second_report = app.world().resource::<GenerationReport>();
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

    let second_anchors = app
        .world()
        .resource::<MapAnchors>()
        .iter()
        .map(|(id, position)| (id.as_str().to_owned(), position))
        .collect::<BTreeMap<_, _>>();
    let second_biomes: BTreeMap<TilePos, BiomeRegionId> =
        app.world().resource::<BiomeRegions>().iter().collect();
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
    let second_blockers = app
        .world()
        .resource::<TraversalBlockers>()
        .iter()
        .collect::<BTreeSet<_>>();
    assert_eq!(second_anchors, first_anchors);
    assert_eq!(second_biomes, first_biomes);
    assert_eq!(second_interior_floors, first_interior_floors);
    assert_eq!(second_interior_roofs, first_interior_roofs);
    assert_eq!(second_lights, first_lights);
    assert_eq!(second_objects, first_objects);
    assert_eq!(second_blockers, first_blockers);
}

#[test]
fn macro_world_generation_is_not_coupled_to_mountain_range_instance_names() {
    let mut app = v3_mountain_range_app();
    {
        let mut settings = app.world_mut().resource_mut::<MapSettings>();
        let TerrainSettings::Procedural(ProceduralSettings::V3(v3)) = &mut settings.terrain else {
            unreachable!("Mountain Range fixture uses procedural V3");
        };
        let V3LayoutSettings::Macro(layout) = &mut v3.layout else {
            unreachable!("Mountain Range fixture uses Macro layout");
        };
        let renamed = layout
            .instances
            .iter()
            .enumerate()
            .map(|(index, instance)| (instance.name.clone(), format!("generic-region-{index:02}")))
            .collect::<BTreeMap<_, _>>();
        let renamed_instance = |name: &str| {
            let Some(renamed) = renamed.get(name) else {
                panic!("generic Macro rename table omitted instance {name:?}");
            };
            renamed.clone()
        };
        for instance in &mut layout.instances {
            instance.name = renamed_instance(&instance.name);
        }
        for connection in &mut layout.liquid_connections {
            match connection {
                MacroLiquidConnectionSettings::Standing {
                    first_instance,
                    second_instance,
                    ..
                } => {
                    *first_instance = renamed_instance(first_instance);
                    *second_instance = renamed_instance(second_instance);
                }
                MacroLiquidConnectionSettings::Directed {
                    source_instance,
                    sink_instance,
                    ..
                } => {
                    *source_instance = renamed_instance(source_instance);
                    *sink_instance = renamed_instance(sink_instance);
                }
            }
        }
        for headwater in &mut layout.headwaters {
            match headwater {
                MacroHeadwaterSettings::CaveFall { instance, .. }
                | MacroHeadwaterSettings::RivuletConfluence { instance, .. } => {
                    *instance = renamed_instance(instance);
                }
            }
        }
        for route_instance in &mut layout.critical_route {
            *route_instance = renamed_instance(route_instance);
        }
    }

    enter_gameplay(&mut app);
    assert!(
        app.world().contains_resource::<TerrainReady>(),
        "generic Macro setup failed: {:?}",
        app.world()
            .get_resource::<GameplaySetupFailure>()
            .map(|failure| failure.reason.as_str())
    );
    let report = app.world().resource::<GenerationReport>();
    let Some(ProceduralRecipeMetrics::Macro(metrics)) = report.recipe_metrics.as_ref() else {
        panic!("a non-Mountain-Range Macro layout must publish generic Macro metrics");
    };
    assert_eq!(metrics.world_columns, 18_019);
    assert_eq!(metrics.macro_cells, 37);
    assert_eq!(metrics.biome_regions, 30);
    assert_eq!(metrics.reciprocal_seams, 74);
    assert!(metrics.critical_route_steps > 0);
    let anchors = app.world().resource::<MapAnchors>();
    for required in ["party_start", "hostile_start", "macro_route_end"] {
        assert!(
            anchors.get(&MapAnchorId::from(required)).is_some(),
            "generic Macro world omitted canonical anchor {required:?}"
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
