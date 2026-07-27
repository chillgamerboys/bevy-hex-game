//! Procedural geometry V2 foundation.
//!
//! V2 is intentionally isolated from the frozen V1 implementation. Recipes construct
//! a validated [`TerrainVolumePlan`] and only then materialize voxels; unsupported or
//! unfinished recipes return an error rather than publishing an empty world.

#[expect(
    dead_code,
    reason = "the foundation is consumed by the sequential V2 recipe PRs"
)]
mod recipe;
#[expect(
    dead_code,
    reason = "the foundation is consumed by the sequential V2 recipe PRs"
)]
mod seed;
#[expect(
    dead_code,
    reason = "the foundation is consumed by the sequential V2 recipe PRs"
)]
mod volume;

use std::fmt;

use crate::settings::{ProceduralV2Settings, V2RecipeSettings};

/// Failure to construct or validate one V2 map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum V2GenerationError {
    /// The sequential recipe PR has not supplied this implementation yet.
    RecipeUnavailable(&'static str),
    /// Recipe-independent volume invariants failed.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "constructed once a V2 recipe reaches common volume validation"
        )
    )]
    InvalidVolume(Vec<String>),
    /// A semantic solid/fill role resolved to the wrong substance behavior.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "constructed once a V2 recipe reaches semantic voxelization"
        )
    )]
    MaterialContract(String),
    /// A canonical fallback failed the same hard contracts as ordinary candidates.
    InvalidFallback(Vec<String>),
}

impl fmt::Display for V2GenerationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RecipeUnavailable(recipe) => {
                write!(formatter, "procedural V2 recipe {recipe} is not available")
            }
            Self::InvalidVolume(issues) => {
                write!(
                    formatter,
                    "invalid procedural V2 volume: {}",
                    issues.join("; ")
                )
            }
            Self::MaterialContract(reason) => formatter.write_str(reason),
            Self::InvalidFallback(issues) => {
                write!(
                    formatter,
                    "invalid procedural V2 canonical fallback: {}",
                    issues.join("; ")
                )
            }
        }
    }
}

impl std::error::Error for V2GenerationError {}

/// Controlled dispatch point used until each sequential recipe PR lands.
///
/// Returning an explicit error is part of the foundation contract: construction must
/// never fabricate an empty semantic plan for an unsupported combination.
pub(crate) fn ensure_recipe_available(
    settings: &ProceduralV2Settings,
) -> Result<(), V2GenerationError> {
    let name = match settings.recipe {
        V2RecipeSettings::Hills(_) => "Hills",
        V2RecipeSettings::LayeredSkyIslands(_) => "LayeredSkyIslands",
        V2RecipeSettings::Mountains(_) => "Mountains",
        V2RecipeSettings::Caves(_) => "Caves",
    };
    Err(V2GenerationError::RecipeUnavailable(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{
        ProceduralV2Settings, V2EnvironmentSettings, V2HillsSettings, V2RecipeSettings,
    };

    #[test]
    fn unfinished_recipe_returns_an_error_instead_of_an_empty_plan() {
        let settings = ProceduralV2Settings {
            environment: V2EnvironmentSettings::TemperateGrassland,
            recipe: V2RecipeSettings::Hills(V2HillsSettings {
                valley_level: 15,
                max_relief: 8,
                hills_per_bank: 3,
            }),
        };

        assert_eq!(
            ensure_recipe_available(&settings),
            Err(V2GenerationError::RecipeUnavailable("Hills"))
        );
    }
}
