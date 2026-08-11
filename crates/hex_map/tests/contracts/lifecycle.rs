use super::*;

#[test]
fn v3_forest_missing_runtime_art_catalog_fails_before_terrain_publication() {
    let mut app = v3_forest_app();
    app.world_mut().remove_resource::<RuntimeArtCatalog>();
    enter_gameplay(&mut app);

    let failure = app
        .world()
        .get_resource::<GameplaySetupFailure>()
        .expect("missing Forest art graph should publish a setup failure");
    assert!(
        failure.reason.contains("runtime art catalog"),
        "unexpected setup failure: {}",
        failure.reason
    );
    assert!(!app.world().contains_resource::<TerrainReady>());
    assert_eq!(tile_count(&mut app), 0);
    assert_eq!(
        app.world_mut()
            .query_filtered::<Entity, With<HexGrid>>()
            .iter(app.world())
            .count(),
        0,
        "failed Forest presentation spawned a partial grid"
    );
}

#[test]
fn invalid_v3_recipe_contract_fails_closed_and_clears_stale_generated_state() {
    let mut app = v3_hills_app();
    {
        let mut settings = app.world_mut().resource_mut::<MapSettings>();
        let TerrainSettings::Procedural(ProceduralSettings::V3(v3)) = &mut settings.terrain else {
            panic!("test uses V3 procedural settings");
        };
        let V3LayoutSettings::Single(patch) = &mut v3.layout else {
            panic!("test uses a Single layout");
        };
        patch.environment = V3EnvironmentSettings::Rocky;
    }
    insert_stale_generated_resources(&mut app);

    enter_gameplay(&mut app);

    let failure = app.world().resource::<GameplaySetupFailure>();
    assert!(
        failure
            .reason
            .contains("Hills does not support the Rocky environment"),
        "unexpected setup failure: {}",
        failure.reason
    );
    assert!(!app.world().contains_resource::<TerrainReady>());
    assert!(!app.world().contains_resource::<VoxelMap>());
    assert!(!app.world().contains_resource::<MapAnchors>());
    assert!(!app.world().contains_resource::<SpecialMovementRegions>());
    assert!(!app.world().contains_resource::<InteriorRegions>());
    assert!(!app.world().contains_resource::<TraversalBlockers>());
    assert!(!app.world().contains_resource::<BiomeRegions>());
    assert!(!app.world().contains_resource::<MapViewHint>());
    assert!(!app.world().contains_resource::<GenerationReport>());

    let grids = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .iter(app.world())
        .count();
    assert_eq!(grids, 0, "failed V3 setup spawned a grid");
}

#[test]
fn procedural_setup_without_a_seed_never_marks_terrain_ready() {
    let mut app = procedural_app();
    app.world_mut().remove_resource::<ResolvedMapSeed>();
    enter_gameplay(&mut app);

    assert!(!app.world().contains_resource::<TerrainReady>());
    assert!(!app.world().contains_resource::<VoxelMap>());
    assert!(
        !app.world().contains_resource::<SpecialMovementRegions>(),
        "failed generation published special-region semantics"
    );
    assert!(app
        .world()
        .resource::<GameplaySetupFailure>()
        .reason
        .contains("generation seed"));
    assert_eq!(tile_count(&mut app), 0);
}

#[test]
fn a_missing_required_substance_never_marks_terrain_ready() {
    let mut app = procedural_app();
    app.insert_resource(substance_table_without(Some("water")));
    enter_gameplay(&mut app);

    assert!(!app.world().contains_resource::<TerrainReady>());
    assert!(!app.world().contains_resource::<VoxelMap>());
    assert!(
        !app.world().contains_resource::<SpecialMovementRegions>(),
        "failed generation published special-region semantics"
    );
    assert!(app
        .world()
        .resource::<GameplaySetupFailure>()
        .reason
        .contains("water"));
    assert_eq!(tile_count(&mut app), 0);
}

