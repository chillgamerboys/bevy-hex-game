//! Integration tests for map construction.
//!
//! # Why these exist
//!
//! Every bug found in this codebase so far was found by a person clicking, not by
//! CI. Two are worth naming, because both were green across `cargo check`, clippy,
//! the unit tests, and every CI check:
//!
//! - The player spawned at ground level and **sank into the terrain**, because it
//!   read tile entities in the same schedule that created them. `Commands`-spawned
//!   entities are not queryable until the queue is applied, so ordering alone was
//!   not enough — the sets needed a sync point between them.
//! - A global observer **panicked on a menu click**, because it took a resource that
//!   only exists during gameplay.
//!
//! Neither is visible to a compiler. Both are visible to a test that runs the
//! schedule and looks at the world afterwards.
//!
//! # What these deliberately do not cover
//!
//! They run headless, with no renderer. A black skybox, a wrong colour, or a mesh at
//! the wrong scale still will not be caught here — only by looking at the window.
//! These raise the floor; they do not replace running the game.

use bevy::app::PluginsState;
use bevy::asset::AssetPlugin;
use bevy::ecs::reflect::AppTypeRegistry;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;

use std::any::TypeId;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{Duration, Instant};

use hex_assets::GameAssets;
use hex_assets::{
    ArtPalette, ObjectBlueprint, ObjectCatalogFile, ObjectInstance, PaletteSwatch,
    RuntimeArtCatalog, SrgbColor, Substance, SubstanceFile, SubstanceTable, SwatchId,
    VoxelStyleCatalog,
};
use hex_core::{
    BiomeRegionId, BiomeRegions, CanopyOccluder, CutawayOccluder, GameplayLight, GameplaySetup,
    GameplaySetupFailure, Headroom, HexCoord, HexGrid, HexSpan, HexTile, InteriorRegionId,
    InteriorRegions, Level, MapAnchorId, MapAnchors, MapViewHint, PresentationOcclusion,
    ResolvedMapSeed, RunBottom, Screen, SpecialMovementRegion, SpecialMovementRegions, SubstanceId,
    TerrainEdit, TerrainReady, TilePos, TraversalBlockers, MAX_HEADROOM,
};
use hex_map::{
    CavesReportMetrics, CrossingSettings, EnvironmentSettings, GenerationReport, HillsSettings,
    LandformSettings, LayeredSkyIslandsSettings, LinkedIslandsSettings, MapSettings,
    MountainsSettings, PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings, PatchSpec,
    PerlinSettings, PerlinStepSettings, ProceduralRecipeMetrics, ProceduralSettings,
    ProceduralV1Settings, ProceduralV2Settings, ProceduralV3Settings, Ring19Metrics, Ring7Metrics,
    SkyIslandsSettings, SubstanceRun, TacticalMetrics, TacticalSettings, TerrainSettings,
    V2EnvironmentSettings, V2HillsSettings, V2RecipeSettings, V3CavesSettings,
    V3DeepForestSettings, V3EnvironmentSettings, V3ForestSettings, V3FortSettings, V3HillsSettings,
    V3LayoutSettings, V3RecipeSettings, V3WaterfallSettings, VoxelMap,
};

/// Radius used by the tests. Small enough to stay fast, large enough that the
/// tile-count formula is a meaningful check.
const TEST_RADIUS: u32 = 4;

/// Builds a headless app with the map wired up and settings already present.
///
/// Settings are inserted directly rather than loaded from RON: this is testing
/// terrain construction, not the asset pipeline, and a test that depends on file IO
/// fails for reasons that have nothing to do with what it is checking.
fn test_app() -> App {
    let mut app = App::new();

    app.add_plugins((MinimalPlugins, AssetPlugin::default(), StatesPlugin));
    app.init_asset::<Mesh>();
    app.init_asset::<StandardMaterial>();
    app.init_state::<Screen>();

    // The real binary configures these in `AppPlugin`. Repeating it here rather than
    // depending on `hex_game` keeps the test on the crate it is about — and means
    // this test would still catch a missing sync point if the binary's wiring were
    // deleted entirely.
    app.configure_sets(
        OnEnter(Screen::Gameplay),
        (
            GameplaySetup::Resources,
            GameplaySetup::Terrain,
            GameplaySetup::Actors,
            GameplaySetup::Perception,
            GameplaySetup::View,
            GameplaySetup::Finalize,
        )
            .chain(),
    );

    // Default handles rather than real assets. Tile *placement* is what these tests
    // check, and it does not depend on a mesh having loaded — dragging in file IO
    // would only add a way for them to fail for unrelated reasons.
    app.insert_resource(GameAssets {
        hex_tile: Handle::default(),
        player_pieces: [Handle::default(), Handle::default()],
    });

    app.insert_resource(substance_table());

    app.insert_resource(MapSettings {
        grid_radius: TEST_RADIUS,
        level_height: 0.4,
        terrain: TerrainSettings::Perlin(PerlinSettings {
            seed: Some(20_260_725),
            // Taller than the shipped default. The banding puts dirt in the top two
            // levels, so shallow terrain produces nothing but one-voxel runs — and a
            // one-voxel run cannot be split, only removed. Digging needs depth to be
            // worth testing.
            steps: vec![PerlinStepSettings {
                x_freq: 0.035,
                y_freq: 0.05,
                magnitude: 14.0,
            }],
        }),
    });

    app.add_plugins(hex_map::grid::plugin);

    while app.plugins_state() != PluginsState::Cleaned {
        app.finish();
        app.cleanup();
    }
    app
}

/// The substances the generator expects, built directly rather than loaded from RON.
///
/// The asset pipeline has its own tests; depending on it here would only add a way
/// for these to fail for unrelated reasons.
fn substance_table() -> SubstanceTable {
    substance_table_without(None)
}

fn substance_table_without(omitted: Option<&str>) -> SubstanceTable {
    substance_table_fixture(omitted)
}

#[expect(
    clippy::expect_used,
    reason = "invalid compile-time fixture data should fail the integration test immediately"
)]
fn substance_table_fixture(omitted_substance: Option<&str>) -> SubstanceTable {
    let swatch = SwatchId::new("test/neutral").expect("the fixture swatch id should be valid");
    let foam = SwatchId::new("liquid/foam").expect("the foam swatch id should be valid");
    let swatches = BTreeMap::from([
        (
            foam,
            PaletteSwatch::new(
                "Water Foam",
                SrgbColor::new(0.896_243_8, 0.959_346_6, 0.991_156_4)
                    .expect("the fixture foam color should be valid"),
                BTreeSet::from(["test".to_owned()]),
            )
            .expect("the fixture foam swatch should be valid"),
        ),
        (
            swatch.clone(),
            PaletteSwatch::new(
                "Test Neutral",
                SrgbColor::new(0.5, 0.5, 0.5).expect("the fixture color should be valid"),
                BTreeSet::from(["test".to_owned()]),
            )
            .expect("the fixture swatch should be valid"),
        ),
    ]);
    let palette = ArtPalette::new(swatches).expect("the fixture palette should be valid");
    let mut substances = bevy::platform::collections::HashMap::default();
    for (name, solid, diggable) in [
        ("air", false, false),
        ("bedrock", true, false),
        ("dirt", true, true),
        ("grass", true, true),
        ("stone", true, true),
        ("gravel", true, true),
        ("water", false, true),
        ("metal", true, true),
        ("worked_stone", true, true),
        ("snow", true, true),
        ("ice", true, true),
        ("basalt", true, true),
        ("lava", false, true),
    ] {
        if omitted_substance == Some(name) {
            continue;
        }
        substances.insert(
            name.to_owned(),
            if name == "air" {
                Substance::invisible(solid, diggable)
            } else {
                Substance::from_swatch(swatch.clone(), solid, diggable)
            },
        );
    }
    SubstanceTable::from_file(&SubstanceFile { substances }, &palette)
        .expect("the fixture substances should resolve through the fixture palette")
}

fn runtime_art_catalog() -> RuntimeArtCatalog {
    runtime_art_catalog_without(None)
}

