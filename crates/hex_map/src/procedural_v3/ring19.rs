//! Whole-world selection and validation for the fixed V3 nineteen-region layout.
//!
//! Ring19 owns one candidate loop for the complete radius-two macro world. Every
//! patch receives the same candidate index, is validated against the resolved
//! layout, and may enter scoring only through checked whole-world composition.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_assets::RuntimeArtCatalog;
use hex_core::config::HEX_CIRCUMRADIUS;
use hex_core::{BiomeRegionId, HexCoord, Level, MapViewHint, TilePos};

use super::composite_patch;
use super::composition::{
    compose_world, GeneratedPatchPlan, PatchAnchorRef, WorldCompositionSettings,
};
use super::layout::{
    resolve_layout, HexSide, LayoutKind, PatchId, ResolvedEdgeId, ResolvedEdgeReference,
    ResolvedLayoutPlan, ResolvedLiquidElevation, ResolvedLiquidPort,
};
use super::liquid::{LiquidBodyId, LiquidFlowState, LiquidNode};
use super::patch::{PatchBuildMode, PatchRecipeContext};
use super::selection::{
    run_recipe, CandidateAttemptError, CandidateContext, FallbackContext, RepairOutcome, V3Recipe,
    ValidatedWorldSelection, WorldValidation,
};
use super::traversal::OrdinaryGraph;
use super::vegetation::CaveVegetationSet;
use super::volume::{FillMaterialRole, LevelInterval, VolumeElement};
use super::world::{GeneratedWorldPlan, WorldIssueCode, WorldValidationIssue};
use super::V3GenerationError;
use crate::procedural::Ring19Metrics as Ring19ReportMetrics;
use crate::settings::{
    ProceduralV3Settings, Ring19RegionSettings, V3EnvironmentSettings, V3LayoutSettings,
    V3RecipeSettings, V3Ring19ProfileSettings, V3Ring19Settings, V3_RING19_REGION_COUNT,
};

const RING_RADIUS: u32 = 55;
const WORLD_COLUMNS: u32 = 9_241;
const PATCH_COUNT: u32 = 19;
const RECIPROCAL_SEAMS: u32 = 42;
const BOUNDARY_SIDES: u32 = 30;
const INTERNAL_LIQUID_SEAMS: u32 = 8;
const BOUNDARY_LIQUID_OUTLETS: u32 = 2;
const SEAM_PORT_WIDTH: u32 = 3;
const WALKER_PORT_COUNT: u8 = 2;
const WALKER_PORT_WIDTH: u32 = 2;
const SEAM_APPROACH_DEPTH: u32 = 3;
const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";
const CONFLICT_CENTER: &str = "conflict_center";
const MOUNTAIN_WATERFALL_OVERLOOK: &str = "mountain_waterfall_overlook";
const CONFLUENCE_OVERLOOK: &str = "confluence_overlook";
const VEGETATION_GRADIENT_OVERLOOK: &str = "vegetation_gradient_overlook";
const FORT_OUTLET_OVERLOOK: &str = "fort_outlet_overlook";
const OASIS_OVERLOOK: &str = "oasis_overlook";
const INNER_DUNE_CREST: &str = "inner_dune_crest";
const OUTER_DUNE_CREST: &str = "outer_dune_crest";
const DESERT_PLAIN_OVERLOOK: &str = "desert_plain_overlook";
const REVIEW_ANCHORS: [&str; 4] = [
    MOUNTAIN_WATERFALL_OVERLOOK,
    CONFLUENCE_OVERLOOK,
    VEGETATION_GRADIENT_OVERLOOK,
    FORT_OUTLET_OVERLOOK,
];
const DESERT_REVIEW_ANCHORS: [&str; 4] = [
    OASIS_OVERLOOK,
    INNER_DUNE_CREST,
    OUTER_DUNE_CREST,
    DESERT_PLAIN_OVERLOOK,
];
const LEGACY_RING19_VIEW_FRAME: f32 = 196.0;
const RING19_VIEW_EYE_UP: f32 = 0.72;
const RING19_VIEW_EYE_BACK: f32 = 0.82;
const RING19_VIEW_OFFSET_LENGTH: f32 = 1.091_237_8;
// The review runner uses Camera3d's 45-degree vertical field of view at 1920x1080.
// Keeping a six-percent screen-edge margin makes overview captures robust to the
// elevated terrain on Ring19's outer boundary.
const RING19_REVIEW_TAN_HALF_VERTICAL_FOV: f32 = 0.414_213_57;
const RING19_REVIEW_ASPECT_RATIO: f32 = 16.0 / 9.0;
const RING19_REVIEW_NDC_LIMIT: f32 = 0.88;
const SQRT_THREE_OVER_TWO: f32 = 0.866_025_4;
const HEX_CORNER_OFFSETS: [(f32, f32); 6] = [
    (0.0, 1.0),
    (SQRT_THREE_OVER_TWO, 0.5),
    (SQRT_THREE_OVER_TWO, -0.5),
    (0.0, -1.0),
    (-SQRT_THREE_OVER_TWO, -0.5),
    (-SQRT_THREE_OVER_TWO, 0.5),
];
const RING19_REVIEW_CAMERA_BASES: [ReviewCameraBasis; 4] = [
    ReviewCameraBasis::new(
        ReviewPoint::new(0.0, -0.659_801_2, -0.751_440_2),
        ReviewPoint::new(1.0, 0.0, 0.0),
        ReviewPoint::new(0.0, 0.751_440_2, -0.659_801_2),
    ),
    ReviewCameraBasis::new(
        ReviewPoint::new(-0.650_766_3, -0.659_801_2, 0.375_720_1),
        ReviewPoint::new(-0.5, 0.0, -SQRT_THREE_OVER_TWO),
        ReviewPoint::new(-0.571_404_6, 0.751_440_2, 0.329_900_6),
    ),
    ReviewCameraBasis::new(
        ReviewPoint::new(0.650_766_3, -0.659_801_2, 0.375_720_1),
        ReviewPoint::new(-0.5, 0.0, SQRT_THREE_OVER_TWO),
        ReviewPoint::new(0.571_404_6, 0.751_440_2, 0.329_900_6),
    ),
    ReviewCameraBasis::new(
        ReviewPoint::new(0.0, -1.0, 0.0),
        ReviewPoint::new(1.0, 0.0, 0.0),
        ReviewPoint::new(0.0, 0.0, -1.0),
    ),
];

#[derive(Debug, Clone, Copy)]
struct ReviewPoint {
    x: f32,
    y: f32,
    z: f32,
}

impl ReviewPoint {
    const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    fn relative_to(self, origin: Self) -> Self {
        Self::new(self.x - origin.x, self.y - origin.y, self.z - origin.z)
    }

    fn dot(self, other: Self) -> f32 {
        self.x
            .mul_add(other.x, self.y.mul_add(other.y, self.z * other.z))
    }
}

#[derive(Debug, Clone, Copy)]
struct ReviewCameraBasis {
    forward: ReviewPoint,
    right: ReviewPoint,
    up: ReviewPoint,
}

impl ReviewCameraBasis {
    const fn new(forward: ReviewPoint, right: ReviewPoint, up: ReviewPoint) -> Self {
        Self { forward, right, up }
    }
}

const FIXED_REGIONS: [FixedRegion; V3_RING19_REGION_COUNT] = [
    FixedRegion::new(
        "Hills confluence",
        V3EnvironmentSettings::TemperateGrassland,
        "Hills",
        0,
    ),
    FixedRegion::new("Frozen Hills", V3EnvironmentSettings::Frozen, "Hills", 0),
    FixedRegion::new(
        "Forest A",
        V3EnvironmentSettings::TemperateGrassland,
        "Forest",
        4,
    ),
    FixedRegion::new(
        "Prairie A",
        V3EnvironmentSettings::TemperateGrassland,
        "Prairie",
        0,
    ),
    FixedRegion::new(
        "downstream Hills",
        V3EnvironmentSettings::TemperateGrassland,
        "Hills",
        0,
    ),
    FixedRegion::new(
        "Waterfall B",
        V3EnvironmentSettings::TemperateGrassland,
        "Waterfall",
        0,
    ),
    FixedRegion::new(
        "Waterfall A",
        V3EnvironmentSettings::TemperateGrassland,
        "Waterfall",
        5,
    ),
    FixedRegion::new(
        "Sky Islands",
        V3EnvironmentSettings::TemperateGrassland,
        "SkyIslands",
        0,
    ),
    FixedRegion::new(
        "Deep Forest A",
        V3EnvironmentSettings::TemperateGrassland,
        "DeepForest",
        0,
    ),
    FixedRegion::new(
        "Deep Forest B",
        V3EnvironmentSettings::TemperateGrassland,
        "DeepForest",
        0,
    ),
    FixedRegion::new(
        "Forest B",
        V3EnvironmentSettings::TemperateGrassland,
        "Forest",
        4,
    ),
    FixedRegion::new(
        "Prairie B",
        V3EnvironmentSettings::TemperateGrassland,
        "Prairie",
        0,
    ),
    FixedRegion::new(
        "outlet Waterfall",
        V3EnvironmentSettings::TemperateGrassland,
        "Waterfall",
        5,
    ),
    FixedRegion::new("Fort", V3EnvironmentSettings::TemperateGrassland, "Fort", 0),
    FixedRegion::new("Caves", V3EnvironmentSettings::Rocky, "Caves", 0),
    FixedRegion::new("Volcano", V3EnvironmentSettings::Volcanic, "Volcano", 3),
    FixedRegion::new("Mountains A", V3EnvironmentSettings::Frozen, "Mountains", 0),
    FixedRegion::new("Mountains B", V3EnvironmentSettings::Frozen, "Mountains", 0),
    FixedRegion::new("Mountains C", V3EnvironmentSettings::Frozen, "Mountains", 0),
];

const DESERT_FIXED_ROTATIONS: [u8; V3_RING19_REGION_COUNT] =
    [0, 0, 1, 2, 3, 4, 5, 0, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0];

const INTERNAL_HYDROLOGY: [(u32, u32, Level); 8] = [
    (16, 5, 29),
    (5, 0, 16),
    (17, 6, 29),
    (6, 0, 16),
    (18, 1, 16),
    (1, 0, 16),
    (0, 4, 16),
    (4, 12, 16),
];

const BOUNDARY_HYDROLOGY: [(u32, HexSide, Level, FillMaterialRole); 2] = [
    (12, HexSide::SouthEast, 3, FillMaterialRole::Water),
    (15, HexSide::West, 14, FillMaterialRole::Lava),
];

#[derive(Debug, Clone, Copy)]
struct FixedRegion {
    #[cfg(test)]
    recipe: &'static str,
    rotation_turns: u8,
}

impl FixedRegion {
    const fn new(
        _name: &'static str,
        _environment: V3EnvironmentSettings,
        _recipe: &'static str,
        rotation_turns: u8,
    ) -> Self {
        Self {
            #[cfg(test)]
            recipe: _recipe,
            rotation_turns,
        }
    }
}

/// Candidate measurements retained beyond the public generation report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ring19Metrics {
    pub(crate) report: Ring19ReportMetrics,
    max_region_entry_steps: u32,
    region_entry_spread: u32,
}

#[derive(Debug)]
struct Ring19Recipe<'a> {
    level_height: f32,
    layout: ResolvedLayoutPlan,
    patch_by_coord: BTreeMap<HexCoord, PatchId>,
    settings: &'a V3Ring19Settings,
    art_catalog: &'a RuntimeArtCatalog,
    cave_vegetation: CaveVegetationSet,
    #[cfg(test)]
    reject_candidates: bool,
}

/// Runs exactly one eight-candidate selection loop for the complete Ring19 world.
pub(crate) fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
    art_catalog: &RuntimeArtCatalog,
) -> Result<ValidatedWorldSelection<Ring19Metrics>, V3GenerationError> {
    generate_with_options(
        grid_radius,
        level_height,
        settings,
        seed,
        art_catalog,
        false,
    )
}

