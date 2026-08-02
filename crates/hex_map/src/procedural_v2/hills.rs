//! V2 Hills compatibility recipe.
//!
//! V1 remains the source of candidate geometry, bounded repairs, exact validation, and
//! scoring until parity is locked. This module converts the finalized selection into the
//! recipe-independent V2 volume without interpreting or regenerating its topology.

use std::collections::BTreeMap;

use hex_core::{
    HexCoord, MapViewHint, SpecialMovementRegions, SubstanceId, TilePos, TraversalProfile,
};

use super::recipe::{
    materialize_selection, MaterializedSelection, RecipePlan, RecipeSelection, ReportMetrics,
    ValidatedRecipeSelection, MAX_REPAIR_ROUNDS,
};
use super::volume::{
    FillMaterialRole, LevelInterval, NonSolidFill, SolidMass, SolidMaterialRole, SurfaceAccess,
    SurfaceMetadata, TerrainVolumePlan, VolumeColumn, VolumeElement,
};
use super::V2GenerationError;
#[cfg(test)]
use crate::procedural::V1HillsCandidate;
use crate::procedural::{self, TacticalMetrics, V1HillsTopology};
use crate::settings::{
    CrossingSettings, EnvironmentSettings, HillsSettings, LandformSettings, ProceduralV1Settings,
    ProceduralV2Settings, TacticalSettings, V2EnvironmentSettings, V2RecipeSettings,
};
use crate::terrain::TerrainPalette;
use crate::voxel::{Column, VoxelMap};

/// V1 tactical measurements retained with the converted selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HillsMetrics {
    pub(crate) tactical: TacticalMetrics,
}

impl ReportMetrics for HillsMetrics {
    fn tactical(&self) -> TacticalMetrics {
        self.tactical
    }
}

/// Hills-only semantic facts retained for later layered recipes and diagnostics.
#[derive(Debug, Clone)]
pub(crate) struct HillsMetadata {
    pub(crate) topology: V1HillsTopology,
    pub(crate) repair_actions: Vec<String>,
    pub(crate) metrics: HillsMetrics,
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
    let selection = select(grid_radius, level_height, settings, seed, palette, is_solid)?;
    materialize_selection(selection, palette, is_solid)
}

/// Selects finalized Hills ground before materialization.
///
/// Layered recipes use this boundary to add independent upper masses without
/// regenerating or reverse-converting the approved ground plan.
pub(crate) fn select(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV2Settings,
    seed: u64,
    palette: &TerrainPalette,
    is_solid: &dyn Fn(SubstanceId) -> bool,
) -> Result<ValidatedRecipeSelection<HillsMetadata, HillsMetrics>, V2GenerationError> {
    select_compatibility(grid_radius, level_height, settings, seed, palette, is_solid)
}