#[expect(
    clippy::expect_used,
    reason = "invalid compile-time art fixtures should fail the integration test immediately"
)]
fn runtime_art_catalog_without(omitted_object: Option<&str>) -> RuntimeArtCatalog {
    let palette: ArtPalette = ron::from_str(include_str!("../../../assets/art/palette.ron"))
        .expect("tracked art palette should parse");
    let styles: VoxelStyleCatalog =
        ron::from_str(include_str!("../../../assets/art/voxel_styles.ron"))
            .expect("tracked voxel styles should parse");
    let mut objects = BTreeMap::new();
    for source in [
        include_str!("../../../assets/art/objects/plant/small-broadleaf.ron"),
        include_str!("../../../assets/art/objects/plant/tall-narrow.ron"),
        include_str!("../../../assets/art/objects/plant/old-growth.ron"),
        include_str!("../../../assets/art/objects/plant/snowy-small-broadleaf.ron"),
        include_str!("../../../assets/art/objects/plant/snowy-tall-narrow.ron"),
        include_str!("../../../assets/art/objects/plant/snowy-old-growth.ron"),
        include_str!("../../../assets/art/objects/prop/cave-lichen.ron"),
        include_str!("../../../assets/art/objects/prop/cave-moss.ron"),
        include_str!("../../../assets/art/objects/prop/grass-tuft.ron"),
        include_str!("../../../assets/art/objects/prop/snowy-grass-tuft.ron"),
        include_str!("../../../assets/art/objects/prop/crystal-low-cluster.ron"),
        include_str!("../../../assets/art/objects/prop/crystal-branched.ron"),
        include_str!("../../../assets/art/objects/prop/crystal-spire.ron"),
    ] {
        let blueprint: ObjectBlueprint =
            ron::from_str(source).expect("tracked object blueprint should parse");
        if omitted_object != Some(blueprint.id.as_str()) {
            objects.insert(blueprint.id.clone(), blueprint);
        }
    }
    let manifest = ObjectCatalogFile::new(objects.keys().cloned())
        .expect("fixture object ids should form a valid manifest");
    RuntimeArtCatalog::from_sources(&palette, &styles, &manifest, objects)
        .expect("tracked runtime art catalog should resolve")
}

/// Runs the app until it has entered gameplay and the world has settled.
fn enter_gameplay(app: &mut App) {
    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Gameplay);
    // Two updates: one to apply the transition and run `OnEnter`, one to flush the
    // commands it queued so the entities are queryable.
    app.update();
    app.update();
}

fn procedural_app() -> App {
    let mut app = test_app();
    app.insert_resource(MapSettings {
        grid_radius: 12,
        level_height: 0.4,
        terrain: TerrainSettings::Procedural(ProceduralSettings::V1(ProceduralV1Settings {
            landform: LandformSettings::Hills(HillsSettings {
                valley_level: 15,
                max_relief: 8,
                hills_per_bank: 3,
            }),
            environment: EnvironmentSettings::TemperateGrassland,
            tactical: TacticalSettings::Crossing(CrossingSettings {
                barrier_half_width: 1,
                bed_level: 12,
                hazard_bottom: 13,
                hazard_top: 14,
                bridge_level: 16,
            }),
        })),
    });
    app.insert_resource(ResolvedMapSeed(20_260_726));
    app
}

fn v2_hills_app() -> App {
    let mut app = procedural_app();
    app.insert_resource(MapSettings {
        grid_radius: 12,
        level_height: 0.4,
        terrain: TerrainSettings::Procedural(ProceduralSettings::V2(ProceduralV2Settings {
            environment: V2EnvironmentSettings::TemperateGrassland,
            recipe: V2RecipeSettings::Hills(V2HillsSettings {
                valley_level: 15,
                max_relief: 8,
                hills_per_bank: 3,
            }),
        })),
    });
    app
}

fn v3_hills_app() -> App {
    let mut app = procedural_app();
    app.insert_resource(runtime_art_catalog());
    app.insert_resource(MapSettings {
        grid_radius: 12,
        level_height: 0.4,
        terrain: TerrainSettings::Procedural(ProceduralSettings::V3(ProceduralV3Settings {
            layout: V3LayoutSettings::Single(PatchSpec {
                environment: V3EnvironmentSettings::TemperateGrassland,
                recipe: V3RecipeSettings::Hills(V3HillsSettings {
                    valley_level: 15,
                    max_relief: 8,
                    hills_per_bank: 3,
                }),
                overlays: Vec::new(),
                mask: PatchMaskSettings::WholeWorld,
                edges: PatchEdgesSettings {
                    east: PatchEdgeContractSettings::WorldBoundary,
                    south_east: PatchEdgeContractSettings::WorldBoundary,
                    south_west: PatchEdgeContractSettings::WorldBoundary,
                    west: PatchEdgeContractSettings::WorldBoundary,
                    north_west: PatchEdgeContractSettings::WorldBoundary,
                    north_east: PatchEdgeContractSettings::WorldBoundary,
                },
            }),
        })),
    });
    app
}

fn v3_frozen_hills_app() -> App {
    let mut app = v3_hills_app();
    let mut settings = app.world_mut().resource_mut::<MapSettings>();
    let TerrainSettings::Procedural(ProceduralSettings::V3(v3)) = &mut settings.terrain else {
        unreachable!("V3 Hills fixture")
    };
    let V3LayoutSettings::Single(patch) = &mut v3.layout else {
        unreachable!("Single Hills fixture")
    };
    patch.environment = V3EnvironmentSettings::Frozen;
    let V3RecipeSettings::Hills(hills) = &mut patch.recipe else {
        unreachable!("Hills fixture")
    };
    hills.max_relief = 12;
    app
}

fn v3_waterfall_app() -> App {
    let mut app = procedural_app();
    app.insert_resource(runtime_art_catalog());
    app.insert_resource(MapSettings {
        grid_radius: 12,
        level_height: 0.4,
        terrain: TerrainSettings::Procedural(ProceduralSettings::V3(ProceduralV3Settings {
            layout: V3LayoutSettings::Single(PatchSpec {
                environment: V3EnvironmentSettings::TemperateGrassland,
                recipe: V3RecipeSettings::Waterfall(V3WaterfallSettings),
                overlays: Vec::new(),
                mask: PatchMaskSettings::WholeWorld,
                edges: PatchEdgesSettings {
                    east: PatchEdgeContractSettings::WorldBoundary,
                    south_east: PatchEdgeContractSettings::WorldBoundary,
                    south_west: PatchEdgeContractSettings::WorldBoundary,
                    west: PatchEdgeContractSettings::WorldBoundary,
                    north_west: PatchEdgeContractSettings::WorldBoundary,
                    north_east: PatchEdgeContractSettings::WorldBoundary,
                },
            }),
        })),
    });
    app.insert_resource(ResolvedMapSeed(771_203_419));
    app
}

#[expect(
    clippy::expect_used,
    reason = "the tracked Volcano world is a compile-time integration fixture"
)]
fn v3_volcano_app() -> App {
    let mut app = procedural_app();
    let settings: MapSettings = ron::from_str(include_str!(
        "../../../assets/config/worlds/procedural-volcanic.ron"
    ))
    .expect("tracked Volcano settings should parse");
    app.insert_resource(settings);
    app.insert_resource(ResolvedMapSeed(444_211_238));
    app
}

fn v3_forest_app() -> App {
    let mut app = procedural_app();
    app.insert_resource(runtime_art_catalog());
    app.insert_resource(MapSettings {
        grid_radius: 12,
        level_height: 0.4,
        terrain: TerrainSettings::Procedural(ProceduralSettings::V3(ProceduralV3Settings {
            layout: V3LayoutSettings::Single(PatchSpec {
                environment: V3EnvironmentSettings::TemperateGrassland,
                recipe: V3RecipeSettings::Forest(V3ForestSettings),
                overlays: Vec::new(),
                mask: PatchMaskSettings::WholeWorld,
                edges: PatchEdgesSettings {
                    east: PatchEdgeContractSettings::WorldBoundary,
                    south_east: PatchEdgeContractSettings::WorldBoundary,
                    south_west: PatchEdgeContractSettings::WorldBoundary,
                    west: PatchEdgeContractSettings::WorldBoundary,
                    north_west: PatchEdgeContractSettings::WorldBoundary,
                    north_east: PatchEdgeContractSettings::WorldBoundary,
                },
            }),
        })),
    });
    app.insert_resource(ResolvedMapSeed(381_654_729));
    app
}

fn v3_deep_forest_app() -> App {
    let mut app = procedural_app();
    app.insert_resource(runtime_art_catalog());
    app.insert_resource(MapSettings {
        grid_radius: 12,
        level_height: 0.4,
        terrain: TerrainSettings::Procedural(ProceduralSettings::V3(ProceduralV3Settings {
            layout: V3LayoutSettings::Single(PatchSpec {
                environment: V3EnvironmentSettings::TemperateGrassland,
                recipe: V3RecipeSettings::DeepForest(V3DeepForestSettings {
                    base_level: 15,
                    max_relief: 4,
                    blocker_coverage_percent: 30,
                    clearing_count: 3,
                }),
                overlays: Vec::new(),
                mask: PatchMaskSettings::WholeWorld,
                edges: PatchEdgesSettings {
                    east: PatchEdgeContractSettings::WorldBoundary,
                    south_east: PatchEdgeContractSettings::WorldBoundary,
                    south_west: PatchEdgeContractSettings::WorldBoundary,
                    west: PatchEdgeContractSettings::WorldBoundary,
                    north_west: PatchEdgeContractSettings::WorldBoundary,
                    north_east: PatchEdgeContractSettings::WorldBoundary,
                },
            }),
        })),
    });
    app.insert_resource(ResolvedMapSeed(1_592_598_566));
    app
}

