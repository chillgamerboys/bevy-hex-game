//! Deterministic schematic candidate construction and selection.
//!
//! Random-looking choices are keyed samples rather than a mutable pseudo-random
//! cursor. A sample therefore depends only on the world seed, candidate, named
//! stage, exact cell, and local ordinal. Adding a woodland choice cannot shift a
//! coast or hydrology result.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use crate::fingerprint::semantic_fingerprint;
use crate::metrics::SchematicMetricsV1;
use crate::model::{
    BoundedRegionKind, BoundedRegionRule, BoundedTarget, CellPlan, FeatureKind, LayerProvenance,
    OverlayProvenance, PlanProvenance, SchematicCoord, SchematicPlanParts, SchematicPlanV1,
    SchematicTemplateV1, StableId,
};
use crate::validate::{validate_plan_draft, validate_template, ValidationError};

/// Exact number of constructive candidates evaluated for every world seed.
pub const CANDIDATE_ATTEMPTS: u8 = 32;

const STREAM_DOMAIN: u64 = 0x7363_6865_6d61_7631;
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// One validated generated plan and its exact recomputed metrics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedSchematic {
    /// Authoritative strict plan.
    pub plan: SchematicPlanV1,
    /// Metrics recomputed by the same hard validator which accepted `plan`.
    pub metrics: SchematicMetricsV1,
}

/// Fail-closed schematic generation error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenerationError {
    /// The input template failed the complete hard validator.
    InvalidTemplate(ValidationError),
    /// A checked model constructor rejected generated data.
    Model(String),
    /// Every normal candidate failed and the independent reference fallback
    /// also failed validation.
    InvalidFallback(ValidationError),
}

impl fmt::Display for GenerationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTemplate(error) => {
                write!(formatter, "invalid schematic template: {error}")
            }
            Self::Model(detail) => write!(formatter, "cannot construct schematic plan: {detail}"),
            Self::InvalidFallback(error) => {
                write!(
                    formatter,
                    "no hard-valid candidate and reference fallback failed: {error}"
                )
            }
        }
    }
}

impl std::error::Error for GenerationError {}

/// Constructs exactly 32 deterministic candidates, retains the hard-valid
/// subset, prefers foundations which preserve the complete configured island
/// group range, then selects by a vegetation-independent semantic score and
/// validates the selected plan.
/// Validation and compound coastline preflight use one bounded exact-template
/// cache; candidate construction and output remain seed-pure.
/// The separately marked reference fallback is used only if no candidate can
/// be accepted.
pub fn generate(
    template: &SchematicTemplateV1,
    world_seed: u64,
) -> Result<GeneratedSchematic, GenerationError> {
    generate_internal(template, world_seed, false, StreamPerturbations::default())
}

/// Builds and independently validates the canonical reference artifact used by
/// the approval pack. Its facts and original authorship come directly from the
/// template; it is distinct from the relabeled fallback used after exhaustion.
pub fn reference_plan(
    template: &SchematicTemplateV1,
    world_seed: u64,
) -> Result<GeneratedSchematic, GenerationError> {
    validate_template(template).map_err(GenerationError::InvalidTemplate)?;
    build_reference_plan(template, PlanProvenance::reference_artifact(world_seed))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct StreamPerturbations {
    islands: u64,
    vegetation: u64,
}

/// Stateless namespace for one candidate's independent generation stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NamedSamples {
    world_seed: u64,
    candidate: u8,
    perturbations: StreamPerturbations,
}

impl NamedSamples {
    const fn new(world_seed: u64, candidate: u8) -> Self {
        Self {
            world_seed,
            candidate,
            perturbations: StreamPerturbations {
                islands: 0,
                vegetation: 0,
            },
        }
    }

    const fn with_perturbations(mut self, perturbations: StreamPerturbations) -> Self {
        self.perturbations = perturbations;
        self
    }

    fn sample(self, stage: &str, cell: Option<(i32, i32, i32)>, ordinal: u32) -> u64 {
        let mut state = FNV_OFFSET;
        state = fold_bytes(state, &STREAM_DOMAIN.to_le_bytes());
        state = fold_bytes(state, &self.world_seed.to_le_bytes());
        state = fold_bytes(state, &[self.candidate]);
        state = fold_bytes(state, stage.as_bytes());
        let perturbation = match stage {
            "stream/islands" => self.perturbations.islands,
            "stream/vegetation" => self.perturbations.vegetation,
            _ => 0,
        };
        // Preserve the established stream sequence when no test perturbation is
        // requested. A nonzero perturbation is deliberately folded only into
        // its named stage so tests can prove that later stages cannot move an
        // earlier decision.
        if perturbation != 0 {
            state = fold_bytes(state, &perturbation.to_le_bytes());
        }
        if let Some((q, r, s)) = cell {
            state = fold_bytes(state, &q.to_le_bytes());
            state = fold_bytes(state, &r.to_le_bytes());
            state = fold_bytes(state, &s.to_le_bytes());
        }
        state = fold_bytes(state, &ordinal.to_le_bytes());
        avalanche(state)
    }

    fn bounded(
        self,
        stage: &str,
        cell: Option<(i32, i32, i32)>,
        ordinal: u32,
        lower: u32,
        upper: u32,
    ) -> u32 {
        let width = upper.saturating_sub(lower).saturating_add(1);
        if width == 0 {
            return lower;
        }
        let offset = self.sample(stage, cell, ordinal) % u64::from(width);
        lower.saturating_add(u32::try_from(offset).unwrap_or(0))
    }

    fn ranked<'a, T: Copy + Ord + 'a>(
        self,
        stage: &'a str,
        ordinal: u32,
        cells: impl IntoIterator<Item = (T, (i32, i32, i32))> + 'a,
    ) -> Vec<T> {
        let mut ranked = cells
            .into_iter()
            .map(|(cell, key)| (self.sample(stage, Some(key), ordinal), cell))
            .collect::<Vec<_>>();
        ranked.sort_unstable();
        ranked.into_iter().map(|(_, cell)| cell).collect()
    }
}

fn generate_internal(
    template: &SchematicTemplateV1,
    world_seed: u64,
    force_candidate_failure: bool,
    perturbations: StreamPerturbations,
) -> Result<GeneratedSchematic, GenerationError> {
    let coastline_moves =
        validated_template_preflight(template).map_err(GenerationError::InvalidTemplate)?;
    let mut accepted = if force_candidate_failure {
        Vec::new()
    } else {
        evaluate_candidates_parallel(
            template,
            world_seed,
            perturbations,
            coastline_moves.as_ref(),
        )?
    };

    let hard_valid_candidates = u8::try_from(accepted.len()).unwrap_or(CANDIDATE_ATTEMPTS);
    accepted.sort_unstable_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    // Preserve the established semantic score order within each structural
    // capacity. Stop at the first full-range candidate; if none exists, retain
    // the first (therefore best-scored) candidate at the highest exact capacity.
    let maximum_island_groups = template
        .bounded_regions
        .iter()
        .find(|rule| rule.kind == BoundedRegionKind::SeaIslands)
        .map(|rule| rule.components.max)
        .unwrap_or(0);
    let mut selected_index = 0;
    let mut selected_capacity = 0;
    for (index, (_, _, _, plan)) in accepted.iter().enumerate() {
        let capacity = sea_island_group_capacity(template, &plan.cells, &plan.networks);
        if capacity > selected_capacity {
            selected_index = index;
            selected_capacity = capacity;
        }
        if capacity == maximum_island_groups {
            selected_index = index;
            break;
        }
    }
    if !accepted.is_empty() {
        let (_, _, candidate, plan) = accepted.swap_remove(selected_index);
        let mut parts = plan.into_parts();
        let samples = NamedSamples::new(world_seed, candidate).with_perturbations(perturbations);
        let islands_applied = apply_island_stage(
            template,
            &mut parts.cells,
            &parts.networks,
            samples,
            SelectionMode::Seeded,
        );
        let woodland_applied =
            apply_woodland_stage(template, &mut parts.cells, samples, SelectionMode::Seeded);
        if !islands_applied || !woodland_applied {
            return Err(GenerationError::Model(
                "selected candidate lost its validated canonical island or woodland witness"
                    .to_owned(),
            ));
        }
        parts.provenance = PlanProvenance::candidate(world_seed, candidate, hard_valid_candidates)
            .map_err(|error| GenerationError::Model(error.to_string()))?;
        parts.semantic_fingerprint = 0;
        let mut plan = SchematicPlanV1::new(parts)
            .map_err(|error| GenerationError::Model(error.to_string()))?;
        plan.semantic_fingerprint = semantic_fingerprint(&plan);
        let metrics =
            validate_plan_draft(template, &plan).map_err(GenerationError::InvalidFallback)?;
        return Ok(GeneratedSchematic { plan, metrics });
    }

    build_reference_plan(template, PlanProvenance::reference_fallback(world_seed))
}

type AcceptedCandidate = (u32, u64, u8, SchematicPlanV1);

fn evaluate_candidate(
    template: &SchematicTemplateV1,
    world_seed: u64,
    candidate: u8,
    perturbations: StreamPerturbations,
    coastline_moves: &[CoastlineMove],
) -> Result<Option<AcceptedCandidate>, GenerationError> {
    let samples = NamedSamples::new(world_seed, candidate).with_perturbations(perturbations);
    let (mut varied_cells, networks) =
        construct_candidate_foundation(template, samples, coastline_moves);
    if !apply_island_stage(
        template,
        &mut varied_cells,
        &networks,
        samples,
        SelectionMode::Canonical,
    ) || !apply_woodland_stage(
        template,
        &mut varied_cells,
        samples,
        SelectionMode::Canonical,
    ) {
        return Ok(None);
    }
    let candidate_plan = finish_plan(
        template,
        PlanProvenance::candidate(world_seed, candidate, 1)
            .map_err(|error| GenerationError::Model(error.to_string()))?,
        varied_cells,
        template.fixed_claims.clone(),
        networks,
    )?;
    if validate_plan_draft(template, &candidate_plan).is_err() {
        return Ok(None);
    }
    Ok(Some((
        candidate_quality(template, &candidate_plan),
        non_vegetation_semantic_fingerprint(&candidate_plan),
        candidate,
        candidate_plan,
    )))
}

fn evaluate_candidates_parallel(
    template: &SchematicTemplateV1,
    world_seed: u64,
    perturbations: StreamPerturbations,
    coastline_moves: &[CoastlineMove],
) -> Result<Vec<AcceptedCandidate>, GenerationError> {
    let worker_count = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(4)
        .min(usize::from(CANDIDATE_ATTEMPTS));
    evaluate_candidates_with_worker_count(
        template,
        world_seed,
        perturbations,
        coastline_moves,
        worker_count,
    )
}

fn evaluate_candidates_with_worker_count(
    template: &SchematicTemplateV1,
    world_seed: u64,
    perturbations: StreamPerturbations,
    coastline_moves: &[CoastlineMove],
    worker_count: usize,
) -> Result<Vec<AcceptedCandidate>, GenerationError> {
    let worker_count = worker_count.max(1).min(usize::from(CANDIDATE_ATTEMPTS));
    if worker_count <= 1 {
        return (0..CANDIDATE_ATTEMPTS).try_fold(
            Vec::with_capacity(usize::from(CANDIDATE_ATTEMPTS)),
            |mut accepted, candidate| {
                if let Some(result) = evaluate_candidate(
                    template,
                    world_seed,
                    candidate,
                    perturbations,
                    coastline_moves,
                )? {
                    accepted.push(result);
                }
                Ok(accepted)
            },
        );
    }

    std::thread::scope(|scope| {
        let mut workers = Vec::with_capacity(worker_count);
        for worker in 0..worker_count {
            workers.push(scope.spawn(move || {
                let mut accepted =
                    Vec::with_capacity(usize::from(CANDIDATE_ATTEMPTS).div_ceil(worker_count));
                for candidate in (worker..usize::from(CANDIDATE_ATTEMPTS)).step_by(worker_count) {
                    let candidate = u8::try_from(candidate).map_err(|error| {
                        GenerationError::Model(format!(
                            "candidate index does not fit the schema: {error}"
                        ))
                    })?;
                    if let Some(result) = evaluate_candidate(
                        template,
                        world_seed,
                        candidate,
                        perturbations,
                        coastline_moves,
                    )? {
                        accepted.push(result);
                    }
                }
                Ok::<_, GenerationError>(accepted)
            }));
        }

        let mut accepted = Vec::with_capacity(usize::from(CANDIDATE_ATTEMPTS));
        for worker in workers {
            let mut results = worker.join().map_err(|_panic| {
                GenerationError::Model("schematic candidate worker panicked".to_owned())
            })??;
            accepted.append(&mut results);
        }
        accepted.sort_unstable_by_key(|result| result.2);
        Ok(accepted)
    })
}

