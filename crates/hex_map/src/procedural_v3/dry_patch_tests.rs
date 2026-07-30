use std::collections::{BTreeSet, VecDeque};

use hex_core::{SpecialMovementRegion, TilePos};

use super::composition::GeneratedPatchPlan;
use super::layout::ResolvedLiquidPort;
use super::layout::{resolve_layout, PatchId, ResolvedLayoutPlan};
use super::liquid::LiquidFlowState;
use super::patch::{PatchBuildMode, PatchRecipeContext};
use super::seam::validate_patch_walker_seams;
use super::{caves, fort, hills, mountains, prairie, sky, waterfall};
use crate::settings::{
    EdgeElevationSettings, EdgeLiquidPortSettings, EdgeLiquidSettings, PatchEdgeContractSettings,
    PatchEdgesSettings, PatchMaskSettings, PatchSpec, ProceduralV3Settings, SharedEdgeSettings,
    V3CavesSettings, V3EnvironmentSettings, V3ForestSettings, V3FortSettings, V3HillsSettings,
    V3LayoutSettings, V3MountainsSettings, V3PrairieSettings, V3RecipeSettings, V3Ring7Settings,
    V3SkyIslandsSettings, V3WaterfallSettings, WalkerPortSettings,
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
fn at_least_one_complete_candidate_index_constructs_every_dry_patch() {
    let (settings, recipes) = dry_ring_settings();
    let layout = resolve_layout(33, &settings).expect("the dry Ring7 fixture should resolve");
    let plans = (0..8)
        .find_map(|candidate| {
            construct_dry_plans(
                &layout,
                &recipes,
                PatchBuildMode::Candidate {
                    world_seed: 703_700_113,
                    candidate,
                },
            )
            .ok()
        })
        .expect("one complete candidate index should construct every dry patch");
    for plan in plans {
        assert_strict_patch(&layout, plan);
    }
}

#[test]
fn stitched_sky_rejects_corrupted_upper_bridge_structures_after_local_projection() {
    let (settings, recipes) = dry_ring_settings();
    let layout = resolve_layout(33, &settings).expect("the dry Ring7 fixture should resolve");
    let context = patch(&layout, 6).expect("Sky Islands slot should resolve as a patch context");
    let catalog = super::vegetation::tests::runtime_art_catalog();
    let baseline = sky::construct_patch_with_catalog(
        context,
        &recipes.sky,
        V3EnvironmentSettings::TemperateGrassland,
        LEVEL_HEIGHT,
        PatchBuildMode::CanonicalFallback,
        catalog,
    )
    .expect("stitched Sky Islands should construct");
    let metrics = match sky::validate_patch(
        context,
        &baseline,
        &recipes.sky,
        V3EnvironmentSettings::TemperateGrassland,
        catalog,
    ) {
        super::selection::WorldValidation::Valid(metrics) => metrics,
        super::selection::WorldValidation::Invalid(issues) => {
            panic!("the unmodified stitched Sky Islands fixture must validate: {issues:?}");
        }
    };
    assert_eq!(metrics.primary_islands, 3);
    assert_eq!(metrics.upper_tree_roots, 3);
    assert!((15..=35).contains(&metrics.upper_grass_percent));

    let bridge_ids = baseline
        .structures
        .by_id
        .iter()
        .filter_map(|(id, structure)| {
            (structure.kind == super::world::StructureKind::Bridge
                && structure.voxels.iter().all(|voxel| {
                    baseline.volume.surfaces.get(voxel).is_some_and(|metadata| {
                        metadata.access
                            == super::volume::SurfaceAccess::SpecialMovement(SpecialMovementRegion(
                                0,
                            ))
                    })
                }))
            .then_some(*id)
        })
        .collect::<Vec<_>>();
    let [first_id, second_id] = bridge_ids.as_slice() else {
        panic!("the stitched fixture must contain exactly two upper bridges");
    };
    assert_stitched_sky_topology(&baseline, *first_id, *second_id);

    let mut missing = baseline.clone();
    missing.structures.by_id.remove(first_id);
    assert_validation_rejects_with(
        sky::validate_patch(
            context,
            &missing,
            &recipes.sky,
            V3EnvironmentSettings::TemperateGrassland,
            catalog,
        ),
        "requires exactly two upper bridge structures",
    );

    let first_voxel = baseline
        .structures
        .by_id
        .get(first_id)
        .and_then(|structure| structure.voxels.first())
        .copied()
        .expect("the first upper bridge should contain exact voxels");
    let mut overlapping = baseline.clone();
    overlapping
        .structures
        .by_id
        .get_mut(second_id)
        .expect("the second upper bridge should remain present")
        .voxels
        .insert(first_voxel);
    assert_validation_rejects_with(
        sky::validate_patch(
            context,
            &overlapping,
            &recipes.sky,
            V3EnvironmentSettings::TemperateGrassland,
            catalog,
        ),
        "upper bridge structures overlap",
    );

    let mut narrowed = baseline.clone();
    narrowed
        .structures
        .by_id
        .get_mut(first_id)
        .expect("the first upper bridge should remain present")
        .voxels
        .remove(&first_voxel);
    narrowed
        .structures
        .by_id
        .get_mut(second_id)
        .expect("the second upper bridge should remain present")
        .voxels
        .insert(first_voxel);
    assert_validation_rejects_with(
        sky::validate_patch(
            context,
            &narrowed,
            &recipes.sky,
            V3EnvironmentSettings::TemperateGrassland,
            catalog,
        ),
        "is not a two-wide span",
    );
}

fn assert_stitched_sky_topology(
    plan: &GeneratedPatchPlan,
    first_id: super::world::StructureId,
    second_id: super::world::StructureId,
) {
    let primary = plan
        .volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| {
            (metadata.access
                == super::volume::SurfaceAccess::SpecialMovement(SpecialMovementRegion(0)))
            .then_some(*position)
        })
        .collect::<BTreeSet<_>>();
    let bridges = [first_id, second_id].map(|id| {
        plan.structures
            .by_id
            .get(&id)
            .expect("every upper bridge id should resolve")
            .voxels
            .clone()
    });
    let [first_bridge, second_bridge] = &bridges;
    assert!(first_bridge.is_disjoint(second_bridge));
    let bridge_union = bridges
        .iter()
        .flat_map(|bridge| bridge.iter().copied())
        .collect::<BTreeSet<_>>();
    assert!(bridge_union.is_subset(&primary));
    assert!(bridges
        .iter()
        .all(|bridge| bridge.len() >= 4 && bridge.len() % 2 == 0));
    assert!(bridge_union.iter().all(|surface| {
        bridge_union
            .iter()
            .filter(|neighbor| surface.coord.distance(neighbor.coord) == 1)
            .all(|neighbor| surface.level.abs_diff(neighbor.level) <= 1)
    }));

    let islands = test_surface_components(
        &primary
            .difference(&bridge_union)
            .copied()
            .collect::<BTreeSet<_>>(),
    );
    assert_eq!(islands.len(), 3);
    assert_eq!(
        islands
            .iter()
            .flat_map(|island| island.iter().map(|surface| surface.level))
            .collect::<BTreeSet<_>>()
            .len(),
        3
    );
    for bridge in bridges {
        let contacts = islands
            .iter()
            .filter(|island| {
                bridge
                    .iter()
                    .any(|voxel| island.iter().any(|surface| test_adjoin(*voxel, *surface)))
            })
            .count();
        assert_eq!(
            contacts, 2,
            "each upper span should avoid the unrelated island"
        );
    }
}

