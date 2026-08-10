//! Deterministic authored Crystal Ascent landmark.
//!
//! The landmark is a true stacked V3 volume: the chamber remains open, stair
//! treads sit on radial stone haunches, and the shell/crown are separate occupied
//! runs. Only crystal silhouettes and summit trees vary with the seed.

use std::collections::{BTreeMap, BTreeSet};

use hex_assets::{HexObjectRotation, RuntimeArtCatalog};
use hex_core::{
    upper_dome_contains, ExactGridPoint, HexCoord, IlluminationLevel, InteriorRegionId, Level,
    MapViewHint, SpecialMovementRegion, TilePos,
};

use super::composition::{compose_single_patch, GeneratedPatchPlan};
use super::layout::{resolve_layout, LayoutKind, PatchId, ResolvedLayoutPlan};
use super::patch::{PatchBuildMode, PatchRecipeContext};
use super::seed::SeedStream;
use super::selection::{
    run_recipe, CandidateAttemptError, CandidateContext, FallbackContext, RepairOutcome, V3Recipe,
    ValidatedWorldSelection, WorldValidation,
};
use super::traversal::OrdinaryGraph;
use super::vegetation::TemperateTreeSet;
use super::volume::{
    LevelInterval, SolidMass, SolidMaterialRole, SurfaceAccess, SurfaceMetadata, VolumeColumn,
    VolumeElement, VolumePlan,
};
use super::world::{
    CaveCrystalKind, CrystalAscentCrystalKind, CrystalAscentCrystalPresentation, FeatureClearing,
    FeatureId, FeatureKind, FeaturePlan, GeneratedWorldPlan, InteriorPlan, LightId, PlannedFeature,
    PlannedGameplayLight, PlannedInterior, PlannedLightPresentation, PlannedStructure,
    ProtectedFeatureRoute, StructureId, StructureKind, StructurePlan, WorldIssueCode,
    WorldValidationIssue,
};
use super::{CrystalAscentObjectSet, V3GenerationError};
use crate::settings::{
    ProceduralV3Settings, V3CrystalAscentSettings, V3EnvironmentSettings, V3LayoutSettings,
    V3RecipeSettings,
};

const SITE_RADIUS: u32 = 32;
const CHAMBER_RADIUS: u32 = 23;
const OCULUS_RADIUS: u32 = 11;
const CLEARING_RADIUS: u32 = 18;
const SHELL_INNER_RADIUS: u32 = 28;
const SHELL_OUTER_RADIUS: u32 = 32;
const BUTTRESS_INNER_RADIUS: u32 = 30;
const BUTTRESS_OUTER_RADIUS: u32 = SITE_RADIUS;
const BUTTRESS_FLANK_OFFSET: i32 = 8;
const CIRCUIT_BANDS: [(u32, u32); 3] = [(24, 27), (21, 24), (18, 21)];
const CIRCUIT_COUNT: usize = 3;
const FLIGHTS_PER_CIRCUIT: usize = 6;
const FLIGHT_COUNT: usize = CIRCUIT_COUNT * FLIGHTS_PER_CIRCUIT;
const LANDING_COUNT: usize = FLIGHT_COUNT;
const LANDING_DIM_RADIUS: u32 = 18;
const LANDING_BRIGHT_RADIUS: u32 = 4;
const HEART_DIM_RADIUS: u32 = 24;
const HEART_BRIGHT_RADIUS: u32 = 8;
const INTERIOR: InteriorRegionId = InteriorRegionId(40);
const SHELL_TOPS: SpecialMovementRegion = SpecialMovementRegion(40);

const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";
const LOWER_ENTRY: &str = "crystal_ascent.lower_entry";
const BOTTOM_CHAMBER: &str = "crystal_ascent.bottom_chamber";
const UPPER_EXIT: &str = "crystal_ascent.upper_exit";
const LOWER_TERMINAL: &str = "crystal_ascent.lower_terminal_pad";
const UPPER_TERMINAL: &str = "crystal_ascent.upper_terminal_pad";
const EXIT_TRAIL: &str = "crystal_ascent.summit_exit_trail";
const SUMMIT_CLEARING: &str = "crystal_ascent.summit_clearing";
const MID_FLIGHT: &str = "crystal_ascent.mid_flight";
const CORNER_LANDING: &str = "crystal_ascent.corner_landing";
const UPPER_CONTRACTION: &str = "crystal_ascent.upper_contraction";

/// Deterministic diagnostics for selection, reports, and acceptance tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CrystalAscentMetrics {
    pub(crate) circuits: u32,
    pub(crate) flights: u32,
    pub(crate) landings: u32,
    pub(crate) stair_surfaces: u32,
    pub(crate) chamber_surfaces: u32,
    pub(crate) crown_surfaces: u32,
    pub(crate) tree_roots: u32,
    pub(crate) crystal_fixtures: u32,
    pub(crate) gameplay_lights: u32,
    pub(crate) ordinary_surfaces: u32,
    pub(crate) reachable_elevation_levels: u32,
    pub(crate) critical_route_steps: u32,
    pub(crate) rise_levels: Level,
    pub(crate) minimum_stair_headroom: Level,
}

#[derive(Debug)]
struct CrystalAscentRecipe {
    level_height: f32,
    layout: ResolvedLayoutPlan,
    settings: V3CrystalAscentSettings,
    trees: TemperateTreeSet,
    objects: CrystalAscentObjectSet,
}

#[derive(Debug, Clone, Copy)]
struct CrystalAscentStreams<'a> {
    crystal_kinds: SeedStream<'a>,
    crystal_rotations: SeedStream<'a>,
    tree_sites: SeedStream<'a>,
    tree_rotations: SeedStream<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MassSpec {
    bottom: Level,
    top: Level,
    material: SolidMaterialRole,
}

#[derive(Debug)]
struct AuthoredGeometry {
    stair_surfaces: BTreeSet<TilePos>,
    circuit_surfaces: Vec<BTreeSet<TilePos>>,
    crown_surfaces: BTreeSet<TilePos>,
    landing_alcoves: Vec<TilePos>,
    lower_pad: BTreeSet<TilePos>,
    upper_pad: BTreeSet<TilePos>,
    summit_trail: BTreeSet<TilePos>,
    summit_clearing: BTreeSet<TilePos>,
    lower_entry: TilePos,
    bottom_chamber: TilePos,
    upper_exit: TilePos,
    mid_flight: TilePos,
    corner_landing: TilePos,
    upper_contraction: TilePos,
}

/// Runs the common V3 selector for one standalone Crystal Ascent world.
pub(crate) fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
    art_catalog: &RuntimeArtCatalog,
) -> Result<ValidatedWorldSelection<CrystalAscentMetrics>, V3GenerationError> {
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V3GenerationError::RecipeContract(
            "Crystal Ascent level height must be positive and finite".to_owned(),
        ));
    }
    let recipe_settings = validate_recipe_settings(settings)?;
    let objects = CrystalAscentObjectSet::resolve(art_catalog).map_err(|error| {
        V3GenerationError::RecipeContract(format!(
            "Crystal Ascent authored object preflight failed: {error}"
        ))
    })?;
    let trees = TemperateTreeSet::resolve(art_catalog, "Crystal Ascent")
        .map_err(V3GenerationError::RecipeContract)?;
    let layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    run_recipe(
        &CrystalAscentRecipe {
            level_height,
            layout,
            settings: recipe_settings,
            trees,
            objects,
        },
        settings,
        grid_radius,
        seed,
    )
}

impl V3Recipe for CrystalAscentRecipe {
    type Settings = ProceduralV3Settings;
    type Metrics = CrystalAscentMetrics;
    type Score = (u32, u8);

    fn construct(
        &self,
        context: CandidateContext,
        settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, CandidateAttemptError> {
        validate_recipe_settings(settings).map_err(CandidateAttemptError::Fatal)?;
        if context.grid_radius != self.layout.grid_radius {
            return Err(CandidateAttemptError::Fatal(
                V3GenerationError::RecipeContract(
                    "Crystal Ascent candidate radius disagrees with its resolved layout".to_owned(),
                ),
            ));
        }
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))
            .map_err(CandidateAttemptError::Fatal)?;
        let fragment = construct_patch(
            patch,
            &self.settings,
            self.level_height,
            PatchBuildMode::Candidate {
                world_seed: context.seed,
                candidate: context.candidate,
            },
            &self.trees,
            &self.objects,
        )
        .map_err(CandidateAttemptError::Rejected)?;
        compose_single_patch(self.layout.clone(), fragment).map_err(|error| {
            CandidateAttemptError::Fatal(V3GenerationError::RecipeContract(format!(
                "Crystal Ascent single-patch composition failed: {error:?}"
            )))
        })
    }

    fn validate(
        &self,
        _settings: &Self::Settings,
        plan: &GeneratedWorldPlan,
    ) -> WorldValidation<Self::Metrics> {
        validate_crystal_ascent(plan, &self.settings, &self.objects)
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
        (metrics.tree_roots.abs_diff(42), candidate)
    }

    fn canonical_fallback(
        &self,
        context: FallbackContext,
        settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, V3GenerationError> {
        validate_recipe_settings(settings)?;
        if context.grid_radius != self.layout.grid_radius {
            return Err(V3GenerationError::RecipeContract(
                "Crystal Ascent fallback radius disagrees with its resolved layout".to_owned(),
            ));
        }
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))?;
        let fragment = construct_patch(
            patch,
            &self.settings,
            self.level_height,
            PatchBuildMode::CanonicalFallback,
            &self.trees,
            &self.objects,
        )
        .map_err(|issues| {
            V3GenerationError::RecipeContract(
                issues
                    .into_iter()
                    .map(|issue| issue.detail)
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        })?;
        compose_single_patch(self.layout.clone(), fragment).map_err(|error| {
            V3GenerationError::RecipeContract(format!(
                "Crystal Ascent fallback composition failed: {error:?}"
            ))
        })
    }
}

fn validate_recipe_settings(
    settings: &ProceduralV3Settings,
) -> Result<V3CrystalAscentSettings, V3GenerationError> {
    let V3LayoutSettings::Single(patch) = &settings.layout else {
        return Err(V3GenerationError::RecipeUnavailable("CrystalAscent Single"));
    };
    if patch.environment != V3EnvironmentSettings::TemperateGrassland {
        return Err(V3GenerationError::RecipeContract(
            "Crystal Ascent requires the TemperateGrassland environment".to_owned(),
        ));
    }
    let V3RecipeSettings::CrystalAscent(recipe) = patch.recipe else {
        return Err(V3GenerationError::RecipeUnavailable("CrystalAscent"));
    };
    if !patch.overlays.is_empty() {
        return Err(V3GenerationError::RecipeContract(
            "Crystal Ascent overlays are not implemented".to_owned(),
        ));
    }
    Ok(recipe)
}

pub(crate) fn construct_patch(
    patch: PatchRecipeContext<'_>,
    settings: &V3CrystalAscentSettings,
    level_height: f32,
    mode: PatchBuildMode,
    trees: &TemperateTreeSet,
    objects: &CrystalAscentObjectSet,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    let streams = mode.seed_streams(&patch);
    let streams = streams.map(|streams| CrystalAscentStreams {
        crystal_kinds: streams.stage("crystal_ascent.crystals.kind"),
        crystal_rotations: streams.stage("crystal_ascent.crystals.rotation"),
        tree_sites: streams.stage("crystal_ascent.summit.trees"),
        tree_rotations: streams.stage("crystal_ascent.summit.tree_rotation"),
    });
    construct_patch_with_streams(patch, settings, level_height, streams, trees, objects)
}

fn construct_patch_with_streams(
    patch: PatchRecipeContext<'_>,
    settings: &V3CrystalAscentSettings,
    level_height: f32,
    streams: Option<CrystalAscentStreams<'_>>,
    trees: &TemperateTreeSet,
    objects: &CrystalAscentObjectSet,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    let frame = patch
        .local_frame()
        .map_err(|error| vec![recipe_issue(error.to_string())])?;
    let local_mask = frame
        .local_mask(patch.mask())
        .map_err(|error| vec![recipe_issue(error)])?;
    let required_site = HexCoord::ORIGIN
        .within_radius(SITE_RADIUS)
        .into_iter()
        .collect::<BTreeSet<_>>();
    if !required_site.is_subset(&local_mask) {
        return Err(vec![recipe_issue(format!(
            "Crystal Ascent requires an unobstructed radius-{SITE_RADIUS} authored site"
        ))]);
    }

    let base = settings.base_level;
    let summit = base.saturating_add(settings.rise_levels);
    let composite = patch.layout().kind == LayoutKind::Macro;
    let mut masses = local_mask
        .iter()
        .copied()
        .map(|coord| (coord, Vec::<MassSpec>::new()))
        .collect::<BTreeMap<_, _>>();
    let mut surface_intents = BTreeMap::<TilePos, SurfaceMetadata>::new();

    for coord in &local_mask {
        let radius = coord.distance(HexCoord::ORIGIN);
        if radius > SITE_RADIUS {
            // A Macro claim may retain a few central-cell fringe columns beyond
            // the exact radius-32 authored site. Those columns belong to the
            // high woodland crown, not the standalone level-six showcase apron.
            // Standalone generation keeps its historical base-level grass.
            let fringe_level = if composite { summit } else { base };
            add_ground(&mut masses, *coord, fringe_level, SolidMaterialRole::Grass);
            surface_intents.insert(TilePos::new(*coord, fringe_level), exterior_surface());
        } else if radius <= CHAMBER_RADIUS
            || (radius < SHELL_INNER_RADIUS && is_lower_connector(*coord))
            || (radius >= SHELL_INNER_RADIUS && is_lower_aperture(*coord))
        {
            add_ground(&mut masses, *coord, base, SolidMaterialRole::WorkedStone);
            surface_intents.insert(TilePos::new(*coord, base), interior_surface());
        }
    }

    let mut geometry = build_stairs(&mut masses, &mut surface_intents, settings)?;
    build_shell(
        &mut masses,
        &mut surface_intents,
        &geometry.landing_alcoves,
        base,
        summit,
    )?;
    // The protected terminal is the exterior apron at the mouth of the arch.
    // The Dark interior begins one ring inward so a party spawned on the terminal
    // remains in exterior daylight while crossing the physical threshold.
    for position in &geometry.lower_pad {
        surface_intents.insert(*position, exterior_surface());
    }
    build_buttresses_and_ribs(&mut masses, &mut surface_intents, settings)?;
    let stair_surfaces = geometry.stair_surfaces.clone();
    build_crown(
        &mut masses,
        &mut surface_intents,
        &stair_surfaces,
        summit,
        &mut geometry,
    );

    let mut volume = finish_volume(local_mask.clone(), masses, surface_intents)?;
    let roof_voxels = tag_interior_shell_cutaway(&mut volume, base);
    let features = build_features(&volume, &geometry, streams, trees)?;
    let mut blockers = features
        .by_id
        .values()
        .flat_map(|feature| feature.blocker_footprint.iter().copied())
        .collect::<BTreeSet<_>>();
    let heart_visual_origin = TilePos::new(
        HexCoord::ORIGIN,
        base.checked_add(1).ok_or_else(|| {
            vec![recipe_issue(
                "cathedral heart visual origin overflows the authored level range",
            )]
        })?,
    );
    let heart_rotation = HexObjectRotation::new(0).map_err(|error| {
        vec![recipe_issue(format!(
            "cathedral heart rotation preflight failed: {error}"
        ))]
    })?;
    let heart_blockers = objects
        .project_heart_traversal_blockers(
            volume.surfaces.keys().copied(),
            heart_visual_origin,
            heart_rotation,
        )
        .ok_or_else(|| {
            vec![recipe_issue(
                "cathedral heart exact movement projection overflowed",
            )]
        })?;
    blockers.extend(heart_blockers);
    let lights = build_lights(&geometry, base, streams);
    let structures = build_structures(&volume, &geometry);
    let interior_floors = volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| {
            (metadata.interior == Some(INTERIOR)).then_some(*position)
        })
        .collect::<BTreeSet<_>>();
    let entrance_surfaces = radial_pad(SITE_RADIUS.saturating_sub(1), 0, 4, base)
        .into_iter()
        .filter(|position| interior_floors.contains(position))
        .collect();
    let interiors = InteriorPlan {
        by_id: BTreeMap::from([(
            INTERIOR,
            PlannedInterior {
                floors: interior_floors,
                entrances: entrance_surfaces,
                roof_voxels,
            },
        )]),
    };
    let anchors = BTreeMap::from([
        (PARTY_START.to_owned(), geometry.lower_entry),
        (HOSTILE_START.to_owned(), geometry.upper_exit),
        (LOWER_ENTRY.to_owned(), geometry.lower_entry),
        (BOTTOM_CHAMBER.to_owned(), geometry.bottom_chamber),
        (UPPER_EXIT.to_owned(), geometry.upper_exit),
        (MID_FLIGHT.to_owned(), geometry.mid_flight),
        (CORNER_LANDING.to_owned(), geometry.corner_landing),
        (UPPER_CONTRACTION.to_owned(), geometry.upper_contraction),
    ]);
    let biome_regions = volume
        .surfaces
        .keys()
        .copied()
        .map(|surface| (surface, patch.biome_region()))
        .collect();
    let view_hint = ascent_view_hint(level_height, base, summit);
    let mut fragment = GeneratedPatchPlan {
        patch_id: patch.id,
        volume,
        liquids: Default::default(),
        features,
        structures,
        blockers,
        lights,
        biome_regions,
        interiors,
        anchors,
        view_hint,
    };
    frame
        .patch_to_world(&mut fragment)
        .map_err(|error| vec![recipe_issue(error)])?;
    let issues = fragment
        .validate_against(patch.layout())
        .into_iter()
        .map(|issue| {
            recipe_issue(format!(
                "patch {:?} failed {:?}: {}",
                issue.patch, issue.code, issue.detail
            ))
        })
        .collect::<Vec<_>>();
    if issues.is_empty() {
        Ok(fragment)
    } else {
        Err(issues)
    }
}

