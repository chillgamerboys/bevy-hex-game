//! Complete semantic world storage for procedural generator V3.
//!
//! Recipe planners work against this private representation. Runtime consumers see
//! only the exact projections produced after the whole plan has passed common and
//! recipe-specific validation.

use std::collections::{BTreeMap, BTreeSet};

use hex_core::{BiomeRegionId, IlluminationLevel, InteriorRegionId, MapViewHint, TilePos};

use super::layout::{ResolvedEdgeContract, ResolvedLayoutPlan, ResolvedLiquidPort};
pub(crate) use super::liquid::LiquidPlan;
use super::liquid::{LiquidBodyId, LiquidIssue};
use super::volume::{NonSolidFill, SurfaceAccess, VolumeElement, VolumeIssue, VolumePlan};

/// Stable map-local identity of a planned surface feature.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct FeatureId(pub(crate) u32);

/// Feature behavior whose concrete assets remain private to `hex_map`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FeatureKind {
    Tree,
    TallGrass,
}

/// One exact surface feature placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlannedFeature {
    pub(crate) root: TilePos,
    pub(crate) kind: FeatureKind,
}

/// Surface features keyed independently from their position.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct FeaturePlan {
    pub(crate) by_id: BTreeMap<FeatureId, PlannedFeature>,
}

/// Stable map-local identity of one authored generated structure.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct StructureId(pub(crate) u32);

/// Semantic structure family. Voxel material remains in [`VolumePlan`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum StructureKind {
    Bridge,
    Wall,
    Stair,
    Tower,
    Gate,
    Keep,
}

/// Exact solid voxels participating in one generated structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlannedStructure {
    pub(crate) kind: StructureKind,
    pub(crate) voxels: BTreeSet<TilePos>,
}

/// Generated structures keyed independently from volume and rendering.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct StructurePlan {
    pub(crate) by_id: BTreeMap<StructureId, PlannedStructure>,
}

/// Stable map-local identity of a generated gameplay light.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct LightId(pub(crate) u32);

/// Exact logical source used later to spawn a public `GameplayLight`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlannedGameplayLight {
    pub(crate) origin: TilePos,
    pub(crate) level: IlluminationLevel,
    pub(crate) radius: u32,
}

/// One generated interior network.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct PlannedInterior {
    pub(crate) floors: BTreeSet<TilePos>,
    pub(crate) entrances: BTreeSet<TilePos>,
    pub(crate) roof_voxels: BTreeSet<TilePos>,
}

/// Exact interior-domain metadata, cross-checked against the semantic volume.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct InteriorPlan {
    pub(crate) by_id: BTreeMap<InteriorRegionId, PlannedInterior>,
}

/// The complete private semantic output of one V3 world candidate.
#[derive(Debug, Clone)]
pub(crate) struct GeneratedWorldPlan {
    pub(crate) layout: ResolvedLayoutPlan,
    pub(crate) volume: VolumePlan,
    pub(crate) liquids: LiquidPlan,
    pub(crate) features: FeaturePlan,
    pub(crate) structures: StructurePlan,
    pub(crate) blockers: BTreeSet<TilePos>,
    pub(crate) lights: BTreeMap<LightId, PlannedGameplayLight>,
    pub(crate) biome_regions: BTreeMap<TilePos, BiomeRegionId>,
    pub(crate) interiors: InteriorPlan,
    pub(crate) anchors: BTreeMap<String, TilePos>,
    pub(crate) view_hint: MapViewHint,
}

impl GeneratedWorldPlan {
    /// Checks recipe-independent relationships across every semantic layer.
    #[must_use]
    pub(crate) fn validate(&self) -> Vec<WorldValidationIssue> {
        let mut issues = Vec::new();

        if let Err(error) = self.layout.validate() {
            issues.extend(
                error.issues().iter().map(|issue| {
                    WorldValidationIssue::new(WorldIssueCode::Layout, issue.to_string())
                }),
            );
        }
        if self.layout.footprint != self.volume.mask {
            issues.push(WorldValidationIssue::new(
                WorldIssueCode::Layout,
                "resolved layout footprint does not match the semantic volume mask",
            ));
        }
        if let Err(error) = self.volume.validate() {
            append_volume_issues(&mut issues, &error);
        }

        append_liquid_issues(&mut issues, &self.liquids.validate(&self.volume));
        self.validate_liquid_seams(&mut issues);
        self.validate_features_and_blockers(&mut issues);
        self.validate_structures(&mut issues);
        self.validate_biomes(&mut issues);
        self.validate_interiors(&mut issues);
        self.validate_lights(&mut issues);
        self.validate_anchors(&mut issues);
        if !self.view_hint.is_valid() {
            issues.push(WorldValidationIssue::new(
                WorldIssueCode::View,
                "generated map view hint is non-finite or degenerate",
            ));
        }

        issues
    }

