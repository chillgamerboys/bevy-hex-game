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
//! The cost is that a typo in a path is not a compile error. It is caught by a test in
//! `hex_game` that opens every world file a scenario names.

use bevy::prelude::*;
use serde::Deserialize;

use crate::settings::ScenarioSettings;

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
#[derive(Reflect, Debug, Clone, Deserialize)]
pub struct Scenario {
    /// What the title screen calls it.
    pub name: String,
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
    #[serde(default = "shipped_lighting")]
    pub lighting: String,
    /// Where the units start.
    pub units: ScenarioSettings,
}

/// The lighting a scenario gets when it does not name one.
fn shipped_lighting() -> String {
    "config/lighting.ron".to_owned()
}

#[cfg(test)]
mod tests {
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
}