fn generate_with_options(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
    art_catalog: &RuntimeArtCatalog,
    _reject_candidates: bool,
) -> Result<ValidatedWorldSelection<Ring19Metrics>, V3GenerationError> {
    if grid_radius != RING_RADIUS {
        return Err(V3GenerationError::RecipeContract(format!(
            "Ring19 requires grid radius {RING_RADIUS}, got {grid_radius}"
        )));
    }
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V3GenerationError::RecipeContract(
            "Ring19 level height must be positive and finite".to_owned(),
        ));
    }
    let ring = validate_recipe_settings(settings)?;
    let layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    validate_resolved_layout(&layout, ring.profile)?;
    let patch_by_coord = resolved_patch_index(&layout)?;
    let cave_vegetation = CaveVegetationSet::resolve(art_catalog, "Ring19 Caves")
        .map_err(V3GenerationError::RecipeContract)?;

    run_recipe(
        &Ring19Recipe {
            level_height,
            layout,
            patch_by_coord,
            settings: ring,
            art_catalog,
            cave_vegetation,
            #[cfg(test)]
            reject_candidates: _reject_candidates,
        },
        settings,
        grid_radius,
        seed,
    )
}

impl V3Recipe for Ring19Recipe<'_> {
    type Settings = ProceduralV3Settings;
    type Metrics = Ring19Metrics;
    type Score = (u32, u32, Reverse<u32>, u8);

    fn construct(
        &self,
        context: CandidateContext,
        settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, CandidateAttemptError> {
        validate_recipe_settings(settings).map_err(CandidateAttemptError::Fatal)?;
        if context.grid_radius != self.layout.grid_radius {
            return Err(CandidateAttemptError::Fatal(
                V3GenerationError::RecipeContract(
                    "Ring19 candidate radius disagrees with its resolved layout".to_owned(),
                ),
            ));
        }
        #[cfg(test)]
        if self.reject_candidates {
            return Err(CandidateAttemptError::Rejected(vec![recipe_issue(
                "forced Ring19 candidate rejection",
            )]));
        }

        self.construct_world(PatchBuildMode::Candidate {
            world_seed: context.seed,
            candidate: context.candidate,
        })
        .map_err(CandidateAttemptError::Rejected)
    }

    fn validate(
        &self,
        _settings: &Self::Settings,
        plan: &GeneratedWorldPlan,
    ) -> WorldValidation<Self::Metrics> {
        validate_ring19(plan, &self.patch_by_coord, self.settings.profile)
    }

    fn repair(
        &self,
        _context: CandidateContext,
        _settings: &Self::Settings,
        _plan: &mut GeneratedWorldPlan,
        _round: u8,
        _issues: &[WorldValidationIssue],
    ) -> Result<RepairOutcome, CandidateAttemptError> {
        Ok(RepairOutcome::NoChange)
    }

    fn score(
        &self,
        _settings: &Self::Settings,
        metrics: &Self::Metrics,
        candidate: u8,
    ) -> Self::Score {
        (
            metrics.max_region_entry_steps,
            metrics.region_entry_spread,
            Reverse(metrics.report.reachable_elevation_levels),
            candidate,
        )
    }

    fn canonical_fallback(
        &self,
        context: FallbackContext,
        settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, V3GenerationError> {
        validate_recipe_settings(settings)?;
        if context.grid_radius != self.layout.grid_radius {
            return Err(V3GenerationError::RecipeContract(
                "Ring19 fallback radius disagrees with its resolved layout".to_owned(),
            ));
        }
        self.construct_world(PatchBuildMode::CanonicalFallback)
            .map_err(recipe_issues_to_error)
    }
}

impl Ring19Recipe<'_> {
    fn construct_world(
        &self,
        mode: PatchBuildMode,
    ) -> Result<GeneratedWorldPlan, Vec<WorldValidationIssue>> {
        let fragments = self.construct_fragments(mode)?;
        let view_hint = ring19_view_hint(&fragments, self.level_height)?;
        compose_world(
            self.layout.clone(),
            fragments,
            WorldCompositionSettings {
                canonical_anchors: canonical_anchors(self.settings.profile),
                view_hint,
            },
        )
        .map_err(|error| {
            vec![recipe_issue(format!(
                "Ring19 checked composition failed: {error:?}"
            ))]
        })
    }

    fn construct_fragments(
        &self,
        mode: PatchBuildMode,
    ) -> Result<Vec<GeneratedPatchPlan>, Vec<WorldValidationIssue>> {
        let mut fragments = Vec::with_capacity(V3_RING19_REGION_COUNT);
        for (index, region) in self.settings.regions.iter().enumerate() {
            let id = PatchId(u32::try_from(index).map_err(|error| {
                vec![recipe_issue(format!(
                    "Ring19 region index {index} does not fit a patch identity: {error}"
                ))]
            })?);
            let patch = PatchRecipeContext::resolve(&self.layout, id)
                .map_err(|error| vec![recipe_issue(error.to_string())])?;
            let fragment = composite_patch::construct_fragment(
                patch,
                region.environment,
                &region.recipe,
                self.level_height,
                mode,
                self.art_catalog,
                &self.cave_vegetation,
            )
            .map_err(|issues| contextualize_fragment_issues(id, region, "construction", issues))?;
            composite_patch::validate_fragment(
                patch,
                region.environment,
                &region.recipe,
                &fragment,
                self.art_catalog,
                &self.cave_vegetation,
            )
            .map_err(|issues| contextualize_fragment_issues(id, region, "validation", issues))?;
            fragments.push(fragment);
        }
        Ok(fragments)
    }
}

fn canonical_anchors(profile: V3Ring19ProfileSettings) -> BTreeMap<String, PatchAnchorRef> {
    match profile {
        V3Ring19ProfileSettings::TwoRings => BTreeMap::from([
            (PARTY_START.to_owned(), anchor_ref(0, PARTY_START)),
            (HOSTILE_START.to_owned(), anchor_ref(0, HOSTILE_START)),
            (CONFLICT_CENTER.to_owned(), anchor_ref(0, CONFLICT_CENTER)),
            (
                MOUNTAIN_WATERFALL_OVERLOOK.to_owned(),
                anchor_ref(16, "stream_fall_overlook"),
            ),
            (
                CONFLUENCE_OVERLOOK.to_owned(),
                anchor_ref(0, CONFLICT_CENTER),
            ),
            (
                VEGETATION_GRADIENT_OVERLOOK.to_owned(),
                anchor_ref(2, "prairie_overlook"),
            ),
            (
                FORT_OUTLET_OVERLOOK.to_owned(),
                anchor_ref(12, "fall_overlook"),
            ),
        ]),
        V3Ring19ProfileSettings::DesertOasis => BTreeMap::from([
            (PARTY_START.to_owned(), anchor_ref(0, PARTY_START)),
            (HOSTILE_START.to_owned(), anchor_ref(18, HOSTILE_START)),
            (CONFLICT_CENTER.to_owned(), anchor_ref(0, OASIS_OVERLOOK)),
            (OASIS_OVERLOOK.to_owned(), anchor_ref(0, OASIS_OVERLOOK)),
            (INNER_DUNE_CREST.to_owned(), anchor_ref(1, "dune_crest")),
            (OUTER_DUNE_CREST.to_owned(), anchor_ref(7, "dune_crest")),
            (
                DESERT_PLAIN_OVERLOOK.to_owned(),
                anchor_ref(8, DESERT_PLAIN_OVERLOOK),
            ),
        ]),
    }
}

fn anchor_ref(patch: u32, local_name: &str) -> PatchAnchorRef {
    PatchAnchorRef {
        patch: PatchId(patch),
        local_name: local_name.to_owned(),
    }
}

fn contextualize_fragment_issues(
    id: PatchId,
    region: &Ring19RegionSettings,
    phase: &str,
    issues: Vec<WorldValidationIssue>,
) -> Vec<WorldValidationIssue> {
    issues
        .into_iter()
        .map(|issue| {
            recipe_issue(format!(
                "patch {} {} {phase} {:?}: {}",
                id.0,
                recipe_name(&region.recipe),
                issue.code,
                issue.detail
            ))
        })
        .collect()
}

fn validate_recipe_settings(
    settings: &ProceduralV3Settings,
) -> Result<&V3Ring19Settings, V3GenerationError> {
    let V3LayoutSettings::Ring19(ring) = &settings.layout else {
        return Err(V3GenerationError::RecipeUnavailable("Ring19"));
    };
    match ring.profile {
        V3Ring19ProfileSettings::TwoRings => ring.validate_two_rings_contract(),
        V3Ring19ProfileSettings::DesertOasis => ring.validate_desert_oasis_contract(),
    }
    .map_err(V3GenerationError::RecipeContract)?;
    Ok(ring)
}

fn validate_resolved_layout(
    layout: &ResolvedLayoutPlan,
    profile: V3Ring19ProfileSettings,
) -> Result<(), V3GenerationError> {
    if layout.kind != LayoutKind::Ring19
        || layout.grid_radius != RING_RADIUS
        || count_u32(layout.footprint.len()) != WORLD_COLUMNS
        || count_u32(layout.patches.len()) != PATCH_COUNT
        || count_u32(layout.shared_edges.len()) != RECIPROCAL_SEAMS
    {
        return Err(V3GenerationError::RecipeContract(format!(
            "Ring19 resolved layout must be radius {RING_RADIUS} with {WORLD_COLUMNS} columns, \
             {PATCH_COUNT} patches, and {RECIPROCAL_SEAMS} seams"
        )));
    }
    let boundary_sides = layout
        .patches
        .values()
        .flat_map(|patch| patch.edges.values())
        .filter(|reference| matches!(reference, ResolvedEdgeReference::WorldBoundary))
        .count();
    if count_u32(boundary_sides) != BOUNDARY_SIDES {
        return Err(V3GenerationError::RecipeContract(format!(
            "Ring19 resolved {boundary_sides} outer boundary sides instead of {BOUNDARY_SIDES}"
        )));
    }
    for index in 0..V3_RING19_REGION_COUNT {
        let id = PatchId(u32::try_from(index).unwrap_or(u32::MAX));
        let Some(patch) = layout.patches.get(&id) else {
            return Err(V3GenerationError::RecipeContract(format!(
                "Ring19 resolved layout is missing patch {index}"
            )));
        };
        let expected_rotation = fixed_rotation(profile, index).unwrap_or_default();
        if patch.biome_region != BiomeRegionId(id.0) || patch.rotation_turns != expected_rotation {
            let detail = match profile {
                V3Ring19ProfileSettings::TwoRings => {
                    format!("Ring19 patch {index} identity/rotation disagrees with its fixed slot")
                }
                V3Ring19ProfileSettings::DesertOasis => format!(
                    "Ring19 DesertOasis patch {index} identity/rotation disagrees with its fixed slot"
                ),
            };
            return Err(V3GenerationError::RecipeContract(detail));
        }
    }
    validate_resolved_hydrology(layout, profile)
}

fn fixed_rotation(profile: V3Ring19ProfileSettings, index: usize) -> Option<u8> {
    match profile {
        V3Ring19ProfileSettings::TwoRings => {
            FIXED_REGIONS.get(index).map(|region| region.rotation_turns)
        }
        V3Ring19ProfileSettings::DesertOasis => DESERT_FIXED_ROTATIONS.get(index).copied(),
    }
}

fn resolved_patch_index(
    layout: &ResolvedLayoutPlan,
) -> Result<BTreeMap<HexCoord, PatchId>, V3GenerationError> {
    let mut index = BTreeMap::new();
    for (patch_id, patch) in &layout.patches {
        for coord in &patch.mask {
            if let Some(previous) = index.insert(*coord, *patch_id) {
                return Err(V3GenerationError::RecipeContract(format!(
                    "Ring19 coordinate {coord:?} belongs to both {previous:?} and {patch_id:?}"
                )));
            }
        }
    }
    if index.len() != layout.footprint.len() || !index.keys().eq(layout.footprint.iter()) {
        return Err(V3GenerationError::RecipeContract(
            "Ring19 coordinate-to-patch index does not exactly cover the resolved footprint"
                .to_owned(),
        ));
    }
    Ok(index)
}

