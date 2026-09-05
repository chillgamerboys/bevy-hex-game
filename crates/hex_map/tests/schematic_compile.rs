//! External contract for compiling the selected Grand V3 schematic artifact.

#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use bevy::prelude::World;
use hex_assets::{
    ArtPalette, ObjectBlueprint, ObjectCatalogFile, RuntimeArtCatalog, SubstanceFile,
    SubstanceTable, VoxelStyleCatalog,
};
use hex_map::{
    admit_schematic_topology, compile_schematic, export_world_snapshot_v1,
    fingerprint_world_snapshot_v1, CompiledSchematicMap, GenerationReport, MapSettings,
    ProceduralRecipeMetrics, V3GrandV3BasicTerrainProfile, VoxelMap,
};

const DEFAULT_HERO_SEED: u64 = 1_592_598_566;
const SCHEMATIC_CELLS: usize = hex_schematic::SCHEMATIC_CELL_COUNT;
const SCHEMATIC_CELLS_U32: u32 = 217;
const WORLD_COLUMNS: usize = 105_469;
const WORLD_COLUMNS_U32: u32 = 105_469;
const RESIDENT_CHUNKS: u32 = 444;
static FULL_WORLD_COMPILATION: Mutex<()> = Mutex::new(());

struct CompilationInputs {
    template: hex_schematic::SchematicTemplateV1,
    settings: MapSettings,
    table: SubstanceTable,
    art_catalog: RuntimeArtCatalog,
}