    fn validate_liquid_seams(&self, issues: &mut Vec<WorldValidationIssue>) {
        let fill_runs = self.volume.fill_runs_by_top();
        let mut runs_by_coord = BTreeMap::<_, Vec<_>>::new();
        for (position, fill) in &fill_runs {
            runs_by_coord
                .entry(position.coord)
                .or_default()
                .push((*position, *fill));
        }
        validate_uncontracted_liquid_crossings(&self.layout, &self.liquids, &runs_by_coord, issues);

        for (edge_id, edge) in &self.layout.shared_edges {
            let mut crossings = Vec::new();
            for (body_id, body) in &self.liquids.bodies {
                for (source, node) in &body.nodes {
                    let Some(target) = node.downstream else {
                        continue;
                    };
                    if let Some((lane, forward)) =
                        oriented_seam_lane(edge, source.coord, target.coord)
                    {
                        crossings.push(SeamCrossing {
                            body: *body_id,
                            source: *source,
                            target,
                            lane,
                            forward,
                        });
                    }
                }
            }

            match &edge.liquid {
                ResolvedLiquidPort::Dry => {
                    for crossing in &crossings {
                        issues.push(WorldValidationIssue::new(
                            WorldIssueCode::Liquid,
                            format!(
                                "dry seam {edge_id:?} has directed liquid crossing {:?} -> {:?}",
                                crossing.source, crossing.target
                            ),
                        ));
                    }
                    validate_dry_seam_contacts(*edge_id, edge, &runs_by_coord, issues);
                }
                ResolvedLiquidPort::Directed { source, sink, port } => {
                    let source_is_first = *source == edge.first.0 && *sink == edge.second.0;
                    let source_is_second = *source == edge.second.0 && *sink == edge.first.0;
                    if !source_is_first && !source_is_second {
                        continue;
                    }
                    let mut realized = BTreeMap::new();
                    for crossing in crossings {
                        if !port.lanes.contains(&crossing.lane)
                            || crossing.forward != source_is_first
                        {
                            issues.push(WorldValidationIssue::new(
                                WorldIssueCode::Liquid,
                                format!(
                                    "liquid body {:?} crosses seam {edge_id:?} outside its exact directed port: {:?} -> {:?}",
                                    crossing.body, crossing.source, crossing.target
                                ),
                            ));
                            continue;
                        }
                        if !level_in_edge_band(crossing.source.level, edge)
                            || !level_in_edge_band(crossing.target.level, edge)
                        {
                            issues.push(WorldValidationIssue::new(
                                WorldIssueCode::Liquid,
                                format!(
                                    "liquid seam {edge_id:?} crossing {:?} -> {:?} leaves elevation band {}..={}",
                                    crossing.source,
                                    crossing.target,
                                    edge.elevation.min,
                                    edge.elevation.max
                                ),
                            ));
                            continue;
                        }
                        let normalized = if crossing.forward {
                            (crossing.source, crossing.target)
                        } else {
                            (crossing.target, crossing.source)
                        };
                        if realized.insert(crossing.lane, normalized).is_some() {
                            issues.push(WorldValidationIssue::new(
                                WorldIssueCode::Liquid,
                                format!(
                                    "liquid seam {edge_id:?} realizes lane {:?} more than once",
                                    crossing.lane
                                ),
                            ));
                        }
                    }
                    for lane in &port.lanes {
                        if !realized.contains_key(lane) {
                            issues.push(WorldValidationIssue::new(
                                WorldIssueCode::Liquid,
                                format!(
                                    "liquid seam {edge_id:?} does not realize exact port lane {lane:?}"
                                ),
                            ));
                        }
                    }
                    validate_directed_seam_contacts(
                        *edge_id,
                        edge,
                        &realized,
                        &runs_by_coord,
                        issues,
                    );
                }
            }
        }
    }

    fn validate_features_and_blockers(&self, issues: &mut Vec<WorldValidationIssue>) {
        let mut expected_blockers = BTreeSet::new();
        for (id, feature) in &self.features.by_id {
            if !self.volume.surfaces.contains_key(&feature.root) {
                issues.push(WorldValidationIssue::new(
                    WorldIssueCode::Feature,
                    format!("feature {id:?} is not rooted on an exact generated surface"),
                ));
            }
            if feature.kind == FeatureKind::Tree {
                expected_blockers.insert(feature.root);
            }
        }
        if self.blockers != expected_blockers {
            issues.push(WorldValidationIssue::new(
                WorldIssueCode::Blocker,
                "traversal blockers do not exactly match blocking feature roots",
            ));
        }
        for position in &self.blockers {
            if !matches!(
                self.volume
                    .surfaces
                    .get(position)
                    .map(|metadata| metadata.access),
                Some(SurfaceAccess::Ordinary)
            ) {
                issues.push(WorldValidationIssue::new(
                    WorldIssueCode::Blocker,
                    format!("blocker {position:?} does not name ordinary walker footing"),
                ));
            }
        }
    }

    fn validate_structures(&self, issues: &mut Vec<WorldValidationIssue>) {
        for (id, structure) in &self.structures.by_id {
            if structure.voxels.is_empty() {
                issues.push(WorldValidationIssue::new(
                    WorldIssueCode::Structure,
                    format!("structure {id:?} contains no exact solid voxels"),
                ));
            }
            for position in &structure.voxels {
                if !self.solid_voxel_exists(*position) {
                    issues.push(WorldValidationIssue::new(
                        WorldIssueCode::Structure,
                        format!(
                            "structure {id:?} names {position:?}, which is not solid planned volume"
                        ),
                    ));
                }
            }
        }
    }

