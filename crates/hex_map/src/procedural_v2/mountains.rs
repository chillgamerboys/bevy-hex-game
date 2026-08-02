//! Native V2 mountain geometry.
//!
//! Mountains use a signed-axis ridge as a structural separator. Two explicit
//! two-wide portal ribbons are the only ordinary surfaces in that separator; sharp
//! ridge and summit components are classified from the final walker graph.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_core::{
    Headroom, HexCoord, Level, MapViewHint, SpecialMovementRegion, SubstanceId, TilePos,
    TraversalEndpoint, TraversalProfile, MAX_HEADROOM,
};

use super::recipe::{
    materialize_selection, run_recipe, CandidateAttemptError, CandidateContext, FallbackContext,
    MaterializedSelection, RecipePlan, RecipeValidation, RepairOutcome, ReportMetrics, V2Recipe,
    ValidationContext,
};
use super::volume::{
    LevelInterval, SolidMass, SolidMaterialRole, SurfaceAccess, SurfaceMetadata, TerrainVolumePlan,
    VolumeColumn, VolumeElement,
};
use super::V2GenerationError;
use crate::procedural::TacticalMetrics;
use crate::settings::{
    MountainsSettings, ProceduralV2Settings, V2EnvironmentSettings, V2RecipeSettings,
};
use crate::terrain::TerrainPalette;

const RIDGE_CREST_RISE: Level = 5;
const RIDGE_SHOULDER_RISE: Level = 2;
const MOUNTAIN_EDGE_RISE: Level = 4;
const ROUTE_HALF_LENGTH: i32 = 4;
const MIN_PEAK_SEPARATION: u32 = 2;
const EXPANDED_ROUTE_HALF_LENGTH: i32 = 5;
const EXPANDED_RIDGE_RISE: Level = 7;
const EXPANDED_BRANCH_RISE: Level = 5;
const EXPANDED_MIN_PEAK_SEPARATION: u32 = 3;
const FOOTHILL_TERRACE_DEPTH: u32 = 3;
const MIN_ACCESSIBLE_FOOTHILL_PERCENT: u32 = 18;
const MIN_ACCESSIBLE_FOOTHILL_PATCH: usize = 24;
const MIN_FOOTHILL_PLAIN_LANDINGS: usize = 2;
const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";
const CONFLICT_CENTER: &str = "conflict_center";
const HIGH_PASS: &str = "high_pass";
const LOW_BYPASS: &str = "low_bypass";

#[derive(Debug, Clone)]
struct MountainRoute {
    rows: Vec<[TilePos; 2]>,
}

impl MountainRoute {
    fn positions(&self) -> BTreeSet<TilePos> {
        self.rows
            .iter()
            .flat_map(|row| row.iter().copied())
            .collect()
    }

    fn coords(&self) -> BTreeSet<HexCoord> {
        self.rows
            .iter()
            .flat_map(|row| row.iter().map(|position| position.coord))
            .collect()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MountainsMetadata {
    orientation: u8,
    heights: BTreeMap<HexCoord, Level>,
    materials: BTreeMap<HexCoord, SolidMaterialRole>,
    peak_centres: Vec<HexCoord>,
    peak_targets: BTreeMap<HexCoord, Level>,
    spur_cells: BTreeSet<HexCoord>,
    ridge_cells: BTreeSet<HexCoord>,
    high_pass: MountainRoute,
    low_bypass: MountainRoute,
    ordinary: BTreeSet<HexCoord>,
    mountain_cells: BTreeSet<HexCoord>,
    main_spine: Vec<HexCoord>,
    branch_spines: Vec<Vec<HexCoord>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MountainsMetrics {
    tactical: TacticalMetrics,
    inaccessible_peaks: u8,
    cliff_edges: u32,
    high_pass_steps: u32,
    low_bypass_steps: u32,
    exposed_stone_percent: u32,
    mountain_coverage_percent: u32,
    peak_height_spread: Level,
    spine_turns: u32,
    branch_count: u32,
}

impl ReportMetrics for MountainsMetrics {
    fn tactical(&self) -> TacticalMetrics {
        self.tactical
    }
}

struct MountainsRecipe {
    level_height: f32,
}

pub(crate) fn build(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV2Settings,
    seed: u64,
    palette: &TerrainPalette,
    is_solid: &dyn Fn(SubstanceId) -> bool,
) -> Result<MaterializedSelection<MountainsMetadata, MountainsMetrics>, V2GenerationError> {
    let V2RecipeSettings::Mountains(mountain_settings) = &settings.recipe else {
        return Err(V2GenerationError::RecipeUnavailable("Mountains"));
    };
    if settings.environment != V2EnvironmentSettings::Frozen {
        return Err(V2GenerationError::RecipeUnavailable(
            "Mountains with non-Frozen environment",
        ));
    }
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V2GenerationError::RecipeContract(
            "Mountains level height must be positive and finite".to_owned(),
        ));
    }

    let recipe = MountainsRecipe { level_height };
    let selection = run_recipe(&recipe, mountain_settings, grid_radius, seed)?;
    materialize_selection(selection, palette, is_solid)
}

impl V2Recipe for MountainsRecipe {
    type Settings = MountainsSettings;
    type Metadata = MountainsMetadata;
    type Metrics = MountainsMetrics;
    type Score = (u32, u32, u32, u32, u32, u32, u8);

    fn construct(
        &self,
        context: CandidateContext,
        settings: &Self::Settings,
    ) -> Result<RecipePlan<Self::Metadata>, CandidateAttemptError> {
        construct_plan(self, context, settings, false)
    }

    fn validate(
        &self,
        _context: ValidationContext,
        settings: &Self::Settings,
        plan: &RecipePlan<Self::Metadata>,
    ) -> RecipeValidation<Self::Metrics> {
        validate_plan(settings, plan)
    }

    fn repair(
        &self,
        _context: CandidateContext,
        _settings: &Self::Settings,
        _plan: &mut RecipePlan<Self::Metadata>,
        _round: u8,
        _issues: &[String],
    ) -> Result<RepairOutcome, CandidateAttemptError> {
        Ok(RepairOutcome::NoChange)
    }

    fn score(
        &self,
        settings: &Self::Settings,
        metrics: &Self::Metrics,
        candidate: u8,
    ) -> Self::Score {
        if uses_expanded_geometry(settings) {
            (
                u32::from(
                    settings
                        .peak_count
                        .saturating_sub(metrics.inaccessible_peaks),
                ),
                metrics
                    .mountain_coverage_percent
                    .abs_diff(expanded_coverage_target(settings)),
                metrics
                    .peak_height_spread
                    .abs_diff(expanded_peak_spread(settings)),
                metrics
                    .branch_count
                    .abs_diff(expanded_branch_count(settings))
                    .saturating_add(
                        metrics
                            .spine_turns
                            .abs_diff(expanded_spine_turn_target(settings)),
                    ),
                metrics
                    .cliff_edges
                    .abs_diff(expanded_cliff_target(settings)),
                metrics.tactical.alternate_detour_percent.abs_diff(40),
                candidate,
            )
        } else {
            (
                u32::from(
                    settings
                        .peak_count
                        .saturating_sub(metrics.inaccessible_peaks),
                ),
                metrics.cliff_edges.abs_diff(36),
                metrics.tactical.alternate_detour_percent.abs_diff(40),
                0,
                0,
                0,
                candidate,
            )
        }
    }

