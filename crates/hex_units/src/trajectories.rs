//! Exact material trajectories and effect-volume clipping through the stacked hex
//! voxel grid.
//!
//! This module deliberately works only in [`hex_core::TilePos`] integer space. It does not
//! consult rendered spans, transforms, level height, or headroom. A straight segment
//! uses an inclusive supercover: every voxel whose closed prism touches the segment is
//! returned, including face, edge, and corner grazes. That conservative boundary rule
//! makes obstruction stable and direction-independent. After a cast reaches its
//! selected anchor, [`clip_effect_volume`](crate::trajectories::clip_effect_volume) and
//! [`clip_known_effect_volume`](crate::trajectories::clip_known_effect_volume) reuse
//! that same direct supercover to remove candidates hidden behind intermediate material.

use std::fmt;

use hex_assets::Trajectory;
use hex_core::{
    ElementId, ExactGridPoint, HexCoord, Level, TerrainBatchId, TerrainImpact, TilePos,
};

use crate::{AuthoredObjectOccupancy, KnownTerrainOccupancy, TerrainOccupancy};

/// Why a resolved effect volume could not be clipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectVolumeClipError {
    /// The input was not strictly sorted and deduplicated.
    NonCanonicalVolume,
}

impl fmt::Display for EffectVolumeClipError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonCanonicalVolume => {
                write!(
                    formatter,
                    "effect volume is not strictly sorted and deduplicated"
                )
            }
        }
    }
}

impl std::error::Error for EffectVolumeClipError {}

/// Resolves the selected surface into the endpoint a trajectory actually reaches.
///
/// Ordinary spells travel to the body/air voxel above their selected surface. A
/// construction spell instead travels to the material surface that authorizes
/// placement, then validates its separate creation volume above that anchor.
#[must_use]
pub const fn trajectory_destination(selected_surface: TilePos, creates_terrain: bool) -> TilePos {
    if creates_terrain {
        selected_surface
    } else {
        selected_surface.above()
    }
}

/// Every voxel touched by the centre-to-centre segment, including both endpoints.
///
/// Results are sorted and deduplicated. Intersection is evaluated with exact rational
/// bounds over the three horizontal cube coordinates and the vertical level, so no
/// floating-point rounding or directional nudge chooses one side of a tie.
#[must_use]
pub fn supercover(source: TilePos, destination: TilePos) -> Vec<TilePos> {
    let q_min = source
        .coord
        .x()
        .min(destination.coord.x())
        .saturating_sub(1);
    let q_max = source
        .coord
        .x()
        .max(destination.coord.x())
        .saturating_add(1);
    let r_min = source
        .coord
        .y()
        .min(destination.coord.y())
        .saturating_sub(1);
    let r_max = source
        .coord
        .y()
        .max(destination.coord.y())
        .saturating_add(1);
    let level_min = source.level.min(destination.level).saturating_sub(1);
    let level_max = source.level.max(destination.level).saturating_add(1);

    let start = [
        i64::from(source.coord.x()),
        i64::from(source.coord.y()),
        i64::from(source.coord.z()),
        i64::from(source.level),
    ];
    let end = [
        i64::from(destination.coord.x()),
        i64::from(destination.coord.y()),
        i64::from(destination.coord.z()),
        i64::from(destination.level),
    ];

    let mut touched = Vec::new();
    for q in q_min..=q_max {
        for r in r_min..=r_max {
            let coord = HexCoord::from_axial(q, r);
            for level in level_min..=level_max {
                let centre = [
                    i64::from(coord.x()),
                    i64::from(coord.y()),
                    i64::from(coord.z()),
                    i64::from(level),
                ];
                if segment_touches_closed_voxel(start, end, centre) {
                    touched.push(TilePos::new(coord, level));
                }
            }
        }
    }
    touched.sort_unstable();
    touched.dedup();
    touched
}

/// Whether authoritative terrain leaves one exact sight segment unobstructed.
///
/// A terrain run blocks only when the segment crosses the run's open interior for a
/// nonzero interval. Exact contact with a face, edge, corner, or either segment
/// endpoint therefore remains clear. Candidate columns come from a one-cell corridor
/// around the anchored hex line and each compact vertical run is tested directly;
/// this does not allocate or sort a three-dimensional voxel supercover.
#[must_use]
pub fn sight_segment_is_clear(
    source: ExactGridPoint,
    destination: ExactGridPoint,
    terrain: &TerrainOccupancy,
) -> bool {
    let corridor = sight_candidate_columns(source.anchor(), destination.anchor());
    sight_segment_is_clear_in_corridor(source, destination, terrain, &corridor, None)
}

/// Whether opt-in authored objects leave one exact sight segment unobstructed.
///
/// Unlike standing-character terrain sight, authored objects never receive the
/// observer-relative low-cover omission. Exact face, edge, corner, and endpoint-only
/// contacts remain clear through the same strict-interior kernel.
#[must_use]
pub fn authored_object_sight_segment_is_clear(
    source: ExactGridPoint,
    destination: ExactGridPoint,
    authored_objects: &AuthoredObjectOccupancy,
) -> bool {
    let corridor = sight_candidate_columns(source.anchor(), destination.anchor());
    sight_segment_is_clear_in_corridor(source, destination, authored_objects, &corridor, None)
}

/// Whether a standing observer has terrain-clear sight to an exposed surface.
///
/// The center ray starts at the centre of the second air voxel above
/// `observer_support`; six perimeter rays start at the character body volume's upper
/// corners. The target is accepted when the center ray is clear or when at least three
/// paired perimeter rays reach the correspondingly oriented target top-face corners.
/// The exposed top voxel of a run within one level of the observer's support is low
/// cover for this bundle when it is attached to material below: only the run's deeper
/// core participates in the intersection. This clears ordinary nearby steps without
/// turning disconnected bridge decks or overhead roofs transparent. Taller and
/// vertically remote runs retain their complete blocking volume. Each call evaluates
/// one observer in full, so callers pooling faction sight cannot accidentally combine
/// corner samples from different characters.
#[must_use]
pub fn terrain_sight_is_clear(
    observer_support: TilePos,
    target_surface: TilePos,
    terrain: &TerrainOccupancy,
) -> bool {
    sight_bundle_is_clear(observer_support, target_surface, terrain, None)
}

