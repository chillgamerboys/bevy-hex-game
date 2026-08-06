//! Shared vocabulary for gameplay illumination, sight, and remembered terrain.
//!
//! This module deliberately contains no systems. `hex_perception` owns observation
//! and faction knowledge, while consumers use the small projections defined here
//! without depending on that crate.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use bevy_reflect::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{HexCoord, InteriorRegionId, TilePos, TraversalEndpoint};

/// Gameplay-relevant illumination at an exact position.
///
/// These tiers are deterministic simulation facts, not measurements of Bevy's
/// physical lights. Their ordering lets overlapping sources select the strongest
/// contribution with [`Ord::max`].
#[derive(Reflect, Debug, Default, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IlluminationLevel {
    /// No ambient or local gameplay light reaches the position.
    #[default]
    Dark,
    /// Moonlight or a similarly weak source reaches the position.
    Dim,
    /// Sunlight or a strong local source reaches the position.
    Bright,
}

/// Authoritative ambient illumination for the exterior domain.
///
/// A lighting-profile adapter publishes this resource from authored static or
/// fixed-time cycle settings before perception resolves exact positions. It is not
/// sampled from renderer lux, shadows, exposure, or emissive materials. Interiors
/// remain dark unless an applicable [`GameplayLight`] raises their tier.
#[derive(Resource, Reflect, Debug, Clone, Copy, PartialEq, Eq)]
#[reflect(Resource)]
pub struct ExteriorIllumination {
    /// Ambient tier supplied to exterior positions.
    pub level: IlluminationLevel,
}

impl ExteriorIllumination {
    /// Creates an explicit exterior ambient projection.
    #[must_use]
    pub const fn new(level: IlluminationLevel) -> Self {
        Self { level }
    }
}

/// Sight radius for one illumination tier.
#[derive(Reflect, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SightBand {
    /// Inclusive radius of the exact upper-dome sight volume.
    pub radius: u32,
}

impl SightBand {
    /// Creates an inclusive upper-dome sight band.
    #[must_use]
    pub const fn new(radius: u32) -> Self {
        Self { radius }
    }
}

/// Sight limits selected by gameplay illumination.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SightProfile {
    /// Limits under direct sunlight or equally strong local light.
    pub bright: SightBand,
    /// Limits under moonlight or equally weak local light.
    pub dim: SightBand,
    /// Limits without ambient or local light.
    pub dark: SightBand,
}

impl SightProfile {
    /// Initial ordinary sight contract used by the V3 perception roadmap.
    pub const DEFAULT: Self = Self {
        bright: SightBand::new(36),
        dim: SightBand::new(12),
        dark: SightBand::new(1),
    };

    /// Returns the base limits for an illumination tier.
    #[must_use]
    pub const fn band(self, illumination: IlluminationLevel) -> SightBand {
        match illumination {
            IlluminationLevel::Dark => self.dark,
            IlluminationLevel::Dim => self.dim,
            IlluminationLevel::Bright => self.bright,
        }
    }
}

impl Default for SightProfile {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// One exact point in the stacked hex grid, stored in sixths of a voxel.
///
/// Sixth-voxel coordinates represent voxel centres, horizontal top-face corners,
/// and vertical top faces without floating point. The horizontal coordinates are
/// cube coordinates whose three components always sum to zero. `anchor` identifies
/// the voxel column that authored the point and lets exact line queries build a
/// narrow candidate corridor without rounding a corner back to an arbitrary tile.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExactGridPoint {
    cube_sixths: [i64; 3],
    level_sixths: i64,
    anchor: HexCoord,
}

impl ExactGridPoint {
    const SIXTHS_PER_VOXEL: i64 = 6;
    const TOP_FACE_OFFSET: i64 = 3;
    const STANDING_EYE_OFFSET: i64 = 12;

    /// Returns the centre of one exact voxel.
    #[must_use]
    pub fn voxel_center(pos: TilePos) -> Self {
        Self::at_offsets(pos, [0, 0, 0], 0)
    }

    /// Returns the centre of a material voxel's top face.
    #[must_use]
    pub fn voxel_top_center(pos: TilePos) -> Self {
        Self::at_offsets(pos, [0, 0, 0], Self::TOP_FACE_OFFSET)
    }