fn reconcile_final_coastline(template: &SchematicTemplateV1, cells: &mut [CellPlan]) {
    if let Some(stream_id) = template
        .generation
        .named_streams
        .iter()
        .find(|stream| stream.as_str() == "stream/coastline")
    {
        reconcile_coastline_overlay(template, cells, stream_id);
    }
}

fn build_reference_plan(
    template: &SchematicTemplateV1,
    provenance: PlanProvenance,
) -> Result<GeneratedSchematic, GenerationError> {
    let (reference_cells, fixed_features) = if provenance.is_reference_artifact {
        (
            template.reference_cells.clone(),
            template.fixed_claims.clone(),
        )
    } else {
        let cells = template
            .reference_cells
            .iter()
            .cloned()
            .map(reference_fallback_cell)
            .collect();
        let features = template
            .fixed_claims
            .iter()
            .cloned()
            .map(|mut feature| {
                feature.provenance = LayerProvenance::ReferenceFallback {
                    source: feature.id.clone(),
                };
                feature
            })
            .collect();
        (cells, features)
    };
    let plan = finish_plan(
        template,
        provenance,
        reference_cells,
        fixed_features,
        template.networks.clone(),
    )?;
    let metrics = validate_plan_draft(template, &plan).map_err(GenerationError::InvalidFallback)?;
    Ok(GeneratedSchematic { plan, metrics })
}

fn finish_plan(
    template: &SchematicTemplateV1,
    provenance: PlanProvenance,
    cells: Vec<CellPlan>,
    features: Vec<crate::model::FeatureClaim>,
    networks: Vec<crate::model::Network>,
) -> Result<SchematicPlanV1, GenerationError> {
    let mut plan = SchematicPlanV1::new(SchematicPlanParts {
        template_id: template.id.clone(),
        template_revision: template.revision,
        provenance,
        cells,
        features,
        networks,
        semantic_fingerprint: 0,
    })
    .map_err(|error| GenerationError::Model(error.to_string()))?;
    plan.semantic_fingerprint = semantic_fingerprint(&plan);
    Ok(plan)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionMode {
    Canonical,
    Seeded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CoastlineMove {
    coord: SchematicCoord,
    repair: Option<LandformRepair>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LandformRepair {
    coord: SchematicCoord,
    landform: crate::model::LandformKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TemplatePreflight {
    template: SchematicTemplateV1,
    coastline_moves: Arc<[CoastlineMove]>,
}

static TEMPLATE_PREFLIGHT: OnceLock<Mutex<Option<TemplatePreflight>>> = OnceLock::new();

fn template_preflight_cache() -> &'static Mutex<Option<TemplatePreflight>> {
    TEMPLATE_PREFLIGHT.get_or_init(|| Mutex::new(None))
}

fn lock_template_preflight(
    cache: &Mutex<Option<TemplatePreflight>>,
) -> MutexGuard<'_, Option<TemplatePreflight>> {
    match cache.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            *guard = None;
            cache.clear_poison();
            guard
        }
    }
}

fn validated_template_preflight(
    template: &SchematicTemplateV1,
) -> Result<Arc<[CoastlineMove]>, ValidationError> {
    validated_template_preflight_with_cache(template, template_preflight_cache())
}

fn validated_template_preflight_with_cache(
    template: &SchematicTemplateV1,
    cache: &Mutex<Option<TemplatePreflight>>,
) -> Result<Arc<[CoastlineMove]>, ValidationError> {
    {
        let guard = lock_template_preflight(cache);
        if let Some(cached) = guard.as_ref().filter(|cached| cached.template == *template) {
            return Ok(Arc::clone(&cached.coastline_moves));
        }
    }

    validate_template(template)?;
    let coastline_moves = Arc::<[CoastlineMove]>::from(valid_coastline_moves(template));
    let preflight = TemplatePreflight {
        template: template.clone(),
        coastline_moves: Arc::clone(&coastline_moves),
    };
    let mut guard = lock_template_preflight(cache);
    if let Some(cached) = guard.as_ref().filter(|cached| cached.template == *template) {
        return Ok(Arc::clone(&cached.coastline_moves));
    }
    *guard = Some(preflight);
    Ok(coastline_moves)
}

/// Builds the coast, hydrology, and landform foundation used to decide which
/// candidates are eligible. Islands and woodland are completed separately so
/// their random samples cannot affect candidate selection.
fn construct_candidate_foundation(
    template: &SchematicTemplateV1,
    samples: NamedSamples,
    coastline_moves: &[CoastlineMove],
) -> (Vec<CellPlan>, Vec<crate::model::Network>) {
    let mut cells = template.reference_cells.clone();
    vary_coastline(template, &mut cells, samples, coastline_moves);
    apply_generation_stage(
        template,
        &mut cells,
        samples,
        "stream/hydrology",
        &BTreeSet::new(),
    );
    let networks = resolve_hydrology(template, &mut cells, samples);
    reconcile_final_coastline(template, &mut cells);

    let hydrology = established_hydrology_cells(&cells, &networks);
    apply_generation_stage(
        template,
        &mut cells,
        samples,
        "stream/landforms",
        &hydrology,
    );
    (cells, networks)
}

fn apply_generation_stage(
    template: &SchematicTemplateV1,
    cells: &mut [CellPlan],
    samples: NamedSamples,
    stage: &str,
    protected: &BTreeSet<SchematicCoord>,
) {
    let variation_lane = samples.candidate % 4;
    if (stage == "stream/hydrology" && matches!(variation_lane, 0 | 2))
        || (stage == "stream/landforms" && matches!(variation_lane, 0 | 1))
    {
        return;
    }
    let Some(stream_id) = template
        .generation
        .named_streams
        .iter()
        .find(|stream_id| stream_id.as_str() == stage)
    else {
        return;
    };
    let mut rules = template
        .bounded_regions
        .iter()
        .filter(|rule| {
            generation_stage(rule) == stage
                && !rule
                    .targets
                    .contains(&BoundedTarget::Overlay(FeatureKind::River))
        })
        .enumerate()
        .collect::<Vec<_>>();
    if stage == "stream/landforms" {
        rules.sort_unstable_by_key(|(ordinal, rule)| {
            (
                samples.sample(
                    stage,
                    None,
                    4_096_u32.saturating_add(u32::try_from(*ordinal).unwrap_or(u32::MAX)),
                ),
                &rule.id,
            )
        });
    }
    for (ordinal, rule) in rules {
        let ordinal = u32::try_from(ordinal).unwrap_or(u32::MAX);
        let selected = vary_region(template, rule, samples, stage, ordinal, protected);
        apply_region(
            template, cells, rule, &selected, stage, stream_id, samples, ordinal, protected,
        );
        if stage == "stream/landforms" {
            break;
        }
    }
}

fn established_hydrology_cells(
    cells: &[CellPlan],
    networks: &[crate::model::Network],
) -> BTreeSet<SchematicCoord> {
    let mut protected = networks
        .iter()
        .filter(|network| network.kind == crate::model::NetworkKind::Hydrology)
        .flat_map(|network| &network.edges)
        .flat_map(|edge| edge.path.iter().copied())
        .collect::<BTreeSet<_>>();
    protected.extend(
        cells
            .iter()
            .filter(|cell| has_hydrology_overlay(cell))
            .map(|cell| cell.coord),
    );
    protected
}

fn apply_island_stage(
    template: &SchematicTemplateV1,
    cells: &mut [CellPlan],
    networks: &[crate::model::Network],
    samples: NamedSamples,
    mode: SelectionMode,
) -> bool {
    let Some((ordinal, rule)) = template
        .bounded_regions
        .iter()
        .enumerate()
        .find(|(_, rule)| rule.kind == BoundedRegionKind::SeaIslands)
    else {
        return false;
    };
    let Some(stream_id) = template
        .generation
        .named_streams
        .iter()
        .find(|stream| stream.as_str() == "stream/islands")
    else {
        return false;
    };
    let ordinal = u32::try_from(ordinal).unwrap_or(u32::MAX);
    let original = cells.to_vec();
    let modes = match mode {
        SelectionMode::Canonical => [Some(SelectionMode::Canonical), None],
        SelectionMode::Seeded => [Some(SelectionMode::Seeded), Some(SelectionMode::Canonical)],
    };
    for attempt_mode in modes.into_iter().flatten() {
        let Some(selected) =
            vary_sea_islands(&original, networks, rule, samples, ordinal, attempt_mode)
        else {
            continue;
        };
        let mut trial = original.clone();
        apply_region(
            template,
            &mut trial,
            rule,
            &selected,
            "stream/islands",
            stream_id,
            samples,
            ordinal,
            &BTreeSet::new(),
        );
        if connected_sea(&trial)
            && connected_mainland(&trial)
            && bounded_shapes_are_valid(template, &trial)
        {
            cells.clone_from_slice(&trial);
            return true;
        }
    }
    false
}

fn vary_sea_islands(
    cells: &[CellPlan],
    networks: &[crate::model::Network],
    rule: &BoundedRegionRule,
    samples: NamedSamples,
    ordinal: u32,
    mode: SelectionMode,
) -> Option<BTreeSet<SchematicCoord>> {
    let safe = safe_sea_island_cells(cells, networks, rule);
    let desired_groups = match mode {
        SelectionMode::Canonical => 2,
        SelectionMode::Seeded => usize::try_from(samples.bounded(
            "stream/islands",
            None,
            ordinal.saturating_add(2_048),
            2,
            6,
        ))
        .unwrap_or(2),
    };
    let seed_candidates = ranked_coords(
        mode,
        samples,
        "stream/islands",
        ordinal.saturating_add(2_049),
        safe.iter().copied().collect(),
    );
    let seeds = separated_seed_packing(&seed_candidates, desired_groups)?;
    let mut groups = seeds
        .iter()
        .copied()
        .map(|seed| BTreeSet::from([seed]))
        .collect::<Vec<_>>();
    let mut selected = seeds.iter().copied().collect::<BTreeSet<_>>();

    for (group_index, group) in groups.iter_mut().enumerate() {
        let seed = group.first().copied()?;
        let group_ordinal = u32::try_from(group_index).unwrap_or(u32::MAX);
        let desired_size = match mode {
            SelectionMode::Canonical => 1,
            SelectionMode::Seeded => usize::try_from(samples.bounded(
                "stream/islands",
                Some((seed.q(), seed.r(), seed.s())),
                ordinal.saturating_add(2_128),
                1,
                4,
            ))
            .unwrap_or(1),
        };
        while group.len() < desired_size {
            let addition_candidates = safe
                .iter()
                .copied()
                .filter(|coord| !selected.contains(coord))
                .filter(|coord| {
                    schematic_neighbors(*coord)
                        .into_iter()
                        .any(|neighbor| group.contains(&neighbor))
                })
                .filter(|coord| {
                    schematic_neighbors(*coord)
                        .into_iter()
                        .all(|neighbor| !selected.contains(&neighbor) || group.contains(&neighbor))
                })
                .collect::<Vec<_>>();
            let additions = ranked_coords(
                mode,
                samples,
                "stream/islands",
                ordinal.saturating_add(2_256).saturating_add(group_ordinal),
                addition_candidates,
            );
            let Some(next) = additions.first().copied() else {
                break;
            };
            group.insert(next);
            selected.insert(next);
        }
    }

    if shape_is_valid(&selected, rule) {
        Some(selected)
    } else {
        None
    }
}