/// Whether terrain and opt-in authored objects leave a standing sight bundle clear.
///
/// Terrain retains its observer-relative low-cover projection. Authored-object runs
/// always intersect at full height, so an opted-in pillar cannot become transparent
/// merely because its top is near the observer's support level.
#[must_use]
pub fn terrain_and_authored_object_sight_is_clear(
    observer_support: TilePos,
    target_surface: TilePos,
    terrain: &TerrainOccupancy,
    authored_objects: &AuthoredObjectOccupancy,
) -> bool {
    sight_bundle_is_clear(
        observer_support,
        target_surface,
        terrain,
        Some(authored_objects),
    )
}

fn sight_bundle_is_clear(
    observer_support: TilePos,
    target_surface: TilePos,
    terrain: &TerrainOccupancy,
    authored_objects: Option<&AuthoredObjectOccupancy>,
) -> bool {
    let eye = ExactGridPoint::standing_eye(observer_support);
    let corridor = sight_candidate_columns(observer_support.coord, target_surface.coord);
    let low_cover_band = (
        observer_support.level.saturating_sub(1),
        observer_support.level.saturating_add(1),
    );
    if combined_sight_segment_is_clear(
        eye,
        ExactGridPoint::voxel_top_center(target_surface),
        terrain,
        authored_objects,
        &corridor,
        low_cover_band,
    ) {
        return true;
    }

    let mut clear_corners = 0_u8;
    for (index, (source, destination)) in
        ExactGridPoint::standing_body_top_corners(observer_support)
            .into_iter()
            .zip(ExactGridPoint::voxel_top_corners(target_surface))
            .enumerate()
    {
        if combined_sight_segment_is_clear(
            source,
            destination,
            terrain,
            authored_objects,
            &corridor,
            low_cover_band,
        ) {
            clear_corners += 1;
            if clear_corners >= 3 {
                return true;
            }
        }
        let remaining = 5_usize.saturating_sub(index);
        if usize::from(clear_corners) + remaining < 3 {
            return false;
        }
    }
    false
}

fn combined_sight_segment_is_clear(
    source: ExactGridPoint,
    destination: ExactGridPoint,
    terrain: &TerrainOccupancy,
    authored_objects: Option<&AuthoredObjectOccupancy>,
    corridor: &[HexCoord],
    low_cover_band: (Level, Level),
) -> bool {
    sight_segment_is_clear_in_corridor(source, destination, terrain, corridor, Some(low_cover_band))
        && authored_objects.is_none_or(|occupancy| {
            sight_segment_is_clear_in_corridor(source, destination, occupancy, corridor, None)
        })
}

fn sight_candidate_columns(source: HexCoord, destination: HexCoord) -> Vec<HexCoord> {
    let line = exact_hex_line(source, destination);
    let mut corridor = Vec::with_capacity(line.len().saturating_mul(7));
    for coord in line {
        corridor.push(coord);
        corridor.extend(representable_neighbors(coord));
    }
    corridor.sort_unstable();
    corridor.dedup();
    corridor
}

fn representable_neighbors(coord: HexCoord) -> impl Iterator<Item = HexCoord> {
    const AXIAL_OFFSETS: [(i32, i32); 6] = [(1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1)];
    AXIAL_OFFSETS.into_iter().filter_map(move |(q, r)| {
        Some(HexCoord::from_axial(
            coord.x().checked_add(q)?,
            coord.y().checked_add(r)?,
        ))
    })
}

/// Translation-invariant integer rasterization of a cube-coordinate segment.
///
/// The general-purpose lattice line helper interpolates in renderer-friendly
/// floating point. Sight uses this exact variant so a short line retains the same
/// candidate columns even when both endpoints are near the edge of the grid's valid
/// coordinate range.
fn exact_hex_line(source: HexCoord, destination: HexCoord) -> Vec<HexCoord> {
    let [source_x, source_y, source_z] = widened_cube(source);
    let [destination_x, destination_y, destination_z] = widened_cube(destination);
    let steps = [
        (destination_x - source_x).abs(),
        (destination_y - source_y).abs(),
        (destination_z - source_z).abs(),
    ]
    .into_iter()
    .max()
    .unwrap_or(0);
    if steps == 0 {
        return vec![source];
    }

    (0..=steps)
        .map(|step| {
            let weight_source = steps - step;
            let numerator_x = source_x * weight_source + destination_x * step;
            let numerator_y = source_y * weight_source + destination_y * step;
            let numerator_z = source_z * weight_source + destination_z * step;
            let mut rounded_x = round_ratio(numerator_x, steps);
            let mut rounded_y = round_ratio(numerator_y, steps);
            let rounded_z = round_ratio(numerator_z, steps);
            let error_x = (rounded_x * steps - numerator_x).abs();
            let error_y = (rounded_y * steps - numerator_y).abs();
            let error_z = (rounded_z * steps - numerator_z).abs();
            if error_x > error_y && error_x > error_z {
                rounded_x = -rounded_y - rounded_z;
            } else if error_y > error_z {
                rounded_y = -rounded_x - rounded_z;
            }
            HexCoord::from_axial(
                exact_line_component(rounded_x),
                exact_line_component(rounded_y),
            )
        })
        .collect()
}

fn widened_cube(coord: HexCoord) -> [i128; 3] {
    let q = i128::from(coord.x());
    let r = i128::from(coord.y());
    [q, r, -q - r]
}

fn exact_line_component(value: i128) -> i32 {
    match i32::try_from(value) {
        Ok(component) => component,
        // Convex interpolation between valid HexCoord endpoints stays in range. Keep
        // release behavior total if that invariant is ever violated by a future
        // coordinate representation; clamping expands toward the grid boundary.
        Err(_) if value.is_negative() => i32::MIN,
        Err(_) => i32::MAX,
    }
}

fn round_ratio(numerator: i128, positive_denominator: i128) -> i128 {
    debug_assert!(positive_denominator > 0);
    let quotient = numerator.div_euclid(positive_denominator);
    let remainder = numerator.rem_euclid(positive_denominator);
    if remainder * 2 >= positive_denominator {
        quotient + 1
    } else {
        quotient
    }
}

trait SightRunOccupancy {
    fn column_runs(&self, coord: HexCoord) -> &[(Level, Level)];
}

impl SightRunOccupancy for TerrainOccupancy {
    fn column_runs(&self, coord: HexCoord) -> &[(Level, Level)] {
        self.column_runs(coord)
    }
}

