//! Recipe-independent candidate selection for procedural generator V2.
use std::fmt::Debug;
use std::ops::Deref;

use hex_core::{
    InteriorRegions, MapAnchorId, MapAnchors, MapViewHint, SpecialMovementRegions, SubstanceId,
};
use xxhash_rust::xxh3::xxh3_64;

use super::seed::SeedStreams;
use super::volume::{voxelize_prevalidated, SurfaceAccess, TerrainVolumePlan, VoxelizedTerrain};
use super::V2GenerationError;
use crate::procedural::TacticalMetrics;
use crate::terrain::TerrainPalette;
use crate::voxel::VoxelMap;

pub(crate) const CANDIDATE_COUNT: u8 = 8;
pub(crate) const MAX_REPAIR_ROUNDS: u8 = 4;

/// Supplies the stable tactical subset used by the cross-version generation report.
///
/// Recipe-specific metrics remain strongly typed. This adapter is the only shared
/// reporting surface, so Mountains and Caves do not have to pretend their semantic
/// measurements are Hills measurements.
pub(crate) trait ReportMetrics {
    fn tactical(&self) -> TacticalMetrics;
}

/// Stable inputs available while constructing or repairing one candidate.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CandidateContext {
    pub(crate) grid_radius: u32,
    pub(crate) candidate: u8,
    pub(crate) streams: SeedStreams,
}

impl CandidateContext {
    #[must_use]
    fn new(grid_radius: u32, seed: u64, candidate: u8) -> Self {
        Self {
            grid_radius,
            candidate,
            streams: SeedStreams::new(seed, candidate),
        }
    }
}

/// Stable inputs available while constructing a separately authored fallback.
///
/// Native V2 fallbacks derive geometry without sampling the requested seed.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FallbackContext {
    pub(crate) grid_radius: u32,
}

impl FallbackContext {
    #[must_use]
    const fn new(grid_radius: u32) -> Self {
        Self { grid_radius }
    }
}

/// Origin of a semantic plan being validated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValidationProvenance {
    Candidate(u8),
    Fallback,
}

/// Stable non-random inputs available to recipe validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ValidationContext {
    pub(crate) grid_radius: u32,
    pub(crate) provenance: ValidationProvenance,
}

impl ValidationContext {
    #[must_use]
    const fn candidate(grid_radius: u32, candidate: u8) -> Self {
        Self {
            grid_radius,
            provenance: ValidationProvenance::Candidate(candidate),
        }
    }

    #[must_use]
    const fn fallback(grid_radius: u32) -> Self {
        Self {
            grid_radius,
            provenance: ValidationProvenance::Fallback,
        }
    }
}

/// One recipe's semantic plan and topology-specific metadata.
#[derive(Debug, Clone)]
pub(crate) struct RecipePlan<M> {
    pub(crate) volume: TerrainVolumePlan,
    pub(crate) metadata: M,
}

/// An expected candidate rejection or an error that must stop generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CandidateAttemptError {
    Rejected(Vec<String>),
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "fatal native-recipe construction is exercised by the common runner tests"
        )
    )]
    Fatal(V2GenerationError),
}

impl CandidateAttemptError {
    #[must_use]
    pub(crate) fn rejected(issue: impl Into<String>) -> Self {
        Self::Rejected(vec![issue.into()])
    }

    #[must_use]
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "fatal native-recipe construction is exercised by the common runner tests"
        )
    )]
    pub(crate) const fn fatal(error: V2GenerationError) -> Self {
        Self::Fatal(error)
    }
}

/// Result of one recipe-specific validation pass.
#[derive(Debug, Clone)]
pub(crate) enum RecipeValidation<M> {
    Valid(M),
    Invalid(Vec<String>),
}

impl<M> RecipeValidation<M> {
    #[must_use]
    pub(crate) fn valid(metrics: M) -> Self {
        Self::Valid(metrics)
    }

