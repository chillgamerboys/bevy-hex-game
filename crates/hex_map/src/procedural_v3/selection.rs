//! Whole-world candidate selection for procedural generator V3.
//!
//! A composite candidate is one complete world. Patches cannot win independently,
//! and neither repair nor fallback may smuggle legacy seed state into the result.

use super::fingerprint::semantic_plan_fingerprint;
use super::world::{GeneratedWorldPlan, WorldValidationIssue};
use super::V3GenerationError;

pub(crate) const CANDIDATE_COUNT: u8 = 8;
pub(crate) const MAX_REPAIR_ROUNDS: u8 = 4;

/// Inputs available while constructing and repairing one deterministic candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CandidateContext {
    pub(crate) grid_radius: u32,
    pub(crate) seed: u64,
    pub(crate) candidate: u8,
}

impl CandidateContext {
    #[must_use]
    const fn new(grid_radius: u32, seed: u64, candidate: u8) -> Self {
        Self {
            grid_radius,
            seed,
            candidate,
        }
    }
}

/// Inputs available to the separately authored canonical fallback.
///
/// It deliberately exposes no seed, candidate, or stream API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FallbackContext {
    pub(crate) grid_radius: u32,
}

impl FallbackContext {
    #[must_use]
    const fn new(grid_radius: u32) -> Self {
        Self { grid_radius }
    }
}

/// Whether validation admitted a complete plan and, if so, its recipe metrics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorldValidation<M> {
    Valid(M),
    Invalid(Vec<WorldValidationIssue>),
}

/// A candidate-local failure which may reject only that candidate or stop the run.
#[derive(Debug)]
pub(crate) enum CandidateAttemptError {
    Rejected(Vec<WorldValidationIssue>),
    Fatal(V3GenerationError),
}

/// One deterministic semantic repair action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepairAction {
    pub(crate) code: &'static str,
    pub(crate) detail: String,
}

impl RepairAction {
    #[must_use]
    pub(crate) fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

/// Result of one repair call.
#[derive(Debug)]
pub(crate) enum RepairOutcome {
    NoChange,
    Changed(Vec<RepairAction>),
}

/// Actions from one counted repair call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepairRound {
    pub(crate) index: u8,
    pub(crate) actions: Vec<RepairAction>,
}

/// Provenance retained for deterministic diagnostics, without parsing strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CandidateNote {
    ConstructionRejected {
        candidate: u8,
        issues: Vec<WorldValidationIssue>,
    },
    ValidationRejected {
        candidate: u8,
        issues: Vec<WorldValidationIssue>,
    },
    RepairRejected {
        candidate: u8,
        round: u8,
        issues: Vec<WorldValidationIssue>,
    },
    FallbackSelected,
}

/// Recipe behavior plugged into the common V3 whole-world runner.
pub(crate) trait V3Recipe {
    type Settings;
    type Metrics: Clone;
    type Score: Ord;

    fn construct(
        &self,
        context: CandidateContext,
        settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, CandidateAttemptError>;

    fn validate(
        &self,
        settings: &Self::Settings,
        plan: &GeneratedWorldPlan,
    ) -> WorldValidation<Self::Metrics>;

    fn repair(
        &self,
        context: CandidateContext,
        settings: &Self::Settings,
        plan: &mut GeneratedWorldPlan,
        round: u8,
        issues: &[WorldValidationIssue],
    ) -> Result<RepairOutcome, CandidateAttemptError>;

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
    ) -> Result<GeneratedWorldPlan, V3GenerationError>;
}

/// Opaque proof that common and recipe-specific validation admitted a plan.
#[derive(Debug)]
pub(crate) struct ValidatedWorldPlan {
    pub(super) plan: GeneratedWorldPlan,
    pub(super) semantic_fingerprint: u64,
}

/// Selected complete world plus deterministic runner provenance.
#[derive(Debug)]
pub(crate) struct ValidatedWorldSelection<M> {
    pub(crate) validated: ValidatedWorldPlan,
    pub(crate) metrics: M,
    pub(crate) selected_candidate: Option<u8>,
    pub(crate) candidates_evaluated: u8,
    pub(crate) valid_candidates: u8,
    pub(crate) repair_rounds: Vec<RepairRound>,
    pub(crate) used_fallback: bool,
    pub(crate) notes: Vec<CandidateNote>,
}

#[derive(Debug)]
struct ValidCandidate<M, S> {
    plan: GeneratedWorldPlan,
    metrics: M,
    candidate: u8,
    repairs: Vec<RepairRound>,
    score: S,
}

