//! Strict, versioned public measurements of one validated schematic plan.

use std::collections::{BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::model::{
    AccessIntent, BoundedRegionKind, BoundedRegionRule, BoundedTarget, ClimateKind, FeatureKind,
    LandformKind, SchematicCoord, SchematicPlanV1, SchematicTemplateV1, StableId, SurfaceKind,
    VegetationDensity,
};

/// Exact counts for the mutually exclusive surface layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceCountsV1 {
    /// Dry land cells.
    pub land: u16,
    /// Open-water cells, including ocean and lakes.
    pub open_water: u16,
}

/// Exact counts for the mutually exclusive optional landform layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LandformCountsV1 {
    /// Cells with no landform because their surface is open water.
    pub none: u16,
    /// Scenic or lake-island cells.
    pub island: u16,
    /// Low sandy coastal cells.
    pub beach: u16,
    /// Raised coastal cells.
    pub shore: u16,
    /// Valley cells.
    pub valley: u16,
    /// Plateau cells.
    pub plateau: u16,
    /// Hill cells.
    pub hill: u16,
    /// Mountain cells.
    pub mountain: u16,
    /// Broad massif cells.
    pub massif: u16,
    /// Sharp peak cells.
    pub sharp_peak: u16,
}

/// Exact counts for the mutually exclusive climate layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClimateCountsV1 {
    /// Marine cells.
    pub marine: u16,
    /// Temperate cells.
    pub temperate: u16,
    /// Alpine cells.
    pub alpine: u16,
    /// Frozen cells.
    pub frozen: u16,
}

/// Exact counts for the mutually exclusive vegetation-density layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VegetationCountsV1 {
    /// Cells without planned vegetation.
    pub none: u16,
    /// Sparse vegetation cells.
    pub sparse: u16,
    /// Light vegetation cells.
    pub light: u16,
    /// Moderate vegetation cells.
    pub moderate: u16,
    /// Dense vegetation cells.
    pub dense: u16,
}

/// Exact counts for the mutually exclusive access-intent layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessCountsV1 {
    /// Ordinary playable cells.
    pub ordinary: u16,
    /// Scenic cells not required by an ordinary route.
    pub scenic: u16,
    /// Intentionally inaccessible cells.
    pub inaccessible: u16,
}

/// Exact membership counts for every canonical overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayCountsV1 {
    /// Reference or resolved coastline cells.
    pub coastline: u16,
    /// River overlay cells.
    pub river: u16,
    /// Waterfall overlay cells.
    pub waterfall: u16,
    /// Variable valley-lake cells.
    pub valley_lake: u16,
    /// Elevated mountain-lake overlay cells.
    pub mountain_lake: u16,
    /// Island inside the mountain lake.
    pub lake_island: u16,
    /// Exact frozen-woods overlay cells.
    pub frozen_woods: u16,
    /// Exact authored sharp-peak enclosure cells.
    pub peak_ring: u16,
    /// Crystal Ascent overlay cells.
    pub crystal_ascent: u16,
    /// Tunnel overlay cells.
    pub tunnel: u16,
    /// Generated scenic sea-island cells.
    pub sea_island: u16,
}

/// Resolved measurement of one template bounded-region rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundedRegionMetricsV1 {
    /// Stable rule identity.
    pub id: StableId,
    /// Resolved cells carrying the rule's target fact.
    pub cells: u16,
    /// Connected components in that exact membership.
    pub components: u16,
}