fn safe_sea_island_cells(
    cells: &[CellPlan],
    networks: &[crate::model::Network],
    rule: &BoundedRegionRule,
) -> BTreeSet<SchematicCoord> {
    let network_paths = networks
        .iter()
        .flat_map(|network| &network.edges)
        .flat_map(|edge| edge.path.iter().copied())
        .collect::<BTreeSet<_>>();
    rule.envelope
        .iter()
        .copied()
        .filter(|coord| !network_paths.contains(coord))
        .filter(|coord| {
            crate::model::canonical_coordinate_index(*coord)
                .and_then(|index| cells.get(index))
                .is_some_and(|cell| {
                    is_sea_cell(cell) || cell.facts.overlays.contains(&FeatureKind::SeaIsland)
                })
        })
        .filter(|coord| {
            schematic_neighbors(*coord).into_iter().all(|neighbor| {
                crate::model::canonical_coordinate_index(neighbor)
                    .and_then(|index| cells.get(index))
                    .is_none_or(|cell| {
                        cell.facts.surface != crate::model::SurfaceKind::Land
                            || cell.facts.overlays.contains(&FeatureKind::SeaIsland)
                    })
            })
        })
        .collect()
}

/// Returns the largest configured component count for which this foundation
/// has an exact separated seed packing.
fn sea_island_group_capacity(
    template: &SchematicTemplateV1,
    cells: &[CellPlan],
    networks: &[crate::model::Network],
) -> u16 {
    let Some(rule) = template
        .bounded_regions
        .iter()
        .find(|rule| rule.kind == BoundedRegionKind::SeaIslands)
    else {
        return 0;
    };
    let mut ranked = safe_sea_island_cells(cells, networks, rule)
        .into_iter()
        .collect::<Vec<_>>();
    ranked.sort_unstable_by_key(|coord| {
        crate::model::canonical_coordinate_index(*coord).unwrap_or(usize::MAX)
    });
    (rule.components.min..=rule.components.max)
        .rev()
        .find(|desired| {
            separated_seed_packing(&ranked, usize::from(*desired)).is_some_and(|seeds| {
                let selected = seeds.into_iter().collect::<BTreeSet<_>>();
                shape_is_valid(&selected, rule)
            })
        })
        .unwrap_or(0)
}

/// Selects the lexicographically first independent set under the caller's
/// deterministic ranking. Unlike greedy seed placement, bounded backtracking
/// cannot strand a feasible sixth one-cell island behind an early local choice.
fn separated_seed_packing(
    ranked: &[SchematicCoord],
    desired: usize,
) -> Option<Vec<SchematicCoord>> {
    if desired == 0 {
        return Some(Vec::new());
    }
    if ranked.len() <= u128::BITS as usize {
        let adjacency = ranked
            .iter()
            .map(|candidate| {
                ranked
                    .iter()
                    .enumerate()
                    .fold(0_u128, |mask, (index, other)| {
                        let adjacent = schematic_neighbors(*candidate)
                            .into_iter()
                            .any(|neighbor| neighbor == *other);
                        mask | (u128::from(adjacent) << index)
                    })
            })
            .collect::<Vec<_>>();
        let candidates = if ranked.len() == u128::BITS as usize {
            u128::MAX
        } else {
            (1_u128 << ranked.len()).saturating_sub(1)
        };
        let mut selected = Vec::with_capacity(desired);
        let mut dead = BTreeSet::new();
        if bitset_independent_set(candidates, desired, &adjacency, &mut selected, &mut dead) {
            return selected
                .into_iter()
                .map(|index| ranked.get(index).copied())
                .collect();
        }
        return None;
    }
    vector_seed_packing(ranked, desired)
}

fn vector_seed_packing(ranked: &[SchematicCoord], desired: usize) -> Option<Vec<SchematicCoord>> {
    fn search(
        candidates: &[SchematicCoord],
        needed: usize,
        selected: &mut Vec<SchematicCoord>,
    ) -> bool {
        if needed == 0 {
            return true;
        }
        if candidates.len() < needed {
            return false;
        }
        for index in 0..=candidates.len().saturating_sub(needed) {
            let Some(candidate) = candidates.get(index).copied() else {
                return false;
            };
            selected.push(candidate);
            let Some(tail) = candidates.get(index.saturating_add(1)..) else {
                selected.pop();
                return false;
            };
            let remaining = tail
                .iter()
                .copied()
                .filter(|other| {
                    !schematic_neighbors(candidate)
                        .into_iter()
                        .any(|neighbor| neighbor == *other)
                })
                .collect::<Vec<_>>();
            if search(&remaining, needed.saturating_sub(1), selected) {
                return true;
            }
            selected.pop();
        }
        false
    }

    if desired == 0 {
        return Some(Vec::new());
    }
    let mut selected = Vec::with_capacity(desired);
    search(ranked, desired, &mut selected).then_some(selected)
}

fn bitset_independent_set(
    candidates: u128,
    needed: usize,
    adjacency: &[u128],
    selected: &mut Vec<usize>,
    dead: &mut BTreeSet<(u128, usize)>,
) -> bool {
    if needed == 0 {
        return true;
    }
    if candidates.count_ones() < u32::try_from(needed).unwrap_or(u32::MAX)
        || independent_set_upper_bound(candidates, adjacency) < needed
        || dead.contains(&(candidates, needed))
    {
        return false;
    }

    let mut remaining = candidates;
    while remaining.count_ones() >= u32::try_from(needed).unwrap_or(u32::MAX) {
        let index = usize::try_from(remaining.trailing_zeros()).unwrap_or(usize::MAX);
        let bit = 1_u128 << index;
        remaining &= !bit;
        selected.push(index);
        let Some(conflicts) = adjacency.get(index).copied() else {
            selected.pop();
            return false;
        };
        if bitset_independent_set(
            remaining & !conflicts,
            needed.saturating_sub(1),
            adjacency,
            selected,
            dead,
        ) {
            return true;
        }
        selected.pop();
    }
    dead.insert((candidates, needed));
    false
}

/// A clique cover of the conflict graph is an upper bound on the number of
/// pairwise nonadjacent cells: an independent set can take at most one member
/// from each clique. The greedy cover is cheap and makes impossible six-group
/// proofs terminate before enumerating their coordinate combinations.
fn independent_set_upper_bound(candidates: u128, adjacency: &[u128]) -> usize {
    let mut cliques = Vec::<u128>::new();
    let mut remaining = candidates;
    while remaining != 0 {
        let index = usize::try_from(remaining.trailing_zeros()).unwrap_or(usize::MAX);
        let bit = 1_u128 << index;
        remaining &= !bit;
        let conflicts = adjacency.get(index).copied().unwrap_or(u128::MAX);
        if let Some(clique) = cliques.iter_mut().find(|clique| **clique & !conflicts == 0) {
            *clique |= bit;
        } else {
            cliques.push(bit);
        }
    }
    cliques.len()
}

fn ranked_coords(
    mode: SelectionMode,
    samples: NamedSamples,
    stage: &str,
    ordinal: u32,
    mut cells: Vec<SchematicCoord>,
) -> Vec<SchematicCoord> {
    match mode {
        SelectionMode::Canonical => {
            cells.sort_unstable_by_key(|coord| {
                crate::model::canonical_coordinate_index(*coord).unwrap_or(usize::MAX)
            });
            cells
        }
        SelectionMode::Seeded => samples.ranked(stage, ordinal, cells.into_iter().map(coord_key)),
    }
}

fn apply_woodland_stage(
    template: &SchematicTemplateV1,
    cells: &mut [CellPlan],
    samples: NamedSamples,
    mode: SelectionMode,
) -> bool {
    let Some((ordinal, rule)) = template
        .bounded_regions
        .iter()
        .enumerate()
        .find(|(_, rule)| rule.kind == BoundedRegionKind::Woodland)
    else {
        return false;
    };
    let Some(stream_id) = template
        .generation
        .named_streams
        .iter()
        .find(|stream| stream.as_str() == "stream/vegetation")
    else {
        return false;
    };
    let ordinal = u32::try_from(ordinal).unwrap_or(u32::MAX);
    let mut selected = vary_woodland(cells, rule, samples, ordinal, mode);
    if selected.is_none() && mode == SelectionMode::Seeded {
        selected = vary_woodland(cells, rule, samples, ordinal, SelectionMode::Canonical);
    }
    let Some(selected) = selected else {
        return false;
    };
    apply_region(
        template,
        cells,
        rule,
        &selected,
        "stream/vegetation",
        stream_id,
        samples,
        ordinal,
        &BTreeSet::new(),
    );
    true
}

fn vary_woodland(
    cells: &[CellPlan],
    rule: &BoundedRegionRule,
    samples: NamedSamples,
    ordinal: u32,
    mode: SelectionMode,
) -> Option<BTreeSet<SchematicCoord>> {
    let eligible = rule
        .envelope
        .iter()
        .copied()
        .filter(|coord| {
            crate::model::canonical_coordinate_index(*coord)
                .and_then(|index| cells.get(index))
                .is_some_and(|cell| {
                    cell.facts.surface == crate::model::SurfaceKind::Land
                        && matches!(
                            cell.facts.landform,
                            crate::model::LandformKind::Hill | crate::model::LandformKind::Valley
                        )
                        && !cell.facts.overlays.contains(&FeatureKind::FrozenWoods)
                        && !target_is_locked(cell, BoundedTarget::Vegetated)
                })
        })
        .collect::<BTreeSet<_>>();
    let components = component_sets(&eligible);
    let lower = usize::from(rule.count.min.max(rule.component_size.min));
    let upper = components
        .iter()
        .map(BTreeSet::len)
        .max()
        .unwrap_or(0)
        .min(usize::from(rule.count.max))
        .min(usize::from(rule.component_size.max));
    if lower > upper {
        return None;
    }
    let desired = match mode {
        SelectionMode::Canonical => lower,
        SelectionMode::Seeded => usize::try_from(samples.bounded(
            "stream/vegetation",
            None,
            ordinal.saturating_add(3_072),
            u32::try_from(lower).unwrap_or(u32::MAX),
            u32::try_from(upper).unwrap_or(u32::MAX),
        ))
        .unwrap_or(lower),
    };
    let seeds = ranked_coords(
        mode,
        samples,
        "stream/vegetation",
        ordinal.saturating_add(3_073),
        eligible.iter().copied().collect(),
    );
    for seed in seeds {
        let Some(component) = components
            .iter()
            .find(|component| component.contains(&seed) && component.len() >= desired)
        else {
            continue;
        };
        let mut selected = BTreeSet::from([seed]);
        while selected.len() < desired {
            let additions = component
                .difference(&selected)
                .copied()
                .filter(|coord| {
                    schematic_neighbors(*coord)
                        .into_iter()
                        .any(|neighbor| selected.contains(&neighbor))
                })
                .collect::<Vec<_>>();
            let ranked = ranked_coords(
                mode,
                samples,
                "stream/vegetation",
                ordinal
                    .saturating_add(3_200)
                    .saturating_add(u32::try_from(selected.len()).unwrap_or(u32::MAX)),
                additions,
            );
            let Some(next) = ranked.first().copied() else {
                break;
            };
            selected.insert(next);
        }
        if shape_is_valid(&selected, rule) {
            return Some(selected);
        }
    }
    None
}

