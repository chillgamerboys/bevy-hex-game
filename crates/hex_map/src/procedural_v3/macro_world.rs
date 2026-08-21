//! Authored radius-three Macro world construction.
//!
//! Macro cells resolve ownership and seams in [`super::layout`]. This runner then
//! invokes one deterministic terrain pass per logical biome instance, including
//! instances whose mask is the union of several atomic cells.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_assets::{HexObjectRotation, RuntimeArtCatalog};
use hex_core::{
    upper_dome_contains, ExactGridPoint, HexCoord, IlluminationLevel, Level, MapViewHint, TilePos,
};

use crate::procedural::{MacroMetrics, MountainRangeMetrics, OceanArchipelagoMetrics};
use crate::settings::{
    MacroAxisSettings, MacroBiomeInstanceSettings, MacroHeadwaterSettings, MacroLayoutSettings,
    MacroSpanningFeatureSettings, ProceduralV3Settings, V3CrystalAscentSettings, V3LayoutSettings,
    V3RecipeSettings, MAX_V3_LEVEL,
};

use super::composition::{
    compose_world, finalize_world, merge_world, GeneratedPatchPlan, PatchAnchorRef,
    WorldCompositionSettings,
};
use super::layout::{
    resolve_layout, resolve_macro_contracts, PatchId, ResolvedLayoutPlan, ResolvedLiquidElevation,
    ResolvedLiquidPort, ResolvedMacroContracts,
};
use super::liquid::{LiquidBodyId, LiquidBodyPlan, LiquidFlowState, LiquidNode, LiquidPlan};
use super::macro_spanning::{
    apply_macro_spanning, namespace_patch_local_interior, plan_macro_spanning,
    PlannedMacroSpanning, RawSpanningDestination, RawSpanningDestinations,
};
use super::patch::{PatchBuildMode, PatchRecipeContext};
use super::seam::shape_walker_seams;
use super::seed::SeedStreams;
use super::selection::{
    run_recipe, CandidateAttemptError, CandidateContext, FallbackContext, RepairOutcome, V3Recipe,
    ValidatedWorldSelection, WorldValidation,
};
use super::traversal::OrdinaryGraph;
use super::vegetation::{TemperateTreeSet, TemperateVegetationSet, VegetationObjectSpec};
use super::volume::{
    FillMaterialRole, LevelInterval, NonSolidFill, SolidMass, SolidMaterialRole, SurfaceAccess,
    SurfaceMetadata, VolumeColumn, VolumeElement, VolumePlan,
};
use super::world::{
    FeatureClearing, FeatureId, FeatureKind, FeaturePlan, GeneratedWorldPlan, InteriorPlan,
    PlannedFeature, PlannedLightPresentation, StructurePlan, WorldIssueCode, WorldValidationIssue,
};
use super::{CrystalAscentObjectSet, V3GenerationError};

const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";
const MACRO_ROUTE_END: &str = "macro_route_end";
const DEEP_MOUNTAIN_BASE: &str = "deep_mountain_base";
const COAST_REVIEW: &str = "coast_review";
const BEACH_REVIEW: &str = "beach_review";
const INLAND_REVIEW: &str = "inland_review";
const FOOTHILL_REVIEW: &str = "foothill_review";
const MASSIF_FRONT_REVIEW: &str = "massif_front_review";
const DEEP_MOUNTAIN_REVIEW: &str = "deep_mountain_review";
const CAVE_SOURCE_REVIEW: &str = "cave_source_review";
const RIVULET_SOURCE_REVIEW: &str = "rivulet_source_review";
const DEFAULT_ALPINE_TREELINE: Level = 36;
const DEFAULT_ALPINE_SNOWLINE: Level = 52;
const MACRO_GRASS_CEILING: Level = 36;
const CRYSTAL_LOWER_TERMINAL: &str = "crystal_ascent.lower_terminal_pad";
const CRYSTAL_UPPER_TERMINAL: &str = "crystal_ascent.upper_terminal_pad";

#[derive(Debug, Clone)]
pub(crate) struct MacroWorldMetrics {
    pub(crate) report: MacroMetrics,
    pub(crate) mountain_range: Option<MountainRangeMetrics>,
    pub(crate) ocean_archipelago: Option<OceanArchipelagoMetrics>,
}

struct MacroWorldRecipe<'a> {
    grid_radius: u32,
    level_height: f32,
    setup: MacroWorldSetup<'a>,
    prepared_spanning: Option<PlannedMacroSpanning>,
    #[cfg(test)]
    force_candidate_construction_failure: bool,
}

struct MacroWorldSetup<'a> {
    layout: ResolvedLayoutPlan,
    contracts: ResolvedMacroContracts,
    settings: &'a MacroLayoutSettings,
    vegetation: TemperateVegetationSet,
    canonical_anchors: BTreeMap<String, PatchAnchorRef>,
    alpine_climate: MacroAlpineClimate,
    crystal_ascent_assets: Option<MacroCrystalAscentAssets>,
}

#[derive(Debug, Clone)]
struct MacroCrystalAscentAssets {
    trees: TemperateTreeSet,
    objects: CrystalAscentObjectSet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct MacroAlpineClimate {
    treeline: Level,
    snowline: Level,
}

pub(crate) fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
    art_catalog: &RuntimeArtCatalog,
) -> Result<ValidatedWorldSelection<MacroWorldMetrics>, V3GenerationError> {
    let setup = resolve_macro_world_setup(grid_radius, settings, art_catalog)?;
    let prepared_spanning = prepare_macro_spanning(level_height, &setup)?;
    run_recipe(
        &MacroWorldRecipe {
            grid_radius,
            level_height,
            setup,
            prepared_spanning,
            #[cfg(test)]
            force_candidate_construction_failure: false,
        },
        settings,
        grid_radius,
        seed,
    )
}

/// Resolves candidate-independent spanning geometry once per complete selection.
///
/// Macro still constructs and validates all eight complete candidate worlds. The
/// expensive exact four-lane route is independent of seed and candidate, though,
/// so recomputing its exhaustive disjoint-path search eight times cannot change a
/// score or rejection. Candidate construction checks its authored destination
/// facts against this prepared result before applying it.
fn prepare_macro_spanning(
    level_height: f32,
    setup: &MacroWorldSetup<'_>,
) -> Result<Option<PlannedMacroSpanning>, V3GenerationError> {
    let macro_settings = setup.settings;
    if macro_settings.spanning_features.is_empty() {
        return Ok(None);
    }
    let mut fragments = BTreeMap::new();
    for (index, instance) in macro_settings.instances.iter().enumerate() {
        let V3RecipeSettings::CrystalAscent(crystal_settings) = &instance.recipe else {
            continue;
        };
        let patch_id = PatchId(u32::try_from(index).unwrap_or(u32::MAX));
        let patch = PatchRecipeContext::resolve(&setup.layout, patch_id)?;
        let assets = setup.crystal_ascent_assets.as_ref().ok_or_else(|| {
            V3GenerationError::RecipeContract(
                "Macro Crystal Ascent assets were not preflighted".to_owned(),
            )
        })?;
        let fragment = construct_macro_crystal_ascent(
            patch,
            crystal_settings,
            level_height,
            None,
            &setup.layout,
            assets,
        )?;
        fragments.insert(patch_id, fragment);
    }
    let raw_destinations = raw_spanning_destinations(&setup.contracts, &fragments)?;
    plan_macro_spanning(&setup.layout, &setup.contracts, &raw_destinations)
        .map(Some)
        .map_err(|error| {
            V3GenerationError::RecipeContract(format!(
                "Macro spanning-feature preparation failed: {error}"
            ))
        })
}

impl V3Recipe for MacroWorldRecipe<'_> {
    type Settings = ProceduralV3Settings;
    type Metrics = MacroWorldMetrics;
    type Score = (Reverse<u32>, Reverse<u32>);

    fn construct(
        &self,
        context: CandidateContext,
        settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, CandidateAttemptError> {
        self.validate_invocation(context.grid_radius, settings)
            .map_err(CandidateAttemptError::Fatal)?;
        #[cfg(test)]
        if self.force_candidate_construction_failure {
            return Err(reject_candidate_construction(
                V3GenerationError::RecipeContract(
                    "forced candidate-local Macro construction failure".to_owned(),
                ),
            ));
        }
        construct_world(
            self.level_height,
            &self.setup,
            Some((context.seed, context.candidate)),
            self.prepared_spanning.as_ref(),
            false,
        )
        .map_err(reject_candidate_construction)
    }

    fn validate(
        &self,
        settings: &Self::Settings,
        plan: &GeneratedWorldPlan,
    ) -> WorldValidation<Self::Metrics> {
        if let Err(error) = self.validate_invocation(self.grid_radius, settings) {
            return WorldValidation::Invalid(vec![macro_issue(error.to_string())]);
        }
        if plan.layout != self.setup.layout {
            return WorldValidation::Invalid(vec![macro_issue(
                "Macro candidate layout changed after candidate-independent setup",
            )]);
        }
        validate_macro_world(settings, plan, Some(&self.setup.contracts))
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
        _candidate: u8,
    ) -> Self::Score {
        (
            Reverse(
                metrics
                    .mountain_range
                    .map_or(0, |metrics| metrics.high_massif_surfaces),
            ),
            Reverse(metrics.report.reachable_surfaces),
        )
    }

    fn canonical_fallback(
        &self,
        context: FallbackContext,
        settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, V3GenerationError> {
        self.validate_invocation(context.grid_radius, settings)?;
        construct_world(
            self.level_height,
            &self.setup,
            None,
            self.prepared_spanning.as_ref(),
            false,
        )
    }
}

impl MacroWorldRecipe<'_> {
    fn validate_invocation(
        &self,
        grid_radius: u32,
        settings: &ProceduralV3Settings,
    ) -> Result<(), V3GenerationError> {
        let V3LayoutSettings::Macro(macro_settings) = &settings.layout else {
            return Err(V3GenerationError::RecipeContract(
                "Macro runner requires V3LayoutSettings::Macro".to_owned(),
            ));
        };
        if grid_radius != self.grid_radius || macro_settings != self.setup.settings {
            return Err(V3GenerationError::RecipeContract(
                "Macro runner settings changed after candidate-independent setup".to_owned(),
            ));
        }
        Ok(())
    }
}

fn resolve_macro_world_setup<'a>(
    grid_radius: u32,
    settings: &'a ProceduralV3Settings,
    art_catalog: &RuntimeArtCatalog,
) -> Result<MacroWorldSetup<'a>, V3GenerationError> {
    let V3LayoutSettings::Macro(macro_settings) = &settings.layout else {
        return Err(V3GenerationError::RecipeContract(
            "Macro runner requires V3LayoutSettings::Macro".to_owned(),
        ));
    };
    let vegetation = TemperateVegetationSet::resolve(art_catalog, "Macro world")
        .map_err(V3GenerationError::RecipeContract)?;

    let layout = resolve_layout(grid_radius, settings).map_err(|error| {
        V3GenerationError::RecipeContract(format!("Macro layout resolution failed: {error}"))
    })?;
    let contracts = resolve_macro_contracts(macro_settings, &layout).map_err(|error| {
        V3GenerationError::RecipeContract(format!("Macro extension resolution failed: {error}"))
    })?;
    let canonical_anchors = canonical_anchor_settings(macro_settings, &contracts)?;
    let alpine_climate = resolve_macro_alpine_climate(macro_settings)?;
    let crystal_ascent_assets = macro_settings
        .instances
        .iter()
        .any(|instance| matches!(instance.recipe, V3RecipeSettings::CrystalAscent(_)))
        .then(|| {
            let trees = TemperateTreeSet::resolve(art_catalog, "Macro Crystal Ascent")
                .map_err(V3GenerationError::RecipeContract)?;
            let objects = CrystalAscentObjectSet::resolve(art_catalog).map_err(|error| {
                V3GenerationError::RecipeContract(format!(
                    "Macro Crystal Ascent authored object preflight failed: {error}"
                ))
            })?;
            Ok::<_, V3GenerationError>(MacroCrystalAscentAssets { trees, objects })
        })
        .transpose()?;
    Ok(MacroWorldSetup {
        layout,
        contracts,
        settings: macro_settings,
        vegetation,
        canonical_anchors,
        alpine_climate,
        crystal_ascent_assets,
    })
}

fn resolve_macro_alpine_climate(
    settings: &MacroLayoutSettings,
) -> Result<MacroAlpineClimate, V3GenerationError> {
    let climates = settings
        .instances
        .iter()
        .filter_map(|instance| match instance.recipe {
            V3RecipeSettings::DeepMountain(settings) => Some(MacroAlpineClimate {
                treeline: settings.treeline,
                snowline: settings.snowline,
            }),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    if climates.len() > 1 {
        return Err(V3GenerationError::RecipeContract(
            "Macro Deep Mountain instances must agree on treeline and snowline".to_owned(),
        ));
    }
    Ok(climates.first().copied().unwrap_or(MacroAlpineClimate {
        treeline: DEFAULT_ALPINE_TREELINE,
        snowline: DEFAULT_ALPINE_SNOWLINE,
    }))
}

fn construct_world(
    level_height: f32,
    setup: &MacroWorldSetup<'_>,
    candidate: Option<(u64, u8)>,
    prepared_spanning: Option<&PlannedMacroSpanning>,
    finalize: bool,
) -> Result<GeneratedWorldPlan, V3GenerationError> {
    let layout = &setup.layout;
    let contracts = &setup.contracts;
    let macro_settings = setup.settings;
    let vegetation = &setup.vegetation;
    let canonical_anchors = &setup.canonical_anchors;
    let alpine_climate = setup.alpine_climate;
    let crystal_ascent_assets = setup.crystal_ascent_assets.as_ref();
    let natural_levels =
        super::macro_landform::plan_base_surface_levels(layout, macro_settings, candidate)?;
    let alpine_levels =
        super::macro_alpine::plan_alpine_height_field(layout, macro_settings, candidate).map_err(
            |error| {
                V3GenerationError::RecipeContract(format!(
                    "Macro alpine height-field planning failed: {error}"
                ))
            },
        )?;
    if natural_levels
        .keys()
        .any(|patch_id| alpine_levels.contains_key(patch_id))
    {
        return Err(V3GenerationError::RecipeContract(
            "Macro natural and alpine height fields overlap one logical instance".to_owned(),
        ));
    }
    let world_support = natural_levels
        .values()
        .chain(alpine_levels.values())
        .flat_map(|field| field.iter().map(|(coord, level)| (*coord, *level)))
        .collect::<BTreeMap<_, _>>();
    let configured_maximum_level = macro_settings
        .instances
        .iter()
        .map(|instance| match &instance.recipe {
            V3RecipeSettings::Mountains(mountains) => mountains
                .base_level
                .saturating_add(mountains.relief)
                .max(instance.elevation.high),
            V3RecipeSettings::DeepMountain(mountain) => mountain.hard_cap,
            _ => instance.elevation.high,
        })
        .max()
        .unwrap_or_default();
    let provisional_view_hint = macro_view_hint(layout, configured_maximum_level, level_height);
    // Authored destinations are built first so the whole-world tunnel planner can
    // consume their exact apertures and interior identity. The resulting corridor
    // is then reserved in every ordinary patch before liquids and decoration run.
    let mut fragments_by_patch = BTreeMap::new();
    for (index, instance) in macro_settings.instances.iter().enumerate() {
        let patch_id = PatchId(u32::try_from(index).unwrap_or(u32::MAX));
        let V3RecipeSettings::CrystalAscent(settings) = &instance.recipe else {
            continue;
        };
        let patch = PatchRecipeContext::resolve(layout, patch_id)?;
        let assets = crystal_ascent_assets.ok_or_else(|| {
            V3GenerationError::RecipeContract(
                "Macro Crystal Ascent assets were not preflighted".to_owned(),
            )
        })?;
        let fragment = construct_macro_crystal_ascent(
            patch,
            settings,
            level_height,
            candidate,
            layout,
            assets,
        )?;
        fragments_by_patch.insert(patch_id, fragment);
    }

    let raw_destinations = raw_spanning_destinations(contracts, &fragments_by_patch)?;
    let planned_spanning;
    let spanning = if let Some(spanning) = prepared_spanning {
        validate_prepared_spanning(spanning, contracts, &raw_destinations)?;
        spanning
    } else {
        planned_spanning =
            plan_macro_spanning(layout, contracts, &raw_destinations).map_err(|error| {
                V3GenerationError::RecipeContract(format!(
                    "Macro spanning-feature planning failed: {error}"
                ))
            })?;
        &planned_spanning
    };

    let no_spanning_reservations = BTreeSet::new();
    for (index, instance) in macro_settings.instances.iter().enumerate() {
        let patch_id = PatchId(u32::try_from(index).unwrap_or(u32::MAX));
        if matches!(instance.recipe, V3RecipeSettings::CrystalAscent(_)) {
            continue;
        }
        let patch = PatchRecipeContext::resolve(layout, patch_id)?;
        if matches!(
            instance.recipe,
            V3RecipeSettings::SandyIslets(_) | V3RecipeSettings::WoodedIsland(_)
        ) {
            let fragment =
                construct_macro_island(patch, instance, level_height, candidate, vegetation)?;
            fragments_by_patch.insert(patch_id, fragment);
            continue;
        }
        let streams =
            candidate.map(|(seed, candidate)| SeedStreams::new(seed, candidate, patch_id.0));
        let reservations = spanning
            .reservations_by_patch
            .get(&patch_id)
            .unwrap_or(&no_spanning_reservations);
        let fragment = construct_fragment(
            patch,
            instance,
            macro_settings,
            alpine_climate,
            natural_levels
                .get(&patch_id)
                .or_else(|| alpine_levels.get(&patch_id)),
            &alpine_levels,
            &world_support,
            reservations,
            streams,
            vegetation,
            provisional_view_hint,
        )?;
        fragments_by_patch.insert(patch_id, fragment);
    }
    let mut fragments = fragments_by_patch.into_values().collect::<Vec<_>>();

    let generated_maximum_level = fragments
        .iter()
        .flat_map(|fragment| fragment.volume.surfaces.keys())
        .map(|surface| surface.level)
        .max()
        .unwrap_or_default();
    let view_hint = macro_view_hint(layout, generated_maximum_level, level_height);
    for fragment in &mut fragments {
        fragment.view_hint = view_hint;
    }

    let composition = WorldCompositionSettings {
        canonical_anchors: canonical_anchors.clone(),
        view_hint,
    };
    if spanning.tunnels.is_empty() {
        let result = if finalize {
            compose_world(layout.clone(), fragments, composition)
        } else {
            merge_world(layout.clone(), fragments, composition)
        };
        return result.map_err(|error| {
            V3GenerationError::RecipeContract(format!("Macro composition failed: {error:?}"))
        });
    }

    let world = merge_world(layout.clone(), fragments, composition).map_err(|error| {
        V3GenerationError::RecipeContract(format!("Macro merge failed: {error:?}"))
    })?;
    let mut world = apply_macro_spanning(world, spanning).map_err(|error| {
        V3GenerationError::RecipeContract(format!(
            "Macro spanning-feature application failed: {error}"
        ))
    })?;
    finalize_crystal_mountain_landscape(macro_settings, contracts, &mut world)?;
    if finalize {
        finalize_world(world).map_err(|error| {
            V3GenerationError::RecipeContract(format!("Macro finalization failed: {error:?}"))
        })
    } else {
        Ok(world)
    }
}

fn construct_macro_island(
    patch: PatchRecipeContext<'_>,
    instance: &MacroBiomeInstanceSettings,
    level_height: f32,
    candidate: Option<(u64, u8)>,
    vegetation: &TemperateVegetationSet,
) -> Result<GeneratedPatchPlan, V3GenerationError> {
    let mode = candidate.map_or(PatchBuildMode::CanonicalFallback, |(seed, candidate)| {
        PatchBuildMode::Candidate {
            world_seed: seed,
            candidate,
        }
    });
    let fragment = match &instance.recipe {
        V3RecipeSettings::SandyIslets(settings) => {
            let fragment = super::sandy_islets::construct_patch(
                patch,
                settings,
                instance.environment,
                level_height,
                mode,
            )
            .map_err(|issues| island_recipe_error(&instance.name, "Sandy Islets", issues))?;
            match super::sandy_islets::validate_patch(patch, settings, &fragment) {
                WorldValidation::Valid(_) => fragment,
                WorldValidation::Invalid(issues) => {
                    return Err(island_recipe_error(
                        &instance.name,
                        "Sandy Islets validation",
                        issues,
                    ));
                }
            }
        }
        V3RecipeSettings::WoodedIsland(settings) => {
            let fragment = super::wooded_island::construct_patch_with_vegetation(
                patch,
                settings,
                instance.environment,
                level_height,
                mode,
                vegetation,
            )
            .map_err(|issues| island_recipe_error(&instance.name, "Wooded Island", issues))?;
            match super::wooded_island::validate_patch_with_vegetation(
                patch, settings, &fragment, vegetation,
            ) {
                WorldValidation::Valid(_) => fragment,
                WorldValidation::Invalid(issues) => {
                    return Err(island_recipe_error(
                        &instance.name,
                        "Wooded Island validation",
                        issues,
                    ));
                }
            }
        }
        _ => {
            return Err(V3GenerationError::RecipeContract(format!(
                "Macro instance {:?} is not an island recipe",
                instance.name
            )));
        }
    };
    Ok(fragment)
}

fn island_recipe_error(
    instance: &str,
    phase: &str,
    issues: Vec<WorldValidationIssue>,
) -> V3GenerationError {
    V3GenerationError::RecipeContract(format!(
        "Macro instance {instance:?} {phase} failed: {}",
        format_issues(&issues)
    ))
}

fn raw_spanning_destinations(
    contracts: &ResolvedMacroContracts,
    fragments: &BTreeMap<PatchId, GeneratedPatchPlan>,
) -> Result<RawSpanningDestinations, V3GenerationError> {
    let mut destinations = RawSpanningDestinations::new();
    for feature in contracts.spanning_features.values() {
        let super::layout::ResolvedMacroSpanningFeature::Tunnel(tunnel) = feature;
        let fragment = fragments
            .get(&tunnel.destination_anchor.instance)
            .ok_or_else(|| {
                V3GenerationError::RecipeContract(format!(
                    "Macro tunnel {:?} destination patch was not authored before planning",
                    tunnel.name
                ))
            })?;
        let anchor = fragment
            .anchors
            .get(&tunnel.destination_anchor.anchor)
            .copied()
            .ok_or_else(|| {
                V3GenerationError::RecipeContract(format!(
                    "Macro tunnel {:?} destination anchor {:?} is missing",
                    tunnel.name, tunnel.destination_anchor.anchor
                ))
            })?;
        let terminal = fragment
            .features
            .protected_routes
            .get(CRYSTAL_LOWER_TERMINAL)
            .map(|route| route.surfaces.clone())
            .ok_or_else(|| {
                V3GenerationError::RecipeContract(format!(
                    "Macro tunnel {:?} destination has no exact lower terminal",
                    tunnel.name
                ))
            })?;
        let summit_threshold = fragment
            .features
            .protected_routes
            .get(CRYSTAL_UPPER_TERMINAL)
            .map(|route| route.surfaces.clone())
            .ok_or_else(|| {
                V3GenerationError::RecipeContract(format!(
                    "Macro tunnel {:?} destination has no exact upper terminal",
                    tunnel.name
                ))
            })?;
        let interiors = fragment
            .interiors
            .by_id
            .iter()
            .filter_map(|(id, interior)| {
                terminal
                    .iter()
                    .any(|surface| {
                        surface
                            .coord
                            .neighbors()
                            .into_iter()
                            .map(|coord| TilePos::new(coord, surface.level))
                            .any(|neighbor| interior.floors.contains(&neighbor))
                    })
                    .then_some(*id)
            })
            .collect::<Vec<_>>();
        let [interior] = interiors.as_slice() else {
            return Err(V3GenerationError::RecipeContract(format!(
                "Macro tunnel {:?} destination terminal must border exactly one authored interior",
                tunnel.name
            )));
        };
        let key = (
            tunnel.destination_anchor.instance,
            tunnel.destination_anchor.anchor.clone(),
        );
        if destinations
            .insert(
                key,
                RawSpanningDestination {
                    anchor,
                    terminal,
                    interior: Some(*interior),
                    summit_threshold,
                },
            )
            .is_some()
        {
            return Err(V3GenerationError::RecipeContract(format!(
                "Macro tunnel {:?} duplicates one authored destination",
                tunnel.name
            )));
        }
    }
    Ok(destinations)
}

fn validate_prepared_spanning(
    prepared: &PlannedMacroSpanning,
    contracts: &ResolvedMacroContracts,
    destinations: &RawSpanningDestinations,
) -> Result<(), V3GenerationError> {
    if prepared
        .tunnels
        .keys()
        .ne(contracts.spanning_features.keys())
    {
        return Err(V3GenerationError::RecipeContract(
            "prepared Macro spanning-feature names changed during candidate construction"
                .to_owned(),
        ));
    }
    for (name, feature) in &contracts.spanning_features {
        let super::layout::ResolvedMacroSpanningFeature::Tunnel(contract) = feature;
        let key = (
            contract.destination_anchor.instance,
            contract.destination_anchor.anchor.clone(),
        );
        let raw = destinations.get(&key).ok_or_else(|| {
            V3GenerationError::RecipeContract(format!(
                "prepared Macro tunnel {name:?} lost its candidate destination facts"
            ))
        })?;
        let planned = prepared.tunnels.get(name).ok_or_else(|| {
            V3GenerationError::RecipeContract(format!(
                "prepared Macro tunnel {name:?} disappeared during candidate construction"
            ))
        })?;
        let raw_interior = raw.interior.ok_or_else(|| {
            V3GenerationError::RecipeContract(format!(
                "prepared Macro tunnel {name:?} lost its candidate destination interior"
            ))
        })?;
        let destination_interior = namespace_patch_local_interior(
            contract.destination_anchor.instance,
            raw_interior,
        )
        .map_err(|error| {
            V3GenerationError::RecipeContract(format!(
                "prepared Macro tunnel {name:?} cannot namespace its candidate destination interior: {error}"
            ))
        })?;
        if planned.name != *name
            || planned.instance_route != contract.instance_route
            || planned.floor_level != contract.floor_level
            || planned.destination_anchor != raw.anchor
            || planned.destination_terminal != raw.terminal
            || planned.summit_threshold != raw.summit_threshold
            || planned.destination_interior != destination_interior
        {
            return Err(V3GenerationError::RecipeContract(format!(
                "prepared Macro tunnel {name:?} disagrees with candidate-authored destination facts"
            )));
        }
    }
    Ok(())
}

fn construct_macro_crystal_ascent(
    patch: PatchRecipeContext<'_>,
    settings: &V3CrystalAscentSettings,
    level_height: f32,
    candidate: Option<(u64, u8)>,
    layout: &ResolvedLayoutPlan,
    assets: &MacroCrystalAscentAssets,
) -> Result<GeneratedPatchPlan, V3GenerationError> {
    let mode = candidate.map_or(PatchBuildMode::CanonicalFallback, |(seed, candidate)| {
        PatchBuildMode::Candidate {
            world_seed: seed,
            candidate,
        }
    });
    let fragment = super::crystal_ascent::construct_patch(
        patch,
        settings,
        level_height,
        mode,
        &assets.trees,
        &assets.objects,
    )
    .map_err(|issues| {
        V3GenerationError::RecipeContract(format!(
            "Macro Crystal Ascent construction failed: {}",
            issues
                .into_iter()
                .map(|issue| issue.detail)
                .collect::<Vec<_>>()
                .join("; ")
        ))
    })?;
    match super::crystal_ascent::validate_composite_fragment(
        &fragment,
        layout,
        settings,
        &assets.objects,
    ) {
        WorldValidation::Valid(_) => Ok(fragment),
        WorldValidation::Invalid(issues) => Err(V3GenerationError::RecipeContract(format!(
            "Macro Crystal Ascent validation failed: {}",
            issues
                .into_iter()
                .map(|issue| issue.detail)
                .collect::<Vec<_>>()
                .join("; ")
        ))),
    }
}

fn reject_candidate_construction(error: V3GenerationError) -> CandidateAttemptError {
    let detail = match error {
        V3GenerationError::RecipeContract(detail) => detail,
        error => error.to_string(),
    };
    CandidateAttemptError::Rejected(vec![macro_issue(detail)])
}

fn construct_fragment(
    patch: PatchRecipeContext<'_>,
    instance: &MacroBiomeInstanceSettings,
    macro_settings: &MacroLayoutSettings,
    alpine_climate: MacroAlpineClimate,
    planned_levels: Option<&BTreeMap<HexCoord, Level>>,
    alpine_levels: &super::macro_alpine::AlpineHeightField,
    world_support: &BTreeMap<HexCoord, Level>,
    spanning_reservations: &BTreeSet<HexCoord>,
    streams: Option<SeedStreams>,
    vegetation: &TemperateVegetationSet,
    view_hint: MapViewHint,
) -> Result<GeneratedPatchPlan, V3GenerationError> {
    let mut levels = if let Some(planned) = planned_levels {
        if planned.len() != patch.mask().len()
            || planned.keys().any(|coord| !patch.mask().contains(coord))
        {
            return Err(V3GenerationError::RecipeContract(format!(
                "Macro instance {:?} received an incomplete or foreign shared height field",
                instance.name
            )));
        }
        planned.clone()
    } else {
        base_surface_levels(patch.mask(), instance, streams)?
    };
    let alpine_boundary_levels =
        resolved_alpine_boundary_levels(&patch, macro_settings, &mut levels, alpine_levels);
    let massif_levels =
        matches!(instance.recipe, V3RecipeSettings::DeepMountain(_)).then(|| levels.clone());
    let seam_shape = shape_walker_seams(&patch, &mut levels).map_err(|issues| {
        V3GenerationError::RecipeContract(format!(
            "Macro instance {:?} walker seam shaping failed: {}",
            instance.name,
            format_issues(&issues)
        ))
    })?;
    let walker_approaches = patch.walker_protected_approaches();
    for (coord, level) in &alpine_boundary_levels {
        if !walker_approaches.contains(coord) {
            levels.insert(*coord, *level);
        }
    }
    if let Some(massif_levels) = massif_levels {
        for (coord, level) in massif_levels {
            if !walker_approaches.contains(&coord) && !alpine_boundary_levels.contains_key(&coord) {
                levels.insert(coord, level);
            }
        }
    }
    let land_route = shape_internal_land_route(&patch, &mut levels)?;
    let bridgeable_route = land_route
        .difference(&walker_approaches)
        .copied()
        .collect::<BTreeSet<_>>();
    let mut protected = patch.protected_approaches();
    protected.extend(spanning_reservations.iter().copied());
    protected.extend(land_route.iter().copied());
    let liquid_geometry = plan_liquids(
        &patch,
        instance,
        macro_settings,
        &mut levels,
        &protected,
        &bridgeable_route,
    )
    .map_err(|error| {
        V3GenerationError::RecipeContract(format!(
            "Macro instance {:?} liquid planning failed: {error}",
            instance.name
        ))
    })?;
    raise_land_route_over_liquids(
        &mut levels,
        &land_route,
        &walker_approaches,
        &liquid_geometry.top_by_coord,
    )?;

    let mut volume = build_volume(
        patch.mask(),
        instance,
        alpine_climate,
        &levels,
        &liquid_geometry.top_by_coord,
        &liquid_geometry.plan,
        &bridgeable_route,
        &liquid_geometry.overhangs,
    );
    let protected_river_surfaces = walker_approaches
        .iter()
        .filter_map(|coord| {
            volume.surfaces.iter().find_map(|(surface, metadata)| {
                (surface.coord == *coord
                    && matches!(
                        metadata.access,
                        SurfaceAccess::Ordinary | SurfaceAccess::SpecialMovement(_)
                    ))
                .then_some(*surface)
            })
        })
        .collect::<BTreeSet<_>>();
    let bed_material = if matches!(
        instance.recipe,
        V3RecipeSettings::Mountains(_) | V3RecipeSettings::DeepMountain(_)
    ) {
        SolidMaterialRole::Gravel
    } else {
        SolidMaterialRole::Dirt
    };
    super::river_terrain::fit_river_terrain(
        &mut volume,
        &liquid_geometry.plan,
        &protected_river_surfaces,
        bed_material,
    )
    .map_err(|issues| {
        V3GenerationError::RecipeContract(format!(
            "Macro instance {:?} river terrain fitting failed: {}",
            instance.name,
            issues
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        ))
    })?;
    seam_shape.apply(&mut volume).map_err(|issues| {
        V3GenerationError::RecipeContract(format!(
            "Macro instance {:?} seam projection failed: {}",
            instance.name,
            format_issues(&issues)
        ))
    })?;
    validate_internal_land_route(&patch, &volume, &land_route)?;

    let anchors = local_anchors(
        instance,
        macro_settings,
        &patch,
        &volume,
        &liquid_geometry.top_by_coord,
        liquid_geometry.headwater_review,
    )?;
    let (features, blockers) = place_vegetation(
        instance,
        &patch,
        &volume,
        alpine_climate,
        &anchors,
        &protected,
        world_support,
        streams,
        vegetation,
        is_crystal_mountain_layout(macro_settings) && instance.name == "summit-forest",
    )?;
    let biome_regions = volume
        .surfaces
        .keys()
        .copied()
        .map(|surface| (surface, patch.biome_region()))
        .collect();

    Ok(GeneratedPatchPlan {
        patch_id: patch.id,
        volume,
        liquids: liquid_geometry.plan,
        features,
        structures: StructurePlan::default(),
        blockers,
        lights: BTreeMap::new(),
        biome_regions,
        interiors: InteriorPlan::default(),
        anchors,
        view_hint,
    })
}

/// Resolves a one-level-Lipschitz height field over all alpine seam columns.
///
/// Ordinary route endpoints and the massif front are fixed to their authored
/// datums. The remaining seam columns stay as close as possible to the incident
/// pair's preferred height while respecting those fixed points. Solving this on
/// the complete boundary graph makes three-instance corners deterministic and
/// keeps the progressive tiers instead of flattening every connected seam.
fn resolved_alpine_boundary_levels(
    patch: &PatchRecipeContext<'_>,
    settings: &MacroLayoutSettings,
    levels: &mut BTreeMap<HexCoord, Level>,
    alpine_levels: &super::macro_alpine::AlpineHeightField,
) -> BTreeMap<HexCoord, Level> {
    let is_alpine = |id: PatchId| {
        settings
            .instances
            .get(usize::try_from(id.0).unwrap_or(usize::MAX))
            .is_some_and(|instance| {
                matches!(
                    &instance.recipe,
                    V3RecipeSettings::Mountains(_) | V3RecipeSettings::DeepMountain(_)
                )
            })
    };
    let is_deep = |id: PatchId| {
        settings
            .instances
            .get(usize::try_from(id.0).unwrap_or(usize::MAX))
            .is_some_and(|instance| matches!(&instance.recipe, V3RecipeSettings::DeepMountain(_)))
    };
    let same_mountain_tier = |first: PatchId, second: PatchId| {
        let Some(first) = settings
            .instances
            .get(usize::try_from(first.0).unwrap_or(usize::MAX))
        else {
            return false;
        };
        let Some(second) = settings
            .instances
            .get(usize::try_from(second.0).unwrap_or(usize::MAX))
        else {
            return false;
        };
        matches!(first.recipe, V3RecipeSettings::Mountains(_))
            && matches!(second.recipe, V3RecipeSettings::Mountains(_))
            && first.elevation == second.elevation
    };

    // Same-tier boundaries are ownership seams, not authored contour lines. Both
    // sides sample the shared alpine field and meet at their pairwise mean. This
    // retains the broad ridge shape instead of drawing one flat hex around every
    // atomic Mountain instance.
    let mut shared_samples = BTreeMap::<HexCoord, Vec<Level>>::new();
    for edge in patch
        .layout()
        .shared_edges
        .values()
        .filter(|edge| same_mountain_tier(edge.first.0, edge.second.0))
    {
        for (first, second) in &edge.boundary_pairs {
            let Some(first_level) = alpine_levels
                .get(&edge.first.0)
                .and_then(|field| field.get(first))
                .copied()
            else {
                continue;
            };
            let Some(second_level) = alpine_levels
                .get(&edge.second.0)
                .and_then(|field| field.get(second))
                .copied()
            else {
                continue;
            };
            let shared = Level::try_from((i64::from(first_level) + i64::from(second_level)) / 2)
                .unwrap_or(first_level);
            shared_samples.entry(*first).or_default().push(shared);
            shared_samples.entry(*second).or_default().push(shared);
        }
    }
    let local_shared_levels = shared_samples
        .into_iter()
        .filter(|(coord, _)| patch.mask().contains(coord))
        .map(|(coord, samples)| {
            let count = i64::try_from(samples.len()).unwrap_or(1).max(1);
            let total = samples.into_iter().map(i64::from).sum::<i64>();
            (coord, Level::try_from(total / count).unwrap_or_default())
        })
        .collect::<BTreeMap<_, _>>();
    for (coord, level) in &local_shared_levels {
        levels.insert(*coord, *level);
    }

    let mut neighbors = BTreeMap::<HexCoord, BTreeSet<HexCoord>>::new();
    let mut samples = BTreeMap::<HexCoord, Vec<Level>>::new();
    let mut fixed = BTreeMap::<HexCoord, BTreeSet<Level>>::new();
    for edge in patch.layout().shared_edges.values() {
        if !is_alpine(edge.first.0)
            || !is_alpine(edge.second.0)
            || same_mountain_tier(edge.first.0, edge.second.0)
        {
            continue;
        }
        let fixes_massif_front = is_deep(edge.first.0) || is_deep(edge.second.0);
        for (first, second) in &edge.boundary_pairs {
            neighbors.entry(*first).or_default().insert(*second);
            neighbors.entry(*second).or_default().insert(*first);
            samples
                .entry(*first)
                .or_default()
                .push(edge.elevation.preferred);
            samples
                .entry(*second)
                .or_default()
                .push(edge.elevation.preferred);
            if fixes_massif_front {
                fixed
                    .entry(*first)
                    .or_default()
                    .insert(edge.elevation.preferred);
                fixed
                    .entry(*second)
                    .or_default()
                    .insert(edge.elevation.preferred);
            }
        }
        for port in &edge.walker.ports {
            for coord in port.first_approach.iter().chain(&port.second_approach) {
                fixed
                    .entry(*coord)
                    .or_default()
                    .insert(edge.elevation.preferred);
            }
        }
    }

    let fixed = fixed
        .into_iter()
        .filter(|(coord, _)| neighbors.contains_key(coord))
        .map(|(coord, datums)| {
            let count = i64::try_from(datums.len()).unwrap_or(1).max(1);
            let sum = datums.into_iter().map(i64::from).sum::<i64>();
            (coord, Level::try_from(sum / count).unwrap_or_default())
        })
        .collect::<BTreeMap<_, _>>();

    // Fixed points induce exact feasible lower and upper bounds at every node.
    let mut lower = neighbors
        .keys()
        .copied()
        .map(|coord| (coord, Level::MIN))
        .collect::<BTreeMap<_, _>>();
    let mut upper = neighbors
        .keys()
        .copied()
        .map(|coord| (coord, Level::MAX))
        .collect::<BTreeMap<_, _>>();
    let mut lower_pending = VecDeque::new();
    let mut upper_pending = VecDeque::new();
    for (coord, datum) in &fixed {
        lower.insert(*coord, *datum);
        upper.insert(*coord, *datum);
        lower_pending.push_back(*coord);
        upper_pending.push_back(*coord);
    }
    while let Some(coord) = lower_pending.pop_front() {
        let candidate = lower
            .get(&coord)
            .copied()
            .unwrap_or(Level::MIN)
            .saturating_sub(1);
        for neighbor in neighbors.get(&coord).into_iter().flatten() {
            if lower
                .get(neighbor)
                .is_none_or(|current| candidate > *current)
            {
                lower.insert(*neighbor, candidate);
                lower_pending.push_back(*neighbor);
            }
        }
    }
    while let Some(coord) = upper_pending.pop_front() {
        let candidate = upper
            .get(&coord)
            .copied()
            .unwrap_or(Level::MAX)
            .saturating_add(1);
        for neighbor in neighbors.get(&coord).into_iter().flatten() {
            if upper
                .get(neighbor)
                .is_none_or(|current| candidate < *current)
            {
                upper.insert(*neighbor, candidate);
                upper_pending.push_back(*neighbor);
            }
        }
    }

    let mut minorant = samples
        .into_iter()
        .map(|(coord, samples)| {
            let count = i64::try_from(samples.len()).unwrap_or(1).max(1);
            let sum = samples.into_iter().map(i64::from).sum::<i64>();
            let preferred = Level::try_from(sum / count).unwrap_or_default();
            let minimum = lower.get(&coord).copied().unwrap_or(Level::MIN);
            let maximum = upper.get(&coord).copied().unwrap_or(Level::MAX);
            let bounded = if minimum <= maximum {
                preferred.clamp(minimum, maximum)
            } else {
                preferred
            };
            (coord, bounded)
        })
        .collect::<BTreeMap<_, _>>();
    let mut pending = neighbors.keys().copied().collect::<VecDeque<_>>();
    while let Some(coord) = pending.pop_front() {
        let candidate = minorant
            .get(&coord)
            .copied()
            .unwrap_or_default()
            .saturating_add(1);
        for neighbor in neighbors.get(&coord).into_iter().flatten() {
            if minorant
                .get(neighbor)
                .is_some_and(|current| candidate < *current)
            {
                minorant.insert(*neighbor, candidate);
                pending.push_back(*neighbor);
            }
        }
    }
    let resolved = minorant
        .into_iter()
        .map(|(coord, datum)| {
            (
                coord,
                datum.max(lower.get(&coord).copied().unwrap_or(Level::MIN)),
            )
        })
        .collect::<BTreeMap<_, _>>();

    let local = resolved
        .into_iter()
        .filter(|(coord, _)| patch.mask().contains(coord))
        .collect::<BTreeMap<_, _>>();
    for (coord, datum) in &local {
        levels.insert(*coord, *datum);
    }
    // The alpine planner resolves the complete boundary graph once, including
    // three-instance corners. Reapply that global answer after the local route
    // setup above so per-patch averaging cannot split a physical boundary lane.
    let global = patch
        .shared_edges()
        .filter(|edge| is_alpine(edge.contract.first.0) && is_alpine(edge.contract.second.0))
        .flat_map(|edge| edge.boundary_pairs().into_iter().map(|(local, _)| local))
        .filter_map(|coord| {
            alpine_levels
                .get(&patch.id)
                .and_then(|field| field.get(&coord))
                .copied()
                .map(|level| (coord, level))
        })
        .collect::<BTreeMap<_, _>>();
    for (coord, level) in &global {
        levels.insert(*coord, *level);
    }
    global
}

fn validate_internal_land_route(
    patch: &PatchRecipeContext<'_>,
    volume: &VolumePlan,
    land_route: &BTreeSet<HexCoord>,
) -> Result<(), V3GenerationError> {
    let groups = patch
        .shared_edges()
        .filter(|edge| edge.contract.walker.count > 0)
        .map(|edge| {
            edge.walker_ports()
                .into_iter()
                .flat_map(|port| port.first_approach)
                .map(|coord| TilePos::new(coord, edge.preferred_level()))
                .collect::<BTreeSet<_>>()
        })
        .filter(|group| !group.is_empty())
        .collect::<Vec<_>>();
    let Some(start) = groups.first().and_then(BTreeSet::first).copied() else {
        return Ok(());
    };
    if groups.len() < 2 {
        return Ok(());
    }
    let ordinary = OrdinaryGraph::from_volume(volume, None);
    let distances = ordinary.distances_from(start);
    let reached = groups
        .iter()
        .map(|group| group.iter().any(|surface| distances.contains_key(surface)))
        .collect::<Vec<_>>();
    if reached.iter().all(|is_reached| *is_reached) {
        Ok(())
    } else {
        let frontier = land_route
            .iter()
            .flat_map(|coord| {
                coord.neighbors().into_iter().filter_map(|neighbor| {
                    if !land_route.contains(&neighbor) {
                        return None;
                    }
                    let local = volume
                        .surfaces
                        .iter()
                        .filter(|(surface, metadata)| {
                            surface.coord == *coord && metadata.access == SurfaceAccess::Ordinary
                        })
                        .map(|(surface, _)| (*surface, distances.contains_key(surface)))
                        .collect::<Vec<_>>();
                    let adjacent = volume
                        .surfaces
                        .iter()
                        .filter(|(surface, metadata)| {
                            surface.coord == neighbor && metadata.access == SurfaceAccess::Ordinary
                        })
                        .map(|(surface, _)| (*surface, distances.contains_key(surface)))
                        .collect::<Vec<_>>();
                    (local.iter().any(|(_, is_reached)| *is_reached)
                        != adjacent.iter().any(|(_, is_reached)| *is_reached))
                    .then_some((local, adjacent))
                })
            })
            .take(8)
            .collect::<Vec<_>>();
        Err(V3GenerationError::RecipeContract(format!(
            "Macro patch {} internal walker groups are disconnected from {start:?}: {reached:?}; first route frontier {frontier:?}",
            patch.id.0,
        )))
    }
}

fn raise_land_route_over_liquids(
    levels: &mut BTreeMap<HexCoord, Level>,
    land_route: &BTreeSet<HexCoord>,
    fixed_approaches: &BTreeSet<HexCoord>,
    liquid_tops: &BTreeMap<HexCoord, Level>,
) -> Result<(), V3GenerationError> {
    let Some((start, goal)) = fixed_approaches
        .iter()
        .flat_map(|start| {
            fixed_approaches
                .range((std::ops::Bound::Excluded(start), std::ops::Bound::Unbounded))
                .map(move |goal| (*start, *goal))
        })
        .max_by_key(|(start, goal)| (start.distance(*goal), Reverse(*start), Reverse(*goal)))
    else {
        return Ok(());
    };
    let Some(path) = shortest_coord_path(start, goal, |coord| land_route.contains(&coord)) else {
        return Err(V3GenerationError::RecipeContract(format!(
            "Macro protected land route has no internal spine from {start:?} to {goal:?}"
        )));
    };
    let start_level = levels.get(&start).copied().unwrap_or_default();
    let goal_level = levels.get(&goal).copied().unwrap_or(start_level);
    let transitions = i32::try_from(path.len().saturating_sub(1)).unwrap_or(i32::MAX);
    let change = goal_level.saturating_sub(start_level);
    if transitions < i32::try_from(change.unsigned_abs()).unwrap_or(i32::MAX) {
        return Err(V3GenerationError::RecipeContract(format!(
            "Macro protected route needs {change} levels across only {transitions} steps"
        )));
    }
    let mut assigned = path
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let index = i32::try_from(index).unwrap_or(transitions);
            if transitions == 0 {
                start_level
            } else {
                start_level.saturating_add(change.saturating_mul(index) / transitions)
            }
        })
        .collect::<Vec<_>>();
    for (constraint_index, coord) in path.iter().enumerate() {
        let Some(water_top) = liquid_tops.get(coord).copied() else {
            continue;
        };
        let required = water_top.saturating_add(1);
        for (index, level) in assigned.iter_mut().enumerate() {
            let distance = i32::try_from(index.abs_diff(constraint_index)).unwrap_or(i32::MAX);
            *level = (*level).max(required.saturating_sub(distance));
        }
    }
    if assigned.first().copied() != Some(start_level)
        || assigned.last().copied() != Some(goal_level)
    {
        return Err(V3GenerationError::RecipeContract(format!(
            "Macro route bridge cannot retain fixed endpoint levels {start_level}->{goal_level}"
        )));
    }
    for (coord, level) in path.into_iter().zip(assigned) {
        if !fixed_approaches.contains(&coord) {
            levels.insert(coord, level);
        }
    }
    for coord in land_route {
        if let Some(water_top) = liquid_tops.get(coord).copied() {
            levels
                .entry(*coord)
                .and_modify(|level| *level = (*level).max(water_top.saturating_add(1)));
        }
    }
    let mut pending = land_route.iter().copied().collect::<VecDeque<_>>();
    while let Some(coord) = pending.pop_front() {
        let level = levels.get(&coord).copied().unwrap_or_default();
        let required_neighbor = level.saturating_sub(1);
        for neighbor in coord.neighbors() {
            if !land_route.contains(&neighbor) {
                continue;
            }
            let neighbor_level = levels.get(&neighbor).copied().unwrap_or_default();
            if neighbor_level >= required_neighbor {
                continue;
            }
            if fixed_approaches.contains(&neighbor) {
                return Err(V3GenerationError::RecipeContract(format!(
                    "Macro route relaxation cannot retain fixed approach {neighbor:?} at level {neighbor_level}"
                )));
            }
            levels.insert(neighbor, required_neighbor);
            pending.push_back(neighbor);
        }
    }
    Ok(())
}

