//! The scenarios offered on the title screen.
//!
//! A scenario is a **world plus the units standing on it**: pick one and you get that
//! terrain with those pieces, without editing a file or restarting.
//!
//! # Why the world is a string
//!
//! Terrain settings live in `hex_map`, which depends on *this* crate — so naming
//! `MapSettings` here would invert the crate graph and fail to compile. A scenario
//! therefore names its world by **asset path** and stays deliberately ignorant of what
//! that file parses into. `hex_game`, the one crate that can see both, turns the string
//! into a handle.
//!
//! That is the same trick the settings loader already uses: the generic machinery lives
//! here and gets instantiated at a concrete type from wherever that type is defined.
//!
//! The cost is that a typo in a path is not a compile error. It is caught by tests in
//! `hex_game` that open every world, lighting and encounter file a scenario names.

use bevy::prelude::*;
use serde::{de::Error as _, Deserialize, Deserializer};

/// `assets/config/scenarios.ron` — everything the title screen offers.
///
/// Order is the order they appear, so it is a designer's decision rather than an
/// accident of hashing.
#[derive(Asset, Resource, Reflect, Debug, Clone, Deserialize)]
#[reflect(Resource)]
pub struct ScenarioLibrary {
    /// The scenarios, in the order they are listed.
    pub scenarios: Vec<Scenario>,
}

/// One playable setup: a world, and where the units start on it.
#[derive(Reflect, Debug, Clone)]
pub struct Scenario {
    /// What the title screen calls it.
    pub name: String,
    /// Which framed title-screen column owns this scenario.
    pub category: ScenarioCategory,
    /// One line under the name, saying what is interesting about it.
    pub blurb: String,
    /// Asset path of the world file, relative to `assets/`.
    ///
    /// A path rather than the settings themselves, because this crate cannot name a
    /// terrain type — see the module documentation.
    pub world: String,
    /// Asset path of the lighting file: sun, sky, clouds and fog.
    ///
    /// Optional, because most scenarios want the shipped look and requiring it would
    /// mean every new entry copying a path it will never change. Called `lighting`
    /// rather than `sky` because it also decides the sun's angle and colour, and so
    /// which way the shadows fall.
    pub lighting: String,
    /// Reproducible terrain seed for a generated world.
    ///
    /// Authored scenarios omit this. The title screen can replace a configured seed
    /// for the current process, but never writes that replacement back to this asset.
    pub generation_seed: Option<u64>,
    /// Optional time of day at which this scenario starts, in `[0, 24)`.
    ///
    /// Only cyclic lighting profiles accept an override. That cross-asset contract is
    /// checked after both this scenario and its lighting file have loaded.
    pub starting_time_hours: Option<f32>,
    /// Asset path of the encounter file: the roster standing on this world.
    ///
    /// A path for the same reason `world` is one — a scenario is a world, a sky and an
    /// encounter, each authored on its own and reusable by the next scenario. Six
    /// generated maps share one anchored skirmish today.
    ///
    /// Not optional: a scenario with no encounter has nothing to play.
    pub encounter: String,
}

/// The three title-screen lanes a scenario can inhabit.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
pub enum ScenarioCategory {
    /// Worlds whose terrain or traversal is the main attraction.
    Map,
    /// Setups authored to exercise the combat loop.
    Combat,
    /// Focused mechanics showcases and rules probes.
    Demo,
}

/// Deserialization mirror used so a missing category can name the scenario it broke.
///
/// A derived `Deserialize` on [`Scenario`] can only report `missing field category`.
/// That is needlessly hostile in a nine-entry content file: the designer then has to
/// count parentheses to discover which entry failed. Reading the other fields first
/// lets the error identify the exact scenario while keeping category genuinely required.
#[derive(Deserialize)]
struct ScenarioFields {
    name: String,
    #[serde(default, deserialize_with = "deserialize_present_category")]
    category: Option<ScenarioCategory>,
    blurb: String,
    world: String,
    #[serde(default = "shipped_lighting")]
    lighting: String,
    #[serde(default)]
    generation_seed: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_optional_hour")]
    starting_time_hours: Option<f32>,
    encounter: String,
}

impl<'de> Deserialize<'de> for Scenario {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let fields = ScenarioFields::deserialize(deserializer)?;
        let Some(category) = fields.category else {
            return Err(D::Error::custom(format!(
                "scenario {:?} is missing required field `category`",
                fields.name
            )));
        };
        Ok(Self {
            name: fields.name,
            category,
            blurb: fields.blurb,
            world: fields.world,
            lighting: fields.lighting,
            generation_seed: fields.generation_seed,
            starting_time_hours: fields.starting_time_hours,
            encounter: fields.encounter,
        })
    }
}

