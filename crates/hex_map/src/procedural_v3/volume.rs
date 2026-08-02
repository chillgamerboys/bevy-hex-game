//! Recipe-independent semantic volume storage for procedural generator V3.
//!
//! A V3 recipe describes occupied intervals before it writes voxels. Air is
//! implicit, so gaps naturally represent caves, the space below floating islands,
//! and the clearance between stacked floors. The horizontal mask is explicit and
//! need not be a radius-shaped footprint.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use hex_core::{
    Headroom, HexCoord, InteriorRegionId, InteriorRegions, Level, SpecialMovementRegion,
    SpecialMovementRegions, SubstanceId, TilePos, TraversalProfile, MAX_HEADROOM,
};

use crate::terrain::TerrainPalette;
use crate::voxel::{Column, VoxelMap};

/// Highest voxel level admitted by a V3 semantic volume.
///
/// The bound keeps malformed settings from causing an unbounded allocation during
/// materialization. Intervals use an exclusive top, so their greatest valid top is
/// one greater than this value.
pub(crate) const MAX_VOLUME_LEVEL: Level = 128;

/// Inclusive-bottom, exclusive-top vertical interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct LevelInterval {
    pub(crate) bottom: Level,
    pub(crate) top: Level,
}

impl LevelInterval {
    #[must_use]
    pub(crate) const fn new(bottom: Level, top: Level) -> Self {
        Self { bottom, top }
    }

    #[must_use]
    const fn contains(self, level: Level) -> bool {
        self.bottom <= level && level < self.top
    }
}

/// A material which contributes solid voxels to the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SolidMaterialRole {
    Bedrock,
    Stone,
    Dirt,
    Grass,
    Gravel,
    Metal,
    WorkedStone,
    Snow,
    Ice,
    Basalt,
    Limestone,
    Slate,
    Timber,
    Terracotta,
}

/// A visible material which occupies volume but cannot support footing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FillMaterialRole {
    Water,
    Lava,
}

/// Whether one semantic material role contributes solid or non-solid occupancy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MaterialRole {
    Solid(SolidMaterialRole),
    Fill(FillMaterialRole),
}

/// One solid material interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SolidMass {
    pub(crate) levels: LevelInterval,
    pub(crate) material: SolidMaterialRole,
    /// Interior whose presentation may hide these exact roof voxels.
    pub(crate) cutaway_for: Option<InteriorRegionId>,
}

/// One non-solid material interval. Unlisted space is air.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NonSolidFill {
    pub(crate) levels: LevelInterval,
    pub(crate) material: FillMaterialRole,
}

/// One occupied interval in a semantic column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VolumeElement {
    Solid(SolidMass),
    Fill(NonSolidFill),
}

impl VolumeElement {
    #[must_use]
    const fn levels(self) -> LevelInterval {
        match self {
            Self::Solid(mass) => mass.levels,
            Self::Fill(fill) => fill.levels,
        }
    }
}

/// Ordered occupied intervals at one horizontal coordinate.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct VolumeColumn {
    pub(crate) elements: Vec<VolumeElement>,
}

impl VolumeColumn {
    /// Counts implicit air from `from`, using the same saturation rule as
    /// [`Column::headroom_above`].
    #[must_use]
    pub(crate) fn headroom_above(&self, from: Level) -> Headroom {
        let clear = self
            .elements
            .iter()
            .copied()
            .filter_map(|element| {
                let levels = element.levels();
                if levels.contains(from) {
                    Some(0)
                } else {
                    (levels.bottom > from).then_some(levels.bottom.saturating_sub(from))
                }
            })
            .min()
            .unwrap_or(MAX_HEADROOM)
            .clamp(0, MAX_HEADROOM);
        Headroom(clear)
    }
}

/// How ordinary traversal should classify an exact exposed solid boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SurfaceAccess {
    /// Part of the ordinary walker network.
    Ordinary,
    /// Geometrically standable, but outside the ordinary walker network.
    SpecialMovement(SpecialMovementRegion),
    /// Not valid footing, for example a submerged bed, low opening, or authored
    /// slippery Ice hazard.
    NonStandable,
}

/// Semantic facts attached to one exact exposed solid surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SurfaceMetadata {
    pub(crate) access: SurfaceAccess,
    /// Interior domain containing this exact floor, if any.
    pub(crate) interior: Option<InteriorRegionId>,
}