    #[must_use]
    pub(crate) fn invalid(issues: Vec<String>) -> Self {
        Self::Invalid(issues)
    }
}

/// Whether a bounded repair changed semantic intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RepairOutcome {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Layered Sky Islands constructs hard-valid candidates without local repairs"
        )
    )]
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
    ) -> Result<RecipePlan<Self::Metadata>, CandidateAttemptError>;

    fn validate(
        &self,
        context: ValidationContext,
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
    ) -> Result<RepairOutcome, CandidateAttemptError>;

    fn score(
        &self,
        settings: &Self::Settings,
        metrics: &Self::Metrics,
        candidate: u8,
    ) -> Self::Score;

    /// Semantic repairs completed before the common V2 runner receives the plan.
    ///
    /// Compatibility constructors may import repairs already performed by a frozen
    /// generator. Native V2 recipes should normally return an empty list and use
    /// [`Self::repair`] so the common runner owns their repair bound.
    fn preexisting_repair_actions(&self, _plan: &RecipePlan<Self::Metadata>) -> Vec<String> {
        Vec::new()
    }

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

/// Type-state proof that the complete semantic volume passed common validation.
///
/// The wrapper deliberately exposes no mutable dereference. A layered recipe must
/// consume it with [`Self::into_unvalidated`], change the raw plan, then submit the
/// layered recipe through [`run_recipe`] before materialization.
#[derive(Debug)]
pub(crate) struct ValidatedRecipeSelection<M, V>(RecipeSelection<M, V>);

impl<M, V> ValidatedRecipeSelection<M, V> {
    /// Imports a selection whose recipe-specific validation happened outside V2.
    ///
    /// Native and layered V2 recipes must use [`run_recipe`] so this common volume
    /// check cannot preserve stale recipe metrics after a semantic change.
    pub(super) fn from_compatibility_import(
        selection: RecipeSelection<M, V>,
    ) -> Result<Self, V2GenerationError> {
        selection.plan.volume.validate()?;
        Ok(Self(selection))
    }

    pub(crate) fn into_unvalidated(self) -> RecipeSelection<M, V> {
        self.0
    }

    /// Adds report-only facts without reopening the validated semantic plan.
    pub(crate) fn prepend_diagnostics(&mut self, note: String, inherited_fallback: bool) {
        self.0.notes.insert(0, note);
        self.0.used_fallback |= inherited_fallback;
    }

    const fn from_recipe_validation(selection: RecipeSelection<M, V>) -> Self {
        Self(selection)
    }
}

impl<M, V> Deref for ValidatedRecipeSelection<M, V> {
    type Target = RecipeSelection<M, V>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
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
    selection: ValidatedRecipeSelection<M, V>,
    palette: &TerrainPalette,
    is_solid: &dyn Fn(SubstanceId) -> bool,
) -> Result<MaterializedSelection<M, V>, V2GenerationError> {
    let selection = selection.into_unvalidated();
    let VoxelizedTerrain { map, interiors } =
        voxelize_prevalidated(&selection.plan.volume, palette, is_solid)?;
    let special_regions = exact_special_regions(&selection.plan.volume);
    let map_fingerprint = materialized_map_fingerprint(&map, &special_regions, &interiors);
    Ok(assemble_materialized(
        selection,
        map,
        interiors,
        special_regions,
        map_fingerprint,
    ))
}

fn exact_special_regions(volume: &TerrainVolumePlan) -> SpecialMovementRegions {
    volume
        .surfaces
        .iter()
        .filter_map(|(position, surface)| match surface.access {
            SurfaceAccess::SpecialMovement(region) => Some((*position, region)),
            SurfaceAccess::Ordinary | SurfaceAccess::NonStandable => None,
        })
        .collect()
}

