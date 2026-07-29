//! Native V3 rolling-hills geometry.
//!
//! Height cones are one-Lipschitz, so the generated ordinary surface remains
//! walker-connected by construction. Shared-edge approaches clamp those cones to
//! the resolved seam datum without a post-generation blend pass.

use std::collections::{BTreeMap, BTreeSet};

use hex_core::{HexCoord, Level, MapViewHint, TilePos};

use super::layout::{resolve_layout, PatchId, ResolvedLayoutPlan};
use super::patch::PatchRecipeContext;
use super::seed::SeedStream;
use super::selection::{
    run_recipe, CandidateAttemptError, CandidateContext, FallbackContext, RepairOutcome, V3Recipe,
    ValidatedWorldSelection, WorldValidation,
};
use super::traversal::OrdinaryGraph;
use super::volume::{
    LevelInterval, SolidMass, SolidMaterialRole, SurfaceAccess, SurfaceMetadata, VolumeColumn,
    VolumeElement, VolumePlan,
};
use super::world::{
    FeaturePlan, GeneratedWorldPlan, InteriorPlan, StructurePlan, WorldIssueCode,
    WorldValidationIssue,
};
use super::V3GenerationError;
use crate::settings::{
    ProceduralV3Settings, V3EnvironmentSettings, V3HillsSettings, V3LayoutSettings,
    V3RecipeSettings,
};

const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";
const CONFLICT_CENTER: &str = "conflict_center";

/// Deterministic measurements for one admitted Hills plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HillsMetrics {
    pub(crate) ordinary_surfaces: u32,
    pub(crate) reachable_elevation_levels: u32,
    pub(crate) relief: Level,
    pub(crate) critical_route_steps: u32,
    pub(crate) hill_centres: u32,
}

#[derive(Debug)]
struct HillsRecipe {
    level_height: f32,
    layout: ResolvedLayoutPlan,
    settings: V3HillsSettings,
    environment: V3EnvironmentSettings,
}

/// Runs the common eight-candidate selector for one native V3 Hills world.
pub(crate) fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
) -> Result<ValidatedWorldSelection<HillsMetrics>, V3GenerationError> {
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V3GenerationError::RecipeContract(
            "Hills level height must be positive and finite".to_owned(),
        ));
    }
    let (hills, environment) = recipe_settings(settings)?;
    let layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    run_recipe(
        &HillsRecipe {
            level_height,
            layout,
            settings: hills.clone(),
            environment,
        },
        settings,
        grid_radius,
        seed,
    )
}

impl V3Recipe for HillsRecipe {
    type Settings = ProceduralV3Settings;
    type Metrics = HillsMetrics;
    type Score = (u32, u32, u8);

    fn construct(
        &self,
        context: CandidateContext,
        _settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, CandidateAttemptError> {
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))
            .map_err(CandidateAttemptError::Fatal)?;
        let streams = patch.seed_streams(context.seed, context.candidate);
        construct_plan(
            self.layout.clone(),
            PatchId(0),
            &self.settings,
            self.environment,
            self.level_height,
            Some((
                streams.stage("hills.orientation"),
                streams.stage("hills.centres"),
            )),
        )
        .map_err(CandidateAttemptError::Rejected)
    }

    fn validate(
        &self,
        _settings: &Self::Settings,
        plan: &GeneratedWorldPlan,
    ) -> WorldValidation<Self::Metrics> {
        validate_hills(plan, &self.settings)
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
            metrics.relief.abs_diff(self.settings.max_relief),
            metrics
                .hill_centres
                .abs_diff(u32::from(self.settings.hills_per_bank).saturating_mul(2)),
            candidate,
        )
    }

    fn canonical_fallback(
        &self,
        context: FallbackContext,
        _settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, V3GenerationError> {
        if context.grid_radius != self.layout.grid_radius {
            return Err(V3GenerationError::RecipeContract(
                "Hills fallback radius disagrees with its resolved layout".to_owned(),
            ));
        }
        construct_plan(
            self.layout.clone(),
            PatchId(0),
            &self.settings,
            self.environment,
            self.level_height,
            None,
        )
        .map_err(|issues| {
            V3GenerationError::RecipeContract(
                issues
                    .into_iter()
                    .map(|issue| issue.detail)
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        })
    }
}

fn recipe_settings(
    settings: &ProceduralV3Settings,
) -> Result<(&V3HillsSettings, V3EnvironmentSettings), V3GenerationError> {
    let V3LayoutSettings::Single(patch) = &settings.layout else {
        return Err(V3GenerationError::RecipeUnavailable("Ring7"));
    };
    let V3RecipeSettings::Hills(hills) = &patch.recipe else {
        return Err(V3GenerationError::RecipeUnavailable(recipe_name(
            &patch.recipe,
        )));
    };
    if patch.environment == V3EnvironmentSettings::Rocky {
        return Err(V3GenerationError::RecipeContract(
            "Hills does not support the Rocky environment".to_owned(),
        ));
    }
    if !patch.overlays.is_empty() {
        return Err(V3GenerationError::RecipeContract(
            "Hills overlays are not implemented yet".to_owned(),
        ));
    }
    Ok((hills, patch.environment))
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
    }
}

