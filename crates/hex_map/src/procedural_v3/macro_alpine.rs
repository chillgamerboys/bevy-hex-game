//! Shared alpine landform planning for authored Macro worlds.
//!
//! Atomic macro cells are ownership units, not landform units. This planner builds
//! one world-space field across every ordinary Mountain instance and one separate
//! union-mask field for Deep Mountain. Consequently, a single-cell instance does
//! not receive its own centred cone and same-tier cell boundaries do not appear in
//! the height function.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_core::{HexCoord, Level};

use crate::settings::{
    MacroAxisSettings, MacroLayoutSettings, V3DeepMountainSettings, V3RecipeSettings,
};

use super::layout::{PatchId, ResolvedLayoutPlan};
use super::seed::{SeedStream, SeedStreams};

/// The otherwise-unused top value of Macro's six-bit namespace owns world fields.
const SHARED_ALPINE_STREAM_ID: u32 = 63;
const REGULAR_PEAK_COUNT: usize = 8;
const REGULAR_PEAK_SPACING: u32 = 15;
const DATUM_FADE_DEPTH: u32 = 6;
const SHARED_DATUM_BLEND_DEPTH: u32 = 8;
const DEEP_MOUNTAIN_MAX_LOCAL_STEP: Level = 4;

/// Complete shared alpine height field, partitioned back into logical instances.
pub(crate) type AlpineHeightField = BTreeMap<PatchId, BTreeMap<HexCoord, Level>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct TierKey {
    low: Level,
    high: Level,
    cap: Level,
    axis: MacroAxisSettings,
}

#[derive(Debug)]
struct TierShape {
    mask: BTreeSet<HexCoord>,
    low_distances: BTreeMap<HexCoord, u32>,
    high_distances: BTreeMap<HexCoord, u32>,
}

#[derive(Debug, Clone, Copy)]
struct PeakLobe {
    center: HexCoord,
    satellite: HexCoord,
    radius: u32,
    strength: Level,
}

