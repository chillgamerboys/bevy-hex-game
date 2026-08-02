//! Sandbox composition, deployment, and exact-launch orchestration.

use std::borrow::Cow;
use std::collections::BTreeMap;

use bevy::ecs::system::SystemParam;
use bevy::picking::events::{Click, Pointer};
use bevy::picking::Pickable;
use bevy::prelude::*;
use hex_assets::{
    character_lattice_file, character_runtime_key, combined_spell_file, AcceptedContentRevision,
    CombatSettings, ContentIndex, CreationCellKind, CreationPresetCatalog, CustomCharacterId,
    ElementCatalog, Encounter, EncounterFaction, EncounterPlacement, FixedSettingsFreeze,
    FormationCenter, GameAssets, LatticeFile, LatticeLibrary, PlayerSettings, PresetAudience,
    Roster, RosterEntry, SandboxDeploymentRegion, SandboxMapCatalog, SandboxMapDefinition,
    SandboxRegionCenter, SavedCharacter, Scenario, ScenarioLibrary, SpellBook, SpellFile,
    SubstanceTable,
};
use hex_core::{
    GameplayPhase, GameplaySetup, GameplaySetupFailure, Headroom, HexSpan, HexTile,
    PresentationOcclusion, PresentationOcclusionReason, ResolvedMapSeed, Screen, SubstanceId,
    TilePos, TraversalBlockers, TraversalProfile, UnitId,
};
use hex_gameplay_model::{
    CampaignSlotId, CreatorNavigation, MainMenuModel, MainMenuRoute, SandboxBackResult,
    SandboxCharacter, SandboxDeploymentModel, SandboxDeploymentSlot, SandboxDeploymentStage,
    SandboxDestination, SandboxEntryOrigin, SandboxMapSelection, SandboxModel,
    SandboxPlacementRefusal, SandboxRoute, SandboxSide, SandboxSlotIndex, SandboxStartBlocker,
};
use hex_units::{Archetype, Body, Faction, Footing, StandsOn};

use crate::creation_store::CreationStore;
use crate::scenarios::ScenarioToLoad;
use hex_ui::{
    DeploymentIntent as DeploymentAction, DeploymentQueueEntryView, DeploymentView,
    SandboxCharacterView, SandboxIntent, SandboxLatticeCellKind, SandboxLatticeCellView,
    SandboxMapView, SandboxRosterSlotView, SandboxView, UiIntent, UiSystems,
};

type RosterChoice = SandboxCharacter<CustomCharacterId>;
pub(crate) type SandboxState = SandboxModel<CustomCharacterId>;

/// Enters Sandbox from the Creator's saved, clean, Map-ready action.
#[derive(Resource, Debug, Clone, Copy)]
pub(crate) struct CreatorOpenSandboxRequest {
    pub(crate) character: CustomCharacterId,
}

/// Restores one exact picker after a Creator excursion.
#[derive(Resource, Debug, Clone, Copy)]
pub(crate) struct CreatorPickerReturn {
    pub(crate) side: SandboxSide,
    pub(crate) slot: SandboxSlotIndex,
    pub(crate) saved_character: Option<CustomCharacterId>,
}

/// One coherent raw-and-derived gameplay content graph.
///
/// The accepted-revision publisher validates these five Creator-overlaid resources
/// as one subset of the broader graph, which also includes elements, substances, and
/// terrain damage. Keeping the overlay bundled prevents Creator sessions from
/// installing a combined `SpellBook` while leaving its shipped raw files behind,
/// which would make Loading either accept stale content or wait forever.
#[derive(Debug, Clone)]
struct CreatorContentBundle {
    spell_file: SpellFile,
    spells: SpellBook,
    lattice_file: LatticeFile,
    content: ContentIndex,
    lattices: LatticeLibrary,
}

impl CreatorContentBundle {
    fn insert_with_commands(&self, commands: &mut Commands) {
        commands.insert_resource(self.spell_file.clone());
        commands.insert_resource(self.spells.clone());
        commands.insert_resource(self.lattice_file.clone());
        commands.insert_resource(self.content.clone());
        commands.insert_resource(self.lattices.clone());
    }

    fn insert_into_world(&self, world: &mut World) {
        world.insert_resource(self.spell_file.clone());
        world.insert_resource(self.spells.clone());
        world.insert_resource(self.lattice_file.clone());
        world.insert_resource(self.content.clone());
        world.insert_resource(self.lattices.clone());
    }
}

/// Frozen Creator and shipped snapshots applied at the final loading boundary.
#[derive(Resource, Debug, Clone)]
pub(crate) struct CreatorContentOverlay {
    active: CreatorContentBundle,
    shipped: CreatorContentBundle,
    display_names: BTreeMap<String, String>,
}

/// Optional player-facing name that leaves the stable runtime archetype key intact.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub(crate) struct CreatorDisplayName(pub(crate) String);

/// Provenance for the currently loaded gameplay session.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub(crate) enum GameplaySessionOrigin {
    Campaign(CampaignSlotId),
    Sandbox,
    #[cfg(feature = "test-support")]
    TestFixture(String),
}

/// Exact deployment frozen after the player confirms placement.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SandboxDeploymentSnapshot {
    pub(crate) party: Vec<TilePos>,
    pub(crate) enemies: Vec<TilePos>,
}

/// Immutable Sandbox launch identity reused by Loading and Retry Exact.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SandboxLaunchSnapshot {
    pub(crate) map: SandboxMapSelection,
    pub(crate) scenario: String,
    pub(crate) party: Vec<RosterChoice>,
    pub(crate) enemies: Vec<RosterChoice>,
    pub(crate) content_revision: Option<u64>,
    pub(crate) deployment: Option<SandboxDeploymentSnapshot>,
    pub(crate) rules: CombatSettings,
    scenario_to_load: ScenarioToLoad,
}

/// Marks a temporary Sandbox gameplay session and carries its exact retry identity.
#[derive(Resource, Debug, Clone, PartialEq)]
pub(crate) struct SandboxSession {
    pub(crate) launch: SandboxLaunchSnapshot,
}

impl SandboxLaunchSnapshot {
    #[expect(
        clippy::too_many_arguments,
        reason = "the constructor freezes every independently sourced launch fact in one place"
    )]
    pub(crate) fn new(
        map: SandboxMapSelection,
        scenario_name: String,
        party: Vec<RosterChoice>,
        enemies: Vec<RosterChoice>,
        content_revision: Option<u64>,
        rules: CombatSettings,
        scenario: Scenario,
        encounter: Encounter,
    ) -> Self {
        let scenario_to_load = ScenarioToLoad {
            scenario,
            resolved_seed: map.resolved_seed.map(ResolvedMapSeed),
            encounter_override: Some(encounter),
        };
        Self {
            map,
            scenario: scenario_name,
            party,
            enemies,
            content_revision,
            deployment: None,
            rules,
            scenario_to_load,
        }
    }

    pub(crate) fn loading_input(&self) -> ScenarioToLoad {
        self.scenario_to_load.clone()
    }

    pub(crate) fn freeze_deployment(
        &mut self,
        deployment: SandboxDeploymentSnapshot,
        encounter: Encounter,
        content_revision: Option<u64>,
    ) {
        self.deployment = Some(deployment);
        self.content_revision = content_revision;
        self.scenario_to_load.encounter_override = Some(encounter);
    }
}

/// Frozen human deployment state carried over the already-loaded terrain.
#[derive(Resource, Debug, Clone)]
struct DeploymentSession {
    map_definition: SandboxMapDefinition,
    party: Vec<RosterChoice>,
    enemies: Vec<RosterChoice>,
    deployment: SandboxDeploymentModel,
}

impl DeploymentSession {
    fn new(
        map_definition: SandboxMapDefinition,
        party: Vec<RosterChoice>,
        enemies: Vec<RosterChoice>,
        deployment: SandboxDeploymentModel,
    ) -> Self {
        Self {
            map_definition,
            party,
            enemies,
            deployment,
        }
    }

    fn complete(&self) -> bool {
        self.deployment.is_complete()
            && matches!(self.deployment.stage, SandboxDeploymentStage::Review)
    }

    fn choice(&self, slot: SandboxDeploymentSlot) -> Option<&RosterChoice> {
        let index = self
            .deployment
            .order()
            .iter()
            .filter(|candidate| candidate.side == slot.side)
            .position(|candidate| *candidate == slot)?;
        match slot.side {
            SandboxSide::Party => self.party.get(index),
            SandboxSide::Enemies => self.enemies.get(index),
        }
    }
}

#[derive(Component)]
struct DeploymentWorldEntity;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct DeploymentPlacementMarker {
    slot: SandboxDeploymentSlot,
}

#[derive(Resource)]
struct DeploymentMarkerMaterials {
    player: Handle<StandardMaterial>,
    hostile: Handle<StandardMaterial>,
}

#[derive(Resource, Debug, Default)]
struct SandboxNotice(Option<String>);

#[derive(Resource, Debug)]
struct SandboxSeedSequence(u64);

impl Default for SandboxSeedSequence {
    fn default() -> Self {
        Self(0x5A4E_4442_4F58)
    }
}