/// Strict public metrics for one schema-v1 plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchematicMetricsV1 {
    /// Metrics wire version. It currently equals the plan schema version.
    pub schema_version: u16,
    /// Exact source template identity.
    pub template_id: StableId,
    /// Exact source template revision.
    pub template_revision: u32,
    /// Requested world seed from plan provenance.
    pub world_seed: u64,
    /// Verified semantic plan fingerprint.
    pub semantic_fingerprint: u64,
    /// Exact number of candidate ordinals evaluated before selection or fallback.
    pub candidates_evaluated: u8,
    /// Number of candidate plans which passed the complete hard validator.
    pub hard_valid_candidates: u8,
    /// Selected normal candidate, or none for a reference artifact or fallback.
    pub selected_candidate: Option<u8>,
    /// Whether the separately validated reference fallback was used.
    pub used_reference_fallback: bool,
    /// Complete canonical coarse-cell count.
    pub cell_count: u16,
    /// Undirected adjacency count inside the radius-eight grid.
    pub internal_adjacencies: u16,
    /// Cells on the outer ring.
    pub boundary_cells: u16,
    /// Cell sides facing outside the complete grid.
    pub outward_sides: u16,
    /// Surface distribution.
    pub surfaces: SurfaceCountsV1,
    /// Landform distribution.
    pub landforms: LandformCountsV1,
    /// Climate distribution.
    pub climates: ClimateCountsV1,
    /// Vegetation distribution.
    pub vegetation: VegetationCountsV1,
    /// Access-intent distribution.
    pub access: AccessCountsV1,
    /// Overlay membership distribution.
    pub overlays: OverlayCountsV1,
    /// Stable-id-ordered resolved bounded-region measurements.
    pub bounded_regions: Vec<BoundedRegionMetricsV1>,
    /// Cells entering or leaving coastline membership versus the reference trace.
    pub moved_coast_cells: u16,
    /// Bidirectional Hausdorff-like hex distance between resolved and reference coasts.
    pub maximum_coast_displacement: u8,
    /// Cells in the bounded valley lake.
    pub valley_lake_cells: u16,
    /// Connected scenic sea-island group count.
    pub sea_island_groups: u16,
    /// Total cells in scenic sea-island groups.
    pub sea_island_cells: u16,
    /// Smallest scenic sea-island group, or zero when none exist.
    pub smallest_sea_island: u16,
    /// Largest scenic sea-island group, or zero when none exist.
    pub largest_sea_island: u16,
    /// Cells on which the template permits generated woodland.
    pub eligible_woodland_cells: u16,
    /// Eligible cells carrying generated woodland.
    pub woodland_cells: u16,
    /// Rounded integer percentage of eligible woodland cells selected.
    pub woodland_percent: u8,
    /// Stable feature claim count.
    pub feature_claims: u16,
    /// Stable network count.
    pub networks: u16,
    /// Total network node count.
    pub network_nodes: u16,
    /// Total directed network edge count.
    pub network_edges: u16,
    /// Total cells across ordered network edge paths.
    pub network_path_cells: u16,
}