fn validate_resolved_hydrology(
    layout: &ResolvedLayoutPlan,
    profile: V3Ring19ProfileSettings,
) -> Result<(), V3GenerationError> {
    let (preferred, minimum, maximum) = match profile {
        V3Ring19ProfileSettings::TwoRings => (17, 16, 18),
        V3Ring19ProfileSettings::DesertOasis => (17, 15, 19),
    };
    let mut actual_internal = BTreeSet::new();
    for edge in layout.shared_edges.values() {
        if edge.elevation.preferred != preferred
            || edge.elevation.min != minimum
            || edge.elevation.max != maximum
            || edge.walker.count != WALKER_PORT_COUNT
            || edge.walker.width != WALKER_PORT_WIDTH
            || edge.walker.ports.len() != usize::from(WALKER_PORT_COUNT)
            || edge.approach_depth != SEAM_APPROACH_DEPTH
        {
            return Err(V3GenerationError::RecipeContract(
                "Ring19 resolved a seam outside its fixed walker authority".to_owned(),
            ));
        }
        if profile == V3Ring19ProfileSettings::DesertOasis
            && !matches!(edge.liquid, ResolvedLiquidPort::Dry)
        {
            return Err(V3GenerationError::RecipeContract(
                "Ring19 DesertOasis requires every reciprocal seam to remain dry".to_owned(),
            ));
        }
        let ResolvedLiquidPort::Directed {
            source,
            sink,
            port,
            elevation,
        } = &edge.liquid
        else {
            continue;
        };
        let ResolvedLiquidElevation::Exact(level) = elevation else {
            return Err(V3GenerationError::RecipeContract(
                "Ring19 directed liquid seams require exact elevation authority".to_owned(),
            ));
        };
        actual_internal.insert((source.0, sink.0, count_u32(port.lanes.len()), *level));
    }
    let expected_internal = match profile {
        V3Ring19ProfileSettings::TwoRings => INTERNAL_HYDROLOGY
            .into_iter()
            .map(|(source, sink, level)| (source, sink, SEAM_PORT_WIDTH, level))
            .collect::<BTreeSet<_>>(),
        V3Ring19ProfileSettings::DesertOasis => BTreeSet::new(),
    };
    if actual_internal != expected_internal {
        return Err(V3GenerationError::RecipeContract(format!(
            "Ring19 resolved internal hydrology differs from its fixed graph: \
             expected {expected_internal:?}, got {actual_internal:?}"
        )));
    }

    let actual_boundary = layout
        .boundary_liquid_outlets
        .iter()
        .map(|((source, side), outlet)| {
            (
                source.0,
                *side,
                count_u32(outlet.lanes.len()),
                outlet.level,
                outlet.approach_depth,
                outlet.source == *source,
            )
        })
        .collect::<BTreeSet<_>>();
    let expected_boundary = match profile {
        V3Ring19ProfileSettings::TwoRings => BOUNDARY_HYDROLOGY
            .into_iter()
            .map(|(source, side, level, _)| {
                (
                    source,
                    side,
                    SEAM_PORT_WIDTH,
                    level,
                    SEAM_APPROACH_DEPTH,
                    true,
                )
            })
            .collect::<BTreeSet<_>>(),
        V3Ring19ProfileSettings::DesertOasis => BTreeSet::new(),
    };
    if actual_boundary != expected_boundary {
        return Err(V3GenerationError::RecipeContract(format!(
            "Ring19 resolved boundary hydrology differs from its fixed outlets: \
             expected {expected_boundary:?}, got {actual_boundary:?}"
        )));
    }
    Ok(())
}

fn validate_ring19(
    plan: &GeneratedWorldPlan,
    patch_by_coord: &BTreeMap<HexCoord, PatchId>,
    profile: V3Ring19ProfileSettings,
) -> WorldValidation<Ring19Metrics> {
    // The common runner admits this callback only after `GeneratedWorldPlan::validate`.
    // Keeping layout-specific checks here avoids a third full scan of 9,241 columns.
    let mut issues = Vec::new();
    if let Err(error) = validate_resolved_layout(&plan.layout, profile) {
        issues.push(recipe_issue(error.to_string()));
    }

    let boundary_sides = plan
        .layout
        .patches
        .values()
        .flat_map(|patch| patch.edges.values())
        .filter(|reference| matches!(reference, ResolvedEdgeReference::WorldBoundary))
        .count();
    let biome_regions = plan
        .biome_regions
        .values()
        .copied()
        .collect::<BTreeSet<_>>();
    let expected_regions = (0..PATCH_COUNT).map(BiomeRegionId).collect::<BTreeSet<_>>();
    if biome_regions != expected_regions {
        issues.push(recipe_issue(format!(
            "Ring19 semantic surfaces publish biome regions {biome_regions:?}, expected \
             {expected_regions:?}"
        )));
    }

    let ordinary = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
    let Some(party) = plan.anchors.get(PARTY_START).copied() else {
        issues.push(recipe_issue(
            "Ring19 is missing its canonical party_start alias",
        ));
        return WorldValidation::Invalid(issues);
    };
    let Some(hostile) = plan.anchors.get(HOSTILE_START).copied() else {
        issues.push(recipe_issue(
            "Ring19 is missing its canonical hostile_start alias",
        ));
        return WorldValidation::Invalid(issues);
    };
    if !plan.anchors.contains_key(CONFLICT_CENTER) {
        issues.push(recipe_issue(
            "Ring19 is missing its canonical conflict_center alias",
        ));
    }
    let distances = ordinary.distances_from(party);
    for &name in review_anchors(profile) {
        match plan.anchors.get(name).copied() {
            Some(position) if ordinary.contains(position) && distances.contains_key(&position) => {}
            Some(position) => issues.push(recipe_issue(format!(
                "Ring19 review anchor {name:?} is not unblocked ordinary footing in the connected \
                 world network at {position:?}"
            ))),
            None => issues.push(recipe_issue(format!(
                "Ring19 is missing required whole-world review anchor {name:?}"
            ))),
        }
    }
    let disconnected_routes = plan
        .features
        .protected_routes
        .values()
        .flat_map(|route| &route.surfaces)
        .filter(|position| !distances.contains_key(position))
        .copied()
        .take(8)
        .collect::<Vec<_>>();
    if !disconnected_routes.is_empty() {
        issues.push(recipe_issue(format!(
            "Ring19 protected routes leave the connected ordinary world network at \
             {disconnected_routes:?}"
        )));
    }
    let disconnected_anchors = plan
        .anchors
        .iter()
        .filter(|(name, _)| {
            matches!(name.as_str(), PARTY_START | HOSTILE_START | CONFLICT_CENTER)
                || name.ends_with("_party_start")
                || name.ends_with("_hostile_start")
                || name.ends_with("_conflict_center")
        })
        .filter(|(_, position)| ordinary.contains(**position) && !distances.contains_key(position))
        .map(|(name, position)| (name.clone(), *position))
        .take(8)
        .collect::<Vec<_>>();
    if !disconnected_anchors.is_empty() {
        issues.push(recipe_issue(format!(
            "Ring19 ordinary anchors leave the connected world network at \
             {disconnected_anchors:?}"
        )));
    }
    let critical_route_steps = distances.get(&hostile).copied();
    if critical_route_steps.is_none() {
        issues.push(recipe_issue(
            "Ring19 canonical actor anchors are not joined by ordinary traversal",
        ));
    }

    let mut region_entry_steps = BTreeMap::<PatchId, u32>::new();
    for id in 0..PATCH_COUNT {
        let region = BiomeRegionId(id);
        let entry = distances
            .iter()
            .filter(|(position, _)| plan.biome_regions.get(position) == Some(&region))
            .map(|(_, distance)| *distance)
            .min();
        match entry {
            Some(distance) => {
                region_entry_steps.insert(PatchId(id), distance);
            }
            None => issues.push(recipe_issue(format!(
                "Ring19 patch {id} has no ordinary surface reachable from party_start"
            ))),
        }
    }

    let seam_resilience = physical_seam_resilience(plan, &ordinary, party, &expected_regions);
    if seam_resilience.attached_seams != RECIPROCAL_SEAMS {
        issues.push(recipe_issue(format!(
            "Ring19 opens {} of {RECIPROCAL_SEAMS} required walker seams",
            seam_resilience.attached_seams
        )));
    }
    if !seam_resilience.unreachable_endpoints.is_empty() {
        issues.push(recipe_issue(format!(
            "Ring19 walker seam endpoints outside the intact party network: {:?}",
            seam_resilience
                .unreachable_endpoints
                .iter()
                .copied()
                .take(8)
                .collect::<Vec<_>>()
        )));
    }
    if !seam_resilience.removal_failures.is_empty() {
        let failures = seam_resilience
            .removal_failures
            .iter()
            .take(4)
            .map(|failure| {
                format!(
                    "{:?}: missing regions {:?}, stranded endpoints {:?}",
                    failure.edge,
                    failure.missing_regions,
                    failure
                        .stranded_endpoints
                        .iter()
                        .copied()
                        .take(8)
                        .collect::<Vec<_>>()
                )
            })
            .collect::<Vec<_>>();
        issues.push(recipe_issue(format!(
            "Ring19 physical seam-removal checks failed: {failures:?}"
        )));
    }
    let redundant_regions = count_u32(seam_resilience.robust_regions.len());
    if redundant_regions != PATCH_COUNT {
        issues.push(recipe_issue(format!(
            "Ring19 physical walker graph retains redundant routes for \
             {redundant_regions}/{PATCH_COUNT} patches"
        )));
    }

    let liquid_metrics = validate_routed_liquids(plan, patch_by_coord, &mut issues);
    let (expected_internal_liquid_seams, expected_boundary_liquid_outlets) =
        expected_hydrology_counts(profile);
    if liquid_metrics.internal_seams != expected_internal_liquid_seams {
        let detail = match profile {
            V3Ring19ProfileSettings::TwoRings => format!(
                "Ring19 realizes {} directed liquid seams instead of {INTERNAL_LIQUID_SEAMS}",
                liquid_metrics.internal_seams
            ),
            V3Ring19ProfileSettings::DesertOasis => format!(
                "Ring19 DesertOasis realizes {} directed liquid seams instead of 0",
                liquid_metrics.internal_seams
            ),
        };
        issues.push(recipe_issue(detail));
    }
    if liquid_metrics.boundary_outlets != expected_boundary_liquid_outlets {
        let detail = match profile {
            V3Ring19ProfileSettings::TwoRings => format!(
                "Ring19 realizes {} boundary liquid outlets instead of \
                 {BOUNDARY_LIQUID_OUTLETS}",
                liquid_metrics.boundary_outlets
            ),
            V3Ring19ProfileSettings::DesertOasis => format!(
                "Ring19 DesertOasis realizes {} boundary liquid outlets instead of 0",
                liquid_metrics.boundary_outlets
            ),
        };
        issues.push(recipe_issue(detail));
    }

    let reachable_levels = distances
        .keys()
        .map(|position| position.level)
        .collect::<BTreeSet<_>>();
    let min_level = reachable_levels.iter().next().copied().unwrap_or_default();
    let max_level = reachable_levels
        .iter()
        .next_back()
        .copied()
        .unwrap_or_default();
    let min_outer_entry = region_entry_steps
        .iter()
        .filter_map(|(patch, distance)| (patch.0 != 0).then_some(*distance))
        .min()
        .unwrap_or_default();
    let max_outer_entry = region_entry_steps
        .iter()
        .filter_map(|(patch, distance)| (patch.0 != 0).then_some(*distance))
        .max()
        .unwrap_or_default();
    let liquid_cells = plan
        .liquids
        .bodies
        .values()
        .flat_map(|body| body.nodes.keys().map(|position| position.coord))
        .collect::<BTreeSet<_>>()
        .len();
    let report = Ring19ReportMetrics {
        world_columns: count_u32(plan.volume.mask.len()),
        biome_regions: count_u32(biome_regions.len()),
        reciprocal_seams: count_u32(plan.layout.shared_edges.len()),
        boundary_sides: count_u32(boundary_sides),
        ordinary_surfaces: count_u32(ordinary.len()),
        reachable_surfaces: count_u32(distances.len()),
        reachable_elevation_levels: count_u32(reachable_levels.len()),
        relief: max_level.saturating_sub(min_level),
        critical_route_steps: critical_route_steps.unwrap_or_default(),
        macro_edges: seam_resilience.attached_seams,
        redundant_regions,
        directed_liquid_seams: liquid_metrics.internal_seams,
        boundary_liquid_outlets: liquid_metrics.boundary_outlets,
        liquid_cells: count_u32(liquid_cells),
        feature_instances: count_u32(plan.features.by_id.len()),
        structures: count_u32(plan.structures.by_id.len()),
        gameplay_lights: count_u32(plan.lights.len()),
        interiors: count_u32(plan.interiors.by_id.len()),
    };
    if issues.is_empty() {
        WorldValidation::Valid(Ring19Metrics {
            report,
            max_region_entry_steps: max_outer_entry,
            region_entry_spread: max_outer_entry.saturating_sub(min_outer_entry),
        })
    } else {
        WorldValidation::Invalid(issues)
    }
}