fn v3_fort_app() -> App {
    let mut app = procedural_app();
    app.insert_resource(MapSettings {
        grid_radius: 12,
        level_height: 0.4,
        terrain: TerrainSettings::Procedural(ProceduralSettings::V3(ProceduralV3Settings {
            layout: V3LayoutSettings::Single(PatchSpec {
                environment: V3EnvironmentSettings::TemperateGrassland,
                recipe: V3RecipeSettings::Fort(V3FortSettings),
                overlays: Vec::new(),
                mask: PatchMaskSettings::WholeWorld,
                edges: PatchEdgesSettings {
                    east: PatchEdgeContractSettings::WorldBoundary,
                    south_east: PatchEdgeContractSettings::WorldBoundary,
                    south_west: PatchEdgeContractSettings::WorldBoundary,
                    west: PatchEdgeContractSettings::WorldBoundary,
                    north_west: PatchEdgeContractSettings::WorldBoundary,
                    north_east: PatchEdgeContractSettings::WorldBoundary,
                },
            }),
        })),
    });
    app.insert_resource(ResolvedMapSeed(640_367_719));
    app
}

fn sky_islands_app() -> App {
    let mut app = procedural_app();
    app.insert_resource(MapSettings {
        grid_radius: 12,
        level_height: 0.4,
        terrain: TerrainSettings::Procedural(ProceduralSettings::V1(ProceduralV1Settings {
            landform: LandformSettings::SkyIslands(SkyIslandsSettings {
                surface_level: 15,
                island_radius: 3,
            }),
            environment: EnvironmentSettings::TemperateGrassland,
            tactical: TacticalSettings::LinkedIslands(LinkedIslandsSettings { bridge_width: 2 }),
        })),
    });
    app
}

fn v2_layered_sky_app() -> App {
    let mut app = procedural_app();
    app.insert_resource(MapSettings {
        grid_radius: 12,
        level_height: 0.4,
        terrain: TerrainSettings::Procedural(ProceduralSettings::V2(ProceduralV2Settings {
            environment: V2EnvironmentSettings::TemperateGrassland,
            recipe: V2RecipeSettings::LayeredSkyIslands(LayeredSkyIslandsSettings {
                ground: V2HillsSettings {
                    valley_level: 15,
                    max_relief: 8,
                    hills_per_bank: 3,
                },
                min_clearance: 22,
                upper_coverage_percent: 24,
            }),
        })),
    });
    app
}

fn v2_mountains_app() -> App {
    let mut app = procedural_app();
    app.insert_resource(MapSettings {
        grid_radius: 12,
        level_height: 0.4,
        terrain: TerrainSettings::Procedural(ProceduralSettings::V2(ProceduralV2Settings {
            environment: V2EnvironmentSettings::Frozen,
            recipe: V2RecipeSettings::Mountains(MountainsSettings {
                base_level: 15,
                relief: 24,
                peak_count: 7,
            }),
        })),
    });
    app
}

fn v3_caves_app() -> App {
    let mut app = v3_caves_app_without_art_catalog();
    app.insert_resource(runtime_art_catalog());
    app
}

fn v3_caves_app_without_art_catalog() -> App {
    let mut app = procedural_app();
    app.insert_resource(MapSettings {
        grid_radius: 12,
        level_height: 0.4,
        terrain: TerrainSettings::Procedural(ProceduralSettings::V3(ProceduralV3Settings {
            layout: V3LayoutSettings::Single(PatchSpec {
                environment: V3EnvironmentSettings::Rocky,
                recipe: V3RecipeSettings::Caves(V3CavesSettings {
                    surface_level: 17,
                    cave_floor_level: 6,
                    chamber_count: 12,
                }),
                overlays: Vec::new(),
                mask: PatchMaskSettings::WholeWorld,
                edges: PatchEdgesSettings {
                    east: PatchEdgeContractSettings::WorldBoundary,
                    south_east: PatchEdgeContractSettings::WorldBoundary,
                    south_west: PatchEdgeContractSettings::WorldBoundary,
                    west: PatchEdgeContractSettings::WorldBoundary,
                    north_west: PatchEdgeContractSettings::WorldBoundary,
                    north_east: PatchEdgeContractSettings::WorldBoundary,
                },
            }),
        })),
    });
    app.insert_resource(ResolvedMapSeed(736_283_041));
    app
}

#[expect(
    clippy::expect_used,
    reason = "the tracked Ring7 review world is a compile-time integration fixture"
)]
fn v3_ring7_app() -> App {
    let mut app = test_app();
    let settings: MapSettings = ron::from_str(include_str!(
        "../../../assets/config/worlds/procedural-ring7.ron"
    ))
    .expect("tracked Ring7 settings should parse");
    app.insert_resource(settings);
    app.insert_resource(ResolvedMapSeed(703_700_113));
    app.insert_resource(runtime_art_catalog());
    app
}

#[expect(
    clippy::expect_used,
    reason = "the tracked Two Rings review world is a compile-time integration fixture"
)]
fn v3_ring19_app() -> App {
    let mut app = test_app();
    let settings: MapSettings = ron::from_str(include_str!(
        "../../../assets/config/worlds/procedural-two-rings.ron"
    ))
    .expect("tracked Two Rings settings should parse");
    app.insert_resource(settings);
    app.insert_resource(ResolvedMapSeed(1_592_598_566));
    app.insert_resource(runtime_art_catalog());
    app
}

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
fn v3_forest_protects_feature_roots_and_rebuilds_them_deterministically() {
    let mut app = v3_forest_app();
    enter_gameplay(&mut app);

    let initial_roots: BTreeMap<_, _> = feature_roots(&mut app)
        .into_iter()
        .map(|(entity, _kind, position, _parent)| (position, entity))
        .collect();
    let tree_root = app
        .world()
        .resource::<TraversalBlockers>()
        .iter()
        .next()
        .expect("Forest should publish a tree blocker");
    let original_substance = app.world().resource::<VoxelMap>().get(tree_root);
    let first_grid = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("Forest grid should exist");

    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: tree_root });
    app.update();
    app.update();

    assert_eq!(
        app.world().resource::<VoxelMap>().get(tree_root),
        original_substance,
        "static feature support was edited without feature reprojection"
    );
    let unchanged_grid = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("Forest grid should remain");
    assert_eq!(unchanged_grid, first_grid, "rejected edit rebuilt the grid");
    let unchanged_roots: BTreeMap<_, _> = feature_roots(&mut app)
        .into_iter()
        .map(|(entity, _kind, position, _parent)| (position, entity))
        .collect();
    assert_eq!(unchanged_roots, initial_roots);

    let blocker_coords: BTreeSet<_> = app
        .world()
        .resource::<TraversalBlockers>()
        .iter()
        .map(|position| position.coord)
        .collect();
    let unrelated = {
        let world = app.world();
        let table = world.resource::<SubstanceTable>();
        world
            .resource::<VoxelMap>()
            .columns()
            .filter(|(coord, _column)| !blocker_coords.contains(coord))
            .find_map(|(coord, column)| {
                hex_map::runs(column)
                    .into_iter()
                    .rev()
                    .find(|run| table.is_solid(run.substance) && table.is_diggable(run.substance))
                    .map(|run| TilePos::new(coord, run.top - 1))
            })
            .expect("Forest should have unrelated diggable terrain")
    };
    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: unrelated });
    app.update();
    app.update();

    let rebuilt_grid = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("rebuilt Forest grid should exist");
    assert_ne!(rebuilt_grid, first_grid);
    let rebuilt_roots = feature_roots(&mut app);
    assert_eq!(
        rebuilt_roots
            .iter()
            .map(|(_entity, _kind, position, _parent)| *position)
            .collect::<BTreeSet<_>>(),
        initial_roots.keys().copied().collect()
    );
    assert!(rebuilt_roots
        .iter()
        .all(|(_entity, _kind, _position, parent)| *parent == rebuilt_grid));
    assert!(initial_roots
        .values()
        .all(|entity| app.world().get_entity(*entity).is_err()));

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();
    assert!(feature_roots(&mut app).is_empty());
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
fn clearing_a_tagged_surface_prunes_its_exact_membership() {
    let mut app = sky_islands_app();
    enter_gameplay(&mut app);
    let target = app
        .world()
        .resource::<SpecialMovementRegions>()
        .iter()
        .map(|(position, _)| position)
        .next()
        .expect("sky islands should publish optional surfaces");

    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: target });
    app.update();
    app.update();

    assert_eq!(
        app.world().resource::<SpecialMovementRegions>().get(target),
        None
    );
}

