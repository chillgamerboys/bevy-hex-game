//! Default-off typed launch requests and immutable gameplay observations.
//!
//! Shipping builds do not compile this module. Deterministic fixtures remain useful
//! to simulations, app contracts, and review tooling without becoming navigation or
//! product persistence.

use bevy::prelude::*;
use hex_assets::{
    character_runtime_key, CombatRulesProfile, CombatSettings, CreationLibraryFile, CubeCoord,
    CustomCharacterId, Encounter, EncounterFaction, EncounterPlacement, FixedSettingsFreeze,
    FormationCenter, LatticeFile, Roster, RosterEntry, SandboxMapCatalog, ScenarioLibrary,
    SpellFile,
};
use hex_combat::{CombatSummary, EncounterResolution};
use hex_core::{
    DeterministicFixtureInitialState, DeterministicRosterEntry, DeterministicRosterPlacement,
    GameplayPhase, GameplaySetup, GameplaySetupFailure, Mode, PendingDecision, ResolvedMapSeed,
    Screen, TilePos, Turn, UnitId,
};
use hex_gameplay_model::{CampaignSlotId, SandboxCharacter, SandboxMapSelection};
use hex_lattice::SpellTable as _;
use hex_lattice::{LatticeSpec, LatticeState};
use hex_units::{Archetype, Downed, Faction, StandsOn};

use crate::screens::{gameplay, sandbox};

pub use hex_core::{deterministic_fixture, DeterministicFixtureDefinition, DETERMINISTIC_FIXTURES};
pub use hex_ui::test_support::HeadlessUiPlugin;

/// Registers test-only fixture mutations after gameplay actors have spawned.
pub(crate) fn plugin(app: &mut App) {
    app.add_systems(
        OnEnter(Screen::Gameplay),
        apply_deterministic_fixture_initial_state.in_set(GameplaySetup::Restore),
    )
    .add_systems(OnEnter(Screen::Title), clear_deterministic_fixture_session)
    .add_systems(
        OnEnter(Screen::Sandbox),
        clear_deterministic_fixture_session,
    );
}

fn clear_deterministic_fixture_session(
    mut commands: Commands,
    session: Option<Res<DeterministicFixtureSession>>,
    origin: Option<Res<sandbox::GameplaySessionOrigin>>,
) {
    let fixture_owned = session.is_some()
        || matches!(
            origin.as_deref(),
            Some(sandbox::GameplaySessionOrigin::TestFixture(_))
        );
    if !fixture_owned {
        return;
    }
    if let Some(shipped) = session
        .as_deref()
        .and_then(|session| session.shipped_rules.clone())
    {
        commands.insert_resource(shipped);
    }
    commands.remove_resource::<FixedSettingsFreeze<CombatSettings>>();
    commands.remove_resource::<FixedSettingsFreeze<SpellFile>>();
    commands.remove_resource::<FixedSettingsFreeze<LatticeFile>>();
    commands.remove_resource::<DeterministicFixtureSession>();
    if matches!(
        origin.as_deref(),
        Some(sandbox::GameplaySessionOrigin::TestFixture(_))
    ) {
        commands.remove_resource::<sandbox::GameplaySessionOrigin>();
    }
}

/// Exact Creator records retained outside the shipping asset graph.
pub fn deterministic_creator_library() -> Result<CreationLibraryFile, String> {
    let library: CreationLibraryFile = ron::from_str(include_str!(
        "../testdata/deterministic_creator_library.ron"
    ))
    .map_err(|error| format!("deterministic Creator library is invalid RON: {error}"))?;
    library.validate_integrity().map_err(|issues| {
        format!(
            "deterministic Creator library failed integrity validation: {}",
            issues.join("; ")
        )
    })?;
    Ok(library)
}

/// Typed request to launch one immutable deterministic fixture.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct DeterministicFixtureLaunchRequest {
    /// Definition resolved once at the typed boundary.
    definition: &'static DeterministicFixtureDefinition,
    /// Optional non-shipping combat profile injected by the test.
    pub rules_profile: Option<CombatRulesProfile>,
}

impl DeterministicFixtureLaunchRequest {
    /// Creates a request only when the shared manifest owns the identity.
    pub fn new(
        stable_id: impl Into<String>,
        rules_profile: Option<CombatRulesProfile>,
    ) -> Result<Self, String> {
        let stable_id = stable_id.into();
        let definition = deterministic_fixture(&stable_id)
            .ok_or_else(|| format!("unknown deterministic fixture {stable_id:?}"))?;
        Ok(Self {
            definition,
            rules_profile,
        })
    }

    /// Resolves the immutable shared definition.
    #[must_use]
    pub fn definition(&self) -> &'static DeterministicFixtureDefinition {
        self.definition
    }
}

/// Renderer-free observation of a deterministic fixture request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeterministicFixtureLaunchSnapshot {
    /// Stable machine identity.
    pub stable_id: String,
    /// Scenario launch contract selected by the shared manifest.
    pub scenario: String,
    /// Sandbox catalog identity associated with the fixture.
    pub sandbox_map: String,
    /// Exact expected Party count.
    pub party_count: usize,
    /// Exact expected Enemy count.
    pub enemy_count: usize,
    /// Exact Party identities in stable launch order.
    pub party: Vec<String>,
    /// Exact Enemy identities in stable launch order.
    pub enemies: Vec<String>,
    /// Exact post-spawn mutation, when the fixture owns one.
    pub initial_state: Option<DeterministicFixtureInitialState>,
    /// Whether exact test-only Creator content is required.
    pub creator_content: bool,
    /// Whether a test-only rules override was requested.
    pub rules_override: bool,
}

