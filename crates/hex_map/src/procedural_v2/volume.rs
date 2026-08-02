//! Recipe-independent semantic volume model for procedural generator V2.
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use bevy::platform::collections::{HashMap, HashSet};
use hex_core::{
    Headroom, HexCoord, InteriorRegionId, InteriorRegions, Level, MapViewHint,
    SpecialMovementRegion, SubstanceId, TilePos, TraversalEndpoint, TraversalProfile, MAX_HEADROOM,
};

use crate::settings::MAX_PROCEDURAL_LEVEL;
use crate::terrain::TerrainPalette;
use crate::voxel::{Column, VoxelMap};

use super::V2GenerationError;

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
}

/// Substance role for structural material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SolidMaterialRole {
    Bedrock,
    Stone,
    Dirt,
    Grass,
    Gravel,
    Metal,
    Snow,
    Ice,
    Basalt,
}

/// Substance role for visible material which cannot support ordinary footing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FillMaterialRole {
    Water,
    Lava,
}

/// One solid material interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SolidMass {
    pub(crate) levels: LevelInterval,
    pub(crate) material: SolidMaterialRole,
    pub(crate) cutaway_for: Option<InteriorRegionId>,
}

/// One non-solid material interval. Air remains implicit.
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

/// How live traversal should classify an exact upward solid boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SurfaceAccess {
    Ordinary,
    SpecialMovement(SpecialMovementRegion),
    NonStandable,
}

/// Semantic facts attached to one exact surface voxel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SurfaceMetadata {
    pub(crate) access: SurfaceAccess,
    pub(crate) interior: Option<InteriorRegionId>,
}

/// Recipe-authored description of one interior network.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct InteriorVolume {
    pub(crate) floors: BTreeSet<TilePos>,
    pub(crate) entrances: BTreeSet<TilePos>,
    pub(crate) clear_air: BTreeMap<HexCoord, LevelInterval>,
}

/// Complete recipe-independent V2 geometry before voxel materialization.
#[derive(Debug, Clone)]
pub(crate) struct TerrainVolumePlan {
    pub(crate) grid_radius: u32,
    pub(crate) columns: BTreeMap<HexCoord, VolumeColumn>,
    pub(crate) surfaces: BTreeMap<TilePos, SurfaceMetadata>,
    pub(crate) anchors: BTreeMap<String, TilePos>,
    pub(crate) interiors: BTreeMap<InteriorRegionId, InteriorVolume>,
    pub(crate) view_hint: MapViewHint,
}