fn review_anchors(profile: V3Ring19ProfileSettings) -> &'static [&'static str] {
    match profile {
        V3Ring19ProfileSettings::TwoRings => &REVIEW_ANCHORS,
        V3Ring19ProfileSettings::DesertOasis => &DESERT_REVIEW_ANCHORS,
    }
}

const fn expected_hydrology_counts(profile: V3Ring19ProfileSettings) -> (u32, u32) {
    match profile {
        V3Ring19ProfileSettings::TwoRings => (INTERNAL_LIQUID_SEAMS, BOUNDARY_LIQUID_OUTLETS),
        V3Ring19ProfileSettings::DesertOasis => (0, 0),
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct LiquidMetrics {
    internal_seams: u32,
    boundary_outlets: u32,
}

type LiquidNodeIndex = BTreeMap<TilePos, (LiquidBodyId, LiquidNode, FillMaterialRole)>;

fn validate_routed_liquids(
    plan: &GeneratedWorldPlan,
    patch_by_coord: &BTreeMap<HexCoord, PatchId>,
    issues: &mut Vec<WorldValidationIssue>,
) -> LiquidMetrics {
    let mut index = LiquidNodeIndex::new();
    for (body_id, body) in &plan.liquids.bodies {
        for (position, node) in &body.nodes {
            if index
                .insert(*position, (*body_id, *node, body.material))
                .is_some()
            {
                issues.push(recipe_issue(format!(
                    "Ring19 liquid position {position:?} belongs to more than one body"
                )));
            }
        }
    }

    let mut metrics = LiquidMetrics::default();
    let mut routed_starts = BTreeSet::new();
    let mut allowed_crossings = BTreeSet::new();
    for edge in plan.layout.shared_edges.values() {
        let ResolvedLiquidPort::Directed {
            source,
            sink,
            port,
            elevation,
        } = &edge.liquid
        else {
            continue;
        };
        metrics.internal_seams = metrics.internal_seams.saturating_add(1);
        let ResolvedLiquidElevation::Exact(level) = elevation else {
            issues.push(recipe_issue(format!(
                "Ring19 liquid seam {source:?}->{sink:?} lacks an exact level"
            )));
            continue;
        };
        let source_is_first = *source == edge.first.0 && *sink == edge.second.0;
        for (first, second) in &port.lanes {
            let (source_coord, sink_coord) = if source_is_first {
                (*first, *second)
            } else {
                (*second, *first)
            };
            let source_position = TilePos::new(source_coord, *level);
            let sink_position = TilePos::new(sink_coord, *level);
            allowed_crossings.insert((source_position, sink_position));
            match (
                index.get(&source_position).copied(),
                index.get(&sink_position).copied(),
            ) {
                (
                    Some((source_body, source_node, source_material)),
                    Some((sink_body, _, sink_material)),
                ) => {
                    routed_starts.insert((source_body, source_position));
                    if source_body != sink_body
                        || source_material != FillMaterialRole::Water
                        || sink_material != FillMaterialRole::Water
                    {
                        issues.push(recipe_issue(format!(
                            "Ring19 water seam {source:?}->{sink:?} does not belong to one Water body"
                        )));
                    }
                    if source_node.downstream != Some(sink_position) {
                        issues.push(recipe_issue(format!(
                            "Ring19 water seam {source:?}->{sink:?} does not link exact endpoints \
                             {source_position:?}->{sink_position:?}"
                        )));
                    }
                }
                _ => issues.push(recipe_issue(format!(
                    "Ring19 water seam {source:?}->{sink:?} is missing exact endpoints \
                     {source_position:?}/{sink_position:?}"
                ))),
            }
        }
    }

    let mut expected_terminals = BTreeMap::<LiquidBodyId, BTreeSet<TilePos>>::new();
    for ((source, side), outlet) in &plan.layout.boundary_liquid_outlets {
        metrics.boundary_outlets = metrics.boundary_outlets.saturating_add(1);
        let expected_material =
            BOUNDARY_HYDROLOGY
                .iter()
                .find_map(|(expected_source, expected_side, _, material)| {
                    (*expected_source == source.0 && expected_side == side).then_some(*material)
                });
        for (inside, _) in &outlet.lanes {
            let position = TilePos::new(*inside, outlet.level);
            let Some((body_id, node, material)) = index.get(&position).copied() else {
                issues.push(recipe_issue(format!(
                    "Ring19 boundary outlet {source:?}/{side:?} is missing {position:?}"
                )));
                continue;
            };
            expected_terminals
                .entry(body_id)
                .or_default()
                .insert(position);
            if expected_material != Some(material) {
                issues.push(recipe_issue(format!(
                    "Ring19 boundary outlet {source:?}/{side:?} has material {material:?}, \
                     expected {expected_material:?}"
                )));
            }
            if node.state != LiquidFlowState::Still || node.downstream.is_some() {
                issues.push(recipe_issue(format!(
                    "Ring19 boundary outlet {source:?}/{side:?} node {position:?} is not an exact \
                     Still terminal"
                )));
            }
        }
    }

    for (body_id, start) in routed_starts {
        match routed_terminal(&index, body_id, start) {
            Ok(terminal)
                if expected_terminals
                    .get(&body_id)
                    .is_some_and(|expected| expected.contains(&terminal)) => {}
            Ok(terminal) => issues.push(recipe_issue(format!(
                "Ring19 routed liquid path from {start:?} terminates internally at \
                 {terminal:?} instead of a declared boundary outlet"
            ))),
            Err(detail) => issues.push(recipe_issue(detail)),
        }
    }

    for body in plan.liquids.bodies.values() {
        for (source, node) in &body.nodes {
            let Some(sink) = node.downstream else {
                continue;
            };
            let source_patch = patch_by_coord.get(&source.coord);
            let sink_patch = patch_by_coord.get(&sink.coord);
            if source_patch != sink_patch && !allowed_crossings.contains(&(*source, sink)) {
                issues.push(recipe_issue(format!(
                    "Ring19 liquid crosses patches outside a declared directed lane: \
                     {source:?}->{sink:?}"
                )));
            }
        }
    }
    metrics
}

fn routed_terminal(
    index: &LiquidNodeIndex,
    body_id: LiquidBodyId,
    start: TilePos,
) -> Result<TilePos, String> {
    let mut current = start;
    let mut visited = BTreeSet::new();
    loop {
        if !visited.insert(current) {
            return Err(format!(
                "Ring19 routed liquid path from {start:?} contains a cycle at {current:?}"
            ));
        }
        let Some((current_body, node, _)) = index.get(&current).copied() else {
            return Err(format!(
                "Ring19 routed liquid path from {start:?} leaves its authored body at {current:?}"
            ));
        };
        if current_body != body_id {
            return Err(format!(
                "Ring19 routed liquid path from {start:?} changes body from {body_id:?} to \
                 {current_body:?} at {current:?}"
            ));
        }
        let Some(downstream) = node.downstream else {
            return Ok(current);
        };
        current = downstream;
    }
}

#[derive(Debug)]
struct SeamRemovalFailure {
    edge: ResolvedEdgeId,
    missing_regions: Vec<BiomeRegionId>,
    stranded_endpoints: Vec<TilePos>,
}

#[derive(Debug)]
struct PhysicalSeamResilience {
    attached_seams: u32,
    robust_regions: BTreeSet<BiomeRegionId>,
    unreachable_endpoints: BTreeSet<TilePos>,
    removal_failures: Vec<SeamRemovalFailure>,
}

type SeamTransitions = BTreeMap<ResolvedEdgeId, BTreeSet<(TilePos, TilePos)>>;

fn physical_seam_resilience(
    plan: &GeneratedWorldPlan,
    ordinary: &OrdinaryGraph,
    party: TilePos,
    expected_regions: &BTreeSet<BiomeRegionId>,
) -> PhysicalSeamResilience {
    let transitions = plan
        .layout
        .shared_edges
        .iter()
        .map(|(edge_id, edge)| {
            let level = edge.elevation.preferred;
            let lanes = edge
                .walker
                .ports
                .iter()
                .flat_map(|port| &port.lanes)
                .map(|(first, second)| {
                    ordered_transition(TilePos::new(*first, level), TilePos::new(*second, level))
                })
                .collect::<BTreeSet<_>>();
            (*edge_id, lanes)
        })
        .collect::<SeamTransitions>();
    analyze_physical_seam_resilience(
        ordinary,
        party,
        &transitions,
        &plan.biome_regions,
        expected_regions,
    )
}

fn analyze_physical_seam_resilience(
    ordinary: &OrdinaryGraph,
    party: TilePos,
    transitions: &SeamTransitions,
    biome_regions: &BTreeMap<TilePos, BiomeRegionId>,
    expected_regions: &BTreeSet<BiomeRegionId>,
) -> PhysicalSeamResilience {
    let intact = ordinary
        .distances_from(party)
        .into_keys()
        .collect::<BTreeSet<_>>();
    let all_endpoints = transitions
        .values()
        .flat_map(|lanes| lanes.iter().flat_map(|(first, second)| [*first, *second]))
        .collect::<BTreeSet<_>>();
    let unreachable_endpoints = all_endpoints
        .difference(&intact)
        .copied()
        .collect::<BTreeSet<_>>();
    let attached_seams = transitions
        .values()
        .filter(|lanes| {
            !lanes.is_empty()
                && lanes.iter().all(|(first, second)| {
                    intact.contains(first)
                        && intact.contains(second)
                        && ordinary.admits(*first, *second)
                })
        })
        .count();

    let mut robust_regions = expected_regions.clone();
    let mut removal_failures = Vec::new();
    for (edge, removed) in transitions {
        let reached = reachable_avoiding_transitions(ordinary, party, removed);
        let reached_regions = reached
            .iter()
            .filter_map(|position| biome_regions.get(position).copied())
            .collect::<BTreeSet<_>>();
        robust_regions.retain(|region| reached_regions.contains(region));
        let missing_regions = expected_regions
            .difference(&reached_regions)
            .copied()
            .collect::<Vec<_>>();
        let stranded_endpoints = all_endpoints
            .difference(&reached)
            .copied()
            .collect::<Vec<_>>();
        if !missing_regions.is_empty() || !stranded_endpoints.is_empty() {
            removal_failures.push(SeamRemovalFailure {
                edge: *edge,
                missing_regions,
                stranded_endpoints,
            });
        }
    }
    PhysicalSeamResilience {
        attached_seams: count_u32(attached_seams),
        robust_regions,
        unreachable_endpoints,
        removal_failures,
    }
}

fn reachable_avoiding_transitions(
    ordinary: &OrdinaryGraph,
    start: TilePos,
    removed: &BTreeSet<(TilePos, TilePos)>,
) -> BTreeSet<TilePos> {
    if !ordinary.contains(start) {
        return BTreeSet::new();
    }
    let mut reached = BTreeSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(current) = queue.pop_front() {
        for neighbor in ordinary.neighbors(current) {
            if removed.contains(&ordered_transition(current, *neighbor)) {
                continue;
            }
            if reached.insert(*neighbor) {
                queue.push_back(*neighbor);
            }
        }
    }
    reached
}

fn ordered_transition(first: TilePos, second: TilePos) -> (TilePos, TilePos) {
    if first <= second {
        (first, second)
    } else {
        (second, first)
    }
}

#[cfg(test)]
fn redundant_region_count(edges: &BTreeSet<(PatchId, PatchId)>) -> u32 {
    (0..PATCH_COUNT)
        .filter(|target| {
            edges
                .iter()
                .all(|removed| reachable_patches(edges, Some(*removed)).contains(&PatchId(*target)))
        })
        .count()
        .try_into()
        .unwrap_or(u32::MAX)
}

#[cfg(test)]
fn reachable_patches(
    edges: &BTreeSet<(PatchId, PatchId)>,
    removed: Option<(PatchId, PatchId)>,
) -> BTreeSet<PatchId> {
    let mut reached = BTreeSet::from([PatchId(0)]);
    let mut queue = VecDeque::from([PatchId(0)]);
    while let Some(current) = queue.pop_front() {
        for edge in edges {
            if Some(*edge) == removed {
                continue;
            }
            let neighbor = if edge.0 == current {
                Some(edge.1)
            } else if edge.1 == current {
                Some(edge.0)
            } else {
                None
            };
            if let Some(neighbor) = neighbor.filter(|neighbor| reached.insert(*neighbor)) {
                queue.push_back(neighbor);
            }
        }
    }
    reached
}

fn ring19_visual_points(fragments: &[GeneratedPatchPlan], level_height: f32) -> Vec<ReviewPoint> {
    let mut points = Vec::new();
    for (coord, column) in fragments
        .iter()
        .flat_map(|fragment| &fragment.volume.columns)
    {
        let mut occupied_bounds: Option<LevelInterval> = None;
        for element in &column.elements {
            let levels = match *element {
                VolumeElement::Solid(mass) => mass.levels,
                VolumeElement::Fill(fill) => fill.levels,
            };
            occupied_bounds = Some(occupied_bounds.map_or(levels, |bounds| {
                LevelInterval::new(bounds.bottom.min(levels.bottom), bounds.top.max(levels.top))
            }));
        }
        let Some(occupied_bounds) = occupied_bounds else {
            continue;
        };
        let center = coord.to_world(0.0);
        let bottom = level_world_height(occupied_bounds.bottom, level_height);
        let top = level_world_height(occupied_bounds.top, level_height);
        for (offset_x, offset_z) in HEX_CORNER_OFFSETS {
            let x = center.x + offset_x * HEX_CIRCUMRADIUS;
            let z = center.z + offset_z * HEX_CIRCUMRADIUS;
            points.push(ReviewPoint::new(x, bottom, z));
            points.push(ReviewPoint::new(x, top, z));
        }
    }
    points
}

#[expect(
    clippy::cast_precision_loss,
    reason = "validated V3 levels are bounded to 128 and therefore exact in f32"
)]
fn level_world_height(level: Level, level_height: f32) -> f32 {
    (level as f32) * level_height
}