/// Evaluates exactly eight complete-world candidates and then one independent
/// fallback only when every candidate fails.
pub(crate) fn run_recipe<R>(
    recipe: &R,
    settings: &R::Settings,
    grid_radius: u32,
    seed: u64,
) -> Result<ValidatedWorldSelection<R::Metrics>, V3GenerationError>
where
    R: V3Recipe,
{
    let mut best: Option<ValidCandidate<R::Metrics, R::Score>> = None;
    let mut valid_candidates = 0_u8;
    let mut notes = Vec::new();

    for candidate in 0..CANDIDATE_COUNT {
        let context = CandidateContext::new(grid_radius, seed, candidate);
        let mut plan = match recipe.construct(context, settings) {
            Ok(plan) => plan,
            Err(CandidateAttemptError::Rejected(issues)) => {
                notes.push(CandidateNote::ConstructionRejected { candidate, issues });
                continue;
            }
            Err(CandidateAttemptError::Fatal(source)) => {
                return Err(V3GenerationError::FatalCandidateConstruction {
                    candidate,
                    source: Box::new(source),
                });
            }
        };
        let (validation, repairs, repair_note) =
            validate_and_repair(recipe, settings, context, &mut plan)?;
        if let Some(note) = repair_note {
            notes.push(note);
        }
        match validation {
            WorldValidation::Valid(metrics) => {
                let score = recipe.score(settings, &metrics, candidate);
                let selected = ValidCandidate {
                    plan,
                    metrics,
                    candidate,
                    repairs,
                    score,
                };
                valid_candidates = valid_candidates.saturating_add(1);
                let replaces_best = best.as_ref().is_none_or(|current| {
                    selected
                        .score
                        .cmp(&current.score)
                        .then_with(|| selected.candidate.cmp(&current.candidate))
                        .is_lt()
                });
                if replaces_best {
                    best = Some(selected);
                }
            }
            WorldValidation::Invalid(issues) => {
                notes.push(CandidateNote::ValidationRejected { candidate, issues });
            }
        }
    }

    if let Some(selected) = best {
        let semantic_fingerprint =
            semantic_plan_fingerprint(&selected.plan).map_err(V3GenerationError::Fingerprint)?;
        return Ok(ValidatedWorldSelection {
            validated: ValidatedWorldPlan {
                plan: selected.plan,
                semantic_fingerprint,
            },
            metrics: selected.metrics,
            selected_candidate: Some(selected.candidate),
            candidates_evaluated: CANDIDATE_COUNT,
            valid_candidates,
            repair_rounds: selected.repairs,
            used_fallback: false,
            notes,
        });
    }

    let fallback = recipe
        .canonical_fallback(FallbackContext::new(grid_radius), settings)
        .map_err(|source| V3GenerationError::FatalFallbackConstruction(Box::new(source)))?;
    let metrics = match validate_plan(recipe, settings, &fallback) {
        WorldValidation::Valid(metrics) => metrics,
        WorldValidation::Invalid(issues) => {
            return Err(V3GenerationError::InvalidFallback(issues));
        }
    };
    let semantic_fingerprint =
        semantic_plan_fingerprint(&fallback).map_err(V3GenerationError::Fingerprint)?;
    notes.push(CandidateNote::FallbackSelected);

    Ok(ValidatedWorldSelection {
        validated: ValidatedWorldPlan {
            plan: fallback,
            semantic_fingerprint,
        },
        metrics,
        selected_candidate: None,
        candidates_evaluated: CANDIDATE_COUNT,
        valid_candidates: 0,
        repair_rounds: Vec::new(),
        used_fallback: true,
        notes,
    })
}

fn validate_and_repair<R>(
    recipe: &R,
    settings: &R::Settings,
    context: CandidateContext,
    plan: &mut GeneratedWorldPlan,
) -> Result<
    (
        WorldValidation<R::Metrics>,
        Vec<RepairRound>,
        Option<CandidateNote>,
    ),
    V3GenerationError,
