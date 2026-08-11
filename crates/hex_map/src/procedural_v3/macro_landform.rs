//! Whole-world rolling land for temperate Macro biomes.
//!
//! Atomic Macro masks are ownership, not landform units. Forest, Prairie, Hills,
//! and Waterfall instances therefore share one world-space height field. Internal
//! instance edges never become noise or grade boundaries; the caller extracts a
//! patch-local view only after the complete temperate field has been shaped.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_core::{HexCoord, Level};

use crate::settings::{MacroAxisSettings, MacroLayoutSettings, V3RecipeSettings, MAX_V3_LEVEL};

use super::layout::{LayoutKind, PatchId, ResolvedLayoutPlan};
use super::seed::{SeedStream, SeedStreams};
use super::V3GenerationError;

const MIN_SURFACE_LEVEL: Level = 4;
const SHARED_LANDFORM_NAMESPACE: u32 = 63;
const CANONICAL_LANDFORM_SEED: u64 = 0x6d61_6372_6f2d_6c61;
const LOBE_COUNT: u64 = 9;
const MIN_LOBE_RADIUS: u32 = 22;
const LOBE_RADIUS_SPAN: u64 = 21;
const RELIEF_LIMIT: Level = 2;
const SMOOTHING_PASSES: u8 = 3;
const SMOOTHING_SELF_WEIGHT: i64 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ElevationKey {
    low: Level,
    high: Level,
    axis: MacroAxisSettings,
}

