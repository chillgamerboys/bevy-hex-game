//! Human sandbox composition and scalable deterministic fixture selection.

use std::collections::BTreeMap;

use bevy::prelude::*;
use bevy::text::EditableText;
use bevy::ui_widgets::ScrollArea;
use hex_assets::{
    character_lattice_file, character_runtime_key, combined_spell_file, ContentIndex,
    CreationCellKind, CreationPresetCatalog, CustomCharacterId, ElementCatalog, Encounter,
    EncounterFaction, EncounterPlacement, FormationCenter, LatticeFile, LatticeLibrary,
    PresetAudience, Roster, RosterEntry, SavedCharacter, Scenario, ScenarioLibrary, SpellBook,
    SpellFile, SpellReference, SubstanceTable,
};
use hex_core::{GameplaySetup, ResolvedMapSeed, Screen};

use crate::creation_store::CreationStore;
use crate::menus::widgets::{
    blurb, display, fine, heading, label, panel, panel_node, row_button, UiAssets, DANGER,
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

    const fn label(self) -> &'static str {
        match self {
            Self::Flat => "Flat Arena",
            Self::Crossing => "The Crossing",
            Self::Hills => "Procedural Hills",
        }
    }

    const fn scenario_name(self) -> &'static str {
        match self {
            Self::Flat => "Ability Lab",
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
    deployment: bool,
    player_placements: Vec<Option<u8>>,
    hostile_placements: Vec<Option<u8>>,
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
            deployment: false,
            player_placements: Vec::new(),
            hostile_placements: Vec::new(),
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
    PrepareDeployment,
    AutoPlace,
    PlacePlayer(usize, u8),
    PlaceHostile(usize, u8),
    BackToSetup,
    StartSandbox,
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
}

