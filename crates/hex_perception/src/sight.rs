//! Deterministic faction sight over exact illuminated surfaces.

use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::Resource;
use bevy_ecs::reflect::ReflectResource;
use bevy_reflect::Reflect;
use hex_core::{ExteriorIllumination, LightDomain, SightProfile, TilePos, UnitId};
use hex_units::Faction;

use crate::{
    resolve_illumination_at, FactionMapKnowledge, LightSourceSnapshot, ObservedUnit,
    PerceptionError, ResolvedIllumination, ResolvedLight,
};

/// Exact current observation for one faction.
///
/// Surface positions may include a formerly known position that is currently in
/// sight but has been deleted. Applying the observation purges that stale snapshot.
/// Units, by contrast, are present only while currently authoritative and observed.
#[derive(Reflect, Debug, Default, Clone, PartialEq, Eq)]
pub struct FactionObservation {
    surfaces: BTreeSet<TilePos>,
    units: BTreeMap<UnitId, ObservedUnit>,
}

impl FactionObservation {
    /// Creates an empty observation.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one exact position as currently observed.
    ///
    /// Returns whether the position was newly inserted.
    pub fn insert_surface(&mut self, pos: TilePos) -> bool {
        self.surfaces.insert(pos)
    }

    /// Records one currently observed unit, rejecting duplicate stable identities.
    pub fn try_insert_unit(&mut self, unit: ObservedUnit) -> Result<(), PerceptionError> {
        if self.units.contains_key(&unit.id) {
            return Err(PerceptionError::DuplicateUnit(unit.id));
        }
        self.units.insert(unit.id, unit);
        Ok(())
    }

    /// Whether the faction currently observes one exact position.
    #[must_use]
    pub fn observes(&self, pos: TilePos) -> bool {
        self.surfaces.contains(&pos)
    }

    /// Iterates over observed exact positions in position order.
    pub fn surfaces(&self) -> impl Iterator<Item = TilePos> + '_ {
        self.surfaces.iter().copied()
    }

    /// Returns one currently observed unit.
    #[must_use]
    pub fn unit(&self, id: UnitId) -> Option<ObservedUnit> {
        self.units.get(&id).copied()
    }

    /// Iterates over observed units in stable identity order.
    pub fn units(&self) -> impl Iterator<Item = (UnitId, ObservedUnit)> + '_ {
        self.units.iter().map(|(id, unit)| (*id, *unit))
    }

    /// Number of exact positions currently observed.
    #[must_use]
    pub fn surface_count(&self) -> usize {
        self.surfaces.len()
    }

    /// Number of units currently observed.
    #[must_use]
    pub fn unit_count(&self) -> usize {
        self.units.len()
    }

    /// Whether this observation contains neither positions nor units.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.surfaces.is_empty() && self.units.is_empty()
    }
}

/// Current spatial observations for both factions without ordering [`Faction`].
#[derive(Resource, Reflect, Debug, Default, Clone, PartialEq, Eq)]
#[reflect(Resource)]
pub struct FactionObservations {
    player: FactionObservation,
    hostile: FactionObservation,
}

impl FactionObservations {
    /// Creates empty observations for both factions.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates one complete observation snapshot from independent faction slots.
    #[must_use]
    pub const fn from_factions(player: FactionObservation, hostile: FactionObservation) -> Self {
        Self { player, hostile }
    }

    /// Creates observations with one faction slot populated.
    ///
    /// This is primarily useful to adapters and focused headless tests. Normal
    /// gameplay should call [`resolve_observations`] so both slots use one snapshot
    /// of current truth.
    #[must_use]
    pub fn with_faction(faction: Faction, observation: FactionObservation) -> Self {
        match faction {
            Faction::Player => Self {
                player: observation,
                hostile: FactionObservation::default(),
            },
            Faction::Hostile => Self {
                player: FactionObservation::default(),
                hostile: observation,
            },
        }
    }

