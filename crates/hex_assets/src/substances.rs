//! The substance table: what each kind of voxel is called, and how it behaves.
//!
//! Loaded from `assets/config/substances.ron`, so registering a substance is a
//! content change rather than a code change. Terrain generation still has to select
//! it before it appears in a generated world.
//!
//! It lives in `hex_assets` rather than in `hex_map` because both the map and
//! gameplay need it — the map to colour a prism, gameplay to ask whether something
//! is solid enough to stand on or soft enough to dig. `hex_units` cannot see
//! `hex_map`, so a table defined there would be unreachable.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use hex_core::{Screen, SubstanceId};
use serde::Deserialize;
use thiserror::Error;

use crate::{ArtPalette, LoadSettings, Rgb, SwatchId, CONFIG_EXTENSIONS};

/// Registers the substance table for loading.
pub fn plugin(app: &mut App) {
    app.register_type::<SubstanceTable>();
    app.load_settings::<SubstanceFile>("config/substances.ron", CONFIG_EXTENSIONS);
    register_table_builder(app);
}

/// Keeps live voxel ids stable until the current world has been torn down.
fn register_table_builder(app: &mut App) {
    app.add_systems(
        Update,
        build_table_when_loaded.run_if(not(in_state(Screen::Gameplay))),
    );
}

/// How one substance behaves.
#[derive(Reflect, Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Substance {
    /// Exact authored palette entry, or [`None`] only for invisible `air`.
    #[serde(default)]
    pub swatch: Option<SwatchId>,
    /// Resolved colour of a prism made of this substance.
    ///
    /// This is populated only in [`SubstanceTable`]. Authored files name `swatch`
    /// instead, and stale embedded `color` fields are rejected.
    #[serde(skip)]
    pub color: Rgb,
    /// Whether something can stand on it. Air is the obvious `false`; liquids may
    /// join it later.
    pub solid: bool,
    /// Whether it can be dug or tunnelled through. Bedrock is `false`, which is what
    /// stops anything leaving the bottom of the world.
    pub diggable: bool,
}

impl Substance {
    /// Defines a rendered substance through one exact authored palette swatch.
    #[must_use]
    pub fn from_swatch(swatch: SwatchId, solid: bool, diggable: bool) -> Self {
        Self {
            swatch: Some(swatch),
            color: (0.0, 0.0, 0.0),
            solid,
            diggable,
        }
    }

    /// Defines an invisible substance. Cross-file validation permits this only for air.
    #[must_use]
    pub const fn invisible(solid: bool, diggable: bool) -> Self {
        Self {
            swatch: None,
            color: (0.0, 0.0, 0.0),
            solid,
            diggable,
        }
    }
}

/// The raw file, before names are turned into ids.
#[derive(Asset, Resource, Reflect, Debug, Clone, Deserialize)]
#[reflect(Resource)]
pub struct SubstanceFile {
    /// Substances by name.
    pub substances: HashMap<String, Substance>,
}

/// Substances, indexed by the [`SubstanceId`] stored in every voxel.
///
/// # Ids are assigned from sorted names, not file order
///
/// A voxel stores a `SubstanceId`, so if ids were handed out in the order entries
/// appear in the file, **reordering the file would silently rewrite the world** —
/// every stone voxel would become dirt without anything being edited. Sorting the
/// names first makes the mapping depend only on the set of names, so entries can be
/// moved around freely and adding one only shifts ids alphabetically after it.
///
/// [`SubstanceId::AIR`] is pinned to 0 regardless, because it is a compile-time
/// constant that the rest of the game compares against.
#[derive(Resource, Reflect, Debug, Clone, Default)]
#[reflect(Resource)]
pub struct SubstanceTable {
    by_id: Vec<Substance>,
    names: Vec<String>,
    #[reflect(ignore)]
    by_name: HashMap<String, SubstanceId>,
    #[reflect(ignore)]
    source_substances: HashMap<String, Substance>,
    #[reflect(ignore)]
    source_palette: Option<ArtPalette>,
    source_palette_fingerprint: u64,
}

