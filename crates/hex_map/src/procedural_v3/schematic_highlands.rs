//! Connected highland shaping for the Grand V3 schematic compiler.
//!
//! Schematic cells retain semantic ownership, but they are not individual
//! terrain stamps. The authored peak cells become connected ridge chains, the
//! massif becomes one centrally crested body, and Crystal Ascent receives a
//! contextual mountain mantle outside its exact radius-32 authored site. When
//! that exact site separates the fine Massif ownership mask, the height field
//! may cross the shortest eligible Mountain corridor without transferring any
//! column to another biome owner.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};

use hex_core::{HexCoord, Level};
use hex_schematic::{
    CellPlan, FeatureKind as SchematicFeature, LandformKind, SchematicCoord, SchematicPlanV1,
    SurfaceKind,
};

use super::layout::{PatchId, ResolvedLayoutPlan};
use super::V3GenerationError;
use crate::settings::{V3GrandV3BasicTerrainProfile, MAX_V3_LEVEL};

const CELL_PITCH: i32 = 22;
pub(super) const CRYSTAL_SITE_RADIUS: u32 = 32;
// The mantle is a six-level-per-column cone, extended until its authored
// target is below even the lowest canonical dry-land datum.  Ending the old
// radius-50 mantle while clamping it to `high_core_level` left a 33-level
// circular cliff where the clamp stopped, including across otherwise ordinary
// Massif ownership seams.  Letting the cone finish below the surrounding base
// field means `max(base, mantle)` joins continuously instead of exposing the
// implementation radius in the terrain.
const CRYSTAL_MANTLE_OUTER_RADIUS: u32 = 61;
const CRYSTAL_MANTLE_TAPER_PER_HEX: Level = 6;
const CRYSTAL_MANTLE_EXIT_CLEARANCE_DEPTH: u32 = 20;
const CRYSTAL_MANTLE_EXIT_CLEARANCE_BUFFER: u32 = 1;
pub(super) const CRYSTAL_ARCHITECTURE_TOP: Level = 169;
const CRYSTAL_MANTLE_INNER_LEVEL: Level = 176;
const PEAK_SUMMIT_MIN: Level = 200;
const PEAK_SUMMIT_MAX: Level = 218;
pub(super) const MASSIF_SUMMIT_MIN: Level = 224;
pub(super) const MASSIF_SUMMIT_MAX: Level = 236;
const MASSIF_RIDGE_FLOOR: Level = 178;
const MASSIF_RIDGE_SLOPE: Level = 3;
const MASSIF_BOUNDARY_RISE_PER_HEX: Level = 2;
const PEAK_RIDGE_SLOPE: Level = 2;
const MASSIF_MAXIMUM_SUPPRESSION: Level = 30;

#[derive(Debug, Clone, PartialEq, Eq)]
struct MassifField {
    mask: BTreeSet<HexCoord>,
    ridge_influence: BTreeMap<HexCoord, Level>,
    ridge_distance: BTreeMap<HexCoord, u32>,
    boundary_depth: BTreeMap<HexCoord, u32>,
    crest: HexCoord,
    summit: Level,
    floor: Level,
}

/// Final-world evidence for the two locked Grand V3 peak chains.
///
/// Route construction is allowed to cut narrow saddles and foothill ledges
/// through PeakRing ownership after the scalar field is built.  Keeping the
/// exact six patch masks and seeded summit pins separate from those routes lets
/// final validation prove that both high ridges survived without requiring
/// every coordinate in a PeakRing patch to remain high.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PeakRidgeAuthority {
    pub(super) components: Vec<PeakRidgeComponentAuthority>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PeakRidgeComponentAuthority {
    pub(super) patch_masks: BTreeMap<PatchId, BTreeSet<HexCoord>>,
    pub(super) expected_high_band: BTreeMap<HexCoord, Level>,
    pub(super) summit_pins: BTreeMap<HexCoord, Level>,
    /// Exact levels intentionally graded by the three authored peak routes.
    ///
    /// Foundation construction leaves this unsealed.  The schematic compiler
    /// seals it immediately after publishing those routes and before any
    /// generic connector is allowed to mutate terrain.  Final validation can
    /// consequently admit only the production-observed footprint, rather than
    /// every coordinate that happens to belong to a broad route corridor.
    pub(super) authorized_route_grades: Option<BTreeMap<HexCoord, Level>>,
}

/// Immutable ownership evidence for the connected visual Massif field.
///
/// Crystal's exact site may split the semantic Massif masks for some generated
/// schematics. The scalar terrain field is allowed to bridge those pieces only
/// through overlay-free Mountain columns, without transferring their stable
/// biome ownership. Capturing both masks and every connector owner here lets the
/// final-world validator distinguish that visual projection from semantic
/// ownership instead of silently weakening either contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MassifVisualAuthority {
    pub(super) visual_mask: BTreeSet<HexCoord>,
    pub(super) semantic_owner_mask: BTreeSet<HexCoord>,
    pub(super) connector_owners: BTreeMap<HexCoord, PatchId>,
}

impl MassifField {
    fn resolve(&self, coord: HexCoord, baseline: Level) -> Level {
        if !self.mask.contains(&coord) {
            return baseline;
        }
        let ridge_distance = self.ridge_distance.get(&coord).copied().unwrap_or_default();
        let boundary_depth = self.boundary_depth.get(&coord).copied().unwrap_or_default();
        let suppression = i32::try_from(ridge_distance)
            .unwrap_or(i32::MAX)
            .saturating_mul(2)
            .min(
                i32::try_from(boundary_depth)
                    .unwrap_or(i32::MAX)
                    .saturating_mul(3),
            )
            .min(MASSIF_MAXIMUM_SUPPRESSION);
        let carved = baseline.saturating_sub(suppression).max(self.floor);
        let target = self
            .ridge_influence
            .get(&coord)
            .copied()
            .unwrap_or(self.floor);
        if coord == self.crest {
            return self.summit;
        }
        // Fade the shared massif field inward without exposing coarse-owner
        // seams. Matching the ridge slope leaves room for the independently
        // blended baseline to vary while keeping adjacent shoulders gradual.
        let boundary_raise = i32::try_from(boundary_depth)
            .unwrap_or(i32::MAX)
            .saturating_mul(MASSIF_BOUNDARY_RISE_PER_HEX);
        // First build a globally smooth ridge shoulder from the already blended
        // baseline, then cut back one third of the bounded ridge-distance
        // suppression. Applying the complete suppression before the rise made
        // neighbouring coarse owners diverge by as much as eight levels; applying
        // no suppression made the chisel mathematically dead and produced a broad
        // regular mound. The two-up/one-down envelope retains visible gullies while
        // preserving the six-level ownership-seam contract.
        let smooth_ridge = baseline
            .saturating_add(boundary_raise)
            .min(target)
            .max(baseline);
        smooth_ridge.saturating_sub(suppression / 3).max(carved)
    }
}

/// Exact deterministic highland corrections applied over the shared scalar base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GrandHighlandField {
    peak_levels: BTreeMap<HexCoord, Level>,
    peak_authority: PeakRidgeAuthority,
    massif: MassifField,
    massif_visual_authority: MassifVisualAuthority,
    crystal_mantle: BTreeMap<HexCoord, Level>,
    #[cfg(test)]
    crystal_center: HexCoord,
    #[cfg(test)]
    crystal_mask: BTreeSet<HexCoord>,
    #[cfg(test)]
    crystal_mantle_exit_clearance: BTreeSet<HexCoord>,
}