/// Leaving gameplay must remove everything the map built. A leak here would grow
/// the world every time the player returned to the title screen and started again.
#[test]
fn leaving_gameplay_removes_the_map() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    assert!(tile_count(&mut app) > 0, "precondition: tiles exist");

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();

    assert_eq!(
        tile_count(&mut app),
        0,
        "tiles outlived the gameplay screen"
    );
    let grids = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .iter(app.world())
        .count();
    assert_eq!(grids, 0, "the grid parent outlived the gameplay screen");
    assert!(
        app.world().get_resource::<VoxelMap>().is_none(),
        "voxel storage outlived the gameplay screen"
    );
    assert!(
        app.world().get_resource::<MapAnchors>().is_none(),
        "map anchors outlived the gameplay screen"
    );
    assert!(
        app.world()
            .get_resource::<SpecialMovementRegions>()
            .is_none(),
        "special-movement regions outlived the gameplay screen"
    );
    assert!(
        app.world().get_resource::<TerrainReady>().is_none(),
        "terrain readiness outlived the gameplay screen"
    );
}

#[test]
fn leaving_v2_hills_removes_all_generated_resources() {
    let mut app = v2_hills_app();
    enter_gameplay(&mut app);
    assert!(app.world().contains_resource::<TerrainReady>());
    assert!(app.world().contains_resource::<MapViewHint>());
    assert!(app.world().contains_resource::<GenerationReport>());

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();

    assert_eq!(tile_count(&mut app), 0);
    assert!(!app.world().contains_resource::<VoxelMap>());
    assert!(!app.world().contains_resource::<MapAnchors>());
    assert!(!app.world().contains_resource::<SpecialMovementRegions>());
    assert!(!app.world().contains_resource::<InteriorRegions>());
    assert!(!app.world().contains_resource::<TraversalBlockers>());
    assert!(!app.world().contains_resource::<BiomeRegions>());
    assert!(!app.world().contains_resource::<MapViewHint>());
    assert!(!app.world().contains_resource::<GenerationReport>());
    assert!(!app.world().contains_resource::<TerrainReady>());
}

/// Re-entering rebuilds a complete grid rather than doubling it or leaving gaps.
#[test]
fn gameplay_can_be_re_entered() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    let first = tile_count(&mut app);

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();
    enter_gameplay(&mut app);

    assert_eq!(
        tile_count(&mut app),
        first,
        "rebuild should match the first"
    );
}

#[test]
fn v2_hills_reentry_is_deterministic() {
    let mut app = v2_hills_app();
    enter_gameplay(&mut app);
    let first_report = app.world().resource::<GenerationReport>().clone();
    let first_view = *app.world().resource::<MapViewHint>();
    let first_anchors: BTreeMap<String, TilePos> = app
        .world()
        .resource::<MapAnchors>()
        .iter()
        .map(|(id, position)| (id.as_str().to_owned(), position))
        .collect();

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();
    enter_gameplay(&mut app);

    let second_report = app.world().resource::<GenerationReport>();
    assert_eq!(second_report.map_fingerprint, first_report.map_fingerprint);
    assert_eq!(
        second_report.settings_fingerprint,
        first_report.settings_fingerprint
    );
    assert_eq!(
        second_report.selected_candidate,
        first_report.selected_candidate
    );
    assert_eq!(
        second_report.valid_candidates,
        first_report.valid_candidates
    );
    assert_eq!(second_report.repair_actions, first_report.repair_actions);
    assert_eq!(second_report.metrics, first_report.metrics);
    assert_eq!(*app.world().resource::<MapViewHint>(), first_view);

    let second_anchors: BTreeMap<String, TilePos> = app
        .world()
        .resource::<MapAnchors>()
        .iter()
        .map(|(id, position)| (id.as_str().to_owned(), position))
        .collect();
    assert_eq!(second_anchors, first_anchors);
}