/// Source semantics for one failed cross-file build.
///
/// Invalid initial sources keep Loading blocked. A rejected hot reload is restored to
/// the table's accepted sources, while this latch remembers the rejected candidate so
/// a repeated asset event restores it without emitting the same diagnostic again.
#[derive(Resource, Debug, Clone)]
struct FailedSubstanceTableBuild {
    source_substances: HashMap<String, Substance>,
    source_palette_fingerprint: u64,
}

impl FailedSubstanceTableBuild {
    fn from_sources(file: &SubstanceFile, palette: &ArtPalette) -> Self {
        Self {
            source_substances: file.substances.clone(),
            source_palette_fingerprint: palette.semantic_fingerprint(),
        }
    }

    fn matches_sources(&self, file: &SubstanceFile, palette: &ArtPalette) -> bool {
        self.source_substances == file.substances
            && self.source_palette_fingerprint == palette.semantic_fingerprint()
    }
}

/// A cross-file failure while resolving authored substances through the art palette.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SubstanceTableError {
    /// A rendered substance omitted its required stable palette reference.
    #[error("rendered substance '{substance}' must reference an art-palette swatch")]
    MissingSwatch {
        /// Substance name from `substances.ron`.
        substance: String,
    },
    /// The invisible air sentinel tried to claim a visible authored colour.
    #[error("air is never rendered and cannot reference art-palette swatch '{swatch}'")]
    AirHasSwatch {
        /// Invalid swatch named by air.
        swatch: SwatchId,
    },
    /// The explicit air sentinel tried to participate in terrain behavior.
    #[error(
        "air must be non-solid and non-diggable, but found solid={solid}, diggable={diggable}"
    )]
    InvalidAirBehavior {
        /// Invalid authored solidity.
        solid: bool,
        /// Invalid authored diggability.
        diggable: bool,
    },
    /// A stable reference did not resolve in `palette.ron`.
    #[error("substance '{substance}' references missing art-palette swatch '{swatch}'")]
    UnknownSwatch {
        /// Substance name from `substances.ron`.
        substance: String,
        /// Missing palette reference.
        swatch: SwatchId,
    },
}

impl SubstanceTable {
    /// Properties of a substance, or [`None`] if the id is not in the table.
    #[must_use]
    pub fn get(&self, id: SubstanceId) -> Option<&Substance> {
        self.by_id.get(id.0 as usize)
    }

    /// The id a name maps to, or [`None`] if the table has no such substance.
    #[must_use]
    pub fn id(&self, name: &str) -> Option<SubstanceId> {
        self.by_name.get(name).copied()
    }

    /// The name of a substance, for logs and debugging.
    #[must_use]
    pub fn name(&self, id: SubstanceId) -> Option<&str> {
        self.names.get(id.0 as usize).map(String::as_str)
    }

    /// Whether something can stand on this substance. Unknown ids are not solid,
    /// which fails towards "you cannot walk there" rather than towards falling
    /// through the floor.
    #[must_use]
    pub fn is_solid(&self, id: SubstanceId) -> bool {
        self.get(id).is_some_and(|s| s.solid)
    }

    /// Whether this substance can be dug through. Unknown ids are not diggable.
    #[must_use]
    pub fn is_diggable(&self, id: SubstanceId) -> bool {
        self.get(id).is_some_and(|s| s.diggable)
    }

    /// How many substances the table holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// Whether the table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// Resolves a colour from the exact palette accepted with this table.
    ///
    /// Keeping render consumers on the accepted snapshot prevents a rejected
    /// cross-file hot reload from mixing new palette semantics with the previous
    /// substance ids and behavior.
    #[must_use]
    pub fn palette_color(&self, swatch: &str) -> Option<Rgb> {
        let [red, green, blue] = self
            .source_palette
            .as_ref()?
            .get_str(swatch)?
            .color()
            .to_array();
        Some((red, green, blue))
    }