impl TerrainVolumePlan {
    /// Checks storage, surface, traversal, anchor, and interior invariants shared by
    /// every V2 recipe.
    pub(crate) fn validate(&self) -> Result<(), V2GenerationError> {
        if !(12..=40).contains(&self.grid_radius) {
            return Err(V2GenerationError::InvalidVolume(vec![format!(
                "volume grid_radius {} is outside the supported range 12..=40",
                self.grid_radius
            )]));
        }
        let mut issues = Vec::new();
        let radius = usize::try_from(self.grid_radius).unwrap_or(usize::MAX);
        let expected_columns = 1_usize.saturating_add(
            3_usize.saturating_mul(radius.saturating_mul(radius.saturating_add(1))),
        );
        let outside_footprint = self
            .columns
            .keys()
            .any(|coord| HexCoord::ORIGIN.distance(*coord) > self.grid_radius);
        if self.columns.len() != expected_columns || outside_footprint {
            issues.push(format!(
                "volume footprint has {} in-bounds columns; expected {expected_columns}",
                self.columns.len()
            ));
        }
        if !self.view_hint.is_valid() {
            issues.push("generated map view hint is invalid".to_owned());
        }

        let mut expected_surfaces = BTreeSet::new();
        let mut headroom = BTreeMap::new();
        let mut solid_intervals = 0_usize;
        for (coord, column) in &self.columns {
            validate_column(*coord, column, &mut issues);
            for (index, element) in column.elements.iter().copied().enumerate() {
                let VolumeElement::Solid(mass) = element else {
                    continue;
                };
                solid_intervals = solid_intervals.saturating_add(1);
                let covered_by_solid = column.elements.get(index + 1).is_some_and(|next| {
                    matches!(next, VolumeElement::Solid(_))
                        && next.levels().bottom == mass.levels.top
                });
                if covered_by_solid {
                    continue;
                }
                let position = TilePos::new(*coord, mass.levels.top.saturating_sub(1));
                expected_surfaces.insert(position);
                headroom.insert(
                    position,
                    Headroom(clear_levels_above(column, mass.levels.top)),
                );
            }
        }

        if self.surfaces.len() != expected_surfaces.len()
            || !self
                .surfaces
                .keys()
                .all(|position| expected_surfaces.contains(position))
        {
            let missing = expected_surfaces
                .iter()
                .find(|position| !self.surfaces.contains_key(position));
            let extra = self
                .surfaces
                .keys()
                .find(|position| !expected_surfaces.contains(position));
            issues.push(format!(
                "surface metadata does not match exposed solid boundaries \
                 (missing {missing:?}, extra {extra:?})"
            ));
        }

        let mut traversable = HashMap::new();
        let mut ordinary = BTreeSet::new();
        let mut special = BTreeMap::<SpecialMovementRegion, BTreeSet<TilePos>>::new();
        for (position, metadata) in &self.surfaces {
            let room = headroom.get(position).copied().unwrap_or_default();
            let endpoint = TraversalEndpoint::new(*position, true, room);
            let standable = TraversalProfile::WALKER.admits_surface(true, room);
            match metadata.access {
                SurfaceAccess::Ordinary => {
                    if !standable {
                        issues.push(format!("ordinary surface {position:?} is not standable"));
                    }
                    ordinary.insert(*position);
                    traversable.insert(*position, endpoint);
                }
                SurfaceAccess::SpecialMovement(region) => {
                    if !standable {
                        issues.push(format!(
                            "special-movement surface {position:?} is not standable"
                        ));
                    }
                    special.entry(region).or_default().insert(*position);
                    traversable.insert(*position, endpoint);
                }
                SurfaceAccess::NonStandable if standable => issues.push(format!(
                    "surface {position:?} is labelled non-standable but admits the walker"
                )),
                SurfaceAccess::NonStandable => {}
            }
        }

        if solid_intervals == 0 {
            issues.push("volume contains no solid mass".to_owned());
        }
        if ordinary.is_empty() {
            issues.push("volume contains no ordinary walker surface".to_owned());
        }
        if self.anchors.is_empty() {
            issues.push("volume publishes no required actor anchors".to_owned());
        }
        validate_traversal_components(&ordinary, &special, &traversable, &mut issues);
        for (name, position) in &self.anchors {
            if !ordinary.contains(position) {
                issues.push(format!(
                    "anchor {name:?} does not name an ordinary standable surface"
                ));
            }
        }
        validate_interiors(self, &mut issues);

        if issues.is_empty() {
            Ok(())
        } else {
            Err(V2GenerationError::InvalidVolume(issues))
        }
    }
}

fn validate_column(coord: HexCoord, column: &VolumeColumn, issues: &mut Vec<String>) {
    let mut previous: Option<VolumeElement> = None;
    for element in column.elements.iter().copied() {
        let levels = element.levels();
        if levels.bottom < 0
            || levels.bottom >= levels.top
            || levels.top > MAX_PROCEDURAL_LEVEL.saturating_add(1)
        {
            issues.push(format!("column {coord:?} has invalid interval {levels:?}"));
        }
        if let Some(before) = previous {
            let before_levels = before.levels();
            if levels.bottom < before_levels.top {
                issues.push(format!(
                    "column {coord:?} has overlapping intervals {before_levels:?} and {levels:?}"
                ));
            }
            if levels.bottom == before_levels.top && same_material(before, element) {
                let reason = if same_render_semantics(before, element) {
                    "adjacent identical noncanonical elements"
                } else {
                    "an equal-material boundary whose metadata would be lost when rendered"
                };
                issues.push(format!("column {coord:?} has {reason}"));
            }
        }
        previous = Some(element);
    }
}

fn same_material(left: VolumeElement, right: VolumeElement) -> bool {
    match (left, right) {
        (VolumeElement::Solid(left), VolumeElement::Solid(right)) => {
            left.material == right.material
        }
        (VolumeElement::Fill(left), VolumeElement::Fill(right)) => left.material == right.material,
        _ => false,
    }
}

