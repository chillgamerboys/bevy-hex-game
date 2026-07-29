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
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use hex_assets::GameAssets;
use hex_assets::{
    ArtPalette, PaletteSwatch, SrgbColor, Substance, SubstanceFile, SubstanceTable, SwatchId,
};
use hex_core::{
    BiomeRegionId, BiomeRegions, CanopyOccluder, CutawayOccluder, GameplayLight, GameplaySetup,
    GameplaySetupFailure, Headroom, HexCoord, HexGrid, HexSpan, HexTile, InteriorRegionId,
    InteriorRegions, Level, MapAnchorId, MapAnchors, MapViewHint, PresentationOcclusion,
    ResolvedMapSeed, Screen, SpecialMovementRegion, SpecialMovementRegions, SubstanceId,
    TerrainEdit, TerrainReady, TilePos, TraversalBlockers, MAX_HEADROOM,
};
use hex_map::{
    CavesReportMetrics, CrossingSettings, EnvironmentSettings, GenerationReport, HillsSettings,
    LandformSettings, LayeredSkyIslandsSettings, LinkedIslandsSettings, MapSettings,
    MountainsSettings, PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings, PatchSpec,
    PerlinSettings, PerlinStepSettings, ProceduralRecipeMetrics, ProceduralSettings,
    ProceduralV1Settings, ProceduralV2Settings, ProceduralV3Settings, SkyIslandsSettings,
    SubstanceRun, TacticalMetrics, TacticalSettings, TerrainSettings, V2EnvironmentSettings,
    V2HillsSettings, V2RecipeSettings, V3CavesSettings, V3EnvironmentSettings, V3ForestSettings,
    V3FortSettings, V3HillsSettings, V3LayoutSettings, V3RecipeSettings, V3WaterfallSettings,
    VoxelMap,
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

fn substance_table_without_swatch(omitted: &str) -> SubstanceTable {
    substance_table_fixture(None, Some(omitted))
}

fn substance_table_without(omitted: Option<&str>) -> SubstanceTable {
    substance_table_fixture(omitted, None)
}

#[expect(
    clippy::expect_used,
    reason = "invalid compile-time fixture data should fail the integration test immediately"
)]
fn substance_table_fixture(
    omitted_substance: Option<&str>,
    omitted_swatch: Option<&str>,
) -> SubstanceTable {
    let swatch = SwatchId::new("test/neutral").expect("the fixture swatch id should be valid");
    let foam = SwatchId::new("liquid/foam").expect("the foam swatch id should be valid");
    let mut swatches = BTreeMap::from([
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
    for (id, display_name, (red, green, blue)) in [
        ("plant/trunk", "Tree Trunk", (0.28, 0.15, 0.07)),
        ("plant/foliage-dark", "Dark Foliage", (0.12, 0.34, 0.12)),
        ("plant/foliage-mid", "Mid Foliage", (0.18, 0.42, 0.14)),
        ("plant/foliage-light", "Light Foliage", (0.25, 0.48, 0.16)),
        ("plant/grass-dark", "Dark Grass Blade", (0.34, 0.52, 0.14)),
        ("plant/grass-light", "Light Grass Blade", (0.45, 0.62, 0.18)),
    ] {
        if omitted_swatch == Some(id) {
            continue;
        }
        swatches.insert(
            SwatchId::new(id).expect("fixture plant swatch ids should be valid"),
            PaletteSwatch::new(
                display_name,
                SrgbColor::new(red, green, blue)
                    .expect("fixture plant swatch colors should be valid"),
                BTreeSet::from(["test".to_owned()]),
            )
            .expect("fixture plant swatches should be valid"),
        );
    }
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

fn v3_waterfall_app() -> App {
    let mut app = procedural_app();
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

fn v3_forest_app() -> App {
    let mut app = procedural_app();
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
        Some(8_012_354_130_252_983_421)
    );
    assert_eq!(report.map_fingerprint, 17_075_345_429_537_665_322);
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
    assert_eq!(metrics.fall_height, 11);
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
    assert!(app.world().resource::<TraversalBlockers>().is_empty());
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
    assert_eq!(report.valid_candidates, 4);
    assert!(!report.used_fallback);
    assert_eq!(report.repair_rounds, 0);
    assert!(report.repair_actions.is_empty());
    assert_eq!(report.notes.len(), 4);
    assert!(report
        .notes
        .iter()
        .all(|note| note.starts_with("candidate ")));
    assert_eq!(report.selected_candidate, Some(0));
    assert_eq!(report.settings_fingerprint, 2_658_105_648_444_344_100);
    assert_eq!(
        report.semantic_plan_fingerprint,
        Some(14_183_726_856_212_867_729)
    );
    assert_eq!(report.map_fingerprint, 17_318_082_348_573_723_024);
    let Some(ProceduralRecipeMetrics::Forest(metrics)) = &report.recipe_metrics else {
        panic!("V3 Forest should publish exact recipe metrics");
    };
    assert_eq!(metrics.clearing_count, 4);
    assert_eq!(metrics.relief, 4);
    assert_eq!(metrics.tree_roots, 53);
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
    assert_eq!(tree_roots, blockers);
    assert!(grass_roots.is_disjoint(&blockers));
    let canopy_roots: BTreeSet<_> = {
        let world = app.world_mut();
        let mut canopies =
            world.query::<(&CanopyOccluder, &PresentationOcclusion, Option<&HexTile>)>();
        canopies
            .iter(world)
            .map(|(canopy, occlusion, tile)| {
                assert!(tile.is_none(), "a tree canopy became terrain footing");
                assert!(
                    !occlusion.is_hidden(),
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
    assert_eq!(app.world().resource::<SpecialMovementRegions>().len(), 23);
    assert!(app.world().resource::<MapViewHint>().is_valid());
}

#[test]
fn v3_forest_missing_required_feature_swatch_fails_presentation_setup() {
    let mut app = v3_forest_app();
    app.insert_resource(substance_table_without_swatch("plant/foliage-mid"));
    enter_gameplay(&mut app);

    let failure = app
        .world()
        .get_resource::<GameplaySetupFailure>()
        .expect("missing Forest presentation colour should publish a setup failure");
    assert!(
        failure.reason.contains("plant/foliage-mid"),
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

    let root_coords: BTreeSet<_> = initial_roots
        .keys()
        .map(|position| position.coord)
        .collect();
    let unrelated = {
        let world = app.world();
        let table = world.resource::<SubstanceTable>();
        world
            .resource::<VoxelMap>()
            .columns()
            .filter(|(coord, _column)| !root_coords.contains(coord))
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
        optional_dark_floors,
        minimum_roof_thickness,
        ..
    })) = report.recipe_metrics
    else {
        panic!("V3 Caves should publish exact recipe metrics");
    };
    assert_eq!(chamber_count, 12);
    assert!(gameplay_lights > 0);
    assert!(optional_dark_floors > 0);
    assert!(minimum_roof_thickness >= 3);

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

fn feature_roots(app: &mut App) -> Vec<(Entity, String, TilePos, Entity)> {
    let world = app.world_mut();
    let level_height = world.resource::<MapSettings>().level_height;
    let mut query = world.query::<(Entity, &Name, &Transform, &ChildOf, Option<&HexTile>)>();
    query
        .iter(world)
        .filter(|(_entity, name, _transform, _parent, _tile)| {
            matches!(name.as_str(), "GeneratedTree" | "GeneratedTallGrass")
        })
        .map(|(entity, name, transform, parent, tile)| {
            assert!(
                tile.is_none(),
                "feature roots must not become terrain tiles"
            );
            let surface_boundary = transform.translation.y / level_height;
            assert!(
                (surface_boundary - surface_boundary.round()).abs() < 1.0e-4,
                "feature root height must resolve to one exact voxel boundary"
            );
            #[expect(
                clippy::cast_possible_truncation,
                reason = "validated map levels are bounded signed integers"
            )]
            let level = surface_boundary.round() as i32 - 1;
            (
                entity,
                name.as_str().to_owned(),
                TilePos::new(HexCoord::from_world(transform.translation), level),
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

    let mut query = app
        .world_mut()
        .query_filtered::<(&HexCoord, &TilePos, &HexSpan, &SubstanceId, &Headroom), With<HexTile>>(
        );

    let mut checked = 0;
    for (coord, pos, span, substance, headroom) in query.iter(app.world()) {
        assert!(!substance.is_air(), "air should not be spawned as a prism");
        assert_eq!(pos.coord, *coord, "a tile's position must match its column");
        assert!(span.height() > 0.0, "a tile span must have positive height");
        assert!(
            (0..=MAX_HEADROOM).contains(&headroom.0),
            "headroom must remain bounded"
        );
        checked += 1;
    }
    assert!(checked > 0, "no tiles were checked");
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
    let target = {
        let map = app
            .world()
            .get_resource::<VoxelMap>()
            .expect("a world should exist");
        map.columns()
            .find_map(|(coord, column)| {
                hex_map::runs(column)
                    .into_iter()
                    .find(|run| run.levels() >= 3)
                    .map(|run| TilePos::new(coord, run.bottom + 1))
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

    assert!(!first_floors.is_empty());
    assert!(!first_roofs.is_empty());
    assert!(!first_lights.is_empty());

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
    assert_eq!(second_anchors, first_anchors);
    assert_eq!(second_floors, first_floors);
    assert_eq!(second_roofs, first_roofs);
    assert_eq!(second_lights, first_lights);
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