fn test_surface_components(surfaces: &BTreeSet<TilePos>) -> Vec<BTreeSet<TilePos>> {
    let mut remaining = surfaces.clone();
    let mut components = Vec::new();
    while let Some(start) = remaining.first().copied() {
        remaining.remove(&start);
        let mut component = BTreeSet::from([start]);
        let mut frontier = VecDeque::from([start]);
        while let Some(position) = frontier.pop_front() {
            let neighbors = remaining
                .iter()
                .copied()
                .filter(|neighbor| test_adjoin(position, *neighbor))
                .collect::<Vec<_>>();
            for neighbor in neighbors {
                remaining.remove(&neighbor);
                component.insert(neighbor);
                frontier.push_back(neighbor);
            }
        }
        components.push(component);
    }
    components
}

fn test_adjoin(first: TilePos, second: TilePos) -> bool {
    first.coord.distance(second.coord) == 1 && first.level.abs_diff(second.level) <= 1
}

#[test]
fn prairie_stitched_patch_keeps_protected_approaches_clear_and_validates_exact_coverage() {
    let (settings, _) = dry_ring_settings();
    let layout = resolve_layout(33, &settings).expect("the dry Ring7 fixture should resolve");
    let context = patch(&layout, 3).expect("Forest slot should resolve as a patch context");
    let protected = context.protected_approaches();
    assert!(
        !protected.is_empty(),
        "the stitched regression requires exact protected seam approaches"
    );
    let prairie_settings = V3PrairieSettings {
        base_level: 15,
        max_relief: 4,
        grass_coverage_percent: 70,
    };
    let catalog = super::vegetation::tests::runtime_art_catalog();
    let plan = prairie::construct_patch(
        context,
        &prairie_settings,
        V3EnvironmentSettings::TemperateGrassland,
        LEVEL_HEIGHT,
        PatchBuildMode::Candidate {
            world_seed: 1_592_598_566,
            candidate: 0,
        },
        catalog,
    )
    .expect("Prairie should construct inside a stitched vegetation patch");
    assert!(plan
        .features
        .by_id
        .values()
        .all(|feature| !protected.contains(&feature.root.coord)));
    let metrics = match prairie::validate_patch(context, &prairie_settings, &plan, catalog) {
        super::selection::WorldValidation::Valid(metrics) => metrics,
        super::selection::WorldValidation::Invalid(issues) => {
            panic!(
                "Prairie construction and patch validation must share exact eligibility: \
                 {issues:?}"
            );
        }
    };
    assert_eq!(
        metrics.grass_roots,
        u32::try_from(plan.features.by_id.len()).unwrap_or(u32::MAX)
    );
    assert!((65..=75).contains(&metrics.grass_coverage_percent));
    assert_strict_patch(&layout, plan);
}