/// One recipe-independent semantic volume contract violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VolumeIssue {
    EmptyMask,
    DisconnectedMask,
    MaskCoverage {
        missing: Vec<HexCoord>,
        extra: Vec<HexCoord>,
    },
    InvalidInterval {
        coord: HexCoord,
        index: usize,
        levels: LevelInterval,
    },
    UnorderedIntervals {
        coord: HexCoord,
        index: usize,
    },
    OverlappingIntervals {
        coord: HexCoord,
        before: LevelInterval,
        after: LevelInterval,
    },
    MergeableAdjacent {
        coord: HexCoord,
        before_index: usize,
        after_index: usize,
    },
    NoSolidMasses,
    SurfaceMetadataMismatch {
        missing: Vec<TilePos>,
        extra: Vec<TilePos>,
    },
    InsufficientHeadroom {
        surface: TilePos,
        clear_levels: Level,
    },
    NonStandableWithHeadroom {
        surface: TilePos,
        clear_levels: Level,
    },
}

impl fmt::Display for VolumeIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyMask => formatter.write_str("volume mask is empty"),
            Self::DisconnectedMask => {
                formatter.write_str("volume mask is not one horizontally connected component")
            }
            Self::MaskCoverage { missing, extra } => write!(
                formatter,
                "volume columns do not exactly cover the mask (missing {missing:?}, extra \
                 {extra:?})"
            ),
            Self::InvalidInterval {
                coord,
                index,
                levels,
            } => write!(
                formatter,
                "column {coord:?} element {index} has out-of-range interval {levels:?}"
            ),
            Self::UnorderedIntervals { coord, index } => write!(
                formatter,
                "column {coord:?} elements are not ordered at index {index}"
            ),
            Self::OverlappingIntervals {
                coord,
                before,
                after,
            } => write!(
                formatter,
                "column {coord:?} has overlapping intervals {before:?} and {after:?}"
            ),
            Self::MergeableAdjacent {
                coord,
                before_index,
                after_index,
            } => write!(
                formatter,
                "column {coord:?} has adjacent mergeable elements at index {before_index} and \
                 {after_index}"
            ),
            Self::NoSolidMasses => formatter.write_str("volume contains no solid masses"),
            Self::SurfaceMetadataMismatch { missing, extra } => write!(
                formatter,
                "surface metadata does not exactly match exposed solid boundaries (missing \
                 {missing:?}, extra {extra:?})"
            ),
            Self::InsufficientHeadroom {
                surface,
                clear_levels,
            } => write!(
                formatter,
                "standable surface {surface:?} has only {clear_levels} clear level(s)"
            ),
            Self::NonStandableWithHeadroom {
                surface,
                clear_levels,
            } => write!(
                formatter,
                "surface {surface:?} is marked non-standable despite {clear_levels} clear level(s)"
            ),
        }
    }
}

/// Complete recipe-independent V3 geometry before voxel materialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VolumePlan {
    /// Exact horizontal footprint owned by this volume.
    pub(crate) mask: BTreeSet<HexCoord>,
    /// One semantic column for every coordinate in `mask`, including all-air
    /// columns.
    pub(crate) columns: BTreeMap<HexCoord, VolumeColumn>,
    /// Metadata for every and only every exposed upward solid boundary.
    pub(crate) surfaces: BTreeMap<TilePos, SurfaceMetadata>,
}

impl VolumePlan {
    /// Creates an all-air volume over an arbitrary connected mask.
    #[must_use]
    pub(crate) fn new(mask: BTreeSet<HexCoord>) -> Self {
        let columns = mask
            .iter()
            .copied()
            .map(|coord| (coord, VolumeColumn::default()))
            .collect();
        Self {
            mask,
            columns,
            surfaces: BTreeMap::new(),
        }
    }

    /// Measures the headroom above one declared exact surface.
    #[must_use]
    pub(crate) fn surface_headroom(&self, surface: TilePos) -> Option<Headroom> {
        self.surfaces.get(&surface)?;
        self.columns
            .get(&surface.coord)
            .map(|column| column.headroom_above(surface.level.saturating_add(1)))
    }

    /// Returns every non-solid fill run keyed by its top occupied voxel.
    ///
    /// This is a run identity, not an exposed-surface query. A fill immediately
    /// below a bridge, roof, or another occupied interval is retained so stacked
    /// liquid topology cannot collapse to one entry per horizontal coordinate.
    #[must_use]
    pub(crate) fn fill_runs_by_top(&self) -> BTreeMap<TilePos, NonSolidFill> {
        self.columns
            .iter()
            .flat_map(|(coord, column)| {
                column.elements.iter().filter_map(move |element| {
                    let VolumeElement::Fill(fill) = *element else {
                        return None;
                    };
                    (fill.levels.bottom < fill.levels.top).then(|| {
                        (
                            TilePos::new(*coord, fill.levels.top.saturating_sub(1)),
                            fill,
                        )
                    })
                })
            })
            .collect()
    }