    /// Returns the eye point of a standing two-voxel-tall character.
    ///
    /// `support` is the material voxel beneath the character. The eye is the centre
    /// of the second air voxel above that support, at `support.level + 2`.
    #[must_use]
    pub fn standing_eye(support: TilePos) -> Self {
        Self::at_offsets(support, [0, 0, 0], Self::STANDING_EYE_OFFSET)
    }

    /// Returns the six exact corners of a material voxel's top face.
    ///
    /// Cube-coordinate corner offsets are permutations of `(2/3, -1/3, -1/3)`.
    /// Expressing them in sixths keeps all later intersection tests integral.
    #[must_use]
    pub fn voxel_top_corners(pos: TilePos) -> [Self; 6] {
        [
            [4, -2, -2],
            [2, 2, -4],
            [-2, 4, -2],
            [-4, 2, 2],
            [-2, -2, 4],
            [2, -4, 2],
        ]
        .map(|offsets| Self::at_offsets(pos, offsets, Self::TOP_FACE_OFFSET))
    }

    fn at_offsets(pos: TilePos, offsets: [i64; 3], level_offset: i64) -> Self {
        let scale = Self::SIXTHS_PER_VOXEL;
        let [q_offset, r_offset, s_offset] = offsets;
        let q = i64::from(pos.coord.x());
        let r = i64::from(pos.coord.y());
        // Widen before deriving the third cube component. `HexCoord::z()` returns
        // `i32`, so a valid boundary coordinate such as `(i32::MIN, 1, i32::MAX)`
        // would otherwise overflow while evaluating `-x` in debug builds.
        let s = -q - r;
        Self {
            cube_sixths: [
                q * scale + q_offset,
                r * scale + r_offset,
                s * scale + s_offset,
            ],
            level_sixths: i64::from(pos.level) * scale + level_offset,
            anchor: pos.coord,
        }
    }

    /// Returns the exact cube coordinates in sixths of a voxel.
    #[must_use]
    pub const fn cube_sixths(self) -> [i64; 3] {
        self.cube_sixths
    }

    /// Returns the exact vertical coordinate in sixths of a voxel.
    #[must_use]
    pub const fn level_sixths(self) -> i64 {
        self.level_sixths
    }

    /// Returns the voxel column that authored this point.
    #[must_use]
    pub const fn anchor(self) -> HexCoord {
        self.anchor
    }
}

/// Whether `target` is inside `source`'s inclusive upper-dome radius.
///
/// Horizontal distance is the continuous cube-coordinate distance. Only upward
/// vertical distance contributes, so the shape is a sphere's upper half over an
/// unbounded-downward cylinder. All arithmetic is widened before squaring; boundary
/// points are accepted exactly and no floating-point epsilon participates.
#[must_use]
pub fn upper_dome_contains(source: ExactGridPoint, target: ExactGridPoint, radius: u32) -> bool {
    let source_cube = source.cube_sixths();
    let target_cube = target.cube_sixths();
    let horizontal_sixths = source_cube
        .into_iter()
        .zip(target_cube)
        .map(|(source, target)| i128::from(target) - i128::from(source))
        .map(i128::abs)
        .max()
        .unwrap_or(0);
    let upward_sixths =
        (i128::from(target.level_sixths()) - i128::from(source.level_sixths())).max(0);
    let radius_sixths = i128::from(radius) * i128::from(ExactGridPoint::SIXTHS_PER_VOXEL);

    horizontal_sixths * horizontal_sixths + upward_sixths * upward_sixths
        <= radius_sixths * radius_sixths
}

/// How much a faction currently knows about an exact map position.
#[derive(
    Reflect,
    Serialize,
    Deserialize,
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
)]
pub enum KnowledgeState {
    /// The faction has never observed the position.
    #[default]
    Unknown,
    /// The faction retains its last observed snapshot of the position.
    Remembered,
    /// The faction currently observes the position.
    Observed,
}

/// How a faction came to know something.
///
/// The two channels are deliberately separate, and this enum is what keeps them
/// from being conflated. Sight establishes *where* a unit is; it reveals nothing
/// about that unit's lattice. Divination is the channel that reveals lattice
/// facts, and it is a sanctioned writer of knowledge in its own right rather than
/// a modifier on observation.
///
/// That is why knowledge is tagged with its source and expires per entry instead
/// of being derived from whatever is currently visible: a store keyed on "can I
/// see it" has nowhere to put a fact that arrived from a cast.
#[derive(Reflect, Debug, Default, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum KnowledgeSource {
    /// Learned by seeing it.
    #[default]
    Observation,
    /// Learned from a cast that revealed it.
    Divination,
}