    fn validate_biomes(&self, issues: &mut Vec<WorldValidationIssue>) {
        let surfaces: BTreeSet<_> = self.volume.surfaces.keys().copied().collect();
        let memberships: BTreeSet<_> = self.biome_regions.keys().copied().collect();
        if surfaces != memberships {
            issues.push(WorldValidationIssue::new(
                WorldIssueCode::Biome,
                "biome membership does not exactly cover every generated surface",
            ));
        }
        let declared_regions: BTreeSet<_> = self
            .layout
            .patches
            .values()
            .map(|patch| patch.biome_region)
            .collect();
        let owning_regions: BTreeMap<_, _> = self
            .layout
            .patches
            .values()
            .flat_map(|patch| {
                patch
                    .mask
                    .iter()
                    .copied()
                    .map(|coord| (coord, patch.biome_region))
            })
            .collect();
        for (position, region) in &self.biome_regions {
            if !declared_regions.contains(region) {
                issues.push(WorldValidationIssue::new(
                    WorldIssueCode::Biome,
                    format!("surface {position:?} names undeclared biome region {region:?}"),
                ));
            }
            match owning_regions.get(&position.coord) {
                Some(expected) if expected != region => {
                    issues.push(WorldValidationIssue::new(
                        WorldIssueCode::Biome,
                        format!(
                            "surface {position:?} names biome region {region:?}, but its patch \
                             owns region {expected:?}"
                        ),
                    ));
                }
                None => {
                    issues.push(WorldValidationIssue::new(
                        WorldIssueCode::Biome,
                        format!("surface {position:?} is not owned by any resolved patch"),
                    ));
                }
                Some(_) => {}
            }
        }
    }

    fn validate_interiors(&self, issues: &mut Vec<WorldValidationIssue>) {
        let mut floors = BTreeMap::new();
        let mut roofs = BTreeMap::new();
        for (region, interior) in &self.interiors.by_id {
            if interior.floors.is_empty() {
                issues.push(WorldValidationIssue::new(
                    WorldIssueCode::Interior,
                    format!("interior {region:?} contains no floor surfaces"),
                ));
            }
            if !interior.entrances.is_subset(&interior.floors) {
                issues.push(WorldValidationIssue::new(
                    WorldIssueCode::Interior,
                    format!("interior {region:?} has an entrance outside its floor set"),
                ));
            }
            for position in &interior.floors {
                if floors.insert(*position, *region).is_some() {
                    issues.push(WorldValidationIssue::new(
                        WorldIssueCode::Interior,
                        format!("interior floor {position:?} belongs to more than one region"),
                    ));
                }
            }
            for position in &interior.roof_voxels {
                if roofs.insert(*position, *region).is_some() {
                    issues.push(WorldValidationIssue::new(
                        WorldIssueCode::Interior,
                        format!("interior roof voxel {position:?} belongs to multiple regions"),
                    ));
                }
            }
        }

        let volume_floors: BTreeMap<_, _> = self
            .volume
            .surfaces
            .iter()
            .filter_map(|(position, metadata)| metadata.interior.map(|region| (*position, region)))
            .collect();
        if floors != volume_floors {
            issues.push(WorldValidationIssue::new(
                WorldIssueCode::Interior,
                "interior floors disagree with exact volume surface metadata",
            ));
        }

        let mut volume_roofs = BTreeMap::new();
        for (coord, column) in &self.volume.columns {
            for element in &column.elements {
                let VolumeElement::Solid(mass) = *element else {
                    continue;
                };
                let Some(region) = mass.cutaway_for else {
                    continue;
                };
                for level in mass.levels.bottom..mass.levels.top {
                    volume_roofs.insert(TilePos::new(*coord, level), region);
                }
            }
        }
        if roofs != volume_roofs {
            issues.push(WorldValidationIssue::new(
                WorldIssueCode::Interior,
                "interior roofs disagree with exact volume cutaway metadata",
            ));
        }
    }

    fn validate_lights(&self, issues: &mut Vec<WorldValidationIssue>) {
        for (id, light) in &self.lights {
            if !self.volume.surfaces.contains_key(&light.origin) {
                issues.push(WorldValidationIssue::new(
                    WorldIssueCode::Light,
                    format!("gameplay light {id:?} is not rooted on a generated surface"),
                ));
            }
            if light.radius == 0 {
                issues.push(WorldValidationIssue::new(
                    WorldIssueCode::Light,
                    format!("gameplay light {id:?} has zero radius"),
                ));
            }
            if light.level == IlluminationLevel::Dark {
                issues.push(WorldValidationIssue::new(
                    WorldIssueCode::Light,
                    format!("gameplay light {id:?} contributes no illumination"),
                ));
            }
        }
    }

