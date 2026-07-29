//! Human sandbox composition and scalable deterministic fixture selection.

use std::collections::BTreeMap;

use bevy::ecs::system::SystemParam;
use bevy::picking::events::{Click, Pointer};
use bevy::prelude::*;
use bevy::text::EditableText;
use bevy::ui_widgets::ScrollArea;
use hex_assets::{
    character_lattice_file, character_runtime_key, combined_spell_file, CombatLabDeploymentRegion,
    CombatLabMapCatalog, CombatLabMapDefinition, CombatLabRegionCenter, ContentIndex,
    CreationCellKind, CreationPresetCatalog, CustomCharacterId, ElementCatalog, Encounter,
    EncounterFaction, EncounterPlacement, FormationCenter, GameAssets, LatticeFile, LatticeLibrary,
    PresetAudience, Roster, RosterEntry, SavedCharacter, Scenario, ScenarioLibrary, SpellBook,
    SpellFile, SpellReference, SubstanceTable,
};
use hex_core::{
    GameplayPhase, GameplaySetup, Headroom, HexCoord, HexSpan, HexTile, MapAnchorId, MapAnchors,
    ResolvedMapSeed, Screen, SubstanceId, TilePos, TraversalBlockers, TraversalProfile,
};
use hex_units::{Body, Faction, Footing, Reach, StandsOn};

use crate::creation_presentation::CharacterBuildSummary;
use crate::creation_store::CreationStore;
use crate::menus::lattice_view::short_name;
use crate::menus::widgets::{
    blurb, display, element_color, fine, heading, label, panel, panel_node, row_button, UiAssets,
    DANGER, FUSION_COLOR,
};
use crate::scenarios::ScenarioToLoad;

use super::{despawn_screen, screen_root, screen_root_node};

const MAX_ROSTER: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum LabTab {
    #[default]
    Sandbox,
    Fixtures,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SandboxMap {
    Flat,
    Crossing,
    Hills,
}

impl SandboxMap {
    const ALL: [Self; 3] = [Self::Flat, Self::Crossing, Self::Hills];

    const fn stable_id(self) -> &'static str {
        match self {
            Self::Flat => "flat-arena",
            Self::Crossing => "the-crossing",
            Self::Hills => "procedural-hills",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Flat => "Flat Arena",
            Self::Crossing => "The Crossing",
            Self::Hills => "Procedural Hills",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RosterChoice {
    Template(String),
    Custom(CustomCharacterId),
}

#[derive(Resource, Debug)]
struct CombatLabState {
    tab: LabTab,
    map: SandboxMap,
    players: Vec<RosterChoice>,
    hostiles: Vec<RosterChoice>,
    fixture_filter: String,
    creator_origin: bool,
    notice: String,
    revision: u64,
}

impl Default for CombatLabState {
    fn default() -> Self {
        Self {
            tab: LabTab::Sandbox,
            map: SandboxMap::Flat,
            players: vec![RosterChoice::Template("hedge-mage".to_owned())],
            hostiles: vec![RosterChoice::Template("raider".to_owned())],
            fixture_filter: String::new(),
            creator_origin: false,
            notice: String::new(),
            revision: 1,
        }
    }
}

impl CombatLabState {
    fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
}

/// Prefills the Sandbox when Test on Map originates in the Creator.
#[derive(Resource, Debug, Clone, Copy)]
pub(crate) struct CreatorTestRequest {
    pub(crate) character: CustomCharacterId,
}

/// Opens a blocked Sandbox record in the Creator without discarding the transient setup.
#[derive(Resource, Debug, Clone, Copy)]
pub(crate) struct CreatorEditRequest {
    pub(crate) character: CustomCharacterId,
}

/// Frozen content resources applied at the final loading boundary.
#[derive(Resource, Debug, Clone)]
pub(crate) struct CreatorContentOverlay {
    spells: SpellBook,
    content: ContentIndex,
    lattices: LatticeLibrary,
    display_names: BTreeMap<String, String>,
}

/// Optional player-facing name that leaves the stable runtime archetype key intact.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub(crate) struct CreatorDisplayName(pub(crate) String);

/// Why a temporary test session was launched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CombatLabSessionKind {
    /// Human-composed transient map and rosters.
    Sandbox,
    /// Immutable catalog fixture addressed by stable machine id.
    FixedFixture(String),
}

/// Marks gameplay launched by Combat Lab and owns its frozen retry/return contract.
///
/// Normal scenarios have no such resource, making all three session kinds typed and
/// mutually exclusive.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub(crate) struct CombatLabSession {
    pub(crate) kind: CombatLabSessionKind,
    pub(crate) return_to: Screen,
}

/// Frozen human deployment state carried over the already-loaded terrain.
#[derive(Resource, Debug, Clone)]
struct DeploymentSession {
    map_definition: CombatLabMapDefinition,
    players: Vec<RosterChoice>,
    hostiles: Vec<RosterChoice>,
    player_placements: Vec<Option<TilePos>>,
    hostile_placements: Vec<Option<TilePos>>,
    active_player: bool,
    active_index: usize,
    undo: Vec<(bool, usize, Option<TilePos>)>,
    player_surfaces: Vec<TilePos>,
    hostile_surfaces: Vec<TilePos>,
    notice: String,
}

impl DeploymentSession {
    fn new(
        map_definition: CombatLabMapDefinition,
        players: Vec<RosterChoice>,
        hostiles: Vec<RosterChoice>,
    ) -> Self {
        let player_len = players.len();
        let hostile_len = hostiles.len();
        Self {
            map_definition,
            players,
            hostiles,
            player_placements: vec![None; player_len],
            hostile_placements: vec![None; hostile_len],
            active_player: true,
            active_index: 0,
            undo: Vec::new(),
            player_surfaces: Vec::new(),
            hostile_surfaces: Vec::new(),
            notice: "Select a highlighted Player surface for unit 1.".to_owned(),
        }
    }

    fn complete(&self) -> bool {
        placements_complete_exact(&self.player_placements, self.players.len())
            && placements_complete_exact(&self.hostile_placements, self.hostiles.len())
    }
}

#[derive(Component, Debug, Clone, Copy)]
struct DeploymentSurface {
    pos: TilePos,
    player: bool,
}

#[derive(Component)]
struct DeploymentWorldEntity;

#[derive(Component)]
struct DeploymentHidden;

#[derive(Component)]
struct DeploymentHud;

#[derive(Component, Debug, Clone, Copy)]
enum DeploymentAction {
    Undo,
    ClearPlayer,
    ClearHostile,
    AutoPlace,
    Back,
    StartCombat,
}

#[derive(Component, Debug, Clone)]
enum LabAction {
    Tab(LabTab),
    Back,
    SelectMap(SandboxMap),
    AddPlayerTemplate(String),
    AddHostileTemplate(String),
    AddPlayerCustom(CustomCharacterId),
    AddHostileCustom(CustomCharacterId),
    RemovePlayer(usize),
    RemoveHostile(usize),
    MovePlayer(usize, i8),
    MoveHostile(usize, i8),
    EditCustom(CustomCharacterId),
    PrepareDeployment,
    StartFixture(String),
}

#[derive(Component)]
struct LabRoot;

#[derive(Component)]
struct FixtureFilter;

#[derive(Debug, Clone, Copy)]
struct FixtureDefinition {
    id: &'static str,
    name: &'static str,
    tags: &'static str,
    description: &'static str,
    scenario: &'static str,
    map_seed: &'static str,
    roster: &'static str,
}

const FIXTURES: [FixtureDefinition; 4] = [
    FixtureDefinition {
        id: "ability-lab",
        name: "Ability Lab",
        tags: "aiming reveal restore revival",
        description: "A flat 2v1 for aiming, friendly damage, reveal, restoration, and revival.",
        scenario: "Ability Lab",
        map_seed: "Flat Arena · authored",
        roster: "2 Player · 1 Hostile",
    },
    FixtureDefinition {
        id: "raider-mirror",
        name: "Raider Mirror",
        tags: "identity defense enchantment",
        description: "Same archetype on both sides, with deterministic defensive enchantments.",
        scenario: "Raider Mirror",
        map_seed: "Flat Arena · authored",
        roster: "1 Player Raider · 1 Hostile Raider",
    },
    FixtureDefinition {
        id: "creator-spell-matrix",
        name: "Creator Spell Matrix",
        tags: "creator disable burn reveal restore defense",
        description: "Creator-format spell delivery against the flat deterministic roster.",
        scenario: "Ability Lab",
        map_seed: "Flat Arena · authored",
        roster: "Fixture Caster · Fixture Target",
    },
    FixtureDefinition {
        id: "creator-roster-matrix",
        name: "Creator Roster Matrix",
        tags: "creator roster selection ordering",
        description: "Mixed roster selection, stable unit ordering, and multi-unit combat.",
        scenario: "Ability Lab",
        map_seed: "Flat Arena · authored",
        roster: "2 Player · 2 Hostile creator records",
    },
];

/// Scenario name behind a stable automated fixture id.
#[cfg(feature = "visual-walk")]
pub(crate) fn fixture_scenario_name(id: &str) -> Option<&'static str> {
    FIXTURES
        .iter()
        .find(|fixture| fixture.id == id)
        .map(|fixture| fixture.scenario)
}