fn vary_coastline(
    template: &SchematicTemplateV1,
    cells: &mut [CellPlan],
    samples: NamedSamples,
    valid_moves: &[CoastlineMove],
) {
    let ranked = samples.ranked(
        "stream/coastline",
        0,
        valid_moves
            .iter()
            .map(|candidate| coord_key(candidate.coord)),
    );
    let Some(coord) = ranked.first().copied() else {
        return;
    };
    let Some(selected_move) = valid_moves
        .iter()
        .find(|candidate| candidate.coord == coord)
    else {
        return;
    };
    let mut trial = cells.to_vec();
    if apply_preflighted_coastline_move(template, &mut trial, *selected_move) {
        cells.clone_from_slice(&trial);
    }
}

/// Preflights each compound coast move once, including its optional minimal
/// landform repair. Candidate construction ranks the compact results by their
/// coast coordinates and reapplies the recorded repair directly.
fn valid_coastline_moves(template: &SchematicTemplateV1) -> Vec<CoastlineMove> {
    let Some(rule) = template
        .bounded_regions
        .iter()
        .find(|rule| rule.kind == BoundedRegionKind::Coastline)
    else {
        return Vec::new();
    };
    let Some(stream_id) = template
        .generation
        .named_streams
        .iter()
        .find(|stream| stream.as_str() == "stream/coastline")
    else {
        return Vec::new();
    };
    let envelope = rule.envelope.iter().copied().collect::<BTreeSet<_>>();
    rule.envelope
        .iter()
        .copied()
        .filter(|coord| {
            template.cell(*coord).is_some_and(|reference| {
                !cell_has_locked_fact(reference)
                    && !is_network_path_cell(template, *coord)
                    && !has_hydrology_overlay(reference)
            })
        })
        .filter_map(|coord| {
            let mut trial = template.reference_cells.clone();
            if !flip_coastal_surface(template, &mut trial, coord, stream_id) {
                return None;
            }
            reconcile_coastline_overlay(template, &mut trial, stream_id);
            let coast = coastline_cells(&trial);
            let surface_valid =
                template
                    .reference_cells
                    .iter()
                    .zip(&trial)
                    .all(|(reference, resolved)| {
                        reference.facts.surface == resolved.facts.surface
                            || envelope.contains(&resolved.coord)
                    })
                    && coast.is_subset(&envelope)
                    && shape_is_valid(&coast, rule)
                    && connected_sea(&trial)
                    && connected_mainland(&trial);
            if !surface_valid {
                return None;
            }
            let repair = repair_coast_displaced_landform(template, &mut trial)?;
            Some(CoastlineMove { coord, repair })
        })
        .collect()
}

/// Repairs the single bounded-landform seam which a one-cell coast move may
/// displace. The repair is part of the compound coast move: it is accepted only
/// when one deterministic, otherwise-authorized landform reassignment restores
/// every bounded shape. This keeps candidate scoring over hard-valid
/// foundations while allowing a legal retreat to expose the sixth-island
/// packing which exists in the template envelope. A successful repair remains
/// applied and is returned for compact replay; rejected trials are reverted in
/// place instead of cloning the complete cell vector.
fn repair_coast_displaced_landform(
    template: &SchematicTemplateV1,
    cells: &mut [CellPlan],
) -> Option<Option<LandformRepair>> {
    if bounded_shapes_are_valid(template, cells) {
        return Some(None);
    }
    let stream_id = template
        .generation
        .named_streams
        .iter()
        .find(|stream| stream.as_str() == "stream/landforms")?;
    let coast = coastline_cells(cells);
    let invalid_rules = template
        .bounded_regions
        .iter()
        .filter(|rule| {
            let selected = cells
                .iter()
                .filter(|cell| {
                    rule.targets
                        .iter()
                        .all(|target| cell_has_target(cell, *target))
                        && (rule.kind != BoundedRegionKind::Woodland
                            || rule.envelope.contains(&cell.coord))
                })
                .map(|cell| cell.coord)
                .collect::<BTreeSet<_>>();
            !selected.iter().all(|coord| rule.envelope.contains(coord))
                || !shape_is_valid(&selected, rule)
        })
        .filter_map(|rule| {
            let mut landforms = rule.targets.iter().filter_map(|target| match target {
                BoundedTarget::Landform(kind) => Some(*kind),
                _ => None,
            });
            let landform = landforms.next()?;
            landforms.next().is_none().then_some((rule, landform))
        })
        .collect::<Vec<_>>();

    for (rule, landform) in invalid_rules {
        let mut candidates = rule.envelope.clone();
        candidates.sort_unstable_by_key(|coord| {
            crate::model::canonical_coordinate_index(*coord).unwrap_or(usize::MAX)
        });
        for coord in candidates {
            let Some(index) = crate::model::canonical_coordinate_index(coord) else {
                continue;
            };
            let Some(cell) = cells.get(index) else {
                continue;
            };
            if cell.facts.surface != crate::model::SurfaceKind::Land
                || cell.facts.landform == landform
                || coast.contains(&coord)
                || !membership_change_allowed(template, rule, coord, true, &coast)
            {
                continue;
            }
            let Some(cell) = cells.get_mut(index) else {
                continue;
            };
            let previous_landform = cell.facts.landform;
            let previous_provenance = cell.provenance.landform.clone();
            cell.facts.landform = landform;
            cell.provenance.landform = seeded_provenance(stream_id);
            if bounded_shapes_are_valid(template, cells) {
                return Some(Some(LandformRepair { coord, landform }));
            }
            let cell = cells.get_mut(index)?;
            cell.facts.landform = previous_landform;
            cell.provenance.landform = previous_provenance;
        }
    }
    None
}

fn apply_preflighted_coastline_move(
    template: &SchematicTemplateV1,
    cells: &mut [CellPlan],
    selected_move: CoastlineMove,
) -> bool {
    let Some(coastline_stream) = template
        .generation
        .named_streams
        .iter()
        .find(|stream| stream.as_str() == "stream/coastline")
    else {
        return false;
    };
    if !flip_coastal_surface(template, cells, selected_move.coord, coastline_stream) {
        return false;
    }
    reconcile_coastline_overlay(template, cells, coastline_stream);
    if let Some(repair) = selected_move.repair {
        let Some(landform_stream) = template
            .generation
            .named_streams
            .iter()
            .find(|stream| stream.as_str() == "stream/landforms")
        else {
            return false;
        };
        let Some(index) = crate::model::canonical_coordinate_index(repair.coord) else {
            return false;
        };
        let Some(cell) = cells.get_mut(index) else {
            return false;
        };
        cell.facts.landform = repair.landform;
        cell.provenance.landform = seeded_provenance(landform_stream);
    }
    true
}

fn flip_coastal_surface(
    template: &SchematicTemplateV1,
    cells: &mut [CellPlan],
    coord: SchematicCoord,
    stream: &StableId,
) -> bool {
    let Some(index) = crate::model::canonical_coordinate_index(coord) else {
        return false;
    };
    let Some(current) = cells.get(index).cloned() else {
        return false;
    };
    let Some(cell) = cells.get_mut(index) else {
        return false;
    };
    let provenance = seeded_provenance(stream);
    match current.facts.surface {
        crate::model::SurfaceKind::Land => {
            cell.facts.surface = crate::model::SurfaceKind::OpenWater;
            cell.facts.landform = crate::model::LandformKind::None;
            cell.facts.climate = crate::model::ClimateKind::Marine;
            cell.facts.vegetation = crate::model::VegetationDensity::None;
            cell.provenance.surface = provenance.clone();
            cell.provenance.landform = provenance.clone();
            cell.provenance.climate = provenance.clone();
            cell.provenance.vegetation = provenance;
        }
        crate::model::SurfaceKind::OpenWater => {
            let donor = nearest_reference_land(template, coord);
            let Some(donor) = donor else {
                return false;
            };
            cell.facts.surface = crate::model::SurfaceKind::Land;
            cell.facts.landform = donor.facts.landform;
            cell.facts.climate = donor.facts.climate;
            cell.facts.vegetation = donor.facts.vegetation;
            cell.facts.access = donor.facts.access;
            cell.provenance.surface = provenance.clone();
            cell.provenance.landform = provenance.clone();
            cell.provenance.climate = provenance.clone();
            cell.provenance.vegetation = provenance.clone();
            cell.provenance.access = provenance;
        }
    }
    true
}

fn nearest_reference_land(
    template: &SchematicTemplateV1,
    coord: SchematicCoord,
) -> Option<&CellPlan> {
    template
        .reference_cells
        .iter()
        .filter(|cell| {
            cell.facts.surface == crate::model::SurfaceKind::Land
                && matches!(
                    cell.facts.landform,
                    crate::model::LandformKind::Beach | crate::model::LandformKind::Shore
                )
        })
        .min_by_key(|cell| {
            (
                coord.checked_distance(cell.coord).unwrap_or(u32::MAX),
                cell.id,
            )
        })
}

fn reconcile_coastline_overlay(
    template: &SchematicTemplateV1,
    cells: &mut [CellPlan],
    stream: &StableId,
) {
    let coast = coastline_cells(cells);
    for cell in cells {
        let has = cell.facts.overlays.binary_search(&FeatureKind::Coastline);
        if coast.contains(&cell.coord) {
            if !matches!(
                cell.facts.landform,
                crate::model::LandformKind::Beach | crate::model::LandformKind::Shore
            ) && !matches!(cell.provenance.landform, LayerProvenance::Locked { .. })
            {
                if let Some(donor) = nearest_reference_land(template, cell.coord) {
                    cell.facts.landform = donor.facts.landform;
                    cell.provenance.landform = seeded_provenance(stream);
                }
            }
            match has {
                Ok(index) => {
                    if let Some(source) = cell.provenance.overlays.get_mut(index) {
                        source.source = seeded_provenance(stream);
                    }
                }
                Err(index) => {
                    cell.facts.overlays.insert(index, FeatureKind::Coastline);
                    cell.provenance.overlays.insert(
                        index,
                        OverlayProvenance {
                            feature: FeatureKind::Coastline,
                            source: seeded_provenance(stream),
                        },
                    );
                }
            }
        } else if let Ok(index) = has {
            let locked = cell
                .provenance
                .overlays
                .get(index)
                .is_some_and(|source| matches!(source.source, LayerProvenance::Locked { .. }));
            if !locked {
                cell.facts.overlays.remove(index);
                if index < cell.provenance.overlays.len() {
                    cell.provenance.overlays.remove(index);
                }
            }
        }
    }
}

fn coastline_cells(cells: &[CellPlan]) -> BTreeSet<SchematicCoord> {
    cells
        .iter()
        .filter(|cell| {
            cell.facts.surface == crate::model::SurfaceKind::Land
                && !cell.facts.overlays.contains(&FeatureKind::SeaIsland)
                && !cell.facts.overlays.contains(&FeatureKind::LakeIsland)
        })
        .filter(|cell| {
            schematic_neighbors(cell.coord).into_iter().any(|neighbor| {
                crate::model::canonical_coordinate_index(neighbor)
                    .and_then(|index| cells.get(index))
                    .is_some_and(is_sea_cell)
            })
        })
        .map(|cell| cell.coord)
        .collect()
}

fn connected_sea(cells: &[CellPlan]) -> bool {
    let selected = cells
        .iter()
        .filter(|cell| is_sea_cell(cell))
        .map(|cell| cell.coord)
        .collect::<BTreeSet<_>>();
    component_sets(&selected).len() == 1
}

fn is_sea_cell(cell: &CellPlan) -> bool {
    cell.facts.surface == crate::model::SurfaceKind::OpenWater
        && cell.facts.climate == crate::model::ClimateKind::Marine
        && !cell.facts.overlays.iter().any(|feature| {
            matches!(
                feature,
                FeatureKind::River
                    | FeatureKind::Waterfall
                    | FeatureKind::ValleyLake
                    | FeatureKind::MountainLake
            )
        })
}