    /// Checks mask, interval, surface, and access invariants shared by every recipe.
    pub(crate) fn validate(&self) -> Result<(), Vec<VolumeIssue>> {
        let mut issues = Vec::new();

        if self.mask.is_empty() {
            issues.push(VolumeIssue::EmptyMask);
        } else if !mask_is_connected(&self.mask) {
            issues.push(VolumeIssue::DisconnectedMask);
        }

        if self.columns.len() != self.mask.len()
            || !self.columns.keys().copied().eq(self.mask.iter().copied())
        {
            let column_coords: BTreeSet<_> = self.columns.keys().copied().collect();
            let missing: Vec<_> = self.mask.difference(&column_coords).copied().collect();
            let extra: Vec<_> = column_coords.difference(&self.mask).copied().collect();
            issues.push(VolumeIssue::MaskCoverage { missing, extra });
        }

        let mut expected_surfaces = BTreeSet::new();
        let mut solid_count = 0_usize;
        for (coord, column) in &self.columns {
            validate_column(*coord, column, &mut issues);
            for (index, element) in column.elements.iter().copied().enumerate() {
                let VolumeElement::Solid(mass) = element else {
                    continue;
                };
                solid_count = solid_count.saturating_add(1);
                if mass.levels.bottom >= mass.levels.top {
                    continue;
                }
                let covered_by_solid = column.elements.get(index + 1).is_some_and(|next| {
                    matches!(next, VolumeElement::Solid(_))
                        && next.levels().bottom == mass.levels.top
                });
                if !covered_by_solid {
                    expected_surfaces
                        .insert(TilePos::new(*coord, mass.levels.top.saturating_sub(1)));
                }
            }
        }

        if solid_count == 0 {
            issues.push(VolumeIssue::NoSolidMasses);
        }

        if self.surfaces.len() != expected_surfaces.len()
            || !self
                .surfaces
                .keys()
                .copied()
                .eq(expected_surfaces.iter().copied())
        {
            let actual_surfaces: BTreeSet<_> = self.surfaces.keys().copied().collect();
            let missing: Vec<_> = expected_surfaces
                .difference(&actual_surfaces)
                .copied()
                .collect();
            let extra: Vec<_> = actual_surfaces
                .difference(&expected_surfaces)
                .copied()
                .collect();
            issues.push(VolumeIssue::SurfaceMetadataMismatch { missing, extra });
        }

        for (surface, metadata) in &self.surfaces {
            if !expected_surfaces.contains(surface) {
                continue;
            }
            let Some(headroom) = self
                .columns
                .get(&surface.coord)
                .map(|column| column.headroom_above(surface.level.saturating_add(1)))
            else {
                continue;
            };
            let walker_admitted = TraversalProfile::WALKER.admits_surface(true, headroom);
            let authored_ice_hazard = self.columns.get(&surface.coord).is_some_and(|column| {
                column.elements.iter().any(|element| {
                    matches!(
                        element,
                        VolumeElement::Solid(mass)
                            if mass.material == SolidMaterialRole::Ice
                                && mass.levels.contains(surface.level)
                    )
                })
            });
            match metadata.access {
                SurfaceAccess::Ordinary | SurfaceAccess::SpecialMovement(_) if !walker_admitted => {
                    issues.push(VolumeIssue::InsufficientHeadroom {
                        surface: *surface,
                        clear_levels: headroom.0,
                    });
                }
                SurfaceAccess::NonStandable if walker_admitted && !authored_ice_hazard => {
                    issues.push(VolumeIssue::NonStandableWithHeadroom {
                        surface: *surface,
                        clear_levels: headroom.0,
                    });
                }
                SurfaceAccess::Ordinary
                | SurfaceAccess::SpecialMovement(_)
                | SurfaceAccess::NonStandable => {}
            }
        }

        if issues.is_empty() {
            Ok(())
        } else {
            Err(issues)
        }
    }