/// Frozen creator content and encounter behind an automation fixture, if it uses one.
///
/// Shipped Ability Lab and Raider Mirror return `None`; creator matrices return the
/// same isolated payload whether launched through the selector or directly by a walk.
pub(crate) fn creator_fixture_payload(
    id: &str,
    presets: Option<&CreationPresetCatalog>,
    shipped_spells: Option<&SpellFile>,
    base_lattices: Option<&LatticeFile>,
    elements: Option<&ElementCatalog>,
    substances: Option<&SubstanceTable>,
) -> Result<Option<(CreatorContentOverlay, Encounter)>, String> {
    let rosters = match id {
        "creator-spell-matrix" => Some((
            vec![RosterChoice::Custom(CustomCharacterId(1001))],
            vec![RosterChoice::Custom(CustomCharacterId(1002))],
        )),
        "creator-roster-matrix" => Some((
            vec![
                RosterChoice::Custom(CustomCharacterId(1001)),
                RosterChoice::Custom(CustomCharacterId(1003)),
            ],
            vec![
                RosterChoice::Custom(CustomCharacterId(1002)),
                RosterChoice::Custom(CustomCharacterId(1001)),
            ],
        )),
        _ => None,
    };
    let Some((players, hostiles)) = rosters else {
        return Ok(None);
    };
    let presets =
        presets.ok_or_else(|| "packaged creator fixtures are still loading".to_owned())?;
    let packaged = presets.library_for(PresetAudience::AutomationFixture);
    let overlay = build_creator_overlay(
        &players,
        &hostiles,
        &packaged,
        shipped_spells,
        base_lattices,
        elements,
        substances,
    )?;
    let encounter = sandbox_encounter(SandboxMap::Flat, &players, &hostiles, None);
    Ok(Some((overlay, encounter)))
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<CombatLabState>()
        .add_systems(
            OnEnter(Screen::Gameplay),
            (
                apply_creator_display_names.in_set(GameplaySetup::Restore),
                enter_deployment.in_set(GameplaySetup::Finalize),
            ),
        )
        .add_systems(
            Update,
            handle_deployment_actions.run_if(in_state(Screen::Gameplay)),
        )
        .add_observer(on_deployment_surface_clicked)
        .add_systems(OnExit(Screen::Gameplay), clear_deployment_world)
        .add_systems(
            OnEnter(Screen::CombatLab),
            (initialize_lab, spawn_lab).chain(),
        )
        .add_systems(
            Update,
            (sync_fixture_filter, handle_lab_actions, rebuild_lab)
                .chain()
                .run_if(in_state(Screen::CombatLab)),
        )
        .add_systems(OnExit(Screen::CombatLab), despawn_screen(Screen::CombatLab));
}

fn initialize_lab(
    mut commands: Commands,
    request: Option<Res<CreatorTestRequest>>,
    mut state: ResMut<CombatLabState>,
    spell_file: Option<Res<SpellFile>>,
    lattice_file: Option<Res<LatticeFile>>,
    elements: Option<Res<ElementCatalog>>,
    substances: Option<Res<SubstanceTable>>,
) {
    restore_shipped_content(
        &mut commands,
        spell_file.as_deref(),
        lattice_file.as_deref(),
        elements.as_deref(),
        substances.as_deref(),
    );
    commands.remove_resource::<CombatLabSession>();
    commands.remove_resource::<DeploymentSession>();
    commands.insert_resource(GameplayPhase::Active);
    if let Some(request) = request {
        state.tab = LabTab::Sandbox;
        state.players = vec![RosterChoice::Custom(request.character)];
        state.notice = "Creator character prefilled; choose the rest of the test.".to_owned();
        state.creator_origin = true;
        commands.remove_resource::<CreatorTestRequest>();
    } else {
        state.creator_origin = false;
    }
    state.bump();
}

fn spawn_lab(
    mut commands: Commands,
    assets: Res<UiAssets>,
    state: Res<CombatLabState>,
    store: Res<CreationStore>,
    elements: Option<Res<ElementCatalog>>,
    spells: Option<Res<SpellBook>>,
    presets: Option<Res<CreationPresetCatalog>>,
    maps: Option<Res<CombatLabMapCatalog>>,
) {
    spawn_lab_ui(
        &mut commands,
        &assets,
        &state,
        &store,
        elements.as_deref(),
        spells.as_deref(),
        presets.as_deref(),
        maps.as_deref(),
    );
}

fn rebuild_lab(
    mut commands: Commands,
    roots: Query<Entity, With<LabRoot>>,
    assets: Res<UiAssets>,
    state: Res<CombatLabState>,
    store: Res<CreationStore>,
    elements: Option<Res<ElementCatalog>>,
    spells: Option<Res<SpellBook>>,
    presets: Option<Res<CreationPresetCatalog>>,
    maps: Option<Res<CombatLabMapCatalog>>,
    mut last_revision: Local<u64>,
) {
    if roots.is_empty() || *last_revision != state.revision {
        for root in &roots {
            commands.entity(root).despawn();
        }
        spawn_lab_ui(
            &mut commands,
            &assets,
            &state,
            &store,
            elements.as_deref(),
            spells.as_deref(),
            presets.as_deref(),
            maps.as_deref(),
        );
        *last_revision = state.revision;
    }
}

fn spawn_lab_ui(
    commands: &mut Commands,
    assets: &UiAssets,
    state: &CombatLabState,
    store: &CreationStore,
    elements: Option<&ElementCatalog>,
    spells: Option<&SpellBook>,
    presets: Option<&CreationPresetCatalog>,
    maps: Option<&CombatLabMapCatalog>,
) {
    commands
        .spawn((screen_root(Screen::CombatLab, "Combat Lab Screen"), LabRoot))
        .insert(Node {
            padding: UiRect::all(Val::Px(18.0)),
            justify_content: JustifyContent::FlexStart,
            ..screen_root_node()
        })
        .with_children(|root| {
            root.spawn(display(assets, "Combat Lab"));
            root.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|tabs| {
                lab_button(
                    tabs,
                    assets,
                    "Sandbox",
                    LabAction::Tab(LabTab::Sandbox),
                    170.0,
                );
                lab_button(
                    tabs,
                    assets,
                    "Fixed Fixtures",
                    LabAction::Tab(LabTab::Fixtures),
                    170.0,
                );
                lab_button(tabs, assets, "Back", LabAction::Back, 100.0);
            });
            if !state.notice.is_empty() {
                root.spawn(blurb(assets, state.notice.clone()));
            }
            match state.tab {
                LabTab::Sandbox => {
                    spawn_sandbox_setup(
                        root, assets, state, store, elements, spells, presets, maps,
                    );
                }
                LabTab::Fixtures => spawn_fixture_selector(root, assets, state),
            }
        });
}

fn lab_button(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    text: impl Into<String>,
    action: LabAction,
    width: f32,
) {
    let text = text.into();
    parent
        .spawn((row_button(text.clone(), width), action))
        .with_child(label(assets, text));
}