fn connected_mainland(cells: &[CellPlan]) -> bool {
    let selected = cells
        .iter()
        .filter(|cell| {
            cell.facts.surface == crate::model::SurfaceKind::Land
                && cell.facts.access == crate::model::AccessIntent::Ordinary
                && !cell.facts.overlays.contains(&FeatureKind::SeaIsland)
        })
        .map(|cell| cell.coord)
        .collect::<BTreeSet<_>>();
    component_sets(&selected).len() == 1
}

fn cell_has_locked_fact(cell: &CellPlan) -> bool {
    matches!(cell.provenance.surface, LayerProvenance::Locked { .. })
        || matches!(cell.provenance.landform, LayerProvenance::Locked { .. })
        || matches!(cell.provenance.climate, LayerProvenance::Locked { .. })
        || matches!(cell.provenance.vegetation, LayerProvenance::Locked { .. })
        || matches!(cell.provenance.access, LayerProvenance::Locked { .. })
        || cell
            .provenance
            .overlays
            .iter()
            .any(|source| matches!(source.source, LayerProvenance::Locked { .. }))
}

fn resolve_hydrology(
    template: &SchematicTemplateV1,
    cells: &mut [CellPlan],
    samples: NamedSamples,
) -> Vec<crate::model::Network> {
    let Some(rule) = template.bounded_regions.iter().find(|rule| {
        rule.kind == BoundedRegionKind::TracedRegion
            && rule
                .targets
                .contains(&BoundedTarget::Overlay(FeatureKind::River))
    }) else {
        return template.networks.clone();
    };
    let Some(stream_id) = template
        .generation
        .named_streams
        .iter()
        .find(|stream| stream.as_str() == "stream/hydrology")
    else {
        return template.networks.clone();
    };
    let corridor = rule.envelope.iter().copied().collect::<BTreeSet<_>>();
    let mut networks = template.networks.clone();
    let mut river_cells = BTreeSet::new();
    let mut varied_any = false;

    for network in &mut networks {
        if network.kind != crate::model::NetworkKind::Hydrology {
            continue;
        }
        let nodes = network
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node.coord))
            .collect::<std::collections::BTreeMap<_, _>>();
        for (edge_ordinal, edge) in network.edges.iter_mut().enumerate() {
            let reference_has_river = edge.path.iter().any(|coord| {
                template
                    .cell(*coord)
                    .is_some_and(|cell| cell.facts.overlays.contains(&FeatureKind::River))
            });
            if !reference_has_river {
                continue;
            }
            let Some(start) = nodes.get(edge.from.as_str()).copied() else {
                continue;
            };
            let Some(goal) = nodes.get(edge.to.as_str()).copied() else {
                continue;
            };
            // Hydrology is resolved before scenic islands. In particular, the
            // template's reference-island witness must not become an implicit
            // obstacle which lets a later island choice steer this path.
            let mut admitted = corridor.clone();
            admitted.insert(start);
            admitted.insert(goal);
            if let Some(path) = seeded_shortest_path(
                start,
                goal,
                &admitted,
                samples,
                u32::try_from(edge_ordinal).unwrap_or(u32::MAX),
            ) {
                edge.path = path;
                varied_any = true;
            }
            for coord in &edge.path {
                let fixed_water = template.cell(*coord).is_some_and(|cell| {
                    is_sea_cell(cell)
                        || cell.facts.overlays.iter().any(|feature| {
                            matches!(
                                feature,
                                FeatureKind::Waterfall
                                    | FeatureKind::ValleyLake
                                    | FeatureKind::MountainLake
                            )
                        })
                });
                if !fixed_water {
                    river_cells.insert(*coord);
                }
            }
        }
    }
    if !varied_any || !shape_is_valid(&river_cells, rule) {
        return template.networks.clone();
    }
    apply_region(
        template,
        cells,
        rule,
        &river_cells,
        "stream/hydrology",
        stream_id,
        samples,
        1_024,
        &BTreeSet::new(),
    );
    networks
}

fn seeded_shortest_path(
    start: SchematicCoord,
    goal: SchematicCoord,
    admitted: &BTreeSet<SchematicCoord>,
    samples: NamedSamples,
    ordinal: u32,
) -> Option<Vec<SchematicCoord>> {
    let mut pending = std::collections::VecDeque::from([start]);
    let mut visited = BTreeSet::from([start]);
    let mut previous = std::collections::BTreeMap::<SchematicCoord, SchematicCoord>::new();
    while let Some(current) = pending.pop_front() {
        if current == goal {
            break;
        }
        let neighbors = samples.ranked(
            "stream/hydrology",
            ordinal,
            schematic_neighbors(current)
                .into_iter()
                .filter(|neighbor| admitted.contains(neighbor))
                .map(coord_key),
        );
        for neighbor in neighbors {
            if visited.insert(neighbor) {
                previous.insert(neighbor, current);
                pending.push_back(neighbor);
            }
        }
    }
    if !visited.contains(&goal) {
        return None;
    }
    let mut reversed = vec![goal];
    let mut current = goal;
    while current != start {
        let prior = previous.get(&current).copied()?;
        reversed.push(prior);
        current = prior;
    }
    reversed.reverse();
    Some(reversed)
}

fn generation_stage(rule: &BoundedRegionRule) -> &'static str {
    match rule.kind {
        BoundedRegionKind::Coastline => "stream/coastline",
        BoundedRegionKind::ValleyLake => "stream/hydrology",
        BoundedRegionKind::SeaIslands => "stream/islands",
        BoundedRegionKind::Woodland => "stream/vegetation",
        BoundedRegionKind::Massif => "stream/landforms",
        BoundedRegionKind::TracedRegion
            if rule
                .targets
                .contains(&BoundedTarget::Overlay(FeatureKind::River)) =>
        {
            "stream/hydrology"
        }
        BoundedRegionKind::TracedRegion => "stream/landforms",
    }
}

fn vary_region(
    template: &SchematicTemplateV1,
    rule: &BoundedRegionRule,
    samples: NamedSamples,
    stage: &str,
    ordinal: u32,
    protected: &BTreeSet<SchematicCoord>,
) -> BTreeSet<SchematicCoord> {
    let changes_terrain = rule_changes_terrain(rule);
    let reference = rule
        .reference_mask
        .iter()
        .copied()
        .filter(|coord| !changes_terrain || !protected.contains(coord))
        .collect::<BTreeSet<_>>();
    let mut selected = reference.clone();
    let desired = samples.bounded(
        stage,
        None,
        ordinal.saturating_mul(32),
        u32::from(rule.count.min),
        u32::from(rule.count.max),
    );
    let desired = usize::try_from(desired).unwrap_or(usize::MAX);
    let envelope = rule.envelope.iter().copied().collect::<BTreeSet<_>>();

    while selected.len() < desired {
        let candidates = samples.ranked(
            stage,
            ordinal.saturating_mul(32).saturating_add(1),
            envelope
                .difference(&selected)
                .copied()
                .filter(|coord| {
                    schematic_neighbors(*coord)
                        .into_iter()
                        .any(|neighbor| selected.contains(&neighbor))
                })
                .map(coord_key),
        );
        let Some(next) = candidates.into_iter().find(|coord| {
            let mut trial = selected.clone();
            trial.insert(*coord);
            membership_change_allowed(template, rule, *coord, true, protected)
                && shape_is_valid(&trial, rule)
        }) else {
            break;
        };
        selected.insert(next);
    }
    while selected.len() > desired {
        let candidates = samples.ranked(
            stage,
            ordinal.saturating_mul(32).saturating_add(2),
            selected.iter().copied().map(coord_key),
        );
        let Some(next) = candidates.into_iter().find(|coord| {
            let mut trial = selected.clone();
            trial.remove(coord);
            membership_change_allowed(template, rule, *coord, false, protected)
                && shape_is_valid(&trial, rule)
        }) else {
            break;
        };
        selected.remove(&next);
    }

    let swap_limit = usize::try_from(samples.bounded(
        stage,
        None,
        ordinal.saturating_mul(32).saturating_add(3),
        2,
        8,
    ))
    .unwrap_or(2);
    for swap_ordinal in 0..swap_limit {
        let swap_ordinal = ordinal
            .saturating_mul(32)
            .saturating_add(4)
            .saturating_add(u32::try_from(swap_ordinal).unwrap_or(u32::MAX));
        let removals = samples.ranked(stage, swap_ordinal, selected.iter().copied().map(coord_key));
        let additions = samples.ranked(
            stage,
            swap_ordinal.saturating_add(128),
            envelope.difference(&selected).copied().map(coord_key),
        );
        let mut replacement = None;
        'pairs: for remove in &removals {
            if !membership_change_allowed(template, rule, *remove, false, protected) {
                continue;
            }
            for add in &additions {
                if !membership_change_allowed(template, rule, *add, true, protected) {
                    continue;
                }
                let mut trial = selected.clone();
                trial.remove(remove);
                trial.insert(*add);
                if trial != reference && shape_is_valid(&trial, rule) {
                    replacement = Some(trial);
                    break 'pairs;
                }
            }
        }
        let Some(next) = replacement else {
            break;
        };
        selected = next;
    }
    if shape_is_valid(&selected, rule) {
        selected
    } else {
        reference
    }
}

fn shape_is_valid(cells: &BTreeSet<SchematicCoord>, rule: &BoundedRegionRule) -> bool {
    if !rule
        .count
        .contains(u16::try_from(cells.len()).unwrap_or(u16::MAX))
    {
        return false;
    }
    let groups = component_sets(cells);
    rule.components
        .contains(u16::try_from(groups.len()).unwrap_or(u16::MAX))
        && groups.iter().all(|group| {
            rule.component_size
                .contains(u16::try_from(group.len()).unwrap_or(u16::MAX))
        })
}

fn apply_region(
    template: &SchematicTemplateV1,
    cells: &mut [CellPlan],
    rule: &BoundedRegionRule,
    selected: &BTreeSet<SchematicCoord>,
    stage: &str,
    stream_id: &StableId,
    samples: NamedSamples,
    ordinal: u32,
    protected: &BTreeSet<SchematicCoord>,
) {
    let reference = rule.reference_mask.iter().copied().collect::<BTreeSet<_>>();
    for coord in &rule.envelope {
        if rule_changes_terrain(rule) && protected.contains(coord) {
            continue;
        }
        let Some(index) = crate::model::canonical_coordinate_index(*coord) else {
            continue;
        };
        let desired = selected.contains(coord);
        let was_member = cells.get(index).is_some_and(|cell| {
            rule.targets
                .iter()
                .all(|target| cell_has_target(cell, *target))
        });
        let donor = if desired {
            None
        } else {
            background_donor(template, rule, *coord, samples, stage, ordinal)
        };
        let Some(cell) = cells.get_mut(index) else {
            continue;
        };
        for target in &rule.targets {
            if desired {
                set_target(cell, *target, stream_id, samples, ordinal);
            } else if reference.contains(coord) || was_member {
                clear_target(cell, donor, *target, stream_id);
            }
        }
        canonicalize_overlays(cell);
    }
}

fn background_donor<'a>(
    template: &'a SchematicTemplateV1,
    rule: &BoundedRegionRule,
    coord: SchematicCoord,
    samples: NamedSamples,
    stage: &str,
    ordinal: u32,
) -> Option<&'a CellPlan> {
    let reference = rule.reference_mask.iter().copied().collect::<BTreeSet<_>>();
    let mut donors = rule
        .envelope
        .iter()
        .copied()
        .filter(|candidate| !reference.contains(candidate))
        .filter_map(|candidate| {
            template
                .cell(candidate)
                .map(|cell| (cell, coord.checked_distance(candidate).unwrap_or(u32::MAX)))
        })
        .collect::<Vec<_>>();
    donors.sort_unstable_by(|(left_cell, left_distance), (right_cell, right_distance)| {
        left_distance
            .cmp(right_distance)
            .then_with(|| {
                samples
                    .sample(
                        stage,
                        Some((
                            left_cell.coord.q(),
                            left_cell.coord.r(),
                            left_cell.coord.s(),
                        )),
                        ordinal.saturating_add(255),
                    )
                    .cmp(&samples.sample(
                        stage,
                        Some((
                            right_cell.coord.q(),
                            right_cell.coord.r(),
                            right_cell.coord.s(),
                        )),
                        ordinal.saturating_add(255),
                    ))
            })
            .then_with(|| left_cell.id.cmp(&right_cell.id))
    });
    donors.first().map(|(cell, _)| *cell)
}