    fn canonical_fallback(
        &self,
        context: FallbackContext,
        settings: &Self::Settings,
    ) -> Result<RecipePlan<Self::Metadata>, V2GenerationError> {
        construct_plan(
            self,
            CandidateContext {
                grid_radius: context.grid_radius,
                candidate: 0,
                streams: super::seed::SeedStreams::new(0, 0),
            },
            settings,
            true,
        )
        .map_err(|error| match error {
            CandidateAttemptError::Rejected(issues) => V2GenerationError::InvalidFallback(issues),
            CandidateAttemptError::Fatal(error) => error,
        })
    }
}

fn construct_plan(
    recipe: &MountainsRecipe,
    context: CandidateContext,
    settings: &MountainsSettings,
    fallback: bool,
) -> Result<RecipePlan<MountainsMetadata>, CandidateAttemptError> {
    if uses_expanded_geometry(settings) {
        return construct_expanded_plan(recipe, context, settings, fallback);
    }
    construct_legacy_plan(recipe, context, settings, fallback)
}

fn construct_legacy_plan(
    recipe: &MountainsRecipe,
    context: CandidateContext,
    settings: &MountainsSettings,
    fallback: bool,
) -> Result<RecipePlan<MountainsMetadata>, CandidateAttemptError> {
    let radius = i32::try_from(context.grid_radius)
        .map_err(|_out_of_range| CandidateAttemptError::rejected("mountain radius is too large"))?;
    let orientation = if fallback {
        0
    } else {
        u8::try_from(context.streams.stage("mountains.orientation").sample(0) % 3)
            .unwrap_or_default()
    };
    let high_u = -radius / 3;
    let low_u = radius / 3;
    let spawn_v = (radius / 2).max(5);
    let party_coord = from_local(high_u, -spawn_v, orientation);
    let hostile_coord = from_local(high_u, spawn_v, orientation);
    let peak_centres = peak_centres(radius, settings.peak_count, high_u, low_u, orientation)?;
    let peak_targets: BTreeMap<_, _> = peak_centres
        .iter()
        .copied()
        .enumerate()
        .map(|(index, centre)| {
            let variation = if fallback || index == 0 {
                0
            } else {
                u32::try_from(
                    context
                        .streams
                        .stage("mountains.peaks")
                        .sample(u64::try_from(index).unwrap_or(u64::MAX))
                        % 3,
                )
                .unwrap_or_default()
            };
            (
                centre,
                settings
                    .base_level
                    .saturating_add(settings.relief.saturating_sub_unsigned(variation)),
            )
        })
        .collect();

    let footprint = HexCoord::ORIGIN.within_radius(context.grid_radius);
    let mut heights = BTreeMap::new();
    for coord in footprint {
        let local = to_local(coord, orientation);
        let mut height = settings.base_level;
        if local.y().unsigned_abs() <= 2 {
            height = height.max(settings.base_level.saturating_add(RIDGE_SHOULDER_RISE));
        }
        if local.y().unsigned_abs() <= 1 {
            height = height.max(settings.base_level.saturating_add(RIDGE_CREST_RISE));
        }
        for (centre, target) in &peak_targets {
            let target_rise = target.saturating_sub(settings.base_level);
            let distance = centre.distance(coord);
            let falloff = distance.saturating_mul(3);
            let rise = target_rise.saturating_sub_unsigned(falloff);
            if rise >= MOUNTAIN_EDGE_RISE {
                height = height.max(settings.base_level.saturating_add(rise));
            }
        }
        heights.insert(coord, height);
    }
    let spur_cells = apply_spurs(
        &mut heights,
        &peak_targets,
        orientation,
        context,
        settings.base_level,
    );

    flatten_zone(&mut heights, party_coord, settings.base_level, 2);
    flatten_zone(&mut heights, hostile_coord, settings.base_level, 2);
    let high_pass = build_route(high_u, orientation, settings.base_level, |local_v| {
        (ROUTE_HALF_LENGTH - local_v.abs()).clamp(0, 3)
    });
    let low_bypass = build_route(low_u, orientation, settings.base_level, |local_v| {
        i32::from(local_v.abs() <= 1)
    });
    apply_route(&mut heights, &high_pass);
    apply_route(&mut heights, &low_bypass);

    let party_position = surface_position(&heights, party_coord)?;
    let hostile_position = surface_position(&heights, hostile_coord)?;
    let components = walker_components(&heights);
    let Some(ordinary) = components
        .iter()
        .find(|component| component.contains(&party_coord))
        .cloned()
    else {
        return Err(CandidateAttemptError::rejected(
            "party start is absent from the mountain surface graph",
        ));
    };
    if !ordinary.contains(&hostile_coord) {
        return Err(CandidateAttemptError::rejected(
            "the two mountain routes do not connect the required actors",
        ));
    }

    let mut access = BTreeMap::new();
    for coord in &ordinary {
        access.insert(*coord, SurfaceAccess::Ordinary);
    }
    let mut special_components: Vec<_> = components
        .into_iter()
        .filter(|component| !component.contains(&party_coord))
        .collect();
    special_components.sort_by_key(|component| component.first().copied());
    for (index, component) in special_components.into_iter().enumerate() {
        let region =
            SpecialMovementRegion(u32::try_from(index.saturating_add(1)).unwrap_or(u32::MAX));
        for coord in component {
            access.insert(coord, SurfaceAccess::SpecialMovement(region));
        }
    }

    let mut materials = BTreeMap::new();
    let mut columns = BTreeMap::new();
    let mut surfaces = BTreeMap::new();
    for (coord, surface) in &heights {
        let material = classify_material(*coord, *surface, settings, &heights, context);
        materials.insert(*coord, material);
        columns.insert(*coord, mountain_column(*surface, material));
        surfaces.insert(
            TilePos::new(*coord, *surface),
            SurfaceMetadata {
                access: access
                    .get(coord)
                    .copied()
                    .unwrap_or(SurfaceAccess::Ordinary),
                interior: None,
            },
        );
    }

    let high_position = high_pass
        .rows
        .get(high_pass.rows.len() / 2)
        .and_then(|row| row.first().copied())
        .ok_or_else(|| CandidateAttemptError::rejected("high pass has no centre"))?;
    let low_position = low_bypass
        .rows
        .get(low_bypass.rows.len() / 2)
        .and_then(|row| row.first().copied())
        .ok_or_else(|| CandidateAttemptError::rejected("low bypass has no centre"))?;
    let anchors = BTreeMap::from([
        (PARTY_START.to_owned(), party_position),
        (HOSTILE_START.to_owned(), hostile_position),
        (CONFLICT_CENTER.to_owned(), high_position),
        (HIGH_PASS.to_owned(), high_position),
        (LOW_BYPASS.to_owned(), low_position),
    ]);
    let ridge_cells = heights
        .keys()
        .copied()
        .filter(|coord| to_local(*coord, orientation).y().unsigned_abs() <= 1)
        .collect::<BTreeSet<_>>();
    let mountain_cells = heights
        .iter()
        .filter_map(|(coord, height)| (*height > settings.base_level).then_some(*coord))
        .collect();
    let main_spine = (-radius..=radius)
        .map(|local_u| from_local(local_u, 0, orientation))
        .collect();
    let view_hint = mountain_view_hint(
        context.grid_radius,
        recipe.level_height,
        settings.base_level,
        settings.relief,
        orientation,
    )?;

    Ok(RecipePlan {
        volume: TerrainVolumePlan {
            grid_radius: context.grid_radius,
            columns,
            surfaces,
            anchors,
            interiors: BTreeMap::new(),
            view_hint,
        },
        metadata: MountainsMetadata {
            orientation,
            heights,
            materials,
            peak_centres,
            peak_targets,
            spur_cells,
            ridge_cells,
            high_pass,
            low_bypass,
            ordinary,
            mountain_cells,
            main_spine,
            branch_spines: Vec::new(),
        },
    })
}

const fn uses_expanded_geometry(settings: &MountainsSettings) -> bool {
    settings.relief > 16 || settings.peak_count > 5
}

fn expanded_coverage_target(settings: &MountainsSettings) -> u32 {
    let relief_bonus =
        u32::try_from(settings.relief.saturating_sub(18).max(0) / 2).unwrap_or_default();
    let peak_bonus = u32::from(settings.peak_count.saturating_sub(5)).saturating_mul(2);
    53_u32
        .saturating_add(relief_bonus)
        .saturating_add(peak_bonus)
        .clamp(52, 65)
}

fn expanded_peak_spread(settings: &MountainsSettings) -> Level {
    (settings.relief / 3).clamp(5, 8)
}

fn expanded_branch_count(settings: &MountainsSettings) -> u32 {
    u32::from(settings.peak_count.saturating_sub(3)).clamp(2, 5)
}

fn expanded_cliff_target(settings: &MountainsSettings) -> u32 {
    60_u32.saturating_add(expanded_branch_count(settings).saturating_mul(12))
}

fn expanded_spine_turn_target(settings: &MountainsSettings) -> u32 {
    expanded_branch_count(settings).saturating_mul(2)
}

fn expanded_spine_amplitude(radius: i32, settings: &MountainsSettings) -> i32 {
    let radius_amplitude = radius.saturating_sub(12).saturating_add(11).div_euclid(12);
    1_i32
        .saturating_add(settings.relief.saturating_sub(18).max(0) / 3)
        .saturating_add(radius_amplitude)
        .clamp(1, 6)
}

const fn expanded_spine_segment_count(radius: i32) -> usize {
    match radius {
        ..=12 => 6,
        13..=20 => 8,
        _ => 12,
    }
}

fn expanded_branch_base_length(radius: i32, settings: &MountainsSettings) -> i32 {
    5_i32
        .saturating_add(settings.relief.saturating_sub(18).max(0) / 3)
        .saturating_add(radius.saturating_sub(12).div_euclid(4))
        .clamp(5, radius.saturating_sub(2))
}

fn construct_expanded_plan(
    recipe: &MountainsRecipe,
    context: CandidateContext,
    settings: &MountainsSettings,
    fallback: bool,
) -> Result<RecipePlan<MountainsMetadata>, CandidateAttemptError> {
    let radius = i32::try_from(context.grid_radius)
        .map_err(|_out_of_range| CandidateAttemptError::rejected("mountain radius is too large"))?;
    let orientation = if fallback {
        0
    } else {
        u8::try_from(context.streams.stage("mountains.orientation").sample(0) % 3)
            .unwrap_or_default()
    };
    let high_u = -radius / 3;
    let low_u = radius / 3;
    let spawn_v = (radius / 2).max(5);
    let party_coord = from_local(high_u, -spawn_v, orientation);
    let hostile_coord = from_local(high_u, spawn_v, orientation);
    let footprint: BTreeSet<_> = HexCoord::ORIGIN
        .within_radius(context.grid_radius)
        .into_iter()
        .collect();

    let main_spine = expanded_main_spine(radius, orientation, context, settings, fallback)?;
    let branch_spines = expanded_branch_spines(
        radius,
        orientation,
        context,
        settings,
        fallback,
        &main_spine,
        &footprint,
    )?;
    let ridge_cells: BTreeSet<_> = main_spine.iter().copied().collect();
    let spur_cells: BTreeSet<_> = branch_spines
        .iter()
        .flat_map(|branch| branch.iter().copied().skip(1))
        .collect();

    let high_pass = build_route_with_half_length(
        high_u,
        orientation,
        settings.base_level,
        EXPANDED_ROUTE_HALF_LENGTH,
        |local_v| (EXPANDED_ROUTE_HALF_LENGTH - local_v.abs()).clamp(0, 4),
    );
    let low_bypass = build_route_with_half_length(
        low_u,
        orientation,
        settings.base_level,
        EXPANDED_ROUTE_HALF_LENGTH,
        |local_v| i32::from(local_v.abs() <= 1),
    );
    let route_coords: BTreeSet<_> = high_pass
        .coords()
        .union(&low_bypass.coords())
        .copied()
        .collect();

    let mut protected = BTreeSet::new();
    protected.extend(party_coord.within_radius(2));
    protected.extend(hostile_coord.within_radius(2));
    let party_approach = [
        party_coord,
        from_local(high_u.saturating_add(1), -spawn_v, orientation),
    ];
    let hostile_approach = [
        hostile_coord,
        from_local(high_u.saturating_add(1), spawn_v, orientation),
    ];
    for route in [&high_pass, &low_bypass] {
        let route_start =
            route.rows.first().copied().ok_or_else(|| {
                CandidateAttemptError::rejected("mountain route has no first row")
            })?;
        let route_end = route
            .rows
            .last()
            .copied()
            .ok_or_else(|| CandidateAttemptError::rejected("mountain route has no last row"))?;
        for (start, end) in party_approach.into_iter().zip(route_start) {
            protected.extend(start.line_between(end.coord));
        }
        for (start, end) in hostile_approach.into_iter().zip(route_end) {
            protected.extend(start.line_between(end.coord));
        }
    }

    let skeleton: BTreeSet<_> = ridge_cells
        .union(&spur_cells)
        .copied()
        .filter(|coord| footprint.contains(coord))
        .collect();
    let target_count = coverage_cell_target(footprint.len(), expanded_coverage_target(settings));
    let mut mountain_cells =
        grow_mountain_footprint(skeleton, target_count, &footprint, &protected, context)?;

    let peak_centres = expanded_peak_centres(
        settings.peak_count,
        &main_spine,
        &branch_spines,
        &route_coords,
        &protected,
        &footprint,
        context,
    )?;
    let peak_targets = expanded_peak_targets(&peak_centres, settings, context, fallback);

    let mut heights: BTreeMap<_, _> = footprint
        .iter()
        .copied()
        .map(|coord| (coord, settings.base_level))
        .collect();
    for coord in &mountain_cells {
        let main_distance = distance_to_set(*coord, &ridge_cells);
        let branch_distance = distance_to_set(*coord, &spur_cells);
        let rise = if main_distance == 0 {
            EXPANDED_RIDGE_RISE
        } else if branch_distance == 0 {
            EXPANDED_BRANCH_RISE
        } else {
            match main_distance.min(branch_distance) {
                1 => 3,
                2 => 2,
                _ => 1,
            }
        };
        if let Some(height) = heights.get_mut(coord) {
            *height = settings.base_level.saturating_add(rise);
        }
    }
    for (centre, target) in &peak_targets {
        let target_rise = target.saturating_sub(settings.base_level);
        for coord in &mountain_cells {
            let falloff = Level::try_from(centre.distance(*coord))
                .unwrap_or(Level::MAX)
                .saturating_mul(3);
            let rise = target_rise.saturating_sub(falloff);
            if rise <= 0 {
                continue;
            }
            if let Some(height) = heights.get_mut(coord) {
                *height = (*height).max(settings.base_level.saturating_add(rise));
            }
        }
    }

    flatten_zone(&mut heights, party_coord, settings.base_level, 2);
    flatten_zone(&mut heights, hostile_coord, settings.base_level, 2);
    for coord in &protected {
        if let Some(height) = heights.get_mut(coord) {
            *height = settings.base_level;
        }
    }
    apply_route(&mut heights, &high_pass);
    apply_route(&mut heights, &low_bypass);
    mountain_cells = heights
        .iter()
        .filter_map(|(coord, height)| (*height > settings.base_level).then_some(*coord))
        .collect();
    retain_elevated_core(
        &mut heights,
        &mut mountain_cells,
        settings.base_level,
        &main_spine,
        &peak_centres,
    )?;
    if mountain_cells.len() > target_count {
        return Err(CandidateAttemptError::rejected(
            "expanded mountain core exceeds its coverage target after route carving",
        ));
    }
    top_up_mountain_coverage(
        &mut heights,
        &mut mountain_cells,
        target_count,
        settings.base_level,
        &footprint,
        &protected,
        &route_coords,
        context,
    )?;
    terrace_mountain_foothills(
        &mut heights,
        &mountain_cells,
        settings.base_level,
        orientation,
        &ridge_cells,
        &peak_centres,
        &route_coords,
    );
    if coordinate_components(&mountain_cells).len() != 1 {
        return Err(CandidateAttemptError::rejected(
            "expanded mountain top-up did not preserve one connected massif",
        ));
    }

    let party_position = surface_position(&heights, party_coord)?;
    let hostile_position = surface_position(&heights, hostile_coord)?;
    let components = walker_components(&heights);
    let Some(ordinary) = components
        .iter()
        .find(|component| component.contains(&party_coord))
        .cloned()
    else {
        return Err(CandidateAttemptError::rejected(
            "party start is absent from the mountain surface graph",
        ));
    };
    if !ordinary.contains(&hostile_coord) {
        return Err(CandidateAttemptError::rejected(
            "the two mountain routes do not connect the required actors",
        ));
    }

    let mut access = BTreeMap::new();
    for coord in &ordinary {
        access.insert(*coord, SurfaceAccess::Ordinary);
    }
    let mut special_components: Vec<_> = components
        .into_iter()
        .filter(|component| !component.contains(&party_coord))
        .collect();
    special_components.sort_by_key(|component| component.first().copied());
    for (index, component) in special_components.into_iter().enumerate() {
        let region =
            SpecialMovementRegion(u32::try_from(index.saturating_add(1)).unwrap_or(u32::MAX));
        for coord in component {
            access.insert(coord, SurfaceAccess::SpecialMovement(region));
        }
    }

    let mut materials = BTreeMap::new();
    let mut columns = BTreeMap::new();
    let mut surfaces = BTreeMap::new();
    for (coord, surface) in &heights {
        let material = classify_material(*coord, *surface, settings, &heights, context);
        materials.insert(*coord, material);
        columns.insert(*coord, mountain_column(*surface, material));
        surfaces.insert(
            TilePos::new(*coord, *surface),
            SurfaceMetadata {
                access: access
                    .get(coord)
                    .copied()
                    .unwrap_or(SurfaceAccess::Ordinary),
                interior: None,
            },
        );
    }

    let high_position = high_pass
        .rows
        .get(high_pass.rows.len() / 2)
        .and_then(|row| row.first().copied())
        .ok_or_else(|| CandidateAttemptError::rejected("high pass has no centre"))?;
    let low_position = low_bypass
        .rows
        .get(low_bypass.rows.len() / 2)
        .and_then(|row| row.first().copied())
        .ok_or_else(|| CandidateAttemptError::rejected("low bypass has no centre"))?;
    let anchors = BTreeMap::from([
        (PARTY_START.to_owned(), party_position),
        (HOSTILE_START.to_owned(), hostile_position),
        (CONFLICT_CENTER.to_owned(), high_position),
        (HIGH_PASS.to_owned(), high_position),
        (LOW_BYPASS.to_owned(), low_position),
    ]);
    let view_hint = mountain_view_hint(
        context.grid_radius,
        recipe.level_height,
        settings.base_level,
        settings.relief,
        orientation,
    )?;

    Ok(RecipePlan {
        volume: TerrainVolumePlan {
            grid_radius: context.grid_radius,
            columns,
            surfaces,
            anchors,
            interiors: BTreeMap::new(),
            view_hint,
        },
        metadata: MountainsMetadata {
            orientation,
            heights,
            materials,
            peak_centres,
            peak_targets,
            spur_cells,
            ridge_cells,
            high_pass,
            low_bypass,
            ordinary,
            mountain_cells,
            main_spine,
            branch_spines,
        },
    })
}

fn expanded_main_spine(
    radius: i32,
    orientation: u8,
    context: CandidateContext,
    settings: &MountainsSettings,
    fallback: bool,
) -> Result<Vec<HexCoord>, CandidateAttemptError> {
    let amplitude = expanded_spine_amplitude(radius, settings);
    let segment_count = expanded_spine_segment_count(radius);
    let local_xs: Vec<_> = (0..=segment_count)
        .map(|index| {
            let numerator = i32::try_from(index)
                .unwrap_or(i32::MAX)
                .saturating_mul(radius.saturating_mul(2));
            -radius + numerator.div_euclid(i32::try_from(segment_count).unwrap_or(i32::MAX).max(1))
        })
        .collect();
    let mut knots = Vec::with_capacity(local_xs.len());
    for (index, local_x) in local_xs.iter().copied().enumerate() {
        let local_y = if index == 0 || index == local_xs.len().saturating_sub(1) {
            0
        } else if fallback {
            match index % 4 {
                1 => amplitude,
                3 => -amplitude,
                _ => 0,
            }
        } else {
            context
                .streams
                .stage("mountains.massif.spine")
                .range_i32(
                    u64::try_from(index).unwrap_or(u64::MAX),
                    -amplitude,
                    amplitude,
                )
                .map_err(CandidateAttemptError::rejected)?
        };
        knots.push(from_local(local_x, local_y, orientation));
    }

    let mut spine = Vec::new();
    for pair in knots.windows(2) {
        let [start, end] = pair else {
            continue;
        };
        for coord in start.line_between(*end) {
            if spine.last().copied() != Some(coord) {
                spine.push(coord);
            }
        }
    }
    Ok(spine)
}

fn expanded_branch_spines(
    radius: i32,
    orientation: u8,
    context: CandidateContext,
    settings: &MountainsSettings,
    fallback: bool,
    main_spine: &[HexCoord],
    footprint: &BTreeSet<HexCoord>,
) -> Result<Vec<Vec<HexCoord>>, CandidateAttemptError> {
    let count = usize::try_from(expanded_branch_count(settings)).unwrap_or_default();
    let mut branches = Vec::with_capacity(count);
    let phase = if fallback {
        0
    } else {
        usize::try_from(
            context
                .streams
                .stage("mountains.massif.branch_phase")
                .sample(0)
                % 2,
        )
        .unwrap_or_default()
    };
    let main_cells: BTreeSet<_> = main_spine.iter().copied().collect();
    let mut occupied_interiors = BTreeSet::new();
    for index in 0..count {
        let slot =
            index.saturating_add(1).saturating_mul(main_spine.len()) / count.saturating_add(1);
        let Some(start) = main_spine
            .get(slot.min(main_spine.len().saturating_sub(1)))
            .copied()
        else {
            return Err(CandidateAttemptError::rejected(
                "expanded mountain spine has no branch attachment",
            ));
        };
        let local = to_local(start, orientation);
        let side: i32 = if index.saturating_add(phase).is_multiple_of(2) {
            1
        } else {
            -1
        };
        let base_length = expanded_branch_base_length(radius, settings);
        let variation = if fallback {
            0
        } else {
            i32::try_from(
                context
                    .streams
                    .stage("mountains.massif.branch_lengths")
                    .sample(u64::try_from(index).unwrap_or(u64::MAX))
                    % 3,
            )
            .unwrap_or_default()
        };
        let length = base_length.saturating_add(variation).min(radius - 2);
        let bend_magnitude = 2_i32
            .saturating_add(radius.saturating_sub(12).div_euclid(10))
            .clamp(2, 5);
        let preferred_bend = if fallback
            || context
                .streams
                .stage("mountains.massif.branch_bends")
                .sample(u64::try_from(index).unwrap_or(u64::MAX))
                .is_multiple_of(2)
        {
            bend_magnitude
        } else {
            -bend_magnitude
        };
        let outward = from_local(local.x(), local.y().saturating_add(side), orientation);
        let endpoints = [preferred_bend, -preferred_bend].map(|bend| {
            from_local(
                local.x().saturating_add(bend),
                local.y().saturating_add(side.saturating_mul(length)),
                orientation,
            )
        });
        let branch = endpoints
            .into_iter()
            .map(|end| {
                let mut path = vec![start];
                path.extend(
                    outward
                        .line_between(end)
                        .into_iter()
                        .take_while(|coord| footprint.contains(coord)),
                );
                path.dedup();
                path
            })
            .filter(|path| path.len() >= 4)
            .min_by_key(|path| {
                let interior_overlap = path
                    .iter()
                    .skip(1)
                    .filter(|coord| occupied_interiors.contains(*coord))
                    .count();
                let spine_overlap = path
                    .iter()
                    .skip(1)
                    .filter(|coord| main_cells.contains(coord))
                    .count();
                (
                    interior_overlap,
                    spine_overlap,
                    Reverse(path.len()),
                    path.last().copied(),
                )
            })
            .unwrap_or_default();
        let interior_overlap = branch
            .iter()
            .skip(1)
            .any(|coord| occupied_interiors.contains(coord));
        let leaves_spine = branch
            .iter()
            .skip(1)
            .filter(|coord| !main_cells.contains(coord))
            .count()
            >= branch.len().saturating_sub(2);
        if branch.len() < 4 || interior_overlap || !leaves_spine {
            return Err(CandidateAttemptError::rejected(
                "expanded mountain cannot place a distinct branch on both ridge sides",
            ));
        }
        occupied_interiors.extend(branch.iter().copied().skip(1));
        branches.push(branch);
    }
    Ok(branches)
}

fn expanded_peak_centres(
    peak_count: u8,
    main_spine: &[HexCoord],
    branch_spines: &[Vec<HexCoord>],
    route_coords: &BTreeSet<HexCoord>,
    protected: &BTreeSet<HexCoord>,
    footprint: &BTreeSet<HexCoord>,
    context: CandidateContext,
) -> Result<Vec<HexCoord>, CandidateAttemptError> {
    let desired = usize::from(peak_count);
    let mut centres = Vec::with_capacity(desired);
    for branch in branch_spines {
        let centre_index = branch.len().saturating_mul(2) / 3;
        let Some(centre) = branch.get(centre_index).copied() else {
            continue;
        };
        if !route_coords.contains(&centre)
            && !protected.contains(&centre)
            && branch
                .iter()
                .take(centre_index.saturating_add(1))
                .all(|coord| !protected.contains(coord) && !route_coords.contains(coord))
            && centres
                .iter()
                .all(|other| centre.distance(*other) >= EXPANDED_MIN_PEAK_SEPARATION)
        {
            centres.push(centre);
        }
    }

    let candidates: Vec<_> = main_spine
        .iter()
        .copied()
        .filter(|coord| {
            footprint.contains(coord)
                && coord.distance(HexCoord::ORIGIN)
                    < footprint
                        .iter()
                        .map(|candidate| candidate.distance(HexCoord::ORIGIN))
                        .max()
                        .unwrap_or_default()
                        .saturating_sub(1)
                && !route_coords.contains(coord)
                && !protected.contains(coord)
        })
        .collect();
    while centres.len() < desired {
        let centre = candidates
            .iter()
            .copied()
            .filter(|candidate| !centres.contains(candidate))
            .filter(|candidate| {
                centres
                    .iter()
                    .all(|other| candidate.distance(*other) >= EXPANDED_MIN_PEAK_SEPARATION)
            })
            .max_by_key(|candidate| {
                let separation = centres
                    .iter()
                    .map(|other| candidate.distance(*other))
                    .min()
                    .unwrap_or(u32::MAX);
                let tie_break = context
                    .streams
                    .stage("mountains.massif.peaks")
                    .sample_coord(*candidate, 0);
                (separation, tie_break)
            })
            .ok_or_else(|| {
                CandidateAttemptError::rejected(
                    "expanded mountain cannot place enough separated peaks",
                )
            })?;
        centres.push(centre);
    }
    Ok(centres)
}

fn expanded_peak_targets(
    centres: &[HexCoord],
    settings: &MountainsSettings,
    context: CandidateContext,
    fallback: bool,
) -> BTreeMap<HexCoord, Level> {
    let spread = expanded_peak_spread(settings);
    let denominator = Level::try_from(centres.len().saturating_sub(1))
        .unwrap_or(Level::MAX)
        .max(1);
    let rotation = if fallback || centres.len() <= 2 {
        0
    } else {
        usize::try_from(
            context
                .streams
                .stage("mountains.massif.peak_heights")
                .sample(0)
                % u64::try_from(centres.len().saturating_sub(1)).unwrap_or(1),
        )
        .unwrap_or_default()
    };
    centres
        .iter()
        .copied()
        .enumerate()
        .map(|(index, centre)| {
            let spread_index = if index == 0 {
                0
            } else {
                1 + (index.saturating_sub(1).saturating_add(rotation)
                    % centres.len().saturating_sub(1))
            };
            let variation = Level::try_from(spread_index)
                .unwrap_or(Level::MAX)
                .saturating_mul(spread)
                / denominator;
            (
                centre,
                settings
                    .base_level
                    .saturating_add(settings.relief.saturating_sub(variation)),
            )
        })
        .collect()
}

fn coverage_cell_target(cell_count: usize, percent: u32) -> usize {
    cell_count
        .saturating_mul(usize::try_from(percent).unwrap_or(usize::MAX))
        .saturating_add(99)
        / 100
}

fn grow_mountain_footprint(
    mut mountain_cells: BTreeSet<HexCoord>,
    target_count: usize,
    footprint: &BTreeSet<HexCoord>,
    protected: &BTreeSet<HexCoord>,
    context: CandidateContext,
) -> Result<BTreeSet<HexCoord>, CandidateAttemptError> {
    let skeleton = mountain_cells.clone();
    let mut frontier = BTreeSet::new();
    for coord in &mountain_cells {
        for neighbor in coord.neighbors() {
            if footprint.contains(&neighbor)
                && !protected.contains(&neighbor)
                && !mountain_cells.contains(&neighbor)
            {
                frontier.insert((
                    mountain_footprint_priority(neighbor, &skeleton, context),
                    neighbor,
                ));
            }
        }
    }
    while mountain_cells.len() < target_count {
        let Some((_priority, next)) = frontier.pop_first() else {
            return Err(CandidateAttemptError::rejected(
                "expanded mountain footprint cannot reach its coverage target",
            ));
        };
        mountain_cells.insert(next);
        for neighbor in next.neighbors() {
            if footprint.contains(&neighbor)
                && !protected.contains(&neighbor)
                && !mountain_cells.contains(&neighbor)
            {
                frontier.insert((
                    mountain_footprint_priority(neighbor, &skeleton, context),
                    neighbor,
                ));
            }
        }
    }
    Ok(mountain_cells)
}

fn mountain_footprint_priority(
    coord: HexCoord,
    skeleton: &BTreeSet<HexCoord>,
    context: CandidateContext,
) -> u64 {
    let distance = distance_to_set(coord, skeleton);
    let roughness = context
        .streams
        .stage("mountains.massif.footprint")
        .sample_coord(
            HexCoord::from_axial(coord.x().div_euclid(2), coord.y().div_euclid(2)),
            0,
        )
        % 11;
    u64::from(distance).saturating_mul(5) + roughness
}

#[expect(
    clippy::too_many_arguments,
    reason = "coverage top-up needs the exact immutable exclusion sets that define its contract"
)]
fn top_up_mountain_coverage(
    heights: &mut BTreeMap<HexCoord, Level>,
    mountain_cells: &mut BTreeSet<HexCoord>,
    target_count: usize,
    base_level: Level,
    footprint: &BTreeSet<HexCoord>,
    protected: &BTreeSet<HexCoord>,
    route_coords: &BTreeSet<HexCoord>,
    context: CandidateContext,
) -> Result<(), CandidateAttemptError> {
    let mut frontier = BTreeSet::new();
    for coord in mountain_cells.iter() {
        for neighbor in coord.neighbors() {
            if footprint.contains(&neighbor)
                && !protected.contains(&neighbor)
                && !route_coords.contains(&neighbor)
                && !mountain_cells.contains(&neighbor)
            {
                frontier.insert((mountain_coverage_priority(neighbor, context), neighbor));
            }
        }
    }
    while mountain_cells.len() < target_count {
        let Some((_priority, next)) = frontier.pop_first() else {
            return Err(CandidateAttemptError::rejected(
                "expanded mountain elevations cannot reach their coverage target",
            ));
        };
        heights.insert(next, base_level.saturating_add(1));
        mountain_cells.insert(next);
        for neighbor in next.neighbors() {
            if footprint.contains(&neighbor)
                && !protected.contains(&neighbor)
                && !route_coords.contains(&neighbor)
                && !mountain_cells.contains(&neighbor)
            {
                frontier.insert((mountain_coverage_priority(neighbor, context), neighbor));
            }
        }
    }
    Ok(())
}