pub(crate) fn compute_metrics_unchecked(
    template: &SchematicTemplateV1,
    plan: &SchematicPlanV1,
) -> SchematicMetricsV1 {
    let mut surfaces = SurfaceCountsV1 {
        land: 0,
        open_water: 0,
    };
    let mut landforms = LandformCountsV1 {
        none: 0,
        island: 0,
        beach: 0,
        shore: 0,
        valley: 0,
        plateau: 0,
        hill: 0,
        mountain: 0,
        massif: 0,
        sharp_peak: 0,
    };
    let mut climates = ClimateCountsV1 {
        marine: 0,
        temperate: 0,
        alpine: 0,
        frozen: 0,
    };
    let mut vegetation = VegetationCountsV1 {
        none: 0,
        sparse: 0,
        light: 0,
        moderate: 0,
        dense: 0,
    };
    let mut access = AccessCountsV1 {
        ordinary: 0,
        scenic: 0,
        inaccessible: 0,
    };
    let mut overlays = OverlayCountsV1 {
        coastline: 0,
        river: 0,
        waterfall: 0,
        valley_lake: 0,
        mountain_lake: 0,
        lake_island: 0,
        frozen_woods: 0,
        peak_ring: 0,
        crystal_ascent: 0,
        tunnel: 0,
        sea_island: 0,
    };

    for cell in &plan.cells {
        match cell.facts.surface {
            SurfaceKind::Land => surfaces.land = surfaces.land.saturating_add(1),
            SurfaceKind::OpenWater => {
                surfaces.open_water = surfaces.open_water.saturating_add(1);
            }
        }
        match cell.facts.landform {
            LandformKind::None => landforms.none = landforms.none.saturating_add(1),
            LandformKind::Island => landforms.island = landforms.island.saturating_add(1),
            LandformKind::Beach => landforms.beach = landforms.beach.saturating_add(1),
            LandformKind::Shore => landforms.shore = landforms.shore.saturating_add(1),
            LandformKind::Valley => landforms.valley = landforms.valley.saturating_add(1),
            LandformKind::Plateau => landforms.plateau = landforms.plateau.saturating_add(1),
            LandformKind::Hill => landforms.hill = landforms.hill.saturating_add(1),
            LandformKind::Mountain => landforms.mountain = landforms.mountain.saturating_add(1),
            LandformKind::Massif => landforms.massif = landforms.massif.saturating_add(1),
            LandformKind::SharpPeak => {
                landforms.sharp_peak = landforms.sharp_peak.saturating_add(1);
            }
        }
        match cell.facts.climate {
            ClimateKind::Marine => climates.marine = climates.marine.saturating_add(1),
            ClimateKind::Temperate => climates.temperate = climates.temperate.saturating_add(1),
            ClimateKind::Alpine => climates.alpine = climates.alpine.saturating_add(1),
            ClimateKind::Frozen => climates.frozen = climates.frozen.saturating_add(1),
        }
        match cell.facts.vegetation {
            VegetationDensity::None => vegetation.none = vegetation.none.saturating_add(1),
            VegetationDensity::Sparse => vegetation.sparse = vegetation.sparse.saturating_add(1),
            VegetationDensity::Light => vegetation.light = vegetation.light.saturating_add(1),
            VegetationDensity::Moderate => {
                vegetation.moderate = vegetation.moderate.saturating_add(1);
            }
            VegetationDensity::Dense => vegetation.dense = vegetation.dense.saturating_add(1),
        }
        match cell.facts.access {
            AccessIntent::Ordinary => access.ordinary = access.ordinary.saturating_add(1),
            AccessIntent::Scenic => access.scenic = access.scenic.saturating_add(1),
            AccessIntent::Inaccessible => {
                access.inaccessible = access.inaccessible.saturating_add(1);
            }
        }
        for overlay in &cell.facts.overlays {
            match overlay {
                FeatureKind::Coastline => {
                    overlays.coastline = overlays.coastline.saturating_add(1);
                }
                FeatureKind::River => overlays.river = overlays.river.saturating_add(1),
                FeatureKind::Waterfall => {
                    overlays.waterfall = overlays.waterfall.saturating_add(1);
                }
                FeatureKind::ValleyLake => {
                    overlays.valley_lake = overlays.valley_lake.saturating_add(1);
                }
                FeatureKind::MountainLake => {
                    overlays.mountain_lake = overlays.mountain_lake.saturating_add(1);
                }
                FeatureKind::LakeIsland => {
                    overlays.lake_island = overlays.lake_island.saturating_add(1);
                }
                FeatureKind::FrozenWoods => {
                    overlays.frozen_woods = overlays.frozen_woods.saturating_add(1);
                }
                FeatureKind::PeakRing => {
                    overlays.peak_ring = overlays.peak_ring.saturating_add(1);
                }
                FeatureKind::CrystalAscent => {
                    overlays.crystal_ascent = overlays.crystal_ascent.saturating_add(1);
                }
                FeatureKind::Tunnel => overlays.tunnel = overlays.tunnel.saturating_add(1),
                FeatureKind::SeaIsland => {
                    overlays.sea_island = overlays.sea_island.saturating_add(1);
                }
            }
        }
    }

    let mut bounded_regions = Vec::with_capacity(template.bounded_regions.len());
    let mut moved_coast_cells = 0;
    let mut maximum_coast_displacement = 0;
    let mut valley_lake_cells = 0;
    let mut sea_island_groups = 0;
    let mut sea_island_cells = 0;
    let mut smallest_sea_island = 0;
    let mut largest_sea_island = 0;
    let mut eligible_woodland_cells = 0;
    let mut woodland_cells = 0;

    for rule in &template.bounded_regions {
        let resolved = resolved_region(plan, rule);
        let groups = component_sets(&resolved);
        bounded_regions.push(BoundedRegionMetricsV1 {
            id: rule.id.clone(),
            cells: len_u16(resolved.len()),
            components: len_u16(groups.len()),
        });
        match rule.kind {
            BoundedRegionKind::Coastline => {
                let reference = rule.reference_mask.iter().copied().collect::<BTreeSet<_>>();
                let changed = reference
                    .symmetric_difference(&resolved)
                    .copied()
                    .collect::<Vec<_>>();
                moved_coast_cells = len_u16(changed.len());
                let resolved_to_reference = resolved.iter().filter_map(|cell| {
                    reference
                        .iter()
                        .filter_map(|reference| cell.checked_distance(*reference))
                        .min()
                });
                let reference_to_resolved = reference.iter().filter_map(|cell| {
                    resolved
                        .iter()
                        .filter_map(|resolved| cell.checked_distance(*resolved))
                        .min()
                });
                maximum_coast_displacement = resolved_to_reference
                    .chain(reference_to_resolved)
                    .filter_map(|distance| u8::try_from(distance).ok())
                    .max()
                    .unwrap_or(0);
            }
            BoundedRegionKind::ValleyLake => {
                valley_lake_cells = len_u16(resolved.len());
            }
            BoundedRegionKind::SeaIslands => {
                sea_island_groups = len_u16(groups.len());
                sea_island_cells = len_u16(resolved.len());
                smallest_sea_island = groups
                    .iter()
                    .map(BTreeSet::len)
                    .min()
                    .map(len_u16)
                    .unwrap_or(0);
                largest_sea_island = groups
                    .iter()
                    .map(BTreeSet::len)
                    .max()
                    .map(len_u16)
                    .unwrap_or(0);
            }
            BoundedRegionKind::Woodland => {
                eligible_woodland_cells = len_u16(rule.envelope.len());
                woodland_cells = len_u16(resolved.len());
            }
            BoundedRegionKind::Massif | BoundedRegionKind::TracedRegion => {}
        }
    }

    let network_nodes = plan.networks.iter().fold(0_u16, |total, network| {
        total.saturating_add(len_u16(network.nodes.len()))
    });
    let network_edges = plan.networks.iter().fold(0_u16, |total, network| {
        total.saturating_add(len_u16(network.edges.len()))
    });
    let network_path_cells = plan
        .networks
        .iter()
        .flat_map(|network| &network.edges)
        .fold(0_u16, |total, edge| {
            total.saturating_add(len_u16(edge.path.len()))
        });

    SchematicMetricsV1 {
        schema_version: plan.schema_version,
        template_id: plan.template_id.clone(),
        template_revision: plan.template_revision,
        world_seed: plan.provenance.world_seed,
        semantic_fingerprint: plan.semantic_fingerprint,
        candidates_evaluated: plan.provenance.candidates_evaluated,
        hard_valid_candidates: plan.provenance.hard_valid_candidates,
        selected_candidate: plan.provenance.selected_candidate,
        used_reference_fallback: plan.provenance.used_reference_fallback,
        cell_count: len_u16(plan.cells.len()),
        internal_adjacencies: 600,
        boundary_cells: 48,
        outward_sides: 102,
        surfaces,
        landforms,
        climates,
        vegetation,
        access,
        overlays,
        bounded_regions,
        moved_coast_cells,
        maximum_coast_displacement,
        valley_lake_cells,
        sea_island_groups,
        sea_island_cells,
        smallest_sea_island,
        largest_sea_island,
        eligible_woodland_cells,
        woodland_cells,
        woodland_percent: rounded_percent(woodland_cells, eligible_woodland_cells),
        feature_claims: len_u16(plan.features.len()),
        networks: len_u16(plan.networks.len()),
        network_nodes,
        network_edges,
        network_path_cells,
    }
}