    fn validate_anchors(&self, issues: &mut Vec<WorldValidationIssue>) {
        if self.anchors.is_empty() {
            issues.push(WorldValidationIssue::new(
                WorldIssueCode::Anchor,
                "generated world publishes no actor anchors",
            ));
        }
        for (name, position) in &self.anchors {
            if !valid_stable_name(name) {
                issues.push(WorldValidationIssue::new(
                    WorldIssueCode::Anchor,
                    format!("anchor name {name:?} is not a stable identifier"),
                ));
            }
            if !matches!(
                self.volume
                    .surfaces
                    .get(position)
                    .map(|metadata| metadata.access),
                Some(SurfaceAccess::Ordinary)
            ) {
                issues.push(WorldValidationIssue::new(
                    WorldIssueCode::Anchor,
                    format!("anchor {name:?} does not name ordinary walker footing"),
                ));
            }
            if self.blockers.contains(position) {
                issues.push(WorldValidationIssue::new(
                    WorldIssueCode::Anchor,
                    format!("anchor {name:?} is occupied by a traversal blocker"),
                ));
            }
        }
    }

    fn solid_voxel_exists(&self, position: TilePos) -> bool {
        self.volume
            .columns
            .get(&position.coord)
            .is_some_and(|column| {
                column.elements.iter().any(|element| {
                    let VolumeElement::Solid(mass) = *element else {
                        return false;
                    };
                    mass.levels.bottom <= position.level && position.level < mass.levels.top
                })
            })
    }
}

#[derive(Debug, Clone, Copy)]
struct SeamCrossing {
    body: LiquidBodyId,
    source: TilePos,
    target: TilePos,
    lane: (hex_core::HexCoord, hex_core::HexCoord),
    forward: bool,
}

fn oriented_seam_lane(
    edge: &ResolvedEdgeContract,
    source: hex_core::HexCoord,
    target: hex_core::HexCoord,
) -> Option<((hex_core::HexCoord, hex_core::HexCoord), bool)> {
    if edge.boundary_pairs.contains(&(source, target)) {
        Some(((source, target), true))
    } else if edge.boundary_pairs.contains(&(target, source)) {
        Some(((target, source), false))
    } else {
        None
    }
}

fn level_in_edge_band(level: i32, edge: &ResolvedEdgeContract) -> bool {
    (edge.elevation.min..=edge.elevation.max).contains(&level)
}

fn fills_touch(first: NonSolidFill, second: NonSolidFill) -> bool {
    first.levels.bottom < second.levels.top && second.levels.bottom < first.levels.top
}

fn normalized_coord_pair(
    first: hex_core::HexCoord,
    second: hex_core::HexCoord,
) -> (hex_core::HexCoord, hex_core::HexCoord) {
    if first <= second {
        (first, second)
    } else {
        (second, first)
    }
}

fn validate_uncontracted_liquid_crossings(
    layout: &ResolvedLayoutPlan,
    liquids: &LiquidPlan,
    runs_by_coord: &BTreeMap<hex_core::HexCoord, Vec<(TilePos, NonSolidFill)>>,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let patch_by_coord: BTreeMap<_, _> = layout
        .patches
        .iter()
        .flat_map(|(patch, resolved)| resolved.mask.iter().map(|coord| (*coord, *patch)))
        .collect();
    let contracted: BTreeSet<_> = layout
        .shared_edges
        .values()
        .flat_map(|edge| {
            edge.boundary_pairs
                .iter()
                .map(|(first, second)| normalized_coord_pair(*first, *second))
        })
        .collect();

    for (body_id, body) in &liquids.bodies {
        for (source, node) in &body.nodes {
            let Some(target) = node.downstream else {
                continue;
            };
            let crosses_patches =
                patch_by_coord.get(&source.coord) != patch_by_coord.get(&target.coord);
            if crosses_patches
                && !contracted.contains(&normalized_coord_pair(source.coord, target.coord))
            {
                issues.push(WorldValidationIssue::new(
                    WorldIssueCode::Liquid,
                    format!(
                        "liquid body {body_id:?} crosses uncontracted patch boundary: \
                         {source:?} -> {target:?}"
                    ),
                ));
            }
        }
    }

    for (first_coord, first_runs) in runs_by_coord {
        for second_coord in first_coord.neighbors() {
            if *first_coord >= second_coord
                || patch_by_coord.get(first_coord) == patch_by_coord.get(&second_coord)
                || contracted.contains(&normalized_coord_pair(*first_coord, second_coord))
            {
                continue;
            }
            let Some(second_runs) = runs_by_coord.get(&second_coord) else {
                continue;
            };
            for (first, first_fill) in first_runs {
                for (second, second_fill) in second_runs {
                    if fills_touch(*first_fill, *second_fill) {
                        issues.push(WorldValidationIssue::new(
                            WorldIssueCode::Liquid,
                            format!(
                                "liquid runs {first:?} and {second:?} touch across an \
                                 uncontracted patch boundary"
                            ),
                        ));
                    }
                }
            }
        }
    }
}