fn mountain_coverage_priority(coord: HexCoord, context: CandidateContext) -> u64 {
    context
        .streams
        .stage("mountains.massif.coverage")
        .sample_coord(coord, 0)
}

fn terrace_mountain_foothills(
    heights: &mut BTreeMap<HexCoord, Level>,
    mountain_cells: &BTreeSet<HexCoord>,
    base_level: Level,
    orientation: u8,
    ridge_cells: &BTreeSet<HexCoord>,
    peak_centres: &[HexCoord],
    route_coords: &BTreeSet<HexCoord>,
) {
    let protected: BTreeSet<_> = ridge_cells
        .union(route_coords)
        .copied()
        .chain(peak_centres.iter().copied())
        .collect();
    let mut distances = BTreeMap::new();
    let mut frontier = VecDeque::new();
    for coord in mountain_cells {
        let borders_plain = coord
            .neighbors()
            .into_iter()
            .any(|neighbor| heights.contains_key(&neighbor) && !mountain_cells.contains(&neighbor));
        if borders_plain {
            distances.insert(*coord, 1_u32);
            frontier.push_back(*coord);
        }
    }

    while let Some(coord) = frontier.pop_front() {
        let distance = distances.get(&coord).copied().unwrap_or(u32::MAX);
        if distance >= FOOTHILL_TERRACE_DEPTH {
            continue;
        }
        for neighbor in coord.neighbors() {
            if mountain_cells.contains(&neighbor) && !distances.contains_key(&neighbor) {
                distances.insert(neighbor, distance.saturating_add(1));
                frontier.push_back(neighbor);
            }
        }
    }

    for (coord, distance) in distances {
        if protected.contains(&coord) || to_local(coord, orientation).y() > 0 {
            continue;
        }
        let terrace_rise = Level::try_from(distance).unwrap_or(Level::MAX);
        let terrace_height = base_level.saturating_add(terrace_rise);
        if let Some(height) = heights.get_mut(&coord) {
            *height = (*height).min(terrace_height);
        }
    }
}

