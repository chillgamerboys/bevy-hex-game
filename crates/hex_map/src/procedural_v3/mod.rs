//! Procedural world generation V3 foundation.
//!
//! V3 remains isolated from the temporary V1/V2 implementations. Settings dispatch
//! may select it before every recipe is available, but unsupported recipes fail
//! setup explicitly rather than publishing an empty or partially validated world.

use std::fmt;

use crate::settings::{ProceduralV3Settings, V3LayoutSettings, V3RecipeSettings};
use world::WorldValidationIssue;

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the encoder is consumed as V3 plan and materialization layers land"
    )
)]
mod fingerprint;
#[expect(
    dead_code,
    reason = "resolved layouts are consumed by sequential V3 recipe implementations"
)]
mod layout;
pub(crate) use layout::HexSide;
#[expect(
    dead_code,
    reason = "liquid topology is consumed by the sequential V3 Waterfall recipe"
)]
mod liquid;
pub(crate) use liquid::LiquidFlowState;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "materialization is consumed once the first V3 recipe is runnable"
    )
)]
mod materialize;
pub(crate) use materialize::MapPresentationProjection;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the seed API is consumed by sequential V3 recipe implementations"
    )
)]
mod seed;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the whole-world runner is consumed by sequential V3 recipes"
    )
)]
mod selection;
#[expect(
    dead_code,
    reason = "the volume foundation is consumed by sequential V3 recipe implementations"
)]
mod volume;
pub(crate) use volume::FillMaterialRole;
#[expect(
    dead_code,
    reason = "the complete semantic plan is consumed by the V3 selection runner"
)]
mod world;

/// Failure to construct or validate one V3 world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum V3GenerationError {
    /// The sequential recipe PR has not supplied this implementation yet.
    RecipeUnavailable(&'static str),
    /// A recipe violated an invariant required by the common candidate runner.
    RecipeContract(String),
    /// Candidate construction encountered a failure that is not a normal rejection.
    FatalCandidateConstruction { candidate: u8, source: Box<Self> },
    /// Candidate repair encountered a failure that is not a normal rejection.
    FatalCandidateRepair {
        candidate: u8,
        round: u8,
        source: Box<Self>,
    },
    /// The separately authored canonical fallback could not be constructed.
    FatalFallbackConstruction(Box<Self>),
    /// The canonical fallback failed common or recipe-specific validation.
    InvalidFallback(Vec<WorldValidationIssue>),
    /// A deterministic fingerprint could not encode a semantic value.
    Fingerprint(String),
}

impl fmt::Display for V3GenerationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RecipeUnavailable(recipe) => {
                write!(formatter, "procedural V3 recipe {recipe} is not available")
            }
            Self::RecipeContract(reason) => {
                write!(formatter, "procedural V3 recipe contract failed: {reason}")
            }
            Self::FatalCandidateConstruction { candidate, source } => write!(
                formatter,
                "procedural V3 candidate {candidate} construction failed fatally: {source}"
            ),
            Self::FatalCandidateRepair {
                candidate,
                round,
                source,
            } => write!(
                formatter,
                "procedural V3 candidate {candidate} repair round {round} failed fatally: \
                 {source}"
            ),
            Self::FatalFallbackConstruction(source) => {
                write!(
                    formatter,
                    "procedural V3 canonical fallback failed: {source}"
                )
            }
            Self::InvalidFallback(issues) => write!(
                formatter,
                "invalid procedural V3 canonical fallback: {}",
                issues
                    .iter()
                    .map(|issue| format!("{:?}: {}", issue.code, issue.detail))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            Self::Fingerprint(reason) => {
                write!(formatter, "procedural V3 fingerprint failed: {reason}")
            }
        }
    }
}

impl std::error::Error for V3GenerationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::FatalCandidateConstruction { source, .. }
            | Self::FatalCandidateRepair { source, .. }
            | Self::FatalFallbackConstruction(source) => Some(source),
            Self::RecipeUnavailable(_)
            | Self::RecipeContract(_)
            | Self::InvalidFallback(_)
            | Self::Fingerprint(_) => None,
        }
    }
}

/// Controlled dispatch point used until sequential V3 recipe PRs land.
///
/// Returning an explicit error is part of the foundation contract: construction
/// never fabricates an empty semantic plan for an unsupported layout or recipe.
pub(crate) fn ensure_recipe_available(
    settings: &ProceduralV3Settings,
) -> Result<(), V3GenerationError> {
    let name = match &settings.layout {
        V3LayoutSettings::Single(patch) => recipe_name(&patch.recipe),
        V3LayoutSettings::Ring7(_) => "Ring7",
    };
    Err(V3GenerationError::RecipeUnavailable(name))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{
        PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings, PatchSpec,
        V3EnvironmentSettings, V3HillsSettings,
    };

    fn world_edges() -> PatchEdgesSettings {
        PatchEdgesSettings {
            east: PatchEdgeContractSettings::WorldBoundary,
            south_east: PatchEdgeContractSettings::WorldBoundary,
            south_west: PatchEdgeContractSettings::WorldBoundary,
            west: PatchEdgeContractSettings::WorldBoundary,
            north_west: PatchEdgeContractSettings::WorldBoundary,
            north_east: PatchEdgeContractSettings::WorldBoundary,
        }
    }

    #[test]
    fn unfinished_recipe_fails_instead_of_fabricating_a_plan() {
        let settings = ProceduralV3Settings {
            layout: V3LayoutSettings::Single(PatchSpec {
                environment: V3EnvironmentSettings::TemperateGrassland,
                recipe: V3RecipeSettings::Hills(V3HillsSettings {
                    valley_level: 15,
                    max_relief: 8,
                    hills_per_bank: 3,
                }),
                overlays: Vec::new(),
                mask: PatchMaskSettings::WholeWorld,
                edges: world_edges(),
            }),
        };

        assert_eq!(
            ensure_recipe_available(&settings),
            Err(V3GenerationError::RecipeUnavailable("Hills"))
        );
    }
}