fn validate_dry_seam_contacts(
    edge_id: super::layout::ResolvedEdgeId,
    edge: &ResolvedEdgeContract,
    runs_by_coord: &BTreeMap<hex_core::HexCoord, Vec<(TilePos, NonSolidFill)>>,
    issues: &mut Vec<WorldValidationIssue>,
) {
    for lane in &edge.boundary_pairs {
        for (first, first_fill) in runs_by_coord.get(&lane.0).into_iter().flatten() {
            for (second, second_fill) in runs_by_coord.get(&lane.1).into_iter().flatten() {
                if fills_touch(*first_fill, *second_fill) {
                    issues.push(WorldValidationIssue::new(
                        WorldIssueCode::Liquid,
                        format!(
                            "dry seam {edge_id:?} has touching liquid runs {first:?} and {second:?}"
                        ),
                    ));
                }
            }
        }
    }
}

fn validate_directed_seam_contacts(
    edge_id: super::layout::ResolvedEdgeId,
    edge: &ResolvedEdgeContract,
    realized: &BTreeMap<(hex_core::HexCoord, hex_core::HexCoord), (TilePos, TilePos)>,
    runs_by_coord: &BTreeMap<hex_core::HexCoord, Vec<(TilePos, NonSolidFill)>>,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let realized_first: BTreeSet<_> = realized.values().map(|(first, _)| *first).collect();
    let realized_second: BTreeSet<_> = realized.values().map(|(_, second)| *second).collect();
    for lane in &edge.boundary_pairs {
        for (first, first_fill) in runs_by_coord.get(&lane.0).into_iter().flatten() {
            for (second, second_fill) in runs_by_coord.get(&lane.1).into_iter().flatten() {
                if !fills_touch(*first_fill, *second_fill) {
                    continue;
                }
                let inside_port =
                    realized_first.contains(first) && realized_second.contains(second);
                if !inside_port {
                    issues.push(WorldValidationIssue::new(
                        WorldIssueCode::Liquid,
                        format!(
                            "liquid runs {first:?} and {second:?} touch across seam {edge_id:?} \
                             outside its exact directed port"
                        ),
                    ));
                }
            }
        }
    }
}

fn append_volume_issues(issues: &mut Vec<WorldValidationIssue>, volume_issues: &[VolumeIssue]) {
    issues.extend(
        volume_issues
            .iter()
            .map(|issue| WorldValidationIssue::new(WorldIssueCode::Volume, issue.to_string())),
    );
}

fn append_liquid_issues(issues: &mut Vec<WorldValidationIssue>, liquid_issues: &[LiquidIssue]) {
    issues.extend(
        liquid_issues
            .iter()
            .map(|issue| WorldValidationIssue::new(WorldIssueCode::Liquid, issue.to_string())),
    );
}

fn valid_stable_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// Recipe-independent category for a validation issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum WorldIssueCode {
    Layout,
    Volume,
    Liquid,
    Feature,
    Structure,
    Blocker,
    Light,
    Biome,
    Interior,
    Anchor,
    View,
    Recipe(&'static str),
}

/// Typed validation failure. Diagnostics never infer ownership by parsing text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorldValidationIssue {
    pub(crate) code: WorldIssueCode,
    pub(crate) detail: String,
}