fn assemble_materialized<M, V>(
    selection: RecipeSelection<M, V>,
    map: VoxelMap,
    interiors: InteriorRegions,
    special_regions: SpecialMovementRegions,
    map_fingerprint: u64,
) -> MaterializedSelection<M, V> {
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
    let view_hint = volume.view_hint;

    MaterializedSelection {
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
    }
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
) -> Result<ValidatedRecipeSelection<R::Metadata, R::Metrics>, V2GenerationError>
where
    R: V2Recipe,
{
    let mut valid = Vec::new();
    let mut rejected_notes = Vec::new();

    for candidate in 0..CANDIDATE_COUNT {
        let context = CandidateContext::new(grid_radius, seed, candidate);
        let mut plan = match recipe.construct(context, settings) {
            Ok(plan) => plan,
            Err(CandidateAttemptError::Rejected(issues)) => {
                rejected_notes.push(format!(
                    "candidate {candidate}: construction rejected: {}",
                    describe_issues(&issues)
                ));
                continue;
            }
            Err(CandidateAttemptError::Fatal(source)) => {
                return Err(V2GenerationError::FatalCandidateConstruction {
                    candidate,
                    source: Box::new(source),
                });
            }
        };
        let (validation, repair_actions) =
            validate_and_repair(recipe, settings, context, &mut plan)?;
        match validation {
            RecipeValidation::Valid(metrics) => {
                let score = recipe.score(settings, &metrics, candidate);
                valid.push(ValidCandidate {
                    plan,
                    metrics,
                    candidate,
                    repair_actions,
                    score,
                });
            }
            RecipeValidation::Invalid(issues) => {
                rejected_notes.push(format!(
                    "candidate {candidate}: {}",
                    describe_issues(&issues)
                ));
            }
        }
    }

    let valid_candidates = u8::try_from(valid.len()).unwrap_or(u8::MAX);
    if let Some(selected) = valid.into_iter().min_by(|left, right| {
        left.score
            .cmp(&right.score)
            .then_with(|| left.candidate.cmp(&right.candidate))
    }) {
        return Ok(ValidatedRecipeSelection::from_recipe_validation(
            RecipeSelection {
                plan: selected.plan,
                metrics: selected.metrics,
                selected_candidate: Some(selected.candidate),
                candidates_evaluated: CANDIDATE_COUNT,
                valid_candidates,
                repair_actions: selected.repair_actions,
                used_fallback: false,
                notes: rejected_notes,
            },
        ));
    }

    let fallback_context = FallbackContext::new(grid_radius);
    let fallback = recipe.canonical_fallback(fallback_context, settings)?;
    let fallback_repair_actions = bounded_preexisting_repairs(recipe, &fallback)?;
    let validation = validate_plan(
        recipe,
        settings,
        ValidationContext::fallback(grid_radius),
        &fallback,
    )?;
    let metrics = match validation {
        RecipeValidation::Valid(metrics) => metrics,
        RecipeValidation::Invalid(issues) => {
            return Err(V2GenerationError::InvalidFallback(issues));
        }
    };
    rejected_notes.push("all random candidates failed; canonical fallback selected".to_owned());

    Ok(ValidatedRecipeSelection::from_recipe_validation(
        RecipeSelection {
            plan: fallback,
            metrics,
            selected_candidate: None,
            candidates_evaluated: CANDIDATE_COUNT,
            valid_candidates: 0,
            repair_actions: fallback_repair_actions,
            used_fallback: true,
            notes: rejected_notes,
        },
    ))
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
    let mut repair_actions = bounded_preexisting_repairs(recipe, plan)?;
    let first_available_round = u8::try_from(repair_actions.len()).unwrap_or(MAX_REPAIR_ROUNDS);
    let validation_context = ValidationContext::candidate(context.grid_radius, context.candidate);
    let mut validation = validate_plan(recipe, settings, validation_context, plan)?;
    for round in first_available_round..MAX_REPAIR_ROUNDS {
        let RecipeValidation::Invalid(issues) = &validation else {
            break;
        };
        match recipe.repair(context, settings, plan, round, issues.as_slice()) {
            Ok(RepairOutcome::Changed(action)) => repair_actions.push(action),
            Ok(RepairOutcome::NoChange) => break,
            Err(CandidateAttemptError::Rejected(reasons)) => {
                let mut issues = issues.clone();
                issues.push(format!(
                    "repair round {round} rejected candidate: {}",
                    describe_issues(&reasons)
                ));
                return Ok((RecipeValidation::Invalid(issues), repair_actions));
            }
            Err(CandidateAttemptError::Fatal(source)) => {
                return Err(V2GenerationError::FatalCandidateRepair {
                    candidate: context.candidate,
                    round,
                    source: Box::new(source),
                });
            }
        }
        validation = validate_plan(recipe, settings, validation_context, plan)?;
    }
    Ok((validation, repair_actions))
}