pub(crate) fn construct_plan(
    layout: ResolvedLayoutPlan,
    patch_id: PatchId,
    settings: &V3HillsSettings,
    environment: V3EnvironmentSettings,
    level_height: f32,
    streams: Option<(SeedStream<'_>, SeedStream<'_>)>,
) -> Result<GeneratedWorldPlan, Vec<WorldValidationIssue>> {
    let patch = PatchRecipeContext::resolve(&layout, patch_id)
        .map_err(|error| vec![recipe_issue(error.to_string())])?;
    let orientation = streams.map_or(0, |(orientation, _)| {
        u8::try_from(orientation.sample(0) % 6).unwrap_or_default()
    });
    let centre_stream = streams.map(|(_, centres)| centres);
    let centres = select_hill_centres(
        patch.mask(),
        settings.hills_per_bank,
        orientation,
        centre_stream,
        &patch.protected_approaches(),
    )?;
    let mut surface_by_coord = BTreeMap::new();
    for coord in patch.mask() {
        let rise = centres
            .iter()
            .map(|centre| {
                settings
                    .max_relief
                    .saturating_sub(i32::try_from(centre.distance(*coord)).unwrap_or(i32::MAX))
                    .max(0)
            })
            .max()
            .unwrap_or_default();
        surface_by_coord.insert(*coord, settings.valley_level.saturating_add(rise));
    }
    fit_shared_approaches(&patch, &mut surface_by_coord);

    let mut columns = BTreeMap::new();
    let mut surfaces = BTreeMap::new();
    for (coord, level) in &surface_by_coord {
        columns.insert(*coord, land_column(*level, environment));
        surfaces.insert(
            TilePos::new(*coord, *level),
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: None,
            },
        );
    }
    let volume = VolumePlan {
        mask: patch.mask().clone(),
        columns,
        surfaces,
    };
    let (party_coord, hostile_coord) = opposing_landings(patch.mask(), orientation)?;
    let conflict_coord = patch
        .mask()
        .iter()
        .copied()
        .min_by_key(|coord| {
            (
                coord
                    .distance(party_coord)
                    .abs_diff(coord.distance(hostile_coord)),
                coord.distance(HexCoord::ORIGIN),
                *coord,
            )
        })
        .ok_or_else(|| vec![recipe_issue("Hills patch has no conflict landing")])?;
    let exact = |coord| {
        surface_by_coord
            .get(&coord)
            .copied()
            .map(|level| TilePos::new(coord, level))
    };
    let anchors = BTreeMap::from([
        (
            PARTY_START.to_owned(),
            exact(party_coord)
                .ok_or_else(|| vec![recipe_issue("Hills party landing is missing")])?,
        ),
        (
            HOSTILE_START.to_owned(),
            exact(hostile_coord)
                .ok_or_else(|| vec![recipe_issue("Hills hostile landing is missing")])?,
        ),
        (
            CONFLICT_CENTER.to_owned(),
            exact(conflict_coord)
                .ok_or_else(|| vec![recipe_issue("Hills conflict landing is missing")])?,
        ),
    ]);
    let biome_regions = volume
        .surfaces
        .keys()
        .copied()
        .map(|surface| (surface, patch.biome_region()))
        .collect();
    let view_hint = hills_view_hint(
        patch.mask(),
        &surface_by_coord,
        layout.grid_radius,
        level_height,
    )?;

    Ok(GeneratedWorldPlan {
        layout,
        volume,
        liquids: Default::default(),
        features: FeaturePlan::default(),
        structures: StructurePlan::default(),
        blockers: BTreeSet::new(),
        lights: BTreeMap::new(),
        biome_regions,
        interiors: InteriorPlan::default(),
        anchors,
        view_hint,
    })
}

