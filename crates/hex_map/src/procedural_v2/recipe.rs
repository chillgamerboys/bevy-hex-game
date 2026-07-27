//! Recipe-independent candidate selection for procedural generator V2.

use std::fmt::Debug;

use hex_core::{
    InteriorRegions, MapAnchorId, MapAnchors, MapViewHint, SpecialMovementRegions, SubstanceId,
};
use xxhash_rust::xxh3::xxh3_64;

use super::seed::SeedStreams;
use super::volume::{voxelize, SurfaceAccess, TerrainVolumePlan, VoxelizedTerrain};
use super::V2GenerationError;
use crate::terrain::TerrainPalette;
use crate::voxel::VoxelMap;

pub(crate) const CANDIDATE_COUNT: u8 = 8;
pub(crate) const MAX_REPAIR_ROUNDS: u8 = 4;

/// Stable inputs available while constructing or repairing one candidate.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CandidateContext {
    pub(crate) grid_radius: u32,
    pub(crate) seed: u64,
    pub(crate) candidate: u8,
    pub(crate) streams: SeedStreams,
}

impl CandidateContext {
    #[must_use]
    fn new(grid_radius: u32, seed: u64, candidate: u8) -> Self {
        Self {
            grid_radius,
            seed,
            candidate,
            streams: SeedStreams::new(seed, candidate),
        }
    }
}

/// Stable inputs available while constructing a separately authored fallback.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FallbackContext {
    pub(crate) grid_radius: u32,
    pub(crate) seed: u64,
    pub(crate) streams: SeedStreams,
}

impl FallbackContext {
    #[must_use]
    fn new(grid_radius: u32, seed: u64) -> Self {
        Self {
            grid_radius,
            seed,
            streams: SeedStreams::new(seed, 0),
        }
    }
}

/// One recipe's semantic plan and topology-specific metadata.
#[derive(Debug, Clone)]
pub(crate) struct RecipePlan<M> {
    pub(crate) volume: TerrainVolumePlan,
    pub(crate) metadata: M,
}

/// Result of one recipe-specific validation pass.
#[derive(Debug, Clone)]
pub(crate) struct RecipeValidation<M> {
    pub(crate) issues: Vec<String>,
    pub(crate) metrics: M,
}

impl<M> RecipeValidation<M> {
    #[must_use]
    pub(crate) fn valid(metrics: M) -> Self {
        Self {
            issues: Vec::new(),
            metrics,
        }
    }

    #[must_use]
    pub(crate) fn invalid(metrics: M, issues: Vec<String>) -> Self {
        Self { issues, metrics }
    }
}

/// Whether a bounded repair changed semantic intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RepairOutcome {
    Changed(String),
    NoChange,
}

/// Complete recipe contract used by the common V2 orchestrator.
///
/// Metadata, metrics, and scores remain typed per recipe. Dynamic dispatch is
/// deliberately avoided so adding one recipe cannot weaken another recipe's
/// validator or force unrelated metrics into a shared shape.
pub(crate) trait V2Recipe {
    type Settings;
    type Metadata: Clone + Debug;
    type Metrics: Clone + Debug;
    type Score: Ord;

    fn construct(
        &self,
        context: CandidateContext,
        settings: &Self::Settings,
    ) -> Result<RecipePlan<Self::Metadata>, V2GenerationError>;

    fn validate(
        &self,
        context: CandidateContext,
        settings: &Self::Settings,
        plan: &RecipePlan<Self::Metadata>,
    ) -> RecipeValidation<Self::Metrics>;

    fn repair(
        &self,
        context: CandidateContext,
        settings: &Self::Settings,
        plan: &mut RecipePlan<Self::Metadata>,
        round: u8,
        issues: &[String],
    ) -> Result<RepairOutcome, V2GenerationError>;

    fn score(
        &self,
        settings: &Self::Settings,
        metrics: &Self::Metrics,
        candidate: u8,
    ) -> Self::Score;