fn ring19_visual_bounds(points: &[ReviewPoint]) -> Option<(ReviewPoint, ReviewPoint)> {
    let first = points.first().copied()?;
    Some(
        points
            .iter()
            .copied()
            .skip(1)
            .fold((first, first), |(minimum, maximum), point| {
                (
                    ReviewPoint::new(
                        minimum.x.min(point.x),
                        minimum.y.min(point.y),
                        minimum.z.min(point.z),
                    ),
                    ReviewPoint::new(
                        maximum.x.max(point.x),
                        maximum.y.max(point.y),
                        maximum.z.max(point.z),
                    ),
                )
            }),
    )
}

fn ring19_review_frame(points: &[ReviewPoint], focus: ReviewPoint) -> f32 {
    let horizontal_scale =
        RING19_REVIEW_NDC_LIMIT * RING19_REVIEW_TAN_HALF_VERTICAL_FOV * RING19_REVIEW_ASPECT_RATIO;
    let vertical_scale = RING19_REVIEW_NDC_LIMIT * RING19_REVIEW_TAN_HALF_VERTICAL_FOV;
    let mut frame = LEGACY_RING19_VIEW_FRAME;
    for point in points {
        let relative = point.relative_to(focus);
        for basis in RING19_REVIEW_CAMERA_BASES {
            let projected_depth = (relative.dot(basis.right).abs() / horizontal_scale)
                .max(relative.dot(basis.up).abs() / vertical_scale);
            let required_frame =
                (projected_depth - relative.dot(basis.forward)) / RING19_VIEW_OFFSET_LENGTH;
            frame = frame.max(required_frame);
        }
    }
    frame.ceil()
}

fn ring19_view_hint(
    fragments: &[GeneratedPatchPlan],
    level_height: f32,
) -> Result<MapViewHint, Vec<WorldValidationIssue>> {
    let points = ring19_visual_points(fragments, level_height);
    let Some((minimum, maximum)) = ring19_visual_bounds(&points) else {
        return Err(vec![recipe_issue(
            "Ring19 cannot frame a world without occupied semantic terrain",
        )]);
    };
    let focus = ReviewPoint::new(
        (minimum.x + maximum.x) * 0.5,
        (minimum.y + maximum.y) * 0.5,
        (minimum.z + maximum.z) * 0.5,
    );
    let frame = ring19_review_frame(&points, focus);
    let hint = MapViewHint::new(
        (
            focus.x,
            focus.y + frame * RING19_VIEW_EYE_UP,
            focus.z + frame * RING19_VIEW_EYE_BACK,
        ),
        (focus.x, focus.y, focus.z),
    );
    if hint.is_valid() {
        Ok(hint)
    } else {
        Err(vec![recipe_issue("Ring19 overview camera hint is invalid")])
    }
}

#[cfg(test)]
fn ordered_edge(first: PatchId, second: PatchId) -> (PatchId, PatchId) {
    if first < second {
        (first, second)
    } else {
        (second, first)
    }
}

fn recipe_issues_to_error(issues: Vec<WorldValidationIssue>) -> V3GenerationError {
    V3GenerationError::RecipeContract(
        issues
            .into_iter()
            .map(|issue| issue.detail)
            .collect::<Vec<_>>()
            .join("; "),
    )
}

fn recipe_issue(detail: impl Into<String>) -> WorldValidationIssue {
    WorldValidationIssue::new(WorldIssueCode::Recipe("ring19"), detail)
}

const fn recipe_name(recipe: &V3RecipeSettings) -> &'static str {
    match recipe {
        V3RecipeSettings::Hills(_) => "Hills",
        V3RecipeSettings::SkyIslands(_) => "SkyIslands",
        V3RecipeSettings::Mountains(_) => "Mountains",
        V3RecipeSettings::Caves(_) => "Caves",
        V3RecipeSettings::Waterfall(_) => "Waterfall",
        V3RecipeSettings::Forest(_) => "Forest",
        V3RecipeSettings::Fort(_) => "Fort",
        V3RecipeSettings::Volcano(_) => "Volcano",
        V3RecipeSettings::DeepForest(_) => "DeepForest",
        V3RecipeSettings::Prairie(_) => "Prairie",
        V3RecipeSettings::ShallowSea(_) => "ShallowSea",
        V3RecipeSettings::Beach(_) => "Beach",
        V3RecipeSettings::Shore(_) => "Shore",
        V3RecipeSettings::DeepMountain(_) => "DeepMountain",
        V3RecipeSettings::CrystalAscent(_) => "CrystalAscent",
        V3RecipeSettings::DesertTransition(_) => "DesertTransition",
        V3RecipeSettings::DesertPlain(_) => "DesertPlain",
        V3RecipeSettings::Dunes(_) => "Dunes",
        V3RecipeSettings::Oasis(_) => "Oasis",
        V3RecipeSettings::SandyIslets(_) => "SandyIslets",
        V3RecipeSettings::WoodedIsland(_) => "WoodedIsland",
    }
}

