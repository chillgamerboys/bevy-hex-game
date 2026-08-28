//! Grand V3 schematic-to-map compilation.
//!
//! The compiler builds one continuous authored world from the selected schematic.
//! Coarse cells retain stable biome ownership, while terrain, coast detail,
//! hydrology, routes, landmarks, and decoration are resolved globally so cell
//! borders never become visible patch seams.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashSet, VecDeque};

use hex_assets::{HexObjectRotation, RuntimeArtCatalog};
use hex_core::{
    BiomeRegionId, HexCoord, IlluminationLevel, InteriorRegionId, Level, MapViewHint,
    SpecialMovementRegion, TilePos,
};
use hex_schematic::{
    AccessIntent, CellPlan, ClimateKind, FeatureKind as SchematicFeature, GeneratedSchematic,
    LandformKind, NetworkKind, SchematicCoord, SchematicPlanV1, SurfaceKind, VegetationDensity,
};

use super::layout::{resolve_layout, LayoutKind, PatchId, ResolvedLayoutPlan};
use super::liquid::{LiquidBodyId, LiquidBodyPlan, LiquidFlowState, LiquidNode, LiquidPlan};
use super::schematic_ecology::{self, VegetationFamily};
use super::selection::{CandidateNote, ValidatedWorldPlan, ValidatedWorldSelection};
use super::traversal::{ordinary_surface_is_node, ordinary_transition_is_admitted, OrdinaryGraph};
use super::vegetation::{SnowyVegetationSet, TemperateVegetationSet, VegetationObjectSpec};
use super::volume::{
    FillMaterialRole, LevelInterval, NonSolidFill, SolidMass, SolidMaterialRole, SurfaceAccess,
    SurfaceMetadata, VolumeColumn, VolumeElement, VolumePlan,
};
use super::world::{
    CaveCrystalKind, CaveCrystalPresentation, CaveCrystalSiteKind, FeatureId, FeatureKind,
    FeaturePlan, GeneratedWorldPlan, InteriorPlan, LightId, PlannedFeature, PlannedGameplayLight,
    PlannedInterior, PlannedLightPresentation, PlannedStructure, ProtectedFeatureRoute,
    StructureId, StructureKind, StructurePlan, WorldIssueCode, WorldValidationIssue,
};
use super::V3GenerationError;
use crate::settings::{
    ProceduralV3Settings, V3GrandV3BasicTerrainProfile, V3LayoutSettings,
    V3SchematicLayoutSettings, V3SchematicTemplate, V3SchematicTerrainProfile, MAX_V3_LEVEL,
    V3_GRAND_V3_TEMPLATE_REVISION, V3_SCHEMATIC_GRID_RADIUS,
};

#[path = "schematic_corrective.rs"]
mod corrective;

const WORLD_NAMESPACE: u32 = 255 << 24;
const SCENIC_MOVEMENT_REGION: SpecialMovementRegion = SpecialMovementRegion(WORLD_NAMESPACE | 1);
const INACCESSIBLE_MOVEMENT_REGION: SpecialMovementRegion =
    SpecialMovementRegion(WORLD_NAMESPACE | 2);
const HYDROLOGY_BODY_BASE: u32 = WORLD_NAMESPACE | 0x0001_0000;
const BRIDGE_STRUCTURE_BASE: u32 = WORLD_NAMESPACE | 0x0002_0000;
const VEGETATION_FEATURE_BASE: u32 = WORLD_NAMESPACE | 0x0003_0000;
const TUNNEL_LIGHT_BASE: u32 = WORLD_NAMESPACE | 0x0004_0000;
const TUNNEL_DIM_LIGHT_RADIUS: u32 = 18;
const UPPER_REGION_THRESHOLD: Level = 121;
// The final edge enters an already-authored network surface, so 191 search
// steps retain the public 192-edge / 193-cell connector ceiling.
const MAXIMUM_ORDINARY_CONNECTOR_SEARCH_STEPS: u32 = 191;
// Crossing exact authored terrain is a last resort. This must stay larger than
// the complete connector edge budget so every wholly mutable route wins first.
const ORDINARY_CONNECTOR_PRESERVED_COST: u32 = 1_024;
const CRYSTAL_MANTLE_REVIEW_MINIMUM_DISTANCE: u32 = 50;
const CRYSTAL_MANTLE_REVIEW_PREFERRED_DISTANCE: u32 = 56;
const TREELINE_REVIEW_TARGET_LEVEL: Level = 136;
const TREELINE_REVIEW_TREE_SEARCH_RADIUS: u32 = 66;
const TREELINE_REVIEW_UPHILL_SEARCH_RADIUS: u32 = 44;
// Review anchors reserve three horizontal cells and the shipped broadleaf
// visual volumes extend at most two. A root must therefore be at least six
// cells away for its complete canopy to remain outside the anchor reservation.
const REVIEW_ANCHOR_TREE_ROOT_CLEAR_RADIUS: u32 = 5;
const TREELINE_REVIEW_TREE_FREE_RADIUS: u32 = REVIEW_ANCHOR_TREE_ROOT_CLEAR_RADIUS;
const REVIEW_ANCHOR_BLOCKER_CLEAR_RADIUS: u32 = 1;

/// Owned Grand world plus the sealed construction evidence which permits its
/// narrow final-validation fast path.
///
/// Construction can create this value only by consuming the world in
/// [`admit_reconciled_grand_world`]. No mutable plan escapes after the final
/// interior projection has been reconciled.
#[derive(Debug)]
pub(super) struct GrandWorldConstructionAdmission {
    plan: GeneratedWorldPlan,
    _layout: super::schematic_crystal::ClaimedSchematicLayoutAdmission,
}

impl GrandWorldConstructionAdmission {
    pub(super) fn into_plan(self) -> GeneratedWorldPlan {
        self.plan
    }
}

#[derive(Debug, Default)]
struct HydrologyCompilation {
    liquids: LiquidPlan,
    water_coords: BTreeSet<HexCoord>,
    river_centerline: Vec<TilePos>,
    waterfall_centerline: Vec<TilePos>,
    watercourse_rows: Vec<BTreeSet<TilePos>>,
    river_rows: Vec<BTreeSet<TilePos>>,
    outlet: BTreeSet<TilePos>,
}

#[derive(Debug)]
struct BridgeAuthority {
    structure: StructureId,
    river_row_indices: [usize; 2],
    deck: BTreeSet<TilePos>,
    water_deck: BTreeSet<TilePos>,
}

#[derive(Debug, Default)]
struct BridgeCompilation {
    structures: StructurePlan,
    crossings: Vec<BridgeAuthority>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BridgeBankApproach {
    structure: StructureId,
    bank_index: usize,
    lane_index: usize,
    surface: TilePos,
}

#[derive(Debug)]
struct OutletAuthority {
    edges: BTreeMap<TilePos, TilePos>,
    downstream_course: BTreeSet<TilePos>,
}

#[derive(Debug, Default)]
struct TunnelCompilation {
    route: ProtectedFeatureRoute,
    interior: PlannedInterior,
    lights: BTreeMap<LightId, PlannedGameplayLight>,
    anchors: BTreeMap<String, TilePos>,
    overburden: TunnelOverburdenAuthority,
}

/// Exact natural strata that must remain visually unchanged above the tunnel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct TunnelOverburdenAuthority {
    pub(super) columns: BTreeMap<HexCoord, TunnelOverburdenColumnAuthority>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TunnelOverburdenColumnAuthority {
    pub(super) surface: TilePos,
    pub(super) voxels: BTreeMap<Level, TunnelOverburdenVoxelAuthority>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TunnelOverburdenVoxelAuthority {
    pub(super) material: SolidMaterialRole,
    pub(super) cutaway_for: Option<InteriorRegionId>,
}

#[derive(Debug, Default)]
struct NaturalPassCompilation {
    route: ProtectedFeatureRoute,
    anchor: Option<TilePos>,
    width: u32,
}

#[derive(Debug, Default)]
struct PeakSaddleCompilation {
    route: ProtectedFeatureRoute,
    anchor: Option<TilePos>,
}

#[derive(Debug)]
struct OrdinaryNetworkCompilation {
    route: ProtectedFeatureRoute,
    graph: OrdinaryGraph,
    full_graph_rebuilds: u32,
    local_graph_repairs: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OrdinaryReachability {
    reachable_surfaces: u32,
    reachable_elevation_levels: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FineWorldOwner {
    patch: PatchId,
    biome: BiomeRegionId,
}

/// Canonical fine-column ownership shared by every global compiler stage.
///
/// Crystal claims can move columns away from their nearest schematic centre, so
/// later stages must consume the resolved masks rather than recomputing Voronoi
/// ownership. Building this exact projection once avoids repeatedly flattening
/// all 217 masks into separate 105k-entry maps.
#[derive(Debug)]
struct FineWorldIndex {
    by_coord: BTreeMap<HexCoord, FineWorldOwner>,
}

impl FineWorldIndex {
    fn from_layout(layout: &ResolvedLayoutPlan) -> Result<Self, V3GenerationError> {
        let mut by_coord = BTreeMap::new();
        for (patch_id, patch) in &layout.patches {
            for coord in &patch.mask {
                if let Some(previous) = by_coord.insert(
                    *coord,
                    FineWorldOwner {
                        patch: *patch_id,
                        biome: patch.biome_region,
                    },
                ) {
                    return Err(schematic_contract(format!(
                        "fine column {coord:?} is owned by both {:?} and {patch_id:?}",
                        previous.patch
                    )));
                }
            }
        }
        if by_coord.len() != layout.footprint.len()
            || !by_coord
                .keys()
                .copied()
                .eq(layout.footprint.iter().copied())
        {
            return Err(schematic_contract(
                "fine ownership index does not exactly cover the resolved footprint",
            ));
        }
        Ok(Self { by_coord })
    }

    fn patch(&self, coord: HexCoord) -> Option<PatchId> {
        self.by_coord.get(&coord).map(|owner| owner.patch)
    }

    fn biome(&self, coord: HexCoord) -> Option<BiomeRegionId> {
        self.by_coord.get(&coord).map(|owner| owner.biome)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OrdinaryRegionBand {
    Lower,
    Upper,
}

impl OrdinaryRegionBand {
    fn containing(level: Level) -> Self {
        if level < UPPER_REGION_THRESHOLD {
            Self::Lower
        } else {
            Self::Upper
        }
    }

    fn accepts_existing(self, level: Level) -> bool {
        Self::containing(level) == self
    }

    fn accepts_new(self, level: Level) -> bool {
        match self {
            Self::Lower => level <= UPPER_REGION_THRESHOLD.saturating_sub(2),
            Self::Upper => level >= UPPER_REGION_THRESHOLD.saturating_add(1),
        }
    }
}

/// Deterministic measurements from the fully compiled schematic world.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SchematicWorldMetrics {
    pub(crate) schematic_cells: u32,
    pub(crate) world_columns: u32,
    pub(crate) expected_chunks: u32,
    pub(crate) ordinary_surfaces: u32,
    pub(crate) water_columns: u32,
    pub(crate) liquid_bodies: u32,
    pub(crate) reachable_surfaces: u32,
    pub(crate) reachable_elevation_levels: u32,
    pub(crate) ordinary_graph_full_rebuilds: u32,
    pub(crate) ordinary_graph_local_repairs: u32,
    pub(crate) minimum_surface: Level,
    pub(crate) maximum_surface: Level,
    pub(crate) schematic_fingerprint: u64,
}

/// Generates the Grand V3 schematic exactly once, then compiles its selected plan.
pub(crate) fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
    art_catalog: &RuntimeArtCatalog,
) -> Result<ValidatedWorldSelection<SchematicWorldMetrics>, V3GenerationError> {
    let schematic = schematic_settings(settings, grid_radius, level_height)?;
    let template = match schematic.template {
        V3SchematicTemplate::GrandV3 => hex_schematic::grand_v3_reference_template()
            .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?,
    };
    if template.revision != schematic.template_revision {
        return Err(V3GenerationError::RecipeContract(format!(
            "Grand V3 template revision {} disagrees with configured revision {}",
            template.revision, schematic.template_revision
        )));
    }
    let generated = hex_schematic::generate(&template, seed)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    compile_generated_schematic(
        generated,
        settings,
        schematic,
        grid_radius,
        level_height,
        art_catalog,
    )
}

/// Compiles an exact generated or reference plan after replaying its complete
/// schematic validity contract. Runtime generation uses [`generate`] so the
/// 32-candidate schematic selection itself is never repeated.
pub(crate) fn compile_schematic(
    plan: &SchematicPlanV1,
    settings: &ProceduralV3Settings,
    grid_radius: u32,
    level_height: f32,
    art_catalog: &RuntimeArtCatalog,
) -> Result<ValidatedWorldSelection<SchematicWorldMetrics>, V3GenerationError> {
    let schematic = schematic_settings(settings, grid_radius, level_height)?;
    let template = hex_schematic::grand_v3_reference_template()
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    let metrics = hex_schematic::validate_plan(&template, plan)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    compile_generated_schematic(
        GeneratedSchematic {
            plan: plan.clone(),
            metrics,
        },
        settings,
        schematic,
        grid_radius,
        level_height,
        art_catalog,
    )
}

/// Runs the seed-dependent fine-topology portion of Grand V3 compilation.
///
/// This is intentionally narrower than [`compile_schematic`]: it stops before
/// Crystal object construction, vegetation, presentation, whole-world
/// fingerprinting, and materialization. The admitted work is still the runtime
/// compiler's exact claimed ownership, base terrain, coast, directed hydrology,
/// bridge, natural-pass, upper-threshold, and tunnel-splice logic.
pub(crate) fn admit_schematic_topology(
    plan: &SchematicPlanV1,
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
) -> Result<crate::GrandV3TopologyAdmission, V3GenerationError> {
    let schematic = schematic_settings(settings, grid_radius, level_height)?;
    let template = hex_schematic::grand_v3_reference_template()
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    hex_schematic::validate_plan(&template, plan)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    plan.validate_structure()
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    if plan.template_revision != schematic.template_revision
        || plan.template_id.as_str() != "template/grand-v3"
        || plan.semantic_fingerprint != hex_schematic::semantic_fingerprint(plan)
    {
        return Err(schematic_contract(
            "schematic plan identity, revision, or semantic fingerprint is invalid",
        ));
    }

    let V3SchematicTerrainProfile::GrandV3BasicV1(profile) = schematic.terrain_profile;
    let mut layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    let claimed_layout = super::schematic_crystal::claim_site(
        plan,
        &mut layout,
        i32::try_from(schematic.cell_pitch)
            .map_err(|_| schematic_contract("schematic pitch exceeds i32"))?,
    )?;
    let crystal_patch_id = claimed_layout.patch_id();
    let SchematicFoundation {
        fine_index,
        centers: _,
        massif_crest,
        massif_visual: _,
        peak_ridges: _,
        mut volume,
        mut biome_regions,
        minimum_surface: _,
        maximum_surface: _,
    } = build_schematic_foundation(plan, &layout, profile)?;
    let world_seed = plan.provenance.world_seed;
    apply_coast_detail(
        plan,
        world_seed,
        profile,
        &layout,
        &fine_index,
        &mut volume,
        &mut biome_regions,
    )?;
    let hydrology = compile_authoritative_hydrology(
        plan,
        profile,
        world_seed,
        &layout,
        &fine_index,
        &mut volume,
        &mut biome_regions,
    )?;
    let bridges = compile_river_bridges(
        plan,
        &hydrology.river_centerline,
        &hydrology.river_rows,
        &hydrology.water_coords,
        &layout,
        &fine_index,
        &mut volume,
        &mut biome_regions,
    )?;
    let crystal_patch = layout.patches.get(&crystal_patch_id).ok_or_else(|| {
        schematic_contract("claimed Crystal patch disappeared during topology admission")
    })?;
    let crystal_mantle_screen = super::schematic_highlands::crystal_mantle_inner_screen(
        &crystal_patch.mask,
        crystal_patch.rotation_turns,
        profile,
        &layout.footprint,
    )?;
    let mut surface_route_exclusion = crystal_mantle_screen;
    surface_route_exclusion.extend(massif_crest.coords());

    let natural_pass = compile_natural_pass(
        world_seed,
        &layout,
        &fine_index,
        &hydrology.water_coords,
        &surface_route_exclusion,
        &mut volume,
        &mut biome_regions,
    )?;
    let mut route_features = FeaturePlan::default();
    route_features.protected_routes.insert(
        "grand_v3.natural_pass".to_owned(),
        natural_pass.route.clone(),
    );
    let _sealed = seal_unplanned_upper_crossings(&mut volume, &route_features);
    validate_admitted_natural_upper_entry(&volume, &natural_pass.route)?;
    massif_crest.validate_geometry("topology natural-pass construction", &volume)?;

    let lower_terminal = super::crystal_ascent::macro_lower_terminal_coords(
        &crystal_patch.mask,
        crystal_patch.rotation_turns,
        profile.crystal_base_level,
    )
    .map_err(schematic_contract)?
    .into_iter()
    .map(|coord| TilePos::new(coord, profile.crystal_base_level))
    .collect::<BTreeSet<_>>();
    let locked_tunnel = fine_network_path(
        &schematic_network_path(plan, NetworkKind::Tunnel, "edge/tunnel-complete")?,
        i32::try_from(schematic.cell_pitch)
            .map_err(|_| schematic_contract("schematic pitch exceeds i32"))?,
    );
    let tunnel = resolve_exact_terminal_lane(
        &lower_terminal,
        &crystal_patch.mask,
        &layout.footprint,
        locked_tunnel,
    )
    .ok_or_else(|| {
        schematic_contract(
            "exact Crystal terminal cannot splice into one stable outward tunnel lane frame",
        )
    })?;
    if tunnel.rows.first()
        != Some(
            &lower_terminal
                .iter()
                .map(|position| position.coord)
                .collect::<BTreeSet<_>>(),
        )
        || tunnel.rows.len() != tunnel.centerline.len()
        || tunnel.rows.iter().any(|row| row.len() != 4)
        || tunnel
            .rows
            .windows(2)
            .any(|pair| !lane_rows_connect_smoothly(&pair[0], &pair[1]))
        || tunnel
            .rows
            .iter()
            .skip(1)
            .flatten()
            .any(|coord| crystal_patch.mask.contains(coord))
    {
        return Err(schematic_contract(
            "admitted tunnel does not preserve its exact four-wide Crystal splice",
        ));
    }

    let hydrology_cells = hydrology
        .watercourse_rows
        .iter()
        .flatten()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    let tunnel_cells = tunnel
        .rows
        .iter()
        .flatten()
        .copied()
        .collect::<BTreeSet<_>>();
    let fine_owners = layout
        .patches
        .values()
        .filter(|patch| !patch.mask.is_empty())
        .count();
    Ok(crate::GrandV3TopologyAdmission {
        schematic_fingerprint: plan.semantic_fingerprint,
        schematic_cells: u32::try_from(plan.cells.len()).unwrap_or(u32::MAX),
        fine_columns: u32::try_from(fine_index.by_coord.len()).unwrap_or(u32::MAX),
        fine_owners: u32::try_from(fine_owners).unwrap_or(u32::MAX),
        hydrology_rows: u32::try_from(hydrology.watercourse_rows.len()).unwrap_or(u32::MAX),
        hydrology_cells: u32::try_from(hydrology_cells.len()).unwrap_or(u32::MAX),
        hydrology_outlet_lanes: u32::try_from(hydrology.outlet.len()).unwrap_or(u32::MAX),
        river_bridges: u32::try_from(bridges.crossings.len()).unwrap_or(u32::MAX),
        natural_pass_surfaces: u32::try_from(natural_pass.route.surfaces.len()).unwrap_or(u32::MAX),
        natural_pass_width: natural_pass.width,
        tunnel_rows: u32::try_from(tunnel.rows.len()).unwrap_or(u32::MAX),
        tunnel_cells: u32::try_from(tunnel_cells.len()).unwrap_or(u32::MAX),
        upper_routes: 2,
    })
}

fn schematic_settings(
    settings: &ProceduralV3Settings,
    grid_radius: u32,
    level_height: f32,
) -> Result<&V3SchematicLayoutSettings, V3GenerationError> {
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V3GenerationError::RecipeContract(
            "Schematic level height must be positive and finite".to_owned(),
        ));
    }
    let V3LayoutSettings::Schematic(schematic) = &settings.layout else {
        return Err(V3GenerationError::RecipeContract(
            "schematic compiler requires V3LayoutSettings::Schematic".to_owned(),
        ));
    };
    if schematic.template_revision != V3_GRAND_V3_TEMPLATE_REVISION
        || schematic.cell_pitch != 22
        || grid_radius != V3_SCHEMATIC_GRID_RADIUS
    {
        return Err(V3GenerationError::RecipeContract(
            "schematic compiler requires Grand V3 revision 2, pitch 22, and radius 187".to_owned(),
        ));
    }
    Ok(schematic)
}

fn compile_generated_schematic(
    generated: GeneratedSchematic,
    settings: &ProceduralV3Settings,
    schematic: &V3SchematicLayoutSettings,
    grid_radius: u32,
    level_height: f32,
    art_catalog: &RuntimeArtCatalog,
) -> Result<ValidatedWorldSelection<SchematicWorldMetrics>, V3GenerationError> {
    let profile_started = std::time::Instant::now();
    let mut profile_previous = profile_started;
    let GeneratedSchematic { plan, .. } = generated;
    plan.validate_structure()
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    if plan.template_revision != schematic.template_revision
        || plan.template_id.as_str() != "template/grand-v3"
        || plan.semantic_fingerprint != hex_schematic::semantic_fingerprint(&plan)
    {
        return Err(V3GenerationError::RecipeContract(
            "schematic plan identity, revision, or semantic fingerprint is invalid".to_owned(),
        ));
    }
    let V3SchematicTerrainProfile::GrandV3BasicV1(profile) = schematic.terrain_profile;
    let mut layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    grand_profile_checkpoint("layout", profile_started, &mut profile_previous);
    let claimed_layout = super::schematic_crystal::claim_site(
        &plan,
        &mut layout,
        i32::try_from(schematic.cell_pitch)
            .map_err(|_| schematic_contract("schematic pitch exceeds i32"))?,
    )?;
    grand_profile_checkpoint("crystal claim", profile_started, &mut profile_previous);
    let crystal_fragment = super::schematic_crystal::construct_fragment(
        &layout,
        claimed_layout.patch_id(),
        level_height,
        plan.provenance.world_seed,
        art_catalog,
    )?;
    grand_profile_checkpoint(
        "crystal construction",
        profile_started,
        &mut profile_previous,
    );
    let (world, metrics) = build_proxy_world(
        &plan,
        layout,
        claimed_layout,
        profile,
        level_height,
        crystal_fragment,
        art_catalog,
    )?;
    grand_profile_checkpoint("world construction", profile_started, &mut profile_previous);
    let admission = ValidatedWorldPlan::validate_grand_construction(world)?;
    grand_profile_checkpoint(
        "complete world validation",
        profile_started,
        &mut profile_previous,
    );
    let validated = admission.fingerprint()?;
    grand_profile_checkpoint(
        "semantic fingerprint",
        profile_started,
        &mut profile_previous,
    );
    let provenance = plan.provenance;
    Ok(ValidatedWorldSelection {
        validated,
        metrics,
        selected_candidate: provenance.selected_candidate,
        candidates_evaluated: provenance.candidates_evaluated,
        valid_candidates: provenance.hard_valid_candidates,
        repair_rounds: Vec::new(),
        used_fallback: provenance.used_reference_fallback,
        notes: if provenance.used_reference_fallback {
            vec![CandidateNote::FallbackSelected]
        } else {
            Vec::new()
        },
    })
}

fn grand_profile_checkpoint(
    stage: &str,
    started: std::time::Instant,
    previous: &mut std::time::Instant,
) {
    if std::env::var_os("HEX_GRAND_PROFILE").is_some() {
        let now = std::time::Instant::now();
        eprintln!(
            "grand-v3 profile: {stage}: delta={:?} total={:?}",
            now.duration_since(*previous),
            now.duration_since(started)
        );
        *previous = now;
    }
}

/// Exact pre-feature terrain shared by runtime compilation and the lightweight
/// multi-seed admission corpus.
///
/// Keeping this boundary below Crystal construction, route carving, and
/// decoration lets the corpus exercise every fine ownership decision without
/// loading runtime art. The owned volume is nevertheless the same 105,469-column
/// base volume consumed by the full compiler; this is not a coarse proxy.
struct SchematicFoundation {
    fine_index: FineWorldIndex,
    centers: BTreeMap<PatchId, HexCoord>,
    massif_crest: MassifCrestAuthority,
    massif_visual: super::schematic_highlands::MassifVisualAuthority,
    peak_ridges: super::schematic_highlands::PeakRidgeAuthority,
    volume: VolumePlan,
    biome_regions: BTreeMap<TilePos, BiomeRegionId>,
    minimum_surface: Level,
    maximum_surface: Level,
}

const AUTHORED_PEAK_ROUTE_NAMES: [&str; 3] = [
    "grand_v3.natural_pass",
    "grand_v3.peak_saddle",
    "grand_v3.peak_foothill_ledge",
];

/// Seals the exact peak-ridge footprint changed by authored route grading.
///
/// This boundary deliberately runs after the three named peak routes and before
/// the generic ordinary-hub solver.  A coordinate is admitted only when its
/// level already differs from the immutable highland field *and* that exact
/// coordinate is published by one of those routes.  Every surviving seeded
/// high-band coordinate is returned as a hard connector exclusion.
fn seal_peak_ridge_route_grades(
    authority: &mut super::schematic_highlands::PeakRidgeAuthority,
    volume: &VolumePlan,
    features: &FeaturePlan,
) -> Result<BTreeSet<HexCoord>, V3GenerationError> {
    let authored_route_coords = AUTHORED_PEAK_ROUTE_NAMES
        .into_iter()
        .map(|name| {
            features.protected_routes.get(name).ok_or_else(|| {
                schematic_contract(format!(
                    "peak-ridge authority cannot seal before exact route {name} is published"
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flat_map(|route| route.surfaces.iter().map(|surface| surface.coord))
        .collect::<BTreeSet<_>>();

    let mut surviving_high_band = BTreeSet::new();
    for (component_index, component) in authority.components.iter_mut().enumerate() {
        if component.authorized_route_grades.is_some() {
            return Err(schematic_contract(format!(
                "peak-ridge authority component {component_index} was sealed twice"
            )));
        }
        if let Some(pin) = component
            .summit_pins
            .keys()
            .find(|pin| authored_route_coords.contains(pin))
        {
            return Err(schematic_contract(format!(
                "authored peak route entered immutable summit pin {pin:?}"
            )));
        }
        let mut grades = BTreeMap::new();
        for (coord, expected) in &component.expected_high_band {
            let actual = volume
                .top_surface_at_coord(*coord)
                .map(|(surface, _)| surface.level)
                .ok_or_else(|| {
                    schematic_contract(format!(
                        "peak-ridge authority component {component_index} lost expected surface {coord:?} before sealing"
                    ))
                })?;
            if actual == *expected {
                surviving_high_band.insert(*coord);
                continue;
            }
            if component.summit_pins.contains_key(coord) {
                return Err(schematic_contract(format!(
                    "authored peak routes changed immutable summit pin {coord:?} from {expected} to {actual}"
                )));
            }
            if !authored_route_coords.contains(coord) {
                return Err(schematic_contract(format!(
                    "peak-ridge surface {coord:?} changed from {expected} to {actual} outside the exact authored peak routes"
                )));
            }
            grades.insert(*coord, actual);
        }
        component.authorized_route_grades = Some(grades);
    }
    Ok(surviving_high_band)
}

/// Exact natural terrain retained around Grand's unique Massif summit.
///
/// The crest itself is an observation-only surface, while its radius-two
/// shoulder remains normal authored terrain. Route solvers may walk across an
/// already-valid shoulder, but no route carver may replace any of these 19
/// exposed caps. Capturing exact [`TilePos`] values at foundation time makes
/// that promise independently checkable after every later construction stage.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MassifCrestAuthority {
    crest: TilePos,
    shoulder_surfaces: BTreeSet<TilePos>,
}

impl MassifCrestAuthority {
    const RADIUS: u32 = 2;
    const EXPECTED_SURFACES: usize = 19;

    fn from_foundation(
        crest: TilePos,
        volume: &VolumePlan,
        fine_index: &FineWorldIndex,
        cells: &BTreeMap<PatchId, &CellPlan>,
    ) -> Result<Self, V3GenerationError> {
        let shoulder_coords = crest
            .coord
            .within_radius(Self::RADIUS)
            .into_iter()
            .collect::<BTreeSet<_>>();
        if shoulder_coords.len() != Self::EXPECTED_SURFACES {
            return Err(schematic_contract(format!(
                "Massif crest radius-two shoulder has {} columns, expected {}",
                shoulder_coords.len(),
                Self::EXPECTED_SURFACES
            )));
        }
        let mut shoulder_surfaces = BTreeSet::new();
        for coord in shoulder_coords {
            let patch = fine_index.patch(coord).ok_or_else(|| {
                schematic_contract(format!(
                    "Massif crest shoulder leaves the world footprint at {coord:?}"
                ))
            })?;
            if cells
                .get(&patch)
                .is_none_or(|cell| cell.facts.landform != LandformKind::Massif)
            {
                return Err(schematic_contract(format!(
                    "Massif crest shoulder leaves its natural Massif body at {coord:?}"
                )));
            }
            let surface = volume
                .top_surface_at_coord(coord)
                .map(|(surface, _)| surface)
                .ok_or_else(|| {
                    schematic_contract(format!(
                        "Massif crest shoulder has no exact foundation surface at {coord:?}"
                    ))
                })?;
            shoulder_surfaces.insert(surface);
        }
        let authority = Self {
            crest,
            shoulder_surfaces,
        };
        authority.validate_geometry("foundation", volume)?;
        Ok(authority)
    }

    fn coords(&self) -> impl Iterator<Item = HexCoord> + '_ {
        self.shoulder_surfaces.iter().map(|surface| surface.coord)
    }

    fn validate_geometry(&self, stage: &str, volume: &VolumePlan) -> Result<(), V3GenerationError> {
        if self.shoulder_surfaces.len() != Self::EXPECTED_SURFACES
            || !self.shoulder_surfaces.contains(&self.crest)
        {
            return Err(schematic_contract(format!(
                "{stage} lost the exact 19-surface Massif crest authority"
            )));
        }
        if let Some(expected) = self.shoulder_surfaces.iter().find(|expected| {
            volume
                .top_surface_at_coord(expected.coord)
                .map(|(actual, _)| actual)
                != Some(**expected)
        }) {
            return Err(schematic_contract(format!(
                "{stage} moved or replaced natural Massif shoulder surface {expected:?}"
            )));
        }
        Ok(())
    }

    fn validate_route_disjointness(
        &self,
        stage: &str,
        route_coords: &BTreeSet<HexCoord>,
    ) -> Result<(), V3GenerationError> {
        let shoulder_coords = self.coords().collect::<BTreeSet<_>>();
        if let Some(coord) = shoulder_coords.intersection(route_coords).next() {
            return Err(schematic_contract(format!(
                "{stage} route intersects exact Massif crest authority at {coord:?}"
            )));
        }
        Ok(())
    }
}

fn build_schematic_foundation(
    plan: &SchematicPlanV1,
    layout: &ResolvedLayoutPlan,
    profile: V3GrandV3BasicTerrainProfile,
) -> Result<SchematicFoundation, V3GenerationError> {
    if layout.kind != LayoutKind::Schematic
        || layout.patches.len() != hex_schematic::SCHEMATIC_CELL_COUNT
        || layout.footprint.len() != 105_469
    {
        return Err(V3GenerationError::RecipeContract(
            "resolved schematic ownership is not the exact 217-cell radius-187 contract".to_owned(),
        ));
    }
    let fine_index = FineWorldIndex::from_layout(layout)?;

    // Fine compiler streams belong to the requested world seed. The schematic
    // fingerprint intentionally covers every semantic layer (including
    // vegetation), so using it as random input would let a woodland-only change
    // move unrelated rock and lake beds.
    let world_seed = plan.provenance.world_seed;
    let cells = plan
        .cells
        .iter()
        .map(|cell| (PatchId(u32::from(cell.id.get())), cell))
        .collect::<BTreeMap<_, _>>();
    let centers = plan
        .cells
        .iter()
        .map(|cell| {
            (
                PatchId(u32::from(cell.id.get())),
                schematic_to_world(cell.coord, 22),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let high_core = plan
        .cells
        .iter()
        .filter(|cell| {
            cell.facts.overlays.iter().any(|overlay| {
                matches!(
                    overlay,
                    SchematicFeature::MountainLake
                        | SchematicFeature::LakeIsland
                        | SchematicFeature::FrozenWoods
                        | SchematicFeature::PeakRing
                        | SchematicFeature::CrystalAscent
                )
            })
        })
        .map(|cell| cell.coord)
        .collect::<Vec<_>>();
    let coarse_datums = plan
        .cells
        .iter()
        .map(|cell| {
            (
                PatchId(u32::from(cell.id.get())),
                coarse_datum(cell, &high_core, profile),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let coarse_relief_caps = plan
        .cells
        .iter()
        .map(|cell| {
            (
                PatchId(u32::from(cell.id.get())),
                relief_cap(cell.facts.landform),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let highlands = super::schematic_highlands::GrandHighlandField::build(plan, layout, profile)?;
    let (massif_crest_coord, massif_crest_level) = highlands.massif_crest();
    let massif_crest = TilePos::new(massif_crest_coord, massif_crest_level);
    let massif_visual = highlands.massif_visual_authority().clone();
    let peak_ridges = highlands.peak_ridge_authority().clone();

    let mut volume = VolumePlan::new(layout.footprint.clone());
    let mut biome_regions = BTreeMap::new();
    let mut minimum_surface = Level::MAX;
    let mut maximum_surface = Level::MIN;

    for (patch_id, patch) in &layout.patches {
        let cell = cells.get(patch_id).copied().ok_or_else(|| {
            V3GenerationError::RecipeContract(format!(
                "schematic patch {} has no canonical cell",
                patch_id.0
            ))
        })?;
        for coord in &patch.mask {
            let (column, surface, access) = if cell.facts.surface == SurfaceKind::OpenWater {
                let water = water_level(cell, profile);
                let bed = water_bed_level(cell, *coord, water, world_seed);
                (
                    water_column(bed, water, water_bed_material(cell)),
                    TilePos::new(*coord, bed),
                    SurfaceAccess::NonStandable,
                )
            } else {
                let surface_level = fine_surface_level(
                    cell,
                    *coord,
                    &centers,
                    &coarse_datums,
                    &coarse_relief_caps,
                    &highlands,
                    profile,
                    world_seed,
                );
                let access = if *coord == massif_crest.coord && surface_level == massif_crest.level
                {
                    SurfaceAccess::SpecialMovement(INACCESSIBLE_MOVEMENT_REGION)
                } else {
                    match cell.facts.access {
                        AccessIntent::Ordinary => SurfaceAccess::Ordinary,
                        AccessIntent::Scenic => {
                            SurfaceAccess::SpecialMovement(SCENIC_MOVEMENT_REGION)
                        }
                        AccessIntent::Inaccessible => {
                            SurfaceAccess::SpecialMovement(INACCESSIBLE_MOVEMENT_REGION)
                        }
                    }
                };
                let surface = TilePos::new(*coord, surface_level);
                let cap = schematic_ecology::cap_material_override(cell, surface, world_seed)
                    .unwrap_or_else(|| land_cap_material(cell));
                (land_column(surface_level, cap), surface, access)
            };
            minimum_surface = minimum_surface.min(surface.level);
            maximum_surface = maximum_surface.max(surface.level);
            volume.columns.insert(*coord, column);
            volume.surfaces.insert(
                surface,
                SurfaceMetadata {
                    access,
                    interior: None,
                },
            );
            biome_regions.insert(surface, patch.biome_region);
        }
    }

    let massif_crest =
        MassifCrestAuthority::from_foundation(massif_crest, &volume, &fine_index, &cells)?;

    Ok(SchematicFoundation {
        fine_index,
        centers,
        massif_crest,
        massif_visual,
        peak_ridges,
        volume,
        biome_regions,
        minimum_surface,
        maximum_surface,
    })
}

fn build_proxy_world(
    plan: &SchematicPlanV1,
    layout: ResolvedLayoutPlan,
    claimed_layout: super::schematic_crystal::ClaimedSchematicLayoutAdmission,
    profile: V3GrandV3BasicTerrainProfile,
    level_height: f32,
    crystal_fragment: super::composition::GeneratedPatchPlan,
    art_catalog: &RuntimeArtCatalog,
) -> Result<(GrandWorldConstructionAdmission, SchematicWorldMetrics), V3GenerationError> {
    let profile_started = std::time::Instant::now();
    let mut profile_previous = profile_started;
    let world_seed = plan.provenance.world_seed;
    let SchematicFoundation {
        fine_index,
        centers,
        massif_crest,
        massif_visual,
        mut peak_ridges,
        mut volume,
        mut biome_regions,
        mut minimum_surface,
        mut maximum_surface,
    } = build_schematic_foundation(plan, &layout, profile)?;
    grand_profile_checkpoint("base volume", profile_started, &mut profile_previous);

    apply_coast_detail(
        plan,
        world_seed,
        profile,
        &layout,
        &fine_index,
        &mut volume,
        &mut biome_regions,
    )?;
    grand_profile_checkpoint("coast", profile_started, &mut profile_previous);

    let hydrology = compile_authoritative_hydrology(
        plan,
        profile,
        world_seed,
        &layout,
        &fine_index,
        &mut volume,
        &mut biome_regions,
    )?;
    grand_profile_checkpoint("hydrology", profile_started, &mut profile_previous);
    let water_columns = u32::try_from(hydrology.water_coords.len()).unwrap_or(u32::MAX);
    let bridges = compile_river_bridges(
        plan,
        &hydrology.river_centerline,
        &hydrology.river_rows,
        &hydrology.water_coords,
        &layout,
        &fine_index,
        &mut volume,
        &mut biome_regions,
    )?;
    grand_profile_checkpoint("bridges", profile_started, &mut profile_previous);
    let view_hint = schematic_view_hint(&layout.footprint, level_height, maximum_surface);
    let mut world = GeneratedWorldPlan {
        source_schematic_fingerprint: Some(plan.semantic_fingerprint),
        layout,
        volume,
        liquids: hydrology.liquids.clone(),
        features: FeaturePlan::default(),
        structures: bridges.structures.clone(),
        blockers: BTreeSet::new(),
        lights: BTreeMap::new(),
        biome_regions,
        interiors: InteriorPlan::default(),
        anchors: BTreeMap::new(),
        observation_anchors: BTreeMap::from([(
            "grand_v3.massif_crest".to_owned(),
            massif_crest.crest,
        )]),
        view_hint,
    };
    schematic_ecology::author_lake_island_garden(plan, profile, &mut world)?;
    grand_profile_checkpoint("lake-island garden", profile_started, &mut profile_previous);
    super::schematic_crystal::merge_fragment(&mut world, crystal_fragment)?;
    grand_profile_checkpoint("crystal merge", profile_started, &mut profile_previous);

    let ascent_target = world
        .anchors
        .get("crystal_ascent.lower_entry")
        .copied()
        .ok_or_else(|| schematic_contract("Crystal merge omitted its canonical lower entry"))?;
    let summit_target = world
        .anchors
        .get("crystal_ascent.upper_exit")
        .copied()
        .ok_or_else(|| schematic_contract("Crystal merge omitted its canonical upper exit"))?;
    let lower_terminal = world
        .features
        .protected_routes
        .get("crystal_ascent.lower_terminal_pad")
        .map(|route| route.surfaces.clone())
        .ok_or_else(|| schematic_contract("Crystal merge omitted its exact lower terminal"))?;
    let upper_terminal = world
        .features
        .protected_routes
        .get("crystal_ascent.upper_terminal_pad")
        .map(|route| route.surfaces.clone())
        .ok_or_else(|| schematic_contract("Crystal merge omitted its exact upper terminal"))?;
    if lower_terminal.len() != 4
        || upper_terminal.len() != 4
        || !lower_terminal.contains(&ascent_target)
        || !upper_terminal.contains(&summit_target)
    {
        return Err(schematic_contract(
            "Crystal canonical entry and exit do not belong to exact four-wide terminals",
        ));
    }
    // Standalone Crystal Ascent intentionally publishes `lower_entry` on its
    // exterior apron. Resolve the authored Dark domain from the four interior
    // threshold cells immediately inside that terminal instead of pretending
    // the public exterior anchor already owns an interior.
    let matching_interiors = world
        .interiors
        .by_id
        .iter()
        .filter_map(|(id, interior)| {
            lower_terminal
                .iter()
                .any(|terminal| {
                    terminal
                        .coord
                        .neighbors()
                        .into_iter()
                        .map(|coord| TilePos::new(coord, terminal.level))
                        .any(|neighbor| interior.floors.contains(&neighbor))
                })
                .then_some(*id)
        })
        .collect::<Vec<_>>();
    let [crystal_interior] = matching_interiors.as_slice() else {
        return Err(schematic_contract(
            "Crystal lower terminal must border exactly one authored interior",
        ));
    };
    let crystal_interior = *crystal_interior;
    let authored_lower_threshold = world
        .interiors
        .by_id
        .get(&crystal_interior)
        .map(|interior| interior.entrances.clone())
        .ok_or_else(|| schematic_contract("Crystal authored interior disappeared after merge"))?;
    if authored_lower_threshold.len() != 4
        || authored_lower_threshold.iter().any(|position| {
            world
                .volume
                .surfaces
                .get(position)
                .is_none_or(|metadata| metadata.interior != Some(crystal_interior))
        })
    {
        return Err(schematic_contract(
            "Crystal authored lower threshold is not exact four-wide interior footing",
        ));
    }
    let crystal_mask = world
        .layout
        .patches
        .values()
        .find(|patch| patch.mask.contains(&ascent_target.coord))
        .map(|patch| patch.mask.clone())
        .ok_or_else(|| schematic_contract("Crystal lower entry has no claimed mask"))?;
    let tunnel = compile_tunnel(
        plan,
        profile,
        world_seed,
        crystal_interior,
        ascent_target,
        &lower_terminal,
        &crystal_mask,
        &world.layout,
        &fine_index,
        &mut world.volume,
        &mut world.biome_regions,
    )?;
    let tunnel_overburden = tunnel.overburden.clone();
    grand_profile_checkpoint("tunnel", profile_started, &mut profile_previous);
    let connected_lower_terminal = tunnel
        .interior
        .floors
        .iter()
        .filter(|surface| crystal_mask.contains(&surface.coord))
        .copied()
        .collect::<BTreeSet<_>>();
    if connected_lower_terminal != lower_terminal {
        return Err(schematic_contract(
            "tunnel does not meet the exact four-wide Crystal lower terminal",
        ));
    }
    // In the composite, the lower aperture is no longer an exterior boundary:
    // both exact terminal pads join the unified tunnel/Crystal Dark domain. The
    // fragment's standalone columns and anchors remain byte-for-byte unchanged;
    // only composite surface metadata is reclassified here.
    for position in lower_terminal.iter().chain(&upper_terminal) {
        let metadata = world.volume.surfaces.get_mut(position).ok_or_else(|| {
            schematic_contract(format!(
                "Crystal composite terminal lost exact surface {position:?}"
            ))
        })?;
        metadata.access = SurfaceAccess::Ordinary;
        metadata.interior = Some(crystal_interior);
    }
    world
        .features
        .protected_routes
        .insert("grand_v3.tunnel".to_owned(), tunnel.route);
    let interior = world
        .interiors
        .by_id
        .get_mut(&crystal_interior)
        .ok_or_else(|| schematic_contract("Crystal interior disappeared before tunnel merge"))?;
    interior.floors.extend(tunnel.interior.floors);
    interior.floors.extend(upper_terminal.iter().copied());
    interior.roof_voxels.extend(tunnel.interior.roof_voxels);
    // The authored lower threshold is now internal. Only the four-wide tunnel
    // foot and four-wide summit terminal remain exterior entrances.
    interior.entrances = tunnel
        .interior
        .entrances
        .union(&upper_terminal)
        .copied()
        .collect();
    if interior.entrances.len() != 8
        || !interior.entrances.is_subset(&interior.floors)
        || !authored_lower_threshold.is_disjoint(&interior.entrances)
    {
        return Err(schematic_contract(
            "unified Crystal interior must expose only four foot and four summit entrances",
        ));
    }
    for (id, light) in tunnel.lights {
        if world.lights.insert(id, light).is_some() {
            return Err(schematic_contract(format!(
                "tunnel light {id:?} collided with exact Crystal lighting"
            )));
        }
    }
    world.anchors.extend(tunnel.anchors);
    let tunnel_route = world
        .features
        .protected_routes
        .get("grand_v3.tunnel")
        .cloned()
        .ok_or_else(|| schematic_contract("Crystal/tunnel route vanished after merge"))?;
    let mut crystal_route = compile_exact_crystal_route(
        &world.volume,
        &world.blockers,
        crystal_interior,
        &crystal_mask,
        summit_target,
        &tunnel_route,
    )?;
    let crystal_rotation = world
        .layout
        .patches
        .values()
        .find(|patch| patch.mask == crystal_mask)
        .map(|patch| patch.rotation_turns)
        .ok_or_else(|| schematic_contract("Crystal claimed patch vanished before summit join"))?;
    let frozen_exit = compile_frozen_summit_connection(
        plan,
        profile,
        crystal_rotation,
        summit_target,
        &upper_terminal,
        &crystal_mask,
        &fine_index,
        &mut world.volume,
        &mut world.biome_regions,
    )?;
    let frozen_exit_anchor = *frozen_exit
        .centerline
        .last()
        .ok_or_else(|| schematic_contract("Frozen Woods exit omitted its terminal surface"))?;
    if crystal_route.centerline.last() != frozen_exit.centerline.first() {
        return Err(schematic_contract(
            "canonical Crystal route does not meet its Frozen Woods exit",
        ));
    }
    crystal_route
        .centerline
        .extend(frozen_exit.centerline.iter().copied().skip(1));
    crystal_route
        .surfaces
        .extend(frozen_exit.surfaces.iter().copied());
    world
        .features
        .protected_routes
        .insert("grand_v3.crystal_route".to_owned(), crystal_route);
    world
        .features
        .protected_routes
        .insert("grand_v3.frozen_exit".to_owned(), frozen_exit);
    world
        .anchors
        .insert("grand_v3.frozen_exit".to_owned(), frozen_exit_anchor);

    let crystal_mantle_screen = super::schematic_highlands::crystal_mantle_inner_screen(
        &crystal_mask,
        crystal_rotation,
        profile,
        &world.layout.footprint,
    )?;
    let mut surface_route_exclusion = crystal_mantle_screen.clone();
    surface_route_exclusion.extend(massif_crest.coords());
    // All twelve seeded summit pins are exact terrain identity, not merely a
    // final-height postcondition. Reserve them before any of the three authored
    // peak-route searches so their grading corridors cannot consume a pin and
    // then ask the later authority seal to bless the mutation.
    surface_route_exclusion.extend(
        peak_ridges
            .components
            .iter()
            .flat_map(|component| component.summit_pins.keys().copied()),
    );
    validate_crystal_mantle_screen_caps(
        "Crystal, tunnel, and Frozen-exit construction",
        &world.volume,
        &crystal_mantle_screen,
    )?;
    // Exact route coordinates already carry their own immutable authority.  Add
    // only the remainder of the screen as a new connector reservation so the
    // ordinary network cannot grade a second, accidental aperture through the
    // r33/r34 mantle.
    let published_route_coords = [
        "grand_v3.tunnel",
        "grand_v3.crystal_route",
        "grand_v3.frozen_exit",
    ]
    .into_iter()
    .map(|name| {
        world.features.protected_routes.get(name).ok_or_else(|| {
            schematic_contract(format!(
                "Crystal mantle reservation is missing exact route {name}"
            ))
        })
    })
    .collect::<Result<Vec<_>, _>>()?
    .into_iter()
    .flat_map(|route| route.surfaces.iter().map(|surface| surface.coord))
    .collect::<BTreeSet<_>>();
    massif_crest.validate_route_disjointness(
        "Crystal, tunnel, and Frozen-exit construction",
        &published_route_coords,
    )?;
    let mut connector_route_exclusion = crystal_mantle_screen
        .difference(&published_route_coords)
        .copied()
        .collect::<BTreeSet<_>>();
    connector_route_exclusion.extend(massif_crest.coords());

    let natural_pass = compile_natural_pass(
        world_seed,
        &world.layout,
        &fine_index,
        &hydrology.water_coords,
        &surface_route_exclusion,
        &mut world.volume,
        &mut world.biome_regions,
    )?;
    let expected_natural_pass_width = natural_pass.width;
    grand_profile_checkpoint("natural pass", profile_started, &mut profile_previous);
    if let Some(anchor) = natural_pass.anchor {
        world
            .anchors
            .insert("grand_v3.natural_pass".to_owned(), anchor);
    }
    let peak_saddle = compile_peak_saddle(
        plan,
        &world.layout,
        &fine_index,
        &hydrology.water_coords,
        &natural_pass.route,
        &surface_route_exclusion,
        profile.sharp_peak_bench_max,
        &mut world.volume,
        &mut world.biome_regions,
    )?;
    grand_profile_checkpoint("peak saddle", profile_started, &mut profile_previous);
    let peak_foothill_ledge = compile_peak_foothill_ledge(
        plan,
        &world.layout,
        &fine_index,
        &hydrology.water_coords,
        &natural_pass.route,
        &peak_saddle.route,
        &world.features,
        &surface_route_exclusion,
        &mut world.volume,
        &mut world.biome_regions,
    )?;
    grand_profile_checkpoint(
        "peak foothill ledge",
        profile_started,
        &mut profile_previous,
    );
    if let Some(anchor) = peak_saddle.anchor {
        world
            .anchors
            .insert("grand_v3.peak_saddle".to_owned(), anchor);
    }
    if let Some(anchor) = peak_foothill_ledge.anchor {
        world
            .anchors
            .insert("grand_v3.peak_foothill_ledge".to_owned(), anchor);
    }
    world
        .features
        .protected_routes
        .insert("grand_v3.natural_pass".to_owned(), natural_pass.route);
    world
        .features
        .protected_routes
        .insert("grand_v3.peak_saddle".to_owned(), peak_saddle.route);
    world.features.protected_routes.insert(
        "grand_v3.peak_foothill_ledge".to_owned(),
        peak_foothill_ledge.route,
    );
    let surviving_peak_high_band =
        seal_peak_ridge_route_grades(&mut peak_ridges, &world.volume, &world.features)?;
    connector_route_exclusion.extend(surviving_peak_high_band);
    validate_crystal_mantle_screen_caps(
        "natural-pass and peak-route construction",
        &world.volume,
        &crystal_mantle_screen,
    )?;
    massif_crest.validate_geometry("natural-pass and peak-route construction", &world.volume)?;
    // Remove accidental high/low contacts before the constructive network takes
    // its seed components. The same fail-closed seal is replayed after carving.
    let _sealed = seal_unplanned_upper_crossings(&mut world.volume, &world.features);
    validate_exact_upper_entrances(&world.volume, &world.features)?;
    grand_profile_checkpoint(
        "pre-network upper sealing",
        profile_started,
        &mut profile_previous,
    );
    let OrdinaryNetworkCompilation {
        route: ordinary_route,
        graph: mut ordinary_graph,
        full_graph_rebuilds: ordinary_graph_full_rebuilds,
        local_graph_repairs: ordinary_graph_local_repairs,
    } = compile_ordinary_hub_network(
        plan,
        &fine_index,
        &hydrology.water_coords,
        &bridges,
        &crystal_mask,
        &connector_route_exclusion,
        &mut world,
    )?;
    corrective::validate_peak_ridge_authority(&world, &peak_ridges)?;
    validate_crystal_mantle_screen_caps(
        "ordinary hub-network construction",
        &world.volume,
        &crystal_mantle_screen,
    )?;
    massif_crest.validate_geometry("ordinary hub-network construction", &world.volume)?;
    grand_profile_checkpoint(
        "ordinary construction",
        profile_started,
        &mut profile_previous,
    );
    world
        .features
        .protected_routes
        .insert("grand_v3.ordinary_hubs".to_owned(), ordinary_route);
    let sealed_coords = seal_unplanned_upper_crossings(&mut world.volume, &world.features);
    if !sealed_coords.is_empty() {
        let _affected =
            ordinary_graph.refresh_coords(&world.volume, Some(&world.blockers), sealed_coords);
    }
    grand_profile_checkpoint("upper sealing", profile_started, &mut profile_previous);
    validate_exact_upper_entrances(&world.volume, &world.features)?;
    grand_profile_checkpoint("upper validation", profile_started, &mut profile_previous);

    // Review anchors are gameplay positions, so select them only from the
    // foothill-reachable walker component that construction just proved. An
    // incidental Ordinary cap can still be awaiting final demotion here; if a
    // review anchor claims that cap, authored-surface reconciliation must
    // preserve it and the final admission correctly fails. Seed 14 exposed
    // that case at the valley lake. Reserving the connected choice before
    // vegetation keeps the anchor tree-free without turning a disconnected
    // scenic cap into authored walker intent.
    let review_root = world
        .anchors
        .get("grand_v3.tunnel_mouth")
        .copied()
        .ok_or_else(|| schematic_contract("review anchors have no foothill root"))?;
    let ordinary = ordinary_graph
        .distances_from(review_root)
        .into_keys()
        .collect::<BTreeSet<_>>();
    let review = review_anchors(plan, &ordinary)?;
    for (name, position) in review {
        world.anchors.entry(name).or_insert(position);
    }
    add_final_review_anchors(
        &mut world.anchors,
        &ordinary,
        &hydrology,
        &bridges.structures,
        plan,
        &centers,
    );
    grand_profile_checkpoint("review anchors", profile_started, &mut profile_previous);
    schematic_ecology::reconcile_alpine_caps(plan, world_seed, &mut world)?;
    grand_profile_checkpoint("alpine ecology", profile_started, &mut profile_previous);
    let blockers_before_vegetation = world.blockers.clone();
    compile_schematic_vegetation(plan, world_seed, art_catalog, &crystal_mask, &mut world)?;
    grand_profile_checkpoint("vegetation", profile_started, &mut profile_previous);
    let vegetation_blocker_coords = world
        .blockers
        .difference(&blockers_before_vegetation)
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    if !vegetation_blocker_coords.is_empty() {
        let _affected = ordinary_graph.refresh_coords(
            &world.volume,
            Some(&world.blockers),
            vegetation_blocker_coords,
        );
    }
    // Vegetation can add exact blockers after the constructive hub pass. Audit
    // the two authored upper cuts and measure hub reachability once against the
    // final blocker authority. The compiler retains its admitted graph and
    // reprojects only newly blocked tree columns instead of rebuilding all
    // 105,469 columns after decoration.
    let final_natural_pass = world
        .features
        .protected_routes
        .get("grand_v3.natural_pass")
        .ok_or_else(|| schematic_contract("final width audit has no natural pass"))?;
    let final_natural_pass_width = validate_natural_pass_physical_width(
        world_seed,
        final_natural_pass,
        &world.volume,
        Some(&world.blockers),
    )?;
    if final_natural_pass_width != expected_natural_pass_width {
        return Err(schematic_contract(format!(
            "final natural-pass width {final_natural_pass_width} disagrees with constructed width {expected_natural_pass_width}"
        )));
    }
    let final_reachable = validate_exact_upper_route_cuts(&world, &ordinary_graph)?;
    let durable_review_surfaces = world
        .features
        .protected_routes
        .get("grand_v3.ordinary_hubs")
        .ok_or_else(|| schematic_contract("final review anchors have no durable hub network"))?
        .surfaces
        .iter()
        .copied()
        .filter(|surface| final_reachable.contains_key(surface))
        .collect::<BTreeSet<_>>();
    reconcile_final_review_anchor_reachability(
        &mut world.anchors,
        &final_reachable,
        &durable_review_surfaces,
    )?;
    let corrective_review = resolve_corrective_review_anchors(
        plan,
        &fine_index,
        profile,
        &crystal_mask,
        crystal_rotation,
        &final_reachable,
        &mut world,
    )?;
    let final_authored_surfaces = authored_ordinary_surface_authority(&world, []);
    let _demoted_incidental_surfaces = reconcile_final_incidental_ordinary_access(
        &mut world.volume,
        &world.blockers,
        &mut ordinary_graph,
        &final_reachable,
        &final_authored_surfaces,
    )?;
    let final_reachability = measure_ordinary_hub_network_with_reachability(
        plan,
        &world,
        &ordinary_graph,
        &final_reachable,
    )?;
    grand_profile_checkpoint(
        "final ordinary validation",
        profile_started,
        &mut profile_previous,
    );
    validate_grand_hydrology(&world, plan, profile, &hydrology, &bridges)?;
    grand_profile_checkpoint(
        "final hydrology validation",
        profile_started,
        &mut profile_previous,
    );
    for (name, route) in &world.features.protected_routes {
        // The hub projection names one representative surface for every
        // ordinary coarse cell. It may legitimately reuse an unchanged natural
        // shoulder as an endpoint; the exact geometry check below proves that
        // this did not grade or replace the surface. Every actual authored
        // corridor remains horizontally disjoint from the summit authority.
        if name == "grand_v3.ordinary_hubs" {
            continue;
        }
        massif_crest.validate_route_disjointness(
            &format!("final corrective route {name}"),
            &route.surfaces.iter().map(|surface| surface.coord).collect(),
        )?;
    }
    massif_crest.validate_geometry("final corrective construction", &world.volume)?;
    add_exact_corrective_observation_anchors(&mut world, plan, &centers, massif_crest.crest)?;
    corrective::validate_corrective_world_contract(
        plan,
        &world,
        profile,
        corrective::CorrectiveWorldValidation {
            hydrology: &hydrology,
            crystal_mask: &crystal_mask,
            crystal_rotation,
            fine_index: &fine_index,
            reachable: &final_reachable,
            massif_visual: &massif_visual,
            peak_ridges: &peak_ridges,
            tunnel_overburden: &tunnel_overburden,
            review: &corrective_review,
        },
    )?;
    grand_profile_checkpoint(
        "corrective world validation",
        profile_started,
        &mut profile_previous,
    );

    minimum_surface = world
        .volume
        .surfaces
        .keys()
        .map(|position| position.level)
        .min()
        .unwrap_or(minimum_surface);
    maximum_surface = world
        .volume
        .surfaces
        .keys()
        .map(|position| position.level)
        .max()
        .unwrap_or(maximum_surface);
    let ordinary_surfaces = u32::try_from(
        world
            .volume
            .surfaces
            .values()
            .filter(|metadata| metadata.access == SurfaceAccess::Ordinary)
            .count(),
    )
    .unwrap_or(u32::MAX);
    world.view_hint = schematic_view_hint(&world.layout.footprint, level_height, maximum_surface);
    let metrics = SchematicWorldMetrics {
        schematic_cells: u32::try_from(plan.cells.len()).unwrap_or(u32::MAX),
        world_columns: u32::try_from(world.layout.footprint.len()).unwrap_or(u32::MAX),
        expected_chunks: 444,
        ordinary_surfaces,
        water_columns,
        liquid_bodies: u32::try_from(world.liquids.bodies.len()).unwrap_or(u32::MAX),
        reachable_surfaces: final_reachability.reachable_surfaces,
        reachable_elevation_levels: final_reachability.reachable_elevation_levels,
        ordinary_graph_full_rebuilds,
        ordinary_graph_local_repairs,
        minimum_surface,
        maximum_surface,
        schematic_fingerprint: plan.semantic_fingerprint,
    };
    let admitted = admit_reconciled_grand_world(world, crystal_interior, claimed_layout)?;
    grand_profile_checkpoint(
        "interior reconciliation",
        profile_started,
        &mut profile_previous,
    );
    Ok((admitted, metrics))
}

fn schematic_to_world(coord: SchematicCoord, pitch: i32) -> HexCoord {
    HexCoord::from_axial(
        coord.q().saturating_mul(pitch),
        coord.r().saturating_mul(pitch),
    )
}

fn has_overlay(cell: &CellPlan, overlay: SchematicFeature) -> bool {
    cell.facts.overlays.contains(&overlay)
}

fn is_ordinary_land(cell: &CellPlan) -> bool {
    cell.facts.surface == SurfaceKind::Land && cell.facts.access == AccessIntent::Ordinary
}

fn coarse_datum(
    cell: &CellPlan,
    high_core: &[SchematicCoord],
    profile: V3GrandV3BasicTerrainProfile,
) -> Level {
    if cell.facts.surface == SurfaceKind::OpenWater {
        return water_level(cell, profile);
    }
    if has_overlay(cell, SchematicFeature::FrozenWoods) {
        return profile.frozen_woods_level;
    }
    if has_overlay(cell, SchematicFeature::LakeIsland) {
        return profile.lake_island_min_level;
    }
    if has_overlay(cell, SchematicFeature::CrystalAscent) {
        return profile
            .crystal_base_level
            .saturating_add(profile.crystal_rise_levels);
    }
    if cell.facts.landform == LandformKind::SharpPeak {
        return profile.sharp_peak_bench_min;
    }
    let high_distance = high_core
        .iter()
        .filter_map(|core| cell.coord.checked_distance(*core))
        .min()
        .unwrap_or(u32::MAX);
    let high_influence = profile.high_core_level.saturating_sub(
        i32::try_from(high_distance)
            .unwrap_or(i32::MAX)
            .saturating_mul(profile.high_gradient_per_cell),
    );
    match cell.facts.landform {
        LandformKind::None => profile.sea_level,
        LandformKind::Island => profile.island_level,
        LandformKind::Beach => profile.beach_level,
        LandformKind::Shore => profile.shore_level,
        LandformKind::Valley => profile.valley_level,
        LandformKind::Plateau => profile.plateau_level,
        LandformKind::Hill => profile.hill_level,
        LandformKind::Mountain => profile.mountain_floor.max(high_influence),
        LandformKind::Massif => profile.massif_floor.max(high_influence),
        LandformKind::SharpPeak => profile.sharp_peak_bench_min,
    }
}

fn water_level(cell: &CellPlan, profile: V3GrandV3BasicTerrainProfile) -> Level {
    if has_overlay(cell, SchematicFeature::MountainLake) {
        profile.mountain_lake_level
    } else if has_overlay(cell, SchematicFeature::ValleyLake) {
        profile.valley_lake_level
    } else {
        profile.sea_level
    }
}

fn water_bed_level(cell: &CellPlan, coord: HexCoord, water: Level, seed: u64) -> Level {
    if has_overlay(cell, SchematicFeature::MountainLake) {
        144_i32.saturating_add(
            i32::try_from(named_sample(seed, "mountain_lake_bed", coord) % 6).unwrap_or_default(),
        )
    } else if has_overlay(cell, SchematicFeature::ValleyLake) {
        water.saturating_sub(1)
    } else {
        water.saturating_sub(2)
    }
}

fn fine_surface_level(
    cell: &CellPlan,
    coord: HexCoord,
    centers: &BTreeMap<PatchId, HexCoord>,
    datums: &BTreeMap<PatchId, Level>,
    relief_caps: &BTreeMap<PatchId, Level>,
    highlands: &super::schematic_highlands::GrandHighlandField,
    profile: V3GrandV3BasicTerrainProfile,
    seed: u64,
) -> Level {
    let owner = PatchId(u32::from(cell.id.get()));
    let center = centers.get(&owner).copied().unwrap_or(coord);
    if has_overlay(cell, SchematicFeature::FrozenWoods) {
        let baseline = profile.frozen_woods_level;
        return highlands.resolve_surface_level(cell, coord, baseline);
    }
    if has_overlay(cell, SchematicFeature::CrystalAscent) {
        return profile
            .crystal_base_level
            .saturating_add(profile.crystal_rise_levels);
    }
    if has_overlay(cell, SchematicFeature::LakeIsland) {
        return schematic_ecology::lake_island_surface_level(coord, center, profile, seed);
    }
    let base = *datums.get(&owner).unwrap_or(&profile.valley_level);
    let mut weighted_sum = 0_i64;
    let mut weighted_relief = 0_i64;
    let mut weight_sum = 0_i64;
    for (neighbor, neighbor_center) in centers {
        let distance = neighbor_center.distance(coord);
        if distance > 22 {
            continue;
        }
        let weight = i64::from(23_u32.saturating_sub(distance));
        weighted_sum =
            weighted_sum.saturating_add(i64::from(*datums.get(neighbor).unwrap_or(&base)) * weight);
        weighted_relief = weighted_relief.saturating_add(
            i64::from(*relief_caps.get(neighbor).unwrap_or(&0)).saturating_mul(weight),
        );
        weight_sum = weight_sum.saturating_add(weight);
    }
    let blended = i32::try_from(weighted_sum / weight_sum.max(1)).unwrap_or(base);
    let cap = i32::try_from(weighted_relief / weight_sum.max(1)).unwrap_or_default();
    highlands.resolve_surface_level(
        cell,
        coord,
        blended.saturating_add(smooth_relief(coord, cap, seed)),
    )
}

const fn relief_cap(landform: LandformKind) -> Level {
    match landform {
        LandformKind::None => 0,
        LandformKind::Island | LandformKind::Beach | LandformKind::Shore => 1,
        LandformKind::Valley | LandformKind::Plateau => 2,
        LandformKind::Hill => 4,
        LandformKind::Mountain => 8,
        LandformKind::Massif => 12,
        LandformKind::SharpPeak => 0,
    }
}

fn smooth_relief(coord: HexCoord, cap: Level, seed: u64) -> Level {
    if cap <= 0 {
        return 0;
    }
    let q = i64::from(coord.x());
    let r = i64::from(coord.y());
    let phases = [
        i64::try_from(seed % 61).unwrap_or_default(),
        i64::try_from(seed.rotate_left(17) % 83).unwrap_or_default(),
        i64::try_from(seed.rotate_left(41) % 113).unwrap_or_default(),
    ];
    let waves = [
        normalized_triangle(q.saturating_add(r.saturating_mul(2)) + phases[0], 31),
        normalized_triangle(q.saturating_mul(2).saturating_sub(r) + phases[1], 43),
        normalized_triangle(q.saturating_sub(r.saturating_mul(3)) + phases[2], 59),
    ];
    let normalized = waves.into_iter().sum::<i64>() / 3;
    i32::try_from(i64::from(cap).saturating_mul(normalized) / 1_024).unwrap_or_default()
}

fn normalized_triangle(value: i64, half_period: i64) -> i64 {
    let period = half_period.saturating_mul(2);
    let position = value.rem_euclid(period);
    let rising = half_period.saturating_sub((position - half_period).abs());
    rising
        .saturating_mul(2)
        .saturating_sub(half_period)
        .saturating_mul(1_024)
        / half_period
}

fn named_sample(seed: u64, stream: &str, coord: HexCoord) -> u64 {
    let mut state = 0xcbf2_9ce4_8422_2325_u64;
    for bytes in [
        seed.to_le_bytes().as_slice(),
        stream.as_bytes(),
        coord.x().to_le_bytes().as_slice(),
        coord.y().to_le_bytes().as_slice(),
    ] {
        for byte in bytes {
            state ^= u64::from(*byte);
            state = state.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    state
}

fn land_cap_material(cell: &CellPlan) -> SolidMaterialRole {
    if cell.facts.climate == hex_schematic::ClimateKind::Frozen {
        return SolidMaterialRole::Snow;
    }
    match cell.facts.landform {
        LandformKind::Island | LandformKind::Beach => SolidMaterialRole::Sand,
        LandformKind::Shore
        | LandformKind::Mountain
        | LandformKind::Massif
        | LandformKind::SharpPeak => SolidMaterialRole::Gravel,
        LandformKind::None | LandformKind::Valley | LandformKind::Plateau | LandformKind::Hill => {
            SolidMaterialRole::Grass
        }
    }
}

fn water_bed_material(cell: &CellPlan) -> SolidMaterialRole {
    if has_overlay(cell, SchematicFeature::MountainLake)
        || has_overlay(cell, SchematicFeature::ValleyLake)
    {
        SolidMaterialRole::Gravel
    } else {
        SolidMaterialRole::Sand
    }
}

fn push_canonical_solid(elements: &mut Vec<VolumeElement>, mass: SolidMass) {
    let merge = elements.last_mut().and_then(|element| {
        let VolumeElement::Solid(previous) = element else {
            return None;
        };
        (previous.material == mass.material
            && previous.cutaway_for == mass.cutaway_for
            && previous.levels.top == mass.levels.bottom)
            .then_some(previous)
    });
    if let Some(previous) = merge {
        previous.levels.top = mass.levels.top;
    } else {
        elements.push(VolumeElement::Solid(mass));
    }
}

fn land_column(surface: Level, cap: SolidMaterialRole) -> VolumeColumn {
    let subsoil = surface.saturating_sub(2).max(2);
    let mut elements = vec![VolumeElement::Solid(SolidMass {
        levels: LevelInterval::new(0, 1),
        material: SolidMaterialRole::Bedrock,
        cutaway_for: None,
    })];
    if subsoil > 1 {
        elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(1, subsoil),
            material: SolidMaterialRole::Stone,
            cutaway_for: None,
        }));
    }
    if surface > subsoil {
        elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(subsoil, surface),
            material: SolidMaterialRole::Dirt,
            cutaway_for: None,
        }));
    }
    push_canonical_solid(
        &mut elements,
        SolidMass {
            levels: LevelInterval::new(surface, surface.saturating_add(1)),
            material: cap,
            cutaway_for: None,
        },
    );
    VolumeColumn { elements }
}

fn water_column(bed: Level, water: Level, cap: SolidMaterialRole) -> VolumeColumn {
    let mut elements = vec![
        VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(0, 1),
            material: SolidMaterialRole::Bedrock,
            cutaway_for: None,
        }),
        VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(1, bed),
            material: SolidMaterialRole::Stone,
            cutaway_for: None,
        }),
        VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(bed, bed.saturating_add(1)),
            material: cap,
            cutaway_for: None,
        }),
    ];
    elements.push(VolumeElement::Fill(NonSolidFill {
        levels: LevelInterval::new(bed.saturating_add(1), water.saturating_add(1)),
        material: FillMaterialRole::Water,
    }));
    VolumeColumn { elements }
}

/// Perturbs only the fine mainland/ocean boundary. Coarse ownership never
/// changes: a sea-owned column may expose a small sand spit and a land-owned
/// column may become a shallow inlet, while semantic biome IDs stay fixed.
fn apply_coast_detail(
    plan: &SchematicPlanV1,
    seed: u64,
    profile: V3GrandV3BasicTerrainProfile,
    layout: &ResolvedLayoutPlan,
    fine_index: &FineWorldIndex,
    volume: &mut VolumePlan,
    biome_regions: &mut BTreeMap<TilePos, hex_core::BiomeRegionId>,
) -> Result<(), V3GenerationError> {
    let outlet_reservation = authoritative_outlet_reservation(plan, profile, seed, layout)?;
    let cells = plan
        .cells
        .iter()
        .map(|cell| (PatchId(u32::from(cell.id.get())), cell))
        .collect::<BTreeMap<_, _>>();
    let sea_patches = cells
        .iter()
        .filter_map(|(patch, cell)| {
            (cell.facts.surface == SurfaceKind::OpenWater
                && !has_overlay(cell, SchematicFeature::MountainLake)
                && !has_overlay(cell, SchematicFeature::ValleyLake))
            .then_some(*patch)
        })
        .collect::<BTreeSet<_>>();
    let locked_patches = cells
        .iter()
        .filter_map(|(patch, cell)| {
            cell.facts
                .overlays
                .iter()
                .any(|overlay| {
                    matches!(
                        overlay,
                        SchematicFeature::MountainLake
                            | SchematicFeature::LakeIsland
                            | SchematicFeature::FrozenWoods
                            | SchematicFeature::PeakRing
                            | SchematicFeature::CrystalAscent
                    )
                })
                .then_some(*patch)
        })
        .collect::<BTreeSet<_>>();
    let island_patches = cells
        .iter()
        .filter_map(|(patch, cell)| {
            has_overlay(cell, SchematicFeature::SeaIsland).then_some(*patch)
        })
        .collect::<BTreeSet<_>>();
    let original_sea = fine_index
        .by_coord
        .iter()
        .filter_map(|(coord, owner)| sea_patches.contains(&owner.patch).then_some(*coord))
        .collect::<BTreeSet<_>>();
    let mut candidate_mutations = BTreeMap::new();
    for (coord, owner) in &fine_index.by_coord {
        if locked_patches.contains(&owner.patch) || outlet_reservation.contains(coord) {
            continue;
        }
        let is_sea = sea_patches.contains(&owner.patch);
        let opposite_neighbors = coord
            .neighbors()
            .into_iter()
            .filter(|neighbor| {
                fine_index
                    .patch(*neighbor)
                    .is_some_and(|neighbor_owner| sea_patches.contains(&neighbor_owner) != is_sea)
            })
            .count();
        if opposite_neighbors == 0 || named_sample(seed, "coast_detail", *coord) % 100 >= 46 {
            continue;
        }
        if is_sea {
            let touches_mainland = coord.neighbors().into_iter().any(|neighbor| {
                fine_index.patch(neighbor).is_some_and(|neighbor_owner| {
                    !sea_patches.contains(&neighbor_owner)
                        && !island_patches.contains(&neighbor_owner)
                })
            });
            if touches_mainland {
                candidate_mutations.insert(*coord, CoastMutation::ToLand);
            }
        } else {
            candidate_mutations.insert(*coord, CoastMutation::ToWater);
        }
    }

    let original_mainland = plan
        .cells
        .iter()
        .filter(|cell| {
            cell.facts.surface == SurfaceKind::Land
                && cell.facts.access == AccessIntent::Ordinary
                && !has_overlay(cell, SchematicFeature::SeaIsland)
                && !has_overlay(cell, SchematicFeature::LakeIsland)
        })
        .flat_map(|cell| {
            layout
                .patches
                .get(&PatchId(u32::from(cell.id.get())))
                .into_iter()
                .flat_map(|patch| patch.mask.iter().copied())
        })
        .collect::<BTreeSet<_>>();
    if !connected_coords(&original_sea) || !connected_coords(&original_mainland) {
        return Err(schematic_contract(
            "coast_detail received disconnected authoritative sea or mainland",
        ));
    }

    // Accept mutations in canonical coordinate order while preserving both
    // authoritative components after every operation. Removal is allowed only
    // when the removed cell's remaining same-component neighbours stay locally
    // connected; any global path that formerly crossed the cell can therefore
    // be rerouted around its six-cell ring. Addition must touch the destination
    // component. These two local proofs maintain the global connectivity
    // invariant without running a whole-world flood fill for every candidate.
    let mut sea_after = original_sea.clone();
    let mut mainland_after = original_mainland.clone();
    let mut to_water = BTreeSet::new();
    let mut to_land = BTreeSet::new();
    for (coord, mutation) in candidate_mutations {
        match mutation {
            CoastMutation::ToLand => {
                if removal_preserves_connectedness_locally(&sea_after, coord)
                    && coord
                        .neighbors()
                        .into_iter()
                        .any(|neighbor| mainland_after.contains(&neighbor))
                {
                    sea_after.remove(&coord);
                    mainland_after.insert(coord);
                    to_land.insert(coord);
                }
            }
            CoastMutation::ToWater => {
                let preserves_mainland = !mainland_after.contains(&coord)
                    || removal_preserves_connectedness_locally(&mainland_after, coord);
                if preserves_mainland
                    && coord
                        .neighbors()
                        .into_iter()
                        .any(|neighbor| sea_after.contains(&neighbor))
                {
                    mainland_after.remove(&coord);
                    sea_after.insert(coord);
                    to_water.insert(coord);
                }
            }
        }
    }

    for coord in &to_water {
        let bed = profile.sea_level.saturating_sub(1);
        replace_column_surface(
            volume,
            biome_regions,
            *coord,
            water_column(bed, profile.sea_level, SolidMaterialRole::Sand),
            TilePos::new(*coord, bed),
            SurfaceMetadata {
                access: SurfaceAccess::NonStandable,
                interior: None,
            },
            fine_index.biome(*coord).ok_or_else(|| {
                schematic_contract(format!("coast water mutation {coord:?} has no biome owner"))
            })?,
        );
    }
    for coord in &to_land {
        let level = profile.beach_level.max(profile.sea_level.saturating_add(1));
        replace_column_surface(
            volume,
            biome_regions,
            *coord,
            land_column(level, SolidMaterialRole::Sand),
            TilePos::new(*coord, level),
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: None,
            },
            fine_index.biome(*coord).ok_or_else(|| {
                schematic_contract(format!("coast land mutation {coord:?} has no biome owner"))
            })?,
        );
    }

    // The initial full floods above establish the induction base. Every
    // accepted removal then reconnects all surviving neighbours inside the
    // removed cell's six-cell ring, while every addition touches the connected
    // destination component. Those constructive checks prove both components
    // remain connected after every mutation, so repeating two whole-world
    // release floods here would only replay the same proof. Keep independent
    // debug postconditions for development builds; the exhaustive local-proof
    // unit wedge below guards the admission predicate itself.
    debug_assert!(
        connected_coords(&sea_after),
        "constructive coast detail disconnected the single south-west sea"
    );
    debug_assert!(
        connected_coords(&mainland_after),
        "constructive coast detail disconnected the ordinary mainland"
    );
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CoastMutation {
    ToWater,
    ToLand,
}

/// Sufficient local proof that removing `coord` cannot disconnect a connected
/// hex component. Every surviving neighbour must still be mutually reachable
/// within the six-cell ring around the removed coordinate.
fn removal_preserves_connectedness_locally(
    component: &BTreeSet<HexCoord>,
    coord: HexCoord,
) -> bool {
    if !component.contains(&coord) || component.len() <= 1 {
        return false;
    }
    let neighbors = coord
        .neighbors()
        .into_iter()
        .filter(|neighbor| component.contains(neighbor))
        .collect::<BTreeSet<_>>();
    let Some(start) = neighbors.first().copied() else {
        return false;
    };
    let mut reached = BTreeSet::from([start]);
    let mut frontier = VecDeque::from([start]);
    while let Some(current) = frontier.pop_front() {
        for neighbor in current.neighbors() {
            if neighbors.contains(&neighbor) && reached.insert(neighbor) {
                frontier.push_back(neighbor);
            }
        }
    }
    reached.len() == neighbors.len()
}

fn connected_coords(coords: &BTreeSet<HexCoord>) -> bool {
    let Some(start) = coords.first().copied() else {
        return false;
    };
    // Canonical traversal order is provided by the fixed neighbour array; set
    // iteration order is never observed. Hash membership therefore preserves
    // the exact boolean contract while avoiding logarithmic lookup on each of
    // the six probes for every fine-world coordinate.
    let membership = coords.iter().copied().collect::<HashSet<_>>();
    let mut reached = HashSet::from([start]);
    let mut frontier = VecDeque::from([start]);
    while let Some(coord) = frontier.pop_front() {
        for neighbor in coord.neighbors() {
            if membership.contains(&neighbor) && reached.insert(neighbor) {
                frontier.push_back(neighbor);
            }
        }
    }
    reached.len() == coords.len()
}

fn compile_authoritative_hydrology(
    plan: &SchematicPlanV1,
    profile: V3GrandV3BasicTerrainProfile,
    seed: u64,
    layout: &ResolvedLayoutPlan,
    fine_index: &FineWorldIndex,
    volume: &mut VolumePlan,
    biome_regions: &mut BTreeMap<TilePos, hex_core::BiomeRegionId>,
) -> Result<HydrologyCompilation, V3GenerationError> {
    let (waterfall_centerline, river_centerline) =
        authoritative_hydrology_centerlines(plan, profile, seed, layout)?;
    let mut complete_centerline = waterfall_centerline.clone();
    append_path(&mut complete_centerline, river_centerline.clone());
    let mut watercourse_rows = build_three_lane_rows(
        &complete_centerline,
        &layout.footprint,
        "authoritative hydrology",
    )?;

    let mut authored_water = BTreeMap::<HexCoord, (Level, Level, SolidMaterialRole)>::new();
    let mut claim_indices = BTreeMap::<HexCoord, Vec<usize>>::new();
    for (index, row) in watercourse_rows.iter().enumerate() {
        let falling = index.saturating_add(1) < waterfall_centerline.len();
        let next_level = watercourse_rows
            .get(index.saturating_add(1))
            .and_then(|next| next.first())
            .map_or_else(
                || row.first().map_or(0, |position| position.level),
                |next| next.level,
            );
        for position in row {
            let bed = if falling {
                position.level.saturating_sub(1).min(next_level)
            } else {
                position.level.saturating_sub(1)
            };
            let cap = if falling {
                SolidMaterialRole::Gravel
            } else {
                SolidMaterialRole::Dirt
            };
            claim_indices.entry(position.coord).or_default().push(index);
            authored_water
                .entry(position.coord)
                .and_modify(|claim| {
                    claim.0 = claim.0.min(bed);
                    claim.1 = claim.1.min(position.level);
                    if cap == SolidMaterialRole::Gravel {
                        claim.2 = cap;
                    }
                })
                .or_insert((bed, position.level, cap));
        }
    }
    if let Some((coord, claims)) = claim_indices.iter().find(|(_, claims)| {
        claims
            .last()
            .zip(claims.first())
            .is_some_and(|(last, first)| last.saturating_sub(*first) > 2)
    }) {
        return Err(schematic_contract(format!(
            "authoritative hydrology has a nonlocal self-overlap at {coord:?}: {claims:?}"
        )));
    }
    for row in &mut watercourse_rows {
        *row = row
            .iter()
            .map(|position| TilePos::new(position.coord, authored_water[&position.coord].1))
            .collect();
    }
    let river_start = waterfall_centerline.len().saturating_sub(1);
    let river_rows = watercourse_rows
        .get(river_start..)
        .ok_or_else(|| schematic_contract("river row projection starts outside hydrology"))?
        .to_vec();

    for (coord, (bed, water, cap)) in authored_water {
        if !layout.footprint.contains(&coord) {
            return Err(schematic_contract(format!(
                "authoritative hydrology leaves the radius-187 footprint at {coord:?}"
            )));
        }
        replace_column_surface(
            volume,
            biome_regions,
            coord,
            water_column(bed, water, cap),
            TilePos::new(coord, bed),
            SurfaceMetadata {
                access: SurfaceAccess::NonStandable,
                interior: None,
            },
            fine_index.biome(coord).ok_or_else(|| {
                schematic_contract(format!("water column {coord:?} has no biome owner"))
            })?,
        );
    }

    // Recess every exposed water edge, including both outer ribbon lanes and
    // the fixed lake/sea boundaries. The final validator repeats this proof
    // after routes, structures, and decoration have all been authored.
    enforce_recessed_water_banks(volume, biome_regions, fine_index);

    let semantic_sea = semantic_sea_coords(plan, layout);
    let outlet_authority = exact_outlet_authority(&river_centerline, &river_rows, &semantic_sea)?;
    let fill_runs = volume.fill_runs_by_top();
    let mut nodes = fill_runs
        .keys()
        .copied()
        .map(|position| {
            (
                position,
                LiquidNode {
                    state: LiquidFlowState::Still,
                    downstream: None,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    apply_directed_watercourse(&watercourse_rows, &outlet_authority, &fill_runs, &mut nodes)?;
    let outlet = exact_three_lane_outlet(&nodes, &outlet_authority)?;
    let liquids = liquid_components_with_flow(nodes);
    let water_coords = volume
        .fill_runs_by_top()
        .keys()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();

    // Keep one independent feature stream observable without allowing it to
    // perturb coast, terrain relief, or schematic candidate selection.
    let _feature_variant = named_sample(seed, "feature_variants", river_centerline[0].coord);

    Ok(HydrologyCompilation {
        liquids,
        water_coords,
        river_centerline,
        waterfall_centerline,
        watercourse_rows,
        river_rows,
        outlet,
    })
}

fn authoritative_hydrology_centerlines(
    plan: &SchematicPlanV1,
    profile: V3GrandV3BasicTerrainProfile,
    seed: u64,
    layout: &ResolvedLayoutPlan,
) -> Result<(Vec<TilePos>, Vec<TilePos>), V3GenerationError> {
    let lake_to_falls =
        schematic_network_path(plan, NetworkKind::Hydrology, "edge/hydrology-lake-to-falls")?;
    let falls_to_valley = schematic_network_path(
        plan,
        NetworkKind::Hydrology,
        "edge/hydrology-falls-to-valley",
    )?;
    let valley_to_sea =
        schematic_network_path(plan, NetworkKind::Hydrology, "edge/hydrology-valley-to-sea")?;

    let mut waterfall_coords = fine_network_path(&lake_to_falls, 22);
    let plunge_lip_index = waterfall_coords.len().saturating_sub(1);
    append_path(
        &mut waterfall_coords,
        fine_network_path(&falls_to_valley, 22),
    );
    let semantic_sea = semantic_sea_coords(plan, layout);
    let river_coords =
        meandering_fine_network_path(&valley_to_sea, 22, seed, &layout.footprint, &semantic_sea)?;
    if waterfall_coords.len() < 2 || river_coords.len() < 2 {
        return Err(schematic_contract(
            "authoritative hydrology paths must contain at least two fine columns",
        ));
    }

    let waterfall_levels = plunge_levels(
        profile.mountain_lake_level,
        profile.valley_lake_level,
        waterfall_coords.len(),
        plunge_lip_index,
    )?;
    let river_levels = descending_levels(
        profile.valley_lake_level,
        profile.sea_level,
        river_coords.len(),
    );
    let waterfall_centerline = waterfall_coords
        .iter()
        .copied()
        .zip(waterfall_levels.iter().copied())
        .map(|(coord, level)| TilePos::new(coord, level))
        .collect::<Vec<_>>();
    let river_centerline = river_coords
        .iter()
        .copied()
        .zip(river_levels.iter().copied())
        .map(|(coord, level)| TilePos::new(coord, level))
        .collect::<Vec<_>>();
    if waterfall_centerline.last() != river_centerline.first() {
        return Err(schematic_contract(
            "waterfall and river do not share the exact valley-lake junction",
        ));
    }
    Ok((waterfall_centerline, river_centerline))
}

fn schematic_network_path(
    plan: &SchematicPlanV1,
    kind: NetworkKind,
    edge_id: &str,
) -> Result<Vec<SchematicCoord>, V3GenerationError> {
    plan.networks
        .iter()
        .find(|network| network.kind == kind)
        .and_then(|network| {
            network
                .edges
                .iter()
                .find(|edge| edge.id.as_str() == edge_id)
        })
        .map(|edge| edge.path.clone())
        .ok_or_else(|| schematic_contract(format!("missing exact schematic edge {edge_id}")))
}

fn fine_network_path(coarse: &[SchematicCoord], pitch: i32) -> Vec<HexCoord> {
    let mut result = Vec::new();
    for pair in coarse.windows(2) {
        append_path(
            &mut result,
            schematic_to_world(pair[0], pitch).line_between(schematic_to_world(pair[1], pitch)),
        );
    }
    if result.is_empty() {
        result.extend(
            coarse
                .first()
                .map(|coord| schematic_to_world(*coord, pitch)),
        );
    }
    result
}

/// Resolves a seed-stable fine river inside the corridor declared by its coarse
/// schematic edge.
///
/// Coarse cell centres remain exact waypoints, but intermediate segments bow by
/// four or five columns. That keeps the schematic drainage graph authoritative
/// without turning its pitch-22 edges into ruler-straight canals. The final
/// segment remains straight so the existing one-transition sea outlet and its
/// three-lane matching stay exact.
fn meandering_fine_network_path(
    coarse: &[SchematicCoord],
    pitch: i32,
    seed: u64,
    footprint: &BTreeSet<HexCoord>,
    semantic_sea: &BTreeSet<HexCoord>,
) -> Result<Vec<HexCoord>, V3GenerationError> {
    const MAX_ATTEMPTS: u32 = 32;
    const MINIMUM_LATERAL_EXCURSION: u32 = 3;

    if coarse.len() < 2 || pitch < 8 {
        return Err(schematic_contract(
            "river meander requires at least one coarse edge and pitch eight",
        ));
    }
    let direct = fine_network_path(coarse, pitch);
    let direct_set = direct.iter().copied().collect::<BTreeSet<_>>();
    let corridor_radius = pitch.unsigned_abs().saturating_div(2).saturating_add(5);
    let corridor = coarse
        .iter()
        .flat_map(|coord| schematic_to_world(*coord, pitch).within_radius(corridor_radius))
        .collect::<BTreeSet<_>>();

    for attempt in 0..MAX_ATTEMPTS {
        let candidate = river_meander_candidate(coarse, pitch, seed, attempt);
        if candidate.first() != direct.first()
            || candidate.last() != direct.last()
            || candidate.len() < direct.len()
            || candidate
                .windows(2)
                .any(|pair| pair[0].distance(pair[1]) != 1)
            || candidate.iter().copied().collect::<BTreeSet<_>>().len() != candidate.len()
            || candidate
                .iter()
                .any(|coord| !footprint.contains(coord) || !corridor.contains(coord))
        {
            continue;
        }
        let excursion = candidate
            .iter()
            .map(|coord| {
                direct_set
                    .iter()
                    .map(|direct_coord| coord.distance(*direct_coord))
                    .min()
                    .unwrap_or_default()
            })
            .max()
            .unwrap_or_default();
        if excursion < MINIMUM_LATERAL_EXCURSION
            || longest_straight_run(&candidate)
                >= usize::try_from(pitch.saturating_mul(2)).unwrap_or(usize::MAX)
        {
            continue;
        }

        let level_path = candidate
            .iter()
            .copied()
            .map(|coord| TilePos::new(coord, 0))
            .collect::<Vec<_>>();
        let Ok(rows) = build_three_lane_rows(&level_path, footprint, "river meander candidate")
        else {
            continue;
        };
        if exact_outlet_authority(&level_path, &rows, semantic_sea).is_err() {
            continue;
        }
        return Ok(candidate);
    }

    Err(schematic_contract(
        "the named hydrology stream cannot resolve a simple three-lane meander inside the declared river corridor",
    ))
}

fn river_meander_candidate(
    coarse: &[SchematicCoord],
    pitch: i32,
    seed: u64,
    attempt: u32,
) -> Vec<HexCoord> {
    let mut result = Vec::new();
    let segment_count = coarse.len().saturating_sub(1);
    for (segment_index, pair) in coarse.windows(2).enumerate() {
        let start = schematic_to_world(pair[0], pitch);
        let end = schematic_to_world(pair[1], pitch);
        let direct = start.line_between(end);
        if segment_index.saturating_add(1) == segment_count || direct.len() < 9 {
            append_path(&mut result, direct);
            continue;
        }
        let direction = direct
            .get(1)
            .and_then(|next| {
                start
                    .neighbors()
                    .iter()
                    .position(|neighbor| neighbor == next)
            })
            .unwrap_or(0);
        let stream = format!("hydrology_river_meander/{attempt}/{segment_index}");
        let sample = named_sample(seed, &stream, start);
        // Four columns is the narrowest return dogleg for which the exact
        // three-wide row solver can keep both banks disjoint at every turn.
        let amplitude = 4_u32.saturating_add(u32::try_from(sample % 2).unwrap_or_default());
        let side_direction = if sample & 2 == 0 {
            (direction + 1) % 6
        } else {
            (direction + 5) % 6
        };
        let distance = start.distance(end);
        let run_variation = distance.saturating_div(5).max(1);
        let first_run = distance
            .saturating_div(4)
            .saturating_add(u32::try_from((sample >> 8) % u64::from(run_variation)).unwrap_or(0));
        let final_run = distance
            .saturating_div(4)
            .saturating_add(u32::try_from((sample >> 16) % u64::from(run_variation)).unwrap_or(0));
        let middle_run = distance.saturating_sub(first_run).saturating_sub(final_run);
        append_path(&mut result, vec![start]);
        let mut current = start;
        append_direction_run(&mut result, &mut current, direction, first_run);
        append_direction_run(&mut result, &mut current, side_direction, amplitude);
        append_direction_run(&mut result, &mut current, direction, middle_run);
        append_direction_run(
            &mut result,
            &mut current,
            (side_direction + 3) % 6,
            amplitude,
        );
        append_direction_run(&mut result, &mut current, direction, final_run);
        if current != end {
            return Vec::new();
        }
    }
    result
}

fn append_direction_run(
    path: &mut Vec<HexCoord>,
    current: &mut HexCoord,
    direction: usize,
    steps: u32,
) {
    for _ in 0..steps {
        *current = current.neighbors()[direction];
        path.push(*current);
    }
}

fn longest_straight_run(path: &[HexCoord]) -> usize {
    let mut longest = 0_usize;
    let mut current = 0_usize;
    let mut previous_direction = None;
    for pair in path.windows(2) {
        let direction = pair[0]
            .neighbors()
            .iter()
            .position(|neighbor| *neighbor == pair[1]);
        if direction == previous_direction {
            current = current.saturating_add(1);
        } else {
            current = usize::from(direction.is_some());
            previous_direction = direction;
        }
        longest = longest.max(current);
    }
    longest
}

const CRYSTAL_CONNECTOR_SITE_RADIUS: u32 = 32;
const CRYSTAL_CONNECTOR_RING_RADIUS: u32 = 33;
const TUNNEL_LANE_WIDTH: usize = 4;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct TunnelRingCandidateScore {
    connector_rows: usize,
    direction_rank: u8,
    bias_index: usize,
    start_window: usize,
    goal_window: usize,
    anchor_offset: usize,
    terminal_representative: HexCoord,
    centerline: Vec<HexCoord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ResolvedTunnelLane {
    centerline: Vec<HexCoord>,
    rows: Vec<BTreeSet<HexCoord>>,
    lane_offsets: [i32; 4],
}

/// Slides an exact four-wide corridor around the first ring outside the convex
/// Crystal site until it smoothly meets the locked coarse route.
///
/// Row zero remains the authored radius-32 terminal verbatim. Every generated
/// row is a consecutive four-cell window of radius 33, including windows which
/// straddle a corner and therefore cannot be represented by one straight row
/// axis. Candidate arcs are ordered by length, then by the canonical direction
/// of the deterministic ring walk. The complete locked suffix, including its
/// first straight row, is appended untouched after one adjacent ring anchor.
fn resolve_exact_terminal_lane(
    lower_terminal: &BTreeSet<TilePos>,
    crystal_mask: &BTreeSet<HexCoord>,
    footprint: &BTreeSet<HexCoord>,
    mut locked_centerline: Vec<HexCoord>,
) -> Option<ResolvedTunnelLane> {
    if locked_centerline.len() < 2 || lower_terminal.len() != TUNNEL_LANE_WIDTH {
        return None;
    }
    let terminal_coords = lower_terminal
        .iter()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    let site_center = exact_hex_disk_center(crystal_mask, CRYSTAL_CONNECTOR_SITE_RADIUS)?;
    let site_ring = canonical_convex_hex_ring(site_center, CRYSTAL_CONNECTOR_SITE_RADIUS)?;
    if !terminal_coords.is_subset(crystal_mask)
        || !(0..site_ring.len()).any(|start| {
            convex_ring_window(&site_ring, start, TUNNEL_LANE_WIDTH)
                .is_some_and(|window| window == terminal_coords)
        })
    {
        return None;
    }
    let distance_to_terminal = |coord: HexCoord| {
        terminal_coords
            .iter()
            .map(|terminal| coord.distance(*terminal))
            .min()
            .unwrap_or(u32::MAX)
    };
    let first_distance = distance_to_terminal(*locked_centerline.first()?);
    let last_distance = distance_to_terminal(*locked_centerline.last()?);
    if first_distance == last_distance {
        return None;
    }
    if last_distance < first_distance {
        locked_centerline.reverse();
    }
    if locked_centerline
        .windows(2)
        .any(|pair| pair[0].distance(pair[1]) != 1)
        || locked_centerline
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .len()
            != locked_centerline.len()
    {
        return None;
    }

    let first_outside = locked_centerline
        .iter()
        .position(|coord| !crystal_mask.contains(coord))?;
    if first_outside == 0
        || locked_centerline
            .iter()
            .skip(first_outside)
            .any(|coord| crystal_mask.contains(coord))
    {
        return None;
    }
    if first_outside.saturating_add(1) >= locked_centerline.len() {
        return None;
    }
    let locked_suffix = locked_centerline
        .iter()
        .copied()
        .skip(first_outside)
        .collect::<Vec<_>>();
    let goal_center = *locked_suffix.first()?;
    if site_center.distance(goal_center) != CRYSTAL_CONNECTOR_RING_RADIUS {
        return None;
    }

    let connector_ring = canonical_convex_hex_ring(site_center, CRYSTAL_CONNECTOR_RING_RADIUS)?;
    let start_windows = (0..connector_ring.len())
        .filter_map(|start| {
            let row = convex_ring_window(&connector_ring, start, TUNNEL_LANE_WIDTH)?;
            (valid_outside_tunnel_row(&row, crystal_mask, footprint)
                && lane_rows_connect_smoothly(&terminal_coords, &row))
            .then_some((start, row))
        })
        .collect::<Vec<_>>();
    if start_windows.is_empty() {
        return None;
    }

    const EVEN_WIDTH_BIASES: [[i32; 4]; 2] = [[-1, 0, 1, 2], [-2, -1, 0, 1]];
    let mut best: Option<(TunnelRingCandidateScore, ResolvedTunnelLane)> = None;
    for (bias_index, offsets) in EVEN_WIDTH_BIASES.into_iter().enumerate() {
        let locked_rows = locked_suffix
            .iter()
            .enumerate()
            .map(|(index, _)| tunnel_lane_row(&locked_suffix, index, offsets))
            .collect::<Vec<_>>();
        if locked_rows.iter().any(|row| {
            row.len() != TUNNEL_LANE_WIDTH
                || row
                    .iter()
                    .any(|coord| crystal_mask.contains(coord) || !footprint.contains(coord))
        }) || locked_rows
            .windows(2)
            .any(|pair| !lane_rows_connect_smoothly(&pair[0], &pair[1]))
        {
            continue;
        }

        let Some(locked_goal_row) = locked_rows.first() else {
            continue;
        };
        let goal_windows = (0..connector_ring.len())
            .filter_map(|start| {
                let row = convex_ring_window(&connector_ring, start, TUNNEL_LANE_WIDTH)?;
                (valid_outside_tunnel_row(&row, crystal_mask, footprint)
                    && lane_rows_connect_smoothly(&row, locked_goal_row))
                .then_some((start, row))
            })
            .collect::<Vec<_>>();

        for (start_window, _) in &start_windows {
            for (goal_window, _) in &goal_windows {
                for (direction_rank, arc) in
                    preferred_cyclic_ring_arcs(connector_ring.len(), *start_window, *goal_window)
                {
                    let connector_rows = arc
                        .iter()
                        .filter_map(|start| {
                            convex_ring_window(&connector_ring, *start, TUNNEL_LANE_WIDTH)
                        })
                        .collect::<Vec<_>>();
                    if connector_rows.len() != arc.len()
                        || connector_rows
                            .iter()
                            .any(|row| !valid_outside_tunnel_row(row, crystal_mask, footprint))
                        || connector_rows
                            .windows(2)
                            .any(|pair| !lane_rows_connect_smoothly(&pair[0], &pair[1]))
                    {
                        continue;
                    }

                    for anchor_offset in 0..TUNNEL_LANE_WIDTH {
                        let anchors = arc
                            .iter()
                            .filter_map(|start| {
                                connector_ring
                                    .get(start.saturating_add(anchor_offset) % connector_ring.len())
                                    .copied()
                            })
                            .collect::<Vec<_>>();
                        let Some(first_anchor) = anchors.first().copied() else {
                            continue;
                        };
                        let Some(last_anchor) = anchors.last().copied() else {
                            continue;
                        };
                        let Some(terminal_representative) = terminal_coords
                            .iter()
                            .copied()
                            .filter(|terminal| terminal.distance(first_anchor) == 1)
                            .min()
                        else {
                            continue;
                        };
                        if anchors.len() != arc.len()
                            || last_anchor.distance(goal_center) != 1
                            || anchors.iter().any(|anchor| locked_suffix.contains(anchor))
                        {
                            continue;
                        }

                        let mut centerline = Vec::with_capacity(
                            1_usize
                                .saturating_add(anchors.len())
                                .saturating_add(locked_suffix.len()),
                        );
                        centerline.push(terminal_representative);
                        centerline.extend(anchors);
                        centerline.extend(locked_suffix.iter().copied());
                        if centerline
                            .windows(2)
                            .any(|pair| pair[0].distance(pair[1]) != 1)
                            || centerline.iter().copied().collect::<BTreeSet<_>>().len()
                                != centerline.len()
                        {
                            continue;
                        }

                        let mut rows = Vec::with_capacity(centerline.len());
                        rows.push(terminal_coords.clone());
                        rows.extend(connector_rows.iter().cloned());
                        rows.extend(locked_rows.iter().cloned());
                        if rows.len() != centerline.len()
                            || rows
                                .iter()
                                .skip(1)
                                .any(|row| !valid_outside_tunnel_row(row, crystal_mask, footprint))
                            || rows
                                .windows(2)
                                .any(|pair| !lane_rows_connect_smoothly(&pair[0], &pair[1]))
                        {
                            continue;
                        }

                        let score = TunnelRingCandidateScore {
                            connector_rows: connector_rows.len(),
                            direction_rank,
                            bias_index,
                            start_window: *start_window,
                            goal_window: *goal_window,
                            anchor_offset,
                            terminal_representative,
                            centerline: centerline.clone(),
                        };
                        if best.as_ref().is_none_or(|(current, ..)| score < *current) {
                            best = Some((
                                score,
                                ResolvedTunnelLane {
                                    centerline,
                                    rows,
                                    lane_offsets: offsets,
                                },
                            ));
                        }
                    }
                }
            }
        }
    }
    best.map(|(_, resolved)| resolved)
}

pub(super) fn exact_hex_disk_center(mask: &BTreeSet<HexCoord>, radius: u32) -> Option<HexCoord> {
    let first = *mask.first()?;
    let (mut minimum_x, mut maximum_x) = (first.x(), first.x());
    let (mut minimum_y, mut maximum_y) = (first.y(), first.y());
    let (mut minimum_z, mut maximum_z) = (first.z(), first.z());
    for coord in mask.iter().copied().skip(1) {
        minimum_x = minimum_x.min(coord.x());
        maximum_x = maximum_x.max(coord.x());
        minimum_y = minimum_y.min(coord.y());
        maximum_y = maximum_y.max(coord.y());
        minimum_z = minimum_z.min(coord.z());
        maximum_z = maximum_z.max(coord.z());
    }
    let midpoint = |minimum: i32, maximum: i32| {
        let sum = i64::from(minimum).checked_add(i64::from(maximum))?;
        if sum % 2 == 0 {
            i32::try_from(sum / 2).ok()
        } else {
            None
        }
    };
    let center = HexCoord::try_new_cubic(
        midpoint(minimum_x, maximum_x)?,
        midpoint(minimum_y, maximum_y)?,
        midpoint(minimum_z, maximum_z)?,
    )?;
    let expected = center
        .within_radius(radius)
        .into_iter()
        .collect::<BTreeSet<_>>();
    (expected == *mask).then_some(center)
}

/// Orders one convex hex ring from its canonical lowest coordinate toward its
/// canonical lowest adjacent ring coordinate.
fn canonical_convex_hex_ring(center: HexCoord, radius: u32) -> Option<Vec<HexCoord>> {
    if radius == 0 {
        return Some(vec![center]);
    }
    let ring_set = center
        .within_radius(radius)
        .into_iter()
        .filter(|coord| center.distance(*coord) == radius)
        .collect::<BTreeSet<_>>();
    let expected_len = usize::try_from(radius.checked_mul(6)?).ok()?;
    if ring_set.len() != expected_len {
        return None;
    }
    let start = *ring_set.first()?;
    if start
        .neighbors()
        .into_iter()
        .filter(|neighbor| ring_set.contains(neighbor))
        .count()
        != 2
    {
        return None;
    }
    let first_next = start
        .neighbors()
        .into_iter()
        .filter(|neighbor| ring_set.contains(neighbor))
        .min()?;
    let mut ordered = vec![start];
    let mut seen = BTreeSet::from([start]);
    let mut previous = start;
    let mut current = first_next;
    while current != start {
        if ordered.len() >= expected_len || !seen.insert(current) {
            return None;
        }
        ordered.push(current);
        let mut next = current
            .neighbors()
            .into_iter()
            .filter(|neighbor| *neighbor != previous && ring_set.contains(neighbor));
        let following = next.next()?;
        if next.next().is_some() {
            return None;
        }
        previous = current;
        current = following;
    }
    (ordered.len() == expected_len).then_some(ordered)
}

fn convex_ring_window(ring: &[HexCoord], start: usize, width: usize) -> Option<BTreeSet<HexCoord>> {
    if ring.is_empty() || start >= ring.len() || width == 0 || width > ring.len() {
        return None;
    }
    let row = (0..width)
        .filter_map(|offset| ring.get(start.saturating_add(offset) % ring.len()).copied())
        .collect::<BTreeSet<_>>();
    (row.len() == width).then_some(row)
}

fn cyclic_ring_arc(
    ring_len: usize,
    start: usize,
    goal: usize,
    reverse: bool,
) -> Option<Vec<usize>> {
    if ring_len == 0 || start >= ring_len || goal >= ring_len {
        return None;
    }
    let mut arc = Vec::new();
    let mut current = start;
    for _ in 0..ring_len {
        arc.push(current);
        if current == goal {
            return Some(arc);
        }
        current = if reverse {
            current.checked_sub(1).unwrap_or(ring_len.saturating_sub(1))
        } else {
            current.saturating_add(1) % ring_len
        };
    }
    None
}

fn preferred_cyclic_ring_arcs(ring_len: usize, start: usize, goal: usize) -> Vec<(u8, Vec<usize>)> {
    let mut arcs = [(0_u8, false), (1_u8, true)]
        .into_iter()
        .filter_map(|(direction_rank, reverse)| {
            cyclic_ring_arc(ring_len, start, goal, reverse).map(|arc| (direction_rank, arc))
        })
        .collect::<Vec<_>>();
    arcs.sort_unstable_by_key(|(direction_rank, arc)| (arc.len(), *direction_rank));
    arcs
}

fn valid_outside_tunnel_row(
    row: &BTreeSet<HexCoord>,
    crystal_mask: &BTreeSet<HexCoord>,
    footprint: &BTreeSet<HexCoord>,
) -> bool {
    row.len() == TUNNEL_LANE_WIDTH
        && row
            .iter()
            .all(|coord| !crystal_mask.contains(coord) && footprint.contains(coord))
}

fn lane_rows_connect_smoothly(first: &BTreeSet<HexCoord>, second: &BTreeSet<HexCoord>) -> bool {
    first.len() == 4
        && second.len() == 4
        && first
            .iter()
            .all(|coord| second.iter().any(|neighbor| coord.distance(*neighbor) <= 1))
        && second
            .iter()
            .all(|coord| first.iter().any(|neighbor| coord.distance(*neighbor) <= 1))
}

fn tunnel_lane_row(path: &[HexCoord], index: usize, offsets: [i32; 4]) -> BTreeSet<HexCoord> {
    let center = path[index];
    let direction = forward_path_direction(path, index);
    offsets
        .into_iter()
        .map(|offset| step_in_direction(center, (direction + 2) % 6, offset))
        .collect()
}

/// Direction of travel at one path point. Unlike the general widening helper,
/// the final point preserves the previous→current direction instead of facing
/// back into the route, so an asymmetric even-width row cannot shift sideways.
fn forward_path_direction(path: &[HexCoord], index: usize) -> usize {
    if let Some(next) = path.get(index.saturating_add(1)).copied() {
        return path[index]
            .neighbors()
            .iter()
            .position(|neighbor| *neighbor == next)
            .unwrap_or(0);
    }
    index
        .checked_sub(1)
        .and_then(|previous| {
            path[previous]
                .neighbors()
                .iter()
                .position(|neighbor| *neighbor == path[index])
        })
        .unwrap_or(0)
}

fn append_path<T: Copy + PartialEq>(target: &mut Vec<T>, mut addition: Vec<T>) {
    if target
        .last()
        .is_some_and(|last| addition.first() == Some(last))
    {
        addition.remove(0);
    }
    target.extend(addition);
}

fn descending_levels(start: Level, end: Level, count: usize) -> Vec<Level> {
    let transitions = count.saturating_sub(1).max(1);
    let drop = start.saturating_sub(end).max(0);
    (0..count)
        .map(|index| {
            let progressed = i64::from(drop)
                .saturating_mul(i64::try_from(index).unwrap_or(i64::MAX))
                / i64::try_from(transitions).unwrap_or(1);
            start.saturating_sub(i32::try_from(progressed).unwrap_or(drop))
        })
        .collect()
}

/// Keeps the mountain-lake spillway high through its authored lip, performs
/// one vertical plunge, and keeps the remaining approach at the valley-lake
/// level. Concentrating the drop in one transition gives liquid presentation a
/// true curtain instead of dozens of two-level stair-step falls.
fn plunge_levels(
    start: Level,
    end: Level,
    count: usize,
    plunge_lip_index: usize,
) -> Result<Vec<Level>, V3GenerationError> {
    if start <= end
        || count < 4
        || plunge_lip_index == 0
        || plunge_lip_index.saturating_add(2) >= count
    {
        return Err(schematic_contract(
            "waterfall plunge requires nonempty high and low approaches around a descending lip",
        ));
    }
    Ok((0..count)
        .map(|index| {
            if index <= plunge_lip_index {
                start
            } else {
                end
            }
        })
        .collect())
}

fn validate_plunge_profile(
    centerline: &[TilePos],
    start: Level,
    end: Level,
) -> Result<(), V3GenerationError> {
    if centerline.len() < 4
        || centerline
            .first()
            .is_none_or(|position| position.level != start)
        || centerline
            .last()
            .is_none_or(|position| position.level != end)
    {
        return Err(schematic_contract(
            "waterfall profile lost its high approach or low basin pin",
        ));
    }
    let drops = centerline
        .windows(2)
        .map(|pair| pair[0].level.saturating_sub(pair[1].level).max(0))
        .collect::<Vec<_>>();
    let total_drop = start.saturating_sub(end);
    let concentrated_drop = drops
        .windows(3.min(drops.len()))
        .map(|window| window.iter().copied().sum::<Level>())
        .max()
        .unwrap_or_default();
    let falling = drops
        .iter()
        .enumerate()
        .filter_map(|(index, drop)| (*drop > 0).then_some(index))
        .collect::<Vec<_>>();
    let Some(first_fall) = falling.first().copied() else {
        return Err(schematic_contract("waterfall profile contains no plunge"));
    };
    let last_fall = falling.last().copied().unwrap_or(first_fall);
    if total_drop <= 0
        || concentrated_drop.saturating_mul(10) < total_drop.saturating_mul(9)
        || falling.len() > 3
        || first_fall == 0
        || last_fall.saturating_add(2) >= centerline.len()
        || centerline[..=first_fall]
            .iter()
            .any(|position| position.level != start)
        || centerline[last_fall.saturating_add(1)..]
            .iter()
            .any(|position| position.level != end)
    {
        return Err(schematic_contract(
            "waterfall must keep a high spill approach, concentrate at least ninety percent of its drop within three transitions, and finish in a low basin",
        ));
    }
    Ok(())
}

fn validate_river_meander(
    plan: &SchematicPlanV1,
    centerline: &[TilePos],
) -> Result<(), V3GenerationError> {
    let coarse =
        schematic_network_path(plan, NetworkKind::Hydrology, "edge/hydrology-valley-to-sea")?;
    let direct = fine_network_path(&coarse, 22);
    let direct_set = direct.iter().copied().collect::<BTreeSet<_>>();
    let coords = centerline
        .iter()
        .map(|position| position.coord)
        .collect::<Vec<_>>();
    let maximum_excursion = coords
        .iter()
        .map(|coord| {
            direct_set
                .iter()
                .map(|direct_coord| coord.distance(*direct_coord))
                .min()
                .unwrap_or_default()
        })
        .max()
        .unwrap_or_default();
    if coords.first() != direct.first()
        || coords.last() != direct.last()
        || coords.iter().copied().collect::<BTreeSet<_>>().len() != coords.len()
        || coords.windows(2).any(|pair| pair[0].distance(pair[1]) != 1)
        || maximum_excursion < 3
        || longest_straight_run(&coords) >= 44
    {
        return Err(schematic_contract(
            "river must retain exact endpoints while following one simple, visibly bent fine-grid meander",
        ));
    }
    Ok(())
}

fn build_three_lane_rows(
    centerline: &[TilePos],
    footprint: &BTreeSet<HexCoord>,
    label: &str,
) -> Result<Vec<BTreeSet<TilePos>>, V3GenerationError> {
    if centerline.len() < 2
        || centerline
            .windows(2)
            .any(|pair| pair[0].coord.distance(pair[1].coord) != 1)
    {
        return Err(schematic_contract(format!(
            "{label} centerline is not a contiguous fine-grid path"
        )));
    }
    let coords = centerline
        .iter()
        .map(|position| position.coord)
        .collect::<Vec<_>>();
    #[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
    struct RowResolutionCost {
        noncanonical_rows: usize,
        orientation_changes: usize,
        axes: Vec<usize>,
    }

    #[derive(Clone, Debug)]
    struct RowResolution {
        cost: RowResolutionCost,
        rows: Vec<BTreeSet<TilePos>>,
    }

    let candidate_row = |center: TilePos, axis: usize| {
        (-1_i32..=1)
            .map(|offset| TilePos::new(step_in_direction(center.coord, axis, offset), center.level))
            .collect::<BTreeSet<_>>()
    };
    let mut states = BTreeMap::<Vec<usize>, RowResolution>::new();
    for (index, center) in centerline.iter().copied().enumerate() {
        let direction = forward_path_direction(&coords, index);
        let longitudinal_axis = direction % 3;
        let canonical_axis = (direction + 2) % 3;
        let mut next_states = BTreeMap::<Vec<usize>, RowResolution>::new();
        for axis in (0..3).filter(|axis| *axis != longitudinal_axis) {
            let row = candidate_row(center, axis);
            if row.len() != 3
                || row
                    .iter()
                    .any(|position| !footprint.contains(&position.coord))
            {
                continue;
            }
            if index == 0 {
                let resolution = RowResolution {
                    cost: RowResolutionCost {
                        noncanonical_rows: usize::from(axis != canonical_axis),
                        orientation_changes: 0,
                        axes: vec![axis],
                    },
                    rows: vec![row],
                };
                next_states.insert(vec![axis], resolution);
                continue;
            }
            for state in states.values() {
                let row_count = state.rows.len();
                if state
                    .rows
                    .iter()
                    .take(row_count.saturating_sub(2))
                    .any(|prior| {
                        prior.iter().any(|position| {
                            row.iter()
                                .any(|candidate| candidate.coord == position.coord)
                        })
                    })
                {
                    continue;
                }
                if row_count >= 2
                    && !three_lane_rows_preserve_exact_progression(
                        &state.rows[row_count - 2],
                        &state.rows[row_count - 1],
                        Some(&row),
                    )
                {
                    continue;
                }
                let previous_axis = state.cost.axes.last().copied().unwrap_or(axis);
                let mut resolution = state.clone();
                resolution.cost.noncanonical_rows = resolution
                    .cost
                    .noncanonical_rows
                    .saturating_add(usize::from(axis != canonical_axis));
                resolution.cost.orientation_changes = resolution
                    .cost
                    .orientation_changes
                    .saturating_add(usize::from(axis != previous_axis));
                resolution.cost.axes.push(axis);
                resolution.rows.push(row.clone());
                let key = resolution
                    .cost
                    .axes
                    .iter()
                    .rev()
                    .take(2)
                    .copied()
                    .collect::<Vec<_>>();
                if next_states
                    .get(&key)
                    .is_none_or(|current| resolution.cost < current.cost)
                {
                    next_states.insert(key, resolution);
                }
            }
        }
        states = next_states;
    }
    let rows = states
        .into_values()
        .filter(|state| {
            let count = state.rows.len();
            count >= 2
                && three_lane_rows_preserve_exact_progression(
                    &state.rows[count - 2],
                    &state.rows[count - 1],
                    None,
                )
        })
        .min_by_key(|state| state.cost.clone())
        .map(|state| state.rows);
    let Some(rows) = rows else {
        return Err(schematic_contract(format!(
            "{label} cannot resolve coherent transverse three-lane rows"
        )));
    };
    Ok(rows)
}

/// Proves that the finalized cells in `source` can reach the finalized cells
/// in `target` without skipping a longitudinal row.
///
/// A sharp hex-grid turn can make `later` reclaim one corner shared with
/// `source`. The remaining source cells may converge laterally inside their
/// three-cell row before advancing. This is the discrete equivalent of water
/// rounding the inside of a constant-width bend; it keeps the ribbon exactly
/// three cells wide instead of adding a fourth corner cell.
fn three_lane_rows_preserve_exact_progression(
    source: &BTreeSet<TilePos>,
    target: &BTreeSet<TilePos>,
    later: Option<&BTreeSet<TilePos>>,
) -> bool {
    let target_coords = target
        .iter()
        .map(|position| position.coord)
        .filter(|coord| later.is_none_or(|row| row.iter().all(|position| position.coord != *coord)))
        .collect::<BTreeSet<_>>();
    let source_coords = source
        .iter()
        .map(|position| position.coord)
        .filter(|coord| {
            target.iter().all(|position| position.coord != *coord)
                && later.is_none_or(|row| row.iter().all(|position| position.coord != *coord))
        })
        .collect::<BTreeSet<_>>();
    if source_coords.is_empty() || target_coords.is_empty() {
        return false;
    }
    let mut reached = source_coords
        .iter()
        .copied()
        .filter(|source| {
            source
                .neighbors()
                .into_iter()
                .any(|neighbor| target_coords.contains(&neighbor))
        })
        .collect::<BTreeSet<_>>();
    let mut frontier = reached.iter().copied().collect::<VecDeque<_>>();
    while let Some(current) = frontier.pop_front() {
        for neighbor in current.neighbors() {
            if source_coords.contains(&neighbor) && reached.insert(neighbor) {
                frontier.push_back(neighbor);
            }
        }
    }
    reached == source_coords
}

fn authoritative_outlet_reservation(
    plan: &SchematicPlanV1,
    profile: V3GrandV3BasicTerrainProfile,
    seed: u64,
    layout: &ResolvedLayoutPlan,
) -> Result<BTreeSet<HexCoord>, V3GenerationError> {
    let (_, river) = authoritative_hydrology_centerlines(plan, profile, seed, layout)?;
    let rows = build_three_lane_rows(&river, &layout.footprint, "hydrology outlet reservation")?;
    let mut reservation = BTreeSet::new();
    for position in rows.iter().rev().take(23).flatten() {
        reservation.insert(position.coord);
        reservation.extend(position.coord.neighbors());
    }
    Ok(reservation)
}

fn semantic_sea_coords(plan: &SchematicPlanV1, layout: &ResolvedLayoutPlan) -> BTreeSet<HexCoord> {
    plan.cells
        .iter()
        .filter(|cell| {
            cell.facts.surface == SurfaceKind::OpenWater
                && !has_overlay(cell, SchematicFeature::MountainLake)
                && !has_overlay(cell, SchematicFeature::ValleyLake)
        })
        .flat_map(|cell| {
            layout
                .patches
                .get(&PatchId(u32::from(cell.id.get())))
                .into_iter()
                .flat_map(|patch| patch.mask.iter().copied())
        })
        .collect()
}

fn replace_column_surface(
    volume: &mut VolumePlan,
    biome_regions: &mut BTreeMap<TilePos, hex_core::BiomeRegionId>,
    coord: HexCoord,
    column: VolumeColumn,
    surface: TilePos,
    metadata: SurfaceMetadata,
    biome: hex_core::BiomeRegionId,
) {
    remove_column_surfaces(volume, biome_regions, coord);
    volume.columns.insert(coord, column);
    volume.surfaces.insert(surface, metadata);
    biome_regions.insert(surface, biome);
}

/// Removes one column's exact surface projection and its matching biome facts.
///
/// Both maps share `TilePos` ordering, so the bounded ranges retain stacked
/// surfaces everywhere else while also cleaning any stale same-column biome
/// entry left by a rejected or partially constructed candidate.
fn remove_column_surfaces(
    volume: &mut VolumePlan,
    biome_regions: &mut BTreeMap<TilePos, hex_core::BiomeRegionId>,
    coord: HexCoord,
) {
    volume.remove_surfaces_at_coord(coord);
    let stale_biomes = biome_regions
        .range(TilePos::new(coord, Level::MIN)..=TilePos::new(coord, Level::MAX))
        .map(|(position, _)| *position)
        .collect::<Vec<_>>();
    for position in stale_biomes {
        let _removed_biome = biome_regions.remove(&position);
    }
}

fn enforce_recessed_water_banks(
    volume: &mut VolumePlan,
    biome_regions: &mut BTreeMap<TilePos, hex_core::BiomeRegionId>,
    fine_index: &FineWorldIndex,
) {
    let water = volume
        .fill_runs_by_top()
        .keys()
        .map(|position| (position.coord, position.level))
        .collect::<BTreeMap<_, _>>();
    let mut required_banks = BTreeMap::<HexCoord, Level>::new();
    for (coord, water_level) in &water {
        for neighbor in coord.neighbors() {
            if water.contains_key(&neighbor) || !volume.mask.contains(&neighbor) {
                continue;
            }
            required_banks
                .entry(neighbor)
                .and_modify(|required| *required = (*required).max(water_level.saturating_add(1)))
                .or_insert_with(|| water_level.saturating_add(1));
        }
    }
    for (coord, required_bank) in required_banks {
        let Some((old_surface, old_metadata)) = volume.top_surface_at_coord(coord) else {
            continue;
        };
        if old_surface.level >= required_bank {
            continue;
        }
        let biome = biome_regions
            .get(&old_surface)
            .copied()
            .or_else(|| fine_index.biome(coord))
            .unwrap_or_default();
        let cap = volume
            .columns
            .get(&coord)
            .map(top_solid_material)
            .unwrap_or(SolidMaterialRole::Grass);
        replace_column_surface(
            volume,
            biome_regions,
            coord,
            land_column(required_bank, cap),
            TilePos::new(coord, required_bank),
            old_metadata,
            biome,
        );
    }
}

fn recessed_water_bank_minimums(volume: &VolumePlan) -> BTreeMap<HexCoord, Level> {
    let water = volume
        .fill_runs_by_top()
        .keys()
        .map(|position| (position.coord, position.level))
        .collect::<BTreeMap<_, _>>();
    water
        .iter()
        .flat_map(|(coord, water_level)| {
            coord
                .neighbors()
                .into_iter()
                .map(move |neighbor| (neighbor, water_level.saturating_add(1)))
        })
        .filter(|(coord, _)| volume.mask.contains(coord) && !water.contains_key(coord))
        .fold(
            BTreeMap::<HexCoord, Level>::new(),
            |mut minimums, (coord, minimum)| {
                minimums
                    .entry(coord)
                    .and_modify(|current| *current = (*current).max(minimum))
                    .or_insert(minimum);
                minimums
            },
        )
}

fn apply_directed_watercourse(
    rows: &[BTreeSet<TilePos>],
    outlet: &OutletAuthority,
    fill_runs: &BTreeMap<TilePos, NonSolidFill>,
    nodes: &mut BTreeMap<TilePos, LiquidNode>,
) -> Result<(), V3GenerationError> {
    let mut positions = BTreeMap::<HexCoord, TilePos>::new();
    let mut ranks = BTreeMap::<HexCoord, usize>::new();
    for (index, row) in rows.iter().enumerate() {
        for position in row {
            if positions
                .insert(position.coord, *position)
                .is_some_and(|previous| previous != *position)
            {
                return Err(schematic_contract(format!(
                    "hydrology bend {:?} claims incompatible water levels",
                    position.coord
                )));
            }
            ranks.insert(position.coord, index);
        }
    }
    let final_row = rows
        .last()
        .ok_or_else(|| schematic_contract("directed hydrology has no rows"))?;
    let mut distances = final_row
        .iter()
        .map(|position| (*position, 0_usize))
        .collect::<BTreeMap<_, _>>();
    let mut frontier = final_row.iter().copied().collect::<VecDeque<_>>();
    while let Some(downstream) = frontier.pop_front() {
        let distance = distances[&downstream];
        for upstream in downstream
            .coord
            .neighbors()
            .into_iter()
            .filter_map(|coord| positions.get(&coord).copied())
            .filter(|upstream| {
                permitted_hydrology_edge(*upstream, downstream, outlet)
                    && canonical_hydrology_successor_state(*upstream, downstream, &ranks, fill_runs)
                        .is_some()
            })
        {
            if distances.contains_key(&upstream) {
                continue;
            }
            distances.insert(upstream, distance.saturating_add(1));
            frontier.push_back(upstream);
        }
    }
    if distances.len() != positions.len() {
        return Err(schematic_contract(format!(
            "three-lane hydrology has {} cells with no downstream route through a bend",
            positions.len().saturating_sub(distances.len())
        )));
    }
    for source in positions.values().copied() {
        if final_row.contains(&source) {
            continue;
        }
        let source_distance = distances[&source];
        let downstream = if let Some(forced) = outlet.edges.get(&source).copied() {
            forced
        } else {
            source
                .coord
                .neighbors()
                .into_iter()
                .filter_map(|coord| positions.get(&coord).copied())
                .filter(|target| {
                    distances[target] < source_distance
                        && permitted_hydrology_edge(source, *target, outlet)
                        && canonical_hydrology_successor_state(source, *target, &ranks, fill_runs)
                            .is_some()
                })
                .min_by_key(|target| (distances[target], Reverse(ranks[&target.coord]), *target))
                .ok_or_else(|| {
                    schematic_contract(format!(
                        "hydrology lane source {source:?} has no downstream bend successor"
                    ))
                })?
        };
        let Some(state) =
            canonical_hydrology_successor_state(source, downstream, &ranks, fill_runs)
        else {
            return Err(schematic_contract(format!(
                "hydrology lane edge {source:?} -> {downstream:?} skips its exact intermediate row or violates fall continuity"
            )));
        };
        if !permitted_hydrology_edge(source, downstream, outlet) {
            return Err(schematic_contract(format!(
                "hydrology lane edge {source:?} -> {downstream:?} violates the exact outlet transition"
            )));
        }
        if !nodes.contains_key(&downstream) {
            return Err(schematic_contract(format!(
                "hydrology lane sink {downstream:?} has no authored fill"
            )));
        }
        let Some(node) = nodes.get_mut(&source) else {
            return Err(schematic_contract(format!(
                "hydrology lane source {source:?} has no authored fill"
            )));
        };
        node.state = state;
        node.downstream = Some(downstream);
    }
    Ok(())
}

/// Resolves the only legal flow state for one authored ribbon edge.
///
/// Width rows can overlap at bends, so a coordinate's final water height and
/// progression rank belong to its latest claim. A level-equal edge within one
/// row lets the outside lane round a concave bend before advancing. Every other
/// edge must enter the next exact rank, keeping a lowered overlap from becoming
/// a shortcut around the intermediate waterfall row. Falls additionally require
/// the source fill to extend to one level above the downstream surface, matching
/// the canonical liquid contract.
fn canonical_hydrology_successor_state(
    source: TilePos,
    target: TilePos,
    ranks: &BTreeMap<HexCoord, usize>,
    fill_runs: &BTreeMap<TilePos, NonSolidFill>,
) -> Option<LiquidFlowState> {
    if source.coord.distance(target.coord) != 1 || target.level > source.level {
        return None;
    }
    let source_rank = ranks.get(&source.coord).copied()?;
    let target_rank = ranks.get(&target.coord).copied()?;
    if target_rank == source_rank {
        return (target.level == source.level).then_some(LiquidFlowState::Current);
    }
    if target_rank != source_rank.saturating_add(1) {
        return None;
    }
    let fill = fill_runs.get(&source)?;
    let drop = source.level.saturating_sub(target.level);
    if drop >= 2 {
        (fill.levels.bottom <= target.level.saturating_add(1)).then_some(LiquidFlowState::Fall)
    } else if drop == 1 {
        Some(LiquidFlowState::Rapid)
    } else {
        Some(LiquidFlowState::Current)
    }
}

fn permitted_hydrology_edge(source: TilePos, target: TilePos, outlet: &OutletAuthority) -> bool {
    match outlet.edges.get(&source) {
        Some(forced) => *forced == target,
        None => {
            outlet.downstream_course.contains(&source) == outlet.downstream_course.contains(&target)
        }
    }
}

fn exact_outlet_authority(
    river_centerline: &[TilePos],
    river_rows: &[BTreeSet<TilePos>],
    semantic_sea: &BTreeSet<HexCoord>,
) -> Result<OutletAuthority, V3GenerationError> {
    if river_centerline.len() != river_rows.len() {
        return Err(schematic_contract(
            "river centerline and three-lane rows disagree at the outlet",
        ));
    }
    let candidates = river_centerline
        .windows(2)
        .enumerate()
        .filter(|(_, pair)| {
            !semantic_sea.contains(&pair[0].coord) && semantic_sea.contains(&pair[1].coord)
        })
        .filter_map(|(index, _)| {
            three_lane_matching(&river_rows[index], &river_rows[index + 1])
                .map(|matching| (index, matching))
        })
        .collect::<Vec<_>>();
    let [(index, matching)] = candidates.as_slice() else {
        return Err(schematic_contract(format!(
            "river requires one exact three-wide transition at its semantic sea boundary, found {}",
            candidates.len()
        )));
    };
    let downstream_course = river_rows[index.saturating_add(1)..]
        .iter()
        .flatten()
        .copied()
        .collect();
    Ok(OutletAuthority {
        edges: matching.iter().copied().collect(),
        downstream_course,
    })
}

fn three_lane_matching(
    sources: &BTreeSet<TilePos>,
    targets: &BTreeSet<TilePos>,
) -> Option<Vec<(TilePos, TilePos)>> {
    const PERMUTATIONS: [[usize; 3]; 6] = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let sources = sources.iter().copied().collect::<Vec<_>>();
    let targets = targets.iter().copied().collect::<Vec<_>>();
    if sources.len() != 3 || targets.len() != 3 {
        return None;
    }
    PERMUTATIONS
        .into_iter()
        .filter_map(|permutation| {
            let matching = sources
                .iter()
                .copied()
                .enumerate()
                .map(|(index, source)| (source, targets[permutation[index]]))
                .collect::<Vec<_>>();
            matching
                .iter()
                .all(|(source, target)| {
                    source.coord.distance(target.coord) == 1 && target.level <= source.level
                })
                .then_some(matching)
        })
        .min_by_key(|matching| {
            matching
                .iter()
                .map(|(_, target)| *target)
                .collect::<Vec<_>>()
        })
}

fn exact_three_lane_outlet(
    nodes: &BTreeMap<TilePos, LiquidNode>,
    authority: &OutletAuthority,
) -> Result<BTreeSet<TilePos>, V3GenerationError> {
    let actual = authority
        .edges
        .keys()
        .filter_map(|source| {
            nodes
                .get(source)?
                .downstream
                .map(|target| (*source, target))
        })
        .collect::<BTreeMap<_, _>>();
    let outlet = actual.values().copied().collect::<BTreeSet<_>>();
    if authority.edges.len() != 3 || actual != authority.edges || outlet.len() != 3 {
        return Err(schematic_contract(format!(
            "authoritative river requires exactly one three-wide sea outlet, found {} exact edges into {} targets",
            actual.len(),
            outlet.len()
        )));
    }
    Ok(outlet)
}

fn liquid_components_with_flow(nodes: BTreeMap<TilePos, LiquidNode>) -> LiquidPlan {
    let mut adjacency = nodes
        .keys()
        .copied()
        .map(|position| (position, BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    for (position, node) in &nodes {
        for neighbor in position
            .coord
            .neighbors()
            .map(|coord| TilePos::new(coord, position.level))
        {
            if nodes.contains_key(&neighbor) {
                adjacency.entry(*position).or_default().insert(neighbor);
                adjacency.entry(neighbor).or_default().insert(*position);
            }
        }
        if let Some(downstream) = node.downstream.filter(|target| nodes.contains_key(target)) {
            adjacency.entry(*position).or_default().insert(downstream);
            adjacency.entry(downstream).or_default().insert(*position);
        }
    }

    let mut remaining = nodes.keys().copied().collect::<BTreeSet<_>>();
    let mut bodies = BTreeMap::new();
    let mut ordinal = 0_u32;
    while let Some(start) = remaining.first().copied() {
        remaining.remove(&start);
        let mut component = BTreeSet::from([start]);
        let mut frontier = VecDeque::from([start]);
        while let Some(position) = frontier.pop_front() {
            for neighbor in adjacency.get(&position).into_iter().flatten() {
                if remaining.remove(neighbor) {
                    component.insert(*neighbor);
                    frontier.push_back(*neighbor);
                }
            }
        }
        let component_nodes = component
            .into_iter()
            .filter_map(|position| nodes.get(&position).copied().map(|node| (position, node)))
            .collect();
        bodies.insert(
            LiquidBodyId(HYDROLOGY_BODY_BASE.saturating_add(ordinal)),
            LiquidBodyPlan {
                material: FillMaterialRole::Water,
                nodes: component_nodes,
            },
        );
        ordinal = ordinal.saturating_add(1);
    }
    LiquidPlan { bodies }
}

fn compile_river_bridges(
    plan: &SchematicPlanV1,
    river: &[TilePos],
    river_rows: &[BTreeSet<TilePos>],
    water_coords: &BTreeSet<HexCoord>,
    layout: &ResolvedLayoutPlan,
    fine_index: &FineWorldIndex,
    volume: &mut VolumePlan,
    biome_regions: &mut BTreeMap<TilePos, hex_core::BiomeRegionId>,
) -> Result<BridgeCompilation, V3GenerationError> {
    if river.len() != river_rows.len() {
        return Err(schematic_contract(
            "river centerline and exact three-lane rows disagree",
        ));
    }
    let semantic_sea = semantic_sea_coords(plan, layout);
    let sea_entry = river_rows
        .iter()
        .position(|row| {
            row.iter()
                .all(|position| semantic_sea.contains(&position.coord))
        })
        .ok_or_else(|| schematic_contract("river never reaches its semantic sea sink"))?;
    let water_levels = volume
        .fill_runs_by_top()
        .keys()
        .map(|position| (position.coord, position.level))
        .collect::<BTreeMap<_, _>>();
    let candidates = (1..river_rows.len().saturating_sub(1))
        .filter_map(|index| {
            let (deck, water_deck) = exact_bridge_deck(
                river[index],
                &river_rows[index],
                river[index + 1],
                &river_rows[index + 1],
            )?;
            (index.saturating_add(1) < sea_entry
                && deck
                    .iter()
                    .all(|position| layout.footprint.contains(&position.coord))
                && deck.iter().all(|position| {
                    water_coords.contains(&position.coord) == water_deck.contains(position)
                })
                && bridge_bank_shoulders_clear_adjacent_water(&deck, &water_deck, &water_levels))
            .then_some((index, deck, water_deck))
        })
        .collect::<Vec<_>>();
    let midpoint = sea_entry / 2;
    let valley_requested = sea_entry / 4;
    let coastal_requested = sea_entry.saturating_sub(8);
    let valley = candidates
        .iter()
        .filter(|(index, ..)| *index < midpoint)
        .min_by_key(|(index, deck, ..)| (index.abs_diff(valley_requested), deck.clone()))
        .cloned()
        .ok_or_else(|| schematic_contract("river has no simple bridge site below valley lake"))?;
    let coastal = candidates
        .iter()
        .filter(|(index, ..)| *index >= midpoint)
        .min_by_key(|(index, deck, ..)| (index.abs_diff(coastal_requested), deck.clone()))
        .cloned()
        .ok_or_else(|| schematic_contract("river has no simple coastal bridge site"))?;
    if valley.0.abs_diff(coastal.0) < 4 || !valley.1.is_disjoint(&coastal.1) {
        return Err(schematic_contract(
            "valley and coastal bridge crossings are not distinct",
        ));
    }

    let mut structures = BTreeMap::new();
    let mut crossings = Vec::with_capacity(2);
    for (ordinal, (index, deck, water_deck)) in [valley, coastal].into_iter().enumerate() {
        for surface in &deck {
            let coord = surface.coord;
            let biome = fine_index
                .biome(coord)
                .ok_or_else(|| schematic_contract("bridge deck has no biome owner"))?;
            if water_deck.contains(surface) {
                let column = volume.columns.get_mut(&coord).ok_or_else(|| {
                    schematic_contract("bridge water column is missing from volume")
                })?;
                column.elements.push(VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(surface.level, surface.level.saturating_add(1)),
                    material: SolidMaterialRole::WorkedStone,
                    cutaway_for: None,
                }));
                volume.surfaces.insert(
                    *surface,
                    SurfaceMetadata {
                        access: SurfaceAccess::Ordinary,
                        interior: None,
                    },
                );
                biome_regions.insert(*surface, biome);
            } else {
                let (_, old_metadata) = volume
                    .top_surface_at_coord(coord)
                    .ok_or_else(|| schematic_contract("bridge bank has no surface"))?;
                replace_column_surface(
                    volume,
                    biome_regions,
                    coord,
                    land_column(surface.level, SolidMaterialRole::WorkedStone),
                    *surface,
                    SurfaceMetadata {
                        access: SurfaceAccess::Ordinary,
                        interior: old_metadata.interior,
                    },
                    biome,
                );
            }
        }
        let structure = StructureId(
            BRIDGE_STRUCTURE_BASE.saturating_add(u32::try_from(ordinal).unwrap_or(u32::MAX)),
        );
        structures.insert(
            structure,
            PlannedStructure {
                kind: StructureKind::Bridge,
                voxels: deck.clone(),
            },
        );
        crossings.push(BridgeAuthority {
            structure,
            river_row_indices: [index, index + 1],
            deck,
            water_deck,
        });
    }
    Ok(BridgeCompilation {
        structures: StructurePlan { by_id: structures },
        crossings,
    })
}

/// Rejects a prospective bridge before its worked-stone shoulders replace the
/// already-recessed natural banks.
///
/// A bridge deck is one level above the two crossed river rows. On a descending
/// bend, however, a dry shoulder can also border a different, higher upstream
/// row. Building that candidate would leave the shoulder coplanar with water.
/// Later ordinary-route construction correctly treats structure coordinates as
/// immutable, so it cannot repair that authored mistake. Candidate selection
/// therefore proves every dry deck coordinate against *all* adjacent water,
/// while the six over-water deck voxels remain stacked surfaces rather than
/// banks.
fn bridge_bank_shoulders_clear_adjacent_water(
    deck: &BTreeSet<TilePos>,
    water_deck: &BTreeSet<TilePos>,
    water_levels: &BTreeMap<HexCoord, Level>,
) -> bool {
    deck.difference(water_deck).all(|bank| {
        bank.coord.neighbors().into_iter().all(|neighbor| {
            water_levels
                .get(&neighbor)
                .is_none_or(|water_level| bank.level >= water_level.saturating_add(1))
        })
    })
}

fn exact_bridge_deck(
    first_center: TilePos,
    first_row: &BTreeSet<TilePos>,
    second_center: TilePos,
    second_row: &BTreeSet<TilePos>,
) -> Option<(BTreeSet<TilePos>, BTreeSet<TilePos>)> {
    if first_center.coord.distance(second_center.coord) != 1
        || first_center.level != second_center.level
    {
        return None;
    }
    let first_axis = exact_row_axis(first_center, first_row)?;
    let second_axis = exact_row_axis(second_center, second_row)?;
    if first_axis != second_axis {
        return None;
    }
    let deck_level = first_center.level.saturating_add(1);
    let mut deck = BTreeSet::new();
    for center in [first_center, second_center] {
        deck.extend((-2_i32..=2).map(|offset| {
            TilePos::new(
                step_in_direction(center.coord, first_axis, offset),
                deck_level,
            )
        }));
    }
    let water_deck = first_row
        .iter()
        .chain(second_row)
        .map(|position| TilePos::new(position.coord, deck_level))
        .collect::<BTreeSet<_>>();
    (deck.len() == 10 && water_deck.len() == 6 && water_deck.is_subset(&deck))
        .then_some((deck, water_deck))
}

/// Resolves two independent dry landing contacts on each side of every exact
/// two-wide bridge.
///
/// Bridge voxels are immutable structure authority, so the ordinary connector
/// pass may never grade them after publication. Mutable cliff-side landing
/// columns are resolved at the shoulder level here and graded constructively
/// before that graph is built. Water-bank, blocker, or other authored conflicts
/// remain immutable and therefore reject the bridge fail closed.
fn bridge_bank_approaches(
    bridges: &BridgeCompilation,
    volume: &VolumePlan,
    blockers: Option<&BTreeSet<TilePos>>,
    ordinary_mask: &BTreeSet<HexCoord>,
    non_bank_forbidden: &BTreeSet<HexCoord>,
    immutable_water_banks: &BTreeSet<HexCoord>,
    surface_by_coord: &BTreeMap<HexCoord, TilePos>,
) -> Result<Vec<BridgeBankApproach>, V3GenerationError> {
    let mut approaches = Vec::with_capacity(bridges.crossings.len().saturating_mul(4));
    for bridge in &bridges.crossings {
        if bridge.deck.len() != 10
            || bridge.water_deck.len() != 6
            || !bridge.water_deck.is_subset(&bridge.deck)
            || bridge
                .deck
                .iter()
                .any(|surface| !ordinary_surface_is_node(volume, blockers, *surface))
        {
            return Err(schematic_contract(format!(
                "bridge {:?} has no exact ten-voxel Ordinary deck",
                bridge.structure
            )));
        }

        let mut deck_reached = BTreeSet::new();
        let mut deck_frontier = VecDeque::from([bridge
            .deck
            .first()
            .copied()
            .ok_or_else(|| schematic_contract("bridge deck is empty"))?]);
        while let Some(surface) = deck_frontier.pop_front() {
            if !deck_reached.insert(surface) {
                continue;
            }
            deck_frontier.extend(bridge.deck.iter().copied().filter(|neighbor| {
                ordinary_transition_is_admitted(volume, blockers, surface, *neighbor)
            }));
        }
        if deck_reached != bridge.deck {
            return Err(schematic_contract(format!(
                "bridge {:?} is not one internally walkable deck",
                bridge.structure
            )));
        }

        let mut remaining = bridge
            .deck
            .difference(&bridge.water_deck)
            .copied()
            .collect::<BTreeSet<_>>();
        let mut banks = Vec::<BTreeSet<TilePos>>::new();
        while let Some(start) = remaining.first().copied() {
            let mut bank = BTreeSet::new();
            let mut frontier = VecDeque::from([start]);
            while let Some(surface) = frontier.pop_front() {
                if !remaining.remove(&surface) || !bank.insert(surface) {
                    continue;
                }
                frontier.extend(
                    remaining
                        .iter()
                        .copied()
                        .filter(|candidate| surface.coord.distance(candidate.coord) == 1),
                );
            }
            banks.push(bank);
        }
        banks.sort_unstable_by_key(|bank| bank.first().copied());
        if banks.len() != 2 || banks.iter().any(|bank| bank.len() != 2) {
            return Err(schematic_contract(format!(
                "bridge {:?} does not expose two exact two-wide dry banks",
                bridge.structure
            )));
        }

        for (bank_index, bank) in banks.into_iter().enumerate() {
            let shoulders = bank.iter().copied().collect::<Vec<_>>();
            let candidates =
                shoulders
                    .iter()
                    .map(|shoulder| {
                        shoulder
                            .coord
                            .neighbors()
                            .into_iter()
                            .filter(|coord| {
                                ordinary_mask.contains(coord)
                                    && !bridge.deck.iter().any(|surface| surface.coord == *coord)
                            })
                            .filter_map(|coord| {
                                let existing = surface_by_coord.get(&coord).copied()?;
                                if volume.surfaces.get(&existing).is_none_or(|metadata| {
                                    metadata.access != SurfaceAccess::Ordinary
                                }) {
                                    return None;
                                }
                                let admitted = ordinary_transition_is_admitted(
                                    volume, blockers, *shoulder, existing,
                                );
                                // An immutable water bank may be reused only at
                                // its already-published level. It must not
                                // launder an independent route, structure,
                                // interior, blocker, anchor, or other authored
                                // reservation which happens to share the same
                                // coordinate.
                                if non_bank_forbidden.contains(&coord)
                                    || (immutable_water_banks.contains(&coord) && !admitted)
                                {
                                    return None;
                                }
                                let required = if admitted {
                                    existing
                                } else {
                                    TilePos::new(coord, shoulder.level)
                                };
                                Some((
                                    (
                                        u8::from(!admitted),
                                        existing.level.abs_diff(shoulder.level),
                                        required,
                                    ),
                                    required,
                                ))
                            })
                            .collect::<BTreeMap<_, _>>()
                    })
                    .collect::<Vec<_>>();
            let [first_candidates, second_candidates] = candidates.as_slice() else {
                return Err(schematic_contract(format!(
                    "bridge {:?} bank {bank_index} does not contain exactly two shoulders",
                    bridge.structure
                )));
            };
            let matching = first_candidates.values().copied().find_map(|first| {
                second_candidates
                    .values()
                    .copied()
                    .find(|second| *second != first)
                    .map(|second| [first, second])
            });
            let Some(matching) = matching else {
                return Err(schematic_contract(format!(
                    "bridge {:?} bank {bank_index} has no two independent dry walker approaches",
                    bridge.structure
                )));
            };
            approaches.extend(
                matching
                    .into_iter()
                    .enumerate()
                    .map(|(lane_index, surface)| BridgeBankApproach {
                        structure: bridge.structure,
                        bank_index,
                        lane_index,
                        surface,
                    }),
            );
        }
    }
    Ok(approaches)
}

/// Seals every pregraded bridge-bank approach as exact surface authority before
/// any sibling connector is carved.
///
/// Required bridge connectors are solved sequentially. Without this shared
/// reservation, the first lane may legitimately choose a later lane's column
/// as mutable terrain and grade it away from the exact deck height before that
/// lane is processed. The authority is coordinate-unique and fail-closed: all
/// planned contacts must still be the selected exposed surfaces.
fn exact_bridge_approach_authority(
    approaches: &[BridgeBankApproach],
    surface_by_coord: &BTreeMap<HexCoord, TilePos>,
) -> Result<BTreeMap<HexCoord, TilePos>, V3GenerationError> {
    let authority = approaches
        .iter()
        .map(|approach| (approach.surface.coord, approach.surface))
        .collect::<BTreeMap<_, _>>();
    if authority.len() != approaches.len() {
        return Err(schematic_contract(
            "bridge banks do not own distinct dry approach columns",
        ));
    }
    if let Some((coord, expected)) = authority.iter().find_map(|(coord, expected)| {
        (surface_by_coord.get(coord).copied() != Some(*expected)).then_some((*coord, *expected))
    }) {
        return Err(schematic_contract(format!(
            "pregraded bridge approach {expected:?} is not the selected surface at {coord:?}"
        )));
    }
    Ok(authority)
}

fn exact_row_axis(center: TilePos, row: &BTreeSet<TilePos>) -> Option<usize> {
    (0..3).find(|axis| {
        (-1_i32..=1)
            .map(|offset| {
                TilePos::new(step_in_direction(center.coord, *axis, offset), center.level)
            })
            .collect::<BTreeSet<_>>()
            == *row
    })
}

fn validate_grand_hydrology(
    world: &GeneratedWorldPlan,
    plan: &SchematicPlanV1,
    profile: V3GrandV3BasicTerrainProfile,
    hydrology: &HydrologyCompilation,
    bridges: &BridgeCompilation,
) -> Result<(), V3GenerationError> {
    if hydrology.watercourse_rows.is_empty()
        || hydrology.river_rows.is_empty()
        || hydrology.watercourse_rows.iter().any(|row| row.len() != 3)
    {
        return Err(schematic_contract(
            "authoritative hydrology is not an exact three-lane ribbon",
        ));
    }
    if hydrology
        .waterfall_centerline
        .first()
        .is_none_or(|position| position.level != profile.mountain_lake_level)
        || hydrology
            .waterfall_centerline
            .last()
            .is_none_or(|position| position.level != profile.valley_lake_level)
        || hydrology
            .river_centerline
            .first()
            .is_none_or(|position| position.level != profile.valley_lake_level)
        || hydrology
            .river_centerline
            .last()
            .is_none_or(|position| position.level != profile.sea_level)
    {
        return Err(schematic_contract(
            "authoritative hydrology lost its 150-to-15-to-8 level pins",
        ));
    }
    validate_plunge_profile(
        &hydrology.waterfall_centerline,
        profile.mountain_lake_level,
        profile.valley_lake_level,
    )?;
    validate_river_meander(plan, &hydrology.river_centerline)?;

    let mut node_owners = BTreeMap::<TilePos, (LiquidBodyId, LiquidNode)>::new();
    for (body_id, body) in &world.liquids.bodies {
        for (position, node) in &body.nodes {
            if node_owners.insert(*position, (*body_id, *node)).is_some() {
                return Err(schematic_contract(format!(
                    "hydrology node {position:?} belongs to more than one liquid body"
                )));
            }
        }
    }
    let course = hydrology
        .watercourse_rows
        .iter()
        .flatten()
        .copied()
        .collect::<BTreeSet<_>>();
    let body_ids = course
        .iter()
        .filter_map(|position| node_owners.get(position).map(|(body, _)| *body))
        .collect::<BTreeSet<_>>();
    if body_ids.len() != 1
        || course
            .iter()
            .any(|position| !node_owners.contains_key(position))
    {
        return Err(schematic_contract(
            "mountain lake, waterfall, valley lake, and river are not one liquid body",
        ));
    }
    let final_row = hydrology
        .watercourse_rows
        .last()
        .ok_or_else(|| schematic_contract("authoritative hydrology has no sea sink row"))?;
    let ranks = hydrology
        .watercourse_rows
        .iter()
        .enumerate()
        .flat_map(|(index, row)| row.iter().map(move |position| (position.coord, index)))
        .collect::<BTreeMap<_, _>>();
    let fill_runs = world.volume.fill_runs_by_top();
    for source in course.difference(final_row) {
        let node = node_owners[source].1;
        let Some(downstream) = node.downstream else {
            return Err(schematic_contract(format!(
                "three-lane hydrology stops before the sea at {source:?}"
            )));
        };
        if !course.contains(&downstream)
            || downstream.level > source.level
            || node.state == LiquidFlowState::Still
        {
            return Err(schematic_contract(format!(
                "hydrology lane {source:?} does not move monotonically downstream"
            )));
        }
        let Some(expected_state) =
            canonical_hydrology_successor_state(*source, downstream, &ranks, &fill_runs)
        else {
            return Err(schematic_contract(format!(
                "hydrology lane {source:?} -> {downstream:?} skips its exact intermediate row or violates canonical fall continuity"
            )));
        };
        if node.state != expected_state {
            return Err(schematic_contract(format!(
                "hydrology lane {source:?} -> {downstream:?} uses {:?}, expected {expected_state:?}",
                node.state
            )));
        }
    }
    if final_row.iter().any(|position| {
        let node = node_owners[position].1;
        node.downstream.is_some() || node.state != LiquidFlowState::Still
    }) {
        return Err(schematic_contract(
            "the exact three-wide sea sink is not terminal",
        ));
    }
    for start in &course {
        let mut cursor = *start;
        let mut seen = BTreeSet::new();
        while let Some(downstream) = node_owners[&cursor].1.downstream {
            if !seen.insert(cursor) || !course.contains(&downstream) {
                return Err(schematic_contract(
                    "authoritative hydrology contains a cycle or side leak",
                ));
            }
            cursor = downstream;
        }
        if !final_row.contains(&cursor) {
            return Err(schematic_contract(
                "an authoritative hydrology lane does not terminate at the sea sink",
            ));
        }
    }
    let liquid_nodes = node_owners
        .iter()
        .map(|(position, (_, node))| (*position, *node))
        .collect::<BTreeMap<_, _>>();
    let semantic_sea = semantic_sea_coords(plan, &world.layout);
    let outlet_authority = exact_outlet_authority(
        &hydrology.river_centerline,
        &hydrology.river_rows,
        &semantic_sea,
    )?;
    let outlet = exact_three_lane_outlet(&liquid_nodes, &outlet_authority)?;
    if outlet != hydrology.outlet {
        return Err(schematic_contract(
            "the exact sea outlet changed after hydrology compilation",
        ));
    }

    let bank_violations = water_bank_violations(&world.volume);
    if let Some((water, bank, bank_level)) = bank_violations.first() {
        return Err(schematic_contract(format!(
            "water {water:?} is not at least one voxel below adjacent bank {bank:?} at level {bank_level}"
        )));
    }

    if bridges.crossings.len() != 2
        || bridges.crossings[0].structure == bridges.crossings[1].structure
        || !bridges.crossings[0]
            .deck
            .is_disjoint(&bridges.crossings[1].deck)
        || bridges.crossings[0].river_row_indices[1] >= bridges.crossings[1].river_row_indices[0]
    {
        return Err(schematic_contract(
            "river requires two distinct ordered bridge crossings",
        ));
    }
    let actual_bridge_ids = world
        .structures
        .by_id
        .iter()
        .filter_map(|(id, structure)| (structure.kind == StructureKind::Bridge).then_some(*id))
        .collect::<BTreeSet<_>>();
    let expected_bridge_ids = bridges
        .crossings
        .iter()
        .map(|bridge| bridge.structure)
        .collect::<BTreeSet<_>>();
    if actual_bridge_ids != expected_bridge_ids {
        return Err(schematic_contract(
            "world does not contain exactly the valley and coastal bridges",
        ));
    }
    let mut expected_water_deck = BTreeSet::new();
    for bridge in &bridges.crossings {
        let structure = world
            .structures
            .by_id
            .get(&bridge.structure)
            .ok_or_else(|| schematic_contract("authoritative bridge structure disappeared"))?;
        if bridge.deck.len() != 10
            || bridge.water_deck.len() != 6
            || structure.voxels != bridge.deck
            || bridge.deck.iter().any(|voxel| {
                world
                    .volume
                    .surfaces
                    .get(voxel)
                    .is_none_or(|metadata| metadata.access != SurfaceAccess::Ordinary)
                    || solid_material_at(&world.volume, *voxel)
                        != Some(SolidMaterialRole::WorkedStone)
            })
        {
            return Err(schematic_contract(
                "bridge is not an exact simple two-wide worked-stone crossing",
            ));
        }
        expected_water_deck.extend(bridge.water_deck.iter().copied());
    }
    let river_water = hydrology
        .river_rows
        .iter()
        .flatten()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    let actual_ordinary_water_deck = world
        .volume
        .surfaces
        .iter()
        .filter_map(|(surface, metadata)| {
            (river_water.contains(&surface.coord) && metadata.access == SurfaceAccess::Ordinary)
                .then_some(*surface)
        })
        .collect::<BTreeSet<_>>();
    if actual_ordinary_water_deck != expected_water_deck {
        return Err(schematic_contract(
            "ordinary traversal crosses the river outside the two exact bridges",
        ));
    }
    Ok(())
}

fn water_bank_violations(volume: &VolumePlan) -> Vec<(TilePos, HexCoord, Level)> {
    let water = volume
        .fill_runs_by_top()
        .keys()
        .copied()
        .map(|position| (position.coord, position))
        .collect::<BTreeMap<_, _>>();
    // Bank validation is geometric rather than traversal-specific, so retain
    // the previous "highest surface at this coordinate" meaning while indexing
    // it once. Scanning every world surface for every exposed water edge turns
    // the radius-187 sea into an avoidable O(water-edge × world-surface) pass.
    let bank_levels =
        volume
            .surfaces
            .keys()
            .fold(BTreeMap::<HexCoord, Level>::new(), |mut levels, surface| {
                levels
                    .entry(surface.coord)
                    .and_modify(|level| *level = (*level).max(surface.level))
                    .or_insert(surface.level);
                levels
            });
    let mut violations = Vec::new();
    for (coord, water_position) in &water {
        for bank in coord.neighbors() {
            if water.contains_key(&bank) || !volume.mask.contains(&bank) {
                continue;
            }
            let bank_level = bank_levels.get(&bank).copied().unwrap_or(-1);
            if bank_level < water_position.level.saturating_add(1) {
                violations.push((*water_position, bank, bank_level));
            }
        }
    }
    violations
}

fn solid_material_at(volume: &VolumePlan, voxel: TilePos) -> Option<SolidMaterialRole> {
    volume.columns.get(&voxel.coord).and_then(|column| {
        column.elements.iter().find_map(|element| match element {
            VolumeElement::Solid(mass)
                if mass.levels.bottom <= voxel.level && voxel.level < mass.levels.top =>
            {
                Some(mass.material)
            }
            VolumeElement::Solid(_) | VolumeElement::Fill(_) => None,
        })
    })
}

fn compile_tunnel(
    plan: &SchematicPlanV1,
    profile: V3GrandV3BasicTerrainProfile,
    seed: u64,
    interior_id: InteriorRegionId,
    ascent_target: TilePos,
    lower_terminal: &BTreeSet<TilePos>,
    crystal_mask: &BTreeSet<HexCoord>,
    layout: &ResolvedLayoutPlan,
    fine_index: &FineWorldIndex,
    volume: &mut VolumePlan,
    biome_regions: &mut BTreeMap<TilePos, hex_core::BiomeRegionId>,
) -> Result<TunnelCompilation, V3GenerationError> {
    const CLEARANCE_TOP: Level = 13;
    const ROOF_TOP: Level = 16;
    const GOTHIC_ROWS: usize = 12;
    const APPROACH_LENGTH: usize = 32;
    const MIN_CONCEALED_APPROACH_ROWS: usize = 4;
    const MAX_CONCEALED_APPROACH_ROWS: usize = 8;

    let coarse = schematic_network_path(plan, NetworkKind::Tunnel, "edge/tunnel-complete")?;
    if coarse.len() < 2 {
        return Err(schematic_contract("the tunnel route has no outward cell"));
    }
    if lower_terminal.len() != 4
        || !lower_terminal.contains(&ascent_target)
        || lower_terminal
            .iter()
            .any(|position| position.level != profile.crystal_base_level)
    {
        return Err(schematic_contract(
            "tunnel destination is not the exact four-wide Crystal base terminal",
        ));
    }
    let locked_centerline = fine_network_path(&coarse, 22);
    let ResolvedTunnelLane {
        centerline,
        rows: planned_rows,
        lane_offsets,
    } = resolve_exact_terminal_lane(
        lower_terminal,
        crystal_mask,
        &layout.footprint,
        locked_centerline,
    )
    .ok_or_else(|| {
        schematic_contract(
            "exact Crystal terminal cannot splice into one stable outward tunnel lane frame",
        )
    })?;
    if centerline.len() < GOTHIC_ROWS.saturating_add(2) {
        return Err(schematic_contract(
            "the exact tunnel path is too short for its Gothic transition",
        ));
    }
    if centerline.iter().copied().collect::<BTreeSet<_>>().len() != centerline.len()
        || centerline
            .windows(2)
            .any(|pair| pair[0].distance(pair[1]) != 1)
    {
        return Err(schematic_contract(
            "outward tunnel centerline repeats, reverses, or skips a fine cell",
        ));
    }
    let exact_terminal_coords = lower_terminal
        .iter()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    if planned_rows.first() != Some(&exact_terminal_coords)
        || planned_rows.len() != centerline.len()
    {
        return Err(schematic_contract(
            "resolved tunnel rows do not preserve exact row zero or centerline cardinality",
        ));
    }
    if planned_rows
        .windows(2)
        .any(|pair| !lane_rows_connect_smoothly(&pair[0], &pair[1]))
    {
        return Err(schematic_contract(
            "four-wide tunnel lane rows do not form a smooth continuous ribbon",
        ));
    }
    if planned_rows
        .iter()
        .skip(1)
        .flatten()
        .any(|coord| crystal_mask.contains(coord))
    {
        return Err(schematic_contract(
            "planned outward tunnel rows re-enter the claimed Crystal site",
        ));
    }
    if lower_terminal.iter().any(|floor| {
        volume
            .surfaces
            .get(floor)
            .is_none_or(|metadata| metadata.access != SurfaceAccess::Ordinary)
    }) {
        return Err(schematic_contract(
            "exact Crystal lower terminal is not ordinary footing before tunnel carve",
        ));
    }
    let foot = *centerline
        .last()
        .ok_or_else(|| schematic_contract("the tunnel has no foot terminal"))?;
    let direction = centerline
        .windows(2)
        .last()
        .and_then(|pair| {
            pair[0]
                .neighbors()
                .iter()
                .position(|neighbor| *neighbor == pair[1])
        })
        .ok_or_else(|| schematic_contract("the tunnel terminal direction is malformed"))?;
    // Resolve the roofless approach before selecting interior fixtures. A
    // fixture chosen in this footprint would be erased by the later surface
    // grade, leaving a gameplay light rooted on stale footing.
    let outer = step_in_direction(
        foot,
        direction,
        i32::try_from(APPROACH_LENGTH).unwrap_or(i32::MAX),
    );
    let outer_surface = volume
        .top_surface_at_coord(outer)
        .map(|(position, _)| position)
        .ok_or_else(|| schematic_contract("tunnel foot approach leaves the surface"))?;
    let approach = foot.line_between(outer);
    let approach_rows = approach
        .iter()
        .copied()
        .map(|center| {
            tunnel_approach_lane_offsets(lane_offsets)
                .into_iter()
                .map(move |lane| step_in_direction(center, (direction + 2) % 6, lane))
                .collect::<BTreeSet<_>>()
        })
        .collect::<Vec<_>>();
    if approach_rows.iter().any(|row| row.len() != 4)
        || approach_rows
            .windows(2)
            .any(|rows| !lane_rows_connect_smoothly(&rows[0], &rows[1]))
        || planned_rows.last() != approach_rows.first()
        || approach_rows
            .iter()
            .skip(1)
            .flatten()
            .any(|coord| fine_index.biome(*coord).is_none())
    {
        return Err(schematic_contract(
            "tunnel exterior approach must continue as exact four-wide owned footing",
        ));
    }
    if outer_surface.level < profile.crystal_base_level {
        return Err(schematic_contract(
            "tunnel exterior approach cannot climb down below its level-six interior",
        ));
    }
    let required_grade_transitions = usize::try_from(
        outer_surface
            .level
            .saturating_sub(profile.crystal_base_level),
    )
    .unwrap_or(usize::MAX);
    let maximum_concealed_by_grade = approach
        .len()
        .saturating_sub(1)
        .saturating_sub(required_grade_transitions);
    let concealed_limit = MAX_CONCEALED_APPROACH_ROWS.min(maximum_concealed_by_grade);
    let mut concealed_approach_rows = 0_usize;
    for row in approach_rows.iter().skip(1).take(concealed_limit) {
        if row.iter().all(|coord| {
            volume
                .top_surface_at_coord(*coord)
                .is_some_and(|(surface, _)| surface.level >= ROOF_TOP)
        }) {
            concealed_approach_rows = concealed_approach_rows.saturating_add(1);
        } else {
            break;
        }
    }
    if concealed_approach_rows < MIN_CONCEALED_APPROACH_ROWS {
        return Err(schematic_contract(format!(
            "tunnel foot has only {concealed_approach_rows} concealed approach rows; at least {MIN_CONCEALED_APPROACH_ROWS} are required"
        )));
    }
    let entrance_row = approach_rows[concealed_approach_rows].clone();
    let mouth_index = concealed_approach_rows.saturating_add(1);
    let mouth_center = approach.get(mouth_index).copied().ok_or_else(|| {
        schematic_contract("concealed tunnel approach leaves no exterior mouth row")
    })?;
    let approach_footprint = approach_rows
        .iter()
        .skip(1)
        .flatten()
        .copied()
        .collect::<BTreeSet<_>>();
    let roofed_overburden_coords = planned_rows
        .iter()
        .skip(1)
        .chain(approach_rows.iter().skip(1).take(concealed_approach_rows))
        .flatten()
        .copied()
        .filter(|coord| !crystal_mask.contains(coord))
        .collect::<BTreeSet<_>>();
    let overburden =
        capture_tunnel_overburden_authority(volume, &roofed_overburden_coords, ROOF_TOP)?;

    let mut floors = BTreeSet::new();
    let mut roofs = BTreeSet::new();
    let mut route_centerline = Vec::new();
    // Consecutive four-wide rows intentionally share cells while turning.  A
    // coordinate touched by any of the twelve authored transition rows must
    // therefore remain Gothic even if a later rough-hewn row visits the same
    // cell at the bend.
    let gothic_coords = planned_rows
        .iter()
        .enumerate()
        .filter(|(index, _)| *index > 0 && *index <= GOTHIC_ROWS)
        .flat_map(|(_, row)| row.iter().copied())
        .collect::<BTreeSet<_>>();
    for (index, center) in centerline.iter().copied().enumerate() {
        // Row zero is the exact authored Crystal terminal. The next twelve
        // outside rows are the Gothic transition; every remaining roofed row
        // stays rough-hewn stone.
        let row = planned_rows[index].clone();
        for coord in row {
            let biome = fine_index.biome(coord).ok_or_else(|| {
                schematic_contract(format!("tunnel lane {coord:?} has no biome owner"))
            })?;
            if crystal_mask.contains(&coord) {
                let floor = TilePos::new(coord, profile.crystal_base_level);
                if volume
                    .surfaces
                    .get(&floor)
                    .is_none_or(|metadata| metadata.access != SurfaceAccess::Ordinary)
                {
                    return Err(schematic_contract(format!(
                        "exact Crystal connector is missing ordinary floor {floor:?}"
                    )));
                }
                floors.insert(floor);
                continue;
            }
            let (top_surface, top_metadata) = volume
                .top_surface_at_coord(coord)
                .ok_or_else(|| schematic_contract("tunnel column has no surface"))?;
            let existing_column = volume
                .columns
                .get(&coord)
                .cloned()
                .ok_or_else(|| schematic_contract("tunnel column disappeared before carve"))?;
            let preserved_surface_level = top_surface.level;
            let column = tunnel_column(
                &existing_column,
                preserved_surface_level,
                profile.crystal_base_level,
                CLEARANCE_TOP,
                ROOF_TOP,
                gothic_coords.contains(&coord),
                interior_id,
            );
            remove_column_surfaces(volume, biome_regions, coord);
            volume.columns.insert(coord, column);
            let floor = TilePos::new(coord, profile.crystal_base_level);
            volume.surfaces.insert(
                floor,
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: Some(interior_id),
                },
            );
            biome_regions.insert(floor, biome);
            floors.insert(floor);
            let preserved_surface = TilePos::new(coord, preserved_surface_level);
            volume.surfaces.insert(preserved_surface, top_metadata);
            biome_regions.insert(preserved_surface, biome);
            for level in CLEARANCE_TOP..ROOF_TOP {
                roofs.insert(TilePos::new(coord, level));
            }
        }
        route_centerline.push(TilePos::new(center, profile.crystal_base_level));
    }

    // Nonblocking cave-crystal fixtures occupy small carved side alcoves. Their
    // paired gameplay sources are spaced from the ordered centerline so every
    // four-wide floor remains within Dim-18 while the visible object belongs to
    // the Bright-4 member only.
    let lit_centerline = centerline
        .iter()
        .copied()
        .filter(|coord| !crystal_mask.contains(coord))
        .collect::<Vec<_>>();
    let mut alcove_origins = Vec::new();
    let mut alcove_footprint = BTreeSet::new();
    let route_coords = floors
        .iter()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    let concealed_approach_coords = approach_rows
        .iter()
        .skip(1)
        .take(concealed_approach_rows)
        .flatten()
        .copied()
        .collect::<BTreeSet<_>>();
    let required_interior_route_coords = route_coords
        .union(&concealed_approach_coords)
        .copied()
        .collect::<BTreeSet<_>>();
    let required_interior_floors = required_interior_route_coords
        .iter()
        .copied()
        .map(|coord| TilePos::new(coord, profile.crystal_base_level))
        .collect::<BTreeSet<_>>();
    let required_lit_centerline = lit_centerline
        .iter()
        .copied()
        .chain(
            approach
                .iter()
                .copied()
                .skip(1)
                .take(concealed_approach_rows),
        )
        .collect::<Vec<_>>();
    let fill_coords = volume
        .fill_runs_by_top()
        .keys()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    let select_alcove_origin = |requested: usize,
                                search_centerline: &[HexCoord],
                                adjacent_route_coords: &BTreeSet<HexCoord>,
                                reserved_alcove_footprint: &BTreeSet<HexCoord>,
                                must_cover: Option<HexCoord>| {
        let mut search = (0..search_centerline.len()).collect::<Vec<_>>();
        search.sort_unstable_by_key(|index| (index.abs_diff(requested), *index));
        search.into_iter().find_map(|center_index| {
            let center = *search_centerline.get(center_index)?;
            let mut candidates = center
                .within_radius(4)
                .into_iter()
                .filter(|candidate| {
                    !adjacent_route_coords.contains(candidate)
                        && !crystal_mask.contains(candidate)
                        && !reserved_alcove_footprint.contains(candidate)
                        && !approach_footprint.contains(candidate)
                        && fine_index.by_coord.contains_key(candidate)
                        && must_cover.is_none_or(|floor| {
                            candidate.distance(floor) <= TUNNEL_DIM_LIGHT_RADIUS
                        })
                        && candidate
                            .neighbors()
                            .into_iter()
                            .any(|neighbor| adjacent_route_coords.contains(&neighbor))
                        && tunnel_alcove_candidate_has_complete_roof_support(
                            *candidate,
                            adjacent_route_coords,
                            volume,
                            ROOF_TOP,
                        )
                        && candidate.within_radius(1).into_iter().all(|coord| {
                            layout.footprint.contains(&coord)
                                && !crystal_mask.contains(&coord)
                                && !reserved_alcove_footprint.contains(&coord)
                                && (!approach_footprint.contains(&coord)
                                    || adjacent_route_coords.contains(&coord))
                                && !fill_coords.contains(&coord)
                        })
                })
                .collect::<Vec<_>>();
            candidates.sort_unstable_by_key(|candidate| {
                (
                    candidate.distance(center),
                    named_sample(seed, "tunnel_crystal_alcoves", *candidate),
                    *candidate,
                )
            });
            candidates.first().copied()
        })
    };

    for (fixture_index, requested) in tunnel_light_indices(lit_centerline.len())
        .into_iter()
        .enumerate()
    {
        let origin = select_alcove_origin(
            requested,
            &lit_centerline,
            &route_coords,
            &alcove_footprint,
            None,
        )
        .ok_or_else(|| {
            schematic_contract(format!(
                "tunnel cannot carve crystal alcove {fixture_index}"
            ))
        })?;
        alcove_footprint.extend(
            origin
                .within_radius(1)
                .into_iter()
                .filter(|coord| !route_coords.contains(coord)),
        );
        alcove_origins.push(TilePos::new(origin, profile.crystal_base_level));
    }

    // The concealed approach is part of the Dark interior, but it is resolved
    // after the main tunnel centerline. Retain the established fixture set and
    // add only the deterministic alcoves required to cover those already-known
    // rows. This keeps seeds whose existing fixtures cover the threshold byte-
    // identical while preventing a long concealed approach from extending past
    // the final Dim-18 pool.
    while let Some((uncovered, _)) =
        first_uncovered_tunnel_floor(&required_interior_floors, crystal_mask, &alcove_origins)
    {
        let requested = required_lit_centerline
            .iter()
            .enumerate()
            .min_by_key(|(index, center)| (center.distance(uncovered.coord), *index))
            .map(|(index, _)| index)
            .ok_or_else(|| schematic_contract("tunnel lighting has no interior centerline"))?;
        let fixture_index = alcove_origins.len();
        let origin = select_alcove_origin(
            requested,
            &required_lit_centerline,
            &required_interior_route_coords,
            &alcove_footprint,
            Some(uncovered.coord),
        )
        .ok_or_else(|| {
            let nearest = nearest_tunnel_light_distance(uncovered, &alcove_origins);
            schematic_contract(format!(
                "tunnel cannot carve supplemental crystal alcove {fixture_index} for uncovered floor {uncovered:?}; nearest existing fixture distance is {nearest}"
            ))
        })?;
        alcove_footprint.extend(
            origin
                .within_radius(1)
                .into_iter()
                .filter(|coord| !required_interior_route_coords.contains(coord)),
        );
        alcove_origins.push(TilePos::new(origin, profile.crystal_base_level));
    }

    for origin in alcove_origins.iter().map(|position| position.coord) {
        // The side chamber may touch the four-wide ribbon, but it must never
        // recarve a route column: doing so would erase Gothic materials (and,
        // at tall rows, the authored clearance/roof profile) as the alcove's
        // rough-stone column replaces it.
        for coord in origin
            .within_radius(1)
            .into_iter()
            .filter(|coord| !required_interior_route_coords.contains(coord))
        {
            let biome = fine_index.biome(coord).ok_or_else(|| {
                schematic_contract(format!("tunnel alcove {coord:?} has no biome owner"))
            })?;
            let (top_surface, top_metadata) = volume
                .top_surface_at_coord(coord)
                .ok_or_else(|| schematic_contract("tunnel alcove column has no surface"))?;
            let existing_column = volume.columns[&coord].clone();
            let column = tunnel_column(
                &existing_column,
                top_surface.level,
                profile.crystal_base_level,
                CLEARANCE_TOP,
                ROOF_TOP,
                false,
                interior_id,
            );
            let floor = TilePos::new(coord, profile.crystal_base_level);
            replace_column_surface(
                volume,
                biome_regions,
                coord,
                column,
                floor,
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: Some(interior_id),
                },
                biome,
            );
            if top_surface.level >= ROOF_TOP {
                volume.surfaces.insert(top_surface, top_metadata);
                biome_regions.insert(top_surface, biome);
            }
            floors.insert(floor);
            alcove_footprint.insert(coord);
            for level in CLEARANCE_TOP..ROOF_TOP.min(top_surface.level) {
                roofs.insert(TilePos::new(coord, level));
            }
        }
    }

    let lights = tunnel_crystal_lights(seed, &alcove_origins)?;

    // Keep the inner approach underneath the mountain, then expose only the
    // outer grade. This makes the foot portal a recessed opening instead of a
    // ruler-straight surface trench pointing at the Crystal chamber.
    let open_approach_levels = descending_levels(
        outer_surface.level,
        profile.crystal_base_level,
        approach.len().saturating_sub(concealed_approach_rows),
    )
    .into_iter()
    .rev()
    .skip(1)
    .collect::<Vec<_>>();
    if open_approach_levels.len()
        != approach
            .len()
            .saturating_sub(concealed_approach_rows)
            .saturating_sub(1)
    {
        return Err(schematic_contract(
            "concealed tunnel approach produced a malformed exterior grade",
        ));
    }
    let mut approach_surfaces = BTreeSet::new();
    let mut open_level_index = 0_usize;
    let mut mouth_anchor = None;
    for (index, (center, row)) in approach.iter().copied().zip(&approach_rows).enumerate() {
        if index == 0 {
            continue;
        }
        if index <= concealed_approach_rows {
            for coord in row {
                let biome = fine_index.biome(*coord).ok_or_else(|| {
                    schematic_contract(format!(
                        "concealed tunnel approach lane {coord:?} has no biome owner"
                    ))
                })?;
                let (top_surface, top_metadata) =
                    volume.top_surface_at_coord(*coord).ok_or_else(|| {
                        schematic_contract("concealed approach column has no surface")
                    })?;
                let existing_column =
                    volume.columns.get(coord).cloned().ok_or_else(|| {
                        schematic_contract("concealed approach column disappeared")
                    })?;
                let column = tunnel_column(
                    &existing_column,
                    top_surface.level,
                    profile.crystal_base_level,
                    CLEARANCE_TOP,
                    ROOF_TOP,
                    false,
                    interior_id,
                );
                remove_column_surfaces(volume, biome_regions, *coord);
                volume.columns.insert(*coord, column);
                let floor = TilePos::new(*coord, profile.crystal_base_level);
                volume.surfaces.insert(
                    floor,
                    SurfaceMetadata {
                        access: SurfaceAccess::Ordinary,
                        interior: Some(interior_id),
                    },
                );
                biome_regions.insert(floor, biome);
                volume.surfaces.insert(top_surface, top_metadata);
                biome_regions.insert(top_surface, biome);
                floors.insert(floor);
                approach_surfaces.insert(floor);
                for level in CLEARANCE_TOP..ROOF_TOP {
                    roofs.insert(TilePos::new(*coord, level));
                }
            }
            let center_floor = TilePos::new(center, profile.crystal_base_level);
            if route_centerline.last().copied() != Some(center_floor) {
                route_centerline.push(center_floor);
            }
            continue;
        }

        let level = *open_approach_levels.get(open_level_index).ok_or_else(|| {
            schematic_contract("concealed tunnel approach exhausted its exterior grade")
        })?;
        open_level_index = open_level_index.saturating_add(1);
        if center == mouth_center {
            mouth_anchor = Some(TilePos::new(center, level));
        }
        for coord in row {
            if let Some(existing_floor) = floors
                .iter()
                .copied()
                .find(|surface| surface.coord == *coord)
            {
                approach_surfaces.insert(existing_floor);
                if *coord == center && route_centerline.last().copied() != Some(existing_floor) {
                    route_centerline.push(existing_floor);
                }
                continue;
            }
            let biome = fine_index.biome(*coord).ok_or_else(|| {
                schematic_contract(format!(
                    "tunnel exterior approach lane {coord:?} has no biome owner"
                ))
            })?;
            let cap = volume
                .columns
                .get(coord)
                .map(top_solid_material)
                .ok_or_else(|| {
                    schematic_contract(format!(
                        "tunnel exterior approach lane {coord:?} has no terrain cap"
                    ))
                })?;
            let surface = TilePos::new(*coord, level);
            replace_column_surface(
                volume,
                biome_regions,
                *coord,
                land_column(level, cap),
                surface,
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
                biome,
            );
            approach_surfaces.insert(surface);
            if *coord == center && route_centerline.last().copied() != Some(surface) {
                route_centerline.push(surface);
            }
        }
    }
    let authored_approach = approach_surfaces
        .iter()
        .map(|surface| surface.coord)
        .collect::<BTreeSet<_>>();
    if !approach_footprint.is_subset(&authored_approach) {
        return Err(schematic_contract(
            "tunnel exterior approach lost one or more of its exact four lanes",
        ));
    }

    validate_tunnel_alcove_geometry(
        &alcove_origins,
        &required_interior_route_coords,
        volume,
        interior_id,
        profile.crystal_base_level,
        CLEARANCE_TOP,
        ROOF_TOP,
    )?;

    if let Some((uncovered, nearest)) =
        first_uncovered_tunnel_floor(&floors, crystal_mask, &alcove_origins)
    {
        return Err(schematic_contract(
            format!(
                "paired tunnel fixtures do not cover every interior floor at Dim-18; first uncovered floor is {uncovered:?}, nearest fixture distance is {nearest}"
            ),
        ));
    }

    let entrances = entrance_row
        .iter()
        .copied()
        .map(|coord| TilePos::new(coord, profile.crystal_base_level))
        .filter(|position| floors.contains(position))
        .collect::<BTreeSet<_>>();
    if entrances.len() != 4 {
        return Err(schematic_contract(
            "the roofed tunnel foot threshold is not exact four-wide footing",
        ));
    }
    let route_surfaces = floors
        .iter()
        .copied()
        .chain(approach_surfaces)
        .chain(route_centerline.iter().copied())
        .collect::<BTreeSet<_>>();
    let midpoint = route_centerline[route_centerline.len() / 2];
    let gothic = route_centerline[GOTHIC_ROWS.min(route_centerline.len() - 1)];
    let mouth_anchor = mouth_anchor.ok_or_else(|| {
        schematic_contract("concealed tunnel approach omitted its first exterior mouth surface")
    })?;
    validate_compiled_tunnel_geometry(
        volume,
        interior_id,
        profile.crystal_base_level,
        crystal_mask,
        &planned_rows,
        GOTHIC_ROWS,
        CLEARANCE_TOP,
        ROOF_TOP,
    )?;
    Ok(TunnelCompilation {
        route: ProtectedFeatureRoute {
            centerline: route_centerline,
            surfaces: route_surfaces,
        },
        interior: PlannedInterior {
            floors,
            entrances,
            roof_voxels: roofs,
        },
        lights,
        anchors: BTreeMap::from([
            ("grand_v3.tunnel_mouth".to_owned(), mouth_anchor),
            ("grand_v3.tunnel_midpoint".to_owned(), midpoint),
            ("grand_v3.gothic_transition".to_owned(), gothic),
            ("grand_v3.ascent_threshold".to_owned(), ascent_target),
        ]),
        overburden,
    })
}

/// Resolves the canonical foot-to-summit route as one exact walker path.
///
/// The tunnel compiler orders its centerline from the Crystal threshold toward
/// the foothill. The world route reverses that prefix, then continues through
/// the authored Dark interior to the exact summit terminal. Decorative chamber
/// and alcove floors remain interior surfaces without pretending to be route
/// membership.
fn compile_exact_crystal_route(
    volume: &VolumePlan,
    blockers: &BTreeSet<TilePos>,
    interior_id: InteriorRegionId,
    crystal_mask: &BTreeSet<HexCoord>,
    summit: TilePos,
    tunnel: &ProtectedFeatureRoute,
) -> Result<ProtectedFeatureRoute, V3GenerationError> {
    let threshold = tunnel
        .centerline
        .first()
        .copied()
        .ok_or_else(|| schematic_contract("Crystal route has no tunnel threshold"))?;
    let allowed = volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| {
            (metadata.access == SurfaceAccess::Ordinary
                && crystal_mask.contains(&position.coord)
                && !blockers.contains(position))
            .then_some(*position)
        })
        .collect::<BTreeSet<_>>();
    if !allowed.contains(&threshold) || !allowed.contains(&summit) {
        return Err(schematic_contract(format!(
            "Crystal route endpoints are not exact unblocked site footing: threshold={threshold:?}, summit={summit:?}, interior={interior_id:?}"
        )));
    }

    let graph = OrdinaryGraph::from_volume(volume, Some(blockers));
    let mut parent = BTreeMap::<TilePos, TilePos>::new();
    let mut reached = BTreeSet::from([threshold]);
    let mut frontier = VecDeque::from([threshold]);
    while let Some(position) = frontier.pop_front() {
        if position == summit {
            break;
        }
        for neighbor in graph.neighbors(position) {
            if allowed.contains(neighbor) && reached.insert(*neighbor) {
                parent.insert(*neighbor, position);
                frontier.push_back(*neighbor);
            }
        }
    }
    if !reached.contains(&summit) {
        return Err(schematic_contract(
            "Crystal threshold has no exact site-contained walker path to the summit",
        ));
    }
    let mut interior_path = vec![summit];
    let mut cursor = summit;
    while cursor != threshold {
        cursor = parent.get(&cursor).copied().ok_or_else(|| {
            schematic_contract("Crystal interior walker path lost its deterministic parent")
        })?;
        interior_path.push(cursor);
    }
    interior_path.reverse();

    let mut centerline = tunnel.centerline.iter().rev().copied().collect::<Vec<_>>();
    centerline.extend(interior_path.iter().copied().skip(1));
    if centerline
        .windows(2)
        .any(|pair| !graph.admits(pair[0], pair[1]))
        || centerline.iter().copied().collect::<BTreeSet<_>>().len() != centerline.len()
    {
        return Err(schematic_contract(
            "combined tunnel/Crystal centerline is not one simple adjacent walker path",
        ));
    }
    let mut surfaces = tunnel.surfaces.clone();
    surfaces.extend(interior_path);
    if surfaces.iter().any(|position| {
        volume
            .surfaces
            .get(position)
            .is_none_or(|metadata| metadata.access != SurfaceAccess::Ordinary)
    }) {
        return Err(schematic_contract(
            "combined tunnel/Crystal route contains non-ordinary footing",
        ));
    }
    Ok(ProtectedFeatureRoute {
        centerline,
        surfaces,
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "the summit join validates semantic ownership while mutating exact terrain and biome projections"
)]
fn compile_frozen_summit_connection(
    plan: &SchematicPlanV1,
    profile: V3GrandV3BasicTerrainProfile,
    crystal_rotation: u8,
    summit: TilePos,
    upper_terminal: &BTreeSet<TilePos>,
    crystal_mask: &BTreeSet<HexCoord>,
    fine_index: &FineWorldIndex,
    volume: &mut VolumePlan,
    biome_regions: &mut BTreeMap<TilePos, BiomeRegionId>,
) -> Result<ProtectedFeatureRoute, V3GenerationError> {
    let frozen_patches = plan
        .cells
        .iter()
        .filter(|cell| has_overlay(cell, SchematicFeature::FrozenWoods))
        .map(|cell| PatchId(u32::from(cell.id.get())))
        .collect::<BTreeSet<_>>();
    if frozen_patches.is_empty() {
        return Err(schematic_contract(
            "Crystal summit connection has no authored Frozen Woods destination",
        ));
    }
    let boundary = super::crystal_ascent::macro_upper_terminal_approach_coords(
        crystal_mask,
        crystal_rotation,
        summit.level,
        1,
    )
    .map_err(schematic_contract)?;
    let outward = super::crystal_ascent::macro_upper_terminal_outward_rows(
        crystal_mask,
        crystal_rotation,
        summit.level,
        2,
    )
    .map_err(schematic_contract)?;
    let upper_coords = upper_terminal
        .iter()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    if upper_terminal.len() != 4
        || boundary.len() != 4
        || outward.len() != 2
        || outward.iter().any(|row| row.len() != 4)
        || upper_coords.len() != 4
        || !boundary.is_subset(crystal_mask)
        || !upper_coords.is_subset(crystal_mask)
        || outward.iter().flatten().any(|coord| {
            crystal_mask.contains(coord)
                || fine_index
                    .patch(*coord)
                    .is_none_or(|patch| !frozen_patches.contains(&patch))
        })
    {
        return Err(schematic_contract(
            "Crystal summit does not open through exact four-wide rows directly into Frozen Woods",
        ));
    }
    let ordered_coords = std::iter::once(&upper_coords)
        .chain(std::iter::once(&boundary))
        .chain(outward.iter())
        .collect::<Vec<_>>();
    let distinct_coords = ordered_coords
        .iter()
        .flat_map(|row| row.iter().copied())
        .collect::<BTreeSet<_>>();
    if distinct_coords.len() != 16
        || ordered_coords
            .windows(2)
            .any(|rows| !lane_rows_connect_smoothly(rows[0], rows[1]))
    {
        return Err(schematic_contract(
            "Crystal summit rows must form four disjoint, smoothly connected four-wide lanes",
        ));
    }

    let mut surfaces = upper_terminal.clone();
    let boundary_surfaces = boundary
        .iter()
        .copied()
        .map(|coord| TilePos::new(coord, summit.level))
        .collect::<BTreeSet<_>>();
    if boundary_surfaces.iter().any(|position| {
        volume
            .surfaces
            .get(position)
            .is_none_or(|metadata| metadata.access != SurfaceAccess::Ordinary)
    }) {
        return Err(schematic_contract(
            "Crystal summit boundary row is not exact ordinary footing",
        ));
    }
    surfaces.extend(boundary_surfaces.iter().copied());

    let mut graded_rows = Vec::with_capacity(outward.len());
    for (index, row) in outward.into_iter().enumerate() {
        let level = profile
            .crystal_base_level
            .saturating_add(profile.crystal_rise_levels)
            .saturating_add(i32::try_from(index).unwrap_or(i32::MAX))
            .saturating_add(1);
        let mut graded = BTreeSet::new();
        for coord in row {
            let biome = fine_index.biome(coord).ok_or_else(|| {
                schematic_contract(format!(
                    "Frozen summit connection column {coord:?} has no biome owner"
                ))
            })?;
            let surface = TilePos::new(coord, level);
            replace_column_surface(
                volume,
                biome_regions,
                coord,
                land_column(level, SolidMaterialRole::Snow),
                surface,
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
                biome,
            );
            graded.insert(surface);
        }
        surfaces.extend(graded.iter().copied());
        graded_rows.push(graded);
    }

    let mut centerline = vec![summit];
    let ordered_rows = std::iter::once(&boundary_surfaces).chain(graded_rows.iter());
    for row in ordered_rows {
        let previous = centerline.last().copied().ok_or_else(|| {
            schematic_contract("Frozen summit centerline lost its initial surface")
        })?;
        let next = row
            .iter()
            .copied()
            .filter(|candidate| {
                previous.coord.distance(candidate.coord) == 1
                    && previous.level.abs_diff(candidate.level) <= 1
            })
            .min()
            .ok_or_else(|| {
                schematic_contract(format!(
                    "Frozen summit row does not continue from {previous:?}"
                ))
            })?;
        centerline.push(next);
    }
    Ok(ProtectedFeatureRoute {
        centerline,
        surfaces,
    })
}

/// Rebuilds the one Grand composite interior projection from its authoritative
/// volume tags, then seals the owned world before it can be mutated again.
///
/// The exact Grand contract owns one unified tunnel/Crystal interior. Rejecting
/// foreign plan entries and foreign volume tags here makes the resulting
/// admission equivalent to the generic cross-layer interior projection pass.
fn admit_reconciled_grand_world(
    mut world: GeneratedWorldPlan,
    interior_id: InteriorRegionId,
    claimed_layout: super::schematic_crystal::ClaimedSchematicLayoutAdmission,
) -> Result<GrandWorldConstructionAdmission, V3GenerationError> {
    if !claimed_layout.matches_final_layout(&world.layout) {
        return Err(schematic_contract(
            "Grand construction changed its layout after the Crystal site claim was validated",
        ));
    }
    if world.layout.kind != LayoutKind::Schematic
        || !world
            .layout
            .patches
            .contains_key(&claimed_layout.patch_id())
    {
        return Err(schematic_contract(
            "Grand construction admission does not match its claimed Schematic layout",
        ));
    }
    if world.interiors.by_id.len() != 1 || !world.interiors.by_id.contains_key(&interior_id) {
        return Err(schematic_contract(
            "Grand construction must own exactly one unified tunnel/Crystal interior",
        ));
    }

    let mut floors = BTreeSet::new();
    for (position, metadata) in &world.volume.surfaces {
        let Some(region) = metadata.interior else {
            continue;
        };
        if region != interior_id {
            return Err(schematic_contract(format!(
                "Grand volume surface {position:?} names foreign interior {region:?}"
            )));
        }
        floors.insert(*position);
    }

    let mut roofs = BTreeSet::new();
    for (coord, column) in &world.volume.columns {
        for element in &column.elements {
            let VolumeElement::Solid(mass) = *element else {
                continue;
            };
            let Some(region) = mass.cutaway_for else {
                continue;
            };
            if region != interior_id {
                return Err(schematic_contract(format!(
                    "Grand volume column {coord:?} names foreign cutaway interior {region:?}"
                )));
            }
            roofs.extend(
                (mass.levels.bottom..mass.levels.top).map(|level| TilePos::new(*coord, level)),
            );
        }
    }
    let interior =
        world.interiors.by_id.get_mut(&interior_id).ok_or_else(|| {
            schematic_contract("composite interior vanished before reconciliation")
        })?;
    if interior.entrances.len() != 8 || !interior.entrances.is_subset(&floors) {
        return Err(schematic_contract(
            "reconciled composite interior lost its exact foot/summit entrances",
        ));
    }
    interior.floors = floors;
    interior.roof_voxels = roofs;
    Ok(GrandWorldConstructionAdmission {
        plan: world,
        _layout: claimed_layout,
    })
}

fn tunnel_column(
    existing: &VolumeColumn,
    surface: Level,
    floor: Level,
    clearance_top: Level,
    roof_top: Level,
    gothic: bool,
    interior_id: InteriorRegionId,
) -> VolumeColumn {
    let tunnel_material = if gothic {
        SolidMaterialRole::WorkedStone
    } else {
        SolidMaterialRole::Stone
    };
    let mut elements = vec![
        VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(0, 1),
            material: SolidMaterialRole::Bedrock,
            cutaway_for: None,
        }),
        VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(1, floor),
            material: SolidMaterialRole::Stone,
            cutaway_for: None,
        }),
    ];
    push_canonical_solid(
        &mut elements,
        SolidMass {
            levels: LevelInterval::new(floor, floor.saturating_add(1)),
            material: tunnel_material,
            cutaway_for: None,
        },
    );
    let roof_end = roof_top.min(surface);
    if clearance_top < roof_end {
        push_canonical_solid(
            &mut elements,
            SolidMass {
                levels: LevelInterval::new(clearance_top, roof_end),
                material: tunnel_material,
                cutaway_for: Some(interior_id),
            },
        );
    }
    // Preserve the source column's material strata above the authored roof.
    // Replacing this entire band with uniform Stone made the tunnel visible as
    // a ruler-straight stripe on exposed mountain sides even though its top cap
    // had the correct material.
    let cap = top_solid_material(existing);
    for level in roof_end..=surface {
        let material = solid_mass_at_level(existing, level)
            .map(|mass| mass.material)
            .unwrap_or(if level == surface {
                cap
            } else {
                SolidMaterialRole::Stone
            });
        push_canonical_solid(
            &mut elements,
            SolidMass {
                levels: LevelInterval::new(level, level.saturating_add(1)),
                material,
                cutaway_for: None,
            },
        );
    }
    VolumeColumn { elements }
}

fn capture_tunnel_overburden_authority(
    volume: &VolumePlan,
    coords: &BTreeSet<HexCoord>,
    roof_top: Level,
) -> Result<TunnelOverburdenAuthority, V3GenerationError> {
    if coords.is_empty() {
        return Err(schematic_contract(
            "tunnel overburden authority has no roofed outside columns",
        ));
    }
    let mut columns = BTreeMap::new();
    for coord in coords {
        let surface = volume
            .top_surface_at_coord(*coord)
            .map(|(surface, _)| surface)
            .ok_or_else(|| {
                schematic_contract(format!(
                    "tunnel overburden coordinate {coord:?} has no source surface"
                ))
            })?;
        if surface.level < roof_top {
            return Err(schematic_contract(format!(
                "tunnel overburden source {surface:?} lies below required roof top {roof_top}"
            )));
        }
        let column = volume.columns.get(coord).ok_or_else(|| {
            schematic_contract(format!(
                "tunnel overburden coordinate {coord:?} has no source column"
            ))
        })?;
        let voxels = (roof_top..=surface.level)
            .map(|level| {
                let mass = solid_mass_at_level(column, level).ok_or_else(|| {
                    schematic_contract(format!(
                        "tunnel overburden source has an exposed gap at {:?}",
                        TilePos::new(*coord, level)
                    ))
                })?;
                Ok((
                    level,
                    TunnelOverburdenVoxelAuthority {
                        material: mass.material,
                        cutaway_for: mass.cutaway_for,
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, V3GenerationError>>()?;
        columns.insert(*coord, TunnelOverburdenColumnAuthority { surface, voxels });
    }
    Ok(TunnelOverburdenAuthority { columns })
}

/// Keeps the exterior approach no wider than the concealed four-lane tunnel.
fn tunnel_approach_lane_offsets(lane_offsets: [i32; 4]) -> Vec<i32> {
    lane_offsets.into_iter().collect()
}

#[expect(
    clippy::too_many_arguments,
    reason = "the tunnel validator compares exact volume, interior, route, material, and clearance authorities"
)]
fn validate_compiled_tunnel_geometry(
    volume: &VolumePlan,
    interior_id: InteriorRegionId,
    floor_level: Level,
    crystal_mask: &BTreeSet<HexCoord>,
    rows: &[BTreeSet<HexCoord>],
    gothic_rows: usize,
    clearance_top: Level,
    roof_top: Level,
) -> Result<(), V3GenerationError> {
    if rows.is_empty() || rows.iter().any(|row| row.len() != 4) {
        return Err(schematic_contract(
            "compiled tunnel must retain exactly four lanes in every roofed row",
        ));
    }
    let gothic_coords = rows
        .iter()
        .enumerate()
        .filter(|(index, _)| *index > 0 && *index <= gothic_rows)
        .flat_map(|(_, row)| row.iter().copied())
        .collect::<BTreeSet<_>>();
    let mut outside_rows = 0_usize;
    let mut gothic_outside_rows = 0_usize;
    for (index, row) in rows.iter().enumerate() {
        let authored_terminal = index == 0 && row.iter().all(|coord| crystal_mask.contains(coord));
        if authored_terminal {
            continue;
        }
        outside_rows = outside_rows.saturating_add(1);
        let gothic = index > 0 && index <= gothic_rows;
        gothic_outside_rows = gothic_outside_rows.saturating_add(usize::from(gothic));
        for coord in row {
            let expected_material = if gothic_coords.contains(coord) {
                SolidMaterialRole::WorkedStone
            } else {
                SolidMaterialRole::Stone
            };
            let floor = TilePos::new(*coord, floor_level);
            let metadata = volume.surfaces.get(&floor).ok_or_else(|| {
                schematic_contract(format!("compiled tunnel floor is missing at {floor:?}"))
            })?;
            if metadata.access != SurfaceAccess::Ordinary || metadata.interior != Some(interior_id)
            {
                return Err(schematic_contract(format!(
                    "compiled tunnel floor {floor:?} is not ordinary footing in the unified interior"
                )));
            }
            let column = volume.columns.get(coord).ok_or_else(|| {
                schematic_contract(format!("compiled tunnel column {coord:?} is missing"))
            })?;
            let floor_mass = solid_mass_at_level(column, floor_level).ok_or_else(|| {
                schematic_contract(format!("compiled tunnel floor {floor:?} is unsupported"))
            })?;
            if floor_mass.material != expected_material {
                let row_memberships = rows
                    .iter()
                    .enumerate()
                    .filter_map(|(row_index, candidate)| {
                        candidate.contains(coord).then_some(row_index)
                    })
                    .collect::<Vec<_>>();
                return Err(schematic_contract(format!(
                    "compiled tunnel floor {floor:?} has {:?}, expected {expected_material:?}; rows={row_memberships:?}, column={:?}",
                    floor_mass.material, column.elements
                )));
            }
            for level in floor_level.saturating_add(1)..clearance_top {
                if solid_mass_at_level(column, level).is_some() {
                    return Err(schematic_contract(format!(
                        "compiled tunnel clearance is occupied at {:?}",
                        TilePos::new(*coord, level)
                    )));
                }
            }
            for level in clearance_top..roof_top {
                let roof = solid_mass_at_level(column, level).ok_or_else(|| {
                    schematic_contract(format!(
                        "compiled tunnel lacks its three-level roof at {:?}",
                        TilePos::new(*coord, level)
                    ))
                })?;
                if roof.material != expected_material || roof.cutaway_for != Some(interior_id) {
                    return Err(schematic_contract(format!(
                        "compiled tunnel roof {:?} has {:?}/{:?}, expected {expected_material:?}/{interior_id:?}",
                        TilePos::new(*coord, level),
                        roof.material,
                        roof.cutaway_for
                    )));
                }
            }
        }
    }
    if outside_rows < gothic_rows || gothic_outside_rows != gothic_rows {
        return Err(schematic_contract(format!(
            "compiled tunnel requires exactly {gothic_rows} outside Gothic rows, got {gothic_outside_rows} across {outside_rows} outside rows"
        )));
    }
    Ok(())
}

fn solid_mass_at_level(column: &VolumeColumn, level: Level) -> Option<SolidMass> {
    column.elements.iter().find_map(|element| {
        let VolumeElement::Solid(mass) = *element else {
            return None;
        };
        (mass.levels.bottom <= level && level < mass.levels.top).then_some(mass)
    })
}

fn tunnel_light_indices(centerline_len: usize) -> Vec<usize> {
    const SPACING: usize = 18;
    if centerline_len == 0 {
        return Vec::new();
    }
    let last = centerline_len.saturating_sub(1);
    let mut indices = Vec::new();
    let mut index = last.min(SPACING / 2);
    loop {
        indices.push(index);
        if index.saturating_add(SPACING) >= last {
            break;
        }
        index = index.saturating_add(SPACING);
    }
    if last.saturating_sub(*indices.last().unwrap_or(&0)) > SPACING / 2 {
        indices.push(last.saturating_sub(SPACING / 2));
    }
    indices.sort_unstable();
    indices.dedup();
    indices
}

fn nearest_tunnel_light_distance(floor: TilePos, origins: &[TilePos]) -> u32 {
    origins
        .iter()
        .map(|origin| origin.coord.distance(floor.coord))
        .min()
        .unwrap_or(u32::MAX)
}

fn first_uncovered_tunnel_floor(
    floors: &BTreeSet<TilePos>,
    excluded_coords: &BTreeSet<HexCoord>,
    origins: &[TilePos],
) -> Option<(TilePos, u32)> {
    floors.iter().copied().find_map(|floor| {
        if excluded_coords.contains(&floor.coord) {
            return None;
        }
        let nearest = nearest_tunnel_light_distance(floor, origins);
        (nearest > TUNNEL_DIM_LIGHT_RADIUS).then_some((floor, nearest))
    })
}

fn tunnel_alcove_candidate_has_complete_roof_support(
    origin: HexCoord,
    protected_route_coords: &BTreeSet<HexCoord>,
    volume: &VolumePlan,
    roof_top: Level,
) -> bool {
    origin
        .within_radius(1)
        .into_iter()
        .filter(|coord| !protected_route_coords.contains(coord))
        .all(|coord| {
            volume
                .top_surface_at_coord(coord)
                .is_some_and(|(surface, _)| surface.level >= roof_top)
        })
}

fn validate_tunnel_alcove_geometry(
    origins: &[TilePos],
    protected_route_coords: &BTreeSet<HexCoord>,
    volume: &VolumePlan,
    interior_id: InteriorRegionId,
    floor_level: Level,
    clearance_top: Level,
    roof_top: Level,
) -> Result<(), V3GenerationError> {
    for origin in origins {
        for coord in origin
            .coord
            .within_radius(1)
            .into_iter()
            .filter(|coord| !protected_route_coords.contains(coord))
        {
            let floor = TilePos::new(coord, floor_level);
            if volume.surfaces.get(&floor).is_none_or(|metadata| {
                metadata.access != SurfaceAccess::Ordinary || metadata.interior != Some(interior_id)
            }) {
                return Err(schematic_contract(format!(
                    "tunnel alcove at {:?} lacks exact interior floor {floor:?}",
                    origin.coord
                )));
            }
            let column = volume.columns.get(&coord).ok_or_else(|| {
                schematic_contract(format!(
                    "tunnel alcove at {:?} lost column {coord:?}",
                    origin.coord
                ))
            })?;
            if solid_mass_at_level(column, floor_level).is_none()
                || (floor_level.saturating_add(1)..clearance_top)
                    .any(|level| solid_mass_at_level(column, level).is_some())
            {
                return Err(schematic_contract(format!(
                    "tunnel alcove at {:?} has malformed floor or clearance in column {coord:?}",
                    origin.coord
                )));
            }
            for level in clearance_top..roof_top {
                if solid_mass_at_level(column, level)
                    .is_none_or(|mass| mass.cutaway_for != Some(interior_id))
                {
                    return Err(schematic_contract(format!(
                        "tunnel alcove at {:?} lacks cutaway roof {:?}",
                        origin.coord,
                        TilePos::new(coord, level)
                    )));
                }
            }
        }
    }
    Ok(())
}

fn tunnel_crystal_lights(
    seed: u64,
    origins: &[TilePos],
) -> Result<BTreeMap<LightId, PlannedGameplayLight>, V3GenerationError> {
    let mut lights = BTreeMap::new();
    for (index, origin) in origins.iter().copied().enumerate() {
        let ordinal = u32::try_from(index)
            .map_err(|error| schematic_contract(format!("tunnel light index overflow: {error}")))?;
        let kind = match named_sample(seed, "tunnel_crystal_kind", origin.coord) % 3 {
            0 => CaveCrystalKind::LowCluster,
            1 => CaveCrystalKind::Branched,
            _ => CaveCrystalKind::Spire,
        };
        let rotation =
            u8::try_from(named_sample(seed, "tunnel_crystal_rotation", origin.coord) % 6)
                .unwrap_or_default();
        let bright_id = LightId(
            TUNNEL_LIGHT_BASE
                .checked_add(ordinal.saturating_mul(2))
                .ok_or_else(|| schematic_contract("tunnel bright-light ID overflow"))?,
        );
        let dim_id = LightId(
            TUNNEL_LIGHT_BASE
                .checked_add(ordinal.saturating_mul(2).saturating_add(1))
                .ok_or_else(|| schematic_contract("tunnel dim-light ID overflow"))?,
        );
        lights.insert(
            bright_id,
            PlannedGameplayLight {
                origin,
                level: IlluminationLevel::Bright,
                radius: 4,
                presentation: Some(PlannedLightPresentation::CaveCrystal(
                    CaveCrystalPresentation {
                        kind,
                        site: CaveCrystalSiteKind::InteriorAlcove,
                        rotation,
                    },
                )),
            },
        );
        lights.insert(
            dim_id,
            PlannedGameplayLight {
                origin,
                level: IlluminationLevel::Dim,
                radius: TUNNEL_DIM_LIGHT_RADIUS,
                presentation: None,
            },
        );
    }
    Ok(lights)
}

fn compile_natural_pass(
    seed: u64,
    layout: &ResolvedLayoutPlan,
    fine_index: &FineWorldIndex,
    water_coords: &BTreeSet<HexCoord>,
    surface_route_exclusion: &BTreeSet<HexCoord>,
    volume: &mut VolumePlan,
    biome_regions: &mut BTreeMap<TilePos, hex_core::BiomeRegionId>,
) -> Result<NaturalPassCompilation, V3GenerationError> {
    let preferred_start = schematic_to_world(
        SchematicCoord::new(7, 0, -7).map_err(|error| schematic_contract(error.to_string()))?,
        22,
    );
    let corner = schematic_to_world(
        SchematicCoord::new(7, -7, 0).map_err(|error| schematic_contract(error.to_string()))?,
        22,
    );
    let target = schematic_to_world(
        SchematicCoord::new(3, -7, 4).map_err(|error| schematic_contract(error.to_string()))?,
        22,
    );
    let mut declared_spine = preferred_start.line_between(corner);
    append_path(&mut declared_spine, corner.line_between(target));
    let declared_corridor = declared_spine
        .iter()
        .flat_map(|coord| coord.within_radius(12))
        .collect::<BTreeSet<_>>();
    let centerline_exclusion = surface_route_exclusion
        .iter()
        .flat_map(|coord| coord.within_radius(2))
        .collect::<BTreeSet<_>>();
    let dry_levels = declared_corridor
        .iter()
        .copied()
        .filter(|coord| {
            !water_coords.contains(coord)
                && !centerline_exclusion.contains(coord)
                && (*coord == preferred_start
                    || *coord == target
                    || coord.within_radius(2).into_iter().all(|neighbor| {
                        layout.footprint.contains(&neighbor) && !water_coords.contains(&neighbor)
                    }))
        })
        .filter_map(|coord| {
            volume
                .surfaces_at_coord(coord)
                .filter_map(|(position, metadata)| {
                    (metadata.access != SurfaceAccess::NonStandable).then_some(position.level)
                })
                .max()
                .map(|level| (coord, level))
        })
        .collect::<BTreeMap<_, _>>();
    // The schematic coastline and valley lake are allowed to vary. A valid
    // generated plan can therefore cut the preferred lower end of this fixed
    // mountain corridor without invalidating its dry target side. Resolve the
    // target-connected component first, then advance along the authored spine
    // only as far as needed to find dry lower-band footing. The reference plan
    // retains `preferred_start` exactly; seeded water is never overwritten.
    let mut target_component = BTreeSet::new();
    let mut frontier = VecDeque::from([target]);
    while let Some(coord) = frontier.pop_front() {
        if !dry_levels.contains_key(&coord) || !target_component.insert(coord) {
            continue;
        }
        let mut neighbors = coord.neighbors();
        neighbors.sort_unstable();
        frontier.extend(
            neighbors
                .into_iter()
                .filter(|neighbor| dry_levels.contains_key(neighbor)),
        );
    }
    let start = declared_spine
        .iter()
        .copied()
        .find(|coord| {
            target_component.contains(coord)
                && dry_levels.get(coord).is_some_and(|level| {
                    OrdinaryRegionBand::containing(*level) == OrdinaryRegionBand::Lower
                })
        })
        .ok_or_else(|| {
            schematic_contract(
                "declared mountain corridor has no target-connected dry lower entrance",
            )
        })?;
    let path = minimax_surface_path(start, target, &dry_levels).ok_or_else(|| {
        schematic_contract("declared mountain corridor has no dry minimax natural pass")
    })?;
    let start_surface = TilePos::new(
        start,
        *dry_levels
            .get(&start)
            .ok_or_else(|| schematic_contract("natural pass start has no dry footing"))?,
    );
    let target_surface = TilePos::new(
        target,
        *dry_levels
            .get(&target)
            .ok_or_else(|| schematic_contract("natural pass target has no dry footing"))?,
    );
    if start_surface.level.abs_diff(target_surface.level)
        > u32::try_from(path.len().saturating_sub(1)).unwrap_or(u32::MAX)
    {
        return Err(schematic_contract(
            "natural pass is too short for one-level walker transitions",
        ));
    }
    let width = seeded_natural_pass_width(seed, start);
    let half = i32::try_from(width / 2).unwrap_or_default();
    let first_lane = -half;
    let last_lane = first_lane.saturating_add(i32::try_from(width).unwrap_or_default() - 1);
    let mut pass_coords = BTreeSet::new();
    for (index, center) in path.iter().copied().enumerate() {
        let lane_direction = path_direction(&path, index);
        for lane in first_lane..=last_lane {
            let coord = step_in_direction(center, (lane_direction + 2) % 6, lane);
            if water_coords.contains(&coord) {
                return Err(schematic_contract(format!(
                    "natural pass intersects authoritative water at {coord:?}"
                )));
            }
            if !fine_index.by_coord.contains_key(&coord) {
                return Err(schematic_contract(
                    "natural pass leaves the world footprint",
                ));
            }
            if surface_route_exclusion.contains(&coord) {
                return Err(schematic_contract(format!(
                    "natural pass intersects exact corrective terrain at {coord:?}"
                )));
            }
            pass_coords.insert(coord);
        }
    }
    let bank_minimums = recessed_water_bank_minimums(volume)
        .into_iter()
        .filter(|(coord, _)| pass_coords.contains(coord))
        .collect::<BTreeMap<_, _>>();
    let graded = graded_corridor_levels_with_minimums(
        &pass_coords,
        start_surface,
        target_surface,
        &bank_minimums,
    )
    .ok_or_else(|| {
        schematic_contract(format!(
            "natural-pass width has no one-step height field satisfying {} exact water-bank minimums",
            bank_minimums.len()
        ))
    })?;
    for (coord, level) in graded {
        let biome = fine_index
            .biome(coord)
            .ok_or_else(|| schematic_contract("graded natural pass leaves the world footprint"))?;
        replace_column_surface(
            volume,
            biome_regions,
            coord,
            land_column(level, SolidMaterialRole::Gravel),
            TilePos::new(coord, level),
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: None,
            },
            biome,
        );
    }
    let surfaces_by_coord = pass_coords
        .iter()
        .filter_map(|coord| top_standable_surface(volume, *coord).map(|surface| (*coord, surface)))
        .collect::<BTreeMap<_, _>>();
    let surfaces = pass_coords
        .iter()
        .filter_map(|coord| surfaces_by_coord.get(coord).copied())
        .collect::<BTreeSet<_>>();
    let centerline = path
        .iter()
        .filter_map(|coord| surfaces_by_coord.get(coord).copied())
        .collect::<Vec<_>>();
    if surfaces.len() != pass_coords.len()
        || centerline.len() != path.len()
        || centerline
            .windows(2)
            .any(|pair| pair[0].level.abs_diff(pair[1].level) > 1)
        || pass_coords.iter().any(|coord| {
            let Some(surface) = surfaces_by_coord.get(coord) else {
                return true;
            };
            coord.neighbors().into_iter().any(|neighbor| {
                pass_coords.contains(&neighbor)
                    && surfaces_by_coord
                        .get(&neighbor)
                        .is_none_or(|other| surface.level.abs_diff(other.level) > 1)
            })
        })
    {
        return Err(schematic_contract(
            "natural pass width lost its continuous one-level walker transitions",
        ));
    }
    let anchor = centerline.get(centerline.len() / 2).copied();
    let compilation = NaturalPassCompilation {
        route: ProtectedFeatureRoute {
            centerline,
            surfaces,
        },
        anchor,
        width,
    };
    let admitted_width =
        validate_natural_pass_physical_width(seed, &compilation.route, volume, None)?;
    if admitted_width != compilation.width {
        return Err(schematic_contract(format!(
            "natural pass retained width {admitted_width}, expected {} from its named stream",
            compilation.width
        )));
    }
    Ok(compilation)
}

fn seeded_natural_pass_width(seed: u64, start: HexCoord) -> u32 {
    3_u32.saturating_add(
        u32::try_from(named_sample(seed, "pass_width", start) % 3).unwrap_or_default(),
    )
}

/// Proves that every seeded cross-section remains exact physical walker footing.
///
/// The protected route stores an ordered centreline plus an unordered reserved
/// footprint. Reconstructing each perpendicular row from the centreline makes
/// the authored width independently checkable after later route carving and
/// decoration, rather than trusting the construction loop which first inserted
/// those surfaces.
fn validate_natural_pass_physical_width(
    seed: u64,
    route: &ProtectedFeatureRoute,
    volume: &VolumePlan,
    blockers: Option<&BTreeSet<TilePos>>,
) -> Result<u32, V3GenerationError> {
    let start = route
        .centerline
        .first()
        .copied()
        .ok_or_else(|| schematic_contract("natural pass has no centreline"))?;
    let width = seeded_natural_pass_width(seed, start.coord);
    if !(3..=5).contains(&width) {
        return Err(schematic_contract(format!(
            "natural-pass named stream resolved invalid width {width}"
        )));
    }
    let centerline = route
        .centerline
        .iter()
        .map(|position| position.coord)
        .collect::<Vec<_>>();
    if centerline.len() < 2
        || centerline.iter().copied().collect::<BTreeSet<_>>().len() != centerline.len()
    {
        return Err(schematic_contract(
            "natural pass requires one simple multi-cell centreline",
        ));
    }
    let mut actual_by_coord = BTreeMap::new();
    for position in &route.surfaces {
        if actual_by_coord.insert(position.coord, *position).is_some() {
            return Err(schematic_contract(format!(
                "natural pass publishes stacked reserved surfaces at {:?}",
                position.coord
            )));
        }
    }

    let half = i32::try_from(width / 2).unwrap_or_default();
    let first_lane = -half;
    let last_lane = first_lane.saturating_add(i32::try_from(width).unwrap_or_default() - 1);
    let mut expected_coords = BTreeSet::new();
    for (index, center) in centerline.iter().copied().enumerate() {
        let direction = path_direction(&centerline, index);
        let row = (first_lane..=last_lane)
            .map(|lane| step_in_direction(center, (direction + 2) % 6, lane))
            .collect::<BTreeSet<_>>();
        if row.len() != usize::try_from(width).unwrap_or_default() {
            return Err(schematic_contract(format!(
                "natural-pass cross-section {index} has {} unique cells, expected {width}",
                row.len()
            )));
        }
        for coord in &row {
            let position = actual_by_coord.get(coord).copied().ok_or_else(|| {
                schematic_contract(format!(
                    "natural-pass cross-section {index} lost lane coordinate {coord:?}"
                ))
            })?;
            if !ordinary_surface_is_node(volume, blockers, position) {
                return Err(schematic_contract(format!(
                    "natural-pass lane {position:?} is not exact unblocked Ordinary footing"
                )));
            }
        }
        expected_coords.extend(row);
    }
    if !actual_by_coord
        .keys()
        .copied()
        .eq(expected_coords.iter().copied())
    {
        return Err(schematic_contract(
            "natural-pass reserved footprint does not equal its exact seeded-width rows",
        ));
    }
    for (coord, position) in &actual_by_coord {
        for neighbor in coord
            .neighbors()
            .into_iter()
            .filter(|neighbor| *coord < *neighbor && expected_coords.contains(neighbor))
        {
            let other = actual_by_coord.get(&neighbor).copied().ok_or_else(|| {
                schematic_contract("natural-pass row index lost an expected neighbor")
            })?;
            if !ordinary_transition_is_admitted(volume, blockers, *position, other) {
                return Err(schematic_contract(format!(
                    "natural-pass width lost walker edge {position:?} -> {other:?}"
                )));
            }
        }
    }
    Ok(width)
}

/// Carves the authored upper ledge through the locked north-east PeakRing arm.
///
/// The coarse schematic guarantees this exact arm, but raw sharp-peak cones do
/// not guarantee walker transitions. Constructing the ledge immediately after
/// the natural pass keeps lower-region hub reservations from closing the only
/// intended upper approach to the arm's waterfall-side tail.
fn compile_peak_saddle(
    plan: &SchematicPlanV1,
    layout: &ResolvedLayoutPlan,
    fine_index: &FineWorldIndex,
    water_coords: &BTreeSet<HexCoord>,
    natural_pass: &ProtectedFeatureRoute,
    surface_route_exclusion: &BTreeSet<HexCoord>,
    saddle_level: Level,
    volume: &mut VolumePlan,
    biome_regions: &mut BTreeMap<TilePos, hex_core::BiomeRegionId>,
) -> Result<PeakSaddleCompilation, V3GenerationError> {
    const FROZEN_SHORE_CONTACT: (i32, i32, i32) = (3, -6, 3);
    const NORTH_EAST_ARM: [(i32, i32, i32); 4] = [(4, -7, 3), (5, -7, 2), (6, -7, 1), (6, -6, 0)];

    let start = natural_pass
        .centerline
        .iter()
        .rev()
        .copied()
        .find(|position| {
            OrdinaryRegionBand::containing(position.level) == OrdinaryRegionBand::Upper
        })
        .ok_or_else(|| schematic_contract("natural pass has no upper node for the peak saddle"))?;
    let natural_by_coord = natural_pass.surfaces.iter().copied().try_fold(
        BTreeMap::<HexCoord, TilePos>::new(),
        |mut by_coord, position| {
            if by_coord.insert(position.coord, position).is_some() {
                return Err(schematic_contract(format!(
                    "natural pass publishes stacked reserved surfaces at {:?}",
                    position.coord
                )));
            }
            Ok(by_coord)
        },
    )?;
    if natural_by_coord.get(&start.coord).copied() != Some(start) {
        return Err(schematic_contract(
            "peak-saddle start is not an exact reserved natural-pass surface",
        ));
    }

    let mut authored_corridor = layout
        .patches
        .values()
        .find(|patch| patch.mask.contains(&start.coord))
        .map(|patch| patch.mask.clone())
        .ok_or_else(|| schematic_contract("peak-saddle start has no patch owner"))?;
    authored_corridor.retain(|coord| !surface_route_exclusion.contains(coord));
    let mut allowed = authored_corridor.clone();
    // The exact natural-pass suffix briefly crosses coarse owners outside the
    // authored PeakRing arm before returning to its final locked cells. Admit
    // only that immutable ribbon, never the surrounding outer-patch terrain.
    // The phase machine still prevents a premature merge or a second exit.
    allowed.extend(natural_by_coord.keys().copied());
    let mut waypoint_zones = Vec::with_capacity(NORTH_EAST_ARM.len().saturating_add(1));
    let mut pass_admitting_waypoint_zones =
        Vec::with_capacity(NORTH_EAST_ARM.len().saturating_add(1));

    let contact_coord = SchematicCoord::new(
        FROZEN_SHORE_CONTACT.0,
        FROZEN_SHORE_CONTACT.1,
        FROZEN_SHORE_CONTACT.2,
    )
    .map_err(|error| schematic_contract(error.to_string()))?;
    let contact = plan.cell(contact_coord).ok_or_else(|| {
        schematic_contract("peak saddle is missing its locked frozen-shore contact")
    })?;
    if contact.facts.surface != SurfaceKind::Land
        || contact.facts.landform != LandformKind::Shore
        || contact.facts.climate != ClimateKind::Frozen
        || contact.facts.access != AccessIntent::Ordinary
        || !has_overlay(contact, SchematicFeature::FrozenWoods)
    {
        return Err(schematic_contract(
            "peak-saddle contact lost its locked Land/Shore/Frozen/Ordinary/FrozenWoods contract",
        ));
    }
    let contact_patch = layout
        .patches
        .get(&PatchId(u32::from(contact.id.get())))
        .ok_or_else(|| schematic_contract("peak-saddle contact has no resolved patch"))?;
    authored_corridor.extend(
        contact_patch
            .mask
            .iter()
            .copied()
            .filter(|coord| !surface_route_exclusion.contains(coord)),
    );
    allowed.extend(
        contact_patch
            .mask
            .iter()
            .copied()
            .filter(|coord| !surface_route_exclusion.contains(coord)),
    );
    waypoint_zones.push(peak_saddle_waypoint_zone(
        contact_patch,
        schematic_to_world(contact_coord, 22),
        &natural_by_coord,
        water_coords,
        surface_route_exclusion,
        false,
    )?);
    pass_admitting_waypoint_zones.push(peak_saddle_waypoint_zone(
        contact_patch,
        schematic_to_world(contact_coord, 22),
        &natural_by_coord,
        water_coords,
        surface_route_exclusion,
        true,
    )?);

    for (q, r, s) in NORTH_EAST_ARM {
        let coord =
            SchematicCoord::new(q, r, s).map_err(|error| schematic_contract(error.to_string()))?;
        let cell = plan.cell(coord).ok_or_else(|| {
            schematic_contract(format!("peak-saddle arm is missing locked cell {coord:?}"))
        })?;
        if cell.facts.surface != SurfaceKind::Land
            || cell.facts.landform != LandformKind::SharpPeak
            || cell.facts.access != AccessIntent::Ordinary
            || !has_overlay(cell, SchematicFeature::PeakRing)
        {
            return Err(schematic_contract(format!(
                "peak-saddle arm cell {} lost its locked Land/SharpPeak/Ordinary/PeakRing contract",
                cell.id.get()
            )));
        }
        let patch = layout
            .patches
            .get(&PatchId(u32::from(cell.id.get())))
            .ok_or_else(|| {
                schematic_contract(format!(
                    "peak-saddle arm cell {} has no resolved patch",
                    cell.id.get()
                ))
            })?;
        authored_corridor.extend(
            patch
                .mask
                .iter()
                .copied()
                .filter(|fine| !surface_route_exclusion.contains(fine)),
        );
        allowed.extend(
            patch
                .mask
                .iter()
                .copied()
                .filter(|fine| !surface_route_exclusion.contains(fine)),
        );
        waypoint_zones.push(peak_saddle_waypoint_zone(
            patch,
            schematic_to_world(coord, 22),
            &natural_by_coord,
            water_coords,
            surface_route_exclusion,
            false,
        )?);
        pass_admitting_waypoint_zones.push(peak_saddle_waypoint_zone(
            patch,
            schematic_to_world(coord, 22),
            &natural_by_coord,
            water_coords,
            surface_route_exclusion,
            true,
        )?);
    }

    let bank_minimums = recessed_water_bank_minimums(volume);
    let primary = resolve_peak_saddle_path(
        start,
        &allowed,
        &waypoint_zones,
        &natural_by_coord,
        water_coords,
        &bank_minimums,
        volume,
        saddle_level,
        false,
    );
    let (path, resolved_allowed) = primary
        .map(|path| (path, allowed.clone()))
        .or_else(|primary_diagnostic| {
            let ordinary_patches = plan
                .cells
                .iter()
                .filter(|cell| is_ordinary_land(cell))
                .map(|cell| PatchId(u32::from(cell.id.get())))
                .collect::<BTreeSet<_>>();
            let mut diagnostics = vec![format!("exact corridor: {primary_diagnostic}")];
            for relief_radius in 1..=3 {
                let mut relief_allowed = allowed.clone();
                relief_allowed.extend(
                    authored_corridor
                        .iter()
                        .flat_map(|coord| coord.within_radius(relief_radius))
                        .filter(|coord| {
                            layout.footprint.contains(coord)
                                && !water_coords.contains(coord)
                                && !surface_route_exclusion.contains(coord)
                                && fine_index
                                    .patch(*coord)
                                    .is_some_and(|patch| ordinary_patches.contains(&patch))
                        }),
                );
                match resolve_peak_saddle_path(
                    start,
                    &relief_allowed,
                    &waypoint_zones,
                    &natural_by_coord,
                    water_coords,
                    &bank_minimums,
                    volume,
                    saddle_level,
                    false,
                ) {
                    Ok(path) => return Ok((path, relief_allowed)),
                    Err(diagnostic) => diagnostics.push(format!(
                        "dry Ordinary relief radius {relief_radius}: {diagnostic}"
                    )),
                }
            }

            // The immutable pass can split adjacent locked arm masks into two
            // dry components. Preserve one exact carved detour on the starting
            // side, then let the non-exiting Upper natural suffix satisfy only
            // the later waypoint cells which that pass physically occupies.
            // No additional spatial widening is permitted here.
            let separator = peak_saddle_first_component_separator(
                start,
                &allowed,
                &waypoint_zones,
                &natural_by_coord,
                water_coords,
                volume,
            )
            .ok_or_else(|| {
                format!(
                    "{}; no dry-component separator admits the suffix fallback",
                    diagnostics.join("; ")
                )
            })?;
            if separator == 0 || separator >= waypoint_zones.len() {
                return Err(format!(
                    "{}; invalid suffix-waypoint separator {separator}",
                    diagnostics.join("; ")
                ));
            }
            let mut suffix_waypoint_zones = waypoint_zones.clone();
            for (index, (fallback, pass_admitting)) in suffix_waypoint_zones
                .iter_mut()
                .zip(&pass_admitting_waypoint_zones)
                .enumerate()
                .skip(separator)
            {
                let natural_upper = pass_admitting
                    .iter()
                    .copied()
                    .filter(|coord| {
                        natural_by_coord.get(coord).is_some_and(|position| {
                            OrdinaryRegionBand::containing(position.level)
                                == OrdinaryRegionBand::Upper
                                && ordinary_surface_is_node(volume, None, *position)
                        })
                    })
                    .collect::<BTreeSet<_>>();
                if natural_upper.is_empty() {
                    return Err(format!(
                        "{}; separated waypoint {index} contains no exact dry Upper natural-pass surface",
                        diagnostics.join("; ")
                    ));
                }
                // Once the search enters its immutable suffix, later stages
                // may advance only on the pass itself. Keeping off-pass cells
                // out of these zones prevents the carved detour from silently
                // satisfying a post-separator waypoint before it rejoins.
                *fallback = natural_upper;
            }
            resolve_peak_saddle_path(
                start,
                &allowed,
                &suffix_waypoint_zones,
                &natural_by_coord,
                water_coords,
                &bank_minimums,
                volume,
                saddle_level,
                true,
            )
            .map(|path| (path, allowed.clone()))
            .map_err(|fallback_diagnostic| {
                format!(
                    "{}; suffix-waypoint fallback from stage {separator}: {fallback_diagnostic}",
                    diagnostics.join("; ")
                )
            })
        })
    .map_err(|diagnostic| {
        schematic_contract(format!(
            "peak saddle cannot leave the immutable natural pass through the frozen shore and north-east arm: {diagnostic}"
        ))
    })?;
    let path_coords = path.iter().map(|(coord, _)| *coord).collect::<Vec<_>>();
    if path_coords.len() < 2
        || path_coords
            .windows(2)
            .any(|pair| pair[0].distance(pair[1]) != 1)
    {
        return Err(schematic_contract(
            "peak-saddle construction walk is not adjacent",
        ));
    }
    if let Some(coord) = path_coords.iter().find(|coord| {
        !resolved_allowed.contains(coord)
            || !layout.footprint.contains(coord)
            || water_coords.contains(coord)
            || surface_route_exclusion.contains(coord)
    }) {
        return Err(schematic_contract(format!(
            "peak-saddle centerline leaves its resolved dry arm corridor at {coord:?}"
        )));
    }
    if let Some((coord, marked_natural)) = path
        .iter()
        .find(|(coord, marked_natural)| natural_by_coord.contains_key(coord) != *marked_natural)
    {
        return Err(schematic_contract(format!(
            "peak-saddle search misclassified natural-pass node {coord:?} as natural={marked_natural}"
        )));
    }

    let shared_prefix_len = path
        .iter()
        .take_while(|(_, marked_natural)| *marked_natural)
        .count();
    let branch_coord = path
        .get(shared_prefix_len.saturating_sub(1))
        .map(|(coord, _)| *coord)
        .ok_or_else(|| {
            schematic_contract("peak-saddle route has no exact shared natural-pass prefix")
        })?;
    let branch = natural_by_coord
        .get(&branch_coord)
        .copied()
        .ok_or_else(|| {
            schematic_contract("peak-saddle route has no exact shared natural-pass prefix")
        })?;
    if path
        .iter()
        .take(shared_prefix_len)
        .filter_map(|(coord, _)| natural_by_coord.get(coord))
        .any(|position| OrdinaryRegionBand::containing(position.level) != OrdinaryRegionBand::Upper)
    {
        return Err(schematic_contract(
            "peak-saddle shared prefix leaves the upper natural-pass band",
        ));
    }
    let suffix_start = path
        .iter()
        .enumerate()
        .skip(shared_prefix_len)
        .find_map(|(index, (_, marked_natural))| marked_natural.then_some(index))
        .ok_or_else(|| {
            schematic_contract("peak-saddle route did not rejoin its exact natural-pass suffix")
        })?;
    let carved_end = suffix_start;
    if shared_prefix_len == path.len()
        || shared_prefix_len == carved_end
        || path
            .iter()
            .skip(suffix_start)
            .any(|(_, marked_natural)| !marked_natural)
    {
        return Err(schematic_contract(
            "peak-saddle must contain one carved detour between an exact natural-pass prefix and suffix",
        ));
    }
    let rejoin = path
        .get(suffix_start)
        .and_then(|(coord, _)| natural_by_coord.get(coord))
        .copied()
        .ok_or_else(|| schematic_contract("peak-saddle exact suffix lost its rejoin node"))?;
    if OrdinaryRegionBand::containing(rejoin.level) != OrdinaryRegionBand::Upper {
        return Err(schematic_contract(
            "peak-saddle detour rejoins the natural pass outside the upper band",
        ));
    }

    let mut carved_span = Vec::with_capacity(
        carved_end
            .saturating_sub(shared_prefix_len)
            .saturating_add(2),
    );
    carved_span.push(branch.coord);
    carved_span.extend(
        path.iter()
            .skip(shared_prefix_len)
            .take(carved_end.saturating_sub(shared_prefix_len))
            .map(|(coord, _)| *coord),
    );
    carved_span.push(rejoin.coord);
    let carved_corridor = carved_span.into_iter().collect::<BTreeSet<_>>();
    let levels = graded_upper_bench_levels_with_minimums(
        &carved_corridor,
        branch,
        rejoin,
        saddle_level,
        &bank_minimums,
    )
    .map_err(|diagnostic| schematic_contract(format!("peak-saddle grading: {diagnostic}")))?;
    let mut carved_positions = BTreeMap::<HexCoord, TilePos>::new();
    for (coord, level) in levels {
        if let Some(existing) = natural_by_coord.get(&coord).copied() {
            if !matches!(existing, position if (position == branch || position == rejoin) && position.level == level)
            {
                return Err(schematic_contract(format!(
                    "peak-saddle grading attempted to mutate immutable natural-pass surface {existing:?}"
                )));
            }
            continue;
        }
        if level < UPPER_REGION_THRESHOLD.saturating_add(1) {
            return Err(schematic_contract(format!(
                "peak-saddle created a non-upper node at {:?}",
                TilePos::new(coord, level)
            )));
        }
        let biome = fine_index.biome(coord).ok_or_else(|| {
            schematic_contract(format!("peak-saddle node {coord:?} has no biome owner"))
        })?;
        let position = TilePos::new(coord, level);
        replace_column_surface(
            volume,
            biome_regions,
            coord,
            land_column(level, SolidMaterialRole::Stone),
            position,
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: None,
            },
            biome,
        );
        carved_positions.insert(coord, position);
    }
    let construction_walk = path
        .iter()
        .map(|(coord, marked_natural)| {
            if *marked_natural {
                natural_by_coord.get(coord).copied()
            } else {
                carved_positions.get(coord).copied()
            }
            .ok_or_else(|| {
                schematic_contract(format!(
                    "peak-saddle failed to publish its resolved centerline node at {coord:?}"
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if let Some((from, to)) = construction_walk.windows(2).find_map(|pair| {
        let [from, to] = pair else {
            return None;
        };
        (!ordinary_transition_is_admitted(volume, None, *from, *to)).then_some((*from, *to))
    }) {
        return Err(schematic_contract(format!(
            "peak-saddle construction walk lost exact walker edge {from:?} -> {to:?}"
        )));
    }
    validate_protected_route_integrity(
        "natural pass after peak-saddle carving",
        natural_pass,
        volume,
    )?;
    let surfaces = construction_walk.iter().copied().collect::<BTreeSet<_>>();
    let centerline = simple_reserved_spine(branch, &surfaces, volume)?;
    let anchor = centerline.get(centerline.len() / 2).copied();
    let route = ProtectedFeatureRoute {
        centerline,
        surfaces,
    };
    validate_protected_route_integrity("peak saddle", &route, volume)?;
    Ok(PeakSaddleCompilation { route, anchor })
}

/// Carves Lower-band footing through the two inner PeakRing cells that lie on
/// the far side of the natural-pass ribbon.
///
/// These cells cannot join the Upper saddle without either overwriting the
/// natural pass or introducing a third elevation-band portal. A separate
/// ledge branches once from an exact Lower natural-pass surface, remains below
/// the reserved threshold gap, and never touches the pass again.
fn compile_peak_foothill_ledge(
    plan: &SchematicPlanV1,
    layout: &ResolvedLayoutPlan,
    fine_index: &FineWorldIndex,
    water_coords: &BTreeSet<HexCoord>,
    natural_pass: &ProtectedFeatureRoute,
    upper_saddle: &ProtectedFeatureRoute,
    protected_features: &FeaturePlan,
    surface_route_exclusion: &BTreeSet<HexCoord>,
    volume: &mut VolumePlan,
    biome_regions: &mut BTreeMap<TilePos, hex_core::BiomeRegionId>,
) -> Result<PeakSaddleCompilation, V3GenerationError> {
    const INNER_FOOTHILL_ARM: [(i32, i32, i32); 2] = [(6, -5, -1), (6, -4, -2)];

    let natural_by_coord = natural_pass.surfaces.iter().copied().try_fold(
        BTreeMap::<HexCoord, TilePos>::new(),
        |mut by_coord, position| {
            if by_coord.insert(position.coord, position).is_some() {
                return Err(schematic_contract(format!(
                    "natural pass publishes stacked reserved surfaces at {:?}",
                    position.coord
                )));
            }
            Ok(by_coord)
        },
    )?;
    let protected_coords = protected_features
        .protected_routes
        .values()
        .flat_map(|route| route.surfaces.iter().map(|position| position.coord))
        .chain(upper_saddle.surfaces.iter().map(|position| position.coord))
        .collect::<BTreeSet<_>>();
    let mut waypoint_zones = Vec::with_capacity(INNER_FOOTHILL_ARM.len());
    let mut allowed = BTreeSet::new();
    for (q, r, s) in INNER_FOOTHILL_ARM {
        let coord =
            SchematicCoord::new(q, r, s).map_err(|error| schematic_contract(error.to_string()))?;
        let cell = plan.cell(coord).ok_or_else(|| {
            schematic_contract(format!(
                "peak-foothill arm is missing locked cell {coord:?}"
            ))
        })?;
        if cell.facts.surface != SurfaceKind::Land
            || cell.facts.landform != LandformKind::SharpPeak
            || cell.facts.access != AccessIntent::Ordinary
            || !has_overlay(cell, SchematicFeature::PeakRing)
        {
            return Err(schematic_contract(format!(
                "peak-foothill arm cell {} lost its locked Land/SharpPeak/Ordinary/PeakRing contract",
                cell.id.get()
            )));
        }
        let patch = layout
            .patches
            .get(&PatchId(u32::from(cell.id.get())))
            .ok_or_else(|| {
                schematic_contract(format!(
                    "peak-foothill arm cell {} has no resolved patch",
                    cell.id.get()
                ))
            })?;
        let zone = patch
            .mask
            .iter()
            .copied()
            .filter(|fine| {
                !water_coords.contains(fine)
                    && !natural_by_coord.contains_key(fine)
                    && !protected_coords.contains(fine)
                    && !surface_route_exclusion.contains(fine)
            })
            .collect::<BTreeSet<_>>();
        if zone.is_empty() {
            return Err(schematic_contract(format!(
                "peak-foothill arm cell {} has no dry coordinate outside the natural pass",
                cell.id.get()
            )));
        }
        allowed.extend(zone.iter().copied());
        waypoint_zones.push(zone);
    }

    let first_zone = waypoint_zones
        .first()
        .ok_or_else(|| schematic_contract("peak-foothill ledge has no first waypoint"))?;
    let mut branches = natural_pass
        .surfaces
        .iter()
        .copied()
        .filter(|position| {
            OrdinaryRegionBand::containing(position.level) == OrdinaryRegionBand::Lower
                && volume
                    .surfaces
                    .get(position)
                    .is_some_and(|metadata| metadata.access == SurfaceAccess::Ordinary)
        })
        .map(|position| {
            let nearest_gap = first_zone
                .iter()
                .map(|coord| coord.distance(position.coord))
                .min()
                .unwrap_or(u32::MAX);
            (nearest_gap, position)
        })
        .collect::<Vec<_>>();
    branches.sort_unstable();
    let mut branch_allowed_by_owner = BTreeMap::<PatchId, BTreeSet<HexCoord>>::new();
    let mut selected_candidate = None;
    let mut diagnostics = Vec::new();
    let mut group_start = 0;
    while group_start < branches.len() {
        let nearest_gap = branches[group_start].0;
        let group_end =
            group_start + branches[group_start..].partition_point(|(gap, _)| *gap == nearest_gap);
        let mut candidates = Vec::<(u32, usize, TilePos, Vec<HexCoord>)>::new();
        for (_, branch) in branches[group_start..group_end].iter().copied() {
            let owner = fine_index.patch(branch.coord).ok_or_else(|| {
                schematic_contract(format!(
                    "peak-foothill branch {branch:?} has no fine-grid owner"
                ))
            })?;
            let owner_cell = plan
                .cells
                .iter()
                .find(|cell| u32::from(cell.id.get()) == owner.0)
                .ok_or_else(|| {
                    schematic_contract(format!(
                        "peak-foothill branch owner {owner:?} has no schematic cell"
                    ))
                })?;
            if !is_ordinary_land(owner_cell) {
                if diagnostics.len() < 12 {
                    diagnostics.push(format!("{branch:?} owner {owner:?} is not ordinary land"));
                }
                continue;
            }
            let owner_patch = layout.patches.get(&owner).ok_or_else(|| {
                schematic_contract(format!(
                    "peak-foothill branch {branch:?} lost owner {owner:?}"
                ))
            })?;
            let branch_allowed = branch_allowed_by_owner.entry(owner).or_insert_with(|| {
                let mut branch_allowed = allowed.clone();
                branch_allowed.extend(owner_patch.mask.iter().copied().filter(|coord| {
                    !water_coords.contains(coord)
                        && !natural_by_coord.contains_key(coord)
                        && !protected_coords.contains(coord)
                        && !surface_route_exclusion.contains(coord)
                }));
                branch_allowed
            });
            let exit_count = branch
                .coord
                .neighbors()
                .into_iter()
                .filter(|neighbor| {
                    branch_allowed.contains(neighbor) && !natural_by_coord.contains_key(neighbor)
                })
                .count();
            let advance = |stage: usize, coord: HexCoord| {
                stage.saturating_add(usize::from(
                    waypoint_zones
                        .get(stage)
                        .is_some_and(|zone| zone.contains(&coord)),
                ))
            };
            let start = (branch.coord, advance(0, branch.coord));
            let mut frontier = VecDeque::from([start]);
            let mut seen = BTreeSet::from([start]);
            let mut parent = BTreeMap::<(HexCoord, usize), (HexCoord, usize)>::new();
            let mut goal = None;
            let mut furthest_stage = start.1;
            while let Some(node @ (coord, stage)) = frontier.pop_front() {
                furthest_stage = furthest_stage.max(stage);
                if stage == waypoint_zones.len() {
                    goal = Some(node);
                    break;
                }
                let mut neighbors = coord.neighbors();
                neighbors.sort_unstable();
                for neighbor in neighbors {
                    if !branch_allowed.contains(&neighbor)
                        || water_coords.contains(&neighbor)
                        || surface_route_exclusion.contains(&neighbor)
                        || (neighbor != branch.coord && natural_by_coord.contains_key(&neighbor))
                        || (neighbor == branch.coord && coord != branch.coord)
                    {
                        continue;
                    }
                    let next = (neighbor, advance(stage, neighbor));
                    if seen.insert(next) {
                        parent.insert(next, node);
                        frontier.push_back(next);
                    }
                }
            }
            let Some(mut cursor) = goal else {
                if diagnostics.len() < 12 {
                    diagnostics.push(format!(
                        "{branch:?} owner {owner:?} gap {nearest_gap} exits {exit_count} reached {furthest_stage}/{}",
                        waypoint_zones.len()
                    ));
                }
                continue;
            };
            let mut reversed = vec![cursor.0];
            while cursor != start {
                let Some(previous) = parent.get(&cursor).copied() else {
                    return Err(schematic_contract(
                        "peak-foothill search lost an exact parent state",
                    ));
                };
                cursor = previous;
                reversed.push(cursor.0);
            }
            reversed.reverse();
            candidates.push((nearest_gap, reversed.len(), branch, reversed));
        }
        if let Some(candidate) = candidates
            .into_iter()
            .min_by(|first, second| first.cmp(second))
        {
            // `nearest_gap` is the first field in the canonical selection key.
            // Once one route exists in this equal-gap group, no later group can
            // produce a candidate that outranks it.
            selected_candidate = Some(candidate);
            break;
        }
        group_start = group_end;
    }
    let (_, _, branch, path) = selected_candidate.ok_or_else(|| {
            schematic_contract(format!(
                "no exact Lower natural-pass branch can reach peak cells 92 and 93 through only its owner mask without re-entry; nearest attempts [{}]",
                diagnostics.join("; ")
            ))
        })?;
    if path.len() < 3
        || path.first().copied() != Some(branch.coord)
        || path.iter().copied().collect::<BTreeSet<_>>().len() != path.len()
        || path.windows(2).any(|pair| pair[0].distance(pair[1]) != 1)
        || path
            .iter()
            .skip(1)
            .any(|coord| natural_by_coord.contains_key(coord))
    {
        return Err(schematic_contract(
            "peak-foothill ledge is not one simple branch with no natural-pass re-entry",
        ));
    }

    let step_toward_foothill = |index: usize| {
        let steps = Level::try_from(index).unwrap_or(Level::MAX);
        if branch.level <= 20 {
            branch.level.saturating_add(steps).min(20)
        } else {
            branch.level.saturating_sub(steps).max(20)
        }
    };
    let mut centerline = Vec::with_capacity(path.len());
    centerline.push(branch);
    for (index, coord) in path.into_iter().enumerate().skip(1) {
        let level = step_toward_foothill(index);
        if !OrdinaryRegionBand::Lower.accepts_new(level) {
            return Err(schematic_contract(format!(
                "peak-foothill ledge created a non-lower node at {:?}",
                TilePos::new(coord, level)
            )));
        }
        let biome = fine_index.biome(coord).ok_or_else(|| {
            schematic_contract(format!("peak-foothill node {coord:?} has no biome owner"))
        })?;
        let position = TilePos::new(coord, level);
        replace_column_surface(
            volume,
            biome_regions,
            coord,
            land_column(level, SolidMaterialRole::Stone),
            position,
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: None,
            },
            biome,
        );
        centerline.push(position);
    }
    validate_protected_route_integrity(
        "natural pass after peak-foothill carving",
        natural_pass,
        volume,
    )?;
    validate_protected_route_integrity(
        "upper peak saddle after peak-foothill carving",
        upper_saddle,
        volume,
    )?;
    let surfaces = centerline.iter().copied().collect::<BTreeSet<_>>();
    if surfaces
        .iter()
        .any(|surface| surface_route_exclusion.contains(&surface.coord))
    {
        return Err(schematic_contract(
            "peak-foothill ledge intersects exact corrective terrain",
        ));
    }
    let anchor = centerline.get(centerline.len() / 2).copied();
    let route = ProtectedFeatureRoute {
        centerline,
        surfaces,
    };
    validate_protected_route_integrity("peak foothill ledge", &route, volume)?;
    Ok(PeakSaddleCompilation { route, anchor })
}

fn peak_saddle_waypoint_zone(
    patch: &super::layout::ResolvedPatch,
    center: HexCoord,
    natural_by_coord: &BTreeMap<HexCoord, TilePos>,
    water_coords: &BTreeSet<HexCoord>,
    surface_route_exclusion: &BTreeSet<HexCoord>,
    admits_natural_pass: bool,
) -> Result<BTreeSet<HexCoord>, V3GenerationError> {
    let zone = patch
        .mask
        .iter()
        .copied()
        .filter(|coord| {
            (admits_natural_pass || !natural_by_coord.contains_key(coord))
                && !water_coords.contains(coord)
                && !surface_route_exclusion.contains(coord)
        })
        .collect::<BTreeSet<_>>();
    if zone.is_empty() {
        return Err(schematic_contract(format!(
            "peak-saddle patch around {center:?} has no dry waypoint outside the natural pass"
        )));
    }
    Ok(zone)
}

fn resolve_peak_saddle_path(
    start: TilePos,
    allowed: &BTreeSet<HexCoord>,
    waypoint_zones: &[BTreeSet<HexCoord>],
    natural_by_coord: &BTreeMap<HexCoord, TilePos>,
    water_coords: &BTreeSet<HexCoord>,
    bank_minimums: &BTreeMap<HexCoord, Level>,
    volume: &VolumePlan,
    bench_level: Level,
    allow_suffix_waypoints: bool,
) -> Result<Vec<(HexCoord, bool)>, String> {
    type SearchNode = (HexCoord, usize, u8);
    const NATURAL_PREFIX: u8 = 0;
    const CARVED_DETOUR: u8 = 1;
    const NATURAL_SUFFIX: u8 = 2;
    if !ordinary_surface_is_node(volume, None, start) {
        return Err(format!(
            "branch start {start:?} is not an exact Ordinary walker node"
        ));
    }
    if waypoint_zones.is_empty() {
        return Err("no ordered frozen-shore/PeakRing waypoint zones were resolved".to_owned());
    }
    let advance = |stage: usize, coord: HexCoord| {
        stage.saturating_add(usize::from(
            waypoint_zones
                .get(stage)
                .is_some_and(|zone| zone.contains(&coord)),
        ))
    };
    let start_node = (start.coord, advance(0, start.coord), NATURAL_PREFIX);
    let mut frontier = VecDeque::from([start_node]);
    let mut seen = BTreeSet::from([start_node]);
    let mut parent = BTreeMap::<SearchNode, SearchNode>::new();
    let mut states_by_stage = vec![0_usize; waypoint_zones.len().saturating_add(1)];
    let mut exited_by_stage = vec![0_usize; waypoint_zones.len().saturating_add(1)];
    let mut suffix_by_stage = vec![0_usize; waypoint_zones.len().saturating_add(1)];
    let mut pass_exit_edges = 0_usize;
    let mut pass_suffix_edges = 0_usize;
    let mut blocked_water_edges = 0_usize;
    let mut blocked_outside_edges = 0_usize;
    let mut blocked_reentry_edges = 0_usize;
    let mut grading_rejections = 0_usize;
    let mut first_grading_rejection = None;
    while let Some(node @ (coord, stage, phase)) = frontier.pop_front() {
        if let Some(count) = states_by_stage.get_mut(stage) {
            *count = count.saturating_add(1);
        }
        if phase != NATURAL_PREFIX {
            if let Some(count) = exited_by_stage.get_mut(stage) {
                *count = count.saturating_add(1);
            }
        }
        if phase == NATURAL_SUFFIX {
            if let Some(count) = suffix_by_stage.get_mut(stage) {
                *count = count.saturating_add(1);
            }
        }
        if stage == waypoint_zones.len() && phase == NATURAL_SUFFIX {
            let candidate =
                reconstruct_peak_saddle_search_path(node, start_node, &parent, CARVED_DETOUR)?;
            match peak_saddle_grading_corridor(&candidate, natural_by_coord).and_then(
                |(branch, rejoin, corridor)| {
                    graded_upper_bench_levels_with_minimums(
                        &corridor,
                        branch,
                        rejoin,
                        bench_level,
                        bank_minimums,
                    )
                    .map(|_| ())
                },
            ) {
                Ok(()) => return Ok(candidate),
                Err(diagnostic) => {
                    grading_rejections = grading_rejections.saturating_add(1);
                    if first_grading_rejection.is_none() {
                        first_grading_rejection = Some(diagnostic);
                    }
                    // A suffix state fixes the exact rejoin. Walking farther on
                    // the immutable pass cannot make that junction gradeable;
                    // keep searching for a different carved rejoin instead.
                    continue;
                }
            }
        }
        let mut neighbors = coord.neighbors();
        neighbors.sort_unstable();
        for neighbor in neighbors {
            if !allowed.contains(&neighbor) {
                blocked_outside_edges = blocked_outside_edges.saturating_add(1);
                continue;
            }
            if water_coords.contains(&neighbor) {
                blocked_water_edges = blocked_water_edges.saturating_add(1);
                continue;
            }
            let natural_neighbor = natural_by_coord.get(&neighbor).copied();
            let next_phase = match (phase, natural_neighbor) {
                (NATURAL_PREFIX, Some(next)) => {
                    let Some(current) = natural_by_coord.get(&coord).copied() else {
                        return Err(format!(
                            "natural-prefix state left the natural pass at {coord:?}"
                        ));
                    };
                    if OrdinaryRegionBand::containing(next.level) != OrdinaryRegionBand::Upper
                        || !ordinary_transition_is_admitted(volume, None, current, next)
                    {
                        continue;
                    }
                    NATURAL_PREFIX
                }
                (NATURAL_PREFIX, None) => {
                    pass_exit_edges = pass_exit_edges.saturating_add(1);
                    CARVED_DETOUR
                }
                (CARVED_DETOUR, None) => CARVED_DETOUR,
                (CARVED_DETOUR, Some(next)) => {
                    if (!allow_suffix_waypoints && stage < waypoint_zones.len())
                        || OrdinaryRegionBand::containing(next.level) != OrdinaryRegionBand::Upper
                        || !ordinary_surface_is_node(volume, None, next)
                    {
                        blocked_reentry_edges = blocked_reentry_edges.saturating_add(1);
                        continue;
                    }
                    pass_suffix_edges = pass_suffix_edges.saturating_add(1);
                    NATURAL_SUFFIX
                }
                (NATURAL_SUFFIX, Some(next)) => {
                    let Some(current) = natural_by_coord.get(&coord).copied() else {
                        return Err(format!(
                            "natural-suffix state left the natural pass at {coord:?}"
                        ));
                    };
                    if OrdinaryRegionBand::containing(next.level) != OrdinaryRegionBand::Upper
                        || !ordinary_transition_is_admitted(volume, None, current, next)
                    {
                        continue;
                    }
                    NATURAL_SUFFIX
                }
                (NATURAL_SUFFIX, None) => {
                    blocked_reentry_edges = blocked_reentry_edges.saturating_add(1);
                    continue;
                }
                _ => return Err(format!("unknown peak-saddle search phase {phase}")),
            };
            let next = (neighbor, advance(stage, neighbor), next_phase);
            if seen.insert(next) {
                parent.insert(next, node);
                frontier.push_back(next);
            }
        }
    }

    let furthest_stage = states_by_stage
        .iter()
        .enumerate()
        .rev()
        .find_map(|(stage, count)| (*count > 0).then_some(stage))
        .unwrap_or_default();
    let grading_diagnostic = first_grading_rejection
        .map(|diagnostic| format!("; first grading rejection: {diagnostic}"))
        .unwrap_or_default();
    Err(format!(
        "ordered search reached stage {furthest_stage}/{}; states by stage {states_by_stage:?}; detour-or-suffix states {exited_by_stage:?}; suffix states {suffix_by_stage:?}; pass exits/merges {pass_exit_edges}/{pass_suffix_edges}; blocked edges outside/water/premature-reentry-or-reexit {blocked_outside_edges}/{blocked_water_edges}/{blocked_reentry_edges}; grade-infeasible suffixes {grading_rejections}{grading_diagnostic}",
        waypoint_zones.len()
    ))
}

/// Returns the first ordered waypoint which no reachable dry off-pass component
/// can share with the entire preceding waypoint prefix.
fn peak_saddle_first_component_separator(
    start: TilePos,
    allowed: &BTreeSet<HexCoord>,
    waypoint_zones: &[BTreeSet<HexCoord>],
    natural_by_coord: &BTreeMap<HexCoord, TilePos>,
    water_coords: &BTreeSet<HexCoord>,
    volume: &VolumePlan,
) -> Option<usize> {
    let detour = allowed
        .iter()
        .copied()
        .filter(|coord| !water_coords.contains(coord) && !natural_by_coord.contains_key(coord))
        .collect::<BTreeSet<_>>();
    let mut components = BTreeMap::<HexCoord, u32>::new();
    let mut next_component = 0_u32;
    for component_start in &detour {
        if components.contains_key(component_start) {
            continue;
        }
        components.insert(*component_start, next_component);
        let mut frontier = VecDeque::from([*component_start]);
        while let Some(coord) = frontier.pop_front() {
            for neighbor in coord.neighbors() {
                if detour.contains(&neighbor) && !components.contains_key(&neighbor) {
                    components.insert(neighbor, next_component);
                    frontier.push_back(neighbor);
                }
            }
        }
        next_component = next_component.saturating_add(1);
    }
    let mut reachable_natural = BTreeSet::new();
    let mut natural_frontier = VecDeque::from([start]);
    while let Some(position) = natural_frontier.pop_front() {
        if !allowed.contains(&position.coord)
            || natural_by_coord.get(&position.coord) != Some(&position)
            || OrdinaryRegionBand::containing(position.level) != OrdinaryRegionBand::Upper
            || !reachable_natural.insert(position.coord)
        {
            continue;
        }
        let mut neighbors = position.coord.neighbors();
        neighbors.sort_unstable();
        natural_frontier.extend(neighbors.into_iter().filter_map(|neighbor| {
            let next = natural_by_coord.get(&neighbor).copied()?;
            ordinary_transition_is_admitted(volume, None, position, next).then_some(next)
        }));
    }
    let reachable_components = reachable_natural
        .iter()
        .flat_map(|coord| coord.neighbors())
        .filter_map(|neighbor| components.get(&neighbor).copied())
        .collect::<BTreeSet<_>>();
    let mut waypoint_components = waypoint_zones.iter().map(|zone| {
        zone.iter()
            .filter_map(|coord| components.get(coord).copied())
            .collect::<BTreeSet<_>>()
    });
    let mut viable = waypoint_components.next()?;
    viable.retain(|component| reachable_components.contains(component));
    if viable.is_empty() {
        return Some(0);
    }
    for (index, components) in waypoint_components.enumerate() {
        viable.retain(|component| components.contains(component));
        if viable.is_empty() {
            return Some(index.saturating_add(1));
        }
    }
    None
}

fn reconstruct_peak_saddle_search_path(
    mut cursor: (HexCoord, usize, u8),
    start: (HexCoord, usize, u8),
    parent: &BTreeMap<(HexCoord, usize, u8), (HexCoord, usize, u8)>,
    carved_phase: u8,
) -> Result<Vec<(HexCoord, bool)>, String> {
    let mut reversed = vec![(cursor.0, cursor.2 != carved_phase)];
    while cursor != start {
        let Some(previous) = parent.get(&cursor).copied() else {
            return Err(format!(
                "ordered saddle search lost the parent of state {cursor:?}"
            ));
        };
        cursor = previous;
        reversed.push((cursor.0, cursor.2 != carved_phase));
    }
    reversed.reverse();
    Ok(reversed)
}

fn peak_saddle_grading_corridor(
    path: &[(HexCoord, bool)],
    natural_by_coord: &BTreeMap<HexCoord, TilePos>,
) -> Result<(TilePos, TilePos, BTreeSet<HexCoord>), String> {
    let shared_prefix_len = path
        .iter()
        .take_while(|(_, marked_natural)| *marked_natural)
        .count();
    let branch_coord = path
        .get(shared_prefix_len.saturating_sub(1))
        .map(|(coord, _)| *coord)
        .ok_or_else(|| "candidate has no exact shared natural-pass prefix".to_owned())?;
    let branch = natural_by_coord
        .get(&branch_coord)
        .copied()
        .ok_or_else(|| "candidate prefix does not end on the exact natural pass".to_owned())?;
    let suffix_start = path
        .iter()
        .enumerate()
        .skip(shared_prefix_len)
        .find_map(|(index, (_, marked_natural))| marked_natural.then_some(index))
        .ok_or_else(|| "candidate has no exact natural-pass rejoin".to_owned())?;
    if shared_prefix_len == path.len()
        || shared_prefix_len == suffix_start
        || path
            .iter()
            .skip(suffix_start)
            .any(|(_, marked_natural)| !marked_natural)
    {
        return Err("candidate is not one carved detour between exact pass spans".to_owned());
    }
    let rejoin = path
        .get(suffix_start)
        .and_then(|(coord, _)| natural_by_coord.get(coord))
        .copied()
        .ok_or_else(|| "candidate lost its exact natural-pass rejoin".to_owned())?;
    let corridor = std::iter::once(branch.coord)
        .chain(
            path.iter()
                .skip(shared_prefix_len)
                .take(suffix_start.saturating_sub(shared_prefix_len))
                .map(|(coord, _)| *coord),
        )
        .chain(std::iter::once(rejoin.coord))
        .collect::<BTreeSet<_>>();
    Ok((branch, rejoin, corridor))
}

fn validate_protected_route_integrity(
    label: &str,
    route: &ProtectedFeatureRoute,
    volume: &VolumePlan,
) -> Result<(), V3GenerationError> {
    if let Some(position) = route.surfaces.iter().find(|position| {
        volume
            .surfaces
            .get(position)
            .is_none_or(|metadata| metadata.access != SurfaceAccess::Ordinary)
    }) {
        return Err(schematic_contract(format!(
            "{label} lost exact Ordinary surface {position:?}"
        )));
    }
    if let Some(position) = route.centerline.iter().find(|position| {
        !route.surfaces.contains(position) || !ordinary_surface_is_node(volume, None, **position)
    }) {
        return Err(schematic_contract(format!(
            "{label} centerline lost exact walker node {position:?}"
        )));
    }
    if let Some((from, to)) = route.centerline.windows(2).find_map(|pair| {
        let [from, to] = pair else {
            return None;
        };
        (!ordinary_transition_is_admitted(volume, None, *from, *to)).then_some((*from, *to))
    }) {
        return Err(schematic_contract(format!(
            "{label} centerline lost walker edge {from:?} -> {to:?}"
        )));
    }
    Ok(())
}

/// Reduces an adjacent construction walk's connected reserved footprint to a
/// deterministic simple gameplay spine. The farthest-node tie break is the
/// canonical exact position, so publication is independent of traversal queue
/// history while every construction surface remains protected separately.
fn simple_reserved_spine(
    junction: TilePos,
    surfaces: &BTreeSet<TilePos>,
    volume: &VolumePlan,
) -> Result<Vec<TilePos>, V3GenerationError> {
    if !surfaces.contains(&junction) {
        return Err(schematic_contract(format!(
            "reserved route spine lost its exact junction {junction:?}"
        )));
    }
    if !ordinary_surface_is_node(volume, None, junction) {
        return Err(schematic_contract(format!(
            "reserved route junction {junction:?} is not an Ordinary walker node"
        )));
    }
    let mut frontier = VecDeque::from([junction]);
    let mut distances = BTreeMap::from([(junction, 0_u32)]);
    let mut parent = BTreeMap::<TilePos, TilePos>::new();
    let positions_by_coord = surfaces.iter().copied().fold(
        BTreeMap::<HexCoord, Vec<TilePos>>::new(),
        |mut by_coord, position| {
            by_coord.entry(position.coord).or_default().push(position);
            by_coord
        },
    );
    while let Some(position) = frontier.pop_front() {
        let distance = distances.get(&position).copied().unwrap_or_default();
        let mut neighbors = position
            .coord
            .neighbors()
            .into_iter()
            .flat_map(|coord| positions_by_coord.get(&coord).into_iter().flatten())
            .copied()
            .filter(|neighbor| ordinary_transition_is_admitted(volume, None, position, *neighbor))
            .collect::<Vec<_>>();
        neighbors.sort_unstable();
        for neighbor in neighbors {
            if distances.contains_key(&neighbor) {
                continue;
            }
            distances.insert(neighbor, distance.saturating_add(1));
            parent.insert(neighbor, position);
            frontier.push_back(neighbor);
        }
    }
    if distances.len() != surfaces.len() {
        let unreachable = surfaces
            .iter()
            .find(|position| !distances.contains_key(position))
            .copied();
        return Err(schematic_contract(format!(
            "reserved route footprint is disconnected from {junction:?}; first unreachable {unreachable:?}"
        )));
    }
    let mut farthest = junction;
    let mut farthest_distance = 0_u32;
    for (position, distance) in &distances {
        if *distance > farthest_distance || (*distance == farthest_distance && *position > farthest)
        {
            farthest = *position;
            farthest_distance = *distance;
        }
    }
    let mut reversed = vec![farthest];
    let mut cursor = farthest;
    while cursor != junction {
        cursor = parent.get(&cursor).copied().ok_or_else(|| {
            schematic_contract(format!(
                "reserved route spine lost the parent of {cursor:?}"
            ))
        })?;
        reversed.push(cursor);
    }
    reversed.reverse();
    Ok(reversed)
}

/// Finds a path by the authored pass priority: lowest possible high point,
/// shortest route at that high point, then canonical coordinate order.
fn minimax_surface_path(
    start: HexCoord,
    target: HexCoord,
    levels: &BTreeMap<HexCoord, Level>,
) -> Option<Vec<HexCoord>> {
    let start_level = *levels.get(&start)?;
    levels.get(&target)?;
    // First resolve the globally minimal elevation ceiling. Keeping only one
    // locally best `(maximum, length)` prefix is incorrect: a later unavoidable
    // high saddle can equalize two prefixes, after which total length and
    // canonical path order decide the authored route.
    let mut best_ceiling = BTreeMap::from([(start, start_level)]);
    let mut frontier = BinaryHeap::from([Reverse((start_level, start))]);
    let ceiling = loop {
        let Reverse((maximum, coord)) = frontier.pop()?;
        if best_ceiling
            .get(&coord)
            .is_none_or(|current| *current != maximum)
        {
            continue;
        }
        if coord == target {
            break maximum;
        }
        for neighbor in coord.neighbors() {
            let Some(level) = levels.get(&neighbor).copied() else {
                continue;
            };
            let candidate = maximum.max(level);
            if best_ceiling
                .get(&neighbor)
                .is_none_or(|current| candidate < *current)
            {
                best_ceiling.insert(neighbor, candidate);
                frontier.push(Reverse((candidate, neighbor)));
            }
        }
    };

    // At that exact ceiling, ordinary BFS supplies the shortest route. Sorted
    // neighbors make the first discovered path at each depth the canonical
    // lexicographically smallest one without cloning complete paths per node.
    let mut queue = VecDeque::from([start]);
    let mut reached = BTreeSet::from([start]);
    let mut parent = BTreeMap::<HexCoord, HexCoord>::new();
    while let Some(coord) = queue.pop_front() {
        if coord == target {
            break;
        }
        let mut neighbors = coord.neighbors();
        neighbors.sort_unstable();
        for neighbor in neighbors {
            if levels.get(&neighbor).is_none_or(|level| *level > ceiling)
                || !reached.insert(neighbor)
            {
                continue;
            }
            parent.insert(neighbor, coord);
            queue.push_back(neighbor);
        }
    }
    if !reached.contains(&target) {
        return None;
    }
    let mut reversed = vec![target];
    let mut cursor = target;
    while cursor != start {
        cursor = *parent.get(&cursor)?;
        reversed.push(cursor);
    }
    reversed.reverse();
    Some(reversed)
}

/// Pins exact natural junctions plus one feasible authored maximum, then uses
/// the maximum of their descending distance cones (and the Upper-band floor)
/// as a deterministic one-Lipschitz integer extension over the unique graph.
#[cfg(test)]
fn graded_upper_bench_levels(
    corridor: &BTreeSet<HexCoord>,
    branch: TilePos,
    rejoin: TilePos,
    bench_level: Level,
) -> Result<BTreeMap<HexCoord, Level>, String> {
    graded_upper_bench_levels_with_minimums(corridor, branch, rejoin, bench_level, &BTreeMap::new())
}

fn graded_upper_bench_levels_with_minimums(
    corridor: &BTreeSet<HexCoord>,
    branch: TilePos,
    rejoin: TilePos,
    bench_level: Level,
    minimums: &BTreeMap<HexCoord, Level>,
) -> Result<BTreeMap<HexCoord, Level>, String> {
    let upper_floor = UPPER_REGION_THRESHOLD.saturating_add(1);
    if !corridor.contains(&branch.coord) || !corridor.contains(&rejoin.coord) {
        return Err("corridor does not contain both exact natural junctions".to_owned());
    }
    if OrdinaryRegionBand::containing(branch.level) != OrdinaryRegionBand::Upper
        || OrdinaryRegionBand::containing(rejoin.level) != OrdinaryRegionBand::Upper
        || bench_level < upper_floor
    {
        return Err(format!(
            "Upper field pins violate the natural/new-node bands around {upper_floor}: branch/rejoin/bench {}/{}/{}",
            branch.level, rejoin.level, bench_level
        ));
    }
    if branch.level > bench_level || rejoin.level > bench_level {
        return Err(format!(
            "authored bench {bench_level} is below exact junction levels {}/{}",
            branch.level, rejoin.level
        ));
    }
    if branch.coord == rejoin.coord && branch.level != rejoin.level {
        return Err("one exact junction coordinate requested two levels".to_owned());
    }

    let from_branch = corridor_distances(corridor, branch.coord);
    let from_rejoin = corridor_distances(corridor, rejoin.coord);
    if from_branch.len() != corridor.len() || from_rejoin.len() != corridor.len() {
        return Err("unique construction corridor is disconnected".to_owned());
    }
    if from_branch
        .get(&rejoin.coord)
        .copied()
        .is_none_or(|distance| distance < branch.level.abs_diff(rejoin.level))
    {
        return Err(format!(
            "exact junction levels {} and {} are not one-Lipschitz across the unique corridor",
            branch.level, rejoin.level
        ));
    }

    let branch_rise = branch.level.abs_diff(bench_level);
    let rejoin_rise = rejoin.level.abs_diff(bench_level);
    let bench = corridor
        .iter()
        .copied()
        .filter(|coord| *coord != branch.coord && *coord != rejoin.coord)
        .find(|coord| {
            from_branch
                .get(coord)
                .copied()
                .is_some_and(|distance| distance >= branch_rise)
                && from_rejoin
                    .get(coord)
                    .copied()
                    .is_some_and(|distance| distance >= rejoin_rise)
        })
        .ok_or_else(|| {
            format!(
                "no unique off-pass coordinate can reach authored bench {bench_level} from exact junction levels {}/{}",
                branch.level, rejoin.level
            )
        })?;

    let fixed = BTreeMap::from([
        (branch.coord, branch.level),
        (rejoin.coord, rejoin.level),
        (bench, bench_level),
    ]);
    let fixed_distances = fixed
        .keys()
        .copied()
        .map(|coord| (coord, corridor_distances(corridor, coord)))
        .collect::<BTreeMap<_, _>>();
    let relevant_minimums = minimums
        .iter()
        .filter(|(coord, _)| corridor.contains(coord))
        .map(|(coord, level)| (*coord, *level))
        .collect::<BTreeMap<_, _>>();
    if let Some((coord, minimum)) = relevant_minimums
        .iter()
        .find(|(_, minimum)| **minimum > bench_level)
    {
        return Err(format!(
            "corridor minimum {minimum} at {coord:?} exceeds authored bench {bench_level}"
        ));
    }
    let minimum_distances = relevant_minimums
        .keys()
        .copied()
        .map(|coord| (coord, corridor_distances(corridor, coord)))
        .collect::<BTreeMap<_, _>>();
    for (first, first_level) in &fixed {
        for (second, second_level) in &fixed {
            let distance = fixed_distances
                .get(first)
                .and_then(|distances| distances.get(second))
                .copied()
                .ok_or_else(|| format!("fixed bench pin {first:?} cannot reach {second:?}"))?;
            if first_level.abs_diff(*second_level) > distance {
                return Err(format!(
                    "fixed bench pins {first:?}@{first_level} and {second:?}@{second_level} violate one-Lipschitz distance {distance}"
                ));
            }
        }
    }

    let mut levels = BTreeMap::new();
    for coord in corridor {
        let is_exact_junction = *coord == branch.coord || *coord == rejoin.coord;
        let mut lower = if is_exact_junction {
            Level::MIN
        } else {
            upper_floor
        };
        let mut upper = bench_level;
        for (fixed_coord, fixed_level) in &fixed {
            let distance = fixed_distances
                .get(fixed_coord)
                .and_then(|distances| distances.get(coord))
                .copied()
                .ok_or_else(|| format!("bench field cannot reach corridor coordinate {coord:?}"))?;
            let distance = Level::try_from(distance).unwrap_or(Level::MAX);
            lower = lower.max(fixed_level.saturating_sub(distance));
            upper = upper.min(fixed_level.saturating_add(distance));
        }
        for (minimum_coord, minimum_level) in &relevant_minimums {
            let distance = minimum_distances
                .get(minimum_coord)
                .and_then(|distances| distances.get(coord))
                .copied()
                .ok_or_else(|| {
                    format!("minimum field cannot reach corridor coordinate {coord:?}")
                })?;
            let distance = Level::try_from(distance).unwrap_or(Level::MAX);
            lower = lower.max(minimum_level.saturating_sub(distance));
        }
        if lower > upper {
            return Err(format!(
                "bench envelopes conflict at {coord:?}: lower {lower}, upper {upper}"
            ));
        }
        levels.insert(*coord, lower);
    }
    if levels.get(&branch.coord).copied() != Some(branch.level)
        || levels.get(&rejoin.coord).copied() != Some(rejoin.level)
        || levels.get(&bench).copied() != Some(bench_level)
        || levels.values().copied().max() != Some(bench_level)
        || relevant_minimums
            .iter()
            .any(|(coord, minimum)| levels.get(coord).is_none_or(|level| level < minimum))
        || levels.iter().any(|(coord, level)| {
            *coord != branch.coord && *coord != rejoin.coord && *level < upper_floor
        })
    {
        return Err("bench field lost an exact pin, maximum, or Upper floor".to_owned());
    }
    if let Some((coord, neighbor)) = corridor.iter().find_map(|coord| {
        coord.neighbors().into_iter().find_map(|neighbor| {
            corridor
                .contains(&neighbor)
                .then_some(neighbor)
                .filter(|neighbor| levels[coord].abs_diff(levels[neighbor]) > 1)
                .map(|neighbor| (*coord, neighbor))
        })
    }) {
        return Err(format!(
            "bench field is not one-Lipschitz across {coord:?} -> {neighbor:?}"
        ));
    }
    Ok(levels)
}

/// Extends two exact endpoint heights across a connected corridor as a
/// deterministic one-Lipschitz integer field. The lower and upper envelopes are
/// each one-Lipschitz; their midpoint therefore changes by at most one level
/// across an edge while preserving both endpoints exactly.
fn graded_corridor_levels(
    corridor: &BTreeSet<HexCoord>,
    start: TilePos,
    target: TilePos,
) -> Option<BTreeMap<HexCoord, Level>> {
    let from_start = corridor_distances(corridor, start.coord);
    let from_target = corridor_distances(corridor, target.coord);
    let required = start.level.abs_diff(target.level);
    if from_start.get(&target.coord).copied()? < required
        || from_start.len() != corridor.len()
        || from_target.len() != corridor.len()
    {
        return None;
    }

    corridor
        .iter()
        .copied()
        .map(|coord| {
            let from_start = Level::try_from(*from_start.get(&coord)?).ok()?;
            let from_target = Level::try_from(*from_target.get(&coord)?).ok()?;
            let (lower, upper) = if start.level <= target.level {
                (
                    target.level.saturating_sub(from_target).max(start.level),
                    start.level.saturating_add(from_start).min(target.level),
                )
            } else {
                (
                    start.level.saturating_sub(from_start).max(target.level),
                    target.level.saturating_add(from_target).min(start.level),
                )
            };
            (lower <= upper)
                .then_some((coord, lower.saturating_add(upper.saturating_sub(lower) / 2)))
        })
        .collect()
}

/// Lifts the existing deterministic endpoint grade by the least
/// one-Lipschitz field implied by exact per-coordinate minimums.
///
/// Each minimum contributes a descending distance cone. The maximum of the
/// original one-Lipschitz grade and all such cones is still one-Lipschitz, so a
/// high mountain-lake shoulder raises its approach gradually instead of being
/// post-raised into a cliff. Endpoint equality is checked after propagation;
/// an impossible corridor fails before any terrain is mutated.
fn graded_corridor_levels_with_minimums(
    corridor: &BTreeSet<HexCoord>,
    start: TilePos,
    target: TilePos,
    minimums: &BTreeMap<HexCoord, Level>,
) -> Option<BTreeMap<HexCoord, Level>> {
    let mut levels = graded_corridor_levels(corridor, start, target)?;
    for (minimum_coord, minimum) in minimums {
        if !corridor.contains(minimum_coord) {
            continue;
        }
        let distances = corridor_distances(corridor, *minimum_coord);
        if distances.len() != corridor.len() {
            return None;
        }
        for (coord, distance) in distances {
            let distance = Level::try_from(distance).ok()?;
            let implied = minimum.saturating_sub(distance);
            let level = levels.get_mut(&coord)?;
            *level = (*level).max(implied);
        }
    }
    if levels.get(&start.coord).copied() != Some(start.level)
        || levels.get(&target.coord).copied() != Some(target.level)
        || minimums
            .iter()
            .filter(|(coord, _)| corridor.contains(coord))
            .any(|(coord, minimum)| levels.get(coord).is_none_or(|level| level < minimum))
        || levels.values().any(|level| *level > MAX_V3_LEVEL)
        || corridor.iter().any(|coord| {
            coord.neighbors().into_iter().any(|neighbor| {
                corridor.contains(&neighbor)
                    && levels
                        .get(coord)
                        .zip(levels.get(&neighbor))
                        .is_none_or(|(level, other)| level.abs_diff(*other) > 1)
            })
        })
    {
        return None;
    }
    Some(levels)
}

fn corridor_distances(corridor: &BTreeSet<HexCoord>, start: HexCoord) -> BTreeMap<HexCoord, u32> {
    if !corridor.contains(&start) {
        return BTreeMap::new();
    }
    let mut distances = BTreeMap::from([(start, 0_u32)]);
    let mut frontier = VecDeque::from([start]);
    while let Some(coord) = frontier.pop_front() {
        let distance = distances.get(&coord).copied().unwrap_or_default();
        let mut neighbors = coord.neighbors();
        neighbors.sort_unstable();
        for neighbor in neighbors {
            if corridor.contains(&neighbor) && !distances.contains_key(&neighbor) {
                distances.insert(neighbor, distance.saturating_add(1));
                frontier.push_back(neighbor);
            }
        }
    }
    distances
}

fn top_standable_surface(volume: &VolumePlan, coord: HexCoord) -> Option<TilePos> {
    volume
        .surfaces_at_coord(coord)
        .rev()
        .find_map(|(position, metadata)| {
            (metadata.access != SurfaceAccess::NonStandable).then_some(*position)
        })
}

fn validate_crystal_mantle_screen_caps(
    stage: &str,
    volume: &VolumePlan,
    screen: &BTreeSet<HexCoord>,
) -> Result<(), V3GenerationError> {
    let breach = screen
        .iter()
        .find_map(|coord| match volume.top_surface_at_coord(*coord) {
            Some((surface, _))
                if surface.level <= super::schematic_highlands::CRYSTAL_ARCHITECTURE_TOP =>
            {
                Some(surface)
            }
            Some(_) => None,
            None => Some(TilePos::new(*coord, Level::MIN)),
        });
    if let Some(surface) = breach {
        return Err(schematic_contract(format!(
            "{stage} breached the Crystal mantle inner screen at {surface:?}"
        )));
    }
    Ok(())
}

fn exact_authored_surface_owners(
    world: &GeneratedWorldPlan,
) -> BTreeMap<TilePos, BTreeSet<String>> {
    let mut owners = BTreeMap::<TilePos, BTreeSet<String>>::new();
    let mut record = |position: TilePos, owner: String| {
        owners.entry(position).or_default().insert(owner);
    };
    for (name, route) in &world.features.protected_routes {
        for surface in route.surfaces.iter().chain(&route.centerline) {
            record(*surface, format!("route:{name}"));
        }
    }
    for (name, clearing) in &world.features.clearings {
        for surface in &clearing.surfaces {
            record(*surface, format!("clearing:{name}"));
        }
    }
    for (id, structure) in &world.structures.by_id {
        for voxel in &structure.voxels {
            record(*voxel, format!("structure:{id:?}"));
        }
    }
    for blocker in &world.blockers {
        record(*blocker, "blocker".to_owned());
    }
    for (id, interior) in &world.interiors.by_id {
        for surface in interior.floors.iter().chain(&interior.entrances) {
            record(*surface, format!("interior:{id:?}"));
        }
        for voxel in &interior.roof_voxels {
            record(*voxel, format!("interior-roof:{id:?}"));
        }
    }
    for (id, light) in &world.lights {
        record(light.origin, format!("light:{id:?}"));
    }
    for (name, position) in &world.anchors {
        record(*position, format!("anchor:{name}"));
    }
    owners
}

/// Raises only the exposed cap mass while retaining every lower solid, air,
/// interior, and authored stacked surface in the column.
fn raise_top_bank_surface_preserving_stacks(
    volume: &mut VolumePlan,
    biome_regions: &mut BTreeMap<TilePos, hex_core::BiomeRegionId>,
    coord: HexCoord,
    current: TilePos,
    required: Level,
    fallback_biome: Option<hex_core::BiomeRegionId>,
) -> Result<TilePos, V3GenerationError> {
    if current.coord != coord || current.level >= required {
        return Ok(current);
    }
    let metadata = volume.surfaces.get(&current).copied().ok_or_else(|| {
        schematic_contract(format!(
            "water-bank cap {current:?} disappeared before stack-preserving raise"
        ))
    })?;
    let biome = biome_regions
        .get(&current)
        .copied()
        .or(fallback_biome)
        .unwrap_or_default();
    let old_top = current.level.saturating_add(1);
    let new_top = required.saturating_add(1);
    let column = volume.columns.get(&coord).ok_or_else(|| {
        schematic_contract(format!("water-bank cap {current:?} has no semantic column"))
    })?;
    let cap_index = column
        .elements
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, element)| match element {
            VolumeElement::Solid(mass) if mass.levels.top == old_top => Some(index),
            VolumeElement::Solid(_) | VolumeElement::Fill(_) => None,
        })
        .ok_or_else(|| {
            schematic_contract(format!(
                "water-bank surface {current:?} is not the top of one exact solid run"
            ))
        })?;
    if column
        .elements
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != cap_index)
        .map(|(_, element)| match element {
            VolumeElement::Solid(mass) => mass.levels,
            VolumeElement::Fill(fill) => fill.levels,
        })
        .any(|levels| levels.bottom < new_top && old_top < levels.top)
    {
        return Err(schematic_contract(format!(
            "raising water-bank surface {current:?} to {required} would intersect a stacked authored run"
        )));
    }
    let column = volume.columns.get_mut(&coord).ok_or_else(|| {
        schematic_contract(format!(
            "water-bank column {coord:?} disappeared before its checked cap mutation"
        ))
    })?;
    let element = column.elements.get_mut(cap_index).ok_or_else(|| {
        schematic_contract(format!(
            "water-bank cap index {cap_index} disappeared from column {coord:?} before mutation"
        ))
    })?;
    let VolumeElement::Solid(cap) = element else {
        return Err(schematic_contract(format!(
            "water-bank cap index {cap_index} in column {coord:?} changed from solid before mutation"
        )));
    };
    if cap.levels.top != old_top {
        return Err(schematic_contract(format!(
            "water-bank cap index {cap_index} in column {coord:?} changed top from {old_top} to {} before mutation",
            cap.levels.top
        )));
    }
    cap.levels.top = new_top;
    let _removed_surface = volume.surfaces.remove(&current);
    let _removed_biome = biome_regions.remove(&current);
    let raised = TilePos::new(coord, required);
    volume.surfaces.insert(raised, metadata);
    biome_regions.insert(raised, biome);
    Ok(raised)
}

/// Constructs one narrow, deterministic safety network through each schematic
/// cell authored for Ordinary access.
///
/// Existing walker-connected terrain remains untouched. A disconnected cell is
/// attached to the closest proven surface in the same lower/upper elevation
/// band. New columns keep their existing cap material and biome ownership, but
/// are graded to one-level steps. The strict 119/122 construction bands prevent
/// these repairs from becoming a third lower-to-upper entrance; only the
/// already-authored natural pass and Crystal route may cross level 121.
fn compile_ordinary_hub_network(
    plan: &SchematicPlanV1,
    fine_index: &FineWorldIndex,
    water_coords: &BTreeSet<HexCoord>,
    bridges: &BridgeCompilation,
    crystal_mask: &BTreeSet<HexCoord>,
    connector_route_exclusion: &BTreeSet<HexCoord>,
    world: &mut GeneratedWorldPlan,
) -> Result<OrdinaryNetworkCompilation, V3GenerationError> {
    let profile_started = std::time::Instant::now();
    let mut profile_previous = profile_started;
    let root = world
        .anchors
        .get("grand_v3.tunnel_mouth")
        .copied()
        .ok_or_else(|| schematic_contract("ordinary hub graph has no foothill root"))?;
    let upper_root = world
        .anchors
        .get("crystal_ascent.upper_exit")
        .copied()
        .ok_or_else(|| schematic_contract("ordinary hub graph has no upper-region root"))?;
    let ordinary_cells = plan
        .cells
        .iter()
        .filter(|cell| is_ordinary_land(cell))
        .collect::<Vec<_>>();
    let ordinary_mask = ordinary_cells
        .iter()
        .filter_map(|cell| world.layout.patches.get(&PatchId(u32::from(cell.id.get()))))
        .flat_map(|patch| patch.mask.iter().copied())
        .collect::<BTreeSet<_>>();
    let mut hard_forbidden = water_coords.clone();
    hard_forbidden.extend(crystal_mask.iter().copied());
    hard_forbidden.extend(connector_route_exclusion.iter().copied());
    hard_forbidden.extend(
        world
            .observation_anchors
            .values()
            .map(|position| position.coord),
    );
    hard_forbidden.extend(world.blockers.iter().map(|position| position.coord));
    hard_forbidden.extend(
        world
            .structures
            .by_id
            .values()
            .flat_map(|structure| structure.voxels.iter().map(|voxel| voxel.coord)),
    );
    hard_forbidden.extend(
        world
            .features
            .protected_routes
            .values()
            .flat_map(|route| route.surfaces.iter().map(|surface| surface.coord)),
    );
    let exact_authored_owners = exact_authored_surface_owners(world);
    let water_bank_minimums = world
        .liquids
        .bodies
        .values()
        .flat_map(|body| body.nodes.keys().copied())
        .flat_map(|water| {
            water
                .coord
                .neighbors()
                .into_iter()
                .map(move |coord| (coord, water.level.saturating_add(1)))
        })
        // Protection is exact-TilePos authority, not blanket coordinate
        // authority. A tunnel floor or other lower authored run may share a
        // column with an unrelated exposed terrain cap that still needs to be
        // raised above water.
        .filter(|(coord, _)| !water_coords.contains(coord))
        .fold(
            BTreeMap::<HexCoord, Level>::new(),
            |mut result, (coord, minimum)| {
                result
                    .entry(coord)
                    .and_modify(|current| *current = (*current).max(minimum))
                    .or_insert(minimum);
                result
            },
        );
    // Bank normalization used to call `top_standable_surface` here, which
    // scanned the complete 105k-surface world once per bank coordinate. Keep
    // the exact same top-surface selection in one coordinate index instead.
    let mut surface_by_coord = top_standable_surfaces_by_coord(&world.volume);
    for (coord, minimum) in &water_bank_minimums {
        let Some(current) = surface_by_coord.get(coord).copied() else {
            continue;
        };
        if current.level >= *minimum {
            continue;
        }
        if crystal_mask.contains(coord) {
            return Err(schematic_contract(format!(
                "Crystal-authored water-bank surface {current:?} is below required level {minimum}"
            )));
        }
        if let Some(owners) = exact_authored_owners.get(&current) {
            return Err(schematic_contract(format!(
                "authored water-bank surface {current:?} owned by {owners:?} is below required level {minimum}; its author must select non-coplanar geometry"
            )));
        }
        let raised = raise_top_bank_surface_preserving_stacks(
            &mut world.volume,
            &mut world.biome_regions,
            *coord,
            current,
            *minimum,
            fine_index.biome(*coord),
        )?;
        surface_by_coord.insert(*coord, raised);
    }
    let immutable_water_banks = water_bank_minimums.keys().copied().collect::<BTreeSet<_>>();
    // Preserve the reason for every pre-existing exclusion before water banks
    // are added to the general connector reservation. A coordinate belonging
    // to both sets remains authored-forbidden instead of being admitted by the
    // narrow, already-walkable water-bank exception.
    let bridge_non_bank_forbidden = hard_forbidden
        .iter()
        .copied()
        .chain(exact_authored_owners.keys().map(|surface| surface.coord))
        .collect::<BTreeSet<_>>();
    // Exact authored ownership must also constrain the primary connector
    // search, not only bridge-contact selection and the bridge-apron fallback.
    // Unambiguous exact protected-route surfaces are re-admitted below through
    // `connector_preserved`, at their published levels and without mutation.
    hard_forbidden.extend(exact_authored_owners.keys().map(|surface| surface.coord));
    hard_forbidden.extend(immutable_water_banks.iter().copied());
    grand_profile_checkpoint(
        "ordinary / indices and bank normalization",
        profile_started,
        &mut profile_previous,
    );

    let bridge_approaches = bridge_bank_approaches(
        bridges,
        &world.volume,
        Some(&world.blockers),
        &ordinary_mask,
        &bridge_non_bank_forbidden,
        &immutable_water_banks,
        &surface_by_coord,
    )?;
    if let Some(approach) = bridge_approaches.iter().find(|approach| {
        OrdinaryRegionBand::containing(approach.surface.level) != OrdinaryRegionBand::Lower
    }) {
        return Err(schematic_contract(format!(
            "bridge {:?} bank {} lane {} leaves the lower traversal band at {:?}",
            approach.structure, approach.bank_index, approach.lane_index, approach.surface
        )));
    }
    if bridge_approaches
        .iter()
        .map(|approach| approach.surface.coord)
        .collect::<BTreeSet<_>>()
        .len()
        != bridge_approaches.len()
    {
        return Err(schematic_contract(
            "bridge banks do not own distinct dry approach columns",
        ));
    }
    for approach in &bridge_approaches {
        let Some(existing) = surface_by_coord.get(&approach.surface.coord).copied() else {
            return Err(schematic_contract(format!(
                "bridge approach {:?} has no terrain surface to grade",
                approach.surface
            )));
        };
        if existing == approach.surface {
            continue;
        }
        let metadata = world
            .volume
            .surfaces
            .get(&existing)
            .copied()
            .ok_or_else(|| {
                schematic_contract(format!(
                    "bridge approach source {existing:?} disappeared before grading"
                ))
            })?;
        let biome = world
            .biome_regions
            .get(&existing)
            .copied()
            .or_else(|| fine_index.biome(existing.coord))
            .unwrap_or_default();
        let material = world
            .volume
            .columns
            .get(&existing.coord)
            .map(top_solid_material)
            .unwrap_or(SolidMaterialRole::Gravel);
        replace_column_surface(
            &mut world.volume,
            &mut world.biome_regions,
            existing.coord,
            land_column(approach.surface.level, material),
            approach.surface,
            metadata,
            biome,
        );
        surface_by_coord.insert(existing.coord, approach.surface);
    }
    let bridge_approach_authority =
        exact_bridge_approach_authority(&bridge_approaches, &surface_by_coord)?;
    // All exact contacts are immutable as a set. Required connectors are
    // processed sequentially below, so reserving only the current endpoint
    // would let an earlier lane grade a later lane out from under it.
    hard_forbidden.extend(bridge_approach_authority.keys().copied());
    let bridge_apron_no_touch = bridge_non_bank_forbidden
        .iter()
        .copied()
        .chain(immutable_water_banks.iter().copied())
        .chain(bridge_approach_authority.keys().copied())
        .collect::<BTreeSet<_>>();

    // Bank normalization replaces exact surface identities. Refresh every
    // graph-derived projection before reserving paths so the protected network
    // cannot retain stale pre-normalization TilePos values. Exact bridge-bank
    // approaches are graded before this sole whole-world graph construction for
    // the same reason.
    let mut graph = OrdinaryGraph::from_volume(&world.volume, Some(&world.blockers));
    let full_graph_rebuilds = 1_u32;
    let mut local_graph_repairs = 0_u32;
    let mut lower_distances = ordinary_band_distances(&graph, root, OrdinaryRegionBand::Lower);
    let mut upper_distances =
        ordinary_band_distances(&graph, upper_root, OrdinaryRegionBand::Upper);
    let mut network = lower_distances
        .keys()
        .chain(upper_distances.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    let mut network_by_coord = positions_by_coord(&network);
    if !lower_distances.contains_key(&root) || !upper_distances.contains_key(&upper_root) {
        return Err(schematic_contract(
            "normalized water banks disconnected an exact ordinary hub root",
        ));
    }
    grand_profile_checkpoint(
        "ordinary / initial graph and distances",
        profile_started,
        &mut profile_previous,
    );
    let mut hubs_by_cell = BTreeMap::<u16, TilePos>::new();
    let mut pending_cells = VecDeque::from(ordinary_cells.clone());
    let mut consecutive_deferrals = 0_usize;
    let mut deferred_diagnostics = BTreeMap::<u16, String>::new();
    let mut protected_surfaces = BTreeSet::from([root, upper_root]);

    // The two portal ribbons can belong to separate same-elevation components
    // even though the complete graph joins both through the opposite band. Join
    // each natural-pass end to its own regional root before choosing one hub per
    // coarse cell, otherwise the global connection can conceal a missing
    // upper-only route from the pass crown to the Crystal summit.
    let natural_route = world
        .features
        .protected_routes
        .get("grand_v3.natural_pass")
        .ok_or_else(|| schematic_contract("ordinary construction has no natural pass"))?;
    let natural_lower = natural_route
        .centerline
        .iter()
        .copied()
        .find(|position| {
            OrdinaryRegionBand::containing(position.level) == OrdinaryRegionBand::Lower
        })
        .ok_or_else(|| schematic_contract("natural pass has no lower-band endpoint"))?;
    let natural_upper = natural_route
        .centerline
        .iter()
        .rev()
        .copied()
        .find(|position| {
            OrdinaryRegionBand::containing(position.level) == OrdinaryRegionBand::Upper
        })
        .ok_or_else(|| schematic_contract("natural pass has no upper-band endpoint"))?;
    let structure_coords = world
        .structures
        .by_id
        .values()
        .flat_map(|structure| structure.voxels.iter().map(|voxel| voxel.coord))
        .collect::<BTreeSet<_>>();
    let exact_protected_by_coord = world
        .features
        .protected_routes
        .values()
        .flat_map(|route| route.surfaces.iter().copied())
        .fold(
            BTreeMap::<HexCoord, Option<TilePos>>::new(),
            |mut by_coord, position| {
                by_coord
                    .entry(position.coord)
                    .and_modify(|existing| {
                        if existing.is_some_and(|current| current != position) {
                            *existing = None;
                        }
                    })
                    .or_insert(Some(position));
                by_coord
            },
        );

    let mut required_connectors = vec![
        (
            natural_lower,
            OrdinaryRegionBand::Lower,
            root,
            "natural-pass lower connector".to_owned(),
            None,
        ),
        (
            natural_upper,
            OrdinaryRegionBand::Upper,
            upper_root,
            "natural-pass upper connector".to_owned(),
            None,
        ),
    ];
    required_connectors.extend(bridge_approaches.iter().map(|approach| {
        (
            approach.surface,
            OrdinaryRegionBand::Lower,
            root,
            format!(
                "bridge {:?} bank {} lane {} connector",
                approach.structure, approach.bank_index, approach.lane_index
            ),
            Some(*approach),
        )
    }));

    for (required, band, band_root, label, bridge_approach) in required_connectors {
        let already_connected = match band {
            OrdinaryRegionBand::Lower => lower_distances.contains_key(&required),
            OrdinaryRegionBand::Upper => upper_distances.contains_key(&required),
        };
        if !already_connected {
            // The required endpoint is itself inside a protected route ribbon.
            // Treat unambiguous, blocker-free exact same-band route surfaces
            // like immutable water banks: search may walk across them at their
            // published levels, but grading and carving may never replace them.
            // Structural/Crystal coordinates remain hard obstacles.
            let mut connector_preserved = immutable_water_banks.clone();
            connector_preserved.extend(bridge_approach_authority.keys().copied());
            connector_preserved.extend(
                exact_protected_by_coord
                    .iter()
                    .filter_map(|(coord, position)| (*position).map(|position| (*coord, position)))
                    .filter(|(coord, position)| {
                        OrdinaryRegionBand::containing(position.level) == band
                            && !crystal_mask.contains(coord)
                            && !structure_coords.contains(coord)
                            && !world.blockers.contains(position)
                            && surface_by_coord.get(coord).copied() == Some(*position)
                            && world
                                .volume
                                .surfaces
                                .get(position)
                                .is_some_and(|metadata| metadata.access == SurfaceAccess::Ordinary)
                    })
                    .map(|(coord, _)| coord),
            );
            let solved = solve_required_ordinary_connector(
                required,
                band,
                &world.volume,
                &world.layout.footprint,
                &ordinary_mask,
                &hard_forbidden,
                &connector_preserved,
                &network_by_coord,
                &surface_by_coord,
                &bridge_apron_no_touch,
                bridge_approach,
            );
            let (path, target, levels) = if let Some(solved) = solved {
                solved
            } else {
                // The legacy geometry-only solver is retained here solely as a
                // fail-closed diagnostic. Successful authored connectors never
                // pay for a second whole-corridor search.
                let skeletons = ordinary_connector_to_network(
                    required.coord,
                    band,
                    &world.layout.footprint,
                    &ordinary_mask,
                    &hard_forbidden,
                    &connector_preserved,
                    &network_by_coord,
                    &surface_by_coord,
                );
                let skeleton_count = skeletons.len();
                let minimum_cells = skeletons
                    .iter()
                    .map(|(path, _)| path.len())
                    .min()
                    .unwrap_or_default();
                let maximum_cells = skeletons
                    .iter()
                    .map(|(path, _)| path.len())
                    .max()
                    .unwrap_or_default();
                let target_level_range = skeletons.iter().map(|(_, target)| target.level).fold(
                    None::<(Level, Level)>,
                    |range, level| {
                        Some(range.map_or((level, level), |(minimum, maximum)| {
                            (minimum.min(level), maximum.max(level))
                        }))
                    },
                );
                let blocked_neighbors = required
                    .coord
                    .neighbors()
                    .into_iter()
                    .filter(|coord| {
                        hard_forbidden.contains(coord) && !connector_preserved.contains(coord)
                    })
                    .count();
                let skeleton_diagnostics = skeletons
                    .iter()
                    .enumerate()
                    .map(|(index, (path, target))| {
                        connector_skeleton_diagnostic(
                            index,
                            path,
                            required.level,
                            *target,
                            band,
                            &world.volume,
                            &connector_preserved,
                            &surface_by_coord,
                        )
                    })
                    .collect::<Vec<_>>();
                return Err(schematic_contract(format!(
                    "{label} cannot reach its band root while retaining exact endpoint {required:?} toward {band_root:?}; {skeleton_count} geometric skeletons ({minimum_cells}..={maximum_cells} cells), target levels {target_level_range:?}, {} typed preserved coords, {blocked_neighbors}/6 unpreserved hard-forbidden start neighbors; height-aware search found no simple connector within the 192-edge contract; skeleton diagnostics: {}",
                    connector_preserved.len(),
                    skeleton_diagnostics.join(" | ")
                )));
            };
            let route = carve_ordinary_connector(
                &label,
                &path,
                levels,
                target,
                fine_index,
                &connector_preserved,
                world,
                &mut surface_by_coord,
            )?
            .ok_or_else(|| {
                schematic_contract(format!(
                    "{label} cannot meet immutable-bank step constraints"
                ))
            })?;
            let changed_coords = ordinary_connector_changed_coords(&route, &connector_preserved);
            hard_forbidden.extend(route.iter().map(|position| position.coord));
            protected_surfaces.extend(route);

            network = repair_ordinary_network_cache(
                &mut graph,
                &world.volume,
                &world.blockers,
                &changed_coords,
                root,
                upper_root,
                &mut lower_distances,
                &mut upper_distances,
            )?;
            local_graph_repairs = local_graph_repairs.checked_add(1).ok_or_else(|| {
                schematic_contract("ordinary local graph repair count exceeded u32")
            })?;
            network_by_coord = positions_by_coord(&network);
        }
        let distances = match band {
            OrdinaryRegionBand::Lower => &lower_distances,
            OrdinaryRegionBand::Upper => &upper_distances,
        };
        if !distances.contains_key(&required) {
            return Err(schematic_contract(format!(
                "{label} did not join its same-band root"
            )));
        }
        reserve_deterministic_band_path(
            &graph,
            distances,
            required,
            band_root,
            &mut protected_surfaces,
            &mut hard_forbidden,
        )?;
    }
    grand_profile_checkpoint(
        "ordinary / portal connectors",
        profile_started,
        &mut profile_previous,
    );
    while let Some(cell) = pending_cells.pop_front() {
        let patch = world
            .layout
            .patches
            .get(&PatchId(u32::from(cell.id.get())))
            .ok_or_else(|| {
                schematic_contract(format!(
                    "ordinary cell {} has no resolved patch",
                    cell.id.get()
                ))
            })?;
        let center = schematic_to_world(cell.coord, 22);
        if let Some(hub) = patch
            .mask
            .iter()
            .flat_map(|coord| network_by_coord.get(coord).into_iter().flatten())
            .copied()
            .min_by_key(|position| {
                (
                    position.coord.distance(center),
                    position.level.abs_diff(20),
                    *position,
                )
            })
        {
            if hubs_by_cell.insert(cell.id.get(), hub).is_some() {
                return Err(schematic_contract(format!(
                    "ordinary cell {} published more than one hub",
                    cell.id.get()
                )));
            }
            consecutive_deferrals = 0;
            drop(deferred_diagnostics.remove(&cell.id.get()));
            let band = OrdinaryRegionBand::containing(hub.level);
            let (distances, band_root) = match band {
                OrdinaryRegionBand::Lower => (&lower_distances, root),
                OrdinaryRegionBand::Upper => (&upper_distances, upper_root),
            };
            reserve_deterministic_band_path(
                &graph,
                distances,
                hub,
                band_root,
                &mut protected_surfaces,
                &mut hard_forbidden,
            )?;
            continue;
        }

        let mut candidates = patch
            .mask
            .iter()
            .filter_map(|coord| surface_by_coord.get(coord).copied())
            .filter(|position| {
                (!hard_forbidden.contains(&position.coord)
                    || immutable_water_banks.contains(&position.coord))
                    && !world.blockers.contains(position)
                    && world
                        .volume
                        .surfaces
                        .get(position)
                        .is_some_and(|metadata| metadata.access == SurfaceAccess::Ordinary)
                    && world
                        .volume
                        .surface_headroom(*position)
                        .is_some_and(|headroom| headroom.0 >= 2)
            })
            .collect::<Vec<_>>();
        candidates.sort_unstable_by_key(|position| {
            (
                position.coord.distance(center),
                position.level.abs_diff(20),
                immutable_water_banks.contains(&position.coord),
                *position,
            )
        });
        let candidate_count = candidates.len();
        let mut path_count = 0_usize;
        let mut band_counts = [0_usize; 2];
        let mut reachable_fallback_count = 0_usize;

        let mut connected = None;
        for candidate in candidates.iter().copied().take(24) {
            let band = OrdinaryRegionBand::containing(candidate.level);
            band_counts[usize::from(band == OrdinaryRegionBand::Upper)] =
                band_counts[usize::from(band == OrdinaryRegionBand::Upper)].saturating_add(1);
            let (candidate_paths, candidate_connection) = try_ordinary_candidate_connector(
                candidate,
                cell.id.get(),
                band,
                &ordinary_mask,
                &hard_forbidden,
                &immutable_water_banks,
                &network_by_coord,
                fine_index,
                world,
                &mut surface_by_coord,
            )?;
            path_count = path_count.saturating_add(candidate_paths);
            connected = candidate_connection;
            if connected.is_some() {
                break;
            }
        }
        if connected.is_none() && candidates.len() > 24 {
            let remaining = &candidates[24..];
            let needs_lower = remaining.iter().any(|candidate| {
                OrdinaryRegionBand::containing(candidate.level) == OrdinaryRegionBand::Lower
            });
            let needs_upper = remaining.iter().any(|candidate| {
                OrdinaryRegionBand::containing(candidate.level) == OrdinaryRegionBand::Upper
            });
            let lower_reverse = needs_lower.then(|| {
                ordinary_connector_reverse_distances(
                    OrdinaryRegionBand::Lower,
                    &world.layout.footprint,
                    &ordinary_mask,
                    &hard_forbidden,
                    &immutable_water_banks,
                    &network_by_coord,
                    &surface_by_coord,
                )
            });
            let upper_reverse = needs_upper.then(|| {
                ordinary_connector_reverse_distances(
                    OrdinaryRegionBand::Upper,
                    &world.layout.footprint,
                    &ordinary_mask,
                    &hard_forbidden,
                    &immutable_water_banks,
                    &network_by_coord,
                    &surface_by_coord,
                )
            });
            let mut reachable = remaining
                .iter()
                .copied()
                .filter_map(|candidate| {
                    let reverse = match OrdinaryRegionBand::containing(candidate.level) {
                        OrdinaryRegionBand::Lower => lower_reverse.as_ref(),
                        OrdinaryRegionBand::Upper => upper_reverse.as_ref(),
                    }?;
                    reverse
                        .get(&candidate.coord)
                        .copied()
                        .map(|distance| (distance, candidate))
                })
                .collect::<Vec<_>>();
            reachable.sort_unstable_by_key(|(distance, position)| {
                (
                    *distance,
                    position.coord.distance(center),
                    position.level.abs_diff(20),
                    immutable_water_banks.contains(&position.coord),
                    *position,
                )
            });
            reachable_fallback_count = reachable.len();
            for (_, candidate) in reachable.into_iter().take(24) {
                let band = OrdinaryRegionBand::containing(candidate.level);
                band_counts[usize::from(band == OrdinaryRegionBand::Upper)] =
                    band_counts[usize::from(band == OrdinaryRegionBand::Upper)].saturating_add(1);
                let (candidate_paths, candidate_connection) = try_ordinary_candidate_connector(
                    candidate,
                    cell.id.get(),
                    band,
                    &ordinary_mask,
                    &hard_forbidden,
                    &immutable_water_banks,
                    &network_by_coord,
                    fine_index,
                    world,
                    &mut surface_by_coord,
                )?;
                path_count = path_count.saturating_add(candidate_paths);
                connected = candidate_connection;
                if connected.is_some() {
                    break;
                }
            }
        }
        let Some((hub, route)) = connected else {
            drop(deferred_diagnostics.insert(
                cell.id.get(),
                format!(
                    "ordinary cell {} has no dry same-region connector to the foothill network \
                 ({} candidates, {} reachable fallback candidates, lower/upper attempts {:?}, \
                  {} paths reached the network)",
                    cell.id.get(),
                    candidate_count,
                    reachable_fallback_count,
                    band_counts,
                    path_count
                ),
            ));
            pending_cells.push_back(cell);
            consecutive_deferrals = consecutive_deferrals.saturating_add(1);
            if consecutive_deferrals >= pending_cells.len() {
                let stalled = pending_cells
                    .iter()
                    .map(|pending| pending.id.get())
                    .collect::<Vec<_>>();
                let details = stalled
                    .iter()
                    .filter_map(|id| deferred_diagnostics.get(id))
                    .cloned()
                    .collect::<Vec<_>>();
                return Err(schematic_contract(format!(
                    "ordinary hub frontier stalled for cells {stalled:?}: {}",
                    details.join("; ")
                )));
            }
            continue;
        };
        let changed_coords = ordinary_connector_changed_coords(&route, &immutable_water_banks);
        hard_forbidden.extend(route.iter().map(|position| position.coord));
        protected_surfaces.extend(route);
        if hubs_by_cell.insert(cell.id.get(), hub).is_some() {
            return Err(schematic_contract(format!(
                "ordinary cell {} published more than one hub",
                cell.id.get()
            )));
        }
        consecutive_deferrals = 0;
        drop(deferred_diagnostics.remove(&cell.id.get()));

        network = repair_ordinary_network_cache(
            &mut graph,
            &world.volume,
            &world.blockers,
            &changed_coords,
            root,
            upper_root,
            &mut lower_distances,
            &mut upper_distances,
        )?;
        local_graph_repairs = local_graph_repairs
            .checked_add(1)
            .ok_or_else(|| schematic_contract("ordinary local graph repair count exceeded u32"))?;
        network_by_coord = positions_by_coord(&network);
        if let Some(prior) = hubs_by_cell.values().find(|position| {
            let distances = match OrdinaryRegionBand::containing(position.level) {
                OrdinaryRegionBand::Lower => &lower_distances,
                OrdinaryRegionBand::Upper => &upper_distances,
            };
            !distances.contains_key(position)
        }) {
            return Err(schematic_contract(format!(
                "ordinary connector severed prior same-band hub {prior:?}"
            )));
        }
        let band = OrdinaryRegionBand::containing(hub.level);
        let (distances, band_root) = match band {
            OrdinaryRegionBand::Lower => (&lower_distances, root),
            OrdinaryRegionBand::Upper => (&upper_distances, upper_root),
        };
        reserve_deterministic_band_path(
            &graph,
            distances,
            hub,
            band_root,
            &mut protected_surfaces,
            &mut hard_forbidden,
        )?;
    }
    grand_profile_checkpoint(
        "ordinary / coarse-cell connectors",
        profile_started,
        &mut profile_previous,
    );

    let hubs = ordinary_cells
        .iter()
        .map(|cell| {
            hubs_by_cell.get(&cell.id.get()).copied().ok_or_else(|| {
                schematic_contract(format!(
                    "ordinary cell {} lost its deferred hub",
                    cell.id.get()
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    if hubs.len()
        != plan
            .cells
            .iter()
            .filter(|cell| is_ordinary_land(cell))
            .count()
        || hubs.iter().copied().collect::<BTreeSet<_>>().len() != hubs.len()
    {
        return Err(schematic_contract(
            "ordinary construction did not publish exactly one unique hub per ordinary cell",
        ));
    }

    let distances = graph.distances_from(root);
    if let Some(hub) = hubs.iter().find(|hub| !distances.contains_key(hub)) {
        return Err(schematic_contract(format!(
            "constructed ordinary hub {hub:?} is not physically reachable from the foothill"
        )));
    }
    let reachable = distances.keys().copied().collect::<BTreeSet<_>>();
    if let Some(approach) = bridge_approaches
        .iter()
        .find(|approach| !reachable.contains(&approach.surface))
    {
        return Err(schematic_contract(format!(
            "bridge {:?} bank {} lane {} approach {:?} is not foothill-reachable after constructive attachment",
            approach.structure, approach.bank_index, approach.lane_index, approach.surface
        )));
    }
    if let Some((bridge, surface)) = bridges.crossings.iter().find_map(|bridge| {
        bridge
            .deck
            .iter()
            .find(|surface| !reachable.contains(surface))
            .copied()
            .map(|surface| (bridge, surface))
    }) {
        return Err(schematic_contract(format!(
            "bridge {:?} deck surface {surface:?} is not foothill-reachable after constructive attachment",
            bridge.structure
        )));
    }
    let authored_surfaces =
        authored_ordinary_surface_authority(world, protected_surfaces.iter().copied());
    let mut access_changed_coords = BTreeSet::new();
    for (position, metadata) in &mut world.volume.surfaces {
        if metadata.access == SurfaceAccess::Ordinary
            && !reachable.contains(position)
            && !authored_surfaces.contains(position)
        {
            metadata.access = SurfaceAccess::SpecialMovement(INACCESSIBLE_MOVEMENT_REGION);
            access_changed_coords.insert(position.coord);
        }
    }
    if !access_changed_coords.is_empty() {
        let _affected =
            graph.refresh_coords(&world.volume, Some(&world.blockers), access_changed_coords);
    }
    grand_profile_checkpoint(
        "ordinary / final reachability and access",
        profile_started,
        &mut profile_previous,
    );

    Ok(OrdinaryNetworkCompilation {
        route: ProtectedFeatureRoute {
            centerline: hubs,
            surfaces: protected_surfaces,
        },
        graph,
        full_graph_rebuilds,
        local_graph_repairs,
    })
}

fn top_standable_surfaces_by_coord(volume: &VolumePlan) -> BTreeMap<HexCoord, TilePos> {
    volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| {
            (metadata.access != SurfaceAccess::NonStandable).then_some(*position)
        })
        .fold(BTreeMap::new(), |mut result, position| {
            result
                .entry(position.coord)
                .and_modify(|current| *current = (*current).max(position))
                .or_insert(position);
            result
        })
}

fn positions_by_coord(positions: &BTreeSet<TilePos>) -> BTreeMap<HexCoord, Vec<TilePos>> {
    positions.iter().copied().fold(
        BTreeMap::<HexCoord, Vec<TilePos>>::new(),
        |mut result, position| {
            result.entry(position.coord).or_default().push(position);
            result
        },
    )
}

fn ordinary_band_distances(
    graph: &OrdinaryGraph,
    start: TilePos,
    band: OrdinaryRegionBand,
) -> BTreeMap<TilePos, u32> {
    if !graph.contains(start) || OrdinaryRegionBand::containing(start.level) != band {
        return BTreeMap::new();
    }
    let mut distances = BTreeMap::from([(start, 0_u32)]);
    let mut frontier = VecDeque::from([start]);
    while let Some(position) = frontier.pop_front() {
        let Some(distance) = distances.get(&position).copied() else {
            continue;
        };
        for neighbor in graph.neighbors(position) {
            if OrdinaryRegionBand::containing(neighbor.level) == band
                && !distances.contains_key(neighbor)
            {
                distances.insert(*neighbor, distance.saturating_add(1));
                frontier.push_back(*neighbor);
            }
        }
    }
    distances
}

/// Repairs one exact same-band distance projection after a local graph edit.
///
/// First remove every old distance whose predecessor chain was broken. Because
/// distances strictly decrease along such a chain, retaining a node with any
/// surviving `distance - 1` neighbor proves that its old shortest path still
/// exists. Then seed Dijkstra from the edit boundary and every invalidated node;
/// this restores longer alternative paths and propagates newly shortened paths.
/// The result is identical to [`ordinary_band_distances`] without visiting
/// unaffected graph components.
fn repair_ordinary_band_distances(
    graph: &OrdinaryGraph,
    distances: &mut BTreeMap<TilePos, u32>,
    root: TilePos,
    band: OrdinaryRegionBand,
    affected: &BTreeSet<TilePos>,
) -> Result<(), V3GenerationError> {
    if !graph.contains(root) || OrdinaryRegionBand::containing(root.level) != band {
        return Err(schematic_contract(format!(
            "local ordinary graph repair lost {band:?} root {root:?}"
        )));
    }

    let mut invalidated = BTreeSet::<TilePos>::new();
    let mut pending = affected
        .iter()
        .copied()
        .filter(|position| distances.contains_key(position))
        .collect::<BTreeSet<_>>();
    let mut frontier = pending.iter().copied().collect::<VecDeque<_>>();
    while let Some(position) = frontier.pop_front() {
        pending.remove(&position);
        let Some(distance) = distances.get(&position).copied() else {
            continue;
        };
        let still_valid = graph.contains(position)
            && OrdinaryRegionBand::containing(position.level) == band
            && if position == root {
                distance == 0
            } else {
                distance.checked_sub(1).is_some_and(|predecessor_distance| {
                    graph.neighbors(position).iter().any(|neighbor| {
                        OrdinaryRegionBand::containing(neighbor.level) == band
                            && distances.get(neighbor).copied() == Some(predecessor_distance)
                    })
                })
            };
        if still_valid {
            continue;
        }
        distances.remove(&position);
        invalidated.insert(position);
        for neighbor in graph.neighbors(position) {
            if distances.contains_key(neighbor) && pending.insert(*neighbor) {
                frontier.push_back(*neighbor);
            }
        }
    }

    if distances.get(&root).copied() != Some(0) {
        return Err(schematic_contract(format!(
            "local ordinary graph repair disconnected {band:?} root {root:?}"
        )));
    }

    let mut relaxation_seeds = affected.clone();
    for position in &invalidated {
        if graph.contains(*position) {
            relaxation_seeds.insert(*position);
            relaxation_seeds.extend(graph.neighbors(*position).iter().copied());
        }
    }
    let mut relaxation = BinaryHeap::<Reverse<(u32, TilePos)>>::new();
    for position in relaxation_seeds {
        if !graph.contains(position) || OrdinaryRegionBand::containing(position.level) != band {
            continue;
        }
        let mut candidate = (position == root).then_some(0);
        if position != root {
            for neighbor in graph
                .neighbors(position)
                .iter()
                .filter(|neighbor| OrdinaryRegionBand::containing(neighbor.level) == band)
            {
                let Some(distance) = distances.get(neighbor).copied() else {
                    continue;
                };
                let next_distance = distance.checked_add(1).ok_or_else(|| {
                    schematic_contract("local ordinary graph distance exceeded u32")
                })?;
                candidate =
                    Some(candidate.map_or(next_distance, |current| current.min(next_distance)));
            }
        }
        if let Some(candidate) = candidate {
            if distances
                .get(&position)
                .is_none_or(|current| candidate < *current)
            {
                distances.insert(position, candidate);
            }
        }
        if let Some(distance) = distances.get(&position).copied() {
            relaxation.push(Reverse((distance, position)));
        }
    }

    while let Some(Reverse((distance, position))) = relaxation.pop() {
        if distances.get(&position).copied() != Some(distance) {
            continue;
        }
        let Some(next_distance) = distance.checked_add(1) else {
            return Err(schematic_contract(
                "local ordinary graph distance exceeded u32",
            ));
        };
        for neighbor in graph.neighbors(position) {
            if OrdinaryRegionBand::containing(neighbor.level) != band
                || distances
                    .get(neighbor)
                    .is_some_and(|current| *current <= next_distance)
            {
                continue;
            }
            distances.insert(*neighbor, next_distance);
            relaxation.push(Reverse((next_distance, *neighbor)));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn repair_ordinary_network_cache(
    graph: &mut OrdinaryGraph,
    volume: &VolumePlan,
    blockers: &BTreeSet<TilePos>,
    changed_coords: &BTreeSet<HexCoord>,
    lower_root: TilePos,
    upper_root: TilePos,
    lower_distances: &mut BTreeMap<TilePos, u32>,
    upper_distances: &mut BTreeMap<TilePos, u32>,
) -> Result<BTreeSet<TilePos>, V3GenerationError> {
    let affected = graph.refresh_coords(volume, Some(blockers), changed_coords.iter().copied());
    repair_ordinary_band_distances(
        graph,
        lower_distances,
        lower_root,
        OrdinaryRegionBand::Lower,
        &affected,
    )?;
    repair_ordinary_band_distances(
        graph,
        upper_distances,
        upper_root,
        OrdinaryRegionBand::Upper,
        &affected,
    )?;
    Ok(lower_distances
        .keys()
        .chain(upper_distances.keys())
        .copied()
        .collect())
}

fn ordinary_connector_changed_coords(
    route: &[TilePos],
    preserved_coords: &BTreeSet<HexCoord>,
) -> BTreeSet<HexCoord> {
    route
        .iter()
        .take(route.len().saturating_sub(1))
        .filter_map(|position| {
            (!preserved_coords.contains(&position.coord)).then_some(position.coord)
        })
        .collect()
}

fn reserve_deterministic_band_path(
    graph: &OrdinaryGraph,
    distances: &BTreeMap<TilePos, u32>,
    hub: TilePos,
    root: TilePos,
    protected_surfaces: &mut BTreeSet<TilePos>,
    hard_forbidden: &mut BTreeSet<HexCoord>,
) -> Result<(), V3GenerationError> {
    let mut cursor = hub;
    protected_surfaces.insert(cursor);
    hard_forbidden.insert(cursor.coord);
    while cursor != root {
        let distance = distances.get(&cursor).copied().ok_or_else(|| {
            schematic_contract(format!(
                "ordinary hub {hub:?} lost its same-band root distance"
            ))
        })?;
        cursor = graph
            .neighbors(cursor)
            .iter()
            .copied()
            .filter(|neighbor| distances.get(neighbor).copied() == Some(distance.saturating_sub(1)))
            .min()
            .ok_or_else(|| {
                schematic_contract(format!(
                    "ordinary hub {hub:?} has no deterministic same-band parent"
                ))
            })?;
        protected_surfaces.insert(cursor);
        hard_forbidden.insert(cursor.coord);
    }
    Ok(())
}

fn carve_ordinary_connector(
    label: &str,
    path: &[HexCoord],
    levels: Vec<Level>,
    target: TilePos,
    fine_index: &FineWorldIndex,
    preserved_coords: &BTreeSet<HexCoord>,
    world: &mut GeneratedWorldPlan,
    surface_by_coord: &mut BTreeMap<HexCoord, TilePos>,
) -> Result<Option<Vec<TilePos>>, V3GenerationError> {
    if levels.len() != path.len() {
        return Err(schematic_contract(format!(
            "{label} level plan does not match its coordinate path"
        )));
    }
    let mut route = Vec::with_capacity(path.len());
    for (index, (coord, level)) in path.iter().copied().zip(levels).enumerate() {
        let position = if index == path.len().saturating_sub(1) {
            target
        } else if preserved_coords.contains(&coord) {
            surface_by_coord.get(&coord).copied().ok_or_else(|| {
                schematic_contract(format!("{label} lost immutable bank at {coord:?}"))
            })?
        } else {
            TilePos::new(coord, level)
        };
        route.push(position);
    }
    if route
        .windows(2)
        .any(|pair| pair[0].coord.distance(pair[1].coord) != 1)
        || route
            .windows(2)
            .any(|pair| pair[0].level.abs_diff(pair[1].level) > 1)
    {
        return Ok(None);
    }

    for (index, position) in route.iter().copied().enumerate() {
        if index == route.len().saturating_sub(1) || preserved_coords.contains(&position.coord) {
            continue;
        }
        let coord = position.coord;
        let biome = fine_index.biome(coord).ok_or_else(|| {
            schematic_contract(format!("{label} has no biome owner at {coord:?}"))
        })?;
        let material = world
            .volume
            .columns
            .get(&coord)
            .map(top_solid_material)
            .unwrap_or(SolidMaterialRole::Gravel);
        replace_column_surface(
            &mut world.volume,
            &mut world.biome_regions,
            coord,
            land_column(position.level, material),
            position,
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: None,
            },
            biome,
        );
        surface_by_coord.insert(coord, position);
    }
    Ok(Some(route))
}

#[allow(clippy::too_many_arguments)]
fn try_ordinary_candidate_connector(
    candidate: TilePos,
    cell_id: u16,
    band: OrdinaryRegionBand,
    ordinary_mask: &BTreeSet<HexCoord>,
    hard_forbidden: &BTreeSet<HexCoord>,
    immutable_water_banks: &BTreeSet<HexCoord>,
    network_by_coord: &BTreeMap<HexCoord, Vec<TilePos>>,
    fine_index: &FineWorldIndex,
    world: &mut GeneratedWorldPlan,
    surface_by_coord: &mut BTreeMap<HexCoord, TilePos>,
) -> Result<(usize, Option<(TilePos, Vec<TilePos>)>), V3GenerationError> {
    let skeletons = ordinary_connector_to_network(
        candidate.coord,
        band,
        &world.layout.footprint,
        ordinary_mask,
        hard_forbidden,
        immutable_water_banks,
        network_by_coord,
        surface_by_coord,
    );
    let path_count = skeletons.len();
    if skeletons.is_empty() {
        return Ok((path_count, None));
    }
    let Some((path, target, levels)) = solve_ordinary_connector_candidates(
        skeletons,
        candidate.level,
        band,
        false,
        &world.volume,
        &world.layout.footprint,
        ordinary_mask,
        hard_forbidden,
        immutable_water_banks,
        network_by_coord,
        surface_by_coord,
    ) else {
        return Ok((path_count, None));
    };
    let label = format!("ordinary connector for cell {cell_id}");
    let Some(route) = carve_ordinary_connector(
        &label,
        &path,
        levels,
        target,
        fine_index,
        immutable_water_banks,
        world,
        surface_by_coord,
    )?
    else {
        return Ok((path_count, None));
    };
    Ok((path_count, route.first().copied().map(|hub| (hub, route))))
}

fn ordinary_connector_to_network(
    start: HexCoord,
    band: OrdinaryRegionBand,
    footprint: &BTreeSet<HexCoord>,
    ordinary_mask: &BTreeSet<HexCoord>,
    hard_forbidden: &BTreeSet<HexCoord>,
    preserved_pass_through: &BTreeSet<HexCoord>,
    network_by_coord: &BTreeMap<HexCoord, Vec<TilePos>>,
    surface_by_coord: &BTreeMap<HexCoord, TilePos>,
) -> Vec<(Vec<HexCoord>, TilePos)> {
    const MAXIMUM_SKELETONS: usize = 256;
    let mut best = BTreeMap::from([(start, (0_u32, 0_u32))]);
    let mut parent = BTreeMap::<HexCoord, HexCoord>::new();
    let mut frontier = BinaryHeap::from([Reverse((0_u32, 0_u32, start))]);
    let mut skeletons = Vec::new();
    let mut seen_terminals = BTreeSet::new();
    while let Some(Reverse((cost, steps, coord))) = frontier.pop() {
        if best.get(&coord).copied() != Some((cost, steps)) {
            continue;
        }
        let mut targets = coord
            .neighbors()
            .into_iter()
            .flat_map(|neighbor| network_by_coord.get(&neighbor).into_iter().flatten())
            .copied()
            .filter(|position| band.accepts_existing(position.level))
            .collect::<Vec<_>>();
        targets.sort_unstable();
        if !targets.is_empty() {
            let mut reversed = vec![coord];
            let mut cursor = coord;
            while cursor != start {
                let Some(previous) = parent.get(&cursor).copied() else {
                    return skeletons;
                };
                cursor = previous;
                reversed.push(cursor);
            }
            reversed.reverse();
            for target in targets {
                if seen_terminals.insert((coord, target)) {
                    let mut path = reversed.clone();
                    path.push(target.coord);
                    skeletons.push((path, target));
                    if skeletons.len() >= MAXIMUM_SKELETONS {
                        return skeletons;
                    }
                }
            }
        }
        if steps >= MAXIMUM_ORDINARY_CONNECTOR_SEARCH_STEPS {
            continue;
        }
        let mut neighbors = coord.neighbors();
        neighbors.sort_unstable();
        for neighbor in neighbors {
            if start.distance(neighbor) > MAXIMUM_ORDINARY_CONNECTOR_SEARCH_STEPS
                || !ordinary_connector_coord_is_open(
                    neighbor,
                    band,
                    footprint,
                    ordinary_mask,
                    hard_forbidden,
                    preserved_pass_through,
                    network_by_coord,
                    surface_by_coord,
                )
            {
                continue;
            }
            let next_steps = steps.saturating_add(1);
            let next_cost = cost.saturating_add(1).saturating_add(
                ORDINARY_CONNECTOR_PRESERVED_COST
                    .saturating_mul(u32::from(preserved_pass_through.contains(&neighbor))),
            );
            let next_score = (next_cost, next_steps);
            let replace = best
                .get(&neighbor)
                .is_none_or(|current| next_score < *current)
                || (best.get(&neighbor) == Some(&next_score)
                    && parent
                        .get(&neighbor)
                        .is_none_or(|previous| coord < *previous));
            if replace {
                best.insert(neighbor, next_score);
                parent.insert(neighbor, coord);
                frontier.push(Reverse((next_cost, next_steps, neighbor)));
            }
        }
    }
    skeletons
}

/// Finds an exact-height connector for an authored endpoint in one pass.
///
/// The ordinary hub builder can grade new columns, but an authored route or an
/// immutable bank already has a singleton surface level. Searching only in
/// two dimensions loses that distinction: the one retained parent for a
/// coordinate can cross an incompatible exact surface even when a slightly
/// longer, height-feasible corridor exists. Required portal connectors therefore
/// search `(coordinate, level)` states directly. The only bound is the public
/// 192-edge connector contract; there are no candidate-count, waypoint, or
/// switchback-repair caps. Equal coordinate/level states use deterministic
/// cheapest-parent dominance, while ancestor-coordinate checks keep the
/// returned terrain path simple.
#[allow(clippy::too_many_arguments)]
fn solve_required_ordinary_connector(
    start: TilePos,
    band: OrdinaryRegionBand,
    volume: &VolumePlan,
    footprint: &BTreeSet<HexCoord>,
    ordinary_mask: &BTreeSet<HexCoord>,
    hard_forbidden: &BTreeSet<HexCoord>,
    preserved_pass_through: &BTreeSet<HexCoord>,
    network_by_coord: &BTreeMap<HexCoord, Vec<TilePos>>,
    surface_by_coord: &BTreeMap<HexCoord, TilePos>,
    bridge_apron_no_touch: &BTreeSet<HexCoord>,
    bridge_approach: Option<BridgeBankApproach>,
) -> Option<(Vec<HexCoord>, TilePos, Vec<Level>)> {
    if !band.accepts_existing(start.level)
        || surface_by_coord.get(&start.coord).copied() != Some(start)
    {
        return None;
    }

    // The geometric relaxation proves whether a state still fits inside the
    // public edge budget. The weighted relaxation then directs A* around the
    // much more expensive preserved surfaces without excluding any vertically
    // feasible or coordinate-simple route. Both consume the same immutable
    // dense admission projection so their repeated membership checks remain
    // exact without repeated ordered-tree lookups.
    let domain = RequiredConnectorDomain::new(
        band,
        footprint,
        ordinary_mask,
        hard_forbidden,
        preserved_pass_through,
        network_by_coord,
        surface_by_coord,
    )?;
    let reverse_distances = required_connector_reverse_distances(&domain, band, network_by_coord);
    let start_remaining_steps = domain.metric(&reverse_distances, start.coord)?;
    if start_remaining_steps > MAXIMUM_ORDINARY_CONNECTOR_SEARCH_STEPS {
        return None;
    }
    let reverse_costs =
        required_connector_reverse_costs(&domain, band, preserved_pass_through, network_by_coord);
    let start_estimate = domain.metric(&reverse_costs, start.coord)?;
    let start_state = (start.coord, start.level);
    let mut best = BTreeMap::from([(start_state, (0_u32, 0_u32))]);
    let mut parent = BTreeMap::<(HexCoord, Level), (HexCoord, Level)>::new();
    let mut frontier = BinaryHeap::from([Reverse((start_estimate, 0_u32, 0_u32, start_state))]);

    while let Some(Reverse((_, cost, steps, state))) = frontier.pop() {
        if best.get(&state).copied() != Some((cost, steps)) {
            continue;
        }
        let (coord, level) = state;

        let mut targets = coord
            .neighbors()
            .into_iter()
            .flat_map(|neighbor| network_by_coord.get(&neighbor).into_iter().flatten())
            .copied()
            .filter(|target| band.accepts_existing(target.level))
            .collect::<Vec<_>>();
        targets.sort_unstable();
        for target in targets {
            let target_headroom = volume
                .surface_headroom(target)
                .map_or(0, |headroom| headroom.0);
            let flat_final = target_headroom < 3;
            if level.abs_diff(target.level) > 1 || (flat_final && level != target.level) {
                continue;
            }
            let states = reconstruct_connector_state_path(state, start_state, &parent)?;
            let mut coordinates = states
                .iter()
                .map(|(state_coord, _)| *state_coord)
                .collect::<Vec<_>>();
            if coordinates.iter().copied().collect::<BTreeSet<_>>().len() != coordinates.len() {
                continue;
            }
            let mut levels = states
                .iter()
                .map(|(_, state_level)| *state_level)
                .collect::<Vec<_>>();
            coordinates.push(target.coord);
            levels.push(target.level);
            return Some((coordinates, target, levels));
        }

        if steps >= MAXIMUM_ORDINARY_CONNECTOR_SEARCH_STEPS {
            continue;
        }
        let mut neighbors = coord.neighbors();
        neighbors.sort_unstable();
        for neighbor in neighbors {
            if !domain.is_open(neighbor) {
                continue;
            }

            let Some(next_steps) = steps.checked_add(1) else {
                continue;
            };
            let Some(remaining_steps) = domain.metric(&reverse_distances, neighbor) else {
                continue;
            };
            if next_steps
                .checked_add(remaining_steps)
                .is_none_or(|minimum_total| minimum_total > MAXIMUM_ORDINARY_CONNECTOR_SEARCH_STEPS)
                || connector_state_path_contains_coord(state, neighbor, &parent)
            {
                continue;
            }

            let preserved_neighbor = preserved_pass_through.contains(&neighbor);
            let candidate_levels = if preserved_neighbor {
                let fixed = surface_by_coord.get(&neighbor)?.level;
                [fixed, fixed, fixed]
            } else {
                [level.saturating_sub(1), level, level.saturating_add(1)]
            };
            let Some(preserved_cost) =
                ORDINARY_CONNECTOR_PRESERVED_COST.checked_mul(u32::from(preserved_neighbor))
            else {
                continue;
            };
            let Some(transition_cost) = 1_u32.checked_add(preserved_cost) else {
                continue;
            };
            let Some(next_cost) = cost.checked_add(transition_cost) else {
                continue;
            };
            let mut previous_level = None;
            for next_level in candidate_levels {
                if previous_level == Some(next_level) {
                    continue;
                }
                previous_level = Some(next_level);
                let valid_level = if preserved_neighbor {
                    band.accepts_existing(next_level)
                } else {
                    band.accepts_new(next_level)
                };
                if !valid_level || level.abs_diff(next_level) > 1 {
                    continue;
                }
                let next_state = (neighbor, next_level);
                let next_score = (next_cost, next_steps);
                let replace = best
                    .get(&next_state)
                    .is_none_or(|current| next_score < *current)
                    || (best.get(&next_state) == Some(&next_score)
                        && parent
                            .get(&next_state)
                            .is_none_or(|previous| state < *previous));
                if replace {
                    let Some(remaining_cost) = domain.metric(&reverse_costs, neighbor) else {
                        continue;
                    };
                    let Some(estimate) = next_cost.checked_add(remaining_cost) else {
                        continue;
                    };
                    best.insert(next_state, next_score);
                    parent.insert(next_state, state);
                    frontier.push(Reverse((estimate, next_cost, next_steps, next_state)));
                }
            }
        }
    }
    if let Some(bridge_approach) = bridge_approach {
        return solve_mutable_bridge_bank_apron(
            start,
            bridge_approach,
            band,
            volume,
            footprint,
            ordinary_mask,
            hard_forbidden,
            preserved_pass_through,
            network_by_coord,
            surface_by_coord,
            bridge_apron_no_touch,
        );
    }
    None
}

/// Resolves the one mutable apron permitted between an exact bridge-bank
/// landing and an already-connected ordinary slope.
///
/// The main connector search treats existing network nodes as immutable
/// terminals. That is normally important: a new hub must attach to the network,
/// not rewrite it. A two-wide bridge is narrower, though. Its exact level-15
/// landing can abut a mutable level-17 network surface, leaving no admitted
/// coordinate on which to place the required level-16 step. Bridge connectors
/// may therefore replace exactly one adjacent selected network surface, then
/// must terminate immediately at a second unchanged network node. Authored,
/// protected, water-bank, blocker, and sibling-landing coordinates remain
/// excluded through the shared forbidden and preserved authorities.
#[allow(clippy::too_many_arguments)]
fn solve_mutable_bridge_bank_apron(
    start: TilePos,
    bridge_approach: BridgeBankApproach,
    band: OrdinaryRegionBand,
    volume: &VolumePlan,
    footprint: &BTreeSet<HexCoord>,
    ordinary_mask: &BTreeSet<HexCoord>,
    hard_forbidden: &BTreeSet<HexCoord>,
    preserved_pass_through: &BTreeSet<HexCoord>,
    network_by_coord: &BTreeMap<HexCoord, Vec<TilePos>>,
    surface_by_coord: &BTreeMap<HexCoord, TilePos>,
    bridge_apron_no_touch: &BTreeSet<HexCoord>,
) -> Option<(Vec<HexCoord>, TilePos, Vec<Level>)> {
    if bridge_approach.surface != start
        || !hard_forbidden.contains(&start.coord)
        || !preserved_pass_through.contains(&start.coord)
        || surface_by_coord.get(&start.coord).copied() != Some(start)
    {
        return None;
    }
    let apron_level = start.level.checked_add(1)?;
    let network_level = start.level.checked_add(2)?;
    if !band.accepts_new(apron_level) || !band.accepts_existing(network_level) {
        return None;
    }

    let mut candidates = Vec::new();
    let mut apron_coords = start.coord.neighbors();
    apron_coords.sort_unstable();
    for apron_coord in apron_coords {
        if !footprint.contains(&apron_coord)
            || !ordinary_mask.contains(&apron_coord)
            || hard_forbidden.contains(&apron_coord)
            || preserved_pass_through.contains(&apron_coord)
            || bridge_apron_no_touch.contains(&apron_coord)
        {
            continue;
        }
        let Some(selected) = surface_by_coord.get(&apron_coord).copied() else {
            continue;
        };
        if selected.level != network_level
            || !network_by_coord.get(&apron_coord).is_some_and(|positions| {
                positions.contains(&selected) && band.accepts_existing(selected.level)
            })
            || volume.surfaces.get(&selected).is_none_or(|metadata| {
                metadata.access != SurfaceAccess::Ordinary || metadata.interior.is_some()
            })
            || volume.surfaces_at_coord(apron_coord).count() != 1
        {
            continue;
        }

        let mut terminal_targets = apron_coord
            .neighbors()
            .into_iter()
            .filter(|coord| *coord != start.coord)
            .filter(|coord| {
                !hard_forbidden.contains(coord)
                    && !preserved_pass_through.contains(coord)
                    && !bridge_apron_no_touch.contains(coord)
            })
            .flat_map(|coord| network_by_coord.get(&coord).into_iter().flatten())
            .copied()
            .filter(|target| {
                target.level == network_level
                    && band.accepts_existing(target.level)
                    && surface_by_coord.get(&target.coord).copied() == Some(*target)
                    && volume.surfaces.get(target).is_some_and(|metadata| {
                        metadata.access == SurfaceAccess::Ordinary && metadata.interior.is_none()
                    })
                    && volume.surfaces_at_coord(target.coord).count() == 1
            })
            .collect::<Vec<_>>();
        terminal_targets.sort_unstable();
        terminal_targets.dedup();
        for target in terminal_targets {
            candidates.push((apron_coord, target));
        }
    }
    let (apron_coord, target) = candidates.into_iter().min()?;
    Some((
        vec![start.coord, apron_coord, target.coord],
        target,
        vec![start.level, apron_level, target.level],
    ))
}

fn reconstruct_connector_state_path(
    mut state: (HexCoord, Level),
    start: (HexCoord, Level),
    parent: &BTreeMap<(HexCoord, Level), (HexCoord, Level)>,
) -> Option<Vec<(HexCoord, Level)>> {
    let mut reversed = vec![state];
    while state != start {
        state = *parent.get(&state)?;
        reversed.push(state);
    }
    reversed.reverse();
    Some(reversed)
}

fn connector_state_path_contains_coord(
    mut state: (HexCoord, Level),
    coord: HexCoord,
    parent: &BTreeMap<(HexCoord, Level), (HexCoord, Level)>,
) -> bool {
    loop {
        if state.0 == coord {
            return true;
        }
        let Some(previous) = parent.get(&state).copied() else {
            return false;
        };
        state = previous;
    }
}

/// Dense, read-only admission projection for one required portal connector.
///
/// The authored portal solver revisits the same radius-187 coordinate domain in
/// two reverse searches and then in its exact `(coordinate, level)` A* search.
/// Repeating six ordered-tree membership queries for every visit dominated the
/// compiler even though none of those authorities changes during one solve.
/// This projection evaluates the public admission predicate once per footprint
/// coordinate. It does not participate in ordering: network seeds, neighbor
/// order, heap keys, parents, and tie-breaking remain canonical.
#[derive(Debug)]
struct RequiredConnectorDomain {
    minimum_q: i32,
    minimum_r: i32,
    q_count: usize,
    r_count: usize,
    open: Vec<u8>,
}

impl RequiredConnectorDomain {
    #[allow(clippy::too_many_arguments)]
    fn new(
        band: OrdinaryRegionBand,
        footprint: &BTreeSet<HexCoord>,
        ordinary_mask: &BTreeSet<HexCoord>,
        hard_forbidden: &BTreeSet<HexCoord>,
        preserved_pass_through: &BTreeSet<HexCoord>,
        network_by_coord: &BTreeMap<HexCoord, Vec<TilePos>>,
        surface_by_coord: &BTreeMap<HexCoord, TilePos>,
    ) -> Option<Self> {
        let minimum_q = footprint.iter().map(|coord| coord.x()).min()?;
        let maximum_q = footprint.iter().map(|coord| coord.x()).max()?;
        let minimum_r = footprint.iter().map(|coord| coord.y()).min()?;
        let maximum_r = footprint.iter().map(|coord| coord.y()).max()?;
        let q_count = usize::try_from(maximum_q.checked_sub(minimum_q)?.checked_add(1)?).ok()?;
        let r_count = usize::try_from(maximum_r.checked_sub(minimum_r)?.checked_add(1)?).ok()?;
        let mut open = vec![0_u8; q_count.checked_mul(r_count)?];
        for coord in footprint {
            let q = usize::try_from(coord.x().checked_sub(minimum_q)?).ok()?;
            let r = usize::try_from(coord.y().checked_sub(minimum_r)?).ok()?;
            let index = q.checked_mul(r_count)?.checked_add(r)?;
            let preserved = preserved_pass_through.contains(coord);
            let admitted = ordinary_mask.contains(coord)
                && (!hard_forbidden.contains(coord) || preserved)
                && (!preserved
                    || surface_by_coord
                        .get(coord)
                        .is_some_and(|surface| band.accepts_existing(surface.level)))
                && !network_by_coord.get(coord).is_some_and(|positions| {
                    positions
                        .iter()
                        .any(|position| band.accepts_existing(position.level))
                })
                && surface_by_coord.contains_key(coord);
            open[index] = u8::from(admitted);
        }
        Some(Self {
            minimum_q,
            minimum_r,
            q_count,
            r_count,
            open,
        })
    }

    fn index(&self, coord: HexCoord) -> Option<usize> {
        let q = usize::try_from(coord.x().checked_sub(self.minimum_q)?).ok()?;
        let r = usize::try_from(coord.y().checked_sub(self.minimum_r)?).ok()?;
        if q >= self.q_count || r >= self.r_count {
            return None;
        }
        q.checked_mul(self.r_count)
            .and_then(|base| base.checked_add(r))
            .filter(|index| *index < self.open.len())
    }

    fn is_open(&self, coord: HexCoord) -> bool {
        self.index(coord)
            .and_then(|index| self.open.get(index))
            .copied()
            == Some(1)
    }

    fn metric(&self, metric: &[u32], coord: HexCoord) -> Option<u32> {
        self.index(coord)
            .and_then(|index| metric.get(index))
            .copied()
            .filter(|value| *value != u32::MAX)
    }
}

fn required_connector_reverse_distances(
    domain: &RequiredConnectorDomain,
    band: OrdinaryRegionBand,
    network_by_coord: &BTreeMap<HexCoord, Vec<TilePos>>,
) -> Vec<u32> {
    let mut distances = vec![u32::MAX; domain.open.len()];
    let mut frontier = VecDeque::new();
    for (network_coord, positions) in network_by_coord {
        if !positions
            .iter()
            .any(|position| band.accepts_existing(position.level))
        {
            continue;
        }
        let mut neighbors = network_coord.neighbors();
        neighbors.sort_unstable();
        for neighbor in neighbors {
            let Some(index) = domain.index(neighbor) else {
                continue;
            };
            if domain.is_open(neighbor) && distances[index] == u32::MAX {
                distances[index] = 0;
                frontier.push_back(neighbor);
            }
        }
    }
    while let Some(coord) = frontier.pop_front() {
        let Some(index) = domain.index(coord) else {
            continue;
        };
        let distance = distances[index];
        if distance >= MAXIMUM_ORDINARY_CONNECTOR_SEARCH_STEPS {
            continue;
        }
        let mut neighbors = coord.neighbors();
        neighbors.sort_unstable();
        for neighbor in neighbors {
            let Some(neighbor_index) = domain.index(neighbor) else {
                continue;
            };
            if distances[neighbor_index] != u32::MAX || !domain.is_open(neighbor) {
                continue;
            }
            distances[neighbor_index] = distance.saturating_add(1);
            frontier.push_back(neighbor);
        }
    }
    distances
}

fn required_connector_reverse_costs(
    domain: &RequiredConnectorDomain,
    band: OrdinaryRegionBand,
    preserved_pass_through: &BTreeSet<HexCoord>,
    network_by_coord: &BTreeMap<HexCoord, Vec<TilePos>>,
) -> Vec<u32> {
    let mut costs = vec![u32::MAX; domain.open.len()];
    let mut frontier = BinaryHeap::<Reverse<(u32, HexCoord)>>::new();
    for (network_coord, positions) in network_by_coord {
        if !positions
            .iter()
            .any(|position| band.accepts_existing(position.level))
        {
            continue;
        }
        let mut neighbors = network_coord.neighbors();
        neighbors.sort_unstable();
        for neighbor in neighbors {
            let Some(index) = domain.index(neighbor) else {
                continue;
            };
            if domain.is_open(neighbor) && costs[index] == u32::MAX {
                costs[index] = 0;
                frontier.push(Reverse((0, neighbor)));
            }
        }
    }
    while let Some(Reverse((cost, coord))) = frontier.pop() {
        if domain.metric(&costs, coord) != Some(cost) {
            continue;
        }
        let Some(preserved_cost) = ORDINARY_CONNECTOR_PRESERVED_COST
            .checked_mul(u32::from(preserved_pass_through.contains(&coord)))
        else {
            continue;
        };
        let Some(next_cost) = cost
            .checked_add(1)
            .and_then(|cost| cost.checked_add(preserved_cost))
        else {
            continue;
        };
        let mut predecessors = coord.neighbors();
        predecessors.sort_unstable();
        for predecessor in predecessors {
            let Some(index) = domain.index(predecessor) else {
                continue;
            };
            if !domain.is_open(predecessor) || costs[index] <= next_cost {
                continue;
            }
            costs[index] = next_cost;
            frontier.push(Reverse((next_cost, predecessor)));
        }
    }
    costs
}

#[allow(clippy::too_many_arguments)]
fn ordinary_connector_coord_is_open(
    coord: HexCoord,
    band: OrdinaryRegionBand,
    footprint: &BTreeSet<HexCoord>,
    ordinary_mask: &BTreeSet<HexCoord>,
    hard_forbidden: &BTreeSet<HexCoord>,
    preserved_pass_through: &BTreeSet<HexCoord>,
    network_by_coord: &BTreeMap<HexCoord, Vec<TilePos>>,
    surface_by_coord: &BTreeMap<HexCoord, TilePos>,
) -> bool {
    footprint.contains(&coord)
        && ordinary_mask.contains(&coord)
        && (!hard_forbidden.contains(&coord) || preserved_pass_through.contains(&coord))
        && (!preserved_pass_through.contains(&coord)
            || surface_by_coord
                .get(&coord)
                .is_some_and(|surface| band.accepts_existing(surface.level)))
        && !network_by_coord.get(&coord).is_some_and(|positions| {
            positions
                .iter()
                .any(|position| band.accepts_existing(position.level))
        })
        && surface_by_coord.contains_key(&coord)
}

#[allow(clippy::too_many_arguments)]
fn ordinary_connector_reverse_distances(
    band: OrdinaryRegionBand,
    footprint: &BTreeSet<HexCoord>,
    ordinary_mask: &BTreeSet<HexCoord>,
    hard_forbidden: &BTreeSet<HexCoord>,
    preserved_pass_through: &BTreeSet<HexCoord>,
    network_by_coord: &BTreeMap<HexCoord, Vec<TilePos>>,
    surface_by_coord: &BTreeMap<HexCoord, TilePos>,
) -> BTreeMap<HexCoord, u32> {
    let mut distances = BTreeMap::<HexCoord, u32>::new();
    let mut frontier = VecDeque::new();
    for (network_coord, positions) in network_by_coord {
        if !positions
            .iter()
            .any(|position| band.accepts_existing(position.level))
        {
            continue;
        }
        let mut neighbors = network_coord.neighbors();
        neighbors.sort_unstable();
        for neighbor in neighbors {
            if ordinary_connector_coord_is_open(
                neighbor,
                band,
                footprint,
                ordinary_mask,
                hard_forbidden,
                preserved_pass_through,
                network_by_coord,
                surface_by_coord,
            ) && !distances.contains_key(&neighbor)
            {
                distances.insert(neighbor, 0);
                frontier.push_back(neighbor);
            }
        }
    }
    while let Some(coord) = frontier.pop_front() {
        let Some(distance) = distances.get(&coord).copied() else {
            continue;
        };
        if distance >= MAXIMUM_ORDINARY_CONNECTOR_SEARCH_STEPS {
            continue;
        }
        let mut neighbors = coord.neighbors();
        neighbors.sort_unstable();
        for neighbor in neighbors {
            if distances.contains_key(&neighbor)
                || !ordinary_connector_coord_is_open(
                    neighbor,
                    band,
                    footprint,
                    ordinary_mask,
                    hard_forbidden,
                    preserved_pass_through,
                    network_by_coord,
                    surface_by_coord,
                )
            {
                continue;
            }
            distances.insert(neighbor, distance.saturating_add(1));
            frontier.push_back(neighbor);
        }
    }
    distances
}

fn ordinary_connector_levels(
    preferred_start: Level,
    target: Level,
    target_headroom: Level,
    count: usize,
    band: OrdinaryRegionBand,
) -> Option<Vec<Level>> {
    if count < 2 || !band.accepts_existing(target) {
        return None;
    }
    // A target with only the minimum two levels of headroom needs a flat final
    // transition. Outdoor footing with at least three clear levels can admit a
    // one-level final step in either direction under the exact walker aperture
    // contract.
    let flat_final = target_headroom < 3;
    let mutable_count = count.saturating_sub(usize::from(flat_final));
    let mutable_transitions = i32::try_from(mutable_count.saturating_sub(1)).ok()?;
    let mut minimum = target.saturating_sub(mutable_transitions);
    let mut maximum = target.saturating_add(mutable_transitions);
    match band {
        OrdinaryRegionBand::Lower => {
            maximum = maximum.min(UPPER_REGION_THRESHOLD.saturating_sub(2));
        }
        OrdinaryRegionBand::Upper => {
            minimum = minimum.max(UPPER_REGION_THRESHOLD.saturating_add(1));
        }
    }
    if minimum > maximum {
        return None;
    }
    let start = preferred_start.clamp(minimum, maximum);
    let mut levels = if start >= target {
        descending_levels(start, target, mutable_count)
    } else {
        let mut ascending = descending_levels(target, start, mutable_count);
        ascending.reverse();
        ascending
    };
    if flat_final {
        levels.push(target);
    }
    (levels.len() == count
        && levels
            .iter()
            .take(count.saturating_sub(1))
            .all(|level| band.accepts_new(*level))
        && levels.windows(2).all(|pair| pair[0].abs_diff(pair[1]) <= 1))
    .then_some(levels)
}

fn ordinary_connector_levels_with_preserved_banks(
    path: &[HexCoord],
    preferred_start: Level,
    target: TilePos,
    target_headroom: Level,
    band: OrdinaryRegionBand,
    preserved_banks: &BTreeSet<HexCoord>,
    surface_by_coord: &BTreeMap<HexCoord, TilePos>,
) -> Option<Vec<Level>> {
    if path.len() < 2 || path.last().copied() != Some(target.coord) {
        return None;
    }
    if !path.iter().any(|coord| preserved_banks.contains(coord)) {
        return ordinary_connector_levels(
            preferred_start,
            target.level,
            target_headroom,
            path.len(),
            band,
        );
    }

    let flat_final = target_headroom < 3;
    let new_interval = match band {
        OrdinaryRegionBand::Lower => (0, UPPER_REGION_THRESHOLD.saturating_sub(2)),
        OrdinaryRegionBand::Upper => (UPPER_REGION_THRESHOLD.saturating_add(1), MAX_V3_LEVEL),
    };
    let mut allowed = Vec::with_capacity(path.len());
    for (index, coord) in path.iter().copied().enumerate() {
        let route_fixed = if index == path.len().saturating_sub(1)
            || (flat_final && index == path.len().saturating_sub(2))
        {
            Some(target.level)
        } else {
            None
        };
        let preserved_fixed = if preserved_banks.contains(&coord) {
            surface_by_coord.get(&coord).map(|surface| surface.level)
        } else {
            None
        };
        let fixed = match (route_fixed, preserved_fixed) {
            (Some(route), Some(preserved)) if route != preserved => return None,
            (Some(route), _) => Some(route),
            (_, Some(preserved)) => Some(preserved),
            (None, None) => None,
        };
        let interval = fixed.map_or(new_interval, |level| (level, level));
        if interval.0 > interval.1 || !band.accepts_existing(interval.0) {
            return None;
        }
        allowed.push(interval);
    }

    let mut feasible = vec![(0, 0); path.len()];
    let last = path.len().saturating_sub(1);
    feasible[last] = allowed[last];
    for index in (0..last).rev() {
        let next = feasible[index.saturating_add(1)];
        let lower = allowed[index].0.max(next.0.saturating_sub(1));
        let upper = allowed[index].1.min(next.1.saturating_add(1));
        if lower > upper {
            return None;
        }
        feasible[index] = (lower, upper);
    }

    let mut levels = Vec::<Level>::with_capacity(path.len());
    for (index, coord) in path.iter().copied().enumerate() {
        let (mut lower, mut upper) = feasible[index];
        if let Some(previous) = levels.last().copied() {
            lower = lower.max(previous.saturating_sub(1));
            upper = upper.min(previous.saturating_add(1));
        }
        if lower > upper {
            return None;
        }
        let preferred = if index == 0 {
            preferred_start
        } else {
            surface_by_coord.get(&coord).map_or_else(
                || levels.last().copied().unwrap_or(preferred_start),
                |surface| surface.level,
            )
        };
        levels.push(preferred.clamp(lower, upper));
    }
    (levels.last().copied() == Some(target.level)
        && levels.windows(2).all(|pair| pair[0].abs_diff(pair[1]) <= 1))
    .then_some(levels)
}

#[allow(clippy::too_many_arguments)]
fn solve_ordinary_connector_candidates(
    skeletons: Vec<(Vec<HexCoord>, TilePos)>,
    preferred_start: Level,
    band: OrdinaryRegionBand,
    exact_start: bool,
    volume: &VolumePlan,
    footprint: &BTreeSet<HexCoord>,
    ordinary_mask: &BTreeSet<HexCoord>,
    hard_forbidden: &BTreeSet<HexCoord>,
    preserved_banks: &BTreeSet<HexCoord>,
    network_by_coord: &BTreeMap<HexCoord, Vec<TilePos>>,
    surface_by_coord: &BTreeMap<HexCoord, TilePos>,
) -> Option<(Vec<HexCoord>, TilePos, Vec<Level>)> {
    let mut unresolved = Vec::new();
    for (path, target) in skeletons {
        let target_headroom = volume
            .surface_headroom(target)
            .map_or(0, |headroom| headroom.0);
        let solved = ordinary_connector_levels_with_preserved_banks(
            &path,
            preferred_start,
            target,
            target_headroom,
            band,
            preserved_banks,
            surface_by_coord,
        )
        .filter(|levels| !exact_start || levels.first().copied() == Some(preferred_start));
        if let Some(levels) = solved {
            return Some((path, target, levels));
        }
        let Some(deficit) = connector_total_vertical_deficit(
            &path,
            preferred_start,
            target,
            target_headroom,
            exact_start,
            band,
            preserved_banks,
            surface_by_coord,
        ) else {
            continue;
        };
        unresolved.push((
            deficit,
            target.level.abs_diff(preferred_start),
            target,
            path,
        ));
    }
    unresolved.sort_unstable();
    for (_, _, target, path) in unresolved.into_iter().take(8) {
        let target_headroom = volume
            .surface_headroom(target)
            .map_or(0, |headroom| headroom.0);
        if let Some((path, levels)) = inflate_connector_switchbacks(
            &path,
            preferred_start,
            target,
            target_headroom,
            band,
            exact_start,
            footprint,
            ordinary_mask,
            hard_forbidden,
            preserved_banks,
            network_by_coord,
            surface_by_coord,
        ) {
            return Some((path, target, levels));
        }
    }
    None
}

fn connector_total_vertical_deficit(
    path: &[HexCoord],
    preferred_start: Level,
    target: TilePos,
    target_headroom: Level,
    exact_start: bool,
    band: OrdinaryRegionBand,
    preserved_banks: &BTreeSet<HexCoord>,
    surface_by_coord: &BTreeMap<HexCoord, TilePos>,
) -> Option<u32> {
    let fixed = connector_fixed_anchors(
        path,
        preferred_start,
        target,
        target_headroom,
        exact_start,
        preserved_banks,
        surface_by_coord,
    )?;
    if fixed
        .iter()
        .any(|(_, level)| !band.accepts_existing(*level))
    {
        return None;
    }
    fixed.windows(2).try_fold(0_u32, |total, pair| {
        let edges = u32::try_from(pair[1].0.saturating_sub(pair[0].0)).ok()?;
        Some(total.saturating_add(pair[0].1.abs_diff(pair[1].1).saturating_sub(edges)))
    })
}

#[allow(clippy::too_many_arguments)]
fn inflate_connector_switchbacks(
    skeleton: &[HexCoord],
    preferred_start: Level,
    target: TilePos,
    target_headroom: Level,
    band: OrdinaryRegionBand,
    exact_start: bool,
    footprint: &BTreeSet<HexCoord>,
    ordinary_mask: &BTreeSet<HexCoord>,
    hard_forbidden: &BTreeSet<HexCoord>,
    preserved_banks: &BTreeSet<HexCoord>,
    network_by_coord: &BTreeMap<HexCoord, Vec<TilePos>>,
    surface_by_coord: &BTreeMap<HexCoord, TilePos>,
) -> Option<(Vec<HexCoord>, Vec<Level>)> {
    const MAXIMUM_CONNECTOR_CELLS: usize = 193;

    let mut path = skeleton.to_vec();
    if path.len() < 2
        || path.len() > MAXIMUM_CONNECTOR_CELLS
        || path.last().copied() != Some(target.coord)
        || path.iter().copied().collect::<BTreeSet<_>>().len() != path.len()
    {
        return None;
    }

    loop {
        if let Some(levels) = ordinary_connector_levels_with_preserved_banks(
            &path,
            preferred_start,
            target,
            target_headroom,
            band,
            preserved_banks,
            surface_by_coord,
        )
        .filter(|levels| !exact_start || levels.first().copied() == Some(preferred_start))
        {
            return Some((path, levels));
        }
        if path.len() >= MAXIMUM_CONNECTOR_CELLS {
            return None;
        }

        let fixed = connector_fixed_anchors(
            &path,
            preferred_start,
            target,
            target_headroom,
            exact_start,
            preserved_banks,
            surface_by_coord,
        )?;
        let mut deficient_segment = None;
        for (anchor_index, pair) in fixed.windows(2).enumerate() {
            let (start_index, start_level) = pair[0];
            let (end_index, end_level) = pair[1];
            let edges = u32::try_from(end_index.saturating_sub(start_index)).ok()?;
            let deficit = start_level.abs_diff(end_level).saturating_sub(edges);
            if deficit > deficient_segment.map_or(0, |(_, _, _, current)| current) {
                deficient_segment = Some((anchor_index, start_index, end_index, deficit));
            }
        }
        let (anchor_index, start_index, end_index, deficit) = deficient_segment?;
        if deficit == 0 || start_index >= end_index {
            return None;
        }

        let end = path[end_index];
        let end_level = fixed.get(anchor_index.saturating_add(1))?.1;
        let mut replacement = None;
        // The closest fixed bank can itself sit in a one-cell throat beside a
        // protected route. If it cannot launch a switchback, walk backward
        // through earlier fixed anchors and replace the enclosed bank chain as
        // one longer, bank-preserving detour.
        for earlier_anchor in (0..=anchor_index).rev() {
            let (candidate_start_index, candidate_start_level) = fixed[earlier_anchor];
            let start = path[candidate_start_index];
            let required_edges = candidate_start_level.abs_diff(end_level);
            let outside_cells = path.len().saturating_sub(
                end_index
                    .saturating_sub(candidate_start_index)
                    .saturating_add(1),
            );
            let maximum_segment_cells = MAXIMUM_CONNECTOR_CELLS.saturating_sub(outside_cells);
            let Some(detour) = connector_switchback_detour(
                start,
                end,
                required_edges,
                maximum_segment_cells,
                &path,
                candidate_start_index,
                end_index,
                footprint,
                ordinary_mask,
                hard_forbidden,
                preserved_banks,
                network_by_coord,
                surface_by_coord,
            ) else {
                continue;
            };
            replacement = Some((candidate_start_index, detour));
            break;
        }
        let (replacement_start, detour) = replacement?;
        path.splice(replacement_start..=end_index, detour);
    }
}

#[allow(clippy::too_many_arguments)]
fn connector_switchback_detour(
    start: HexCoord,
    end: HexCoord,
    required_edges: u32,
    maximum_cells: usize,
    complete_path: &[HexCoord],
    replaced_start: usize,
    replaced_end: usize,
    footprint: &BTreeSet<HexCoord>,
    ordinary_mask: &BTreeSet<HexCoord>,
    hard_forbidden: &BTreeSet<HexCoord>,
    preserved_banks: &BTreeSet<HexCoord>,
    network_by_coord: &BTreeMap<HexCoord, Vec<TilePos>>,
    surface_by_coord: &BTreeMap<HexCoord, TilePos>,
) -> Option<Vec<HexCoord>> {
    const WAYPOINT_RADIUS: u32 = 64;
    const MAXIMUM_WAYPOINTS: usize = 512;

    let maximum_edges = u32::try_from(maximum_cells.saturating_sub(1)).ok()?;
    if required_edges > maximum_edges {
        return None;
    }
    let outside_path = complete_path
        .iter()
        .enumerate()
        .filter_map(|(index, coord)| {
            (index < replaced_start || index > replaced_end).then_some(*coord)
        })
        .collect::<BTreeSet<_>>();
    let mut waypoints = start
        .within_radius(WAYPOINT_RADIUS)
        .into_iter()
        .filter(|waypoint| {
            let lower_bound = start
                .distance(*waypoint)
                .saturating_add(waypoint.distance(end));
            lower_bound >= required_edges
                && lower_bound <= maximum_edges
                && *waypoint != start
                && *waypoint != end
                && connector_detour_coord_is_open(
                    *waypoint,
                    start,
                    end,
                    footprint,
                    ordinary_mask,
                    hard_forbidden,
                    preserved_banks,
                    network_by_coord,
                    surface_by_coord,
                    &outside_path,
                )
        })
        .collect::<Vec<_>>();
    waypoints.sort_unstable_by_key(|waypoint| {
        let lower_bound = start
            .distance(*waypoint)
            .saturating_add(waypoint.distance(end));
        (
            lower_bound.saturating_sub(required_edges),
            start.distance(*waypoint).abs_diff(waypoint.distance(end)),
            *waypoint,
        )
    });

    for waypoint in waypoints.into_iter().take(MAXIMUM_WAYPOINTS) {
        let Some(first) = connector_detour_shortest_path(
            start,
            waypoint,
            maximum_edges,
            footprint,
            ordinary_mask,
            hard_forbidden,
            preserved_banks,
            network_by_coord,
            surface_by_coord,
            &outside_path,
            end,
        ) else {
            continue;
        };
        let mut blocked = outside_path.clone();
        blocked.extend(first.iter().copied().filter(|coord| *coord != waypoint));
        let remaining =
            maximum_edges.saturating_sub(u32::try_from(first.len().saturating_sub(1)).ok()?);
        let Some(second) = connector_detour_shortest_path(
            waypoint,
            end,
            remaining,
            footprint,
            ordinary_mask,
            hard_forbidden,
            preserved_banks,
            network_by_coord,
            surface_by_coord,
            &blocked,
            start,
        ) else {
            continue;
        };
        let mut detour = first;
        detour.extend(second.into_iter().skip(1));
        let edges = u32::try_from(detour.len().saturating_sub(1)).ok()?;
        if edges >= required_edges
            && detour.len() <= maximum_cells
            && detour.iter().copied().collect::<BTreeSet<_>>().len() == detour.len()
        {
            return Some(detour);
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn connector_detour_shortest_path(
    start: HexCoord,
    target: HexCoord,
    maximum_edges: u32,
    footprint: &BTreeSet<HexCoord>,
    ordinary_mask: &BTreeSet<HexCoord>,
    hard_forbidden: &BTreeSet<HexCoord>,
    preserved_banks: &BTreeSet<HexCoord>,
    network_by_coord: &BTreeMap<HexCoord, Vec<TilePos>>,
    surface_by_coord: &BTreeMap<HexCoord, TilePos>,
    blocked: &BTreeSet<HexCoord>,
    additionally_blocked_endpoint: HexCoord,
) -> Option<Vec<HexCoord>> {
    let mut parent = BTreeMap::<HexCoord, HexCoord>::new();
    let mut distance = BTreeMap::from([(start, 0_u32)]);
    let mut frontier = VecDeque::from([start]);
    while let Some(coord) = frontier.pop_front() {
        if coord == target {
            break;
        }
        let steps = distance.get(&coord).copied()?;
        if steps >= maximum_edges {
            continue;
        }
        let mut neighbors = coord.neighbors();
        neighbors.sort_unstable();
        for neighbor in neighbors {
            if distance.contains_key(&neighbor)
                || neighbor == additionally_blocked_endpoint
                || (neighbor != target
                    && !connector_detour_coord_is_open(
                        neighbor,
                        start,
                        target,
                        footprint,
                        ordinary_mask,
                        hard_forbidden,
                        preserved_banks,
                        network_by_coord,
                        surface_by_coord,
                        blocked,
                    ))
            {
                continue;
            }
            distance.insert(neighbor, steps.saturating_add(1));
            parent.insert(neighbor, coord);
            frontier.push_back(neighbor);
        }
    }
    distance.get(&target)?;
    let mut reversed = vec![target];
    let mut cursor = target;
    while cursor != start {
        cursor = *parent.get(&cursor)?;
        reversed.push(cursor);
    }
    reversed.reverse();
    Some(reversed)
}

#[allow(clippy::too_many_arguments)]
fn connector_detour_coord_is_open(
    coord: HexCoord,
    start: HexCoord,
    end: HexCoord,
    footprint: &BTreeSet<HexCoord>,
    ordinary_mask: &BTreeSet<HexCoord>,
    hard_forbidden: &BTreeSet<HexCoord>,
    preserved_banks: &BTreeSet<HexCoord>,
    network_by_coord: &BTreeMap<HexCoord, Vec<TilePos>>,
    surface_by_coord: &BTreeMap<HexCoord, TilePos>,
    blocked: &BTreeSet<HexCoord>,
) -> bool {
    coord == start
        || coord == end
        || (footprint.contains(&coord)
            && ordinary_mask.contains(&coord)
            && !hard_forbidden.contains(&coord)
            && !preserved_banks.contains(&coord)
            && !network_by_coord.contains_key(&coord)
            && surface_by_coord.contains_key(&coord)
            && !blocked.contains(&coord))
}

fn connector_fixed_anchors(
    path: &[HexCoord],
    preferred_start: Level,
    target: TilePos,
    target_headroom: Level,
    exact_start: bool,
    preserved_banks: &BTreeSet<HexCoord>,
    surface_by_coord: &BTreeMap<HexCoord, TilePos>,
) -> Option<Vec<(usize, Level)>> {
    let mut fixed = BTreeMap::<usize, Level>::new();
    let mut insert = |index: usize, level: Level| -> Option<()> {
        if fixed
            .insert(index, level)
            .is_some_and(|current| current != level)
        {
            return None;
        }
        Some(())
    };
    if exact_start {
        insert(0, preferred_start)?;
    }
    for (index, coord) in path.iter().copied().enumerate() {
        if preserved_banks.contains(&coord) {
            insert(index, surface_by_coord.get(&coord)?.level)?;
        }
    }
    let last = path.len().checked_sub(1)?;
    if target_headroom < 3 {
        insert(last.checked_sub(1)?, target.level)?;
    }
    insert(last, target.level)?;
    Some(fixed.into_iter().collect())
}

#[allow(clippy::too_many_arguments)]
fn connector_skeleton_diagnostic(
    ordinal: usize,
    path: &[HexCoord],
    preferred_start: Level,
    target: TilePos,
    band: OrdinaryRegionBand,
    volume: &VolumePlan,
    preserved_surfaces: &BTreeSet<HexCoord>,
    surface_by_coord: &BTreeMap<HexCoord, TilePos>,
) -> String {
    let target_headroom = volume
        .surface_headroom(target)
        .map_or(0, |headroom| headroom.0);
    let penultimate = path
        .get(path.len().saturating_sub(2))
        .copied()
        .map(|coord| {
            (
                coord,
                preserved_surfaces.contains(&coord),
                surface_by_coord.get(&coord).copied(),
            )
        });
    let Some(fixed) = connector_fixed_anchors(
        path,
        preferred_start,
        target,
        target_headroom,
        true,
        preserved_surfaces,
        surface_by_coord,
    ) else {
        let preserved_anchors = path
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, coord)| preserved_surfaces.contains(coord))
            .map(|(index, coord)| (index, coord, surface_by_coord.get(&coord).copied()))
            .collect::<Vec<_>>();
        return format!(
            "#{ordinal} target={target:?} headroom={target_headroom} penultimate={penultimate:?} preserved_anchors={preserved_anchors:?} rejected=fixed-anchor collision-or-missing-surface"
        );
    };
    let anchors = fixed
        .iter()
        .filter_map(|(index, level)| {
            path.get(*index)
                .copied()
                .map(|coord| (*index, coord, *level))
        })
        .collect::<Vec<_>>();
    let maximum_deficit = fixed
        .windows(2)
        .map(|pair| {
            let edges = u32::try_from(pair[1].0.saturating_sub(pair[0].0)).unwrap_or(u32::MAX);
            pair[0].1.abs_diff(pair[1].1).saturating_sub(edges)
        })
        .max()
        .unwrap_or_default();
    let direct = ordinary_connector_levels_with_preserved_banks(
        path,
        preferred_start,
        target,
        target_headroom,
        band,
        preserved_surfaces,
        surface_by_coord,
    );
    let rejection = match direct {
        Some(levels) if levels.first().copied() == Some(preferred_start) => "none",
        Some(_) => "exact-start-drift",
        None if fixed
            .iter()
            .any(|(_, level)| !band.accepts_existing(*level)) =>
        {
            "fixed-anchor-outside-band"
        }
        None if maximum_deficit > 0 => "fixed-anchor-vertical-deficit",
        None => "interval-or-flat-final-conflict",
    };
    format!(
        "#{ordinal} target={target:?} headroom={target_headroom} penultimate={penultimate:?} anchors={anchors:?} max_deficit={maximum_deficit} rejected={rejection}"
    )
}

/// Exact surfaces whose authored identity must never be hidden by an
/// inaccessible-access demotion.
///
/// The additional iterator covers protected surfaces still being constructed
/// and not yet published in the world's feature plan.
fn authored_ordinary_surface_authority(
    world: &GeneratedWorldPlan,
    additional: impl IntoIterator<Item = TilePos>,
) -> BTreeSet<TilePos> {
    world
        .features
        .protected_routes
        .values()
        .flat_map(|route| route.surfaces.iter().copied())
        .chain(
            world
                .interiors
                .by_id
                .values()
                .flat_map(|interior| interior.floors.iter().copied()),
        )
        .chain(
            world
                .interiors
                .by_id
                .values()
                .flat_map(|interior| interior.entrances.iter().copied()),
        )
        .chain(world.lights.values().map(|light| light.origin))
        .chain(
            world
                .structures
                .by_id
                .values()
                .flat_map(|structure| structure.voxels.iter().copied()),
        )
        .chain(world.anchors.values().copied())
        .chain(additional)
        .collect()
}

/// Reclassifies only incidental walker surfaces that final blockers separated
/// after the constructive hub pass.
///
/// Authored surfaces remain untouched so the final complete-reachability audit
/// rejects an authored route or review promise that decoration disconnected.
/// The supplied reachability map was computed before these demotions; because
/// only nodes absent from that map are removed, its reachable component remains
/// exact and no second whole-world traversal is necessary.
fn reconcile_final_incidental_ordinary_access(
    volume: &mut VolumePlan,
    blockers: &BTreeSet<TilePos>,
    graph: &mut OrdinaryGraph,
    reachable: &BTreeMap<TilePos, u32>,
    authored_surfaces: &BTreeSet<TilePos>,
) -> Result<usize, V3GenerationError> {
    let demotions = graph
        .positions()
        .filter(|position| {
            !reachable.contains_key(position) && !authored_surfaces.contains(position)
        })
        .collect::<Vec<_>>();
    let mut changed_coords = BTreeSet::new();
    for position in &demotions {
        let metadata = volume.surfaces.get_mut(position).ok_or_else(|| {
            schematic_contract(format!(
                "final incidental Ordinary reconciliation lost graph surface {position:?}"
            ))
        })?;
        if metadata.access != SurfaceAccess::Ordinary || blockers.contains(position) {
            return Err(schematic_contract(format!(
                "final incidental Ordinary reconciliation found stale graph surface {position:?} \
                 (metadata={metadata:?}, blocker={})",
                blockers.contains(position),
            )));
        }
        metadata.access = SurfaceAccess::SpecialMovement(INACCESSIBLE_MOVEMENT_REGION);
        changed_coords.insert(position.coord);
    }
    if !changed_coords.is_empty() {
        let _affected = graph.refresh_coords(volume, Some(blockers), changed_coords);
    }
    Ok(demotions.len())
}

fn measure_ordinary_hub_network_with_reachability(
    plan: &SchematicPlanV1,
    world: &GeneratedWorldPlan,
    graph: &OrdinaryGraph,
    reachable: &BTreeMap<TilePos, u32>,
) -> Result<OrdinaryReachability, V3GenerationError> {
    validate_complete_final_ordinary_reachability(
        &world.volume,
        &world.blockers,
        graph,
        reachable,
    )?;
    let route = world
        .features
        .protected_routes
        .get("grand_v3.ordinary_hubs")
        .ok_or_else(|| schematic_contract("ordinary measurement has no protected hub route"))?;
    let ordinary_cells = plan
        .cells
        .iter()
        .filter(|cell| is_ordinary_land(cell))
        .collect::<Vec<_>>();
    if route.centerline.len() != ordinary_cells.len() {
        return Err(schematic_contract(format!(
            "ordinary hub route has {} hubs for {} ordinary cells",
            route.centerline.len(),
            ordinary_cells.len()
        )));
    }
    for (cell, hub) in ordinary_cells
        .into_iter()
        .zip(route.centerline.iter().copied())
    {
        let patch = world
            .layout
            .patches
            .get(&PatchId(u32::from(cell.id.get())))
            .ok_or_else(|| {
                schematic_contract(format!(
                    "ordinary measurement has no patch for cell {}",
                    cell.id.get()
                ))
            })?;
        if !patch.mask.contains(&hub.coord)
            || world.blockers.contains(&hub)
            || !graph.contains(hub)
            || !reachable.contains_key(&hub)
        {
            return Err(schematic_contract(format!(
                "ordinary cell {} hub {hub:?} is not dry, headroom-safe, and foothill-reachable",
                cell.id.get()
            )));
        }
    }
    for surface in &route.surfaces {
        let blocked = world.blockers.contains(surface);
        let metadata = world.volume.surfaces.get(surface);
        let in_graph = graph.contains(*surface);
        let is_reachable = reachable.contains_key(surface);
        if blocked
            || metadata.is_none_or(|metadata| metadata.access != SurfaceAccess::Ordinary)
            || !is_reachable
        {
            return Err(schematic_contract(format!(
                "protected ordinary network surface {surface:?} is blocked or unreachable \
                 (blocker={blocked}, metadata={metadata:?}, graph={in_graph}, \
                 reachable={is_reachable}, headroom={:?}, current={:?}, column={:?})",
                world.volume.surface_headroom(*surface),
                top_standable_surface(&world.volume, surface.coord),
                world.volume.columns.get(&surface.coord),
            )));
        }
    }
    for required in [
        "crystal_ascent.lower_entry",
        "crystal_ascent.upper_exit",
        "grand_v3.natural_pass",
        "grand_v3.ascent_threshold",
    ] {
        let position =
            world.anchors.get(required).copied().ok_or_else(|| {
                schematic_contract(format!("ordinary graph omits anchor {required}"))
            })?;
        if !reachable.contains_key(&position) {
            return Err(schematic_contract(format!(
                "ordinary graph cannot reach required anchor {required} at {position:?}"
            )));
        }
    }
    Ok(OrdinaryReachability {
        reachable_surfaces: u32::try_from(reachable.len()).unwrap_or(u32::MAX),
        reachable_elevation_levels: u32::try_from(
            reachable
                .keys()
                .map(|position| position.level)
                .collect::<BTreeSet<_>>()
                .len(),
        )
        .unwrap_or(u32::MAX),
    })
}

/// Proves that the final blocker-aware Ordinary projection is exactly one
/// foothill-reachable component.
///
/// Construction may demote incidental disconnected terrain while it builds the
/// coarse-cell hub network. Authored route, interior, light, structure, and
/// anchor surfaces are deliberately exempt from that provisional demotion so a
/// construction bug cannot erase authored intent. Once every author and final
/// blocker has run, however, every remaining `Ordinary` nonblocked surface is a
/// normal walker promise and must be reachable. Scenic and deliberately
/// inaccessible surfaces use `SpecialMovement`, while exact blocker positions
/// are omitted by the same predicate as [`OrdinaryGraph`].
fn validate_complete_final_ordinary_reachability(
    volume: &VolumePlan,
    blockers: &BTreeSet<TilePos>,
    graph: &OrdinaryGraph,
    reachable: &BTreeMap<TilePos, u32>,
) -> Result<(), V3GenerationError> {
    for (position, metadata) in &volume.surfaces {
        if !ordinary_surface_is_node(volume, Some(blockers), *position) {
            continue;
        }
        if !graph.contains(*position) {
            return Err(schematic_contract(format!(
                "final ordinary graph omits authoritative walker surface {position:?} \
                 (metadata={metadata:?}, blocker={}, headroom={:?})",
                blockers.contains(position),
                volume.surface_headroom(*position),
            )));
        }
        if !reachable.contains_key(position) {
            return Err(schematic_contract(format!(
                "final ordinary walker surface {position:?} is not reachable from the foothill \
                 (metadata={metadata:?}, blocker={}, headroom={:?}, admitted_neighbors={})",
                blockers.contains(position),
                volume.surface_headroom(*position),
                graph.neighbors(*position).len(),
            )));
        }
    }

    for position in graph.positions() {
        if !ordinary_surface_is_node(volume, Some(blockers), position) {
            return Err(schematic_contract(format!(
                "final ordinary graph retains stale non-walker surface {position:?} \
                 (metadata={:?}, blocker={})",
                volume.surfaces.get(&position),
                blockers.contains(&position),
            )));
        }
    }
    for position in reachable.keys() {
        if !graph.contains(*position) {
            return Err(schematic_contract(format!(
                "final foothill reachability retains non-graph surface {position:?}"
            )));
        }
    }
    Ok(())
}

/// Resolves the schematic vegetation layer into exact authored objects.
///
/// The coarse percentage is interpreted as horizontal canopy coverage over the
/// eligible fine columns owned by that cell. Roots are consumed in coherent
/// cluster order, but object family and rotation use independent coordinate
/// samples so adding grass or changing one distant cell cannot move a tree.
fn compile_schematic_vegetation(
    plan: &SchematicPlanV1,
    seed: u64,
    catalog: &RuntimeArtCatalog,
    crystal_mask: &BTreeSet<HexCoord>,
    world: &mut GeneratedWorldPlan,
) -> Result<(), V3GenerationError> {
    let temperate = TemperateVegetationSet::resolve(catalog, "Grand V3 schematic")
        .map_err(schematic_contract)?;
    let frozen =
        SnowyVegetationSet::resolve(catalog, "Grand V3 schematic").map_err(schematic_contract)?;
    let supports = world
        .volume
        .surfaces
        .iter()
        .filter(|(_, metadata)| metadata.access != SurfaceAccess::NonStandable)
        .fold(
            BTreeMap::<HexCoord, TilePos>::new(),
            |mut result, (position, _)| {
                result
                    .entry(position.coord)
                    .and_modify(|current| *current = (*current).max(*position))
                    .or_insert(*position);
                result
            },
        );

    let reserved = schematic_vegetation_reserved(world, crystal_mask, &world.blockers);

    let mut occupied_visual = BTreeSet::new();
    let mut occupied_blockers = world.blockers.clone();
    let mut next_id = VEGETATION_FEATURE_BASE;
    for cell in &plan.cells {
        let ecology = schematic_ecology::vegetation_policy(cell);
        if cell.facts.surface != SurfaceKind::Land
            || ecology.density == VegetationDensity::None
            || has_overlay(cell, SchematicFeature::CrystalAscent)
        {
            continue;
        }
        let patch_id = PatchId(u32::from(cell.id.get()));
        let patch = world.layout.patches.get(&patch_id).ok_or_else(|| {
            schematic_contract(format!(
                "vegetation cell {} has no resolved patch",
                cell.id.get()
            ))
        })?;
        let candidate_roots = patch
            .mask
            .iter()
            .copied()
            .filter(|coord| {
                supports.get(coord).is_some_and(|root| {
                    !reserved.contains(coord)
                        && schematic_ecology::tree_root_is_admitted(cell, *root, seed)
                })
            })
            .collect::<BTreeSet<_>>();
        if candidate_roots.is_empty() {
            continue;
        }
        let (tree_objects, eligibility_objects, grass): (
            Vec<&VegetationObjectSpec>,
            Vec<&VegetationObjectSpec>,
            &VegetationObjectSpec,
        ) = if ecology.family == VegetationFamily::Frozen {
            (
                vec![
                    &frozen.old_growth,
                    &frozen.small_broadleaf,
                    &frozen.tall_narrow,
                ],
                // Eligibility is an existential predicate. Trying the compact
                // silhouettes first produces the identical root set without
                // projecting every 125-cell old-growth canopy for the common
                // clear case. Family selection below retains its authored
                // old-growth/small/tall ordering and exact named stream.
                vec![
                    &frozen.small_broadleaf,
                    &frozen.tall_narrow,
                    &frozen.old_growth,
                ],
                &frozen.grass_tuft,
            )
        } else {
            (
                vec![
                    &temperate.old_growth,
                    &temperate.small_broadleaf,
                    &temperate.tall_narrow,
                ],
                vec![
                    &temperate.small_broadleaf,
                    &temperate.tall_narrow,
                    &temperate.old_growth,
                ],
                &temperate.grass_tuft,
            )
        };
        let tree_clearance_projections = tree_objects
            .iter()
            .map(|object| object.clearance_projections())
            .collect::<Vec<_>>();
        let eligible = exact_eligible_tree_roots(
            &candidate_roots,
            &supports,
            &eligibility_objects,
            &reserved,
            &occupied_visual,
            &occupied_blockers,
        );
        let target = vegetation_canopy_target(eligible.len(), ecology.density);
        if target == 0 {
            continue;
        }
        let roots = coherent_vegetation_roots(seed, cell, ecology.density, &eligible);
        let mut covered = BTreeSet::new();
        for root_coord in roots.iter().copied() {
            if covered.len() >= target {
                break;
            }
            let root = supports[&root_coord];
            let family = named_sample(seed, "vegetation_tree_family", root_coord);
            let family_start = if ecology.prefer_old_growth {
                0
            } else {
                usize::try_from(family % 3).unwrap_or_default()
            };
            let rotation_start =
                u8::try_from(named_sample(seed, "vegetation_tree_rotation", root_coord) % 6)
                    .unwrap_or_default();
            let mut accepted = None;
            for object_offset in 0..tree_objects.len() {
                let object_index = (family_start + object_offset) % tree_objects.len();
                let object = tree_objects[object_index];
                for rotation_offset in 0..6_u8 {
                    let rotation =
                        HexObjectRotation::new(rotation_start.saturating_add(rotation_offset) % 6)
                            .map_err(|error| schematic_contract(error.to_string()))?;
                    let Some(clearance) = tree_clearance_projections[object_index]
                        .get(usize::from(rotation.steps()))
                        .and_then(Option::as_ref)
                    else {
                        continue;
                    };
                    if !VegetationObjectSpec::precomputed_projection_is_clear(
                        root,
                        clearance,
                        &supports,
                        &reserved,
                        &occupied_visual,
                        &occupied_blockers,
                    ) {
                        continue;
                    }
                    let Some(blockers) = object.project_blockers(root, rotation, &supports) else {
                        continue;
                    };
                    let Some(visual) = object.project_visual_volume(root, rotation) else {
                        continue;
                    };
                    accepted = Some((object, rotation, visual.cells, blockers));
                    break;
                }
                if accepted.is_some() {
                    break;
                }
            }
            let Some((object, rotation, visual, blockers)) = accepted else {
                continue;
            };
            covered.extend(
                visual
                    .iter()
                    .map(|voxel| voxel.coord)
                    .filter(|coord| eligible.contains(coord)),
            );
            occupied_visual.extend(visual);
            occupied_blockers.extend(blockers.iter().copied());
            insert_world_vegetation_feature(
                &mut world.features,
                &mut next_id,
                PlannedFeature {
                    root,
                    kind: FeatureKind::Tree,
                    object_id: object.id.clone(),
                    rotation,
                    blocker_footprint: blockers,
                },
            )?;
        }
        if covered.len() < target {
            return Err(schematic_contract(format!(
                "vegetation cell {} reached only {}/{} authored canopy columns",
                cell.id.get(),
                covered.len(),
                target
            )));
        }

        // Tufts are a secondary surface detail rather than canopy authority.
        // Keep them sparse enough to remain readable and independently seeded.
        let grass_target = target.div_ceil(20);
        let mut grass_count = 0_usize;
        for root_coord in roots.iter().rev().copied() {
            if grass_count >= grass_target
                || reserved.contains(&root_coord)
                || covered.contains(&root_coord)
            {
                continue;
            }
            let root = supports[&root_coord];
            let rotation = HexObjectRotation::new(
                u8::try_from(named_sample(seed, "vegetation_grass_rotation", root_coord) % 6)
                    .unwrap_or_default(),
            )
            .map_err(|error| schematic_contract(error.to_string()))?;
            if !grass.projection_is_clear(
                root,
                rotation,
                &supports,
                &reserved,
                &occupied_visual,
                &occupied_blockers,
            ) {
                continue;
            }
            let Some(visual) = grass.project_visual_volume(root, rotation) else {
                continue;
            };
            occupied_visual.extend(visual.cells);
            insert_world_vegetation_feature(
                &mut world.features,
                &mut next_id,
                PlannedFeature {
                    root,
                    kind: FeatureKind::TallGrass,
                    object_id: grass.id.clone(),
                    rotation,
                    blocker_footprint: BTreeSet::new(),
                },
            )?;
            grass_count = grass_count.saturating_add(1);
        }
    }
    world.blockers = occupied_blockers;
    Ok(())
}

/// Exact horizontal domain that Grand's authored vegetation may not enter.
///
/// This is deliberately shared by placement and its postcondition tests: roots,
/// blockers, and the complete visual volume must all remain outside liquids,
/// architecture, ordinary routes, review/sight clearings, anchors, lights, and
/// the separately authored Crystal site.
fn schematic_vegetation_reserved(
    world: &GeneratedWorldPlan,
    crystal_mask: &BTreeSet<HexCoord>,
    preexisting_blockers: &BTreeSet<TilePos>,
) -> BTreeSet<HexCoord> {
    let mut reserved = crystal_mask.clone();
    reserved.extend(
        world
            .volume
            .fill_runs_by_top()
            .keys()
            .map(|position| position.coord),
    );
    reserved.extend(
        world
            .structures
            .by_id
            .values()
            .flat_map(|structure| structure.voxels.iter().map(|voxel| voxel.coord)),
    );
    reserved.extend(preexisting_blockers.iter().map(|position| position.coord));
    for route in world.features.protected_routes.values() {
        for surface in &route.surfaces {
            reserve_radius(&mut reserved, surface.coord, 2);
        }
    }
    for clearing in world.features.clearings.values() {
        reserved.extend(clearing.surfaces.iter().map(|surface| surface.coord));
    }
    for anchor in world.anchors.values() {
        reserve_radius(&mut reserved, anchor.coord, 3);
    }
    for light in world.lights.values() {
        reserve_radius(&mut reserved, light.origin.coord, 2);
    }
    reserved
}

const fn vegetation_coverage_percent(density: VegetationDensity) -> usize {
    match density {
        VegetationDensity::None => 0,
        VegetationDensity::Sparse => 4,
        VegetationDensity::Light => 14,
        VegetationDensity::Moderate => 30,
        VegetationDensity::Dense => 52,
    }
}

fn vegetation_canopy_target(eligible_roots: usize, density: VegetationDensity) -> usize {
    eligible_roots
        .saturating_mul(vegetation_coverage_percent(density))
        .saturating_add(99)
        / 100
}

fn exact_eligible_tree_roots(
    candidates: &BTreeSet<HexCoord>,
    supports: &BTreeMap<HexCoord, TilePos>,
    tree_objects: &[&VegetationObjectSpec],
    reserved: &BTreeSet<HexCoord>,
    occupied_visual: &BTreeSet<TilePos>,
    occupied_blockers: &BTreeSet<TilePos>,
) -> BTreeSet<HexCoord> {
    let clearance_projections = tree_objects
        .iter()
        .map(|object| object.clearance_projections())
        .collect::<Vec<_>>();
    candidates
        .iter()
        .copied()
        .filter(|coord| {
            let Some(root) = supports.get(coord).copied() else {
                return false;
            };
            clearance_projections.iter().any(|rotations| {
                rotations.iter().flatten().any(|projection| {
                    VegetationObjectSpec::precomputed_projection_is_clear(
                        root,
                        projection,
                        supports,
                        reserved,
                        occupied_visual,
                        occupied_blockers,
                    )
                })
            })
        })
        .collect()
}

fn coherent_vegetation_roots(
    seed: u64,
    cell: &CellPlan,
    density: VegetationDensity,
    eligible: &BTreeSet<HexCoord>,
) -> Vec<HexCoord> {
    let ordered = eligible.iter().copied().collect::<Vec<_>>();
    let cluster_count = match density {
        VegetationDensity::None | VegetationDensity::Sparse => 1,
        VegetationDensity::Light | VegetationDensity::Moderate => 2,
        VegetationDensity::Dense => 3,
    };
    let coarse_center = schematic_to_world(cell.coord, 22);
    let mut clusters = vec![coarse_center];
    for ordinal in 1..cluster_count {
        let sample = named_sample(
            seed ^ u64::from(cell.id.get()),
            "vegetation_clusters",
            step_in_direction(
                coarse_center,
                ordinal,
                i32::try_from(ordinal).unwrap_or_default(),
            ),
        );
        let index =
            usize::try_from(sample % u64::try_from(ordered.len()).unwrap_or(1)).unwrap_or_default();
        clusters.push(ordered[index]);
    }
    let mut keyed_roots = ordered
        .into_iter()
        .map(|coord| {
            (
                (
                    clusters
                        .iter()
                        .map(|cluster| cluster.distance(coord))
                        .min()
                        .unwrap_or_default(),
                    named_sample(seed, "vegetation_tree_roots", coord),
                    coord,
                ),
                coord,
            )
        })
        .collect::<Vec<_>>();
    keyed_roots.sort_unstable_by_key(|(key, _)| *key);
    keyed_roots.into_iter().map(|(_, coord)| coord).collect()
}

fn reserve_radius(reserved: &mut BTreeSet<HexCoord>, center: HexCoord, radius: u32) {
    reserved.extend(center.within_radius(radius));
}

fn insert_world_vegetation_feature(
    features: &mut FeaturePlan,
    next_id: &mut u32,
    feature: PlannedFeature,
) -> Result<(), V3GenerationError> {
    while features.by_id.contains_key(&FeatureId(*next_id)) {
        *next_id = next_id.saturating_add(1);
    }
    let id = FeatureId(*next_id);
    *next_id = next_id.saturating_add(1);
    if features.by_id.insert(id, feature).is_some() {
        return Err(schematic_contract(format!(
            "schematic vegetation reused feature id {id:?}"
        )));
    }
    Ok(())
}

fn seal_unplanned_upper_crossings(
    volume: &mut VolumePlan,
    features: &FeaturePlan,
) -> BTreeSet<HexCoord> {
    const UPPER_THRESHOLD: Level = 121;
    let protected = features
        .protected_routes
        .values()
        .flat_map(|route| route.surfaces.iter().copied())
        .collect::<BTreeSet<_>>();
    let ordinary = volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| {
            (metadata.access == SurfaceAccess::Ordinary).then_some(*position)
        })
        .collect::<BTreeSet<_>>();
    let ordinary_by_coord = ordinary.iter().copied().fold(
        BTreeMap::<HexCoord, Vec<TilePos>>::new(),
        |mut by_coord, position| {
            by_coord.entry(position.coord).or_default().push(position);
            by_coord
        },
    );
    let mut seal = BTreeSet::new();
    for upper in ordinary
        .iter()
        .copied()
        .filter(|position| position.level >= UPPER_THRESHOLD)
    {
        if protected.contains(&upper) {
            continue;
        }
        for neighbor_coord in upper.coord.neighbors() {
            if ordinary_by_coord.get(&neighbor_coord).is_some_and(|lower| {
                lower.iter().any(|lower| {
                    lower.level < UPPER_THRESHOLD
                        && lower.level.abs_diff(upper.level) <= 1
                        && !protected.contains(lower)
                })
            }) {
                seal.insert(upper);
            }
        }
    }
    let sealed_coords = seal.iter().map(|position| position.coord).collect();
    for position in seal {
        if let Some(metadata) = volume.surfaces.get_mut(&position) {
            metadata.access = SurfaceAccess::SpecialMovement(INACCESSIBLE_MOVEMENT_REGION);
        }
    }
    sealed_coords
}

/// Audits the seed-dependent surface entrance before the invariant Crystal
/// landmark is constructed.
///
/// The lightweight corpus admits the Crystal half of the two-route contract by
/// resolving its exact terminal/tunnel splice. At this earlier terrain stage,
/// every physical lower/upper threshold contact must therefore belong to the
/// independently seeded natural pass, with at least one such contact present.
fn validate_admitted_natural_upper_entry(
    volume: &VolumePlan,
    natural: &ProtectedFeatureRoute,
) -> Result<(), V3GenerationError> {
    let ordinary = volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| {
            (metadata.access == SurfaceAccess::Ordinary).then_some(*position)
        })
        .collect::<BTreeSet<_>>();
    let mut found_natural_crossing = false;
    for lower in ordinary
        .iter()
        .copied()
        .filter(|position| position.level < UPPER_REGION_THRESHOLD)
    {
        for upper in lower.coord.neighbors().into_iter().flat_map(|coord| {
            ordinary
                .range(TilePos::new(coord, UPPER_REGION_THRESHOLD)..)
                .take_while(move |position| position.coord == coord)
                .copied()
        }) {
            if lower.level.abs_diff(upper.level) > 1 {
                continue;
            }
            if natural.surfaces.contains(&lower) || natural.surfaces.contains(&upper) {
                found_natural_crossing = true;
            } else {
                return Err(schematic_contract(format!(
                    "topology admission found undeclared ordinary upper-region crossing {lower:?} -> {upper:?}"
                )));
            }
        }
    }
    if !found_natural_crossing {
        return Err(schematic_contract(
            "topology admission natural pass does not cross into the upper region",
        ));
    }
    Ok(())
}

fn validate_exact_upper_entrances(
    volume: &VolumePlan,
    features: &FeaturePlan,
) -> Result<(), V3GenerationError> {
    const UPPER_THRESHOLD: Level = 121;
    let natural = features
        .protected_routes
        .get("grand_v3.natural_pass")
        .ok_or_else(|| schematic_contract("upper-route audit has no natural pass"))?;
    let crystal = features
        .protected_routes
        .get("grand_v3.crystal_route")
        .ok_or_else(|| schematic_contract("upper-route audit has no Crystal route"))?;
    let ordinary = volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| {
            (metadata.access == SurfaceAccess::Ordinary).then_some(*position)
        })
        .collect::<BTreeSet<_>>();
    let mut natural_crossing = false;
    let mut crystal_crossing = false;
    for lower in ordinary
        .iter()
        .copied()
        .filter(|position| position.level < UPPER_THRESHOLD)
    {
        for upper in lower.coord.neighbors().into_iter().flat_map(|coord| {
            ordinary
                .range(TilePos::new(coord, UPPER_THRESHOLD)..)
                .take_while(move |position| position.coord == coord)
                .copied()
        }) {
            if upper.level < UPPER_THRESHOLD || lower.level.abs_diff(upper.level) > 1 {
                continue;
            }
            let is_natural = natural.surfaces.contains(&lower) || natural.surfaces.contains(&upper);
            let is_crystal = crystal.surfaces.contains(&lower) || crystal.surfaces.contains(&upper);
            match (is_natural, is_crystal) {
                (true, false) => natural_crossing = true,
                (false, true) => crystal_crossing = true,
                (false, false) => {
                    return Err(schematic_contract(format!(
                        "undeclared ordinary upper-region crossing {lower:?} -> {upper:?}"
                    )));
                }
                (true, true) => {
                    return Err(schematic_contract(format!(
                        "natural and Crystal upper routes overlap at {lower:?} -> {upper:?}"
                    )));
                }
            }
        }
    }
    if !natural_crossing || !crystal_crossing {
        return Err(schematic_contract(format!(
            "upper region requires exactly the natural and Crystal entrances (natural={natural_crossing}, Crystal={crystal_crossing})"
        )));
    }
    Ok(())
}

/// Proves the two authored upper entrances as graph cuts, not merely labels.
///
/// `seal_unplanned_upper_crossings` removes incidental threshold contacts and
/// `validate_exact_upper_entrances` classifies every surviving threshold edge.
/// Once the ordinary hub network has connected both elevation bands, this audit
/// verifies the stronger gameplay contract: either declared portal works on its
/// own, while removing both disconnects the foothill from the summit.
fn validate_exact_upper_route_cuts(
    world: &GeneratedWorldPlan,
    graph: &OrdinaryGraph,
) -> Result<BTreeMap<TilePos, u32>, V3GenerationError> {
    let natural = world
        .features
        .protected_routes
        .get("grand_v3.natural_pass")
        .ok_or_else(|| schematic_contract("upper-route cut audit has no natural pass"))?;
    let crystal = world
        .features
        .protected_routes
        .get("grand_v3.crystal_route")
        .ok_or_else(|| schematic_contract("upper-route cut audit has no Crystal route"))?;
    let foothill = world
        .anchors
        .get("grand_v3.tunnel_mouth")
        .copied()
        .ok_or_else(|| schematic_contract("upper-route cut audit has no foothill anchor"))?;
    let summit = world
        .anchors
        .get("crystal_ascent.upper_exit")
        .copied()
        .ok_or_else(|| schematic_contract("upper-route cut audit has no summit anchor"))?;
    validate_upper_route_cut_graph(graph, foothill, summit, natural, crystal)
}

fn validate_upper_route_cut_graph(
    graph: &OrdinaryGraph,
    foothill: TilePos,
    summit: TilePos,
    natural: &ProtectedFeatureRoute,
    crystal: &ProtectedFeatureRoute,
) -> Result<BTreeMap<TilePos, u32>, V3GenerationError> {
    if !graph.contains(foothill) || !graph.contains(summit) {
        return Err(schematic_contract(
            "upper-route cut endpoints are not exact ordinary walker nodes",
        ));
    }

    let mut natural_portal = BTreeSet::new();
    let mut crystal_portal = BTreeSet::new();
    let mut crossing_edges = 0_usize;
    for lower in graph
        .positions()
        .filter(|position| position.level < UPPER_REGION_THRESHOLD)
    {
        for upper in graph
            .neighbors(lower)
            .iter()
            .copied()
            .filter(|position| position.level >= UPPER_REGION_THRESHOLD)
        {
            crossing_edges = crossing_edges.saturating_add(1);
            let belongs_to_natural =
                natural.surfaces.contains(&lower) || natural.surfaces.contains(&upper);
            let belongs_to_crystal =
                crystal.surfaces.contains(&lower) || crystal.surfaces.contains(&upper);
            match (belongs_to_natural, belongs_to_crystal) {
                (true, false) => {
                    natural_portal.insert(canonical_graph_edge(lower, upper));
                }
                (false, true) => {
                    crystal_portal.insert(canonical_graph_edge(lower, upper));
                }
                (false, false) => {
                    return Err(schematic_contract(format!(
                        "unclaimed upper-route graph edge {lower:?} -> {upper:?}"
                    )));
                }
                (true, true) => {
                    return Err(schematic_contract(format!(
                        "natural and Crystal portal cuts overlap at {lower:?} -> {upper:?}"
                    )));
                }
            }
        }
    }
    if crossing_edges == 0 || natural_portal.is_empty() || crystal_portal.is_empty() {
        return Err(schematic_contract(format!(
            "upper-route graph requires two nonempty portal cuts (edges={crossing_edges}, natural={}, Crystal={})",
            natural_portal.len(),
            crystal_portal.len()
        )));
    }

    let baseline = graph.distances_from(foothill);
    if !baseline.contains_key(&summit) {
        return Err(schematic_contract(
            "upper-route graph cannot reach the summit before portal removal",
        ));
    }
    let both = natural_portal
        .union(&crystal_portal)
        .copied()
        .collect::<BTreeSet<_>>();
    // Contract the graph once with both authored portal cuts removed. Testing
    // either remaining portal then becomes a tiny component-graph query. This
    // is exactly equivalent to three additional whole-world reachability walks:
    // ordinary edges define the components, and only the retained portal edges
    // can join them.
    let components = graph_components_avoiding_edges(graph, &both);
    let foothill_component = components
        .get(&foothill)
        .copied()
        .ok_or_else(|| schematic_contract("upper-route component audit lost the foothill node"))?;
    let summit_component = components
        .get(&summit)
        .copied()
        .ok_or_else(|| schematic_contract("upper-route component audit lost the summit node"))?;
    if foothill_component == summit_component {
        return Err(schematic_contract(
            "a third ordinary foothill-to-upper route survives both declared portal cuts",
        ));
    }
    if !portal_components_connect(
        &components,
        &crystal_portal,
        foothill_component,
        summit_component,
    ) {
        return Err(schematic_contract(
            "Crystal route does not independently connect foothill to summit",
        ));
    }
    if !portal_components_connect(
        &components,
        &natural_portal,
        foothill_component,
        summit_component,
    ) {
        return Err(schematic_contract(
            "natural pass does not independently connect foothill to summit",
        ));
    }
    Ok(baseline)
}

fn canonical_graph_edge(first: TilePos, second: TilePos) -> (TilePos, TilePos) {
    if first <= second {
        (first, second)
    } else {
        (second, first)
    }
}

fn graph_components_avoiding_edges(
    graph: &OrdinaryGraph,
    blocked: &BTreeSet<(TilePos, TilePos)>,
) -> BTreeMap<TilePos, u32> {
    let mut components = BTreeMap::new();
    let mut next_component = 0_u32;
    for start in graph.positions() {
        if components.contains_key(&start) {
            continue;
        }
        components.insert(start, next_component);
        let mut frontier = VecDeque::from([start]);
        while let Some(position) = frontier.pop_front() {
            for neighbor in graph.neighbors(position) {
                if !blocked.contains(&canonical_graph_edge(position, *neighbor))
                    && !components.contains_key(neighbor)
                {
                    components.insert(*neighbor, next_component);
                    frontier.push_back(*neighbor);
                }
            }
        }
        next_component = next_component.saturating_add(1);
    }
    components
}

fn portal_components_connect(
    components: &BTreeMap<TilePos, u32>,
    portals: &BTreeSet<(TilePos, TilePos)>,
    start: u32,
    target: u32,
) -> bool {
    let mut neighbors = BTreeMap::<u32, BTreeSet<u32>>::new();
    for (first, second) in portals {
        let (Some(first), Some(second)) = (components.get(first), components.get(second)) else {
            continue;
        };
        if first == second {
            continue;
        }
        neighbors.entry(*first).or_default().insert(*second);
        neighbors.entry(*second).or_default().insert(*first);
    }
    let mut reached = BTreeSet::from([start]);
    let mut frontier = VecDeque::from([start]);
    while let Some(component) = frontier.pop_front() {
        if component == target {
            return true;
        }
        for neighbor in neighbors.get(&component).into_iter().flatten() {
            if reached.insert(*neighbor) {
                frontier.push_back(*neighbor);
            }
        }
    }
    false
}

fn top_solid_material(column: &VolumeColumn) -> SolidMaterialRole {
    column
        .elements
        .iter()
        .rev()
        .find_map(|element| match element {
            VolumeElement::Solid(mass) => Some(mass.material),
            VolumeElement::Fill(_) => None,
        })
        .unwrap_or(SolidMaterialRole::Gravel)
}

fn path_direction(path: &[HexCoord], index: usize) -> usize {
    let current = path[index];
    let target = path
        .get(index.saturating_add(1))
        .copied()
        .or_else(|| {
            index
                .checked_sub(1)
                .and_then(|previous| path.get(previous).copied())
        })
        .unwrap_or(current);
    current
        .neighbors()
        .iter()
        .position(|neighbor| *neighbor == target)
        .unwrap_or(0)
}

fn step_in_direction(mut coord: HexCoord, direction: usize, offset: i32) -> HexCoord {
    let actual_direction = if offset < 0 {
        (direction + 3) % 6
    } else {
        direction
    };
    for _ in 0..offset.unsigned_abs() {
        coord = coord.neighbors()[actual_direction];
    }
    coord
}

fn schematic_contract(detail: impl Into<String>) -> V3GenerationError {
    V3GenerationError::RecipeContract(detail.into())
}

fn review_anchors(
    plan: &SchematicPlanV1,
    ordinary: &BTreeSet<TilePos>,
) -> Result<BTreeMap<String, TilePos>, V3GenerationError> {
    let fallback = ordinary.iter().next().copied().ok_or_else(|| {
        V3GenerationError::InvalidFallback(vec![WorldValidationIssue::new(
            WorldIssueCode::Anchor,
            "schematic world contains no ordinary surfaces",
        )])
    })?;
    let network_node = |kind: NetworkKind, id: &str| {
        plan.networks
            .iter()
            .find(|network| network.kind == kind)
            .and_then(|network| network.nodes.iter().find(|node| node.id.as_str() == id))
            .map(|node| schematic_to_world(node.coord, 22))
    };
    let hydrology_sink = network_node(NetworkKind::Hydrology, "node/hydrology-sea-mouth")
        .ok_or_else(|| {
            V3GenerationError::InvalidFallback(vec![WorldValidationIssue::new(
                WorldIssueCode::Anchor,
                "schematic world is missing the exact hydrology sea-mouth node",
            )])
        })?;
    let nearest = |target: HexCoord| {
        ordinary
            .iter()
            .copied()
            .min_by_key(|position| {
                (
                    position.coord.distance(target),
                    Reverse(position.level),
                    *position,
                )
            })
            .unwrap_or(fallback)
    };
    let party = nearest(hydrology_sink);
    let hostile = ordinary
        .iter()
        .copied()
        .max_by_key(|position| (position.coord.distance(party.coord), *position))
        .unwrap_or(party);
    let anchors = BTreeMap::from([
        ("party_start".to_owned(), party),
        ("hostile_start".to_owned(), hostile),
        ("conflict_center".to_owned(), nearest(HexCoord::ORIGIN)),
    ]);
    Ok(anchors)
}

/// Rebinds only presentation-review anchors that a final vegetation blocker
/// separated from the foothill component.
///
/// These names are observational camera/spawn conveniences rather than exact
/// authored geometry. Route terminals, bridge voxels, the peak saddle, and
/// Crystal anchors are deliberately absent: disconnecting one of those remains
/// a generation failure. The first selection still reserves a three-hex local
/// clearing before vegetation; this final blocker-aware pass handles the rarer
/// case where a tree severs that clearing's more distant egress. Replacements
/// come only from the already protected ordinary-hub route, so rebinding cannot
/// manufacture new walker authority or require decoration to be regenerated.
fn reconcile_final_review_anchor_reachability(
    anchors: &mut BTreeMap<String, TilePos>,
    reachable: &BTreeMap<TilePos, u32>,
    durable_review_surfaces: &BTreeSet<TilePos>,
) -> Result<(), V3GenerationError> {
    const RELOCATABLE_REVIEW_ANCHORS: [&str; 13] = [
        "party_start",
        "hostile_start",
        "conflict_center",
        "grand_v3.waterfall_crown",
        "grand_v3.waterfall_base",
        "grand_v3.waterfall_profile",
        "grand_v3.coast",
        "grand_v3.archipelago",
        "grand_v3.valley_lake",
        "grand_v3.mountain_lake",
        "grand_v3.frozen_woods",
        "grand_v3.massif",
        "grand_v3.river_bend",
    ];

    if reachable.is_empty() || durable_review_surfaces.is_empty() {
        return Err(schematic_contract(
            "review-anchor reconciliation has no durable foothill-reachable surfaces",
        ));
    }
    for name in RELOCATABLE_REVIEW_ANCHORS {
        let Some(current) = anchors.get(name).copied() else {
            continue;
        };
        if reachable.contains_key(&current) {
            continue;
        }
        let replacement = durable_review_surfaces
            .iter()
            .copied()
            .filter(|surface| reachable.contains_key(surface))
            .min_by_key(|surface| {
                (
                    surface.coord.distance(current.coord),
                    Reverse(surface.level),
                    *surface,
                )
            })
            .ok_or_else(|| {
                schematic_contract(format!(
                    "review anchor {name:?} has no foothill-reachable replacement"
                ))
            })?;
        anchors.insert(name.to_owned(), replacement);
    }
    Ok(())
}

fn add_final_review_anchors(
    anchors: &mut BTreeMap<String, TilePos>,
    ordinary: &BTreeSet<TilePos>,
    hydrology: &HydrologyCompilation,
    structures: &StructurePlan,
    plan: &SchematicPlanV1,
    centers: &BTreeMap<PatchId, HexCoord>,
) {
    let Some(fallback) = ordinary.first().copied() else {
        return;
    };
    let nearest = |target: HexCoord| {
        ordinary
            .iter()
            .copied()
            .min_by_key(|surface| {
                (
                    surface.coord.distance(target),
                    Reverse(surface.level),
                    *surface,
                )
            })
            .unwrap_or(fallback)
    };
    if let Some(crown) = hydrology.waterfall_centerline.first() {
        anchors.insert("grand_v3.waterfall_crown".to_owned(), nearest(crown.coord));
    }
    if let Some(base) = hydrology.waterfall_centerline.last() {
        anchors.insert("grand_v3.waterfall_base".to_owned(), nearest(base.coord));
    }
    if let Some((profile, _)) = hydrology
        .waterfall_centerline
        .windows(2)
        .find_map(|pair| (pair[0].level > pair[1].level).then_some((pair[0], pair[1])))
    {
        anchors.insert(
            "grand_v3.waterfall_profile".to_owned(),
            nearest(profile.coord),
        );
    }
    if let Some(outlet) = hydrology.river_centerline.last() {
        anchors.insert("grand_v3.coast".to_owned(), nearest(outlet.coord));
    }
    if let Ok(coarse_river) =
        schematic_network_path(plan, NetworkKind::Hydrology, "edge/hydrology-valley-to-sea")
    {
        let direct = fine_network_path(&coarse_river, 22)
            .into_iter()
            .collect::<BTreeSet<_>>();
        if let Some(bend) = hydrology
            .river_centerline
            .iter()
            .copied()
            .max_by_key(|position| {
                (
                    direct
                        .iter()
                        .map(|coord| position.coord.distance(*coord))
                        .min()
                        .unwrap_or_default(),
                    Reverse(position.coord),
                )
            })
        {
            anchors.insert("grand_v3.river_bend".to_owned(), nearest(bend.coord));
        }
    }
    for (name, ordinal) in [
        ("grand_v3.valley_bridge", 0_u32),
        ("grand_v3.coastal_bridge", 1_u32),
    ] {
        if let Some(bridge) = structures
            .by_id
            .get(&StructureId(BRIDGE_STRUCTURE_BASE.saturating_add(ordinal)))
            .and_then(|structure| structure.voxels.iter().next().copied())
        {
            anchors.entry(name.to_owned()).or_insert(bridge);
        }
    }
    for (name, feature) in [
        ("grand_v3.archipelago", SchematicFeature::SeaIsland),
        ("grand_v3.valley_lake", SchematicFeature::ValleyLake),
        ("grand_v3.mountain_lake", SchematicFeature::MountainLake),
        ("grand_v3.frozen_woods", SchematicFeature::FrozenWoods),
        ("grand_v3.peak_saddle", SchematicFeature::PeakRing),
    ] {
        if let Some(target) = plan
            .cells
            .iter()
            .find(|cell| has_overlay(cell, feature))
            .and_then(|cell| centers.get(&PatchId(u32::from(cell.id.get()))))
            .copied()
        {
            anchors.entry(name.to_owned()).or_insert(nearest(target));
        }
    }
    if let Some(summit) = anchors.get("crystal_ascent.upper_exit").copied() {
        anchors.insert("grand_v3.crystal_summit".to_owned(), summit);
    }
    if let Some(target) = plan
        .cells
        .iter()
        .find(|cell| cell.facts.landform == LandformKind::Massif)
        .and_then(|cell| centers.get(&PatchId(u32::from(cell.id.get()))))
        .copied()
    {
        anchors
            .entry("grand_v3.massif".to_owned())
            .or_insert(nearest(target));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TreelineReviewWitnesses {
    downhill_tree: TilePos,
    uphill_snow: TilePos,
}

/// Exact semantic review selections sealed before incidental access demotion.
///
/// Selection consumes whole-world indexes, but every selected anchor becomes
/// authored authority before the only subsequent mutation. That mutation can
/// demote unrelated unreachable Ordinary surfaces; geometry, features,
/// blockers, and the measured reachable component remain unchanged. Final
/// validation therefore compares the publication to this sealed evidence and
/// rechecks only the local facts which demotion could invalidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CorrectiveReviewAuthority {
    mantle: TilePos,
    treeline: TilePos,
    treeline_witnesses: TreelineReviewWitnesses,
    peak: TilePos,
}

#[derive(Debug)]
struct TreelineReviewIndex {
    tree_roots: BTreeMap<HexCoord, TilePos>,
    snowy_highlands: BTreeMap<HexCoord, TilePos>,
    tree_free_exclusion: BTreeSet<HexCoord>,
    anchor_blocker_exclusion: BTreeSet<HexCoord>,
}

impl TreelineReviewIndex {
    fn build(
        plan: &SchematicPlanV1,
        fine_index: &FineWorldIndex,
        world: &GeneratedWorldPlan,
    ) -> Self {
        let cells = plan
            .cells
            .iter()
            .map(|cell| (PatchId(u32::from(cell.id.get())), cell))
            .collect::<BTreeMap<_, _>>();
        let owner_cell = |coord: HexCoord| {
            fine_index
                .patch(coord)
                .and_then(|patch| cells.get(&patch).copied())
        };
        let all_tree_roots = world
            .features
            .by_id
            .values()
            .filter(|feature| feature.kind == FeatureKind::Tree)
            .map(|feature| (feature.root.coord, feature.root))
            .collect::<BTreeMap<_, _>>();
        let tree_roots = world
            .features
            .by_id
            .values()
            .filter(|feature| feature.kind == FeatureKind::Tree)
            .filter(|feature| {
                owner_cell(feature.root.coord).is_some_and(treeline_downhill_tree_cell)
            })
            .fold(
                BTreeMap::<HexCoord, TilePos>::new(),
                |mut roots, feature| {
                    roots
                        .entry(feature.root.coord)
                        .and_modify(|current| *current = (*current).max(feature.root))
                        .or_insert(feature.root);
                    roots
                },
            );
        let snowy_highlands = world
            .layout
            .footprint
            .iter()
            .copied()
            .filter_map(|coord| {
                let (surface, _) = world.volume.top_surface_at_coord(coord)?;
                (owner_cell(coord).is_some_and(treeline_uphill_cell)
                    && solid_material_at(&world.volume, surface) == Some(SolidMaterialRole::Snow))
                .then_some((coord, surface))
            })
            .collect::<BTreeMap<_, _>>();
        let tree_free_exclusion = all_tree_roots
            .keys()
            .flat_map(|coord| coord.within_radius(TREELINE_REVIEW_TREE_FREE_RADIUS))
            .collect::<BTreeSet<_>>();
        let anchor_blocker_exclusion = world
            .blockers
            .iter()
            .flat_map(|blocker| {
                blocker
                    .coord
                    .within_radius(REVIEW_ANCHOR_BLOCKER_CLEAR_RADIUS)
            })
            .collect::<BTreeSet<_>>();
        Self {
            tree_roots,
            snowy_highlands,
            tree_free_exclusion,
            anchor_blocker_exclusion,
        }
    }

    fn witnesses(&self, anchor: TilePos) -> Option<TreelineReviewWitnesses> {
        let mut downhill_by_direction = BTreeMap::<usize, TilePos>::new();
        let mut uphill_by_direction = BTreeMap::<usize, TilePos>::new();
        for coord in anchor
            .coord
            .within_radius(TREELINE_REVIEW_TREE_SEARCH_RADIUS)
        {
            let distance = anchor.coord.distance(coord);
            if let Some(tree) = self
                .tree_roots
                .get(&coord)
                .copied()
                .filter(|tree| tree.level < anchor.level)
            {
                let direction = primary_review_direction(anchor.coord, coord)?;
                let replace = downhill_by_direction.get(&direction).is_none_or(|current| {
                    (distance, Reverse(tree.level), tree)
                        < (
                            anchor.coord.distance(current.coord),
                            Reverse(current.level),
                            *current,
                        )
                });
                if replace {
                    downhill_by_direction.insert(direction, tree);
                }
            }
            if distance <= TREELINE_REVIEW_UPHILL_SEARCH_RADIUS
                && !self.tree_free_exclusion.contains(&coord)
            {
                if let Some(snow) = self
                    .snowy_highlands
                    .get(&coord)
                    .copied()
                    .filter(|snow| snow.level > anchor.level)
                {
                    let direction = primary_review_direction(anchor.coord, coord)?;
                    let replace = uphill_by_direction.get(&direction).is_none_or(|current| {
                        (distance, snow.level, snow)
                            < (
                                anchor.coord.distance(current.coord),
                                current.level,
                                *current,
                            )
                    });
                    if replace {
                        uphill_by_direction.insert(direction, snow);
                    }
                }
            }
        }

        downhill_by_direction
            .values()
            .copied()
            .flat_map(|tree| {
                uphill_by_direction
                    .values()
                    .copied()
                    .filter(move |snow| {
                        review_vectors_face_opposite(anchor.coord, tree.coord, snow.coord)
                    })
                    .map(move |snow| (tree, snow))
            })
            .min_by_key(|(tree, snow)| {
                let tree_distance = anchor.coord.distance(tree.coord);
                let snow_distance = anchor.coord.distance(snow.coord);
                (
                    tree_distance.saturating_add(snow_distance),
                    tree_distance.max(snow_distance),
                    Reverse(snow.level.saturating_sub(tree.level)),
                    *tree,
                    *snow,
                )
            })
            .map(|(downhill_tree, uphill_snow)| TreelineReviewWitnesses {
                downhill_tree,
                uphill_snow,
            })
    }
}

fn treeline_transition_cell(cell: &CellPlan) -> bool {
    cell.facts.surface == SurfaceKind::Land
        && cell.facts.landform == LandformKind::Mountain
        && cell.facts.climate == ClimateKind::Alpine
        && !has_overlay(cell, SchematicFeature::CrystalAscent)
        && !has_overlay(cell, SchematicFeature::FrozenWoods)
        && !has_overlay(cell, SchematicFeature::LakeIsland)
}

fn treeline_downhill_tree_cell(cell: &CellPlan) -> bool {
    cell.facts.surface == SurfaceKind::Land
        && cell.facts.climate != ClimateKind::Frozen
        && matches!(
            cell.facts.landform,
            LandformKind::Valley
                | LandformKind::Plateau
                | LandformKind::Hill
                | LandformKind::Mountain
        )
        && !has_overlay(cell, SchematicFeature::CrystalAscent)
        && !has_overlay(cell, SchematicFeature::FrozenWoods)
        && !has_overlay(cell, SchematicFeature::LakeIsland)
}

fn treeline_uphill_cell(cell: &CellPlan) -> bool {
    cell.facts.surface == SurfaceKind::Land
        && cell.facts.climate == ClimateKind::Alpine
        && matches!(
            cell.facts.landform,
            LandformKind::Mountain | LandformKind::Massif | LandformKind::SharpPeak
        )
        && !has_overlay(cell, SchematicFeature::CrystalAscent)
        && !has_overlay(cell, SchematicFeature::FrozenWoods)
        && !has_overlay(cell, SchematicFeature::LakeIsland)
}

fn primary_review_direction(origin: HexCoord, target: HexCoord) -> Option<usize> {
    (origin != target).then(|| {
        origin
            .neighbors()
            .into_iter()
            .enumerate()
            .min_by_key(|(index, neighbor)| (neighbor.distance(target), *index))
            .map(|(index, _)| index)
            .unwrap_or_default()
    })
}

fn review_vectors_face_opposite(origin: HexCoord, first: HexCoord, second: HexCoord) -> bool {
    let origin = origin.to_cubic_array().map(i128::from);
    let first = first.to_cubic_array().map(i128::from);
    let second = second.to_cubic_array().map(i128::from);
    first
        .into_iter()
        .zip(origin)
        .map(|(component, origin)| component.saturating_sub(origin))
        .zip(
            second
                .into_iter()
                .zip(origin)
                .map(|(component, origin)| component.saturating_sub(origin)),
        )
        .map(|(first, second)| first.saturating_mul(second))
        .sum::<i128>()
        < 0
}

fn mantle_screen_blocks_review_line(
    world: &GeneratedWorldPlan,
    screen: &BTreeSet<HexCoord>,
    anchor: HexCoord,
    crystal_center: HexCoord,
) -> bool {
    anchor
        .line_between(crystal_center)
        .into_iter()
        .filter(|coord| *coord != anchor && *coord != crystal_center)
        .filter(|coord| screen.contains(coord))
        .any(|coord| {
            world
                .volume
                .top_surface_at_coord(coord)
                .is_some_and(|(surface, _)| {
                    surface.level > super::schematic_highlands::CRYSTAL_ARCHITECTURE_TOP
                })
        })
}

fn resolve_crystal_mantle_overlook(
    plan: &SchematicPlanV1,
    world: &GeneratedWorldPlan,
    profile: V3GrandV3BasicTerrainProfile,
    crystal_mask: &BTreeSet<HexCoord>,
    crystal_rotation: u8,
    reachable: &BTreeMap<TilePos, u32>,
) -> Result<TilePos, V3GenerationError> {
    let crystal_cell = plan
        .cells
        .iter()
        .find(|cell| has_overlay(cell, SchematicFeature::CrystalAscent))
        .ok_or_else(|| schematic_contract("Crystal mantle review has no schematic centre"))?;
    let crystal_center = schematic_to_world(crystal_cell.coord, 22);
    let lower_entry = world
        .anchors
        .get("crystal_ascent.lower_entry")
        .copied()
        .ok_or_else(|| schematic_contract("Crystal mantle review has no exact lower entry"))?;
    let screen = super::schematic_highlands::crystal_mantle_inner_screen(
        crystal_mask,
        crystal_rotation,
        profile,
        &world.layout.footprint,
    )?;
    let blocker_exclusion = world
        .blockers
        .iter()
        .flat_map(|blocker| {
            blocker
                .coord
                .within_radius(REVIEW_ANCHOR_BLOCKER_CLEAR_RADIUS)
        })
        .collect::<BTreeSet<_>>();
    let tree_exclusion = world
        .features
        .by_id
        .values()
        .filter(|feature| feature.kind == FeatureKind::Tree)
        .flat_map(|feature| {
            feature
                .root
                .coord
                .within_radius(REVIEW_ANCHOR_TREE_ROOT_CLEAR_RADIUS)
        })
        .collect::<BTreeSet<_>>();
    reachable
        .keys()
        .copied()
        .filter(|surface| {
            let center_distance = surface.coord.distance(crystal_center);
            surface.level < UPPER_REGION_THRESHOLD
                && center_distance > CRYSTAL_MANTLE_REVIEW_MINIMUM_DISTANCE
                && center_distance > crystal_center.distance(lower_entry.coord)
                && surface.coord.distance(lower_entry.coord) < center_distance
                && !blocker_exclusion.contains(&surface.coord)
                && !tree_exclusion.contains(&surface.coord)
                && world
                    .volume
                    .surfaces
                    .get(surface)
                    .is_some_and(|metadata| metadata.access == SurfaceAccess::Ordinary)
                && mantle_screen_blocks_review_line(
                    world,
                    &screen,
                    surface.coord,
                    crystal_center,
                )
        })
        .min_by_key(|surface| {
            (
                surface.coord.distance(lower_entry.coord),
                surface
                    .coord
                    .distance(crystal_center)
                    .abs_diff(CRYSTAL_MANTLE_REVIEW_PREFERRED_DISTANCE),
                Reverse(surface.level),
                *surface,
            )
        })
        .ok_or_else(|| {
            schematic_contract(
                "Crystal mantle review has no reachable valley-side surface behind the exact screen",
            )
        })
}

fn resolve_treeline_transition(
    plan: &SchematicPlanV1,
    fine_index: &FineWorldIndex,
    world: &GeneratedWorldPlan,
    reachable: &BTreeMap<TilePos, u32>,
) -> Result<(TilePos, TreelineReviewWitnesses), V3GenerationError> {
    let cells = plan
        .cells
        .iter()
        .map(|cell| (PatchId(u32::from(cell.id.get())), cell))
        .collect::<BTreeMap<_, _>>();
    let index = TreelineReviewIndex::build(plan, fine_index, world);
    let mut candidates = reachable
        .keys()
        .copied()
        .filter(|surface| {
            fine_index
                .patch(surface.coord)
                .and_then(|patch| cells.get(&patch).copied())
                .is_some_and(treeline_transition_cell)
                && !index.anchor_blocker_exclusion.contains(&surface.coord)
                && !index.tree_free_exclusion.contains(&surface.coord)
                && world
                    .volume
                    .top_surface_at_coord(surface.coord)
                    .is_some_and(|(top, metadata)| {
                        top == *surface && metadata.access == SurfaceAccess::Ordinary
                    })
                && solid_material_at(&world.volume, *surface) == Some(SolidMaterialRole::Snow)
        })
        .collect::<Vec<_>>();
    candidates.sort_unstable_by_key(|surface| {
        (
            surface.level.abs_diff(TREELINE_REVIEW_TARGET_LEVEL),
            *surface,
        )
    });

    let mut best = None;
    let mut selected_level_delta = None;
    for candidate in candidates {
        let level_delta = candidate.level.abs_diff(TREELINE_REVIEW_TARGET_LEVEL);
        if selected_level_delta.is_some_and(|selected| level_delta > selected) {
            break;
        }
        let Some(witnesses) = index.witnesses(candidate) else {
            continue;
        };
        let tree_distance = candidate.coord.distance(witnesses.downhill_tree.coord);
        let snow_distance = candidate.coord.distance(witnesses.uphill_snow.coord);
        let key = (
            tree_distance.saturating_add(snow_distance),
            tree_distance.max(snow_distance),
            Reverse(
                witnesses
                    .uphill_snow
                    .level
                    .saturating_sub(witnesses.downhill_tree.level),
            ),
            candidate,
            witnesses.downhill_tree,
            witnesses.uphill_snow,
        );
        if best.as_ref().is_none_or(|(current, _, _)| key < *current) {
            best = Some((key, candidate, witnesses));
            selected_level_delta = Some(level_delta);
        }
    }
    best.map(|(_, anchor, witnesses)| (anchor, witnesses))
        .ok_or_else(|| {
            schematic_contract(
                "treeline review has no reachable snowy mountain surface between an actual downhill tree and higher treeless snow",
            )
        })
}

fn resolve_corrective_review_anchors(
    plan: &SchematicPlanV1,
    fine_index: &FineWorldIndex,
    profile: V3GrandV3BasicTerrainProfile,
    crystal_mask: &BTreeSet<HexCoord>,
    crystal_rotation: u8,
    reachable: &BTreeMap<TilePos, u32>,
    world: &mut GeneratedWorldPlan,
) -> Result<CorrectiveReviewAuthority, V3GenerationError> {
    let mantle = resolve_crystal_mantle_overlook(
        plan,
        world,
        profile,
        crystal_mask,
        crystal_rotation,
        reachable,
    )?;
    let (treeline, treeline_witnesses) =
        resolve_treeline_transition(plan, fine_index, world, reachable)?;
    let peak = world
        .anchors
        .get("grand_v3.peak_foothill_ledge")
        .copied()
        .filter(|surface| reachable.contains_key(surface))
        .ok_or_else(|| {
            schematic_contract("peak-ridge review lost its authored reachable peak-foothill ledge")
        })?;
    world
        .anchors
        .insert("grand_v3.crystal_mantle_overlook".to_owned(), mantle);
    world
        .anchors
        .insert("grand_v3.treeline_transition".to_owned(), treeline);
    world
        .anchors
        .insert("grand_v3.peak_ridge_overlook".to_owned(), peak);
    Ok(CorrectiveReviewAuthority {
        mantle,
        treeline,
        treeline_witnesses,
        peak,
    })
}

/// Adds exact observation-only landmarks after walker reconciliation.
///
/// These two anchors intentionally may be scenic or inaccessible: Map review
/// needs the authored Garden centre and the unique massif world-high point,
/// while gameplay reachability must not flatten either feature merely to make
/// it a spawn location.
fn add_exact_corrective_observation_anchors(
    world: &mut GeneratedWorldPlan,
    plan: &SchematicPlanV1,
    centers: &BTreeMap<PatchId, HexCoord>,
    massif_crest: TilePos,
) -> Result<(), V3GenerationError> {
    let lake_island = plan
        .cells
        .iter()
        .find(|cell| has_overlay(cell, SchematicFeature::LakeIsland))
        .and_then(|cell| centers.get(&PatchId(u32::from(cell.id.get()))))
        .and_then(|center| world.volume.top_surface_at_coord(*center))
        .map(|(surface, _)| surface)
        .ok_or_else(|| {
            schematic_contract("corrective review has no exact Garden-island surface")
        })?;

    if world
        .volume
        .top_surface_at_coord(massif_crest.coord)
        .map(|(surface, _)| surface)
        != Some(massif_crest)
        || world
            .volume
            .surfaces
            .get(&massif_crest)
            .is_none_or(|metadata| {
                metadata.access != SurfaceAccess::SpecialMovement(INACCESSIBLE_MOVEMENT_REGION)
            })
        || massif_crest.level
            <= world
                .anchors
                .get("crystal_ascent.upper_exit")
                .map_or(Level::MIN, |surface| surface.level)
    {
        return Err(schematic_contract(
            "exact massif review crest was moved, made ordinary, or does not stand above Crystal Ascent",
        ));
    }
    world
        .observation_anchors
        .insert("grand_v3.lake_island".to_owned(), lake_island);
    world
        .observation_anchors
        .insert("grand_v3.massif_crest".to_owned(), massif_crest);
    Ok(())
}

fn schematic_view_hint(
    footprint: &BTreeSet<HexCoord>,
    level_height: f32,
    maximum_surface: Level,
) -> MapViewHint {
    let mut bounds = None::<(f32, f32, f32, f32)>;
    for coord in footprint {
        let center = coord.to_world(0.0);
        bounds = Some(bounds.map_or(
            (center.x, center.x, center.z, center.z),
            |(minimum_x, maximum_x, minimum_z, maximum_z)| {
                (
                    minimum_x.min(center.x),
                    maximum_x.max(center.x),
                    minimum_z.min(center.z),
                    maximum_z.max(center.z),
                )
            },
        ));
    }
    let Some((minimum_x, maximum_x, minimum_z, maximum_z)) = bounds else {
        return MapViewHint::new((0.0, 80.0, 80.0), (0.0, 0.0, 0.0));
    };

    let focus_x = (minimum_x + maximum_x) * 0.5;
    let focus_z = (minimum_z + maximum_z) * 0.5;
    let half_width = (maximum_x - minimum_x) * 0.5 + 2.0;
    let half_depth = (maximum_z - minimum_z) * 0.5 + 2.0;
    let vertical_span =
        f32::from(i16::try_from(maximum_surface).unwrap_or(i16::MAX)) * level_height;
    // Derive the pose from the complete footprint. The conservative 40-degree
    // vertical cone fits both the 16:9 review target and the ordinary Map lens,
    // so converting this same distance to a top-down pose cannot crop the world.
    let required_vertical_half_extent = half_depth.max(half_width / (16.0 / 9.0));
    let distance = ((required_vertical_half_extent + vertical_span * 0.3 + 12.0)
        / 20.0_f32.to_radians().tan())
        * 1.1;
    let direction_length = (0.45_f32 * 0.45 + 0.82 * 0.82 + 0.35 * 0.35).sqrt();
    let direction = (
        -0.45 / direction_length,
        0.82 / direction_length,
        0.35 / direction_length,
    );
    let focus_y = vertical_span * 0.35;
    MapViewHint::new(
        (
            focus_x + direction.0 * distance,
            focus_y + direction.1 * distance,
            focus_z + direction.2 * distance,
        ),
        (focus_x, focus_y, focus_z),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use hex_schematic::LayerProvenance;

    use super::*;
    use crate::settings::V3GrandV3BasicTerrainProfile;
    use crate::voxel::TERRAIN_CHUNK_SIDE;

    struct ReferenceFixture {
        plan: SchematicPlanV1,
        selection: ValidatedWorldSelection<SchematicWorldMetrics>,
    }

    fn settings() -> ProceduralV3Settings {
        ProceduralV3Settings {
            layout: V3LayoutSettings::Schematic(V3SchematicLayoutSettings {
                template: V3SchematicTemplate::GrandV3,
                template_revision: 2,
                cell_pitch: 22,
                terrain_profile: V3SchematicTerrainProfile::GrandV3BasicV1(
                    V3GrandV3BasicTerrainProfile::canonical(),
                ),
            }),
        }
    }

    fn reference_fixture() -> &'static ReferenceFixture {
        static FIXTURE: OnceLock<ReferenceFixture> = OnceLock::new();
        FIXTURE.get_or_init(|| {
            let template = hex_schematic::grand_v3_reference_template().expect("template parses");
            let reference =
                hex_schematic::reference_plan(&template, 0).expect("reference is valid");
            let selection = compile_schematic(
                &reference.plan,
                &settings(),
                V3_SCHEMATIC_GRID_RADIUS,
                0.4,
                super::super::vegetation::tests::runtime_art_catalog(),
            )
            .expect("reference compiles");
            ReferenceFixture {
                plan: reference.plan,
                selection,
            }
        })
    }

    fn reference_peak_ridge_authority(
    ) -> &'static super::super::schematic_highlands::PeakRidgeAuthority {
        static AUTHORITY: OnceLock<super::super::schematic_highlands::PeakRidgeAuthority> =
            OnceLock::new();
        AUTHORITY.get_or_init(|| {
            let fixture = reference_fixture();
            let mut authority = unsealed_reference_peak_ridge_authority();
            seal_peak_ridge_route_grades(
                &mut authority,
                &fixture.selection.validated.plan.volume,
                &fixture.selection.validated.plan.features,
            )
            .expect("reference final world reproduces its exact authored peak grades");
            authority
        })
    }

    fn unsealed_reference_peak_ridge_authority(
    ) -> super::super::schematic_highlands::PeakRidgeAuthority {
        let fixture = reference_fixture();
        let mut layout = resolve_layout(V3_SCHEMATIC_GRID_RADIUS, &settings())
            .expect("reference peak-authority layout resolves");
        super::super::schematic_crystal::claim_site(&fixture.plan, &mut layout, 22)
            .expect("reference peak-authority Crystal claim validates");
        super::super::schematic_highlands::GrandHighlandField::build(
            &fixture.plan,
            &layout,
            V3GrandV3BasicTerrainProfile::canonical(),
        )
        .expect("reference peak-authority field builds")
        .peak_ridge_authority()
        .clone()
    }

    fn replace_test_surface_level(world: &mut GeneratedWorldPlan, coord: HexCoord, level: Level) {
        let (surface, metadata) = world
            .volume
            .top_surface_at_coord(coord)
            .expect("peak mutation coordinate has one top surface");
        let biome = world
            .biome_regions
            .get(&surface)
            .copied()
            .expect("peak mutation coordinate retains its biome");
        let material = world
            .volume
            .columns
            .get(&coord)
            .map(top_solid_material)
            .expect("peak mutation coordinate retains its column");
        replace_column_surface(
            &mut world.volume,
            &mut world.biome_regions,
            coord,
            land_column(level, material),
            TilePos::new(coord, level),
            metadata,
            biome,
        );
    }

    fn surface_level_at(world: &GeneratedWorldPlan, coord: HexCoord) -> Level {
        let mut matches = world.volume.surfaces_at_coord(coord);
        let level = matches
            .next()
            .unwrap_or_else(|| panic!("{coord:?} has no projected surface"))
            .0
            .level;
        assert!(
            matches.next().is_none(),
            "{coord:?} has more than one projected surface"
        );
        level
    }

    fn world_coord(q: i32, r: i32) -> HexCoord {
        HexCoord::from_axial(q.saturating_mul(22), r.saturating_mul(22))
    }

    #[test]
    fn treeline_review_witnesses_must_face_opposite_sides_of_the_anchor() {
        let anchor = TilePos::new(HexCoord::ORIGIN, TREELINE_REVIEW_TARGET_LEVEL);
        let downhill_tree = TilePos::new(HexCoord::from_axial(-3, 0), 124);
        let uphill_snow = TilePos::new(HexCoord::from_axial(4, 0), 148);
        let index = TreelineReviewIndex {
            tree_roots: BTreeMap::from([(downhill_tree.coord, downhill_tree)]),
            snowy_highlands: BTreeMap::from([(uphill_snow.coord, uphill_snow)]),
            tree_free_exclusion: BTreeSet::new(),
            anchor_blocker_exclusion: BTreeSet::new(),
        };

        assert_eq!(
            index.witnesses(anchor),
            Some(TreelineReviewWitnesses {
                downhill_tree,
                uphill_snow,
            })
        );
        assert!(review_vectors_face_opposite(
            anchor.coord,
            downhill_tree.coord,
            uphill_snow.coord,
        ));

        let same_side_snow = TilePos::new(HexCoord::from_axial(-6, 0), 148);
        let same_side_index = TreelineReviewIndex {
            tree_roots: BTreeMap::from([(downhill_tree.coord, downhill_tree)]),
            snowy_highlands: BTreeMap::from([(same_side_snow.coord, same_side_snow)]),
            tree_free_exclusion: BTreeSet::new(),
            anchor_blocker_exclusion: BTreeSet::new(),
        };
        assert_eq!(same_side_index.witnesses(anchor), None);
    }

    #[test]
    fn waterfall_transition_requires_the_exact_continuous_intermediate_row() {
        let first_source = TilePos::new(HexCoord::from_axial(110, -24), 20);
        let second_source = TilePos::new(HexCoord::from_axial(111, -24), 20);
        let first_intermediate = TilePos::new(HexCoord::from_axial(109, -23), 18);
        let second_intermediate = TilePos::new(HexCoord::from_axial(111, -23), 18);
        let collapsed_shortcut = TilePos::new(HexCoord::from_axial(110, -23), 15);
        let ranks = BTreeMap::from([
            (first_source.coord, 64),
            (second_source.coord, 64),
            (first_intermediate.coord, 65),
            (second_intermediate.coord, 65),
            (collapsed_shortcut.coord, 66),
        ]);
        let source_fill = NonSolidFill {
            levels: LevelInterval::new(19, 21),
            material: FillMaterialRole::Water,
        };
        let intermediate_fill = NonSolidFill {
            levels: LevelInterval::new(16, 19),
            material: FillMaterialRole::Water,
        };
        let fill_runs = BTreeMap::from([
            (first_source, source_fill),
            (second_source, source_fill),
            (first_intermediate, intermediate_fill),
            (second_intermediate, intermediate_fill),
            (collapsed_shortcut, intermediate_fill),
        ]);

        assert_eq!(
            canonical_hydrology_successor_state(
                first_source,
                first_intermediate,
                &ranks,
                &fill_runs,
            ),
            Some(LiquidFlowState::Fall)
        );
        assert_eq!(
            canonical_hydrology_successor_state(
                second_source,
                second_intermediate,
                &ranks,
                &fill_runs,
            ),
            Some(LiquidFlowState::Fall)
        );
        assert_eq!(
            canonical_hydrology_successor_state(
                first_source,
                collapsed_shortcut,
                &ranks,
                &fill_runs,
            ),
            None,
            "the first reported five-level edge skips row 65"
        );
        assert_eq!(
            canonical_hydrology_successor_state(
                second_source,
                collapsed_shortcut,
                &ranks,
                &fill_runs,
            ),
            None,
            "the second reported five-level edge skips row 65"
        );

        let mut adjacent_shortcut_ranks = ranks;
        adjacent_shortcut_ranks.insert(collapsed_shortcut.coord, 65);
        assert_eq!(
            canonical_hydrology_successor_state(
                first_source,
                collapsed_shortcut,
                &adjacent_shortcut_ranks,
                &fill_runs,
            ),
            None,
            "even an adjacent row cannot start a fall below its source fill"
        );
    }

    #[test]
    fn waterfall_profile_keeps_high_spill_one_true_plunge_and_low_basin() {
        let levels = plunge_levels(150, 15, 67, 44).expect("fixture has both approaches");
        let centerline = levels
            .iter()
            .copied()
            .enumerate()
            .map(|(index, level)| {
                TilePos::new(
                    HexCoord::from_axial(i32::try_from(index).unwrap_or(i32::MAX), 0),
                    level,
                )
            })
            .collect::<Vec<_>>();
        validate_plunge_profile(&centerline, 150, 15).expect("authored plunge is valid");
        assert!(levels[..=44].iter().all(|level| *level == 150));
        assert!(levels[45..].iter().all(|level| *level == 15));
        assert_eq!(
            levels.windows(2).filter(|pair| pair[0] > pair[1]).count(),
            1,
            "the full mountain-to-valley drop must render as one curtain"
        );

        let ramp = descending_levels(150, 15, 67)
            .into_iter()
            .enumerate()
            .map(|(index, level)| {
                TilePos::new(
                    HexCoord::from_axial(i32::try_from(index).unwrap_or(i32::MAX), 0),
                    level,
                )
            })
            .collect::<Vec<_>>();
        assert!(
            validate_plunge_profile(&ramp, 150, 15).is_err(),
            "a uniformly descending stair-ramp must not satisfy the waterfall contract"
        );
    }

    #[test]
    fn named_river_meander_is_simple_bounded_three_wide_and_visibly_bent() {
        let coarse = [
            (5, -1, -4),
            (5, 0, -5),
            (4, 1, -5),
            (3, 2, -5),
            (2, 3, -5),
            (1, 3, -4),
            (0, 4, -4),
            (-1, 5, -4),
        ]
        .into_iter()
        .map(|(q, r, s)| SchematicCoord::new(q, r, s).expect("fixture cube coordinate is valid"))
        .collect::<Vec<_>>();
        let direct = fine_network_path(&coarse, 22);
        let semantic_sea = direct
            .iter()
            .rev()
            .take(12)
            .copied()
            .collect::<BTreeSet<_>>();
        let footprint = HexCoord::ORIGIN
            .within_radius(187)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let seed = 1_592_598_566;
        let meander = meandering_fine_network_path(&coarse, 22, seed, &footprint, &semantic_sea)
            .expect("named stream resolves a valid meander");
        assert_eq!(meander.first(), direct.first());
        assert_eq!(meander.last(), direct.last());
        assert_eq!(
            meander.iter().copied().collect::<BTreeSet<_>>().len(),
            meander.len(),
            "the authoritative river must remain simple"
        );
        assert!(meander
            .windows(2)
            .all(|pair| pair[0].distance(pair[1]) == 1));
        assert_ne!(meander, direct, "the river may not remain a ruler line");
        assert!(
            longest_straight_run(&meander) < 44,
            "no straight run may span two complete coarse pitches"
        );
        let direct_set = direct.iter().copied().collect::<BTreeSet<_>>();
        let maximum_excursion = meander
            .iter()
            .map(|coord| {
                direct_set
                    .iter()
                    .map(|direct_coord| coord.distance(*direct_coord))
                    .min()
                    .unwrap_or_default()
            })
            .max()
            .unwrap_or_default();
        assert!(maximum_excursion >= 3, "the bend must read at map scale");

        let level_path = meander
            .iter()
            .copied()
            .map(|coord| TilePos::new(coord, 15))
            .collect::<Vec<_>>();
        let rows = build_three_lane_rows(&level_path, &footprint, "meander test")
            .expect("meander supports exact transverse rows");
        assert!(rows.iter().all(|row| row.len() == 3));
        assert_eq!(
            meandering_fine_network_path(&coarse, 22, seed, &footprint, &semantic_sea,)
                .expect("same named stream resolves again"),
            meander,
            "the same seed must be byte-stable"
        );
        let alternate =
            meandering_fine_network_path(&coarse, 22, seed + 1, &footprint, &semantic_sea)
                .expect("neighboring seed also resolves inside the same corridor");
        assert_ne!(
            alternate, meander,
            "the named hydrology stream must make fine bends seed-variable"
        );
        assert_eq!(alternate.first(), direct.first());
        assert_eq!(alternate.last(), direct.last());
    }

    #[test]
    fn every_non_reversing_three_lane_bend_reaches_the_sink_without_skipping_a_row() {
        let footprint = HexCoord::ORIGIN
            .within_radius(8)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let mut lateral_edges = 0_usize;
        for incoming in 0..6 {
            for turn in [0, 1, 2, 4, 5] {
                let outgoing = (incoming + turn) % 6;
                let mut centerline = (1_i32..=3)
                    .rev()
                    .map(|offset| {
                        TilePos::new(
                            step_in_direction(HexCoord::ORIGIN, (incoming + 3) % 6, offset),
                            8,
                        )
                    })
                    .collect::<Vec<_>>();
                centerline.push(TilePos::new(HexCoord::ORIGIN, 8));
                centerline.extend((1_i32..=3).map(|offset| {
                    TilePos::new(step_in_direction(HexCoord::ORIGIN, outgoing, offset), 8)
                }));
                let rows = build_three_lane_rows(&centerline, &footprint, "bend fixture")
                    .expect("every non-reversing bend resolves");
                assert_eq!(rows.len(), centerline.len());
                assert!(rows.iter().all(|row| row.len() == 3));

                let positions = rows
                    .iter()
                    .flatten()
                    .map(|position| (position.coord, *position))
                    .collect::<BTreeMap<_, _>>();
                let ranks = rows
                    .iter()
                    .enumerate()
                    .flat_map(|(rank, row)| row.iter().map(move |position| (position.coord, rank)))
                    .collect::<BTreeMap<_, _>>();
                let fill = NonSolidFill {
                    levels: LevelInterval::new(7, 9),
                    material: FillMaterialRole::Water,
                };
                let fill_runs = positions
                    .values()
                    .copied()
                    .map(|position| (position, fill))
                    .collect::<BTreeMap<_, _>>();
                let mut nodes = positions
                    .values()
                    .copied()
                    .map(|position| {
                        (
                            position,
                            LiquidNode {
                                state: LiquidFlowState::Still,
                                downstream: None,
                            },
                        )
                    })
                    .collect::<BTreeMap<_, _>>();
                let authority = OutletAuthority {
                    edges: BTreeMap::new(),
                    downstream_course: BTreeSet::new(),
                };
                apply_directed_watercourse(&rows, &authority, &fill_runs, &mut nodes)
                    .expect("every exact lane reaches the terminal row");

                let final_row = rows.last().expect("fixture has a terminal row");
                for (source, node) in &nodes {
                    if final_row.contains(source) {
                        assert_eq!(node.state, LiquidFlowState::Still);
                        assert!(node.downstream.is_none());
                        continue;
                    }
                    let downstream = node.downstream.expect("nonterminal lane keeps flowing");
                    let source_rank = ranks[&source.coord];
                    let target_rank = ranks[&downstream.coord];
                    assert!(target_rank == source_rank || target_rank == source_rank + 1);
                    if target_rank == source_rank {
                        lateral_edges = lateral_edges.saturating_add(1);
                        assert_eq!(source.level, downstream.level);
                        assert_eq!(node.state, LiquidFlowState::Current);
                    }
                }
                for start in positions.values().copied() {
                    let mut cursor = start;
                    let mut seen = BTreeSet::new();
                    while !final_row.contains(&cursor) {
                        assert!(seen.insert(cursor), "bend flow must remain acyclic");
                        cursor = nodes[&cursor]
                            .downstream
                            .expect("every lane terminates at the exact final row");
                    }
                }
            }
        }
        assert!(
            lateral_edges > 0,
            "the corpus must exercise the concave-corner lateral successor"
        );
    }

    #[test]
    fn upper_bench_grading_assigns_one_level_to_a_revisited_coordinate() {
        let staged_walk = (0..=24)
            .chain([23, 24])
            .chain(25..=40)
            .map(|q| HexCoord::from_axial(q, 0))
            .collect::<Vec<_>>();
        assert!(staged_walk
            .windows(2)
            .all(|pair| pair[0].distance(pair[1]) == 1));
        assert!(staged_walk.iter().filter(|coord| coord.x() == 23).count() > 1);
        let corridor = staged_walk.iter().copied().collect::<BTreeSet<_>>();
        let branch = TilePos::new(HexCoord::from_axial(0, 0), 150);
        let rejoin = TilePos::new(HexCoord::from_axial(40, 0), 150);

        let levels = graded_upper_bench_levels(&corridor, branch, rejoin, 166)
            .expect("the unique graph, not occurrence order, determines one exact field");

        assert_eq!(levels.len(), corridor.len());
        assert_eq!(levels[&branch.coord], branch.level);
        assert_eq!(levels[&rejoin.coord], rejoin.level);
        assert_eq!(levels.values().copied().max(), Some(166));
        assert!(levels
            .values()
            .all(|level| *level >= UPPER_REGION_THRESHOLD.saturating_add(1)));
        assert!(corridor.iter().all(|coord| {
            coord.neighbors().into_iter().all(|neighbor| {
                !corridor.contains(&neighbor) || levels[coord].abs_diff(levels[&neighbor]) <= 1
            })
        }));
        assert_eq!(
            staged_walk
                .iter()
                .filter(|coord| **coord == HexCoord::from_axial(23, 0))
                .map(|coord| levels[coord])
                .collect::<BTreeSet<_>>()
                .len(),
            1,
            "every occurrence of a backtracked coordinate resolves to one level"
        );
    }

    #[test]
    fn upper_bench_grading_honors_water_bank_minimums_without_moving_exact_pins() {
        let corridor = (0..=40)
            .map(|q| HexCoord::from_axial(q, 0))
            .collect::<BTreeSet<_>>();
        let branch = TilePos::new(HexCoord::from_axial(0, 0), 150);
        let rejoin = TilePos::new(HexCoord::from_axial(40, 0), 150);
        let bank = HexCoord::from_axial(30, 0);

        let levels = graded_upper_bench_levels_with_minimums(
            &corridor,
            branch,
            rejoin,
            166,
            &BTreeMap::from([(bank, 160)]),
        )
        .expect("the bank cone and exact bench/junction pins share one feasible grade");

        assert_eq!(levels[&branch.coord], branch.level);
        assert_eq!(levels[&rejoin.coord], rejoin.level);
        assert_eq!(levels[&bank], 160);
        assert_eq!(levels.values().copied().max(), Some(166));
        assert!(corridor.iter().all(|coord| {
            coord.neighbors().into_iter().all(|neighbor| {
                !corridor.contains(&neighbor) || levels[coord].abs_diff(levels[&neighbor]) <= 1
            })
        }));
    }

    #[test]
    fn minimax_pass_prefers_the_lower_route_and_breaks_ties_canonically() {
        let start = HexCoord::from_axial(-2, 0);
        let target = HexCoord::from_axial(2, -1);
        let upper = [
            start,
            HexCoord::from_axial(-1, -1),
            HexCoord::from_axial(0, -1),
            HexCoord::from_axial(1, -1),
            target,
        ];
        let lower = [
            start,
            HexCoord::from_axial(-1, 0),
            HexCoord::from_axial(0, 0),
            HexCoord::from_axial(1, 0),
            target,
        ];
        let mut levels = upper
            .into_iter()
            .chain(lower)
            .map(|coord| (coord, 4))
            .collect::<BTreeMap<_, _>>();
        levels.insert(HexCoord::from_axial(0, 0), 9);
        let selected = minimax_surface_path(start, target, &levels).expect("path exists");
        assert_eq!(selected, upper);

        levels.insert(HexCoord::from_axial(0, 0), 4);
        let first = minimax_surface_path(start, target, &levels).expect("tie has a path");
        let second = minimax_surface_path(start, target, &levels).expect("tie is replayable");
        assert_eq!(first, second);
        assert_eq!(first.len(), 5);
        assert_eq!(
            first[1],
            upper[1].min(lower[1]),
            "equal minimax and length routes use canonical coordinate tie-breaking"
        );
    }

    #[test]
    fn local_distance_repair_matches_full_bfs_for_deletions_and_additions() {
        let root_coord = HexCoord::from_axial(0, 0);
        let edited_coord = HexCoord::from_axial(1, 0);
        let goal_coord = HexCoord::from_axial(2, 0);
        let detour_first = HexCoord::from_axial(0, 1);
        let detour_second = HexCoord::from_axial(1, 1);
        let coords = BTreeSet::from([
            root_coord,
            edited_coord,
            goal_coord,
            detour_first,
            detour_second,
        ]);
        let mut volume = VolumePlan::new(coords.clone());
        for coord in coords {
            volume
                .columns
                .insert(coord, land_column(4, SolidMaterialRole::Stone));
            volume.surfaces.insert(
                TilePos::new(coord, 4),
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
            );
        }
        let root = TilePos::new(root_coord, 4);
        let goal = TilePos::new(goal_coord, 4);
        let old_edited = TilePos::new(edited_coord, 4);
        let mut graph = OrdinaryGraph::from_volume(&volume, None);
        let mut distances = ordinary_band_distances(&graph, root, OrdinaryRegionBand::Lower);
        assert_eq!(distances.get(&goal), Some(&2));

        volume
            .columns
            .insert(edited_coord, land_column(6, SolidMaterialRole::Stone));
        assert!(volume.surfaces.remove(&old_edited).is_some());
        let raised = TilePos::new(edited_coord, 6);
        volume.surfaces.insert(
            raised,
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: None,
            },
        );
        let affected = graph.refresh_coords(&volume, None, [edited_coord]);
        repair_ordinary_band_distances(
            &graph,
            &mut distances,
            root,
            OrdinaryRegionBand::Lower,
            &affected,
        )
        .expect("a local deletion retains the longer detour");
        let rebuilt = OrdinaryGraph::from_volume(&volume, None);
        assert_eq!(
            distances,
            ordinary_band_distances(&rebuilt, root, OrdinaryRegionBand::Lower)
        );
        assert_eq!(distances.get(&goal), Some(&3));
        assert!(!distances.contains_key(&raised));

        volume
            .columns
            .insert(edited_coord, land_column(4, SolidMaterialRole::Stone));
        assert!(volume.surfaces.remove(&raised).is_some());
        volume.surfaces.insert(
            old_edited,
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: None,
            },
        );
        let affected = graph.refresh_coords(&volume, None, [edited_coord]);
        repair_ordinary_band_distances(
            &graph,
            &mut distances,
            root,
            OrdinaryRegionBand::Lower,
            &affected,
        )
        .expect("a local addition propagates the shorter path");
        let rebuilt = OrdinaryGraph::from_volume(&volume, None);
        assert_eq!(
            distances,
            ordinary_band_distances(&rebuilt, root, OrdinaryRegionBand::Lower)
        );
        assert_eq!(distances.get(&goal), Some(&2));
        assert_eq!(distances.get(&old_edited), Some(&1));
    }

    #[test]
    fn final_ordinary_admission_rejects_unreachable_authored_walker_intent() {
        let root = TilePos::new(HexCoord::from_axial(0, 0), 4);
        let connected = TilePos::new(HexCoord::from_axial(1, 0), 4);
        let unreachable_authored = TilePos::new(HexCoord::from_axial(4, 0), 4);
        let scenic = TilePos::new(HexCoord::from_axial(5, 0), 4);
        let blocked_ordinary = TilePos::new(HexCoord::from_axial(6, 0), 4);
        let positions = [
            root,
            connected,
            unreachable_authored,
            scenic,
            blocked_ordinary,
        ];
        let mut volume = VolumePlan::new(positions.iter().map(|position| position.coord).collect());
        for position in positions {
            volume.columns.insert(
                position.coord,
                land_column(position.level, SolidMaterialRole::Stone),
            );
            volume.surfaces.insert(
                position,
                SurfaceMetadata {
                    access: if position == scenic {
                        SurfaceAccess::SpecialMovement(SCENIC_MOVEMENT_REGION)
                    } else {
                        SurfaceAccess::Ordinary
                    },
                    interior: None,
                },
            );
        }
        let blockers = BTreeSet::from([blocked_ordinary]);
        let graph = OrdinaryGraph::from_volume(&volume, Some(&blockers));
        let reachable = graph.distances_from(root);

        let error =
            validate_complete_final_ordinary_reachability(&volume, &blockers, &graph, &reachable)
                .expect_err("disconnected authored Ordinary intent must fail final admission");
        let detail = match error {
            V3GenerationError::RecipeContract(detail) => detail,
            unexpected => panic!("ordinary reachability returned the wrong error: {unexpected:?}"),
        };
        assert!(detail.contains("not reachable from the foothill"));
        assert!(detail.contains(&format!("{unreachable_authored:?}")));

        let mut graph = graph;
        assert_eq!(
            reconcile_final_incidental_ordinary_access(
                &mut volume,
                &blockers,
                &mut graph,
                &reachable,
                &BTreeSet::from([unreachable_authored]),
            )
            .expect("authored authority is valid"),
            0,
            "final reconciliation must preserve unreachable authored walker intent"
        );
        assert_eq!(
            volume.surfaces[&unreachable_authored].access,
            SurfaceAccess::Ordinary
        );
        assert_eq!(
            reconcile_final_incidental_ordinary_access(
                &mut volume,
                &blockers,
                &mut graph,
                &reachable,
                &BTreeSet::new(),
            )
            .expect("incidental reconciliation is valid"),
            1,
        );
        assert_eq!(
            volume.surfaces[&unreachable_authored].access,
            SurfaceAccess::SpecialMovement(INACCESSIBLE_MOVEMENT_REGION)
        );
        let reachable = graph.distances_from(root);
        validate_complete_final_ordinary_reachability(&volume, &blockers, &graph, &reachable)
            .expect(
                "scenic, inaccessible, and exactly blocked surfaces are outside Ordinary admission",
            );
    }

    #[test]
    fn final_review_anchor_rebind_moves_only_observational_anchors() {
        let unreachable = TilePos::new(HexCoord::from_axial(8, 0), 20);
        let nearest = TilePos::new(HexCoord::from_axial(6, 0), 20);
        let farther = TilePos::new(HexCoord::from_axial(3, 0), 21);
        let exact_crystal = TilePos::new(HexCoord::from_axial(9, 0), 150);
        let mut anchors = BTreeMap::from([
            ("grand_v3.valley_lake".to_owned(), unreachable),
            ("grand_v3.crystal_summit".to_owned(), exact_crystal),
            ("party_start".to_owned(), farther),
        ]);
        let reachable = BTreeMap::from([(nearest, 4), (farther, 7)]);
        let durable_review_surfaces = BTreeSet::from([nearest, farther]);

        reconcile_final_review_anchor_reachability(
            &mut anchors,
            &reachable,
            &durable_review_surfaces,
        )
        .expect("a disconnected observational anchor should rebind deterministically");

        assert_eq!(anchors["grand_v3.valley_lake"], nearest);
        assert_eq!(anchors["party_start"], farther);
        assert_eq!(
            anchors["grand_v3.crystal_summit"], exact_crystal,
            "exact Crystal route anchors must remain immovable and fail later validation"
        );
    }

    #[test]
    fn upper_route_cut_graph_accepts_two_independent_portals_and_rejects_a_third() {
        let ring = [
            HexCoord::from_axial(2, 0),
            HexCoord::from_axial(1, 1),
            HexCoord::from_axial(0, 2),
            HexCoord::from_axial(-1, 2),
            HexCoord::from_axial(-2, 2),
            HexCoord::from_axial(-2, 1),
            HexCoord::from_axial(-2, 0),
            HexCoord::from_axial(-1, -1),
            HexCoord::from_axial(0, -2),
            HexCoord::from_axial(1, -2),
            HexCoord::from_axial(2, -2),
            HexCoord::from_axial(2, -1),
        ];
        assert!(ring
            .iter()
            .copied()
            .zip(ring.iter().copied().cycle().skip(1))
            .take(ring.len())
            .all(|(first, second)| first.distance(second) == 1));

        let build = |third_crossing: bool| {
            let mut volume = VolumePlan::new(ring.iter().copied().collect());
            let positions = ring
                .iter()
                .copied()
                .enumerate()
                .map(|(index, coord)| {
                    let upper = index >= 6 || (third_crossing && index == 3);
                    TilePos::new(
                        coord,
                        if upper {
                            UPPER_REGION_THRESHOLD
                        } else {
                            UPPER_REGION_THRESHOLD.saturating_sub(1)
                        },
                    )
                })
                .collect::<Vec<_>>();
            for position in &positions {
                volume.columns.insert(
                    position.coord,
                    land_column(position.level, SolidMaterialRole::Stone),
                );
                volume.surfaces.insert(
                    *position,
                    SurfaceMetadata {
                        access: SurfaceAccess::Ordinary,
                        interior: None,
                    },
                );
            }
            (volume, positions)
        };

        let (volume, positions) = build(false);
        let natural = ProtectedFeatureRoute {
            centerline: vec![positions[5], positions[6]],
            surfaces: BTreeSet::from([positions[5], positions[6]]),
        };
        let crystal = ProtectedFeatureRoute {
            centerline: vec![positions[11], positions[0]],
            surfaces: BTreeSet::from([positions[11], positions[0]]),
        };
        let graph = OrdinaryGraph::from_volume(&volume, None);
        let distances =
            validate_upper_route_cut_graph(&graph, positions[2], positions[8], &natural, &crystal)
                .expect(
                    "either declared portal connects, while removing both disconnects the ring",
                );
        assert!(distances.contains_key(&positions[8]));

        let (volume, positions) = build(true);
        let graph = OrdinaryGraph::from_volume(&volume, None);
        let error =
            validate_upper_route_cut_graph(&graph, positions[2], positions[8], &natural, &crystal)
                .expect_err("an unclaimed third lower-to-upper contact must fail closed");
        assert!(matches!(
            error,
            V3GenerationError::RecipeContract(detail)
                if detail.contains("unclaimed upper-route graph edge")
        ));
    }

    #[test]
    fn final_ordinary_admission_rejects_a_stale_incremental_projection() {
        let root = TilePos::new(HexCoord::from_axial(0, 0), 4);
        let changed = TilePos::new(HexCoord::from_axial(1, 0), 4);
        let mut volume = VolumePlan::new(BTreeSet::from([root.coord, changed.coord]));
        for position in [root, changed] {
            volume.columns.insert(
                position.coord,
                land_column(position.level, SolidMaterialRole::Stone),
            );
            volume.surfaces.insert(
                position,
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
            );
        }

        let stale_with_extra = OrdinaryGraph::from_volume(&volume, None);
        volume
            .surfaces
            .get_mut(&changed)
            .expect("fixture retains the changed surface")
            .access = SurfaceAccess::SpecialMovement(INACCESSIBLE_MOVEMENT_REGION);
        let reachable = stale_with_extra.distances_from(root);
        let error = validate_complete_final_ordinary_reachability(
            &volume,
            &BTreeSet::new(),
            &stale_with_extra,
            &reachable,
        )
        .expect_err("a stale graph node must fail final admission");
        assert!(matches!(
            error,
            V3GenerationError::RecipeContract(detail)
                if detail.contains("retains stale non-walker surface")
        ));

        let stale_with_omission = OrdinaryGraph::from_volume(&volume, None);
        volume
            .surfaces
            .get_mut(&changed)
            .expect("fixture retains the changed surface")
            .access = SurfaceAccess::Ordinary;
        let reachable = stale_with_omission.distances_from(root);
        let error = validate_complete_final_ordinary_reachability(
            &volume,
            &BTreeSet::new(),
            &stale_with_omission,
            &reachable,
        )
        .expect_err("a newly intended node omitted by a stale graph must fail final admission");
        assert!(matches!(
            error,
            V3GenerationError::RecipeContract(detail)
                if detail.contains("omits authoritative walker surface")
        ));
    }

    #[test]
    fn upper_connector_never_crosses_an_immutable_lower_bank() {
        let start = HexCoord::from_axial(0, 0);
        let bank = HexCoord::from_axial(1, 0);
        let approach = HexCoord::from_axial(2, 0);
        let target = TilePos::new(HexCoord::from_axial(3, 0), 150);
        let footprint = BTreeSet::from([start, bank, approach]);
        let ordinary_mask = footprint.clone();
        let hard_forbidden = BTreeSet::from([bank]);
        let preserved = BTreeSet::from([bank]);
        let network = BTreeMap::from([(target.coord, vec![target])]);
        let mut surfaces = BTreeMap::from([
            (start, TilePos::new(start, 170)),
            (bank, TilePos::new(bank, 16)),
            (approach, TilePos::new(approach, 170)),
        ]);

        assert!(ordinary_connector_to_network(
            start,
            OrdinaryRegionBand::Upper,
            &footprint,
            &ordinary_mask,
            &hard_forbidden,
            &preserved,
            &network,
            &surfaces,
        )
        .is_empty());

        surfaces.insert(bank, TilePos::new(bank, 151));
        assert!(!ordinary_connector_to_network(
            start,
            OrdinaryRegionBand::Upper,
            &footprint,
            &ordinary_mask,
            &hard_forbidden,
            &preserved,
            &network,
            &surfaces,
        )
        .is_empty());

        surfaces.insert(bank, TilePos::new(bank, 16));
        assert!(!ordinary_connector_to_network(
            start,
            OrdinaryRegionBand::Upper,
            &footprint,
            &ordinary_mask,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &network,
            &surfaces,
        )
        .is_empty());
    }

    #[test]
    fn required_upper_connector_can_exit_through_an_exact_protected_ribbon() {
        let start = HexCoord::from_axial(0, 0);
        let ribbon = HexCoord::from_axial(1, 0);
        let approach = HexCoord::from_axial(2, 0);
        let target = TilePos::new(HexCoord::from_axial(3, 0), 150);
        let footprint = BTreeSet::from([start, ribbon, approach]);
        let ordinary_mask = footprint.clone();
        let hard_forbidden = BTreeSet::from([start, ribbon]);
        let network = BTreeMap::from([(target.coord, vec![target])]);
        let surfaces = BTreeMap::from([
            (start, TilePos::new(start, 150)),
            (ribbon, TilePos::new(ribbon, 151)),
            (approach, TilePos::new(approach, 170)),
        ]);

        assert!(ordinary_connector_to_network(
            start,
            OrdinaryRegionBand::Upper,
            &footprint,
            &ordinary_mask,
            &hard_forbidden,
            &BTreeSet::new(),
            &network,
            &surfaces,
        )
        .is_empty());

        let exact_protected = BTreeSet::from([start, ribbon]);
        let first = ordinary_connector_to_network(
            start,
            OrdinaryRegionBand::Upper,
            &footprint,
            &ordinary_mask,
            &hard_forbidden,
            &exact_protected,
            &network,
            &surfaces,
        );
        let second = ordinary_connector_to_network(
            start,
            OrdinaryRegionBand::Upper,
            &footprint,
            &ordinary_mask,
            &hard_forbidden,
            &exact_protected,
            &network,
            &surfaces,
        );
        assert_eq!(first, second, "exact-ribbon egress must be deterministic");
        assert!(first.iter().any(|(path, endpoint)| {
            path == &[start, ribbon, approach, target.coord] && *endpoint == target
        }));
        let levels = ordinary_connector_levels_with_preserved_banks(
            &[start, ribbon, approach, target.coord],
            150,
            target,
            3,
            OrdinaryRegionBand::Upper,
            &exact_protected,
            &surfaces,
        )
        .expect("exact protected levels admit one Upper-only splice");
        assert_eq!(levels.first().copied(), Some(150));
        assert_eq!(levels.get(1).copied(), Some(151));
        assert_eq!(levels.last().copied(), Some(target.level));
        assert!(levels
            .iter()
            .skip(2)
            .take(levels.len().saturating_sub(3))
            .all(|level| OrdinaryRegionBand::Upper.accepts_new(*level)));
    }

    #[test]
    fn required_connector_search_keeps_a_longer_height_feasible_parent() {
        let start = TilePos::new(HexCoord::from_axial(0, 0), 150);
        let incompatible = HexCoord::from_axial(1, 0);
        let approach = HexCoord::from_axial(2, 0);
        let detour_first = HexCoord::from_axial(0, 1);
        let detour_second = HexCoord::from_axial(1, 1);
        let target = TilePos::new(HexCoord::from_axial(3, 0), 150);
        let footprint = BTreeSet::from([
            start.coord,
            incompatible,
            approach,
            detour_first,
            detour_second,
            target.coord,
        ]);
        let ordinary_mask = footprint.clone();
        let hard_forbidden = BTreeSet::from([incompatible, detour_second]);
        let preserved = BTreeSet::from([incompatible, detour_second]);
        let network = BTreeMap::from([(target.coord, vec![target])]);
        let surfaces = BTreeMap::from([
            (start.coord, start),
            (incompatible, TilePos::new(incompatible, 170)),
            (approach, TilePos::new(approach, 150)),
            (detour_first, TilePos::new(detour_first, 150)),
            (detour_second, TilePos::new(detour_second, 150)),
        ]);
        let mut volume = VolumePlan::new(footprint.clone());
        volume.surfaces.insert(
            target,
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: None,
            },
        );

        let geometric = ordinary_connector_to_network(
            start.coord,
            OrdinaryRegionBand::Upper,
            &footprint,
            &ordinary_mask,
            &hard_forbidden,
            &preserved,
            &network,
            &surfaces,
        );
        assert_eq!(geometric.len(), 1);
        assert!(geometric[0].0.contains(&incompatible));
        assert!(ordinary_connector_levels_with_preserved_banks(
            &geometric[0].0,
            start.level,
            target,
            3,
            OrdinaryRegionBand::Upper,
            &preserved,
            &surfaces,
        )
        .is_none());

        let domain = RequiredConnectorDomain::new(
            OrdinaryRegionBand::Upper,
            &footprint,
            &ordinary_mask,
            &hard_forbidden,
            &preserved,
            &network,
            &surfaces,
        )
        .expect("the bounded fixture should admit a dense connector domain");
        let reverse_costs = required_connector_reverse_costs(
            &domain,
            OrdinaryRegionBand::Upper,
            &preserved,
            &network,
        );
        assert_eq!(domain.metric(&reverse_costs, approach), Some(0));
        assert_eq!(domain.metric(&reverse_costs, incompatible), Some(1));
        assert_eq!(domain.metric(&reverse_costs, detour_second), Some(1));
        assert_eq!(
            domain.metric(&reverse_costs, detour_first),
            Some(ORDINARY_CONNECTOR_PRESERVED_COST + 2)
        );
        assert_eq!(
            domain.metric(&reverse_costs, start.coord),
            Some(ORDINARY_CONNECTOR_PRESERVED_COST + 2)
        );

        let (path, selected_target, levels) = solve_required_ordinary_connector(
            start,
            OrdinaryRegionBand::Upper,
            &volume,
            &footprint,
            &ordinary_mask,
            &hard_forbidden,
            &preserved,
            &network,
            &surfaces,
            &BTreeSet::new(),
            None,
        )
        .expect("height-aware search retains the longer feasible parent");
        assert_eq!(selected_target, target);
        assert_eq!(path.first().copied(), Some(start.coord));
        assert_eq!(path.last().copied(), Some(target.coord));
        assert!(!path.contains(&incompatible));
        assert_eq!(path.len(), levels.len());
        assert_eq!(levels.first().copied(), Some(start.level));
        assert_eq!(levels.last().copied(), Some(target.level));
        assert!(levels.windows(2).all(|pair| pair[0].abs_diff(pair[1]) <= 1));
    }

    #[test]
    fn bridge_candidate_rejects_a_coplanar_upstream_bank_shoulder() {
        let water_coord = HexCoord::from_axial(0, 0);
        let bank_coord = HexCoord::from_axial(-1, 0);
        let over_water = TilePos::new(water_coord, 11);
        let coplanar_bank = TilePos::new(bank_coord, 10);
        let water_deck = BTreeSet::from([over_water]);
        let water_levels = BTreeMap::from([(water_coord, 10)]);

        assert!(!bridge_bank_shoulders_clear_adjacent_water(
            &BTreeSet::from([over_water, coplanar_bank]),
            &water_deck,
            &water_levels,
        ));
        assert!(bridge_bank_shoulders_clear_adjacent_water(
            &BTreeSet::from([over_water, TilePos::new(bank_coord, 11)]),
            &water_deck,
            &water_levels,
        ));
    }

    fn exact_bridge_approach_fixture(
        external_level: Level,
    ) -> (BridgeCompilation, VolumePlan, BTreeSet<HexCoord>) {
        let first = TilePos::new(HexCoord::ORIGIN, 15);
        let [second_coord, ..] = first.coord.neighbors();
        let second = TilePos::new(second_coord, 15);
        let axis = 2_usize;
        let row = |center: TilePos| {
            (-1_i32..=1)
                .map(|offset| {
                    TilePos::new(step_in_direction(center.coord, axis, offset), center.level)
                })
                .collect::<BTreeSet<_>>()
        };
        let (deck, water_deck) = exact_bridge_deck(first, &row(first), second, &row(second))
            .expect("fixture resolves an exact bridge deck");
        let external = [first, second]
            .into_iter()
            .flat_map(|center| {
                [-3_i32, 3].map(|offset| {
                    TilePos::new(
                        step_in_direction(center.coord, axis, offset),
                        external_level,
                    )
                })
            })
            .collect::<BTreeSet<_>>();
        let external_coords = external
            .iter()
            .map(|surface| surface.coord)
            .collect::<BTreeSet<_>>();
        let surfaces = deck.union(&external).copied().collect::<BTreeSet<_>>();
        let mut volume = VolumePlan::new(surfaces.iter().map(|surface| surface.coord).collect());
        for surface in surfaces {
            volume.columns.insert(
                surface.coord,
                land_column(surface.level, SolidMaterialRole::WorkedStone),
            );
            volume.surfaces.insert(
                surface,
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
            );
        }
        let authority = BridgeAuthority {
            structure: StructureId(BRIDGE_STRUCTURE_BASE),
            river_row_indices: [0, 1],
            deck,
            water_deck,
        };
        (
            BridgeCompilation {
                structures: StructurePlan::default(),
                crossings: vec![authority],
            },
            volume,
            external_coords,
        )
    }

    #[test]
    fn exact_bridge_requires_two_dry_walker_contacts_on_each_bank() {
        let (bridges, mut volume, _) = exact_bridge_approach_fixture(15);
        let ordinary_mask = volume.mask.clone();
        let empty = BTreeSet::new();
        let surface_by_coord = top_standable_surfaces_by_coord(&volume);
        let approaches = bridge_bank_approaches(
            &bridges,
            &volume,
            None,
            &ordinary_mask,
            &empty,
            &empty,
            &surface_by_coord,
        )
        .expect("both two-wide banks have independent dry contacts");
        assert_eq!(approaches.len(), 4);
        assert_eq!(
            approaches
                .iter()
                .map(|approach| approach.bank_index)
                .collect::<Vec<_>>(),
            vec![0, 0, 1, 1]
        );

        for approach in approaches
            .iter()
            .filter(|approach| approach.bank_index == 0)
        {
            volume.columns.remove(&approach.surface.coord);
            volume.surfaces.remove(&approach.surface);
        }
        let surface_by_coord = top_standable_surfaces_by_coord(&volume);
        let error = bridge_bank_approaches(
            &bridges,
            &volume,
            None,
            &ordinary_mask,
            &empty,
            &empty,
            &surface_by_coord,
        )
        .expect_err("one-sided bridge landing must fail closed");
        let V3GenerationError::RecipeContract(detail) = error else {
            panic!("bridge landing returned the wrong error: {error:?}");
        };
        assert!(detail.contains("no two independent dry walker approaches"));
    }

    #[test]
    fn bridge_mismatched_height_contacts_resolve_to_the_exact_deck_level() {
        let (bridges, volume, external_coords) = exact_bridge_approach_fixture(8);
        let ordinary_mask = volume.mask.clone();
        let empty = BTreeSet::new();
        let surface_by_coord = top_standable_surfaces_by_coord(&volume);

        let approaches = bridge_bank_approaches(
            &bridges,
            &volume,
            None,
            &ordinary_mask,
            &empty,
            &empty,
            &surface_by_coord,
        )
        .expect("mutable low contacts should receive an exact grading plan");

        assert_eq!(approaches.len(), 4);
        assert_eq!(
            approaches
                .iter()
                .map(|approach| approach.surface.coord)
                .collect::<BTreeSet<_>>(),
            external_coords
        );
        assert!(approaches
            .iter()
            .all(|approach| approach.surface.level == 16));
        assert!(external_coords.iter().all(|coord| {
            surface_by_coord
                .get(coord)
                .is_some_and(|surface| surface.level == 8)
        }));
    }

    #[test]
    fn sibling_bridge_connectors_preserve_every_pregraded_approach() {
        let first = TilePos::new(HexCoord::from_axial(0, 0), 15);
        let sibling = TilePos::new(HexCoord::from_axial(1, 0), 15);
        let mutable = TilePos::new(HexCoord::from_axial(2, 0), 12);
        let target = TilePos::new(HexCoord::from_axial(3, 0), 15);
        let approaches = vec![
            BridgeBankApproach {
                structure: StructureId(BRIDGE_STRUCTURE_BASE),
                bank_index: 0,
                lane_index: 0,
                surface: first,
            },
            BridgeBankApproach {
                structure: StructureId(BRIDGE_STRUCTURE_BASE),
                bank_index: 0,
                lane_index: 1,
                surface: sibling,
            },
        ];
        let surfaces = BTreeMap::from([
            (first.coord, first),
            (sibling.coord, sibling),
            (mutable.coord, mutable),
            (target.coord, target),
        ]);
        let authority = exact_bridge_approach_authority(&approaches, &surfaces)
            .expect("both pregraded bridge lanes publish exact shared authority");
        let hard_forbidden = authority.keys().copied().collect::<BTreeSet<_>>();
        let preserved = hard_forbidden.clone();
        let footprint = surfaces.keys().copied().collect::<BTreeSet<_>>();
        let ordinary_mask = footprint.clone();
        let network = BTreeMap::from([(target.coord, vec![target])]);
        let mut volume = VolumePlan::new(footprint.clone());
        volume.surfaces.insert(
            target,
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: None,
            },
        );

        let (path, selected_target, levels) = solve_required_ordinary_connector(
            first,
            OrdinaryRegionBand::Lower,
            &volume,
            &footprint,
            &ordinary_mask,
            &hard_forbidden,
            &preserved,
            &network,
            &surfaces,
            &BTreeSet::new(),
            None,
        )
        .expect("the first lane can cross its exact sibling without grading it");
        assert_eq!(
            path,
            vec![first.coord, sibling.coord, mutable.coord, target.coord]
        );
        assert_eq!(selected_target, target);
        assert_eq!(levels.first().copied(), Some(first.level));
        assert_eq!(levels.get(1).copied(), Some(sibling.level));
        assert_eq!(levels.last().copied(), Some(target.level));
        assert!(levels.windows(2).all(|pair| pair[0].abs_diff(pair[1]) <= 1));

        let mut drifted = surfaces;
        drifted.insert(sibling.coord, TilePos::new(sibling.coord, 16));
        assert!(exact_bridge_approach_authority(&approaches, &drifted).is_err());
    }

    #[test]
    fn exact_bridge_landing_can_grade_one_mutable_network_apron() {
        let landing = TilePos::new(HexCoord::from_axial(0, 0), 15);
        let apron = TilePos::new(HexCoord::from_axial(1, 0), 17);
        let target = TilePos::new(HexCoord::from_axial(2, 0), 17);
        let footprint = BTreeSet::from([landing.coord, apron.coord, target.coord]);
        let ordinary_mask = footprint.clone();
        let hard_forbidden = BTreeSet::from([landing.coord]);
        let preserved = hard_forbidden.clone();
        let approach = BridgeBankApproach {
            structure: StructureId(BRIDGE_STRUCTURE_BASE),
            bank_index: 0,
            lane_index: 0,
            surface: landing,
        };
        let no_touch = BTreeSet::from([landing.coord]);
        let network = BTreeMap::from([(apron.coord, vec![apron]), (target.coord, vec![target])]);
        let surfaces = BTreeMap::from([
            (landing.coord, landing),
            (apron.coord, apron),
            (target.coord, target),
        ]);
        let mut volume = VolumePlan::new(footprint.clone());
        for surface in [landing, apron, target] {
            volume.columns.insert(
                surface.coord,
                land_column(surface.level, SolidMaterialRole::Gravel),
            );
            volume.surfaces.insert(
                surface,
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
            );
        }

        assert!(solve_required_ordinary_connector(
            landing,
            OrdinaryRegionBand::Lower,
            &volume,
            &footprint,
            &ordinary_mask,
            &hard_forbidden,
            &preserved,
            &network,
            &surfaces,
            &no_touch,
            None,
        )
        .is_none());
        let (path, selected_target, levels) = solve_required_ordinary_connector(
            landing,
            OrdinaryRegionBand::Lower,
            &volume,
            &footprint,
            &ordinary_mask,
            &hard_forbidden,
            &preserved,
            &network,
            &surfaces,
            &no_touch,
            Some(approach),
        )
        .expect("a bridge-only apron supplies the exact 15-to-16-to-17 transition");
        assert_eq!(path, vec![landing.coord, apron.coord, target.coord]);
        assert_eq!(selected_target, target);
        assert_eq!(levels, vec![15, 16, 17]);

        for protected in [apron.coord, target.coord] {
            let protected_apron = BTreeSet::from([landing.coord, protected]);
            assert!(solve_required_ordinary_connector(
                landing,
                OrdinaryRegionBand::Lower,
                &volume,
                &footprint,
                &ordinary_mask,
                &hard_forbidden,
                &preserved,
                &network,
                &surfaces,
                &protected_apron,
                Some(approach),
            )
            .is_none());
        }

        let wrong_approach = BridgeBankApproach {
            surface: TilePos::new(landing.coord, 14),
            ..approach
        };
        assert!(solve_required_ordinary_connector(
            landing,
            OrdinaryRegionBand::Lower,
            &volume,
            &footprint,
            &ordinary_mask,
            &hard_forbidden,
            &preserved,
            &network,
            &surfaces,
            &no_touch,
            Some(wrong_approach),
        )
        .is_none());

        for interior_surface in [apron, target] {
            let mut interior_volume = volume.clone();
            interior_volume
                .surfaces
                .get_mut(&interior_surface)
                .expect("fixture surface remains published")
                .interior = Some(InteriorRegionId(9));
            assert!(solve_required_ordinary_connector(
                landing,
                OrdinaryRegionBand::Lower,
                &interior_volume,
                &footprint,
                &ordinary_mask,
                &hard_forbidden,
                &preserved,
                &network,
                &surfaces,
                &no_touch,
                Some(approach),
            )
            .is_none());
        }

        for nonordinary_surface in [apron, target] {
            let mut nonordinary_volume = volume.clone();
            nonordinary_volume
                .surfaces
                .get_mut(&nonordinary_surface)
                .expect("fixture surface remains published")
                .access = SurfaceAccess::NonStandable;
            assert!(solve_required_ordinary_connector(
                landing,
                OrdinaryRegionBand::Lower,
                &nonordinary_volume,
                &footprint,
                &ordinary_mask,
                &hard_forbidden,
                &preserved,
                &network,
                &surfaces,
                &no_touch,
                Some(approach),
            )
            .is_none());
        }

        for stacked_coord in [apron.coord, target.coord] {
            let mut stacked_volume = volume.clone();
            stacked_volume.surfaces.insert(
                TilePos::new(stacked_coord, 6),
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
            );
            assert!(solve_required_ordinary_connector(
                landing,
                OrdinaryRegionBand::Lower,
                &stacked_volume,
                &footprint,
                &ordinary_mask,
                &hard_forbidden,
                &preserved,
                &network,
                &surfaces,
                &no_touch,
                Some(approach),
            )
            .is_none());
        }

        for (wrong_apron_level, wrong_target_level) in [(18, 17), (17, 18)] {
            let wrong_apron = TilePos::new(apron.coord, wrong_apron_level);
            let wrong_target = TilePos::new(target.coord, wrong_target_level);
            let wrong_network = BTreeMap::from([
                (wrong_apron.coord, vec![wrong_apron]),
                (wrong_target.coord, vec![wrong_target]),
            ]);
            let wrong_surfaces = BTreeMap::from([
                (landing.coord, landing),
                (wrong_apron.coord, wrong_apron),
                (wrong_target.coord, wrong_target),
            ]);
            let mut wrong_volume = VolumePlan::new(footprint.clone());
            for surface in [landing, wrong_apron, wrong_target] {
                wrong_volume.columns.insert(
                    surface.coord,
                    land_column(surface.level, SolidMaterialRole::Gravel),
                );
                wrong_volume.surfaces.insert(
                    surface,
                    SurfaceMetadata {
                        access: SurfaceAccess::Ordinary,
                        interior: None,
                    },
                );
            }
            assert!(solve_required_ordinary_connector(
                landing,
                OrdinaryRegionBand::Lower,
                &wrong_volume,
                &footprint,
                &ordinary_mask,
                &hard_forbidden,
                &preserved,
                &wrong_network,
                &wrong_surfaces,
                &no_touch,
                Some(approach),
            )
            .is_none());
        }
    }

    #[test]
    fn authored_water_bank_cannot_be_laundered_into_a_bridge_approach() {
        let (bridges, volume, external_coords) = exact_bridge_approach_fixture(15);
        let ordinary_mask = volume.mask.clone();
        let empty = BTreeSet::new();
        let surface_by_coord = top_standable_surfaces_by_coord(&volume);

        bridge_bank_approaches(
            &bridges,
            &volume,
            None,
            &ordinary_mask,
            &empty,
            &external_coords,
            &surface_by_coord,
        )
        .expect("an unconflicted, already-walkable water bank may remain immutable");

        let error = bridge_bank_approaches(
            &bridges,
            &volume,
            None,
            &ordinary_mask,
            &external_coords,
            &external_coords,
            &surface_by_coord,
        )
        .expect_err("independent authored authority must win over the water-bank exception");
        let V3GenerationError::RecipeContract(detail) = error else {
            panic!("authored water-bank overlap returned the wrong error: {error:?}");
        };
        assert!(detail.contains("no two independent dry walker approaches"));
    }

    #[test]
    fn coord_local_surface_removal_reconciles_stacked_biome_metadata_only() {
        let coord = HexCoord::ORIGIN;
        let [neighbor, ..] = coord.neighbors();
        let lower = TilePos::new(coord, 4);
        let upper = TilePos::new(coord, 12);
        let stale_biome = TilePos::new(coord, 8);
        let neighboring = TilePos::new(neighbor, 7);
        let mut volume = VolumePlan::new(BTreeSet::from([coord, neighbor]));
        volume.surfaces.insert(
            lower,
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: Some(InteriorRegionId(3)),
            },
        );
        volume.surfaces.insert(
            upper,
            SurfaceMetadata {
                access: SurfaceAccess::SpecialMovement(SpecialMovementRegion(5)),
                interior: None,
            },
        );
        volume.surfaces.insert(
            neighboring,
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: None,
            },
        );
        let mut biomes = BTreeMap::from([
            (lower, hex_core::BiomeRegionId(10)),
            (upper, hex_core::BiomeRegionId(11)),
            (stale_biome, hex_core::BiomeRegionId(12)),
            (neighboring, hex_core::BiomeRegionId(13)),
        ]);

        remove_column_surfaces(&mut volume, &mut biomes, coord);

        assert!(volume.surfaces_at_coord(coord).next().is_none());
        assert!(biomes
            .range(TilePos::new(coord, Level::MIN)..=TilePos::new(coord, Level::MAX))
            .next()
            .is_none());
        assert_eq!(
            volume.top_surface_at_coord(neighbor),
            Some((
                neighboring,
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                }
            ))
        );
        assert_eq!(
            biomes,
            BTreeMap::from([(neighboring, hex_core::BiomeRegionId(13))])
        );
    }

    #[test]
    fn water_bank_raise_preserves_a_lower_authored_stack() {
        let coord = HexCoord::from_axial(0, 0);
        let lower = TilePos::new(coord, 6);
        let old_bank = TilePos::new(coord, 10);
        let raised_bank = TilePos::new(coord, 11);
        let lower_interior = InteriorRegionId(7);
        let mut volume = VolumePlan::new(BTreeSet::from([coord]));
        volume.columns.insert(
            coord,
            VolumeColumn {
                elements: vec![
                    VolumeElement::Solid(SolidMass {
                        levels: LevelInterval::new(0, 7),
                        material: SolidMaterialRole::WorkedStone,
                        cutaway_for: Some(lower_interior),
                    }),
                    VolumeElement::Solid(SolidMass {
                        levels: LevelInterval::new(9, 11),
                        material: SolidMaterialRole::Grass,
                        cutaway_for: None,
                    }),
                ],
            },
        );
        volume.surfaces.insert(
            lower,
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: Some(lower_interior),
            },
        );
        volume.surfaces.insert(
            old_bank,
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: None,
            },
        );
        let mut biomes = BTreeMap::from([
            (lower, hex_core::BiomeRegionId(17)),
            (old_bank, hex_core::BiomeRegionId(19)),
        ]);

        assert_eq!(
            raise_top_bank_surface_preserving_stacks(
                &mut volume,
                &mut biomes,
                coord,
                old_bank,
                raised_bank.level,
                None,
            )
            .expect("unrelated top bank can rise without replacing a lower route"),
            raised_bank,
        );
        assert!(volume.surfaces.contains_key(&lower));
        assert!(volume.surfaces.contains_key(&raised_bank));
        assert!(!volume.surfaces.contains_key(&old_bank));
        assert_eq!(volume.surfaces[&lower].interior, Some(lower_interior));
        assert_eq!(biomes.get(&lower), Some(&hex_core::BiomeRegionId(17)));
        assert_eq!(biomes.get(&raised_bank), Some(&hex_core::BiomeRegionId(19)));
        assert!(matches!(
            volume.columns[&coord].elements.as_slice(),
            [
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval { bottom: 0, top: 7 },
                    ..
                }),
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval { bottom: 9, top: 12 },
                    ..
                })
            ]
        ));
    }

    #[test]
    fn malformed_water_bank_cap_returns_a_typed_error_without_mutation() {
        let coord = HexCoord::ORIGIN;
        let declared = TilePos::new(coord, 10);
        let metadata = SurfaceMetadata {
            access: SurfaceAccess::Ordinary,
            interior: None,
        };
        let mut volume = VolumePlan::new(BTreeSet::from([coord]));
        volume.columns.insert(
            coord,
            VolumeColumn {
                elements: vec![VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(0, 5),
                    material: SolidMaterialRole::Grass,
                    cutaway_for: None,
                })],
            },
        );
        volume.surfaces.insert(declared, metadata);
        let mut biomes = BTreeMap::from([(declared, hex_core::BiomeRegionId(23))]);
        let before_column = volume.columns[&coord].clone();

        let error = raise_top_bank_surface_preserving_stacks(
            &mut volume,
            &mut biomes,
            coord,
            declared,
            11,
            None,
        )
        .expect_err("a declared surface without an exact cap run must fail closed");

        let detail = match error {
            V3GenerationError::RecipeContract(detail) => detail,
            unexpected => {
                panic!("malformed bank cap returned the wrong typed error: {unexpected:?}")
            }
        };
        assert!(detail.contains("not the top of one exact solid run"));
        assert_eq!(volume.columns[&coord], before_column);
        assert_eq!(volume.surfaces.get(&declared), Some(&metadata));
        assert_eq!(biomes.get(&declared), Some(&hex_core::BiomeRegionId(23)));
    }

    #[test]
    fn connector_skeleton_and_deficit_obey_the_shared_band_and_length_ceiling() {
        let coords = (0..=192)
            .map(|q| HexCoord::from_axial(q, 0))
            .collect::<Vec<_>>();
        let start = coords[0];
        let target = TilePos::new(coords[192], 150);
        let footprint = coords[..192].iter().copied().collect::<BTreeSet<_>>();
        let surfaces = footprint
            .iter()
            .copied()
            .map(|coord| (coord, TilePos::new(coord, 170)))
            .collect::<BTreeMap<_, _>>();
        let network = BTreeMap::from([(target.coord, vec![target])]);
        let skeletons = ordinary_connector_to_network(
            start,
            OrdinaryRegionBand::Upper,
            &footprint,
            &footprint,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &network,
            &surfaces,
        );
        assert_eq!(skeletons.len(), 1);
        assert_eq!(skeletons[0].0.len(), 193);

        let short_path = [coords[0], coords[1], coords[2]];
        let preserved = BTreeSet::from([coords[1]]);
        let bank_surfaces = BTreeMap::from([(coords[1], TilePos::new(coords[1], 16))]);
        assert!(connector_total_vertical_deficit(
            &short_path,
            170,
            TilePos::new(coords[2], 150),
            3,
            false,
            OrdinaryRegionBand::Upper,
            &preserved,
            &bank_surfaces,
        )
        .is_none());
    }

    fn straight_four_row_axis(row: &BTreeSet<HexCoord>) -> Option<usize> {
        (0..3).find(|direction| {
            row.iter().copied().any(|start| {
                (0..TUNNEL_LANE_WIDTH)
                    .map(|offset| {
                        step_in_direction(
                            start,
                            *direction,
                            i32::try_from(offset).unwrap_or(i32::MAX),
                        )
                    })
                    .collect::<BTreeSet<_>>()
                    == *row
            })
        })
    }

    #[test]
    fn ring_connector_rotates_through_corners_and_preserves_the_complete_locked_suffix() {
        let site_center = HexCoord::from_axial(7, -11);
        let crystal_mask = site_center
            .within_radius(CRYSTAL_CONNECTOR_SITE_RADIUS)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let site_ring = canonical_convex_hex_ring(site_center, CRYSTAL_CONNECTOR_SITE_RADIUS)
            .expect("exact radius-32 site has one canonical ring");
        let terminal_coords = convex_ring_window(&site_ring, 14, TUNNEL_LANE_WIDTH)
            .expect("fixture terminal is one exact four-cell radius-32 window");
        let terminal = terminal_coords
            .into_iter()
            .map(|coord| TilePos::new(coord, 6))
            .collect::<BTreeSet<_>>();
        let locked = site_center.line_between(step_in_direction(site_center, 1, 48));
        let footprint = site_center
            .within_radius(80)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let resolved =
            resolve_exact_terminal_lane(&terminal, &crystal_mask, &footprint, locked.clone())
                .expect("ring windows rotate the terminal into the locked lane");
        let first_outside = locked
            .iter()
            .position(|coord| !crystal_mask.contains(coord))
            .expect("locked path leaves the exact site");
        let locked_suffix = locked
            .iter()
            .copied()
            .skip(first_outside)
            .collect::<Vec<_>>();
        let splice_index = resolved
            .centerline
            .len()
            .checked_sub(locked_suffix.len())
            .expect("resolved route contains the locked suffix");
        let locked_rows = locked_suffix
            .iter()
            .enumerate()
            .map(|(index, _)| tunnel_lane_row(&locked_suffix, index, resolved.lane_offsets))
            .collect::<Vec<_>>();
        let connector_rows = resolved
            .rows
            .get(1..splice_index)
            .expect("connector has radius-33 rows before the locked suffix");

        assert!(resolved
            .centerline
            .first()
            .is_some_and(|coord| terminal.iter().any(|position| position.coord == *coord)));
        assert_eq!(
            resolved.centerline.get(splice_index..),
            Some(locked_suffix.as_slice())
        );
        assert_eq!(
            resolved.rows.get(splice_index..),
            Some(locked_rows.as_slice())
        );
        assert_eq!(
            resolved
                .centerline
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len(),
            resolved.centerline.len()
        );
        assert!(resolved
            .centerline
            .windows(2)
            .all(|pair| pair[0].distance(pair[1]) == 1));
        assert!(resolved
            .rows
            .windows(2)
            .all(|pair| lane_rows_connect_smoothly(&pair[0], &pair[1])));
        assert!(connector_rows.iter().flatten().all(|coord| {
            site_center.distance(*coord) == CRYSTAL_CONNECTOR_RING_RADIUS
                && !crystal_mask.contains(coord)
        }));
        assert!(connector_rows
            .iter()
            .any(|row| straight_four_row_axis(row).is_none()));

        let terminal_coords = terminal
            .iter()
            .map(|position| position.coord)
            .collect::<BTreeSet<_>>();
        let terminal_axis = straight_four_row_axis(&terminal_coords)
            .expect("authored terminal is straight within one ring side");
        let locked_axis = locked_rows
            .first()
            .and_then(straight_four_row_axis)
            .expect("locked goal row retains its straight coarse-lane axis");
        assert_ne!(terminal_axis, locked_axis);
    }

    #[test]
    fn concealed_tunnel_approach_stays_exactly_four_lanes() {
        let exact_four = [-2, -1, 0, 1];
        assert_eq!(tunnel_approach_lane_offsets(exact_four), exact_four);
    }

    #[test]
    fn tunnel_alcove_rejects_one_shallow_radius_one_side_cell() {
        let origin = HexCoord::ORIGIN;
        let footprint = origin.within_radius(1).into_iter().collect::<BTreeSet<_>>();
        let mut volume = VolumePlan::new(footprint.clone());
        for coord in &footprint {
            let surface = TilePos::new(*coord, 16);
            volume
                .columns
                .insert(*coord, land_column(surface.level, SolidMaterialRole::Stone));
            volume.surfaces.insert(
                surface,
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
            );
        }
        assert!(tunnel_alcove_candidate_has_complete_roof_support(
            origin,
            &BTreeSet::new(),
            &volume,
            16,
        ));

        let shallow = *origin
            .neighbors()
            .first()
            .expect("a hex origin has six radius-one side cells");
        volume.surfaces.remove(&TilePos::new(shallow, 16));
        volume
            .columns
            .insert(shallow, land_column(15, SolidMaterialRole::Stone));
        volume.surfaces.insert(
            TilePos::new(shallow, 15),
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: None,
            },
        );

        assert!(!tunnel_alcove_candidate_has_complete_roof_support(
            origin,
            &BTreeSet::new(),
            &volume,
            16,
        ));
        assert!(tunnel_alcove_candidate_has_complete_roof_support(
            origin,
            &BTreeSet::from([shallow]),
            &volume,
            16,
        ));
    }

    #[test]
    fn concealed_tunnel_preserves_natural_overburden_strata() {
        let existing = land_column(20, SolidMaterialRole::Snow);
        let interior_id = InteriorRegionId(77);
        let carved = tunnel_column(&existing, 20, 6, 13, 16, false, interior_id);

        for level in 13..16 {
            let roof = solid_mass_at_level(&carved, level).expect("roof voxel remains solid");
            assert_eq!(roof.material, SolidMaterialRole::Stone);
            assert_eq!(roof.cutaway_for, Some(interior_id));
        }
        for level in 16..18 {
            let rock = solid_mass_at_level(&carved, level).expect("source rock remains above roof");
            assert_eq!(rock.material, SolidMaterialRole::Stone);
            assert_eq!(rock.cutaway_for, None);
        }
        for level in 18..20 {
            let soil = solid_mass_at_level(&carved, level).expect("source soil stratum remains");
            assert_eq!(soil.material, SolidMaterialRole::Dirt);
            assert_eq!(soil.cutaway_for, None);
        }
        let cap = solid_mass_at_level(&carved, 20).expect("source snow cap remains");
        assert_eq!(cap.material, SolidMaterialRole::Snow);
        assert_eq!(cap.cutaway_for, None);
    }

    #[test]
    fn roofed_tunnel_rejects_a_source_column_below_its_required_roof() {
        let coord = HexCoord::ORIGIN;
        let surface = TilePos::new(coord, 15);
        let mut volume = VolumePlan::new(BTreeSet::from([coord]));
        volume
            .columns
            .insert(coord, land_column(surface.level, SolidMaterialRole::Grass));
        volume.surfaces.insert(
            surface,
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: None,
            },
        );

        let error = capture_tunnel_overburden_authority(&volume, &BTreeSet::from([coord]), 16)
            .expect_err("a low source must not be raised into a visible linear cap");
        assert!(error.to_string().contains("lies below required roof top"));
    }

    #[test]
    fn final_tunnel_overburden_rejects_height_strata_and_cap_mutations() {
        let fixture = reference_fixture();
        let authority = {
            let world = &fixture.selection.validated.plan;
            let route = &world.features.protected_routes["grand_v3.tunnel"];
            let mouth = world.anchors["grand_v3.tunnel_mouth"];
            let crystal_mask = world
                .layout
                .patches
                .values()
                .find(|patch| {
                    patch
                        .mask
                        .contains(&world.anchors["crystal_ascent.lower_entry"].coord)
                })
                .map(|patch| &patch.mask)
                .expect("reference Crystal entry retains one claimed mask");
            let mouth_index = route
                .centerline
                .iter()
                .position(|surface| *surface == mouth)
                .expect("reference mouth belongs to its route");
            let coord = route
                .centerline
                .iter()
                .take(mouth_index)
                .map(|surface| surface.coord)
                .find(|coord| {
                    !crystal_mask.contains(coord)
                        && world
                            .volume
                            .top_surface_at_coord(*coord)
                            .is_some_and(|(surface, _)| surface.level > 16)
                })
                .expect("reference roofed tunnel has deep natural overburden");
            capture_tunnel_overburden_authority(&world.volume, &BTreeSet::from([coord]), 16)
                .expect("reference roofed column captures exact overburden")
        };
        let fine_index = FineWorldIndex::from_layout(&fixture.selection.validated.plan.layout)
            .expect("reference world retains exact fine ownership");
        corrective::validate_tunnel_overburden_authority(
            &fixture.plan,
            &fixture.selection.validated.plan,
            &fine_index,
            &authority,
        )
        .expect("captured final overburden is self-consistent");
        let expected = authority
            .columns
            .first_key_value()
            .map(|(coord, column)| (*coord, column.clone()))
            .expect("one overburden column was captured");

        let mut moved = fixture.selection.validated.plan.clone();
        replace_test_surface_level(
            &mut moved,
            expected.0,
            expected.1.surface.level.saturating_add(1),
        );
        let error = corrective::validate_tunnel_overburden_authority(
            &fixture.plan,
            &moved,
            &fine_index,
            &authority,
        )
        .expect_err("moving the concealed terrain cap must fail closed");
        assert!(error.to_string().contains("overburden moved"));

        let mut changed_stratum = fixture.selection.validated.plan.clone();
        let stratum = 16;
        let mass = changed_stratum
            .volume
            .columns
            .get_mut(&expected.0)
            .expect("mutated overburden retains its column")
            .elements
            .iter_mut()
            .find_map(|element| match element {
                VolumeElement::Solid(mass)
                    if mass.levels.bottom <= stratum && stratum < mass.levels.top =>
                {
                    Some(mass)
                }
                VolumeElement::Solid(_) | VolumeElement::Fill(_) => None,
            })
            .expect("mutated overburden retains level sixteen");
        mass.material = if expected.1.voxels[&stratum].material == SolidMaterialRole::WorkedStone {
            SolidMaterialRole::Grass
        } else {
            SolidMaterialRole::WorkedStone
        };
        let error = corrective::validate_tunnel_overburden_authority(
            &fixture.plan,
            &changed_stratum,
            &fine_index,
            &authority,
        )
        .expect_err("changing one concealed natural stratum must fail closed");
        assert!(error.to_string().contains("overburden changed"));

        let mut changed_cap = fixture.selection.validated.plan.clone();
        let cap_level = expected.1.surface.level;
        let mass = changed_cap
            .volume
            .columns
            .get_mut(&expected.0)
            .expect("mutated cap retains its column")
            .elements
            .iter_mut()
            .find_map(|element| match element {
                VolumeElement::Solid(mass)
                    if mass.levels.bottom <= cap_level && cap_level < mass.levels.top =>
                {
                    Some(mass)
                }
                VolumeElement::Solid(_) | VolumeElement::Fill(_) => None,
            })
            .expect("mutated overburden retains its cap voxel");
        mass.material = if expected.1.voxels[&cap_level].material == SolidMaterialRole::WorkedStone
        {
            SolidMaterialRole::Grass
        } else {
            SolidMaterialRole::WorkedStone
        };
        let error = corrective::validate_tunnel_overburden_authority(
            &fixture.plan,
            &changed_cap,
            &fine_index,
            &authority,
        )
        .expect_err("an unauthorised exposed cap material must fail closed");
        assert!(error.to_string().contains("overburden changed"));
    }

    #[test]
    fn corrective_tunnel_validator_rejects_a_side_lane_roof_ownership_loss() {
        let mut world = reference_fixture().selection.validated.plan.clone();
        let profile = V3GrandV3BasicTerrainProfile::canonical();
        corrective::validate_concealed_tunnel(&world, profile)
            .expect("reference tunnel satisfies the corrective contract");

        let route = &world.features.protected_routes["grand_v3.tunnel"];
        let mouth = world.anchors["grand_v3.tunnel_mouth"];
        let mouth_index = route
            .centerline
            .iter()
            .position(|surface| *surface == mouth)
            .expect("mouth belongs to the tunnel centerline");
        let threshold_index = mouth_index
            .checked_sub(1)
            .expect("mouth follows a roofed row");
        let centerline = route
            .centerline
            .iter()
            .map(|surface| surface.coord)
            .collect::<Vec<_>>();
        let foot_entries = world
            .interiors
            .by_id
            .values()
            .next()
            .expect("reference has one interior")
            .entrances
            .iter()
            .copied()
            .filter(|surface| surface.level == profile.crystal_base_level)
            .collect::<BTreeSet<_>>();
        let threshold_row = [[-1, 0, 1, 2], [-2, -1, 0, 1]]
            .into_iter()
            .map(|offsets| tunnel_lane_row(&centerline, threshold_index, offsets))
            .find(|row| {
                row.iter()
                    .copied()
                    .map(|coord| TilePos::new(coord, profile.crystal_base_level))
                    .collect::<BTreeSet<_>>()
                    == foot_entries
            })
            .expect("one even-width bias reproduces the exact entrance row");
        let side_lane = threshold_row
            .iter()
            .copied()
            .find(|coord| *coord != centerline[threshold_index])
            .expect("four-wide threshold has a side lane");
        let roof = world
            .volume
            .columns
            .get_mut(&side_lane)
            .expect("side lane has a semantic column")
            .elements
            .iter_mut()
            .find_map(|element| match element {
                VolumeElement::Solid(mass) if mass.levels.bottom <= 13 && 13 < mass.levels.top => {
                    Some(mass)
                }
                VolumeElement::Solid(_) | VolumeElement::Fill(_) => None,
            })
            .expect("side lane has the authored roof voxel");
        roof.cutaway_for = None;

        let error = corrective::validate_concealed_tunnel(&world, profile)
            .expect_err("losing one side-lane roof owner must fail closed");
        assert!(error
            .to_string()
            .contains("lacks exact floor, roof, or unified interior ownership"));
    }

    #[test]
    fn exact_terminal_ring_splice_is_bounded_shortest_and_canonical() {
        let site_center = HexCoord::from_axial(7, -11);
        let crystal_mask = site_center
            .within_radius(CRYSTAL_CONNECTOR_SITE_RADIUS)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let site_ring = canonical_convex_hex_ring(site_center, CRYSTAL_CONNECTOR_SITE_RADIUS)
            .expect("exact radius-32 site has one canonical ring");
        let terminal = convex_ring_window(&site_ring, 80, TUNNEL_LANE_WIDTH)
            .expect("fixture terminal is one exact radius-32 window")
            .into_iter()
            .map(|coord| TilePos::new(coord, 6))
            .collect::<BTreeSet<_>>();
        let locked = site_center.line_between(step_in_direction(site_center, 0, 80));
        let footprint = site_center
            .within_radius(112)
            .into_iter()
            .collect::<BTreeSet<_>>();

        let started = std::time::Instant::now();
        let resolved =
            resolve_exact_terminal_lane(&terminal, &crystal_mask, &footprint, locked.clone())
                .expect("sliding windows bend the exact terminal around the convex site");
        let elapsed = started.elapsed();
        let first_outside = locked
            .iter()
            .position(|coord| !crystal_mask.contains(coord))
            .expect("locked path leaves the exact site");
        let locked_suffix = locked
            .iter()
            .copied()
            .skip(first_outside)
            .collect::<Vec<_>>();
        let splice_index = resolved
            .centerline
            .len()
            .checked_sub(locked_suffix.len())
            .expect("resolved route retains the locked suffix");
        let connector_ring = canonical_convex_hex_ring(site_center, CRYSTAL_CONNECTOR_RING_RADIUS)
            .expect("connector ring is exact");
        let terminal_coords = terminal
            .iter()
            .map(|position| position.coord)
            .collect::<BTreeSet<_>>();
        let locked_goal_row = tunnel_lane_row(&locked_suffix, 0, resolved.lane_offsets);
        let start_windows = (0..connector_ring.len()).filter(|start| {
            convex_ring_window(&connector_ring, *start, TUNNEL_LANE_WIDTH)
                .is_some_and(|row| lane_rows_connect_smoothly(&terminal_coords, &row))
        });
        let goal_windows = (0..connector_ring.len())
            .filter(|start| {
                convex_ring_window(&connector_ring, *start, TUNNEL_LANE_WIDTH)
                    .is_some_and(|row| lane_rows_connect_smoothly(&row, &locked_goal_row))
            })
            .collect::<Vec<_>>();
        let connector_ring_len = connector_ring.len();
        let goal_center = *locked_suffix
            .first()
            .expect("locked suffix has its first outside center");
        let mut shortest = None;
        for start in start_windows {
            for goal in &goal_windows {
                for (_, arc) in preferred_cyclic_ring_arcs(connector_ring_len, start, *goal) {
                    let admits_anchors = (0..TUNNEL_LANE_WIDTH).any(|anchor_offset| {
                        let anchors = arc
                            .iter()
                            .filter_map(|window| {
                                connector_ring
                                    .get(window.saturating_add(anchor_offset) % connector_ring_len)
                                    .copied()
                            })
                            .collect::<Vec<_>>();
                        let first_anchor = anchors.first().copied();
                        let last_anchor = anchors.last().copied();
                        first_anchor.is_some_and(|anchor| {
                            terminal_coords
                                .iter()
                                .any(|terminal| terminal.distance(anchor) == 1)
                        }) && last_anchor.is_some_and(|anchor| anchor.distance(goal_center) == 1)
                            && anchors.len() == arc.len()
                            && anchors.iter().all(|anchor| !locked_suffix.contains(anchor))
                    });
                    if admits_anchors {
                        shortest = Some(
                            shortest.map_or(arc.len(), |current: usize| current.min(arc.len())),
                        );
                    }
                }
            }
        }
        let shortest = shortest.expect("fixture has smooth ring windows at both joins");

        assert_eq!(splice_index.saturating_sub(1), shortest);
        let tied_arcs = preferred_cyclic_ring_arcs(12, 0, 6);
        assert_eq!(tied_arcs.first().map(|(rank, _)| *rank), Some(0));
        assert_eq!(
            tied_arcs.first().map(|(_, arc)| arc.len()),
            tied_arcs.get(1).map(|(_, arc)| arc.len())
        );
        assert!(
            elapsed < std::time::Duration::from_millis(2_000),
            "exact terminal ring splice took {elapsed:?}; expected bounded milliseconds"
        );
    }

    #[test]
    fn constrained_pass_grade_is_rotation_symmetric_and_keeps_full_width() {
        let rotate = |mut coord: HexCoord, turns: u32| {
            for _ in 0..turns {
                coord = HexCoord::from_axial(-coord.y(), coord.x().saturating_add(coord.y()));
            }
            coord
        };
        for turns in 0..6 {
            let corridor = (0..=8)
                .flat_map(|q| (-1..=1).map(move |r| rotate(HexCoord::from_axial(q, r), turns)))
                .collect::<BTreeSet<_>>();
            let start = TilePos::new(rotate(HexCoord::from_axial(0, 0), turns), 144);
            let target = TilePos::new(rotate(HexCoord::from_axial(8, 0), turns), 152);
            let bank = rotate(HexCoord::from_axial(6, 1), turns);
            let levels = graded_corridor_levels_with_minimums(
                &corridor,
                start,
                target,
                &BTreeMap::from([(bank, 151)]),
            )
            .expect("rotated full-width pass admits the bank-height cone");
            assert_eq!(levels.len(), corridor.len());
            assert_eq!(levels.get(&start.coord), Some(&start.level));
            assert_eq!(levels.get(&target.coord), Some(&target.level));
            assert!(levels.get(&bank).is_some_and(|level| *level >= 151));
            assert!(corridor.iter().all(|coord| {
                coord.neighbors().into_iter().all(|neighbor| {
                    !corridor.contains(&neighbor)
                        || levels
                            .get(coord)
                            .zip(levels.get(&neighbor))
                            .is_some_and(|(level, other)| level.abs_diff(*other) <= 1)
                })
            }));
        }
    }

    #[test]
    fn hero_natural_pass_clears_every_exact_water_bank() {
        let world = &reference_fixture().selection.validated.plan;
        let natural = world
            .features
            .protected_routes
            .get("grand_v3.natural_pass")
            .expect("reference publishes the natural pass");
        let minimums = recessed_water_bank_minimums(&world.volume);
        assert!(natural.surfaces.iter().all(|surface| {
            minimums
                .get(&surface.coord)
                .is_none_or(|minimum| surface.level >= *minimum)
        }));
    }

    #[test]
    fn exact_reference_compiles_to_the_checkpoint_footprint() {
        let selection = &reference_fixture().selection;
        assert_eq!(selection.metrics.schematic_cells, 217);
        assert_eq!(selection.metrics.world_columns, 105_469);
        assert_eq!(selection.metrics.expected_chunks, 444);
        assert_eq!(selection.validated.plan.layout.patches.len(), 217);
        assert_eq!(selection.validated.plan.layout.shared_edges.len(), 0);
        assert_eq!(
            selection.validated.plan.source_schematic_fingerprint,
            Some(reference_fixture().plan.semantic_fingerprint)
        );
        assert!((super::super::schematic_highlands::MASSIF_SUMMIT_MIN
            ..=super::super::schematic_highlands::MASSIF_SUMMIT_MAX)
            .contains(&selection.metrics.maximum_surface));
        let generally_admitted =
            ValidatedWorldPlan::validate_complete(selection.validated.plan.clone())
                .and_then(super::super::selection::CompleteWorldAdmission::fingerprint)
                .expect("the unchanged general validator admits the Grand construction");
        assert_eq!(
            generally_admitted.semantic_fingerprint(),
            selection.validated.semantic_fingerprint(),
            "Grand admission must not change canonical semantic identity"
        );
    }

    #[test]
    fn final_peak_authority_rejects_one_drifted_seeded_summit_pin() {
        let authority = reference_peak_ridge_authority();
        let mut world = reference_fixture().selection.validated.plan.clone();
        corrective::validate_peak_ridge_authority(&world, authority)
            .expect("reference final peak ridges satisfy their authority");
        let (pin, expected) = authority.components[0]
            .summit_pins
            .first_key_value()
            .map(|(coord, level)| (*coord, *level))
            .expect("reference peak authority has one summit pin");
        replace_test_surface_level(&mut world, pin, expected.saturating_sub(1));
        let error = corrective::validate_peak_ridge_authority(&world, authority)
            .expect_err("one moved summit pin must fail final validation");
        assert!(error.to_string().contains("lost deterministic level"));
    }

    #[test]
    fn final_peak_authority_rejects_a_disconnected_high_chain() {
        let authority = reference_peak_ridge_authority();
        let mut world = reference_fixture().selection.validated.plan.clone();
        let component = &authority.components[0];
        let authorized_low = component
            .authorized_route_grades
            .as_ref()
            .expect("reference peak authority is sealed")
            .iter()
            .filter_map(|(coord, level)| (*level < 200).then_some(*coord))
            .collect::<BTreeSet<_>>();
        let expected_high = component
            .expected_high_band
            .keys()
            .copied()
            .collect::<BTreeSet<_>>();
        let (pin, lowered) = component
            .summit_pins
            .keys()
            .find_map(|pin| {
                let patch = component
                    .patch_masks
                    .values()
                    .find(|mask| mask.contains(pin))?;
                let lowered = patch
                    .intersection(&expected_high)
                    .copied()
                    .filter(|coord| *coord != *pin)
                    .collect::<BTreeSet<_>>();
                let retained = expected_high
                    .difference(&lowered)
                    .copied()
                    .chain(lowered.intersection(&authorized_low).copied())
                    .collect::<BTreeSet<_>>();
                (!lowered.is_empty() && !connected_coords(&retained)).then_some((*pin, lowered))
            })
            .expect("one locked peak patch can be severed beyond the authored low-route bridges");
        for coord in lowered {
            replace_test_surface_level(&mut world, coord, 199);
        }
        assert_eq!(
            world
                .volume
                .top_surface_at_coord(pin)
                .map(|(surface, _)| surface.level),
            component.summit_pins.get(&pin).copied()
        );
        let error = corrective::validate_peak_ridge_authority(&world, authority)
            .expect_err("isolating one summit must break the final high chain");
        assert!(error
            .to_string()
            .contains("not one connected high topology"));
    }

    #[test]
    fn final_peak_authority_rejects_non_route_high_band_flattening() {
        let authority = reference_peak_ridge_authority();
        let mut world = reference_fixture().selection.validated.plan.clone();
        let authorized_grades = authority
            .components
            .iter()
            .flat_map(|component| {
                component
                    .authorized_route_grades
                    .as_ref()
                    .expect("reference peak authority is sealed")
                    .keys()
                    .copied()
            })
            .collect::<BTreeSet<_>>();
        let pins = authority
            .components
            .iter()
            .flat_map(|component| component.summit_pins.keys().copied())
            .collect::<BTreeSet<_>>();
        let (coord, expected) = authority
            .components
            .iter()
            .flat_map(|component| &component.expected_high_band)
            .find(|(coord, _)| !pins.contains(coord) && !authorized_grades.contains(coord))
            .map(|(coord, level)| (*coord, *level))
            .expect("reference ridge has one non-route non-pin high-band coordinate");
        let flattened = if expected < 218 {
            expected.saturating_add(1)
        } else {
            expected.saturating_sub(1)
        };
        replace_test_surface_level(&mut world, coord, flattened);
        let error = corrective::validate_peak_ridge_authority(&world, authority)
            .expect_err("flattening one surviving seeded ridge level must fail closed");
        assert!(error.to_string().contains("changed exact authorized level"));
    }

    #[test]
    fn final_peak_authority_rejects_drift_inside_an_authored_route_grade() {
        let authority = reference_peak_ridge_authority();
        let mut world = reference_fixture().selection.validated.plan.clone();
        let (coord, authorized) = authority
            .components
            .iter()
            .flat_map(|component| {
                component
                    .authorized_route_grades
                    .as_ref()
                    .expect("reference peak authority is sealed")
            })
            .next()
            .map(|(coord, level)| (*coord, *level))
            .expect("reference peak routes grade at least one seeded high-band coordinate");
        let drifted = if authorized == Level::MAX {
            authorized.saturating_sub(1)
        } else {
            authorized.saturating_add(1)
        };
        replace_test_surface_level(&mut world, coord, drifted);

        let error = corrective::validate_peak_ridge_authority(&world, authority)
            .expect_err("an exact authored route grade may not drift later");
        assert!(error.to_string().contains("changed exact authorized level"));
    }

    #[test]
    fn peak_route_grade_seal_rejects_an_off_route_pre_network_mutation() {
        let mut authority = unsealed_reference_peak_ridge_authority();
        let mut world = reference_fixture().selection.validated.plan.clone();
        let authored_routes = AUTHORED_PEAK_ROUTE_NAMES
            .into_iter()
            .flat_map(|name| {
                world.features.protected_routes[name]
                    .surfaces
                    .iter()
                    .map(|surface| surface.coord)
            })
            .collect::<BTreeSet<_>>();
        let (coord, expected) = authority
            .components
            .iter()
            .flat_map(|component| &component.expected_high_band)
            .find(|(coord, _)| !authored_routes.contains(coord))
            .map(|(coord, level)| (*coord, *level))
            .expect("reference ridge has one seeded high coordinate outside authored routes");
        replace_test_surface_level(&mut world, coord, expected.saturating_sub(1));

        let error = seal_peak_ridge_route_grades(&mut authority, &world.volume, &world.features)
            .expect_err("a generic pre-network mutation must not enter peak authority");
        assert!(error
            .to_string()
            .contains("outside the exact authored peak routes"));
    }

    #[test]
    fn final_peak_authority_rejects_an_unauthorized_additional_high_surface() {
        let authority = reference_peak_ridge_authority();
        let mut world = reference_fixture().selection.validated.plan.clone();
        let component = &authority.components[0];
        let intentional_routes = [
            "grand_v3.natural_pass",
            "grand_v3.peak_saddle",
            "grand_v3.peak_foothill_ledge",
        ]
        .into_iter()
        .flat_map(|name| {
            world.features.protected_routes[name]
                .surfaces
                .iter()
                .map(|surface| surface.coord)
        })
        .collect::<BTreeSet<_>>();
        let coord = component
            .patch_masks
            .values()
            .flat_map(|mask| mask.iter().copied())
            .find(|coord| {
                !component.expected_high_band.contains_key(coord)
                    && !intentional_routes.contains(coord)
                    && world
                        .volume
                        .top_surface_at_coord(*coord)
                        .is_some_and(|(surface, _)| surface.level < 200)
            })
            .expect("reference ridge has one lower non-route shoulder");
        replace_test_surface_level(&mut world, coord, 200);

        let error = corrective::validate_peak_ridge_authority(&world, authority)
            .expect_err("raising a new shoulder into the high band must fail closed");
        assert!(error.to_string().contains("unauthorized >=200 surface"));
    }

    #[test]
    fn peak_saddle_reserves_the_authored_walk_and_publishes_a_simple_spine() {
        let world = &reference_fixture().selection.validated.plan;
        let natural = world
            .features
            .protected_routes
            .get("grand_v3.natural_pass")
            .expect("reference publishes the natural pass");
        let saddle = world
            .features
            .protected_routes
            .get("grand_v3.peak_saddle")
            .expect("reference publishes the peak saddle");
        validate_protected_route_integrity("test natural pass", natural, &world.volume)
            .expect("saddle carving preserves every natural-pass surface and edge");
        assert!(saddle.surfaces.len() >= saddle.centerline.len());
        assert_eq!(
            saddle
                .centerline
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len(),
            saddle.centerline.len(),
            "the published saddle spine must remain simple"
        );
        assert!(saddle
            .centerline
            .iter()
            .all(|position| saddle.surfaces.contains(position)));
        let junction = saddle
            .centerline
            .first()
            .copied()
            .expect("saddle spine has one exact natural junction");
        assert!(
            natural.surfaces.contains(&junction),
            "the simple published spine must begin at an exact natural-pass junction"
        );

        assert!(
            saddle
                .surfaces
                .iter()
                .any(|position| natural.surfaces.contains(position)),
            "the reserved saddle footprint needs an exact natural-pass junction"
        );
        assert!(
            saddle
                .surfaces
                .iter()
                .any(|position| !natural.surfaces.contains(position)),
            "the reserved saddle footprint needs carved PeakRing surfaces"
        );

        let expected = [
            ((3, -6, 3), 124_u16),
            ((4, -7, 3), 166),
            ((5, -7, 2), 167),
            ((6, -7, 1), 168),
            ((6, -6, 0), 91),
        ];
        let mut visited = Vec::new();
        for ((q, r, s), expected_id) in expected {
            let coord = SchematicCoord::new(q, r, s).expect("fixed saddle coord is canonical");
            let cell = reference_fixture()
                .plan
                .cell(coord)
                .expect("fixed saddle cell exists");
            assert_eq!(cell.id.get(), expected_id);
            let patch = world
                .layout
                .patches
                .get(&PatchId(u32::from(expected_id)))
                .expect("fixed saddle cell owns one resolved patch");
            assert!(
                saddle
                    .surfaces
                    .iter()
                    .any(|position| patch.mask.contains(&position.coord)),
                "reserved saddle footprint does not cover authored patch {expected_id}"
            );
            visited.push(expected_id);
        }
        assert_eq!(visited, [124, 166, 167, 168, 91]);
    }

    #[test]
    fn authored_peak_routes_never_enter_seeded_summit_pins() {
        let world = &reference_fixture().selection.validated.plan;
        let summit_pins = reference_peak_ridge_authority()
            .components
            .iter()
            .flat_map(|component| component.summit_pins.keys().copied())
            .collect::<BTreeSet<_>>();
        assert_eq!(summit_pins.len(), 12);

        for name in AUTHORED_PEAK_ROUTE_NAMES {
            let route = &world.features.protected_routes[name];
            assert!(
                route
                    .surfaces
                    .iter()
                    .all(|surface| !summit_pins.contains(&surface.coord)),
                "{name} entered one immutable seeded summit pin"
            );
        }
    }

    #[test]
    fn peak_foothill_ledge_is_one_exact_lower_branch_through_cells_92_and_93() {
        let world = &reference_fixture().selection.validated.plan;
        let natural = world
            .features
            .protected_routes
            .get("grand_v3.natural_pass")
            .expect("reference publishes the natural pass");
        let saddle = world
            .features
            .protected_routes
            .get("grand_v3.peak_saddle")
            .expect("reference publishes the Upper peak saddle");
        let ledge = world
            .features
            .protected_routes
            .get("grand_v3.peak_foothill_ledge")
            .expect("reference publishes the Lower peak foothill ledge");
        validate_protected_route_integrity("test peak foothill ledge", ledge, &world.volume)
            .expect("foothill ledge retains every exact Lower walker edge");

        let branch = ledge
            .centerline
            .first()
            .copied()
            .expect("foothill ledge has an exact branch node");
        assert!(natural.surfaces.contains(&branch));
        assert_eq!(
            OrdinaryRegionBand::containing(branch.level),
            OrdinaryRegionBand::Lower
        );
        assert!(ledge.centerline.iter().skip(1).all(|position| {
            OrdinaryRegionBand::Lower.accepts_new(position.level)
                && !natural.surfaces.contains(position)
        }));
        assert!(ledge.surfaces.is_disjoint(&saddle.surfaces));
        let branch_owner = world
            .layout
            .patches
            .iter()
            .find_map(|(owner, patch)| patch.mask.contains(&branch.coord).then_some(*owner))
            .expect("exact Lower branch retains one coarse owner");
        let permitted_owners = BTreeSet::from([branch_owner, PatchId(92), PatchId(93)]);
        assert!(
            ledge.surfaces.iter().all(|position| {
                world.layout.patches.iter().any(|(owner, patch)| {
                    permitted_owners.contains(owner) && patch.mask.contains(&position.coord)
                })
            }),
            "foothill approach must stay inside one exact branch-owner patch plus locked cells 92/93"
        );

        let expected = [((6, -5, -1), 92_u16), ((6, -4, -2), 93_u16)];
        let mut after = 0_usize;
        let mut visited = Vec::new();
        for ((q, r, s), expected_id) in expected {
            let coord = SchematicCoord::new(q, r, s).expect("fixed ledge coord is canonical");
            let cell = reference_fixture()
                .plan
                .cell(coord)
                .expect("fixed ledge cell exists");
            assert_eq!(cell.id.get(), expected_id);
            let patch = world
                .layout
                .patches
                .get(&PatchId(u32::from(expected_id)))
                .expect("fixed ledge cell owns one resolved patch");
            let index = ledge
                .centerline
                .iter()
                .enumerate()
                .skip(after)
                .find_map(|(index, position)| patch.mask.contains(&position.coord).then_some(index))
                .unwrap_or_else(|| {
                    panic!("foothill ledge does not visit patch {expected_id} in authored order")
                });
            after = index.saturating_add(1);
            visited.push(expected_id);
        }
        assert_eq!(visited, [92, 93]);
    }

    #[test]
    fn reference_coast_detail_breaks_a_straight_coarse_boundary_without_changing_ownership() {
        let fixture = reference_fixture();
        let world = &fixture.selection.validated.plan;
        let cells = fixture
            .plan
            .cells
            .iter()
            .map(|cell| (PatchId(u32::from(cell.id.get())), cell))
            .collect::<BTreeMap<_, _>>();
        let is_sea = |patch: PatchId| {
            cells.get(&patch).is_some_and(|cell| {
                cell.facts.surface == SurfaceKind::OpenWater
                    && !has_overlay(cell, SchematicFeature::MountainLake)
                    && !has_overlay(cell, SchematicFeature::ValleyLake)
            })
        };
        let owner = world
            .layout
            .patches
            .iter()
            .flat_map(|(patch, resolved)| resolved.mask.iter().map(move |coord| (*coord, *patch)))
            .collect::<BTreeMap<_, _>>();
        let actual_water = world
            .volume
            .fill_runs_by_top()
            .keys()
            .map(|position| position.coord)
            .collect::<BTreeSet<_>>();
        let mut coarse_boundaries =
            BTreeMap::<(PatchId, PatchId), Vec<(HexCoord, HexCoord)>>::new();
        for (coord, patch) in &owner {
            for neighbor in coord.neighbors() {
                let Some(neighbor_patch) = owner.get(&neighbor).copied() else {
                    continue;
                };
                if *patch >= neighbor_patch || is_sea(*patch) == is_sea(neighbor_patch) {
                    continue;
                }
                coarse_boundaries
                    .entry((*patch, neighbor_patch))
                    .or_default()
                    .push((*coord, neighbor));
            }
        }
        let (_, longest) = coarse_boundaries
            .iter()
            .max_by_key(|(pair, boundary)| (boundary.len(), *pair))
            .expect("reference has a mainland/sea boundary");
        assert!(
            longest.len() >= 10,
            "fixture must exercise a straight coarse edge"
        );
        assert!(
            longest.iter().any(|(first, second)| {
                let first_semantic_water = is_sea(owner[first]);
                let second_semantic_water = is_sea(owner[second]);
                actual_water.contains(first) != first_semantic_water
                    || actual_water.contains(second) != second_semantic_water
            }),
            "coast_detail left the representative pitch-22 Voronoi edge completely straight"
        );
        assert_eq!(
            owner.len(),
            105_469,
            "fine coast detail cannot mutate ownership"
        );
    }

    #[test]
    fn coast_local_removal_proof_exhausts_every_neighbor_pattern() {
        let center = HexCoord::ORIGIN;
        let neighbors = center.neighbors();
        let mut admitted = 0_usize;
        let mut rejected = 0_usize;
        for mask in 0_u8..64 {
            let mut component = BTreeSet::from([center]);
            for (index, neighbor) in neighbors.iter().copied().enumerate() {
                if mask & (1_u8 << index) != 0 {
                    component.insert(neighbor);
                }
            }
            if removal_preserves_connectedness_locally(&component, center) {
                admitted = admitted.saturating_add(1);
                assert!(component.remove(&center));
                assert!(
                    connected_coords(&component),
                    "accepted local removal disconnected neighbor mask {mask:06b}"
                );
            } else {
                rejected = rejected.saturating_add(1);
            }
        }
        assert!(admitted > 0, "local proof rejected every neighbor pattern");
        assert!(rejected > 0, "local proof admitted every neighbor pattern");
        assert!(!removal_preserves_connectedness_locally(
            &BTreeSet::from([center]),
            center,
        ));
    }

    #[test]
    fn default_hero_seed_reference_coast_remains_connected_and_visibly_varied() {
        const DEFAULT_HERO_SEED: u64 = 1_592_598_566;

        let template = hex_schematic::grand_v3_reference_template().expect("template parses");
        let reference = hex_schematic::reference_plan(&template, DEFAULT_HERO_SEED)
            .expect("default reference plan validates");
        let selection = compile_schematic(
            &reference.plan,
            &settings(),
            V3_SCHEMATIC_GRID_RADIUS,
            0.4,
            super::super::vegetation::tests::runtime_art_catalog(),
        )
        .expect("default reference coast compiles without disconnecting either component");
        let world = &selection.validated.plan;
        let cells = reference
            .plan
            .cells
            .iter()
            .map(|cell| (PatchId(u32::from(cell.id.get())), cell))
            .collect::<BTreeMap<_, _>>();
        let is_sea = |patch: PatchId| {
            cells.get(&patch).is_some_and(|cell| {
                cell.facts.surface == SurfaceKind::OpenWater
                    && !has_overlay(cell, SchematicFeature::MountainLake)
                    && !has_overlay(cell, SchematicFeature::ValleyLake)
            })
        };
        let owner = world
            .layout
            .patches
            .iter()
            .flat_map(|(patch, resolved)| resolved.mask.iter().map(move |coord| (*coord, *patch)))
            .collect::<BTreeMap<_, _>>();
        let actual_water = world
            .volume
            .fill_runs_by_top()
            .keys()
            .map(|position| position.coord)
            .collect::<BTreeSet<_>>();
        let varied_boundary_columns = owner
            .iter()
            .filter(|(coord, patch)| {
                coord.neighbors().into_iter().any(|neighbor| {
                    owner
                        .get(&neighbor)
                        .is_some_and(|neighbor_patch| is_sea(*neighbor_patch) != is_sea(**patch))
                })
            })
            .filter(|(coord, patch)| actual_water.contains(*coord) != is_sea(**patch))
            .count();
        assert!(
            varied_boundary_columns > 0,
            "constructive rollback removed every visible default-seed coast mutation"
        );
        let crest = world.observation_anchors["grand_v3.massif_crest"];
        let massif_maxima = reference
            .plan
            .cells
            .iter()
            .filter(|cell| cell.facts.landform == LandformKind::Massif)
            .filter_map(|cell| world.layout.patches.get(&PatchId(u32::from(cell.id.get()))))
            .flat_map(|patch| patch.mask.iter().copied())
            .filter_map(|coord| world.volume.top_surface_at_coord(coord))
            .map(|(surface, _)| surface)
            .filter(|surface| surface.level == crest.level)
            .collect::<BTreeSet<_>>();
        assert_eq!(massif_maxima, BTreeSet::from([crest]));
        assert!(!world.features.protected_routes["grand_v3.ordinary_hubs"]
            .surfaces
            .iter()
            .any(|surface| surface.coord == crest.coord));
    }

    #[test]
    fn massif_crest_authority_captures_and_rejects_mutation_of_all_nineteen_surfaces() {
        let template = hex_schematic::grand_v3_reference_template().expect("template parses");
        let reference = hex_schematic::reference_plan(&template, 0).expect("reference validates");
        let settings = settings();
        let mut layout =
            resolve_layout(V3_SCHEMATIC_GRID_RADIUS, &settings).expect("schematic layout resolves");
        super::super::schematic_crystal::claim_site(&reference.plan, &mut layout, 22)
            .expect("Crystal site claim validates");
        let foundation = build_schematic_foundation(
            &reference.plan,
            &layout,
            V3GrandV3BasicTerrainProfile::canonical(),
        )
        .expect("foundation resolves exact Massif authority");
        let authority = &foundation.massif_crest;
        let expected_coords = authority
            .crest
            .coord
            .within_radius(MassifCrestAuthority::RADIUS)
            .into_iter()
            .collect::<BTreeSet<_>>();

        assert_eq!(authority.shoulder_surfaces.len(), 19);
        assert_eq!(authority.coords().collect::<BTreeSet<_>>(), expected_coords);
        authority
            .validate_geometry("test foundation", &foundation.volume)
            .expect("unmodified foundation retains exact authority");

        let replaced = *authority
            .shoulder_surfaces
            .iter()
            .find(|surface| **surface != authority.crest)
            .expect("radius-two authority has a non-crest shoulder");
        let mut changed = foundation.volume.clone();
        let metadata = changed.surfaces[&replaced];
        changed.remove_surfaces_at_coord(replaced.coord);
        let raised = TilePos::new(replaced.coord, replaced.level.saturating_add(1));
        changed.columns.insert(
            replaced.coord,
            land_column(raised.level, SolidMaterialRole::Stone),
        );
        changed.surfaces.insert(raised, metadata);
        let error = authority
            .validate_geometry("test mutation", &changed)
            .expect_err("replacing one shoulder cap must fail exact authority");
        assert!(error
            .to_string()
            .contains("moved or replaced natural Massif shoulder surface"));

        authority
            .validate_route_disjointness("test clear route", &BTreeSet::new())
            .expect("an empty route is disjoint");
        let error = authority
            .validate_route_disjointness(
                "test overlapping route",
                &BTreeSet::from([replaced.coord]),
            )
            .expect_err("a route may not carve an exact Massif shoulder");
        assert!(error
            .to_string()
            .contains("route intersects exact Massif crest authority"));
    }

    #[test]
    fn final_world_preserves_exact_massif_crest_authority_and_route_disjointness() {
        let fixture = reference_fixture();
        let world = &fixture.selection.validated.plan;
        let settings = settings();
        let mut layout =
            resolve_layout(V3_SCHEMATIC_GRID_RADIUS, &settings).expect("schematic layout resolves");
        super::super::schematic_crystal::claim_site(&fixture.plan, &mut layout, 22)
            .expect("Crystal site claim validates");
        let foundation = build_schematic_foundation(
            &fixture.plan,
            &layout,
            V3GrandV3BasicTerrainProfile::canonical(),
        )
        .expect("foundation resolves exact Massif authority");
        let authority = foundation.massif_crest;

        authority
            .validate_geometry("final compiled world", &world.volume)
            .expect("all nineteen natural Massif caps survive final construction");
        for surface in &authority.shoulder_surfaces {
            let access = world.volume.surfaces[surface].access;
            if *surface == authority.crest {
                assert_eq!(
                    access,
                    SurfaceAccess::SpecialMovement(INACCESSIBLE_MOVEMENT_REGION)
                );
            } else {
                assert!(matches!(
                    access,
                    SurfaceAccess::Ordinary
                        | SurfaceAccess::SpecialMovement(INACCESSIBLE_MOVEMENT_REGION)
                ));
            }
            assert_eq!(world.volume.surfaces[surface].interior, None);
        }
        for (name, route) in &world.features.protected_routes {
            if name == "grand_v3.ordinary_hubs" {
                continue;
            }
            authority
                .validate_route_disjointness(
                    &format!("final protected route {name}"),
                    &route.surfaces.iter().map(|surface| surface.coord).collect(),
                )
                .unwrap_or_else(|error| panic!("{error}"));
        }
        assert_eq!(
            world.observation_anchors["grand_v3.massif_crest"],
            authority.crest
        );
    }

    #[test]
    fn final_content_publishes_exact_bridges_lights_hubs_and_grounded_vegetation() {
        let fixture = reference_fixture();
        let world = &fixture.selection.validated.plan;
        let lower_entry = *world
            .anchors
            .get("crystal_ascent.lower_entry")
            .expect("Crystal lower entry remains present");
        let upper_exit = world.anchors["crystal_ascent.upper_exit"];
        let lower_terminal =
            &world.features.protected_routes["crystal_ascent.lower_terminal_pad"].surfaces;
        let upper_terminal =
            &world.features.protected_routes["crystal_ascent.upper_terminal_pad"].surfaces;
        let crystal_mask = &world
            .layout
            .patches
            .values()
            .find(|patch| patch.mask.contains(&lower_entry.coord))
            .expect("lower entry retains its exact Crystal owner")
            .mask;
        let tunnel_route = &world.features.protected_routes["grand_v3.tunnel"];
        let tunnel_connector = tunnel_route
            .surfaces
            .iter()
            .filter(|surface| crystal_mask.contains(&surface.coord))
            .copied()
            .collect::<BTreeSet<_>>();
        assert_eq!(lower_terminal.len(), 4);
        assert_eq!(upper_terminal.len(), 4);
        assert_eq!(tunnel_connector, *lower_terminal);
        assert_eq!(
            tunnel_route
                .centerline
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len(),
            tunnel_route.centerline.len()
        );
        assert!(tunnel_route
            .centerline
            .windows(2)
            .all(|pair| pair[0].coord.distance(pair[1].coord) == 1));
        let locked_outside_goal = TilePos::new(HexCoord::from_axial(22, -99), 6);
        let goal_index = tunnel_route
            .centerline
            .iter()
            .position(|position| *position == locked_outside_goal)
            .expect("outside connector reaches the locked first-outside center");
        assert!(goal_index > 2, "connector must leave the terminal vicinity");
        let connector_directions = tunnel_route.centerline[..=goal_index]
            .windows(2)
            .filter_map(|pair| {
                pair[0]
                    .coord
                    .neighbors()
                    .iter()
                    .position(|neighbor| *neighbor == pair[1].coord)
            })
            .collect::<BTreeSet<_>>();
        assert!(
            connector_directions.len() > 1,
            "connector must bend around the convex Crystal shell"
        );
        assert!(lower_terminal.contains(&lower_entry));
        assert!(upper_terminal.contains(&upper_exit));
        let frozen_exit = &world.features.protected_routes["grand_v3.frozen_exit"];
        assert_eq!(frozen_exit.surfaces.len(), 16);
        assert_eq!(frozen_exit.centerline.len(), 4);
        assert_eq!(frozen_exit.centerline.first(), Some(&upper_exit));
        assert_eq!(
            world.anchors["grand_v3.frozen_exit"],
            *frozen_exit
                .centerline
                .last()
                .expect("Frozen exit retains its exact final route surface"),
            "Frozen-exit review anchor must identify the route's final Frozen-Woods footing"
        );
        assert!(frozen_exit.centerline.windows(2).all(|pair| {
            pair[0].coord.distance(pair[1].coord) == 1 && pair[0].level.abs_diff(pair[1].level) <= 1
        }));
        assert_eq!(
            world.features.protected_routes["grand_v3.crystal_route"]
                .centerline
                .last(),
            frozen_exit.centerline.last(),
            "the canonical Crystal route must continue into Frozen Woods"
        );
        let frozen_mask = fixture
            .plan
            .cells
            .iter()
            .filter(|cell| has_overlay(cell, SchematicFeature::FrozenWoods))
            .filter_map(|cell| world.layout.patches.get(&PatchId(u32::from(cell.id.get()))))
            .flat_map(|patch| patch.mask.iter().copied())
            .collect::<BTreeSet<_>>();
        let outside_frozen_exit = frozen_exit
            .surfaces
            .iter()
            .filter(|surface| !crystal_mask.contains(&surface.coord))
            .map(|surface| surface.coord)
            .collect::<BTreeSet<_>>();
        assert_eq!(outside_frozen_exit.len(), 8);
        assert!(outside_frozen_exit.contains(
            &frozen_exit
                .centerline
                .last()
                .expect("Frozen exit has one final centerline surface")
                .coord
        ));
        assert!(outside_frozen_exit.is_subset(&frozen_mask));
        let crystal_rotation = world
            .layout
            .patches
            .values()
            .find(|patch| patch.mask == *crystal_mask)
            .map(|patch| patch.rotation_turns)
            .expect("Crystal owner retains its landmark rotation");
        let mantle_screen = super::super::schematic_highlands::crystal_mantle_inner_screen(
            crystal_mask,
            crystal_rotation,
            V3GrandV3BasicTerrainProfile::canonical(),
            &world.layout.footprint,
        )
        .expect("the exact inner mantle screen resolves");
        for name in [
            "grand_v3.natural_pass",
            "grand_v3.peak_saddle",
            "grand_v3.peak_foothill_ledge",
        ] {
            assert!(world.features.protected_routes[name]
                .surfaces
                .iter()
                .all(|surface| !mantle_screen.contains(&surface.coord)));
        }
        assert!(frozen_exit
            .surfaces
            .iter()
            .all(|surface| !mantle_screen.contains(&surface.coord)));
        let stacked_lower_tunnel = tunnel_route
            .surfaces
            .iter()
            .filter(|surface| mantle_screen.contains(&surface.coord))
            .copied()
            .collect::<BTreeSet<_>>();
        assert!(
            !stacked_lower_tunnel.is_empty(),
            "the usable lower tunnel must retain its exact floor beneath the mantle screen"
        );
        assert!(stacked_lower_tunnel.iter().all(|floor| {
            world.volume.surfaces.contains_key(floor)
                && world
                    .volume
                    .top_surface_at_coord(floor.coord)
                    .is_some_and(|(cap, _)| {
                        cap.level > super::super::schematic_highlands::CRYSTAL_ARCHITECTURE_TOP
                    })
        }));
        assert!(world.features.protected_routes["grand_v3.ordinary_hubs"]
            .surfaces
            .iter()
            .filter(|surface| mantle_screen.contains(&surface.coord))
            .all(|surface| {
                world
                    .volume
                    .top_surface_at_coord(surface.coord)
                    .is_some_and(|(cap, _)| {
                        cap.level > super::super::schematic_highlands::CRYSTAL_ARCHITECTURE_TOP
                    })
            }));
        let unified_id = world
            .volume
            .surfaces
            .get(&lower_entry)
            .and_then(|metadata| metadata.interior)
            .expect("composite lower entry belongs to the unified Dark domain");
        assert_eq!(world.interiors.by_id.len(), 1);
        let unified = &world.interiors.by_id[&unified_id];
        assert!(lower_terminal.iter().all(|surface| {
            world
                .volume
                .surfaces
                .get(surface)
                .is_some_and(|metadata| metadata.interior == Some(unified_id))
                && unified.floors.contains(surface)
                && !unified.entrances.contains(surface)
        }));
        assert!(upper_terminal.iter().all(|surface| {
            world
                .volume
                .surfaces
                .get(surface)
                .is_some_and(|metadata| metadata.interior == Some(unified_id))
                && unified.floors.contains(surface)
                && unified.entrances.contains(surface)
        }));
        assert_eq!(
            world.volume.surfaces[&upper_exit].interior,
            Some(unified_id)
        );
        let foot_threshold = unified
            .entrances
            .difference(upper_terminal)
            .copied()
            .collect::<BTreeSet<_>>();
        assert_eq!(foot_threshold.len(), 4);
        assert!(foot_threshold.iter().all(|surface| {
            surface.level == 6
                && !crystal_mask.contains(&surface.coord)
                && world
                    .volume
                    .surfaces
                    .get(surface)
                    .is_some_and(|metadata| metadata.interior == Some(unified_id))
        }));
        assert_eq!(unified.entrances.len(), 8);
        let valley_center = fixture
            .plan
            .cells
            .iter()
            .find(|cell| has_overlay(cell, SchematicFeature::ValleyLake))
            .map(|cell| schematic_to_world(cell.coord, 22))
            .expect("reference has a valley lake");
        let valley_anchor = world.anchors["grand_v3.valley_lake"];
        assert!(valley_anchor.coord.distance(valley_center) <= 22);
        assert_eq!(
            world.volume.surfaces[&valley_anchor].access,
            SurfaceAccess::Ordinary
        );
        let waterfall_profile = world.anchors["grand_v3.waterfall_profile"];
        assert_eq!(
            world.volume.surfaces[&waterfall_profile].access,
            SurfaceAccess::Ordinary,
            "the stable waterfall profile review anchor must be standable"
        );
        for name in [
            "grand_v3.crystal_mantle_overlook",
            "grand_v3.river_bend",
            "grand_v3.treeline_transition",
            "grand_v3.peak_ridge_overlook",
        ] {
            let anchor = world.anchors[name];
            assert_eq!(
                world.volume.surfaces[&anchor].access,
                SurfaceAccess::Ordinary,
                "corrective shipped-camera anchor {name} must remain ordinary"
            );
        }
        let mantle_overlook = world.anchors["grand_v3.crystal_mantle_overlook"];
        let crystal_center = fixture
            .plan
            .cells
            .iter()
            .find(|cell| has_overlay(cell, SchematicFeature::CrystalAscent))
            .map(|cell| schematic_to_world(cell.coord, 22))
            .expect("reference retains one Crystal schematic centre");
        assert!(
            mantle_overlook.coord.distance(lower_entry.coord)
                < mantle_overlook.coord.distance(crystal_center)
        );
        assert!(mantle_screen_blocks_review_line(
            world,
            &mantle_screen,
            mantle_overlook.coord,
            crystal_center,
        ));

        assert_eq!(
            world.anchors["grand_v3.peak_ridge_overlook"],
            world.anchors["grand_v3.peak_foothill_ledge"],
            "the peak review must remain the authored ledge rather than a generic relocation"
        );

        let treeline = world.anchors["grand_v3.treeline_transition"];
        assert_eq!(
            solid_material_at(&world.volume, treeline),
            Some(SolidMaterialRole::Snow)
        );
        let fine_index = FineWorldIndex::from_layout(&world.layout)
            .expect("reference retains exact final fine ownership");
        let treeline_index = TreelineReviewIndex::build(&fixture.plan, &fine_index, world);
        let witnesses = treeline_index
            .witnesses(treeline)
            .expect("treeline retains its actual downhill-tree and higher-snow witnesses");
        assert!(witnesses.downhill_tree.level < treeline.level);
        assert!(witnesses.uphill_snow.level > treeline.level);
        assert!(review_vectors_face_opposite(
            treeline.coord,
            witnesses.downhill_tree.coord,
            witnesses.uphill_snow.coord,
        ));
        let garden_anchor = world.observation_anchors["grand_v3.lake_island"];
        assert!(!world.anchors.contains_key("grand_v3.lake_island"));
        let garden_patch = fixture
            .plan
            .cells
            .iter()
            .find(|cell| has_overlay(cell, SchematicFeature::LakeIsland))
            .and_then(|cell| world.layout.patches.get(&PatchId(u32::from(cell.id.get()))))
            .expect("Garden island keeps one resolved patch");
        assert!(garden_patch.mask.contains(&garden_anchor.coord));
        assert!(world.volume.surfaces.contains_key(&garden_anchor));

        let massif_crest = world.observation_anchors["grand_v3.massif_crest"];
        assert!(!world.anchors.contains_key("grand_v3.massif_crest"));
        let highest_peak = fixture
            .plan
            .cells
            .iter()
            .filter(|cell| cell.facts.landform == LandformKind::SharpPeak)
            .filter_map(|cell| world.layout.patches.get(&PatchId(u32::from(cell.id.get()))))
            .flat_map(|patch| patch.mask.iter().copied())
            .filter_map(|coord| {
                world
                    .volume
                    .top_surface_at_coord(coord)
                    .map(|(surface, _)| surface.level)
            })
            .max()
            .expect("locked peak ring has rendered surfaces");
        assert!(world.volume.surfaces.contains_key(&massif_crest));
        assert!(massif_crest.level > highest_peak);
        assert!(massif_crest.level > upper_exit.level);

        let bridges = world
            .structures
            .by_id
            .iter()
            .filter(|(_, structure)| structure.kind == StructureKind::Bridge)
            .collect::<Vec<_>>();
        assert_eq!(bridges.len(), 2);
        assert!(bridges
            .iter()
            .all(|(id, bridge)| { id.0 >> 24 == 255 && bridge.voxels.len() == 10 }));

        let tunnel_bright = world
            .lights
            .values()
            .filter_map(|light| {
                matches!(
                    light.presentation,
                    Some(PlannedLightPresentation::CaveCrystal(_))
                )
                .then_some(light)
            })
            .collect::<Vec<_>>();
        let tunnel_dim = world
            .lights
            .values()
            .filter(|light| {
                light.presentation.is_none()
                    && light.level == IlluminationLevel::Dim
                    && light.radius == 18
                    && light.origin.level == 6
            })
            .collect::<Vec<_>>();
        assert!(!tunnel_bright.is_empty());
        assert_eq!(tunnel_bright.len(), tunnel_dim.len());
        assert!(tunnel_bright.iter().all(|light| {
            light.level == IlluminationLevel::Bright
                && light.radius == 4
                && tunnel_dim.iter().any(|dim| dim.origin == light.origin)
        }));

        let hub_route = &world.features.protected_routes["grand_v3.ordinary_hubs"];
        let ordinary_cells = fixture
            .plan
            .cells
            .iter()
            .filter(|cell| is_ordinary_land(cell))
            .collect::<Vec<_>>();
        assert_eq!(
            fixture.selection.metrics.ordinary_graph_full_rebuilds, 1,
            "ordinary construction seeds its cache with exactly one full graph projection"
        );
        assert!(
            fixture.selection.metrics.ordinary_graph_local_repairs > 0,
            "the reference fixture exercises local graph repair"
        );
        assert!(
            usize::try_from(fixture.selection.metrics.ordinary_graph_local_repairs)
                .is_ok_and(|repairs| repairs <= ordinary_cells.len().saturating_add(2)),
            "at most one local repair is allowed per cell plus the two required pass ends"
        );
        assert_eq!(hub_route.centerline.len(), ordinary_cells.len());
        assert!(hub_route.centerline.iter().all(|hub| {
            world
                .volume
                .surfaces
                .get(hub)
                .is_some_and(|metadata| metadata.access == SurfaceAccess::Ordinary)
                && !world.blockers.contains(hub)
        }));
        let ordinary_graph = OrdinaryGraph::from_volume(&world.volume, Some(&world.blockers));
        let foothill = world.anchors["grand_v3.tunnel_mouth"];
        let reachable = ordinary_graph.distances_from(foothill);
        for (cell, hub) in ordinary_cells
            .iter()
            .copied()
            .zip(hub_route.centerline.iter().copied())
        {
            let patch = &world.layout.patches[&PatchId(u32::from(cell.id.get()))];
            assert!(patch.mask.contains(&hub.coord));
            assert!(reachable.contains_key(&hub));
        }
        let peak_19 = ordinary_cells
            .iter()
            .position(|cell| cell.id.get() == 19)
            .expect("the locked first sharp-peak cell remains Ordinary");
        let peak_19_hub = hub_route.centerline[peak_19];
        assert!(reachable.contains_key(&peak_19_hub));
        assert_eq!(
            fixture.selection.metrics.reachable_surfaces,
            u32::try_from(reachable.len()).unwrap_or(u32::MAX)
        );
        assert_eq!(
            fixture.selection.metrics.reachable_elevation_levels,
            u32::try_from(
                reachable
                    .keys()
                    .map(|position| position.level)
                    .collect::<BTreeSet<_>>()
                    .len()
            )
            .unwrap_or(u32::MAX)
        );

        let reserved = world
            .features
            .protected_routes
            .values()
            .flat_map(|route| route.surfaces.iter().map(|surface| surface.coord))
            .chain(
                world
                    .structures
                    .by_id
                    .values()
                    .flat_map(|structure| structure.voxels.iter().map(|voxel| voxel.coord)),
            )
            .collect::<BTreeSet<_>>();
        let vegetation = world
            .features
            .by_id
            .iter()
            .filter(|(id, feature)| {
                id.0 >= VEGETATION_FEATURE_BASE
                    && matches!(feature.kind, FeatureKind::Tree | FeatureKind::TallGrass)
            })
            .collect::<Vec<_>>();
        assert!(!vegetation.is_empty());
        assert!(vegetation.iter().all(|(id, feature)| {
            id.0 >> 24 == 255
                && !reserved.contains(&feature.root.coord)
                && world.volume.surfaces.contains_key(&feature.root)
        }));
        // Frozen Woods is one locked four-cell formation. Cell 123 is the
        // Crystal-adjacent donor whose inner portion is intentionally claimed
        // by the authored ascent and cleared for its exact summit exit, so it
        // is not independently required to retain a tree root. The vegetation
        // contract belongs to the complete locked formation; the exhaustive
        // test below still proves exact per-cell canopy targets wherever a
        // post-reservation root remains eligible.
        let locked_frozen_woods = fixture
            .plan
            .cells
            .iter()
            .filter(|cell| has_overlay(cell, SchematicFeature::FrozenWoods))
            .flat_map(|cell| {
                world.layout.patches[&PatchId(u32::from(cell.id.get()))]
                    .mask
                    .iter()
                    .copied()
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            fixture
                .plan
                .cells
                .iter()
                .filter(|cell| has_overlay(cell, SchematicFeature::FrozenWoods))
                .count(),
            4,
            "the locked Frozen Woods formation must retain all four schematic cells"
        );
        assert!(vegetation.iter().any(|(_, feature)| {
            feature.kind == FeatureKind::Tree && locked_frozen_woods.contains(&feature.root.coord)
        }));
    }

    #[test]
    fn final_schematic_vegetation_meets_density_and_full_clearance_contract() {
        let fixture = reference_fixture();
        let world = &fixture.selection.validated.plan;
        let catalog = super::super::vegetation::tests::runtime_art_catalog();
        let temperate = TemperateVegetationSet::resolve(catalog, "Grand vegetation test")
            .expect("temperate vegetation resolves");
        let frozen = SnowyVegetationSet::resolve(catalog, "Grand vegetation test")
            .expect("frozen vegetation resolves");
        let lower_entry = world.anchors["crystal_ascent.lower_entry"];
        let crystal_mask = &world
            .layout
            .patches
            .values()
            .find(|patch| patch.mask.contains(&lower_entry.coord))
            .expect("Crystal owner remains present")
            .mask;
        let supports = world
            .volume
            .surfaces
            .iter()
            .filter(|(_, metadata)| metadata.access != SurfaceAccess::NonStandable)
            .fold(
                BTreeMap::<HexCoord, TilePos>::new(),
                |mut result, (position, _)| {
                    result
                        .entry(position.coord)
                        .and_modify(|current| *current = (*current).max(*position))
                        .or_insert(*position);
                    result
                },
            );
        let schematic_features = world
            .features
            .by_id
            .iter()
            .filter(|(id, feature)| {
                (VEGETATION_FEATURE_BASE..TUNNEL_LIGHT_BASE).contains(&id.0)
                    && matches!(feature.kind, FeatureKind::Tree | FeatureKind::TallGrass)
            })
            .map(|(id, feature)| (*id, feature))
            .collect::<BTreeMap<_, _>>();
        assert!(!schematic_features.is_empty());
        let schematic_blockers = schematic_features
            .values()
            .flat_map(|feature| feature.blocker_footprint.iter().copied())
            .collect::<BTreeSet<_>>();
        let preexisting_blockers = world
            .blockers
            .difference(&schematic_blockers)
            .copied()
            .collect::<BTreeSet<_>>();
        let reserved = schematic_vegetation_reserved(world, crystal_mask, &preexisting_blockers);
        assert!(!world.volume.fill_runs_by_top().is_empty());
        assert!(!world.structures.by_id.is_empty());
        assert!(!world.features.protected_routes.is_empty());
        assert!(!world.features.clearings.is_empty());
        assert!(!world.anchors.is_empty());
        assert!(!world.lights.is_empty());
        let mut expected_reserved = crystal_mask.clone();
        expected_reserved.extend(
            world
                .volume
                .fill_runs_by_top()
                .keys()
                .map(|position| position.coord),
        );
        expected_reserved.extend(
            world
                .structures
                .by_id
                .values()
                .flat_map(|structure| structure.voxels.iter().map(|voxel| voxel.coord)),
        );
        expected_reserved.extend(preexisting_blockers.iter().map(|blocker| blocker.coord));
        expected_reserved.extend(
            world
                .features
                .protected_routes
                .values()
                .flat_map(|route| &route.surfaces)
                .flat_map(|surface| surface.coord.within_radius(2)),
        );
        expected_reserved.extend(
            world
                .features
                .clearings
                .values()
                .flat_map(|clearing| clearing.surfaces.iter().map(|surface| surface.coord)),
        );
        expected_reserved.extend(
            world
                .anchors
                .values()
                .flat_map(|anchor| anchor.coord.within_radius(3)),
        );
        expected_reserved.extend(
            world
                .lights
                .values()
                .flat_map(|light| light.origin.coord.within_radius(2)),
        );
        assert_eq!(
            reserved, expected_reserved,
            "Grand vegetation exclusion authority must remain complete and exact"
        );
        let mut occupied_visual = BTreeSet::new();
        let mut occupied_blockers = preexisting_blockers;

        for cell in &fixture.plan.cells {
            let ecology = schematic_ecology::vegetation_policy(cell);
            let patch = world
                .layout
                .patches
                .get(&PatchId(u32::from(cell.id.get())))
                .expect("every schematic cell retains one resolved patch");
            let actual = schematic_features
                .iter()
                .filter(|(_, feature)| patch.mask.contains(&feature.root.coord))
                .map(|(id, feature)| (*id, *feature))
                .collect::<Vec<_>>();
            if cell.facts.surface != SurfaceKind::Land
                || ecology.density == VegetationDensity::None
                || has_overlay(cell, SchematicFeature::CrystalAscent)
            {
                assert!(
                    actual.is_empty(),
                    "cell {} cannot receive schematic vegetation",
                    cell.id.get()
                );
                continue;
            }

            let candidate_roots = patch
                .mask
                .iter()
                .copied()
                .filter(|coord| {
                    supports.get(coord).is_some_and(|root| {
                        !reserved.contains(coord)
                            && schematic_ecology::tree_root_is_admitted(
                                cell,
                                *root,
                                fixture.plan.provenance.world_seed,
                            )
                    })
                })
                .collect::<BTreeSet<_>>();
            let (trees, eligibility, grass): (
                Vec<&VegetationObjectSpec>,
                Vec<&VegetationObjectSpec>,
                &VegetationObjectSpec,
            ) = if ecology.family == VegetationFamily::Frozen {
                (
                    vec![
                        &frozen.old_growth,
                        &frozen.small_broadleaf,
                        &frozen.tall_narrow,
                    ],
                    vec![
                        &frozen.small_broadleaf,
                        &frozen.tall_narrow,
                        &frozen.old_growth,
                    ],
                    &frozen.grass_tuft,
                )
            } else {
                (
                    vec![
                        &temperate.old_growth,
                        &temperate.small_broadleaf,
                        &temperate.tall_narrow,
                    ],
                    vec![
                        &temperate.small_broadleaf,
                        &temperate.tall_narrow,
                        &temperate.old_growth,
                    ],
                    &temperate.grass_tuft,
                )
            };
            let eligible = exact_eligible_tree_roots(
                &candidate_roots,
                &supports,
                &eligibility,
                &reserved,
                &occupied_visual,
                &occupied_blockers,
            );
            let target = vegetation_canopy_target(eligible.len(), ecology.density);
            if target == 0 {
                assert!(
                    actual.is_empty(),
                    "cell {} has no eligible canopy target but retained vegetation",
                    cell.id.get()
                );
                continue;
            }
            let coherent = coherent_vegetation_roots(
                fixture.plan.provenance.world_seed,
                cell,
                ecology.density,
                &eligible,
            );
            assert_eq!(
                coherent,
                coherent_vegetation_roots(
                    fixture.plan.provenance.world_seed,
                    cell,
                    ecology.density,
                    &eligible,
                ),
                "cell {} clustered root order must be deterministic",
                cell.id.get()
            );
            let coherent_indices = coherent
                .iter()
                .enumerate()
                .map(|(index, coord)| (*coord, index))
                .collect::<BTreeMap<_, _>>();
            let mut last_tree_index = None;
            let mut covered = BTreeSet::new();
            for (id, feature) in actual {
                assert!(
                    !reserved.contains(&feature.root.coord),
                    "vegetation {id:?} roots inside an authored exclusion"
                );
                let object = match feature.kind {
                    FeatureKind::Tree => trees
                        .iter()
                        .copied()
                        .find(|object| object.id == feature.object_id)
                        .expect("tree uses the resolved climate asset"),
                    FeatureKind::TallGrass => {
                        assert_eq!(feature.object_id, grass.id);
                        grass
                    }
                    FeatureKind::CaveVegetation => {
                        panic!("Grand surface vegetation cannot use a cave object")
                    }
                };
                let visual = object
                    .project_visual_volume(feature.root, feature.rotation)
                    .expect("accepted vegetation retains an exact visual projection");
                let reserved_visual = visual
                    .cells
                    .iter()
                    .filter(|voxel| reserved.contains(&voxel.coord))
                    .copied()
                    .collect::<Vec<_>>();
                assert!(
                    reserved_visual.is_empty(),
                    "vegetation {id:?} at {:?} visual volume enters water, route, bridge, anchor, Crystal, or clearing authority at {reserved_visual:?}",
                    feature.root,
                );
                assert!(feature
                    .blocker_footprint
                    .iter()
                    .all(|blocker| !reserved.contains(&blocker.coord)));
                assert!(visual.cells.is_disjoint(&occupied_visual));
                if feature.kind == FeatureKind::Tree {
                    let index = coherent_indices
                        .get(&feature.root.coord)
                        .copied()
                        .expect("accepted tree root belongs to the eligible clustered order");
                    assert!(last_tree_index.is_none_or(|previous| previous < index));
                    last_tree_index = Some(index);
                    covered.extend(
                        visual
                            .cells
                            .iter()
                            .map(|voxel| voxel.coord)
                            .filter(|coord| eligible.contains(coord)),
                    );
                }
                occupied_visual.extend(visual.cells);
                occupied_blockers.extend(feature.blocker_footprint.iter().copied());
            }
            assert!(
                covered.len() >= target,
                "cell {} realizes only {}/{} canopy columns for {:?}",
                cell.id.get(),
                covered.len(),
                target,
                ecology.density
            );
        }
    }

    #[test]
    fn exact_vegetation_density_targets_cover_all_five_schematic_bands() {
        for (density, expected_percent) in [
            (VegetationDensity::None, 0),
            (VegetationDensity::Sparse, 4),
            (VegetationDensity::Light, 14),
            (VegetationDensity::Moderate, 30),
            (VegetationDensity::Dense, 52),
        ] {
            assert_eq!(vegetation_coverage_percent(density), expected_percent);
            assert_eq!(vegetation_canopy_target(0, density), 0);
            assert_eq!(vegetation_canopy_target(100, density), expected_percent);
        }
    }

    #[test]
    fn schematic_view_hint_fits_the_complete_radius_187_footprint() {
        let footprint = HexCoord::ORIGIN
            .within_radius(V3_SCHEMATIC_GRID_RADIUS)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let hint = schematic_view_hint(
            &footprint,
            0.4,
            super::super::schematic_highlands::MASSIF_SUMMIT_MAX,
        );
        assert!(hint.is_valid());
        let distance = ((hint.eye.0 - hint.focus.0).powi(2)
            + (hint.eye.1 - hint.focus.1).powi(2)
            + (hint.eye.2 - hint.focus.2).powi(2))
        .sqrt();
        let vertical_half_extent = distance * 20.0_f32.to_radians().tan();
        for coord in footprint {
            let center = coord.to_world(0.0);
            assert!(
                (center.z - hint.focus.2).abs() + 1.0 < vertical_half_extent,
                "top-down frame crops {coord:?} vertically"
            );
            assert!(
                (center.x - hint.focus.0).abs() + 1.0 < vertical_half_extent * (16.0 / 9.0),
                "top-down frame crops {coord:?} horizontally"
            );
        }
    }

    #[test]
    fn radius_187_proxy_has_the_exact_footprint_and_chunk_occupancy() {
        let selection = &reference_fixture().selection;
        let layout = &selection.validated.plan.layout;
        let expected_footprint = HexCoord::ORIGIN
            .within_radius(V3_SCHEMATIC_GRID_RADIUS)
            .into_iter()
            .collect::<BTreeSet<_>>();
        assert_eq!(layout.footprint, expected_footprint);

        let mut chunk_occupancy = BTreeMap::<(i32, i32), usize>::new();
        for coord in &layout.footprint {
            let chunk = (
                coord.x().div_euclid(TERRAIN_CHUNK_SIDE),
                coord.y().div_euclid(TERRAIN_CHUNK_SIDE),
            );
            *chunk_occupancy.entry(chunk).or_default() += 1;
        }
        assert_eq!(chunk_occupancy.len(), 444);
        assert_eq!(chunk_occupancy.values().copied().sum::<usize>(), 105_469);
        assert_eq!(chunk_occupancy.values().copied().max(), Some(256));
        assert_eq!(
            u32::try_from(chunk_occupancy.len()).unwrap_or(u32::MAX),
            selection.metrics.expected_chunks
        );
    }

    #[test]
    fn reference_proxy_pins_water_and_high_landmark_elevations() {
        let world = &reference_fixture().selection.validated.plan;
        let liquid_positions = world
            .liquids
            .bodies
            .values()
            .flat_map(|body| body.nodes.keys().copied())
            .collect::<BTreeSet<_>>();

        let mountain_lake = world_coord(4, -4);
        assert!(liquid_positions.contains(&TilePos::new(mountain_lake, 150)));
        assert!((144..=149).contains(&surface_level_at(world, mountain_lake)));
        assert!(liquid_positions.contains(&TilePos::new(world_coord(4, -1), 15)));

        assert_eq!(
            surface_level_at(world, world_coord(1, -6)),
            6,
            "the corrected revision-2 trace places Crystal Ascent at this coarse center"
        );
        // The radius-32 Crystal claim deliberately borrows the centre of the
        // adjacent `(2, -6)` Frozen-Woods cell. It is a real stacked authored
        // column: bottom chamber floor, stair circuit, then the snowy exterior
        // cap. Do not collapse it to one surface merely to probe its elevation.
        let borrowed_frozen = world_coord(2, -6);
        let borrowed_surfaces = world
            .volume
            .surfaces_at_coord(borrowed_frozen)
            .map(|(position, metadata)| (*position, *metadata))
            .collect::<Vec<_>>();
        assert_eq!(
            borrowed_surfaces
                .iter()
                .map(|(position, _)| position.level)
                .collect::<Vec<_>>(),
            vec![6, 82, 150]
        );
        let [(_, floor_metadata), (_, circuit_metadata), (_, cap_metadata)] =
            borrowed_surfaces.as_slice()
        else {
            panic!("borrowed Frozen column must retain its exact three-surface stack");
        };
        let borrowed_interior = floor_metadata
            .interior
            .expect("borrowed Frozen column keeps the Crystal chamber floor");
        assert_eq!(circuit_metadata.interior, Some(borrowed_interior));
        assert_eq!(cap_metadata.interior, None);
        assert!(world.structures.by_id.values().any(|structure| {
            structure
                .voxels
                .contains(&TilePos::new(borrowed_frozen, 82))
        }));
        assert_eq!(
            world
                .volume
                .top_surface_at_coord(borrowed_frozen)
                .expect("borrowed Frozen column keeps its snowy exterior cap")
                .0
                .level,
            150
        );

        // These coarse centres remain outside the radius-32 Crystal claim and
        // therefore retain one natural Frozen-Woods surface each.
        for frozen_woods in [(3, -6), (3, -7)] {
            assert!((152..=176).contains(&surface_level_at(
                world,
                world_coord(frozen_woods.0, frozen_woods.1),
            )));
        }
        assert!((151..=158).contains(&surface_level_at(world, world_coord(4, -5))));
        assert!((200..=218).contains(&surface_level_at(world, world_coord(3, -3))));
    }

    #[test]
    fn ordinary_unlocked_patch_boundaries_have_no_coarse_height_step() {
        const MAX_ADJACENT_DELTA: u32 = 6;

        let fixture = reference_fixture();
        let world = &fixture.selection.validated.plan;
        // Locked landmarks and overlay-bearing cells intentionally introduce cliffs,
        // shores, water, or structure boundaries and do not belong to this smoothness
        // contract. Ordinary overlay-free land must remain globally blended.
        let smooth_patches = fixture
            .plan
            .cells
            .iter()
            .filter(|cell| {
                cell.facts.surface == SurfaceKind::Land
                    && cell.facts.access == AccessIntent::Ordinary
                    && cell.facts.overlays.is_empty()
                    && !matches!(&cell.provenance.surface, LayerProvenance::Locked { .. })
            })
            .map(|cell| PatchId(u32::from(cell.id.get())))
            .collect::<BTreeSet<_>>();
        let owner_by_coord = world
            .layout
            .patches
            .iter()
            .flat_map(|(owner, patch)| patch.mask.iter().map(move |coord| (*coord, *owner)))
            .collect::<BTreeMap<_, _>>();
        let levels = world
            .volume
            .surfaces
            .keys()
            .map(|position| (position.coord, position.level))
            .collect::<BTreeMap<_, _>>();
        let authored_route_coords = world
            .features
            .protected_routes
            .values()
            .flat_map(|route| route.surfaces.iter().map(|position| position.coord))
            .collect::<BTreeSet<_>>();

        let mut candidate_pairs = 0_usize;
        let mut checked_pairs = 0_usize;
        let mut worst: Option<(u32, HexCoord, HexCoord, PatchId, PatchId)> = None;
        for (coord, owner) in &owner_by_coord {
            if !smooth_patches.contains(owner) {
                continue;
            }
            for neighbor in coord.neighbors() {
                let Some(neighbor_owner) = owner_by_coord.get(&neighbor) else {
                    continue;
                };
                if *coord >= neighbor
                    || owner == neighbor_owner
                    || !smooth_patches.contains(neighbor_owner)
                {
                    continue;
                }
                candidate_pairs = candidate_pairs.saturating_add(1);
                if authored_route_coords.contains(coord)
                    || authored_route_coords.contains(&neighbor)
                {
                    continue;
                }
                let delta = levels[coord].abs_diff(levels[&neighbor]);
                checked_pairs = checked_pairs.saturating_add(1);
                if worst.is_none_or(|(current, ..)| delta > current) {
                    worst = Some((delta, *coord, neighbor, *owner, *neighbor_owner));
                }
            }
        }

        let scale_floor = world.layout.footprint.len() / 16;
        assert!(
            candidate_pairs >= scale_floor,
            "only {candidate_pairs} ordinary unlocked cross-owner pairs remain for {} world columns",
            world.layout.footprint.len()
        );
        assert!(
            checked_pairs.saturating_mul(4) >= candidate_pairs.saturating_mul(3),
            "authored routes excluded too much seam evidence: checked {checked_pairs}/{candidate_pairs} cross-owner pairs"
        );
        let (delta, first, second, first_owner, second_owner) =
            worst.expect("ordinary unlocked patches must share boundaries");
        assert!(
            delta <= MAX_ADJACENT_DELTA,
            "coarse ownership seam {first:?} ({first_owner:?}) -> {second:?} \
             ({second_owner:?}) jumps {delta} levels; maximum is {MAX_ADJACENT_DELTA}"
        );
    }
}