fn fixture_character(entry: DeterministicRosterEntry) -> SandboxCharacter<CustomCharacterId> {
    match entry {
        DeterministicRosterEntry::Shipped(key) => SandboxCharacter::Template(key.to_owned()),
        DeterministicRosterEntry::Creator(id) => SandboxCharacter::Custom(CustomCharacterId(id)),
    }
}

fn fixture_character_key(entry: DeterministicRosterEntry) -> String {
    match entry {
        DeterministicRosterEntry::Shipped(key) => key.to_owned(),
        DeterministicRosterEntry::Creator(id) => character_runtime_key(CustomCharacterId(id)),
    }
}

fn fixture_placement(placement: DeterministicRosterPlacement) -> EncounterPlacement {
    match placement {
        DeterministicRosterPlacement::Fixed { x, y, z } => {
            EncounterPlacement::Fixed(CubeCoord { x, y, z })
        }
        DeterministicRosterPlacement::Formation { x, y, z, spread } => {
            EncounterPlacement::Formation {
                center: FormationCenter::Fixed(CubeCoord { x, y, z }),
                spread,
            }
        }
    }
}

fn fixture_encounter(definition: &DeterministicFixtureDefinition) -> Encounter {
    let roster = |faction, entries: &[DeterministicRosterEntry], placement| Roster {
        faction,
        placement: fixture_placement(placement),
        units: entries
            .iter()
            .copied()
            .map(|entry| RosterEntry {
                archetype: fixture_character_key(entry),
                placement: None,
                ai_profile: None,
                ai_group: None,
            })
            .collect(),
    };
    Encounter {
        name: format!("Deterministic fixture · {}", definition.name),
        rosters: vec![
            roster(
                EncounterFaction::Player,
                definition.party,
                definition.party_placement,
            ),
            roster(
                EncounterFaction::Hostile,
                definition.enemies,
                definition.enemy_placement,
            ),
        ],
    }
}

/// Observes a typed fixture request without constructing shipping UI state.
#[must_use]
pub fn deterministic_fixture_launch_snapshot(
    request: &DeterministicFixtureLaunchRequest,
) -> DeterministicFixtureLaunchSnapshot {
    let definition = request.definition();
    DeterministicFixtureLaunchSnapshot {
        stable_id: definition.id.to_owned(),
        scenario: definition.scenario.to_owned(),
        sandbox_map: definition.sandbox_map.to_owned(),
        party_count: definition.party.len(),
        enemy_count: definition.enemies.len(),
        party: definition
            .party
            .iter()
            .copied()
            .map(fixture_character_key)
            .collect(),
        enemies: definition
            .enemies
            .iter()
            .copied()
            .map(fixture_character_key)
            .collect(),
        initial_state: definition.initial_state,
        creator_content: definition
            .party
            .iter()
            .chain(definition.enemies)
            .any(|entry| matches!(entry, DeterministicRosterEntry::Creator(_))),
        rules_override: request.rules_profile.is_some(),
    }
}

/// Resolves and installs one deterministic fixture through the internal Scenario
/// launch contract, optionally replacing combat settings for this disposable test.
///
/// Shipping code cannot call this API because the entire module and the optional
/// profile vocabulary are gated behind `hex_game/test-support`.
pub fn install_deterministic_fixture_launch(
    world: &mut World,
    request: &DeterministicFixtureLaunchRequest,
) -> Result<DeterministicFixtureLaunchSnapshot, String> {
    let definition = request.definition();
    let scenarios = world
        .get_resource::<ScenarioLibrary>()
        .ok_or_else(|| "deterministic fixture launch requires ScenarioLibrary".to_owned())?;
    let scenario = scenarios
        .scenarios
        .iter()
        .find(|scenario| scenario.name == definition.scenario)
        .cloned()
        .ok_or_else(|| {
            format!(
                "deterministic fixture {:?} names unavailable scenario {:?}",
                definition.id, definition.scenario
            )
        })?;
    let maps = world
        .get_resource::<SandboxMapCatalog>()
        .ok_or_else(|| "deterministic fixture launch requires SandboxMapCatalog".to_owned())?;
    let _map = maps.get(definition.sandbox_map).ok_or_else(|| {
        format!(
            "deterministic fixture {:?} names unavailable Sandbox map {:?}",
            definition.id, definition.sandbox_map
        )
    })?;
    let resolved_seed = scenario.generation_seed;
    let rules_override = if let Some(profile) = request.rules_profile.as_ref() {
        let shipped = world
            .get_resource::<CombatSettings>()
            .cloned()
            .ok_or_else(|| {
                "a deterministic rules override requires shipped CombatSettings".to_owned()
            })?;
        let effective = profile.effective_settings(&shipped)?;
        Some((shipped, effective))
    } else {
        None
    };
    let party = definition
        .party
        .iter()
        .copied()
        .map(fixture_character)
        .collect::<Vec<_>>();
    let enemies = definition
        .enemies
        .iter()
        .copied()
        .map(fixture_character)
        .collect::<Vec<_>>();
    if party
        .iter()
        .chain(&enemies)
        .any(|character| matches!(character, SandboxCharacter::Custom(_)))
    {
        let library = deterministic_creator_library()?;
        let overlay =
            sandbox::build_deterministic_creator_overlay(world, &party, &enemies, &library)?;
        world.insert_resource(overlay);
        world.insert_resource(FixedSettingsFreeze::<SpellFile>::default());
        world.insert_resource(FixedSettingsFreeze::<LatticeFile>::default());
    }
    world.insert_resource(crate::scenarios::ScenarioToLoad {
        scenario,
        resolved_seed: resolved_seed.map(ResolvedMapSeed),
        encounter_override: Some(fixture_encounter(definition)),
    });
    if let Some((_, effective)) = rules_override.as_ref() {
        world.insert_resource(FixedSettingsFreeze::<CombatSettings>::default());
        world.insert_resource(effective.clone());
    }
    world.insert_resource(DeterministicFixtureSession {
        stable_id: definition.id.to_owned(),
        initial_state: definition.initial_state,
        shipped_rules: rules_override.map(|(shipped, _)| shipped),
    });
    install_test_fixture_origin(world, definition.id);
    Ok(deterministic_fixture_launch_snapshot(request))
}