#[test]
fn terrain_edits_prune_stale_interior_floor_and_roof_voxel_metadata() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    let target = {
        let world = app.world_mut();
        let table = world.resource::<SubstanceTable>().clone();
        let mut tiles = world.query::<(&TilePos, &SubstanceId, &Headroom)>();
        tiles
            .iter(world)
            .find(|(_, substance, headroom)| table.is_diggable(**substance) && headroom.0 >= 2)
            .map(|(position, _, _)| *position)
            .expect("the authored map should have a clearable exposed surface")
    };
    let region = InteriorRegionId(4);
    let mut interiors = InteriorRegions::new();
    interiors.insert_surface(target, region);
    interiors.insert_roof_voxel(target, region);
    app.insert_resource(interiors);

    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: target });
    app.update();
    app.update();

    let interiors = app.world().resource::<InteriorRegions>();
    assert_eq!(interiors.get(target), None);
    assert_eq!(interiors.roof_region(target), None);
}

fn diggable_run(app: &App, minimum_levels: Level) -> Option<(HexCoord, SubstanceRun)> {
    let world = app.world();
    let table = world.resource::<SubstanceTable>();
    let map = world.resource::<VoxelMap>();
    map.columns().find_map(|(coord, column)| {
        hex_map::runs(column)
            .into_iter()
            .find(|run| table.is_diggable(run.substance) && run.levels() >= minimum_levels)
            .map(|run| (coord, run))
    })
}

fn install_roof_metadata(
    app: &mut App,
    coord: HexCoord,
    run: SubstanceRun,
    region: InteriorRegionId,
) {
    let mut interiors = InteriorRegions::new();
    for level in run.bottom..run.top {
        interiors.insert_roof_voxel(TilePos::new(coord, level), region);
    }
    app.insert_resource(interiors);
}

#[test]
fn splitting_a_roof_reprojects_cutaway_onto_both_remaining_runs() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    let (coord, roof) =
        diggable_run(&app, 3).expect("the authored map should contain a tall diggable run");
    let region = InteriorRegionId(8);
    install_roof_metadata(&mut app, coord, roof, region);
    let split_level = roof.bottom + roof.levels() / 2;

    app.world_mut().write_message(TerrainEdit::Clear {
        pos: TilePos::new(coord, split_level),
    });
    app.update();
    app.update();

    let interiors = app.world().resource::<InteriorRegions>();
    assert_eq!(
        interiors.roof_region(TilePos::new(coord, split_level)),
        None
    );
    assert_eq!(
        interiors.roof_region(TilePos::new(coord, split_level - 1)),
        Some(region)
    );
    assert_eq!(
        interiors.roof_region(TilePos::new(coord, roof.top - 1)),
        Some(region)
    );

    let world = app.world_mut();
    let mut tiles = world.query::<(&HexCoord, &TilePos, Option<&CutawayOccluder>)>();
    let projected: HashMap<TilePos, Option<InteriorRegionId>> = tiles
        .iter(world)
        .filter(|(tile_coord, _, _)| **tile_coord == coord)
        .map(|(_, position, cutaway)| (*position, cutaway.map(|tag| tag.0)))
        .collect();
    assert_eq!(
        projected.get(&TilePos::new(coord, split_level - 1)),
        Some(&Some(region)),
        "the lower roof fragment lost its cutaway projection"
    );
    assert_eq!(
        projected.get(&TilePos::new(coord, roof.top - 1)),
        Some(&Some(region)),
        "the upper roof fragment lost its cutaway projection"
    );
}

#[test]
fn replacing_roof_material_does_not_transfer_its_cutaway_tag() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    let (coord, roof) =
        diggable_run(&app, 2).expect("the authored map should contain a tall diggable run");
    let region = InteriorRegionId(9);
    install_roof_metadata(&mut app, coord, roof, region);
    let replaced = TilePos::new(coord, roof.top - 1);
    let replacement = app
        .world()
        .resource::<SubstanceTable>()
        .id("metal")
        .expect("the test substance table should contain metal");
    assert_ne!(replacement, roof.substance);

    app.world_mut().write_message(TerrainEdit::Set {
        pos: replaced,
        substance: replacement,
    });
    app.update();
    app.update();

    let interiors = app.world().resource::<InteriorRegions>();
    assert_eq!(interiors.roof_region(replaced), None);
    assert_eq!(
        interiors.roof_region(TilePos::new(coord, roof.top - 2)),
        Some(region)
    );

    let world = app.world_mut();
    let mut tiles = world.query::<(&HexCoord, &TilePos, &SubstanceId, Option<&CutawayOccluder>)>();
    let replacement_run = tiles
        .iter(world)
        .find(|(tile_coord, position, _, _)| **tile_coord == coord && **position == replaced)
        .expect("the replacement material should render as its own run");
    assert_eq!(*replacement_run.2, replacement);
    assert_eq!(
        replacement_run.3, None,
        "replacement material inherited a stale cutaway tag"
    );

    let remaining_roof = TilePos::new(coord, roof.top - 2);
    let original_run = tiles
        .iter(world)
        .find(|(tile_coord, position, _, _)| **tile_coord == coord && **position == remaining_roof)
        .expect("the original roof material should remain rendered");
    assert_eq!(*original_run.2, roof.substance);
    assert_eq!(
        original_run.3.map(|tag| tag.0),
        Some(region),
        "the remaining roof run lost its cutaway tag"
    );
}

fn insert_stale_generated_resources(app: &mut App) {
    let position = TilePos::new(HexCoord::ORIGIN, 7);

    let mut map = VoxelMap::new();
    map.set(position, SubstanceId(1));
    app.insert_resource(map);

    let mut anchors = MapAnchors::new();
    assert_eq!(
        anchors.insert(MapAnchorId::from("stale_anchor"), position),
        None
    );
    app.insert_resource(anchors);

    let mut special_regions = SpecialMovementRegions::new();
    assert_eq!(
        special_regions.insert(position, SpecialMovementRegion(91)),
        None
    );
    app.insert_resource(special_regions);

    let mut interiors = InteriorRegions::new();
    assert_eq!(
        interiors.insert_surface(position, InteriorRegionId(92)),
        None
    );
    assert_eq!(
        interiors.insert_roof_voxel(position.above(), InteriorRegionId(92)),
        None
    );
    app.insert_resource(interiors);

    let mut blockers = TraversalBlockers::new();
    assert!(blockers.insert(position));
    app.insert_resource(blockers);

    let mut biomes = BiomeRegions::new();
    assert_eq!(biomes.insert(position, BiomeRegionId(93)), None);
    app.insert_resource(biomes);

    app.insert_resource(MapViewHint::new((1.0, 2.0, 3.0), (0.0, 0.0, 0.0)));
    app.insert_resource(GenerationReport {
        generator_version: 99,
        seed: 99,
        selected_candidate: Some(7),
        candidates_evaluated: 8,
        valid_candidates: 1,
        repair_rounds: 0,
        repair_actions: vec!["stale".to_owned()],
        used_fallback: false,
        settings_fingerprint: 1,
        semantic_plan_fingerprint: Some(2),
        map_fingerprint: 3,
        metrics: TacticalMetrics::default(),
        recipe_metrics: None,
        elapsed_micros: 4,
        notes: vec!["stale".to_owned()],
    });
    app.insert_resource(TerrainReady);
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

fn tile_count(app: &mut App) -> usize {
    app.world_mut()
        .query_filtered::<Entity, With<HexTile>>()
        .iter(app.world())
        .count()
}

fn published_run_bounds(app: &mut App, coord: HexCoord) -> BTreeSet<(Level, Level, SubstanceId)> {
    let world = app.world_mut();
    let mut tiles = world.query_filtered::<(&TilePos, &RunBottom, &SubstanceId), With<HexTile>>();
    tiles
        .iter(world)
        .filter(|(position, _, _)| position.coord == coord)
        .map(|(position, bottom, substance)| (bottom.0, position.level, *substance))
        .collect()
}

fn assert_column_run_publication(app: &mut App, coord: HexCoord) {
    let expected: BTreeSet<_> = app
        .world()
        .resource::<VoxelMap>()
        .column(coord)
        .into_iter()
        .flat_map(hex_map::runs)
        .map(|run| (run.bottom, run.top - 1, run.substance))
        .collect();
    let published = published_run_bounds(app, coord);

    assert_eq!(
        published, expected,
        "every material run in {coord:?} must publish its exact inclusive bottom and top"
    );
}

fn object_instance_snapshot(app: &mut App) -> BTreeSet<(String, TilePos, u8)> {
    let world = app.world_mut();
    let mut objects = world.query::<&ObjectInstance>();
    objects
        .iter(world)
        .map(|instance| {
            (
                instance.object_id().as_str().to_owned(),
                instance.origin(),
                instance.rotation().steps(),
            )
        })
        .collect()
}

#[expect(
    clippy::expect_used,
    reason = "invalid generated crystal hierarchy is a broken integration-test fixture"
)]
fn cave_crystal_instances(app: &mut App) -> BTreeMap<TilePos, (String, u8)> {
    let roots: Vec<_> = {
        let world = app.world_mut();
        let mut query = world.query::<(Entity, &ObjectInstance, &Children)>();
        query
            .iter(world)
            .filter(|(_entity, instance, _children)| {
                instance.object_id().as_str().starts_with("prop/crystal-")
            })
            .map(|(entity, instance, children)| {
                (
                    entity,
                    instance.clone(),
                    children.iter().collect::<Vec<_>>(),
                )
            })
            .collect()
    };

    let mut snapshots = BTreeMap::new();
    for (root, instance, children) in roots {
        assert!(app.world().get::<GameplayLight>(root).is_none());
        assert!(app.world().get::<TilePos>(root).is_none());
        let point_lights: Vec<_> = children
            .iter()
            .filter_map(|child| {
                app.world()
                    .get::<PointLight>(*child)
                    .map(|light| (*child, light))
            })
            .collect();
        assert_eq!(
            point_lights.len(),
            1,
            "each crystal object owns one physical point light"
        );
        let (point_light_entity, point_light) = point_lights
            .first()
            .expect("the exact point-light child count was checked above");
        assert!((point_light.intensity - 4_500.0).abs() < f32::EPSILON);
        assert!((point_light.range - 4.5).abs() < f32::EPSILON);
        assert!((point_light.radius - 0.12).abs() < f32::EPSILON);
        assert!(!point_light.shadow_maps_enabled);
        assert!(!point_light.contact_shadows_enabled);
        assert!(
            app.world()
                .get::<GameplayLight>(*point_light_entity)
                .is_none(),
            "physical light must not duplicate gameplay illumination"
        );
        assert!(app.world().get::<TilePos>(*point_light_entity).is_none());

        let origin = instance.origin();
        let floor_level = origin
            .level
            .checked_sub(1)
            .expect("a generated crystal must sit one level above its floor");
        assert!(
            snapshots
                .insert(
                    TilePos::new(origin.coord, floor_level),
                    (
                        instance.object_id().as_str().to_owned(),
                        instance.rotation().steps(),
                    ),
                )
                .is_none(),
            "one authored crystal is expected per gameplay-light floor"
        );
    }
    snapshots
}