impl WorldValidationIssue {
    #[must_use]
    pub(crate) fn new(code: WorldIssueCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use hex_core::SpecialMovementRegion;

    use super::*;
    use crate::procedural_v3::layout::{
        HexSide, LayoutKind, PatchId, ResolvedEdgeContract, ResolvedEdgeId, ResolvedEdgeReference,
        ResolvedElevationBand, ResolvedLayoutPlan, ResolvedLiquidPort, ResolvedPatch, ResolvedPort,
        ResolvedWalkerPorts,
    };
    use crate::procedural_v3::liquid::{LiquidBodyId, LiquidBodyPlan, LiquidFlowState, LiquidNode};
    use crate::procedural_v3::volume::{
        FillMaterialRole, LevelInterval, NonSolidFill, SolidMass, SolidMaterialRole,
        SurfaceMetadata, VolumeColumn,
    };

    #[test]
    fn stable_names_are_deliberately_narrow() {
        for valid in ["party_start", "bridge_2", "a"] {
            assert!(valid_stable_name(valid), "{valid:?} should be valid");
        }
        for invalid in ["", "Party", "two words", "river.port", "café"] {
            assert!(!valid_stable_name(invalid), "{invalid:?} should be invalid");
        }
    }

    fn complete_stacked_plan() -> GeneratedWorldPlan {
        let coord = hex_core::HexCoord::ORIGIN;
        let floor = TilePos::new(coord, 0);
        let roof_surface = TilePos::new(coord, 6);
        let region = InteriorRegionId(3);
        let mask = BTreeSet::from([coord]);
        let edges = HexSide::ALL
            .into_iter()
            .map(|side| (side, ResolvedEdgeReference::WorldBoundary))
            .collect();
        let layout = ResolvedLayoutPlan {
            kind: LayoutKind::Single,
            grid_radius: 12,
            footprint: mask.clone(),
            patches: BTreeMap::from([(
                PatchId(0),
                ResolvedPatch {
                    biome_region: BiomeRegionId(0),
                    mask: mask.clone(),
                    edges,
                },
            )]),
            shared_edges: BTreeMap::new(),
        };
        let mut volume = VolumePlan::new(mask);
        volume.columns.insert(
            coord,
            VolumeColumn {
                elements: vec![
                    VolumeElement::Solid(SolidMass {
                        levels: LevelInterval::new(0, 1),
                        material: SolidMaterialRole::Stone,
                        cutaway_for: None,
                    }),
                    VolumeElement::Solid(SolidMass {
                        levels: LevelInterval::new(4, 7),
                        material: SolidMaterialRole::Stone,
                        cutaway_for: Some(region),
                    }),
                ],
            },
        );
        volume.surfaces.insert(
            floor,
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: Some(region),
            },
        );
        volume.surfaces.insert(
            roof_surface,
            SurfaceMetadata {
                access: SurfaceAccess::SpecialMovement(SpecialMovementRegion(2)),
                interior: None,
            },
        );

        GeneratedWorldPlan {
            layout,
            volume,
            liquids: LiquidPlan::default(),
            features: FeaturePlan::default(),
            structures: StructurePlan {
                by_id: BTreeMap::from([(
                    StructureId(0),
                    PlannedStructure {
                        kind: StructureKind::Bridge,
                        voxels: BTreeSet::from([TilePos::new(coord, 4)]),
                    },
                )]),
            },
            blockers: BTreeSet::new(),
            lights: BTreeMap::from([(
                LightId(0),
                PlannedGameplayLight {
                    origin: floor,
                    level: IlluminationLevel::Bright,
                    radius: 4,
                },
            )]),
            biome_regions: BTreeMap::from([
                (floor, BiomeRegionId(0)),
                (roof_surface, BiomeRegionId(0)),
            ]),
            interiors: InteriorPlan {
                by_id: BTreeMap::from([(
                    region,
                    PlannedInterior {
                        floors: BTreeSet::from([floor]),
                        entrances: BTreeSet::from([floor]),
                        roof_voxels: (4..7).map(|level| TilePos::new(coord, level)).collect(),
                    },
                )]),
            },
            anchors: BTreeMap::from([("party_start".to_owned(), floor)]),
            view_hint: MapViewHint::new((1.0, 4.0, 2.0), (0.0, 0.0, 0.0)),
        }
    }

    fn two_patch_liquid_plan() -> GeneratedWorldPlan {
        let first_low = hex_core::HexCoord::ORIGIN;
        let first_high = hex_core::HexCoord::new_cubic(0, 1, -1);
        let second_low = HexSide::East.neighbor(first_low);
        let second_high = HexSide::East.neighbor(first_high);
        let first_mask = BTreeSet::from([first_low, first_high]);
        let second_mask = BTreeSet::from([second_low, second_high]);
        let footprint: BTreeSet<_> = first_mask.union(&second_mask).copied().collect();
        let edge_id = ResolvedEdgeId(0);
        let mut first_edges = HexSide::ALL
            .into_iter()
            .map(|side| (side, ResolvedEdgeReference::WorldBoundary))
            .collect::<BTreeMap<_, _>>();
        first_edges.insert(HexSide::East, ResolvedEdgeReference::Shared(edge_id));
        let mut second_edges = HexSide::ALL
            .into_iter()
            .map(|side| (side, ResolvedEdgeReference::WorldBoundary))
            .collect::<BTreeMap<_, _>>();
        second_edges.insert(HexSide::West, ResolvedEdgeReference::Shared(edge_id));
        let lanes = BTreeSet::from([(first_low, second_low), (first_high, second_high)]);
        let boundary_pairs = BTreeSet::from([
            (first_low, second_low),
            (first_high, second_low),
            (first_high, second_high),
        ]);
        let port = ResolvedPort {
            lanes: lanes.clone(),
            first_approach: first_mask.clone(),
            second_approach: second_mask.clone(),
        };
        let layout = ResolvedLayoutPlan {
            kind: LayoutKind::Ring7,
            grid_radius: 12,
            footprint: footprint.clone(),
            patches: BTreeMap::from([
                (
                    PatchId(0),
                    ResolvedPatch {
                        biome_region: BiomeRegionId(0),
                        mask: first_mask.clone(),
                        edges: first_edges,
                    },
                ),
                (
                    PatchId(1),
                    ResolvedPatch {
                        biome_region: BiomeRegionId(1),
                        mask: second_mask.clone(),
                        edges: second_edges,
                    },
                ),
            ]),
            shared_edges: BTreeMap::from([(
                edge_id,
                ResolvedEdgeContract {
                    first: (PatchId(0), HexSide::East),
                    second: (PatchId(1), HexSide::West),
                    elevation: ResolvedElevationBand {
                        preferred: 3,
                        min: 2,
                        max: 4,
                    },
                    walker: ResolvedWalkerPorts {
                        count: 0,
                        width: 0,
                        ports: Vec::new(),
                    },
                    liquid: ResolvedLiquidPort::Directed {
                        source: PatchId(0),
                        sink: PatchId(1),
                        port,
                    },
                    approach_depth: 1,
                    boundary_pairs,
                    protected_approaches: BTreeMap::from([
                        (PatchId(0), first_mask),
                        (PatchId(1), second_mask),
                    ]),
                },
            )]),
        };
        let mut volume = VolumePlan::new(footprint);
        for coord in [first_low, first_high, second_low, second_high] {
            volume
                .columns
                .get_mut(&coord)
                .unwrap()
                .elements
                .push(VolumeElement::Fill(NonSolidFill {
                    levels: LevelInterval::new(2, 4),
                    material: FillMaterialRole::Water,
                }));
        }
        let first_low = TilePos::new(first_low, 3);
        let first_high = TilePos::new(first_high, 3);
        let second_low = TilePos::new(second_low, 3);
        let second_high = TilePos::new(second_high, 3);

        GeneratedWorldPlan {
            layout,
            volume,
            liquids: LiquidPlan {
                bodies: BTreeMap::from([(
                    LiquidBodyId(0),
                    LiquidBodyPlan {
                        material: FillMaterialRole::Water,
                        nodes: BTreeMap::from([
                            (
                                first_low,
                                LiquidNode {
                                    state: LiquidFlowState::Current,
                                    downstream: Some(second_low),
                                },
                            ),
                            (
                                first_high,
                                LiquidNode {
                                    state: LiquidFlowState::Current,
                                    downstream: Some(second_high),
                                },
                            ),
                            (
                                second_low,
                                LiquidNode {
                                    state: LiquidFlowState::Still,
                                    downstream: None,
                                },
                            ),
                            (
                                second_high,
                                LiquidNode {
                                    state: LiquidFlowState::Still,
                                    downstream: None,
                                },
                            ),
                        ]),
                    },
                )]),
            },
            features: FeaturePlan::default(),
            structures: StructurePlan::default(),
            blockers: BTreeSet::new(),
            lights: BTreeMap::new(),
            biome_regions: BTreeMap::new(),
            interiors: InteriorPlan::default(),
            anchors: BTreeMap::new(),
            view_hint: MapViewHint::new((1.0, 4.0, 2.0), (0.0, 0.0, 0.0)),
        }
    }

