//! Exact Crystal Ascent reuse inside the Grand V3 schematic world.
//!
//! The landmark remains owned by its established authored recipe. This adapter only
//! expands the locked schematic cell to the recipe's radius-32 site, resolves the
//! accepted art dependencies, and merges the namespaced fragment into the otherwise
//! continuous schematic terrain.

use std::cmp::Reverse;
use std::collections::{btree_map::Entry, BTreeMap, BTreeSet, VecDeque};

use hex_assets::RuntimeArtCatalog;
use hex_core::HexCoord;
use hex_schematic::{FeatureKind as SchematicFeature, NetworkKind, SchematicPlanV1};

use super::composition::{GeneratedPatchPlan, WorldCompositionError};
use super::layout::{LayoutKind, PatchId, ResolvedLayoutPlan};
use super::patch::{PatchBuildMode, PatchRecipeContext};
use super::selection::WorldValidation;
use super::vegetation::TemperateTreeSet;
use super::world::{GeneratedWorldPlan, WorldValidationIssue};
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
    Ok(())
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
                template_revision: 2,
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
                template_revision: 2,
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
                template_revision: 2,
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
