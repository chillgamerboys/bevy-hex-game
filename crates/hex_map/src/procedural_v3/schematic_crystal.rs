//! Exact Crystal Ascent reuse inside the Grand V3 schematic world.
//!
//! The landmark remains owned by its established authored recipe. This adapter only
//! expands the locked schematic cell to the recipe's radius-32 site, resolves the
//! accepted art dependencies, and merges the namespaced fragment into the otherwise
//! continuous schematic terrain.

use std::cmp::Reverse;
use std::collections::{btree_map::Entry, BTreeMap, BTreeSet, VecDeque};

use hex_assets::RuntimeArtCatalog;
use hex_core::{BiomeRegionId, HexCoord, InteriorRegionId, Level, TilePos};
use hex_schematic::{FeatureKind as SchematicFeature, NetworkKind, SchematicPlanV1};

use super::composition::{GeneratedPatchPlan, WorldCompositionError};
use super::layout::{LayoutKind, PatchId, ResolvedLayoutPlan};
use super::patch::{PatchBuildMode, PatchRecipeContext};
use super::selection::WorldValidation;
use super::vegetation::TemperateTreeSet;
use super::volume::{
    LevelInterval, SolidMass, SolidMaterialRole, SurfaceMetadata, VolumeElement, VolumePlan,
};
use super::world::{GeneratedWorldPlan, InteriorPlan, StructurePlan, WorldValidationIssue};
use super::{CrystalAscentObjectSet, V3GenerationError};
use crate::settings::V3CrystalAscentSettings;

const CRYSTAL_SITE_RADIUS: u32 = 32;
const CRYSTAL_BASE_LEVEL: i32 = 6;
const CRYSTAL_RISE_LEVELS: i32 = 144;

/// Sealed evidence that the final claimed Schematic layout passed its strict
/// layout validator.
///
/// The field stays private to this module: callers can carry the admission into
/// Grand whole-world finalization, but cannot manufacture one for an arbitrary
/// [`ResolvedLayoutPlan`].
#[derive(Debug)]
pub(super) struct ClaimedSchematicLayoutAdmission {
    patch_id: PatchId,
    validated_layout: ResolvedLayoutPlan,
}

impl ClaimedSchematicLayoutAdmission {
    #[must_use]
    pub(super) const fn patch_id(&self) -> PatchId {
        self.patch_id
    }