    /// Builds a table from a loaded file, assigning ids deterministically.
    ///
    /// `air` takes id 0 to match [`SubstanceId::AIR`]; everything else follows in
    /// alphabetical order.
    pub fn from_file(
        file: &SubstanceFile,
        palette: &ArtPalette,
    ) -> Result<Self, SubstanceTableError> {
        let mut names: Vec<String> = file
            .substances
            .keys()
            .filter(|name| name.as_str() != AIR_NAME)
            .cloned()
            .collect();
        names.sort();
        names.insert(0, AIR_NAME.to_owned());

        let mut by_id = Vec::with_capacity(names.len());
        let mut by_name = HashMap::default();

        for (index, name) in names.iter().enumerate() {
            let Some(substance) = file.substances.get(name) else {
                // Only reachable for `air` when the file omits it; the fallback keeps
                // id 0 meaning empty space rather than shifting every other id down.
                by_id.push(Substance {
                    swatch: None,
                    color: (0.0, 0.0, 0.0),
                    solid: false,
                    diggable: false,
                });
                by_name.insert(name.clone(), SubstanceId(0));
                continue;
            };
            let mut substance = substance.clone();
            if name == AIR_NAME && (substance.solid || substance.diggable) {
                return Err(SubstanceTableError::InvalidAirBehavior {
                    solid: substance.solid,
                    diggable: substance.diggable,
                });
            }
            substance.color = match (name.as_str(), substance.swatch.as_ref()) {
                (AIR_NAME, None) => (0.0, 0.0, 0.0),
                (AIR_NAME, Some(swatch)) => {
                    return Err(SubstanceTableError::AirHasSwatch {
                        swatch: swatch.clone(),
                    });
                }
                (_, None) => {
                    return Err(SubstanceTableError::MissingSwatch {
                        substance: name.clone(),
                    });
                }
                (_, Some(swatch)) => {
                    let color =
                        palette
                            .get(swatch)
                            .ok_or_else(|| SubstanceTableError::UnknownSwatch {
                                substance: name.clone(),
                                swatch: swatch.clone(),
                            })?;
                    let [red, green, blue] = color.color().to_array();
                    (red, green, blue)
                }
            };
            by_id.push(substance);
            let id = u16::try_from(index).unwrap_or(u16::MAX);
            by_name.insert(name.clone(), SubstanceId(id));
        }

        Ok(Self {
            by_id,
            names,
            by_name,
            source_substances: file.substances.clone(),
            source_palette: Some(palette.clone()),
            source_palette_fingerprint: palette.semantic_fingerprint(),
        })
    }

    /// Whether this table was resolved from these exact current source semantics.
    #[must_use]
    pub fn matches_sources(&self, file: &SubstanceFile, palette: &ArtPalette) -> bool {
        self.source_substances == file.substances
            && self.source_palette_fingerprint == palette.semantic_fingerprint()
    }

    fn accepted_sources(&self) -> Option<(SubstanceFile, ArtPalette)> {
        Some((
            SubstanceFile {
                substances: self.source_substances.clone(),
            },
            self.source_palette.clone()?,
        ))
    }
}

/// The name reserved for empty space.
const AIR_NAME: &str = "air";

