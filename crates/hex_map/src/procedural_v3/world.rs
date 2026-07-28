//! Complete semantic world storage for procedural generator V3.
//!
//! Recipe planners work against this private representation. Runtime consumers see
//! only the exact projections produced after the whole plan has passed common and
//! recipe-specific validation.

use std::collections::{BTreeMap, BTreeSet};

use hex_core::{BiomeRegionId, IlluminationLevel, InteriorRegionId, MapViewHint, TilePos};

use super::layout::ResolvedLayoutPlan;
use super::liquid::LiquidIssue;
pub(crate) use super::liquid::LiquidPlan;
use super::volume::{SurfaceAccess, VolumeElement, VolumeIssue, VolumePlan};

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
        HexSide, LayoutKind, PatchId, ResolvedEdgeReference, ResolvedLayoutPlan, ResolvedPatch,
    };
    use crate::procedural_v3::liquid::{LiquidBodyId, LiquidBodyPlan, LiquidFlowState, LiquidNode};
    use crate::procedural_v3::volume::{
        FillMaterialRole, LevelInterval, SolidMass, SolidMaterialRole, SurfaceMetadata,
        VolumeColumn,
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
