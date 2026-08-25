//! Objective gameplay illumination at exact positions.

use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use bevy_ecs::reflect::ReflectResource;
use bevy_reflect::Reflect;
use hex_core::{
    upper_dome_contains, ExactGridPoint, ExteriorIllumination, GameplayLight, HexCoord,
    IlluminationLevel, LightDomain, TilePos,
};

use crate::{PerceptionError, SurfaceSnapshots};

/// One public local light together with its freshly derived exact domain.
///
/// `GameplayLight` intentionally carries no cached domain. The ECS boundary derives
/// this snapshot from the source's current [`TilePos`] and generated interior
/// metadata each time illumination is recomputed.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq)]
pub struct LightSourceSnapshot {
    /// Exact current position of the light source.
    pub pos: TilePos,
    /// Exterior or exact interior currently containing the source.
    pub domain: LightDomain,
    /// Authored illumination tier and inclusive radius.
    pub light: GameplayLight,
}

/// Resolved objective illumination and domain at one exact surface.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedLight {
    /// Strongest ambient or local gameplay illumination reaching the position.
    pub level: IlluminationLevel,
    /// Exterior or exact generated interior containing the position.
    pub domain: LightDomain,
}

/// Objective gameplay illumination indexed by exact surface.
///
/// The map is ordered by [`TilePos`] so iteration, observation, tests, and future
/// fingerprints do not depend on ECS query order.
#[derive(Resource, Reflect, Debug, Default, Clone, PartialEq, Eq)]
#[reflect(Resource)]
pub struct ResolvedIllumination {
    by_surface: BTreeMap<TilePos, ResolvedLight>,
}

impl ResolvedIllumination {
    /// Resolves illumination for exact target positions and rejects duplicates.
    pub fn try_resolve(
        targets: impl IntoIterator<Item = (TilePos, LightDomain)>,
        exterior: ExteriorIllumination,
        lights: &[LightSourceSnapshot],
    ) -> Result<Self, PerceptionError> {
        let mut targets = targets.into_iter().collect::<Vec<_>>();
        targets.sort_by_key(|(pos, _)| *pos);
        let mut by_surface = BTreeMap::new();
        for (pos, domain) in targets {
            let resolved = ResolvedLight {
                level: resolve_illumination_at(pos, domain, exterior, lights),
                domain,
            };
            if by_surface.insert(pos, resolved).is_some() {
                return Err(PerceptionError::DuplicateSurface(pos));
            }
        }
        Ok(Self { by_surface })
    }

    /// Resolves every indexed current surface.
    pub fn from_surfaces(
        surfaces: &SurfaceSnapshots,
        exterior: ExteriorIllumination,
        lights: &[LightSourceSnapshot],
    ) -> Result<Self, PerceptionError> {
        Self::try_resolve(
            surfaces
                .iter()
                .map(|(position, surface)| (position, surface.domain)),
            exterior,
            lights,
        )
    }

    /// Returns objective illumination at one current exact surface.
    #[must_use]
    pub fn get(&self, pos: TilePos) -> Option<ResolvedLight> {
        self.by_surface.get(&pos).copied()
    }

    /// Iterates over exact surfaces and resolved light in position order.
    pub fn iter(&self) -> impl Iterator<Item = (TilePos, ResolvedLight)> + '_ {
        self.by_surface
            .iter()
            .map(|(position, resolved)| (*position, *resolved))
    }

    /// Iterates over all exact surfaces in one horizontal column.
    ///
    /// [`TilePos`] orders the horizontal coordinate before its level, so this is a
    /// bounded lookup in the canonical map rather than a scan of every world
    /// surface. Stacked cave, bridge, and ground surfaces remain independently
    /// addressable and retain exact-position ordering.
    pub fn iter_at_coord(
        &self,
        coord: HexCoord,
    ) -> impl Iterator<Item = (TilePos, ResolvedLight)> + '_ {
        let bottom = TilePos::new(coord, i32::MIN);
        let top = TilePos::new(coord, i32::MAX);
        self.by_surface
            .range(bottom..=top)
            .map(|(position, resolved)| (*position, *resolved))
    }

    /// Number of exact surfaces with resolved illumination.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_surface.len()
    }

    /// Whether no exact surface has resolved illumination.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_surface.is_empty()
    }
}