fn shape_internal_land_route(
    patch: &PatchRecipeContext<'_>,
    levels: &mut BTreeMap<HexCoord, Level>,
) -> Result<BTreeSet<HexCoord>, V3GenerationError> {
    let mut groups = patch
        .shared_edges()
        .flat_map(|edge| {
            edge.walker_ports()
                .into_iter()
                .map(|port| port.first_approach)
        })
        .filter(|group| !group.is_empty())
        .collect::<Vec<_>>();
    groups.sort_unstable();
    let mut reserved = groups
        .iter()
        .flat_map(|group| group.iter().copied())
        .collect::<BTreeSet<_>>();
    let fixed_approaches = reserved.clone();
    if groups.len() < 2 {
        return Ok(reserved);
    }
    let non_walker_protected = patch
        .protected_approaches()
        .difference(&reserved)
        .copied()
        .collect::<BTreeSet<_>>();
    let shared_boundary = patch
        .shared_edges()
        .flat_map(|edge| edge.boundary_pairs().into_iter().map(|(local, _)| local))
        .collect::<BTreeSet<_>>();

    for pair in groups.windows(2) {
        let [first_group, second_group] = pair else {
            continue;
        };
        let Some((start, goal)) = first_group
            .iter()
            .flat_map(|start| second_group.iter().map(move |goal| (*start, *goal)))
            .min_by_key(|(start, goal)| (start.distance(*goal), *start, *goal))
        else {
            continue;
        };
        let admitted = |coord: HexCoord| {
            patch.mask().contains(&coord)
                && (!non_walker_protected.contains(&coord) || coord == start || coord == goal)
                && (!shared_boundary.contains(&coord) || coord == start || coord == goal)
        };
        let direct = start.distance(goal);
        let waypoint = patch
            .mask()
            .iter()
            .copied()
            .filter(|coord| admitted(*coord))
            .filter(|coord| {
                start.distance(*coord).saturating_add(coord.distance(goal))
                    <= direct.saturating_add(16)
            })
            .max_by_key(|coord| {
                (
                    coord.y(),
                    Reverse(start.distance(*coord).saturating_add(coord.distance(goal))),
                    Reverse(*coord),
                )
            })
            .unwrap_or(goal);
        let goal_shoulder = patch
            .mask()
            .iter()
            .copied()
            .filter(|coord| admitted(*coord) && coord.distance(goal) <= 4)
            .max_by_key(|coord| (coord.y(), Reverse(coord.distance(goal)), Reverse(*coord)))
            .unwrap_or(goal);
        let mut path = Vec::new();
        let mut stops = vec![start, waypoint, goal_shoulder, goal];
        stops.dedup();
        for stop_pair in stops.windows(2) {
            let Some(segment_start) = stop_pair.first().copied() else {
                continue;
            };
            let Some(segment_goal) = stop_pair.get(1).copied() else {
                continue;
            };
            let Some(segment) = shortest_coord_path(segment_start, segment_goal, admitted) else {
                return Err(V3GenerationError::RecipeContract(format!(
                    "Macro patch {} cannot connect route segment {segment_start:?} -> {segment_goal:?}",
                    patch.id.0
                )));
            };
            path.extend(segment.into_iter().skip(usize::from(!path.is_empty())));
        }
        let start_level = levels.get(&start).copied().unwrap_or_default();
        let goal_level = levels.get(&goal).copied().unwrap_or(start_level);
        let transitions = i32::try_from(path.len().saturating_sub(1)).unwrap_or(i32::MAX);
        let change = goal_level.saturating_sub(start_level);
        if transitions < change.unsigned_abs().try_into().unwrap_or(i32::MAX) {
            return Err(V3GenerationError::RecipeContract(format!(
                "Macro patch {} needs {change} levels across only {transitions} route steps",
                patch.id.0
            )));
        }
        let crosses_directed_water = patch
            .shared_edges()
            .filter(|edge| edge.liquid_port().is_some())
            .count()
            >= 2;
        let crest_index = path.len().saturating_sub(2);
        let crest_level = start_level.max(goal_level).saturating_add(1);
        for (index, coord) in path.iter().copied().enumerate() {
            let index = i32::try_from(index).unwrap_or(transitions);
            let level = if crosses_directed_water && crest_index > 0 {
                let crest_index = i32::try_from(crest_index).unwrap_or(transitions);
                if index <= crest_index {
                    start_level.saturating_add(
                        crest_level
                            .saturating_sub(start_level)
                            .saturating_mul(index)
                            / crest_index,
                    )
                } else {
                    let tail = transitions.saturating_sub(crest_index).max(1);
                    crest_level.saturating_add(
                        goal_level
                            .saturating_sub(crest_level)
                            .saturating_mul(index.saturating_sub(crest_index))
                            / tail,
                    )
                }
            } else if transitions == 0 {
                start_level
            } else {
                start_level.saturating_add(change.saturating_mul(index) / transitions)
            };
            if !fixed_approaches.contains(&coord) {
                levels.insert(coord, level);
            }
        }
        reserved.extend(path);
    }
    Ok(reserved)
}

fn base_surface_levels(
    mask: &BTreeSet<HexCoord>,
    instance: &MacroBiomeInstanceSettings,
    streams: Option<SeedStreams>,
) -> Result<BTreeMap<HexCoord, Level>, V3GenerationError> {
    let minimum_grade = mask
        .iter()
        .map(|coord| grade_projection(*coord, instance.elevation.grade_axis))
        .min()
        .unwrap_or_default();
    let maximum_grade = mask
        .iter()
        .map(|coord| grade_projection(*coord, instance.elevation.grade_axis))
        .max()
        .unwrap_or(minimum_grade);
    let grade_span = maximum_grade.saturating_sub(minimum_grade).max(1);
    let boundary_depths = boundary_depths(mask);
    let summit = dominant_summit(mask, &boundary_depths);
    let maximum_summit_distance = mask
        .iter()
        .map(|coord| coord.distance(summit))
        .max()
        .unwrap_or(1)
        .max(1);
    let noise = streams.map(|streams| streams.stage("macro.terrain"));

    mask.iter()
        .copied()
        .map(|coord| {
            let progress = grade_projection(coord, instance.elevation.grade_axis)
                .saturating_sub(minimum_grade);
            let range = instance
                .elevation
                .high
                .saturating_sub(instance.elevation.low);
            let datum = instance.elevation.low.saturating_add(
                range
                    .saturating_mul(progress)
                    .checked_div(grade_span)
                    .unwrap_or_default(),
            );
            let sampled_noise = noise
                .map(|stream| {
                    i32::try_from(stream.sample_coord(coord, 0) % 3).unwrap_or_default() - 1
                })
                .unwrap_or_default();
            let level = match &instance.recipe {
                V3RecipeSettings::ShallowSea(_) => 4,
                V3RecipeSettings::Beach(_) => datum.clamp(9, 13),
                V3RecipeSettings::Shore(settings) => datum
                    .max(8_i32.saturating_add(settings.cliff_height))
                    .clamp(11, 13),
                V3RecipeSettings::Mountains(settings) => {
                    let depth = i32::try_from(*boundary_depths.get(&coord).unwrap_or(&0))
                        .unwrap_or_default();
                    let cap = settings.base_level.saturating_add(settings.relief);
                    datum
                        .saturating_add(depth.saturating_mul(2))
                        .saturating_add(sampled_noise.max(0))
                        .min(cap)
                }
                V3RecipeSettings::DeepMountain(settings) => {
                    let depth = i32::try_from(*boundary_depths.get(&coord).unwrap_or(&0))
                        .unwrap_or_default();
                    let radial = i32::try_from(
                        maximum_summit_distance.saturating_sub(coord.distance(summit)),
                    )
                    .unwrap_or_default();
                    let radial_bonus = 20_i32
                        .saturating_mul(radial)
                        .checked_div(i32::try_from(maximum_summit_distance).unwrap_or(1))
                        .unwrap_or_default()
                        // The massif is one union-mask height field, but its entire
                        // perimeter still has to meet the surrounding second tier
                        // at the authored datum. Fade the radial shoulder in over
                        // the first eight columns instead of letting distance from
                        // the summit lift boundary columns above level 48.
                        .saturating_mul(depth.min(8))
                        .checked_div(8)
                        .unwrap_or_default();
                    let mut height = instance
                        .elevation
                        .low
                        .saturating_add(depth.saturating_mul(3).min(28))
                        .saturating_add(radial_bonus)
                        .min(settings.hard_cap);
                    if coord == summit {
                        height = settings.summit_level.min(settings.hard_cap);
                    }
                    height
                }
                _ => datum
                    .saturating_add(sampled_noise)
                    .clamp(instance.elevation.low, instance.elevation.high),
            };
            if !(4..=MAX_V3_LEVEL).contains(&level) {
                return Err(V3GenerationError::RecipeContract(format!(
                    "Macro instance {:?} produced invalid surface level {level} at {coord:?}",
                    instance.name
                )));
            }
            Ok((coord, level))
        })
        .collect()
}

fn grade_projection(coord: HexCoord, axis: MacroAxisSettings) -> i32 {
    let [x, y, z] = coord.to_cubic_array();
    match axis {
        MacroAxisSettings::East => x.saturating_sub(z),
        MacroAxisSettings::SouthEast => y.saturating_sub(z),
        MacroAxisSettings::SouthWest => y.saturating_sub(x),
        MacroAxisSettings::West => z.saturating_sub(x),
        MacroAxisSettings::NorthWest => z.saturating_sub(y),
        MacroAxisSettings::NorthEast => x.saturating_sub(y),
    }
}

fn boundary_depths(mask: &BTreeSet<HexCoord>) -> BTreeMap<HexCoord, u32> {
    let mut depths = BTreeMap::<HexCoord, u32>::new();
    let mut pending = VecDeque::new();
    for coord in mask.iter().copied() {
        if coord
            .neighbors()
            .into_iter()
            .any(|neighbor| !mask.contains(&neighbor))
        {
            depths.insert(coord, 0);
            pending.push_back(coord);
        }
    }
    while let Some(coord) = pending.pop_front() {
        let depth = depths.get(&coord).copied().unwrap_or_default();
        for neighbor in coord.neighbors() {
            if mask.contains(&neighbor) && !depths.contains_key(&neighbor) {
                depths.insert(neighbor, depth.saturating_add(1));
                pending.push_back(neighbor);
            }
        }
    }
    depths
}

fn dominant_summit(mask: &BTreeSet<HexCoord>, depths: &BTreeMap<HexCoord, u32>) -> HexCoord {
    let count = i64::try_from(mask.len()).unwrap_or(1).max(1);
    let mean_x = mask.iter().map(|coord| i64::from(coord.x())).sum::<i64>() / count;
    let mean_y = mask.iter().map(|coord| i64::from(coord.y())).sum::<i64>() / count;
    mask.iter()
        .copied()
        .max_by_key(|coord| {
            let distance =
                i64::from(coord.x()).abs_diff(mean_x) + i64::from(coord.y()).abs_diff(mean_y);
            (
                depths.get(coord).copied().unwrap_or_default(),
                Reverse(distance),
                Reverse(*coord),
            )
        })
        .unwrap_or(HexCoord::ORIGIN)
}

#[derive(Debug)]
struct PlannedLiquids {
    plan: LiquidPlan,
    top_by_coord: BTreeMap<HexCoord, Level>,
    overhangs: BTreeMap<HexCoord, LevelInterval>,
    headwater_review: Option<HeadwaterReviewHint>,
}

#[derive(Debug, Clone, Copy)]
struct HeadwaterReviewHint {
    feature: HexCoord,
    preferred_bank: Option<HexCoord>,
}

#[derive(Debug, Clone)]
struct DirectedPort {
    coords: Vec<HexCoord>,
    level: Level,
}