#[expect(
    clippy::expect_used,
    reason = "invalid generated cave vegetation is a broken integration-test fixture"
)]
fn cave_vegetation_instances(app: &mut App) -> BTreeMap<TilePos, (String, u8)> {
    let world = app.world_mut();
    let mut query = world.query::<(
        &Name,
        &ObjectInstance,
        Option<&HexTile>,
        Option<&CanopyOccluder>,
    )>();
    query
        .iter(world)
        .filter(|(name, _instance, _tile, _canopy)| name.as_str() == "GeneratedCaveVegetation")
        .map(|(_name, instance, tile, canopy)| {
            assert!(
                tile.is_none(),
                "cave vegetation roots must not become terrain tiles"
            );
            assert!(
                canopy.is_none(),
                "grounded cave vegetation must not publish canopy occlusion"
            );
            let origin = instance.origin();
            let floor_level = origin
                .level
                .checked_sub(1)
                .expect("cave vegetation should sit one level above its exact footing");
            (
                TilePos::new(origin.coord, floor_level),
                (
                    instance.object_id().as_str().to_owned(),
                    instance.rotation().steps(),
                ),
            )
        })
        .collect()
}

#[expect(
    clippy::expect_used,
    reason = "invalid generated cave vegetation is a broken integration-test fixture"
)]
fn cave_vegetation_non_root_visual(app: &mut App) -> (TilePos, TilePos) {
    let instance = {
        let world = app.world_mut();
        let mut query = world.query::<(&Name, &ObjectInstance)>();
        query
            .iter(world)
            .find(|(name, _instance)| name.as_str() == "GeneratedCaveVegetation")
            .map(|(_name, instance)| instance.clone())
            .expect("Caves should publish sparse vegetation")
    };
    let visual_origin = instance.origin();
    let root = TilePos::new(
        visual_origin.coord,
        visual_origin
            .level
            .checked_sub(1)
            .expect("cave vegetation should sit above its exact footing"),
    );
    let catalog = app.world().resource::<RuntimeArtCatalog>();
    let blueprint = catalog
        .object(instance.object_id())
        .expect("generated cave vegetation must resolve through the accepted art catalog");
    let mut non_root = None;
    for placement in &blueprint.placements {
        let rotated = instance
            .rotation()
            .rotate_voxel(placement.position, blueprint.origin)
            .expect("validated object rotation should remain projectable");
        let delta_q = rotated
            .q
            .checked_sub(blueprint.origin.q)
            .expect("validated local coordinates should remain projectable");
        let delta_r = rotated
            .r
            .checked_sub(blueprint.origin.r)
            .expect("validated local coordinates should remain projectable");
        let coord = HexCoord::from_axial(
            root.coord
                .x()
                .checked_add(delta_q)
                .expect("generated object q coordinate should remain in range"),
            root.coord
                .y()
                .checked_add(delta_r)
                .expect("generated object r coordinate should remain in range"),
        );
        let relative_level = rotated
            .level
            .checked_sub(blueprint.origin.level)
            .expect("validated local levels should remain projectable");
        let level = visual_origin
            .level
            .checked_add(relative_level)
            .expect("generated object level should remain in range");
        let visual = TilePos::new(coord, level);
        if visual.coord != root.coord {
            non_root = Some((root, visual));
            break;
        }
    }
    non_root.expect("tracked cave vegetation should occupy at least one non-root coordinate")
}

fn liquid_presentations(app: &mut App) -> Vec<(Entity, Entity, Pickable)> {
    let world = app.world_mut();
    let mut query = world.query::<(Entity, &Name, &ChildOf, &Pickable, Option<&HexTile>)>();
    query
        .iter(world)
        .filter(|(_entity, name, _parent, _pickable, _tile)| {
            matches!(name.as_str(), "LiquidCap" | "LiquidFallCurtain")
        })
        .map(|(entity, _name, parent, pickable, tile)| {
            assert!(
                tile.is_none(),
                "presentation entities must not become tiles"
            );
            (entity, parent.parent(), *pickable)
        })
        .collect()
}

#[expect(
    clippy::expect_used,
    reason = "a generated visual origin below its exact footing is a broken test fixture"
)]
fn feature_roots(app: &mut App) -> Vec<(Entity, String, TilePos, Entity)> {
    let world = app.world_mut();
    let level_height = world.resource::<MapSettings>().level_height;
    let mut query = world.query::<(Entity, &Name, &ObjectInstance, &ChildOf, Option<&HexTile>)>();
    query
        .iter(world)
        .filter(|(_entity, name, _instance, _parent, _tile)| {
            matches!(
                name.as_str(),
                "GeneratedTree" | "GeneratedTallGrass" | "GeneratedCaveVegetation"
            )
        })
        .map(|(entity, name, instance, parent, tile)| {
            assert!(
                tile.is_none(),
                "feature roots must not become terrain tiles"
            );
            assert!((instance.level_height() - level_height).abs() <= f32::EPSILON);
            let visual_origin = instance.origin();
            let level = visual_origin
                .level
                .checked_sub(1)
                .expect("generated visual origins should sit above their exact footing");
            (
                entity,
                name.as_str().to_owned(),
                TilePos::new(visual_origin.coord, level),
                parent.parent(),
            )
        })
        .collect()
}

#[test]
fn nonprocedural_maps_publish_an_empty_region_registry() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    assert!(app.world().resource::<SpecialMovementRegions>().is_empty());
    assert!(app.world().resource::<InteriorRegions>().is_empty());
}

/// Every column produces at least one entity, and typically several — one per
/// substance run.
#[test]
fn entering_gameplay_spawns_a_full_grid() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    // A hexagon of radius r holds 3r² + 3r + 1 columns.
    let columns = (3 * TEST_RADIUS * TEST_RADIUS + 3 * TEST_RADIUS + 1) as usize;
    assert!(
        tile_count(&mut app) >= columns,
        "every column should spawn at least one prism"
    );
}