fn same_render_semantics(left: VolumeElement, right: VolumeElement) -> bool {
    match (left, right) {
        (VolumeElement::Solid(left), VolumeElement::Solid(right)) => {
            left.material == right.material && left.cutaway_for == right.cutaway_for
        }
        (VolumeElement::Fill(left), VolumeElement::Fill(right)) => left.material == right.material,
        _ => false,
    }
}

fn clear_levels_above(column: &VolumeColumn, from: Level) -> Level {
    let Some(obstruction) = column
        .elements
        .iter()
        .map(|element| element.levels())
        .find(|levels| levels.top > from)
    else {
        return MAX_HEADROOM;
    };
    if obstruction.bottom <= from {
        0
    } else {
        obstruction.bottom.saturating_sub(from).min(MAX_HEADROOM)
    }
}

fn validate_traversal_components(
    ordinary: &BTreeSet<TilePos>,
    special: &BTreeMap<SpecialMovementRegion, BTreeSet<TilePos>>,
    endpoints: &HashMap<TilePos, TraversalEndpoint>,
    issues: &mut Vec<String>,
) {
    if let Some(start) = ordinary.first().copied() {
        let reached = reachable(start, ordinary, endpoints);
        if reached.len() != ordinary.len() {
            issues.push("ordinary V2 surfaces are not one walker-connected component".to_owned());
        }
    }

    for (region, surfaces) in special {
        let Some(start) = surfaces.first().copied() else {
            issues.push(format!("special-movement region {region:?} is empty"));
            continue;
        };
        if reachable(start, surfaces, endpoints).len() != surfaces.len() {
            issues.push(format!(
                "special-movement region {region:?} is not internally walker-connected"
            ));
        }
        let ordinary_by_coord = positions_by_coord(ordinary);
        if surfaces.iter().any(|surface| {
            surface.neighbours().into_iter().any(|neighbor| {
                ordinary_by_coord
                    .get(&neighbor.coord)
                    .is_some_and(|at_coord| {
                        at_coord.iter().any(|ordinary_surface| {
                            transition(*surface, *ordinary_surface, endpoints)
                                || transition(*ordinary_surface, *surface, endpoints)
                        })
                    })
            })
        }) {
            issues.push(format!(
                "special-movement region {region:?} touches the ordinary walker network"
            ));
        }
    }

    let special_by_surface: BTreeMap<TilePos, SpecialMovementRegion> = special
        .iter()
        .flat_map(|(region, surfaces)| surfaces.iter().copied().map(|surface| (surface, *region)))
        .collect();
    let special_surfaces: BTreeSet<TilePos> = special_by_surface.keys().copied().collect();
    let special_by_coord = positions_by_coord(&special_surfaces);
    let mut joined_regions = BTreeSet::new();
    for (surface, region) in &special_by_surface {
        for neighbor in surface.neighbours() {
            let Some(candidates) = special_by_coord.get(&neighbor.coord) else {
                continue;
            };
            for candidate in candidates {
                let Some(other_region) = special_by_surface.get(candidate) else {
                    continue;
                };
                if region == other_region
                    || !(transition(*surface, *candidate, endpoints)
                        || transition(*candidate, *surface, endpoints))
                {
                    continue;
                }
                joined_regions.insert(if region < other_region {
                    (*region, *other_region)
                } else {
                    (*other_region, *region)
                });
            }
        }
    }
    for (first, second) in joined_regions {
        issues.push(format!(
            "walker-connected surfaces use different special-movement regions \
             {first:?} and {second:?}"
        ));
    }
}

fn reachable(
    start: TilePos,
    allowed: &BTreeSet<TilePos>,
    endpoints: &HashMap<TilePos, TraversalEndpoint>,
) -> HashSet<TilePos> {
    let by_coord = positions_by_coord(allowed);
    let mut reached = HashSet::from([start]);
    let mut frontier = VecDeque::from([start]);
    while let Some(from) = frontier.pop_front() {
        for neighbor in from.neighbours() {
            let Some(candidates) = by_coord.get(&neighbor.coord) else {
                continue;
            };
            for to in candidates {
                if !reached.contains(to) && transition(from, *to, endpoints) {
                    reached.insert(*to);
                    frontier.push_back(*to);
                }
            }
        }
    }
    reached
}

fn positions_by_coord(surfaces: &BTreeSet<TilePos>) -> HashMap<HexCoord, Vec<TilePos>> {
    let mut by_coord = HashMap::<HexCoord, Vec<TilePos>>::new();
    for surface in surfaces {
        by_coord.entry(surface.coord).or_default().push(*surface);
    }
    by_coord
}