#[test]
fn v3_waterfall_teardown_and_reentry_preserve_exact_generated_state() {
    let mut app = v3_waterfall_app();
    enter_gameplay(&mut app);

    let first_report = app.world().resource::<GenerationReport>().clone();
    let first_view = *app.world().resource::<MapViewHint>();
    let first_anchors: BTreeMap<String, TilePos> = app
        .world()
        .resource::<MapAnchors>()
        .iter()
        .map(|(id, position)| (id.as_str().to_owned(), position))
        .collect();
    let first_biomes: BTreeMap<TilePos, BiomeRegionId> =
        app.world().resource::<BiomeRegions>().iter().collect();
    let first_tile_count = tile_count(&mut app);
    let first_presentation_count = liquid_presentations(&mut app).len();

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();

    assert_eq!(tile_count(&mut app), 0);
    assert!(liquid_presentations(&mut app).is_empty());
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
    assert_eq!(tile_count(&mut app), first_tile_count);
    assert_eq!(
        liquid_presentations(&mut app).len(),
        first_presentation_count
    );
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
    assert_eq!(*app.world().resource::<MapViewHint>(), first_view);
    let second_anchors: BTreeMap<String, TilePos> = app
        .world()
        .resource::<MapAnchors>()
        .iter()
        .map(|(id, position)| (id.as_str().to_owned(), position))
        .collect();
    let second_biomes: BTreeMap<TilePos, BiomeRegionId> =
        app.world().resource::<BiomeRegions>().iter().collect();
    assert_eq!(second_anchors, first_anchors);
    assert_eq!(second_biomes, first_biomes);
}

#[test]
fn v2_layered_sky_teardown_and_reentry_preserve_generated_state() {
    let mut app = v2_layered_sky_app();
    enter_gameplay(&mut app);

    assert!(
        app.world().contains_resource::<TerrainReady>(),
        "setup failed: {:?}",
        app.world()
            .get_resource::<GameplaySetupFailure>()
            .map(|failure| failure.reason.as_str())
    );
    assert!(!app.world().contains_resource::<GameplaySetupFailure>());

    let first_tile_count = tile_count(&mut app);
    let first_report = app.world().resource::<GenerationReport>().clone();
    let first_view = *app.world().resource::<MapViewHint>();
    let first_anchors: BTreeMap<String, TilePos> = app
        .world()
        .resource::<MapAnchors>()
        .iter()
        .map(|(id, position)| (id.as_str().to_owned(), position))
        .collect();
    let first_special_regions: BTreeMap<TilePos, SpecialMovementRegion> = app
        .world()
        .resource::<SpecialMovementRegions>()
        .iter()
        .collect();
    let first_interior_surfaces: BTreeMap<TilePos, InteriorRegionId> = app
        .world()
        .resource::<InteriorRegions>()
        .surfaces()
        .collect();
    let first_roof_voxels: BTreeMap<TilePos, InteriorRegionId> = app
        .world()
        .resource::<InteriorRegions>()
        .roof_voxels()
        .collect();

    assert!(first_tile_count > 0);
    assert!(!first_special_regions.is_empty());
    assert_eq!(first_report.generator_version, 2);

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
    assert!(!app.world().contains_resource::<VoxelMap>());
    assert!(!app.world().contains_resource::<MapAnchors>());
    assert!(!app.world().contains_resource::<SpecialMovementRegions>());
    assert!(!app.world().contains_resource::<InteriorRegions>());
    assert!(!app.world().contains_resource::<MapViewHint>());
    assert!(!app.world().contains_resource::<GenerationReport>());
    assert!(!app.world().contains_resource::<TerrainReady>());

    enter_gameplay(&mut app);

    assert_eq!(tile_count(&mut app), first_tile_count);
    assert!(app.world().contains_resource::<TerrainReady>());
    assert!(!app.world().contains_resource::<GameplaySetupFailure>());

    let second_report = app.world().resource::<GenerationReport>();
    assert_eq!(
        second_report.generator_version,
        first_report.generator_version
    );
    assert_eq!(second_report.seed, first_report.seed);
    assert_eq!(
        second_report.selected_candidate,
        first_report.selected_candidate
    );
    assert_eq!(
        second_report.candidates_evaluated,
        first_report.candidates_evaluated
    );
    assert_eq!(
        second_report.valid_candidates,
        first_report.valid_candidates
    );
    assert_eq!(second_report.repair_rounds, first_report.repair_rounds);
    assert_eq!(second_report.repair_actions, first_report.repair_actions);
    assert_eq!(second_report.used_fallback, first_report.used_fallback);
    assert_eq!(
        second_report.settings_fingerprint,
        first_report.settings_fingerprint
    );
    assert_eq!(second_report.map_fingerprint, first_report.map_fingerprint);
    assert_eq!(second_report.metrics, first_report.metrics);
    assert_eq!(second_report.notes, first_report.notes);
    assert_eq!(*app.world().resource::<MapViewHint>(), first_view);

    let second_anchors: BTreeMap<String, TilePos> = app
        .world()
        .resource::<MapAnchors>()
        .iter()
        .map(|(id, position)| (id.as_str().to_owned(), position))
        .collect();
    let second_special_regions: BTreeMap<TilePos, SpecialMovementRegion> = app
        .world()
        .resource::<SpecialMovementRegions>()
        .iter()
        .collect();
    let second_interior_surfaces: BTreeMap<TilePos, InteriorRegionId> = app
        .world()
        .resource::<InteriorRegions>()
        .surfaces()
        .collect();
    let second_roof_voxels: BTreeMap<TilePos, InteriorRegionId> = app
        .world()
        .resource::<InteriorRegions>()
        .roof_voxels()
        .collect();

    assert_eq!(second_anchors, first_anchors);
    assert_eq!(second_special_regions, first_special_regions);
    assert_eq!(second_interior_surfaces, first_interior_surfaces);
    assert_eq!(second_roof_voxels, first_roof_voxels);
}