impl GrandHighlandField {
    pub(super) fn build(
        plan: &SchematicPlanV1,
        layout: &ResolvedLayoutPlan,
        profile: V3GrandV3BasicTerrainProfile,
    ) -> Result<Self, V3GenerationError> {
        let crystal = crystal_context(plan, layout, profile)?;

        let peak_cells = landmark_cells(plan, layout, |cell| {
            cell.facts.overlays.contains(&SchematicFeature::PeakRing)
        })?;
        let peak_mask = union_masks(layout, peak_cells.values().map(|cell| cell.patch))?;
        let peak_components = schematic_components(&peak_cells)?;
        if peak_cells.len() != 12
            || peak_components.len() != 2
            || peak_components.iter().any(|component| component.len() != 6)
        {
            return Err(contract(format!(
                "Grand V3 highlands require the locked two six-cell peak chains; found {} cells in component sizes {:?}",
                peak_cells.len(),
                peak_components
                    .iter()
                    .map(BTreeSet::len)
                    .collect::<Vec<_>>()
            )));
        }
        let (peak_levels, peak_ridge_components) = build_peak_field(
            &peak_cells,
            &peak_components,
            &peak_mask,
            profile,
            plan.provenance.world_seed,
        )?;
        let peak_authority = build_peak_ridge_authority(
            layout,
            &peak_cells,
            &peak_components,
            &peak_levels,
            &peak_ridge_components,
        )?;

        let massif_cells = landmark_cells(plan, layout, |cell| {
            cell.facts.surface == SurfaceKind::Land && cell.facts.landform == LandformKind::Massif
        })?;
        let massif_owner_mask = union_masks(layout, massif_cells.values().map(|cell| cell.patch))?;
        let massif_visual_authority =
            build_massif_visual_authority(plan, layout, &massif_owner_mask, &crystal.mask)?;
        let massif_crest_owner_mask = union_masks(
            layout,
            massif_cells.iter().filter_map(|(schematic, cell)| {
                schematic
                    .checked_distance(crystal.schematic)
                    .is_some_and(|distance| distance >= 2)
                    .then_some(cell.patch)
            }),
        )?;
        let massif = build_massif_field(
            &massif_visual_authority.visual_mask,
            &massif_crest_owner_mask,
            &crystal.mask,
            profile,
            plan.provenance.world_seed,
        )?;
        let crystal_mantle = build_crystal_mantle(plan, layout, profile, &crystal)?;
        if crystal_mantle
            .keys()
            .any(|coord| crystal.mask.contains(coord))
        {
            return Err(contract(
                "Grand V3 Crystal mantle entered the exact radius-32 authored site",
            ));
        }

        Ok(Self {
            peak_levels,
            peak_authority,
            massif,
            massif_visual_authority,
            crystal_mantle,
            #[cfg(test)]
            crystal_center: crystal.center,
            #[cfg(test)]
            crystal_mask: crystal.mask,
            #[cfg(test)]
            crystal_mantle_exit_clearance: crystal.exit_clearance,
        })
    }

    pub(super) fn resolve_surface_level(
        &self,
        cell: &CellPlan,
        coord: HexCoord,
        baseline: Level,
    ) -> Level {
        let shaped = if self.massif.mask.contains(&coord) {
            // A visual-only connector can traverse an ordinary Mountain patch.
            // Field membership, rather than semantic ownership, is the exact
            // authority for applying its continuous scalar surface.
            self.massif.resolve(coord, baseline)
        } else {
            match cell.facts.landform {
                LandformKind::SharpPeak => {
                    self.peak_levels.get(&coord).copied().unwrap_or(baseline)
                }
                LandformKind::None
                | LandformKind::Island
                | LandformKind::Beach
                | LandformKind::Shore
                | LandformKind::Valley
                | LandformKind::Plateau
                | LandformKind::Hill
                | LandformKind::Mountain
                | LandformKind::Massif => baseline,
            }
        };
        self.crystal_mantle
            .get(&coord)
            .copied()
            .map_or(shaped, |mantle| shaped.max(mantle))
    }

    pub(super) const fn massif_crest(&self) -> (HexCoord, Level) {
        (self.massif.crest, self.massif.summit)
    }

    pub(super) const fn peak_ridge_authority(&self) -> &PeakRidgeAuthority {
        &self.peak_authority
    }