>
where
    R: V3Recipe,
{
    let mut repairs = Vec::new();
    let mut validation = validate_plan(recipe, settings, plan);
    for round in 0..MAX_REPAIR_ROUNDS {
        let WorldValidation::Invalid(issues) = &validation else {
            break;
        };
        let before = semantic_plan_fingerprint(plan).map_err(V3GenerationError::Fingerprint)?;
        let outcome = match recipe.repair(context, settings, plan, round, issues) {
            Ok(outcome) => outcome,
            Err(CandidateAttemptError::Rejected(rejected)) => {
                return Ok((
                    WorldValidation::Invalid(rejected.clone()),
                    repairs,
                    Some(CandidateNote::RepairRejected {
                        candidate: context.candidate,
                        round,
                        issues: rejected,
                    }),
                ));
            }
            Err(CandidateAttemptError::Fatal(source)) => {
                return Err(V3GenerationError::FatalCandidateRepair {
                    candidate: context.candidate,
                    round,
                    source: Box::new(source),
                });
            }
        };
        let after = semantic_plan_fingerprint(plan).map_err(V3GenerationError::Fingerprint)?;
        match outcome {
            RepairOutcome::NoChange => {
                if before != after {
                    return Err(V3GenerationError::RecipeContract(format!(
                        "candidate {} repair round {round} returned NoChange after mutating the \
                         semantic plan",
                        context.candidate
                    )));
                }
                break;
            }
            RepairOutcome::Changed(actions) => {
                if actions.is_empty() {
                    return Err(V3GenerationError::RecipeContract(format!(
                        "candidate {} repair round {round} returned Changed without actions",
                        context.candidate
                    )));
                }
                if before == after {
                    return Err(V3GenerationError::RecipeContract(format!(
                        "candidate {} repair round {round} returned Changed without changing the \
                         semantic plan",
                        context.candidate
                    )));
                }
                repairs.push(RepairRound {
                    index: round,
                    actions,
                });
            }
        }
        validation = validate_plan(recipe, settings, plan);
    }
    Ok((validation, repairs, None))
}