impl SightRunOccupancy for AuthoredObjectOccupancy {
    fn column_runs(&self, coord: HexCoord) -> &[(Level, Level)] {
        self.column_runs(coord)
    }
}

fn sight_segment_is_clear_in_corridor(
    source: ExactGridPoint,
    destination: ExactGridPoint,
    occupancy: &impl SightRunOccupancy,
    corridor: &[HexCoord],
    low_cover_band: Option<(Level, Level)>,
) -> bool {
    if source.cube_sixths() == destination.cube_sixths()
        && source.level_sixths() == destination.level_sixths()
    {
        return true;
    }
    corridor.iter().copied().all(|coord| {
        occupancy.column_runs(coord).iter().all(|&(bottom, top)| {
            let blocking_top = if bottom < top
                && low_cover_band
                    .is_some_and(|(minimum, maximum)| (minimum..=maximum).contains(&top))
            {
                top - 1
            } else {
                top
            };
            !segment_crosses_open_run(source, destination, coord, bottom, blocking_top)
        })
    })
}

fn segment_crosses_open_run(
    source: ExactGridPoint,
    destination: ExactGridPoint,
    coord: HexCoord,
    bottom: Level,
    top: Level,
) -> bool {
    let mut lower = Rational::new(0, 1);
    let mut upper = Rational::new(1, 1);
    if !intersect_open_interval(
        &mut lower,
        &mut upper,
        source.level_sixths(),
        destination.level_sixths() - source.level_sixths(),
        i64::from(bottom) * 6 - 3,
        i64::from(top) * 6 + 3,
    ) {
        return false;
    }

    let [source_q, source_r, source_s] = source.cube_sixths();
    let [destination_q, destination_r, destination_s] = destination.cube_sixths();
    let centre_q = i64::from(coord.x()) * 6;
    let centre_r = i64::from(coord.y()) * 6;
    let centre_s = -centre_q - centre_r;
    let source_differences = [
        source_q - source_r,
        source_r - source_s,
        source_s - source_q,
    ];
    let destination_differences = [
        destination_q - destination_r,
        destination_r - destination_s,
        destination_s - destination_q,
    ];
    let centre_differences = [
        centre_q - centre_r,
        centre_r - centre_s,
        centre_s - centre_q,
    ];
    for ((source_difference, destination_difference), centre_difference) in source_differences
        .into_iter()
        .zip(destination_differences)
        .zip(centre_differences)
    {
        if !intersect_open_interval(
            &mut lower,
            &mut upper,
            source_difference - centre_difference,
            destination_difference - source_difference,
            -6,
            6,
        ) {
            return false;
        }
    }
    true
}

fn intersect_open_interval(
    lower: &mut Rational,
    upper: &mut Rational,
    start: i64,
    delta: i64,
    open_min: i64,
    open_max: i64,
) -> bool {
    if delta == 0 {
        return open_min < start && start < open_max && *lower < *upper;
    }

    let first = Rational::new(open_min - start, delta);
    let second = Rational::new(open_max - start, delta);
    *lower = (*lower).max(first.min(second));
    *upper = (*upper).min(first.max(second));
    *lower < *upper
}

/// Intervening voxels a spell trajectory must keep free of material.
///
/// The true source and destination are excluded: a caster's own launch voxel and the
/// selected material surface authorize the cast rather than blocking it. An arc's
/// deterministic apex is not an endpoint and therefore remains obstructable.
#[must_use]
pub fn trajectory_voxels(
    trajectory: Trajectory,
    source: TilePos,
    destination: TilePos,
) -> Vec<TilePos> {
    let mut touched = match trajectory {
        Trajectory::Direct => supercover(source, destination),
        Trajectory::Arc { rise } => {
            let apex = arc_apex(source, destination, rise);
            let mut voxels = supercover(source, apex);
            voxels.extend(supercover(apex, destination));
            voxels
        }
        Trajectory::None => return Vec::new(),
    };
    touched.retain(|&pos| pos != source && pos != destination);
    touched.sort_unstable();
    touched.dedup();
    touched
}

/// Whether exact published material occupancy leaves this trajectory clear.
#[must_use]
pub fn trajectory_is_clear(
    trajectory: Trajectory,
    source: TilePos,
    destination: TilePos,
    terrain: &TerrainOccupancy,
) -> bool {
    trajectory_voxels(trajectory, source, destination)
        .into_iter()
        .all(|pos| !terrain.contains(pos))
}

/// Whether faction-authorized known material leaves this trajectory clear.
///
/// Presentation, target cycling, and AI use this optimistic projection. Full world
/// occupancy remains exclusive to the authoritative command application boundary.
#[must_use]
pub fn known_trajectory_is_clear(
    trajectory: Trajectory,
    source: TilePos,
    destination: TilePos,
    terrain: &KnownTerrainOccupancy,
) -> bool {
    trajectory_voxels(trajectory, source, destination)
        .into_iter()
        .all(|pos| !terrain.contains(pos))
}

/// Clips a canonical effect volume against complete authoritative material occupancy.
///
/// `Direct` and `Arc` describe how the cast reached `anchor`; once there, both spread
/// from the anchor to each candidate over the direct symmetric supercover. The radial
/// endpoints are excluded, so material at the anchor or candidate remains hittable
/// while intermediate material removes candidates behind it. `None` returns the
/// canonical input unchanged.
///
/// The function only filters. A noncanonical input is rejected rather than sorted,
/// deduplicated, or otherwise repaired.
pub fn clip_effect_volume(
    trajectory: Trajectory,
    anchor: TilePos,
    volume: Vec<TilePos>,
    terrain: &TerrainOccupancy,
) -> Result<Vec<TilePos>, EffectVolumeClipError> {
    clip_effect_volume_with(trajectory, anchor, volume, |pos| terrain.contains(pos))
}

/// Clips a canonical effect volume against faction-authorized known material.
///
/// Preview and AI use this optimistic projection so hidden material cannot change
/// faction-facing volume choices. Authoritative application must use
/// [`clip_effect_volume`] with complete [`TerrainOccupancy`] instead.
pub fn clip_known_effect_volume(
    trajectory: Trajectory,
    anchor: TilePos,
    volume: Vec<TilePos>,
    terrain: &KnownTerrainOccupancy,
) -> Result<Vec<TilePos>, EffectVolumeClipError> {
    clip_effect_volume_with(trajectory, anchor, volume, |pos| terrain.contains(pos))
}