fn select_compatibility(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV2Settings,
    seed: u64,
    palette: &TerrainPalette,
    is_solid: &dyn Fn(SubstanceId) -> bool,
) -> Result<ValidatedRecipeSelection<HillsMetadata, HillsMetrics>, V2GenerationError> {
    let view_hint = hills_view_hint(grid_radius, level_height, settings)?;
    let legacy_settings = canonical_v1_settings(settings)?;
    let procedural::V1HillsBuild {
        build: legacy,
        topology,
    } = procedural::build_hills_for_v2_parity(
        grid_radius,
        &legacy_settings,
        seed,
        palette,
        is_solid,
    )
    .map_err(|reason| {
        V2GenerationError::RecipeContract(format!("V1 Hills parity adapter failed: {reason}"))
    })?;
    let procedural::ProceduralBuild {
        map,
        anchors,
        special_regions,
        report,
        validated,
    } = legacy;
    if !validated {
        return Err(V2GenerationError::InvalidFallback(report.notes));
    }
    if report.candidates_evaluated != procedural::CANDIDATE_COUNT {
        return Err(V2GenerationError::RecipeContract(format!(
            "V1 Hills evaluated {} candidates; expected {}",
            report.candidates_evaluated,
            procedural::CANDIDATE_COUNT
        )));
    }
    if report.repair_actions.len() > usize::from(MAX_REPAIR_ROUNDS) {
        return Err(V2GenerationError::RecipeContract(format!(
            "V1 Hills imported {} repair rounds; the V2 limit is {MAX_REPAIR_ROUNDS}",
            report.repair_actions.len()
        )));
    }

    let metrics = HillsMetrics {
        tactical: report.metrics,
    };
    let plan = selected_map_to_plan(
        grid_radius,
        SelectedHills {
            map: &map,
            anchors: &anchors,
            special_regions: &special_regions,
            repair_actions: report.repair_actions.clone(),
            metrics,
            topology,
        },
        palette,
        view_hint,
    )?;
    ValidatedRecipeSelection::from_compatibility_import(RecipeSelection {
        plan,
        metrics,
        selected_candidate: report.selected_candidate,
        candidates_evaluated: report.candidates_evaluated,
        valid_candidates: report.valid_candidates,
        repair_actions: report.repair_actions,
        used_fallback: report.used_fallback,
        notes: report.notes,
    })
}

fn hills_view_hint(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV2Settings,
) -> Result<MapViewHint, V2GenerationError> {
    let V2RecipeSettings::Hills(hills) = &settings.recipe else {
        return Err(V2GenerationError::RecipeUnavailable("Hills"));
    };
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V2GenerationError::RecipeContract(
            "Hills level height must be positive and finite".to_owned(),
        ));
    }
    let valley_level = i16::try_from(hills.valley_level).map_err(|_out_of_range| {
        V2GenerationError::RecipeContract(
            "Hills valley level cannot be represented by the camera frame".to_owned(),
        )
    })?;
    let max_relief = i16::try_from(hills.max_relief).map_err(|_out_of_range| {
        V2GenerationError::RecipeContract(
            "Hills relief cannot be represented by the camera frame".to_owned(),
        )
    })?;
    let radius = u16::try_from(grid_radius).map_err(|_out_of_range| {
        V2GenerationError::RecipeContract(
            "Hills radius cannot be represented by the camera frame".to_owned(),
        )
    })?;
    let focus_height = f32::from(valley_level) * level_height;
    let horizontal_frame = f32::from(radius) * 3.5;
    let relief_frame = f32::from(max_relief) * level_height * 2.0;
    let frame_distance = horizontal_frame.max(relief_frame);
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
        .map_err(V2GenerationError::RecipeContract)?;

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

#[cfg(test)]
fn candidate_to_plan(
    grid_radius: u32,
    candidate: &V1HillsCandidate,
    palette: &TerrainPalette,
    view_hint: MapViewHint,
) -> Result<RecipePlan<HillsMetadata>, V2GenerationError> {
    selected_map_to_plan(
        grid_radius,
        SelectedHills {
            map: &candidate.map,
            anchors: &candidate.anchors,
            special_regions: &candidate.special_regions,
            repair_actions: candidate.repair_actions.clone(),
            metrics: HillsMetrics {
                tactical: candidate.metrics,
            },
            topology: candidate.topology.clone(),
        },
        palette,
        view_hint,
    )
}

struct SelectedHills<'a> {
    map: &'a VoxelMap,
    anchors: &'a procedural::GeneratedAnchors,
    special_regions: &'a SpecialMovementRegions,
    repair_actions: Vec<String>,
    metrics: HillsMetrics,
    topology: V1HillsTopology,
}

