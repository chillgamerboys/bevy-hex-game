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
const PEAK_SUMMIT_MIN: Level = 200;
const PEAK_SUMMIT_MAX: Level = 218;
const MASSIF_SUMMIT_MIN: Level = 224;
const MASSIF_SUMMIT_MAX: Level = 236;
const GARDEN_STRUCTURE_ID: StructureId = StructureId(WORLD_NAMESPACE | 0x0005_0000);

pub(super) struct CorrectiveWorldValidation<'a> {
    pub(super) hydrology: &'a HydrologyCompilation,
    pub(super) crystal_mask: &'a BTreeSet<HexCoord>,
    pub(super) crystal_rotation: u8,
    pub(super) fine_index: &'a FineWorldIndex,
    pub(super) reachable: &'a BTreeMap<TilePos, u32>,
    pub(super) massif_visual: &'a super::super::schematic_highlands::MassifVisualAuthority,
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
    validate_crystal_mantle(
        world,
        profile,
        validation.crystal_mask,
        validation.crystal_rotation,
    )?;
    validate_highland_hierarchy(
        plan,
        world,
        validation.crystal_mask,
        validation.massif_visual,
    )?;
    validate_peak_ridge_authority(world, validation.peak_ridges)?;
    validate_frozen_exit(plan, world, validation.crystal_mask)?;
    validate_concealed_tunnel(world, profile)?;
    validate_tunnel_overburden_authority(
        plan,
        world,
        validation.fine_index,
        validation.tunnel_overburden,
    )?;
    validate_garden_island(plan, world)?;
    validate_vegetation_gradient(plan, world)?;
    validate_certain_snow_caps(plan, world, validation.crystal_mask)?;
    validate_waterfall_and_review_anchor(world, profile, validation.hydrology)?;
    validate_river_and_review_anchor(plan, world, validation.hydrology)?;
    validate_semantic_review_anchors(plan, world, &validation)?;
    Ok(())
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

fn validate_crystal_mantle(
    world: &GeneratedWorldPlan,
    profile: V3GrandV3BasicTerrainProfile,
    crystal_mask: &BTreeSet<HexCoord>,
    crystal_rotation: u8,
) -> Result<(), V3GenerationError> {
    let required = super::super::schematic_highlands::crystal_mantle_inner_screen(
        crystal_mask,
        crystal_rotation,
        profile,
        &world.layout.footprint,
    )?;
    let site_radius = super::super::schematic_highlands::CRYSTAL_SITE_RADIUS;
    let complete_inner_ring_count = usize::try_from(
        6_u32
            .saturating_mul(site_radius.saturating_add(1))
            .saturating_add(6_u32.saturating_mul(site_radius.saturating_add(2))),
    )
    .unwrap_or(usize::MAX);
    if required.len().saturating_mul(2) < complete_inner_ring_count {
        return Err(schematic_contract(format!(
            "Crystal mantle apertures consume too much of its inner screen: required={}/{}",
            required.len(),
            complete_inner_ring_count
        )));
    }
    let low = required
        .iter()
        .filter_map(|coord| {
            let level = top_surface(world, *coord).map(|surface| surface.level);
            level
                .is_none_or(|level| {
                    level <= super::super::schematic_highlands::CRYSTAL_ARCHITECTURE_TOP
                })
                .then_some((*coord, level))
        })
        .collect::<Vec<_>>();
    if !low.is_empty() {
        return Err(schematic_contract(format!(
            "Crystal mantle leaves {} low inner-screen columns outside its exact apertures; first={:?}",
            low.len(),
            low.first()
        )));
    }
    Ok(())
}