fn expected_circuit_surfaces(
    settings: &V3CrystalAscentSettings,
    circuit: usize,
) -> Option<BTreeSet<TilePos>> {
    let (inner, outer) = CIRCUIT_BANDS.get(circuit).copied()?;
    let mut surfaces = BTreeSet::new();
    for radius in inner..=outer {
        let radius_usize = usize::try_from(radius).ok()?;
        let ring = ring_coordinates(radius);
        if ring.is_empty() || radius_usize == 0 {
            return None;
        }
        let start = start_index(radius);
        for progress in 0..ring.len() {
            let raw = (start + progress) % ring.len();
            let side = progress / radius_usize;
            let offset = progress % radius_usize;
            let flight = circuit
                .checked_mul(FLIGHTS_PER_CIRCUIT)?
                .checked_add(side)?;
            let boundary_low = flight_boundary(settings.base_level, settings.rise_levels, flight);
            let boundary_high =
                flight_boundary(settings.base_level, settings.rise_levels, flight + 1);
            let level =
                flight_tread_level(circuit, side, radius, offset, boundary_low, boundary_high)?;
            surfaces.insert(TilePos::new(*ring.get(raw)?, level));
        }
    }

    // Each contraction is a four-wide radial transfer corridor. The outgoing and
    // incoming circuits own one four-by-four turning pad apiece and overlap only
    // on the four cells at their shared radial boundary.
    if circuit > 0 {
        let incoming_level = flight_boundary(
            settings.base_level,
            settings.rise_levels,
            circuit.checked_mul(FLIGHTS_PER_CIRCUIT)?,
        );
        for radius in inner..=outer {
            surfaces.extend(radial_forward_pad(radius, 0, 4, incoming_level));
        }
    }
    let outgoing_level = flight_boundary(
        settings.base_level,
        settings.rise_levels,
        circuit.checked_add(1)?.checked_mul(FLIGHTS_PER_CIRCUIT)?,
    );
    for radius in inner..=outer {
        surfaces.insert(TilePos::new(landing_coord(radius, 0), outgoing_level));
    }
    if circuit + 1 < CIRCUIT_COUNT {
        for radius in inner..=outer {
            surfaces.extend(radial_forward_pad(radius, 0, 4, outgoing_level));
        }
    }

    if circuit == 0 {
        for radius in CHAMBER_RADIUS..=outer {
            surfaces.extend(radial_pad(radius, 0, 4, settings.base_level));
        }
    }
    if circuit + 1 == CIRCUIT_COUNT {
        let summit = settings.base_level.checked_add(settings.rise_levels)?;
        for radius in OCULUS_RADIUS..=outer {
            surfaces.extend(radial_forward_pad(radius, 0, 4, summit));
        }
    }
    Some(surfaces)
}

fn expected_landing_surfaces(
    settings: &V3CrystalAscentSettings,
    circuit: usize,
    side: usize,
) -> Option<BTreeSet<TilePos>> {
    let (inner, outer) = CIRCUIT_BANDS.get(circuit).copied()?;
    if side >= FLIGHTS_PER_CIRCUIT {
        return None;
    }
    let flight = circuit
        .checked_mul(FLIGHTS_PER_CIRCUIT)?
        .checked_add(side)?;
    let level = flight_boundary(settings.base_level, settings.rise_levels, flight);
    Some(
        (inner..=outer)
            .map(|radius| TilePos::new(landing_coord(radius, side), level))
            .collect(),
    )
}

fn expected_flight_lane(
    settings: &V3CrystalAscentSettings,
    circuit: usize,
    side: usize,
    radius: u32,
) -> Option<Vec<TilePos>> {
    let (inner, outer) = CIRCUIT_BANDS.get(circuit).copied()?;
    if !(inner..=outer).contains(&radius) || side >= FLIGHTS_PER_CIRCUIT {
        return None;
    }
    let radius_usize = usize::try_from(radius).ok()?;
    let ring = ring_coordinates(radius);
    if ring.is_empty() || radius_usize == 0 {
        return None;
    }
    let flight = circuit
        .checked_mul(FLIGHTS_PER_CIRCUIT)?
        .checked_add(side)?;
    let boundary_low = flight_boundary(settings.base_level, settings.rise_levels, flight);
    let boundary_high = flight_boundary(settings.base_level, settings.rise_levels, flight + 1);
    let side_start = side.checked_mul(radius_usize)?;
    let mut lane = Vec::with_capacity(radius_usize.saturating_add(1));
    for offset in 0..radius_usize {
        let progress = side_start.checked_add(offset)?;
        let raw = start_index(radius).checked_add(progress)? % ring.len();
        let level = flight_tread_level(circuit, side, radius, offset, boundary_low, boundary_high)?;
        lane.push(TilePos::new(*ring.get(raw)?, level));
    }
    let next_side = side.checked_add(1)? % FLIGHTS_PER_CIRCUIT;
    lane.push(TilePos::new(
        landing_coord(radius, next_side),
        boundary_high,
    ));
    Some(lane)
}

fn expected_transfer_seam(
    settings: &V3CrystalAscentSettings,
    left_circuit: usize,
) -> Option<BTreeSet<TilePos>> {
    let (left_inner, _) = CIRCUIT_BANDS.get(left_circuit).copied()?;
    let (_, right_outer) = CIRCUIT_BANDS.get(left_circuit.checked_add(1)?).copied()?;
    if left_inner != right_outer {
        return None;
    }
    let level = flight_boundary(
        settings.base_level,
        settings.rise_levels,
        left_circuit
            .checked_add(1)?
            .checked_mul(FLIGHTS_PER_CIRCUIT)?,
    );
    Some(radial_forward_pad(left_inner, 0, 4, level))
}

fn expected_transfer_lane(
    settings: &V3CrystalAscentSettings,
    left_circuit: usize,
    lane: usize,
) -> Option<Vec<TilePos>> {
    if lane >= 4 {
        return None;
    }
    let (_, left_outer) = CIRCUIT_BANDS.get(left_circuit).copied()?;
    let (right_inner, _) = CIRCUIT_BANDS.get(left_circuit.checked_add(1)?).copied()?;
    let level = flight_boundary(
        settings.base_level,
        settings.rise_levels,
        left_circuit
            .checked_add(1)?
            .checked_mul(FLIGHTS_PER_CIRCUIT)?,
    );
    (right_inner..=left_outer)
        .rev()
        .map(|radius| {
            let ring = ring_coordinates(radius);
            let raw = start_index(radius).checked_add(lane)? % ring.len();
            Some(TilePos::new(*ring.get(raw)?, level))
        })
        .collect()
}

fn expected_transfer_pad_cells(
    settings: &V3CrystalAscentSettings,
    left_circuit: usize,
) -> Option<BTreeSet<TilePos>> {
    let (left_inner, left_outer) = CIRCUIT_BANDS.get(left_circuit).copied()?;
    let (right_inner, right_outer) = CIRCUIT_BANDS.get(left_circuit.checked_add(1)?).copied()?;
    if left_inner != right_outer {
        return None;
    }
    let level = flight_boundary(
        settings.base_level,
        settings.rise_levels,
        left_circuit
            .checked_add(1)?
            .checked_mul(FLIGHTS_PER_CIRCUIT)?,
    );
    Some(
        (right_inner..=left_outer)
            .flat_map(|radius| radial_forward_pad(radius, 0, 4, level))
            .collect::<BTreeSet<_>>(),
    )
}

fn expected_lower_interior_approach(settings: &V3CrystalAscentSettings) -> BTreeSet<TilePos> {
    (SHELL_INNER_RADIUS..SITE_RADIUS)
        .flat_map(|radius| radial_pad(radius, 0, 4, settings.base_level))
        .collect()
}

type TransferEdge = (TilePos, TilePos);

fn canonical_transfer_edge(first: TilePos, second: TilePos) -> Option<TransferEdge> {
    (first.coord.distance(second.coord) == 1 && first.level.abs_diff(second.level) <= 1).then_some(
        if first < second {
            (first, second)
        } else {
            (second, first)
        },
    )
}

fn expected_transfer_edge_allowlist(
    settings: &V3CrystalAscentSettings,
    left_circuit: usize,
    left_surfaces: &BTreeSet<TilePos>,
    right_surfaces: &BTreeSet<TilePos>,
) -> Option<BTreeSet<TransferEdge>> {
    let right_circuit = left_circuit.checked_add(1)?;
    let (left_inner, left_outer) = CIRCUIT_BANDS.get(left_circuit).copied()?;
    let (right_inner, right_outer) = CIRCUIT_BANDS.get(right_circuit).copied()?;
    if left_inner != right_outer {
        return None;
    }

    let transfer_cells = (0..4)
        .map(|lane| expected_transfer_lane(settings, left_circuit, lane))
        .collect::<Option<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<BTreeSet<_>>();
    let mut allowed_left = transfer_cells
        .intersection(left_surfaces)
        .copied()
        .collect::<BTreeSet<_>>();
    let mut allowed_right = transfer_cells
        .intersection(right_surfaces)
        .copied()
        .collect::<BTreeSet<_>>();

    // A four-wide hex corridor turning from a circumferential flight into a radial
    // transfer contains one unavoidable corner triangle at each end. Include only
    // the immediate tread on either side of the authored corridor; the equality
    // check below still rejects every more distant cross-circuit adjacency.
    for radius in left_inner..=left_outer {
        let lane = expected_flight_lane(
            settings,
            left_circuit,
            FLIGHTS_PER_CIRCUIT.saturating_sub(1),
            radius,
        )?;
        let guard = lane
            .iter()
            .rev()
            .copied()
            .find(|surface| !transfer_cells.contains(surface))?;
        allowed_left.insert(guard);
    }
    for radius in right_inner..=right_outer {
        let lane = expected_flight_lane(settings, right_circuit, 0, radius)?;
        let guard = lane
            .iter()
            .copied()
            .find(|surface| !transfer_cells.contains(surface))?;
        allowed_right.insert(guard);
    }

    Some(
        allowed_left
            .iter()
            .flat_map(|left| {
                allowed_right
                    .iter()
                    .filter_map(move |right| canonical_transfer_edge(*left, *right))
            })
            .collect(),
    )
}

fn actual_transfer_edges(
    ordinary: &OrdinaryGraph,
    left_surfaces: &BTreeSet<TilePos>,
    right_surfaces: &BTreeSet<TilePos>,
) -> BTreeSet<TransferEdge> {
    left_surfaces
        .iter()
        .flat_map(|left| {
            ordinary
                .neighbors(*left)
                .iter()
                .copied()
                .filter(|right| right_surfaces.contains(right))
                .filter_map(|right| canonical_transfer_edge(*left, right))
        })
        .collect()
}

fn transfer_edges_are_exact(
    expected: &BTreeSet<TransferEdge>,
    actual: &BTreeSet<TransferEdge>,
) -> bool {
    actual == expected
}

fn review_anchor(
    settings: &V3CrystalAscentSettings,
    circuit: usize,
    side: usize,
    radius: u32,
) -> Option<TilePos> {
    let lane = expected_flight_lane(settings, circuit, side, radius)?;
    lane.get(lane.len().checked_div(2)?).copied()
}

fn corner_landing_anchor(settings: &V3CrystalAscentSettings) -> Option<TilePos> {
    let circuit = CIRCUIT_COUNT.checked_sub(1)?;
    let side = FLIGHTS_PER_CIRCUIT.checked_sub(2)?;
    let (_, outer) = CIRCUIT_BANDS.get(circuit).copied()?;
    let landing = expected_landing_surfaces(settings, circuit, side)?;
    let position = TilePos::new(
        landing_coord(outer, side),
        flight_boundary(
            settings.base_level,
            settings.rise_levels,
            circuit
                .checked_mul(FLIGHTS_PER_CIRCUIT)?
                .checked_add(side)?,
        ),
    );
    landing.contains(&position).then_some(position)
}

fn expected_landing_alcoves(settings: &V3CrystalAscentSettings) -> Vec<TilePos> {
    let mut alcoves = Vec::with_capacity(LANDING_COUNT);
    for circuit in 0..CIRCUIT_COUNT {
        let Some((_, outer)) = CIRCUIT_BANDS.get(circuit).copied() else {
            continue;
        };
        for side in 0..FLIGHTS_PER_CIRCUIT {
            let flight = circuit
                .saturating_mul(FLIGHTS_PER_CIRCUIT)
                .saturating_add(side);
            let destination_side = side.saturating_add(1) % FLIGHTS_PER_CIRCUIT;
            alcoves.push(TilePos::new(
                landing_coord(outer.saturating_add(1), destination_side),
                flight_boundary(
                    settings.base_level,
                    settings.rise_levels,
                    flight.saturating_add(1),
                ),
            ));
        }
    }
    alcoves
}

fn expected_all_stair_surfaces(settings: &V3CrystalAscentSettings) -> BTreeSet<TilePos> {
    (0..CIRCUIT_COUNT)
        .filter_map(|circuit| expected_circuit_surfaces(settings, circuit))
        .flatten()
        .collect()
}

fn expected_summit_trail(settings: &V3CrystalAscentSettings) -> BTreeSet<TilePos> {
    let summit = settings.base_level.saturating_add(settings.rise_levels);
    (CLEARING_RADIUS..=SHELL_OUTER_RADIUS)
        .flat_map(|radius| radial_pad(radius, 3, 4, summit))
        .collect()
}

fn crown_conflicts_with_stair(
    coord: HexCoord,
    summit: Level,
    stair_surfaces: &BTreeSet<TilePos>,
) -> bool {
    let crown_bottom = summit.saturating_sub(2);
    stair_surfaces.iter().any(|surface| {
        surface.coord == coord
            && surface.level < summit
            && crown_bottom.saturating_sub(surface.level.saturating_add(1)) < 8
    })
}

fn expected_summit_clearing(settings: &V3CrystalAscentSettings) -> BTreeSet<TilePos> {
    let summit = settings.base_level.saturating_add(settings.rise_levels);
    let stair_surfaces = expected_all_stair_surfaces(settings);
    HexCoord::ORIGIN
        .within_radius(CLEARING_RADIUS)
        .into_iter()
        .filter(|coord| coord.distance(HexCoord::ORIGIN) >= OCULUS_RADIUS)
        .filter(|coord| !crown_conflicts_with_stair(*coord, summit, &stair_surfaces))
        .map(|coord| TilePos::new(coord, summit))
        .collect()
}

fn build_stairs(
    masses: &mut BTreeMap<HexCoord, Vec<MassSpec>>,
    surface_intents: &mut BTreeMap<TilePos, SurfaceMetadata>,
    settings: &V3CrystalAscentSettings,
) -> Result<AuthoredGeometry, Vec<WorldValidationIssue>> {
    let base = settings.base_level;
    let summit = base.saturating_add(settings.rise_levels);
    let mut circuit_surfaces = Vec::with_capacity(CIRCUIT_COUNT);
    for circuit in 0..CIRCUIT_COUNT {
        let surfaces = expected_circuit_surfaces(settings, circuit).ok_or_else(|| {
            vec![recipe_issue(format!(
                "Crystal Ascent circuit {circuit} cannot resolve its exact authored band"
            ))]
        })?;
        circuit_surfaces.push(surfaces);
    }
    let stair_surfaces = circuit_surfaces
        .iter()
        .flat_map(|surfaces| surfaces.iter().copied())
        .collect::<BTreeSet<_>>();
    let haunch_thicknesses = expected_stair_haunch_thicknesses(&circuit_surfaces)?;
    for position in &stair_surfaces {
        let thickness = haunch_thicknesses.get(position).copied().unwrap_or(1);
        add_platform_with_thickness(masses, *position, SolidMaterialRole::WorkedStone, thickness)?;
        surface_intents.insert(*position, interior_surface());
    }

    let landing_alcoves = expected_landing_alcoves(settings);
    for alcove in &landing_alcoves {
        add_platform(masses, *alcove, SolidMaterialRole::WorkedStone)?;
        surface_intents.insert(*alcove, interior_surface());
    }

    let lower_pad = radial_pad(SITE_RADIUS, 0, 4, base);
    let upper_pad = radial_pad(31, 3, 4, summit);
    // Keep the public chamber anchor on the entrance axis, far enough from the
    // radius-four heart that both close cameras can frame the complete object.
    // The selected actor then looks naturally inward from the aperture toward
    // the heart instead of placing the third-person boom inside its facets.
    let bottom_chamber = TilePos::new(landing_coord(16, 0), base);
    let lower_entry = *lower_pad
        .iter()
        .nth(1)
        .unwrap_or(&TilePos::new(HexCoord::ORIGIN, base));
    let upper_exit = *upper_pad
        .iter()
        .nth(1)
        .unwrap_or(&TilePos::new(HexCoord::ORIGIN, summit));
    let mid_flight = review_anchor(settings, 1, 2, 22).ok_or_else(|| {
        vec![recipe_issue(
            "Crystal Ascent mid-flight review anchor cannot resolve its route point",
        )]
    })?;
    let corner_landing = corner_landing_anchor(settings).ok_or_else(|| {
        vec![recipe_issue(
            "Crystal Ascent corner-landing review anchor cannot resolve its route point",
        )]
    })?;
    let upper_contraction = review_anchor(settings, 2, 4, 19).ok_or_else(|| {
        vec![recipe_issue(
            "Crystal Ascent upper-contraction review anchor cannot resolve its route point",
        )]
    })?;
    Ok(AuthoredGeometry {
        stair_surfaces,
        circuit_surfaces,
        crown_surfaces: BTreeSet::new(),
        landing_alcoves,
        lower_pad,
        upper_pad,
        summit_trail: BTreeSet::new(),
        summit_clearing: BTreeSet::new(),
        lower_entry,
        bottom_chamber,
        upper_exit,
        mid_flight,
        corner_landing,
        upper_contraction,
    })
}

fn expected_stair_haunch_thicknesses(
    circuit_surfaces: &[BTreeSet<TilePos>],
) -> Result<BTreeMap<TilePos, Level>, Vec<WorldValidationIssue>> {
    let mut thicknesses = BTreeMap::<TilePos, Level>::new();
    for (circuit, surfaces) in circuit_surfaces.iter().enumerate() {
        let Some((inner, outer)) = CIRCUIT_BANDS.get(circuit).copied() else {
            return Err(vec![recipe_issue(format!(
                "Crystal Ascent circuit {circuit} has no authored radial band"
            ))]);
        };
        for position in surfaces {
            let radius = position.coord.distance(HexCoord::ORIGIN);
            if !(inner..=outer).contains(&radius) {
                continue;
            }
            let lane = radius.saturating_sub(inner).saturating_add(1);
            let thickness = i32::try_from(lane.saturating_mul(2)).map_err(|error| {
                vec![recipe_issue(format!(
                    "Crystal Ascent stair haunch thickness overflows at {position:?}: {error}"
                ))]
            })?;
            thicknesses
                .entry(*position)
                .and_modify(|existing| *existing = (*existing).max(thickness))
                .or_insert(thickness);
        }
    }
    Ok(thicknesses)
}