impl ElevationKey {
    const fn from_instance(instance: &crate::settings::MacroBiomeInstanceSettings) -> Self {
        Self {
            low: instance.elevation.low,
            high: instance.elevation.high,
            axis: instance.elevation.grade_axis,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ReliefLobe {
    center: HexCoord,
    radius: u32,
    amplitude: Level,
}

/// Plans one continuous base field for all temperate Macro land instances.
///
/// The returned map contains only Forest, Prairie, Hills, and Waterfall patch IDs.
/// Coastal and alpine recipes deliberately remain under their dedicated planners.
pub(crate) fn plan_base_surface_levels(
    layout: &ResolvedLayoutPlan,
    settings: &MacroLayoutSettings,
    candidate: Option<(u64, u8)>,
) -> Result<BTreeMap<PatchId, BTreeMap<HexCoord, Level>>, V3GenerationError> {
    if layout.kind != LayoutKind::Macro || layout.patches.len() != settings.instances.len() {
        return Err(landform_error(
            "requires a resolved Macro layout whose patch order matches its instances",
        ));
    }

    let mut groups = BTreeMap::<ElevationKey, BTreeSet<HexCoord>>::new();
    let mut key_by_patch = BTreeMap::<PatchId, ElevationKey>::new();
    for (patch_id, patch) in &layout.patches {
        let instance_index = usize::try_from(patch_id.0).map_err(|error| {
            landform_error(format!(
                "cannot resolve instance index for patch {}: {error}",
                patch_id.0
            ))
        })?;
        let Some(instance) = settings.instances.get(instance_index) else {
            return Err(landform_error(format!(
                "has no instance settings for patch {}",
                patch_id.0
            )));
        };
        if !uses_shared_landform(&instance.recipe) {
            continue;
        }
        if patch.mask.is_empty() {
            return Err(landform_error(format!(
                "temperate patch {} has an empty mask",
                patch_id.0
            )));
        }
        let key = ElevationKey::from_instance(instance);
        groups
            .entry(key)
            .or_default()
            .extend(patch.mask.iter().copied());
        key_by_patch.insert(*patch_id, key);
    }

    if groups.is_empty() {
        return Ok(BTreeMap::new());
    }
    let complete_mask = groups
        .values()
        .flat_map(|mask| mask.iter().copied())
        .collect::<BTreeSet<_>>();
    let stream = shared_stream(candidate);
    let relief = low_frequency_relief(&complete_mask, stream)?;

    let mut levels = BTreeMap::new();
    let mut bounds = BTreeMap::new();
    for (key, mask) in &groups {
        let group_levels = directional_group_levels(mask, *key, &relief)?;
        for (coord, level) in group_levels {
            if levels.insert(coord, level).is_some() {
                return Err(landform_error(format!(
                    "temperate elevation groups overlap at {coord:?}"
                )));
            }
            bounds.insert(coord, (key.low, key.high));
        }
    }

    smooth_complete_field(&complete_mask, &bounds, &mut levels);
    enforce_local_continuity(&complete_mask, &bounds, &mut levels)?;

    let mut by_patch = BTreeMap::new();
    for (patch_id, key) in key_by_patch {
        let Some(patch) = layout.patches.get(&patch_id) else {
            return Err(landform_error(format!(
                "resolved temperate patch {} disappeared during planning",
                patch_id.0
            )));
        };
        let patch_levels = patch
            .mask
            .iter()
            .copied()
            .map(|coord| {
                let level = levels.get(&coord).copied().ok_or_else(|| {
                    landform_error(format!(
                        "temperate patch {} is missing a base level at {coord:?}",
                        patch_id.0
                    ))
                })?;
                if !(key.low..=key.high).contains(&level)
                    || !(MIN_SURFACE_LEVEL..=MAX_V3_LEVEL).contains(&level)
                {
                    return Err(landform_error(format!(
                        "temperate patch {} produced level {level} outside {}..={} at {coord:?}",
                        patch_id.0, key.low, key.high
                    )));
                }
                Ok((coord, level))
            })
            .collect::<Result<BTreeMap<_, _>, V3GenerationError>>()?;
        by_patch.insert(patch_id, patch_levels);
    }
    Ok(by_patch)
}

fn uses_shared_landform(recipe: &V3RecipeSettings) -> bool {
    matches!(
        recipe,
        V3RecipeSettings::Forest(_)
            | V3RecipeSettings::Prairie(_)
            | V3RecipeSettings::Hills(_)
            | V3RecipeSettings::Waterfall(_)
    )
}

fn shared_stream(candidate: Option<(u64, u8)>) -> SeedStream<'static> {
    let (world_seed, candidate) = candidate.unwrap_or((CANONICAL_LANDFORM_SEED, 0));
    SeedStreams::new(world_seed, candidate, SHARED_LANDFORM_NAMESPACE).stage("macro.natural-land")
}

fn directional_group_levels(
    mask: &BTreeSet<HexCoord>,
    key: ElevationKey,
    relief: &BTreeMap<HexCoord, Level>,
) -> Result<BTreeMap<HexCoord, Level>, V3GenerationError> {
    let from_low = directional_boundary_distances(mask, key.axis, false)?;
    let from_high = directional_boundary_distances(mask, key.axis, true)?;
    let range = key.high.saturating_sub(key.low);
    mask.iter()
        .copied()
        .map(|coord| {
            let low_distance = from_low.get(&coord).copied().ok_or_else(|| {
                landform_error(format!("low-side distance field omitted {coord:?}"))
            })?;
            let high_distance = from_high.get(&coord).copied().ok_or_else(|| {
                landform_error(format!("high-side distance field omitted {coord:?}"))
            })?;
            let span = low_distance.saturating_add(high_distance);
            let progress = if span == 0 {
                range / 2
            } else {
                let numerator = i64::from(range)
                    .saturating_mul(i64::from(low_distance))
                    .saturating_add(i64::from(span / 2));
                Level::try_from(numerator / i64::from(span)).unwrap_or_default()
            };
            let level = key
                .low
                .saturating_add(progress)
                .saturating_add(relief.get(&coord).copied().unwrap_or_default())
                .clamp(key.low, key.high);
            Ok((coord, level))
        })
        .collect()
}

fn directional_boundary_distances(
    mask: &BTreeSet<HexCoord>,
    axis: MacroAxisSettings,
    high_side: bool,
) -> Result<BTreeMap<HexCoord, u32>, V3GenerationError> {
    let boundary_axis = if high_side { axis } else { opposite_axis(axis) };
    let seeds = mask
        .iter()
        .copied()
        .filter(|coord| !mask.contains(&axis_neighbor(*coord, boundary_axis)))
        .collect::<BTreeSet<_>>();
    if seeds.is_empty() {
        return Err(landform_error("has no directional boundary seeds"));
    }
    let mut distances = seeds
        .iter()
        .copied()
        .map(|coord| (coord, 0_u32))
        .collect::<BTreeMap<_, _>>();
    let mut pending = seeds.into_iter().collect::<VecDeque<_>>();
    while let Some(coord) = pending.pop_front() {
        let distance = distances.get(&coord).copied().unwrap_or_default();
        for neighbor in coord.neighbors() {
            if mask.contains(&neighbor) && !distances.contains_key(&neighbor) {
                distances.insert(neighbor, distance.saturating_add(1));
                pending.push_back(neighbor);
            }
        }
    }
    if distances.len() != mask.len() {
        return Err(landform_error(
            "contains a component unreachable from its directional boundary",
        ));
    }
    Ok(distances)
}

fn low_frequency_relief(
    mask: &BTreeSet<HexCoord>,
    stream: SeedStream<'_>,
) -> Result<BTreeMap<HexCoord, Level>, V3GenerationError> {
    let candidates = mask.iter().copied().collect::<Vec<_>>();
    let count = u64::try_from(candidates.len())
        .map_err(|error| landform_error(format!("landform mask exceeds u64: {error}")))?;
    if count == 0 {
        return Ok(BTreeMap::new());
    }
    let mut centers = BTreeSet::new();
    let mut lobes = Vec::new();
    for lobe_index in 0..LOBE_COUNT {
        let start = usize::try_from(stream.sample(lobe_index.saturating_mul(4)) % count)
            .unwrap_or_default();
        let center = (0..candidates.len()).find_map(|offset| {
            let index = start.saturating_add(offset) % candidates.len();
            let candidate = candidates.get(index).copied()?;
            centers.insert(candidate).then_some(candidate)
        });
        let Some(center) = center else {
            break;
        };
        let radius_sample = stream.sample(lobe_index.saturating_mul(4).saturating_add(1));
        let radius = MIN_LOBE_RADIUS
            .saturating_add(u32::try_from(radius_sample % LOBE_RADIUS_SPAN).unwrap_or_default());
        let amplitude_sample = stream.sample(lobe_index.saturating_mul(4).saturating_add(2)) % 4;
        let amplitude = match amplitude_sample {
            0 => -2,
            1 => -1,
            2 => 1,
            _ => 2,
        };
        lobes.push(ReliefLobe {
            center,
            radius,
            amplitude,
        });
    }

    Ok(mask
        .iter()
        .copied()
        .map(|coord| {
            let weighted = lobes.iter().fold(0_i64, |total, lobe| {
                total.saturating_add(lobe_contribution(coord, *lobe))
            });
            let rounded = rounded_divide(weighted, 1_024);
            let relief = Level::try_from(rounded)
                .unwrap_or(if rounded.is_negative() {
                    Level::MIN
                } else {
                    Level::MAX
                })
                .clamp(-RELIEF_LIMIT, RELIEF_LIMIT);
            (coord, relief)
        })
        .collect())
}

fn lobe_contribution(coord: HexCoord, lobe: ReliefLobe) -> i64 {
    let distance_squared = cartesian_distance_squared(coord, lobe.center);
    let radius = u64::from(lobe.radius);
    let radius_squared = 12_u64.saturating_mul(radius.saturating_mul(radius));
    if distance_squared >= radius_squared || radius_squared == 0 {
        return 0;
    }
    let remaining = radius_squared.saturating_sub(distance_squared);
    let numerator = u128::from(remaining)
        .saturating_mul(u128::from(remaining))
        .saturating_mul(1_024);
    let denominator = u128::from(radius_squared).saturating_mul(u128::from(radius_squared));
    let weight = i64::try_from(numerator / denominator).unwrap_or(1_024);
    i64::from(lobe.amplitude).saturating_mul(weight)
}

/// Squared centre distance in a Cartesian embedding, scaled by four.
///
/// Every one-step hex neighbor has distance squared twelve. Unlike cube distance,
/// equal-distance contours here are circular rather than visible hexagons.
fn cartesian_distance_squared(first: HexCoord, second: HexCoord) -> u64 {
    let delta_x = i64::from(first.x()).saturating_sub(i64::from(second.x()));
    let delta_y = i64::from(first.y()).saturating_sub(i64::from(second.y()));
    let horizontal = delta_x.saturating_mul(2).saturating_add(delta_y);
    let horizontal_squared = horizontal.saturating_mul(horizontal).saturating_mul(3);
    let vertical_squared = delta_y.saturating_mul(delta_y).saturating_mul(9);
    u64::try_from(horizontal_squared.saturating_add(vertical_squared)).unwrap_or(u64::MAX)
}

fn smooth_complete_field(
    mask: &BTreeSet<HexCoord>,
    bounds: &BTreeMap<HexCoord, (Level, Level)>,
    levels: &mut BTreeMap<HexCoord, Level>,
) {
    for _ in 0..SMOOTHING_PASSES {
        let previous = levels.clone();
        for coord in mask {
            let Some(current) = previous.get(coord).copied() else {
                continue;
            };
            let mut total = i64::from(current).saturating_mul(SMOOTHING_SELF_WEIGHT);
            let mut weight = SMOOTHING_SELF_WEIGHT;
            for neighbor in coord.neighbors() {
                if let Some(level) = previous.get(&neighbor) {
                    total = total.saturating_add(i64::from(*level));
                    weight = weight.saturating_add(1);
                }
            }
            let smoothed = Level::try_from(rounded_divide(total, weight)).unwrap_or(current);
            let bounded = bounds
                .get(coord)
                .map_or(smoothed, |(low, high)| smoothed.clamp(*low, *high));
            levels.insert(*coord, bounded);
        }
    }
}

fn enforce_local_continuity(
    mask: &BTreeSet<HexCoord>,
    bounds: &BTreeMap<HexCoord, (Level, Level)>,
    levels: &mut BTreeMap<HexCoord, Level>,
) -> Result<(), V3GenerationError> {
    let mut pending = mask.iter().copied().collect::<VecDeque<_>>();
    let mut updates = 0_usize;
    let maximum_updates = mask.len().saturating_mul(32).max(1);
    while let Some(coord) = pending.pop_front() {
        let Some(level) = levels.get(&coord).copied() else {
            continue;
        };
        for neighbor in coord.neighbors() {
            let Some(neighbor_level) = levels.get(&neighbor).copied() else {
                continue;
            };
            if level.abs_diff(neighbor_level) <= 1 {
                continue;
            }
            let (high_coord, high, low_coord, low) = if level > neighbor_level {
                (coord, level, neighbor, neighbor_level)
            } else {
                (neighbor, neighbor_level, coord, level)
            };
            let high_minimum = bounds
                .get(&high_coord)
                .map(|bounds| bounds.0)
                .unwrap_or(MIN_SURFACE_LEVEL);
            let low_maximum = bounds
                .get(&low_coord)
                .map(|bounds| bounds.1)
                .unwrap_or(MAX_V3_LEVEL);
            let midpoint = low.saturating_add(high.saturating_sub(low) / 2);
            let mut next_low = midpoint.min(low_maximum).max(low);
            let mut next_high = midpoint
                .saturating_add(high.saturating_sub(low) % 2)
                .max(high_minimum)
                .min(high);
            if next_high > next_low.saturating_add(1) {
                next_high = next_low.saturating_add(1).max(high_minimum).min(high);
            }
            if next_high > next_low.saturating_add(1) {
                next_low = next_high.saturating_sub(1).min(low_maximum).max(low);
            }
            if next_high == high && next_low == low {
                continue;
            }
            levels.insert(high_coord, next_high);
            levels.insert(low_coord, next_low);
            pending.push_back(high_coord);
            pending.push_back(low_coord);
            pending.extend(high_coord.neighbors());
            pending.extend(low_coord.neighbors());
            updates = updates.saturating_add(1);
            if updates > maximum_updates {
                return Err(landform_error(
                    "could not settle its one-level local continuity field",
                ));
            }
        }
    }

    let discontinuity = mask.iter().find_map(|coord| {
        let level = levels.get(coord).copied()?;
        coord.neighbors().into_iter().find_map(|neighbor| {
            let neighbor_level = levels.get(&neighbor).copied()?;
            (level.abs_diff(neighbor_level) > 1).then_some((
                *coord,
                level,
                neighbor,
                neighbor_level,
            ))
        })
    });
    if let Some((first, first_level, second, second_level)) = discontinuity {
        Err(landform_error(format!(
            "cannot reconcile adjacent levels {first:?}@{first_level} and \
             {second:?}@{second_level} within their authored bounds"
        )))
    } else {
        Ok(())
    }
}

const fn opposite_axis(axis: MacroAxisSettings) -> MacroAxisSettings {
    match axis {
        MacroAxisSettings::East => MacroAxisSettings::West,
        MacroAxisSettings::SouthEast => MacroAxisSettings::NorthWest,
        MacroAxisSettings::SouthWest => MacroAxisSettings::NorthEast,
        MacroAxisSettings::West => MacroAxisSettings::East,
        MacroAxisSettings::NorthWest => MacroAxisSettings::SouthEast,
        MacroAxisSettings::NorthEast => MacroAxisSettings::SouthWest,
    }
}

fn axis_neighbor(coord: HexCoord, axis: MacroAxisSettings) -> HexCoord {
    let [x, y, z] = coord.to_cubic_array();
    let [delta_x, delta_y, delta_z] = match axis {
        MacroAxisSettings::East => [1, 0, -1],
        MacroAxisSettings::SouthEast => [0, 1, -1],
        MacroAxisSettings::SouthWest => [-1, 1, 0],
        MacroAxisSettings::West => [-1, 0, 1],
        MacroAxisSettings::NorthWest => [0, -1, 1],
        MacroAxisSettings::NorthEast => [1, -1, 0],
    };
    HexCoord::new_cubic(
        x.saturating_add(delta_x),
        y.saturating_add(delta_y),
        z.saturating_add(delta_z),
    )
}

fn rounded_divide(value: i64, divisor: i64) -> i64 {
    if divisor <= 0 {
        return value;
    }
    if value >= 0 {
        value.saturating_add(divisor / 2) / divisor
    } else {
        -value.saturating_abs().saturating_add(divisor / 2) / divisor
    }
}

fn landform_error(detail: impl Into<String>) -> V3GenerationError {
    V3GenerationError::RecipeContract(format!("Macro natural landform {}", detail.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{MapSettings, ProceduralSettings, TerrainSettings, V3LayoutSettings};

    const MOUNTAIN_RANGE_RON: &str =
        include_str!("../../../../assets/config/worlds/procedural-mountain-range.ron");

    fn fixture() -> (ResolvedLayoutPlan, MacroLayoutSettings) {
        let map: MapSettings =
            ron::from_str(MOUNTAIN_RANGE_RON).expect("shipped Mountain Range settings parse");
        let TerrainSettings::Procedural(ProceduralSettings::V3(procedural)) = map.terrain else {
            panic!("Mountain Range should use procedural terrain");
        };
        let V3LayoutSettings::Macro(settings) = &procedural.layout else {
            panic!("Mountain Range should use a Macro layout");
        };
        let layout = super::super::layout::resolve_layout(map.grid_radius, &procedural)
            .expect("Mountain Range layout resolves");
        (layout, settings.clone())
    }

    fn flattened(
        levels: &BTreeMap<PatchId, BTreeMap<HexCoord, Level>>,
    ) -> BTreeMap<HexCoord, Level> {
        levels
            .values()
            .flat_map(|patch| patch.iter().map(|(coord, level)| (*coord, *level)))
            .collect()
    }

    #[test]
    fn mountain_range_landform_is_deterministic_bounded_and_patch_complete() {
        let (layout, settings) = fixture();
        let first = plan_base_surface_levels(&layout, &settings, Some((129_704_046, 3)))
            .expect("natural landform should resolve");
        let second = plan_base_surface_levels(&layout, &settings, Some((129_704_046, 3)))
            .expect("same natural landform should resolve");

        assert_eq!(first, second);
        assert_eq!(first.len(), 13, "six green and seven hill-band instances");
        for (patch_id, levels) in &first {
            let patch = layout.patches.get(patch_id).expect("planned patch exists");
            let instance = settings
                .instances
                .get(usize::try_from(patch_id.0).expect("patch id fits usize"))
                .expect("planned instance exists");
            assert_eq!(levels.len(), patch.mask.len());
            assert!(levels.values().all(|level| {
                (instance.elevation.low..=instance.elevation.high).contains(level)
            }));
        }
    }

    #[test]
    fn same_band_seams_are_as_smooth_as_interior_neighbors() {
        let (layout, settings) = fixture();
        let planned = plan_base_surface_levels(&layout, &settings, Some((7, 2)))
            .expect("natural landform should resolve");
        let all = flattened(&planned);
        let key_for = |patch: PatchId| {
            settings
                .instances
                .get(usize::try_from(patch.0).ok()?)
                .map(ElevationKey::from_instance)
        };
        let mut same_band_pairs = 0_usize;
        for edge in layout.shared_edges.values() {
            if key_for(edge.first.0) != key_for(edge.second.0)
                || !planned.contains_key(&edge.first.0)
                || !planned.contains_key(&edge.second.0)
            {
                continue;
            }
            for (first, second) in &edge.boundary_pairs {
                let first_level = all.get(first).expect("first seam endpoint is planned");
                let second_level = all.get(second).expect("second seam endpoint is planned");
                assert!(
                    first_level.abs_diff(*second_level) <= 1,
                    "same-band seam jumped at {first:?} -> {second:?}"
                );
                same_band_pairs = same_band_pairs.saturating_add(1);
            }
        }
        assert!(
            same_band_pairs > 100,
            "fixture should exercise broad lateral seams"
        );
        for (coord, level) in &all {
            for neighbor in coord.neighbors() {
                if let Some(neighbor_level) = all.get(&neighbor) {
                    assert!(level.abs_diff(*neighbor_level) <= 1);
                }
            }
        }
    }

    #[test]
    fn authored_temperate_bands_still_rise_toward_the_inland_axis() {
        let (layout, settings) = fixture();
        let planned = plan_base_surface_levels(&layout, &settings, Some((81, 2)))
            .expect("natural landform should resolve");
        let all = flattened(&planned);
        let mut groups = BTreeMap::<ElevationKey, BTreeSet<HexCoord>>::new();
        for (patch_id, levels) in &planned {
            let instance = settings
                .instances
                .get(usize::try_from(patch_id.0).expect("patch id fits usize"))
                .expect("planned instance exists");
            groups
                .entry(ElevationKey::from_instance(instance))
                .or_default()
                .extend(levels.keys().copied());
        }

        for (key, mask) in groups {
            let low_edge = mask
                .iter()
                .filter(|coord| !mask.contains(&axis_neighbor(**coord, opposite_axis(key.axis))))
                .filter_map(|coord| all.get(coord))
                .map(|level| i64::from(*level))
                .collect::<Vec<_>>();
            let high_edge = mask
                .iter()
                .filter(|coord| !mask.contains(&axis_neighbor(**coord, key.axis)))
                .filter_map(|coord| all.get(coord))
                .map(|level| i64::from(*level))
                .collect::<Vec<_>>();
            let low_total = low_edge.iter().sum::<i64>();
            let high_total = high_edge.iter().sum::<i64>();
            let low_count = i64::try_from(low_edge.len()).expect("low edge count fits i64");
            let high_count = i64::try_from(high_edge.len()).expect("high edge count fits i64");
            assert!(
                high_total.saturating_mul(low_count) > low_total.saturating_mul(high_count),
                "authored {:?} band did not rise toward {:?}",
                (key.low, key.high),
                key.axis
            );
        }
    }

    #[test]
    fn internal_patch_ownership_does_not_reset_the_world_field() {
        let (layout, settings) = fixture();
        let baseline = plan_base_surface_levels(&layout, &settings, Some((91, 4)))
            .map(|levels| flattened(&levels))
            .expect("baseline landform should resolve");
        let mut repartitioned = layout.clone();
        let forest_lower = PatchId(6);
        let prairie_lower = PatchId(7);
        let first_coord = repartitioned
            .patches
            .get(&forest_lower)
            .and_then(|patch| patch.mask.first())
            .copied()
            .expect("forest patch has a coordinate");
        let second_coord = repartitioned
            .patches
            .get(&prairie_lower)
            .and_then(|patch| patch.mask.first())
            .copied()
            .expect("prairie patch has a coordinate");
        repartitioned
            .patches
            .get_mut(&forest_lower)
            .expect("forest patch exists")
            .mask
            .remove(&first_coord);
        repartitioned
            .patches
            .get_mut(&forest_lower)
            .expect("forest patch exists")
            .mask
            .insert(second_coord);
        repartitioned
            .patches
            .get_mut(&prairie_lower)
            .expect("prairie patch exists")
            .mask
            .remove(&second_coord);
        repartitioned
            .patches
            .get_mut(&prairie_lower)
            .expect("prairie patch exists")
            .mask
            .insert(first_coord);

        let changed = plan_base_surface_levels(&repartitioned, &settings, Some((91, 4)))
            .map(|levels| flattened(&levels))
            .expect("repartitioned landform should resolve");
        assert_eq!(
            baseline, changed,
            "same-band ownership must not affect height"
        );
    }

    #[test]
    fn relief_is_low_frequency_and_not_one_linear_grade_per_cell() {
        let (layout, settings) = fixture();
        let first = plan_base_surface_levels(&layout, &settings, Some((31, 1)))
            .map(|levels| flattened(&levels))
            .expect("first seeded landform should resolve");
        let second = plan_base_surface_levels(&layout, &settings, Some((32, 1)))
            .map(|levels| flattened(&levels))
            .expect("second seeded landform should resolve");
        assert_ne!(
            first, second,
            "world seed must select the broad relief field"
        );

        let has_curvature = first.iter().any(|(coord, center)| {
            let low = axis_neighbor(*coord, MacroAxisSettings::West);
            let high = axis_neighbor(*coord, MacroAxisSettings::East);
            first
                .get(&low)
                .zip(first.get(&high))
                .is_some_and(|(low, high)| low.saturating_add(*high) != center.saturating_mul(2))
        });
        assert!(
            has_curvature,
            "landform must contain broad non-linear relief"
        );
    }

    #[test]
    fn cartesian_lobes_do_not_use_hex_distance_contours() {
        let center = HexCoord::ORIGIN;
        let east_two = HexCoord::new_cubic(2, 0, -2);
        let bent_two = HexCoord::new_cubic(1, 1, -2);
        assert_eq!(center.distance(east_two), center.distance(bent_two));
        assert_ne!(
            cartesian_distance_squared(center, east_two),
            cartesian_distance_squared(center, bent_two),
            "equal cube distance must not force a visible hexagonal relief contour"
        );
    }
}
