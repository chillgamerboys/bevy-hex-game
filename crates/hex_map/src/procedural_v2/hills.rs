//! V2 Hills compatibility recipe.
//!
//! V1 remains the source of candidate geometry, bounded repairs, exact validation, and
//! scoring until parity is locked. This module converts each finalized candidate into
//! the recipe-independent V2 volume without interpreting or regenerating its topology.

use std::collections::BTreeMap;

use hex_core::{
    HexCoord, MapViewHint, SpecialMovementRegions, SubstanceId, TilePos, TraversalProfile,
};

use super::recipe::{
    materialize_selection, run_recipe, CandidateAttemptError, CandidateContext, FallbackContext,
    MaterializedSelection, RecipePlan, RecipeValidation, RepairOutcome, V2Recipe,
    ValidationContext,
};
use super::volume::{
    FillMaterialRole, LevelInterval, NonSolidFill, SolidMass, SolidMaterialRole, SurfaceAccess,
    SurfaceMetadata, TerrainVolumePlan, VolumeColumn, VolumeElement,
};
use super::V2GenerationError;
use crate::procedural::{self, CandidateScore, TacticalMetrics, V1HillsCandidate, V1HillsTopology};
use crate::settings::{
    CrossingSettings, EnvironmentSettings, HillsSettings, LandformSettings, ProceduralV1Settings,
    ProceduralV2Settings, TacticalSettings, V2EnvironmentSettings, V2RecipeSettings,
};
use crate::terrain::TerrainPalette;
use crate::voxel::{Column, VoxelMap};

const FALLBACK_SEED: u64 = 0;

/// V1 measurements and ordering key cached with one converted candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HillsMetrics {
    pub(crate) tactical: TacticalMetrics,
    score: CandidateScore,
}

/// Hills-only semantic facts retained for later layered recipes and diagnostics.
#[derive(Debug, Clone)]
pub(crate) struct HillsMetadata {
    pub(crate) topology: V1HillsTopology,
    repair_actions: Vec<String>,
    metrics: HillsMetrics,
}

struct HillsRecipe<'a> {
    settings: ProceduralV1Settings,
    view_hint: MapViewHint,
    palette: &'a TerrainPalette,
    is_solid: &'a dyn Fn(SubstanceId) -> bool,
}

/// Generates and materializes V2 Hills while V1 remains the parity oracle.
pub(crate) fn build(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV2Settings,
    seed: u64,
    palette: &TerrainPalette,
    is_solid: &dyn Fn(SubstanceId) -> bool,
) -> Result<MaterializedSelection<HillsMetadata, HillsMetrics>, V2GenerationError> {
    let view_hint = hills_view_hint(grid_radius, level_height, settings)?;
    let recipe = HillsRecipe {
        settings: canonical_v1_settings(settings)?,
        view_hint,
        palette,
        is_solid,
    };
    let selection = run_recipe(&recipe, &(), grid_radius, seed)?;
    materialize_selection(selection, palette, is_solid)
}

impl V2Recipe for HillsRecipe<'_> {
    type Settings = ();
    type Metadata = HillsMetadata;
    type Metrics = HillsMetrics;
    type Score = CandidateScore;

    fn construct(
        &self,
        context: CandidateContext,
        _settings: &Self::Settings,
    ) -> Result<RecipePlan<Self::Metadata>, CandidateAttemptError> {
        let candidate = procedural::build_hills_candidate_for_v2_parity(
            context.grid_radius,
            &self.settings,
            context.seed,
            context.candidate,
            false,
            self.palette,
            self.is_solid,
        )
        .map_err(|reason| {
            CandidateAttemptError::fatal(V2GenerationError::MaterialContract(format!(
                "V1 Hills parity adapter failed: {reason}"
            )))
        })?;
        if !candidate.valid {
            return Err(CandidateAttemptError::Rejected(candidate.validation_notes));
        }
        candidate_to_plan(context.grid_radius, candidate, self.palette, self.view_hint)
            .map_err(CandidateAttemptError::fatal)
    }

    fn validate(
        &self,
        _context: ValidationContext,
        _settings: &Self::Settings,
        plan: &RecipePlan<Self::Metadata>,
    ) -> RecipeValidation<Self::Metrics> {
        RecipeValidation::valid(plan.metadata.metrics)
    }

    fn repair(
        &self,
        _context: CandidateContext,
        _settings: &Self::Settings,
        _plan: &mut RecipePlan<Self::Metadata>,
        _round: u8,
        _issues: &[String],
    ) -> Result<RepairOutcome, CandidateAttemptError> {
        Ok(RepairOutcome::NoChange)
    }

    fn score(
        &self,
        _settings: &Self::Settings,
        metrics: &Self::Metrics,
        _candidate: u8,
    ) -> Self::Score {
        metrics.score
    }

    fn preexisting_repair_actions(&self, plan: &RecipePlan<Self::Metadata>) -> Vec<String> {
        plan.metadata.repair_actions.clone()
    }

    fn canonical_fallback(
        &self,
        context: FallbackContext,
        _settings: &Self::Settings,
    ) -> Result<RecipePlan<Self::Metadata>, V2GenerationError> {
        let candidate = procedural::build_hills_candidate_for_v2_parity(
            context.grid_radius,
            &self.settings,
            FALLBACK_SEED,
            0,
            true,
            self.palette,
            self.is_solid,
        )
        .map_err(|reason| {
            V2GenerationError::InvalidFallback(vec![format!(
                "V1 Hills fallback adapter failed: {reason}"
            )])
        })?;
        if !candidate.valid {
            return Err(V2GenerationError::InvalidFallback(
                candidate.validation_notes,
            ));
        }
        candidate_to_plan(context.grid_radius, candidate, self.palette, self.view_hint)
    }
}