fn build_shell(
    masses: &mut BTreeMap<HexCoord, Vec<MassSpec>>,
    surface_intents: &mut BTreeMap<TilePos, SurfaceMetadata>,
    landing_alcoves: &[TilePos],
    base: Level,
    summit: Level,
) -> Result<(), Vec<WorldValidationIssue>> {
    let alcoves = landing_alcoves
        .iter()
        .copied()
        .map(|position| (position.coord, position.level))
        .collect::<BTreeMap<_, _>>();
    let shell_top = summit.saturating_add(8);
    for coord in HexCoord::ORIGIN.within_radius(SHELL_OUTER_RADIUS) {
        let radius = coord.distance(HexCoord::ORIGIN);
        if radius < SHELL_INNER_RADIUS {
            continue;
        }
        let lower_aperture = is_lower_aperture(coord);
        let upper_trail = is_upper_trail(coord);
        let alcove = alcoves.get(&coord).copied();
        let column = masses.entry(coord).or_default();
        column.clear();

        if let Some(landing) = alcove {
            if lower_aperture && landing > base {
                add_ground_to(column, base, SolidMaterialRole::WorkedStone);
                push_mass(
                    column,
                    landing,
                    landing.saturating_add(1),
                    SolidMaterialRole::WorkedStone,
                );
                surface_intents.insert(TilePos::new(coord, base), interior_surface());
            } else {
                push_mass(
                    column,
                    0,
                    landing.saturating_add(1),
                    SolidMaterialRole::WorkedStone,
                );
            }
            let clearance = if lower_aperture { 18 } else { 8 };
            let upper_bottom = landing.saturating_add(clearance).saturating_add(1);
            let upper_top = if upper_trail {
                summit.saturating_sub(2)
            } else {
                shell_top.saturating_add(1)
            };
            push_mass(
                column,
                upper_bottom,
                upper_top,
                SolidMaterialRole::WorkedStone,
            );
            surface_intents.insert(TilePos::new(coord, landing), interior_surface());
            if upper_trail {
                push_mass(
                    column,
                    summit.saturating_sub(2),
                    summit,
                    SolidMaterialRole::Dirt,
                );
                push_mass(
                    column,
                    summit,
                    summit.saturating_add(1),
                    SolidMaterialRole::Grass,
                );
                surface_intents.insert(TilePos::new(coord, summit), exterior_surface());
            } else {
                surface_intents.insert(TilePos::new(coord, shell_top), shell_top_surface());
            }
        } else if lower_aperture {
            add_ground_to(column, base, SolidMaterialRole::WorkedStone);
            let arch = pointed_arch_clearance(coord);
            push_mass(
                column,
                base.saturating_add(arch).saturating_add(1),
                shell_top.saturating_add(1),
                SolidMaterialRole::WorkedStone,
            );
            surface_intents.insert(TilePos::new(coord, base), interior_surface());
            surface_intents.insert(TilePos::new(coord, shell_top), shell_top_surface());
        } else if upper_trail {
            push_mass(
                column,
                0,
                summit.saturating_sub(2),
                SolidMaterialRole::WorkedStone,
            );
            push_mass(
                column,
                summit.saturating_sub(2),
                summit,
                SolidMaterialRole::Dirt,
            );
            push_mass(
                column,
                summit,
                summit.saturating_add(1),
                SolidMaterialRole::Grass,
            );
            surface_intents.insert(TilePos::new(coord, summit), exterior_surface());
        } else {
            push_mass(
                column,
                0,
                shell_top.saturating_add(1),
                SolidMaterialRole::WorkedStone,
            );
            surface_intents.insert(TilePos::new(coord, shell_top), shell_top_surface());
        }
    }
    Ok(())
}

fn expected_buttress_columns(settings: &V3CrystalAscentSettings) -> BTreeMap<HexCoord, Level> {
    let shell_surface = settings
        .base_level
        .saturating_add(settings.rise_levels)
        .saturating_add(8);
    let mut columns = BTreeMap::new();
    for side in 0..FLIGHTS_PER_CIRCUIT {
        for radius in BUTTRESS_INNER_RADIUS..=BUTTRESS_OUTER_RADIUS {
            let outward_rise = i32::try_from(radius.saturating_sub(BUTTRESS_INNER_RADIUS))
                .unwrap_or(i32::MAX)
                .saturating_mul(2);
            let top = shell_surface.saturating_add(1).saturating_add(outward_rise);
            for offset in [-BUTTRESS_FLANK_OFFSET, BUTTRESS_FLANK_OFFSET] {
                if let Some(coord) = ring_offset_coord(radius, side, offset) {
                    columns.insert(coord, top);
                }
            }
        }
    }
    columns
}

fn expected_rib_voxels(settings: &V3CrystalAscentSettings) -> BTreeSet<TilePos> {
    let shell_surface = settings
        .base_level
        .saturating_add(settings.rise_levels)
        .saturating_add(8);
    let buttress_crown = shell_surface.saturating_add(
        i32::try_from(BUTTRESS_OUTER_RADIUS.saturating_sub(BUTTRESS_INNER_RADIUS))
            .unwrap_or(i32::MAX)
            .saturating_mul(2),
    );
    let mut ribs = BTreeSet::new();
    for side in 0..FLIGHTS_PER_CIRCUIT {
        for offset in -BUTTRESS_FLANK_OFFSET..=BUTTRESS_FLANK_OFFSET {
            let Some(coord) = ring_offset_coord(BUTTRESS_OUTER_RADIUS, side, offset) else {
                continue;
            };
            let rise = BUTTRESS_FLANK_OFFSET.saturating_sub(offset.abs()).max(0);
            ribs.insert(TilePos::new(coord, buttress_crown.saturating_add(rise)));
        }
    }
    ribs
}

fn expected_architecture_voxels_for_side(
    settings: &V3CrystalAscentSettings,
    side: usize,
) -> BTreeSet<TilePos> {
    let shell_surface = settings
        .base_level
        .saturating_add(settings.rise_levels)
        .saturating_add(8);
    let mut voxels = BTreeSet::new();
    for radius in BUTTRESS_INNER_RADIUS..=BUTTRESS_OUTER_RADIUS {
        let outward_rise = i32::try_from(radius.saturating_sub(BUTTRESS_INNER_RADIUS))
            .unwrap_or(i32::MAX)
            .saturating_mul(2);
        let top = shell_surface.saturating_add(1).saturating_add(outward_rise);
        for offset in [-BUTTRESS_FLANK_OFFSET, BUTTRESS_FLANK_OFFSET] {
            let Some(coord) = ring_offset_coord(radius, side, offset) else {
                continue;
            };
            voxels.extend((0..top).map(|level| TilePos::new(coord, level)));
        }
    }
    let buttress_crown = shell_surface.saturating_add(
        i32::try_from(BUTTRESS_OUTER_RADIUS.saturating_sub(BUTTRESS_INNER_RADIUS))
            .unwrap_or(i32::MAX)
            .saturating_mul(2),
    );
    for offset in -BUTTRESS_FLANK_OFFSET..=BUTTRESS_FLANK_OFFSET {
        let Some(coord) = ring_offset_coord(BUTTRESS_OUTER_RADIUS, side, offset) else {
            continue;
        };
        let rise = BUTTRESS_FLANK_OFFSET.saturating_sub(offset.abs()).max(0);
        voxels.insert(TilePos::new(coord, buttress_crown.saturating_add(rise)));
    }
    voxels
}

fn build_buttresses_and_ribs(
    masses: &mut BTreeMap<HexCoord, Vec<MassSpec>>,
    surface_intents: &mut BTreeMap<TilePos, SurfaceMetadata>,
    settings: &V3CrystalAscentSettings,
) -> Result<(), Vec<WorldValidationIssue>> {
    for (coord, top) in expected_buttress_columns(settings) {
        let column = masses.get_mut(&coord).ok_or_else(|| {
            vec![recipe_issue(format!(
                "buttress column {coord:?} leaves the Crystal Ascent mask"
            ))]
        })?;
        column.clear();
        push_mass(column, 0, top, SolidMaterialRole::WorkedStone);
        surface_intents.insert(
            TilePos::new(coord, top.saturating_sub(1)),
            shell_top_surface(),
        );
    }
    for rib in expected_rib_voxels(settings) {
        add_platform(masses, rib, SolidMaterialRole::WorkedStone)?;
        surface_intents.insert(rib, shell_top_surface());
    }
    Ok(())
}

fn build_crown(
    masses: &mut BTreeMap<HexCoord, Vec<MassSpec>>,
    surface_intents: &mut BTreeMap<TilePos, SurfaceMetadata>,
    stair_surfaces: &BTreeSet<TilePos>,
    summit: Level,
    geometry: &mut AuthoredGeometry,
) {
    let crown_bottom = summit.saturating_sub(2);
    for coord in HexCoord::ORIGIN.within_radius(SHELL_INNER_RADIUS - 1) {
        let radius = coord.distance(HexCoord::ORIGIN);
        if radius < OCULUS_RADIUS {
            continue;
        }
        let too_close_to_stairs = stair_surfaces.iter().any(|surface| {
            surface.coord == coord
                && surface.level < summit
                && crown_bottom.saturating_sub(surface.level.saturating_add(1)) < 8
        });
        let occupied_at_crown = masses.get(&coord).is_some_and(|column| {
            column
                .iter()
                .any(|mass| mass.bottom < summit.saturating_add(1) && mass.top > crown_bottom)
        });
        if !too_close_to_stairs && !occupied_at_crown {
            let column = masses.entry(coord).or_default();
            push_mass(column, crown_bottom, summit, SolidMaterialRole::Dirt);
            push_mass(
                column,
                summit,
                summit.saturating_add(1),
                SolidMaterialRole::Grass,
            );
        }
        let crown = TilePos::new(coord, summit);
        if !too_close_to_stairs {
            surface_intents
                .entry(crown)
                .or_insert_with(exterior_surface);
            geometry.crown_surfaces.insert(crown);
            if radius <= CLEARING_RADIUS {
                geometry.summit_clearing.insert(crown);
            }
        }
    }
    for radius in CLEARING_RADIUS..=SHELL_OUTER_RADIUS {
        geometry
            .summit_trail
            .extend(radial_pad(radius, 3, 4, summit));
    }
}

fn finish_volume(
    mask: BTreeSet<HexCoord>,
    masses: BTreeMap<HexCoord, Vec<MassSpec>>,
    surface_intents: BTreeMap<TilePos, SurfaceMetadata>,
) -> Result<VolumePlan, Vec<WorldValidationIssue>> {
    let mut columns = BTreeMap::new();
    let mut surfaces = BTreeMap::new();
    for coord in &mask {
        let compacted = compact_masses(masses.get(coord).cloned().unwrap_or_default())?;
        let elements = compacted
            .iter()
            .map(|mass| {
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(mass.bottom, mass.top),
                    material: mass.material,
                    cutaway_for: None,
                })
            })
            .collect::<Vec<_>>();
        for (index, mass) in compacted.iter().enumerate() {
            let covered = compacted
                .get(index + 1)
                .is_some_and(|next| next.bottom == mass.top);
            if covered {
                continue;
            }
            let position = TilePos::new(*coord, mass.top.saturating_sub(1));
            let metadata = surface_intents
                .get(&position)
                .copied()
                .unwrap_or_else(shell_top_surface);
            surfaces.insert(position, metadata);
        }
        columns.insert(*coord, VolumeColumn { elements });
    }
    let volume = VolumePlan {
        mask,
        columns,
        surfaces,
    };
    volume.validate().map_err(|issues| {
        issues
            .into_iter()
            .map(|issue| recipe_issue(issue.to_string()))
            .collect::<Vec<_>>()
    })?;
    Ok(volume)
}

fn tag_interior_shell_cutaway(volume: &mut VolumePlan, base: Level) -> BTreeSet<TilePos> {
    let ordinary_surfaces = volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| {
            (metadata.access == SurfaceAccess::Ordinary).then_some(*position)
        })
        .collect::<BTreeSet<_>>();
    let mut roofs = BTreeSet::new();
    for (coord, column) in &mut volume.columns {
        let radius = coord.distance(HexCoord::ORIGIN);
        let mut split = Vec::new();
        for element in column.elements.iter().copied() {
            let VolumeElement::Solid(mass) = element else {
                split.push(element);
                continue;
            };
            let tagged = |level: Level| {
                (SHELL_INNER_RADIUS..=BUTTRESS_OUTER_RADIUS).contains(&radius)
                    && mass.material == SolidMaterialRole::WorkedStone
                    && level > base
                    && !ordinary_surfaces.contains(&TilePos::new(*coord, level))
            };
            let mut run_bottom = mass.levels.bottom;
            let mut run_tagged = tagged(run_bottom);
            for level in mass.levels.bottom.saturating_add(1)..mass.levels.top {
                let next_tagged = tagged(level);
                if next_tagged == run_tagged {
                    continue;
                }
                split.push(VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(run_bottom, level),
                    material: mass.material,
                    cutaway_for: run_tagged.then_some(INTERIOR),
                }));
                if run_tagged {
                    roofs.extend(
                        (run_bottom..level).map(|roof_level| TilePos::new(*coord, roof_level)),
                    );
                }
                run_bottom = level;
                run_tagged = next_tagged;
            }
            split.push(VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(run_bottom, mass.levels.top),
                material: mass.material,
                cutaway_for: run_tagged.then_some(INTERIOR),
            }));
            if run_tagged {
                roofs.extend(
                    (run_bottom..mass.levels.top)
                        .map(|roof_level| TilePos::new(*coord, roof_level)),
                );
            }
        }
        column.elements = split;
    }
    roofs
}

fn build_features(
    volume: &VolumePlan,
    geometry: &AuthoredGeometry,
    streams: Option<CrystalAscentStreams<'_>>,
    trees: &TemperateTreeSet,
) -> Result<FeaturePlan, Vec<WorldValidationIssue>> {
    let mut protected_routes = BTreeMap::new();
    protected_routes.insert(
        LOWER_TERMINAL.to_owned(),
        protected_route(&geometry.lower_pad),
    );
    protected_routes.insert(
        UPPER_TERMINAL.to_owned(),
        protected_route(&geometry.upper_pad),
    );
    protected_routes.insert(
        EXIT_TRAIL.to_owned(),
        protected_route(&geometry.summit_trail),
    );
    let clearings = BTreeMap::from([(
        SUMMIT_CLEARING.to_owned(),
        FeatureClearing {
            surfaces: geometry.summit_clearing.clone(),
        },
    )]);
    let reserved = geometry
        .summit_trail
        .iter()
        .chain(&geometry.summit_clearing)
        .chain(
            geometry
                .stair_surfaces
                .iter()
                .filter(|position| position.level == geometry.upper_exit.level),
        )
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    let mut eligible = geometry
        .crown_surfaces
        .iter()
        .copied()
        .filter(|position| {
            let radius = position.coord.distance(HexCoord::ORIGIN);
            (CLEARING_RADIUS + 1..=26).contains(&radius)
                && position
                    .coord
                    .within_radius(2)
                    .into_iter()
                    .all(|coord| !reserved.contains(&coord))
        })
        .collect::<Vec<_>>();
    eligible.sort_unstable_by_key(|position| {
        (
            streams.map_or_else(
                || fallback_priority(position.coord, 11),
                |streams| streams.tree_sites.sample_coord(position.coord, 0),
            ),
            *position,
        )
    });
    let mut selected = Vec::new();
    for root in eligible {
        if selected
            .iter()
            .any(|other: &TilePos| other.coord.distance(root.coord) < 3)
        {
            continue;
        }
        selected.push(root);
        if selected.len() == 42 {
            break;
        }
    }
    let crown_by_coord = geometry
        .crown_surfaces
        .iter()
        .map(|position| (position.coord, *position))
        .collect::<BTreeMap<_, _>>();
    let mut by_id = BTreeMap::new();
    for (index, root) in selected.into_iter().enumerate() {
        let steps = u8::try_from(
            streams.map_or_else(
                || fallback_priority(root.coord, 17),
                |streams| streams.tree_rotations.sample_coord(root.coord, 0),
            ) % 6,
        )
        .unwrap_or_default();
        let rotation = HexObjectRotation::new(steps)
            .map_err(|error| vec![recipe_issue(format!("tree rotation failed: {error}"))])?;
        let blockers = trees
            .small_broadleaf
            .project_blockers(root, rotation, &crown_by_coord)
            .ok_or_else(|| {
                vec![recipe_issue(format!(
                    "summit tree at {root:?} cannot project its exact blocker"
                ))]
            })?;
        if blockers
            .iter()
            .any(|blocker| !volume.surfaces.contains_key(blocker))
        {
            return Err(vec![recipe_issue("summit tree blocker leaves the crown")]);
        }
        by_id.insert(
            FeatureId(u32::try_from(index).unwrap_or(u32::MAX)),
            PlannedFeature {
                root,
                kind: FeatureKind::Tree,
                object_id: trees.small_broadleaf.id.clone(),
                rotation,
                blocker_footprint: blockers,
            },
        );
    }
    Ok(FeaturePlan {
        by_id,
        protected_routes,
        clearings,
    })
}

