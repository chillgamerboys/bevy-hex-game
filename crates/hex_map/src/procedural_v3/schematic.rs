//! Grand V3 schematic-to-map compilation.
//!
//! The checkpoint compiler deliberately builds one continuous, undecorated world.
//! Coarse cells retain stable biome ownership, while height and liquid geometry are
//! resolved globally so their borders never become visible patch seams.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_core::{HexCoord, Level, MapViewHint, SpecialMovementRegion, TilePos};
use hex_schematic::{
    AccessIntent, CellPlan, FeatureKind as SchematicFeature, GeneratedSchematic, LandformKind,
    NetworkKind, SchematicCoord, SchematicPlanV1, SurfaceKind,
};

use super::fingerprint::semantic_plan_fingerprint;
use super::layout::{resolve_layout, LayoutKind, PatchId, ResolvedLayoutPlan};
use super::liquid::{LiquidBodyId, LiquidBodyPlan, LiquidFlowState, LiquidNode, LiquidPlan};
use super::selection::{CandidateNote, ValidatedWorldPlan, ValidatedWorldSelection};
use super::volume::{
    FillMaterialRole, LevelInterval, NonSolidFill, SolidMass, SolidMaterialRole, SurfaceAccess,
    SurfaceMetadata, VolumeColumn, VolumeElement, VolumePlan,
};
use super::world::{
    FeaturePlan, GeneratedWorldPlan, InteriorPlan, StructurePlan, WorldIssueCode,
    WorldValidationIssue,
};
use super::V3GenerationError;
use crate::settings::{
    ProceduralV3Settings, V3GrandV3BasicTerrainProfile, V3LayoutSettings,
    V3SchematicLayoutSettings, V3SchematicTemplate, V3SchematicTerrainProfile,
    V3_GRAND_V3_TEMPLATE_REVISION, V3_SCHEMATIC_GRID_RADIUS,
};

const WORLD_NAMESPACE: u32 = 255 << 24;
const SCENIC_MOVEMENT_REGION: SpecialMovementRegion = SpecialMovementRegion(WORLD_NAMESPACE | 1);
const INACCESSIBLE_MOVEMENT_REGION: SpecialMovementRegion =
    SpecialMovementRegion(WORLD_NAMESPACE | 2);

/// Deterministic measurements retained at the first performance checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SchematicWorldMetrics {
    pub(crate) schematic_cells: u32,
    pub(crate) world_columns: u32,
    pub(crate) expected_chunks: u32,
    pub(crate) ordinary_surfaces: u32,
    pub(crate) water_columns: u32,
    pub(crate) liquid_bodies: u32,
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
    compile_generated_schematic(generated, settings, schematic, grid_radius, level_height)
}