fn transition(from: TilePos, to: TilePos, endpoints: &HashMap<TilePos, TraversalEndpoint>) -> bool {
    let (Some(from), Some(to)) = (endpoints.get(&from), endpoints.get(&to)) else {
        return false;
    };
    TraversalProfile::WALKER.admits_transition(*from, *to)
}

fn validate_interiors(plan: &TerrainVolumePlan, issues: &mut Vec<String>) {
    for (surface, metadata) in &plan.surfaces {
        if let Some(region) = metadata.interior {
            match plan.interiors.get(&region) {
                None => issues.push(format!(
                    "surface {surface:?} references missing interior {region:?}"
                )),
                Some(interior)
                    if !interior.floors.contains(surface)
                        && !interior.entrances.contains(surface) =>
                {
                    issues.push(format!(
                        "surface {surface:?} is not listed by its interior {region:?}"
                    ));
                }
                Some(_) => {}
            }
        }
    }

    for (region, interior) in &plan.interiors {
        for floor in interior.floors.union(&interior.entrances) {
            match plan.surfaces.get(floor) {
                Some(SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: Some(actual),
                }) if actual == region => {}
                _ => issues.push(format!(
                    "interior region {region:?} has a non-ordinary or untagged floor {floor:?}"
                )),
            }
        }
        for (coord, clear) in &interior.clear_air {
            if clear.bottom < 0
                || clear.bottom >= clear.top
                || clear.top > MAX_PROCEDURAL_LEVEL.saturating_add(1)
            {
                issues.push(format!(
                    "interior region {region:?} has invalid clear-air interval {clear:?}"
                ));
                continue;
            }
            let Some(column) = plan.columns.get(coord) else {
                issues.push(format!(
                    "interior region {region:?} names missing column {coord:?}"
                ));
                continue;
            };
            if column.elements.iter().any(|element| {
                let occupied = element.levels();
                occupied.bottom < clear.top && clear.bottom < occupied.top
            }) {
                issues.push(format!(
                    "interior region {region:?} clear-air interval {clear:?} is occupied"
                ));
            }
        }
    }

    for (coord, column) in &plan.columns {
        for element in &column.elements {
            let VolumeElement::Solid(mass) = element else {
                continue;
            };
            if let Some(region) = mass.cutaway_for {
                if !plan.interiors.contains_key(&region) {
                    issues.push(format!(
                        "cutaway mass at {coord:?} references missing interior {region:?}"
                    ));
                }
            }
        }
    }
}

/// Materialized V2 terrain and the exact interior metadata retained alongside it.
#[derive(Debug)]
pub(crate) struct VoxelizedTerrain {
    pub(crate) map: VoxelMap,
    pub(crate) interiors: InteriorRegions,
}

/// Resolves semantic materials only after a plan has passed validation.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "production materialization enters through the validated-selection boundary"
    )
)]
pub(crate) fn voxelize(
    plan: &TerrainVolumePlan,
    palette: &TerrainPalette,
    is_solid: &dyn Fn(SubstanceId) -> bool,
) -> Result<VoxelizedTerrain, V2GenerationError> {
    plan.validate()?;
    voxelize_plan(plan, palette, is_solid)
}

/// Materializes a plan already admitted by recipe selection.
///
/// The validated-selection type state protects this contract in release builds.
/// The debug assertion also rechecks it locally without charging release generation
/// for a second full connectivity pass.
pub(crate) fn voxelize_prevalidated(
    plan: &TerrainVolumePlan,
    palette: &TerrainPalette,
    is_solid: &dyn Fn(SubstanceId) -> bool,
) -> Result<VoxelizedTerrain, V2GenerationError> {
    #[cfg(debug_assertions)]
    plan.validate()?;
    voxelize_plan(plan, palette, is_solid)
}