#[test]
fn liquid_presentation_is_additive_non_pickable_and_tracks_grid_lifecycle() {
    let mut app = procedural_app();
    enter_gameplay(&mut app);

    let expected_tiles: usize = app
        .world()
        .resource::<VoxelMap>()
        .columns()
        .map(|(_coord, column)| hex_map::runs(column).len())
        .sum();
    assert_eq!(
        tile_count(&mut app),
        expected_tiles,
        "presentation geometry changed the authoritative tile count"
    );

    let first_grid = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("the first grid should exist");
    let first_presentations = liquid_presentations(&mut app);
    assert!(
        !first_presentations.is_empty(),
        "the procedural river should produce presentation caps"
    );
    assert!(first_presentations
        .iter()
        .all(|(_entity, parent, pickable)| *parent == first_grid && *pickable == Pickable::IGNORE));

    let solid_edit = {
        let world = app.world();
        let table = world.resource::<SubstanceTable>();
        world
            .resource::<VoxelMap>()
            .columns()
            .find_map(|(coord, column)| {
                hex_map::runs(column)
                    .into_iter()
                    .find(|run| table.is_solid(run.substance) && table.is_diggable(run.substance))
                    .map(|run| TilePos::new(coord, run.top - 1))
            })
            .expect("the generated map should contain diggable solid terrain")
    };
    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: solid_edit });
    app.update();
    app.update();

    let second_grid = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("the rebuilt grid should exist");
    assert_ne!(second_grid, first_grid);
    assert!(first_presentations
        .iter()
        .all(|(entity, _parent, _pickable)| app.world().get_entity(*entity).is_err()));
    let second_presentations = liquid_presentations(&mut app);
    assert!(!second_presentations.is_empty());
    assert!(
        second_presentations
            .iter()
            .all(|(_entity, parent, pickable)| *parent == second_grid
                && *pickable == Pickable::IGNORE)
    );

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();
    assert!(liquid_presentations(&mut app).is_empty());
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
fn v3_waterfall_rejects_liquid_and_support_edits_but_rebuilds_dry_terrain() {
    let mut app = v3_waterfall_app();
    enter_gameplay(&mut app);

    let (water_position, support_position, dry_position, water, support) = {
        let world = app.world();
        let map = world.resource::<VoxelMap>();
        let table = world.resource::<SubstanceTable>();
        let water = table.id("water").expect("water should exist");
        let water_position = map
            .columns()
            .find_map(|(coord, column)| {
                column.iter().enumerate().find_map(|(index, substance)| {
                    (substance == water).then(|| {
                        TilePos::new(coord, i32::try_from(index).expect("test levels fit in i32"))
                    })
                })
            })
            .expect("Waterfall should contain authored water");
        let support_position =
            TilePos::new(water_position.coord, water_position.level.saturating_sub(1));
        let support = map.get(support_position);
        let dry_position = world
            .resource::<BiomeRegions>()
            .iter()
            .map(|(position, _region)| position)
            .find(|position| {
                position.coord.y().abs() > 3
                    && table.is_solid(map.get(*position))
                    && table.is_diggable(map.get(*position))
            })
            .expect("Waterfall should contain a classified dry diggable surface");
        (
            water_position,
            support_position,
            dry_position,
            water,
            support,
        )
    };
    assert!(!support.is_air());

    let original_grid = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("Waterfall grid should exist");
    for protected in [water_position, support_position] {
        app.world_mut()
            .write_message(TerrainEdit::Clear { pos: protected });
        app.update();
        app.update();
        let current_grid = app
            .world_mut()
            .query_filtered::<Entity, With<HexGrid>>()
            .single(app.world())
            .expect("ignored edit should preserve the grid");
        assert_eq!(current_grid, original_grid);
    }
    assert_eq!(
        app.world().resource::<VoxelMap>().get(water_position),
        water
    );
    assert_eq!(
        app.world().resource::<VoxelMap>().get(support_position),
        support
    );

    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: dry_position });
    app.update();
    app.update();
    let rebuilt_grid = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("dry edit should rebuild the grid");
    assert_ne!(rebuilt_grid, original_grid);
    assert!(app
        .world()
        .resource::<VoxelMap>()
        .get(dry_position)
        .is_air());
    assert!(
        app.world()
            .resource::<BiomeRegions>()
            .get(dry_position)
            .is_none(),
        "clearing a generated surface must remove its stale exact biome membership"
    );
}

#[test]
fn the_grid_has_a_single_parent() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let grids = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .iter(app.world())
        .count();
    assert_eq!(grids, 1, "tiles should hang off exactly one grid entity");
}

/// The contract between the map and everything else: a tile carries its rendered
/// run's span, and its transform agrees with that span.
///
/// This is the invariant gameplay leans on to place a piece on a surface, and the
/// one a run-meshing change is most likely to break silently — the tiles would still
/// render, just in the wrong place.
#[test]
fn every_tile_transform_matches_its_span() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let mut query = app
        .world_mut()
        .query_filtered::<(&HexSpan, &Transform), With<HexTile>>();

    let mut checked = 0;
    for (span, transform) in query.iter(app.world()) {
        assert!(
            (transform.translation.y - span.centre()).abs() < 1e-4,
            "tile sits at {} but its span centre is {}",
            transform.translation.y,
            span.centre()
        );
        assert!(
            (transform.scale.y - span.height()).abs() < 1e-4,
            "tile is {} tall but its span is {}",
            transform.scale.y,
            span.height()
        );
        checked += 1;
    }
    assert!(checked > 0, "no tiles were checked");
}

/// Generated terrain is solid from the bedrock floor upward, with no gaps.
///
/// Digging needs something to dig through, so a column starting above ground would be
/// a hole nothing could stand in. Floating spans are legal in general — that is what
/// `HexSpan` is for — but the *generator* must not produce them.
#[test]
fn generated_terrain_is_solid_to_the_floor() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let map = app
        .world()
        .get_resource::<VoxelMap>()
        .expect("gameplay should have generated a world");

    for (coord, column) in map.columns() {
        assert!(!column.is_empty(), "{coord:?} has no ground at all");
        for level in 0..column.top() {
            assert!(
                !column.get(level).is_air(),
                "{coord:?} has a gap at level {level}; generated terrain should be solid"
            );
        }
    }
}

/// Every column has at least one level above bedrock.
///
/// Bedrock is deliberately not diggable, so a column of nothing but bedrock would be a
/// permanent hole in the world.
#[test]
fn no_column_is_bare_bedrock() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let map = app
        .world()
        .get_resource::<VoxelMap>()
        .expect("gameplay should have generated a world");

    for (coord, column) in map.columns() {
        assert!(
            column.top() >= 2,
            "{coord:?} is bare bedrock at height {}",
            column.top()
        );
    }
}

/// Entity count scales with substance *variety*, not with depth.
///
/// This is what makes voxel storage affordable. Without run-merging, a radius-20
/// world with bedrock depth would be tens of thousands of entities.
#[test]
fn entities_scale_with_runs_not_voxels() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let voxels: usize = {
        let map = app
            .world()
            .get_resource::<VoxelMap>()
            .expect("gameplay should have generated a world");
        map.columns()
            .map(|(_, column)| usize::try_from(column.top()).unwrap_or(0))
            .sum()
    };

    let tiles = tile_count(&mut app);
    assert!(
        tiles < voxels,
        "{tiles} entities for {voxels} voxels — runs are not being merged"
    );
}

/// Every tile carries the complete map/gameplay component contract.
#[test]
fn tiles_carry_the_complete_component_contract() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    {
        let registry = app.world().resource::<AppTypeRegistry>().read();
        assert!(
            registry.get(TypeId::of::<RunBottom>()).is_some(),
            "the map plugin must register the shared RunBottom component"
        );
    }

    let mut query = app.world_mut().query_filtered::<(
        &HexCoord,
        &TilePos,
        &RunBottom,
        &HexSpan,
        &SubstanceId,
        &Headroom,
    ), With<HexTile>>();

    let mut checked = 0;
    for (coord, pos, bottom, span, substance, headroom) in query.iter(app.world()) {
        assert!(!substance.is_air(), "air should not be spawned as a prism");
        assert_eq!(pos.coord, *coord, "a tile's position must match its column");
        assert!(
            bottom.0 <= pos.level,
            "a run's inclusive bottom cannot exceed its inclusive top"
        );
        assert!(span.height() > 0.0, "a tile span must have positive height");
        assert!(
            (0..=MAX_HEADROOM).contains(&headroom.0),
            "headroom must remain bounded"
        );
        checked += 1;
    }
    assert!(checked > 0, "no tiles were checked");

    let coords: Vec<_> = app
        .world()
        .resource::<VoxelMap>()
        .columns()
        .map(|(coord, _)| coord)
        .collect();
    for coord in coords {
        assert_column_run_publication(&mut app, coord);
    }
}