fn validate_highland_hierarchy(
    plan: &SchematicPlanV1,
    world: &GeneratedWorldPlan,
    crystal_mask: &BTreeSet<HexCoord>,
    massif_visual: &super::super::schematic_highlands::MassifVisualAuthority,
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
            "massif crest {crest:?} is outside its mask or 224..=236 summit band"
        )));
    }
    let massif_levels = massif_mask
        .iter()
        .filter_map(|coord| top_surface(world, *coord))
        .collect::<Vec<_>>();
    validate_unique_massif_crest(crest, &massif_levels)?;
    let depths = boundary_depth(massif_mask);
    let crest_depth = depths.get(&crest.coord).copied().unwrap_or_default();
    let crystal_cell = overlay_cell(plan, SchematicFeature::CrystalAscent)?;
    let eligible_massif_patches = plan
        .cells
        .iter()
        .filter(|cell| {
            cell.facts.landform == LandformKind::Massif
                && cell
                    .coord
                    .checked_distance(crystal_cell.coord)
                    .is_some_and(|distance| distance >= 2)
        })
        .map(|cell| PatchId(u32::from(cell.id.get())))
        .collect::<BTreeSet<_>>();
    let eligible_massif_mask = union_patch_masks(world, &eligible_massif_patches)?;
    let maximum_eligible_depth = eligible_massif_mask
        .iter()
        .filter(|coord| {
            coord
                .distance(crystal_center)
                .saturating_sub(crystal_site_radius)
                >= CELL_PITCH.unsigned_abs() / 2
        })
        .filter_map(|coord| depths.get(coord).copied())
        .max()
        .unwrap_or_default();
    if crest_depth != maximum_eligible_depth {
        return Err(schematic_contract(format!(
            "massif crest is not centered in its separated eligible body: depth={crest_depth}, maximum={maximum_eligible_depth}"
        )));
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
    let crystal_top = crystal_mask
        .iter()
        .filter_map(|coord| top_surface(world, *coord).map(|surface| surface.level))
        .max()
        .ok_or_else(|| schematic_contract("corrective Crystal site has no final surfaces"))?;
    if !(PEAK_SUMMIT_MIN..=PEAK_SUMMIT_MAX).contains(&highest_peak)
        || crest.level.saturating_sub(highest_peak) < 6
        || crest.level.saturating_sub(crystal_top) < 40
    {
        return Err(schematic_contract(format!(
            "highland hierarchy is too weak: Crystal={crystal_top}, peaks={highest_peak}, massif={}",
            crest.level
        )));
    }
    Ok(())
}