fn clip_effect_volume_with(
    trajectory: Trajectory,
    anchor: TilePos,
    volume: Vec<TilePos>,
    contains_material: impl Fn(TilePos) -> bool,
) -> Result<Vec<TilePos>, EffectVolumeClipError> {
    // Use the announcement contract's own predicate rather than maintaining a
    // second interpretation of "canonical" beside it. The other fields do not
    // participate in `is_canonical` and the vector is recovered without cloning.
    let contract = TerrainImpact {
        batch: TerrainBatchId(0),
        volume,
        element: ElementId(0),
        power: 1,
    };
    if !contract.is_canonical() {
        return Err(EffectVolumeClipError::NonCanonicalVolume);
    }
    let mut volume = contract.volume;
    if matches!(trajectory, Trajectory::None) {
        return Ok(volume);
    }

    volume.retain(|&candidate| {
        trajectory_voxels(Trajectory::Direct, anchor, candidate)
            .into_iter()
            .all(|pos| !contains_material(pos))
    });
    Ok(volume)
}

/// Chooses a source/destination-symmetric horizontal midpoint and raises it.
fn arc_apex(source: TilePos, destination: TilePos, rise: u8) -> TilePos {
    let horizontal_source = TilePos::new(source.coord, 0);
    let horizontal_destination = TilePos::new(destination.coord, 0);
    let coord = supercover(horizontal_source, horizontal_destination)
        .into_iter()
        .map(|pos| pos.coord)
        .min_by_key(|coord| {
            let from_source = coord.distance(source.coord);
            let from_destination = coord.distance(destination.coord);
            (
                from_source.max(from_destination),
                from_source.saturating_add(from_destination),
                coord.x(),
                coord.y(),
            )
        })
        .unwrap_or(source.coord);
    TilePos::new(
        coord,
        source
            .level
            .max(destination.level)
            .saturating_add(Level::from(rise)),
    )
}

/// Exact line/closed-cell intersection in scaled cube-plus-level coordinates.
fn segment_touches_closed_voxel(start: [i64; 4], end: [i64; 4], centre: [i64; 4]) -> bool {
    let mut lower = Rational::new(0, 1);
    let mut upper = Rational::new(1, 1);

    for ((start, end), centre) in start.into_iter().zip(end).zip(centre) {
        // Scaling centres by two turns the closed half-voxel constraint into
        // `-1 <= a + b*t <= 1` without fractions.
        let a = 2 * (start - centre);
        let b = 2 * (end - start);
        if b == 0 {
            if !(-1..=1).contains(&a) {
                return false;
            }
            continue;
        }

        let first = Rational::new(-1 - a, b);
        let second = Rational::new(1 - a, b);
        let dimension_lower = first.min(second);
        let dimension_upper = first.max(second);
        lower = lower.max(dimension_lower);
        upper = upper.min(dimension_upper);
        if lower > upper {
            return false;
        }
    }
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rational {
    numerator: i64,
    denominator: i64,
}

impl Rational {
    fn new(mut numerator: i64, mut denominator: i64) -> Self {
        debug_assert_ne!(denominator, 0);
        if denominator < 0 {
            numerator = -numerator;
            denominator = -denominator;
        }
        Self {
            numerator,
            denominator,
        }
    }
}

impl Ord for Rational {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (i128::from(self.numerator) * i128::from(other.denominator))
            .cmp(&(i128::from(other.numerator) * i128::from(self.denominator)))
    }
}

impl PartialOrd for Rational {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use hex_core::{RunBottom, Sextant};

    use super::*;

    fn at(q: i32, r: i32, level: i32) -> TilePos {
        TilePos::new(HexCoord::from_axial(q, r), level)
    }

    fn occupied(voxels: impl IntoIterator<Item = TilePos>) -> TerrainOccupancy {
        TerrainOccupancy::from_runs(voxels.into_iter().map(|pos| (pos, RunBottom(pos.level))))
            .expect("single-voxel runs are valid")
    }

    fn authored(voxels: impl IntoIterator<Item = TilePos>) -> AuthoredObjectOccupancy {
        AuthoredObjectOccupancy::from_runs(
            voxels
                .into_iter()
                .map(|pos| hex_core::AuthoredObjectVoxelRun::new(pos, pos.level)),
        )
        .expect("single-voxel authored runs are valid")
    }

    fn advance(mut coord: HexCoord, direction: Sextant, distance: u32) -> HexCoord {
        for _ in 0..distance {
            coord = coord.neighbor(direction);
        }
        coord
    }

    fn sight_sample_results(
        observer: TilePos,
        target: TilePos,
        terrain: &TerrainOccupancy,
    ) -> (bool, [bool; 6]) {
        let eye = ExactGridPoint::standing_eye(observer);
        let centre = sight_segment_is_clear(eye, ExactGridPoint::voxel_top_center(target), terrain);
        let mut source_corners = ExactGridPoint::standing_body_top_corners(observer).into_iter();
        let corners = ExactGridPoint::voxel_top_corners(target).map(|destination| {
            source_corners
                .next()
                .is_some_and(|source| sight_segment_is_clear(source, destination, terrain))
        });
        assert!(source_corners.next().is_none());
        (centre, corners)
    }

    #[test]
    fn hidden_world_occupancy_cannot_change_authorized_trajectory_legality() {
        let source = at(0, 0, 2);
        let destination = at(3, 0, 2);
        let hidden = at(1, 0, 2);
        let clear_world = TerrainOccupancy::default();
        let blocked_world = occupied([hidden]);
        let same_knowledge = KnownTerrainOccupancy::default();

        assert!(trajectory_is_clear(
            Trajectory::Direct,
            source,
            destination,
            &clear_world,
        ));
        assert!(!trajectory_is_clear(
            Trajectory::Direct,
            source,
            destination,
            &blocked_world,
        ));
        assert!(known_trajectory_is_clear(
            Trajectory::Direct,
            source,
            destination,
            &same_knowledge,
        ));
    }

