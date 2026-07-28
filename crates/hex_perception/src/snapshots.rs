//! Deterministic snapshots at the ECS boundary.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use bevy_ecs::prelude::Resource;
use hex_core::{Headroom, HexSpan, LightDomain, SubstanceId, TilePos, TraversalEndpoint, UnitId};
use hex_units::Faction;

/// One exposed material surface as gameplay may currently inspect it.
///
/// The snapshot is keyed by its exact [`TilePos`], so a cave floor, ground surface,
/// and bridge in one horizontal column remain independent facts. `span` is retained
/// for presentation consumers, while movement-facing projections use the quantized
/// position, solidity, headroom, and blocker fields.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceSnapshot {
    /// Exact top material voxel of this exposed surface.
    pub pos: TilePos,
    /// Rendered vertical extent of the material run.
    pub span: HexSpan,
    /// Material occupying the run.
    pub substance: SubstanceId,
    /// Consecutive empty levels directly above the surface.
    pub headroom: Headroom,
    /// Whether the material can support ordinary traversal.
    pub is_solid: bool,
    /// Whether a contextual feature currently blocks this surface.
    pub blocked: bool,
    /// Exterior or exact generated interior containing this surface.
    pub domain: LightDomain,
}

impl SurfaceSnapshot {
    /// Returns the shared traversal facts represented by this snapshot.
    #[must_use]
    pub const fn traversal_endpoint(self) -> TraversalEndpoint {
        TraversalEndpoint::new(self.pos, self.is_solid, self.headroom)
    }
}

/// Current identity, faction, and exact position of one unit.
///
/// Faction knowledge stores these only while the unit is observed. The type contains
/// no renderer entity id, so its ordering and equality are stable across runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedUnit {
    /// Stable simulation identity.
    pub id: UnitId,
    /// Side this unit currently belongs to.
    pub faction: Faction,
    /// Exact surface occupied by the unit.
    pub pos: TilePos,
}

/// Current exposed surfaces in deterministic exact-position order.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct SurfaceSnapshots {
    by_pos: BTreeMap<TilePos, SurfaceSnapshot>,
}

impl SurfaceSnapshots {
    /// Builds an exact-position index, rejecting duplicate surfaces.
    ///
    /// Multiple surfaces at one horizontal coordinate are valid when their levels
    /// differ. Two snapshots claiming the same [`TilePos`] are ambiguous and fail
    /// regardless of whether their other fields happen to match.
    pub fn try_from_iter(
        snapshots: impl IntoIterator<Item = SurfaceSnapshot>,
    ) -> Result<Self, PerceptionError> {
        let mut by_pos = BTreeMap::new();
        for snapshot in snapshots {
            if by_pos.insert(snapshot.pos, snapshot).is_some() {
                return Err(PerceptionError::DuplicateSurface(snapshot.pos));
            }
        }
        Ok(Self { by_pos })
    }

    /// Returns one current exact surface.
    #[must_use]
    pub fn get(&self, pos: TilePos) -> Option<SurfaceSnapshot> {
        self.by_pos.get(&pos).copied()
    }

    /// Iterates over current surfaces in exact-position order.
    pub fn iter(&self) -> impl Iterator<Item = (TilePos, SurfaceSnapshot)> + '_ {
        self.by_pos
            .iter()
            .map(|(position, snapshot)| (*position, *snapshot))
    }

    /// Number of current exact surfaces.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_pos.len()
    }

    /// Whether there are no current exact surfaces.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_pos.is_empty()
    }
}

/// Invalid deterministic input supplied to perception.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerceptionError {
    /// Two current snapshots claimed one exact surface.
    DuplicateSurface(TilePos),
    /// Two current unit snapshots claimed one stable identity.
    DuplicateUnit(UnitId),
}

impl fmt::Display for PerceptionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateSurface(pos) => {
                write!(formatter, "duplicate perception surface at {pos:?}")
            }
            Self::DuplicateUnit(id) => write!(formatter, "duplicate perception unit {id:?}"),
        }
    }
}

impl Error for PerceptionError {}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::{HexCoord, InteriorRegionId};

    fn surface(pos: TilePos) -> SurfaceSnapshot {
        SurfaceSnapshot {
            pos,
            span: HexSpan::new(2.0, 3.0),
            substance: SubstanceId(4),
            headroom: Headroom(2),
            is_solid: true,
            blocked: false,
            domain: LightDomain::Exterior,
        }
    }

    #[test]
    fn exact_duplicate_surface_is_rejected() {
        let pos = TilePos::new(HexCoord::ORIGIN, 5);
        let error = SurfaceSnapshots::try_from_iter([surface(pos), surface(pos)])
            .expect_err("duplicate must fail");
        assert_eq!(error, PerceptionError::DuplicateSurface(pos));
    }

    #[test]
    fn stacked_surfaces_remain_distinct_and_ordered() {
        let lower = TilePos::new(HexCoord::ORIGIN, 5);
        let upper = TilePos::new(HexCoord::ORIGIN, 15);
        let mut upper_snapshot = surface(upper);
        upper_snapshot.domain = LightDomain::Interior(InteriorRegionId(2));

        let snapshots = SurfaceSnapshots::try_from_iter([upper_snapshot, surface(lower)])
            .expect("distinct levels are valid");
        let positions = snapshots
            .iter()
            .map(|(position, _)| position)
            .collect::<Vec<_>>();
        assert_eq!(positions, vec![lower, upper]);
        assert_eq!(snapshots.get(upper), Some(upper_snapshot));
    }

    #[test]
    fn surface_snapshot_projects_exact_traversal_facts() {
        let pos = TilePos::new(HexCoord::ORIGIN, 8);
        let mut snapshot = surface(pos);
        snapshot.is_solid = false;
        snapshot.headroom = Headroom(3);
        assert_eq!(
            snapshot.traversal_endpoint(),
            TraversalEndpoint::new(pos, false, Headroom(3))
        );
    }
}
