//! Procedural geometry V2 foundation.
//!
//! V2 is intentionally isolated from the frozen V1 implementation. Recipes construct
//! a validated [`TerrainVolumePlan`] and only then materialize voxels; unsupported or
//! unfinished recipes return an error rather than publishing an empty world.

mod caves;
mod hills;
mod mountains;
mod recipe;
mod seed;
mod sky;
mod volume;

use std::fmt;
use std::time::Instant;

use hex_core::{InteriorRegions, MapAnchors, MapViewHint, SpecialMovementRegions, SubstanceId};

use crate::procedural::GenerationReport;
use crate::settings::{
    ProceduralV2Settings, V2EnvironmentSettings, V2HillsSettings, V2RecipeSettings,
};
use crate::terrain::TerrainPalette;
use crate::voxel::VoxelMap;

/// Failure to construct or validate one V2 map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum V2GenerationError {
    /// The sequential recipe PR has not supplied this implementation yet.
    RecipeUnavailable(&'static str),
    /// Recipe-independent volume invariants failed.
    InvalidVolume(Vec<String>),
    /// A semantic solid/fill role resolved to the wrong substance behavior.
    MaterialContract(String),
    /// Recipe inputs or imported compatibility metadata violated the V2 contract.
    RecipeContract(String),
    /// Candidate construction encountered an error that cannot be treated as rejection.
    FatalCandidateConstruction { candidate: u8, source: Box<Self> },
    /// A bounded repair encountered an error that must stop the complete generation run.
    FatalCandidateRepair {
        candidate: u8,
        round: u8,
        source: Box<Self>,
    },
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
            Self::RecipeContract(reason) => {
                write!(formatter, "procedural V2 recipe contract failed: {reason}")
            }
            Self::FatalCandidateConstruction { candidate, source } => {
                write!(
                    formatter,
                    "procedural V2 candidate {candidate} construction failed fatally: {source}"
                )
            }
            Self::FatalCandidateRepair {
                candidate,
                round,
                source,
            } => {
                write!(
                    formatter,
                    "procedural V2 candidate {candidate} repair round {round} failed fatally: \
                     {source}"
                )
            }
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

impl std::error::Error for V2GenerationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::FatalCandidateConstruction { source, .. }
            | Self::FatalCandidateRepair { source, .. } => Some(source),
            Self::RecipeUnavailable(_)
            | Self::InvalidVolume(_)
            | Self::MaterialContract(_)
            | Self::RecipeContract(_)
            | Self::InvalidFallback(_) => None,
        }
    }
}

/// Complete generated resources published by the runtime after hard validation.
#[derive(Debug)]
pub(crate) struct ProceduralBuild {
    pub(crate) map: VoxelMap,
    pub(crate) anchors: MapAnchors,
    pub(crate) special_regions: SpecialMovementRegions,
    pub(crate) interiors: InteriorRegions,
    pub(crate) view_hint: MapViewHint,
    pub(crate) report: GenerationReport,
}

/// Dispatches a V2 recipe and turns its typed selection into runtime resources.
pub(crate) fn build(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV2Settings,
    seed: u64,
    palette: &TerrainPalette,
    is_solid: &dyn Fn(SubstanceId) -> bool,
) -> Result<ProceduralBuild, V2GenerationError> {
    let started = Instant::now();
    match &settings.recipe {
        V2RecipeSettings::Hills(_) => finish_build(
            hills::build(grid_radius, level_height, settings, seed, palette, is_solid)?,
            grid_radius,
            settings,
            seed,
            started,
        ),
        V2RecipeSettings::LayeredSkyIslands(_) => finish_build(
            sky::build(grid_radius, level_height, settings, seed, palette, is_solid)?,
            grid_radius,
            settings,
            seed,
            started,
        ),
        V2RecipeSettings::Mountains(_) => finish_build(
            mountains::build(grid_radius, level_height, settings, seed, palette, is_solid)?,
            grid_radius,
            settings,
            seed,
            started,
        ),
        V2RecipeSettings::Caves(_) => finish_build(
            caves::build(grid_radius, level_height, settings, seed, palette, is_solid)?,
            grid_radius,
            settings,
            seed,
            started,
        ),
    }
}