    #[test]
    fn direct_supercover_is_symmetric_in_all_six_sextants() {
        let source = at(0, 0, 2);
        for sextant in hex_core::Sextant::ALL {
            let mut coord = source.coord;
            for _ in 0..4 {
                coord = coord.neighbor(sextant);
            }
            let destination = TilePos::new(coord, 4);
            let forward = supercover(source, destination);
            let reverse = supercover(destination, source);
            assert_eq!(forward, reverse, "{source:?} -> {destination:?}");
            assert!(forward.contains(&source));
            assert!(forward.contains(&destination));
        }
    }

    #[test]
    fn arc_apex_and_supercover_are_source_destination_symmetric() {
        let source = at(-2, 1, 1);
        let destination = at(3, -2, 4);
        assert_eq!(
            trajectory_voxels(Trajectory::Arc { rise: 3 }, source, destination),
            trajectory_voxels(Trajectory::Arc { rise: 3 }, destination, source)
        );
    }

    #[test]
    fn vertical_and_mixed_segments_include_every_conservative_graze() {
        assert_eq!(
            supercover(at(0, 0, 0), at(0, 0, 2)),
            vec![at(0, 0, 0), at(0, 0, 1), at(0, 0, 2)]
        );

        let mixed = supercover(at(0, 0, 0), at(2, -1, 2));
        assert!(mixed.contains(&at(0, 0, 0)));
        assert!(mixed.contains(&at(2, -1, 2)));
        assert_eq!(mixed, supercover(at(2, -1, 2), at(0, 0, 0)));
    }

    #[test]
    fn face_edge_and_corner_ties_include_both_sides() {
        let diagonal = supercover(at(0, 0, 0), at(2, 1, 0));
        assert!(diagonal.contains(&at(1, 0, 0)));
        assert!(diagonal.contains(&at(1, 1, 0)));

        let rising = supercover(at(0, 0, 0), at(2, 0, 2));
        assert!(rising.contains(&at(1, 0, 1)));
        assert!(rising.contains(&at(1, 0, 0)));
        assert!(rising.contains(&at(1, 0, 2)));
    }

    #[test]
    fn sight_tangencies_are_clear_but_a_slight_interior_crossing_blocks() {
        let blocker = occupied([at(0, 0, 0)]);
        let top_face_source = ExactGridPoint::voxel_top_center(at(-2, 0, 0));
        let top_face_destination = ExactGridPoint::voxel_top_center(at(2, 0, 0));
        assert!(sight_segment_is_clear(
            top_face_source,
            top_face_destination,
            &blocker,
        ));

        let side_run = TerrainOccupancy::from_runs([(at(0, 0, 1), RunBottom(-1))])
            .expect("the side-tangency run is valid");
        let [first_corner, second_corner, ..] = ExactGridPoint::voxel_top_corners(at(0, 0, -1));
        assert!(
            sight_segment_is_clear(first_corner, second_corner, &side_run),
            "a segment on an exact horizontal face must remain clear"
        );

        let [lower_edge, ..] = ExactGridPoint::voxel_top_corners(at(0, 0, -2));
        let [upper_edge, ..] = ExactGridPoint::voxel_top_corners(at(0, 0, 1));
        assert!(
            sight_segment_is_clear(lower_edge, upper_edge, &blocker),
            "a segment on an exact vertical edge must remain clear"
        );

        let interior_source = ExactGridPoint::voxel_center(at(-2, 0, 0));
        let interior_destination = ExactGridPoint::voxel_center(at(2, 0, 0));
        assert!(!sight_segment_is_clear(
            interior_source,
            interior_destination,
            &blocker,
        ));
    }

    #[test]
    fn authored_object_segments_share_strict_tangencies_and_interior_crossings() {
        let blocker = authored([at(0, 0, 0)]);
        let tangent_source = ExactGridPoint::voxel_top_center(at(-2, 0, 0));
        let tangent_destination = ExactGridPoint::voxel_top_center(at(2, 0, 0));
        assert!(authored_object_sight_segment_is_clear(
            tangent_source,
            tangent_destination,
            &blocker,
        ));

        let interior_source = ExactGridPoint::voxel_center(at(-2, 0, 0));
        let interior_destination = ExactGridPoint::voxel_center(at(2, 0, 0));
        assert!(!authored_object_sight_segment_is_clear(
            interior_source,
            interior_destination,
            &blocker,
        ));
    }

    #[test]
    fn authored_object_runs_never_receive_terrain_low_cover_omission() {
        let observer = at(0, 0, 0);
        let target = at(4, 0, 0);
        let wall_columns = (1..=3)
            .flat_map(|q| (-2..=2).map(move |r| at(q, r, 1)))
            .collect::<Vec<_>>();
        let low_run = TerrainOccupancy::from_runs(
            wall_columns.iter().copied().map(|top| (top, RunBottom(0))),
        )
        .expect("grounded one-level terrain wall");
        let object = AuthoredObjectOccupancy::from_runs(
            wall_columns
                .into_iter()
                .map(|top| hex_core::AuthoredObjectVoxelRun::new(top, 0)),
        )
        .expect("grounded authored-object wall");

        assert!(terrain_sight_is_clear(observer, target, &low_run));
        assert!(!terrain_and_authored_object_sight_is_clear(
            observer,
            target,
            &TerrainOccupancy::default(),
            &object,
        ));
    }

    #[test]
    fn sight_through_a_single_exact_hex_corner_remains_clear() {
        let blocker = at(0, 0, 0);
        let source = at(1, -2, 0);
        let destination = at(0, 3, 0);
        let terrain = occupied([blocker]);
        let source_point = ExactGridPoint::voxel_center(source);
        let destination_point = ExactGridPoint::voxel_center(destination);
        let corner = ExactGridPoint::voxel_top_corners(blocker)[0];
        let [source_q, source_r, source_s] = source_point.cube_sixths();
        let [destination_q, destination_r, destination_s] = destination_point.cube_sixths();

        assert_eq!(
            [
                source_q * 2 + destination_q,
                source_r * 2 + destination_r,
                source_s * 2 + destination_s,
            ],
            corner.cube_sixths().map(|component| component * 3),
            "the segment reaches this exact blocker corner one third of the way through"
        );
        assert!(
            sight_segment_is_clear(source_point, destination_point, &terrain),
            "a point-only contact at cube offset (2/3, -1/3, -1/3) is not an interior crossing"
        );
    }