    /// Whether the final world still owns the exact layout which passed claim
    /// validation.
    ///
    /// Grand construction may carry this admission through later compiler
    /// stages, but none of those stages are authorized to mutate layout
    /// topology. Structural equality keeps that promise fail-closed before the
    /// common validator reuses the earlier layout proof.
    #[must_use]
    pub(super) fn matches_final_layout(&self, layout: &ResolvedLayoutPlan) -> bool {
        self.validated_layout == *layout
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct CrystalSiteClaimStats {
    donor_patches: usize,
    disjoint_donors: usize,
    disjoint_donors_skipped: usize,
    intersecting_donors: usize,
    component_scans: usize,
    intersecting_donors_with_orphans: usize,
    orphan_components_found: usize,
    orphan_components_rehomed: usize,
}

/// Reassigns the exact authored radius-32 site to the locked Crystal Ascent cell.
///
/// Coarse biome identity remains the original cell id. Donor patches must stay
/// non-empty and connected; the complete strict layout validator proves that before
/// construction proceeds.
pub(super) fn claim_site(
    plan: &SchematicPlanV1,
    layout: &mut ResolvedLayoutPlan,
    pitch: i32,
) -> Result<ClaimedSchematicLayoutAdmission, V3GenerationError> {
    let (patch_id, _) = claim_site_with_stats(plan, layout, pitch, true)?;
    Ok(ClaimedSchematicLayoutAdmission {
        patch_id,
        validated_layout: layout.clone(),
    })
}

fn claim_site_with_stats(
    plan: &SchematicPlanV1,
    layout: &mut ResolvedLayoutPlan,
    pitch: i32,
    skip_disjoint_work: bool,
) -> Result<(PatchId, CrystalSiteClaimStats), V3GenerationError> {
    if layout.kind != LayoutKind::Schematic {
        return Err(contract("Crystal site claims require a Schematic layout"));
    }
    let matches = plan
        .cells
        .iter()
        .filter(|cell| {
            cell.facts
                .overlays
                .contains(&SchematicFeature::CrystalAscent)
        })
        .collect::<Vec<_>>();
    let [cell] = matches.as_slice() else {
        return Err(contract(format!(
            "Grand V3 requires exactly one Crystal Ascent cell, found {}",
            matches.len()
        )));
    };
    let patch_id = PatchId(u32::from(cell.id.get()));
    let center = HexCoord::from_axial(
        cell.coord.q().saturating_mul(pitch),
        cell.coord.r().saturating_mul(pitch),
    );
    let site = center
        .within_radius(CRYSTAL_SITE_RADIUS)
        .into_iter()
        .collect::<BTreeSet<_>>();
    if !site.is_subset(&layout.footprint) {
        return Err(contract(
            "locked Crystal Ascent radius-32 site leaves the Grand V3 footprint",
        ));
    }
    let original = layout
        .patches
        .get(&patch_id)
        .ok_or_else(|| contract("locked Crystal Ascent cell has no resolved patch"))?
        .mask
        .clone();
    if !original.is_subset(&site) {
        return Err(contract(
            "locked Crystal Ascent coarse mask is not contained by its exact radius-32 site",
        ));
    }

    let centers = plan
        .cells
        .iter()
        .map(|cell| {
            (
                PatchId(u32::from(cell.id.get())),
                HexCoord::from_axial(
                    cell.coord.q().saturating_mul(pitch),
                    cell.coord.r().saturating_mul(pitch),
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let landform_by_patch = plan
        .cells
        .iter()
        .map(|cell| (PatchId(u32::from(cell.id.get())), cell.facts.landform))
        .collect::<BTreeMap<_, _>>();
    let mut stats = CrystalSiteClaimStats::default();
    let mut orphans = Vec::new();
    for (donor_id, patch) in &mut layout.patches {
        if *donor_id == patch_id {
            continue;
        }
        stats.donor_patches += 1;
        if patch.mask.is_disjoint(&site) {
            stats.disjoint_donors += 1;
            if skip_disjoint_work {
                stats.disjoint_donors_skipped += 1;
                continue;
            }
        } else {
            stats.intersecting_donors += 1;
        }
        patch.mask.retain(|coord| !site.contains(coord));
        stats.component_scans += 1;
        let mut components = connected_components(&patch.mask);
        if components.is_empty() {
            return Err(contract(format!(
                "Crystal radius-32 claim consumed donor patch {}",
                donor_id.0
            )));
        }
        let preferred = centers
            .get(donor_id)
            .and_then(|center| {
                components
                    .iter()
                    .position(|component| component.contains(center))
            })
            .unwrap_or_else(|| {
                components
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, component)| {
                        (Reverse(component.len()), component.first().copied())
                    })
                    .map(|(index, _)| index)
                    .unwrap_or_default()
            });
        let keep = components.remove(preferred);
        patch.mask = keep;
        if !components.is_empty() {
            stats.intersecting_donors_with_orphans += 1;
            stats.orphan_components_found += components.len();
        }
        orphans.extend(
            components
                .into_iter()
                .map(|component| (*donor_id, component)),
        );
    }
    stats.orphan_components_rehomed =
        rehome_orphan_components(layout, patch_id, &landform_by_patch, orphans)?;
    layout
        .patches
        .get_mut(&patch_id)
        .ok_or_else(|| contract("locked Crystal Ascent cell disappeared during claim"))?
        .mask = site.clone();
    let tunnel_target = plan
        .networks
        .iter()
        .find(|network| network.kind == NetworkKind::Tunnel)
        .and_then(|network| {
            network
                .edges
                .iter()
                .find(|edge| edge.id.as_str() == "edge/tunnel-complete")
        })
        .and_then(|edge| {
            edge.path.windows(2).find_map(|pair| {
                (pair[0] == cell.coord)
                    .then_some(pair[1])
                    .or_else(|| (pair[1] == cell.coord).then_some(pair[0]))
            })
        })
        .map(|coord| {
            HexCoord::from_axial(
                coord.q().saturating_mul(pitch),
                coord.r().saturating_mul(pitch),
            )
        })
        .ok_or_else(|| contract("Crystal Ascent has no adjacent locked tunnel node"))?;
    let frozen_patch_ids = plan
        .cells
        .iter()
        .filter(|candidate| {
            candidate
                .facts
                .overlays
                .contains(&SchematicFeature::FrozenWoods)
        })
        .map(|candidate| PatchId(u32::from(candidate.id.get())))
        .collect::<BTreeSet<_>>();
    let frozen_mask = frozen_patch_ids
        .iter()
        .filter_map(|patch_id| layout.patches.get(patch_id))
        .flat_map(|patch| patch.mask.iter().copied())
        .collect::<BTreeSet<_>>();
    if frozen_mask.is_empty() {
        return Err(contract(
            "Crystal Ascent has no remaining Frozen Woods summit destination",
        ));
    }
    let rotation = (0_u8..6)
        .filter_map(|turns| {
            let upper_outward = super::crystal_ascent::macro_upper_terminal_outward_rows(
                &layout.patches.get(&patch_id)?.mask,
                turns,
                CRYSTAL_BASE_LEVEL.saturating_add(CRYSTAL_RISE_LEVELS),
                2,
            )
            .ok()?;
            if upper_outward.len() != 2 || upper_outward.iter().any(|row| row.len() != 4) {
                return None;
            }
            let maximum_frozen_distance = upper_outward
                .iter()
                .flatten()
                .map(|coord| {
                    frozen_mask
                        .iter()
                        .map(|frozen| coord.distance(*frozen))
                        .min()
                        .unwrap_or(u32::MAX)
                })
                .max()
                .unwrap_or(u32::MAX);
            let total_frozen_distance =
                upper_outward
                    .iter()
                    .flatten()
                    .fold(0_u32, |total, coord| {
                        total.saturating_add(
                            frozen_mask
                                .iter()
                                .map(|frozen| coord.distance(*frozen))
                                .min()
                                .unwrap_or(u32::MAX),
                        )
                    });
            let lower_tunnel_distance = super::crystal_ascent::macro_lower_terminal_coords(
                &layout.patches.get(&patch_id)?.mask,
                turns,
                CRYSTAL_BASE_LEVEL,
            )
            .ok()?
            .iter()
            .map(|coord| coord.distance(tunnel_target))
            .min()
            .unwrap_or(u32::MAX);
            Some((
                maximum_frozen_distance,
                total_frozen_distance,
                lower_tunnel_distance,
                turns,
            ))
        })
        .min()
        .and_then(|(maximum_frozen_distance, _, _, turns)| {
            (maximum_frozen_distance == 0).then_some(turns)
        })
        .ok_or_else(|| {
            contract(
                "Crystal summit cannot orient both four-wide outside rows directly into Frozen Woods",
            )
        })?;
    layout
        .patches
        .get_mut(&patch_id)
        .ok_or_else(|| contract("locked Crystal Ascent cell disappeared during rotation"))?
        .rotation_turns = rotation;
    layout
        .validate()
        .map_err(|error| contract(format!("Crystal site ownership is invalid: {error}")))?;
    Ok((patch_id, stats))
}

fn rehome_orphan_components(
    layout: &mut ResolvedLayoutPlan,
    excluded_patch: PatchId,
    landform_by_patch: &BTreeMap<PatchId, hex_schematic::LandformKind>,
    mut orphans: Vec<(PatchId, BTreeSet<HexCoord>)>,
) -> Result<usize, V3GenerationError> {
    orphans.sort_unstable_by_key(|(source, component)| {
        (component.first().copied(), *source, component.len())
    });
    let mut owner_by_coord = layout
        .patches
        .iter()
        .filter(|(owner, _)| **owner != excluded_patch)
        .flat_map(|(owner, patch)| patch.mask.iter().map(move |coord| (*coord, *owner)))
        .collect::<BTreeMap<_, _>>();
    let mut rehomed = 0_usize;
    while !orphans.is_empty() {
        let mut progress = false;
        let mut deferred = Vec::new();
        for (source, component) in orphans {
            let mut shared_edges = BTreeMap::<PatchId, usize>::new();
            for coord in &component {
                for neighbor in coord.neighbors() {
                    let Some(owner) = owner_by_coord.get(&neighbor).copied() else {
                        continue;
                    };
                    if owner != excluded_patch {
                        *shared_edges.entry(owner).or_default() += 1;
                    }
                }
            }
            let recipient = shared_edges
                .into_iter()
                .min_by_key(|(owner, count)| {
                    (
                        landform_by_patch.get(owner) != landform_by_patch.get(&source),
                        Reverse(*count),
                        *owner,
                    )
                })
                .map(|(owner, _)| owner);
            let Some(recipient) = recipient else {
                deferred.push((source, component));
                continue;
            };
            let recipient_mask = &mut layout
                .patches
                .get_mut(&recipient)
                .ok_or_else(|| contract("orphan recipient vanished during Crystal claim"))?
                .mask;
            recipient_mask.extend(component.iter().copied());
            owner_by_coord.extend(component.into_iter().map(|coord| (coord, recipient)));
            rehomed = rehomed.saturating_add(1);
            progress = true;
        }
        if !progress {
            return Err(contract(format!(
                "Crystal radius-32 claim left {} orphan ownership components",
                deferred.len()
            )));
        }
        orphans = deferred;
    }
    Ok(rehomed)
}

fn connected_components(mask: &BTreeSet<HexCoord>) -> Vec<BTreeSet<HexCoord>> {
    let mut remaining = mask.clone();
    let mut components = Vec::new();
    while let Some(start) = remaining.first().copied() {
        remaining.remove(&start);
        let mut component = BTreeSet::from([start]);
        let mut frontier = VecDeque::from([start]);
        while let Some(coord) = frontier.pop_front() {
            for neighbor in coord.neighbors() {
                if remaining.remove(&neighbor) {
                    component.insert(neighbor);
                    frontier.push_back(neighbor);
                }
            }
        }
        components.push(component);
    }
    components
}

/// Builds and validates the existing authored landmark once for the schematic seed.
pub(crate) fn construct_fragment(
    layout: &ResolvedLayoutPlan,
    patch_id: PatchId,
    level_height: f32,
    world_seed: u64,
    art_catalog: &RuntimeArtCatalog,
) -> Result<GeneratedPatchPlan, V3GenerationError> {
    let objects = CrystalAscentObjectSet::resolve(art_catalog).map_err(|error| {
        contract(format!(
            "Grand V3 Crystal Ascent object preflight failed: {error}"
        ))
    })?;
    let trees =
        TemperateTreeSet::resolve(art_catalog, "Grand V3 Crystal Ascent").map_err(contract)?;
    let settings = V3CrystalAscentSettings {
        base_level: CRYSTAL_BASE_LEVEL,
        rise_levels: CRYSTAL_RISE_LEVELS,
    };
    let patch = PatchRecipeContext::resolve(layout, patch_id)?;
    let fragment = super::crystal_ascent::construct_patch(
        patch,
        &settings,
        level_height,
        PatchBuildMode::Candidate {
            world_seed,
            candidate: 0,
        },
        &trees,
        &objects,
    )
    .map_err(fragment_issues)?;
    match super::crystal_ascent::validate_composite_fragment(&fragment, layout, &settings, &objects)
    {
        WorldValidation::Valid(_) => Ok(fragment),
        WorldValidation::Invalid(issues) => Err(fragment_issues(issues)),
    }
}

/// Replaces the proxy terrain at the claimed site with the exact authored fragment.
pub(crate) fn merge_fragment(
    world: &mut GeneratedWorldPlan,
    fragment: GeneratedPatchPlan,
) -> Result<(), V3GenerationError> {
    if world.layout.kind != LayoutKind::Schematic {
        return Err(contract(
            "Crystal fragment merge requires a Schematic world",
        ));
    }
    let expected_mask = world
        .layout
        .patches
        .get(&fragment.patch_id)
        .ok_or_else(|| contract("Crystal fragment patch is absent from world layout"))?
        .mask
        .clone();
    let rotation_turns = world
        .layout
        .patches
        .get(&fragment.patch_id)
        .map(|patch| patch.rotation_turns)
        .ok_or_else(|| contract("Crystal fragment patch lost its authored rotation"))?;
    let natural_shell_overburden = super::crystal_ascent::macro_composite_natural_shell_overburden(
        &expected_mask,
        rotation_turns,
    )
    .map_err(contract)?;
    if fragment.volume.mask != expected_mask {
        return Err(contract(
            "Crystal fragment does not own the exact claimed radius-32 mask",
        ));
    }
    let local_anchor_aliases = fragment
        .anchors
        .iter()
        .filter(|(name, _)| name.starts_with("crystal_ascent."))
        .map(|(name, position)| (name.clone(), *position))
        .collect::<BTreeMap<_, _>>();
    let local_route_aliases = fragment
        .features
        .protected_routes
        .iter()
        .filter(|(name, _)| name.starts_with("crystal_ascent."))
        .map(|(name, route)| (name.clone(), route.clone()))
        .collect::<BTreeMap<_, _>>();

    if world
        .liquids
        .bodies
        .values()
        .flat_map(|body| body.nodes.keys())
        .any(|position| expected_mask.contains(&position.coord))
    {
        return Err(contract(
            "authoritative hydrology overlaps the locked Crystal Ascent site",
        ));
    }
    if world
        .features
        .by_id
        .values()
        .any(|feature| expected_mask.contains(&feature.root.coord))
        || world.structures.by_id.values().any(|structure| {
            structure
                .voxels
                .iter()
                .any(|position| expected_mask.contains(&position.coord))
        })
    {
        return Err(contract(
            "global decoration or structures entered the reserved Crystal Ascent site",
        ));
    }

    let stale_surfaces = world
        .volume
        .surfaces
        .keys()
        .filter(|position| expected_mask.contains(&position.coord))
        .copied()
        .collect::<Vec<_>>();
    for position in stale_surfaces {
        world.volume.surfaces.remove(&position);
        world.biome_regions.remove(&position);
    }
    for coord in &expected_mask {
        world.volume.columns.remove(coord);
    }

    let fragment = fragment
        .namespace(LayoutKind::Schematic, true)
        .map_err(composition_error)?;
    extend_unique(
        &mut world.volume.columns,
        fragment.volume.columns,
        "Crystal terrain column",
    )?;
    extend_unique(
        &mut world.volume.surfaces,
        fragment.volume.surfaces,
        "Crystal surface",
    )?;
    extend_unique(
        &mut world.liquids.bodies,
        fragment.liquids.bodies,
        "Crystal liquid",
    )?;
    extend_unique(
        &mut world.features.by_id,
        fragment.features.by_id,
        "Crystal feature",
    )?;
    extend_unique(
        &mut world.features.protected_routes,
        fragment.features.protected_routes,
        "Crystal protected route",
    )?;
    extend_unique(
        &mut world.features.clearings,
        fragment.features.clearings,
        "Crystal clearing",
    )?;
    extend_unique(
        &mut world.structures.by_id,
        fragment.structures.by_id,
        "Crystal structure",
    )?;
    world.blockers.extend(fragment.blockers);
    extend_unique(&mut world.lights, fragment.lights, "Crystal light")?;
    extend_unique(
        &mut world.biome_regions,
        fragment.biome_regions,
        "Crystal biome surface",
    )?;
    extend_unique(
        &mut world.interiors.by_id,
        fragment.interiors.by_id,
        "Crystal interior",
    )?;
    extend_unique(&mut world.anchors, fragment.anchors, "Crystal anchor")?;
    extend_unique(
        &mut world.anchors,
        local_anchor_aliases,
        "canonical Crystal anchor",
    )?;
    extend_unique(
        &mut world.features.protected_routes,
        local_route_aliases,
        "canonical Crystal protected route",
    )?;
    apply_composite_natural_shell_overburden(
        &mut world.volume,
        &mut world.biome_regions,
        &mut world.interiors,
        &world.structures,
        &natural_shell_overburden,
    )?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CompositeOverburdenColumn {
    coord: HexCoord,
    old_surface: TilePos,
    new_surface: TilePos,
    metadata: SurfaceMetadata,
    biome: BiomeRegionId,
    interior: InteriorRegionId,
}

/// Adds a real alpine cover above the composite shell while keeping every
/// authored shell voxel intact below it.
///
/// The two natural runs inherit Crystal's cutaway owner and are appended to the
/// same interior roof authority. Thus normal views see irregular Stone/Snow
/// shoulders, while review cutaway still exposes the authored stairs and shell.
/// This is called only at the schematic merge boundary; the standalone recipe
/// never receives the cover.
fn apply_composite_natural_shell_overburden(
    volume: &mut VolumePlan,
    biome_regions: &mut BTreeMap<TilePos, BiomeRegionId>,
    interiors: &mut InteriorPlan,
    structures: &StructurePlan,
    natural_shell_overburden: &BTreeMap<HexCoord, Level>,
) -> Result<(), V3GenerationError> {
    if natural_shell_overburden.is_empty() {
        return Err(contract(
            "composite Crystal natural overburden has an empty shell footprint",
        ));
    }

    // Preflight the complete projection before changing one column. A malformed
    // authored roof therefore fails the merge closed rather than leaving a
    // partially covered landmark behind.
    let mut planned = Vec::with_capacity(natural_shell_overburden.len());
    for (coord, thickness) in natural_shell_overburden {
        if !(2..=6).contains(thickness) {
            return Err(contract(format!(
                "composite Crystal natural overburden at {coord:?} has invalid thickness {thickness}"
            )));
        }
        let (surface, metadata) = volume.top_surface_at_coord(*coord).ok_or_else(|| {
            contract(format!(
                "composite Crystal natural overburden has no shell surface at {coord:?}"
            ))
        })?;
        let column = volume.columns.get(coord).ok_or_else(|| {
            contract(format!(
                "composite Crystal natural overburden has no shell column at {coord:?}"
            ))
        })?;
        let highest_occupied_top = column
            .elements
            .iter()
            .map(|element| match *element {
                VolumeElement::Solid(mass) => mass.levels.top,
                VolumeElement::Fill(fill) => fill.levels.top,
            })
            .max();
        if highest_occupied_top != Some(surface.level.saturating_add(1)) {
            return Err(contract(format!(
                "composite Crystal natural overburden found occupied volume above {surface:?}"
            )));
        }
        let cap = solid_mass_covering(volume, surface).ok_or_else(|| {
            contract(format!(
                "composite Crystal natural overburden found no authored roof mass at {surface:?}"
            ))
        })?;
        let Some(interior) = cap.cutaway_for else {
            return Err(contract(format!(
                "composite Crystal natural overburden found an unowned roof at {surface:?}"
            )));
        };
        if cap.material != SolidMaterialRole::WorkedStone
            || cap.levels.top != surface.level.saturating_add(1)
        {
            return Err(contract(format!(
                "composite Crystal natural overburden found a non-authored shell cap at {surface:?}"
            )));
        }
        if interiors
            .by_id
            .get(&interior)
            .is_none_or(|planned_interior| !planned_interior.roof_voxels.contains(&surface))
        {
            return Err(contract(format!(
                "composite Crystal natural overburden roof {surface:?} is absent from its cutaway authority"
            )));
        }
        if !structures.by_id.values().any(|structure| {
            structure.kind == super::world::StructureKind::Wall
                && structure.voxels.contains(&surface)
        }) {
            return Err(contract(format!(
                "composite Crystal natural overburden has no authored shell voxel beneath {surface:?}"
            )));
        }
        let new_level = surface.level.checked_add(*thickness).ok_or_else(|| {
            contract(format!(
                "composite Crystal natural overburden level overflowed above {surface:?}"
            ))
        })?;
        if new_level > crate::settings::MAX_V3_LEVEL {
            return Err(contract(format!(
                "composite Crystal natural overburden exceeds the V3 ceiling at {coord:?}/{new_level}"
            )));
        }
        let biome = biome_regions.get(&surface).copied().ok_or_else(|| {
            contract(format!(
                "composite Crystal natural overburden has no biome authority at {surface:?}"
            ))
        })?;
        planned.push(CompositeOverburdenColumn {
            coord: *coord,
            old_surface: surface,
            new_surface: TilePos::new(*coord, new_level),
            metadata,
            biome,
            interior,
        });
    }

    for cover in planned {
        let column = volume
            .columns
            .get_mut(&cover.coord)
            .ok_or_else(|| contract("preflighted Crystal overburden column disappeared"))?;
        let stone_bottom = cover.old_surface.level.saturating_add(1);
        column.elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(stone_bottom, cover.new_surface.level),
            material: SolidMaterialRole::Stone,
            cutaway_for: Some(cover.interior),
        }));
        column.elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(
                cover.new_surface.level,
                cover.new_surface.level.saturating_add(1),
            ),
            material: SolidMaterialRole::Snow,
            cutaway_for: Some(cover.interior),
        }));
        let removed_surface = volume.surfaces.remove(&cover.old_surface);
        let removed_biome = biome_regions.remove(&cover.old_surface);
        debug_assert_eq!(removed_surface, Some(cover.metadata));
        debug_assert_eq!(removed_biome, Some(cover.biome));
        volume.surfaces.insert(cover.new_surface, cover.metadata);
        biome_regions.insert(cover.new_surface, cover.biome);
        let interior = interiors
            .by_id
            .get_mut(&cover.interior)
            .ok_or_else(|| contract("preflighted Crystal cutaway authority disappeared"))?;
        interior.roof_voxels.extend(
            (stone_bottom..=cover.new_surface.level).map(|level| TilePos::new(cover.coord, level)),
        );
    }
    Ok(())
}