#[test]
fn public_generated_hero_schematic_compiles_publishes_and_exports() {
    let _serial = full_world_compilation_guard();
    let inputs = compilation_inputs();
    let generated = hex_schematic::generate(&inputs.template, DEFAULT_HERO_SEED)
        .expect("the approved hero schematic should generate");
    assert_eq!(generated.plan.provenance.selected_candidate, Some(29));
    assert_eq!(generated.plan.semantic_fingerprint, 0xf8c7_b2a1_a177_a982);
    let seeded_valley_lake = hex_schematic::SchematicCoord::new(7, -2, -5)
        .expect("the seeded valley-lake coordinate is valid");
    let seeded_valley_lake = generated
        .plan
        .cell(seeded_valley_lake)
        .expect("the hero plan contains every radius-eight cell");
    assert_eq!(
        seeded_valley_lake.facts.surface,
        hex_schematic::SurfaceKind::OpenWater
    );
    assert_eq!(
        seeded_valley_lake.facts.access,
        hex_schematic::AccessIntent::Ordinary,
        "the layered fixture deliberately retains independently seeded ordinary access"
    );
    assert!(
        seeded_valley_lake
            .facts
            .overlays
            .contains(&hex_schematic::FeatureKind::ValleyLake),
        "the regression fixture must retain the seeded water cut across the preferred pass spine"
    );
    let compiled = compile_schematic(
        &generated.plan,
        &inputs.settings,
        &inputs.table,
        &inputs.art_catalog,
    )
    .expect("the exact public plan should compile");
    assert_hero_camera_anchor_positions(&compiled);
    let party_start = compiled
        .anchors
        .get(&hex_core::MapAnchorId::from("party_start"))
        .expect("the Grand world should publish its exact party start");
    let coast = compiled
        .anchors
        .get(&hex_core::MapAnchorId::from("grand_v3.coast"))
        .expect("the Grand world should publish its coast review anchor");
    assert_eq!(
        party_start, coast,
        "the seed-exact camera route starts by proving the shipped coastal spawn"
    );
    assert_eq!(
        party_start,
        hex_core::TilePos::new(hex_core::HexCoord::from_axial(-21, 99), 10),
        "moving the hero-seed coastal spawn requires a fresh camera-route review"
    );
    assert_complete_world_contract(
        &compiled,
        DEFAULT_HERO_SEED,
        generated.plan.semantic_fingerprint,
    );
    // Retain these identities before publication consumes the compiled plan.
    // Check all three together after the independent publication/export facts.
    let compiled_semantic_fingerprint = compiled.report.semantic_plan_fingerprint;
    let materialized_map_fingerprint = compiled.report.map_fingerprint;
    assert_eq!(compiled.map.len(), WORLD_COLUMNS);
    let presentation = compiled.presentation_counts();
    assert!(presentation.liquids > 30_000);
    assert!(
        presentation.features > 0,
        "the exact Crystal and vegetation presentation must be retained"
    );
    assert!(
        presentation.structures >= 3,
        "two bridges and the exact Crystal architecture must be retained"
    );
    assert!(
        presentation.lights > 0,
        "the exact Crystal gameplay lights must be retained"
    );
    assert!(
        compiled.report.metrics.reachable_surfaces > 0,
        "the final report must publish measured ordinary reachability"
    );
    assert!(
        compiled.report.metrics.reachable_elevation_levels > 1,
        "the final report must publish measured vertical traversal"
    );

    let mut world = World::new();
    world.insert_resource(inputs.settings);
    world.insert_resource(inputs.table);
    world.insert_resource(inputs.art_catalog);
    compiled.publish(&mut world);
    assert!(
        !world.contains_resource::<hex_core::TerrainReady>(),
        "resource publication cannot claim readiness before chunk roots exist"
    );
    assert_eq!(world.resource::<VoxelMap>().len(), WORLD_COLUMNS);
    assert_eq!(world.resource::<GenerationReport>().seed, DEFAULT_HERO_SEED);

    let snapshot = export_world_snapshot_v1(&world).expect("published exact plan should export");
    let exported_snapshot_fingerprint = fingerprint_world_snapshot_v1(&snapshot)
        .expect("the exported Grand V3 snapshot should fingerprint")
        .0;
    assert_eq!(snapshot.columns.len(), WORLD_COLUMNS);
    assert_eq!(snapshot.liquids.len(), presentation.liquids);
    assert_eq!(snapshot.version, 1);
    for scenic in [
        "grand_v3.lake_island",
        "grand_v3.massif_crest",
        "grand_v3.waterfall_base",
        "grand_v3.waterfall_crown",
    ] {
        assert!(
            snapshot
                .anchors
                .iter()
                .all(|anchor| anchor.name.as_str() != scenic),
            "review-only landmark {scenic} must not enter gameplay Snapshot V1"
        );
    }
    let actual_fingerprints = (
        ("compiled semantic plan", compiled_semantic_fingerprint),
        ("materialized map", materialized_map_fingerprint),
        ("exported Snapshot V1", exported_snapshot_fingerprint),
    );
    let expected_fingerprints = (
        ("compiled semantic plan", Some(0x2929_4c79_400f_865e)),
        ("materialized map", 0x8fcf_4662_ac5b_93e8),
        ("exported Snapshot V1", 0xc929_2402_a372_f3b2),
    );
    assert_eq!(
        actual_fingerprints, expected_fingerprints,
        "the exact Grand world identities must remain pinned after independent geometry and publication checks; actual={actual_fingerprints:#x?}, expected={expected_fingerprints:#x?}"
    );
}