fn validate_unique_massif_crest(
    crest: TilePos,
    massif_surfaces: &[TilePos],
) -> Result<(), V3GenerationError> {
    let highest = massif_surfaces
        .iter()
        .map(|surface| surface.level)
        .max()
        .ok_or_else(|| schematic_contract("visual Massif has no final surfaces"))?;
    let maxima = massif_surfaces
        .iter()
        .copied()
        .filter(|surface| surface.level == highest)
        .collect::<Vec<_>>();
    if highest != crest.level || maxima != [crest] {
        return Err(schematic_contract(format!(
            "massif must retain one decisive authoritative crest; expected {crest:?}, highest={highest}, maxima={maxima:?}"
        )));
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
    let expected_visual = semantic_massif_mask
        .union(&connector_coords)
        .copied()
        .collect::<BTreeSet<_>>();
    if authority.semantic_owner_mask != *semantic_massif_mask
        || authority.visual_mask != expected_visual
        || !semantic_massif_mask.is_subset(&authority.visual_mask)
        || !connected(&authority.visual_mask)
        || !authority.visual_mask.is_disjoint(crystal_mask)
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
    Ok(())
}

/// Proves the final terrain still contains both seeded six-cell high ridges.
///
/// The natural pass, upper saddle, and lower foothill ledge may deliberately
/// grade PeakRing ownership. Their exact changed coordinates and levels are
/// sealed before generic route construction; final validation admits only that
/// immutable footprint, while preserving all twelve deterministic summit pins.
pub(super) fn validate_peak_ridge_authority(
    world: &GeneratedWorldPlan,
    authority: &super::super::schematic_highlands::PeakRidgeAuthority,
) -> Result<(), V3GenerationError> {
    if authority.components.len() != 2 {
        return Err(schematic_contract(format!(
            "final peak authority requires two ridge chains, found {}",
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
            || component.summit_pins.len() != 6
            || !connected(&expected_high_coords)
            || component
                .patch_masks
                .values()
                .any(|mask| mask.is_disjoint(&expected_high_coords))
        {
            return Err(schematic_contract(format!(
                "seeded peak ridge {component_index} has malformed six-cell authority"
            )));
        }
        let authorized_route_grades = component.authorized_route_grades.as_ref().ok_or_else(|| {
            schematic_contract(format!(
                "final peak ridge {component_index} reached validation before its exact route grades were sealed"
            ))
        })?;
        if let Some((coord, _)) = authorized_route_grades.iter().find(|(coord, level)| {
            component
                .expected_high_band
                .get(*coord)
                .is_none_or(|expected| expected == *level)
                || component.summit_pins.contains_key(*coord)
        }) {
            return Err(schematic_contract(format!(
                "final peak ridge {component_index} contains malformed route-grade authority at {coord:?}"
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
                "final peak ridge {component_index} added an unauthorized >=200 surface at {coord:?}"
            )));
        }
        let authored_low_bridges = authorized_route_grades
            .iter()
            .filter_map(|(coord, level)| (*level < PEAK_SUMMIT_MIN).then_some(*coord))
            .collect::<BTreeSet<_>>();
        let final_ridge_topology = final_high_band
            .union(&authored_low_bridges)
            .copied()
            .collect::<BTreeSet<_>>();
        if !connected(&final_ridge_topology) {
            return Err(schematic_contract(format!(
                "final peak ridge {component_index} is not one connected high topology after admitting exact authored low-route bridges"
            )));
        }
        if let Some((patch, _)) = component
            .patch_masks
            .iter()
            .find(|(_, mask)| mask.is_disjoint(&final_high_band))
        {
            return Err(schematic_contract(format!(
                "final peak ridge {component_index} no longer reaches locked patch {}",
                patch.0
            )));
        }

        if let Some((coord, expected)) = component.summit_pins.iter().find(|(coord, expected)| {
            top_surface(world, **coord).is_none_or(|surface| surface.level != **expected)
        }) {
            return Err(schematic_contract(format!(
                "final peak summit pin {coord:?} lost deterministic level {expected}"
            )));
        }

        if let Some((coord, authorized)) =
            component
                .expected_high_band
                .iter()
                .find_map(|(coord, expected)| {
                    let authorized = authorized_route_grades
                        .get(coord)
                        .copied()
                        .unwrap_or(*expected);
                    top_surface(world, *coord)
                        .is_none_or(|surface| surface.level != authorized)
                        .then_some((*coord, authorized))
                })
        {
            return Err(schematic_contract(format!(
                "final peak ridge {component_index} changed exact authorized level {authorized} at {coord:?}"
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
    }
    Ok(())
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
    // Route and Crystal membership are diagnostic facts, not exemptions. A
    // high exposed Grass/Gravel/Dirt/Sand cap is a visible green/brown stripe
    // even when it belongs to a protected path or the natural Crystal site.
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
    let drops = hydrology
        .waterfall_centerline
        .windows(2)
        .enumerate()
        .filter_map(|(index, pair)| (pair[0].level > pair[1].level).then_some((index, pair)))
        .collect::<Vec<_>>();
    let [(drop_index, drop)] = drops.as_slice() else {
        return Err(schematic_contract(format!(
            "corrective waterfall requires one true plunge, found {} descending transitions",
            drops.len()
        )));
    };
    let total_drop = profile
        .mountain_lake_level
        .saturating_sub(profile.valley_lake_level);
    let plunge = drop[0].level.saturating_sub(drop[1].level);
    if plunge.saturating_mul(10) < total_drop.saturating_mul(9) {
        return Err(schematic_contract(format!(
            "waterfall plunge {plunge} does not carry ninety percent of total drop {total_drop}"
        )));
    }
    let source_row = hydrology
        .watercourse_rows
        .get(*drop_index)
        .ok_or_else(|| schematic_contract("waterfall plunge has no exact source row"))?;
    let sink_row = hydrology
        .watercourse_rows
        .get(drop_index.saturating_add(1))
        .ok_or_else(|| schematic_contract("waterfall plunge has no exact sink row"))?;
    let nodes = world
        .liquids
        .bodies
        .values()
        .flat_map(|body| body.nodes.iter().map(|(position, node)| (*position, *node)))
        .collect::<BTreeMap<_, _>>();
    if source_row.len() != 3
        || source_row.iter().any(|source| {
            nodes.get(source).is_none_or(|node| {
                node.state != LiquidFlowState::Fall
                    || node
                        .downstream
                        .is_none_or(|target| !sink_row.contains(&target))
            })
        })
    {
        return Err(schematic_contract(
            "all three waterfall lanes must publish one exact Fall into the receiving row",
        ));
    }
    for (name, target, maximum_distance) in [
        (
            "grand_v3.waterfall_crown",
            hydrology.waterfall_centerline[0],
            12,
        ),
        (
            "grand_v3.waterfall_base",
            *hydrology
                .waterfall_centerline
                .last()
                .ok_or_else(|| schematic_contract("waterfall has no base"))?,
            12,
        ),
        ("grand_v3.waterfall_profile", drop[0], 12),
    ] {
        let anchor = world
            .anchors
            .get(name)
            .ok_or_else(|| schematic_contract(format!("waterfall review lost anchor {name}")))?;
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
    let foothill_route = world
        .features
        .protected_routes
        .get("grand_v3.peak_foothill_ledge")
        .ok_or_else(|| schematic_contract("corrective review lost its protected peak ledge"))?;
    if peak != validation.review.peak
        || foothill != validation.review.peak
        || !foothill_route.surfaces.contains(&peak)
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
    let waterfall_crown = validation
        .hydrology
        .waterfall_centerline
        .first()
        .copied()
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

fn boundary_depth(mask: &BTreeSet<HexCoord>) -> BTreeMap<HexCoord, u32> {
    let mut depths = BTreeMap::new();
    let mut queue = VecDeque::new();
    for coord in mask.iter().copied().filter(|coord| {
        coord
            .neighbors()
            .into_iter()
            .any(|neighbor| !mask.contains(&neighbor))
    }) {
        depths.insert(coord, 0_u32);
        queue.push_back(coord);
    }
    while let Some(current) = queue.pop_front() {
        let next = depths
            .get(&current)
            .copied()
            .unwrap_or_default()
            .saturating_add(1);
        for neighbor in current.neighbors() {
            if mask.contains(&neighbor) && !depths.contains_key(&neighbor) {
                depths.insert(neighbor, next);
                queue.push_back(neighbor);
            }
        }
    }
    depths
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
    fn unique_massif_crest_rejects_any_higher_visual_surface() {
        let crest = TilePos::new(HexCoord::ORIGIN, 230);
        let lower = TilePos::new(HexCoord::from_axial(1, 0), 226);
        validate_unique_massif_crest(crest, &[lower, crest])
            .expect("one authoritative highest crest is valid");

        let higher = TilePos::new(HexCoord::from_axial(2, 0), 231);
        let error = validate_unique_massif_crest(crest, &[lower, crest, higher])
            .expect_err("a visual-Massif surface above the crest must fail closed");
        assert!(error.to_string().contains("highest=231"));
    }

    #[test]
    fn seed_175_final_visual_massif_authority_preserves_split_semantic_ownership() {
        let template = hex_schematic::grand_v3_reference_template().expect("template parses");
        let plan = hex_schematic::generate(&template, 175)
            .expect("seed 175 schematic generates")
            .plan;
        let settings = ProceduralV3Settings {
            layout: V3LayoutSettings::Schematic(V3SchematicLayoutSettings {
                template: V3SchematicTemplate::GrandV3,
                template_revision: 2,
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
}