fn spawn_sandbox_setup(
    root: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    state: &CombatLabState,
    store: &CreationStore,
    elements: Option<&ElementCatalog>,
    spells: Option<&SpellBook>,
    presets: Option<&CreationPresetCatalog>,
    maps: Option<&CombatLabMapCatalog>,
) {
    root.spawn(Node {
        width: Val::Percent(96.0),
        height: Val::Px(0.0),
        min_height: Val::Px(0.0),
        flex_grow: 1.0,
        flex_direction: FlexDirection::Row,
        column_gap: Val::Px(12.0),
        ..default()
    })
    .with_children(|body| {
        body.spawn(panel())
            .insert(Node {
                width: Val::Px(300.0),
                min_height: Val::Px(0.0),
                ..panel_node()
            })
            .with_children(|map_panel| {
                map_panel.spawn(heading(assets, "map"));
                for map in SandboxMap::ALL {
                    let label = maps
                        .and_then(|catalog| catalog.get(map.stable_id()))
                        .map_or_else(
                            || map.label().to_owned(),
                            |record| record.display_name.clone(),
                        );
                    lab_button(map_panel, assets, label, LabAction::SelectMap(map), 250.0);
                }
                map_panel.spawn(fine(
                    assets,
                    "All maps use fixed seeds and authored deployment regions.",
                ));
            });

        body.spawn(panel())
            .insert(Node {
                min_width: Val::Px(0.0),
                min_height: Val::Px(0.0),
                flex_grow: 1.0,
                ..panel_node()
            })
            .with_children(|rosters| {
                let map_label = maps
                    .and_then(|catalog| catalog.get(state.map.stable_id()))
                    .map_or_else(|| state.map.label(), |record| record.display_name.as_str());
                rosters.spawn(heading(assets, format!("{map_label} · rosters")));
                rosters
                    .spawn(Node {
                        min_height: Val::Px(0.0),
                        flex_grow: 1.0,
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(12.0),
                        ..default()
                    })
                    .with_children(|columns| {
                        spawn_roster_column(
                            columns,
                            assets,
                            "Player",
                            &state.players,
                            true,
                            store,
                            elements,
                            spells,
                            presets,
                        );
                        spawn_roster_column(
                            columns,
                            assets,
                            "Hostile · baseline AI",
                            &state.hostiles,
                            false,
                            store,
                            elements,
                            spells,
                            presets,
                        );
                    });
                let ready = !state.players.is_empty()
                    && !state.hostiles.is_empty()
                    && state.players.len() <= MAX_ROSTER
                    && state.hostiles.len() <= MAX_ROSTER;
                if ready {
                    lab_button(
                        rosters,
                        assets,
                        "Load Map & Deploy",
                        LabAction::PrepareDeployment,
                        230.0,
                    );
                } else {
                    rosters
                        .spawn(blurb(assets, "Each side needs 1–6 Map-ready characters."))
                        .insert(TextColor(DANGER));
                }
            });
    });
}

#[expect(
    clippy::too_many_arguments,
    reason = "one roster column presents both packaged and saved choices"
)]
fn spawn_roster_column(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    title: &str,
    roster: &[RosterChoice],
    player: bool,
    store: &CreationStore,
    elements: Option<&ElementCatalog>,
    spells: Option<&SpellBook>,
    presets: Option<&CreationPresetCatalog>,
) {
    parent
        .spawn(panel())
        .insert(Node {
            min_width: Val::Px(0.0),
            min_height: Val::Px(0.0),
            flex_grow: 1.0,
            ..panel_node()
        })
        .with_children(|column| {
            column.spawn(heading(assets, title));
            column
                .spawn((
                    ScrollArea,
                    Node {
                        min_height: Val::Px(0.0),
                        flex_grow: 1.0,
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(7.0),
                        overflow: Overflow::scroll_y(),
                        ..default()
                    },
                ))
                .with_children(|list| {
                    for (index, choice) in roster.iter().enumerate() {
                        let (up, down, remove) = if player {
                            (
                                LabAction::MovePlayer(index, -1),
                                LabAction::MovePlayer(index, 1),
                                LabAction::RemovePlayer(index),
                            )
                        } else {
                            (
                                LabAction::MoveHostile(index, -1),
                                LabAction::MoveHostile(index, 1),
                                LabAction::RemoveHostile(index),
                            )
                        };
                        spawn_build_card(
                            list,
                            assets,
                            choice,
                            store,
                            presets,
                            elements,
                            spells,
                            Some((index + 1, up, down, remove)),
                            None,
                        );
                    }
                    if roster.len() < MAX_ROSTER {
                        list.spawn(fine(assets, "ADD TEMPLATE"));
                        for template in ["wolf", "raider", "hedge-mage"] {
                            spawn_build_card(
                                list,
                                assets,
                                &RosterChoice::Template(template.to_owned()),
                                store,
                                presets,
                                elements,
                                spells,
                                None,
                                Some(if player {
                                    LabAction::AddPlayerTemplate(template.to_owned())
                                } else {
                                    LabAction::AddHostileTemplate(template.to_owned())
                                }),
                            );
                        }
                        list.spawn(fine(assets, "ADD SAVED CHARACTER"));
                        for character in &store.file.characters {
                            let ready = elements.zip(spells).is_some_and(|(elements, spells)| {
                                character_is_map_ready(character, &store.file, elements, spells)
                            });
                            spawn_build_card(
                                list,
                                assets,
                                &RosterChoice::Custom(character.id),
                                store,
                                presets,
                                elements,
                                spells,
                                None,
                                ready.then_some(if player {
                                    LabAction::AddPlayerCustom(character.id)
                                } else {
                                    LabAction::AddHostileCustom(character.id)
                                }),
                            );
                        }
                    }
                });
        });
}

#[expect(
    clippy::too_many_arguments,
    reason = "the shared roster card renders a complete frozen build projection"
)]
fn spawn_build_card(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    choice: &RosterChoice,
    store: &CreationStore,
    presets: Option<&CreationPresetCatalog>,
    elements: Option<&ElementCatalog>,
    spells: Option<&SpellBook>,
    roster_actions: Option<(usize, LabAction, LabAction, LabAction)>,
    add_action: Option<LabAction>,
) {
    let Some((character, library, source)) = choice_record(choice, store, presets) else {
        parent
            .spawn(blurb(
                assets,
                format!("Missing record: {}", choice_name(choice, store)),
            ))
            .insert(TextColor(DANGER));
        return;
    };
    let summary = CharacterBuildSummary::from_saved(&character, &library, elements, spells);
    let ready = summary.ready()
        && match choice {
            RosterChoice::Template(_) => true,
            RosterChoice::Custom(_) => {
                character_is_map_ready_optional(&character, &library, elements, spells)
            }
        };
    parent
        .spawn(panel())
        .insert((
            Node {
                width: Val::Percent(100.0),
                padding: UiRect::all(Val::Px(8.0)),
                row_gap: Val::Px(3.0),
                ..panel_node()
            },
            BorderColor::all(if ready {
                Color::srgba(0.93, 0.79, 0.46, 0.42)
            } else {
                Color::srgba(0.94, 0.36, 0.30, 0.65)
            }),
        ))
        .with_children(|card| {
            card.spawn(Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(9.0),
                ..default()
            })
            .with_children(|top| {
                spawn_mini_lattice(top, assets, &character, elements);
                top.spawn(Node {
                    min_width: Val::Px(0.0),
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(2.0),
                    ..default()
                })
                .with_children(|text| {
                    text.spawn(heading(
                        assets,
                        roster_actions.as_ref().map_or_else(
                            || summary.name.clone(),
                            |(slot, _, _, _)| format!("{slot}. {}", summary.name),
                        ),
                    ));
                    text.spawn(fine(
                        assets,
                        format!("{source} · {}", summary.compact_line()),
                    ));
                    if !summary.attunement.is_empty() {
                        text.spawn(fine(
                            assets,
                            format!("Attunement / channel · {}", summary.attunement.join(", ")),
                        ));
                    }
                    for spell in &summary.spells {
                        text.spawn(fine(assets, format!("{} · {}", spell.name, spell.sentence)));
                    }
                });
            });
            if !ready {
                let reason = if summary.issues.is_empty() {
                    "Needs at least one supported, fresh-cast spell.".to_owned()
                } else {
                    summary.issues.join(" · ")
                };
                card.spawn(fine(assets, format!("BLOCKED · {reason}")))
                    .insert(TextColor(DANGER));
            }
            card.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(4.0),
                ..default()
            })
            .with_children(|actions| {
                if let Some((_, up, down, remove)) = roster_actions {
                    lab_button(actions, assets, "↑", up, 42.0);
                    lab_button(actions, assets, "↓", down, 42.0);
                    lab_button(actions, assets, "Remove", remove, 78.0);
                } else if let Some(add) = add_action {
                    lab_button(actions, assets, "Add to roster", add, 132.0);
                } else if !ready {
                    if let RosterChoice::Custom(id) = choice {
                        lab_button(
                            actions,
                            assets,
                            "Edit in Creator",
                            LabAction::EditCustom(*id),
                            142.0,
                        );
                    }
                }
            });
        });
}