#[test]
fn rotated_waterfall_outlet_aligns_with_the_center_hills_inlet() {
    let (mut settings, recipes) = dry_ring_settings();
    let V3LayoutSettings::Ring7(ring) = &mut settings.layout else {
        panic!("the fixture must remain Ring7");
    };
    ring.center.edges.east = shared_edge(EdgeLiquidSettings::Inlet(EdgeLiquidPortSettings {
        width: 3,
    }));
    ring.waterfall.edges.west = shared_edge(EdgeLiquidSettings::Outlet(EdgeLiquidPortSettings {
        width: 3,
    }));
    ring.center.edges.west = shared_edge(EdgeLiquidSettings::Outlet(EdgeLiquidPortSettings {
        width: 3,
    }));
    ring.caves.edges.east = shared_edge(EdgeLiquidSettings::Inlet(EdgeLiquidPortSettings {
        width: 3,
    }));
    let layout = resolve_layout(33, &settings).expect("the directed Ring7 fixture should resolve");
    let mode = PatchBuildMode::Candidate {
        world_seed: 17,
        candidate: 2,
    };
    let hills = hills::construct_patch_with_catalog(
        patch(&layout, 0).expect("center patch"),
        &recipes.hills,
        V3EnvironmentSettings::TemperateGrassland,
        LEVEL_HEIGHT,
        mode,
        super::vegetation::tests::runtime_art_catalog(),
    )
    .expect("center Hills should align its river inlet");
    let waterfall = waterfall::construct_patch_with_catalog(
        patch(&layout, 2).expect("Waterfall patch"),
        &V3WaterfallSettings,
        V3EnvironmentSettings::TemperateGrassland,
        LEVEL_HEIGHT,
        mode,
        super::vegetation::tests::runtime_art_catalog(),
    )
    .expect("Waterfall should rotate its low outlet toward the center");
    let edge = layout
        .shared_edges
        .values()
        .find(|edge| {
            matches!(
                edge.liquid,
                ResolvedLiquidPort::Directed {
                    source: PatchId(2),
                    sink: PatchId(0),
                    ..
                }
            )
        })
        .expect("the center/Waterfall liquid edge should be directed");
    let ResolvedLiquidPort::Directed { port, .. } = &edge.liquid else {
        unreachable!();
    };
    let hills_nodes = &hills
        .liquids
        .bodies
        .values()
        .next()
        .expect("Hills river body")
        .nodes;
    let waterfall_nodes = &waterfall
        .liquids
        .bodies
        .values()
        .next()
        .expect("Waterfall body")
        .nodes;

    for (center_coord, waterfall_coord) in &port.lanes {
        let center = hills_nodes
            .iter()
            .find(|(position, _)| position.coord == *center_coord)
            .expect("every Hills inlet lane should have one liquid node");
        let outlet = waterfall_nodes
            .iter()
            .find(|(position, _)| position.coord == *waterfall_coord)
            .expect("every Waterfall outlet lane should have one liquid node");
        assert_eq!(outlet.0.level.saturating_sub(center.0.level), 1);
        assert_eq!(outlet.1.state, LiquidFlowState::Still);
        assert_eq!(outlet.1.downstream, None);
    }
}

