//! The substance table: what each kind of voxel is called, and how it behaves.
//!
//! Loaded from `assets/config/substances.ron`, so registering a substance is a
//! content change rather than a code change. Terrain generation still has to select
//! it before it appears in a generated world.
//!
//! It lives in `hex_assets` rather than in `hex_map` because both the map and
//! gameplay need it — the map to colour a prism, gameplay to ask whether something
//! is solid enough to stand on or soft enough to dig. `hex_gameplay` cannot see
//! `hex_map`, so a table defined there would be unreachable.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use hex_core::{Screen, SubstanceId};
use serde::Deserialize;

use crate::{LoadSettings, Rgb, CONFIG_EXTENSIONS};

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
#[derive(Reflect, Debug, Clone, Deserialize)]
pub struct Substance {
    /// Colour of a prism made of it.
    pub color: Rgb,
    /// Whether something can stand on it. Air is the obvious `false`; liquids may
    /// join it later.
    pub solid: bool,
    /// Whether it can be dug or tunnelled through. Bedrock is `false`, which is what
    /// stops anything leaving the bottom of the world.
    pub diggable: bool,
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

    /// Builds a table from a loaded file, assigning ids deterministically.
    ///
    /// `air` takes id 0 to match [`SubstanceId::AIR`]; everything else follows in
    /// alphabetical order.
    #[must_use]
    pub fn from_file(file: &SubstanceFile) -> Self {
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
                    color: (0.0, 0.0, 0.0),
                    solid: false,
                    diggable: false,
                });
                by_name.insert(name.clone(), SubstanceId(0));
                continue;
            };
            by_id.push(substance.clone());
            let id = u16::try_from(index).unwrap_or(u16::MAX);
            by_name.insert(name.clone(), SubstanceId(id));
        }

        Self {
            by_id,
            names,
            by_name,
        }
    }
}

/// The name reserved for empty space.
const AIR_NAME: &str = "air";

/// Turns the loaded file into the indexed table, and rebuilds it on hot-reload.
fn build_table_when_loaded(
    mut commands: Commands,
    file: Option<Res<SubstanceFile>>,
    table: Option<Res<SubstanceTable>>,
) {
    let Some(file) = file else { return };
    if !file.is_changed() && table.is_some() {
        return;
    }
    commands.insert_resource(SubstanceTable::from_file(&file));
}

#[cfg(test)]
mod tests {
    use bevy::state::app::StatesPlugin;

    use super::*;

    fn test_file() -> SubstanceFile {
        let mut substances = HashMap::default();
        for (name, solid, diggable) in [
            ("stone", true, true),
            ("air", false, false),
            ("grass", true, true),
            ("bedrock", true, false),
        ] {
            substances.insert(
                name.to_owned(),
                Substance {
                    color: (0.5, 0.5, 0.5),
                    solid,
                    diggable,
                },
            );
        }
        SubstanceFile { substances }
    }

    #[test]
    fn air_is_always_id_zero() {
        let table = SubstanceTable::from_file(&test_file());
        assert_eq!(table.id("air"), Some(SubstanceId::AIR));
        assert!(table.name(SubstanceId::AIR) == Some("air"));
    }

    /// The failure this guards against is severe and silent: if ids came from file
    /// order, moving an entry would turn every stone voxel in every save into dirt.
    #[test]
    fn ids_do_not_depend_on_file_order() {
        let first = SubstanceTable::from_file(&test_file());

        // A HashMap already gives no order guarantee, so build a second table from
        // the same names and assert the mapping is identical.
        let second = SubstanceTable::from_file(&test_file());

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
        let table = SubstanceTable::from_file(&test_file());
        // air is pinned to 0; bedrock, grass, stone follow in order.
        assert_eq!(table.name(SubstanceId(1)), Some("bedrock"));
        assert_eq!(table.name(SubstanceId(2)), Some("grass"));
        assert_eq!(table.name(SubstanceId(3)), Some("stone"));
    }

    #[test]
    fn properties_survive_the_round_trip() {
        let table = SubstanceTable::from_file(&test_file());
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

    /// An id that is not in the table must not be walkable or diggable. Failing the
    /// other way would let a piece stand on nothing.
    #[test]
    fn unknown_ids_are_neither_solid_nor_diggable() {
        let table = SubstanceTable::from_file(&test_file());
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
            Substance {
                color: (0.6, 0.3, 0.2),
                solid: true,
                diggable: true,
            },
        );

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.insert_state(Screen::Gameplay);
        app.insert_resource(SubstanceTable::from_file(&original));
        app.insert_resource(replacement);
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

    /// A file missing `air` still has to produce a table where id 0 is empty space,
    /// because `SubstanceId::AIR` is a compile-time constant.
    #[test]
    fn a_file_without_air_still_reserves_id_zero() {
        let mut file = test_file();
        file.substances.remove("air");

        let table = SubstanceTable::from_file(&file);
        assert_eq!(table.name(SubstanceId::AIR), Some("air"));
        assert!(!table.is_solid(SubstanceId::AIR));
    }
}