fn choice_record(
    choice: &RosterChoice,
    store: &CreationStore,
    presets: Option<&CreationPresetCatalog>,
) -> Option<(
    SavedCharacter,
    hex_assets::CreationLibraryFile,
    &'static str,
)> {
    match choice {
        RosterChoice::Custom(id) => store
            .file
            .characters
            .iter()
            .find(|character| character.id == *id)
            .cloned()
            .map(|character| (character, store.file.clone(), "Custom")),
        RosterChoice::Template(name) => {
            let presets = presets?;
            let library = presets.library_for(PresetAudience::HumanTemplate);
            presets
                .characters
                .iter()
                .find(|record| {
                    record.audience == PresetAudience::HumanTemplate
                        && record.key == format!("template-{name}")
                })
                .map(|record| (record.character.clone(), library, "Template"))
        }
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "creator coordinates are schema-bounded to 64 cells and miniature layout uses pixels"
)]
fn spawn_mini_lattice(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    character: &SavedCharacter,
    elements: Option<&ElementCatalog>,
) {
    let cell_width = 20.0;
    let cell_height = 23.0;
    parent
        .spawn(Node {
            width: Val::Px(102.0),
            height: Val::Px(76.0),
            position_type: PositionType::Relative,
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|canvas| {
            for cell in &character.cells {
                let x = 40.0 + (cell.q as f32 + cell.r as f32 * 0.5) * cell_width * 0.88;
                let y = 27.0 + cell.r as f32 * cell_height * 0.74;
                let (color, text) = match &cell.kind {
                    CreationCellKind::Gem(name) => (
                        elements
                            .map(|catalog| element_color(catalog.id(name), catalog))
                            .unwrap_or(Color::srgb(0.16, 0.45, 0.52)),
                        short_name(name),
                    ),
                    CreationCellKind::Fusion(name) => (FUSION_COLOR, short_name(name)),
                    CreationCellKind::Spell(_) => {
                        (Color::srgba(0.86, 0.80, 0.62, 0.94), "S".to_owned())
                    }
                    CreationCellKind::Blank => {
                        (Color::srgba(0.36, 0.38, 0.42, 0.88), "·".to_owned())
                    }
                };
                canvas
                    .spawn((
                        ImageNode::new(assets.hex_cell.clone()).with_color(color),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(x),
                            top: Val::Px(y),
                            width: Val::Px(cell_width),
                            height: Val::Px(cell_height),
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::Center,
                            ..default()
                        },
                    ))
                    .with_child((
                        Text::new(text),
                        TextFont {
                            font: assets.body.clone().into(),
                            ..TextFont::from_font_size(7.0)
                        },
                        TextColor(Color::BLACK),
                    ));
            }
        });
}

fn character_is_map_ready_optional(
    character: &SavedCharacter,
    library: &hex_assets::CreationLibraryFile,
    elements: Option<&ElementCatalog>,
    spells: Option<&SpellBook>,
) -> bool {
    elements.zip(spells).is_some_and(|(elements, spells)| {
        character_is_map_ready(character, library, elements, spells)
    })
}

fn spawn_fixture_selector(
    root: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    state: &CombatLabState,
) {
    root.spawn(panel())
        .insert(Node {
            width: Val::Percent(88.0),
            min_height: Val::Px(0.0),
            flex_grow: 1.0,
            ..panel_node()
        })
        .with_children(|fixture_panel| {
            fixture_panel.spawn(heading(assets, "fixed deterministic fixtures"));
            fixture_panel.spawn(blurb(
                assets,
                "Immutable map, seed, roster, AI, and placement. Local creations are never read.",
            ));
            fixture_panel.spawn((
                Name::new("Fixture Search"),
                EditableText {
                    max_characters: Some(48),
                    visible_width: Some(32.0),
                    ..EditableText::new(&state.fixture_filter)
                },
                TextFont {
                    font: assets.body.clone().into(),
                    ..TextFont::from_font_size(17.0)
                },
                TextColor(Color::WHITE),
                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.08)),
                Node {
                    width: Val::Percent(100.0),
                    min_height: Val::Px(42.0),
                    padding: UiRect::all(Val::Px(8.0)),
                    ..default()
                },
                FixtureFilter,
            ));
            fixture_panel
                .spawn((
                    ScrollArea,
                    Node {
                        min_height: Val::Px(0.0),
                        flex_grow: 1.0,
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(8.0),
                        overflow: Overflow::scroll_y(),
                        ..default()
                    },
                ))
                .with_children(|list| {
                    let filter = state.fixture_filter.to_lowercase();
                    for fixture in FIXTURES {
                        let searchable = format!(
                            "{} {} {} {} {} {}",
                            fixture.id,
                            fixture.name,
                            fixture.tags,
                            fixture.description,
                            fixture.map_seed,
                            fixture.roster
                        )
                        .to_lowercase();
                        if !filter.is_empty() && !searchable.contains(&filter) {
                            continue;
                        }
                        list.spawn(panel())
                            .insert(Node {
                                width: Val::Percent(100.0),
                                ..panel_node()
                            })
                            .with_children(|card| {
                                card.spawn(heading(assets, fixture.name));
                                card.spawn(fine(
                                    assets,
                                    format!("{} · {}", fixture.id, fixture.tags),
                                ));
                                card.spawn(fine(
                                    assets,
                                    format!("{} · {}", fixture.map_seed, fixture.roster),
                                ));
                                card.spawn(blurb(assets, fixture.description));
                                lab_button(
                                    card,
                                    assets,
                                    "Run Fixture",
                                    LabAction::StartFixture(fixture.id.to_owned()),
                                    150.0,
                                );
                            });
                    }
                });
        });
}

