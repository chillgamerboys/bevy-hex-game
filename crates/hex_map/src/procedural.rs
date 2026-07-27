//! Deterministic semantic-first procedural terrain.
//!
//! Geometry and tactical intent are represented as a small [`TerrainPlan`] before
//! any substances are written. This keeps route repair meaningful: changing a
//! crossing or lowering a slope can rerun material classification and voxelization
//! instead of carving unexplained scars into an already materialized map.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Instant;

use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use hex_core::{
    HexCoord, Level, SpecialMovementRegion, SpecialMovementRegions, SubstanceId, TilePos,
    TraversalProfile,
};
use xxhash_rust::xxh3::{xxh3_64, xxh3_64_with_seed};

use crate::settings::{
    CrossingSettings, EnvironmentSettings, HillsSettings, LandformSettings, LinkedIslandsSettings,
    ProceduralV1Settings as ProceduralSettings, SkyIslandsSettings, TacticalSettings,
};
use crate::terrain::TerrainPalette;
use crate::voxel::{Column, VoxelMap};

pub(crate) const CANDIDATE_COUNT: u8 = 8;
const MAX_REPAIR_ROUNDS: u8 = 4;
const TOPSOIL_LEVELS: Level = 3;

pub(crate) const PARTY_START: &str = "party_start";
pub(crate) const HOSTILE_START: &str = "hostile_start";
pub(crate) const CONFLICT_CENTER: &str = "conflict_center";
pub(crate) const BRIDGE: &str = "bridge";
pub(crate) const ALTERNATE_CROSSING: &str = "alternate_crossing";

/// Diagnostics for one completed procedural generation.
#[derive(Resource, Reflect, Debug, Clone)]
#[reflect(Resource)]
pub struct GenerationReport {
    /// Algorithm contract used for the build.
    pub generator_version: u32,
    /// Resolved scenario or session seed.
    pub seed: u64,
    /// Index of the selected candidate, or `None` when the canonical fallback won.
    pub selected_candidate: Option<u8>,
    /// Number of candidates evaluated. V1 always evaluates exactly eight.
    pub candidates_evaluated: u8,
    /// Number of candidates that passed every hard validation.
    pub valid_candidates: u8,
    /// Semantic repair rounds used by the selected result.
    pub repair_rounds: u8,
    /// Ordered semantic repair actions applied to the selected result.
    pub repair_actions: Vec<String>,
    /// Whether every random candidate failed and the canonical layout was used.
    pub used_fallback: bool,
    /// Stable hash of settings that affect generated output.
    pub settings_fingerprint: u64,
    /// Stable hash of the sorted voxel map and its special-movement memberships.
    pub map_fingerprint: u64,
    /// Tactical measurements of the selected result.
    pub metrics: TacticalMetrics,
    /// Time spent evaluating all candidates, excluded from deterministic comparisons.
    pub elapsed_micros: u64,
    /// Validation notes. Empty for an ordinary valid candidate.
    pub notes: Vec<String>,
}

/// Small, deterministic measurements used to compare hard-valid candidates.
#[derive(Reflect, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TacticalMetrics {
    /// Highest dry surface minus the valley or island-chain surface.
    pub relief: Level,
    /// Number of coordinates occupied by the semantic hazard barrier.
    pub barrier_cells: u32,
    /// Ordinary-walker steps between the opposing start anchors.
    pub critical_route_steps: u32,
    /// Difference between the highest surfaces near the two start anchors.
    pub spawn_height_difference: Level,
    /// Difference between the highest reachable surfaces on the two opposing banks.
    pub bank_high_ground_difference: Level,
    /// Number of standable surfaces reachable from the party anchor.
    pub reachable_surfaces: u32,
    /// Number of distinct reachable surface levels.
    pub reachable_elevation_levels: u32,
    /// Percentage by which the alternate crossing route exceeds the bridge route.
    pub alternate_detour_percent: u32,
    /// Extra river-centreline length over the direct edge-to-edge distance.
    pub river_sinuosity_percent: u32,
    /// Percentage of cells carrying the environment's characteristic surface.
    pub environment_signature_percent: u32,
}

#[derive(Debug)]
pub(crate) struct ProceduralBuild {
    pub(crate) map: VoxelMap,
    pub(crate) anchors: GeneratedAnchors,
    pub(crate) special_regions: SpecialMovementRegions,
    pub(crate) report: GenerationReport,
    pub(crate) validated: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct GeneratedAnchors {
    positions: BTreeMap<&'static str, TilePos>,
}

impl GeneratedAnchors {
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&'static str, TilePos)> + '_ {
        self.positions.iter().map(|(name, pos)| (*name, *pos))
    }

