//! Shared vocabulary for gameplay illumination, sight, and remembered terrain.
//!
//! This module deliberately contains no systems. The future `hex_perception` crate
//! owns observation and faction knowledge, while consumers use the small projections
//! defined here without depending on that crate.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use bevy_reflect::prelude::*;

use crate::{InteriorRegionId, Level, TilePos, TraversalEndpoint};

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

/// Horizontal and vertical sight limits for one illumination tier.
#[derive(Reflect, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SightBand {
    /// Maximum horizontal hex distance from an observer.
    pub horizontal: u32,
    /// Maximum absolute voxel-level distance from an observer.
    pub vertical: u32,
}

impl SightBand {
    /// Creates a sight band with independent horizontal and vertical limits.
    #[must_use]
    pub const fn new(horizontal: u32, vertical: u32) -> Self {
        Self {
            horizontal,
            vertical,
        }
    }
}

/// Sight limits selected by gameplay illumination.
///
/// Downhill sight may extend beyond the horizontal band by one hex for every
/// `downhill_levels_per_bonus` full levels below the observer, up to
/// `max_downhill_bonus`. A non-positive divisor disables the bonus rather than
/// allowing invalid settings to divide by zero.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SightProfile {
    /// Limits under direct sunlight or equally strong local light.
    pub bright: SightBand,
    /// Limits under moonlight or equally weak local light.
    pub dim: SightBand,
    /// Limits without ambient or local light.
    pub dark: SightBand,
    /// Full downhill levels required for one additional horizontal hex.
    pub downhill_levels_per_bonus: Level,
    /// Maximum horizontal range added by downhill elevation.
    pub max_downhill_bonus: u32,
}

impl SightProfile {
    /// Initial ordinary sight contract used by the V3 perception roadmap.
    pub const DEFAULT: Self = Self {
        bright: SightBand::new(36, 36),
        dim: SightBand::new(12, 12),
        dark: SightBand::new(1, 1),
        downhill_levels_per_bonus: 4,
        max_downhill_bonus: 6,
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

    /// Returns the capped horizontal bonus for a target below the observer.
    ///
    /// `downhill_levels` is measured as a non-negative difference. Passing zero
    /// therefore produces no bonus, as does a malformed non-positive divisor.
    /// Darkness never receives the elevation bonus.
    #[must_use]
    pub fn downhill_bonus(self, illumination: IlluminationLevel, downhill_levels: Level) -> u32 {
        if illumination == IlluminationLevel::Dark
            || downhill_levels <= 0
            || self.downhill_levels_per_bonus <= 0
        {
            return 0;
        }
        let bonus = downhill_levels / self.downhill_levels_per_bonus;
        u32::try_from(bonus)
            .unwrap_or(u32::MAX)
            .min(self.max_downhill_bonus)
    }
}

impl Default for SightProfile {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// How much a faction currently knows about an exact map position.
#[derive(Reflect, Debug, Default, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
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
/// A source affects only targets in the same domain. Domains are derived from exact
/// current positions; [`GameplayLight`] intentionally does not cache one, because a
/// carried light may cross a cave entrance. The first perception milestone also uses
/// matching domains as a coarse sight-eligibility boundary so lit interiors cannot
/// leak through opaque roofs; portal-aware sight may later refine that separate rule.
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
/// The future perception system combines this component with the source entity's
/// exact [`TilePos`] and freshly derived [`LightDomain`]. `radius` applies equally to
/// horizontal hex distance and absolute vertical level distance. Physical lights and
/// emissive materials are presentation details and do not establish gameplay sight.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Eq)]
#[reflect(Component)]
pub struct GameplayLight {
    /// Illumination contributed inside the source area.
    pub level: IlluminationLevel,
    /// Inclusive horizontal and vertical source radius.
    pub radius: u32,
}

impl GameplayLight {
    /// Creates a gameplay light with one radius for both distance axes.
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
/// The richer faction knowledge and presentation state belong to the future
/// `hex_perception` crate. `hex_units` consumes only this exact-surface projection.
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
/// crates opt into the shared phases without depending on the future perception
/// owner.
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
    fn default_sight_profile_selects_contract_bands_and_caps_downhill_bonus() {
        let profile = SightProfile::default();
        assert_eq!(
            profile.band(IlluminationLevel::Bright),
            SightBand::new(36, 36)
        );
        assert_eq!(profile.band(IlluminationLevel::Dim), SightBand::new(12, 12));
        assert_eq!(profile.band(IlluminationLevel::Dark), SightBand::new(1, 1));
        assert_eq!(profile.downhill_bonus(IlluminationLevel::Bright, 3), 0);
        assert_eq!(profile.downhill_bonus(IlluminationLevel::Bright, 4), 1);
        assert_eq!(profile.downhill_bonus(IlluminationLevel::Dim, 100), 6);
        assert_eq!(profile.downhill_bonus(IlluminationLevel::Bright, -4), 0);
        assert_eq!(profile.downhill_bonus(IlluminationLevel::Dark, 100), 0);
    }

    #[test]
    fn malformed_downhill_divisor_disables_bonus() {
        let profile = SightProfile {
            downhill_levels_per_bonus: 0,
            ..SightProfile::default()
        };
        assert_eq!(profile.downhill_bonus(IlluminationLevel::Bright, 12), 0);
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