#[test]
fn v2_mountains_teardown_and_reentry_preserve_generated_state() {
    let mut app = v2_mountains_app();
    enter_gameplay(&mut app);

    assert!(
        app.world().contains_resource::<TerrainReady>(),
        "setup failed: {:?}",
        app.world()
            .get_resource::<GameplaySetupFailure>()
            .map(|failure| failure.reason.as_str())
    );
    let first_tile_count = tile_count(&mut app);
    let first_report = app.world().resource::<GenerationReport>().clone();
    let first_view = *app.world().resource::<MapViewHint>();
    let first_anchors: BTreeMap<String, TilePos> = app
        .world()
        .resource::<MapAnchors>()
        .iter()
        .map(|(id, position)| (id.as_str().to_owned(), position))
        .collect();
    let first_regions: BTreeMap<TilePos, SpecialMovementRegion> = app
        .world()
        .resource::<SpecialMovementRegions>()
        .iter()
        .collect();

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();

    assert_eq!(tile_count(&mut app), 0);
    assert!(!app.world().contains_resource::<VoxelMap>());
    assert!(!app.world().contains_resource::<MapAnchors>());
    assert!(!app.world().contains_resource::<SpecialMovementRegions>());
    assert!(!app.world().contains_resource::<InteriorRegions>());
    assert!(!app.world().contains_resource::<MapViewHint>());
    assert!(!app.world().contains_resource::<GenerationReport>());
    assert!(!app.world().contains_resource::<TerrainReady>());

    enter_gameplay(&mut app);

    assert_eq!(tile_count(&mut app), first_tile_count);
    assert!(app.world().contains_resource::<TerrainReady>());
    assert!(!app.world().contains_resource::<GameplaySetupFailure>());
    let second_report = app.world().resource::<GenerationReport>();
    assert_eq!(second_report.seed, first_report.seed);
    assert_eq!(
        second_report.selected_candidate,
        first_report.selected_candidate
    );
    assert_eq!(
        second_report.valid_candidates,
        first_report.valid_candidates
    );
    assert_eq!(second_report.repair_actions, first_report.repair_actions);
    assert_eq!(
        second_report.settings_fingerprint,
        first_report.settings_fingerprint
    );
    assert_eq!(second_report.map_fingerprint, first_report.map_fingerprint);
    assert_eq!(second_report.metrics, first_report.metrics);
    assert_eq!(*app.world().resource::<MapViewHint>(), first_view);

    let second_anchors: BTreeMap<String, TilePos> = app
        .world()
        .resource::<MapAnchors>()
        .iter()
        .map(|(id, position)| (id.as_str().to_owned(), position))
        .collect();
    let second_regions: BTreeMap<TilePos, SpecialMovementRegion> = app
        .world()
        .resource::<SpecialMovementRegions>()
        .iter()
        .collect();
    assert_eq!(second_anchors, first_anchors);
    assert_eq!(second_regions, first_regions);
}