fn sync_fixture_filter(
    inputs: Query<&EditableText, (Changed<EditableText>, With<FixtureFilter>)>,
    mut state: ResMut<CombatLabState>,
) {
    for input in &inputs {
        let value = input.value().to_string();
        if state.fixture_filter != value {
            state.fixture_filter = value;
            // Do not rebuild while typing; focus and composition must survive.
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "launch construction freezes all content catalogs in one reducer"
)]
fn handle_lab_actions(
    clicked: Query<(&Interaction, &LabAction), Changed<Interaction>>,
    mut state: ResMut<CombatLabState>,
    store: Res<CreationStore>,
    scenarios: Option<Res<ScenarioLibrary>>,
    shipped_spell_file: Option<Res<SpellFile>>,
    base_lattice_file: Option<Res<LatticeFile>>,
    elements: Option<Res<ElementCatalog>>,
    substances: Option<Res<SubstanceTable>>,
    presets: Option<Res<CreationPresetCatalog>>,
    map_catalog: Option<Res<CombatLabMapCatalog>>,
    mut commands: Commands,
    mut next: ResMut<NextState<Screen>>,
) {
    for (interaction, action) in &clicked {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match action {
            LabAction::Tab(tab) => {
                state.tab = *tab;
            }
            LabAction::Back => next.set(Screen::Title),
            LabAction::SelectMap(map) => {
                state.map = *map;
            }
            LabAction::AddPlayerTemplate(name) => {
                if state.players.len() < MAX_ROSTER {
                    state.players.push(RosterChoice::Template(name.clone()));
                }
            }
            LabAction::AddHostileTemplate(name) => {
                if state.hostiles.len() < MAX_ROSTER {
                    state.hostiles.push(RosterChoice::Template(name.clone()));
                }
            }
            LabAction::AddPlayerCustom(id) => {
                if state.players.len() < MAX_ROSTER {
                    state.players.push(RosterChoice::Custom(*id));
                }
            }
            LabAction::AddHostileCustom(id) => {
                if state.hostiles.len() < MAX_ROSTER {
                    state.hostiles.push(RosterChoice::Custom(*id));
                }
            }
            LabAction::RemovePlayer(index) => remove_at(&mut state.players, *index),
            LabAction::RemoveHostile(index) => remove_at(&mut state.hostiles, *index),
            LabAction::MovePlayer(index, delta) => move_at(&mut state.players, *index, *delta),
            LabAction::MoveHostile(index, delta) => {
                move_at(&mut state.hostiles, *index, *delta);
            }
            LabAction::EditCustom(character) => {
                commands.insert_resource(CreatorEditRequest {
                    character: *character,
                });
                next.set(Screen::CharacterCreator);
            }
            LabAction::PrepareDeployment => {
                let Some(map_definition) = map_catalog
                    .as_deref()
                    .and_then(|catalog| catalog.get(state.map.stable_id()))
                else {
                    state.notice = format!(
                        "Packaged map definition {:?} is unavailable.",
                        state.map.stable_id()
                    );
                    state.bump();
                    continue;
                };
                let Some(library) = scenarios.as_deref() else {
                    state.notice = "Scenario catalog is still loading.".to_owned();
                    state.bump();
                    continue;
                };
                let Some(scenario) = scenario_named(library, &map_definition.scenario) else {
                    state.notice = "Selected sandbox map is not available.".to_owned();
                    state.bump();
                    continue;
                };
                let overlay = match build_creator_overlay(
                    &state.players,
                    &state.hostiles,
                    &store.file,
                    shipped_spell_file.as_deref(),
                    base_lattice_file.as_deref(),
                    elements.as_deref(),
                    substances.as_deref(),
                ) {
                    Ok(overlay) => overlay,
                    Err(error) => {
                        state.notice = error;
                        state.bump();
                        continue;
                    }
                };
                let encounter = sandbox_encounter(
                    state.map,
                    &state.players,
                    &state.hostiles,
                    Some(map_definition),
                );
                let resolved_seed = map_definition
                    .fixed_seed
                    .or(scenario.generation_seed)
                    .map(ResolvedMapSeed);
                commands.insert_resource(overlay);
                commands.insert_resource(DeploymentSession::new(
                    map_definition.clone(),
                    state.players.clone(),
                    state.hostiles.clone(),
                ));
                commands.insert_resource(GameplayPhase::Preparing);
                commands.insert_resource(CombatLabSession {
                    kind: CombatLabSessionKind::Sandbox,
                    return_to: if state.creator_origin {
                        Screen::CharacterCreator
                    } else {
                        Screen::CombatLab
                    },
                });
                commands.insert_resource(ScenarioToLoad {
                    scenario,
                    resolved_seed,
                    encounter_override: Some(encounter),
                });
                next.set(Screen::Loading);
            }
            LabAction::StartFixture(id) => {
                let Some(fixture) = FIXTURES.iter().find(|fixture| fixture.id == id) else {
                    state.notice = format!("Unknown fixture {id:?}.");
                    state.bump();
                    continue;
                };
                let Some(library) = scenarios.as_deref() else {
                    state.notice = "Scenario catalog is still loading.".to_owned();
                    state.bump();
                    continue;
                };
                let Some(scenario) = scenario_named(library, fixture.scenario) else {
                    state.notice = format!("Fixture scenario {:?} is missing.", fixture.scenario);
                    state.bump();
                    continue;
                };
                let resolved_seed = scenario.generation_seed.map(ResolvedMapSeed);
                let payload = match creator_fixture_payload(
                    id,
                    presets.as_deref(),
                    shipped_spell_file.as_deref(),
                    base_lattice_file.as_deref(),
                    elements.as_deref(),
                    substances.as_deref(),
                ) {
                    Ok(payload) => payload,
                    Err(error) => {
                        state.notice = format!("Fixture {id:?} is invalid: {error}");
                        state.bump();
                        continue;
                    }
                };
                let encounter_override = payload.map(|(overlay, encounter)| {
                    commands.insert_resource(overlay);
                    encounter
                });
                commands.insert_resource(CombatLabSession {
                    kind: CombatLabSessionKind::FixedFixture(id.clone()),
                    return_to: Screen::CombatLab,
                });
                commands.insert_resource(ScenarioToLoad {
                    scenario,
                    resolved_seed,
                    encounter_override,
                });
                next.set(Screen::Loading);
            }
        }
        state.bump();
    }
}

fn scenario_named(library: &ScenarioLibrary, name: &str) -> Option<Scenario> {
    library
        .scenarios
        .iter()
        .find(|scenario| scenario.name == name)
        .cloned()
}

fn build_creator_overlay(
    players: &[RosterChoice],
    hostiles: &[RosterChoice],
    library: &hex_assets::CreationLibraryFile,
    shipped_spells: Option<&SpellFile>,
    base_lattices: Option<&LatticeFile>,
    elements: Option<&ElementCatalog>,
    substances: Option<&SubstanceTable>,
) -> Result<CreatorContentOverlay, String> {
    let (Some(shipped_spells), Some(base_lattices), Some(elements), Some(substances)) =
        (shipped_spells, base_lattices, elements, substances)
    else {
        return Err("Content catalogs are still loading.".to_owned());
    };
    let selected_custom: Vec<&SavedCharacter> = players
        .iter()
        .chain(hostiles)
        .filter_map(|choice| match choice {
            RosterChoice::Custom(id) => library.characters.iter().find(|saved| saved.id == *id),
            RosterChoice::Template(_) => None,
        })
        .collect();
    let used_spell_ids: std::collections::BTreeSet<_> = selected_custom
        .iter()
        .flat_map(|character| character.custom_spell_references())
        .collect();
    let custom_spells = library
        .spells
        .iter()
        .filter(|spell| used_spell_ids.contains(&spell.id))
        .cloned()
        .collect::<Vec<_>>();
    for saved in &custom_spells {
        if !hex_assets::creator_spell_issues(saved, elements).is_empty()
            || hex_combat::creator_spell_deployability(&saved.spell).is_err()
        {
            return Err(format!("Custom spell {:?} is not Map-ready.", saved.name));
        }
    }
    let spell_file = combined_spell_file(shipped_spells, custom_spells)?;
    let spell_book = SpellBook::from_file(&spell_file);
    let content = ContentIndex::build(elements, &spell_book, substances).map_err(|errors| {
        errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    })?;

    let mut lattice_file = base_lattices.clone();
    for character in selected_custom {
        let issues =
            hex_assets::creator_character_issues(character, library, elements, &spell_book);
        if !issues.is_empty() {
            return Err(format!(
                "Character {:?} is not Map-ready: {}",
                character.name,
                issues.join("; ")
            ));
        }
        lattice_file
            .archetypes
            .extend(character_lattice_file(character, library)?.archetypes);
    }
    let lattices =
        LatticeLibrary::build(&lattice_file, elements, &spell_book).map_err(|errors| {
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        })?;
    let tables = content.tables(elements);
    for character in players
        .iter()
        .chain(hostiles)
        .filter_map(|choice| match choice {
            RosterChoice::Custom(id) => library.characters.iter().find(|saved| saved.id == *id),
            RosterChoice::Template(_) => None,
        })
    {
        let key = character_runtime_key(character.id);
        let archetype = lattices
            .get(&key)
            .ok_or_else(|| format!("Resolved character {:?} disappeared.", character.name))?;
        let state = hex_lattice::LatticeState::new(&archetype.spec, &archetype.stats);
        let has_fresh_cast = archetype.spec.cells().any(|(coord, kind)| {
            matches!(kind, hex_lattice::CellKind::Spell { .. })
                && hex_lattice::castable(&archetype.spec, &state, coord, &tables).is_ok()
        });
        if !has_fresh_cast {
            return Err(format!(
                "Character {:?} needs at least one spell castable from a fresh lattice.",
                character.name
            ));
        }
    }
    Ok(CreatorContentOverlay {
        spells: spell_book,
        content,
        lattices,
        display_names: players
            .iter()
            .chain(hostiles)
            .filter_map(|choice| match choice {
                RosterChoice::Custom(id) => library
                    .characters
                    .iter()
                    .find(|character| character.id == *id)
                    .map(|character| (character_runtime_key(character.id), character.name.clone())),
                RosterChoice::Template(_) => None,
            })
            .collect(),
    })
}

fn sandbox_encounter(
    map: SandboxMap,
    players: &[RosterChoice],
    hostiles: &[RosterChoice],
    definition: Option<&CombatLabMapDefinition>,
) -> Encounter {
    let (player_placement, hostile_placement) = definition.map_or_else(
        || match map {
            SandboxMap::Flat => (
                EncounterPlacement::Formation {
                    center: FormationCenter::Fixed(hex_assets::CubeCoord { x: -2, y: 0, z: 2 }),
                    spread: 3,
                },
                EncounterPlacement::Formation {
                    center: FormationCenter::Fixed(hex_assets::CubeCoord { x: 2, y: 0, z: -2 }),
                    spread: 3,
                },
            ),
            SandboxMap::Crossing => (
                EncounterPlacement::Formation {
                    center: FormationCenter::Fixed(hex_assets::CubeCoord { x: 0, y: 8, z: -8 }),
                    spread: 3,
                },
                EncounterPlacement::Formation {
                    center: FormationCenter::Fixed(hex_assets::CubeCoord { x: 0, y: -8, z: 8 }),
                    spread: 3,
                },
            ),
            SandboxMap::Hills => (
                EncounterPlacement::Formation {
                    center: FormationCenter::Anchor("party_start".to_owned()),
                    spread: 3,
                },
                EncounterPlacement::Formation {
                    center: FormationCenter::Anchor("hostile_start".to_owned()),
                    spread: 3,
                },
            ),
        },
        |definition| {
            (
                deployment_region_placement(&definition.player_region),
                deployment_region_placement(&definition.hostile_region),
            )
        },
    );
    Encounter {
        name: format!(
            "Creator Sandbox · {}",
            definition.map_or_else(|| map.label(), |record| record.display_name.as_str())
        ),
        rosters: vec![
            Roster {
                faction: EncounterFaction::Player,
                placement: player_placement,
                units: players
                    .iter()
                    .map(|choice| roster_entry(choice, None))
                    .collect(),
            },
            Roster {
                faction: EncounterFaction::Hostile,
                placement: hostile_placement,
                units: hostiles
                    .iter()
                    .map(|choice| roster_entry(choice, None))
                    .collect(),
            },
        ],
    }
}

fn deployment_region_placement(region: &CombatLabDeploymentRegion) -> EncounterPlacement {
    let center = match &region.center {
        CombatLabRegionCenter::Fixed(coord) => FormationCenter::Fixed(*coord),
        CombatLabRegionCenter::Anchor(anchor) => FormationCenter::Anchor(anchor.clone()),
    };
    EncounterPlacement::Formation {
        center,
        spread: region.radius,
    }
}

fn roster_entry(choice: &RosterChoice, placement: Option<EncounterPlacement>) -> RosterEntry {
    RosterEntry {
        archetype: match choice {
            RosterChoice::Template(name) => name.clone(),
            RosterChoice::Custom(id) => character_runtime_key(*id),
        },
        placement,
        ai_profile: None,
        ai_group: None,
    }
}

fn choice_name(choice: &RosterChoice, store: &CreationStore) -> String {
    match choice {
        RosterChoice::Template(name) => format!("{name} · template"),
        RosterChoice::Custom(id) => store
            .file
            .characters
            .iter()
            .find(|saved| saved.id == *id)
            .map_or_else(|| format!("missing #{}", id.0), |saved| saved.name.clone()),
    }
}

fn character_is_map_ready(
    character: &SavedCharacter,
    library: &hex_assets::CreationLibraryFile,
    elements: &ElementCatalog,
    spells: &SpellBook,
) -> bool {
    if !hex_assets::creator_character_issues(character, library, elements, spells).is_empty() {
        return false;
    }
    if !character
        .cells
        .iter()
        .any(|cell| matches!(cell.kind, CreationCellKind::Spell(_)))
    {
        return false;
    }
    character.cells.iter().all(|cell| match &cell.kind {
        CreationCellKind::Spell(SpellReference::Shipped(name)) => spells
            .id(name)
            .and_then(|id| spells.spell(id))
            .is_some_and(|spell| {
                matches!(
                    spell.targeting.shape,
                    hex_assets::TargetShape::SelfCast | hex_assets::TargetShape::Single
                ) && hex_combat::delivers_anything(spell)
            }),
        CreationCellKind::Spell(SpellReference::Custom(id)) => library
            .spells
            .iter()
            .find(|spell| spell.id == *id)
            .is_some_and(|spell| {
                hex_assets::creator_spell_issues(spell, elements).is_empty()
                    && hex_combat::creator_spell_deployability(&spell.spell).is_ok()
            }),
        _ => true,
    })
}

fn remove_at<T>(items: &mut Vec<T>, index: usize) {
    if index < items.len() {
        items.remove(index);
    }
}

fn move_at<T>(items: &mut [T], index: usize, delta: i8) {
    let other = if delta < 0 {
        index.saturating_sub(1)
    } else {
        index.saturating_add(1)
    };
    if index < items.len() && other < items.len() {
        items.swap(index, other);
    }
}

fn placements_complete_exact(placements: &[Option<TilePos>], roster_len: usize) -> bool {
    placements.len() == roster_len
        && !placements.is_empty()
        && placements.iter().all(Option::is_some)
}

type DeploymentTileQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static TilePos,
        &'static HexSpan,
        &'static SubstanceId,
        &'static Headroom,
    ),
    With<HexTile>,