fn count_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    use super::super::{deep_forest, desert_vegetation, prairie, vegetation_landform};
    use super::*;
    use crate::procedural_v3::liquid::LiquidPlan;
    use crate::procedural_v3::volume::{
        LevelInterval, SolidMass, SolidMaterialRole, SurfaceAccess, SurfaceMetadata, VolumeColumn,
        VolumeElement, VolumePlan,
    };
    use crate::procedural_v3::world::{FeaturePlan, InteriorPlan, StructurePlan};
    use crate::settings::{
        ring19_region_coord, EdgeElevationSettings, EdgeLiquidSettings, MapSettings,
        ProceduralSettings, SharedEdgeSettings, TerrainSettings, V3DesertPlainSettings,
        V3DunesSettings, V3OasisSettings, WalkerPortSettings,
    };

    fn assert_view_hint_close(actual: MapViewHint, expected: MapViewHint) {
        for (actual, expected) in [
            (actual.eye.0, expected.eye.0),
            (actual.eye.1, expected.eye.1),
            (actual.eye.2, expected.eye.2),
            (actual.focus.0, expected.focus.0),
            (actual.focus.1, expected.focus.1),
            (actual.focus.2, expected.focus.2),
        ] {
            assert!(
                (actual - expected).abs() < 0.000_1,
                "view-hint round trip produced {actual}, expected {expected}"
            );
        }
    }

    #[test]
    fn fixed_roster_and_hydrology_match_the_radius_two_slot_contract() {
        assert_eq!(FIXED_REGIONS.len(), V3_RING19_REGION_COUNT);
        assert_eq!(
            FIXED_REGIONS.first().map(|region| region.recipe),
            Some("Hills")
        );
        assert_eq!(
            FIXED_REGIONS.get(7).map(|region| region.recipe),
            Some("SkyIslands")
        );
        assert_eq!(
            FIXED_REGIONS.get(15).map(|region| region.recipe),
            Some("Volcano")
        );
        assert_eq!(
            FIXED_REGIONS.get(18).map(|region| region.recipe),
            Some("Mountains")
        );

        for (source, sink, _) in INTERNAL_HYDROLOGY {
            let source = u8::try_from(source).ok().and_then(ring19_region_coord);
            let sink = u8::try_from(sink).ok().and_then(ring19_region_coord);
            assert!(
                source.is_some() && sink.is_some(),
                "fixed liquid slots must have coordinates"
            );
            let (Some(source), Some(sink)) = (source, sink) else {
                continue;
            };
            let distance = source
                .0
                .abs_diff(sink.0)
                .max(source.1.abs_diff(sink.1))
                .max(source.2.abs_diff(sink.2));
            assert_eq!(distance, 1, "fixed liquid handoff must use one seam");
        }
    }

    #[test]
    fn desert_profile_resolves_fixed_rotations_dry_seams_and_aliases() {
        let settings = desert_settings();
        let ring = validate_recipe_settings(&settings).expect("desert profile should validate");
        assert_eq!(ring.profile, V3Ring19ProfileSettings::DesertOasis);
        let layout = resolve_layout(RING_RADIUS, &settings).expect("desert layout should resolve");
        validate_resolved_layout(&layout, ring.profile)
            .expect("desert resolved layout should retain its profile contract");
        assert!(validate_resolved_layout(&layout, V3Ring19ProfileSettings::TwoRings).is_err());
        assert_eq!(count_u32(layout.shared_edges.len()), RECIPROCAL_SEAMS);
        assert_eq!(count_u32(layout.patches.len()), PATCH_COUNT);
        assert!(layout.boundary_liquid_outlets.is_empty());
        assert!(layout.shared_edges.values().all(|edge| {
            edge.elevation.preferred == 17
                && edge.elevation.min == 15
                && edge.elevation.max == 19
                && edge.walker.count == WALKER_PORT_COUNT
                && edge.walker.width == WALKER_PORT_WIDTH
                && matches!(edge.liquid, ResolvedLiquidPort::Dry)
        }));
        for (index, expected_rotation) in DESERT_FIXED_ROTATIONS.into_iter().enumerate() {
            let id = PatchId(u32::try_from(index).expect("desert slot should fit a patch id"));
            assert_eq!(
                layout.patches.get(&id).map(|patch| patch.rotation_turns),
                Some(expected_rotation)
            );
        }
        assert_eq!(
            expected_hydrology_counts(V3Ring19ProfileSettings::DesertOasis),
            (0, 0)
        );

        let aliases = canonical_anchors(V3Ring19ProfileSettings::DesertOasis);
        assert_anchor_ref(&aliases, PARTY_START, 0, PARTY_START);
        assert_anchor_ref(&aliases, HOSTILE_START, 18, HOSTILE_START);
        assert_anchor_ref(&aliases, CONFLICT_CENTER, 0, OASIS_OVERLOOK);
        assert_anchor_ref(&aliases, OASIS_OVERLOOK, 0, OASIS_OVERLOOK);
        assert_anchor_ref(&aliases, INNER_DUNE_CREST, 1, "dune_crest");
        assert_anchor_ref(&aliases, OUTER_DUNE_CREST, 7, "dune_crest");
        assert_anchor_ref(&aliases, DESERT_PLAIN_OVERLOOK, 8, DESERT_PLAIN_OVERLOOK);
        assert_eq!(aliases.len(), 7);
        assert_eq!(
            review_anchors(V3Ring19ProfileSettings::DesertOasis),
            &DESERT_REVIEW_ANCHORS
        );
    }

    fn assert_anchor_ref(
        aliases: &BTreeMap<String, PatchAnchorRef>,
        alias: &str,
        patch: u32,
        local_name: &str,
    ) {
        let actual = aliases
            .get(alias)
            .unwrap_or_else(|| panic!("desert profile should publish alias {alias:?}"));
        assert_eq!(actual.patch, PatchId(patch));
        assert_eq!(actual.local_name, local_name);
    }

    #[test]
    fn vegetation_fragment_view_hints_round_trip_once_from_rotated_non_origin_patches() {
        let layout = resolve_layout(RING_RADIUS, settings()).expect("Ring19 layout should resolve");
        let V3LayoutSettings::Ring19(ring) = &settings().layout else {
            panic!("tracked settings should use Ring19");
        };

        let deep_patch = PatchRecipeContext::resolve(&layout, PatchId(2))
            .expect("Forest A patch should resolve");
        assert_ne!(deep_patch.patch.rotation_turns, 0);
        assert_ne!(
            deep_patch.local_frame().expect("Deep frame").center(),
            HexCoord::ORIGIN
        );
        let deep_region = ring.regions.get(8).expect("slot 8 should exist");
        let V3RecipeSettings::DeepForest(deep_settings) = &deep_region.recipe else {
            panic!("slot 8 should provide Deep Forest settings");
        };
        let deep = deep_forest::construct_patch(
            deep_patch,
            deep_settings,
            deep_region.environment,
            0.4,
            PatchBuildMode::CanonicalFallback,
            runtime_art_catalog(),
        )
        .expect("Deep Forest should construct in a rotated non-origin patch");
        let deep_frame = deep_patch.local_frame().expect("Deep frame should resolve");
        let deep_local = deep_frame
            .canonical_local_world(&deep)
            .expect("Deep Forest should round-trip to its local frame");
        assert_view_hint_close(
            deep_local.view_hint,
            vegetation_landform::view_hint(
                deep_frame.scale(),
                deep_settings.base_level,
                deep_settings.max_relief,
                0.4,
                "deep forest",
            )
            .expect("Deep Forest local view should be valid"),
        );

        let prairie_patch = PatchRecipeContext::resolve(&layout, PatchId(10))
            .expect("Forest B patch should resolve");
        assert_ne!(prairie_patch.patch.rotation_turns, 0);
        assert_ne!(
            prairie_patch.local_frame().expect("Prairie frame").center(),
            HexCoord::ORIGIN
        );
        let prairie_region = ring.regions.get(3).expect("slot 3 should exist");
        let V3RecipeSettings::Prairie(prairie_settings) = &prairie_region.recipe else {
            panic!("slot 3 should provide Prairie settings");
        };
        let prairie = prairie::construct_patch(
            prairie_patch,
            prairie_settings,
            prairie_region.environment,
            0.4,
            PatchBuildMode::CanonicalFallback,
            runtime_art_catalog(),
        )
        .expect("Prairie should construct in a rotated non-origin patch");
        let prairie_frame = prairie_patch
            .local_frame()
            .expect("Prairie frame should resolve");
        let prairie_local = prairie_frame
            .canonical_local_world(&prairie)
            .expect("Prairie should round-trip to its local frame");
        assert_view_hint_close(
            prairie_local.view_hint,
            vegetation_landform::view_hint(
                prairie_frame.scale(),
                prairie_settings.base_level,
                prairie_settings.max_relief,
                0.4,
                "prairie",
            )
            .expect("Prairie local view should be valid"),
        );
    }

    #[test]
    fn fixed_radius_two_layout_topology_survives_removal_of_any_one_seam() {
        let coords = (0..PATCH_COUNT)
            .filter_map(|id| {
                u8::try_from(id)
                    .ok()
                    .and_then(ring19_region_coord)
                    .map(|coord| (PatchId(id), coord))
            })
            .collect::<Vec<_>>();
        assert_eq!(count_u32(coords.len()), PATCH_COUNT);
        let mut edges = BTreeSet::new();
        for (index, (first_id, first)) in coords.iter().enumerate() {
            for (second_id, second) in coords.iter().skip(index.saturating_add(1)) {
                let distance = first
                    .0
                    .abs_diff(second.0)
                    .max(first.1.abs_diff(second.1))
                    .max(first.2.abs_diff(second.2));
                if distance == 1 {
                    edges.insert(ordered_edge(*first_id, *second_id));
                }
            }
        }
        assert_eq!(count_u32(edges.len()), RECIPROCAL_SEAMS);
        assert_eq!(redundant_region_count(&edges), PATCH_COUNT);
    }

    #[derive(Debug, Clone, Copy)]
    enum FramingTestView {
        Default,
        Rotated,
        CounterRotated,
        TopDown,
    }

    #[test]
    fn legacy_ring19_frame_breaches_margin_at_the_canonical_elevated_satellite() {
        let satellite = framing_fragment([(HexCoord::new_cubic(48, -54, 6), 60)]);
        let points = ring19_visual_points(&[satellite], 0.4);
        let legacy_hint = MapViewHint::new(
            (
                0.0,
                20.0_f32.mul_add(0.4, LEGACY_RING19_VIEW_FRAME * RING19_VIEW_EYE_UP),
                LEGACY_RING19_VIEW_FRAME * RING19_VIEW_EYE_BACK,
            ),
            (0.0, 20.0 * 0.4, 0.0),
        );
        let extent = maximum_projected_extent(&points, legacy_hint, FramingTestView::TopDown);

        assert!(
            extent > RING19_REVIEW_NDC_LIMIT,
            "the legacy fixed frame unexpectedly retained the intended review margin: {extent}"
        );
    }

    #[test]
    fn derived_ring19_frame_keeps_elevated_edges_and_world_bounds_inside_margin() {
        let fragment = framing_fragment([
            (HexCoord::ORIGIN, 21),
            (HexCoord::new_cubic(55, -55, 0), 21),
            (HexCoord::new_cubic(55, 0, -55), 21),
            (HexCoord::new_cubic(0, 55, -55), 21),
            (HexCoord::new_cubic(-55, 55, 0), 21),
            (HexCoord::new_cubic(-55, 0, 55), 21),
            (HexCoord::new_cubic(0, -55, 55), 21),
            (HexCoord::new_cubic(48, -54, 6), 60),
        ]);
        let points = ring19_visual_points(std::slice::from_ref(&fragment), 0.4);
        let hint = ring19_view_hint(&[fragment], 0.4).expect("representative bounds should frame");
        let (eye_x, eye_y, eye_z) = hint.eye;
        let (focus_x, focus_y, focus_z) = hint.focus;
        let distance = ReviewPoint::new(eye_x - focus_x, eye_y - focus_y, eye_z - focus_z).length();

        assert!(
            distance / RING19_VIEW_OFFSET_LENGTH > LEGACY_RING19_VIEW_FRAME,
            "the elevated outer terrain should require a wider-than-legacy frame"
        );
        for view in [
            FramingTestView::Default,
            FramingTestView::Rotated,
            FramingTestView::CounterRotated,
            FramingTestView::TopDown,
        ] {
            let extent = maximum_projected_extent(&points, hint, view);
            assert!(
                extent <= RING19_REVIEW_NDC_LIMIT + 0.000_1,
                "{view:?} projection reached {extent}, outside the intended review margin"
            );
        }
    }

    fn framing_fragment(
        columns: impl IntoIterator<Item = (HexCoord, Level)>,
    ) -> GeneratedPatchPlan {
        let columns = columns.into_iter().collect::<Vec<_>>();
        let mask = columns.iter().map(|(coord, _)| *coord).collect();
        let mut volume = VolumePlan::new(mask);
        for (coord, top) in columns {
            let column = volume
                .columns
                .get_mut(&coord)
                .expect("framing fixture coordinate should belong to its mask");
            column.elements = vec![VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(0, top),
                material: SolidMaterialRole::Stone,
                cutaway_for: None,
            })];
        }
        GeneratedPatchPlan {
            patch_id: PatchId(0),
            volume,
            liquids: LiquidPlan::default(),
            features: FeaturePlan::default(),
            structures: StructurePlan::default(),
            blockers: BTreeSet::new(),
            lights: BTreeMap::new(),
            biome_regions: BTreeMap::new(),
            interiors: InteriorPlan::default(),
            anchors: BTreeMap::new(),
            view_hint: MapViewHint::new((0.0, 1.0, 1.0), (0.0, 0.0, 0.0)),
        }
    }

    fn maximum_projected_extent(
        points: &[ReviewPoint],
        hint: MapViewHint,
        view: FramingTestView,
    ) -> f32 {
        points
            .iter()
            .copied()
            .flat_map(|point| {
                let (horizontal, vertical) = projected_ndc(point, hint, view);
                [horizontal.abs(), vertical.abs()]
            })
            .fold(0.0, f32::max)
    }

    fn projected_ndc(point: ReviewPoint, hint: MapViewHint, view: FramingTestView) -> (f32, f32) {
        let eye = ReviewPoint::from_tuple(hint.eye);
        let focus = ReviewPoint::from_tuple(hint.focus);
        let offset = eye.relative_to(focus);
        let (eye, requested_up) = match view {
            FramingTestView::Default => (eye, ReviewPoint::new(0.0, 1.0, 0.0)),
            FramingTestView::Rotated => (
                focus.add(offset.rotated_y(SQRT_THREE_OVER_TWO)),
                ReviewPoint::new(0.0, 1.0, 0.0),
            ),
            FramingTestView::CounterRotated => (
                focus.add(offset.rotated_y(-SQRT_THREE_OVER_TWO)),
                ReviewPoint::new(0.0, 1.0, 0.0),
            ),
            FramingTestView::TopDown => (
                focus.add(ReviewPoint::new(0.0, offset.length(), 0.0)),
                ReviewPoint::new(0.0, 0.0, -1.0),
            ),
        };
        let forward = focus.relative_to(eye).normalized();
        let right = forward.cross(requested_up).normalized();
        let up = right.cross(forward).normalized();
        let eye_relative = point.relative_to(eye);
        let depth = eye_relative.dot(forward);
        assert!(
            depth > 0.0,
            "framed terrain must remain ahead of the camera"
        );
        (
            eye_relative.dot(right)
                / (depth * RING19_REVIEW_TAN_HALF_VERTICAL_FOV * RING19_REVIEW_ASPECT_RATIO),
            eye_relative.dot(up) / (depth * RING19_REVIEW_TAN_HALF_VERTICAL_FOV),
        )
    }

    impl ReviewPoint {
        fn from_tuple((x, y, z): (f32, f32, f32)) -> Self {
            Self::new(x, y, z)
        }

        fn add(self, other: Self) -> Self {
            Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
        }

        fn cross(self, other: Self) -> Self {
            Self::new(
                self.y.mul_add(other.z, -(self.z * other.y)),
                self.z.mul_add(other.x, -(self.x * other.z)),
                self.x.mul_add(other.y, -(self.y * other.x)),
            )
        }

        fn length(self) -> f32 {
            self.dot(self).sqrt()
        }

        fn normalized(self) -> Self {
            let length = self.length();
            assert!(length > f32::EPSILON, "test vector must be non-degenerate");
            Self::new(self.x / length, self.y / length, self.z / length)
        }

        fn rotated_y(self, sine: f32) -> Self {
            let cosine: f32 = -0.5;
            Self::new(
                cosine.mul_add(self.x, sine * self.z),
                self.y,
                (-sine).mul_add(self.x, cosine * self.z),
            )
        }
    }

    #[test]
    fn physical_seam_resilience_accepts_a_connected_three_region_cycle() {
        let (ordinary, party, transitions, regions, expected) =
            physical_seam_cycle([0, 0, 0, 0, 0, 0]);
        let resilience =
            analyze_physical_seam_resilience(&ordinary, party, &transitions, &regions, &expected);

        assert_eq!(resilience.attached_seams, 3);
        assert!(resilience.unreachable_endpoints.is_empty());
        assert!(resilience.removal_failures.is_empty());
        assert_eq!(resilience.robust_regions, expected);
    }

    #[test]
    fn physical_seam_resilience_rejects_an_abstractly_redundant_split_region() {
        let (ordinary, party, transitions, regions, expected) =
            physical_seam_cycle([0, 0, 3, 3, 2, 1]);
        let intact = ordinary.distances_from(party);
        let resilience =
            analyze_physical_seam_resilience(&ordinary, party, &transitions, &regions, &expected);

        assert_eq!(intact.len(), 6, "every aperture endpoint begins reachable");
        assert_eq!(resilience.attached_seams, 3);
        assert!(resilience.unreachable_endpoints.is_empty());
        assert_eq!(
            resilience.robust_regions,
            BTreeSet::from([BiomeRegionId(0), BiomeRegionId(1)])
        );
        assert!(resilience.removal_failures.iter().any(|failure| {
            failure.edge == ResolvedEdgeId(2)
                && failure.missing_regions == [BiomeRegionId(2)]
                && !failure.stranded_endpoints.is_empty()
        }));
    }

    fn physical_seam_cycle(
        levels: [Level; 6],
    ) -> (
        OrdinaryGraph,
        TilePos,
        SeamTransitions,
        BTreeMap<TilePos, BiomeRegionId>,
        BTreeSet<BiomeRegionId>,
    ) {
        let coords = [
            HexCoord::new_cubic(1, -1, 0),
            HexCoord::new_cubic(1, 0, -1),
            HexCoord::new_cubic(0, 1, -1),
            HexCoord::new_cubic(-1, 1, 0),
            HexCoord::new_cubic(-1, 0, 1),
            HexCoord::new_cubic(0, -1, 1),
        ];
        let mut volume = VolumePlan::new(coords.into_iter().collect());
        let mut positions = Vec::new();
        for (coord, level) in coords.into_iter().zip(levels) {
            let position = TilePos::new(coord, level);
            positions.push(position);
            let previous = volume.columns.insert(
                coord,
                VolumeColumn {
                    elements: vec![VolumeElement::Solid(SolidMass {
                        levels: LevelInterval::new(0, level.saturating_add(1)),
                        material: SolidMaterialRole::Stone,
                        cutaway_for: None,
                    })],
                },
            );
            assert!(previous.is_some());
            assert!(volume
                .surfaces
                .insert(
                    position,
                    SurfaceMetadata {
                        access: SurfaceAccess::Ordinary,
                        interior: None,
                    },
                )
                .is_none());
        }
        let ordinary = OrdinaryGraph::from_volume(&volume, None);
        let [position_0, position_1, position_2, position_3, position_4, position_5] = positions
            .try_into()
            .expect("the six-coordinate fixture should produce six positions");
        let regions = BTreeMap::from([
            (position_0, BiomeRegionId(0)),
            (position_5, BiomeRegionId(0)),
            (position_1, BiomeRegionId(1)),
            (position_2, BiomeRegionId(1)),
            (position_3, BiomeRegionId(2)),
            (position_4, BiomeRegionId(2)),
        ]);
        let transitions = BTreeMap::from([
            (
                ResolvedEdgeId(0),
                BTreeSet::from([ordered_transition(position_0, position_1)]),
            ),
            (
                ResolvedEdgeId(1),
                BTreeSet::from([ordered_transition(position_2, position_3)]),
            ),
            (
                ResolvedEdgeId(2),
                BTreeSet::from([ordered_transition(position_4, position_5)]),
            ),
        ]);
        (
            ordinary,
            position_0,
            transitions,
            regions,
            BTreeSet::from([BiomeRegionId(0), BiomeRegionId(1), BiomeRegionId(2)]),
        )
    }

    #[test]
    fn tracked_settings_force_one_complete_seed_independent_fallback() {
        let first =
            generate_with_options(RING_RADIUS, 0.4, settings(), 1, runtime_art_catalog(), true)
                .expect("tracked Ring19 fallback should validate");
        let second = generate_with_options(
            RING_RADIUS,
            0.4,
            settings(),
            u64::MAX,
            runtime_art_catalog(),
            true,
        )
        .expect("tracked Ring19 fallback should ignore seed state");

        assert!(first.used_fallback);
        assert_eq!(first.selected_candidate, None);
        assert_eq!(first.valid_candidates, 0);
        assert_eq!(
            first.validated.semantic_fingerprint,
            second.validated.semantic_fingerprint
        );
        assert_eq!(first.metrics, second.metrics);
        assert_eq!(first.metrics.report.world_columns, WORLD_COLUMNS);
        assert_eq!(first.metrics.report.biome_regions, PATCH_COUNT);
        assert_eq!(first.metrics.report.reciprocal_seams, RECIPROCAL_SEAMS);
        assert_eq!(first.metrics.report.boundary_sides, BOUNDARY_SIDES);
        assert_eq!(
            first.metrics.report.directed_liquid_seams,
            INTERNAL_LIQUID_SEAMS
        );
        assert_eq!(
            first.metrics.report.boundary_liquid_outlets,
            BOUNDARY_LIQUID_OUTLETS
        );
        assert!(first.metrics.report.reachable_surfaces <= first.metrics.report.ordinary_surfaces);
        assert!(first.metrics.report.reachable_surfaces > 0);
        assert_eq!(first.metrics.report.redundant_regions, PATCH_COUNT);
        assert_ring19_mountain_hydrology(&first.validated.plan, true);
    }

    #[test]
    fn tracked_seed_selects_one_complete_candidate() {
        let first = generate(
            RING_RADIUS,
            0.4,
            settings(),
            1_592_598_566,
            runtime_art_catalog(),
        )
        .expect("tracked Ring19 seed should select a complete valid world");
        let repeated = generate(
            RING_RADIUS,
            0.4,
            settings(),
            1_592_598_566,
            runtime_art_catalog(),
        )
        .expect("repeated tracked Ring19 seed should select the same valid world");

        assert!(
            !first.used_fallback,
            "tracked Ring19 seed should not fall back: {:#?}",
            first.notes
        );
        assert_eq!(first.selected_candidate, Some(0));
        assert_eq!(first.candidates_evaluated, 8);
        assert!(first.valid_candidates > 0);
        assert_eq!(
            first.validated.plan.view_hint,
            MapViewHint::new((0.0, 168.96, 178.76), (0.0, 12.0, 0.0))
        );
        for (alias, source) in [
            (
                MOUNTAIN_WATERFALL_OVERLOOK,
                "mountains_a_stream_fall_overlook",
            ),
            (CONFLUENCE_OVERLOOK, "center_conflict_center"),
            (VEGETATION_GRADIENT_OVERLOOK, "forest_a_prairie_overlook"),
            (FORT_OUTLET_OVERLOOK, "waterfall_outlet_fall_overlook"),
        ] {
            assert_eq!(
                first.validated.plan.anchors.get(alias),
                first.validated.plan.anchors.get(source),
                "whole-world review alias {alias:?} must retain source anchor {source:?}"
            );
        }
        assert_eq!(
            first.validated.semantic_fingerprint,
            3_259_497_139_560_498_268
        );
        assert_eq!(first.selected_candidate, repeated.selected_candidate);
        assert_eq!(
            first.validated.semantic_fingerprint,
            repeated.validated.semantic_fingerprint
        );
        assert_eq!(first.metrics, repeated.metrics);
        assert_eq!(first.metrics.report.world_columns, WORLD_COLUMNS);
        assert_eq!(first.metrics.report.biome_regions, PATCH_COUNT);
        assert_eq!(first.metrics.report.redundant_regions, PATCH_COUNT);
        assert_ring19_mountain_hydrology(&first.validated.plan, false);
    }

    #[test]
    fn tracked_desert_oasis_rings_hero_seed_is_complete_connected_and_deterministic() {
        let map = desert_oasis_rings_map_settings();
        let TerrainSettings::Procedural(ProceduralSettings::V3(settings)) = &map.terrain else {
            panic!("tracked Desert Oasis Rings world should select procedural V3");
        };
        let first = generate(
            map.grid_radius,
            map.level_height,
            settings,
            1_592_598_566,
            runtime_art_catalog(),
        )
        .expect("tracked Desert Oasis Rings hero seed should generate a valid world");
        let repeated = generate(
            map.grid_radius,
            map.level_height,
            settings,
            1_592_598_566,
            runtime_art_catalog(),
        )
        .expect("repeated Desert Oasis Rings hero seed should generate the same valid world");

        assert!(
            !first.used_fallback,
            "tracked Desert Oasis Rings hero seed should not fall back: {:#?}",
            first.notes
        );
        assert_eq!(first.metrics.report.world_columns, WORLD_COLUMNS);
        assert_eq!(first.metrics.report.biome_regions, PATCH_COUNT);
        assert_eq!(first.metrics.report.reciprocal_seams, RECIPROCAL_SEAMS);
        assert_eq!(
            first.validated.plan.layout.patches.len(),
            PATCH_COUNT as usize
        );
        assert_eq!(
            first.validated.plan.layout.shared_edges.len(),
            RECIPROCAL_SEAMS as usize
        );
        assert!(first
            .validated
            .plan
            .layout
            .shared_edges
            .values()
            .all(|edge| matches!(edge.liquid, ResolvedLiquidPort::Dry)));
        assert!(first
            .validated
            .plan
            .layout
            .boundary_liquid_outlets
            .is_empty());
        assert_eq!(first.metrics.report.directed_liquid_seams, 0);
        assert_eq!(first.metrics.report.boundary_liquid_outlets, 0);

        let palms = first
            .validated
            .plan
            .features
            .by_id
            .values()
            .filter(|feature| feature.object_id.as_str() == desert_vegetation::DATE_PALM_ID)
            .collect::<Vec<_>>();
        assert_eq!(
            palms.len(),
            12,
            "the center oasis should own twelve date palms"
        );
        assert!(palms.iter().all(|feature| {
            first.validated.plan.biome_regions.get(&feature.root) == Some(&BiomeRegionId(0))
        }));

        for (alias, source) in [
            (PARTY_START, "center_party_start"),
            (HOSTILE_START, "mountains_c_hostile_start"),
            (CONFLICT_CENTER, "center_oasis_overlook"),
            (OASIS_OVERLOOK, "center_oasis_overlook"),
            (INNER_DUNE_CREST, "frozen_hills_dune_crest"),
            (OUTER_DUNE_CREST, "sky_islands_dune_crest"),
            (DESERT_PLAIN_OVERLOOK, "deep_forest_a_desert_plain_overlook"),
        ] {
            assert_eq!(
                first.validated.plan.anchors.get(alias),
                first.validated.plan.anchors.get(source),
                "whole-world desert alias {alias:?} must retain source anchor {source:?}"
            );
        }

        let ordinary = OrdinaryGraph::from_volume(
            &first.validated.plan.volume,
            Some(&first.validated.plan.blockers),
        );
        let party = first
            .validated
            .plan
            .anchors
            .get(PARTY_START)
            .copied()
            .expect("the validated desert world publishes party_start");
        let reachable = ordinary.distances_from(party);
        assert_eq!(
            reachable.len(),
            ordinary.len(),
            "every unblocked ordinary Desert Oasis Rings surface should share one global network"
        );
        assert_eq!(
            first.metrics.report.reachable_surfaces,
            first.metrics.report.ordinary_surfaces
        );

        assert_eq!(first.selected_candidate, repeated.selected_candidate);
        assert_eq!(
            first.validated.semantic_fingerprint,
            repeated.validated.semantic_fingerprint
        );
        assert_eq!(first.metrics, repeated.metrics);
    }

    #[test]
    #[ignore = "manual release-mode Ring7/Ring19 generation benchmark"]
    fn ring19_generation_p95_stays_within_three_and_a_half_times_ring7() {
        require_release_benchmark();

        const WARMUP_RUNS: usize = 2;
        const SAMPLE_COUNT: usize = 20;
        const RING7_REVIEW_SEED: u64 = 703_700_113;
        const RING19_REVIEW_SEED: u64 = 1_592_598_566;

        let ring7_map = seven_regions_map_settings();
        let ring19_map = two_rings_map_settings();
        let TerrainSettings::Procedural(ProceduralSettings::V3(ring7_settings)) =
            &ring7_map.terrain
        else {
            panic!("tracked Seven Regions world should select procedural V3");
        };
        let generate_ring7 = || {
            super::super::ring7::generate(
                ring7_map.grid_radius,
                ring7_map.level_height,
                ring7_settings,
                RING7_REVIEW_SEED,
                runtime_art_catalog(),
            )
            .expect("canonical Seven Regions generation should succeed")
        };
        let generate_ring19 = || {
            generate(
                ring19_map.grid_radius,
                ring19_map.level_height,
                settings(),
                RING19_REVIEW_SEED,
                runtime_art_catalog(),
            )
            .expect("canonical Two Rings generation should succeed")
        };

        for _ in 0..WARMUP_RUNS {
            std::hint::black_box(generate_ring7());
            std::hint::black_box(generate_ring19());
        }

        let mut ring7_samples = Vec::with_capacity(SAMPLE_COUNT);
        let mut ring19_samples = Vec::with_capacity(SAMPLE_COUNT);
        for sample in 0..SAMPLE_COUNT {
            if sample.is_multiple_of(2) {
                ring7_samples.push(measure_generation(generate_ring7));
                ring19_samples.push(measure_generation(generate_ring19));
            } else {
                ring19_samples.push(measure_generation(generate_ring19));
                ring7_samples.push(measure_generation(generate_ring7));
            }
        }
        ring7_samples.sort_unstable();
        ring19_samples.sort_unstable();

        let ring7_median = sample_median(&ring7_samples);
        let ring7_p95 = sample_p95(&ring7_samples);
        let ring19_median = sample_median(&ring19_samples);
        let ring19_p95 = sample_p95(&ring19_samples);
        eprintln!(
            "Ring generation release benchmark ({SAMPLE_COUNT} samples, {WARMUP_RUNS} warm-ups): \
             Ring7 median={ring7_median:?} p95={ring7_p95:?}; \
             Ring19 median={ring19_median:?} p95={ring19_p95:?}"
        );

        assert!(
            ring19_p95.as_nanos().saturating_mul(2) <= ring7_p95.as_nanos().saturating_mul(7),
            "Ring19 p95 {ring19_p95:?} exceeded 3.5x Ring7 p95 {ring7_p95:?}"
        );
    }

    #[cfg(debug_assertions)]
    fn require_release_benchmark() {
        panic!(
            "run this manual gate with `cargo test --release -p hex_map \
             procedural_v3::ring19::tests::ring19_generation_p95_stays_within_three_and_a_half_times_ring7 \
             -- --ignored --exact --nocapture`"
        );
    }

    #[cfg(not(debug_assertions))]
    fn require_release_benchmark() {}

    fn measure_generation<T>(generate: impl FnOnce() -> T) -> Duration {
        let started = Instant::now();
        let generated = std::hint::black_box(generate());
        let elapsed = started.elapsed();
        std::hint::black_box(generated);
        elapsed
    }

    fn sample_median(samples: &[Duration]) -> Duration {
        samples
            .get(samples.len() / 2)
            .copied()
            .expect("benchmark should record samples")
    }

    fn sample_p95(samples: &[Duration]) -> Duration {
        let rank = samples
            .len()
            .saturating_mul(95)
            .div_ceil(100)
            .saturating_sub(1);
        samples
            .get(rank)
            .copied()
            .expect("benchmark should record samples")
    }

    fn assert_ring19_mountain_hydrology(
        plan: &GeneratedWorldPlan,
        require_translated_high_streams: bool,
    ) {
        let water = plan
            .liquids
            .bodies
            .values()
            .filter(|body| body.material == FillMaterialRole::Water)
            .flat_map(|body| body.nodes.iter().map(|(position, node)| (*position, *node)))
            .collect::<BTreeMap<_, _>>();
        let downstream_targets = water
            .values()
            .filter_map(|node| node.downstream)
            .collect::<BTreeSet<_>>();
        let mut normalized_high_patches = Vec::new();

        for (patch_id, route_prefix) in [(PatchId(16), "mountains_a"), (PatchId(17), "mountains_b")]
        {
            let patch = PatchRecipeContext::resolve(&plan.layout, patch_id)
                .expect("tracked Ring19 mountain patch should resolve");
            let outgoing = patch
                .shared_edges()
                .filter_map(|edge| {
                    edge.liquid_port()
                        .filter(|liquid| liquid.is_source)
                        .map(|liquid| (liquid, edge.side))
                })
                .collect::<Vec<_>>();
            let [(liquid, _side)] = outgoing.as_slice() else {
                panic!("high Ring19 mountain should have one directed outlet");
            };
            assert_eq!(
                liquid.elevation,
                ResolvedLiquidElevation::Exact(29),
                "high Ring19 mountain should retain its exact handoff level"
            );
            let patch_water = water
                .keys()
                .filter(|position| patch.mask().contains(&position.coord))
                .copied()
                .collect::<BTreeSet<_>>();
            let patch_water_coords = patch_water
                .iter()
                .map(|position| position.coord)
                .collect::<BTreeSet<_>>();
            assert!(
                liquid.port.first_approach.is_subset(&patch_water_coords),
                "high Ring19 mountain water should exactly occupy every authored approach lane"
            );
            let expected_outlets = liquid
                .port
                .lanes
                .iter()
                .map(|(local, _)| TilePos::new(*local, 29))
                .collect::<BTreeSet<_>>();
            assert!(
                expected_outlets.is_subset(&patch_water),
                "high Ring19 mountain should terminate on all three exact source-side lanes"
            );

            let route_coords = ["high_pass", "lower_bypass"]
                .into_iter()
                .flat_map(|route| {
                    plan.features
                        .protected_routes
                        .get(&format!("{route_prefix}_{route}"))
                        .expect("tracked mountain route should be namespaced")
                        .surfaces
                        .iter()
                        .map(|surface| surface.coord)
                })
                .collect::<BTreeSet<_>>();
            assert!(
                patch_water_coords.is_disjoint(&route_coords),
                "high Ring19 mountain streams should not consume protected route surfaces"
            );

            let roots = patch_water
                .iter()
                .filter(|position| !downstream_targets.contains(position))
                .copied()
                .collect::<Vec<_>>();
            assert_eq!(
                roots.len(),
                3,
                "high Ring19 mountain should have three springs"
            );
            let mut occupied = BTreeSet::new();
            for root in &roots {
                let mut current = *root;
                let mut visited = BTreeSet::new();
                while patch.mask().contains(&current.coord) {
                    assert!(
                        visited.insert(current.coord),
                        "mountain stream should not cycle inside its source patch"
                    );
                    assert!(
                        occupied.insert(current.coord),
                        "mountain streams should be pairwise vertex-disjoint"
                    );
                    let Some(next) = water.get(&current).and_then(|node| node.downstream) else {
                        break;
                    };
                    current = next;
                }
            }
            assert!(
                liquid.port.first_approach.is_subset(&occupied),
                "the three spring paths should jointly cover the full authored approach"
            );

            let maximum_surface = plan
                .volume
                .surfaces
                .keys()
                .filter(|position| patch.mask().contains(&position.coord))
                .map(|position| position.level)
                .max()
                .expect("mountain patch should publish surfaces");
            let summits = plan
                .volume
                .surfaces
                .keys()
                .filter_map(|position| {
                    (patch.mask().contains(&position.coord) && position.level == maximum_surface)
                        .then_some(position.coord)
                })
                .collect::<BTreeSet<_>>();
            let source_summits = roots
                .iter()
                .filter_map(|root| {
                    summits
                        .iter()
                        .copied()
                        .min_by_key(|summit| (summit.distance(root.coord), *summit))
                })
                .collect::<BTreeSet<_>>();
            assert_eq!(
                source_summits.len(),
                1,
                "all high Ring19 mountain springs should draw from one summit"
            );
            assert!(
                roots.iter().all(|root| {
                    source_summits
                        .first()
                        .is_some_and(|summit| summit.distance(root.coord) <= 2)
                }),
                "every high Ring19 mountain spring should begin in the near-summit band"
            );

            let center =
                super::super::layout::ring19_patch_center(patch_id).expect("fixed patch center");
            let [center_x, center_y, center_z] = center.to_cubic_array();
            normalized_high_patches.push(
                patch_water
                    .iter()
                    .map(|position| {
                        let [x, y, z] = position.coord.to_cubic_array();
                        ([x - center_x, y - center_y, z - center_z], position.level)
                    })
                    .collect::<BTreeSet<_>>(),
            );
        }
        if require_translated_high_streams {
            let [mountains_a, mountains_b] = normalized_high_patches.as_slice() else {
                panic!("tracked Ring19 should contain two high mountain sources");
            };
            assert_eq!(
                mountains_a, mountains_b,
                "canonical high mountain patches should realize identical translated streams"
            );
        }

        let patch = PatchRecipeContext::resolve(&plan.layout, PatchId(18))
            .expect("tracked low Ring19 mountain patch should resolve");
        let outgoing = patch
            .shared_edges()
            .filter_map(|edge| edge.liquid_port().filter(|liquid| liquid.is_source))
            .collect::<Vec<_>>();
        let [liquid] = outgoing.as_slice() else {
            panic!("low Ring19 mountain should have one directed outlet");
        };
        assert_eq!(liquid.elevation, ResolvedLiquidElevation::Exact(16));
        for (local, _) in &liquid.port.lanes {
            assert!(
                water.contains_key(&TilePos::new(*local, 16)),
                "low Ring19 mountain should preserve its exact level-16 handoff"
            );
        }
    }

    fn seven_regions_map_settings() -> &'static MapSettings {
        static SETTINGS: OnceLock<MapSettings> = OnceLock::new();
        SETTINGS.get_or_init(|| {
            ron::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/config/worlds/procedural-ring7.ron"
            )))
            .expect("tracked Seven Regions world should parse")
        })
    }

    fn two_rings_map_settings() -> &'static MapSettings {
        static SETTINGS: OnceLock<MapSettings> = OnceLock::new();
        SETTINGS.get_or_init(|| {
            ron::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/config/worlds/procedural-two-rings.ron"
            )))
            .expect("tracked Two Rings world should parse")
        })
    }

    fn desert_oasis_rings_map_settings() -> &'static MapSettings {
        static SETTINGS: OnceLock<MapSettings> = OnceLock::new();
        SETTINGS.get_or_init(|| {
            ron::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/config/worlds/procedural-desert-oasis-rings.ron"
            )))
            .expect("tracked Desert Oasis Rings world should parse")
        })
    }

    fn settings() -> &'static ProceduralV3Settings {
        let TerrainSettings::Procedural(ProceduralSettings::V3(settings)) =
            &two_rings_map_settings().terrain
        else {
            panic!("tracked Two Rings world should select procedural V3");
        };
        settings
    }

    fn desert_settings() -> ProceduralV3Settings {
        let regions = (0..V3_RING19_REGION_COUNT)
            .map(|index| {
                let (recipe, rotation_turns) = match index {
                    0 => (
                        V3RecipeSettings::Oasis(V3OasisSettings {
                            base_level: 15,
                            pool_radius: 5,
                            palm_count: 12,
                            grass_ring_width: 3,
                        }),
                        0,
                    ),
                    1..=6 => (
                        V3RecipeSettings::Dunes(V3DunesSettings {
                            base_level: 15,
                            ridge_height: 4,
                            ridge_spacing: 10,
                            ridge_count: 3,
                        }),
                        u8::try_from(index - 1).expect("inner-ring rotation should fit u8"),
                    ),
                    7 | 9 | 11 | 13 | 15 | 17 => (
                        V3RecipeSettings::Dunes(V3DunesSettings {
                            base_level: 15,
                            ridge_height: 6,
                            ridge_spacing: 12,
                            ridge_count: 4,
                        }),
                        u8::try_from((index - 7) / 2).expect("outer-ring rotation should fit u8"),
                    ),
                    8 | 10 | 12 | 14 | 16 | 18 => (
                        V3RecipeSettings::DesertPlain(V3DesertPlainSettings {
                            base_level: 15,
                            max_relief: 2,
                        }),
                        0,
                    ),
                    _ => unreachable!("Ring19 fixture index is bounded"),
                };
                Ring19RegionSettings {
                    environment: V3EnvironmentSettings::Arid,
                    recipe,
                    overlays: Vec::new(),
                    rotation_turns,
                }
            })
            .collect();
        ProceduralV3Settings {
            layout: V3LayoutSettings::Ring19(V3Ring19Settings {
                profile: V3Ring19ProfileSettings::DesertOasis,
                regions,
                seam_defaults: SharedEdgeSettings {
                    elevation: EdgeElevationSettings {
                        preferred: 17,
                        min: 15,
                        max: 19,
                    },
                    walker: WalkerPortSettings { count: 2, width: 2 },
                    liquid: EdgeLiquidSettings::Dry,
                    approach_depth: 3,
                },
                liquid_connections: Vec::new(),
                boundary_outlets: Vec::new(),
            }),
        }
    }

    fn runtime_art_catalog() -> &'static RuntimeArtCatalog {
        super::super::vegetation::tests::runtime_art_catalog()
    }
}