    /// Returns one faction's current observation.
    #[must_use]
    pub const fn faction(&self, faction: Faction) -> &FactionObservation {
        match faction {
            Faction::Player => &self.player,
            Faction::Hostile => &self.hostile,
        }
    }
}

/// Whether one exact current surface observes another.
///
/// The target's illumination tier selects the sight band. Both surfaces must share a
/// light domain, horizontal cube distance must fit the band plus any downhill bonus,
/// and absolute level distance must fit the independent vertical band.
#[must_use]
pub fn can_observe(
    observer: TilePos,
    target: TilePos,
    illumination: &ResolvedIllumination,
    profile: SightProfile,
) -> bool {
    let Some(observer_light) = illumination.get(observer) else {
        return false;
    };
    let Some(target_light) = illumination.get(target) else {
        return false;
    };
    can_observe_resolved(
        observer,
        observer_light.domain,
        target,
        target_light,
        profile,
    )
}

/// Resolves pooled observations for both factions.
///
/// Supplied units whose [`ObservedUnit::provides_sight`] flag is set are active
/// observers for their own faction. Other supplied units remain eligible to be seen
/// by an active observer without extending their faction's field of view.
/// Current surfaces come from `illumination`. Formerly known positions that have
/// disappeared from current truth are also tested using their last-seen domain and
/// current ambient/local-light rules; if in sight they enter the observation so
/// knowledge application can purge them.
pub fn resolve_observations(
    units: impl IntoIterator<Item = ObservedUnit>,
    illumination: &ResolvedIllumination,
    prior_knowledge: &FactionMapKnowledge,
    exterior: ExteriorIllumination,
    lights: &[LightSourceSnapshot],
    profile: SightProfile,
) -> Result<FactionObservations, PerceptionError> {
    let units = index_units(units)?;
    for unit in units.values() {
        if illumination.get(unit.pos).is_none() {
            return Err(PerceptionError::UnitMissingSurface {
                id: unit.id,
                pos: unit.pos,
            });
        }
    }
    let player = resolve_faction(
        Faction::Player,
        &units,
        illumination,
        prior_knowledge,
        exterior,
        lights,
        profile,
    )?;
    let hostile = resolve_faction(
        Faction::Hostile,
        &units,
        illumination,
        prior_knowledge,
        exterior,
        lights,
        profile,
    )?;
    Ok(FactionObservations { player, hostile })
}

fn index_units(
    units: impl IntoIterator<Item = ObservedUnit>,
) -> Result<BTreeMap<UnitId, ObservedUnit>, PerceptionError> {
    let mut units = units.into_iter().collect::<Vec<_>>();
    units.sort_by_key(|unit| unit.id);
    let mut indexed = BTreeMap::new();
    for unit in units {
        if indexed.insert(unit.id, unit).is_some() {
            return Err(PerceptionError::DuplicateUnit(unit.id));
        }
    }
    Ok(indexed)
}

fn resolve_faction(
    faction: Faction,
    units: &BTreeMap<UnitId, ObservedUnit>,
    illumination: &ResolvedIllumination,
    prior_knowledge: &FactionMapKnowledge,
    exterior: ExteriorIllumination,
    lights: &[LightSourceSnapshot],
    profile: SightProfile,
) -> Result<FactionObservation, PerceptionError> {
    let mut observers = Vec::new();
    for unit in units
        .values()
        .filter(|unit| unit.faction == faction && unit.provides_sight)
    {
        let Some(resolved) = illumination.get(unit.pos) else {
            return Err(PerceptionError::UnitMissingSurface {
                id: unit.id,
                pos: unit.pos,
            });
        };
        observers.push((unit.pos, resolved.domain));
    }

    let mut targets = illumination.iter().collect::<BTreeMap<_, _>>();
    for (_, known) in prior_knowledge.faction(faction).surfaces() {
        let snapshot = known.snapshot();
        targets
            .entry(snapshot.pos)
            .or_insert_with(|| ResolvedLight {
                level: resolve_illumination_at(snapshot.pos, snapshot.domain, exterior, lights),
                domain: snapshot.domain,
            });
    }

    let mut observation = FactionObservation::new();
    for (target, target_light) in targets {
        if observers.iter().any(|(observer, domain)| {
            can_observe_resolved(*observer, *domain, target, target_light, profile)
        }) {
            observation.insert_surface(target);
        }
    }

    for unit in units.values().copied() {
        if illumination.get(unit.pos).is_some() && observation.observes(unit.pos) {
            observation.try_insert_unit(unit)?;
        }
    }
    Ok(observation)
}

