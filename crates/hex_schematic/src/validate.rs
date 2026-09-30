//! Renderer-independent hard validation for schematic plans.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use crate::fingerprint::semantic_fingerprint;
use crate::metrics::{
    cell_matches_rule, compute_metrics_unchecked, resolved_region, rounded_percent,
    SchematicMetricsV1,
};
use crate::model::{
    bounded_envelope, canonical_coordinate_index, traced_twenty_percent_range, BoundedRegionKind,
    BoundedRegionRule, BoundedTarget, CellPlan, ClimateKind, FeatureClaim, FeatureKind,
    LandformKind, LayerProvenance, Network, NetworkKind, NetworkNodeKind, SchematicCoord,
    SchematicPlanV1, SchematicTemplateV1, StableId, SurfaceKind, VegetationDensity,
    SCHEMATIC_CELL_COUNT, SCHEMATIC_RADIUS, SCHEMATIC_SCHEMA_VERSION,
};

const REQUIRED_STREAMS: [&str; 5] = [
    "stream/coastline",
    "stream/hydrology",
    "stream/islands",
    "stream/landforms",
    "stream/vegetation",
];

const REQUIRED_FIXED_FEATURES: [FeatureKind; 7] = [
    FeatureKind::Waterfall,
    FeatureKind::MountainLake,
    FeatureKind::LakeIsland,
    FeatureKind::FrozenWoods,
    FeatureKind::PeakRing,
    FeatureKind::CrystalAscent,
    FeatureKind::Tunnel,
];

/// Stable category for one hard schematic validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ValidationCode {
    /// Schema or template identity is unsupported.
    Schema,
    /// Canonical radius-eight coverage or ordering is malformed.
    Grid,
    /// A fixed template claim was lost or changed.
    FixedClaim,
    /// Coast or surface classification violates its envelope.
    Coast,
    /// Lake, river, or waterfall hydrology is malformed.
    Hydrology,
    /// A landform violates its exact traced or bounded contract.
    Landform,
    /// Scenic sea-island count, size, or separation is invalid.
    Islands,
    /// Woodland eligibility, coverage, or coherence is invalid.
    Woodland,
    /// A stable feature claim is malformed or duplicated.
    Feature,
    /// A semantic network is malformed, disconnected, or duplicated.
    Network,
    /// Selection or fingerprint provenance is inconsistent.
    Provenance,
}

/// One precise validation failure.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ValidationIssue {
    /// Stable issue category.
    pub code: ValidationCode,
    /// Designer-facing detail naming the failed fact.
    pub detail: String,
}

impl ValidationIssue {
    pub(crate) fn new(code: ValidationCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

/// Complete sorted set of reasons a template or plan was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    issues: Vec<ValidationIssue>,
}

impl ValidationError {
    pub(crate) fn from_issues(mut issues: Vec<ValidationIssue>) -> Option<Self> {
        if issues.is_empty() {
            return None;
        }
        issues.sort();
        issues.dedup();
        Some(Self { issues })
    }