const FIXTURES: [FixtureDefinition; 4] = [
    FixtureDefinition {
        id: "ability-lab",
        name: "Ability Lab",
        tags: "aiming reveal restore revival",
        description: "A flat 2v1 for aiming, friendly damage, reveal, restoration, and revival.",
        scenario: "Ability Lab",
    },
    FixtureDefinition {
        id: "raider-mirror",
        name: "Raider Mirror",
        tags: "identity defense enchantment",
        description: "Same archetype on both sides, with deterministic defensive enchantments.",
        scenario: "Raider Mirror",
    },
    FixtureDefinition {
        id: "creator-spell-matrix",
        name: "Creator Spell Matrix",
        tags: "creator disable burn reveal restore defense",
        description: "Creator-format spell delivery against the flat deterministic roster.",
        scenario: "Ability Lab",
    },
    FixtureDefinition {
        id: "creator-roster-matrix",
        name: "Creator Roster Matrix",
        tags: "creator roster selection ordering",
        description: "Mixed roster selection, stable unit ordering, and multi-unit combat.",
        scenario: "Ability Lab",
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
            apply_creator_display_names.in_set(GameplaySetup::Restore),
        )
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
    if let Some(request) = request {
        state.tab = LabTab::Sandbox;
        state.players = vec![RosterChoice::Custom(request.character)];
        state.deployment = false;
        state.player_placements.clear();
        state.hostile_placements.clear();
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
) {
    spawn_lab_ui(
        &mut commands,
        &assets,
        &state,
        &store,
        elements.as_deref(),
        spells.as_deref(),
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
                    if state.deployment {
                        spawn_deployment(root, assets, state, store);
                    } else {
                        spawn_sandbox_setup(root, assets, state, store, elements, spells);
                    }
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
            .with_children(|maps| {
                maps.spawn(heading(assets, "map"));
                for map in SandboxMap::ALL {
                    lab_button(maps, assets, map.label(), LabAction::SelectMap(map), 250.0);
                }
                maps.spawn(fine(
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
                rosters.spawn(heading(assets, format!("{} · rosters", state.map.label())));
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
                        "Prepare Deployment",
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
            for (index, choice) in roster.iter().enumerate() {
                column.spawn(fine(
                    assets,
                    format!("{}. {}", index + 1, choice_name(choice, store)),
                ));
                column
                    .spawn(Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(4.0),
                        ..default()
                    })
                    .with_children(|row| {
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
                        lab_button(row, assets, "↑", up, 45.0);
                        lab_button(row, assets, "↓", down, 45.0);
                        lab_button(row, assets, "Remove", remove, 82.0);
                    });
            }
            if roster.len() < MAX_ROSTER {
                column.spawn(fine(assets, "Packaged templates"));
                for template in ["wolf", "raider", "hedge-mage"] {
                    lab_button(
                        column,
                        assets,
                        format!("+ {template}"),
                        if player {
                            LabAction::AddPlayerTemplate(template.to_owned())
                        } else {
                            LabAction::AddHostileTemplate(template.to_owned())
                        },
                        210.0,
                    );
                }
                column.spawn(fine(assets, "Saved Map-ready characters"));
                for character in &store.file.characters {
                    let ready = elements.zip(spells).is_some_and(|(elements, spells)| {
                        character_is_map_ready(character, &store.file, elements, spells)
                    });
                    if ready {
                        lab_button(
                            column,
                            assets,
                            format!("+ {}", character.name),
                            if player {
                                LabAction::AddPlayerCustom(character.id)
                            } else {
                                LabAction::AddHostileCustom(character.id)
                            },
                            210.0,
                        );
                    }
                }
            }
        });
}

fn spawn_deployment(
    root: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    state: &CombatLabState,
    store: &CreationStore,
) {
    root.spawn(panel())
        .insert(Node {
            width: Val::Percent(90.0),
            min_height: Val::Px(0.0),
            flex_grow: 1.0,
            ..panel_node()
        })
        .with_children(|deployment| {
        deployment.spawn(heading(
            assets,
            format!("Deployment · {}", state.map.label()),
        ));
        deployment.spawn(blurb(
            assets,
            "Deployment regions are deterministic. Auto-place fills valid, unique surfaces in roster order.",
        ));
        deployment.spawn(Node {
            min_height: Val::Px(0.0),
            flex_grow: 1.0,
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(24.0),
            ..default()
        })
        .with_children(|sides| {
            for (title, roster, placements, player) in [
                (
                    "Player region",
                    state.players.as_slice(),
                    state.player_placements.as_slice(),
                    true,
                ),
                (
                    "Hostile region",
                    state.hostiles.as_slice(),
                    state.hostile_placements.as_slice(),
                    false,
                ),
            ] {
                sides
                    .spawn(panel())
                    .insert(Node {
                        min_width: Val::Px(0.0),
                        flex_grow: 1.0,
                        ..panel_node()
                    })
                    .with_children(|side| {
                    side.spawn(heading(assets, title));
                    for (index, choice) in roster.iter().enumerate() {
                        let selected = placements.get(index).copied().flatten();
                        side.spawn(fine(
                            assets,
                            format!(
                                "{} · {}",
                                choice_name(choice, store),
                                selected.map_or_else(
                                    || "choose a surface".to_owned(),
                                    |slot| format!("surface {}", slot + 1)
                                )
                            ),
                        ));
                        side.spawn(Node {
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(3.0),
                            flex_wrap: FlexWrap::Wrap,
                            ..default()
                        })
                        .with_children(|slots| {
                            for slot in 0_u8..6 {
                                lab_button(
                                    slots,
                                    assets,
                                    format!("{}", slot + 1),
                                    if player {
                                        LabAction::PlacePlayer(index, slot)
                                    } else {
                                        LabAction::PlaceHostile(index, slot)
                                    },
                                    42.0,
                                );
                            }
                        });
                    }
                });
            }
        });
        deployment.spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(8.0),
            ..default()
        })
        .with_children(|actions| {
            lab_button(
                actions,
                assets,
                "Back to Setup",
                LabAction::BackToSetup,
                150.0,
            );
            lab_button(
                actions,
                assets,
                "Auto-place",
                LabAction::AutoPlace,
                130.0,
            );
            if placements_complete(&state.player_placements, state.players.len())
                && placements_complete(&state.hostile_placements, state.hostiles.len())
            {
                lab_button(
                    actions,
                    assets,
                    "Start Combat",
                    LabAction::StartSandbox,
                    150.0,
                );
            }
        });
    });
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
                            "{} {} {} {}",
                            fixture.id, fixture.name, fixture.tags, fixture.description
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
                state.deployment = false;
            }
            LabAction::Back => next.set(Screen::Title),
            LabAction::SelectMap(map) => {
                state.map = *map;
                state.player_placements.clear();
                state.hostile_placements.clear();
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
            LabAction::PrepareDeployment => {
                state.deployment = true;
                state.player_placements = vec![None; state.players.len()];
                state.hostile_placements = vec![None; state.hostiles.len()];
            }
            LabAction::AutoPlace => {
                state.player_placements = (0..state.players.len())
                    .map(|index| u8::try_from(index).ok())
                    .collect();
                state.hostile_placements = (0..state.hostiles.len())
                    .map(|index| u8::try_from(index).ok())
                    .collect();
            }
            LabAction::PlacePlayer(unit, slot) => {
                assign_slot(&mut state.player_placements, *unit, *slot);
            }
            LabAction::PlaceHostile(unit, slot) => {
                assign_slot(&mut state.hostile_placements, *unit, *slot);
            }
            LabAction::BackToSetup => {
                state.deployment = false;
                state.player_placements.clear();
                state.hostile_placements.clear();
            }
            LabAction::StartSandbox => {
                let Some(library) = scenarios.as_deref() else {
                    state.notice = "Scenario catalog is still loading.".to_owned();
                    state.bump();
                    continue;
                };
                let Some(scenario) = scenario_named(library, state.map.scenario_name()) else {
                    state.notice = "Selected sandbox map is not available.".to_owned();
                    state.bump();
                    continue;
                };
                let result = build_creator_overlay(
                    &state.players,
                    &state.hostiles,
                    &store.file,
                    shipped_spell_file.as_deref(),
                    base_lattice_file.as_deref(),
                    elements.as_deref(),
                    substances.as_deref(),
                );
                let overlay = match result {
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
                    Some((&state.player_placements, &state.hostile_placements)),
                );
                let resolved_seed = scenario.generation_seed.map(ResolvedMapSeed);
                commands.insert_resource(overlay);
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
    placements: Option<(&[Option<u8>], &[Option<u8>])>,
) -> Encounter {
    let (player_placement, hostile_placement) = match map {
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
    };
    Encounter {
        name: format!("Creator Sandbox · {}", map.label()),
        rosters: vec![
            Roster {
                faction: EncounterFaction::Player,
                placement: player_placement,
                units: players
                    .iter()
                    .enumerate()
                    .map(|(index, choice)| {
                        roster_entry(
                            choice,
                            placements
                                .and_then(|(player, _)| player.get(index).copied().flatten())
                                .and_then(|slot| deployment_placement(map, true, slot)),
                        )
                    })
                    .collect(),
            },
            Roster {
                faction: EncounterFaction::Hostile,
                placement: hostile_placement,
                units: hostiles
                    .iter()
                    .enumerate()
                    .map(|(index, choice)| {
                        roster_entry(
                            choice,
                            placements
                                .and_then(|(_, hostile)| hostile.get(index).copied().flatten())
                                .and_then(|slot| deployment_placement(map, false, slot)),
                        )
                    })
                    .collect(),
            },
        ],
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

fn deployment_placement(map: SandboxMap, player: bool, slot: u8) -> Option<EncounterPlacement> {
    let index = usize::from(slot);
    let flat_player = [
        (-2, 0, 2),
        (-3, 1, 2),
        (-2, 1, 1),
        (-3, 0, 3),
        (-1, 0, 1),
        (-1, -1, 2),
    ];
    let flat_hostile = [
        (2, 0, -2),
        (3, -1, -2),
        (2, -1, -1),
        (3, 0, -3),
        (1, 0, -1),
        (1, 1, -2),
    ];
    let crossing_player = [
        (0, 8, -8),
        (1, 7, -8),
        (-1, 8, -7),
        (0, 7, -7),
        (1, 8, -9),
        (-1, 9, -8),
    ];
    let crossing_hostile = [
        (0, -8, 8),
        (-1, -7, 8),
        (1, -8, 7),
        (0, -7, 7),
        (-1, -8, 9),
        (1, -9, 8),
    ];
    let coordinates = match (map, player) {
        (SandboxMap::Flat, true) => flat_player.get(index),
        (SandboxMap::Flat, false) => flat_hostile.get(index),
        (SandboxMap::Crossing, true) => crossing_player.get(index),
        (SandboxMap::Crossing, false) => crossing_hostile.get(index),
        // Generated terrain resolves exact surfaces only after its anchors exist.
        // Slot order still deterministically assigns the formation candidates.
        (SandboxMap::Hills, _) => return None,
    }?;
    Some(EncounterPlacement::Fixed(hex_assets::CubeCoord {
        x: coordinates.0,
        y: coordinates.1,
        z: coordinates.2,
    }))
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

fn assign_slot(placements: &mut [Option<u8>], unit: usize, slot: u8) {
    if unit >= placements.len() || usize::from(slot) >= MAX_ROSTER {
        return;
    }
    for assigned in placements.iter_mut() {
        if *assigned == Some(slot) {
            *assigned = None;
        }
    }
    if let Some(placement) = placements.get_mut(unit) {
        *placement = Some(slot);
    }
}

fn placements_complete(placements: &[Option<u8>], roster_len: usize) -> bool {
    placements.len() == roster_len
        && !placements.is_empty()
        && placements.iter().all(Option::is_some)
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
    fn deployment_slots_are_unique_and_complete_only_when_every_unit_is_placed() {
        let mut placements = vec![None, None];
        assign_slot(&mut placements, 0, 2);
        assign_slot(&mut placements, 1, 2);
        assert_eq!(placements, vec![None, Some(2)]);
        assert!(!placements_complete(&placements, 2));
        assign_slot(&mut placements, 0, 1);
        assert!(placements_complete(&placements, 2));
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