    fn get(&self, name: &str) -> Option<TilePos> {
        self.positions.get(name).copied()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SurfaceRole {
    Grass,
    Gravel,
    Dirt,
    Stone,
    Snow,
    Ice,
    Basalt,
    Metal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HazardRole {
    Water,
    Lava,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Foundation {
    SolidToBedrock,
    Floating { bottom: Level },
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Hazard {
    bottom: Level,
    top: Level,
    role: HazardRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Overlay {
    level: Level,
    role: SurfaceRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlannedCell {
    surface: Level,
    role: SurfaceRole,
    foundation: Foundation,
    hazard: Option<Hazard>,
    overlay: Option<Overlay>,
    gated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecipeKind {
    HillsCrossing,
    LinkedSkyIslands,
}

#[derive(Debug, Clone)]
struct TerrainPlan {
    grid_radius: u32,
    cells: HashMap<HexCoord, PlannedCell>,
    anchors: GeneratedAnchors,
    barrier: BTreeSet<HexCoord>,
    barrier_centres: Vec<HexCoord>,
    barrier_sections: Vec<[HexCoord; 3]>,
    bridge: BTreeSet<TilePos>,
    alternate: BTreeSet<TilePos>,
    crossing_lanes: Vec<(BTreeSet<TilePos>, BTreeSet<TilePos>)>,
    crossing_rows: Vec<Vec<[TilePos; 2]>>,
    sky_bridge_lanes: Vec<(BTreeSet<TilePos>, BTreeSet<TilePos>)>,
    gated: BTreeSet<HexCoord>,
    base_level: Level,
    kind: RecipeKind,
}

#[derive(Debug)]
struct ValidCandidate {
    plan: TerrainPlan,
    map: VoxelMap,
    candidate: u8,
    repair_actions: Vec<String>,
    metrics: TacticalMetrics,
    score: CandidateScore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct CandidateScore {
    detour_band_distance: u32,
    route_band_distance: u32,
    spawn_imbalance: Level,
    high_ground_band_distance: u32,
    relief_band_distance: u32,
    elevation_diversity_distance: u32,
    river_shape_band_distance: u32,
    environment_signature_distance: u32,
    candidate: u8,
}

#[derive(Debug)]
struct Validation {
    valid: bool,
    notes: Vec<String>,
    metrics: TacticalMetrics,
}

/// Standable positions and their precomputed walker edges for one candidate.
///
/// Hard validation runs several reachability checks with different crossing tiles
/// removed. Building adjacency once keeps those checks linear instead of repeatedly
/// rediscovering the same six neighbors through ordered-set range queries.
struct TraversalGraph {
    surfaces: HashSet<TilePos>,
    edges: HashMap<TilePos, Vec<TilePos>>,
}

/// Generates, validates, and selects one complete deterministic procedural map.
pub(crate) fn build(
    grid_radius: u32,
    settings: &ProceduralSettings,
    seed: u64,
    palette: &TerrainPalette,
    walker: TraversalProfile,
    is_solid: &dyn Fn(SubstanceId) -> bool,
) -> ProceduralBuild {
    build_with_candidate_selection(grid_radius, settings, seed, palette, walker, is_solid, true)
}

fn build_with_candidate_selection(
    grid_radius: u32,
    settings: &ProceduralSettings,
    seed: u64,
    palette: &TerrainPalette,
    walker: TraversalProfile,
    is_solid: &dyn Fn(SubstanceId) -> bool,
    select_random_candidates: bool,
) -> ProceduralBuild {
    let started = Instant::now();
    let mut valid = Vec::new();
    let mut rejected_notes = Vec::new();
    let mut hard_valid_candidates = 0_u8;

    for candidate in 0..CANDIDATE_COUNT {
        let mut plan = construct_plan(grid_radius, settings, seed, candidate, false);
        let (map, validation, repair_actions) = voxelize_validate_repair(
            &mut plan, settings, seed, candidate, palette, walker, is_solid,
        );
        if validation.valid {
            hard_valid_candidates = hard_valid_candidates.saturating_add(1);
            if select_random_candidates {
                let score =
                    score_candidate(&plan, settings.environment, validation.metrics, candidate);
                valid.push(ValidCandidate {
                    plan,
                    map,
                    candidate,
                    repair_actions,
                    metrics: validation.metrics,
                    score,
                });
            } else {
                rejected_notes.push(format!(
                    "candidate {candidate}: excluded by forced-fallback verification"
                ));
            }
        } else {
            rejected_notes.push(format!(
                "candidate {candidate}: {}",
                validation.notes.join("; ")
            ));
        }
    }

    let selected = valid.into_iter().min_by_key(|candidate| candidate.score);

    let (
        map,
        anchors,
        special_regions,
        selected_candidate,
        repair_actions,
        used_fallback,
        metrics,
        notes,
        validated,
    ) = if let Some(candidate) = selected {
        let special_regions = special_movement_regions(&candidate.plan);
        (
            candidate.map,
            candidate.plan.anchors,
            special_regions,
            Some(candidate.candidate),
            candidate.repair_actions,
            false,
            candidate.metrics,
            Vec::new(),
            true,
        )
    } else {
        let mut plan = construct_plan(grid_radius, settings, seed, 0, true);
        let (map, validation, repair_actions) =
            voxelize_validate_repair(&mut plan, settings, seed, 0, palette, walker, is_solid);
        let special_regions = special_movement_regions(&plan);
        let mut notes = rejected_notes;
        notes.push("all random candidates failed; canonical fallback selected".to_owned());
        notes.extend(validation.notes);
        let validated = validation.valid;
        (
            map,
            plan.anchors,
            special_regions,
            None,
            repair_actions,
            true,
            validation.metrics,
            notes,
            validated,
        )
    };

    let elapsed_micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    let repair_rounds = u8::try_from(repair_actions.len()).unwrap_or(u8::MAX);
    let report = GenerationReport {
        generator_version: 1,
        seed,
        selected_candidate,
        candidates_evaluated: CANDIDATE_COUNT,
        valid_candidates: hard_valid_candidates,
        repair_rounds,
        repair_actions,
        used_fallback,
        settings_fingerprint: settings_fingerprint(grid_radius, settings),
        map_fingerprint: map_fingerprint(&map, &special_regions),
        metrics,
        elapsed_micros,
        notes,
    };

    ProceduralBuild {
        map,
        anchors,
        special_regions,
        report,
        validated,
    }
}

/// Assigns deterministic map-local ids to connected gated areas in the final plan.
///
/// Repairs can change surface levels, so exact [`TilePos`] values are deliberately
/// derived only when a candidate is validated or published. Connected components begin
/// at the lowest remaining coordinate, making their numeric ids stable across hash-map
/// iteration order.
fn special_movement_regions(plan: &TerrainPlan) -> SpecialMovementRegions {
    let mut remaining = plan.gated.clone();
    let mut regions = SpecialMovementRegions::new();
    let mut region_id = 0_u32;

    while let Some(start) = remaining.first().copied() {
        let region = SpecialMovementRegion(region_id);
        let mut frontier = VecDeque::from([start]);
        remaining.remove(&start);

        while let Some(coord) = frontier.pop_front() {
            if let Some(cell) = plan.cells.get(&coord) {
                let _previous = regions.insert(TilePos::new(coord, top_surface(*cell)), region);
            }
            for neighbor in coord.neighbors() {
                if remaining.remove(&neighbor) {
                    frontier.push_back(neighbor);
                }
            }
        }

        region_id = region_id.saturating_add(1);
    }

    regions
}

fn construct_plan(
    grid_radius: u32,
    settings: &ProceduralSettings,
    seed: u64,
    candidate: u8,
    fallback: bool,
) -> TerrainPlan {
    match (&settings.landform, &settings.tactical) {
        (LandformSettings::Hills(hills), TacticalSettings::Crossing(crossing)) => {
            hills_crossing_plan(
                grid_radius,
                hills,
                crossing,
                settings.environment,
                seed,
                candidate,
                fallback,
            )
        }
        (LandformSettings::SkyIslands(islands), TacticalSettings::LinkedIslands(linked)) => {
            linked_islands_plan(
                grid_radius,
                islands,
                linked,
                settings.environment,
                seed,
                candidate,
                fallback,
            )
        }
        _ => TerrainPlan {
            grid_radius,
            cells: HashMap::default(),
            anchors: GeneratedAnchors::default(),
            barrier: BTreeSet::new(),
            barrier_centres: Vec::new(),
            barrier_sections: Vec::new(),
            bridge: BTreeSet::new(),
            alternate: BTreeSet::new(),
            crossing_lanes: Vec::new(),
            crossing_rows: Vec::new(),
            sky_bridge_lanes: Vec::new(),
            gated: BTreeSet::new(),
            base_level: 0,
            kind: RecipeKind::HillsCrossing,
        },
    }
}

#[derive(Debug)]
struct RiverGeometry {
    orientation: u8,
    centres: Vec<HexCoord>,
    sections: Vec<[HexCoord; 3]>,
    barrier: BTreeSet<HexCoord>,
    bridge_lanes: (BTreeSet<HexCoord>, BTreeSet<HexCoord>),
    alternate_lanes: (BTreeSet<HexCoord>, BTreeSet<HexCoord>),
    bridge_rows: Vec<[HexCoord; 2]>,
    alternate_rows: Vec<[HexCoord; 2]>,
    bridge_centre: HexCoord,
    alternate_centre: HexCoord,
    party: HexCoord,
    hostile: HexCoord,
}

#[derive(Debug)]
struct FittedCrossing {
    lanes: (BTreeSet<HexCoord>, BTreeSet<HexCoord>),
    rows: Vec<[HexCoord; 2]>,
    centre: HexCoord,
    site_x: i32,
    centre_y: i32,
}

/// One integer cone participating in a coherent hill shape.
#[derive(Debug, Clone, Copy)]
struct HillLobe {
    centre: HexCoord,
    relief: Level,
}

/// A short bent ridge assembled from overlapping semantic lobes.
///
/// Every lobe remains a one-level-per-hex field, so taking their maximum preserves a
/// projectable walker slope while avoiding the concentric contours of one perfect cone.
#[derive(Debug, Clone, Copy)]
struct HillShape {
    lobes: [HillLobe; 4],
}

impl HillShape {
    fn relief_at(self, coord: HexCoord) -> Level {
        self.lobes
            .iter()
            .map(|lobe| {
                let distance = Level::try_from(coord.distance(lobe.centre)).unwrap_or(Level::MAX);
                lobe.relief.saturating_sub(distance)
            })
            .max()
            .unwrap_or(0)
    }
}

fn hills_crossing_plan(
    grid_radius: u32,
    hills: &HillsSettings,
    crossing: &CrossingSettings,
    environment: EnvironmentSettings,
    seed: u64,
    candidate: u8,
    fallback: bool,
) -> TerrainPlan {
    let geometry = river_geometry(grid_radius, crossing, seed, candidate, fallback);
    let domain = HexCoord::ORIGIN.within_radius(grid_radius);
    let hill_shapes = hill_shapes(grid_radius, hills, &geometry, seed, candidate, fallback);
    let protected = protected_hill_cells(&geometry);

    let mut cells = HashMap::default();
    for coord in &domain {
        let mut surface = hills.valley_level;
        if !protected.contains(coord) && !geometry.barrier.contains(coord) {
            for hill in &hill_shapes {
                surface = surface.max(hills.valley_level.saturating_add(hill.relief_at(*coord)));
            }
        }
        surface = surface.min(hills.valley_level.saturating_add(hills.max_relief));

        let role = initial_land_role(surface, hills.valley_level, environment);
        let cell = PlannedCell {
            surface,
            role,
            foundation: Foundation::SolidToBedrock,
            hazard: None,
            overlay: None,
            gated: false,
        };
        cells.insert(*coord, cell);
    }

    let hazard_role = match environment {
        EnvironmentSettings::Volcanic => HazardRole::Lava,
        EnvironmentSettings::TemperateGrassland | EnvironmentSettings::Frozen => HazardRole::Water,
    };
    let channel_role = match environment {
        EnvironmentSettings::Volcanic => SurfaceRole::Basalt,
        EnvironmentSettings::TemperateGrassland | EnvironmentSettings::Frozen => {
            SurfaceRole::Gravel
        }
    };

    for coord in &geometry.barrier {
        if let Some(cell) = cells.get_mut(coord) {
            cell.surface = crossing.bed_level;
            cell.role = channel_role;
            cell.hazard = Some(Hazard {
                bottom: crossing.hazard_bottom,
                top: crossing.hazard_top,
                role: hazard_role,
            });
        }
    }

    let mut bridge = BTreeSet::new();
    let mut bridge_lanes = (BTreeSet::new(), BTreeSet::new());
    for coord in geometry.bridge_lanes.0.union(&geometry.bridge_lanes.1) {
        if let Some(cell) = cells.get_mut(coord) {
            cell.overlay = Some(Overlay {
                level: crossing.bridge_level,
                role: SurfaceRole::Metal,
            });
            bridge.insert(TilePos::new(*coord, crossing.bridge_level));
        }
    }
    bridge_lanes.0.extend(
        geometry
            .bridge_lanes
            .0
            .iter()
            .map(|coord| TilePos::new(*coord, crossing.bridge_level)),
    );
    bridge_lanes.1.extend(
        geometry
            .bridge_lanes
            .1
            .iter()
            .map(|coord| TilePos::new(*coord, crossing.bridge_level)),
    );

    let ford_surface = hills.valley_level.saturating_sub(1);
    let mut alternate = BTreeSet::new();
    let mut alternate_lanes = (BTreeSet::new(), BTreeSet::new());
    for coord in geometry
        .alternate_lanes
        .0
        .union(&geometry.alternate_lanes.1)
    {
        if let Some(cell) = cells.get_mut(coord) {
            cell.surface = ford_surface;
            cell.role = channel_role;
            cell.hazard = None;
            cell.overlay = None;
            alternate.insert(TilePos::new(*coord, ford_surface));
        }
    }
    alternate_lanes.0.extend(
        geometry
            .alternate_lanes
            .0
            .iter()
            .map(|coord| TilePos::new(*coord, ford_surface)),
    );
    alternate_lanes.1.extend(
        geometry
            .alternate_lanes
            .1
            .iter()
            .map(|coord| TilePos::new(*coord, ford_surface)),
    );

    feather_spawn_approaches(&mut cells, &geometry, hills.valley_level);
    lower_only_slope_projection(&mut cells, &geometry.barrier);
    let route_coords: BTreeSet<HexCoord> = bridge
        .iter()
        .chain(alternate.iter())
        .map(|position| position.coord)
        .collect();
    reclassify_hill_cells(
        &mut cells,
        &route_coords,
        hills.valley_level,
        environment,
        seed,
        candidate,
    );

    let party_level = cells
        .get(&geometry.party)
        .map_or(hills.valley_level, |cell| cell.surface);
    let hostile_level = cells
        .get(&geometry.hostile)
        .map_or(hills.valley_level, |cell| cell.surface);
    let mut anchors = GeneratedAnchors::default();
    anchors
        .positions
        .insert(PARTY_START, TilePos::new(geometry.party, party_level));
    anchors
        .positions
        .insert(HOSTILE_START, TilePos::new(geometry.hostile, hostile_level));
    anchors.positions.insert(
        CONFLICT_CENTER,
        TilePos::new(geometry.bridge_centre, crossing.bridge_level),
    );
    anchors.positions.insert(
        BRIDGE,
        TilePos::new(geometry.bridge_centre, crossing.bridge_level),
    );
    anchors.positions.insert(
        ALTERNATE_CROSSING,
        TilePos::new(geometry.alternate_centre, ford_surface),
    );

    TerrainPlan {
        grid_radius,
        cells,
        anchors,
        barrier: geometry.barrier,
        barrier_centres: geometry.centres,
        barrier_sections: geometry.sections,
        bridge,
        alternate,
        crossing_lanes: vec![bridge_lanes, alternate_lanes],
        crossing_rows: vec![
            tile_rows(&geometry.bridge_rows, crossing.bridge_level),
            tile_rows(&geometry.alternate_rows, ford_surface),
        ],
        sky_bridge_lanes: Vec::new(),
        gated: BTreeSet::new(),
        base_level: hills.valley_level,
        kind: RecipeKind::HillsCrossing,
    }
}

fn river_geometry(
    grid_radius: u32,
    _crossing: &CrossingSettings,
    seed: u64,
    candidate: u8,
    fallback: bool,
) -> RiverGeometry {
    let radius = i32::try_from(grid_radius).unwrap_or(i32::MAX);
    let orientation = if fallback {
        0
    } else {
        u8::try_from(named_hash(seed, candidate, "river.orientation", 0) % 3).unwrap_or(0)
    };
    let control_xs = [
        -radius,
        -(2 * radius / 3),
        -(radius / 3),
        0,
        radius / 3,
        2 * radius / 3,
        radius,
    ];
    let jitter = (radius / 6).clamp(1, 3);
    let mut controls = Vec::with_capacity(control_xs.len());
    for (index, x) in control_xs.into_iter().enumerate() {
        let y = if index == 0 || index + 1 == control_xs.len() || fallback {
            0
        } else {
            sample_i32(
                seed,
                candidate,
                "river.meander",
                index as u64,
                -jitter,
                jitter,
            )
        };
        let y = clamp_axial_y(x, y, radius);
        controls.push(rotate_third(HexCoord::from_axial(x, y), orientation));
    }

    let centres = ordered_centerline(&controls).unwrap_or_default();
    let sections = river_ribbon_sections(&centres, grid_radius).unwrap_or_default();
    let barrier = ribbon_barrier(&sections);

    let bridge_x = 0;
    let alternate_x = (radius.saturating_mul(5) / 12).max(5);
    let bridge = fit_crossing_lanes(grid_radius, orientation, &barrier, &centres, bridge_x)
        .unwrap_or_else(|| empty_crossing(orientation, &centres, bridge_x));
    let alternate = fit_crossing_lanes(grid_radius, orientation, &barrier, &centres, alternate_x)
        .unwrap_or_else(|| empty_crossing(orientation, &centres, alternate_x));
    let start_distance = (radius * 2 / 3).max(4);
    let party = rotate_third(
        HexCoord::from_axial(0, clamp_axial_y(0, start_distance, radius)),
        orientation,
    );
    let hostile = rotate_third(
        HexCoord::from_axial(0, clamp_axial_y(0, -start_distance, radius)),
        orientation,
    );

    RiverGeometry {
        orientation,
        centres,
        sections,
        barrier,
        bridge_lanes: bridge.lanes,
        alternate_lanes: alternate.lanes,
        bridge_rows: bridge.rows,
        alternate_rows: alternate.rows,
        bridge_centre: bridge.centre,
        alternate_centre: alternate.centre,
        party,
        hostile,
    }
}

fn ordered_centerline(controls: &[HexCoord]) -> Option<Vec<HexCoord>> {
    let mut centres = Vec::new();
    let mut seen = BTreeSet::new();
    for pair in controls.windows(2) {
        let [start, end] = pair else {
            continue;
        };
        for coord in start.line_between(*end) {
            if centres.last() == Some(&coord) {
                continue;
            }
            if centres
                .last()
                .is_some_and(|previous: &HexCoord| previous.distance(coord) != 1)
                || !seen.insert(coord)
            {
                return None;
            }
            centres.push(coord);
        }
    }
    (centres.len() >= 2).then_some(centres)
}

fn common_edge_neighbors(from: HexCoord, to: HexCoord) -> Option<[HexCoord; 2]> {
    if from.distance(to) != 1 {
        return None;
    }
    let mut common: Vec<HexCoord> = from
        .neighbors()
        .into_iter()
        .filter(|neighbor| neighbor.distance(to) == 1)
        .collect();
    common.sort_unstable();
    let [first, second] = common.as_slice() else {
        return None;
    };
    Some([*first, *second])
}

fn river_ribbon_sections(centres: &[HexCoord], grid_radius: u32) -> Option<Vec<[HexCoord; 3]>> {
    if centres.len() < 2 {
        return None;
    }
    let mut sections = Vec::with_capacity(centres.len());
    for (index, centre) in centres.iter().copied().enumerate() {
        let edge = match centres.get(index.saturating_add(1)).copied() {
            Some(next) => (centre, next),
            None => (centres.get(index.checked_sub(1)?).copied()?, centre),
        };
        let [first, second] = common_edge_neighbors(edge.0, edge.1)?;
        let section = [centre, first, second];
        if section
            .iter()
            .any(|coord| HexCoord::ORIGIN.distance(*coord) > grid_radius)
        {
            return None;
        }
        sections.push(section);
    }
    Some(sections)
}

fn ribbon_barrier(sections: &[[HexCoord; 3]]) -> BTreeSet<HexCoord> {
    sections
        .iter()
        .flat_map(|section| section.iter().copied())
        .collect()
}

fn standard_centre_y(centres: &[HexCoord], orientation: u8, target_x: i32) -> i32 {
    centres
        .iter()
        .copied()
        .map(|coord| unrotate_third(coord, orientation))
        .min_by_key(|coord| (coord.x().abs_diff(target_x), coord.y().abs()))
        .map_or(0, HexCoord::y)
}

fn fit_crossing_lanes(
    grid_radius: u32,
    orientation: u8,
    barrier: &BTreeSet<HexCoord>,
    centres: &[HexCoord],
    target_x: i32,
) -> Option<FittedCrossing> {
    [0, -1, 1, -2, 2]
        .into_iter()
        .filter_map(|offset| {
            fit_crossing_at(
                grid_radius,
                orientation,
                barrier,
                centres,
                target_x.saturating_add(offset),
            )
        })
        .min_by_key(|crossing| {
            (
                crossing.site_x.abs_diff(target_x),
                crossing.rows.len(),
                crossing.centre_y.unsigned_abs(),
                crossing.centre,
            )
        })
}

fn fit_crossing_at(
    grid_radius: u32,
    orientation: u8,
    barrier: &BTreeSet<HexCoord>,
    centres: &[HexCoord],
    site_x: i32,
) -> Option<FittedCrossing> {
    let radius = i32::try_from(grid_radius).ok()?;
    let centre_y = standard_centre_y(centres, orientation, site_x);
    let available: Vec<(i32, [HexCoord; 2])> = (-radius..=radius)
        .filter_map(|y| {
            let first = HexCoord::from_axial(site_x, y);
            let second = HexCoord::from_axial(site_x.saturating_add(1), y);
            if HexCoord::ORIGIN.distance(first) > grid_radius
                || HexCoord::ORIGIN.distance(second) > grid_radius
            {
                return None;
            }
            Some((
                y,
                [
                    rotate_third(first, orientation),
                    rotate_third(second, orientation),
                ],
            ))
        })
        .collect();
    let row_hits = |row: &[HexCoord; 2]| row.iter().any(|coord| barrier.contains(coord));
    let first_hit = available.iter().position(|(_, row)| row_hits(row))?;
    let last_hit = available.iter().rposition(|(_, row)| row_hits(row))?;
    if first_hit == 0 || last_hit.saturating_add(1) >= available.len() {
        return None;
    }
    if available
        .iter()
        .enumerate()
        .any(|(index, (_, row))| (first_hit..=last_hit).contains(&index) != row_hits(row))
    {
        return None;
    }
    let first_lane_hits = available
        .iter()
        .skip(first_hit)
        .take(last_hit.saturating_sub(first_hit).saturating_add(1))
        .any(|(_, row)| {
            let [first, _] = row;
            barrier.contains(first)
        });
    let second_lane_hits = available
        .iter()
        .skip(first_hit)
        .take(last_hit.saturating_sub(first_hit).saturating_add(1))
        .any(|(_, row)| {
            let [_, second] = row;
            barrier.contains(second)
        });
    if !first_lane_hits || !second_lane_hits {
        return None;
    }

    let rows: Vec<[HexCoord; 2]> = available
        .iter()
        .skip(first_hit - 1)
        .take(last_hit.saturating_sub(first_hit).saturating_add(3))
        .map(|(_, row)| *row)
        .collect();
    let centre = available
        .iter()
        .skip(first_hit)
        .take(last_hit.saturating_sub(first_hit).saturating_add(1))
        .filter(|(_, row)| row_hits(row))
        .min_by_key(|(y, row)| (y.abs_diff(centre_y), *row))
        .and_then(|(_, row)| row.iter().copied().find(|coord| barrier.contains(coord)))?;
    let lanes = coordinate_lanes(&rows);
    Some(FittedCrossing {
        lanes,
        rows,
        centre,
        site_x,
        centre_y,
    })
}

fn empty_crossing(orientation: u8, centres: &[HexCoord], site_x: i32) -> FittedCrossing {
    let centre_y = standard_centre_y(centres, orientation, site_x);
    FittedCrossing {
        lanes: (BTreeSet::new(), BTreeSet::new()),
        rows: Vec::new(),
        centre: rotate_third(HexCoord::from_axial(site_x, centre_y), orientation),
        site_x,
        centre_y,
    }
}

fn coordinate_lanes(rows: &[[HexCoord; 2]]) -> (BTreeSet<HexCoord>, BTreeSet<HexCoord>) {
    let mut first = BTreeSet::new();
    let mut second = BTreeSet::new();
    for &[first_coord, second_coord] in rows {
        first.insert(first_coord);
        second.insert(second_coord);
    }
    (first, second)
}

fn tile_rows(rows: &[[HexCoord; 2]], level: Level) -> Vec<[TilePos; 2]> {
    rows.iter()
        .map(|&[first, second]| [TilePos::new(first, level), TilePos::new(second, level)])
        .collect()
}

fn hill_shapes(
    grid_radius: u32,
    hills: &HillsSettings,
    geometry: &RiverGeometry,
    seed: u64,
    candidate: u8,
    fallback: bool,
) -> Vec<HillShape> {
    let radius = i32::try_from(grid_radius).unwrap_or(i32::MAX);
    let mut shapes = Vec::new();
    for bank in [-1, 1] {
        for index in 0..hills.hills_per_bank {
            let serial = u64::from(index) + if bank > 0 { 100 } else { 0 };
            let x = if fallback {
                -radius / 2 + i32::from(index) * (radius / 2).max(1)
            } else {
                sample_i32(
                    seed,
                    candidate,
                    "hills.x",
                    serial,
                    (-radius).saturating_add(3),
                    radius.saturating_sub(3),
                )
            };
            let min_offset = 4;
            let max_offset = (radius - 3).max(min_offset);
            let magnitude = if fallback {
                min_offset + i32::from(index) % (max_offset - min_offset + 1)
            } else {
                sample_i32(
                    seed,
                    candidate,
                    "hills.bank_offset",
                    serial,
                    min_offset,
                    max_offset,
                )
            };
            let y = clamp_axial_y(x, bank * magnitude, radius);
            let standard = HexCoord::from_axial(x, y);
            let centre = rotate_third(standard, geometry.orientation);
            let min_relief = hills.max_relief.saturating_sub(3).max(2);
            let relief = if fallback {
                min_relief.saturating_add(Level::from(index % 3))
            } else {
                sample_level(
                    seed,
                    candidate,
                    "hills.relief",
                    serial,
                    min_relief,
                    hills.max_relief,
                )
            };
            let axis = if fallback {
                let bank_offset = i32::from(bank > 0);
                u8::try_from((i32::from(index) * 2 + bank_offset) % 6).unwrap_or(0)
            } else {
                u8::try_from(named_hash(seed, candidate, "hills.lobe.axis", serial) % 6)
                    .unwrap_or(0)
            };
            let reach = if fallback {
                2
            } else {
                2 + i32::try_from(named_hash(seed, candidate, "hills.lobe.reach", serial) % 2)
                    .unwrap_or(0)
            };
            let bend_clockwise = if fallback {
                index.is_multiple_of(2)
            } else {
                named_hash(seed, candidate, "hills.lobe.bend", serial).is_multiple_of(2)
            };
            let bend = if bend_clockwise { 1 } else { 5 };
            let bent_axis = axis.wrapping_add(bend) % 6;
            let crest = offset_hex(centre, axis, 1);
            let long_lobe = offset_hex(centre, axis, reach);
            let side_lobe = offset_hex(centre, bent_axis, 2);
            shapes.push(HillShape {
                lobes: [
                    HillLobe { centre, relief },
                    HillLobe {
                        centre: crest,
                        relief,
                    },
                    HillLobe {
                        centre: long_lobe,
                        relief: relief.saturating_sub(1),
                    },
                    HillLobe {
                        centre: side_lobe,
                        relief: relief.saturating_sub(2),
                    },
                ],
            });
        }
    }
    shapes
}

fn offset_hex(coord: HexCoord, direction: u8, distance: i32) -> HexCoord {
    let (x_step, y_step) = match direction % 6 {
        0 => (1, 0),
        1 => (1, -1),
        2 => (0, -1),
        3 => (-1, 0),
        4 => (-1, 1),
        _ => (0, 1),
    };
    HexCoord::from_axial(
        coord.x().saturating_add(x_step * distance),
        coord.y().saturating_add(y_step * distance),
    )
}

fn protected_hill_cells(geometry: &RiverGeometry) -> BTreeSet<HexCoord> {
    let mut protected = geometry.barrier.clone();
    for coord in [geometry.bridge_centre, geometry.alternate_centre] {
        protected.extend(coord.within_radius(2));
    }
    protected.insert(geometry.party);
    protected.insert(geometry.hostile);
    protected.extend(
        geometry
            .bridge_lanes
            .0
            .union(&geometry.bridge_lanes.1)
            .copied(),
    );
    protected.extend(
        geometry
            .alternate_lanes
            .0
            .union(&geometry.alternate_lanes.1)
            .copied(),
    );
    protected
}

const fn initial_land_role(
    surface: Level,
    valley: Level,
    environment: EnvironmentSettings,
) -> SurfaceRole {
    match environment {
        EnvironmentSettings::TemperateGrassland => {
            if surface >= valley.saturating_add(6) {
                SurfaceRole::Stone
            } else {
                SurfaceRole::Grass
            }
        }
        EnvironmentSettings::Frozen => SurfaceRole::Snow,
        EnvironmentSettings::Volcanic => SurfaceRole::Basalt,
    }
}

fn feather_spawn_approaches(
    cells: &mut HashMap<HexCoord, PlannedCell>,
    geometry: &RiverGeometry,
    valley_level: Level,
) {
    for spawn in [geometry.party, geometry.hostile] {
        let egress = spawn_egress(spawn, geometry.bridge_centre);
        let bridge_distance = spawn.distance(geometry.bridge_centre);
        for coord in spawn.within_radius(2) {
            let distance = spawn.distance(coord);
            if let Some(cell) = cells.get_mut(&coord) {
                if cell.hazard.is_some() {
                    continue;
                }
                if distance == 0 || egress.contains(&coord) {
                    cell.surface = valley_level;
                } else {
                    let shoulder_level = valley_level.saturating_add(1);
                    cell.surface = cell.surface.min(shoulder_level);
                    if coord.distance(geometry.bridge_centre) > bridge_distance {
                        cell.surface = shoulder_level;
                    }
                }
            }
        }
    }
}

fn spawn_egress(spawn: HexCoord, target: HexCoord) -> BTreeSet<HexCoord> {
    let mut neighbors = spawn.neighbors().to_vec();
    neighbors.sort_by_key(|coord| (coord.distance(target), *coord));
    let Some(first) = neighbors.first().copied() else {
        return BTreeSet::new();
    };
    let second = neighbors
        .iter()
        .copied()
        .filter(|coord| *coord != first && coord.distance(first) == 1)
        .min_by_key(|coord| (coord.distance(target), *coord));

    [Some(first), second].into_iter().flatten().collect()
}

fn lower_only_slope_projection(
    cells: &mut HashMap<HexCoord, PlannedCell>,
    barrier: &BTreeSet<HexCoord>,
) {
    let mut changed = true;
    while changed {
        changed = false;
        let snapshot: Vec<(HexCoord, Level)> = cells
            .iter()
            .filter(|(coord, cell)| {
                !barrier.contains(coord)
                    && cell.hazard.is_none()
                    && cell.foundation != Foundation::None
            })
            .map(|(coord, cell)| (*coord, cell.surface))
            .collect();
        for (coord, surface) in snapshot {
            let lowest_neighbor = coord
                .neighbors()
                .into_iter()
                .filter_map(|neighbor| cells.get(&neighbor))
                .filter(|cell| cell.hazard.is_none() && cell.foundation != Foundation::None)
                .map(|cell| cell.surface)
                .min();
            let Some(lowest) = lowest_neighbor else {
                continue;
            };
            let legal = lowest.saturating_add(1);
            if surface > legal {
                if let Some(cell) = cells.get_mut(&coord) {
                    cell.surface = legal;
                    changed = true;
                }
            }
        }
    }
}

fn linked_islands_plan(
    grid_radius: u32,
    islands: &SkyIslandsSettings,
    linked: &LinkedIslandsSettings,
    environment: EnvironmentSettings,
    seed: u64,
    candidate: u8,
    fallback: bool,
) -> TerrainPlan {
    let radius = i32::try_from(grid_radius).unwrap_or(i32::MAX);
    let orientation = if fallback {
        0
    } else {
        u8::try_from(named_hash(seed, candidate, "islands.orientation", 0) % 3).unwrap_or(0)
    };
    let spacing =
        i32::try_from(islands.island_radius.saturating_mul(2).saturating_add(2)).unwrap_or(8);
    let chain_centres = [
        rotate_third(HexCoord::from_axial(-spacing, 0), orientation),
        HexCoord::ORIGIN,
        rotate_third(HexCoord::from_axial(spacing, 0), orientation),
    ];
    let optional_offset = (radius - 3).max(spacing);
    let optional_centres = [
        rotate_third(HexCoord::from_axial(0, optional_offset), orientation),
        rotate_third(HexCoord::from_axial(0, -optional_offset), orientation),
    ];

    let mut cells = HashMap::default();
    let mut gated = BTreeSet::new();
    for centre in chain_centres {
        add_island(
            &mut cells,
            grid_radius,
            centre,
            islands.island_radius,
            islands.surface_level,
            environment,
            false,
        );
    }
    for (index, centre) in optional_centres.into_iter().enumerate() {
        let surface = islands
            .surface_level
            .saturating_add(3)
            .saturating_add(Level::try_from(index).unwrap_or(0));
        add_island(
            &mut cells,
            grid_radius,
            centre,
            islands.island_radius.saturating_sub(1).max(2),
            surface,
            environment,
            true,
        );
        gated.extend(
            centre
                .within_radius(islands.island_radius.saturating_sub(1).max(2))
                .into_iter()
                .filter(|coord| HexCoord::ORIGIN.distance(*coord) <= grid_radius),
        );
    }

    let bridge_level = islands.surface_level;
    let mut bridge = BTreeSet::new();
    let mut bridge_centres = Vec::new();
    let mut sky_bridge_lanes = Vec::new();
    for pair in chain_centres.windows(2) {
        let Some(start) = pair.first().copied() else {
            continue;
        };
        let Some(end) = pair.get(1).copied() else {
            continue;
        };
        let line = start.line_between(end);
        if let Some(midpoint) = line.get(line.len() / 2).copied() {
            bridge_centres.push(midpoint);
        }
        let line_cells: BTreeSet<HexCoord> = line.iter().copied().collect();
        let lane_direction = line
            .get(line.len() / 2)
            .and_then(|midpoint| {
                midpoint
                    .neighbors()
                    .into_iter()
                    .enumerate()
                    .find(|(_, neighbor)| !line_cells.contains(neighbor))
                    .map(|(index, _)| index)
            })
            .unwrap_or(0);
        let mut first_lane = BTreeSet::new();
        let mut second_lane = BTreeSet::new();
        for coord in line {
            add_sky_bridge_cell(&mut cells, &mut bridge, coord, bridge_level);
            first_lane.insert(TilePos::new(coord, bridge_level));
            if linked.bridge_width == 2 {
                let neighbors = coord.neighbors();
                let lane = neighbors.get(lane_direction).copied().unwrap_or(coord);
                add_sky_bridge_cell(&mut cells, &mut bridge, lane, bridge_level);
                second_lane.insert(TilePos::new(lane, bridge_level));
            }
        }
        sky_bridge_lanes.push((first_lane, second_lane));
    }

    let [party_coord, conflict_coord, hostile_coord] = chain_centres;
    let party_level = cells
        .get(&party_coord)
        .map_or(islands.surface_level, |cell| top_surface(*cell));
    let hostile_level = cells
        .get(&hostile_coord)
        .map_or(islands.surface_level, |cell| top_surface(*cell));
    let conflict_level = cells
        .get(&conflict_coord)
        .map_or(islands.surface_level, |cell| top_surface(*cell));
    let first_bridge = bridge_centres.first().copied().unwrap_or(conflict_coord);
    let second_bridge = bridge_centres.get(1).copied().unwrap_or(conflict_coord);

    let mut anchors = GeneratedAnchors::default();
    anchors
        .positions
        .insert(PARTY_START, TilePos::new(party_coord, party_level));
    anchors
        .positions
        .insert(HOSTILE_START, TilePos::new(hostile_coord, hostile_level));
    anchors.positions.insert(
        CONFLICT_CENTER,
        TilePos::new(conflict_coord, conflict_level),
    );
    anchors
        .positions
        .insert(BRIDGE, TilePos::new(first_bridge, bridge_level));
    anchors.positions.insert(
        ALTERNATE_CROSSING,
        TilePos::new(second_bridge, bridge_level),
    );

    TerrainPlan {
        grid_radius,
        cells,
        anchors,
        barrier: BTreeSet::new(),
        barrier_centres: Vec::new(),
        barrier_sections: Vec::new(),
        bridge,
        alternate: BTreeSet::new(),
        crossing_lanes: Vec::new(),
        crossing_rows: Vec::new(),
        sky_bridge_lanes,
        gated,
        base_level: islands.surface_level,
        kind: RecipeKind::LinkedSkyIslands,
    }
}

fn add_island(
    cells: &mut HashMap<HexCoord, PlannedCell>,
    grid_radius: u32,
    centre: HexCoord,
    radius: u32,
    surface: Level,
    environment: EnvironmentSettings,
    gated: bool,
) {
    for coord in centre.within_radius(radius) {
        if HexCoord::ORIGIN.distance(coord) > grid_radius {
            continue;
        }
        let distance = Level::try_from(centre.distance(coord)).unwrap_or(Level::MAX);
        let local_surface = surface.saturating_sub(distance / 2);
        let thickness = 4_i32.saturating_sub(distance / 2).max(2);
        let role = initial_land_role(local_surface, surface.saturating_sub(2), environment);
        let proposed = PlannedCell {
            surface: local_surface,
            role,
            foundation: Foundation::Floating {
                bottom: local_surface.saturating_sub(thickness),
            },
            hazard: None,
            overlay: None,
            gated,
        };
        cells
            .entry(coord)
            .and_modify(|existing| {
                if proposed.surface > existing.surface {
                    *existing = proposed;
                }
            })
            .or_insert(proposed);
    }
}

fn add_sky_bridge_cell(
    cells: &mut HashMap<HexCoord, PlannedCell>,
    bridge: &mut BTreeSet<TilePos>,
    coord: HexCoord,
    level: Level,
) {
    cells
        .entry(coord)
        .and_modify(|cell| {
            cell.overlay = Some(Overlay {
                level,
                role: SurfaceRole::Metal,
            });
            cell.gated = false;
        })
        .or_insert(PlannedCell {
            surface: 0,
            role: SurfaceRole::Stone,
            foundation: Foundation::None,
            hazard: None,
            overlay: Some(Overlay {
                level,
                role: SurfaceRole::Metal,
            }),
            gated: false,
        });
    bridge.insert(TilePos::new(coord, level));
}

fn voxelize_validate_repair(
    plan: &mut TerrainPlan,
    settings: &ProceduralSettings,
    seed: u64,
    candidate: u8,
    palette: &TerrainPalette,
    walker: TraversalProfile,
    is_solid: &dyn Fn(SubstanceId) -> bool,
) -> (VoxelMap, Validation, Vec<String>) {
    let mut repair_actions = Vec::new();
    let mut round = 0;
    loop {
        let map = voxelize(plan, settings.environment, palette);
        let mut validation = validate_exact(plan, &map, palette, walker, is_solid);
        if validation.valid || round == MAX_REPAIR_ROUNDS {
            return (map, validation, repair_actions);
        }
        let before = plan.clone();
        repair_plan(plan, round);
        synchronize_anchor_levels(plan);
        if plan.kind == RecipeKind::HillsCrossing {
            let route_coords: BTreeSet<HexCoord> = plan
                .bridge
                .iter()
                .chain(plan.alternate.iter())
                .map(|position| position.coord)
                .collect();
            reclassify_hill_cells(
                &mut plan.cells,
                &route_coords,
                plan.base_level,
                settings.environment,
                seed,
                candidate,
            );
        }
        let changed = plan
            .cells
            .iter()
            .filter(|(coord, cell)| before.cells.get(*coord) != Some(*cell))
            .count();
        let maximum_local_repair = plan.cells.len().saturating_div(20).max(12);
        if changed > maximum_local_repair {
            *plan = before;
            validation.notes.push(format!(
                "{} rejected after changing {changed} cells; local limit is \
                 {maximum_local_repair}",
                repair_label(round)
            ));
            return (map, validation, repair_actions);
        }
        if changed > 0 {
            repair_actions.push(format!("{} ({changed} cells)", repair_label(round)));
        }
        round = round.saturating_add(1);
    }
}

const fn repair_label(round: u8) -> &'static str {
    match round {
        0 => "anchor approach adjustment",
        1 => "local slope projection",
        2 => "crossing landing adjustment",
        _ => "protected spawn adjustment",
    }
}

fn repair_plan(plan: &mut TerrainPlan, round: u8) {
    match round {
        0 => {
            let anchor_coords: Vec<HexCoord> =
                plan.anchors.iter().map(|(_, pos)| pos.coord).collect();
            for anchor in anchor_coords {
                let anchor_level = plan
                    .cells
                    .get(&anchor)
                    .map_or(plan.base_level, |cell| top_surface(*cell));
                for coord in anchor.within_radius(2) {
                    if let Some(cell) = plan.cells.get_mut(&coord) {
                        if cell.hazard.is_none() && cell.overlay.is_none() && !cell.gated {
                            cell.surface = cell.surface.clamp(
                                anchor_level.saturating_sub(1),
                                anchor_level.saturating_add(1),
                            );
                        }
                    }
                }
            }
        }
        1 => lower_only_slope_projection(&mut plan.cells, &plan.barrier),
        2 => {
            let crossing_positions: Vec<TilePos> = plan
                .bridge
                .iter()
                .chain(plan.alternate.iter())
                .copied()
                .collect();
            for position in crossing_positions {
                for neighbor in position.coord.neighbors() {
                    if let Some(cell) = plan.cells.get_mut(&neighbor) {
                        if !cell.gated && cell.hazard.is_none() && cell.overlay.is_none() {
                            cell.surface = cell.surface.clamp(
                                position.level.saturating_sub(1),
                                position.level.saturating_add(1),
                            );
                        }
                    }
                }
            }
        }
        _ => {
            let crossing_coords: BTreeSet<HexCoord> = plan
                .bridge
                .iter()
                .chain(plan.alternate.iter())
                .map(|position| position.coord)
                .collect();
            let anchor_coords: Vec<HexCoord> = plan
                .anchors
                .iter()
                .map(|(_, position)| position.coord)
                .collect();
            for anchor in anchor_coords {
                for coord in anchor.within_radius(3) {
                    if let Some(cell) = plan.cells.get_mut(&coord) {
                        if !crossing_coords.contains(&coord)
                            && !cell.gated
                            && cell.hazard.is_none()
                            && cell.overlay.is_none()
                        {
                            cell.surface = cell
                                .surface
                                .clamp(plan.base_level, plan.base_level.saturating_add(1));
                        }
                    }
                }
            }
        }
    }
}

fn reclassify_hill_cells(
    cells: &mut HashMap<HexCoord, PlannedCell>,
    route_cells: &BTreeSet<HexCoord>,
    valley_level: Level,
    environment: EnvironmentSettings,
    seed: u64,
    candidate: u8,
) {
    let mut near_hazard: [HashSet<HexCoord>; 2] = [HashSet::default(), HashSet::default()];
    for hazard_coord in cells
        .iter()
        .filter_map(|(coord, cell)| cell.hazard.is_some().then_some(*coord))
    {
        for nearby in hazard_coord.within_radius(2) {
            let distance = hazard_coord.distance(nearby);
            if distance <= 1 {
                near_hazard[0].insert(nearby);
            }
            near_hazard[1].insert(nearby);
        }
    }

    let mut classifications: Vec<(HexCoord, SurfaceRole)> = cells
        .iter()
        .filter(|(_, cell)| cell.hazard.is_none())
        .map(|(coord, cell)| {
            let hazard_distance = if near_hazard[0].contains(coord) {
                Some(1)
            } else if near_hazard[1].contains(coord) {
                Some(2)
            } else {
                None
            };
            let role = classify_hill_surface(
                *coord,
                *cell,
                cells,
                route_cells.contains(coord),
                hazard_distance,
                valley_level,
                environment,
                seed,
                candidate,
            );
            (*coord, role)
        })
        .collect();
    if environment == EnvironmentSettings::TemperateGrassland {
        suppress_small_temperate_exposures(&mut classifications);
    }
    if environment == EnvironmentSettings::Frozen
        && !classifications
            .iter()
            .any(|(_, role)| *role == SurfaceRole::Ice)
    {
        if let Some((_, role)) = classifications
            .iter_mut()
            .filter(|(coord, _)| !route_cells.contains(coord))
            .min_by_key(|(coord, _)| {
                (
                    cells.get(coord).map_or(Level::MAX, |cell| cell.surface),
                    named_hash(
                        seed,
                        candidate,
                        "materials.required_ice",
                        coord_serial(*coord),
                    ),
                )
            })
        {
            *role = SurfaceRole::Ice;
        }
    }
    for (coord, role) in classifications {
        if let Some(cell) = cells.get_mut(&coord) {
            cell.role = role;
        }
    }
}

fn suppress_small_temperate_exposures(classifications: &mut [(HexCoord, SurfaceRole)]) {
    for (source, replacement) in [
        (SurfaceRole::Stone, SurfaceRole::Dirt),
        (SurfaceRole::Dirt, SurfaceRole::Grass),
    ] {
        let mut remaining: BTreeSet<HexCoord> = classifications
            .iter()
            .filter_map(|(coord, role)| (*role == source).then_some(*coord))
            .collect();
        let mut small_components = BTreeSet::new();
        while let Some(start) = remaining.first().copied() {
            remaining.remove(&start);
            let mut component = Vec::from([start]);
            let mut frontier = VecDeque::from([start]);
            while let Some(coord) = frontier.pop_front() {
                for neighbor in coord.neighbors() {
                    if remaining.remove(&neighbor) {
                        component.push(neighbor);
                        frontier.push_back(neighbor);
                    }
                }
            }
            if component.len() < 3 {
                small_components.extend(component);
            }
        }
        for (coord, role) in classifications.iter_mut() {
            if small_components.contains(coord) {
                *role = replacement;
            }
        }
    }
}

fn synchronize_anchor_levels(plan: &mut TerrainPlan) {
    for position in plan.anchors.positions.values_mut() {
        if let Some(cell) = plan.cells.get(&position.coord) {
            position.level = top_surface(*cell);
        }
    }
}

fn classify_hill_surface(
    coord: HexCoord,
    cell: PlannedCell,
    cells: &HashMap<HexCoord, PlannedCell>,
    route_used: bool,
    hazard_distance: Option<u32>,
    valley_level: Level,
    environment: EnvironmentSettings,
    seed: u64,
    candidate: u8,
) -> SurfaceRole {
    let mut maximum_slope = 0;
    let mut lower_neighbors = 0;
    for neighbor_coord in coord.neighbors() {
        let Some(neighbor) = cells.get(&neighbor_coord) else {
            continue;
        };
        if neighbor.hazard.is_some() {
            continue;
        }
        maximum_slope = maximum_slope.max(neighbor.surface.abs_diff(cell.surface));
        lower_neighbors += usize::from(neighbor.surface < cell.surface);
    }
    let height_above_valley = cell.surface.saturating_sub(valley_level);
    match environment {
        EnvironmentSettings::TemperateGrassland => {
            let stone_boundary =
                coherent_material_roll(seed, candidate, "materials.stone_boundary", coord);
            let dirt_patch = coherent_material_roll(seed, candidate, "materials.dirt_patch", coord);
            let gravel_boundary =
                coherent_material_roll(seed, candidate, "materials.gravel_boundary", coord);
            if route_used {
                SurfaceRole::Gravel
            } else if height_above_valley >= 6
                || (height_above_valley == 5 && lower_neighbors >= 3 && stone_boundary < 62)
            {
                SurfaceRole::Stone
            } else if height_above_valley >= 2
                && maximum_slope > 0
                && lower_neighbors >= 2
                && dirt_patch < 38
            {
                SurfaceRole::Dirt
            } else if height_above_valley <= 1
                && (hazard_distance == Some(1)
                    || (hazard_distance == Some(2) && gravel_boundary < 35))
            {
                SurfaceRole::Gravel
            } else {
                SurfaceRole::Grass
            }
        }
        EnvironmentSettings::Frozen => {
            let ice_boundary =
                coherent_material_roll(seed, candidate, "materials.ice_boundary", coord);
            let patch_coord =
                HexCoord::from_axial(coord.x().div_euclid(3), coord.y().div_euclid(3));
            let lowland_ice_patch = height_above_valley <= 2
                && named_hash(
                    seed,
                    candidate,
                    "materials.ice_patch",
                    coord_serial(patch_coord),
                )
                .is_multiple_of(5);
            if route_used {
                SurfaceRole::Gravel
            } else if maximum_slope == 0
                && (hazard_distance == Some(1)
                    || (hazard_distance == Some(2) && ice_boundary < 50)
                    || lowland_ice_patch)
            {
                SurfaceRole::Ice
            } else {
                SurfaceRole::Snow
            }
        }
        EnvironmentSettings::Volcanic => SurfaceRole::Basalt,
    }
}

fn coherent_material_roll(seed: u64, candidate: u8, stream: &str, coord: HexCoord) -> u64 {
    let coarse = HexCoord::from_axial(coord.x().div_euclid(3), coord.y().div_euclid(3));
    let shifted = HexCoord::from_axial(
        coord.x().saturating_add(1).div_euclid(4),
        coord.y().saturating_sub(1).div_euclid(4),
    );
    let first = named_hash(seed, candidate, stream, coord_serial(coarse)) % 100;
    let second = named_hash(
        seed,
        candidate,
        stream,
        coord_serial(shifted).wrapping_add(0x9E37_79B9_7F4A_7C15),
    ) % 100;
    first.saturating_mul(2).saturating_add(second) / 3
}

fn voxelize(
    plan: &TerrainPlan,
    environment: EnvironmentSettings,
    palette: &TerrainPalette,
) -> VoxelMap {
    let mut map = VoxelMap::new();
    for (coord, cell) in &plan.cells {
        let mut column = match cell.foundation {
            Foundation::SolidToBedrock => solid_column(*cell, environment, palette),
            Foundation::Floating { bottom } => floating_column(*cell, bottom, environment, palette),
            Foundation::None => Column::new(),
        };

        if let Some(hazard) = cell.hazard {
            let substance = match hazard.role {
                HazardRole::Water => palette.water,
                HazardRole::Lava => palette.lava,
            };
            for level in hazard.bottom..=hazard.top {
                column.set(level, substance);
            }
        }
        if let Some(overlay) = cell.overlay {
            column.set(overlay.level, substance_for_surface(overlay.role, palette));
        }
        map.insert_column(*coord, column);
    }
    map
}

fn solid_column(
    cell: PlannedCell,
    environment: EnvironmentSettings,
    palette: &TerrainPalette,
) -> Column {
    let mut column = Column::new();
    for level in 0..=cell.surface {
        let substance = if level == 0 {
            palette.bedrock
        } else if level == cell.surface {
            substance_for_surface(cell.role, palette)
        } else if level >= cell.surface.saturating_sub(TOPSOIL_LEVELS)
            && environment != EnvironmentSettings::Volcanic
        {
            palette.dirt
        } else if environment == EnvironmentSettings::Volcanic {
            palette.basalt
        } else {
            palette.stone
        };
        column.set(level, substance);
    }
    column
}

fn floating_column(
    cell: PlannedCell,
    bottom: Level,
    environment: EnvironmentSettings,
    palette: &TerrainPalette,
) -> Column {
    let mut column = Column::new();
    for level in bottom..=cell.surface {
        let substance = if level == cell.surface {
            substance_for_surface(cell.role, palette)
        } else if level >= cell.surface.saturating_sub(2)
            && environment != EnvironmentSettings::Volcanic
        {
            palette.dirt
        } else if environment == EnvironmentSettings::Volcanic {
            palette.basalt
        } else {
            palette.stone
        };
        column.set(level, substance);
    }
    column
}

const fn substance_for_surface(role: SurfaceRole, palette: &TerrainPalette) -> SubstanceId {
    match role {
        SurfaceRole::Grass => palette.grass,
        SurfaceRole::Gravel => palette.gravel,
        SurfaceRole::Dirt => palette.dirt,
        SurfaceRole::Stone => palette.stone,
        SurfaceRole::Snow => palette.snow,
        SurfaceRole::Ice => palette.ice,
        SurfaceRole::Basalt => palette.basalt,
        SurfaceRole::Metal => palette.metal,
    }
}

fn validate_exact(
    plan: &TerrainPlan,
    map: &VoxelMap,
    palette: &TerrainPalette,
    walker: TraversalProfile,
    is_solid: &dyn Fn(SubstanceId) -> bool,
) -> Validation {
    let graph = traversal_graph(map, walker, is_solid);
    let special_regions = special_movement_regions(plan);
    let mut notes = Vec::new();

    if plan
        .cells
        .keys()
        .any(|coord| HexCoord::ORIGIN.distance(*coord) > plan.grid_radius)
    {
        notes.push("terrain plan contains a coordinate outside grid_radius".to_owned());
    }

    for required in [
        PARTY_START,
        HOSTILE_START,
        CONFLICT_CENTER,
        BRIDGE,
        ALTERNATE_CROSSING,
    ] {
        if plan.anchors.get(required).is_none() {
            notes.push(format!("required map anchor \"{required}\" is missing"));
        }
    }
    for (name, anchor) in plan.anchors.iter() {
        if !graph.surfaces.contains(&anchor) {
            notes.push(format!("{name} anchor is not standable at {anchor:?}"));
        }
        if special_regions.get(anchor).is_some() {
            notes.push(format!(
                "{name} anchor belongs to a special-movement region at {anchor:?}"
            ));
        }
    }

    let party = plan.anchors.get(PARTY_START);
    let hostile = plan.anchors.get(HOSTILE_START);
    let distances = party.map_or_else(HashMap::default, |start| {
        traversal_distances(start, &graph, &BTreeSet::new())
    });
    if hostile.is_some_and(|target| !distances.contains_key(&target)) {
        notes.push("opposing anchors are disconnected for the ordinary walker".to_owned());
    }

    if let Some(surface) = graph
        .surfaces
        .iter()
        .filter(|surface| {
            special_regions.get(**surface).is_none() && !distances.contains_key(*surface)
        })
        .min()
    {
        notes.push(format!(
            "ordinary surface {surface:?} is outside the critical network"
        ));
    }
    if special_regions.len() != plan.gated.len() {
        notes.push("a gated coordinate has no planned surface".to_owned());
    }
    let mut memberships: Vec<(TilePos, SpecialMovementRegion)> = special_regions.iter().collect();
    memberships.sort_unstable();
    for (surface, _) in memberships {
        if !graph.surfaces.contains(&surface) {
            notes.push(format!(
                "special-movement surface {surface:?} is not standable"
            ));
            break;
        }
        if distances.contains_key(&surface) {
            notes.push(format!(
                "special-movement surface {surface:?} is walker-reachable"
            ));
            break;
        }
    }

    let alternate_detour_percent = if plan.kind == RecipeKind::HillsCrossing {
        validate_hills_columns(plan, map, palette, &mut notes, is_solid);
        validate_crossing_topology(plan, &graph, party, hostile, &mut notes)
    } else {
        validate_sky_bridges(plan, &graph, &mut notes, walker);
        0
    };

    let route_steps = hostile
        .and_then(|target| distances.get(&target).copied())
        .unwrap_or(u32::MAX);
    let party_height = party.map_or(plan.base_level, |pos| pos.level);
    let hostile_height = hostile.map_or(plan.base_level, |pos| pos.level);
    let (party_high_ground, hostile_high_ground) = match (party, hostile) {
        (Some(party_anchor), Some(hostile_anchor)) => (
            reachable_bank_highest(party_anchor, hostile_anchor, &graph, &distances),
            reachable_bank_highest(hostile_anchor, party_anchor, &graph, &distances),
        ),
        _ => (plan.base_level, plan.base_level),
    };
    let reachable_elevation_levels = distances
        .keys()
        .map(|position| position.level)
        .collect::<BTreeSet<_>>();
    let maximum = plan
        .cells
        .values()
        .filter(|cell| !cell.gated && cell.hazard.is_none())
        .map(|cell| top_surface(*cell))
        .max()
        .unwrap_or(plan.base_level);
    let metrics = TacticalMetrics {
        relief: maximum.saturating_sub(plan.base_level),
        barrier_cells: u32::try_from(plan.barrier.len()).unwrap_or(u32::MAX),
        critical_route_steps: route_steps,
        spawn_height_difference: party_height.abs_diff(hostile_height) as Level,
        bank_high_ground_difference: party_high_ground.abs_diff(hostile_high_ground) as Level,
        reachable_surfaces: u32::try_from(distances.len()).unwrap_or(u32::MAX),
        reachable_elevation_levels: u32::try_from(reachable_elevation_levels.len())
            .unwrap_or(u32::MAX),
        alternate_detour_percent,
        river_sinuosity_percent: river_sinuosity_percent(plan),
        environment_signature_percent: environment_signature_percent(plan),
    };

    Validation {
        valid: notes.is_empty(),
        notes,
        metrics,
    }
}

fn reachable_bank_highest(
    anchor: TilePos,
    opposing_anchor: TilePos,
    graph: &TraversalGraph,
    distances: &HashMap<TilePos, u32>,
) -> Level {
    graph
        .surfaces
        .iter()
        .filter(|position| {
            position.coord.distance(anchor.coord) <= position.coord.distance(opposing_anchor.coord)
                && distances.contains_key(*position)
        })
        .map(|position| position.level)
        .max()
        .unwrap_or(anchor.level)
}

fn river_sinuosity_percent(plan: &TerrainPlan) -> u32 {
    let (Some(first), Some(last)) = (
        plan.barrier_centres.first().copied(),
        plan.barrier_centres.last().copied(),
    ) else {
        return 0;
    };
    let direct_steps = first.distance(last);
    let centreline_steps = plan
        .barrier_centres
        .windows(2)
        .map(|pair| match pair {
            [from, to] => (*from).distance(*to),
            _ => 0,
        })
        .fold(0_u32, u32::saturating_add);
    if direct_steps == 0 || centreline_steps <= direct_steps {
        return 0;
    }
    centreline_steps
        .saturating_sub(direct_steps)
        .saturating_mul(100)
        .checked_div(direct_steps)
        .unwrap_or(0)
}

fn environment_signature_percent(plan: &TerrainPlan) -> u32 {
    let frozen = plan
        .cells
        .values()
        .any(|cell| matches!(cell.role, SurfaceRole::Snow | SurfaceRole::Ice));
    let volcanic = plan.cells.values().any(|cell| {
        cell.role == SurfaceRole::Basalt
            || cell
                .hazard
                .is_some_and(|hazard| hazard.role == HazardRole::Lava)
    });
    let (signature_cells, eligible_cells) =
        plan.cells
            .values()
            .fold((0_u32, 0_u32), |(signature, eligible), cell| {
                let eligible_cell = if volcanic {
                    true
                } else {
                    cell.hazard.is_none() && cell.foundation != Foundation::None
                };
                if !eligible_cell {
                    return (signature, eligible);
                }
                let signature_cell = if frozen {
                    cell.role == SurfaceRole::Ice
                } else if volcanic {
                    cell.hazard
                        .is_some_and(|hazard| hazard.role == HazardRole::Lava)
                } else {
                    cell.role == SurfaceRole::Gravel
                };
                (
                    signature.saturating_add(u32::from(signature_cell)),
                    eligible.saturating_add(1),
                )
            });
    signature_cells
        .saturating_mul(100)
        .checked_div(eligible_cells)
        .unwrap_or(0)
}

fn validate_crossing_topology(
    plan: &TerrainPlan,
    graph: &TraversalGraph,
    party: Option<TilePos>,
    hostile: Option<TilePos>,
    notes: &mut Vec<String>,
) -> u32 {
    validate_barrier_ribbon(plan, notes);
    if !coords_connected(&plan.barrier) {
        notes.push("semantic hazard barrier is not connected".to_owned());
    }
    let mut lane_pairs = plan.crossing_lanes.iter();
    let bridge_lanes = lane_pairs.next();
    let alternate_lanes = lane_pairs.next();
    if lane_pairs.next().is_some() || bridge_lanes.is_none() || alternate_lanes.is_none() {
        notes.push("crossing recipe must publish exactly two two-lane crossings".to_owned());
    }
    let mut crossing_rows = plan.crossing_rows.iter();
    let bridge_rows = crossing_rows.next();
    let alternate_rows = crossing_rows.next();
    if crossing_rows.next().is_some() || bridge_rows.is_none() || alternate_rows.is_none() {
        notes.push("crossing recipe must publish exactly two fitted row sets".to_owned());
    }
    if let (Some((first, second)), Some(rows)) = (bridge_lanes, bridge_rows) {
        validate_fitted_crossing(
            "bridge",
            rows,
            first,
            second,
            &plan.bridge,
            &plan.barrier,
            &graph.surfaces,
            plan.grid_radius,
            notes,
        );
        let declared: BTreeSet<TilePos> = first.union(second).copied().collect();
        if declared != plan.bridge {
            notes.push("bridge lane geometry does not match declared bridge surfaces".to_owned());
        }
    }
    if let (Some((first, second)), Some(rows)) = (alternate_lanes, alternate_rows) {
        validate_fitted_crossing(
            "alternate crossing",
            rows,
            first,
            second,
            &plan.alternate,
            &plan.barrier,
            &graph.surfaces,
            plan.grid_radius,
            notes,
        );
        let declared: BTreeSet<TilePos> = first.union(second).copied().collect();
        if declared != plan.alternate {
            notes.push(
                "alternate lane geometry does not match declared crossing surfaces".to_owned(),
            );
        }
    }
    for position in &plan.bridge {
        if !graph.surfaces.contains(position) {
            notes.push(format!("bridge surface {position:?} is not standable"));
            break;
        }
    }
    for (name, anchor, crossing) in [
        (BRIDGE, plan.anchors.get(BRIDGE), &plan.bridge),
        (
            CONFLICT_CENTER,
            plan.anchors.get(CONFLICT_CENTER),
            &plan.bridge,
        ),
        (
            ALTERNATE_CROSSING,
            plan.anchors.get(ALTERNATE_CROSSING),
            &plan.alternate,
        ),
    ] {
        if anchor.is_some_and(|position| {
            !crossing.contains(&position) || !plan.barrier.contains(&position.coord)
        }) {
            notes.push(format!(
                "{name} anchor is not on a barrier-intersecting crossing row"
            ));
        }
    }
    for position in &plan.alternate {
        if !graph.surfaces.contains(position) {
            notes.push(format!(
                "alternate crossing surface {position:?} is not standable"
            ));
            break;
        }
    }

    let Some(start) = party else {
        return 0;
    };
    let Some(target) = hostile else {
        return 0;
    };
    let alternate_steps = traversal_distance_to(start, target, graph, &plan.bridge);
    if alternate_steps.is_none() {
        notes.push("alternate crossing does not independently connect the banks".to_owned());
    }
    let bridge_steps = traversal_distance_to(start, target, graph, &plan.alternate);
    if bridge_steps.is_none() {
        notes.push("bridge does not independently connect the banks".to_owned());
    }
    let removed_both: BTreeSet<TilePos> = plan.bridge.union(&plan.alternate).copied().collect();
    if traversal_distance_to(start, target, graph, &removed_both).is_some() {
        notes.push("banks remain connected after both declared crossings are removed".to_owned());
    }

    let bridge_coords: BTreeSet<HexCoord> =
        plan.bridge.iter().map(|position| position.coord).collect();
    let alternate_coords: BTreeSet<HexCoord> = plan
        .alternate
        .iter()
        .map(|position| position.coord)
        .collect();
    let separation = bridge_coords
        .iter()
        .flat_map(|bridge| {
            alternate_coords
                .iter()
                .map(move |alternate| bridge.distance(*alternate))
        })
        .min()
        .unwrap_or(0);
    if separation < 3 {
        notes.push("bridge and alternate crossing are not meaningfully separated".to_owned());
    }

    let detour_percent = match (bridge_steps, alternate_steps) {
        (Some(direct), Some(alternate)) if direct > 0 && alternate >= direct => alternate
            .saturating_sub(direct)
            .saturating_mul(100)
            .checked_div(direct)
            .unwrap_or(0),
        _ => 0,
    };
    if !(20..=60).contains(&detour_percent) {
        notes.push(format!(
            "alternate route detour is {detour_percent}%; expected 20% through 60%"
        ));
    }
    detour_percent
}

fn validate_barrier_ribbon(plan: &TerrainPlan, notes: &mut Vec<String>) {
    let unique_centres: BTreeSet<HexCoord> = plan.barrier_centres.iter().copied().collect();
    if unique_centres.len() != plan.barrier_centres.len() {
        notes.push("river centreline repeats a coordinate".to_owned());
    }
    if plan
        .barrier_centres
        .windows(2)
        .any(|pair| matches!(pair, [from, to] if from.distance(*to) != 1))
    {
        notes.push("river centreline contains a non-adjacent step".to_owned());
    }
    let endpoints_span_boundaries = plan
        .barrier_centres
        .first()
        .zip(plan.barrier_centres.last())
        .is_some_and(|(first, last)| {
            HexCoord::ORIGIN.distance(*first) == plan.grid_radius
                && HexCoord::ORIGIN.distance(*last) == plan.grid_radius
                && first.distance(*last) >= plan.grid_radius.saturating_mul(2).saturating_sub(2)
        });
    if !endpoints_span_boundaries {
        notes.push("river centreline does not span opposing map boundaries".to_owned());
    }

    let Some(expected_sections) = river_ribbon_sections(&plan.barrier_centres, plan.grid_radius)
    else {
        notes.push("river centreline cannot form exact three-cell sections".to_owned());
        return;
    };
    if plan.barrier_sections != expected_sections {
        notes.push("stored river sections do not match the ordered centreline".to_owned());
    }
    if plan
        .barrier_sections
        .iter()
        .any(|section| section.iter().collect::<BTreeSet<_>>().len() != 3)
    {
        notes.push("a river section is not exactly three distinct cells wide".to_owned());
    }
    if ribbon_barrier(&plan.barrier_sections) != plan.barrier {
        notes.push("semantic hazard barrier is not exactly the union of its sections".to_owned());
    }
}

fn validate_hills_columns(
    plan: &TerrainPlan,
    map: &VoxelMap,
    palette: &TerrainPalette,
    notes: &mut Vec<String>,
    is_solid: &dyn Fn(SubstanceId) -> bool,
) {
    let radius = u64::from(plan.grid_radius);
    let expected = 3_u64
        .saturating_mul(radius)
        .saturating_mul(radius)
        .saturating_add(3_u64.saturating_mul(radius))
        .saturating_add(1);
    if u64::try_from(map.len()).unwrap_or(u64::MAX) != expected {
        notes.push(format!(
            "Hills map has {} columns; expected {expected}",
            map.len()
        ));
    }

    let volcanic = plan.cells.values().any(|cell| {
        cell.role == SurfaceRole::Basalt
            || cell
                .hazard
                .is_some_and(|hazard| hazard.role == HazardRole::Lava)
    });
    let frozen = plan
        .cells
        .values()
        .any(|cell| matches!(cell.role, SurfaceRole::Snow | SurfaceRole::Ice));
    if frozen
        && !plan
            .cells
            .values()
            .any(|cell| cell.role == SurfaceRole::Ice)
    {
        notes.push("Frozen hills contain no ice surface".to_owned());
    }
    let mut planned_cells: Vec<(&HexCoord, &PlannedCell)> = plan.cells.iter().collect();
    planned_cells.sort_unstable_by_key(|(coord, _)| **coord);
    for (coord, cell) in planned_cells {
        if cell.foundation != Foundation::SolidToBedrock {
            notes.push(format!("Hills coordinate {coord:?} is not bedrock-founded"));
            break;
        }
        let Some(column) = map.column(*coord) else {
            notes.push(format!("Hills coordinate {coord:?} has no voxel column"));
            break;
        };
        if column.iter().next() != Some(palette.bedrock) {
            notes.push(format!(
                "Hills coordinate {coord:?} is missing bedrock at level 0"
            ));
            break;
        }
        let expected_surface = substance_for_surface(cell.role, palette);
        if column.get(cell.surface) != expected_surface {
            notes.push(format!(
                "Hills coordinate {coord:?} has the wrong surface material at level {}",
                cell.surface
            ));
            break;
        }
        let surface_index = usize::try_from(cell.surface).unwrap_or(usize::MAX);
        let topsoil_start =
            usize::try_from(cell.surface.saturating_sub(TOPSOIL_LEVELS)).unwrap_or(usize::MAX);
        let mut wrong_stratum = None;
        for (index, actual) in column.iter().enumerate().take(surface_index).skip(1) {
            let expected_substance = if volcanic {
                palette.basalt
            } else if index >= topsoil_start {
                palette.dirt
            } else {
                palette.stone
            };
            if actual != expected_substance {
                wrong_stratum = Some((index, expected_substance, actual));
                break;
            }
        }
        if let Some((index, expected_substance, actual)) = wrong_stratum {
            notes.push(format!(
                "Hills coordinate {coord:?} has substance {actual:?} at level {index}; \
                 expected {expected_substance:?}"
            ));
            break;
        }
    }

    for position in &plan.bridge {
        if map.get(*position) != palette.metal {
            notes.push(format!("bridge position {position:?} is not a metal voxel"));
            break;
        }
    }
    for position in &plan.alternate {
        let cell_has_hazard = plan
            .cells
            .get(&position.coord)
            .is_some_and(|cell| cell.hazard.is_some());
        let substance = map.get(*position);
        let correct_material = substance == palette.gravel || substance == palette.basalt;
        if cell_has_hazard || !is_solid(substance) || !correct_material {
            notes.push(format!(
                "alternate crossing {position:?} is not dry gravel or basalt"
            ));
            break;
        }
    }

    let alternate_coords: BTreeSet<HexCoord> = plan
        .alternate
        .iter()
        .map(|position| position.coord)
        .collect();
    for coord in &plan.barrier {
        let Some(cell) = plan.cells.get(coord) else {
            notes.push(format!("barrier coordinate {coord:?} has no planned cell"));
            break;
        };
        if alternate_coords.contains(coord) {
            if cell.hazard.is_some() {
                notes.push(format!(
                    "dry alternate crossing retains a hazard at {coord:?}"
                ));
                break;
            }
            continue;
        }
        let Some(hazard) = cell.hazard else {
            notes.push(format!("barrier coordinate {coord:?} has no hazard"));
            break;
        };
        let expected = match hazard.role {
            HazardRole::Water => palette.water,
            HazardRole::Lava => palette.lava,
        };
        if (hazard.bottom..=hazard.top)
            .any(|level| map.get(TilePos::new(*coord, level)) != expected)
        {
            notes.push(format!(
                "barrier coordinate {coord:?} does not contain the expected hazard through \
                 levels {}-{}",
                hazard.bottom, hazard.top
            ));
            break;
        }
    }

    for anchor_name in [PARTY_START, HOSTILE_START] {
        let Some(anchor) = plan.anchors.get(anchor_name) else {
            continue;
        };
        let dominant_perch = anchor.coord.within_radius(2).into_iter().any(|coord| {
            plan.cells.get(&coord).is_some_and(|cell| {
                !cell.gated
                    && cell.hazard.is_none()
                    && top_surface(*cell) > anchor.level.saturating_add(1)
            })
        });
        if dominant_perch {
            notes.push(format!(
                "{anchor_name} has dominant high ground inside its protected spawn zone"
            ));
        }
    }
}

fn coords_connected(coords: &BTreeSet<HexCoord>) -> bool {
    let Some(start) = coords.first().copied() else {
        return false;
    };
    let mut visited = BTreeSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(current) = queue.pop_front() {
        for neighbor in current.neighbors() {
            if coords.contains(&neighbor) && visited.insert(neighbor) {
                queue.push_back(neighbor);
            }
        }
    }
    visited.len() == coords.len()
}

fn validate_sky_bridges(
    plan: &TerrainPlan,
    graph: &TraversalGraph,
    notes: &mut Vec<String>,
    walker: TraversalProfile,
) {
    if plan.sky_bridge_lanes.len() != 2 {
        notes.push("sky-island critical chain must contain exactly two bridges".to_owned());
        return;
    }

    for (index, (first, second)) in plan.sky_bridge_lanes.iter().enumerate() {
        validate_parallel_lanes(
            &format!("sky bridge {index}"),
            first,
            second,
            &graph.surfaces,
            notes,
        );

        for lane in [first, second] {
            let Some(start) = lane.first().copied() else {
                continue;
            };
            let lane_graph =
                graph_from_surfaces(lane.iter().copied().collect::<HashSet<_>>(), walker);
            if traversal_distances(start, &lane_graph, &BTreeSet::new()).len() != lane.len() {
                notes.push(format!(
                    "sky bridge {index} lane is not connected under walker rules"
                ));
            }
        }
    }
}

fn validate_parallel_lanes(
    label: &str,
    first: &BTreeSet<TilePos>,
    second: &BTreeSet<TilePos>,
    surfaces: &HashSet<TilePos>,
    notes: &mut Vec<String>,
) {
    if first.is_empty() || first.len() != second.len() || !first.is_disjoint(second) {
        notes.push(format!(
            "{label} does not contain two distinct equal-length lanes"
        ));
        return;
    }
    if first
        .iter()
        .chain(second.iter())
        .any(|position| !surfaces.contains(position))
    {
        notes.push(format!("{label} contains a non-standable lane surface"));
        return;
    }
    let first_coords: BTreeSet<HexCoord> = first.iter().map(|position| position.coord).collect();
    let second_coords: BTreeSet<HexCoord> = second.iter().map(|position| position.coord).collect();
    if !coords_connected(&first_coords) || !coords_connected(&second_coords) {
        notes.push(format!("{label} has a disconnected lane"));
    }
    let lanes_are_adjacent = first.iter().all(|position| {
        second
            .iter()
            .any(|other| position.level == other.level && position.coord.distance(other.coord) == 1)
    }) && second.iter().all(|position| {
        first
            .iter()
            .any(|other| position.level == other.level && position.coord.distance(other.coord) == 1)
    });
    if !lanes_are_adjacent {
        notes.push(format!("{label} lanes are not consistently one cell apart"));
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "crossing validation compares the semantic rows, both published lanes, \
              the declared surface set, the barrier, and the exact traversal graph"
)]
fn validate_fitted_crossing(
    label: &str,
    rows: &[[TilePos; 2]],
    first: &BTreeSet<TilePos>,
    second: &BTreeSet<TilePos>,
    declared: &BTreeSet<TilePos>,
    barrier: &BTreeSet<HexCoord>,
    surfaces: &HashSet<TilePos>,
    grid_radius: u32,
    notes: &mut Vec<String>,
) {
    validate_parallel_lanes(label, first, second, surfaces, notes);
    if rows.len() < 3 {
        notes.push(format!(
            "{label} has too few rows to cross the barrier with two landings"
        ));
        return;
    }

    let mut rows_first = BTreeSet::new();
    let mut rows_second = BTreeSet::new();
    for &[first_position, second_position] in rows {
        rows_first.insert(first_position);
        rows_second.insert(second_position);
        if first_position.level != second_position.level
            || first_position.coord.distance(second_position.coord) != 1
        {
            notes.push(format!("{label} contains a malformed paired row"));
            break;
        }
    }
    if &rows_first != first || &rows_second != second {
        notes.push(format!(
            "{label} fitted rows do not match its declared lanes"
        ));
    }
    if rows_first
        .union(&rows_second)
        .copied()
        .collect::<BTreeSet<_>>()
        != *declared
    {
        notes.push(format!(
            "{label} fitted rows do not match its declared crossing surfaces"
        ));
    }
    if rows.windows(2).any(|pair| {
        let [from, to] = pair else {
            return true;
        };
        let [from_first, from_second] = *from;
        let [to_first, to_second] = *to;
        from_first.level != to_first.level
            || from_second.level != to_second.level
            || from_first.coord.distance(to_first.coord) != 1
            || from_second.coord.distance(to_second.coord) != 1
    }) {
        notes.push(format!("{label} fitted rows are not contiguous"));
    }

    let row_hits_barrier =
        |row: &[TilePos; 2]| row.iter().any(|position| barrier.contains(&position.coord));
    let endpoints_are_dry = rows
        .first()
        .zip(rows.last())
        .is_some_and(|(first_row, last_row)| {
            !row_hits_barrier(first_row) && !row_hits_barrier(last_row)
        });
    if !endpoints_are_dry {
        notes.push(format!(
            "{label} does not have one dry landing row on each bank"
        ));
    }
    if rows
        .iter()
        .skip(1)
        .take(rows.len().saturating_sub(2))
        .any(|row| !row_hits_barrier(row))
    {
        notes.push(format!(
            "{label} contains an excess dry row inside its fitted barrier span"
        ));
    }
    let (mut first_lane_hits, mut second_lane_hits) = (false, false);
    for &[first_position, second_position] in rows {
        first_lane_hits |= barrier.contains(&first_position.coord);
        second_lane_hits |= barrier.contains(&second_position.coord);
    }
    if !first_lane_hits || !second_lane_hits {
        notes.push(format!("{label} does not cross the barrier in both lanes"));
    }

    if crossing_has_barrier_reentry(rows, barrier, grid_radius) {
        notes.push(format!(
            "{label} meets the barrier again beyond a declared landing"
        ));
    }
}

fn crossing_has_barrier_reentry(
    rows: &[[TilePos; 2]],
    barrier: &BTreeSet<HexCoord>,
    grid_radius: u32,
) -> bool {
    let Some([first_row, second_row]) = rows.get(0..2) else {
        return false;
    };
    let Some([penultimate_row, last_row]) = rows.get(rows.len().saturating_sub(2)..) else {
        return false;
    };
    crossing_side_has_barrier(
        *first_row,
        crossing_step(*second_row, *first_row),
        barrier,
        grid_radius,
    ) || crossing_side_has_barrier(
        *last_row,
        crossing_step(*penultimate_row, *last_row),
        barrier,
        grid_radius,
    )
}

fn crossing_step(from: [TilePos; 2], to: [TilePos; 2]) -> (i32, i32) {
    let [from_first, _] = from;
    let [to_first, _] = to;
    (
        to_first.coord.x().saturating_sub(from_first.coord.x()),
        to_first.coord.y().saturating_sub(from_first.coord.y()),
    )
}

fn crossing_side_has_barrier(
    endpoint: [TilePos; 2],
    step: (i32, i32),
    barrier: &BTreeSet<HexCoord>,
    grid_radius: u32,
) -> bool {
    if step == (0, 0) {
        return false;
    }
    let [first, second] = endpoint;
    let mut row = [
        shift_coord(first.coord, step),
        shift_coord(second.coord, step),
    ];
    while row
        .iter()
        .any(|coord| HexCoord::ORIGIN.distance(*coord) <= grid_radius)
    {
        if row.iter().any(|coord| barrier.contains(coord)) {
            return true;
        }
        let [first, second] = row;
        row = [shift_coord(first, step), shift_coord(second, step)];
    }
    false
}

fn shift_coord(coord: HexCoord, step: (i32, i32)) -> HexCoord {
    HexCoord::from_axial(
        coord.x().saturating_add(step.0),
        coord.y().saturating_add(step.1),
    )
}

fn standable_surfaces(
    map: &VoxelMap,
    profile: TraversalProfile,
    is_solid: &dyn Fn(SubstanceId) -> bool,
) -> HashSet<TilePos> {
    let mut standable = HashSet::with_capacity(map.len());
    for (coord, column) in map.columns() {
        for level in (0..column.top()).rev() {
            let substance = column.get(level);
            let headroom = column.headroom_above(level.saturating_add(1));
            if profile.admits_surface(is_solid(substance), headroom) {
                standable.insert(TilePos::new(coord, level));
            }
        }
    }
    standable
}

fn traversal_graph(
    map: &VoxelMap,
    profile: TraversalProfile,
    is_solid: &dyn Fn(SubstanceId) -> bool,
) -> TraversalGraph {
    graph_from_surfaces(standable_surfaces(map, profile, is_solid), profile)
}

fn graph_from_surfaces(surfaces: HashSet<TilePos>, profile: TraversalProfile) -> TraversalGraph {
    let mut by_coord = HashMap::<HexCoord, Vec<TilePos>>::with_capacity(surfaces.len());
    for surface in &surfaces {
        by_coord.entry(surface.coord).or_default().push(*surface);
    }

    let mut edges = HashMap::with_capacity(surfaces.len());
    for surface in &surfaces {
        let mut adjacent = Vec::with_capacity(6);
        for neighbor in surface.coord.neighbors() {
            if let Some(candidates) = by_coord.get(&neighbor) {
                for candidate in candidates {
                    if profile.admits_step(*surface, *candidate) {
                        adjacent.push(*candidate);
                    }
                }
            }
        }
        edges.insert(*surface, adjacent);
    }

    TraversalGraph { surfaces, edges }
}

fn traversal_distances(
    start: TilePos,
    graph: &TraversalGraph,
    excluded: &BTreeSet<TilePos>,
) -> HashMap<TilePos, u32> {
    if !graph.surfaces.contains(&start) || excluded.contains(&start) {
        return HashMap::default();
    }
    let mut distances = HashMap::with_capacity(graph.surfaces.len());
    distances.insert(start, 0_u32);
    let mut queue = VecDeque::with_capacity(graph.surfaces.len());
    queue.push_back((start, 0_u32));
    while let Some((current, steps)) = queue.pop_front() {
        if let Some(adjacent) = graph.edges.get(&current) {
            for next in adjacent {
                if excluded.contains(next) || distances.contains_key(next) {
                    continue;
                }
                let distance = steps.saturating_add(1);
                distances.insert(*next, distance);
                queue.push_back((*next, distance));
            }
        }
    }
    distances
}

fn traversal_distance_to(
    start: TilePos,
    target: TilePos,
    graph: &TraversalGraph,
    excluded: &BTreeSet<TilePos>,
) -> Option<u32> {
    if !graph.surfaces.contains(&start)
        || !graph.surfaces.contains(&target)
        || excluded.contains(&start)
        || excluded.contains(&target)
    {
        return None;
    }
    if start == target {
        return Some(0);
    }

    let mut visited = HashSet::with_capacity(graph.surfaces.len());
    visited.insert(start);
    let mut queue = VecDeque::with_capacity(graph.surfaces.len());
    queue.push_back((start, 0_u32));
    while let Some((current, steps)) = queue.pop_front() {
        let Some(adjacent) = graph.edges.get(&current) else {
            continue;
        };
        for next in adjacent {
            if excluded.contains(next) || !visited.insert(*next) {
                continue;
            }
            let distance = steps.saturating_add(1);
            if *next == target {
                return Some(distance);
            }
            queue.push_back((*next, distance));
        }
    }
    None
}

fn score_candidate(
    plan: &TerrainPlan,
    environment: EnvironmentSettings,
    metrics: TacticalMetrics,
    candidate: u8,
) -> CandidateScore {
    let direct_route = match (
        plan.anchors.get(PARTY_START),
        plan.anchors.get(HOSTILE_START),
    ) {
        (Some(party), Some(hostile)) => party.coord.distance(hostile.coord),
        _ => plan.grid_radius,
    };
    let route_allowance = plan.grid_radius.saturating_div(2).max(4);
    let relief = u32::try_from(metrics.relief).unwrap_or_default();
    let (relief_minimum, relief_maximum, elevation_minimum, elevation_maximum) =
        if plan.kind == RecipeKind::HillsCrossing {
            (5, 8, 5, 10)
        } else {
            (0, 2, 2, 4)
        };
    let signature_band = match (plan.kind, environment) {
        (RecipeKind::LinkedSkyIslands, _) => (0, 10),
        (RecipeKind::HillsCrossing, EnvironmentSettings::TemperateGrassland) => (5, 15),
        (RecipeKind::HillsCrossing, EnvironmentSettings::Frozen) => (8, 20),
        (RecipeKind::HillsCrossing, EnvironmentSettings::Volcanic) => (8, 25),
    };

    CandidateScore {
        detour_band_distance: if plan.kind == RecipeKind::HillsCrossing {
            distance_to_band(metrics.alternate_detour_percent, 30, 50)
        } else {
            0
        },
        route_band_distance: distance_to_band(
            metrics.critical_route_steps,
            direct_route,
            direct_route.saturating_add(route_allowance),
        ),
        spawn_imbalance: metrics.spawn_height_difference,
        high_ground_band_distance: distance_to_band(
            u32::try_from(metrics.bank_high_ground_difference).unwrap_or(u32::MAX),
            0,
            1,
        ),
        relief_band_distance: distance_to_band(relief, relief_minimum, relief_maximum),
        elevation_diversity_distance: distance_to_band(
            metrics.reachable_elevation_levels,
            elevation_minimum,
            elevation_maximum,
        ),
        river_shape_band_distance: if plan.kind == RecipeKind::HillsCrossing {
            distance_to_band(metrics.river_sinuosity_percent, 8, 24)
        } else {
            0
        },
        environment_signature_distance: distance_to_band(
            metrics.environment_signature_percent,
            signature_band.0,
            signature_band.1,
        ),
        candidate,
    }
}

const fn distance_to_band(value: u32, minimum: u32, maximum: u32) -> u32 {
    if value < minimum {
        minimum - value
    } else {
        value.saturating_sub(maximum)
    }
}

const fn top_surface(cell: PlannedCell) -> Level {
    match cell.overlay {
        Some(overlay) => {
            if overlay.level > cell.surface {
                overlay.level
            } else {
                cell.surface
            }
        }
        None => cell.surface,
    }
}

pub(crate) fn named_hash(seed: u64, candidate: u8, stage: &str, index: u64) -> u64 {
    let mut bytes = Vec::with_capacity(stage.len().saturating_add(17));
    bytes.extend_from_slice(&seed.to_le_bytes());
    bytes.push(candidate);
    bytes.extend_from_slice(&index.to_le_bytes());
    bytes.extend_from_slice(stage.as_bytes());
    xxh3_64_with_seed(&bytes, seed)
}

fn sample_i32(seed: u64, candidate: u8, stage: &str, index: u64, min: i32, max: i32) -> i32 {
    if min >= max {
        return min;
    }
    let span = i64::from(max) - i64::from(min) + 1;
    let sampled = named_hash(seed, candidate, stage, index) % u64::try_from(span).unwrap_or(1);
    i32::try_from(i64::from(min) + i64::try_from(sampled).unwrap_or(0)).unwrap_or(min)
}

fn sample_level(
    seed: u64,
    candidate: u8,
    stage: &str,
    index: u64,
    min: Level,
    max: Level,
) -> Level {
    sample_i32(seed, candidate, stage, index, min, max)
}

fn coord_serial(coord: HexCoord) -> u64 {
    let x = u64::from(u32::from_le_bytes(coord.x().to_le_bytes()));
    let y = u64::from(u32::from_le_bytes(coord.y().to_le_bytes()));
    (x << 32) | y
}

fn rotate_third(coord: HexCoord, turns: u8) -> HexCoord {
    match turns % 3 {
        0 => coord,
        1 => HexCoord::from_axial(coord.z(), coord.x()),
        _ => HexCoord::from_axial(coord.y(), coord.z()),
    }
}

fn unrotate_third(coord: HexCoord, turns: u8) -> HexCoord {
    match turns % 3 {
        0 => coord,
        1 => HexCoord::from_axial(coord.y(), coord.z()),
        _ => HexCoord::from_axial(coord.z(), coord.x()),
    }
}

fn clamp_axial_y(x: i32, y: i32, radius: i32) -> i32 {
    let minimum = (-radius).max(-x - radius);
    let maximum = radius.min(-x + radius);
    y.clamp(minimum, maximum)
}

fn settings_fingerprint(grid_radius: u32, settings: &ProceduralSettings) -> u64 {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&grid_radius.to_le_bytes());
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    match &settings.landform {
        LandformSettings::Hills(hills) => {
            bytes.push(0);
            bytes.extend_from_slice(&hills.valley_level.to_le_bytes());
            bytes.extend_from_slice(&hills.max_relief.to_le_bytes());
            bytes.push(hills.hills_per_bank);
        }
        LandformSettings::SkyIslands(islands) => {
            bytes.push(1);
            bytes.extend_from_slice(&islands.surface_level.to_le_bytes());
            bytes.extend_from_slice(&islands.island_radius.to_le_bytes());
        }
    }
    bytes.push(match settings.environment {
        EnvironmentSettings::TemperateGrassland => 0,
        EnvironmentSettings::Frozen => 1,
        EnvironmentSettings::Volcanic => 2,
    });
    match &settings.tactical {
        TacticalSettings::Crossing(crossing) => {
            bytes.push(0);
            bytes.extend_from_slice(&crossing.barrier_half_width.to_le_bytes());
            bytes.extend_from_slice(&crossing.bed_level.to_le_bytes());
            bytes.extend_from_slice(&crossing.hazard_bottom.to_le_bytes());
            bytes.extend_from_slice(&crossing.hazard_top.to_le_bytes());
            bytes.extend_from_slice(&crossing.bridge_level.to_le_bytes());
        }
        TacticalSettings::LinkedIslands(linked) => {
            bytes.push(1);
            bytes.extend_from_slice(&linked.bridge_width.to_le_bytes());
        }
    }
    xxh3_64(&bytes)
}

fn map_fingerprint(map: &VoxelMap, special_regions: &SpecialMovementRegions) -> u64 {
    let mut bytes = Vec::new();
    let mut columns: Vec<(HexCoord, &Column)> = map.columns().collect();
    columns.sort_by_key(|(coord, _)| *coord);
    for (coord, column) in columns {
        bytes.extend_from_slice(&coord.x().to_le_bytes());
        bytes.extend_from_slice(&coord.y().to_le_bytes());
        bytes.extend_from_slice(&column.top().to_le_bytes());
        for substance in column.iter() {
            bytes.extend_from_slice(&substance.0.to_le_bytes());
        }
    }
    if !special_regions.is_empty() {
        bytes.extend_from_slice(b"special-movement-regions-v1");
        let mut memberships: Vec<(TilePos, SpecialMovementRegion)> =
            special_regions.iter().collect();
        memberships.sort_unstable();
        for (position, region) in memberships {
            bytes.extend_from_slice(&position.coord.x().to_le_bytes());
            bytes.extend_from_slice(&position.coord.y().to_le_bytes());
            bytes.extend_from_slice(&position.level.to_le_bytes());
            bytes.extend_from_slice(&region.0.to_le_bytes());
        }
    }
    xxh3_64(&bytes)
}

#[cfg(test)]
mod tests {
    use hex_assets::{SubstanceFile, SubstanceTable};

    use super::*;
    use crate::settings::TerrainSettings;
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
    const WALKER: TraversalProfile = TraversalProfile {
        levels_tall: 2,
        max_climb: 1,
        max_drop: 1,
    };
    const HERO_SEED: u64 = 1_592_598_566;
    // Selected once from seeds 0..1_024 for the iteration-one review pack. The
    // labels describe the measured extreme or median each seed represented; the
    // selector was intentionally removed after recording this provenance so tests
    // exercise the fixed corpus rather than rediscovering it.
    const FIXED_REGRESSION_SEEDS: [(&str, u64); 6] = [
        ("median", 4),
        ("relief-min", 1),
        ("relief-max", 275),
        ("sinuosity-min", 9),
        ("sinuosity-max", 850),
        ("fallback-pressure", 677),
    ];

    fn palette() -> TerrainPalette {
        TerrainPalette {
            bedrock: BEDROCK,
            stone: STONE,
            dirt: DIRT,
            grass: GRASS,
            gravel: GRAVEL,
            water: WATER,
            metal: METAL,
            snow: SNOW,
            ice: ICE,
            basalt: BASALT,
            lava: LAVA,
        }
    }

    fn solid(substance: SubstanceId) -> bool {
        !matches!(substance, SubstanceId::AIR | WATER | LAVA)
    }

    fn hills(environment: EnvironmentSettings) -> ProceduralSettings {
        ProceduralSettings {
            landform: LandformSettings::Hills(HillsSettings {
                valley_level: 15,
                max_relief: 8,
                hills_per_bank: 3,
            }),
            environment,
            tactical: TacticalSettings::Crossing(CrossingSettings {
                barrier_half_width: 1,
                bed_level: 12,
                hazard_bottom: 13,
                hazard_top: 14,
                bridge_level: 16,
            }),
        }
    }

    fn sky() -> ProceduralSettings {
        ProceduralSettings {
            landform: LandformSettings::SkyIslands(SkyIslandsSettings {
                surface_level: 15,
                island_radius: 3,
            }),
            environment: EnvironmentSettings::TemperateGrassland,
            tactical: TacticalSettings::LinkedIslands(LinkedIslandsSettings { bridge_width: 2 }),
        }
    }

    #[test]
    fn evaluates_exactly_eight_candidates_deterministically() {
        let first = build(
            12,
            &hills(EnvironmentSettings::TemperateGrassland),
            42,
            &palette(),
            WALKER,
            &solid,
        );
        let second = build(
            12,
            &hills(EnvironmentSettings::TemperateGrassland),
            42,
            &palette(),
            WALKER,
            &solid,
        );

        assert_eq!(first.report.candidates_evaluated, CANDIDATE_COUNT);
        assert_eq!(first.report.map_fingerprint, second.report.map_fingerprint);
        assert_eq!(
            first.report.map_fingerprint, 405_765_401_978_454_475,
            "generator v1 output changed without an explicit version update"
        );
        assert_eq!(
            first.report.selected_candidate,
            second.report.selected_candidate
        );
        assert_eq!(
            first.anchors.get(PARTY_START),
            second.anchors.get(PARTY_START)
        );
        assert!(first.special_regions.is_empty());
        assert!(second.special_regions.is_empty());
    }

    #[test]
    fn generator_v1_preset_goldens_are_stable() {
        let substances: SubstanceFile =
            ron::from_str(include_str!("../../../assets/config/substances.ron"))
                .expect("the shipped substances should parse");
        let table = SubstanceTable::from_file(&substances);
        let cases = [
            (
                "temperate hills",
                hills(EnvironmentSettings::TemperateGrassland),
                HERO_SEED,
                Some(1),
                4_508_295_216_895_027_881_u64,
                2_287_003_626_836_917_910_u64,
            ),
            (
                "frozen hills",
                hills(EnvironmentSettings::Frozen),
                484_450_342,
                Some(1),
                11_000_385_881_747_978_286,
                15_645_056_389_872_482_358,
            ),
            (
                "volcanic hills",
                hills(EnvironmentSettings::Volcanic),
                444_211_238,
                Some(1),
                15_742_618_080_999_901_279,
                9_467_682_862_694_642_740,
            ),
            (
                "sky islands",
                sky(),
                94_445_606,
                Some(0),
                6_724_558_830_461_654_069,
                17_755_945_497_268_195_861,
            ),
        ];

        for (
            label,
            settings,
            seed,
            expected_candidate,
            expected_settings_fingerprint,
            expected_map_fingerprint,
        ) in cases
        {
            let terrain = TerrainSettings::Procedural(crate::settings::ProceduralSettings::V1(
                settings.clone(),
            ));
            let runtime_palette = TerrainPalette::for_terrain(&table, &terrain)
                .expect("the shipped substance table should cover procedural terrain");
            let result = build(
                12,
                &settings,
                seed,
                &runtime_palette,
                WALKER,
                &|substance| table.is_solid(substance),
            );
            assert!(result.validated, "{label}: {:?}", result.report.notes);
            assert!(!result.report.used_fallback, "{label}");
            assert_eq!(
                result.report.selected_candidate, expected_candidate,
                "{label} changed candidate selection"
            );
            assert_eq!(
                result.report.settings_fingerprint, expected_settings_fingerprint,
                "{label} changed its V1 settings fingerprint"
            );
            assert_eq!(
                result.report.map_fingerprint, expected_map_fingerprint,
                "{label} changed its V1 map fingerprint"
            );
        }
    }

    #[test]
    fn generator_v1_multi_seed_selection_checksum_is_stable() {
        let settings = hills(EnvironmentSettings::TemperateGrassland);
        let mut bytes = Vec::new();
        for seed in [0, 1, 2, 42, HERO_SEED] {
            let result = build(12, &settings, seed, &palette(), WALKER, &solid);
            bytes.extend_from_slice(&seed.to_le_bytes());
            bytes.push(result.report.selected_candidate.unwrap_or(u8::MAX));
            bytes.extend_from_slice(&result.report.map_fingerprint.to_le_bytes());
        }
        assert_eq!(
            xxh3_64(&bytes),
            2_362_059_555_535_686_801,
            "generator v1 candidate selection changed without an explicit version update"
        );
    }

    #[test]
    fn named_streams_do_not_depend_on_other_stream_consumption() {
        let before = named_hash(77, 3, "river.meander", 4);
        for index in 0..100 {
            let _unused = named_hash(77, 3, "hills.relief", index);
        }
        let after = named_hash(77, 3, "river.meander", 4);
        assert_eq!(before, after);
    }

    #[test]
    fn scoring_uses_bands_before_later_quality_metrics() {
        assert_eq!(distance_to_band(29, 30, 50), 1);
        assert_eq!(distance_to_band(30, 30, 50), 0);
        assert_eq!(distance_to_band(41, 30, 50), 0);
        assert_eq!(distance_to_band(51, 30, 50), 1);

        let settings = hills(EnvironmentSettings::TemperateGrassland);
        let plan = construct_plan(12, &settings, 11, 0, false);
        let map = voxelize(&plan, settings.environment, &palette());
        let validation = validate_exact(&plan, &map, &palette(), WALKER, &solid);
        assert!(validation.valid, "{:?}", validation.notes);

        let mut balanced = validation.metrics;
        balanced.alternate_detour_percent = 30;
        balanced.bank_high_ground_difference = 0;
        let mut imbalanced = balanced;
        imbalanced.alternate_detour_percent = 50;
        imbalanced.bank_high_ground_difference = 2;
        assert!(
            score_candidate(&plan, settings.environment, balanced, 0)
                < score_candidate(&plan, settings.environment, imbalanced, 0),
            "later balance metrics should decide between values in the same tactical band"
        );

        let mut outside = balanced;
        outside.alternate_detour_percent = 29;
        outside.bank_high_ground_difference = 0;
        let mut inside = balanced;
        inside.bank_high_ground_difference = 20;
        assert!(
            score_candidate(&plan, settings.environment, inside, 0)
                < score_candidate(&plan, settings.environment, outside, 0),
            "an in-band tactical result should beat an out-of-band result"
        );
    }

    #[test]
    fn river_sinuosity_uses_path_distance_and_is_rotation_invariant() {
        let settings = hills(EnvironmentSettings::TemperateGrassland);
        let mut plan = construct_plan(12, &settings, 25, 0, false);
        let expected_steps = plan
            .barrier_centres
            .windows(2)
            .map(|pair| match pair {
                [from, to] => from.distance(*to),
                _ => 0,
            })
            .fold(0_u32, u32::saturating_add);
        assert!(
            expected_steps
                >= plan
                    .barrier_centres
                    .first()
                    .zip(plan.barrier_centres.last())
                    .map_or(0, |(first, last)| first.distance(*last))
        );
        let original = river_sinuosity_percent(&plan);
        plan.barrier_centres = plan
            .barrier_centres
            .iter()
            .map(|coord| rotate_third(*coord, 1))
            .collect();
        assert_eq!(river_sinuosity_percent(&plan), original);
    }

    #[test]
    fn ordered_ribbon_is_exact_through_straights_bends_boundaries_and_rotations() {
        let straight_controls = [HexCoord::from_axial(-4, 0), HexCoord::from_axial(4, 0)];
        let straight =
            ordered_centerline(&straight_controls).expect("straight centreline should be valid");
        let straight_sections = river_ribbon_sections(&straight, 8)
            .expect("straight centreline should produce a ribbon");
        let straight_barrier = ribbon_barrier(&straight_sections);
        assert_eq!(
            straight_barrier.len(),
            straight.len().saturating_mul(3).saturating_sub(2)
        );

        let controls = [
            HexCoord::from_axial(-4, 0),
            HexCoord::from_axial(0, 0),
            HexCoord::from_axial(2, -2),
            HexCoord::from_axial(4, -2),
        ];
        for orientation in 0..3 {
            let rotated_controls: Vec<HexCoord> = controls
                .iter()
                .map(|coord| rotate_third(*coord, orientation))
                .collect();
            let centres = ordered_centerline(&rotated_controls)
                .expect("ordered bend should remain adjacent and unique");
            assert!(centres
                .windows(2)
                .all(|pair| matches!(pair, [from, to] if from.distance(*to) == 1)));
            assert_eq!(
                centres.iter().copied().collect::<BTreeSet<_>>().len(),
                centres.len()
            );
            let sections =
                river_ribbon_sections(&centres, 8).expect("bend should retain exact width");
            assert!(sections
                .iter()
                .all(|section| { section.iter().copied().collect::<BTreeSet<_>>().len() == 3 }));
            let barrier = ribbon_barrier(&sections);
            let old_expansion: BTreeSet<HexCoord> = centres
                .iter()
                .flat_map(|centre| centre.within_radius(1))
                .collect();
            assert!(barrier.is_subset(&old_expansion));
            assert!(
                old_expansion.len() > barrier.len(),
                "edge ribbon should omit bend and end-cap bulges"
            );
        }

        for orientation in 0..3 {
            let boundary_controls = [
                rotate_third(HexCoord::from_axial(-12, 0), orientation),
                rotate_third(HexCoord::from_axial(12, 0), orientation),
            ];
            let centres = ordered_centerline(&boundary_controls)
                .expect("boundary centreline should remain ordered");
            let sections = river_ribbon_sections(&centres, 12)
                .expect("boundary sections should remain three-wide");
            assert!(sections.iter().all(|section| {
                section.iter().copied().collect::<BTreeSet<_>>().len() == 3
                    && section
                        .iter()
                        .all(|coord| HexCoord::ORIGIN.distance(*coord) <= 12)
            }));
        }
    }

    #[test]
    fn ordered_centreline_rejects_a_non_adjacent_repeat() {
        let controls = [
            HexCoord::ORIGIN,
            HexCoord::from_axial(2, 0),
            HexCoord::ORIGIN,
        ];
        assert!(ordered_centerline(&controls).is_none());
    }

    #[test]
    fn fitted_crossings_follow_actual_barrier_extent_in_every_rotation() {
        for orientation in 0..3 {
            let controls = [
                rotate_third(HexCoord::from_axial(-12, 0), orientation),
                rotate_third(HexCoord::from_axial(12, 0), orientation),
            ];
            let centres =
                ordered_centerline(&controls).expect("straight crossing line should be valid");
            let sections =
                river_ribbon_sections(&centres, 12).expect("straight river should form a ribbon");
            let barrier = ribbon_barrier(&sections);
            let crossing = fit_crossing_lanes(12, orientation, &barrier, &centres, 0)
                .expect("straight river should admit a fitted crossing");

            assert_eq!(crossing.rows.len(), 5);
            assert_eq!(crossing.lanes.0.len(), 5);
            assert_eq!(crossing.lanes.1.len(), 5);
            let row_hits = |row: &[HexCoord; 2]| row.iter().any(|coord| barrier.contains(coord));
            assert!(crossing.rows.first().is_some_and(|row| !row_hits(row)));
            assert!(crossing.rows.last().is_some_and(|row| !row_hits(row)));
            assert!(crossing
                .rows
                .iter()
                .skip(1)
                .take(crossing.rows.len().saturating_sub(2))
                .all(row_hits));
            assert!(barrier.contains(&crossing.centre));
        }

        for orientation in 0..3 {
            let bend_controls = [
                rotate_third(HexCoord::from_axial(-12, 0), orientation),
                rotate_third(HexCoord::from_axial(-1, 0), orientation),
                rotate_third(HexCoord::from_axial(2, -3), orientation),
                rotate_third(HexCoord::from_axial(12, -3), orientation),
            ];
            let centres =
                ordered_centerline(&bend_controls).expect("bent centreline should be valid");
            let sections =
                river_ribbon_sections(&centres, 12).expect("bent river should form a ribbon");
            let barrier = ribbon_barrier(&sections);
            let crossing = fit_crossing_lanes(12, orientation, &barrier, &centres, 0)
                .expect("bend should admit a bounded fitted crossing");
            let barrier_rows = crossing
                .rows
                .iter()
                .filter(|row| row.iter().any(|coord| barrier.contains(coord)))
                .count();
            assert_eq!(crossing.rows.len(), barrier_rows.saturating_add(2));
            assert_eq!(crossing.lanes.0.len(), crossing.lanes.1.len());
            assert!(barrier.contains(&crossing.centre));
        }
    }

    #[test]
    fn fitted_crossing_validation_rejects_excess_land_and_barrier_reentry() {
        let controls = [HexCoord::from_axial(-12, 0), HexCoord::from_axial(12, 0)];
        let centres = ordered_centerline(&controls).expect("straight line should be valid");
        let sections = river_ribbon_sections(&centres, 12).expect("ribbon should be valid");
        let barrier = ribbon_barrier(&sections);
        let crossing = fit_crossing_lanes(12, 0, &barrier, &centres, 0)
            .expect("straight river should admit a crossing");
        let mut rows = tile_rows(&crossing.rows, 16);
        let Some([first_row, second_row]) = rows.get(0..2) else {
            panic!("fitted crossing should have at least two rows");
        };
        let outward = crossing_step(*second_row, *first_row);
        let [first, second] = *first_row;
        rows.insert(
            0,
            [
                TilePos::new(shift_coord(first.coord, outward), first.level),
                TilePos::new(shift_coord(second.coord, outward), second.level),
            ],
        );
        let coordinate_rows: Vec<[HexCoord; 2]> = rows
            .iter()
            .map(|&[first, second]| [first.coord, second.coord])
            .collect();
        let coordinate_lanes = coordinate_lanes(&coordinate_rows);
        let first_lane: BTreeSet<TilePos> = coordinate_lanes
            .0
            .iter()
            .map(|coord| TilePos::new(*coord, 16))
            .collect();
        let second_lane: BTreeSet<TilePos> = coordinate_lanes
            .1
            .iter()
            .map(|coord| TilePos::new(*coord, 16))
            .collect();
        let declared: BTreeSet<TilePos> = first_lane.union(&second_lane).copied().collect();
        let surfaces: HashSet<TilePos> = declared.iter().copied().collect();
        let mut notes = Vec::new();
        validate_fitted_crossing(
            "test crossing",
            &rows,
            &first_lane,
            &second_lane,
            &declared,
            &barrier,
            &surfaces,
            12,
            &mut notes,
        );
        assert!(
            notes.iter().any(|note| note.contains("excess dry row")),
            "{notes:?}"
        );

        let Some(last_row) = rows.last().copied() else {
            panic!("fitted crossing should have an ending");
        };
        let Some(penultimate_row) = rows.get(rows.len().saturating_sub(2)).copied() else {
            panic!("fitted crossing should have a penultimate row");
        };
        let reentry_step = crossing_step(penultimate_row, last_row);
        let [last_first, _] = last_row;
        let reentry = shift_coord(last_first.coord, reentry_step);
        let mut reentering_barrier = barrier;
        reentering_barrier.insert(reentry);
        notes.clear();
        validate_fitted_crossing(
            "test crossing",
            &rows,
            &first_lane,
            &second_lane,
            &declared,
            &reentering_barrier,
            &surfaces,
            12,
            &mut notes,
        );
        assert!(
            notes.iter().any(|note| note.contains("again beyond")),
            "{notes:?}"
        );
    }

    #[test]
    fn fitted_crossing_validation_terminates_on_a_repeated_row() {
        let controls = [HexCoord::from_axial(-12, 0), HexCoord::from_axial(12, 0)];
        let centres = ordered_centerline(&controls).expect("straight line should be valid");
        let sections = river_ribbon_sections(&centres, 12).expect("ribbon should be valid");
        let barrier = ribbon_barrier(&sections);
        let crossing = fit_crossing_lanes(12, 0, &barrier, &centres, 0)
            .expect("straight river should admit a crossing");
        let mut rows = tile_rows(&crossing.rows, 16);
        let repeated = rows
            .first()
            .copied()
            .expect("fitted crossing should have a first row");
        *rows
            .get_mut(1)
            .expect("fitted crossing should have a second row") = repeated;

        let first_lane: BTreeSet<TilePos> = rows.iter().map(|row| row[0]).collect();
        let second_lane: BTreeSet<TilePos> = rows.iter().map(|row| row[1]).collect();
        let declared: BTreeSet<TilePos> = first_lane.union(&second_lane).copied().collect();
        let surfaces: HashSet<TilePos> = declared.iter().copied().collect();
        let mut notes = Vec::new();

        validate_fitted_crossing(
            "corrupt crossing",
            &rows,
            &first_lane,
            &second_lane,
            &declared,
            &barrier,
            &surfaces,
            12,
            &mut notes,
        );

        assert!(
            notes.iter().any(|note| note.contains("not contiguous")),
            "{notes:?}"
        );
    }

    #[test]
    fn exact_validation_rejects_corrupt_ribbon_metadata() {
        let settings = hills(EnvironmentSettings::TemperateGrassland);
        let mut plan = construct_plan(12, &settings, 73, 0, false);
        let Some(section) = plan.barrier_sections.first_mut() else {
            panic!("hills should publish ribbon sections");
        };
        let [centre, first, _] = *section;
        *section = [centre, first, centre];
        let map = voxelize(&plan, settings.environment, &palette());
        let validation = validate_exact(&plan, &map, &palette(), WALKER, &solid);
        assert!(
            validation
                .notes
                .iter()
                .any(|note| note.contains("sections") || note.contains("three distinct")),
            "{:?}",
            validation.notes
        );
    }

    #[test]
    fn validation_requires_every_declared_anchor() {
        let settings = hills(EnvironmentSettings::TemperateGrassland);
        for required in [
            PARTY_START,
            HOSTILE_START,
            CONFLICT_CENTER,
            BRIDGE,
            ALTERNATE_CROSSING,
        ] {
            let mut plan = construct_plan(12, &settings, 91, 0, false);
            plan.anchors.positions.remove(required);
            let map = voxelize(&plan, settings.environment, &palette());
            let validation = validate_exact(&plan, &map, &palette(), WALKER, &solid);
            assert!(!validation.valid);
            assert!(
                validation.notes.iter().any(|note| note.contains(required)),
                "{required}: {:?}",
                validation.notes
            );
        }
    }

    #[test]
    fn exact_validation_rejects_corrupt_strata_and_hazard_depth() {
        let settings = hills(EnvironmentSettings::TemperateGrassland);
        let plan = construct_plan(12, &settings, 73, 0, false);

        let mut corrupt_strata = voxelize(&plan, settings.environment, &palette());
        let (&coord, cell) = plan
            .cells
            .iter()
            .find(|(_, cell)| cell.hazard.is_none() && cell.overlay.is_none())
            .expect("hills should contain dry terrain");
        corrupt_strata.set(TilePos::new(coord, cell.surface - 2), GRASS);
        let strata_validation = validate_exact(&plan, &corrupt_strata, &palette(), WALKER, &solid);
        assert!(
            strata_validation
                .notes
                .iter()
                .any(|note| note.contains("expected SubstanceId")),
            "{:?}",
            strata_validation.notes
        );

        let mut corrupt_hazard = voxelize(&plan, settings.environment, &palette());
        let (&hazard_coord, hazard) = plan
            .cells
            .iter()
            .filter_map(|(coord, cell)| cell.hazard.map(|hazard| (coord, hazard)))
            .next()
            .expect("hills should contain a hazard");
        corrupt_hazard.set(TilePos::new(hazard_coord, hazard.bottom), SubstanceId::AIR);
        let hazard_validation = validate_exact(&plan, &corrupt_hazard, &palette(), WALKER, &solid);
        assert!(
            hazard_validation
                .notes
                .iter()
                .any(|note| note.contains("expected hazard")),
            "{:?}",
            hazard_validation.notes
        );
    }

    #[test]
    fn local_height_defect_is_repaired_within_the_round_limit() {
        let settings = hills(EnvironmentSettings::TemperateGrassland);
        let mut plan = construct_plan(12, &settings, 101, 0, false);
        let party = plan
            .anchors
            .get(PARTY_START)
            .expect("party anchor should exist");
        let neighbor = party
            .coord
            .neighbors()
            .into_iter()
            .find(|coord| {
                plan.cells
                    .get(coord)
                    .is_some_and(|cell| cell.hazard.is_none() && cell.overlay.is_none())
            })
            .expect("party should have a dry neighbor");
        plan.cells
            .get_mut(&neighbor)
            .expect("neighbor should be planned")
            .surface = party.level + 4;

        let (_, validation, repairs) =
            voxelize_validate_repair(&mut plan, &settings, 101, 0, &palette(), WALKER, &solid);
        assert!(validation.valid, "{:?}", validation.notes);
        assert!(!repairs.is_empty());
        assert!(repairs.len() <= usize::from(MAX_REPAIR_ROUNDS));
    }

    #[test]
    fn every_repair_round_preserves_exact_crossing_and_anchor_positions() {
        for environment in [
            EnvironmentSettings::TemperateGrassland,
            EnvironmentSettings::Volcanic,
        ] {
            let settings = hills(environment);
            let mut plan = construct_plan(12, &settings, 101, 0, false);
            let expected_alternate = plan.alternate.clone();

            for round in 0..MAX_REPAIR_ROUNDS {
                repair_plan(&mut plan, round);
                synchronize_anchor_levels(&mut plan);

                assert_eq!(
                    plan.alternate,
                    expected_alternate,
                    "repair {} moved the declared ford or causeway",
                    round + 1
                );
                let alternate_lanes = plan
                    .crossing_lanes
                    .get(1)
                    .expect("hills plans should declare an alternate crossing");
                let declared_lanes: BTreeSet<TilePos> = alternate_lanes
                    .0
                    .union(&alternate_lanes.1)
                    .copied()
                    .collect();
                assert_eq!(
                    declared_lanes,
                    plan.alternate,
                    "repair {} desynchronized the alternate crossing lanes",
                    round + 1
                );
                for position in &plan.alternate {
                    let cell = plan
                        .cells
                        .get(&position.coord)
                        .expect("alternate crossing cell should remain planned");
                    assert_eq!(
                        top_surface(*cell),
                        position.level,
                        "repair {} desynchronized an alternate crossing TilePos",
                        round + 1
                    );
                }
                for (name, position) in plan.anchors.iter() {
                    let cell = plan
                        .cells
                        .get(&position.coord)
                        .expect("anchor cell should remain planned");
                    assert_eq!(
                        top_surface(*cell),
                        position.level,
                        "repair {} desynchronized the {name} anchor",
                        round + 1
                    );
                }
                assert!(
                    plan.alternate.contains(
                        &plan
                            .anchors
                            .get(ALTERNATE_CROSSING)
                            .expect("alternate crossing anchor should exist")
                    ),
                    "repair {} moved the alternate anchor off its crossing",
                    round + 1
                );
            }
        }
    }

    #[test]
    fn hills_have_two_independent_crossings_and_disconnect_without_them() {
        let settings = hills(EnvironmentSettings::TemperateGrassland);
        let result = build(12, &settings, 20_260_726, &palette(), WALKER, &solid);
        assert!(!result.report.used_fallback, "{:?}", result.report.notes);
        assert_eq!(result.report.valid_candidates, CANDIDATE_COUNT);

        let plan = construct_plan(12, &settings, 20_260_726, 0, false);
        let map = voxelize(&plan, settings.environment, &palette());
        let validation = validate_exact(&plan, &map, &palette(), WALKER, &solid);
        assert!(validation.valid, "{:?}", validation.notes);
    }

    #[test]
    fn hills_preserve_bedrock_strata_relief_and_two_level_headroom() {
        let result = build(
            12,
            &hills(EnvironmentSettings::TemperateGrassland),
            123,
            &palette(),
            WALKER,
            &solid,
        );
        assert_eq!(result.map.len(), 469);
        assert!(result.report.metrics.relief <= 8);
        for (coord, column) in result.map.columns() {
            assert_eq!(column.get(0), BEDROCK, "missing bedrock at {coord:?}");
        }

        let surfaces = standable_surfaces(&result.map, WALKER, &solid);
        for surface in surfaces {
            let Some(column) = result.map.column(surface.coord) else {
                continue;
            };
            assert!(column.get(surface.level + 1).is_air());
            assert!(column.get(surface.level + 2).is_air());
        }
    }

    fn selected_hills_plan(seed: u64) -> (TerrainPlan, Vec<HillShape>) {
        let settings = hills(EnvironmentSettings::TemperateGrassland);
        let selected = build(12, &settings, seed, &palette(), WALKER, &solid);
        assert!(
            selected.validated,
            "seed {seed}: {:?}",
            selected.report.notes
        );
        let candidate = selected.report.selected_candidate.unwrap_or(0);
        let fallback = selected.report.used_fallback;
        let mut plan = construct_plan(12, &settings, seed, candidate, fallback);
        let (_, validation, _) = voxelize_validate_repair(
            &mut plan,
            &settings,
            seed,
            candidate,
            &palette(),
            WALKER,
            &solid,
        );
        assert!(validation.valid, "seed {seed}: {:?}", validation.notes);

        let LandformSettings::Hills(hills) = &settings.landform else {
            panic!("test settings should use hills");
        };
        let TacticalSettings::Crossing(crossing) = &settings.tactical else {
            panic!("test settings should use a crossing");
        };
        let geometry = river_geometry(12, crossing, seed, candidate, fallback);
        let shapes = hill_shapes(12, hills, &geometry, seed, candidate, fallback);
        (plan, shapes)
    }

    fn role_components(plan: &TerrainPlan, role: SurfaceRole) -> Vec<usize> {
        let mut remaining: BTreeSet<HexCoord> = plan
            .cells
            .iter()
            .filter_map(|(coord, cell)| {
                (cell.hazard.is_none() && cell.overlay.is_none() && cell.role == role)
                    .then_some(*coord)
            })
            .collect();
        let mut sizes = Vec::new();
        while let Some(start) = remaining.first().copied() {
            remaining.remove(&start);
            let mut size = 0_usize;
            let mut frontier = VecDeque::from([start]);
            while let Some(coord) = frontier.pop_front() {
                size = size.saturating_add(1);
                for neighbor in coord.neighbors() {
                    if remaining.remove(&neighbor) {
                        frontier.push_back(neighbor);
                    }
                }
            }
            sizes.push(size);
        }
        sizes
    }

    #[test]
    fn hero_corpus_has_lobed_contours_feathered_spawns_and_coherent_materials() {
        let seeds = std::iter::once(("hero", HERO_SEED)).chain(FIXED_REGRESSION_SEEDS);
        let mut measured_rings = 0_usize;
        let mut asymmetric_rings = 0_usize;
        let mut ordinary_cells = 0_usize;
        let mut grass_cells = 0_usize;
        let mut dirt_cells = 0_usize;
        let mut stone_cells = 0_usize;

        for (label, seed) in seeds {
            let (plan, shapes) = selected_hills_plan(seed);
            let mut map_measured_rings = 0_usize;
            let mut map_asymmetric_rings = 0_usize;
            for shape in shapes {
                let levels: BTreeSet<Level> = shape.lobes[0]
                    .centre
                    .neighbors()
                    .into_iter()
                    .filter_map(|coord| plan.cells.get(&coord))
                    .filter(|cell| cell.hazard.is_none())
                    .map(|cell| cell.surface)
                    .collect();
                if levels.len() > 1 {
                    asymmetric_rings = asymmetric_rings.saturating_add(1);
                    map_asymmetric_rings = map_asymmetric_rings.saturating_add(1);
                }
                if !levels.is_empty() {
                    measured_rings = measured_rings.saturating_add(1);
                    map_measured_rings = map_measured_rings.saturating_add(1);
                }
            }
            assert!(
                map_asymmetric_rings.saturating_mul(100) >= map_measured_rings.saturating_mul(40),
                "{label} breaks radial symmetry around only \
                 {map_asymmetric_rings}/{map_measured_rings} hill centres"
            );

            for anchor_name in [PARTY_START, HOSTILE_START] {
                let anchor = plan
                    .anchors
                    .get(anchor_name)
                    .expect("hills should publish both start anchors");
                let legal_neighbors: Vec<HexCoord> = anchor
                    .coord
                    .neighbors()
                    .into_iter()
                    .filter(|coord| {
                        plan.cells.get(coord).is_some_and(|cell| {
                            cell.hazard.is_none()
                                && WALKER
                                    .admits_step(anchor, TilePos::new(*coord, top_surface(*cell)))
                        })
                    })
                    .collect();
                assert!(
                    legal_neighbors.iter().any(|first| {
                        legal_neighbors
                            .iter()
                            .any(|second| first != second && first.distance(*second) == 1)
                    }),
                    "{label} {anchor_name} has no two-wide ordinary egress"
                );

                let mut level_counts = BTreeMap::new();
                for coord in anchor.coord.within_radius(2) {
                    let Some(cell) = plan.cells.get(&coord) else {
                        continue;
                    };
                    if cell.hazard.is_some() {
                        continue;
                    }
                    *level_counts.entry(cell.surface).or_insert(0_usize) += 1;
                }
                let measured_cells = level_counts.values().sum::<usize>();
                let modal_cells = level_counts.values().copied().max().unwrap_or(0);
                assert!(
                    modal_cells.saturating_mul(100) <= measured_cells.saturating_mul(85),
                    "{label} {anchor_name} has a flat spawn arena: \
                     {modal_cells}/{measured_cells} cells share one level"
                );
            }

            let dirt_components = role_components(&plan, SurfaceRole::Dirt);
            let stone_components = role_components(&plan, SurfaceRole::Stone);
            assert!(
                dirt_components.iter().all(|size| *size >= 3),
                "{label} contains a tiny dirt exposure: {dirt_components:?}"
            );
            assert!(
                stone_components.iter().all(|size| *size >= 3),
                "{label} contains a tiny stone cap: {stone_components:?}"
            );

            for cell in plan
                .cells
                .values()
                .filter(|cell| cell.hazard.is_none() && cell.overlay.is_none() && !cell.gated)
            {
                ordinary_cells = ordinary_cells.saturating_add(1);
                grass_cells =
                    grass_cells.saturating_add(usize::from(cell.role == SurfaceRole::Grass));
                dirt_cells = dirt_cells.saturating_add(usize::from(cell.role == SurfaceRole::Dirt));
                stone_cells =
                    stone_cells.saturating_add(usize::from(cell.role == SurfaceRole::Stone));
            }
        }

        assert!(
            asymmetric_rings.saturating_mul(100) >= measured_rings.saturating_mul(55),
            "only {asymmetric_rings}/{measured_rings} first contours break radial symmetry"
        );
        assert!(
            grass_cells.saturating_mul(100) >= ordinary_cells.saturating_mul(55),
            "grass covers only {grass_cells}/{ordinary_cells} ordinary terrace cells"
        );
        assert!(
            dirt_cells.saturating_mul(100) >= ordinary_cells.saturating_mul(3)
                && dirt_cells.saturating_mul(100) <= ordinary_cells.saturating_mul(18),
            "dirt coverage {dirt_cells}/{ordinary_cells} is outside the 3-18% proxy band"
        );
        assert!(
            stone_cells.saturating_mul(100) >= ordinary_cells
                && stone_cells.saturating_mul(100) <= ordinary_cells.saturating_mul(15),
            "stone coverage {stone_cells}/{ordinary_cells} is outside the 1-15% proxy band"
        );
    }

    #[test]
    fn final_hill_materials_match_the_projected_elevation() {
        let settings = hills(EnvironmentSettings::TemperateGrassland);
        let plan = construct_plan(12, &settings, 314, 0, false);
        for (coord, cell) in &plan.cells {
            if cell.role == SurfaceRole::Stone {
                let lower_neighbors = coord
                    .neighbors()
                    .into_iter()
                    .filter_map(|neighbor| plan.cells.get(&neighbor))
                    .filter(|neighbor| neighbor.surface < cell.surface)
                    .count();
                assert!(
                    cell.surface >= plan.base_level + 6
                        || (cell.surface == plan.base_level + 5 && lower_neighbors >= 3),
                    "stone at {coord:?} level {} is neither high nor exposed",
                    cell.surface
                );
            }
        }
    }

    #[test]
    fn hills_follow_each_exact_river_orientation() {
        let settings = hills(EnvironmentSettings::TemperateGrassland);
        let LandformSettings::Hills(hills) = &settings.landform else {
            panic!("test recipe should use Hills")
        };
        let TacticalSettings::Crossing(crossing) = &settings.tactical else {
            panic!("test recipe should use Crossing")
        };
        let mut seen = [false; 3];
        for seed in 0..128 {
            let geometry = river_geometry(12, crossing, seed, 0, false);
            let index = usize::from(geometry.orientation);
            if seen.get(index).copied().unwrap_or(true) {
                continue;
            }
            if let Some(slot) = seen.get_mut(index) {
                *slot = true;
            }
            let shapes = hill_shapes(12, hills, &geometry, seed, 0, false);
            let bank_count = usize::from(hills.hills_per_bank);
            assert!(
                shapes.iter().take(bank_count).all(|shape| {
                    unrotate_third(shape.lobes[0].centre, geometry.orientation).y() < 0
                }),
                "seed {seed}, orientation {}, shapes {shapes:?}",
                geometry.orientation
            );
            assert!(
                shapes.iter().skip(bank_count).all(|shape| {
                    unrotate_third(shape.lobes[0].centre, geometry.orientation).y() > 0
                }),
                "seed {seed}, orientation {}, shapes {shapes:?}",
                geometry.orientation
            );
        }
        assert!(seen.into_iter().all(|orientation| orientation));
    }

    #[test]
    fn frozen_and_volcanic_variants_keep_the_hills_contract() {
        for environment in [EnvironmentSettings::Frozen, EnvironmentSettings::Volcanic] {
            let result = build(12, &hills(environment), 9_001, &palette(), WALKER, &solid);
            assert!(result.report.valid_candidates > 0);
            assert!(!result.report.used_fallback, "{:?}", result.report.notes);
            let used: BTreeSet<SubstanceId> = result
                .map
                .columns()
                .flat_map(|(_, column)| column.iter())
                .collect();
            match environment {
                EnvironmentSettings::Frozen => {
                    assert!(used.contains(&SNOW));
                    assert!(used.contains(&ICE));
                    assert!(used.contains(&WATER));
                }
                EnvironmentSettings::Volcanic => {
                    assert!(used.contains(&BASALT));
                    assert!(used.contains(&LAVA));
                }
                EnvironmentSettings::TemperateGrassland => {}
            }
        }
    }

    #[test]
    fn frozen_validation_rejects_a_plan_without_an_ice_surface() {
        let settings = hills(EnvironmentSettings::Frozen);
        let mut plan = construct_plan(12, &settings, 9_001, 0, false);
        assert!(
            plan.cells
                .values()
                .any(|cell| cell.role == SurfaceRole::Ice),
            "the frozen recipe should guarantee an ice surface before validation"
        );
        for cell in plan.cells.values_mut() {
            if cell.role == SurfaceRole::Ice {
                cell.role = SurfaceRole::Snow;
            }
        }

        let map = voxelize(&plan, settings.environment, &palette());
        let validation = validate_exact(&plan, &map, &palette(), WALKER, &solid);
        assert!(!validation.valid);
        assert!(
            validation
                .notes
                .iter()
                .any(|note| note.contains("no ice surface")),
            "{:?}",
            validation.notes
        );
    }

    #[test]
    fn validation_uses_the_same_data_driven_solidity_as_live_movement() {
        let metal_is_not_solid = |substance: SubstanceId| solid(substance) && substance != METAL;
        let result = build(
            12,
            &hills(EnvironmentSettings::TemperateGrassland),
            77,
            &palette(),
            WALKER,
            &metal_is_not_solid,
        );

        assert!(!result.validated);
        assert!(result.report.used_fallback);
        assert!(
            result
                .report
                .notes
                .iter()
                .any(|note| note.contains("bridge") || note.contains("anchor")),
            "{:?}",
            result.report.notes
        );
    }

    #[test]
    fn standability_uses_column_headroom_for_every_surface() {
        let coord = HexCoord::ORIGIN;
        let mut column = Column::filled(STONE, 8);
        column.set(3, SubstanceId::AIR);
        column.set(4, SubstanceId::AIR);
        let mut map = VoxelMap::new();
        map.insert_column(coord, column.clone());

        let surfaces = standable_surfaces(&map, WALKER, &solid);
        for level in 0..column.top() {
            let position = TilePos::new(coord, level);
            let expected = WALKER.admits_surface(
                solid(column.get(level)),
                column.headroom_above(level.saturating_add(1)),
            );
            assert_eq!(
                surfaces.contains(&position),
                expected,
                "standability diverged from Column headroom at {position:?}"
            );
        }
        assert!(surfaces.contains(&TilePos::new(coord, 2)));
        assert!(surfaces.contains(&TilePos::new(coord, 7)));
        assert!(!surfaces.contains(&TilePos::new(coord, 1)));
    }

    #[test]
    fn linked_sky_islands_connect_required_anchors_but_not_gated_islands() {
        let settings = sky();
        let result = build(12, &settings, 808, &palette(), WALKER, &solid);
        assert!(
            result.report.valid_candidates > 0,
            "{:?}",
            result.report.notes
        );
        assert!(!result.report.used_fallback, "{:?}", result.report.notes);

        let has_floating_air = result.map.columns().any(|(_, column)| {
            column.get(0).is_air() && (1..column.top()).any(|level| !column.get(level).is_air())
        });
        assert!(has_floating_air);

        let memberships: BTreeMap<TilePos, SpecialMovementRegion> =
            result.special_regions.iter().collect();
        let region_ids: BTreeSet<SpecialMovementRegion> = memberships.values().copied().collect();
        assert!(!memberships.is_empty());
        assert_eq!(
            region_ids.len(),
            2,
            "the two optional islands should retain distinct region ids"
        );
        assert!(
            result
                .anchors
                .iter()
                .all(|(_, position)| !memberships.contains_key(&position)),
            "critical anchors must stay outside special-movement regions"
        );

        let graph = traversal_graph(&result.map, WALKER, &solid);
        let party = result
            .anchors
            .get(PARTY_START)
            .expect("sky islands should publish the party anchor");
        let distances = traversal_distances(party, &graph, &BTreeSet::new());
        for position in memberships.keys() {
            assert!(
                graph.surfaces.contains(position),
                "tagged surface {position:?} should be standable"
            );
            assert!(
                !distances.contains_key(position),
                "tagged surface {position:?} should require special movement"
            );
        }

        let repeated = build(12, &settings, 808, &palette(), WALKER, &solid);
        assert_eq!(
            repeated.special_regions.iter().collect::<BTreeMap<_, _>>(),
            memberships,
            "region ids and exact surfaces should be deterministic"
        );
        assert_eq!(
            repeated.report.map_fingerprint,
            result.report.map_fingerprint
        );
        assert_ne!(
            map_fingerprint(&result.map, &SpecialMovementRegions::new()),
            result.report.map_fingerprint,
            "special-movement semantics must contribute to the map fingerprint"
        );

        let plan = construct_plan(12, &settings, 808, 0, false);
        assert_eq!(plan.sky_bridge_lanes.len(), 2);
        for (first, second) in &plan.sky_bridge_lanes {
            assert_eq!(first.len(), second.len());
            assert!(first.is_disjoint(second));
            assert!(first.iter().all(|position| {
                second
                    .iter()
                    .any(|other| position.coord.distance(other.coord) == 1)
            }));
        }
    }

    #[test]
    fn validation_rejects_a_critical_anchor_tagged_as_special_movement() {
        let settings = sky();
        let mut plan = construct_plan(12, &settings, 808, 0, false);
        let party = plan
            .anchors
            .get(PARTY_START)
            .expect("sky islands should publish the party anchor");
        plan.gated.insert(party.coord);
        if let Some(cell) = plan.cells.get_mut(&party.coord) {
            cell.gated = true;
        }

        let map = voxelize(&plan, settings.environment, &palette());
        let validation = validate_exact(&plan, &map, &palette(), WALKER, &solid);

        assert!(!validation.valid);
        assert!(
            validation
                .notes
                .iter()
                .any(|note| note.contains("anchor belongs")),
            "{:?}",
            validation.notes
        );
    }

    #[test]
    fn seed_808_sky_slope_projection_leaves_later_repairs_viable() {
        let settings = sky();
        let mut plan = construct_plan(12, &settings, 808, 0, false);
        let deck_only_before: BTreeMap<HexCoord, PlannedCell> = plan
            .cells
            .iter()
            .filter(|(_, cell)| cell.foundation == Foundation::None)
            .map(|(coord, cell)| (*coord, *cell))
            .collect();
        assert!(
            !deck_only_before.is_empty(),
            "the regression requires bridge cells over empty space"
        );

        let before_projection = plan.clone();
        repair_plan(&mut plan, 1);
        let changed = plan
            .cells
            .iter()
            .filter(|(coord, cell)| before_projection.cells.get(*coord) != Some(*cell))
            .count();
        let maximum_local_repair = plan.cells.len().saturating_div(20).max(12);
        assert!(
            changed <= maximum_local_repair,
            "slope projection changed {changed} cells; local limit is {maximum_local_repair}"
        );
        for (coord, expected) in deck_only_before {
            assert_eq!(
                plan.cells.get(&coord),
                Some(&expected),
                "deck-only cell {coord:?} participated in slope projection"
            );
        }

        repair_plan(&mut plan, 2);
        synchronize_anchor_levels(&mut plan);
        let map = voxelize(&plan, settings.environment, &palette());
        let validation = validate_exact(&plan, &map, &palette(), WALKER, &solid);
        assert!(
            validation.valid,
            "the later crossing repair should remain usable: {:?}",
            validation.notes
        );
    }

    #[test]
    fn canonical_fallbacks_pass_final_validation() {
        for settings in [
            hills(EnvironmentSettings::TemperateGrassland),
            hills(EnvironmentSettings::Frozen),
            hills(EnvironmentSettings::Volcanic),
            sky(),
        ] {
            let mut plan = construct_plan(12, &settings, 55, 0, true);
            let (_map, validation, _) =
                voxelize_validate_repair(&mut plan, &settings, 55, 0, &palette(), WALKER, &solid);
            assert!(validation.valid, "{:?}", validation.notes);
        }
    }

    #[test]
    fn forced_fallback_is_valid_and_reported() {
        let settings = hills(EnvironmentSettings::TemperateGrassland);
        let result =
            build_with_candidate_selection(12, &settings, 505, &palette(), WALKER, &solid, false);
        assert!(result.validated, "{:?}", result.report.notes);
        assert!(result.report.used_fallback);
        assert_eq!(result.report.selected_candidate, None);
        assert_eq!(result.report.candidates_evaluated, CANDIDATE_COUNT);
        assert!(result.report.valid_candidates > 0);
        assert!(result
            .report
            .notes
            .iter()
            .any(|note| note.contains("canonical fallback selected")));
    }

    #[test]
    fn representative_radius_12_20_40_corpus_is_valid() {
        let settings = hills(EnvironmentSettings::TemperateGrassland);
        for (radius, seed) in [(12, 12), (20, 20), (40, 40)] {
            let result = build(radius, &settings, seed, &palette(), WALKER, &solid);
            assert!(
                result.validated,
                "radius {radius}: {:?}",
                result.report.notes
            );
            assert_eq!(result.report.candidates_evaluated, CANDIDATE_COUNT);
        }
    }

    #[test]
    fn different_seeds_explore_different_maps() {
        let settings = hills(EnvironmentSettings::TemperateGrassland);
        let first = build(12, &settings, 1, &palette(), WALKER, &solid);
        let second = build(12, &settings, 2, &palette(), WALKER, &solid);
        assert_ne!(first.report.map_fingerprint, second.report.map_fingerprint);
    }

    #[test]
    fn fixed_seed_corpus_never_needs_fallback() {
        let settings = hills(EnvironmentSettings::TemperateGrassland);
        for seed in 0..32 {
            let result = build(12, &settings, seed, &palette(), WALKER, &solid);
            assert!(
                !result.report.used_fallback,
                "seed {seed} used fallback: {:?}",
                result.report.notes
            );
        }
    }

    #[test]
    fn named_regression_corpus_is_valid_and_deterministic() {
        let settings = hills(EnvironmentSettings::TemperateGrassland);
        for (label, seed) in FIXED_REGRESSION_SEEDS {
            let first = build(12, &settings, seed, &palette(), WALKER, &solid);
            let second = build(12, &settings, seed, &palette(), WALKER, &solid);

            assert!(
                first.validated,
                "{label} seed {seed}: {:?}",
                first.report.notes
            );
            assert!(
                !first.report.used_fallback,
                "{label} seed {seed} used fallback: {:?}",
                first.report.notes
            );
            assert_eq!(
                first.report.map_fingerprint, second.report.map_fingerprint,
                "{label} seed {seed} changed map fingerprint"
            );
            assert_eq!(
                first.report.selected_candidate, second.report.selected_candidate,
                "{label} seed {seed} changed selected candidate"
            );
            assert_eq!(
                first.anchors.iter().collect::<BTreeMap<_, _>>(),
                second.anchors.iter().collect::<BTreeMap<_, _>>(),
                "{label} seed {seed} changed generated anchors"
            );
        }
    }

    #[test]
    #[ignore = "10,000 seeds are a manual stress corpus"]
    fn ten_thousand_seed_corpus_has_less_than_one_percent_fallbacks() {
        let settings = hills(EnvironmentSettings::TemperateGrassland);
        let started = Instant::now();
        let mut fallbacks = 0;
        let mut invalid = 0;
        let mut fingerprints = BTreeSet::new();
        let mut repair_distribution = BTreeMap::<u8, usize>::new();
        let mut relief_range = (Level::MAX, Level::MIN);
        let mut detour_range = (u32::MAX, u32::MIN);
        let mut sinuosity_range = (u32::MAX, u32::MIN);
        let mut slowest_micros = 0;

        for seed in 0..10_000 {
            let result = build(12, &settings, seed, &palette(), WALKER, &solid);
            fallbacks += usize::from(result.report.used_fallback);
            invalid += usize::from(!result.validated);
            fingerprints.insert(result.report.map_fingerprint);
            *repair_distribution
                .entry(result.report.repair_rounds)
                .or_default() += 1;
            relief_range.0 = relief_range.0.min(result.report.metrics.relief);
            relief_range.1 = relief_range.1.max(result.report.metrics.relief);
            detour_range.0 = detour_range
                .0
                .min(result.report.metrics.alternate_detour_percent);
            detour_range.1 = detour_range
                .1
                .max(result.report.metrics.alternate_detour_percent);
            sinuosity_range.0 = sinuosity_range
                .0
                .min(result.report.metrics.river_sinuosity_percent);
            sinuosity_range.1 = sinuosity_range
                .1
                .max(result.report.metrics.river_sinuosity_percent);
            slowest_micros = slowest_micros.max(result.report.elapsed_micros);
        }

        eprintln!(
            "10k hills: invalid={invalid}, fallbacks={fallbacks}, unique_fingerprints={}, \
             repairs={repair_distribution:?}, relief={relief_range:?}, detour={detour_range:?}, \
             sinuosity={sinuosity_range:?}, slowest={}us, wall={}ms",
            fingerprints.len(),
            slowest_micros,
            started.elapsed().as_millis()
        );

        assert_eq!(invalid, 0, "{invalid} final maps failed hard validation");
        assert!(fallbacks < 100, "{fallbacks} of 10,000 seeds used fallback");
    }

    #[test]
    #[ignore = "manual release/debug generator benchmark"]
    fn procedural_radius_benchmark_meets_the_radius_40_target() {
        let settings = hills(EnvironmentSettings::TemperateGrassland);
        let mut radius_40_worst = 0;

        for radius in [12, 20, 40] {
            let mut samples = Vec::new();
            for seed in 0..12 {
                let result = build(radius, &settings, seed, &palette(), WALKER, &solid);
                assert!(result.validated, "radius {radius}, seed {seed}");
                samples.push(result.report.elapsed_micros);
            }
            samples.sort_unstable();
            let median = samples.get(samples.len() / 2).copied().unwrap_or(u64::MAX);
            let worst = samples.last().copied().unwrap_or(u64::MAX);
            eprintln!("radius {radius}: median={median}us worst={worst}us");
            if radius == 40 {
                radius_40_worst = worst;
            }
        }

        let started = Instant::now();
        let plan = construct_plan(40, &settings, 0, 0, false);
        let planned_micros = started.elapsed().as_micros();
        let started = Instant::now();
        let map = voxelize(&plan, settings.environment, &palette());
        let voxelized_micros = started.elapsed().as_micros();
        let started = Instant::now();
        let validation = validate_exact(&plan, &map, &palette(), WALKER, &solid);
        let validated_micros = started.elapsed().as_micros();
        assert!(validation.valid, "{:?}", validation.notes);
        eprintln!(
            "radius 40 candidate breakdown: plan={planned_micros}us, \
             voxelize={voxelized_micros}us, validate={validated_micros}us"
        );

        let target_micros = if cfg!(debug_assertions) {
            250_000
        } else {
            50_000
        };
        assert!(
            radius_40_worst < target_micros,
            "radius 40 worst case was {radius_40_worst}us; target is {target_micros}us"
        );
    }
}