fn set_target(
    cell: &mut CellPlan,
    target: BoundedTarget,
    stream: &StableId,
    samples: NamedSamples,
    ordinal: u32,
) {
    let provenance = seeded_provenance(stream);
    match target {
        BoundedTarget::Surface(value) => {
            if cell.facts.surface == value
                && matches!(cell.provenance.surface, LayerProvenance::Locked { .. })
            {
                return;
            }
            cell.facts.surface = value;
            cell.provenance.surface = provenance;
        }
        BoundedTarget::Landform(value) => {
            if cell.facts.landform == value
                && matches!(cell.provenance.landform, LayerProvenance::Locked { .. })
            {
                return;
            }
            cell.facts.landform = value;
            cell.provenance.landform = provenance;
        }
        BoundedTarget::Climate(value) => {
            if cell.facts.climate == value
                && matches!(cell.provenance.climate, LayerProvenance::Locked { .. })
            {
                return;
            }
            cell.facts.climate = value;
            cell.provenance.climate = provenance;
        }
        BoundedTarget::Vegetation(value) => {
            if cell.facts.vegetation == value
                && matches!(cell.provenance.vegetation, LayerProvenance::Locked { .. })
            {
                return;
            }
            cell.facts.vegetation = value;
            cell.provenance.vegetation = provenance;
        }
        BoundedTarget::Vegetated => {
            if matches!(cell.provenance.vegetation, LayerProvenance::Locked { .. }) {
                return;
            }
            cell.facts.vegetation = match samples.bounded(
                stream.as_str(),
                Some((cell.coord.q(), cell.coord.r(), cell.coord.s())),
                ordinal.saturating_add(512),
                0,
                2,
            ) {
                0 => crate::model::VegetationDensity::Light,
                1 => crate::model::VegetationDensity::Moderate,
                _ => crate::model::VegetationDensity::Dense,
            };
            cell.provenance.vegetation = provenance;
        }
        BoundedTarget::Access(value) => {
            if cell.facts.access == value
                && matches!(cell.provenance.access, LayerProvenance::Locked { .. })
            {
                return;
            }
            cell.facts.access = value;
            cell.provenance.access = provenance;
        }
        BoundedTarget::Overlay(feature) => {
            if let Err(index) = cell.facts.overlays.binary_search(&feature) {
                cell.facts.overlays.insert(index, feature);
                cell.provenance.overlays.insert(
                    index,
                    OverlayProvenance {
                        feature,
                        source: provenance,
                    },
                );
            } else if let Ok(index) = cell.facts.overlays.binary_search(&feature) {
                if let Some(source) = cell.provenance.overlays.get_mut(index) {
                    if !matches!(source.source, LayerProvenance::Locked { .. }) {
                        source.source = provenance;
                    }
                }
            }
        }
    }
}

fn clear_target(
    cell: &mut CellPlan,
    donor: Option<&CellPlan>,
    target: BoundedTarget,
    stream: &StableId,
) {
    let provenance = seeded_provenance(stream);
    match target {
        BoundedTarget::Surface(_) => {
            if let Some(donor) = donor {
                cell.facts.surface = donor.facts.surface;
                cell.provenance.surface = provenance;
            }
        }
        BoundedTarget::Landform(_) => {
            if let Some(donor) = donor {
                cell.facts.landform = donor.facts.landform;
                cell.provenance.landform = provenance;
            }
        }
        BoundedTarget::Climate(_) => {
            if let Some(donor) = donor {
                cell.facts.climate = donor.facts.climate;
                cell.provenance.climate = provenance;
            }
        }
        BoundedTarget::Vegetation(_) => {
            if let Some(donor) = donor {
                cell.facts.vegetation = donor.facts.vegetation;
                cell.provenance.vegetation = provenance;
            }
        }
        BoundedTarget::Vegetated => {
            if let Some(donor) = donor {
                cell.facts.vegetation = if matches!(
                    donor.facts.vegetation,
                    crate::model::VegetationDensity::Light
                        | crate::model::VegetationDensity::Moderate
                        | crate::model::VegetationDensity::Dense
                ) {
                    crate::model::VegetationDensity::None
                } else {
                    donor.facts.vegetation
                };
                cell.provenance.vegetation = provenance;
            }
        }
        BoundedTarget::Access(_) => {
            if let Some(donor) = donor {
                cell.facts.access = donor.facts.access;
                cell.provenance.access = provenance;
            }
        }
        BoundedTarget::Overlay(feature) => {
            if let Ok(index) = cell.facts.overlays.binary_search(&feature) {
                cell.facts.overlays.remove(index);
                if index < cell.provenance.overlays.len() {
                    cell.provenance.overlays.remove(index);
                }
            }
        }
    }
}

fn canonicalize_overlays(cell: &mut CellPlan) {
    let mut paired = cell
        .facts
        .overlays
        .iter()
        .copied()
        .zip(cell.provenance.overlays.iter().cloned())
        .collect::<Vec<_>>();
    paired.sort_unstable_by_key(|(feature, _)| *feature);
    paired.dedup_by_key(|(feature, _)| *feature);
    cell.facts.overlays = paired.iter().map(|(feature, _)| *feature).collect();
    cell.provenance.overlays = paired.into_iter().map(|(_, source)| source).collect();
}

fn membership_change_allowed(
    template: &SchematicTemplateV1,
    rule: &BoundedRegionRule,
    coord: SchematicCoord,
    desired: bool,
    protected: &BTreeSet<SchematicCoord>,
) -> bool {
    let Some(cell) = template.cell(coord) else {
        return false;
    };
    let changes_terrain = rule_changes_terrain(rule);
    if changes_terrain && protected.contains(&coord) {
        return false;
    }
    let current = rule
        .targets
        .iter()
        .all(|target| cell_has_target(cell, *target));
    if current == desired {
        return true;
    }
    if is_network_node(template, coord)
        || (changes_terrain
            && (cell_has_locked_fact(cell)
                || is_network_path_cell(template, coord)
                || has_foreign_hydrology_overlay(cell, rule)))
    {
        return false;
    }
    !rule
        .targets
        .iter()
        .any(|target| target_is_locked(cell, *target))
}

fn rule_changes_terrain(rule: &BoundedRegionRule) -> bool {
    rule.targets.iter().any(|target| {
        matches!(
            target,
            BoundedTarget::Surface(_) | BoundedTarget::Landform(_)
        )
    })
}

fn has_foreign_hydrology_overlay(cell: &CellPlan, rule: &BoundedRegionRule) -> bool {
    cell.facts.overlays.iter().any(|feature| {
        matches!(
            feature,
            FeatureKind::River
                | FeatureKind::Waterfall
                | FeatureKind::ValleyLake
                | FeatureKind::MountainLake
        ) && !rule.targets.contains(&BoundedTarget::Overlay(*feature))
    })
}

fn is_network_node(template: &SchematicTemplateV1, coord: SchematicCoord) -> bool {
    template
        .networks
        .iter()
        .flat_map(|network| &network.nodes)
        .any(|node| node.coord == coord)
}

fn is_network_path_cell(template: &SchematicTemplateV1, coord: SchematicCoord) -> bool {
    template
        .networks
        .iter()
        .flat_map(|network| &network.edges)
        .any(|edge| edge.path.contains(&coord))
}

fn has_hydrology_overlay(cell: &CellPlan) -> bool {
    cell.facts.overlays.iter().any(|feature| {
        matches!(
            feature,
            FeatureKind::River
                | FeatureKind::Waterfall
                | FeatureKind::ValleyLake
                | FeatureKind::MountainLake
        )
    })
}

fn bounded_shapes_are_valid(template: &SchematicTemplateV1, cells: &[CellPlan]) -> bool {
    template.bounded_regions.iter().all(|rule| {
        let selected = cells
            .iter()
            .filter(|cell| {
                rule.targets
                    .iter()
                    .all(|target| cell_has_target(cell, *target))
                    && (rule.kind != BoundedRegionKind::Woodland
                        || rule.envelope.contains(&cell.coord))
            })
            .map(|cell| cell.coord)
            .collect::<BTreeSet<_>>();
        selected.iter().all(|coord| rule.envelope.contains(coord))
            && shape_is_valid(&selected, rule)
    })
}

fn target_is_locked(cell: &CellPlan, target: BoundedTarget) -> bool {
    match target {
        BoundedTarget::Surface(_) => {
            matches!(cell.provenance.surface, LayerProvenance::Locked { .. })
        }
        BoundedTarget::Landform(_) => {
            matches!(cell.provenance.landform, LayerProvenance::Locked { .. })
        }
        BoundedTarget::Climate(_) => {
            matches!(cell.provenance.climate, LayerProvenance::Locked { .. })
        }
        BoundedTarget::Vegetation(_) | BoundedTarget::Vegetated => {
            matches!(cell.provenance.vegetation, LayerProvenance::Locked { .. })
        }
        BoundedTarget::Access(_) => {
            matches!(cell.provenance.access, LayerProvenance::Locked { .. })
        }
        BoundedTarget::Overlay(feature) => cell
            .provenance
            .overlays
            .iter()
            .find(|source| source.feature == feature)
            .is_some_and(|source| matches!(source.source, LayerProvenance::Locked { .. })),
    }
}

fn cell_has_target(cell: &CellPlan, target: BoundedTarget) -> bool {
    match target {
        BoundedTarget::Surface(value) => cell.facts.surface == value,
        BoundedTarget::Landform(value) => cell.facts.landform == value,
        BoundedTarget::Climate(value) => cell.facts.climate == value,
        BoundedTarget::Vegetation(value) => cell.facts.vegetation == value,
        BoundedTarget::Vegetated => matches!(
            cell.facts.vegetation,
            crate::model::VegetationDensity::Light
                | crate::model::VegetationDensity::Moderate
                | crate::model::VegetationDensity::Dense
        ),
        BoundedTarget::Access(value) => cell.facts.access == value,
        BoundedTarget::Overlay(value) => cell.facts.overlays.binary_search(&value).is_ok(),
    }
}

fn seeded_provenance(stream: &StableId) -> LayerProvenance {
    LayerProvenance::Seeded {
        stream: stream.clone(),
    }
}

fn reference_fallback_cell(mut cell: CellPlan) -> CellPlan {
    cell.provenance.surface = fallback_source(&cell.provenance.surface);
    cell.provenance.landform = fallback_source(&cell.provenance.landform);
    cell.provenance.climate = fallback_source(&cell.provenance.climate);
    cell.provenance.vegetation = fallback_source(&cell.provenance.vegetation);
    cell.provenance.access = fallback_source(&cell.provenance.access);
    for overlay in &mut cell.provenance.overlays {
        overlay.source = fallback_source(&overlay.source);
    }
    cell
}

fn fallback_source(source: &LayerProvenance) -> LayerProvenance {
    let source = match source {
        LayerProvenance::Locked { claim } => claim.clone(),
        LayerProvenance::Bounded { rule } => rule.clone(),
        LayerProvenance::Seeded { stream } => stream.clone(),
        LayerProvenance::ReferenceFallback { source } => source.clone(),
    };
    LayerProvenance::ReferenceFallback { source }
}

fn coord_key(coord: SchematicCoord) -> (SchematicCoord, (i32, i32, i32)) {
    (coord, (coord.q(), coord.r(), coord.s()))
}

