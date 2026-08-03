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

use bevy::ecs::reflect::AppTypeRegistry;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use std::any::TypeId;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{Duration, Instant};

use hex_assets::GameAssets;
use hex_assets::{
    ArtPalette, CameraSettings, ElementCatalog, ElementFile, ObjectBlueprint, ObjectCatalogFile,
    ObjectInstance, PaletteSwatch, RuntimeArtCatalog, SrgbColor, Substance, SubstanceFile,
    SubstanceTable, SwatchId, TerrainDamageFile, TerrainDamagePair, TerrainDamageTable,
    VoxelStyleCatalog,
};
use hex_core::{
    BiomeRegionId, BiomeRegions, CanopyOccluder, CutawayOccluder, DamagedVoxels, ElementId,
    GameplayLight, GameplaySetupFailure, Headroom, HexCoord, HexGrid, HexSpan, HexTile,
    InteriorRegionId, InteriorRegions, Level, MapAnchorId, MapAnchors, MapViewHint,
    PausableSystems, Pause, PerceptionSystems, PresentationOcclusion, ResolvedMapSeed, RunBottom,
    Screen, SpecialMovementRegion, SpecialMovementRegions, SubstanceId, TerrainBatchId,
    TerrainEdit, TerrainImpact, TerrainImpactDisposition, TerrainImpactOutcome,
    TerrainImpactRejection, TerrainImpactResult, TerrainReady, TerrainSystems, TerrainVoxelHealth,
    TilePos, TraversalBlockers, TreeOccluder, MAX_HEADROOM,
};
use hex_map::{
    CavesReportMetrics, CrossingSettings, EnvironmentSettings, GenerationReport, HillsSettings,
    LandformSettings, LayeredSkyIslandsSettings, LinkedIslandsSettings,
    MacroLiquidConnectionSettings, MacroMetrics, MapSettings, MountainRangeMetrics,
    MountainsSettings, PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings, PatchSpec,
    PerlinSettings, PerlinStepSettings, ProceduralRecipeMetrics, ProceduralSettings,
    ProceduralV1Settings, ProceduralV2Settings, ProceduralV3Settings, Ring19Metrics, Ring7Metrics,
    SkyIslandsSettings, SubstanceRun, TacticalMetrics, TacticalSettings, TerrainSettings,
    V2EnvironmentSettings, V2HillsSettings, V2RecipeSettings, V3CavesSettings,
    V3DeepForestSettings, V3EnvironmentSettings, V3ForestSettings, V3FortSettings, V3HillsSettings,
    V3LayoutSettings, V3RecipeSettings, V3WaterfallSettings, VoxelMap,
};
use hex_test_support::{enter_gameplay, TestAppBuilder};

/// Radius used by the tests. Small enough to stay fast, large enough that the
/// tile-count formula is a meaningful check.
const TEST_RADIUS: u32 = 4;

/// Builds a headless app with the map wired up and settings already present.
///
/// Settings are inserted directly rather than loaded from RON: this is testing
/// terrain construction, not the asset pipeline, and a test that depends on file IO
/// fails for reasons that have nothing to do with what it is checking.
fn test_app() -> App {
    let mut builder = TestAppBuilder::new();
    let app = builder.app_mut();

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
    app.configure_sets(Update, PausableSystems.run_if(in_state(Pause(false))));
    builder.build()
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
        ("sand", true, true),
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
        let substance = if name == "air" {
            Substance::invisible(solid, diggable)
        } else {
            Substance::from_swatch(swatch.clone(), solid, diggable)
        };
        let toughness = match name {
            "grass" | "snow" => Some(1),
            "dirt" | "gravel" | "ice" => Some(2),
            "stone" | "basalt" => Some(4),
            "worked_stone" | "metal" => Some(8),
            _ => None,
        };
        substances.insert(name.to_owned(), substance.with_toughness(toughness));
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

#[expect(
    clippy::expect_used,
    reason = "the tracked Mountain Range review world is a compile-time integration fixture"
)]
fn v3_mountain_range_app() -> App {
    let mut app = test_app();
    let settings: MapSettings = ron::from_str(include_str!(
        "../../../assets/config/worlds/procedural-mountain-range.ron"
    ))
    .expect("tracked Mountain Range settings should parse");
    app.insert_resource(settings);
    app.insert_resource(ResolvedMapSeed(129_704_046));
    app.insert_resource(runtime_art_catalog());
    app
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

#[path = "contracts/composed_worlds.rs"]
mod composed_worlds;
#[path = "contracts/lifecycle.rs"]
mod lifecycle;
#[path = "contracts/presentation.rs"]
mod presentation;
#[path = "contracts/procedural_publication.rs"]
mod procedural_publication;
#[path = "contracts/publication.rs"]
mod publication;
#[path = "contracts/terrain_damage.rs"]
mod terrain_damage;
#[path = "contracts/terrain_edits.rs"]
mod terrain_edits;
