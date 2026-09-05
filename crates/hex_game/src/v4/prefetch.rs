//! Route-directed loading priority from measured preparation latency.

use hex_world_contracts::{ResidencyRequest, VoxelPosition};

const SPEED_HEXES_PER_SECOND: f64 = 4.0;
const MAX_LEAD: u32 = 16;

/// Keep the existing surrounding buffer, but prepare the next exact route area
/// first. No floating-point world coordinates or speculative traversal are used.
pub(super) fn ahead(
    owner: &str,
    route: impl Iterator<Item = VoxelPosition>,
    measured_milliseconds: Option<f64>,
    outstanding_jobs: usize,
    workers: usize,
) -> Option<ResidencyRequest> {
    let latency = measured_milliseconds
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(250.0)
        / 1000.0;
    let waves = outstanding_jobs.div_ceil(workers.max(1)).clamp(1, 64);
    let waves = u32::try_from(waves).unwrap_or(64);
    let needed = latency * f64::from(waves) + 0.5;
    let lead = (1..=MAX_LEAD)
        .find(|lead| f64::from(*lead) / SPEED_HEXES_PER_SECOND >= needed)
        .unwrap_or(MAX_LEAD);
    let center = route
        .take(usize::try_from(lead).unwrap_or(16))
        .last()?
        .column;
    Some(ResidencyRequest {
        id: format!("prefetch/{owner}"),
        center,
        radius: 8,
        retention_radius: 16,
        priority: 20,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_world_contracts::WorldHex;

    fn route() -> impl Iterator<Item = VoxelPosition> {
        (1..=64).map(|offset| VoxelPosition {
            column: WorldHex::new(9_000_000_000_000 + offset, -9_000_000_000_000),
            level: 4,
        })
    }

    #[test]
    fn slow_storage_and_backlog_increase_bounded_exact_route_lead() {
        let fast = ahead("party/0", route(), Some(1.0), 1, 2).expect("route");
        let slow = ahead("party/0", route(), Some(500.0), 10, 2).expect("route");
        let blocked = ahead("party/0", route(), Some(60_000.0), usize::MAX, 0).expect("route");
        assert_eq!(fast.center.q, 9_000_000_000_003);
        assert_eq!(slow.center.q, 9_000_000_000_012);
        assert_eq!(blocked.center.q, 9_000_000_000_016);
        assert_eq!(blocked.center.r, -9_000_000_000_000);
        assert_eq!(blocked.radius, 8);
        assert_eq!(blocked.retention_radius, 16);
    }

    #[test]
    fn no_route_or_finished_short_route_never_extrapolates_unknown_geography() {
        assert!(ahead("party/0", std::iter::empty(), None, 0, 2).is_none());
        let short = ahead("party/1", route().take(1), Some(f64::NAN), 0, 2).expect("one leg");
        assert_eq!(short.center.q, 9_000_000_000_001);
        assert_eq!(short.id, "prefetch/party/1");
    }
}