    pub(super) const fn massif_visual_authority(&self) -> &MassifVisualAuthority {
        &self.massif_visual_authority
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LandmarkCell {
    patch: PatchId,
    representative: HexCoord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CrystalContext {
    schematic: SchematicCoord,
    center: HexCoord,
    mask: BTreeSet<HexCoord>,
    exit_clearance: BTreeSet<HexCoord>,
}

fn landmark_cells(
    plan: &SchematicPlanV1,
    layout: &ResolvedLayoutPlan,
    predicate: impl Fn(&CellPlan) -> bool,
) -> Result<BTreeMap<SchematicCoord, LandmarkCell>, V3GenerationError> {
    plan.cells
        .iter()
        .filter(|cell| predicate(cell))
        .map(|cell| {
            let patch_id = PatchId(u32::from(cell.id.get()));
            let patch = layout.patches.get(&patch_id).ok_or_else(|| {
                contract(format!(
                    "Grand V3 highland cell {} has no resolved patch",
                    cell.id.get()
                ))
            })?;
            let nominal = schematic_to_world(cell.coord);
            let representative = patch
                .mask
                .iter()
                .copied()
                .min_by_key(|coord| (coord.distance(nominal), *coord))
                .ok_or_else(|| {
                    contract(format!(
                        "Grand V3 highland cell {} has an empty resolved mask",
                        cell.id.get()
                    ))
                })?;
            Ok((
                cell.coord,
                LandmarkCell {
                    patch: patch_id,
                    representative,
                },
            ))
        })
        .collect()
}

fn union_masks(
    layout: &ResolvedLayoutPlan,
    patches: impl IntoIterator<Item = PatchId>,
) -> Result<BTreeSet<HexCoord>, V3GenerationError> {
    let mut mask = BTreeSet::new();
    for patch_id in patches {
        let patch = layout.patches.get(&patch_id).ok_or_else(|| {
            contract(format!(
                "Grand V3 highland patch {} disappeared during field construction",
                patch_id.0
            ))
        })?;
        mask.extend(patch.mask.iter().copied());
    }
    if mask.is_empty() {
        return Err(contract("Grand V3 highland mask is empty"));
    }
    Ok(mask)
}

/// Connects separated fine Massif ownership components through the shortest
/// available overlay-free Mountain corridor without mutating layout ownership.
///
/// Crystal's exact radius-32 claim is allowed to interrupt the nearest-centre
/// Massif union. The global height field still needs one connected propagation
/// domain, but reassigning Mountain columns would change their stable biome IDs
/// and every downstream semantic lookup. This visual mask is deliberately an
/// independent derived projection.
fn build_massif_visual_authority(
    plan: &SchematicPlanV1,
    layout: &ResolvedLayoutPlan,
    massif_owner_mask: &BTreeSet<HexCoord>,
    crystal_mask: &BTreeSet<HexCoord>,
) -> Result<MassifVisualAuthority, V3GenerationError> {
    if massif_owner_mask.is_empty() || !massif_owner_mask.is_disjoint(crystal_mask) {
        return Err(contract(
            "Grand V3 Massif ownership is empty or enters the exact Crystal site",
        ));
    }
    let mountain_connector_patches = plan
        .cells
        .iter()
        .filter(|cell| {
            cell.facts.surface == SurfaceKind::Land
                && cell.facts.landform == LandformKind::Mountain
                && cell.facts.overlays.is_empty()
        })
        .map(|cell| PatchId(u32::from(cell.id.get())))
        .collect::<BTreeSet<_>>();
    let mut allowed = massif_owner_mask.clone();
    for patch_id in mountain_connector_patches {
        let patch = layout.patches.get(&patch_id).ok_or_else(|| {
            contract(format!(
                "Grand V3 Massif connector patch {} disappeared from the layout",
                patch_id.0
            ))
        })?;
        allowed.extend(
            patch
                .mask
                .iter()
                .copied()
                .filter(|coord| !crystal_mask.contains(coord)),
        );
    }

    let mut components = fine_components(massif_owner_mask);
    let primary_index = components
        .iter()
        .enumerate()
        .max_by_key(|(_, component)| {
            (
                component.len(),
                Reverse(component.first().copied().unwrap_or(HexCoord::ORIGIN)),
            )
        })
        .map(|(index, _)| index)
        .ok_or_else(|| contract("Grand V3 Massif has no fine ownership component"))?;
    let mut connected_body = components.remove(primary_index);
    while !components.is_empty() {
        let goals = components
            .iter()
            .flat_map(|component| component.iter().copied())
            .collect::<BTreeSet<_>>();
        let path = shortest_path_between_sets(&allowed, &connected_body, &goals).ok_or_else(|| {
            contract(format!(
                "Crystal radius-32 claim split the seeded Massif into {} visual components with no overlay-free Mountain connector",
                components.len().saturating_add(1)
            ))
        })?;
        let destination = path
            .last()
            .copied()
            .ok_or_else(|| contract("Grand V3 Massif resolved an empty visual connector"))?;
        let destination_index = components
            .iter()
            .position(|component| component.contains(&destination))
            .ok_or_else(|| {
                contract("Grand V3 Massif connector missed every remaining component")
            })?;
        connected_body.extend(path);
        connected_body.extend(components.remove(destination_index));
    }
    if !massif_owner_mask.is_subset(&connected_body)
        || !connected(&connected_body)
        || !connected_body.is_disjoint(crystal_mask)
    {
        return Err(contract(
            "Grand V3 Massif visual connector did not preserve one Crystal-disjoint body",
        ));
    }
    let cells = plan
        .cells
        .iter()
        .map(|cell| (PatchId(u32::from(cell.id.get())), cell))
        .collect::<BTreeMap<_, _>>();
    let mut connector_owners = BTreeMap::new();
    for coord in connected_body.difference(massif_owner_mask).copied() {
        let owners = layout
            .patches
            .iter()
            .filter_map(|(owner, patch)| patch.mask.contains(&coord).then_some(*owner))
            .collect::<Vec<_>>();
        let [owner] = owners.as_slice() else {
            return Err(contract(format!(
                "Grand V3 Massif visual connector {coord:?} has {} layout owners",
                owners.len()
            )));
        };
        let cell = cells.get(owner).copied().ok_or_else(|| {
            contract(format!(
                "Grand V3 Massif visual connector owner {} has no schematic cell",
                owner.0
            ))
        })?;
        if cell.facts.surface != SurfaceKind::Land
            || cell.facts.landform != LandformKind::Mountain
            || !cell.facts.overlays.is_empty()
            || crystal_mask.contains(&coord)
        {
            return Err(contract(format!(
                "Grand V3 Massif visual connector {coord:?} is not overlay-free Mountain terrain outside Crystal"
            )));
        }
        connector_owners.insert(coord, *owner);
    }
    Ok(MassifVisualAuthority {
        visual_mask: connected_body,
        semantic_owner_mask: massif_owner_mask.clone(),
        connector_owners,
    })
}

fn fine_components(mask: &BTreeSet<HexCoord>) -> Vec<BTreeSet<HexCoord>> {
    let mut remaining = mask.clone();
    let mut components = Vec::new();
    while let Some(start) = remaining.pop_first() {
        let mut component = BTreeSet::from([start]);
        let mut queue = VecDeque::from([start]);
        while let Some(current) = queue.pop_front() {
            let mut neighbors = current.neighbors();
            neighbors.sort_unstable();
            for neighbor in neighbors {
                if remaining.remove(&neighbor) {
                    component.insert(neighbor);
                    queue.push_back(neighbor);
                }
            }
        }
        components.push(component);
    }
    components
}

fn shortest_path_between_sets(
    mask: &BTreeSet<HexCoord>,
    sources: &BTreeSet<HexCoord>,
    goals: &BTreeSet<HexCoord>,
) -> Option<Vec<HexCoord>> {
    if sources.is_empty() || goals.is_empty() || !sources.is_subset(mask) || !goals.is_subset(mask)
    {
        return None;
    }
    let mut parent = BTreeMap::<HexCoord, HexCoord>::new();
    let mut visited = sources.clone();
    let mut queue = sources.iter().copied().collect::<VecDeque<_>>();
    let destination = loop {
        let current = queue.pop_front()?;
        if goals.contains(&current) {
            break current;
        }
        let mut neighbors = current.neighbors();
        neighbors.sort_unstable();
        for neighbor in neighbors {
            if mask.contains(&neighbor) && visited.insert(neighbor) {
                parent.insert(neighbor, current);
                queue.push_back(neighbor);
            }
        }
    };
    let mut reversed = vec![destination];
    let mut current = destination;
    while !sources.contains(&current) {
        current = parent.get(&current).copied()?;
        reversed.push(current);
    }
    reversed.reverse();
    Some(reversed)
}

fn schematic_components(
    cells: &BTreeMap<SchematicCoord, LandmarkCell>,
) -> Result<Vec<BTreeSet<SchematicCoord>>, V3GenerationError> {
    let mut remaining = cells.keys().copied().collect::<BTreeSet<_>>();
    let mut components = Vec::new();
    while let Some(start) = remaining.pop_first() {
        let mut component = BTreeSet::from([start]);
        let mut queue = VecDeque::from([start]);
        while let Some(current) = queue.pop_front() {
            let neighbors = current
                .neighbors()
                .ok_or_else(|| contract("Grand V3 highland schematic adjacency overflowed"))?;
            for neighbor in neighbors {
                if remaining.remove(&neighbor) {
                    component.insert(neighbor);
                    queue.push_back(neighbor);
                }
            }
        }
        components.push(component);
    }
    components.sort_by_key(|component| component.iter().next().copied());
    Ok(components)
}

fn build_peak_field(
    cells: &BTreeMap<SchematicCoord, LandmarkCell>,
    components: &[BTreeSet<SchematicCoord>],
    mask: &BTreeSet<HexCoord>,
    profile: V3GrandV3BasicTerrainProfile,
    seed: u64,
) -> Result<(BTreeMap<HexCoord, Level>, Vec<BTreeSet<HexCoord>>), V3GenerationError> {
    if PEAK_SUMMIT_MIN <= CRYSTAL_ARCHITECTURE_TOP || PEAK_SUMMIT_MAX >= MASSIF_SUMMIT_MIN {
        return Err(contract(
            "Grand V3 peak hierarchy must remain above Crystal and below the massif crest",
        ));
    }
    let summit_span = PEAK_SUMMIT_MAX
        .saturating_sub(PEAK_SUMMIT_MIN)
        .saturating_add(1);
    let summit_by_cell = cells
        .iter()
        .map(|(schematic, cell)| {
            let summit = PEAK_SUMMIT_MIN.saturating_add(
                i32::try_from(
                    named_sample(seed, "grand_v3.highlands.peak_summits", cell.representative)
                        % u64::try_from(summit_span).unwrap_or(1),
                )
                .unwrap_or_default(),
            );
            (*schematic, summit)
        })
        .collect::<BTreeMap<_, _>>();

    let mut ridge_targets = BTreeMap::<HexCoord, Level>::new();
    let mut ridge_components = Vec::new();
    for component in components {
        let mut ridge = BTreeSet::new();
        for schematic in component {
            let cell = cells.get(schematic).ok_or_else(|| {
                contract("Grand V3 peak component references an absent authored cell")
            })?;
            let summit = summit_by_cell
                .get(schematic)
                .copied()
                .ok_or_else(|| contract("Grand V3 peak component omitted one summit level"))?;
            ridge.insert(cell.representative);
            update_max(&mut ridge_targets, cell.representative, summit);
            let neighbors = schematic
                .neighbors()
                .ok_or_else(|| contract("Grand V3 peak adjacency overflowed"))?;
            for neighbor in neighbors
                .into_iter()
                .filter(|neighbor| component.contains(neighbor) && schematic < neighbor)
            {
                let destination = cells.get(&neighbor).ok_or_else(|| {
                    contract("Grand V3 peak edge references an absent authored cell")
                })?;
                let destination_summit = summit_by_cell
                    .get(&neighbor)
                    .copied()
                    .ok_or_else(|| contract("Grand V3 peak edge omitted its destination summit"))?;
                let line = cell.representative.line_between(destination.representative);
                paint_interpolated_segment(
                    &line,
                    summit,
                    destination_summit,
                    mask,
                    &mut ridge_targets,
                );
                ridge.extend(line.into_iter().filter(|coord| mask.contains(coord)));
            }
        }
        if !connected(&ridge) {
            return Err(contract(
                "one authored Grand V3 peak chain did not produce a connected exact ridge",
            ));
        }
        ridge_components.push(ridge);
    }

    let influence = propagate_influence(
        mask,
        &ridge_targets,
        PEAK_RIDGE_SLOPE,
        profile.sharp_peak_bench_min,
    );
    let depths = boundary_depth(mask);
    let mut levels = BTreeMap::new();
    for coord in mask {
        let depth = depths.get(coord).copied().unwrap_or_default();
        let depth_level = i32::try_from(depth).unwrap_or(i32::MAX);
        let roughness = if depth > 1 {
            i32::try_from(named_sample(seed, "grand_v3.highlands.peak_chisel", *coord) % 7)
                .unwrap_or_default()
                .saturating_sub(3)
        } else {
            0
        };
        let shoulder = profile
            .sharp_peak_bench_min
            .saturating_add(depth_level.saturating_mul(2).min(12))
            .saturating_add(roughness)
            .clamp(profile.sharp_peak_bench_min, profile.sharp_peak_bench_max);
        // Boundary depth, rather than a per-cell radius, fades the connected
        // ridge into its neighbors and avoids cylindrical benches.
        let ridge = influence
            .get(coord)
            .copied()
            .unwrap_or(profile.sharp_peak_bench_min)
            .min(
                profile
                    .sharp_peak_bench_min
                    .saturating_add(depth_level.saturating_mul(7)),
            );
        levels.insert(*coord, shoulder.max(ridge).min(PEAK_SUMMIT_MAX));
    }
    // The interpolated chain is authored summit authority, not merely an
    // influence source for nearby shoulders.  Reapply its exact levels after
    // the boundary-depth clamp so the six coarse peaks in each chain remain one
    // physically connected high ridge rather than six isolated high pins joined
    // only by lower benches.
    for (coord, ridge_level) in &ridge_targets {
        levels.insert(*coord, *ridge_level);
    }
    for (schematic, summit) in summit_by_cell {
        let representative = cells
            .get(&schematic)
            .map(|cell| cell.representative)
            .ok_or_else(|| contract("Grand V3 peak summit lost its representative"))?;
        // Coarse centers are deliberately exact summit pins, while every cell
        // between them belongs to the same connected ridge interpolation.
        levels.insert(representative, summit);
    }
    Ok((levels, ridge_components))
}

fn build_peak_ridge_authority(
    layout: &ResolvedLayoutPlan,
    cells: &BTreeMap<SchematicCoord, LandmarkCell>,
    components: &[BTreeSet<SchematicCoord>],
    peak_levels: &BTreeMap<HexCoord, Level>,
    ridge_components: &[BTreeSet<HexCoord>],
) -> Result<PeakRidgeAuthority, V3GenerationError> {
    if components.len() != 2 || ridge_components.len() != components.len() {
        return Err(contract(format!(
            "Grand V3 peak authority requires two matching ridge components, found {}/{}",
            components.len(),
            ridge_components.len()
        )));
    }

    let mut authority_components = Vec::with_capacity(components.len());
    for (component, expected_spine) in components.iter().zip(ridge_components) {
        let mut patch_masks = BTreeMap::new();
        let mut summit_pins = BTreeMap::new();
        for schematic in component {
            let cell = cells.get(schematic).ok_or_else(|| {
                contract("Grand V3 peak authority references an absent locked cell")
            })?;
            let patch = layout.patches.get(&cell.patch).ok_or_else(|| {
                contract(format!(
                    "Grand V3 peak authority lost resolved patch {}",
                    cell.patch.0
                ))
            })?;
            if patch_masks.insert(cell.patch, patch.mask.clone()).is_some() {
                return Err(contract("Grand V3 peak authority assigned one patch twice"));
            }
            let summit = peak_levels
                .get(&cell.representative)
                .copied()
                .ok_or_else(|| contract("Grand V3 peak authority lost one seeded summit pin"))?;
            summit_pins.insert(cell.representative, summit);
        }
        let component_mask = patch_masks
            .values()
            .flat_map(|mask| mask.iter().copied())
            .collect::<BTreeSet<_>>();
        let expected_high_band = component_mask
            .iter()
            .copied()
            .filter_map(|coord| {
                peak_levels
                    .get(&coord)
                    .copied()
                    .filter(|level| *level >= PEAK_SUMMIT_MIN)
                    .map(|level| (coord, level))
            })
            .collect::<BTreeMap<_, _>>();
        let expected_high_coords = expected_high_band.keys().copied().collect::<BTreeSet<_>>();
        if patch_masks.len() != 6
            || summit_pins.len() != 6
            || !summit_pins
                .keys()
                .all(|coord| expected_high_band.contains_key(coord))
            || !expected_spine.is_subset(&component_mask)
            || !connected(&expected_high_coords)
            || patch_masks
                .values()
                .any(|mask| mask.is_disjoint(&expected_high_coords))
        {
            return Err(contract(
                "Grand V3 peak authority did not resolve one connected six-cell high ridge",
            ));
        }
        authority_components.push(PeakRidgeComponentAuthority {
            patch_masks,
            expected_high_band,
            summit_pins,
            authorized_route_grades: None,
        });
    }
    Ok(PeakRidgeAuthority {
        components: authority_components,
    })
}

fn build_massif_field(
    mask: &BTreeSet<HexCoord>,
    crest_owner_mask: &BTreeSet<HexCoord>,
    crystal_mask: &BTreeSet<HexCoord>,
    profile: V3GrandV3BasicTerrainProfile,
    seed: u64,
) -> Result<MassifField, V3GenerationError> {
    if !connected(mask) {
        return Err(contract(
            "Grand V3 massif fine mask must remain one connected mountain body",
        ));
    }
    let crystal_center = super::schematic::exact_hex_disk_center(crystal_mask, CRYSTAL_SITE_RADIUS)
        .ok_or_else(|| contract("Grand V3 massif requires the exact radius-32 Crystal site"))?;
    let depths = boundary_depth(mask);
    let centroid = integer_centroid(mask)?;
    let eligible_crests = mask
        .iter()
        .copied()
        .filter(|coord| {
            crest_owner_mask.contains(coord)
                && coord
                    .distance(crystal_center)
                    .saturating_sub(CRYSTAL_SITE_RADIUS)
                    >= CELL_PITCH.unsigned_abs() / 2
        })
        .collect::<BTreeSet<_>>();
    let crest = eligible_crests
        .iter()
        .copied()
        .max_by_key(|coord| {
            (
                depths.get(coord).copied().unwrap_or_default(),
                Reverse(coord.distance(centroid)),
                Reverse(named_sample(
                    seed,
                    "grand_v3.highlands.massif_crest",
                    *coord,
                )),
                Reverse(*coord),
            )
        })
        .ok_or_else(|| contract("Grand V3 massif cannot select a central non-Crystal crest"))?;
    let maximum_depth = depths.get(&crest).copied().unwrap_or_default();
    let minimum_terminal_depth = (maximum_depth / 2).max(4);
    let inset = eligible_crests
        .iter()
        .copied()
        .filter(|coord| depths.get(coord).copied().unwrap_or_default() >= minimum_terminal_depth)
        .collect::<BTreeSet<_>>();
    if inset.is_empty() {
        return Err(contract(
            "Grand V3 massif has no inset terrain for a connected central ridge",
        ));
    }
    let terminals = axis_extremes(&inset, seed);
    if terminals.len() < 3 {
        return Err(contract(
            "Grand V3 massif cannot resolve at least three separated ridge arms",
        ));
    }
    let summit_span = MASSIF_SUMMIT_MAX
        .saturating_sub(MASSIF_SUMMIT_MIN)
        .saturating_add(1);
    let summit = MASSIF_SUMMIT_MIN.saturating_add(
        i32::try_from(
            named_sample(seed, "grand_v3.highlands.massif_summit", crest)
                % u64::try_from(summit_span).unwrap_or(1),
        )
        .unwrap_or_default(),
    );
    let mut ridge_targets = BTreeMap::from([(crest, summit)]);
    let mut ridge = BTreeSet::from([crest]);
    for terminal in terminals {
        let path = shortest_path(mask, crest, terminal).ok_or_else(|| {
            contract(format!(
                "Grand V3 massif cannot connect crest {crest:?} to ridge arm {terminal:?}"
            ))
        })?;
        for coord in path {
            let target = summit
                .saturating_sub(i32::try_from(crest.distance(coord)).unwrap_or(i32::MAX))
                .max(MASSIF_RIDGE_FLOOR);
            update_max(&mut ridge_targets, coord, target);
            ridge.insert(coord);
        }
    }
    if !connected(&ridge) {
        return Err(contract(
            "Grand V3 massif ridge arms are not connected through their central crest",
        ));
    }
    let ridge_influence = propagate_influence(
        mask,
        &ridge_targets,
        MASSIF_RIDGE_SLOPE,
        profile.massif_floor,
    );
    let ridge_distance = distances_from(mask, ridge.iter().copied());
    if summit <= PEAK_SUMMIT_MAX || summit >= MAX_V3_LEVEL {
        return Err(contract(format!(
            "Grand V3 massif summit {summit} must remain above peaks and below the V3 ceiling"
        )));
    }
    Ok(MassifField {
        mask: mask.clone(),
        ridge_influence,
        ridge_distance,
        boundary_depth: depths,
        crest,
        summit,
        floor: profile.massif_floor,
    })
}

fn crystal_context(
    plan: &SchematicPlanV1,
    layout: &ResolvedLayoutPlan,
    profile: V3GrandV3BasicTerrainProfile,
) -> Result<CrystalContext, V3GenerationError> {
    let crystal_cells = plan
        .cells
        .iter()
        .filter(|cell| {
            cell.facts
                .overlays
                .contains(&SchematicFeature::CrystalAscent)
        })
        .collect::<Vec<_>>();
    let [crystal_cell] = crystal_cells.as_slice() else {
        return Err(contract(format!(
            "Grand V3 highlands require exactly one Crystal cell; found {}",
            crystal_cells.len()
        )));
    };
    let crystal_patch_id = PatchId(u32::from(crystal_cell.id.get()));
    let crystal_patch = layout
        .patches
        .get(&crystal_patch_id)
        .ok_or_else(|| contract("Grand V3 Crystal cell has no resolved radius-32 patch"))?;
    let center = schematic_to_world(crystal_cell.coord);
    let expected_site = 1_u32.saturating_add(
        3_u32
            .saturating_mul(CRYSTAL_SITE_RADIUS)
            .saturating_mul(CRYSTAL_SITE_RADIUS.saturating_add(1)),
    );
    if crystal_patch.mask.len() != usize::try_from(expected_site).unwrap_or(usize::MAX)
        || crystal_patch
            .mask
            .iter()
            .any(|coord| center.distance(*coord) > CRYSTAL_SITE_RADIUS)
    {
        return Err(contract(
            "Grand V3 Crystal claim is not the exact radius-32 authored site",
        ));
    }
    let exit_clearance = crystal_mantle_exit_clearance(
        &crystal_patch.mask,
        crystal_patch.rotation_turns,
        profile,
        &layout.footprint,
    )?;
    Ok(CrystalContext {
        schematic: crystal_cell.coord,
        center,
        mask: crystal_patch.mask.clone(),
        exit_clearance,
    })
}

fn build_crystal_mantle(
    plan: &SchematicPlanV1,
    layout: &ResolvedLayoutPlan,
    profile: V3GrandV3BasicTerrainProfile,
    crystal: &CrystalContext,
) -> Result<BTreeMap<HexCoord, Level>, V3GenerationError> {
    let outer_decay =
        i32::try_from(CRYSTAL_MANTLE_OUTER_RADIUS.saturating_sub(CRYSTAL_SITE_RADIUS + 1))
            .unwrap_or(i32::MAX)
            .saturating_mul(CRYSTAL_MANTLE_TAPER_PER_HEX);
    if CRYSTAL_MANTLE_INNER_LEVEL.saturating_sub(outer_decay) > profile.beach_level {
        return Err(contract(
            "Grand V3 Crystal mantle ends above the lowest canonical dry-land datum",
        ));
    }
    let mut mantle = BTreeMap::new();
    let mut required_inner_shoulders = BTreeSet::new();
    for cell in &plan.cells {
        if cell.facts.surface != SurfaceKind::Land
            || cell.facts.overlays.iter().any(|overlay| {
                matches!(
                    overlay,
                    SchematicFeature::MountainLake
                        | SchematicFeature::LakeIsland
                        | SchematicFeature::Waterfall
                )
            })
        {
            continue;
        }
        let patch_id = PatchId(u32::from(cell.id.get()));
        let patch = layout.patches.get(&patch_id).ok_or_else(|| {
            contract(format!(
                "Grand V3 mantle cell {} has no resolved patch",
                cell.id.get()
            ))
        })?;
        for coord in &patch.mask {
            let radius = crystal.center.distance(*coord);
            if radius <= CRYSTAL_SITE_RADIUS
                || radius > CRYSTAL_MANTLE_OUTER_RADIUS
                || crystal.exit_clearance.contains(coord)
            {
                continue;
            }
            if radius <= CRYSTAL_SITE_RADIUS + 2 {
                required_inner_shoulders.insert(*coord);
            }
            let decay = i32::try_from(radius.saturating_sub(CRYSTAL_SITE_RADIUS + 1))
                .unwrap_or(i32::MAX)
                .saturating_mul(CRYSTAL_MANTLE_TAPER_PER_HEX);
            let target = CRYSTAL_MANTLE_INNER_LEVEL.saturating_sub(decay);
            mantle.insert(*coord, target);
        }
    }
    if required_inner_shoulders.is_empty()
        || required_inner_shoulders.iter().any(|coord| {
            mantle
                .get(coord)
                .is_none_or(|level| *level <= CRYSTAL_ARCHITECTURE_TOP)
        })
    {
        return Err(contract(
            "Grand V3 inner Crystal mantle is incomplete or not prominent above exterior architecture",
        ));
    }
    Ok(mantle)
}

fn crystal_mantle_exit_clearance(
    crystal_mask: &BTreeSet<HexCoord>,
    rotation_turns: u8,
    profile: V3GrandV3BasicTerrainProfile,
    footprint: &BTreeSet<HexCoord>,
) -> Result<BTreeSet<HexCoord>, V3GenerationError> {
    let upper_rows = super::crystal_ascent::macro_upper_terminal_outward_rows(
        crystal_mask,
        rotation_turns,
        profile
            .crystal_base_level
            .saturating_add(profile.crystal_rise_levels),
        CRYSTAL_MANTLE_EXIT_CLEARANCE_DEPTH,
    )
    .map_err(|error| contract(error))?;
    Ok(upper_rows
        .into_iter()
        .flatten()
        .flat_map(|coord| coord.within_radius(CRYSTAL_MANTLE_EXIT_CLEARANCE_BUFFER))
        .filter(|coord| footprint.contains(coord))
        .collect())
}

/// Exact inner screen which later surface-route construction must preserve.
///
/// The lower tunnel may share these columns below ground without changing the
/// exposed cap.  Only the authored upper Frozen-Woods aperture is removed from
/// the screen; deriving it from the same rotated outward rows as mantle shaping
/// prevents route reservation and final validation from disagreeing.
pub(super) fn crystal_mantle_inner_screen(
    crystal_mask: &BTreeSet<HexCoord>,
    rotation_turns: u8,
    profile: V3GrandV3BasicTerrainProfile,
    footprint: &BTreeSet<HexCoord>,
) -> Result<BTreeSet<HexCoord>, V3GenerationError> {
    let center = super::schematic::exact_hex_disk_center(crystal_mask, CRYSTAL_SITE_RADIUS)
        .ok_or_else(|| contract("Grand V3 Crystal mantle cannot recover its exact site centre"))?;
    let clearance =
        crystal_mantle_exit_clearance(crystal_mask, rotation_turns, profile, footprint)?;
    Ok(center
        .within_radius(CRYSTAL_SITE_RADIUS.saturating_add(2))
        .into_iter()
        .filter(|coord| {
            (CRYSTAL_SITE_RADIUS.saturating_add(1)..=CRYSTAL_SITE_RADIUS.saturating_add(2))
                .contains(&center.distance(*coord))
                && footprint.contains(coord)
                && !clearance.contains(coord)
        })
        .collect())
}

fn integer_centroid(mask: &BTreeSet<HexCoord>) -> Result<HexCoord, V3GenerationError> {
    let count = i64::try_from(mask.len()).map_err(|_| contract("highland mask is too large"))?;
    if count == 0 {
        return Err(contract(
            "cannot find the centroid of an empty highland mask",
        ));
    }
    let q = mask
        .iter()
        .map(|coord| i64::from(coord.x()))
        .sum::<i64>()
        .checked_div(count)
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| contract("highland centroid q coordinate overflowed"))?;
    let r = mask
        .iter()
        .map(|coord| i64::from(coord.y()))
        .sum::<i64>()
        .checked_div(count)
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| contract("highland centroid r coordinate overflowed"))?;
    Ok(HexCoord::from_axial(q, r))
}

fn axis_extremes(mask: &BTreeSet<HexCoord>, seed: u64) -> BTreeSet<HexCoord> {
    let mut result = BTreeSet::new();
    for axis in 0..3 {
        for maximum in [false, true] {
            let selected = if maximum {
                mask.iter().copied().max_by_key(|coord| {
                    (
                        coord
                            .to_cubic_array()
                            .get(axis)
                            .copied()
                            .unwrap_or_default(),
                        Reverse(named_sample(seed, "grand_v3.highlands.massif_arms", *coord)),
                        Reverse(*coord),
                    )
                })
            } else {
                mask.iter().copied().min_by_key(|coord| {
                    (
                        coord
                            .to_cubic_array()
                            .get(axis)
                            .copied()
                            .unwrap_or_default(),
                        named_sample(seed, "grand_v3.highlands.massif_arms", *coord),
                        *coord,
                    )
                })
            };
            result.extend(selected);
        }
    }
    result
}

fn paint_interpolated_segment(
    line: &[HexCoord],
    start: Level,
    end: Level,
    mask: &BTreeSet<HexCoord>,
    targets: &mut BTreeMap<HexCoord, Level>,
) {
    let denominator = i64::try_from(line.len().saturating_sub(1))
        .unwrap_or(1)
        .max(1);
    for (index, coord) in line.iter().copied().enumerate() {
        if !mask.contains(&coord) {
            continue;
        }
        let index = i64::try_from(index).unwrap_or(i64::MAX).min(denominator);
        let numerator = i64::from(start)
            .saturating_mul(denominator.saturating_sub(index))
            .saturating_add(i64::from(end).saturating_mul(index));
        let level = i32::try_from(numerator / denominator).unwrap_or(start);
        update_max(targets, coord, level);
    }
}

fn update_max(targets: &mut BTreeMap<HexCoord, Level>, coord: HexCoord, level: Level) {
    targets
        .entry(coord)
        .and_modify(|current| *current = (*current).max(level))
        .or_insert(level);
}

fn propagate_influence(
    mask: &BTreeSet<HexCoord>,
    sources: &BTreeMap<HexCoord, Level>,
    slope: Level,
    floor: Level,
) -> BTreeMap<HexCoord, Level> {
    let mut result = sources.clone();
    let mut queue = sources
        .iter()
        .map(|(coord, level)| (*level, Reverse(*coord)))
        .collect::<BinaryHeap<_>>();
    while let Some((level, Reverse(coord))) = queue.pop() {
        if result.get(&coord).copied() != Some(level) || level <= floor {
            continue;
        }
        let next_level = level.saturating_sub(slope).max(floor);
        for neighbor in coord.neighbors() {
            if !mask.contains(&neighbor)
                || result
                    .get(&neighbor)
                    .is_some_and(|current| *current >= next_level)
            {
                continue;
            }
            result.insert(neighbor, next_level);
            queue.push((next_level, Reverse(neighbor)));
        }
    }
    result
}

fn boundary_depth(mask: &BTreeSet<HexCoord>) -> BTreeMap<HexCoord, u32> {
    let boundary = mask
        .iter()
        .copied()
        .filter(|coord| {
            coord
                .neighbors()
                .into_iter()
                .any(|neighbor| !mask.contains(&neighbor))
        })
        .collect::<Vec<_>>();
    distances_from(mask, boundary)
}

fn distances_from(
    mask: &BTreeSet<HexCoord>,
    sources: impl IntoIterator<Item = HexCoord>,
) -> BTreeMap<HexCoord, u32> {
    let mut distances = BTreeMap::new();
    let mut queue = VecDeque::new();
    for source in sources {
        if mask.contains(&source) && distances.insert(source, 0_u32).is_none() {
            queue.push_back(source);
        }
    }
    while let Some(current) = queue.pop_front() {
        let distance = distances.get(&current).copied().unwrap_or_default();
        for neighbor in current.neighbors() {
            if mask.contains(&neighbor) && !distances.contains_key(&neighbor) {
                distances.insert(neighbor, distance.saturating_add(1));
                queue.push_back(neighbor);
            }
        }
    }
    distances
}

fn shortest_path(
    mask: &BTreeSet<HexCoord>,
    start: HexCoord,
    end: HexCoord,
) -> Option<Vec<HexCoord>> {
    if !mask.contains(&start) || !mask.contains(&end) {
        return None;
    }
    let mut parent = BTreeMap::<HexCoord, HexCoord>::new();
    let mut visited = BTreeSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(current) = queue.pop_front() {
        if current == end {
            break;
        }
        for neighbor in current.neighbors() {
            if mask.contains(&neighbor) && visited.insert(neighbor) {
                parent.insert(neighbor, current);
                queue.push_back(neighbor);
            }
        }
    }
    if !visited.contains(&end) {
        return None;
    }
    let mut reversed = vec![end];
    let mut current = end;
    while current != start {
        current = parent.get(&current).copied()?;
        reversed.push(current);
    }
    reversed.reverse();
    Some(reversed)
}

fn connected(mask: &BTreeSet<HexCoord>) -> bool {
    let Some(start) = mask.iter().next().copied() else {
        return false;
    };
    distances_from(mask, [start]).len() == mask.len()
}

fn schematic_to_world(coord: SchematicCoord) -> HexCoord {
    HexCoord::from_axial(
        coord.q().saturating_mul(CELL_PITCH),
        coord.r().saturating_mul(CELL_PITCH),
    )
}

fn named_sample(seed: u64, stream: &str, coord: HexCoord) -> u64 {
    let mut state = 0xcbf2_9ce4_8422_2325_u64;
    for bytes in [
        seed.to_le_bytes().as_slice(),
        stream.as_bytes(),
        coord.x().to_le_bytes().as_slice(),
        coord.y().to_le_bytes().as_slice(),
    ] {
        for byte in bytes {
            state ^= u64::from(*byte);
            state = state.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    state
}

fn contract(reason: impl Into<String>) -> V3GenerationError {
    V3GenerationError::RecipeContract(reason.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{
        ProceduralV3Settings, V3LayoutSettings, V3SchematicLayoutSettings, V3SchematicTemplate,
        V3SchematicTerrainProfile, V3_SCHEMATIC_GRID_RADIUS,
    };

    fn reference_field() -> GrandHighlandField {
        let template = hex_schematic::grand_v3_reference_template().expect("template parses");
        let reference = hex_schematic::reference_plan(&template, 0).expect("reference validates");
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
        let mut layout = super::super::layout::resolve_layout(V3_SCHEMATIC_GRID_RADIUS, &settings)
            .expect("reference layout resolves");
        super::super::schematic_crystal::claim_site(&reference.plan, &mut layout, 22)
            .expect("Crystal site claim validates");
        GrandHighlandField::build(
            &reference.plan,
            &layout,
            V3GrandV3BasicTerrainProfile::canonical(),
        )
        .expect("reference highland field builds")
    }

    #[test]
    fn locked_peak_chains_become_connected_irregular_high_ridges() {
        let field = reference_field();
        assert_eq!(field.peak_authority.components.len(), 2);
        assert!(field
            .peak_authority
            .components
            .iter()
            .all(|component| connected(
                &component
                    .expected_high_band
                    .keys()
                    .copied()
                    .collect::<BTreeSet<_>>()
            )));
        assert!(field.peak_authority.components.iter().all(|component| {
            component.patch_masks.len() == 6
                && component.summit_pins.len() == 6
                && component.authorized_route_grades.is_none()
                && component.patch_masks.values().all(|mask| {
                    component
                        .expected_high_band
                        .keys()
                        .any(|coord| mask.contains(coord))
                })
        }));
        let ridge_levels = field
            .peak_authority
            .components
            .iter()
            .flat_map(|component| component.expected_high_band.values().copied())
            .collect::<BTreeSet<_>>();
        assert!(ridge_levels.iter().all(|level| *level >= PEAK_SUMMIT_MIN));
        assert!(ridge_levels.iter().all(|level| *level <= PEAK_SUMMIT_MAX));
        assert!(
            ridge_levels.len() >= 8,
            "peak ridge lost its irregular height profile"
        );
    }

    #[test]
    fn massif_crest_is_the_interior_world_high_point_away_from_crystal() {
        let field = reference_field();
        let crest_depth = field
            .massif
            .boundary_depth
            .get(&field.massif.crest)
            .copied()
            .unwrap_or_default();
        let deepest_non_crystal = field
            .massif
            .mask
            .iter()
            .filter(|coord| {
                !field.crystal_mask.contains(coord)
                    && coord
                        .neighbors()
                        .into_iter()
                        .all(|neighbor| !field.crystal_mask.contains(&neighbor))
            })
            .filter_map(|coord| field.massif.boundary_depth.get(coord).copied())
            .max()
            .unwrap_or_default();
        assert_eq!(crest_depth, deepest_non_crystal);
        assert!(!field.crystal_mask.contains(&field.massif.crest));
        assert!(field
            .massif
            .crest
            .neighbors()
            .into_iter()
            .all(|neighbor| !field.crystal_mask.contains(&neighbor)));
        assert!((MASSIF_SUMMIT_MIN..=MASSIF_SUMMIT_MAX).contains(&field.massif.summit));
        assert!(field.massif.summit > PEAK_SUMMIT_MAX);
        assert!(field.massif.summit < MAX_V3_LEVEL);
    }

    #[test]
    fn massif_has_a_connected_crest_and_nonflat_shoulders() {
        let field = reference_field();
        let baseline = V3GrandV3BasicTerrainProfile::canonical().high_core_level;
        assert_eq!(
            field.massif.resolve(field.massif.crest, baseline),
            field.massif.summit
        );
        let summit_cells = field
            .massif
            .mask
            .iter()
            .copied()
            .filter(|coord| field.massif.resolve(*coord, baseline) == field.massif.summit)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            summit_cells,
            BTreeSet::from([field.massif.crest]),
            "massif must have one decisive, maximally interior world high point"
        );
        let resolved = field
            .massif
            .mask
            .iter()
            .map(|coord| field.massif.resolve(*coord, baseline))
            .collect::<BTreeSet<_>>();
        assert!(
            resolved.len() >= 20,
            "massif collapsed into broad level bands"
        );
        assert!(resolved.contains(&baseline));
        assert!(resolved.iter().any(|level| *level > PEAK_SUMMIT_MAX));
    }

    #[test]
    fn massif_shared_field_keeps_the_deep_reference_owner_seam_gradual() {
        let field = reference_field();
        let first = HexCoord::from_axial(-66, -77);
        let second = HexCoord::from_axial(-65, -78);
        let first_level = field.massif.resolve(first, 84);
        let second_level = field.massif.resolve(second, 87);
        assert!(first_level > 84 && second_level > 87);
        assert_ne!(first_level - 84, second_level - 87);
        assert!(first_level.abs_diff(second_level) <= 6);
    }

    #[test]
    fn massif_crest_selection_enforces_the_same_crystal_separation_as_final_validation() {
        let massif_center = HexCoord::from_axial(50, 0);
        let mask = massif_center
            .within_radius(30)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let crystal_mask = HexCoord::ORIGIN
            .within_radius(CRYSTAL_SITE_RADIUS)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let crest_owner_mask = massif_center
            .within_radius(5)
            .into_iter()
            .filter(|coord| mask.contains(coord))
            .collect::<BTreeSet<_>>();
        let field = build_massif_field(
            &mask,
            &crest_owner_mask,
            &crystal_mask,
            V3GrandV3BasicTerrainProfile::canonical(),
            41,
        )
        .expect("a broad massif retains separated central crest candidates");
        let separation = crystal_mask
            .iter()
            .map(|crystal| field.crest.distance(*crystal))
            .min()
            .expect("Crystal fixture is nonempty");
        assert!(crest_owner_mask.contains(&field.crest));
        assert!(separation >= CELL_PITCH.unsigned_abs() / 2);
        let maximum_eligible_depth = crest_owner_mask
            .iter()
            .filter(|coord| {
                crystal_mask
                    .iter()
                    .map(|crystal| coord.distance(*crystal))
                    .min()
                    .is_some_and(|distance| distance >= CELL_PITCH.unsigned_abs() / 2)
            })
            .filter_map(|coord| field.boundary_depth.get(coord).copied())
            .max();
        assert_eq!(
            field.boundary_depth.get(&field.crest).copied(),
            maximum_eligible_depth
        );
    }

    #[test]
    fn exact_crystal_disk_formula_matches_set_distance() {
        let center = HexCoord::from_axial(17, -9);
        let crystal_mask = center
            .within_radius(CRYSTAL_SITE_RADIUS)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let recovered =
            super::super::schematic::exact_hex_disk_center(&crystal_mask, CRYSTAL_SITE_RADIUS)
                .expect("translated exact Crystal disk retains one exact centre");
        assert_eq!(recovered, center);

        let mut samples = center.within_radius(4).into_iter().collect::<BTreeSet<_>>();
        for distance in 0_i32..=64 {
            for (q, r) in [
                (distance, 0),
                (0, distance),
                (-distance, distance),
                (-distance, 0),
                (0, -distance),
                (distance, -distance),
                (distance / 2, distance),
                (-distance, distance / 2),
            ] {
                samples.insert(HexCoord::from_axial(
                    center.x().saturating_add(q),
                    center.y().saturating_add(r),
                ));
            }
        }
        for coord in samples {
            let exact_set_distance = crystal_mask
                .iter()
                .map(|crystal| coord.distance(*crystal))
                .min();
            assert_eq!(
                exact_set_distance,
                Some(
                    coord
                        .distance(recovered)
                        .saturating_sub(CRYSTAL_SITE_RADIUS)
                ),
                "exact disk distance formula drifted at {coord:?}"
            );
        }
    }

    #[test]
    fn massif_field_rejects_a_malformed_crystal_disk() {
        let mut crystal_mask = HexCoord::ORIGIN
            .within_radius(CRYSTAL_SITE_RADIUS)
            .into_iter()
            .collect::<BTreeSet<_>>();
        assert!(crystal_mask.remove(&HexCoord::ORIGIN));
        assert!(
            super::super::schematic::exact_hex_disk_center(&crystal_mask, CRYSTAL_SITE_RADIUS)
                .is_none()
        );

        let massif_center = HexCoord::from_axial(50, 0);
        let mask = massif_center
            .within_radius(30)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let crest_owner_mask = massif_center
            .within_radius(5)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let error = build_massif_field(
            &mask,
            &crest_owner_mask,
            &crystal_mask,
            V3GrandV3BasicTerrainProfile::canonical(),
            41,
        )
        .expect_err("malformed Crystal authority must fail before massif shaping");
        let V3GenerationError::RecipeContract(detail) = error else {
            panic!("malformed Crystal disk returned the wrong error: {error:?}");
        };
        assert!(detail.contains("exact radius-32 Crystal site"));
    }

    #[test]
    fn seed_175_connects_the_visual_massif_without_changing_biome_ownership() {
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
        let mut layout = super::super::layout::resolve_layout(V3_SCHEMATIC_GRID_RADIUS, &settings)
            .expect("seed 175 layout resolves");
        super::super::schematic_crystal::claim_site(&plan, &mut layout, 22)
            .expect("seed 175 Crystal site claim validates");
        let claimed_layout = layout.clone();
        let massif_patches = plan
            .cells
            .iter()
            .filter(|cell| cell.facts.landform == LandformKind::Massif)
            .map(|cell| PatchId(u32::from(cell.id.get())))
            .collect::<BTreeSet<_>>();
        let massif_owner_mask = union_masks(&layout, massif_patches.iter().copied())
            .expect("seed 175 has Massif ownership");
        assert!(
            fine_components(&massif_owner_mask).len() > 1,
            "seed 175 remains the defining Crystal-split Massif fixture"
        );

        let field =
            GrandHighlandField::build(&plan, &layout, V3GrandV3BasicTerrainProfile::canonical())
                .expect("seed 175 highland visual field builds");

        assert_eq!(
            layout, claimed_layout,
            "visual highland construction must not mutate biome ownership"
        );
        assert_eq!(
            field.massif_visual_authority.semantic_owner_mask,
            massif_owner_mask
        );
        assert_eq!(field.massif_visual_authority.visual_mask, field.massif.mask);
        assert!(connected(&field.massif.mask));
        assert!(massif_owner_mask.is_subset(&field.massif.mask));
        let connector = field
            .massif
            .mask
            .difference(&massif_owner_mask)
            .copied()
            .collect::<BTreeSet<_>>();
        assert!(
            !connector.is_empty(),
            "the split fixture must exercise a visual-only connector"
        );
        assert_eq!(
            field
                .massif_visual_authority
                .connector_owners
                .keys()
                .copied()
                .collect::<BTreeSet<_>>(),
            connector
        );
        for coord in &connector {
            assert!(!field.crystal_mask.contains(coord));
            let owner = layout
                .patches
                .iter()
                .find_map(|(owner, patch)| patch.mask.contains(coord).then_some(*owner))
                .expect("connector coordinate retains one layout owner");
            let cell = plan
                .cells
                .iter()
                .find(|cell| u32::from(cell.id.get()) == owner.0)
                .expect("connector owner has a schematic cell");
            assert_eq!(cell.facts.surface, SurfaceKind::Land);
            assert_eq!(cell.facts.landform, LandformKind::Mountain);
            assert!(cell.facts.overlays.is_empty());
            assert_eq!(
                field.massif_visual_authority.connector_owners.get(coord),
                Some(&owner)
            );
        }
        let probe_coord = connector
            .first()
            .copied()
            .expect("seed 175 connector has a deterministic first coordinate");
        let probe_owner = layout
            .patches
            .iter()
            .find_map(|(owner, patch)| patch.mask.contains(&probe_coord).then_some(*owner))
            .expect("connector probe retains one layout owner");
        let probe_cell = plan
            .cells
            .iter()
            .find(|cell| u32::from(cell.id.get()) == probe_owner.0)
            .expect("connector probe owner has a schematic cell");
        let mut application_probe = field.clone();
        // Isolate the visual-Massif projection: this defining connector also
        // happens to sit beneath Crystal's independent higher mantle.
        application_probe.crystal_mantle.remove(&probe_coord);
        application_probe
            .massif
            .boundary_depth
            .insert(probe_coord, 4);
        application_probe
            .massif
            .ridge_distance
            .insert(probe_coord, 0);
        application_probe
            .massif
            .ridge_influence
            .insert(probe_coord, 200);
        let baseline = 80;
        let expected = application_probe.massif.resolve(probe_coord, baseline);
        assert!(expected > baseline);
        assert_eq!(
            application_probe.resolve_surface_level(probe_cell, probe_coord, baseline),
            expected,
            "semantic Mountain connector coordinates must consume the visual Massif field"
        );
        let crest_owner = layout
            .patches
            .iter()
            .find_map(|(owner, patch)| patch.mask.contains(&field.massif.crest).then_some(*owner))
            .expect("Massif crest retains one layout owner");
        let crest_cell = plan
            .cells
            .iter()
            .find(|cell| u32::from(cell.id.get()) == crest_owner.0)
            .expect("Massif crest owner has a schematic cell");
        assert_eq!(crest_cell.facts.landform, LandformKind::Massif);
        assert!(massif_patches.contains(&crest_owner));
    }

    #[test]
    fn crystal_mantle_is_prominent_without_entering_the_authored_site_or_exits() {
        let field = reference_field();
        assert!(field
            .crystal_mantle
            .keys()
            .all(|coord| !field.crystal_mask.contains(coord)));
        assert!(field
            .crystal_mantle
            .keys()
            .all(|coord| !field.crystal_mantle_exit_clearance.contains(coord)));
        let inner = field
            .crystal_mantle
            .iter()
            .filter(|(coord, _)| field.crystal_center.distance(**coord) <= CRYSTAL_SITE_RADIUS + 2)
            .map(|(_, level)| *level)
            .collect::<Vec<_>>();
        assert!(!inner.is_empty());
        assert!(inner.iter().all(|level| *level > CRYSTAL_ARCHITECTURE_TOP));
    }

    #[test]
    fn crystal_mantle_tapers_below_the_lowest_dry_datum_before_its_outer_edge() {
        let field = reference_field();
        let profile = V3GrandV3BasicTerrainProfile::canonical();
        let outer = field
            .crystal_mantle
            .iter()
            .filter(|(coord, _)| {
                field.crystal_center.distance(**coord) == CRYSTAL_MANTLE_OUTER_RADIUS
            })
            .map(|(_, level)| *level)
            .collect::<Vec<_>>();
        assert!(!outer.is_empty());
        assert!(
            outer.iter().all(|level| *level <= profile.beach_level),
            "the mantle's implementation boundary must be hidden below every canonical dry-land surface"
        );
        assert!(field.crystal_mantle.iter().all(|(coord, level)| {
            coord.neighbors().into_iter().all(|neighbor| {
                field
                    .crystal_mantle
                    .get(&neighbor)
                    .is_none_or(|neighbor_level| {
                        level.abs_diff(*neighbor_level)
                            <= u32::try_from(CRYSTAL_MANTLE_TAPER_PER_HEX).unwrap_or(u32::MAX)
                    })
            })
        }));
    }

    #[test]
    fn crystal_mantle_clears_only_the_upper_exit_in_all_six_landmark_rotations() {
        let mask = HexCoord::ORIGIN
            .within_radius(CRYSTAL_SITE_RADIUS)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let footprint = HexCoord::ORIGIN
            .within_radius(CRYSTAL_MANTLE_OUTER_RADIUS + 4)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let profile = V3GrandV3BasicTerrainProfile::canonical();
        let mut rotated_clearances = BTreeSet::new();
        for rotation in 0..6 {
            let clearance = crystal_mantle_exit_clearance(&mask, rotation, profile, &footprint)
                .expect("rotated upper-exit clearance resolves");
            let lower = super::super::crystal_ascent::macro_lower_terminal_coords(
                &mask,
                rotation,
                profile.crystal_base_level,
            )
            .expect("rotated lower terminal resolves");
            let upper = super::super::crystal_ascent::macro_upper_terminal_outward_rows(
                &mask,
                rotation,
                profile
                    .crystal_base_level
                    .saturating_add(profile.crystal_rise_levels),
                CRYSTAL_MANTLE_EXIT_CLEARANCE_DEPTH,
            )
            .expect("rotated upper-exit rows resolve")
            .into_iter()
            .flatten()
            .collect::<BTreeSet<_>>();
            let expected = upper
                .iter()
                .copied()
                .flat_map(|coord| coord.within_radius(CRYSTAL_MANTLE_EXIT_CLEARANCE_BUFFER))
                .filter(|coord| footprint.contains(coord))
                .collect::<BTreeSet<_>>();
            assert_eq!(clearance, expected);
            assert!(lower.is_disjoint(&clearance));
            rotated_clearances.insert(clearance);
        }
        assert_eq!(rotated_clearances.len(), 6);
    }

    #[test]
    fn highland_field_is_stable_for_one_plan_and_seed() {
        assert_eq!(reference_field(), reference_field());
    }
}