fn bounded_preexisting_repairs<R>(
    recipe: &R,
    plan: &RecipePlan<R::Metadata>,
) -> Result<Vec<String>, V2GenerationError>
where
    R: V2Recipe,
{
    let actions = recipe.preexisting_repair_actions(plan);
    if actions.len() > usize::from(MAX_REPAIR_ROUNDS) {
        return Err(V2GenerationError::RecipeContract(format!(
            "candidate imported {} repair rounds; the V2 limit is {MAX_REPAIR_ROUNDS}",
            actions.len()
        )));
    }
    Ok(actions)
}

fn validate_plan<R>(
    recipe: &R,
    settings: &R::Settings,
    context: ValidationContext,
    plan: &RecipePlan<R::Metadata>,
) -> Result<RecipeValidation<R::Metrics>, V2GenerationError>
where
    R: V2Recipe,
{
    match plan.volume.validate() {
        Ok(()) => Ok(recipe.validate(context, settings, plan)),
        Err(V2GenerationError::InvalidVolume(issues)) => Ok(RecipeValidation::Invalid(issues)),
        Err(other) => Err(other),
    }
}

fn describe_issues(issues: &[String]) -> String {
    if issues.is_empty() {
        "rejected without a reason".to_owned()
    } else {
        issues.join("; ")
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;

    use hex_core::{HexCoord, InteriorRegionId, MapViewHint, TilePos};

    use super::*;
    use crate::procedural_v2::volume::{
        LevelInterval, SolidMass, SolidMaterialRole, SurfaceAccess, SurfaceMetadata, VolumeColumn,
        VolumeElement,
    };
    use crate::terrain::TerrainPalette;

    #[derive(Debug, Default, Clone, Copy)]
    struct MockSettings {
        force_fallback: bool,
        invalid_fallback: bool,
        invalid_fallback_volume: bool,
        rejected_construction: Option<u8>,
        rejected_repair: Option<(u8, u8)>,
        fatal_construction: Option<u8>,
        fatal_repair: Option<(u8, u8)>,
        no_change: Option<u8>,
        equal_scores: bool,
        imported_repairs: u8,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct MockMetadata {
        candidate: u8,
        repairs: u8,
        fallback: bool,
        imported_repairs: u8,
    }

    #[derive(Default)]
    struct MockRecipe {
        constructions: Cell<u8>,
        repairs: Cell<u8>,
        validations: RefCell<Vec<ValidationProvenance>>,
    }

    impl V2Recipe for MockRecipe {
        type Settings = MockSettings;
        type Metadata = MockMetadata;
        type Metrics = u8;
        type Score = (u8, u8);

        fn construct(
            &self,
            context: CandidateContext,
            settings: &Self::Settings,
        ) -> Result<RecipePlan<Self::Metadata>, CandidateAttemptError> {
            self.constructions
                .set(self.constructions.get().saturating_add(1));
            if settings.rejected_construction == Some(context.candidate) {
                return Err(CandidateAttemptError::rejected(
                    "candidate-specific construction constraint",
                ));
            }
            if settings.fatal_construction == Some(context.candidate) {
                return Err(CandidateAttemptError::fatal(
                    V2GenerationError::MaterialContract("construction exploded".to_owned()),
                ));
            }
            Ok(mock_plan(
                context.grid_radius,
                MockMetadata {
                    candidate: context.candidate,
                    repairs: settings.imported_repairs,
                    fallback: false,
                    imported_repairs: settings.imported_repairs,
                },
            ))
        }

        fn validate(
            &self,
            context: ValidationContext,
            settings: &Self::Settings,
            plan: &RecipePlan<Self::Metadata>,
        ) -> RecipeValidation<Self::Metrics> {
            self.validations.borrow_mut().push(context.provenance);
            let required_repairs = if settings.force_fallback {
                u8::MAX
            } else if settings.equal_scores {
                0
            } else if plan.metadata.candidate == 0 {
                5
            } else if plan.metadata.candidate == 1 {
                1
            } else {
                0
            };
            if context.provenance == ValidationProvenance::Fallback && settings.invalid_fallback {
                return RecipeValidation::invalid(vec!["fallback topology failed".to_owned()]);
            }
            if plan.metadata.repairs < required_repairs && !plan.metadata.fallback {
                RecipeValidation::invalid(vec!["candidate needs repair".to_owned()])
            } else {
                RecipeValidation::valid(plan.metadata.candidate)
            }
        }

        fn repair(
            &self,
            context: CandidateContext,
            settings: &Self::Settings,
            plan: &mut RecipePlan<Self::Metadata>,
            round: u8,
            _issues: &[String],
        ) -> Result<RepairOutcome, CandidateAttemptError> {
            self.repairs.set(self.repairs.get().saturating_add(1));
            if settings.rejected_repair == Some((context.candidate, round)) {
                return Err(CandidateAttemptError::rejected(
                    "candidate-specific repair constraint",
                ));
            }
            if settings.fatal_repair == Some((context.candidate, round)) {
                return Err(CandidateAttemptError::fatal(
                    V2GenerationError::MaterialContract("repair exploded".to_owned()),
                ));
            }
            if settings.no_change == Some(context.candidate) {
                return Ok(RepairOutcome::NoChange);
            }
            plan.metadata.repairs = plan.metadata.repairs.saturating_add(1);
            Ok(RepairOutcome::Changed(format!("repair round {round}")))
        }

        fn score(
            &self,
            settings: &Self::Settings,
            _metrics: &Self::Metrics,
            candidate: u8,
        ) -> Self::Score {
            if settings.equal_scores {
                (0, 0)
            } else {
                ((candidate > 1).into(), candidate)
            }
        }

        fn preexisting_repair_actions(&self, plan: &RecipePlan<Self::Metadata>) -> Vec<String> {
            let count = if plan.metadata.fallback {
                plan.metadata.imported_repairs.max(1)
            } else {
                plan.metadata.imported_repairs
            };
            (0..count)
                .map(|round| format!("imported repair {round}"))
                .collect()
        }

        fn canonical_fallback(
            &self,
            context: FallbackContext,
            settings: &Self::Settings,
        ) -> Result<RecipePlan<Self::Metadata>, V2GenerationError> {
            let mut plan = mock_plan(
                context.grid_radius,
                MockMetadata {
                    candidate: 0,
                    repairs: 0,
                    fallback: true,
                    imported_repairs: settings.imported_repairs,
                },
            );
            if settings.invalid_fallback_volume {
                plan.volume.columns.remove(&HexCoord::ORIGIN);
            }
            Ok(plan)
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
        let _stream_identity = CandidateContext::new(12, 77, 0)
            .streams
            .stage("mock.construct")
            .sample(0);
        let recipe = MockRecipe::default();
        let selection = run_recipe(&recipe, &MockSettings::default(), 12, 77)
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
    fn imported_repairs_consume_the_shared_four_round_budget() {
        let recipe = MockRecipe::default();
        let selection = run_recipe(
            &recipe,
            &MockSettings {
                imported_repairs: MAX_REPAIR_ROUNDS,
                ..MockSettings::default()
            },
            12,
            77,
        )
        .expect("a candidate finalized within the imported repair budget should pass");

        assert_eq!(selection.selected_candidate, Some(1));
        assert_eq!(
            selection.repair_actions.len(),
            usize::from(MAX_REPAIR_ROUNDS)
        );
        assert_eq!(
            recipe.repairs.get(),
            0,
            "the common runner must not add rounds after the imported budget is exhausted"
        );
    }

    #[test]
    fn excessive_imported_repairs_are_a_recipe_contract_error() {
        let error = run_recipe(
            &MockRecipe::default(),
            &MockSettings {
                imported_repairs: MAX_REPAIR_ROUNDS.saturating_add(1),
                ..MockSettings::default()
            },
            12,
            77,
        )
        .expect_err("more than four imported repair rounds must be rejected");

        assert!(
            matches!(error, V2GenerationError::RecipeContract(_)),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn fatal_construction_stops_generation_with_candidate_context() {
        let recipe = MockRecipe::default();
        let error = run_recipe(
            &recipe,
            &MockSettings {
                fatal_construction: Some(3),
                ..MockSettings::default()
            },
            12,
            77,
        )
        .expect_err("a fatal construction error must stop the complete runner");

        assert_eq!(
            error,
            V2GenerationError::FatalCandidateConstruction {
                candidate: 3,
                source: Box::new(V2GenerationError::MaterialContract(
                    "construction exploded".to_owned()
                )),
            }
        );
        assert_eq!(
            recipe.constructions.get(),
            4,
            "later candidates must not hide a fatal construction error"
        );
    }

    #[test]
    fn fatal_repair_stops_generation_with_candidate_and_round_context() {
        let recipe = MockRecipe::default();
        let error = run_recipe(
            &recipe,
            &MockSettings {
                fatal_repair: Some((0, 2)),
                ..MockSettings::default()
            },
            12,
            77,
        )
        .expect_err("a fatal repair error must stop the complete runner");

        assert_eq!(
            error,
            V2GenerationError::FatalCandidateRepair {
                candidate: 0,
                round: 2,
                source: Box::new(V2GenerationError::MaterialContract(
                    "repair exploded".to_owned()
                )),
            }
        );
        assert_eq!(
            recipe.constructions.get(),
            1,
            "later candidates must not hide a fatal repair error"
        );
    }

    #[test]
    fn explicit_candidate_rejections_are_diagnostic_and_do_not_abort() {
        let selection = run_recipe(
            &MockRecipe::default(),
            &MockSettings {
                rejected_construction: Some(0),
                rejected_repair: Some((1, 0)),
                ..MockSettings::default()
            },
            12,
            77,
        )
        .expect("explicitly rejected candidates should not abort later attempts");

        assert_eq!(selection.selected_candidate, Some(2));
        assert!(selection.notes.iter().any(|note| {
            note.contains("candidate 0")
                && note.contains("construction rejected")
                && note.contains("candidate-specific construction constraint")
        }));
        assert!(selection.notes.iter().any(|note| {
            note.contains("candidate 1")
                && note.contains("repair round 0 rejected candidate")
                && note.contains("candidate-specific repair constraint")
        }));
    }

    #[test]
    fn no_change_stops_repairing_only_the_current_candidate() {
        let recipe = MockRecipe::default();
        let selection = run_recipe(
            &recipe,
            &MockSettings {
                no_change: Some(1),
                ..MockSettings::default()
            },
            12,
            77,
        )
        .expect("later candidates should remain available after NoChange");

        assert_eq!(selection.selected_candidate, Some(2));
        assert_eq!(
            recipe.repairs.get(),
            MAX_REPAIR_ROUNDS + 1,
            "candidate 1 should stop after its first NoChange"
        );
        assert!(selection
            .notes
            .iter()
            .any(|note| note.contains("candidate 1: candidate needs repair")));
    }

    #[test]
    fn equal_scores_choose_the_lowest_candidate_deterministically() {
        for seed in [0, 1, u64::MAX] {
            let selection = run_recipe(
                &MockRecipe::default(),
                &MockSettings {
                    equal_scores: true,
                    ..MockSettings::default()
                },
                12,
                seed,
            )
            .expect("all equal-score candidates should be valid");

            assert_eq!(selection.valid_candidates, CANDIDATE_COUNT);
            assert_eq!(selection.selected_candidate, Some(0));
            assert_eq!(selection.plan.metadata.candidate, 0);
        }
    }

    #[test]
    fn all_failed_candidates_use_a_separately_validated_fallback() {
        let recipe = MockRecipe::default();
        let selection = run_recipe(
            &recipe,
            &MockSettings {
                force_fallback: true,
                ..MockSettings::default()
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
            selection.repair_actions,
            ["imported repair 0"],
            "compatibility fallback repairs must survive common selection"
        );
        assert_eq!(
            recipe.repairs.get(),
            CANDIDATE_COUNT.saturating_mul(MAX_REPAIR_ROUNDS)
        );
        assert_eq!(
            recipe.validations.borrow().last(),
            Some(&ValidationProvenance::Fallback),
            "fallback validation must not masquerade as candidate 0"
        );
    }

    #[test]
    fn an_invalid_fallback_is_an_error_not_an_empty_plan() {
        let error = run_recipe(
            &MockRecipe::default(),
            &MockSettings {
                force_fallback: true,
                invalid_fallback: true,
                ..MockSettings::default()
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
    fn common_volume_failure_rejects_fallback_before_recipe_validation() {
        let recipe = MockRecipe::default();
        let error = run_recipe(
            &recipe,
            &MockSettings {
                force_fallback: true,
                invalid_fallback_volume: true,
                ..MockSettings::default()
            },
            12,
            99,
        )
        .expect_err("a malformed canonical fallback must never publish");

        assert!(matches!(&error, V2GenerationError::InvalidFallback(_)));
        assert!(
            error.to_string().contains("volume footprint"),
            "the common volume issue should be preserved: {error}"
        );
        assert!(
            !recipe
                .validations
                .borrow()
                .contains(&ValidationProvenance::Fallback),
            "recipe validation must not derive metrics from a common-invalid volume"
        );
    }

    #[test]
    fn canonical_fallback_output_does_not_depend_on_the_requested_seed() {
        let settings = MockSettings {
            force_fallback: true,
            ..MockSettings::default()
        };
        let first = run_recipe(&MockRecipe::default(), &settings, 12, 11)
            .expect("the first fallback should pass");
        let second = run_recipe(&MockRecipe::default(), &settings, 12, 98_765)
            .expect("the second fallback should pass");
        assert_eq!(first.plan.metadata, second.plan.metadata);

        let first = materialize_selection(first, &palette(), &test_is_solid)
            .expect("the first fallback should materialize");
        let second = materialize_selection(second, &palette(), &test_is_solid)
            .expect("the second fallback should materialize");
        assert_eq!(first.map_fingerprint, second.map_fingerprint);
    }

    #[test]
    fn materialization_publishes_exact_plan_outputs_and_preserves_v1_identity() {
        let selection = run_recipe(&MockRecipe::default(), &MockSettings::default(), 12, 77)
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
    fn compatibility_import_rechecks_common_volume() {
        let validated = run_recipe(&MockRecipe::default(), &MockSettings::default(), 12, 77)
            .expect("the source selection should be valid");
        let mut changed = validated.into_unvalidated();
        changed.plan.volume.columns.remove(&HexCoord::ORIGIN);

        let error = ValidatedRecipeSelection::from_compatibility_import(changed)
            .expect_err("a compatibility import must re-enter common validation");
        assert!(
            error.to_string().contains("volume footprint"),
            "the common validation issue should be preserved: {error}"
        );
    }

    #[test]
    fn materialized_fingerprint_orders_and_includes_exact_interior_semantics() {
        let selection = run_recipe(&MockRecipe::default(), &MockSettings::default(), 12, 77)
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