/// Compiles an exact generated or reference plan after replaying its complete
/// schematic validity contract. Runtime generation uses [`generate`] so the
/// 32-candidate schematic selection itself is never repeated.
pub(crate) fn compile_schematic(
    plan: &SchematicPlanV1,
    settings: &ProceduralV3Settings,
    grid_radius: u32,
    level_height: f32,
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
    )
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
) -> Result<ValidatedWorldSelection<SchematicWorldMetrics>, V3GenerationError> {
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
    let layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    let (world, metrics) = build_proxy_world(&plan, layout, profile, level_height)?;
    let issues = world.validate();
    if !issues.is_empty() {
        return Err(V3GenerationError::InvalidFallback(issues));
    }
    let semantic_fingerprint =
        semantic_plan_fingerprint(&world).map_err(V3GenerationError::Fingerprint)?;
    let provenance = plan.provenance;
    Ok(ValidatedWorldSelection {
        validated: ValidatedWorldPlan {
            plan: world,
            semantic_fingerprint,
        },
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

fn build_proxy_world(
    plan: &SchematicPlanV1,
    layout: ResolvedLayoutPlan,
    profile: V3GrandV3BasicTerrainProfile,
    level_height: f32,
) -> Result<(GeneratedWorldPlan, SchematicWorldMetrics), V3GenerationError> {
    if layout.kind != LayoutKind::Schematic
        || layout.patches.len() != hex_schematic::SCHEMATIC_CELL_COUNT
        || layout.footprint.len() != 105_469
    {
        return Err(V3GenerationError::RecipeContract(
            "resolved schematic ownership is not the exact 217-cell radius-187 contract".to_owned(),
        ));
    }

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

    let mut volume = VolumePlan::new(layout.footprint.clone());
    let mut biome_regions = BTreeMap::new();
    let mut liquid_positions = BTreeMap::<Level, BTreeSet<TilePos>>::new();
    let mut minimum_surface = Level::MAX;
    let mut maximum_surface = Level::MIN;
    let mut ordinary_surfaces = 0_u32;
    let mut water_columns = 0_u32;

    for (patch_id, patch) in &layout.patches {
        let cell = cells.get(patch_id).copied().ok_or_else(|| {
            V3GenerationError::RecipeContract(format!(
                "schematic patch {} has no canonical cell",
                patch_id.0
            ))
        })?;
        let center = centers.get(patch_id).copied().ok_or_else(|| {
            V3GenerationError::RecipeContract(format!(
                "schematic patch {} has no canonical center",
                patch_id.0
            ))
        })?;
        for coord in &patch.mask {
            let (column, surface, access, water_level) = if cell.facts.surface
                == SurfaceKind::OpenWater
            {
                let water = water_level(cell, profile);
                let bed = water_bed_level(cell, *coord, water, world_seed);
                (
                    water_column(bed, water, water_bed_material(cell)),
                    TilePos::new(*coord, bed),
                    SurfaceAccess::NonStandable,
                    Some(water),
                )
            } else {
                let surface_level = fine_surface_level(
                    cell,
                    *coord,
                    center,
                    &centers,
                    &coarse_datums,
                    &coarse_relief_caps,
                    profile,
                    world_seed,
                );
                let access = match cell.facts.access {
                    AccessIntent::Ordinary => SurfaceAccess::Ordinary,
                    AccessIntent::Scenic => SurfaceAccess::SpecialMovement(SCENIC_MOVEMENT_REGION),
                    AccessIntent::Inaccessible => {
                        SurfaceAccess::SpecialMovement(INACCESSIBLE_MOVEMENT_REGION)
                    }
                };
                (
                    land_column(surface_level, land_cap_material(cell)),
                    TilePos::new(*coord, surface_level),
                    access,
                    None,
                )
            };
            minimum_surface = minimum_surface.min(surface.level);
            maximum_surface = maximum_surface.max(surface.level);
            ordinary_surfaces =
                ordinary_surfaces.saturating_add(u32::from(access == SurfaceAccess::Ordinary));
            if let Some(level) = water_level {
                water_columns = water_columns.saturating_add(1);
                liquid_positions
                    .entry(level)
                    .or_default()
                    .insert(TilePos::new(*coord, level));
            }
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

    let liquids = liquid_components(liquid_positions);
    let ordinary = volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| {
            (metadata.access == SurfaceAccess::Ordinary).then_some(*position)
        })
        .collect::<BTreeSet<_>>();
    let anchors = review_anchors(plan, &ordinary, &centers)?;
    let view_hint = schematic_view_hint(level_height, maximum_surface);
    let world = GeneratedWorldPlan {
        source_schematic_fingerprint: Some(plan.semantic_fingerprint),
        layout,
        volume,
        liquids,
        features: FeaturePlan::default(),
        structures: StructurePlan::default(),
        blockers: BTreeSet::new(),
        lights: BTreeMap::new(),
        biome_regions,
        interiors: InteriorPlan::default(),
        anchors,
        view_hint,
    };
    let metrics = SchematicWorldMetrics {
        schematic_cells: u32::try_from(plan.cells.len()).unwrap_or(u32::MAX),
        world_columns: u32::try_from(world.layout.footprint.len()).unwrap_or(u32::MAX),
        expected_chunks: 444,
        ordinary_surfaces,
        water_columns,
        liquid_bodies: u32::try_from(world.liquids.bodies.len()).unwrap_or(u32::MAX),
        minimum_surface,
        maximum_surface,
        schematic_fingerprint: plan.semantic_fingerprint,
    };
    Ok((world, metrics))
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
    center: HexCoord,
    centers: &BTreeMap<PatchId, HexCoord>,
    datums: &BTreeMap<PatchId, Level>,
    relief_caps: &BTreeMap<PatchId, Level>,
    profile: V3GrandV3BasicTerrainProfile,
    seed: u64,
) -> Level {
    let local_distance = center.distance(coord);
    if has_overlay(cell, SchematicFeature::FrozenWoods) {
        return profile.frozen_woods_level;
    }
    if has_overlay(cell, SchematicFeature::CrystalAscent) {
        return profile
            .crystal_base_level
            .saturating_add(profile.crystal_rise_levels);
    }
    if has_overlay(cell, SchematicFeature::LakeIsland) {
        let span = profile
            .lake_island_max_level
            .saturating_sub(profile.lake_island_min_level)
            .saturating_add(1);
        return profile.lake_island_min_level.saturating_add(
            i32::try_from(
                named_sample(seed, "lake_island", coord) % u64::try_from(span).unwrap_or(1),
            )
            .unwrap_or_default(),
        );
    }
    if cell.facts.landform == LandformKind::SharpPeak {
        let ordinal = u64::from(cell.id.get());
        let summit_span = profile
            .sharp_peak_max
            .saturating_sub(profile.sharp_peak_min)
            .saturating_add(1);
        let summit = profile.sharp_peak_min.saturating_add(
            i32::try_from(
                named_sample(seed ^ ordinal, "sharp_peak", center)
                    % u64::try_from(summit_span).unwrap_or(1),
            )
            .unwrap_or_default(),
        );
        let bench_span = profile
            .sharp_peak_bench_max
            .saturating_sub(profile.sharp_peak_bench_min)
            .saturating_add(1);
        let bench = profile.sharp_peak_bench_min.saturating_add(
            i32::try_from(
                named_sample(seed ^ ordinal, "sharp_bench", center)
                    % u64::try_from(bench_span).unwrap_or(1),
            )
            .unwrap_or_default(),
        );
        return summit
            .saturating_sub(
                i32::try_from(local_distance)
                    .unwrap_or(i32::MAX)
                    .saturating_mul(3),
            )
            .max(bench);
    }

    let owner = PatchId(u32::from(cell.id.get()));
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
    blended.saturating_add(smooth_relief(coord, cap, seed))
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
    elements.push(VolumeElement::Solid(SolidMass {
        levels: LevelInterval::new(surface, surface.saturating_add(1)),
        material: cap,
        cutaway_for: None,
    }));
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

fn liquid_components(by_level: BTreeMap<Level, BTreeSet<TilePos>>) -> LiquidPlan {
    let mut bodies = BTreeMap::new();
    let mut ordinal = 0_u32;
    for (_level, positions) in by_level {
        let mut remaining = positions;
        while let Some(start) = remaining.first().copied() {
            remaining.remove(&start);
            let mut component = BTreeSet::from([start]);
            let mut frontier = VecDeque::from([start]);
            while let Some(position) = frontier.pop_front() {
                for neighbor in position
                    .coord
                    .neighbors()
                    .map(|coord| TilePos::new(coord, position.level))
                {
                    if remaining.remove(&neighbor) {
                        component.insert(neighbor);
                        frontier.push_back(neighbor);
                    }
                }
            }
            let nodes = component
                .into_iter()
                .map(|position| {
                    (
                        position,
                        LiquidNode {
                            state: LiquidFlowState::Still,
                            downstream: None,
                        },
                    )
                })
                .collect();
            bodies.insert(
                LiquidBodyId(WORLD_NAMESPACE | ordinal),
                LiquidBodyPlan {
                    material: FillMaterialRole::Water,
                    nodes,
                },
            );
            ordinal = ordinal.saturating_add(1);
        }
    }
    LiquidPlan { bodies }
}

fn review_anchors(
    plan: &SchematicPlanV1,
    ordinary: &BTreeSet<TilePos>,
    centers: &BTreeMap<PatchId, HexCoord>,
) -> Result<BTreeMap<String, TilePos>, V3GenerationError> {
    let fallback = ordinary.iter().next().copied().ok_or_else(|| {
        V3GenerationError::InvalidFallback(vec![WorldValidationIssue::new(
            WorldIssueCode::Anchor,
            "schematic proxy contains no ordinary surfaces",
        )])
    })?;
    let feature_center = |feature: SchematicFeature| {
        plan.cells
            .iter()
            .find(|cell| has_overlay(cell, feature))
            .and_then(|cell| centers.get(&PatchId(u32::from(cell.id.get()))).copied())
    };
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
                "schematic proxy is missing the exact hydrology sea-mouth node",
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
    let mut anchors = BTreeMap::from([
        ("party_start".to_owned(), party),
        ("hostile_start".to_owned(), hostile),
        ("conflict_center".to_owned(), nearest(HexCoord::ORIGIN)),
        ("proxy.coast_overlook".to_owned(), nearest(hydrology_sink)),
    ]);
    for (name, feature) in [
        ("proxy.archipelago_overlook", SchematicFeature::SeaIsland),
        (
            "proxy.mountain_lake_overlook",
            SchematicFeature::MountainLake,
        ),
        ("proxy.frozen_woods_overlook", SchematicFeature::FrozenWoods),
        ("proxy.peak_overlook", SchematicFeature::PeakRing),
        (
            "proxy.crystal_site_overlook",
            SchematicFeature::CrystalAscent,
        ),
    ] {
        if let Some(target) = feature_center(feature) {
            anchors.insert(name.to_owned(), nearest(target));
        }
    }
    for (name, kind, id) in [
        (
            "proxy.mountain_lake_source_overlook",
            NetworkKind::Hydrology,
            "node/hydrology-mountain-lake",
        ),
        (
            "proxy.waterfall_site_overlook",
            NetworkKind::Hydrology,
            "node/hydrology-waterfall",
        ),
        (
            "proxy.valley_lake_overlook",
            NetworkKind::Hydrology,
            "node/hydrology-valley-lake",
        ),
        (
            "proxy.tunnel_ascent_terminal_overlook",
            NetworkKind::Tunnel,
            "node/tunnel-ascent",
        ),
        (
            "proxy.tunnel_hill_terminal_overlook",
            NetworkKind::Tunnel,
            "node/tunnel-hill-terminal",
        ),
    ] {
        let target = network_node(kind, id).ok_or_else(|| {
            V3GenerationError::InvalidFallback(vec![WorldValidationIssue::new(
                WorldIssueCode::Anchor,
                format!("schematic proxy is missing exact review node {id}"),
            )])
        })?;
        anchors.insert(name.to_owned(), nearest(target));
    }
    if let Some(massif) = plan
        .cells
        .iter()
        .find(|cell| cell.facts.landform == LandformKind::Massif)
        .and_then(|cell| centers.get(&PatchId(u32::from(cell.id.get()))))
        .copied()
    {
        anchors.insert("proxy.massif_overlook".to_owned(), nearest(massif));
    }
    Ok(anchors)
}

fn schematic_view_hint(level_height: f32, maximum_surface: Level) -> MapViewHint {
    let eye_coord = HexCoord::from_axial(-187, 96);
    let eye = eye_coord.to_world(
        f32::from(i16::try_from(maximum_surface.saturating_add(80)).unwrap_or(i16::MAX))
            * level_height,
    );
    let focus = HexCoord::ORIGIN
        .to_world(f32::from(i16::try_from(maximum_surface / 3).unwrap_or(i16::MAX)) * level_height);
    MapViewHint::new((eye.x, eye.y, eye.z), (focus.x, focus.y, focus.z))
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
            let selection =
                compile_schematic(&reference.plan, &settings(), V3_SCHEMATIC_GRID_RADIUS, 0.4)
                    .expect("reference compiles");
            ReferenceFixture {
                plan: reference.plan,
                selection,
            }
        })
    }

    fn surface_level_at(world: &GeneratedWorldPlan, coord: HexCoord) -> Level {
        let mut matches = world
            .volume
            .surfaces
            .keys()
            .filter(|position| position.coord == coord);
        let level = matches
            .next()
            .unwrap_or_else(|| panic!("{coord:?} has no projected surface"))
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
        assert!((178..=192).contains(&selection.metrics.maximum_surface));
        assert!(selection.validated.plan.validate().is_empty());
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

        assert_eq!(surface_level_at(world, world_coord(1, -6)), 150);
        for frozen_woods in [(2, -6), (3, -6), (2, -7), (3, -7)] {
            assert_eq!(
                surface_level_at(world, world_coord(frozen_woods.0, frozen_woods.1)),
                152
            );
        }
        assert!((151..=158).contains(&surface_level_at(world, world_coord(4, -5))));
        assert!((178..=192).contains(&surface_level_at(world, world_coord(3, -3))));
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
                let delta = levels[coord].abs_diff(levels[&neighbor]);
                checked_pairs = checked_pairs.saturating_add(1);
                if worst.is_none_or(|(current, ..)| delta > current) {
                    worst = Some((delta, *coord, neighbor, *owner, *neighbor_owner));
                }
            }
        }

        assert!(
            checked_pairs >= 9_000,
            "smoothness exclusions left only {checked_pairs} coarse-boundary pairs"
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