/// Resolves objective illumination at an arbitrary exact position and domain.
///
/// Exterior positions start at the authored ambient tier; interiors start Dark.
/// Applicable local sources must share the target's domain and fall inside their
/// inclusive upper-dome radius. Downward reach is cylindrical; upward reach follows
/// the exact spherical cap shared with sight.
#[must_use]
pub fn resolve_illumination_at(
    pos: TilePos,
    domain: LightDomain,
    exterior: ExteriorIllumination,
    lights: &[LightSourceSnapshot],
) -> IlluminationLevel {
    let ambient = match domain {
        LightDomain::Exterior => exterior.level,
        LightDomain::Interior(_) => IlluminationLevel::Dark,
    };

    lights
        .iter()
        .filter(|source| {
            source.domain == domain
                && upper_dome_contains(
                    ExactGridPoint::voxel_center(source.pos),
                    ExactGridPoint::voxel_center(pos),
                    source.light.radius,
                )
        })
        .fold(ambient, |level, source| level.max(source.light.level))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::{HexCoord, InteriorRegionId};

    fn pos(q: i32, r: i32, level: i32) -> TilePos {
        TilePos::new(HexCoord::from_axial(q, r), level)
    }

    fn source(
        pos: TilePos,
        domain: LightDomain,
        level: IlluminationLevel,
        radius: u32,
    ) -> LightSourceSnapshot {
        LightSourceSnapshot {
            pos,
            domain,
            light: GameplayLight::new(level, radius),
        }
    }

    #[test]
    fn ambient_is_exterior_only() {
        let exterior = ExteriorIllumination::new(IlluminationLevel::Bright);
        let interior = LightDomain::Interior(InteriorRegionId(3));
        assert_eq!(
            resolve_illumination_at(pos(0, 0, 5), LightDomain::Exterior, exterior, &[]),
            IlluminationLevel::Bright
        );
        assert_eq!(
            resolve_illumination_at(pos(0, 0, 5), interior, exterior, &[]),
            IlluminationLevel::Dark
        );
    }

    #[test]
    fn local_radius_uses_an_inclusive_upper_dome_and_downward_cylinder() {
        let exterior = ExteriorIllumination::new(IlluminationLevel::Dark);
        let light = source(
            pos(0, 0, 10),
            LightDomain::Exterior,
            IlluminationLevel::Dim,
            5,
        );

        assert_eq!(
            resolve_illumination_at(pos(3, 0, 14), LightDomain::Exterior, exterior, &[light],),
            IlluminationLevel::Dim
        );
        assert_eq!(
            resolve_illumination_at(pos(4, 0, 14), LightDomain::Exterior, exterior, &[light],),
            IlluminationLevel::Dark
        );
        assert_eq!(
            resolve_illumination_at(pos(5, 0, -100), LightDomain::Exterior, exterior, &[light],),
            IlluminationLevel::Dim
        );
        assert_eq!(
            resolve_illumination_at(pos(6, 0, -100), LightDomain::Exterior, exterior, &[light],),
            IlluminationLevel::Dark
        );
    }

    #[test]
    fn light_does_not_cross_domains() {
        let exterior = ExteriorIllumination::new(IlluminationLevel::Dark);
        let cave_a = LightDomain::Interior(InteriorRegionId(1));
        let cave_b = LightDomain::Interior(InteriorRegionId(2));
        let lamp = source(pos(0, 0, 6), cave_a, IlluminationLevel::Bright, 7);

        assert_eq!(
            resolve_illumination_at(pos(1, 0, 6), cave_a, exterior, &[lamp]),
            IlluminationLevel::Bright
        );
        assert_eq!(
            resolve_illumination_at(pos(1, 0, 6), cave_b, exterior, &[lamp]),
            IlluminationLevel::Dark
        );
        assert_eq!(
            resolve_illumination_at(pos(1, 0, 6), LightDomain::Exterior, exterior, &[lamp]),
            IlluminationLevel::Dark
        );
    }

    #[test]
    fn overlapping_sources_take_maximum_tier_in_any_order() {
        let exterior = ExteriorIllumination::new(IlluminationLevel::Dark);
        let dim = source(
            pos(0, 0, 4),
            LightDomain::Exterior,
            IlluminationLevel::Dim,
            5,
        );
        let bright = source(
            pos(1, 0, 4),
            LightDomain::Exterior,
            IlluminationLevel::Bright,
            5,
        );
        let target = pos(2, 0, 4);

        let forward =
            resolve_illumination_at(target, LightDomain::Exterior, exterior, &[dim, bright]);
        let reverse =
            resolve_illumination_at(target, LightDomain::Exterior, exterior, &[bright, dim]);
        assert_eq!(forward, IlluminationLevel::Bright);
        assert_eq!(reverse, forward);
    }

    #[test]
    fn exact_resolution_is_ordered_and_rejects_duplicates() {
        let low = pos(0, 0, 4);
        let high = pos(0, 0, 14);
        let exterior = ExteriorIllumination::new(IlluminationLevel::Dim);
        let resolved = ResolvedIllumination::try_resolve(
            [(high, LightDomain::Exterior), (low, LightDomain::Exterior)],
            exterior,
            &[],
        )
        .expect("distinct stacked targets");
        assert_eq!(
            resolved
                .iter()
                .map(|(position, _)| position)
                .collect::<Vec<_>>(),
            vec![low, high]
        );

        let error = ResolvedIllumination::try_resolve(
            [(low, LightDomain::Exterior), (low, LightDomain::Exterior)],
            exterior,
            &[],
        )
        .expect_err("duplicate exact targets must fail");
        assert_eq!(error, PerceptionError::DuplicateSurface(low));
    }
}