impl SandboxSeedSequence {
    fn next(&mut self, catalog_id: &str) -> u64 {
        self.0 = self.0.wrapping_add(1);
        xxhash_rust::xxh3::xxh3_64(format!("{catalog_id}:{}", self.0).as_bytes())
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<SandboxState>()
        .init_resource::<SandboxNotice>()
        .init_resource::<SandboxSeedSequence>()
        .add_systems(
            OnEnter(Screen::Gameplay),
            (
                apply_creator_display_names.in_set(GameplaySetup::Restore),
                enter_deployment.in_set(GameplaySetup::Finalize),
            ),
        )
        .add_systems(
            Update,
            (
                handle_deployment_actions.after(UiSystems::EmitIntents),
                sync_deployment_markers,
                publish_deployment_view,
            )
                .chain()
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_observer(on_deployment_tile_clicked)
        .add_systems(OnExit(Screen::Gameplay), clear_deployment_world)
        .add_systems(OnEnter(Screen::Title), cleanup_sandbox_before_main_menu)
        .add_systems(
            OnEnter(Screen::Sandbox),
            (initialize_sandbox, publish_sandbox_view).chain(),
        )
        .add_systems(
            Update,
            (
                handle_sandbox_intents.after(UiSystems::EmitIntents),
                publish_sandbox_view,
            )
                .chain()
                .run_if(in_state(Screen::Sandbox)),
        );
}

fn initialize_sandbox(
    mut commands: Commands,
    open_request: Option<Res<CreatorOpenSandboxRequest>>,
    picker_return: Option<Res<CreatorPickerReturn>>,
    gameplay_session: Option<Res<SandboxSession>>,
    overlay: Option<Res<CreatorContentOverlay>>,
    setup_failure: Option<Res<GameplaySetupFailure>>,
    mut state: ResMut<SandboxState>,
    mut notice: ResMut<SandboxNotice>,
) {
    let returning_from_failed_launch = gameplay_session.is_some();
    clear_temporary_sandbox_resources(&mut commands, overlay.as_deref());
    commands.remove_resource::<crate::save::ActiveCampaign>();
    notice.0 = returning_from_failed_launch
        .then(|| {
            setup_failure
                .as_deref()
                .map(|failure| failure.reason.clone())
        })
        .flatten();
    if returning_from_failed_launch {
        commands.remove_resource::<GameplaySetupFailure>();
    }

    if let Some(request) = open_request.as_deref() {
        state.open_from_creator(RosterChoice::Custom(request.character));
        commands.remove_resource::<CreatorOpenSandboxRequest>();
    } else if let Some(returned) = picker_return.as_deref() {
        state.open_character_picker(returned.side, returned.slot);
        if let Some(character) = returned.saved_character {
            let _previewed = state.preview_character(RosterChoice::Custom(character));
        }
        commands.remove_resource::<CreatorPickerReturn>();
    } else if gameplay_session.is_some() {
        let origin = state.entry_origin;
        state.enter(origin);
    } else {
        state.enter(SandboxEntryOrigin::MainMenu);
        commands.remove_resource::<super::creator::CreatorSandboxReturn>();
    }
}

/// Clears every temporary Sandbox-owned launch fact at the Main Menu boundary.
///
/// A setup failure can return directly from Loading to the Main Menu without ever
/// entering Sandbox again. Restoring the shipped content graph and unfreezing rules
/// here ensures a subsequent Campaign launch cannot observe the abandoned Sandbox
/// overlay, encounter, or retry identity.
fn cleanup_sandbox_before_main_menu(
    mut commands: Commands,
    overlay: Option<Res<CreatorContentOverlay>>,
    sandbox: Option<Res<SandboxSession>>,
    origin: Option<Res<GameplaySessionOrigin>>,
) {
    let sandbox_owned = sandbox.is_some()
        || overlay.is_some()
        || matches!(origin.as_deref(), Some(GameplaySessionOrigin::Sandbox));
    if sandbox_owned {
        clear_temporary_sandbox_resources(&mut commands, overlay.as_deref());
    }
    commands.remove_resource::<super::creator::CreatorSandboxReturn>();
}

fn clear_temporary_sandbox_resources(
    commands: &mut Commands,
    overlay: Option<&CreatorContentOverlay>,
) {
    restore_shipped_content(commands, overlay);
    commands.remove_resource::<FixedSettingsFreeze<CombatSettings>>();
    commands.remove_resource::<FixedSettingsFreeze<SpellFile>>();
    commands.remove_resource::<FixedSettingsFreeze<LatticeFile>>();
    commands.remove_resource::<DeploymentSession>();
    commands.remove_resource::<DeploymentMarkerMaterials>();
    commands.remove_resource::<SandboxSession>();
    commands.remove_resource::<GameplaySessionOrigin>();
    commands.remove_resource::<CreatorOpenSandboxRequest>();
    commands.remove_resource::<CreatorPickerReturn>();
    commands.remove_resource::<ScenarioToLoad>();
    commands.remove_resource::<crate::scenarios::ActiveScenario>();
    commands.remove_resource::<Encounter>();
    commands.insert_resource(GameplayPhase::Active);
}

fn publish_sandbox_view(
    state: Res<SandboxState>,
    notice: Res<SandboxNotice>,
    store: Res<CreationStore>,
    catalogs: SandboxLaunchCatalogs,
    mut view: ResMut<SandboxView>,
) {
    let catalog_changed = catalogs
        .elements
        .as_ref()
        .is_some_and(|value| value.is_changed())
        || catalogs
            .spells
            .as_ref()
            .is_some_and(|value| value.is_changed())
        || catalogs
            .presets
            .as_ref()
            .is_some_and(|value| value.is_changed())
        || catalogs
            .map_catalog
            .as_ref()
            .is_some_and(|value| value.is_changed())
        || catalogs
            .scenarios
            .as_ref()
            .is_some_and(|value| value.is_changed())
        || catalogs
            .combat
            .as_ref()
            .is_some_and(|value| value.is_changed())
        || catalogs
            .shipped_spell_file
            .as_ref()
            .is_some_and(|value| value.is_changed())
        || catalogs
            .base_lattice_file
            .as_ref()
            .is_some_and(|value| value.is_changed())
        || catalogs
            .substances
            .as_ref()
            .is_some_and(|value| value.is_changed());
    if !state.is_changed()
        && !store.is_changed()
        && !notice.is_changed()
        && !catalog_changed
        && view.active
    {
        return;
    }

    let start_blocker = prepare_sandbox_launch(&state, &store, &catalogs).err();
    let map =
        state.draft.map.as_ref().and_then(|selection| {
            project_map_selection(selection, catalogs.map_catalog.as_deref())
        });
    let pending_map = state
        .pending_map
        .as_ref()
        .and_then(|selection| project_map_selection(selection, catalogs.map_catalog.as_deref()));
    let catalog_maps = catalogs
        .map_catalog
        .as_deref()
        .map_or_else(Vec::new, |catalog| {
            catalog
                .maps
                .iter()
                .map(|definition| project_map_definition(definition, definition.fixed_seed))
                .collect()
        });
    let party = project_roster(
        &state,
        SandboxSide::Party,
        &store,
        catalogs.presets.as_deref(),
        catalogs.elements.as_deref(),
        catalogs.spells.as_deref(),
    );
    let enemies = project_roster(
        &state,
        SandboxSide::Enemies,
        &store,
        catalogs.presets.as_deref(),
        catalogs.elements.as_deref(),
        catalogs.spells.as_deref(),
    );
    let characters = character_choices(
        &state,
        &store,
        catalogs.presets.as_deref(),
        catalogs.elements.as_deref(),
        catalogs.spells.as_deref(),
    );
    let preview = state.preview.as_ref().and_then(|choice| {
        project_character(
            choice,
            &store,
            catalogs.presets.as_deref(),
            catalogs.elements.as_deref(),
            catalogs.spells.as_deref(),
            true,
        )
    });

    *view = SandboxView {
        active: true,
        route: state.route,
        map,
        pending_map,
        maps: catalog_maps,
        party,
        enemies,
        characters,
        preview,
        start_blocker,
        notice: notice.0.clone().or_else(|| store.error.clone()),
    };
}

fn project_map_selection(
    selection: &SandboxMapSelection,
    catalog: Option<&SandboxMapCatalog>,
) -> Option<SandboxMapView> {
    let definition = catalog?.get(&selection.catalog_id)?;
    Some(project_map_definition(definition, selection.resolved_seed))
}

fn project_map_definition(
    definition: &SandboxMapDefinition,
    resolved_seed: Option<u64>,
) -> SandboxMapView {
    SandboxMapView {
        id: definition.id.clone(),
        name: definition.display_name.clone(),
        description: definition.description.clone(),
        preview: definition.preview.clone(),
        resolved_seed,
        can_regenerate: definition.fixed_seed.is_some(),
    }
}

fn project_roster(
    state: &SandboxState,
    side: SandboxSide,
    store: &CreationStore,
    presets: Option<&CreationPresetCatalog>,
    elements: Option<&ElementCatalog>,
    spells: Option<&SpellBook>,
) -> Vec<SandboxRosterSlotView> {
    SandboxSlotIndex::ALL
        .into_iter()
        .map(|slot| SandboxRosterSlotView {
            slot,
            character: state.draft.character(side, slot).and_then(|choice| {
                project_character(choice, store, presets, elements, spells, false)
            }),
        })
        .collect()
}

fn character_choices(
    state: &SandboxState,
    store: &CreationStore,
    presets: Option<&CreationPresetCatalog>,
    elements: Option<&ElementCatalog>,
    spells: Option<&SpellBook>,
) -> Vec<SandboxCharacterView> {
    let templates = presets.into_iter().flat_map(|catalog| {
        catalog
            .characters
            .iter()
            .filter(|record| record.audience == PresetAudience::HumanTemplate)
            .filter_map(|record| record.key.strip_prefix("template-"))
            .map(|key| RosterChoice::Template(key.to_owned()))
    });
    let custom = store
        .file
        .characters
        .iter()
        .map(|character| RosterChoice::Custom(character.id));
    templates
        .chain(custom)
        .filter_map(|choice| {
            let selected = state.preview.as_ref() == Some(&choice);
            project_character(&choice, store, presets, elements, spells, selected)
        })
        .collect()
}

fn project_character(
    choice: &RosterChoice,
    store: &CreationStore,
    presets: Option<&CreationPresetCatalog>,
    elements: Option<&ElementCatalog>,
    spells: Option<&SpellBook>,
    selected: bool,
) -> Option<SandboxCharacterView> {
    let (character, library) = choice_record(choice, store, presets).ok()?;
    let summary = hex_ui::CharacterBuildSummary::from_saved(character, &library, elements, spells);
    Some(SandboxCharacterView {
        character: choice.clone(),
        name: character.name.trim_end_matches(" Template").to_owned(),
        lattice: summary.compact_line(),
        cells: character
            .cells
            .iter()
            .map(|cell| {
                let (label, kind) = match &cell.kind {
                    CreationCellKind::Gem(name) => {
                        (compact_cell_label(name), SandboxLatticeCellKind::Gem)
                    }
                    CreationCellKind::Fusion(name) => {
                        (compact_cell_label(name), SandboxLatticeCellKind::Fusion)
                    }
                    CreationCellKind::Spell(_) => ("S".to_owned(), SandboxLatticeCellKind::Spell),
                    CreationCellKind::Blank => ("·".to_owned(), SandboxLatticeCellKind::Blank),
                };
                SandboxLatticeCellView {
                    q: cell.q,
                    r: cell.r,
                    label,
                    kind,
                }
            })
            .collect(),
        blocked: choice_readiness(choice, store, presets, elements, spells).err(),
        selected,
    })
}

fn compact_cell_label(name: &str) -> String {
    name.chars()
        .filter(|character| character.is_alphanumeric())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}

fn choice_record<'a>(
    choice: &RosterChoice,
    store: &'a CreationStore,
    presets: Option<&'a CreationPresetCatalog>,
) -> Result<(&'a SavedCharacter, Cow<'a, hex_assets::CreationLibraryFile>), String> {
    match choice {
        RosterChoice::Custom(id) => store
            .file
            .characters
            .iter()
            .find(|character| character.id == *id)
            .map(|character| (character, Cow::Borrowed(&store.file)))
            .ok_or_else(|| format!("Character #{} is missing.", id.0)),
        RosterChoice::Template(key) => {
            let presets =
                presets.ok_or_else(|| "Character templates are still loading.".to_owned())?;
            let record = presets
                .characters
                .iter()
                .find(|record| {
                    record.audience == PresetAudience::HumanTemplate
                        && record.key == format!("template-{key}")
                })
                .ok_or_else(|| format!("Template {key:?} is unavailable."))?;
            // Packaged characters are validated against an isolated HumanTemplate
            // library so their stable local IDs cannot collide with user data.
            Ok((
                &record.character,
                Cow::Owned(presets.library_for(PresetAudience::HumanTemplate)),
            ))
        }
    }
}

fn choice_readiness(
    choice: &RosterChoice,
    store: &CreationStore,
    presets: Option<&CreationPresetCatalog>,
    elements: Option<&ElementCatalog>,
    spells: Option<&SpellBook>,
) -> Result<(), String> {
    let (Some(elements), Some(spells)) = (elements, spells) else {
        return Err("Content catalogs are still loading.".to_owned());
    };
    let (character, library) = choice_record(choice, store, presets)?;
    character_map_readiness(character, &library, elements, spells)
}

fn character_map_readiness(
    character: &SavedCharacter,
    library: &hex_assets::CreationLibraryFile,
    elements: &ElementCatalog,
    spells: &SpellBook,
) -> Result<(), String> {
    let issues = super::creator::character_map_issues(character, library, elements, spells);
    if issues.is_empty() {
        Ok(())
    } else {
        Err(issues.join("; "))
    }
}

#[derive(SystemParam)]
struct SandboxLaunchCatalogs<'w> {
    scenarios: Option<Res<'w, ScenarioLibrary>>,
    combat: Option<Res<'w, CombatSettings>>,
    shipped_spell_file: Option<Res<'w, SpellFile>>,
    base_lattice_file: Option<Res<'w, LatticeFile>>,
    elements: Option<Res<'w, ElementCatalog>>,
    spells: Option<Res<'w, SpellBook>>,
    substances: Option<Res<'w, SubstanceTable>>,
    presets: Option<Res<'w, CreationPresetCatalog>>,
    map_catalog: Option<Res<'w, SandboxMapCatalog>>,
    accepted: Option<Res<'w, AcceptedContentRevision>>,
}

/// Fully resolved launch input produced by the same decision used for presentation.
struct PreparedSandboxLaunch {
    selection: SandboxMapSelection,
    definition: SandboxMapDefinition,
    scenario: Scenario,
    rules: CombatSettings,
    party: Vec<RosterChoice>,
    enemies: Vec<RosterChoice>,
    deployment: SandboxDeploymentModel,
    overlay: CreatorContentOverlay,
}

/// Resolves the one authoritative Start Sandbox decision.
///
/// Both the immutable view and the intent handler call this adapter. Scenario and
/// combat-settings readiness therefore cannot disagree between a disabled button and
/// its handler, and every refusal remains in the six-message model vocabulary.
fn prepare_sandbox_launch(
    state: &SandboxState,
    store: &CreationStore,
    catalogs: &SandboxLaunchCatalogs,
) -> Result<PreparedSandboxLaunch, SandboxStartBlocker> {
    let maps_loaded =
        catalogs.map_catalog.is_some() && catalogs.scenarios.is_some() && catalogs.combat.is_some();
    let map_available = state.draft.map.as_ref().is_some_and(|selection| {
        catalogs
            .map_catalog
            .as_deref()
            .and_then(|catalog| catalog.get(&selection.catalog_id))
            .is_some_and(|definition| {
                catalogs
                    .scenarios
                    .as_deref()
                    .is_some_and(|library| scenario_named(library, &definition.scenario).is_some())
            })
    });
    let mut checked_characters = Vec::new();
    let mut resolved_overlay = None;
    if let Some(blocker) = state.start_blocker(maps_loaded, map_available, |choice| {
        choice_readiness(
            choice,
            store,
            catalogs.presets.as_deref(),
            catalogs.elements.as_deref(),
            catalogs.spells.as_deref(),
        )?;
        // Resolve stable-order prefixes, not isolated choices. This makes the first
        // slot whose addition breaks the combined namespace the exact typed blocker,
        // while the last successful prefix is already the final launch overlay.
        checked_characters.push(choice.clone());
        resolved_overlay = Some(build_creator_overlay(
            &checked_characters,
            &[],
            &store.file,
            catalogs.shipped_spell_file.as_deref(),
            catalogs.base_lattice_file.as_deref(),
            catalogs.elements.as_deref(),
            catalogs.substances.as_deref(),
        )?);
        Ok(())
    }) {
        return Err(blocker);
    }

    let selection = state
        .draft
        .map
        .clone()
        .ok_or(SandboxStartBlocker::ChooseMap)?;
    let definition = catalogs
        .map_catalog
        .as_deref()
        .and_then(|catalog| catalog.get(&selection.catalog_id))
        .cloned()
        .ok_or(SandboxStartBlocker::MapUnavailable)?;
    let scenario = catalogs
        .scenarios
        .as_deref()
        .and_then(|library| scenario_named(library, &definition.scenario))
        .ok_or(SandboxStartBlocker::MapUnavailable)?;
    let rules = catalogs
        .combat
        .as_deref()
        .cloned()
        .ok_or(SandboxStartBlocker::MapsLoading)?;
    let deployment = SandboxDeploymentModel::from_draft(&state.draft);
    let party = state.draft.flattened_roster(SandboxSide::Party);
    let enemies = state.draft.flattened_roster(SandboxSide::Enemies);
    let Some(overlay) = resolved_overlay else {
        // The renderer-free blocker above already proves both sides non-empty.
        // Keep the defensive path in the same typed vocabulary if that invariant is
        // ever changed independently.
        return Err(SandboxStartBlocker::PartyEmpty);
    };

    Ok(PreparedSandboxLaunch {
        selection,
        definition,
        scenario,
        rules,
        party,
        enemies,
        deployment,
        overlay,
    })
}

