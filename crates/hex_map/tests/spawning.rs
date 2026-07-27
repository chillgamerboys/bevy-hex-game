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
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;

use std::collections::{BTreeMap, HashMap};

use hex_assets::GameAssets;
use hex_assets::{Substance, SubstanceFile, SubstanceTable};
use hex_core::{
    CutawayOccluder, GameplaySetup, GameplaySetupFailure, Headroom, HexCoord, HexGrid, HexSpan,
    HexTile, InteriorRegionId, InteriorRegions, Level, MapAnchorId, MapAnchors, MapViewHint,
    ResolvedMapSeed, Screen, SpecialMovementRegion, SpecialMovementRegions, SubstanceId,
    TerrainEdit, TerrainReady, TilePos, MAX_HEADROOM,
};
use hex_map::{
    CrossingSettings, EnvironmentSettings, GenerationReport, HillsSettings, LandformSettings,
    LinkedIslandsSettings, MapSettings, MountainsSettings, PerlinSettings, PerlinStepSettings,
    ProceduralSettings, ProceduralV1Settings, ProceduralV2Settings, SkyIslandsSettings,
    SubstanceRun, TacticalSettings, TerrainSettings, V2EnvironmentSettings, V2HillsSettings,
    V2RecipeSettings, VoxelMap,
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
        ("snow", true, true),
        ("ice", true, true),
        ("basalt", true, true),
        ("lava", false, true),
    ] {
        if omitted == Some(name) {
            continue;
        }
        substances.insert(
            name.to_owned(),
            Substance {
                color: (0.5, 0.5, 0.5),
                solid,
                diggable,
            },
        );
    }
    SubstanceTable::from_file(&SubstanceFile { substances })
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
fn unavailable_v2_recipe_reports_failure_without_partial_terrain() {
    let mut app = procedural_app();
    app.insert_resource(MapSettings {
        grid_radius: 12,
        level_height: 0.4,
        terrain: TerrainSettings::Procedural(ProceduralSettings::V2(ProceduralV2Settings {
            environment: V2EnvironmentSettings::Frozen,
            recipe: V2RecipeSettings::Mountains(MountainsSettings {
                base_level: 15,
                relief: 15,
                peak_count: 4,
            }),
        })),
    });

    enter_gameplay(&mut app);

    assert!(!app.world().contains_resource::<TerrainReady>());
    assert!(!app.world().contains_resource::<VoxelMap>());
    assert!(!app.world().contains_resource::<MapAnchors>());
    assert!(!app.world().contains_resource::<SpecialMovementRegions>());
    assert!(!app.world().contains_resource::<InteriorRegions>());
    assert!(!app.world().contains_resource::<MapViewHint>());
    assert!(!app.world().contains_resource::<GenerationReport>());
    assert!(app
        .world()
        .resource::<GameplaySetupFailure>()
        .reason
        .contains("V2 recipe Mountains is not available"));
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