>;

#[expect(
    clippy::too_many_arguments,
    reason = "deployment projects live terrain, actors, HUD, and the frozen launch together"
)]
fn enter_deployment(
    mut commands: Commands,
    mut session: Option<ResMut<DeploymentSession>>,
    mut phase: ResMut<GameplayPhase>,
    tiles: DeploymentTileQuery,
    table: Res<SubstanceTable>,
    blockers: Option<Res<TraversalBlockers>>,
    anchors: Option<Res<MapAnchors>>,
    game_assets: Res<GameAssets>,
    ui_assets: Res<UiAssets>,
    store: Res<CreationStore>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut hidden: Query<
        (Entity, &mut Visibility),
        Or<(
            With<Faction>,
            With<crate::readouts::HudElement>,
            With<hex_units::UnitRing>,
        )>,
    >,
) {
    let Some(session) = session.as_deref_mut() else {
        *phase = GameplayPhase::Active;
        return;
    };
    *phase = GameplayPhase::Deployment;
    for (entity, mut visibility) in &mut hidden {
        *visibility = Visibility::Hidden;
        commands.entity(entity).insert(DeploymentHidden);
    }

    let footing = deployment_footing(&tiles, &table, blockers.as_deref());
    let Some(player_center) = deployment_center(
        &session.map_definition.player_region,
        &footing,
        anchors.as_deref(),
    ) else {
        session.notice = "Player deployment region could not resolve on this terrain.".to_owned();
        spawn_deployment_hud(&mut commands, &ui_assets, session, &store);
        return;
    };
    let Some(hostile_center) = deployment_center(
        &session.map_definition.hostile_region,
        &footing,
        anchors.as_deref(),
    ) else {
        session.notice = "Hostile deployment region could not resolve on this terrain.".to_owned();
        spawn_deployment_hud(&mut commands, &ui_assets, session, &store);
        return;
    };
    session.player_surfaces = ordered_deployment_surfaces(
        player_center,
        &footing,
        session.map_definition.player_region.radius,
    );
    session.hostile_surfaces = ordered_deployment_surfaces(
        hostile_center,
        &footing,
        session.map_definition.hostile_region.radius,
    );

    let player_material = materials.add(deployment_material(Color::srgba(0.20, 0.68, 0.98, 0.58)));
    let hostile_material = materials.add(deployment_material(Color::srgba(0.94, 0.30, 0.24, 0.58)));
    for (player, positions, material) in [
        (true, session.player_surfaces.as_slice(), &player_material),
        (
            false,
            session.hostile_surfaces.as_slice(),
            &hostile_material,
        ),
    ] {
        for pos in positions {
            let Some(standing) = footing.at(*pos) else {
                continue;
            };
            commands.spawn((
                Name::new(if player {
                    "Player Deployment Surface"
                } else {
                    "Hostile Deployment Surface"
                }),
                DeploymentWorldEntity,
                DeploymentSurface { pos: *pos, player },
                Mesh3d(game_assets.hex_tile.clone()),
                MeshMaterial3d(material.clone()),
                Transform {
                    translation: pos.coord.to_world(standing.span.top + 0.035),
                    scale: Vec3::new(0.88, 0.035, 0.88),
                    ..default()
                },
            ));
        }
    }
    spawn_deployment_hud(&mut commands, &ui_assets, session, &store);
}

fn deployment_footing(
    tiles: &DeploymentTileQuery,
    table: &SubstanceTable,
    blockers: Option<&TraversalBlockers>,
) -> Footing {
    Footing::from_tiles(
        tiles.iter(),
        table,
        Body::new(TraversalProfile::WALKER),
        blockers,
    )
}

fn ordered_deployment_surfaces(
    center: hex_units::Standing,
    footing: &Footing,
    radius: u32,
) -> Vec<TilePos> {
    let reach = Reach::from(center, footing, Some(radius));
    let mut positions = reach
        .surfaces()
        .map(|standing| standing.pos)
        .collect::<Vec<_>>();
    positions.sort_by_key(|position| (reach.cost(*position).unwrap_or(u32::MAX), *position));
    positions
}