pub(crate) fn cell_matches_rule(cell: &crate::model::CellPlan, rule: &BoundedRegionRule) -> bool {
    !rule.targets.is_empty()
        && rule
            .targets
            .iter()
            .all(|target| cell_matches_target(cell, *target))
}

pub(crate) fn cell_matches_target(cell: &crate::model::CellPlan, target: BoundedTarget) -> bool {
    match target {
        BoundedTarget::Surface(value) => cell.facts.surface == value,
        BoundedTarget::Landform(value) => cell.facts.landform == value,
        BoundedTarget::Climate(value) => cell.facts.climate == value,
        BoundedTarget::Vegetation(value) => cell.facts.vegetation == value,
        BoundedTarget::Vegetated => matches!(
            cell.facts.vegetation,
            VegetationDensity::Light | VegetationDensity::Moderate | VegetationDensity::Dense
        ),
        BoundedTarget::Access(value) => cell.facts.access == value,
        BoundedTarget::Overlay(value) => cell.facts.overlays.binary_search(&value).is_ok(),
    }
}

pub(crate) fn resolved_region(
    plan: &SchematicPlanV1,
    rule: &BoundedRegionRule,
) -> BTreeSet<SchematicCoord> {
    plan.cells
        .iter()
        .filter(|cell| {
            cell_matches_rule(cell, rule)
                && (rule.kind != BoundedRegionKind::Woodland || rule.envelope.contains(&cell.coord))
        })
        .map(|cell| cell.coord)
        .collect()
}

fn component_sets(cells: &BTreeSet<SchematicCoord>) -> Vec<BTreeSet<SchematicCoord>> {
    let mut remaining = cells.clone();
    let mut result = Vec::new();
    while let Some(start) = remaining.first().copied() {
        let mut component = BTreeSet::from([start]);
        let mut pending = VecDeque::from([start]);
        remaining.remove(&start);
        while let Some(cell) = pending.pop_front() {
            let Some(neighbors) = cell.neighbors() else {
                continue;
            };
            for neighbor in neighbors {
                if remaining.remove(&neighbor) {
                    component.insert(neighbor);
                    pending.push_back(neighbor);
                }
            }
        }
        result.push(component);
    }
    result
}

fn len_u16(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

pub(crate) fn rounded_percent(numerator: u16, denominator: u16) -> u8 {
    if denominator == 0 {
        return 0;
    }
    let rounded = u32::from(numerator)
        .saturating_mul(100)
        .saturating_add(u32::from(denominator) / 2)
        / u32::from(denominator);
    u8::try_from(rounded).unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentage_is_integer_rounded_and_total() {
        assert_eq!(rounded_percent(0, 0), 0);
        assert_eq!(rounded_percent(1, 3), 33);
        assert_eq!(rounded_percent(2, 3), 67);
        assert_eq!(rounded_percent(80, 100), 80);
    }
}
