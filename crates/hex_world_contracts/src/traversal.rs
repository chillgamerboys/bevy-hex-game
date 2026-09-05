//! Exact traversal geometry shared by authoring and resident-world consumers.

use crate::Surface;

/// Whether two known solid support surfaces admit an adjacent body transition.
///
/// Callers supply the gameplay profile in quantized levels; ordinary walking uses
/// `(levels_tall, max_climb, max_drop) = (2, 1, 1)`. A [`Surface`] already denotes
/// solid support, so material IDs are not inspected here. Availability and route
/// ownership remain the caller's responsibility.
///
/// The geometry preserves `hex_core::TraversalProfile::admits_transition` with
/// endpoints translated to levels zero and their checked relative delta. This
/// avoids absolute-coordinate overflow while retaining the existing saturating
/// i32 clearance convention: sky and larger u32 clearances map to `i32::MAX`.
#[must_use]
pub fn admits_surface_transition(
    from: &Surface,
    to: &Surface,
    levels_tall: i32,
    max_climb: i32,
    max_drop: i32,
) -> bool {
    if levels_tall <= 0 || from.position.column.checked_distance(to.position.column) != Ok(1) {
        return false;
    }
    let delta = i64::from(to.position.level) - i64::from(from.position.level);
    if delta > i64::from(max_climb) || -delta > i64::from(max_drop) {
        return false;
    }
    let Ok(delta) = i32::try_from(delta) else {
        return false;
    };
    let clearance = |surface: &Surface| {
        surface
            .headroom
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or(i32::MAX)
    };
    let from_clear = clearance(from);
    let to_clear = clearance(to);
    if from_clear < levels_tall || to_clear < levels_tall {
        return false;
    }
    let higher_floor = delta.max(0);
    let lower_clear_top = from_clear.min(delta.saturating_add(to_clear));
    lower_clear_top.saturating_sub(higher_floor) >= levels_tall
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{VoxelPosition, WorldHex};

    fn surface(q: i64, r: i64, level: i32, headroom: Option<u32>) -> Surface {
        Surface {
            position: VoxelPosition {
                column: WorldHex::new(q, r),
                level,
            },
            material: "stone".into(),
            headroom,
        }
    }

    #[test]
    fn two_standable_steps_need_shared_lateral_aperture_in_both_directions() {
        let mut lower = surface(0, 0, 10, Some(2));
        let upper = surface(1, 0, 11, Some(2));
        assert!(!admits_surface_transition(&lower, &upper, 2, 1, 1));
        assert!(!admits_surface_transition(&upper, &lower, 2, 1, 1));
        lower.headroom = Some(3);
        assert!(admits_surface_transition(&lower, &upper, 2, 1, 1));
        assert!(admits_surface_transition(&upper, &lower, 2, 1, 1));
        assert!(!admits_surface_transition(&lower, &upper, 3, 1, 1));
    }

    #[test]
    fn adjacency_body_height_and_asymmetric_step_limits_remain_exact() {
        let from = surface(0, 0, 10, Some(2));
        assert!(admits_surface_transition(
            &from,
            &surface(1, 0, 10, Some(2)),
            2,
            0,
            0
        ));
        for to in [
            surface(0, 0, 10, Some(2)),
            surface(2, 0, 10, Some(2)),
            surface(1, 0, 10, Some(1)),
        ] {
            assert!(!admits_surface_transition(&from, &to, 2, 1, 1));
        }
        let lower = surface(0, 0, 10, None);
        let upper = surface(1, 0, 11, None);
        assert!(!admits_surface_transition(&lower, &upper, 2, 0, 1));
        assert!(admits_surface_transition(&upper, &lower, 2, 0, 1));
        assert!(!admits_surface_transition(&lower, &upper, 0, 1, 1));
        assert!(!admits_surface_transition(&lower, &upper, -1, 1, 1));
        assert!(!admits_surface_transition(&upper, &lower, 2, 1, -1));
    }

    #[test]
    fn checked_relative_geometry_handles_world_coordinate_and_level_extremes() {
        for level in [i32::MIN, i32::MAX - 1] {
            let from = surface(i64::MAX - 1, i64::MIN + 1, level, Some(3));
            let to = surface(i64::MAX, i64::MIN + 1, level + 1, Some(2));
            assert!(admits_surface_transition(&from, &to, 2, 1, 1));
            assert!(admits_surface_transition(&to, &from, 2, 1, 1));
        }
        let from = surface(0, 0, i32::MIN, None);
        let to = surface(1, 0, i32::MAX, None);
        assert!(!admits_surface_transition(
            &from,
            &to,
            2,
            i32::MAX,
            i32::MAX
        ));
        assert!(!admits_surface_transition(
            &to,
            &from,
            2,
            i32::MAX,
            i32::MAX
        ));
        assert!(!admits_surface_transition(
            &surface(i64::MIN, i64::MIN, 0, None),
            &surface(i64::MAX, i64::MAX, 0, None),
            2,
            1,
            1
        ));
    }

    #[test]
    fn unbounded_clearance_preserves_existing_saturating_i32_thresholds() {
        for headroom in [None, Some(i32::MAX as u32), Some(u32::MAX)] {
            let from = surface(0, 0, i32::MAX - 1, headroom);
            let flat = surface(1, 0, i32::MAX - 1, headroom);
            let raised = surface(1, 0, i32::MAX, headroom);
            assert!(admits_surface_transition(&from, &flat, i32::MAX, 1, 1));
            assert!(admits_surface_transition(&from, &raised, 2, 1, 1));
            assert!(!admits_surface_transition(&from, &raised, i32::MAX, 1, 1));
        }
    }
}