fn schematic_neighbors(coord: SchematicCoord) -> [SchematicCoord; 6] {
    coord.neighbors().unwrap_or([coord; 6])
}

fn component_sets(cells: &BTreeSet<SchematicCoord>) -> Vec<BTreeSet<SchematicCoord>> {
    let mut remaining = cells.clone();
    let mut result = Vec::new();
    while let Some(start) = remaining.first().copied() {
        let mut component = BTreeSet::from([start]);
        let mut pending = std::collections::VecDeque::from([start]);
        remaining.remove(&start);
        while let Some(cell) = pending.pop_front() {
            for neighbor in schematic_neighbors(cell) {
                if remaining.remove(&neighbor) {
                    component.insert(neighbor);
                    pending.push_back(neighbor);
                }
            }
        }
        result.push(component);
    }
    result
}

fn non_vegetation_semantic_fingerprint(plan: &SchematicPlanV1) -> u64 {
    let mut state = FNV_OFFSET;
    for cell in &plan.cells {
        state = fold_bytes(state, &cell.id.get().to_le_bytes());
        state = fold_bytes(state, &[surface_score_tag(cell.facts.surface)]);
        state = fold_bytes(state, &[landform_score_tag(cell.facts.landform)]);
        state = fold_bytes(state, &[climate_score_tag(cell.facts.climate)]);
        state = fold_bytes(state, &[access_score_tag(cell.facts.access)]);
        for overlay in &cell.facts.overlays {
            state = fold_bytes(state, &[feature_score_tag(*overlay)]);
        }
        state = fold_bytes(state, &[u8::MAX]);
    }
    for network in &plan.networks {
        state = fold_bytes(state, network.id.as_str().as_bytes());
        for edge in &network.edges {
            state = fold_bytes(state, edge.id.as_str().as_bytes());
            for coord in &edge.path {
                state = fold_bytes(state, &coord.q().to_le_bytes());
                state = fold_bytes(state, &coord.r().to_le_bytes());
                state = fold_bytes(state, &coord.s().to_le_bytes());
            }
        }
    }
    avalanche(state)
}

fn candidate_quality(template: &SchematicTemplateV1, plan: &SchematicPlanV1) -> u32 {
    template
        .reference_cells
        .iter()
        .zip(&plan.cells)
        .fold(0_u32, |score, (reference, resolved)| {
            let changed = reference.facts.surface != resolved.facts.surface
                || reference.facts.landform != resolved.facts.landform
                || reference.facts.climate != resolved.facts.climate
                || reference.facts.access != resolved.facts.access
                || reference.facts.overlays != resolved.facts.overlays;
            score.saturating_add(u32::from(changed))
        })
}

const fn surface_score_tag(value: crate::model::SurfaceKind) -> u8 {
    match value {
        crate::model::SurfaceKind::Land => 0,
        crate::model::SurfaceKind::OpenWater => 1,
    }
}

const fn landform_score_tag(value: crate::model::LandformKind) -> u8 {
    match value {
        crate::model::LandformKind::None => 0,
        crate::model::LandformKind::Island => 1,
        crate::model::LandformKind::Beach => 2,
        crate::model::LandformKind::Shore => 3,
        crate::model::LandformKind::Valley => 4,
        crate::model::LandformKind::Plateau => 5,
        crate::model::LandformKind::Hill => 6,
        crate::model::LandformKind::Mountain => 7,
        crate::model::LandformKind::Massif => 8,
        crate::model::LandformKind::SharpPeak => 9,
    }
}

const fn climate_score_tag(value: crate::model::ClimateKind) -> u8 {
    match value {
        crate::model::ClimateKind::Marine => 0,
        crate::model::ClimateKind::Temperate => 1,
        crate::model::ClimateKind::Alpine => 2,
        crate::model::ClimateKind::Frozen => 3,
    }
}

const fn access_score_tag(value: crate::model::AccessIntent) -> u8 {
    match value {
        crate::model::AccessIntent::Ordinary => 0,
        crate::model::AccessIntent::Scenic => 1,
        crate::model::AccessIntent::Inaccessible => 2,
    }
}

const fn feature_score_tag(value: FeatureKind) -> u8 {
    match value {
        FeatureKind::Coastline => 0,
        FeatureKind::River => 1,
        FeatureKind::Waterfall => 2,
        FeatureKind::ValleyLake => 3,
        FeatureKind::MountainLake => 4,
        FeatureKind::LakeIsland => 5,
        FeatureKind::FrozenWoods => 6,
        FeatureKind::PeakRing => 7,
        FeatureKind::CrystalAscent => 8,
        FeatureKind::Tunnel => 9,
        FeatureKind::SeaIsland => 10,
    }
}

fn fold_bytes(mut state: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        state ^= u64::from(*byte);
        state = state.wrapping_mul(FNV_PRIME);
    }
    state
}

const fn avalanche(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
#[expect(
    clippy::panic_in_result_fn,
    reason = "fallible unit-test setup uses Result while assertions express exact contracts"
)]
mod tests {
    use super::*;
    use crate::template::grand_v3_reference_template;

    #[test]
    fn named_samples_are_repeatable_and_stage_independent() {
        let samples = NamedSamples::new(42, 7);
        let cell = (2, -1, -1);
        let coast = samples.sample("coast", Some(cell), 0);
        assert_eq!(coast, samples.sample("coast", Some(cell), 0));
        assert_ne!(coast, samples.sample("hydrology", Some(cell), 0));
        assert_ne!(
            coast,
            NamedSamples::new(43, 7).sample("coast", Some(cell), 0)
        );
        assert_ne!(
            coast,
            NamedSamples::new(42, 8).sample("coast", Some(cell), 0)
        );

        let _unrelated = (0..100)
            .map(|ordinal| samples.sample("woodland", Some(cell), ordinal))
            .collect::<Vec<_>>();
        assert_eq!(coast, samples.sample("coast", Some(cell), 0));
    }

    #[test]
    fn exact_template_preflight_cache_hits_misses_and_fails_closed(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let template = grand_v3_reference_template()?;
        let cache = Mutex::new(None);
        let first = validated_template_preflight_with_cache(&template, &cache)?;
        let hit = validated_template_preflight_with_cache(&template, &cache)?;
        assert!(Arc::ptr_eq(&first, &hit));

        let mut changed = template.clone();
        changed.id = StableId::new("template/grand-v3-cache-miss")?;
        let missed = validated_template_preflight_with_cache(&changed, &cache)?;
        assert!(!Arc::ptr_eq(&first, &missed));
        let changed_hit = validated_template_preflight_with_cache(&changed, &cache)?;
        assert!(Arc::ptr_eq(&missed, &changed_hit));

        let mut invalid = changed.clone();
        invalid.radius = invalid.radius.saturating_sub(1);
        assert!(validated_template_preflight_with_cache(&invalid, &cache).is_err());
        let after_invalid = validated_template_preflight_with_cache(&changed, &cache)?;
        assert!(Arc::ptr_eq(&missed, &after_invalid));
        Ok(())
    }