    /// Validates and resolves semantic roles into concrete voxel substances.
    pub(crate) fn materialize(
        &self,
        palette: &TerrainPalette,
        is_solid: &dyn Fn(SubstanceId) -> bool,
    ) -> Result<MaterializedVolume, VolumeMaterializationError> {
        self.validate()
            .map_err(VolumeMaterializationError::InvalidVolume)?;

        let mut map = VoxelMap::new();
        let mut interiors = InteriorRegions::new();
        let mut special_regions = SpecialMovementRegions::new();

        for (coord, planned) in &self.columns {
            let mut column = Column::new();
            for element in &planned.elements {
                let (levels, substance, expected_solid, role) = match *element {
                    VolumeElement::Solid(mass) => (
                        mass.levels,
                        solid_substance(mass.material, palette),
                        true,
                        MaterialRole::Solid(mass.material),
                    ),
                    VolumeElement::Fill(fill) => (
                        fill.levels,
                        fill_substance(fill.material, palette),
                        false,
                        MaterialRole::Fill(fill.material),
                    ),
                };
                if substance.is_air() || is_solid(substance) != expected_solid {
                    return Err(VolumeMaterializationError::MaterialContract {
                        coord: *coord,
                        role,
                        substance,
                        expected_solid,
                    });
                }
                for level in levels.bottom..levels.top {
                    column.set(level, substance);
                }

                if let VolumeElement::Solid(SolidMass {
                    cutaway_for: Some(region),
                    ..
                }) = *element
                {
                    for level in levels.bottom..levels.top {
                        let _previous =
                            interiors.insert_roof_voxel(TilePos::new(*coord, level), region);
                    }
                }
            }
            map.insert_column(*coord, column);
        }

        for (surface, metadata) in &self.surfaces {
            if let Some(region) = metadata.interior {
                let _previous = interiors.insert_surface(*surface, region);
            }
            if let SurfaceAccess::SpecialMovement(region) = metadata.access {
                let _previous = special_regions.insert(*surface, region);
            }
        }

        Ok(MaterializedVolume {
            map,
            interiors,
            special_regions,
        })
    }
}

fn validate_column(coord: HexCoord, column: &VolumeColumn, issues: &mut Vec<VolumeIssue>) {
    let mut previous: Option<VolumeElement> = None;
    for (index, element) in column.elements.iter().copied().enumerate() {
        let levels = element.levels();
        if levels.bottom < 0
            || levels.bottom >= levels.top
            || levels.top > MAX_VOLUME_LEVEL.saturating_add(1)
        {
            issues.push(VolumeIssue::InvalidInterval {
                coord,
                index,
                levels,
            });
        }

        for before in column.elements.iter().copied().take(index) {
            let before_levels = before.levels();
            if intervals_overlap(before_levels, levels) {
                issues.push(VolumeIssue::OverlappingIntervals {
                    coord,
                    before: before_levels,
                    after: levels,
                });
            }
        }

        if let Some(before) = previous {
            let before_levels = before.levels();
            if levels.bottom < before_levels.bottom {
                issues.push(VolumeIssue::UnorderedIntervals { coord, index });
            }
            if levels.bottom == before_levels.top
                && mergeable_without_losing_semantics(before, element)
            {
                issues.push(VolumeIssue::MergeableAdjacent {
                    coord,
                    before_index: index.saturating_sub(1),
                    after_index: index,
                });
            }
        }
        previous = Some(element);
    }
}

const fn intervals_overlap(left: LevelInterval, right: LevelInterval) -> bool {
    left.bottom < right.top && right.bottom < left.top
}

fn mergeable_without_losing_semantics(left: VolumeElement, right: VolumeElement) -> bool {
    match (left, right) {
        (VolumeElement::Solid(left), VolumeElement::Solid(right)) => {
            left.material == right.material && left.cutaway_for == right.cutaway_for
        }
        (VolumeElement::Fill(left), VolumeElement::Fill(right)) => left.material == right.material,
        _ => false,
    }
}

fn mask_is_connected(mask: &BTreeSet<HexCoord>) -> bool {
    let Some(start) = mask.first().copied() else {
        return false;
    };
    let mut reached = BTreeSet::from([start]);
    let mut frontier = VecDeque::from([start]);
    while let Some(coord) = frontier.pop_front() {
        for neighbour in coord.neighbors() {
            if mask.contains(&neighbour) && reached.insert(neighbour) {
                frontier.push_back(neighbour);
            }
        }
    }
    reached.len() == mask.len()
}

const fn solid_substance(role: SolidMaterialRole, palette: &TerrainPalette) -> SubstanceId {
    match role {
        SolidMaterialRole::Bedrock => palette.bedrock,
        SolidMaterialRole::Stone => palette.stone,
        SolidMaterialRole::Dirt => palette.dirt,
        SolidMaterialRole::Grass => palette.grass,
        SolidMaterialRole::Gravel => palette.gravel,
        SolidMaterialRole::Metal => palette.metal,
        SolidMaterialRole::WorkedStone => palette.worked_stone,
        SolidMaterialRole::Snow => palette.snow,
        SolidMaterialRole::Ice => palette.ice,
        SolidMaterialRole::Basalt => palette.basalt,
        SolidMaterialRole::Limestone => palette.limestone,
        SolidMaterialRole::Slate => palette.slate,
        SolidMaterialRole::Timber => palette.timber,
        SolidMaterialRole::Terracotta => palette.terracotta,
    }
}

