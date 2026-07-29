use super::composition::GeneratedPatchPlan;
use super::layout::{resolve_layout, PatchId, ResolvedLayoutPlan};
use super::patch::{PatchBuildMode, PatchRecipeContext};
use super::seam::validate_patch_walker_seams;
use super::{caves, fort, hills, mountains, sky};
use crate::settings::{
    EdgeElevationSettings, EdgeLiquidSettings, PatchEdgeContractSettings, PatchEdgesSettings,
    PatchMaskSettings, PatchSpec, ProceduralV3Settings, SharedEdgeSettings, V3CavesSettings,
    V3EnvironmentSettings, V3ForestSettings, V3FortSettings, V3HillsSettings, V3LayoutSettings,
    V3MountainsSettings, V3RecipeSettings, V3Ring7Settings, V3SkyIslandsSettings,
    V3WaterfallSettings, WalkerPortSettings,
};

const LEVEL_HEIGHT: f32 = 0.4;

#[test]
fn every_dry_recipe_fallback_is_a_strict_patch_plan() {
    let (settings, recipes) = dry_ring_settings();
    let layout = resolve_layout(33, &settings).expect("the dry Ring7 fixture should resolve");
    let mode = PatchBuildMode::CanonicalFallback;

    let plans = construct_dry_plans(&layout, &recipes, mode)
        .expect("every dry recipe fallback should fit its resolved patch");
    assert_eq!(plans.len(), 5);
    for plan in plans {
        assert_strict_patch(&layout, plan);
    }
}

#[test]
fn one_complete_candidate_index_constructs_every_dry_patch() {
    let (settings, recipes) = dry_ring_settings();
    let layout = resolve_layout(33, &settings).expect("the dry Ring7 fixture should resolve");
    let plans = construct_dry_plans(
        &layout,
        &recipes,
        PatchBuildMode::Candidate {
            world_seed: 0,
            candidate: 2,
        },
    )
    .expect("the pinned complete candidate should construct every dry patch");
    for plan in plans {
        assert_strict_patch(&layout, plan);
    }
}

fn construct_dry_plans(
    layout: &ResolvedLayoutPlan,
    recipes: &DryRecipes,
    mode: PatchBuildMode,
) -> Result<Vec<GeneratedPatchPlan>, Vec<super::world::WorldValidationIssue>> {
    Ok(vec![
        hills::construct_patch(
            patch(layout, 0)?,
            &recipes.hills,
            V3EnvironmentSettings::TemperateGrassland,
            LEVEL_HEIGHT,
            mode,
        )?,
        mountains::construct_patch(patch(layout, 1)?, &recipes.mountains, LEVEL_HEIGHT, mode)?,
        fort::construct_patch(patch(layout, 4)?, &V3FortSettings, LEVEL_HEIGHT, mode)?,
        caves::construct_patch(patch(layout, 5)?, &recipes.caves, LEVEL_HEIGHT, mode)?,
        sky::construct_patch(
            patch(layout, 6)?,
            &recipes.sky,
            V3EnvironmentSettings::TemperateGrassland,
            LEVEL_HEIGHT,
            mode,
        )?,
    ])
}

fn patch(
    layout: &ResolvedLayoutPlan,
    id: u32,
) -> Result<PatchRecipeContext<'_>, Vec<super::world::WorldValidationIssue>> {
    PatchRecipeContext::resolve(layout, PatchId(id)).map_err(|error| {
        vec![super::world::WorldValidationIssue::new(
            super::world::WorldIssueCode::Recipe("dry_patch_test"),
            error.to_string(),
        )]
    })
}

fn assert_strict_patch(layout: &ResolvedLayoutPlan, plan: GeneratedPatchPlan) {
    assert_eq!(
        plan.volume.mask,
        layout
            .patches
            .get(&plan.patch_id)
            .expect("the patch remains resolved")
            .mask
    );
    assert_eq!(plan.validate_against(layout), Vec::new());
    let context =
        PatchRecipeContext::resolve(layout, plan.patch_id).expect("the patch remains resolved");
    assert_eq!(
        validate_patch_walker_seams(&context, &plan.volume),
        Vec::new()
    );
}

#[derive(Debug)]
struct DryRecipes {
    hills: V3HillsSettings,
    mountains: V3MountainsSettings,
    caves: V3CavesSettings,
    sky: V3SkyIslandsSettings,
}