    fn canonical_fallback(
        &self,
        context: FallbackContext,
        settings: &Self::Settings,
    ) -> Result<RecipePlan<Self::Metadata>, V2GenerationError>;
}

/// Selected hard-valid semantic result, before material resolution.
#[derive(Debug)]
pub(crate) struct RecipeSelection<M, V> {
    pub(crate) plan: RecipePlan<M>,
    pub(crate) metrics: V,
    pub(crate) selected_candidate: Option<u8>,
    pub(crate) candidates_evaluated: u8,
    pub(crate) valid_candidates: u8,
    pub(crate) repair_actions: Vec<String>,
    pub(crate) used_fallback: bool,
    pub(crate) notes: Vec<String>,
}

/// Fully materialized result of one selected recipe candidate.
///
/// Selection remains semantic until this boundary. Exact resources are derived from
/// the final validated plan so repairs cannot leave stale anchors, region memberships,
/// interior metadata, or framing behind.
#[derive(Debug)]
pub(crate) struct MaterializedSelection<M, V> {
    pub(crate) map: VoxelMap,
    pub(crate) anchors: MapAnchors,
    pub(crate) special_regions: SpecialMovementRegions,
    pub(crate) interiors: InteriorRegions,
    pub(crate) view_hint: MapViewHint,
    pub(crate) metadata: M,
    pub(crate) metrics: V,
    pub(crate) selected_candidate: Option<u8>,
    pub(crate) candidates_evaluated: u8,
    pub(crate) valid_candidates: u8,
    pub(crate) repair_actions: Vec<String>,
    pub(crate) used_fallback: bool,
    pub(crate) notes: Vec<String>,
    pub(crate) map_fingerprint: u64,
}

/// Resolves one selected semantic plan into voxels and exact generated resources.
pub(crate) fn materialize_selection<M, V>(
    selection: RecipeSelection<M, V>,
    palette: &TerrainPalette,
    is_solid: &dyn Fn(SubstanceId) -> bool,
) -> Result<MaterializedSelection<M, V>, V2GenerationError> {
    let RecipeSelection {
        plan,
        metrics,
        selected_candidate,
        candidates_evaluated,
        valid_candidates,
        repair_actions,
        used_fallback,
        notes,
    } = selection;
    let RecipePlan { volume, metadata } = plan;

    let anchors = volume
        .anchors
        .iter()
        .map(|(name, position)| (MapAnchorId::from(name.clone()), *position))
        .collect();
    let special_regions = volume
        .surfaces
        .iter()
        .filter_map(|(position, surface)| match surface.access {
            SurfaceAccess::SpecialMovement(region) => Some((*position, region)),
            SurfaceAccess::Ordinary | SurfaceAccess::NonStandable => None,
        })
        .collect();
    let view_hint = volume.view_hint;
    let VoxelizedTerrain { map, interiors } = voxelize(&volume, palette, is_solid)?;
    let map_fingerprint = materialized_map_fingerprint(&map, &special_regions, &interiors);

    Ok(MaterializedSelection {
        map,
        anchors,
        special_regions,
        interiors,
        view_hint,
        metadata,
        metrics,
        selected_candidate,
        candidates_evaluated,
        valid_candidates,
        repair_actions,
        used_fallback,
        notes,
        map_fingerprint,
    })
}

/// Extends the frozen V1 identity only when a map has exact interior semantics.
fn materialized_map_fingerprint(
    map: &VoxelMap,
    special_regions: &SpecialMovementRegions,
    interiors: &InteriorRegions,
) -> u64 {
    let v1_fingerprint = crate::procedural::map_fingerprint(map, special_regions);
    if interiors.is_empty() {
        return v1_fingerprint;
    }

    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"procedural-v2-interiors-v1");
    bytes.extend_from_slice(&v1_fingerprint.to_le_bytes());

    let mut floors: Vec<_> = interiors.surfaces().collect();
    floors.sort_unstable();
    bytes.extend_from_slice(b"floors");
    bytes.extend_from_slice(
        &u64::try_from(floors.len())
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    for (position, region) in floors {
        append_interior_membership(&mut bytes, position, region.0);
    }

    let mut roofs: Vec<_> = interiors.roof_voxels().collect();
    roofs.sort_unstable();
    bytes.extend_from_slice(b"roofs");
    bytes.extend_from_slice(&u64::try_from(roofs.len()).unwrap_or(u64::MAX).to_le_bytes());
    for (position, region) in roofs {
        append_interior_membership(&mut bytes, position, region.0);
    }

    xxh3_64(&bytes)
}

