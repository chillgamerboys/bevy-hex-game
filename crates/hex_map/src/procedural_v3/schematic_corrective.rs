//! Final-world acceptance checks for the Grand V3 corrective art pass.
//!
//! These checks deliberately run in the production compiler. The public hero
//! fixture, extreme-seed fixtures, and release corpus therefore exercise the
//! same objective geometry instead of validating only a hand-built unit shape.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_core::{HexCoord, Level, TilePos};
use hex_schematic::{
    FeatureKind as SchematicFeature, LandformKind, NetworkKind, SurfaceKind, VegetationDensity,
};

use super::*;

const CELL_PITCH: i32 = 22;
const CERTAIN_SNOW_LEVEL: Level = 146;
const PEAK_SUMMIT_MIN: Level = super::super::schematic_highlands::PEAK_SUMMIT_MIN;
const PEAK_SUMMIT_MAX: Level = super::super::schematic_highlands::PEAK_SUMMIT_MAX;
const MASSIF_SUMMIT_MIN: Level = super::super::schematic_highlands::MASSIF_SUMMIT_MIN;
const MASSIF_SUMMIT_MAX: Level = super::super::schematic_highlands::MASSIF_SUMMIT_MAX;
const FROZEN_PLATEAU_MIN: Level = 151;
const FROZEN_PLATEAU_MAX: Level = 153;
const FROZEN_PLATEAU_HALO_DEPTH: u32 = 6;
const CRYSTAL_MANTLE_MAXIMUM_CONNECTED_BOUNDARY_PROTRUSIONS: usize = 2;
const CRYSTAL_MANTLE_MAXIMUM_BOUNDARY_PROTRUSION_DIVISOR: usize = 100;
const CRYSTAL_MANTLE_MAXIMUM_ISOLATED_BOUNDARY_DROP: Level = 24;
const MASSIF_MAXIMUM_OUTER_PROTRUSION: Level = 12;
const PEAK_VISUAL_WALL_THRESHOLD: Level =
    super::super::schematic_highlands::PEAK_VISUAL_WALL_THRESHOLD;
const PEAK_OUTER_FEATHER_MAXIMUM_STEP: Level = 9;
const GARDEN_STRUCTURE_ID: StructureId = StructureId(WORLD_NAMESPACE | 0x0005_0000);

pub(super) struct CorrectiveWorldValidation<'a> {
    pub(super) hydrology: &'a HydrologyCompilation,
    pub(super) crystal_mask: &'a BTreeSet<HexCoord>,
    pub(super) crystal_mantle: &'a super::super::schematic_highlands::CrystalMantleAuthority,
    pub(super) crystal_terrain_top: Level,
    pub(super) crystal_upper_exit: TilePos,
    pub(super) fine_index: &'a FineWorldIndex,
    pub(super) reachable: &'a BTreeMap<TilePos, u32>,
    pub(super) massif_visual: &'a super::super::schematic_highlands::MassifVisualAuthority,
    pub(super) massif_scenic_cliff_edges: &'a BTreeSet<(TilePos, TilePos)>,
    pub(super) peak_ridges: &'a super::super::schematic_highlands::PeakRidgeAuthority,
    pub(super) tunnel_overburden: &'a TunnelOverburdenAuthority,
    pub(super) review: &'a CorrectiveReviewAuthority,
}

pub(super) fn validate_corrective_world_contract(
    plan: &SchematicPlanV1,
    world: &GeneratedWorldPlan,
    profile: V3GrandV3BasicTerrainProfile,
    validation: CorrectiveWorldValidation<'_>,
) -> Result<(), V3GenerationError> {
    let mut failures = Vec::new();
    macro_rules! audit {
        ($validation:expr) => {
            if let Err(error) = $validation {
                failures.push(error.to_string());
            }
        };
    }
    audit!(validate_crystal_mantle_with_scenic_cliffs(
        world,
        validation.crystal_mask,
        validation.crystal_mantle,
        validation.crystal_terrain_top,
        validation.crystal_upper_exit,
        validation.massif_scenic_cliff_edges,
    ));
    audit!(validate_highland_hierarchy(
        plan,
        world,
        validation.crystal_mask,
        validation.massif_visual,
        validation.crystal_terrain_top,
    ));
    audit!(validate_peak_ridge_authority(world, validation.peak_ridges));
    audit!(validate_peak_massif_seams_with_scenic_cliffs(
        world,
        validation.peak_ridges,
        validation.massif_visual,
        validation.massif_scenic_cliff_edges,
    ));
    audit!(validate_frozen_exit(plan, world, validation.crystal_mask));
    audit!(validate_frozen_plateau(
        plan,
        world,
        validation.crystal_mask
    ));
    audit!(validate_concealed_tunnel(world, profile));
    audit!(validate_tunnel_overburden_authority(
        plan,
        world,
        validation.fine_index,
        validation.tunnel_overburden,
    ));
    audit!(validate_garden_island(plan, world));
    audit!(validate_vegetation_gradient(plan, world));
    audit!(validate_certain_snow_caps(
        plan,
        world,
        validation.crystal_mask
    ));
    audit!(validate_waterfall_and_review_anchor(
        world,
        profile,
        validation.hydrology
    ));
    audit!(validate_river_and_review_anchor(
        plan,
        world,
        validation.hydrology
    ));
    audit!(validate_semantic_review_anchors(plan, world, &validation));
    if failures.is_empty() {
        Ok(())
    } else {
        Err(schematic_contract(format!(
            "corrective world validation found {} independent failure(s): {}",
            failures.len(),
            failures.join(" | ")
        )))
    }
}

pub(super) fn validate_concealed_tunnel(
    world: &GeneratedWorldPlan,
    profile: V3GrandV3BasicTerrainProfile,
) -> Result<(), V3GenerationError> {
    const CLEARANCE_TOP: Level = 13;
    const ROOF_TOP: Level = 16;
    const MINIMUM_RECESSED_ROWS: usize = 4;

    let route = world
        .features
        .protected_routes
        .get("grand_v3.tunnel")
        .ok_or_else(|| schematic_contract("corrective world lost its tunnel route"))?;
    let mouth = *world
        .anchors
        .get("grand_v3.tunnel_mouth")
        .ok_or_else(|| schematic_contract("corrective tunnel lost its recessed mouth anchor"))?;
    let mouth_index = route
        .centerline
        .iter()
        .position(|surface| *surface == mouth)
        .ok_or_else(|| {
            schematic_contract("tunnel mouth is not the first exposed centerline surface")
        })?;
    if mouth_index < MINIMUM_RECESSED_ROWS
        || world
            .volume
            .surfaces
            .get(&mouth)
            .is_none_or(|metadata| metadata.interior.is_some())
    {
        return Err(schematic_contract(
            "tunnel mouth is not exterior footing after a recessed approach",
        ));
    }

    let (interior_id, interior) = world
        .interiors
        .by_id
        .iter()
        .next()
        .ok_or_else(|| schematic_contract("corrective tunnel has no unified interior"))?;
    if world.interiors.by_id.len() != 1 {
        return Err(schematic_contract(
            "corrective tunnel and Crystal Ascent are not one unified interior",
        ));
    }
    let foot_entries = interior
        .entrances
        .iter()
        .copied()
        .filter(|surface| surface.level == profile.crystal_base_level)
        .collect::<BTreeSet<_>>();
    let threshold = *route
        .centerline
        .get(mouth_index.saturating_sub(1))
        .ok_or_else(|| schematic_contract("corrective tunnel has no recessed threshold"))?;

    let centerline = route
        .centerline
        .iter()
        .map(|surface| surface.coord)
        .collect::<Vec<_>>();
    const EVEN_WIDTH_BIASES: [[i32; 4]; 2] = [[-1, 0, 1, 2], [-2, -1, 0, 1]];
    let first_recessed = mouth_index.saturating_sub(MINIMUM_RECESSED_ROWS);
    let resolved_rows = EVEN_WIDTH_BIASES.into_iter().find_map(|offsets| {
        let rows = (first_recessed..mouth_index)
            .map(|index| tunnel_lane_row(&centerline, index, offsets))
            .collect::<Vec<_>>();
        let entrance = rows
            .last()?
            .iter()
            .copied()
            .map(|coord| TilePos::new(coord, profile.crystal_base_level))
            .collect::<BTreeSet<_>>();
        (entrance == foot_entries).then_some(rows)
    });
    let Some(recessed_rows) = resolved_rows else {
        return Err(schematic_contract(format!(
            "tunnel recessed threshold is not one exact four-wide lane row: entries={}",
            foot_entries.len()
        )));
    };
    if !foot_entries.contains(&threshold) {
        return Err(schematic_contract(
            "tunnel recessed threshold omits its ordered centerline floor",
        ));
    }

    for row in &recessed_rows {
        if row.len() != TUNNEL_LANE_WIDTH {
            return Err(schematic_contract(
                "tunnel recessed approach has a non-four-wide lane row",
            ));
        }
        for coord in row {
            let floor = TilePos::new(*coord, profile.crystal_base_level);
            let column = world.volume.columns.get(coord).ok_or_else(|| {
                schematic_contract(format!(
                    "tunnel recessed lane {coord:?} has no semantic column"
                ))
            })?;
            if !route.surfaces.contains(&floor)
                || !interior.floors.contains(&floor)
                || world
                    .volume
                    .surfaces
                    .get(&floor)
                    .is_none_or(|metadata| metadata.interior != Some(*interior_id))
                || (CLEARANCE_TOP..ROOF_TOP).any(|level| {
                    let roof = TilePos::new(*coord, level);
                    !interior.roof_voxels.contains(&roof)
                        || solid_mass_at_level(column, level)
                            .is_none_or(|mass| mass.cutaway_for != Some(*interior_id))
                })
                || top_surface(world, *coord).is_none_or(|surface| surface.level < ROOF_TOP)
            {
                return Err(schematic_contract(format!(
                    "tunnel recessed lane {coord:?} lacks exact floor, roof, or unified interior ownership"
                )));
            }
        }
    }

    let mouth_row = EVEN_WIDTH_BIASES
        .into_iter()
        .map(|offsets| tunnel_lane_row(&centerline, mouth_index, offsets))
        .find(|row| {
            row.iter().all(|coord| {
                let surface = TilePos::new(*coord, mouth.level);
                route.surfaces.contains(&surface)
                    && world
                        .volume
                        .surfaces
                        .get(&surface)
                        .is_some_and(|metadata| metadata.interior.is_none())
            })
        })
        .ok_or_else(|| {
            schematic_contract("tunnel mouth is not one exact exterior four-wide row")
        })?;
    if mouth_row.len() != TUNNEL_LANE_WIDTH {
        return Err(schematic_contract(
            "tunnel mouth exterior row is not exactly four lanes",
        ));
    }
    Ok(())
}

