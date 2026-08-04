//! Shared construction and validation for recipes admitted by composite layouts.
//!
//! Layout-specific runners retain authority over their roster and overlays. This
//! module only prevents Ring7 and Ring19 from growing separate recipe dispatchers.

use hex_assets::RuntimeArtCatalog;
use hex_core::HexCoord;

use super::composition::GeneratedPatchPlan;
use super::patch::{PatchBuildMode, PatchRecipeContext};
use super::selection::WorldValidation;
use super::vegetation::CaveVegetationSet;
use super::world::{GeneratedWorldPlan, WorldIssueCode, WorldValidationIssue};
use super::{caves, deep_forest, forest, fort, hills, mountains, prairie, sky, volcano, waterfall};
use crate::settings::{V3EnvironmentSettings, V3RecipeSettings};

pub(crate) fn construct_fragment(
    patch: PatchRecipeContext<'_>,
    environment: V3EnvironmentSettings,
    recipe: &V3RecipeSettings,
    level_height: f32,
    mode: PatchBuildMode,
    art_catalog: &RuntimeArtCatalog,
    cave_vegetation: &CaveVegetationSet,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    match recipe {
        V3RecipeSettings::Hills(settings) => hills::construct_patch_with_catalog(
            patch,
            settings,
            environment,
            level_height,
            mode,
            art_catalog,
        ),
        V3RecipeSettings::Mountains(settings) => mountains::construct_patch_with_catalog(
            patch,
            settings,
            level_height,
            mode,
            art_catalog,
        ),
        V3RecipeSettings::Waterfall(settings) => waterfall::construct_patch_with_catalog(
            patch,
            settings,
            environment,
            level_height,
            mode,
            art_catalog,
        ),
        V3RecipeSettings::Forest(settings) => forest::construct_patch(
            patch,
            settings,
            environment,
            level_height,
            mode,
            art_catalog,
        ),
        V3RecipeSettings::Fort(settings) => {
            fort::construct_patch(patch, settings, level_height, mode)
        }
        V3RecipeSettings::Caves(settings) => {
            caves::construct_patch(patch, settings, level_height, mode, cave_vegetation)
        }
        V3RecipeSettings::SkyIslands(settings) => sky::construct_patch_with_catalog(
            patch,
            settings,
            environment,
            level_height,
            mode,
            art_catalog,
        ),
        V3RecipeSettings::Volcano(settings) => {
            volcano::construct_patch(patch, settings, level_height, mode)
        }
        V3RecipeSettings::DeepForest(settings) => deep_forest::construct_patch(
            patch,
            settings,
            environment,
            level_height,
            mode,
            art_catalog,
        ),
        V3RecipeSettings::Prairie(settings) => prairie::construct_patch(
            patch,
            settings,
            environment,
            level_height,
            mode,
            art_catalog,
        ),
        V3RecipeSettings::ShallowSea(_)
        | V3RecipeSettings::Beach(_)
        | V3RecipeSettings::Shore(_)
        | V3RecipeSettings::DeepMountain(_) => Err(vec![composite_issue(
            "Macro-only recipes are constructed by the authored Macro runner",
        )]),
    }
}

pub(crate) fn validate_fragment(
    patch: PatchRecipeContext<'_>,
    environment: V3EnvironmentSettings,
    recipe: &V3RecipeSettings,
    fragment: &GeneratedPatchPlan,
    art_catalog: &RuntimeArtCatalog,
    cave_vegetation: &CaveVegetationSet,
) -> Result<(), Vec<WorldValidationIssue>> {
    let common = fragment.validate_against(patch.layout());
    if !common.is_empty() {
        return Err(common
            .into_iter()
            .map(|issue| {
                composite_issue(format!(
                    "patch {} common validation {:?}: {}",
                    issue.patch.0, issue.code, issue.detail
                ))
            })
            .collect());
    }

    let validation = match recipe {
        V3RecipeSettings::Waterfall(_) => {
            discard_metrics(waterfall::validate_patch(patch, fragment, art_catalog))
        }
        V3RecipeSettings::Forest(_) => {
            discard_metrics(forest::validate_patch(patch, fragment, art_catalog))
        }
        V3RecipeSettings::Hills(settings) => discard_metrics(hills::validate_patch(
            patch,
            fragment,
            settings,
            environment,
            art_catalog,
        )),
        V3RecipeSettings::Mountains(settings) => discard_metrics(mountains::validate_patch(
            patch,
            fragment,
            settings,
            art_catalog,
        )),
        V3RecipeSettings::Fort(_) => {
            let ground_level = fort::patch_ground_level(patch.layout(), patch.id);
            validate_canonical(patch, fragment, move |plan| {
                fort::validate_fort_at_ground(plan, ground_level)
            })
        }
        V3RecipeSettings::Caves(settings) => discard_metrics(
            caves::validate_caves_with_surface_sink(patch, fragment, settings, cave_vegetation),
        ),
        V3RecipeSettings::SkyIslands(settings) => discard_metrics(sky::validate_patch(
            patch,
            fragment,
            settings,
            environment,
            art_catalog,
        )),
        V3RecipeSettings::Volcano(settings) => {
            discard_metrics(volcano::validate_patch(patch, fragment, settings))
        }
        V3RecipeSettings::DeepForest(settings) => discard_metrics(deep_forest::validate_patch(
            patch,
            settings,
            fragment,
            art_catalog,
        )),
        V3RecipeSettings::Prairie(settings) => discard_metrics(prairie::validate_patch(
            patch,
            settings,
            fragment,
            art_catalog,
        )),
        V3RecipeSettings::ShallowSea(_)
        | V3RecipeSettings::Beach(_)
        | V3RecipeSettings::Shore(_)
        | V3RecipeSettings::DeepMountain(_) => WorldValidation::Invalid(vec![composite_issue(
            "Macro-only recipes are validated by the authored Macro runner",
        )]),
    };
    match validation {
        WorldValidation::Valid(()) => Ok(()),
        WorldValidation::Invalid(issues) => Err(issues),
    }
}

fn validate_canonical<M>(
    patch: PatchRecipeContext<'_>,
    fragment: &GeneratedPatchPlan,
    validate: impl FnOnce(&GeneratedWorldPlan) -> WorldValidation<M>,
) -> WorldValidation<()> {
    let frame = match patch.local_frame() {
        Ok(frame) => frame,
        Err(error) => {
            return WorldValidation::Invalid(vec![composite_issue(format!(
                "patch {} validation frame failed: {error}",
                patch.id.0
            ))]);
        }
    };
    let frame_center = frame.center();
    let mut plan = match frame.canonical_local_world(fragment) {
        Ok(plan) => plan,
        Err(error) => {
            return WorldValidation::Invalid(vec![composite_issue(format!(
                "patch {} validation projection around {frame_center:?} failed: {error}",
                patch.id.0,
            ))]);
        }
    };
    plan.layout.grid_radius = plan
        .layout
        .footprint
        .iter()
        .map(|coord| HexCoord::ORIGIN.distance(*coord))
        .max()
        .unwrap_or_default();
    discard_metrics(validate(&plan))
}

fn discard_metrics<M>(validation: WorldValidation<M>) -> WorldValidation<()> {
    match validation {
        WorldValidation::Valid(_) => WorldValidation::Valid(()),
        WorldValidation::Invalid(issues) => WorldValidation::Invalid(issues),
    }
}

fn composite_issue(detail: impl Into<String>) -> WorldValidationIssue {
    WorldValidationIssue::new(WorldIssueCode::Recipe("composite"), detail)
}