/// Frozen test-only fixture state reused on every exact entry into Gameplay.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct DeterministicFixtureSession {
    /// Stable manifest identity retained across exact retries.
    pub stable_id: String,
    /// Exact post-spawn mutation reapplied on each Gameplay entry.
    pub initial_state: Option<DeterministicFixtureInitialState>,
    /// Shipped rules restored when a profile-injected fixture is abandoned.
    pub shipped_rules: Option<CombatSettings>,
}

fn apply_deterministic_fixture_initial_state(
    mut commands: Commands,
    session: Option<Res<DeterministicFixtureSession>>,
    content: Option<Res<hex_assets::ContentIndex>>,
    elements: Option<Res<hex_assets::ElementCatalog>>,
    mut units: Query<(
        &Faction,
        &Archetype,
        &hex_lattice::LatticeSpec,
        &mut hex_lattice::LatticeState,
    )>,
) {
    let (Some(DeterministicFixtureInitialState::ChannelAttrition), Some(content), Some(elements)) = (
        session.as_deref().and_then(|session| session.initial_state),
        content.as_deref(),
        elements.as_deref(),
    ) else {
        return;
    };
    let tables = content.tables(elements);
    for (faction, archetype, spec, mut state) in &mut units {
        match (*faction, archetype.0.as_str()) {
            (Faction::Player | Faction::Hostile, "hedge-mage") => {
                if let Err(error) = apply_fixture_cast(spec, &mut state, &tables, false) {
                    commands.insert_resource(GameplaySetupFailure::new(error));
                    return;
                }
            }
            (Faction::Player, "raider") => {
                if let Err(error) = apply_fixture_cast(spec, &mut state, &tables, true) {
                    commands.insert_resource(GameplaySetupFailure::new(error));
                    return;
                }
            }
            (Faction::Player, "wolf") => {
                if let Some((coord, _)) = spec.cells().next() {
                    hex_lattice::apply_disables(&mut state, &[coord]);
                }
            }
            (Faction::Hostile, "wolf") => {
                let cells = spec.cells().map(|(coord, _)| coord).collect::<Vec<_>>();
                hex_lattice::apply_disables(&mut state, &cells);
            }
            _ => {}
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

/// Marks a headless gameplay world with typed fixture provenance.
pub fn install_test_fixture_origin(world: &mut World, stable_id: impl Into<String>) {
    world.insert_resource(sandbox::GameplaySessionOrigin::TestFixture(
        stable_id.into(),
    ));
}

/// Renderer-free provenance exposed without making the shipping resource public.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameplaySessionOriginSnapshot {
    /// A save-eligible Campaign slot.
    Campaign(CampaignSlotId),
    /// A temporary Sandbox run.
    Sandbox,
    /// A non-shipping deterministic fixture.
    TestFixture(String),
}

/// Reads the typed origin that governs save and return behavior.
#[must_use]
pub fn gameplay_session_origin_snapshot(world: &World) -> Option<GameplaySessionOriginSnapshot> {
    world
        .get_resource::<sandbox::GameplaySessionOrigin>()
        .map(|origin| match origin {
            sandbox::GameplaySessionOrigin::Campaign(slot) => {
                GameplaySessionOriginSnapshot::Campaign(*slot)
            }
            sandbox::GameplaySessionOrigin::Sandbox => GameplaySessionOriginSnapshot::Sandbox,
            sandbox::GameplaySessionOrigin::TestFixture(stable_id) => {
                GameplaySessionOriginSnapshot::TestFixture(stable_id.clone())
            }
        })
}

/// Installs an exact Sandbox launch identity for headless app and retry tests.
///
/// This is deliberately test-support-only: production code constructs the same
/// private snapshot after validating the authoritative `SandboxModel`.
#[expect(
    clippy::too_many_arguments,
    reason = "the helper spells out every frozen launch field so tests cannot accidentally infer identity"
)]
pub fn install_sandbox_launch_for_test(
    world: &mut World,
    catalog_id: impl Into<String>,
    resolved_seed: Option<u64>,
    scenario: impl Into<String>,
    party: Vec<SandboxCharacter<CustomCharacterId>>,
    enemies: Vec<SandboxCharacter<CustomCharacterId>>,
    content_revision: Option<u64>,
    deployment: Option<(Vec<TilePos>, Vec<TilePos>)>,
) -> Result<(), String> {
    world.remove_resource::<DeterministicFixtureSession>();
    let scenario_name = scenario.into();
    let library: ScenarioLibrary =
        ron::from_str(include_str!("../../../assets/config/scenarios.ron"))
            .map_err(|error| format!("shipped Scenario library is invalid: {error}"))?;
    let scenario = library
        .scenarios
        .into_iter()
        .find(|candidate| candidate.name == scenario_name)
        .ok_or_else(|| format!("unknown Sandbox test Scenario {scenario_name:?}"))?;
    let archetype = |character: &SandboxCharacter<CustomCharacterId>| match character {
        SandboxCharacter::Template(key) => key.clone(),
        SandboxCharacter::Custom(id) => character_runtime_key(*id),
    };
    let fallback = EncounterPlacement::Formation {
        center: FormationCenter::Fixed(CubeCoord { x: 0, y: 0, z: 0 }),
        spread: 0,
    };
    let initial_encounter = Encounter {
        name: format!("Sandbox test · {scenario_name}"),
        rosters: vec![
            Roster {
                faction: EncounterFaction::Player,
                placement: fallback.clone(),
                units: party
                    .iter()
                    .map(|character| RosterEntry {
                        archetype: archetype(character),
                        placement: None,
                        ai_profile: None,
                        ai_group: None,
                    })
                    .collect(),
            },
            Roster {
                faction: EncounterFaction::Hostile,
                placement: fallback,
                units: enemies
                    .iter()
                    .map(|character| RosterEntry {
                        archetype: archetype(character),
                        placement: None,
                        ai_profile: None,
                        ai_group: None,
                    })
                    .collect(),
            },
        ],
    };
    let mut launch = sandbox::SandboxLaunchSnapshot::new(
        SandboxMapSelection::new(catalog_id, resolved_seed),
        scenario_name,
        party.clone(),
        enemies.clone(),
        content_revision,
        CombatSettings::default(),
        scenario,
        initial_encounter,
    );
    if let Some((party_surfaces, enemy_surfaces)) = deployment {
        let exact = Encounter {
            name: "Sandbox test · exact deployment".to_owned(),
            rosters: [
                (EncounterFaction::Player, &party, &party_surfaces),
                (EncounterFaction::Hostile, &enemies, &enemy_surfaces),
            ]
            .into_iter()
            .map(|(faction, roster, surfaces)| Roster {
                faction,
                placement: EncounterPlacement::Formation {
                    center: FormationCenter::Fixed(CubeCoord { x: 0, y: 0, z: 0 }),
                    spread: 0,
                },
                units: roster
                    .iter()
                    .zip(surfaces)
                    .map(|(character, surface)| RosterEntry {
                        archetype: archetype(character),
                        placement: Some(EncounterPlacement::Surface(*surface)),
                        ai_profile: None,
                        ai_group: None,
                    })
                    .collect(),
            })
            .collect(),
        };
        launch.freeze_deployment(
            sandbox::SandboxDeploymentSnapshot {
                party: party_surfaces,
                enemies: enemy_surfaces,
            },
            exact,
            content_revision,
        );
    }
    world.insert_resource(launch.loading_input());
    world.insert_resource(sandbox::SandboxSession { launch });
    world.insert_resource(sandbox::GameplaySessionOrigin::Sandbox);
    Ok(())
}