    fn seam_issues(plan: &GeneratedWorldPlan) -> Vec<WorldValidationIssue> {
        let mut issues = Vec::new();
        plan.validate_liquid_seams(&mut issues);
        issues
    }

    fn liquid_node_mut(plan: &mut GeneratedWorldPlan, position: TilePos) -> &mut LiquidNode {
        let Some(body) = plan.liquids.bodies.get_mut(&LiquidBodyId(0)) else {
            panic!("the seam fixture should contain its liquid body");
        };
        let Some(node) = body.nodes.get_mut(&position) else {
            panic!("the seam fixture should contain node {position:?}");
        };
        node
    }

    #[test]
    fn directed_liquid_seams_require_every_exact_lane() {
        let plan = two_patch_liquid_plan();
        assert_eq!(seam_issues(&plan), Vec::new());

        let mut missing = plan.clone();
        let source = TilePos::new(hex_core::HexCoord::new_cubic(0, 1, -1), 3);
        *liquid_node_mut(&mut missing, source) = LiquidNode {
            state: LiquidFlowState::Still,
            downstream: None,
        };
        assert!(seam_issues(&missing)
            .iter()
            .any(|issue| issue.detail.contains("does not realize exact port lane")));

        let mut outside_port = plan;
        let first_low = hex_core::HexCoord::ORIGIN;
        let second_low = HexSide::East.neighbor(first_low);
        let first_high = hex_core::HexCoord::new_cubic(0, 1, -1);
        let Some(edge) = outside_port.layout.shared_edges.get_mut(&ResolvedEdgeId(0)) else {
            panic!("the seam fixture should contain its edge");
        };
        let ResolvedLiquidPort::Directed { port, .. } = &mut edge.liquid else {
            panic!("the seam fixture should contain a directed port");
        };
        port.lanes = BTreeSet::from([(first_low, second_low)]);
        *liquid_node_mut(&mut outside_port, TilePos::new(first_high, 3)) = LiquidNode {
            state: LiquidFlowState::Still,
            downstream: None,
        };
        assert!(seam_issues(&outside_port)
            .iter()
            .any(|issue| issue.detail.contains("outside its exact directed port")));

        let mut stacked = two_patch_liquid_plan();
        for coord in [first_low, second_low] {
            let Some(column) = stacked.volume.columns.get_mut(&coord) else {
                panic!("the seam fixture should contain column {coord:?}");
            };
            column.elements.push(VolumeElement::Fill(NonSolidFill {
                levels: LevelInterval::new(5, 7),
                material: FillMaterialRole::Water,
            }));
        }
        assert!(seam_issues(&stacked).iter().any(|issue| {
            issue.detail.contains("outside its exact directed port")
                && issue.detail.contains("level: 6")
        }));
    }