fn voxelize_plan(
    plan: &TerrainVolumePlan,
    palette: &TerrainPalette,
    is_solid: &dyn Fn(SubstanceId) -> bool,
) -> Result<VoxelizedTerrain, V2GenerationError> {
    let mut map = VoxelMap::new();
    for (coord, planned) in &plan.columns {
        let mut column = Column::new();
        for element in &planned.elements {
            let (levels, substance, expected_solid) = match *element {
                VolumeElement::Solid(mass) => {
                    (mass.levels, solid_substance(mass.material, palette), true)
                }
                VolumeElement::Fill(fill) => {
                    (fill.levels, fill_substance(fill.material, palette), false)
                }
            };
            if substance.is_air() || is_solid(substance) != expected_solid {
                return Err(V2GenerationError::MaterialContract(format!(
                    "semantic volume material at {coord:?} resolved to incompatible substance \
                     {substance:?}"
                )));
            }
            for level in levels.bottom..levels.top {
                column.set(level, substance);
            }
        }
        map.insert_column(*coord, column);
    }
    let mut interiors = InteriorRegions::new();
    for (region, interior) in &plan.interiors {
        for surface in interior.floors.union(&interior.entrances) {
            let _previous = interiors.insert_surface(*surface, *region);
        }
    }
    for (coord, column) in &plan.columns {
        for element in &column.elements {
            let VolumeElement::Solid(mass) = element else {
                continue;
            };
            if let Some(region) = mass.cutaway_for {
                for level in mass.levels.bottom..mass.levels.top {
                    let _previous =
                        interiors.insert_roof_voxel(TilePos::new(*coord, level), region);
                }
            }
        }
    }
    Ok(VoxelizedTerrain { map, interiors })
}

const fn solid_substance(role: SolidMaterialRole, palette: &TerrainPalette) -> SubstanceId {
    match role {
        SolidMaterialRole::Bedrock => palette.bedrock,
        SolidMaterialRole::Stone => palette.stone,
        SolidMaterialRole::Dirt => palette.dirt,
        SolidMaterialRole::Grass => palette.grass,
        SolidMaterialRole::Gravel => palette.gravel,
        SolidMaterialRole::Metal => palette.metal,
        SolidMaterialRole::Snow => palette.snow,
        SolidMaterialRole::Ice => palette.ice,
        SolidMaterialRole::Basalt => palette.basalt,
    }
}