/// Stable, product-neutral observation of canonical combat instrumentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombatObservationSnapshot {
    /// Completed rounds.
    pub rounds: u32,
    /// Completed turns.
    pub turns: u32,
    /// Accepted and refused commands.
    pub commands: (u32, u32),
    /// Exact movement distance and budget consumed.
    pub movement: (u32, u32),
    /// Successful Channel actions.
    pub channels: u32,
    /// Raw, prevented, and applied disable totals.
    pub disables: (u32, u32, u32),
    /// Stable unit identities.
    pub units: Vec<UnitId>,
    /// Terminal encounter result, when present.
    pub outcome: Option<hex_combat::EncounterOutcome>,
}

/// Projects canonical instrumentation without parsing rendered text.
#[must_use]
pub fn combat_observation_snapshot(summary: &CombatSummary) -> CombatObservationSnapshot {
    CombatObservationSnapshot {
        rounds: summary.rounds,
        turns: summary.turns,
        commands: (summary.successful_commands, summary.refused_commands),
        movement: (summary.movement_distance, summary.movement_budget_used),
        channels: summary.channels,
        disables: (
            summary.raw_disables,
            summary.prevented_disables,
            summary.applied_disables,
        ),
        units: summary.units.keys().copied().collect(),
        outcome: summary.outcome,
    }
}