/// In gap-free generated terrain, only the top run of each column has headroom, and
/// under open sky it saturates.
///
/// This is the map's half of a contract gameplay cannot check for itself: a run knows
/// its own extent but nothing about what is stacked on it, so only the map can measure
/// the space above. Getting it wrong is what put the player inside the terrain and
/// left every route walking through the bedrock.
///
/// Generated terrain has no caves or overhangs, so exactly one run per column has room
/// above it and that room is open sky. A column with a bridge over it would report the
/// gap instead, which the platform test below covers.
#[test]
fn only_the_top_of_each_column_has_headroom() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let mut query = app
        .world_mut()
        .query_filtered::<(&TilePos, &Headroom), With<HexTile>>();

    let mut tops: HashMap<HexCoord, Level> = HashMap::new();
    let mut clear_per_column: HashMap<HexCoord, usize> = HashMap::new();
    let mut clear_levels: HashMap<HexCoord, Level> = HashMap::new();

    for (pos, headroom) in query.iter(app.world()) {
        let top = tops.entry(pos.coord).or_insert(pos.level);
        *top = (*top).max(pos.level);
        if headroom.0 > 0 {
            *clear_per_column.entry(pos.coord).or_insert(0) += 1;
            clear_levels.insert(pos.coord, pos.level);
            assert_eq!(
                headroom.0, MAX_HEADROOM,
                "the surface of column {:?} is under open sky and should saturate",
                pos.coord
            );
        }
    }

    assert!(!tops.is_empty(), "no tiles were checked");
    for (coord, top) in &tops {
        assert_eq!(
            clear_per_column.get(coord).copied().unwrap_or(0),
            1,
            "column {coord:?} should have exactly one run with room above it"
        );
        assert_eq!(
            clear_levels.get(coord).copied(),
            Some(*top),
            "the run with room above it in column {coord:?} should be its topmost"
        );
    }
}

/// Headroom under a platform is the size of the gap, not open sky.
///
/// This is what makes a body's size mean anything: build a roof two levels up and the
/// ground below reports 2, so a three-level body no longer fits there. Without this,
/// every surface would look infinitely tall and overhangs would be free to walk under.
#[test]
fn a_platform_overhead_reduces_the_headroom_below() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let coord = HexCoord::ORIGIN;
    let (surface, stone) = {
        let world = app.world();
        let map = world
            .get_resource::<VoxelMap>()
            .expect("a world should exist");
        let table = world
            .get_resource::<SubstanceTable>()
            .expect("a substance table should exist");
        (
            map.surface(coord).expect("the origin should have ground"),
            table.id("stone").expect("stone should be defined"),
        )
    };

    // A roof three levels above the surface leaves exactly two clear voxels between.
    let gap = 2;
    app.world_mut().write_message(TerrainEdit::Set {
        pos: TilePos::new(coord, surface + gap + 1),
        substance: stone,
    });
    app.update();
    app.update();

    assert_column_run_publication(&mut app, coord);
    let published = published_run_bounds(&mut app, coord);
    assert!(
        published.contains(&(surface + gap + 1, surface + gap + 1, stone)),
        "the one-voxel platform must publish its exact bottom and inclusive top"
    );
    assert!(
        published.iter().any(|(_, top, _)| *top == surface),
        "the ground run below the platform must retain its own published bounds"
    );

    let mut query = app
        .world_mut()
        .query_filtered::<(&TilePos, &Headroom), With<HexTile>>();
    let headroom = query
        .iter(app.world())
        .find(|(pos, _)| pos.coord == coord && pos.level == surface)
        .map(|(_, headroom)| headroom.0)
        .expect("the original surface should still be a tile");

    assert_eq!(
        headroom, gap,
        "the ground under a platform should report the gap, not open sky"
    );
}

/// Digging a voxel out of the middle of a run splits it in two, which is what makes
/// caves and tunnels fall out of the same mechanism as everything else.
///
/// The run has to be at least three levels deep. Clearing a run that is only one
/// voxel tall *removes* it rather than splitting it, so entity count goes down — a
/// first version of this test picked an arbitrary level, hit a single-voxel dirt
/// band, and failed with 156 -> 155.
#[test]
fn clearing_a_voxel_splits_a_run() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let before = tile_count(&mut app);

    // Find a run thick enough that hollowing its middle leaves material either side.
    let (target, original_run) = {
        let map = app
            .world()
            .get_resource::<VoxelMap>()
            .expect("a world should exist");
        map.columns()
            .find_map(|(coord, column)| {
                hex_map::runs(column)
                    .into_iter()
                    .find(|run| run.levels() >= 3)
                    .map(|run| (TilePos::new(coord, run.bottom + 1), run))
            })
            .expect("generated terrain should contain at least one run three levels deep")
    };

    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: target });
    app.update();
    app.update();

    let after = tile_count(&mut app);
    let map = app
        .world()
        .get_resource::<VoxelMap>()
        .expect("a world should exist");

    assert!(map.get(target).is_air(), "the dug voxel should be air");
    assert!(
        !map.get(target.below()).is_air(),
        "material below the hole should survive"
    );
    assert!(
        !map.get(target.above()).is_air(),
        "material above the hole should survive"
    );
    assert_eq!(
        after,
        before + 1,
        "splitting one run into two should add exactly one entity"
    );

    assert_column_run_publication(&mut app, target.coord);
    let published = published_run_bounds(&mut app, target.coord);
    assert!(
        published.contains(&(
            original_run.bottom,
            target.level - 1,
            original_run.substance
        )),
        "the lower cave wall fragment must publish the original run bottom"
    );
    assert!(
        published.contains(&(
            target.level + 1,
            original_run.top - 1,
            original_run.substance
        )),
        "the overhanging fragment must publish the first material voxel above the cave"
    );
}

/// Neither digging nor replacement may remove the world's non-diggable floor.
#[test]
fn terrain_edits_preserve_non_diggable_bedrock() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let pos = TilePos::ORIGIN;
    let (bedrock, stone) = {
        let table = app
            .world()
            .get_resource::<SubstanceTable>()
            .expect("a substance table should exist");
        (
            table.id("bedrock").expect("bedrock should be defined"),
            table.id("stone").expect("stone should be defined"),
        )
    };
    assert_eq!(
        app.world()
            .get_resource::<VoxelMap>()
            .expect("a world should exist")
            .get(pos),
        bedrock,
        "the test target should begin as bedrock"
    );

    app.world_mut().write_message(TerrainEdit::Clear { pos });
    app.update();
    app.update();
    assert_eq!(
        app.world()
            .get_resource::<VoxelMap>()
            .expect("a world should exist")
            .get(pos),
        bedrock,
        "clearing must not remove non-diggable bedrock"
    );

    app.world_mut().write_message(TerrainEdit::Set {
        pos,
        substance: SubstanceId::AIR,
    });
    app.update();
    app.update();
    assert_eq!(
        app.world()
            .get_resource::<VoxelMap>()
            .expect("a world should exist")
            .get(pos),
        bedrock,
        "setting air must not remove non-diggable bedrock"
    );

    app.world_mut().write_message(TerrainEdit::Set {
        pos,
        substance: stone,
    });
    app.update();
    app.update();
    assert_eq!(
        app.world()
            .get_resource::<VoxelMap>()
            .expect("a world should exist")
            .get(pos),
        bedrock,
        "replacement must not overwrite non-diggable bedrock"
    );
}

/// Positions below the bedrock floor are outside the map and must not trigger work.
#[test]
fn terrain_edits_below_the_floor_are_ignored() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let grid_before = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("the grid should exist");
    let stone = app
        .world()
        .get_resource::<SubstanceTable>()
        .and_then(|table| table.id("stone"))
        .expect("stone should be defined");

    app.world_mut().write_message(TerrainEdit::Set {
        pos: TilePos::new(HexCoord::ORIGIN, -1),
        substance: stone,
    });
    app.update();
    app.update();

    let grid_after = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("the grid should still exist");
    assert_eq!(
        grid_after, grid_before,
        "an ignored edit should not rebuild the grid"
    );
}

#[test]
fn one_hundred_idle_frames_do_not_rebuild_or_republish_terrain() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let grid_before = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("the grid should exist");
    let tiles_before: BTreeSet<_> = {
        let mut tiles = app.world_mut().query_filtered::<Entity, With<HexTile>>();
        tiles.iter(app.world()).collect()
    };
    app.world_mut().clear_trackers();

    for _ in 0..100 {
        app.update();
    }

    let grid_after = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("the grid should still exist");
    let tiles_after: BTreeSet<_> = {
        let mut tiles = app.world_mut().query_filtered::<Entity, With<HexTile>>();
        tiles.iter(app.world()).collect()
    };
    assert_eq!(grid_after, grid_before);
    assert_eq!(
        tiles_after, tiles_before,
        "idle terrain reconciliation replaced unchanged run entities"
    );
    assert!(
        !app.world().resource_ref::<VoxelMap>().is_changed(),
        "an empty terrain-edit stream marked voxel storage changed"
    );
    assert!(
        !app.world()
            .resource_ref::<SpecialMovementRegions>()
            .is_changed(),
        "an empty terrain-edit stream marked special regions changed"
    );
}