fn build_lights(
    geometry: &AuthoredGeometry,
    base: Level,
    streams: Option<CrystalAscentStreams<'_>>,
) -> BTreeMap<LightId, PlannedGameplayLight> {
    let mut lights = BTreeMap::new();
    let mut next = 0_u32;
    for (index, origin) in geometry.landing_alcoves.iter().copied().enumerate() {
        let kind = match streams.map_or_else(
            || fallback_priority(origin.coord, 23),
            |streams| {
                streams
                    .crystal_kinds
                    .sample(u64::try_from(index).unwrap_or(u64::MAX))
            },
        ) % 3
        {
            0 => CaveCrystalKind::LowCluster,
            1 => CaveCrystalKind::Branched,
            _ => CaveCrystalKind::Spire,
        };
        let rotation = u8::try_from(
            streams.map_or_else(
                || fallback_priority(origin.coord, 29),
                |streams| {
                    streams
                        .crystal_rotations
                        .sample(u64::try_from(index).unwrap_or(u64::MAX))
                },
            ) % 6,
        )
        .unwrap_or_default();
        lights.insert(
            LightId(next),
            PlannedGameplayLight {
                origin,
                level: IlluminationLevel::Bright,
                radius: LANDING_BRIGHT_RADIUS,
                presentation: Some(PlannedLightPresentation::CrystalAscent(
                    CrystalAscentCrystalPresentation {
                        kind: CrystalAscentCrystalKind::Landing(kind),
                        rotation,
                    },
                )),
            },
        );
        next = next.saturating_add(1);
        lights.insert(
            LightId(next),
            PlannedGameplayLight {
                origin,
                level: IlluminationLevel::Dim,
                radius: LANDING_DIM_RADIUS,
                presentation: None,
            },
        );
        next = next.saturating_add(1);
    }
    let heart = TilePos::new(HexCoord::ORIGIN, base);
    lights.insert(
        LightId(next),
        PlannedGameplayLight {
            origin: heart,
            level: IlluminationLevel::Bright,
            radius: HEART_BRIGHT_RADIUS,
            presentation: Some(PlannedLightPresentation::CrystalAscent(
                CrystalAscentCrystalPresentation {
                    kind: CrystalAscentCrystalKind::Heart,
                    rotation: 0,
                },
            )),
        },
    );
    next = next.saturating_add(1);
    lights.insert(
        LightId(next),
        PlannedGameplayLight {
            origin: heart,
            level: IlluminationLevel::Dim,
            radius: HEART_DIM_RADIUS,
            presentation: None,
        },
    );
    lights
}

fn build_structures(volume: &VolumePlan, geometry: &AuthoredGeometry) -> StructurePlan {
    let mut shell_voxels = BTreeSet::new();
    for (coord, column) in &volume.columns {
        let radius = coord.distance(HexCoord::ORIGIN);
        if !(SHELL_INNER_RADIUS..=BUTTRESS_OUTER_RADIUS).contains(&radius) {
            continue;
        }
        for element in &column.elements {
            let VolumeElement::Solid(mass) = *element else {
                continue;
            };
            if mass.material != SolidMaterialRole::WorkedStone {
                continue;
            }
            shell_voxels.extend(
                (mass.levels.bottom..mass.levels.top).map(|level| TilePos::new(*coord, level)),
            );
        }
    }
    let mut by_id = BTreeMap::new();
    if !shell_voxels.is_empty() {
        by_id.insert(
            StructureId(0),
            PlannedStructure {
                kind: StructureKind::Wall,
                voxels: shell_voxels,
            },
        );
    }
    for circuit in 0..CIRCUIT_COUNT {
        let voxels = geometry
            .circuit_surfaces
            .get(circuit)
            .cloned()
            .unwrap_or_default();
        by_id.insert(
            StructureId(u32::try_from(circuit + 1).unwrap_or(u32::MAX)),
            PlannedStructure {
                kind: StructureKind::Stair,
                voxels,
            },
        );
    }
    StructurePlan { by_id }
}

pub(crate) fn validate_crystal_ascent(
    plan: &GeneratedWorldPlan,
    settings: &V3CrystalAscentSettings,
    objects: &CrystalAscentObjectSet,
) -> WorldValidation<CrystalAscentMetrics> {
    let mut issues = Vec::new();
    let base = settings.base_level;
    let summit = base.saturating_add(settings.rise_levels);
    for (name, expected_level) in [
        (LOWER_ENTRY, base),
        (BOTTOM_CHAMBER, base),
        (UPPER_EXIT, summit),
    ] {
        if plan.anchors.get(name).map(|position| position.level) != Some(expected_level) {
            issues.push(recipe_issue(format!(
                "anchor {name:?} must remain at exact level {expected_level}"
            )));
        }
    }
    let expected_chamber = TilePos::new(landing_coord(16, 0), base);
    if plan.anchors.get(BOTTOM_CHAMBER).copied() != Some(expected_chamber) {
        issues.push(recipe_issue(format!(
            "anchor {BOTTOM_CHAMBER:?} must remain at exact entrance-axis vista {expected_chamber:?}"
        )));
    }
    let Some(lower) = plan.anchors.get(LOWER_ENTRY).copied() else {
        return WorldValidation::Invalid(vec![recipe_issue("missing lower entry anchor")]);
    };
    let Some(upper) = plan.anchors.get(UPPER_EXIT).copied() else {
        return WorldValidation::Invalid(vec![recipe_issue("missing upper exit anchor")]);
    };
    if lower.level.abs_diff(upper.level) != settings.rise_levels.unsigned_abs() {
        issues.push(recipe_issue(
            "terminal elevation does not equal the requested rise",
        ));
    }

    let ordinary = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
    let distances = ordinary.distances_from(lower);
    let reverse_distances = ordinary.distances_from(upper);
    if !distances.contains_key(&upper) {
        issues.push(recipe_issue(
            "ordinary traversal does not connect the lower entry to the upper exit",
        ));
    }

    let expected_circuits = (0..CIRCUIT_COUNT)
        .filter_map(|circuit| expected_circuit_surfaces(settings, circuit))
        .collect::<Vec<_>>();
    if expected_circuits.len() != CIRCUIT_COUNT {
        issues.push(recipe_issue(
            "Crystal Ascent cannot derive all three authored stair circuits",
        ));
    }
    match expected_stair_haunch_thicknesses(&expected_circuits) {
        Ok(haunches) => {
            'haunch: for (surface, thickness) in haunches {
                let bottom = surface
                    .level
                    .saturating_add(1)
                    .saturating_sub(thickness)
                    .max(0);
                for level in bottom..=surface.level {
                    let material = solid_material_at(&plan.volume, surface.coord, level);
                    if material.is_none()
                        || (level > base && material != Some(SolidMaterialRole::WorkedStone))
                    {
                        issues.push(recipe_issue(format!(
                            "stair haunch below {surface:?} is not the exact {thickness}-voxel solid worked-stone support"
                        )));
                        break 'haunch;
                    }
                }
                if bottom > base.saturating_add(1)
                    && solid_material_at(&plan.volume, surface.coord, bottom.saturating_sub(1))
                        .is_some()
                {
                    issues.push(recipe_issue(format!(
                        "stair haunch below {surface:?} extends beneath its exact {thickness}-voxel support"
                    )));
                    break 'haunch;
                }
            }
        }
        Err(mut haunch_issues) => issues.append(&mut haunch_issues),
    }
    let mut validated_circuits = 0_usize;
    for circuit in 0..CIRCUIT_COUNT {
        let id = StructureId(u32::try_from(circuit.saturating_add(1)).unwrap_or(u32::MAX));
        let expected = expected_circuits.get(circuit);
        let actual = plan.structures.by_id.get(&id);
        if actual.is_some_and(|structure| {
            structure.kind == StructureKind::Stair
                && expected.is_some_and(|expected| structure.voxels == *expected)
        }) {
            validated_circuits = validated_circuits.saturating_add(1);
        } else {
            issues.push(recipe_issue(format!(
                "stair structure {id:?} does not equal authored circuit {circuit}"
            )));
        }
    }
    if plan
        .structures
        .by_id
        .values()
        .filter(|structure| structure.kind == StructureKind::Stair)
        .count()
        != CIRCUIT_COUNT
    {
        issues.push(recipe_issue(
            "Crystal Ascent must publish exactly three stair structures",
        ));
    }
    let shell = plan.structures.by_id.get(&StructureId(0));
    let expected_buttress_voxels = expected_buttress_columns(settings)
        .into_iter()
        .flat_map(|(coord, top)| (0..top).map(move |level| TilePos::new(coord, level)))
        .chain(expected_rib_voxels(settings))
        .collect::<BTreeSet<_>>();
    if shell.is_none_or(|structure| {
        structure.kind != StructureKind::Wall
            || !expected_buttress_voxels.is_subset(&structure.voxels)
    }) {
        issues.push(recipe_issue(
            "worked-stone shell omits authored buttress or pointed-rib voxels",
        ));
    }
    for side in 0..FLIGHTS_PER_CIRCUIT {
        let expected_side = expected_architecture_voxels_for_side(settings, side);
        let side_has_architecture = !expected_side.is_empty()
            && shell.is_some_and(|structure| expected_side.is_subset(&structure.voxels));
        if !side_has_architecture {
            issues.push(recipe_issue(format!(
                "shell direction {side} lacks its derived buttress and rib geometry"
            )));
        }
    }
    let stair_surfaces = plan
        .structures
        .by_id
        .values()
        .filter(|structure| structure.kind == StructureKind::Stair)
        .flat_map(|structure| structure.voxels.iter().copied())
        .collect::<BTreeSet<_>>();
    let expected_stair_surfaces = expected_circuits
        .iter()
        .flat_map(|surfaces| surfaces.iter().copied())
        .collect::<BTreeSet<_>>();
    if stair_surfaces != expected_stair_surfaces {
        issues.push(recipe_issue(
            "published stair surfaces do not equal the exact authored route",
        ));
    }
    let minimum_stair_headroom = stair_surfaces
        .iter()
        .filter_map(|surface| plan.volume.surface_headroom(*surface))
        .map(|headroom| headroom.0)
        .min()
        .unwrap_or_default();
    if minimum_stair_headroom < 4 {
        issues.push(recipe_issue(format!(
            "stair headroom falls below four levels: {minimum_stair_headroom}"
        )));
    }
    if stair_surfaces.iter().any(|surface| {
        !ordinary.contains(*surface)
            || !distances.contains_key(surface)
            || !reverse_distances.contains_key(surface)
    }) {
        issues.push(recipe_issue(
            "every authored stair surface must lie on the connected terminal route",
        ));
    }

    let mut validated_flights = 0_usize;
    let mut validated_lanes = 0_usize;
    let mut validated_landings = 0_usize;
    for circuit in 0..CIRCUIT_COUNT {
        let Some((inner, outer)) = CIRCUIT_BANDS.get(circuit).copied() else {
            continue;
        };
        for side in 0..FLIGHTS_PER_CIRCUIT {
            let mut flight_valid = outer.saturating_sub(inner).saturating_add(1) == 4;
            for radius in inner..=outer {
                let Some(lane) = expected_flight_lane(settings, circuit, side, radius) else {
                    flight_valid = false;
                    continue;
                };
                let lane_valid = lane.windows(2).all(|pair| {
                    let Some(from) = pair.first().copied() else {
                        return false;
                    };
                    let Some(to) = pair.get(1).copied() else {
                        return false;
                    };
                    from.coord.distance(to.coord) == 1
                        && from.level.abs_diff(to.level) <= 1
                        && ordinary.admits(from, to)
                });
                if lane_valid {
                    validated_lanes = validated_lanes.saturating_add(1);
                } else {
                    flight_valid = false;
                }
            }
            if flight_valid {
                validated_flights = validated_flights.saturating_add(1);
            } else {
                issues.push(recipe_issue(format!(
                    "circuit {circuit} flight {side} is not an exact four-lane one-level route"
                )));
            }

            let Some(landing) = expected_landing_surfaces(settings, circuit, side) else {
                issues.push(recipe_issue(format!(
                    "circuit {circuit} landing {side} cannot be derived"
                )));
                continue;
            };
            let landing_has_clearance = landing.len() == 4
                && landing.iter().all(|surface| {
                    ordinary.contains(*surface)
                        && plan
                            .volume
                            .surface_headroom(*surface)
                            .is_some_and(|headroom| headroom.0 >= 8)
                });
            let landing_is_connected = (inner..outer).all(|radius| {
                ordinary.admits(
                    TilePos::new(
                        landing_coord(radius, side),
                        flight_boundary(
                            base,
                            settings.rise_levels,
                            circuit
                                .saturating_mul(FLIGHTS_PER_CIRCUIT)
                                .saturating_add(side),
                        ),
                    ),
                    TilePos::new(
                        landing_coord(radius.saturating_add(1), side),
                        flight_boundary(
                            base,
                            settings.rise_levels,
                            circuit
                                .saturating_mul(FLIGHTS_PER_CIRCUIT)
                                .saturating_add(side),
                        ),
                    ),
                )
            });
            if landing_has_clearance && landing_is_connected {
                validated_landings = validated_landings.saturating_add(1);
            } else {
                issues.push(recipe_issue(format!(
                    "circuit {circuit} landing {side} must be exactly four wide with eight-level headroom"
                )));
            }
        }
    }
    if validated_lanes != FLIGHT_COUNT.saturating_mul(4) {
        issues.push(recipe_issue(format!(
            "validated {validated_lanes} of the required 72 exact flight lanes"
        )));
    }

    for left in 0..CIRCUIT_COUNT.saturating_sub(1) {
        let right = left.saturating_add(1);
        let Some(left_surfaces) = expected_circuits.get(left) else {
            continue;
        };
        let Some(right_surfaces) = expected_circuits.get(right) else {
            continue;
        };
        let expected_seam = expected_transfer_seam(settings, left).unwrap_or_default();
        let actual_seam = left_surfaces
            .intersection(right_surfaces)
            .copied()
            .collect::<BTreeSet<_>>();
        if expected_seam.len() != 4 || actual_seam != expected_seam {
            issues.push(recipe_issue(format!(
                "circuits {left} and {right} must share exactly their four-cell transfer seam"
            )));
        }

        let mut transfer_cells = BTreeSet::new();
        let mut valid_transfer_lanes = 0_usize;
        for lane_index in 0..4 {
            let Some(lane) = expected_transfer_lane(settings, left, lane_index) else {
                continue;
            };
            let disjoint = lane.iter().all(|surface| transfer_cells.insert(*surface));
            let contained = lane
                .iter()
                .all(|surface| left_surfaces.contains(surface) || right_surfaces.contains(surface));
            let admitted = lane.iter().all(|surface| ordinary.contains(*surface))
                && lane.windows(2).all(|pair| {
                    pair.first().zip(pair.get(1)).is_some_and(|(from, to)| {
                        from.coord.distance(to.coord) == 1
                            && from.level.abs_diff(to.level) <= 1
                            && ordinary.admits(*from, *to)
                    })
                });
            if disjoint && contained && admitted {
                valid_transfer_lanes = valid_transfer_lanes.saturating_add(1);
            }
        }
        if valid_transfer_lanes != 4 {
            issues.push(recipe_issue(format!(
                "circuits {left} and {right} retain {valid_transfer_lanes} of four independent transfer lanes"
            )));
        }

        let transfer_pad = expected_transfer_pad_cells(settings, left).unwrap_or_default();
        if transfer_pad.len() != 28
            || transfer_pad.iter().any(|surface| {
                !ordinary.contains(*surface)
                    || plan
                        .volume
                        .surface_headroom(*surface)
                        .is_none_or(|headroom| headroom.0 < 8)
            })
        {
            issues.push(recipe_issue(format!(
                "circuits {left} and {right} must retain exact four-by-four turning pads with eight-level headroom"
            )));
        }

        let expected_transfer_edges =
            expected_transfer_edge_allowlist(settings, left, left_surfaces, right_surfaces)
                .unwrap_or_default();
        let actual_edges = actual_transfer_edges(&ordinary, left_surfaces, right_surfaces);
        if expected_transfer_edges.is_empty()
            || !transfer_edges_are_exact(&expected_transfer_edges, &actual_edges)
        {
            issues.push(recipe_issue(format!(
                "circuits {left} and {right} contain an ordinary edge outside their exact transfer corridor"
            )));
        }
    }
    if expected_circuits
        .first()
        .zip(expected_circuits.last())
        .is_some_and(|(first, last)| {
            first.iter().any(|surface| {
                ordinary
                    .neighbors(*surface)
                    .iter()
                    .any(|neighbor| last.contains(neighbor))
            })
        })
    {
        issues.push(recipe_issue(
            "circuits 0 and 2 contain an unintended cross-loop adjacency",
        ));
    }

    let allowed_midlevel = expected_stair_surfaces
        .iter()
        .copied()
        .chain(expected_landing_alcoves(settings))
        .collect::<BTreeSet<_>>();
    if ordinary.positions().any(|surface| {
        surface.level > base && surface.level < summit && !allowed_midlevel.contains(&surface)
    }) {
        issues.push(recipe_issue(
            "an unintended ordinary mid-level surface creates a shaft or shell shortcut",
        ));
    }

    for (name, expected) in [
        (MID_FLIGHT, review_anchor(settings, 1, 2, 22)),
        (CORNER_LANDING, corner_landing_anchor(settings)),
        (UPPER_CONTRACTION, review_anchor(settings, 2, 4, 19)),
    ] {
        if plan.anchors.get(name).copied() != expected
            || expected.is_none_or(|position| !ordinary.contains(position))
        {
            issues.push(recipe_issue(format!(
                "review anchor {name:?} must remain on its exact authored route point"
            )));
        }
    }

    let lower_pad = radial_pad(SITE_RADIUS, 0, 4, base);
    let upper_pad = radial_pad(31, 3, 4, summit);
    let summit_trail = expected_summit_trail(settings);
    for (name, expected) in [
        (LOWER_TERMINAL, &lower_pad),
        (UPPER_TERMINAL, &upper_pad),
        (EXIT_TRAIL, &summit_trail),
    ] {
        let exact = plan
            .features
            .protected_routes
            .get(name)
            .is_some_and(|route| {
                route.surfaces == *expected
                    && route
                        .centerline
                        .iter()
                        .copied()
                        .eq(expected.iter().copied())
            });
        if !exact || expected.iter().any(|surface| !ordinary.contains(*surface)) {
            issues.push(recipe_issue(format!(
                "protected route {name:?} does not equal its exact authored footprint"
            )));
        }
    }
    if lower_pad.iter().any(|surface| {
        plan.volume
            .surfaces
            .get(surface)
            .is_none_or(|metadata| metadata.interior.is_some())
    }) {
        issues.push(recipe_issue(
            "the exact four-wide lower terminal must remain an exterior apron",
        ));
    }
    let expected_entrances = radial_pad(SITE_RADIUS.saturating_sub(1), 0, 4, base);
    let actual_entrances = plan
        .interiors
        .by_id
        .get(&INTERIOR)
        .map(|interior| &interior.entrances);
    if actual_entrances != Some(&expected_entrances)
        || expected_entrances.iter().any(|surface| {
            plan.volume
                .surfaces
                .get(surface)
                .is_none_or(|metadata| metadata.interior != Some(INTERIOR))
        })
    {
        issues.push(recipe_issue(
            "the Dark interior must begin at the exact four-wide radius-31 threshold",
        ));
    }
    let expected_clearing = expected_summit_clearing(settings);
    if plan
        .features
        .clearings
        .get(SUMMIT_CLEARING)
        .map(|clearing| &clearing.surfaces)
        != Some(&expected_clearing)
    {
        issues.push(recipe_issue(
            "summit clearing does not equal the authored radius-11 through radius-18 footprint",
        ));
    }

    for radius in SHELL_INNER_RADIUS..=SHELL_OUTER_RADIUS {
        let expected = radial_pad(radius, 0, 12, base);
        let actual = plan
            .volume
            .surfaces
            .keys()
            .copied()
            .filter(|surface| {
                surface.level == base && surface.coord.distance(HexCoord::ORIGIN) == radius
            })
            .collect::<BTreeSet<_>>();
        if actual != expected {
            issues.push(recipe_issue(format!(
                "shell radius {radius} does not preserve the exact twelve-wide lower aperture"
            )));
        }
    }
    for radius in CHAMBER_RADIUS.saturating_add(1)..SHELL_INNER_RADIUS {
        let mut expected = radial_pad(radius, 0, 4, base);
        expected.extend(stair_surfaces.iter().copied().filter(|surface| {
            surface.level == base && surface.coord.distance(HexCoord::ORIGIN) == radius
        }));
        let actual = plan
            .volume
            .surfaces
            .keys()
            .copied()
            .filter(|surface| {
                surface.level == base && surface.coord.distance(HexCoord::ORIGIN) == radius
            })
            .collect::<BTreeSet<_>>();
        if actual != expected {
            issues.push(recipe_issue(format!(
                "interior approach radius {radius} does not narrow to the exact four-wide route"
            )));
        }
    }
    let aperture = radial_pad(SHELL_OUTER_RADIUS, 0, 12, base);
    let aperture_headrooms = aperture
        .iter()
        .filter_map(|surface| exact_clear_levels_above(&plan.volume, *surface))
        .collect::<Vec<_>>();
    if aperture_headrooms.len() != 12
        || aperture_headrooms.iter().copied().min().unwrap_or_default() < 12
        || aperture_headrooms.iter().copied().max().unwrap_or_default() < 18
    {
        issues.push(recipe_issue(
            "pointed lower aperture must retain twelve-wide clearance and an eighteen-level apex",
        ));
    }
    let oculus_rim = ring_coordinates(OCULUS_RADIUS)
        .into_iter()
        .map(|coord| TilePos::new(coord, summit))
        .collect::<BTreeSet<_>>();
    if plan.volume.surfaces.keys().any(|surface| {
        surface.level == summit && surface.coord.distance(HexCoord::ORIGIN) < OCULUS_RADIUS
    }) || oculus_rim
        .iter()
        .any(|surface| !plan.volume.surfaces.contains_key(surface))
    {
        issues.push(recipe_issue(
            "summit must retain an open radius-11 oculus with a complete rim",
        ));
    }

    let chamber_route = HexCoord::ORIGIN
        .within_radius(CHAMBER_RADIUS)
        .into_iter()
        .map(|coord| TilePos::new(coord, base))
        .collect::<BTreeSet<_>>();
    if chamber_route.iter().any(|surface| {
        plan.volume
            .surfaces
            .get(surface)
            .is_none_or(|metadata| metadata.interior != Some(INTERIOR))
    }) {
        issues.push(recipe_issue(
            "bottom chamber does not retain its exact radius-23 Interior floor",
        ));
    }
    let lower_interior_approach = expected_lower_interior_approach(settings);
    if lower_interior_approach.len() != 16
        || lower_interior_approach.iter().any(|surface| {
            plan.volume
                .surfaces
                .get(surface)
                .is_none_or(|metadata| metadata.interior != Some(INTERIOR))
        })
    {
        issues.push(recipe_issue(
            "the lower radius-28 through radius-31 approach must remain exactly four-wide and Interior",
        ));
    }

    if plan
        .lights
        .values()
        .filter(|light| light.presentation.is_some())
        .count()
        != LANDING_COUNT + 1
        || plan.lights.len() != LANDING_COUNT * 2 + 2
    {
        issues.push(recipe_issue(
            "Crystal Ascent must publish eighteen landing fixtures, one heart, and exact paired gameplay lights",
        ));
    }
    let expected_fixture_origins = expected_landing_alcoves(settings)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let actual_fixture_origins = plan
        .lights
        .values()
        .filter_map(|light| match light.presentation {
            Some(PlannedLightPresentation::CrystalAscent(CrystalAscentCrystalPresentation {
                kind: CrystalAscentCrystalKind::Landing(_),
                ..
            })) => Some(light.origin),
            Some(PlannedLightPresentation::CrystalAscent(CrystalAscentCrystalPresentation {
                kind: CrystalAscentCrystalKind::Heart,
                ..
            }))
            | Some(PlannedLightPresentation::CaveCrystal(_))
            | None => None,
        })
        .collect::<BTreeSet<_>>();
    if actual_fixture_origins != expected_fixture_origins {
        issues.push(recipe_issue(
            "landing crystal fixtures do not equal all eighteen destination alcoves",
        ));
    }
    for origin in &expected_fixture_origins {
        let pair = plan
            .lights
            .values()
            .filter(|light| light.origin == *origin)
            .collect::<Vec<_>>();
        let exact_bright = pair.iter().filter(|light| {
            light.level == IlluminationLevel::Bright
                && light.radius == LANDING_BRIGHT_RADIUS
                && matches!(
                    light.presentation,
                    Some(PlannedLightPresentation::CrystalAscent(
                        CrystalAscentCrystalPresentation {
                            kind: CrystalAscentCrystalKind::Landing(_),
                            rotation: 0..=5,
                        }
                    ))
                )
        });
        let exact_dim = pair.iter().filter(|light| {
            light.level == IlluminationLevel::Dim
                && light.radius == LANDING_DIM_RADIUS
                && light.presentation.is_none()
        });
        if pair.len() != 2 || exact_bright.count() != 1 || exact_dim.count() != 1 {
            issues.push(recipe_issue(format!(
                "landing crystal at {origin:?} must own one exact Bright/Dim gameplay-light pair"
            )));
            break;
        }
    }
    let heart_origin = TilePos::new(HexCoord::ORIGIN, base);
    let heart_pair = plan
        .lights
        .values()
        .filter(|light| light.origin == heart_origin)
        .collect::<Vec<_>>();
    let exact_heart_bright = heart_pair
        .iter()
        .filter(|light| {
            light.level == IlluminationLevel::Bright
                && light.radius == HEART_BRIGHT_RADIUS
                && matches!(
                    light.presentation,
                    Some(PlannedLightPresentation::CrystalAscent(
                        CrystalAscentCrystalPresentation {
                            kind: CrystalAscentCrystalKind::Heart,
                            rotation: 0,
                        }
                    ))
                )
        })
        .count();
    let exact_heart_dim = heart_pair
        .iter()
        .filter(|light| {
            light.level == IlluminationLevel::Dim
                && light.radius == HEART_DIM_RADIUS
                && light.presentation.is_none()
        })
        .count();
    if heart_pair.len() != 2 || exact_heart_bright != 1 || exact_heart_dim != 1 {
        issues.push(recipe_issue(
            "cathedral heart must own one exact Bright/Dim gameplay-light pair",
        ));
    }
    if stair_surfaces.iter().any(|surface| {
        plan.volume
            .surfaces
            .get(surface)
            .is_none_or(|metadata| metadata.interior != Some(INTERIOR))
    }) {
        issues.push(recipe_issue(
            "the complete authored staircase must remain in one Dark interior domain",
        ));
    }
    let dim_sources = plan
        .lights
        .values()
        .filter(|light| light.level == IlluminationLevel::Dim)
        .collect::<Vec<_>>();
    let required_dim_surfaces = stair_surfaces
        .iter()
        .chain(&chamber_route)
        .chain(&lower_interior_approach)
        .copied()
        .collect::<BTreeSet<_>>();
    for surface in &required_dim_surfaces {
        if !dim_sources.iter().any(|source| {
            plan.volume
                .surfaces
                .get(&source.origin)
                .and_then(|metadata| metadata.interior)
                == Some(INTERIOR)
                && upper_dome_contains(
                    ExactGridPoint::voxel_center(source.origin),
                    ExactGridPoint::voxel_center(*surface),
                    source.radius,
                )
        }) {
            issues.push(recipe_issue(format!(
                "required interior route surface {surface:?} is not at least Dim"
            )));
            break;
        }
    }
    let heart_visual_origin = base
        .checked_add(1)
        .map(|level| TilePos::new(HexCoord::ORIGIN, level));
    let heart_rotation = HexObjectRotation::new(0).ok();
    let expected_heart_blockers =
        heart_visual_origin
            .zip(heart_rotation)
            .and_then(|(origin, rotation)| {
                objects.project_heart_traversal_blockers(
                    plan.volume.surfaces.keys().copied(),
                    origin,
                    rotation,
                )
            });
    let feature_blockers = plan
        .features
        .by_id
        .values()
        .flat_map(|feature| feature.blocker_footprint.iter().copied())
        .collect::<BTreeSet<_>>();
    let actual_heart_blockers = plan
        .blockers
        .difference(&feature_blockers)
        .copied()
        .collect::<BTreeSet<_>>();
    if expected_heart_blockers.as_ref() != Some(&actual_heart_blockers) {
        issues.push(recipe_issue(
            "cathedral heart blockers do not equal its projected structural runs",
        ));
    }

    let expected_roofs = plan
        .volume
        .columns
        .iter()
        .flat_map(|(coord, column)| {
            let radius = coord.distance(HexCoord::ORIGIN);
            column.elements.iter().flat_map(move |element| {
                let VolumeElement::Solid(mass) = *element else {
                    return Vec::new().into_iter();
                };
                (mass.levels.bottom..mass.levels.top)
                    .filter(move |level| {
                        (SHELL_INNER_RADIUS..=BUTTRESS_OUTER_RADIUS).contains(&radius)
                            && mass.material == SolidMaterialRole::WorkedStone
                            && *level > base
                            && !plan
                                .volume
                                .surfaces
                                .get(&TilePos::new(*coord, *level))
                                .is_some_and(|metadata| metadata.access == SurfaceAccess::Ordinary)
                    })
                    .map(move |level| TilePos::new(*coord, level))
                    .collect::<Vec<_>>()
                    .into_iter()
            })
        })
        .collect::<BTreeSet<_>>();
    let actual_roofs = plan
        .interiors
        .by_id
        .get(&INTERIOR)
        .map(|interior| &interior.roof_voxels);
    if expected_roofs.is_empty() || actual_roofs != Some(&expected_roofs) {
        issues.push(recipe_issue(
            "interior cutaway must tag the exact non-traversable worked-stone shell",
        ));
    }
    if expected_roofs.iter().any(|position| {
        stair_surfaces.contains(position)
            || position.coord.distance(HexCoord::ORIGIN) < OCULUS_RADIUS
            || (position.level > base
                && position.level <= base.saturating_add(pointed_arch_clearance(position.coord))
                && is_lower_aperture(position.coord))
    }) {
        issues.push(recipe_issue(
            "interior cutaway enters stairs, the oculus, or lower-aperture clearance",
        ));
    }

    let tree_roots = plan
        .features
        .by_id
        .values()
        .filter(|feature| feature.kind == FeatureKind::Tree)
        .count();
    if plan.features.by_id.values().any(|feature| {
        feature.kind == FeatureKind::Tree
            && (feature.root.coord.distance(HexCoord::ORIGIN) <= CLEARING_RADIUS
                || summit_trail.contains(&feature.root))
    }) {
        issues.push(recipe_issue(
            "summit trees enter the clearing or protected exit trail",
        ));
    }
    if !issues.is_empty() {
        return WorldValidation::Invalid(issues);
    }
    let ordinary_levels = ordinary
        .positions()
        .map(|position| position.level)
        .collect::<BTreeSet<_>>();
    let chamber_surfaces = plan.interiors.by_id.get(&INTERIOR).map_or(0, |interior| {
        interior
            .floors
            .iter()
            .filter(|surface| {
                surface.level == base
                    && surface.coord.distance(HexCoord::ORIGIN) <= CHAMBER_RADIUS
                    && ordinary.contains(**surface)
            })
            .count()
    });
    let crown_surfaces = plan
        .volume
        .surfaces
        .keys()
        .filter(|surface| {
            surface.level == summit && surface.coord.distance(HexCoord::ORIGIN) >= OCULUS_RADIUS
        })
        .count();
    WorldValidation::Valid(CrystalAscentMetrics {
        circuits: count_u32(validated_circuits),
        flights: count_u32(validated_flights),
        landings: count_u32(validated_landings),
        stair_surfaces: count_u32(stair_surfaces.len()),
        chamber_surfaces: count_u32(chamber_surfaces),
        crown_surfaces: count_u32(crown_surfaces),
        tree_roots: count_u32(tree_roots),
        crystal_fixtures: count_u32(LANDING_COUNT + 1),
        gameplay_lights: count_u32(plan.lights.len()),
        ordinary_surfaces: count_u32(ordinary.len()),
        reachable_elevation_levels: count_u32(ordinary_levels.len()),
        critical_route_steps: distances.get(&upper).copied().unwrap_or_default(),
        rise_levels: settings.rise_levels,
        minimum_stair_headroom,
    })
}