/// Turns the loaded file into the indexed table, and rebuilds it on hot-reload.
///
/// A cross-file failure after a successful build restores both raw resources from
/// the accepted table snapshot. This keeps every runtime consumer on one coherent
/// palette/substance pair and lets a later Loading screen proceed.
fn build_table_when_loaded(
    mut commands: Commands,
    file: Option<Res<SubstanceFile>>,
    palette: Option<Res<ArtPalette>>,
    table: Option<Res<SubstanceTable>>,
    failed_build: Option<Res<FailedSubstanceTableBuild>>,
) {
    let (Some(file), Some(palette)) = (file, palette) else {
        return;
    };
    if table
        .as_deref()
        .is_some_and(|table| table.matches_sources(&file, &palette))
    {
        return;
    }
    if failed_build
        .as_deref()
        .is_some_and(|failed| failed.matches_sources(&file, &palette))
    {
        if let Some((accepted_file, accepted_palette)) =
            table.as_deref().and_then(SubstanceTable::accepted_sources)
        {
            commands.insert_resource(accepted_file);
            commands.insert_resource(accepted_palette);
        }
        return;
    }
    match SubstanceTable::from_file(&file, &palette) {
        Ok(rebuilt) => {
            commands.remove_resource::<FailedSubstanceTableBuild>();
            commands.insert_resource(rebuilt);
        }
        Err(error) => {
            commands.insert_resource(FailedSubstanceTableBuild::from_sources(&file, &palette));
            if let Some((accepted_file, accepted_palette)) =
                table.as_deref().and_then(SubstanceTable::accepted_sources)
            {
                error!(
                    "could not resolve config/substances.ron through art/palette.ron: {error}; \
                     restoring the previous valid substance and palette semantics"
                );
                commands.insert_resource(accepted_file);
                commands.insert_resource(accepted_palette);
            } else {
                error!(
                    "could not resolve initial config/substances.ron through art/palette.ron: \
                     {error}; Loading remains blocked"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use bevy::state::app::StatesPlugin;

    use super::*;
    use crate::{PaletteSwatch, SrgbColor};

    fn swatch_id(name: &str) -> SwatchId {
        SwatchId::new(format!("test/{name}")).expect("test swatch ids should be valid")
    }

    fn test_palette() -> ArtPalette {
        let swatches = ["bedrock", "clay", "grass", "stone"]
            .into_iter()
            .map(|name| {
                let color = if name == "clay" {
                    [0.6, 0.3, 0.2]
                } else {
                    [0.5, 0.5, 0.5]
                };
                (swatch_id(name), test_swatch(name, color))
            })
            .collect::<BTreeMap<_, _>>();
        ArtPalette::new(swatches).expect("test palette should be valid")
    }

    fn test_swatch(name: &str, [red, green, blue]: [f32; 3]) -> PaletteSwatch {
        PaletteSwatch::new(
            name.to_owned(),
            SrgbColor::new(red, green, blue).expect("test swatch color should be valid"),
            BTreeSet::from(["test".to_owned()]),
        )
        .expect("test palette entry should be valid")
    }

    fn test_file() -> SubstanceFile {
        let mut substances = HashMap::default();
        for (name, solid, diggable) in [
            ("stone", true, true),
            ("air", false, false),
            ("grass", true, true),
            ("bedrock", true, false),
        ] {
            let substance = if name == AIR_NAME {
                Substance::invisible(solid, diggable)
            } else {
                Substance::from_swatch(swatch_id(name), solid, diggable)
            };
            substances.insert(name.to_owned(), substance);
        }
        SubstanceFile { substances }
    }

    fn shipped_file() -> SubstanceFile {
        ron::from_str(include_str!("../../../assets/config/substances.ron"))
            .expect("the shipped substance file should parse")
    }

    fn shipped_palette() -> ArtPalette {
        ron::from_str(include_str!("../../../assets/art/palette.ron"))
            .expect("the shipped art palette should parse")
    }

    #[test]
    fn air_is_always_id_zero() {
        let table = SubstanceTable::from_file(&test_file(), &test_palette())
            .expect("test substances should resolve");
        assert_eq!(table.id("air"), Some(SubstanceId::AIR));
        assert!(table.name(SubstanceId::AIR) == Some("air"));
    }

    /// The failure this guards against is severe and silent: if ids came from file
    /// order, moving an entry would turn every stone voxel in every save into dirt.
    #[test]
    fn ids_do_not_depend_on_file_order() {
        let palette = test_palette();
        let first = SubstanceTable::from_file(&test_file(), &palette)
            .expect("first test table should resolve");

        // A HashMap already gives no order guarantee, so build a second table from
        // the same names and assert the mapping is identical.
        let second = SubstanceTable::from_file(&test_file(), &palette)
            .expect("second test table should resolve");

        for name in ["air", "stone", "grass", "bedrock"] {
            assert_eq!(
                first.id(name),
                second.id(name),
                "{name} moved between builds"
            );
        }
    }

    #[test]
    fn non_air_substances_are_alphabetical() {
        let table = SubstanceTable::from_file(&test_file(), &test_palette())
            .expect("test substances should resolve");
        // air is pinned to 0; bedrock, grass, stone follow in order.
        assert_eq!(table.name(SubstanceId(1)), Some("bedrock"));
        assert_eq!(table.name(SubstanceId(2)), Some("grass"));
        assert_eq!(table.name(SubstanceId(3)), Some("stone"));
    }

    #[test]
    fn properties_survive_the_round_trip() {
        let table = SubstanceTable::from_file(&test_file(), &test_palette())
            .expect("test substances should resolve");
        let Some(stone) = table.id("stone") else {
            unreachable!("the test file defines stone")
        };
        let Some(bedrock) = table.id("bedrock") else {
            unreachable!("the test file defines bedrock")
        };

        assert!(table.is_solid(stone));
        assert!(table.is_diggable(stone));
        assert!(table.is_solid(bedrock));
        assert!(
            !table.is_diggable(bedrock),
            "bedrock is what stops anything leaving the bottom of the world"
        );
        assert!(!table.is_solid(SubstanceId::AIR));
    }

    #[test]
    fn shipped_substances_resolve_exact_palette_swatches() {
        let file = shipped_file();
        let palette = shipped_palette();
        let table =
            SubstanceTable::from_file(&file, &palette).expect("shipped substances should resolve");

        for (substance_name, swatch_name, expected) in [
            ("grass", "terrain/grass", (0.35, 0.62, 0.30)),
            ("dirt", "terrain/dirt", (0.45, 0.33, 0.22)),
            ("stone", "terrain/stone", (0.55, 0.55, 0.58)),
            ("gravel", "terrain/gravel", (0.42, 0.40, 0.36)),
            ("water", "liquid/water", (0.08, 0.32, 0.65)),
            ("metal", "structure/metal", (0.30, 0.34, 0.40)),
            ("snow", "terrain/snow", (0.82, 0.88, 0.92)),
            ("ice", "terrain/ice", (0.42, 0.72, 0.88)),
            ("basalt", "terrain/basalt", (0.20, 0.22, 0.24)),
            ("lava", "liquid/lava", (0.90, 0.20, 0.04)),
            ("bedrock", "terrain/bedrock", (0.25, 0.24, 0.28)),
        ] {
            let id = table
                .id(substance_name)
                .unwrap_or_else(|| panic!("{substance_name} should be registered"));
            let substance = table
                .get(id)
                .unwrap_or_else(|| panic!("{substance_name} should resolve"));
            assert_eq!(
                substance.swatch.as_ref().map(SwatchId::as_str),
                Some(swatch_name)
            );
            assert_eq!(substance.color, expected);
        }

        let air = table
            .get(SubstanceId::AIR)
            .expect("air should always resolve");
        assert_eq!(air.swatch, None);
        assert_eq!(air.color, (0.0, 0.0, 0.0));
    }

    #[test]
    fn accepted_palette_colors_are_available_to_render_consumers() {
        let table = SubstanceTable::from_file(&test_file(), &test_palette())
            .expect("test substances should resolve");

        assert_eq!(table.palette_color("test/clay"), Some((0.6, 0.3, 0.2)));
        assert_eq!(table.palette_color("test/missing"), None);
    }

    #[test]
    fn rendered_substances_require_a_known_palette_swatch() {
        let mut missing = test_file();
        missing
            .substances
            .get_mut("stone")
            .expect("stone should exist")
            .swatch = None;
        assert_eq!(
            SubstanceTable::from_file(&missing, &test_palette())
                .expect_err("a rendered substance without a swatch must fail"),
            SubstanceTableError::MissingSwatch {
                substance: "stone".to_owned(),
            }
        );

        let mut unknown = test_file();
        unknown
            .substances
            .get_mut("stone")
            .expect("stone should exist")
            .swatch = Some(swatch_id("unknown"));
        assert_eq!(
            SubstanceTable::from_file(&unknown, &test_palette())
                .expect_err("an unknown swatch must fail"),
            SubstanceTableError::UnknownSwatch {
                substance: "stone".to_owned(),
                swatch: swatch_id("unknown"),
            }
        );
    }

    #[test]
    fn air_cannot_claim_a_visible_palette_swatch() {
        let mut file = test_file();
        file.substances
            .get_mut(AIR_NAME)
            .expect("air should exist")
            .swatch = Some(swatch_id("stone"));

        assert_eq!(
            SubstanceTable::from_file(&file, &test_palette())
                .expect_err("air cannot own a visible swatch"),
            SubstanceTableError::AirHasSwatch {
                swatch: swatch_id("stone"),
            }
        );
    }

    #[test]
    fn explicit_air_cannot_be_solid_or_diggable() {
        for (solid, diggable) in [(true, false), (false, true), (true, true)] {
            let mut file = test_file();
            let air = file.substances.get_mut(AIR_NAME).expect("air should exist");
            air.solid = solid;
            air.diggable = diggable;

            assert_eq!(
                SubstanceTable::from_file(&file, &test_palette())
                    .expect_err("explicit air must remain empty and immutable"),
                SubstanceTableError::InvalidAirBehavior { solid, diggable }
            );
        }
    }

    #[test]
    fn stale_embedded_substance_colors_are_rejected() {
        let error = ron::from_str::<SubstanceFile>(
            r#"(
                substances: {
                    "stone": (
                        swatch: Some("test/stone"),
                        color: (0.5, 0.5, 0.5),
                        solid: true,
                        diggable: true,
                    ),
                },
            )"#,
        )
        .expect_err("substance colors must come from the art palette");

        assert!(
            error.to_string().contains("color"),
            "stale color field returned an unrelated error: {error}"
        );
    }

    #[test]
    fn source_matching_detects_file_and_palette_changes() {
        let file = test_file();
        let palette = test_palette();
        let table =
            SubstanceTable::from_file(&file, &palette).expect("test substances should resolve");
        assert!(table.matches_sources(&file, &palette));

        let mut changed_file = file.clone();
        changed_file
            .substances
            .get_mut("stone")
            .expect("stone should exist")
            .solid = false;
        assert!(!table.matches_sources(&changed_file, &palette));

        let mut changed_palette = palette.clone();
        changed_palette
            .insert(swatch_id("stone"), test_swatch("Stone", [0.2, 0.3, 0.4]))
            .expect("the replacement swatch should be valid");
        assert!(!table.matches_sources(&file, &changed_palette));
    }

    /// An id that is not in the table must not be walkable or diggable. Failing the
    /// other way would let a piece stand on nothing.
    #[test]
    fn unknown_ids_are_neither_solid_nor_diggable() {
        let table = SubstanceTable::from_file(&test_file(), &test_palette())
            .expect("test substances should resolve");
        let unknown = SubstanceId(999);

        assert!(table.get(unknown).is_none());
        assert!(!table.is_solid(unknown));
        assert!(!table.is_diggable(unknown));
    }

    /// Reassigning sorted ids under a live world would reinterpret existing voxels.
    #[test]
    fn table_rebuild_waits_until_gameplay_ends() {
        let original = test_file();
        let mut replacement = test_file();
        replacement.substances.insert(
            "clay".to_owned(),
            Substance::from_swatch(swatch_id("clay"), true, true),
        );
        let palette = test_palette();

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.insert_state(Screen::Gameplay);
        app.insert_resource(
            SubstanceTable::from_file(&original, &palette)
                .expect("the original table should resolve"),
        );
        app.insert_resource(replacement);
        app.insert_resource(palette);
        register_table_builder(&mut app);

        app.update();
        assert!(
            app.world()
                .resource::<SubstanceTable>()
                .id("clay")
                .is_none(),
            "the live world must keep the table its voxel ids were generated from"
        );

        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Title);
        app.update();
        assert!(
            app.world()
                .resource::<SubstanceTable>()
                .id("clay")
                .is_some(),
            "the table should rebuild once gameplay has torn down"
        );
    }

    #[test]
    fn palette_rebuild_waits_until_gameplay_ends() {
        let file = test_file();
        let mut palette = test_palette();
        let original_table =
            SubstanceTable::from_file(&file, &palette).expect("the original table should resolve");
        palette
            .insert(swatch_id("stone"), test_swatch("Stone", [0.2, 0.3, 0.4]))
            .expect("the replacement swatch should be valid");

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.insert_state(Screen::Gameplay);
        app.insert_resource(original_table);
        app.insert_resource(file);
        app.insert_resource(palette);
        register_table_builder(&mut app);

        app.update();
        let stone = app
            .world()
            .resource::<SubstanceTable>()
            .id("stone")
            .expect("stone should resolve");
        assert_eq!(
            app.world()
                .resource::<SubstanceTable>()
                .get(stone)
                .expect("stone should resolve")
                .color,
            (0.5, 0.5, 0.5),
            "a live world must retain its resolved palette"
        );

        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Title);
        app.update();
        assert_eq!(
            app.world()
                .resource::<SubstanceTable>()
                .get(stone)
                .expect("stone should resolve")
                .color,
            (0.2, 0.3, 0.4),
            "the palette should resolve once gameplay has torn down"
        );
    }

    #[test]
    fn invalid_substance_hot_reload_restores_the_previous_source_pair() {
        let file = test_file();
        let palette = test_palette();
        let original =
            SubstanceTable::from_file(&file, &palette).expect("the original table should resolve");

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.insert_state(Screen::Title);
        app.insert_resource(original);
        app.insert_resource(file);
        app.insert_resource(palette);
        register_table_builder(&mut app);
        app.update();

        app.world_mut()
            .resource_mut::<SubstanceFile>()
            .substances
            .get_mut("stone")
            .expect("stone should exist")
            .swatch = Some(swatch_id("unknown"));
        let rejected = app.world().resource::<SubstanceFile>().clone();
        app.update();

        let table = app.world().resource::<SubstanceTable>();
        let stone = table.id("stone").expect("stone should remain registered");
        assert_eq!(
            table.get(stone).expect("stone should resolve").color,
            (0.5, 0.5, 0.5),
            "an invalid source replaced the previous valid table"
        );
        assert!(
            table.matches_sources(
                app.world().resource::<SubstanceFile>(),
                app.world().resource::<ArtPalette>()
            ),
            "a rejected substance file remained installed beside the accepted table"
        );
        assert_eq!(
            app.world()
                .resource::<SubstanceFile>()
                .substances
                .get("stone")
                .and_then(|substance| substance.swatch.as_ref())
                .map(SwatchId::as_str),
            Some("test/stone")
        );
        assert!(
            app.world()
                .resource::<FailedSubstanceTableBuild>()
                .matches_sources(&rejected, app.world().resource::<ArtPalette>()),
            "the rejected candidate should remain latched after source restoration"
        );

        app.world_mut().insert_resource(rejected);
        app.world_mut().clear_trackers();
        app.update();
        assert!(
            app.world().resource::<SubstanceTable>().matches_sources(
                app.world().resource::<SubstanceFile>(),
                app.world().resource::<ArtPalette>()
            ),
            "a repeated rejected asset event must restore the accepted sources again"
        );
        assert!(
            !app.world()
                .resource_ref::<FailedSubstanceTableBuild>()
                .is_changed(),
            "a repeated rejected candidate should reuse its diagnostic latch"
        );
    }

    #[test]
    fn invalid_palette_hot_reload_restores_the_previous_source_pair() {
        let file = test_file();
        let palette = test_palette();
        let original =
            SubstanceTable::from_file(&file, &palette).expect("the original table should resolve");

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.insert_state(Screen::Title);
        app.insert_resource(original);
        app.insert_resource(file);
        app.insert_resource(palette);
        register_table_builder(&mut app);
        app.update();

        app.world_mut()
            .resource_mut::<ArtPalette>()
            .remove(&swatch_id("stone"))
            .expect("the test palette should permit removing stone");
        let rejected = app.world().resource::<ArtPalette>().clone();
        app.update();

        let table = app.world().resource::<SubstanceTable>();
        assert!(
            table.matches_sources(
                app.world().resource::<SubstanceFile>(),
                app.world().resource::<ArtPalette>()
            ),
            "a rejected palette remained installed beside the accepted table"
        );
        let restored_color = app
            .world()
            .resource::<ArtPalette>()
            .get(&swatch_id("stone"))
            .expect("the accepted stone swatch should be restored")
            .color()
            .to_array()
            .map(f32::to_bits);
        assert_eq!(restored_color, [0.5_f32.to_bits(); 3]);
        assert!(
            app.world()
                .resource::<FailedSubstanceTableBuild>()
                .matches_sources(app.world().resource::<SubstanceFile>(), &rejected),
            "the rejected palette should remain latched after source restoration"
        );
    }

    #[test]
    fn invalid_initial_sources_are_latched_until_their_semantics_change() {
        let mut file = test_file();
        file.substances
            .get_mut("stone")
            .expect("stone should exist")
            .swatch = Some(swatch_id("unknown"));
        let palette = test_palette();

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.insert_state(Screen::Title);
        app.insert_resource(file);
        app.insert_resource(palette);
        register_table_builder(&mut app);

        app.update();
        assert!(
            !app.world().contains_resource::<SubstanceTable>(),
            "an invalid initial pair fabricated a substance table"
        );
        let failed = app.world().resource::<FailedSubstanceTableBuild>().clone();
        assert!(failed.matches_sources(
            app.world().resource::<SubstanceFile>(),
            app.world().resource::<ArtPalette>()
        ));

        app.world_mut().clear_trackers();
        app.update();
        assert!(
            app.world()
                .resource::<FailedSubstanceTableBuild>()
                .matches_sources(
                    app.world().resource::<SubstanceFile>(),
                    app.world().resource::<ArtPalette>()
                ),
            "unchanged invalid sources should remain latched"
        );
        assert!(
            !app.world()
                .resource_ref::<FailedSubstanceTableBuild>()
                .is_changed(),
            "an unchanged invalid pair was retried instead of retaining its failure latch"
        );

        app.world_mut()
            .resource_mut::<SubstanceFile>()
            .substances
            .get_mut("stone")
            .expect("stone should exist")
            .swatch = Some(swatch_id("stone"));
        app.update();
        assert!(
            app.world().contains_resource::<SubstanceTable>(),
            "changing the failed source semantics should retry the build"
        );
        assert!(
            !app.world().contains_resource::<FailedSubstanceTableBuild>(),
            "a successful retry should clear the failure latch"
        );
    }

    /// A file missing `air` still has to produce a table where id 0 is empty space,
    /// because `SubstanceId::AIR` is a compile-time constant.
    #[test]
    fn a_file_without_air_still_reserves_id_zero() {
        let mut file = test_file();
        file.substances.remove("air");

        let table = SubstanceTable::from_file(&file, &test_palette())
            .expect("a missing air entry should still resolve");
        assert_eq!(table.name(SubstanceId::AIR), Some("air"));
        assert!(!table.is_solid(SubstanceId::AIR));
    }

    #[test]
    fn terrain_substances_have_the_required_behaviour() {
        let table = SubstanceTable::from_file(&shipped_file(), &shipped_palette())
            .expect("shipped substances should resolve");
        let gravel = table.id("gravel").expect("gravel should be registered");
        let water = table.id("water").expect("water should be registered");
        let metal = table.id("metal").expect("metal should be registered");
        let bedrock = table.id("bedrock").expect("bedrock should be registered");
        let snow = table.id("snow").expect("snow should be registered");
        let ice = table.id("ice").expect("ice should be registered");
        let basalt = table.id("basalt").expect("basalt should be registered");
        let lava = table.id("lava").expect("lava should be registered");

        assert!(table.is_solid(gravel));
        assert!(table.is_diggable(gravel));
        assert!(!table.is_solid(water), "water must not be footing");
        assert!(table.is_diggable(water), "water should be clearable");
        assert!(table.is_solid(metal));
        assert!(table.is_diggable(metal));
        for (name, substance) in [("snow", snow), ("ice", ice), ("basalt", basalt)] {
            assert!(table.is_solid(substance), "{name} must be footing");
            assert!(table.is_diggable(substance), "{name} must be diggable");
        }
        assert!(!table.is_solid(lava), "lava must not be footing");
        assert!(table.is_diggable(lava), "lava should be clearable");
        assert!(!table.is_diggable(bedrock));
    }
}