fn hills_view_hint(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV2Settings,
) -> Result<MapViewHint, V2GenerationError> {
    let V2RecipeSettings::Hills(hills) = &settings.recipe else {
        return Err(V2GenerationError::RecipeUnavailable("Hills"));
    };
    let valley_level = i16::try_from(hills.valley_level).map_err(|_out_of_range| {
        V2GenerationError::InvalidVolume(vec![
            "Hills valley level cannot be represented by the camera frame".to_owned(),
        ])
    })?;
    let radius = u16::try_from(grid_radius).map_err(|_out_of_range| {
        V2GenerationError::InvalidVolume(vec![
            "Hills radius cannot be represented by the camera frame".to_owned(),
        ])
    })?;
    let focus_height = f32::from(valley_level) * level_height;
    let frame_distance = f32::from(radius) * 3.5;
    Ok(MapViewHint::new(
        (0.0, focus_height + frame_distance, frame_distance),
        (0.0, focus_height, 0.0),
    ))
}

fn canonical_v1_settings(
    settings: &ProceduralV2Settings,
) -> Result<ProceduralV1Settings, V2GenerationError> {
    let V2RecipeSettings::Hills(hills) = &settings.recipe else {
        return Err(V2GenerationError::RecipeUnavailable("Hills"));
    };
    let environment = match settings.environment {
        V2EnvironmentSettings::TemperateGrassland => EnvironmentSettings::TemperateGrassland,
        V2EnvironmentSettings::Frozen => EnvironmentSettings::Frozen,
        V2EnvironmentSettings::Volcanic => EnvironmentSettings::Volcanic,
        V2EnvironmentSettings::Rocky => {
            return Err(V2GenerationError::RecipeUnavailable(
                "Hills with Rocky environment",
            ));
        }
    };
    let crossing = hills
        .derived_crossing()
        .map_err(|reason| V2GenerationError::InvalidVolume(vec![reason]))?;

    Ok(ProceduralV1Settings {
        landform: LandformSettings::Hills(HillsSettings {
            valley_level: hills.valley_level,
            max_relief: hills.max_relief,
            hills_per_bank: hills.hills_per_bank,
        }),
        environment,
        tactical: TacticalSettings::Crossing(CrossingSettings {
            barrier_half_width: crossing.hazard_half_width,
            bed_level: crossing.bed_level,
            hazard_bottom: crossing.hazard_bottom,
            hazard_top: crossing.hazard_top,
            bridge_level: crossing.bridge_level,
        }),
    })
}

fn candidate_to_plan(
    grid_radius: u32,
    candidate: V1HillsCandidate,
    palette: &TerrainPalette,
    view_hint: MapViewHint,
) -> Result<RecipePlan<HillsMetadata>, V2GenerationError> {
    let V1HillsCandidate {
        map,
        anchors,
        special_regions,
        metrics,
        valid: _,
        validation_notes: _,
        repair_actions,
        score,
        topology,
    } = candidate;
    let anchors = anchors
        .iter()
        .map(|(name, position)| (name.to_owned(), position))
        .collect();
    let volume = convert_map(
        grid_radius,
        &map,
        anchors,
        &special_regions,
        palette,
        view_hint,
    )?;
    Ok(RecipePlan {
        volume,
        metadata: HillsMetadata {
            topology,
            repair_actions,
            metrics: HillsMetrics {
                tactical: metrics,
                score,
            },
        },
    })
}