    /// Returns every deterministic failure in stable category/detail order.
    #[must_use]
    pub fn issues(&self) -> &[ValidationIssue] {
        &self.issues
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "schematic validation failed with {} issue(s)",
            self.issues.len()
        )?;
        for issue in &self.issues {
            write!(formatter, "; {:?}: {}", issue.code, issue.detail)?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationError {}

/// Validates the complete designer template contract.
pub fn validate_template(template: &SchematicTemplateV1) -> Result<(), ValidationError> {
    let mut issues = Vec::new();
    if let Err(error) = template.validate_structure() {
        issues.push(ValidationIssue::new(
            ValidationCode::Schema,
            error.to_string(),
        ));
    }

    validate_generation_settings(template, &mut issues);
    validate_reference_cells(template, &mut issues);
    validate_world_topology(&template.reference_cells, &mut issues);
    validate_bounded_rules(template, &mut issues);
    validate_fixed_claims(template, &mut issues);
    validate_networks(&template.networks, &template.reference_cells, &mut issues);
    validate_reference_hydrology(template, &mut issues);
    validate_cell_provenance(template, &template.reference_cells, false, &mut issues);

    ValidationError::from_issues(issues).map_or(Ok(()), Err)
}

/// Validates one authoritative plan against its exact source template and
/// returns freshly recomputed strict metrics.
pub fn validate_plan(
    template: &SchematicTemplateV1,
    plan: &SchematicPlanV1,
) -> Result<SchematicMetricsV1, ValidationError> {
    validate_template(template)?;
    let metrics = validate_plan_draft(template, plan)?;
    if !plan.provenance.is_reference_artifact {
        match crate::generator::generate(template, plan.provenance.world_seed) {
            Ok(expected) if expected.plan == *plan => {}
            Ok(_) => {
                return Err(ValidationError {
                    issues: vec![ValidationIssue::new(
                        ValidationCode::Provenance,
                        "plan does not match deterministic 32-candidate replay and selection",
                    )],
                });
            }
            Err(error) => {
                return Err(ValidationError {
                    issues: vec![ValidationIssue::new(
                        ValidationCode::Provenance,
                        format!("cannot replay deterministic candidate selection: {error}"),
                    )],
                });
            }
        }
    }
    Ok(metrics)
}

pub(crate) fn validate_plan_draft(
    template: &SchematicTemplateV1,
    plan: &SchematicPlanV1,
) -> Result<SchematicMetricsV1, ValidationError> {
    let mut issues = Vec::new();
    if let Err(error) = plan.validate_structure() {
        issues.push(ValidationIssue::new(
            ValidationCode::Schema,
            error.to_string(),
        ));
    }
    if plan.schema_version != SCHEMATIC_SCHEMA_VERSION
        || plan.radius != SCHEMATIC_RADIUS
        || plan.template_id != template.id
        || plan.template_revision != template.revision
    {
        issues.push(ValidationIssue::new(
            ValidationCode::Schema,
            "plan header does not identify the exact validated template revision",
        ));
    }

    validate_plan_cells(template, plan, &mut issues);
    validate_world_topology(&plan.cells, &mut issues);
    validate_coast_contract(template, plan, &mut issues);
    validate_plan_features(template, plan, &mut issues);
    validate_networks(&plan.networks, &plan.cells, &mut issues);
    validate_plan_network_contracts(template, plan, &mut issues);
    validate_bounded_plan_regions(template, plan, &mut issues);
    let uses_reference_facts =
        plan.provenance.used_reference_fallback || plan.provenance.is_reference_artifact;
    validate_cell_provenance(
        template,
        &plan.cells,
        plan.provenance.used_reference_fallback,
        &mut issues,
    );
    if !uses_reference_facts {
        validate_changed_provenance(template, plan, &mut issues);
    }
    if uses_reference_facts {
        validate_reference_copy(template, plan, &mut issues);
    }

    let expected_fingerprint = semantic_fingerprint(plan);
    if plan.semantic_fingerprint != expected_fingerprint {
        issues.push(ValidationIssue::new(
            ValidationCode::Provenance,
            format!(
                "semantic fingerprint {:016x} does not match recomputed {:016x}",
                plan.semantic_fingerprint, expected_fingerprint
            ),
        ));
    }

    if let Some(error) = ValidationError::from_issues(issues) {
        return Err(error);
    }
    Ok(compute_metrics_unchecked(template, plan))
}

fn validate_generation_settings(template: &SchematicTemplateV1, issues: &mut Vec<ValidationIssue>) {
    let actual = template
        .generation
        .named_streams
        .iter()
        .map(StableId::as_str)
        .collect::<Vec<_>>();
    if actual != REQUIRED_STREAMS {
        issues.push(ValidationIssue::new(
            ValidationCode::Schema,
            format!(
                "named streams must be exactly {} in stable order",
                REQUIRED_STREAMS.join(", ")
            ),
        ));
    }
    if template.generation.candidate_attempts != 32 {
        issues.push(ValidationIssue::new(
            ValidationCode::Schema,
            "candidate_attempts must equal 32",
        ));
    }
}

fn validate_reference_cells(template: &SchematicTemplateV1, issues: &mut Vec<ValidationIssue>) {
    if template.reference_cells.len() != SCHEMATIC_CELL_COUNT {
        issues.push(ValidationIssue::new(
            ValidationCode::Grid,
            format!(
                "reference grid contains {} cells; expected {SCHEMATIC_CELL_COUNT}",
                template.reference_cells.len()
            ),
        ));
        return;
    }
    for cell in &template.reference_cells {
        validate_cell_semantics(cell, issues);
    }
}

fn validate_world_topology(cells: &[CellPlan], issues: &mut Vec<ValidationIssue>) {
    if cells.len() != SCHEMATIC_CELL_COUNT {
        return;
    }
    let sea = cells
        .iter()
        .filter(|cell| is_sea_cell(cell))
        .map(|cell| cell.coord)
        .collect::<BTreeSet<_>>();
    if !connected(&sea, schematic_neighbors) {
        issues.push(ValidationIssue::new(
            ValidationCode::Coast,
            "marine sea cells must form exactly one connected component",
        ));
    }
    let mainland = cells
        .iter()
        .filter(|cell| {
            cell.facts.surface == SurfaceKind::Land
                && cell.facts.access == crate::model::AccessIntent::Ordinary
                && !cell.facts.overlays.contains(&FeatureKind::SeaIsland)
                && !cell.facts.overlays.contains(&FeatureKind::LakeIsland)
        })
        .map(|cell| cell.coord)
        .collect::<BTreeSet<_>>();
    if !connected(&mainland, schematic_neighbors) {
        issues.push(ValidationIssue::new(
            ValidationCode::Landform,
            "ordinary mainland must form exactly one connected component",
        ));
    }
    let mountain_lake = feature_membership(cells, FeatureKind::MountainLake);
    let frozen_woods = feature_membership(cells, FeatureKind::FrozenWoods);
    let frozen_shore_contacts = frozen_shore_contacts(cells, &mountain_lake, &frozen_woods);
    for cell in cells {
        if cell.facts.surface != SurfaceKind::Land {
            continue;
        }
        for neighbor in schematic_neighbors(cell.coord) {
            let Some(other) =
                canonical_coordinate_index(neighbor).and_then(|index| cells.get(index))
            else {
                continue;
            };
            let fixed_frozen_peak_contact = (frozen_shore_contacts.contains(&cell.coord)
                && other.facts.overlays.contains(&FeatureKind::PeakRing))
                || (frozen_shore_contacts.contains(&other.coord)
                    && cell.facts.overlays.contains(&FeatureKind::PeakRing));
            if other.facts.surface == SurfaceKind::Land
                && !fixed_frozen_peak_contact
                && !legal_landform_transition(cell.facts.landform, other.facts.landform)
            {
                issues.push(ValidationIssue::new(
                    ValidationCode::Landform,
                    format!(
                        "illegal {:?}-to-{:?} landform transition between {} and {}",
                        cell.facts.landform,
                        other.facts.landform,
                        coord_label(cell.coord),
                        coord_label(other.coord)
                    ),
                ));
            }
        }
    }
    let actual_coast = ocean_coastline(cells);
    let declared_coast = cells
        .iter()
        .filter(|cell| cell.facts.overlays.contains(&FeatureKind::Coastline))
        .map(|cell| cell.coord)
        .collect::<BTreeSet<_>>();
    if actual_coast != declared_coast {
        issues.push(ValidationIssue::new(
            ValidationCode::Coast,
            "Coastline overlays must equal the land-side boundary of the connected marine sea",
        ));
    }
    for coord in &actual_coast {
        if canonical_coordinate_index(*coord)
            .and_then(|index| cells.get(index))
            .is_none_or(|cell| !is_coastal_landform(cell.facts.landform))
        {
            issues.push(ValidationIssue::new(
                ValidationCode::Coast,
                format!(
                    "mainland ocean-coast cell {} must be Beach or Shore",
                    coord_label(*coord)
                ),
            ));
        }
    }
    validate_landmark_topology(cells, issues);
}

const fn is_coastal_landform(landform: LandformKind) -> bool {
    matches!(landform, LandformKind::Beach | LandformKind::Shore)
}

fn validate_coast_contract(
    template: &SchematicTemplateV1,
    plan: &SchematicPlanV1,
    issues: &mut Vec<ValidationIssue>,
) {
    let Some(_rule) = template
        .bounded_regions
        .iter()
        .find(|rule| rule.kind == BoundedRegionKind::Coastline)
    else {
        return;
    };
    let envelope = template
        .bounded_regions
        .iter()
        .filter(|candidate| {
            candidate.kind == BoundedRegionKind::Coastline
                || candidate
                    .targets
                    .iter()
                    .any(|target| matches!(target, BoundedTarget::Surface(_)))
        })
        .flat_map(|candidate| candidate.envelope.iter().copied())
        .collect::<BTreeSet<_>>();
    for (reference, resolved) in template.reference_cells.iter().zip(&plan.cells) {
        if reference.facts.surface != resolved.facts.surface && !envelope.contains(&resolved.coord)
        {
            issues.push(ValidationIssue::new(
                ValidationCode::Coast,
                format!(
                    "surface changed outside every governing bounded envelope at {}",
                    coord_label(resolved.coord)
                ),
            ));
        }
    }
}

fn is_sea_cell(cell: &CellPlan) -> bool {
    cell.facts.surface == SurfaceKind::OpenWater
        && cell.facts.climate == ClimateKind::Marine
        && !cell
            .facts
            .overlays
            .iter()
            .any(|feature| matches!(feature, FeatureKind::ValleyLake | FeatureKind::MountainLake))
}

fn ocean_coastline(cells: &[CellPlan]) -> BTreeSet<SchematicCoord> {
    cells
        .iter()
        .filter(|cell| {
            cell.facts.surface == SurfaceKind::Land
                && !cell.facts.overlays.contains(&FeatureKind::SeaIsland)
                && !cell.facts.overlays.contains(&FeatureKind::LakeIsland)
        })
        .filter(|cell| {
            schematic_neighbors(cell.coord).into_iter().any(|neighbor| {
                canonical_coordinate_index(neighbor)
                    .and_then(|index| cells.get(index))
                    .is_some_and(is_sea_cell)
            })
        })
        .map(|cell| cell.coord)
        .collect()
}

const fn legal_landform_transition(left: LandformKind, right: LandformKind) -> bool {
    use LandformKind::{
        Beach, Hill, Island, Massif, Mountain, None, Plateau, SharpPeak, Shore, Valley,
    };
    match left {
        None => false,
        Island => matches!(right, Island),
        Beach => matches!(right, Beach | Shore | Valley | Plateau | Hill),
        Shore => matches!(right, Beach | Shore | Valley | Plateau | Hill | Mountain),
        Valley => matches!(right, Beach | Shore | Valley | Plateau | Hill | Mountain),
        Plateau => matches!(
            right,
            Beach | Shore | Valley | Plateau | Hill | Mountain | Massif
        ),
        Hill => matches!(
            right,
            Beach | Shore | Valley | Plateau | Hill | Mountain | Massif
        ),
        Mountain => matches!(
            right,
            Shore | Valley | Plateau | Hill | Mountain | Massif | SharpPeak
        ),
        Massif => matches!(right, Plateau | Hill | Mountain | Massif | SharpPeak),
        SharpPeak => matches!(right, Mountain | Massif | SharpPeak),
    }
}

fn validate_landmark_topology(cells: &[CellPlan], issues: &mut Vec<ValidationIssue>) {
    let mountain_lake = feature_membership(cells, FeatureKind::MountainLake);
    let lake_island = feature_membership(cells, FeatureKind::LakeIsland);
    let peak_ring = feature_membership(cells, FeatureKind::PeakRing);
    let waterfall = feature_membership(cells, FeatureKind::Waterfall);
    let frozen_woods = feature_membership(cells, FeatureKind::FrozenWoods);
    let shore_contacts = frozen_shore_contacts(cells, &mountain_lake, &frozen_woods);
    let peak_chains = components(&peak_ring, schematic_neighbors);
    let contact_joins_peak_chains = shore_contacts.len() == 1
        && shore_contacts.first().is_some_and(|contact| {
            peak_chains.iter().all(|chain| {
                schematic_neighbors(*contact)
                    .into_iter()
                    .any(|neighbor| chain.contains(&neighbor))
            })
        });
    let peak_barrier = peak_ring
        .union(&shore_contacts)
        .copied()
        .collect::<BTreeSet<_>>();
    if peak_ring.len() != 12
        || peak_chains.len() != 2
        || peak_chains.iter().any(|chain| chain.len() != 6)
        || !contact_joins_peak_chains
        || !connected(&peak_barrier, schematic_neighbors)
    {
        issues.push(ValidationIssue::new(
            ValidationCode::FixedClaim,
            "PeakRing must form two exact six-cell chains joined into one lake barrier by the single fixed FrozenWoods shore contact",
        ));
    }

    if lake_island.is_empty()
        || lake_island.iter().any(|coord| {
            canonical_coordinate_index(*coord)
                .and_then(|index| cells.get(index))
                .is_none_or(|cell| {
                    cell.facts.surface != SurfaceKind::Land
                        || cell.facts.landform != LandformKind::Island
                        || cell.facts.access != crate::model::AccessIntent::Scenic
                })
        })
        || lake_island.iter().any(|coord| {
            !schematic_neighbors(*coord)
                .into_iter()
                .any(|neighbor| mountain_lake.contains(&neighbor))
        })
    {
        issues.push(ValidationIssue::new(
            ValidationCode::Feature,
            "the exact lake island must be scenic land inside and touching the mountain lake",
        ));
    }
    for lake_cell in &mountain_lake {
        for neighbor in schematic_neighbors(*lake_cell) {
            if canonical_coordinate_index(neighbor).is_none()
                || mountain_lake.contains(&neighbor)
                || lake_island.contains(&neighbor)
                || peak_ring.contains(&neighbor)
                || waterfall.contains(&neighbor)
                || shore_contacts.contains(&neighbor)
            {
                continue;
            }
            issues.push(ValidationIssue::new(
                ValidationCode::FixedClaim,
                format!(
                    "mountain-lake boundary at {} is not enclosed by the authored peak barrier or fixed waterfall outlet",
                    coord_label(neighbor)
                ),
            ));
        }
    }
    let frozen_core = frozen_woods
        .difference(&shore_contacts)
        .copied()
        .collect::<BTreeSet<_>>();
    let frozen_surrounded = frozen_core.iter().all(|coord| {
        schematic_neighbors(*coord).into_iter().all(|neighbor| {
            if frozen_core.contains(&neighbor) || shore_contacts.contains(&neighbor) {
                return true;
            }
            canonical_coordinate_index(neighbor)
                .and_then(|index| cells.get(index))
                .is_some_and(|cell| {
                    matches!(
                        cell.facts.landform,
                        LandformKind::Mountain | LandformKind::Massif | LandformKind::SharpPeak
                    )
                })
        })
    });
    if frozen_core.len() != 3
        || frozen_woods.len() != 4
        || !connected(&frozen_core, schematic_neighbors)
        || shore_contacts.len() != 1
        || !frozen_surrounded
    {
        issues.push(ValidationIssue::new(
            ValidationCode::FixedClaim,
            "frozen woods must remain one exact three-cell mountain-surrounded core plus exactly one shore contact touching both woods and mountain lake",
        ));
    }
}

fn feature_membership(cells: &[CellPlan], feature: FeatureKind) -> BTreeSet<SchematicCoord> {
    cells
        .iter()
        .filter(|cell| cell.facts.overlays.contains(&feature))
        .map(|cell| cell.coord)
        .collect()
}

fn frozen_shore_contacts(
    cells: &[CellPlan],
    mountain_lake: &BTreeSet<SchematicCoord>,
    frozen_woods: &BTreeSet<SchematicCoord>,
) -> BTreeSet<SchematicCoord> {
    cells
        .iter()
        .filter(|cell| {
            cell.facts.landform == LandformKind::Shore
                && cell.facts.overlays.contains(&FeatureKind::FrozenWoods)
        })
        .filter(|cell| {
            let neighbors = schematic_neighbors(cell.coord);
            neighbors
                .iter()
                .any(|neighbor| mountain_lake.contains(neighbor))
                && neighbors
                    .iter()
                    .any(|neighbor| frozen_woods.contains(neighbor))
        })
        .map(|cell| cell.coord)
        .collect()
}

fn validate_cell_semantics(cell: &CellPlan, issues: &mut Vec<ValidationIssue>) {
    let label = coord_label(cell.coord);
    match (cell.facts.surface, cell.facts.landform) {
        (SurfaceKind::OpenWater, LandformKind::None) => {}
        (SurfaceKind::Land, landform) if landform != LandformKind::None => {}
        _ => issues.push(ValidationIssue::new(
            ValidationCode::Landform,
            format!("{label} surface and landform disagree"),
        )),
    }
    if cell.facts.surface == SurfaceKind::OpenWater
        && cell.facts.vegetation != VegetationDensity::None
    {
        issues.push(ValidationIssue::new(
            ValidationCode::Woodland,
            format!("{label} open water carries vegetation"),
        ));
    }
    for overlay in &cell.facts.overlays {
        let valid = match overlay {
            FeatureKind::Coastline => true,
            FeatureKind::River => cell.facts.surface == SurfaceKind::Land,
            FeatureKind::Waterfall => true,
            FeatureKind::ValleyLake | FeatureKind::MountainLake => {
                cell.facts.surface == SurfaceKind::OpenWater
            }
            FeatureKind::LakeIsland | FeatureKind::SeaIsland => {
                cell.facts.surface == SurfaceKind::Land
                    && cell.facts.landform == LandformKind::Island
            }
            FeatureKind::FrozenWoods => {
                cell.facts.surface == SurfaceKind::Land
                    && cell.facts.climate == ClimateKind::Frozen
                    && cell.facts.vegetation != VegetationDensity::None
            }
            FeatureKind::PeakRing => {
                cell.facts.surface == SurfaceKind::Land
                    && cell.facts.landform == LandformKind::SharpPeak
            }
            FeatureKind::CrystalAscent => cell.facts.surface == SurfaceKind::Land,
            FeatureKind::Tunnel => true,
        };
        if !valid {
            issues.push(ValidationIssue::new(
                ValidationCode::Feature,
                format!("{label} has layer facts incompatible with {overlay:?}"),
            ));
        }
    }
    if cell.facts.overlays.contains(&FeatureKind::SeaIsland)
        && cell.facts.access != crate::model::AccessIntent::Scenic
    {
        issues.push(ValidationIssue::new(
            ValidationCode::Islands,
            format!("{label} sea island must use Scenic access"),
        ));
    }
}

fn validate_bounded_rules(template: &SchematicTemplateV1, issues: &mut Vec<ValidationIssue>) {
    for required in [
        BoundedRegionKind::Coastline,
        BoundedRegionKind::SeaIslands,
        BoundedRegionKind::Woodland,
        BoundedRegionKind::ValleyLake,
        BoundedRegionKind::Massif,
    ] {
        let count = template
            .bounded_regions
            .iter()
            .filter(|rule| rule.kind == required)
            .count();
        if count != 1 {
            issues.push(ValidationIssue::new(
                ValidationCode::Schema,
                format!("template must contain exactly one {required:?} bounded rule"),
            ));
        }
    }
    for target in [
        BoundedTarget::Landform(LandformKind::Mountain),
        BoundedTarget::Landform(LandformKind::Hill),
        BoundedTarget::Landform(LandformKind::Valley),
        BoundedTarget::Landform(LandformKind::Plateau),
        BoundedTarget::Landform(LandformKind::Beach),
        BoundedTarget::Landform(LandformKind::Shore),
        BoundedTarget::Overlay(FeatureKind::River),
    ] {
        let count = template
            .bounded_regions
            .iter()
            .filter(|rule| {
                rule.kind == BoundedRegionKind::TracedRegion && rule.targets.contains(&target)
            })
            .count();
        if count != 1 {
            issues.push(ValidationIssue::new(
                ValidationCode::Schema,
                format!("template must contain exactly one TracedRegion rule targeting {target:?}"),
            ));
        }
    }

    for rule in &template.bounded_regions {
        validate_bounded_rule(template, rule, issues);
    }
}

fn validate_bounded_rule(
    template: &SchematicTemplateV1,
    rule: &BoundedRegionRule,
    issues: &mut Vec<ValidationIssue>,
) {
    let prefix = format!("bounded rule {}", rule.id);
    if rule.targets.is_empty()
        || rule
            .targets
            .windows(2)
            .any(|pair| pair.first().zip(pair.get(1)).is_some_and(|(a, b)| a >= b))
    {
        issues.push(ValidationIssue::new(
            ValidationCode::Schema,
            format!("{prefix} targets must be non-empty, unique, and canonical"),
        ));
    }
    if targets_conflict(&rule.targets) {
        issues.push(ValidationIssue::new(
            ValidationCode::Schema,
            format!("{prefix} contains conflicting values for one layer"),
        ));
    }
    for target in &rule.targets {
        match target {
            BoundedTarget::Overlay(feature) if REQUIRED_FIXED_FEATURES.contains(feature) => {
                issues.push(ValidationIssue::new(
                    ValidationCode::FixedClaim,
                    format!(
                        "{prefix} may not target fixed {:?} overlay membership",
                        feature
                    ),
                ));
            }
            _ => {}
        }
    }
    validate_coordinate_subset(
        &rule.reference_mask,
        &format!("{prefix} reference mask"),
        issues,
    );
    validate_coordinate_subset(&rule.envelope, &format!("{prefix} envelope"), issues);
    if rule.reference_mask.is_empty() {
        issues.push(ValidationIssue::new(
            ValidationCode::Schema,
            format!("{prefix} reference mask is empty"),
        ));
        return;
    }
    if usize::from(rule.baseline_count) != rule.reference_mask.len() {
        issues.push(ValidationIssue::new(
            ValidationCode::Schema,
            format!("{prefix} baseline_count does not equal its reference mask"),
        ));
    }
    if rule.max_displacement != 2 {
        issues.push(ValidationIssue::new(
            ValidationCode::Schema,
            format!("{prefix} must use the exact two-cell envelope"),
        ));
    }
    match bounded_envelope(&rule.reference_mask, 2) {
        Ok(expected) => {
            let expected = if rule.kind == BoundedRegionKind::Woodland {
                expected
                    .into_iter()
                    .filter(|coord| woodland_template_eligible(template, *coord))
                    .collect::<Vec<_>>()
            } else {
                expected
            };
            if expected != rule.envelope {
                let qualifier = if rule.kind == BoundedRegionKind::Woodland {
                    "exact eligible Hill/Valley subset of its clipped two-cell dilation"
                } else {
                    "exact clipped two-cell dilation"
                };
                issues.push(ValidationIssue::new(
                    ValidationCode::Schema,
                    format!("{prefix} envelope is not the {qualifier}"),
                ));
            }
        }
        Err(error) => issues.push(ValidationIssue::new(
            ValidationCode::Schema,
            format!("{prefix} envelope cannot be constructed: {error}"),
        )),
    }
    if rule.count.min > rule.count.max
        || rule.components.min > rule.components.max
        || rule.component_size.min > rule.component_size.max
        || usize::from(rule.count.max) > rule.envelope.len()
        || !rule.count.contains(rule.baseline_count)
    {
        issues.push(ValidationIssue::new(
            ValidationCode::Schema,
            format!("{prefix} count/component ranges are malformed"),
        ));
    }
    if let Some(range) = rule.coverage_percent {
        if range.min > range.max || range.max > 100 {
            issues.push(ValidationIssue::new(
                ValidationCode::Schema,
                format!("{prefix} coverage percentage must be ordered inside 0..=100"),
            ));
        }
        if rule.kind != BoundedRegionKind::Woodland {
            issues.push(ValidationIssue::new(
                ValidationCode::Schema,
                format!("{prefix} may declare coverage_percent only for Woodland"),
            ));
        }
    }
    let required_count = match rule.kind {
        BoundedRegionKind::Massif => crate::model::CountRange { min: 20, max: 30 },
        BoundedRegionKind::ValleyLake => crate::model::CountRange { min: 3, max: 7 },
        BoundedRegionKind::SeaIslands => crate::model::CountRange { min: 2, max: 24 },
        BoundedRegionKind::Woodland => {
            let eligible = u32::try_from(rule.envelope.len()).unwrap_or(u32::MAX);
            crate::model::CountRange {
                min: u16::try_from(eligible.saturating_mul(30).saturating_add(99) / 100)
                    .unwrap_or(u16::MAX),
                max: u16::try_from(eligible.saturating_mul(80) / 100).unwrap_or(u16::MAX),
            }
        }
        BoundedRegionKind::Coastline | BoundedRegionKind::TracedRegion => {
            traced_twenty_percent_range(rule.baseline_count)
        }
    };
    if rule.count != required_count {
        issues.push(ValidationIssue::new(
            ValidationCode::Schema,
            format!(
                "{prefix} count must be exactly {}..={}",
                required_count.min, required_count.max
            ),
        ));
    }

    let reference = rule.reference_mask.iter().copied().collect::<BTreeSet<_>>();
    let envelope = rule.envelope.iter().copied().collect::<BTreeSet<_>>();
    if !reference.is_subset(&envelope) {
        issues.push(ValidationIssue::new(
            ValidationCode::Schema,
            format!("{prefix} reference mask must be contained by its envelope"),
        ));
    }
    let actual_reference = template
        .reference_cells
        .iter()
        .filter(|cell| {
            cell_matches_rule(cell, rule)
                && (rule.kind != BoundedRegionKind::Woodland || envelope.contains(&cell.coord))
        })
        .map(|cell| cell.coord)
        .collect::<BTreeSet<_>>();
    if actual_reference != reference {
        issues.push(ValidationIssue::new(
            ValidationCode::Schema,
            format!("{prefix} reference mask must exactly match its resolved target membership"),
        ));
    }
    let groups = components(&reference, schematic_neighbors);
    if matches!(
        rule.kind,
        BoundedRegionKind::Coastline | BoundedRegionKind::TracedRegion
    ) && !exact_component_contract(rule.components, groups.len())
    {
        issues.push(ValidationIssue::new(
            ValidationCode::Schema,
            format!(
                "{prefix} must preserve the exact {}-component reference topology",
                groups.len()
            ),
        ));
    }
    if !rule.components.contains(len_u16(groups.len()))
        || groups
            .iter()
            .any(|group| !rule.component_size.contains(len_u16(group.len())))
    {
        issues.push(ValidationIssue::new(
            ValidationCode::Schema,
            format!("{prefix} reference topology lies outside its declared ranges"),
        ));
    }
    for coord in &rule.reference_mask {
        if template
            .cell(*coord)
            .is_none_or(|cell| !cell_matches_rule(cell, rule))
        {
            issues.push(ValidationIssue::new(
                ValidationCode::Schema,
                format!(
                    "{prefix} reference cell {} does not carry every target",
                    coord_label(*coord)
                ),
            ));
        }
    }
    validate_rule_kind_contract(template, rule, issues);
}

fn exact_component_contract(
    components: crate::model::CountRange,
    reference_components: usize,
) -> bool {
    components.min == components.max && usize::from(components.min) == reference_components
}

fn validate_rule_kind_contract(
    template: &SchematicTemplateV1,
    rule: &BoundedRegionRule,
    issues: &mut Vec<ValidationIssue>,
) {
    let prefix = format!("bounded rule {}", rule.id);
    let has = |target| rule.targets.binary_search(&target).is_ok();
    let required_target = match rule.kind {
        BoundedRegionKind::Coastline => Some(BoundedTarget::Overlay(FeatureKind::Coastline)),
        BoundedRegionKind::SeaIslands => Some(BoundedTarget::Overlay(FeatureKind::SeaIsland)),
        BoundedRegionKind::Woodland => None,
        BoundedRegionKind::ValleyLake => Some(BoundedTarget::Overlay(FeatureKind::ValleyLake)),
        BoundedRegionKind::Massif => Some(BoundedTarget::Landform(LandformKind::Massif)),
        BoundedRegionKind::TracedRegion => None,
    };
    if required_target.is_some_and(|target| !has(target)) {
        issues.push(ValidationIssue::new(
            ValidationCode::Schema,
            format!("{prefix} lacks its canonical target"),
        ));
    }
    match rule.kind {
        BoundedRegionKind::Coastline => {
            if rule.targets != [BoundedTarget::Overlay(FeatureKind::Coastline)] {
                issues.push(ValidationIssue::new(
                    ValidationCode::Coast,
                    format!("{prefix} must target only Coastline overlay membership"),
                ));
            }
        }
        BoundedRegionKind::SeaIslands => {
            if rule.components != (crate::model::CountRange { min: 2, max: 6 })
                || rule.component_size != (crate::model::CountRange { min: 1, max: 4 })
            {
                issues.push(ValidationIssue::new(
                    ValidationCode::Islands,
                    format!("{prefix} must bound 2..=6 groups of 1..=4 cells"),
                ));
            }
        }
        BoundedRegionKind::Woodland => {
            if rule.targets != [BoundedTarget::Vegetated]
                || rule.coverage_percent != Some(crate::model::PercentRange { min: 30, max: 80 })
            {
                issues.push(ValidationIssue::new(
                    ValidationCode::Woodland,
                    format!("{prefix} must target only Vegetated with exact 30..=80% coverage"),
                ));
            }
            if rule
                .envelope
                .iter()
                .any(|coord| !woodland_template_eligible(template, *coord))
            {
                issues.push(ValidationIssue::new(
                    ValidationCode::Woodland,
                    format!(
                        "{prefix} envelope must contain only traced Hill or Valley land outside fixed surface landmarks"
                    ),
                ));
            }
        }
        BoundedRegionKind::ValleyLake => {
            if rule.components != (crate::model::CountRange { min: 1, max: 1 }) {
                issues.push(ValidationIssue::new(
                    ValidationCode::Hydrology,
                    format!("{prefix} must stay connected and inside 3..=7 cells"),
                ));
            }
        }
        BoundedRegionKind::Massif => {
            if rule.components != (crate::model::CountRange { min: 1, max: 1 }) {
                issues.push(ValidationIssue::new(
                    ValidationCode::Landform,
                    format!("{prefix} massif must remain one connected component"),
                ));
            }
        }
        BoundedRegionKind::TracedRegion => {}
    }
}

fn woodland_template_eligible(template: &SchematicTemplateV1, coord: SchematicCoord) -> bool {
    template.cell(coord).is_some_and(|cell| {
        cell.facts.surface == SurfaceKind::Land
            && matches!(
                cell.facts.landform,
                LandformKind::Hill | LandformKind::Valley
            )
            && !cell.facts.overlays.iter().any(|feature| {
                matches!(
                    feature,
                    FeatureKind::CrystalAscent | FeatureKind::FrozenWoods
                )
            })
    })
}

fn validate_fixed_claims(template: &SchematicTemplateV1, issues: &mut Vec<ValidationIssue>) {
    for kind in REQUIRED_FIXED_FEATURES {
        let count = template
            .fixed_claims
            .iter()
            .filter(|claim| claim.kind == kind)
            .count();
        if count != 1 {
            issues.push(ValidationIssue::new(
                ValidationCode::FixedClaim,
                format!("template must contain exactly one fixed {kind:?} claim"),
            ));
        }
    }
    for claim in &template.fixed_claims {
        if !REQUIRED_FIXED_FEATURES.contains(&claim.kind) {
            issues.push(ValidationIssue::new(
                ValidationCode::FixedClaim,
                format!(
                    "fixed claim {} uses non-fixed {:?} overlay kind",
                    claim.id, claim.kind
                ),
            ));
        }
        validate_feature_claim(template, claim, issues);
    }
    validate_fixed_claim_overlaps(&template.fixed_claims, issues);
}

fn validate_fixed_claim_overlaps(claims: &[FeatureClaim], issues: &mut Vec<ValidationIssue>) {
    for (left_index, left) in claims.iter().enumerate() {
        let left_cells = left.cells.iter().copied().collect::<BTreeSet<_>>();
        for right in claims.iter().skip(left_index.saturating_add(1)) {
            let right_cells = right.cells.iter().copied().collect::<BTreeSet<_>>();
            let overlap = left_cells.intersection(&right_cells).count();
            if overlap == 0 {
                continue;
            }
            let pair = if left.kind <= right.kind {
                (left.kind, right.kind)
            } else {
                (right.kind, left.kind)
            };
            let allowed = match pair {
                (FeatureKind::Waterfall, FeatureKind::River)
                | (FeatureKind::Waterfall, FeatureKind::MountainLake)
                | (FeatureKind::Waterfall, FeatureKind::ValleyLake)
                | (FeatureKind::River, FeatureKind::ValleyLake)
                | (FeatureKind::CrystalAscent, FeatureKind::Tunnel) => overlap == 1,
                (FeatureKind::PeakRing, FeatureKind::Tunnel) => overlap == 1,
                _ => false,
            };
            if !allowed {
                issues.push(ValidationIssue::new(
                    ValidationCode::FixedClaim,
                    format!(
                        "locked claims {} ({:?}) and {} ({:?}) overlap in {overlap} unauthorized cell(s)",
                        left.id, left.kind, right.id, right.kind
                    ),
                ));
            }
        }
    }
}

fn validate_feature_claim(
    template: &SchematicTemplateV1,
    claim: &FeatureClaim,
    issues: &mut Vec<ValidationIssue>,
) {
    let prefix = format!("fixed claim {}", claim.id);
    if claim.cells.is_empty() {
        issues.push(ValidationIssue::new(
            ValidationCode::FixedClaim,
            format!("{prefix} is empty"),
        ));
    }
    validate_coordinate_subset(&claim.cells, &prefix, issues);
    if claim.provenance
        != (LayerProvenance::Locked {
            claim: claim.id.clone(),
        })
    {
        issues.push(ValidationIssue::new(
            ValidationCode::FixedClaim,
            format!("{prefix} must own itself with Locked provenance"),
        ));
    }
    let claimed = claim.cells.iter().copied().collect::<BTreeSet<_>>();
    let declared = template
        .reference_cells
        .iter()
        .filter(|cell| cell.facts.overlays.contains(&claim.kind))
        .map(|cell| cell.coord)
        .collect::<BTreeSet<_>>();
    if declared != claimed {
        issues.push(ValidationIssue::new(
            ValidationCode::FixedClaim,
            format!(
                "{prefix} cells must exactly match {:?} overlay membership",
                claim.kind
            ),
        ));
    }
    for coord in &claim.cells {
        let Some(cell) = template.cell(*coord) else {
            continue;
        };
        let overlay_index = cell.facts.overlays.binary_search(&claim.kind);
        let valid = overlay_index.ok().is_some_and(|index| {
            cell.provenance.overlays.get(index).is_some_and(|source| {
                source.feature == claim.kind
                    && source.source
                        == (LayerProvenance::Locked {
                            claim: claim.id.clone(),
                        })
            })
        });
        if !valid {
            issues.push(ValidationIssue::new(
                ValidationCode::FixedClaim,
                format!(
                    "{prefix} cell {} lacks its exact locked overlay",
                    coord_label(*coord)
                ),
            ));
        }
    }
}

fn validate_networks(
    networks: &[Network],
    authority: &[CellPlan],
    issues: &mut Vec<ValidationIssue>,
) {
    for kind in [NetworkKind::Hydrology, NetworkKind::Tunnel] {
        if networks
            .iter()
            .filter(|network| network.kind == kind)
            .count()
            != 1
        {
            issues.push(ValidationIssue::new(
                ValidationCode::Network,
                format!("schematic must contain exactly one {kind:?} network"),
            ));
        }
    }
    if networks.windows(2).any(|pair| {
        pair.first()
            .zip(pair.get(1))
            .is_some_and(|(a, b)| a.id >= b.id)
    }) {
        issues.push(ValidationIssue::new(
            ValidationCode::Network,
            "network identifiers must be unique and stable-id ordered",
        ));
    }
    for network in networks {
        validate_network(authority, network, issues);
    }
}

fn validate_network(authority: &[CellPlan], network: &Network, issues: &mut Vec<ValidationIssue>) {
    let prefix = format!("network {}", network.id);
    if network.nodes.len() < 2 || network.edges.is_empty() {
        issues.push(ValidationIssue::new(
            ValidationCode::Network,
            format!("{prefix} must contain at least two nodes and one edge"),
        ));
        return;
    }
    if network.nodes.windows(2).any(|pair| {
        pair.first()
            .zip(pair.get(1))
            .is_some_and(|(a, b)| a.id >= b.id)
    }) || network.edges.windows(2).any(|pair| {
        pair.first()
            .zip(pair.get(1))
            .is_some_and(|(a, b)| a.id >= b.id)
    }) {
        issues.push(ValidationIssue::new(
            ValidationCode::Network,
            format!("{prefix} nodes and edges must be uniquely stable-id ordered"),
        ));
    }
    let nodes = network
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    if network
        .nodes
        .iter()
        .any(|node| canonical_coordinate_index(node.coord).is_none())
    {
        issues.push(ValidationIssue::new(
            ValidationCode::Network,
            format!("{prefix} contains a node outside the radius-eight grid"),
        ));
    }
    if !network
        .nodes
        .iter()
        .any(|node| node.kind == NetworkNodeKind::Source)
        || !network
            .nodes
            .iter()
            .any(|node| node.kind == NetworkNodeKind::Sink)
    {
        issues.push(ValidationIssue::new(
            ValidationCode::Network,
            format!("{prefix} requires at least one source and sink"),
        ));
    }

    let mut undirected = BTreeMap::<&str, BTreeSet<&str>>::new();
    let mut outgoing = BTreeMap::<&str, BTreeSet<&str>>::new();
    for node in &network.nodes {
        undirected.entry(node.id.as_str()).or_default();
        outgoing.entry(node.id.as_str()).or_default();
    }
    for edge in &network.edges {
        let Some(from) = nodes.get(edge.from.as_str()) else {
            issues.push(ValidationIssue::new(
                ValidationCode::Network,
                format!("{prefix} edge {} names a missing source node", edge.id),
            ));
            continue;
        };
        let Some(to) = nodes.get(edge.to.as_str()) else {
            issues.push(ValidationIssue::new(
                ValidationCode::Network,
                format!("{prefix} edge {} names a missing destination node", edge.id),
            ));
            continue;
        };
        if edge.from == edge.to
            || !contiguous_path(&edge.path, |first, second| {
                first.checked_distance(*second) == Some(1)
            })
            || edge.path.first() != Some(&from.coord)
            || edge.path.last() != Some(&to.coord)
            || edge.path.iter().collect::<BTreeSet<_>>().len() != edge.path.len()
            || edge
                .path
                .iter()
                .any(|coord| canonical_coordinate_index(*coord).is_none())
        {
            issues.push(ValidationIssue::new(
                ValidationCode::Network,
                format!("{prefix} edge {} has a malformed complete path", edge.id),
            ));
        }
        for coord in &edge.path {
            if !network_cell_compatible(authority, network.kind, *coord) {
                issues.push(ValidationIssue::new(
                    ValidationCode::Network,
                    format!(
                        "{prefix} edge {} crosses incompatible cell {}",
                        edge.id,
                        coord_label(*coord)
                    ),
                ));
            }
        }
        undirected
            .entry(edge.from.as_str())
            .or_default()
            .insert(edge.to.as_str());
        undirected
            .entry(edge.to.as_str())
            .or_default()
            .insert(edge.from.as_str());
        outgoing
            .entry(edge.from.as_str())
            .or_default()
            .insert(edge.to.as_str());
    }
    let node_ids = nodes.keys().copied().collect::<BTreeSet<_>>();
    if !connected_graph(&node_ids, &undirected) {
        issues.push(ValidationIssue::new(
            ValidationCode::Network,
            format!("{prefix} node graph is disconnected"),
        ));
    }
    let sources = network
        .nodes
        .iter()
        .filter(|node| node.kind == NetworkNodeKind::Source)
        .map(|node| node.id.as_str())
        .collect::<Vec<_>>();
    let reachable = reachable_nodes(&sources, &outgoing);
    if reachable.len() != network.nodes.len() {
        issues.push(ValidationIssue::new(
            ValidationCode::Network,
            format!("{prefix} has nodes unreachable from every source"),
        ));
    }
    if network.kind == NetworkKind::Hydrology {
        validate_hydrology_chain(authority, network, issues);
    } else if network.kind == NetworkKind::Tunnel {
        validate_tunnel_contract(authority, network, issues);
    }
}

fn validate_tunnel_contract(
    cells: &[CellPlan],
    network: &Network,
    issues: &mut Vec<ValidationIssue>,
) {
    let sources = network
        .nodes
        .iter()
        .filter(|node| node.kind == NetworkNodeKind::Source)
        .collect::<Vec<_>>();
    let sinks = network
        .nodes
        .iter()
        .filter(|node| node.kind == NetworkNodeKind::Sink)
        .collect::<Vec<_>>();
    let exact_shape = sources.len() == 1
        && sinks.len() == 1
        && network.nodes.len() == 2
        && network.edges.len() == 1;
    if !exact_shape {
        issues.push(ValidationIssue::new(
            ValidationCode::Network,
            format!(
                "tunnel network {} must contain exactly one source, one sink, and one complete path",
                network.id
            ),
        ));
        return;
    }

    let Some(source) = sources.first() else {
        return;
    };
    let Some(sink) = sinks.first() else {
        return;
    };
    let Some(edge) = network.edges.first() else {
        return;
    };
    if edge.from != source.id || edge.to != sink.id {
        issues.push(ValidationIssue::new(
            ValidationCode::Network,
            format!(
                "tunnel network {} path must run from its source to its sink",
                network.id
            ),
        ));
    }

    let source_is_ascent = canonical_coordinate_index(source.coord)
        .and_then(|index| cells.get(index))
        .is_some_and(|cell| cell.facts.overlays.contains(&FeatureKind::CrystalAscent));
    if !source_is_ascent {
        issues.push(ValidationIssue::new(
            ValidationCode::Network,
            format!(
                "tunnel network {} source must lie on CrystalAscent",
                network.id
            ),
        ));
    }

    let sink_is_ordinary_hill = canonical_coordinate_index(sink.coord)
        .and_then(|index| cells.get(index))
        .is_some_and(|cell| {
            cell.facts.surface == SurfaceKind::Land
                && cell.facts.landform == LandformKind::Hill
                && cell.facts.access == crate::model::AccessIntent::Ordinary
        });
    if !sink_is_ordinary_hill {
        issues.push(ValidationIssue::new(
            ValidationCode::Network,
            format!(
                "tunnel network {} sink must lie on ordinary Hill land",
                network.id
            ),
        ));
    }

    let routed = edge.path.iter().copied().collect::<BTreeSet<_>>();
    let declared = cells
        .iter()
        .filter(|cell| cell.facts.overlays.contains(&FeatureKind::Tunnel))
        .map(|cell| cell.coord)
        .collect::<BTreeSet<_>>();
    if routed.len() != edge.path.len() || routed != declared {
        issues.push(ValidationIssue::new(
            ValidationCode::Network,
            format!(
                "tunnel network {} simple path must match exact Tunnel overlay membership",
                network.id
            ),
        ));
    }
}

fn validate_hydrology_chain(
    cells: &[CellPlan],
    network: &Network,
    issues: &mut Vec<ValidationIssue>,
) {
    let mut indegree = BTreeMap::<&str, usize>::new();
    let mut outdegree = BTreeMap::<&str, usize>::new();
    for node in &network.nodes {
        indegree.insert(node.id.as_str(), 0);
        outdegree.insert(node.id.as_str(), 0);
    }
    for edge in &network.edges {
        if let Some(value) = outdegree.get_mut(edge.from.as_str()) {
            *value = value.saturating_add(1);
        }
        if let Some(value) = indegree.get_mut(edge.to.as_str()) {
            *value = value.saturating_add(1);
        }
    }
    let sources = network
        .nodes
        .iter()
        .filter(|node| node.kind == NetworkNodeKind::Source)
        .collect::<Vec<_>>();
    let sinks = network
        .nodes
        .iter()
        .filter(|node| node.kind == NetworkNodeKind::Sink)
        .collect::<Vec<_>>();
    let exact_chain = sources.len() == 1
        && sinks.len() == 1
        && network.nodes.iter().all(|node| match node.kind {
            NetworkNodeKind::Source => {
                indegree.get(node.id.as_str()) == Some(&0)
                    && outdegree.get(node.id.as_str()) == Some(&1)
            }
            NetworkNodeKind::Junction => {
                indegree.get(node.id.as_str()) == Some(&1)
                    && outdegree.get(node.id.as_str()) == Some(&1)
            }
            NetworkNodeKind::Sink => {
                indegree.get(node.id.as_str()) == Some(&1)
                    && outdegree.get(node.id.as_str()) == Some(&0)
            }
        });
    if !exact_chain {
        issues.push(ValidationIssue::new(
            ValidationCode::Hydrology,
            format!(
                "hydrology network {} must be one directed acyclic source-to-sink chain",
                network.id
            ),
        ));
    }
    let outlet_is_sea = sinks.first().is_some_and(|sink| {
        canonical_coordinate_index(sink.coord)
            .and_then(|index| cells.get(index))
            .is_some_and(is_sea_cell)
    });
    if !outlet_is_sea {
        issues.push(ValidationIssue::new(
            ValidationCode::Hydrology,
            format!(
                "hydrology network {} must end at exactly one marine sea outlet",
                network.id
            ),
        ));
    }
}

fn network_cell_compatible(cells: &[CellPlan], kind: NetworkKind, coord: SchematicCoord) -> bool {
    let Some(cell) = canonical_coordinate_index(coord).and_then(|index| cells.get(index)) else {
        return false;
    };
    match kind {
        NetworkKind::Tunnel => cell.facts.overlays.contains(&FeatureKind::Tunnel),
        NetworkKind::Hydrology => {
            cell.facts.surface == SurfaceKind::OpenWater
                || cell.facts.overlays.iter().any(|feature| {
                    matches!(
                        feature,
                        FeatureKind::River
                            | FeatureKind::Waterfall
                            | FeatureKind::ValleyLake
                            | FeatureKind::MountainLake
                    )
                })
        }
    }
}

fn validate_plan_network_contracts(
    template: &SchematicTemplateV1,
    plan: &SchematicPlanV1,
    issues: &mut Vec<ValidationIssue>,
) {
    if plan.networks.len() != template.networks.len() {
        issues.push(ValidationIssue::new(
            ValidationCode::Network,
            "generated plan must preserve the exact number of declared networks",
        ));
    }
    for (reference, resolved) in template.networks.iter().zip(&plan.networks) {
        if reference.id != resolved.id || reference.kind != resolved.kind {
            issues.push(ValidationIssue::new(
                ValidationCode::Network,
                format!(
                    "generated network {} changed identity or kind",
                    reference.id
                ),
            ));
            continue;
        }
        match reference.kind {
            NetworkKind::Tunnel => {
                if reference != resolved {
                    issues.push(ValidationIssue::new(
                        ValidationCode::Network,
                        format!("exact tunnel network {} changed", reference.id),
                    ));
                }
            }
            NetworkKind::Hydrology => {
                validate_resolved_hydrology(template, plan, reference, resolved, issues);
            }
        }
    }
}

fn validate_reference_hydrology(template: &SchematicTemplateV1, issues: &mut Vec<ValidationIssue>) {
    let Some(rule) = template.bounded_regions.iter().find(|rule| {
        rule.kind == BoundedRegionKind::TracedRegion
            && rule
                .targets
                .contains(&BoundedTarget::Overlay(FeatureKind::River))
    }) else {
        return;
    };
    let Some(network) = template
        .networks
        .iter()
        .find(|network| network.kind == NetworkKind::Hydrology)
    else {
        return;
    };
    let corridor = rule.envelope.iter().copied().collect::<BTreeSet<_>>();
    let declared = template
        .reference_cells
        .iter()
        .filter(|cell| cell.facts.overlays.contains(&FeatureKind::River))
        .map(|cell| cell.coord)
        .collect::<BTreeSet<_>>();
    let mut routed = BTreeSet::new();
    for edge in &network.edges {
        for coord in &edge.path {
            let Some(cell) = template.cell(*coord) else {
                continue;
            };
            if cell.facts.overlays.contains(&FeatureKind::River) {
                routed.insert(*coord);
            }
            let allowed_endpoint =
                edge.path.first() == Some(coord) || edge.path.last() == Some(coord);
            let fixed_water = is_sea_cell(cell)
                || cell.facts.overlays.iter().any(|feature| {
                    matches!(
                        feature,
                        FeatureKind::Waterfall
                            | FeatureKind::ValleyLake
                            | FeatureKind::MountainLake
                    )
                });
            if !corridor.contains(coord) && !fixed_water && !allowed_endpoint {
                issues.push(ValidationIssue::new(
                    ValidationCode::Hydrology,
                    format!(
                        "reference hydrology edge {} leaves the declared River corridor at {}",
                        edge.id,
                        coord_label(*coord)
                    ),
                ));
            }
        }
    }
    if routed != declared {
        issues.push(ValidationIssue::new(
            ValidationCode::Hydrology,
            "reference hydrology routes and River overlay membership disagree",
        ));
    }
}

fn validate_resolved_hydrology(
    template: &SchematicTemplateV1,
    plan: &SchematicPlanV1,
    reference: &Network,
    resolved: &Network,
    issues: &mut Vec<ValidationIssue>,
) {
    let prefix = format!("hydrology network {}", resolved.id);
    if resolved.nodes != reference.nodes || resolved.edges.len() != reference.edges.len() {
        issues.push(ValidationIssue::new(
            ValidationCode::Hydrology,
            format!("{prefix} must preserve exact nodes and edge identities"),
        ));
        return;
    }
    let river_corridor = template.bounded_regions.iter().find(|rule| {
        rule.kind == BoundedRegionKind::TracedRegion
            && rule
                .targets
                .contains(&BoundedTarget::Overlay(FeatureKind::River))
    });
    let Some(river_corridor) = river_corridor else {
        issues.push(ValidationIssue::new(
            ValidationCode::Hydrology,
            "template lacks the traced River corridor rule",
        ));
        return;
    };
    let corridor = river_corridor
        .envelope
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut routed_river_cells = BTreeSet::new();
    for (reference_edge, resolved_edge) in reference.edges.iter().zip(&resolved.edges) {
        if reference_edge.id != resolved_edge.id
            || reference_edge.from != resolved_edge.from
            || reference_edge.to != resolved_edge.to
        {
            issues.push(ValidationIssue::new(
                ValidationCode::Hydrology,
                format!("{prefix} changed edge identity or endpoints"),
            ));
            continue;
        }
        let reference_is_variable = reference_edge.path.iter().any(|coord| {
            template
                .cell(*coord)
                .is_some_and(|cell| cell.facts.overlays.contains(&FeatureKind::River))
        });
        if !reference_is_variable && resolved_edge.path != reference_edge.path {
            issues.push(ValidationIssue::new(
                ValidationCode::Hydrology,
                format!("{prefix} changed fixed edge {}", reference_edge.id),
            ));
        }
        for coord in &resolved_edge.path {
            let allowed_endpoint = resolved_edge.path.first() == Some(coord)
                || resolved_edge.path.last() == Some(coord);
            let is_fixed_hydrology = plan.cell(*coord).is_some_and(|cell| {
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
            if reference_is_variable
                && !corridor.contains(coord)
                && !is_fixed_hydrology
                && !allowed_endpoint
            {
                issues.push(ValidationIssue::new(
                    ValidationCode::Hydrology,
                    format!(
                        "{prefix} edge {} leaves the declared River corridor at {}",
                        resolved_edge.id,
                        coord_label(*coord)
                    ),
                ));
            }
            if plan
                .cell(*coord)
                .is_some_and(|cell| cell.facts.overlays.contains(&FeatureKind::River))
            {
                routed_river_cells.insert(*coord);
            }
        }
    }
    let declared_river_cells = plan
        .cells
        .iter()
        .filter(|cell| cell.facts.overlays.contains(&FeatureKind::River))
        .map(|cell| cell.coord)
        .collect::<BTreeSet<_>>();
    if routed_river_cells != declared_river_cells {
        issues.push(ValidationIssue::new(
            ValidationCode::Hydrology,
            format!("{prefix} route and River overlay membership disagree"),
        ));
    }

    let sources = resolved
        .nodes
        .iter()
        .filter(|node| node.kind == NetworkNodeKind::Source)
        .collect::<Vec<_>>();
    let sinks = resolved
        .nodes
        .iter()
        .filter(|node| node.kind == NetworkNodeKind::Sink)
        .collect::<Vec<_>>();
    let mut indegree = BTreeMap::<&str, usize>::new();
    let mut outdegree = BTreeMap::<&str, usize>::new();
    for node in &resolved.nodes {
        indegree.insert(node.id.as_str(), 0);
        outdegree.insert(node.id.as_str(), 0);
    }
    for edge in &resolved.edges {
        if let Some(value) = outdegree.get_mut(edge.from.as_str()) {
            *value = value.saturating_add(1);
        }
        if let Some(value) = indegree.get_mut(edge.to.as_str()) {
            *value = value.saturating_add(1);
        }
    }
    let simple_chain = sources.len() == 1
        && sinks.len() == 1
        && resolved.nodes.iter().all(|node| match node.kind {
            NetworkNodeKind::Source => {
                indegree.get(node.id.as_str()) == Some(&0)
                    && outdegree.get(node.id.as_str()) == Some(&1)
            }
            NetworkNodeKind::Junction => {
                indegree.get(node.id.as_str()) == Some(&1)
                    && outdegree.get(node.id.as_str()) == Some(&1)
            }
            NetworkNodeKind::Sink => {
                indegree.get(node.id.as_str()) == Some(&1)
                    && outdegree.get(node.id.as_str()) == Some(&0)
            }
        });
    if !simple_chain {
        issues.push(ValidationIssue::new(
            ValidationCode::Hydrology,
            format!("{prefix} must be one exact directed acyclic source-to-sea chain"),
        ));
    }
    let sea_outlet = sinks.first().is_some_and(|sink| {
        plan.cell(sink.coord).is_some_and(|cell| {
            cell.facts.surface == SurfaceKind::OpenWater
                && cell.facts.climate == ClimateKind::Marine
                && !cell.facts.overlays.iter().any(|feature| {
                    matches!(feature, FeatureKind::ValleyLake | FeatureKind::MountainLake)
                })
        })
    });
    if !sea_outlet {
        issues.push(ValidationIssue::new(
            ValidationCode::Hydrology,
            format!("{prefix} must have exactly one marine open-water outlet"),
        ));
    }
}

fn validate_plan_cells(
    template: &SchematicTemplateV1,
    plan: &SchematicPlanV1,
    issues: &mut Vec<ValidationIssue>,
) {
    if plan.cells.len() != SCHEMATIC_CELL_COUNT {
        issues.push(ValidationIssue::new(
            ValidationCode::Grid,
            format!(
                "generated grid contains {} cells; expected {SCHEMATIC_CELL_COUNT}",
                plan.cells.len()
            ),
        ));
        return;
    }
    for (reference, resolved) in template.reference_cells.iter().zip(&plan.cells) {
        validate_cell_semantics(resolved, issues);
        validate_locked_scalar(
            reference,
            resolved,
            &reference.provenance.surface,
            "surface",
            reference.facts.surface == resolved.facts.surface,
            issues,
        );
        validate_locked_scalar(
            reference,
            resolved,
            &reference.provenance.landform,
            "landform",
            reference.facts.landform == resolved.facts.landform,
            issues,
        );
        validate_locked_scalar(
            reference,
            resolved,
            &reference.provenance.climate,
            "climate",
            reference.facts.climate == resolved.facts.climate,
            issues,
        );
        validate_locked_scalar(
            reference,
            resolved,
            &reference.provenance.vegetation,
            "vegetation",
            reference.facts.vegetation == resolved.facts.vegetation,
            issues,
        );
        validate_locked_scalar(
            reference,
            resolved,
            &reference.provenance.access,
            "access",
            reference.facts.access == resolved.facts.access,
            issues,
        );
        for overlay in &reference.provenance.overlays {
            if matches!(overlay.source, LayerProvenance::Locked { .. })
                && !resolved.facts.overlays.contains(&overlay.feature)
            {
                issues.push(ValidationIssue::new(
                    ValidationCode::FixedClaim,
                    format!(
                        "{} lost locked {:?} overlay",
                        coord_label(reference.coord),
                        overlay.feature
                    ),
                ));
            }
        }
    }
}

fn validate_reference_copy(
    template: &SchematicTemplateV1,
    plan: &SchematicPlanV1,
    issues: &mut Vec<ValidationIssue>,
) {
    let copy_label = if plan.provenance.is_reference_artifact {
        "reference artifact"
    } else {
        "reference fallback"
    };
    if plan.networks != template.networks {
        issues.push(ValidationIssue::new(
            ValidationCode::Provenance,
            format!("{copy_label} changed an exact template network"),
        ));
    }
    for (reference, resolved) in template.reference_cells.iter().zip(&plan.cells) {
        if reference.facts != resolved.facts {
            issues.push(ValidationIssue::new(
                ValidationCode::Provenance,
                format!(
                    "{copy_label} changed facts at {}",
                    coord_label(reference.coord)
                ),
            ));
        }
        if plan.provenance.is_reference_artifact {
            if reference.provenance != resolved.provenance {
                issues.push(ValidationIssue::new(
                    ValidationCode::Provenance,
                    format!(
                        "reference artifact changed original authorship at {}",
                        coord_label(reference.coord)
                    ),
                ));
            }
            continue;
        }
        for (label, expected, actual) in [
            (
                "surface",
                provenance_id(&reference.provenance.surface),
                &resolved.provenance.surface,
            ),
            (
                "landform",
                provenance_id(&reference.provenance.landform),
                &resolved.provenance.landform,
            ),
            (
                "climate",
                provenance_id(&reference.provenance.climate),
                &resolved.provenance.climate,
            ),
            (
                "vegetation",
                provenance_id(&reference.provenance.vegetation),
                &resolved.provenance.vegetation,
            ),
            (
                "access",
                provenance_id(&reference.provenance.access),
                &resolved.provenance.access,
            ),
        ] {
            let matches = expected.is_some_and(|expected| {
                matches!(
                    actual,
                    LayerProvenance::ReferenceFallback { source } if source == expected
                )
            });
            if !matches {
                issues.push(ValidationIssue::new(
                    ValidationCode::Provenance,
                    format!(
                        "{copy_label} {label} source is incorrect at {}",
                        coord_label(reference.coord)
                    ),
                ));
            }
        }
        for (expected, actual) in reference
            .provenance
            .overlays
            .iter()
            .zip(&resolved.provenance.overlays)
        {
            let expected_source = provenance_id(&expected.source);
            let matches = expected.feature == actual.feature
                && expected_source.is_some_and(|expected_source| {
                    matches!(
                        &actual.source,
                        LayerProvenance::ReferenceFallback { source }
                            if source == expected_source
                    )
                });
            if !matches {
                issues.push(ValidationIssue::new(
                    ValidationCode::Provenance,
                    format!(
                        "{copy_label} overlay source is incorrect at {}",
                        coord_label(reference.coord)
                    ),
                ));
            }
        }
    }
}

fn provenance_id(source: &LayerProvenance) -> Option<&StableId> {
    match source {
        LayerProvenance::Locked { claim } => Some(claim),
        LayerProvenance::Bounded { rule } => Some(rule),
        LayerProvenance::Seeded { stream } => Some(stream),
        LayerProvenance::ReferenceFallback { .. } => None,
    }
}

fn validate_locked_scalar(
    reference: &CellPlan,
    _resolved: &CellPlan,
    provenance: &LayerProvenance,
    layer: &str,
    equal: bool,
    issues: &mut Vec<ValidationIssue>,
) {
    if matches!(provenance, LayerProvenance::Locked { .. }) && !equal {
        issues.push(ValidationIssue::new(
            ValidationCode::FixedClaim,
            format!("{} changed locked {layer}", coord_label(reference.coord)),
        ));
    }
}

fn validate_plan_features(
    template: &SchematicTemplateV1,
    plan: &SchematicPlanV1,
    issues: &mut Vec<ValidationIssue>,
) {
    if plan.features.len() != template.fixed_claims.len() {
        issues.push(ValidationIssue::new(
            ValidationCode::Feature,
            "generated feature claims must correspond one-for-one with fixed template claims",
        ));
    }
    for expected in &template.fixed_claims {
        let actual_membership = plan
            .cells
            .iter()
            .filter(|cell| cell.facts.overlays.contains(&expected.kind))
            .map(|cell| cell.coord)
            .collect::<Vec<_>>();
        if actual_membership != expected.cells {
            issues.push(ValidationIssue::new(
                ValidationCode::FixedClaim,
                format!(
                    "generated {:?} overlay membership does not equal fixed claim {}",
                    expected.kind, expected.id
                ),
            ));
        }
    }
    for (expected, actual) in template.fixed_claims.iter().zip(&plan.features) {
        if actual.id != expected.id
            || actual.kind != expected.kind
            || actual.cells != expected.cells
        {
            issues.push(ValidationIssue::new(
                ValidationCode::FixedClaim,
                format!(
                    "generated claim {} does not preserve its exact trace",
                    expected.id
                ),
            ));
        }
        let expected_provenance = if plan.provenance.used_reference_fallback {
            LayerProvenance::ReferenceFallback {
                source: expected.id.clone(),
            }
        } else {
            expected.provenance.clone()
        };
        if actual.provenance != expected_provenance {
            issues.push(ValidationIssue::new(
                ValidationCode::Provenance,
                format!("generated claim {} has incorrect provenance", expected.id),
            ));
        }
    }
}

fn validate_bounded_plan_regions(
    template: &SchematicTemplateV1,
    plan: &SchematicPlanV1,
    issues: &mut Vec<ValidationIssue>,
) {
    for rule in &template.bounded_regions {
        let resolved = resolved_region(plan, rule);
        let envelope = rule.envelope.iter().copied().collect::<BTreeSet<_>>();
        let groups = components(&resolved, schematic_neighbors);
        let code = validation_code_for_rule(rule.kind);
        if !resolved.is_subset(&envelope) {
            issues.push(ValidationIssue::new(
                code,
                format!(
                    "bounded rule {} selected cells outside its envelope",
                    rule.id
                ),
            ));
        }
        if !rule.count.contains(len_u16(resolved.len())) {
            issues.push(ValidationIssue::new(
                code,
                format!(
                    "bounded rule {} selected {} cells outside {}..={}",
                    rule.id,
                    resolved.len(),
                    rule.count.min,
                    rule.count.max
                ),
            ));
        }
        if !rule.components.contains(len_u16(groups.len())) {
            issues.push(ValidationIssue::new(
                code,
                format!(
                    "bounded rule {} formed {} components outside {}..={}",
                    rule.id,
                    groups.len(),
                    rule.components.min,
                    rule.components.max
                ),
            ));
        }
        for group in &groups {
            if !rule.component_size.contains(len_u16(group.len())) {
                issues.push(ValidationIssue::new(
                    code,
                    format!(
                        "bounded rule {} formed a {}-cell component outside {}..={}",
                        rule.id,
                        group.len(),
                        rule.component_size.min,
                        rule.component_size.max
                    ),
                ));
            }
        }
        if let Some(range) = rule.coverage_percent {
            let selected = u32::try_from(resolved.len()).unwrap_or(u32::MAX);
            let eligible = u32::try_from(rule.envelope.len()).unwrap_or(u32::MAX);
            if selected.saturating_mul(100) < u32::from(range.min).saturating_mul(eligible)
                || selected.saturating_mul(100) > u32::from(range.max).saturating_mul(eligible)
            {
                issues.push(ValidationIssue::new(
                    code,
                    format!(
                        "bounded rule {} coverage {}% lies outside {}..={} percent",
                        rule.id,
                        rounded_percent(len_u16(resolved.len()), len_u16(rule.envelope.len())),
                        range.min,
                        range.max
                    ),
                ));
            }
        }
        validate_resolved_kind_contract(plan, rule, &resolved, &groups, issues);
    }
}

fn validate_resolved_kind_contract(
    plan: &SchematicPlanV1,
    rule: &BoundedRegionRule,
    resolved: &BTreeSet<SchematicCoord>,
    groups: &[BTreeSet<SchematicCoord>],
    issues: &mut Vec<ValidationIssue>,
) {
    match rule.kind {
        BoundedRegionKind::Coastline => {
            let reference = rule.reference_mask.iter().copied().collect::<BTreeSet<_>>();
            let max_forward = directed_set_distance(resolved, &reference);
            let max_reverse = directed_set_distance(&reference, resolved);
            if max_forward.max(max_reverse) > 2 {
                issues.push(ValidationIssue::new(
                    ValidationCode::Coast,
                    format!("bounded coastline {} moved more than two cells", rule.id),
                ));
            }
        }
        BoundedRegionKind::SeaIslands => {
            if !(2..=6).contains(&groups.len())
                || groups.iter().any(|group| !(1..=4).contains(&group.len()))
            {
                issues.push(ValidationIssue::new(
                    ValidationCode::Islands,
                    format!("{} must resolve to 2..=6 groups of 1..=4 cells", rule.id),
                ));
            }
            for cell in resolved {
                for neighbor in schematic_neighbors(*cell) {
                    if resolved.contains(&neighbor) {
                        continue;
                    }
                    if plan.cell(neighbor).is_some_and(|neighbor_cell| {
                        neighbor_cell.facts.surface == SurfaceKind::Land
                    }) {
                        issues.push(ValidationIssue::new(
                            ValidationCode::Islands,
                            format!(
                                "{} scenic island at {} touches non-island land",
                                rule.id,
                                coord_label(*cell)
                            ),
                        ));
                    }
                }
            }
        }
        BoundedRegionKind::Woodland => {
            let percent = rounded_percent(len_u16(resolved.len()), len_u16(rule.envelope.len()));
            if !(30..=80).contains(&percent) {
                issues.push(ValidationIssue::new(
                    ValidationCode::Woodland,
                    format!("{} woodland coverage is {percent}%", rule.id),
                ));
            }
            for coord in resolved {
                if plan.cell(*coord).is_none_or(|cell| {
                    !matches!(
                        cell.facts.landform,
                        LandformKind::Hill | LandformKind::Valley
                    ) || cell.facts.overlays.contains(&FeatureKind::FrozenWoods)
                }) {
                    issues.push(ValidationIssue::new(
                        ValidationCode::Woodland,
                        format!(
                            "{} selected ineligible woodland cell {}",
                            rule.id,
                            coord_label(*coord)
                        ),
                    ));
                }
            }
        }
        BoundedRegionKind::ValleyLake => {
            if !(3..=7).contains(&resolved.len()) || groups.len() != 1 {
                issues.push(ValidationIssue::new(
                    ValidationCode::Hydrology,
                    format!("{} valley lake must be one 3..=7-cell component", rule.id),
                ));
            }
        }
        BoundedRegionKind::Massif => {
            if !(20..=30).contains(&resolved.len()) || groups.len() != 1 {
                issues.push(ValidationIssue::new(
                    ValidationCode::Landform,
                    format!("{} massif must be one 20..=30-cell component", rule.id),
                ));
            }
        }
        BoundedRegionKind::TracedRegion => {}
    }
}

fn validate_cell_provenance(
    template: &SchematicTemplateV1,
    cells: &[CellPlan],
    reference_fallback: bool,
    issues: &mut Vec<ValidationIssue>,
) {
    let claims = template
        .fixed_claims
        .iter()
        .map(|claim| (claim.id.as_str(), claim))
        .collect::<BTreeMap<_, _>>();
    let rules = template
        .bounded_regions
        .iter()
        .map(|rule| (rule.id.as_str(), rule))
        .collect::<BTreeMap<_, _>>();
    let streams = template
        .generation
        .named_streams
        .iter()
        .map(StableId::as_str)
        .collect::<BTreeSet<_>>();

    for cell in cells {
        validate_layer_source(
            cell,
            LayerSlot::Surface,
            &cell.provenance.surface,
            reference_fallback,
            &claims,
            &rules,
            &streams,
            issues,
        );
        validate_layer_source(
            cell,
            LayerSlot::Landform,
            &cell.provenance.landform,
            reference_fallback,
            &claims,
            &rules,
            &streams,
            issues,
        );
        validate_layer_source(
            cell,
            LayerSlot::Climate,
            &cell.provenance.climate,
            reference_fallback,
            &claims,
            &rules,
            &streams,
            issues,
        );
        validate_layer_source(
            cell,
            LayerSlot::Vegetation,
            &cell.provenance.vegetation,
            reference_fallback,
            &claims,
            &rules,
            &streams,
            issues,
        );
        validate_layer_source(
            cell,
            LayerSlot::Access,
            &cell.provenance.access,
            reference_fallback,
            &claims,
            &rules,
            &streams,
            issues,
        );
        for overlay in &cell.provenance.overlays {
            validate_layer_source(
                cell,
                LayerSlot::Overlay(overlay.feature),
                &overlay.source,
                reference_fallback,
                &claims,
                &rules,
                &streams,
                issues,
            );
        }
    }
}

fn validate_changed_provenance(
    template: &SchematicTemplateV1,
    plan: &SchematicPlanV1,
    issues: &mut Vec<ValidationIssue>,
) {
    for (reference, resolved) in template.reference_cells.iter().zip(&plan.cells) {
        for (changed, slot, source) in [
            (
                reference.facts.surface != resolved.facts.surface,
                LayerSlot::Surface,
                &resolved.provenance.surface,
            ),
            (
                reference.facts.landform != resolved.facts.landform,
                LayerSlot::Landform,
                &resolved.provenance.landform,
            ),
            (
                reference.facts.climate != resolved.facts.climate,
                LayerSlot::Climate,
                &resolved.provenance.climate,
            ),
            (
                reference.facts.vegetation != resolved.facts.vegetation,
                LayerSlot::Vegetation,
                &resolved.provenance.vegetation,
            ),
            (
                reference.facts.access != resolved.facts.access,
                LayerSlot::Access,
                &resolved.provenance.access,
            ),
        ] {
            if changed && !change_source_is_authorized(template, resolved.coord, slot, source) {
                issues.push(ValidationIssue::new(
                    ValidationCode::Provenance,
                    format!(
                        "changed {:?} at {} lacks its governing named stream",
                        slot,
                        coord_label(resolved.coord)
                    ),
                ));
            }
        }

        let reference_overlays = reference
            .facts
            .overlays
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let resolved_overlays = resolved
            .facts
            .overlays
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        for feature in reference_overlays.symmetric_difference(&resolved_overlays) {
            let slot = LayerSlot::Overlay(*feature);
            let source = resolved
                .provenance
                .overlays
                .iter()
                .find(|overlay| overlay.feature == *feature)
                .map(|overlay| &overlay.source);
            let authorized = source.is_some_and(|source| {
                change_source_is_authorized(template, resolved.coord, slot, source)
            }) || template.bounded_regions.iter().any(|rule| {
                rule.envelope.contains(&resolved.coord)
                    && rule.targets.contains(&BoundedTarget::Overlay(*feature))
            });
            if !authorized {
                issues.push(ValidationIssue::new(
                    ValidationCode::Provenance,
                    format!(
                        "changed {:?} overlay at {} lies outside its governing rule",
                        feature,
                        coord_label(resolved.coord)
                    ),
                ));
            }
        }
    }
}

fn change_source_is_authorized(
    template: &SchematicTemplateV1,
    coord: SchematicCoord,
    slot: LayerSlot,
    source: &LayerProvenance,
) -> bool {
    let LayerProvenance::Seeded { stream } = source else {
        return false;
    };
    template.bounded_regions.iter().any(|rule| {
        if !rule.envelope.contains(&coord) || generation_stream(rule) != stream.as_str() {
            return false;
        }
        let declared_target = rule
            .targets
            .iter()
            .any(|target| slot_accepts(slot, *target));
        let coupled_coast_fact = rule.kind == BoundedRegionKind::Coastline
            && matches!(
                slot,
                LayerSlot::Surface
                    | LayerSlot::Landform
                    | LayerSlot::Climate
                    | LayerSlot::Vegetation
                    | LayerSlot::Access
                    | LayerSlot::Overlay(FeatureKind::Coastline)
            );
        declared_target || coupled_coast_fact
    })
}

fn generation_stream(rule: &BoundedRegionRule) -> &'static str {
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

#[derive(Debug, Clone, Copy)]
enum LayerSlot {
    Surface,
    Landform,
    Climate,
    Vegetation,
    Access,
    Overlay(FeatureKind),
}

fn validate_layer_source(
    cell: &CellPlan,
    slot: LayerSlot,
    source: &LayerProvenance,
    reference_fallback: bool,
    claims: &BTreeMap<&str, &FeatureClaim>,
    rules: &BTreeMap<&str, &BoundedRegionRule>,
    streams: &BTreeSet<&str>,
    issues: &mut Vec<ValidationIssue>,
) {
    let valid = match source {
        LayerProvenance::Locked { claim } => {
            !reference_fallback
                && claims
                    .get(claim.as_str())
                    .is_some_and(|owner| owner.cells.contains(&cell.coord))
        }
        LayerProvenance::Bounded { rule } => {
            !reference_fallback
                && rules.get(rule.as_str()).is_some_and(|owner| {
                    owner.reference_mask.contains(&cell.coord)
                        && owner
                            .targets
                            .iter()
                            .any(|target| slot_matches_target(cell, slot, *target))
                })
        }
        LayerProvenance::Seeded { stream } => {
            !reference_fallback && streams.contains(stream.as_str())
        }
        LayerProvenance::ReferenceFallback { source } => {
            reference_fallback
                && (claims.contains_key(source.as_str())
                    || rules.contains_key(source.as_str())
                    || streams.contains(source.as_str()))
        }
    };
    if !valid {
        issues.push(ValidationIssue::new(
            ValidationCode::Provenance,
            format!(
                "{} has invalid {:?} provenance {:?}",
                coord_label(cell.coord),
                slot,
                source
            ),
        ));
    }
}

fn slot_matches_target(cell: &CellPlan, slot: LayerSlot, target: BoundedTarget) -> bool {
    match (slot, target) {
        (LayerSlot::Surface, BoundedTarget::Surface(expected)) => cell.facts.surface == expected,
        (LayerSlot::Landform, BoundedTarget::Landform(expected)) => cell.facts.landform == expected,
        (LayerSlot::Climate, BoundedTarget::Climate(expected)) => cell.facts.climate == expected,
        (LayerSlot::Vegetation, BoundedTarget::Vegetation(expected)) => {
            cell.facts.vegetation == expected
        }
        (LayerSlot::Vegetation, BoundedTarget::Vegetated) => matches!(
            cell.facts.vegetation,
            VegetationDensity::Light | VegetationDensity::Moderate | VegetationDensity::Dense
        ),
        (LayerSlot::Access, BoundedTarget::Access(expected)) => cell.facts.access == expected,
        (LayerSlot::Overlay(actual), BoundedTarget::Overlay(expected)) => {
            actual == expected && cell.facts.overlays.contains(&actual)
        }
        _ => false,
    }
}

fn slot_accepts(slot: LayerSlot, target: BoundedTarget) -> bool {
    matches!(
        (slot, target),
        (LayerSlot::Surface, BoundedTarget::Surface(_))
            | (LayerSlot::Landform, BoundedTarget::Landform(_))
            | (LayerSlot::Climate, BoundedTarget::Climate(_))
            | (LayerSlot::Vegetation, BoundedTarget::Vegetation(_))
            | (LayerSlot::Vegetation, BoundedTarget::Vegetated)
            | (LayerSlot::Access, BoundedTarget::Access(_))
            | (LayerSlot::Overlay(_), BoundedTarget::Overlay(_))
    ) && match (slot, target) {
        (LayerSlot::Overlay(actual), BoundedTarget::Overlay(expected)) => actual == expected,
        _ => true,
    }
}

fn validation_code_for_rule(kind: BoundedRegionKind) -> ValidationCode {
    match kind {
        BoundedRegionKind::Coastline => ValidationCode::Coast,
        BoundedRegionKind::SeaIslands => ValidationCode::Islands,
        BoundedRegionKind::Woodland => ValidationCode::Woodland,
        BoundedRegionKind::ValleyLake => ValidationCode::Hydrology,
        BoundedRegionKind::Massif | BoundedRegionKind::TracedRegion => ValidationCode::Landform,
    }
}

fn targets_conflict(targets: &[BoundedTarget]) -> bool {
    let mut layers = BTreeSet::new();
    targets
        .iter()
        .any(|target| !layers.insert(target_layer(*target)))
}

const fn target_layer(target: BoundedTarget) -> u8 {
    match target {
        BoundedTarget::Surface(_) => 0,
        BoundedTarget::Landform(_) => 1,
        BoundedTarget::Climate(_) => 2,
        BoundedTarget::Vegetation(_) | BoundedTarget::Vegetated => 3,
        BoundedTarget::Access(_) => 4,
        BoundedTarget::Overlay(feature) => 5_u8.saturating_add(feature_tag(feature)),
    }
}

const fn feature_tag(feature: FeatureKind) -> u8 {
    match feature {
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

fn validate_coordinate_subset(
    coordinates: &[SchematicCoord],
    label: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    if coordinates.is_empty() {
        return;
    }
    let indices = coordinates
        .iter()
        .map(|coord| canonical_coordinate_index(*coord))
        .collect::<Vec<_>>();
    if indices.iter().any(Option::is_none)
        || indices
            .windows(2)
            .any(|pair| pair.first().zip(pair.get(1)).is_some_and(|(a, b)| a >= b))
    {
        issues.push(ValidationIssue::new(
            ValidationCode::Grid,
            format!("{label} must be unique and in canonical radius-eight order"),
        ));
    }
}

fn directed_set_distance(from: &BTreeSet<SchematicCoord>, to: &BTreeSet<SchematicCoord>) -> u32 {
    from.iter()
        .filter_map(|cell| {
            to.iter()
                .filter_map(|target| cell.checked_distance(*target))
                .min()
        })
        .max()
        .unwrap_or(u32::MAX)
}

fn connected_graph<'a>(
    nodes: &BTreeSet<&'a str>,
    adjacency: &BTreeMap<&'a str, BTreeSet<&'a str>>,
) -> bool {
    let Some(start) = nodes.first().copied() else {
        return false;
    };
    let mut reached = BTreeSet::from([start]);
    let mut pending = VecDeque::from([start]);
    while let Some(node) = pending.pop_front() {
        for neighbor in adjacency.get(node).into_iter().flatten() {
            if nodes.contains(neighbor) && reached.insert(neighbor) {
                pending.push_back(neighbor);
            }
        }
    }
    reached.len() == nodes.len()
}

fn reachable_nodes<'a>(
    starts: &[&'a str],
    outgoing: &BTreeMap<&'a str, BTreeSet<&'a str>>,
) -> BTreeSet<&'a str> {
    let mut reached = starts.iter().copied().collect::<BTreeSet<_>>();
    let mut pending = starts.iter().copied().collect::<VecDeque<_>>();
    while let Some(node) = pending.pop_front() {
        for neighbor in outgoing.get(node).into_iter().flatten() {
            if reached.insert(neighbor) {
                pending.push_back(neighbor);
            }
        }
    }
    reached
}

fn schematic_neighbors(coord: SchematicCoord) -> [SchematicCoord; 6] {
    coord.neighbors().unwrap_or([coord; 6])
}

fn coord_label(coord: SchematicCoord) -> String {
    format!("({},{},{})", coord.q(), coord.r(), coord.s())
}

fn len_u16(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

#[cfg(test)]
const fn canonical_cell_count(radius: u16) -> u16 {
    1_u16.saturating_add(
        3_u16
            .saturating_mul(radius)
            .saturating_mul(radius.saturating_add(1)),
    )
}

#[cfg(test)]
const fn canonical_internal_adjacencies(radius: u16) -> u16 {
    3_u16
        .saturating_mul(radius)
        .saturating_mul(3_u16.saturating_mul(radius).saturating_add(1))
}

#[cfg(test)]
const fn canonical_boundary_cells(radius: u16) -> u16 {
    6_u16.saturating_mul(radius)
}

#[cfg(test)]
const fn canonical_outward_sides(radius: u16) -> u16 {
    6_u16.saturating_mul(2_u16.saturating_mul(radius).saturating_add(1))
}

fn connected<T: Copy + Ord>(cells: &BTreeSet<T>, neighbors: impl Fn(T) -> [T; 6]) -> bool {
    let Some(start) = cells.first().copied() else {
        return false;
    };
    let mut reached = BTreeSet::from([start]);
    let mut pending = VecDeque::from([start]);
    while let Some(cell) = pending.pop_front() {
        for neighbor in neighbors(cell) {
            if cells.contains(&neighbor) && reached.insert(neighbor) {
                pending.push_back(neighbor);
            }
        }
    }
    reached.len() == cells.len()
}

fn components<T: Copy + Ord>(
    cells: &BTreeSet<T>,
    neighbors: impl Fn(T) -> [T; 6] + Copy,
) -> Vec<BTreeSet<T>> {
    let mut remaining = cells.clone();
    let mut result = Vec::new();
    while let Some(start) = remaining.first().copied() {
        let mut component = BTreeSet::from([start]);
        let mut pending = VecDeque::from([start]);
        remaining.remove(&start);
        while let Some(cell) = pending.pop_front() {
            for neighbor in neighbors(cell) {
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

#[cfg(test)]
fn dilated<T: Copy + Ord>(
    cells: &BTreeSet<T>,
    steps: u8,
    neighbors: impl Fn(T) -> [T; 6] + Copy,
    admitted: impl Fn(T) -> bool + Copy,
) -> BTreeSet<T> {
    let mut result = cells.clone();
    let mut frontier = cells.clone();
    for _ in 0..steps {
        let next = frontier
            .iter()
            .flat_map(|cell| neighbors(*cell))
            .filter(|cell| admitted(*cell) && !result.contains(cell))
            .collect::<BTreeSet<_>>();
        result.extend(next.iter().copied());
        frontier = next;
        if frontier.is_empty() {
            break;
        }
    }
    result
}

fn contiguous_path<T>(path: &[T], adjacent: impl Fn(&T, &T) -> bool) -> bool {
    !path.is_empty()
        && path.windows(2).all(|pair| {
            pair.first()
                .zip(pair.get(1))
                .is_some_and(|(a, b)| adjacent(a, b))
        })
}

#[cfg(test)]
#[expect(
    clippy::indexing_slicing,
    clippy::panic_in_result_fn,
    reason = "fixed-size test fixtures use checked setup followed by assertions and compact indexing"
)]
mod tests {
    use super::*;

    type Coord = (i32, i32);

    fn neighbors((q, r): Coord) -> [Coord; 6] {
        [
            (q + 1, r),
            (q + 1, r - 1),
            (q, r - 1),
            (q - 1, r),
            (q - 1, r + 1),
            (q, r + 1),
        ]
    }

    fn adjacent(first: &Coord, second: &Coord) -> bool {
        neighbors(*first).contains(second)
    }

    #[test]
    fn connectivity_and_components_use_exact_hex_adjacency() {
        let first = (0, 0);
        let second = (1, -1);
        let remote = (3, -3);
        assert!(connected(&BTreeSet::from([first, second]), neighbors));
        assert!(!connected(
            &BTreeSet::from([first, second, remote]),
            neighbors
        ));
        assert_eq!(
            components(&BTreeSet::from([first, second, remote]), neighbors).len(),
            2
        );
    }

    #[test]
    fn canonical_radius_eight_geometry_is_exact() {
        assert_eq!(canonical_cell_count(8), 217);
        assert_eq!(canonical_internal_adjacencies(8), 600);
        assert_eq!(canonical_boundary_cells(8), 48);
        assert_eq!(canonical_outward_sides(8), 102);
    }

    #[test]
    fn dilation_is_exact_and_clipped_by_the_admitted_grid() {
        let origin = BTreeSet::from([(0, 0)]);
        let radius_two = dilated(&origin, 2, neighbors, |cell| {
            cube_distance((0, 0), cell) <= 2
        });
        assert_eq!(radius_two.len(), 19);
        let clipped = dilated(&origin, 2, neighbors, |cell| {
            cube_distance((0, 0), cell) <= 1
        });
        assert_eq!(clipped.len(), 7);
    }

    #[test]
    fn paths_require_at_least_one_cell_and_unit_steps() {
        let first = (0, 0);
        let second = (1, -1);
        let third = (1, 0);
        assert!(!contiguous_path::<Coord>(&[], adjacent));
        assert!(contiguous_path(&[first], adjacent));
        assert!(contiguous_path(&[first, second, third], adjacent));
        assert!(!contiguous_path(&[first, (2, -2)], adjacent));
    }

    #[test]
    fn traced_component_contract_is_exact_and_matches_the_reference() {
        let exact_two = crate::model::CountRange { min: 2, max: 2 };
        let variable = crate::model::CountRange { min: 1, max: 3 };
        let wrong_exact = crate::model::CountRange { min: 3, max: 3 };

        assert!(exact_component_contract(exact_two, 2));
        assert!(!exact_component_contract(variable, 2));
        assert!(!exact_component_contract(wrong_exact, 2));
    }

    #[test]
    fn approved_landmark_topology_uses_two_peak_chains_and_one_frozen_shore_contact(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut cells = blank_landmark_cells()?;
        for (q, r) in [
            (1, -4),
            (2, -5),
            (2, -4),
            (2, -3),
            (3, -3),
            (4, -7),
            (4, -3),
            (5, -7),
            (6, -7),
            (6, -6),
            (6, -5),
            (6, -4),
        ] {
            set_landmark_cell(
                &mut cells,
                q,
                r,
                SurfaceKind::Land,
                LandformKind::SharpPeak,
                ClimateKind::Alpine,
                VegetationDensity::None,
                crate::model::AccessIntent::Ordinary,
                &[FeatureKind::PeakRing],
            );
        }
        for (q, r) in [
            (3, -5),
            (3, -4),
            (4, -6),
            (4, -4),
            (5, -6),
            (5, -5),
            (5, -4),
        ] {
            set_landmark_cell(
                &mut cells,
                q,
                r,
                SurfaceKind::OpenWater,
                LandformKind::None,
                ClimateKind::Alpine,
                VegetationDensity::None,
                crate::model::AccessIntent::Scenic,
                &[FeatureKind::MountainLake],
            );
        }
        add_overlay(&mut cells, 5, -4, FeatureKind::Waterfall);
        set_landmark_cell(
            &mut cells,
            5,
            -3,
            SurfaceKind::Land,
            LandformKind::Mountain,
            ClimateKind::Alpine,
            VegetationDensity::None,
            crate::model::AccessIntent::Ordinary,
            &[FeatureKind::Waterfall],
        );
        set_landmark_cell(
            &mut cells,
            5,
            -2,
            SurfaceKind::OpenWater,
            LandformKind::None,
            ClimateKind::Temperate,
            VegetationDensity::None,
            crate::model::AccessIntent::Scenic,
            &[FeatureKind::Waterfall, FeatureKind::ValleyLake],
        );
        set_landmark_cell(
            &mut cells,
            4,
            -5,
            SurfaceKind::Land,
            LandformKind::Island,
            ClimateKind::Alpine,
            VegetationDensity::Sparse,
            crate::model::AccessIntent::Scenic,
            &[FeatureKind::LakeIsland],
        );
        for (q, r) in [(2, -7), (2, -6), (3, -7)] {
            set_landmark_cell(
                &mut cells,
                q,
                r,
                SurfaceKind::Land,
                LandformKind::Mountain,
                ClimateKind::Frozen,
                VegetationDensity::Moderate,
                crate::model::AccessIntent::Ordinary,
                &[FeatureKind::FrozenWoods],
            );
        }
        set_landmark_cell(
            &mut cells,
            3,
            -6,
            SurfaceKind::Land,
            LandformKind::Shore,
            ClimateKind::Frozen,
            VegetationDensity::Moderate,
            crate::model::AccessIntent::Ordinary,
            &[FeatureKind::FrozenWoods],
        );

        let mut issues = Vec::new();
        validate_landmark_topology(&cells, &mut issues);
        assert!(issues.is_empty(), "approved topology rejected: {issues:?}");

        remove_overlay(&mut cells, 1, -4, FeatureKind::PeakRing);
        let mut issues = Vec::new();
        validate_landmark_topology(&cells, &mut issues);
        assert!(issues.iter().any(|issue| {
            issue.code == ValidationCode::FixedClaim
                && issue.detail.contains("two exact six-cell chains")
        }));
        Ok(())
    }

    #[test]
    fn hydrology_overlays_preserve_their_underlying_land_and_water(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut cells = blank_landmark_cells()?;
        set_landmark_cell(
            &mut cells,
            0,
            0,
            SurfaceKind::Land,
            LandformKind::Valley,
            ClimateKind::Temperate,
            VegetationDensity::Light,
            crate::model::AccessIntent::Ordinary,
            &[FeatureKind::River],
        );
        set_landmark_cell(
            &mut cells,
            1,
            -1,
            SurfaceKind::Land,
            LandformKind::Massif,
            ClimateKind::Alpine,
            VegetationDensity::None,
            crate::model::AccessIntent::Inaccessible,
            &[FeatureKind::Waterfall],
        );
        set_landmark_cell(
            &mut cells,
            1,
            0,
            SurfaceKind::OpenWater,
            LandformKind::None,
            ClimateKind::Temperate,
            VegetationDensity::None,
            crate::model::AccessIntent::Scenic,
            &[FeatureKind::Waterfall, FeatureKind::ValleyLake],
        );
        for (q, r) in [(0, 0), (1, -1), (1, 0)] {
            let cell = &cells[canonical_coordinate_index(coord(q, r)).expect("test coordinate")];
            let mut issues = Vec::new();
            validate_cell_semantics(cell, &mut issues);
            assert!(issues.is_empty(), "hydrology cell rejected: {issues:?}");
            assert!(network_cell_compatible(
                &cells,
                NetworkKind::Hydrology,
                cell.coord
            ));
        }

        set_landmark_cell(
            &mut cells,
            -1,
            0,
            SurfaceKind::OpenWater,
            LandformKind::None,
            ClimateKind::Marine,
            VegetationDensity::None,
            crate::model::AccessIntent::Scenic,
            &[FeatureKind::Waterfall],
        );
        assert!(is_sea_cell(
            &cells[canonical_coordinate_index(coord(-1, 0)).expect("test coordinate")]
        ));

        set_landmark_cell(
            &mut cells,
            0,
            -1,
            SurfaceKind::OpenWater,
            LandformKind::None,
            ClimateKind::Marine,
            VegetationDensity::None,
            crate::model::AccessIntent::Scenic,
            &[FeatureKind::River],
        );
        let river_on_water =
            &cells[canonical_coordinate_index(coord(0, -1)).expect("test coordinate")];
        let mut issues = Vec::new();
        validate_cell_semantics(river_on_water, &mut issues);
        assert!(issues.iter().any(|issue| {
            issue.code == ValidationCode::Feature
                && issue.detail.contains("incompatible with River")
        }));
        Ok(())
    }

    #[test]
    fn exact_fixed_feature_endpoint_overlaps_are_authorized_once(
    ) -> Result<(), Box<dyn std::error::Error>> {
        for (left_kind, right_kind) in [
            (FeatureKind::PeakRing, FeatureKind::Tunnel),
            (FeatureKind::Waterfall, FeatureKind::ValleyLake),
        ] {
            let first = coord(0, 0);
            let second = coord(1, -1);
            let mut claims = vec![
                fixed_claim("claim/left", left_kind, vec![first])?,
                fixed_claim("claim/right", right_kind, vec![first])?,
            ];
            let mut issues = Vec::new();
            validate_fixed_claim_overlaps(&claims, &mut issues);
            assert!(issues.is_empty(), "one-cell overlap rejected: {issues:?}");

            claims[0].cells.push(second);
            claims[1].cells.push(second);
            let mut issues = Vec::new();
            validate_fixed_claim_overlaps(&claims, &mut issues);
            assert!(issues.iter().any(|issue| {
                issue.code == ValidationCode::FixedClaim
                    && issue.detail.contains("overlap in 2 unauthorized cell(s)")
            }));
        }
        Ok(())
    }

    #[test]
    fn bounded_rules_cannot_claim_fixed_overlay_authority() -> Result<(), Box<dyn std::error::Error>>
    {
        let template = crate::template::grand_v3_reference_template()?;
        let base_rule = template
            .bounded_regions
            .first()
            .ok_or_else(|| std::io::Error::other("packaged template has no bounded rule"))?;

        for feature in REQUIRED_FIXED_FEATURES {
            let mut rule = base_rule.clone();
            rule.targets = vec![BoundedTarget::Overlay(feature)];
            let mut issues = Vec::new();
            validate_bounded_rule(&template, &rule, &mut issues);
            if !issues.iter().any(|issue| {
                issue.code == ValidationCode::FixedClaim
                    && issue.detail.contains("may not target fixed")
            }) {
                return Err(format!("bounded rule accepted fixed {feature:?} authority").into());
            }
        }
        Ok(())
    }

    #[test]
    fn fixed_claims_reject_variable_overlay_kinds() -> Result<(), Box<dyn std::error::Error>> {
        let mut template = crate::template::grand_v3_reference_template()?;
        let mut unexpected = template
            .fixed_claims
            .first()
            .cloned()
            .ok_or_else(|| std::io::Error::other("packaged template has no fixed claim"))?;
        unexpected.id = StableId::new("claim/unexpected-variable-overlay")?;
        unexpected.kind = FeatureKind::SeaIsland;
        template.fixed_claims.push(unexpected);

        let mut issues = Vec::new();
        validate_fixed_claims(&template, &mut issues);
        if !issues.iter().any(|issue| {
            issue.code == ValidationCode::FixedClaim
                && issue
                    .detail
                    .contains("uses non-fixed SeaIsland overlay kind")
        }) {
            return Err("fixed claims accepted variable SeaIsland authority".into());
        }
        Ok(())
    }

    #[test]
    fn plan_fixed_overlay_membership_cannot_expand_past_its_claim(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let template = crate::template::grand_v3_reference_template()?;
        let mut plan = crate::generator::reference_plan(&template, 0)?.plan;
        let claim = template
            .fixed_claims
            .iter()
            .find(|claim| claim.kind == FeatureKind::CrystalAscent)
            .ok_or_else(|| std::io::Error::other("packaged template has no Crystal Ascent"))?;
        let extra = plan
            .cells
            .iter_mut()
            .find(|cell| !claim.cells.contains(&cell.coord))
            .ok_or_else(|| std::io::Error::other("no cell outside Crystal Ascent"))?;
        let index = extra
            .facts
            .overlays
            .binary_search(&FeatureKind::CrystalAscent)
            .expect_err("the selected cell must be outside Crystal Ascent");
        extra
            .facts
            .overlays
            .insert(index, FeatureKind::CrystalAscent);
        extra.provenance.overlays.insert(
            index,
            crate::model::OverlayProvenance {
                feature: FeatureKind::CrystalAscent,
                source: LayerProvenance::Seeded {
                    stream: StableId::new("stream/landforms")?,
                },
            },
        );

        let mut issues = Vec::new();
        validate_plan_features(&template, &plan, &mut issues);
        if !issues.iter().any(|issue| {
            issue.code == ValidationCode::FixedClaim
                && issue
                    .detail
                    .contains("CrystalAscent overlay membership does not equal fixed claim")
        }) {
            return Err("plan accepted Crystal Ascent membership outside its exact claim".into());
        }
        Ok(())
    }

    #[test]
    fn bounded_scalar_provenance_requires_exact_target_and_reference_membership(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let template = crate::template::grand_v3_reference_template()?;
        let hill_rule = template
            .bounded_regions
            .iter()
            .find(|rule| {
                rule.targets
                    .contains(&BoundedTarget::Landform(LandformKind::Hill))
            })
            .cloned()
            .ok_or_else(|| std::io::Error::other("packaged template has no Hill rule"))?;
        let reference_coord = hill_rule
            .reference_mask
            .first()
            .copied()
            .ok_or_else(|| std::io::Error::other("Hill rule has no reference cell"))?;
        let reference_cell = template
            .cell(reference_coord)
            .ok_or_else(|| std::io::Error::other("Hill reference cell is absent"))?;

        let mut wrong_target_rule = hill_rule.clone();
        wrong_target_rule.targets = vec![BoundedTarget::Landform(LandformKind::Mountain)];
        let wrong_source = LayerProvenance::Bounded {
            rule: wrong_target_rule.id.clone(),
        };
        let wrong_rules = BTreeMap::from([(wrong_target_rule.id.as_str(), &wrong_target_rule)]);
        let mut issues = Vec::new();
        validate_layer_source(
            reference_cell,
            LayerSlot::Landform,
            &wrong_source,
            false,
            &BTreeMap::new(),
            &wrong_rules,
            &BTreeSet::new(),
            &mut issues,
        );
        if !issues
            .iter()
            .any(|issue| issue.code == ValidationCode::Provenance)
        {
            return Err("bounded provenance accepted a different scalar target".into());
        }

        let outside_coord = hill_rule
            .envelope
            .iter()
            .copied()
            .find(|coord| !hill_rule.reference_mask.contains(coord))
            .ok_or_else(|| std::io::Error::other("Hill rule has no envelope-only cell"))?;
        let mut outside_cell = template
            .cell(outside_coord)
            .cloned()
            .ok_or_else(|| std::io::Error::other("Hill envelope cell is absent"))?;
        outside_cell.facts.landform = LandformKind::Hill;
        let source = LayerProvenance::Bounded {
            rule: hill_rule.id.clone(),
        };
        let rules = BTreeMap::from([(hill_rule.id.as_str(), &hill_rule)]);
        let mut issues = Vec::new();
        validate_layer_source(
            &outside_cell,
            LayerSlot::Landform,
            &source,
            false,
            &BTreeMap::new(),
            &rules,
            &BTreeSet::new(),
            &mut issues,
        );
        if !issues
            .iter()
            .any(|issue| issue.code == ValidationCode::Provenance)
        {
            return Err("bounded provenance accepted an envelope-only cell".into());
        }
        Ok(())
    }

    #[test]
    fn reference_artifact_is_exact_and_fallback_requires_exhausted_candidates(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let template = crate::template::grand_v3_reference_template()?;
        let reference = crate::generator::reference_plan(&template, 9)?;
        let artifact = reference.plan;
        if validate_plan(&template, &artifact)? != reference.metrics {
            return Err("exact reference artifact metrics changed during validation".into());
        }
        let mut relabeled_parts = artifact.clone().into_parts();
        let first_cell = relabeled_parts
            .cells
            .first_mut()
            .ok_or_else(|| std::io::Error::other("reference artifact has no cells"))?;
        let owner = provenance_id(&first_cell.provenance.surface)
            .ok_or_else(|| std::io::Error::other("template source was already fallback"))?
            .clone();
        first_cell.provenance.surface = LayerProvenance::ReferenceFallback { source: owner };
        let relabeled_artifact = SchematicPlanV1::new(relabeled_parts)?;
        let relabeled_error = validate_plan(&template, &relabeled_artifact)
            .expect_err("a reference artifact must retain its original layer authorship");
        if !relabeled_error.issues().iter().any(|issue| {
            issue.code == ValidationCode::Provenance
                && issue.detail.contains("changed original authorship")
        }) {
            return Err(
                "relabeled reference artifact did not fail exact-authorship validation".into(),
            );
        }

        let mut fallback_parts = artifact.clone().into_parts();
        fallback_parts.provenance = crate::model::PlanProvenance::reference_fallback(9);
        for cell in &mut fallback_parts.cells {
            for source in [
                &mut cell.provenance.surface,
                &mut cell.provenance.landform,
                &mut cell.provenance.climate,
                &mut cell.provenance.vegetation,
                &mut cell.provenance.access,
            ] {
                let owner = provenance_id(source)
                    .ok_or_else(|| std::io::Error::other("template source was already fallback"))?
                    .clone();
                *source = LayerProvenance::ReferenceFallback { source: owner };
            }
            for overlay in &mut cell.provenance.overlays {
                let owner = provenance_id(&overlay.source)
                    .ok_or_else(|| std::io::Error::other("template source was already fallback"))?
                    .clone();
                overlay.source = LayerProvenance::ReferenceFallback { source: owner };
            }
        }
        for feature in &mut fallback_parts.features {
            feature.provenance = LayerProvenance::ReferenceFallback {
                source: feature.id.clone(),
            };
        }
        let ineligible_fallback = SchematicPlanV1::new(fallback_parts)?;
        let error = validate_plan(&template, &ineligible_fallback)
            .expect_err("a fallback must not validate while normal candidates exist");
        if !error.issues().iter().any(|issue| {
            issue.code == ValidationCode::Provenance
                && issue.detail.contains("32-candidate replay and selection")
        }) {
            return Err("ineligible fallback did not fail deterministic replay".into());
        }
        Ok(())
    }

    #[test]
    fn shore_coastline_is_legal_and_counted_by_overlay() -> Result<(), Box<dyn std::error::Error>> {
        let coord = SchematicCoord::new(0, 0, 0)?;
        let stream = StableId::new("stream/coastline")?;
        let source = LayerProvenance::Seeded {
            stream: stream.clone(),
        };
        let cell = CellPlan {
            id: crate::model::CellId::new(0)?,
            coord,
            facts: crate::model::CellFacts {
                surface: SurfaceKind::Land,
                landform: LandformKind::Shore,
                climate: ClimateKind::Temperate,
                vegetation: VegetationDensity::Sparse,
                access: crate::model::AccessIntent::Ordinary,
                overlays: vec![FeatureKind::Coastline],
            },
            provenance: crate::model::CellProvenance {
                surface: source.clone(),
                landform: source.clone(),
                climate: source.clone(),
                vegetation: source.clone(),
                access: source.clone(),
                overlays: vec![crate::model::OverlayProvenance {
                    feature: FeatureKind::Coastline,
                    source,
                }],
            },
        };
        let rule = BoundedRegionRule {
            id: StableId::new("rule/coastline")?,
            kind: BoundedRegionKind::Coastline,
            targets: vec![BoundedTarget::Overlay(FeatureKind::Coastline)],
            reference_mask: vec![coord],
            envelope: vec![coord],
            max_displacement: 2,
            baseline_count: 1,
            count: crate::model::CountRange { min: 1, max: 1 },
            components: crate::model::CountRange { min: 1, max: 1 },
            component_size: crate::model::CountRange { min: 1, max: 1 },
            coverage_percent: None,
        };

        let mut issues = Vec::new();
        validate_cell_semantics(&cell, &mut issues);
        let counted = [cell]
            .iter()
            .filter(|candidate| cell_matches_rule(candidate, &rule))
            .count();
        if !issues.is_empty()
            || counted != 1
            || !is_coastal_landform(LandformKind::Shore)
            || is_coastal_landform(LandformKind::Hill)
        {
            return Err("Shore coastline was rejected or omitted from overlay membership".into());
        }
        Ok(())
    }

    #[test]
    fn packaged_woodland_envelope_is_the_exact_eligible_dilation(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let template = crate::template::grand_v3_reference_template()?;
        let Some(rule) = template
            .bounded_regions
            .iter()
            .find(|rule| rule.kind == BoundedRegionKind::Woodland)
        else {
            return Err(std::io::Error::other("packaged template has no Woodland rule").into());
        };
        let expected = bounded_envelope(&rule.reference_mask, 2)?
            .into_iter()
            .filter(|coord| woodland_template_eligible(&template, *coord))
            .collect::<Vec<_>>();
        if rule.envelope != expected {
            return Err(std::io::Error::other(format!(
                "expected {} woodland envelope cells: {expected:?}",
                expected.len()
            ))
            .into());
        }
        Ok(())
    }

    fn blank_landmark_cells() -> Result<Vec<CellPlan>, Box<dyn std::error::Error>> {
        let mut cells = crate::template::grand_v3_reference_template()?.reference_cells;
        for cell in &mut cells {
            cell.facts = crate::model::CellFacts {
                surface: SurfaceKind::Land,
                landform: LandformKind::Mountain,
                climate: ClimateKind::Alpine,
                vegetation: VegetationDensity::Sparse,
                access: crate::model::AccessIntent::Ordinary,
                overlays: Vec::new(),
            };
        }
        Ok(cells)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "fixture helper spells every independent cell layer explicitly"
    )]
    fn set_landmark_cell(
        cells: &mut [CellPlan],
        q: i32,
        r: i32,
        surface: SurfaceKind,
        landform: LandformKind,
        climate: ClimateKind,
        vegetation: VegetationDensity,
        access: crate::model::AccessIntent,
        overlays: &[FeatureKind],
    ) {
        let index = canonical_coordinate_index(coord(q, r)).expect("landmark test coordinate");
        cells[index].facts = crate::model::CellFacts {
            surface,
            landform,
            climate,
            vegetation,
            access,
            overlays: overlays.to_vec(),
        };
        cells[index].facts.overlays.sort_unstable();
    }

    fn add_overlay(cells: &mut [CellPlan], q: i32, r: i32, feature: FeatureKind) {
        let index = canonical_coordinate_index(coord(q, r)).expect("landmark test coordinate");
        let overlays = &mut cells[index].facts.overlays;
        if let Err(insertion) = overlays.binary_search(&feature) {
            overlays.insert(insertion, feature);
        }
    }

    fn remove_overlay(cells: &mut [CellPlan], q: i32, r: i32, feature: FeatureKind) {
        let index = canonical_coordinate_index(coord(q, r)).expect("landmark test coordinate");
        let overlays = &mut cells[index].facts.overlays;
        if let Ok(removal) = overlays.binary_search(&feature) {
            overlays.remove(removal);
        }
    }

    fn fixed_claim(
        id: &str,
        kind: FeatureKind,
        cells: Vec<SchematicCoord>,
    ) -> Result<FeatureClaim, Box<dyn std::error::Error>> {
        let id = StableId::new(id)?;
        Ok(FeatureClaim {
            id: id.clone(),
            kind,
            provenance: LayerProvenance::Locked { claim: id },
            cells,
        })
    }

    fn coord(q: i32, r: i32) -> SchematicCoord {
        SchematicCoord::new(q, r, -q - r).expect("valid test cube coordinate")
    }

    fn cube_distance((aq, ar): Coord, (bq, br): Coord) -> u32 {
        let as_ = -aq - ar;
        let bs = -bq - br;
        aq.abs_diff(bq).max(ar.abs_diff(br)).max(as_.abs_diff(bs))
    }
}