fn solid_mass_covering(volume: &VolumePlan, position: TilePos) -> Option<SolidMass> {
    volume
        .columns
        .get(&position.coord)?
        .elements
        .iter()
        .find_map(|element| {
            let VolumeElement::Solid(mass) = *element else {
                return None;
            };
            (mass.levels.bottom <= position.level && position.level < mass.levels.top)
                .then_some(mass)
        })
}

fn extend_unique<K, V>(
    target: &mut BTreeMap<K, V>,
    additions: BTreeMap<K, V>,
    label: &str,
) -> Result<(), V3GenerationError>
where
    K: Ord,
{
    for (key, value) in additions {
        match target.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(value);
            }
            Entry::Occupied(_) => {
                return Err(contract(format!("{label} collided during exact merge")));
            }
        }
    }
    Ok(())
}

fn fragment_issues(issues: Vec<WorldValidationIssue>) -> V3GenerationError {
    contract(format!(
        "Grand V3 Crystal Ascent validation failed: {}",
        issues
            .into_iter()
            .map(|issue| issue.detail)
            .collect::<Vec<_>>()
            .join("; ")
    ))
}

fn composition_error(error: WorldCompositionError) -> V3GenerationError {
    contract(format!(
        "Grand V3 Crystal Ascent namespacing failed: {error:?}"
    ))
}