fn assert_hero_camera_anchor_positions(compiled: &CompiledSchematicMap) {
    use hex_core::{HexCoord, MapAnchorId, TilePos};

    let expected = BTreeMap::from([
        ("grand_v3.coast", (-21, 99, 10)),
        ("grand_v3.archipelago", (-75, 125, 10)),
        ("grand_v3.coastal_bridge", (-5, 91, 10)),
        ("grand_v3.valley_bridge", (80, 10, 15)),
        ("grand_v3.valley_lake", (66, -10, 20)),
        ("grand_v3.natural_pass", (-16, -123, 161)),
        ("grand_v3.massif", (-1, -44, 139)),
        ("grand_v3.peak_saddle", (119, -147, 160)),
        ("grand_v3.mountain_lake", (65, -68, 227)),
        ("grand_v3.frozen_woods", (44, -132, 150)),
        ("grand_v3.tunnel_mouth", (22, 31, 7)),
        ("grand_v3.tunnel_midpoint", (22, -47, 6)),
        ("grand_v3.gothic_transition", (-11, -103, 6)),
        ("grand_v3.ascent_threshold", (-10, -115, 6)),
        ("crystal_ascent.bottom_chamber", (6, -124, 6)),
        ("crystal_ascent.corner_landing", (33, -122, 134)),
        ("crystal_ascent.mid_flight", (44, -154, 74)),
        ("crystal_ascent.upper_contraction", (22, -113, 138)),
        ("crystal_ascent.upper_exit", (53, -148, 150)),
        ("grand_v3.frozen_exit", (56, -151, 152)),
    ]);
    let actual = expected
        .keys()
        .map(|name| {
            let position = compiled
                .anchors
                .get(&MapAnchorId::from(*name))
                .unwrap_or_else(|| panic!("hero map omitted camera destination {name}"));
            (
                *name,
                (position.coord.x(), position.coord.y(), position.level),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        actual, expected,
        "hero camera destinations changed; refresh the walk and visual evidence together"
    );

    assert_eq!(
        compiled.anchors.get(&MapAnchorId::from("party_start")),
        Some(TilePos::new(HexCoord::from_axial(-21, 99), 10))
    );
}

#[test]
fn public_hero_schematic_fine_topology_admits_without_runtime_content() {
    let template = hex_schematic::grand_v3_reference_template().expect("template should parse");
    let settings: MapSettings = ron::de::from_str(include_str!(
        "../../../assets/config/worlds/procedural-grand-v3-baseline.ron"
    ))
    .expect("Grand V3 Baseline settings should parse");
    let generated = hex_schematic::generate(&template, DEFAULT_HERO_SEED)
        .expect("the approved hero schematic should generate");
    let admission = admit_schematic_topology(&generated.plan, &settings)
        .expect("the approved hero fine topology should admit without runtime content");
    assert_topology_admission(&admission, generated.plan.semantic_fingerprint);
}

#[test]
fn public_schematic_fine_topology_admits_256_seeds() {
    let template = hex_schematic::grand_v3_reference_template().expect("template should parse");
    let settings: MapSettings = ron::de::from_str(include_str!(
        "../../../assets/config/worlds/procedural-grand-v3-baseline.ron"
    ))
    .expect("Grand V3 Baseline settings should parse");
    let mut fingerprints = BTreeSet::new();
    let mut natural_pass_widths = BTreeSet::new();

    for seed in 0..256_u64 {
        let generated = hex_schematic::generate(&template, seed)
            .unwrap_or_else(|error| panic!("seed {seed} should generate: {error}"));
        assert_normal_generated_schematic(&template, &generated, seed);
        let admission = admit_schematic_topology(&generated.plan, &settings)
            .unwrap_or_else(|error| panic!("seed {seed} fine topology should admit: {error}"));
        assert_topology_admission(&admission, generated.plan.semantic_fingerprint);
        natural_pass_widths.insert(admission.natural_pass_width);
        assert!(
            fingerprints.insert(generated.plan.semantic_fingerprint),
            "seed {seed} duplicated an earlier semantic fingerprint"
        );
    }

    assert_eq!(fingerprints.len(), 256);
    assert_eq!(
        natural_pass_widths,
        BTreeSet::from([3, 4, 5]),
        "the independent pass-width stream must exercise every admitted width"
    );
}

fn assert_topology_admission(
    admission: &hex_map::GrandV3TopologyAdmission,
    expected_schematic_fingerprint: u64,
) {
    assert_eq!(
        admission.schematic_fingerprint,
        expected_schematic_fingerprint
    );
    assert_eq!(admission.schematic_cells, SCHEMATIC_CELLS_U32);
    assert_eq!(admission.fine_columns, WORLD_COLUMNS_U32);
    assert_eq!(admission.fine_owners, SCHEMATIC_CELLS_U32);
    assert!(admission.hydrology_rows > 1);
    assert!(admission.hydrology_cells >= admission.hydrology_rows);
    assert_eq!(admission.hydrology_outlet_lanes, 3);
    assert_eq!(admission.river_bridges, 2);
    assert!(admission.natural_pass_surfaces > 0);
    assert!((3..=5).contains(&admission.natural_pass_width));
    assert!(admission.tunnel_rows > 1);
    assert!(admission.tunnel_cells >= admission.tunnel_rows);
    assert_eq!(admission.upper_routes, 2);
}

#[test]
fn public_reference_schematic_compiles_a_complete_world() {
    let _serial = full_world_compilation_guard();
    let inputs = compilation_inputs();
    let reference = hex_schematic::reference_plan(&inputs.template, DEFAULT_HERO_SEED)
        .expect("the canonical reference should validate");
    let compiled = compile_schematic(
        &reference.plan,
        &inputs.settings,
        &inputs.table,
        &inputs.art_catalog,
    )
    .expect("the canonical reference should compile");

    assert!(reference.plan.provenance.is_reference_artifact);
    assert_complete_world_contract(
        &compiled,
        DEFAULT_HERO_SEED,
        reference.plan.semantic_fingerprint,
    );
}

#[test]
fn public_zero_seed_schematic_compiles_a_complete_world() {
    assert_generated_complete_world(0);
}

#[test]
fn public_seed_2_schematic_covers_the_concealed_tunnel_approach() {
    // Seed 2 exposed that scheduled origins were chosen from the main
    // centerline before concealed rows entered the exact coverage set. The
    // supplemental planner must cover the complete precomputed interior.
    assert_generated_complete_world(2);
}

#[test]
fn public_seed_14_schematic_keeps_review_anchors_on_the_reachable_walker_component() {
    // Seed 14 places an incidental Ordinary cap closest to the valley-lake
    // coarse center. A review anchor must not promote that disconnected cap to
    // authored walker intent during final access reconciliation.
    assert_generated_complete_world(14);
}

#[test]
fn public_maximum_seed_schematic_compiles_a_complete_world() {
    assert_generated_complete_world(u64::MAX);
}

fn assert_generated_complete_world(seed: u64) {
    let _serial = full_world_compilation_guard();
    let inputs = compilation_inputs();
    let generated = hex_schematic::generate(&inputs.template, seed)
        .unwrap_or_else(|error| panic!("extreme seed {seed} should generate: {error}"));
    assert_normal_generated_schematic(&inputs.template, &generated, seed);
    let compiled = compile_schematic(
        &generated.plan,
        &inputs.settings,
        &inputs.table,
        &inputs.art_catalog,
    )
    .unwrap_or_else(|error| panic!("extreme seed {seed} should compile: {error}"));
    assert_complete_world_contract(&compiled, seed, generated.plan.semantic_fingerprint);
}

#[test]
#[ignore = "release-only: run with `cargo test --release -p hex_map --test schematic_compile grand_v3_full_world_release_corpus_compiles_32_seeds -- --ignored --exact --test-threads=1`"]
fn grand_v3_full_world_release_corpus_compiles_32_seeds() {
    if cfg!(debug_assertions) {
        panic!("the full-world corpus must run with --release");
    }
    let _serial = full_world_compilation_guard();
    let inputs = compilation_inputs();
    let mut schematic_fingerprints = BTreeSet::new();

    for seed in 0..32_u64 {
        let generated = hex_schematic::generate(&inputs.template, seed)
            .unwrap_or_else(|error| panic!("release seed {seed} should generate: {error}"));
        assert_normal_generated_schematic(&inputs.template, &generated, seed);
        assert!(
            schematic_fingerprints.insert(generated.plan.semantic_fingerprint),
            "release seed {seed} duplicated an earlier schematic fingerprint"
        );
        let compiled = compile_schematic(
            &generated.plan,
            &inputs.settings,
            &inputs.table,
            &inputs.art_catalog,
        )
        .unwrap_or_else(|error| panic!("release seed {seed} should compile: {error}"));
        assert_complete_world_contract(&compiled, seed, generated.plan.semantic_fingerprint);
    }

    assert_eq!(schematic_fingerprints.len(), 32);
}

fn assert_normal_generated_schematic(
    template: &hex_schematic::SchematicTemplateV1,
    generated: &hex_schematic::GeneratedSchematic,
    seed: u64,
) {
    assert_eq!(generated.plan.provenance.world_seed, seed);
    assert_eq!(
        generated.plan.provenance.candidates_evaluated,
        hex_schematic::CANDIDATE_ATTEMPTS
    );
    assert!(generated.plan.provenance.hard_valid_candidates > 0);
    assert!(generated.plan.provenance.selected_candidate.is_some());
    assert!(!generated.plan.provenance.used_reference_fallback);
    assert_eq!(generated.plan.cells.len(), SCHEMATIC_CELLS);
    assert_eq!(generated.metrics.cell_count, 217);
    assert_eq!(generated.metrics.internal_adjacencies, 600);
    assert_eq!(
        generated.plan.semantic_fingerprint,
        hex_schematic::semantic_fingerprint(&generated.plan)
    );
    assert_eq!(
        hex_schematic::validate_plan(template, &generated.plan),
        Ok(generated.metrics.clone())
    );
    assert_eq!(
        generated
            .plan
            .networks
            .iter()
            .filter(|network| network.kind == hex_schematic::NetworkKind::Hydrology)
            .count(),
        1
    );
    assert_eq!(
        generated
            .plan
            .networks
            .iter()
            .filter(|network| network.kind == hex_schematic::NetworkKind::Tunnel)
            .count(),
        1
    );
}

fn assert_complete_world_contract(
    compiled: &CompiledSchematicMap,
    expected_seed: u64,
    expected_schematic_fingerprint: u64,
) {
    assert_eq!(compiled.map.len(), WORLD_COLUMNS);
    assert_eq!(compiled.report.generator_version, 3);
    assert_eq!(compiled.report.seed, expected_seed);
    assert!(!compiled.report.used_fallback);
    assert!(compiled.report.notes.is_empty());
    assert!(compiled.report.semantic_plan_fingerprint.is_some());
    assert!(compiled.report.metrics.reachable_surfaces > 0);
    assert!(compiled.report.metrics.reachable_elevation_levels > 1);

    let Some(ProceduralRecipeMetrics::GrandV3(metrics)) = compiled.report.recipe_metrics.as_ref()
    else {
        panic!("compiled schematic omitted its Grand V3 metrics");
    };
    assert_eq!(metrics.schematic_cells, SCHEMATIC_CELLS_U32);
    assert_eq!(metrics.world_columns, WORLD_COLUMNS_U32);
    assert_eq!(metrics.resident_chunks, RESIDENT_CHUNKS);
    assert!(metrics.ordinary_surfaces > 0);
    assert!(
        metrics.water_columns > WORLD_COLUMNS_U32 / 10,
        "the compiled schematic should retain a materially large south-west sea, got {} water columns",
        metrics.water_columns
    );
    assert!(metrics.liquid_bodies > 0);
    assert_eq!(
        metrics.schematic_fingerprint,
        expected_schematic_fingerprint
    );
    let profile = V3GrandV3BasicTerrainProfile::canonical();
    assert!(metrics.minimum_surface >= profile.crystal_base_level);
    assert!(metrics.maximum_surface > profile.sharp_peak_max);
    assert!(
        (330..=350).contains(&metrics.maximum_surface),
        "the connected Massif should remain the world crest below the inclusive V3 level-384 ceiling, got {}",
        metrics.maximum_surface
    );

    let region_counts = compiled.biome_regions.iter().fold(
        BTreeMap::<_, usize>::new(),
        |mut counts, (_, region)| {
            *counts.entry(region).or_default() += 1;
            counts
        },
    );
    assert_eq!(region_counts.len(), SCHEMATIC_CELLS);
    assert!(region_counts.values().all(|count| *count > 0));
    assert!(!compiled.blockers.is_empty());
    assert!(!compiled.interiors.is_empty());
    assert!(!compiled.special_regions.is_empty());

    for anchor in [
        "grand_v3.archipelago",
        "grand_v3.coast",
        "grand_v3.valley_bridge",
        "grand_v3.coastal_bridge",
        "grand_v3.valley_lake",
        "grand_v3.waterfall_profile",
        "grand_v3.mountain_lake",
        "grand_v3.frozen_woods",
        "grand_v3.natural_pass",
        "grand_v3.massif",
        "grand_v3.peak_saddle",
        "grand_v3.tunnel_mouth",
        "grand_v3.ascent_threshold",
        "grand_v3.crystal_summit",
        "grand_v3.frozen_exit",
        "grand_v3.crystal_mantle_overlook",
        "grand_v3.river_bend",
        "grand_v3.treeline_transition",
        "grand_v3.peak_ridge_overlook",
    ] {
        assert!(
            compiled
                .anchors
                .get(&hex_core::MapAnchorId::from(anchor))
                .is_some(),
            "compiled world omitted required anchor {anchor}"
        );
    }
    for anchor in [
        "grand_v3.lake_island",
        "grand_v3.massif_crest",
        "grand_v3.waterfall_base",
        "grand_v3.waterfall_crown",
    ] {
        let id = hex_core::MapAnchorId::from(anchor);
        assert!(
            compiled.anchors.get(&id).is_none(),
            "scenic landmark {anchor} must not be a gameplay placement anchor"
        );
        assert!(
            compiled.observation_anchors.get(&id).is_some(),
            "compiled world omitted required observation anchor {anchor}"
        );
    }
    let bridge_anchors = compiled
        .anchors
        .iter()
        .filter(|(id, _)| {
            matches!(
                id.as_str(),
                "grand_v3.valley_bridge" | "grand_v3.coastal_bridge"
            )
        })
        .map(|(_, position)| position)
        .collect::<BTreeSet<_>>();
    assert_eq!(bridge_anchors.len(), 2);

    let presentation = compiled.presentation_counts();
    assert!(presentation.liquids > 30_000);
    assert!(presentation.features > 0);
    assert!(presentation.structures >= 3);
    assert!(presentation.lights > 0);
}

fn compilation_inputs() -> CompilationInputs {
    let template = hex_schematic::grand_v3_reference_template().expect("template should parse");
    let settings: MapSettings = ron::de::from_str(include_str!(
        "../../../assets/config/worlds/procedural-grand-v3-baseline.ron"
    ))
    .expect("Grand V3 Baseline settings should parse");
    let palette: ArtPalette = ron::de::from_str(include_str!("../../../assets/art/palette.ron"))
        .expect("art palette should parse");
    let substances: SubstanceFile =
        ron::de::from_str(include_str!("../../../assets/config/substances.ron"))
            .expect("substances should parse");
    let table = SubstanceTable::from_file(&substances, &palette)
        .expect("accepted content should resolve substances");
    let art_catalog = runtime_art_catalog(&palette);
    CompilationInputs {
        template,
        settings,
        table,
        art_catalog,
    }
}

fn full_world_compilation_guard() -> MutexGuard<'static, ()> {
    FULL_WORLD_COMPILATION
        .lock()
        // A failed seed must not hide the results for the other complete-world
        // fixtures in the same test process. The guard only serializes the
        // memory-heavy compiler; it does not protect mutable shared state.
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn runtime_art_catalog(palette: &ArtPalette) -> RuntimeArtCatalog {
    let styles: VoxelStyleCatalog =
        ron::from_str(include_str!("../../../assets/art/voxel_styles.ron"))
            .expect("shipped voxel styles should parse");
    let manifest: ObjectCatalogFile =
        ron::from_str(include_str!("../../../assets/art/object_catalog.ron"))
            .expect("shipped object manifest should parse");
    let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/art/objects");
    let mut objects = BTreeMap::new();
    for id in manifest.ids() {
        let path = assets.join(format!("{}.ron", id.as_str()));
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let blueprint: ObjectBlueprint = ron::from_str(&source)
            .unwrap_or_else(|error| panic!("cannot parse {}: {error}", path.display()));
        assert!(objects.insert(blueprint.id.clone(), blueprint).is_none());
    }
    RuntimeArtCatalog::from_sources(palette, &styles, &manifest, objects)
        .expect("shipped runtime art catalog should resolve")
}
