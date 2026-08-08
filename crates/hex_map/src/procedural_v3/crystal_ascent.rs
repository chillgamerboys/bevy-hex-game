//! Deterministic authored Crystal Ascent landmark.
//!
//! The landmark is a true stacked V3 volume: the chamber remains open, stair
//! treads are one-voxel platforms, and the shell/crown are separate occupied runs.
//! Only crystal silhouettes and summit trees vary with the seed.

use std::collections::{BTreeMap, BTreeSet};

use hex_assets::{HexObjectRotation, RuntimeArtCatalog};
use hex_core::{
    upper_dome_contains, ExactGridPoint, HexCoord, IlluminationLevel, InteriorRegionId, Level,
    MapViewHint, SpecialMovementRegion, TilePos,
};

use super::composition::{compose_single_patch, GeneratedPatchPlan};
use super::layout::{resolve_layout, PatchId, ResolvedLayoutPlan};
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
const HEART_RADIUS: u32 = 4;
const OCULUS_RADIUS: u32 = 12;
const CLEARING_RADIUS: u32 = 18;
const SHELL_INNER_RADIUS: u32 = 28;
const SHELL_OUTER_RADIUS: u32 = 32;
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
    chamber_surfaces: BTreeSet<TilePos>,
    crown_surfaces: BTreeSet<TilePos>,
    landing_surfaces: Vec<BTreeSet<TilePos>>,
    landing_alcoves: Vec<TilePos>,
    lower_pad: BTreeSet<TilePos>,
    upper_pad: BTreeSet<TilePos>,
    lower_route: BTreeSet<TilePos>,
    summit_trail: BTreeSet<TilePos>,
    summit_clearing: BTreeSet<TilePos>,
    lower_entry: TilePos,
    bottom_chamber: TilePos,
    upper_exit: TilePos,
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
    CrystalAscentObjectSet::resolve(art_catalog).map_err(|error| {
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
        validate_crystal_ascent(plan, &self.settings)
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
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    let streams = mode.seed_streams(&patch);
    let streams = streams.map(|streams| CrystalAscentStreams {
        crystal_kinds: streams.stage("crystal_ascent.crystals.kind"),
        crystal_rotations: streams.stage("crystal_ascent.crystals.rotation"),
        tree_sites: streams.stage("crystal_ascent.summit.trees"),
        tree_rotations: streams.stage("crystal_ascent.summit.tree_rotation"),
    });
    construct_patch_with_streams(patch, settings, level_height, streams, trees)
}

fn construct_patch_with_streams(
    patch: PatchRecipeContext<'_>,
    settings: &V3CrystalAscentSettings,
    level_height: f32,
    streams: Option<CrystalAscentStreams<'_>>,
    trees: &TemperateTreeSet,
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
    let mut masses = local_mask
        .iter()
        .copied()
        .map(|coord| (coord, Vec::<MassSpec>::new()))
        .collect::<BTreeMap<_, _>>();
    let mut surface_intents = BTreeMap::<TilePos, SurfaceMetadata>::new();

    for coord in &local_mask {
        let radius = coord.distance(HexCoord::ORIGIN);
        if radius > SITE_RADIUS {
            add_ground(&mut masses, *coord, base, SolidMaterialRole::Grass);
            surface_intents.insert(TilePos::new(*coord, base), exterior_surface());
        } else if radius <= CHAMBER_RADIUS {
            add_ground(&mut masses, *coord, base, SolidMaterialRole::WorkedStone);
            surface_intents.insert(TilePos::new(*coord, base), interior_surface());
        } else if is_lower_aperture(*coord) {
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
    let stair_surfaces = geometry.stair_surfaces.clone();
    build_crown(
        &mut masses,
        &mut surface_intents,
        &stair_surfaces,
        summit,
        &mut geometry,
    );

    let volume = finish_volume(local_mask.clone(), masses, surface_intents)?;
    let features = build_features(&volume, &geometry, streams, trees)?;
    let mut blockers = features
        .by_id
        .values()
        .flat_map(|feature| feature.blocker_footprint.iter().copied())
        .collect::<BTreeSet<_>>();
    blockers.extend(
        HexCoord::ORIGIN
            .within_radius(HEART_RADIUS)
            .into_iter()
            .map(|coord| TilePos::new(coord, base)),
    );
    let lights = build_lights(&geometry, base, streams);
    let structures = build_structures(&volume, &geometry);
    let interior_floors = volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| {
            (metadata.interior == Some(INTERIOR)).then_some(*position)
        })
        .collect::<BTreeSet<_>>();
    let entrance_surfaces = radial_pad(32, 0, 4, base)
        .into_iter()
        .filter(|position| interior_floors.contains(position))
        .collect();
    let interiors = InteriorPlan {
        by_id: BTreeMap::from([(
            INTERIOR,
            PlannedInterior {
                floors: interior_floors,
                entrances: entrance_surfaces,
                roof_voxels: BTreeSet::new(),
            },
        )]),
    };
    let anchors = BTreeMap::from([
        (PARTY_START.to_owned(), geometry.lower_entry),
        (HOSTILE_START.to_owned(), geometry.upper_exit),
        (LOWER_ENTRY.to_owned(), geometry.lower_entry),
        (BOTTOM_CHAMBER.to_owned(), geometry.bottom_chamber),
        (UPPER_EXIT.to_owned(), geometry.upper_exit),
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

fn build_stairs(
    masses: &mut BTreeMap<HexCoord, Vec<MassSpec>>,
    surface_intents: &mut BTreeMap<TilePos, SurfaceMetadata>,
    settings: &V3CrystalAscentSettings,
) -> Result<AuthoredGeometry, Vec<WorldValidationIssue>> {
    let base = settings.base_level;
    let summit = base.saturating_add(settings.rise_levels);
    let mut stair_surfaces = BTreeSet::new();
    let mut landing_surfaces = Vec::with_capacity(LANDING_COUNT);
    let mut landing_alcoves = Vec::with_capacity(LANDING_COUNT);

    for (circuit, (inner, outer)) in CIRCUIT_BANDS.into_iter().enumerate() {
        for radius in inner..=outer {
            let ring = ring_coordinates(radius);
            let len = ring.len();
            let start = start_index(radius);
            for progress in 0..len {
                let raw = (start + progress) % len;
                let side = progress / usize::try_from(radius).unwrap_or(usize::MAX);
                let offset = progress % usize::try_from(radius).unwrap_or(usize::MAX);
                let flight = circuit * FLIGHTS_PER_CIRCUIT + side;
                let boundary_low = flight_boundary(base, settings.rise_levels, flight);
                let boundary_high = flight_boundary(base, settings.rise_levels, flight + 1);
                let delta = boundary_high.saturating_sub(boundary_low);
                let level = boundary_low.saturating_add(
                    i32::try_from(offset)
                        .unwrap_or(i32::MAX)
                        .saturating_mul(delta)
                        .checked_div(i32::try_from(radius).unwrap_or(1))
                        .unwrap_or_default(),
                );
                let position = TilePos::new(ring[raw], level);
                add_platform(masses, position, SolidMaterialRole::WorkedStone)?;
                surface_intents.insert(position, interior_surface());
                stair_surfaces.insert(position);
            }
        }

        for side in 0..FLIGHTS_PER_CIRCUIT {
            let level = flight_boundary(
                base,
                settings.rise_levels,
                circuit * FLIGHTS_PER_CIRCUIT + side,
            );
            let landing = (inner..=outer)
                .map(|radius| TilePos::new(landing_coord(radius, side), level))
                .collect::<BTreeSet<_>>();
            landing_surfaces.push(landing);
            let alcove = TilePos::new(landing_coord(outer.saturating_add(1), side), level);
            add_platform(masses, alcove, SolidMaterialRole::WorkedStone)?;
            surface_intents.insert(alcove, interior_surface());
            landing_alcoves.push(alcove);
        }
    }

    // Full-width transfer landings join the high seam of one circuit to the low
    // seam of the next without introducing a cross-loop keyhole elsewhere.
    for circuit in 0..(CIRCUIT_COUNT - 1) {
        let boundary = (circuit + 1) * FLIGHTS_PER_CIRCUIT;
        let next_inner = CIRCUIT_BANDS[circuit + 1].0;
        let outer = CIRCUIT_BANDS[circuit].1;
        let level = flight_boundary(base, settings.rise_levels, boundary);
        for radius in next_inner..=outer {
            let position = TilePos::new(landing_coord(radius, 0), level);
            add_platform(masses, position, SolidMaterialRole::WorkedStone)?;
            surface_intents.insert(position, interior_surface());
            stair_surfaces.insert(position);
        }
    }

    for radius in CHAMBER_RADIUS..=CIRCUIT_BANDS[0].1 {
        let position = TilePos::new(landing_coord(radius, 0), base);
        add_platform(masses, position, SolidMaterialRole::WorkedStone)?;
        surface_intents.insert(position, interior_surface());
        stair_surfaces.insert(position);
    }
    for radius in OCULUS_RADIUS..=CIRCUIT_BANDS[2].1 {
        let position = TilePos::new(landing_coord(radius, 0), summit);
        add_platform(masses, position, SolidMaterialRole::WorkedStone)?;
        surface_intents.insert(
            position,
            if radius >= CIRCUIT_BANDS[2].0 {
                interior_surface()
            } else {
                exterior_surface()
            },
        );
        stair_surfaces.insert(position);
    }

    let chamber_surfaces = HexCoord::ORIGIN
        .within_radius(CHAMBER_RADIUS)
        .into_iter()
        .map(|coord| TilePos::new(coord, base))
        .collect::<BTreeSet<_>>();
    let lower_pad = radial_pad(35, 0, 4, base);
    let upper_pad = radial_pad(31, 3, 4, summit);
    let mut lower_route = BTreeSet::new();
    for radius in CHAMBER_RADIUS..=37 {
        lower_route.extend(radial_pad(radius, 0, 4, base));
    }
    let bottom_chamber = TilePos::new(HexCoord::from_axial(5, 0), base);
    let lower_entry = *lower_pad
        .iter()
        .nth(1)
        .unwrap_or(&TilePos::new(HexCoord::ORIGIN, base));
    let upper_exit = *upper_pad
        .iter()
        .nth(1)
        .unwrap_or(&TilePos::new(HexCoord::ORIGIN, summit));
    Ok(AuthoredGeometry {
        stair_surfaces,
        chamber_surfaces,
        crown_surfaces: BTreeSet::new(),
        landing_surfaces,
        landing_alcoves,
        lower_pad,
        upper_pad,
        lower_route,
        summit_trail: BTreeSet::new(),
        summit_clearing: BTreeSet::new(),
        lower_entry,
        bottom_chamber,
        upper_exit,
    })
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
            push_mass(
                column,
                0,
                landing.saturating_add(1),
                SolidMaterialRole::WorkedStone,
            );
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
                && crown_bottom.saturating_sub(surface.level.saturating_add(1)) < 4
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
    let stair_voxels = geometry.stair_surfaces.clone();
    let mut shell_voxels = BTreeSet::new();
    for (coord, column) in &volume.columns {
        let radius = coord.distance(HexCoord::ORIGIN);
        if !(SHELL_INNER_RADIUS..=SHELL_OUTER_RADIUS).contains(&radius) {
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
        let band = CIRCUIT_BANDS[circuit];
        let voxels = stair_voxels
            .iter()
            .copied()
            .filter(|position| {
                let radius = position.coord.distance(HexCoord::ORIGIN);
                (band.0..=band.1).contains(&radius)
            })
            .collect::<BTreeSet<_>>();
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
    if !distances.contains_key(&upper) {
        issues.push(recipe_issue(
            "ordinary traversal does not connect the lower entry to the upper exit",
        ));
    }
    let stair_surfaces = plan
        .structures
        .by_id
        .values()
        .filter(|structure| structure.kind == StructureKind::Stair)
        .flat_map(|structure| structure.voxels.iter().copied())
        .collect::<BTreeSet<_>>();
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
    let dim_sources = plan
        .lights
        .values()
        .filter(|light| light.level == IlluminationLevel::Dim)
        .collect::<Vec<_>>();
    let chamber_route = HexCoord::ORIGIN
        .within_radius(CHAMBER_RADIUS)
        .into_iter()
        .map(|coord| TilePos::new(coord, base))
        .collect::<BTreeSet<_>>();
    for surface in stair_surfaces.iter().chain(&chamber_route) {
        if !dim_sources.iter().any(|source| {
            upper_dome_contains(
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
    let expected_heart_blockers = HexCoord::ORIGIN
        .within_radius(HEART_RADIUS)
        .into_iter()
        .map(|coord| TilePos::new(coord, base))
        .collect::<BTreeSet<_>>();
    if !expected_heart_blockers.is_subset(&plan.blockers) {
        issues.push(recipe_issue(
            "cathedral heart does not publish its exact radius-four movement footprint",
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
            && feature.root.coord.distance(HexCoord::ORIGIN) <= CLEARING_RADIUS
    }) {
        issues.push(recipe_issue("summit trees enter the radius-18 clearing"));
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
                surface.level == base && surface.coord.distance(HexCoord::ORIGIN) <= CHAMBER_RADIUS
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
        circuits: CIRCUIT_COUNT as u32,
        flights: FLIGHT_COUNT as u32,
        landings: LANDING_COUNT as u32,
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
    push_mass(
        column,
        position.level,
        position.level.saturating_add(1),
        material,
    );
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
    let raw = (start_index(radius)
        + side.saturating_mul(usize::try_from(radius).unwrap_or(usize::MAX)))
        % ring.len();
    ring[raw]
}

fn radial_pad(radius: u32, side: usize, width: usize, level: Level) -> BTreeSet<TilePos> {
    let ring = ring_coordinates(radius);
    let center = (start_index(radius)
        + side.saturating_mul(usize::try_from(radius).unwrap_or(usize::MAX)))
        % ring.len();
    let before = width / 2;
    (0..width)
        .map(|offset| {
            let raw = (center + ring.len() + offset).saturating_sub(before) % ring.len();
            TilePos::new(ring[raw], level)
        })
        .collect()
}

fn shift(coord: HexCoord, delta: HexCoord) -> HexCoord {
    let [x, y, z] = coord.to_cubic_array();
    let [dx, dy, dz] = delta.to_cubic_array();
    HexCoord::new_cubic(x + dx, y + dy, z + dz)
}

fn is_lower_aperture(coord: HexCoord) -> bool {
    let [x, y, z] = coord.to_cubic_array();
    z > 0 && x.abs_diff(y) <= 11
}

fn is_upper_trail(coord: HexCoord) -> bool {
    let [x, y, z] = coord.to_cubic_array();
    z < 0 && x.abs_diff(y) <= 3
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