fn retain_elevated_core(
    heights: &mut BTreeMap<HexCoord, Level>,
    mountain_cells: &mut BTreeSet<HexCoord>,
    base_level: Level,
    main_spine: &[HexCoord],
    peak_centres: &[HexCoord],
) -> Result<(), CandidateAttemptError> {
    let spine_cells: BTreeSet<_> = main_spine.iter().copied().collect();
    let Some(core) = coordinate_components(mountain_cells)
        .into_iter()
        .max_by_key(|component| {
            let peak_count = peak_centres
                .iter()
                .filter(|peak| component.contains(*peak))
                .count();
            let spine_count = component.intersection(&spine_cells).count();
            (
                peak_count,
                spine_count,
                component.len(),
                Reverse(component.first().copied()),
            )
        })
    else {
        return Err(CandidateAttemptError::rejected(
            "protected carving removed the expanded mountain spine",
        ));
    };
    if core.is_disjoint(&spine_cells) {
        return Err(CandidateAttemptError::rejected(
            "protected carving detached every authored peak from the mountain spine",
        ));
    }
    if let Some(pruned_peak) = peak_centres
        .iter()
        .find(|peak| !core.contains(peak))
        .copied()
    {
        return Err(CandidateAttemptError::rejected(format!(
            "protected carving detached authored mountain peak {pruned_peak:?}"
        )));
    }

    let pruned: Vec<_> = mountain_cells.difference(&core).copied().collect();
    for coord in pruned {
        if let Some(height) = heights.get_mut(&coord) {
            *height = base_level;
        }
    }
    *mountain_cells = core;
    Ok(())
}

fn coordinate_component(start: HexCoord, cells: &BTreeSet<HexCoord>) -> BTreeSet<HexCoord> {
    if !cells.contains(&start) {
        return BTreeSet::new();
    }
    let mut reached = BTreeSet::from([start]);
    let mut frontier = VecDeque::from([start]);
    while let Some(coord) = frontier.pop_front() {
        for neighbor in coord.neighbors() {
            if cells.contains(&neighbor) && reached.insert(neighbor) {
                frontier.push_back(neighbor);
            }
        }
    }
    reached
}

fn coordinate_components(cells: &BTreeSet<HexCoord>) -> Vec<BTreeSet<HexCoord>> {
    let mut remaining = cells.clone();
    let mut components = Vec::new();
    while let Some(start) = remaining.first().copied() {
        let component = coordinate_component(start, &remaining);
        for coord in &component {
            remaining.remove(coord);
        }
        components.push(component);
    }
    components
}

fn distance_to_set(coord: HexCoord, positions: &BTreeSet<HexCoord>) -> u32 {
    positions
        .iter()
        .map(|other| coord.distance(*other))
        .min()
        .unwrap_or(u32::MAX)
}

fn peak_centres(
    radius: i32,
    peak_count: u8,
    high_u: i32,
    low_u: i32,
    orientation: u8,
) -> Result<Vec<HexCoord>, CandidateAttemptError> {
    let available: Vec<_> = ((-radius + 2)..=(radius - 2))
        .filter(|u| {
            (*u - high_u).abs() > 2
                && (*u - high_u.saturating_add(1)).abs() > 2
                && (*u - low_u).abs() > 2
                && (*u - low_u.saturating_add(1)).abs() > 2
        })
        .collect();
    let count = usize::from(peak_count);
    if count == 0 || available.len() < count {
        return Err(CandidateAttemptError::rejected(
            "the ridge has insufficient room for separated peaks and two routes",
        ));
    }

    let mut centres = Vec::with_capacity(count);
    for index in 0..count {
        let slot = index
            .saturating_mul(2)
            .saturating_add(1)
            .saturating_mul(available.len())
            / count.saturating_mul(2);
        let Some(u) = available.get(slot).copied() else {
            return Err(CandidateAttemptError::rejected(
                "peak distribution escaped the available ridge",
            ));
        };
        centres.push(from_local(u, 0, orientation));
    }
    if centres.windows(2).any(
        |pair| matches!(pair, [first, second] if first.distance(*second) < MIN_PEAK_SEPARATION),
    ) {
        return Err(CandidateAttemptError::rejected(
            "the ridge cannot keep every requested peak distinct",
        ));
    }
    Ok(centres)
}

fn apply_spurs(
    heights: &mut BTreeMap<HexCoord, Level>,
    peak_targets: &BTreeMap<HexCoord, Level>,
    orientation: u8,
    context: CandidateContext,
    base_level: Level,
) -> BTreeSet<HexCoord> {
    let mut spur_cells = BTreeSet::new();
    for (index, (peak, _target)) in peak_targets.iter().enumerate() {
        let local = to_local(*peak, orientation);
        let bends_toward_centre = -local.x().signum();
        let preferred_side = if context
            .streams
            .stage("mountains.spurs")
            .sample(u64::try_from(index).unwrap_or(u64::MAX))
            .is_multiple_of(2)
        {
            1
        } else {
            -1
        };
        let candidates = [
            spur_path(local.x(), preferred_side, bends_toward_centre, orientation),
            spur_path(local.x(), -preferred_side, bends_toward_centre, orientation),
        ];
        let Some(path) = candidates.into_iter().max_by_key(|path| {
            path.iter()
                .filter(|(coord, _rise)| heights.contains_key(coord))
                .count()
        }) else {
            continue;
        };
        for (coord, rise) in path {
            let Some(height) = heights.get_mut(&coord) else {
                continue;
            };
            *height = (*height).max(base_level.saturating_add(rise));
            spur_cells.insert(coord);
        }
    }
    spur_cells
}

fn spur_path(local_x: i32, side: i32, bend: i32, orientation: u8) -> [(HexCoord, Level); 3] {
    [
        (from_local(local_x, side.saturating_mul(4), orientation), 5),
        (
            from_local(
                local_x.saturating_add(bend),
                side.saturating_mul(5),
                orientation,
            ),
            3,
        ),
        (
            from_local(
                local_x.saturating_add(bend),
                side.saturating_mul(6),
                orientation,
            ),
            2,
        ),
    ]
}

fn build_route(
    local_u: i32,
    orientation: u8,
    base_level: Level,
    rise_at: impl Fn(i32) -> i32,
) -> MountainRoute {
    build_route_with_half_length(local_u, orientation, base_level, ROUTE_HALF_LENGTH, rise_at)
}

fn build_route_with_half_length(
    local_u: i32,
    orientation: u8,
    base_level: Level,
    half_length: i32,
    rise_at: impl Fn(i32) -> i32,
) -> MountainRoute {
    let rows = (-half_length..=half_length)
        .map(|local_v| {
            let level = base_level.saturating_add(rise_at(local_v));
            [
                TilePos::new(from_local(local_u, local_v, orientation), level),
                TilePos::new(
                    from_local(local_u.saturating_add(1), local_v, orientation),
                    level,
                ),
            ]
        })
        .collect();
    MountainRoute { rows }
}

fn apply_route(heights: &mut BTreeMap<HexCoord, Level>, route: &MountainRoute) {
    for position in route.rows.iter().flatten() {
        heights.insert(position.coord, position.level);
    }
}

fn flatten_zone(
    heights: &mut BTreeMap<HexCoord, Level>,
    centre: HexCoord,
    level: Level,
    radius: u32,
) {
    for coord in centre.within_radius(radius) {
        if let Some(surface) = heights.get_mut(&coord) {
            *surface = level;
        }
    }
}

fn surface_position(
    heights: &BTreeMap<HexCoord, Level>,
    coord: HexCoord,
) -> Result<TilePos, CandidateAttemptError> {
    heights
        .get(&coord)
        .copied()
        .map(|level| TilePos::new(coord, level))
        .ok_or_else(|| CandidateAttemptError::rejected("anchor escaped the mountain footprint"))
}

fn walker_components(heights: &BTreeMap<HexCoord, Level>) -> Vec<BTreeSet<HexCoord>> {
    walker_components_within(&heights.keys().copied().collect(), heights)
}

fn walker_components_within(
    cells: &BTreeSet<HexCoord>,
    heights: &BTreeMap<HexCoord, Level>,
) -> Vec<BTreeSet<HexCoord>> {
    let mut remaining = cells.clone();
    let mut components = Vec::new();
    while let Some(start) = remaining.pop_first() {
        let mut component = BTreeSet::from([start]);
        let mut frontier = VecDeque::from([start]);
        while let Some(coord) = frontier.pop_front() {
            for neighbor in coord.neighbors() {
                if !remaining.contains(&neighbor)
                    || !walker_step(coord, neighbor, heights)
                    || !remaining.remove(&neighbor)
                {
                    continue;
                }
                component.insert(neighbor);
                frontier.push_back(neighbor);
            }
        }
        components.push(component);
    }
    components
}

fn walker_step(from: HexCoord, to: HexCoord, heights: &BTreeMap<HexCoord, Level>) -> bool {
    let (Some(from_level), Some(to_level)) = (heights.get(&from), heights.get(&to)) else {
        return false;
    };
    TraversalProfile::WALKER.admits_transition(
        TraversalEndpoint::new(
            TilePos::new(from, *from_level),
            true,
            Headroom(MAX_HEADROOM),
        ),
        TraversalEndpoint::new(TilePos::new(to, *to_level), true, Headroom(MAX_HEADROOM)),
    )
}

fn classify_material(
    coord: HexCoord,
    surface: Level,
    settings: &MountainsSettings,
    heights: &BTreeMap<HexCoord, Level>,
    context: CandidateContext,
) -> SolidMaterialRole {
    let steep = coord.neighbors().into_iter().any(|neighbor| {
        heights
            .get(&neighbor)
            .is_some_and(|other| surface.abs_diff(*other) >= 2)
    });
    if steep || surface.saturating_sub(settings.base_level) >= RIDGE_CREST_RISE {
        SolidMaterialRole::Stone
    } else if context
        .streams
        .stage("mountains.materials")
        .sample_coord(
            HexCoord::from_axial(coord.x().div_euclid(3), coord.y().div_euclid(3)),
            0,
        )
        .is_multiple_of(7)
    {
        SolidMaterialRole::Ice
    } else {
        SolidMaterialRole::Snow
    }
}

fn mountain_column(surface: Level, top_material: SolidMaterialRole) -> VolumeColumn {
    let mut elements = vec![VolumeElement::Solid(SolidMass {
        levels: LevelInterval::new(0, 1),
        material: SolidMaterialRole::Bedrock,
        cutaway_for: None,
    })];
    if top_material == SolidMaterialRole::Stone {
        elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(1, surface.saturating_add(1)),
            material: SolidMaterialRole::Stone,
            cutaway_for: None,
        }));
        return VolumeColumn { elements };
    }

    let dirt_bottom = surface.saturating_sub(3).max(1);
    if dirt_bottom > 1 {
        elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(1, dirt_bottom),
            material: SolidMaterialRole::Stone,
            cutaway_for: None,
        }));
    }
    if dirt_bottom < surface {
        elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(dirt_bottom, surface),
            material: SolidMaterialRole::Dirt,
            cutaway_for: None,
        }));
    }
    elements.push(VolumeElement::Solid(SolidMass {
        levels: LevelInterval::new(surface, surface.saturating_add(1)),
        material: top_material,
        cutaway_for: None,
    }));
    VolumeColumn { elements }
}