fn assert_validation_rejects_with<T>(
    validation: super::selection::WorldValidation<T>,
    expected_detail: &str,
) {
    let super::selection::WorldValidation::Invalid(issues) = validation else {
        panic!("corrupted stitched Sky Islands unexpectedly validated");
    };
    assert!(
        issues
            .iter()
            .any(|issue| issue.detail.contains(expected_detail)),
        "expected an issue containing {expected_detail:?}, got {issues:?}"
    );
}

fn construct_dry_plans(
    layout: &ResolvedLayoutPlan,
    recipes: &DryRecipes,
    mode: PatchBuildMode,
) -> Result<Vec<GeneratedPatchPlan>, Vec<super::world::WorldValidationIssue>> {
    Ok(vec![
        hills::construct_patch_with_catalog(
            patch(layout, 0)?,
            &recipes.hills,
            V3EnvironmentSettings::TemperateGrassland,
            LEVEL_HEIGHT,
            mode,
            super::vegetation::tests::runtime_art_catalog(),
        )?,
        mountains::construct_patch_with_catalog(
            patch(layout, 1)?,
            &recipes.mountains,
            LEVEL_HEIGHT,
            mode,
            super::vegetation::tests::runtime_art_catalog(),
        )?,
        fort::construct_patch(patch(layout, 4)?, &V3FortSettings, LEVEL_HEIGHT, mode)?,
        caves::construct_patch_without_vegetation(
            patch(layout, 5)?,
            &recipes.caves,
            LEVEL_HEIGHT,
            mode,
        )?,
        sky::construct_patch_with_catalog(
            patch(layout, 6)?,
            &recipes.sky,
            V3EnvironmentSettings::TemperateGrassland,
            LEVEL_HEIGHT,
            mode,
            super::vegetation::tests::runtime_art_catalog(),
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
        surface_level: 17,
        cave_floor_level: 6,
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
    shared_edge(EdgeLiquidSettings::Dry)
}

fn shared_edge(liquid: EdgeLiquidSettings) -> PatchEdgeContractSettings {
    PatchEdgeContractSettings::Shared(SharedEdgeSettings {
        elevation: EdgeElevationSettings {
            preferred: 15,
            min: 14,
            max: 16,
        },
        walker: WalkerPortSettings { count: 1, width: 2 },
        liquid,
        approach_depth: 2,
    })
}