fn select_hill_centres(
    mask: &BTreeSet<HexCoord>,
    per_bank: u8,
    orientation: u8,
    stream: Option<SeedStream<'_>>,
    excluded: &BTreeSet<HexCoord>,
) -> Result<Vec<HexCoord>, Vec<WorldValidationIssue>> {
    let mut selected = Vec::new();
    for bank in [-1_i32, 1_i32] {
        let mut candidates: Vec<_> = mask
            .iter()
            .copied()
            .filter(|coord| axis_value(*coord, orientation).signum() == bank)
            .filter(|coord| !excluded.contains(coord))
            .collect();
        candidates.sort_by_key(|coord| {
            (
                stream.map_or(0, |stream| {
                    stream.sample_coord(*coord, bank.unsigned_abs().into())
                }),
                *coord,
            )
        });
        for coord in candidates {
            if selected
                .iter()
                .all(|centre: &HexCoord| centre.distance(coord) >= 3)
            {
                selected.push(coord);
                if selected
                    .iter()
                    .filter(|centre| axis_value(**centre, orientation).signum() == bank)
                    .count()
                    == usize::from(per_bank)
                {
                    break;
                }
            }
        }
    }
    let expected = usize::from(per_bank).saturating_mul(2);
    if selected.len() != expected {
        return Err(vec![recipe_issue(format!(
            "Hills patch placed {} separated centres; expected {expected}",
            selected.len()
        ))]);
    }
    Ok(selected)
}

fn fit_shared_approaches(patch: &PatchRecipeContext<'_>, levels: &mut BTreeMap<HexCoord, Level>) {
    for edge in patch.shared_edges() {
        let preferred = edge.preferred_level();
        let approaches = edge.protected_approaches();
        for coord in patch.mask() {
            let distance = approaches
                .iter()
                .map(|approach| approach.distance(*coord))
                .min()
                .unwrap_or(u32::MAX);
            let distance = i32::try_from(distance).unwrap_or(i32::MAX);
            if let Some(level) = levels.get_mut(coord) {
                *level = (*level)
                    .min(preferred.saturating_add(distance))
                    .max(preferred.saturating_sub(distance));
            }
        }
    }
}

fn opposing_landings(
    mask: &BTreeSet<HexCoord>,
    orientation: u8,
) -> Result<(HexCoord, HexCoord), Vec<WorldValidationIssue>> {
    let party = mask
        .iter()
        .copied()
        .min_by_key(|coord| (axis_value(*coord, orientation), *coord));
    let hostile = mask
        .iter()
        .copied()
        .max_by_key(|coord| (axis_value(*coord, orientation), *coord));
    match (party, hostile) {
        (Some(party), Some(hostile)) if party != hostile => Ok((party, hostile)),
        _ => Err(vec![recipe_issue(
            "Hills patch cannot fit two opposing actor landings",
        )]),
    }
}

fn axis_value(coord: HexCoord, turns: u8) -> i32 {
    let [x, y, z] = rotate(coord, turns).to_cubic_array();
    x.saturating_sub(z).saturating_add(y / 2)
}

fn rotate(coord: HexCoord, turns: u8) -> HexCoord {
    let [mut x, mut y, mut z] = coord.to_cubic_array();
    for _ in 0..turns % 6 {
        (x, y, z) = (-z, -x, -y);
    }
    HexCoord::new_cubic(x, y, z)
}