/// Plans all Mountain and Deep Mountain surfaces from shared world-space fields.
///
/// Candidate worlds vary only the broad peaks and low-frequency undulation. The
/// canonical fallback uses its own stable namespace rather than reverting to one
/// centred peak per instance.
pub(crate) fn plan_alpine_height_field(
    layout: &ResolvedLayoutPlan,
    settings: &MacroLayoutSettings,
    candidate: Option<(u64, u8)>,
) -> Result<AlpineHeightField, String> {
    if layout.patches.len() != settings.instances.len() {
        return Err(format!(
            "Macro alpine settings contain {} instances but the resolved layout contains {} patches",
            settings.instances.len(),
            layout.patches.len()
        ));
    }
    let (world_seed, candidate_index) = candidate.unwrap_or((0x4d41_5353_4946, 0));
    let stream = SeedStreams::new(world_seed, candidate_index, SHARED_ALPINE_STREAM_ID)
        .stage("macro.alpine.world-field");

    let mut patch_tiers = BTreeMap::<PatchId, TierKey>::new();
    let mut tier_masks = BTreeMap::<TierKey, BTreeSet<HexCoord>>::new();
    let mut deep = None::<(PatchId, BTreeSet<HexCoord>, V3DeepMountainSettings, Level)>;
    for (index, instance) in settings.instances.iter().enumerate() {
        let patch_id = PatchId(u32::try_from(index).unwrap_or(u32::MAX));
        let patch = layout.patches.get(&patch_id).ok_or_else(|| {
            format!(
                "Macro alpine instance {:?} has no resolved patch",
                instance.name
            )
        })?;
        match &instance.recipe {
            V3RecipeSettings::Mountains(mountains) => {
                let tier = TierKey {
                    low: instance.elevation.low,
                    high: instance.elevation.high,
                    cap: mountains.base_level.saturating_add(mountains.relief),
                    axis: instance.elevation.grade_axis,
                };
                patch_tiers.insert(patch_id, tier);
                tier_masks
                    .entry(tier)
                    .or_default()
                    .extend(patch.mask.iter().copied());
            }
            V3RecipeSettings::DeepMountain(deep_settings) => {
                if deep.is_some() {
                    return Err(
                        "Macro alpine planning supports one Deep Mountain instance".to_owned()
                    );
                }
                deep = Some((
                    patch_id,
                    patch.mask.clone(),
                    *deep_settings,
                    instance.elevation.low,
                ));
            }
            _ => {}
        }
    }

    let tier_shapes = tier_masks
        .into_iter()
        .map(|(tier, mask)| {
            let (low_frontier, high_frontier) =
                tier_frontiers(layout, settings, &patch_tiers, tier, &mask);
            let low_frontier = frontier_or_extreme(&mask, low_frontier, tier.axis, false);
            let high_frontier = frontier_or_extreme(&mask, high_frontier, tier.axis, true);
            (
                tier,
                TierShape {
                    low_distances: distances_within(&mask, &low_frontier),
                    high_distances: distances_within(&mask, &high_frontier),
                    mask,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();

    let regular_mask = tier_shapes
        .values()
        .flat_map(|shape| shape.mask.iter().copied())
        .collect::<BTreeSet<_>>();
    let lobes = select_regular_lobes(&regular_mask, stream);
    let ridge = ridge_distances(&regular_mask, &lobes);
    let mut result = BTreeMap::new();
    for (patch_id, tier) in patch_tiers {
        let patch = layout
            .patches
            .get(&patch_id)
            .ok_or_else(|| format!("Macro alpine patch {} disappeared", patch_id.0))?;
        let shape = tier_shapes
            .get(&tier)
            .ok_or_else(|| format!("Macro alpine tier {tier:?} has no shared shape"))?;
        let levels = patch
            .mask
            .iter()
            .copied()
            .map(|coord| {
                let low_distance = shape.low_distances.get(&coord).copied().unwrap_or_default();
                let high_distance = shape
                    .high_distances
                    .get(&coord)
                    .copied()
                    .unwrap_or_default();
                let distance_sum = low_distance.saturating_add(high_distance).max(1);
                let datum = tier.low.saturating_add(
                    tier.high
                        .saturating_sub(tier.low)
                        .saturating_mul(i32::try_from(low_distance).unwrap_or(i32::MAX))
                        / i32::try_from(distance_sum).unwrap_or(i32::MAX).max(1),
                );
                let fade = low_distance.min(high_distance).min(DATUM_FADE_DEPTH);
                let landform = regular_landform(coord, &lobes, &ridge, stream);
                let faded_landform = landform
                    .saturating_mul(i32::try_from(fade).unwrap_or_default())
                    / i32::try_from(DATUM_FADE_DEPTH).unwrap_or(1);
                let level = datum
                    .saturating_add(faded_landform)
                    .clamp(tier.low, tier.cap);
                (coord, level)
            })
            .collect::<BTreeMap<_, _>>();
        result.insert(patch_id, levels);
    }

    if let Some((patch_id, mask, deep_settings, base_level)) = deep {
        result.insert(
            patch_id,
            deep_mountain_levels(&mask, deep_settings, base_level, stream)?,
        );
    }
    blend_shared_datums(layout, settings, &mut result)?;
    resolve_shared_alpine_seams(layout, settings, &mut result)?;
    smooth_deep_mountain_slopes(layout, settings, &mut result, stream)?;
    Ok(result)
}

/// Relaxes the massif between its exact perimeter datums and stepped summit.
///
/// The authored seams and summit crown are fixed constraints. Everything between
/// them retains the low-frequency union field unless an adjacent pair would form
/// a sharp voxel cliff, in which case the pair is moved toward a bounded grade.
fn smooth_deep_mountain_slopes(
    layout: &ResolvedLayoutPlan,
    settings: &MacroLayoutSettings,
    fields: &mut AlpineHeightField,
    stream: SeedStream<'_>,
) -> Result<(), String> {
    let Some((deep_index, _)) = settings
        .instances
        .iter()
        .enumerate()
        .find(|(_, instance)| matches!(instance.recipe, V3RecipeSettings::DeepMountain(_)))
    else {
        return Ok(());
    };
    let deep_id = PatchId(u32::try_from(deep_index).unwrap_or(u32::MAX));
    let mut fixed = BTreeSet::new();
    for edge in layout
        .shared_edges
        .values()
        .filter(|edge| edge.first.0 == deep_id || edge.second.0 == deep_id)
    {
        fixed.extend(edge.boundary_pairs.iter().map(|(first, second)| {
            if edge.first.0 == deep_id {
                *first
            } else {
                *second
            }
        }));
    }
    let Some(field) = fields.get_mut(&deep_id) else {
        return Err("Macro alpine Deep Mountain field disappeared before smoothing".to_owned());
    };
    let summit = field
        .iter()
        .max_by_key(|(coord, level)| (**level, Reverse(**coord)))
        .map(|(coord, _)| *coord)
        .ok_or_else(|| "Macro alpine Deep Mountain field is empty".to_owned())?;
    fixed.extend(
        summit
            .within_radius(3)
            .into_iter()
            .filter(|coord| field.contains_key(coord)),
    );
    let original = field.clone();
    let mut fixed_levels = fixed
        .iter()
        .filter_map(|coord| original.get(coord).copied().map(|level| (*coord, level)))
        .collect::<BTreeMap<_, _>>();
    let mut soft_anchors = original
        .iter()
        .filter(|(coord, _)| !fixed.contains(coord))
        .map(|(coord, level)| (*coord, *level))
        .collect::<Vec<_>>();
    soft_anchors.sort_unstable_by_key(|(coord, _)| (stream.sample_coord(*coord, 113), *coord));
    for (coord, level) in soft_anchors {
        let compatible = fixed_levels.iter().all(|(fixed_coord, fixed_level)| {
            let allowance = DEEP_MOUNTAIN_MAX_LOCAL_STEP.saturating_mul(
                Level::try_from(coord.distance(*fixed_coord)).unwrap_or(Level::MAX),
            );
            level.abs_diff(*fixed_level) <= u32::try_from(allowance).unwrap_or(u32::MAX)
        });
        if compatible {
            fixed_levels.insert(coord, level);
        }
    }
    for (coord, level) in field.iter_mut() {
        if let Some(fixed_level) = fixed_levels.get(coord).copied() {
            *level = fixed_level;
            continue;
        }
        let (lower, upper) = fixed_levels.iter().fold(
            (Level::MIN, Level::MAX),
            |(lower, upper), (fixed_coord, fixed_level)| {
                let allowance = DEEP_MOUNTAIN_MAX_LOCAL_STEP.saturating_mul(
                    Level::try_from(coord.distance(*fixed_coord)).unwrap_or(Level::MAX),
                );
                (
                    lower.max(fixed_level.saturating_sub(allowance)),
                    upper.min(fixed_level.saturating_add(allowance)),
                )
            },
        );
        if lower > upper {
            return Err(format!(
                "Macro Deep Mountain has incompatible slope bounds at {coord:?}: \
                 {lower}..={upper}"
            ));
        }
        *level = lower.saturating_add(upper.saturating_sub(lower) / 2);
    }
    let edges = field
        .keys()
        .copied()
        .flat_map(|coord| {
            coord
                .neighbors()
                .into_iter()
                .filter(move |neighbor| coord < *neighbor)
                .map(move |neighbor| (coord, neighbor))
        })
        .filter(|(_, neighbor)| field.contains_key(neighbor))
        .collect::<Vec<_>>();
    for (first, second) in edges {
        let first_level = field.get(&first).copied().unwrap_or_default();
        let second_level = field.get(&second).copied().unwrap_or_default();
        if first_level.abs_diff(second_level)
            > u32::try_from(DEEP_MOUNTAIN_MAX_LOCAL_STEP).unwrap_or_default()
        {
            return Err(format!(
                "Macro Deep Mountain slope solve left {first:?}@{first_level} -> \
                 {second:?}@{second_level}"
            ));
        }
    }
    Ok(())
}

/// Resolves every alpine ownership boundary as one global constrained graph.
///
/// Same-tier targets retain the shared world-space field. Authored tier and massif
/// transitions target their resolved seam datums, while ordinary route approaches
/// are the only hard constraints. Treating the complete graph together matters at
/// three-instance corners: averaging each patch's boundary independently can give
/// the two ends of one physical lane different answers.
fn resolve_shared_alpine_seams(
    layout: &ResolvedLayoutPlan,
    settings: &MacroLayoutSettings,
    fields: &mut AlpineHeightField,
) -> Result<(), String> {
    let original = fields
        .iter()
        .flat_map(|(patch_id, field)| {
            field
                .iter()
                .map(move |(coord, level)| (*coord, (*patch_id, *level)))
        })
        .collect::<BTreeMap<_, _>>();
    let mut neighbors = BTreeMap::<HexCoord, BTreeSet<HexCoord>>::new();
    let mut authored_targets = BTreeMap::<HexCoord, BTreeSet<Level>>::new();
    let mut route_targets = BTreeMap::<HexCoord, Level>::new();
    let mut deep_minima = BTreeMap::<PatchId, Level>::new();

    for edge in layout.shared_edges.values() {
        let Some(first) = settings
            .instances
            .get(usize::try_from(edge.first.0 .0).unwrap_or(usize::MAX))
        else {
            continue;
        };
        let Some(second) = settings
            .instances
            .get(usize::try_from(edge.second.0 .0).unwrap_or(usize::MAX))
        else {
            continue;
        };
        if !is_alpine_recipe(&first.recipe) || !is_alpine_recipe(&second.recipe) {
            continue;
        }
        for (patch_id, instance) in [(edge.first.0, first), (edge.second.0, second)] {
            if matches!(instance.recipe, V3RecipeSettings::DeepMountain(_)) {
                deep_minima
                    .entry(patch_id)
                    .and_modify(|level| *level = (*level).min(edge.elevation.preferred))
                    .or_insert(edge.elevation.preferred);
            }
        }
        for (first_coord, second_coord) in &edge.boundary_pairs {
            neighbors
                .entry(*first_coord)
                .or_default()
                .insert(*second_coord);
            neighbors
                .entry(*second_coord)
                .or_default()
                .insert(*first_coord);
            if !same_mountain_profile(first, second) {
                authored_targets
                    .entry(*first_coord)
                    .or_default()
                    .insert(edge.elevation.preferred);
                authored_targets
                    .entry(*second_coord)
                    .or_default()
                    .insert(edge.elevation.preferred);
            }
        }
        for coord in edge
            .walker
            .ports
            .iter()
            .flat_map(|port| port.first_approach.iter().chain(&port.second_approach))
            .copied()
        {
            if route_targets
                .insert(coord, edge.elevation.preferred)
                .is_some_and(|existing| existing != edge.elevation.preferred)
            {
                return Err(format!(
                    "Macro alpine route approach {coord:?} requires conflicting seam datums"
                ));
            }
        }
    }
    if neighbors.is_empty() {
        return Ok(());
    }
    // The seam is a two-dimensional boundary manifold, not a bag of unrelated
    // cross-patch pairs. Adjacent boundary columns owned by the same patch must
    // participate in the same Lipschitz solve or a corner correction can leave a
    // sharp step along the inside edge of that patch.
    let boundary_coords = neighbors.keys().copied().collect::<Vec<_>>();
    for coord in boundary_coords {
        let owner = original.get(&coord).map(|(patch_id, _)| *patch_id);
        for neighbor in coord.neighbors() {
            if owner.is_some()
                && owner == original.get(&neighbor).map(|(patch_id, _)| *patch_id)
                && neighbors.contains_key(&neighbor)
            {
                neighbors.entry(coord).or_default().insert(neighbor);
                neighbors.entry(neighbor).or_default().insert(coord);
            }
        }
    }
    let fixed_routes = route_targets
        .into_iter()
        .filter(|(coord, _)| neighbors.contains_key(coord))
        .collect::<BTreeMap<_, _>>();

    let targets = neighbors
        .keys()
        .copied()
        .map(|coord| {
            let target = authored_targets.get(&coord).map_or_else(
                || {
                    original
                        .get(&coord)
                        .map(|(_, level)| *level)
                        .unwrap_or_default()
                },
                |samples| mean_levels(samples.iter().copied()),
            );
            (coord, target)
        })
        .collect::<BTreeMap<_, _>>();

    // Exact route datums induce the feasible interval at every connected seam
    // coordinate. Massif-front datums remain soft targets so a route crossing a
    // three-instance corner can taper instead of becoming an impossible 34/41
    // discontinuity.
    let mut lower = neighbors
        .keys()
        .copied()
        .map(|coord| (coord, Level::MIN))
        .collect::<BTreeMap<_, _>>();
    let mut upper = neighbors
        .keys()
        .copied()
        .map(|coord| (coord, Level::MAX))
        .collect::<BTreeMap<_, _>>();
    let mut lower_pending = VecDeque::new();
    let mut upper_pending = VecDeque::new();
    for (coord, (patch_id, _)) in &original {
        let Some(minimum) = deep_minima.get(patch_id).copied() else {
            continue;
        };
        if neighbors.contains_key(coord) {
            lower.insert(*coord, minimum);
            lower_pending.push_back(*coord);
        }
    }
    for (fixed_coord, fixed_level) in &fixed_routes {
        lower.insert(*fixed_coord, *fixed_level);
        upper.insert(*fixed_coord, *fixed_level);
        lower_pending.push_back(*fixed_coord);
        upper_pending.push_back(*fixed_coord);
    }
    while let Some(coord) = lower_pending.pop_front() {
        let candidate = lower
            .get(&coord)
            .copied()
            .unwrap_or(Level::MIN)
            .saturating_sub(1);
        for neighbor in neighbors.get(&coord).into_iter().flatten() {
            if lower
                .get(neighbor)
                .is_none_or(|current| candidate > *current)
            {
                lower.insert(*neighbor, candidate);
                lower_pending.push_back(*neighbor);
            }
        }
    }
    while let Some(coord) = upper_pending.pop_front() {
        let candidate = upper
            .get(&coord)
            .copied()
            .unwrap_or(Level::MAX)
            .saturating_add(1);
        for neighbor in neighbors.get(&coord).into_iter().flatten() {
            if upper
                .get(neighbor)
                .is_none_or(|current| candidate < *current)
            {
                upper.insert(*neighbor, candidate);
                upper_pending.push_back(*neighbor);
            }
        }
    }
    if let Some((coord, minimum, maximum)) = lower.iter().find_map(|(coord, minimum)| {
        let maximum = upper.get(coord).copied().unwrap_or(Level::MAX);
        (*minimum > maximum).then_some((*coord, *minimum, maximum))
    }) {
        return Err(format!(
            "Macro alpine route constraints are infeasible at {coord:?}: {minimum}>{maximum}"
        ));
    }

    let mut resolved = targets
        .into_iter()
        .map(|(coord, target)| {
            let minimum = lower.get(&coord).copied().unwrap_or(Level::MIN);
            let maximum = upper.get(&coord).copied().unwrap_or(Level::MAX);
            (coord, target.clamp(minimum, maximum))
        })
        .collect::<BTreeMap<_, _>>();
    let mut pending = neighbors.keys().copied().collect::<VecDeque<_>>();
    while let Some(coord) = pending.pop_front() {
        let maximum_neighbor = resolved
            .get(&coord)
            .copied()
            .unwrap_or_default()
            .saturating_add(1);
        for neighbor in neighbors.get(&coord).into_iter().flatten() {
            if resolved
                .get(neighbor)
                .is_some_and(|level| *level > maximum_neighbor)
            {
                resolved.insert(*neighbor, maximum_neighbor);
                pending.push_back(*neighbor);
            }
        }
    }
    for (coord, minimum) in &lower {
        if let Some(level) = resolved.get_mut(coord) {
            *level = (*level).max(*minimum);
        }
    }
    for (coord, fixed_level) in &fixed_routes {
        if resolved.get(coord) != Some(fixed_level) {
            return Err(format!(
                "Macro alpine seam solver moved route approach {coord:?} away from level {fixed_level}"
            ));
        }
    }
    taper_resolved_seam_corrections(&original, &resolved, fields)?;
    for (coord, level) in &resolved {
        let Some((patch_id, _)) = original.get(coord).copied() else {
            return Err(format!(
                "Macro alpine seam coordinate {coord:?} has no owning height field"
            ));
        };
        let Some(field) = fields.get_mut(&patch_id) else {
            return Err(format!(
                "Macro alpine seam coordinate {coord:?} references missing patch {}",
                patch_id.0
            ));
        };
        field.insert(*coord, *level);
    }
    clamp_inward_seam_neighbors(fields, &original, &resolved)?;
    Ok(())
}

fn clamp_inward_seam_neighbors(
    fields: &mut AlpineHeightField,
    original: &BTreeMap<HexCoord, (PatchId, Level)>,
    resolved: &BTreeMap<HexCoord, Level>,
) -> Result<(), String> {
    for (patch_id, field) in fields {
        let local_boundary = resolved
            .iter()
            .filter(|(coord, _)| {
                original
                    .get(coord)
                    .is_some_and(|(owner, _)| owner == patch_id)
            })
            .map(|(coord, level)| (*coord, *level))
            .collect::<BTreeMap<_, _>>();
        let inward = field
            .keys()
            .copied()
            .filter(|coord| !local_boundary.contains_key(coord))
            .filter_map(|coord| {
                let adjacent = coord
                    .neighbors()
                    .into_iter()
                    .filter_map(|neighbor| local_boundary.get(&neighbor).copied())
                    .collect::<Vec<_>>();
                (!adjacent.is_empty()).then_some((coord, adjacent))
            })
            .collect::<Vec<_>>();
        for (coord, adjacent) in inward {
            let minimum = adjacent
                .iter()
                .copied()
                .map(|level| level.saturating_sub(3))
                .max()
                .unwrap_or(Level::MIN);
            let maximum = adjacent
                .iter()
                .copied()
                .map(|level| level.saturating_add(3))
                .min()
                .unwrap_or(Level::MAX);
            if minimum > maximum {
                return Err(format!(
                    "Macro alpine inward seam constraints are infeasible at {coord:?}: {minimum}>{maximum}"
                ));
            }
            if let Some(level) = field.get_mut(&coord) {
                *level = (*level).clamp(minimum, maximum);
            }
        }
    }
    Ok(())
}

fn taper_resolved_seam_corrections(
    original: &BTreeMap<HexCoord, (PatchId, Level)>,
    resolved: &BTreeMap<HexCoord, Level>,
    fields: &mut AlpineHeightField,
) -> Result<(), String> {
    let mut corrections = BTreeMap::<PatchId, BTreeMap<HexCoord, Level>>::new();
    for (coord, resolved_level) in resolved {
        let Some((patch_id, original_level)) = original.get(coord).copied() else {
            return Err(format!(
                "Macro alpine seam correction {coord:?} has no owning height field"
            ));
        };
        let correction = resolved_level.saturating_sub(original_level);
        if correction != 0 {
            corrections
                .entry(patch_id)
                .or_default()
                .insert(*coord, correction);
        }
    }
    for (patch_id, sources) in corrections {
        let Some(field) = fields.get_mut(&patch_id) else {
            return Err(format!(
                "Macro alpine seam correction references missing patch {}",
                patch_id.0
            ));
        };
        let before = field.clone();
        for (coord, level) in field.iter_mut() {
            let distance = sources
                .keys()
                .map(|source| coord.distance(*source))
                .min()
                .unwrap_or(u32::MAX);
            if distance == 0 || distance > SHARED_DATUM_BLEND_DEPTH {
                continue;
            }
            let nearest = sources
                .iter()
                .filter(|(source, _)| coord.distance(**source) == distance)
                .map(|(_, correction)| *correction)
                .collect::<Vec<_>>();
            let correction = mean_levels(nearest.into_iter());
            let fade = Level::try_from(SHARED_DATUM_BLEND_DEPTH.saturating_sub(distance))
                .unwrap_or_default();
            let depth = Level::try_from(SHARED_DATUM_BLEND_DEPTH).unwrap_or(1);
            let tapered = correction.saturating_mul(fade) / depth.max(1);
            *level = before
                .get(coord)
                .copied()
                .unwrap_or(*level)
                .saturating_add(tapered);
        }
    }
    Ok(())
}

fn mean_levels(levels: impl Iterator<Item = Level>) -> Level {
    let (sum, count) = levels.fold((0_i64, 0_i64), |(sum, count), level| {
        (
            sum.saturating_add(i64::from(level)),
            count.saturating_add(1),
        )
    });
    Level::try_from(sum / count.max(1)).unwrap_or_default()
}

/// Tapers authored tier transitions and massif buttresses into both incident
/// landforms. Exact seam columns retain the shared datum, but it is no longer a
/// one-column post-process against an otherwise unrelated height field.
fn blend_shared_datums(
    layout: &ResolvedLayoutPlan,
    settings: &MacroLayoutSettings,
    fields: &mut AlpineHeightField,
) -> Result<(), String> {
    let mut sources = BTreeMap::<PatchId, BTreeMap<HexCoord, BTreeSet<Level>>>::new();
    for edge in layout.shared_edges.values() {
        let Some(first) = settings
            .instances
            .get(usize::try_from(edge.first.0 .0).unwrap_or(usize::MAX))
        else {
            continue;
        };
        let Some(second) = settings
            .instances
            .get(usize::try_from(edge.second.0 .0).unwrap_or(usize::MAX))
        else {
            continue;
        };
        if !is_alpine_recipe(&first.recipe)
            || !is_alpine_recipe(&second.recipe)
            || same_mountain_profile(first, second)
        {
            continue;
        }
        for (first_coord, second_coord) in &edge.boundary_pairs {
            sources
                .entry(edge.first.0)
                .or_default()
                .entry(*first_coord)
                .or_default()
                .insert(edge.elevation.preferred);
            sources
                .entry(edge.second.0)
                .or_default()
                .entry(*second_coord)
                .or_default()
                .insert(edge.elevation.preferred);
        }
    }

    for (patch_id, patch_sources) in sources {
        let Some(field) = fields.get_mut(&patch_id) else {
            return Err(format!(
                "Macro alpine seam blend references missing patch {}",
                patch_id.0
            ));
        };
        let source_levels = patch_sources
            .into_iter()
            .map(|(coord, samples)| {
                let count = i64::try_from(samples.len()).unwrap_or(1).max(1);
                let total = samples.into_iter().map(i64::from).sum::<i64>();
                (coord, Level::try_from(total / count).unwrap_or_default())
            })
            .collect::<BTreeMap<_, _>>();
        let original = field.clone();
        for (coord, level) in field.iter_mut() {
            let distance = source_levels
                .keys()
                .map(|source| coord.distance(*source))
                .min()
                .unwrap_or(u32::MAX);
            if distance > SHARED_DATUM_BLEND_DEPTH {
                continue;
            }
            let nearest = source_levels
                .iter()
                .filter(|(source, _)| coord.distance(**source) == distance)
                .map(|(_, target)| *target)
                .collect::<Vec<_>>();
            let count = i64::try_from(nearest.len()).unwrap_or(1).max(1);
            let target = Level::try_from(nearest.into_iter().map(i64::from).sum::<i64>() / count)
                .unwrap_or(*level);
            let retained = i64::from(distance.min(SHARED_DATUM_BLEND_DEPTH));
            let authored = i64::from(SHARED_DATUM_BLEND_DEPTH).saturating_sub(retained);
            let original_level = original.get(coord).copied().unwrap_or(*level);
            *level = Level::try_from(
                (i64::from(target)
                    .saturating_mul(authored)
                    .saturating_add(i64::from(original_level).saturating_mul(retained))
                    .saturating_add(i64::from(SHARED_DATUM_BLEND_DEPTH / 2)))
                    / i64::from(SHARED_DATUM_BLEND_DEPTH),
            )
            .unwrap_or(original_level);
        }
    }
    Ok(())
}

fn is_alpine_recipe(recipe: &V3RecipeSettings) -> bool {
    matches!(
        recipe,
        V3RecipeSettings::Mountains(_) | V3RecipeSettings::DeepMountain(_)
    )
}

fn same_mountain_profile(
    first: &crate::settings::MacroBiomeInstanceSettings,
    second: &crate::settings::MacroBiomeInstanceSettings,
) -> bool {
    matches!(first.recipe, V3RecipeSettings::Mountains(_))
        && matches!(second.recipe, V3RecipeSettings::Mountains(_))
        && first.elevation == second.elevation
}

fn tier_frontiers(
    layout: &ResolvedLayoutPlan,
    settings: &MacroLayoutSettings,
    patch_tiers: &BTreeMap<PatchId, TierKey>,
    tier: TierKey,
    tier_mask: &BTreeSet<HexCoord>,
) -> (BTreeSet<HexCoord>, BTreeSet<HexCoord>) {
    let mut low = BTreeSet::new();
    let mut high = BTreeSet::new();
    for edge in layout.shared_edges.values() {
        for (local_id, other_id, local_coord) in
            edge.boundary_pairs
                .iter()
                .flat_map(|(first_coord, second_coord)| {
                    [
                        (edge.first.0, edge.second.0, *first_coord),
                        (edge.second.0, edge.first.0, *second_coord),
                    ]
                })
        {
            if patch_tiers.get(&local_id) != Some(&tier) || !tier_mask.contains(&local_coord) {
                continue;
            }
            let Some(other) = settings
                .instances
                .get(usize::try_from(other_id.0).unwrap_or(usize::MAX))
            else {
                continue;
            };
            if other.elevation.high <= tier.low {
                low.insert(local_coord);
            }
            if other.elevation.low >= tier.high {
                high.insert(local_coord);
            }
        }
    }
    (low, high)
}

fn frontier_or_extreme(
    mask: &BTreeSet<HexCoord>,
    frontier: BTreeSet<HexCoord>,
    axis: MacroAxisSettings,
    high: bool,
) -> BTreeSet<HexCoord> {
    if !frontier.is_empty() {
        return frontier;
    }
    let extreme = mask
        .iter()
        .map(|coord| grade_projection(*coord, axis))
        .reduce(|first, second| {
            if high {
                first.max(second)
            } else {
                first.min(second)
            }
        })
        .unwrap_or_default();
    mask.iter()
        .copied()
        .filter(|coord| grade_projection(*coord, axis) == extreme)
        .collect()
}

fn select_regular_lobes(mask: &BTreeSet<HexCoord>, stream: SeedStream<'_>) -> Vec<PeakLobe> {
    if mask.is_empty() {
        return Vec::new();
    }
    let boundary = boundary_depths(mask);
    let maximum_depth = boundary.values().copied().max().unwrap_or_default();
    let preferred_depth = maximum_depth.saturating_mul(2) / 5;
    let mut candidates = mask
        .iter()
        .copied()
        .filter(|coord| boundary.get(coord).copied().unwrap_or_default() >= preferred_depth)
        .collect::<Vec<_>>();
    candidates.sort_unstable_by_key(|coord| (stream.sample_coord(*coord, 0), *coord));
    candidates.reverse();

    let mut centers = Vec::new();
    for coord in candidates.iter().copied() {
        if centers
            .iter()
            .all(|existing: &HexCoord| existing.distance(coord) >= REGULAR_PEAK_SPACING)
        {
            centers.push(coord);
            if centers.len() == REGULAR_PEAK_COUNT {
                break;
            }
        }
    }
    if centers.len() < REGULAR_PEAK_COUNT {
        for coord in candidates {
            if !centers.contains(&coord)
                && centers
                    .iter()
                    .all(|existing| existing.distance(coord) >= REGULAR_PEAK_SPACING / 2)
            {
                centers.push(coord);
                if centers.len() == REGULAR_PEAK_COUNT {
                    break;
                }
            }
        }
    }

    centers
        .into_iter()
        .enumerate()
        .map(|(index, center)| {
            let salt = u64::try_from(index).unwrap_or(u64::MAX);
            let radius = 13_u32.saturating_add(
                u32::try_from(stream.sample(salt.saturating_mul(3)) % 10).unwrap_or_default(),
            );
            let strength = 8_i32.saturating_add(
                i32::try_from(stream.sample(salt.saturating_mul(3).saturating_add(1)) % 7)
                    .unwrap_or_default(),
            );
            let direction =
                usize::try_from(stream.sample(salt.saturating_mul(3).saturating_add(2)) % 6)
                    .unwrap_or_default();
            let satellite_target = offset(
                center,
                direction,
                radius.saturating_div(3).saturating_add(2),
            );
            let satellite = mask
                .iter()
                .copied()
                .min_by_key(|coord| (coord.distance(satellite_target), *coord))
                .unwrap_or(center);
            PeakLobe {
                center,
                satellite,
                radius,
                strength,
            }
        })
        .collect()
}

fn regular_landform(
    coord: HexCoord,
    lobes: &[PeakLobe],
    ridge: &BTreeMap<HexCoord, u32>,
    stream: SeedStream<'_>,
) -> Level {
    let mut strongest = 0;
    let mut second = 0;
    for lobe in lobes {
        let main = tapered_bonus(coord.distance(lobe.center), lobe.radius, lobe.strength);
        let satellite = tapered_bonus(
            coord.distance(lobe.satellite),
            lobe.radius.saturating_mul(2) / 3,
            lobe.strength.saturating_mul(2) / 3,
        );
        let combined = main.max(satellite).saturating_add(main.min(satellite) / 3);
        if combined > strongest {
            second = strongest;
            strongest = combined;
        } else if combined > second {
            second = combined;
        }
    }
    let ridge_bonus = match ridge.get(&coord).copied().unwrap_or(u32::MAX) {
        0 => 3,
        1 => 2,
        2 => 1,
        _ => 0,
    };
    strongest
        .saturating_add(second / 4)
        .saturating_add(ridge_bonus)
        .saturating_add(low_frequency_variation(coord, stream))
}

fn ridge_distances(mask: &BTreeSet<HexCoord>, lobes: &[PeakLobe]) -> BTreeMap<HexCoord, u32> {
    let mut ordered = lobes.iter().map(|lobe| lobe.center).collect::<Vec<_>>();
    ordered.sort_unstable_by_key(|coord| (coord.x(), coord.y(), coord.z()));
    let mut ridge = BTreeSet::new();
    for pair in ordered.windows(2) {
        let Some(start) = pair.first().copied() else {
            continue;
        };
        let Some(goal) = pair.get(1).copied() else {
            continue;
        };
        if let Some(path) = shortest_path(mask, start, goal) {
            ridge.extend(path);
        }
    }
    distances_within(mask, &ridge)
}

fn deep_mountain_levels(
    mask: &BTreeSet<HexCoord>,
    settings: V3DeepMountainSettings,
    base_level: Level,
    stream: SeedStream<'_>,
) -> Result<BTreeMap<HexCoord, Level>, String> {
    if mask.is_empty() {
        return Err("Deep Mountain has no resolved columns".to_owned());
    }
    let depths = boundary_depths(mask);
    let maximum_depth = depths.values().copied().max().unwrap_or_default().max(1);
    let summit = asymmetric_summit(mask, &depths, maximum_depth, stream);
    let shoulder_direction = usize::try_from(stream.sample(91) % 6).unwrap_or_default();
    let shoulder_target = offset(
        summit,
        shoulder_direction,
        maximum_depth.saturating_div(2).max(5),
    );
    let shoulder_focus = mask
        .iter()
        .copied()
        .filter(|coord| depths.get(coord).copied().unwrap_or_default() >= maximum_depth / 3)
        .min_by_key(|coord| (coord.distance(shoulder_target), Reverse(*coord)))
        .unwrap_or(summit);
    // Let the shoulder continue rising across the complete union depth. Reaching
    // its cap two thirds of the way inward produced a broad tabletop, especially
    // once the formerly two-mask-wide summit influence was added on top. The
    // tighter lobes retain one rear-biased high core while leaving most of the
    // five-cell wedge in the broad 60-76 shoulder band.
    let summit_radius = maximum_depth.saturating_sub(2).max(16);
    let shoulder_radius = maximum_depth.max(14);
    let outer_shoulder_depth = maximum_depth.clamp(1, 6);
    let inner_shoulder_depth = maximum_depth.saturating_sub(outer_shoulder_depth).max(1);
    let summit_level = settings.summit_level.min(settings.hard_cap);
    let subordinate_cap = summit_level.saturating_sub(1);

    Ok(mask
        .iter()
        .copied()
        .map(|coord| {
            let depth = depths.get(&coord).copied().unwrap_or_default();
            let broad_shoulder = 14_i32
                .saturating_mul(i32::try_from(depth.min(outer_shoulder_depth)).unwrap_or(i32::MAX))
                / i32::try_from(outer_shoulder_depth).unwrap_or(1).max(1)
                + 12_i32.saturating_mul(
                    i32::try_from(depth.saturating_sub(outer_shoulder_depth)).unwrap_or(i32::MAX),
                ) / i32::try_from(inner_shoulder_depth).unwrap_or(1).max(1);
            let fade = depth.min(8);
            let summit_distance = coord.distance(summit);
            let dominant = tapered_bonus(summit_distance, summit_radius, 22)
                .saturating_mul(i32::try_from(fade).unwrap_or_default())
                / 8;
            let asymmetric_lobe = tapered_bonus(coord.distance(shoulder_focus), shoulder_radius, 6)
                .saturating_mul(i32::try_from(fade).unwrap_or_default())
                / 8;
            let variation = low_frequency_variation(coord, stream)
                .saturating_mul(i32::try_from(fade).unwrap_or_default())
                / 8;
            let mut level = base_level
                .saturating_add(broad_shoulder)
                .saturating_add(dominant.max(asymmetric_lobe))
                .saturating_add(dominant.min(asymmetric_lobe) / 5)
                .saturating_add(variation)
                .clamp(base_level, settings.hard_cap);
            if coord == summit {
                level = summit_level;
            } else {
                level = level.min(subordinate_cap);
                // The dominant lobe supplies the broad high core. These narrow
                // bands only guarantee that the authored apex is supported by a
                // small stepped crown instead of becoming one forced vertical
                // column. Each shell retains two possible levels, so the shared
                // field and low-frequency variation still break radial symmetry.
                let crown_band = match summit_distance {
                    1 => Some((
                        summit_level.saturating_sub(3),
                        summit_level.saturating_sub(2),
                    )),
                    2 => Some((
                        summit_level.saturating_sub(5),
                        summit_level.saturating_sub(4),
                    )),
                    3 => Some((
                        summit_level.saturating_sub(7),
                        summit_level.saturating_sub(6),
                    )),
                    _ => None,
                };
                if let Some((minimum, maximum)) = crown_band {
                    let minimum = minimum.max(base_level).min(settings.hard_cap);
                    let maximum = maximum.max(minimum).min(settings.hard_cap);
                    level = level.clamp(minimum, maximum);
                }
            }
            (coord, level)
        })
        .collect())
}

fn asymmetric_summit(
    mask: &BTreeSet<HexCoord>,
    depths: &BTreeMap<HexCoord, u32>,
    maximum_depth: u32,
    stream: SeedStream<'_>,
) -> HexCoord {
    let min_x = mask.iter().map(|coord| coord.x()).min().unwrap_or_default();
    let max_x = mask.iter().map(|coord| coord.x()).max().unwrap_or(min_x);
    let min_y = mask.iter().map(|coord| coord.y()).min().unwrap_or_default();
    let max_y = mask.iter().map(|coord| coord.y()).max().unwrap_or(min_y);
    // Bias the dominant summit toward the rear of the five-cell wedge while the
    // depth constraint keeps it off the exposed world perimeter.
    let x_percent =
        74_i32.saturating_add(i32::try_from(stream.sample(87) % 17).unwrap_or_default());
    let y_percent =
        24_i32.saturating_add(i32::try_from(stream.sample(88) % 23).unwrap_or_default());
    let target = HexCoord::from_axial(
        min_x.saturating_add(max_x.saturating_sub(min_x).saturating_mul(x_percent) / 100),
        min_y.saturating_add(max_y.saturating_sub(min_y).saturating_mul(y_percent) / 100),
    );
    let minimum_depth = maximum_depth.saturating_mul(4) / 5;
    mask.iter()
        .copied()
        .filter(|coord| depths.get(coord).copied().unwrap_or_default() >= minimum_depth)
        .min_by_key(|coord| {
            (
                coord.distance(target),
                Reverse(depths.get(coord).copied().unwrap_or_default()),
                Reverse(stream.sample_coord(*coord, 97)),
            )
        })
        .unwrap_or_else(|| {
            mask.iter()
                .copied()
                .max_by_key(|coord| {
                    (
                        depths.get(coord).copied().unwrap_or_default(),
                        Reverse(*coord),
                    )
                })
                .unwrap_or(HexCoord::ORIGIN)
        })
}

fn tapered_bonus(distance: u32, radius: u32, strength: Level) -> Level {
    if distance >= radius || radius == 0 {
        return 0;
    }
    strength.saturating_mul(i32::try_from(radius - distance).unwrap_or_default())
        / i32::try_from(radius).unwrap_or(1)
}

fn low_frequency_variation(coord: HexCoord, stream: SeedStream<'_>) -> Level {
    let phase_a = i32::try_from(stream.sample(41) % 19).unwrap_or_default();
    let phase_b = i32::try_from(stream.sample(42) % 29).unwrap_or_default();
    let phase_c = i32::try_from(stream.sample(43) % 37).unwrap_or_default();
    let first = triangle_wave(
        coord.x().saturating_mul(2).saturating_add(coord.y()),
        phase_a,
        19,
        2,
    );
    let second = triangle_wave(
        coord.y().saturating_mul(2).saturating_sub(coord.x()),
        phase_b,
        29,
        2,
    );
    let third = triangle_wave(
        coord.x().saturating_add(coord.y().saturating_mul(3)),
        phase_c,
        37,
        1,
    );
    (first + second + third) / 2
}

fn triangle_wave(value: i32, phase: i32, period: i32, amplitude: i32) -> i32 {
    let wrapped = value.saturating_add(phase).rem_euclid(period);
    let half = period / 2;
    let distance = wrapped.abs_diff(half);
    amplitude.saturating_sub(
        amplitude
            .saturating_mul(2)
            .saturating_mul(i32::try_from(distance).unwrap_or_default())
            / half.max(1),
    )
}

fn offset(origin: HexCoord, direction: usize, distance: u32) -> HexCoord {
    let directions: [[i32; 3]; 6] = [
        [1, 0, -1],
        [1, -1, 0],
        [0, -1, 1],
        [-1, 0, 1],
        [-1, 1, 0],
        [0, 1, -1],
    ];
    let [dx, dy, dz] = directions.get(direction).copied().unwrap_or([1, 0, -1]);
    let distance = i32::try_from(distance).unwrap_or(i32::MAX);
    HexCoord::new_cubic(
        origin.x().saturating_add(dx.saturating_mul(distance)),
        origin.y().saturating_add(dy.saturating_mul(distance)),
        origin.z().saturating_add(dz.saturating_mul(distance)),
    )
}

fn grade_projection(coord: HexCoord, axis: MacroAxisSettings) -> i32 {
    let [x, y, z] = coord.to_cubic_array();
    match axis {
        MacroAxisSettings::East => x.saturating_sub(z),
        MacroAxisSettings::SouthEast => y.saturating_sub(z),
        MacroAxisSettings::SouthWest => y.saturating_sub(x),
        MacroAxisSettings::West => z.saturating_sub(x),
        MacroAxisSettings::NorthWest => z.saturating_sub(y),
        MacroAxisSettings::NorthEast => x.saturating_sub(y),
    }
}

fn boundary_depths(mask: &BTreeSet<HexCoord>) -> BTreeMap<HexCoord, u32> {
    let boundary = mask
        .iter()
        .copied()
        .filter(|coord| {
            coord
                .neighbors()
                .into_iter()
                .any(|neighbor| !mask.contains(&neighbor))
        })
        .collect::<BTreeSet<_>>();
    distances_within(mask, &boundary)
}

fn distances_within(
    mask: &BTreeSet<HexCoord>,
    sources: &BTreeSet<HexCoord>,
) -> BTreeMap<HexCoord, u32> {
    let mut distances = BTreeMap::new();
    let mut pending = VecDeque::new();
    for source in sources.iter().copied().filter(|coord| mask.contains(coord)) {
        distances.insert(source, 0_u32);
        pending.push_back(source);
    }
    while let Some(coord) = pending.pop_front() {
        let next = distances
            .get(&coord)
            .copied()
            .unwrap_or_default()
            .saturating_add(1);
        for neighbor in coord.neighbors() {
            if mask.contains(&neighbor) && !distances.contains_key(&neighbor) {
                distances.insert(neighbor, next);
                pending.push_back(neighbor);
            }
        }
    }
    distances
}

fn shortest_path(
    mask: &BTreeSet<HexCoord>,
    start: HexCoord,
    goal: HexCoord,
) -> Option<Vec<HexCoord>> {
    let mut previous = BTreeMap::from([(start, start)]);
    let mut pending = VecDeque::from([start]);
    while let Some(coord) = pending.pop_front() {
        if coord == goal {
            break;
        }
        for neighbor in coord.neighbors() {
            if mask.contains(&neighbor) && !previous.contains_key(&neighbor) {
                previous.insert(neighbor, coord);
                pending.push_back(neighbor);
            }
        }
    }
    if !previous.contains_key(&goal) {
        return None;
    }
    let mut path = vec![goal];
    let mut current = goal;
    while current != start {
        current = *previous.get(&current)?;
        path.push(current);
    }
    path.reverse();
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::super::layout::resolve_layout;
    use super::*;
    use crate::settings::{MapSettings, ProceduralSettings, TerrainSettings, V3LayoutSettings};

    const MOUNTAIN_RANGE_RON: &str =
        include_str!("../../../../assets/config/worlds/procedural-mountain-range.ron");

    fn mountain_range() -> (MacroLayoutSettings, ResolvedLayoutPlan) {
        let map: MapSettings = ron::from_str(MOUNTAIN_RANGE_RON).expect("Mountain Range parses");
        let TerrainSettings::Procedural(ProceduralSettings::V3(settings)) = map.terrain else {
            panic!("Mountain Range should use V3");
        };
        let V3LayoutSettings::Macro(macro_settings) = settings.layout else {
            panic!("Mountain Range should use Macro");
        };
        let layout = resolve_layout(
            77,
            &crate::settings::ProceduralV3Settings {
                layout: V3LayoutSettings::Macro(macro_settings.clone()),
            },
        )
        .expect("Mountain Range resolves");
        (macro_settings, layout)
    }

    #[test]
    fn mountain_range_massif_cells_form_a_compact_cluster() {
        let (settings, _) = mountain_range();
        let deep = settings
            .instances
            .iter()
            .find(|instance| matches!(instance.recipe, V3RecipeSettings::DeepMountain(_)))
            .expect("one Deep Mountain instance");
        let cells = deep
            .cells
            .iter()
            .map(|cell| (cell.x, cell.y, cell.z))
            .collect::<BTreeSet<_>>();

        assert_eq!(
            cells,
            BTreeSet::from([
                (2, -2, 0),
                (2, -1, -1),
                (3, -2, -1),
                (3, -1, -2),
                (3, 0, -3),
            ])
        );
        let internal_adjacencies = deep
            .cells
            .iter()
            .filter_map(|cell| HexCoord::try_new_cubic(cell.x, cell.y, cell.z))
            .map(|cell| {
                cell.neighbors()
                    .into_iter()
                    .filter(|neighbor| {
                        deep.cells.iter().any(|other| {
                            HexCoord::try_new_cubic(other.x, other.y, other.z) == Some(*neighbor)
                        })
                    })
                    .count()
            })
            .sum::<usize>()
            / 2;
        assert_eq!(
            internal_adjacencies, 6,
            "the five cells should form a connected three-back/two-front wedge"
        );
    }

    #[test]
    fn shared_field_is_deterministic_varied_and_has_one_dominant_massif() {
        let (settings, layout) = mountain_range();
        let first = plan_alpine_height_field(&layout, &settings, Some((1_592_598_566, 3)))
            .expect("shared alpine field");
        let second = plan_alpine_height_field(&layout, &settings, Some((1_592_598_566, 3)))
            .expect("deterministic shared alpine field");
        assert_eq!(first, second);

        for edge in layout.shared_edges.values() {
            let first_instance = settings
                .instances
                .get(usize::try_from(edge.first.0 .0).expect("first alpine instance id"))
                .expect("first alpine instance");
            let second_instance = settings
                .instances
                .get(usize::try_from(edge.second.0 .0).expect("second alpine instance id"))
                .expect("second alpine instance");
            if !is_alpine_recipe(&first_instance.recipe)
                || !is_alpine_recipe(&second_instance.recipe)
            {
                continue;
            }
            let first_field = first.get(&edge.first.0).expect("first alpine field");
            let second_field = first.get(&edge.second.0).expect("second alpine field");
            for (first_coord, second_coord) in &edge.boundary_pairs {
                let first_level = first_field
                    .get(first_coord)
                    .copied()
                    .expect("first alpine boundary level");
                let second_level = second_field
                    .get(second_coord)
                    .copied()
                    .expect("second alpine boundary level");
                assert!(
                    first_level.abs_diff(second_level) <= 1,
                    "shared alpine boundary {}<->{} jumps from {first_coord:?}@{first_level} to {second_coord:?}@{second_level}",
                    first_instance.name,
                    second_instance.name,
                );
            }
        }

        let alpine_columns = settings
            .instances
            .iter()
            .enumerate()
            .filter(|(_, instance)| {
                matches!(
                    instance.recipe,
                    V3RecipeSettings::Mountains(_) | V3RecipeSettings::DeepMountain(_)
                )
            })
            .map(|(index, _)| {
                layout
                    .patches
                    .get(&PatchId(u32::try_from(index).unwrap_or(u32::MAX)))
                    .map_or(0, |patch| patch.mask.len())
            })
            .sum::<usize>();
        assert_eq!(
            first.values().map(BTreeMap::len).sum::<usize>(),
            alpine_columns
        );

        let deep_index = settings
            .instances
            .iter()
            .position(|instance| matches!(instance.recipe, V3RecipeSettings::DeepMountain(_)))
            .expect("one Deep Mountain");
        let deep = first
            .get(&PatchId(u32::try_from(deep_index).unwrap_or(u32::MAX)))
            .expect("Deep Mountain field");
        let summits = deep
            .iter()
            .filter(|(_, level)| **level == 96)
            .map(|(coord, _)| *coord)
            .collect::<Vec<_>>();
        assert_eq!(
            summits.len(),
            1,
            "the massif should have one dominant summit"
        );
        let summit = summits
            .first()
            .copied()
            .expect("the massif should retain its dominant summit");
        let minimum_x = deep.keys().map(|coord| coord.x()).min().unwrap_or_default();
        let maximum_x = deep
            .keys()
            .map(|coord| coord.x())
            .max()
            .unwrap_or(minimum_x);
        assert!(
            summit.x().saturating_sub(minimum_x).saturating_mul(2)
                >= maximum_x.saturating_sub(minimum_x),
            "the dominant summit {summit:?} should sit in the rear half of the massif"
        );
        let deep_mask = deep.keys().copied().collect::<BTreeSet<_>>();
        let deep_depths = boundary_depths(&deep_mask);
        let maximum_depth = deep_depths.values().copied().max().unwrap_or_default();
        assert!(
            deep_depths.get(&summit).copied().unwrap_or_default()
                >= maximum_depth.saturating_mul(4) / 5,
            "the rear summit must remain deep enough in the union mask to support a crown"
        );
        let crown = deep
            .iter()
            .filter(|(coord, _)| coord.distance(summit) <= 3)
            .map(|(coord, level)| (*coord, *level))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(crown.len(), 37, "the radius-three crown should be complete");
        assert!(
            crown.values().copied().collect::<BTreeSet<_>>().len() >= 4,
            "the crown should contain several stepped levels rather than one plateau"
        );
        for (coord, level) in &crown {
            for neighbor in coord
                .neighbors()
                .into_iter()
                .filter(|neighbor| crown.contains_key(neighbor))
            {
                let neighbor_level = crown
                    .get(&neighbor)
                    .copied()
                    .expect("filtered crown neighbor should have a level");
                assert!(
                    level.abs_diff(neighbor_level) <= 3,
                    "summit crown jumps from {coord:?}@{level} to {neighbor:?}@{neighbor_level}"
                );
            }
        }
        let shoulder_count = deep
            .values()
            .filter(|level| (60..=76).contains(*level))
            .count();
        assert!(
            shoulder_count.saturating_mul(100) >= deep.len().saturating_mul(30),
            "the compact massif should retain broad shoulders, got {shoulder_count}/{} columns",
            deep.len()
        );

        let deep_id = PatchId(u32::try_from(deep_index).unwrap_or(u32::MAX));
        let mut lower_buttress_edges = 0_usize;
        let mut upper_front_edges = 0_usize;
        for edge in layout
            .shared_edges
            .values()
            .filter(|edge| edge.first.0 == deep_id || edge.second.0 == deep_id)
        {
            if edge.elevation.preferred == 41 {
                lower_buttress_edges = lower_buttress_edges.saturating_add(1);
            } else if edge.elevation.preferred == 48 {
                upper_front_edges = upper_front_edges.saturating_add(1);
            } else {
                assert!(
                    matches!(edge.elevation.preferred, 41 | 48),
                    "Deep Mountain resolved unexpected seam datum {}",
                    edge.elevation.preferred
                );
            }
            for (patch_id, boundary_coords) in [
                (
                    edge.first.0,
                    edge.boundary_pairs
                        .iter()
                        .map(|pair| pair.0)
                        .collect::<Vec<_>>(),
                ),
                (
                    edge.second.0,
                    edge.boundary_pairs
                        .iter()
                        .map(|pair| pair.1)
                        .collect::<Vec<_>>(),
                ),
            ] {
                let patch = layout.patches.get(&patch_id).expect("alpine seam patch");
                let levels = first.get(&patch_id).expect("alpine seam height field");
                for boundary in boundary_coords {
                    let boundary_level = levels
                        .get(&boundary)
                        .copied()
                        .expect("alpine seam boundary level");
                    for inward in boundary
                        .neighbors()
                        .into_iter()
                        .filter(|neighbor| patch.mask.contains(neighbor))
                    {
                        let inward_level = levels
                            .get(&inward)
                            .copied()
                            .expect("alpine inward taper level");
                        assert!(
                            boundary_level.abs_diff(inward_level) <= 3,
                            "alpine seam taper jumped from {boundary:?}@{boundary_level} to {inward:?}@{inward_level}"
                        );
                    }
                }
            }
        }
        assert!(lower_buttress_edges >= 2);
        assert!(upper_front_edges >= 2);

        let mut local_peak_offsets = BTreeSet::new();
        let mut off_center = 0;
        for (index, instance) in settings.instances.iter().enumerate() {
            if !matches!(instance.recipe, V3RecipeSettings::Mountains(_)) {
                continue;
            }
            let patch_id = PatchId(u32::try_from(index).unwrap_or(u32::MAX));
            let patch = layout.patches.get(&patch_id).expect("Mountain patch");
            let levels = first.get(&patch_id).expect("Mountain levels");
            let peak = levels
                .iter()
                .max_by_key(|(coord, level)| (**level, Reverse(**coord)))
                .map(|(coord, _)| *coord)
                .expect("Mountain patch has a peak");
            let count = i32::try_from(patch.mask.len()).unwrap_or(1).max(1);
            let mean_x = patch.mask.iter().map(|coord| coord.x()).sum::<i32>() / count;
            let mean_y = patch.mask.iter().map(|coord| coord.y()).sum::<i32>() / count;
            let center = HexCoord::from_axial(mean_x, mean_y);
            let offset = (
                peak.x().saturating_sub(mean_x),
                peak.y().saturating_sub(mean_y),
            );
            local_peak_offsets.insert(offset);
            off_center += usize::from(peak.distance(center) >= 4);
        }
        assert!(
            off_center >= 7,
            "most Mountain maxima should be visibly off-centre"
        );
        assert!(
            local_peak_offsets.len() >= 7,
            "shared peaks must not repeat one local cell-centre signature"
        );
    }
}
