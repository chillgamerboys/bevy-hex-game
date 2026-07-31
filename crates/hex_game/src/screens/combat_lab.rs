//! Human sandbox composition and scalable deterministic fixture selection.

use std::collections::{BTreeMap, BTreeSet};

use bevy::ecs::system::SystemParam;
use bevy::picking::events::{Click, Pointer};
use bevy::picking::Pickable;
use bevy::prelude::*;
use hex_assets::{
    character_lattice_file, character_runtime_key, combined_spell_file, AcceptedContentRevision,
    CombatLabDeploymentRegion, CombatLabMapCatalog, CombatLabMapDefinition, CombatLabRegionCenter,
    CombatRulesPreset, CombatRulesProfile, CombatSettings, ContentIndex, CreationCellKind,
    CreationPresetCatalog, CustomCharacterId, ElementCatalog, Encounter, EncounterFaction,
    EncounterPlacement, FormationCenter, GameAssets, LatticeFile, LatticeLibrary, PlayerSettings,
    PresetAudience, Roster, RosterEntry, SavedCharacter, Scenario, ScenarioLibrary, SpellBook,
    SpellFile, SpellReference, SubstanceTable,
};
use hex_core::{
    combat_lab_fixture, GameplayPhase, GameplaySetup, GameplaySetupFailure, Headroom, HexCoord,
    HexSpan, HexTile, MapAnchorId, MapAnchors, ResolvedMapSeed, Screen, SubstanceId, TilePos,
    TraversalBlockers, TraversalProfile,
};
use hex_gameplay_model::{
    CombatLabEdit, CombatLabModel, LabTab, RosterChoice as ModelRosterChoice, SandboxRestore,
    SandboxStep, MAX_COMBAT_LAB_ROSTER,
};
use hex_lattice::SpellTable as _;
use hex_units::{Archetype, Body, Faction, Footing, Reach, StandsOn, UnitOccupancy};

use crate::combat_reports::{
    CombatLabReportController, CombatLabReportDeployment, CombatLabReportId, CombatLabReportMap,
    CombatLabReportOrigin, CombatLabReportRosterEntry, CombatLabReportRosters,
    CombatLabReportStore,
};
use crate::creation_store::CreationStore;
use crate::scenarios::{ScenarioContractStatus, ScenarioToLoad};
use crate::storage::StoragePaths;
use hex_ui::{
    CombatLabComparisonView, CombatLabIntent as LabAction, CombatLabReportCardView,
    CombatLabReportField, CombatLabReportsView, CombatLabScreenView, CreatorLibraryView,
    DeploymentIntent as DeploymentAction, DeploymentRosterEntryView, DeploymentView, UiIntent,
    UiSystems,
};

use super::despawn_screen;

pub(crate) use hex_ui::CombatLabRulesVariant as FixtureRulesVariant;

const MAX_ROSTER: usize = MAX_COMBAT_LAB_ROSTER;

type RosterChoice = ModelRosterChoice<CustomCharacterId>;
pub(crate) type CombatLabState = CombatLabModel<CombatRulesProfile, CustomCharacterId>;

#[derive(Resource, Debug, Default)]
pub(crate) struct FrozenSandboxOverlay(Option<CreatorContentOverlay>);

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

/// One coherent raw-and-derived gameplay content graph.
///
/// The accepted-revision publisher validates all five resources as a unit. Keeping
/// them bundled prevents Creator sessions from installing a combined `SpellBook`
/// while leaving the shipped raw files behind, which would make Loading either accept
/// stale content or wait forever.
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
    /// Frozen validated profile reused by Retry.
    pub(crate) profile: CombatRulesProfile,
    /// Authored settings restored whenever the Lab session leaves gameplay.
    pub(crate) shipped_combat: CombatSettings,
    /// Stable map identity frozen before Loading.
    pub(crate) report_map: CombatLabReportMap,
    /// Immutable pre-combat fixture mutation reapplied on every exact Retry.
    pub(crate) initial_state: Option<FixedFixtureInitialState>,
}

/// Frozen report inputs captured at the instant active combat begins.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub(crate) struct CombatLabReportLaunch {
    pub(crate) origin: CombatLabReportOrigin,
    pub(crate) map: CombatLabReportMap,
    pub(crate) content_revision: u64,
    pub(crate) rosters: CombatLabReportRosters,
    pub(crate) deployment: CombatLabReportDeployment,
}

/// Outcome-to-Sandbox handoff used by Tune and fixed-fixture copying.
#[derive(Resource, Debug, Clone)]
pub(crate) struct CombatLabSandboxRequest {
    pub(crate) report: crate::combat_reports::CombatLabReport,
    pub(crate) overlay: Option<CreatorContentOverlay>,
}

/// Whether a frozen Lab profile was admitted at the Loading boundary.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CombatLabRulesStatus {
    /// The effective settings were validated and installed.
    Ready,
    /// The profile failed closed and gameplay must not start.
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FixedFixtureInitialState {
    ChannelAttrition,
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
        preserved: Option<&CombatLabReportDeployment>,
    ) -> Self {
        let player_len = players.len();
        let hostile_len = hostiles.len();
        Self {
            map_definition,
            players,
            hostiles,
            player_placements: preserved.map_or_else(
                || vec![None; player_len],
                |deployment| {
                    deployment
                        .players
                        .iter()
                        .copied()
                        .map(Some)
                        .chain(std::iter::repeat(None))
                        .take(player_len)
                        .collect()
                },
            ),
            hostile_placements: preserved.map_or_else(
                || vec![None; hostile_len],
                |deployment| {
                    deployment
                        .hostiles
                        .iter()
                        .copied()
                        .map(Some)
                        .chain(std::iter::repeat(None))
                        .take(hostile_len)
                        .collect()
                },
            ),
            active_player: true,
            active_index: 0,
            undo: Vec::new(),
            player_surfaces: Vec::new(),
            hostile_surfaces: Vec::new(),
            notice: "PLAYER 1 · Click a BLUE highlighted surface.".to_owned(),
        }
    }

    fn complete(&self) -> bool {
        self.capacity_notice().is_none()
            && placements_complete_exact(&self.player_placements, self.players.len())
            && placements_complete_exact(&self.hostile_placements, self.hostiles.len())
            && placements_belong_to_region(&self.player_placements, &self.player_surfaces)
            && placements_belong_to_region(&self.hostile_placements, &self.hostile_surfaces)
            && !deployment_occupancy(self).has_overlaps()
    }

    fn capacity_notice(&self) -> Option<String> {
        let player_shortfall = self
            .players
            .len()
            .saturating_sub(self.player_surfaces.len());
        let hostile_shortfall = self
            .hostiles
            .len()
            .saturating_sub(self.hostile_surfaces.len());
        if player_shortfall == 0 && hostile_shortfall == 0 {
            return None;
        }

        let mut sides = Vec::new();
        if player_shortfall > 0 {
            sides.push(format!(
                "Player region provides {} of {} required surfaces",
                self.player_surfaces.len(),
                self.players.len()
            ));
        }
        if hostile_shortfall > 0 {
            sides.push(format!(
                "Hostile region provides {} of {} required surfaces",
                self.hostile_surfaces.len(),
                self.hostiles.len()
            ));
        }
        Some(format!(
            "{}. Go Back and reduce that roster or choose another map.",
            sides.join("; ")
        ))
    }
}

fn placements_belong_to_region(placements: &[Option<TilePos>], surfaces: &[TilePos]) -> bool {
    placements
        .iter()
        .all(|placement| placement.is_some_and(|position| surfaces.contains(&position)))
}

#[derive(Component, Debug, Clone, Copy)]
struct DeploymentSurface {
    pos: TilePos,
    player: bool,
}

#[derive(Component)]
struct DeploymentWorldEntity;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct DeploymentPlacementMarker {
    player: bool,
    index: usize,
}

#[derive(Resource)]
struct DeploymentMarkerMaterials {
    player: Handle<StandardMaterial>,
    hostile: Handle<StandardMaterial>,
}

#[derive(Component)]
struct DeploymentHidden;

/// Scenario name behind a stable automated fixture id.
#[cfg(feature = "visual-walk")]
pub(crate) fn fixture_scenario_name(id: &str) -> Option<&'static str> {
    combat_lab_fixture(id).map(|fixture| fixture.scenario)
}

#[cfg(feature = "visual-walk")]
pub(crate) fn fixture_sandbox_map(id: &str) -> Option<&'static str> {
    combat_lab_fixture(id).map(|fixture| fixture.sandbox_map)
}

pub(crate) fn fixture_profile(
    variant: FixtureRulesVariant,
    shipped: &CombatSettings,
) -> CombatRulesProfile {
    match variant {
        FixtureRulesVariant::Shipped => CombatRulesProfile::shipped(shipped),
        FixtureRulesVariant::TacticalTwoStep => CombatRulesProfile::tactical_two_step(shipped),
        FixtureRulesVariant::CustomThreeStep => {
            let mut profile =
                CombatRulesProfile::custom_from(&CombatRulesProfile::shipped(shipped));
            profile.movement_per_turn = 3;
            profile
        }
    }
}