fn land_column(surface: Level, environment: V3EnvironmentSettings) -> VolumeColumn {
    let surface_material = match environment {
        V3EnvironmentSettings::TemperateGrassland => SolidMaterialRole::Grass,
        V3EnvironmentSettings::Frozen => SolidMaterialRole::Snow,
        V3EnvironmentSettings::Volcanic => SolidMaterialRole::Basalt,
        V3EnvironmentSettings::Rocky => SolidMaterialRole::Stone,
    };
    let core_material = if environment == V3EnvironmentSettings::Volcanic {
        SolidMaterialRole::Basalt
    } else {
        SolidMaterialRole::Stone
    };
    VolumeColumn {
        elements: vec![
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(0, 1),
                material: SolidMaterialRole::Bedrock,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(1, surface.saturating_sub(3)),
                material: core_material,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(surface.saturating_sub(3), surface),
                material: SolidMaterialRole::Dirt,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(surface, surface.saturating_add(1)),
                material: surface_material,
                cutaway_for: None,
            }),
        ],
    }
}

fn validate_hills(
    plan: &GeneratedWorldPlan,
    settings: &V3HillsSettings,
) -> WorldValidation<HillsMetrics> {
    let mut issues = plan.validate();
    let ordinary = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
    let Some(party) = plan.anchors.get(PARTY_START).copied() else {
        issues.push(recipe_issue("Hills is missing party_start"));
        return WorldValidation::Invalid(issues);
    };
    let Some(hostile) = plan.anchors.get(HOSTILE_START).copied() else {
        issues.push(recipe_issue("Hills is missing hostile_start"));
        return WorldValidation::Invalid(issues);
    };
    let distances = ordinary.distances_from(party);
    if distances.len() != ordinary.len() {
        issues.push(recipe_issue(
            "Hills ordinary surfaces are not one walker-connected network",
        ));
    }
    let Some(critical_route_steps) = distances.get(&hostile).copied() else {
        issues.push(recipe_issue(
            "Hills actor anchors are not connected by ordinary movement",
        ));
        return WorldValidation::Invalid(issues);
    };
    let levels: BTreeSet<_> = ordinary
        .positions()
        .map(|position| position.level)
        .collect();
    let min = levels.iter().next().copied().unwrap_or_default();
    let max = levels.iter().next_back().copied().unwrap_or_default();
    let relief = max.saturating_sub(min);
    if relief > settings.max_relief {
        issues.push(recipe_issue(format!(
            "Hills relief {relief} exceeds configured maximum {}",
            settings.max_relief
        )));
    }
    validate_shared_approaches(plan, &ordinary, &mut issues);
    if !issues.is_empty() {
        return WorldValidation::Invalid(issues);
    }
    WorldValidation::Valid(HillsMetrics {
        ordinary_surfaces: count_u32(ordinary.len()),
        reachable_elevation_levels: count_u32(levels.len()),
        relief,
        critical_route_steps,
        hill_centres: u32::from(settings.hills_per_bank).saturating_mul(2),
    })
}

fn validate_shared_approaches(
    plan: &GeneratedWorldPlan,
    ordinary: &OrdinaryGraph,
    issues: &mut Vec<WorldValidationIssue>,
) {
    for edge in plan.layout.shared_edges.values() {
        for (patch, approaches) in &edge.protected_approaches {
            for coord in approaches {
                let Some(position) = plan
                    .volume
                    .surfaces
                    .keys()
                    .find(|position| position.coord == *coord)
                    .copied()
                else {
                    issues.push(recipe_issue(format!(
                        "Hills patch {} has no seam approach surface at {coord:?}",
                        patch.0
                    )));
                    continue;
                };
                if position.level != edge.elevation.preferred {
                    issues.push(recipe_issue(format!(
                        "Hills seam approach {position:?} does not use preferred level {}",
                        edge.elevation.preferred
                    )));
                }
                if !ordinary.contains(position) {
                    issues.push(recipe_issue(format!(
                        "Hills seam approach {position:?} is not ordinary footing"
                    )));
                }
            }
        }
    }
}