fn append_interior_membership(bytes: &mut Vec<u8>, position: hex_core::TilePos, region: u32) {
    bytes.extend_from_slice(&position.coord.x().to_le_bytes());
    bytes.extend_from_slice(&position.coord.y().to_le_bytes());
    bytes.extend_from_slice(&position.level.to_le_bytes());
    bytes.extend_from_slice(&region.to_le_bytes());
}

#[derive(Debug)]
struct ValidCandidate<M, V, S> {
    plan: RecipePlan<M>,
    metrics: V,
    candidate: u8,
    repair_actions: Vec<String>,
    score: S,
}

/// Evaluates exactly eight candidates, applies at most four semantic repair rounds to
/// each, and validates a separately constructed canonical fallback when all fail.
pub(crate) fn run_recipe<R>(
    recipe: &R,
    settings: &R::Settings,
    grid_radius: u32,
    seed: u64,
) -> Result<RecipeSelection<R::Metadata, R::Metrics>, V2GenerationError>
where
    R: V2Recipe,
{
    let mut valid = Vec::new();
    let mut rejected_notes = Vec::new();

    for candidate in 0..CANDIDATE_COUNT {
        let context = CandidateContext::new(grid_radius, seed, candidate);
        let mut plan = match recipe.construct(context, settings) {
            Ok(plan) => plan,
            Err(error) => {
                rejected_notes.push(format!(
                    "candidate {candidate}: construction failed: {error}"
                ));
                continue;
            }
        };
        let (validation, repair_actions) =
            validate_and_repair(recipe, settings, context, &mut plan)?;
        if validation.issues.is_empty() {
            let score = recipe.score(settings, &validation.metrics, candidate);
            valid.push(ValidCandidate {
                plan,
                metrics: validation.metrics,
                candidate,
                repair_actions,
                score,
            });
        } else {
            rejected_notes.push(format!(
                "candidate {candidate}: {}",
                validation.issues.join("; ")
            ));
        }
    }

    let valid_candidates = u8::try_from(valid.len()).unwrap_or(u8::MAX);
    if let Some(selected) = valid.into_iter().min_by(|left, right| {
        left.score
            .cmp(&right.score)
            .then_with(|| left.candidate.cmp(&right.candidate))
    }) {
        return Ok(RecipeSelection {
            plan: selected.plan,
            metrics: selected.metrics,
            selected_candidate: Some(selected.candidate),
            candidates_evaluated: CANDIDATE_COUNT,
            valid_candidates,
            repair_actions: selected.repair_actions,
            used_fallback: false,
            notes: Vec::new(),
        });
    }

    let fallback_context = FallbackContext::new(grid_radius, seed);
    let fallback = recipe.canonical_fallback(fallback_context, settings)?;
    let validation = validate_plan(
        recipe,
        settings,
        CandidateContext::new(grid_radius, seed, 0),
        &fallback,
    );
    if !validation.issues.is_empty() {
        return Err(V2GenerationError::InvalidFallback(validation.issues));
    }
    rejected_notes.push("all random candidates failed; canonical fallback selected".to_owned());

    Ok(RecipeSelection {
        plan: fallback,
        metrics: validation.metrics,
        selected_candidate: None,
        candidates_evaluated: CANDIDATE_COUNT,
        valid_candidates: 0,
        repair_actions: Vec::new(),
        used_fallback: true,
        notes: rejected_notes,
    })
}