    #[test]
    fn strict_sight_intersection_is_direction_symmetric() {
        let terrain = TerrainOccupancy::from_runs([
            (at(0, 0, 2), RunBottom(-1)),
            (at(1, -1, 5), RunBottom(4)),
        ])
        .expect("the symmetric fixture is valid");
        let source = ExactGridPoint::standing_eye(at(-3, 1, 0));
        let destination = ExactGridPoint::voxel_top_corners(at(4, -2, 1))
            .into_iter()
            .next()
            .expect("a hex has corners");

        assert_eq!(
            sight_segment_is_clear(source, destination, &terrain),
            sight_segment_is_clear(destination, source, &terrain),
        );
    }

    #[test]
    fn zero_length_sight_segment_has_no_nonzero_interior_crossing() {
        let position = at(0, 0, 0);
        let point = ExactGridPoint::voxel_center(position);
        let terrain = occupied([position]);

        assert!(sight_segment_is_clear(point, point, &terrain));
    }

    #[test]
    fn sight_corridor_is_translation_invariant_at_large_coordinates() {
        let origin_observer = at(0, 0, 0);
        let origin_target = at(10, 0, 0);
        let origin_wall = TerrainOccupancy::from_runs([(at(5, 0, 3), RunBottom(0))])
            .expect("the origin wall run is valid");
        let offset = 1_000_000_000;
        let translated_observer = at(offset, 0, 0);
        let translated_target = at(offset + 10, 0, 0);
        let translated_wall = TerrainOccupancy::from_runs([(at(offset + 5, 0, 3), RunBottom(0))])
            .expect("the translated wall run is valid");

        assert_eq!(
            terrain_sight_is_clear(origin_observer, origin_target, &origin_wall),
            terrain_sight_is_clear(translated_observer, translated_target, &translated_wall,)
        );
        assert!(!terrain_sight_is_clear(
            translated_observer,
            translated_target,
            &translated_wall,
        ));
    }

    #[test]
    fn sight_corridor_is_total_at_a_valid_i32_cube_boundary() {
        let observer = at(i32::MIN, 1, 0);
        let target = at(i32::MIN + 1, 0, 0);
        let terrain = TerrainOccupancy::default();

        assert_eq!(
            sight_candidate_columns(observer.coord, target.coord),
            sight_candidate_columns(target.coord, observer.coord),
        );
        assert!(terrain_sight_is_clear(observer, target, &terrain));
        assert!(terrain_sight_is_clear(target, observer, &terrain));
        for (source, destination) in ExactGridPoint::standing_body_top_corners(observer)
            .into_iter()
            .zip(ExactGridPoint::voxel_top_corners(target))
        {
            assert!(sight_segment_is_clear(source, destination, &terrain));
            assert!(sight_segment_is_clear(destination, source, &terrain));
        }
    }

    #[test]
    fn body_top_bundle_sees_all_surrounding_ground_from_a_ten_level_pillar() {
        let observer = at(0, 0, 10);
        let terrain = TerrainOccupancy::from_runs(
            std::iter::once((observer, RunBottom(0))).chain(
                HexCoord::ORIGIN
                    .within_radius(8)
                    .into_iter()
                    .filter(|coord| *coord != HexCoord::ORIGIN)
                    .map(|coord| (TilePos::new(coord, 0), RunBottom(0))),
            ),
        )
        .expect("the pillar and surrounding ground are valid runs");

        for target_coord in HexCoord::ORIGIN
            .within_radius(8)
            .into_iter()
            .filter(|coord| *coord != HexCoord::ORIGIN)
        {
            let target = TilePos::new(target_coord, 0);
            assert!(
                terrain_sight_is_clear(observer, target, &terrain),
                "ten-level downward sight failed at {target_coord:?}"
            );
        }
    }

    #[test]
    fn one_level_lips_clear_in_every_rotation_and_for_off_axis_targets() {
        let observer = at(0, 0, 0);
        for direction in Sextant::ALL {
            let lip = TerrainOccupancy::from_runs([(
                TilePos::new(advance(HexCoord::ORIGIN, direction, 2), 1),
                RunBottom(0),
            )])
            .expect("a one-voxel lip attached to ground is a valid run");
            let aligned = TilePos::new(advance(HexCoord::ORIGIN, direction, 4), 0);
            let off_axis = TilePos::new(aligned.coord.neighbor(direction.turned(1)), 0);

            assert!(
                terrain_sight_is_clear(observer, aligned, &lip),
                "aligned low cover blocked toward {direction:?}"
            );
            assert!(
                terrain_sight_is_clear(observer, off_axis, &lip),
                "off-axis low cover blocked toward {direction:?}"
            );
        }
    }

    #[test]
    fn one_level_ridges_on_full_ground_runs_are_low_cover_in_every_rotation() {
        let observer = at(0, 0, 0);
        for direction in Sextant::ALL {
            let first = advance(HexCoord::ORIGIN, direction, 1);
            let ridge = advance(HexCoord::ORIGIN, direction, 2);
            let target_coord = advance(HexCoord::ORIGIN, direction, 3);
            let target = TilePos::new(target_coord, 0);
            let off_axis_target = TilePos::new(target_coord.neighbor(direction.turned(1)), 0);
            let terrain = TerrainOccupancy::from_runs([
                (observer, RunBottom(-4)),
                (TilePos::new(first, 0), RunBottom(-4)),
                (TilePos::new(ridge, 1), RunBottom(-4)),
                (target, RunBottom(-4)),
                (off_axis_target, RunBottom(-4)),
            ])
            .expect("the runtime-shaped ground and ridge runs are valid");

            assert!(
                terrain_sight_is_clear(observer, target, &terrain),
                "one-level full-run ridge blocked toward {direction:?}"
            );
            assert!(
                terrain_sight_is_clear(observer, off_axis_target, &terrain),
                "off-axis ground behind a full-run ridge blocked toward {direction:?}"
            );
        }
    }

    #[test]
    fn one_level_stepped_relief_cannot_occlude_its_near_field() {
        let observer = at(0, 0, 0);
        let surfaces = HexCoord::ORIGIN
            .within_radius(5)
            .into_iter()
            .map(|coord| {
                let level = i32::from((coord.x() + coord.y()).rem_euclid(2) != 0);
                TilePos::new(coord, level)
            })
            .collect::<Vec<_>>();
        let terrain = TerrainOccupancy::from_runs(
            surfaces
                .iter()
                .copied()
                .map(|position| (position, RunBottom(-4))),
        )
        .expect("the stepped near-field runs are valid");

        for target in surfaces
            .into_iter()
            .filter(|position| *position != observer)
        {
            assert!(
                terrain_sight_is_clear(observer, target, &terrain),
                "one-level stepped relief hid nearby surface {target:?}"
            );
        }
    }