fn hills_view_hint(
    mask: &BTreeSet<HexCoord>,
    levels: &BTreeMap<HexCoord, Level>,
    grid_radius: u32,
    level_height: f32,
) -> Result<MapViewHint, Vec<WorldValidationIssue>> {
    let focus_level = if levels.is_empty() {
        0.0
    } else {
        let total = levels.values().try_fold(0.0_f32, |sum, level| {
            i16::try_from(*level)
                .map(|level| sum + f32::from(level))
                .map_err(|error| {
                    vec![recipe_issue(format!(
                        "Hills camera level does not fit inside i16: {error}"
                    ))]
                })
        })?;
        let count = f32::from(u16::try_from(levels.len()).map_err(|error| {
            vec![recipe_issue(format!(
                "Hills camera footprint does not fit inside u16: {error}"
            ))]
        })?);
        (total / count) * level_height
    };
    let radius = u16::try_from(grid_radius)
        .map_err(|error| vec![recipe_issue(format!("Hills camera radius: {error}"))])?;
    let frame = f32::from(radius).mul_add(3.2, 8.0);
    let center = mask
        .iter()
        .copied()
        .min_by_key(|coord| coord.distance(HexCoord::ORIGIN))
        .unwrap_or(HexCoord::ORIGIN);
    let horizontal_offset = f32::from(i16::try_from(center.x()).map_err(|error| {
        vec![recipe_issue(format!(
            "Hills camera coordinate does not fit inside i16: {error}"
        ))]
    })?) * 0.5;
    Ok(MapViewHint::new(
        (horizontal_offset, focus_level + frame * 0.75, frame * 0.8),
        (horizontal_offset, focus_level, 0.0),
    ))
}

fn recipe_issue(detail: impl Into<String>) -> WorldValidationIssue {
    WorldValidationIssue::new(WorldIssueCode::Recipe("hills"), detail)
}

fn count_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{
        PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings, PatchSpec,
    };

    fn world_boundaries() -> PatchEdgesSettings {
        PatchEdgesSettings {
            east: PatchEdgeContractSettings::WorldBoundary,
            south_east: PatchEdgeContractSettings::WorldBoundary,
            south_west: PatchEdgeContractSettings::WorldBoundary,
            west: PatchEdgeContractSettings::WorldBoundary,
            north_west: PatchEdgeContractSettings::WorldBoundary,
            north_east: PatchEdgeContractSettings::WorldBoundary,
        }
    }

    fn settings() -> ProceduralV3Settings {
        ProceduralV3Settings {
            layout: V3LayoutSettings::Single(PatchSpec {
                environment: V3EnvironmentSettings::TemperateGrassland,
                recipe: V3RecipeSettings::Hills(V3HillsSettings {
                    valley_level: 15,
                    max_relief: 8,
                    hills_per_bank: 3,
                }),
                overlays: Vec::new(),
                mask: PatchMaskSettings::WholeWorld,
                edges: world_boundaries(),
            }),
        }
    }

    #[test]
    fn native_hills_are_deterministic_connected_and_stratified() {
        let settings = settings();
        let first = generate(12, 0.4, &settings, 883).expect("valid Hills");
        let second = generate(12, 0.4, &settings, 883).expect("same valid Hills");
        assert_eq!(
            first.validated.semantic_fingerprint,
            second.validated.semantic_fingerprint
        );
        assert_eq!(first.metrics.ordinary_surfaces, 469);
        assert_eq!(first.metrics.hill_centres, 6);
        assert!(first.metrics.relief <= 8);
        assert!(first.metrics.reachable_elevation_levels >= 2);
    }

    #[test]
    fn rocky_hills_fail_instead_of_fabricating_a_plan() {
        let mut settings = settings();
        let V3LayoutSettings::Single(patch) = &mut settings.layout else {
            unreachable!("test uses Single")
        };
        patch.environment = V3EnvironmentSettings::Rocky;
        assert!(generate(12, 0.4, &settings, 1).is_err());
    }
}