fn handle_sandbox_intents(
    mut intents: MessageReader<UiIntent>,
    mut state: ResMut<SandboxState>,
    mut notice: ResMut<SandboxNotice>,
    store: Res<CreationStore>,
    catalogs: SandboxLaunchCatalogs,
    creator_return: Option<Res<super::creator::CreatorSandboxReturn>>,
    mut seeds: ResMut<SandboxSeedSequence>,
    mut main_menu: ResMut<MainMenuModel>,
    mut commands: Commands,
    mut next: ResMut<NextState<Screen>>,
) {
    for intent in intents.read() {
        let UiIntent::Sandbox(action) = intent else {
            continue;
        };
        notice.0 = None;
        match action {
            SandboxIntent::Back => match state.back() {
                SandboxBackResult::Routed => {}
                SandboxBackResult::Exit(SandboxDestination::MainMenu) => {
                    main_menu.show(MainMenuRoute::Root);
                    next.set(Screen::Title);
                }
                SandboxBackResult::Exit(SandboxDestination::Creator) => {
                    let navigation = creator_return
                        .as_deref()
                        .map_or_else(CreatorNavigation::default, |retained| retained.navigation);
                    commands.insert_resource(super::creator::CreatorRestoreRequest(navigation));
                    commands.remove_resource::<super::creator::CreatorSandboxReturn>();
                    next.set(Screen::CharacterCreator);
                }
            },
            SandboxIntent::OpenMapBrowser => state.open_map_browser(),
            SandboxIntent::SelectMap(id) => {
                let Some(definition) = catalogs
                    .map_catalog
                    .as_deref()
                    .and_then(|catalog| catalog.get(id))
                else {
                    notice.0 = Some("The selected map is unavailable.".to_owned());
                    continue;
                };
                state.select_map(SandboxMapSelection::new(
                    definition.id.clone(),
                    definition.fixed_seed,
                ));
            }
            SandboxIntent::RegenerateMap => {
                let Some(selection) = state.pending_map.as_ref() else {
                    continue;
                };
                let Some(definition) = catalogs
                    .map_catalog
                    .as_deref()
                    .and_then(|catalog| catalog.get(&selection.catalog_id))
                else {
                    notice.0 = Some("The selected map is unavailable.".to_owned());
                    continue;
                };
                if definition.fixed_seed.is_some() {
                    let seed = seeds.next(&selection.catalog_id);
                    let _changed = state.set_pending_seed(Some(seed));
                }
            }
            SandboxIntent::UseMap => {
                if !state.use_pending_map() {
                    notice.0 = Some("Choose a map before confirming it.".to_owned());
                }
            }
            SandboxIntent::OpenRoster(side) => state.open_roster(*side),
            SandboxIntent::OpenCharacterPicker { side, slot } => {
                state.open_character_picker(*side, *slot);
            }
            SandboxIntent::PreviewCharacter(character) => {
                let _previewed = state.preview_character(character.clone());
            }
            SandboxIntent::UseCharacter => {
                if !state.use_previewed_character() {
                    notice.0 = Some("Choose a character before applying it.".to_owned());
                }
            }
            SandboxIntent::ClearSlot { side, slot } => state.clear_character(*side, *slot),
            SandboxIntent::CreateCharacter => {
                let SandboxRoute::CharacterPicker { side, slot } = state.route else {
                    continue;
                };
                commands.insert_resource(super::creator::CreatorEntryRequest(
                    hex_gameplay_model::CreatorEntry::CharacterLibrary(
                        hex_gameplay_model::CreatorOrigin::SandboxCharacterPicker { side, slot },
                    ),
                ));
                next.set(Screen::CharacterCreator);
            }
            SandboxIntent::StartSandbox => {
                let prepared = match prepare_sandbox_launch(&state, &store, &catalogs) {
                    Ok(prepared) => prepared,
                    Err(blocker) => {
                        notice.0 = Some(blocker.message());
                        continue;
                    }
                };
                let encounter =
                    sandbox_encounter(&prepared.party, &prepared.enemies, &prepared.definition);
                let launch = SandboxLaunchSnapshot::new(
                    prepared.selection,
                    prepared.definition.scenario.clone(),
                    prepared.party.clone(),
                    prepared.enemies.clone(),
                    catalogs
                        .accepted
                        .as_deref()
                        .map(AcceptedContentRevision::fingerprint),
                    prepared.rules,
                    prepared.scenario,
                    encounter,
                );
                commands.insert_resource(prepared.overlay);
                commands.insert_resource(DeploymentSession::new(
                    prepared.definition,
                    prepared.party,
                    prepared.enemies,
                    prepared.deployment,
                ));
                commands.insert_resource(GameplayPhase::Preparing);
                commands.insert_resource(GameplaySessionOrigin::Sandbox);
                commands.insert_resource(SandboxSession {
                    launch: launch.clone(),
                });
                commands.insert_resource(FixedSettingsFreeze::<CombatSettings>::default());
                commands.insert_resource(FixedSettingsFreeze::<SpellFile>::default());
                commands.insert_resource(FixedSettingsFreeze::<LatticeFile>::default());
                commands.insert_resource(launch.loading_input());
                commands.insert_resource(launch.rules.clone());
                commands.remove_resource::<crate::save::ActiveCampaign>();
                next.set(Screen::Loading);
            }
        }
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
    party: &[RosterChoice],
    enemies: &[RosterChoice],
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
    let selected_custom: Vec<&SavedCharacter> = party
        .iter()
        .chain(enemies)
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
    for character in party
        .iter()
        .chain(enemies)
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
    let shipped_spell_book = SpellBook::from_file(shipped_spells);
    let shipped_content =
        ContentIndex::build(elements, &shipped_spell_book, substances).map_err(|errors| {
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        })?;
    let shipped_lattices = LatticeLibrary::build(base_lattices, elements, &shipped_spell_book)
        .map_err(|errors| {
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        })?;

    Ok(CreatorContentOverlay {
        active: CreatorContentBundle {
            spell_file,
            spells: spell_book,
            lattice_file,
            content,
            lattices,
        },
        shipped: CreatorContentBundle {
            spell_file: shipped_spells.clone(),
            spells: shipped_spell_book,
            lattice_file: base_lattices.clone(),
            content: shipped_content,
            lattices: shipped_lattices,
        },
        display_names: party
            .iter()
            .chain(enemies)
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

/// Uses the exact Sandbox overlay resolver for Creator's preflight action.
pub(super) fn creator_character_map_readiness(
    character: CustomCharacterId,
    library: &hex_assets::CreationLibraryFile,
    shipped_spells: Option<&SpellFile>,
    base_lattices: Option<&LatticeFile>,
    elements: Option<&ElementCatalog>,
    substances: Option<&SubstanceTable>,
) -> Result<(), String> {
    build_creator_overlay(
        &[RosterChoice::Custom(character)],
        &[],
        library,
        shipped_spells,
        base_lattices,
        elements,
        substances,
    )
    .map(|_| ())
}

#[cfg(feature = "test-support")]
pub(crate) fn build_deterministic_creator_overlay(
    world: &World,
    party: &[SandboxCharacter<CustomCharacterId>],
    enemies: &[SandboxCharacter<CustomCharacterId>],
    library: &hex_assets::CreationLibraryFile,
) -> Result<CreatorContentOverlay, String> {
    build_creator_overlay(
        party,
        enemies,
        library,
        world.get_resource::<SpellFile>(),
        world.get_resource::<LatticeFile>(),
        world.get_resource::<ElementCatalog>(),
        world.get_resource::<SubstanceTable>(),
    )
}

fn sandbox_encounter(
    party: &[RosterChoice],
    enemies: &[RosterChoice],
    definition: &SandboxMapDefinition,
) -> Encounter {
    encounter_with_placements(
        &definition.display_name,
        party,
        enemies,
        deployment_region_placement(&definition.player_region),
        deployment_region_placement(&definition.hostile_region),
    )
}

fn encounter_with_placements(
    map_name: &str,
    party: &[RosterChoice],
    enemies: &[RosterChoice],
    party_placement: EncounterPlacement,
    enemy_placement: EncounterPlacement,
) -> Encounter {
    Encounter {
        name: format!("Creator Sandbox · {map_name}"),
        rosters: vec![
            Roster {
                faction: EncounterFaction::Player,
                placement: party_placement,
                units: party
                    .iter()
                    .map(|choice| roster_entry(choice, None))
                    .collect(),
            },
            Roster {
                faction: EncounterFaction::Hostile,
                placement: enemy_placement,
                units: enemies
                    .iter()
                    .map(|choice| roster_entry(choice, None))
                    .collect(),
            },
        ],
    }
}

fn deployment_region_placement(region: &SandboxDeploymentRegion) -> EncounterPlacement {
    let center = match &region.center {
        SandboxRegionCenter::Fixed(coord) => FormationCenter::Fixed(*coord),
        SandboxRegionCenter::Anchor(anchor) => FormationCenter::Anchor(anchor.clone()),
    };
    EncounterPlacement::Formation {
        center,
        spread: region.radius,
    }
}

fn roster_entry(choice: &RosterChoice, placement: Option<EncounterPlacement>) -> RosterEntry {
    RosterEntry {
        archetype: roster_archetype_key(choice).into_owned(),
        placement,
        ai_profile: None,
        ai_group: None,
    }
}

fn roster_archetype_key(choice: &RosterChoice) -> Cow<'_, str> {
    match choice {
        RosterChoice::Template(name) => Cow::Borrowed(name),
        RosterChoice::Custom(id) => Cow::Owned(character_runtime_key(*id)),
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

fn deployment_snapshot(session: &DeploymentSession) -> Option<SandboxDeploymentSnapshot> {
    session
        .deployment
        .frozen_placements()
        .map(|(party, enemies)| SandboxDeploymentSnapshot { party, enemies })
}

fn deployment_notice(session: &DeploymentSession, store: &CreationStore) -> String {
    if let Some(refusal) = session.deployment.refusal {
        return refusal.message();
    }
    match session.deployment.stage {
        SandboxDeploymentStage::Placing(slot) => {
            let name = session
                .choice(slot)
                .map_or_else(|| slot.to_string(), |choice| choice_name(choice, store));
            let (current, total) = session
                .deployment
                .progress(slot)
                .unwrap_or((1, session.deployment.order().len()));
            format!(
                "{} {} of {total} · Choose any open surface for {name}.",
                slot.side.to_string().to_uppercase(),
                current
            )
        }
        SandboxDeploymentStage::Review => format!(
            "All {} characters placed · select one to reposition or start combat.",
            session.deployment.order().len()
        ),
    }
}

fn publish_deployment_view(
    session: Option<Res<DeploymentSession>>,
    store: Res<CreationStore>,
    mut view: ResMut<DeploymentView>,
) {
    let session_changed = session.as_ref().is_some_and(|session| session.is_changed());
    let Some(session) = session.as_deref() else {
        if view.active {
            *view = DeploymentView::default();
        }
        return;
    };
    if !session_changed && !store.is_changed() && view.active {
        return;
    }
    let queue = session
        .deployment
        .order()
        .iter()
        .copied()
        .map(|slot| DeploymentQueueEntryView {
            slot,
            name: session.choice(slot).map_or_else(
                || format!("Unavailable {slot}"),
                |choice| choice_name(choice, &store),
            ),
            selected: session.deployment.active_slot() == Some(slot),
            placed: session.deployment.placement(slot).is_some(),
            selectable: session.deployment.is_selectable(slot),
        })
        .collect();
    *view = DeploymentView {
        active: true,
        map_name: session.map_definition.display_name.clone(),
        notice: deployment_notice(session, &store),
        stage: Some(session.deployment.stage),
        queue,
        can_undo: session.deployment.can_undo(),
        complete: session.complete(),
    };
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

fn enter_deployment(
    mut commands: Commands,
    session: Option<Res<DeploymentSession>>,
    mut phase: ResMut<GameplayPhase>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut actors: Query<&mut PresentationOcclusion, With<Faction>>,
) {
    let Some(_session) = session.as_deref() else {
        *phase = GameplayPhase::Active;
        return;
    };
    *phase = GameplayPhase::Deployment;
    for mut occlusion in &mut actors {
        occlusion.insert(PresentationOcclusionReason::SandboxDeployment);
    }
    commands.insert_resource(DeploymentMarkerMaterials {
        player: materials.add(deployment_marker_material(Color::srgb(0.10, 0.68, 1.0))),
        hostile: materials.add(deployment_marker_material(Color::srgb(1.0, 0.20, 0.13))),
    });
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

fn deployment_marker_material(color: Color) -> StandardMaterial {
    StandardMaterial {
        base_color: color,
        unlit: true,
        ..default()
    }
}

fn on_deployment_tile_clicked(
    click: On<Pointer<Click>>,
    tiles: DeploymentTileQuery,
    table: Option<Res<SubstanceTable>>,
    blockers: Option<Res<TraversalBlockers>>,
    mut session: Option<ResMut<DeploymentSession>>,
) {
    if click.button != PointerButton::Primary {
        return;
    }
    let Ok((pos, _, _, _)) = tiles.get(click.event_target()) else {
        return;
    };
    let Some(session) = session.as_deref_mut() else {
        return;
    };
    let Some(table) = table.as_deref() else {
        session
            .deployment
            .refuse(SandboxPlacementRefusal::TerrainUnavailable);
        return;
    };
    let footing = deployment_footing(&tiles, table, blockers.as_deref());
    if footing.at(*pos).is_none() {
        session
            .deployment
            .refuse(SandboxPlacementRefusal::InvalidFooting);
        return;
    }
    let _placed = session.deployment.place_validated(*pos);
}

#[derive(SystemParam)]
struct DeploymentMarkerRuntime<'w, 's> {
    tiles: DeploymentTileQuery<'w, 's>,
    table: Option<Res<'w, SubstanceTable>>,
    blockers: Option<Res<'w, TraversalBlockers>>,
    game_assets: Option<Res<'w, GameAssets>>,
    player_settings: Option<Res<'w, PlayerSettings>>,
    materials: Option<Res<'w, DeploymentMarkerMaterials>>,
    entities: Query<'w, 's, Entity, With<DeploymentPlacementMarker>>,
}

fn sync_deployment_markers(
    mut commands: Commands,
    session: Option<Res<DeploymentSession>>,
    runtime: DeploymentMarkerRuntime,
) {
    let session_changed = session
        .as_ref()
        .is_some_and(|resource| resource.is_changed());
    let Some(session) = session.as_deref() else {
        return;
    };
    let expected = resolved_deployment_markers(session).count();
    let resources_changed = runtime
        .table
        .as_ref()
        .is_some_and(|resource| resource.is_changed())
        || runtime
            .blockers
            .as_ref()
            .is_some_and(|resource| resource.is_changed())
        || runtime
            .game_assets
            .as_ref()
            .is_some_and(|resource| resource.is_changed())
        || runtime
            .player_settings
            .as_ref()
            .is_some_and(|resource| resource.is_changed())
        || runtime
            .materials
            .as_ref()
            .is_some_and(|resource| resource.is_changed());
    if !session_changed && !resources_changed && runtime.entities.iter().count() == expected {
        return;
    }
    rebuild_deployment_markers(&mut commands, &runtime, session);
}

fn rebuild_deployment_markers(
    commands: &mut Commands,
    runtime: &DeploymentMarkerRuntime,
    session: &DeploymentSession,
) {
    for entity in &runtime.entities {
        commands.entity(entity).despawn();
    }
    let (Some(table), Some(materials), Some(game_assets), Some(player_settings)) = (
        runtime.table.as_deref(),
        runtime.materials.as_deref(),
        runtime.game_assets.as_deref(),
        runtime.player_settings.as_deref(),
    ) else {
        return;
    };
    let footing = deployment_footing(&runtime.tiles, table, runtime.blockers.as_deref());
    let scale = player_settings.scale * 1.08;
    let child_transform = Transform {
        translation: Vec3::new(-scale, -scale, -10.0 * scale),
        scale: Vec3::splat(scale),
        ..default()
    };
    for (slot, pos) in resolved_deployment_markers(session) {
        let Some(standing) = footing.at(pos) else {
            continue;
        };
        let material = match slot.side {
            SandboxSide::Party => materials.player.clone(),
            SandboxSide::Enemies => materials.hostile.clone(),
        };
        let [mesh_a, mesh_b] = game_assets.player_pieces.clone();
        commands
            .spawn((
                Name::new(format!("{} Placement {}", slot.side, slot.slot)),
                DeploymentWorldEntity,
                DeploymentPlacementMarker { slot },
                Transform::from_translation(standing.world_position()),
                Visibility::default(),
                Pickable::IGNORE,
            ))
            .with_children(|marker| {
                marker.spawn((
                    Mesh3d(game_assets.hex_tile.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform {
                        translation: Vec3::new(0.0, 0.045, 0.0),
                        scale: Vec3::new(0.52, 0.08, 0.52),
                        ..default()
                    },
                    Pickable::IGNORE,
                ));
                for mesh in [mesh_a, mesh_b] {
                    marker.spawn((
                        Mesh3d(mesh),
                        MeshMaterial3d(material.clone()),
                        child_transform,
                        Pickable::IGNORE,
                    ));
                }
            });
    }
}

fn resolved_deployment_markers(
    session: &DeploymentSession,
) -> impl Iterator<Item = (SandboxDeploymentSlot, TilePos)> + '_ {
    session
        .deployment
        .order()
        .iter()
        .copied()
        .filter_map(|slot| session.deployment.placement(slot).map(|pos| (slot, pos)))
}

#[derive(SystemParam)]
struct DeploymentRuntime<'w, 's> {
    markers: DeploymentMarkerRuntime<'w, 's>,
    units: Query<
        'w,
        's,
        (
            Entity,
            &'static hex_core::UnitId,
            &'static Faction,
            &'static Archetype,
            &'static mut StandsOn,
            &'static mut Transform,
        ),
    >,
    actor_occlusion: Query<'w, 's, &'static mut PresentationOcclusion, With<Faction>>,
    world_entities: Query<'w, 's, Entity, With<DeploymentWorldEntity>>,
    encounter: Option<ResMut<'w, Encounter>>,
    active: Option<ResMut<'w, crate::scenarios::ActiveScenario>>,
    sandbox: Option<ResMut<'w, SandboxSession>>,
    accepted: Option<Res<'w, AcceptedContentRevision>>,
    origin: Option<Res<'w, GameplaySessionOrigin>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OrderedDeploymentUnit {
    id: UnitId,
    entity: Entity,
}

/// Associates the spawned roster with launch slots exclusively through stable ids.
///
/// Query and entity allocation order are deliberately irrelevant. Exact side counts
/// and archetype identities are checked before any actor moves, so a partial or
/// reordered spawn cannot silently receive another slot's frozen placement.
fn ordered_deployment_units(
    units: impl IntoIterator<Item = (Entity, UnitId, Faction, String)>,
    party: &[RosterChoice],
    enemies: &[RosterChoice],
) -> Result<(Vec<OrderedDeploymentUnit>, Vec<OrderedDeploymentUnit>), String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut players = Vec::new();
    let mut hostiles = Vec::new();
    for (entity, id, faction, archetype) in units {
        if !seen.insert(id) {
            return Err(format!(
                "Sandbox deployment found duplicate stable unit id {:?}.",
                id
            ));
        }
        match faction {
            Faction::Player => players.push((id, entity, archetype)),
            Faction::Hostile => hostiles.push((id, entity, archetype)),
        }
    }
    players.sort_by_key(|(id, _, _)| *id);
    hostiles.sort_by_key(|(id, _, _)| *id);

    fn validate_side(
        label: &str,
        actual: Vec<(UnitId, Entity, String)>,
        expected: &[RosterChoice],
    ) -> Result<Vec<OrderedDeploymentUnit>, String> {
        if actual.len() != expected.len() {
            return Err(format!(
                "Sandbox deployment expected {} {label} units, but found {}.",
                expected.len(),
                actual.len()
            ));
        }
        actual
            .into_iter()
            .zip(expected)
            .enumerate()
            .map(|(index, ((id, entity, actual), expected))| {
                let expected = roster_archetype_key(expected);
                if actual != expected {
                    return Err(format!(
                        "Sandbox deployment {label} slot {} expected {:?}, but {:?} carries {:?}.",
                        index + 1,
                        expected,
                        id,
                        actual
                    ));
                }
                Ok(OrderedDeploymentUnit { id, entity })
            })
            .collect()
    }

    Ok((
        validate_side("Party", players, party)?,
        validate_side("Enemy", hostiles, enemies)?,
    ))
}

fn deployment_moves(
    units: &[OrderedDeploymentUnit],
    placements: &[TilePos],
    footing: &Footing,
) -> Result<Vec<(Entity, hex_units::Standing)>, SandboxPlacementRefusal> {
    if units.len() != placements.len() {
        return Err(SandboxPlacementRefusal::RosterChanged);
    }
    units
        .iter()
        .zip(placements)
        .map(|(unit, pos)| {
            footing
                .at(*pos)
                .map(|standing| (unit.entity, standing))
                .ok_or(SandboxPlacementRefusal::InvalidFooting)
        })
        .collect()
}

fn handle_deployment_actions(
    mut intents: MessageReader<UiIntent>,
    mut session: Option<ResMut<DeploymentSession>>,
    mut phase: ResMut<GameplayPhase>,
    mut runtime: DeploymentRuntime,
    mut commands: Commands,
    mut next: ResMut<NextState<Screen>>,
) {
    'intents: for intent in intents.read() {
        let UiIntent::Deployment(action) = intent else {
            continue;
        };
        let Some(session) = session.as_deref_mut() else {
            return;
        };
        match action {
            DeploymentAction::SelectSlot(slot) => {
                let _selected = session.deployment.select_slot(*slot);
            }
            DeploymentAction::Undo => {
                let _undone = session.deployment.undo();
            }
            DeploymentAction::Back => {
                *phase = GameplayPhase::Active;
                next.set(Screen::Sandbox);
                commands.remove_resource::<DeploymentSession>();
                continue;
            }
            DeploymentAction::StartCombat => {
                let identity_issue = if runtime.encounter.is_none() {
                    Some("Encounter is missing")
                } else if runtime.active.is_none() {
                    Some("ActiveScenario is missing")
                } else if runtime.sandbox.is_none() {
                    Some("SandboxSession is missing")
                } else if runtime.accepted.is_none() {
                    Some("AcceptedContentRevision is missing")
                } else if !matches!(
                    runtime.origin.as_deref(),
                    Some(GameplaySessionOrigin::Sandbox)
                ) {
                    Some("Sandbox gameplay provenance is missing or inconsistent")
                } else {
                    None
                };
                if let Some(issue) = identity_issue {
                    error!(
                        "refusing to commit Sandbox deployment because its exact launch identity is incomplete: {issue}"
                    );
                    session
                        .deployment
                        .refuse(SandboxPlacementRefusal::LaunchIdentityUnavailable);
                    continue;
                }
                let (Some(encounter), Some(active), Some(sandbox), Some(accepted)) = (
                    runtime.encounter.as_deref_mut(),
                    runtime.active.as_deref_mut(),
                    runtime.sandbox.as_deref_mut(),
                    runtime.accepted.as_deref(),
                ) else {
                    error!(
                        "refusing to commit Sandbox deployment because its exact launch identity changed during validation"
                    );
                    session
                        .deployment
                        .refuse(SandboxPlacementRefusal::LaunchIdentityUnavailable);
                    continue;
                };
                let expected_loading_input = sandbox.launch.loading_input();
                if active.0 != expected_loading_input
                    || expected_loading_input.encounter_override.as_ref() != Some(&*encounter)
                {
                    error!(
                        "refusing to commit Sandbox deployment because its active scenario, encounter, and frozen launch identity disagree"
                    );
                    session
                        .deployment
                        .refuse(SandboxPlacementRefusal::LaunchIdentityUnavailable);
                    continue;
                }
                let accepted_content_revision = accepted.fingerprint();
                let Some(table) = runtime.markers.table.as_deref() else {
                    session
                        .deployment
                        .refuse(SandboxPlacementRefusal::TerrainUnavailable);
                    continue;
                };
                if !session.complete() {
                    session
                        .deployment
                        .refuse(SandboxPlacementRefusal::Incomplete);
                    continue;
                }
                let footing = deployment_footing(
                    &runtime.markers.tiles,
                    table,
                    runtime.markers.blockers.as_deref(),
                );
                let ordered = ordered_deployment_units(
                    runtime
                        .units
                        .iter_mut()
                        .map(|(entity, id, faction, archetype, _, _)| {
                            (entity, *id, *faction, archetype.0.clone())
                        }),
                    &session.party,
                    &session.enemies,
                );
                let (players, hostiles) = match ordered {
                    Ok(ordered) => ordered,
                    Err(reason) => {
                        error!("refusing Sandbox deployment after roster validation: {reason}");
                        session
                            .deployment
                            .refuse(SandboxPlacementRefusal::RosterChanged);
                        continue;
                    }
                };
                let party_placements = session
                    .deployment
                    .ordered_placements(SandboxSide::Party)
                    .collect::<Vec<_>>();
                let enemy_placements = session
                    .deployment
                    .ordered_placements(SandboxSide::Enemies)
                    .collect::<Vec<_>>();
                let moves = deployment_moves(&players, &party_placements, &footing).and_then(
                    |mut moves| {
                        moves.extend(deployment_moves(&hostiles, &enemy_placements, &footing)?);
                        Ok(moves)
                    },
                );
                let moves = match moves {
                    Ok(moves) => moves,
                    Err(refusal) => {
                        session.deployment.refuse(refusal);
                        continue;
                    }
                };
                for (entity, standing) in moves {
                    let Ok((_, _, _, _, mut on, mut transform)) = runtime.units.get_mut(entity)
                    else {
                        error!(
                            "a Sandbox unit disappeared between exact deployment validation and placement"
                        );
                        session
                            .deployment
                            .refuse(SandboxPlacementRefusal::RosterChanged);
                        continue 'intents;
                    };
                    on.0 = standing;
                    transform.translation = standing.world_position();
                }
                let Some(deployment) = deployment_snapshot(session) else {
                    session
                        .deployment
                        .refuse(SandboxPlacementRefusal::Incomplete);
                    continue;
                };
                let exact = exact_deployed_encounter(session);
                *encounter = exact.clone();
                sandbox.launch.freeze_deployment(
                    deployment,
                    exact,
                    Some(accepted_content_revision),
                );
                active.0 = sandbox.launch.loading_input();
                for mut occlusion in &mut runtime.actor_occlusion {
                    occlusion.remove(PresentationOcclusionReason::SandboxDeployment);
                }
                for entity in &runtime.world_entities {
                    commands.entity(entity).despawn();
                }
                commands.remove_resource::<DeploymentSession>();
                commands.remove_resource::<DeploymentMarkerMaterials>();
                *phase = GameplayPhase::Active;
                continue;
            }
        }
    }
}

fn exact_deployed_encounter(session: &DeploymentSession) -> Encounter {
    let party_placements = session.deployment.ordered_placements(SandboxSide::Party);
    let enemy_placements = session.deployment.ordered_placements(SandboxSide::Enemies);
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
                    .party
                    .iter()
                    .zip(party_placements)
                    .map(|(choice, placement)| {
                        roster_entry(choice, Some(EncounterPlacement::Surface(placement)))
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
                    .enemies
                    .iter()
                    .zip(enemy_placements)
                    .map(|(choice, placement)| {
                        roster_entry(choice, Some(EncounterPlacement::Surface(placement)))
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
    commands.remove_resource::<DeploymentMarkerMaterials>();
}

/// Installs the frozen combined namespace before the accepted-revision publisher.
///
/// This is exclusive so all five resources become visible in the same schedule
/// boundary without relying on deferred-command ordering.
pub(crate) fn apply_creator_content_overlay(world: &mut World) {
    let Some(active) = world
        .get_resource::<CreatorContentOverlay>()
        .map(|overlay| overlay.active.clone())
    else {
        return;
    };
    active.insert_into_world(world);
}

/// Restores the base namespace after a creator session froze combined ids.
pub(crate) fn restore_shipped_content(
    commands: &mut Commands,
    overlay: Option<&CreatorContentOverlay>,
) {
    if let Some(overlay) = overlay {
        overlay.shipped.insert_with_commands(commands);
    }
    commands.remove_resource::<CreatorContentOverlay>();
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
    use std::time::Duration;

    use bevy::camera::NormalizedRenderTarget;
    use bevy::picking::backend::HitData;
    use bevy::picking::pointer::{Location, PointerButton, PointerId};
    use bevy::state::app::StatesPlugin;
    use hex_assets::{
        ArtPalette, ElementFile, SandboxDeploymentRegion, SandboxRegionCenter, SubstanceFile,
        TerrainDamageFile, TerrainDamageTable,
    };
    use hex_core::HexCoord;

    use super::*;

    fn map_definition() -> SandboxMapDefinition {
        SandboxMapDefinition {
            id: "test-map".to_owned(),
            display_name: "Test Map".to_owned(),
            description: "Exact deployment test map.".to_owned(),
            tags: vec!["Test".to_owned()],
            preview: "ui/sandbox/flat-arena.png".to_owned(),
            scenario: "Ability Lab".to_owned(),
            fixed_seed: Some(42),
            player_region: SandboxDeploymentRegion {
                center: SandboxRegionCenter::Fixed(hex_assets::CubeCoord { x: 0, y: 0, z: 0 }),
                radius: 1,
            },
            hostile_region: SandboxDeploymentRegion {
                center: SandboxRegionCenter::Fixed(hex_assets::CubeCoord { x: 2, y: -2, z: 0 }),
                radius: 1,
            },
        }
    }

    fn deployment_session(
        definition: SandboxMapDefinition,
        party: Vec<RosterChoice>,
        enemies: Vec<RosterChoice>,
    ) -> DeploymentSession {
        let mut party_choices = party.iter().cloned();
        let mut enemy_choices = enemies.iter().cloned();
        let draft = hex_gameplay_model::SandboxDraft::<CustomCharacterId> {
            map: None,
            party: std::array::from_fn(|_| party_choices.next()),
            enemies: std::array::from_fn(|_| enemy_choices.next()),
        };
        DeploymentSession::new(
            definition,
            party,
            enemies,
            SandboxDeploymentModel::from_draft(&draft),
        )
    }

    fn trigger_primary_click(app: &mut App, target: Entity) {
        let window = app.world_mut().spawn_empty().id();
        let target_window = bevy::window::WindowRef::Entity(window)
            .normalize(Some(window))
            .expect("the explicit window target normalizes");
        let location = Location {
            target: NormalizedRenderTarget::Window(target_window),
            position: Vec2::ZERO,
        };
        let click = Click {
            button: PointerButton::Primary,
            hit: HitData::new(target, 0.0, None, None),
            duration: Duration::from_millis(1),
            count: 1,
        };
        app.world_mut()
            .trigger(Pointer::new(PointerId::Mouse, location, click, target));
    }

    fn sandbox_navigation_app(initial: Screen) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .insert_state(initial)
            .add_message::<UiIntent>()
            .init_resource::<SandboxState>()
            .init_resource::<SandboxNotice>()
            .init_resource::<SandboxSeedSequence>()
            .init_resource::<CreationStore>()
            .init_resource::<MainMenuModel>()
            .add_systems(OnEnter(Screen::Sandbox), initialize_sandbox)
            .add_systems(OnEnter(Screen::Title), cleanup_sandbox_before_main_menu)
            .add_systems(
                Update,
                handle_sandbox_intents.run_if(in_state(Screen::Sandbox)),
            );
        app
    }

    #[test]
    fn unrelated_click_is_safe_before_deployment_assets_arrive() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_observer(on_deployment_tile_clicked);
        let target = app.world_mut().spawn_empty().id();
        trigger_primary_click(&mut app, target);
    }

    #[test]
    fn creator_origin_back_restores_the_exact_creator_navigation() {
        let mut app = sandbox_navigation_app(Screen::Sandbox);
        let navigation = CreatorNavigation {
            tab: hex_gameplay_model::CreatorSurface::Spells,
            origin: hex_gameplay_model::CreatorOrigin::SandboxCharacterPicker {
                side: SandboxSide::Enemies,
                slot: SandboxSlotIndex::Four,
            },
            parent_surface: Some(hex_gameplay_model::CreatorSurface::Characters),
        };
        app.insert_resource(super::super::creator::CreatorSandboxReturn { navigation });
        app.insert_resource(CreatorOpenSandboxRequest {
            character: CustomCharacterId(42),
        });

        app.update();

        assert_eq!(
            app.world().resource::<SandboxState>().entry_origin,
            SandboxEntryOrigin::Creator
        );
        assert_eq!(
            app.world()
                .resource::<super::super::creator::CreatorSandboxReturn>()
                .navigation,
            navigation
        );

        app.world_mut()
            .write_message(UiIntent::Sandbox(SandboxIntent::Back));
        app.update();
        assert_eq!(
            app.world().resource::<SandboxState>().route,
            SandboxRoute::Overview
        );

        app.world_mut()
            .write_message(UiIntent::Sandbox(SandboxIntent::Back));
        app.update();

        assert_eq!(
            app.world()
                .resource::<super::super::creator::CreatorRestoreRequest>()
                .0,
            navigation
        );
        assert!(!app
            .world()
            .contains_resource::<super::super::creator::CreatorSandboxReturn>());
        app.update();
        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::CharacterCreator
        );
    }

    #[test]
    fn main_menu_exit_and_reentry_preserve_the_in_memory_draft() {
        let mut app = sandbox_navigation_app(Screen::Sandbox);
        app.update();
        {
            let mut state = app.world_mut().resource_mut::<SandboxState>();
            state.draft.map = Some(SandboxMapSelection::new("procedural-hills", Some(913)));
            state.open_character_picker(SandboxSide::Party, SandboxSlotIndex::Five);
            assert!(state.preview_character(RosterChoice::Template("wolf".to_owned())));
            assert!(state.use_previewed_character());
            state.open_character_picker(SandboxSide::Enemies, SandboxSlotIndex::Three);
            assert!(state.preview_character(RosterChoice::Template("hedge-mage".to_owned())));
            assert!(state.use_previewed_character());
            state.enter(SandboxEntryOrigin::MainMenu);
        }
        let expected = app.world().resource::<SandboxState>().draft.clone();

        app.world_mut()
            .write_message(UiIntent::Sandbox(SandboxIntent::Back));
        app.update();
        app.update();
        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Title
        );

        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Sandbox);
        app.update();
        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Sandbox
        );
        let state = app.world().resource::<SandboxState>();
        assert_eq!(state.draft, expected);
        assert_eq!(state.route, SandboxRoute::Overview);
        assert_eq!(state.entry_origin, SandboxEntryOrigin::MainMenu);
    }

    #[test]
    fn view_and_start_handler_share_scenario_and_combat_readiness() {
        let maps: SandboxMapCatalog = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/sandbox_maps.ron"
        )))
        .expect("sandbox_maps.ron parses");
        let combat: CombatSettings = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/combat.ron"
        )))
        .expect("combat.ron parses");
        let scenarios: ScenarioLibrary = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/scenarios.ron"
        )))
        .expect("scenarios.ron parses");
        let mut app = sandbox_navigation_app(Screen::Sandbox);
        app.init_resource::<SandboxView>()
            .insert_resource(maps)
            .insert_resource(combat.clone())
            .add_systems(
                Update,
                publish_sandbox_view
                    .after(handle_sandbox_intents)
                    .run_if(in_state(Screen::Sandbox)),
            );
        app.update();

        for (scenario_catalog, combat_catalog) in [
            (None, Some(combat.clone())),
            (Some(scenarios.clone()), None),
        ] {
            if let Some(scenarios) = scenario_catalog {
                app.insert_resource(scenarios);
            } else {
                app.world_mut().remove_resource::<ScenarioLibrary>();
            }
            if let Some(combat) = combat_catalog {
                app.insert_resource(combat);
            } else {
                app.world_mut().remove_resource::<CombatSettings>();
            }
            app.world_mut()
                .write_message(UiIntent::Sandbox(SandboxIntent::StartSandbox));
            app.update();

            assert_eq!(
                app.world().resource::<SandboxView>().start_blocker,
                Some(SandboxStartBlocker::MapsLoading)
            );
            assert_eq!(
                app.world().resource::<SandboxNotice>().0.as_deref(),
                Some("Sandbox maps are still loading.")
            );
            assert!(matches!(
                app.world().resource::<NextState<Screen>>(),
                &NextState::Unchanged
            ));
        }

        let mut missing_selected_scenario = scenarios;
        let selected_scenario = app
            .world()
            .resource::<SandboxMapCatalog>()
            .get("flat-arena")
            .map(|definition| definition.scenario.clone())
            .expect("the default map exists");
        missing_selected_scenario
            .scenarios
            .retain(|scenario| scenario.name != selected_scenario);
        app.insert_resource(missing_selected_scenario);
        app.insert_resource(combat);
        app.world_mut()
            .write_message(UiIntent::Sandbox(SandboxIntent::StartSandbox));
        app.update();
        assert_eq!(
            app.world().resource::<SandboxView>().start_blocker,
            Some(SandboxStartBlocker::MapUnavailable)
        );
        assert_eq!(
            app.world().resource::<SandboxNotice>().0.as_deref(),
            Some("The selected map is unavailable.")
        );
    }

    #[test]
    fn regenerated_seeds_change_without_losing_determinism_or_map_identity() {
        let mut first = SandboxSeedSequence::default();
        let mut replay = SandboxSeedSequence::default();
        let one = first.next("procedural-hills");
        let two = first.next("procedural-hills");
        assert_ne!(one, two);
        assert_eq!(one, replay.next("procedural-hills"));
        assert_ne!(one, SandboxSeedSequence::default().next("frozen-hills"));
    }

    #[test]
    fn deployment_guides_stable_order_and_freezes_exact_stacked_surfaces() {
        let mut session = deployment_session(
            map_definition(),
            vec![
                RosterChoice::Template("wolf".to_owned()),
                RosterChoice::Template("hedge-mage".to_owned()),
            ],
            vec![RosterChoice::Template("raider".to_owned())],
        );
        let first_party = TilePos::new(HexCoord::ORIGIN, 3);
        let second_party = TilePos::new(HexCoord::from_axial(1, 0), 3);
        let stacked_enemy = TilePos::new(first_party.coord, first_party.level + 1);
        assert_eq!(
            session.deployment.place_validated(first_party),
            Ok(SandboxDeploymentStage::Placing(SandboxDeploymentSlot::new(
                SandboxSide::Party,
                SandboxSlotIndex::Two
            )))
        );
        assert_eq!(
            session.deployment.place_validated(second_party),
            Ok(SandboxDeploymentStage::Placing(SandboxDeploymentSlot::new(
                SandboxSide::Enemies,
                SandboxSlotIndex::One
            )))
        );
        assert!(matches!(
            session.deployment.place_validated(first_party),
            Err(SandboxPlacementRefusal::Occupied { .. })
        ));
        assert_eq!(
            session.deployment.place_validated(stacked_enemy),
            Ok(SandboxDeploymentStage::Review)
        );
        assert!(
            session.complete(),
            "stacked surfaces at distinct elevations remain distinct"
        );
        let first_party_slot =
            SandboxDeploymentSlot::new(SandboxSide::Party, SandboxSlotIndex::One);
        assert!(session.deployment.select_slot(first_party_slot));
        session
            .deployment
            .place_validated(TilePos::new(HexCoord::from_axial(-1, 0), 3))
            .expect("a Review reposition returns to Review");
        assert!(session.deployment.undo());
        assert!(
            session.complete(),
            "undoing a Review reposition must restore the Start Combat gate"
        );
        assert_eq!(
            resolved_deployment_markers(&session).collect::<Vec<_>>(),
            vec![
                (
                    SandboxDeploymentSlot::new(SandboxSide::Party, SandboxSlotIndex::One),
                    first_party
                ),
                (
                    SandboxDeploymentSlot::new(SandboxSide::Party, SandboxSlotIndex::Two),
                    second_party
                ),
                (
                    SandboxDeploymentSlot::new(SandboxSide::Enemies, SandboxSlotIndex::One),
                    stacked_enemy
                )
            ]
        );
        assert_eq!(
            deployment_snapshot(&session),
            Some(SandboxDeploymentSnapshot {
                party: vec![first_party, second_party],
                enemies: vec![stacked_enemy],
            })
        );

        let encounter = exact_deployed_encounter(&session);
        let party_entries = encounter.rosters.first().map(|roster| &roster.units);
        let enemy_entries = encounter.rosters.get(1).map(|roster| &roster.units);
        assert_eq!(
            party_entries.map(|entries| {
                entries
                    .iter()
                    .map(|entry| (entry.archetype.as_str(), entry.placement.clone()))
                    .collect::<Vec<_>>()
            }),
            Some(vec![
                ("wolf", Some(EncounterPlacement::Surface(first_party))),
                (
                    "hedge-mage",
                    Some(EncounterPlacement::Surface(second_party))
                ),
            ])
        );
        assert_eq!(
            enemy_entries.and_then(|entries| entries.first()),
            Some(&RosterEntry {
                archetype: "raider".to_owned(),
                placement: Some(EncounterPlacement::Surface(stacked_enemy)),
                ai_profile: None,
                ai_group: None,
            })
        );
    }

    #[test]
    fn terrain_clicks_place_outside_legacy_regions_and_refuse_invalid_or_occupied_surfaces() {
        let fixture = content_fixture().expect("the shipped content fixture should resolve");
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_observer(on_deployment_tile_clicked)
            .insert_resource(fixture.substances.clone())
            .insert_resource(deployment_session(
                map_definition(),
                vec![RosterChoice::Template("hedge-mage".to_owned())],
                vec![RosterChoice::Template("raider".to_owned())],
            ));
        let grass = fixture
            .substances
            .id("grass")
            .expect("the fixture has standable grass");
        let outside_party_region = TilePos::new(HexCoord::from_axial(8, -3), 1);
        let outside_enemy_region = TilePos::new(HexCoord::from_axial(-7, 2), 1);
        let invalid = TilePos::new(HexCoord::from_axial(9, -3), 1);
        let valid_span = HexSpan::from_ground(1.0);
        let first = app
            .world_mut()
            .spawn((
                HexTile,
                outside_party_region,
                valid_span,
                grass,
                Headroom(10),
            ))
            .id();
        let second = app
            .world_mut()
            .spawn((
                HexTile,
                outside_enemy_region,
                valid_span,
                grass,
                Headroom(10),
            ))
            .id();
        let invalid_target = app
            .world_mut()
            .spawn((HexTile, invalid, valid_span, grass, Headroom(0)))
            .id();

        trigger_primary_click(&mut app, first);
        let session = app.world().resource::<DeploymentSession>();
        assert_eq!(
            session.deployment.active_slot(),
            Some(SandboxDeploymentSlot::new(
                SandboxSide::Enemies,
                SandboxSlotIndex::One
            ))
        );

        trigger_primary_click(&mut app, first);
        assert!(matches!(
            app.world()
                .resource::<DeploymentSession>()
                .deployment
                .refusal,
            Some(SandboxPlacementRefusal::Occupied { .. })
        ));
        trigger_primary_click(&mut app, invalid_target);
        assert_eq!(
            app.world()
                .resource::<DeploymentSession>()
                .deployment
                .refusal,
            Some(SandboxPlacementRefusal::InvalidFooting)
        );
        trigger_primary_click(&mut app, second);
        assert_eq!(
            app.world().resource::<DeploymentSession>().deployment.stage,
            SandboxDeploymentStage::Review
        );
        assert!(app.world().resource::<DeploymentSession>().complete());
    }

    #[test]
    fn deployment_associates_heterogeneous_rosters_by_unit_id_on_launch_and_retry() {
        let party = vec![
            RosterChoice::Template("hedge-mage".to_owned()),
            RosterChoice::Template("wolf".to_owned()),
        ];
        let enemies = vec![
            RosterChoice::Template("raider".to_owned()),
            RosterChoice::Template("hedge-mage".to_owned()),
        ];
        let mut first_world = World::new();
        let entity_for_party_two = first_world.spawn_empty().id();
        let entity_for_enemy_two = first_world.spawn_empty().id();
        let entity_for_enemy_one = first_world.spawn_empty().id();
        let entity_for_party_one = first_world.spawn_empty().id();
        let first = ordered_deployment_units(
            [
                (
                    entity_for_party_two,
                    UnitId(1),
                    Faction::Player,
                    "wolf".to_owned(),
                ),
                (
                    entity_for_enemy_two,
                    UnitId(3),
                    Faction::Hostile,
                    "hedge-mage".to_owned(),
                ),
                (
                    entity_for_enemy_one,
                    UnitId(2),
                    Faction::Hostile,
                    "raider".to_owned(),
                ),
                (
                    entity_for_party_one,
                    UnitId(0),
                    Faction::Player,
                    "hedge-mage".to_owned(),
                ),
            ],
            &party,
            &enemies,
        )
        .expect("the first launch should resolve by stable id");
        assert_eq!(
            first.0,
            vec![
                OrderedDeploymentUnit {
                    id: UnitId(0),
                    entity: entity_for_party_one,
                },
                OrderedDeploymentUnit {
                    id: UnitId(1),
                    entity: entity_for_party_two,
                },
            ]
        );
        assert_eq!(
            first.1,
            vec![
                OrderedDeploymentUnit {
                    id: UnitId(2),
                    entity: entity_for_enemy_one,
                },
                OrderedDeploymentUnit {
                    id: UnitId(3),
                    entity: entity_for_enemy_two,
                },
            ]
        );

        // Retry Exact may allocate different ECS entities, but the same frozen
        // encounter deals the same stable ids and therefore preserves slot order.
        let mut retry_world = World::new();
        let retry_enemy_one = retry_world.spawn_empty().id();
        let retry_party_one = retry_world.spawn_empty().id();
        let retry_party_two = retry_world.spawn_empty().id();
        let retry_enemy_two = retry_world.spawn_empty().id();
        let retry = ordered_deployment_units(
            [
                (
                    retry_enemy_two,
                    UnitId(3),
                    Faction::Hostile,
                    "hedge-mage".to_owned(),
                ),
                (
                    retry_party_two,
                    UnitId(1),
                    Faction::Player,
                    "wolf".to_owned(),
                ),
                (
                    retry_party_one,
                    UnitId(0),
                    Faction::Player,
                    "hedge-mage".to_owned(),
                ),
                (
                    retry_enemy_one,
                    UnitId(2),
                    Faction::Hostile,
                    "raider".to_owned(),
                ),
            ],
            &party,
            &enemies,
        )
        .expect("Retry Exact should resolve the same slots by stable id");
        assert_eq!(
            retry
                .0
                .iter()
                .chain(&retry.1)
                .map(|unit| unit.id)
                .collect::<Vec<_>>(),
            vec![UnitId(0), UnitId(1), UnitId(2), UnitId(3)]
        );
        let [party_one, party_two] = retry.0.as_slice() else {
            panic!("Retry Exact should retain exactly two Party slots");
        };
        let [enemy_one, enemy_two] = retry.1.as_slice() else {
            panic!("Retry Exact should retain exactly two Enemy slots");
        };
        assert_eq!(party_one.entity, retry_party_one);
        assert_eq!(party_two.entity, retry_party_two);
        assert_eq!(enemy_one.entity, retry_enemy_one);
        assert_eq!(enemy_two.entity, retry_enemy_two);
    }

    #[test]
    fn deployment_refuses_count_or_archetype_drift_before_moving_any_unit() {
        let party = vec![
            RosterChoice::Template("hedge-mage".to_owned()),
            RosterChoice::Template("wolf".to_owned()),
        ];
        let enemies = vec![RosterChoice::Template("raider".to_owned())];
        let mut world = World::new();
        let one = world.spawn_empty().id();
        let two = world.spawn_empty().id();

        let count_error = ordered_deployment_units(
            [
                (one, UnitId(0), Faction::Player, "hedge-mage".to_owned()),
                (two, UnitId(2), Faction::Hostile, "raider".to_owned()),
            ],
            &party,
            &enemies,
        )
        .expect_err("a missing Party entity must refuse exact deployment");
        assert_eq!(
            count_error,
            "Sandbox deployment expected 2 Party units, but found 1."
        );

        let three = world.spawn_empty().id();
        let archetype_error = ordered_deployment_units(
            [
                (one, UnitId(0), Faction::Player, "wolf".to_owned()),
                (three, UnitId(1), Faction::Player, "wolf".to_owned()),
                (two, UnitId(2), Faction::Hostile, "raider".to_owned()),
            ],
            &party,
            &enemies,
        )
        .expect_err("heterogeneous roster drift must refuse exact deployment");
        assert!(archetype_error.contains("Party slot 1 expected \"hedge-mage\""));
        assert!(archetype_error.contains("UnitId(0) carries \"wolf\""));
    }

    #[test]
    fn sandbox_encounter_preserves_duplicate_order_and_catalog_staging_regions() {
        let definition = map_definition();
        let party = vec![
            RosterChoice::Template("hedge-mage".to_owned()),
            RosterChoice::Template("hedge-mage".to_owned()),
        ];
        let enemies = vec![RosterChoice::Template("raider".to_owned())];
        let encounter = sandbox_encounter(&party, &enemies, &definition);
        assert_eq!(encounter.name, "Creator Sandbox · Test Map");
        assert_eq!(
            encounter.rosters.first().map(|roster| {
                roster
                    .units
                    .iter()
                    .map(|entry| entry.archetype.as_str())
                    .collect::<Vec<_>>()
            }),
            Some(vec!["hedge-mage", "hedge-mage"])
        );
        assert_eq!(
            encounter
                .rosters
                .get(1)
                .and_then(|roster| roster.units.first())
                .map(|entry| entry.archetype.as_str()),
            Some("raider")
        );
    }

    struct ContentFixture {
        element_file: ElementFile,
        elements: ElementCatalog,
        substance_file: SubstanceFile,
        palette: ArtPalette,
        substances: SubstanceTable,
        shipped: CreatorContentBundle,
        active: CreatorContentBundle,
    }

    fn bundle(
        spell_file: SpellFile,
        lattice_file: LatticeFile,
        elements: &ElementCatalog,
        substances: &SubstanceTable,
    ) -> Result<CreatorContentBundle, String> {
        let spells = SpellBook::from_file(&spell_file);
        let content = ContentIndex::build(elements, &spells, substances).map_err(|errors| {
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        })?;
        let lattices =
            LatticeLibrary::build(&lattice_file, elements, &spells).map_err(|errors| {
                errors
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ")
            })?;
        Ok(CreatorContentBundle {
            spell_file,
            spells,
            lattice_file,
            content,
            lattices,
        })
    }

    fn content_fixture() -> Result<ContentFixture, String> {
        let element_file: ElementFile = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/elements.ron"
        )))
        .map_err(|error| error.to_string())?;
        let shipped_spell_file: SpellFile = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/spells.ron"
        )))
        .map_err(|error| error.to_string())?;
        let lattice_file: LatticeFile = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/lattices.ron"
        )))
        .map_err(|error| error.to_string())?;
        let substance_file: SubstanceFile = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/substances.ron"
        )))
        .map_err(|error| error.to_string())?;
        let palette: ArtPalette = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/art/palette.ron"
        )))
        .map_err(|error| error.to_string())?;

        let elements = ElementCatalog::from_file(&element_file);
        let substances = SubstanceTable::from_file(&substance_file, &palette)
            .map_err(|error| error.to_string())?;
        let shipped = bundle(
            shipped_spell_file.clone(),
            lattice_file.clone(),
            &elements,
            &substances,
        )?;
        let mut creator_spell_file = shipped_spell_file;
        let fixture_spell = creator_spell_file
            .spells
            .values()
            .next()
            .cloned()
            .ok_or_else(|| "shipped spell fixture is empty".to_owned())?;
        creator_spell_file
            .spells
            .insert("Creator Acceptance Test".to_owned(), fixture_spell);
        let active = bundle(creator_spell_file, lattice_file, &elements, &substances)?;
        Ok(ContentFixture {
            element_file,
            elements,
            substance_file,
            palette,
            substances,
            shipped,
            active,
        })
    }

    #[test]
    fn creator_preflight_uses_the_same_fresh_cast_gate_as_sandbox_start() {
        let fixture = content_fixture().expect("the shipped content fixture should resolve");
        let presets: CreationPresetCatalog = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/creation_presets.ron"
        )))
        .expect("the shipped Creator presets should parse");
        let mut character = presets
            .characters
            .iter()
            .find(|record| record.key == "template-hedge-mage")
            .expect("the Hedge Mage template should exist")
            .character
            .clone();
        for capacity in character.attunement.values_mut() {
            *capacity = 0;
        }
        let mut library = hex_assets::CreationLibraryFile::default();
        library.characters.push(character.clone());
        let shipped_book = SpellBook::from_file(&fixture.shipped.spell_file);
        assert!(super::super::creator::character_map_issues(
            &character,
            &library,
            &fixture.elements,
            &shipped_book,
        )
        .is_empty());

        let refusal = creator_character_map_readiness(
            character.id,
            &library,
            Some(&fixture.shipped.spell_file),
            Some(&fixture.shipped.lattice_file),
            Some(&fixture.elements),
            Some(&fixture.substances),
        )
        .expect_err("zero starting mana must block Creator's Sandbox preflight");
        assert!(refusal.contains("castable from a fresh lattice"));
    }

    fn app_with_content_fixture(fixture: &ContentFixture) -> App {
        let mut app = App::new();
        let terrain_damage_file = TerrainDamageFile {
            damaging_pairs: Vec::new(),
        };
        let terrain_damage = TerrainDamageTable::from_file(
            &terrain_damage_file,
            &fixture.elements,
            &fixture.substances,
        )
        .expect("the empty terrain-damage fixture should resolve");
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.insert_state(Screen::Loading);
        app.insert_resource(fixture.element_file.clone());
        app.insert_resource(fixture.elements.clone());
        app.insert_resource(fixture.substance_file.clone());
        app.insert_resource(fixture.palette.clone());
        app.insert_resource(fixture.substances.clone());
        app.insert_resource(terrain_damage_file);
        app.insert_resource(terrain_damage);
        fixture.shipped.insert_into_world(app.world_mut());
        app.add_plugins(hex_assets::content_index::plugin);
        app.add_systems(
            PostUpdate,
            apply_creator_content_overlay
                .before(hex_assets::ContentReadinessSystems::PublishAcceptedRevision),
        );
        app
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum MissingDeploymentIdentity {
        Encounter,
        ActiveScenario,
        SandboxSession,
        AcceptedContentRevision,
        GameplaySessionOrigin,
    }

    impl MissingDeploymentIdentity {
        fn remove(self, world: &mut World) {
            let removed = match self {
                Self::Encounter => world.remove_resource::<Encounter>().is_some(),
                Self::ActiveScenario => world
                    .remove_resource::<crate::scenarios::ActiveScenario>()
                    .is_some(),
                Self::SandboxSession => world.remove_resource::<SandboxSession>().is_some(),
                Self::AcceptedContentRevision => {
                    world.remove_resource::<AcceptedContentRevision>().is_some()
                }
                Self::GameplaySessionOrigin => {
                    world.remove_resource::<GameplaySessionOrigin>().is_some()
                }
            };
            assert!(removed, "{self:?} must exist before the refusal case");
        }
    }

    struct DeploymentIdentityFixture {
        app: App,
        party_unit: Entity,
        deployment_world_entity: Entity,
        original_standing: hex_units::Standing,
        original_transform: Transform,
        encounter: Encounter,
        active: ScenarioToLoad,
        sandbox: SandboxSession,
        accepted_fingerprint: u64,
    }

    fn deployment_identity_fixture() -> DeploymentIdentityFixture {
        let fixture = content_fixture().expect("the shipped content fixture should resolve");
        let mut app = app_with_content_fixture(&fixture);
        app.add_message::<UiIntent>()
            .insert_resource(GameAssets {
                hex_tile: Handle::default(),
                player_pieces: [Handle::default(), Handle::default()],
            })
            .insert_resource(PlayerSettings {
                scale: 1.0,
                speed: 1.0,
            })
            .insert_resource(GameplayPhase::Deployment)
            .insert_resource(GameplaySessionOrigin::Sandbox)
            .add_systems(Update, handle_deployment_actions);
        app.update();

        let accepted_fingerprint = app
            .world()
            .resource::<AcceptedContentRevision>()
            .fingerprint();
        let rules: CombatSettings = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/combat.ron"
        )))
        .expect("combat.ron parses");
        let library: ScenarioLibrary = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/scenarios.ron"
        )))
        .expect("scenarios.ron parses");
        let scenario = scenario_named(&library, "Ability Lab").expect("fixture scenario exists");
        let definition = map_definition();
        let party = vec![RosterChoice::Template("hedge-mage".to_owned())];
        let enemies = vec![RosterChoice::Template("raider".to_owned())];
        let encounter = sandbox_encounter(&party, &enemies, &definition);
        let launch = SandboxLaunchSnapshot::new(
            SandboxMapSelection::new(definition.id.clone(), definition.fixed_seed),
            definition.scenario.clone(),
            party.clone(),
            enemies.clone(),
            Some(accepted_fingerprint),
            rules,
            scenario,
            encounter.clone(),
        );
        let active = launch.loading_input();
        let sandbox = SandboxSession { launch };

        let party_surface = TilePos::new(HexCoord::ORIGIN, 1);
        let enemy_surface = TilePos::new(HexCoord::from_axial(2, -2), 1);
        let span = HexSpan::from_ground(1.0);
        let grass = fixture
            .substances
            .id("grass")
            .expect("the fixture has standable grass");
        for position in [party_surface, enemy_surface] {
            app.world_mut()
                .spawn((HexTile, position, span, grass, Headroom(10)));
        }

        let original_standing = hex_units::Standing {
            pos: TilePos::new(HexCoord::from_axial(-4, 4), 1),
            span,
        };
        let original_transform = Transform::from_xyz(-17.0, 3.0, 11.0);
        let party_unit = app
            .world_mut()
            .spawn((
                UnitId(0),
                Faction::Player,
                Archetype("hedge-mage".to_owned()),
                StandsOn(original_standing),
                original_transform,
                Visibility::Hidden,
                PresentationOcclusion::from_reason(PresentationOcclusionReason::SandboxDeployment),
            ))
            .id();
        app.world_mut().spawn((
            UnitId(
                u64::try_from(hex_gameplay_model::SANDBOX_ROSTER_SIZE)
                    .expect("the roster bound fits u64"),
            ),
            Faction::Hostile,
            Archetype("raider".to_owned()),
            StandsOn(hex_units::Standing {
                pos: TilePos::new(HexCoord::from_axial(4, -4), 1),
                span,
            }),
            Transform::from_xyz(17.0, 3.0, -11.0),
            Visibility::Hidden,
            PresentationOcclusion::from_reason(PresentationOcclusionReason::SandboxDeployment),
        ));
        let deployment_world_entity = app.world_mut().spawn(DeploymentWorldEntity).id();
        app.insert_resource(DeploymentMarkerMaterials {
            player: Handle::default(),
            hostile: Handle::default(),
        });
        let mut deployment = deployment_session(definition, party, enemies);
        deployment
            .deployment
            .place_validated(party_surface)
            .expect("the Party surface is exact");
        deployment
            .deployment
            .place_validated(enemy_surface)
            .expect("the Enemy surface completes Review");
        assert!(deployment.complete(), "the refusal fixture is commit-ready");
        app.insert_resource(deployment);
        app.insert_resource(encounter.clone());
        app.insert_resource(crate::scenarios::ActiveScenario(active.clone()));
        app.insert_resource(sandbox.clone());

        DeploymentIdentityFixture {
            app,
            party_unit,
            deployment_world_entity,
            original_standing,
            original_transform,
            encounter,
            active,
            sandbox,
            accepted_fingerprint,
        }
    }

    #[test]
    fn deployment_refuses_each_missing_launch_identity_without_partial_commit() {
        for missing in [
            MissingDeploymentIdentity::Encounter,
            MissingDeploymentIdentity::ActiveScenario,
            MissingDeploymentIdentity::SandboxSession,
            MissingDeploymentIdentity::AcceptedContentRevision,
            MissingDeploymentIdentity::GameplaySessionOrigin,
        ] {
            let mut fixture = deployment_identity_fixture();
            missing.remove(fixture.app.world_mut());
            fixture
                .app
                .world_mut()
                .write_message(UiIntent::Deployment(DeploymentAction::StartCombat));
            fixture.app.update();

            let world = fixture.app.world();
            assert_eq!(
                world.resource::<DeploymentSession>().deployment.refusal,
                Some(SandboxPlacementRefusal::LaunchIdentityUnavailable),
                "{missing:?} must produce the visible identity refusal"
            );
            assert_eq!(
                *world.resource::<GameplayPhase>(),
                GameplayPhase::Deployment,
                "{missing:?} must keep deployment active"
            );
            assert!(
                world.get_entity(fixture.deployment_world_entity).is_ok(),
                "{missing:?} must not tear down the deployment world"
            );
            assert!(
                world.contains_resource::<DeploymentMarkerMaterials>(),
                "{missing:?} must not tear down deployment materials"
            );
            let party_unit = world
                .get_entity(fixture.party_unit)
                .expect("the Party unit must remain spawned");
            assert_eq!(
                party_unit.get::<StandsOn>().map(|standing| standing.0),
                Some(fixture.original_standing),
                "{missing:?} must not partially move actors"
            );
            assert_eq!(
                party_unit.get::<Transform>(),
                Some(&fixture.original_transform),
                "{missing:?} must not partially move presentation"
            );
            assert_eq!(party_unit.get::<Visibility>(), Some(&Visibility::Hidden));
            assert!(party_unit.get::<PresentationOcclusion>().is_some_and(
                |occlusion| occlusion.contains(PresentationOcclusionReason::SandboxDeployment)
            ));

            if missing == MissingDeploymentIdentity::Encounter {
                assert!(!world.contains_resource::<Encounter>());
            } else {
                assert_eq!(world.resource::<Encounter>(), &fixture.encounter);
            }
            if missing == MissingDeploymentIdentity::ActiveScenario {
                assert!(!world.contains_resource::<crate::scenarios::ActiveScenario>());
            } else {
                assert_eq!(
                    world.resource::<crate::scenarios::ActiveScenario>().0,
                    fixture.active
                );
            }
            if missing == MissingDeploymentIdentity::SandboxSession {
                assert!(!world.contains_resource::<SandboxSession>());
            } else {
                assert_eq!(world.resource::<SandboxSession>(), &fixture.sandbox);
                assert_eq!(world.resource::<SandboxSession>().launch.deployment, None);
            }
            assert_eq!(
                world.resource::<AcceptedContentRevision>().fingerprint(),
                fixture.accepted_fingerprint
            );
            if missing == MissingDeploymentIdentity::GameplaySessionOrigin {
                assert!(!world.contains_resource::<GameplaySessionOrigin>());
            } else {
                assert_eq!(
                    world.resource::<GameplaySessionOrigin>(),
                    &GameplaySessionOrigin::Sandbox
                );
            }
            assert!(matches!(
                world.resource::<NextState<Screen>>(),
                &NextState::Unchanged
            ));
        }
    }

    #[test]
    fn deployment_refuses_divergent_launch_identity_without_partial_commit() {
        let mut fixture = deployment_identity_fixture();
        let mut divergent_active = fixture.active.clone();
        divergent_active.resolved_seed = Some(ResolvedMapSeed(9_999));
        fixture
            .app
            .insert_resource(crate::scenarios::ActiveScenario(divergent_active.clone()));
        fixture
            .app
            .world_mut()
            .write_message(UiIntent::Deployment(DeploymentAction::StartCombat));
        fixture.app.update();

        let world = fixture.app.world();
        assert_eq!(
            world.resource::<DeploymentSession>().deployment.refusal,
            Some(SandboxPlacementRefusal::LaunchIdentityUnavailable)
        );
        assert_eq!(
            *world.resource::<GameplayPhase>(),
            GameplayPhase::Deployment
        );
        assert!(world.get_entity(fixture.deployment_world_entity).is_ok());
        assert!(world.contains_resource::<DeploymentMarkerMaterials>());
        let party_unit = world
            .get_entity(fixture.party_unit)
            .expect("the Party unit must remain spawned");
        assert_eq!(
            party_unit.get::<StandsOn>().map(|standing| standing.0),
            Some(fixture.original_standing)
        );
        assert_eq!(
            party_unit.get::<Transform>(),
            Some(&fixture.original_transform)
        );
        assert_eq!(party_unit.get::<Visibility>(), Some(&Visibility::Hidden));
        assert!(party_unit.get::<PresentationOcclusion>().is_some_and(
            |occlusion| occlusion.contains(PresentationOcclusionReason::SandboxDeployment)
        ));
        assert_eq!(world.resource::<Encounter>(), &fixture.encounter);
        assert_eq!(
            world.resource::<crate::scenarios::ActiveScenario>().0,
            divergent_active,
            "the refusal must not overwrite the divergent identity"
        );
        assert_eq!(world.resource::<SandboxSession>(), &fixture.sandbox);
        assert_eq!(world.resource::<SandboxSession>().launch.deployment, None);
        assert_eq!(
            world.resource::<AcceptedContentRevision>().fingerprint(),
            fixture.accepted_fingerprint
        );
        assert_eq!(
            world.resource::<GameplaySessionOrigin>(),
            &GameplaySessionOrigin::Sandbox
        );
        assert!(matches!(
            world.resource::<NextState<Screen>>(),
            &NextState::Unchanged
        ));
    }

    #[test]
    fn successful_deployment_commit_removes_only_its_occlusion_reason_and_freezes_retry() {
        let mut fixture = deployment_identity_fixture();
        let expected_deployment =
            deployment_snapshot(fixture.app.world().resource::<DeploymentSession>())
                .expect("the complete deployment fixture must expose exact surfaces");
        fixture
            .app
            .world_mut()
            .entity_mut(fixture.party_unit)
            .get_mut::<PresentationOcclusion>()
            .expect("Sandbox actors carry composable presentation state")
            .insert(PresentationOcclusionReason::CharacterCameraProximity);

        fixture
            .app
            .world_mut()
            .write_message(UiIntent::Deployment(DeploymentAction::StartCombat));
        fixture.app.update();

        let world = fixture.app.world();
        assert_eq!(*world.resource::<GameplayPhase>(), GameplayPhase::Active);
        assert!(!world.contains_resource::<DeploymentSession>());
        assert!(!world.contains_resource::<DeploymentMarkerMaterials>());
        assert!(world.get_entity(fixture.deployment_world_entity).is_err());

        let party_unit = world
            .get_entity(fixture.party_unit)
            .expect("the deployed Party unit remains spawned");
        assert_eq!(
            party_unit.get::<StandsOn>().map(|standing| standing.0.pos),
            expected_deployment.party.first().copied()
        );
        assert_eq!(party_unit.get::<Visibility>(), Some(&Visibility::Hidden));
        let occlusion = party_unit
            .get::<PresentationOcclusion>()
            .expect("the actor keeps its composable presentation state");
        assert!(!occlusion.contains(PresentationOcclusionReason::SandboxDeployment));
        assert!(occlusion.contains(PresentationOcclusionReason::CharacterCameraProximity));

        let sandbox = world.resource::<SandboxSession>();
        assert_eq!(sandbox.launch.deployment, Some(expected_deployment));
        assert_eq!(
            sandbox.launch.content_revision,
            Some(fixture.accepted_fingerprint)
        );
        assert_eq!(
            world.resource::<crate::scenarios::ActiveScenario>().0,
            sandbox.launch.loading_input(),
            "Retry Exact must retain the committed exact encounter"
        );
        assert_eq!(
            sandbox.launch.loading_input().encounter_override.as_ref(),
            Some(world.resource::<Encounter>())
        );
    }

    #[derive(Resource)]
    struct RestoreCreatorContent;

    fn restore_creator_content_once(
        mut commands: Commands,
        overlay: Option<Res<CreatorContentOverlay>>,
    ) {
        restore_shipped_content(&mut commands, overlay.as_deref());
        commands.remove_resource::<RestoreCreatorContent>();
    }

    #[test]
    fn creator_overlay_publishes_one_accepted_revision_and_restores_shipped_content() {
        let fixture = content_fixture().expect("shipped content fixture should resolve");
        let mut app = app_with_content_fixture(&fixture);
        app.add_systems(
            PreUpdate,
            restore_creator_content_once.run_if(resource_exists::<RestoreCreatorContent>),
        );
        app.update();
        let shipped_revision = app
            .world()
            .resource::<AcceptedContentRevision>()
            .fingerprint();
        let overlay = CreatorContentOverlay {
            active: fixture.active,
            shipped: fixture.shipped,
            display_names: BTreeMap::new(),
        };

        for _ in 0..2 {
            app.insert_resource(overlay.clone());
            app.update();
            let world = app.world();
            let accepted = world.resource::<AcceptedContentRevision>();
            assert_ne!(accepted.fingerprint(), shipped_revision);
            assert!(accepted.matches_resolved(
                world.resource::<ContentIndex>(),
                world.resource::<LatticeLibrary>()
            ));
            assert!(accepted.matches_terrain_damage(world.resource::<TerrainDamageTable>()));
            assert!(world
                .resource::<SpellBook>()
                .matches_source(world.resource::<SpellFile>()));

            app.insert_resource(RestoreCreatorContent);
            app.update();
            let world = app.world();
            assert!(!world.contains_resource::<CreatorContentOverlay>());
            assert_eq!(
                world.resource::<AcceptedContentRevision>().fingerprint(),
                shipped_revision
            );
            assert!(world
                .resource::<AcceptedContentRevision>()
                .matches_terrain_damage(world.resource::<TerrainDamageTable>()));
            assert!(world
                .resource::<SpellBook>()
                .matches_source(world.resource::<SpellFile>()));
        }
    }

    #[test]
    fn failed_creator_sandbox_loading_returns_through_sandbox_then_cleans_for_campaign() {
        let fixture = content_fixture().expect("shipped content fixture should resolve");
        let mut app = app_with_content_fixture(&fixture);
        app.init_resource::<SandboxState>()
            .init_resource::<SandboxNotice>()
            .add_systems(OnEnter(Screen::Sandbox), initialize_sandbox)
            .add_systems(OnEnter(Screen::Title), cleanup_sandbox_before_main_menu);
        let rules: CombatSettings = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/combat.ron"
        )))
        .expect("combat.ron parses");
        let library: ScenarioLibrary = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/scenarios.ron"
        )))
        .expect("scenarios.ron parses");
        let scenario = scenario_named(&library, "Ability Lab").expect("fixture scenario exists");
        let definition = map_definition();
        let party = vec![RosterChoice::Template("hedge-mage".to_owned())];
        let enemies = vec![RosterChoice::Template("raider".to_owned())];
        let encounter = sandbox_encounter(&party, &enemies, &definition);
        let launch = SandboxLaunchSnapshot::new(
            SandboxMapSelection::new(definition.id.clone(), definition.fixed_seed),
            definition.scenario.clone(),
            party,
            enemies,
            None,
            rules.clone(),
            scenario,
            encounter.clone(),
        );
        let overlay = CreatorContentOverlay {
            active: fixture.active.clone(),
            shipped: fixture.shipped.clone(),
            display_names: BTreeMap::new(),
        };
        overlay.active.insert_into_world(app.world_mut());
        app.insert_resource(rules.clone());
        app.insert_resource(overlay);
        app.insert_resource(FixedSettingsFreeze::<CombatSettings>::default());
        app.insert_resource(FixedSettingsFreeze::<SpellFile>::default());
        app.insert_resource(FixedSettingsFreeze::<LatticeFile>::default());
        app.insert_resource(GameplaySessionOrigin::Sandbox);
        app.insert_resource(super::super::creator::CreatorSandboxReturn {
            navigation: CreatorNavigation::default(),
        });
        app.insert_resource(SandboxSession {
            launch: launch.clone(),
        });
        app.insert_resource(launch.loading_input());
        app.insert_resource(crate::scenarios::ActiveScenario(launch.loading_input()));
        app.insert_resource(encounter);
        app.insert_resource(hex_core::GameplaySetupFailure::new(
            "Sandbox loading fixture failed.",
        ));

        app.update();
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Sandbox);
        app.update();

        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Sandbox
        );
        assert_eq!(
            app.world().resource::<SandboxNotice>().0.as_deref(),
            Some("Sandbox loading fixture failed.")
        );
        assert!(app
            .world()
            .contains_resource::<super::super::creator::CreatorSandboxReturn>());
        assert!(!app.world().contains_resource::<SandboxSession>());

        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Title);
        app.update();

        let world = app.world();
        assert_eq!(*world.resource::<State<Screen>>().get(), Screen::Title);
        assert!(!world.contains_resource::<CreatorContentOverlay>());
        assert!(!world.contains_resource::<SandboxSession>());
        assert!(!world.contains_resource::<GameplaySessionOrigin>());
        assert!(!world.contains_resource::<super::super::creator::CreatorSandboxReturn>());
        assert!(!world.contains_resource::<FixedSettingsFreeze<CombatSettings>>());
        assert!(!world.contains_resource::<FixedSettingsFreeze<SpellFile>>());
        assert!(!world.contains_resource::<FixedSettingsFreeze<LatticeFile>>());
        assert!(!world.contains_resource::<ScenarioToLoad>());
        assert!(!world.contains_resource::<crate::scenarios::ActiveScenario>());
        assert!(!world.contains_resource::<Encounter>());
        assert!(world
            .resource::<SpellBook>()
            .matches_source(world.resource::<SpellFile>()));
        assert!(world
            .resource::<SpellBook>()
            .matches_source(&fixture.shipped.spell_file));
        assert!(world
            .resource::<SpellBook>()
            .id("Creator Acceptance Test")
            .is_none());
        assert_eq!(world.resource::<CombatSettings>(), &rules);
        assert_eq!(*world.resource::<GameplayPhase>(), GameplayPhase::Active);

        // The next launch may bind Campaign provenance without observing any
        // temporary Sandbox namespace, retry identity, or frozen rules marker.
        app.insert_resource(GameplaySessionOrigin::Campaign(CampaignSlotId::Two));
        assert_eq!(
            app.world().resource::<GameplaySessionOrigin>(),
            &GameplaySessionOrigin::Campaign(CampaignSlotId::Two)
        );
        assert!(!app.world().contains_resource::<SandboxSession>());
        assert!(!app
            .world()
            .contains_resource::<FixedSettingsFreeze<CombatSettings>>());
        assert!(!app
            .world()
            .contains_resource::<FixedSettingsFreeze<SpellFile>>());
        assert!(!app
            .world()
            .contains_resource::<FixedSettingsFreeze<LatticeFile>>());
    }
}