fn dry_ring_settings() -> (ProceduralV3Settings, DryRecipes) {
    let hills = V3HillsSettings {
        valley_level: 15,
        max_relief: 8,
        hills_per_bank: 3,
    };
    let mountains = V3MountainsSettings {
        base_level: 15,
        relief: 18,
        peak_count: 5,
    };
    let caves = V3CavesSettings {
        surface_level: 16,
        cave_floor_level: 7,
        chamber_count: 9,
    };
    let sky = V3SkyIslandsSettings {
        ground: hills.clone(),
        min_clearance: 14,
        upper_coverage_percent: 20,
    };
    let mut ring = V3Ring7Settings {
        center: generated_patch(
            V3EnvironmentSettings::TemperateGrassland,
            V3RecipeSettings::Hills(hills.clone()),
        ),
        mountains: generated_patch(
            V3EnvironmentSettings::Frozen,
            V3RecipeSettings::Mountains(mountains.clone()),
        ),
        waterfall: generated_patch(
            V3EnvironmentSettings::TemperateGrassland,
            V3RecipeSettings::Waterfall(V3WaterfallSettings),
        ),
        forest: generated_patch(
            V3EnvironmentSettings::TemperateGrassland,
            V3RecipeSettings::Forest(V3ForestSettings),
        ),
        fort: generated_patch(
            V3EnvironmentSettings::TemperateGrassland,
            V3RecipeSettings::Fort(V3FortSettings),
        ),
        caves: generated_patch(
            V3EnvironmentSettings::Rocky,
            V3RecipeSettings::Caves(caves.clone()),
        ),
        sky_islands: generated_patch(
            V3EnvironmentSettings::TemperateGrassland,
            V3RecipeSettings::SkyIslands(sky.clone()),
        ),
    };
    let shared = dry_shared_edge();
    ring.center.edges.north_east = shared.clone();
    ring.mountains.edges.south_west = shared.clone();
    ring.center.edges.east = shared.clone();
    ring.waterfall.edges.west = shared.clone();
    ring.center.edges.south_east = shared.clone();
    ring.forest.edges.north_west = shared.clone();
    ring.center.edges.south_west = shared.clone();
    ring.fort.edges.north_east = shared.clone();
    ring.center.edges.west = shared.clone();
    ring.caves.edges.east = shared.clone();
    ring.center.edges.north_west = shared.clone();
    ring.sky_islands.edges.south_east = shared.clone();
    ring.mountains.edges.south_east = shared.clone();
    ring.waterfall.edges.north_west = shared.clone();
    ring.waterfall.edges.south_west = shared.clone();
    ring.forest.edges.north_east = shared.clone();
    ring.forest.edges.west = shared.clone();
    ring.fort.edges.east = shared.clone();
    ring.fort.edges.north_west = shared.clone();
    ring.caves.edges.south_east = shared.clone();
    ring.caves.edges.north_east = shared.clone();
    ring.sky_islands.edges.south_west = shared.clone();
    ring.sky_islands.edges.east = shared.clone();
    ring.mountains.edges.west = shared;

    (
        ProceduralV3Settings {
            layout: V3LayoutSettings::Ring7(ring),
        },
        DryRecipes {
            hills,
            mountains,
            caves,
            sky,
        },
    )
}

fn generated_patch(environment: V3EnvironmentSettings, recipe: V3RecipeSettings) -> PatchSpec {
    PatchSpec {
        environment,
        recipe,
        overlays: Vec::new(),
        mask: PatchMaskSettings::GeneratedRegion,
        edges: world_boundary_edges(),
    }
}

fn world_boundary_edges() -> PatchEdgesSettings {
    PatchEdgesSettings {
        east: PatchEdgeContractSettings::WorldBoundary,
        south_east: PatchEdgeContractSettings::WorldBoundary,
        south_west: PatchEdgeContractSettings::WorldBoundary,
        west: PatchEdgeContractSettings::WorldBoundary,
        north_west: PatchEdgeContractSettings::WorldBoundary,
        north_east: PatchEdgeContractSettings::WorldBoundary,
    }
}

fn dry_shared_edge() -> PatchEdgeContractSettings {
    PatchEdgeContractSettings::Shared(SharedEdgeSettings {
        elevation: EdgeElevationSettings {
            preferred: 15,
            min: 14,
            max: 16,
        },
        walker: WalkerPortSettings { count: 1, width: 2 },
        liquid: EdgeLiquidSettings::Dry,
        approach_depth: 2,
    })
}