/// How long one piece of knowledge survives.
///
/// Round-based rather than wall-clock, because the design expresses decay in
/// rounds — "revealed information decays or is one-time, unless the divination is
/// an enchantment" — and a round is the only clock a fight has.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum KnowledgeExpiry {
    /// Survives this many further round rollovers, then is forgotten.
    ///
    /// `Rounds(0)` is the design's *one-time* reveal: known for the remainder of
    /// the current round and gone at the next rollover. There is deliberately no
    /// separate `OneTime` variant — it would decay identically, and two spellings
    /// of one behaviour drift apart the moment either is edited.
    Rounds(u32),
    /// Never decays on its own.
    ///
    /// An enchantment-backed divination holds knowledge this way; ending the
    /// enchantment is what removes it, so the writer owns the lifetime rather
    /// than the clock.
    Sustained,
}

impl KnowledgeExpiry {
    /// Advances one round rollover, returning [`None`] once the fact has lapsed.
    ///
    /// [`Self::Sustained`] is returned unchanged, which is what makes an
    /// enchantment's knowledge outlive the rounds a decaying reveal is measured
    /// in.
    #[must_use]
    pub const fn tick(self) -> Option<Self> {
        match self {
            Self::Rounds(0) => None,
            Self::Rounds(remaining) => Some(Self::Rounds(remaining - 1)),
            Self::Sustained => Some(Self::Sustained),
        }
    }
}

/// Generated spatial domain containing a light source or exact position.
///
/// A light source affects only targets in the same domain. Domains are derived from
/// exact current positions; [`GameplayLight`] intentionally does not cache one,
/// because a carried light may cross a cave entrance. Sight itself uses physical
/// terrain obstruction and may cross a domain boundary through an open threshold.
#[derive(Reflect, Debug, Default, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LightDomain {
    /// Open-air terrain and structures.
    #[default]
    Exterior,
    /// One generated interior network.
    Interior(InteriorRegionId),
}

/// An obstruction-agnostic gameplay light source.
///
/// Perception combines this component with the source entity's exact [`TilePos`] and
/// freshly derived [`LightDomain`]. `radius` uses the shared exact upper-dome metric:
/// upward distance consumes horizontal reach while downward targets keep the full
/// cylinder. Physical lights and emissive materials are presentation details and do
/// not establish gameplay sight.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Eq)]
#[reflect(Component)]
pub struct GameplayLight {
    /// Illumination contributed inside the source area.
    pub level: IlluminationLevel,
    /// Inclusive upper-dome source radius.
    pub radius: u32,
}

impl GameplayLight {
    /// Creates a gameplay light with one exact upper-dome radius.
    #[must_use]
    pub const fn new(level: IlluminationLevel, radius: u32) -> Self {
        Self { level, radius }
    }
}

/// Last published traversal facts for one remembered or observed surface.
///
/// Keeping the endpoint snapshot here prevents route planning through remembered
/// terrain from consulting live hidden geometry after a terrain edit.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownTraversal {
    /// Whether this snapshot is remembered or currently observed.
    pub state: KnowledgeState,
    /// Exact standability and clearance facts captured when it was observed.
    pub endpoint: TraversalEndpoint,
    /// Whether a contextual feature blocked the surface when it was observed.
    pub blocked: bool,
}

/// Local player's traversal-facing projection of faction map knowledge.
///
/// The richer faction knowledge and presentation state belong to `hex_perception`.
/// `hex_units` will consume only this exact-surface projection.
/// Unknown positions are absent and read back as [`KnowledgeState::Unknown`].
#[derive(Resource, Debug, Default, Clone)]
pub struct LocalMapKnowledge {
    by_surface: BTreeMap<TilePos, KnownTraversal>,
}

impl LocalMapKnowledge {
    /// Creates an empty knowledge projection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Publishes a remembered or observed traversal snapshot.
    ///
    /// Publishing [`KnowledgeState::Unknown`] removes the entry, because unknown
    /// terrain must not retain traversal facts. Returns the previous snapshot.
    pub fn set(
        &mut self,
        state: KnowledgeState,
        endpoint: TraversalEndpoint,
        blocked: bool,
    ) -> Option<KnownTraversal> {
        if state == KnowledgeState::Unknown {
            self.by_surface.remove(&endpoint.pos)
        } else {
            self.by_surface.insert(
                endpoint.pos,
                KnownTraversal {
                    state,
                    endpoint,
                    blocked,
                },
            )
        }
    }