fn can_observe_resolved(
    observer: TilePos,
    observer_domain: LightDomain,
    target: TilePos,
    target_light: ResolvedLight,
    profile: SightProfile,
) -> bool {
    if observer_domain != target_light.domain {
        return false;
    }

    let band = profile.band(target_light.level);
    if observer.level.abs_diff(target.level) > band.vertical {
        return false;
    }

    let downhill_levels = observer.level.saturating_sub(target.level).max(0);
    let horizontal_limit = band
        .horizontal
        .saturating_add(profile.downhill_bonus(target_light.level, downhill_levels));
    observer.coord.distance(target.coord) <= horizontal_limit
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::{GameplayLight, HexCoord, IlluminationLevel, InteriorRegionId, SightBand};

    use crate::{
        apply_observations, FactionObservation, FactionObservations, SurfaceSnapshot,
        SurfaceSnapshots,
    };

    fn pos(q: i32, r: i32, level: i32) -> TilePos {
        TilePos::new(HexCoord::from_axial(q, r), level)
    }

    fn profile(bright: u32, dim: u32, dark: u32, vertical: u32) -> SightProfile {
        SightProfile {
            bright: SightBand::new(bright, vertical),
            dim: SightBand::new(dim, vertical),
            dark: SightBand::new(dark, vertical),
            downhill_levels_per_bonus: 4,
            max_downhill_bonus: 6,
        }
    }

    fn light(
        position: TilePos,
        domain: LightDomain,
        level: IlluminationLevel,
        radius: u32,
    ) -> LightSourceSnapshot {
        LightSourceSnapshot {
            pos: position,
            domain,
            light: GameplayLight::new(level, radius),
        }
    }

    fn unit(id: u64, faction: Faction, position: TilePos) -> ObservedUnit {
        ObservedUnit {
            id: UnitId(id),
            faction,
            pos: position,
            provides_sight: true,
        }
    }

    fn inactive_unit(id: u64, faction: Faction, position: TilePos) -> ObservedUnit {
        ObservedUnit {
            provides_sight: false,
            ..unit(id, faction, position)
        }
    }

    fn surface(position: TilePos) -> SurfaceSnapshot {
        SurfaceSnapshot {
            pos: position,
            span: hex_core::HexSpan::new(1.0, 2.0),
            substance: hex_core::SubstanceId(1),
            headroom: hex_core::Headroom(2),
            is_solid: true,
            blocked: false,
            domain: LightDomain::Exterior,
        }
    }

    #[test]
    fn target_illumination_selects_band() {
        let observer = pos(0, 0, 5);
        let target = pos(3, 0, 5);
        let exterior = ExteriorIllumination::new(IlluminationLevel::Dark);
        let dim_target = light(target, LightDomain::Exterior, IlluminationLevel::Dim, 0);
        let bright_observer = light(
            observer,
            LightDomain::Exterior,
            IlluminationLevel::Bright,
            0,
        );
        let dim = ResolvedIllumination::try_resolve(
            [
                (observer, LightDomain::Exterior),
                (target, LightDomain::Exterior),
            ],
            exterior,
            &[bright_observer, dim_target],
        )
        .expect("illumination");
        assert!(!can_observe(observer, target, &dim, profile(4, 2, 1, 10)));

        let bright_target = light(target, LightDomain::Exterior, IlluminationLevel::Bright, 0);
        let bright = ResolvedIllumination::try_resolve(
            [
                (observer, LightDomain::Exterior),
                (target, LightDomain::Exterior),
            ],
            exterior,
            &[bright_observer, bright_target],
        )
        .expect("illumination");
        assert!(can_observe(observer, target, &bright, profile(4, 2, 1, 10)));
    }

    #[test]
    fn domains_and_vertical_band_are_independent_limits() {
        let observer = pos(0, 0, 5);
        let vertical_target = pos(1, 0, 8);
        let other_domain = pos(1, 0, 5);
        let cave = LightDomain::Interior(InteriorRegionId(1));
        let illumination = ResolvedIllumination::try_resolve(
            [
                (observer, LightDomain::Exterior),
                (vertical_target, LightDomain::Exterior),
                (other_domain, cave),
            ],
            ExteriorIllumination::new(IlluminationLevel::Bright),
            &[light(other_domain, cave, IlluminationLevel::Bright, 0)],
        )
        .expect("illumination");

        assert!(!can_observe(
            observer,
            vertical_target,
            &illumination,
            profile(5, 5, 1, 2)
        ));
        assert!(!can_observe(
            observer,
            other_domain,
            &illumination,
            profile(5, 5, 1, 10)
        ));
    }

    #[test]
    fn downhill_bonus_uses_complete_levels_caps_and_skips_darkness() {
        let observer = pos(0, 0, 24);
        let bright_at_cap = pos(8, 0, 0);
        let bright_past_cap = pos(9, 0, 0);
        let dark_downhill = pos(2, 0, 20);
        let exterior = ExteriorIllumination::new(IlluminationLevel::Dark);
        let illumination = ResolvedIllumination::try_resolve(
            [
                (observer, LightDomain::Exterior),
                (bright_at_cap, LightDomain::Exterior),
                (bright_past_cap, LightDomain::Exterior),
                (dark_downhill, LightDomain::Exterior),
            ],
            exterior,
            &[
                light(
                    bright_at_cap,
                    LightDomain::Exterior,
                    IlluminationLevel::Bright,
                    0,
                ),
                light(
                    bright_past_cap,
                    LightDomain::Exterior,
                    IlluminationLevel::Bright,
                    0,
                ),
            ],
        )
        .expect("illumination");
        let sight = profile(2, 2, 1, 30);

        assert!(can_observe(observer, bright_at_cap, &illumination, sight));
        assert!(!can_observe(
            observer,
            bright_past_cap,
            &illumination,
            sight
        ));
        assert!(!can_observe(observer, dark_downhill, &illumination, sight));
    }

    #[test]
    fn stacked_surfaces_are_tested_exactly() {
        let observer = pos(0, 0, 5);
        let near_stack = pos(0, 0, 7);
        let far_stack = pos(0, 0, 8);
        let illumination = ResolvedIllumination::try_resolve(
            [
                (observer, LightDomain::Exterior),
                (near_stack, LightDomain::Exterior),
                (far_stack, LightDomain::Exterior),
            ],
            ExteriorIllumination::new(IlluminationLevel::Bright),
            &[],
        )
        .expect("illumination");
        let sight = profile(1, 1, 1, 2);

        assert!(can_observe(observer, near_stack, &illumination, sight));
        assert!(!can_observe(observer, far_stack, &illumination, sight));
    }

    #[test]
    fn party_sight_is_the_union_of_all_active_units() {
        let left = pos(0, 0, 5);
        let right = pos(10, 0, 5);
        let target = pos(12, 0, 5);
        let illumination = ResolvedIllumination::try_resolve(
            [
                (left, LightDomain::Exterior),
                (right, LightDomain::Exterior),
                (target, LightDomain::Exterior),
            ],
            ExteriorIllumination::new(IlluminationLevel::Dim),
            &[],
        )
        .expect("illumination");
        let hostile = unit(9, Faction::Hostile, target);
        let observations = resolve_observations(
            [
                unit(1, Faction::Player, left),
                unit(2, Faction::Player, right),
                hostile,
            ],
            &illumination,
            &FactionMapKnowledge::new(),
            ExteriorIllumination::new(IlluminationLevel::Dim),
            &[],
            profile(4, 2, 1, 10),
        )
        .expect("observations");

        let player = observations.faction(Faction::Player);
        assert!(player.observes(target));
        assert_eq!(player.unit(hostile.id), Some(hostile));
    }

    #[test]
    fn inactive_units_remain_visible_without_extending_faction_sight() {
        let active = pos(0, 0, 5);
        let inactive = pos(1, 0, 5);
        let beyond = pos(2, 0, 5);
        let illumination = ResolvedIllumination::try_resolve(
            [
                (active, LightDomain::Exterior),
                (inactive, LightDomain::Exterior),
                (beyond, LightDomain::Exterior),
            ],
            ExteriorIllumination::new(IlluminationLevel::Bright),
            &[],
        )
        .expect("illumination");
        let inactive_player = inactive_unit(2, Faction::Player, inactive);
        let hidden_hostile = unit(3, Faction::Hostile, beyond);

        let observations = resolve_observations(
            [
                unit(1, Faction::Player, active),
                inactive_player,
                hidden_hostile,
            ],
            &illumination,
            &FactionMapKnowledge::new(),
            ExteriorIllumination::new(IlluminationLevel::Bright),
            &[],
            profile(1, 1, 1, 10),
        )
        .expect("observations");

        let player = observations.faction(Faction::Player);
        assert_eq!(player.unit(inactive_player.id), Some(inactive_player));
        assert!(player.observes(inactive));
        assert_eq!(player.unit(hidden_hostile.id), None);
        assert!(
            !player.observes(beyond),
            "the inactive unit must not extend sight one more hex"
        );
    }

    #[test]
    fn target_light_can_make_detection_asymmetric() {
        let player_pos = pos(0, 0, 5);
        let hostile_pos = pos(3, 0, 5);
        let exterior = ExteriorIllumination::new(IlluminationLevel::Dark);
        let illumination = ResolvedIllumination::try_resolve(
            [
                (player_pos, LightDomain::Exterior),
                (hostile_pos, LightDomain::Exterior),
            ],
            exterior,
            &[light(
                player_pos,
                LightDomain::Exterior,
                IlluminationLevel::Bright,
                0,
            )],
        )
        .expect("illumination");
        let observations = resolve_observations(
            [
                unit(1, Faction::Player, player_pos),
                unit(2, Faction::Hostile, hostile_pos),
            ],
            &illumination,
            &FactionMapKnowledge::new(),
            exterior,
            &[],
            profile(4, 2, 1, 10),
        )
        .expect("observations");

        assert!(!observations.faction(Faction::Player).observes(hostile_pos));
        assert!(observations.faction(Faction::Hostile).observes(player_pos));
    }

    #[test]
    fn duplicate_unit_id_is_rejected_before_observation() {
        let position = pos(0, 0, 5);
        let illumination = ResolvedIllumination::try_resolve(
            [(position, LightDomain::Exterior)],
            ExteriorIllumination::new(IlluminationLevel::Bright),
            &[],
        )
        .expect("illumination");
        let duplicate = UnitId(4);
        let error = resolve_observations(
            [
                unit(duplicate.0, Faction::Player, position),
                unit(duplicate.0, Faction::Hostile, position),
            ],
            &illumination,
            &FactionMapKnowledge::new(),
            ExteriorIllumination::new(IlluminationLevel::Bright),
            &[],
            SightProfile::default(),
        )
        .expect_err("duplicate stable identity must fail");
        assert_eq!(error, PerceptionError::DuplicateUnit(duplicate));
    }

    #[test]
    fn duplicate_unit_error_selects_the_lowest_id_in_any_input_order() {
        let position = pos(0, 0, 5);
        let illumination = ResolvedIllumination::try_resolve(
            [(position, LightDomain::Exterior)],
            ExteriorIllumination::new(IlluminationLevel::Bright),
            &[],
        )
        .expect("illumination");
        let units = |first, second| {
            [
                unit(first, Faction::Player, position),
                unit(second, Faction::Hostile, position),
                unit(first, Faction::Hostile, position),
                unit(second, Faction::Player, position),
            ]
        };
        let resolve = |units| {
            resolve_observations(
                units,
                &illumination,
                &FactionMapKnowledge::new(),
                ExteriorIllumination::new(IlluminationLevel::Bright),
                &[],
                SightProfile::default(),
            )
            .expect_err("duplicates must fail")
        };

        let forward = resolve(units(9, 2));
        let reverse = resolve(units(2, 9));
        assert_eq!(forward, PerceptionError::DuplicateUnit(UnitId(2)));
        assert_eq!(reverse, forward);
    }

    #[test]
    fn unit_without_an_exposed_surface_is_rejected() {
        let surface = pos(0, 0, 5);
        let missing = pos(1, 0, 5);
        let illumination = ResolvedIllumination::try_resolve(
            [(surface, LightDomain::Exterior)],
            ExteriorIllumination::new(IlluminationLevel::Bright),
            &[],
        )
        .expect("illumination");
        let id = UnitId(7);

        let error = resolve_observations(
            [unit(id.0, Faction::Player, missing)],
            &illumination,
            &FactionMapKnowledge::new(),
            ExteriorIllumination::new(IlluminationLevel::Bright),
            &[],
            SightProfile::default(),
        )
        .expect_err("an invalid unit projection must not silently blind its faction");

        assert_eq!(
            error,
            PerceptionError::UnitMissingSurface { id, pos: missing }
        );
    }

    #[test]
    fn resolver_emits_only_deleted_known_positions_still_in_sight() {
        let observer = pos(0, 0, 5);
        let visible_deleted = pos(1, 0, 5);
        let hidden_deleted = pos(5, 0, 5);
        let original = SurfaceSnapshots::try_from_iter([
            surface(observer),
            surface(visible_deleted),
            surface(hidden_deleted),
        ])
        .expect("original surfaces");
        let mut initial_player = FactionObservation::new();
        for position in [observer, visible_deleted, hidden_deleted] {
            initial_player.insert_surface(position);
        }
        let mut knowledge = FactionMapKnowledge::new();
        apply_observations(
            &mut knowledge,
            &original,
            &FactionObservations::with_faction(Faction::Player, initial_player),
        );

        let illumination = ResolvedIllumination::try_resolve(
            [(observer, LightDomain::Exterior)],
            ExteriorIllumination::new(IlluminationLevel::Bright),
            &[],
        )
        .expect("remaining surface illumination");
        let observations = resolve_observations(
            [unit(1, Faction::Player, observer)],
            &illumination,
            &knowledge,
            ExteriorIllumination::new(IlluminationLevel::Bright),
            &[],
            profile(2, 2, 1, 10),
        )
        .expect("observations");

        let player = observations.faction(Faction::Player);
        assert!(player.observes(visible_deleted));
        assert!(!player.observes(hidden_deleted));

        let remaining =
            SurfaceSnapshots::try_from_iter([surface(observer)]).expect("remaining surface");
        apply_observations(&mut knowledge, &remaining, &observations);
        assert_eq!(
            knowledge.faction(Faction::Player).state(visible_deleted),
            hex_core::KnowledgeState::Unknown
        );
        assert_eq!(
            knowledge.faction(Faction::Player).state(hidden_deleted),
            hex_core::KnowledgeState::Remembered
        );
    }
}