const fn fill_substance(role: FillMaterialRole, palette: &TerrainPalette) -> SubstanceId {
    match role {
        FillMaterialRole::Water => palette.water,
        FillMaterialRole::Lava => palette.lava,
    }
}

/// Failure while turning an admitted semantic volume into runtime resources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VolumeMaterializationError {
    InvalidVolume(Vec<VolumeIssue>),
    MaterialContract {
        coord: HexCoord,
        role: MaterialRole,
        substance: SubstanceId,
        expected_solid: bool,
    },
}

impl fmt::Display for VolumeMaterializationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidVolume(issues) => {
                formatter.write_str("invalid V3 semantic volume: ")?;
                for (index, issue) in issues.iter().enumerate() {
                    if index != 0 {
                        formatter.write_str("; ")?;
                    }
                    issue.fmt(formatter)?;
                }
                Ok(())
            }
            Self::MaterialContract {
                coord,
                role,
                substance,
                expected_solid,
            } => write!(
                formatter,
                "V3 material role {role:?} at {coord:?} resolved to {substance:?}; expected a {} \
                 non-air substance",
                if *expected_solid {
                    "solid"
                } else {
                    "non-solid"
                }
            ),
        }
    }
}

impl std::error::Error for VolumeMaterializationError {}

/// Runtime terrain and the exact metadata projected from a V3 semantic volume.
#[derive(Debug)]
pub(crate) struct MaterializedVolume {
    pub(crate) map: VoxelMap,
    pub(crate) interiors: InteriorRegions,
    pub(crate) special_regions: SpecialMovementRegions,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mass(
        bottom: Level,
        top: Level,
        material: SolidMaterialRole,
        cutaway_for: Option<InteriorRegionId>,
    ) -> VolumeElement {
        VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(bottom, top),
            material,
            cutaway_for,
        })
    }

    fn fill(bottom: Level, top: Level, material: FillMaterialRole) -> VolumeElement {
        VolumeElement::Fill(NonSolidFill {
            levels: LevelInterval::new(bottom, top),
            material,
        })
    }

    fn surface(access: SurfaceAccess, interior: Option<InteriorRegionId>) -> SurfaceMetadata {
        SurfaceMetadata { access, interior }
    }

    fn palette() -> TerrainPalette {
        TerrainPalette {
            bedrock: SubstanceId(1),
            stone: SubstanceId(2),
            dirt: SubstanceId(3),
            grass: SubstanceId(4),
            gravel: SubstanceId(5),
            water: SubstanceId(6),
            metal: SubstanceId(7),
            worked_stone: SubstanceId(12),
            limestone: SubstanceId(13),
            slate: SubstanceId(14),
            timber: SubstanceId(15),
            terracotta: SubstanceId(16),
            snow: SubstanceId(8),
            ice: SubstanceId(9),
            basalt: SubstanceId(10),
            lava: SubstanceId(11),
        }
    }

    fn test_is_solid(substance: SubstanceId) -> bool {
        matches!(
            substance.0,
            1 | 2 | 3 | 4 | 5 | 7 | 8 | 9 | 10 | 12 | 13 | 14 | 15 | 16
        )
    }

    fn issues_text(issues: &[VolumeIssue]) -> String {
        issues
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    }

    #[test]
    fn fill_run_tops_preserve_covered_and_stacked_liquids() {
        let coord = HexCoord::ORIGIN;
        let mut plan = VolumePlan::new(BTreeSet::from([coord]));
        plan.columns
            .get_mut(&coord)
            .expect("the origin is in the mask")
            .elements = vec![
            mass(0, 1, SolidMaterialRole::Stone, None),
            fill(1, 3, FillMaterialRole::Water),
            mass(3, 4, SolidMaterialRole::Metal, None),
            fill(6, 8, FillMaterialRole::Lava),
        ];

        assert_eq!(
            plan.fill_runs_by_top(),
            BTreeMap::from([
                (
                    TilePos::new(coord, 2),
                    NonSolidFill {
                        levels: LevelInterval::new(1, 3),
                        material: FillMaterialRole::Water,
                    },
                ),
                (
                    TilePos::new(coord, 7),
                    NonSolidFill {
                        levels: LevelInterval::new(6, 8),
                        material: FillMaterialRole::Lava,
                    },
                ),
            ])
        );
    }

    #[test]
    fn arbitrary_connected_mask_is_exactly_covered() {
        let origin = HexCoord::ORIGIN;
        let [east, ..] = origin.neighbors();
        let [_, bend, ..] = east.neighbors();
        let mask = BTreeSet::from([origin, east, bend]);
        let mut plan = VolumePlan::new(mask.clone());
        for coord in &mask {
            plan.columns
                .get_mut(coord)
                .expect("the constructor should create every masked column")
                .elements = vec![mass(0, 1, SolidMaterialRole::Stone, None)];
            plan.surfaces.insert(
                TilePos::new(*coord, 0),
                surface(SurfaceAccess::Ordinary, None),
            );
        }

        assert!(plan.validate().is_ok());

        let removed = *mask.first().expect("the test mask is nonempty");
        plan.columns.remove(&removed);
        let outside = HexCoord::from_axial(20, -7);
        plan.columns.insert(outside, VolumeColumn::default());
        let error = plan.validate().expect_err("mask substitution must fail");
        assert!(issues_text(&error).contains("exactly cover the mask"));
    }

    #[test]
    fn disconnected_mask_is_rejected() {
        let mask = BTreeSet::from([HexCoord::ORIGIN, HexCoord::from_axial(4, 0)]);
        let plan = VolumePlan::new(mask);
        let issues = plan
            .validate()
            .expect_err("a V3 footprint must be connected");
        assert!(issues_text(&issues).contains("not one horizontally connected"));
    }

    #[test]
    fn overlap_out_of_range_and_noncanonical_intervals_are_rejected() {
        let coord = HexCoord::ORIGIN;
        let mut plan = VolumePlan::new(BTreeSet::from([coord]));
        plan.columns
            .get_mut(&coord)
            .expect("the origin is in the test mask")
            .elements = vec![
            mass(0, 4, SolidMaterialRole::Stone, None),
            mass(3, 5, SolidMaterialRole::Dirt, None),
            fill(
                MAX_VOLUME_LEVEL,
                MAX_VOLUME_LEVEL.saturating_add(2),
                FillMaterialRole::Water,
            ),
        ];
        plan.surfaces.insert(
            TilePos::new(coord, 4),
            surface(SurfaceAccess::Ordinary, None),
        );

        let error = plan.validate().expect_err("invalid intervals must fail");
        let text = issues_text(&error);
        assert!(text.contains("overlapping intervals"));
        assert!(text.contains("out-of-range interval"));

        plan.columns
            .get_mut(&coord)
            .expect("the origin is in the test mask")
            .elements = vec![
            mass(0, 2, SolidMaterialRole::Stone, None),
            mass(2, 4, SolidMaterialRole::Stone, None),
        ];
        plan.surfaces.clear();
        plan.surfaces.insert(
            TilePos::new(coord, 3),
            surface(SurfaceAccess::Ordinary, None),
        );
        let issues = plan
            .validate()
            .expect_err("mergeable runs are noncanonical");
        assert!(issues_text(&issues).contains("adjacent mergeable"));
    }

    #[test]
    fn gaps_and_stacked_surfaces_preserve_exact_metadata() {
        let coord = HexCoord::ORIGIN;
        let cave = InteriorRegionId(4);
        let upper = SpecialMovementRegion(9);
        let mut plan = VolumePlan::new(BTreeSet::from([coord]));
        plan.columns
            .get_mut(&coord)
            .expect("the origin is in the test mask")
            .elements = vec![
            mass(0, 5, SolidMaterialRole::Stone, None),
            mass(8, 10, SolidMaterialRole::Stone, Some(cave)),
        ];
        let lower_surface = TilePos::new(coord, 4);
        let upper_surface = TilePos::new(coord, 9);
        plan.surfaces
            .insert(lower_surface, surface(SurfaceAccess::Ordinary, Some(cave)));
        plan.surfaces.insert(
            upper_surface,
            surface(SurfaceAccess::SpecialMovement(upper), None),
        );

        assert_eq!(plan.surface_headroom(lower_surface), Some(Headroom(3)));
        assert_eq!(
            plan.surface_headroom(upper_surface),
            Some(Headroom(MAX_HEADROOM))
        );
        assert!(plan.validate().is_ok());

        let materialized = plan
            .materialize(&palette(), &test_is_solid)
            .expect("stacked geometry should materialize");
        for level in 5..8 {
            assert!(materialized.map.get(TilePos::new(coord, level)).is_air());
        }
        assert_eq!(materialized.interiors.get(lower_surface), Some(cave));
        assert_eq!(
            materialized.interiors.roof_region(TilePos::new(coord, 8)),
            Some(cave)
        );
        assert_eq!(materialized.special_regions.get(upper_surface), Some(upper));
    }

    #[test]
    fn surface_metadata_must_match_every_exposed_boundary_exactly() {
        let coord = HexCoord::ORIGIN;
        let mut plan = VolumePlan::new(BTreeSet::from([coord]));
        plan.columns
            .get_mut(&coord)
            .expect("the origin is in the test mask")
            .elements = vec![mass(0, 4, SolidMaterialRole::Grass, None)];
        plan.surfaces.insert(
            TilePos::new(coord, 2),
            surface(SurfaceAccess::Ordinary, None),
        );

        let error = plan
            .validate()
            .expect_err("shifted metadata cannot name the surface");
        let text = issues_text(&error);
        assert!(text.contains("exposed solid boundaries"));
        assert!(text.contains("missing"));
        assert!(text.contains("extra"));
    }

    #[test]
    fn one_clear_level_rejects_walker_but_two_clear_levels_admit_it() {
        let low = HexCoord::ORIGIN;
        let [high, ..] = low.neighbors();
        let mut plan = VolumePlan::new(BTreeSet::from([low, high]));
        plan.columns
            .get_mut(&low)
            .expect("the low coordinate is in the test mask")
            .elements = vec![
            mass(0, 5, SolidMaterialRole::Stone, None),
            mass(6, 8, SolidMaterialRole::Stone, None),
        ];
        plan.columns
            .get_mut(&high)
            .expect("the high coordinate is in the test mask")
            .elements = vec![
            mass(0, 5, SolidMaterialRole::Stone, None),
            mass(7, 9, SolidMaterialRole::Stone, None),
        ];
        let low_floor = TilePos::new(low, 4);
        let high_floor = TilePos::new(high, 4);
        for (position, access) in [
            (low_floor, SurfaceAccess::NonStandable),
            (TilePos::new(low, 7), SurfaceAccess::Ordinary),
            (high_floor, SurfaceAccess::Ordinary),
            (TilePos::new(high, 8), SurfaceAccess::Ordinary),
        ] {
            plan.surfaces.insert(position, surface(access, None));
        }

        assert_eq!(plan.surface_headroom(low_floor), Some(Headroom(1)));
        assert_eq!(plan.surface_headroom(high_floor), Some(Headroom(2)));
        assert!(plan.validate().is_ok());

        plan.surfaces
            .get_mut(&low_floor)
            .expect("the low floor metadata was inserted")
            .access = SurfaceAccess::Ordinary;
        let issues = plan
            .validate()
            .expect_err("one clear level cannot admit the walker");
        assert!(issues_text(&issues).contains("only 1 clear level"));
    }

    #[test]
    fn exposed_ice_may_be_an_authored_nonstandable_hazard() {
        let coord = HexCoord::ORIGIN;
        let mut plan = VolumePlan::new(BTreeSet::from([coord]));
        plan.columns
            .get_mut(&coord)
            .expect("the origin is in the test mask")
            .elements = vec![mass(0, 2, SolidMaterialRole::Ice, None)];
        let ice = TilePos::new(coord, 1);
        plan.surfaces
            .insert(ice, surface(SurfaceAccess::NonStandable, None));

        assert_eq!(plan.surface_headroom(ice), Some(Headroom(MAX_HEADROOM)));
        assert!(plan.validate().is_ok());

        plan.columns
            .get_mut(&coord)
            .expect("the origin is in the test mask")
            .elements = vec![mass(0, 2, SolidMaterialRole::Stone, None)];
        let issues = plan
            .validate()
            .expect_err("ordinary stone with full headroom cannot hide from traversal");
        assert!(issues_text(&issues).contains("marked non-standable"));
    }

    #[test]
    fn materialization_maps_every_v3_material_role_and_preserves_air_gaps() {
        let coord = HexCoord::ORIGIN;
        let mut plan = VolumePlan::new(BTreeSet::from([coord]));
        plan.columns
            .get_mut(&coord)
            .expect("the origin is in the test mask")
            .elements = vec![
            mass(0, 1, SolidMaterialRole::Bedrock, None),
            mass(1, 2, SolidMaterialRole::Stone, None),
            mass(2, 3, SolidMaterialRole::Dirt, None),
            mass(3, 4, SolidMaterialRole::Grass, None),
            mass(4, 5, SolidMaterialRole::Gravel, None),
            mass(5, 6, SolidMaterialRole::Metal, None),
            mass(6, 7, SolidMaterialRole::WorkedStone, None),
            mass(7, 8, SolidMaterialRole::Snow, None),
            mass(8, 9, SolidMaterialRole::Ice, None),
            mass(9, 10, SolidMaterialRole::Limestone, None),
            mass(10, 11, SolidMaterialRole::Slate, None),
            mass(11, 12, SolidMaterialRole::Timber, None),
            mass(12, 13, SolidMaterialRole::Terracotta, None),
            mass(13, 14, SolidMaterialRole::Basalt, None),
            fill(15, 16, FillMaterialRole::Water),
            fill(17, 18, FillMaterialRole::Lava),
        ];
        plan.surfaces.insert(
            TilePos::new(coord, 13),
            surface(SurfaceAccess::NonStandable, None),
        );

        let palette = palette();
        let materialized = plan
            .materialize(&palette, &test_is_solid)
            .expect("every role should resolve");
        let expected = [
            palette.bedrock,
            palette.stone,
            palette.dirt,
            palette.grass,
            palette.gravel,
            palette.metal,
            palette.worked_stone,
            palette.snow,
            palette.ice,
            palette.limestone,
            palette.slate,
            palette.timber,
            palette.terracotta,
            palette.basalt,
        ];
        for (level, substance) in expected.into_iter().enumerate() {
            assert_eq!(
                materialized.map.get(TilePos::new(
                    coord,
                    Level::try_from(level).expect("the material index fits in Level"),
                )),
                substance
            );
        }
        assert!(materialized.map.get(TilePos::new(coord, 14)).is_air());
        assert_eq!(materialized.map.get(TilePos::new(coord, 15)), palette.water);
        assert!(materialized.map.get(TilePos::new(coord, 16)).is_air());
        assert_eq!(materialized.map.get(TilePos::new(coord, 17)), palette.lava);
    }

    #[test]
    fn materialization_rejects_air_or_wrong_solidity_role_mapping() {
        let coord = HexCoord::ORIGIN;
        let mut plan = VolumePlan::new(BTreeSet::from([coord]));
        plan.columns
            .get_mut(&coord)
            .expect("the origin is in the test mask")
            .elements = vec![mass(0, 1, SolidMaterialRole::Stone, None)];
        plan.surfaces.insert(
            TilePos::new(coord, 0),
            surface(SurfaceAccess::Ordinary, None),
        );

        let mut missing = palette();
        missing.stone = SubstanceId::AIR;
        assert!(plan
            .materialize(&missing, &test_is_solid)
            .expect_err("solid roles cannot resolve to air")
            .to_string()
            .contains("non-air"));

        let mut solid_water = palette();
        plan.columns
            .get_mut(&coord)
            .expect("the origin is in the test mask")
            .elements = vec![
            mass(0, 1, SolidMaterialRole::Bedrock, None),
            fill(1, 2, FillMaterialRole::Water),
        ];
        plan.surfaces
            .get_mut(&TilePos::new(coord, 0))
            .expect("the bedrock surface metadata was inserted")
            .access = SurfaceAccess::NonStandable;
        solid_water.water = solid_water.stone;
        assert!(plan
            .materialize(&solid_water, &test_is_solid)
            .expect_err("fill roles cannot resolve to solid substances")
            .to_string()
            .contains("non-solid"));
    }

    #[test]
    fn semantic_and_materialized_headroom_match_every_surface() {
        let coord = HexCoord::ORIGIN;
        let mut plan = VolumePlan::new(BTreeSet::from([coord]));
        plan.columns
            .get_mut(&coord)
            .expect("the origin is in the test mask")
            .elements = vec![
            mass(0, 5, SolidMaterialRole::Stone, None),
            mass(8, 10, SolidMaterialRole::Metal, None),
        ];
        for (position, access) in [
            (TilePos::new(coord, 4), SurfaceAccess::Ordinary),
            (TilePos::new(coord, 9), SurfaceAccess::Ordinary),
        ] {
            plan.surfaces.insert(position, surface(access, None));
        }

        let materialized = plan
            .materialize(&palette(), &test_is_solid)
            .expect("valid volume should materialize");
        for position in plan.surfaces.keys().copied() {
            let semantic = plan
                .surface_headroom(position)
                .expect("every test position is a declared surface");
            let runtime = materialized
                .map
                .column(position.coord)
                .expect("the materialized map retains every masked column")
                .headroom_above(position.level.saturating_add(1));
            assert_eq!(semantic, runtime, "headroom drifted at {position:?}");
        }
    }
}