/// Verifies that carving the roofed tunnel never changed the terrain visible
/// above it. The final ecology pass may deterministically recolour only the
/// exposed cap; every lower overburden voxel and all cutaway ownership remain
/// byte-for-byte equivalent to the pre-carve source.
pub(super) fn validate_tunnel_overburden_authority(
    plan: &SchematicPlanV1,
    world: &GeneratedWorldPlan,
    fine_index: &FineWorldIndex,
    authority: &TunnelOverburdenAuthority,
) -> Result<(), V3GenerationError> {
    if authority.columns.is_empty() {
        return Err(schematic_contract(
            "final tunnel has no captured natural overburden authority",
        ));
    }
    let cells = plan
        .cells
        .iter()
        .map(|cell| (PatchId(u32::from(cell.id.get())), cell))
        .collect::<BTreeMap<_, _>>();
    for (coord, expected_column) in &authority.columns {
        let final_surface = top_surface(world, *coord).ok_or_else(|| {
            schematic_contract(format!(
                "final tunnel overburden column {coord:?} has no exposed surface"
            ))
        })?;
        if final_surface != expected_column.surface {
            return Err(schematic_contract(format!(
                "final tunnel overburden moved from {:?} to {final_surface:?}",
                expected_column.surface
            )));
        }
        let patch = fine_index.patch(*coord).ok_or_else(|| {
            schematic_contract(format!(
                "final tunnel overburden {coord:?} has no semantic owner"
            ))
        })?;
        let cell = cells.get(&patch).copied().ok_or_else(|| {
            schematic_contract(format!(
                "final tunnel overburden owner {} has no schematic cell",
                patch.0
            ))
        })?;
        let cap_override = super::super::schematic_ecology::summit_band_requires_snow_cap(
            cell,
            expected_column.surface,
        )
        .then_some(SolidMaterialRole::Snow)
        .or_else(|| {
            super::super::schematic_ecology::cap_material_override(
                cell,
                expected_column.surface,
                plan.provenance.world_seed,
            )
        });
        let final_column = world.volume.columns.get(coord).ok_or_else(|| {
            schematic_contract(format!(
                "final tunnel overburden {coord:?} lost its semantic column"
            ))
        })?;
        for (level, expected) in &expected_column.voxels {
            let actual = solid_mass_at_level(final_column, *level).ok_or_else(|| {
                schematic_contract(format!(
                    "final tunnel overburden has a gap at {:?}",
                    TilePos::new(*coord, *level)
                ))
            })?;
            let expected_material = if *level == expected_column.surface.level {
                cap_override.unwrap_or(expected.material)
            } else {
                expected.material
            };
            if actual.material != expected_material || actual.cutaway_for != expected.cutaway_for {
                return Err(schematic_contract(format!(
                    "final tunnel overburden changed at {:?}: {:?}/{:?}, expected {:?}/{:?}",
                    TilePos::new(*coord, *level),
                    actual.material,
                    actual.cutaway_for,
                    expected_material,
                    expected.cutaway_for
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn validate_crystal_mantle(
    world: &GeneratedWorldPlan,
    crystal_mask: &BTreeSet<HexCoord>,
    authority: &super::super::schematic_highlands::CrystalMantleAuthority,
    expected_crystal_top: Level,
    expected_upper_exit: TilePos,
) -> Result<(), V3GenerationError> {
    validate_crystal_mantle_with_scenic_cliffs(
        world,
        crystal_mask,
        authority,
        expected_crystal_top,
        expected_upper_exit,
        &BTreeSet::new(),
    )
}

fn validate_crystal_mantle_with_scenic_cliffs(
    world: &GeneratedWorldPlan,
    crystal_mask: &BTreeSet<HexCoord>,
    authority: &super::super::schematic_highlands::CrystalMantleAuthority,
    expected_crystal_top: Level,
    expected_upper_exit: TilePos,
    massif_scenic_cliff_edges: &BTreeSet<(TilePos, TilePos)>,
) -> Result<(), V3GenerationError> {
    let actual_crystal_top = super::crystal_terrain_top(&world.volume, crystal_mask)?;
    if actual_crystal_top != expected_crystal_top {
        return Err(schematic_contract(format!(
            "Crystal terrain-top authority drifted from {expected_crystal_top} to {actual_crystal_top}"
        )));
    }
    let expected_shell_band =
        super::super::crystal_ascent::macro_composite_shell_band_coords(crystal_mask)
            .map_err(schematic_contract)?;
    let crystal_rotation = world
        .layout
        .patches
        .values()
        .find(|patch| patch.mask == *crystal_mask)
        .map(|patch| patch.rotation_turns)
        .ok_or_else(|| {
            schematic_contract("composite Crystal overburden has no exact claimed patch")
        })?;
    let expected_overburden =
        super::super::crystal_ascent::macro_composite_natural_shell_overburden(
            crystal_mask,
            crystal_rotation,
        )
        .map_err(schematic_contract)?;
    let expected_overburden_coords = expected_overburden.keys().copied().collect::<BTreeSet<_>>();
    let mut expected_shell_floors = BTreeMap::new();
    let mut expected_shell_ceilings = BTreeMap::new();
    for shell_coord in &expected_overburden_coords {
        let natural_shell_top = top_surface(world, *shell_coord).ok_or_else(|| {
            schematic_contract(format!(
                "composite Crystal buried shell lost its natural cap at {shell_coord:?}"
            ))
        })?;
        let highest_worked = world
            .volume
            .columns
            .get(shell_coord)
            .into_iter()
            .flat_map(|column| &column.elements)
            .filter_map(|element| {
                let VolumeElement::Solid(mass) = *element else {
                    return None;
                };
                (mass.material == SolidMaterialRole::WorkedStone)
                    .then_some(mass.levels.top.saturating_sub(1))
            })
            .max()
            .ok_or_else(|| {
                schematic_contract(format!(
                    "composite Crystal buried shell has no worked-stone authority at {shell_coord:?}"
                ))
            })?;
        for neighbor in shell_coord.neighbors() {
            if crystal_mask.contains(&neighbor) || authority.opening_clearance.contains(&neighbor) {
                continue;
            }
            expected_shell_floors
                .entry(neighbor)
                .and_modify(|floor: &mut Level| *floor = (*floor).max(highest_worked))
                .or_insert(highest_worked);
            let ceiling = natural_shell_top.level.saturating_add(
                super::super::schematic_highlands::CRYSTAL_SHELL_MAXIMUM_APRON_RISE,
            );
            expected_shell_ceilings
                .entry(neighbor)
                .and_modify(|current: &mut Level| *current = (*current).min(ceiling))
                .or_insert(ceiling);
        }
    }
    let shell_apron = authority.shell_concealment_apron();
    let resolved_shell_band = authority
        .natural_shell_skin
        .union(&authority.exposed_shell_openings)
        .copied()
        .collect::<BTreeSet<_>>();
    let center = super::exact_hex_disk_center(
        crystal_mask,
        super::super::schematic_highlands::CRYSTAL_SITE_RADIUS,
    )
    .ok_or_else(|| schematic_contract("Crystal mantle has no exact site centre"))?;
    let attainable_enclosure_band = authority.attainable_enclosure_band();
    let forced_low = authority
        .forced_low_frozen_halo
        .keys()
        .chain(authority.forced_low_exit_blend.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    let forced_low_divisor =
        super::super::schematic_highlands::CRYSTAL_ENCLOSURE_FORCED_LOW_MAXIMUM_FRACTION_DIVISOR;
    let forced_low_budget = authority
        .enclosure_band
        .len()
        .saturating_add(forced_low_divisor - 1)
        / forced_low_divisor;
    if authority.enclosure_band.len() < 1_000
        || authority.crystal_center != center
        || authority.composite_crystal_top != actual_crystal_top
        || !authority.uplift_core.is_subset(&attainable_enclosure_band)
        || !authority
            .enclosure_band
            .is_subset(&authority.support_footprint)
        || forced_low.is_empty()
        || forced_low.len() > forced_low_budget
        || authority
            .forced_low_frozen_halo
            .iter()
            .chain(&authority.forced_low_exit_blend)
            .any(|(coord, ceiling)| {
                !authority.enclosure_band.contains(coord)
                    || *ceiling > authority.composite_crystal_top
            })
        || attainable_enclosure_band
            .len()
            .saturating_add(forced_low.len())
            != authority.enclosure_band.len()
        || !authority.route_exclusion.is_subset(&authority.uplift_core)
        || authority.route_exclusion.len() < 100
        || !authority.uplift_core.is_disjoint(crystal_mask)
        || !authority.support_footprint.is_disjoint(crystal_mask)
        || !authority.enclosure_band.is_disjoint(crystal_mask)
        || !authority.route_exclusion.is_disjoint(crystal_mask)
        || !authority
            .opening_clearance
            .is_disjoint(&authority.uplift_core)
        || !authority
            .opening_clearance
            .is_disjoint(&authority.support_footprint)
        || !authority
            .opening_clearance
            .is_disjoint(&authority.enclosure_band)
        || authority.natural_shell_skin.is_empty()
        || !authority.natural_shell_skin.is_subset(crystal_mask)
        || !authority.exposed_shell_openings.is_subset(crystal_mask)
        || !authority
            .natural_shell_skin
            .is_disjoint(&authority.exposed_shell_openings)
        || authority.natural_shell_skin != expected_overburden_coords
        || shell_apron.is_empty()
        || authority.shell_concealment_floors != expected_shell_floors
        || authority.shell_concealment_ceilings != expected_shell_ceilings
        || resolved_shell_band != expected_shell_band
    {
        return Err(schematic_contract(
            "Crystal enclosure lost broad neighboring-biome uplift, entered the exact site, or sealed an authored opening",
        ));
    }
    let protected_lobes = fine_components(&authority.route_exclusion);
    if protected_lobes.len() != 6
        || protected_lobes.iter().any(|lobe| {
            authority
                .sector_pins
                .values()
                .filter(|(pin, _)| lobe.contains(pin))
                .count()
                != 1
        })
    {
        return Err(schematic_contract(format!(
            "Crystal route authority must be six separated shoulder lobes rather than a ring wall; components={}",
            protected_lobes.len()
        )));
    }
    let expected_uplift_caps = authority.expected_uplift_caps.as_ref().ok_or_else(|| {
        schematic_contract("final Crystal enclosure reached validation before sealing")
    })?;
    if expected_uplift_caps.len() != authority.route_exclusion.len() {
        return Err(schematic_contract(
            "Crystal enclosure has incomplete sealed uplift-cap authority",
        ));
    }
    if let Some((coord, expected)) = expected_uplift_caps.iter().find(|(coord, expected)| {
        top_surface(world, **coord).is_none_or(|surface| surface.level != **expected)
    }) {
        return Err(schematic_contract(format!(
            "Crystal enclosure cap {coord:?} drifted from sealed level {expected}"
        )));
    }
    if authority.sector_pins.len() != 6 {
        return Err(schematic_contract(format!(
            "Crystal enclosure lost its six broad neighboring-biome sector authorities: pins={}",
            authority.sector_pins.len()
        )));
    }
    if let Some((sector, (coord, level))) =
        authority
            .sector_pins
            .iter()
            .find(|(sector, (coord, level))| {
                usize::from(**sector) >= 6
                    || crystal_enclosure_sector(center, *coord) != usize::from(**sector)
                    || !authority.uplift_core.contains(coord)
                    || expected_uplift_caps.get(coord) != Some(level)
                    || *level < super::super::schematic_highlands::CRYSTAL_ENCLOSURE_HIGH_MIN
            })
    {
        return Err(schematic_contract(format!(
            "Crystal enclosure lost broad neighboring-biome sector {sector} authority at {coord:?}: authored={level}, sealed={:?}, resolved-sector={}, in-core={}",
            expected_uplift_caps.get(coord),
            crystal_enclosure_sector(center, *coord),
            authority.uplift_core.contains(coord),
        )));
    }
    authority
        .validate_attainable_coverage("final admission", |coord| {
            top_surface(world, coord).map(|surface| surface.level)
        })
        .map_err(schematic_contract)?;
    for (coord, thickness) in &expected_overburden {
        validate_crystal_shell_overburden_column(world, *coord, *thickness)?;
    }
    validate_crystal_shell_openings(
        world,
        crystal_mask,
        crystal_rotation,
        &authority.exposed_shell_openings,
        expected_upper_exit,
    )?;
    let unexpected_worked_shell_cap = authority
        .natural_shell_skin
        .union(&authority.exposed_shell_openings)
        .find_map(|coord| {
            let surface = top_surface(world, *coord)?;
            (solid_mass_at_surface(&world.volume, surface)
                .is_some_and(|mass| mass.material == SolidMaterialRole::WorkedStone)
                && !authority.exposed_shell_openings.contains(coord))
            .then_some(surface)
        });
    if let Some(surface) = unexpected_worked_shell_cap {
        return Err(schematic_contract(format!(
            "composite Crystal retains an exposed worked-stone shell cap away from its exact openings at {surface:?}"
        )));
    }
    if let Some((coord, floor, actual)) =
        authority
            .shell_concealment_floors
            .iter()
            .find_map(|(coord, floor)| {
                let actual = top_surface(world, *coord).map(|surface| surface.level);
                (actual.is_none_or(|level| level < *floor)).then_some((*coord, *floor, actual))
            })
    {
        return Err(schematic_contract(format!(
            "composite Crystal shell-concealment apron fell below {floor} at {coord:?}: {actual:?}"
        )));
    }
    if let Some((coord, ceiling, actual)) =
        authority
            .shell_concealment_ceilings
            .iter()
            .find_map(|(coord, ceiling)| {
                let actual = top_surface(world, *coord).map(|surface| surface.level);
                (actual.is_none_or(|level| level > *ceiling)).then_some((*coord, *ceiling, actual))
            })
    {
        return Err(schematic_contract(format!(
            "composite Crystal shell-concealment apron rose above {ceiling} at {coord:?}: {actual:?}"
        )));
    }
    let exposed_worked_edge = crystal_mask
        .iter()
        .copied()
        .filter(|coord| {
            coord
                .neighbors()
                .into_iter()
                .any(|neighbor| !crystal_mask.contains(&neighbor))
        })
        .filter(|coord| {
            let Some(highest_worked) = world
                .volume
                .columns
                .get(coord)
                .into_iter()
                .flat_map(|column| &column.elements)
                .filter_map(|element| {
                    let VolumeElement::Solid(mass) = *element else {
                        return None;
                    };
                    (mass.material == SolidMaterialRole::WorkedStone)
                        .then_some(mass.levels.top.saturating_sub(1))
                })
                .max()
            else {
                return false;
            };
            coord.neighbors().into_iter().any(|neighbor| {
                !crystal_mask.contains(&neighbor)
                    && !authority.opening_clearance.contains(&neighbor)
                    && top_surface(world, neighbor)
                        .is_none_or(|surface| surface.level < highest_worked)
            })
        })
        .collect::<BTreeSet<_>>();
    if !exposed_worked_edge.is_empty() {
        return Err(schematic_contract(format!(
            "composite Crystal retains {} exposed worked-stone exterior columns away from its openings; first={:?}",
            exposed_worked_edge.len(),
            exposed_worked_edge.first()
        )));
    }
    // The exact Crystal disk is an enclosed authored hole, not the outside
    // terrain datum for the surrounding scalar mantle. Its natural shell and
    // two architectural openings were validated independently above. Compare
    // only the mantle's true exterior edge here, matching the edge-depth
    // authority used when the highland field was projected.
    let support_boundary = true_outer_boundary_edges(&authority.support_footprint, crystal_mask);
    let mut boundary_protrusions = BTreeSet::new();
    let mut first_boundary_protrusion = None;
    let mut maximum_boundary_drop = 0;
    for (inside, outside_neighbors) in &support_boundary {
        let Some(inside_surface) = top_surface(world, *inside) else {
            return Err(schematic_contract(format!(
                "Crystal mantle support boundary lost its surface at {inside:?}"
            )));
        };
        for outside in outside_neighbors {
            let Some(outside_surface) = top_surface(world, *outside) else {
                continue;
            };
            if admits_exact_scenic_cliff(massif_scenic_cliff_edges, inside_surface, outside_surface)
            {
                continue;
            }
            if exceeds_upward_boundary_protrusion(
                inside_surface.level,
                outside_surface.level,
                super::super::schematic_highlands::CRYSTAL_SHELL_MAXIMUM_APRON_RISE,
            ) {
                boundary_protrusions.insert(*inside);
                first_boundary_protrusion.get_or_insert((inside_surface, outside_surface));
                maximum_boundary_drop = maximum_boundary_drop
                    .max(inside_surface.level.saturating_sub(outside_surface.level));
            }
        }
    }
    if retains_connected_boundary_wall(
        &boundary_protrusions,
        support_boundary.len(),
        maximum_boundary_drop,
    ) {
        if let Some((inside, outside)) = first_boundary_protrusion {
            let components = fine_components(&boundary_protrusions);
            #[cfg(test)]
            let containing_routes = world
                .features
                .protected_routes
                .iter()
                .filter_map(|(name, route)| {
                    route
                        .surfaces
                        .iter()
                        .any(|surface| surface.coord == inside.coord)
                        .then_some(name.as_str())
                })
                .collect::<Vec<_>>();
            #[cfg(not(test))]
            let containing_routes = Vec::<&str>::new();
            return Err(schematic_contract(format!(
                "Crystal mantle ends in exposed retaining cliffs across {} boundary columns in components {:?}; first between {inside:?} and {outside:?}; inside-routes={containing_routes:?}",
                boundary_protrusions.len(),
                components.iter().map(BTreeSet::len).collect::<Vec<_>>(),
            )));
        }
    }
    let low = expected_uplift_caps
        .iter()
        .filter(|(_, level)| **level <= actual_crystal_top)
        .count();
    if low != 0 {
        return Err(schematic_contract(format!(
            "Crystal enclosure leaves {low} authoritative uplift columns at or below the final Crystal top"
        )));
    }
    Ok(())
}

fn crystal_enclosure_sector(center: HexCoord, coord: HexCoord) -> usize {
    center
        .neighbors()
        .into_iter()
        .enumerate()
        .min_by_key(|(direction, probe)| (probe.distance(coord), *direction))
        .map(|(direction, _)| direction)
        .unwrap_or_default()
}

fn peak_directional_extents(summit: HexCoord, coords: &BTreeSet<HexCoord>) -> [u32; 6] {
    let mut extents = [0_u32; 6];
    for coord in coords {
        let sector = crystal_enclosure_sector(summit, *coord);
        if let Some(extent) = extents.get_mut(sector) {
            *extent = (*extent).max(summit.distance(*coord));
        }
    }
    extents
}

fn validate_highland_hierarchy(
    plan: &SchematicPlanV1,
    world: &GeneratedWorldPlan,
    crystal_mask: &BTreeSet<HexCoord>,
    massif_visual: &super::super::schematic_highlands::MassifVisualAuthority,
    crystal_top: Level,
) -> Result<(), V3GenerationError> {
    let massif_patches = patches_for_landform(plan, LandformKind::Massif);
    let peak_patches = patches_for_landform(plan, LandformKind::SharpPeak);
    let semantic_massif_mask = union_patch_masks(world, &massif_patches)?;
    let peak_mask = union_patch_masks(world, &peak_patches)?;
    let crystal_site_radius = super::super::schematic_highlands::CRYSTAL_SITE_RADIUS;
    let crystal_center = super::exact_hex_disk_center(crystal_mask, crystal_site_radius)
        .ok_or_else(|| {
            schematic_contract(
                "corrective highland hierarchy requires the exact radius-32 Crystal site",
            )
        })?;
    validate_massif_visual_authority(
        plan,
        &world.layout,
        crystal_mask,
        &semantic_massif_mask,
        massif_visual,
    )?;
    let massif_mask = &massif_visual.visual_mask;

    let crest = *world
        .observation_anchors
        .get("grand_v3.massif_crest")
        .ok_or_else(|| schematic_contract("corrective massif has no exact crest anchor"))?;
    if !semantic_massif_mask.contains(&crest.coord)
        || !(MASSIF_SUMMIT_MIN..=MASSIF_SUMMIT_MAX).contains(&crest.level)
    {
        return Err(schematic_contract(format!(
            "massif crest {crest:?} is outside its mask or {MASSIF_SUMMIT_MIN}..={MASSIF_SUMMIT_MAX} summit band"
        )));
    }
    // Quantify the authored Massif itself. The visual mask also contains a
    // narrow Mountain connector and two low Mountain feather rings whose job
    // is precisely to fall below the Massif; counting those as Massif would
    // make a healthier taper weaken the semantic height contract.
    let massif_levels = massif_visual
        .semantic_owner_mask
        .iter()
        .filter_map(|coord| top_surface(world, *coord))
        .collect::<Vec<_>>();
    validate_massif_crown_shape(world, crest, massif_mask)?;
    validate_massif_outer_taper(world, massif_visual, crystal_mask, crest)?;
    let broadly_high = massif_levels
        .iter()
        .filter(|surface| surface.level >= crystal_top.saturating_add(8))
        .count();
    if !has_strong_highland_majority(broadly_high, massif_levels.len()) {
        return Err(schematic_contract(format!(
            "a strong majority of the Massif must stand meaningfully above Crystal: high={broadly_high}, total={}, threshold={}",
            massif_levels.len(),
            crystal_top.saturating_add(8)
        )));
    }
    let crystal_cell = overlay_cell(plan, SchematicFeature::CrystalAscent)?;
    // Crest authoring now prioritizes enough viable sectors for the irregular
    // summit lobes, then distance from the narrow visual connector, before
    // boundary depth. Requiring the crest to be the single deepest eligible
    // coordinate was the older radial-field contract and rejects a healthy,
    // deliberately offset massif. Preserve the spatial guarantee that rule
    // was meant to provide: the complete protected crown must still belong to
    // authored Massif terrain rather than borrowing its room from the visual
    // connector or the outer Mountain feather.
    if crest
        .coord
        .within_radius(super::super::schematic_highlands::MASSIF_SUMMIT_BODY_RADIUS)
        .into_iter()
        .any(|coord| !semantic_massif_mask.contains(&coord))
    {
        return Err(schematic_contract(
            "massif crest does not retain its complete protected crown inside authored Massif terrain",
        ));
    }

    let crest_patch = world
        .layout
        .patches
        .iter()
        .find_map(|(patch_id, patch)| patch.mask.contains(&crest.coord).then_some(*patch_id))
        .ok_or_else(|| schematic_contract("massif crest has no coarse owner"))?;
    let crest_cell = plan
        .cells
        .iter()
        .find(|cell| u32::from(cell.id.get()) == crest_patch.0)
        .ok_or_else(|| schematic_contract("massif crest owner has no schematic cell"))?;
    if crest_cell
        .coord
        .checked_distance(crystal_cell.coord)
        .is_none_or(|distance| distance < 2)
        || crest
            .coord
            .distance(crystal_center)
            .saturating_sub(crystal_site_radius)
            < CELL_PITCH.unsigned_abs() / 2
    {
        return Err(schematic_contract(
            "massif crest is adjacent to the Crystal site instead of centered in the massif",
        ));
    }

    let highest_peak = peak_mask
        .iter()
        .filter_map(|coord| top_surface(world, *coord).map(|surface| surface.level))
        .max()
        .ok_or_else(|| schematic_contract("corrective peak ring has no final surfaces"))?;
    if !(PEAK_SUMMIT_MIN..=PEAK_SUMMIT_MAX).contains(&highest_peak)
        || crest.level.saturating_sub(highest_peak) < 30
        || crest.level.saturating_sub(crystal_top) < 40
    {
        return Err(schematic_contract(format!(
            "highland hierarchy is too weak: Crystal={crystal_top}, peaks={highest_peak}, massif={}",
            crest.level
        )));
    }
    Ok(())
}

fn validate_massif_outer_taper(
    world: &GeneratedWorldPlan,
    authority: &super::super::schematic_highlands::MassifVisualAuthority,
    crystal_mask: &BTreeSet<HexCoord>,
    crest: TilePos,
) -> Result<(), V3GenerationError> {
    let connector = authority
        .connector_owners
        .keys()
        .copied()
        .collect::<BTreeSet<_>>();
    let connector_core = authority
        .semantic_owner_mask
        .union(&connector)
        .copied()
        .collect::<BTreeSet<_>>();
    if connector.iter().any(|coord| {
        coord
            .neighbors()
            .into_iter()
            .all(|neighbor| connector_core.contains(&neighbor))
    }) {
        return Err(schematic_contract(
            "Massif visual connector widened into an interior highland strip",
        ));
    }
    let boundary_edges = true_outer_boundary_edges(&authority.visual_mask, crystal_mask);
    let boundary = boundary_edges.keys().copied().collect::<BTreeSet<_>>();
    let mut protrusions = BTreeSet::new();
    let mut protrusion_edges = Vec::new();
    for (inside, outside_coords) in &boundary_edges {
        let Some(inside_surface) = top_surface(world, *inside) else {
            return Err(schematic_contract(format!(
                "Massif outer taper lost its surface at {inside:?}"
            )));
        };
        for outside in outside_coords.iter().copied() {
            let Some(outside_surface) = top_surface(world, outside) else {
                continue;
            };
            if exceeds_upward_boundary_protrusion(
                inside_surface.level,
                outside_surface.level,
                MASSIF_MAXIMUM_OUTER_PROTRUSION,
            ) {
                protrusions.insert(*inside);
                protrusion_edges.push((inside_surface, outside_surface));
            }
        }
    }
    if let Some((inside, outside)) = protrusion_edges.first() {
        let inside_routes = world
            .features
            .protected_routes
            .iter()
            .filter_map(|(name, route)| {
                route
                    .surfaces
                    .iter()
                    .any(|surface| surface.coord == inside.coord)
                    .then_some((
                        name.as_str(),
                        route
                            .centerline
                            .iter()
                            .any(|surface| surface.coord == inside.coord),
                    ))
            })
            .collect::<Vec<_>>();
        let outside_routes = world
            .features
            .protected_routes
            .iter()
            .filter_map(|(name, route)| {
                route
                    .surfaces
                    .iter()
                    .any(|surface| surface.coord == outside.coord)
                    .then_some(name.as_str())
            })
            .collect::<Vec<_>>();
        return Err(schematic_contract(format!(
            "Massif field ends in protruding mask-edge walls across {} of {} boundary columns in components {:?}; first between {inside:?} and {outside:?}, inside routes={inside_routes:?}, outside routes={outside_routes:?}",
            protrusions.len(),
            boundary.len(),
            fine_components(&protrusions)
                .iter()
                .map(BTreeSet::len)
                .collect::<Vec<_>>(),
        )));
    }
    for direction in 0..6 {
        let levels = (0..MAX_V3_LEVEL.unsigned_abs())
            .map(|distance| {
                super::super::schematic_highlands::step_in_direction(
                    crest.coord,
                    direction,
                    distance,
                )
            })
            .take_while(|coord| authority.visual_mask.contains(coord))
            .map(|coord| {
                top_surface(world, coord)
                    .map(|surface| surface.level)
                    .ok_or_else(|| {
                        schematic_contract(format!(
                            "Massif radial {direction} lost its surface at {coord:?}"
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        validate_complete_massif_radial(direction, &levels)?;
    }
    Ok(())
}

fn validate_complete_massif_radial(
    direction: usize,
    levels: &[Level],
) -> Result<(), V3GenerationError> {
    let (Some(inner), Some(outer)) = (levels.first(), levels.last()) else {
        return Err(schematic_contract(format!(
            "Massif radial {direction} is empty"
        )));
    };
    let outward_reversals = levels
        .windows(2)
        .filter(|pair| {
            pair.first()
                .zip(pair.get(1))
                .is_some_and(|(current, next)| next > current)
        })
        .count();
    let high_shoulders = levels.iter().filter(|level| **level > 150).count();
    if levels.len() < 30 || inner.saturating_sub(*outer) < 80 || high_shoulders < 20 {
        return Err(schematic_contract(format!(
            "Massif radial {direction} lost its broad overall-rising profile: samples={}, inner={inner}, outer={outer}, reversals={outward_reversals}, high_shoulders={high_shoulders}",
            levels.len()
        )));
    }
    Ok(())
}

pub(super) fn validate_massif_crown_shape(
    world: &GeneratedWorldPlan,
    crest: TilePos,
    massif_mask: &BTreeSet<HexCoord>,
) -> Result<(), V3GenerationError> {
    let crown = crest
        .coord
        .within_radius(super::super::schematic_highlands::MASSIF_SUMMIT_BODY_RADIUS)
        .into_iter()
        .map(|coord| {
            top_surface(world, coord)
                .filter(|_| massif_mask.contains(&coord))
                .map(|surface| (coord, surface.level))
                .ok_or_else(|| {
                    schematic_contract(format!(
                        "massif summit crown lost its natural surface at {coord:?}"
                    ))
                })
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let maxima = crown
        .iter()
        .filter_map(|(coord, level)| (*level == crest.level).then_some(*coord))
        .collect::<BTreeSet<_>>();
    let near_max = crown
        .values()
        .filter(|level| **level >= crest.level.saturating_sub(8))
        .count();
    let broad_shoulders = crown
        .values()
        .filter(|level| **level >= crest.level.saturating_sub(40))
        .count();
    let expected_crown_size = usize::try_from(
        1_u32.saturating_add(
            3_u32
                .saturating_mul(super::super::schematic_highlands::MASSIF_SUMMIT_BODY_RADIUS)
                .saturating_mul(
                    super::super::schematic_highlands::MASSIF_SUMMIT_BODY_RADIUS.saturating_add(1),
                ),
        ),
    )
    .unwrap_or(usize::MAX);
    if crown.len() != expected_crown_size
        || !(1..=7).contains(&maxima.len())
        || !(1..=25).contains(&near_max)
        || broad_shoulders < 50
        || crown.values().any(|level| *level > crest.level)
        || crown.values().copied().collect::<BTreeSet<_>>().len() < 8
    {
        return Err(schematic_contract(
            "massif summit is capped, columnar, too narrow, or collapsed into regular level bands",
        ));
    }
    for (coord, level) in &crown {
        for neighbor in coord.neighbors() {
            if !massif_mask.contains(&neighbor) {
                continue;
            }
            let neighbor_level = top_surface(world, neighbor)
                .map(|surface| surface.level)
                .ok_or_else(|| {
                    schematic_contract(format!(
                        "massif summit body has no natural transition at {neighbor:?}"
                    ))
                })?;
            if level.abs_diff(neighbor_level) > 9 {
                return Err(schematic_contract(format!(
                    "massif summit body forms a cliff or drum wall between {coord:?} level {level} (radius {}) and {neighbor:?} level {neighbor_level} (radius {})",
                    crest.coord.distance(*coord),
                    crest.coord.distance(neighbor)
                )));
            }
        }
    }
    for direction in 0..6 {
        let profile = (0..=super::super::schematic_highlands::MASSIF_SUMMIT_BODY_RADIUS)
            .map(|distance| {
                let coord = super::super::schematic_highlands::step_in_direction(
                    crest.coord,
                    direction,
                    distance,
                );
                crown.get(&coord).copied()
            })
            .collect::<Option<Vec<_>>>();
        let valid = profile.is_some_and(|levels| {
            levels
                .first()
                .zip(levels.last())
                .is_some_and(|(inner, outer)| outer < inner)
                && levels.windows(2).all(|pair| pair[0].abs_diff(pair[1]) <= 9)
                && levels
                    .windows(5)
                    .all(|window| window.iter().copied().collect::<BTreeSet<_>>().len() > 1)
        });
        if !valid {
            return Err(schematic_contract(format!(
                "massif summit body lost its rising, non-plateau cross-section in direction {direction}"
            )));
        }
    }
    Ok(())
}

/// Revalidates the immutable visual-Massif projection against final ownership.
///
/// Semantic Massif masks are allowed to remain split by Crystal's exact claim;
/// only the scalar field is connected. Every added coordinate must retain the
/// exact overlay-free Mountain owner captured before terrain construction.
fn validate_massif_visual_authority(
    plan: &SchematicPlanV1,
    layout: &ResolvedLayoutPlan,
    crystal_mask: &BTreeSet<HexCoord>,
    semantic_massif_mask: &BTreeSet<HexCoord>,
    authority: &super::super::schematic_highlands::MassifVisualAuthority,
) -> Result<(), V3GenerationError> {
    let connector_coords = authority
        .connector_owners
        .keys()
        .copied()
        .collect::<BTreeSet<_>>();
    let feather_coords = authority
        .feather_owners
        .keys()
        .copied()
        .collect::<BTreeSet<_>>();
    let expected_visual = semantic_massif_mask
        .union(&connector_coords)
        .copied()
        .chain(feather_coords.iter().copied())
        .collect::<BTreeSet<_>>();
    if authority.semantic_owner_mask != *semantic_massif_mask
        || authority.visual_mask != expected_visual
        || !semantic_massif_mask.is_subset(&authority.visual_mask)
        || !connected(&authority.visual_mask)
        || !authority.visual_mask.is_disjoint(crystal_mask)
        || !connector_coords.is_disjoint(&feather_coords)
    {
        return Err(schematic_contract(
            "final Massif visual authority changed semantic ownership, disconnected, or entered Crystal",
        ));
    }

    let cells = plan
        .cells
        .iter()
        .map(|cell| (PatchId(u32::from(cell.id.get())), cell))
        .collect::<BTreeMap<_, _>>();
    for (coord, expected_owner) in &authority.connector_owners {
        let owners = layout
            .patches
            .iter()
            .filter_map(|(owner, patch)| patch.mask.contains(coord).then_some(*owner))
            .collect::<Vec<_>>();
        let [actual_owner] = owners.as_slice() else {
            return Err(schematic_contract(format!(
                "final Massif visual connector {coord:?} has {} owners",
                owners.len()
            )));
        };
        let cell = cells.get(actual_owner).copied().ok_or_else(|| {
            schematic_contract(format!(
                "final Massif visual connector owner {} has no schematic cell",
                actual_owner.0
            ))
        })?;
        if actual_owner != expected_owner
            || cell.facts.surface != SurfaceKind::Land
            || cell.facts.landform != LandformKind::Mountain
            || !cell.facts.overlays.is_empty()
            || crystal_mask.contains(coord)
        {
            return Err(schematic_contract(format!(
                "final Massif visual connector {coord:?} changed owner or left overlay-free Mountain terrain"
            )));
        }
    }
    for (coord, expected_owner) in &authority.feather_owners {
        let owners = layout
            .patches
            .iter()
            .filter_map(|(owner, patch)| patch.mask.contains(coord).then_some(*owner))
            .collect::<Vec<_>>();
        let [actual_owner] = owners.as_slice() else {
            return Err(schematic_contract(format!(
                "final Massif outer feather {coord:?} has {} owners",
                owners.len()
            )));
        };
        let cell = cells.get(actual_owner).copied().ok_or_else(|| {
            schematic_contract(format!(
                "final Massif outer-feather owner {} has no schematic cell",
                actual_owner.0
            ))
        })?;
        if actual_owner != expected_owner
            || cell.facts.surface != SurfaceKind::Land
            || cell.facts.landform != LandformKind::Mountain
            || !cell.facts.overlays.is_empty()
            || crystal_mask.contains(coord)
        {
            return Err(schematic_contract(format!(
                "final Massif outer feather {coord:?} changed owner or left overlay-free Mountain terrain"
            )));
        }
    }
    Ok(())
}

/// Proves the final terrain retains two connected lower chains and twelve
/// independent upper crowns.
///
/// The natural pass, outer upper saddle, and inner-chain ledge may deliberately
/// grade PeakRing ownership. Their exact changed coordinates and levels are
/// sealed before generic route construction; final validation admits only that
/// immutable footprint. Neighboring lower and middle slopes remain continuous,
/// with much-lower natural saddles separating only the crown silhouettes.
pub(super) fn validate_peak_ridge_authority(
    world: &GeneratedWorldPlan,
    authority: &super::super::schematic_highlands::PeakRidgeAuthority,
) -> Result<(), V3GenerationError> {
    if authority.components.len() != 2 {
        return Err(schematic_contract(format!(
            "final peak authority requires two six-body groups, found {}",
            authority.components.len()
        )));
    }
    for (component_index, component) in authority.components.iter().enumerate() {
        let component_mask = component
            .patch_masks
            .values()
            .flat_map(|mask| mask.iter().copied())
            .collect::<BTreeSet<_>>();
        let expected_high_coords = component
            .expected_high_band
            .keys()
            .copied()
            .collect::<BTreeSet<_>>();
        if component.patch_masks.len() != 6
            || component.expected_peak_bodies.len() != 6
            || component.summit_pins.len() != 6
            || component.expected_saddle_swaths.len() < 5
            || component.feather_owners.is_empty()
            || component.feather_boundary_edges.is_empty()
            || !component
                .summit_pins
                .keys()
                .all(|coord| expected_high_coords.contains(coord))
            || component
                .patch_masks
                .values()
                .any(|mask| mask.is_disjoint(&expected_high_coords))
        {
            return Err(schematic_contract(format!(
                "seeded peak group {component_index} has malformed six-body authority"
            )));
        }
        let body_coords = component
            .expected_peak_bodies
            .iter()
            .map(|(patch, body)| (*patch, body, body.keys().copied().collect::<BTreeSet<_>>()))
            .collect::<Vec<_>>();
        for (patch, body, coords) in &body_coords {
            let pin_count = component
                .summit_pins
                .keys()
                .filter(|pin| coords.contains(pin))
                .count();
            let near_max = body.values().copied().max().map_or(0, |maximum| {
                body.values()
                    .filter(|level| **level >= maximum.saturating_sub(8))
                    .count()
            });
            let steep_edge = body.iter().find(|(coord, level)| {
                coord.neighbors().into_iter().any(|neighbor| {
                    body.get(&neighbor)
                        .is_some_and(|neighbor_level| level.abs_diff(*neighbor_level) > 9)
                })
            });
            let summit_coord = component
                .summit_pins
                .keys()
                .find(|pin| coords.contains(pin))
                .copied();
            let silhouette = peak_silhouette_band(body);
            let extents = summit_coord
                .map(|summit| peak_directional_extents(summit, &silhouette))
                .unwrap_or_default();
            let extent_variation = extents
                .iter()
                .copied()
                .max()
                .unwrap_or_default()
                .saturating_sub(extents.iter().copied().min().unwrap_or_default());
            if body.len() < 150
                || !connected(coords)
                || pin_count != 1
                // Cross-patch shoulder overlap can place a neighboring crown
                // above this patch's own pin while the owned body remains a
                // broad, connected, asymmetric formation. Require a paired
                // near-maximum facet rather than four cells measured against
                // that neighboring maximum; the silhouette test below still
                // rejects a rotationally perfect cone.
                || near_max < 1
                || steep_edge.is_some()
                || silhouette.len() < 7
                || extent_variation < 1
                || !component
                    .patch_masks
                    .get(patch)
                    .is_some_and(|mask| coords.is_subset(mask))
            {
                return Err(schematic_contract(format!(
                    "peak body in patch {} is narrow, disconnected, radially regular, too steep, or escaped ownership: columns={}, pins={pin_count}, near_max={near_max}, extents={extents:?}, steep_edge={steep_edge:?}",
                    patch.0,
                    body.len()
                )));
            }
        }
        validate_peak_feather_authority(world, component_index, component, &component_mask)?;
        let authorized_route_grades = component.authorized_route_grades.as_ref().ok_or_else(|| {
            schematic_contract(format!(
                "final peak ridge {component_index} reached validation before its exact route grades were sealed"
            ))
        })?;
        let authorized_waterfall_openings = component
            .authorized_waterfall_openings
            .as_ref()
            .ok_or_else(|| {
                schematic_contract(format!(
                    "final peak ridge {component_index} reached validation before its exact waterfall openings were sealed"
                ))
            })?;
        if !component.ordered_saddle_spines.is_empty() {
            let route = world
                .features
                .protected_routes
                .get("grand_v3.inner_peak_ledge")
                .ok_or_else(|| {
                    schematic_contract(
                        "final ordered peak-saddle spine has no published inner-peak ledge",
                    )
                })?;
            for (owner, spine) in &component.ordered_saddle_spines {
                let owner_mask = component.patch_masks.get(owner).ok_or_else(|| {
                    schematic_contract(format!(
                        "final ordered peak-saddle spine lost owner {}",
                        owner.0
                    ))
                })?;
                let ingress_mask =
                    component
                        .patch_masks
                        .get(&spine.ingress_from)
                        .ok_or_else(|| {
                            schematic_contract(format!(
                                "final ordered peak-saddle spine {} lost ingress owner {}",
                                owner.0, spine.ingress_from.0
                            ))
                        })?;
                let egress_mask = component.patch_masks.get(&spine.egress_to).ok_or_else(|| {
                    schematic_contract(format!(
                        "final ordered peak-saddle spine {} lost egress owner {}",
                        owner.0, spine.egress_to.0
                    ))
                })?;
                let runway = route
                    .centerline
                    .iter()
                    .filter(|surface| owner_mask.contains(&surface.coord))
                    .copied()
                    .collect::<Vec<_>>();
                let expected_runway = spine
                    .centerline
                    .iter()
                    .map(|coord| TilePos::new(*coord, spine.authored_grades[coord]))
                    .collect::<Vec<_>>();
                let ingress = route
                    .centerline
                    .windows(2)
                    .filter_map(|pair| {
                        (ingress_mask.contains(&pair[0].coord)
                            && owner_mask.contains(&pair[1].coord))
                        .then_some((pair[0].coord, pair[1].coord))
                    })
                    .collect::<BTreeSet<_>>();
                let egress = route
                    .centerline
                    .windows(2)
                    .filter_map(|pair| {
                        (owner_mask.contains(&pair[0].coord)
                            && egress_mask.contains(&pair[1].coord))
                        .then_some((pair[0].coord, pair[1].coord))
                    })
                    .collect::<BTreeSet<_>>();
                let changed_owner_coords = authorized_route_grades
                    .keys()
                    .filter(|coord| owner_mask.contains(coord))
                    .copied()
                    .collect::<BTreeSet<_>>();
                if spine.owner != *owner
                    || runway != expected_runway
                    || ingress.len() != 1
                    || egress.len() != 1
                    || !ingress.is_subset(&spine.ingress_portals)
                    || !egress.is_subset(&spine.egress_portals)
                    || !changed_owner_coords.is_subset(&spine.support_domain)
                    || runway.iter().any(|surface| {
                        world.volume.surfaces.get(surface).is_none_or(|metadata| {
                            metadata.access != SurfaceAccess::Ordinary
                                || top_surface(world, surface.coord) != Some(*surface)
                        })
                    })
                {
                    return Err(schematic_contract(format!(
                        "final inner-peak ledge did not retain ordered graded spine {}: runway={runway:?}, expected={expected_runway:?}, ingress={ingress:?}, egress={egress:?}, changed-owner={}",
                        owner.0,
                        changed_owner_coords.len()
                    )));
                }
            }
        }
        if let Some((coord, _)) = authorized_route_grades.iter().find(|(coord, level)| {
            component
                .expected_ridge_profile
                .get(*coord)
                .is_none_or(|expected| expected == *level)
                || component.summit_pins.contains_key(*coord)
        }) {
            return Err(schematic_contract(format!(
                "final peak ridge {component_index} contains malformed route-grade authority at {coord:?}"
            )));
        }
        if let Some((coord, _)) = authorized_waterfall_openings.iter().find(|(coord, level)| {
            component
                .expected_ridge_profile
                .get(*coord)
                .is_none_or(|expected| expected == *level)
                || component.summit_pins.contains_key(*coord)
        }) {
            return Err(schematic_contract(format!(
                "final peak ridge {component_index} contains malformed waterfall-opening authority at {coord:?}"
            )));
        }
        if let Some((coord, expected)) = component.summit_pins.iter().find(|(coord, expected)| {
            top_surface(world, **coord).is_none_or(|surface| surface.level != **expected)
        }) {
            return Err(schematic_contract(format!(
                "final peak summit pin {coord:?} lost deterministic level {expected}"
            )));
        }

        let final_high_band = component_mask
            .iter()
            .copied()
            .filter(|coord| {
                top_surface(world, *coord).is_some_and(|surface| surface.level >= PEAK_SUMMIT_MIN)
            })
            .collect::<BTreeSet<_>>();
        if let Some(coord) = final_high_band.difference(&expected_high_coords).next() {
            return Err(schematic_contract(format!(
                "final peak group {component_index} added an unauthorized tall surface at {coord:?}"
            )));
        }
        let high_components = fine_components(&final_high_band);
        if high_components.len() != 6
            || high_components.iter().any(|high_component| {
                component
                    .summit_pins
                    .keys()
                    .filter(|pin| high_component.contains(pin))
                    .count()
                    != 1
            })
        {
            let component_sizes_and_pins = high_components
                .iter()
                .map(|high_component| {
                    (
                        high_component.len(),
                        component
                            .summit_pins
                            .keys()
                            .filter(|pin| high_component.contains(pin))
                            .count(),
                    )
                })
                .collect::<Vec<_>>();
            return Err(schematic_contract(format!(
                "final peak group {component_index} does not retain six independent summit crowns: components={component_sizes_and_pins:?}"
            )));
        }
        let final_visual_band = component_mask
            .iter()
            .copied()
            .filter(|coord| {
                top_surface(world, *coord)
                    .is_some_and(|surface| surface.level >= PEAK_VISUAL_WALL_THRESHOLD)
            })
            .collect::<BTreeSet<_>>();
        let visual_components = fine_components(&final_visual_band);
        if !has_six_independent_peak_bodies(&visual_components, &component.summit_pins) {
            return Err(schematic_contract(format!(
                "final peak group {component_index} joins independent mountains into a >= {PEAK_VISUAL_WALL_THRESHOLD} wall: components={}",
                visual_components.len()
            )));
        }
        if let Some((patch, _)) = component
            .patch_masks
            .iter()
            .find(|(_, mask)| mask.is_disjoint(&final_high_band))
        {
            return Err(schematic_contract(format!(
                "final peak group {component_index} no longer reaches locked patch {}",
                patch.0
            )));
        }

        if let Some((coord, authorized)) =
            component
                .expected_ridge_profile
                .iter()
                .find_map(|(coord, expected)| {
                    let authorized = authorized_route_grades
                        .get(coord)
                        .copied()
                        .or_else(|| authorized_waterfall_openings.get(coord).copied())
                        .unwrap_or(*expected);
                    top_surface(world, *coord)
                        .is_none_or(|surface| surface.level != authorized)
                        .then_some((*coord, authorized))
                })
        {
            return Err(schematic_contract(format!(
                "final peak group {component_index} changed exact authorized level {authorized} at {coord:?}"
            )));
        }
        if let Some((first, second, step)) =
            component.expected_ridge_profile.keys().find_map(|coord| {
                if authorized_route_grades.contains_key(coord)
                    || authorized_waterfall_openings.contains_key(coord)
                {
                    return None;
                }
                let first = top_surface(world, *coord)?;
                coord.neighbors().into_iter().find_map(|neighbor| {
                    (component.expected_ridge_profile.contains_key(&neighbor)
                        && !authorized_route_grades.contains_key(&neighbor)
                        && !authorized_waterfall_openings.contains_key(&neighbor))
                    .then(|| top_surface(world, neighbor))
                    .flatten()
                    .and_then(|second| {
                        let step = first.level.abs_diff(second.level);
                        (step > 9).then_some((first, second, step))
                    })
                })
            })
        {
            return Err(schematic_contract(format!(
                "final connected peak body exceeds its nine-level slope budget between {first:?} and {second:?}: {step}"
            )));
        }

        let expected_pin_profile = component
            .summit_pins
            .values()
            .copied()
            .collect::<BTreeSet<_>>();
        let final_pin_profile = component
            .summit_pins
            .keys()
            .filter_map(|coord| top_surface(world, *coord).map(|surface| surface.level))
            .collect::<BTreeSet<_>>();
        if final_pin_profile != expected_pin_profile {
            return Err(schematic_contract(format!(
                "final peak ridge {component_index} lost its deterministic summit irregularity"
            )));
        }
        for ((first_patch, second_patch), swath) in &component.expected_saddle_swaths {
            let first_pin = component
                .summit_pins
                .iter()
                .find(|(pin, _)| component.expected_peak_bodies[first_patch].contains_key(pin));
            let second_pin = component
                .summit_pins
                .iter()
                .find(|(pin, _)| component.expected_peak_bodies[second_patch].contains_key(pin));
            let (Some((_, first_level)), Some((_, second_level))) = (first_pin, second_pin) else {
                return Err(schematic_contract(
                    "adjacent peak saddle authority lost one summit owner",
                ));
            };
            let saddle_ceiling = (*first_level)
                .min(*second_level)
                .saturating_sub(30)
                .min(PEAK_VISUAL_WALL_THRESHOLD.saturating_sub(1));
            if let Some(coord) = swath
                .iter()
                .find(|coord| !component.expected_ridge_profile.contains_key(coord))
            {
                return Err(schematic_contract(format!(
                    "adjacent peak saddle lost its sealed foundation level at {coord:?}"
                )));
            }
            let high = swath
                .iter()
                .filter_map(|coord| {
                    let admitted_ceiling =
                        saddle_ceiling.max(component.expected_ridge_profile[coord]);
                    top_surface(world, *coord).filter(|surface| surface.level > admitted_ceiling)
                })
                .collect::<Vec<_>>();
            let liquid_coords = world
                .liquids
                .bodies
                .values()
                .flat_map(|body| body.nodes.keys().map(|surface| surface.coord))
                .collect::<BTreeSet<_>>();
            let dry_scenic = swath
                .iter()
                .copied()
                .filter(|coord| !liquid_coords.contains(coord))
                .filter(|coord| {
                    world
                        .volume
                        .top_surface_at_coord(*coord)
                        .is_some_and(|(_, metadata)| metadata.access != SurfaceAccess::Ordinary)
                })
                .collect::<BTreeSet<_>>();
            let scenic_cross_section = fine_components(&dry_scenic)
                .into_iter()
                .map(|component| component.len())
                .max()
                .unwrap_or_default();
            if swath.len() < 4 || !high.is_empty() || scenic_cross_section < 4 {
                return Err(schematic_contract(format!(
                    "adjacent peak patches {} and {} lost their dry scenic multi-column saddle: swath={}, high={high:?}, scenic_cross_section={scenic_cross_section}, ceiling={saddle_ceiling}",
                    first_patch.0,
                    second_patch.0,
                    swath.len()
                )));
            }
        }
    }
    Ok(())
}

/// Proves the final connected Peak slopes meet the final visual Massif without
/// recreating a cliff at the boundary between their independently-authored
/// scalar fields.
///
/// This deliberately reads final top surfaces. The only exception is an exact
/// endpoint-and-level edge sealed by the non-route Massif scenic experiment;
/// later mutations cannot reuse it. Every other authored route grade,
/// Peak-owned slope, or Mountain feather remains part of the visible seam.
#[cfg(test)]
pub(super) fn validate_peak_massif_seams(
    world: &GeneratedWorldPlan,
    peak_authority: &super::super::schematic_highlands::PeakRidgeAuthority,
    massif_authority: &super::super::schematic_highlands::MassifVisualAuthority,
) -> Result<(), V3GenerationError> {
    validate_peak_massif_seams_with_scenic_cliffs(
        world,
        peak_authority,
        massif_authority,
        &BTreeSet::new(),
    )
}

fn validate_peak_massif_seams_with_scenic_cliffs(
    world: &GeneratedWorldPlan,
    peak_authority: &super::super::schematic_highlands::PeakRidgeAuthority,
    massif_authority: &super::super::schematic_highlands::MassifVisualAuthority,
    massif_scenic_cliff_edges: &BTreeSet<(TilePos, TilePos)>,
) -> Result<(), V3GenerationError> {
    let peak_visual_mask = peak_authority
        .components
        .iter()
        .flat_map(|component| {
            component
                .patch_masks
                .values()
                .flat_map(|mask| mask.iter().copied())
                .chain(component.feather_owners.keys().copied())
        })
        .collect::<BTreeSet<_>>();
    if let Some(overlap) = peak_visual_mask
        .intersection(&massif_authority.visual_mask)
        .next()
    {
        return Err(schematic_contract(format!(
            "Peak and Massif visual authorities overlap at {overlap:?}"
        )));
    }

    for peak_coord in peak_visual_mask {
        for massif_coord in peak_coord
            .neighbors()
            .into_iter()
            .filter(|neighbor| massif_authority.visual_mask.contains(neighbor))
        {
            let peak = top_surface(world, peak_coord).ok_or_else(|| {
                schematic_contract(format!(
                    "direct Peak/Massif seam lost its Peak surface at {peak_coord:?}"
                ))
            })?;
            let massif = top_surface(world, massif_coord).ok_or_else(|| {
                schematic_contract(format!(
                    "direct Peak/Massif seam lost its Massif surface at {massif_coord:?}"
                ))
            })?;
            let step = peak.level.abs_diff(massif.level);
            if step > PEAK_OUTER_FEATHER_MAXIMUM_STEP.unsigned_abs()
                && !admits_exact_scenic_cliff(massif_scenic_cliff_edges, peak, massif)
            {
                return Err(schematic_contract(format!(
                    "direct Peak/Massif seam forms a cliff between {peak:?} and {massif:?}: {step}"
                )));
            }
        }
    }
    Ok(())
}

fn admits_exact_scenic_cliff(
    admitted: &BTreeSet<(TilePos, TilePos)>,
    first: TilePos,
    second: TilePos,
) -> bool {
    admitted.contains(&(first, second)) || admitted.contains(&(second, first))
}

fn peak_silhouette_band(body: &BTreeMap<HexCoord, Level>) -> BTreeSet<HexCoord> {
    let maximum = body.values().copied().max().unwrap_or(Level::MIN);
    let silhouette_floor = PEAK_VISUAL_WALL_THRESHOLD.max(maximum.saturating_sub(56));
    body.iter()
        .filter_map(|(coord, level)| (*level >= silhouette_floor).then_some(*coord))
        .collect()
}

fn validate_peak_feather_authority(
    world: &GeneratedWorldPlan,
    component_index: usize,
    component: &super::super::schematic_highlands::PeakRidgeComponentAuthority,
    component_mask: &BTreeSet<HexCoord>,
) -> Result<(), V3GenerationError> {
    let feather_mask = component
        .feather_owners
        .keys()
        .copied()
        .collect::<BTreeSet<_>>();
    if feather_mask.is_empty()
        || !feather_mask.is_disjoint(component_mask)
        || !connected(
            &component_mask
                .union(&feather_mask)
                .copied()
                .collect::<BTreeSet<_>>(),
        )
    {
        return Err(schematic_contract(format!(
            "peak group {component_index} lost its connected Mountain feather"
        )));
    }
    for (coord, expected_owner) in &component.feather_owners {
        let owners = world
            .layout
            .patches
            .iter()
            .filter_map(|(owner, patch)| patch.mask.contains(coord).then_some(*owner))
            .collect::<Vec<_>>();
        let [actual_owner] = owners.as_slice() else {
            return Err(schematic_contract(format!(
                "peak group {component_index} feather {coord:?} has {} layout owners",
                owners.len()
            )));
        };
        if actual_owner != expected_owner {
            return Err(schematic_contract(format!(
                "peak group {component_index} feather {coord:?} changed owner from {} to {}",
                expected_owner.0, actual_owner.0
            )));
        }
    }
    validate_peak_feather_internal_levels(component_index, &feather_mask, |coord| {
        top_surface(world, coord).map(|surface| surface.level)
    })?;
    validate_peak_feather_edge_levels(
        component_index,
        &component.feather_boundary_edges,
        |coord| top_surface(world, coord).map(|surface| surface.level),
    )
}

fn validate_peak_feather_internal_levels(
    component_index: usize,
    feather_mask: &BTreeSet<HexCoord>,
    mut level_at: impl FnMut(HexCoord) -> Option<Level>,
) -> Result<(), V3GenerationError> {
    for coord in feather_mask {
        let level = level_at(*coord).ok_or_else(|| {
            schematic_contract(format!(
                "peak group {component_index} Mountain feather lost internal surface {coord:?}"
            ))
        })?;
        for neighbor in coord
            .neighbors()
            .into_iter()
            .filter(|neighbor| *coord < *neighbor && feather_mask.contains(neighbor))
        {
            let neighbor_level = level_at(neighbor).ok_or_else(|| {
                schematic_contract(format!(
                    "peak group {component_index} Mountain feather lost internal surface {neighbor:?}"
                ))
            })?;
            let step = level.abs_diff(neighbor_level);
            if step > PEAK_OUTER_FEATHER_MAXIMUM_STEP.unsigned_abs() {
                return Err(schematic_contract(format!(
                    "peak group {component_index} Mountain feather has an internal cliff between {coord:?} level {level} and {neighbor:?} level {neighbor_level}: {step}"
                )));
            }
        }
    }
    Ok(())
}

fn validate_peak_feather_edge_levels(
    component_index: usize,
    edges: &BTreeSet<(HexCoord, HexCoord)>,
    mut level_at: impl FnMut(HexCoord) -> Option<Level>,
) -> Result<(), V3GenerationError> {
    if edges.is_empty() {
        return Err(schematic_contract(format!(
            "peak group {component_index} has no Mountain feather edges"
        )));
    }
    for (inside, outside) in edges {
        if inside.distance(*outside) != 1 {
            return Err(schematic_contract(format!(
                "peak group {component_index} has non-adjacent Mountain feather edge {inside:?} -> {outside:?}"
            )));
        }
        let inside_level = level_at(*inside).ok_or_else(|| {
            schematic_contract(format!(
                "peak group {component_index} Mountain feather lost surface {inside:?}"
            ))
        })?;
        let outside_level = level_at(*outside).ok_or_else(|| {
            schematic_contract(format!(
                "peak group {component_index} Mountain feather lost surface {outside:?}"
            ))
        })?;
        if inside_level.abs_diff(outside_level) > PEAK_OUTER_FEATHER_MAXIMUM_STEP.unsigned_abs() {
            return Err(schematic_contract(format!(
                "peak group {component_index} Mountain feather forms a cliff between {inside:?} level {inside_level} and {outside:?} level {outside_level}"
            )));
        }
    }
    Ok(())
}

fn exceeds_upward_boundary_protrusion(inside: Level, outside: Level, maximum: Level) -> bool {
    inside > outside.saturating_add(maximum)
}

fn true_outer_boundary_edges(
    mask: &BTreeSet<HexCoord>,
    enclosed_hole: &BTreeSet<HexCoord>,
) -> BTreeMap<HexCoord, BTreeSet<HexCoord>> {
    mask.iter()
        .copied()
        .filter_map(|inside| {
            let outside = inside
                .neighbors()
                .into_iter()
                .filter(|neighbor| !mask.contains(neighbor) && !enclosed_hole.contains(neighbor))
                .collect::<BTreeSet<_>>();
            (!outside.is_empty()).then_some((inside, outside))
        })
        .collect()
}

fn retains_connected_boundary_wall(
    protrusions: &BTreeSet<HexCoord>,
    boundary_columns: usize,
    maximum_drop: Level,
) -> bool {
    let longest = fine_components(protrusions)
        .into_iter()
        .map(|component| component.len())
        .max()
        .unwrap_or_default();
    let maximum_sparse_protrusions = boundary_columns
        .saturating_add(CRYSTAL_MANTLE_MAXIMUM_BOUNDARY_PROTRUSION_DIVISOR - 1)
        / CRYSTAL_MANTLE_MAXIMUM_BOUNDARY_PROTRUSION_DIVISOR;
    longest > CRYSTAL_MANTLE_MAXIMUM_CONNECTED_BOUNDARY_PROTRUSIONS
        || protrusions.len() > maximum_sparse_protrusions.max(2)
        || maximum_drop > CRYSTAL_MANTLE_MAXIMUM_ISOLATED_BOUNDARY_DROP
}

fn has_strong_highland_majority(high: usize, total: usize) -> bool {
    total != 0 && high.saturating_mul(20) >= total.saturating_mul(11)
}

fn has_six_independent_peak_bodies(
    components: &[BTreeSet<HexCoord>],
    summit_pins: &BTreeMap<HexCoord, Level>,
) -> bool {
    components.len() == 6
        && components.iter().all(|component| {
            summit_pins
                .keys()
                .filter(|pin| component.contains(pin))
                .count()
                == 1
        })
}

fn validate_frozen_plateau(
    plan: &SchematicPlanV1,
    world: &GeneratedWorldPlan,
    crystal_mask: &BTreeSet<HexCoord>,
) -> Result<(), V3GenerationError> {
    let frozen_patches = patches_for_overlay(plan, SchematicFeature::FrozenWoods);
    let frozen_mask = union_patch_masks(world, &frozen_patches)?;
    let frozen_surfaces = frozen_mask
        .iter()
        .copied()
        .map(|coord| {
            top_surface(world, coord).ok_or_else(|| {
                schematic_contract(format!(
                    "Frozen-Woods plateau lost its terrain surface at {coord:?}"
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let in_band = frozen_surfaces
        .iter()
        .filter(|surface| (FROZEN_PLATEAU_MIN..=FROZEN_PLATEAU_MAX).contains(&surface.level))
        .count();
    if frozen_surfaces.is_empty()
        || in_band.saturating_mul(10) < frozen_surfaces.len().saturating_mul(7)
    {
        return Err(schematic_contract(format!(
            "Frozen Woods is not predominantly a level-151..=153 plateau: in_band={in_band}, total={}",
            frozen_surfaces.len()
        )));
    }
    let exact_route_surfaces = world
        .features
        .protected_routes
        .values()
        .flat_map(|route| route.surfaces.iter().copied())
        .collect::<BTreeSet<_>>();
    if let Some(surface) = frozen_surfaces.iter().find(|surface| {
        surface.level >= PEAK_VISUAL_WALL_THRESHOLD
            || (solid_material_at(&world.volume, **surface) != Some(SolidMaterialRole::Snow)
                && !exact_route_surfaces.contains(surface))
    }) {
        return Err(schematic_contract(format!(
            "Frozen-Woods plateau retained a peak crown or non-snow cap at {surface:?}"
        )));
    }

    let lake_mask = union_patch_masks(
        world,
        &patches_for_overlay(plan, SchematicFeature::MountainLake),
    )?;
    if !frozen_mask.iter().any(|coord| {
        coord
            .neighbors()
            .into_iter()
            .any(|neighbor| lake_mask.contains(&neighbor))
    }) {
        return Err(schematic_contract(
            "Frozen-Woods plateau lost its authored mountain-lake contact",
        ));
    }

    let cells_by_patch = plan
        .cells
        .iter()
        .map(|cell| (PatchId(u32::from(cell.id.get())), cell))
        .collect::<BTreeMap<_, _>>();
    let eligible_halo = world
        .layout
        .patches
        .iter()
        .filter_map(|(patch, resolved)| {
            let cell = cells_by_patch.get(patch)?;
            (cell.facts.surface == SurfaceKind::Land
                && matches!(
                    cell.facts.landform,
                    LandformKind::Mountain | LandformKind::Massif | LandformKind::SharpPeak
                )
                && cell.facts.overlays.iter().all(|overlay| {
                    !matches!(
                        overlay,
                        SchematicFeature::MountainLake
                            | SchematicFeature::LakeIsland
                            | SchematicFeature::Waterfall
                            | SchematicFeature::CrystalAscent
                    )
                }))
            .then_some(resolved.mask.iter().copied())
        })
        .flatten()
        .filter(|coord| !crystal_mask.contains(coord))
        .collect::<BTreeSet<_>>();
    let mut distances = frozen_mask
        .iter()
        .copied()
        .map(|coord| (coord, 0_u32))
        .collect::<BTreeMap<_, _>>();
    let mut frontier = frozen_mask.clone();
    for distance in 1..=FROZEN_PLATEAU_HALO_DEPTH {
        let next = frontier
            .iter()
            .flat_map(|coord| coord.neighbors())
            .filter(|coord| eligible_halo.contains(coord) && !distances.contains_key(coord))
            .collect::<BTreeSet<_>>();
        for coord in &next {
            distances.insert(*coord, distance);
        }
        frontier = next;
    }
    validate_frozen_halo_coverage(&frozen_mask, &eligible_halo, &distances)?;
    // Do not reapply a blanket local-height limit after route and terrain
    // authorities have composed the map. The Frozen field already authors a
    // bounded six-row taper, while exact routes independently prove their
    // walker transitions. A late global edge veto rejects legitimate exposed
    // cliffs and authorised trail seams without detecting a volume or
    // traversal defect.
    Ok(())
}

fn validate_frozen_halo_coverage(
    frozen_mask: &BTreeSet<HexCoord>,
    eligible_halo: &BTreeSet<HexCoord>,
    distances: &BTreeMap<HexCoord, u32>,
) -> Result<(), V3GenerationError> {
    let first_ring = frozen_mask
        .iter()
        .flat_map(|coord| coord.neighbors())
        .filter(|coord| eligible_halo.contains(coord) && !frozen_mask.contains(coord))
        .collect::<BTreeSet<_>>();
    if first_ring.is_empty() {
        return Err(schematic_contract(
            "Frozen-Woods plateau has no eligible first-row mountain blend",
        ));
    }

    for (component_index, component) in fine_components(&first_ring).iter().enumerate() {
        let component_distances =
            graph_distances(component, eligible_halo, FROZEN_PLATEAU_HALO_DEPTH);
        if !component_distances
            .values()
            .any(|distance| *distance == FROZEN_PLATEAU_HALO_DEPTH)
        {
            return Err(schematic_contract(format!(
                "Frozen-Woods first-row blend component {component_index} stops before row {FROZEN_PLATEAU_HALO_DEPTH}"
            )));
        }
    }

    let center = approximate_mask_center(frozen_mask).ok_or_else(|| {
        schematic_contract("Frozen-Woods plateau has no center for halo-sector validation")
    })?;
    let represented_sectors = first_ring
        .iter()
        .map(|coord| crystal_enclosure_sector(center, *coord))
        .collect::<BTreeSet<_>>();
    for sector in represented_sectors {
        let seeds = first_ring
            .iter()
            .copied()
            .filter(|coord| crystal_enclosure_sector(center, *coord) == sector)
            .collect::<BTreeSet<_>>();
        let sector_halo = eligible_halo
            .iter()
            .copied()
            .filter(|coord| crystal_enclosure_sector(center, *coord) == sector)
            .collect::<BTreeSet<_>>();
        let sector_distances = graph_distances(&seeds, &sector_halo, FROZEN_PLATEAU_HALO_DEPTH);
        if !sector_distances
            .values()
            .any(|distance| *distance == FROZEN_PLATEAU_HALO_DEPTH)
        {
            return Err(schematic_contract(format!(
                "Frozen-Woods first-row blend sector {sector} stops before row {FROZEN_PLATEAU_HALO_DEPTH}"
            )));
        }
    }

    if !distances
        .values()
        .any(|distance| *distance == FROZEN_PLATEAU_HALO_DEPTH)
    {
        return Err(schematic_contract(
            "Frozen-Woods plateau does not retain a complete six-row mountain blend",
        ));
    }
    Ok(())
}

fn graph_distances(
    seeds: &BTreeSet<HexCoord>,
    admitted: &BTreeSet<HexCoord>,
    maximum: u32,
) -> BTreeMap<HexCoord, u32> {
    let mut distances = seeds
        .iter()
        .copied()
        .filter(|coord| admitted.contains(coord))
        .map(|coord| (coord, 1_u32))
        .collect::<BTreeMap<_, _>>();
    let mut frontier = distances.keys().copied().collect::<BTreeSet<_>>();
    for distance in 2..=maximum {
        let next = frontier
            .iter()
            .flat_map(|coord| coord.neighbors())
            .filter(|coord| admitted.contains(coord) && !distances.contains_key(coord))
            .collect::<BTreeSet<_>>();
        for coord in &next {
            distances.insert(*coord, distance);
        }
        frontier = next;
    }
    distances
}

fn approximate_mask_center(mask: &BTreeSet<HexCoord>) -> Option<HexCoord> {
    let count = i64::try_from(mask.len()).ok()?;
    if count == 0 {
        return None;
    }
    let (x_total, z_total) = mask.iter().fold((0_i64, 0_i64), |(x, z), coord| {
        let [coord_x, _, coord_z] = coord.to_cubic_array();
        (
            x.saturating_add(i64::from(coord_x)),
            z.saturating_add(i64::from(coord_z)),
        )
    });
    let rounded = |total: i64| {
        let half = count / 2;
        if total >= 0 {
            total.saturating_add(half) / count
        } else {
            total.saturating_sub(half) / count
        }
    };
    Some(HexCoord::from_axial(
        i32::try_from(rounded(x_total)).ok()?,
        i32::try_from(rounded(z_total)).ok()?,
    ))
}

fn validate_frozen_exit(
    plan: &SchematicPlanV1,
    world: &GeneratedWorldPlan,
    crystal_mask: &BTreeSet<HexCoord>,
) -> Result<(), V3GenerationError> {
    let route = world
        .features
        .protected_routes
        .get("grand_v3.frozen_exit")
        .ok_or_else(|| schematic_contract("corrective Crystal route has no Frozen exit"))?;
    let terminal = route
        .centerline
        .last()
        .copied()
        .ok_or_else(|| schematic_contract("corrective Frozen exit has no terminal surface"))?;
    let anchor = world
        .anchors
        .get("grand_v3.frozen_exit")
        .copied()
        .ok_or_else(|| schematic_contract("corrective Frozen exit lost its exact anchor"))?;
    if anchor != terminal {
        return Err(schematic_contract(format!(
            "Frozen-exit anchor {anchor:?} does not equal its final route surface {terminal:?}"
        )));
    }
    let frozen_patches = patches_for_overlay(plan, SchematicFeature::FrozenWoods);
    let frozen_mask = union_patch_masks(world, &frozen_patches)?;
    let outside = route
        .surfaces
        .iter()
        .copied()
        .filter(|surface| !crystal_mask.contains(&surface.coord))
        .collect::<BTreeSet<_>>();
    if route.centerline.len() != 4
        || route.surfaces.len() != 16
        || outside.len() != 8
        || !outside.contains(&terminal)
        || outside
            .iter()
            .any(|surface| !frozen_mask.contains(&surface.coord))
    {
        return Err(schematic_contract(format!(
            "Crystal exit is not two exact four-wide rows in Frozen Woods: centerline={}, surfaces={}, outside={}",
            route.centerline.len(),
            route.surfaces.len(),
            outside.len()
        )));
    }
    if outside
        .iter()
        .any(|surface| solid_material_at(&world.volume, *surface) != Some(SolidMaterialRole::Snow))
    {
        return Err(schematic_contract(
            "Crystal Frozen-Woods exit contains a non-snow surface",
        ));
    }
    if let Some(surface) = outside
        .iter()
        .find(|surface| !(FROZEN_PLATEAU_MIN..=FROZEN_PLATEAU_MAX).contains(&surface.level))
    {
        return Err(schematic_contract(format!(
            "Crystal Frozen-Woods exit left the level-{FROZEN_PLATEAU_MIN}..={FROZEN_PLATEAU_MAX} plateau at {surface:?}"
        )));
    }
    if let Some((from, to)) = route.centerline.windows(2).find_map(|pair| {
        let (Some(from), Some(to)) = (pair.first().copied(), pair.get(1).copied()) else {
            return None;
        };
        (from.level.abs_diff(to.level) > 1).then_some((from, to))
    }) {
        return Err(schematic_contract(format!(
            "Crystal Frozen-Woods exit exceeds its one-level route transition between {from:?} and {to:?}"
        )));
    }
    Ok(())
}

fn validate_garden_island(
    plan: &SchematicPlanV1,
    world: &GeneratedWorldPlan,
) -> Result<(), V3GenerationError> {
    let garden_cell = overlay_cell(plan, SchematicFeature::LakeIsland)?;
    let garden_patch_id = PatchId(u32::from(garden_cell.id.get()));
    let garden_patch = world
        .layout
        .patches
        .get(&garden_patch_id)
        .ok_or_else(|| schematic_contract("Garden island has no final ownership patch"))?;
    let structure = world
        .structures
        .by_id
        .get(&GARDEN_STRUCTURE_ID)
        .ok_or_else(|| schematic_contract("Garden island lost its stable column structure"))?;
    if structure.kind != StructureKind::Tower
        || structure
            .voxels
            .iter()
            .any(|voxel| !garden_patch.mask.contains(&voxel.coord))
    {
        return Err(schematic_contract(
            "Garden island structure is not one site-contained Tower",
        ));
    }
    let by_coord = structure.voxels.iter().fold(
        BTreeMap::<HexCoord, BTreeSet<Level>>::new(),
        |mut grouped, voxel| {
            grouped.entry(voxel.coord).or_default().insert(voxel.level);
            grouped
        },
    );
    let supports = by_coord
        .iter()
        .filter(|(_, levels)| levels.len() > 1)
        .collect::<Vec<_>>();
    let canopy = by_coord
        .values()
        .filter(|levels| levels.len() == 1)
        .collect::<Vec<_>>();
    if by_coord.len() != 16 || supports.len() != 6 || canopy.len() != 10 {
        return Err(schematic_contract(format!(
            "Garden architecture requires six columns and ten broken-canopy cells; coords={}, supports={}, canopy={}",
            by_coord.len(),
            supports.len(),
            canopy.len()
        )));
    }
    for (coord, levels) in supports {
        let first = levels.first().copied().unwrap_or_default();
        let last = levels.last().copied().unwrap_or_default();
        if usize::try_from(last.saturating_sub(first).saturating_add(1)).ok() != Some(levels.len())
            || first <= 0
            || solid_material_at(&world.volume, TilePos::new(*coord, first - 1)).is_none()
        {
            return Err(schematic_contract(format!(
                "Garden support {coord:?} is discontinuous or ungrounded"
            )));
        }
    }

    let structure_coords = by_coord.keys().copied().collect::<BTreeSet<_>>();
    for coord in garden_patch.mask.difference(&structure_coords) {
        let surface = top_surface(world, *coord).ok_or_else(|| {
            schematic_contract(format!("Garden island has no surface at {coord:?}"))
        })?;
        if solid_material_at(&world.volume, surface) != Some(SolidMaterialRole::Grass) {
            return Err(schematic_contract(format!(
                "Garden island natural cap {surface:?} is not warm grass"
            )));
        }
    }
    let garden_trees = world
        .features
        .by_id
        .values()
        .filter(|feature| {
            feature.kind == FeatureKind::Tree && garden_patch.mask.contains(&feature.root.coord)
        })
        .collect::<Vec<_>>();
    if garden_trees.len() < 6
        || garden_trees
            .iter()
            .any(|tree| tree.object_id.as_str().contains("snowy"))
    {
        return Err(schematic_contract(format!(
            "Garden island must keep many temperate trees and no snowy variants; found {}",
            garden_trees.len()
        )));
    }
    Ok(())
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct WoodlandCoverageBand {
    /// Number of fine columns below the exact organic treeline in this band.
    admitted_columns: u128,
    /// Sum of per-column semantic canopy percentages. This is an aggregate
    /// coverage target, not a fragile count of tree objects with different-sized
    /// canopies.
    weighted_canopy_percent: u128,
    final_tree_roots: usize,
}

impl WoodlandCoverageBand {
    fn add_columns(&mut self, columns: usize, density: VegetationDensity) {
        let columns = u128::try_from(columns).unwrap_or(u128::MAX);
        self.admitted_columns = self.admitted_columns.saturating_add(columns);
        self.weighted_canopy_percent =
            self.weighted_canopy_percent
                .saturating_add(columns.saturating_mul(u128::from(
                    super::super::schematic_ecology::vegetation_coverage_percent(density),
                )));
    }

    fn strictly_exceeds(self, other: Self) -> bool {
        self.admitted_columns > 0
            && other.admitted_columns > 0
            && self
                .weighted_canopy_percent
                .saturating_mul(other.admitted_columns)
                > other
                    .weighted_canopy_percent
                    .saturating_mul(self.admitted_columns)
    }

    fn basis_points(self) -> u128 {
        if self.admitted_columns == 0 {
            return 0;
        }
        self.weighted_canopy_percent
            .saturating_mul(100)
            .checked_div(self.admitted_columns)
            .unwrap_or_default()
    }
}

fn gradient_band(landform: LandformKind) -> Option<usize> {
    match landform {
        LandformKind::Valley => Some(0),
        LandformKind::Hill => Some(1),
        LandformKind::Mountain => Some(2),
        LandformKind::None
        | LandformKind::Island
        | LandformKind::Beach
        | LandformKind::Shore
        | LandformKind::Plateau
        | LandformKind::Massif
        | LandformKind::SharpPeak => None,
    }
}

fn authored_tree_exception(cell: &CellPlan) -> bool {
    cell.facts.overlays.iter().any(|overlay| {
        matches!(
            overlay,
            SchematicFeature::LakeIsland
                | SchematicFeature::FrozenWoods
                | SchematicFeature::CrystalAscent
        )
    })
}

fn validate_woodland_coverage_order(
    valley: WoodlandCoverageBand,
    hill: WoodlandCoverageBand,
    mountain: WoodlandCoverageBand,
) -> Result<(), V3GenerationError> {
    if !valley.strictly_exceeds(hill)
        || !hill.strictly_exceeds(mountain)
        || [valley, hill, mountain]
            .iter()
            .any(|band| band.final_tree_roots == 0)
    {
        return Err(schematic_contract(format!(
            "final woodland gradient is not valley > hill > mountain-base with populated bands: valley={}bp/{} roots, hill={}bp/{} roots, mountain={}bp/{} roots",
            valley.basis_points(),
            valley.final_tree_roots,
            hill.basis_points(),
            hill.final_tree_roots,
            mountain.basis_points(),
            mountain.final_tree_roots,
        )));
    }
    Ok(())
}

/// Proves the final composed world, rather than only the placement policy, has
/// the intended vegetation taper. Coverage compares semantic canopy percentages
/// over every fine column admitted below the organic treeline, so differently
/// sized tree silhouettes and small exclusion changes cannot make the contract
/// oscillate. Exact final roots separately prove every band is populated and no
/// ordinary tree escaped above its coordinate-specific treeline or into a summit.
fn validate_vegetation_gradient(
    plan: &SchematicPlanV1,
    world: &GeneratedWorldPlan,
) -> Result<(), V3GenerationError> {
    let cells = plan
        .cells
        .iter()
        .map(|cell| (PatchId(u32::from(cell.id.get())), cell))
        .collect::<BTreeMap<_, _>>();
    let mut owner_by_coord = BTreeMap::new();
    let mut bands = [WoodlandCoverageBand::default(); 3];
    for (patch_id, patch) in &world.layout.patches {
        let cell = cells.get(patch_id).copied().ok_or_else(|| {
            schematic_contract(format!(
                "woodland gradient patch {} has no schematic owner",
                patch_id.0
            ))
        })?;
        for coord in &patch.mask {
            if owner_by_coord.insert(*coord, *patch_id).is_some() {
                return Err(schematic_contract(format!(
                    "woodland gradient found duplicate fine owner at {coord:?}"
                )));
            }
        }
        let Some(index) = gradient_band(cell.facts.landform) else {
            continue;
        };
        if authored_tree_exception(cell) {
            continue;
        }
        let admitted = patch
            .mask
            .iter()
            .filter_map(|coord| top_surface(world, *coord))
            .filter(|surface| {
                super::super::schematic_ecology::tree_root_is_admitted(
                    cell,
                    *surface,
                    plan.provenance.world_seed,
                )
            })
            .count();
        bands[index].add_columns(
            admitted,
            super::super::schematic_ecology::vegetation_policy(cell).density,
        );
    }

    for tree in world
        .features
        .by_id
        .values()
        .filter(|feature| feature.kind == FeatureKind::Tree)
    {
        let patch_id = owner_by_coord.get(&tree.root.coord).ok_or_else(|| {
            schematic_contract(format!(
                "final tree root {:?} has no schematic owner",
                tree.root
            ))
        })?;
        let cell = cells.get(patch_id).copied().ok_or_else(|| {
            schematic_contract(format!(
                "final tree root {:?} resolved an absent schematic owner",
                tree.root
            ))
        })?;
        if authored_tree_exception(cell) {
            continue;
        }
        if !super::super::schematic_ecology::tree_root_is_admitted(
            cell,
            tree.root,
            plan.provenance.world_seed,
        ) {
            return Err(schematic_contract(format!(
                "ordinary tree root {:?} escaped above its exact organic treeline",
                tree.root
            )));
        }
        if matches!(
            cell.facts.landform,
            LandformKind::Massif | LandformKind::SharpPeak
        ) {
            return Err(schematic_contract(format!(
                "ordinary tree root {:?} entered a treeless highland summit band",
                tree.root
            )));
        }
        if let Some(index) = gradient_band(cell.facts.landform) {
            bands[index].final_tree_roots = bands[index].final_tree_roots.saturating_add(1);
        }
    }

    let [valley, hill, mountain] = bands;
    validate_woodland_coverage_order(valley, hill, mountain)
}

fn validate_certain_snow_caps(
    plan: &SchematicPlanV1,
    world: &GeneratedWorldPlan,
    crystal_mask: &BTreeSet<HexCoord>,
) -> Result<(), V3GenerationError> {
    // Route and broad Crystal membership are diagnostic facts, not exemptions.
    // Every high Grass/Gravel/Dirt/Sand cap remains a visible stripe.
    let route_coords = world
        .features
        .protected_routes
        .values()
        .flat_map(|route| route.surfaces.iter().map(|surface| surface.coord))
        .collect::<BTreeSet<_>>();
    let highest_fill_by_coord = world.volume.fill_runs_by_top().keys().fold(
        BTreeMap::<HexCoord, Level>::new(),
        |mut highest, fill| {
            highest
                .entry(fill.coord)
                .and_modify(|level| *level = (*level).max(fill.level))
                .or_insert(fill.level);
            highest
        },
    );
    let garden_patches = patches_for_overlay(plan, SchematicFeature::LakeIsland);
    let garden_mask = union_patch_masks(world, &garden_patches)?;
    let peak_mask = union_patch_masks(world, &patches_for_landform(plan, LandformKind::SharpPeak))?;
    let massif_mask = union_patch_masks(world, &patches_for_landform(plan, LandformKind::Massif))?;
    let unsnowed = world
        .layout
        .footprint
        .iter()
        .filter_map(|coord| {
            let surface = top_surface(world, *coord)?;
            if surface.level < CERTAIN_SNOW_LEVEL {
                return None;
            }
            let material = solid_material_at(&world.volume, surface);
            let requires_explicit_summit_snow = (peak_mask.contains(coord)
                && surface.level >= PEAK_SUMMIT_MIN)
                || (massif_mask.contains(coord) && surface.level >= MASSIF_SUMMIT_MIN);
            high_cap_violates_snow_contract(
                material,
                fill_covers_surface(highest_fill_by_coord.get(coord).copied(), surface.level),
                garden_mask.contains(coord),
                requires_explicit_summit_snow,
            )
            .then_some((
                surface,
                material,
                route_coords.contains(coord),
                crystal_mask.contains(coord),
            ))
        })
        .take(8)
        .collect::<Vec<_>>();
    if !unsnowed.is_empty() {
        return Err(schematic_contract(format!(
            "high-altitude natural or summit-band caps remain unsnowed (surface, material, protected-route, Crystal-site): {unsnowed:?}"
        )));
    }
    Ok(())
}

/// Whether an exposed high cap uses one of the natural roles reconciled by the
/// organic snowline but is not snow. Outside exact summit bands, authored
/// masonry, rock, metal, ice, and basalt remain valid high-altitude exceptions.
fn unsnowed_natural_cap(material: Option<SolidMaterialRole>) -> bool {
    matches!(
        material,
        Some(
            SolidMaterialRole::Dirt
                | SolidMaterialRole::Grass
                | SolidMaterialRole::Gravel
                | SolidMaterialRole::Sand
        )
    )
}

/// The high lake's water bed and magical Garden island are the only
/// coordinate-level exceptions. Protected routes and Crystal-site membership
/// are deliberately absent: they never excuse an exposed natural cap.
fn high_cap_violates_snow_contract(
    material: Option<SolidMaterialRole>,
    is_water: bool,
    is_garden: bool,
    requires_explicit_summit_snow: bool,
) -> bool {
    if is_water || is_garden {
        return false;
    }
    if requires_explicit_summit_snow {
        return material != Some(SolidMaterialRole::Snow);
    }
    unsnowed_natural_cap(material)
}

/// A lower stacked fill does not visually cover a higher exposed terrain cap.
/// Only a fill whose top occupied voxel reaches that exact surface may use the
/// high-lake exemption.
fn fill_covers_surface(highest_fill: Option<Level>, surface: Level) -> bool {
    highest_fill.is_some_and(|fill| fill >= surface)
}

fn validate_waterfall_and_review_anchor(
    world: &GeneratedWorldPlan,
    profile: V3GrandV3BasicTerrainProfile,
    hydrology: &HydrologyCompilation,
) -> Result<(), V3GenerationError> {
    validate_plunge_profile(
        &hydrology.waterfall_centerline,
        profile.mountain_lake_level,
        profile.valley_lake_level,
        hydrology.waterfall_lip_index,
    )?;
    validate_waterfall_cliff_interface(
        &world.volume,
        &hydrology.watercourse_rows,
        hydrology.waterfall_lip_index,
        &hydrology.waterfall_cliff,
    )?;
    let gorge_coords = hydrology
        .waterfall_cliff
        .gorge_surfaces
        .iter()
        .map(|surface| surface.coord)
        .collect::<BTreeSet<_>>();
    if let Some((name, surface)) =
        world
            .features
            .protected_routes
            .iter()
            .find_map(|(name, route)| {
                route
                    .surfaces
                    .iter()
                    .find(|surface| gorge_coords.contains(&surface.coord))
                    .copied()
                    .map(|surface| (name, surface))
            })
    {
        return Err(schematic_contract(format!(
            "protected route {name:?} entered the sealed waterfall gorge at {surface:?}"
        )));
    }
    let drops = hydrology
        .waterfall_centerline
        .windows(2)
        .enumerate()
        .filter_map(|(index, pair)| {
            (pair[0].level.saturating_sub(pair[1].level) >= 4).then_some((index, pair))
        })
        .collect::<Vec<_>>();
    if !(7..=9).contains(&drops.len()) {
        return Err(schematic_contract(format!(
            "corrective waterfall requires every small cascade and the final fall, found {} descending transitions",
            drops.len()
        )));
    }
    let nodes = world
        .liquids
        .bodies
        .values()
        .flat_map(|body| body.nodes.iter().map(|(position, node)| (*position, *node)))
        .collect::<BTreeMap<_, _>>();
    for (stage, (drop_index, _)) in drops.iter().enumerate() {
        let source_row = hydrology
            .watercourse_rows
            .get(*drop_index)
            .ok_or_else(|| schematic_contract("waterfall stage has no exact source row"))?;
        let sink_row = hydrology
            .watercourse_rows
            .get((*drop_index).saturating_add(1))
            .ok_or_else(|| schematic_contract("waterfall stage has no exact sink row"))?;
        if source_row.len() != 3
            || sink_row.len() != 3
            || source_row.iter().any(|source| {
                nodes.get(source).is_none_or(|node| {
                    node.state != LiquidFlowState::Fall
                        || node
                            .downstream
                            .is_none_or(|target| !sink_row.contains(&target))
                })
            })
        {
            return Err(schematic_contract(format!(
                "all three waterfall lanes must publish an exact Fall into cascade stage {}",
                stage.saturating_add(1)
            )));
        }
    }
    let (waterfall_crown, waterfall_base, profile_source) =
        waterfall_review_targets(&hydrology.waterfall_centerline).ok_or_else(|| {
            schematic_contract("waterfall review cannot resolve its cascade targets")
        })?;
    for (anchors, kind, name, target, maximum_distance) in [
        (
            &world.observation_anchors,
            "observation",
            "grand_v3.waterfall_crown",
            waterfall_crown,
            12,
        ),
        // The jagged radius-twelve gorge has one exact liquid lane outside its
        // dry shoulder. The nearest local base observation can therefore be
        // thirteen hexes from the centerline target without weakening the
        // authored gorge or moving the gameplay route.
        (
            &world.observation_anchors,
            "observation",
            "grand_v3.waterfall_base",
            waterfall_base,
            13,
        ),
        (
            &world.anchors,
            "gameplay",
            "grand_v3.waterfall_profile",
            profile_source,
            WATERFALL_GORGE_MAXIMUM_RADIUS.saturating_add(WATERFALL_GORGE_MINIMUM_SHOULDER_REACH),
        ),
    ] {
        let anchor = anchors.get(name).ok_or_else(|| {
            schematic_contract(format!("waterfall review lost {kind} anchor {name}"))
        })?;
        if anchor.coord.distance(target.coord) > maximum_distance {
            return Err(schematic_contract(format!(
                "waterfall review anchor {name} drifted {} hexes from its authored target",
                anchor.coord.distance(target.coord)
            )));
        }
    }
    Ok(())
}

fn validate_river_and_review_anchor(
    plan: &SchematicPlanV1,
    world: &GeneratedWorldPlan,
    hydrology: &HydrologyCompilation,
) -> Result<(), V3GenerationError> {
    let coarse =
        schematic_network_path(plan, NetworkKind::Hydrology, "edge/hydrology-valley-to-sea")?;
    let direct = fine_network_path(&coarse, CELL_PITCH)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let coords = hydrology
        .river_centerline
        .iter()
        .map(|surface| surface.coord)
        .collect::<Vec<_>>();
    let bend = coords
        .iter()
        .copied()
        .max_by_key(|coord| {
            (
                direct
                    .iter()
                    .map(|direct_coord| coord.distance(*direct_coord))
                    .min()
                    .unwrap_or_default(),
                Reverse(*coord),
            )
        })
        .ok_or_else(|| schematic_contract("corrective river has no centerline"))?;
    let excursion = direct
        .iter()
        .map(|direct_coord| bend.distance(*direct_coord))
        .min()
        .unwrap_or_default();
    let direction_count = coords
        .windows(2)
        .filter_map(|pair| {
            pair[0]
                .neighbors()
                .iter()
                .position(|neighbor| *neighbor == pair[1])
        })
        .collect::<BTreeSet<_>>()
        .len();
    if excursion < 3 || direction_count < 2 || longest_straight_run(&coords) >= 44 {
        return Err(schematic_contract(format!(
            "river is not visibly bent: excursion={excursion}, directions={direction_count}"
        )));
    }
    let anchor = world
        .anchors
        .get("grand_v3.river_bend")
        .ok_or_else(|| schematic_contract("corrective river lost its bend review anchor"))?;
    if anchor.coord.distance(bend) > 8 {
        return Err(schematic_contract(format!(
            "river bend review anchor drifted {} hexes from maximum excursion",
            anchor.coord.distance(bend)
        )));
    }
    Ok(())
}

fn validate_semantic_review_anchors(
    plan: &SchematicPlanV1,
    world: &GeneratedWorldPlan,
    validation: &CorrectiveWorldValidation<'_>,
) -> Result<(), V3GenerationError> {
    let mantle = world
        .anchors
        .get("grand_v3.crystal_mantle_overlook")
        .copied()
        .ok_or_else(|| schematic_contract("corrective review lost its Crystal-mantle overlook"))?;
    if mantle != validation.review.mantle
        || !validation.reachable.contains_key(&mantle)
        || world
            .volume
            .top_surface_at_coord(mantle.coord)
            .is_none_or(|(top, metadata)| {
                top != mantle || metadata.access != SurfaceAccess::Ordinary
            })
    {
        return Err(schematic_contract(format!(
            "Crystal-mantle overlook {mantle:?} lost its sealed reachable Ordinary top-surface authority {:?}",
            validation.review.mantle
        )));
    }
    let crystal_center = super::exact_hex_disk_center(
        validation.crystal_mask,
        super::super::schematic_highlands::CRYSTAL_SITE_RADIUS,
    )
    .ok_or_else(|| schematic_contract("Crystal-mantle review lost the exact site centre"))?;
    let durable_hub_coords = world
        .features
        .protected_routes
        .get("grand_v3.ordinary_hubs")
        .ok_or_else(|| schematic_contract("Crystal-mantle review lost durable hub authority"))?
        .surfaces
        .iter()
        .map(|surface| surface.coord)
        .collect::<BTreeSet<_>>();
    let blocked_origins = validation
        .reachable
        .keys()
        .copied()
        .filter(|surface| surface.coord.distance(mantle.coord) <= 3)
        .filter(|surface| durable_hub_coords.contains(&surface.coord))
        .filter(|surface| {
            world
                .volume
                .surfaces
                .get(surface)
                .is_some_and(|metadata| metadata.access == SurfaceAccess::Ordinary)
                && super::mantle_screen_blocks_review_line(
                    world,
                    &validation.crystal_mantle.uplift_core,
                    surface.coord,
                    crystal_center,
                    validation.crystal_terrain_top,
                )
        })
        .count();
    if blocked_origins < 3 {
        return Err(schematic_contract(format!(
            "Crystal's offset natural ridge blocks only {blocked_origins} nearby valley review sightlines; expected at least 3"
        )));
    }

    let peak = world
        .anchors
        .get("grand_v3.peak_ridge_overlook")
        .copied()
        .ok_or_else(|| schematic_contract("corrective review lost its peak-ridge overlook"))?;
    let foothill = world
        .anchors
        .get("grand_v3.peak_foothill_ledge")
        .copied()
        .ok_or_else(|| schematic_contract("corrective review lost its authored peak ledge"))?;
    let peak_route = world
        .features
        .protected_routes
        .get("grand_v3.inner_peak_ledge")
        .ok_or_else(|| {
            schematic_contract("corrective review lost its protected inner peak ledge")
        })?;
    if peak != validation.review.peak
        || foothill != validation.review.peak
        || !peak_route.surfaces.contains(&peak)
        || !validation.reachable.contains_key(&peak)
    {
        return Err(schematic_contract(format!(
            "peak-ridge overlook {peak:?} is not its sealed authored reachable foothill ledge {:?} (published foothill {foothill:?})",
            validation.review.peak
        )));
    }
    let peak_components = peak_component_masks(plan, world)?;
    let [first_peak_chain, second_peak_chain] = peak_components.as_slice() else {
        return Err(schematic_contract(format!(
            "peak-ridge review requires two peak chains; found {}",
            peak_components.len()
        )));
    };
    let (waterfall_crown, _, _) =
        waterfall_review_targets(&validation.hydrology.waterfall_centerline)
            .ok_or_else(|| schematic_contract("peak-ridge review has no waterfall opening"))?;
    let first_distance = peak
        .coord
        .distance_to_set(first_peak_chain)
        .unwrap_or(u32::MAX);
    let second_distance = peak
        .coord
        .distance_to_set(second_peak_chain)
        .unwrap_or(u32::MAX);
    let waterfall_distance = peak.coord.distance(waterfall_crown.coord);
    if first_distance > u32::try_from(CELL_PITCH.saturating_mul(4)).unwrap_or(u32::MAX)
        || second_distance > u32::try_from(CELL_PITCH.saturating_mul(4)).unwrap_or(u32::MAX)
        || waterfall_distance > u32::try_from(CELL_PITCH.saturating_mul(3)).unwrap_or(u32::MAX)
    {
        return Err(schematic_contract(format!(
            "peak-ridge overlook does not frame both chains and the waterfall opening: chain distances={first_distance}/{second_distance}, waterfall={waterfall_distance}"
        )));
    }

    let treeline = world
        .anchors
        .get("grand_v3.treeline_transition")
        .copied()
        .ok_or_else(|| schematic_contract("corrective review lost its treeline transition"))?;
    let witnesses = validation.review.treeline_witnesses;
    let downhill_tree_exists = world.features.by_id.values().any(|feature| {
        feature.kind == FeatureKind::Tree && feature.root == witnesses.downhill_tree
    });
    let uphill_snow_exists = world
        .volume
        .top_surface_at_coord(witnesses.uphill_snow.coord)
        .is_some_and(|(top, _)| {
            top == witnesses.uphill_snow
                && solid_material_at(&world.volume, top) == Some(SolidMaterialRole::Snow)
        });
    let treeline_is_ordinary_top = world
        .volume
        .top_surface_at_coord(treeline.coord)
        .is_some_and(|(top, metadata)| {
            top == treeline && metadata.access == SurfaceAccess::Ordinary
        });
    if treeline != validation.review.treeline
        || !validation.reachable.contains_key(&treeline)
        || !treeline_is_ordinary_top
        || solid_material_at(&world.volume, treeline) != Some(SolidMaterialRole::Snow)
        || !downhill_tree_exists
        || !uphill_snow_exists
        || witnesses.downhill_tree.level >= treeline.level
        || witnesses.uphill_snow.level <= treeline.level
        || !review_vectors_face_opposite(
            treeline.coord,
            witnesses.downhill_tree.coord,
            witnesses.uphill_snow.coord,
        )
    {
        return Err(schematic_contract(format!(
            "treeline anchor {treeline:?} lost its sealed snowy/tree/uphill authority: expected={:?}, witnesses={witnesses:?}, tree_exists={downhill_tree_exists}, snow_exists={uphill_snow_exists}",
            validation.review.treeline
        )));
    }
    Ok(())
}

fn peak_component_masks(
    plan: &SchematicPlanV1,
    world: &GeneratedWorldPlan,
) -> Result<Vec<BTreeSet<HexCoord>>, V3GenerationError> {
    let cells = plan
        .cells
        .iter()
        .filter(|cell| {
            cell.facts.landform == LandformKind::SharpPeak
                && cell.facts.overlays.contains(&SchematicFeature::PeakRing)
        })
        .map(|cell| (cell.coord, PatchId(u32::from(cell.id.get()))))
        .collect::<BTreeMap<_, _>>();
    let mut remaining = cells.keys().copied().collect::<BTreeSet<_>>();
    let mut component_masks = Vec::new();
    while let Some(start) = remaining.pop_first() {
        let mut schematic_component = BTreeSet::from([start]);
        let mut queue = VecDeque::from([start]);
        while let Some(current) = queue.pop_front() {
            let neighbors = current.neighbors().ok_or_else(|| {
                schematic_contract(format!(
                    "peak-ring schematic coordinate {current:?} overflowed while finding neighbors"
                ))
            })?;
            for neighbor in neighbors {
                if remaining.remove(&neighbor) {
                    schematic_component.insert(neighbor);
                    queue.push_back(neighbor);
                }
            }
        }
        let patches = schematic_component
            .iter()
            .filter_map(|coord| cells.get(coord).copied())
            .collect::<BTreeSet<_>>();
        component_masks.push(union_patch_masks(world, &patches)?);
    }
    component_masks.sort_by_key(|mask| mask.first().copied());
    Ok(component_masks)
}

fn overlay_cell(
    plan: &SchematicPlanV1,
    overlay: SchematicFeature,
) -> Result<&CellPlan, V3GenerationError> {
    let matches = plan
        .cells
        .iter()
        .filter(|cell| cell.facts.overlays.contains(&overlay))
        .collect::<Vec<_>>();
    let [cell] = matches.as_slice() else {
        return Err(schematic_contract(format!(
            "corrective contract requires exactly one {overlay:?} cell; found {}",
            matches.len()
        )));
    };
    Ok(*cell)
}

fn patches_for_overlay(plan: &SchematicPlanV1, overlay: SchematicFeature) -> BTreeSet<PatchId> {
    plan.cells
        .iter()
        .filter(|cell| cell.facts.overlays.contains(&overlay))
        .map(|cell| PatchId(u32::from(cell.id.get())))
        .collect()
}

fn patches_for_landform(plan: &SchematicPlanV1, landform: LandformKind) -> BTreeSet<PatchId> {
    plan.cells
        .iter()
        .filter(|cell| cell.facts.landform == landform)
        .map(|cell| PatchId(u32::from(cell.id.get())))
        .collect()
}

fn union_patch_masks(
    world: &GeneratedWorldPlan,
    patches: &BTreeSet<PatchId>,
) -> Result<BTreeSet<HexCoord>, V3GenerationError> {
    let mask = patches
        .iter()
        .filter_map(|patch| world.layout.patches.get(patch))
        .flat_map(|patch| patch.mask.iter().copied())
        .collect::<BTreeSet<_>>();
    if mask.is_empty() {
        Err(schematic_contract(
            "corrective contract resolved an empty semantic mask",
        ))
    } else {
        Ok(mask)
    }
}

fn top_surface(world: &GeneratedWorldPlan, coord: HexCoord) -> Option<TilePos> {
    world
        .volume
        .surfaces_at_coord(coord)
        .map(|(surface, _)| *surface)
        .max()
}

fn solid_mass_at_surface(volume: &VolumePlan, surface: TilePos) -> Option<SolidMass> {
    volume
        .columns
        .get(&surface.coord)?
        .elements
        .iter()
        .find_map(|element| {
            let VolumeElement::Solid(mass) = *element else {
                return None;
            };
            (mass.levels.bottom <= surface.level && surface.level < mass.levels.top).then_some(mass)
        })
}

fn validate_crystal_shell_openings(
    world: &GeneratedWorldPlan,
    crystal_mask: &BTreeSet<HexCoord>,
    rotation_turns: u8,
    exposed_openings: &BTreeSet<HexCoord>,
    expected_upper_exit: TilePos,
) -> Result<(), V3GenerationError> {
    let lower_level = world
        .anchors
        .get("crystal_ascent.lower_entry")
        .map(|surface| surface.level)
        .ok_or_else(|| schematic_contract("composite Crystal lost its exact lower entry"))?;
    let upper_exit = world
        .anchors
        .get("crystal_ascent.upper_exit")
        .copied()
        .ok_or_else(|| schematic_contract("composite Crystal lost its exact summit exit"))?;
    if upper_exit != expected_upper_exit {
        return Err(schematic_contract(format!(
            "composite Crystal summit opening drifted from {expected_upper_exit:?} to {upper_exit:?}"
        )));
    }
    let lower = super::super::crystal_ascent::macro_composite_lower_aperture_coords(
        crystal_mask,
        rotation_turns,
    )
    .map_err(schematic_contract)?;
    let lower_headrooms = super::super::crystal_ascent::macro_composite_lower_aperture_headrooms(
        crystal_mask,
        rotation_turns,
    )
    .map_err(schematic_contract)?;
    let upper = super::super::crystal_ascent::macro_composite_upper_trail_coords(
        crystal_mask,
        rotation_turns,
    )
    .map_err(schematic_contract)?;
    let expected_openings = lower.union(&upper).copied().collect::<BTreeSet<_>>();
    if expected_openings != *exposed_openings
        || lower_headrooms.keys().any(|coord| !lower.contains(coord))
    {
        return Err(schematic_contract(
            "composite Crystal opening authority no longer equals its exact lower aperture and summit trail",
        ));
    }

    validate_crystal_opening_geometry(
        &lower,
        &lower_headrooms,
        &upper,
        lower_level,
        upper_exit.level,
        SolidMaterialRole::Snow,
        |surface| world.volume.surfaces.contains_key(&surface),
        |position| solid_mass_at_surface(&world.volume, position).map(|mass| mass.material),
        |surface| world.volume.surface_clearance(surface),
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "the validator keeps each independently supplied Crystal contract fact explicit"
)]
fn validate_crystal_opening_geometry(
    lower: &BTreeSet<HexCoord>,
    lower_headrooms: &BTreeMap<HexCoord, Level>,
    upper: &BTreeSet<HexCoord>,
    lower_level: Level,
    summit_level: Level,
    expected_summit_material: SolidMaterialRole,
    mut has_surface: impl FnMut(TilePos) -> bool,
    mut material_at: impl FnMut(TilePos) -> Option<SolidMaterialRole>,
    mut clearance_at: impl FnMut(TilePos) -> Option<Level>,
) -> Result<(), V3GenerationError> {
    for coord in lower {
        let floor = TilePos::new(*coord, lower_level);
        if !has_surface(floor) || material_at(floor) != Some(SolidMaterialRole::WorkedStone) {
            return Err(schematic_contract(format!(
                "composite Crystal lower aperture lost its exact worked-stone floor at {floor:?}"
            )));
        }
    }
    for (coord, expected_headroom) in lower_headrooms {
        let floor = TilePos::new(*coord, lower_level);
        let actual_clearance = clearance_at(floor);
        let cap = TilePos::new(
            *coord,
            lower_level
                .saturating_add(*expected_headroom)
                .saturating_add(1),
        );
        if actual_clearance != Some(*expected_headroom)
            || material_at(cap) != Some(SolidMaterialRole::WorkedStone)
        {
            return Err(schematic_contract(format!(
                "composite Crystal lower aperture is capped or filled incorrectly at {floor:?}: clearance={actual_clearance:?}, expected={expected_headroom}, cap={:?}",
                material_at(cap)
            )));
        }
    }
    for coord in upper {
        let trail = TilePos::new(*coord, summit_level);
        let clearance = clearance_at(trail);
        if !has_surface(trail)
            || material_at(trail) != Some(expected_summit_material)
            || clearance.is_none_or(|clear| clear < 8)
        {
            return Err(schematic_contract(format!(
                "composite Crystal summit trail is absent, obstructed, or changed from its expected {expected_summit_material:?} surface at {trail:?}: clearance={clearance:?}, material={:?}",
                material_at(trail)
            )));
        }
    }
    Ok(())
}

/// Proves that one architecturally authored shell cap is still present but is
/// no longer visible from above: a bounded Stone shoulder and one Snow cap
/// occupy the complete interval above it, and every added voxel participates in
/// the same review cutaway as the original shell.
fn validate_crystal_shell_overburden_column(
    world: &GeneratedWorldPlan,
    coord: HexCoord,
    expected_thickness: Level,
) -> Result<(), V3GenerationError> {
    let authored_shell = world
        .structures
        .by_id
        .values()
        .filter(|structure| structure.kind == StructureKind::Wall)
        .flat_map(|structure| structure.voxels.iter().copied())
        .filter(|position| position.coord == coord)
        .max_by_key(|position| position.level)
        .ok_or_else(|| {
            schematic_contract(format!(
                "composite Crystal overburden has no buried authored shell at {coord:?}"
            ))
        })?;
    let natural_surface = top_surface(world, coord).ok_or_else(|| {
        schematic_contract(format!(
            "composite Crystal overburden has no exposed natural surface at {coord:?}"
        ))
    })?;
    if natural_surface.level != authored_shell.level.saturating_add(expected_thickness) {
        return Err(schematic_contract(format!(
            "composite Crystal overburden at {coord:?} has thickness {}, expected {expected_thickness}",
            natural_surface.level.saturating_sub(authored_shell.level)
        )));
    }
    if world.volume.surfaces.contains_key(&authored_shell) {
        return Err(schematic_contract(format!(
            "composite Crystal authored shell remains exposed beneath overburden at {authored_shell:?}"
        )));
    }
    let buried_mass = solid_mass_at_surface(&world.volume, authored_shell).ok_or_else(|| {
        schematic_contract(format!(
            "composite Crystal overburden removed authored shell voxel {authored_shell:?}"
        ))
    })?;
    let Some(cutaway) = buried_mass.cutaway_for else {
        return Err(schematic_contract(format!(
            "composite Crystal buried shell has no cutaway owner at {authored_shell:?}"
        )));
    };
    if buried_mass.material != SolidMaterialRole::WorkedStone {
        return Err(schematic_contract(format!(
            "composite Crystal overburden changed authored shell material at {authored_shell:?}"
        )));
    }
    let interior = world.interiors.by_id.get(&cutaway).ok_or_else(|| {
        schematic_contract(format!(
            "composite Crystal overburden references missing cutaway {cutaway:?}"
        ))
    })?;
    for level in authored_shell.level.saturating_add(1)..natural_surface.level {
        let position = TilePos::new(coord, level);
        let mass = solid_mass_at_surface(&world.volume, position).ok_or_else(|| {
            schematic_contract(format!(
                "composite Crystal natural shoulder has an air gap at {position:?}"
            ))
        })?;
        if mass.material != SolidMaterialRole::Stone
            || mass.cutaway_for != Some(cutaway)
            || !interior.roof_voxels.contains(&position)
        {
            return Err(schematic_contract(format!(
                "composite Crystal natural shoulder is invalid at {position:?}: material={:?}, cutaway={:?}, roof-owned={} ",
                mass.material,
                mass.cutaway_for,
                interior.roof_voxels.contains(&position)
            )));
        }
    }
    let snow = solid_mass_at_surface(&world.volume, natural_surface).ok_or_else(|| {
        schematic_contract(format!(
            "composite Crystal natural shoulder has no Snow cap at {natural_surface:?}"
        ))
    })?;
    if snow.material != SolidMaterialRole::Snow
        || snow.cutaway_for != Some(cutaway)
        || !interior.roof_voxels.contains(&natural_surface)
        || world.structures.by_id.values().any(|structure| {
            structure.kind == StructureKind::Wall && structure.voxels.contains(&natural_surface)
        })
    {
        return Err(schematic_contract(format!(
            "composite Crystal Snow cap is not natural cutaway-owned overburden at {natural_surface:?}"
        )));
    }
    Ok(())
}

fn fine_components(mask: &BTreeSet<HexCoord>) -> Vec<BTreeSet<HexCoord>> {
    let mut remaining = mask.clone();
    let mut components = Vec::new();
    while let Some(start) = remaining.pop_first() {
        let mut component = BTreeSet::from([start]);
        let mut queue = VecDeque::from([start]);
        while let Some(current) = queue.pop_front() {
            for neighbor in current.neighbors() {
                if remaining.remove(&neighbor) {
                    component.insert(neighbor);
                    queue.push_back(neighbor);
                }
            }
        }
        components.push(component);
    }
    components.sort_by_key(|component| component.first().copied());
    components
}

fn connected(mask: &BTreeSet<HexCoord>) -> bool {
    let Some(start) = mask.first().copied() else {
        return false;
    };
    let mut reached = BTreeSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(current) = queue.pop_front() {
        for neighbor in current.neighbors() {
            if mask.contains(&neighbor) && reached.insert(neighbor) {
                queue.push_back(neighbor);
            }
        }
    }
    reached.len() == mask.len()
}

trait DistanceToSet {
    fn distance_to_set(&self, set: &BTreeSet<HexCoord>) -> Option<u32>;
}

impl DistanceToSet for HexCoord {
    fn distance_to_set(&self, set: &BTreeSet<HexCoord>) -> Option<u32> {
        set.iter().map(|coord| self.distance(*coord)).min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_175_final_visual_massif_authority_preserves_split_semantic_ownership() {
        let template = hex_schematic::grand_v3_reference_template().expect("template parses");
        let plan = hex_schematic::generate(&template, 175)
            .expect("seed 175 schematic generates")
            .plan;
        let settings = ProceduralV3Settings {
            layout: V3LayoutSettings::Schematic(V3SchematicLayoutSettings {
                template: V3SchematicTemplate::GrandV3,
                template_revision: crate::settings::V3_GRAND_V3_TEMPLATE_REVISION,
                cell_pitch: 22,
                terrain_profile: V3SchematicTerrainProfile::GrandV3BasicV1(
                    V3GrandV3BasicTerrainProfile::canonical(),
                ),
            }),
        };
        let mut layout =
            resolve_layout(V3_SCHEMATIC_GRID_RADIUS, &settings).expect("seed 175 layout resolves");
        super::super::super::schematic_crystal::claim_site(&plan, &mut layout, 22)
            .expect("seed 175 Crystal site claim validates");
        let crystal_patch = plan
            .cells
            .iter()
            .find(|cell| {
                cell.facts
                    .overlays
                    .contains(&SchematicFeature::CrystalAscent)
            })
            .map(|cell| PatchId(u32::from(cell.id.get())))
            .expect("seed 175 retains one Crystal cell");
        let crystal_mask = layout.patches[&crystal_patch].mask.clone();
        let semantic_massif_mask = plan
            .cells
            .iter()
            .filter(|cell| cell.facts.landform == LandformKind::Massif)
            .flat_map(|cell| {
                layout.patches[&PatchId(u32::from(cell.id.get()))]
                    .mask
                    .iter()
                    .copied()
            })
            .collect::<BTreeSet<_>>();
        assert!(!connected(&semantic_massif_mask));
        let field = super::super::super::schematic_highlands::GrandHighlandField::build(
            &plan,
            &layout,
            V3GrandV3BasicTerrainProfile::canonical(),
        )
        .expect("seed 175 visual Massif field builds");
        let authority = field.massif_visual_authority().clone();
        validate_massif_visual_authority(
            &plan,
            &layout,
            &crystal_mask,
            &semantic_massif_mask,
            &authority,
        )
        .expect("split semantic ownership is valid under the exact visual authority");

        let (connector, owner) = authority
            .connector_owners
            .first_key_value()
            .map(|(coord, owner)| (*coord, *owner))
            .expect("seed 175 exercises one Mountain connector");
        let mut changed_layout = layout.clone();
        assert!(changed_layout
            .patches
            .get_mut(&owner)
            .expect("captured connector owner remains present")
            .mask
            .remove(&connector));
        let error = validate_massif_visual_authority(
            &plan,
            &changed_layout,
            &crystal_mask,
            &semantic_massif_mask,
            &authority,
        )
        .expect_err("changing one connector owner must fail final validation");
        assert!(error.to_string().contains("visual connector"));
    }

    #[test]
    fn high_snow_validation_catches_every_unsnowed_recolourable_natural_cap() {
        for material in [
            SolidMaterialRole::Dirt,
            SolidMaterialRole::Grass,
            SolidMaterialRole::Gravel,
            SolidMaterialRole::Sand,
        ] {
            assert!(
                high_cap_violates_snow_contract(Some(material), false, false, false),
                "{material:?} must not escape merely because its coordinate is a route or Crystal site"
            );
        }
        for material in [
            SolidMaterialRole::Snow,
            SolidMaterialRole::Stone,
            SolidMaterialRole::WorkedStone,
            SolidMaterialRole::Metal,
            SolidMaterialRole::Ice,
            SolidMaterialRole::Basalt,
            SolidMaterialRole::Bedrock,
        ] {
            assert!(
                !high_cap_violates_snow_contract(Some(material), false, false, false),
                "authored or already-snowy cap {material:?} must remain a valid exception"
            );
        }
        assert!(!high_cap_violates_snow_contract(None, false, false, false));
    }

    #[test]
    fn high_snow_validation_preserves_only_water_and_the_magical_garden() {
        assert!(!high_cap_violates_snow_contract(
            Some(SolidMaterialRole::Sand),
            true,
            false,
            false,
        ));
        assert!(!high_cap_violates_snow_contract(
            Some(SolidMaterialRole::Grass),
            false,
            true,
            false,
        ));
        assert!(high_cap_violates_snow_contract(
            Some(SolidMaterialRole::Grass),
            false,
            false,
            false,
        ));
    }

    #[test]
    fn lower_stacked_water_does_not_exempt_a_high_natural_cap() {
        assert!(!fill_covers_surface(Some(12), 150));
        assert!(high_cap_violates_snow_contract(
            Some(SolidMaterialRole::Grass),
            fill_covers_surface(Some(12), 150),
            false,
            false,
        ));
        assert!(fill_covers_surface(Some(150), 150));
        assert!(!high_cap_violates_snow_contract(
            Some(SolidMaterialRole::Grass),
            fill_covers_surface(Some(150), 150),
            false,
            false,
        ));
    }

    #[test]
    fn summit_snow_rejects_exposed_rock_but_preserves_the_garden_exception() {
        for material in [
            SolidMaterialRole::Stone,
            SolidMaterialRole::WorkedStone,
            SolidMaterialRole::Grass,
        ] {
            assert!(high_cap_violates_snow_contract(
                Some(material),
                false,
                false,
                true,
            ));
        }
        assert!(!high_cap_violates_snow_contract(
            Some(SolidMaterialRole::Snow),
            false,
            false,
            true,
        ));
        assert!(!high_cap_violates_snow_contract(
            Some(SolidMaterialRole::Grass),
            false,
            true,
            true,
        ));
    }

    #[test]
    fn woodland_gradient_uses_ordered_coverage_bands_not_exact_tree_counts() {
        let valley = WoodlandCoverageBand {
            admitted_columns: 1_000,
            weighted_canopy_percent: 30_000,
            final_tree_roots: 19,
        };
        let hill = WoodlandCoverageBand {
            admitted_columns: 1_000,
            weighted_canopy_percent: 9_000,
            final_tree_roots: 31,
        };
        let mountain = WoodlandCoverageBand {
            admitted_columns: 1_000,
            weighted_canopy_percent: 4_000,
            final_tree_roots: 7,
        };
        validate_woodland_coverage_order(valley, hill, mountain).expect(
            "different tree silhouettes may change counts while the coverage bands remain ordered",
        );

        let flat_hill = WoodlandCoverageBand {
            weighted_canopy_percent: 4_000,
            ..hill
        };
        assert!(validate_woodland_coverage_order(valley, flat_hill, mountain).is_err());
        assert!(validate_woodland_coverage_order(
            WoodlandCoverageBand {
                final_tree_roots: 0,
                ..valley
            },
            hill,
            mountain,
        )
        .is_err());
    }

    #[test]
    fn outer_highland_boundary_rejects_the_old_hard_clipped_wall() {
        assert!(exceeds_upward_boundary_protrusion(168, 132, 12));
        assert!(!exceeds_upward_boundary_protrusion(144, 132, 12));
        assert!(!exceeds_upward_boundary_protrusion(120, 132, 12));

        let isolated_natural_cliff = [HexCoord::from_axial(0, 0), HexCoord::from_axial(1, 0)]
            .into_iter()
            .collect::<BTreeSet<_>>();
        assert!(!retains_connected_boundary_wall(
            &isolated_natural_cliff,
            100,
            13,
        ));

        let old_clipped_wall = (0..=2)
            .map(|q| HexCoord::from_axial(q, 0))
            .collect::<BTreeSet<_>>();
        assert!(retains_connected_boundary_wall(&old_clipped_wall, 500, 13,));

        let repeated_sawtooth = (0_i32..=5)
            .map(|q| HexCoord::from_axial(q.saturating_mul(3), 0))
            .collect::<BTreeSet<_>>();
        assert!(retains_connected_boundary_wall(&repeated_sawtooth, 500, 13,));

        let isolated_sheer_cliff = BTreeSet::from([HexCoord::ORIGIN]);
        assert!(retains_connected_boundary_wall(
            &isolated_sheer_cliff,
            500,
            25,
        ));
    }

    #[test]
    fn exact_massif_scenic_cliff_admission_cannot_be_reused_after_a_level_change() {
        let first = TilePos::new(HexCoord::ORIGIN, 236);
        let second = TilePos::new(HexCoord::from_axial(1, 0), 211);
        let admitted = BTreeSet::from([(first, second)]);
        assert!(admits_exact_scenic_cliff(&admitted, first, second));
        assert!(admits_exact_scenic_cliff(&admitted, second, first));
        assert!(!admits_exact_scenic_cliff(
            &admitted,
            TilePos::new(first.coord, first.level.saturating_add(1)),
            second,
        ));
    }

    #[test]
    fn mantle_outer_boundary_ignores_the_authored_crystal_hole() {
        let crystal = BTreeSet::from([HexCoord::ORIGIN]);
        let support = HexCoord::ORIGIN
            .within_radius(2)
            .into_iter()
            .filter(|coord| !crystal.contains(coord))
            .collect::<BTreeSet<_>>();
        let without_hole = true_outer_boundary_edges(&support, &BTreeSet::new());
        assert!(without_hole
            .values()
            .any(|outside| outside.contains(&HexCoord::ORIGIN)));

        let with_hole = true_outer_boundary_edges(&support, &crystal);
        assert!(with_hole
            .values()
            .all(|outside| !outside.is_empty() && !outside.contains(&HexCoord::ORIGIN)));
        assert!(HexCoord::ORIGIN
            .neighbors()
            .into_iter()
            .all(|inner| !with_hole.contains_key(&inner)));
        assert!(support
            .iter()
            .filter(|coord| coord.distance(HexCoord::ORIGIN) == 2)
            .all(|outer| with_hole.contains_key(outer)));

        let gap = HexCoord::ORIGIN.neighbors()[0];
        let mut gapped_support = support;
        gapped_support.remove(&gap);
        let with_non_crystal_gap = true_outer_boundary_edges(&gapped_support, &crystal);
        assert!(with_non_crystal_gap
            .values()
            .any(|outside| outside.contains(&gap)));
    }

    #[test]
    fn massif_requires_a_strong_majority_above_crystal_not_the_old_two_fifths() {
        assert!(!has_strong_highland_majority(40, 100));
        assert!(!has_strong_highland_majority(54, 100));
        assert!(has_strong_highland_majority(55, 100));
    }

    #[test]
    fn complete_massif_radial_allows_rugged_cliffs_but_rejects_a_collapsed_profile() {
        let natural = (0_i32..40)
            .map(|distance| 350_i32.saturating_sub(distance.saturating_mul(4)))
            .collect::<Vec<_>>();
        validate_complete_massif_radial(0, &natural)
            .expect("a long continuously descending shoulder validates");

        let collapsed = vec![200; natural.len()];
        assert!(validate_complete_massif_radial(1, &collapsed).is_err());

        let mut cliff = natural.clone();
        if let Some(level) = cliff.get_mut(18) {
            *level = level.saturating_sub(20);
        }
        validate_complete_massif_radial(2, &cliff)
            .expect("an otherwise broad inward-rising Massif radial may contain a natural cliff");

        let mut zigzag = natural;
        if let Some(level) = zigzag.get_mut(20) {
            *level = level.saturating_add(8);
        }
        validate_complete_massif_radial(3, &zigzag)
            .expect("a broad Massif radial may include a local outward reversal");
    }

    #[test]
    fn lower_visual_peak_band_rejects_a_190_level_connecting_wall() {
        let pins = (0_i32..6)
            .map(|index| (HexCoord::from_axial(index.saturating_mul(10), 0), 220))
            .collect::<BTreeMap<_, _>>();
        let separated_band = pins.keys().copied().collect::<BTreeSet<_>>();
        let separated = fine_components(&separated_band);
        assert!(has_six_independent_peak_bodies(&separated, &pins));

        let first = HexCoord::from_axial(0, 0);
        let second = HexCoord::from_axial(10, 0);
        let mut walled_band = separated_band;
        walled_band.extend(first.line_between(second));
        let walled = fine_components(&walled_band);
        assert!(!has_six_independent_peak_bodies(&walled, &pins));
    }

    #[test]
    fn peak_silhouette_extents_ignore_the_low_semantic_patch_fringe() {
        let summit = HexCoord::ORIGIN;
        let mut body = summit
            .within_radius(4)
            .into_iter()
            .map(|coord| {
                let level = 300_i32.saturating_sub(
                    i32::try_from(summit.distance(coord))
                        .unwrap_or(i32::MAX)
                        .saturating_mul(8),
                );
                (coord, level)
            })
            .collect::<BTreeMap<_, _>>();
        let silhouette = peak_silhouette_band(&body);
        let expected = peak_directional_extents(summit, &silhouette);
        let low_patch_fringe = HexCoord::from_axial(18, 0);
        body.insert(low_patch_fringe, 150);
        let with_fringe = peak_directional_extents(summit, &peak_silhouette_band(&body));
        assert!(!silhouette.contains(&low_patch_fringe));
        assert_eq!(with_fringe, expected);
        assert!(with_fringe.iter().all(|extent| *extent <= 4));
    }

    #[test]
    fn peak_feather_edge_validation_rejects_one_reintroduced_outer_cliff() {
        let inside = HexCoord::ORIGIN;
        let outside = HexCoord::from_axial(1, 0);
        let edges = BTreeSet::from([(inside, outside)]);
        validate_peak_feather_edge_levels(0, &edges, |_| Some(180))
            .expect("a level Mountain feather edge validates");

        let error = validate_peak_feather_edge_levels(0, &edges, |coord| {
            Some(if coord == inside { 180 } else { 170 })
        })
        .expect_err("a ten-level mutation must recreate a rejected outer cliff");
        assert!(error.to_string().contains("Mountain feather forms a cliff"));
    }

    #[test]
    fn peak_feather_validation_rejects_one_reintroduced_internal_cliff() {
        let first = HexCoord::ORIGIN;
        let second = HexCoord::from_axial(1, 0);
        let third = HexCoord::from_axial(2, 0);
        let feather = BTreeSet::from([first, second, third]);
        validate_peak_feather_internal_levels(0, &feather, |_| Some(180))
            .expect("a level internal Mountain feather validates");

        let error = validate_peak_feather_internal_levels(0, &feather, |coord| {
            Some(if coord == second { 170 } else { 180 })
        })
        .expect_err("a ten-level internal-feather mutation must fail");
        assert!(error.to_string().contains("internal cliff"));
    }

    #[test]
    fn crystal_opening_validation_rejects_a_lower_aperture_cap() {
        let lower_coord = HexCoord::ORIGIN;
        let upper_coord = HexCoord::from_axial(8, 0);
        let lower = BTreeSet::from([lower_coord]);
        let lower_headrooms = BTreeMap::from([(lower_coord, 12)]);
        let upper = BTreeSet::from([upper_coord]);
        let lower_floor = TilePos::new(lower_coord, 6);
        let lower_cap = TilePos::new(lower_coord, 19);
        let upper_trail = TilePos::new(upper_coord, 150);
        let surfaces = BTreeSet::from([lower_floor, upper_trail]);
        let materials = BTreeMap::from([
            (lower_floor, SolidMaterialRole::WorkedStone),
            (lower_cap, SolidMaterialRole::WorkedStone),
            (upper_trail, SolidMaterialRole::Snow),
        ]);
        let valid_headrooms = BTreeMap::from([(lower_floor, 12), (upper_trail, 8)]);
        validate_crystal_opening_geometry(
            &lower,
            &lower_headrooms,
            &upper,
            6,
            150,
            SolidMaterialRole::Snow,
            |surface| surfaces.contains(&surface),
            |position| materials.get(&position).copied(),
            |surface| valid_headrooms.get(&surface).copied(),
        )
        .expect("the exact lower aperture and summit opening validate");

        let capped_headrooms = BTreeMap::from([(lower_floor, 4), (upper_trail, 8)]);
        let error = validate_crystal_opening_geometry(
            &lower,
            &lower_headrooms,
            &upper,
            6,
            150,
            SolidMaterialRole::Snow,
            |surface| surfaces.contains(&surface),
            |position| materials.get(&position).copied(),
            |surface| capped_headrooms.get(&surface).copied(),
        )
        .expect_err("a lower-aperture cap inside the pointed arch must fail");
        assert!(error.to_string().contains("lower aperture is capped"));

        let wrong_cap_materials = BTreeMap::from([
            (lower_floor, SolidMaterialRole::WorkedStone),
            (lower_cap, SolidMaterialRole::Stone),
            (upper_trail, SolidMaterialRole::Snow),
        ]);
        let error = validate_crystal_opening_geometry(
            &lower,
            &lower_headrooms,
            &upper,
            6,
            150,
            SolidMaterialRole::Snow,
            |surface| surfaces.contains(&surface),
            |position| wrong_cap_materials.get(&position).copied(),
            |surface| valid_headrooms.get(&surface).copied(),
        )
        .expect_err("a lower-aperture cap with the wrong material must fail independently");
        assert!(error.to_string().contains("lower aperture is capped"));
    }

    #[test]
    fn crystal_opening_validation_rejects_an_unsnowed_grand_summit() {
        let lower_coord = HexCoord::ORIGIN;
        let upper_coord = HexCoord::from_axial(8, 0);
        let lower = BTreeSet::from([lower_coord]);
        let lower_headrooms = BTreeMap::from([(lower_coord, 12)]);
        let upper = BTreeSet::from([upper_coord]);
        let lower_floor = TilePos::new(lower_coord, 6);
        let lower_cap = TilePos::new(lower_coord, 19);
        let upper_trail = TilePos::new(upper_coord, 150);
        let surfaces = BTreeSet::from([lower_floor, upper_trail]);
        let materials = BTreeMap::from([
            (lower_floor, SolidMaterialRole::WorkedStone),
            (lower_cap, SolidMaterialRole::WorkedStone),
            (upper_trail, SolidMaterialRole::Grass),
        ]);
        let headrooms = BTreeMap::from([(lower_floor, 12), (upper_trail, 8)]);

        let error = validate_crystal_opening_geometry(
            &lower,
            &lower_headrooms,
            &upper,
            6,
            150,
            SolidMaterialRole::Snow,
            |surface| surfaces.contains(&surface),
            |position| materials.get(&position).copied(),
            |surface| headrooms.get(&surface).copied(),
        )
        .expect_err("Grand ecology must snow-cap the otherwise exact summit trail");
        assert!(error.to_string().contains("expected Snow surface"));
    }

    #[test]
    fn crystal_opening_validation_rejects_a_summit_fill() {
        let lower_coord = HexCoord::ORIGIN;
        let upper_coord = HexCoord::from_axial(8, 0);
        let lower = BTreeSet::from([lower_coord]);
        let lower_headrooms = BTreeMap::from([(lower_coord, 12)]);
        let upper = BTreeSet::from([upper_coord]);
        let lower_floor = TilePos::new(lower_coord, 6);
        let lower_cap = TilePos::new(lower_coord, 19);
        let upper_trail = TilePos::new(upper_coord, 150);
        let filled_top = TilePos::new(upper_coord, 151);
        let surfaces = BTreeSet::from([lower_floor, upper_trail, filled_top]);
        let materials = BTreeMap::from([
            (lower_floor, SolidMaterialRole::WorkedStone),
            (lower_cap, SolidMaterialRole::WorkedStone),
            (upper_trail, SolidMaterialRole::Snow),
            (filled_top, SolidMaterialRole::Stone),
        ]);
        let headrooms = BTreeMap::from([(lower_floor, 12), (upper_trail, 0)]);

        let error = validate_crystal_opening_geometry(
            &lower,
            &lower_headrooms,
            &upper,
            6,
            150,
            SolidMaterialRole::Snow,
            |surface| surfaces.contains(&surface),
            |position| materials.get(&position).copied(),
            |surface| headrooms.get(&surface).copied(),
        )
        .expect_err("a filled summit opening must fail");
        assert!(error
            .to_string()
            .contains("summit trail is absent, obstructed"));
    }

    #[test]
    fn frozen_halo_rejects_one_partial_directional_tendril() {
        fn ray(direction: usize, depth: u32) -> BTreeSet<HexCoord> {
            let mut current = HexCoord::ORIGIN;
            let mut result = BTreeSet::new();
            for _ in 0..depth {
                current = current.neighbors()[direction];
                result.insert(current);
            }
            result
        }

        let frozen = BTreeSet::from([HexCoord::ORIGIN]);
        let complete = ray(0, FROZEN_PLATEAU_HALO_DEPTH)
            .union(&ray(3, FROZEN_PLATEAU_HALO_DEPTH))
            .copied()
            .collect::<BTreeSet<_>>();
        let mut complete_distances = BTreeMap::from([(HexCoord::ORIGIN, 0)]);
        complete_distances.extend(
            complete
                .iter()
                .copied()
                .map(|coord| (coord, HexCoord::ORIGIN.distance(coord))),
        );
        validate_frozen_halo_coverage(&frozen, &complete, &complete_distances)
            .expect("two complete directional halo tendrils validate");

        let partial = ray(0, FROZEN_PLATEAU_HALO_DEPTH)
            .union(&ray(3, 3))
            .copied()
            .collect::<BTreeSet<_>>();
        let mut partial_distances = BTreeMap::from([(HexCoord::ORIGIN, 0)]);
        partial_distances.extend(
            partial
                .iter()
                .copied()
                .map(|coord| (coord, HexCoord::ORIGIN.distance(coord))),
        );
        let error = validate_frozen_halo_coverage(&frozen, &partial, &partial_distances)
            .expect_err("one three-row tendril must not hide behind another complete sector");
        assert!(error.to_string().contains("stops before row 6"));
    }
}