fn finish_build<M, V>(
    selection: recipe::MaterializedSelection<M, V>,
    grid_radius: u32,
    settings: &ProceduralV2Settings,
    seed: u64,
    started: Instant,
) -> Result<ProceduralBuild, V2GenerationError>
where
    V: recipe::ReportMetrics,
{
    let recipe::MaterializedSelection {
        map,
        anchors,
        special_regions,
        interiors,
        view_hint,
        metadata,
        metrics,
        selected_candidate,
        candidates_evaluated,
        valid_candidates,
        repair_actions,
        used_fallback,
        notes,
        map_fingerprint,
    } = selection;
    drop(metadata);
    let repair_rounds = u8::try_from(repair_actions.len()).unwrap_or(u8::MAX);
    let elapsed_micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    let report = GenerationReport {
        generator_version: 2,
        seed,
        selected_candidate,
        candidates_evaluated,
        valid_candidates,
        repair_rounds,
        repair_actions,
        used_fallback,
        settings_fingerprint: settings_fingerprint(grid_radius, settings),
        semantic_plan_fingerprint: None,
        map_fingerprint,
        metrics: metrics.tactical(),
        recipe_metrics: None,
        elapsed_micros,
        notes,
    };

    Ok(ProceduralBuild {
        map,
        anchors,
        special_regions,
        interiors,
        view_hint,
        report,
    })
}

/// Stable hash of every V2 setting that can affect generated output.
///
/// This deliberately uses a V2-specific domain and includes the version number.
/// Equivalent Hills parameters therefore remain output-compatible with V1 without
/// making their settings/report identity ambiguous.
pub(crate) fn settings_fingerprint(grid_radius: u32, settings: &ProceduralV2Settings) -> u64 {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"procedural-v2-settings");
    bytes.extend_from_slice(&2_u32.to_le_bytes());
    bytes.extend_from_slice(&grid_radius.to_le_bytes());
    bytes.push(match settings.environment {
        V2EnvironmentSettings::TemperateGrassland => 0,
        V2EnvironmentSettings::Frozen => 1,
        V2EnvironmentSettings::Volcanic => 2,
        V2EnvironmentSettings::Rocky => 3,
    });
    match &settings.recipe {
        V2RecipeSettings::Hills(hills) => {
            bytes.push(0);
            append_hills_settings(&mut bytes, hills);
        }
        V2RecipeSettings::LayeredSkyIslands(islands) => {
            bytes.push(1);
            append_hills_settings(&mut bytes, &islands.ground);
            bytes.extend_from_slice(&islands.min_clearance.to_le_bytes());
            bytes.push(islands.upper_coverage_percent);
        }
        V2RecipeSettings::Mountains(mountains) => {
            bytes.push(2);
            bytes.extend_from_slice(&mountains.base_level.to_le_bytes());
            bytes.extend_from_slice(&mountains.relief.to_le_bytes());
            bytes.push(mountains.peak_count);
        }
        V2RecipeSettings::Caves(caves) => {
            bytes.push(3);
            bytes.extend_from_slice(&caves.surface_level.to_le_bytes());
            bytes.extend_from_slice(&caves.cave_floor_level.to_le_bytes());
            bytes.push(caves.chamber_count);
        }
    }
    seed::fingerprint(&bytes)
}