/// Exact frozen identity of a Sandbox launch and eventual deployment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxLaunchIdentitySnapshot {
    /// Stable catalog map identity.
    pub catalog_id: String,
    /// Exact generated seed, or authored-map `None`.
    pub resolved_seed: Option<u64>,
    /// Internal Scenario launch identity.
    pub scenario: String,
    /// Flattened Party roster in stable slot order.
    pub party: Vec<String>,
    /// Flattened Enemy roster in stable slot order.
    pub enemies: Vec<String>,
    /// Accepted content revision frozen at deployment.
    pub content_revision: Option<u64>,
    /// Exact shipped combat settings frozen for the run and Retry Exact.
    pub rules: CombatSettings,
    /// Exact Party and Enemy surfaces, once deployment has been confirmed.
    pub deployment: Option<(Vec<TilePos>, Vec<TilePos>)>,
}

/// Reads the exact Sandbox launch snapshot retained for Retry Exact.
#[must_use]
pub fn sandbox_launch_identity_snapshot(world: &World) -> Option<SandboxLaunchIdentitySnapshot> {
    let launch = &world.get_resource::<sandbox::SandboxSession>()?.launch;
    let key = |character: &SandboxCharacter<CustomCharacterId>| match character {
        SandboxCharacter::Template(key) => key.clone(),
        SandboxCharacter::Custom(id) => format!("custom-character-{}", id.0),
    };
    Some(SandboxLaunchIdentitySnapshot {
        catalog_id: launch.map.catalog_id.clone(),
        resolved_seed: launch.map.resolved_seed,
        scenario: launch.scenario.clone(),
        party: launch.party.iter().map(key).collect(),
        enemies: launch.enemies.iter().map(key).collect(),
        content_revision: launch.content_revision,
        rules: launch.rules.clone(),
        deployment: launch
            .deployment
            .as_ref()
            .map(|deployment| (deployment.party.clone(), deployment.enemies.clone())),
    })
}

/// Production gameplay UI registration exposed only to headless integration tests.
pub struct HeadlessGameplayUiPlugin;

impl Plugin for HeadlessGameplayUiPlugin {
    fn build(&self, app: &mut App) {
        gameplay::plugin(app);
    }
}

/// Exact turn facts for the active unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnStateSnapshot {
    /// Movement remaining in canonical hex steps.
    pub movement_left: u32,
    /// Whether the action has been consumed.
    pub acted: bool,
}

/// Exact lattice cell state observed from the owning components.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LatticeCellStateSnapshot {
    /// Stable lattice coordinate.
    pub coord: hex_core::LatticeCoord,
    /// Whether the cell is disabled.
    pub disabled: bool,
    /// Current mana, zero for non-gem cells.
    pub mana: u16,
    /// Whether an enchantment currently locks the cell.
    pub locked: bool,
}

/// Exact authoritative state for one live unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitStateSnapshot {
    /// Stable unit identity.
    pub id: UnitId,
    /// Exact occupied surface, including level.
    pub position: TilePos,
    /// Active turn budget, when this is the acting unit.
    pub turn: Option<TurnStateSnapshot>,
    /// Whether the unit is downed.
    pub downed: bool,
    /// Canonically ordered lattice cells.
    pub lattice: Vec<LatticeCellStateSnapshot>,
}

/// Immutable gameplay observation for headless application tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameplayStateSnapshot {
    /// Current top-level screen.
    pub screen: Option<Screen>,
    /// Current gameplay lifecycle phase.
    pub phase: Option<GameplayPhase>,
    /// Current exploration/combat mode.
    pub mode: Option<Mode>,
    /// Stable initiative order.
    pub turn_order: Vec<UnitId>,
    /// Current actor named by the turn authority.
    pub acting: Option<UnitId>,
    /// Completed round count.
    pub round: u32,
    /// Open canonical decision, if any.
    pub pending: PendingDecision,
    /// Commands currently waiting at the authoritative funnel.
    pub queued_commands: usize,
    /// Application-authorized action projection.
    pub presented_actions: Vec<hex_ui::ActionAffordance>,
    /// Exact unit state in stable id order.
    pub units: Vec<UnitStateSnapshot>,
    /// Current encounter outcome.
    pub outcome: Option<hex_combat::EncounterOutcome>,
    /// Canonical deterministic instrumentation, when installed.
    pub combat: Option<CombatObservationSnapshot>,
}

/// Reads authority resources/components and explicitly named presentation projections.
#[must_use]
pub fn gameplay_state_snapshot(world: &mut World) -> GameplayStateSnapshot {
    let screen = world
        .get_resource::<State<Screen>>()
        .map(|state| *state.get());
    let phase = world.get_resource::<GameplayPhase>().copied();
    let mode = world
        .get_resource::<State<Mode>>()
        .map(|state| *state.get());
    let (turn_order, acting, round) = world.get_resource::<hex_combat::TurnOrder>().map_or_else(
        || (Vec::new(), None, 0),
        |order| (order.order().to_vec(), order.current(), order.round),
    );
    let pending = world
        .get_resource::<PendingDecision>()
        .cloned()
        .unwrap_or_default();
    let queued_commands = world
        .get_resource::<hex_core::CommandQueue>()
        .map_or(0, hex_core::CommandQueue::len);
    let presented_actions = world
        .get_resource::<hex_ui::GameplayHudView>()
        .map_or_else(Vec::new, |view| view.actions.clone());
    let outcome = world
        .get_resource::<EncounterResolution>()
        .and_then(EncounterResolution::outcome);
    let combat = world
        .get_resource::<CombatSummary>()
        .map(combat_observation_snapshot);

    let mut query = world.query::<(
        &UnitId,
        &StandsOn,
        Option<&Turn>,
        Option<&LatticeSpec>,
        Option<&LatticeState>,
        Has<Downed>,
    )>();
    let mut units = query
        .iter(world)
        .map(|(id, standing, turn, spec, state, downed)| {
            let lattice = spec.zip(state).map_or_else(Vec::new, |(spec, state)| {
                let mut cells = spec
                    .cells()
                    .map(|(coord, _)| LatticeCellStateSnapshot {
                        coord,
                        disabled: state.is_disabled(coord),
                        mana: state.mana(coord),
                        locked: state.is_locked(coord),
                    })
                    .collect::<Vec<_>>();
                cells.sort_by_key(|cell| cell.coord);
                cells
            });
            UnitStateSnapshot {
                id: *id,
                position: standing.0.pos,
                turn: turn.map(|turn| TurnStateSnapshot {
                    movement_left: turn.movement_left,
                    acted: turn.acted,
                }),
                downed,
                lattice,
            }
        })
        .collect::<Vec<_>>();
    units.sort_by_key(|unit| unit.id);
    GameplayStateSnapshot {
        screen,
        phase,
        mode,
        turn_order,
        acting,
        round,
        pending,
        queued_commands,
        presented_actions,
        units,
        outcome,
        combat,
    }
}