fn contract(detail: impl Into<String>) -> V3GenerationError {
    V3GenerationError::RecipeContract(detail.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procedural_v3::layout::resolve_layout;
    use crate::settings::{
        V3GrandV3BasicTerrainProfile, V3LayoutSettings, V3SchematicLayoutSettings,
        V3SchematicTemplate, V3SchematicTerrainProfile,
    };

    #[test]
    fn composite_overburden_adds_real_stone_and_snow_and_extends_cutaway_ownership() {
        let coord = HexCoord::ORIGIN;
        let surface = hex_core::TilePos::new(coord, 9);
        let interior = hex_core::InteriorRegionId(7);
        let mut volume = VolumePlan::new(BTreeSet::from([coord]));
        volume.columns.insert(
            coord,
            super::super::volume::VolumeColumn {
                elements: vec![
                    VolumeElement::Solid(SolidMass {
                        levels: LevelInterval::new(0, 9),
                        material: SolidMaterialRole::WorkedStone,
                        cutaway_for: None,
                    }),
                    VolumeElement::Solid(SolidMass {
                        levels: LevelInterval::new(9, 10),
                        material: SolidMaterialRole::WorkedStone,
                        cutaway_for: Some(interior),
                    }),
                ],
            },
        );
        let metadata = super::super::volume::SurfaceMetadata {
            access: super::super::volume::SurfaceAccess::SpecialMovement(
                hex_core::SpecialMovementRegion(7),
            ),
            interior: None,
        };
        volume.surfaces.insert(surface, metadata);
        let biome = hex_core::BiomeRegionId(9);
        let mut biome_regions = BTreeMap::from([(surface, biome)]);
        let mut interiors = InteriorPlan {
            by_id: BTreeMap::from([(
                interior,
                super::super::world::PlannedInterior {
                    floors: BTreeSet::new(),
                    entrances: BTreeSet::new(),
                    roof_voxels: BTreeSet::from([surface]),
                },
            )]),
        };
        let structures = StructurePlan {
            by_id: BTreeMap::from([(
                super::super::world::StructureId(1),
                super::super::world::PlannedStructure {
                    kind: super::super::world::StructureKind::Wall,
                    voxels: BTreeSet::from([surface]),
                },
            )]),
        };

        apply_composite_natural_shell_overburden(
            &mut volume,
            &mut biome_regions,
            &mut interiors,
            &structures,
            &BTreeMap::from([(coord, 3)]),
        )
        .expect("worked shell cap accepts real composite overburden");

        assert_eq!(
            volume.columns[&coord].elements,
            vec![
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(0, 9),
                    material: SolidMaterialRole::WorkedStone,
                    cutaway_for: None,
                }),
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(9, 10),
                    material: SolidMaterialRole::WorkedStone,
                    cutaway_for: Some(interior),
                }),
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(10, 12),
                    material: SolidMaterialRole::Stone,
                    cutaway_for: Some(interior),
                }),
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(12, 13),
                    material: SolidMaterialRole::Snow,
                    cutaway_for: Some(interior),
                }),
            ]
        );
        let natural_surface = TilePos::new(coord, 12);
        assert!(!volume.surfaces.contains_key(&surface));
        assert_eq!(volume.surfaces.get(&natural_surface), Some(&metadata));
        assert_eq!(biome_regions.get(&natural_surface), Some(&biome));
        assert_eq!(
            structures.by_id[&super::super::world::StructureId(1)].voxels,
            BTreeSet::from([surface])
        );
        assert_eq!(
            interiors.by_id[&interior].roof_voxels,
            (9..=12)
                .map(|level| TilePos::new(coord, level))
                .collect::<BTreeSet<_>>()
        );
        volume
            .validate()
            .expect("real overburden remains a valid volume");
    }

    #[test]
    fn exact_composite_fragment_buries_shell_but_preserves_openings_and_authored_structure() {
        let plan = hex_schematic::reference_plan(
            &hex_schematic::grand_v3_reference_template().expect("template parses"),
            0,
        )
        .expect("reference validates")
        .plan;
        let settings = crate::settings::ProceduralV3Settings {
            layout: V3LayoutSettings::Schematic(V3SchematicLayoutSettings {
                template: V3SchematicTemplate::GrandV3,
                template_revision: crate::settings::V3_GRAND_V3_TEMPLATE_REVISION,
                cell_pitch: 22,
                terrain_profile: V3SchematicTerrainProfile::GrandV3BasicV1(
                    V3GrandV3BasicTerrainProfile::canonical(),
                ),
            }),
        };
        let mut layout = resolve_layout(187, &settings).expect("layout resolves");
        let admission = claim_site(&plan, &mut layout, 22).expect("site claim validates");
        let mut fragment = construct_fragment(
            &layout,
            admission.patch_id(),
            0.4,
            0,
            crate::procedural_v3::crystal_ascent_assets::tests::runtime_art_catalog(),
        )
        .expect("exact composite fragment constructs");
        let patch = layout
            .patches
            .get(&admission.patch_id())
            .expect("claimed Crystal patch remains present");
        let overburden = super::super::crystal_ascent::macro_composite_natural_shell_overburden(
            &patch.mask,
            patch.rotation_turns,
        )
        .expect("natural shell overburden resolves");
        let openings = super::super::crystal_ascent::macro_composite_exposed_shell_opening_coords(
            &patch.mask,
            patch.rotation_turns,
        )
        .expect("authored shell openings resolve");
        let before = fragment.volume.clone();
        let before_structures = fragment.structures.clone();
        let before_interiors = fragment.interiors.clone();
        apply_composite_natural_shell_overburden(
            &mut fragment.volume,
            &mut fragment.biome_regions,
            &mut fragment.interiors,
            &fragment.structures,
            &overburden,
        )
        .expect("exact composite fragment accepts its natural overburden");
        let after = &fragment.volume;

        assert_eq!(after.mask, before.mask);
        assert_eq!(fragment.structures, before_structures);
        for coord in &openings {
            assert_eq!(after.columns.get(coord), before.columns.get(coord));
            assert_eq!(
                after.top_surface_at_coord(*coord),
                before.top_surface_at_coord(*coord)
            );
        }
        let mut distinct_thicknesses = BTreeSet::new();
        for (coord, thickness) in &overburden {
            distinct_thicknesses.insert(*thickness);
            let old_surface = before
                .top_surface_at_coord(*coord)
                .map(|(surface, _)| surface)
                .expect("authored shell has an exact roof surface");
            let surface = after
                .top_surface_at_coord(*coord)
                .map(|(surface, _)| surface)
                .expect("covered shell has an exact natural surface");
            assert_eq!(surface.level, old_surface.level.saturating_add(*thickness));
            let before_mass = solid_mass_covering(&before, old_surface)
                .expect("authored shell cap is solid before covering");
            let buried_mass = solid_mass_covering(after, old_surface)
                .expect("authored shell cap remains solid below the cover");
            let stone =
                solid_mass_covering(after, TilePos::new(*coord, surface.level.saturating_sub(1)))
                    .expect("natural rock underlies the snow cap");
            let snow =
                solid_mass_covering(after, surface).expect("natural snow caps the composite cover");
            assert_eq!(before_mass.material, SolidMaterialRole::WorkedStone);
            assert_eq!(buried_mass, before_mass);
            assert_eq!(stone.material, SolidMaterialRole::Stone);
            assert_eq!(snow.material, SolidMaterialRole::Snow);
            assert_eq!(
                snow.levels,
                LevelInterval::new(surface.level, surface.level + 1)
            );
            assert_eq!(stone.cutaway_for, before_mass.cutaway_for);
            assert_eq!(snow.cutaway_for, before_mass.cutaway_for);
            let interior = snow
                .cutaway_for
                .expect("natural cover keeps cutaway ownership");
            assert!(fragment.interiors.by_id[&interior]
                .roof_voxels
                .contains(&surface));
            assert!(before_structures
                .by_id
                .values()
                .any(|structure| structure.voxels.contains(&old_surface)));
            assert!(!fragment
                .structures
                .by_id
                .values()
                .any(|structure| structure.voxels.contains(&surface)));
        }
        assert!(distinct_thicknesses.len() >= 4);
        let before_voxels = solid_voxels(&before);
        let after_voxels = solid_voxels(after);
        assert!(before_voxels.is_subset(&after_voxels));
        assert_eq!(
            after_voxels.len().saturating_sub(before_voxels.len()),
            overburden
                .values()
                .map(|thickness| usize::try_from(*thickness).unwrap_or_default())
                .sum::<usize>()
        );
        assert!(fragment.interiors.by_id.iter().all(|(id, interior)| {
            before_interiors
                .by_id
                .get(id)
                .is_some_and(|before_interior| {
                    before_interior.floors == interior.floors
                        && before_interior.entrances == interior.entrances
                        && before_interior.roof_voxels.is_subset(&interior.roof_voxels)
                })
        }));
        after.validate().expect("covered fragment volume validates");
    }

    fn solid_voxels(volume: &VolumePlan) -> BTreeSet<hex_core::TilePos> {
        volume
            .columns
            .iter()
            .flat_map(|(coord, column)| {
                column.elements.iter().flat_map(move |element| {
                    let VolumeElement::Solid(mass) = *element else {
                        return Vec::new().into_iter();
                    };
                    (mass.levels.bottom..mass.levels.top)
                        .map(|level| hex_core::TilePos::new(*coord, level))
                        .collect::<Vec<_>>()
                        .into_iter()
                })
            })
            .collect()
    }

    #[test]
    fn exact_radius_32_claim_preserves_all_217_connected_owners() {
        let plan = hex_schematic::reference_plan(
            &hex_schematic::grand_v3_reference_template().expect("template parses"),
            0,
        )
        .expect("reference validates")
        .plan;
        let settings = crate::settings::ProceduralV3Settings {
            layout: V3LayoutSettings::Schematic(V3SchematicLayoutSettings {
                template: V3SchematicTemplate::GrandV3,
                template_revision: crate::settings::V3_GRAND_V3_TEMPLATE_REVISION,
                cell_pitch: 22,
                terrain_profile: V3SchematicTerrainProfile::GrandV3BasicV1(
                    V3GrandV3BasicTerrainProfile::canonical(),
                ),
            }),
        };
        let mut layout = resolve_layout(187, &settings).expect("layout resolves");
        let original_layout = layout.clone();
        let (crystal, stats) =
            claim_site_with_stats(&plan, &mut layout, 22, true).expect("site claim validates");
        let cell = plan
            .cells
            .iter()
            .find(|cell| u32::from(cell.id.get()) == crystal.0)
            .expect("Crystal cell remains present");
        let center = HexCoord::from_axial(cell.coord.q() * 22, cell.coord.r() * 22);
        let site = center
            .within_radius(32)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let original_disjoint = original_layout
            .patches
            .iter()
            .filter(|(patch, resolved)| **patch != crystal && resolved.mask.is_disjoint(&site))
            .map(|(patch, resolved)| (*patch, resolved.mask.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut baseline_layout = original_layout;
        let (baseline_crystal, baseline_stats) =
            claim_site_with_stats(&plan, &mut baseline_layout, 22, false)
                .expect("unoptimized reference claim validates");

        assert_eq!(layout.patches.len(), 217);
        assert_eq!(layout.patches[&crystal].mask.len(), 3_169);
        assert_eq!(layout.patches[&crystal].mask, site);
        assert_eq!(crystal, baseline_crystal);
        assert_eq!(layout, baseline_layout);
        assert_eq!(stats.donor_patches, 216);
        assert_eq!(stats.disjoint_donors, original_disjoint.len());
        assert_eq!(stats.disjoint_donors_skipped, original_disjoint.len());
        assert_eq!(stats.intersecting_donors, 216 - original_disjoint.len());
        assert_eq!(stats.component_scans, stats.intersecting_donors);
        assert_eq!(baseline_stats.donor_patches, 216);
        assert_eq!(baseline_stats.disjoint_donors, original_disjoint.len());
        assert_eq!(baseline_stats.disjoint_donors_skipped, 0);
        assert_eq!(baseline_stats.component_scans, 216);
        assert_eq!(
            stats.orphan_components_found,
            baseline_stats.orphan_components_found
        );
        assert_eq!(
            stats.orphan_components_rehomed,
            baseline_stats.orphan_components_rehomed
        );
        assert!(original_disjoint.iter().all(|(patch, original_mask)| {
            original_mask.is_subset(&layout.patches[patch].mask)
        }));
        let chosen = layout.patches[&crystal].rotation_turns;
        let frozen_patches = plan
            .cells
            .iter()
            .filter(|candidate| {
                candidate
                    .facts
                    .overlays
                    .contains(&SchematicFeature::FrozenWoods)
            })
            .map(|candidate| PatchId(u32::from(candidate.id.get())))
            .collect::<BTreeSet<_>>();
        let frozen_mask = frozen_patches
            .iter()
            .flat_map(|patch| layout.patches[patch].mask.iter().copied())
            .collect::<BTreeSet<_>>();
        let outward = super::super::crystal_ascent::macro_upper_terminal_outward_rows(
            &layout.patches[&crystal].mask,
            chosen,
            CRYSTAL_BASE_LEVEL + CRYSTAL_RISE_LEVELS,
            2,
        )
        .expect("chosen summit orientation resolves");
        assert_eq!(outward.len(), 2);
        assert!(outward
            .iter()
            .all(|row| { row.len() == 4 && row.is_subset(&frozen_mask) }));
        assert_eq!(
            layout
                .patches
                .values()
                .map(|patch| patch.mask.len())
                .sum::<usize>(),
            105_469
        );
        layout.validate().expect("claimed layout stays strict");
    }

    #[test]
    fn claimed_layout_witness_accepts_exact_layout_and_rejects_valid_mutation() {
        let plan = hex_schematic::reference_plan(
            &hex_schematic::grand_v3_reference_template().expect("template parses"),
            0,
        )
        .expect("reference validates")
        .plan;
        let settings = crate::settings::ProceduralV3Settings {
            layout: V3LayoutSettings::Schematic(V3SchematicLayoutSettings {
                template: V3SchematicTemplate::GrandV3,
                template_revision: crate::settings::V3_GRAND_V3_TEMPLATE_REVISION,
                cell_pitch: 22,
                terrain_profile: V3SchematicTerrainProfile::GrandV3BasicV1(
                    V3GrandV3BasicTerrainProfile::canonical(),
                ),
            }),
        };
        let mut layout = resolve_layout(187, &settings).expect("layout resolves");
        let admission = claim_site(&plan, &mut layout, 22).expect("site claim validates");

        assert!(admission.matches_final_layout(&layout));

        let mut mutated = layout.clone();
        let (_, patch) = mutated
            .patches
            .iter_mut()
            .find(|(patch_id, _)| **patch_id != admission.patch_id())
            .expect("claimed layout has another patch");
        patch.rotation_turns = (patch.rotation_turns + 1) % 6;
        mutated
            .validate()
            .expect("the alternate rotation remains a valid Schematic layout");
        assert!(!admission.matches_final_layout(&mutated));
    }

    #[test]
    fn default_seed_rehomes_every_disconnected_crystal_donor_component() {
        let template = hex_schematic::grand_v3_reference_template().expect("template parses");
        let plan = hex_schematic::generate(&template, 1_592_598_566)
            .expect("default schematic generates")
            .plan;
        let settings = crate::settings::ProceduralV3Settings {
            layout: V3LayoutSettings::Schematic(V3SchematicLayoutSettings {
                template: V3SchematicTemplate::GrandV3,
                template_revision: crate::settings::V3_GRAND_V3_TEMPLATE_REVISION,
                cell_pitch: 22,
                terrain_profile: V3SchematicTerrainProfile::GrandV3BasicV1(
                    V3GrandV3BasicTerrainProfile::canonical(),
                ),
            }),
        };
        let mut layout = resolve_layout(187, &settings).expect("layout resolves");
        let (crystal, stats) = claim_site_with_stats(&plan, &mut layout, 22, true)
            .expect("site claim rehomes orphans");

        assert_eq!(layout.patches.len(), 217);
        assert!(stats.intersecting_donors_with_orphans > 0);
        assert!(stats.orphan_components_found > 0);
        assert_eq!(
            stats.orphan_components_rehomed,
            stats.orphan_components_found
        );
        assert!(layout
            .patches
            .iter()
            .all(|(patch, resolved)| *patch == crystal
                || connected_components(&resolved.mask).len() == 1));
        assert_eq!(
            layout
                .patches
                .values()
                .map(|patch| patch.mask.len())
                .sum::<usize>(),
            105_469
        );
        layout.validate().expect("reassigned layout stays strict");
    }
}