fn deserialize_present_category<'de, D>(
    deserializer: D,
) -> Result<Option<ScenarioCategory>, D::Error>
where
    D: Deserializer<'de>,
{
    ScenarioCategory::deserialize(deserializer).map(Some)
}

/// The lighting a scenario gets when it does not name one.
fn shipped_lighting() -> String {
    "config/lighting.ron".to_owned()
}

fn deserialize_optional_hour<'de, D>(deserializer: D) -> Result<Option<f32>, D::Error>
where
    D: Deserializer<'de>,
{
    let hours = Option::<f32>::deserialize(deserializer)?;
    if hours.is_some_and(|hours| !hours.is_finite() || !(0.0..24.0).contains(&hours)) {
        return Err(D::Error::custom(
            "starting_time_hours must be finite and in [0, 24)",
        ));
    }
    Ok(hours)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    /// The shipped file parses, and says enough to build a menu from.
    ///
    /// Mirrors the camera settings test: content that ships is content that can be
    /// wrong, and a `scenarios.ron` that will not parse is a game stuck on "loading…"
    /// with a RON error nobody reads until they run it.
    #[test]
    fn the_shipped_library_parses() {
        let library: ScenarioLibrary =
            ron::from_str(include_str!("../../../assets/config/scenarios.ron"))
                .expect("the shipped scenarios should parse");

        assert!(
            library.scenarios.len() >= 2,
            "a picker with one entry is not a picker"
        );
        for scenario in &library.scenarios {
            assert!(!scenario.name.is_empty(), "a scenario needs a name");
            assert!(!scenario.world.is_empty(), "a scenario needs a world");
            assert!(
                !scenario.encounter.is_empty(),
                "a scenario needs an encounter"
            );
        }
    }

    /// Two scenarios with the same name are indistinguishable on the title screen.
    #[test]
    fn scenario_names_are_unique() {
        let library: ScenarioLibrary =
            ron::from_str(include_str!("../../../assets/config/scenarios.ron"))
                .expect("the shipped scenarios should parse");

        let mut names: Vec<&str> = library.scenarios.iter().map(|s| s.name.as_str()).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "two scenarios share a name");
    }

    #[test]
    fn starting_time_must_be_a_finite_hour_in_the_day() {
        let scenario = |hours: &str| {
            format!(
                r#"(
                    name: "Time",
                    category: Demo,
                    blurb: "Time validation.",
                    world: "config/world.ron",
                    starting_time_hours: {hours},
                    encounter: "config/encounters/bridge-crossing.ron",
                )"#
            )
        };

        for valid in ["None", "Some(0.0)", "Some(12.5)", "Some(23.999)"] {
            assert!(
                ron::from_str::<Scenario>(&scenario(valid)).is_ok(),
                "{valid} should be a valid starting time"
            );
        }
        for invalid in ["Some(-0.1)", "Some(24.0)", "Some(inf)", "Some(NaN)"] {
            let error = ron::from_str::<Scenario>(&scenario(invalid))
                .expect_err("an invalid starting time should be rejected")
                .to_string();
            assert!(
                error.contains("starting_time_hours"),
                "unexpected error for {invalid}: {error}"
            );
        }
    }

    #[test]
    fn a_missing_category_error_names_the_scenario() {
        let error = ron::from_str::<Scenario>(
            r#"(
                name: "Forgotten Lane",
                blurb: "Invalid on purpose.",
                world: "config/world.ron",
                encounter: "config/encounters/bridge-crossing.ron",
            )"#,
        )
        .expect_err("category is a required authoring decision")
        .to_string();

        assert!(error.contains("category"), "{error}");
        assert!(error.contains("Forgotten Lane"), "{error}");
    }

    /// Generated scenarios own distinct reproducible seeds and name an encounter file.
    ///
    /// Whether that encounter places its units through generated *anchors* is a
    /// cross-file fact — the encounter is a separate asset — so it is checked in
    /// `hex_game`, which is allowed to open both. This crate can only see the path.
    #[test]
    fn procedural_scenarios_use_distinct_seeds_and_name_an_encounter() {
        let library: ScenarioLibrary =
            ron::from_str(include_str!("../../../assets/config/scenarios.ron"))
                .expect("the shipped scenarios should parse");
        let generated: Vec<&Scenario> = library
            .scenarios
            .iter()
            .filter(|scenario| scenario.generation_seed.is_some())
            .collect();

        assert_eq!(
            generated.len(),
            8,
            "the scenario library should include all eight generated maps"
        );
        let seeds: HashSet<u64> = generated
            .iter()
            .filter_map(|scenario| scenario.generation_seed)
            .collect();
        assert_eq!(
            seeds.len(),
            generated.len(),
            "generated scenarios should not start on the same configured seed"
        );

        for scenario in generated {
            assert!(
                scenario.encounter.starts_with("config/encounters/"),
                "scenario {:?} does not name an encounter file",
                scenario.name
            );
        }
    }
}