#[cfg(feature = "visual-walk")]
pub(crate) fn walk_fixture_profile(
    name: Option<&str>,
    shipped: &CombatSettings,
) -> Result<CombatRulesProfile, String> {
    let variant = match name.unwrap_or("shipped") {
        "shipped" => FixtureRulesVariant::Shipped,
        "tactical" => FixtureRulesVariant::TacticalTwoStep,
        "custom-three-step" => FixtureRulesVariant::CustomThreeStep,
        other => {
            return Err(format!(
            "unknown fixture profile {other:?}; expected shipped, tactical, or custom-three-step"
        ))
        }
    };
    Ok(fixture_profile(variant, shipped))
}

pub(crate) fn fixed_fixture_encounter(id: &str) -> Option<Encounter> {
    let template = |name: &str| RosterChoice::Template(name.to_owned());
    match id {
        "occupancy-matrix" | "tempo-matrix" => Some(encounter_with_placements(
            "Wave 7 Crossing Matrix",
            &[template("raider"), template("wolf"), template("raider")],
            &[template("raider"), template("wolf"), template("raider")],
            EncounterPlacement::Formation {
                center: FormationCenter::Fixed(hex_assets::CubeCoord { x: 0, y: 2, z: -2 }),
                spread: 2,
            },
            EncounterPlacement::Formation {
                center: FormationCenter::Fixed(hex_assets::CubeCoord { x: 0, y: -2, z: 2 }),
                spread: 2,
            },
        )),
        "channel-attrition" => Some(encounter_with_placements(
            "Wave 7 Channel Attrition",
            &[template("hedge-mage"), template("raider"), template("wolf")],
            &[template("hedge-mage"), template("raider"), template("wolf")],
            EncounterPlacement::Formation {
                center: FormationCenter::Fixed(hex_assets::CubeCoord { x: -2, y: 0, z: 2 }),
                spread: 2,
            },
            EncounterPlacement::Formation {
                center: FormationCenter::Fixed(hex_assets::CubeCoord { x: 2, y: 0, z: -2 }),
                spread: 2,
            },
        )),
        _ => None,
    }
}

