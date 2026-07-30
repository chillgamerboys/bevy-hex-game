//! Waits for assets before letting gameplay spawn.
//!
//! This screen is what makes `OnEnter(Screen::Gameplay)` a safe place to build the
//! world. Before it existed, the grid spawned in `PreStartup` and the player in
//! `Startup`, and the only thing stopping the player system from reading a
//! resource that did not exist yet was the gap between those two schedules —
//! undocumented, unenforced, and one refactor away from a panic.

use bevy::prelude::*;
use hex_assets::{
    AcceptedContentRevision, ArtPalette, ContentIndex, ContentReadinessSystems, ElementCatalog,
    ElementFile, GameAssets, LatticeFile, LatticeLibrary, SettingsRegistry, SpellBook, SpellFile,
    SubstanceFile, SubstanceTable,
};
use hex_core::Screen;

use super::{despawn_screen, screen_root};
use crate::menus::widgets::UiAssets;
use crate::scenarios::ScenarioContractStatus;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Screen::Loading), spawn_loading);
    app.add_systems(
        PostUpdate,
        (
            crate::scenarios::validate_loaded_scenario,
            super::combat_lab::apply_creator_content_overlay,
            enter_gameplay_when_ready,
        )
            .chain()
            .after(ContentReadinessSystems::PublishAcceptedRevision)
            .run_if(in_state(Screen::Loading)),
    );
    app.add_systems(OnExit(Screen::Loading), despawn_screen(Screen::Loading));
}

fn spawn_loading(mut commands: Commands, assets: Res<UiAssets>) {
    commands
        .spawn(screen_root(Screen::Loading, "Loading Screen"))
        .with_children(|parent| {
            parent.spawn((
                Text::new("loading..."),
                TextFont {
                    font: assets.body.clone().into(),
                    ..TextFont::from_font_size(24.0)
                },
                TextColor(Color::srgb(0.8, 0.8, 0.8)),
            ));
        });
}