#[cfg(test)]
mod tests {
    use bevy::state::app::StatesPlugin;
    use bevy::MinimalPlugins;
    use hex_assets::{
        ArtPalette, ContentIndex, ElementCatalog, ElementFile, LatticeFile, LatticeLibrary,
        SpellBook, SpellFile, SubstanceFile, SubstanceTable,
    };

    use super::*;

    struct ContentFixture {
        elements: ElementCatalog,
        spell_file: SpellFile,
        lattice_file: LatticeFile,
        substances: SubstanceTable,
        content: ContentIndex,
        lattices: LatticeLibrary,
    }

    fn content_fixture() -> ContentFixture {
        let element_file: ElementFile = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/elements.ron"
        )))
        .expect("shipped elements should parse");
        let spell_file: SpellFile = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/spells.ron"
        )))
        .expect("shipped spells should parse");
        let lattice_file: LatticeFile = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/lattices.ron"
        )))
        .expect("shipped lattices should parse");
        let substance_file: SubstanceFile = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/substances.ron"
        )))
        .expect("shipped substances should parse");
        let palette: ArtPalette = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/art/palette.ron"
        )))
        .expect("shipped palette should parse");
        let elements = ElementCatalog::from_file(&element_file);
        let substances = SubstanceTable::from_file(&substance_file, &palette)
            .expect("shipped substances should resolve");
        let spells = SpellBook::from_file(&spell_file);
        let content = ContentIndex::build(&elements, &spells, &substances)
            .expect("shipped content should resolve");
        let lattices = LatticeLibrary::build(&lattice_file, &elements, &spells)
            .expect("shipped lattices should resolve");
        ContentFixture {
            elements,
            spell_file,
            lattice_file,
            substances,
            content,
            lattices,
        }
    }

    fn fixture_world(with_creator_sources: bool) -> World {
        let scenarios: ScenarioLibrary = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/scenarios.ron"
        )))
        .expect("shipped scenarios should parse");
        let maps: SandboxMapCatalog = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/sandbox_maps.ron"
        )))
        .expect("shipped Sandbox maps should parse");
        let mut world = World::new();
        world.insert_resource(scenarios);
        world.insert_resource(maps);
        world.insert_resource(CombatSettings::default());
        if with_creator_sources {
            let fixture = content_fixture();
            world.insert_resource(fixture.elements);
            world.insert_resource(fixture.spell_file);
            world.insert_resource(fixture.lattice_file);
            world.insert_resource(fixture.substances);
        }
        world
    }

    #[test]
    fn stable_fixture_request_uses_the_shared_manifest() {
        let request = DeterministicFixtureLaunchRequest::new("tempo-matrix", None)
            .expect("stable fixture exists");
        let snapshot = deterministic_fixture_launch_snapshot(&request);
        assert_eq!(snapshot.stable_id, "tempo-matrix");
        assert_eq!(snapshot.party_count, 3);
        assert_eq!(snapshot.enemy_count, 3);
        assert_eq!(snapshot.party, ["raider", "wolf", "raider"]);
        assert_eq!(snapshot.enemies, ["raider", "wolf", "raider"]);
        assert_eq!(snapshot.initial_state, None);
        assert!(!snapshot.creator_content);
        assert!(!snapshot.rules_override);
    }

    #[test]
    fn deterministic_creator_records_preserve_exact_stable_inputs() {
        let library = deterministic_creator_library().expect("test-only library should resolve");
        assert_eq!(library.next_character_id, 1004);
        assert_eq!(library.next_spell_id, 1005);
        assert_eq!(
            library
                .characters
                .iter()
                .map(|character| (character.id.0, character.name.as_str()))
                .collect::<Vec<_>>(),
            [
                (1001, "Fixture Caster"),
                (1002, "Fixture Target"),
                (1003, "Fixture Support"),
            ]
        );
        assert_eq!(
            library
                .spells
                .iter()
                .map(|spell| (spell.id.0, spell.name.as_str()))
                .collect::<Vec<_>>(),
            [
                (1001, "Fixture Scorch"),
                (1002, "Fixture Mend"),
                (1003, "Fixture Sight"),
                (1004, "Fixture Ward"),
            ]
        );
    }

    #[test]
    fn unknown_fixture_request_fails_closed() {
        assert!(DeterministicFixtureLaunchRequest::new("missing", None).is_err());
    }

    #[test]
    fn every_manifest_definition_installs_its_exact_rosters_and_placements() {
        for definition in DETERMINISTIC_FIXTURES {
            let mut world = fixture_world(true);
            let request = DeterministicFixtureLaunchRequest::new(definition.id, None)
                .expect("manifest identity should construct a request");
            let snapshot = install_deterministic_fixture_launch(&mut world, &request)
                .unwrap_or_else(|error| panic!("fixture {:?} failed: {error}", definition.id));
            let encounter = world
                .resource::<crate::scenarios::ScenarioToLoad>()
                .encounter_override
                .as_ref()
                .expect("every deterministic fixture freezes an encounter");
            assert_eq!(snapshot.party_count, definition.party.len());
            assert_eq!(snapshot.enemy_count, definition.enemies.len());
            let [party_roster, enemy_roster] = encounter.rosters.as_slice() else {
                panic!("every deterministic fixture must freeze exactly two rosters");
            };
            assert_eq!(
                party_roster.placement,
                fixture_placement(definition.party_placement)
            );
            assert_eq!(
                enemy_roster.placement,
                fixture_placement(definition.enemy_placement)
            );
            assert_eq!(
                party_roster
                    .units
                    .iter()
                    .map(|unit| unit.archetype.as_str())
                    .collect::<Vec<_>>(),
                snapshot
                    .party
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                enemy_roster
                    .units
                    .iter()
                    .map(|unit| unit.archetype.as_str())
                    .collect::<Vec<_>>(),
                snapshot
                    .enemies
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn fixture_launch_uses_the_internal_scenario_contract_and_test_only_rules_override() {
        let shipped = CombatSettings::default();
        let profile = CombatRulesProfile::tactical_two_step(&shipped);
        let request = DeterministicFixtureLaunchRequest::new("tempo-matrix", Some(profile))
            .expect("stable fixture should exist");
        let mut world = fixture_world(false);

        let snapshot = install_deterministic_fixture_launch(&mut world, &request)
            .expect("the typed fixture should resolve through shipped content");

        assert_eq!(snapshot.stable_id, "tempo-matrix");
        assert!(snapshot.rules_override);
        assert_eq!(
            world
                .resource::<crate::scenarios::ScenarioToLoad>()
                .scenario
                .name,
            "Party Trial"
        );
        assert_eq!(
            world
                .resource::<crate::scenarios::ScenarioToLoad>()
                .resolved_seed,
            None
        );
        let encounter = world
            .resource::<crate::scenarios::ScenarioToLoad>()
            .encounter_override
            .as_ref()
            .expect("fixtures always freeze exact rosters");
        let [party_roster, enemy_roster] = encounter.rosters.as_slice() else {
            panic!("the Tempo fixture must freeze exactly two rosters");
        };
        assert_eq!(
            party_roster
                .units
                .iter()
                .map(|unit| unit.archetype.as_str())
                .collect::<Vec<_>>(),
            ["raider", "wolf", "raider"]
        );
        assert_eq!(
            enemy_roster
                .units
                .iter()
                .map(|unit| unit.archetype.as_str())
                .collect::<Vec<_>>(),
            ["raider", "wolf", "raider"]
        );
        assert_eq!(world.resource::<CombatSettings>().movement_per_turn, 2);
        assert!(world.contains_resource::<FixedSettingsFreeze<CombatSettings>>());
        assert_eq!(
            world
                .resource::<DeterministicFixtureSession>()
                .shipped_rules
                .as_ref(),
            Some(&CombatSettings::default())
        );
        assert_eq!(
            gameplay_session_origin_snapshot(&world),
            Some(GameplaySessionOriginSnapshot::TestFixture(
                "tempo-matrix".to_owned()
            ))
        );
    }

    #[test]
    fn creator_fixtures_install_exact_rosters_and_test_only_content() {
        let cases = [
            (
                "creator-spell-matrix",
                vec!["custom-character-1001"],
                vec!["custom-character-1002"],
            ),
            (
                "creator-roster-matrix",
                vec!["custom-character-1001", "custom-character-1003"],
                vec!["custom-character-1002", "custom-character-1001"],
            ),
        ];
        for (stable_id, expected_party, expected_enemies) in cases {
            let mut world = fixture_world(true);
            let request = DeterministicFixtureLaunchRequest::new(stable_id, None)
                .expect("stable Creator fixture should exist");
            let snapshot = install_deterministic_fixture_launch(&mut world, &request)
                .expect("exact Creator fixture should install");
            assert_eq!(snapshot.party, expected_party);
            assert_eq!(snapshot.enemies, expected_enemies);
            assert!(snapshot.creator_content);
            assert!(world.contains_resource::<sandbox::CreatorContentOverlay>());
            assert!(world.contains_resource::<FixedSettingsFreeze<SpellFile>>());
            assert!(world.contains_resource::<FixedSettingsFreeze<LatticeFile>>());
            let encounter = world
                .resource::<crate::scenarios::ScenarioToLoad>()
                .encounter_override
                .as_ref()
                .expect("Creator fixture should freeze its exact encounter");
            let [party_roster, enemy_roster] = encounter.rosters.as_slice() else {
                panic!("every Creator fixture must freeze exactly two rosters");
            };
            assert_eq!(
                party_roster
                    .units
                    .iter()
                    .map(|unit| unit.archetype.as_str())
                    .collect::<Vec<_>>(),
                expected_party
            );
            assert_eq!(
                enemy_roster
                    .units
                    .iter()
                    .map(|unit| unit.archetype.as_str())
                    .collect::<Vec<_>>(),
                expected_enemies
            );
        }
    }

    #[test]
    fn channel_attrition_materializes_every_declared_lattice_state() {
        let fixture = content_fixture();
        let mut app = App::new();
        app.insert_resource(DeterministicFixtureSession {
            stable_id: "channel-attrition".to_owned(),
            initial_state: Some(DeterministicFixtureInitialState::ChannelAttrition),
            shipped_rules: None,
        })
        .insert_resource(fixture.content.clone())
        .insert_resource(fixture.elements.clone())
        .add_systems(Update, apply_deterministic_fixture_initial_state);
        for faction in [Faction::Player, Faction::Hostile] {
            for name in ["hedge-mage", "raider", "wolf"] {
                let archetype = fixture
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
        assert!(!app.world().contains_resource::<GameplaySetupFailure>());

        let mut units = app.world_mut().query::<(
            &Faction,
            &Archetype,
            &hex_lattice::LatticeSpec,
            &hex_lattice::LatticeState,
        )>();
        for (faction, archetype, spec, state) in units.iter(app.world()) {
            match (*faction, archetype.0.as_str()) {
                (Faction::Player | Faction::Hostile, "hedge-mage") => {
                    let fresh = fixture.lattices.get("hedge-mage").expect("fresh mage");
                    let fresh = hex_lattice::LatticeState::new(&fresh.spec, &fresh.stats);
                    assert!(state.total_gem_mana() < fresh.total_gem_mana());
                }
                (Faction::Player, "raider") => {
                    assert_eq!(state.enchantment_count(), 1);
                    assert!(state.total_locked_mana() > 0);
                }
                (Faction::Player, "wolf") => {
                    assert_eq!(
                        spec.cells()
                            .filter(|(coord, _)| state.is_disabled(*coord))
                            .count(),
                        1
                    );
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

    #[test]
    fn fixture_mutation_state_is_cleared_before_an_unrelated_session() {
        let shipped = CombatSettings::default();
        let effective = CombatSettings {
            movement_per_turn: 2,
            ..shipped.clone()
        };
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .insert_state(Screen::Gameplay)
            .insert_resource(effective)
            .insert_resource(FixedSettingsFreeze::<CombatSettings>::default())
            .insert_resource(FixedSettingsFreeze::<SpellFile>::default())
            .insert_resource(FixedSettingsFreeze::<LatticeFile>::default())
            .insert_resource(DeterministicFixtureSession {
                stable_id: "channel-attrition".to_owned(),
                initial_state: Some(DeterministicFixtureInitialState::ChannelAttrition),
                shipped_rules: Some(shipped.clone()),
            })
            .insert_resource(sandbox::GameplaySessionOrigin::TestFixture(
                "channel-attrition".to_owned(),
            ));
        plugin(&mut app);
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Title);
        app.update();
        assert!(!app
            .world()
            .contains_resource::<DeterministicFixtureSession>());
        assert!(!app
            .world()
            .contains_resource::<sandbox::GameplaySessionOrigin>());
        assert!(!app
            .world()
            .contains_resource::<FixedSettingsFreeze<CombatSettings>>());
        assert!(!app
            .world()
            .contains_resource::<FixedSettingsFreeze<SpellFile>>());
        assert!(!app
            .world()
            .contains_resource::<FixedSettingsFreeze<LatticeFile>>());
        assert_eq!(app.world().resource::<CombatSettings>(), &shipped);
    }

    #[test]
    fn fixture_retry_loading_preserves_provenance_mutation_and_frozen_rules() {
        let shipped = CombatSettings::default();
        let effective = CombatSettings {
            movement_per_turn: 2,
            ..shipped.clone()
        };
        let session = DeterministicFixtureSession {
            stable_id: "channel-attrition".to_owned(),
            initial_state: Some(DeterministicFixtureInitialState::ChannelAttrition),
            shipped_rules: Some(shipped),
        };
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .insert_state(Screen::Gameplay)
            .insert_resource(effective.clone())
            .insert_resource(FixedSettingsFreeze::<CombatSettings>::default())
            .insert_resource(session.clone())
            .insert_resource(sandbox::GameplaySessionOrigin::TestFixture(
                "channel-attrition".to_owned(),
            ));
        plugin(&mut app);
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Loading);
        app.update();
        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Loading
        );
        assert_eq!(
            app.world().resource::<DeterministicFixtureSession>(),
            &session
        );
        assert_eq!(app.world().resource::<CombatSettings>(), &effective);
        assert!(app
            .world()
            .contains_resource::<FixedSettingsFreeze<CombatSettings>>());
        assert_eq!(
            gameplay_session_origin_snapshot(app.world()),
            Some(GameplaySessionOriginSnapshot::TestFixture(
                "channel-attrition".to_owned()
            ))
        );
    }
}