#[test]
fn v3_caves_missing_crystal_asset_fails_before_any_world_publication() {
    let mut app = v3_caves_app_without_art_catalog();
    app.insert_resource(runtime_art_catalog_without(Some("prop/crystal-spire")));
    enter_gameplay(&mut app);

    assert!(!app.world().contains_resource::<TerrainReady>());
    assert!(!app.world().contains_resource::<VoxelMap>());
    assert!(!app.world().contains_resource::<MapAnchors>());
    assert!(!app.world().contains_resource::<InteriorRegions>());
    assert_eq!(tile_count(&mut app), 0);
    assert!(cave_crystal_instances(&mut app).is_empty());
    let gameplay_light_count = app
        .world_mut()
        .query::<&GameplayLight>()
        .iter(app.world())
        .count();
    let point_light_count = app
        .world_mut()
        .query::<&PointLight>()
        .iter(app.world())
        .count();
    assert_eq!(gameplay_light_count, 0);
    assert_eq!(point_light_count, 0);
    assert!(app
        .world()
        .resource::<GameplaySetupFailure>()
        .reason
        .contains("prop/crystal-spire"));
}

#[test]
fn v3_caves_missing_vegetation_asset_fails_before_any_world_publication() {
    let mut app = v3_caves_app_without_art_catalog();
    app.insert_resource(runtime_art_catalog_without(Some("prop/cave-moss")));
    enter_gameplay(&mut app);

    assert!(!app.world().contains_resource::<TerrainReady>());
    assert!(!app.world().contains_resource::<VoxelMap>());
    assert!(!app.world().contains_resource::<MapAnchors>());
    assert!(!app.world().contains_resource::<InteriorRegions>());
    assert_eq!(tile_count(&mut app), 0);
    assert!(cave_crystal_instances(&mut app).is_empty());
    assert!(cave_vegetation_instances(&mut app).is_empty());
    assert!(app
        .world()
        .resource::<GameplaySetupFailure>()
        .reason
        .contains("prop/cave-moss"));
}

#[test]
fn v3_caves_teardown_and_reentry_preserve_exact_interiors_and_lights() {
    let mut app = v3_caves_app();
    enter_gameplay(&mut app);

    assert!(
        app.world().contains_resource::<TerrainReady>(),
        "setup failed: {:?}",
        app.world()
            .get_resource::<GameplaySetupFailure>()
            .map(|failure| failure.reason.as_str())
    );
    let first_tile_count = tile_count(&mut app);
    let first_report = app.world().resource::<GenerationReport>().clone();
    let first_view = *app.world().resource::<MapViewHint>();
    let first_anchors: BTreeMap<String, TilePos> = app
        .world()
        .resource::<MapAnchors>()
        .iter()
        .map(|(id, position)| (id.as_str().to_owned(), position))
        .collect();
    let first_floors: BTreeMap<TilePos, InteriorRegionId> = app
        .world()
        .resource::<InteriorRegions>()
        .surfaces()
        .collect();
    let first_roofs: BTreeMap<TilePos, InteriorRegionId> = app
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
    let first_crystals = cave_crystal_instances(&mut app);
    let first_vegetation = cave_vegetation_instances(&mut app);

    assert!(!first_floors.is_empty());
    assert!(!first_roofs.is_empty());
    assert!(!first_lights.is_empty());
    assert_eq!(first_crystals.len(), first_lights.len());
    assert_eq!(first_vegetation.len(), 4);
    assert!(
        first_crystals
            .keys()
            .all(|position| first_lights.contains_key(position)),
        "every physical crystal must share the exact semantic gameplay-light floor"
    );

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();

    assert_eq!(tile_count(&mut app), 0);
    assert!(!app.world().contains_resource::<VoxelMap>());
    assert!(!app.world().contains_resource::<MapAnchors>());
    assert!(!app.world().contains_resource::<SpecialMovementRegions>());
    assert!(!app.world().contains_resource::<InteriorRegions>());
    assert!(!app.world().contains_resource::<MapViewHint>());
    assert!(!app.world().contains_resource::<GenerationReport>());
    assert!(!app.world().contains_resource::<TerrainReady>());
    let light_count_after_exit = {
        let world = app.world_mut();
        let mut lights = world.query::<&GameplayLight>();
        lights.iter(world).count()
    };
    assert_eq!(light_count_after_exit, 0);
    assert!(cave_crystal_instances(&mut app).is_empty());
    assert!(cave_vegetation_instances(&mut app).is_empty());
    let point_light_count_after_exit = {
        let world = app.world_mut();
        let mut lights = world.query::<&PointLight>();
        lights.iter(world).count()
    };
    assert_eq!(point_light_count_after_exit, 0);

    enter_gameplay(&mut app);

    assert_eq!(tile_count(&mut app), first_tile_count);
    assert!(app.world().contains_resource::<TerrainReady>());
    assert!(!app.world().contains_resource::<GameplaySetupFailure>());
    let second_report = app.world().resource::<GenerationReport>();
    assert_eq!(second_report.seed, first_report.seed);
    assert_eq!(
        second_report.selected_candidate,
        first_report.selected_candidate
    );
    assert_eq!(
        second_report.valid_candidates,
        first_report.valid_candidates
    );
    assert_eq!(second_report.repair_actions, first_report.repair_actions);
    assert_eq!(
        second_report.settings_fingerprint,
        first_report.settings_fingerprint
    );
    assert_eq!(second_report.map_fingerprint, first_report.map_fingerprint);
    assert_eq!(second_report.metrics, first_report.metrics);
    assert_eq!(*app.world().resource::<MapViewHint>(), first_view);

    let second_anchors: BTreeMap<String, TilePos> = app
        .world()
        .resource::<MapAnchors>()
        .iter()
        .map(|(id, position)| (id.as_str().to_owned(), position))
        .collect();
    let second_floors: BTreeMap<TilePos, InteriorRegionId> = app
        .world()
        .resource::<InteriorRegions>()
        .surfaces()
        .collect();
    let second_roofs: BTreeMap<TilePos, InteriorRegionId> = app
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
    let second_crystals = cave_crystal_instances(&mut app);
    let second_vegetation = cave_vegetation_instances(&mut app);
    assert_eq!(second_anchors, first_anchors);
    assert_eq!(second_floors, first_floors);
    assert_eq!(second_roofs, first_roofs);
    assert_eq!(second_lights, first_lights);
    assert_eq!(second_crystals, first_crystals);
    assert_eq!(second_vegetation, first_vegetation);
}