/// Gameplay may only start once asset handles are terminal and every settings file
/// and the derived substance table are present.
///
/// Asking the registry rather than listing settings types keeps this screen ignorant
/// of which crates define what — `MapSettings` lives in `hex_map`, and naming it here
/// would put a dependency on the map into the binary's screen code.
///
/// The registry tracks the raw `SubstanceFile` and `ArtPalette`; `SubstanceTable` is
/// resolved from both by a separate `Update` system. This check runs in `PostUpdate`,
/// after deferred resource insertions, and compares deterministic source identities
/// rather than transient Bevy change ticks. That prevents an existing table from
/// satisfying the check during either kind of hot reload even after a rejected edit
/// has remained settled for many frames.
fn enter_gameplay_when_ready(
    assets: Res<GameAssets>,
    asset_server: Res<AssetServer>,
    settings: Res<SettingsRegistry>,
    palette: Option<Res<ArtPalette>>,
    substance_file: Option<Res<SubstanceFile>>,
    substances: Option<Res<SubstanceTable>>,
    scenario_contract: Option<Res<ScenarioContractStatus>>,
    accepted_content: Option<Res<AcceptedContentRevision>>,
    content: Option<Res<ContentIndex>>,
    lattices: Option<Res<LatticeLibrary>>,
    element_file: Option<Res<ElementFile>>,
    elements: Option<Res<ElementCatalog>>,
    spell_file: Option<Res<SpellFile>>,
    spells: Option<Res<SpellBook>>,
    lattice_file: Option<Res<LatticeFile>>,
    mut next: ResMut<NextState<Screen>>,
) {
    let substances_are_current = substance_file
        .as_ref()
        .zip(palette.as_ref())
        .zip(substances.as_ref())
        .is_some_and(|((file, palette), table)| table.matches_sources(file, palette));

    let scenario_is_valid = scenario_contract
        .as_deref()
        .is_some_and(|status| *status == ScenarioContractStatus::Ready);

    // Every layer must describe the same semantic revision. The builders deliberately
    // retain their last valid values after a rejected cross-file edit, so presence and
    // change ticks cannot establish coherence: the changed flag settles after one frame
    // while the retained ids remain stale indefinitely.
    let content_is_current = match (
        accepted_content.as_deref(),
        content.as_deref(),
        lattices.as_deref(),
        element_file.as_deref(),
        elements.as_deref(),
        spell_file.as_deref(),
        spells.as_deref(),
        lattice_file.as_deref(),
        substances.as_deref(),
    ) {
        (
            Some(accepted),
            Some(content),
            Some(lattices),
            Some(element_file),
            Some(elements),
            Some(spell_file),
            Some(spells),
            Some(lattice_file),
            Some(substances),
        ) => {
            accepted.matches_resolved(content, lattices)
                && elements.matches_source(element_file)
                && spells.matches_source(spell_file)
                && content.matches_sources(elements, spells, substances)
                && lattices.matches_sources(lattice_file, elements, spells)
        }
        _ => false,
    };

    if assets.is_ready(&asset_server)
        && settings.all_loaded()
        && substances_are_current
        && scenario_is_valid
        && content_is_current
    {
        next.set(Screen::Gameplay);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use bevy::asset::AssetPlugin;
    use bevy::platform::collections::HashMap;
    use bevy::state::app::StatesPlugin;
    use hex_assets::{PaletteSwatch, SrgbColor, Substance, SwatchId};

    use super::*;

    fn swatch_id(name: &str) -> SwatchId {
        SwatchId::new(format!("test/{name}")).expect("the test swatch id should be valid")
    }

    fn test_palette(stone_red: f32) -> ArtPalette {
        let swatches = [("stone", stone_red), ("clay", 0.6)]
            .into_iter()
            .map(|(name, red)| {
                let swatch = PaletteSwatch::new(
                    format!("Test {name}"),
                    SrgbColor::new(red, 0.5, 0.5).expect("the test color should be valid"),
                    BTreeSet::from(["test".to_owned()]),
                )
                .expect("the test swatch should be valid");
                (swatch_id(name), swatch)
            })
            .collect::<BTreeMap<_, _>>();
        ArtPalette::new(swatches).expect("the test palette should be valid")
    }

    fn substance_file(name: &str) -> SubstanceFile {
        let mut substances = HashMap::default();
        substances.insert("air".to_owned(), Substance::invisible(false, false));
        substances.insert(
            name.to_owned(),
            Substance::from_swatch(swatch_id(name), true, true),
        );
        SubstanceFile { substances }
    }

    fn queue_replacement(mut commands: Commands) {
        commands.insert_resource(substance_file("clay"));
    }

    fn queue_palette_replacement(mut commands: Commands) {
        commands.insert_resource(test_palette(0.8));
    }

    fn app_with_resolved_substances(file: SubstanceFile, palette: ArtPalette) -> App {
        let table = SubstanceTable::from_file(&file, &palette)
            .expect("the initial test substance sources should resolve");
        let element_file: ElementFile = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/elements.ron"
        )))
        .expect("the shipped element fixture should parse");
        let spells: SpellFile = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/spells.ron"
        )))
        .expect("the shipped spell fixture should parse");
        let lattice_file: LatticeFile = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/lattices.ron"
        )))
        .expect("the shipped lattice fixture should parse");
        let elements = ElementCatalog::from_file(&element_file);
        let spell_book = SpellBook::from_file(&spells);
        let content = ContentIndex::build(&elements, &spell_book, &table)
            .expect("the stable fixtures should form a valid content index");
        let lattices = LatticeLibrary::build(&lattice_file, &elements, &spell_book)
            .expect("the stable fixtures should form a valid lattice library");
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin {
                file_path: std::env::temp_dir()
                    .join("hex-game-loading-test-no-assets")
                    .to_string_lossy()
                    .into_owned(),
                ..default()
            },
            StatesPlugin,
        ));
        app.insert_state(Screen::Loading);
        app.insert_resource(UiAssets {
            display: Handle::default(),
            body: Handle::default(),
            hex_cell: Handle::default(),
        });
        app.insert_resource(GameAssets {
            hex_tile: Handle::default(),
            player_pieces: [Handle::default(), Handle::default()],
        });
        app.add_plugins((
            hex_assets::substances::plugin,
            hex_assets::content_index::plugin,
        ));
        // This test installs the cross-file sources directly, so the fixed-file
        // loader registered by the substances plugin has nothing to wait for.
        app.insert_resource(SettingsRegistry::default());
        app.insert_resource(ScenarioContractStatus::Invalid);
        app.insert_resource(file);
        app.insert_resource(palette);
        app.insert_resource(table);
        // These tests isolate substance/palette recovery. Install one fully coherent
        // content graph so the independent acceptance gate cannot mask that seam.
        app.insert_resource(element_file);
        app.insert_resource(elements);
        app.insert_resource(spells);
        app.insert_resource(spell_book);
        app.insert_resource(lattice_file);
        app.insert_resource(content);
        app.insert_resource(lattices);
        plugin(&mut app);
        app
    }

    fn assert_loading_accepts_restored_sources(app: &mut App) {
        app.world_mut()
            .insert_resource(ScenarioContractStatus::Ready);
        app.update();
        assert!(
            matches!(
                app.world().resource::<NextState<Screen>>(),
                &NextState::Pending(Screen::Gameplay)
            ),
            "Loading should proceed after the accepted source pair has settled"
        );
    }

    #[test]
    fn loading_rejects_a_settled_stale_content_graph_and_recovers_after_repair() {
        let mut app = app_with_resolved_substances(substance_file("stone"), test_palette(0.5));
        app.update();
        assert!(
            app.world().contains_resource::<AcceptedContentRevision>(),
            "the fixture should begin as one accepted content revision"
        );

        let original = app.world().resource::<SpellFile>().clone();
        let mut broken = original.clone();
        broken
            .spells
            .get_mut("Ember")
            .expect("the shipped fixture contains Ember")
            .requirements
            .first_mut()
            .expect("Ember has a requirement")
            .element = "Aether".to_owned();
        app.world_mut()
            .insert_resource(SpellBook::from_file(&broken));
        app.world_mut().insert_resource(broken);
        app.world_mut()
            .insert_resource(ScenarioContractStatus::Ready);

        for _ in 0..4 {
            app.update();
            assert!(
                matches!(
                    app.world().resource::<NextState<Screen>>(),
                    &NextState::Unchanged
                ),
                "Loading must remain blocked after source change ticks settle"
            );
            assert!(
                !app.world().contains_resource::<AcceptedContentRevision>(),
                "a stale retained index must not remain accepted"
            );
        }

        app.world_mut()
            .insert_resource(SpellBook::from_file(&original));
        app.world_mut().insert_resource(original);
        app.update();
        assert!(matches!(
            app.world().resource::<NextState<Screen>>(),
            &NextState::Pending(Screen::Gameplay)
        ));
    }

    /// A settings replacement queued in `Update` must be visible before readiness is
    /// evaluated, or gameplay can start with the previous derived table.
    #[test]
    fn hot_reload_waits_for_the_substance_table_to_catch_up() {
        let original = substance_file("stone");
        let palette = test_palette(0.5);
        let original_table = SubstanceTable::from_file(&original, &palette)
            .expect("the original test substances should resolve");

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), StatesPlugin));
        app.insert_state(Screen::Loading);
        app.insert_resource(UiAssets {
            display: Handle::default(),
            body: Handle::default(),
            hex_cell: Handle::default(),
        });
        app.init_resource::<SettingsRegistry>();
        app.insert_resource(GameAssets {
            hex_tile: Handle::default(),
            player_pieces: [Handle::default(), Handle::default()],
        });
        app.insert_resource(ScenarioContractStatus::Ready);
        app.insert_resource(original);
        app.insert_resource(palette);
        plugin(&mut app);

        // Prime this system's change detection without a table, so it cannot enter
        // gameplay yet.
        app.update();
        assert!(matches!(
            app.world().resource::<NextState<Screen>>(),
            &NextState::Unchanged
        ));

        app.insert_resource(original_table);
        app.add_systems(Update, queue_replacement);
        app.update();

        let world = app.world();
        assert!(
            world
                .resource::<SubstanceFile>()
                .substances
                .contains_key("clay"),
            "the replacement file should have been applied"
        );
        assert!(
            world.resource::<SubstanceTable>().id("clay").is_none(),
            "the table should still represent the previous file in this frame"
        );
        assert!(
            matches!(world.resource::<NextState<Screen>>(), &NextState::Unchanged),
            "gameplay must wait for the derived table to catch up"
        );
    }

    /// A palette-only hot reload also invalidates the derived substance table.
    #[test]
    fn hot_reload_waits_for_palette_colors_to_rebuild_the_substance_table() {
        let file = substance_file("stone");
        let palette = test_palette(0.5);
        let original_table = SubstanceTable::from_file(&file, &palette)
            .expect("the original test substances should resolve");

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), StatesPlugin));
        app.insert_state(Screen::Loading);
        app.insert_resource(UiAssets {
            display: Handle::default(),
            body: Handle::default(),
            hex_cell: Handle::default(),
        });
        app.init_resource::<SettingsRegistry>();
        app.insert_resource(GameAssets {
            hex_tile: Handle::default(),
            player_pieces: [Handle::default(), Handle::default()],
        });
        app.insert_resource(ScenarioContractStatus::Ready);
        app.insert_resource(file);
        app.insert_resource(palette);
        plugin(&mut app);

        app.update();
        assert!(matches!(
            app.world().resource::<NextState<Screen>>(),
            &NextState::Unchanged
        ));

        app.insert_resource(original_table);
        app.add_systems(Update, queue_palette_replacement);
        app.update();

        let world = app.world();
        let palette = world.resource::<ArtPalette>();
        let changed_red = palette
            .get_str("test/stone")
            .expect("the replacement palette should contain stone")
            .color()
            .red();
        assert_eq!(changed_red.to_bits(), 0.8_f32.to_bits());
        assert!(
            !world
                .resource::<SubstanceTable>()
                .matches_sources(world.resource::<SubstanceFile>(), palette),
            "the old table should not match the replacement palette"
        );
        assert!(
            matches!(world.resource::<NextState<Screen>>(), &NextState::Unchanged),
            "gameplay must wait for palette colors to reach the derived table"
        );
    }

    #[test]
    fn loading_recovers_after_an_invalid_substance_reference_hot_reload() {
        let file = substance_file("stone");
        let palette = test_palette(0.5);
        let mut app = app_with_resolved_substances(file, palette);
        app.update();

        app.world_mut()
            .resource_mut::<SubstanceFile>()
            .substances
            .get_mut("stone")
            .expect("stone should exist")
            .swatch = Some(swatch_id("missing"));
        app.update();

        let world = app.world();
        assert!(
            world.resource::<SubstanceTable>().matches_sources(
                world.resource::<SubstanceFile>(),
                world.resource::<ArtPalette>()
            ),
            "the rejected substance source should be restored before Loading retries"
        );
        assert_loading_accepts_restored_sources(&mut app);
    }

    #[test]
    fn loading_recovers_after_a_palette_only_cross_file_failure() {
        let file = substance_file("stone");
        let palette = test_palette(0.5);
        let mut app = app_with_resolved_substances(file, palette);
        app.update();

        app.world_mut()
            .resource_mut::<ArtPalette>()
            .remove(&swatch_id("stone"))
            .expect("the test palette should permit removing stone");
        app.update();

        let world = app.world();
        assert!(
            world.resource::<SubstanceTable>().matches_sources(
                world.resource::<SubstanceFile>(),
                world.resource::<ArtPalette>()
            ),
            "the rejected palette should be restored before Loading retries"
        );
        assert_loading_accepts_restored_sources(&mut app);
    }
}
