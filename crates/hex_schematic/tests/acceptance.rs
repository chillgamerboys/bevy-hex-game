//! Black-box acceptance for the strict Grand V3 schematic contract and CLI.
#![expect(
    clippy::panic_in_result_fn,
    reason = "black-box tests use Result for fallible setup and assertions for contract failures"
)]

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use hex_schematic::{
    canonical_cell_id, canonical_coordinates, generate, grand_v3_reference_template,
    reference_plan, semantic_fingerprint, validate_plan, validate_template, AccessIntent,
    BoundedRegionRule, BoundedTarget, CellPlan, FeatureKind, GeneratedSchematic, LandformKind,
    LayerProvenance, NetworkKind, NetworkNodeKind, SchematicCoord, SchematicMetricsV1,
    SchematicPlanV1, SchematicTemplateV1, StableId, SurfaceKind, VegetationDensity,
    CANDIDATE_ATTEMPTS, GRAND_V3_TEMPLATE_RON, SCHEMATIC_CELL_COUNT, SCHEMATIC_RADIUS,
    SCHEMATIC_SCHEMA_VERSION,
};

type TestResult = Result<(), Box<dyn Error>>;

static SCRATCH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct ScratchDirectory {
    path: PathBuf,
}

impl ScratchDirectory {
    fn new(label: &str) -> io::Result<Self> {
        let sequence = SCRATCH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "hex-schematic-acceptance-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ScratchDirectory {
    fn drop(&mut self) {
        drop(fs::remove_dir_all(&self.path));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VariationSignature {
    coast: Vec<SchematicCoord>,
    islands: Vec<SchematicCoord>,
    river: Vec<(String, Vec<SchematicCoord>)>,
    woodland: Vec<(SchematicCoord, VegetationDensity)>,
}

#[test]
fn canonical_radius_eight_geometry_is_exact_and_bidirectional() -> TestResult {
    let coordinates = canonical_coordinates();
    assert_eq!(SCHEMATIC_SCHEMA_VERSION, 1);
    assert_eq!(SCHEMATIC_RADIUS, 8);
    assert_eq!(SCHEMATIC_CELL_COUNT, 217);
    assert_eq!(coordinates.len(), SCHEMATIC_CELL_COUNT);
    assert_eq!(
        coordinates.first().copied(),
        Some(SchematicCoord::new(0, 0, 0)?)
    );

    let expected_first_ring = [
        SchematicCoord::new(1, -1, 0)?,
        SchematicCoord::new(1, 0, -1)?,
        SchematicCoord::new(0, 1, -1)?,
        SchematicCoord::new(-1, 1, 0)?,
        SchematicCoord::new(-1, 0, 1)?,
        SchematicCoord::new(0, -1, 1)?,
    ];
    assert_eq!(coordinates.get(1..7), Some(expected_first_ring.as_slice()));

    let origin = SchematicCoord::new(0, 0, 0)?;
    let unique = coordinates.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(unique.len(), SCHEMATIC_CELL_COUNT);
    for (ordinal, coord) in coordinates.iter().copied().enumerate() {
        assert_eq!(
            i64::from(coord.q()) + i64::from(coord.r()) + i64::from(coord.s()),
            0
        );
        assert!(origin
            .checked_distance(coord)
            .is_some_and(|distance| distance <= 8));
        assert_eq!(
            canonical_cell_id(coord).map(|id| usize::from(id.get())),
            Some(ordinal)
        );
    }
    for radius in 0_u32..=8 {
        let expected = if radius == 0 { 1 } else { 6 * radius };
        assert_eq!(
            coordinates
                .iter()
                .filter(|coord| origin.checked_distance(**coord) == Some(radius))
                .count(),
            usize::try_from(expected)?,
        );
        if radius > 0 {
            let start = usize::try_from(1 + 3 * (radius - 1) * radius)?;
            let end = start + usize::try_from(6 * radius)? - 1;
            let radius = i32::try_from(radius)?;
            assert_eq!(
                coordinates.get(start).copied(),
                Some(SchematicCoord::new(radius, -radius, 0)?)
            );
            assert_eq!(
                coordinates.get(end).copied(),
                Some(SchematicCoord::new(radius - 1, -radius, 1)?)
            );
        }
    }

    let mut directed_internal_adjacencies = 0_usize;
    let mut outward_sides = 0_usize;
    for coord in &coordinates {
        for neighbor in coord
            .neighbors()
            .ok_or_else(|| io::Error::other("canonical neighbor construction overflowed"))?
        {
            if unique.contains(&neighbor) {
                directed_internal_adjacencies += 1;
                assert!(neighbor
                    .neighbors()
                    .is_some_and(|neighbors| neighbors.contains(coord)));
            } else {
                outward_sides += 1;
            }
        }
    }
    assert_eq!(directed_internal_adjacencies / 2, 600);
    assert_eq!(outward_sides, 102);
    assert_eq!(
        coordinates
            .iter()
            .filter(|coord| origin.checked_distance(**coord) == Some(8))
            .count(),
        48,
    );
    assert!(SchematicCoord::new(1, 1, 1).is_err());
    assert!(SchematicCoord::from_axial(i32::MAX, i32::MAX).is_err());
    assert_eq!(canonical_cell_id(SchematicCoord::new(9, -9, 0)?), None);
    Ok(())
}

#[test]
fn primitive_ron_contracts_reject_unknown_and_malformed_values() {
    assert!(ron::from_str::<SchematicCoord>("(q: 1, r: 1, s: 1)").is_err());
    assert!(ron::from_str::<SchematicCoord>("(q: 0, r: 0, s: 0, extra: 1)").is_err());
    assert!(ron::from_str::<StableId>("\"Bad/Identifier\"").is_err());
    assert!(ron::from_str::<StableId>("\"double//segment\"").is_err());
    assert!(ron::from_str::<StableId>("\"valid/kebab-2\"").is_ok());
}

#[test]
fn grid_cli_atomically_publishes_one_neutral_complete_projection() -> TestResult {
    let scratch = ScratchDirectory::new("grid-cli")?;
    let destination = scratch.path().join("grid");
    require_success(
        invoke(&[
            OsString::from("grid"),
            OsString::from("--output"),
            destination.as_os_str().to_owned(),
        ])?,
        "grid",
    )?;
    let ron = fs::read(destination.join("grid.ron"))?;
    let svg = fs::read_to_string(destination.join("grid.svg"))?;
    assert_eq!(fs::read_dir(&destination)?.count(), 2);
    assert_eq!(
        svg.matches("role=\"group\" aria-label=\"Cell ").count(),
        217
    );
    assert_eq!(
        svg.matches("class=\"authorship-outline authorship-grid\"")
            .count(),
        217,
    );
    assert!(!svg.contains("class=\"authorship-outline authorship-locked\" points="));

    let rejected = invoke(&[
        OsString::from("grid"),
        OsString::from("--output"),
        destination.as_os_str().to_owned(),
    ])?;
    assert_eq!(rejected.status.code(), Some(1));
    assert_eq!(fs::read(destination.join("grid.ron"))?, ron);
    assert_eq!(fs::read_dir(&destination)?.count(), 2);
    assert!(fs::read_dir(scratch.path())?
        .filter_map(Result::ok)
        .all(|entry| !entry.file_name().to_string_lossy().contains(".staging")));
    Ok(())
}

#[test]
fn packaged_template_and_reference_plan_round_trip_strictly() -> TestResult {
    assert_eq!(
        fs::read(packaged_template_path())?,
        GRAND_V3_TEMPLATE_RON.as_bytes(),
        "the embedded template must come from this checkout, not copied Cargo output"
    );
    let template = grand_v3_reference_template()?;
    validate_template(&template)?;
    assert_eq!(template.schema_version, SCHEMATIC_SCHEMA_VERSION);
    assert_eq!(template.radius, SCHEMATIC_RADIUS);
    assert_eq!(template.reference_cells.len(), SCHEMATIC_CELL_COUNT);
    assert_eq!(
        template
            .reference_cells
            .iter()
            .map(|cell| cell.coord)
            .collect::<Vec<_>>(),
        canonical_coordinates(),
    );

    let reparsed_template: SchematicTemplateV1 = ron::from_str(GRAND_V3_TEMPLATE_RON)?;
    assert_eq!(reparsed_template, template);
    let template_wire = ron::ser::to_string(&template)?;
    assert_eq!(
        ron::from_str::<SchematicTemplateV1>(&template_wire)?,
        template
    );

    let reference = reference_plan(&template, 0)?;
    assert!(!reference.plan.provenance.used_reference_fallback);
    assert!(reference.plan.provenance.is_reference_artifact);
    assert_eq!(reference.plan.provenance.selected_candidate, None);
    assert_eq!(reference.plan.provenance.hard_valid_candidates, 0);
    assert_eq!(reference.plan.provenance.candidates_evaluated, 0);
    assert_eq!(reference.plan.cells, template.reference_cells);
    assert_eq!(reference.plan.features, template.fixed_claims);
    assert_eq!(
        reference.plan.semantic_fingerprint,
        semantic_fingerprint(&reference.plan)
    );
    assert_eq!(
        validate_plan(&template, &reference.plan)?,
        reference.metrics
    );
    assert_eq!(reference.plan.cells.len(), SCHEMATIC_CELL_COUNT);

    let plan_wire = ron::ser::to_string(&reference.plan)?;
    let reparsed_plan: SchematicPlanV1 = ron::from_str(&plan_wire)?;
    assert_eq!(reparsed_plan, reference.plan);
    let metrics_wire = ron::ser::to_string(&reference.metrics)?;
    let reparsed_metrics: SchematicMetricsV1 = ron::from_str(&metrics_wire)?;
    assert_eq!(reparsed_metrics, reference.metrics);
    Ok(())
}

#[test]
fn packaged_template_preserves_the_approved_trace_with_revision_three_access() -> TestResult {
    let template = grand_v3_reference_template()?;
    assert_eq!(template.revision, 3);
    let waterfall_gorge = template
        .reference_cells
        .iter()
        .find(|cell| cell.id.get() == 63)
        .expect("revision 3 retains the authored waterfall-gorge cell");
    assert_eq!(waterfall_gorge.facts.access, AccessIntent::Scenic);
    assert!(matches!(
        waterfall_gorge.provenance.access,
        LayerProvenance::Locked { .. }
    ));
    for cell_id in [127, 128, 214, 215] {
        let backdrop = template
            .reference_cells
            .iter()
            .find(|cell| cell.id.get() == cell_id)
            .expect("revision 3 retains each outer Peak-backdrop shelf");
        assert_eq!(backdrop.facts.surface, SurfaceKind::Land);
        assert_eq!(backdrop.facts.landform, LandformKind::Mountain);
        assert_eq!(backdrop.facts.access, AccessIntent::Scenic);
        assert!(backdrop.facts.overlays.is_empty());
    }
    assert_eq!(
        template
            .reference_cells
            .iter()
            .filter(|cell| cell.facts.access == AccessIntent::Ordinary)
            .count(),
        175
    );
    assert_eq!(
        template
            .reference_cells
            .iter()
            .filter(|cell| cell.facts.access == AccessIntent::Scenic)
            .count(),
        42
    );
    let expected_columns = [
        (-8, "GGGTTBBBB"),
        (-7, "GGGGGTBBOB"),
        (-6, "PGGGGGTTBOB"),
        (-5, "PPPPGGYYTBBB"),
        (-4, "PPPPPPGYYTBBB"),
        (-3, "AAAPPPGGGYTBOB"),
        (-2, "AAAAAPPGGYTBBBB"),
        (-1, "AAAAAAPPPGYTBTTT"),
        (0, "AAAAPPAPPPGYTTYYY"),
        (1, "AARPKPPPPGGYYYYG"),
        (2, "AUUKKKPPPGYYYGG"),
        (3, "AUTBBKPPGYYGGG"),
        (4, "AKBOBKPBYYGGG"),
        (5, "PKBBBABBYGPP"),
        (6, "PKKKKPBYGGP"),
        (7, "PPPPPPPPGP"),
        (8, "PPPPPPPPP"),
    ];
    let mut projected_cells = 0_usize;
    for (q, expected) in expected_columns {
        let first_r = (-8_i32).max(-q - 8);
        let actual = expected
            .chars()
            .enumerate()
            .map(|(offset, _)| -> Result<char, Box<dyn Error>> {
                let r = first_r + i32::try_from(offset)?;
                let coord = SchematicCoord::from_axial(q, r)?;
                let cell = template
                    .cell(coord)
                    .ok_or_else(|| io::Error::other(format!("source column omitted ({q}, {r})")))?;
                source_trace_code(cell).ok_or_else(|| {
                    io::Error::other(format!(
                        "source projection has no literal category for ({q}, {r})"
                    ))
                    .into()
                })
            })
            .collect::<Result<String, Box<dyn Error>>>()?;
        projected_cells = projected_cells.saturating_add(actual.len());
        assert_eq!(actual, expected, "literal source column q={q} moved");
    }
    assert_eq!(projected_cells, SCHEMATIC_CELL_COUNT);
    Ok(())
}

#[test]
fn strict_ron_rejects_unknown_duplicate_and_malformed_data() -> TestResult {
    let unknown_template = inject_top_level_field(GRAND_V3_TEMPLATE_RON, "unexpected: true,")?;
    assert!(ron::from_str::<SchematicTemplateV1>(&unknown_template).is_err());
    let duplicate_template = inject_top_level_field(GRAND_V3_TEMPLATE_RON, "schema_version: 1,")?;
    assert!(ron::from_str::<SchematicTemplateV1>(&duplicate_template).is_err());
    let template = grand_v3_reference_template()?;
    let generated = generate(&template, 17)?;
    let plan_wire = ron::ser::to_string(&generated.plan)?;
    let unknown_plan = inject_top_level_field(&plan_wire, "unexpected: true,")?;
    assert!(ron::from_str::<SchematicPlanV1>(&unknown_plan).is_err());
    let metrics_wire = ron::ser::to_string(&generated.metrics)?;
    let unknown_metrics = inject_top_level_field(&metrics_wire, "unexpected: true,")?;
    assert!(ron::from_str::<SchematicMetricsV1>(&unknown_metrics).is_err());

    let mut unsupported_template_schema = template.clone();
    unsupported_template_schema.schema_version = SCHEMATIC_SCHEMA_VERSION.saturating_add(1);
    assert!(ron::from_str::<SchematicTemplateV1>(&ron::ser::to_string(
        &unsupported_template_schema
    )?)
    .is_err());
    let mut unsupported_plan_schema = generated.plan.clone();
    unsupported_plan_schema.schema_version = SCHEMATIC_SCHEMA_VERSION.saturating_add(1);
    assert!(
        ron::from_str::<SchematicPlanV1>(&ron::ser::to_string(&unsupported_plan_schema)?).is_err()
    );

    let first_claim_id = template
        .fixed_claims
        .first()
        .ok_or_else(|| io::Error::other("template unexpectedly had no fixed claims"))?
        .id
        .clone();
    let mut duplicate_claim_name = template.clone();
    duplicate_claim_name
        .fixed_claims
        .get_mut(1)
        .ok_or_else(|| io::Error::other("template unexpectedly had fewer than two claims"))?
        .id = first_claim_id;
    assert!(
        ron::from_str::<SchematicTemplateV1>(&ron::ser::to_string(&duplicate_claim_name)?).is_err()
    );

    let first_rule_id = template
        .bounded_regions
        .first()
        .ok_or_else(|| io::Error::other("template unexpectedly had no bounded rules"))?
        .id
        .clone();
    let mut duplicate_rule_name = template.clone();
    duplicate_rule_name
        .bounded_regions
        .get_mut(1)
        .ok_or_else(|| io::Error::other("template unexpectedly had fewer than two rules"))?
        .id = first_rule_id;
    assert!(
        ron::from_str::<SchematicTemplateV1>(&ron::ser::to_string(&duplicate_rule_name)?).is_err()
    );

    let first_network_id = template
        .networks
        .first()
        .ok_or_else(|| io::Error::other("template unexpectedly had no networks"))?
        .id
        .clone();
    let mut duplicate_network_name = template.clone();
    duplicate_network_name
        .networks
        .get_mut(1)
        .ok_or_else(|| io::Error::other("template unexpectedly had fewer than two networks"))?
        .id = first_network_id;
    assert!(
        ron::from_str::<SchematicTemplateV1>(&ron::ser::to_string(&duplicate_network_name)?)
            .is_err()
    );

    let mut duplicate_parts = generated.plan.clone().into_parts();
    let duplicate = duplicate_parts
        .cells
        .first()
        .cloned()
        .ok_or_else(|| io::Error::other("generated plan unexpectedly had no cells"))?;
    let second = duplicate_parts
        .cells
        .get_mut(1)
        .ok_or_else(|| io::Error::other("generated plan unexpectedly had fewer than two cells"))?;
    *second = duplicate;
    assert!(SchematicPlanV1::new(duplicate_parts).is_err());

    let mut bad_fingerprint_parts = generated.plan.into_parts();
    bad_fingerprint_parts.semantic_fingerprint ^= 1;
    let bad_fingerprint = SchematicPlanV1::new(bad_fingerprint_parts)?;
    assert!(validate_plan(&template, &bad_fingerprint).is_err());
    Ok(())
}

#[test]
fn fixed_claims_locked_layers_and_exact_tunnel_survive_generation() -> TestResult {
    let template = grand_v3_reference_template()?;
    validate_template(&template)?;
    assert_golden_locked_footprints(&template)?;
    let required_features = BTreeSet::from([
        FeatureKind::Waterfall,
        FeatureKind::MountainLake,
        FeatureKind::LakeIsland,
        FeatureKind::FrozenWoods,
        FeatureKind::PeakRing,
        FeatureKind::CrystalAscent,
        FeatureKind::Tunnel,
    ]);
    assert_eq!(
        template
            .fixed_claims
            .iter()
            .map(|claim| claim.kind)
            .collect::<BTreeSet<_>>(),
        required_features,
    );
    assert_eq!(
        template
            .networks
            .iter()
            .filter(|network| network.kind == NetworkKind::Hydrology)
            .count(),
        1,
    );
    assert_eq!(
        template
            .networks
            .iter()
            .filter(|network| network.kind == NetworkKind::Tunnel)
            .count(),
        1,
    );

    let expected_tunnel = template
        .networks
        .iter()
        .find(|network| network.kind == NetworkKind::Tunnel)
        .ok_or_else(|| io::Error::other("template omitted its exact tunnel network"))?;
    for seed in [0, 1, 2, 17, 42, 255, u64::MAX] {
        let generated = generate(&template, seed)?;
        assert!(
            !generated.plan.provenance.used_reference_fallback,
            "seed {seed} unexpectedly used the reference fallback"
        );
        assert_locked_contract(&template, &generated.plan);
        let actual_tunnel = generated
            .plan
            .networks
            .iter()
            .find(|network| network.kind == NetworkKind::Tunnel)
            .ok_or_else(|| io::Error::other("generated plan omitted its tunnel network"))?;
        assert_eq!(actual_tunnel, expected_tunnel);
    }
    Ok(())
}

#[test]
fn generation_is_repeatable_and_semantic_fingerprints_ignore_provenance() -> TestResult {
    let template = grand_v3_reference_template()?;
    for seed in [0, 1, 42, 1_000_003, u64::MAX] {
        let first = generate(&template, seed)?;
        let second = generate(&template, seed)?;
        assert_eq!(first, second);
        assert_eq!(
            first.plan.semantic_fingerprint,
            semantic_fingerprint(&first.plan)
        );
    }

    let first_reference = reference_plan(&template, 1)?;
    let second_reference = reference_plan(&template, u64::MAX)?;
    assert_ne!(
        first_reference.plan.provenance,
        second_reference.plan.provenance
    );
    assert_eq!(
        first_reference.plan.semantic_fingerprint,
        second_reference.plan.semantic_fingerprint,
    );
    Ok(())
}

#[test]
fn vegetation_only_reference_change_does_not_shift_other_named_streams() -> TestResult {
    let template = grand_v3_reference_template()?;
    let mut variant = template.clone();
    let vegetation_mutation_envelopes = variant
        .bounded_regions
        .iter()
        .filter(|rule| {
            rule.targets.iter().any(|target| {
                matches!(
                    target,
                    BoundedTarget::Surface(_)
                        | BoundedTarget::Vegetation(_)
                        | BoundedTarget::Vegetated
                )
            })
        })
        .flat_map(|rule| rule.envelope.iter().copied())
        .collect::<BTreeSet<_>>();
    let changed_cell = variant
        .reference_cells
        .iter_mut()
        .find(|cell| {
            cell.facts.surface == hex_schematic::SurfaceKind::Land
                && matches!(
                    cell.facts.vegetation,
                    VegetationDensity::None | VegetationDensity::Sparse
                )
                && matches!(
                    &cell.provenance.vegetation,
                    LayerProvenance::Seeded { stream }
                        if stream.as_str() == "stream/vegetation"
                )
                && !vegetation_mutation_envelopes.contains(&cell.coord)
        })
        .ok_or_else(|| {
            io::Error::other("template has no isolated mutable vegetation reference cell")
        })?;
    changed_cell.facts.vegetation = match changed_cell.facts.vegetation {
        VegetationDensity::None => VegetationDensity::Sparse,
        VegetationDensity::Sparse => VegetationDensity::None,
        VegetationDensity::Light | VegetationDensity::Moderate | VegetationDensity::Dense => {
            return Err(io::Error::other("isolated cell had woodland vegetation").into());
        }
    };
    validate_template(&variant)?;

    let mut observed_vegetation_change = false;
    for seed in 0..32 {
        let baseline = generate(&template, seed)?;
        let modified = generate(&variant, seed)?;
        assert_eq!(baseline.plan.provenance, modified.plan.provenance);
        assert_non_vegetation_streams_equal(&baseline.plan, &modified.plan);
        observed_vegetation_change |= baseline
            .plan
            .cells
            .iter()
            .zip(&modified.plan.cells)
            .any(|(left, right)| left.facts.vegetation != right.facts.vegetation);
    }
    assert!(observed_vegetation_change);
    Ok(())
}

#[test]
fn hydrology_is_final_before_scenic_island_placement() -> TestResult {
    let template = grand_v3_reference_template()?;
    for seed in 0..32_u64 {
        let generated = generate(&template, seed)?;
        let hydrology_path = generated
            .plan
            .networks
            .iter()
            .filter(|network| network.kind == NetworkKind::Hydrology)
            .flat_map(|network| &network.edges)
            .flat_map(|edge| edge.path.iter().copied())
            .collect::<BTreeSet<_>>();
        let islands = generated
            .plan
            .cells
            .iter()
            .filter(|cell| cell.facts.overlays.contains(&FeatureKind::SeaIsland))
            .map(|cell| cell.coord)
            .collect::<BTreeSet<_>>();
        assert!(
            hydrology_path.is_disjoint(&islands),
            "seed {seed} placed a scenic island on resolved hydrology"
        );
        for coord in hydrology_path {
            let cell = generated
                .plan
                .cell(coord)
                .ok_or_else(|| io::Error::other("hydrology path left the canonical grid"))?;
            assert_ne!(cell.facts.landform, hex_schematic::LandformKind::Island);
            assert!(!cell.facts.overlays.contains(&FeatureKind::SeaIsland));
        }
    }
    Ok(())
}

#[test]
fn normal_ci_corpus_of_256_seeds_is_valid_unique_and_varied() -> TestResult {
    let template = grand_v3_reference_template()?;
    validate_template(&template)?;
    const REQUIRED_VARIABLE_RULES: [&str; 8] = [
        "rule/valley-lake",
        "rule/massif",
        "rule/mountain",
        "rule/hill",
        "rule/valley",
        "rule/plateau",
        "rule/beach",
        "rule/shore",
    ];
    let varying_rules = template
        .bounded_regions
        .iter()
        .filter(|rule| REQUIRED_VARIABLE_RULES.contains(&rule.id.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        varying_rules
            .iter()
            .map(|rule| rule.id.as_str())
            .collect::<BTreeSet<_>>(),
        REQUIRED_VARIABLE_RULES.into_iter().collect::<BTreeSet<_>>(),
    );
    let mut fingerprints = HashSet::with_capacity(256);
    let mut coast_variants = BTreeSet::new();
    let mut island_group_counts = BTreeSet::new();
    let mut woodland_percentages = BTreeSet::new();
    let mut bounded_rule_variants = varying_rules
        .iter()
        .map(|rule| (rule.id.to_string(), BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    for seed in 0..256_u64 {
        let generated = generate(&template, seed)?;
        assert_normal_generated(&template, &generated, seed);
        assert!(
            fingerprints.insert(generated.plan.semantic_fingerprint),
            "seed {seed} duplicated an earlier semantic fingerprint"
        );
        let signature = variation_signature(&generated.plan);
        coast_variants.insert(signature.coast);
        island_group_counts.insert(generated.metrics.sea_island_groups);
        woodland_percentages.insert(generated.metrics.woodland_percent);
        for rule in &varying_rules {
            bounded_rule_variants
                .get_mut(rule.id.as_str())
                .expect("required rule was initialized above")
                .insert(selected_rule_mask(&generated.plan, rule));
        }
    }
    assert_eq!(fingerprints.len(), 256);
    assert!(
        coast_variants.len() >= 2
            && island_group_counts.len() >= 2
            && woodland_percentages.len() >= 2,
        "256-seed diversity was coast={}, island-group-counts={island_group_counts:?}, woodland-percentages={woodland_percentages:?}",
        coast_variants.len(),
    );
    for (rule, variants) in bounded_rule_variants {
        assert!(
            variants.len() >= 2,
            "{rule} resolved to only {} coordinate mask(s) across 256 selected plans",
            variants.len(),
        );
    }
    Ok(())
}

#[test]
fn cli_is_strict_deterministic_and_publishes_complete_atomic_bundles() -> TestResult {
    let scratch = ScratchDirectory::new("cli")?;
    let template_path = packaged_template_path();
    let first_output = scratch.path().join("seed-a");
    let second_output = scratch.path().join("seed-b");
    require_success(
        invoke(&generate_arguments(&template_path, 42, &first_output))?,
        "first generate",
    )?;
    require_success(
        invoke(&generate_arguments(&template_path, 42, &second_output))?,
        "second generate",
    )?;
    for file in [
        "plan.ron",
        "metrics.ron",
        "composite.svg",
        "diagnostics.svg",
    ] {
        assert_eq!(
            fs::read(first_output.join(file))?,
            fs::read(second_output.join(file))?
        );
    }
    let template = grand_v3_reference_template()?;
    let plan: SchematicPlanV1 = ron::from_str(&fs::read_to_string(first_output.join("plan.ron"))?)?;
    let metrics: SchematicMetricsV1 =
        ron::from_str(&fs::read_to_string(first_output.join("metrics.ron"))?)?;
    assert_eq!(validate_plan(&template, &plan)?, metrics);
    require_success(
        invoke(&[
            OsString::from("validate"),
            OsString::from("--template"),
            template_path.as_os_str().to_owned(),
            OsString::from("--plan"),
            first_output.join("plan.ron").into_os_string(),
        ])?,
        "validate with inferred sibling metrics",
    )?;

    let before = fs::read(first_output.join("plan.ron"))?;
    let rejected = invoke(&generate_arguments(&template_path, 42, &first_output))?;
    assert_eq!(rejected.status.code(), Some(1));
    assert_eq!(fs::read(first_output.join("plan.ron"))?, before);

    let invalid_template = scratch.path().join("invalid-template.ron");
    fs::write(
        &invalid_template,
        inject_top_level_field(GRAND_V3_TEMPLATE_RON, "unknown: true,")?,
    )?;
    let absent_output = scratch.path().join("must-stay-absent");
    let invalid = invoke(&generate_arguments(&invalid_template, 7, &absent_output))?;
    assert_eq!(invalid.status.code(), Some(1));
    assert!(!absent_output.exists());

    let gallery = scratch.path().join("gallery");
    require_success(
        invoke(&[
            OsString::from("gallery"),
            OsString::from("--template"),
            template_path.as_os_str().to_owned(),
            OsString::from("--first-seed"),
            OsString::from("9"),
            OsString::from("--output"),
            gallery.as_os_str().to_owned(),
        ])?,
        "gallery",
    )?;
    assert_complete_gallery(&gallery, 9)?;
    let gallery_index = fs::read(gallery.join("index.html"))?;
    let rejected_gallery = invoke(&[
        OsString::from("gallery"),
        OsString::from("--template"),
        template_path.as_os_str().to_owned(),
        OsString::from("--first-seed"),
        OsString::from("21"),
        OsString::from("--output"),
        gallery.as_os_str().to_owned(),
    ])?;
    assert_eq!(rejected_gallery.status.code(), Some(1));
    assert_eq!(fs::read(gallery.join("index.html"))?, gallery_index);
    assert_complete_gallery(&gallery, 9)?;
    Ok(())
}

#[test]
fn cli_rejects_unknown_duplicate_missing_positional_and_overflow_inputs() -> TestResult {
    let scratch = ScratchDirectory::new("cli-syntax")?;
    let template_path = packaged_template_path();
    let cases = vec![
        vec![OsString::from("unknown")],
        vec![OsString::from("grid")],
        vec![
            OsString::from("grid"),
            OsString::from("--unknown"),
            OsString::from("value"),
            OsString::from("--output"),
            scratch.path().join("unknown-flag").into_os_string(),
        ],
        vec![
            OsString::from("grid"),
            OsString::from("--output"),
            scratch.path().join("a").into_os_string(),
            OsString::from("--output"),
            scratch.path().join("b").into_os_string(),
        ],
        vec![
            OsString::from("generate"),
            OsString::from("positional"),
            OsString::from("--template"),
            template_path.as_os_str().to_owned(),
            OsString::from("--seed"),
            OsString::from("1"),
            OsString::from("--output"),
            scratch.path().join("c").into_os_string(),
        ],
        vec![
            OsString::from("validate"),
            OsString::from("--template"),
            template_path.as_os_str().to_owned(),
            OsString::from("--metrics"),
            OsString::from("metrics.ron"),
        ],
        vec![
            OsString::from("gallery"),
            OsString::from("--template"),
            template_path.as_os_str().to_owned(),
            OsString::from("--first-seed"),
            OsString::from(u64::MAX.to_string()),
            OsString::from("--output"),
            scratch.path().join("overflow").into_os_string(),
        ],
    ];
    for arguments in cases {
        let output = invoke(&arguments)?;
        assert_eq!(output.status.code(), Some(2));
    }
    assert_eq!(fs::read_dir(scratch.path())?.count(), 0);
    Ok(())
}

#[test]
#[ignore = "release-only: run with `cargo test --release -p hex_schematic --test acceptance release_corpus_diversity_performance_and_memory_gate -- --ignored --exact --test-threads=1`"]
fn release_corpus_diversity_performance_and_memory_gate() -> TestResult {
    if cfg!(debug_assertions) {
        return Err(io::Error::other("release corpus must use --release").into());
    }
    let template = grand_v3_reference_template()?;
    validate_template(&template)?;
    let mut fingerprints = HashSet::with_capacity(10_000);
    let mut group_counts = BTreeSet::new();
    let mut group_sizes = BTreeSet::new();
    let mut prior_signature = None;
    let mut sufficiently_different_pairs = 0_u32;
    let mut durations = Vec::with_capacity(10_000);

    for seed in 0..10_000_u64 {
        let started = Instant::now();
        let generated = generate(&template, seed)?;
        durations.push(started.elapsed());
        assert_normal_generated(&template, &generated, seed);
        assert!(
            fingerprints.insert(generated.plan.semantic_fingerprint),
            "seed {seed} duplicated an earlier semantic fingerprint"
        );
        group_counts.insert(generated.metrics.sea_island_groups);
        group_sizes.extend(sea_island_component_sizes(&generated.plan));
        let signature = variation_signature(&generated.plan);
        if prior_signature
            .as_ref()
            .is_some_and(|prior| differing_stream_count(prior, &signature) >= 2)
        {
            sufficiently_different_pairs += 1;
        }
        prior_signature = Some(signature);
    }

    assert_eq!(fingerprints.len(), 10_000);
    assert_eq!(group_counts, BTreeSet::from([2, 3, 4, 5, 6]));
    assert_eq!(group_sizes, BTreeSet::from([1, 2, 3, 4]));
    assert!(sufficiently_different_pairs * 100 >= 90 * 9_999);

    durations.sort_unstable();
    let p95_index = durations
        .len()
        .saturating_mul(95)
        .div_ceil(100)
        .saturating_sub(1);
    let p95 = durations
        .get(p95_index)
        .copied()
        .ok_or_else(|| io::Error::other("release duration corpus was empty"))?;
    assert!(p95 < Duration::from_millis(50), "release p95 was {p95:?}");
    if let Some(peak_kib) = linux_peak_resident_kib()? {
        assert!(peak_kib < 64 * 1_024, "VmHWM was {peak_kib} KiB");
    }
    Ok(())
}

fn inject_top_level_field(source: &str, field: &str) -> io::Result<String> {
    let offset = source
        .find('(')
        .ok_or_else(|| io::Error::other("RON document has no top-level struct opening"))?
        + 1;
    let mut mutated = String::with_capacity(source.len() + field.len() + 2);
    mutated.push_str(&source[..offset]);
    mutated.push('\n');
    mutated.push_str(field);
    mutated.push('\n');
    mutated.push_str(&source[offset..]);
    Ok(mutated)
}

fn assert_normal_generated(
    template: &SchematicTemplateV1,
    generated: &GeneratedSchematic,
    seed: u64,
) {
    assert_eq!(generated.plan.provenance.world_seed, seed);
    assert_eq!(
        generated.plan.provenance.candidates_evaluated,
        CANDIDATE_ATTEMPTS
    );
    assert!(generated.plan.provenance.hard_valid_candidates > 0);
    assert!(generated.plan.provenance.selected_candidate.is_some());
    assert!(
        !generated.plan.provenance.used_reference_fallback,
        "seed {seed} unexpectedly used the reference fallback"
    );
    assert_eq!(
        generated.plan.semantic_fingerprint,
        semantic_fingerprint(&generated.plan)
    );
    assert_eq!(
        validate_plan(template, &generated.plan),
        Ok(generated.metrics.clone())
    );
    assert_eq!(generated.metrics.cell_count, 217);
    assert_eq!(generated.metrics.internal_adjacencies, 600);
    assert_eq!(generated.metrics.boundary_cells, 48);
    assert_eq!(generated.metrics.outward_sides, 102);
    assert!((2..=6).contains(&generated.metrics.sea_island_groups));
    assert!((1..=4).contains(&generated.metrics.smallest_sea_island));
    assert!((1..=4).contains(&generated.metrics.largest_sea_island));
    assert!((30..=80).contains(&generated.metrics.woodland_percent));
    assert!(generated.metrics.maximum_coast_displacement <= 2);
    assert_locked_contract(template, &generated.plan);
}

fn assert_locked_contract(template: &SchematicTemplateV1, plan: &SchematicPlanV1) {
    assert_eq!(template.reference_cells.len(), plan.cells.len());
    for (reference, resolved) in template.reference_cells.iter().zip(&plan.cells) {
        assert_eq!(reference.id, resolved.id);
        assert_eq!(reference.coord, resolved.coord);
        if matches!(reference.provenance.surface, LayerProvenance::Locked { .. }) {
            assert_eq!(reference.facts.surface, resolved.facts.surface);
            assert_eq!(reference.provenance.surface, resolved.provenance.surface);
        }
        if matches!(
            reference.provenance.landform,
            LayerProvenance::Locked { .. }
        ) {
            assert_eq!(reference.facts.landform, resolved.facts.landform);
            assert_eq!(reference.provenance.landform, resolved.provenance.landform);
        }
        if matches!(reference.provenance.climate, LayerProvenance::Locked { .. }) {
            assert_eq!(reference.facts.climate, resolved.facts.climate);
            assert_eq!(reference.provenance.climate, resolved.provenance.climate);
        }
        if matches!(
            reference.provenance.vegetation,
            LayerProvenance::Locked { .. }
        ) {
            assert_eq!(reference.facts.vegetation, resolved.facts.vegetation);
            assert_eq!(
                reference.provenance.vegetation,
                resolved.provenance.vegetation
            );
        }
        if matches!(reference.provenance.access, LayerProvenance::Locked { .. }) {
            assert_eq!(reference.facts.access, resolved.facts.access);
            assert_eq!(reference.provenance.access, resolved.provenance.access);
        }
        for source in &reference.provenance.overlays {
            if !matches!(source.source, LayerProvenance::Locked { .. }) {
                continue;
            }
            let actual = resolved
                .provenance
                .overlays
                .iter()
                .find(|candidate| candidate.feature == source.feature);
            assert_eq!(actual, Some(source));
            assert!(resolved.facts.overlays.contains(&source.feature));
        }
    }
    for claim in &template.fixed_claims {
        assert!(plan.features.iter().any(|actual| actual == claim));
    }
}

fn assert_golden_locked_footprints(template: &SchematicTemplateV1) -> TestResult {
    let claims = template
        .fixed_claims
        .iter()
        .map(|claim| {
            (
                claim.id.as_str(),
                claim.kind,
                claim
                    .cells
                    .iter()
                    .copied()
                    .map(coord_tuple)
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        claims,
        vec![
            (
                "claim/crystal-ascent",
                FeatureKind::CrystalAscent,
                vec![(1, -6, 5)]
            ),
            (
                "claim/frozen-woods",
                FeatureKind::FrozenWoods,
                vec![(2, -6, 4), (3, -6, 3), (2, -7, 5), (3, -7, 4)],
            ),
            (
                "claim/lake-island",
                FeatureKind::LakeIsland,
                vec![(4, -5, 1)]
            ),
            (
                "claim/mountain-lake",
                FeatureKind::MountainLake,
                vec![
                    (4, -4, 0),
                    (3, -4, 1),
                    (5, -5, 0),
                    (5, -4, -1),
                    (3, -5, 2),
                    (4, -6, 2),
                    (5, -6, 1),
                ],
            ),
            (
                "claim/peak-ring",
                FeatureKind::PeakRing,
                vec![
                    (3, -3, 0),
                    (2, -3, 1),
                    (4, -3, -1),
                    (1, -4, 3),
                    (2, -4, 2),
                    (2, -5, 3),
                    (6, -6, 0),
                    (6, -5, -1),
                    (6, -4, -2),
                    (4, -7, 3),
                    (5, -7, 2),
                    (6, -7, 1),
                ],
            ),
            (
                "claim/tunnel",
                FeatureKind::Tunnel,
                vec![
                    (1, -1, 0),
                    (1, 0, -1),
                    (1, 1, -2),
                    (1, -2, 1),
                    (1, -3, 2),
                    (1, -4, 3),
                    (1, -5, 4),
                    (1, -6, 5),
                ],
            ),
            (
                "claim/waterfall",
                FeatureKind::Waterfall,
                vec![(5, -4, -1), (5, -3, -2), (5, -2, -3)],
            ),
        ]
    );

    let tunnel = template
        .networks
        .iter()
        .find(|network| network.kind == NetworkKind::Tunnel)
        .ok_or_else(|| io::Error::other("validated template omitted the tunnel network"))?;
    assert_eq!(tunnel.id.as_str(), "network/tunnel");
    assert_eq!(
        tunnel
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node.kind, coord_tuple(node.coord)))
            .collect::<Vec<_>>(),
        vec![
            ("node/tunnel-ascent", NetworkNodeKind::Source, (1, -6, 5),),
            (
                "node/tunnel-hill-terminal",
                NetworkNodeKind::Sink,
                (1, 1, -2),
            ),
        ]
    );
    assert_eq!(tunnel.edges.len(), 1);
    let edge = tunnel
        .edges
        .first()
        .ok_or_else(|| io::Error::other("validated tunnel network omitted its complete edge"))?;
    assert_eq!(edge.id.as_str(), "edge/tunnel-complete");
    assert_eq!(edge.from.as_str(), "node/tunnel-ascent");
    assert_eq!(edge.to.as_str(), "node/tunnel-hill-terminal");
    assert_eq!(
        edge.path
            .iter()
            .copied()
            .map(coord_tuple)
            .collect::<Vec<_>>(),
        vec![
            (1, -6, 5),
            (1, -5, 4),
            (1, -4, 3),
            (1, -3, 2),
            (1, -2, 1),
            (1, -1, 0),
            (1, 0, -1),
            (1, 1, -2),
        ]
    );
    Ok(())
}

fn source_trace_code(cell: &CellPlan) -> Option<char> {
    if cell.facts.overlays.contains(&FeatureKind::CrystalAscent) {
        return Some('R');
    }
    if cell.facts.overlays.contains(&FeatureKind::FrozenWoods)
        && cell.facts.landform != LandformKind::Shore
    {
        return Some('U');
    }
    if cell.facts.overlays.contains(&FeatureKind::PeakRing) {
        return Some('K');
    }
    if cell.facts.landform == LandformKind::Island {
        return Some('O');
    }
    if cell.facts.surface == SurfaceKind::OpenWater {
        return Some('B');
    }
    if cell.facts.landform == LandformKind::Massif
        || cell.facts.overlays.contains(&FeatureKind::Waterfall)
    {
        return Some('A');
    }
    match cell.facts.landform {
        LandformKind::Mountain => Some('P'),
        LandformKind::Hill => Some('G'),
        LandformKind::Valley => Some('Y'),
        LandformKind::Beach | LandformKind::Shore | LandformKind::Plateau => Some('T'),
        LandformKind::None
        | LandformKind::Island
        | LandformKind::Massif
        | LandformKind::SharpPeak => None,
    }
}

const fn coord_tuple(coord: SchematicCoord) -> (i32, i32, i32) {
    (coord.q(), coord.r(), coord.s())
}

fn assert_non_vegetation_streams_equal(left: &SchematicPlanV1, right: &SchematicPlanV1) {
    assert_eq!(left.template_id, right.template_id);
    assert_eq!(left.template_revision, right.template_revision);
    assert_eq!(left.features, right.features);
    assert_eq!(left.networks, right.networks);
    assert_eq!(left.cells.len(), right.cells.len());
    for (left, right) in left.cells.iter().zip(&right.cells) {
        assert_eq!(left.id, right.id);
        assert_eq!(left.coord, right.coord);
        assert_eq!(left.facts.surface, right.facts.surface);
        assert_eq!(left.facts.landform, right.facts.landform);
        assert_eq!(left.facts.climate, right.facts.climate);
        assert_eq!(left.facts.access, right.facts.access);
        assert_eq!(left.facts.overlays, right.facts.overlays);
        assert_eq!(left.provenance.surface, right.provenance.surface);
        assert_eq!(left.provenance.landform, right.provenance.landform);
        assert_eq!(left.provenance.climate, right.provenance.climate);
        assert_eq!(left.provenance.access, right.provenance.access);
        assert_eq!(left.provenance.overlays, right.provenance.overlays);
    }
}

fn selected_rule_mask(plan: &SchematicPlanV1, rule: &BoundedRegionRule) -> Vec<SchematicCoord> {
    plan.cells
        .iter()
        .filter(|cell| {
            rule.targets.iter().all(|target| match target {
                BoundedTarget::Surface(value) => cell.facts.surface == *value,
                BoundedTarget::Landform(value) => cell.facts.landform == *value,
                BoundedTarget::Climate(value) => cell.facts.climate == *value,
                BoundedTarget::Vegetation(value) => cell.facts.vegetation == *value,
                BoundedTarget::Vegetated => matches!(
                    cell.facts.vegetation,
                    VegetationDensity::Light
                        | VegetationDensity::Moderate
                        | VegetationDensity::Dense
                ),
                BoundedTarget::Access(value) => cell.facts.access == *value,
                BoundedTarget::Overlay(value) => cell.facts.overlays.binary_search(value).is_ok(),
            })
        })
        .map(|cell| cell.coord)
        .collect()
}

fn variation_signature(plan: &SchematicPlanV1) -> VariationSignature {
    let cells_with = |feature| {
        plan.cells
            .iter()
            .filter(|cell| cell.facts.overlays.contains(&feature))
            .map(|cell| cell.coord)
            .collect::<Vec<_>>()
    };
    let river = plan
        .networks
        .iter()
        .filter(|network| network.kind == NetworkKind::Hydrology)
        .flat_map(|network| {
            network
                .edges
                .iter()
                .map(|edge| (edge.id.to_string(), edge.path.clone()))
        })
        .collect();
    let woodland = plan
        .cells
        .iter()
        .filter(|cell| {
            matches!(
                cell.facts.vegetation,
                VegetationDensity::Light | VegetationDensity::Moderate | VegetationDensity::Dense
            ) && !cell.facts.overlays.contains(&FeatureKind::FrozenWoods)
        })
        .map(|cell| (cell.coord, cell.facts.vegetation))
        .collect();
    VariationSignature {
        coast: cells_with(FeatureKind::Coastline),
        islands: cells_with(FeatureKind::SeaIsland),
        river,
        woodland,
    }
}

fn differing_stream_count(left: &VariationSignature, right: &VariationSignature) -> u8 {
    u8::from(left.coast != right.coast)
        + u8::from(left.islands != right.islands)
        + u8::from(left.river != right.river)
        + u8::from(left.woodland != right.woodland)
}

fn sea_island_component_sizes(plan: &SchematicPlanV1) -> BTreeSet<u16> {
    let mut remaining = plan
        .cells
        .iter()
        .filter(|cell| cell.facts.overlays.contains(&FeatureKind::SeaIsland))
        .map(|cell| cell.coord)
        .collect::<BTreeSet<_>>();
    let mut sizes = BTreeSet::new();
    while let Some(start) = remaining.first().copied() {
        remaining.remove(&start);
        let mut pending = VecDeque::from([start]);
        let mut size = 0_u16;
        while let Some(cell) = pending.pop_front() {
            size = size.saturating_add(1);
            if let Some(neighbors) = cell.neighbors() {
                for neighbor in neighbors {
                    if remaining.remove(&neighbor) {
                        pending.push_back(neighbor);
                    }
                }
            }
        }
        sizes.insert(size);
    }
    sizes
}

fn packaged_template_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/config/schematics/grand-v3-template.ron")
}

fn generate_arguments(template: &Path, seed: u64, output: &Path) -> Vec<OsString> {
    vec![
        OsString::from("generate"),
        OsString::from("--template"),
        template.as_os_str().to_owned(),
        OsString::from("--seed"),
        OsString::from(seed.to_string()),
        OsString::from("--output"),
        output.as_os_str().to_owned(),
    ]
}

fn invoke(arguments: &[OsString]) -> io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_hex_schematic"))
        .args(arguments)
        .output()
}

fn require_success(output: Output, operation: &str) -> io::Result<Output> {
    if output.status.success() {
        Ok(output)
    } else {
        Err(io::Error::other(format!(
            "{operation} failed with {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

fn assert_complete_gallery(gallery: &Path, first_seed: u64) -> TestResult {
    assert_eq!(fs::read_dir(gallery)?.count(), 15);
    for seed in first_seed..=first_seed + 11 {
        let directory = gallery.join(format!("seed-{seed:020}"));
        assert_complete_bundle(&directory)?;
    }
    assert_complete_bundle(&gallery.join("reference"))?;
    let html = fs::read_to_string(gallery.join("index.html"))?;
    assert!(html.contains("Canonical reference artifact"));
    assert!(html.contains("reference/plan.ron"));
    assert_eq!(html.matches("class=\"card\"").count(), 12);
    let contact = fs::read_to_string(gallery.join("contact-sheet.svg"))?;
    assert_eq!(contact.matches("role=\"group\"").count(), 12);
    assert!(!contact.contains("<image"));
    assert_eq!(contact.matches("class=\"mini-cell ").count(), 12 * 217);
    assert!(contact.contains("fingerprint"));
    assert!(contact.contains("class=\"summary\""));
    Ok(())
}

fn assert_complete_bundle(directory: &Path) -> io::Result<()> {
    let files = fs::read_dir(directory)?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|entry| entry.file_name())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        files,
        BTreeSet::from([
            OsString::from("composite.svg"),
            OsString::from("diagnostics.svg"),
            OsString::from("metrics.ron"),
            OsString::from("plan.ron"),
        ])
    );
    Ok(())
}

fn linux_peak_resident_kib() -> io::Result<Option<u64>> {
    if !cfg!(target_os = "linux") {
        return Ok(None);
    }
    let status = fs::read_to_string("/proc/self/status")?;
    let Some(line) = status.lines().find(|line| line.starts_with("VmHWM:")) else {
        return Ok(None);
    };
    let value = line
        .split_ascii_whitespace()
        .nth(1)
        .ok_or_else(|| io::Error::other("VmHWM line omitted its numeric value"))?
        .parse()
        .map_err(|error| io::Error::other(format!("invalid VmHWM value: {error}")))?;
    Ok(Some(value))
}