    /// Removes all knowledge of one exact surface.
    pub fn remove(&mut self, pos: TilePos) -> Option<KnownTraversal> {
        self.by_surface.remove(&pos)
    }

    /// Returns the traversal snapshot for one remembered or observed surface.
    #[must_use]
    pub fn get(&self, pos: TilePos) -> Option<KnownTraversal> {
        self.by_surface.get(&pos).copied()
    }

    /// Returns the knowledge state for an exact surface.
    #[must_use]
    pub fn state(&self, pos: TilePos) -> KnowledgeState {
        self.get(pos)
            .map_or(KnowledgeState::Unknown, |known| known.state)
    }

    /// Iterates over every remembered or observed exact surface in position order.
    pub fn iter(&self) -> impl Iterator<Item = (TilePos, KnownTraversal)> + '_ {
        self.by_surface
            .iter()
            .map(|(position, known)| (*position, *known))
    }

    /// Number of remembered or observed exact surfaces.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_surface.len()
    }

    /// Whether the projection contains no remembered or observed surfaces.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_surface.is_empty()
    }
}

/// Shared ordering phases for gameplay perception updates.
///
/// The binary configures these in order for gameplay entry and later updates. Other
/// crates opt into the shared phases without depending on the perception owner.
#[derive(SystemSet, Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum PerceptionSystems {
    /// Publish authored ambient facts such as [`ExteriorIllumination`].
    PublishAmbient,
    /// Resolve ambient and local illumination for exact positions.
    ResolveIllumination,
    /// Determine what each faction currently observes.
    ResolveObservation,
    /// Publish local traversal knowledge for movement consumers.
    PublishKnowledge,
    /// Project knowledge into fog and other presentation state.
    ApplyPresentation,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Headroom, HexCoord};

    fn endpoint(pos: TilePos) -> TraversalEndpoint {
        TraversalEndpoint::new(pos, true, Headroom(2))
    }

    #[test]
    fn default_sight_profile_selects_contract_radii() {
        let profile = SightProfile::default();
        assert_eq!(profile.band(IlluminationLevel::Bright), SightBand::new(36));
        assert_eq!(profile.band(IlluminationLevel::Dim), SightBand::new(12));
        assert_eq!(profile.band(IlluminationLevel::Dark), SightBand::new(1));
    }

    #[test]
    fn exact_points_represent_eye_top_centre_and_all_regular_hex_corners() {
        let surface = TilePos::new(HexCoord::from_axial(2, -3), 7);
        let centre = ExactGridPoint::voxel_center(surface);
        let top = ExactGridPoint::voxel_top_center(surface);
        let eye = ExactGridPoint::standing_eye(surface);

        assert_eq!(centre.cube_sixths(), [12, -18, 6]);
        assert_eq!(centre.level_sixths(), 42);
        assert_eq!(top.level_sixths(), 45);
        assert_eq!(eye.level_sixths(), 54);
        assert_eq!(top.anchor(), surface.coord);

        let corners = ExactGridPoint::voxel_top_corners(surface);
        assert_eq!(corners.len(), 6);
        for corner in corners {
            assert_eq!(corner.cube_sixths().into_iter().sum::<i64>(), 0);
            assert_eq!(corner.level_sixths(), top.level_sixths());
            assert_eq!(corner.anchor(), surface.coord);
            let offsets = corner
                .cube_sixths()
                .into_iter()
                .zip(centre.cube_sixths())
                .map(|(corner, centre)| corner - centre)
                .collect::<Vec<_>>();
            let large = offsets
                .iter()
                .copied()
                .find(|offset| offset.abs() == 4)
                .expect("one corner axis has magnitude four sixths");
            assert_eq!(offsets.iter().filter(|offset| offset.abs() == 2).count(), 2);
            assert!(offsets
                .iter()
                .copied()
                .filter(|offset| offset.abs() == 2)
                .all(|offset| offset.signum() == -large.signum()));
        }
    }

    #[test]
    fn upper_dome_uses_an_inclusive_squared_boundary() {
        let source = ExactGridPoint::voxel_center(TilePos::new(HexCoord::ORIGIN, 0));
        let boundary = ExactGridPoint::voxel_center(TilePos::new(HexCoord::from_axial(3, 0), 4));
        let outside = ExactGridPoint::voxel_center(TilePos::new(HexCoord::from_axial(4, 0), 4));

        assert!(upper_dome_contains(source, boundary, 5));
        assert!(!upper_dome_contains(source, outside, 5));
        assert!(upper_dome_contains(boundary, source, 3));
        assert!(!upper_dome_contains(boundary, source, 2));
    }

    #[test]
    fn upper_dome_is_a_downward_cylinder_and_preserves_half_levels() {
        let origin = TilePos::new(HexCoord::ORIGIN, 0);
        let source = ExactGridPoint::standing_eye(origin);
        let far_below =
            ExactGridPoint::voxel_top_center(TilePos::new(HexCoord::from_axial(5, 0), -100));
        let half_level_up = ExactGridPoint::voxel_top_center(TilePos::new(HexCoord::ORIGIN, 2));

        assert!(upper_dome_contains(source, far_below, 5));
        assert!(!upper_dome_contains(source, far_below, 4));
        assert!(!upper_dome_contains(source, half_level_up, 0));
        assert!(upper_dome_contains(source, half_level_up, 1));
    }

    #[test]
    fn upper_dome_handles_wide_coordinates_without_narrowing_or_overflow() {
        let source = ExactGridPoint::voxel_center(TilePos::new(
            HexCoord::from_axial(-1_000_000_000, 0),
            -1_000_000_000,
        ));
        let target = ExactGridPoint::voxel_center(TilePos::new(
            HexCoord::from_axial(1_000_000_000, 0),
            1_000_000_000,
        ));

        assert!(upper_dome_contains(source, target, u32::MAX));
        assert!(!upper_dome_contains(source, target, 2_000_000_000));
    }

    #[test]
    fn exact_points_widen_before_deriving_a_boundary_cube_component() {
        let coord = HexCoord::from_axial(i32::MIN, 1);
        let point = ExactGridPoint::voxel_center(TilePos::new(coord, 0));

        assert_eq!(
            point.cube_sixths(),
            [i64::from(i32::MIN) * 6, 6, i64::from(i32::MAX) * 6]
        );
        assert!(upper_dome_contains(point, point, 0));
    }

    #[test]
    fn strongest_illumination_has_the_greatest_order() {
        let exterior = ExteriorIllumination::new(IlluminationLevel::Dim);
        assert_eq!(exterior.level, IlluminationLevel::Dim);
        assert_eq!(
            IlluminationLevel::Dim.max(IlluminationLevel::Bright),
            IlluminationLevel::Bright
        );
        assert_eq!(
            IlluminationLevel::Dark.max(IlluminationLevel::Dim),
            IlluminationLevel::Dim
        );
    }

    #[test]
    fn unknown_surface_cannot_retain_a_traversal_snapshot() {
        let pos = TilePos::new(HexCoord::ORIGIN, 4);
        let mut knowledge = LocalMapKnowledge::new();
        knowledge.set(KnowledgeState::Observed, endpoint(pos), true);
        assert_eq!(knowledge.state(pos), KnowledgeState::Observed);

        knowledge.set(KnowledgeState::Remembered, endpoint(pos), true);
        assert_eq!(knowledge.state(pos), KnowledgeState::Remembered);
        let remembered = knowledge.get(pos).expect("remembered snapshot");
        assert_eq!(remembered.endpoint.pos, pos);
        assert!(remembered.blocked);

        knowledge.set(KnowledgeState::Unknown, endpoint(pos), false);
        assert_eq!(knowledge.state(pos), KnowledgeState::Unknown);
        assert!(knowledge.get(pos).is_none());
    }

    #[test]
    fn exact_surfaces_in_one_column_have_independent_knowledge() {
        let lower = TilePos::new(HexCoord::ORIGIN, 4);
        let upper = TilePos::new(HexCoord::ORIGIN, 12);
        let mut knowledge = LocalMapKnowledge::new();
        knowledge.set(KnowledgeState::Remembered, endpoint(lower), false);
        knowledge.set(KnowledgeState::Observed, endpoint(upper), false);

        assert_eq!(knowledge.state(lower), KnowledgeState::Remembered);
        assert_eq!(knowledge.state(upper), KnowledgeState::Observed);
        assert_eq!(knowledge.len(), 2);
    }
}