fn deployment_center(
    region: &CombatLabDeploymentRegion,
    footing: &Footing,
    anchors: Option<&MapAnchors>,
) -> Option<hex_units::Standing> {
    match &region.center {
        CombatLabRegionCenter::Fixed(coord) => {
            footing.ground(HexCoord::new_cubic(coord.x, coord.y, coord.z))
        }
        CombatLabRegionCenter::Anchor(anchor) => anchors
            .and_then(|anchors| anchors.get(&MapAnchorId::new(anchor.clone())))
            .and_then(|pos| footing.at(pos)),
    }
}

fn deployment_material(color: Color) -> StandardMaterial {
    StandardMaterial {
        base_color: color,
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        depth_bias: 18.0,
        ..default()
    }
}

fn spawn_deployment_hud(
    commands: &mut Commands,
    assets: &UiAssets,
    session: &DeploymentSession,
    store: &CreationStore,
) {
    commands
        .spawn((
            Name::new("Combat Lab Deployment HUD"),
            DeploymentHud,
            DeploymentWorldEntity,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(22.0),
                right: Val::Px(22.0),
                top: Val::Px(18.0),
                min_height: Val::Px(126.0),
                padding: UiRect::all(Val::Px(13.0)),
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(18.0),
                border_radius: BorderRadius::all(Val::Px(7.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.015, 0.022, 0.035, 0.94)),
            BorderColor::all(Color::srgba(0.93, 0.79, 0.46, 0.52)),
        ))
        .with_children(|hud| {
            hud.spawn(Node {
                width: Val::Px(300.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(3.0),
                ..default()
            })
            .with_children(|summary| {
                summary.spawn(heading(
                    assets,
                    format!("DEPLOY · {}", session.map_definition.display_name),
                ));
                summary.spawn(blurb(assets, session.notice.clone()));
                summary.spawn(fine(
                    assets,
                    "Blue = Player region · Red = Hostile region · labels remain in this HUD",
                ));
            });
            for (title, roster, placements) in [
                (
                    "PLAYER",
                    session.players.as_slice(),
                    session.player_placements.as_slice(),
                ),
                (
                    "HOSTILE",
                    session.hostiles.as_slice(),
                    session.hostile_placements.as_slice(),
                ),
            ] {
                hud.spawn(Node {
                    width: Val::Px(245.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(2.0),
                    ..default()
                })
                .with_children(|side| {
                    side.spawn(fine(assets, title));
                    for (index, choice) in roster.iter().enumerate() {
                        let placement = placements.get(index).copied().flatten();
                        side.spawn(fine(
                            assets,
                            format!(
                                "{}. {} · {}",
                                index + 1,
                                choice_name(choice, store),
                                placement.map_or_else(
                                    || "choose surface".to_owned(),
                                    |pos| format!(
                                        "({}, {}, {}) · elevation {}",
                                        pos.coord.x(),
                                        pos.coord.y(),
                                        pos.coord.z(),
                                        pos.level
                                    )
                                )
                            ),
                        ));
                    }
                });
            }
            hud.spawn(Node {
                width: Val::Px(170.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(5.0),
                ..default()
            })
            .with_children(|actions| {
                deployment_button(actions, assets, "Undo", DeploymentAction::Undo);
                deployment_button(
                    actions,
                    assets,
                    "Clear Player",
                    DeploymentAction::ClearPlayer,
                );
                deployment_button(
                    actions,
                    assets,
                    "Clear Hostile",
                    DeploymentAction::ClearHostile,
                );
                deployment_button(
                    actions,
                    assets,
                    "Deterministic Auto-place",
                    DeploymentAction::AutoPlace,
                );
                deployment_button(actions, assets, "Back to Setup", DeploymentAction::Back);
                if session.complete() {
                    deployment_button(
                        actions,
                        assets,
                        "Start Combat",
                        DeploymentAction::StartCombat,
                    );
                }
            });
        });
}

fn deployment_button(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    text: &'static str,
    action: DeploymentAction,
) {
    parent
        .spawn((row_button(text, 166.0), action))
        .with_child(label(assets, text));
}

fn on_deployment_surface_clicked(
    click: On<Pointer<Click>>,
    surfaces: Query<&DeploymentSurface>,
    mut session: Option<ResMut<DeploymentSession>>,
    hud: Query<Entity, With<DeploymentHud>>,
    mut commands: Commands,
    assets: Res<UiAssets>,
    store: Res<CreationStore>,
) {
    if click.button != PointerButton::Primary {
        return;
    }
    let Ok(surface) = surfaces.get(click.event_target()) else {
        return;
    };
    let Some(session) = session.as_deref_mut() else {
        return;
    };
    if surface.player != session.active_player {
        session.notice = format!(
            "Place the current {} unit inside its labeled region.",
            if session.active_player {
                "Player"
            } else {
                "Hostile"
            }
        );
        rebuild_deployment_hud(&mut commands, &hud, &assets, session, &store);
        return;
    }
    let occupied = session
        .player_placements
        .iter()
        .chain(&session.hostile_placements)
        .any(|placement| *placement == Some(surface.pos));
    if occupied {
        session.notice = "That exact surface is already occupied.".to_owned();
        rebuild_deployment_hud(&mut commands, &hud, &assets, session, &store);
        return;
    }
    let placements = if session.active_player {
        &mut session.player_placements
    } else {
        &mut session.hostile_placements
    };
    let previous = placements.get(session.active_index).copied().flatten();
    if let Some(placement) = placements.get_mut(session.active_index) {
        *placement = Some(surface.pos);
        session
            .undo
            .push((session.active_player, session.active_index, previous));
    }
    advance_deployment_cursor(session);
    rebuild_deployment_hud(&mut commands, &hud, &assets, session, &store);
}

fn advance_deployment_cursor(session: &mut DeploymentSession) {
    if let Some(index) = session.player_placements.iter().position(Option::is_none) {
        session.active_player = true;
        session.active_index = index;
        session.notice = format!(
            "Select a highlighted Player surface for unit {}.",
            index + 1
        );
    } else if let Some(index) = session.hostile_placements.iter().position(Option::is_none) {
        session.active_player = false;
        session.active_index = index;
        session.notice = format!(
            "Select a highlighted Hostile surface for unit {}.",
            index + 1
        );
    } else {
        session.notice = "Deployment complete. Start Combat or reposition with Undo.".to_owned();
    }
}

fn rebuild_deployment_hud(
    commands: &mut Commands,
    roots: &Query<Entity, With<DeploymentHud>>,
    assets: &UiAssets,
    session: &DeploymentSession,
    store: &CreationStore,
) {
    for root in roots {
        commands.entity(root).despawn();
    }
    spawn_deployment_hud(commands, assets, session, store);
}

#[derive(SystemParam)]
struct DeploymentRuntime<'w, 's> {
    tiles: DeploymentTileQuery<'w, 's>,
    table: Option<Res<'w, SubstanceTable>>,
    blockers: Option<Res<'w, TraversalBlockers>>,
    units: Query<
        'w,
        's,
        (
            Entity,
            &'static Faction,
            &'static mut StandsOn,
            &'static mut Transform,
        ),
    >,
    hidden_presentation: Query<'w, 's, (Entity, &'static mut Visibility), With<DeploymentHidden>>,
    world_entities: Query<'w, 's, Entity, With<DeploymentWorldEntity>>,
    hud: Query<'w, 's, Entity, With<DeploymentHud>>,
    encounter: Option<ResMut<'w, Encounter>>,
    active: Option<ResMut<'w, crate::scenarios::ActiveScenario>>,
    lab: Option<Res<'w, CombatLabSession>>,
}

fn handle_deployment_actions(
    clicked: Query<(&Interaction, &DeploymentAction), Changed<Interaction>>,
    mut session: Option<ResMut<DeploymentSession>>,
    mut phase: ResMut<GameplayPhase>,
    mut runtime: DeploymentRuntime,
    assets: Res<UiAssets>,
    store: Res<CreationStore>,
    mut commands: Commands,
    mut next: ResMut<NextState<Screen>>,
) {
    let Some(session) = session.as_deref_mut() else {
        return;
    };
    for (interaction, action) in &clicked {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match action {
            DeploymentAction::Undo => {
                if let Some((player, index, previous)) = session.undo.pop() {
                    let placements = if player {
                        &mut session.player_placements
                    } else {
                        &mut session.hostile_placements
                    };
                    if let Some(placement) = placements.get_mut(index) {
                        *placement = previous;
                    }
                    session.active_player = player;
                    session.active_index = index;
                    session.notice = format!(
                        "Reposition {} unit {}.",
                        if player { "Player" } else { "Hostile" },
                        index + 1
                    );
                }
            }
            DeploymentAction::ClearPlayer => {
                session.player_placements.fill(None);
                session.undo.clear();
                session.active_player = true;
                session.active_index = 0;
                session.notice = "Player placements cleared.".to_owned();
            }
            DeploymentAction::ClearHostile => {
                session.hostile_placements.fill(None);
                session.undo.clear();
                session.active_player = false;
                session.active_index = 0;
                session.notice = "Hostile placements cleared.".to_owned();
            }
            DeploymentAction::AutoPlace => {
                let mut used = std::collections::BTreeSet::new();
                for (placement, surface) in session
                    .player_placements
                    .iter_mut()
                    .zip(&session.player_surfaces)
                {
                    *placement = Some(*surface);
                    used.insert(*surface);
                }
                let hostile = session
                    .hostile_surfaces
                    .iter()
                    .filter(|surface| !used.contains(surface))
                    .copied()
                    .take(session.hostile_placements.len())
                    .collect::<Vec<_>>();
                for (placement, surface) in session.hostile_placements.iter_mut().zip(hostile) {
                    *placement = Some(surface);
                }
                session.undo.clear();
                advance_deployment_cursor(session);
            }
            DeploymentAction::Back => {
                *phase = GameplayPhase::Active;
                next.set(
                    runtime
                        .lab
                        .as_deref()
                        .map_or(Screen::CombatLab, |session| session.return_to),
                );
                commands.remove_resource::<DeploymentSession>();
                continue;
            }
            DeploymentAction::StartCombat => {
                let Some(table) = runtime.table.as_deref() else {
                    session.notice = "Terrain rules are still loading.".to_owned();
                    continue;
                };
                if !session.complete() {
                    session.notice = "Every roster entry needs one unique surface.".to_owned();
                    continue;
                }
                let footing =
                    deployment_footing(&runtime.tiles, table, runtime.blockers.as_deref());
                let mut players = runtime
                    .units
                    .iter_mut()
                    .filter(|(_, faction, _, _)| **faction == Faction::Player)
                    .map(|(entity, _, _, _)| entity)
                    .collect::<Vec<_>>();
                let mut hostiles = runtime
                    .units
                    .iter_mut()
                    .filter(|(_, faction, _, _)| **faction == Faction::Hostile)
                    .map(|(entity, _, _, _)| entity)
                    .collect::<Vec<_>>();
                players.sort_by_key(|entity| entity.index());
                hostiles.sort_by_key(|entity| entity.index());
                for (entities, placements) in [
                    (players.as_slice(), session.player_placements.as_slice()),
                    (hostiles.as_slice(), session.hostile_placements.as_slice()),
                ] {
                    for (entity, placement) in entities.iter().zip(placements) {
                        let Some(pos) = placement else { continue };
                        let Some(standing) = footing.at(*pos) else {
                            session.notice =
                                format!("Selected surface {pos:?} is no longer valid footing.");
                            continue;
                        };
                        if let Ok((_, _, mut on, mut transform)) = runtime.units.get_mut(*entity) {
                            on.0 = standing;
                            transform.translation = standing.world_position();
                        }
                    }
                }
                let exact = exact_deployed_encounter(session);
                if let Some(encounter) = runtime.encounter.as_deref_mut() {
                    *encounter = exact.clone();
                }
                if let Some(active) = runtime.active.as_deref_mut() {
                    active.0.encounter_override = Some(exact);
                }
                for (entity, mut visibility) in &mut runtime.hidden_presentation {
                    *visibility = Visibility::Inherited;
                    commands.entity(entity).remove::<DeploymentHidden>();
                }
                for entity in &runtime.world_entities {
                    commands.entity(entity).despawn();
                }
                commands.remove_resource::<DeploymentSession>();
                *phase = GameplayPhase::Active;
                continue;
            }
        }
        rebuild_deployment_hud(&mut commands, &runtime.hud, &assets, session, &store);
    }
}

fn exact_deployed_encounter(session: &DeploymentSession) -> Encounter {
    Encounter {
        name: format!("Creator Sandbox · {}", session.map_definition.display_name),
        rosters: vec![
            Roster {
                faction: EncounterFaction::Player,
                placement: EncounterPlacement::Formation {
                    center: FormationCenter::Fixed(hex_assets::CubeCoord { x: 0, y: 0, z: 0 }),
                    spread: 0,
                },
                units: session
                    .players
                    .iter()
                    .zip(&session.player_placements)
                    .map(|(choice, placement)| {
                        roster_entry(choice, placement.map(EncounterPlacement::Surface))
                    })
                    .collect(),
            },
            Roster {
                faction: EncounterFaction::Hostile,
                placement: EncounterPlacement::Formation {
                    center: FormationCenter::Fixed(hex_assets::CubeCoord { x: 0, y: 0, z: 0 }),
                    spread: 0,
                },
                units: session
                    .hostiles
                    .iter()
                    .zip(&session.hostile_placements)
                    .map(|(choice, placement)| {
                        roster_entry(choice, placement.map(EncounterPlacement::Surface))
                    })
                    .collect(),
            },
        ],
    }
}

fn clear_deployment_world(
    mut commands: Commands,
    entities: Query<Entity, With<DeploymentWorldEntity>>,
) {
    for entity in &entities {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<DeploymentSession>();
}

/// Reapplies a frozen combined namespace after normal hot-reload builders run.
pub(crate) fn apply_creator_content_overlay(
    mut commands: Commands,
    overlay: Option<Res<CreatorContentOverlay>>,
) {
    let Some(overlay) = overlay else { return };
    commands.insert_resource(overlay.spells.clone());
    commands.insert_resource(overlay.content.clone());
    commands.insert_resource(overlay.lattices.clone());
}

/// Restores the base namespace after a creator session froze combined ids.
pub(crate) fn restore_shipped_content(
    commands: &mut Commands,
    spell_file: Option<&SpellFile>,
    lattice_file: Option<&LatticeFile>,
    elements: Option<&ElementCatalog>,
    substances: Option<&SubstanceTable>,
) {
    commands.remove_resource::<CreatorContentOverlay>();
    let (Some(spell_file), Some(lattice_file), Some(elements), Some(substances)) =
        (spell_file, lattice_file, elements, substances)
    else {
        return;
    };
    let spells = SpellBook::from_file(spell_file);
    let Ok(content) = ContentIndex::build(elements, &spells, substances) else {
        return;
    };
    let Ok(lattices) = LatticeLibrary::build(lattice_file, elements, &spells) else {
        return;
    };
    commands.insert_resource(spells);
    commands.insert_resource(content);
    commands.insert_resource(lattices);
}

fn apply_creator_display_names(
    mut commands: Commands,
    overlay: Option<Res<CreatorContentOverlay>>,
    mut units: Query<(Entity, &hex_units::Archetype, &mut Name)>,
) {
    let Some(overlay) = overlay else { return };
    for (entity, archetype, mut name) in &mut units {
        if let Some(display) = overlay.display_names.get(&archetype.0) {
            name.set(display.clone());
            commands
                .entity(entity)
                .insert(CreatorDisplayName(display.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roster_ordering_and_cap_helpers_are_deterministic() {
        let mut roster = vec![1, 2, 3];
        move_at(&mut roster, 2, -1);
        assert_eq!(roster, vec![1, 3, 2]);
        remove_at(&mut roster, 1);
        assert_eq!(roster, vec![1, 2]);
    }

    #[test]
    fn exact_deployment_is_complete_only_when_every_unit_has_a_surface() {
        let first = TilePos::new(HexCoord::ORIGIN, 2);
        let second = TilePos::new(HexCoord::new_cubic(1, -1, 0), 5);
        let mut placements = vec![Some(first), None];
        assert!(!placements_complete_exact(&placements, 2));
        *placements.get_mut(1).expect("second placement") = Some(second);
        assert!(placements_complete_exact(&placements, 2));
    }

    #[test]
    fn fixture_ids_are_unique_and_stable() {
        let ids: std::collections::BTreeSet<_> =
            FIXTURES.iter().map(|fixture| fixture.id).collect();
        assert_eq!(ids.len(), FIXTURES.len());
        assert!(ids.contains("ability-lab"));
        assert!(ids.contains("creator-spell-matrix"));
    }
}