    #[test]
    fn two_level_ridges_on_full_ground_runs_remain_blocking() {
        let observer = at(0, 0, 0);
        for direction in Sextant::ALL {
            let first = advance(HexCoord::ORIGIN, direction, 1);
            let ridge = advance(HexCoord::ORIGIN, direction, 2);
            let target = TilePos::new(advance(HexCoord::ORIGIN, direction, 3), 0);
            let terrain = TerrainOccupancy::from_runs([
                (observer, RunBottom(-4)),
                (TilePos::new(first, 0), RunBottom(-4)),
                (TilePos::new(ridge, 2), RunBottom(-4)),
                (target, RunBottom(-4)),
            ])
            .expect("the runtime-shaped ground and wall runs are valid");

            assert!(
                !terrain_sight_is_clear(observer, target, &terrain),
                "two-level full-run ridge failed to block toward {direction:?}"
            );
        }
    }

    #[test]
    fn observer_relative_low_cover_can_make_character_sight_directional() {
        let upper_observer = at(0, 0, 0);
        let lower_observer = at(2, 0, -5);
        let ridge = TerrainOccupancy::from_runs([(at(1, 0, -1), RunBottom(-2))])
            .expect("the grounded ridge is valid");
        let (raw_centre, raw_corners) =
            sight_sample_results(upper_observer, lower_observer, &ridge);

        assert!(!raw_centre);
        assert!(raw_corners.into_iter().filter(|clear| *clear).count() < 3);
        assert!(terrain_sight_is_clear(
            upper_observer,
            lower_observer,
            &ridge
        ));
        assert!(!terrain_sight_is_clear(
            lower_observer,
            upper_observer,
            &ridge
        ));
    }

    #[test]
    fn a_two_level_character_height_wall_blocks_the_body_top_bundle() {
        let observer = at(0, 0, 0);
        for direction in Sextant::ALL {
            let wall_coord = advance(HexCoord::ORIGIN, direction, 2);
            let wall = TerrainOccupancy::from_runs([(TilePos::new(wall_coord, 2), RunBottom(1))])
                .expect("a two-level wall is a valid run");
            let target = TilePos::new(advance(HexCoord::ORIGIN, direction, 4), 0);

            assert!(
                !terrain_sight_is_clear(observer, target, &wall),
                "character-height wall failed to block toward {direction:?}"
            );
        }
    }

    #[test]
    fn target_endpoint_material_does_not_obstruct_its_own_top_face() {
        let observer = at(0, 0, 0);
        let target = at(4, 0, 0);
        let terrain = occupied([target]);

        let (centre, corners) = sight_sample_results(observer, target, &terrain);
        assert!(centre);
        assert!(corners.into_iter().all(|clear| clear));
        assert!(terrain_sight_is_clear(observer, target, &terrain));
    }

    #[test]
    fn a_full_wall_blocks_while_a_stacked_bridge_gap_stays_clear() {
        let observer = at(0, 0, 0);
        let target = at(4, 0, 0);
        let wall = TerrainOccupancy::from_runs([(at(1, 0, 3), RunBottom(0))])
            .expect("the wall run is valid");
        assert!(!terrain_sight_is_clear(observer, target, &wall));

        let bridge_gap = TerrainOccupancy::from_runs([
            (at(2, 0, -1), RunBottom(-3)),
            (at(2, 0, 5), RunBottom(4)),
        ])
        .expect("the stacked bridge fixture is valid");
        assert!(terrain_sight_is_clear(observer, target, &bridge_gap));
    }

    #[test]
    fn sight_to_a_lower_surface_is_blocked_by_a_cliff_ridge() {
        let observer = at(0, 0, 0);
        let target = at(5, 0, 0);
        let cliff = TerrainOccupancy::from_runs([
            (observer, RunBottom(observer.level)),
            (at(2, 0, 5), RunBottom(-2)),
            (at(3, 0, 5), RunBottom(-2)),
            (target, RunBottom(target.level)),
        ])
        .expect("the cliff fixture is valid");

        assert!(!terrain_sight_is_clear(observer, target, &cliff));
    }

    #[test]
    fn an_overhead_roof_blocks_sight_to_an_elevated_surface() {
        let observer = at(0, 0, 0);
        let elevated_target = at(4, 0, 4);
        let roof_centre = HexCoord::from_axial(2, 0);
        let roof = occupied(
            std::iter::once(TilePos::new(roof_centre, 3))
                .chain(roof_centre.neighbors().map(|coord| TilePos::new(coord, 3))),
        );

        assert!(!terrain_sight_is_clear(observer, elevated_target, &roof,));
    }

    #[test]
    fn a_one_voxel_deck_below_a_high_observer_blocks_downward_sight() {
        let observer = at(0, 0, 10);
        for direction in Sextant::ALL {
            let deck_coord = advance(HexCoord::ORIGIN, direction, 3);
            let target = TilePos::new(advance(HexCoord::ORIGIN, direction, 7), 0);
            let deck = occupied([TilePos::new(deck_coord, 7)]);

            assert!(
                !terrain_sight_is_clear(observer, target, &deck),
                "one-voxel deck failed to block downward sight toward {direction:?}"
            );
        }
    }

    #[test]
    fn a_disconnected_deck_inside_the_low_cover_band_remains_blocking() {
        let observer = at(0, 0, 10);
        for direction in Sextant::ALL {
            let deck_centre = advance(HexCoord::ORIGIN, direction, 2);
            let target = TilePos::new(advance(HexCoord::ORIGIN, direction, 7), 0);
            let deck = occupied(
                std::iter::once(TilePos::new(deck_centre, 9))
                    .chain(deck_centre.neighbors().map(|coord| TilePos::new(coord, 9))),
            );

            assert!(
                !terrain_sight_is_clear(observer, target, &deck),
                "disconnected deck inside the low-cover band failed to block toward {direction:?}"
            );
        }
    }