const fn fill_substance(role: FillMaterialRole, palette: &TerrainPalette) -> SubstanceId {
    match role {
        FillMaterialRole::Water => palette.water,
        FillMaterialRole::Lava => palette.lava,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_RADIUS: u32 = 12;

    fn empty_plan(radius: u32) -> TerrainVolumePlan {
        TerrainVolumePlan {
            grid_radius: radius,
            columns: HexCoord::ORIGIN
                .within_radius(radius)
                .into_iter()
                .map(|coord| (coord, VolumeColumn::default()))
                .collect(),
            surfaces: BTreeMap::new(),
            anchors: BTreeMap::new(),
            interiors: BTreeMap::new(),
            view_hint: MapViewHint::new((0.0, 10.0, 10.0), (0.0, 0.0, 0.0)),
        }
    }

    fn mass(bottom: Level, top: Level, material: SolidMaterialRole) -> VolumeElement {
        VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(bottom, top),
            material,
            cutaway_for: None,
        })
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

    fn stacked_volume_plan() -> TerrainVolumePlan {
        let mut plan = empty_plan(TEST_RADIUS);
        let coord = HexCoord::ORIGIN;
        let [filled_coord, ..] = coord.neighbors();
        let region = InteriorRegionId(3);
        let floor = TilePos::new(coord, 4);
        let roof = TilePos::new(coord, 9);

        plan.columns.insert(
            coord,
            VolumeColumn {
                elements: vec![
                    mass(0, 1, SolidMaterialRole::Bedrock),
                    mass(1, 4, SolidMaterialRole::Stone),
                    mass(4, 5, SolidMaterialRole::Grass),
                    VolumeElement::Solid(SolidMass {
                        levels: LevelInterval::new(8, 10),
                        material: SolidMaterialRole::Stone,
                        cutaway_for: Some(region),
                    }),
                ],
            },
        );
        plan.columns.insert(
            filled_coord,
            VolumeColumn {
                elements: vec![
                    mass(0, 1, SolidMaterialRole::Bedrock),
                    mass(1, 3, SolidMaterialRole::Gravel),
                    VolumeElement::Fill(NonSolidFill {
                        levels: LevelInterval::new(3, 5),
                        material: FillMaterialRole::Water,
                    }),
                ],
            },
        );
        plan.surfaces.insert(
            floor,
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: Some(region),
            },
        );
        plan.surfaces.insert(
            roof,
            SurfaceMetadata {
                access: SurfaceAccess::SpecialMovement(SpecialMovementRegion(7)),
                interior: None,
            },
        );
        plan.surfaces.insert(
            TilePos::new(filled_coord, 2),
            SurfaceMetadata {
                access: SurfaceAccess::NonStandable,
                interior: None,
            },
        );
        plan.anchors.insert("party_start".to_owned(), floor);
        plan.interiors.insert(
            region,
            InteriorVolume {
                floors: BTreeSet::from([floor]),
                entrances: BTreeSet::new(),
                clear_air: BTreeMap::from([(coord, LevelInterval::new(5, 8))]),
            },
        );
        plan
    }

    #[test]
    fn volume_rejects_overlap_and_missing_surface_metadata() {
        let mut plan = empty_plan(TEST_RADIUS);
        plan.columns.insert(
            HexCoord::ORIGIN,
            VolumeColumn {
                elements: vec![
                    mass(0, 4, SolidMaterialRole::Stone),
                    mass(3, 6, SolidMaterialRole::Dirt),
                ],
            },
        );

        let error = plan.validate().expect_err("overlap must fail");
        assert!(error.to_string().contains("overlapping intervals"));
        assert!(error.to_string().contains("surface metadata"));
    }

    #[test]
    fn footprint_rejects_an_equal_sized_out_of_bounds_substitution() {
        let mut plan = empty_plan(TEST_RADIUS);
        let removed = plan
            .columns
            .remove(&HexCoord::ORIGIN)
            .expect("the complete footprint should contain its origin");
        let outside = HexCoord::from_axial(
            i32::try_from(TEST_RADIUS).expect("the test radius should fit") + 1,
            0,
        );
        assert!(plan.columns.insert(outside, removed).is_none());

        let error = plan
            .validate()
            .expect_err("an outside column cannot replace a missing in-bounds column");
        assert!(error.to_string().contains("volume footprint"));
    }

    #[test]
    fn stacked_floor_and_roof_require_exact_metadata() {
        let mut plan = empty_plan(TEST_RADIUS);
        let coord = HexCoord::ORIGIN;
        plan.columns.insert(
            coord,
            VolumeColumn {
                elements: vec![
                    mass(0, 5, SolidMaterialRole::Stone),
                    mass(8, 12, SolidMaterialRole::Stone),
                ],
            },
        );
        plan.surfaces.insert(
            TilePos::new(coord, 4),
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: Some(InteriorRegionId(0)),
            },
        );
        plan.surfaces.insert(
            TilePos::new(coord, 11),
            SurfaceMetadata {
                access: SurfaceAccess::SpecialMovement(SpecialMovementRegion(0)),
                interior: None,
            },
        );
        plan.anchors
            .insert("party_start".to_owned(), TilePos::new(coord, 4));
        plan.interiors.insert(
            InteriorRegionId(0),
            InteriorVolume {
                floors: BTreeSet::from([TilePos::new(coord, 4)]),
                entrances: BTreeSet::new(),
                clear_air: BTreeMap::from([(coord, LevelInterval::new(5, 8))]),
            },
        );

        assert!(plan.validate().is_ok());
    }

    #[test]
    fn fill_directly_above_a_bed_is_nonstandable() {
        let mut plan = empty_plan(TEST_RADIUS);
        let coord = HexCoord::ORIGIN;
        let support = coord.neighbors()[0];
        plan.columns.insert(
            coord,
            VolumeColumn {
                elements: vec![
                    mass(0, 5, SolidMaterialRole::Gravel),
                    VolumeElement::Fill(NonSolidFill {
                        levels: LevelInterval::new(5, 7),
                        material: FillMaterialRole::Water,
                    }),
                ],
            },
        );
        plan.surfaces.insert(
            TilePos::new(coord, 4),
            SurfaceMetadata {
                access: SurfaceAccess::NonStandable,
                interior: None,
            },
        );
        plan.columns.insert(
            support,
            VolumeColumn {
                elements: vec![mass(0, 3, SolidMaterialRole::Stone)],
            },
        );
        let support_surface = TilePos::new(support, 2);
        plan.surfaces.insert(
            support_surface,
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: None,
            },
        );
        plan.anchors
            .insert("party_start".to_owned(), support_surface);

        assert!(plan.validate().is_ok());
    }

    #[test]
    fn shared_aperture_is_used_by_volume_connectivity() {
        let mut plan = empty_plan(TEST_RADIUS);
        let low = HexCoord::ORIGIN;
        let high = low.neighbors()[0];
        plan.columns.insert(
            low,
            VolumeColumn {
                elements: vec![
                    mass(0, 5, SolidMaterialRole::Stone),
                    mass(7, 9, SolidMaterialRole::Stone),
                ],
            },
        );
        plan.columns.insert(
            high,
            VolumeColumn {
                elements: vec![mass(0, 6, SolidMaterialRole::Stone)],
            },
        );
        for position in [TilePos::new(low, 4), TilePos::new(high, 5)] {
            plan.surfaces.insert(
                position,
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
            );
        }
        plan.surfaces.insert(
            TilePos::new(low, 8),
            SurfaceMetadata {
                access: SurfaceAccess::NonStandable,
                interior: None,
            },
        );
        plan.anchors
            .insert("party_start".to_owned(), TilePos::new(low, 4));

        let error = plan
            .validate()
            .expect_err("the low lintel disconnects the ramp");
        assert!(error.to_string().contains("not one walker-connected"));
    }

    #[test]
    fn walker_connected_special_surfaces_cannot_use_different_region_ids() {
        let mut plan = empty_plan(TEST_RADIUS);
        let ordinary_coord = HexCoord::from_axial(-5, 0);
        let first_coord = HexCoord::ORIGIN;
        let second_coord = first_coord.neighbors()[0];
        for coord in [ordinary_coord, first_coord, second_coord] {
            plan.columns.insert(
                coord,
                VolumeColumn {
                    elements: vec![mass(0, 5, SolidMaterialRole::Stone)],
                },
            );
        }

        let ordinary = TilePos::new(ordinary_coord, 4);
        plan.surfaces.insert(
            ordinary,
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: None,
            },
        );
        plan.anchors.insert("party_start".to_owned(), ordinary);
        for (coord, region) in [
            (first_coord, SpecialMovementRegion(2)),
            (second_coord, SpecialMovementRegion(9)),
        ] {
            plan.surfaces.insert(
                TilePos::new(coord, 4),
                SurfaceMetadata {
                    access: SurfaceAccess::SpecialMovement(region),
                    interior: None,
                },
            );
        }

        let error = plan
            .validate()
            .expect_err("one walker component cannot claim two special-region ids");
        assert!(error
            .to_string()
            .contains("different special-movement regions"));
    }

    #[test]
    fn empty_volume_and_unbounded_intervals_are_rejected() {
        assert!(empty_plan(0)
            .validate()
            .expect_err("unsupported radius must fail before allocating a footprint")
            .to_string()
            .contains("supported range"));

        let empty = empty_plan(TEST_RADIUS);
        let error = empty
            .validate()
            .expect_err("an all-air plan must never materialize");
        assert!(error.to_string().contains("no solid mass"));
        assert!(error.to_string().contains("no ordinary walker surface"));
        assert!(error.to_string().contains("no required actor anchors"));

        let mut unbounded = empty_plan(TEST_RADIUS);
        unbounded.columns.insert(
            HexCoord::ORIGIN,
            VolumeColumn {
                elements: vec![mass(
                    0,
                    MAX_PROCEDURAL_LEVEL.saturating_add(2),
                    SolidMaterialRole::Stone,
                )],
            },
        );
        assert!(unbounded
            .validate()
            .expect_err("oversized allocation must be rejected")
            .to_string()
            .contains("invalid interval"));
    }

    #[test]
    fn cutaway_boundaries_must_survive_render_run_merging() {
        let mut plan = empty_plan(TEST_RADIUS);
        let region = InteriorRegionId(0);
        plan.columns.insert(
            HexCoord::ORIGIN,
            VolumeColumn {
                elements: vec![
                    mass(0, 4, SolidMaterialRole::Stone),
                    VolumeElement::Solid(SolidMass {
                        levels: LevelInterval::new(4, 8),
                        material: SolidMaterialRole::Stone,
                        cutaway_for: Some(region),
                    }),
                ],
            },
        );

        assert!(plan
            .validate()
            .expect_err("merged render runs cannot preserve an internal cutaway boundary")
            .to_string()
            .contains("metadata would be lost"));
    }

    #[test]
    fn interior_clear_air_intervals_are_bounded_and_ordered() {
        let mut plan = empty_plan(TEST_RADIUS);
        plan.interiors.insert(
            InteriorRegionId(0),
            InteriorVolume {
                floors: BTreeSet::new(),
                entrances: BTreeSet::new(),
                clear_air: BTreeMap::from([(HexCoord::ORIGIN, LevelInterval::new(8, 7))]),
            },
        );

        assert!(plan
            .validate()
            .expect_err("reversed clear-air interval must fail")
            .to_string()
            .contains("invalid clear-air interval"));
    }

    #[test]
    fn voxelize_resolves_roles_and_preserves_stacked_air_gaps() {
        let plan = stacked_volume_plan();
        let palette = palette();
        let voxelized = voxelize(&plan, &palette, &test_is_solid)
            .expect("the valid semantic volume should materialize");
        let coord = HexCoord::ORIGIN;
        let [filled_coord, ..] = coord.neighbors();

        assert_eq!(voxelized.map.get(TilePos::new(coord, 0)), palette.bedrock);
        assert_eq!(voxelized.map.get(TilePos::new(coord, 3)), palette.stone);
        assert_eq!(voxelized.map.get(TilePos::new(coord, 4)), palette.grass);
        for level in 5..8 {
            assert!(
                voxelized.map.get(TilePos::new(coord, level)).is_air(),
                "stacked masses must leave level {level} as air"
            );
        }
        assert_eq!(voxelized.map.get(TilePos::new(coord, 8)), palette.stone);
        assert_eq!(
            voxelized.map.get(TilePos::new(filled_coord, 3)),
            palette.water
        );
        assert_eq!(
            voxelized.map.get(TilePos::new(filled_coord, 4)),
            palette.water
        );
    }

    #[test]
    fn voxelize_rejects_material_resolution_contract_mismatches() {
        let plan = stacked_volume_plan();

        let mut air_solid = palette();
        air_solid.stone = SubstanceId::AIR;
        let solid_error = voxelize(&plan, &air_solid, &test_is_solid)
            .expect_err("a solid semantic role must not resolve to air");
        assert!(solid_error.to_string().contains("incompatible substance"));

        let mut solid_fill = palette();
        solid_fill.water = solid_fill.stone;
        let fill_error = voxelize(&plan, &solid_fill, &test_is_solid)
            .expect_err("a fill semantic role must resolve to a non-solid substance");
        assert!(fill_error.to_string().contains("incompatible substance"));
    }

    #[test]
    fn voxelize_retains_exact_interior_and_cutaway_metadata() {
        let plan = stacked_volume_plan();
        let voxelized = voxelize(&plan, &palette(), &test_is_solid)
            .expect("the valid semantic volume should materialize");
        let region = InteriorRegionId(3);
        let floor = TilePos::new(HexCoord::ORIGIN, 4);
        let roof_bottom = TilePos::new(HexCoord::ORIGIN, 8);
        let roof_top = TilePos::new(HexCoord::ORIGIN, 9);

        assert_eq!(voxelized.interiors.get(floor), Some(region));
        assert_eq!(voxelized.interiors.roof_region(roof_bottom), Some(region));
        assert_eq!(voxelized.interiors.roof_region(roof_top), Some(region));
        assert_eq!(voxelized.interiors.get(roof_top), None);
        assert_eq!(voxelized.interiors.roof_region(floor), None);
        assert_eq!(voxelized.interiors.surfaces().count(), 1);
        assert_eq!(voxelized.interiors.roof_voxels().count(), 2);
    }

    #[test]
    fn voxelized_columns_match_semantic_headroom_for_every_surface() {
        let plan = stacked_volume_plan();
        let voxelized = voxelize(&plan, &palette(), &test_is_solid)
            .expect("the valid semantic volume should materialize");

        for position in plan.surfaces.keys().copied() {
            let planned_column = plan
                .columns
                .get(&position.coord)
                .expect("declared surfaces must have a semantic column");
            let materialized_column = voxelized
                .map
                .column(position.coord)
                .expect("declared surfaces must have a materialized column");
            let from = position.level.saturating_add(1);
            let semantic = Headroom(clear_levels_above(planned_column, from));
            let materialized = materialized_column.headroom_above(from);

            assert_eq!(
                materialized, semantic,
                "headroom changed while materializing {position:?}"
            );
        }
    }
}