fn accessible_foothills(metadata: &MountainsMetadata) -> BTreeSet<HexCoord> {
    let route_coords: BTreeSet<_> = metadata
        .high_pass
        .coords()
        .union(&metadata.low_bypass.coords())
        .copied()
        .collect();
    metadata
        .mountain_cells
        .intersection(&metadata.ordinary)
        .copied()
        .filter(|coord| {
            !route_coords.contains(coord) && to_local(*coord, metadata.orientation).y() <= 0
        })
        .collect()
}

fn foothill_plain_landings(foothills: &BTreeSet<HexCoord>, metadata: &MountainsMetadata) -> usize {
    let route_coords: BTreeSet<_> = metadata
        .high_pass
        .coords()
        .union(&metadata.low_bypass.coords())
        .copied()
        .collect();
    foothills
        .iter()
        .filter(|coord| {
            coord.neighbors().into_iter().any(|neighbor| {
                metadata.ordinary.contains(&neighbor)
                    && !metadata.mountain_cells.contains(&neighbor)
                    && !route_coords.contains(&neighbor)
                    && to_local(neighbor, metadata.orientation).y() <= 0
                    && walker_step(**coord, neighbor, &metadata.heights)
            })
        })
        .count()
}

fn validate_plan(
    settings: &MountainsSettings,
    plan: &RecipePlan<MountainsMetadata>,
) -> RecipeValidation<MountainsMetrics> {
    let metadata = &plan.metadata;
    let mut issues = Vec::new();
    let min_height = metadata.heights.values().copied().min().unwrap_or(0);
    let max_height = metadata.heights.values().copied().max().unwrap_or(0);
    if min_height != settings.base_level || max_height.saturating_sub(min_height) != settings.relief
    {
        issues.push(format!(
            "mountain relief is {}..={}; expected base {} and relief {}",
            min_height, max_height, settings.base_level, settings.relief
        ));
    }
    let unique_peaks: BTreeSet<_> = metadata.peak_centres.iter().copied().collect();
    if metadata.peak_centres.len() != usize::from(settings.peak_count)
        || unique_peaks.len() != metadata.peak_centres.len()
    {
        issues.push("mountain peak count does not match its settings".to_owned());
    }
    if metadata
        .peak_centres
        .iter()
        .enumerate()
        .any(|(index, centre)| {
            metadata
                .peak_centres
                .iter()
                .skip(index.saturating_add(1))
                .any(|other| {
                    centre.distance(*other)
                        < if uses_expanded_geometry(settings) {
                            EXPANDED_MIN_PEAK_SEPARATION
                        } else {
                            MIN_PEAK_SEPARATION
                        }
                })
        })
    {
        issues.push("mountain peak centres are not spatially distinct".to_owned());
    }
    for (centre, target) in &metadata.peak_targets {
        if metadata.heights.get(centre) != Some(target) {
            issues.push(format!(
                "mountain summit {centre:?} does not reach its authored target"
            ));
            break;
        }
        if centre
            .neighbors()
            .into_iter()
            .filter_map(|neighbor| metadata.heights.get(&neighbor))
            .any(|neighbor| neighbor >= target)
        {
            issues.push(format!(
                "mountain summit {centre:?} is not a strict local maximum"
            ));
            break;
        }
    }
    if metadata.peak_targets.len() != metadata.peak_centres.len()
        || metadata
            .peak_centres
            .iter()
            .any(|centre| !metadata.peak_targets.contains_key(centre))
    {
        issues.push("mountain summit targets do not match their centres".to_owned());
    }
    if metadata.spur_cells.is_empty()
        || metadata
            .spur_cells
            .iter()
            .any(|coord| !metadata.heights.contains_key(coord))
    {
        issues.push("mountain ridge has no valid attached spur cells".to_owned());
    }
    if uses_expanded_geometry(settings) {
        let expected_mountain_cells: BTreeSet<_> = metadata
            .heights
            .iter()
            .filter_map(|(coord, level)| (*level > settings.base_level).then_some(*coord))
            .collect();
        if metadata.mountain_cells != expected_mountain_cells {
            issues.push("expanded mountain coverage metadata is stale".to_owned());
        }
        if coordinate_components(&expected_mountain_cells).len() != 1 {
            issues.push("expanded mountain cells do not form one connected massif".to_owned());
        }
        let coverage_percent = u32::try_from(metadata.mountain_cells.len())
            .unwrap_or(u32::MAX)
            .saturating_mul(100)
            / u32::try_from(metadata.heights.len())
                .unwrap_or(u32::MAX)
                .max(1);
        if !(52..=65).contains(&coverage_percent)
            || coverage_percent.abs_diff(expanded_coverage_target(settings)) > 1
        {
            issues.push(format!(
                "expanded mountain coverage is {coverage_percent}%; expected {}% within 52..=65",
                expanded_coverage_target(settings)
            ));
        }
        if metadata.main_spine.len() < 2
            || metadata
                .main_spine
                .windows(2)
                .any(|pair| matches!(pair, [first, second] if first.distance(*second) != 1))
        {
            issues.push("expanded mountain main spine is not contiguous".to_owned());
        }
        let expected_ridge_cells: BTreeSet<_> = metadata.main_spine.iter().copied().collect();
        if metadata.ridge_cells != expected_ridge_cells {
            issues.push("expanded mountain spine metadata is stale".to_owned());
        }
        if count_spine_turns(&metadata.main_spine) < 2 {
            issues.push("expanded mountain main spine does not meander".to_owned());
        }
        let radius = i32::try_from(plan.volume.grid_radius).unwrap_or(i32::MAX);
        let local_spine_rows: Vec<_> = metadata
            .main_spine
            .iter()
            .map(|coord| to_local(*coord, metadata.orientation).y())
            .collect();
        let spine_row_spread = local_spine_rows
            .iter()
            .copied()
            .max()
            .zip(local_spine_rows.iter().copied().min())
            .map(|(highest, lowest)| highest.saturating_sub(lowest))
            .unwrap_or_default();
        let min_spine_spread = expanded_spine_amplitude(radius, settings);
        if spine_row_spread < min_spine_spread {
            issues.push(format!(
                "expanded mountain spine spans {spine_row_spread} rows; expected at least \
                 {min_spine_spread} at radius {radius}"
            ));
        }
        let expected_branches =
            usize::try_from(expanded_branch_count(settings)).unwrap_or(usize::MAX);
        let min_branch_len =
            usize::try_from(expanded_branch_base_length(radius, settings).saturating_add(1))
                .unwrap_or(usize::MAX);
        if metadata.branch_spines.len() != expected_branches
            || metadata.branch_spines.iter().any(|branch| {
                branch.len() < min_branch_len
                    || branch
                        .first()
                        .is_none_or(|start| !metadata.ridge_cells.contains(start))
                    || branch
                        .windows(2)
                        .any(|pair| matches!(pair, [first, second] if first.distance(*second) != 1))
            })
        {
            issues.push(
                "expanded mountain branches do not match the requested contiguous complexity"
                    .to_owned(),
            );
        }
        let expected_spurs: BTreeSet<_> = metadata
            .branch_spines
            .iter()
            .flat_map(|branch| branch.iter().copied().skip(1))
            .collect();
        if metadata.spur_cells != expected_spurs {
            issues.push("expanded mountain branch metadata is stale".to_owned());
        }
        let branch_sides: BTreeSet<_> = metadata
            .branch_spines
            .iter()
            .filter_map(|branch| branch_side(branch, metadata.orientation))
            .collect();
        let branches_leave_spine = metadata.branch_spines.iter().all(|branch| {
            branch
                .iter()
                .skip(1)
                .filter(|coord| !metadata.ridge_cells.contains(coord))
                .count()
                >= branch.len().saturating_sub(2)
        });
        let branches_overlap = metadata
            .branch_spines
            .iter()
            .enumerate()
            .any(|(index, branch)| {
                let branch_cells: BTreeSet<_> = branch.iter().copied().collect();
                metadata
                    .branch_spines
                    .iter()
                    .skip(index.saturating_add(1))
                    .any(|other| other.iter().any(|coord| branch_cells.contains(coord)))
            });
        if branch_sides != BTreeSet::from([-1, 1]) || !branches_leave_spine || branches_overlap {
            issues.push(
                "expanded mountain branches are not distinct structures on both ridge sides"
                    .to_owned(),
            );
        }
        let distinct_peak_levels = metadata
            .peak_targets
            .values()
            .copied()
            .collect::<BTreeSet<_>>()
            .len();
        let peak_spread = metadata
            .peak_targets
            .values()
            .copied()
            .max()
            .zip(metadata.peak_targets.values().copied().min())
            .map(|(highest, lowest)| highest.saturating_sub(lowest))
            .unwrap_or_default();
        if distinct_peak_levels < 3 || peak_spread != expanded_peak_spread(settings) {
            issues.push(format!(
                "expanded mountain summits have {distinct_peak_levels} levels spanning {peak_spread}; expected at least three levels spanning {}",
                expanded_peak_spread(settings)
            ));
        }
        let local_peak_rows: BTreeSet<_> = metadata
            .peak_centres
            .iter()
            .map(|centre| to_local(*centre, metadata.orientation).y())
            .collect();
        if local_peak_rows.len() < 2 {
            issues.push("expanded mountain summits remain collinear".to_owned());
        }
    }
    if !valid_route(&metadata.high_pass) || !valid_route(&metadata.low_bypass) {
        issues.push("a mountain route is not a contiguous two-wide walker ribbon".to_owned());
    }
    let high_route_peak = metadata
        .high_pass
        .rows
        .iter()
        .flatten()
        .map(|position| position.level)
        .max();
    let low_route_peak = metadata
        .low_bypass
        .rows
        .iter()
        .flatten()
        .map(|position| position.level)
        .max();
    if high_route_peak <= low_route_peak {
        issues.push("the high pass does not rise above the low bypass".to_owned());
    }
    let high_positions = metadata.high_pass.positions();
    let low_positions = metadata.low_bypass.positions();
    if !high_positions.is_disjoint(&low_positions) {
        issues.push("the high pass and low bypass overlap".to_owned());
    }
    if metadata
        .high_pass
        .rows
        .iter()
        .chain(&metadata.low_bypass.rows)
        .flatten()
        .any(|position| {
            plan.volume
                .surfaces
                .get(position)
                .map(|surface| surface.access)
                != Some(SurfaceAccess::Ordinary)
        })
    {
        issues.push("a declared mountain route is not ordinary footing".to_owned());
    }

    let allowed_portals: BTreeSet<_> = metadata
        .high_pass
        .coords()
        .union(&metadata.low_bypass.coords())
        .copied()
        .collect();
    let accessible_foothills = accessible_foothills(metadata);
    let accessible_foothill_percent = u32::try_from(accessible_foothills.len())
        .unwrap_or(u32::MAX)
        .saturating_mul(100)
        / u32::try_from(metadata.mountain_cells.len())
            .unwrap_or(u32::MAX)
            .max(1);
    let largest_foothill_patch = walker_components_within(&accessible_foothills, &metadata.heights)
        .into_iter()
        .max_by_key(BTreeSet::len);
    let largest_foothill_patch_size = largest_foothill_patch.as_ref().map_or(0, BTreeSet::len);
    let accessible_foothill_relief = largest_foothill_patch
        .iter()
        .flat_map(|component| component.iter())
        .filter_map(|coord| metadata.heights.get(coord))
        .copied()
        .max()
        .unwrap_or(settings.base_level)
        .saturating_sub(settings.base_level);
    let foothill_plain_landings = largest_foothill_patch
        .as_ref()
        .map_or(0, |component| foothill_plain_landings(component, metadata));
    if uses_expanded_geometry(settings)
        && (accessible_foothill_percent < MIN_ACCESSIBLE_FOOTHILL_PERCENT
            || largest_foothill_patch_size < MIN_ACCESSIBLE_FOOTHILL_PATCH
            || accessible_foothill_relief < Level::try_from(FOOTHILL_TERRACE_DEPTH).unwrap_or(3)
            || foothill_plain_landings < MIN_FOOTHILL_PLAIN_LANDINGS)
    {
        issues.push(format!(
            "expanded player-side mountain foothills expose {accessible_foothill_percent}% \
             ordinary terrain \
             with a largest walker patch of {largest_foothill_patch_size} cells and relief \
             {accessible_foothill_relief}, attached to the plain at \
             {foothill_plain_landings} non-corridor landings; expected at least \
             {MIN_ACCESSIBLE_FOOTHILL_PERCENT}%, {MIN_ACCESSIBLE_FOOTHILL_PATCH} connected cells, \
             relief {FOOTHILL_TERRACE_DEPTH}, and {MIN_FOOTHILL_PLAIN_LANDINGS} landings"
        ));
    }
    if metadata
        .ridge_cells
        .intersection(&metadata.ordinary)
        .any(|coord| !allowed_portals.contains(coord))
    {
        issues.push("ordinary terrain creates an undeclared third ridge crossing".to_owned());
    }
    let grid_radius = i32::try_from(plan.volume.grid_radius).unwrap_or(i32::MAX);
    for edge_u in [-grid_radius, grid_radius] {
        let edge = from_local(edge_u, 0, metadata.orientation);
        if !metadata.ridge_cells.contains(&edge)
            || metadata
                .heights
                .get(&edge)
                .is_none_or(|height| *height < settings.base_level.saturating_add(RIDGE_CREST_RISE))
        {
            issues.push("the mountain ridge does not seal both map boundaries".to_owned());
            break;
        }
    }

    let party = plan.volume.anchors.get(PARTY_START).copied();
    let hostile = plan.volume.anchors.get(HOSTILE_START).copied();
    for (name, route) in [
        (HIGH_PASS, &metadata.high_pass),
        (LOW_BYPASS, &metadata.low_bypass),
    ] {
        let expected = route
            .rows
            .get(route.rows.len() / 2)
            .and_then(|row| row.first().copied());
        if plan.volume.anchors.get(name).copied() != expected {
            issues.push(format!("mountain anchor {name} is not route-derived"));
        }
    }
    let ordinary = &metadata.ordinary;
    let mut high_pass_steps = None;
    let mut low_bypass_steps = None;
    if let (Some(party), Some(hostile)) = (party, hostile) {
        let high_coords = metadata.high_pass.coords();
        let low_coords = metadata.low_bypass.coords();
        low_bypass_steps = shortest_path_steps(
            ordinary,
            &metadata.heights,
            party.coord,
            hostile.coord,
            &high_coords,
        );
        if low_bypass_steps.is_none() {
            issues.push("removing the high pass also broke the low bypass".to_owned());
        }
        high_pass_steps = shortest_path_steps(
            ordinary,
            &metadata.heights,
            party.coord,
            hostile.coord,
            &low_coords,
        );
        if high_pass_steps.is_none() {
            issues.push("removing the low bypass also broke the high pass".to_owned());
        }
        let both: BTreeSet<_> = high_coords.union(&low_coords).copied().collect();
        if shortest_path_steps(
            ordinary,
            &metadata.heights,
            party.coord,
            hostile.coord,
            &both,
        )
        .is_some()
        {
            issues.push(
                "the two sides remain connected after removing both declared routes".to_owned(),
            );
        }
    } else {
        issues.push("mountain actor anchors are missing".to_owned());
    }

    if plan
        .volume
        .columns
        .values()
        .flat_map(|column| &column.elements)
        .any(|element| {
            !matches!(
                element,
                VolumeElement::Solid(SolidMass {
                    material: SolidMaterialRole::Bedrock
                        | SolidMaterialRole::Stone
                        | SolidMaterialRole::Dirt
                        | SolidMaterialRole::Snow
                        | SolidMaterialRole::Ice,
                    ..
                })
            )
        })
    {
        issues.push("mountains contain a fill, metal, or non-Frozen material".to_owned());
    }

    let cliff_edges = count_cliff_edges(&metadata.heights);
    if cliff_edges < 12 {
        issues.push(format!(
            "mountain ridge has only {cliff_edges} deliberate cliff edges"
        ));
    }
    let inaccessible_peaks = u8::try_from(
        metadata
            .peak_centres
            .iter()
            .filter(|centre| !metadata.ordinary.contains(centre))
            .count(),
    )
    .unwrap_or(u8::MAX);
    let mountain_coverage_percent = u32::try_from(metadata.mountain_cells.len())
        .unwrap_or(u32::MAX)
        .saturating_mul(100)
        / u32::try_from(metadata.heights.len())
            .unwrap_or(u32::MAX)
            .max(1);
    let peak_height_spread = metadata
        .peak_targets
        .values()
        .copied()
        .max()
        .zip(metadata.peak_targets.values().copied().min())
        .map(|(highest, lowest)| highest.saturating_sub(lowest))
        .unwrap_or_default();
    let spine_turns = count_spine_turns(&metadata.main_spine);
    let branch_count = u32::try_from(metadata.branch_spines.len()).unwrap_or(u32::MAX);
    if !issues.is_empty() {
        return RecipeValidation::invalid(issues);
    }

    let reachable_elevation_levels = u32::try_from(
        ordinary
            .iter()
            .filter_map(|coord| metadata.heights.get(coord))
            .collect::<BTreeSet<_>>()
            .len(),
    )
    .unwrap_or(u32::MAX);
    let high_pass_steps = high_pass_steps.unwrap_or(u32::MAX);
    let low_bypass_steps = low_bypass_steps.unwrap_or(u32::MAX);
    let exposed = metadata
        .materials
        .values()
        .filter(|material| **material == SolidMaterialRole::Stone)
        .count();
    let exposed_stone_percent = u32::try_from(exposed)
        .unwrap_or(u32::MAX)
        .saturating_mul(100)
        / u32::try_from(metadata.materials.len())
            .unwrap_or(u32::MAX)
            .max(1);
    let spawn_height_difference = party
        .zip(hostile)
        .map(|(party, hostile)| {
            Level::try_from(party.level.abs_diff(hostile.level)).unwrap_or(Level::MAX)
        })
        .unwrap_or(0);
    let alternate_detour_percent = if high_pass_steps == 0 || high_pass_steps == u32::MAX {
        0
    } else {
        low_bypass_steps
            .saturating_sub(high_pass_steps)
            .saturating_mul(100)
            / high_pass_steps
    };
    let metrics = MountainsMetrics {
        tactical: TacticalMetrics {
            relief: settings.relief,
            barrier_cells: 0,
            critical_route_steps: high_pass_steps,
            spawn_height_difference,
            bank_high_ground_difference: 0,
            reachable_surfaces: u32::try_from(ordinary.len()).unwrap_or(u32::MAX),
            reachable_elevation_levels,
            alternate_detour_percent,
            river_sinuosity_percent: 0,
            environment_signature_percent: 100_u32.saturating_sub(exposed_stone_percent),
        },
        inaccessible_peaks,
        cliff_edges,
        high_pass_steps,
        low_bypass_steps,
        exposed_stone_percent,
        mountain_coverage_percent,
        peak_height_spread,
        spine_turns,
        branch_count,
    };
    RecipeValidation::valid(metrics)
}