/// Validates the complete authored landmark while it is still a patch-local
/// fragment, before Macro namespaces its IDs and aliases into the combined
/// world. This deliberately skips the standalone radius contract: the resolved
/// Macro mask remains authoritative, while every stair, shell, light, blocker,
/// interior, and terminal invariant stays identical to the standalone recipe.
pub(crate) fn validate_composite_fragment(
    fragment: &GeneratedPatchPlan,
    layout: &ResolvedLayoutPlan,
    settings: &V3CrystalAscentSettings,
    objects: &CrystalAscentObjectSet,
) -> WorldValidation<CrystalAscentMetrics> {
    let patch = match PatchRecipeContext::resolve(layout, fragment.patch_id) {
        Ok(patch) => patch,
        Err(error) => {
            return WorldValidation::Invalid(vec![recipe_issue(format!(
                "Crystal Ascent composite fragment has no resolved Macro patch: {error}"
            ))]);
        }
    };
    let frame = match patch.local_frame() {
        Ok(frame) => frame,
        Err(error) => {
            return WorldValidation::Invalid(vec![recipe_issue(format!(
                "Crystal Ascent composite frame could not be resolved: {error}"
            ))]);
        }
    };
    let isolated = match frame.canonical_local_world(fragment) {
        Ok(isolated) => isolated,
        Err(error) => {
            return WorldValidation::Invalid(vec![recipe_issue(format!(
                "Crystal Ascent composite fragment could not be normalized: {error}"
            ))]);
        }
    };
    validate_crystal_ascent(&isolated, settings, objects)
}

fn exact_clear_levels_above(volume: &VolumePlan, surface: TilePos) -> Option<Level> {
    volume.surfaces.get(&surface)?;
    let from = surface.level.checked_add(1)?;
    let column = volume.columns.get(&surface.coord)?;
    Some(
        column
            .elements
            .iter()
            .filter_map(|element| {
                let levels = match *element {
                    VolumeElement::Solid(mass) => mass.levels,
                    VolumeElement::Fill(fill) => fill.levels,
                };
                if levels.bottom <= from && from < levels.top {
                    Some(0)
                } else {
                    (levels.bottom > from).then_some(levels.bottom.saturating_sub(from))
                }
            })
            .min()
            .unwrap_or(i32::MAX),
    )
}

fn solid_material_at(
    volume: &VolumePlan,
    coord: HexCoord,
    level: Level,
) -> Option<SolidMaterialRole> {
    volume
        .columns
        .get(&coord)?
        .elements
        .iter()
        .find_map(|element| match element {
            VolumeElement::Solid(mass)
                if mass.levels.bottom <= level && level < mass.levels.top =>
            {
                Some(mass.material)
            }
            _ => None,
        })
}

fn add_ground(
    masses: &mut BTreeMap<HexCoord, Vec<MassSpec>>,
    coord: HexCoord,
    surface: Level,
    top: SolidMaterialRole,
) {
    add_ground_to(masses.entry(coord).or_default(), surface, top);
}

fn add_ground_to(column: &mut Vec<MassSpec>, surface: Level, top: SolidMaterialRole) {
    push_mass(column, 0, 1, SolidMaterialRole::Bedrock);
    push_mass(
        column,
        1,
        surface.saturating_sub(3),
        SolidMaterialRole::Stone,
    );
    push_mass(
        column,
        surface.saturating_sub(3),
        surface,
        SolidMaterialRole::Dirt,
    );
    push_mass(column, surface, surface.saturating_add(1), top);
}

fn add_platform(
    masses: &mut BTreeMap<HexCoord, Vec<MassSpec>>,
    position: TilePos,
    material: SolidMaterialRole,
) -> Result<(), Vec<WorldValidationIssue>> {
    add_platform_with_thickness(masses, position, material, 1)
}

