use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_core::{SpecialMovementRegion, TilePos};

use super::composition::GeneratedPatchPlan;
use super::layout::ResolvedLiquidPort;
use super::layout::{resolve_layout, HexSide, PatchId, ResolvedEdgeReference, ResolvedLayoutPlan};
use super::liquid::LiquidFlowState;
use super::patch::{PatchBuildMode, PatchRecipeContext};
use super::seam::validate_patch_walker_seams;
use super::{caves, deep_forest, fort, hills, mountains, prairie, sky, volcano, waterfall};
use crate::settings::{
    EdgeElevationSettings, EdgeLiquidPortSettings, EdgeLiquidSettings, PatchEdgeContractSettings,
    PatchEdgesSettings, PatchMaskSettings, PatchSpec, ProceduralV3Settings, SharedEdgeSettings,
    V3CavesSettings, V3DeepForestSettings, V3EnvironmentSettings, V3ForestSettings, V3FortSettings,
    V3HillsSettings, V3LayoutSettings, V3MountainsSettings, V3PrairieSettings, V3RecipeSettings,
    V3Ring7Settings, V3SkyIslandsSettings, V3VolcanoSettings, V3WaterfallSettings,
    WalkerPortSettings,
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
fn deep_forest_stitched_patch_keeps_authored_volumes_out_of_protected_approaches() {
    let (settings, _) = dry_ring_settings();
    let layout = resolve_layout(33, &settings).expect("the dry Ring7 fixture should resolve");
    let context = patch(&layout, 3).expect("the vegetation slot should resolve");
    let protected = context.protected_approaches();
    assert!(
        !protected.is_empty(),
        "the stitched regression requires exact protected seam approaches"
    );
    let recipe = V3DeepForestSettings {
        base_level: 15,
        max_relief: 4,
        blocker_coverage_percent: 30,
        clearing_count: 3,
    };
    let catalog = super::vegetation::tests::runtime_art_catalog();
    let plan = deep_forest::construct_patch(
        context,
        &recipe,
        V3EnvironmentSettings::TemperateGrassland,
        LEVEL_HEIGHT,
        PatchBuildMode::Candidate {
            world_seed: 1_592_598_566,
            candidate: 0,
        },
        catalog,
    )
    .expect("Deep Forest should construct inside a stitched vegetation patch");
    assert!(plan.features.by_id.values().all(|feature| feature
        .blocker_footprint
        .iter()
        .all(|blocker| !protected.contains(&blocker.coord))));
    let metrics = match deep_forest::validate_patch(context, &recipe, &plan, catalog) {
        super::selection::WorldValidation::Valid(metrics) => metrics,
        super::selection::WorldValidation::Invalid(issues) => {
            panic!(
                "Deep Forest construction and patch validation must share exact authored-volume \
                 eligibility: {issues:?}"
            );
        }
    };
    assert_eq!(metrics.clearing_count, 3);
    assert!((28..=32).contains(&metrics.blocker_coverage_percent));
    assert_eq!(
        metrics.tree_blocker_surfaces,
        u32::try_from(plan.blockers.len()).unwrap_or(u32::MAX)
    );
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

#[test]
fn center_hills_routes_three_inlets_into_one_outlet() {
    let (mut settings, recipes) = dry_ring_settings();
    let V3LayoutSettings::Ring7(ring) = &mut settings.layout else {
        panic!("the fixture must remain Ring7");
    };
    ring.center.edges.west = shared_edge(EdgeLiquidSettings::Inlet(EdgeLiquidPortSettings {
        width: 3,
    }));
    ring.caves.edges.east = shared_edge(EdgeLiquidSettings::Outlet(EdgeLiquidPortSettings {
        width: 3,
    }));
    ring.center.edges.north_east = shared_edge(EdgeLiquidSettings::Inlet(EdgeLiquidPortSettings {
        width: 3,
    }));
    ring.mountains.edges.south_west =
        shared_edge(EdgeLiquidSettings::Outlet(EdgeLiquidPortSettings {
            width: 3,
        }));
    ring.center.edges.north_west = shared_edge(EdgeLiquidSettings::Inlet(EdgeLiquidPortSettings {
        width: 3,
    }));
    ring.sky_islands.edges.south_east =
        shared_edge(EdgeLiquidSettings::Outlet(EdgeLiquidPortSettings {
            width: 3,
        }));
    ring.center.edges.south_west =
        shared_edge(EdgeLiquidSettings::Outlet(EdgeLiquidPortSettings {
            width: 3,
        }));
    ring.fort.edges.north_east = shared_edge(EdgeLiquidSettings::Inlet(EdgeLiquidPortSettings {
        width: 3,
    }));

    let layout = resolve_layout(33, &settings).expect("the confluence fixture should resolve");
    let context = patch(&layout, 0).expect("center patch");
    let mode = PatchBuildMode::CanonicalFallback;
    let first = hills::construct_patch_with_catalog(
        context,
        &recipes.hills,
        V3EnvironmentSettings::TemperateGrassland,
        LEVEL_HEIGHT,
        mode,
        super::vegetation::tests::runtime_art_catalog(),
    )
    .expect("central Hills should construct a confluence");
    let second = hills::construct_patch_with_catalog(
        context,
        &recipes.hills,
        V3EnvironmentSettings::TemperateGrassland,
        LEVEL_HEIGHT,
        mode,
        super::vegetation::tests::runtime_art_catalog(),
    )
    .expect("the same confluence should construct again");
    assert_eq!(first.volume, second.volume);
    assert_eq!(first.liquids, second.liquids);
    assert_strict_patch(&layout, first.clone());
    let seeded = hills::construct_patch_with_catalog(
        context,
        &recipes.hills,
        V3EnvironmentSettings::TemperateGrassland,
        LEVEL_HEIGHT,
        PatchBuildMode::Candidate {
            world_seed: 1_592_598_566,
            candidate: 0,
        },
        super::vegetation::tests::runtime_art_catalog(),
    )
    .expect("the exact Ring19 confluence should construct as a seeded candidate");
    assert_strict_patch(&layout, seeded.clone());
    match hills::validate_patch(
        context,
        &seeded,
        &recipes.hills,
        V3EnvironmentSettings::TemperateGrassland,
        super::vegetation::tests::runtime_art_catalog(),
    ) {
        super::selection::WorldValidation::Valid(_) => {}
        super::selection::WorldValidation::Invalid(issues) => {
            panic!("the seeded Ring19 confluence must validate: {issues:?}");
        }
    }
    let mut revised_hills = recipes.hills.clone();
    revised_hills.max_relief = 12;
    for mode in [
        PatchBuildMode::CanonicalFallback,
        PatchBuildMode::Candidate {
            world_seed: 1_592_598_566,
            candidate: 0,
        },
    ] {
        let revised = hills::construct_patch_with_catalog(
            context,
            &revised_hills,
            V3EnvironmentSettings::TemperateGrassland,
            LEVEL_HEIGHT,
            mode,
            super::vegetation::tests::runtime_art_catalog(),
        )
        .expect("the revised relief-12 Hills confluence should construct");
        assert_strict_patch(&layout, revised.clone());
        let ordinary_without_blockers = revised
            .volume
            .surfaces
            .iter()
            .filter_map(|(position, metadata)| {
                (metadata.access == super::volume::SurfaceAccess::Ordinary).then_some(*position)
            })
            .collect::<BTreeSet<_>>();
        let ordinary_with_blockers = ordinary_without_blockers
            .difference(&revised.blockers)
            .copied()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            test_surface_components(&ordinary_without_blockers).len(),
            1,
            "seam closure must not recreate the observed 351+15 surface split"
        );
        assert_eq!(
            test_surface_components(&ordinary_with_blockers).len(),
            1,
            "authored vegetation must preserve the repaired confluence network"
        );
        let metrics = match hills::validate_patch(
            context,
            &revised,
            &revised_hills,
            V3EnvironmentSettings::TemperateGrassland,
            super::vegetation::tests::runtime_art_catalog(),
        ) {
            super::selection::WorldValidation::Valid(metrics) => metrics,
            super::selection::WorldValidation::Invalid(issues) => {
                panic!("the revised relief-12 Hills confluence must validate: {issues:?}");
            }
        };
        assert!(
            metrics.relief >= 10,
            "the revised confluence must retain at least ten reachable relief levels"
        );
    }

    let body = first
        .liquids
        .bodies
        .values()
        .next()
        .expect("Hills confluence body");
    let nodes_by_coord = body
        .nodes
        .iter()
        .map(|(position, node)| (position.coord, (*position, *node)))
        .collect::<BTreeMap<_, _>>();
    let mut inlets = Vec::new();
    let mut outlets = BTreeSet::new();
    let mut liquid_approaches = BTreeSet::new();
    for edge in context.shared_edges() {
        let Some((source, port)) = edge.liquid_port() else {
            continue;
        };
        let boundary = port
            .lanes
            .iter()
            .map(|(coord, _)| *coord)
            .collect::<BTreeSet<_>>();
        liquid_approaches.extend(port.first_approach.iter().copied());
        assert!(port
            .first_approach
            .iter()
            .all(|coord| nodes_by_coord.contains_key(coord)));
        if source {
            outlets.extend(boundary);
        } else {
            inlets.extend(boundary);
        }
    }
    assert_eq!(inlets.len(), 9);
    assert_eq!(outlets.len(), 3);
    assert!(outlets.iter().all(|coord| {
        nodes_by_coord.get(coord).is_some_and(|(_, node)| {
            node.state == LiquidFlowState::Still && node.downstream.is_none()
        })
    }));
    assert!(inlets.iter().all(|coord| {
        let Some((mut position, _)) = nodes_by_coord.get(coord).copied() else {
            return false;
        };
        let mut visited = BTreeSet::new();
        loop {
            if !visited.insert(position) {
                return false;
            }
            if outlets.contains(&position.coord) {
                break true;
            }
            let Some(next) = body.nodes.get(&position).and_then(|node| node.downstream) else {
                break false;
            };
            position = next;
        }
    }));
    let mut indegree = BTreeMap::<TilePos, usize>::new();
    for downstream in body.nodes.values().filter_map(|node| node.downstream) {
        *indegree.entry(downstream).or_default() += 1;
    }
    assert!(indegree.values().any(|count| *count > 1));

    let walker_only_approaches = context
        .protected_approaches()
        .difference(&liquid_approaches)
        .copied()
        .collect::<BTreeSet<_>>();
    assert!(walker_only_approaches
        .is_disjoint(&nodes_by_coord.keys().copied().collect::<BTreeSet<_>>()));
    match hills::validate_patch(
        context,
        &first,
        &recipes.hills,
        V3EnvironmentSettings::TemperateGrassland,
        super::vegetation::tests::runtime_art_catalog(),
    ) {
        super::selection::WorldValidation::Valid(_) => {}
        super::selection::WorldValidation::Invalid(issues) => {
            panic!("the confluence must pass Hills validation: {issues:?}");
        }
    }

    let body_id = *first
        .liquids
        .bodies
        .keys()
        .next()
        .expect("Hills confluence body id");
    let inlet_position = nodes_by_coord
        .get(inlets.first().expect("an inlet coordinate"))
        .map(|(position, _)| *position)
        .expect("inlet position");
    let outlet_position = nodes_by_coord
        .get(outlets.first().expect("an outlet coordinate"))
        .map(|(position, _)| *position)
        .expect("outlet position");
    let validate = |plan: &GeneratedPatchPlan| {
        hills::validate_patch(
            context,
            plan,
            &recipes.hills,
            V3EnvironmentSettings::TemperateGrassland,
            super::vegetation::tests::runtime_art_catalog(),
        )
    };
    let (mut unsupported_settings, _) = dry_ring_settings();
    let V3LayoutSettings::Ring7(unsupported_ring) = &mut unsupported_settings.layout else {
        panic!("the fixture must remain Ring7");
    };
    for side in HexSide::ALL {
        let (center, outer) = if side == HexSide::SouthWest {
            (
                EdgeLiquidSettings::Outlet(EdgeLiquidPortSettings { width: 3 }),
                EdgeLiquidSettings::Inlet(EdgeLiquidPortSettings { width: 3 }),
            )
        } else if matches!(side, HexSide::East | HexSide::West | HexSide::NorthWest) {
            (
                EdgeLiquidSettings::Inlet(EdgeLiquidPortSettings { width: 3 }),
                EdgeLiquidSettings::Outlet(EdgeLiquidPortSettings { width: 3 }),
            )
        } else {
            (EdgeLiquidSettings::Dry, EdgeLiquidSettings::Dry)
        };
        set_center_liquid_pair(unsupported_ring, side, center, outer);
    }
    let unsupported_layout =
        resolve_layout(33, &unsupported_settings).expect("unsupported topology still resolves");
    let unsupported_context = patch(&unsupported_layout, 0).expect("unsupported center patch");
    assert_validation_rejects_with(
        hills::validate_patch(
            unsupported_context,
            &first,
            &recipes.hills,
            V3EnvironmentSettings::TemperateGrassland,
            super::vegetation::tests::runtime_art_catalog(),
        ),
        "requires an inlet opposite its",
    );

    let fill_coords = first
        .volume
        .fill_runs_by_top()
        .keys()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    let bridge = first
        .features
        .protected_routes
        .get("bridge_crossing")
        .expect("main bridge route");
    let alternate = first
        .features
        .protected_routes
        .get("alternate_crossing")
        .expect("main alternate route");
    assert_eq!(bridge.surfaces.len(), 14);
    assert_eq!(alternate.surfaces.len(), 14);
    let main_crossings = bridge
        .surfaces
        .union(&alternate.surfaces)
        .copied()
        .collect::<BTreeSet<_>>();
    let auxiliary_crossings = first
        .volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| {
            (metadata.access == super::volume::SurfaceAccess::Ordinary
                && fill_coords.contains(&position.coord)
                && !main_crossings.contains(position))
            .then_some(*position)
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(auxiliary_crossings.len(), 2);
    let walkable = first
        .volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| {
            (metadata.access == super::volume::SurfaceAccess::Ordinary
                && !first.blockers.contains(position))
            .then_some(*position)
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        test_surface_components(&walkable).len(),
        1,
        "the unmodified confluence walker network must be connected"
    );
    for auxiliary in &auxiliary_crossings {
        let mut without_auxiliary = walkable.clone();
        without_auxiliary.remove(auxiliary);
        assert!(
            test_surface_components(&without_auxiliary).len() > 1,
            "selected auxiliary crossing {auxiliary:?} must be indispensable"
        );
    }

    let auxiliary = *auxiliary_crossings
        .first()
        .expect("one exact auxiliary tributary crossing");
    let mut misclassified_auxiliary = first.clone();
    misclassified_auxiliary
        .features
        .protected_routes
        .get_mut("alternate_crossing")
        .expect("alternate route")
        .surfaces
        .insert(auxiliary);
    assert_validation_rejects_with(
        validate(&misclassified_auxiliary),
        "cannot rederive auxiliary tributary crossings",
    );

    let mut wrong_auxiliary_material = first.clone();
    let auxiliary_support = wrong_auxiliary_material
        .volume
        .columns
        .get_mut(&auxiliary.coord)
        .expect("auxiliary causeway column")
        .elements
        .iter_mut()
        .find_map(|element| match element {
            super::volume::VolumeElement::Solid(solid)
                if solid.levels.bottom <= auxiliary.level && auxiliary.level < solid.levels.top =>
            {
                Some(solid)
            }
            _ => None,
        })
        .expect("auxiliary causeway support");
    auxiliary_support.material = super::volume::SolidMaterialRole::Stone;
    assert_validation_rejects_with(
        validate(&wrong_auxiliary_material),
        "does not use the exact causeway material",
    );

    let mut missing_auxiliary_membership = first.clone();
    missing_auxiliary_membership
        .biome_regions
        .remove(&auxiliary)
        .expect("auxiliary biome-region membership");
    assert_validation_rejects_with(
        validate(&missing_auxiliary_membership),
        "missing exact biome-region membership",
    );

    let mut auxiliary_support_gap = first.clone();
    let auxiliary_fill = auxiliary_support_gap
        .volume
        .columns
        .get_mut(&auxiliary.coord)
        .expect("auxiliary causeway column")
        .elements
        .iter_mut()
        .find_map(|element| match element {
            super::volume::VolumeElement::Fill(fill) => Some(fill),
            _ => None,
        })
        .expect("auxiliary liquid fill");
    auxiliary_fill.levels = super::volume::LevelInterval::new(
        auxiliary_fill.levels.bottom,
        auxiliary_fill.levels.top.saturating_sub(1),
    );
    assert_validation_rejects_with(validate(&auxiliary_support_gap), "unsupported vertical gap");

    let moved_coord = auxiliary
        .coord
        .neighbors()
        .into_iter()
        .find(|coord| {
            fill_coords.contains(coord)
                && !auxiliary_crossings
                    .iter()
                    .any(|surface| surface.coord == *coord)
                && !main_crossings.iter().any(|surface| surface.coord == *coord)
                && !first.volume.surfaces.iter().any(|(surface, metadata)| {
                    surface.coord == *coord
                        && metadata.access == super::volume::SurfaceAccess::Ordinary
                })
        })
        .expect("a neighboring supported tributary liquid cell");
    let moved_position = TilePos::new(moved_coord, auxiliary.level);
    let mut moved_auxiliary = first.clone();
    let auxiliary_metadata = moved_auxiliary
        .volume
        .surfaces
        .remove(&auxiliary)
        .expect("old auxiliary surface metadata");
    let auxiliary_biome = moved_auxiliary
        .biome_regions
        .remove(&auxiliary)
        .expect("old auxiliary biome membership");
    moved_auxiliary
        .volume
        .columns
        .get_mut(&auxiliary.coord)
        .expect("old auxiliary column")
        .elements
        .retain(|element| {
            !matches!(
                element,
                super::volume::VolumeElement::Solid(solid)
                    if solid.material == super::volume::SolidMaterialRole::Gravel
                        && solid.levels.bottom <= auxiliary.level
                        && auxiliary.level < solid.levels.top
            )
        });
    moved_auxiliary
        .volume
        .columns
        .get_mut(&moved_coord)
        .expect("new auxiliary column")
        .elements
        .push(super::volume::VolumeElement::Solid(
            super::volume::SolidMass {
                levels: super::volume::LevelInterval::new(
                    moved_position.level,
                    moved_position.level.saturating_add(1),
                ),
                material: super::volume::SolidMaterialRole::Gravel,
                cutaway_for: None,
            },
        ));
    assert!(moved_auxiliary
        .volume
        .surfaces
        .insert(moved_position, auxiliary_metadata)
        .is_none());
    assert!(moved_auxiliary
        .biome_regions
        .insert(moved_position, auxiliary_biome)
        .is_none());
    assert_validation_rejects_with(
        validate(&moved_auxiliary),
        "do not match their rederived liquid-branch authority",
    );

    let shifted_position = TilePos::new(auxiliary.coord, auxiliary.level.saturating_add(1));
    let mut shifted_auxiliary = first.clone();
    let auxiliary_metadata = shifted_auxiliary
        .volume
        .surfaces
        .remove(&auxiliary)
        .expect("old auxiliary surface metadata");
    let auxiliary_biome = shifted_auxiliary
        .biome_regions
        .remove(&auxiliary)
        .expect("old auxiliary biome membership");
    let auxiliary_support = shifted_auxiliary
        .volume
        .columns
        .get_mut(&auxiliary.coord)
        .expect("auxiliary causeway column")
        .elements
        .iter_mut()
        .find_map(|element| match element {
            super::volume::VolumeElement::Solid(solid)
                if solid.material == super::volume::SolidMaterialRole::Gravel
                    && solid.levels.bottom <= auxiliary.level
                    && auxiliary.level < solid.levels.top =>
            {
                Some(solid)
            }
            _ => None,
        })
        .expect("auxiliary causeway support");
    auxiliary_support.levels = super::volume::LevelInterval::new(
        shifted_position.level,
        shifted_position.level.saturating_add(1),
    );
    assert!(shifted_auxiliary
        .volume
        .surfaces
        .insert(shifted_position, auxiliary_metadata)
        .is_none());
    assert!(shifted_auxiliary
        .biome_regions
        .insert(shifted_position, auxiliary_biome)
        .is_none());
    assert_validation_rejects_with(
        validate(&shifted_auxiliary),
        "do not match their rederived liquid-branch authority",
    );

    let mut missing_inlet = first.clone();
    missing_inlet
        .liquids
        .bodies
        .get_mut(&body_id)
        .expect("body")
        .nodes
        .remove(&inlet_position);
    assert_validation_rejects_with(validate(&missing_inlet), "liquid approach");

    let mut stopped_inlet = first.clone();
    let inlet_node = stopped_inlet
        .liquids
        .bodies
        .get_mut(&body_id)
        .expect("body")
        .nodes
        .get_mut(&inlet_position)
        .expect("inlet");
    inlet_node.state = LiquidFlowState::Still;
    inlet_node.downstream = None;
    assert_validation_rejects_with(validate(&stopped_inlet), "does not flow into the patch");

    let mut flowing_outlet = first.clone();
    let outlet_node = flowing_outlet
        .liquids
        .bodies
        .get_mut(&body_id)
        .expect("body")
        .nodes
        .get_mut(&outlet_position)
        .expect("outlet");
    outlet_node.state = LiquidFlowState::Current;
    outlet_node.downstream = Some(inlet_position);
    assert_validation_rejects_with(
        validate(&flowing_outlet),
        "must be Still before composition",
    );

    let mut cycle = first.clone();
    let internal = cycle
        .liquids
        .bodies
        .get(&body_id)
        .expect("body")
        .nodes
        .keys()
        .copied()
        .find(|position| !outlets.contains(&position.coord) && position != &inlet_position)
        .expect("internal flow node");
    cycle
        .liquids
        .bodies
        .get_mut(&body_id)
        .expect("body")
        .nodes
        .get_mut(&internal)
        .expect("internal flow node")
        .downstream = Some(internal);
    assert_validation_rejects_with(validate(&cycle), "internal terminal or cycle");

    let undeclared_boundary = context
        .shared_edges()
        .filter(|edge| edge.liquid_port().is_none())
        .flat_map(|edge| edge.boundary_pairs())
        .map(|(inside, _)| inside)
        .find(|coord| !nodes_by_coord.contains_key(coord))
        .expect("a dry shared-boundary coordinate");
    let mut leaked = first.clone();
    leaked
        .liquids
        .bodies
        .get_mut(&body_id)
        .expect("body")
        .nodes
        .insert(
            TilePos::new(undeclared_boundary, inlet_position.level),
            super::liquid::LiquidNode {
                state: LiquidFlowState::Still,
                downstream: None,
            },
        );
    assert_validation_rejects_with(validate(&leaked), "undeclared shared-boundary cells");

    let mut wrong_level = first;
    let displaced = wrong_level
        .liquids
        .bodies
        .get_mut(&body_id)
        .expect("body")
        .nodes
        .remove(&outlet_position)
        .expect("outlet node");
    wrong_level
        .liquids
        .bodies
        .get_mut(&body_id)
        .expect("body")
        .nodes
        .insert(
            TilePos::new(
                outlet_position.coord,
                outlet_position.level.saturating_add(20),
            ),
            displaced,
        );
    assert_validation_rejects_with(validate(&wrong_level), "liquid approach");
}

#[test]
fn center_hills_confluence_topology_policy_is_exhaustive() {
    let mut supported = 0_usize;
    let mut rejected = 0_usize;
    let catalog = super::vegetation::tests::runtime_art_catalog();

    for (outlet_index, outlet) in HexSide::ALL.into_iter().enumerate() {
        for inlet_bits in 0_u8..(1_u8 << HexSide::ALL.len()) {
            if inlet_bits & (1_u8 << outlet_index) != 0
                || !(2..=5).contains(&inlet_bits.count_ones())
            {
                continue;
            }
            let (mut settings, recipes) = dry_ring_settings();
            let V3LayoutSettings::Ring7(ring) = &mut settings.layout else {
                panic!("the fixture must remain Ring7");
            };
            for (side_index, side) in HexSide::ALL.into_iter().enumerate() {
                let (center, outer) = if side == outlet {
                    (
                        EdgeLiquidSettings::Outlet(EdgeLiquidPortSettings { width: 3 }),
                        EdgeLiquidSettings::Inlet(EdgeLiquidPortSettings { width: 3 }),
                    )
                } else if inlet_bits & (1_u8 << side_index) != 0 {
                    (
                        EdgeLiquidSettings::Inlet(EdgeLiquidPortSettings { width: 3 }),
                        EdgeLiquidSettings::Outlet(EdgeLiquidPortSettings { width: 3 }),
                    )
                } else {
                    (EdgeLiquidSettings::Dry, EdgeLiquidSettings::Dry)
                };
                set_center_liquid_pair(ring, side, center, outer);
            }

            let layout =
                resolve_layout(33, &settings).expect("the settings-valid topology should resolve");
            let context = patch(&layout, 0).expect("center patch");
            let result = hills::construct_patch_with_catalog(
                context,
                &recipes.hills,
                V3EnvironmentSettings::TemperateGrassland,
                LEVEL_HEIGHT,
                PatchBuildMode::CanonicalFallback,
                catalog,
            );
            let opposite_index = HexSide::ALL
                .iter()
                .position(|side| *side == outlet.opposite())
                .expect("every side has one opposite");
            if inlet_bits & (1_u8 << opposite_index) != 0 {
                let plan = result.unwrap_or_else(|issues| {
                    panic!(
                        "supported confluence outlet {outlet:?}, inlet bits {inlet_bits:06b} failed construction: {issues:?}"
                    )
                });
                match hills::validate_patch(
                    context,
                    &plan,
                    &recipes.hills,
                    V3EnvironmentSettings::TemperateGrassland,
                    catalog,
                ) {
                    super::selection::WorldValidation::Valid(_) => {}
                    super::selection::WorldValidation::Invalid(issues) => panic!(
                        "supported confluence outlet {outlet:?}, inlet bits {inlet_bits:06b} failed validation: {issues:?}"
                    ),
                }
                supported = supported.saturating_add(1);
            } else {
                let issues = result.expect_err(
                    "a confluence without an opposite inlet must fail its explicit recipe policy",
                );
                assert!(
                    issues.iter().any(|issue| issue
                        .detail
                        .contains("requires an inlet opposite its")),
                    "unexpected policy error for outlet {outlet:?}, inlet bits {inlet_bits:06b}: {issues:?}"
                );
                rejected = rejected.saturating_add(1);
            }
        }
    }

    assert_eq!(supported, 90);
    assert_eq!(rejected, 66);
}

#[test]
fn western_composite_volcano_revalidates_its_exact_outlet_authority() {
    let (settings, _) = dry_ring_settings();
    let layout = resolve_layout(33, &settings).expect("the dry Ring7 fixture should resolve");
    let context = patch(&layout, 5).expect("western outer patch");
    assert!(context.is_world_boundary(HexSide::West));
    let volcano_settings = V3VolcanoSettings {
        base_level: 15,
        summit_relief: 20,
        massif_coverage_percent: 25,
        bridge_clearance: 4,
    };
    let baseline = volcano::construct_patch(
        context,
        &volcano_settings,
        LEVEL_HEIGHT,
        PatchBuildMode::CanonicalFallback,
    )
    .expect("Volcano should construct in the western outer patch");
    assert_strict_patch(&layout, baseline.clone());
    match volcano::validate_patch(context, &baseline, &volcano_settings) {
        super::selection::WorldValidation::Valid(metrics) => {
            assert_eq!(metrics.summit_relief, 20);
            assert_eq!(metrics.bridge_clearance, 4);
        }
        super::selection::WorldValidation::Invalid(issues) => {
            panic!("the resolved composite Volcano must validate: {issues:?}");
        }
    }

    let body_id = *baseline
        .liquids
        .bodies
        .keys()
        .next()
        .expect("Volcano lava body");
    let terminals = baseline
        .liquids
        .bodies
        .get(&body_id)
        .expect("Volcano lava body")
        .nodes
        .iter()
        .filter_map(|(position, node)| node.downstream.is_none().then_some(*position))
        .collect::<BTreeSet<_>>();
    assert_eq!(terminals.len(), 3);
    assert!(terminal_coords_are_connected(&terminals));
    assert_eq!(
        terminals
            .iter()
            .map(|position| position.level)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([volcano_settings
            .base_level
            .saturating_add(3)
            .saturating_sub(volcano_settings.bridge_clearance)
            .max(3),])
    );
    assert!(terminals.iter().all(|position| !context
        .mask()
        .contains(&HexSide::West.neighbor(position.coord))));
    let validate =
        |plan: &GeneratedPatchPlan| volcano::validate_patch(context, plan, &volcano_settings);

    let terminal = *terminals.first().expect("one terminal");
    let mut missing_terminal = baseline.clone();
    missing_terminal
        .liquids
        .bodies
        .get_mut(&body_id)
        .expect("lava body")
        .nodes
        .remove(&terminal);
    assert_validation_rejects_with(validate(&missing_terminal), "expected exactly 3");

    let mut moving_terminal = baseline.clone();
    moving_terminal
        .liquids
        .bodies
        .get_mut(&body_id)
        .expect("lava body")
        .nodes
        .get_mut(&terminal)
        .expect("terminal")
        .state = LiquidFlowState::Current;
    assert_validation_rejects_with(validate(&moving_terminal), "is not Still");

    let mut extra_terminal = baseline.clone();
    let flowing = extra_terminal
        .liquids
        .bodies
        .get(&body_id)
        .expect("lava body")
        .nodes
        .iter()
        .find_map(|(position, node)| node.downstream.is_some().then_some(*position))
        .expect("one flowing lava node");
    let flowing_node = extra_terminal
        .liquids
        .bodies
        .get_mut(&body_id)
        .expect("lava body")
        .nodes
        .get_mut(&flowing)
        .expect("flowing node");
    flowing_node.state = LiquidFlowState::Still;
    flowing_node.downstream = None;
    assert_validation_rejects_with(validate(&extra_terminal), "expected exactly 3");

    let mut wrong_terminal_level = baseline.clone();
    let shifted_terminal = TilePos::new(terminal.coord, terminal.level.saturating_add(1));
    let terminal_node = wrong_terminal_level
        .liquids
        .bodies
        .get_mut(&body_id)
        .expect("lava body")
        .nodes
        .remove(&terminal)
        .expect("terminal node");
    let terminal_predecessor = wrong_terminal_level
        .liquids
        .bodies
        .get_mut(&body_id)
        .expect("lava body")
        .nodes
        .values_mut()
        .find(|node| node.downstream == Some(terminal))
        .expect("terminal predecessor");
    terminal_predecessor.downstream = Some(shifted_terminal);
    assert!(wrong_terminal_level
        .liquids
        .bodies
        .get_mut(&body_id)
        .expect("lava body")
        .nodes
        .insert(shifted_terminal, terminal_node)
        .is_none());
    assert_validation_rejects_with(
        validate(&wrong_terminal_level),
        "exact contiguous three-lane western outlet positions",
    );

    let mut relocated_terminals = baseline.clone();
    for terminal in &terminals {
        reverse_lava_terminal_tail(
            relocated_terminals
                .liquids
                .bodies
                .get_mut(&body_id)
                .expect("lava body"),
            *terminal,
            1,
        );
    }
    let relocated_positions = lava_terminals(
        relocated_terminals
            .liquids
            .bodies
            .get(&body_id)
            .expect("lava body"),
    );
    assert_eq!(relocated_positions.len(), terminals.len());
    assert_ne!(relocated_positions, terminals);
    assert!(terminal_coords_are_connected(&relocated_positions));
    assert_validation_rejects_with(
        validate(&relocated_terminals),
        "exact contiguous three-lane western outlet positions",
    );

    let noncontiguous_terminals = (2..=6)
        .find_map(|steps| {
            let mut corrupted = baseline.clone();
            reverse_lava_terminal_tail(
                corrupted
                    .liquids
                    .bodies
                    .get_mut(&body_id)
                    .expect("lava body"),
                terminal,
                steps,
            );
            let relocated =
                lava_terminals(corrupted.liquids.bodies.get(&body_id).expect("lava body"));
            (!terminal_coords_are_connected(&relocated)).then_some(corrupted)
        })
        .expect("moving one terminal inward must break the exact contiguous lane set");
    assert_validation_rejects_with(
        validate(&noncontiguous_terminals),
        "exact contiguous three-lane western outlet positions",
    );

    let bridge_id = baseline
        .structures
        .by_id
        .iter()
        .find_map(|(id, structure)| {
            (structure.kind == super::world::StructureKind::Bridge).then_some(*id)
        })
        .expect("bridge structure");
    let bridge_voxel = *baseline
        .structures
        .by_id
        .get(&bridge_id)
        .and_then(|structure| structure.voxels.first())
        .expect("bridge voxel");
    let mut missing_bridge_membership = baseline.clone();
    missing_bridge_membership
        .structures
        .by_id
        .get_mut(&bridge_id)
        .expect("bridge structure")
        .voxels
        .remove(&bridge_voxel);
    assert_validation_rejects_with(
        validate(&missing_bridge_membership),
        "exact oriented 2-by-3 deck authority",
    );

    let stair_id = baseline
        .structures
        .by_id
        .iter()
        .find_map(|(id, structure)| {
            (structure.kind == super::world::StructureKind::Stair).then_some(*id)
        })
        .expect("stair structure");
    let stair_voxel = *baseline
        .structures
        .by_id
        .get(&stair_id)
        .and_then(|structure| structure.voxels.first())
        .expect("stair voxel");
    let mut missing_stair_membership = baseline.clone();
    missing_stair_membership
        .structures
        .by_id
        .get_mut(&stair_id)
        .expect("stair structure")
        .voxels
        .remove(&stair_voxel);
    assert_validation_rejects_with(
        validate(&missing_stair_membership),
        "rederived worked-stone surfaces",
    );

    let mut narrowed_route = baseline.clone();
    let route = narrowed_route
        .features
        .protected_routes
        .get_mut("bridge_route")
        .expect("protected bridge route");
    let centerline = route.centerline.iter().copied().collect::<BTreeSet<_>>();
    let second_lane = route
        .surfaces
        .difference(&centerline)
        .copied()
        .next()
        .expect("a second-lane route surface");
    route.surfaces.remove(&second_lane);
    assert_validation_rejects_with(
        validate(&narrowed_route),
        "exact two-wide ordinary stair approach",
    );

    let mut missing_west_layout = layout.clone();
    let shared_reference = missing_west_layout
        .patches
        .get(&PatchId(5))
        .and_then(|patch| patch.edges.get(&HexSide::East))
        .copied()
        .expect("western patch east seam");
    assert!(matches!(shared_reference, ResolvedEdgeReference::Shared(_)));
    missing_west_layout
        .patches
        .get_mut(&PatchId(5))
        .expect("western patch")
        .edges
        .insert(HexSide::West, shared_reference);
    let missing_west =
        PatchRecipeContext::resolve(&missing_west_layout, PatchId(5)).expect("corrupt context");
    assert_validation_rejects_with(
        volcano::validate_patch(missing_west, &baseline, &volcano_settings),
        "western world-boundary outlet",
    );

    let (mut wet_settings, _) = dry_ring_settings();
    let V3LayoutSettings::Ring7(ring) = &mut wet_settings.layout else {
        panic!("the fixture must remain Ring7");
    };
    ring.center.edges.west = shared_edge(EdgeLiquidSettings::Inlet(EdgeLiquidPortSettings {
        width: 3,
    }));
    ring.caves.edges.east = shared_edge(EdgeLiquidSettings::Outlet(EdgeLiquidPortSettings {
        width: 3,
    }));
    let wet_layout =
        resolve_layout(33, &wet_settings).expect("the wet western seam should resolve");
    let wet_context = patch(&wet_layout, 5).expect("wet western patch");
    let construction_issues = volcano::construct_patch(
        wet_context,
        &volcano_settings,
        LEVEL_HEIGHT,
        PatchBuildMode::CanonicalFallback,
    )
    .expect_err("Volcano must reject a stitched liquid seam");
    assert!(construction_issues
        .iter()
        .any(|issue| issue.detail.contains("separate from stitched liquid ports")));
    assert_validation_rejects_with(
        volcano::validate_patch(wet_context, &baseline, &volcano_settings),
        "separate from stitched liquid ports",
    );
}

fn lava_terminals(body: &super::liquid::LiquidBodyPlan) -> BTreeSet<TilePos> {
    body.nodes
        .iter()
        .filter_map(|(position, node)| node.downstream.is_none().then_some(*position))
        .collect()
}

fn reverse_lava_terminal_tail(
    body: &mut super::liquid::LiquidBodyPlan,
    terminal: TilePos,
    steps: usize,
) {
    let mut reversed = vec![terminal];
    for _ in 0..steps {
        let current = *reversed.last().expect("terminal tail");
        let predecessor = body
            .nodes
            .iter()
            .find_map(|(position, node)| (node.downstream == Some(current)).then_some(*position))
            .expect("lava terminal predecessor");
        reversed.push(predecessor);
    }
    for pair in reversed.windows(2) {
        let [from, to] = pair else {
            unreachable!("windows are exact pairs");
        };
        let node = body.nodes.get_mut(from).expect("reversed lava node");
        node.state = LiquidFlowState::Current;
        node.downstream = Some(*to);
    }
    let relocated = *reversed.last().expect("relocated terminal");
    let node = body
        .nodes
        .get_mut(&relocated)
        .expect("relocated lava terminal");
    node.state = LiquidFlowState::Still;
    node.downstream = None;
}

fn terminal_coords_are_connected(terminals: &BTreeSet<TilePos>) -> bool {
    let coords = terminals
        .iter()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    let Some(start) = coords.first().copied() else {
        return false;
    };
    let mut reachable = BTreeSet::from([start]);
    let mut frontier = VecDeque::from([start]);
    while let Some(coord) = frontier.pop_front() {
        for neighbor in coord.neighbors() {
            if coords.contains(&neighbor) && reachable.insert(neighbor) {
                frontier.push_back(neighbor);
            }
        }
    }
    reachable.len() == coords.len()
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

fn set_center_liquid_pair(
    ring: &mut V3Ring7Settings,
    side: HexSide,
    center: EdgeLiquidSettings,
    outer: EdgeLiquidSettings,
) {
    let center = shared_edge(center);
    let outer = shared_edge(outer);
    match side {
        HexSide::NorthEast => {
            ring.center.edges.north_east = center;
            ring.mountains.edges.south_west = outer;
        }
        HexSide::East => {
            ring.center.edges.east = center;
            ring.waterfall.edges.west = outer;
        }
        HexSide::SouthEast => {
            ring.center.edges.south_east = center;
            ring.forest.edges.north_west = outer;
        }
        HexSide::SouthWest => {
            ring.center.edges.south_west = center;
            ring.fort.edges.north_east = outer;
        }
        HexSide::West => {
            ring.center.edges.west = center;
            ring.caves.edges.east = outer;
        }
        HexSide::NorthWest => {
            ring.center.edges.north_west = center;
            ring.sky_islands.edges.south_east = outer;
        }
    }
}