fn validate_and_repair<R>(
    recipe: &R,
    settings: &R::Settings,
    context: CandidateContext,
    plan: &mut RecipePlan<R::Metadata>,
) -> Result<(RecipeValidation<R::Metrics>, Vec<String>), V2GenerationError>
where
    R: V2Recipe,
{
    let mut repair_actions = Vec::new();
    let mut validation = validate_plan(recipe, settings, context, plan);
    for round in 0..MAX_REPAIR_ROUNDS {
        if validation.issues.is_empty() {
            break;
        }
        match recipe.repair(context, settings, plan, round, validation.issues.as_slice())? {
            RepairOutcome::Changed(action) => repair_actions.push(action),
            RepairOutcome::NoChange => break,
        }
        validation = validate_plan(recipe, settings, context, plan);
    }
    Ok((validation, repair_actions))
}

fn validate_plan<R>(
    recipe: &R,
    settings: &R::Settings,
    context: CandidateContext,
    plan: &RecipePlan<R::Metadata>,
) -> RecipeValidation<R::Metrics>
where
    R: V2Recipe,
{
    let mut recipe_validation = recipe.validate(context, settings, plan);
    if let Err(error) = plan.volume.validate() {
        match error {
            V2GenerationError::InvalidVolume(mut issues) => {
                issues.append(&mut recipe_validation.issues);
                recipe_validation.issues = issues;
            }
            other => recipe_validation.issues.insert(0, other.to_string()),
        }
    }
    recipe_validation
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::BTreeMap;

    use hex_core::{HexCoord, InteriorRegionId, MapViewHint, TilePos};

    use super::*;
    use crate::procedural_v2::volume::{
        LevelInterval, SolidMass, SolidMaterialRole, SurfaceAccess, SurfaceMetadata, VolumeColumn,
        VolumeElement,
    };
    use crate::terrain::TerrainPalette;

    #[derive(Debug, Clone, Copy)]
    struct MockSettings {
        force_fallback: bool,
        invalid_fallback: bool,
    }

    #[derive(Debug, Clone, Copy)]
    struct MockMetadata {
        candidate: u8,
        repairs: u8,
        fallback: bool,
    }

    #[derive(Default)]
    struct MockRecipe {
        constructions: Cell<u8>,
        repairs: Cell<u8>,
    }

    impl V2Recipe for MockRecipe {
        type Settings = MockSettings;
        type Metadata = MockMetadata;
        type Metrics = u8;
        type Score = (u8, u8);

        fn construct(
            &self,
            context: CandidateContext,
            _settings: &Self::Settings,
        ) -> Result<RecipePlan<Self::Metadata>, V2GenerationError> {
            self.constructions
                .set(self.constructions.get().saturating_add(1));
            Ok(mock_plan(
                context.grid_radius,
                MockMetadata {
                    candidate: context.candidate,
                    repairs: 0,
                    fallback: false,
                },
            ))
        }

        fn validate(
            &self,
            _context: CandidateContext,
            settings: &Self::Settings,
            plan: &RecipePlan<Self::Metadata>,
        ) -> RecipeValidation<Self::Metrics> {
            let required_repairs = if settings.force_fallback {
                u8::MAX
            } else if plan.metadata.candidate == 0 {
                5
            } else if plan.metadata.candidate == 1 {
                1
            } else {
                0
            };
            if plan.metadata.fallback && settings.invalid_fallback {
                return RecipeValidation::invalid(0, vec!["fallback topology failed".to_owned()]);
            }
            if plan.metadata.repairs < required_repairs && !plan.metadata.fallback {
                RecipeValidation::invalid(
                    plan.metadata.candidate,
                    vec!["candidate needs repair".to_owned()],
                )
            } else {
                RecipeValidation::valid(plan.metadata.candidate)
            }
        }

        fn repair(
            &self,
            _context: CandidateContext,
            _settings: &Self::Settings,
            plan: &mut RecipePlan<Self::Metadata>,
            round: u8,
            _issues: &[String],
        ) -> Result<RepairOutcome, V2GenerationError> {
            self.repairs.set(self.repairs.get().saturating_add(1));
            plan.metadata.repairs = plan.metadata.repairs.saturating_add(1);
            Ok(RepairOutcome::Changed(format!("repair round {round}")))
        }

        fn score(
            &self,
            _settings: &Self::Settings,
            _metrics: &Self::Metrics,
            candidate: u8,
        ) -> Self::Score {
            ((candidate > 1).into(), candidate)
        }

        fn canonical_fallback(
            &self,
            context: FallbackContext,
            _settings: &Self::Settings,
        ) -> Result<RecipePlan<Self::Metadata>, V2GenerationError> {
            Ok(mock_plan(
                context.grid_radius,
                MockMetadata {
                    candidate: 0,
                    repairs: 0,
                    fallback: true,
                },
            ))
        }
    }

    fn mock_plan(radius: u32, metadata: MockMetadata) -> RecipePlan<MockMetadata> {
        let coord = HexCoord::ORIGIN;
        let surface = TilePos::new(coord, 4);
        let mut columns: BTreeMap<HexCoord, VolumeColumn> = coord
            .within_radius(radius)
            .into_iter()
            .map(|coord| (coord, VolumeColumn::default()))
            .collect();
        columns.insert(
            coord,
            VolumeColumn {
                elements: vec![VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(0, 5),
                    material: SolidMaterialRole::Stone,
                    cutaway_for: None,
                })],
            },
        );
        RecipePlan {
            volume: TerrainVolumePlan {
                grid_radius: radius,
                columns,
                surfaces: BTreeMap::from([(
                    surface,
                    SurfaceMetadata {
                        access: SurfaceAccess::Ordinary,
                        interior: None,
                    },
                )]),
                anchors: BTreeMap::from([("party_start".to_owned(), surface)]),
                interiors: BTreeMap::new(),
                view_hint: MapViewHint::new((0.0, 10.0, 10.0), (0.0, 0.0, 0.0)),
            },
            metadata,
        }
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
            snow: SubstanceId(8),
            ice: SubstanceId(9),
            basalt: SubstanceId(10),
            lava: SubstanceId(11),
        }
    }

    fn test_is_solid(substance: SubstanceId) -> bool {
        matches!(substance.0, 1 | 2 | 3 | 4 | 5 | 7 | 8 | 9 | 10)
    }

    #[test]
    fn evaluates_exactly_eight_candidates_and_caps_each_repair_sequence() {
        let recipe = MockRecipe::default();
        let selection = run_recipe(
            &recipe,
            &MockSettings {
                force_fallback: false,
                invalid_fallback: false,
            },
            12,
            77,
        )
        .expect("at least one deterministic candidate should pass");

        assert_eq!(recipe.constructions.get(), CANDIDATE_COUNT);
        assert_eq!(selection.candidates_evaluated, CANDIDATE_COUNT);
        assert_eq!(selection.selected_candidate, Some(1));
        assert_eq!(selection.repair_actions.len(), 1);
        assert!(
            recipe.repairs.get() <= CANDIDATE_COUNT.saturating_mul(MAX_REPAIR_ROUNDS),
            "no candidate may exceed the repair bound"
        );
    }

    #[test]
    fn all_failed_candidates_use_a_separately_validated_fallback() {
        let recipe = MockRecipe::default();
        let selection = run_recipe(
            &recipe,
            &MockSettings {
                force_fallback: true,
                invalid_fallback: false,
            },
            12,
            88,
        )
        .expect("the canonical fallback should pass");

        assert!(selection.used_fallback);
        assert_eq!(selection.selected_candidate, None);
        assert_eq!(selection.valid_candidates, 0);
        assert!(selection.plan.metadata.fallback);
        assert_eq!(
            recipe.repairs.get(),
            CANDIDATE_COUNT.saturating_mul(MAX_REPAIR_ROUNDS)
        );
    }

    #[test]
    fn an_invalid_fallback_is_an_error_not_an_empty_plan() {
        let error = run_recipe(
            &MockRecipe::default(),
            &MockSettings {
                force_fallback: true,
                invalid_fallback: true,
            },
            12,
            99,
        )
        .expect_err("an invalid canonical fallback must never publish");

        assert!(
            error.to_string().contains("fallback topology failed"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn materialization_publishes_exact_plan_outputs_and_preserves_v1_identity() {
        let selection = run_recipe(
            &MockRecipe::default(),
            &MockSettings {
                force_fallback: false,
                invalid_fallback: false,
            },
            12,
            77,
        )
        .expect("the mock runner should select a valid plan");
        let expected_surface = TilePos::new(HexCoord::ORIGIN, 4);
        let materialized = materialize_selection(selection, &palette(), &test_is_solid)
            .expect("the selected plan should materialize");

        assert_eq!(
            materialized.anchors.get(&MapAnchorId::from("party_start")),
            Some(expected_surface)
        );
        assert_eq!(
            materialized.map.get(expected_surface),
            SubstanceId(2),
            "the semantic stone role should resolve through the supplied palette"
        );
        assert!(materialized.special_regions.is_empty());
        assert!(materialized.interiors.is_empty());
        assert_eq!(
            materialized.view_hint,
            MapViewHint::new((0.0, 10.0, 10.0), (0.0, 0.0, 0.0))
        );
        assert_eq!(materialized.metadata.candidate, 1);
        assert_eq!(materialized.metrics, 1);
        assert_eq!(materialized.selected_candidate, Some(1));
        assert_eq!(materialized.candidates_evaluated, CANDIDATE_COUNT);
        assert_eq!(
            materialized.map_fingerprint,
            crate::procedural::map_fingerprint(&materialized.map, &materialized.special_regions),
            "an interior-free V2 map must retain the exact V1 map identity"
        );
    }

    #[test]
    fn materialized_fingerprint_orders_and_includes_exact_interior_semantics() {
        let selection = run_recipe(
            &MockRecipe::default(),
            &MockSettings {
                force_fallback: false,
                invalid_fallback: false,
            },
            12,
            77,
        )
        .expect("the mock runner should select a valid plan");
        let materialized = materialize_selection(selection, &palette(), &test_is_solid)
            .expect("the selected plan should materialize");
        let first_floor = TilePos::new(HexCoord::ORIGIN, 4);
        let [neighbor, ..] = HexCoord::ORIGIN.neighbors();
        let second_floor = TilePos::new(neighbor, 5);
        let first_roof = TilePos::new(HexCoord::ORIGIN, 8);
        let second_roof = TilePos::new(neighbor, 9);
        let low_region = InteriorRegionId(2);
        let high_region = InteriorRegionId(7);

        let mut forward = InteriorRegions::new();
        forward.insert_surface(first_floor, low_region);
        forward.insert_surface(second_floor, high_region);
        forward.insert_roof_voxel(first_roof, low_region);
        forward.insert_roof_voxel(second_roof, high_region);

        let mut reverse = InteriorRegions::new();
        reverse.insert_roof_voxel(second_roof, high_region);
        reverse.insert_roof_voxel(first_roof, low_region);
        reverse.insert_surface(second_floor, high_region);
        reverse.insert_surface(first_floor, low_region);

        let forward_fingerprint = materialized_map_fingerprint(
            &materialized.map,
            &materialized.special_regions,
            &forward,
        );
        assert_eq!(
            forward_fingerprint,
            materialized_map_fingerprint(
                &materialized.map,
                &materialized.special_regions,
                &reverse
            ),
            "hash-map insertion order must not affect map identity"
        );

        reverse.insert_roof_voxel(second_roof, low_region);
        assert_ne!(
            forward_fingerprint,
            materialized_map_fingerprint(
                &materialized.map,
                &materialized.special_regions,
                &reverse
            ),
            "changing exact roof ownership must change map identity"
        );
    }
}