fn convert_map(
    grid_radius: u32,
    map: &VoxelMap,
    anchors: BTreeMap<String, TilePos>,
    special_regions: &SpecialMovementRegions,
    palette: &TerrainPalette,
    view_hint: MapViewHint,
) -> Result<TerrainVolumePlan, V2GenerationError> {
    let mut columns = BTreeMap::new();
    for (coord, column) in map.columns() {
        columns.insert(coord, convert_column(coord, column, palette)?);
    }

    let mut surfaces = BTreeMap::new();
    for (coord, column) in &columns {
        let source = map.column(*coord).ok_or_else(|| {
            V2GenerationError::MaterialContract(format!(
                "converted Hills coordinate {coord:?} lost its source column"
            ))
        })?;
        for (index, element) in column.elements.iter().copied().enumerate() {
            let VolumeElement::Solid(mass) = element else {
                continue;
            };
            let covered_by_solid = column.elements.get(index + 1).is_some_and(|next| {
                matches!(next, VolumeElement::Solid(_))
                    && element_levels(*next).bottom == mass.levels.top
            });
            if covered_by_solid {
                continue;
            }
            let position = TilePos::new(*coord, mass.levels.top.saturating_sub(1));
            let access = if let Some(region) = special_regions.get(position) {
                SurfaceAccess::SpecialMovement(region)
            } else if TraversalProfile::WALKER
                .admits_surface(true, source.headroom_above(mass.levels.top))
            {
                SurfaceAccess::Ordinary
            } else {
                SurfaceAccess::NonStandable
            };
            surfaces.insert(
                position,
                SurfaceMetadata {
                    access,
                    interior: None,
                },
            );
        }
    }

    for (position, expected) in special_regions.iter() {
        let actual = surfaces.get(&position).map(|surface| surface.access);
        if actual != Some(SurfaceAccess::SpecialMovement(expected)) {
            return Err(V2GenerationError::MaterialContract(format!(
                "V1 special-movement surface {position:?} has no matching converted solid boundary"
            )));
        }
    }

    Ok(TerrainVolumePlan {
        grid_radius,
        columns,
        surfaces,
        anchors,
        interiors: BTreeMap::new(),
        view_hint,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SemanticMaterial {
    Solid(SolidMaterialRole),
    Fill(FillMaterialRole),
}

fn convert_column(
    coord: HexCoord,
    column: &Column,
    palette: &TerrainPalette,
) -> Result<VolumeColumn, V2GenerationError> {
    let mut elements = Vec::new();
    for (index, substance) in column.iter().enumerate() {
        if substance.is_air() {
            continue;
        }
        let level = i32::try_from(index).unwrap_or(i32::MAX);
        let material = semantic_material(substance, palette).map_err(|reason| {
            V2GenerationError::MaterialContract(format!(
                "cannot convert Hills voxel at {:?}: {reason}",
                TilePos::new(coord, level)
            ))
        })?;
        if extend_last(&mut elements, level, material) {
            continue;
        }
        let levels = LevelInterval::new(level, level.saturating_add(1));
        elements.push(match material {
            SemanticMaterial::Solid(material) => VolumeElement::Solid(SolidMass {
                levels,
                material,
                cutaway_for: None,
            }),
            SemanticMaterial::Fill(material) => {
                VolumeElement::Fill(NonSolidFill { levels, material })
            }
        });
    }
    Ok(VolumeColumn { elements })
}

fn extend_last(elements: &mut [VolumeElement], level: i32, material: SemanticMaterial) -> bool {
    let Some(last) = elements.last_mut() else {
        return false;
    };
    match (last, material) {
        (VolumeElement::Solid(mass), SemanticMaterial::Solid(role))
            if mass.material == role && mass.levels.top == level =>
        {
            mass.levels.top = level.saturating_add(1);
            true
        }
        (VolumeElement::Fill(fill), SemanticMaterial::Fill(role))
            if fill.material == role && fill.levels.top == level =>
        {
            fill.levels.top = level.saturating_add(1);
            true
        }
        _ => false,
    }
}

fn semantic_material(
    substance: SubstanceId,
    palette: &TerrainPalette,
) -> Result<SemanticMaterial, String> {
    let roles = [
        (
            palette.bedrock,
            SemanticMaterial::Solid(SolidMaterialRole::Bedrock),
        ),
        (
            palette.stone,
            SemanticMaterial::Solid(SolidMaterialRole::Stone),
        ),
        (
            palette.dirt,
            SemanticMaterial::Solid(SolidMaterialRole::Dirt),
        ),
        (
            palette.grass,
            SemanticMaterial::Solid(SolidMaterialRole::Grass),
        ),
        (
            palette.gravel,
            SemanticMaterial::Solid(SolidMaterialRole::Gravel),
        ),
        (
            palette.metal,
            SemanticMaterial::Solid(SolidMaterialRole::Metal),
        ),
        (
            palette.snow,
            SemanticMaterial::Solid(SolidMaterialRole::Snow),
        ),
        (palette.ice, SemanticMaterial::Solid(SolidMaterialRole::Ice)),
        (
            palette.basalt,
            SemanticMaterial::Solid(SolidMaterialRole::Basalt),
        ),
        (
            palette.water,
            SemanticMaterial::Fill(FillMaterialRole::Water),
        ),
        (palette.lava, SemanticMaterial::Fill(FillMaterialRole::Lava)),
    ];
    let mut matches = roles
        .into_iter()
        .filter_map(|(candidate, role)| (candidate == substance).then_some(role));
    let Some(role) = matches.next() else {
        return Err(format!("substance {substance:?} has no V2 semantic role"));
    };
    if matches.next().is_some() {
        return Err(format!(
            "substance {substance:?} maps to multiple V2 semantic roles"
        ));
    }
    Ok(role)
}

const fn element_levels(element: VolumeElement) -> LevelInterval {
    match element {
        VolumeElement::Solid(mass) => mass.levels,
        VolumeElement::Fill(fill) => fill.levels,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use hex_core::{MapAnchorId, SpecialMovementRegion};

    use super::*;
    use crate::procedural::map_fingerprint;
    use crate::procedural_v2::volume::voxelize;

    const BEDROCK: SubstanceId = SubstanceId(1);
    const STONE: SubstanceId = SubstanceId(2);
    const DIRT: SubstanceId = SubstanceId(3);
    const GRASS: SubstanceId = SubstanceId(4);
    const GRAVEL: SubstanceId = SubstanceId(5);
    const WATER: SubstanceId = SubstanceId(6);
    const METAL: SubstanceId = SubstanceId(7);
    const SNOW: SubstanceId = SubstanceId(8);
    const ICE: SubstanceId = SubstanceId(9);
    const BASALT: SubstanceId = SubstanceId(10);
    const LAVA: SubstanceId = SubstanceId(11);

    fn palette() -> TerrainPalette {
        TerrainPalette {
            bedrock: BEDROCK,
            stone: STONE,
            dirt: DIRT,
            grass: GRASS,
            gravel: GRAVEL,
            water: WATER,
            metal: METAL,
            snow: SNOW,
            ice: ICE,
            basalt: BASALT,
            lava: LAVA,
        }
    }

    fn is_solid(substance: SubstanceId) -> bool {
        !matches!(substance, SubstanceId::AIR | WATER | LAVA)
    }

    fn settings(environment: V2EnvironmentSettings) -> ProceduralV2Settings {
        ProceduralV2Settings {
            environment,
            recipe: V2RecipeSettings::Hills(crate::settings::V2HillsSettings {
                valley_level: 15,
                max_relief: 8,
                hills_per_bank: 3,
            }),
        }
    }

    fn sorted_regions(regions: &SpecialMovementRegions) -> Vec<(TilePos, SpecialMovementRegion)> {
        let mut memberships: Vec<_> = regions.iter().collect();
        memberships.sort_unstable();
        memberships
    }

    #[test]
    fn v2_hills_selection_round_trips_the_v1_map_and_diagnostics() {
        for (environment, seed) in [
            (V2EnvironmentSettings::TemperateGrassland, 1_592_598_566),
            (V2EnvironmentSettings::Frozen, 484_450_342),
            (V2EnvironmentSettings::Volcanic, 444_211_238),
        ] {
            let v2_settings = settings(environment);
            let v1_settings =
                canonical_v1_settings(&v2_settings).expect("the Hills mapping should be valid");
            let legacy = procedural::build(
                12,
                &v1_settings,
                seed,
                &palette(),
                TraversalProfile::WALKER,
                &is_solid,
            );
            let converted = build(12, 0.4, &v2_settings, seed, &palette(), &is_solid)
                .expect("V2 should losslessly select and materialize V1 Hills");

            assert_eq!(
                converted.selected_candidate,
                legacy.report.selected_candidate
            );
            assert_eq!(converted.valid_candidates, legacy.report.valid_candidates);
            assert_eq!(converted.repair_actions, legacy.report.repair_actions);
            assert_eq!(converted.metrics.tactical, legacy.report.metrics);
            assert_eq!(converted.map_fingerprint, legacy.report.map_fingerprint);
            assert!(!converted.used_fallback);
            let _candidate_diagnostics = &converted.notes;
            assert!(converted.interiors.is_empty());
            assert_eq!(
                converted.view_hint,
                MapViewHint::new((0.0, 48.0, 42.0), (0.0, 6.0, 0.0))
            );
            assert!(!converted.metadata.topology.barrier.is_empty());
            assert!(!converted.metadata.topology.bridge.is_empty());
            assert!(!converted.metadata.topology.alternate_crossing.is_empty());
            assert!(converted
                .metadata
                .topology
                .bridge
                .iter()
                .chain(&converted.metadata.topology.alternate_crossing)
                .all(|position| converted
                    .metadata
                    .topology
                    .protected_approaches
                    .contains(&position.coord)));

            let expected_anchors: BTreeMap<String, TilePos> = legacy
                .anchors
                .iter()
                .map(|(name, position)| (name.to_owned(), position))
                .collect();
            let actual_anchors: BTreeMap<String, TilePos> = converted
                .anchors
                .iter()
                .map(|(name, position)| (name.as_str().to_owned(), position))
                .collect();
            assert_eq!(actual_anchors, expected_anchors);
            assert_eq!(
                sorted_regions(&converted.special_regions),
                sorted_regions(&legacy.special_regions)
            );
        }
    }

    #[test]
    fn view_hint_scales_with_radius_and_level_height() {
        let settings = settings(V2EnvironmentSettings::TemperateGrassland);
        assert_eq!(
            hills_view_hint(20, 0.5, &settings).expect("the Hills hint should derive"),
            MapViewHint::new((0.0, 77.5, 70.0), (0.0, 7.5, 0.0))
        );
    }

    #[test]
    fn converted_materials_preserve_each_environment_and_hazard() {
        for (environment, seed, expected_fill, required_solids) in [
            (
                V2EnvironmentSettings::TemperateGrassland,
                1_592_598_566,
                FillMaterialRole::Water,
                vec![
                    SolidMaterialRole::Bedrock,
                    SolidMaterialRole::Stone,
                    SolidMaterialRole::Dirt,
                    SolidMaterialRole::Grass,
                    SolidMaterialRole::Gravel,
                    SolidMaterialRole::Metal,
                ],
            ),
            (
                V2EnvironmentSettings::Frozen,
                484_450_342,
                FillMaterialRole::Water,
                vec![
                    SolidMaterialRole::Bedrock,
                    SolidMaterialRole::Stone,
                    SolidMaterialRole::Dirt,
                    SolidMaterialRole::Snow,
                    SolidMaterialRole::Ice,
                    SolidMaterialRole::Gravel,
                    SolidMaterialRole::Metal,
                ],
            ),
            (
                V2EnvironmentSettings::Volcanic,
                444_211_238,
                FillMaterialRole::Lava,
                vec![
                    SolidMaterialRole::Bedrock,
                    SolidMaterialRole::Basalt,
                    SolidMaterialRole::Metal,
                ],
            ),
        ] {
            let v2_settings = settings(environment);
            let v1_settings =
                canonical_v1_settings(&v2_settings).expect("the Hills mapping should be valid");
            let candidate = procedural::build_hills_candidate_for_v2_parity(
                12,
                &v1_settings,
                seed,
                1,
                false,
                &palette(),
                &is_solid,
            )
            .expect("the fixed parity candidate should construct");
            assert!(candidate.valid, "{:?}", candidate.validation_notes);
            let expected_fingerprint = map_fingerprint(&candidate.map, &candidate.special_regions);
            let plan = candidate_to_plan(
                12,
                candidate,
                &palette(),
                MapViewHint::new((0.0, 48.0, 42.0), (0.0, 6.0, 0.0)),
            )
            .expect("the finalized candidate should convert");
            plan.volume
                .validate()
                .expect("the converted candidate should pass common V2 validation");

            let mut solids = BTreeSet::new();
            let mut fills = BTreeSet::new();
            for element in plan
                .volume
                .columns
                .values()
                .flat_map(|column| &column.elements)
            {
                match element {
                    VolumeElement::Solid(mass) => {
                        solids.insert(mass.material);
                    }
                    VolumeElement::Fill(fill) => {
                        fills.insert(fill.material);
                    }
                }
            }
            assert_eq!(fills, BTreeSet::from([expected_fill]));
            assert!(
                required_solids.iter().all(|role| solids.contains(role)),
                "{environment:?} omitted a required role: {solids:?}"
            );

            let rematerialized =
                voxelize(&plan.volume, &palette(), &is_solid).expect("the plan should materialize");
            assert_eq!(
                map_fingerprint(&rematerialized.map, &SpecialMovementRegions::new()),
                expected_fingerprint,
                "semantic conversion must round-trip every voxel"
            );
        }
    }

    #[test]
    fn column_conversion_coalesces_equal_material_runs_without_crossing_air() {
        let coord = HexCoord::ORIGIN;
        let mut column = Column::new();
        column.set(0, BEDROCK);
        for level in 1..4 {
            column.set(level, STONE);
        }
        for level in 4..7 {
            column.set(level, DIRT);
        }
        column.set(7, GRASS);
        column.set(10, METAL);

        let converted =
            convert_column(coord, &column, &palette()).expect("all materials should be known");
        assert_eq!(
            converted.elements,
            vec![
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(0, 1),
                    material: SolidMaterialRole::Bedrock,
                    cutaway_for: None,
                }),
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(1, 4),
                    material: SolidMaterialRole::Stone,
                    cutaway_for: None,
                }),
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(4, 7),
                    material: SolidMaterialRole::Dirt,
                    cutaway_for: None,
                }),
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(7, 8),
                    material: SolidMaterialRole::Grass,
                    cutaway_for: None,
                }),
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(10, 11),
                    material: SolidMaterialRole::Metal,
                    cutaway_for: None,
                }),
            ]
        );
    }

    #[test]
    fn unknown_and_ambiguous_substances_fail_conversion() {
        let coord = HexCoord::ORIGIN;
        let mut map = VoxelMap::new();
        let mut unknown = Column::new();
        unknown.set(0, SubstanceId(99));
        map.insert_column(coord, unknown);
        let error = convert_map(
            12,
            &map,
            BTreeMap::new(),
            &SpecialMovementRegions::new(),
            &palette(),
            MapViewHint::new((0.0, 10.0, 10.0), (0.0, 0.0, 0.0)),
        )
        .expect_err("an unknown substance cannot be represented losslessly");
        assert!(error.to_string().contains("has no V2 semantic role"));

        let mut ambiguous_palette = palette();
        ambiguous_palette.stone = BEDROCK;
        let mut map = VoxelMap::new();
        let mut ambiguous = Column::new();
        ambiguous.set(0, BEDROCK);
        map.insert_column(coord, ambiguous);
        let error = convert_map(
            12,
            &map,
            BTreeMap::new(),
            &SpecialMovementRegions::new(),
            &ambiguous_palette,
            MapViewHint::new((0.0, 10.0, 10.0), (0.0, 0.0, 0.0)),
        )
        .expect_err("an ambiguous substance cannot be represented losslessly");
        assert!(error.to_string().contains("multiple V2 semantic roles"));
    }

    #[test]
    fn materialized_anchors_remain_exact_tile_positions() {
        let generated = build(
            12,
            0.4,
            &settings(V2EnvironmentSettings::TemperateGrassland),
            1_592_598_566,
            &palette(),
            &is_solid,
        )
        .expect("the hero Hills seed should build");
        for id in [
            procedural::PARTY_START,
            procedural::HOSTILE_START,
            procedural::CONFLICT_CENTER,
            procedural::BRIDGE,
            procedural::ALTERNATE_CROSSING,
        ] {
            assert!(
                generated.anchors.get(&MapAnchorId::from(id)).is_some(),
                "missing exact V1 anchor {id}"
            );
        }
    }
}