fn add_platform_with_thickness(
    masses: &mut BTreeMap<HexCoord, Vec<MassSpec>>,
    position: TilePos,
    material: SolidMaterialRole,
    thickness: Level,
) -> Result<(), Vec<WorldValidationIssue>> {
    if thickness < 1 {
        return Err(vec![recipe_issue(format!(
            "platform {position:?} requires positive thickness"
        ))]);
    }
    let column = masses.get_mut(&position.coord).ok_or_else(|| {
        vec![recipe_issue(format!(
            "platform {position:?} leaves the authored mask"
        ))]
    })?;
    if column
        .iter()
        .any(|mass| mass.bottom <= position.level && position.level < mass.top)
    {
        return Ok(());
    }
    let top = position.level.saturating_add(1);
    let mut bottom = top.saturating_sub(thickness).max(0);
    let mut existing = column.to_vec();
    existing.sort_unstable_by_key(|mass| (mass.bottom, mass.top, mass.material));
    for mass in existing {
        if mass.top <= bottom || mass.bottom >= top {
            continue;
        }
        if mass.bottom <= bottom {
            bottom = mass.top;
            continue;
        }
        return Err(vec![recipe_issue(format!(
            "platform support {bottom}..{top} at {position:?} overlaps isolated authored mass {mass:?}"
        ))]);
    }
    push_mass(column, bottom, top, material);
    Ok(())
}

fn push_mass(column: &mut Vec<MassSpec>, bottom: Level, top: Level, material: SolidMaterialRole) {
    if bottom < top {
        column.push(MassSpec {
            bottom,
            top,
            material,
        });
    }
}

fn compact_masses(mut masses: Vec<MassSpec>) -> Result<Vec<MassSpec>, Vec<WorldValidationIssue>> {
    masses.sort_unstable_by_key(|mass| (mass.bottom, mass.top, mass.material));
    let mut compacted: Vec<MassSpec> = Vec::with_capacity(masses.len());
    for mass in masses {
        let Some(previous) = compacted.last_mut() else {
            compacted.push(mass);
            continue;
        };
        if mass.bottom < previous.top {
            if mass == *previous {
                continue;
            }
            return Err(vec![recipe_issue(format!(
                "authored solid intervals overlap: {previous:?} and {mass:?}"
            ))]);
        }
        if mass.bottom == previous.top && mass.material == previous.material {
            previous.top = mass.top;
        } else {
            compacted.push(mass);
        }
    }
    Ok(compacted)
}

fn flight_boundary(base: Level, rise: Level, flight: usize) -> Level {
    base.saturating_add(
        i32::try_from(flight)
            .unwrap_or(i32::MAX)
            .saturating_mul(rise)
            .checked_div(i32::try_from(FLIGHT_COUNT).unwrap_or(1))
            .unwrap_or_default(),
    )
}

fn flight_tread_level(
    circuit: usize,
    side: usize,
    radius: u32,
    offset: usize,
    boundary_low: Level,
    boundary_high: Level,
) -> Option<Level> {
    let transfer_landing_depth = if circuit > 0 && side == 0 { 3 } else { 0 };
    let adjusted_offset = u32::try_from(offset)
        .ok()?
        .saturating_sub(transfer_landing_depth);
    let adjusted_run = radius.saturating_sub(transfer_landing_depth).max(1);
    let delta = boundary_high.saturating_sub(boundary_low);
    boundary_low.checked_add(
        i32::try_from(adjusted_offset)
            .ok()?
            .saturating_mul(delta)
            .checked_div(i32::try_from(adjusted_run).ok()?)?,
    )
}

fn ring_coordinates(radius: u32) -> Vec<HexCoord> {
    if radius == 0 {
        return vec![HexCoord::ORIGIN];
    }
    let directions = [
        HexCoord::new_cubic(1, -1, 0),
        HexCoord::new_cubic(1, 0, -1),
        HexCoord::new_cubic(0, 1, -1),
        HexCoord::new_cubic(-1, 1, 0),
        HexCoord::new_cubic(-1, 0, 1),
        HexCoord::new_cubic(0, -1, 1),
    ];
    let radius_i32 = i32::try_from(radius).unwrap_or(i32::MAX);
    let mut current = HexCoord::new_cubic(-radius_i32, 0, radius_i32);
    let mut ring = Vec::with_capacity(usize::try_from(radius.saturating_mul(6)).unwrap_or(0));
    for direction in directions {
        for _ in 0..radius {
            ring.push(current);
            current = shift(current, direction);
        }
    }
    ring
}

fn start_index(radius: u32) -> usize {
    usize::try_from(radius / 2).unwrap_or_default()
}

fn landing_coord(radius: u32, side: usize) -> HexCoord {
    let ring = ring_coordinates(radius);
    if ring.is_empty() {
        return HexCoord::ORIGIN;
    }
    let raw = (start_index(radius)
        + side.saturating_mul(usize::try_from(radius).unwrap_or(usize::MAX)))
        % ring.len();
    ring.get(raw).copied().unwrap_or(HexCoord::ORIGIN)
}

fn radial_pad(radius: u32, side: usize, width: usize, level: Level) -> BTreeSet<TilePos> {
    radial_coords(radius, side, width)
        .into_iter()
        .map(|coord| TilePos::new(coord, level))
        .collect()
}

fn radial_forward_pad(radius: u32, side: usize, width: usize, level: Level) -> BTreeSet<TilePos> {
    let ring = ring_coordinates(radius);
    if ring.is_empty() {
        return BTreeSet::new();
    }
    let center = (start_index(radius)
        + side.saturating_mul(usize::try_from(radius).unwrap_or(usize::MAX)))
        % ring.len();
    (0..width)
        .filter_map(|offset| {
            ring.get(center.saturating_add(offset) % ring.len())
                .copied()
                .map(|coord| TilePos::new(coord, level))
        })
        .collect()
}

fn ring_offset_coord(radius: u32, side: usize, offset: i32) -> Option<HexCoord> {
    let ring = ring_coordinates(radius);
    let len = i64::try_from(ring.len()).ok()?;
    if len == 0 {
        return None;
    }
    let center = start_index(radius)
        .checked_add(side.checked_mul(usize::try_from(radius).ok()?)?)?
        % ring.len();
    let raw = (i64::try_from(center).ok()? + i64::from(offset)).rem_euclid(len);
    ring.get(usize::try_from(raw).ok()?).copied()
}

fn radial_coords(radius: u32, side: usize, width: usize) -> BTreeSet<HexCoord> {
    let ring = ring_coordinates(radius);
    if ring.is_empty() {
        return BTreeSet::new();
    }
    let center = (start_index(radius)
        + side.saturating_mul(usize::try_from(radius).unwrap_or(usize::MAX)))
        % ring.len();
    let before = width / 2;
    (0..width)
        .filter_map(|offset| {
            let raw = (center + ring.len() + offset).saturating_sub(before) % ring.len();
            ring.get(raw).copied()
        })
        .collect()
}

fn shift(coord: HexCoord, delta: HexCoord) -> HexCoord {
    let [x, y, z] = coord.to_cubic_array();
    let [dx, dy, dz] = delta.to_cubic_array();
    HexCoord::new_cubic(x + dx, y + dy, z + dz)
}

fn is_lower_aperture(coord: HexCoord) -> bool {
    radial_coords(coord.distance(HexCoord::ORIGIN), 0, 12).contains(&coord)
}

fn is_lower_connector(coord: HexCoord) -> bool {
    radial_coords(coord.distance(HexCoord::ORIGIN), 0, 4).contains(&coord)
}

fn is_upper_trail(coord: HexCoord) -> bool {
    radial_coords(coord.distance(HexCoord::ORIGIN), 3, 4).contains(&coord)
}

fn pointed_arch_clearance(coord: HexCoord) -> Level {
    let [x, y, _] = coord.to_cubic_array();
    let lateral = i32::try_from(x.abs_diff(y) / 2).unwrap_or(i32::MAX);
    18_i32.saturating_sub(lateral).max(12)
}

const fn interior_surface() -> SurfaceMetadata {
    SurfaceMetadata {
        access: SurfaceAccess::Ordinary,
        interior: Some(INTERIOR),
    }
}

const fn exterior_surface() -> SurfaceMetadata {
    SurfaceMetadata {
        access: SurfaceAccess::Ordinary,
        interior: None,
    }
}

const fn shell_top_surface() -> SurfaceMetadata {
    SurfaceMetadata {
        access: SurfaceAccess::SpecialMovement(SHELL_TOPS),
        interior: None,
    }
}

fn protected_route(surfaces: &BTreeSet<TilePos>) -> ProtectedFeatureRoute {
    ProtectedFeatureRoute {
        centerline: surfaces.iter().copied().collect(),
        surfaces: surfaces.clone(),
    }
}

fn fallback_priority(coord: HexCoord, salt: u64) -> u64 {
    let mut value = salt ^ 0x9e37_79b9_7f4a_7c15;
    for component in coord.to_cubic_array() {
        value ^= u64::from_le_bytes(i64::from(component).to_le_bytes());
        value = value.wrapping_mul(0x100_0000_01b3);
    }
    value
}

fn ascent_view_hint(level_height: f32, base: Level, summit: Level) -> MapViewHint {
    let eye_coord = landing_coord(38, 0);
    let focus_coord = HexCoord::ORIGIN;
    let eye = eye_coord.to_world(
        f32::from(i16::try_from(summit.saturating_add(16)).unwrap_or(i16::MAX)) * level_height,
    );
    let focus = focus_coord.to_world(
        f32::from(i16::try_from(base.saturating_add(48)).unwrap_or(i16::MAX)) * level_height,
    );
    MapViewHint::new((eye.x, eye.y, eye.z), (focus.x, focus.y, focus.z))
}