fn plan_liquids(
    patch: &PatchRecipeContext<'_>,
    instance: &MacroBiomeInstanceSettings,
    settings: &MacroLayoutSettings,
    levels: &mut BTreeMap<HexCoord, Level>,
    protected: &BTreeSet<HexCoord>,
    bridgeable_route: &BTreeSet<HexCoord>,
) -> Result<PlannedLiquids, V3GenerationError> {
    let waterfall_flow = matches!(instance.recipe, V3RecipeSettings::Waterfall(_));
    let mut standing_lanes = BTreeSet::new();
    let mut incoming = Vec::new();
    let mut outgoing = Vec::new();
    let mut allowed_boundary = BTreeSet::new();
    let mut all_shared_boundary = BTreeSet::new();
    for edge in patch.shared_edges() {
        all_shared_boundary.extend(edge.boundary_pairs().into_iter().map(|(local, _)| local));
        if let Some(standing) = edge.standing_water_port() {
            let level = exact_liquid_level(standing.elevation, edge.preferred_level());
            let coords = standing
                .port
                .lanes
                .iter()
                .map(|(local, _)| *local)
                .collect::<BTreeSet<_>>();
            standing_lanes.extend(coords.iter().map(|coord| (*coord, level)));
            allowed_boundary.extend(coords);
        }
        if let Some(liquid) = edge.liquid_port() {
            let level = exact_liquid_level(liquid.elevation, edge.preferred_level());
            let mut coords = liquid
                .port
                .lanes
                .iter()
                .map(|(local, _)| *local)
                .collect::<Vec<_>>();
            coords.sort_unstable();
            allowed_boundary.extend(coords.iter().copied());
            let port = DirectedPort { coords, level };
            if liquid.is_source {
                outgoing.push(port);
            } else {
                incoming.push(port);
            }
        }
    }

    let mut top_by_coord = coastal_water_footprint(
        patch.mask(),
        instance,
        &standing_lanes,
        &allowed_boundary,
        &all_shared_boundary,
        protected,
    );
    for (coord, level) in &standing_lanes {
        top_by_coord.insert(*coord, *level);
    }

    let mut bodies = Vec::<BTreeMap<TilePos, LiquidNode>>::new();
    let mut occupied = BTreeSet::new();
    let mut overhangs = BTreeMap::new();
    let mut headwater_review = None;
    if !top_by_coord.is_empty() {
        let mut standing_body = top_by_coord
            .iter()
            .map(|(coord, level)| {
                (
                    TilePos::new(*coord, *level),
                    LiquidNode {
                        state: LiquidFlowState::Still,
                        downstream: None,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        occupied.extend(top_by_coord.keys().copied());
        let targets = standing_body.keys().copied().collect::<Vec<_>>();
        for port in &incoming {
            for (lane_index, start) in port.coords.iter().copied().enumerate() {
                if top_by_coord.contains_key(&start) {
                    continue;
                }
                let Some((target, mut path)) = reachable_liquid_target(
                    patch.mask(),
                    start,
                    &targets,
                    lane_index,
                    &occupied,
                    &all_shared_boundary,
                    &allowed_boundary,
                    protected,
                ) else {
                    continue;
                };
                path.pop();
                append_flow_path(
                    &mut standing_body,
                    &mut top_by_coord,
                    &path,
                    port.level,
                    target.level,
                    Some(target),
                    waterfall_flow,
                )?;
                occupied.extend(path);
            }
        }
        for port in &outgoing {
            for (lane_index, endpoint) in port.coords.iter().copied().enumerate() {
                if top_by_coord.contains_key(&endpoint) {
                    continue;
                }
                let Some((_target, mut path)) = reachable_liquid_target(
                    patch.mask(),
                    endpoint,
                    &targets,
                    lane_index,
                    &occupied,
                    &all_shared_boundary,
                    &allowed_boundary,
                    protected,
                ) else {
                    continue;
                };
                path.reverse();
                append_still_path(&mut standing_body, &mut top_by_coord, &path, port.level);
                occupied.extend(path);
            }
        }
        bodies.push(standing_body);
    }

    if bodies.is_empty() {
        let incoming_lanes = flatten_ports(&incoming);
        let outgoing_lanes = flatten_ports(&outgoing);
        if !incoming_lanes.is_empty() && !outgoing_lanes.is_empty() {
            let mut body = BTreeMap::new();
            let mut available = (0..outgoing_lanes.len()).collect::<BTreeSet<_>>();
            let clearable_route = bridgeable_route;
            let relaxed_protected = protected
                .difference(clearable_route)
                .copied()
                .collect::<BTreeSet<_>>();
            for (start, start_level) in incoming_lanes {
                if available.is_empty() {
                    available.extend(0..outgoing_lanes.len());
                }
                let mut targets = available.iter().copied().collect::<Vec<_>>();
                targets.sort_unstable_by_key(|index| {
                    let target = outgoing_lanes.get(*index).copied().unwrap_or_default();
                    (start.distance(target.0), target.0, *index)
                });
                let mut reusable_targets = (0..outgoing_lanes.len()).collect::<Vec<_>>();
                reusable_targets.sort_unstable_by_key(|index| {
                    let target = outgoing_lanes.get(*index).copied().unwrap_or_default();
                    (start.distance(target.0), target.0, *index)
                });
                let try_targets =
                    |candidate_targets: &[usize], route_protection: &BTreeSet<HexCoord>| {
                        candidate_targets.iter().copied().find_map(|index| {
                            let (end, end_level) = outgoing_lanes.get(index).copied()?;
                            (start_level >= end_level)
                                .then(|| {
                                    liquid_path(
                                        patch.mask(),
                                        start,
                                        end,
                                        &occupied,
                                        &all_shared_boundary,
                                        &allowed_boundary,
                                        route_protection,
                                    )
                                    .ok()
                                    .map(|path| (index, end_level, path))
                                })
                                .flatten()
                        })
                    };
                let Some((target_index, end_level, path)) = try_targets(&targets, protected)
                    .or_else(|| try_targets(&reusable_targets, protected))
                    .or_else(|| try_targets(&targets, &relaxed_protected))
                    .or_else(|| try_targets(&reusable_targets, &relaxed_protected))
                else {
                    return Err(V3GenerationError::RecipeContract(format!(
                        "Macro liquid channel cannot reach any downstream lane from {start:?}; bridgeable route columns: {clearable_route:?}"
                    )));
                };
                available.remove(&target_index);
                append_flow_path(
                    &mut body,
                    &mut top_by_coord,
                    &path,
                    start_level,
                    end_level,
                    None,
                    waterfall_flow,
                )?;
            }
            occupied.extend(body.keys().map(|position| position.coord));
            bodies.push(body);
        } else if incoming_lanes.is_empty() && !outgoing_lanes.is_empty() {
            if let Some(headwater) = settings
                .headwaters
                .iter()
                .find(|headwater| headwater_instance(headwater) == instance.name)
            {
                let planned = plan_headwater(
                    patch,
                    instance,
                    headwater,
                    &outgoing,
                    levels,
                    protected,
                    &all_shared_boundary,
                    &allowed_boundary,
                )?;
                top_by_coord.extend(planned.top_by_coord);
                overhangs.extend(planned.overhangs);
                headwater_review = Some(planned.review);
                bodies.push(planned.body);
            } else {
                for (coord, level) in outgoing_lanes {
                    top_by_coord.insert(coord, level);
                    bodies.push(BTreeMap::from([(
                        TilePos::new(coord, level),
                        LiquidNode {
                            state: LiquidFlowState::Still,
                            downstream: None,
                        },
                    )]));
                }
            }
        } else {
            for (coord, level) in incoming_lanes.into_iter().chain(outgoing_lanes) {
                top_by_coord.insert(coord, level);
                bodies.push(BTreeMap::from([(
                    TilePos::new(coord, level),
                    LiquidNode {
                        state: LiquidFlowState::Still,
                        downstream: None,
                    },
                )]));
            }
        }
    }

    trim_coastal_water_coverage(
        patch.mask(),
        instance,
        &standing_lanes,
        &mut bodies,
        &mut top_by_coord,
    );
    let bodies = bodies
        .into_iter()
        .enumerate()
        .filter(|(_, nodes)| !nodes.is_empty())
        .map(|(index, nodes)| {
            (
                LiquidBodyId(u32::try_from(index).unwrap_or(u32::MAX)),
                LiquidBodyPlan {
                    material: FillMaterialRole::Water,
                    nodes,
                },
            )
        })
        .collect();
    Ok(PlannedLiquids {
        plan: LiquidPlan { bodies },
        top_by_coord,
        overhangs,
        headwater_review,
    })
}

#[derive(Debug)]
struct PlannedHeadwater {
    body: BTreeMap<TilePos, LiquidNode>,
    top_by_coord: BTreeMap<HexCoord, Level>,
    overhangs: BTreeMap<HexCoord, LevelInterval>,
    review: HeadwaterReviewHint,
}

#[derive(Debug)]
struct RoutedHeadwater {
    paths: Vec<RoutedHeadwaterPath>,
    review_feature: HexCoord,
}

#[derive(Debug)]
struct RoutedHeadwaterPath {
    coords: Vec<HexCoord>,
    start_level: Level,
    end_level: Level,
}

const fn headwater_instance(headwater: &MacroHeadwaterSettings) -> &str {
    match headwater {
        MacroHeadwaterSettings::CaveFall { instance, .. }
        | MacroHeadwaterSettings::RivuletConfluence { instance, .. } => instance.as_str(),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "headwater routing must honor every authored seam and terrain exclusion"
)]
fn plan_headwater(
    patch: &PatchRecipeContext<'_>,
    instance: &MacroBiomeInstanceSettings,
    headwater: &MacroHeadwaterSettings,
    outgoing: &[DirectedPort],
    levels: &mut BTreeMap<HexCoord, Level>,
    protected: &BTreeSet<HexCoord>,
    shared_boundary: &BTreeSet<HexCoord>,
    allowed_boundary: &BTreeSet<HexCoord>,
) -> Result<PlannedHeadwater, V3GenerationError> {
    let mut outlets = flatten_ports(outgoing);
    outlets.sort_unstable();
    let (source_level, branch_count, cave_overhang) = match headwater {
        MacroHeadwaterSettings::CaveFall {
            source_level,
            overhang_depth,
            ..
        } => (*source_level, outlets.len(), Some(*overhang_depth)),
        MacroHeadwaterSettings::RivuletConfluence {
            source_level,
            branch_count,
            ..
        } => (*source_level, usize::from(*branch_count), None),
    };
    if outlets.is_empty() {
        return Err(V3GenerationError::RecipeContract(format!(
            "Macro headwater {:?} needs at least one outgoing lane",
            instance.name
        )));
    }
    let outlet_level = outlets
        .iter()
        .map(|(_, level)| *level)
        .collect::<BTreeSet<_>>();
    if outlet_level.len() != 1 {
        return Err(V3GenerationError::RecipeContract(format!(
            "Macro headwater {:?} outgoing lanes do not share one exact level: {outlet_level:?}",
            instance.name
        )));
    }
    let outlet_level = outlet_level.first().copied().unwrap_or(source_level);
    let waterfall = cave_overhang.is_some();
    let sources = select_headwater_sources(
        patch.mask(),
        &outlets,
        source_level,
        outlet_level,
        branch_count,
        waterfall,
        levels,
        protected,
        shared_boundary,
    )?;
    let routed = if let Some(depth) = cave_overhang {
        let paths = route_headwater_paths(
            patch.mask(),
            &sources,
            &outlets,
            source_level,
            protected,
            shared_boundary,
            allowed_boundary,
        )?;
        let review_feature =
            representative_path_coord(&paths, usize::from(depth).saturating_sub(1))
                .unwrap_or_else(|| sources.first().copied().unwrap_or(HexCoord::ORIGIN));
        RoutedHeadwater {
            paths: paths
                .into_iter()
                .map(|(coords, end_level)| RoutedHeadwaterPath {
                    coords,
                    start_level: source_level,
                    end_level,
                })
                .collect(),
            review_feature,
        }
    } else {
        route_rivulet_confluence(
            patch.mask(),
            &sources,
            &outlets,
            source_level,
            outlet_level,
            protected,
            shared_boundary,
            allowed_boundary,
        )?
    };

    let mut body = BTreeMap::new();
    let mut top_by_coord = BTreeMap::new();
    let mut path_positions = Vec::new();
    for path in &routed.paths {
        if let Some(depth) = cave_overhang {
            append_cave_fall_path(
                &mut body,
                &mut top_by_coord,
                &path.coords,
                path.start_level,
                path.end_level,
                depth,
            )?;
        } else {
            append_flow_path(
                &mut body,
                &mut top_by_coord,
                &path.coords,
                path.start_level,
                path.end_level,
                None,
                false,
            )?;
        }
        path_positions.push(
            path.coords
                .iter()
                .filter_map(|coord| {
                    top_by_coord
                        .get(coord)
                        .copied()
                        .map(|level| TilePos::new(*coord, level))
                })
                .collect::<Vec<_>>(),
        );
    }
    normalize_headwater_flow_states(&mut body, waterfall)?;
    shape_headwater_landform(
        patch.mask(),
        &path_positions,
        levels,
        protected,
        shared_boundary,
        if waterfall { 2 } else { 1 },
    )?;

    let mut overhangs = BTreeMap::new();
    if let Some(depth) = cave_overhang {
        let bottom = source_level.saturating_add(3);
        let roof = LevelInterval::new(bottom, bottom.saturating_add(3));
        for position in path_positions
            .iter()
            .flat_map(|path| path.iter().take(usize::from(depth)))
        {
            overhangs.insert(position.coord, roof);
        }
    }
    let water = path_positions
        .iter()
        .flatten()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    let preferred_bank =
        preferred_headwater_review_bank(patch.mask(), routed.review_feature, &water, &outlets);
    Ok(PlannedHeadwater {
        body,
        top_by_coord,
        overhangs,
        review: HeadwaterReviewHint {
            feature: routed.review_feature,
            preferred_bank,
        },
    })
}

fn normalize_headwater_flow_states(
    body: &mut BTreeMap<TilePos, LiquidNode>,
    waterfall: bool,
) -> Result<(), V3GenerationError> {
    let positions = body.keys().copied().collect::<Vec<_>>();
    for position in positions {
        let Some(node) = body.get(&position).copied() else {
            continue;
        };
        let state = match node.downstream {
            None => LiquidFlowState::Still,
            Some(downstream) if downstream.level > position.level => {
                return Err(V3GenerationError::RecipeContract(format!(
                    "Macro headwater replacement created uphill flow {position:?} -> {downstream:?}"
                )));
            }
            Some(downstream)
                if waterfall && position.level.saturating_sub(downstream.level) >= 2 =>
            {
                LiquidFlowState::Fall
            }
            Some(downstream) if waterfall && position.level > downstream.level => {
                LiquidFlowState::Rapid
            }
            Some(_) => LiquidFlowState::Current,
        };
        if let Some(node) = body.get_mut(&position) {
            node.state = state;
        }
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "source selection consumes the complete authored river exclusion set"
)]
fn select_headwater_sources(
    mask: &BTreeSet<HexCoord>,
    outlets: &[(HexCoord, Level)],
    source_level: Level,
    outlet_level: Level,
    count: usize,
    clustered: bool,
    levels: &BTreeMap<HexCoord, Level>,
    protected: &BTreeSet<HexCoord>,
    shared_boundary: &BTreeSet<HexCoord>,
) -> Result<Vec<HexCoord>, V3GenerationError> {
    let outlet_coords = outlets
        .iter()
        .map(|(coord, _)| *coord)
        .collect::<BTreeSet<_>>();
    let distances = distances_within(mask, &outlet_coords);
    let drop = u32::try_from(source_level.saturating_sub(outlet_level)).unwrap_or_default();
    let minimum_depth = drop.saturating_add(3);
    let target_depth = minimum_depth.saturating_add(if clustered { 2 } else { 4 });
    let mut candidates = mask
        .iter()
        .copied()
        .filter(|coord| {
            distances.get(coord).copied().unwrap_or_default() >= minimum_depth
                && !protected.contains(coord)
                && !shared_boundary.contains(coord)
        })
        .collect::<Vec<_>>();
    candidates.sort_unstable_by_key(|coord| {
        (
            distances
                .get(coord)
                .copied()
                .unwrap_or_default()
                .abs_diff(target_depth),
            Reverse(levels.get(coord).copied().unwrap_or_default()),
            canonical_coord_hash(*coord),
            *coord,
        )
    });

    if clustered {
        for center in &candidates {
            let mut group = candidates
                .iter()
                .copied()
                .filter(|candidate| center.distance(*candidate) <= 1)
                .collect::<Vec<_>>();
            group.sort_unstable_by_key(|coord| (center.distance(*coord), *coord));
            if group.len() >= count {
                group.truncate(count);
                return Ok(group);
            }
        }
    } else {
        for separation in (2..=4).rev() {
            let mut selected = Vec::new();
            for candidate in &candidates {
                if selected
                    .iter()
                    .all(|selected: &HexCoord| selected.distance(*candidate) >= separation)
                {
                    selected.push(*candidate);
                    if selected.len() == count {
                        return Ok(selected);
                    }
                }
            }
        }
    }
    Err(V3GenerationError::RecipeContract(format!(
        "Macro headwater cannot place {count} {} source tips at least {minimum_depth} columns inland",
        if clustered { "clustered" } else { "separated" }
    )))
}

fn route_headwater_paths(
    mask: &BTreeSet<HexCoord>,
    sources: &[HexCoord],
    outlets: &[(HexCoord, Level)],
    source_level: Level,
    protected: &BTreeSet<HexCoord>,
    shared_boundary: &BTreeSet<HexCoord>,
    allowed_boundary: &BTreeSet<HexCoord>,
) -> Result<Vec<(Vec<HexCoord>, Level)>, V3GenerationError> {
    for reverse in [false, true] {
        for offset in 0..sources.len() {
            let mut ordered_sources = sources.to_vec();
            ordered_sources.sort_unstable();
            if reverse {
                ordered_sources.reverse();
            }
            ordered_sources.rotate_left(offset);
            let assigned = ordered_sources
                .into_iter()
                .zip(outlets.iter().copied())
                .collect::<Vec<_>>();
            // The center thread of a broad cave mouth often has to claim its
            // corridor before either flank. Try every stable cyclic/reversed
            // routing order instead of making lexicographic outlet order a
            // hidden topology constraint.
            for route_reverse in [false, true] {
                for route_offset in 0..assigned.len() {
                    let mut ordered = assigned.clone();
                    if route_reverse {
                        ordered.reverse();
                    }
                    ordered.rotate_left(route_offset);
                    let mut occupied = BTreeSet::new();
                    let mut routed = Vec::new();
                    let mut failed = false;
                    for (source, (outlet, outlet_level)) in ordered {
                        let result = liquid_path(
                            mask,
                            source,
                            outlet,
                            &occupied,
                            shared_boundary,
                            allowed_boundary,
                            protected,
                        );
                        let Ok(path) = result else {
                            failed = true;
                            break;
                        };
                        let needed = usize::try_from(source_level.saturating_sub(outlet_level))
                            .unwrap_or(usize::MAX);
                        if path.len().saturating_sub(1) < needed {
                            failed = true;
                            break;
                        }
                        occupied.extend(path.iter().copied());
                        routed.push((path, outlet_level));
                    }
                    if !failed && routed.len() == outlets.len() {
                        return Ok(routed);
                    }
                }
            }
        }
    }
    Err(V3GenerationError::RecipeContract(
        "Macro headwater cannot route disjoint source threads to every outgoing lane".to_owned(),
    ))
}

fn representative_path_coord(paths: &[(Vec<HexCoord>, Level)], index: usize) -> Option<HexCoord> {
    let candidates = paths
        .iter()
        .filter_map(|(path, _)| path.get(index).copied())
        .collect::<Vec<_>>();
    candidates.iter().copied().min_by_key(|candidate| {
        let total_distance = candidates.iter().fold(0_u32, |total, other| {
            total.saturating_add(candidate.distance(*other))
        });
        (total_distance, *candidate)
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "rivulet routing consumes source, seam, and exclusion geometry"
)]
fn route_rivulet_confluence(
    mask: &BTreeSet<HexCoord>,
    sources: &[HexCoord],
    outlets: &[(HexCoord, Level)],
    source_level: Level,
    outlet_level: Level,
    protected: &BTreeSet<HexCoord>,
    shared_boundary: &BTreeSet<HexCoord>,
    allowed_boundary: &BTreeSet<HexCoord>,
) -> Result<RoutedHeadwater, V3GenerationError> {
    let outlet_coords = outlets
        .iter()
        .map(|(coord, _)| *coord)
        .collect::<BTreeSet<_>>();
    let Some(center_outlet) = outlets
        .iter()
        .map(|(coord, _)| *coord)
        .min_by_key(|candidate| {
            let total_distance = outlet_coords.iter().fold(0_u32, |total, other| {
                total.saturating_add(candidate.distance(*other))
            });
            (total_distance, *candidate)
        })
    else {
        return Err(V3GenerationError::RecipeContract(
            "Macro rivulet confluence has no outgoing lane".to_owned(),
        ));
    };
    let distances = distances_within(mask, &outlet_coords);
    let mut junctions = mask
        .iter()
        .copied()
        .filter(|coord| {
            distances.get(coord).copied().unwrap_or_default() >= 5
                && !sources.contains(coord)
                && !protected.contains(coord)
                && !shared_boundary.contains(coord)
        })
        .collect::<Vec<_>>();
    junctions.sort_unstable_by_key(|coord| {
        let source_distance = sources.iter().fold(0_u32, |total, source| {
            total.saturating_add(coord.distance(*source))
        });
        (
            distances
                .get(coord)
                .copied()
                .unwrap_or_default()
                .abs_diff(7),
            source_distance,
            canonical_coord_hash(*coord),
            *coord,
        )
    });
    let required_drop =
        usize::try_from(source_level.saturating_sub(outlet_level)).unwrap_or(usize::MAX);

    for junction in junctions {
        let Ok(trunk) = liquid_path(
            mask,
            junction,
            center_outlet,
            &BTreeSet::new(),
            shared_boundary,
            allowed_boundary,
            protected,
        ) else {
            continue;
        };
        let mut routed_feeders = None;
        for reverse in [false, true] {
            for offset in 0..sources.len() {
                let mut ordered = sources.to_vec();
                ordered.sort_unstable();
                if reverse {
                    ordered.reverse();
                }
                ordered.rotate_left(offset);
                let mut occupied = trunk
                    .iter()
                    .copied()
                    .filter(|coord| *coord != junction)
                    .collect::<BTreeSet<_>>();
                let mut feeders = Vec::new();
                let mut failed = false;
                for source in ordered {
                    let Ok(path) = liquid_path(
                        mask,
                        source,
                        junction,
                        &occupied,
                        shared_boundary,
                        allowed_boundary,
                        protected,
                    ) else {
                        failed = true;
                        break;
                    };
                    if path.len().saturating_sub(1) < required_drop {
                        failed = true;
                        break;
                    }
                    occupied.extend(path.iter().copied());
                    feeders.push(path);
                }
                if !failed && feeders.len() == sources.len() {
                    routed_feeders = Some((feeders, occupied));
                    break;
                }
            }
            if routed_feeders.is_some() {
                break;
            }
        }
        let Some((feeders, mut occupied)) = routed_feeders else {
            continue;
        };
        let mut paths = feeders
            .into_iter()
            .map(|feeder| RoutedHeadwaterPath {
                coords: feeder
                    .into_iter()
                    .chain(trunk.iter().copied().skip(1))
                    .collect(),
                start_level: source_level,
                end_level: outlet_level,
            })
            .collect::<Vec<_>>();

        let fan_starts = trunk
            .iter()
            .rev()
            .take(4)
            .flat_map(|coord| coord.neighbors())
            .filter(|coord| {
                mask.contains(coord)
                    && !occupied.contains(coord)
                    && !protected.contains(coord)
                    && (!shared_boundary.contains(coord) || outlet_coords.contains(coord))
            })
            .collect::<BTreeSet<_>>();
        let mut fan_failed = false;
        for side_outlet in outlet_coords
            .iter()
            .copied()
            .filter(|coord| *coord != center_outlet)
        {
            let mut candidates = fan_starts
                .iter()
                .copied()
                .filter(|start| !occupied.contains(start))
                .filter_map(|start| {
                    liquid_path(
                        mask,
                        start,
                        side_outlet,
                        &occupied,
                        shared_boundary,
                        allowed_boundary,
                        protected,
                    )
                    .ok()
                    .filter(|path| path.len() <= 5)
                })
                .collect::<Vec<_>>();
            candidates.sort_unstable_by_key(|path| {
                (path.len().abs_diff(3), Reverse(path.len()), path.clone())
            });
            let Some(path) = candidates.into_iter().next() else {
                fan_failed = true;
                break;
            };
            occupied.extend(path.iter().copied());
            paths.push(RoutedHeadwaterPath {
                coords: path,
                start_level: outlet_level,
                end_level: outlet_level,
            });
        }
        if !fan_failed {
            return Ok(RoutedHeadwater {
                paths,
                review_feature: junction,
            });
        }
    }

    Err(V3GenerationError::RecipeContract(
        "Macro rivulet sources cannot converge into one trunk before the outgoing seam".to_owned(),
    ))
}

fn preferred_headwater_review_bank(
    mask: &BTreeSet<HexCoord>,
    feature: HexCoord,
    water: &BTreeSet<HexCoord>,
    outlets: &[(HexCoord, Level)],
) -> Option<HexCoord> {
    let outlet_coords = outlets
        .iter()
        .map(|(coord, _)| *coord)
        .collect::<BTreeSet<_>>();
    let distances = distances_within(mask, &outlet_coords);
    feature
        .neighbors()
        .into_iter()
        .filter(|coord| mask.contains(coord) && !water.contains(coord))
        .min_by_key(|coord| (distances.get(coord).copied().unwrap_or(u32::MAX), *coord))
}

fn shape_headwater_landform(
    mask: &BTreeSet<HexCoord>,
    paths: &[Vec<TilePos>],
    levels: &mut BTreeMap<HexCoord, Level>,
    protected: &BTreeSet<HexCoord>,
    shared_boundary: &BTreeSet<HexCoord>,
    bank_clearance: Level,
) -> Result<(), V3GenerationError> {
    let water = paths
        .iter()
        .flatten()
        .map(|position| (position.coord, position.level))
        .collect::<BTreeMap<_, _>>();
    let mut banks = BTreeMap::<HexCoord, Level>::new();
    for position in paths.iter().flatten() {
        for neighbor in position.coord.neighbors() {
            if mask.contains(&neighbor) && !water.contains_key(&neighbor) {
                banks
                    .entry(neighbor)
                    .and_modify(|required| {
                        *required = (*required).max(position.level.saturating_add(bank_clearance));
                    })
                    .or_insert_with(|| position.level.saturating_add(bank_clearance));
            }
        }
    }
    for (coord, required) in &banks {
        let authored = levels.get(coord).copied().unwrap_or_default();
        if (protected.contains(coord) || shared_boundary.contains(coord)) && authored < *required {
            return Err(V3GenerationError::RecipeContract(format!(
                "Macro headwater needs immutable bank {coord:?} at {required}, got {authored}"
            )));
        }
        levels.insert(*coord, authored.max(*required));
    }
    // One broader shoulder prevents the containing bank from becoming a narrow
    // levee. It is intentionally a lower, one-level taper rather than another
    // vertical wall around the water.
    let bank_coords = banks.keys().copied().collect::<BTreeSet<_>>();
    for (bank, bank_level) in banks {
        for outward in bank.neighbors() {
            if !mask.contains(&outward)
                || water.contains_key(&outward)
                || bank_coords.contains(&outward)
            {
                continue;
            }
            let required = bank_level.saturating_sub(1);
            let authored = levels.get(&outward).copied().unwrap_or_default();
            if (protected.contains(&outward) || shared_boundary.contains(&outward))
                && authored < required
            {
                return Err(V3GenerationError::RecipeContract(format!(
                    "Macro headwater needs immutable bank shoulder {outward:?} at {required}, got {authored}"
                )));
            }
            levels.insert(outward, authored.max(required));
        }
    }
    Ok(())
}

fn trim_coastal_water_coverage(
    mask: &BTreeSet<HexCoord>,
    instance: &MacroBiomeInstanceSettings,
    standing_lanes: &BTreeSet<(HexCoord, Level)>,
    bodies: &mut [BTreeMap<TilePos, LiquidNode>],
    top_by_coord: &mut BTreeMap<HexCoord, Level>,
) {
    let target_percent = match &instance.recipe {
        V3RecipeSettings::Beach(settings) => usize::from(settings.water_coverage_percent),
        V3RecipeSettings::Shore(settings) => usize::from(settings.water_coverage_percent),
        _ => return,
    };
    let target = mask.len().saturating_mul(target_percent) / 100;
    if top_by_coord.len() <= target {
        return;
    }
    let sources = standing_lanes
        .iter()
        .map(|(coord, _)| *coord)
        .collect::<BTreeSet<_>>();
    let distances = distances_within(mask, &sources);
    let mut required = sources;
    for body in bodies.iter() {
        for (position, node) in body {
            if node.state != LiquidFlowState::Still || node.downstream.is_some() {
                required.insert(position.coord);
            }
            if let Some(downstream) = node.downstream {
                required.insert(downstream.coord);
            }
        }
    }
    let remove_count = top_by_coord.len().saturating_sub(target);
    let mut removable = top_by_coord
        .keys()
        .copied()
        .filter(|coord| !required.contains(coord))
        .collect::<Vec<_>>();
    removable.sort_unstable_by_key(|coord| {
        (
            Reverse(distances.get(coord).copied().unwrap_or(u32::MAX)),
            Reverse(*coord),
        )
    });
    for coord in removable.into_iter().take(remove_count) {
        let Some(level) = top_by_coord.remove(&coord) else {
            continue;
        };
        let position = TilePos::new(coord, level);
        for body in bodies.iter_mut() {
            body.remove(&position);
        }
    }
}

fn exact_liquid_level(elevation: ResolvedLiquidElevation, fallback: Level) -> Level {
    match elevation {
        ResolvedLiquidElevation::EdgeBand => fallback,
        ResolvedLiquidElevation::Exact(level) => level,
    }
}

fn coastal_water_footprint(
    mask: &BTreeSet<HexCoord>,
    instance: &MacroBiomeInstanceSettings,
    standing_lanes: &BTreeSet<(HexCoord, Level)>,
    allowed_boundary: &BTreeSet<HexCoord>,
    all_shared_boundary: &BTreeSet<HexCoord>,
    protected: &BTreeSet<HexCoord>,
) -> BTreeMap<HexCoord, Level> {
    let (target_percent, sea_level) = match &instance.recipe {
        V3RecipeSettings::ShallowSea(settings) => (100_usize, settings.sea_level),
        V3RecipeSettings::Beach(settings) => (usize::from(settings.water_coverage_percent), 8),
        V3RecipeSettings::Shore(settings) => (usize::from(settings.water_coverage_percent), 8),
        _ => return BTreeMap::new(),
    };
    if target_percent == 100 {
        return mask
            .iter()
            .copied()
            .map(|coord| (coord, sea_level))
            .collect();
    }
    let sources = standing_lanes
        .iter()
        .map(|(coord, _)| *coord)
        .collect::<BTreeSet<_>>();
    let distances = distances_within(mask, &sources);
    let mut eligible = mask
        .iter()
        .copied()
        .filter(|coord| {
            !sources.contains(coord)
                && !protected.contains(coord)
                && (!all_shared_boundary.contains(coord) || allowed_boundary.contains(coord))
        })
        .collect::<Vec<_>>();
    eligible
        .sort_unstable_by_key(|coord| (distances.get(coord).copied().unwrap_or(u32::MAX), *coord));
    let target = mask.len().saturating_mul(target_percent) / 100;
    let mut wet = sources;
    wet.extend(eligible.into_iter().take(target.saturating_sub(wet.len())));
    wet.into_iter().map(|coord| (coord, sea_level)).collect()
}

fn distances_within(
    mask: &BTreeSet<HexCoord>,
    sources: &BTreeSet<HexCoord>,
) -> BTreeMap<HexCoord, u32> {
    let mut distances = BTreeMap::<HexCoord, u32>::new();
    let mut pending = VecDeque::new();
    for source in sources.iter().copied().filter(|coord| mask.contains(coord)) {
        distances.insert(source, 0);
        pending.push_back(source);
    }
    while let Some(coord) = pending.pop_front() {
        let next = distances
            .get(&coord)
            .copied()
            .unwrap_or_default()
            .saturating_add(1);
        for neighbor in coord.neighbors() {
            if mask.contains(&neighbor) && !distances.contains_key(&neighbor) {
                distances.insert(neighbor, next);
                pending.push_back(neighbor);
            }
        }
    }
    distances
}

fn flatten_ports(ports: &[DirectedPort]) -> Vec<(HexCoord, Level)> {
    ports
        .iter()
        .flat_map(|port| port.coords.iter().copied().map(|coord| (coord, port.level)))
        .collect()
}

#[expect(
    clippy::too_many_arguments,
    reason = "target selection must honor every liquid routing exclusion"
)]
fn reachable_liquid_target(
    mask: &BTreeSet<HexCoord>,
    start: HexCoord,
    targets: &[TilePos],
    salt: usize,
    occupied: &BTreeSet<HexCoord>,
    shared_boundary: &BTreeSet<HexCoord>,
    allowed_boundary: &BTreeSet<HexCoord>,
    protected: &BTreeSet<HexCoord>,
) -> Option<(TilePos, Vec<HexCoord>)> {
    let mut ordered = targets.to_vec();
    ordered.sort_unstable_by_key(|target| (start.distance(target.coord), target.coord, salt));
    let strict = ordered.iter().copied().find_map(|target| {
        liquid_path(
            mask,
            start,
            target.coord,
            occupied,
            shared_boundary,
            allowed_boundary,
            protected,
        )
        .ok()
        .map(|path| (target, path))
    });
    strict.or_else(|| {
        let reusable_water = BTreeSet::new();
        ordered.into_iter().find_map(|target| {
            liquid_path(
                mask,
                start,
                target.coord,
                &reusable_water,
                shared_boundary,
                allowed_boundary,
                protected,
            )
            .ok()
            .map(|path| (target, path))
        })
    })
}

fn liquid_path(
    mask: &BTreeSet<HexCoord>,
    start: HexCoord,
    goal: HexCoord,
    occupied: &BTreeSet<HexCoord>,
    shared_boundary: &BTreeSet<HexCoord>,
    allowed_boundary: &BTreeSet<HexCoord>,
    protected: &BTreeSet<HexCoord>,
) -> Result<Vec<HexCoord>, V3GenerationError> {
    let admitted = |coord: HexCoord| {
        mask.contains(&coord)
            && (!occupied.contains(&coord) || coord == start || coord == goal)
            && (!protected.contains(&coord) || coord == start || coord == goal)
            && (!shared_boundary.contains(&coord)
                || allowed_boundary.contains(&coord)
                || coord == start
                || coord == goal)
    };
    shortest_coord_path(start, goal, admitted).ok_or_else(|| {
        V3GenerationError::RecipeContract(format!(
            "Macro liquid channel cannot connect {start:?} to {goal:?}"
        ))
    })
}

fn shortest_coord_path(
    start: HexCoord,
    goal: HexCoord,
    admitted: impl Fn(HexCoord) -> bool,
) -> Option<Vec<HexCoord>> {
    let mut previous = BTreeMap::from([(start, start)]);
    let mut pending = VecDeque::from([start]);
    while let Some(coord) = pending.pop_front() {
        if coord == goal {
            break;
        }
        for neighbor in coord.neighbors() {
            if admitted(neighbor) && !previous.contains_key(&neighbor) {
                previous.insert(neighbor, coord);
                pending.push_back(neighbor);
            }
        }
    }
    if !previous.contains_key(&goal) {
        return None;
    }
    let mut path = vec![goal];
    let mut current = goal;
    while current != start {
        current = *previous.get(&current)?;
        path.push(current);
    }
    path.reverse();
    Some(path)
}

fn append_flow_path(
    body: &mut BTreeMap<TilePos, LiquidNode>,
    top_by_coord: &mut BTreeMap<HexCoord, Level>,
    path: &[HexCoord],
    start_level: Level,
    end_level: Level,
    terminal: Option<TilePos>,
    waterfall_flow: bool,
) -> Result<(), V3GenerationError> {
    if start_level < end_level {
        return Err(V3GenerationError::RecipeContract(format!(
            "Macro liquid channel rises from {start_level} to {end_level}"
        )));
    }
    let transitions = path.len().saturating_sub(1);
    let drop = usize::try_from(start_level.saturating_sub(end_level)).unwrap_or(usize::MAX);
    if drop > transitions {
        return Err(V3GenerationError::RecipeContract(format!(
            "Macro liquid channel needs {drop} drops but has only {transitions} transitions"
        )));
    }
    let positions = path
        .iter()
        .copied()
        .enumerate()
        .map(|(index, coord)| {
            let dropped = if waterfall_flow && drop >= 2 {
                index.saturating_mul(2).min(drop)
            } else {
                index.min(drop)
            };
            let level = start_level.saturating_sub(i32::try_from(dropped).unwrap_or(i32::MAX));
            TilePos::new(coord, level)
        })
        .collect::<Vec<_>>();
    append_position_path(body, top_by_coord, &positions, terminal, waterfall_flow);
    Ok(())
}

fn append_cave_fall_path(
    body: &mut BTreeMap<TilePos, LiquidNode>,
    top_by_coord: &mut BTreeMap<HexCoord, Level>,
    path: &[HexCoord],
    start_level: Level,
    end_level: Level,
    overhang_depth: u8,
) -> Result<(), V3GenerationError> {
    if start_level < end_level {
        return Err(V3GenerationError::RecipeContract(format!(
            "Macro cave source rises from {start_level} to {end_level}"
        )));
    }
    let depth = usize::from(overhang_depth);
    let transitions = path.len().saturating_sub(1);
    let drop = usize::try_from(start_level.saturating_sub(end_level)).unwrap_or(usize::MAX);
    let lip_drop = drop.min(3);
    let runout_drop = drop.saturating_sub(lip_drop);
    if depth == 0 || transitions < depth || runout_drop > transitions.saturating_sub(depth) {
        return Err(V3GenerationError::RecipeContract(format!(
            "Macro cave source needs {depth} covered columns and {drop} drops but has only {transitions} transitions"
        )));
    }
    let positions = path
        .iter()
        .copied()
        .enumerate()
        .map(|(index, coord)| {
            let dropped = if index < depth {
                0
            } else {
                lip_drop.saturating_add(index.saturating_sub(depth).min(runout_drop))
            };
            let level = start_level.saturating_sub(i32::try_from(dropped).unwrap_or(i32::MAX));
            TilePos::new(coord, level)
        })
        .collect::<Vec<_>>();
    append_position_path(body, top_by_coord, &positions, None, true);
    Ok(())
}

fn append_position_path(
    body: &mut BTreeMap<TilePos, LiquidNode>,
    top_by_coord: &mut BTreeMap<HexCoord, Level>,
    positions: &[TilePos],
    terminal: Option<TilePos>,
    waterfall_flow: bool,
) {
    for (index, position) in positions.iter().copied().enumerate() {
        replace_liquid_surface(body, top_by_coord, position);
        let downstream = positions.get(index + 1).copied().or(terminal);
        let following = positions
            .get(index + 2)
            .copied()
            .or_else(|| (index + 2 == positions.len()).then_some(terminal).flatten());
        let state = match downstream {
            None => LiquidFlowState::Still,
            Some(next) if waterfall_flow && position.level.saturating_sub(next.level) >= 2 => {
                LiquidFlowState::Fall
            }
            Some(next) if waterfall_flow && position.level > next.level => {
                // A one-level descending reach is fast, broken water leading out
                // of the vertical fall rather than an ordinary flat current.
                LiquidFlowState::Rapid
            }
            Some(next)
                if waterfall_flow && following.is_some_and(|after| after.level < next.level) =>
            {
                LiquidFlowState::Rapid
            }
            Some(_) => LiquidFlowState::Current,
        };
        body.insert(position, LiquidNode { state, downstream });
    }
}

fn append_still_path(
    body: &mut BTreeMap<TilePos, LiquidNode>,
    top_by_coord: &mut BTreeMap<HexCoord, Level>,
    path: &[HexCoord],
    level: Level,
) {
    for coord in path {
        let position = TilePos::new(*coord, level);
        replace_liquid_surface(body, top_by_coord, position);
        body.entry(position).or_insert(LiquidNode {
            state: LiquidFlowState::Still,
            downstream: None,
        });
    }
}

fn replace_liquid_surface(
    body: &mut BTreeMap<TilePos, LiquidNode>,
    top_by_coord: &mut BTreeMap<HexCoord, Level>,
    replacement: TilePos,
) {
    let Some(previous_level) = top_by_coord.insert(replacement.coord, replacement.level) else {
        return;
    };
    if previous_level == replacement.level {
        return;
    }
    let previous = TilePos::new(replacement.coord, previous_level);
    body.remove(&previous);
    for node in body.values_mut() {
        if node.downstream == Some(previous) {
            node.downstream = Some(replacement);
        }
    }
}

fn build_volume(
    mask: &BTreeSet<HexCoord>,
    instance: &MacroBiomeInstanceSettings,
    alpine_climate: MacroAlpineClimate,
    levels: &BTreeMap<HexCoord, Level>,
    liquid_tops: &BTreeMap<HexCoord, Level>,
    liquids: &LiquidPlan,
    bridgeable_route: &BTreeSet<HexCoord>,
    overhangs: &BTreeMap<HexCoord, LevelInterval>,
) -> VolumePlan {
    let mut columns = BTreeMap::new();
    let mut surfaces = BTreeMap::new();
    let water_coords = liquid_tops.keys().copied().collect::<BTreeSet<_>>();
    let coastal_distances = distances_within(mask, &water_coords);
    for coord in mask.iter().copied() {
        let dry_level = levels
            .get(&coord)
            .copied()
            .unwrap_or(instance.elevation.low);
        let liquid_top = liquid_tops.get(&coord).copied();
        let bridge_level = liquid_top
            .filter(|_| bridgeable_route.contains(&coord))
            .map(|top| dry_level.max(top.saturating_add(1)));
        let ordinary_ground_level = liquid_top.map_or(dry_level, |top| {
            if matches!(instance.recipe, V3RecipeSettings::ShallowSea(_)) {
                4
            } else if matches!(instance.recipe, V3RecipeSettings::Shore(_)) {
                top.saturating_sub(1).max(4)
            } else {
                top.saturating_sub(2).max(4)
            }
        });
        let fall_ground_level = liquids.bodies.values().find_map(|body| {
            body.nodes.iter().find_map(|(position, node)| {
                (position.coord == coord && node.state == LiquidFlowState::Fall)
                    .then_some(node.downstream?.level)
            })
        });
        let ground_level = fall_ground_level
            .map(|fall_level| ordinary_ground_level.min(fall_level))
            .unwrap_or(ordinary_ground_level);
        let coastal_distance = coastal_distances.get(&coord).copied();
        let ground_material =
            surface_material(instance, ground_level, coastal_distance, alpine_climate);
        let mut column = solid_column(instance, ground_level, ground_material);
        if let Some(water_top) = liquid_top {
            column.elements.push(VolumeElement::Fill(NonSolidFill {
                levels: LevelInterval::new(
                    ground_level.saturating_add(1),
                    water_top.saturating_add(1),
                ),
                material: FillMaterialRole::Water,
            }));
        }
        if let Some(bridge_level) = bridge_level {
            column.elements.push(solid(
                bridge_level,
                bridge_level.saturating_add(1),
                surface_material(instance, bridge_level, coastal_distance, alpine_climate),
            ));
            surfaces.insert(
                TilePos::new(coord, bridge_level),
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
            );
        }
        columns.insert(coord, column);
        surfaces.insert(
            TilePos::new(coord, ground_level),
            SurfaceMetadata {
                access: if liquid_top.is_some() {
                    SurfaceAccess::NonStandable
                } else {
                    SurfaceAccess::Ordinary
                },
                interior: None,
            },
        );
        if let Some(levels) = overhangs.get(&coord).copied() {
            if let Some(column) = columns.get_mut(&coord) {
                column
                    .elements
                    .push(solid(levels.bottom, levels.top, SolidMaterialRole::Stone));
            }
            surfaces.insert(
                TilePos::new(coord, levels.top.saturating_sub(1)),
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
            );
        }
    }
    VolumePlan {
        mask: mask.clone(),
        columns,
        surfaces,
    }
}

fn solid_column(
    instance: &MacroBiomeInstanceSettings,
    surface: Level,
    surface_material: SolidMaterialRole,
) -> VolumeColumn {
    if matches!(instance.recipe, V3RecipeSettings::ShallowSea(_)) {
        return VolumeColumn {
            elements: vec![
                solid(0, 1, SolidMaterialRole::Bedrock),
                solid(1, 3, SolidMaterialRole::Stone),
                solid(3, 4, SolidMaterialRole::Dirt),
                solid(4, 5, SolidMaterialRole::Sand),
            ],
        };
    }
    let mut elements = vec![solid(0, 1, SolidMaterialRole::Bedrock)];
    match surface_material {
        SolidMaterialRole::Grass => {
            elements.push(solid(
                1,
                surface.saturating_sub(1),
                SolidMaterialRole::Stone,
            ));
            elements.push(solid(
                surface.saturating_sub(1),
                surface,
                SolidMaterialRole::Dirt,
            ));
            elements.push(solid(
                surface,
                surface.saturating_add(1),
                SolidMaterialRole::Grass,
            ));
        }
        SolidMaterialRole::Sand => {
            elements.push(solid(
                1,
                surface.saturating_sub(1),
                SolidMaterialRole::Stone,
            ));
            elements.push(solid(
                surface.saturating_sub(1),
                surface,
                SolidMaterialRole::Dirt,
            ));
            elements.push(solid(
                surface,
                surface.saturating_add(1),
                SolidMaterialRole::Sand,
            ));
        }
        SolidMaterialRole::Snow => {
            elements.push(solid(1, surface, SolidMaterialRole::Stone));
            elements.push(solid(
                surface,
                surface.saturating_add(1),
                SolidMaterialRole::Snow,
            ));
        }
        material => elements.push(solid(1, surface.saturating_add(1), material)),
    }
    elements.retain(|element| match element {
        VolumeElement::Solid(mass) => mass.levels.bottom < mass.levels.top,
        VolumeElement::Fill(fill) => fill.levels.bottom < fill.levels.top,
    });
    VolumeColumn { elements }
}

const fn solid(bottom: Level, top: Level, material: SolidMaterialRole) -> VolumeElement {
    VolumeElement::Solid(SolidMass {
        levels: LevelInterval::new(bottom, top),
        material,
        cutaway_for: None,
    })
}

fn surface_material(
    instance: &MacroBiomeInstanceSettings,
    surface: Level,
    coastal_distance: Option<u32>,
    alpine_climate: MacroAlpineClimate,
) -> SolidMaterialRole {
    match &instance.recipe {
        V3RecipeSettings::ShallowSea(_) => SolidMaterialRole::Sand,
        V3RecipeSettings::Beach(_) if coastal_distance.is_some_and(|distance| distance <= 4) => {
            SolidMaterialRole::Sand
        }
        V3RecipeSettings::Shore(_) if surface <= 10 => SolidMaterialRole::Sand,
        V3RecipeSettings::Mountains(_) | V3RecipeSettings::DeepMountain(_)
            if surface >= alpine_climate.snowline =>
        {
            SolidMaterialRole::Snow
        }
        V3RecipeSettings::Mountains(_) | V3RecipeSettings::DeepMountain(_) => {
            SolidMaterialRole::Stone
        }
        _ => SolidMaterialRole::Grass,
    }
}

fn local_anchors(
    instance: &MacroBiomeInstanceSettings,
    settings: &MacroLayoutSettings,
    patch: &PatchRecipeContext<'_>,
    volume: &VolumePlan,
    liquid_tops: &BTreeMap<HexCoord, Level>,
    headwater_review: Option<HeadwaterReviewHint>,
) -> Result<BTreeMap<String, TilePos>, V3GenerationError> {
    let mut names = Vec::new();
    if settings.critical_route.first() == Some(&instance.name) {
        names.push(PARTY_START);
    }
    let hostile_index = settings.critical_route.len().saturating_sub(1).min(2);
    if settings.critical_route.get(hostile_index) == Some(&instance.name) {
        names.push(HOSTILE_START);
    }
    if settings.critical_route.last() == Some(&instance.name) {
        names.push(MACRO_ROUTE_END);
    }
    match instance.name.as_str() {
        "beach-lower" => names.push(BEACH_REVIEW),
        "shore-center" => names.push(COAST_REVIEW),
        "prairie-route" => names.push(INLAND_REVIEW),
        "hills-center" => names.push(FOOTHILL_REVIEW),
        "mountains-tier2-center" => names.push(MASSIF_FRONT_REVIEW),
        "deep-mountain" => names.extend([DEEP_MOUNTAIN_BASE, DEEP_MOUNTAIN_REVIEW]),
        _ => {}
    }
    if let Some(headwater) = settings
        .headwaters
        .iter()
        .find(|headwater| headwater_instance(headwater) == instance.name)
    {
        names.push(match headwater {
            MacroHeadwaterSettings::CaveFall { .. } => CAVE_SOURCE_REVIEW,
            MacroHeadwaterSettings::RivuletConfluence { .. } => RIVULET_SOURCE_REVIEW,
        });
    }
    if names.is_empty() {
        return Ok(BTreeMap::new());
    }
    let route_approaches = patch.walker_protected_approaches();
    let mut ordinary_dry = volume
        .surfaces
        .iter()
        .filter(|(surface, metadata)| {
            metadata.access == SurfaceAccess::Ordinary && !liquid_tops.contains_key(&surface.coord)
        })
        .map(|(surface, _)| *surface)
        .collect::<Vec<_>>();
    ordinary_dry.sort_by_key(|surface| {
        (
            surface.coord.distance(HexCoord::ORIGIN),
            Reverse(surface.level),
            *surface,
        )
    });

    let mut route_candidates = ordinary_dry
        .iter()
        .copied()
        .filter(|surface| route_approaches.contains(&surface.coord))
        .collect::<Vec<_>>();
    route_candidates.sort_by_key(|surface| {
        (
            surface.coord.distance(HexCoord::ORIGIN),
            surface.level,
            *surface,
        )
    });

    // Review anchors retain the old preference for protected route approaches, but
    // may use any ordinary dry surface once those are exhausted. Functional actor
    // and route anchors below are deliberately stricter: silently moving one off
    // the protected route would invalidate the authored traversal contract.
    let mut review_candidates = route_candidates.clone();
    review_candidates.extend(
        ordinary_dry
            .iter()
            .copied()
            .filter(|surface| !route_approaches.contains(&surface.coord)),
    );

    let mut coast_candidates = ordinary_dry
        .iter()
        .copied()
        .filter(|surface| {
            surface
                .coord
                .neighbors()
                .into_iter()
                .any(|neighbor| liquid_tops.contains_key(&neighbor))
        })
        .collect::<Vec<_>>();
    coast_candidates.sort_by_key(|surface| (surface.level, surface.coord, *surface));

    let source_coord = headwater_review.map(|review| review.feature).or_else(|| {
        liquid_tops
            .iter()
            .max_by_key(|(coord, level)| (**level, Reverse(**coord)))
            .map(|(coord, _)| *coord)
    });
    let mut source_candidates = ordinary_dry.clone();
    source_candidates.sort_by_key(|surface| {
        (
            headwater_review
                .and_then(|review| review.preferred_bank)
                .is_none_or(|preferred| surface.coord != preferred),
            headwater_review
                .and_then(|review| review.preferred_bank)
                .map_or(u32::MAX, |preferred| preferred.distance(surface.coord)),
            source_coord.map_or(u32::MAX, |source| source.distance(surface.coord)),
            Reverse(surface.level),
            *surface,
        )
    });

    let mut used_coords = BTreeSet::new();
    let mut anchors = BTreeMap::new();
    for name in names {
        let (candidates, requirement) =
            if matches!(name, PARTY_START | HOSTILE_START | MACRO_ROUTE_END) {
                (
                    route_candidates.clone(),
                    "ordinary dry protected-route surface",
                )
            } else if matches!(name, COAST_REVIEW | BEACH_REVIEW) {
                (coast_candidates.clone(), "ordinary dry water-edge surface")
            } else if matches!(name, CAVE_SOURCE_REVIEW | RIVULET_SOURCE_REVIEW) {
                (
                    source_candidates.clone(),
                    "ordinary dry headwater-edge surface",
                )
            } else if used_coords.is_empty() {
                (review_candidates.clone(), "ordinary dry review surface")
            } else {
                let mut near_existing_anchor = ordinary_dry.clone();
                near_existing_anchor.sort_by_key(|surface| {
                    let nearest_anchor = used_coords
                        .iter()
                        .map(|coord| surface.coord.distance(*coord))
                        .min()
                        .unwrap_or_default();
                    (
                        nearest_anchor,
                        !route_approaches.contains(&surface.coord),
                        surface.coord.distance(HexCoord::ORIGIN),
                        Reverse(surface.level),
                        *surface,
                    )
                });
                (near_existing_anchor, "ordinary dry review surface")
            };
        let anchor = candidates
            .into_iter()
            .find(|surface| !used_coords.contains(&surface.coord))
            .ok_or_else(|| {
                V3GenerationError::RecipeContract(format!(
                    "Macro instance {:?} has no unused {requirement} for anchor {name:?}",
                    instance.name
                ))
            })?;
        used_coords.insert(anchor.coord);
        anchors.insert(name.to_owned(), anchor);
    }
    Ok(anchors)
}

#[expect(
    clippy::too_many_arguments,
    reason = "feature placement consumes every exclusion owned by the generated fragment"
)]
fn place_vegetation(
    instance: &MacroBiomeInstanceSettings,
    patch: &PatchRecipeContext<'_>,
    volume: &VolumePlan,
    alpine_climate: MacroAlpineClimate,
    anchors: &BTreeMap<String, TilePos>,
    protected: &BTreeSet<HexCoord>,
    world_support: &BTreeMap<HexCoord, Level>,
    streams: Option<SeedStreams>,
    vegetation: &TemperateVegetationSet,
    preserve_complete_ordinary_connectivity: bool,
) -> Result<(FeaturePlan, BTreeSet<TilePos>), V3GenerationError> {
    // Keep the alpine face visually above the treeline. The sparse tree layers in
    // the green foothill recipes form the missing coast-to-mountain ecotone while
    // retaining each recipe's independently authored grass layer.
    let layers = match &instance.recipe {
        V3RecipeSettings::Beach(settings) => vec![MacroVegetationLayer::trees(
            settings.tree_coverage_percent,
            &vegetation.small_broadleaf,
            "macro.coast.trees",
            false,
        )],
        V3RecipeSettings::Shore(settings) => vec![MacroVegetationLayer::trees(
            settings.tree_coverage_percent,
            &vegetation.small_broadleaf,
            "macro.coast.trees",
            false,
        )],
        V3RecipeSettings::Forest(_) | V3RecipeSettings::DeepForest(_) => {
            vec![MacroVegetationLayer::trees(
                18,
                &vegetation.small_broadleaf,
                "macro.forest.trees",
                true,
            )]
        }
        V3RecipeSettings::Prairie(settings) => vec![
            MacroVegetationLayer::trees(
                4,
                &vegetation.small_broadleaf,
                "macro.foothill.trees",
                true,
            ),
            MacroVegetationLayer::grass(
                settings.grass_coverage_percent,
                &vegetation.grass_tuft,
                "macro.prairie.grass",
            ),
        ],
        V3RecipeSettings::Hills(_) => vec![
            MacroVegetationLayer::trees(
                10,
                &vegetation.small_broadleaf,
                "macro.foothill.trees",
                true,
            ),
            MacroVegetationLayer::grass(28, &vegetation.grass_tuft, "macro.hills.grass"),
        ],
        V3RecipeSettings::Waterfall(_) => vec![
            MacroVegetationLayer::trees(
                8,
                &vegetation.small_broadleaf,
                "macro.foothill.trees",
                true,
            ),
            MacroVegetationLayer::grass(18, &vegetation.grass_tuft, "macro.waterfall.grass"),
        ],
        // Mountains and Deep Mountain deliberately carry no trees. Their visual
        // treeline is expressed by the clustered foothill layers immediately below.
        _ => Vec::new(),
    };
    if layers.is_empty() {
        return Ok((FeaturePlan::default(), BTreeSet::new()));
    }
    let anchor_surfaces = anchors.values().copied().collect::<BTreeSet<_>>();
    let mut vegetation_protected = protected.clone();
    vegetation_protected.extend(
        patch
            .layout()
            .shared_edges
            .values()
            .flat_map(|edge| edge.protected_approaches.values())
            .flatten()
            .copied(),
    );
    let local_surface_by_coord = volume
        .surfaces
        .iter()
        .filter_map(|(surface, metadata)| {
            matches!(
                metadata.access,
                SurfaceAccess::Ordinary | SurfaceAccess::SpecialMovement(_)
            )
            .then_some((surface.coord, *surface))
        })
        .collect::<BTreeMap<_, _>>();
    // Canopies may cross an ownership seam even though their root and blocker
    // remain local to this biome instance. Use the already-planned world-space
    // temperate field as support beyond the fragment so an invisible mask edge
    // does not create a one-column treeless hex outline.
    let mut surface_by_coord = world_support
        .iter()
        .map(|(coord, level)| (*coord, TilePos::new(*coord, *level)))
        .collect::<BTreeMap<_, _>>();
    surface_by_coord.extend(
        local_surface_by_coord
            .iter()
            .map(|(coord, surface)| (*coord, *surface)),
    );
    let mut features = BTreeMap::new();
    let mut blockers = BTreeSet::new();
    let mut occupied_roots = BTreeSet::new();
    for layer in layers {
        let stream = streams.map(|streams| streams.stage(layer.seed_stage));
        let mut eligible = local_surface_by_coord
            .values()
            .copied()
            .filter(|surface| {
                !anchor_surfaces.contains(surface)
                    && !vegetation_protected.contains(&surface.coord)
                    && !occupied_roots.contains(&surface.coord)
                    && !blockers
                        .iter()
                        .any(|blocker: &TilePos| blocker.coord == surface.coord)
                    && vegetation_below_climate_ceiling(
                        &instance.recipe,
                        layer.kind,
                        surface.level,
                        alpine_climate,
                    )
            })
            .collect::<Vec<_>>();
        eligible.sort_unstable_by_key(|surface| {
            (
                vegetation_rank(stream, surface.coord, layer.clustered),
                *surface,
            )
        });
        let target = eligible.len().saturating_mul(usize::from(layer.percent)) / 100;
        let mut placements = Vec::with_capacity(eligible.len());
        for root in eligible {
            let rotation = HexObjectRotation::new(
                u8::try_from(
                    vegetation_sample(stream, root.coord, 1)
                        .wrapping_add(u64::from(layer.kind == FeatureKind::TallGrass))
                        % 6,
                )
                .unwrap_or_default(),
            )
            .map_err(|error| {
                V3GenerationError::RecipeContract(format!(
                    "Macro vegetation rotation failed in patch {}: {error}",
                    patch.id.0
                ))
            })?;
            let Some(visual) = layer.object.project_visual_volume(root, rotation) else {
                continue;
            };
            if visual.cells.iter().any(|cell| {
                vegetation_protected.contains(&cell.coord)
                    || anchor_surfaces
                        .iter()
                        .any(|anchor| anchor.coord == cell.coord)
                    || surface_by_coord
                        .get(&cell.coord)
                        .is_none_or(|support| cell.level <= support.level)
            }) {
                continue;
            }
            let blocker_footprint = if layer.kind == FeatureKind::Tree {
                let Some(projected) =
                    layer
                        .object
                        .project_blockers(root, rotation, &surface_by_coord)
                else {
                    continue;
                };
                projected
            } else {
                BTreeSet::new()
            };
            placements.push((root, rotation, blocker_footprint));
        }
        if placements.len() < target {
            return Err(V3GenerationError::RecipeContract(format!(
                "Macro {:?} needs {target} {} placements but only {} project safely",
                instance.name,
                layer.seed_stage,
                placements.len()
            )));
        }
        let selected = if preserve_complete_ordinary_connectivity && layer.kind == FeatureKind::Tree
        {
            connected_vegetation_placements(volume, &blockers, placements, target)
        } else {
            placements.into_iter().take(target).collect::<Vec<_>>()
        };
        if selected.len() < target {
            return Err(V3GenerationError::RecipeContract(format!(
                "Macro {:?} needs {target} connected {} placements but only {} preserve the complete ordinary network",
                instance.name,
                layer.seed_stage,
                selected.len()
            )));
        }
        for (root, rotation, blocker_footprint) in selected {
            occupied_roots.insert(root.coord);
            blockers.extend(blocker_footprint.iter().copied());
            let feature_id = FeatureId(u32::try_from(features.len()).unwrap_or(u32::MAX));
            features.insert(
                feature_id,
                PlannedFeature {
                    root,
                    kind: layer.kind,
                    object_id: layer.object.id.clone(),
                    rotation,
                    blocker_footprint,
                },
            );
        }
    }
    Ok((
        FeaturePlan {
            by_id: features,
            ..Default::default()
        },
        blockers,
    ))
}

type MacroVegetationPlacement = (TilePos, HexObjectRotation, BTreeSet<TilePos>);

fn connected_vegetation_placements(
    volume: &VolumePlan,
    existing_blockers: &BTreeSet<TilePos>,
    placements: Vec<MacroVegetationPlacement>,
    target: usize,
) -> Vec<MacroVegetationPlacement> {
    let ordinary = OrdinaryGraph::from_volume(volume, Some(existing_blockers));
    let positions = ordinary.positions().collect::<Vec<_>>();
    let indices = positions
        .iter()
        .copied()
        .enumerate()
        .map(|(index, position)| (position, index))
        .collect::<BTreeMap<_, _>>();
    let neighbors = positions
        .iter()
        .map(|position| {
            ordinary
                .neighbors(*position)
                .iter()
                .filter_map(|neighbor| indices.get(neighbor).copied())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut connectivity = IndexedConnectivity::new(neighbors);
    let mut selected = Vec::with_capacity(target);
    for placement in placements {
        if selected.len() == target {
            break;
        }
        let candidate = placement
            .2
            .iter()
            .filter_map(|blocker| indices.get(blocker).copied())
            .collect::<Vec<_>>();
        if connectivity.try_block(&candidate) {
            selected.push(placement);
        }
    }
    selected
}

/// Allocation-free connectivity checks for deterministic greedy blocker placement.
///
/// The summit forest evaluates hundreds of ranked tree roots. Rebuilding ordered
/// blocker and reachability sets for every root dominated Macro generation, even
/// though the underlying ordinary graph never changes. Dense indices retain the
/// exact same sequential rule while reusing one frontier and visitation table.
struct IndexedConnectivity {
    neighbors: Vec<Vec<usize>>,
    blocked: Vec<bool>,
    visited_epoch: Vec<u32>,
    epoch: u32,
    frontier: VecDeque<usize>,
    active: usize,
}

impl IndexedConnectivity {
    fn new(neighbors: Vec<Vec<usize>>) -> Self {
        let active = neighbors.len();
        Self {
            blocked: vec![false; active],
            visited_epoch: vec![0; active],
            neighbors,
            epoch: 0,
            frontier: VecDeque::new(),
            active,
        }
    }

    fn try_block(&mut self, candidate: &[usize]) -> bool {
        let newly_blocked = candidate
            .iter()
            .copied()
            .filter(|index| self.blocked.get(*index).is_some_and(|blocked| !blocked))
            .collect::<Vec<_>>();
        for index in &newly_blocked {
            if let Some(blocked) = self.blocked.get_mut(*index) {
                *blocked = true;
            }
        }
        let remaining = self.active.saturating_sub(newly_blocked.len());
        if remaining != 0 && self.reachable_count() == remaining {
            self.active = remaining;
            true
        } else {
            for index in newly_blocked {
                if let Some(blocked) = self.blocked.get_mut(index) {
                    *blocked = false;
                }
            }
            false
        }
    }

    fn reachable_count(&mut self) -> usize {
        let Some(start) = self.blocked.iter().position(|blocked| !blocked) else {
            return 0;
        };
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.visited_epoch.fill(0);
            self.epoch = 1;
        }
        self.frontier.clear();
        self.frontier.push_back(start);
        if let Some(visited) = self.visited_epoch.get_mut(start) {
            *visited = self.epoch;
        }
        let mut reached = 1;
        while let Some(position) = self.frontier.pop_front() {
            let Some(neighbors) = self.neighbors.get(position) else {
                continue;
            };
            for neighbor in neighbors {
                if self.blocked.get(*neighbor).copied().unwrap_or(true)
                    || self.visited_epoch.get(*neighbor).copied() == Some(self.epoch)
                {
                    continue;
                }
                let Some(visited) = self.visited_epoch.get_mut(*neighbor) else {
                    continue;
                };
                *visited = self.epoch;
                reached += 1;
                self.frontier.push_back(*neighbor);
            }
        }
        reached
    }
}

#[cfg(test)]
fn complete_ordinary_network_is_connected(
    ordinary: &OrdinaryGraph,
    blockers: &BTreeSet<TilePos>,
) -> bool {
    let Some(start) = ordinary
        .positions()
        .find(|surface| !blockers.contains(surface))
    else {
        return false;
    };
    let reached = ordinary.reachable_avoiding(start, blockers);
    ordinary
        .positions()
        .all(|surface| blockers.contains(&surface) || reached.contains(&surface))
}

fn vegetation_below_climate_ceiling(
    recipe: &V3RecipeSettings,
    kind: FeatureKind,
    level: Level,
    alpine_climate: MacroAlpineClimate,
) -> bool {
    // The alpine treeline is a climate rule for foothill vegetation, not a
    // global world-height limit. An authored temperate Forest basin remains a
    // forest even when a landmark places it above an enclosing mountain tier.
    if kind == FeatureKind::Tree
        && matches!(
            recipe,
            V3RecipeSettings::Forest(_) | V3RecipeSettings::DeepForest(_)
        )
    {
        return true;
    }
    let ceiling = if kind == FeatureKind::Tree {
        alpine_climate.treeline
    } else {
        MACRO_GRASS_CEILING
    };
    level < ceiling
}

#[derive(Debug, Clone, Copy)]
struct MacroVegetationLayer<'a> {
    percent: u8,
    kind: FeatureKind,
    object: &'a VegetationObjectSpec,
    seed_stage: &'static str,
    clustered: bool,
}

impl<'a> MacroVegetationLayer<'a> {
    const fn trees(
        percent: u8,
        object: &'a VegetationObjectSpec,
        seed_stage: &'static str,
        clustered: bool,
    ) -> Self {
        Self {
            percent,
            kind: FeatureKind::Tree,
            object,
            seed_stage,
            clustered,
        }
    }

    const fn grass(
        percent: u8,
        object: &'a VegetationObjectSpec,
        seed_stage: &'static str,
    ) -> Self {
        Self {
            percent,
            kind: FeatureKind::TallGrass,
            object,
            seed_stage,
            clustered: false,
        }
    }
}

fn vegetation_rank(
    stream: Option<super::seed::SeedStream<'_>>,
    coord: HexCoord,
    clustered: bool,
) -> u128 {
    if !clustered {
        return u128::from(vegetation_sample(stream, coord, 0));
    }
    // Adjacent columns share five samples in this seven-column kernel. Sorting by
    // the sum therefore produces irregular groves rather than evenly spaced dots,
    // without introducing a cell- or instance-shaped boundary.
    std::iter::once(coord)
        .chain(coord.neighbors())
        .map(|sample_coord| u128::from(vegetation_sample(stream, sample_coord, 0)))
        .sum()
}

fn vegetation_sample(
    stream: Option<super::seed::SeedStream<'_>>,
    coord: HexCoord,
    salt: u64,
) -> u64 {
    stream.map_or_else(
        || canonical_coord_hash(coord).wrapping_add(salt.wrapping_mul(0x9e37_79b9_7f4a_7c15)),
        |stream| stream.sample_coord(coord, salt),
    )
}

fn canonical_coord_hash(coord: HexCoord) -> u64 {
    let [x, y, z] = coord.to_cubic_array();
    let mut value = u64::from(x.unsigned_abs());
    value = value
        .wrapping_mul(1_099_511_628_211)
        .wrapping_add(u64::from(y.unsigned_abs()));
    value
        .wrapping_mul(1_099_511_628_211)
        .wrapping_add(u64::from(z.unsigned_abs()))
}

fn canonical_anchor_settings(
    settings: &MacroLayoutSettings,
    contracts: &ResolvedMacroContracts,
) -> Result<BTreeMap<String, PatchAnchorRef>, V3GenerationError> {
    let ids = settings
        .instances
        .iter()
        .enumerate()
        .map(|(index, instance)| {
            (
                instance.name.as_str(),
                PatchId(u32::try_from(index).unwrap_or(u32::MAX)),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut canonical = contracts
        .anchor_aliases
        .iter()
        .map(|(alias, reference)| {
            (
                alias.clone(),
                PatchAnchorRef {
                    patch: reference.instance,
                    local_name: reference.anchor.clone(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut targets = Vec::new();
    if let (Some(route_start), Some(route_end)) = (
        settings.critical_route.first(),
        settings.critical_route.last(),
    ) {
        let hostile_index = settings.critical_route.len().saturating_sub(1).min(2);
        let hostile = settings
            .critical_route
            .get(hostile_index)
            .unwrap_or(route_end);
        targets.extend([
            (PARTY_START, route_start.as_str(), PARTY_START),
            (HOSTILE_START, hostile.as_str(), HOSTILE_START),
            (MACRO_ROUTE_END, route_end.as_str(), MACRO_ROUTE_END),
        ]);
    }
    targets.extend(
        [
            (BEACH_REVIEW, "beach-lower", BEACH_REVIEW),
            (COAST_REVIEW, "shore-center", COAST_REVIEW),
            (INLAND_REVIEW, "prairie-route", INLAND_REVIEW),
            (FOOTHILL_REVIEW, "hills-center", FOOTHILL_REVIEW),
            (
                MASSIF_FRONT_REVIEW,
                "mountains-tier2-center",
                MASSIF_FRONT_REVIEW,
            ),
            (DEEP_MOUNTAIN_BASE, "deep-mountain", DEEP_MOUNTAIN_BASE),
            (DEEP_MOUNTAIN_REVIEW, "deep-mountain", DEEP_MOUNTAIN_REVIEW),
        ]
        .into_iter()
        .filter(|(_, instance, _)| ids.contains_key(instance)),
    );
    targets.extend(settings.headwaters.iter().map(|headwater| match headwater {
        MacroHeadwaterSettings::CaveFall { instance, .. } => {
            (CAVE_SOURCE_REVIEW, instance.as_str(), CAVE_SOURCE_REVIEW)
        }
        MacroHeadwaterSettings::RivuletConfluence { instance, .. } => (
            RIVULET_SOURCE_REVIEW,
            instance.as_str(),
            RIVULET_SOURCE_REVIEW,
        ),
    }));
    for (alias, instance, local) in targets {
        let patch = ids.get(instance).copied().ok_or_else(|| {
            V3GenerationError::RecipeContract(format!(
                "Macro canonical anchor references missing instance {instance:?}"
            ))
        })?;
        if canonical
            .insert(
                alias.to_owned(),
                PatchAnchorRef {
                    patch,
                    local_name: local.to_owned(),
                },
            )
            .is_some()
        {
            return Err(V3GenerationError::RecipeContract(format!(
                "Macro canonical anchor alias {alias:?} is declared more than once"
            )));
        }
    }
    Ok(canonical)
}

fn macro_view_hint(
    layout: &super::layout::ResolvedLayoutPlan,
    maximum_level: Level,
    level_height: f32,
) -> MapViewHint {
    let bounds = layout
        .footprint
        .iter()
        .map(|coord| coord.to_world(0.0))
        .fold(None::<(f32, f32, f32, f32)>, |bounds, point| match bounds {
            None => Some((point.x, point.x, point.z, point.z)),
            Some((min_x, max_x, min_z, max_z)) => Some((
                min_x.min(point.x),
                max_x.max(point.x),
                min_z.min(point.z),
                max_z.max(point.z),
            )),
        });
    let (min_x, max_x, min_z, max_z) = bounds.unwrap_or((0.0, 0.0, 0.0, 0.0));
    let vertical_span = i16::try_from(maximum_level)
        .map(f32::from)
        .unwrap_or_default()
        * level_height;
    let horizontal_span = (max_x - min_x).hypot(max_z - min_z);
    let frame = (horizontal_span * 0.78).max(vertical_span * 2.2).max(40.0);
    let focus = (
        (min_x + max_x) * 0.5,
        vertical_span * 0.36,
        (min_z + max_z) * 0.5,
    );
    MapViewHint::new(
        (focus.0, focus.1 + frame * 0.72, focus.2 + frame * 0.82),
        focus,
    )
}

fn is_mountain_range_layout(settings: &MacroLayoutSettings) -> bool {
    let recipe = |name: &str| {
        settings
            .instances
            .iter()
            .find(|instance| instance.name == name)
            .map(|instance| &instance.recipe)
    };
    settings.instances.len() == 30
        && settings.critical_route.iter().map(String::as_str).eq([
            "shore-center",
            "prairie-route",
            "hills-center",
            "mountains-tier1-center",
            "mountains-tier2-center",
            "deep-mountain",
        ])
        && matches!(recipe("shallow-sea"), Some(V3RecipeSettings::ShallowSea(_)))
        && matches!(recipe("shore-center"), Some(V3RecipeSettings::Shore(_)))
        && matches!(recipe("prairie-route"), Some(V3RecipeSettings::Prairie(_)))
        && matches!(recipe("hills-center"), Some(V3RecipeSettings::Hills(_)))
        && matches!(
            recipe("mountains-tier1-center"),
            Some(V3RecipeSettings::Mountains(_))
        )
        && matches!(
            recipe("mountains-tier2-center"),
            Some(V3RecipeSettings::Mountains(_))
        )
        && matches!(
            recipe("deep-mountain"),
            Some(V3RecipeSettings::DeepMountain(_))
        )
}

fn is_crystal_mountain_layout(settings: &MacroLayoutSettings) -> bool {
    let recipe = |name: &str| {
        settings
            .instances
            .iter()
            .find(|instance| instance.name == name)
            .map(|instance| &instance.recipe)
    };
    let canonical_tunnels = settings
        .spanning_features
        .iter()
        .filter(|feature| match feature {
            MacroSpanningFeatureSettings::Tunnel(tunnel) => tunnel.canonical_route,
        })
        .count();
    settings.instances.len() == 4
        && settings.critical_route.is_empty()
        && canonical_tunnels == 1
        && matches!(
            recipe("crystal-ascent"),
            Some(V3RecipeSettings::CrystalAscent(_))
        )
        && matches!(recipe("summit-forest"), Some(V3RecipeSettings::Forest(_)))
        && matches!(
            recipe("inner-mountain"),
            Some(V3RecipeSettings::Mountains(_))
        )
        && matches!(
            recipe("outer-mountain"),
            Some(V3RecipeSettings::Mountains(_))
        )
}

fn is_ocean_archipelago_layout(settings: &MacroLayoutSettings) -> bool {
    let recipe = |name: &str| {
        settings
            .instances
            .iter()
            .find(|instance| instance.name == name)
            .map(|instance| &instance.recipe)
    };
    settings.instances.len() == 6
        && settings
            .critical_route
            .iter()
            .map(String::as_str)
            .eq(["home-landing", "wooded-heart"])
        && settings.liquid_connections.len() == 10
        && settings.walker_connections.len() == 1
        && settings.headwaters.is_empty()
        && settings.spanning_features.is_empty()
        && matches!(recipe("open-sea"), Some(V3RecipeSettings::ShallowSea(_)))
        && [
            "east-islets",
            "south-islets",
            "northwest-islets",
            "home-landing",
        ]
        .into_iter()
        .all(|name| matches!(recipe(name), Some(V3RecipeSettings::SandyIslets(_))))
        && matches!(
            recipe("wooded-heart"),
            Some(V3RecipeSettings::WoodedIsland(_))
        )
}

fn finalize_crystal_mountain_landscape(
    settings: &MacroLayoutSettings,
    contracts: &ResolvedMacroContracts,
    world: &mut GeneratedWorldPlan,
) -> Result<(), V3GenerationError> {
    if !is_crystal_mountain_layout(settings) {
        return Ok(());
    }
    let patch_id = |name: &str| {
        settings
            .instances
            .iter()
            .position(|instance| instance.name == name)
            .and_then(|index| u32::try_from(index).ok())
            .map(PatchId)
            .ok_or_else(|| {
                V3GenerationError::RecipeContract(format!(
                    "Crystal Mountain is missing logical instance {name:?}"
                ))
            })
    };
    let crystal = patch_id("crystal-ascent")?;
    let forest = patch_id("summit-forest")?;
    let outer = patch_id("outer-mountain")?;
    let connection = contracts
        .walker_connections
        .iter()
        .find(|connection| {
            BTreeSet::from([connection.first, connection.second])
                == BTreeSet::from([crystal, forest])
        })
        .ok_or_else(|| {
            V3GenerationError::RecipeContract(
                "Crystal Mountain has no explicit Crystal-to-Forest walker connection".to_owned(),
            )
        })?;
    if connection.level != 150 || connection.port.lanes.len() != 4 {
        return Err(V3GenerationError::RecipeContract(
            "Crystal Mountain summit walker connection must remain exactly four-wide at level 150"
                .to_owned(),
        ));
    }
    let (forest_approach, forest_seam) = if connection.first == forest {
        (
            &connection.port.first_approach,
            connection
                .port
                .lanes
                .iter()
                .map(|(first, _)| *first)
                .collect::<BTreeSet<_>>(),
        )
    } else {
        (
            &connection.port.second_approach,
            connection
                .port
                .lanes
                .iter()
                .map(|(_, second)| *second)
                .collect::<BTreeSet<_>>(),
        )
    };
    let clearing_surfaces = forest_approach
        .iter()
        .copied()
        .map(|coord| TilePos::new(coord, connection.level))
        .collect::<BTreeSet<_>>();
    if clearing_surfaces.len() < 4
        || clearing_surfaces.iter().any(|surface| {
            world
                .volume
                .surfaces
                .get(surface)
                .is_none_or(|metadata| metadata.access != SurfaceAccess::Ordinary)
                || world.blockers.contains(surface)
                || world
                    .features
                    .by_id
                    .values()
                    .any(|feature| feature.root == *surface)
        })
    {
        return Err(V3GenerationError::RecipeContract(
            "Crystal Mountain basin approach is not a feature-free ordinary level-150 clearing"
                .to_owned(),
        ));
    }
    let basin = clearing_surfaces
        .iter()
        .copied()
        .min_by_key(|surface| {
            let seam_distance = forest_seam
                .iter()
                .map(|coord| surface.coord.distance(*coord))
                .min()
                .unwrap_or_default();
            (Reverse(seam_distance), *surface)
        })
        .ok_or_else(|| {
            V3GenerationError::RecipeContract(
                "Crystal Mountain basin clearing has no stable review footing".to_owned(),
            )
        })?;
    match world
        .features
        .clearings
        .get("crystal_mountain.basin_clearing")
    {
        Some(existing) if existing.surfaces == clearing_surfaces => {}
        Some(_) => {
            return Err(V3GenerationError::RecipeContract(
                "Crystal Mountain basin clearing collides with another feature membership"
                    .to_owned(),
            ));
        }
        None => {
            world.features.clearings.insert(
                "crystal_mountain.basin_clearing".to_owned(),
                FeatureClearing {
                    surfaces: clearing_surfaces,
                },
            );
        }
    }

    let outer_patch = world.layout.patches.get(&outer).ok_or_else(|| {
        V3GenerationError::RecipeContract(
            "Crystal Mountain outer ridge has no resolved patch".to_owned(),
        )
    })?;
    let ridge = world
        .volume
        .surfaces
        .iter()
        .filter(|(surface, metadata)| {
            outer_patch.mask.contains(&surface.coord)
                && metadata.access == SurfaceAccess::Ordinary
                && !world.blockers.contains(surface)
        })
        .map(|(surface, _)| *surface)
        .min_by_key(|surface| (Reverse(surface.level), *surface))
        .ok_or_else(|| {
            V3GenerationError::RecipeContract(
                "Crystal Mountain outer ridge has no ordinary review footing".to_owned(),
            )
        })?;

    let foot = world
        .anchors
        .get("crystal_mountain.foot_apron")
        .copied()
        .ok_or_else(|| {
            V3GenerationError::RecipeContract(
                "Crystal Mountain tunnel did not publish its foot apron".to_owned(),
            )
        })?;
    for (name, position) in [
        ("crystal_mountain.basin_clearing", basin),
        ("crystal_mountain.ridge", ridge),
        (PARTY_START, foot),
        (HOSTILE_START, basin),
        (MACRO_ROUTE_END, basin),
    ] {
        match world.anchors.get(name) {
            Some(existing) if *existing == position => {}
            Some(existing) => {
                return Err(V3GenerationError::RecipeContract(format!(
                    "Crystal Mountain anchor {name:?} conflicts at {existing:?} instead of {position:?}"
                )));
            }
            None => {
                world.anchors.insert(name.to_owned(), position);
            }
        }
    }
    Ok(())
}

fn validate_macro_world(
    settings: &ProceduralV3Settings,
    plan: &GeneratedWorldPlan,
    prepared_contracts: Option<&ResolvedMacroContracts>,
) -> WorldValidation<MacroWorldMetrics> {
    let V3LayoutSettings::Macro(macro_settings) = &settings.layout else {
        return WorldValidation::Invalid(vec![macro_issue(
            "Macro validation received a non-Macro layout",
        )]);
    };
    let mountain_range_layout = is_mountain_range_layout(macro_settings);
    let crystal_mountain_layout = is_crystal_mountain_layout(macro_settings);
    let ocean_archipelago_layout = is_ocean_archipelago_layout(macro_settings);
    // `V3Recipe::validate` is reached only after the shared selection runner has
    // admitted the complete recipe-independent world contract. Keep this pass
    // Macro-specific so radius-77 candidates do not repeat that full validation.
    let mut issues = Vec::new();
    if plan.layout.grid_radius != 77 || plan.layout.footprint.len() != 18_019 {
        issues.push(macro_issue(format!(
            "V3 Macro requires radius 77 and 18,019 columns, got radius {} and {} columns",
            plan.layout.grid_radius,
            plan.layout.footprint.len()
        )));
    }
    if plan.layout.patches.len() != macro_settings.instances.len() {
        issues.push(macro_issue(
            "Macro logical biome count changed after resolution",
        ));
    }
    let highest_surface_by_coord = plan.volume.surfaces.keys().copied().fold(
        BTreeMap::<HexCoord, Level>::new(),
        |mut levels, surface| {
            levels
                .entry(surface.coord)
                .and_modify(|level| *level = (*level).max(surface.level))
                .or_insert(surface.level);
            levels
        },
    );
    for edge in plan.layout.shared_edges.values() {
        let first = macro_settings
            .instances
            .get(usize::try_from(edge.first.0 .0).unwrap_or(usize::MAX));
        let second = macro_settings
            .instances
            .get(usize::try_from(edge.second.0 .0).unwrap_or(usize::MAX));
        let alpine_pair = [first, second].into_iter().all(|instance| {
            instance.is_some_and(|instance| {
                matches!(
                    &instance.recipe,
                    V3RecipeSettings::Mountains(_) | V3RecipeSettings::DeepMountain(_)
                )
            })
        });
        if !alpine_pair {
            continue;
        }
        for (first_coord, second_coord) in &edge.boundary_pairs {
            let (Some(first_level), Some(second_level)) = (
                highest_surface_by_coord.get(first_coord),
                highest_surface_by_coord.get(second_coord),
            ) else {
                continue;
            };
            if first_level.abs_diff(*second_level) > 1 {
                issues.push(macro_issue(format!(
                    "alpine seam {:?}<->{:?} jumps from level {first_level} at {first_coord:?} to level {second_level} at {second_coord:?}",
                    first.map(|instance| instance.name.as_str()),
                    second.map(|instance| instance.name.as_str()),
                )));
                break;
            }
        }
    }
    let party = plan.anchors.get(PARTY_START).copied();
    let route_end = plan.anchors.get(MACRO_ROUTE_END).copied();
    let ordinary = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
    let distances = party
        .map(|party| ordinary.distances_from(party))
        .unwrap_or_default();
    let critical_route_steps = route_end
        .and_then(|base| distances.get(&base).copied())
        .unwrap_or_default();
    if party.is_none() || route_end.is_none() || critical_route_steps == 0 {
        let reached_instances = macro_settings
            .critical_route
            .iter()
            .filter_map(|name| {
                let index = macro_settings
                    .instances
                    .iter()
                    .position(|instance| instance.name == *name)?;
                let patch = plan
                    .layout
                    .patches
                    .get(&PatchId(u32::try_from(index).ok()?))?;
                plan.volume
                    .surfaces
                    .keys()
                    .any(|surface| {
                        patch.mask.contains(&surface.coord) && distances.contains_key(surface)
                    })
                    .then_some(name.as_str())
            })
            .collect::<Vec<_>>();
        let route_ids = macro_settings
            .instances
            .iter()
            .enumerate()
            .map(|(index, instance)| {
                (
                    instance.name.as_str(),
                    PatchId(u32::try_from(index).unwrap_or(u32::MAX)),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let seam_reachability = macro_settings
            .critical_route
            .windows(2)
            .filter_map(|pair| {
                let first_name = pair.first()?;
                let second_name = pair.get(1)?;
                let first_id = route_ids.get(first_name.as_str())?;
                let second_id = route_ids.get(second_name.as_str())?;
                let edge = plan.layout.shared_edges.values().find(|edge| {
                    BTreeSet::from([edge.first.0, edge.second.0])
                        == BTreeSet::from([*first_id, *second_id])
                })?;
                let first_is_edge_first = edge.first.0 == *first_id;
                let first_reached = edge.walker.ports.iter().any(|port| {
                    let approaches = if first_is_edge_first {
                        &port.first_approach
                    } else {
                        &port.second_approach
                    };
                    approaches.iter().any(|coord| {
                        distances.contains_key(&TilePos::new(*coord, edge.elevation.preferred))
                    })
                });
                let second_reached = edge.walker.ports.iter().any(|port| {
                    let approaches = if first_is_edge_first {
                        &port.second_approach
                    } else {
                        &port.first_approach
                    };
                    approaches.iter().any(|coord| {
                        distances.contains_key(&TilePos::new(*coord, edge.elevation.preferred))
                    })
                });
                Some((
                    format!("{first_name}->{second_name}@{}", edge.elevation.preferred),
                    first_reached,
                    second_reached,
                ))
            })
            .collect::<Vec<_>>();
        issues.push(macro_issue(format!(
            "Macro critical route does not connect first anchor {party:?} to last anchor {route_end:?}; reached critical instances {reached_instances:?}; seam reachability {seam_reachability:?}"
        )));
    }

    validate_sea_strata(macro_settings, plan, &mut issues);
    validate_coastal_coverage(macro_settings, plan, &mut issues);
    validate_coastal_vegetation(macro_settings, plan, &mut issues);
    validate_waterfall_flow(macro_settings, plan, &mut issues);
    if crystal_mountain_layout {
        validate_crystal_mountain(
            macro_settings,
            plan,
            prepared_contracts,
            &distances,
            &mut issues,
        );
    }
    if mountain_range_layout {
        validate_mountain_watershed(macro_settings, plan, &mut issues);
        issues.extend(
            super::river_terrain::validate_river_terrain(&plan.volume, &plan.liquids)
                .into_iter()
                .map(|issue| macro_issue(format!("river terrain: {issue}"))),
        );
    }
    let ocean_archipelago = ocean_archipelago_layout.then(|| {
        validate_ocean_archipelago(macro_settings, plan, &ordinary, &distances, &mut issues)
    });
    let deep_patch = macro_settings
        .instances
        .iter()
        .position(|instance| matches!(instance.recipe, V3RecipeSettings::DeepMountain(_)))
        .and_then(|index| {
            plan.layout
                .patches
                .get(&PatchId(u32::try_from(index).ok()?))
        });
    let deep_surfaces = deep_patch
        .into_iter()
        .flat_map(|patch| {
            plan.volume
                .surfaces
                .keys()
                .filter(move |surface| patch.mask.contains(&surface.coord))
                .copied()
        })
        .collect::<Vec<_>>();
    if mountain_range_layout {
        if let Some(deep_patch) = deep_patch {
            let perimeter_levels = boundary_depths(&deep_patch.mask)
                .into_iter()
                .filter_map(|(coord, depth)| {
                    (depth == 0)
                        .then(|| highest_surface_by_coord.get(&coord).copied())
                        .flatten()
                })
                .collect::<BTreeSet<_>>();
            if perimeter_levels.first().copied() != Some(41)
                || perimeter_levels.last().copied() != Some(48)
                || perimeter_levels
                    .iter()
                    .any(|level| !(41..=48).contains(level))
            {
                issues.push(macro_issue(format!(
                    "Deep Mountain perimeter must taper from level-41 lower buttresses to its level-48 upper front, got {perimeter_levels:?}"
                )));
            }
        }
    }
    let summit_level = deep_surfaces
        .iter()
        .map(|surface| surface.level)
        .max()
        .unwrap_or_default();
    let summit_count = deep_surfaces
        .iter()
        .filter(|surface| surface.level == summit_level)
        .count();
    let high_massif_surfaces = deep_surfaces
        .iter()
        .filter(|surface| surface.level >= 60)
        .count();
    if mountain_range_layout
        && (!(92..=104).contains(&summit_level) || summit_count > 7 || high_massif_surfaces < 100)
    {
        issues.push(macro_issue(format!(
            "Deep Mountain needs one dominant 92..=104 summit and broad shoulders; got level {summit_level}, {summit_count} summit surfaces, and {high_massif_surfaces} high surfaces"
        )));
    }

    let standing_water_seams = plan
        .layout
        .shared_edges
        .values()
        .filter(|edge| matches!(edge.liquid, ResolvedLiquidPort::Standing { .. }))
        .count();
    let directed_liquid_seams = plan
        .layout
        .shared_edges
        .values()
        .filter(|edge| matches!(edge.liquid, ResolvedLiquidPort::Directed { .. }))
        .count();
    let liquid_coords = plan
        .liquids
        .bodies
        .values()
        .flat_map(|body| body.nodes.keys().map(|position| position.coord))
        .collect::<BTreeSet<_>>();
    let reachable_levels = distances
        .keys()
        .map(|position| position.level)
        .collect::<BTreeSet<_>>()
        .len();
    let minimum_surface = plan
        .volume
        .surfaces
        .keys()
        .map(|surface| surface.level)
        .min()
        .unwrap_or_default();
    let maximum_surface = plan
        .volume
        .surfaces
        .keys()
        .map(|surface| surface.level)
        .max()
        .unwrap_or_default();

    if !issues.is_empty() {
        return WorldValidation::Invalid(issues);
    }
    let report = MacroMetrics {
        world_columns: count_u32(plan.layout.footprint.len()),
        macro_cells: 37,
        biome_regions: count_u32(plan.layout.patches.len()),
        reciprocal_seams: count_u32(plan.layout.shared_edges.len()),
        outer_macro_sides: 42,
        ordinary_surfaces: count_u32(ordinary.len()),
        reachable_surfaces: count_u32(distances.len()),
        reachable_elevation_levels: count_u32(reachable_levels),
        relief: maximum_surface.saturating_sub(minimum_surface),
        critical_route_steps,
        standing_water_seams: count_u32(standing_water_seams),
        directed_liquid_seams: count_u32(directed_liquid_seams),
        liquid_cells: count_u32(liquid_coords.len()),
    };
    let mountain_range = mountain_range_layout.then_some(MountainRangeMetrics {
        world_columns: report.world_columns,
        macro_cells: report.macro_cells,
        biome_regions: report.biome_regions,
        reciprocal_seams: report.reciprocal_seams,
        outer_macro_sides: report.outer_macro_sides,
        ordinary_surfaces: report.ordinary_surfaces,
        reachable_surfaces: report.reachable_surfaces,
        reachable_elevation_levels: report.reachable_elevation_levels,
        relief: report.relief,
        critical_route_steps: report.critical_route_steps,
        standing_water_seams: report.standing_water_seams,
        directed_liquid_seams: report.directed_liquid_seams,
        liquid_cells: report.liquid_cells,
        summit_level,
        high_massif_surfaces: count_u32(high_massif_surfaces),
    });
    WorldValidation::Valid(MacroWorldMetrics {
        report,
        mountain_range,
        ocean_archipelago,
    })
}

fn validate_ocean_archipelago(
    settings: &MacroLayoutSettings,
    plan: &GeneratedWorldPlan,
    ordinary: &OrdinaryGraph,
    distances: &BTreeMap<TilePos, u32>,
    issues: &mut Vec<WorldValidationIssue>,
) -> OceanArchipelagoMetrics {
    const INSTANCE_CELLS: [(&str, &[(i32, i32, i32)]); 6] = [
        (
            "open-sea",
            &[
                (3, 0, -3),
                (2, 1, -3),
                (1, 2, -3),
                (0, 3, -3),
                (-1, 3, -2),
                (-2, 3, -1),
                (-3, 3, 0),
                (-3, 2, 1),
                (-3, 1, 2),
                (-3, 0, 3),
                (-2, -1, 3),
                (-1, -2, 3),
                (0, -3, 3),
                (1, -3, 2),
                (2, -3, 1),
                (3, -3, 0),
                (3, -2, -1),
                (3, -1, -2),
                (2, -2, 0),
                (1, -2, 1),
                (-2, 0, 2),
                (-2, 1, 1),
                (0, 2, -2),
                (1, 1, -2),
            ],
        ),
        ("east-islets", &[(2, 0, -2), (2, -1, -1)]),
        ("south-islets", &[(0, -2, 2), (-1, -1, 2)]),
        ("northwest-islets", &[(-2, 2, 0), (-1, 2, -1)]),
        ("home-landing", &[(-1, 0, 1)]),
        (
            "wooded-heart",
            &[
                (0, 0, 0),
                (1, -1, 0),
                (1, 0, -1),
                (0, 1, -1),
                (-1, 1, 0),
                (0, -1, 1),
            ],
        ),
    ];

    for (name, expected) in INSTANCE_CELLS {
        let actual = settings
            .instances
            .iter()
            .find(|instance| instance.name == name)
            .map(|instance| {
                instance
                    .cells
                    .iter()
                    .map(|cell| (cell.x, cell.y, cell.z))
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        let expected = expected.iter().copied().collect::<BTreeSet<_>>();
        if actual != expected {
            issues.push(macro_issue(format!(
                "Ocean Archipelagoes instance {name:?} changed its exact atomic-cell roster"
            )));
        }
    }

    let mut remaining = ordinary.positions().collect::<BTreeSet<_>>();
    let mut dry_components = Vec::<BTreeSet<TilePos>>::new();
    while let Some(start) = remaining.first().copied() {
        let mut component = BTreeSet::from([start]);
        let mut frontier = VecDeque::from([start]);
        remaining.remove(&start);
        while let Some(position) = frontier.pop_front() {
            for neighbor in ordinary.neighbors(position) {
                if remaining.remove(neighbor) {
                    component.insert(*neighbor);
                    frontier.push_back(*neighbor);
                }
            }
        }
        dry_components.push(component);
    }
    let scenic_components = dry_components
        .iter()
        .filter(|component| {
            component
                .iter()
                .all(|surface| !distances.contains_key(surface))
        })
        .count();
    if dry_components.len() != 7 || scenic_components != 6 {
        issues.push(macro_issue(format!(
            "Ocean Archipelagoes requires seven dry components with six scenic satellites, got {} and {scenic_components}",
            dry_components.len()
        )));
    }

    if plan.liquids.bodies.len() != 1
        || plan.liquids.bodies.values().any(|body| {
            body.material != FillMaterialRole::Water
                || body
                    .nodes
                    .values()
                    .any(|node| node.state != LiquidFlowState::Still)
        })
    {
        issues.push(macro_issue(
            "Ocean Archipelagoes requires exactly one connected still-water body",
        ));
    }
    let liquid_coords = plan
        .liquids
        .bodies
        .values()
        .flat_map(|body| body.nodes.keys().map(|position| position.coord))
        .collect::<BTreeSet<_>>();
    let dry_coords = ordinary
        .positions()
        .map(|surface| surface.coord)
        .collect::<BTreeSet<_>>();
    let boundary_is_water = plan
        .layout
        .footprint
        .iter()
        .filter(|coord| coord.distance(HexCoord::ORIGIN) == plan.layout.grid_radius)
        .all(|coord| liquid_coords.contains(coord) && !dry_coords.contains(coord));
    if !boundary_is_water {
        issues.push(macro_issue(
            "Ocean Archipelagoes requires uninterrupted ocean on every world-boundary column",
        ));
    }

    let standing_water_seams = plan
        .layout
        .shared_edges
        .values()
        .filter(|edge| matches!(edge.liquid, ResolvedLiquidPort::Standing { .. }))
        .count();
    if standing_water_seams != 10 {
        issues.push(macro_issue(format!(
            "Ocean Archipelagoes requires exactly ten Standing-water seams, got {standing_water_seams}"
        )));
    }
    let ids = settings
        .instances
        .iter()
        .enumerate()
        .map(|(index, instance)| {
            (
                instance.name.as_str(),
                PatchId(u32::try_from(index).unwrap_or(u32::MAX)),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let normalized_pair = |first: PatchId, second: PatchId| {
        if first <= second {
            (first, second)
        } else {
            (second, first)
        }
    };
    let named_pair = |first: &str, second: &str| {
        ids.get(first)
            .copied()
            .zip(ids.get(second).copied())
            .map(|(first, second)| normalized_pair(first, second))
    };
    let expected_standing_pairs = [
        ("open-sea", "east-islets"),
        ("open-sea", "south-islets"),
        ("open-sea", "northwest-islets"),
        ("open-sea", "home-landing"),
        ("open-sea", "wooded-heart"),
        ("east-islets", "wooded-heart"),
        ("south-islets", "home-landing"),
        ("south-islets", "wooded-heart"),
        ("northwest-islets", "wooded-heart"),
        ("home-landing", "wooded-heart"),
    ]
    .into_iter()
    .filter_map(|(first, second)| named_pair(first, second))
    .collect::<BTreeSet<_>>();
    let standing_edges = plan
        .layout
        .shared_edges
        .values()
        .filter(|edge| matches!(edge.liquid, ResolvedLiquidPort::Standing { .. }))
        .map(|edge| (normalized_pair(edge.first.0, edge.second.0), edge))
        .collect::<BTreeMap<_, _>>();
    if standing_edges.keys().copied().collect::<BTreeSet<_>>() != expected_standing_pairs {
        issues.push(macro_issue(
            "Ocean Archipelagoes Standing water changed its exact ten instance pairs",
        ));
    }
    let causeway_pair = named_pair("home-landing", "wooded-heart");
    for (pair, edge) in &standing_edges {
        let ResolvedLiquidPort::Standing { port, elevation } = &edge.liquid else {
            continue;
        };
        if *elevation != ResolvedLiquidElevation::Exact(8) {
            issues.push(macro_issue(format!(
                "Ocean Archipelagoes Standing seam {pair:?} must remain exactly level 8"
            )));
        }
        if Some(*pair) == causeway_pair {
            let first_approach = edge
                .walker
                .ports
                .iter()
                .flat_map(|walker| walker.first_approach.iter().copied())
                .collect::<BTreeSet<_>>();
            let second_approach = edge
                .walker
                .ports
                .iter()
                .flat_map(|walker| walker.second_approach.iter().copied())
                .collect::<BTreeSet<_>>();
            let exclusions = edge
                .boundary_pairs
                .iter()
                .filter(|(first, second)| {
                    first_approach.contains(first) || second_approach.contains(second)
                })
                .copied()
                .collect::<BTreeSet<_>>();
            if edge.elevation.preferred != 9
                || edge.walker.ports.len() != 1
                || edge
                    .walker
                    .ports
                    .first()
                    .is_none_or(|walker| walker.lanes.len() != 4)
                || port
                    .lanes
                    .union(&exclusions)
                    .copied()
                    .collect::<BTreeSet<_>>()
                    != edge.boundary_pairs
                || !port.lanes.is_disjoint(&exclusions)
            {
                issues.push(macro_issue(
                    "Ocean Archipelagoes home causeway must remain four-wide at level 9 with its exact standing-water exclusion",
                ));
            }
        } else if !edge.walker.ports.is_empty() || port.lanes != edge.boundary_pairs {
            issues.push(macro_issue(format!(
                "Ocean Archipelagoes non-causeway Standing seam {pair:?} must remain full-width"
            )));
        }
    }
    let exact_coastal_walker = plan
        .layout
        .shared_edges
        .values()
        .filter(|edge| {
            matches!(edge.liquid, ResolvedLiquidPort::Standing { .. })
                && edge.walker.ports.len() == 1
                && edge
                    .walker
                    .ports
                    .first()
                    .is_some_and(|port| port.lanes.len() == 4)
        })
        .count();
    if exact_coastal_walker != 1 {
        issues.push(macro_issue(format!(
            "Ocean Archipelagoes requires one exact four-lane causeway excluded from standing water, got {exact_coastal_walker}"
        )));
    }

    let expected_aliases = [
        (
            "archipelago.home_beach",
            "home-landing",
            "sandy_islets_primary_overlook",
        ),
        (
            "archipelago.channel_overlook",
            "home-landing",
            "sandy_islets_channel_overlook",
        ),
        (
            "archipelago.home_ridge",
            "wooded-heart",
            "wooded_island_ridge",
        ),
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    let actual_aliases = settings
        .anchor_aliases
        .iter()
        .map(|alias| {
            (
                alias.alias.as_str(),
                alias.instance.as_str(),
                alias.anchor.as_str(),
            )
        })
        .collect::<BTreeSet<_>>();
    if actual_aliases != expected_aliases {
        issues.push(macro_issue(
            "Ocean Archipelagoes changed its exact world-level anchor alias sources",
        ));
    }

    for anchor in [
        PARTY_START,
        MACRO_ROUTE_END,
        "archipelago.home_beach",
        "archipelago.channel_overlook",
        "archipelago.home_ridge",
    ] {
        if !plan.anchors.contains_key(anchor) {
            issues.push(macro_issue(format!(
                "Ocean Archipelagoes is missing stable anchor {anchor:?}"
            )));
        }
    }

    let shoreline_surfaces = ordinary
        .positions()
        .filter(|surface| {
            surface
                .coord
                .neighbors()
                .into_iter()
                .any(|neighbor| liquid_coords.contains(&neighbor))
        })
        .count();
    let tree_roots = plan
        .features
        .by_id
        .values()
        .filter(|feature| feature.kind == FeatureKind::Tree)
        .count();
    if tree_roots == 0 {
        issues.push(macro_issue(
            "Ocean Archipelagoes wooded heart requires rooted broadleaf trees",
        ));
    }

    OceanArchipelagoMetrics {
        world_columns: count_u32(plan.layout.footprint.len()),
        macro_cells: 37,
        biome_regions: count_u32(plan.layout.patches.len()),
        standing_water_seams: count_u32(standing_water_seams),
        liquid_cells: count_u32(liquid_coords.len()),
        dry_components: u8::try_from(dry_components.len()).unwrap_or(u8::MAX),
        scenic_dry_components: u8::try_from(scenic_components).unwrap_or(u8::MAX),
        ordinary_surfaces: count_u32(ordinary.len()),
        reachable_surfaces: count_u32(distances.len()),
        critical_route_steps: plan
            .anchors
            .get(MACRO_ROUTE_END)
            .and_then(|end| distances.get(end))
            .copied()
            .unwrap_or_default(),
        shoreline_surfaces: count_u32(shoreline_surfaces),
        tree_roots: count_u32(tree_roots),
    }
}

fn validate_crystal_mountain(
    settings: &MacroLayoutSettings,
    plan: &GeneratedWorldPlan,
    prepared_contracts: Option<&ResolvedMacroContracts>,
    distances: &BTreeMap<TilePos, u32>,
    issues: &mut Vec<WorldValidationIssue>,
) {
    const WORLD_NAMESPACE_PREFIX: u32 = 63;
    const MACRO_LOCAL_ID_BITS: u32 = 26;
    const TUNNEL_ROUTE: &str = "crystal_mountain.tunnel";
    const REQUIRED_ANCHORS: [&str; 9] = [
        "crystal_mountain.foot_apron",
        "crystal_mountain.tunnel_mouth",
        "crystal_mountain.midpoint",
        "crystal_mountain.gothic_transition",
        "crystal_mountain.ascent_threshold",
        "crystal_mountain.summit_exit",
        "crystal_mountain.basin_clearing",
        "crystal_mountain.ridge",
        MACRO_ROUTE_END,
    ];

    let instance_id = |name: &str| {
        settings
            .instances
            .iter()
            .position(|instance| instance.name == name)
            .and_then(|index| u32::try_from(index).ok())
            .map(PatchId)
    };
    let patch = |name: &str| {
        instance_id(name).and_then(|id| plan.layout.patches.get(&id).map(|patch| (id, patch)))
    };

    let expected_site = HexCoord::ORIGIN
        .within_radius(32)
        .into_iter()
        .collect::<BTreeSet<_>>();
    match patch("crystal-ascent") {
        Some((_, crystal)) if expected_site.is_subset(&crystal.mask) => {}
        Some((_, crystal)) => issues.push(macro_issue(format!(
            "Crystal Mountain landmark must contain the complete radius-32 site, got {} columns",
            crystal.mask.len()
        ))),
        None => issues.push(macro_issue(
            "Crystal Mountain has no resolved Crystal Ascent patch",
        )),
    }

    for name in REQUIRED_ANCHORS {
        if !plan.anchors.contains_key(name) {
            issues.push(macro_issue(format!(
                "Crystal Mountain is missing stable anchor {name:?}"
            )));
        }
    }
    for (alias, expected_level) in [
        ("crystal_mountain.foot_apron", 6),
        ("crystal_mountain.tunnel_mouth", 6),
        ("crystal_mountain.midpoint", 6),
        ("crystal_mountain.gothic_transition", 6),
        ("crystal_mountain.ascent_threshold", 6),
        ("crystal_mountain.summit_exit", 150),
    ] {
        if plan
            .anchors
            .get(alias)
            .is_some_and(|anchor| anchor.level != expected_level)
        {
            issues.push(macro_issue(format!(
                "Crystal Mountain anchor {alias:?} must remain at level {expected_level}"
            )));
        }
    }
    if plan
        .anchors
        .get("crystal_mountain.basin_clearing")
        .is_some_and(|anchor| !(149..=151).contains(&anchor.level))
    {
        issues.push(macro_issue(
            "Crystal Mountain basin clearing must remain within levels 149..=151",
        ));
    }
    if plan
        .anchors
        .get("crystal_mountain.ridge")
        .is_some_and(|anchor| !(178..=192).contains(&anchor.level))
    {
        issues.push(macro_issue(
            "Crystal Mountain ridge review anchor must remain within levels 178..=192",
        ));
    }

    let Some(route) = plan.features.protected_routes.get(TUNNEL_ROUTE) else {
        issues.push(macro_issue(
            "Crystal Mountain has no exact protected tunnel route",
        ));
        return;
    };
    if route.centerline.is_empty() || route.surfaces.is_empty() {
        issues.push(macro_issue(
            "Crystal Mountain tunnel route has no centerline or reserved footprint",
        ));
    }
    if route.surfaces.iter().any(|surface| {
        surface.level != 6
            || plan
                .volume
                .surfaces
                .get(surface)
                .is_none_or(|metadata| metadata.access != SurfaceAccess::Ordinary)
    }) {
        issues.push(macro_issue(
            "Crystal Mountain tunnel must contain only exact level-6 ordinary floors",
        ));
    }

    let route_owners = route
        .surfaces
        .iter()
        .filter_map(|surface| {
            plan.layout
                .patches
                .iter()
                .find_map(|(id, patch)| patch.mask.contains(&surface.coord).then_some(*id))
        })
        .collect::<BTreeSet<_>>();
    let expected_route_owners = [
        instance_id("outer-mountain"),
        instance_id("inner-mountain"),
        instance_id("crystal-ascent"),
    ]
    .into_iter()
    .flatten()
    .collect::<BTreeSet<_>>();
    if route_owners != expected_route_owners {
        issues.push(macro_issue(format!(
            "Crystal Mountain tunnel crosses unexpected logical instances: {route_owners:?}"
        )));
    }

    let resolved_contracts;
    let contracts: Result<&ResolvedMacroContracts, String> =
        if let Some(prepared) = prepared_contracts {
            Ok(prepared)
        } else {
            resolved_contracts = resolve_macro_contracts(settings, &plan.layout);
            resolved_contracts.as_ref().map_err(ToString::to_string)
        };
    match contracts {
        Ok(contracts) => {
            let ordinary = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
            for feature in contracts.spanning_features.values() {
                let super::layout::ResolvedMacroSpanningFeature::Tunnel(tunnel) = feature;
                if !tunnel.canonical_route {
                    continue;
                }
                for seam in &tunnel.seams {
                    for (source, destination) in &seam.port.lanes {
                        for coord in [source, destination] {
                            if !route
                                .surfaces
                                .contains(&TilePos::new(*coord, tunnel.floor_level))
                            {
                                issues.push(macro_issue(format!(
                                    "Crystal Mountain tunnel omits declared seam lane {coord:?}"
                                )));
                            }
                        }
                    }
                    let edge = plan.layout.shared_edges.values().find(|edge| {
                        BTreeSet::from([edge.first.0, edge.second.0])
                            == BTreeSet::from([seam.source, seam.destination])
                    });
                    let Some(edge) = edge else {
                        issues.push(macro_issue(format!(
                            "Crystal Mountain tunnel seam {:?}->{:?} has no shared edge",
                            seam.source, seam.destination
                        )));
                        continue;
                    };
                    let source_cells = seam
                        .port
                        .lanes
                        .iter()
                        .map(|(source, _)| *source)
                        .collect::<BTreeSet<_>>();
                    let destination_cells = seam
                        .port
                        .lanes
                        .iter()
                        .map(|(_, destination)| *destination)
                        .collect::<BTreeSet<_>>();
                    let oriented = |first: HexCoord, second: HexCoord| {
                        if edge.first.0 == seam.source {
                            (first, second)
                        } else {
                            (second, first)
                        }
                    };
                    let expected = edge
                        .boundary_pairs
                        .iter()
                        .map(|(first, second)| oriented(*first, *second))
                        .filter(|(source, destination)| {
                            source_cells.contains(source) && destination_cells.contains(destination)
                        })
                        .collect::<BTreeSet<_>>();
                    let actual = edge
                        .boundary_pairs
                        .iter()
                        .map(|(first, second)| oriented(*first, *second))
                        .filter(|(source, destination)| {
                            let source = TilePos::new(*source, tunnel.floor_level);
                            let destination = TilePos::new(*destination, tunnel.floor_level);
                            ordinary.admits(source, destination)
                                || ordinary.admits(destination, source)
                        })
                        .collect::<BTreeSet<_>>();
                    if actual != expected {
                        issues.push(macro_issue(format!(
                            "Crystal Mountain tunnel seam {:?}->{:?} admits {actual:?}, expected exact port footprint {expected:?}",
                            seam.source, seam.destination
                        )));
                    }
                }
            }
        }
        Err(error) => issues.push(macro_issue(format!(
            "Crystal Mountain resolved contracts changed during validation: {error}"
        ))),
    }

    let unified = plan
        .interiors
        .by_id
        .iter()
        .find(|(id, _)| id.0 >> MACRO_LOCAL_ID_BITS == WORLD_NAMESPACE_PREFIX);
    if plan.interiors.by_id.len() != 1 || unified.is_none() {
        issues.push(macro_issue(
            "Crystal Mountain tunnel and Crystal Ascent must form one world-owned interior",
        ));
    }
    if let Some((region, interior)) = unified {
        if interior.entrances.len() != 8
            || interior
                .entrances
                .iter()
                .filter(|entrance| entrance.level == 6)
                .count()
                != 4
            || interior
                .entrances
                .iter()
                .filter(|entrance| entrance.level == 150)
                .count()
                != 4
        {
            issues.push(macro_issue(
                "Crystal Mountain interior must expose only four foot and four summit threshold entrances",
            ));
        }
        let exterior_apron = route
            .surfaces
            .iter()
            .filter(|surface| {
                plan.volume
                    .surfaces
                    .get(surface)
                    .is_some_and(|metadata| metadata.interior.is_none())
            })
            .copied()
            .collect::<BTreeSet<_>>();
        if exterior_apron.len() != 8
            || exterior_apron.iter().any(|surface| {
                surface
                    .coord
                    .neighbors()
                    .into_iter()
                    .all(|neighbor| plan.volume.mask.contains(&neighbor))
            })
            || route.surfaces.iter().any(|surface| {
                !exterior_apron.contains(surface)
                    && plan
                        .volume
                        .surfaces
                        .get(surface)
                        .is_none_or(|metadata| metadata.interior != Some(*region))
            })
        {
            issues.push(macro_issue(
                "Crystal Mountain tunnel must have exactly eight exterior apron floors before its unified interior",
            ));
        }

        let world_lights = plan
            .lights
            .iter()
            .filter(|(id, _)| id.0 >> MACRO_LOCAL_ID_BITS == WORLD_NAMESPACE_PREFIX)
            .collect::<Vec<_>>();
        let fixture_origins = world_lights
            .iter()
            .map(|(_, light)| light.origin)
            .collect::<BTreeSet<_>>();
        for origin in fixture_origins {
            let pair = world_lights
                .iter()
                .filter(|(_, light)| light.origin == origin)
                .map(|(_, light)| *light)
                .collect::<Vec<_>>();
            let bright = pair.iter().filter(|light| {
                light.level == IlluminationLevel::Bright
                    && light.radius == 4
                    && matches!(
                        light.presentation,
                        Some(PlannedLightPresentation::CaveCrystal(_))
                    )
            });
            let dim = pair.iter().filter(|light| {
                light.level == IlluminationLevel::Dim
                    && light.radius == 18
                    && light.presentation.is_none()
            });
            if pair.len() != 2 || bright.count() != 1 || dim.count() != 1 {
                issues.push(macro_issue(format!(
                    "Crystal Mountain tunnel fixture at {origin:?} lacks one exact Bright-4/Dim-18 pair"
                )));
                break;
            }
        }
        let dim_sources = world_lights
            .iter()
            .map(|(_, light)| *light)
            .filter(|light| light.level == IlluminationLevel::Dim)
            .collect::<Vec<_>>();
        if route.surfaces.iter().any(|surface| {
            !dim_sources.iter().any(|source| {
                upper_dome_contains(
                    ExactGridPoint::voxel_center(source.origin),
                    ExactGridPoint::voxel_center(*surface),
                    source.radius,
                )
            })
        }) {
            issues.push(macro_issue(
                "every required Crystal Mountain tunnel floor must resolve to at least Dim",
            ));
        }
    }

    let route_order = [
        "crystal_mountain.foot_apron",
        "crystal_mountain.tunnel_mouth",
        "crystal_mountain.midpoint",
        "crystal_mountain.gothic_transition",
        "crystal_mountain.ascent_threshold",
        "crystal_mountain.summit_exit",
        "crystal_mountain.basin_clearing",
    ]
    .into_iter()
    .filter_map(|name| {
        plan.anchors
            .get(name)
            .and_then(|anchor| distances.get(anchor))
    })
    .copied()
    .collect::<Vec<_>>();
    if route_order.len() != 7
        || route_order
            .windows(2)
            .any(|pair| matches!(pair, [first, second] if first >= second))
    {
        issues.push(macro_issue(format!(
            "Crystal Mountain route anchors are not reached in canonical order: {route_order:?}"
        )));
    }

    if let (Some(party), Some(basin), Some(lower_terminal)) = (
        plan.anchors.get(PARTY_START).copied(),
        plan.anchors.get("crystal_mountain.basin_clearing").copied(),
        plan.features
            .protected_routes
            .iter()
            .find(|(name, _)| name.ends_with("crystal_ascent.lower_terminal_pad"))
            .map(|(_, route)| route),
    ) {
        let mut cut_blockers = plan.blockers.clone();
        cut_blockers.extend(lower_terminal.surfaces.iter().copied());
        let without_ascent = OrdinaryGraph::from_volume(&plan.volume, Some(&cut_blockers));
        if without_ascent.distances_from(party).contains_key(&basin) {
            issues.push(macro_issue(
                "Crystal Mountain exposes an ordinary foot-to-basin bypass outside Crystal Ascent",
            ));
        }
    }

    for name in ["inner-mountain", "outer-mountain"] {
        let Some((_, mountain)) = patch(name) else {
            continue;
        };
        if distances.keys().any(|surface| {
            mountain.mask.contains(&surface.coord)
                && surface.level != 6
                && !route.surfaces.contains(surface)
        }) {
            issues.push(macro_issue(format!(
                "Crystal Mountain party can reach the high {name:?} surface outside the tunnel"
            )));
        }
    }
    if let Some((_, forest)) = patch("summit-forest") {
        let playable_basin = plan
            .volume
            .surfaces
            .iter()
            .filter_map(|(surface, metadata)| {
                (forest.mask.contains(&surface.coord)
                    && metadata.access == SurfaceAccess::Ordinary
                    && !plan.blockers.contains(surface))
                .then_some(*surface)
            })
            .collect::<BTreeSet<_>>();
        let unreachable_basin = playable_basin
            .iter()
            .filter(|surface| !distances.contains_key(surface))
            .copied()
            .collect::<Vec<_>>();
        if !unreachable_basin.is_empty() {
            issues.push(macro_issue(format!(
                "Crystal Mountain summit connections leave {} of {} playable Forest surfaces unreachable; first gaps {:?}",
                unreachable_basin.len(),
                playable_basin.len(),
                unreachable_basin.iter().take(8).collect::<Vec<_>>()
            )));
        }
        let tree_count = plan
            .features
            .by_id
            .values()
            .filter(|feature| {
                feature.kind == FeatureKind::Tree && forest.mask.contains(&feature.root.coord)
            })
            .count();
        if tree_count == 0 {
            issues.push(macro_issue(
                "Crystal Mountain summit Forest must retain high-elevation broadleaf trees",
            ));
        }
        if plan
            .volume
            .surfaces
            .keys()
            .filter(|surface| forest.mask.contains(&surface.coord))
            .any(|surface| !(149..=151).contains(&surface.level))
        {
            issues.push(macro_issue(
                "Crystal Mountain summit Forest must remain within levels 149..=151",
            ));
        }
    }
}

fn validate_sea_strata(
    settings: &MacroLayoutSettings,
    plan: &GeneratedWorldPlan,
    issues: &mut Vec<WorldValidationIssue>,
) {
    for (index, instance) in settings.instances.iter().enumerate() {
        if !matches!(instance.recipe, V3RecipeSettings::ShallowSea(_)) {
            continue;
        }
        let id = PatchId(u32::try_from(index).unwrap_or(u32::MAX));
        let Some(patch) = plan.layout.patches.get(&id) else {
            continue;
        };
        for coord in &patch.mask {
            let Some(column) = plan.volume.columns.get(coord) else {
                continue;
            };
            let expected = vec![
                solid(0, 1, SolidMaterialRole::Bedrock),
                solid(1, 3, SolidMaterialRole::Stone),
                solid(3, 4, SolidMaterialRole::Dirt),
                solid(4, 5, SolidMaterialRole::Sand),
                VolumeElement::Fill(NonSolidFill {
                    levels: LevelInterval::new(5, 9),
                    material: FillMaterialRole::Water,
                }),
            ];
            if column.elements != expected {
                issues.push(macro_issue(format!(
                    "Shallow Sea column {coord:?} does not match exact 0..8 strata"
                )));
                return;
            }
        }
    }
}

fn validate_coastal_coverage(
    settings: &MacroLayoutSettings,
    plan: &GeneratedWorldPlan,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let liquid_coords = plan
        .volume
        .fill_runs_by_top()
        .keys()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    for (index, instance) in settings.instances.iter().enumerate() {
        let (range, authored_percent) = match &instance.recipe {
            V3RecipeSettings::Beach(beach) => (60..=75, beach.water_coverage_percent),
            V3RecipeSettings::Shore(shore) => (20..=40, shore.water_coverage_percent),
            _ => continue,
        };
        let id = PatchId(u32::try_from(index).unwrap_or(u32::MAX));
        let Some(patch) = plan.layout.patches.get(&id) else {
            continue;
        };
        let wet = patch
            .mask
            .iter()
            .filter(|coord| liquid_coords.contains(coord))
            .count();
        let percent = count_u32(wet)
            .saturating_mul(100)
            .checked_div(count_u32(patch.mask.len()))
            .unwrap_or_default();
        if !range.contains(&percent) {
            issues.push(macro_issue(format!(
                "coastal instance {:?} has {percent}% submerged footprint",
                instance.name
            )));
        }
        if percent.abs_diff(u32::from(authored_percent)) > 1 {
            issues.push(macro_issue(format!(
                "coastal instance {:?} authored {authored_percent}% water but generated {percent}%",
                instance.name
            )));
        }

        let local_wet = patch
            .mask
            .iter()
            .copied()
            .filter(|coord| liquid_coords.contains(coord))
            .collect::<BTreeSet<_>>();
        if matches!(&instance.recipe, V3RecipeSettings::Beach(_)) {
            let distances = distances_within(&patch.mask, &local_wet);
            let dry_sand_distances = patch
                .mask
                .iter()
                .filter(|coord| !local_wet.contains(coord))
                .filter_map(|coord| {
                    let top_material = plan.volume.columns.get(coord).and_then(|column| {
                        column
                            .elements
                            .iter()
                            .filter_map(|element| match element {
                                VolumeElement::Solid(mass) => {
                                    Some((mass.levels.top, mass.material))
                                }
                                VolumeElement::Fill(_) => None,
                            })
                            .max_by_key(|(top, _)| *top)
                            .map(|(_, material)| material)
                    });
                    (top_material == Some(SolidMaterialRole::Sand))
                        .then(|| distances.get(coord).copied())
                        .flatten()
                })
                .collect::<BTreeSet<_>>();
            let maximum_strip = dry_sand_distances.last().copied().unwrap_or_default();
            if !(2..=4).contains(&maximum_strip) {
                issues.push(macro_issue(format!(
                    "Beach {:?} needs a 2..=4-column dry sand strip, got distances {dry_sand_distances:?}",
                    instance.name
                )));
            }
        }

        if matches!(&instance.recipe, V3RecipeSettings::Shore(_)) {
            let top_levels = plan.volume.surfaces.keys().copied().fold(
                BTreeMap::<HexCoord, Level>::new(),
                |mut levels, surface| {
                    levels
                        .entry(surface.coord)
                        .and_modify(|level| *level = (*level).max(surface.level))
                        .or_insert(surface.level);
                    levels
                },
            );
            let cliff_drops = local_wet
                .iter()
                .flat_map(|wet| {
                    wet.neighbors().into_iter().filter_map(|dry| {
                        if local_wet.contains(&dry) || !patch.mask.contains(&dry) {
                            return None;
                        }
                        Some(top_levels.get(&dry)?.abs_diff(*top_levels.get(wet)?))
                    })
                })
                .collect::<BTreeSet<u32>>();
            if cliff_drops.is_empty() || cliff_drops.iter().any(|drop| !(3..=6).contains(drop)) {
                issues.push(macro_issue(format!(
                    "Shore {:?} needs 3..=6-level wet-edge cliffs, got {cliff_drops:?}",
                    instance.name
                )));
            }
        }
    }
}

fn validate_coastal_vegetation(
    settings: &MacroLayoutSettings,
    plan: &GeneratedWorldPlan,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let liquid_coords = plan
        .volume
        .fill_runs_by_top()
        .keys()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    for (index, instance) in settings.instances.iter().enumerate() {
        let expected = match &instance.recipe {
            V3RecipeSettings::Beach(_) => 2..=5,
            V3RecipeSettings::Shore(_) => 8..=12,
            _ => continue,
        };
        let id = PatchId(u32::try_from(index).unwrap_or(u32::MAX));
        let Some(patch) = plan.layout.patches.get(&id) else {
            continue;
        };
        let eligible_dry = plan
            .volume
            .surfaces
            .iter()
            .filter(|(surface, metadata)| {
                patch.mask.contains(&surface.coord)
                    && metadata.access == SurfaceAccess::Ordinary
                    && !liquid_coords.contains(&surface.coord)
            })
            .count();
        let tree_roots = plan
            .features
            .by_id
            .values()
            .filter(|feature| {
                feature.kind == FeatureKind::Tree && patch.mask.contains(&feature.root.coord)
            })
            .map(|feature| feature.root)
            .collect::<BTreeSet<_>>();
        let coverage = count_u32(tree_roots.len())
            .saturating_mul(100)
            .checked_div(count_u32(eligible_dry))
            .unwrap_or_default();
        if !expected.contains(&coverage) {
            issues.push(macro_issue(format!(
                "coastal instance {:?} needs tree coverage {expected:?} of eligible dry columns, got {coverage}% ({}/{eligible_dry})",
                instance.name,
                tree_roots.len(),
            )));
        }
        if !tree_roots.iter().all(|root| plan.blockers.contains(root)) {
            issues.push(macro_issue(format!(
                "coastal instance {:?} has a tree without its authored blocker",
                instance.name
            )));
        }
    }
}

fn validate_waterfall_flow(
    settings: &MacroLayoutSettings,
    plan: &GeneratedWorldPlan,
    issues: &mut Vec<WorldValidationIssue>,
) {
    for (index, instance) in settings.instances.iter().enumerate() {
        if !matches!(instance.recipe, V3RecipeSettings::Waterfall(_)) {
            continue;
        }
        let id = PatchId(u32::try_from(index).unwrap_or(u32::MAX));
        let Some(patch) = plan.layout.patches.get(&id) else {
            continue;
        };
        let states = plan
            .liquids
            .bodies
            .values()
            .flat_map(|body| &body.nodes)
            .filter(|(position, _)| patch.mask.contains(&position.coord))
            .map(|(_, node)| node.state)
            .collect::<BTreeSet<_>>();
        for required in [
            LiquidFlowState::Rapid,
            LiquidFlowState::Fall,
            LiquidFlowState::Current,
        ] {
            if !states.contains(&required) {
                issues.push(macro_issue(format!(
                    "Waterfall instance {:?} is missing {required:?} flow",
                    instance.name
                )));
            }
        }
    }
}

fn validate_mountain_watershed(
    settings: &MacroLayoutSettings,
    plan: &GeneratedWorldPlan,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let Some((_body_id, body)) = plan.liquids.bodies.first_key_value() else {
        issues.push(macro_issue(
            "Mountain Range watershed contains no water body",
        ));
        return;
    };
    if plan.liquids.bodies.len() != 1 {
        issues.push(macro_issue(format!(
            "Mountain Range watershed must compose into one water body, got {}",
            plan.liquids.bodies.len()
        )));
        return;
    }
    if body.material != FillMaterialRole::Water {
        issues.push(macro_issue(format!(
            "Mountain Range watershed must contain water, got {:?}",
            body.material
        )));
    }

    let patch_for = |index: usize| {
        plan.layout
            .patches
            .get(&PatchId(u32::try_from(index).unwrap_or(u32::MAX)))
    };
    let shallow_sea = settings
        .instances
        .iter()
        .enumerate()
        .find(|(_, instance)| matches!(instance.recipe, V3RecipeSettings::ShallowSea(_)))
        .and_then(|(index, _)| patch_for(index));
    let Some(shallow_sea) = shallow_sea else {
        issues.push(macro_issue(
            "Mountain Range watershed cannot resolve its Shallow Sea instance",
        ));
        return;
    };
    let shallow_nodes = body
        .nodes
        .iter()
        .filter(|(position, _)| shallow_sea.mask.contains(&position.coord))
        .collect::<Vec<_>>();
    if shallow_nodes.is_empty() {
        issues.push(macro_issue(
            "Mountain Range watershed has no liquid nodes in Shallow Sea",
        ));
        return;
    }
    if shallow_nodes
        .iter()
        .any(|(_, node)| node.state != LiquidFlowState::Still || node.downstream.is_some())
    {
        issues.push(macro_issue(
            "Mountain Range Shallow Sea must remain standing water without an authored current",
        ));
    }

    if let Some((source, target)) = body.nodes.iter().find_map(|(source, node)| {
        node.downstream
            .filter(|target| target.level > source.level)
            .map(|target| (*source, target))
    }) {
        issues.push(macro_issue(format!(
            "Mountain Range watershed flows uphill from {source:?} to {target:?}"
        )));
    }
    if liquid_graph_has_cycle(body) {
        issues.push(macro_issue("Mountain Range watershed contains a cycle"));
    }

    let waterfall_patches = settings
        .instances
        .iter()
        .enumerate()
        .filter(|(_, instance)| matches!(instance.recipe, V3RecipeSettings::Waterfall(_)))
        .filter_map(|(index, instance)| {
            patch_for(index).map(|patch| (instance.name.as_str(), patch))
        })
        .collect::<Vec<_>>();
    if waterfall_patches.len() != 2 {
        issues.push(macro_issue(format!(
            "Mountain Range watershed requires exactly two Waterfall tributaries, got {}",
            waterfall_patches.len()
        )));
        return;
    }
    let hills_center = settings
        .instances
        .iter()
        .position(|instance| instance.name == "hills-center")
        .and_then(patch_for);
    let Some(hills_center) = hills_center else {
        issues.push(macro_issue(
            "Mountain Range watershed cannot resolve its hills-center confluence",
        ));
        return;
    };

    let tributaries = waterfall_patches
        .iter()
        .map(|(name, patch)| {
            let exits = body
                .nodes
                .iter()
                .filter_map(|(position, node)| {
                    let downstream = node.downstream?;
                    (patch.mask.contains(&position.coord)
                        && !patch.mask.contains(&downstream.coord))
                    .then_some(*position)
                })
                .collect::<Vec<_>>();
            let traces = exits
                .iter()
                .filter_map(|source| match trace_liquid_downstream(body, *source) {
                    Ok(trace) => Some(trace),
                    Err(detail) => {
                        issues.push(macro_issue(format!(
                            "Mountain Range tributary {name:?} has an invalid downstream chain from {source:?}: {detail}"
                        )));
                        None
                    }
                })
                .collect::<Vec<_>>();
            if exits.is_empty() {
                issues.push(macro_issue(format!(
                    "Mountain Range tributary {name:?} has no directed outflow"
                )));
            }
            (*name, traces)
        })
        .collect::<Vec<_>>();
    let Some((first_name, first_traces)) = tributaries.first() else {
        return;
    };
    let Some((second_name, second_traces)) = tributaries.get(1) else {
        return;
    };

    for (name, traces, other_name, other_traces) in [
        (first_name, first_traces, second_name, second_traces),
        (second_name, second_traces, first_name, first_traces),
    ] {
        for trace in traces {
            let joins_other_tributary = other_traces.iter().any(|other| {
                trace.iter().any(|position| {
                    other.contains(position)
                        && hills_center.mask.contains(&position.coord)
                        && body
                            .nodes
                            .get(position)
                            .is_some_and(|node| node.downstream.is_some())
                })
            });
            if !joins_other_tributary {
                issues.push(macro_issue(format!(
                    "Mountain Range tributary {name:?} has an outflow lane that does not converge with {other_name:?} in hills-center"
                )));
                break;
            }
        }
    }

    let terminals = tributaries
        .iter()
        .flat_map(|(_, traces)| traces)
        .filter_map(|trace| trace.last().copied())
        .collect::<BTreeSet<_>>();
    if terminals.is_empty() {
        issues.push(macro_issue(
            "Mountain Range waterfall tributaries publish no terminal",
        ));
    }
    for terminal in terminals {
        if !still_water_reaches_mask(body, terminal, &shallow_sea.mask) {
            issues.push(macro_issue(format!(
                "Mountain Range waterfall terminal {terminal:?} does not join the Shallow Sea standing-water body"
            )));
        }
    }
}

fn liquid_graph_has_cycle(body: &LiquidBodyPlan) -> bool {
    let mut indegree = body
        .nodes
        .keys()
        .copied()
        .map(|position| (position, 0_usize))
        .collect::<BTreeMap<_, _>>();
    for node in body.nodes.values() {
        if let Some(count) = node
            .downstream
            .and_then(|downstream| indegree.get_mut(&downstream))
        {
            *count = count.saturating_add(1);
        }
    }
    let mut ready = indegree
        .iter()
        .filter_map(|(position, count)| (*count == 0).then_some(*position))
        .collect::<VecDeque<_>>();
    let mut visited = 0_usize;
    while let Some(position) = ready.pop_front() {
        visited = visited.saturating_add(1);
        let Some(downstream) = body.nodes.get(&position).and_then(|node| node.downstream) else {
            continue;
        };
        let Some(count) = indegree.get_mut(&downstream) else {
            continue;
        };
        *count = count.saturating_sub(1);
        if *count == 0 {
            ready.push_back(downstream);
        }
    }
    visited != body.nodes.len()
}

fn trace_liquid_downstream(body: &LiquidBodyPlan, start: TilePos) -> Result<Vec<TilePos>, String> {
    let mut trace = Vec::new();
    let mut visited = BTreeSet::new();
    let mut position = start;
    loop {
        if !visited.insert(position) {
            return Err(format!("cycle revisits {position:?}"));
        }
        trace.push(position);
        let Some(node) = body.nodes.get(&position) else {
            return Err(format!("missing liquid node {position:?}"));
        };
        let Some(downstream) = node.downstream else {
            if node.state != LiquidFlowState::Still {
                return Err(format!(
                    "moving {state:?} node {position:?} terminates before standing water",
                    state = node.state
                ));
            }
            return Ok(trace);
        };
        if downstream.level > position.level {
            return Err(format!("flow rises from {position:?} to {downstream:?}"));
        }
        position = downstream;
    }
}

fn still_water_reaches_mask(
    body: &LiquidBodyPlan,
    start: TilePos,
    target_mask: &BTreeSet<HexCoord>,
) -> bool {
    let is_standing = |position: &TilePos| {
        body.nodes
            .get(position)
            .is_some_and(|node| node.state == LiquidFlowState::Still && node.downstream.is_none())
    };
    if !is_standing(&start) {
        return false;
    }
    let mut reached = BTreeSet::from([start]);
    let mut pending = VecDeque::from([start]);
    while let Some(position) = pending.pop_front() {
        if target_mask.contains(&position.coord) {
            return true;
        }
        for coord in position.coord.neighbors() {
            let neighbor = TilePos::new(coord, position.level);
            if is_standing(&neighbor) && reached.insert(neighbor) {
                pending.push_back(neighbor);
            }
        }
    }
    false
}

fn macro_issue(detail: impl Into<String>) -> WorldValidationIssue {
    WorldValidationIssue::new(WorldIssueCode::Recipe("mountain_range"), detail)
}

fn format_issues(issues: &[WorldValidationIssue]) -> String {
    issues
        .iter()
        .map(|issue| format!("{:?}: {}", issue.code, issue.detail))
        .collect::<Vec<_>>()
        .join("; ")
}

fn count_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    use hex_core::TraversalProfile;

    use super::super::selection::{CandidateNote, CANDIDATE_COUNT};
    use super::*;
    use crate::settings::{
        CubeCoord, MacroAxisSettings, MacroBoundarySideSettings, MacroSpanningFeatureSettings,
        MapSettings, ProceduralSettings, TerrainSettings,
    };

    const MOUNTAIN_RANGE_RON: &str =
        include_str!("../../../../assets/config/worlds/procedural-mountain-range.ron");
    const CRYSTAL_MOUNTAIN_RON: &str =
        include_str!("../../../../assets/config/worlds/procedural-crystal-mountain.ron");
    const OCEAN_ARCHIPELAGO_RON: &str =
        include_str!("../../../../assets/config/worlds/procedural-ocean-archipelagoes.ron");
    const TWO_RINGS_RON: &str =
        include_str!("../../../../assets/config/worlds/procedural-two-rings.ron");

    #[test]
    fn waterfall_profile_marks_its_one_level_runout_as_rapid() {
        let path = (0..8)
            .map(|x| HexCoord::from_axial(x, 0))
            .collect::<Vec<_>>();
        let mut body = BTreeMap::new();
        let mut top_by_coord = BTreeMap::new();

        append_flow_path(&mut body, &mut top_by_coord, &path, 21, 16, None, true)
            .expect("five-level waterfall profile should fit an eight-column reach");

        let states = body
            .values()
            .map(|node| node.state)
            .collect::<BTreeSet<_>>();
        assert!(states.contains(&LiquidFlowState::Fall));
        assert!(states.contains(&LiquidFlowState::Rapid));
        assert!(states.contains(&LiquidFlowState::Current));
    }

    #[test]
    fn radius_three_macro_geometry_has_expected_counts() {
        let cells = HexCoord::ORIGIN.within_radius(3);
        let cell_set = cells.iter().copied().collect::<BTreeSet<_>>();
        let adjacencies = cells
            .iter()
            .map(|cell| {
                cell.neighbors()
                    .into_iter()
                    .filter(|neighbor| cell_set.contains(neighbor))
                    .count()
            })
            .sum::<usize>()
            / 2;
        let outer_sides = cells
            .iter()
            .flat_map(|cell| cell.neighbors())
            .filter(|neighbor| !cell_set.contains(neighbor))
            .count();

        assert_eq!(cells.len(), 37);
        assert_eq!(adjacencies, 90);
        assert_eq!(outer_sides, 42);
        assert_eq!(HexCoord::ORIGIN.within_radius(77).len(), 18_019);
    }

    #[test]
    fn summit_forest_tree_selection_skips_articulations_and_keeps_exact_density() {
        let coords = (0..6)
            .map(|q| HexCoord::from_axial(q, 0))
            .collect::<Vec<_>>();
        let mut volume = VolumePlan::new(coords.iter().copied().collect());
        for coord in &coords {
            assert!(
                volume
                    .columns
                    .insert(
                        *coord,
                        VolumeColumn {
                            elements: vec![VolumeElement::Solid(SolidMass {
                                levels: LevelInterval::new(0, 1),
                                material: SolidMaterialRole::Dirt,
                                cutaway_for: None,
                            })],
                        },
                    )
                    .is_some(),
                "VolumePlan::new should predeclare every fixture column"
            );
            assert!(
                volume
                    .surfaces
                    .insert(
                        TilePos::new(*coord, 0),
                        SurfaceMetadata {
                            access: SurfaceAccess::Ordinary,
                            interior: None,
                        },
                    )
                    .is_none(),
                "fixture surfaces should remain unique"
            );
        }
        let rotation = HexObjectRotation::new(0).expect("zero is a valid object rotation");
        let placement = |index: usize| {
            let root = TilePos::new(*coords.get(index).expect("fixture index should exist"), 0);
            (root, rotation, BTreeSet::from([root]))
        };
        let multi_cell_cut = (
            placement(2).0,
            rotation,
            BTreeSet::from([placement(2).0, placement(3).0]),
        );
        let empty_blocker = (placement(1).0, rotation, BTreeSet::new());
        let ranked = vec![
            multi_cell_cut,
            placement(2),
            empty_blocker,
            placement(0),
            placement(5),
        ];

        let ordinary = OrdinaryGraph::from_volume(&volume, None);
        let mut reference_blockers = BTreeSet::new();
        let mut reference = Vec::new();
        for candidate in ranked.iter().cloned() {
            if reference.len() == 3 {
                break;
            }
            let mut candidate_blockers = reference_blockers.clone();
            candidate_blockers.extend(candidate.2.iter().copied());
            if complete_ordinary_network_is_connected(&ordinary, &candidate_blockers) {
                reference_blockers = candidate_blockers;
                reference.push(candidate);
            }
        }

        let first = connected_vegetation_placements(&volume, &BTreeSet::new(), ranked.clone(), 3);
        let second = connected_vegetation_placements(&volume, &BTreeSet::new(), ranked, 3);
        assert_eq!(first, second, "ranked selection must be deterministic");
        assert_eq!(first, reference, "indexed filtering must match exact BFS");
        assert_eq!(
            first.len(),
            3,
            "the requested tree density must be retained"
        );
        let west = *coords.first().expect("fixture has a west endpoint");
        let east = *coords.last().expect("fixture has an east endpoint");
        assert_eq!(
            first
                .iter()
                .map(|placement| placement.0)
                .collect::<Vec<_>>(),
            [
                TilePos::new(*coords.get(1).expect("fixture has an empty blocker"), 0),
                TilePos::new(west, 0),
                TilePos::new(east, 0),
            ],
            "multi-cell and single articulation cuts should be skipped before safe candidates"
        );
        let blockers = first
            .iter()
            .flat_map(|placement| placement.2.iter().copied())
            .collect::<BTreeSet<_>>();
        let ordinary = OrdinaryGraph::from_volume(&volume, None);
        assert!(complete_ordinary_network_is_connected(&ordinary, &blockers));
    }

    #[test]
    fn shipped_crystal_mountain_resolves_exact_landmark_tunnel_and_summit_contracts() {
        let map = crystal_mountain_map();
        let settings = v3_settings(map);
        let setup = resolve_macro_world_setup(map.grid_radius, settings, runtime_art_catalog())
            .expect("shipped Crystal Mountain setup should resolve");
        assert_eq!(setup.layout.footprint.len(), 18_019);
        let crystal = setup
            .layout
            .patches
            .get(&PatchId(0))
            .expect("central Crystal patch should resolve");
        let authored_site = HexCoord::ORIGIN
            .within_radius(32)
            .into_iter()
            .collect::<BTreeSet<_>>();
        assert!(
            authored_site.is_subset(&crystal.mask),
            "the central-seven mask must be expanded enough to contain the authored site"
        );
        let mask_is_connected = |mask: &BTreeSet<HexCoord>| {
            let Some(start) = mask.first().copied() else {
                return false;
            };
            let mut reached = BTreeSet::from([start]);
            let mut pending = VecDeque::from([start]);
            while let Some(coord) = pending.pop_front() {
                for neighbor in coord.neighbors() {
                    if mask.contains(&neighbor) && reached.insert(neighbor) {
                        pending.push_back(neighbor);
                    }
                }
            }
            reached.len() == mask.len()
        };
        assert!(setup
            .layout
            .patches
            .values()
            .all(|patch| mask_is_connected(&patch.mask)));

        let connection = setup
            .contracts
            .walker_connections
            .first()
            .expect("summit walker connection should resolve");
        assert_eq!(connection.level, 150);
        assert_eq!(connection.port.lanes.len(), 4);
        let tunnel = setup
            .contracts
            .spanning_features
            .values()
            .next()
            .expect("canonical tunnel should resolve");
        let super::super::layout::ResolvedMacroSpanningFeature::Tunnel(tunnel) = tunnel;
        assert!(tunnel.canonical_route);
        assert_eq!(
            tunnel.instance_route,
            vec![PatchId(3), PatchId(2), PatchId(0)]
        );
        assert_eq!(tunnel.seams.len(), 2);
        assert!(tunnel.seams.iter().all(|seam| seam.port.lanes.len() == 4));

        let assets = setup
            .crystal_ascent_assets
            .as_ref()
            .expect("Crystal assets should be preflighted");
        let V3LayoutSettings::Macro(macro_settings) = &settings.layout else {
            panic!("shipped Crystal Mountain must remain Macro");
        };
        let Some(V3RecipeSettings::CrystalAscent(crystal_settings)) = macro_settings
            .instances
            .first()
            .map(|instance| &instance.recipe)
        else {
            panic!("central instance must remain Crystal Ascent");
        };
        let patch = PatchRecipeContext::resolve(&setup.layout, PatchId(0))
            .expect("central Crystal patch context should resolve");
        let fragment = construct_macro_crystal_ascent(
            patch,
            crystal_settings,
            map.level_height,
            None,
            &setup.layout,
            assets,
        )
        .expect("composite Crystal Ascent should construct");
        let upper_terminal = fragment
            .features
            .protected_routes
            .get("crystal_ascent.upper_terminal_pad")
            .expect("Crystal Ascent should publish its four-wide upper terminal");
        assert_eq!(upper_terminal.surfaces.len(), 4);
        let crystal_approach = if connection.first == PatchId(0) {
            &connection.port.first_approach
        } else {
            &connection.port.second_approach
        };
        assert!(
            upper_terminal
                .surfaces
                .iter()
                .all(|surface| crystal_approach.contains(&surface.coord)),
            "upper terminal {:?} must belong to resolved Crystal approach {:?}",
            upper_terminal.surfaces,
            crystal_approach
        );
        let crystal_lanes = if connection.first == PatchId(0) {
            connection
                .port
                .lanes
                .iter()
                .map(|(coord, _)| *coord)
                .collect::<BTreeSet<_>>()
        } else {
            connection
                .port
                .lanes
                .iter()
                .map(|(_, coord)| *coord)
                .collect::<BTreeSet<_>>()
        };
        assert!(crystal_lanes.iter().all(|coord| {
            fragment
                .volume
                .surfaces
                .contains_key(&TilePos::new(*coord, 150))
        }));

        let aliases = canonical_anchor_settings(macro_settings, &setup.contracts)
            .expect("Crystal Mountain aliases should resolve");
        assert_eq!(aliases.len(), 8);
        assert!(!aliases.contains_key(PARTY_START));
        assert!(aliases.contains_key("crystal_mountain.summit_exit"));

        let fragments = BTreeMap::from([(PatchId(0), fragment)]);
        let raw_destinations = raw_spanning_destinations(&setup.contracts, &fragments)
            .expect("the authored landmark should publish exact tunnel destination facts");
        let prepared = plan_macro_spanning(&setup.layout, &setup.contracts, &raw_destinations)
            .expect("the authored destination should resolve one prepared tunnel");
        validate_prepared_spanning(&prepared, &setup.contracts, &raw_destinations)
            .expect("prepared tunnel facts should match their source fragment");

        let assert_rejected = |forged: &RawSpanningDestinations, fact: &str| {
            let error = validate_prepared_spanning(&prepared, &setup.contracts, forged)
                .expect_err("a prepared tunnel must reject changed destination facts");
            assert!(
                error
                    .to_string()
                    .contains("disagrees with candidate-authored destination facts"),
                "changed {fact} produced the wrong fail-closed diagnostic: {error}"
            );
        };
        fn destination_mut(
            destinations: &mut RawSpanningDestinations,
        ) -> &mut RawSpanningDestination {
            destinations
                .values_mut()
                .next()
                .expect("Crystal Mountain should publish one spanning destination")
        }

        let mut forged_anchor = raw_destinations.clone();
        let destination = destination_mut(&mut forged_anchor);
        destination.anchor = TilePos::new(
            destination.anchor.coord,
            destination.anchor.level.saturating_add(1),
        );
        assert_rejected(&forged_anchor, "destination anchor");

        let mut forged_terminal = raw_destinations.clone();
        destination_mut(&mut forged_terminal).terminal.clear();
        assert_rejected(&forged_terminal, "destination terminal");

        let mut forged_summit = raw_destinations.clone();
        destination_mut(&mut forged_summit).summit_threshold.clear();
        assert_rejected(&forged_summit, "summit threshold");

        let mut forged_interior = raw_destinations.clone();
        let destination = destination_mut(&mut forged_interior);
        let authored_interior = destination
            .interior
            .expect("the Crystal destination must belong to its authored interior");
        destination.interior = Some(hex_core::InteriorRegionId(
            authored_interior.0.saturating_add(1),
        ));
        assert_rejected(&forged_interior, "destination interior");
    }

    #[test]
    fn shipped_ocean_archipelago_resolves_exact_cells_water_seams_and_home_port() {
        let map = ocean_archipelago_map();
        let settings = v3_settings(map);
        let V3LayoutSettings::Macro(macro_settings) = &settings.layout else {
            panic!("tracked Ocean Archipelagoes settings should use Macro");
        };
        assert!(is_ocean_archipelago_layout(macro_settings));
        let layout = super::super::layout::resolve_layout(map.grid_radius, settings)
            .expect("tracked Ocean Archipelagoes layout should resolve");
        assert_eq!(layout.grid_radius, 77);
        assert_eq!(layout.footprint.len(), 18_019);
        assert_eq!(layout.patches.len(), 6);
        assert_eq!(
            macro_settings
                .instances
                .iter()
                .map(|instance| instance.cells.len())
                .collect::<Vec<_>>(),
            [24, 2, 2, 2, 1, 6]
        );
        assert_eq!(
            layout
                .shared_edges
                .values()
                .filter(|edge| matches!(edge.liquid, ResolvedLiquidPort::Standing { .. }))
                .count(),
            10
        );
        let causeway_edges = layout
            .shared_edges
            .values()
            .filter(|edge| !edge.walker.ports.is_empty())
            .collect::<Vec<_>>();
        assert_eq!(causeway_edges.len(), 1);
        assert_eq!(
            causeway_edges.first().map(|edge| edge.walker.ports.len()),
            Some(1)
        );
        assert_eq!(
            causeway_edges
                .first()
                .and_then(|edge| edge.walker.ports.first())
                .map(|port| port.lanes.len()),
            Some(4)
        );
        assert_eq!(
            causeway_edges.first().map(|edge| edge.elevation.preferred),
            Some(9)
        );
        let causeway = causeway_edges
            .first()
            .copied()
            .expect("Ocean Archipelagoes should resolve one causeway seam");
        let ResolvedLiquidPort::Standing { port, elevation } = &causeway.liquid else {
            panic!("the causeway coast must retain standing ocean");
        };
        let walker_first_approach = causeway
            .walker
            .ports
            .iter()
            .flat_map(|port| port.first_approach.iter().copied())
            .collect::<BTreeSet<_>>();
        let walker_second_approach = causeway
            .walker
            .ports
            .iter()
            .flat_map(|port| port.second_approach.iter().copied())
            .collect::<BTreeSet<_>>();
        let causeway_exclusions = causeway
            .boundary_pairs
            .iter()
            .filter(|(first, second)| {
                walker_first_approach.contains(first) || walker_second_approach.contains(second)
            })
            .copied()
            .collect::<BTreeSet<_>>();
        assert_eq!(*elevation, ResolvedLiquidElevation::Exact(8));
        assert_eq!(
            port.lanes,
            causeway
                .boundary_pairs
                .difference(&causeway_exclusions)
                .copied()
                .collect(),
            "the full standing coast must exclude the exact protected causeway footprint"
        );
    }

    #[test]
    fn shipped_ocean_archipelago_constructs_seven_dry_components_and_one_sea() {
        let map = ocean_archipelago_map();
        let settings = v3_settings(map);
        let setup = resolve_macro_world_setup(map.grid_radius, settings, runtime_art_catalog())
            .expect("shipped Ocean Archipelagoes setup should resolve");
        let world = construct_world(map.level_height, &setup, None, None, true)
            .expect("shipped Ocean Archipelagoes canonical world should construct");
        assert!(world.validate().is_empty());
        let metrics = match validate_macro_world(settings, &world, None) {
            WorldValidation::Valid(MacroWorldMetrics {
                ocean_archipelago: Some(metrics),
                ..
            }) => metrics,
            WorldValidation::Valid(_) => {
                panic!("shipped Ocean Archipelagoes must publish its exact profile metrics")
            }
            WorldValidation::Invalid(issues) => panic!(
                "shipped Ocean Archipelagoes recipe validation failed: {}",
                format_issues(&issues)
            ),
        };
        assert_eq!(metrics.world_columns, 18_019);
        assert_eq!(metrics.macro_cells, 37);
        assert_eq!(metrics.biome_regions, 6);
        assert_eq!(metrics.standing_water_seams, 10);
        assert_eq!(metrics.dry_components, 7);
        assert_eq!(metrics.scenic_dry_components, 6);
        assert_eq!(world.liquids.bodies.len(), 1);
        assert!(metrics.tree_roots > 0);
        assert!(metrics.critical_route_steps > 0);
        assert!(metrics.reachable_surfaces < metrics.ordinary_surfaces);
    }

    #[test]
    fn shipped_ocean_archipelago_rejects_retargeted_world_aliases() {
        let map = ocean_archipelago_map();
        let setup =
            resolve_macro_world_setup(map.grid_radius, v3_settings(map), runtime_art_catalog())
                .expect("shipped Ocean Archipelagoes setup should resolve");
        let world = construct_world(map.level_height, &setup, None, None, true)
            .expect("shipped Ocean Archipelagoes canonical world should construct");

        let mut retargeted = map.clone();
        let TerrainSettings::Procedural(ProceduralSettings::V3(settings)) = &mut retargeted.terrain
        else {
            panic!("tracked Ocean Archipelagoes settings should use procedural V3");
        };
        let V3LayoutSettings::Macro(layout) = &mut settings.layout else {
            panic!("tracked Ocean Archipelagoes settings should use Macro");
        };
        layout
            .anchor_aliases
            .first_mut()
            .expect("the shipped profile should publish aliases")
            .anchor = "sandy_islets_channel_overlook".to_owned();

        let WorldValidation::Invalid(issues) = validate_macro_world(settings, &world, None) else {
            panic!("the exact Ocean Archipelagoes profile must reject a retargeted alias");
        };
        assert!(
            format_issues(&issues).contains("anchor alias sources"),
            "unexpected alias-profile diagnostic: {}",
            format_issues(&issues)
        );
    }

    #[test]
    fn shipped_crystal_mountain_constructs_one_complete_spanning_interior() {
        const WORLD_NAMESPACE_PREFIX: u32 = 63;
        const MACRO_LOCAL_ID_BITS: u32 = 26;

        let map = crystal_mountain_map();
        let settings = v3_settings(map);
        let setup = resolve_macro_world_setup(map.grid_radius, settings, runtime_art_catalog())
            .expect("shipped Crystal Mountain setup should resolve");
        let world = construct_world(map.level_height, &setup, None, None, true)
            .expect("shipped Crystal Mountain canonical world should construct");
        assert!(world.validate().is_empty());
        match validate_macro_world(settings, &world, None) {
            WorldValidation::Valid(_) => {}
            WorldValidation::Invalid(issues) => panic!(
                "shipped Crystal Mountain recipe validation failed: {}",
                format_issues(&issues)
            ),
        }

        let world_interiors = world
            .interiors
            .by_id
            .iter()
            .filter(|(id, _)| id.0 >> MACRO_LOCAL_ID_BITS == WORLD_NAMESPACE_PREFIX)
            .collect::<Vec<_>>();
        assert_eq!(world_interiors.len(), 1);
        let (interior_id, interior) = world_interiors
            .first()
            .copied()
            .expect("Crystal Mountain must publish one world-owned interior");
        assert_eq!(interior.entrances.len(), 8);

        let route = world
            .features
            .protected_routes
            .get("crystal_mountain.tunnel")
            .expect("Crystal Mountain should publish its exact tunnel route");
        let exterior = route
            .surfaces
            .iter()
            .filter(|surface| {
                world
                    .volume
                    .surfaces
                    .get(surface)
                    .is_some_and(|metadata| metadata.interior.is_none())
            })
            .count();
        assert_eq!(exterior, 8);
        assert!(route.surfaces.iter().all(|surface| {
            world.volume.surfaces.get(surface).is_some_and(|metadata| {
                metadata.interior.is_none() || metadata.interior == Some(*interior_id)
            })
        }));
        for anchor in [
            PARTY_START,
            "crystal_mountain.tunnel_mouth",
            "crystal_mountain.midpoint",
            "crystal_mountain.gothic_transition",
            "crystal_mountain.ascent_threshold",
            "crystal_mountain.summit_exit",
            "crystal_mountain.basin_clearing",
            "crystal_mountain.ridge",
        ] {
            assert!(world.anchors.contains_key(anchor), "missing {anchor:?}");
        }
        let review_anchors = [
            PARTY_START,
            "crystal_mountain.tunnel_mouth",
            "crystal_mountain.midpoint",
            "crystal_mountain.gothic_transition",
            "crystal_mountain.ascent_threshold",
            "crystal_mountain.summit_exit",
            "crystal_mountain.basin_clearing",
            "crystal_mountain.ridge",
        ]
        .into_iter()
        .map(|name| {
            (
                name,
                world
                    .anchors
                    .get(name)
                    .copied()
                    .expect("review anchor should exist"),
            )
        })
        .collect::<BTreeMap<_, _>>();
        assert_eq!(
            review_anchors,
            BTreeMap::from([
                (PARTY_START, TilePos::new(HexCoord::from_axial(-77, 3), 6),),
                (
                    "crystal_mountain.tunnel_mouth",
                    TilePos::new(HexCoord::from_axial(-76, 1), 6),
                ),
                (
                    "crystal_mountain.midpoint",
                    TilePos::new(HexCoord::from_axial(-48, -1), 6),
                ),
                (
                    "crystal_mountain.gothic_transition",
                    TilePos::new(HexCoord::from_axial(-29, -15), 6),
                ),
                (
                    "crystal_mountain.ascent_threshold",
                    TilePos::new(HexCoord::from_axial(-17, -15), 6),
                ),
                (
                    "crystal_mountain.summit_exit",
                    TilePos::new(HexCoord::from_axial(16, 15), 150),
                ),
                (
                    "crystal_mountain.basin_clearing",
                    TilePos::new(HexCoord::from_axial(15, 20), 150),
                ),
                (
                    "crystal_mountain.ridge",
                    TilePos::new(HexCoord::from_axial(77, 0), 192),
                ),
            ])
        );
    }

    #[test]
    fn crystal_mountain_representative_seed_candidates_preserve_the_authored_route() {
        let map = crystal_mountain_map();
        let settings = v3_settings(map);
        let mut expected_route = None;
        for (seed, candidate) in [(0_u64, 0_u8), (1_592_598_566, 7), (u64::MAX, 31)] {
            let setup = resolve_macro_world_setup(map.grid_radius, settings, runtime_art_catalog())
                .unwrap_or_else(|error| panic!("seed {seed} setup failed: {error}"));
            let world = construct_world(
                map.level_height,
                &setup,
                Some((seed, candidate)),
                None,
                true,
            )
            .unwrap_or_else(|error| panic!("seed {seed} candidate {candidate} failed: {error}"));
            match validate_macro_world(settings, &world, None) {
                WorldValidation::Valid(_) => {}
                WorldValidation::Invalid(issues) => panic!(
                    "seed {seed} candidate {candidate} failed validation: {}",
                    format_issues(&issues)
                ),
            }
            let route = world
                .features
                .protected_routes
                .get("crystal_mountain.tunnel")
                .expect("candidate should publish the canonical tunnel")
                .surfaces
                .clone();
            match &expected_route {
                Some(expected) => assert_eq!(
                    &route, expected,
                    "seed variation must not move the authored spanning route"
                ),
                None => expected_route = Some(route),
            }
        }
    }

    #[test]
    fn crystal_mountain_prepared_spanning_matches_candidate_local_planning() {
        use super::super::fingerprint::semantic_plan_fingerprint;

        let map = crystal_mountain_map();
        let settings = v3_settings(map);
        let setup = resolve_macro_world_setup(map.grid_radius, settings, runtime_art_catalog())
            .expect("shipped Crystal Mountain setup should resolve");
        let prepared = prepare_macro_spanning(map.level_height, &setup)
            .expect("spanning preparation should succeed")
            .expect("Crystal Mountain should prepare its canonical tunnel");
        let candidate = Some((1_592_598_566, 3));

        let candidate_planned = construct_world(map.level_height, &setup, candidate, None, false)
            .expect("candidate-local tunnel planning should construct");
        let prepared_reuse =
            construct_world(map.level_height, &setup, candidate, Some(&prepared), false)
                .expect("prepared tunnel reuse should construct");

        assert_eq!(
            semantic_plan_fingerprint(&prepared_reuse),
            semantic_plan_fingerprint(&candidate_planned),
            "hoisting candidate-independent tunnel planning must preserve the exact semantic world",
        );
        assert!(candidate_planned.validate().is_empty());
        assert!(prepared_reuse.validate().is_empty());
        let validate = |world: &GeneratedWorldPlan| match validate_macro_world(
            settings,
            world,
            Some(&setup.contracts),
        ) {
            WorldValidation::Valid(metrics) => metrics,
            WorldValidation::Invalid(issues) => panic!(
                "prepared-spanning differential world failed validation: {}",
                format_issues(&issues)
            ),
        };
        let candidate_metrics = validate(&candidate_planned);
        let prepared_metrics = validate(&prepared_reuse);
        assert_eq!(candidate_metrics.report, prepared_metrics.report);
        assert_eq!(
            candidate_metrics.mountain_range,
            prepared_metrics.mountain_range
        );
        let score = |metrics: &MacroWorldMetrics| {
            (
                Reverse(
                    metrics
                        .mountain_range
                        .map_or(0, |mountain| mountain.high_massif_surfaces),
                ),
                Reverse(metrics.report.reachable_surfaces),
            )
        };
        assert_eq!(score(&candidate_metrics), score(&prepared_metrics));
    }

    #[test]
    fn crystal_mountain_global_rotation_contracts_resolve_for_all_six_turns() {
        let expected_boundary_sides = [
            MacroBoundarySideSettings::West,
            MacroBoundarySideSettings::SouthWest,
            MacroBoundarySideSettings::SouthEast,
            MacroBoundarySideSettings::East,
            MacroBoundarySideSettings::NorthEast,
            MacroBoundarySideSettings::NorthWest,
        ];
        let expected_outer_axes = [
            MacroAxisSettings::East,
            MacroAxisSettings::NorthEast,
            MacroAxisSettings::NorthWest,
            MacroAxisSettings::West,
            MacroAxisSettings::SouthWest,
            MacroAxisSettings::SouthEast,
        ];

        for turns in 0..6 {
            let map = rotated_crystal_mountain_map(turns);
            map.validate()
                .unwrap_or_else(|error| panic!("rotation {turns} settings failed: {error}"));
            let settings = v3_settings(&map);
            let V3LayoutSettings::Macro(layout) = &settings.layout else {
                panic!("rotated Crystal Mountain settings should remain Macro");
            };
            let outer = layout
                .instances
                .iter()
                .find(|instance| instance.name == "outer-mountain")
                .expect("rotated settings retain the outer mountain");
            assert_eq!(
                outer.elevation.grade_axis,
                *expected_outer_axes
                    .get(usize::from(turns))
                    .expect("six expected grade axes")
            );
            assert!(layout
                .instances
                .iter()
                .all(|instance| instance.rotation_turns == turns));
            let MacroSpanningFeatureSettings::Tunnel(tunnel) = layout
                .spanning_features
                .first()
                .expect("rotated settings retain the canonical tunnel");
            assert_eq!(
                tunnel.boundary_terminal.side,
                *expected_boundary_sides
                    .get(usize::from(turns))
                    .expect("six expected boundary sides")
            );

            let setup = resolve_macro_world_setup(map.grid_radius, settings, runtime_art_catalog())
                .unwrap_or_else(|error| panic!("rotation {turns} setup failed: {error}"));
            assert_eq!(setup.layout.footprint.len(), 18_019);
            assert_eq!(setup.contracts.walker_connections.len(), 1);
            assert_eq!(setup.contracts.spanning_features.len(), 1);
        }
    }

    #[test]
    #[ignore = "release acceptance constructs and validates six radius-77 Crystal Mountain rotations"]
    fn crystal_mountain_constructs_as_one_valid_world_in_all_six_global_rotations() {
        let ascent_threshold = TilePos::new(HexCoord::from_axial(-17, -15), 6);
        let summit_exit = TilePos::new(HexCoord::from_axial(16, 15), 150);

        for turns in 0..6 {
            let map = rotated_crystal_mountain_map(turns);
            let settings = v3_settings(&map);
            let setup = resolve_macro_world_setup(map.grid_radius, settings, runtime_art_catalog())
                .unwrap_or_else(|error| panic!("rotation {turns} setup failed: {error}"));
            let world = construct_world(
                map.level_height,
                &setup,
                Some((1_592_598_566, 0)),
                None,
                true,
            )
            .unwrap_or_else(|error| panic!("rotation {turns} construction failed: {error}"));
            match validate_macro_world(settings, &world, None) {
                WorldValidation::Valid(_) => {}
                WorldValidation::Invalid(issues) => panic!(
                    "rotation {turns} validation failed: {}",
                    format_issues(&issues)
                ),
            }

            assert_eq!(world.layout.footprint.len(), 18_019);
            let route = world
                .features
                .protected_routes
                .get("crystal_mountain.tunnel")
                .expect("every rotation should publish the spanning route");
            assert!(route.surfaces.iter().all(|surface| surface.level == 6));
            for anchor in [
                "crystal_mountain.foot_apron",
                "crystal_mountain.ascent_threshold",
                "crystal_mountain.summit_exit",
                "crystal_mountain.basin_clearing",
                "crystal_mountain.ridge",
            ] {
                assert!(
                    world.anchors.contains_key(anchor),
                    "rotation {turns} omitted {anchor:?}"
                );
            }
            assert_eq!(
                world
                    .anchors
                    .get("crystal_mountain.ascent_threshold")
                    .copied(),
                Some(rotate_tile_pos(ascent_threshold, turns))
            );
            assert_eq!(
                world.anchors.get("crystal_mountain.summit_exit").copied(),
                Some(rotate_tile_pos(summit_exit, turns))
            );
        }
    }

    #[test]
    #[ignore = "32-seed release corpus for the radius-77 Crystal Mountain world"]
    fn crystal_mountain_release_corpus_validates_32_seeds() {
        for seed in 0..32_u64 {
            let selection = generate_crystal_mountain(seed)
                .unwrap_or_else(|error| panic!("Crystal Mountain seed {seed} failed: {error}"));
            assert!(
                !selection.used_fallback,
                "Crystal Mountain seed {seed} unexpectedly used its fallback"
            );
        }
    }

    #[test]
    fn deep_mountain_thresholds_drive_macro_snow_and_vegetation_ceilings() {
        let map = mountain_range_map();
        let V3LayoutSettings::Macro(mut settings) = v3_settings(map).layout.clone() else {
            panic!("tracked Mountain Range should use the Macro layout");
        };
        assert_eq!(
            resolve_macro_alpine_climate(&settings)
                .expect("tracked Mountain Range climate should resolve"),
            MacroAlpineClimate {
                treeline: 36,
                snowline: 52,
            }
        );

        let deep_mountain = settings
            .instances
            .iter_mut()
            .find(|instance| matches!(instance.recipe, V3RecipeSettings::DeepMountain(_)))
            .expect("tracked Mountain Range should contain Deep Mountain");
        let V3RecipeSettings::DeepMountain(deep_settings) = &mut deep_mountain.recipe else {
            unreachable!("the selected instance is Deep Mountain");
        };
        deep_settings.treeline = 40;
        deep_settings.snowline = 60;

        let climate = resolve_macro_alpine_climate(&settings)
            .expect("mutated valid Mountain Range climate should resolve");
        assert_eq!(
            climate,
            MacroAlpineClimate {
                treeline: 40,
                snowline: 60,
            }
        );
        let mountain = settings
            .instances
            .iter()
            .find(|instance| matches!(instance.recipe, V3RecipeSettings::Mountains(_)))
            .expect("tracked Mountain Range should contain a Mountains tier");
        assert_eq!(
            surface_material(mountain, 59, None, climate),
            SolidMaterialRole::Stone
        );
        assert_eq!(
            surface_material(mountain, 60, None, climate),
            SolidMaterialRole::Snow
        );
        assert!(vegetation_below_climate_ceiling(
            &mountain.recipe,
            FeatureKind::Tree,
            39,
            climate
        ));
        assert!(!vegetation_below_climate_ceiling(
            &mountain.recipe,
            FeatureKind::Tree,
            40,
            climate
        ));
        assert!(vegetation_below_climate_ceiling(
            &mountain.recipe,
            FeatureKind::TallGrass,
            35,
            climate
        ));
        assert!(!vegetation_below_climate_ceiling(
            &mountain.recipe,
            FeatureKind::TallGrass,
            36,
            climate
        ));
        let forest = settings
            .instances
            .iter()
            .find(|instance| matches!(instance.recipe, V3RecipeSettings::Forest(_)))
            .expect("tracked Mountain Range should contain a Forest instance");
        assert!(vegetation_below_climate_ceiling(
            &forest.recipe,
            FeatureKind::Tree,
            150,
            climate
        ));
    }

    #[test]
    fn macro_runner_rejects_candidate_construction_but_keeps_setup_failures_fatal() {
        let map = mountain_range_map();
        let settings = v3_settings(map);
        let setup = resolve_macro_world_setup(map.grid_radius, settings, runtime_art_catalog())
            .expect("tracked Mountain Range setup should resolve");
        let recipe = MacroWorldRecipe {
            grid_radius: map.grid_radius,
            level_height: map.level_height,
            setup,
            prepared_spanning: None,
            force_candidate_construction_failure: true,
        };
        let selection = run_recipe(&recipe, settings, map.grid_radius, 129_704_046)
            .expect("candidate-local Macro failures should leave the canonical fallback available");

        assert!(selection.used_fallback);
        assert_eq!(selection.selected_candidate, None);
        assert_eq!(selection.candidates_evaluated, CANDIDATE_COUNT);
        assert_eq!(selection.valid_candidates, 0);
        let rejected = selection
            .notes
            .iter()
            .filter_map(|note| match note {
                CandidateNote::ConstructionRejected { candidate, issues } => {
                    Some((*candidate, issues))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(rejected.len(), usize::from(CANDIDATE_COUNT));
        for (expected, (candidate, issues)) in (0..CANDIDATE_COUNT).zip(rejected) {
            assert_eq!(candidate, expected);
            assert_eq!(issues.len(), 1);
            let issue = issues
                .first()
                .expect("each rejected Macro candidate should retain one typed diagnostic");
            assert_eq!(issue.code, WorldIssueCode::Recipe("mountain_range"));
            assert_eq!(
                issue.detail,
                "forced candidate-local Macro construction failure"
            );
        }
        assert!(matches!(
            selection.notes.last(),
            Some(CandidateNote::FallbackSelected)
        ));

        let non_macro = two_rings_map();
        let error = run_recipe(
            &recipe,
            v3_settings(non_macro),
            non_macro.grid_radius,
            129_704_046,
        )
        .expect_err("candidate-independent Macro setup failures must stop the runner");
        assert!(matches!(
            error,
            V3GenerationError::FatalCandidateConstruction {
                candidate: 0,
                source,
            } if matches!(
                *source,
                V3GenerationError::RecipeContract(ref detail)
                    if detail == "Macro runner requires V3LayoutSettings::Macro"
            )
        ));
    }

    #[test]
    fn mountain_range_anchor_roles_are_distinct_and_keep_their_placement_contracts() {
        const REVIEW_SEED: u64 = 129_704_046;

        let selection = generate_mountain_range(REVIEW_SEED)
            .expect("tracked Mountain Range anchors should generate");
        let plan = &selection.validated.plan;
        let V3LayoutSettings::Macro(settings) = &v3_settings(mountain_range_map()).layout else {
            panic!("tracked Mountain Range should use the Macro layout");
        };
        let contracts = resolve_macro_contracts(settings, &plan.layout)
            .expect("tracked Macro contracts should resolve");
        let anchor_settings = canonical_anchor_settings(settings, &contracts)
            .expect("tracked canonical anchors should resolve");
        let liquid_coords = plan
            .liquids
            .bodies
            .values()
            .flat_map(|body| body.nodes.keys().map(|position| position.coord))
            .collect::<BTreeSet<_>>();

        for names in [
            &[PARTY_START, COAST_REVIEW][..],
            &[HOSTILE_START, FOOTHILL_REVIEW][..],
            &[MACRO_ROUTE_END, DEEP_MOUNTAIN_BASE, DEEP_MOUNTAIN_REVIEW][..],
        ] {
            let anchors = names
                .iter()
                .map(|name| {
                    plan.anchors
                        .get(*name)
                        .copied()
                        .unwrap_or_else(|| panic!("Mountain Range should publish {name:?}"))
                })
                .collect::<Vec<_>>();
            assert_eq!(
                anchors.iter().copied().collect::<BTreeSet<_>>().len(),
                anchors.len(),
                "anchor roles in one biome instance must not alias: {names:?} -> {anchors:?}"
            );
            assert_eq!(
                anchors
                    .iter()
                    .map(|anchor| anchor.coord)
                    .collect::<BTreeSet<_>>()
                    .len(),
                anchors.len(),
                "anchor roles in one biome instance should use different columns: \
                 {names:?} -> {anchors:?}"
            );
        }

        for (name, anchor) in &plan.anchors {
            if !anchor_settings.contains_key(name) {
                continue;
            }
            assert_eq!(
                plan.volume
                    .surfaces
                    .get(anchor)
                    .map(|metadata| metadata.access),
                Some(SurfaceAccess::Ordinary),
                "canonical anchor {name:?} should use an ordinary surface"
            );
            assert!(
                !liquid_coords.contains(&anchor.coord),
                "canonical anchor {name:?} should use a dry column, got {anchor:?}"
            );
        }

        for name in [PARTY_START, HOSTILE_START, MACRO_ROUTE_END] {
            let anchor = plan
                .anchors
                .get(name)
                .copied()
                .unwrap_or_else(|| panic!("Mountain Range should publish {name:?}"));
            let target = anchor_settings
                .get(name)
                .unwrap_or_else(|| panic!("Mountain Range should resolve {name:?}"));
            let patch = PatchRecipeContext::resolve(&plan.layout, target.patch)
                .expect("canonical functional anchor patch should resolve");
            assert!(
                patch.walker_protected_approaches().contains(&anchor.coord),
                "functional anchor {name:?} should remain on a protected route approach, \
                 got {anchor:?}"
            );
        }

        for name in [BEACH_REVIEW, COAST_REVIEW] {
            let anchor = plan
                .anchors
                .get(name)
                .copied()
                .unwrap_or_else(|| panic!("Mountain Range should publish {name:?}"));
            assert!(
                anchor
                    .coord
                    .neighbors()
                    .into_iter()
                    .any(|neighbor| liquid_coords.contains(&neighbor)),
                "coastal review anchor {name:?} should remain at the water edge, got {anchor:?}"
            );
        }
    }

    #[test]
    fn mountain_range_massif_shoulders_and_protected_climb_are_continuous() {
        const REVIEW_SEED: u64 = 129_704_046;
        const MACRO_CELL_OFFSET: i32 = 22;

        let selection = generate_mountain_range(REVIEW_SEED)
            .expect("tracked Mountain Range alpine terrain should generate");
        let plan = &selection.validated.plan;
        let V3LayoutSettings::Macro(settings) = &v3_settings(mountain_range_map()).layout else {
            panic!("tracked Mountain Range should use the Macro layout");
        };
        let (deep_index, deep_instance) = settings
            .instances
            .iter()
            .enumerate()
            .find(|(_, instance)| instance.name == "deep-mountain")
            .expect("Mountain Range should author one Deep Mountain instance");
        let deep_id = PatchId(u32::try_from(deep_index).expect("instance index should fit u32"));
        let deep_patch = plan
            .layout
            .patches
            .get(&deep_id)
            .expect("Deep Mountain should resolve to one logical patch");
        let surface_levels = highest_surface_levels(plan);

        let atomic_cells = HexCoord::ORIGIN
            .within_radius(settings.macro_radius)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let atomic_centers = atomic_cells
            .iter()
            .copied()
            .map(|cell| {
                let [x, y, z] = cell.to_cubic_array();
                (
                    cell,
                    HexCoord::new_cubic(
                        x * MACRO_CELL_OFFSET,
                        y * MACRO_CELL_OFFSET,
                        z * MACRO_CELL_OFFSET,
                    ),
                )
            })
            .collect::<Vec<_>>();
        let deep_atomic_cells = deep_instance
            .cells
            .iter()
            .map(|cell| {
                HexCoord::try_new_cubic(cell.x, cell.y, cell.z)
                    .expect("validated Deep Mountain cells should be valid cube coordinates")
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(deep_atomic_cells.len(), 5);

        let mut surfaces_by_cell = BTreeMap::<HexCoord, usize>::new();
        let mut shoulders_by_cell = BTreeMap::<HexCoord, usize>::new();
        for coord in &deep_patch.mask {
            let level = surface_levels
                .get(coord)
                .copied()
                .expect("every Deep Mountain column should expose a surface");
            let owner = nearest_macro_cell(*coord, &atomic_centers);
            assert!(
                deep_atomic_cells.contains(&owner),
                "Deep Mountain union mask included a column owned by {owner:?}"
            );
            *surfaces_by_cell.entry(owner).or_default() += 1;
            if (60..=76).contains(&level) {
                *shoulders_by_cell.entry(owner).or_default() += 1;
            }
        }
        assert_eq!(
            surfaces_by_cell.keys().copied().collect::<BTreeSet<_>>(),
            deep_atomic_cells,
            "the logical Deep Mountain must retain all five authored atomic cells"
        );
        let shoulder_count = shoulders_by_cell.values().sum::<usize>();
        assert!(
            shoulder_count.saturating_mul(100) >= deep_patch.mask.len().saturating_mul(30),
            "levels 60-76 should form broad massif shoulders, got {shoulder_count}/{} columns",
            deep_patch.mask.len()
        );
        for (cell, surface_count) in &surfaces_by_cell {
            let cell_shoulders = shoulders_by_cell.get(cell).copied().unwrap_or_default();
            assert!(
                cell_shoulders.saturating_mul(100) >= surface_count.saturating_mul(25),
                "Deep Mountain atomic cell {cell:?} lacks a broad 60-76 shoulder: \
                 {cell_shoulders}/{surface_count} columns"
            );
        }

        let mut expected_internal_edges = BTreeSet::new();
        for cell in &deep_atomic_cells {
            for neighbor in cell.neighbors() {
                if deep_atomic_cells.contains(&neighbor) && *cell < neighbor {
                    expected_internal_edges.insert((*cell, neighbor));
                }
            }
        }
        let mut observed_internal_edges = BTreeSet::new();
        let mut cross_boundary_pairs = 0_usize;
        let mut maximum_cross_boundary_jump = 0;
        let mut maximum_cross_boundary_pair = None;
        for coord in &deep_patch.mask {
            let owner = nearest_macro_cell(*coord, &atomic_centers);
            let level = surface_levels.get(coord).copied().unwrap_or_default();
            for neighbor in coord.neighbors() {
                if *coord >= neighbor || !deep_patch.mask.contains(&neighbor) {
                    continue;
                }
                let neighbor_owner = nearest_macro_cell(neighbor, &atomic_centers);
                if owner == neighbor_owner {
                    continue;
                }
                let neighbor_level = surface_levels
                    .get(&neighbor)
                    .copied()
                    .expect("adjacent Deep Mountain column should expose a surface");
                observed_internal_edges.insert(ordered_coord_pair(owner, neighbor_owner));
                cross_boundary_pairs = cross_boundary_pairs.saturating_add(1);
                let jump = level.abs_diff(neighbor_level);
                if jump > maximum_cross_boundary_jump {
                    maximum_cross_boundary_jump = jump;
                    maximum_cross_boundary_pair = Some((*coord, level, neighbor, neighbor_level));
                }
            }
        }
        assert_eq!(observed_internal_edges, expected_internal_edges);
        assert!(cross_boundary_pairs > 0);
        assert!(
            maximum_cross_boundary_jump <= 6,
            "erased atomic-cell boundaries must retain the union height field's local gradient; \
             maximum jump was {maximum_cross_boundary_jump} levels at \
             {maximum_cross_boundary_pair:?}"
        );

        let ordinary = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
        let party = plan
            .anchors
            .get(PARTY_START)
            .copied()
            .expect("Mountain Range should publish the party anchor");
        let reachable = ordinary.distances_from(party);
        for (lower_name, higher_name, expected_datum) in [
            ("hills-center", "mountains-tier1-center", 24),
            ("mountains-tier1-center", "mountains-tier2-center", 34),
            ("mountains-tier2-center", "deep-mountain", 48),
        ] {
            let lower = named_instance_id(settings, lower_name);
            let higher = named_instance_id(settings, higher_name);
            let edge = plan
                .layout
                .shared_edges
                .values()
                .find(|edge| {
                    (edge.first.0 == lower && edge.second.0 == higher)
                        || (edge.first.0 == higher && edge.second.0 == lower)
                })
                .unwrap_or_else(|| panic!("{lower_name} should border {higher_name}"));
            assert_eq!(
                edge.elevation.preferred, expected_datum,
                "{lower_name}->{higher_name} should use its authored climb datum"
            );
            assert_eq!(edge.walker.count, 1);
            assert_eq!(edge.walker.width, 2);
            if lower_name.starts_with("mountains-") {
                let maximum_seam_jump = edge
                    .boundary_pairs
                    .iter()
                    .map(|(first, second)| {
                        let first_level = surface_levels
                            .get(first)
                            .copied()
                            .expect("lower alpine boundary should expose a surface");
                        let second_level = surface_levels
                            .get(second)
                            .copied()
                            .expect("higher alpine boundary should expose a surface");
                        first_level.abs_diff(second_level)
                    })
                    .max()
                    .unwrap_or_default();
                assert!(
                    maximum_seam_jump <= 1,
                    "the complete {lower_name}->{higher_name} seam should remain continuous; \
                     maximum jump was {maximum_seam_jump} levels"
                );
            }
            let port = edge
                .walker
                .ports
                .first()
                .expect("every critical climb seam should resolve one walker port");
            assert_eq!(edge.walker.ports.len(), 1);
            for (first, second) in &port.lanes {
                let first = TilePos::new(*first, expected_datum);
                let second = TilePos::new(*second, expected_datum);
                assert!(ordinary.admits(first, second));
                assert!(ordinary.admits(second, first));
            }
            for coord in port.first_approach.iter().chain(&port.second_approach) {
                let position = TilePos::new(*coord, expected_datum);
                assert_eq!(
                    plan.volume
                        .surfaces
                        .get(&position)
                        .map(|surface| surface.access),
                    Some(SurfaceAccess::Ordinary),
                    "protected {lower_name}->{higher_name} approach {coord:?} should remain \
                     ordinary at level {expected_datum}"
                );
                assert!(
                    plan.volume
                        .surface_headroom(position)
                        .is_some_and(|headroom| {
                            headroom.0 >= TraversalProfile::WALKER.levels_tall
                        }),
                    "protected climb approach {position:?} should retain walker headroom"
                );
                assert!(
                    reachable.contains_key(&position),
                    "protected climb approach {position:?} should be reachable from the party"
                );
            }
        }
    }

    #[test]
    fn mountain_range_waterfalls_converge_and_terminate_in_shallow_sea() {
        const REVIEW_SEED: u64 = 129_704_046;

        let mut selection = generate_mountain_range(REVIEW_SEED)
            .expect("tracked Mountain Range watershed should generate");
        let V3LayoutSettings::Macro(settings) = &v3_settings(mountain_range_map()).layout else {
            panic!("tracked Mountain Range should use the Macro layout");
        };
        let mut issues = Vec::new();

        validate_mountain_watershed(settings, &selection.validated.plan, &mut issues);

        assert!(
            issues.is_empty(),
            "tracked Mountain Range watershed violated its confluence contract: {}",
            format_issues(&issues)
        );

        let first_waterfall_index = settings
            .instances
            .iter()
            .position(|instance| matches!(instance.recipe, V3RecipeSettings::Waterfall(_)))
            .expect("Mountain Range should contain a Waterfall tributary");
        let waterfall_mask = selection
            .validated
            .plan
            .layout
            .patches
            .get(&PatchId(
                u32::try_from(first_waterfall_index).expect("instance index should fit a patch id"),
            ))
            .expect("Waterfall instance should resolve to a patch")
            .mask
            .clone();
        let body_id = selection
            .validated
            .plan
            .liquids
            .bodies
            .first_key_value()
            .map(|(body_id, _)| *body_id)
            .expect("Mountain Range should contain its watershed");
        let severed_nodes = selection
            .validated
            .plan
            .liquids
            .bodies
            .get(&body_id)
            .expect("Mountain Range watershed body should remain present")
            .nodes
            .iter()
            .filter_map(|(position, node)| {
                node.downstream
                    .filter(|downstream| {
                        waterfall_mask.contains(&position.coord)
                            && !waterfall_mask.contains(&downstream.coord)
                    })
                    .map(|_| (*position, *node))
            })
            .collect::<Vec<_>>();
        assert!(!severed_nodes.is_empty());
        {
            let body = selection
                .validated
                .plan
                .liquids
                .bodies
                .get_mut(&body_id)
                .expect("Mountain Range watershed body should remain present");
            for (position, _) in &severed_nodes {
                body.nodes.insert(
                    *position,
                    LiquidNode {
                        state: LiquidFlowState::Still,
                        downstream: None,
                    },
                );
            }
        }
        issues.clear();
        validate_mountain_watershed(settings, &selection.validated.plan, &mut issues);
        assert!(
            issues
                .iter()
                .any(|issue| issue.detail.contains("has no directed outflow")),
            "severing a tributary should fail the Mountain Range watershed contract: {}",
            format_issues(&issues)
        );
        {
            let body = selection
                .validated
                .plan
                .liquids
                .bodies
                .get_mut(&body_id)
                .expect("Mountain Range watershed body should remain present");
            for (position, node) in &severed_nodes {
                body.nodes.insert(*position, *node);
            }
        }

        let watershed = selection
            .validated
            .plan
            .liquids
            .bodies
            .get(&body_id)
            .expect("Mountain Range watershed body should remain present");
        let (fall_source, fall_target) = watershed
            .nodes
            .iter()
            .find_map(|(source, node)| {
                node.downstream
                    .filter(|target| source.level > target.level)
                    .map(|target| (*source, target))
            })
            .expect("Mountain Range tributaries should contain a descending flow edge");
        let original_target = watershed
            .nodes
            .get(&fall_target)
            .copied()
            .expect("descending flow target should remain in the watershed body");
        selection
            .validated
            .plan
            .liquids
            .bodies
            .get_mut(&body_id)
            .expect("Mountain Range watershed body should remain present")
            .nodes
            .insert(
                fall_target,
                LiquidNode {
                    state: LiquidFlowState::Current,
                    downstream: Some(fall_source),
                },
            );
        issues.clear();
        validate_mountain_watershed(settings, &selection.validated.plan, &mut issues);
        assert!(
            issues
                .iter()
                .any(|issue| issue.detail.contains("flows uphill")),
            "an uphill watershed edge should fail the Mountain Range contract: {}",
            format_issues(&issues)
        );
        assert!(
            issues
                .iter()
                .any(|issue| issue.detail.contains("contains a cycle")),
            "a watershed cycle should fail the Mountain Range contract: {}",
            format_issues(&issues)
        );
        selection
            .validated
            .plan
            .liquids
            .bodies
            .get_mut(&body_id)
            .expect("Mountain Range watershed body should remain present")
            .nodes
            .insert(fall_target, original_target);
    }

    #[test]
    #[ignore = "128-seed release corpus for the radius-77 Mountain Range world"]
    fn mountain_range_release_corpus_validates_128_seeds() {
        for seed in 0..128_u64 {
            let selection = generate_mountain_range(seed)
                .unwrap_or_else(|error| panic!("Mountain Range seed {seed} failed: {error}"));
            assert!(
                !selection.used_fallback,
                "Mountain Range seed {seed} unexpectedly used its fallback"
            );
        }
    }

    #[test]
    #[ignore = "10,000 seeds are a manual Mountain Range stress corpus"]
    fn mountain_range_stress_corpus_keeps_fallbacks_below_one_percent() {
        let mut fallbacks = 0_u32;
        for seed in 0..10_000_u64 {
            let selection = generate_mountain_range(seed)
                .unwrap_or_else(|error| panic!("Mountain Range seed {seed} failed: {error}"));
            fallbacks = fallbacks.saturating_add(u32::from(selection.used_fallback));
        }
        assert!(
            fallbacks < 100,
            "Mountain Range fallback rate must remain below 1%, got {fallbacks}/10000"
        );
    }

    #[test]
    #[ignore = "manual release-mode Mountain Range/Ring19 generation benchmark"]
    fn mountain_range_generation_p95_stays_within_two_and_a_half_times_ring19() {
        require_release_benchmark();

        const WARMUP_RUNS: usize = 1;
        const SAMPLE_COUNT: usize = 8;
        const RING19_REVIEW_SEED: u64 = 1_592_598_566;
        const MOUNTAIN_RANGE_REVIEW_SEED: u64 = 129_704_046;

        let ring19 = two_rings_map();
        let ring19_settings = v3_settings(ring19);
        let generate_ring19 = || {
            super::super::ring19::generate(
                ring19.grid_radius,
                ring19.level_height,
                ring19_settings,
                RING19_REVIEW_SEED,
                runtime_art_catalog(),
            )
            .expect("canonical Ring19 generation should succeed")
        };
        let generate_macro = || {
            generate_mountain_range(MOUNTAIN_RANGE_REVIEW_SEED)
                .expect("canonical Mountain Range generation should succeed")
        };

        for _ in 0..WARMUP_RUNS {
            std::hint::black_box(generate_ring19());
            std::hint::black_box(generate_macro());
        }

        let mut ring19_samples = Vec::with_capacity(SAMPLE_COUNT);
        let mut macro_samples = Vec::with_capacity(SAMPLE_COUNT);
        for sample in 0..SAMPLE_COUNT {
            if sample.is_multiple_of(2) {
                ring19_samples.push(measure_generation(generate_ring19));
                macro_samples.push(measure_generation(generate_macro));
            } else {
                macro_samples.push(measure_generation(generate_macro));
                ring19_samples.push(measure_generation(generate_ring19));
            }
        }
        ring19_samples.sort_unstable();
        macro_samples.sort_unstable();
        let ring19_p95 = sample_p95(&ring19_samples);
        let macro_p95 = sample_p95(&macro_samples);
        eprintln!(
            "Mountain Range generation release benchmark ({SAMPLE_COUNT} samples, \
             {WARMUP_RUNS} warm-up): Ring19 p95={ring19_p95:?}; \
             Mountain Range p95={macro_p95:?}"
        );
        assert!(
            macro_p95.as_nanos().saturating_mul(2) <= ring19_p95.as_nanos().saturating_mul(5),
            "Mountain Range p95 {macro_p95:?} exceeded 2.5x Ring19 p95 {ring19_p95:?}"
        );
    }

    #[test]
    #[ignore = "manual release-mode Crystal Mountain/Ring19 generation benchmark"]
    fn crystal_mountain_generation_benchmark_p95_stays_within_existing_macro_budget() {
        require_release_benchmark();

        const WARMUP_RUNS: usize = 1;
        const SAMPLE_COUNT: usize = 4;
        const REVIEW_SEED: u64 = 1_592_598_566;
        let ring19 = two_rings_map();
        let generate_ring19 = || {
            super::super::ring19::generate(
                ring19.grid_radius,
                ring19.level_height,
                v3_settings(ring19),
                REVIEW_SEED,
                runtime_art_catalog(),
            )
            .expect("canonical Ring19 generation should succeed")
        };
        let generate_crystal = || {
            generate_crystal_mountain(REVIEW_SEED)
                .expect("canonical Crystal Mountain generation should succeed")
        };

        for _ in 0..WARMUP_RUNS {
            std::hint::black_box(generate_ring19());
            std::hint::black_box(generate_crystal());
        }
        let mut ring19_samples = Vec::with_capacity(SAMPLE_COUNT);
        let mut crystal_samples = Vec::with_capacity(SAMPLE_COUNT);
        for sample in 0..SAMPLE_COUNT {
            if sample.is_multiple_of(2) {
                ring19_samples.push(measure_generation(generate_ring19));
                crystal_samples.push(measure_generation(generate_crystal));
            } else {
                crystal_samples.push(measure_generation(generate_crystal));
                ring19_samples.push(measure_generation(generate_ring19));
            }
        }
        ring19_samples.sort_unstable();
        crystal_samples.sort_unstable();
        let ring19_p95 = sample_p95(&ring19_samples);
        let crystal_p95 = sample_p95(&crystal_samples);
        eprintln!(
            "Crystal Mountain generation release benchmark ({SAMPLE_COUNT} samples, \
             {WARMUP_RUNS} warm-up): Ring19 p95={ring19_p95:?}; \
             Crystal Mountain p95={crystal_p95:?}"
        );
        assert!(
            crystal_p95.as_nanos().saturating_mul(2)
                <= ring19_p95.as_nanos().saturating_mul(5),
            "Crystal Mountain p95 {crystal_p95:?} exceeded the existing 2.5x Ring19 Macro budget ({ring19_p95:?})"
        );
    }

    fn generate_mountain_range(
        seed: u64,
    ) -> Result<ValidatedWorldSelection<MacroWorldMetrics>, V3GenerationError> {
        let map = mountain_range_map();
        generate(
            map.grid_radius,
            map.level_height,
            v3_settings(map),
            seed,
            runtime_art_catalog(),
        )
    }

    fn generate_crystal_mountain(
        seed: u64,
    ) -> Result<ValidatedWorldSelection<MacroWorldMetrics>, V3GenerationError> {
        let map = crystal_mountain_map();
        generate(
            map.grid_radius,
            map.level_height,
            v3_settings(map),
            seed,
            runtime_art_catalog(),
        )
    }

    fn highest_surface_levels(plan: &GeneratedWorldPlan) -> BTreeMap<HexCoord, Level> {
        let mut levels = BTreeMap::<HexCoord, Level>::new();
        for surface in plan.volume.surfaces.keys() {
            levels
                .entry(surface.coord)
                .and_modify(|level| *level = (*level).max(surface.level))
                .or_insert(surface.level);
        }
        levels
    }

    fn nearest_macro_cell(coord: HexCoord, centers: &[(HexCoord, HexCoord)]) -> HexCoord {
        centers
            .iter()
            .min_by_key(|(cell, center)| (coord.distance(*center), *cell))
            .map(|(cell, _)| *cell)
            .expect("radius-three Macro layout should have atomic cell centers")
    }

    fn ordered_coord_pair(first: HexCoord, second: HexCoord) -> (HexCoord, HexCoord) {
        if first < second {
            (first, second)
        } else {
            (second, first)
        }
    }

    fn named_instance_id(settings: &MacroLayoutSettings, name: &str) -> PatchId {
        settings
            .instances
            .iter()
            .position(|instance| instance.name == name)
            .map(|index| {
                PatchId(u32::try_from(index).expect("instance index should fit in a patch ID"))
            })
            .unwrap_or_else(|| panic!("Mountain Range should author {name}"))
    }

    fn mountain_range_map() -> &'static MapSettings {
        static SETTINGS: OnceLock<MapSettings> = OnceLock::new();
        SETTINGS.get_or_init(|| {
            ron::from_str(MOUNTAIN_RANGE_RON).expect("tracked Mountain Range settings should parse")
        })
    }

    fn crystal_mountain_map() -> &'static MapSettings {
        static SETTINGS: OnceLock<MapSettings> = OnceLock::new();
        SETTINGS.get_or_init(|| {
            ron::from_str(CRYSTAL_MOUNTAIN_RON)
                .expect("tracked Crystal Mountain settings should parse")
        })
    }

    fn ocean_archipelago_map() -> &'static MapSettings {
        static SETTINGS: OnceLock<MapSettings> = OnceLock::new();
        SETTINGS.get_or_init(|| {
            ron::from_str(OCEAN_ARCHIPELAGO_RON)
                .expect("tracked Ocean Archipelagoes settings should parse")
        })
    }

    fn rotated_crystal_mountain_map(turns: u8) -> MapSettings {
        let mut map = crystal_mountain_map().clone();
        let TerrainSettings::Procedural(ProceduralSettings::V3(settings)) = &mut map.terrain else {
            panic!("tracked Crystal Mountain settings should use procedural V3");
        };
        let V3LayoutSettings::Macro(layout) = &mut settings.layout else {
            panic!("tracked Crystal Mountain settings should use Macro");
        };
        for instance in &mut layout.instances {
            for cell in &mut instance.cells {
                *cell = rotate_cube(*cell, turns);
            }
            instance.rotation_turns = (instance.rotation_turns + turns % 6) % 6;
            instance.elevation.grade_axis = rotate_macro_axis(instance.elevation.grade_axis, turns);
        }
        for feature in &mut layout.spanning_features {
            let MacroSpanningFeatureSettings::Tunnel(tunnel) = feature;
            tunnel.boundary_terminal.side =
                rotate_boundary_side(tunnel.boundary_terminal.side, turns);
        }
        map
    }

    fn rotate_cube(mut cell: CubeCoord, turns: u8) -> CubeCoord {
        for _ in 0..turns % 6 {
            cell = CubeCoord {
                x: -cell.z,
                y: -cell.x,
                z: -cell.y,
            };
        }
        cell
    }

    fn rotate_tile_pos(position: TilePos, turns: u8) -> TilePos {
        let [x, y, z] = position.coord.to_cubic_array();
        let rotated = rotate_cube(CubeCoord { x, y, z }, turns);
        TilePos::new(
            HexCoord::new_cubic(rotated.x, rotated.y, rotated.z),
            position.level,
        )
    }

    fn rotate_macro_axis(mut axis: MacroAxisSettings, turns: u8) -> MacroAxisSettings {
        for _ in 0..turns % 6 {
            axis = match axis {
                MacroAxisSettings::East => MacroAxisSettings::NorthEast,
                MacroAxisSettings::NorthEast => MacroAxisSettings::NorthWest,
                MacroAxisSettings::NorthWest => MacroAxisSettings::West,
                MacroAxisSettings::West => MacroAxisSettings::SouthWest,
                MacroAxisSettings::SouthWest => MacroAxisSettings::SouthEast,
                MacroAxisSettings::SouthEast => MacroAxisSettings::East,
            };
        }
        axis
    }

    fn rotate_boundary_side(
        mut side: MacroBoundarySideSettings,
        turns: u8,
    ) -> MacroBoundarySideSettings {
        for _ in 0..turns % 6 {
            side = match side {
                MacroBoundarySideSettings::East => MacroBoundarySideSettings::NorthEast,
                MacroBoundarySideSettings::NorthEast => MacroBoundarySideSettings::NorthWest,
                MacroBoundarySideSettings::NorthWest => MacroBoundarySideSettings::West,
                MacroBoundarySideSettings::West => MacroBoundarySideSettings::SouthWest,
                MacroBoundarySideSettings::SouthWest => MacroBoundarySideSettings::SouthEast,
                MacroBoundarySideSettings::SouthEast => MacroBoundarySideSettings::East,
            };
        }
        side
    }

    fn two_rings_map() -> &'static MapSettings {
        static SETTINGS: OnceLock<MapSettings> = OnceLock::new();
        SETTINGS.get_or_init(|| {
            ron::from_str(TWO_RINGS_RON).expect("tracked Two Rings settings should parse")
        })
    }

    fn v3_settings(map: &MapSettings) -> &ProceduralV3Settings {
        let TerrainSettings::Procedural(ProceduralSettings::V3(settings)) = &map.terrain else {
            panic!("tracked benchmark world should select procedural V3");
        };
        settings
    }

    fn runtime_art_catalog() -> &'static RuntimeArtCatalog {
        super::super::vegetation::tests::runtime_art_catalog()
    }

    #[cfg(debug_assertions)]
    fn require_release_benchmark() {
        panic!(
            "run this manual gate with `cargo test --release -p hex_map \
             procedural_v3::macro_world::tests::mountain_range_generation_p95_stays_within_two_and_a_half_times_ring19 \
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
}