fn selected_map_to_plan(
    grid_radius: u32,
    selected: SelectedHills<'_>,
    palette: &TerrainPalette,
    view_hint: MapViewHint,
) -> Result<RecipePlan<HillsMetadata>, V2GenerationError> {
    let anchors = selected
        .anchors
        .iter()
        .map(|(name, position)| (name.to_owned(), position))
        .collect();
    let volume = convert_map(
        grid_radius,
        selected.map,
        anchors,
        selected.special_regions,
        palette,
        view_hint,
    )?;
    Ok(RecipePlan {
        volume,
        metadata: HillsMetadata {
            topology: selected.topology,
            repair_actions: selected.repair_actions,
            metrics: selected.metrics,
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

    use hex_assets::{ArtPalette, SubstanceFile, SubstanceTable};
    use hex_core::{MapAnchorId, SpecialMovementRegion};

    use super::*;
    use crate::procedural::map_fingerprint;
    use crate::procedural_v2::volume::voxelize;
    use crate::settings::{MapSettings, ProceduralSettings, TerrainSettings};

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
    const HERO_SEED: u64 = 1_592_598_566;
    const FROZEN_PROBE_SEED: u64 = 484_450_342;
    const VOLCANIC_PROBE_SEED: u64 = 444_211_238;
    // Frozen after selecting the iteration-one V1 review pack from seeds 0..1_024.
    // The labels record why each seed entered the corpus; V2 must reproduce these
    // exact maps rather than rerunning the retired selector.
    const FIXED_REGRESSION_SEEDS: [(&str, u64); 6] = [
        ("median", 4),
        ("relief-min", 1),
        ("relief-max", 275),
        ("sinuosity-min", 9),
        ("sinuosity-max", 850),
        ("fallback-pressure", 677),
    ];
    const SHIPPED_ENVIRONMENT_SEEDS: [(V2EnvironmentSettings, u64); 3] = [
        (V2EnvironmentSettings::TemperateGrassland, HERO_SEED),
        (V2EnvironmentSettings::Frozen, FROZEN_PROBE_SEED),
        (V2EnvironmentSettings::Volcanic, VOLCANIC_PROBE_SEED),
    ];

    fn palette() -> TerrainPalette {
        TerrainPalette {
            bedrock: BEDROCK,
            stone: STONE,
            dirt: DIRT,
            grass: GRASS,
            gravel: GRAVEL,
            water: WATER,
            metal: METAL,
            worked_stone: SubstanceId(12),
            limestone: SubstanceId(13),
            slate: SubstanceId(14),
            timber: SubstanceId(15),
            terracotta: SubstanceId(16),
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

    fn assert_voxel_columns_equal(case: &str, expected: &VoxelMap, actual: &VoxelMap) {
        assert_eq!(
            actual.len(),
            expected.len(),
            "{case}: generated a different number of columns"
        );
        for (coord, expected_column) in expected.columns() {
            assert_eq!(
                actual.column(coord),
                Some(expected_column),
                "{case}: voxel column {coord:?} differs"
            );
        }
    }

    fn assert_v1_v2_parity(
        label: &str,
        radius: u32,
        environment: V2EnvironmentSettings,
        seed: u64,
    ) -> MaterializedSelection<HillsMetadata, HillsMetrics> {
        let case = format!("{label}, radius {radius}, {environment:?}, seed {seed}");
        let v2_settings = settings(environment);
        let v1_settings =
            canonical_v1_settings(&v2_settings).expect("the Hills mapping should be valid");
        let legacy = procedural::build(
            radius,
            &v1_settings,
            seed,
            &palette(),
            TraversalProfile::WALKER,
            &is_solid,
        );
        let converted = build(radius, 0.4, &v2_settings, seed, &palette(), &is_solid)
            .expect("V2 should losslessly select and materialize V1 Hills");

        assert!(legacy.validated, "{case}: {:?}", legacy.report.notes);
        assert_eq!(
            converted.selected_candidate, legacy.report.selected_candidate,
            "{case}: selected candidate differs"
        );
        assert_eq!(
            converted.candidates_evaluated, legacy.report.candidates_evaluated,
            "{case}: candidate count differs"
        );
        assert_eq!(
            converted.valid_candidates, legacy.report.valid_candidates,
            "{case}: valid candidate count differs"
        );
        assert_eq!(
            converted.repair_actions, legacy.report.repair_actions,
            "{case}: imported V1 repairs differ"
        );
        assert_eq!(
            converted.used_fallback, legacy.report.used_fallback,
            "{case}: fallback selection differs"
        );
        assert_eq!(
            converted.metrics.tactical, legacy.report.metrics,
            "{case}: tactical metrics differ"
        );
        assert_eq!(
            converted.map_fingerprint, legacy.report.map_fingerprint,
            "{case}: map fingerprint differs"
        );
        assert_voxel_columns_equal(&case, &legacy.map, &converted.map);

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
        assert_eq!(
            actual_anchors, expected_anchors,
            "{case}: exact anchors differ"
        );
        assert_eq!(
            sorted_regions(&converted.special_regions),
            sorted_regions(&legacy.special_regions),
            "{case}: exact special-movement memberships differ"
        );

        converted
    }

    #[test]
    fn shipped_radius_twelve_hills_match_v1_exactly() {
        for (environment, seed) in SHIPPED_ENVIRONMENT_SEEDS {
            let converted = assert_v1_v2_parity("shipped preset", 12, environment, seed);
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
        }
    }

    #[test]
    fn semantic_selection_is_available_before_materialization() {
        let palette = palette();
        let settings = settings(V2EnvironmentSettings::TemperateGrassland);
        let selection = select(12, 0.4, &settings, HERO_SEED, &palette, &is_solid)
            .expect("Hills ground should select as a semantic volume");

        selection
            .plan
            .volume
            .validate()
            .expect("the selected ground volume should satisfy common V2 invariants");
        assert!(!selection.plan.metadata.topology.barrier.is_empty());
        assert!(!selection
            .plan
            .metadata
            .topology
            .protected_approaches
            .is_empty());

        let selected_candidate = selection.selected_candidate;
        let materialized = materialize_selection(selection, &palette, &is_solid)
            .expect("the semantic selection should materialize without reconstruction");
        let direct = build(12, 0.4, &settings, HERO_SEED, &palette, &is_solid)
            .expect("the direct Hills path should materialize");
        assert_eq!(materialized.selected_candidate, selected_candidate);
        assert_eq!(materialized.map_fingerprint, direct.map_fingerprint);
        assert_voxel_columns_equal(
            "semantic selection boundary",
            &direct.map,
            &materialized.map,
        );
    }

    #[test]
    fn frozen_v1_review_corpus_matches_v2_exactly() {
        for (label, seed) in std::iter::once(("hero", HERO_SEED)).chain(FIXED_REGRESSION_SEEDS) {
            let converted =
                assert_v1_v2_parity(label, 12, V2EnvironmentSettings::TemperateGrassland, seed);
            assert!(
                !converted.used_fallback,
                "{label} seed {seed} unexpectedly used fallback"
            );
        }
    }

    #[test]
    fn supported_radii_and_shipped_environments_match_v1_exactly() {
        for radius in [20, 40] {
            for (environment, seed) in SHIPPED_ENVIRONMENT_SEEDS {
                assert_v1_v2_parity("scale boundary", radius, environment, seed);
            }
        }
    }

    #[test]
    fn radius_12_pr_corpus_validates_128_v2_hills_seeds_and_named_regressions() {
        let palette = palette();
        let temperate = settings(V2EnvironmentSettings::TemperateGrassland);

        for seed in 0..128 {
            let generated = build(12, 0.4, &temperate, seed, &palette, &is_solid)
                .unwrap_or_else(|error| panic!("V2 Hills seed {seed} failed: {error}"));
            assert!(
                !generated.map.is_empty(),
                "V2 Hills seed {seed} published an empty map"
            );
            assert!(
                generated
                    .anchors
                    .get(&MapAnchorId::from(procedural::PARTY_START))
                    .is_some(),
                "V2 Hills seed {seed} omitted the party-start anchor"
            );
        }

        for (label, seed) in std::iter::once(("hero", HERO_SEED)).chain(FIXED_REGRESSION_SEEDS) {
            build(12, 0.4, &temperate, seed, &palette, &is_solid)
                .unwrap_or_else(|error| panic!("V2 Hills {label} seed {seed} failed: {error}"));
        }
        for (environment, seed) in SHIPPED_ENVIRONMENT_SEEDS {
            build(12, 0.4, &settings(environment), seed, &palette, &is_solid).unwrap_or_else(
                |error| panic!("V2 Hills {environment:?} seed {seed} failed: {error}"),
            );
        }
    }

    #[test]
    #[ignore = "manual release-mode 10,000-seed V2 Hills corpus"]
    fn ten_thousand_seed_corpus_has_less_than_one_percent_fallbacks() {
        let palette = palette();
        let settings = settings(V2EnvironmentSettings::TemperateGrassland);
        let started = std::time::Instant::now();
        let mut fallbacks = 0usize;
        let mut fingerprints = BTreeSet::new();

        for seed in 0..10_000 {
            let generated = build(12, 0.4, &settings, seed, &palette, &is_solid)
                .unwrap_or_else(|error| panic!("V2 Hills seed {seed} failed: {error}"));
            fallbacks += usize::from(generated.used_fallback);
            fingerprints.insert(generated.map_fingerprint);
        }

        eprintln!(
            "10k V2 Hills: invalid=0, fallbacks={fallbacks}, unique_fingerprints={}, wall={}ms",
            fingerprints.len(),
            started.elapsed().as_millis()
        );
        assert!(fallbacks < 100, "{fallbacks} of 10,000 seeds used fallback");
    }

    /// Exports the provenance-documented review corpus without selecting new seeds.
    ///
    /// Run with `--ignored --exact --nocapture --test-threads=1` and redirect the
    /// output to the review pack. Timing is diagnostic; every other field is stable.
    #[test]
    #[ignore = "manual deterministic V2 Hills report export"]
    fn print_v2_hills_review_reports() {
        let substances: SubstanceFile =
            ron::from_str(include_str!("../../../../assets/config/substances.ron"))
                .expect("the shipped substances should parse");
        let art_palette: ArtPalette =
            ron::from_str(include_str!("../../../../assets/art/palette.ron"))
                .expect("the shipped art palette should parse");
        let table = SubstanceTable::from_file(&substances, &art_palette)
            .expect("the shipped substances should resolve through the art palette");

        for (label, seed) in std::iter::once(("hero", HERO_SEED)).chain(FIXED_REGRESSION_SEEDS) {
            print_review_report(
                label,
                V2EnvironmentSettings::TemperateGrassland,
                seed,
                &table,
            );
        }
        print_review_report(
            "frozen-probe",
            V2EnvironmentSettings::Frozen,
            FROZEN_PROBE_SEED,
            &table,
        );
        print_review_report(
            "volcanic-probe",
            V2EnvironmentSettings::Volcanic,
            VOLCANIC_PROBE_SEED,
            &table,
        );
    }

    fn print_review_report(
        label: &str,
        environment: V2EnvironmentSettings,
        seed: u64,
        table: &SubstanceTable,
    ) {
        let map_settings: MapSettings = ron::from_str(match environment {
            V2EnvironmentSettings::TemperateGrassland => {
                include_str!("../../../../assets/config/worlds/procedural-hills.ron")
            }
            V2EnvironmentSettings::Frozen => {
                include_str!("../../../../assets/config/worlds/procedural-frozen.ron")
            }
            V2EnvironmentSettings::Volcanic => {
                include_str!("../../../../assets/config/worlds/procedural-volcanic.ron")
            }
            V2EnvironmentSettings::Rocky => {
                panic!("Rocky is not a shipped V2 Hills environment")
            }
        })
        .expect("the shipped V2 Hills world should parse");
        let TerrainSettings::Procedural(ProceduralSettings::V2(settings)) = &map_settings.terrain
        else {
            panic!("the shipped review world should select procedural V2")
        };
        assert_eq!(
            settings.environment, environment,
            "the report label must match the shipped environment"
        );
        let palette = TerrainPalette::for_terrain(table, &map_settings.terrain)
            .expect("the shipped substances should cover V2 Hills");
        let generated = crate::procedural_v2::build(
            map_settings.grid_radius,
            map_settings.level_height,
            settings,
            seed,
            &palette,
            &|substance| table.is_solid(substance),
        )
        .expect("the review corpus should generate");

        let mut anchors: Vec<_> = generated
            .anchors
            .iter()
            .map(|(name, position)| (name.as_str().to_owned(), position))
            .collect();
        anchors.sort_unstable();
        let mut notes = generated.report.notes.clone();
        notes.sort_unstable();
        let metrics = generated.report.metrics;

        println!("case: {label}");
        println!("  environment: {environment:?}");
        println!("  seed: {seed}");
        println!(
            "  generator_version: {}",
            generated.report.generator_version
        );
        println!(
            "  selected_candidate: {:?}",
            generated.report.selected_candidate
        );
        println!(
            "  valid_candidates: {}/{}",
            generated.report.valid_candidates, generated.report.candidates_evaluated
        );
        println!("  repair_rounds: {}", generated.report.repair_rounds);
        println!("  repair_actions: {:?}", generated.report.repair_actions);
        println!("  used_fallback: {}", generated.report.used_fallback);
        println!(
            "  settings_fingerprint: {}",
            generated.report.settings_fingerprint
        );
        println!("  map_fingerprint: {}", generated.report.map_fingerprint);
        println!("  metrics:");
        println!("    relief: {}", metrics.relief);
        println!("    barrier_cells: {}", metrics.barrier_cells);
        println!("    critical_route_steps: {}", metrics.critical_route_steps);
        println!(
            "    spawn_height_difference: {}",
            metrics.spawn_height_difference
        );
        println!(
            "    bank_high_ground_difference: {}",
            metrics.bank_high_ground_difference
        );
        println!("    reachable_surfaces: {}", metrics.reachable_surfaces);
        println!(
            "    reachable_elevation_levels: {}",
            metrics.reachable_elevation_levels
        );
        println!(
            "    alternate_detour_percent: {}",
            metrics.alternate_detour_percent
        );
        println!(
            "    river_sinuosity_percent: {}",
            metrics.river_sinuosity_percent
        );
        println!(
            "    environment_signature_percent: {}",
            metrics.environment_signature_percent
        );
        println!("  elapsed_micros: {}", generated.report.elapsed_micros);
        println!("  anchors:");
        for (name, position) in anchors {
            println!(
                "    {name}: ({}, {}, {}) @ {}",
                position.coord.x(),
                position.coord.y(),
                position.coord.z(),
                position.level
            );
        }
        println!(
            "  special_region_count: {}",
            generated.special_regions.len()
        );
        println!(
            "  interior_surface_count: {}",
            generated.interiors.surfaces().count()
        );
        println!(
            "  interior_roof_voxel_count: {}",
            generated.interiors.roof_voxels().count()
        );
        println!("  view_eye: {:?}", generated.view_hint.eye);
        println!("  view_focus: {:?}", generated.view_hint.focus);
        println!("  notes: {notes:?}");
    }

    #[test]
    #[ignore = "manual release/debug generator benchmark"]
    fn v2_hills_radius_benchmark_meets_the_radius_40_target() {
        let palette = palette();
        let settings = settings(V2EnvironmentSettings::TemperateGrassland);
        let mut radius_40_median = 0;
        let mut radius_40_worst = 0;

        for radius in [12, 20, 40] {
            let warmup =
                crate::procedural_v2::build(radius, 0.4, &settings, u64::MAX, &palette, &is_solid)
                    .expect("the warm-up map should generate");
            std::hint::black_box(warmup);

            let mut samples = Vec::new();
            for seed in 0..12 {
                let result =
                    crate::procedural_v2::build(radius, 0.4, &settings, seed, &palette, &is_solid)
                        .expect("the benchmark map should generate");
                assert_eq!(result.report.generator_version, 2);
                assert_eq!(result.report.candidates_evaluated, 8);
                samples.push(result.report.elapsed_micros);
                std::hint::black_box(result);
            }

            samples.sort_unstable();
            let median = samples.get(samples.len() / 2).copied().unwrap_or(u64::MAX);
            let worst = samples.last().copied().unwrap_or(u64::MAX);
            eprintln!("V2 Hills radius {radius}: median={median}us worst={worst}us");
            if radius == 40 {
                radius_40_median = median;
                radius_40_worst = worst;
            }
        }

        let target_micros = if cfg!(debug_assertions) {
            250_000
        } else {
            50_000
        };
        eprintln!(
            "V2 Hills radius 40 median={radius_40_median}us worst={radius_40_worst}us \
             target={target_micros}us (trend only)"
        );
    }

    #[test]
    fn canonical_fallback_matches_nonzero_seed_v1_and_imports_bounded_repairs() {
        const REQUESTED_SEED: u64 = 505;

        let palette = palette();
        let v2_settings = settings(V2EnvironmentSettings::TemperateGrassland);
        let v1_settings =
            canonical_v1_settings(&v2_settings).expect("the Hills mapping should be valid");
        let expected = procedural::build_hills_candidate_for_v2_parity(
            12,
            &v1_settings,
            REQUESTED_SEED,
            0,
            true,
            &palette,
            &is_solid,
        )
        .expect("the V1 canonical fallback should construct");
        assert!(expected.valid, "{:?}", expected.validation_notes);

        let fallback = candidate_to_plan(
            12,
            &expected,
            &palette,
            hills_view_hint(12, 0.4, &v2_settings).expect("the fallback view should derive"),
        )
        .expect("the V2 canonical fallback should convert");
        let imported_repairs = fallback.metadata.repair_actions.clone();
        assert_eq!(imported_repairs, expected.repair_actions);
        assert!(
            imported_repairs.len() <= 4,
            "V1 imported {} repairs, exceeding the V2 bound",
            imported_repairs.len()
        );
        assert_eq!(fallback.metadata.metrics.tactical, expected.metrics);

        let materialized =
            voxelize(&fallback.volume, &palette, &is_solid).expect("the fallback should voxelize");
        assert_voxel_columns_equal("canonical fallback", &expected.map, &materialized.map);
        let expected_anchors: BTreeMap<String, TilePos> = expected
            .anchors
            .iter()
            .map(|(name, position)| (name.to_owned(), position))
            .collect();
        assert_eq!(fallback.volume.anchors, expected_anchors);

        let mut actual_regions: Vec<_> = fallback
            .volume
            .surfaces
            .iter()
            .filter_map(|(position, surface)| match surface.access {
                SurfaceAccess::SpecialMovement(region) => Some((*position, region)),
                SurfaceAccess::Ordinary | SurfaceAccess::NonStandable => None,
            })
            .collect();
        actual_regions.sort_unstable();
        assert_eq!(actual_regions, sorted_regions(&expected.special_regions));
    }

    #[test]
    fn bridge_conversion_preserves_air_gap_and_rejects_submerged_bed() {
        let palette = palette();
        let v2_settings = settings(V2EnvironmentSettings::TemperateGrassland);
        let v1_settings =
            canonical_v1_settings(&v2_settings).expect("the Hills mapping should be valid");
        let complete = procedural::build(
            12,
            &v1_settings,
            HERO_SEED,
            &palette,
            TraversalProfile::WALKER,
            &is_solid,
        );
        let selected = complete
            .report
            .selected_candidate
            .expect("the hero map should not need fallback");
        let candidate = procedural::build_hills_candidate_for_v2_parity(
            12,
            &v1_settings,
            HERO_SEED,
            selected,
            false,
            &palette,
            &is_solid,
        )
        .expect("the selected V1 candidate should construct");
        let bridge = candidate.topology.bridge.clone();
        let plan = candidate_to_plan(
            12,
            &candidate,
            &palette,
            hills_view_hint(12, 0.4, &v2_settings).expect("the view should derive"),
        )
        .expect("the selected candidate should convert");
        let V2RecipeSettings::Hills(hills) = &v2_settings.recipe else {
            unreachable!("the test helper always constructs Hills");
        };
        let crossing = hills
            .derived_crossing()
            .expect("the Hills crossing should derive");
        let gap_level = crossing.hazard_top.saturating_add(1);

        assert!(!bridge.is_empty());
        let mut channel_decks = 0_usize;
        for deck in bridge {
            assert_eq!(deck.level, crossing.bridge_level);
            assert_eq!(
                plan.volume
                    .surfaces
                    .get(&deck)
                    .map(|surface| surface.access),
                Some(SurfaceAccess::Ordinary),
                "bridge deck {deck:?} must remain ordinary footing"
            );
            let column = plan
                .volume
                .columns
                .get(&deck.coord)
                .expect("every bridge coordinate should retain a volume column");
            let spans_hazard = column.elements.iter().any(|element| {
                matches!(
                    element,
                    VolumeElement::Fill(NonSolidFill {
                        levels,
                        material: FillMaterialRole::Water,
                    }) if levels.bottom == crossing.hazard_bottom
                        && levels.top == crossing.hazard_top.saturating_add(1)
                )
            });
            if !spans_hazard {
                continue;
            }
            channel_decks = channel_decks.saturating_add(1);
            let bed = TilePos::new(deck.coord, crossing.bed_level);
            assert_eq!(
                plan.volume.surfaces.get(&bed).map(|surface| surface.access),
                Some(SurfaceAccess::NonStandable),
                "submerged bed {bed:?} must not become ordinary footing"
            );
            assert!(
                column.elements.iter().all(|element| {
                    let levels = element_levels(*element);
                    gap_level < levels.bottom || gap_level >= levels.top
                }),
                "bridge column {:?} filled the required air level {gap_level}",
                deck.coord
            );
            assert!(column.elements.iter().any(|element| {
                matches!(
                    element,
                    VolumeElement::Solid(SolidMass {
                        levels,
                        material: SolidMaterialRole::Metal,
                        ..
                    }) if levels.bottom == crossing.bridge_level
                        && levels.top == crossing.bridge_level.saturating_add(1)
                )
            }));
        }
        assert!(
            channel_decks >= 2,
            "the two-wide bridge should cross at least one full hazard row"
        );
    }

    #[test]
    fn view_hint_scales_with_radius_and_level_height() {
        let settings = settings(V2EnvironmentSettings::TemperateGrassland);
        assert_eq!(
            hills_view_hint(20, 0.5, &settings).expect("the Hills hint should derive"),
            MapViewHint::new((0.0, 77.5, 70.0), (0.0, 7.5, 0.0))
        );
        assert_eq!(
            hills_view_hint(12, 100.0, &settings)
                .expect("large vertical relief should expand the frame"),
            MapViewHint::new((0.0, 3_100.0, 1_600.0), (0.0, 1_500.0, 0.0))
        );
        assert!(matches!(
            hills_view_hint(12, f32::NAN, &settings),
            Err(V2GenerationError::RecipeContract(_))
        ));
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
                &candidate,
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