fn valid_route(route: &MountainRoute) -> bool {
    !route.rows.is_empty()
        && route.rows.iter().all(|row| {
            let [first, second] = *row;
            first.coord.distance(second.coord) == 1 && first.level == second.level
        })
        && route.rows.windows(2).all(|pair| {
            let [before, after] = pair else {
                return false;
            };
            before[0].coord.distance(after[0].coord) == 1
                && before[1].coord.distance(after[1].coord) == 1
                && before[0].level.abs_diff(after[0].level) <= 1
                && before[1].level.abs_diff(after[1].level) <= 1
        })
}

fn shortest_path_steps(
    ordinary: &BTreeSet<HexCoord>,
    heights: &BTreeMap<HexCoord, Level>,
    start: HexCoord,
    goal: HexCoord,
    removed: &BTreeSet<HexCoord>,
) -> Option<u32> {
    if removed.contains(&start) || removed.contains(&goal) {
        return None;
    }
    let mut distances = BTreeMap::from([(start, 0_u32)]);
    let mut frontier = VecDeque::from([start]);
    while let Some(coord) = frontier.pop_front() {
        let steps = distances.get(&coord).copied().unwrap_or(u32::MAX);
        if coord == goal {
            return Some(steps);
        }
        for neighbor in coord.neighbors() {
            if ordinary.contains(&neighbor)
                && !removed.contains(&neighbor)
                && !distances.contains_key(&neighbor)
                && walker_step(coord, neighbor, heights)
            {
                distances.insert(neighbor, steps.saturating_add(1));
                frontier.push_back(neighbor);
            }
        }
    }
    None
}

fn count_cliff_edges(heights: &BTreeMap<HexCoord, Level>) -> u32 {
    u32::try_from(
        heights
            .iter()
            .flat_map(|(coord, level)| {
                coord
                    .neighbors()
                    .into_iter()
                    .filter_map(|neighbor| heights.get(&neighbor).map(|other| (neighbor, *other)))
                    .filter(move |(neighbor, other)| {
                        coord < neighbor && level.abs_diff(*other) >= 2
                    })
            })
            .count(),
    )
    .unwrap_or(u32::MAX)
}

fn count_spine_turns(spine: &[HexCoord]) -> u32 {
    u32::try_from(
        spine
            .windows(3)
            .filter(|window| {
                let [first, middle, last] = window else {
                    return false;
                };
                (
                    middle.x().saturating_sub(first.x()),
                    middle.y().saturating_sub(first.y()),
                ) != (
                    last.x().saturating_sub(middle.x()),
                    last.y().saturating_sub(middle.y()),
                )
            })
            .count(),
    )
    .unwrap_or(u32::MAX)
}

fn branch_side(branch: &[HexCoord], orientation: u8) -> Option<i32> {
    let (Some(start), Some(end)) = (branch.first(), branch.last()) else {
        return None;
    };
    let start = to_local(*start, orientation);
    let end = to_local(*end, orientation);
    let side = end.y().saturating_sub(start.y()).signum();
    (side != 0).then_some(side)
}

fn mountain_view_hint(
    grid_radius: u32,
    level_height: f32,
    base_level: Level,
    relief: Level,
    orientation: u8,
) -> Result<MapViewHint, CandidateAttemptError> {
    let radius = u16::try_from(grid_radius)
        .map_err(|_out_of_range| CandidateAttemptError::rejected("mountain radius is too large"))?;
    let base = i16::try_from(base_level)
        .map_err(|_out_of_range| CandidateAttemptError::rejected("mountain base is too high"))?;
    let relief = i16::try_from(relief)
        .map_err(|_out_of_range| CandidateAttemptError::rejected("mountain relief is too high"))?;
    let focus_height = f32::from(base.saturating_add(relief / 3)) * level_height;
    let frame = (f32::from(radius) * 4.0).max(f32::from(relief) * level_height * 2.5);
    let ridge_normal = from_local(0, i32::from(radius), orientation).to_world(0.0);
    let horizontal_length = (ridge_normal
        .x
        .mul_add(ridge_normal.x, ridge_normal.z * ridge_normal.z))
    .sqrt();
    if horizontal_length <= f32::EPSILON {
        return Err(CandidateAttemptError::rejected(
            "mountain ridge orientation has no view direction",
        ));
    }
    let eye_x = ridge_normal.x / horizontal_length * frame;
    let eye_z = ridge_normal.z / horizontal_length * frame;
    Ok(MapViewHint::new(
        (eye_x, focus_height + frame, eye_z),
        (0.0, focus_height, 0.0),
    ))
}

const fn from_local(local_x: i32, local_y: i32, orientation: u8) -> HexCoord {
    rotate_third(HexCoord::from_axial(local_x, local_y), orientation)
}

const fn to_local(coord: HexCoord, orientation: u8) -> HexCoord {
    rotate_third(coord, (3 - orientation % 3) % 3)
}