fn append_hills_settings(bytes: &mut Vec<u8>, hills: &V2HillsSettings) {
    bytes.extend_from_slice(&hills.valley_level.to_le_bytes());
    bytes.extend_from_slice(&hills.max_relief.to_le_bytes());
    bytes.push(hills.hills_per_bank);
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::settings::{
        CavesSettings, LayeredSkyIslandsSettings, MountainsSettings, ProceduralV2Settings,
        V2EnvironmentSettings, V2HillsSettings, V2RecipeSettings,
    };

    type SettingsFactory = fn() -> ProceduralV2Settings;
    type SettingsMutation = fn(&mut ProceduralV2Settings);

    fn hills_settings() -> ProceduralV2Settings {
        ProceduralV2Settings {
            environment: V2EnvironmentSettings::TemperateGrassland,
            recipe: V2RecipeSettings::Hills(V2HillsSettings {
                valley_level: 15,
                max_relief: 8,
                hills_per_bank: 3,
            }),
        }
    }

    fn layered_sky_islands_settings() -> ProceduralV2Settings {
        ProceduralV2Settings {
            environment: V2EnvironmentSettings::TemperateGrassland,
            recipe: V2RecipeSettings::LayeredSkyIslands(LayeredSkyIslandsSettings {
                ground: V2HillsSettings {
                    valley_level: 15,
                    max_relief: 8,
                    hills_per_bank: 3,
                },
                min_clearance: 8,
                upper_coverage_percent: 20,
            }),
        }
    }

    fn mountains_settings() -> ProceduralV2Settings {
        ProceduralV2Settings {
            environment: V2EnvironmentSettings::Frozen,
            recipe: V2RecipeSettings::Mountains(MountainsSettings {
                base_level: 15,
                relief: 15,
                peak_count: 4,
            }),
        }
    }

    fn caves_settings() -> ProceduralV2Settings {
        ProceduralV2Settings {
            environment: V2EnvironmentSettings::Rocky,
            recipe: V2RecipeSettings::Caves(CavesSettings {
                surface_level: 15,
                cave_floor_level: 7,
                chamber_count: 7,
            }),
        }
    }

    #[test]
    fn v2_settings_fingerprint_is_deterministic_and_version_distinct() {
        let settings = hills_settings();
        let fingerprint = settings_fingerprint(12, &settings);

        assert_eq!(
            fingerprint, 13_620_952_131_205_421_838,
            "update only with an explicit V2 generator-version decision"
        );
        assert_eq!(settings_fingerprint(12, &settings), fingerprint);
        assert_ne!(
            fingerprint, 4_508_295_216_895_027_881,
            "equivalent V1 and V2 settings need distinct report identities"
        );
        assert_ne!(settings_fingerprint(20, &settings), fingerprint);
    }

    #[test]
    fn v2_settings_fingerprint_covers_environment_and_recipe_identity() {
        let mut environment_fingerprints = BTreeSet::new();
        for environment in [
            V2EnvironmentSettings::TemperateGrassland,
            V2EnvironmentSettings::Frozen,
            V2EnvironmentSettings::Volcanic,
            V2EnvironmentSettings::Rocky,
        ] {
            let mut settings = hills_settings();
            settings.environment = environment;
            assert!(
                environment_fingerprints.insert(settings_fingerprint(12, &settings)),
                "environment {environment:?} did not change the fingerprint"
            );
        }

        // Equal raw payloads make the first three variants specifically exercise
        // their recipe discriminants rather than merely differing parameter bytes.
        let recipes = [
            V2RecipeSettings::Hills(V2HillsSettings {
                valley_level: 15,
                max_relief: 8,
                hills_per_bank: 3,
            }),
            V2RecipeSettings::Mountains(MountainsSettings {
                base_level: 15,
                relief: 8,
                peak_count: 3,
            }),
            V2RecipeSettings::Caves(CavesSettings {
                surface_level: 15,
                cave_floor_level: 8,
                chamber_count: 3,
            }),
            layered_sky_islands_settings().recipe,
        ];
        let mut recipe_fingerprints = BTreeSet::new();
        for recipe in recipes {
            let settings = ProceduralV2Settings {
                environment: V2EnvironmentSettings::TemperateGrassland,
                recipe,
            };
            assert!(
                recipe_fingerprints.insert(settings_fingerprint(12, &settings)),
                "recipe {:?} did not change the fingerprint",
                settings.recipe
            );
        }
    }

    #[test]
    fn v2_settings_fingerprint_covers_every_recipe_field() {
        let cases: &[(&str, SettingsFactory, SettingsMutation)] = &[
            ("Hills.valley_level", hills_settings, |settings| {
                let V2RecipeSettings::Hills(hills) = &mut settings.recipe else {
                    unreachable!("fixture must be Hills");
                };
                hills.valley_level = 14;
            }),
            ("Hills.max_relief", hills_settings, |settings| {
                let V2RecipeSettings::Hills(hills) = &mut settings.recipe else {
                    unreachable!("fixture must be Hills");
                };
                hills.max_relief = 7;
            }),
            ("Hills.hills_per_bank", hills_settings, |settings| {
                let V2RecipeSettings::Hills(hills) = &mut settings.recipe else {
                    unreachable!("fixture must be Hills");
                };
                hills.hills_per_bank = 4;
            }),
            (
                "LayeredSkyIslands.ground.valley_level",
                layered_sky_islands_settings,
                |settings| {
                    let V2RecipeSettings::LayeredSkyIslands(islands) = &mut settings.recipe else {
                        unreachable!("fixture must be LayeredSkyIslands");
                    };
                    islands.ground.valley_level = 14;
                },
            ),
            (
                "LayeredSkyIslands.ground.max_relief",
                layered_sky_islands_settings,
                |settings| {
                    let V2RecipeSettings::LayeredSkyIslands(islands) = &mut settings.recipe else {
                        unreachable!("fixture must be LayeredSkyIslands");
                    };
                    islands.ground.max_relief = 7;
                },
            ),
            (
                "LayeredSkyIslands.ground.hills_per_bank",
                layered_sky_islands_settings,
                |settings| {
                    let V2RecipeSettings::LayeredSkyIslands(islands) = &mut settings.recipe else {
                        unreachable!("fixture must be LayeredSkyIslands");
                    };
                    islands.ground.hills_per_bank = 4;
                },
            ),
            (
                "LayeredSkyIslands.min_clearance",
                layered_sky_islands_settings,
                |settings| {
                    let V2RecipeSettings::LayeredSkyIslands(islands) = &mut settings.recipe else {
                        unreachable!("fixture must be LayeredSkyIslands");
                    };
                    islands.min_clearance = 9;
                },
            ),
            (
                "LayeredSkyIslands.upper_coverage_percent",
                layered_sky_islands_settings,
                |settings| {
                    let V2RecipeSettings::LayeredSkyIslands(islands) = &mut settings.recipe else {
                        unreachable!("fixture must be LayeredSkyIslands");
                    };
                    islands.upper_coverage_percent = 21;
                },
            ),
            ("Mountains.base_level", mountains_settings, |settings| {
                let V2RecipeSettings::Mountains(mountains) = &mut settings.recipe else {
                    unreachable!("fixture must be Mountains");
                };
                mountains.base_level = 16;
            }),
            ("Mountains.relief", mountains_settings, |settings| {
                let V2RecipeSettings::Mountains(mountains) = &mut settings.recipe else {
                    unreachable!("fixture must be Mountains");
                };
                mountains.relief = 14;
            }),
            ("Mountains.peak_count", mountains_settings, |settings| {
                let V2RecipeSettings::Mountains(mountains) = &mut settings.recipe else {
                    unreachable!("fixture must be Mountains");
                };
                mountains.peak_count = 5;
            }),
            ("Caves.surface_level", caves_settings, |settings| {
                let V2RecipeSettings::Caves(caves) = &mut settings.recipe else {
                    unreachable!("fixture must be Caves");
                };
                caves.surface_level = 16;
            }),
            ("Caves.cave_floor_level", caves_settings, |settings| {
                let V2RecipeSettings::Caves(caves) = &mut settings.recipe else {
                    unreachable!("fixture must be Caves");
                };
                caves.cave_floor_level = 8;
            }),
            ("Caves.chamber_count", caves_settings, |settings| {
                let V2RecipeSettings::Caves(caves) = &mut settings.recipe else {
                    unreachable!("fixture must be Caves");
                };
                caves.chamber_count = 8;
            }),
        ];

        for (field, factory, mutate) in cases {
            let baseline = factory();
            let mut changed = baseline.clone();
            mutate(&mut changed);
            assert_ne!(
                settings_fingerprint(12, &baseline),
                settings_fingerprint(12, &changed),
                "changing {field} must change the fingerprint"
            );
        }
    }
}