#[test]
fn v3_crystal_ascent_teardown_and_reentry_preserve_the_complete_landmark() {
    type ObjectSnapshot = (String, TilePos, u8, Vec<(TilePos, Level)>, usize);
    type CutawaySnapshot = (TilePos, Level, InteriorRegionId);

    fn object_snapshot(app: &mut App) -> BTreeSet<ObjectSnapshot> {
        let world = app.world_mut();
        let mut objects = world.query::<(
            &ObjectInstance,
            Option<&AuthoredObjectVoxelRuns>,
            Option<&Children>,
        )>();
        objects
            .iter(world)
            .map(|(instance, occupancy, children)| {
                let point_lights = children.map_or(0, |children| {
                    children
                        .iter()
                        .filter(|child| world.get::<PointLight>(*child).is_some())
                        .count()
                });
                let occupied_runs = occupancy.map_or_else(Vec::new, |runs| {
                    runs.iter()
                        .map(|run| (run.top, run.bottom))
                        .collect::<Vec<_>>()
                });
                (
                    instance.object_id().as_str().to_owned(),
                    instance.origin(),
                    instance.rotation().steps(),
                    occupied_runs,
                    point_lights,
                )
            })
            .collect()
    }

    fn cutaway_snapshot(app: &mut App) -> BTreeSet<CutawaySnapshot> {
        let world = app.world_mut();
        let mut cutaways = world.query::<(&TilePos, &RunBottom, &CutawayOccluder)>();
        cutaways
            .iter(world)
            .map(|(top, bottom, cutaway)| (*top, bottom.0, cutaway.0))
            .collect()
    }

    fn gameplay_light_snapshot(app: &mut App) -> Vec<(TilePos, IlluminationLevel, u32)> {
        let world = app.world_mut();
        let mut lights = world.query::<(&TilePos, &GameplayLight)>();
        let mut snapshot = lights
            .iter(world)
            .map(|(position, light)| (*position, light.level, light.radius))
            .collect::<Vec<_>>();
        snapshot.sort_unstable();
        snapshot
    }

    fn point_light_count(app: &mut App) -> usize {
        let world = app.world_mut();
        let mut lights = world.query::<&PointLight>();
        lights.iter(world).count()
    }

    fn grid_count(app: &mut App) -> usize {
        let world = app.world_mut();
        let mut grids = world.query_filtered::<Entity, With<HexGrid>>();
        grids.iter(world).count()
    }

    fn assert_same_deterministic_report(actual: &GenerationReport, expected: &GenerationReport) {
        assert_eq!(actual.generator_version, expected.generator_version);
        assert_eq!(actual.seed, expected.seed);
        assert_eq!(actual.selected_candidate, expected.selected_candidate);
        assert_eq!(actual.candidates_evaluated, expected.candidates_evaluated);
        assert_eq!(actual.valid_candidates, expected.valid_candidates);
        assert_eq!(actual.repair_rounds, expected.repair_rounds);
        assert_eq!(actual.repair_actions, expected.repair_actions);
        assert_eq!(actual.used_fallback, expected.used_fallback);
        assert_eq!(actual.settings_fingerprint, expected.settings_fingerprint);
        assert_eq!(
            actual.semantic_plan_fingerprint,
            expected.semantic_plan_fingerprint
        );
        assert_eq!(actual.map_fingerprint, expected.map_fingerprint);
        assert_eq!(actual.metrics, expected.metrics);
        assert_eq!(actual.recipe_metrics, expected.recipe_metrics);
        assert_eq!(actual.notes, expected.notes);
    }

    let mut app = v3_crystal_ascent_app();
    enter_gameplay(&mut app);

    assert!(
        app.world().contains_resource::<TerrainReady>(),
        "Crystal Ascent setup failed: {:?}",
        app.world()
            .get_resource::<GameplaySetupFailure>()
            .map(|failure| failure.reason.as_str())
    );
    let first_tile_count = tile_count(&mut app);
    let first_report = app.world().resource::<GenerationReport>().clone();
    assert_eq!(first_report.generator_version, 3);
    assert_eq!(first_report.seed, 1_592_598_566);
    assert!(first_report.semantic_plan_fingerprint.is_some());
    assert!(matches!(
        &first_report.recipe_metrics,
        Some(ProceduralRecipeMetrics::CrystalAscent(_))
    ));
    if let Some(ProceduralRecipeMetrics::CrystalAscent(metrics)) = &first_report.recipe_metrics {
        assert_eq!(metrics.landings, 18);
        assert_eq!(metrics.crystal_fixtures, 19);
        assert_eq!(metrics.gameplay_lights, 38);
        assert_eq!(metrics.rise_levels, 144);
    }
    let first_view = *app.world().resource::<MapViewHint>();
    let first_anchors = app
        .world()
        .resource::<MapAnchors>()
        .iter()
        .map(|(id, position)| (id.as_str().to_owned(), position))
        .collect::<BTreeMap<_, _>>();
    let first_floors = app
        .world()
        .resource::<InteriorRegions>()
        .surfaces()
        .collect::<BTreeMap<_, _>>();
    let first_roofs = app
        .world()
        .resource::<InteriorRegions>()
        .roof_voxels()
        .collect::<BTreeMap<_, _>>();
    let first_cutaways = cutaway_snapshot(&mut app);
    let first_lights = gameplay_light_snapshot(&mut app);
    let first_objects = object_snapshot(&mut app);

    assert_eq!(
        first_anchors
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "party_start",
            "hostile_start",
            "crystal_ascent.lower_entry",
            "crystal_ascent.bottom_chamber",
            "crystal_ascent.upper_exit",
            "crystal_ascent.mid_flight",
            "crystal_ascent.corner_landing",
            "crystal_ascent.upper_contraction",
        ])
    );
    assert_eq!(
        first_anchors.get("party_start"),
        first_anchors.get("crystal_ascent.lower_entry")
    );
    assert_eq!(
        first_anchors.get("hostile_start"),
        first_anchors.get("crystal_ascent.upper_exit")
    );
    assert!(matches!(
        (
            first_anchors.get("party_start"),
            first_anchors.get("hostile_start")
        ),
        (Some(lower), Some(upper)) if upper.level.saturating_sub(lower.level) == 144
    ));
    assert_eq!(first_lights.len(), 38);
    assert_eq!(
        first_objects
            .iter()
            .filter(|(id, _, _, _, _)| id.starts_with("prop/crystal-"))
            .count(),
        19
    );
    assert_eq!(
        first_objects
            .iter()
            .filter(|(_, _, _, occupied_runs, _)| !occupied_runs.is_empty())
            .count(),
        1,
        "only the cathedral heart may publish authored gameplay occupancy"
    );
    assert_eq!(
        first_objects
            .iter()
            .filter(|(id, _, _, occupied_runs, point_lights)| {
                id == "prop/crystal-cathedral-heart"
                    && !occupied_runs.is_empty()
                    && *point_lights == 4
            })
            .count(),
        1
    );
    assert_eq!(
        first_objects
            .iter()
            .filter(|(id, _, _, occupied_runs, point_lights)| {
                id.starts_with("prop/crystal-")
                    && id != "prop/crystal-cathedral-heart"
                    && occupied_runs.is_empty()
                    && *point_lights == 1
            })
            .count(),
        18,
        "every landing fixture must remain nonblocking and own one emitter"
    );
    assert_eq!(point_light_count(&mut app), 22);
    assert!(!first_floors.is_empty());
    assert!(!first_roofs.is_empty());
    assert!(!first_cutaways.is_empty());
    assert!(first_cutaways.iter().all(|(top, bottom, region)| {
        (*bottom..=top.level)
            .all(|level| first_roofs.get(&TilePos::new(top.coord, level)) == Some(region))
    }));
    assert!(first_roofs.iter().all(|(roof, region)| {
        first_cutaways.iter().any(|(top, bottom, cutaway)| {
            roof.coord == top.coord
                && (*bottom..=top.level).contains(&roof.level)
                && cutaway == region
        })
    }));

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();

    assert_eq!(tile_count(&mut app), 0);
    assert_eq!(grid_count(&mut app), 0);
    assert!(!app.world().contains_resource::<VoxelMap>());
    assert!(!app.world().contains_resource::<MapAnchors>());
    assert!(!app.world().contains_resource::<SpecialMovementRegions>());
    assert!(!app.world().contains_resource::<InteriorRegions>());
    assert!(!app.world().contains_resource::<TraversalBlockers>());
    assert!(!app.world().contains_resource::<BiomeRegions>());
    assert!(!app.world().contains_resource::<MapViewHint>());
    assert!(!app.world().contains_resource::<GenerationReport>());
    assert!(!app.world().contains_resource::<TerrainReady>());
    assert!(gameplay_light_snapshot(&mut app).is_empty());
    assert!(object_snapshot(&mut app).is_empty());
    assert!(cutaway_snapshot(&mut app).is_empty());
    assert_eq!(point_light_count(&mut app), 0);

    enter_gameplay(&mut app);

    assert_eq!(tile_count(&mut app), first_tile_count);
    assert!(app.world().contains_resource::<TerrainReady>());
    assert!(!app.world().contains_resource::<GameplaySetupFailure>());
    assert_same_deterministic_report(app.world().resource::<GenerationReport>(), &first_report);
    assert_eq!(*app.world().resource::<MapViewHint>(), first_view);
    assert_eq!(
        app.world()
            .resource::<MapAnchors>()
            .iter()
            .map(|(id, position)| (id.as_str().to_owned(), position))
            .collect::<BTreeMap<_, _>>(),
        first_anchors
    );
    assert_eq!(
        app.world()
            .resource::<InteriorRegions>()
            .surfaces()
            .collect::<BTreeMap<_, _>>(),
        first_floors
    );
    assert_eq!(
        app.world()
            .resource::<InteriorRegions>()
            .roof_voxels()
            .collect::<BTreeMap<_, _>>(),
        first_roofs
    );
    assert_eq!(cutaway_snapshot(&mut app), first_cutaways);
    assert_eq!(gameplay_light_snapshot(&mut app), first_lights);
    assert_eq!(object_snapshot(&mut app), first_objects);
}