fn validate_plan<R>(
    recipe: &R,
    settings: &R::Settings,
    plan: &GeneratedWorldPlan,
) -> WorldValidation<R::Metrics>
where
    R: V3Recipe,
{
    let common = plan.validate();
    if common.is_empty() {
        recipe.validate(settings, plan)
    } else {
        WorldValidation::Invalid(common)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::{BTreeMap, BTreeSet};

    use hex_core::{BiomeRegionId, HexCoord, MapViewHint, TilePos};

    use super::*;
    use crate::procedural_v3::layout::{
        LayoutKind, PatchId, ResolvedEdgeReference, ResolvedLayoutPlan, ResolvedPatch,
    };
    use crate::procedural_v3::volume::{
        LevelInterval, SolidMass, SolidMaterialRole, SurfaceAccess, SurfaceMetadata, VolumeColumn,
        VolumeElement, VolumePlan,
    };
    use crate::procedural_v3::world::{
        FeaturePlan, InteriorPlan, LiquidPlan, StructurePlan, WorldIssueCode,
    };

    #[derive(Debug, Default, Clone, Copy)]
    struct MockSettings {
        force_fallback: bool,
        invalid_fallback: bool,
        common_invalid: bool,
        repairs_before_valid: u8,
        no_change_mutates: bool,
        changed_without_mutation: bool,
        reject_construction: Option<u8>,
        fatal_construction: Option<u8>,
        equal_scores: bool,
    }

    #[derive(Default)]
    struct MockRecipe {
        constructions: Cell<u8>,
        validations: Cell<u8>,
        repair_calls: Cell<u8>,
        fallback_calls: Cell<u8>,
    }

    impl V3Recipe for MockRecipe {
        type Settings = MockSettings;
        type Metrics = u8;
        type Score = (u8, u8);

        fn construct(
            &self,
            context: CandidateContext,
            settings: &Self::Settings,
        ) -> Result<GeneratedWorldPlan, CandidateAttemptError> {
            self.constructions
                .set(self.constructions.get().saturating_add(1));
            if settings.reject_construction == Some(context.candidate) {
                return Err(CandidateAttemptError::Rejected(vec![
                    WorldValidationIssue::new(
                        WorldIssueCode::Recipe("mock"),
                        "construction rejected",
                    ),
                ]));
            }
            if settings.fatal_construction == Some(context.candidate) {
                return Err(CandidateAttemptError::Fatal(
                    V3GenerationError::RecipeContract("construction exploded".to_owned()),
                ));
            }
            let mut plan = mock_plan(0, false);
            if settings.common_invalid {
                plan.biome_regions.clear();
            }
            Ok(plan)
        }

        fn validate(
            &self,
            settings: &Self::Settings,
            plan: &GeneratedWorldPlan,
        ) -> WorldValidation<Self::Metrics> {
            self.validations
                .set(self.validations.get().saturating_add(1));
            let marker = marker(plan);
            let fallback = marker == u8::MAX;
            if (fallback && settings.invalid_fallback)
                || (!fallback
                    && (settings.force_fallback || marker < settings.repairs_before_valid))
            {
                WorldValidation::Invalid(vec![WorldValidationIssue::new(
                    WorldIssueCode::Recipe("mock"),
                    "mock recipe remains invalid",
                )])
            } else {
                WorldValidation::Valid(marker)
            }
        }

        fn repair(
            &self,
            _context: CandidateContext,
            settings: &Self::Settings,
            plan: &mut GeneratedWorldPlan,
            _round: u8,
            _issues: &[WorldValidationIssue],
        ) -> Result<RepairOutcome, CandidateAttemptError> {
            self.repair_calls
                .set(self.repair_calls.get().saturating_add(1));
            if settings.no_change_mutates {
                bump_marker(plan);
                return Ok(RepairOutcome::NoChange);
            }
            if settings.changed_without_mutation {
                return Ok(RepairOutcome::Changed(vec![RepairAction::new(
                    "mock",
                    "claimed mutation",
                )]));
            }
            bump_marker(plan);
            Ok(RepairOutcome::Changed(vec![RepairAction::new(
                "mock",
                "advance marker",
            )]))
        }

        fn score(
            &self,
            settings: &Self::Settings,
            metrics: &Self::Metrics,
            candidate: u8,
        ) -> Self::Score {
            if settings.equal_scores {
                (0, 0)
            } else {
                (*metrics, candidate)
            }
        }

        fn canonical_fallback(
            &self,
            context: FallbackContext,
            settings: &Self::Settings,
        ) -> Result<GeneratedWorldPlan, V3GenerationError> {
            if context.grid_radius != 12 {
                return Err(V3GenerationError::RecipeContract(
                    "mock fallback expected radius 12".to_owned(),
                ));
            }
            self.fallback_calls
                .set(self.fallback_calls.get().saturating_add(1));
            let mut plan = mock_plan(u8::MAX, true);
            if settings.common_invalid {
                plan.biome_regions.clear();
            }
            Ok(plan)
        }
    }

    fn mock_plan(marker: u8, fallback: bool) -> GeneratedWorldPlan {
        let coord = HexCoord::ORIGIN;
        let position = TilePos::new(coord, 0);
        let mask = BTreeSet::from([coord]);
        let edges = crate::procedural_v3::layout::HexSide::ALL
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
                    rotation_turns: 0,
                    mask: mask.clone(),
                    edges,
                },
            )]),
            shared_edges: BTreeMap::new(),
            boundary_liquid_outlets: BTreeMap::new(),
        };
        let mut volume = VolumePlan::new(mask);
        volume.columns.insert(
            coord,
            VolumeColumn {
                elements: vec![VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(0, 1),
                    material: SolidMaterialRole::Bedrock,
                    cutaway_for: None,
                })],
            },
        );
        volume.surfaces.insert(
            position,
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: None,
            },
        );
        GeneratedWorldPlan {
            layout,
            volume,
            liquids: LiquidPlan::default(),
            features: FeaturePlan::default(),
            structures: StructurePlan::default(),
            blockers: BTreeSet::new(),
            lights: BTreeMap::new(),
            biome_regions: BTreeMap::from([(position, BiomeRegionId(0))]),
            interiors: InteriorPlan::default(),
            anchors: {
                let mut anchors = BTreeMap::from([("party_start".to_owned(), position)]);
                if fallback {
                    anchors.insert("fallback_marker".to_owned(), position);
                } else if marker > 0 {
                    anchors.insert(format!("repair_{marker}"), position);
                }
                anchors
            },
            view_hint: MapViewHint::new((1.0, 2.0, 1.0), (0.0, 0.0, 0.0)),
        }
    }

    fn marker(plan: &GeneratedWorldPlan) -> u8 {
        if plan.anchors.contains_key("fallback_marker") {
            return u8::MAX;
        }
        plan.anchors
            .keys()
            .find_map(|name| name.strip_prefix("repair_"))
            .and_then(|value| value.parse().ok())
            .unwrap_or(0)
    }

    fn bump_marker(plan: &mut GeneratedWorldPlan) {
        let next = marker(plan).saturating_add(1);
        plan.anchors.retain(|name, _| !name.starts_with("repair_"));
        let position = *plan
            .anchors
            .get("party_start")
            .expect("mock plan keeps its party anchor");
        plan.anchors.insert(format!("repair_{next}"), position);
    }

    #[test]
    fn all_eight_candidates_are_evaluated_and_ties_choose_lowest_index() {
        let recipe = MockRecipe::default();
        let selected = run_recipe(
            &recipe,
            &MockSettings {
                equal_scores: true,
                ..Default::default()
            },
            12,
            77,
        )
        .expect("valid candidates");

        assert_eq!(recipe.constructions.get(), CANDIDATE_COUNT);
        assert_eq!(selected.candidates_evaluated, CANDIDATE_COUNT);
        assert_eq!(selected.valid_candidates, CANDIDATE_COUNT);
        assert_eq!(selected.selected_candidate, Some(0));
        assert!(!selected.used_fallback);
        assert_eq!(selected.metrics, 0);
        assert_ne!(selected.validated.semantic_fingerprint, 0);
        assert!(selected.validated.plan.anchors.contains_key("party_start"));
    }

    #[test]
    fn repair_calls_are_bounded_by_rounds_not_action_count() {
        let recipe = MockRecipe::default();
        let selected = run_recipe(
            &recipe,
            &MockSettings {
                repairs_before_valid: 2,
                ..Default::default()
            },
            12,
            77,
        )
        .expect("candidates become valid");

        assert_eq!(recipe.repair_calls.get(), CANDIDATE_COUNT * 2);
        assert_eq!(selected.repair_rounds.len(), 2);
        assert_eq!(
            selected.repair_rounds.first().map(|round| round.index),
            Some(0)
        );
        assert_eq!(
            selected.repair_rounds.get(1).map(|round| round.index),
            Some(1)
        );
    }

    #[test]
    fn fallback_runs_only_after_eight_failures_and_receives_no_repairs() {
        let recipe = MockRecipe::default();
        let selected = run_recipe(
            &recipe,
            &MockSettings {
                force_fallback: true,
                ..Default::default()
            },
            12,
            91,
        )
        .expect("valid fallback");

        assert_eq!(recipe.constructions.get(), CANDIDATE_COUNT);
        assert_eq!(
            recipe.repair_calls.get(),
            CANDIDATE_COUNT * MAX_REPAIR_ROUNDS
        );
        assert_eq!(recipe.fallback_calls.get(), 1);
        assert!(selected.used_fallback);
        assert!(selected.repair_rounds.is_empty());
        assert_eq!(selected.selected_candidate, None);
    }

    #[test]
    fn invalid_fallback_is_fatal() {
        let error = run_recipe(
            &MockRecipe::default(),
            &MockSettings {
                force_fallback: true,
                invalid_fallback: true,
                ..Default::default()
            },
            12,
            1,
        )
        .expect_err("fallback must pass hard validation");

        assert!(matches!(error, V3GenerationError::InvalidFallback(_)));
    }

    #[test]
    fn common_validation_rejects_candidates_and_fallback_before_recipe_validation() {
        let settings = MockSettings {
            common_invalid: true,
            ..Default::default()
        };
        let recipe = MockRecipe::default();
        let mut proof = mock_plan(0, false);
        proof.biome_regions.clear();
        assert!(matches!(
            recipe.validate(&settings, &proof),
            WorldValidation::Valid(0)
        ));
        recipe.validations.set(0);

        let error = run_recipe(&recipe, &settings, 12, 1)
            .expect_err("the common-invalid fallback must fail closed");
        assert!(matches!(error, V3GenerationError::InvalidFallback(_)));
        assert_eq!(recipe.constructions.get(), CANDIDATE_COUNT);
        assert_eq!(recipe.fallback_calls.get(), 1);
        assert_eq!(
            recipe.validations.get(),
            0,
            "recipe-specific validation must never admit a common-invalid plan"
        );
    }

    #[test]
    fn repair_outcomes_must_match_actual_mutation() {
        for settings in [
            MockSettings {
                repairs_before_valid: 1,
                no_change_mutates: true,
                ..Default::default()
            },
            MockSettings {
                repairs_before_valid: 1,
                changed_without_mutation: true,
                ..Default::default()
            },
        ] {
            let error = run_recipe(&MockRecipe::default(), &settings, 12, 1)
                .expect_err("dishonest repair outcome must fail");
            assert!(matches!(error, V3GenerationError::RecipeContract(_)));
        }
    }

    #[test]
    fn rejected_and_fatal_construction_are_distinct() {
        let rejected = run_recipe(
            &MockRecipe::default(),
            &MockSettings {
                reject_construction: Some(0),
                equal_scores: true,
                ..Default::default()
            },
            12,
            1,
        )
        .expect("other candidates remain selectable");
        assert_eq!(rejected.valid_candidates, CANDIDATE_COUNT - 1);
        assert!(matches!(
            rejected.notes.first(),
            Some(CandidateNote::ConstructionRejected { candidate: 0, .. })
        ));

        let fatal = run_recipe(
            &MockRecipe::default(),
            &MockSettings {
                fatal_construction: Some(3),
                ..Default::default()
            },
            12,
            1,
        )
        .expect_err("fatal construction stops the run");
        assert!(matches!(
            fatal,
            V3GenerationError::FatalCandidateConstruction { candidate: 3, .. }
        ));
    }
}