    #[test]
    fn liquid_seams_reject_reversed_extra_and_dry_crossings() {
        let plan = two_patch_liquid_plan();
        let first_low = TilePos::new(hex_core::HexCoord::ORIGIN, 3);
        let second_low = TilePos::new(HexSide::East.neighbor(hex_core::HexCoord::ORIGIN), 3);

        let mut reversed = plan.clone();
        *liquid_node_mut(&mut reversed, first_low) = LiquidNode {
            state: LiquidFlowState::Still,
            downstream: None,
        };
        *liquid_node_mut(&mut reversed, second_low) = LiquidNode {
            state: LiquidFlowState::Current,
            downstream: Some(first_low),
        };
        assert!(seam_issues(&reversed)
            .iter()
            .any(|issue| issue.detail.contains("outside its exact directed port")));

        let mut extra = plan.clone();
        let first_high = TilePos::new(hex_core::HexCoord::new_cubic(0, 1, -1), 3);
        *liquid_node_mut(&mut extra, first_high) = LiquidNode {
            state: LiquidFlowState::Current,
            downstream: Some(second_low),
        };
        assert!(seam_issues(&extra)
            .iter()
            .any(|issue| issue.detail.contains("outside its exact directed port")));

        let mut dry = plan;
        let Some(edge) = dry.layout.shared_edges.get_mut(&ResolvedEdgeId(0)) else {
            panic!("the seam fixture should contain its edge");
        };
        edge.liquid = ResolvedLiquidPort::Dry;
        let dry_issues = seam_issues(&dry);
        assert!(dry_issues
            .iter()
            .any(|issue| issue.detail.contains("dry seam") && issue.detail.contains("crossing")));
        assert!(dry_issues.iter().any(|issue| {
            issue.detail.contains("dry seam") && issue.detail.contains("touching liquid runs")
        }));

        let mut uncontracted = two_patch_liquid_plan();
        let Some(edge) = uncontracted.layout.shared_edges.get_mut(&ResolvedEdgeId(0)) else {
            panic!("the seam fixture should contain its edge");
        };
        edge.boundary_pairs.clear();
        let uncontracted_issues = seam_issues(&uncontracted);
        assert!(uncontracted_issues
            .iter()
            .any(|issue| issue.detail.contains("uncontracted patch boundary")));
    }

    #[test]
    fn complete_stacked_plan_cross_checks_every_exact_layer() {
        let plan = complete_stacked_plan();
        assert_eq!(plan.validate(), Vec::new());
    }

    #[test]
    fn cross_layer_corruption_has_typed_ownership() {
        type Mutator = fn(&mut GeneratedWorldPlan);
        let cases: &[(WorldIssueCode, Mutator)] = &[
            (WorldIssueCode::Biome, |plan| {
                plan.biome_regions.pop_last();
            }),
            (WorldIssueCode::Interior, |plan| {
                plan.interiors.by_id.clear();
            }),
            (WorldIssueCode::Light, |plan| {
                plan.lights.get_mut(&LightId(0)).unwrap().radius = 0;
            }),
            (WorldIssueCode::Structure, |plan| {
                plan.structures
                    .by_id
                    .get_mut(&StructureId(0))
                    .unwrap()
                    .voxels
                    .insert(TilePos::new(hex_core::HexCoord::ORIGIN, 3));
            }),
            (WorldIssueCode::Liquid, |plan| {
                plan.liquids.bodies.insert(
                    LiquidBodyId(0),
                    LiquidBodyPlan {
                        material: FillMaterialRole::Water,
                        nodes: BTreeMap::from([(
                            TilePos::new(hex_core::HexCoord::ORIGIN, 2),
                            LiquidNode {
                                state: LiquidFlowState::Still,
                                downstream: None,
                            },
                        )]),
                    },
                );
            }),
            (WorldIssueCode::Anchor, |plan| {
                plan.anchors.insert(
                    "Invalid Name".to_owned(),
                    TilePos::new(hex_core::HexCoord::ORIGIN, 0),
                );
            }),
            (WorldIssueCode::View, |plan| {
                plan.view_hint.eye.0 = f32::NAN;
            }),
        ];

        for (expected, mutate) in cases {
            let mut plan = complete_stacked_plan();
            mutate(&mut plan);
            let issues = plan.validate();
            assert!(
                issues.iter().any(|issue| issue.code == *expected),
                "expected {expected:?}, got {issues:?}"
            );
        }
    }

    #[test]
    fn stacked_surfaces_must_use_their_owning_patch_biome() {
        let mut plan = complete_stacked_plan();
        let foreign_coord = hex_core::HexCoord::new_cubic(1, -1, 0);
        let template = plan
            .layout
            .patches
            .get(&PatchId(0))
            .cloned()
            .expect("fixture has one patch");
        plan.layout.patches.insert(
            PatchId(1),
            ResolvedPatch {
                biome_region: BiomeRegionId(1),
                mask: BTreeSet::from([foreign_coord]),
                edges: template.edges,
            },
        );
        for region in plan.biome_regions.values_mut() {
            *region = BiomeRegionId(1);
        }

        let mut issues = Vec::new();
        plan.validate_biomes(&mut issues);

        let wrong_owner_count = issues
            .iter()
            .filter(|issue| {
                issue.code == WorldIssueCode::Biome && issue.detail.contains("its patch owns")
            })
            .count();
        assert_eq!(
            wrong_owner_count, 2,
            "both stacked surfaces must retain the horizontal patch's biome: {issues:?}"
        );
    }
}