fn count_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn recipe_issue(detail: impl Into<String>) -> WorldValidationIssue {
    WorldValidationIssue::new(WorldIssueCode::Recipe("crystal_ascent"), detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::BiomeRegionId;

    use crate::procedural_v3::crystal_ascent_assets::tests::runtime_art_catalog;
    use crate::procedural_v3::layout::{
        HexSide, LayoutKind, ResolvedEdgeContract, ResolvedEdgeId, ResolvedEdgeReference,
        ResolvedElevationBand, ResolvedLiquidPort, ResolvedPatch, ResolvedWalkerPorts,
    };
    use crate::procedural_v3::local_frame::LocalPatchFrame;
    use crate::settings::{
        PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings, PatchSpec,
    };

    fn settings(rise_levels: Level) -> ProceduralV3Settings {
        let boundary = PatchEdgeContractSettings::WorldBoundary;
        ProceduralV3Settings {
            layout: V3LayoutSettings::Single(PatchSpec {
                environment: V3EnvironmentSettings::TemperateGrassland,
                recipe: V3RecipeSettings::CrystalAscent(V3CrystalAscentSettings {
                    base_level: 6,
                    rise_levels,
                }),
                overlays: Vec::new(),
                mask: PatchMaskSettings::WholeWorld,
                edges: PatchEdgesSettings {
                    east: boundary.clone(),
                    south_east: boundary.clone(),
                    south_west: boundary.clone(),
                    west: boundary.clone(),
                    north_west: boundary.clone(),
                    north_east: boundary,
                },
            }),
        }
    }

    fn raw_fragment(
        rise_levels: Level,
        mode: PatchBuildMode,
    ) -> (
        ResolvedLayoutPlan,
        GeneratedPatchPlan,
        CrystalAscentObjectSet,
    ) {
        let settings = settings(rise_levels);
        let recipe = validate_recipe_settings(&settings).expect("test settings should validate");
        let layout = resolve_layout(40, &settings).expect("Single layout should resolve");
        let patch =
            PatchRecipeContext::resolve(&layout, PatchId(0)).expect("Single patch should resolve");
        let catalog = runtime_art_catalog();
        let trees = TemperateTreeSet::resolve(catalog, "Crystal Ascent test")
            .expect("tracked tree set should resolve");
        let objects = CrystalAscentObjectSet::resolve(catalog)
            .expect("tracked Crystal Ascent objects should resolve");
        let fragment =
            construct_patch(patch, &recipe, 0.4, mode, &trees, &objects).unwrap_or_else(|issues| {
                panic!(
                    "Crystal Ascent patch should construct: {}",
                    issues
                        .into_iter()
                        .map(|issue| issue.detail)
                        .collect::<Vec<_>>()
                        .join("; ")
                )
            });
        (layout, fragment, objects)
    }

    fn raw_plan_with_mode(
        rise_levels: Level,
        mode: PatchBuildMode,
    ) -> (GeneratedWorldPlan, CrystalAscentObjectSet) {
        let (layout, fragment, objects) = raw_fragment(rise_levels, mode);
        let plan = compose_single_patch(layout, fragment)
            .expect("Crystal Ascent patch should compose as a Single world");
        (plan, objects)
    }

    fn raw_plan(rise_levels: Level) -> (GeneratedWorldPlan, CrystalAscentObjectSet) {
        raw_plan_with_mode(rise_levels, PatchBuildMode::CanonicalFallback)
    }

    fn exact_radius_site_layout() -> ResolvedLayoutPlan {
        let mask = HexCoord::ORIGIN
            .within_radius(SITE_RADIUS)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let edges = HexSide::ALL
            .into_iter()
            .map(|side| (side, ResolvedEdgeReference::WorldBoundary))
            .collect();
        ResolvedLayoutPlan {
            kind: LayoutKind::Single,
            grid_radius: 40,
            footprint: mask.clone(),
            patches: BTreeMap::from([(
                PatchId(0),
                ResolvedPatch {
                    biome_region: BiomeRegionId(0),
                    rotation_turns: 0,
                    mask,
                    edges,
                },
            )]),
            shared_edges: BTreeMap::new(),
            boundary_liquid_outlets: BTreeMap::new(),
        }
    }

    fn translated_rotated_macro_site_layout(rotation_turns: u8) -> ResolvedLayoutPlan {
        const FIXTURE_MACRO_RADIUS: u32 = 77;
        const FIXTURE_PATCH_RADIUS: u32 = 40;
        let center = HexCoord::from_axial(20, -10);
        let [center_x, center_y, center_z] = center.to_cubic_array();
        let crystal_mask = HexCoord::ORIGIN
            .within_radius(FIXTURE_PATCH_RADIUS)
            .into_iter()
            .map(|coord| {
                let [x, y, z] = coord.to_cubic_array();
                HexCoord::new_cubic(x + center_x, y + center_y, z + center_z)
            })
            .collect::<BTreeSet<_>>();
        let footprint = HexCoord::ORIGIN
            .within_radius(FIXTURE_MACRO_RADIUS)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let remainder = footprint
            .difference(&crystal_mask)
            .copied()
            .collect::<BTreeSet<_>>();
        let boundary_pairs = crystal_mask
            .iter()
            .flat_map(|coord| {
                coord
                    .neighbors()
                    .into_iter()
                    .filter(|neighbor| remainder.contains(neighbor))
                    .map(|neighbor| (*coord, neighbor))
            })
            .collect::<BTreeSet<_>>();
        let first_side = HexSide::ALL
            .into_iter()
            .max_by_key(|side| {
                boundary_pairs
                    .iter()
                    .filter(|(first, second)| side.neighbor(*first) == *second)
                    .count()
            })
            .expect("the translated fixture has a boundary direction");
        let edges = HexSide::ALL
            .into_iter()
            .map(|side| (side, ResolvedEdgeReference::WorldBoundary))
            .collect::<BTreeMap<_, _>>();
        ResolvedLayoutPlan {
            kind: LayoutKind::Macro,
            grid_radius: FIXTURE_MACRO_RADIUS,
            footprint,
            patches: BTreeMap::from([
                (
                    PatchId(0),
                    ResolvedPatch {
                        biome_region: BiomeRegionId(0),
                        rotation_turns,
                        mask: crystal_mask,
                        edges: edges.clone(),
                    },
                ),
                (
                    PatchId(1),
                    ResolvedPatch {
                        biome_region: BiomeRegionId(1),
                        rotation_turns: 0,
                        mask: remainder,
                        edges,
                    },
                ),
            ]),
            shared_edges: BTreeMap::from([(
                ResolvedEdgeId(0),
                ResolvedEdgeContract {
                    first: (PatchId(0), first_side),
                    second: (PatchId(1), first_side.opposite()),
                    elevation: ResolvedElevationBand {
                        preferred: 0,
                        min: 0,
                        max: 0,
                    },
                    walker: ResolvedWalkerPorts {
                        count: 0,
                        width: 0,
                        ports: Vec::new(),
                    },
                    liquid: ResolvedLiquidPort::Dry,
                    approach_depth: 0,
                    boundary_pairs,
                    protected_approaches: BTreeMap::from([
                        (PatchId(0), BTreeSet::new()),
                        (PatchId(1), BTreeSet::new()),
                    ]),
                },
            )]),
            boundary_liquid_outlets: BTreeMap::new(),
        }
    }

    fn validated_metrics(
        plan: &GeneratedWorldPlan,
        objects: &CrystalAscentObjectSet,
        rise_levels: Level,
    ) -> CrystalAscentMetrics {
        match validate_crystal_ascent(
            plan,
            &V3CrystalAscentSettings {
                base_level: 6,
                rise_levels,
            },
            objects,
        ) {
            WorldValidation::Valid(metrics) => metrics,
            WorldValidation::Invalid(issues) => panic!(
                "Crystal Ascent should validate: {}",
                issues
                    .into_iter()
                    .map(|issue| issue.detail)
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        }
    }

    #[test]
    fn showcase_geometry_is_recipe_valid() {
        let (plan, objects) = raw_plan(144);
        let metrics = validated_metrics(&plan, &objects, 144);
        assert_eq!(metrics.rise_levels, 144);
        assert_eq!(
            (metrics.circuits, metrics.flights, metrics.landings),
            (3, 18, 18)
        );
    }

    #[test]
    fn stair_treads_retain_exact_radial_stone_haunches() {
        for rise_levels in [100, 144, 200] {
            let (plan, objects) = raw_plan(rise_levels);
            let metrics = validated_metrics(&plan, &objects, rise_levels);
            let settings = V3CrystalAscentSettings {
                base_level: 6,
                rise_levels,
            };
            let circuits = (0..CIRCUIT_COUNT)
                .map(|circuit| {
                    expected_circuit_surfaces(&settings, circuit)
                        .expect("authored circuit should resolve")
                })
                .collect::<Vec<_>>();
            let haunches = expected_stair_haunch_thicknesses(&circuits)
                .expect("authored haunches should resolve");
            assert_eq!(haunches.len(), 1_668);

            let mut lane_thicknesses = BTreeMap::<u32, BTreeSet<Level>>::new();
            for (surface, thickness) in &haunches {
                let lane =
                    u32::try_from(*thickness).expect("haunch thickness should be positive") / 2;
                lane_thicknesses.entry(lane).or_default().insert(*thickness);

                let bottom = surface
                    .level
                    .saturating_add(1)
                    .saturating_sub(*thickness)
                    .max(0);
                for level in bottom..=surface.level {
                    let solid = solid_material_at(&plan.volume, surface.coord, level);
                    assert!(
                        solid.is_some(),
                        "rise {rise_levels} leaves air inside {thickness}-voxel haunch below {surface:?} at level {level}"
                    );
                    if level > settings.base_level {
                        assert_eq!(
                            solid,
                            Some(SolidMaterialRole::WorkedStone),
                            "elevated haunch support must be carved worked stone"
                        );
                    }
                }
            }
            assert_eq!(
                lane_thicknesses,
                BTreeMap::from([
                    (1, BTreeSet::from([2])),
                    (2, BTreeSet::from([4])),
                    (3, BTreeSet::from([6])),
                    (4, BTreeSet::from([8])),
                ])
            );
            assert_eq!(
                metrics.stair_surfaces as usize,
                expected_all_stair_surfaces(&settings).len()
            );
            for (circuit, (inner, outer)) in CIRCUIT_BANDS.iter().copied().enumerate() {
                let surfaces = circuits
                    .get(circuit)
                    .expect("authored circuit should exist");
                for surface in surfaces {
                    let radius = surface.coord.distance(HexCoord::ORIGIN);
                    if !(inner..=outer).contains(&radius) {
                        continue;
                    }
                    let expected = i32::try_from(
                        radius
                            .saturating_sub(inner)
                            .saturating_add(1)
                            .saturating_mul(2),
                    )
                    .expect("four authored lanes fit a level");
                    assert!(
                        haunches.get(surface).is_some_and(|actual| *actual >= expected),
                        "circuit {circuit} radius {radius} must deepen outward to at least {expected} voxels"
                    );
                }
            }
        }
    }

    #[test]
    fn malformed_outer_stair_haunch_fails_recipe_validation() {
        let (plan, objects) = raw_plan(144);
        let settings = V3CrystalAscentSettings {
            base_level: 6,
            rise_levels: 144,
        };
        let circuits = (0..CIRCUIT_COUNT)
            .map(|circuit| {
                expected_circuit_surfaces(&settings, circuit)
                    .expect("authored circuit should resolve")
            })
            .collect::<Vec<_>>();
        let haunches =
            expected_stair_haunch_thicknesses(&circuits).expect("authored haunches should resolve");
        let (surface, thickness) = haunches
            .iter()
            .find(|(surface, thickness)| **thickness == 8 && surface.level > 20)
            .map(|(surface, thickness)| (*surface, *thickness))
            .expect("fixture should contain an elevated outer-lane haunch");
        let bottom = surface.level + 1 - thickness;
        let mut missing = plan.clone();
        let column = missing
            .volume
            .columns
            .get_mut(&surface.coord)
            .expect("haunch column should exist");
        let mass = column
            .elements
            .iter_mut()
            .find_map(|element| match element {
                VolumeElement::Solid(mass)
                    if mass.material == SolidMaterialRole::WorkedStone
                        && mass.levels.bottom == bottom
                        && mass.levels.top == surface.level + 1 =>
                {
                    Some(mass)
                }
                _ => None,
            })
            .expect("outer-lane support should be one compact worked-stone run");
        mass.levels.bottom += 1;

        let WorldValidation::Invalid(issues) =
            validate_crystal_ascent(&missing, &settings, &objects)
        else {
            panic!("missing haunch voxel must fail recipe validation");
        };
        assert!(issues
            .iter()
            .any(|issue| issue.detail.contains("stair haunch")));

        let mut oversized = plan;
        let column = oversized
            .volume
            .columns
            .get_mut(&surface.coord)
            .expect("haunch column should exist");
        let mass = column
            .elements
            .iter_mut()
            .find_map(|element| match element {
                VolumeElement::Solid(mass)
                    if mass.material == SolidMaterialRole::WorkedStone
                        && mass.levels.bottom == bottom
                        && mass.levels.top == surface.level + 1 =>
                {
                    Some(mass)
                }
                _ => None,
            })
            .expect("outer-lane support should be one compact worked-stone run");
        mass.levels.bottom -= 1;
        let WorldValidation::Invalid(issues) =
            validate_crystal_ascent(&oversized, &settings, &objects)
        else {
            panic!("over-deep haunch must fail recipe validation");
        };
        assert!(issues.iter().any(|issue| issue
            .detail
            .contains("extends beneath its exact 8-voxel support")));
    }

    #[test]
    fn interior_review_anchors_are_stable_clear_and_fixture_aware() {
        let (plan, objects) = raw_plan(144);
        validated_metrics(&plan, &objects, 144);
        let chamber = plan
            .anchors
            .get(BOTTOM_CHAMBER)
            .copied()
            .expect("Crystal Ascent should publish its bottom-chamber review anchor");
        assert_eq!(chamber, TilePos::new(HexCoord::from_axial(-8, -8), 6));
        assert!(plan
            .volume
            .surface_headroom(chamber)
            .is_some_and(|headroom| headroom.0 >= 2));
        assert!(!plan.blockers.contains(&chamber));

        let settings = V3CrystalAscentSettings {
            base_level: 6,
            rise_levels: 144,
        };
        let anchor = plan
            .anchors
            .get(CORNER_LANDING)
            .copied()
            .expect("Crystal Ascent should publish its corner-landing review anchor");
        assert_eq!(anchor, TilePos::new(HexCoord::from_axial(-10, 21), 134));
        assert!(expected_landing_surfaces(&settings, 2, 4)
            .expect("upper corner landing should resolve")
            .contains(&anchor));
        assert!(plan
            .volume
            .surface_headroom(anchor)
            .is_some_and(|headroom| headroom.0 >= 8));
        assert_eq!(
            plan.lights
                .values()
                .filter(|light| {
                    light.origin.level == anchor.level
                        && light.origin.coord.distance(anchor.coord) == 1
                        && matches!(
                            light.presentation,
                            Some(PlannedLightPresentation::CrystalAscent(
                                CrystalAscentCrystalPresentation {
                                    kind: CrystalAscentCrystalKind::Landing(_),
                                    ..
                                }
                            ))
                        )
                })
                .count(),
            1
        );
    }

    #[test]
    fn chamber_metric_counts_only_ordinary_floor_outside_the_heart() {
        let (plan, objects) = raw_plan(144);
        let metrics = validated_metrics(&plan, &objects, 144);
        let chamber_area = 1_u32 + 3 * CHAMBER_RADIUS * CHAMBER_RADIUS.saturating_add(1);
        let heart_area = 1_u32 + 3 * 4 * 5;
        assert_eq!(metrics.chamber_surfaces, chamber_area - heart_area);
        assert_eq!(metrics.chamber_surfaces, 1_596);
    }

    #[test]
    fn lower_terminal_is_exterior_and_dark_interior_begins_one_ring_inward() {
        let (plan, objects) = raw_plan(144);
        validated_metrics(&plan, &objects, 144);
        let base = 6;
        let apron = radial_pad(SITE_RADIUS, 0, 4, base);
        assert!(apron.iter().all(|surface| {
            plan.volume
                .surfaces
                .get(surface)
                .is_some_and(|metadata| metadata.interior.is_none())
        }));

        let threshold = radial_pad(SITE_RADIUS - 1, 0, 4, base);
        let interior = plan
            .interiors
            .by_id
            .get(&INTERIOR)
            .expect("Crystal Ascent should publish its Dark interior");
        assert_eq!(interior.entrances, threshold);
        assert!(threshold.iter().all(|surface| {
            plan.volume
                .surfaces
                .get(surface)
                .is_some_and(|metadata| metadata.interior == Some(INTERIOR))
        }));

        let approach = expected_lower_interior_approach(&V3CrystalAscentSettings {
            base_level: base,
            rise_levels: 144,
        });
        assert_eq!(approach.len(), 16);
        assert!(approach.iter().all(|surface| {
            (SHELL_INNER_RADIUS..SITE_RADIUS).contains(&surface.coord.distance(HexCoord::ORIGIN))
                && plan
                    .volume
                    .surfaces
                    .get(surface)
                    .is_some_and(|metadata| metadata.interior == Some(INTERIOR))
        }));
    }

    #[test]
    fn summit_stair_remains_interior_until_it_reaches_the_exterior_crown() {
        let (plan, objects) = raw_plan(144);
        validated_metrics(&plan, &objects, 144);
        let settings = V3CrystalAscentSettings {
            base_level: 6,
            rise_levels: 144,
        };
        let summit = settings.base_level + settings.rise_levels;
        let stairs = expected_all_stair_surfaces(&settings);
        let summit_stairs = stairs
            .iter()
            .filter(|surface| surface.level == summit)
            .copied()
            .collect::<BTreeSet<_>>();
        assert!(!summit_stairs.is_empty());
        assert_eq!(OCULUS_RADIUS, 11);
        assert!(plan.volume.surfaces.keys().all(|surface| {
            surface.level != summit || surface.coord.distance(HexCoord::ORIGIN) >= OCULUS_RADIUS
        }));
        let rim = ring_coordinates(OCULUS_RADIUS)
            .into_iter()
            .map(|coord| TilePos::new(coord, summit))
            .collect::<BTreeSet<_>>();
        assert_eq!(rim.len(), 66);
        assert!(rim
            .iter()
            .all(|surface| plan.volume.surfaces.contains_key(surface)));
        for radius in OCULUS_RADIUS..=21 {
            assert_eq!(
                stairs
                    .iter()
                    .filter(|surface| {
                        surface.level == summit
                            && surface.coord.distance(HexCoord::ORIGIN) == radius
                    })
                    .count(),
                4,
                "summit ingress must remain exactly four wide at radius {radius}"
            );
        }
        assert!(summit_stairs.iter().all(|surface| {
            plan.volume
                .surfaces
                .get(surface)
                .is_some_and(|metadata| metadata.interior == Some(INTERIOR))
        }));

        let exterior_crown = expected_summit_clearing(&settings)
            .difference(&stairs)
            .next()
            .copied()
            .expect("summit clearing should include exterior cells beyond the stair threshold");
        assert!(plan
            .volume
            .surfaces
            .get(&exterior_crown)
            .is_some_and(|metadata| metadata.interior.is_none()));
    }

    #[test]
    fn contractions_publish_exact_four_cell_seams_and_four_disjoint_lanes() {
        for rise_levels in [100, 144, 200] {
            let (plan, objects) = raw_plan(rise_levels);
            validated_metrics(&plan, &objects, rise_levels);
            let recipe_settings = V3CrystalAscentSettings {
                base_level: 6,
                rise_levels,
            };
            let ordinary = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
            let circuits = (0..CIRCUIT_COUNT)
                .map(|circuit| {
                    expected_circuit_surfaces(&recipe_settings, circuit)
                        .expect("authored circuit should resolve")
                })
                .collect::<Vec<_>>();

            for left in 0..CIRCUIT_COUNT - 1 {
                let right = left + 1;
                let left_surfaces = circuits
                    .get(left)
                    .expect("left authored circuit should exist");
                let right_surfaces = circuits
                    .get(right)
                    .expect("right authored circuit should exist");
                let seam = expected_transfer_seam(&recipe_settings, left)
                    .expect("authored transfer seam should resolve");
                assert_eq!(seam.len(), 4);
                assert_eq!(
                    left_surfaces
                        .intersection(right_surfaces)
                        .copied()
                        .collect::<BTreeSet<_>>(),
                    seam
                );
                let mut claimed = BTreeSet::new();
                for lane_index in 0..4 {
                    let lane = expected_transfer_lane(&recipe_settings, left, lane_index)
                        .expect("authored transfer lane should resolve");
                    assert!(lane.iter().all(|surface| ordinary.contains(*surface)));
                    assert!(lane.windows(2).all(|pair| {
                        let [from, to] = pair else { return false };
                        ordinary.admits(*from, *to)
                    }));
                    assert!(lane.into_iter().all(|surface| claimed.insert(surface)));
                }

                let transfer_pad = expected_transfer_pad_cells(&recipe_settings, left)
                    .expect("authored transfer pads should resolve");
                assert_eq!(transfer_pad.len(), 28);
                assert!(transfer_pad.iter().all(|surface| {
                    ordinary.contains(*surface)
                        && plan
                            .volume
                            .surface_headroom(*surface)
                            .is_some_and(|headroom| headroom.0 >= 8)
                }));

                let expected_edges = expected_transfer_edge_allowlist(
                    &recipe_settings,
                    left,
                    left_surfaces,
                    right_surfaces,
                )
                .expect("authored transfer edge allowlist should resolve");
                let actual_edges = actual_transfer_edges(&ordinary, left_surfaces, right_surfaces);
                assert_eq!(actual_edges, expected_edges);
                assert_eq!(
                    actual_edges.len(),
                    21,
                    "rise {rise_levels} contraction {left}->{right} changed its exact transfer-edge topology"
                );
                assert_eq!(
                    actual_edges
                        .iter()
                        .filter(|(first, second)| {
                            !seam.contains(first) && !seam.contains(second)
                        })
                        .count(),
                    1,
                    "rise {rise_levels} contraction {left}->{right} should retain exactly one intentional hex-corner diagonal"
                );
            }
        }
    }

    #[test]
    fn transfer_edge_allowlist_rejects_a_non_corridor_edge() {
        let recipe_settings = V3CrystalAscentSettings {
            base_level: 6,
            rise_levels: 144,
        };
        let left = 0;
        let right = left + 1;
        let left_surfaces =
            expected_circuit_surfaces(&recipe_settings, left).expect("left circuit should resolve");
        let right_surfaces = expected_circuit_surfaces(&recipe_settings, right)
            .expect("right circuit should resolve");
        let expected = expected_transfer_edge_allowlist(
            &recipe_settings,
            left,
            &left_surfaces,
            &right_surfaces,
        )
        .expect("authored transfer edge allowlist should resolve");
        let seam_level = flight_boundary(
            recipe_settings.base_level,
            recipe_settings.rise_levels,
            FLIGHTS_PER_CIRCUIT,
        );
        let shared_radius = CIRCUIT_BANDS
            .get(left)
            .expect("left circuit band should exist")
            .0;
        let forbidden = canonical_transfer_edge(
            TilePos::new(landing_coord(shared_radius, 3), seam_level),
            TilePos::new(
                landing_coord(shared_radius.saturating_sub(1), 3),
                seam_level,
            ),
        )
        .expect("the synthetic corruption should be one geometric walker edge");
        assert!(!expected.contains(&forbidden));

        let mut corrupted = expected.clone();
        assert!(corrupted.insert(forbidden));
        assert!(!transfer_edges_are_exact(&expected, &corrupted));
    }

    #[test]
    fn accepted_rises_have_exact_boundaries_and_four_flat_incoming_pad_treads() {
        let base = 6;
        for rise_levels in [100, 144, 200] {
            for flight in 0..=FLIGHT_COUNT {
                let expected = base
                    + i32::try_from(flight).expect("fixture flight fits i32") * rise_levels
                        / i32::try_from(FLIGHT_COUNT).expect("fixture flight count fits i32");
                assert_eq!(
                    flight_boundary(base, rise_levels, flight),
                    expected,
                    "rise {rise_levels} flight {flight} changed its exact boundary"
                );
            }
            assert_eq!(
                flight_boundary(base, rise_levels, FLIGHT_COUNT),
                base + rise_levels
            );

            for circuit in 1..CIRCUIT_COUNT {
                let (inner, outer) = *CIRCUIT_BANDS
                    .get(circuit)
                    .expect("accepted circuit should have an authored band");
                let flight = circuit * FLIGHTS_PER_CIRCUIT;
                let boundary_low = base
                    + i32::try_from(flight).expect("fixture flight fits i32") * rise_levels
                        / i32::try_from(FLIGHT_COUNT).expect("fixture flight count fits i32");
                let boundary_high = base
                    + i32::try_from(flight + 1).expect("fixture flight fits i32") * rise_levels
                        / i32::try_from(FLIGHT_COUNT).expect("fixture flight count fits i32");
                for radius in inner..=outer {
                    for offset in 0..4 {
                        assert_eq!(
                            flight_tread_level(
                                circuit,
                                0,
                                radius,
                                offset,
                                boundary_low,
                                boundary_high,
                            ),
                            Some(boundary_low),
                            "rise {rise_levels} circuit {circuit} radius {radius} offset {offset} must remain on the flat transfer pad"
                        );
                    }

                    let mut levels = (0..usize::try_from(radius)
                        .expect("fixture radius fits usize"))
                        .map(|offset| {
                            flight_tread_level(
                                circuit,
                                0,
                                radius,
                                offset,
                                boundary_low,
                                boundary_high,
                            )
                            .expect("accepted fixture tread should resolve")
                        })
                        .collect::<Vec<_>>();
                    levels.push(boundary_high);
                    assert!(levels.windows(2).all(|pair| {
                        let [from, to] = pair else { return false };
                        from <= to && from.abs_diff(*to) <= 1
                    }));
                }
            }
        }
    }

    #[test]
    fn validator_rejects_a_transfer_seam_with_only_three_lanes() {
        let (mut plan, objects) = raw_plan(144);
        let recipe_settings = V3CrystalAscentSettings {
            base_level: 6,
            rise_levels: 144,
        };
        let removed = expected_transfer_seam(&recipe_settings, 0)
            .and_then(|seam| seam.iter().next().copied())
            .expect("first transfer should have a seam cell");
        assert!(plan.volume.surfaces.remove(&removed).is_some());
        for id in [StructureId(1), StructureId(2)] {
            assert!(plan
                .structures
                .by_id
                .get_mut(&id)
                .expect("adjacent circuit structure should exist")
                .voxels
                .remove(&removed));
        }
        let WorldValidation::Invalid(issues) =
            validate_crystal_ascent(&plan, &recipe_settings, &objects)
        else {
            panic!("removing one transfer lane must invalidate Crystal Ascent")
        };
        assert!(issues.iter().any(|issue| {
            issue.detail.contains("four independent transfer lanes")
                || issue.detail.contains("four-cell transfer seam")
        }));
    }

    #[test]
    fn exact_radius_32_site_constructs_after_translation_and_every_rotation() {
        let recipe_settings = V3CrystalAscentSettings {
            base_level: 6,
            rise_levels: 144,
        };
        let catalog = runtime_art_catalog();
        let trees = TemperateTreeSet::resolve(catalog, "Crystal Ascent exact-site test")
            .expect("tracked tree set should resolve");
        let objects = CrystalAscentObjectSet::resolve(catalog)
            .expect("tracked Crystal Ascent objects should resolve");
        let layout = exact_radius_site_layout();
        layout
            .validate()
            .expect("exact radius-32 fixture should be a valid Single patch layout");
        let patch = PatchRecipeContext::resolve(&layout, PatchId(0))
            .expect("exact-site patch should resolve");
        assert_eq!(
            patch
                .local_frame()
                .expect("exact-site frame should resolve")
                .local_mask(patch.mask())
                .expect("exact-site mask should normalize"),
            HexCoord::ORIGIN
                .within_radius(SITE_RADIUS)
                .into_iter()
                .collect()
        );
        let fragment = construct_patch(
            patch,
            &recipe_settings,
            0.4,
            PatchBuildMode::CanonicalFallback,
            &trees,
            &objects,
        )
        .unwrap_or_else(|issues| {
            panic!(
                "the exact radius-32 site should construct: {}",
                issues
                    .into_iter()
                    .map(|issue| issue.detail)
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        });
        assert_eq!(fragment.volume.mask, layout.footprint);

        let center = HexCoord::from_axial(20, -10);
        for rotation_turns in 0..6 {
            let frame = LocalPatchFrame::from_resolved_ring19(center, SITE_RADIUS, rotation_turns);
            let expected_world_mask = HexCoord::ORIGIN
                .within_radius(SITE_RADIUS)
                .into_iter()
                .map(|coord| {
                    frame
                        .to_world(coord)
                        .expect("small translated site coordinate should remain in range")
                })
                .collect::<BTreeSet<_>>();
            let mut transformed = fragment.clone();
            frame
                .patch_to_world(&mut transformed)
                .expect("exact-site semantics should translate and rotate");
            assert_eq!(transformed.volume.mask, expected_world_mask);
            assert_eq!(
                frame
                    .canonical_local_world(&transformed)
                    .expect("exact-site semantics should normalize")
                    .volume,
                fragment.volume
            );
        }
    }

    #[test]
    fn real_macro_patch_context_constructs_at_a_translated_site_in_every_rotation() {
        let recipe_settings = V3CrystalAscentSettings {
            base_level: 6,
            rise_levels: 144,
        };
        let (_, canonical, _) = raw_fragment(144, PatchBuildMode::CanonicalFallback);
        let catalog = runtime_art_catalog();
        let trees = TemperateTreeSet::resolve(catalog, "Crystal Ascent Macro-frame test")
            .expect("tracked tree set should resolve");
        let objects = CrystalAscentObjectSet::resolve(catalog)
            .expect("tracked Crystal Ascent objects should resolve");
        let translated_center = HexCoord::from_axial(20, -10);

        for rotation_turns in 0..6 {
            let layout = translated_rotated_macro_site_layout(rotation_turns);
            layout
                .validate()
                .unwrap_or_else(|error| panic!("translated Macro fixture must validate: {error}"));
            let patch = PatchRecipeContext::resolve(&layout, PatchId(0))
                .expect("translated Macro patch should resolve through the production context");
            let frame = patch
                .local_frame()
                .expect("translated Macro patch should resolve its production frame");
            assert_eq!(frame.center(), translated_center);
            let fragment = construct_patch(
                patch,
                &recipe_settings,
                0.4,
                PatchBuildMode::CanonicalFallback,
                &trees,
                &objects,
            )
            .unwrap_or_else(|issues| {
                panic!(
                    "translated/rotated Macro patch should construct: {}",
                    issues
                        .into_iter()
                        .map(|issue| issue.detail)
                        .collect::<Vec<_>>()
                        .join("; ")
                )
            });
            assert!(matches!(
                validate_composite_fragment(&fragment, &layout, &recipe_settings, &objects),
                WorldValidation::Valid(_)
            ));
            assert_eq!(
                fragment.volume.mask,
                layout
                    .patches
                    .get(&PatchId(0))
                    .expect("translated Crystal Ascent patch should exist")
                    .mask
            );
            let heart_rotation = fragment
                .lights
                .values()
                .find_map(|light| match light.presentation {
                    Some(PlannedLightPresentation::CrystalAscent(
                        CrystalAscentCrystalPresentation {
                            kind: CrystalAscentCrystalKind::Heart,
                            rotation,
                        },
                    )) => Some(rotation),
                    _ => None,
                })
                .expect("translated Crystal Ascent should retain its heart presentation");
            assert_eq!(
                heart_rotation, rotation_turns,
                "Macro patch rotation must compose into asymmetric heart occupancy"
            );
            for anchor in [LOWER_ENTRY, BOTTOM_CHAMBER, UPPER_EXIT] {
                let expected = frame
                    .position_to_world(
                        *canonical
                            .anchors
                            .get(anchor)
                            .expect("canonical Crystal Ascent anchor should exist"),
                    )
                    .expect("fixture anchor should translate and rotate");
                assert_eq!(
                    fragment.anchors.get(anchor).copied(),
                    Some(expected),
                    "translated Crystal Ascent anchor should exist"
                );
            }

            let local = frame
                .canonical_local_world(&fragment)
                .expect("production Macro semantics should normalize");
            let authored_site = HexCoord::ORIGIN
                .within_radius(SITE_RADIUS)
                .into_iter()
                .collect::<BTreeSet<_>>();
            for coord in &authored_site {
                assert_eq!(
                    local.volume.columns.get(coord),
                    canonical.volume.columns.get(coord),
                    "Macro composition must preserve exact authored-site volume at {coord:?}"
                );
            }
            let local_site_surfaces = local
                .volume
                .surfaces
                .iter()
                .filter(|(surface, _)| authored_site.contains(&surface.coord))
                .collect::<BTreeMap<_, _>>();
            let canonical_site_surfaces = canonical
                .volume
                .surfaces
                .iter()
                .filter(|(surface, _)| authored_site.contains(&surface.coord))
                .collect::<BTreeMap<_, _>>();
            assert_eq!(local_site_surfaces, canonical_site_surfaces);
            let composite_fringe = local
                .volume
                .mask
                .difference(&authored_site)
                .copied()
                .collect::<BTreeSet<_>>();
            assert!(!composite_fringe.is_empty());
            for coord in &composite_fringe {
                let surface = TilePos::new(*coord, 150);
                assert!(local.volume.surfaces.contains_key(&surface));
                assert_eq!(
                    solid_material_at(&local.volume, *coord, 150),
                    Some(SolidMaterialRole::Grass)
                );
                assert!(!local.volume.surfaces.contains_key(&TilePos::new(*coord, 6)));
            }
            assert_eq!(local.features, canonical.features);
            assert_eq!(local.structures, canonical.structures);
            assert_eq!(local.blockers, canonical.blockers);
            assert_eq!(local.lights, canonical.lights);
            assert_eq!(
                local
                    .biome_regions
                    .iter()
                    .filter(|(surface, _)| authored_site.contains(&surface.coord))
                    .collect::<BTreeMap<_, _>>(),
                canonical
                    .biome_regions
                    .iter()
                    .filter(|(surface, _)| authored_site.contains(&surface.coord))
                    .collect::<BTreeMap<_, _>>()
            );
            assert_eq!(local.interiors, canonical.interiors);
            assert_eq!(local.anchors, canonical.anchors);
        }
    }

    #[test]
    fn boundary_rises_preserve_all_authored_geometry_contracts() {
        for rise_levels in [100, 144, 200] {
            let (plan, objects) = raw_plan(rise_levels);
            let metrics = validated_metrics(&plan, &objects, rise_levels);
            assert_eq!(metrics.rise_levels, rise_levels);
            assert_eq!(
                (metrics.circuits, metrics.flights, metrics.landings),
                (3, 18, 18)
            );
            assert!(metrics.minimum_stair_headroom >= 4);
            assert_eq!(metrics.crystal_fixtures, 19);
            assert_eq!(metrics.gameplay_lights, 38);
        }
    }

    #[test]
    fn validator_rejects_a_landing_light_moved_to_another_fixture_origin() {
        let (mut plan, objects) = raw_plan(144);
        let recipe_settings = V3CrystalAscentSettings {
            base_level: 6,
            rise_levels: 144,
        };
        let fixture_origins = expected_landing_alcoves(&recipe_settings);
        let [source, destination, ..] = fixture_origins.as_slice() else {
            panic!("landing fixtures should expose at least two origins")
        };
        let moved = plan.lights.values_mut().find(|light| {
            light.origin == *source
                && light.level == IlluminationLevel::Dim
                && light.radius == LANDING_DIM_RADIUS
                && light.presentation.is_none()
        });
        moved
            .expect("the source fixture should own its exact Dim light")
            .origin = *destination;

        let WorldValidation::Invalid(issues) =
            validate_crystal_ascent(&plan, &recipe_settings, &objects)
        else {
            panic!("moving one light to another fixture must invalidate Crystal Ascent")
        };
        assert!(issues.iter().any(|issue| issue
            .detail
            .contains("exact Bright/Dim gameplay-light pair")));
    }

    #[test]
    fn validator_rejects_a_malformed_heart_light_pair() {
        let (mut plan, objects) = raw_plan(144);
        let recipe_settings = V3CrystalAscentSettings {
            base_level: 6,
            rise_levels: 144,
        };
        let heart_origin = TilePos::new(HexCoord::ORIGIN, recipe_settings.base_level);
        let malformed = plan.lights.values_mut().find(|light| {
            light.origin == heart_origin
                && light.level == IlluminationLevel::Dim
                && light.radius == HEART_DIM_RADIUS
                && light.presentation.is_none()
        });
        malformed
            .expect("the cathedral heart should own its exact Dim light")
            .radius = HEART_DIM_RADIUS.saturating_sub(1);

        let WorldValidation::Invalid(issues) =
            validate_crystal_ascent(&plan, &recipe_settings, &objects)
        else {
            panic!("malforming one heart light must invalidate Crystal Ascent")
        };
        assert!(issues.iter().any(|issue| issue
            .detail
            .contains("cathedral heart must own one exact Bright/Dim gameplay-light pair")));
    }

    #[test]
    fn seed_changes_only_decorative_crystal_and_tree_choices() {
        let (reference, objects) = raw_plan_with_mode(
            144,
            PatchBuildMode::Candidate {
                world_seed: 7,
                candidate: 0,
            },
        );
        validated_metrics(&reference, &objects, 144);
        for seed in [11, 808, 4_294_967_311] {
            let (candidate, candidate_objects) = raw_plan_with_mode(
                144,
                PatchBuildMode::Candidate {
                    world_seed: seed,
                    candidate: 0,
                },
            );
            validated_metrics(&candidate, &candidate_objects, 144);
            assert_eq!(candidate.volume, reference.volume);
            assert_eq!(candidate.structures, reference.structures);
            assert_eq!(candidate.interiors, reference.interiors);
            assert_eq!(candidate.anchors, reference.anchors);
            assert_eq!(
                candidate.features.protected_routes,
                reference.features.protected_routes
            );
            assert_eq!(candidate.features.clearings, reference.features.clearings);
            assert_eq!(
                candidate
                    .lights
                    .values()
                    .map(|light| (light.origin, light.level, light.radius))
                    .collect::<Vec<_>>(),
                reference
                    .lights
                    .values()
                    .map(|light| (light.origin, light.level, light.radius))
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn patch_semantics_round_trip_through_all_six_rotations_and_translation() {
        let (_, fragment, _) = raw_fragment(144, PatchBuildMode::CanonicalFallback);
        for turns in 0..6 {
            let frame =
                LocalPatchFrame::from_resolved_ring19(HexCoord::from_axial(80, -40), 40, turns);
            let mut transformed = fragment.clone();
            frame
                .patch_to_world(&mut transformed)
                .expect("translated Crystal Ascent semantics should remain exact");
            for (name, local) in &fragment.anchors {
                assert_eq!(
                    transformed.anchors.get(name).copied(),
                    Some(
                        frame
                            .position_to_world(*local)
                            .expect("translated anchor should remain in range")
                    )
                );
            }
            let round_trip = frame
                .canonical_local_world(&transformed)
                .expect("rotated Crystal Ascent semantics should normalize");
            assert_eq!(round_trip.volume, fragment.volume);
            assert_eq!(round_trip.features, fragment.features);
            assert_eq!(round_trip.structures, fragment.structures);
            assert_eq!(round_trip.blockers, fragment.blockers);
            assert_eq!(round_trip.lights, fragment.lights);
            assert_eq!(round_trip.biome_regions, fragment.biome_regions);
            assert_eq!(round_trip.interiors, fragment.interiors);
            assert_eq!(round_trip.anchors, fragment.anchors);
        }
    }

    #[test]
    fn cutaway_is_nonempty_exact_shell_and_excludes_route_openings() {
        let (plan, objects) = raw_plan(144);
        validated_metrics(&plan, &objects, 144);
        let roofs = &plan
            .interiors
            .by_id
            .get(&INTERIOR)
            .expect("Crystal Ascent interior should exist")
            .roof_voxels;
        assert!(!roofs.is_empty());
        let stairs = plan
            .structures
            .by_id
            .values()
            .filter(|structure| structure.kind == StructureKind::Stair)
            .flat_map(|structure| structure.voxels.iter().copied())
            .collect::<BTreeSet<_>>();
        assert!(roofs.is_disjoint(&stairs));
        assert!(roofs.iter().all(|position| {
            (SHELL_INNER_RADIUS..=BUTTRESS_OUTER_RADIUS)
                .contains(&position.coord.distance(HexCoord::ORIGIN))
                && position.coord.distance(HexCoord::ORIGIN) >= OCULUS_RADIUS
                && !(is_lower_aperture(position.coord)
                    && position.level
                        <= 6_i32.saturating_add(pointed_arch_clearance(position.coord)))
        }));
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct BenchmarkCounts {
        columns: usize,
        surfaces: usize,
        solid_runs: usize,
        solid_voxels_by_material: BTreeMap<SolidMaterialRole, u64>,
        structure_voxels: usize,
        feature_roots: usize,
        gameplay_lights: usize,
        roof_voxels: usize,
    }

    fn benchmark_counts(plan: &GeneratedWorldPlan) -> BenchmarkCounts {
        let mut solid_runs = 0_usize;
        let mut solid_voxels_by_material = BTreeMap::<SolidMaterialRole, u64>::new();
        for column in plan.volume.columns.values() {
            for element in &column.elements {
                let VolumeElement::Solid(mass) = *element else {
                    continue;
                };
                solid_runs = solid_runs.saturating_add(1);
                let voxels = u64::from(
                    mass.levels
                        .top
                        .saturating_sub(mass.levels.bottom)
                        .unsigned_abs(),
                );
                let count = solid_voxels_by_material.entry(mass.material).or_default();
                *count = count.saturating_add(voxels);
            }
        }
        BenchmarkCounts {
            columns: plan.volume.columns.len(),
            surfaces: plan.volume.surfaces.len(),
            solid_runs,
            solid_voxels_by_material,
            structure_voxels: plan
                .structures
                .by_id
                .values()
                .map(|structure| structure.voxels.len())
                .sum(),
            feature_roots: plan.features.by_id.len(),
            gameplay_lights: plan.lights.len(),
            roof_voxels: plan
                .interiors
                .by_id
                .values()
                .map(|interior| interior.roof_voxels.len())
                .sum(),
        }
    }

    #[test]
    #[ignore = "manual release/debug Crystal Ascent generation and allocation benchmark"]
    fn crystal_ascent_boundary_rise_benchmark_tracks_timing_and_plan_counts() {
        for rise_levels in [100, 144, 200] {
            let warmup = raw_plan_with_mode(
                rise_levels,
                PatchBuildMode::Candidate {
                    world_seed: u64::MAX,
                    candidate: 0,
                },
            );
            std::hint::black_box(warmup);

            let mut samples = Vec::new();
            let mut expected_counts = None;
            for seed in 0..6 {
                let started = std::time::Instant::now();
                let (plan, objects) = raw_plan_with_mode(
                    rise_levels,
                    PatchBuildMode::Candidate {
                        world_seed: seed,
                        candidate: 0,
                    },
                );
                validated_metrics(&plan, &objects, rise_levels);
                samples.push(started.elapsed());
                let counts = benchmark_counts(&plan);
                if let Some(expected) = &expected_counts {
                    assert_eq!(
                        &counts, expected,
                        "seed {seed} changed rise-{rise_levels} plan/material counts"
                    );
                } else {
                    expected_counts = Some(counts);
                }
                std::hint::black_box(plan);
            }
            samples.sort_unstable();
            let median = samples
                .get(samples.len() / 2)
                .copied()
                .expect("benchmark records six samples");
            let p95 = samples
                .last()
                .copied()
                .expect("benchmark records six samples");
            let counts = expected_counts.expect("benchmark records stable plan counts");
            eprintln!(
                "Crystal Ascent rise {rise_levels}: median={median:?} p95={p95:?} counts={counts:?}"
            );
            assert!(median <= p95);
            assert!(counts.columns > 0 && counts.solid_runs > 0 && counts.roof_voxels > 0);
        }
    }

    #[test]
    fn validator_rejects_a_missing_route_lane_voxel() {
        let (mut plan, objects) = raw_plan(144);
        let structure = plan
            .structures
            .by_id
            .get_mut(&StructureId(1))
            .expect("first circuit structure should exist");
        let removed = structure
            .voxels
            .iter()
            .next()
            .copied()
            .expect("first circuit should contain route voxels");
        assert!(structure.voxels.remove(&removed));
        let validation = validate_crystal_ascent(
            &plan,
            &V3CrystalAscentSettings {
                base_level: 6,
                rise_levels: 144,
            },
            &objects,
        );
        let WorldValidation::Invalid(issues) = validation else {
            panic!("missing a circuit voxel must invalidate Crystal Ascent")
        };
        assert!(issues.iter().any(|issue| {
            issue.detail.contains("does not equal authored circuit")
                || issue.detail.contains("exact authored route")
        }));
    }
}