#[cfg(feature = "visual-walk")]
pub(crate) fn walk_fixture_encounter(id: &str) -> Option<Encounter> {
    fixed_fixture_encounter(id)
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
            vec![RosterChoice::Packaged(CustomCharacterId(1001))],
            vec![RosterChoice::Packaged(CustomCharacterId(1002))],
        )),
        "creator-roster-matrix" => Some((
            vec![
                RosterChoice::Packaged(CustomCharacterId(1001)),
                RosterChoice::Packaged(CustomCharacterId(1003)),
            ],
            vec![
                RosterChoice::Packaged(CustomCharacterId(1002)),
                RosterChoice::Packaged(CustomCharacterId(1001)),
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
    let encounter = encounter_with_placements(
        "Flat Arena",
        &players,
        &hostiles,
        EncounterPlacement::Formation {
            center: FormationCenter::Fixed(hex_assets::CubeCoord { x: -2, y: 0, z: 2 }),
            spread: 3,
        },
        EncounterPlacement::Formation {
            center: FormationCenter::Fixed(hex_assets::CubeCoord { x: 2, y: 0, z: -2 }),
            spread: 3,
        },
    );
    Ok(Some((overlay, encounter)))
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<CombatLabState>()
        .init_resource::<FrozenSandboxOverlay>()
        .add_systems(
            OnEnter(Screen::Gameplay),
            (
                apply_creator_display_names.in_set(GameplaySetup::Restore),
                apply_fixed_fixture_initial_state.in_set(GameplaySetup::Restore),
                enter_deployment.in_set(GameplaySetup::Finalize),
                capture_fixed_fixture_report_launch.in_set(GameplaySetup::Finalize),
            ),
        )
        .add_systems(
            Update,
            (
                handle_deployment_actions.after(UiSystems::EmitIntents),
                publish_deployment_view,
            )
                .chain()
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_observer(on_deployment_surface_clicked)
        .add_systems(OnExit(Screen::Gameplay), clear_deployment_world)
        .add_systems(OnExit(Screen::Gameplay), restore_shipped_combat_rules)
        .add_systems(
            OnEnter(Screen::CombatLab),
            (initialize_lab, publish_combat_lab_view).chain(),
        )
        .add_systems(
            Update,
            (
                handle_lab_actions.after(UiSystems::EmitIntents),
                publish_combat_lab_view,
            )
                .chain()
                .run_if(in_state(Screen::CombatLab)),
        )
        .add_systems(OnExit(Screen::CombatLab), despawn_screen(Screen::CombatLab));
}

pub(crate) fn initialize_lab(
    mut commands: Commands,
    creator_request: Option<Res<CreatorTestRequest>>,
    sandbox_request: Option<Res<CombatLabSandboxRequest>>,
    mut state: ResMut<CombatLabState>,
    mut frozen_overlay: ResMut<FrozenSandboxOverlay>,
    overlay: Option<Res<CreatorContentOverlay>>,
) {
    let sandbox_request = sandbox_request.as_deref().cloned();
    restore_shipped_content(&mut commands, overlay.as_deref());
    commands.remove_resource::<CombatLabSession>();
    commands.remove_resource::<DeploymentSession>();
    commands.remove_resource::<CombatLabReportLaunch>();
    commands.insert_resource(GameplayPhase::Active);
    if let Some(request) = sandbox_request {
        let packaged = matches!(
            request.report.origin,
            CombatLabReportOrigin::FixedFixture { ref stable_id }
                if stable_id.starts_with("creator-")
        );
        let players = request
            .report
            .rosters
            .players
            .iter()
            .map(|entry| report_roster_choice(entry, packaged))
            .collect();
        let hostiles = request
            .report
            .rosters
            .hostiles
            .iter()
            .map(|entry| report_roster_choice(entry, packaged))
            .collect();
        state.restore_sandbox(SandboxRestore {
            map: request.report.map.catalog_id.clone(),
            players,
            hostiles,
            rules: request.report.profile.clone(),
            deployment: request.report.deployment.clone(),
        });
        frozen_overlay.0 = request.overlay;
        commands.remove_resource::<CombatLabSandboxRequest>();
        return;
    } else if let Some(request) = creator_request {
        state.tab = LabTab::Sandbox;
        state.sandbox_step = SandboxStep::Rosters;
        state.players = vec![RosterChoice::Custom(request.character)];
        state.notice = "Creator character prefilled; choose the rest of the test.".to_owned();
        state.creator_origin = true;
        frozen_overlay.0 = None;
        commands.remove_resource::<CreatorTestRequest>();
    } else {
        state.creator_origin = false;
        frozen_overlay.0 = None;
    }
    state.bump();
}

fn report_roster_choice(entry: &CombatLabReportRosterEntry, packaged: bool) -> RosterChoice {
    entry
        .archetype
        .strip_prefix("custom-character-")
        .and_then(|id| id.parse::<u64>().ok())
        .map_or_else(
            || RosterChoice::Template(entry.archetype.clone()),
            |id| {
                if packaged {
                    RosterChoice::Packaged(CustomCharacterId(id))
                } else {
                    RosterChoice::Custom(CustomCharacterId(id))
                }
            },
        )
}

#[cfg(feature = "test-support")]
pub(crate) fn roster_choice_key(choice: &RosterChoice) -> String {
    match choice {
        RosterChoice::Template(key) => key.clone(),
        RosterChoice::Custom(id) | RosterChoice::Packaged(id) => {
            format!("custom-character-{}", id.0)
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the composition root freezes every Lab catalog into one immutable presentation view"
)]
fn publish_combat_lab_view(
    state: Res<CombatLabState>,
    store: Res<CreationStore>,
    elements: Option<Res<ElementCatalog>>,
    spells: Option<Res<SpellBook>>,
    presets: Option<Res<CreationPresetCatalog>>,
    maps: Option<Res<CombatLabMapCatalog>>,
    combat: Option<Res<CombatSettings>>,
    reports: Res<CombatLabReportStore>,
    mut view: ResMut<CombatLabScreenView>,
    mut last_revision: Local<u64>,
) {
    let catalog_changed = elements.as_ref().is_some_and(|value| value.is_changed())
        || spells.as_ref().is_some_and(|value| value.is_changed())
        || presets.as_ref().is_some_and(|value| value.is_changed())
        || maps.as_ref().is_some_and(|value| value.is_changed())
        || combat.as_ref().is_some_and(|value| value.is_changed());
    if *last_revision == state.revision && !store.is_changed() && !catalog_changed {
        return;
    }

    let mut map_ready_choices = Vec::new();
    if let (Some(elements), Some(spells)) = (elements.as_deref(), spells.as_deref()) {
        map_ready_choices.extend(
            store
                .file
                .characters
                .iter()
                .filter(|character| {
                    character_is_map_ready(character, &store.file, elements, spells)
                })
                .map(|character| RosterChoice::Custom(character.id)),
        );
        if let Some(presets) = presets.as_deref() {
            let library = presets.library_for(PresetAudience::AutomationFixture);
            map_ready_choices.extend(
                library
                    .characters
                    .iter()
                    .filter(|character| {
                        character_is_map_ready(character, &library, elements, spells)
                    })
                    .map(|character| RosterChoice::Packaged(character.id)),
            );
        }
    }

    *view = CombatLabScreenView {
        active: true,
        tab: state.tab,
        sandbox_step: state.sandbox_step,
        map: state.map.clone(),
        players: state.players.clone(),
        hostiles: state.hostiles.clone(),
        fixture_filter: state.fixture_filter.clone(),
        notice: state.notice.clone(),
        rules: state.rules.clone(),
        pending_report_delete: state.pending_report_delete,
        library: CreatorLibraryView {
            file: store.file.clone(),
            error: store.error.clone(),
        },
        elements: elements.as_deref().cloned(),
        spells: spells.as_deref().cloned(),
        presets: presets.as_deref().cloned(),
        maps: maps.as_deref().cloned(),
        combat: combat.as_deref().cloned(),
        map_ready_choices,
        reports: project_combat_lab_reports(&state, &reports),
    };
    *last_revision = state.revision;
}

fn project_combat_lab_reports(
    state: &CombatLabState,
    store: &CombatLabReportStore,
) -> CombatLabReportsView {
    let (left_id, right_id) = selected_compare_ids(state, store);
    let reports = store
        .history
        .reports
        .iter()
        .map(|saved| CombatLabReportCardView {
            id: saved.id,
            heading: format!("REPORT {} · {:?}", saved.id.0, saved.report.termination),
            label: saved.label.clone(),
            notes: saved.notes.clone(),
            metadata: format!(
                "{:?} · {} · seed {} · P{} / H{} · {:016X}",
                saved.report.profile.preset,
                saved.report.map.scenario,
                saved
                    .report
                    .map
                    .resolved_seed
                    .map_or_else(|| "authored".to_owned(), |seed| seed.to_string()),
                saved.report.rosters.players.len(),
                saved.report.rosters.hostiles.len(),
                saved.report.summary_fingerprint,
            ),
            summary: format!(
                "Rounds {} · commands {}/{} · move {} · Channel {} · applied disables {}",
                saved.report.summary.rounds,
                saved.report.summary.successful_commands,
                saved.report.summary.refused_commands,
                saved.report.summary.movement_distance,
                saved.report.summary.channels,
                saved.report.summary.applied_disables,
            ),
            left_selected: left_id == Some(saved.id),
            right_selected: right_id == Some(saved.id),
            pending_delete: state.pending_report_delete == Some(saved.id),
        })
        .collect();
    let comparison = selected_compare_reports(state, store).map(|(left, right)| {
        CombatLabComparisonView {
            heading: format!("compare reports {} vs {}", left.id.0, right.id.0),
            frozen: format!(
                "FROZEN LEFT · {}\nFROZEN RIGHT · {}",
                frozen_report_header(&left.report),
                frozen_report_header(&right.report),
            ),
            deltas: format!(
                "Stops: {:?} → {:?}\nRounds {:+} · turns {:+} · successful commands {:+} · refused commands {:+} · movement {:+} · Channel {:+} · applied disables {:+} · no-progress current/max {:+}/{:+}\n{}",
                left.report.termination,
                right.report.termination,
                signed_delta(right.report.summary.rounds, left.report.summary.rounds),
                signed_delta(right.report.summary.turns, left.report.summary.turns),
                signed_delta(
                    right.report.summary.successful_commands,
                    left.report.summary.successful_commands,
                ),
                signed_delta(
                    right.report.summary.refused_commands,
                    left.report.summary.refused_commands,
                ),
                signed_delta(
                    right.report.summary.movement_distance,
                    left.report.summary.movement_distance,
                ),
                signed_delta(right.report.summary.channels, left.report.summary.channels),
                signed_delta(
                    right.report.summary.applied_disables,
                    left.report.summary.applied_disables,
                ),
                signed_delta(
                    right.report.summary.no_progress_current,
                    left.report.summary.no_progress_current,
                ),
                signed_delta(
                    right.report.summary.no_progress_max,
                    left.report.summary.no_progress_max,
                ),
                detailed_report_deltas(&left.report, &right.report),
            ),
        }
    });
    CombatLabReportsView {
        error: store.error.clone(),
        reports,
        comparison,
    }
}

fn frozen_report_header(report: &crate::combat_reports::CombatLabReport) -> String {
    let roster = |entries: &[CombatLabReportRosterEntry]| {
        entries
            .iter()
            .map(|entry| entry.display_name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "{:?} [move {} · strike {} · initiative {} {:?} · {:?}/{:?}/{:?} · engage {} · margin {} · levels {} · reveal {}] · {} / seed {:?} / content {:016X} · P [{}] @ {:?} · H [{}] @ {:?}",
        report.profile.preset,
        report.profile.movement_per_turn,
        report.profile.strike_disables,
        report.profile.default_initiative,
        report.profile.initiative_policy,
        report.profile.action_economy,
        report.profile.channelling_trickle,
        report.profile.rout_policy,
        report.profile.engage_range,
        report.profile.disengage_margin,
        report.profile.levels_per_bonus_range,
        report.profile.reveal_duration,
        report.map.scenario,
        report.map.resolved_seed,
        report.content_revision,
        roster(&report.rosters.players),
        report.deployment.players,
        roster(&report.rosters.hostiles),
        report.deployment.hostiles,
    )
}

fn detailed_report_deltas(
    left: &crate::combat_reports::CombatLabReport,
    right: &crate::combat_reports::CombatLabReport,
) -> String {
    let units = left
        .summary
        .units
        .keys()
        .chain(right.summary.units.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    let per_unit = units
        .into_iter()
        .map(|unit| {
            let left_unit = left.summary.units.get(&unit);
            let right_unit = right.summary.units.get(&unit);
            format!(
                "{unit:?}: turns {:+}, commands {:+}, move {:+}, disables {:+}, no-progress max {:+}",
                signed_delta(unit_value(right_unit, |unit| unit.turns), unit_value(left_unit, |unit| unit.turns)),
                signed_delta(
                    unit_value(right_unit, |unit| unit.successful_commands),
                    unit_value(left_unit, |unit| unit.successful_commands),
                ),
                signed_delta(
                    unit_value(right_unit, |unit| unit.movement_distance),
                    unit_value(left_unit, |unit| unit.movement_distance),
                ),
                signed_delta(
                    unit_value(right_unit, |unit| unit.applied_disables),
                    unit_value(left_unit, |unit| unit.applied_disables),
                ),
                signed_delta(
                    unit_value(right_unit, |unit| unit.no_progress_max),
                    unit_value(left_unit, |unit| unit.no_progress_max),
                ),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let spells = left
        .summary
        .casts_by_spell
        .keys()
        .chain(right.summary.casts_by_spell.keys())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|spell| {
            format!(
                "{spell} {:+}",
                signed_delta(
                    right
                        .summary
                        .casts_by_spell
                        .get(&spell)
                        .copied()
                        .unwrap_or_default(),
                    left.summary
                        .casts_by_spell
                        .get(&spell)
                        .copied()
                        .unwrap_or_default(),
                )
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let effects = left
        .summary
        .delivered_effects
        .keys()
        .chain(right.summary.delivered_effects.keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|effect| {
            format!(
                "{effect:?} {:+}",
                signed_delta(
                    right
                        .summary
                        .delivered_effects
                        .get(&effect)
                        .copied()
                        .unwrap_or_default(),
                    left.summary
                        .delivered_effects
                        .get(&effect)
                        .copied()
                        .unwrap_or_default(),
                )
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("Per unit\n{per_unit}\nSpells: {spells}\nEffects: {effects}")
}

fn unit_value(
    summary: Option<&hex_combat::UnitCombatSummary>,
    field: fn(&hex_combat::UnitCombatSummary) -> u32,
) -> u32 {
    summary.map_or(0, field)
}

fn selected_compare_ids(
    state: &CombatLabState,
    store: &CombatLabReportStore,
) -> (Option<CombatLabReportId>, Option<CombatLabReportId>) {
    let first = store.history.reports.first().map(|saved| saved.id);
    let last = store.history.reports.last().map(|saved| saved.id);
    (
        state
            .compare_left
            .filter(|id| store.history.reports.iter().any(|saved| saved.id == *id))
            .or(first),
        state
            .compare_right
            .filter(|id| store.history.reports.iter().any(|saved| saved.id == *id))
            .or(last),
    )
}

fn selected_compare_reports<'a>(
    state: &CombatLabState,
    store: &'a CombatLabReportStore,
) -> Option<(
    &'a crate::combat_reports::SavedCombatLabReport,
    &'a crate::combat_reports::SavedCombatLabReport,
)> {
    let (left, right) = selected_compare_ids(state, store);
    let left = left?;
    let right = right?;
    if left == right {
        return None;
    }
    Some((
        store
            .history
            .reports
            .iter()
            .find(|saved| saved.id == left)?,
        store
            .history
            .reports
            .iter()
            .find(|saved| saved.id == right)?,
    ))
}

fn signed_delta(right: u32, left: u32) -> i64 {
    i64::from(right) - i64::from(left)
}

fn handle_lab_actions(
    mut intents: MessageReader<UiIntent>,
    mut state: ResMut<CombatLabState>,
    store: Res<CreationStore>,
    scenarios: Option<Res<ScenarioLibrary>>,
    shipped_spell_file: Option<Res<SpellFile>>,
    base_lattice_file: Option<Res<LatticeFile>>,
    elements: Option<Res<ElementCatalog>>,
    substances: Option<Res<SubstanceTable>>,
    presets: Option<Res<CreationPresetCatalog>>,
    map_catalog: Option<Res<CombatLabMapCatalog>>,
    combat: Option<Res<CombatSettings>>,
    mut reports: ResMut<CombatLabReportStore>,
    frozen_overlay: Res<FrozenSandboxOverlay>,
    paths: Res<StoragePaths>,
    mut commands: Commands,
    mut next: ResMut<NextState<Screen>>,
) {
    for intent in intents.read() {
        let UiIntent::CombatLab(action) = intent else {
            continue;
        };
        match action {
            LabAction::Tab(tab) => {
                state.apply(CombatLabEdit::Tab(*tab));
                continue;
            }
            LabAction::Back => next.set(Screen::Title),
            LabAction::ShowSandboxStep(step) => {
                state.apply(CombatLabEdit::SandboxStep(*step));
                continue;
            }
            LabAction::SelectMap(map) => {
                state.apply(CombatLabEdit::SelectMap(map.clone()));
                continue;
            }
            LabAction::AddPlayerTemplate(name) => {
                state.apply(CombatLabEdit::AddPlayer(RosterChoice::Template(
                    name.clone(),
                )));
                continue;
            }
            LabAction::AddHostileTemplate(name) => {
                state.apply(CombatLabEdit::AddHostile(RosterChoice::Template(
                    name.clone(),
                )));
                continue;
            }
            LabAction::AddPlayerCustom(id) => {
                state.apply(CombatLabEdit::AddPlayer(RosterChoice::Custom(*id)));
                continue;
            }
            LabAction::AddHostileCustom(id) => {
                state.apply(CombatLabEdit::AddHostile(RosterChoice::Custom(*id)));
                continue;
            }
            LabAction::RemovePlayer(index) => {
                state.apply(CombatLabEdit::RemovePlayer(*index));
                continue;
            }
            LabAction::RemoveHostile(index) => {
                state.apply(CombatLabEdit::RemoveHostile(*index));
                continue;
            }
            LabAction::MovePlayer(index, delta) => {
                state.apply(CombatLabEdit::MovePlayer(*index, *delta));
                continue;
            }
            LabAction::MoveHostile(index, delta) => {
                state.apply(CombatLabEdit::MoveHostile(*index, *delta));
                continue;
            }
            LabAction::EditCustom(character) => {
                commands.insert_resource(CreatorEditRequest {
                    character: *character,
                });
                next.set(Screen::CharacterCreator);
            }
            LabAction::SelectRulesPreset(preset) => {
                let Some(shipped) = combat.as_deref() else {
                    state.notice = "Shipped combat rules are still loading.".to_owned();
                    state.bump();
                    continue;
                };
                let current = state
                    .rules
                    .clone()
                    .unwrap_or_else(|| CombatRulesProfile::shipped(shipped));
                state.rules = Some(match preset {
                    CombatRulesPreset::Shipped => CombatRulesProfile::shipped(shipped),
                    CombatRulesPreset::TacticalTwoStep => {
                        CombatRulesProfile::tactical_two_step(shipped)
                    }
                    CombatRulesPreset::Custom => CombatRulesProfile::custom_from(&current),
                });
                state.notice.clear();
            }
            LabAction::AdjustRule(field, delta) => {
                let Some(shipped) = combat.as_deref() else {
                    state.notice = "Shipped combat rules are still loading.".to_owned();
                    state.bump();
                    continue;
                };
                let mut profile = state
                    .rules
                    .clone()
                    .unwrap_or_else(|| CombatRulesProfile::shipped(shipped));
                let amount = u32::from(delta.unsigned_abs());
                let next = if *delta < 0 {
                    profile.value(*field).checked_sub(amount)
                } else {
                    profile.value(*field).checked_add(amount)
                };
                match next.and_then(|value| profile.set_custom(*field, value).ok().map(|_| value)) {
                    Some(_) => {
                        state.rules = Some(profile);
                        state.notice.clear();
                    }
                    None => {
                        let bounds = field.bounds();
                        state.notice = format!(
                            "{} must remain in {}..={}.",
                            field.label(),
                            bounds.min,
                            bounds.max
                        );
                    }
                }
            }
            LabAction::ResetRules => {
                let Some(shipped) = combat.as_deref() else {
                    state.notice = "Shipped combat rules are still loading.".to_owned();
                    state.bump();
                    continue;
                };
                state.rules = Some(CombatRulesProfile::shipped(shipped));
                state.notice = "Rules reset to the shipped profile.".to_owned();
            }
            LabAction::PrepareDeployment => {
                let Some(shipped_combat) = combat.as_deref() else {
                    state.notice = "Shipped combat rules are still loading.".to_owned();
                    state.bump();
                    continue;
                };
                let Some(map_definition) = map_catalog
                    .as_deref()
                    .and_then(|catalog| catalog.get(&state.map))
                else {
                    state.notice =
                        format!("Packaged map definition {:?} is unavailable.", state.map);
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
                let overlay = if let Some(overlay) = frozen_overlay.0.clone() {
                    overlay
                } else {
                    match build_creator_overlay(
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
                    }
                };
                let encounter = sandbox_encounter(&state.players, &state.hostiles, map_definition);
                let resolved_seed = map_definition
                    .fixed_seed
                    .or(scenario.generation_seed)
                    .map(ResolvedMapSeed);
                let profile = state
                    .rules
                    .clone()
                    .unwrap_or_else(|| CombatRulesProfile::shipped(shipped_combat));
                if let Err(error) = profile.validate(shipped_combat) {
                    state.notice = format!("Rules profile refused: {error}");
                    state.bump();
                    continue;
                }
                state.rules = Some(profile.clone());
                commands.insert_resource(overlay);
                commands.insert_resource(DeploymentSession::new(
                    map_definition.clone(),
                    state.players.clone(),
                    state.hostiles.clone(),
                    state.preserved_deployment.as_ref(),
                ));
                commands.insert_resource(GameplayPhase::Preparing);
                commands.insert_resource(CombatLabSession {
                    kind: CombatLabSessionKind::Sandbox,
                    return_to: if state.creator_origin {
                        Screen::CharacterCreator
                    } else {
                        Screen::CombatLab
                    },
                    profile,
                    shipped_combat: shipped_combat.clone(),
                    report_map: CombatLabReportMap {
                        catalog_id: map_definition.id.clone(),
                        scenario: map_definition.scenario.clone(),
                        resolved_seed: resolved_seed.map(|seed| seed.0),
                    },
                    initial_state: None,
                });
                commands.insert_resource(ScenarioToLoad {
                    scenario,
                    resolved_seed,
                    encounter_override: Some(encounter),
                });
                next.set(Screen::Loading);
            }
            LabAction::StartFixture(id, variant) => {
                let Some(shipped_combat) = combat.as_deref() else {
                    state.notice = "Shipped combat rules are still loading.".to_owned();
                    state.bump();
                    continue;
                };
                let Some(fixture) = combat_lab_fixture(id) else {
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
                let encounter_override = payload
                    .map(|(overlay, encounter)| {
                        commands.insert_resource(overlay);
                        encounter
                    })
                    .or_else(|| fixed_fixture_encounter(id));
                commands.insert_resource(CombatLabSession {
                    kind: CombatLabSessionKind::FixedFixture(id.clone()),
                    return_to: Screen::CombatLab,
                    profile: fixture_profile(*variant, shipped_combat),
                    shipped_combat: shipped_combat.clone(),
                    report_map: CombatLabReportMap {
                        catalog_id: fixture.sandbox_map.to_owned(),
                        scenario: fixture.scenario.to_owned(),
                        resolved_seed: resolved_seed.map(|seed| seed.0),
                    },
                    initial_state: (id == "channel-attrition")
                        .then_some(FixedFixtureInitialState::ChannelAttrition),
                });
                commands.insert_resource(ScenarioToLoad {
                    scenario,
                    resolved_seed,
                    encounter_override,
                });
                next.set(Screen::Loading);
            }
            LabAction::SelectCompareLeft(id) => {
                state.apply(CombatLabEdit::SelectCompareLeft(*id));
                continue;
            }
            LabAction::SelectCompareRight(id) => {
                state.apply(CombatLabEdit::SelectCompareRight(*id));
                continue;
            }
            LabAction::RequestReportDelete(id) => {
                state.apply(CombatLabEdit::RequestReportDelete(*id));
                continue;
            }
            LabAction::ConfirmReportDelete(id) => {
                let Some(shipped) = combat.as_deref() else {
                    state.notice = "Shipped combat rules are still loading.".to_owned();
                    state.bump();
                    continue;
                };
                match reports.delete(*id, shipped, &paths) {
                    Ok(()) => state.confirm_report_deleted(*id),
                    Err(error) => state.notice = error,
                }
            }
            LabAction::CancelReportDelete => {
                state.apply(CombatLabEdit::CancelReportDelete);
                continue;
            }
            LabAction::SetFixtureFilter(value) => {
                if state.fixture_filter != *value {
                    state.fixture_filter.clone_from(value);
                }
                continue;
            }
            LabAction::SetReportField(field, value) => {
                let id = match field {
                    CombatLabReportField::Label(id) | CombatLabReportField::Notes(id) => *id,
                };
                let Some(saved) = reports.history.reports.iter().find(|saved| saved.id == id)
                else {
                    continue;
                };
                let (label, notes) = match field {
                    CombatLabReportField::Label(_) => (value.clone(), saved.notes.clone()),
                    CombatLabReportField::Notes(_) => (saved.label.clone(), value.clone()),
                };
                if label == saved.label && notes == saved.notes {
                    continue;
                }
                let Some(shipped) = combat.as_deref() else {
                    state.notice = "Shipped combat rules are still loading.".to_owned();
                    state.bump();
                    continue;
                };
                if let Err(error) = reports.annotate(id, label, notes, shipped, &paths) {
                    reports.error = Some(error);
                }
                continue;
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
            RosterChoice::Custom(id) | RosterChoice::Packaged(id) => {
                library.characters.iter().find(|saved| saved.id == *id)
            }
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
            RosterChoice::Custom(id) | RosterChoice::Packaged(id) => {
                library.characters.iter().find(|saved| saved.id == *id)
            }
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
        display_names: players
            .iter()
            .chain(hostiles)
            .filter_map(|choice| match choice {
                RosterChoice::Custom(id) | RosterChoice::Packaged(id) => library
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
    players: &[RosterChoice],
    hostiles: &[RosterChoice],
    definition: &CombatLabMapDefinition,
) -> Encounter {
    encounter_with_placements(
        &definition.display_name,
        players,
        hostiles,
        deployment_region_placement(&definition.player_region),
        deployment_region_placement(&definition.hostile_region),
    )
}

fn encounter_with_placements(
    map_name: &str,
    players: &[RosterChoice],
    hostiles: &[RosterChoice],
    player_placement: EncounterPlacement,
    hostile_placement: EncounterPlacement,
) -> Encounter {
    Encounter {
        name: format!("Creator Sandbox · {map_name}"),
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
            RosterChoice::Custom(id) | RosterChoice::Packaged(id) => character_runtime_key(*id),
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
        RosterChoice::Packaged(id) => format!("fixture character #{}", id.0),
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

fn placements_complete_exact(placements: &[Option<TilePos>], roster_len: usize) -> bool {
    placements.len() == roster_len
        && !placements.is_empty()
        && placements.iter().all(Option::is_some)
}

fn deployment_snapshot(session: &DeploymentSession) -> Option<CombatLabReportDeployment> {
    session.complete().then(|| CombatLabReportDeployment {
        players: session
            .player_placements
            .iter()
            .copied()
            .flatten()
            .collect(),
        hostiles: session
            .hostile_placements
            .iter()
            .copied()
            .flatten()
            .collect(),
    })
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
    let entries = |player: bool, roster: &[RosterChoice], placements: &[Option<TilePos>]| {
        roster
            .iter()
            .enumerate()
            .map(|(index, choice)| DeploymentRosterEntryView {
                index,
                name: choice_name(choice, &store),
                selected: session.active_player == player && session.active_index == index,
                position: placements.get(index).copied().flatten(),
            })
            .collect()
    };
    *view = DeploymentView {
        active: true,
        map_name: session.map_definition.display_name.clone(),
        notice: session.notice.clone(),
        players: entries(true, &session.players, &session.player_placements),
        hostiles: entries(false, &session.hostiles, &session.hostile_placements),
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
        return;
    };
    let Some(hostile_center) = deployment_center(
        &session.map_definition.hostile_region,
        &footing,
        anchors.as_deref(),
    ) else {
        session.notice = "Hostile deployment region could not resolve on this terrain.".to_owned();
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
    let mut occupied = std::collections::BTreeSet::new();
    let mut dropped = 0;
    for placements in [
        session.player_placements.as_mut_slice(),
        session.hostile_placements.as_mut_slice(),
    ] {
        for placement in placements {
            if placement.is_some_and(|position| {
                footing.at(position).is_none() || !occupied.insert(position)
            }) {
                *placement = None;
                dropped += 1;
            }
        }
    }
    if dropped == 0 && session.complete() {
        session.notice =
            "Exact frozen deployment retained · select any row to reposition.".to_owned();
    } else if dropped > 0 {
        session.notice = format!(
            "{dropped} frozen placement{} no longer valid; place the highlighted roster rows.",
            if dropped == 1 { " is" } else { "s are" }
        );
        advance_deployment_cursor(session);
    }
    if let Some(notice) = session.capacity_notice() {
        // Capacity is the hard start gate, so it outranks retained-placement cleanup.
        session.notice = notice;
    }

    let player_material = materials.add(deployment_material(Color::srgba(0.20, 0.68, 0.98, 0.58)));
    let hostile_material = materials.add(deployment_material(Color::srgba(0.94, 0.30, 0.24, 0.58)));
    commands.insert_resource(DeploymentMarkerMaterials {
        player: materials.add(deployment_marker_material(Color::srgb(0.10, 0.68, 1.0))),
        hostile: materials.add(deployment_marker_material(Color::srgb(1.0, 0.20, 0.13))),
    });
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

fn deployment_marker_material(color: Color) -> StandardMaterial {
    StandardMaterial {
        base_color: color,
        unlit: true,
        ..default()
    }
}

fn on_deployment_surface_clicked(
    click: On<Pointer<Click>>,
    surfaces: Query<&DeploymentSurface>,
    mut session: Option<ResMut<DeploymentSession>>,
    mut commands: Commands,
    markers: DeploymentMarkerRuntime,
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
            "{} {} · Click a {} highlighted surface.",
            if session.active_player {
                "PLAYER"
            } else {
                "HOSTILE"
            },
            session.active_index + 1,
            if session.active_player { "BLUE" } else { "RED" }
        );
        return;
    }
    let active = deployment_unit_id(session.active_player, session.active_index);
    let occupied = deployment_occupancy(session).is_occupied(surface.pos, Some(active));
    if occupied {
        session.notice = "That exact surface is already occupied.".to_owned();
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
    rebuild_deployment_markers(&mut commands, &markers, session);
}

fn deployment_unit_id(player: bool, index: usize) -> hex_core::UnitId {
    let side = if player {
        0
    } else {
        u64::try_from(MAX_ROSTER).unwrap_or(u64::MAX)
    };
    hex_core::UnitId(side.saturating_add(u64::try_from(index).unwrap_or(u64::MAX)))
}

fn deployment_occupancy(session: &DeploymentSession) -> UnitOccupancy {
    UnitOccupancy::from_positions(
        session
            .player_placements
            .iter()
            .enumerate()
            .filter_map(|(index, placement)| {
                placement.map(|position| (deployment_unit_id(true, index), position))
            })
            .chain(session.hostile_placements.iter().enumerate().filter_map(
                |(index, placement)| {
                    placement.map(|position| (deployment_unit_id(false, index), position))
                },
            )),
    )
}

fn advance_deployment_cursor(session: &mut DeploymentSession) {
    if let Some(index) = session.player_placements.iter().position(Option::is_none) {
        session.active_player = true;
        session.active_index = index;
        session.notice = format!("PLAYER {} · Click a BLUE highlighted surface.", index + 1);
    } else if let Some(index) = session.hostile_placements.iter().position(Option::is_none) {
        session.active_player = false;
        session.active_index = index;
        session.notice = format!("HOSTILE {} · Click a RED highlighted surface.", index + 1);
    } else {
        session.notice =
            "DEPLOYMENT COMPLETE · Solid blue/red tokens mark the resolved surfaces. Start Combat or reposition with Undo."
                .to_owned();
    }
}

#[derive(SystemParam)]
struct DeploymentMarkerRuntime<'w, 's> {
    tiles: DeploymentTileQuery<'w, 's>,
    table: Option<Res<'w, SubstanceTable>>,
    blockers: Option<Res<'w, TraversalBlockers>>,
    game_assets: Res<'w, GameAssets>,
    player_settings: Res<'w, PlayerSettings>,
    materials: Option<Res<'w, DeploymentMarkerMaterials>>,
    entities: Query<'w, 's, Entity, With<DeploymentPlacementMarker>>,
}

fn rebuild_deployment_markers(
    commands: &mut Commands,
    runtime: &DeploymentMarkerRuntime,
    session: &DeploymentSession,
) {
    for entity in &runtime.entities {
        commands.entity(entity).despawn();
    }
    let (Some(table), Some(materials)) = (runtime.table.as_deref(), runtime.materials.as_deref())
    else {
        return;
    };
    let footing = deployment_footing(&runtime.tiles, table, runtime.blockers.as_deref());
    let scale = runtime.player_settings.scale * 1.08;
    let child_transform = Transform {
        translation: Vec3::new(-scale, -scale, -10.0 * scale),
        scale: Vec3::splat(scale),
        ..default()
    };
    for (player, index, pos) in resolved_deployment_markers(session) {
        let Some(standing) = footing.at(pos) else {
            continue;
        };
        let material = if player {
            materials.player.clone()
        } else {
            materials.hostile.clone()
        };
        let [mesh_a, mesh_b] = runtime.game_assets.player_pieces.clone();
        commands
            .spawn((
                Name::new(format!(
                    "{} Placement {}",
                    if player { "Player" } else { "Hostile" },
                    index + 1
                )),
                DeploymentWorldEntity,
                DeploymentPlacementMarker { player, index },
                Transform::from_translation(standing.world_position()),
                Visibility::default(),
                Pickable::IGNORE,
            ))
            .with_children(|marker| {
                marker.spawn((
                    Mesh3d(runtime.game_assets.hex_tile.clone()),
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
) -> impl Iterator<Item = (bool, usize, TilePos)> + '_ {
    session
        .player_placements
        .iter()
        .enumerate()
        .filter_map(|(index, placement)| placement.map(|pos| (true, index, pos)))
        .chain(
            session
                .hostile_placements
                .iter()
                .enumerate()
                .filter_map(|(index, placement)| placement.map(|pos| (false, index, pos))),
        )
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
            &'static Name,
            &'static mut StandsOn,
            &'static mut Transform,
        ),
    >,
    hidden_presentation: Query<'w, 's, (Entity, &'static mut Visibility), With<DeploymentHidden>>,
    world_entities: Query<'w, 's, Entity, With<DeploymentWorldEntity>>,
    encounter: Option<ResMut<'w, Encounter>>,
    active: Option<ResMut<'w, crate::scenarios::ActiveScenario>>,
    lab: Option<Res<'w, CombatLabSession>>,
    accepted: Option<Res<'w, AcceptedContentRevision>>,
}

fn handle_deployment_actions(
    mut intents: MessageReader<UiIntent>,
    mut session: Option<ResMut<DeploymentSession>>,
    mut lab_state: ResMut<CombatLabState>,
    mut phase: ResMut<GameplayPhase>,
    mut runtime: DeploymentRuntime,
    mut commands: Commands,
    mut next: ResMut<NextState<Screen>>,
) {
    let Some(session) = session.as_deref_mut() else {
        return;
    };
    for intent in intents.read() {
        let UiIntent::Deployment(action) = intent else {
            continue;
        };
        match action {
            DeploymentAction::Select { player, index } => {
                let valid = if *player {
                    *index < session.players.len()
                } else {
                    *index < session.hostiles.len()
                };
                if valid {
                    session.active_player = *player;
                    session.active_index = *index;
                    session.notice = format!(
                        "{} {} selected · click a {} highlighted surface to place or reposition.",
                        if *player { "PLAYER" } else { "HOSTILE" },
                        index + 1,
                        if *player { "BLUE" } else { "RED" }
                    );
                }
            }
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
                session.player_placements.fill(None);
                session.hostile_placements.fill(None);
                let mut occupancy = UnitOccupancy::default();
                for (index, (placement, surface)) in session
                    .player_placements
                    .iter_mut()
                    .zip(&session.player_surfaces)
                    .enumerate()
                {
                    *placement = Some(*surface);
                    occupancy.relocate(deployment_unit_id(true, index), *surface);
                }
                let hostile = session
                    .hostile_surfaces
                    .iter()
                    .filter(|surface| !occupancy.is_occupied(**surface, None))
                    .copied()
                    .take(session.hostile_placements.len())
                    .collect::<Vec<_>>();
                for (index, (placement, surface)) in session
                    .hostile_placements
                    .iter_mut()
                    .zip(hostile)
                    .enumerate()
                {
                    *placement = Some(surface);
                    occupancy.relocate(deployment_unit_id(false, index), surface);
                }
                session.undo.clear();
                advance_deployment_cursor(session);
            }
            DeploymentAction::Back => {
                if let Some(deployment) = deployment_snapshot(session) {
                    lab_state.preserved_deployment = Some(deployment);
                    lab_state.bump();
                }
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
                let Some(table) = runtime.markers.table.as_deref() else {
                    session.notice = "Terrain rules are still loading.".to_owned();
                    continue;
                };
                if let Some(notice) = session.capacity_notice() {
                    session.notice = notice;
                    continue;
                }
                if !session.complete() {
                    session.notice = "Every roster entry needs one unique surface.".to_owned();
                    continue;
                }
                let footing = deployment_footing(
                    &runtime.markers.tiles,
                    table,
                    runtime.markers.blockers.as_deref(),
                );
                let mut players = runtime
                    .units
                    .iter_mut()
                    .filter(|(_, _, faction, _, _, _, _)| **faction == Faction::Player)
                    .map(|(entity, _, _, _, _, _, _)| entity)
                    .collect::<Vec<_>>();
                let mut hostiles = runtime
                    .units
                    .iter_mut()
                    .filter(|(_, _, faction, _, _, _, _)| **faction == Faction::Hostile)
                    .map(|(entity, _, _, _, _, _, _)| entity)
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
                        if let Ok((_, _, _, _, _, mut on, mut transform)) =
                            runtime.units.get_mut(*entity)
                        {
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
                if let (Some(lab), Some(accepted)) =
                    (runtime.lab.as_deref(), runtime.accepted.as_deref())
                {
                    let units = runtime
                        .units
                        .iter_mut()
                        .map(|(_, id, faction, archetype, name, on, _)| {
                            (
                                *id,
                                *faction,
                                archetype.0.clone(),
                                name.as_str().to_owned(),
                                on.0.pos,
                            )
                        })
                        .collect();
                    commands.insert_resource(report_launch(lab, accepted.fingerprint(), units));
                }
                for (entity, mut visibility) in &mut runtime.hidden_presentation {
                    *visibility = Visibility::Inherited;
                    commands.entity(entity).remove::<DeploymentHidden>();
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
        rebuild_deployment_markers(&mut commands, &runtime.markers, session);
    }
}

fn capture_fixed_fixture_report_launch(
    mut commands: Commands,
    lab: Option<Res<CombatLabSession>>,
    accepted: Option<Res<AcceptedContentRevision>>,
    existing: Option<Res<CombatLabReportLaunch>>,
    units: Query<(&hex_core::UnitId, &Faction, &Archetype, &Name, &StandsOn)>,
) {
    let (Some(lab), Some(accepted)) = (lab.as_deref(), accepted.as_deref()) else {
        return;
    };
    if existing.is_some() || !matches!(lab.kind, CombatLabSessionKind::FixedFixture(_)) {
        return;
    }
    let units = units
        .iter()
        .map(|(id, faction, archetype, name, on)| {
            (
                *id,
                *faction,
                archetype.0.clone(),
                name.as_str().to_owned(),
                on.0.pos,
            )
        })
        .collect();
    commands.insert_resource(report_launch(lab, accepted.fingerprint(), units));
}

fn report_launch(
    lab: &CombatLabSession,
    content_revision: u64,
    mut units: Vec<(hex_core::UnitId, Faction, String, String, TilePos)>,
) -> CombatLabReportLaunch {
    units.sort_by_key(|(id, ..)| *id);
    let roster = |faction| {
        units
            .iter()
            .filter(|(_, side, ..)| *side == faction)
            .map(
                |(id, _, archetype, display_name, _)| CombatLabReportRosterEntry {
                    unit_id: id.0,
                    archetype: archetype.clone(),
                    display_name: display_name.clone(),
                    controller: if faction == Faction::Player {
                        CombatLabReportController::Human
                    } else {
                        CombatLabReportController::BaselineAi
                    },
                },
            )
            .collect()
    };
    let deployment = |faction| {
        units
            .iter()
            .filter(|(_, side, ..)| *side == faction)
            .map(|(_, _, _, _, position)| *position)
            .collect()
    };
    CombatLabReportLaunch {
        origin: match &lab.kind {
            CombatLabSessionKind::Sandbox => CombatLabReportOrigin::Sandbox,
            CombatLabSessionKind::FixedFixture(stable_id) => CombatLabReportOrigin::FixedFixture {
                stable_id: stable_id.clone(),
            },
        },
        map: lab.report_map.clone(),
        content_revision,
        rosters: CombatLabReportRosters {
            players: roster(Faction::Player),
            hostiles: roster(Faction::Hostile),
        },
        deployment: CombatLabReportDeployment {
            players: deployment(Faction::Player),
            hostiles: deployment(Faction::Hostile),
        },
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

/// Validates and installs the frozen effective Lab rules before gameplay admission.
pub(crate) fn apply_combat_rules_profile(world: &mut World) {
    let Some(session) = world.get_resource::<CombatLabSession>().cloned() else {
        world.remove_resource::<CombatLabRulesStatus>();
        return;
    };
    if world
        .get_resource::<CombatLabRulesStatus>()
        .is_some_and(|status| *status == CombatLabRulesStatus::Ready)
    {
        return;
    }
    match session.profile.effective_settings(&session.shipped_combat) {
        Ok(effective) => {
            world.insert_resource(effective);
            world.insert_resource(CombatLabRulesStatus::Ready);
        }
        Err(error) => {
            if world
                .get_resource::<CombatLabRulesStatus>()
                .is_none_or(|status| *status != CombatLabRulesStatus::Invalid)
            {
                error!("Combat Lab rules profile refused before gameplay: {error}");
            }
            world.remove_resource::<CombatSettings>();
            world.insert_resource(GameplaySetupFailure::new(format!(
                "Combat Lab rules profile was refused: {error}"
            )));
            world.insert_resource(ScenarioContractStatus::Invalid);
            world.insert_resource(CombatLabRulesStatus::Invalid);
        }
    }
}

fn restore_shipped_combat_rules(mut commands: Commands, session: Option<Res<CombatLabSession>>) {
    if let Some(session) = session {
        commands.insert_resource(session.shipped_combat.clone());
    }
    commands.remove_resource::<CombatLabRulesStatus>();
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

fn apply_fixed_fixture_initial_state(
    mut commands: Commands,
    session: Option<Res<CombatLabSession>>,
    content: Option<Res<ContentIndex>>,
    elements: Option<Res<ElementCatalog>>,
    mut units: Query<(
        &Faction,
        &Archetype,
        &hex_lattice::LatticeSpec,
        &mut hex_lattice::LatticeState,
    )>,
) {
    let (Some(setup), Some(content), Some(elements)) = (
        session.as_deref().and_then(|session| session.initial_state),
        content.as_deref(),
        elements.as_deref(),
    ) else {
        return;
    };
    match setup {
        FixedFixtureInitialState::ChannelAttrition => {
            let tables = content.tables(elements);
            for (faction, archetype, spec, mut state) in &mut units {
                match (*faction, archetype.0.as_str()) {
                    // Both controllers begin with missing mana, so the human can
                    // repeat Channel and baseline AI has the same canonical option.
                    (Faction::Player | Faction::Hostile, "hedge-mage") => {
                        if let Err(error) = apply_fixture_cast(spec, &mut state, &tables, false) {
                            commands.insert_resource(GameplaySetupFailure::new(error));
                            return;
                        }
                    }
                    // Locked mana proves Channel does not refill funding already
                    // committed to an enchantment.
                    (Faction::Player, "raider") => {
                        if let Err(error) = apply_fixture_cast(spec, &mut state, &tables, true) {
                            commands.insert_resource(GameplaySetupFailure::new(error));
                            return;
                        }
                    }
                    // One live-but-damaged unit exercises disabled-cell exclusion.
                    (Faction::Player, "wolf") => {
                        if let Some((coord, _)) = spec.cells().next() {
                            hex_lattice::apply_disables(&mut state, &[coord]);
                        }
                    }
                    // One fully disabled body proves the downed-unit refusal path.
                    (Faction::Hostile, "wolf") => {
                        let cells = spec.cells().map(|(coord, _)| coord).collect::<Vec<_>>();
                        hex_lattice::apply_disables(&mut state, &cells);
                    }
                    // The hostile raider remains completely full as the no-op
                    // Channel/cap reference state.
                    _ => {}
                }
            }
        }
    }
}

fn apply_fixture_cast(
    spec: &hex_lattice::LatticeSpec,
    state: &mut hex_lattice::LatticeState,
    tables: &hex_assets::ContentTables<'_>,
    enchantment: bool,
) -> Result<(), String> {
    let plan = spec.cells().find_map(|(coord, kind)| {
        if !matches!(kind, hex_lattice::CellKind::Spell { .. }) {
            return None;
        }
        let plan = hex_lattice::castable(spec, state, coord, tables).ok()?;
        let is_enchantment = matches!(
            tables.casting(plan.spell),
            hex_lattice::Casting::Enchantment { .. }
        );
        (is_enchantment == enchantment).then_some(plan)
    });
    let Some(plan) = plan else {
        return Err(format!(
            "Channel fixture could not find a castable {} spell.",
            if enchantment {
                "enchantment"
            } else {
                "non-enchantment"
            }
        ));
    };
    if !hex_lattice::apply_cast(state, &plan, tables) {
        return Err("Channel fixture cast plan failed to apply atomically.".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use bevy::state::app::StatesPlugin;
    use hex_assets::{
        AcceptedContentRevision, ArtPalette, ElementFile, SubstanceFile, SubstanceTable,
    };
    use hex_core::COMBAT_LAB_FIXTURES;

    use super::*;

    fn rules_session(profile: CombatRulesProfile, shipped: CombatSettings) -> CombatLabSession {
        CombatLabSession {
            kind: CombatLabSessionKind::Sandbox,
            return_to: Screen::CombatLab,
            profile,
            shipped_combat: shipped,
            report_map: CombatLabReportMap {
                catalog_id: "flat-arena".to_owned(),
                scenario: "Flat Arena".to_owned(),
                resolved_seed: Some(42),
            },
            initial_state: None,
        }
    }

    #[test]
    fn report_launch_freezes_stable_roster_order_and_exact_initial_surfaces() {
        let shipped = CombatSettings::default();
        let session = rules_session(CombatRulesProfile::shipped(&shipped), shipped);
        let player = TilePos::new(HexCoord::ORIGIN, 5);
        let hostile = TilePos::new(HexCoord::ORIGIN, 1);
        let launch = report_launch(
            &session,
            77,
            vec![
                (
                    hex_core::UnitId(9),
                    Faction::Hostile,
                    "raider".to_owned(),
                    "Raider".to_owned(),
                    hostile,
                ),
                (
                    hex_core::UnitId(2),
                    Faction::Player,
                    "hedge-mage".to_owned(),
                    "Hedge Mage".to_owned(),
                    player,
                ),
            ],
        );
        assert_eq!(launch.origin, CombatLabReportOrigin::Sandbox);
        assert_eq!(launch.content_revision, 77);
        assert_eq!(
            launch
                .rosters
                .players
                .iter()
                .map(|entry| entry.archetype.as_str())
                .collect::<Vec<_>>(),
            vec!["hedge-mage"]
        );
        assert_eq!(
            launch
                .rosters
                .hostiles
                .iter()
                .map(|entry| entry.controller)
                .collect::<Vec<_>>(),
            vec![CombatLabReportController::BaselineAi]
        );
        assert_eq!(launch.deployment.players, vec![player]);
        assert_eq!(launch.deployment.hostiles, vec![hostile]);
    }

    #[test]
    fn loading_installs_a_frozen_tactical_profile_without_changing_shipped_snapshot() {
        let shipped = CombatSettings::default();
        let profile = CombatRulesProfile::tactical_two_step(&shipped);
        let mut app = App::new();
        app.insert_resource(shipped.clone());
        app.insert_resource(rules_session(profile.clone(), shipped.clone()));

        apply_combat_rules_profile(app.world_mut());

        assert_eq!(
            app.world().resource::<CombatSettings>().movement_per_turn,
            2
        );
        let session = app.world().resource::<CombatLabSession>();
        assert_eq!(session.profile, profile);
        assert_eq!(session.shipped_combat, shipped);
        assert_eq!(
            *app.world().resource::<CombatLabRulesStatus>(),
            CombatLabRulesStatus::Ready
        );
    }

    #[test]
    fn loading_fails_closed_on_a_profile_that_lies_about_preset_identity() {
        let shipped = CombatSettings::default();
        let mut profile = CombatRulesProfile::shipped(&shipped);
        profile.movement_per_turn = 3;
        let mut app = App::new();
        app.insert_resource(shipped.clone());
        app.insert_resource(rules_session(profile, shipped));

        apply_combat_rules_profile(app.world_mut());

        assert!(!app.world().contains_resource::<CombatSettings>());
        assert_eq!(
            *app.world().resource::<CombatLabRulesStatus>(),
            CombatLabRulesStatus::Invalid
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
    fn channel_attrition_materializes_every_declared_lattice_state() {
        let fixture = content_fixture().expect("shipped content fixture should resolve");
        let shipped = CombatSettings::default();
        let session = CombatLabSession {
            initial_state: Some(FixedFixtureInitialState::ChannelAttrition),
            ..rules_session(CombatRulesProfile::shipped(&shipped), shipped)
        };
        let mut app = App::new();
        app.insert_resource(session)
            .insert_resource(fixture.shipped.content.clone())
            .insert_resource(fixture.elements.clone())
            .add_systems(Update, apply_fixed_fixture_initial_state);
        for faction in [Faction::Player, Faction::Hostile] {
            for name in ["hedge-mage", "raider", "wolf"] {
                let archetype = fixture
                    .shipped
                    .lattices
                    .get(name)
                    .unwrap_or_else(|| panic!("missing shipped {name}"));
                app.world_mut().spawn((
                    faction,
                    Archetype(name.to_owned()),
                    archetype.spec.clone(),
                    hex_lattice::LatticeState::new(&archetype.spec, &archetype.stats),
                ));
            }
        }
        app.update();

        let mut units = app.world_mut().query::<(
            &Faction,
            &Archetype,
            &hex_lattice::LatticeSpec,
            &hex_lattice::LatticeState,
        )>();
        for (faction, archetype, spec, state) in units.iter(app.world()) {
            match (*faction, archetype.0.as_str()) {
                (Faction::Player | Faction::Hostile, "hedge-mage") => {
                    let fresh = fixture
                        .shipped
                        .lattices
                        .get("hedge-mage")
                        .expect("fresh mage");
                    let fresh = hex_lattice::LatticeState::new(&fresh.spec, &fresh.stats);
                    assert!(state.total_gem_mana() < fresh.total_gem_mana());
                }
                (Faction::Player, "raider") => {
                    assert_eq!(state.enchantment_count(), 1);
                    assert!(state.total_locked_mana() > 0);
                }
                (Faction::Player, "wolf") => {
                    let disabled = spec
                        .cells()
                        .filter(|(coord, _)| state.is_disabled(*coord))
                        .count();
                    assert_eq!(disabled, 1);
                }
                (Faction::Hostile, "wolf") => {
                    assert!(spec.cells().all(|(coord, _)| state.is_disabled(coord)));
                }
                (Faction::Hostile, "raider") => {
                    assert_eq!(state.enchantment_count(), 0);
                    assert_eq!(state.total_locked_mana(), 0);
                }
                _ => {}
            }
        }
    }

    fn app_with_content_fixture(fixture: &ContentFixture) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.insert_state(Screen::Loading);
        app.insert_resource(fixture.element_file.clone());
        app.insert_resource(fixture.elements.clone());
        app.insert_resource(fixture.substance_file.clone());
        app.insert_resource(fixture.palette.clone());
        app.insert_resource(fixture.substances.clone());
        fixture.shipped.insert_into_world(app.world_mut());
        app.add_plugins(hex_assets::content_index::plugin);
        app.add_systems(
            PostUpdate,
            apply_creator_content_overlay
                .before(hex_assets::ContentReadinessSystems::PublishAcceptedRevision),
        );
        app
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
    fn exact_deployment_is_complete_only_when_every_unit_has_a_surface() {
        let first = TilePos::new(HexCoord::ORIGIN, 2);
        let second = TilePos::new(HexCoord::new_cubic(1, -1, 0), 5);
        let mut placements = vec![Some(first), None];
        assert!(!placements_complete_exact(&placements, 2));
        *placements.get_mut(1).expect("second placement") = Some(second);
        assert!(placements_complete_exact(&placements, 2));
    }

    #[test]
    fn deployment_capacity_shortfall_is_actionable_data() {
        let region = CombatLabDeploymentRegion {
            center: CombatLabRegionCenter::Fixed(hex_assets::CubeCoord { x: 0, y: 0, z: 0 }),
            radius: 1,
        };
        let map_definition = CombatLabMapDefinition {
            id: "capacity-test".to_owned(),
            display_name: "Capacity Test".to_owned(),
            description: String::new(),
            tags: Vec::new(),
            preview: String::new(),
            scenario: "Flat Arena".to_owned(),
            fixed_seed: Some(1),
            player_region: region.clone(),
            hostile_region: region,
        };
        let roster = || {
            vec![
                RosterChoice::Template("wolf".to_owned()),
                RosterChoice::Template("raider".to_owned()),
            ]
        };
        let mut session = DeploymentSession::new(map_definition, roster(), roster(), None);
        session.player_surfaces = vec![TilePos::new(HexCoord::ORIGIN, 1)];
        session.hostile_surfaces = vec![
            TilePos::new(HexCoord::ORIGIN, 2),
            TilePos::new(HexCoord::from_axial(1, 0), 2),
        ];

        assert_eq!(
            session.capacity_notice().as_deref(),
            Some(
                "Player region provides 1 of 2 required surfaces. Go Back and reduce that roster \
                 or choose another map."
            )
        );
        session
            .player_surfaces
            .push(TilePos::new(HexCoord::from_axial(-1, 0), 1));
        assert_eq!(session.capacity_notice(), None);
    }

    #[test]
    fn placement_markers_keep_faction_roster_number_and_exact_elevation() {
        let map_definition = CombatLabMapDefinition {
            id: "marker-test".to_owned(),
            display_name: "Marker Test".to_owned(),
            description: String::new(),
            tags: Vec::new(),
            preview: String::new(),
            scenario: "Flat Arena".to_owned(),
            fixed_seed: Some(1),
            player_region: CombatLabDeploymentRegion {
                center: CombatLabRegionCenter::Fixed(hex_assets::CubeCoord { x: 0, y: 0, z: 0 }),
                radius: 1,
            },
            hostile_region: CombatLabDeploymentRegion {
                center: CombatLabRegionCenter::Fixed(hex_assets::CubeCoord { x: 2, y: -2, z: 0 }),
                radius: 1,
            },
        };
        let mut session = DeploymentSession::new(
            map_definition,
            vec![
                RosterChoice::Template("wolf".to_owned()),
                RosterChoice::Template("hedge-mage".to_owned()),
            ],
            vec![RosterChoice::Template("raider".to_owned())],
            None,
        );
        let player = TilePos::new(HexCoord::ORIGIN, 3);
        let hostile = TilePos::new(HexCoord::new_cubic(2, -2, 0), 7);
        session.player_placements = vec![None, Some(player)];
        session.hostile_placements = vec![Some(hostile)];

        assert_eq!(
            resolved_deployment_markers(&session).collect::<Vec<_>>(),
            vec![(true, 1, player), (false, 0, hostile)]
        );

        let second_player = TilePos::new(HexCoord::from_axial(1, 0), 3);
        let stacked_hostile = TilePos::new(player.coord, player.level + 1);
        session.player_placements = vec![Some(player), Some(second_player)];
        session.hostile_placements = vec![Some(player)];
        session.player_surfaces = vec![player, second_player];
        session.hostile_surfaces = vec![player, stacked_hostile];
        assert!(
            !session.complete(),
            "deployment uses the shared exact-surface overlap rule"
        );
        session.hostile_placements = vec![Some(stacked_hostile)];
        assert!(
            session.complete(),
            "stacked surfaces at distinct elevations remain distinct"
        );
        assert_eq!(
            deployment_snapshot(&session),
            Some(CombatLabReportDeployment {
                players: vec![player, second_player],
                hostiles: vec![stacked_hostile],
            }),
            "leaving deployment must preserve the tester's latest exact surfaces"
        );
    }

    #[test]
    fn fixture_ids_are_unique_and_stable() {
        let ids: std::collections::BTreeSet<_> = COMBAT_LAB_FIXTURES
            .iter()
            .map(|fixture| fixture.id)
            .collect();
        assert_eq!(ids.len(), COMBAT_LAB_FIXTURES.len());
        assert!(ids.contains("ability-lab"));
        assert!(ids.contains("creator-spell-matrix"));
        assert!(ids.contains("occupancy-matrix"));
        assert!(ids.contains("channel-attrition"));
        assert!(ids.contains("tempo-matrix"));
    }

    #[test]
    fn creator_overlay_publishes_one_accepted_raw_and_derived_revision() {
        let fixture = content_fixture().expect("shipped content fixture should resolve");
        let mut app = app_with_content_fixture(&fixture);
        app.update();
        let shipped_revision = app
            .world()
            .resource::<AcceptedContentRevision>()
            .fingerprint();

        app.insert_resource(CreatorContentOverlay {
            active: fixture.active,
            shipped: fixture.shipped,
            display_names: BTreeMap::new(),
        });
        app.update();

        let world = app.world();
        let accepted = world.resource::<AcceptedContentRevision>();
        assert_ne!(accepted.fingerprint(), shipped_revision);
        assert!(
            accepted.matches_resolved(
                world.resource::<ContentIndex>(),
                world.resource::<LatticeLibrary>()
            ),
            "the creator graph should be accepted through the normal publisher"
        );
        assert!(world
            .resource::<SpellBook>()
            .matches_source(world.resource::<SpellFile>()));
        assert!(world.resource::<LatticeLibrary>().matches_sources(
            world.resource::<LatticeFile>(),
            world.resource::<ElementCatalog>(),
            world.resource::<SpellBook>()
        ));
    }

    #[test]
    fn partial_creator_bundle_cannot_remain_accepted() {
        let fixture = content_fixture().expect("shipped content fixture should resolve");
        let shipped_raw_spells = fixture.shipped.spell_file.clone();
        let mut app = app_with_content_fixture(&fixture);
        app.insert_resource(CreatorContentOverlay {
            active: fixture.active,
            shipped: fixture.shipped,
            display_names: BTreeMap::new(),
        });
        app.update();
        assert!(app.world().contains_resource::<AcceptedContentRevision>());

        app.world_mut().remove_resource::<CreatorContentOverlay>();
        app.insert_resource(shipped_raw_spells);
        app.update();

        assert!(
            !app.world().contains_resource::<AcceptedContentRevision>(),
            "a shipped raw file paired with creator-derived tables must fail closed"
        );
    }

    #[test]
    fn exit_retry_and_reentry_restore_the_shipped_accepted_revision() {
        let fixture = content_fixture().expect("shipped content fixture should resolve");
        let mut app = app_with_content_fixture(&fixture);
        let overlay = CreatorContentOverlay {
            active: fixture.active,
            shipped: fixture.shipped,
            display_names: BTreeMap::new(),
        };
        app.add_systems(
            PreUpdate,
            restore_creator_content_once.run_if(resource_exists::<RestoreCreatorContent>),
        );
        app.update();
        let shipped_revision = app
            .world()
            .resource::<AcceptedContentRevision>()
            .fingerprint();

        for _ in 0..2 {
            app.insert_resource(overlay.clone());
            app.update();
            assert_ne!(
                app.world()
                    .resource::<AcceptedContentRevision>()
                    .fingerprint(),
                shipped_revision,
                "each Creator entry should install its frozen revision"
            );

            app.insert_resource(RestoreCreatorContent);
            app.update();
            let world = app.world();
            assert!(!world.contains_resource::<CreatorContentOverlay>());
            assert_eq!(
                world.resource::<AcceptedContentRevision>().fingerprint(),
                shipped_revision,
                "exit and the next retry/re-entry must start from shipped content"
            );
            assert!(world
                .resource::<SpellBook>()
                .matches_source(world.resource::<SpellFile>()));
        }
    }
}