    #[test]
    fn one_cell_candidate_corridor_contains_every_small_exact_intersection() {
        let observer = at(0, 0, 0);
        for target_coord in HexCoord::ORIGIN.within_radius(5) {
            let target = TilePos::new(target_coord, 0);
            let corridor = sight_candidate_columns(observer.coord, target_coord);
            let reverse_corridor = sight_candidate_columns(target_coord, observer.coord);
            let sources = std::iter::once(ExactGridPoint::standing_eye(observer))
                .chain(ExactGridPoint::standing_body_top_corners(observer));
            for source in sources {
                let destinations = std::iter::once(ExactGridPoint::voxel_top_center(target))
                    .chain(ExactGridPoint::voxel_top_corners(target));
                for destination in destinations {
                    for blocker_coord in HexCoord::ORIGIN.within_radius(7) {
                        if segment_crosses_open_run(source, destination, blocker_coord, -2, 3) {
                            assert!(
                                corridor.contains(&blocker_coord),
                                "corridor missed {blocker_coord:?} on {source:?} -> {destination:?}"
                            );
                            assert!(
                                reverse_corridor.contains(&blocker_coord),
                                "reverse corridor missed {blocker_coord:?} on {destination:?} -> {source:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn two_clear_corners_are_insufficient_for_each_observer() {
        let target = at(0, 0, 0);
        let blocker = at(-4, 2, 2);
        let first_observer = at(-5, 2, 0);
        let second_observer = at(-5, 3, 0);
        let terrain = occupied([blocker]);

        let (first_centre, first_corners) = sight_sample_results(first_observer, target, &terrain);
        let (second_centre, second_corners) =
            sight_sample_results(second_observer, target, &terrain);
        assert!(!first_centre);
        assert!(!second_centre);
        assert_eq!(first_corners, [false, false, false, false, true, true]);
        assert_eq!(second_corners, [false, true, true, false, false, false]);
        assert!(!terrain_sight_is_clear(first_observer, target, &terrain));
        assert!(!terrain_sight_is_clear(second_observer, target, &terrain));
    }

    #[test]
    fn unpaired_cross_corner_keyholes_do_not_contribute_to_the_threshold() {
        let observer = at(-5, 2, 0);
        let target = at(0, 0, 0);
        let terrain = occupied([at(-4, 2, 2)]);
        let (centre, paired) = sight_sample_results(observer, target, &terrain);
        let reachable_destinations = ExactGridPoint::voxel_top_corners(target)
            .into_iter()
            .enumerate()
            .filter(|(destination_index, destination)| {
                ExactGridPoint::standing_body_top_corners(observer)
                    .into_iter()
                    .enumerate()
                    .any(|(source_index, source)| {
                        source_index != *destination_index
                            && sight_segment_is_clear(source, *destination, &terrain)
                    })
            })
            .count();

        assert!(!centre);
        assert_eq!(paired.into_iter().filter(|clear| *clear).count(), 2);
        assert!(
            reachable_destinations >= 3,
            "the fixture must expose an area-to-area keyhole that pairing rejects"
        );
        assert!(!terrain_sight_is_clear(observer, target, &terrain));
    }

    #[test]
    fn exactly_three_clear_corners_make_the_surface_visible() {
        let target = at(0, 0, 0);
        let blocker = at(-3, 0, 1);
        let observer = at(-7, 1, 0);
        let terrain = occupied([blocker]);

        let (centre, corners) = sight_sample_results(observer, target, &terrain);
        assert!(!centre);
        assert_eq!(corners, [false, true, true, true, false, false]);
        assert!(terrain_sight_is_clear(observer, target, &terrain));
    }

    #[test]
    fn endpoints_authorize_but_an_intervening_wall_blocks() {
        let source = at(0, 0, 1);
        let destination = at(3, 0, 1);
        let endpoints = occupied([source, destination]);
        assert!(trajectory_is_clear(
            Trajectory::Direct,
            source,
            destination,
            &endpoints
        ));

        let wall = occupied([at(1, 0, 1)]);
        assert!(!trajectory_is_clear(
            Trajectory::Direct,
            source,
            destination,
            &wall
        ));
    }

    #[test]
    fn an_arc_clears_a_bridge_while_a_direct_shot_hits_it() {
        let source = at(0, 0, 2);
        let destination = at(4, 0, 2);
        let bridge = occupied([at(2, 0, 2), at(2, -1, 2)]);

        assert!(!trajectory_is_clear(
            Trajectory::Direct,
            source,
            destination,
            &bridge
        ));
        assert!(trajectory_is_clear(
            Trajectory::Arc { rise: 3 },
            source,
            destination,
            &bridge
        ));
    }

    #[test]
    fn direct_under_a_bridge_stays_in_the_gap() {
        let source = at(0, 0, 2);
        let destination = at(4, 0, 2);
        let bridge = occupied([at(2, 0, 5), at(2, -1, 5)]);
        assert!(trajectory_is_clear(
            Trajectory::Direct,
            source,
            destination,
            &bridge
        ));
    }

    #[test]
    fn a_cave_ceiling_blocks_the_arc_apex() {
        let source = at(0, 0, 1);
        let destination = at(4, 0, 1);
        let ceiling = occupied([at(2, 0, 4)]);
        assert!(!trajectory_is_clear(
            Trajectory::Arc { rise: 3 },
            source,
            destination,
            &ceiling
        ));
    }

    #[test]
    fn none_deliberately_bypasses_material() {
        let source = at(0, 0, 1);
        let destination = at(3, 0, 1);
        let wall = occupied([at(1, 0, 1), at(2, 0, 1)]);
        assert!(trajectory_is_clear(
            Trajectory::None,
            source,
            destination,
            &wall
        ));
        assert!(trajectory_voxels(Trajectory::None, source, destination).is_empty());
    }

    #[test]
    fn ordinary_and_creation_casts_resolve_distinct_endpoint_voxels() {
        let surface = at(2, -1, 4);
        assert_eq!(trajectory_destination(surface, false), at(2, -1, 5));
        assert_eq!(trajectory_destination(surface, true), surface);
    }

    #[test]
    fn flat_ground_does_not_block_an_ordinary_level_shot() {
        let standing = at(0, 0, 1);
        let target_surface = at(3, 0, 1);
        let floor = occupied((0..=3).map(|q| at(q, 0, 1)));

        assert!(trajectory_is_clear(
            Trajectory::Direct,
            standing.above(),
            trajectory_destination(target_surface, false),
            &floor,
        ));
    }
}