const fn rotate_third(coord: HexCoord, turns: u8) -> HexCoord {
    match turns % 3 {
        0 => coord,
        1 => HexCoord::from_axial(coord.z(), coord.x()),
        _ => HexCoord::from_axial(coord.y(), coord.z()),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    const BEDROCK: SubstanceId = SubstanceId(1);
    const STONE: SubstanceId = SubstanceId(2);
    const DIRT: SubstanceId = SubstanceId(3);
    const GRASS: SubstanceId = SubstanceId(4);
    const GRAVEL: SubstanceId = SubstanceId(5);
    const WATER: SubstanceId = SubstanceId(6);
    const METAL: SubstanceId = SubstanceId(7);
    const SNOW: SubstanceId = SubstanceId(8);
    const ICE: SubstanceId = SubstanceId(9);
    const BASALT: SubstanceId = SubstanceId(10);
    const LAVA: SubstanceId = SubstanceId(11);

    fn palette() -> TerrainPalette {
        TerrainPalette {
            bedrock: BEDROCK,
            stone: STONE,
            dirt: DIRT,
            grass: GRASS,
            gravel: GRAVEL,
            water: WATER,
            metal: METAL,
            worked_stone: SubstanceId(12),
            limestone: SubstanceId(13),
            slate: SubstanceId(14),
            timber: SubstanceId(15),
            terracotta: SubstanceId(16),
            snow: SNOW,
            ice: ICE,
            basalt: BASALT,
            lava: LAVA,
        }
    }

    fn is_solid(substance: SubstanceId) -> bool {
        !matches!(substance, SubstanceId::AIR | WATER | LAVA)
    }

    const fn mountain_settings(peak_count: u8) -> MountainsSettings {
        MountainsSettings {
            base_level: 15,
            relief: 15,
            peak_count,
        }
    }

    fn settings(peak_count: u8) -> ProceduralV2Settings {
        ProceduralV2Settings {
            environment: V2EnvironmentSettings::Frozen,
            recipe: V2RecipeSettings::Mountains(mountain_settings(peak_count)),
        }
    }

    fn expanded_settings(relief: Level, peak_count: u8) -> ProceduralV2Settings {
        ProceduralV2Settings {
            environment: V2EnvironmentSettings::Frozen,
            recipe: V2RecipeSettings::Mountains(MountainsSettings {
                base_level: 15,
                relief,
                peak_count,
            }),
        }
    }

    #[test]
    fn legacy_mountains_compatibility_map_is_pinned_and_keeps_two_independent_routes() {
        let first = build(12, 0.4, &settings(4), 129_704_046, &palette(), &is_solid)
            .expect("the legacy Mountains compatibility seed should generate");
        let second = build(12, 0.4, &settings(4), 129_704_046, &palette(), &is_solid)
            .expect("the repeated Mountains seed should generate");

        assert_eq!(first.map_fingerprint, second.map_fingerprint);
        assert_eq!(first.selected_candidate, second.selected_candidate);
        assert_eq!(first.selected_candidate, Some(1));
        assert_eq!(first.map_fingerprint, 5_936_297_593_294_798_068);
        assert_eq!(first.map.len(), 469);
        assert_eq!(first.candidates_evaluated, 8);
        assert!(!first.used_fallback);
        assert!(!first.special_regions.is_empty());
        assert_eq!(first.metadata.peak_centres.len(), 4);
        assert!(first.anchors.get(&HIGH_PASS.into()).is_some());
        assert!(first.anchors.get(&LOW_BYPASS.into()).is_some());
        assert!(first.metrics.inaccessible_peaks > 0);
        assert_eq!(first.metrics.tactical.barrier_cells, 0);
        assert_eq!(
            first.metrics.tactical.critical_route_steps,
            first.metrics.high_pass_steps
        );
        assert!(first.metrics.low_bypass_steps > first.metrics.high_pass_steps);
        assert_eq!(
            first.metrics.tactical.alternate_detour_percent,
            first
                .metrics
                .low_bypass_steps
                .saturating_sub(first.metrics.high_pass_steps)
                .saturating_mul(100)
                / first.metrics.high_pass_steps
        );
    }

    #[test]
    fn shipped_mountain_selection_is_pinned() {
        let generated = build(
            12,
            0.4,
            &expanded_settings(24, 7),
            129_704_046,
            &palette(),
            &is_solid,
        )
        .expect("the shipped Mountains selection should generate");

        assert_eq!(generated.selected_candidate, Some(0));
        assert_eq!(generated.map_fingerprint, 228_308_395_851_360_446);
        assert_eq!(generated.metrics.mountain_coverage_percent, 60);
        assert_eq!(generated.metadata.peak_centres.len(), 7);
        assert!(!generated.used_fallback);
        let foothills = accessible_foothills(&generated.metadata);
        let accessible_percent = u32::try_from(foothills.len())
            .unwrap_or(u32::MAX)
            .saturating_mul(100)
            / u32::try_from(generated.metadata.mountain_cells.len())
                .unwrap_or(u32::MAX)
                .max(1);
        let largest_patch = walker_components_within(&foothills, &generated.metadata.heights)
            .into_iter()
            .max_by_key(BTreeSet::len)
            .expect("the shipped mountains should have an accessible foothill patch");
        let foothill_relief = largest_patch
            .iter()
            .filter_map(|coord| generated.metadata.heights.get(coord))
            .copied()
            .max()
            .unwrap_or_default()
            .saturating_sub(15);
        assert!(accessible_percent >= MIN_ACCESSIBLE_FOOTHILL_PERCENT);
        assert!(largest_patch.len() >= MIN_ACCESSIBLE_FOOTHILL_PATCH);
        assert!(foothill_relief >= Level::try_from(FOOTHILL_TERRACE_DEPTH).unwrap_or(3));
        assert!(
            foothill_plain_landings(&largest_patch, &generated.metadata)
                >= MIN_FOOTHILL_PLAIN_LANDINGS
        );
    }

    #[test]
    fn expanded_mountain_settings_are_broad_complex_and_deterministic() {
        for (relief, peak_count, expected_coverage, expected_branches) in
            [(18, 5, 53, 2), (21, 6, 56, 3), (24, 7, 60, 4)]
        {
            let settings = expanded_settings(relief, peak_count);
            let first = build(12, 0.4, &settings, 129_704_046, &palette(), &is_solid)
                .unwrap_or_else(|error| {
                    panic!(
                        "expanded relief {relief} with {peak_count} peaks should generate: {error}"
                    )
                });
            let second = build(12, 0.4, &settings, 129_704_046, &palette(), &is_solid)
                .expect("the repeated expanded mountain should generate");

            assert_eq!(first.map_fingerprint, second.map_fingerprint);
            assert_eq!(first.selected_candidate, second.selected_candidate);
            assert!(!first.used_fallback);
            assert_eq!(first.metrics.tactical.relief, relief);
            assert_eq!(first.metrics.mountain_coverage_percent, expected_coverage);
            assert_eq!(first.metrics.branch_count, expected_branches);
            assert_eq!(
                first.metrics.peak_height_spread,
                expanded_peak_spread(match &settings.recipe {
                    V2RecipeSettings::Mountains(settings) => settings,
                    _ => unreachable!("fixture is Mountains"),
                })
            );
            assert!(first.metrics.spine_turns >= 2);
            assert!(first.metrics.low_bypass_steps > first.metrics.high_pass_steps);
            assert!(first.metrics.inaccessible_peaks > 0);
            assert_eq!(
                coordinate_components(&first.metadata.mountain_cells).len(),
                1
            );
            assert_eq!(
                first
                    .metadata
                    .branch_spines
                    .iter()
                    .filter_map(|branch| branch_side(branch, first.metadata.orientation))
                    .collect::<BTreeSet<_>>(),
                BTreeSet::from([-1, 1])
            );
        }
    }

    #[test]
    fn expanded_fixed_seed_corpus_preserves_massif_and_branch_contracts() {
        for (relief, peak_count, expected_coverage) in [(18, 5, 53), (21, 6, 56), (24, 7, 60)] {
            for seed in [0, 1, 42, 505, 808, 129_704_046, u64::MAX] {
                let generated = build(
                    12,
                    0.4,
                    &expanded_settings(relief, peak_count),
                    seed,
                    &palette(),
                    &is_solid,
                )
                .unwrap_or_else(|error| {
                    panic!(
                        "expanded relief {relief}, peaks {peak_count}, seed {seed} should generate: {error}"
                    )
                });

                assert!(
                    !generated.used_fallback,
                    "expanded relief {relief}, peaks {peak_count}, seed {seed} used fallback"
                );
                assert_eq!(
                    generated.metrics.mountain_coverage_percent, expected_coverage,
                    "expanded relief {relief}, peaks {peak_count}, seed {seed} changed coverage"
                );
                assert_eq!(
                    coordinate_components(&generated.metadata.mountain_cells).len(),
                    1,
                    "expanded relief {relief}, peaks {peak_count}, seed {seed} fragmented"
                );
                assert_eq!(
                    generated
                        .metadata
                        .branch_spines
                        .iter()
                        .filter_map(|branch| {
                            branch_side(branch, generated.metadata.orientation)
                        })
                        .collect::<BTreeSet<_>>(),
                    BTreeSet::from([-1, 1]),
                    "expanded relief {relief}, peaks {peak_count}, seed {seed} lost a ridge side"
                );
            }
        }
    }

    #[test]
    fn expanded_seed_6401_keeps_every_authored_peak_connected() {
        let generated = build(
            12,
            0.4,
            &expanded_settings(21, 6),
            6_401,
            &palette(),
            &is_solid,
        )
        .expect("the expanded Mountains regression seed should generate");

        assert_eq!(
            coordinate_components(&generated.metadata.mountain_cells).len(),
            1
        );
        assert_eq!(generated.metadata.peak_centres.len(), 6);
        assert!(generated
            .metadata
            .peak_centres
            .iter()
            .all(|peak| generated.metadata.mountain_cells.contains(peak)));
    }

    #[test]
    fn expanded_mountain_coverage_scales_to_large_radii() {
        for (radius, relief, peak_count, expected_coverage, expected_branches) in
            [(20, 21, 6, 56, 3), (40, 24, 7, 60, 4)]
        {
            let generated = build(
                radius,
                0.4,
                &expanded_settings(relief, peak_count),
                129_704_046,
                &palette(),
                &is_solid,
            )
            .unwrap_or_else(|error| {
                panic!(
                    "expanded radius {radius} with relief {relief} and {peak_count} peaks should generate: {error}"
                )
            });

            let expected_columns =
                1_u32.saturating_add(3_u32.saturating_mul(radius).saturating_mul(radius + 1));
            assert_eq!(
                generated.map.len(),
                usize::try_from(expected_columns).unwrap_or(usize::MAX)
            );
            assert!(!generated.used_fallback);
            assert_eq!(
                generated.metrics.mountain_coverage_percent,
                expected_coverage
            );
            assert_eq!(generated.metrics.branch_count, expected_branches);
            assert!(generated.metrics.spine_turns >= 2);
            assert!(generated.metrics.low_bypass_steps > generated.metrics.high_pass_steps);
            assert_eq!(
                coordinate_components(&generated.metadata.mountain_cells).len(),
                1
            );
        }
    }

    #[test]
    fn expanded_fallback_geometry_scales_with_supported_radius() {
        let recipe = MountainsRecipe { level_height: 0.4 };
        let settings = MountainsSettings {
            base_level: 15,
            relief: 24,
            peak_count: 7,
        };
        for (radius, min_row_spread, min_branch_len) in [(12, 6, 8), (20, 8, 10), (40, 12, 15)] {
            let plan = recipe
                .canonical_fallback(
                    FallbackContext {
                        grid_radius: radius,
                    },
                    &settings,
                )
                .unwrap_or_else(|error| {
                    panic!("expanded radius {radius} fallback should construct: {error}")
                });
            let local_rows: Vec<_> = plan
                .metadata
                .main_spine
                .iter()
                .map(|coord| to_local(*coord, plan.metadata.orientation).y())
                .collect();
            let row_spread = local_rows
                .iter()
                .copied()
                .max()
                .zip(local_rows.iter().copied().min())
                .map(|(highest, lowest)| highest.saturating_sub(lowest))
                .unwrap_or_default();
            let shortest_branch = plan
                .metadata
                .branch_spines
                .iter()
                .map(Vec::len)
                .min()
                .unwrap_or_default();

            assert!(
                row_spread >= min_row_spread,
                "radius {radius} spine spread {row_spread} is below {min_row_spread}"
            );
            assert!(
                shortest_branch >= min_branch_len,
                "radius {radius} shortest branch {shortest_branch} is below {min_branch_len}"
            );
            assert_eq!(
                plan.metadata
                    .branch_spines
                    .iter()
                    .filter_map(|branch| branch_side(branch, plan.metadata.orientation))
                    .collect::<BTreeSet<_>>(),
                BTreeSet::from([-1, 1])
            );
            assert_eq!(
                coordinate_components(&plan.metadata.mountain_cells).len(),
                1
            );
        }
    }

    #[test]
    fn expanded_canonical_fallback_is_valid_and_seed_independent() {
        let recipe = MountainsRecipe { level_height: 0.4 };
        for (relief, peak_count) in [(18, 5), (21, 6), (24, 7)] {
            let settings = MountainsSettings {
                base_level: 15,
                relief,
                peak_count,
            };
            let first = recipe
                .canonical_fallback(FallbackContext { grid_radius: 12 }, &settings)
                .unwrap_or_else(|error| {
                    panic!(
                        "expanded relief {relief}, peaks {peak_count} fallback should construct: \
                         {error}"
                    )
                });
            let second = recipe
                .canonical_fallback(FallbackContext { grid_radius: 12 }, &settings)
                .expect("the repeated expanded mountain fallback should construct");

            first
                .volume
                .validate()
                .expect("the expanded mountain fallback volume should validate");
            assert!(matches!(
                validate_plan(&settings, &first),
                RecipeValidation::Valid(_)
            ));
            assert!(first
                .metadata
                .peak_centres
                .iter()
                .all(|peak| first.metadata.mountain_cells.contains(peak)));
            assert_eq!(first.volume.columns, second.volume.columns);
            assert_eq!(first.volume.surfaces, second.volume.surfaces);
            assert_eq!(first.volume.anchors, second.volume.anchors);
            assert_eq!(first.metadata.heights, second.metadata.heights);
        }
    }

    #[test]
    fn supported_radii_and_peak_counts_generate_hard_valid_maps() {
        for radius in [12, 20, 40] {
            for peak_count in 3..=5 {
                let generated = build(
                    radius,
                    0.4,
                    &settings(peak_count),
                    u64::from(radius).saturating_mul(100) + u64::from(peak_count),
                    &palette(),
                    &is_solid,
                )
                .unwrap_or_else(|error| {
                    panic!("radius {radius} with {peak_count} peaks should generate: {error}")
                });
                assert_eq!(generated.metadata.peak_centres.len(), peak_count as usize);
                assert_eq!(generated.metrics.tactical.relief, 15);
            }
        }
    }

    #[test]
    fn fixed_seed_corpus_is_valid_deterministic_and_repair_free() {
        for seed in [0, 1, 505, 808, 129_704_046, u64::MAX] {
            let first = build(12, 0.4, &settings(4), seed, &palette(), &is_solid)
                .unwrap_or_else(|error| panic!("seed {seed} should generate: {error}"));
            let second = build(12, 0.4, &settings(4), seed, &palette(), &is_solid)
                .unwrap_or_else(|error| panic!("repeated seed {seed} should generate: {error}"));

            assert!(
                !first.used_fallback,
                "seed {seed} unexpectedly used fallback"
            );
            assert!(first.valid_candidates > 0);
            assert!(first.repair_actions.is_empty());
            assert_eq!(first.map_fingerprint, second.map_fingerprint);
            assert_eq!(first.selected_candidate, second.selected_candidate);
        }
    }

    #[test]
    fn radius_12_pr_corpus_validates_128_mountain_seeds_and_named_regressions() {
        let settings = expanded_settings(24, 7);
        let mut seeds: BTreeSet<u64> = (0..128).collect();
        seeds.extend([505, 808, 129_704_046, u64::MAX]);
        let mut fallbacks = 0_usize;

        for &seed in &seeds {
            let generated = build(12, 0.4, &settings, seed, &palette(), &is_solid)
                .unwrap_or_else(|error| panic!("radius-12 Mountains seed {seed}: {error}"));
            fallbacks += usize::from(generated.used_fallback);
        }

        assert!(
            fallbacks.saturating_mul(100) < seeds.len(),
            "{fallbacks}/{} radius-12 Mountains seeds used fallback",
            seeds.len()
        );
    }

    #[test]
    fn canonical_fallback_is_valid_and_seed_independent() {
        let recipe = MountainsRecipe { level_height: 0.4 };
        let settings = mountain_settings(4);
        let first = recipe
            .canonical_fallback(FallbackContext { grid_radius: 12 }, &settings)
            .expect("the canonical mountain fallback should construct");
        let second = recipe
            .canonical_fallback(FallbackContext { grid_radius: 12 }, &settings)
            .expect("the repeated mountain fallback should construct");

        first
            .volume
            .validate()
            .expect("the canonical mountain fallback volume should validate");
        assert!(matches!(
            validate_plan(&settings, &first),
            RecipeValidation::Valid(_)
        ));
        assert_eq!(first.volume.columns, second.volume.columns);
        assert_eq!(first.volume.surfaces, second.volume.surfaces);
        assert_eq!(first.volume.anchors, second.volume.anchors);
        assert_eq!(first.volume.view_hint, second.volume.view_hint);
        assert_eq!(first.metadata.heights, second.metadata.heights);
        assert_eq!(first.metadata.peak_targets, second.metadata.peak_targets);
    }

    #[test]
    fn validation_rejects_corrupt_mountain_semantics() {
        let recipe = MountainsRecipe { level_height: 0.4 };
        let settings = mountain_settings(4);
        let context = CandidateContext {
            grid_radius: 12,
            candidate: 0,
            streams: super::super::seed::SeedStreams::new(129_704_046, 0),
        };
        let valid = construct_plan(&recipe, context, &settings, false)
            .expect("the fixed mountain candidate should construct");

        let mut duplicate_peak = valid.clone();
        let first_peak = duplicate_peak
            .metadata
            .peak_centres
            .first()
            .copied()
            .expect("the valid candidate should have a peak");
        *duplicate_peak
            .metadata
            .peak_centres
            .get_mut(1)
            .expect("the valid candidate should have a second peak") = first_peak;
        assert!(validation_issues(&settings, &duplicate_peak)
            .iter()
            .any(|issue| issue.contains("peak count")));

        let mut stale_target = valid.clone();
        let centre = stale_target
            .metadata
            .peak_centres
            .first()
            .copied()
            .expect("the valid candidate should have a peak");
        stale_target.metadata.peak_targets.remove(&centre);
        assert!(validation_issues(&settings, &stale_target)
            .iter()
            .any(|issue| issue.contains("summit targets")));

        let mut inverted_routes = valid.clone();
        for position in inverted_routes.metadata.high_pass.rows.iter_mut().flatten() {
            position.level = settings.base_level;
        }
        assert!(validation_issues(&settings, &inverted_routes)
            .iter()
            .any(|issue| issue.contains("high pass does not rise")));

        let mut extra_portal = valid.clone();
        let route_coords: BTreeSet<_> = extra_portal
            .metadata
            .high_pass
            .coords()
            .union(&extra_portal.metadata.low_bypass.coords())
            .copied()
            .collect();
        let undeclared = extra_portal
            .metadata
            .ridge_cells
            .iter()
            .copied()
            .find(|coord| !route_coords.contains(coord))
            .expect("the valid ridge should contain a non-route cell");
        extra_portal.metadata.ordinary.insert(undeclared);
        assert!(validation_issues(&settings, &extra_portal)
            .iter()
            .any(|issue| issue.contains("undeclared third ridge crossing")));

        let mut metal = valid;
        let column = metal
            .volume
            .columns
            .values_mut()
            .next()
            .expect("the valid plan should contain a column");
        let VolumeElement::Solid(top) = column
            .elements
            .last_mut()
            .expect("the valid mountain column should contain solid material")
        else {
            panic!("the valid mountain column should end in solid material");
        };
        top.material = SolidMaterialRole::Metal;
        assert!(validation_issues(&settings, &metal)
            .iter()
            .any(|issue| issue.contains("fill, metal, or non-Frozen")));
    }

    #[test]
    fn expanded_validation_rejects_fragmented_or_overlapping_geometry() {
        let recipe = MountainsRecipe { level_height: 0.4 };
        let settings = MountainsSettings {
            base_level: 15,
            relief: 24,
            peak_count: 7,
        };
        let valid = recipe
            .canonical_fallback(FallbackContext { grid_radius: 12 }, &settings)
            .expect("the expanded fallback should construct");

        let mut fragmented = valid.clone();
        let route_coords: BTreeSet<_> = fragmented
            .metadata
            .high_pass
            .coords()
            .union(&fragmented.metadata.low_bypass.coords())
            .copied()
            .collect();
        let island = fragmented
            .metadata
            .mountain_cells
            .iter()
            .copied()
            .find(|coord| {
                !fragmented.metadata.peak_centres.contains(coord)
                    && !fragmented.metadata.ridge_cells.contains(coord)
                    && !route_coords.contains(coord)
                    && fragmented
                        .metadata
                        .peak_centres
                        .iter()
                        .all(|peak| coord.distance(*peak) > 2)
                    && coord
                        .neighbors()
                        .into_iter()
                        .all(|neighbor| fragmented.metadata.heights.contains_key(&neighbor))
            })
            .expect("the fallback should have a noncritical interior mountain cell");
        for neighbor in island.neighbors() {
            if let Some(height) = fragmented.metadata.heights.get_mut(&neighbor) {
                *height = settings.base_level;
                fragmented.metadata.mountain_cells.remove(&neighbor);
            }
        }
        assert!(
            validation_issues(&settings, &fragmented)
                .iter()
                .any(|issue| issue.contains("one connected massif")),
            "an isolated elevated cell must fail the massif contract"
        );

        let mut corridor_only = valid.clone();
        let route_coords: BTreeSet<_> = corridor_only
            .metadata
            .high_pass
            .coords()
            .union(&corridor_only.metadata.low_bypass.coords())
            .copied()
            .collect();
        let mountain_cells = corridor_only.metadata.mountain_cells.clone();
        corridor_only
            .metadata
            .ordinary
            .retain(|coord| !mountain_cells.contains(coord) || route_coords.contains(coord));
        assert!(
            validation_issues(&settings, &corridor_only)
                .iter()
                .any(|issue| issue.contains("player-side mountain foothills expose")),
            "route-only mountain access must fail the foothill contract"
        );

        let mut overlapping = valid;
        let first_branch = overlapping
            .metadata
            .branch_spines
            .first()
            .cloned()
            .expect("the fallback should have a branch");
        *overlapping
            .metadata
            .branch_spines
            .get_mut(1)
            .expect("the fallback should have a second branch") = first_branch;
        overlapping.metadata.spur_cells = overlapping
            .metadata
            .branch_spines
            .iter()
            .flat_map(|branch| branch.iter().copied().skip(1))
            .collect();
        assert!(
            validation_issues(&settings, &overlapping)
                .iter()
                .any(|issue| issue.contains("distinct structures on both ridge sides")),
            "overlapping branches must fail the geometric-complexity contract"
        );
    }

    #[test]
    fn elevated_core_rejects_a_peak_detached_from_the_spine() {
        let spine = [HexCoord::ORIGIN, HexCoord::from_axial(1, 0)];
        let detached = [HexCoord::from_axial(5, 0), HexCoord::from_axial(6, 0)];
        let mut heights = spine
            .into_iter()
            .chain(detached)
            .map(|coord| (coord, 20))
            .collect::<BTreeMap<_, _>>();
        let mut mountain_cells = heights.keys().copied().collect();
        let error = retain_elevated_core(
            &mut heights,
            &mut mountain_cells,
            15,
            &spine,
            &[spine[0], detached[0]],
        )
        .expect_err("a detached authored peak must reject the candidate");

        assert!(
            matches!(
                error,
                CandidateAttemptError::Rejected(ref issues)
                    if issues.iter().any(|issue| issue.contains("detached authored mountain peak"))
            ),
            "unexpected retention error: {error:?}"
        );
    }

    fn validation_issues(
        settings: &MountainsSettings,
        plan: &RecipePlan<MountainsMetadata>,
    ) -> Vec<String> {
        match validate_plan(settings, plan) {
            RecipeValidation::Valid(_metrics) => Vec::new(),
            RecipeValidation::Invalid(issues) => issues,
        }
    }

    #[test]
    fn generated_view_looks_across_every_ridge_orientation() {
        let mut eyes = BTreeSet::new();
        for orientation in 0..3 {
            let hint = mountain_view_hint(12, 0.4, 15, 15, orientation)
                .expect("every ridge orientation should have a valid view");
            let normal = from_local(0, 12, orientation).to_world(0.0);
            let dot = hint.eye.0.mul_add(normal.x, hint.eye.2 * normal.z);
            assert!(dot > 0.0);
            eyes.insert((
                hint.eye.0.to_bits(),
                hint.eye.1.to_bits(),
                hint.eye.2.to_bits(),
            ));
        }
        assert_eq!(eyes.len(), 3);
    }

    #[test]
    #[ignore = "10,000 seeds are a manual stress corpus"]
    fn ten_thousand_seed_corpus_has_less_than_one_percent_fallbacks() {
        let mut fallback_count = 0_u32;
        for seed in 0..10_000 {
            let generated = build(12, 0.4, &settings(4), seed, &palette(), &is_solid)
                .unwrap_or_else(|error| panic!("seed {seed} should generate: {error}"));
            fallback_count += u32::from(generated.used_fallback);
        }
        assert!(
            fallback_count < 100,
            "{fallback_count} of 10,000 maps used fallback"
        );
    }

    #[test]
    #[ignore = "30,000 expanded maps are a manual stress corpus"]
    fn expanded_ten_thousand_seed_corpora_have_less_than_one_percent_fallbacks() {
        for (relief, peak_count) in [(18, 5), (21, 6), (24, 7)] {
            let settings = expanded_settings(relief, peak_count);
            let mut fallback_count = 0_u32;
            for seed in 0..10_000 {
                let generated = build(12, 0.4, &settings, seed, &palette(), &is_solid)
                    .unwrap_or_else(|error| {
                        panic!(
                            "expanded relief {relief}, peaks {peak_count}, seed {seed} should generate: {error}"
                        )
                    });
                fallback_count = fallback_count.saturating_add(u32::from(generated.used_fallback));
            }
            assert!(
                fallback_count < 100,
                "expanded relief {relief}, peaks {peak_count} used fallback for \
                 {fallback_count} of 10,000 maps"
            );
        }
    }

    #[test]
    #[ignore = "manual release/debug generator benchmark"]
    fn mountain_radius_benchmark_tracks_the_radius_40_target() {
        let settings = settings(4);
        let palette = palette();
        let mut radius_40_median = 0_u128;
        for radius in [12, 20, 40] {
            let mut samples = Vec::new();
            for seed in 0..8 {
                let started = Instant::now();
                let generated = build(radius, 0.4, &settings, seed, &palette, &is_solid)
                    .expect("the benchmark map should generate");
                samples.push(started.elapsed().as_micros());
                std::hint::black_box(generated);
            }
            samples.sort_unstable();
            let median = samples
                .get(samples.len() / 2)
                .copied()
                .expect("the benchmark always records eight samples");
            eprintln!("Mountains radius {radius}: median={median}us");
            if radius == 40 {
                radius_40_median = median;
            }
        }

        let target_micros = if cfg!(debug_assertions) {
            250_000
        } else {
            50_000
        };
        eprintln!(
            "Mountains radius 40 median={radius_40_median}us target={target_micros}us (trend only)"
        );
    }

    #[test]
    #[ignore = "manual release/debug expanded-generator benchmark"]
    fn expanded_mountain_radius_benchmark_tracks_the_radius_40_target() {
        let settings = expanded_settings(24, 7);
        let palette = palette();
        let mut radius_40_median = 0_u128;
        for radius in [12, 20, 40] {
            let mut samples = Vec::new();
            for seed in 0..8 {
                let started = Instant::now();
                let generated = build(radius, 0.4, &settings, seed, &palette, &is_solid)
                    .expect("the expanded benchmark map should generate");
                samples.push(started.elapsed().as_micros());
                std::hint::black_box(generated);
            }
            samples.sort_unstable();
            let median = samples
                .get(samples.len() / 2)
                .copied()
                .expect("the benchmark always records eight samples");
            eprintln!("Expanded Mountains radius {radius}: median={median}us");
            if radius == 40 {
                radius_40_median = median;
            }
        }

        let target_micros = if cfg!(debug_assertions) {
            250_000
        } else {
            50_000
        };
        eprintln!(
            "Expanded Mountains radius 40 median={radius_40_median}us \
             target={target_micros}us (trend only)"
        );
    }
}