#[test]
#[ignore = "manual release-mode localized terrain-edit stress gate"]
fn one_hundred_localized_terrain_edits_stay_within_the_interactive_budget() {
    let mut app = procedural_app();
    enter_gameplay(&mut app);

    let (target, original) = {
        let map = app.world().resource::<VoxelMap>();
        let table = app.world().resource::<SubstanceTable>();
        map.columns()
            .find_map(|(coord, column)| {
                let level = column.surface()?;
                let substance = column.get(level);
                table
                    .is_diggable(substance)
                    .then_some((TilePos::new(coord, level), substance))
            })
            .expect("radius-12 Hills should expose diggable surface terrain")
    };
    let meshes_before = app.world().resource::<Assets<Mesh>>().len();
    let materials_before = app.world().resource::<Assets<StandardMaterial>>().len();
    let mut samples = Vec::with_capacity(100);
    let mut created_entities = 0_usize;
    let mut maximum_created = 0_usize;
    let mut assets_after_warmup = None;

    for index in 0..100 {
        let before: BTreeSet<_> = {
            let mut tiles = app.world_mut().query_filtered::<Entity, With<HexTile>>();
            tiles.iter(app.world()).collect()
        };
        if index % 2 == 0 {
            app.world_mut()
                .write_message(TerrainEdit::Clear { pos: target });
        } else {
            app.world_mut().write_message(TerrainEdit::Set {
                pos: target,
                substance: original,
            });
        }

        let started = Instant::now();
        app.update();
        samples.push(started.elapsed());

        let after: BTreeSet<_> = {
            let mut tiles = app.world_mut().query_filtered::<Entity, With<HexTile>>();
            tiles.iter(app.world()).collect()
        };
        let created = after.difference(&before).count();
        created_entities = created_entities.saturating_add(created);
        maximum_created = maximum_created.max(created);
        let expected = if index % 2 == 0 {
            SubstanceId::AIR
        } else {
            original
        };
        assert_eq!(
            app.world().resource::<VoxelMap>().get(target),
            expected,
            "localized edit {index} did not settle in one update"
        );
        if index == 3 {
            assets_after_warmup = Some((
                app.world().resource::<Assets<Mesh>>().len(),
                app.world().resource::<Assets<StandardMaterial>>().len(),
            ));
        }
    }

    samples.sort_unstable();
    let p95 = samples
        .get(94)
        .copied()
        .expect("the terrain benchmark records exactly 100 samples");
    let worst = samples
        .get(99)
        .copied()
        .expect("the terrain benchmark records exactly 100 samples");
    let (meshes_after_warmup, materials_after_warmup) =
        assets_after_warmup.expect("the terrain benchmark completes its four-edit warmup");
    let meshes_after = app.world().resource::<Assets<Mesh>>().len();
    let materials_after = app.world().resource::<Assets<StandardMaterial>>().len();
    eprintln!(
        "radius-12 localized terrain edits: p95={p95:?}, worst={worst:?}, \
         created_total={created_entities}, max_created_per_edit={maximum_created}, \
         meshes={meshes_before}->{meshes_after_warmup}->{meshes_after}, \
         materials={materials_before}->{materials_after_warmup}->{materials_after}"
    );
    assert_eq!(
        meshes_after, meshes_after_warmup,
        "localized terrain edits kept allocating mesh assets after the warmup"
    );
    assert_eq!(
        materials_after, materials_after_warmup,
        "localized terrain edits kept allocating material assets after the warmup"
    );

    let (p95_budget, worst_budget) = if cfg!(debug_assertions) {
        (Duration::from_millis(100), Duration::from_millis(250))
    } else {
        (Duration::from_micros(16_700), Duration::from_millis(50))
    };
    assert!(
        p95 < p95_budget && worst < worst_budget,
        "localized terrain edits exceeded the interaction budget: p95={p95:?}, worst={worst:?}"
    );
}

/// Building above the surface leaves the space between as air — a floating platform.
#[test]
fn setting_a_voxel_above_the_surface_builds_a_platform() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let coord = HexCoord::ORIGIN;
    let (surface, stone) = {
        let world = app.world();
        let map = world
            .get_resource::<VoxelMap>()
            .expect("a world should exist");
        let table = world
            .get_resource::<SubstanceTable>()
            .expect("a substance table should exist");
        (
            map.surface(coord).expect("the origin should have ground"),
            table.id("stone").expect("stone should be defined"),
        )
    };

    let platform = TilePos::new(coord, surface + 4);
    app.world_mut().write_message(TerrainEdit::Set {
        pos: platform,
        substance: stone,
    });
    app.update();
    app.update();

    let map = app
        .world()
        .get_resource::<VoxelMap>()
        .expect("a world should exist");
    assert_eq!(map.get(platform), stone, "the platform should exist");
    assert!(
        map.get(TilePos::new(coord, surface + 2)).is_air(),
        "the space beneath a floating platform stays empty"
    );
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
fn terrain_edits_retire_cave_vegetation_with_invalidated_support() {
    let mut app = v3_caves_app();
    enter_gameplay(&mut app);
    let before = cave_vegetation_instances(&mut app);
    let target = *before
        .keys()
        .next()
        .expect("Caves should publish sparse vegetation");

    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: target });
    app.update();
    app.update();

    let after = cave_vegetation_instances(&mut app);
    assert_eq!(after.len(), before.len().saturating_sub(1));
    assert!(!after.contains_key(&target));
}

#[test]
fn clearing_non_root_support_retires_the_complete_cave_vegetation_feature() {
    let mut app = v3_caves_app();
    enter_gameplay(&mut app);
    let before = cave_vegetation_instances(&mut app);
    let (root, visual) = cave_vegetation_non_root_visual(&mut app);
    let support = TilePos::new(visual.coord, root.level);
    assert!(app
        .world()
        .resource::<SubstanceTable>()
        .is_diggable(app.world().resource::<VoxelMap>().get(support)));

    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: support });
    app.update();
    app.update();

    assert!(app.world().resource::<VoxelMap>().get(support).is_air());
    let after = cave_vegetation_instances(&mut app);
    assert_eq!(after.len(), before.len().saturating_sub(1));
    assert!(!after.contains_key(&root));
}

#[test]
fn building_into_non_root_visual_cell_retires_the_complete_cave_vegetation_feature() {
    let mut app = v3_caves_app();
    enter_gameplay(&mut app);
    let before = cave_vegetation_instances(&mut app);
    let (root, visual) = cave_vegetation_non_root_visual(&mut app);
    let stone = app
        .world()
        .resource::<SubstanceTable>()
        .id("stone")
        .expect("the fixture substance table should contain stone");
    assert!(app.world().resource::<VoxelMap>().get(visual).is_air());

    app.world_mut().write_message(TerrainEdit::Set {
        pos: visual,
        substance: stone,
    });
    app.update();
    app.update();

    assert_eq!(app.world().resource::<VoxelMap>().get(visual), stone);
    let after = cave_vegetation_instances(&mut app);
    assert_eq!(after.len(), before.len().saturating_sub(1));
    assert!(!after.contains_key(&root));
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

/// The world has to exist before the tiles built from it.
///
/// Directly guards `GameplaySetup::Resources` running before `::Terrain`. Systems in
/// one `OnEnter` schedule run in unspecified order unless a set says otherwise, and
/// the two live in different crates, so `.chain()` cannot express it.
#[test]
fn the_world_exists_once_gameplay_starts() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let map = app.world().get_resource::<VoxelMap>();
    assert!(map.is_some(), "tiles spawned without a world to build from");
}

/// Every column within the radius is represented, and nothing outside it is.
///
/// Coordinates now repeat — one entity per substance run — so this checks coverage
/// rather than uniqueness.
#[test]
fn tiles_cover_the_radius_and_nothing_beyond() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let mut query = app.world_mut().query_filtered::<&HexCoord, With<HexTile>>();
    let coords: Vec<HexCoord> = query.iter(app.world()).copied().collect();

    for coord in &coords {
        assert!(
            HexCoord::ORIGIN.distance(*coord) <= TEST_RADIUS,
            "{coord:?} lies outside the configured radius"
        );
    }

    let mut unique = coords;
    unique.sort_by_key(|c| (c.x(), c.y()));
    unique.dedup();
    let expected = (3 * TEST_RADIUS * TEST_RADIUS + 3 * TEST_RADIUS + 1) as usize;
    assert_eq!(unique.len(), expected, "some columns were not spawned");
}