    #[test]
    fn poisoned_template_preflight_cache_is_cleared_before_reuse(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let template = grand_v3_reference_template()?;
        let cache = Mutex::new(None);
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = cache.lock().expect("fresh local cache lock");
            panic!("intentional template-preflight poison witness");
        }));
        assert!(poisoned.is_err());
        assert!(cache.is_poisoned());

        let preflight = validated_template_preflight_with_cache(&template, &cache)?;
        assert!(!preflight.is_empty());
        assert!(!cache.is_poisoned());
        Ok(())
    }

    #[test]
    fn concurrent_template_preflight_misses_converge_on_one_exact_entry(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let template = grand_v3_reference_template()?;
        let cache = Mutex::new(None);
        let results = std::thread::scope(|scope| {
            let workers = (0..8)
                .map(|_| scope.spawn(|| validated_template_preflight_with_cache(&template, &cache)))
                .collect::<Vec<_>>();
            workers
                .into_iter()
                .map(|worker| {
                    worker
                        .join()
                        .expect("template-preflight worker must not panic")
                })
                .collect::<Result<Vec<_>, _>>()
        })?;
        let first = results.first().ok_or("missing preflight witness")?;
        assert!(results.iter().all(|result| Arc::ptr_eq(first, result)));
        Ok(())
    }

    #[test]
    fn parallel_candidate_evaluation_is_byte_identical_to_serial(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let template = grand_v3_reference_template()?;
        let coastline_moves = validated_template_preflight(&template)?;
        for seed in [0, 1, 17, 42, 255, u64::MAX] {
            let serial = evaluate_candidates_with_worker_count(
                &template,
                seed,
                StreamPerturbations::default(),
                coastline_moves.as_ref(),
                1,
            )?;
            let parallel = evaluate_candidates_with_worker_count(
                &template,
                seed,
                StreamPerturbations::default(),
                coastline_moves.as_ref(),
                4,
            )?;
            assert_eq!(parallel, serial, "worker count changed seed {seed}");
        }
        Ok(())
    }

    #[test]
    fn bounded_samples_include_both_limits_and_degenerate_cleanly() {
        let samples = NamedSamples::new(9, 3);
        let values = (0..1_000)
            .map(|ordinal| samples.bounded("count", None, ordinal, 2, 6))
            .collect::<Vec<_>>();
        assert!(values.iter().all(|value| (2..=6).contains(value)));
        assert!(values.contains(&2));
        assert!(values.contains(&6));
        assert_eq!(samples.bounded("fixed", None, 0, 4, 4), 4);
    }

    #[test]
    fn ranking_is_canonical_for_an_unordered_input() {
        let samples = NamedSamples::new(123, 0);
        let cells = [(0_u8, (0, 0, 0)), (1, (1, -1, 0)), (2, (0, 1, -1))];
        let mut reversed = cells;
        reversed.reverse();
        assert_eq!(
            samples.ranked("islands", 0, cells),
            samples.ranked("islands", 0, reversed)
        );
    }

    #[test]
    fn separated_seed_packing_backtracks_past_a_blocking_first_choice(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let center = SchematicCoord::from_axial(0, 0)?;
        let east = SchematicCoord::from_axial(1, 0)?;
        let north_west = SchematicCoord::from_axial(-1, 1)?;
        let south_west = SchematicCoord::from_axial(0, -1)?;
        let packed = separated_seed_packing(&[center, east, north_west, south_west], 3)
            .ok_or_else(|| std::io::Error::other("three-way packing was rejected"))?;
        if packed != [east, north_west, south_west] {
            return Err(format!("packing did not backtrack deterministically: {packed:?}").into());
        }
        Ok(())
    }

    #[test]
    fn bitset_seed_packing_matches_the_vector_reference() {
        let canonical = crate::model::canonical_coordinates()
            .into_iter()
            .take(19)
            .collect::<Vec<_>>();
        let mut reversed = canonical.clone();
        reversed.reverse();
        let sparse = canonical
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(index, coord)| (index % 3 != 1).then_some(coord))
            .collect::<Vec<_>>();
        for ranked in [&canonical, &reversed, &sparse] {
            for desired in 0..=7 {
                assert_eq!(
                    separated_seed_packing(ranked, desired),
                    vector_seed_packing(ranked, desired),
                    "bitset packing diverged for {} cells and target {desired}",
                    ranked.len()
                );
            }
        }
    }

    #[test]
    fn corrected_coast_retreat_supports_six_separated_island_groups(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let template = grand_v3_reference_template()?;
        let mut cells = template.reference_cells.clone();
        let retreat = SchematicCoord::from_axial(-3, 5)?;
        let selected_move = valid_coastline_moves(&template)
            .into_iter()
            .find(|candidate| candidate.coord == retreat)
            .ok_or_else(|| std::io::Error::other("approved coast retreat was not preflighted"))?;
        if !apply_preflighted_coastline_move(&template, &mut cells, selected_move)
            || !bounded_shapes_are_valid(&template, &cells)
        {
            return Err("coast retreat did not restore every bounded landform shape".into());
        }
        let bridge = SchematicCoord::from_axial(-3, 3)?;
        if template.cell(bridge).map(|cell| cell.facts.landform)
            != Some(crate::model::LandformKind::Hill)
            || cells
                .get(
                    crate::model::canonical_coordinate_index(bridge).ok_or_else(|| {
                        std::io::Error::other("repair bridge has no canonical index")
                    })?,
                )
                .map(|cell| cell.facts.landform)
                != Some(crate::model::LandformKind::Valley)
        {
            return Err("coast repair did not derive the minimal Hill-to-Valley bridge".into());
        }

        let (rule_ordinal, rule) = template
            .bounded_regions
            .iter()
            .enumerate()
            .find(|(_, rule)| rule.kind == BoundedRegionKind::SeaIslands)
            .ok_or_else(|| std::io::Error::other("template has no sea-island rule"))?;
        let rule_ordinal = u32::try_from(rule_ordinal)?;
        let samples = (0..256_u64)
            .flat_map(|world_seed| {
                (0..CANDIDATE_ATTEMPTS)
                    .map(move |candidate| NamedSamples::new(world_seed, candidate))
            })
            .find(|samples| {
                samples.bounded(
                    "stream/islands",
                    None,
                    rule_ordinal.saturating_add(2_048),
                    2,
                    6,
                ) == 6
            })
            .ok_or_else(|| std::io::Error::other("no deterministic six-group sample"))?;
        let selected = vary_sea_islands(
            &cells,
            &template.networks,
            rule,
            samples,
            rule_ordinal,
            SelectionMode::Seeded,
        )
        .ok_or_else(|| {
            std::io::Error::other("six-group packing found no corrected-coast witness")
        })?;
        let packed_groups = component_sets(&selected);
        if packed_groups.len() != 6
            || packed_groups
                .iter()
                .any(|group| !(1..=4).contains(&group.len()))
            || !selected.iter().all(|coord| rule.envelope.contains(coord))
            || !shape_is_valid(&selected, rule)
        {
            return Err(
                format!("expected six groups of 1..=4 cells, found {packed_groups:?}").into(),
            );
        }
        let island_stream = template
            .generation
            .named_streams
            .iter()
            .find(|stream| stream.as_str() == "stream/islands")
            .ok_or_else(|| std::io::Error::other("template has no island stream"))?;
        let mut applied = cells.clone();
        apply_region(
            &template,
            &mut applied,
            rule,
            &selected,
            "stream/islands",
            island_stream,
            samples,
            rule_ordinal,
            &BTreeSet::new(),
        );
        let applied_islands = applied
            .iter()
            .filter(|cell| cell.facts.overlays.contains(&FeatureKind::SeaIsland))
            .map(|cell| cell.coord)
            .collect::<BTreeSet<_>>();
        if applied_islands != selected || !connected_sea(&applied) || !connected_mainland(&applied)
        {
            return Err("six-group packing broke the sea or mainland when applied".into());
        }
        let hydrology = established_hydrology_cells(&applied, &template.networks);
        if !applied_islands.is_disjoint(&hydrology) {
            return Err("six-group packing crossed authoritative hydrology".into());
        }
        Ok(())
    }

    #[test]
    fn selected_generation_exercises_the_six_group_bucket() -> Result<(), Box<dyn std::error::Error>>
    {
        let template = grand_v3_reference_template()?;
        let generated = generate(&template, 5)?;

        if generated.metrics.sea_island_groups != 6 {
            return Err(format!(
                "seed 5 selected {} sea-island groups instead of six",
                generated.metrics.sea_island_groups
            )
            .into());
        }
        if generated.plan.provenance.selected_candidate.is_none()
            || generated.plan.provenance.used_reference_fallback
        {
            return Err("seed 5 did not use a normal selected candidate".into());
        }
        if sea_island_group_capacity(&template, &generated.plan.cells, &generated.plan.networks)
            != 6
        {
            return Err("seed 5 lost the full configured island-group capacity".into());
        }
        validate_plan_draft(&template, &generated.plan)?;
        Ok(())
    }

    #[test]
    fn later_named_streams_cannot_shift_candidate_selection_or_hydrology(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let template = grand_v3_reference_template()?;
        let mut islands_actually_varied = false;
        let mut woodland_actually_varied = false;
        for world_seed in 0..8_u64 {
            let baseline =
                generate_internal(&template, world_seed, false, StreamPerturbations::default())?;
            let islands = generate_internal(
                &template,
                world_seed,
                false,
                StreamPerturbations {
                    islands: 0x6973_6c61_6e64_7301,
                    vegetation: 0,
                },
            )?;
            let woodland = generate_internal(
                &template,
                world_seed,
                false,
                StreamPerturbations {
                    islands: 0,
                    vegetation: 0x776f_6f64_6c61_6e01,
                },
            )?;

            if baseline.plan.provenance != islands.plan.provenance
                || baseline.plan.networks != islands.plan.networks
                || river_cells(&baseline.plan) != river_cells(&islands.plan)
            {
                return Err(std::io::Error::other(format!(
                    "island stream perturbation shifted candidate selection or hydrology for seed {world_seed}"
                ))
                .into());
            }
            islands_actually_varied |=
                sea_island_cells(&baseline.plan) != sea_island_cells(&islands.plan);

            if baseline.plan.provenance != woodland.plan.provenance
                || non_vegetation_semantic_fingerprint(&baseline.plan)
                    != non_vegetation_semantic_fingerprint(&woodland.plan)
            {
                return Err(std::io::Error::other(format!(
                    "woodland stream perturbation shifted candidate selection or non-vegetation output for seed {world_seed}"
                ))
                .into());
            }
            woodland_actually_varied |= baseline
                .plan
                .cells
                .iter()
                .map(|cell| cell.facts.vegetation)
                .ne(woodland.plan.cells.iter().map(|cell| cell.facts.vegetation));
        }
        if !islands_actually_varied {
            return Err("island perturbation never exercised the actual island sampler".into());
        }
        if !woodland_actually_varied {
            return Err(
                "vegetation perturbation never exercised the actual woodland sampler".into(),
            );
        }
        Ok(())
    }

    fn river_cells(plan: &SchematicPlanV1) -> BTreeSet<SchematicCoord> {
        plan.cells
            .iter()
            .filter(|cell| cell.facts.overlays.contains(&FeatureKind::River))
            .map(|cell| cell.coord)
            .collect()
    }

    fn sea_island_cells(plan: &SchematicPlanV1) -> BTreeSet<SchematicCoord> {
        plan.cells
            .iter()
            .filter(|cell| cell.facts.overlays.contains(&FeatureKind::SeaIsland))
            .map(|cell| cell.coord)
            .collect()
    }

    #[test]
    fn forced_candidate_failure_uses_the_validated_reference_fallback(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let template = grand_v3_reference_template()?;
        let generated = generate_internal(&template, 42, true, StreamPerturbations::default())?;
        let provenance = &generated.plan.provenance;
        if !provenance.used_reference_fallback
            || provenance.is_reference_artifact
            || provenance.selected_candidate.is_some()
            || provenance.candidates_evaluated != CANDIDATE_ATTEMPTS
            || provenance.hard_valid_candidates != 0
        {
            return Err("forced candidate failure was not marked as the reference fallback".into());
        }
        let expected_cells = template
            .reference_cells
            .iter()
            .cloned()
            .map(reference_fallback_cell)
            .collect::<Vec<_>>();
        if generated.plan.cells != expected_cells {
            return Err("reference fallback did not wrap every original layer source".into());
        }
        let expected_features = template
            .fixed_claims
            .iter()
            .cloned()
            .map(|mut feature| {
                feature.provenance = LayerProvenance::ReferenceFallback {
                    source: feature.id.clone(),
                };
                feature
            })
            .collect::<Vec<_>>();
        if generated.plan.features != expected_features {
            return Err("reference fallback did not wrap every fixed-claim source".into());
        }

        // `force_candidate_failure` is a test-only path and intentionally does
        // not match normal deterministic replay. The draft validator still
        // proves that the independently built fallback satisfies every hard
        // geometry and provenance contract.
        let recomputed = validate_plan_draft(&template, &generated.plan)?;
        if recomputed != generated.metrics {
            return Err("fallback metrics differ from an independent validation".into());
        }
        Ok(())
    }

    #[test]
    fn packaged_template_has_a_normal_hard_valid_candidate(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let template = grand_v3_reference_template()?;
        let world_seed = 0;
        let coastline_moves = valid_coastline_moves(&template);
        let mut failures = Vec::new();
        for candidate in 0..CANDIDATE_ATTEMPTS {
            let samples = NamedSamples::new(world_seed, candidate);
            let (mut cells, networks) =
                construct_candidate_foundation(&template, samples, &coastline_moves);
            if !apply_island_stage(
                &template,
                &mut cells,
                &networks,
                samples,
                SelectionMode::Canonical,
            ) || !apply_woodland_stage(&template, &mut cells, samples, SelectionMode::Canonical)
            {
                failures.push(format!(
                    "candidate {candidate}: no canonical island/woodland witness"
                ));
                continue;
            }
            let plan = finish_plan(
                &template,
                PlanProvenance::candidate(world_seed, candidate, 1)?,
                cells,
                template.fixed_claims.clone(),
                networks,
            )?;
            match validate_plan_draft(&template, &plan) {
                Ok(_) => return Ok(()),
                Err(error) if failures.len() < 3 => {
                    failures.push(format!("candidate {candidate}: {}", error))
                }
                Err(_) => {}
            }
        }
        Err(std::io::Error::other(format!(
            "all normal candidates failed; first failures: {}",
            failures.join(" | ")
        ))
        .into())
    }

    #[test]
    fn required_bounded_streams_do_not_silently_freeze() -> Result<(), Box<dyn std::error::Error>> {
        let template = grand_v3_reference_template()?;
        let coastline_moves = valid_coastline_moves(&template);
        let required = template
            .bounded_regions
            .iter()
            .filter(|rule| {
                rule.kind == BoundedRegionKind::ValleyLake
                    || rule.targets.iter().any(|target| {
                        matches!(
                            target,
                            BoundedTarget::Landform(
                                crate::model::LandformKind::Massif
                                    | crate::model::LandformKind::Mountain
                                    | crate::model::LandformKind::Hill
                                    | crate::model::LandformKind::Valley
                                    | crate::model::LandformKind::Plateau
                                    | crate::model::LandformKind::Beach
                                    | crate::model::LandformKind::Shore
                            )
                        )
                    })
            })
            .collect::<Vec<_>>();
        let mut varied = BTreeSet::new();
        'seeds: for world_seed in 0..64_u64 {
            for candidate in 0..CANDIDATE_ATTEMPTS {
                let (cells, _) = construct_candidate_foundation(
                    &template,
                    NamedSamples::new(world_seed, candidate),
                    &coastline_moves,
                );
                for rule in &required {
                    let selected = cells
                        .iter()
                        .filter(|cell| {
                            rule.targets
                                .iter()
                                .all(|target| cell_has_target(cell, *target))
                        })
                        .map(|cell| cell.coord)
                        .collect::<BTreeSet<_>>();
                    let reference = rule.reference_mask.iter().copied().collect::<BTreeSet<_>>();
                    if selected != reference {
                        varied.insert(rule.id.as_str());
                    }
                }
                if varied.len() == required.len() {
                    break 'seeds;
                }
            }
        }
        let frozen = required
            .iter()
            .filter(|rule| !varied.contains(rule.id.as_str()))
            .map(|rule| rule.id.as_str())
            .collect::<Vec<_>>();
        if !frozen.is_empty() {
            return Err(std::io::Error::other(format!(
                "required bounded streams remained frozen: {}",
                frozen.join(", ")
            ))
            .into());
        }
        Ok(())
    }
}
