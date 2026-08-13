//! Three atomic, build-bound Campaign save slots.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;
use std::time::Duration;

use bevy::prelude::*;
use bevy::time::Real;
use hex_assets::{
    AcceptedContentRevision, ContentIndex, ElementCatalog, FormationCatalog, LatticeLibrary,
    Scenario, ScenarioLibrary, SubstanceTable,
};
use hex_combat::{CombatSystems, EncounterResolution};
use hex_core::{
    Busy, CommandQueue, GameplayPhase, GameplaySetup, GameplaySetupFailure, Headroom, HexSpan,
    HexTile, InputAction, InputBindings, Mode, PartyFormation, Pause, PendingDecision,
    ResolvedMapSeed, Screen, SimSeeds, SimulationRole, SubstanceId, TilePos, TraversalBlockers,
    TraversalProfile, UnitId,
};
use hex_gameplay_model::CampaignSlotId;
use hex_lattice::{CellKind, LatticeState};
use hex_map::{
    CampaignWorldRestoreOutcomeV2, CampaignWorldRestoreResultV2, CurrentWorldSnapshotV1,
    MapSettings, PendingCampaignWorldSnapshotV2, TerrainSettings,
};
use hex_multiplayer::{
    BoundedText, CampaignSaveRefusalV2, CampaignSaveStateV2, CampaignSaveStatusV2,
    CampaignUnitCheckpointV2, ContentFingerprint, HostCampaignCheckpointV2, RulesManifestV1,
    SessionAdmissionAuthority, CAMPAIGN_CHECKPOINT_VERSION_V2, MAX_IDENTITY_BYTES,
};
use hex_ui::{
    CampaignPartyMemberView, CampaignSlotStatusView, CampaignSlotView, MainMenuIntent,
    SandboxLatticeCellKind, SandboxLatticeCellView, UiIntent, UiSystems,
};
use hex_units::{
    Archetype as UnitArchetype, Body, Downed, Faction, Footing, MovingTo, Selected, StandsOn,
};
use serde::{Deserialize, Serialize};

use crate::campaign_authority::{
    export_campaign_gameplay_checkpoint, CampaignGameplayCheckpointV2,
    CampaignGameplayRestoreOutcomeV2, CampaignGameplayRestoreResultV2,
    PendingCampaignGameplayCheckpointV2,
};
use crate::scenarios::{ActiveScenario, ScenarioToLoad};
use crate::screens::sandbox::GameplaySessionOrigin;
use crate::storage::{read, write_atomic, StoragePaths};

const LEGACY_RESUME_VERSION: u32 = 1;
const CAMPAIGNS_VERSION_V1: u32 = 1;
const CAMPAIGNS_VERSION_V2: u32 = 2;

/// Exact digest translation table for resumes written by PR #175's `dev` head.
///
/// The cutover changed comment-only text in three digest-bound assets:
/// `scenarios.ron`, `anchored-skirmish.ron`, and `procedural-hills.ron`. Their parsed
/// meaning is unchanged. A PR #175 digest is accepted only while the current digest
/// still equals the corresponding cutover digest, so any later semantic asset change
/// keeps invalidating the legacy resume as intended.
const LEGACY_RESUME_DIGESTS: &[(&str, u64, u64)] = &[
    ("The Crossing", 0x5FFB_DCD6_C8CF_30CC, 0xAC2F_13D5_7865_2646),
    (
        "Procedural Hills",
        0x8F25_010C_85CF_CAF3,
        0x6308_39FC_0537_3D71,
    ),
    (
        "Rolling Hills",
        0x2DE9_1507_D357_ABF4,
        0xED15_7C75_33CA_75BE,
    ),
    ("Frozen Hills", 0xE6DD_2CCD_12D1_45E5, 0x3582_3F3E_7518_D437),
    (
        "Volcanic Hills",
        0xEB7E_01A8_AAA2_286F,
        0x3A92_0DAE_E458_945D,
    ),
    ("Sky Islands", 0x8071_6B2B_0888_E4FA, 0x57C9_6540_6E72_6F20),
    ("Mountains", 0x3DDE_18E7_4C6A_569D, 0x86BA_BFE3_09B0_FA7F),
    ("Caves", 0x9BCD_C2F9_D17D_D72A, 0x3D97_6234_8A97_BAD0),
    ("Waterfall", 0x5FD0_1EF4_38CE_8941, 0xE3A8_F74E_C1F3_BB33),
    ("Forest", 0xB4F3_CBD7_781A_03E7, 0x4339_0018_80EB_5865),
    ("Deep Forest", 0xE738_EC86_5931_590B, 0x01B6_E318_48E4_C3F9),
    ("Prairie", 0x61EF_B225_B791_AC6E, 0xA321_BC25_8F8D_A414),
    ("Fort", 0x1C14_BC36_4158_CE43, 0x3DF1_6E83_BDDF_44C1),
    (
        "Seven Regions",
        0xA5E5_86ED_155D_1FCF,
        0x80BB_85FD_657F_507D,
    ),
    ("Two Rings", 0xE4C0_B13F_0B78_00BD, 0x7EDA_10A6_7B46_E45F),
    ("Party Trial", 0xC8EA_6229_346D_CF96, 0xAA13_0315_396C_E50C),
    ("Ability Lab", 0x26E9_C8E6_07F1_C52E, 0x3829_4F64_D4E7_D6D4),
    (
        "Raider Mirror",
        0x4D4B_EBCF_B5C8_AB54,
        0x71B4_8117_1E3B_905E,
    ),
];

/// Build-bound inputs whose semantic changes invalidate every Campaign slot.
///
/// Keep the asset paths beside their compiled contents: tests use the paths to prove
/// that every dependency named by `scenarios.ron` participates in the digest.
const SHIPPED_CAMPAIGN_INPUTS: &[(&str, &str)] = &[
    (
        "config/scenarios.ron",
        include_str!("../../../assets/config/scenarios.ron"),
    ),
    (
        "art/object_catalog.ron",
        include_str!("../../../assets/art/object_catalog.ron"),
    ),
    (
        "art/objects/plant/old-growth.ron",
        include_str!("../../../assets/art/objects/plant/old-growth.ron"),
    ),
    (
        "art/objects/plant/small-broadleaf.ron",
        include_str!("../../../assets/art/objects/plant/small-broadleaf.ron"),
    ),
    (
        "art/objects/plant/snowy-old-growth.ron",
        include_str!("../../../assets/art/objects/plant/snowy-old-growth.ron"),
    ),
    (
        "art/objects/plant/snowy-small-broadleaf.ron",
        include_str!("../../../assets/art/objects/plant/snowy-small-broadleaf.ron"),
    ),
    (
        "art/objects/plant/snowy-tall-narrow.ron",
        include_str!("../../../assets/art/objects/plant/snowy-tall-narrow.ron"),
    ),
    (
        "art/objects/plant/tall-narrow.ron",
        include_str!("../../../assets/art/objects/plant/tall-narrow.ron"),
    ),
    (
        "art/objects/prop/cave-lichen.ron",
        include_str!("../../../assets/art/objects/prop/cave-lichen.ron"),
    ),
    (
        "art/objects/prop/cave-moss.ron",
        include_str!("../../../assets/art/objects/prop/cave-moss.ron"),
    ),
    (
        "art/objects/prop/crystal-branched.ron",
        include_str!("../../../assets/art/objects/prop/crystal-branched.ron"),
    ),
    (
        "art/objects/prop/crystal-cathedral-heart.ron",
        include_str!("../../../assets/art/objects/prop/crystal-cathedral-heart.ron"),
    ),
    (
        "art/objects/prop/crystal-low-cluster.ron",
        include_str!("../../../assets/art/objects/prop/crystal-low-cluster.ron"),
    ),
    (
        "art/objects/prop/crystal-spire.ron",
        include_str!("../../../assets/art/objects/prop/crystal-spire.ron"),
    ),
    (
        "art/objects/prop/grass-tuft.ron",
        include_str!("../../../assets/art/objects/prop/grass-tuft.ron"),
    ),
    (
        "art/objects/prop/snowy-grass-tuft.ron",
        include_str!("../../../assets/art/objects/prop/snowy-grass-tuft.ron"),
    ),
    (
        "config/formations.ron",
        include_str!("../../../assets/config/formations.ron"),
    ),
    (
        "config/lattices.ron",
        include_str!("../../../assets/config/lattices.ron"),
    ),
    (
        "config/elements.ron",
        include_str!("../../../assets/config/elements.ron"),
    ),
    (
        "config/spells.ron",
        include_str!("../../../assets/config/spells.ron"),
    ),
    (
        "config/ai_profiles.ron",
        include_str!("../../../assets/config/ai_profiles.ron"),
    ),
    (
        "config/combat.ron",
        include_str!("../../../assets/config/combat.ron"),
    ),
    (
        "config/perception.ron",
        include_str!("../../../assets/config/perception.ron"),
    ),
    (
        "config/player.ron",
        include_str!("../../../assets/config/player.ron"),
    ),
    (
        "config/substances.ron",
        include_str!("../../../assets/config/substances.ron"),
    ),
    (
        "config/terrain_damage.ron",
        include_str!("../../../assets/config/terrain_damage.ron"),
    ),
    (
        "config/lighting.ron",
        include_str!("../../../assets/config/lighting.ron"),
    ),
    (
        "config/lighting/overcast.ron",
        include_str!("../../../assets/config/lighting/overcast.ron"),
    ),
    (
        "config/world.ron",
        include_str!("../../../assets/config/world.ron"),
    ),
    (
        "config/worlds/flat-combat.ron",
        include_str!("../../../assets/config/worlds/flat-combat.ron"),
    ),
    (
        "config/worlds/procedural-caves.ron",
        include_str!("../../../assets/config/worlds/procedural-caves.ron"),
    ),
    (
        "config/worlds/procedural-crystal-ascent.ron",
        include_str!("../../../assets/config/worlds/procedural-crystal-ascent.ron"),
    ),
    (
        "config/worlds/procedural-deep-forest.ron",
        include_str!("../../../assets/config/worlds/procedural-deep-forest.ron"),
    ),
    (
        "config/worlds/procedural-forest.ron",
        include_str!("../../../assets/config/worlds/procedural-forest.ron"),
    ),
    (
        "config/worlds/procedural-fort.ron",
        include_str!("../../../assets/config/worlds/procedural-fort.ron"),
    ),
    (
        "config/worlds/procedural-frozen.ron",
        include_str!("../../../assets/config/worlds/procedural-frozen.ron"),
    ),
    (
        "config/worlds/procedural-hills.ron",
        include_str!("../../../assets/config/worlds/procedural-hills.ron"),
    ),
    (
        "config/worlds/procedural-mountains.ron",
        include_str!("../../../assets/config/worlds/procedural-mountains.ron"),
    ),
    (
        "config/worlds/procedural-mountain-range.ron",
        include_str!("../../../assets/config/worlds/procedural-mountain-range.ron"),
    ),
    (
        "config/worlds/procedural-prairie.ron",
        include_str!("../../../assets/config/worlds/procedural-prairie.ron"),
    ),
    (
        "config/worlds/procedural-ring7.ron",
        include_str!("../../../assets/config/worlds/procedural-ring7.ron"),
    ),
    (
        "config/worlds/procedural-two-rings.ron",
        include_str!("../../../assets/config/worlds/procedural-two-rings.ron"),
    ),
    (
        "config/worlds/procedural-sky-islands.ron",
        include_str!("../../../assets/config/worlds/procedural-sky-islands.ron"),
    ),
    (
        "config/worlds/procedural-volcanic.ron",
        include_str!("../../../assets/config/worlds/procedural-volcanic.ron"),
    ),
    (
        "config/worlds/procedural-waterfall.ron",
        include_str!("../../../assets/config/worlds/procedural-waterfall.ron"),
    ),
    (
        "config/worlds/rolling-hills.ron",
        include_str!("../../../assets/config/worlds/rolling-hills.ron"),
    ),
    (
        "config/encounters/ability-lab.ron",
        include_str!("../../../assets/config/encounters/ability-lab.ron"),
    ),
    (
        "config/encounters/anchored-skirmish.ron",
        include_str!("../../../assets/config/encounters/anchored-skirmish.ron"),
    ),
    (
        "config/encounters/bridge-crossing.ron",
        include_str!("../../../assets/config/encounters/bridge-crossing.ron"),
    ),
    (
        "config/encounters/crystal-ascent-showcase.ron",
        include_str!("../../../assets/config/encounters/crystal-ascent-showcase.ron"),
    ),
    (
        "config/encounters/open-ground.ron",
        include_str!("../../../assets/config/encounters/open-ground.ron"),
    ),
    (
        "config/encounters/party-trial.ron",
        include_str!("../../../assets/config/encounters/party-trial.ron"),
    ),
    (
        "config/encounters/raider-mirror.ron",
        include_str!("../../../assets/config/encounters/raider-mirror.ron"),
    ),
];

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
struct LegacyResumeFile {
    format_version: u32,
    build_version: String,
    scenario_name: String,
    scenario_digest: u64,
    resolved_seed: Option<u64>,
    generator_version: Option<u32>,
    formation: PartyFormation,
    selected: Option<UnitId>,
    units: Vec<LegacyUnitResume>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
struct LegacyUnitResume {
    id: UnitId,
    faction: Faction,
    position: TilePos,
    lattice: Option<LatticeState>,
    downed: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
struct CampaignsFile {
    format_version: u32,
    /// Persisted so incompatible legacy slot-1 data cannot silently become empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    legacy_slot_one_refusal: Option<String>,
    /// Strict legacy/V1 records. A successful V2 write clears only its selected slot.
    slots: [Option<CampaignSave>; 3],
    /// Complete authority checkpoints introduced by document version 2.
    #[serde(default, skip_serializing_if = "campaign_v2_slots_are_empty")]
    v2_slots: [Option<CampaignSaveV2>; 3],
}

impl Default for CampaignsFile {
    fn default() -> Self {
        Self {
            format_version: CAMPAIGNS_VERSION_V2,
            legacy_slot_one_refusal: None,
            slots: std::array::from_fn(|_| None),
            v2_slots: std::array::from_fn(|_| None),
        }
    }
}

fn campaign_v2_slots_are_empty(slots: &[Option<CampaignSaveV2>; 3]) -> bool {
    slots.iter().all(Option::is_none)
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
struct CampaignSave {
    slot: CampaignSlotId,
    build_version: String,
    scenario_name: String,
    scenario_digest: u64,
    /// Accepted semantic content graph used to create this save. Legacy resumes
    /// predate the graph fingerprint and retain `None` compatibility.
    #[serde(default)]
    content_revision: Option<u64>,
    resolved_seed: Option<u64>,
    generator_version: Option<u32>,
    formation: PartyFormation,
    selected: Option<UnitId>,
    active_play_millis: u64,
    units: Vec<CampaignUnitSave>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CampaignSaveV2 {
    slot: CampaignSlotId,
    checkpoint: HostCampaignCheckpointV2,
}

#[derive(Debug, Clone)]
enum CampaignRecord {
    V1(CampaignSave),
    V2(Box<CampaignSaveV2>),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
struct CampaignUnitSave {
    id: UnitId,
    faction: Faction,
    position: TilePos,
    #[serde(default)]
    archetype: String,
    lattice: Option<LatticeState>,
    downed: bool,
    display_name: String,
}

/// Parsed Campaign persistence, including refusals that must remain visible.
#[derive(Resource, Debug, Clone)]
pub(crate) struct CampaignStore {
    file: Option<CampaignsFile>,
    unreadable: Option<String>,
    /// Sticky failures discovered while restoring a specific saved world.
    runtime_invalid: [Option<String>; 3],
    /// Recomputed refusals against the currently accepted shipped catalogs.
    catalog_invalid: [Option<String>; 3],
}

impl Default for CampaignStore {
    fn default() -> Self {
        Self {
            file: Some(CampaignsFile::default()),
            unreadable: None,
            runtime_invalid: std::array::from_fn(|_| None),
            catalog_invalid: std::array::from_fn(|_| None),
        }
    }
}

/// Latest Campaign save feedback shown while paused.
#[derive(Resource, Debug, Default, Clone)]
pub(crate) struct CampaignSaveNotice(pub(crate) Option<String>);

/// Latest typed Campaign save projection consumed by multiplayer UI.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CampaignSaveStatusProjection {
    pub(crate) operation_id: u64,
    pub(crate) state: Option<CampaignSaveStateV2>,
}

#[derive(Resource, Debug, Default)]
struct CampaignSaveRuntime {
    next_operation_id: u64,
    pending: Option<PendingCampaignWriteV2>,
}

#[derive(Debug)]
struct PendingCampaignWriteV2 {
    operation_id: u64,
    slot: CampaignSlotId,
    save: CampaignSaveV2,
}

/// Slot identity and elapsed time for the currently running Campaign session.
#[derive(Resource, Debug, Clone)]
pub(crate) struct ActiveCampaign {
    slot: CampaignSlotId,
    content_revision: u64,
    persisted_active_play_millis: u64,
    session_active_play: Duration,
    /// Whether the real-time interval ending at the next update began in active play.
    count_previous_interval: bool,
}

impl ActiveCampaign {
    fn new(slot: CampaignSlotId, content_revision: u64, persisted_active_play_millis: u64) -> Self {
        Self {
            slot,
            content_revision,
            persisted_active_play_millis,
            session_active_play: Duration::ZERO,
            count_previous_interval: false,
        }
    }

    fn active_play_millis(&self) -> u64 {
        let session = u64::try_from(self.session_active_play.as_millis()).unwrap_or(u64::MAX);
        self.persisted_active_play_millis.saturating_add(session)
    }

    fn mark_persisted(&mut self) {
        self.persisted_active_play_millis = self.active_play_millis();
        self.session_active_play = Duration::ZERO;
    }
}

/// Exact saved world applied only after its scenario has spawned.
#[derive(Resource, Debug, Clone)]
pub(crate) struct PendingCampaign(CampaignSave);

/// A Direct/LAN host request selected by the multiplayer shell.
///
/// The endpoint remains in `PendingDirectHostSetup`; this save-owned request contains
/// only the durable slot identity and therefore cannot leak transport state into a
/// checkpoint.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CampaignMultiplayerHostRequest {
    pub(crate) slot: CampaignSlotId,
}

/// Stable refusal category for preparing a host-owned Campaign lobby.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CampaignMultiplayerHostRefusal {
    ContentUnavailable,
    IncompatibleCheckpoint,
    RestoreFailed,
    IncompleteCheckpoint,
}

/// Renderer-safe progress/refusal projection for L4.
#[derive(Resource, Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct CampaignMultiplayerHostStatus {
    pub(crate) slot: Option<CampaignSlotId>,
    pub(crate) preparing: bool,
    pub(crate) refusal: Option<CampaignMultiplayerHostRefusal>,
    pub(crate) notice: Option<String>,
}

#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
struct PendingCampaignHostBootstrap {
    slot: CampaignSlotId,
    requires_v2_restore: bool,
}

pub(crate) fn plugin(app: &mut App) {
    crate::campaign_authority::plugin(app);
    app.init_resource::<StoragePaths>()
        .init_resource::<CampaignStore>()
        .init_resource::<CampaignSaveNotice>()
        .init_resource::<CampaignSaveStatusProjection>()
        .init_resource::<CampaignSaveRuntime>()
        .init_resource::<CampaignMultiplayerHostStatus>()
        .add_systems(Startup, load_campaigns)
        .add_systems(
            Update,
            validate_campaign_catalog.run_if(in_state(Screen::Title)),
        )
        .add_systems(
            Update,
            handle_campaign_intents
                .after(UiSystems::EmitIntents)
                .run_if(in_state(Screen::Title)),
        )
        .add_systems(
            Update,
            begin_campaign_multiplayer_host.run_if(in_state(Screen::Multiplayer)),
        )
        .add_systems(
            Update,
            (
                accumulate_active_play_time,
                commit_pending_campaign_save,
                complete_campaign_multiplayer_bootstrap,
                save_exploration,
                capture_remote_campaign_save_status,
            )
                .chain()
                .after(CombatSystems::Resolve),
        )
        .add_systems(
            OnEnter(Screen::Gameplay),
            restore_pending_campaign.in_set(GameplaySetup::Restore),
        )
        .add_systems(OnEnter(Screen::Loading), clear_campaign_save_notice)
        .add_systems(OnEnter(Screen::Title), clear_abandoned_campaign_session)
        .add_systems(OnEnter(Screen::Sandbox), clear_abandoned_campaign_session);
}

fn clear_campaign_save_notice(
    mut notice: ResMut<CampaignSaveNotice>,
    status: Option<ResMut<CampaignSaveStatusProjection>>,
) {
    notice.0 = None;
    if let Some(mut status) = status {
        status.state = None;
    }
}

fn clear_abandoned_campaign_session(
    mut commands: Commands,
    mut notice: ResMut<CampaignSaveNotice>,
    runtime: Option<ResMut<CampaignSaveRuntime>>,
    status: Option<ResMut<CampaignSaveStatusProjection>>,
    origin: Option<Res<GameplaySessionOrigin>>,
) {
    commands.remove_resource::<PendingCampaign>();
    commands.remove_resource::<PendingCampaignWorldSnapshotV2>();
    commands.remove_resource::<PendingCampaignGameplayCheckpointV2>();
    commands.remove_resource::<CampaignMultiplayerHostRequest>();
    commands.remove_resource::<PendingCampaignHostBootstrap>();
    commands.remove_resource::<ActiveCampaign>();
    if matches!(origin.as_deref(), Some(GameplaySessionOrigin::Campaign(_))) {
        commands.remove_resource::<GameplaySessionOrigin>();
    }
    if let Some(mut runtime) = runtime {
        runtime.pending = None;
    }
    if let Some(mut status) = status {
        status.state = None;
    }
    notice.0 = None;
}

fn load_campaigns(paths: Res<StoragePaths>, mut store: ResMut<CampaignStore>) {
    *store = match read(&paths.campaigns) {
        Ok(text) => decode_campaigns(&text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => migrate_legacy(&paths),
        Err(error) => CampaignStore {
            file: None,
            unreadable: Some(format!("Campaign data could not be read: {error}")),
            runtime_invalid: std::array::from_fn(|_| None),
            catalog_invalid: std::array::from_fn(|_| None),
        },
    };
}

fn decode_campaigns(text: &str) -> CampaignStore {
    match ron::from_str::<CampaignsFile>(text) {
        Ok(file) if campaigns_file_refusal(&file).is_none() => CampaignStore {
            file: Some(file),
            unreadable: None,
            runtime_invalid: std::array::from_fn(|_| None),
            catalog_invalid: std::array::from_fn(|_| None),
        },
        Ok(file) => CampaignStore {
            file: None,
            unreadable: campaigns_file_refusal(&file),
            runtime_invalid: std::array::from_fn(|_| None),
            catalog_invalid: std::array::from_fn(|_| None),
        },
        Err(error) => CampaignStore {
            file: None,
            unreadable: Some(format!("Campaign data could not be parsed: {error}")),
            runtime_invalid: std::array::from_fn(|_| None),
            catalog_invalid: std::array::from_fn(|_| None),
        },
    }
}

fn campaigns_file_refusal(file: &CampaignsFile) -> Option<String> {
    if !matches!(
        file.format_version,
        CAMPAIGNS_VERSION_V1 | CAMPAIGNS_VERSION_V2
    ) {
        return Some(format!(
            "Campaign format {} is incompatible with {}.",
            file.format_version, CAMPAIGNS_VERSION_V2
        ));
    }
    if file.format_version == CAMPAIGNS_VERSION_V1 && file.v2_slots.iter().any(Option::is_some) {
        return Some("Campaign format 1 contains an impossible V2 checkpoint.".to_owned());
    }
    if file
        .slots
        .iter()
        .zip(&file.v2_slots)
        .any(|(v1, v2)| v1.is_some() && v2.is_some())
    {
        return Some("Campaign data contains two records for one slot.".to_owned());
    }
    None
}

fn migrate_legacy(paths: &StoragePaths) -> CampaignStore {
    let migration = match read(&paths.resume) {
        Ok(text) => ron::from_str::<LegacyResumeFile>(&text)
            .map_err(|error| format!("Legacy resume data could not be parsed: {error}"))
            .and_then(|legacy| {
                validate_legacy_resume(&legacy)?;
                Ok(Some(CampaignSave::from_legacy(legacy)))
            }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Legacy resume data could not be read: {error}")),
    };

    let mut file = CampaignsFile::default();
    match migration {
        Ok(Some(save)) => {
            if let Some(slot_one) = file.slots.first_mut() {
                *slot_one = Some(save);
            }
        }
        Ok(None) => {}
        Err(reason) => file.legacy_slot_one_refusal = Some(reason),
    }
    let migration_refusal = match encode_campaigns(&file).and_then(|serialized| {
        write_atomic(&paths.campaigns, &serialized)
            .map_err(|error| format!("Legacy resume migration could not be saved: {error}"))
    }) {
        Ok(()) => None,
        Err(reason) => {
            warn!("{reason}");
            Some(reason)
        }
    };
    CampaignStore {
        file: Some(file),
        unreadable: migration_refusal,
        runtime_invalid: std::array::from_fn(|_| None),
        catalog_invalid: std::array::from_fn(|_| None),
    }
}

impl CampaignSave {
    fn from_legacy(legacy: LegacyResumeFile) -> Self {
        Self {
            slot: CampaignSlotId::ALL[0],
            build_version: legacy.build_version,
            scenario_name: legacy.scenario_name,
            scenario_digest: legacy.scenario_digest,
            content_revision: None,
            resolved_seed: legacy.resolved_seed,
            generator_version: legacy.generator_version,
            formation: legacy.formation,
            selected: legacy.selected,
            active_play_millis: 0,
            units: legacy
                .units
                .into_iter()
                .map(|unit| CampaignUnitSave {
                    id: unit.id,
                    faction: unit.faction,
                    position: unit.position,
                    archetype: String::new(),
                    lattice: unit.lattice,
                    downed: unit.downed,
                    display_name: format!("Unit {}", unit.id.0),
                })
                .collect(),
        }
    }
}

impl CampaignStore {
    /// Projects exactly three slot cards in canonical numeric order.
    pub(crate) fn slot_views(
        &self,
        lattices: Option<&LatticeLibrary>,
        elements: Option<&ElementCatalog>,
    ) -> Vec<CampaignSlotView> {
        CampaignSlotId::ALL
            .into_iter()
            .map(|slot| CampaignSlotView {
                slot,
                status: match self.record(slot) {
                    Ok(None) => CampaignSlotStatusView::Empty,
                    Ok(Some(record)) => campaign_slot_status(&record, lattices, elements),
                    Err(reason) => CampaignSlotStatusView::Invalid { reason },
                },
            })
            .collect()
    }

    fn record(&self, slot: CampaignSlotId) -> Result<Option<CampaignRecord>, String> {
        if let Some(reason) = &self.unreadable {
            return Err(reason.clone());
        }
        let Some(file) = &self.file else {
            return Err("Campaign data is unavailable.".to_owned());
        };
        if slot.index() == 0 {
            if let Some(reason) = &file.legacy_slot_one_refusal {
                return Err(reason.clone());
            }
        }
        if let Some(reason) = self
            .runtime_invalid
            .get(slot.index())
            .and_then(Option::as_ref)
        {
            return Err(reason.clone());
        }
        if let Some(reason) = self
            .catalog_invalid
            .get(slot.index())
            .and_then(Option::as_ref)
        {
            return Err(reason.clone());
        }
        if let Some(save) = file.v2_slots.get(slot.index()).and_then(Option::as_ref) {
            validate_campaign_save_v2(save, slot)?;
            return Ok(Some(CampaignRecord::V2(Box::new(save.clone()))));
        }
        if let Some(save) = file.slots.get(slot.index()).and_then(Option::as_ref) {
            validate_campaign_save(save, slot)?;
            return Ok(Some(CampaignRecord::V1(save.clone())));
        }
        Ok(None)
    }

    fn available_record(&self, slot: CampaignSlotId) -> Option<CampaignRecord> {
        self.record(slot).ok().flatten()
    }

    fn is_empty(&self, slot: CampaignSlotId) -> bool {
        matches!(self.record(slot), Ok(None))
    }

    fn mark_invalid(&mut self, slot: CampaignSlotId, reason: String) {
        if let Some(refusal) = self.runtime_invalid.get_mut(slot.index()) {
            *refusal = Some(reason);
        }
    }

    fn mark_catalog_invalid(&mut self, slot: CampaignSlotId, reason: String) {
        if let Some(refusal) = self.catalog_invalid.get_mut(slot.index()) {
            *refusal = Some(reason);
        }
    }

    #[cfg(test)]
    fn write_slot(
        &mut self,
        paths: &StoragePaths,
        slot: CampaignSlotId,
        save: CampaignSave,
    ) -> Result<(), String> {
        if let Err(reason) = self.record(slot) {
            return Err(format!(
                "Campaign slot {} is invalid and was left untouched: {reason}",
                slot.number()
            ));
        }
        validate_campaign_save(&save, slot)?;
        let Some(file) = &self.file else {
            return Err("Campaign data is unavailable; it was left untouched.".to_owned());
        };
        let mut next = file.clone();
        next.format_version = CAMPAIGNS_VERSION_V2;
        let Some(target) = next.slots.get_mut(slot.index()) else {
            return Err(format!(
                "Campaign slot {} is outside the fixed slot document.",
                slot.number()
            ));
        };
        *target = Some(save);
        if let Some(target) = next.v2_slots.get_mut(slot.index()) {
            *target = None;
        }
        let serialized = encode_campaigns(&next)?;
        write_atomic(&paths.campaigns, &serialized)
            .map_err(|error| format!("Campaign could not be saved: {error}"))?;
        self.file = Some(next);
        if let Some(refusal) = self.runtime_invalid.get_mut(slot.index()) {
            *refusal = None;
        }
        if let Some(refusal) = self.catalog_invalid.get_mut(slot.index()) {
            *refusal = None;
        }
        Ok(())
    }

    fn write_v2_slot(
        &mut self,
        paths: &StoragePaths,
        slot: CampaignSlotId,
        save: CampaignSaveV2,
    ) -> Result<(), String> {
        if let Err(reason) = self.record(slot) {
            return Err(format!(
                "Campaign slot {} is invalid and was left untouched: {reason}",
                slot.number()
            ));
        }
        validate_campaign_save_v2(&save, slot)?;
        let Some(file) = &self.file else {
            return Err("Campaign data is unavailable; it was left untouched.".to_owned());
        };
        let mut next = file.clone();
        next.format_version = CAMPAIGNS_VERSION_V2;
        let Some(target) = next.v2_slots.get_mut(slot.index()) else {
            return Err(format!(
                "Campaign slot {} is outside the fixed slot document.",
                slot.number()
            ));
        };
        *target = Some(save);
        if let Some(target) = next.slots.get_mut(slot.index()) {
            *target = None;
        }
        let serialized = encode_campaigns(&next)?;
        write_atomic(&paths.campaigns, &serialized)
            .map_err(|error| format!("Campaign could not be saved: {error}"))?;
        self.file = Some(next);
        if let Some(refusal) = self.runtime_invalid.get_mut(slot.index()) {
            *refusal = None;
        }
        if let Some(refusal) = self.catalog_invalid.get_mut(slot.index()) {
            *refusal = None;
        }
        Ok(())
    }

    #[cfg(test)]
    fn slot(&self, slot: CampaignSlotId) -> Result<Option<&CampaignSave>, String> {
        if let Some(reason) = &self.unreadable {
            return Err(reason.clone());
        }
        let Some(file) = &self.file else {
            return Err("Campaign data is unavailable.".to_owned());
        };
        if slot.index() == 0 {
            if let Some(reason) = &file.legacy_slot_one_refusal {
                return Err(reason.clone());
            }
        }
        if let Some(reason) = self
            .runtime_invalid
            .get(slot.index())
            .and_then(Option::as_ref)
            .or_else(|| {
                self.catalog_invalid
                    .get(slot.index())
                    .and_then(Option::as_ref)
            })
        {
            return Err(reason.clone());
        }
        let save = file.slots.get(slot.index()).and_then(Option::as_ref);
        if let Some(save) = save {
            validate_campaign_save(save, slot)?;
        }
        Ok(save)
    }

    #[cfg(test)]
    fn available(&self, slot: CampaignSlotId) -> Option<CampaignSave> {
        self.slot(slot).ok().flatten().cloned()
    }
}

fn campaign_slot_status(
    record: &CampaignRecord,
    lattices: Option<&LatticeLibrary>,
    elements: Option<&ElementCatalog>,
) -> CampaignSlotStatusView {
    let (party, active_play_millis) = match record {
        CampaignRecord::V1(save) => (
            save.units
                .iter()
                .filter(|unit| unit.faction == Faction::Player)
                .map(|unit| CampaignPartyMemberView {
                    name: campaign_party_member_name(unit, lattices),
                    lattice: unit.lattice.as_ref().map_or_else(
                        || "No lattice".to_owned(),
                        |lattice| format!("{} mana", lattice.total_gem_mana()),
                    ),
                    cells: campaign_lattice_cells(unit, lattices, elements),
                })
                .collect(),
            save.active_play_millis,
        ),
        CampaignRecord::V2(save) => (
            save.checkpoint
                .units
                .as_slice()
                .iter()
                .filter(|unit| unit.faction == Faction::Player)
                .map(|unit| CampaignPartyMemberView {
                    name: unit.display_name.as_str().to_owned(),
                    lattice: unit.lattice.as_ref().map_or_else(
                        || "No lattice".to_owned(),
                        |lattice| format!("{} mana", lattice.total_gem_mana()),
                    ),
                    cells: campaign_v2_lattice_cells(unit, lattices, elements),
                })
                .collect(),
            save.checkpoint.active_play_millis,
        ),
    };
    CampaignSlotStatusView::Available {
        party,
        active_time: format_active_time(active_play_millis),
    }
}

fn campaign_lattice_cells(
    unit: &CampaignUnitSave,
    lattices: Option<&LatticeLibrary>,
    elements: Option<&ElementCatalog>,
) -> Vec<SandboxLatticeCellView> {
    let (Some(state), Some(lattices), Some(elements)) = (unit.lattice.as_ref(), lattices, elements)
    else {
        return Vec::new();
    };
    let archetype = if unit.archetype.is_empty() {
        infer_legacy_archetype(state, lattices).map(|(_, archetype)| archetype)
    } else {
        lattices.get(&unit.archetype)
    };
    let Some(archetype) = archetype else {
        return Vec::new();
    };
    project_campaign_lattice_cells(archetype, elements)
}

fn campaign_v2_lattice_cells(
    unit: &CampaignUnitCheckpointV2,
    lattices: Option<&LatticeLibrary>,
    elements: Option<&ElementCatalog>,
) -> Vec<SandboxLatticeCellView> {
    let (Some(_state), Some(lattices), Some(elements)) =
        (unit.lattice.as_ref(), lattices, elements)
    else {
        return Vec::new();
    };
    let Some(archetype) = lattices.get(unit.archetype_identity.as_str()) else {
        return Vec::new();
    };
    project_campaign_lattice_cells(archetype, elements)
}

fn project_campaign_lattice_cells(
    archetype: &hex_assets::Archetype,
    elements: &ElementCatalog,
) -> Vec<SandboxLatticeCellView> {
    archetype
        .spec
        .cells()
        .map(|(coord, kind)| {
            let (label, kind) = match kind {
                CellKind::Gem { element } => (
                    compact_lattice_label(elements.name(element)),
                    SandboxLatticeCellKind::Gem,
                ),
                CellKind::Fusion { output } => (
                    compact_lattice_label(elements.name(output)),
                    SandboxLatticeCellKind::Fusion,
                ),
                CellKind::Spell { .. } => ("S".to_owned(), SandboxLatticeCellKind::Spell),
                CellKind::Blank => ("·".to_owned(), SandboxLatticeCellKind::Blank),
            };
            SandboxLatticeCellView {
                q: coord.q(),
                r: coord.r(),
                label,
                kind,
            }
        })
        .collect()
}

fn infer_legacy_archetype<'a>(
    state: &LatticeState,
    lattices: &'a LatticeLibrary,
) -> Option<(&'a str, &'a hex_assets::Archetype)> {
    let saved_gems = state
        .mana_cells()
        .map(|(coord, _)| coord)
        .collect::<BTreeSet<_>>();
    let mut matches = lattices.iter().filter_map(|(name, archetype)| {
        let archetype_gems = archetype
            .spec
            .cells()
            .filter_map(|(coord, kind)| matches!(kind, CellKind::Gem { .. }).then_some(coord))
            .collect::<BTreeSet<_>>();
        (archetype_gems == saved_gems).then_some((name, archetype))
    });
    let inferred = matches.next()?;
    matches.next().is_none().then_some(inferred)
}

fn campaign_party_member_name(
    unit: &CampaignUnitSave,
    lattices: Option<&LatticeLibrary>,
) -> String {
    if !unit.archetype.is_empty() {
        return unit.display_name.clone();
    }
    let inferred = unit
        .lattice
        .as_ref()
        .and_then(|state| lattices.and_then(|lattices| infer_legacy_archetype(state, lattices)));
    inferred.map_or_else(
        || unit.display_name.clone(),
        |(name, _)| campaign_archetype_display_name(name),
    )
}

fn campaign_archetype_display_name(name: &str) -> String {
    name.split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            characters.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(characters).collect::<String>()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn compact_lattice_label(name: Option<&str>) -> String {
    let label = name
        .into_iter()
        .flat_map(str::chars)
        .filter(|character| character.is_alphanumeric())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    if label.is_empty() {
        "?".to_owned()
    } else {
        label
    }
}

fn encode_campaigns(file: &CampaignsFile) -> Result<String, String> {
    ron::ser::to_string_pretty(file, ron::ser::PrettyConfig::new())
        .map_err(|error| format!("Campaign data could not be encoded: {error}"))
}

fn validate_legacy_resume(resume: &LegacyResumeFile) -> Result<(), String> {
    if resume.format_version != LEGACY_RESUME_VERSION {
        return Err(format!(
            "Legacy resume format {} is incompatible with {}.",
            resume.format_version, LEGACY_RESUME_VERSION
        ));
    }
    validate_save_identity(
        &resume.build_version,
        &resume.scenario_name,
        resume.selected,
        resume
            .units
            .iter()
            .map(|unit| (unit.id, unit.faction, unit.position)),
    )?;
    let formations = shipped_formation_catalog()?;
    validate_formation(
        &resume.formation,
        resume.units.iter().map(|unit| (unit.id, unit.faction)),
        formations,
    )
}

fn validate_campaign_save(save: &CampaignSave, expected: CampaignSlotId) -> Result<(), String> {
    validate_campaign_save_against_catalog(save, expected, shipped_formation_catalog()?)
}

fn validate_campaign_save_v2(
    save: &CampaignSaveV2,
    expected: CampaignSlotId,
) -> Result<(), String> {
    if save.slot != expected {
        return Err(format!(
            "Campaign slot {} contains a record for slot {}.",
            expected.number(),
            save.slot.number()
        ));
    }
    save.checkpoint
        .validate()
        .map_err(|error| format!("Campaign checkpoint is invalid: {error}."))?;
    let local = crate::screens::multiplayer::local_build_identity()
        .map_err(|error| format!("Local Campaign build identity is invalid: {error}."))?;
    if save.checkpoint.build != local {
        return Err(format!(
            "Campaign build {:?} does not match this build {:?}.",
            save.checkpoint.build, local
        ));
    }
    validate_formation(
        &save.checkpoint.formation,
        save.checkpoint
            .units
            .as_slice()
            .iter()
            .map(|unit| (unit.unit, unit.faction)),
        shipped_formation_catalog()?,
    )
}

fn validate_campaign_save_against_catalog(
    save: &CampaignSave,
    expected: CampaignSlotId,
    formations: &FormationCatalog,
) -> Result<(), String> {
    if save.slot != expected {
        return Err(format!(
            "Campaign slot {} contains a record for slot {}.",
            expected.number(),
            save.slot.number()
        ));
    }
    validate_save_identity(
        &save.build_version,
        &save.scenario_name,
        save.selected,
        save.units
            .iter()
            .map(|unit| (unit.id, unit.faction, unit.position)),
    )?;
    validate_formation(
        &save.formation,
        save.units.iter().map(|unit| (unit.id, unit.faction)),
        formations,
    )
}

fn validate_save_identity(
    build_version: &str,
    scenario_name: &str,
    selected: Option<UnitId>,
    units: impl Iterator<Item = (UnitId, Faction, TilePos)>,
) -> Result<(), String> {
    if build_version != build_identity() {
        return Err(format!(
            "Campaign build {build_version} does not match this build {}.",
            build_identity()
        ));
    }
    if scenario_name.trim().is_empty() {
        return Err("Campaign data has no scenario name.".to_owned());
    }
    let units = units.collect::<Vec<_>>();
    if units.is_empty() {
        return Err("Campaign data has no units.".to_owned());
    }
    if !units
        .iter()
        .any(|(_, faction, _)| *faction == Faction::Player)
    {
        return Err("Campaign data has no player party.".to_owned());
    }
    let ids = units.iter().map(|(id, _, _)| *id).collect::<Vec<_>>();
    let unique: BTreeSet<UnitId> = ids.iter().copied().collect();
    if unique.len() != ids.len() {
        return Err("Campaign data repeats a unit id.".to_owned());
    }
    let positions = units
        .iter()
        .map(|(_, _, position)| *position)
        .collect::<BTreeSet<_>>();
    if positions.len() != units.len() {
        return Err("Campaign data places multiple units at one position.".to_owned());
    }
    if let Some(selected) = selected {
        let Some((_, faction, _)) = units.iter().find(|(id, _, _)| *id == selected) else {
            return Err("Campaign data selects a unit that is not present.".to_owned());
        };
        if *faction != Faction::Player {
            return Err("Campaign data selects a unit outside the player party.".to_owned());
        }
    }
    Ok(())
}

fn validate_formation(
    formation: &PartyFormation,
    units: impl Iterator<Item = (UnitId, Faction)>,
    catalog: &FormationCatalog,
) -> Result<(), String> {
    let players = units
        .filter_map(|(id, faction)| (faction == Faction::Player).then_some(id))
        .collect::<BTreeSet<_>>();
    if formation
        .assignments
        .keys()
        .any(|member| !players.contains(member))
    {
        return Err("Campaign formation references a unit outside the player party.".to_owned());
    }
    let unique_slots = formation
        .assignments
        .values()
        .copied()
        .collect::<BTreeSet<_>>();
    if unique_slots.len() != formation.assignments.len() {
        return Err("Campaign formation assigns multiple units to one slot.".to_owned());
    }
    let Some(preset) = catalog.get(&formation.preset) else {
        return Err(format!(
            "Campaign formation preset {:?} is unavailable.",
            formation.preset
        ));
    };
    let authored = preset
        .slots
        .iter()
        .map(|slot| slot.offset)
        .collect::<BTreeSet<_>>();
    if let Some(offset) = formation
        .assignments
        .values()
        .find(|offset| !authored.contains(offset))
    {
        return Err(format!(
            "Campaign formation uses offset {offset:?} outside preset {:?}.",
            formation.preset
        ));
    }
    Ok(())
}

fn shipped_formation_catalog() -> Result<&'static FormationCatalog, String> {
    static CATALOG: OnceLock<Result<FormationCatalog, String>> = OnceLock::new();
    CATALOG
        .get_or_init(|| {
            ron::from_str(include_str!("../../../assets/config/formations.ron"))
                .map_err(|error| format!("Shipped Campaign formations are invalid: {error}"))
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn format_active_time(active_play_millis: u64) -> String {
    let total_seconds = active_play_millis / 1_000;
    let hours = total_seconds / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;
    if hours == 0 {
        format!("{minutes:02}:{seconds:02}")
    } else {
        format!("{hours}:{minutes:02}:{seconds:02}")
    }
}

fn validate_campaign_catalog(
    library: Option<Res<ScenarioLibrary>>,
    accepted: Option<Res<AcceptedContentRevision>>,
    formations: Option<Res<FormationCatalog>>,
    rules: Option<Res<hex_assets::CombatSettings>>,
    mut store: ResMut<CampaignStore>,
) {
    let (Some(library), Some(accepted), Some(formations), Some(rules)) =
        (library, accepted, formations, rules)
    else {
        return;
    };
    let refusals = std::array::from_fn(|index| {
        let slot = CampaignSlotId::ALL.get(index).copied()?;
        let file = store.file.as_ref()?;
        if let Some(save) = file.v2_slots.get(index)?.as_ref() {
            return validate_campaign_save_v2(save, slot).err().or_else(|| {
                campaign_v2_content_refusal(save, &library, accepted.fingerprint(), &rules)
            });
        }
        let save = file.slots.get(index)?.as_ref()?;
        validate_campaign_save_against_catalog(save, slot, &formations)
            .err()
            .or_else(|| campaign_content_refusal(save, &library, accepted.fingerprint()))
    });
    if store.catalog_invalid != refusals {
        store.catalog_invalid = refusals;
    }
}

fn campaign_v2_content_refusal(
    save: &CampaignSaveV2,
    library: &ScenarioLibrary,
    accepted_fingerprint: u64,
    rules: &hex_assets::CombatSettings,
) -> Option<String> {
    let checkpoint = &save.checkpoint;
    let Some(scenario) = library
        .scenarios
        .iter()
        .find(|scenario| scenario.name == checkpoint.scenario_identity.as_str())
    else {
        return Some(format!(
            "The saved scenario {:?} is no longer available.",
            checkpoint.scenario_identity.as_str()
        ));
    };
    if scenario_digest(scenario) != checkpoint.scenario_digest {
        return Some(format!(
            "The saved scenario {:?} changed and cannot be resumed.",
            checkpoint.scenario_identity.as_str()
        ));
    }
    if scenario.world != checkpoint.map_catalog_identity.as_str() {
        return Some("The saved map catalog identity is incompatible.".to_owned());
    }
    if scenario.generation_seed.is_some() != checkpoint.resolved_seed.is_some() {
        return Some("The saved seed contract is incompatible.".to_owned());
    }
    if checkpoint.content_fingerprint != ContentFingerprint(accepted_fingerprint) {
        return Some("The saved authored content revision is incompatible.".to_owned());
    }
    if checkpoint.rules.profile_identity.as_str() != "campaign-rules-v1"
        || checkpoint.rules.fingerprint != crate::screens::sandbox::direct_rules_fingerprint(rules)
    {
        return Some("The saved Campaign rules are incompatible.".to_owned());
    }
    None
}

fn campaign_content_refusal(
    save: &CampaignSave,
    library: &ScenarioLibrary,
    accepted_fingerprint: u64,
) -> Option<String> {
    let Some(scenario) = library
        .scenarios
        .iter()
        .find(|scenario| scenario.name == save.scenario_name)
    else {
        return Some(format!(
            "The saved scenario {:?} is no longer available.",
            save.scenario_name
        ));
    };
    let current_digest = scenario_digest(scenario);
    if current_digest != save.scenario_digest
        && !(save.content_revision.is_none()
            && legacy_resume_digest_is_compatible(
                &scenario.name,
                save.scenario_digest,
                current_digest,
            ))
    {
        return Some(format!(
            "The saved scenario {:?} changed and cannot be resumed.",
            save.scenario_name
        ));
    }
    if scenario.generation_seed.is_some() != save.resolved_seed.is_some() {
        return Some(format!(
            "The saved seed contract for {:?} is incompatible.",
            save.scenario_name
        ));
    }
    if save
        .content_revision
        .is_some_and(|saved| saved != accepted_fingerprint)
    {
        return Some("The saved authored content revision is incompatible.".to_owned());
    }
    None
}

fn stage_new_campaign(
    slot: CampaignSlotId,
    library: &ScenarioLibrary,
    accepted: &AcceptedContentRevision,
    _formations: &FormationCatalog,
    commands: &mut Commands,
) -> Result<(), String> {
    let scenario = library.default_scenario().ok_or_else(|| {
        format!(
            "The configured default game {:?} does not exist.",
            library.default_game
        )
    })?;
    commands.remove_resource::<GameplaySetupFailure>();
    commands.remove_resource::<PendingCampaign>();
    commands.remove_resource::<PendingCampaignWorldSnapshotV2>();
    commands.remove_resource::<PendingCampaignGameplayCheckpointV2>();
    commands.insert_resource(ActiveCampaign::new(slot, accepted.fingerprint(), 0));
    commands.insert_resource(GameplaySessionOrigin::Campaign(slot));
    commands.insert_resource(ScenarioToLoad {
        scenario: scenario.clone(),
        resolved_seed: scenario.generation_seed.map(ResolvedMapSeed),
        encounter_override: None,
    });
    Ok(())
}

fn stage_saved_campaign(
    slot: CampaignSlotId,
    record: CampaignRecord,
    library: &ScenarioLibrary,
    accepted: &AcceptedContentRevision,
    formations: &FormationCatalog,
    rules: &hex_assets::CombatSettings,
    commands: &mut Commands,
) -> Result<bool, String> {
    let requires_v2_restore = matches!(&record, CampaignRecord::V2(_));
    match record {
        CampaignRecord::V1(save) => {
            validate_campaign_save_against_catalog(&save, slot, formations)?;
            if let Some(reason) = campaign_content_refusal(&save, library, accepted.fingerprint()) {
                return Err(reason);
            }
            let scenario = library
                .scenarios
                .iter()
                .find(|scenario| scenario.name == save.scenario_name)
                .ok_or_else(|| {
                    format!(
                        "The saved scenario {:?} is no longer available.",
                        save.scenario_name
                    )
                })?;
            commands.remove_resource::<GameplaySetupFailure>();
            commands.remove_resource::<PendingCampaignWorldSnapshotV2>();
            commands.remove_resource::<PendingCampaignGameplayCheckpointV2>();
            commands.insert_resource(ScenarioToLoad {
                scenario: scenario.clone(),
                resolved_seed: save.resolved_seed.map(ResolvedMapSeed),
                encounter_override: None,
            });
            commands.insert_resource(PendingCampaign(save.clone()));
            commands.insert_resource(ActiveCampaign::new(
                slot,
                save.content_revision
                    .unwrap_or_else(|| accepted.fingerprint()),
                save.active_play_millis,
            ));
        }
        CampaignRecord::V2(save) => {
            if let Some(reason) =
                campaign_v2_content_refusal(&save, library, accepted.fingerprint(), rules)
            {
                return Err(reason);
            }
            let checkpoint = save.checkpoint;
            let scenario = library
                .scenarios
                .iter()
                .find(|scenario| scenario.name == checkpoint.scenario_identity.as_str())
                .ok_or_else(|| "The saved scenario is no longer available.".to_owned())?;
            commands.remove_resource::<GameplaySetupFailure>();
            commands.remove_resource::<PendingCampaign>();
            commands.insert_resource(PendingCampaignWorldSnapshotV2::new(
                checkpoint.world.clone(),
            ));
            commands.insert_resource(PendingCampaignGameplayCheckpointV2(
                CampaignGameplayCheckpointV2 {
                    units: checkpoint.units.clone(),
                    effects: checkpoint.effects.clone(),
                    formation: checkpoint.formation.clone(),
                },
            ));
            commands.insert_resource(ScenarioToLoad {
                scenario: scenario.clone(),
                resolved_seed: checkpoint.resolved_seed.map(ResolvedMapSeed),
                encounter_override: None,
            });
            commands.insert_resource(ActiveCampaign::new(
                slot,
                checkpoint.content_fingerprint.0,
                checkpoint.active_play_millis,
            ));
        }
    }
    commands.insert_resource(GameplaySessionOrigin::Campaign(slot));
    Ok(requires_v2_restore)
}

fn handle_campaign_intents(
    mut intents: MessageReader<UiIntent>,
    mut store: ResMut<CampaignStore>,
    library: Option<Res<ScenarioLibrary>>,
    accepted: Option<Res<AcceptedContentRevision>>,
    formations: Option<Res<FormationCatalog>>,
    rules: Option<Res<hex_assets::CombatSettings>>,
    mut commands: Commands,
    mut next: ResMut<NextState<Screen>>,
) {
    for intent in intents.read() {
        let UiIntent::MainMenu(intent) = intent else {
            continue;
        };
        let (Some(library), Some(accepted), Some(formations), Some(rules)) = (
            library.as_deref(),
            accepted.as_deref(),
            formations.as_deref(),
            rules.as_deref(),
        ) else {
            if matches!(
                intent,
                MainMenuIntent::NewCampaign(_) | MainMenuIntent::ContinueCampaign(_)
            ) {
                commands.insert_resource(GameplaySetupFailure::new(
                    "Campaign content is still loading.",
                ));
            }
            continue;
        };
        let result = match *intent {
            MainMenuIntent::NewCampaign(slot) if store.is_empty(slot) => {
                stage_new_campaign(slot, library, accepted, formations, &mut commands)
            }
            MainMenuIntent::ContinueCampaign(slot) => {
                let Some(record) = store.available_record(slot) else {
                    continue;
                };
                stage_saved_campaign(
                    slot,
                    record,
                    library,
                    accepted,
                    formations,
                    rules,
                    &mut commands,
                )
                .map(|_requires_v2_restore| ())
                .inspect_err(|reason| {
                    store.mark_catalog_invalid(slot, reason.clone());
                })
            }
            _ => continue,
        };
        match result {
            Ok(()) => {
                next.set(Screen::Loading);
                return;
            }
            Err(reason) => commands.insert_resource(GameplaySetupFailure::new(reason)),
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the Campaign host bootstrap validates one persisted slot against every accepted catalog before it may enter world setup"
)]
fn begin_campaign_multiplayer_host(
    request: Option<Res<CampaignMultiplayerHostRequest>>,
    endpoint: Option<Res<crate::screens::multiplayer::PendingDirectHostSetup>>,
    mut store: ResMut<CampaignStore>,
    library: Option<Res<ScenarioLibrary>>,
    accepted: Option<Res<AcceptedContentRevision>>,
    formations: Option<Res<FormationCatalog>>,
    rules: Option<Res<hex_assets::CombatSettings>>,
    mut status: ResMut<CampaignMultiplayerHostStatus>,
    mut commands: Commands,
    mut next: ResMut<NextState<Screen>>,
) {
    let Some(request) = request.as_deref().copied() else {
        return;
    };
    commands.remove_resource::<CampaignMultiplayerHostRequest>();
    commands.remove_resource::<crate::screens::multiplayer::PreparedDirectSandboxSession>();
    commands.remove_resource::<crate::screens::multiplayer::PreparedDirectCampaignSession>();
    status.slot = Some(request.slot);
    status.preparing = false;
    status.refusal = None;
    status.notice = None;

    if endpoint.is_none() {
        refuse_campaign_multiplayer_host(
            &mut status,
            CampaignMultiplayerHostRefusal::ContentUnavailable,
            "The Direct/LAN endpoint is unavailable; Campaign hosting was not started.",
        );
        return;
    }
    let (Some(library), Some(accepted), Some(formations), Some(rules)) = (
        library.as_deref(),
        accepted.as_deref(),
        formations.as_deref(),
        rules.as_deref(),
    ) else {
        refuse_campaign_multiplayer_host(
            &mut status,
            CampaignMultiplayerHostRefusal::ContentUnavailable,
            "Campaign content is still loading.",
        );
        return;
    };

    let staged = match store.record(request.slot) {
        Ok(None) => stage_new_campaign(request.slot, library, accepted, formations, &mut commands)
            .map(|()| false),
        Ok(Some(record)) => stage_saved_campaign(
            request.slot,
            record,
            library,
            accepted,
            formations,
            rules,
            &mut commands,
        ),
        Err(reason) => Err(reason),
    };
    let requires_v2_restore = match staged {
        Ok(value) => value,
        Err(reason) => {
            store.mark_catalog_invalid(request.slot, reason.clone());
            refuse_campaign_multiplayer_host(
                &mut status,
                CampaignMultiplayerHostRefusal::IncompatibleCheckpoint,
                reason,
            );
            return;
        }
    };

    commands.remove_resource::<CampaignWorldRestoreResultV2>();
    commands.remove_resource::<CampaignGameplayRestoreResultV2>();
    commands.insert_resource(PendingCampaignHostBootstrap {
        slot: request.slot,
        requires_v2_restore,
    });
    status.preparing = true;
    status.notice = Some(format!(
        "Preparing Campaign slot {} for a fresh assignment lobby…",
        request.slot.number()
    ));
    next.set(Screen::Loading);
}

fn refuse_campaign_multiplayer_host(
    status: &mut CampaignMultiplayerHostStatus,
    refusal: CampaignMultiplayerHostRefusal,
    notice: impl Into<String>,
) {
    status.preparing = false;
    status.refusal = Some(refusal);
    status.notice = Some(notice.into());
}

fn complete_campaign_multiplayer_bootstrap(world: &mut World) {
    let Some(bootstrap) = world
        .get_resource::<PendingCampaignHostBootstrap>()
        .copied()
    else {
        return;
    };
    if world
        .get_resource::<State<Screen>>()
        .is_none_or(|screen| *screen.get() != Screen::Gameplay)
        || world
            .get_resource::<GameplayPhase>()
            .is_none_or(|phase| *phase != GameplayPhase::Active)
    {
        return;
    }
    if let Some(failure) = world.get_resource::<GameplaySetupFailure>() {
        let reason = failure.reason.clone();
        finish_campaign_multiplayer_refusal(
            world,
            CampaignMultiplayerHostRefusal::RestoreFailed,
            reason,
        );
        return;
    }
    if bootstrap.requires_v2_restore {
        let world_restored = world
            .get_resource::<CampaignWorldRestoreResultV2>()
            .is_some_and(|result| {
                matches!(
                    result.outcome,
                    CampaignWorldRestoreOutcomeV2::Applied { .. }
                )
            });
        let gameplay_restored = world
            .get_resource::<CampaignGameplayRestoreResultV2>()
            .is_some_and(|result| {
                matches!(
                    result.outcome,
                    CampaignGameplayRestoreOutcomeV2::Applied { .. }
                )
            });
        if !world_restored || !gameplay_restored {
            // OnEnter restore systems may still be publishing their typed results.
            return;
        }
    }
    let Some(active) = world.get_resource::<ActiveCampaign>().cloned() else {
        finish_campaign_multiplayer_refusal(
            world,
            CampaignMultiplayerHostRefusal::IncompleteCheckpoint,
            "Campaign ownership disappeared before the multiplayer checkpoint was complete.",
        );
        return;
    };
    let checkpoint = match build_host_campaign_checkpoint(world, &active) {
        Ok(checkpoint) => checkpoint,
        Err((_refusal, reason)) => {
            finish_campaign_multiplayer_refusal(
                world,
                CampaignMultiplayerHostRefusal::IncompleteCheckpoint,
                reason,
            );
            return;
        }
    };
    let prepared = match crate::screens::multiplayer::PreparedDirectCampaignSession::from_checkpoint(
        checkpoint,
        bootstrap.slot,
    ) {
        Ok(prepared) => prepared,
        Err(reason) => {
            finish_campaign_multiplayer_refusal(
                world,
                CampaignMultiplayerHostRefusal::IncompatibleCheckpoint,
                reason,
            );
            return;
        }
    };
    world.insert_resource(prepared);
    world.remove_resource::<PendingCampaignHostBootstrap>();
    world.remove_resource::<GameplaySetupFailure>();
    let mut status = world.resource_mut::<CampaignMultiplayerHostStatus>();
    status.preparing = false;
    status.refusal = None;
    status.notice = Some(format!(
        "Campaign slot {} is ready for fresh seat assignment.",
        bootstrap.slot.number()
    ));
    world
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Multiplayer);
}

fn finish_campaign_multiplayer_refusal(
    world: &mut World,
    refusal: CampaignMultiplayerHostRefusal,
    notice: impl Into<String>,
) {
    world.remove_resource::<PendingCampaignHostBootstrap>();
    let notice = notice.into();
    let mut status = world.resource_mut::<CampaignMultiplayerHostStatus>();
    refuse_campaign_multiplayer_host(&mut status, refusal, notice);
    world
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Multiplayer);
}

fn accumulate_active_play_time(
    time: Res<Time<Real>>,
    screen: Res<State<Screen>>,
    pause: Option<Res<State<Pause>>>,
    phase: Res<GameplayPhase>,
    resolution: Res<EncounterResolution>,
    origin: Option<Res<GameplaySessionOrigin>>,
    active: Option<ResMut<ActiveCampaign>>,
) {
    let Some(mut active) = active else { return };
    let counts_now = counts_as_active_play(
        *screen.get(),
        pause.as_deref().map(|pause| *pause.get()),
        *phase,
        resolution.is_resolved(),
        campaign_origin_matches(origin.as_deref(), active.slot),
    );
    if active.count_previous_interval {
        active.session_active_play = active.session_active_play.saturating_add(time.delta());
    }
    active.count_previous_interval = counts_now;
}

fn campaign_origin_matches(origin: Option<&GameplaySessionOrigin>, slot: CampaignSlotId) -> bool {
    campaign_origin_refusal(origin, slot).is_none()
}

fn campaign_origin_refusal(
    origin: Option<&GameplaySessionOrigin>,
    slot: CampaignSlotId,
) -> Option<&'static str> {
    match origin {
        Some(GameplaySessionOrigin::Campaign(origin_slot)) if *origin_slot == slot => None,
        Some(GameplaySessionOrigin::Campaign(_)) => {
            Some("Campaign not saved: the bound slot does not match this session.")
        }
        _ => Some("Campaign not saved: this session is temporary."),
    }
}

fn counts_as_active_play(
    screen: Screen,
    pause: Option<Pause>,
    phase: GameplayPhase,
    resolved: bool,
    campaign_bound: bool,
) -> bool {
    campaign_bound
        && screen == Screen::Gameplay
        && pause == Some(Pause(false))
        && phase == GameplayPhase::Active
        && !resolved
}

fn safe_for_manual_campaign_save(
    screen: Screen,
    mode: Option<Mode>,
    pause: Option<Pause>,
    phase: GameplayPhase,
    commands_settled: bool,
    resolved: bool,
) -> bool {
    screen == Screen::Gameplay
        && mode == Some(Mode::Exploring)
        && pause == Some(Pause(true))
        && phase == GameplayPhase::Active
        && commands_settled
        && !resolved
}

fn save_exploration(world: &mut World) {
    let requested = world
        .get_resource::<State<Screen>>()
        .is_some_and(|screen| *screen.get() == Screen::Gameplay)
        && world
            .get_resource::<InputBindings>()
            .zip(world.get_resource::<ButtonInput<KeyCode>>())
            .is_some_and(|(bindings, keys)| bindings.just_pressed(keys, InputAction::Save));
    if !requested {
        return;
    }
    if !world.contains_resource::<CampaignSaveRuntime>() {
        world.insert_resource(CampaignSaveRuntime::default());
    }
    if !world.contains_resource::<CampaignSaveStatusProjection>() {
        world.insert_resource(CampaignSaveStatusProjection::default());
    }
    if world.resource::<CampaignSaveRuntime>().pending.is_some() {
        world.resource_mut::<CampaignSaveNotice>().0 =
            Some("Campaign save is already in progress.".to_owned());
        return;
    }

    let Some(active) = world.get_resource::<ActiveCampaign>().cloned() else {
        world.resource_mut::<CampaignSaveNotice>().0 =
            Some("Campaign not saved: this session is temporary.".to_owned());
        return;
    };
    let operation_id = {
        let mut runtime = world.resource_mut::<CampaignSaveRuntime>();
        let Some(next) = runtime.next_operation_id.checked_add(1) else {
            world.resource_mut::<CampaignSaveNotice>().0 =
                Some("Campaign not saved: save operation IDs are exhausted.".to_owned());
            return;
        };
        runtime.next_operation_id = next;
        next
    };
    if world
        .get_resource::<SimulationRole>()
        .is_some_and(|role| *role != SimulationRole::Authority)
    {
        refuse_campaign_save(
            world,
            operation_id,
            CampaignSaveRefusalV2::NotAuthority,
            "Campaign not saved: only the listen host owns the Campaign save.".to_owned(),
        );
        return;
    }
    if let Some(refusal) =
        campaign_origin_refusal(world.get_resource::<GameplaySessionOrigin>(), active.slot)
    {
        refuse_campaign_save(
            world,
            operation_id,
            CampaignSaveRefusalV2::NotAuthority,
            refusal.to_owned(),
        );
        return;
    }

    let commands_settled = world.resource::<CommandQueue>().is_empty()
        && !world.resource::<PendingDecision>().is_open()
        && world
            .query_filtered::<Entity, Or<(With<MovingTo>, With<Busy>)>>()
            .iter(world)
            .next()
            .is_none();
    let safe = safe_for_manual_campaign_save(
        *world.resource::<State<Screen>>().get(),
        world.get_resource::<State<Mode>>().map(|mode| *mode.get()),
        world
            .get_resource::<State<Pause>>()
            .map(|pause| *pause.get()),
        *world.resource::<GameplayPhase>(),
        commands_settled,
        world.resource::<EncounterResolution>().is_resolved(),
    );
    if !safe {
        refuse_campaign_save(
            world,
            operation_id,
            CampaignSaveRefusalV2::UnsafeBoundary,
            "Campaign not saved: pause during safe exploration with no movement or decision pending."
                .to_owned(),
        );
        return;
    }

    let checkpoint = match build_host_campaign_checkpoint(world, &active) {
        Ok(checkpoint) => checkpoint,
        Err((refusal, reason)) => {
            refuse_campaign_save(world, operation_id, refusal, reason);
            return;
        }
    };
    let save = CampaignSaveV2 {
        slot: active.slot,
        checkpoint,
    };
    if let Err(reason) = validate_campaign_save_v2(&save, active.slot) {
        refuse_campaign_save(
            world,
            operation_id,
            CampaignSaveRefusalV2::IncompleteCheckpoint,
            format!("Campaign not saved: {reason}"),
        );
        return;
    }
    world.resource_mut::<CampaignSaveRuntime>().pending = Some(PendingCampaignWriteV2 {
        operation_id,
        slot: active.slot,
        save,
    });
    publish_campaign_save_status(world, operation_id, CampaignSaveStateV2::Saving);
    world.resource_mut::<CampaignSaveNotice>().0 =
        Some(format!("Saving Campaign slot {}…", active.slot.number()));
}

fn build_host_campaign_checkpoint(
    world: &mut World,
    active: &ActiveCampaign,
) -> Result<HostCampaignCheckpointV2, (CampaignSaveRefusalV2, String)> {
    let active_scenario = world
        .get_resource::<ActiveScenario>()
        .cloned()
        .ok_or_else(|| {
            (
                CampaignSaveRefusalV2::IncompleteCheckpoint,
                "Campaign not saved: scenario setup is incomplete.".to_owned(),
            )
        })?;
    let map = world
        .get_resource::<MapSettings>()
        .cloned()
        .ok_or_else(|| {
            (
                CampaignSaveRefusalV2::IncompleteCheckpoint,
                "Campaign not saved: map setup is incomplete.".to_owned(),
            )
        })?;
    let simulation_seeds = world.get_resource::<SimSeeds>().copied().ok_or_else(|| {
        (
            CampaignSaveRefusalV2::IncompleteCheckpoint,
            "Campaign not saved: simulation seeds are unavailable.".to_owned(),
        )
    })?;
    let rules = world
        .get_resource::<hex_assets::CombatSettings>()
        .cloned()
        .ok_or_else(|| {
            (
                CampaignSaveRefusalV2::IncompleteCheckpoint,
                "Campaign not saved: gameplay rules are unavailable.".to_owned(),
            )
        })?;
    let accepted = world
        .get_resource::<AcceptedContentRevision>()
        .map(AcceptedContentRevision::fingerprint)
        .ok_or_else(|| {
            (
                CampaignSaveRefusalV2::IncompatibleContent,
                "Campaign not saved: authored content is not accepted.".to_owned(),
            )
        })?;
    if accepted != active.content_revision {
        return Err((
            CampaignSaveRefusalV2::IncompatibleContent,
            "Campaign not saved: authored content changed during this session.".to_owned(),
        ));
    }
    let world_snapshot = world
        .get_resource::<CurrentWorldSnapshotV1>()
        .map(|snapshot| snapshot.snapshot().clone())
        .ok_or_else(|| {
            (
                CampaignSaveRefusalV2::IncompleteCheckpoint,
                "Campaign not saved: the complete world snapshot is unavailable.".to_owned(),
            )
        })?;
    let gameplay = export_campaign_gameplay_checkpoint(world).map_err(|error| {
        (
            CampaignSaveRefusalV2::IncompleteCheckpoint,
            format!("Campaign not saved: gameplay checkpoint is incomplete: {error}."),
        )
    })?;
    let identity = |value: String| {
        BoundedText::<MAX_IDENTITY_BYTES>::new(value).map_err(|error| {
            (
                CampaignSaveRefusalV2::IncompleteCheckpoint,
                format!("Campaign not saved: checkpoint identity is invalid: {error}."),
            )
        })
    };
    let (generator_identity, generator_version) = campaign_generator_contract(&map);
    let checkpoint = HostCampaignCheckpointV2 {
        version: CAMPAIGN_CHECKPOINT_VERSION_V2,
        build: crate::screens::multiplayer::local_build_identity().map_err(|error| {
            (
                CampaignSaveRefusalV2::IncompleteCheckpoint,
                format!("Campaign not saved: local build identity is invalid: {error}."),
            )
        })?,
        content_fingerprint: ContentFingerprint(accepted),
        scenario_identity: identity(active_scenario.0.scenario.name.clone())?,
        scenario_digest: scenario_digest(&active_scenario.0.scenario),
        map_catalog_identity: identity(active_scenario.0.scenario.world.clone())?,
        generator_identity: identity(generator_identity.to_owned())?,
        generator_version,
        resolved_seed: active_scenario.0.resolved_seed.map(|seed| seed.0),
        rules: RulesManifestV1 {
            profile_identity: identity("campaign-rules-v1".to_owned())?,
            fingerprint: crate::screens::sandbox::direct_rules_fingerprint(&rules),
        },
        simulation_seeds,
        world: world_snapshot,
        units: gameplay.units,
        effects: gameplay.effects,
        formation: gameplay.formation,
        active_play_millis: active.active_play_millis(),
    };
    checkpoint.validate().map_err(|error| {
        (
            CampaignSaveRefusalV2::IncompleteCheckpoint,
            format!("Campaign not saved: checkpoint validation failed: {error}."),
        )
    })?;
    Ok(checkpoint)
}

fn campaign_generator_contract(map: &MapSettings) -> (&'static str, u32) {
    match &map.terrain {
        TerrainSettings::Showcase(_) => ("showcase", 1),
        TerrainSettings::Perlin(_) => ("perlin", 1),
        TerrainSettings::Procedural(hex_map::ProceduralSettings::V1(_)) => ("procedural-v1", 1),
        TerrainSettings::Procedural(hex_map::ProceduralSettings::V2(_)) => ("procedural-v2", 2),
        TerrainSettings::Procedural(hex_map::ProceduralSettings::V3(_)) => ("procedural-v3", 3),
    }
}

fn commit_pending_campaign_save(world: &mut World) {
    let Some(pending) = world.resource_mut::<CampaignSaveRuntime>().pending.take() else {
        return;
    };
    let valid_owner = world
        .get_resource::<SimulationRole>()
        .is_none_or(|role| *role == SimulationRole::Authority)
        && world
            .get_resource::<ActiveCampaign>()
            .is_some_and(|active| active.slot == pending.slot);
    if !valid_owner {
        refuse_campaign_save(
            world,
            pending.operation_id,
            CampaignSaveRefusalV2::NotAuthority,
            "Campaign save was cancelled because its authoritative session ended.".to_owned(),
        );
        return;
    }
    let paths = world.resource::<StoragePaths>().clone();
    match world
        .resource_mut::<CampaignStore>()
        .write_v2_slot(&paths, pending.slot, pending.save)
    {
        Ok(()) => {
            if let Some(mut active) = world.get_resource_mut::<ActiveCampaign>() {
                active.mark_persisted();
            }
            publish_campaign_save_status(world, pending.operation_id, CampaignSaveStateV2::Saved);
            world.resource_mut::<CampaignSaveNotice>().0 =
                Some(format!("Campaign slot {} saved.", pending.slot.number()));
        }
        Err(reason) => refuse_campaign_save(
            world,
            pending.operation_id,
            CampaignSaveRefusalV2::StorageUnavailable,
            reason,
        ),
    }
}

fn refuse_campaign_save(
    world: &mut World,
    operation_id: u64,
    refusal: CampaignSaveRefusalV2,
    notice: String,
) {
    publish_campaign_save_status(world, operation_id, CampaignSaveStateV2::Refused(refusal));
    world.resource_mut::<CampaignSaveNotice>().0 = Some(notice);
}

fn publish_campaign_save_status(world: &mut World, operation_id: u64, state: CampaignSaveStateV2) {
    if !world.contains_resource::<CampaignSaveStatusProjection>() {
        world.insert_resource(CampaignSaveStatusProjection::default());
    }
    let mut projection = world.resource_mut::<CampaignSaveStatusProjection>();
    projection.operation_id = operation_id;
    projection.state = Some(state);
    let session = world
        .get_resource::<SessionAdmissionAuthority>()
        .filter(|authority| {
            authority.manifest().launch_kind == hex_multiplayer::SessionLaunchKindV1::Campaign
        })
        .map(|authority| authority.manifest().session_instance_id);
    let Some(session_instance_id) = session else {
        return;
    };
    if let Some(mut messages) = world.get_resource_mut::<
        bevy::ecs::message::Messages<bevy_replicon::prelude::ToClients<CampaignSaveStatusV2>>,
    >() {
        messages.write(bevy_replicon::prelude::ToClients {
            targets: bevy_replicon::prelude::SendTargets::All,
            message: CampaignSaveStatusV2 {
                session_instance_id,
                operation_id,
                state,
            },
        });
    }
}

fn capture_remote_campaign_save_status(
    role: Res<SimulationRole>,
    mut statuses: MessageReader<CampaignSaveStatusV2>,
    mut projection: ResMut<CampaignSaveStatusProjection>,
    mut notice: ResMut<CampaignSaveNotice>,
) {
    if *role != SimulationRole::Replica {
        statuses.clear();
        return;
    }
    for status in statuses.read() {
        if status.operation_id < projection.operation_id {
            continue;
        }
        projection.operation_id = status.operation_id;
        projection.state = Some(status.state);
        notice.0 = Some(match status.state {
            CampaignSaveStateV2::Saving => "The host is saving the Campaign…".to_owned(),
            CampaignSaveStateV2::Saved => "The host saved the Campaign.".to_owned(),
            CampaignSaveStateV2::Refused(_) => {
                "The host could not save the Campaign at this boundary.".to_owned()
            }
        });
    }
}

#[expect(
    clippy::expect_used,
    reason = "the full immutable roster and footing maps were preflighted in this same system"
)]
fn restore_pending_campaign(
    mut commands: Commands,
    pending: Option<Res<PendingCampaign>>,
    map: Res<MapSettings>,
    formations: Res<FormationCatalog>,
    content: Res<ContentIndex>,
    elements: Res<ElementCatalog>,
    lattices: Res<LatticeLibrary>,
    substances: Res<SubstanceTable>,
    blockers: Option<Res<TraversalBlockers>>,
    tiles: Query<(&TilePos, &HexSpan, &SubstanceId, &Headroom), With<HexTile>>,
    mut units: Query<(
        Entity,
        &UnitId,
        &Faction,
        &UnitArchetype,
        &mut StandsOn,
        &mut Transform,
        Option<&mut LatticeState>,
        Has<Downed>,
        Has<Selected>,
    )>,
    mut formation: ResMut<PartyFormation>,
    mut store: ResMut<CampaignStore>,
    mut next: ResMut<NextState<Screen>>,
) {
    let Some(pending) = pending else { return };
    let save = &pending.0;
    if let Err(reason) = validate_campaign_save_against_catalog(save, save.slot, &formations) {
        fail_restore(&mut commands, &mut next, &mut store, save.slot, reason);
        return;
    }
    if generator_version(&map) != save.generator_version {
        fail_restore(
            &mut commands,
            &mut next,
            &mut store,
            save.slot,
            format!(
                "The saved generator version {:?} does not match {:?}.",
                save.generator_version,
                generator_version(&map)
            ),
        );
        return;
    }

    let saved: BTreeMap<UnitId, &CampaignUnitSave> =
        save.units.iter().map(|unit| (unit.id, unit)).collect();
    let runtime_ids = units.iter().map(|(_, id, ..)| *id).collect::<Vec<_>>();
    let runtime_unique = runtime_ids.iter().copied().collect::<BTreeSet<_>>();
    let saved_ids = saved.keys().copied().collect::<BTreeSet<_>>();
    if runtime_unique.len() != runtime_ids.len() || runtime_unique != saved_ids {
        fail_restore(
            &mut commands,
            &mut next,
            &mut store,
            save.slot,
            "The saved roster no longer matches this scenario.".to_owned(),
        );
        return;
    }

    // Refuse before mutating anything. GameplaySetup continues through perception
    // and finalization in this frame, so a half-restored world is not an acceptable
    // transient state even though the next screen has already been requested.
    let tables = content.tables(&elements);
    let footing = Footing::from_tiles(
        tiles.iter(),
        &substances,
        Body::new(TraversalProfile::WALKER),
        blockers.as_deref(),
    );
    let mut restored_standing = BTreeMap::new();
    for (_, id, faction, archetype, _, _, lattice, _, _) in units.iter() {
        let Some(snapshot) = saved.get(id) else {
            fail_restore(
                &mut commands,
                &mut next,
                &mut store,
                save.slot,
                format!("The saved roster is missing unit {}.", id.0),
            );
            return;
        };
        if snapshot.faction != *faction {
            fail_restore(
                &mut commands,
                &mut next,
                &mut store,
                save.slot,
                format!("The saved faction for unit {} changed.", id.0),
            );
            return;
        }
        if let Some(reason) = campaign_lattice_refusal(snapshot, archetype, &lattices, &tables) {
            fail_restore(&mut commands, &mut next, &mut store, save.slot, reason);
            return;
        }
        let Some(standing) = footing.at(snapshot.position) else {
            fail_restore(
                &mut commands,
                &mut next,
                &mut store,
                save.slot,
                format!(
                    "The saved position for unit {} is no longer valid footing.",
                    id.0
                ),
            );
            return;
        };
        restored_standing.insert(*id, standing);
        if snapshot.lattice.is_some() != lattice.is_some() {
            fail_restore(
                &mut commands,
                &mut next,
                &mut store,
                save.slot,
                format!("The saved lattice for unit {} no longer matches.", id.0),
            );
            return;
        }
    }

    // Every player-data-dependent refusal above has completed. These lookups are
    // defensive invariants over the same query and maps; they cannot be changed by
    // another system while this system is running.
    for (entity, id, _, _, mut standing, mut transform, lattice, downed, selected) in &mut units {
        let snapshot = saved
            .get(id)
            .expect("every runtime unit was matched during Campaign restore preflight");
        let restored = restored_standing
            .get(id)
            .expect("every saved position produced footing during Campaign restore preflight");
        standing.0 = *restored;
        transform.translation = standing.0.world_position();
        if let (Some(saved), Some(mut current)) = (snapshot.lattice.as_ref(), lattice) {
            *current = saved.clone();
        }
        if snapshot.downed && !downed {
            commands.entity(entity).insert(Downed);
        } else if !snapshot.downed && downed {
            commands.entity(entity).remove::<Downed>();
        }
        let should_select = save.selected == Some(*id);
        if should_select && !selected {
            commands.entity(entity).insert(Selected);
        } else if !should_select && selected {
            commands.entity(entity).remove::<Selected>();
        }
    }
    *formation = save.formation.clone();
    commands.remove_resource::<PendingCampaign>();
    info!(
        "restored Campaign slot {} for {}",
        save.slot.number(),
        save.scenario_name
    );
}

fn campaign_lattice_refusal(
    snapshot: &CampaignUnitSave,
    runtime_archetype: &UnitArchetype,
    lattices: &LatticeLibrary,
    spells: &impl hex_lattice::Tables,
) -> Option<String> {
    if !snapshot.archetype.is_empty() && snapshot.archetype != runtime_archetype.0 {
        return Some(format!(
            "The saved archetype for unit {} changed.",
            snapshot.id.0
        ));
    }
    let saved_lattice = snapshot.lattice.as_ref()?;
    let Some(definition) = lattices.get(&runtime_archetype.0) else {
        return Some(format!(
            "The saved archetype for unit {} is unavailable.",
            snapshot.id.0
        ));
    };
    saved_lattice
        .validate_against(&definition.spec, &definition.stats, spells)
        .err()
        .map(|error| {
            format!(
                "The saved lattice for unit {} is invalid: {error}.",
                snapshot.id.0
            )
        })
}

fn fail_restore(
    commands: &mut Commands,
    next: &mut NextState<Screen>,
    store: &mut CampaignStore,
    slot: CampaignSlotId,
    reason: String,
) {
    commands.insert_resource(GameplaySetupFailure::new(format!(
        "Campaign slot {} is incompatible: {reason}",
        slot.number()
    )));
    store.mark_invalid(slot, reason);
    commands.remove_resource::<PendingCampaign>();
    commands.remove_resource::<ActiveCampaign>();
    next.set(Screen::Title);
}

fn generator_version(map: &MapSettings) -> Option<u32> {
    match &map.terrain {
        TerrainSettings::Procedural(settings) => Some(settings.generator_version()),
        TerrainSettings::Showcase(_) | TerrainSettings::Perlin(_) => None,
    }
}

fn scenario_digest(scenario: &Scenario) -> u64 {
    let mut digest = 0xcbf2_9ce4_8422_2325_u64;
    let mut fold = |bytes: &[u8]| {
        for byte in bytes {
            digest ^= u64::from(*byte);
            digest = digest.wrapping_mul(0x0000_0100_0000_01B3);
        }
        digest ^= 0xff;
        digest = digest.wrapping_mul(0x0000_0100_0000_01B3);
    };
    fold(build_identity().as_bytes());
    fold(scenario.name.as_bytes());
    fold(scenario.world.as_bytes());
    fold(scenario.lighting.as_bytes());
    fold(scenario.encounter.as_bytes());
    if let Some(seed) = scenario.generation_seed {
        fold(&seed.to_le_bytes());
    }
    if let Some(hours) = scenario.starting_time_hours {
        fold(&hours.to_bits().to_le_bytes());
    }
    // Coarse by design: any shipped gameplay or world content change refuses the
    // build-bound save instead of pretending to migrate semantic ids.
    for (_, content) in SHIPPED_CAMPAIGN_INPUTS {
        fold(content.as_bytes());
    }
    digest
}

fn legacy_resume_digest_is_compatible(
    scenario_name: &str,
    saved_digest: u64,
    current_digest: u64,
) -> bool {
    LEGACY_RESUME_DIGESTS.iter().any(|(name, legacy, cutover)| {
        *name == scenario_name && *legacy == saved_digest && *cutover == current_digest
    })
}

fn build_identity() -> &'static str {
    option_env!("HEX_GAME_BUILD_ID").unwrap_or(env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use bevy::state::app::StatesPlugin;
    use bevy::time::TimeUpdateStrategy;
    use bevy::MinimalPlugins;
    use hex_assets::{
        ArtPalette, ContentIndex, ElementFile, Encounter, LatticeFile, ScenarioCategory, SpellBook,
        SpellFile, SubstanceFile, SubstanceTable, TerrainDamageFile, TerrainDamageTable,
    };
    use hex_multiplayer::{
        BoundedVec, CampaignEffectLedgerV2, PublicWorldFingerprint, WorldColumnSnapshotV1,
        WorldRunSnapshotV1, WorldSnapshotV1, WORLD_SNAPSHOT_VERSION_V1,
    };
    use hex_units::Standing;

    use super::*;

    fn scenario() -> Scenario {
        Scenario {
            name: "Party Trial".to_owned(),
            category: ScenarioCategory::Demo,
            blurb: "Integrated.".to_owned(),
            world: "config/world.ron".to_owned(),
            lighting: "config/lighting.ron".to_owned(),
            generation_seed: None,
            starting_time_hours: None,
            encounter: "config/encounters/party-trial.ron".to_owned(),
        }
    }

    fn compact_formation(members: &[UnitId]) -> PartyFormation {
        let preset = shipped_formation_catalog()
            .expect("the shipped formation catalog is valid")
            .get("Compact")
            .expect("the shipped Compact formation exists");
        let mut formation = PartyFormation::default();
        formation.select_preset(preset, members);
        formation
    }

    fn legacy_resume() -> LegacyResumeFile {
        LegacyResumeFile {
            format_version: LEGACY_RESUME_VERSION,
            build_version: build_identity().to_owned(),
            scenario_name: "Party Trial".to_owned(),
            scenario_digest: scenario_digest(&scenario()),
            resolved_seed: None,
            generator_version: None,
            formation: compact_formation(&[UnitId(0)]),
            selected: Some(UnitId(0)),
            units: vec![LegacyUnitResume {
                id: UnitId(0),
                faction: Faction::Player,
                position: TilePos::ORIGIN,
                lattice: None,
                downed: false,
            }],
        }
    }

    fn campaign(slot: CampaignSlotId) -> CampaignSave {
        CampaignSave {
            slot,
            build_version: build_identity().to_owned(),
            scenario_name: "Party Trial".to_owned(),
            scenario_digest: scenario_digest(&scenario()),
            content_revision: Some(0xC0DE_CAFE),
            resolved_seed: None,
            generator_version: None,
            formation: compact_formation(&[UnitId(0)]),
            selected: Some(UnitId(0)),
            active_play_millis: 3_723_456,
            units: vec![CampaignUnitSave {
                id: UnitId(0),
                faction: Faction::Player,
                position: TilePos::ORIGIN,
                archetype: "hedge-mage".to_owned(),
                lattice: None,
                downed: false,
                display_name: "Hedge Mage".to_owned(),
            }],
        }
    }

    fn shipped_lattice_tables() -> (ElementCatalog, LatticeLibrary) {
        let element_file: ElementFile =
            ron::from_str(include_str!("../../../assets/config/elements.ron"))
                .expect("the shipped elements should parse");
        let spell_file: SpellFile =
            ron::from_str(include_str!("../../../assets/config/spells.ron"))
                .expect("the shipped spells should parse");
        let lattice_file: LatticeFile =
            ron::from_str(include_str!("../../../assets/config/lattices.ron"))
                .expect("the shipped lattices should parse");
        let elements = ElementCatalog::from_file(&element_file);
        let spells = SpellBook::from_file(&spell_file);
        let lattices = LatticeLibrary::build(&lattice_file, &elements, &spells)
            .expect("the shipped lattices should resolve");
        (elements, lattices)
    }

    fn insert_coherent_content(app: &mut App, spell_file: SpellFile) {
        let element_file: ElementFile =
            ron::from_str(include_str!("../../../assets/config/elements.ron"))
                .expect("the shipped elements should parse");
        let lattice_file: LatticeFile =
            ron::from_str(include_str!("../../../assets/config/lattices.ron"))
                .expect("the shipped lattices should parse");
        let substance_file: SubstanceFile =
            ron::from_str(include_str!("../../../assets/config/substances.ron"))
                .expect("the shipped substances should parse");
        let terrain_damage_file: TerrainDamageFile =
            ron::from_str(include_str!("../../../assets/config/terrain_damage.ron"))
                .expect("the shipped terrain-damage policy should parse");
        let palette: ArtPalette = ron::from_str(include_str!("../../../assets/art/palette.ron"))
            .expect("the shipped palette should parse");
        let elements = ElementCatalog::from_file(&element_file);
        let spells = SpellBook::from_file(&spell_file);
        let substances = SubstanceTable::from_file(&substance_file, &palette)
            .expect("the shipped substances should resolve");
        let terrain_damage =
            TerrainDamageTable::from_file(&terrain_damage_file, &elements, &substances)
                .expect("the shipped terrain-damage policy should resolve");
        let content = ContentIndex::build(&elements, &spells, &substances)
            .expect("the shipped content should resolve");
        let lattices = LatticeLibrary::build(&lattice_file, &elements, &spells)
            .expect("the shipped lattices should resolve");
        app.insert_resource(element_file)
            .insert_resource(elements)
            .insert_resource(spell_file)
            .insert_resource(spells)
            .insert_resource(substance_file)
            .insert_resource(palette)
            .insert_resource(substances)
            .insert_resource(terrain_damage_file)
            .insert_resource(terrain_damage)
            .insert_resource(lattice_file)
            .insert_resource(content)
            .insert_resource(lattices);
    }

    fn scratch_root(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "hex-game-campaign-{label}-{}-{unique}",
            std::process::id()
        ))
    }

    fn assert_elemental_fixture_matches_fresh_party_trial(
        pending: Res<PendingCampaign>,
        encounter: Res<Encounter>,
        formation: Res<PartyFormation>,
        party: Res<hex_units::Party>,
        units: Query<(
            &UnitId,
            &Faction,
            &UnitArchetype,
            &StandsOn,
            &LatticeState,
            Has<Downed>,
        )>,
    ) {
        let save = &pending.0;
        assert_eq!(encounter.name, save.scenario_name);

        let declarations = encounter
            .entries()
            .enumerate()
            .map(|(index, entry)| {
                (
                    UnitId(u64::try_from(index).expect("the shipped roster is tiny")),
                    (entry.faction, entry.archetype.to_owned()),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let runtime_count = units.iter().count();
        let runtime = units
            .iter()
            .map(|(id, faction, archetype, standing, lattice, downed)| {
                (
                    *id,
                    (
                        *faction,
                        archetype.0.clone(),
                        standing.0.pos,
                        lattice.clone(),
                        downed,
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>();

        assert_eq!(
            runtime.len(),
            runtime_count,
            "the production Party Trial spawn must deal unique unit ids"
        );
        assert_eq!(declarations.len(), save.units.len());
        assert_eq!(runtime.len(), save.units.len());
        assert_eq!(
            save.selected,
            party.members.first().copied(),
            "the fixture selection must match the fresh Party Trial selection target"
        );
        for snapshot in &save.units {
            let (declared_faction, declared_archetype) = declarations
                .get(&snapshot.id)
                .expect("every saved id comes from the shipped encounter declaration");
            let (runtime_faction, runtime_archetype, position, lattice, downed) = runtime
                .get(&snapshot.id)
                .expect("every saved id was spawned by the production unit plugin");
            assert_eq!(runtime_faction, declared_faction);
            assert_eq!(runtime_archetype, declared_archetype);
            assert_eq!(*runtime_faction, snapshot.faction);
            assert_eq!(*position, snapshot.position);
            assert_eq!(Some(lattice), snapshot.lattice.as_ref());
            assert_eq!(*downed, snapshot.downed);
        }
        assert_eq!(
            *formation, save.formation,
            "the fixture formation must be the production Party Trial formation"
        );
    }

    #[test]
    fn campaigns_round_trip_has_exactly_three_explicit_slots_and_unit_zero() {
        let mut original = CampaignsFile::default();
        original.slots[0] = Some(campaign(CampaignSlotId::One));
        original.slots[2] = Some(campaign(CampaignSlotId::Three));
        let text = encode_campaigns(&original).expect("campaigns should encode");
        let decoded: CampaignsFile = ron::from_str(&text).expect("campaigns should decode");
        assert_eq!(decoded, original);
        assert_eq!(decoded.slots.len(), 3);
        assert_eq!(
            decoded.slots[0].as_ref().and_then(|save| save.selected),
            Some(UnitId(0))
        );
        assert_eq!(
            validate_campaign_save(
                decoded.slots[0].as_ref().expect("slot one is occupied"),
                CampaignSlotId::One,
            ),
            Ok(())
        );
    }

    #[test]
    fn version_one_document_remains_readable_until_its_selected_slot_is_upgraded() {
        let mut version_one = CampaignsFile {
            format_version: CAMPAIGNS_VERSION_V1,
            ..Default::default()
        };
        version_one.slots[0] = Some(campaign(CampaignSlotId::One));
        let encoded = encode_campaigns(&version_one).expect("V1 document should encode");
        let decoded = decode_campaigns(&encoded);

        assert!(matches!(
            decoded.record(CampaignSlotId::One),
            Ok(Some(CampaignRecord::V1(_)))
        ));
        assert!(matches!(decoded.record(CampaignSlotId::Two), Ok(None)));
        assert!(decoded.unreadable.is_none());
    }

    #[test]
    fn document_rejects_v1_v2_overlap_and_v2_data_under_a_v1_header() {
        let empty_world = WorldSnapshotV1 {
            version: WORLD_SNAPSHOT_VERSION_V1,
            public_fingerprint: PublicWorldFingerprint(0),
            columns: BoundedVec::default(),
            damage: BoundedVec::default(),
            anchors: BoundedVec::default(),
            interior_surfaces: BoundedVec::default(),
            interior_roofs: BoundedVec::default(),
            special_regions: BoundedVec::default(),
            biome_regions: BoundedVec::default(),
            blockers: BoundedVec::default(),
            view_hint: None,
            lights: BoundedVec::default(),
            liquids: BoundedVec::default(),
            objects: BoundedVec::default(),
        };
        let v2 = CampaignSaveV2 {
            slot: CampaignSlotId::One,
            checkpoint: HostCampaignCheckpointV2 {
                version: CAMPAIGN_CHECKPOINT_VERSION_V2,
                build: crate::screens::multiplayer::local_build_identity()
                    .expect("test build identity fits"),
                content_fingerprint: ContentFingerprint(1),
                scenario_identity: BoundedText::new("fixture".to_owned()).expect("identity fits"),
                scenario_digest: 2,
                map_catalog_identity: BoundedText::new("map".to_owned()).expect("identity fits"),
                generator_identity: BoundedText::new("generator".to_owned())
                    .expect("identity fits"),
                generator_version: 1,
                resolved_seed: None,
                rules: RulesManifestV1 {
                    profile_identity: BoundedText::new("rules".to_owned()).expect("identity fits"),
                    fingerprint: 3,
                },
                simulation_seeds: SimSeeds::default(),
                world: empty_world,
                units: BoundedVec::default(),
                effects: CampaignEffectLedgerV2::default(),
                formation: PartyFormation::default(),
                active_play_millis: 0,
            },
        };
        let mut overlap = CampaignsFile::default();
        overlap.slots[0] = Some(campaign(CampaignSlotId::One));
        overlap.v2_slots[0] = Some(v2.clone());
        assert_eq!(
            campaigns_file_refusal(&overlap).as_deref(),
            Some("Campaign data contains two records for one slot.")
        );

        let mut wrong_header = CampaignsFile {
            format_version: CAMPAIGNS_VERSION_V1,
            ..Default::default()
        };
        wrong_header.v2_slots[0] = Some(v2);
        assert_eq!(
            campaigns_file_refusal(&wrong_header).as_deref(),
            Some("Campaign format 1 contains an impossible V2 checkpoint.")
        );
    }

    #[test]
    fn manual_save_serializes_the_bound_slot_with_exact_character_state_and_active_time() {
        let (_, lattices) = shipped_lattice_tables();
        let archetype = lattices
            .get("hedge-mage")
            .expect("the shipped Hedge Mage should resolve");
        let lattice = LatticeState::new(&archetype.spec, &archetype.stats);
        let root = scratch_root("system-save");
        let paths = StoragePaths::under(&root);
        let slot_one = campaign(CampaignSlotId::One);
        let slot_three = campaign(CampaignSlotId::Three);
        let mut file = CampaignsFile::default();
        file.slots[0] = Some(slot_one.clone());
        file.slots[2] = Some(slot_three.clone());
        let store = CampaignStore {
            file: Some(file),
            unreadable: None,
            runtime_invalid: std::array::from_fn(|_| None),
            catalog_invalid: std::array::from_fn(|_| None),
        };
        let accepted_fingerprint = 0xC0DE_CAFE;
        let mut active = ActiveCampaign::new(CampaignSlotId::Two, accepted_fingerprint, 12_000);
        active.session_active_play = Duration::from_millis(345);
        let scenario = scenario();
        let position = TilePos::ORIGIN;
        let checkpoint = HostCampaignCheckpointV2 {
            version: CAMPAIGN_CHECKPOINT_VERSION_V2,
            build: crate::screens::multiplayer::local_build_identity()
                .expect("test build identity fits"),
            content_fingerprint: ContentFingerprint(accepted_fingerprint),
            scenario_identity: BoundedText::new(scenario.name.clone())
                .expect("scenario identity fits"),
            scenario_digest: scenario_digest(&scenario),
            map_catalog_identity: BoundedText::new(scenario.world.clone())
                .expect("map identity fits"),
            generator_identity: BoundedText::new("showcase".to_owned())
                .expect("generator identity fits"),
            generator_version: 1,
            resolved_seed: None,
            rules: RulesManifestV1 {
                profile_identity: BoundedText::new("campaign-rules-v1".to_owned())
                    .expect("rules identity fits"),
                fingerprint: 7,
            },
            simulation_seeds: SimSeeds {
                world: 1,
                ai_flavor: 2,
                cosmetic: 3,
            },
            world: WorldSnapshotV1 {
                version: WORLD_SNAPSHOT_VERSION_V1,
                public_fingerprint: PublicWorldFingerprint(9),
                columns: BoundedVec::new(vec![WorldColumnSnapshotV1 {
                    coord: position.coord,
                    runs: BoundedVec::new(vec![WorldRunSnapshotV1 {
                        position,
                        run_bottom: 0,
                        span_bottom_bits: 0.0_f32.to_bits(),
                        span_top_bits: 1.0_f32.to_bits(),
                        substance: BoundedText::new("stone".to_owned())
                            .expect("substance identity fits"),
                        headroom: hex_core::MAX_HEADROOM,
                    }])
                    .expect("one run fits"),
                }])
                .expect("one column fits"),
                damage: BoundedVec::default(),
                anchors: BoundedVec::default(),
                interior_surfaces: BoundedVec::default(),
                interior_roofs: BoundedVec::default(),
                special_regions: BoundedVec::default(),
                biome_regions: BoundedVec::default(),
                blockers: BoundedVec::default(),
                view_hint: None,
                lights: BoundedVec::default(),
                liquids: BoundedVec::default(),
                objects: BoundedVec::default(),
            },
            units: BoundedVec::new(vec![CampaignUnitCheckpointV2 {
                unit: UnitId(0),
                faction: Faction::Player,
                archetype_identity: BoundedText::new("hedge-mage".to_owned())
                    .expect("archetype identity fits"),
                position,
                lattice: Some(lattice.clone()),
                downed: false,
                display_name: BoundedText::new("Saved Hedge Mage".to_owned())
                    .expect("display name fits"),
            }])
            .expect("one unit fits"),
            effects: CampaignEffectLedgerV2::default(),
            formation: compact_formation(&[UnitId(0)]),
            active_play_millis: 12_345,
        };
        checkpoint.validate().expect("checkpoint should be valid");

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(active)
            .insert_resource(paths.clone())
            .insert_resource(store)
            .insert_resource(CampaignSaveNotice::default())
            .insert_resource(CampaignSaveStatusProjection::default())
            .insert_resource(CampaignSaveRuntime {
                next_operation_id: 1,
                pending: Some(PendingCampaignWriteV2 {
                    operation_id: 1,
                    slot: CampaignSlotId::Two,
                    save: CampaignSaveV2 {
                        slot: CampaignSlotId::Two,
                        checkpoint: checkpoint.clone(),
                    },
                }),
            })
            .add_systems(Update, commit_pending_campaign_save);

        publish_campaign_save_status(app.world_mut(), 1, CampaignSaveStateV2::Saving);
        assert_eq!(
            app.world().resource::<CampaignSaveStatusProjection>().state,
            Some(CampaignSaveStateV2::Saving),
            "the accepted operation must be observable before its atomic commit"
        );

        app.update();

        assert_eq!(
            app.world().resource::<CampaignSaveNotice>().0.as_deref(),
            Some("Campaign slot 2 saved.")
        );
        let persisted: CampaignsFile =
            ron::from_str(&read(&paths.campaigns).expect("the Campaign file should be written"))
                .expect("the Campaign file should decode");
        assert_eq!(
            persisted.slots.first().and_then(Option::as_ref),
            Some(&slot_one),
            "saving slot two must leave slot one byte-equivalent in the document"
        );
        assert_eq!(
            persisted.slots.get(2).and_then(Option::as_ref),
            Some(&slot_three),
            "saving slot two must leave slot three byte-equivalent in the document"
        );
        assert!(persisted.slots[1].is_none());
        let saved = persisted.v2_slots[1]
            .as_ref()
            .expect("the V2 save should occupy the bound slot two");
        assert_eq!(saved.slot, CampaignSlotId::Two);
        assert_eq!(saved.checkpoint.active_play_millis, 12_345);
        assert_eq!(
            saved.checkpoint.content_fingerprint,
            ContentFingerprint(accepted_fingerprint)
        );
        assert_eq!(saved.checkpoint.units.len(), 1);
        let saved_unit = saved
            .checkpoint
            .units
            .first()
            .expect("the exact saved player should remain present");
        assert_eq!(saved_unit.archetype_identity.as_str(), "hedge-mage");
        assert_eq!(saved_unit.display_name.as_str(), "Saved Hedge Mage");
        assert_eq!(saved_unit.lattice.as_ref(), Some(&lattice));
        let serialized_v2 = ron::to_string(saved).expect("the V2 record should serialize");
        assert!(!serialized_v2.contains("selected"));
        assert!(!serialized_v2.contains("credential"));
        assert!(!serialized_v2.contains("principal"));
        assert_eq!(
            app.world().resource::<CampaignSaveStatusProjection>().state,
            Some(CampaignSaveStateV2::Saved)
        );
        let active = app.world().resource::<ActiveCampaign>();
        assert_eq!(active.active_play_millis(), 12_345);
        assert_eq!(active.session_active_play, Duration::ZERO);
        std::fs::remove_dir_all(root).expect("scratch directory should clean up");
    }

    #[test]
    fn occupied_slot_projects_the_saved_character_lattice_and_infers_legacy_identity() {
        let (elements, lattices) = shipped_lattice_tables();
        let archetype = lattices
            .get("hedge-mage")
            .expect("the shipped Hedge Mage should resolve");
        let state = LatticeState::new(&archetype.spec, &archetype.stats);
        let expected_cells = archetype.spec.cells().count();
        let mut save = campaign(CampaignSlotId::One);
        save.units
            .first_mut()
            .expect("the Campaign fixture has one unit")
            .lattice = Some(state);
        let mut file = CampaignsFile::default();
        *file
            .slots
            .first_mut()
            .expect("Campaign has exactly three slots") = Some(save.clone());
        let store = CampaignStore {
            file: Some(file),
            unreadable: None,
            runtime_invalid: std::array::from_fn(|_| None),
            catalog_invalid: std::array::from_fn(|_| None),
        };

        let explicit = store.slot_views(Some(&lattices), Some(&elements));
        let CampaignSlotStatusView::Available { party, .. } = &explicit
            .first()
            .expect("Campaign projects exactly three slots")
            .status
        else {
            panic!("slot one should be available");
        };
        let member = party.first().expect("the saved party has one member");
        assert_eq!(member.cells.len(), expected_cells);
        assert!(member
            .cells
            .iter()
            .any(|cell| cell.kind == SandboxLatticeCellKind::Spell));
        let explicit_cells = member.cells.clone();

        let mut legacy = save;
        legacy
            .units
            .first_mut()
            .expect("the Campaign fixture has one unit")
            .archetype
            .clear();
        let mut legacy_file = CampaignsFile::default();
        *legacy_file
            .slots
            .first_mut()
            .expect("Campaign has exactly three slots") = Some(legacy);
        let legacy_store = CampaignStore {
            file: Some(legacy_file),
            unreadable: None,
            runtime_invalid: std::array::from_fn(|_| None),
            catalog_invalid: std::array::from_fn(|_| None),
        };
        let inferred = legacy_store.slot_views(Some(&lattices), Some(&elements));
        let CampaignSlotStatusView::Available { party, .. } = &inferred
            .first()
            .expect("Campaign projects exactly three slots")
            .status
        else {
            panic!("legacy slot one should remain available");
        };
        assert_eq!(
            party
                .first()
                .expect("the inferred party has one member")
                .cells,
            explicit_cells
        );
        assert_eq!(
            party
                .first()
                .expect("the inferred party has one member")
                .name,
            "Hedge Mage"
        );
    }

    #[test]
    fn legacy_party_trial_projects_exact_inferred_roster_names() {
        let (elements, lattices) = shipped_lattice_tables();
        let party_trial = [
            (UnitId(0), "hedge-mage", hex_core::HexCoord::ORIGIN),
            (UnitId(1), "raider", hex_core::HexCoord::from_axial(1, 0)),
            (UnitId(2), "wolf", hex_core::HexCoord::from_axial(2, 0)),
        ];
        let player_ids = party_trial.iter().map(|(id, _, _)| *id).collect::<Vec<_>>();
        let units = party_trial
            .iter()
            .map(|(id, name, position)| {
                let definition = lattices
                    .get(name)
                    .expect("the Party Trial archetype should remain shipped");
                LegacyUnitResume {
                    id: *id,
                    faction: Faction::Player,
                    position: TilePos::new(*position, 0),
                    lattice: Some(LatticeState::new(&definition.spec, &definition.stats)),
                    downed: false,
                }
            })
            .collect();
        let mut legacy = legacy_resume();
        legacy.formation = compact_formation(&player_ids);
        legacy.units = units;
        let save = CampaignSave::from_legacy(legacy);
        assert_eq!(
            save.units
                .iter()
                .map(|unit| unit.display_name.as_str())
                .collect::<Vec<_>>(),
            ["Unit 0", "Unit 1", "Unit 2"],
            "the projection must replace the migration-only fallback labels"
        );
        let mut file = CampaignsFile::default();
        *file
            .slots
            .first_mut()
            .expect("Campaign has exactly three slots") = Some(save);
        let store = CampaignStore {
            file: Some(file),
            unreadable: None,
            runtime_invalid: std::array::from_fn(|_| None),
            catalog_invalid: std::array::from_fn(|_| None),
        };

        let views = store.slot_views(Some(&lattices), Some(&elements));
        let CampaignSlotStatusView::Available { party, .. } = &views
            .first()
            .expect("Campaign projects exactly three slots")
            .status
        else {
            panic!("the migrated Party Trial should remain available");
        };
        assert_eq!(
            party
                .iter()
                .map(|member| member.name.as_str())
                .collect::<Vec<_>>(),
            ["Hedge Mage", "Raider", "Wolf"]
        );
    }

    #[test]
    fn same_archetype_with_another_lattice_shape_is_refused_before_restore() {
        let spell_file: SpellFile =
            ron::from_str(include_str!("../../../assets/config/spells.ron"))
                .expect("the shipped spells should parse");
        let mut app = App::new();
        insert_coherent_content(&mut app, spell_file);
        let world = app.world();
        let lattices = world.resource::<LatticeLibrary>();
        let raider = lattices
            .get("raider")
            .expect("the shipped Raider should resolve");
        let wrong_state = LatticeState::new(&raider.spec, &raider.stats);
        let mut snapshot = campaign(CampaignSlotId::One)
            .units
            .into_iter()
            .next()
            .expect("the Campaign fixture has one unit");
        snapshot.lattice = Some(wrong_state);
        let runtime = UnitArchetype("hedge-mage".to_owned());
        let tables = world
            .resource::<ContentIndex>()
            .tables(world.resource::<ElementCatalog>());

        let refusal = campaign_lattice_refusal(&snapshot, &runtime, lattices, &tables)
            .expect("the wrong lattice shape must be refused");
        assert!(refusal.contains("gem coordinates do not match"));
    }

    #[test]
    fn pending_archetype_mismatch_is_refused_before_any_world_mutation() {
        let spell_file: SpellFile =
            ron::from_str(include_str!("../../../assets/config/spells.ron"))
                .expect("the shipped spells should parse");
        let map: MapSettings = ron::from_str(include_str!("../../../assets/config/world.ron"))
            .expect("the shipped authored map should parse");
        let formations: FormationCatalog =
            ron::from_str(include_str!("../../../assets/config/formations.ron"))
                .expect("the shipped formations should parse");
        let (_, lattices) = shipped_lattice_tables();
        let archetype = lattices
            .get("hedge-mage")
            .expect("the shipped Hedge Mage should resolve");
        let lattice = LatticeState::new(&archetype.spec, &archetype.stats);
        let runtime_standing = Standing {
            pos: TilePos {
                coord: hex_core::HexCoord::from_axial(1, 0),
                level: 0,
            },
            span: HexSpan::new(1.0, 2.0),
        };
        let runtime_transform = Transform::from_translation(Vec3::new(9.0, 8.0, 7.0));
        let original_formation = PartyFormation::default();
        let mut pending = campaign(CampaignSlotId::One);
        let snapshot = pending
            .units
            .first_mut()
            .expect("the Campaign fixture has one unit");
        snapshot.archetype = "raider".to_owned();
        snapshot.lattice = Some(lattice.clone());

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .insert_state(Screen::Gameplay)
            .insert_resource(map)
            .insert_resource(formations)
            .insert_resource(original_formation.clone())
            .insert_resource(CampaignStore::default())
            .insert_resource(PendingCampaign(pending))
            .insert_resource(ActiveCampaign::new(CampaignSlotId::One, 0xC0DE_CAFE, 0))
            .add_systems(Update, restore_pending_campaign);
        insert_coherent_content(&mut app, spell_file);
        app.world_mut()
            .spawn((HexTile, TilePos::ORIGIN, HexSpan::new(0.0, 1.0)));
        let unit = app
            .world_mut()
            .spawn((
                UnitId(0),
                Faction::Player,
                UnitArchetype("hedge-mage".to_owned()),
                StandsOn(runtime_standing),
                runtime_transform,
                lattice.clone(),
                Downed,
            ))
            .id();

        app.update();

        let failure = app.world().resource::<GameplaySetupFailure>();
        assert!(failure
            .reason
            .contains("saved archetype for unit 0 changed"));
        assert!(!app.world().contains_resource::<PendingCampaign>());
        assert!(!app.world().contains_resource::<ActiveCampaign>());
        assert_eq!(
            *app.world().resource::<PartyFormation>(),
            original_formation,
            "formation restore must not begin after a preflight refusal"
        );
        let entity = app.world().entity(unit);
        assert_eq!(
            entity
                .get::<StandsOn>()
                .expect("the runtime unit should remain spawned")
                .0,
            runtime_standing
        );
        let transform = entity
            .get::<Transform>()
            .expect("the runtime transform should remain present");
        assert_eq!(transform.translation, runtime_transform.translation);
        assert_eq!(transform.rotation, runtime_transform.rotation);
        assert_eq!(transform.scale, runtime_transform.scale);
        assert_eq!(entity.get::<LatticeState>(), Some(&lattice));
        assert!(entity.contains::<Downed>());
        assert!(!entity.contains::<Selected>());
    }

    #[test]
    fn invalid_saved_footing_is_refused_atomically_for_every_canonical_case() {
        for (case, substance_name, headroom, blocked) in [
            ("non-solid", "water", Headroom(8), false),
            ("buried", "stone", Headroom(0), false),
            ("cramped", "stone", Headroom(1), false),
            ("blocked", "stone", Headroom(8), true),
        ] {
            let spell_file: SpellFile =
                ron::from_str(include_str!("../../../assets/config/spells.ron"))
                    .expect("the shipped spells should parse");
            let map: MapSettings = ron::from_str(include_str!("../../../assets/config/world.ron"))
                .expect("the shipped authored map should parse");
            let formations: FormationCatalog =
                ron::from_str(include_str!("../../../assets/config/formations.ron"))
                    .expect("the shipped formations should parse");
            let saved_player_position = TilePos::ORIGIN;
            let saved_hostile_position = TilePos::new(hex_core::HexCoord::from_axial(1, 0), 0);
            let runtime_player_standing = Standing {
                pos: TilePos::new(hex_core::HexCoord::from_axial(3, 0), 0),
                span: HexSpan::new(3.0, 4.0),
            };
            let runtime_hostile_standing = Standing {
                pos: TilePos::new(hex_core::HexCoord::from_axial(4, 0), 0),
                span: HexSpan::new(4.0, 5.0),
            };
            let runtime_player_transform = Transform::from_translation(Vec3::new(9.0, 8.0, 7.0));
            let runtime_hostile_transform = Transform::from_translation(Vec3::new(6.0, 5.0, 4.0));
            let original_formation = PartyFormation::default();
            let mut pending = campaign(CampaignSlotId::One);
            pending
                .units
                .first_mut()
                .expect("the Campaign fixture has one player")
                .position = saved_player_position;
            pending.units.push(CampaignUnitSave {
                id: UnitId(1),
                faction: Faction::Hostile,
                position: saved_hostile_position,
                archetype: "raider".to_owned(),
                lattice: None,
                downed: true,
                display_name: "Raider".to_owned(),
            });

            let mut app = App::new();
            app.add_plugins((MinimalPlugins, StatesPlugin))
                .insert_state(Screen::Gameplay)
                .insert_resource(map)
                .insert_resource(formations)
                .insert_resource(original_formation.clone())
                .insert_resource(CampaignStore::default())
                .insert_resource(PendingCampaign(pending))
                .insert_resource(ActiveCampaign::new(CampaignSlotId::One, 0xC0DE_CAFE, 0))
                .add_systems(Update, restore_pending_campaign);
            insert_coherent_content(&mut app, spell_file);
            let (stone, invalid_substance) = {
                let substances = app.world().resource::<SubstanceTable>();
                (
                    substances
                        .id("stone")
                        .expect("the shipped stone should resolve"),
                    substances
                        .id(substance_name)
                        .expect("the invalid-case substance should resolve"),
                )
            };
            app.world_mut().spawn((
                HexTile,
                saved_player_position,
                HexSpan::new(0.0, 1.0),
                stone,
                Headroom(8),
            ));
            app.world_mut().spawn((
                HexTile,
                saved_hostile_position,
                HexSpan::new(0.0, 1.0),
                invalid_substance,
                headroom,
            ));
            if blocked {
                let mut blockers = TraversalBlockers::new();
                assert!(blockers.insert(saved_hostile_position));
                app.insert_resource(blockers);
            }
            let player = app
                .world_mut()
                .spawn((
                    UnitId(0),
                    Faction::Player,
                    UnitArchetype("hedge-mage".to_owned()),
                    StandsOn(runtime_player_standing),
                    runtime_player_transform,
                    Downed,
                ))
                .id();
            let hostile = app
                .world_mut()
                .spawn((
                    UnitId(1),
                    Faction::Hostile,
                    UnitArchetype("raider".to_owned()),
                    StandsOn(runtime_hostile_standing),
                    runtime_hostile_transform,
                    Selected,
                ))
                .id();

            app.update();

            assert_eq!(
                app.world().resource::<GameplaySetupFailure>().reason,
                "Campaign slot 1 is incompatible: The saved position for unit 1 is no longer valid footing.",
                "{case} footing should be refused visibly"
            );
            assert!(
                !app.world().contains_resource::<PendingCampaign>(),
                "{case}"
            );
            assert!(!app.world().contains_resource::<ActiveCampaign>(), "{case}");
            assert_eq!(
                *app.world().resource::<PartyFormation>(),
                original_formation,
                "{case} refusal must precede formation mutation"
            );
            let player = app.world().entity(player);
            assert_eq!(
                player
                    .get::<StandsOn>()
                    .expect("the player should remain spawned")
                    .0,
                runtime_player_standing,
                "{case} refusal must preserve an otherwise-restorable unit"
            );
            assert_eq!(
                player
                    .get::<Transform>()
                    .expect("the player transform should remain present")
                    .translation,
                runtime_player_transform.translation,
                "{case} refusal must preserve the player transform"
            );
            assert!(player.contains::<Downed>(), "{case}");
            assert!(!player.contains::<Selected>(), "{case}");
            let hostile = app.world().entity(hostile);
            assert_eq!(
                hostile
                    .get::<StandsOn>()
                    .expect("the hostile should remain spawned")
                    .0,
                runtime_hostile_standing,
                "{case} refusal must preserve the invalid unit"
            );
            assert_eq!(
                hostile
                    .get::<Transform>()
                    .expect("the hostile transform should remain present")
                    .translation,
                runtime_hostile_transform.translation,
                "{case} refusal must preserve the hostile transform"
            );
            assert!(!hostile.contains::<Downed>(), "{case}");
            assert!(hostile.contains::<Selected>(), "{case}");
        }
    }

    #[test]
    fn campaign_intent_without_loaded_catalogs_publishes_a_visible_refusal() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .insert_state(Screen::Title)
            .insert_resource(CampaignStore::default())
            .add_message::<UiIntent>()
            .add_systems(Update, handle_campaign_intents);
        app.world_mut()
            .write_message(UiIntent::MainMenu(MainMenuIntent::NewCampaign(
                CampaignSlotId::One,
            )));

        app.update();

        assert_eq!(
            app.world().resource::<GameplaySetupFailure>().reason,
            "Campaign content is still loading."
        );
        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Title
        );
    }

    #[test]
    fn temporary_loading_content_cannot_poison_a_campaign_slot() {
        let library: ScenarioLibrary =
            ron::from_str(include_str!("../../../assets/config/scenarios.ron"))
                .expect("the shipped scenario library should parse");
        let formations: FormationCatalog =
            ron::from_str(include_str!("../../../assets/config/formations.ron"))
                .expect("the shipped formations should parse");
        let shipped_spells: SpellFile =
            ron::from_str(include_str!("../../../assets/config/spells.ron"))
                .expect("the shipped spells should parse");
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .insert_state(Screen::Title)
            .add_plugins(hex_assets::content_index::plugin)
            .insert_resource(CampaignStore::default())
            .insert_resource(library)
            .insert_resource(formations)
            .add_systems(
                Update,
                validate_campaign_catalog.run_if(in_state(Screen::Title)),
            );
        insert_coherent_content(&mut app, shipped_spells.clone());
        app.update();
        let shipped_fingerprint = app
            .world()
            .resource::<AcceptedContentRevision>()
            .fingerprint();
        let mut save = campaign(CampaignSlotId::One);
        save.content_revision = Some(shipped_fingerprint);
        app.world_mut()
            .resource_mut::<CampaignStore>()
            .file
            .as_mut()
            .expect("the store should be readable")
            .slots
            .first_mut()
            .expect("Campaign has exactly three slots")
            .replace(save);
        app.update();
        assert!(app
            .world()
            .resource::<CampaignStore>()
            .available(CampaignSlotId::One)
            .is_some());

        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Loading);
        app.update();
        let mut creator_spells = shipped_spells.clone();
        let ember = creator_spells
            .spells
            .get_mut("Ember")
            .expect("the shipped Ember spell should exist");
        ember.targeting.range = ember.targeting.range.saturating_add(1);
        insert_coherent_content(&mut app, creator_spells);
        app.update();
        let temporary_fingerprint = app
            .world()
            .resource::<AcceptedContentRevision>()
            .fingerprint();
        assert_ne!(temporary_fingerprint, shipped_fingerprint);
        assert!(app
            .world()
            .resource::<CampaignStore>()
            .available(CampaignSlotId::One)
            .is_some());

        insert_coherent_content(&mut app, shipped_spells);
        app.update();
        assert_eq!(
            app.world()
                .resource::<AcceptedContentRevision>()
                .fingerprint(),
            shipped_fingerprint
        );
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Title);
        app.update();
        assert!(app
            .world()
            .resource::<CampaignStore>()
            .available(CampaignSlotId::One)
            .is_some());
    }

    #[test]
    fn catalog_repair_never_clears_a_sticky_restore_refusal() {
        let mut store = CampaignStore::default();
        store
            .file
            .as_mut()
            .expect("store is readable")
            .slots
            .first_mut()
            .expect("Campaign has exactly three slots")
            .replace(campaign(CampaignSlotId::One));
        store.mark_invalid(
            CampaignSlotId::One,
            "The saved roster no longer matches this scenario.".to_owned(),
        );
        *store
            .catalog_invalid
            .first_mut()
            .expect("Campaign has exactly three refusal slots") =
            Some("The saved authored content revision is incompatible.".to_owned());

        *store
            .catalog_invalid
            .first_mut()
            .expect("Campaign has exactly three refusal slots") = None;

        assert_eq!(
            store.slot(CampaignSlotId::One),
            Err("The saved roster no longer matches this scenario.".to_owned())
        );
    }

    #[test]
    fn preflight_catalog_refusal_clears_after_catalog_repair() {
        let mut store = CampaignStore::default();
        store
            .file
            .as_mut()
            .expect("store is readable")
            .slots
            .first_mut()
            .expect("Campaign has exactly three slots")
            .replace(campaign(CampaignSlotId::One));
        store.mark_catalog_invalid(
            CampaignSlotId::One,
            "The saved authored content revision is incompatible.".to_owned(),
        );
        assert!(store.available(CampaignSlotId::One).is_none());
        assert!(store.runtime_invalid.first().is_some_and(Option::is_none));

        *store
            .catalog_invalid
            .first_mut()
            .expect("Campaign has exactly three refusal slots") = None;

        assert!(store.available(CampaignSlotId::One).is_some());
    }

    #[test]
    fn corrupt_identity_positions_selection_formation_and_slot_are_refused() {
        let mut no_party = campaign(CampaignSlotId::One);
        no_party
            .units
            .first_mut()
            .expect("fixture has one unit")
            .faction = Faction::Hostile;
        no_party.selected = None;
        assert_eq!(
            validate_campaign_save(&no_party, CampaignSlotId::One),
            Err("Campaign data has no player party.".to_owned())
        );

        let mut duplicate = campaign(CampaignSlotId::One);
        let repeated = duplicate
            .units
            .first()
            .expect("fixture has one unit")
            .clone();
        duplicate.units.push(repeated);
        assert!(validate_campaign_save(&duplicate, CampaignSlotId::One).is_err());

        let mut overlapping = campaign(CampaignSlotId::One);
        overlapping.units.push(CampaignUnitSave {
            id: UnitId(1),
            faction: Faction::Hostile,
            position: TilePos::ORIGIN,
            archetype: "raider".to_owned(),
            lattice: None,
            downed: false,
            display_name: "Raider".to_owned(),
        });
        assert_eq!(
            validate_campaign_save(&overlapping, CampaignSlotId::One),
            Err("Campaign data places multiple units at one position.".to_owned())
        );

        let mut hostile_selected = campaign(CampaignSlotId::One);
        hostile_selected.units.push(CampaignUnitSave {
            id: UnitId(1),
            faction: Faction::Hostile,
            position: TilePos {
                coord: hex_core::HexCoord::from_axial(1, 0),
                level: 0,
            },
            archetype: "raider".to_owned(),
            lattice: None,
            downed: false,
            display_name: "Raider".to_owned(),
        });
        hostile_selected.selected = Some(UnitId(1));
        assert_eq!(
            validate_campaign_save(&hostile_selected, CampaignSlotId::One),
            Err("Campaign data selects a unit outside the player party.".to_owned())
        );

        let mut old_build = campaign(CampaignSlotId::One);
        old_build.build_version = "old".to_owned();
        assert!(validate_campaign_save(&old_build, CampaignSlotId::One).is_err());

        let wrong_slot = campaign(CampaignSlotId::Three);
        assert!(validate_campaign_save(&wrong_slot, CampaignSlotId::Two).is_err());

        let mut invalid_formation = campaign(CampaignSlotId::One);
        invalid_formation
            .formation
            .assignments
            .insert(UnitId(99), hex_core::HexCoord::ORIGIN);
        assert!(validate_campaign_save(&invalid_formation, CampaignSlotId::One).is_err());

        let mut missing_preset = campaign(CampaignSlotId::One);
        missing_preset.formation.preset = "Missing".to_owned();
        assert_eq!(
            validate_campaign_save(&missing_preset, CampaignSlotId::One),
            Err("Campaign formation preset \"Missing\" is unavailable.".to_owned())
        );

        let mut foreign_offset = campaign(CampaignSlotId::One);
        foreign_offset
            .formation
            .assignments
            .insert(UnitId(0), hex_core::HexCoord::from_axial(9, 9));
        assert!(validate_campaign_save(&foreign_offset, CampaignSlotId::One)
            .expect_err("an unauthored offset must be refused")
            .contains("outside preset"));
    }

    #[test]
    fn legacy_resume_migrates_to_slot_one_with_zero_elapsed_time() {
        let migrated = CampaignSave::from_legacy(legacy_resume());
        assert_eq!(migrated.slot, CampaignSlotId::One);
        assert_eq!(migrated.active_play_millis, 0);
        assert_eq!(migrated.content_revision, None);
        assert_eq!(migrated.selected, Some(UnitId(0)));
        assert_eq!(
            validate_campaign_save(&migrated, CampaignSlotId::One),
            Ok(())
        );
    }

    #[test]
    fn pr175_digest_translation_is_retired_after_elemental_and_world_cutovers() {
        let legacy_text = include_str!("../testdata/legacy_resume_pr175.ron");
        let legacy: LegacyResumeFile =
            ron::from_str(legacy_text).expect("the historical PR #175 resume should parse");
        assert_eq!(legacy.build_version, build_identity());
        assert_eq!(legacy.scenario_digest, 0xC8EA_6229_346D_CF96);
        validate_legacy_resume(&legacy).expect("the PR #175 record itself remains well formed");

        let library: ScenarioLibrary =
            ron::from_str(include_str!("../../../assets/config/scenarios.ron"))
                .expect("the current scenario library should parse");
        for (name, legacy_digest, cutover_digest) in LEGACY_RESUME_DIGESTS {
            let current = library
                .scenarios
                .iter()
                .find(|current| current.name == *name)
                .expect("every PR #175 scenario has an explicit digest translation");
            assert!(legacy_resume_digest_is_compatible(
                &current.name,
                *legacy_digest,
                *cutover_digest,
            ));
            let live_digest = scenario_digest(current);
            assert_ne!(
                live_digest, *cutover_digest,
                "{} must reflect the later elemental and world cutovers",
                current.name
            );
            assert!(!legacy_resume_digest_is_compatible(
                &current.name,
                *legacy_digest,
                live_digest,
            ));
        }
        assert!(
            library
                .scenarios
                .iter()
                .any(|scenario| scenario.name == "Mountain Range")
                && !LEGACY_RESUME_DIGESTS
                    .iter()
                    .any(|(name, _, _)| *name == "Mountain Range"),
            "post-cutover scenarios must not fabricate a PR #175 legacy digest"
        );

        let current = library
            .scenarios
            .iter()
            .find(|candidate| candidate.name == legacy.scenario_name)
            .expect("Party Trial remains the canonical Campaign");
        assert_ne!(scenario_digest(current), 0xAA13_0315_396C_E50C);
        assert!(!legacy_resume_digest_is_compatible(
            &current.name,
            legacy.scenario_digest,
            scenario_digest(current),
        ));

        let root = scratch_root("pr175-historical");
        let paths = StoragePaths::under(&root);
        write_atomic(&paths.resume, legacy_text).expect("historical fixture should write");
        let store = migrate_legacy(&paths);
        let migrated = store
            .available(CampaignSlotId::One)
            .expect("the well-formed PR #175 record should remain preserved");
        let refusal = campaign_content_refusal(&migrated, &library, 0xC0DE_CAFE)
            .expect("the current cutovers must refuse incompatible PR #175 content");
        assert_eq!(
            refusal,
            "The saved scenario \"Party Trial\" changed and cannot be resumed."
        );
        assert_eq!(
            read(&paths.resume).expect("historical resume remains"),
            legacy_text
        );
        std::fs::remove_dir_all(root).expect("scratch directory should clean up");
    }

    #[test]
    fn elemental_example_resume_migrates_and_restores_as_compatible() {
        let example_text = include_str!("../testdata/example_resume_elemental_grid.ron");
        let example: LegacyResumeFile =
            ron::from_str(example_text).expect("the elemental example resume should parse");
        assert_eq!(example.build_version, build_identity());
        assert_eq!(example.scenario_digest, 0x5DF9_C632_EA7D_97D3);
        assert_eq!(example.units.len(), 6, "Party Trial is a complete 3v3");
        validate_legacy_resume(&example).expect("the elemental example record itself is valid");

        let library: ScenarioLibrary =
            ron::from_str(include_str!("../../../assets/config/scenarios.ron"))
                .expect("the elemental scenario library should parse");
        let current = library
            .scenarios
            .iter()
            .find(|candidate| candidate.name == example.scenario_name)
            .expect("Party Trial remains the canonical Campaign");
        assert_eq!(scenario_digest(current), example.scenario_digest);

        let root = scratch_root("elemental-grid-example");
        let paths = StoragePaths::under(&root);
        write_atomic(&paths.resume, example_text).expect("example fixture should write");
        let store = migrate_legacy(&paths);
        let migrated = store
            .available(CampaignSlotId::One)
            .expect("the elemental example should migrate to slot one")
            .clone();
        assert_eq!(migrated.scenario_digest, example.scenario_digest);
        assert_eq!(migrated.active_play_millis, 0);
        assert_eq!(migrated.content_revision, None);
        assert_eq!(
            campaign_content_refusal(&migrated, &library, 0xC0DE_CAFE),
            None
        );
        assert_eq!(read(&paths.resume).expect("example remains"), example_text);

        let expected_formation = migrated.formation.clone();
        let mut app =
            crate::scenarios::tests::procedural_gameplay_app_with_combat("Party Trial", true);
        app.insert_resource(CampaignStore::default())
            .insert_resource(PendingCampaign(migrated.clone()))
            .insert_resource(ActiveCampaign::new(CampaignSlotId::One, 0xC0DE_CAFE, 0))
            .add_systems(
                OnEnter(Screen::Gameplay),
                (
                    assert_elemental_fixture_matches_fresh_party_trial,
                    restore_pending_campaign,
                )
                    .chain()
                    .in_set(GameplaySetup::Restore),
            );

        crate::scenarios::tests::enter_screen(&mut app, Screen::Gameplay);

        assert!(!app.world().contains_resource::<PendingCampaign>());
        assert!(app.world().contains_resource::<ActiveCampaign>());
        assert!(!app.world().contains_resource::<GameplaySetupFailure>());
        assert_eq!(
            *app.world().resource::<PartyFormation>(),
            expected_formation
        );
        let restored = {
            let world = app.world_mut();
            let mut units = world.query::<(&UnitId, &StandsOn, &LatticeState, Option<&Selected>)>();
            units
                .iter(world)
                .map(|(id, standing, lattice, selected)| {
                    (*id, (standing.0.pos, lattice.clone(), selected.is_some()))
                })
                .collect::<BTreeMap<_, _>>()
        };
        assert_eq!(restored.len(), migrated.units.len());
        for snapshot in &migrated.units {
            let (position, lattice, selected) = restored
                .get(&snapshot.id)
                .expect("every saved Party Trial unit was restored");
            assert_eq!(*position, snapshot.position);
            assert_eq!(Some(lattice), snapshot.lattice.as_ref());
            assert_eq!(*selected, migrated.selected == Some(snapshot.id));
        }
        std::fs::remove_dir_all(root).expect("scratch directory should clean up");
    }

    #[test]
    fn pre_terrain_resume_is_preserved_as_invalid() {
        let legacy_text = include_str!("../testdata/legacy_resume_pre_terrain.ron");
        let legacy: LegacyResumeFile =
            ron::from_str(legacy_text).expect("the pre-terrain resume fixture should parse");
        assert_eq!(legacy.scenario_digest, 0xB392_E2D3_BC35_DBDB);
        validate_legacy_resume(&legacy).expect("the pre-terrain record itself is well formed");

        let library: ScenarioLibrary =
            ron::from_str(include_str!("../../../assets/config/scenarios.ron"))
                .expect("the cutover scenario library should parse");
        let current = library
            .scenarios
            .iter()
            .find(|candidate| candidate.name == legacy.scenario_name)
            .expect("Party Trial remains the canonical Campaign");
        assert!(!legacy_resume_digest_is_compatible(
            &current.name,
            legacy.scenario_digest,
            scenario_digest(current),
        ));

        let root = scratch_root("pre-terrain-legacy");
        let paths = StoragePaths::under(&root);
        write_atomic(&paths.resume, legacy_text).expect("legacy fixture should write");

        let mut store = migrate_legacy(&paths);
        let migrated = store
            .available(CampaignSlotId::One)
            .expect("migration should preserve the well-formed legacy record");
        assert_eq!(migrated.scenario_digest, legacy.scenario_digest);
        assert_eq!(migrated.content_revision, None);
        let refusal = campaign_content_refusal(&migrated, &library, 0xC0DE_CAFE)
            .expect("pre-terrain content must not be resumed");
        assert_eq!(
            refusal,
            "The saved scenario \"Party Trial\" changed and cannot be resumed."
        );
        store.mark_catalog_invalid(CampaignSlotId::One, refusal.clone());
        assert_eq!(store.slot(CampaignSlotId::One), Err(refusal.clone()));

        let campaigns_text = read(&paths.campaigns).expect("migration should persist campaigns");
        let mut reloaded = decode_campaigns(&campaigns_text);
        let persisted = reloaded
            .available(CampaignSlotId::One)
            .expect("the pre-terrain record should remain persisted");
        assert_eq!(persisted.scenario_digest, legacy.scenario_digest);
        reloaded.mark_catalog_invalid(CampaignSlotId::One, refusal.clone());
        assert_eq!(reloaded.slot(CampaignSlotId::One), Err(refusal));
        assert_eq!(read(&paths.resume).expect("legacy remains"), legacy_text);
        std::fs::remove_dir_all(root).expect("scratch directory should clean up");
    }

    #[test]
    fn migration_never_reads_writes_or_deletes_the_retired_report_file() {
        let root = scratch_root("legacy");
        let paths = StoragePaths::under(&root);
        let legacy_text =
            ron::ser::to_string_pretty(&legacy_resume(), ron::ser::PrettyConfig::new())
                .expect("legacy resume should encode");
        let report_sentinel = "existing report bytes";
        write_atomic(&paths.resume, &legacy_text).expect("legacy fixture should write");
        write_atomic(&paths.combat_reports, report_sentinel).expect("report sentinel should write");

        let (store, accesses) = crate::storage::record_storage_accesses(|| migrate_legacy(&paths));

        assert!(matches!(
            store.slot(CampaignSlotId::One),
            Ok(Some(CampaignSave {
                active_play_millis: 0,
                ..
            }))
        ));
        assert_eq!(read(&paths.resume).expect("legacy remains"), legacy_text);
        assert!(
            accesses.iter().all(|access| matches!(
                access.kind,
                crate::storage::StorageAccessKind::Read | crate::storage::StorageAccessKind::Write
            ) && access.path != paths.combat_reports),
            "retired report path was accessed: {accesses:?}"
        );
        assert_eq!(
            read(&paths.combat_reports).expect("report remains"),
            report_sentinel
        );
        assert!(read(&paths.campaigns).is_ok());
        std::fs::remove_dir_all(root).expect("scratch directory should clean up");
    }

    #[test]
    fn missing_legacy_resume_writes_a_one_time_empty_migration_marker() {
        let root = scratch_root("missing-legacy");
        let paths = StoragePaths::under(&root);

        let first = migrate_legacy(&paths);
        assert!(CampaignSlotId::ALL
            .into_iter()
            .all(|slot| matches!(first.slot(slot), Ok(None))));
        let marker = read(&paths.campaigns).expect("migration should write an empty marker");

        let late_legacy =
            ron::ser::to_string_pretty(&legacy_resume(), ron::ser::PrettyConfig::new())
                .expect("legacy resume should encode");
        write_atomic(&paths.resume, &late_legacy).expect("late legacy fixture should write");

        let second = decode_campaigns(
            &read(&paths.campaigns).expect("a later launch should prefer the Campaign marker"),
        );
        assert!(CampaignSlotId::ALL
            .into_iter()
            .all(|slot| matches!(second.slot(slot), Ok(None))));
        assert_eq!(
            read(&paths.campaigns).expect("marker should remain authoritative"),
            marker
        );
        assert_eq!(
            read(&paths.resume).expect("late legacy remains"),
            late_legacy
        );
        std::fs::remove_dir_all(root).expect("scratch directory should clean up");
    }

    #[test]
    fn invalid_legacy_data_remains_a_visible_slot_one_refusal() {
        let root = scratch_root("invalid-legacy");
        let paths = StoragePaths::under(&root);
        let mut legacy = legacy_resume();
        legacy.build_version = "old-build".to_owned();
        let legacy_text = ron::ser::to_string_pretty(&legacy, ron::ser::PrettyConfig::new())
            .expect("legacy resume should encode");
        write_atomic(&paths.resume, &legacy_text).expect("legacy fixture should write");

        let store = migrate_legacy(&paths);

        assert!(matches!(
            store
                .slot_views(None, None)
                .first()
                .map(|view| &view.status),
            Some(CampaignSlotStatusView::Invalid { .. })
        ));
        assert!(matches!(
            store.slot_views(None, None).get(1).map(|view| &view.status),
            Some(CampaignSlotStatusView::Empty)
        ));
        assert_eq!(read(&paths.resume).expect("legacy remains"), legacy_text);
        std::fs::remove_dir_all(root).expect("scratch directory should clean up");
    }

    #[test]
    fn first_save_then_same_slot_overwrite_preserves_occupied_siblings_atomically() {
        let root = scratch_root("atomic");
        let paths = StoragePaths::under(&root);
        let slot_one = campaign(CampaignSlotId::One);
        let slot_three = campaign(CampaignSlotId::Three);
        let mut file = CampaignsFile::default();
        file.slots[0] = Some(slot_one.clone());
        file.slots[2] = Some(slot_three.clone());
        let mut store = CampaignStore {
            file: Some(file),
            unreadable: None,
            runtime_invalid: std::array::from_fn(|_| None),
            catalog_invalid: std::array::from_fn(|_| None),
        };

        let _bound_without_save = ActiveCampaign::new(CampaignSlotId::Two, 0xC0DE_CAFE, 0);
        assert!(matches!(store.slot(CampaignSlotId::Two), Ok(None)));

        let mut first = campaign(CampaignSlotId::Two);
        first.active_play_millis = 1_000;
        store
            .write_slot(&paths, CampaignSlotId::Two, first)
            .expect("the first normal save should occupy slot two");

        let mut overwritten = campaign(CampaignSlotId::Two);
        overwritten.active_play_millis = 2_000;
        store
            .write_slot(&paths, CampaignSlotId::Two, overwritten.clone())
            .expect("a later save should overwrite only the bound slot");

        let decoded: CampaignsFile =
            ron::from_str(&read(&paths.campaigns).expect("campaigns should exist"))
                .expect("campaigns should decode");
        assert_eq!(decoded.slots.len(), 3);
        assert_eq!(
            decoded.slots.first().and_then(Option::as_ref),
            Some(&slot_one)
        );
        assert_eq!(
            decoded.slots.get(1).and_then(Option::as_ref),
            Some(&overwritten)
        );
        assert_eq!(
            decoded.slots.get(2).and_then(Option::as_ref),
            Some(&slot_three)
        );
        std::fs::remove_dir_all(root).expect("scratch directory should clean up");
    }

    #[test]
    fn mixed_empty_available_and_invalid_slots_project_independently() {
        let mut file = CampaignsFile::default();
        if let Some(slot) = file.slots.first_mut() {
            *slot = Some(campaign(CampaignSlotId::One));
        }
        if let Some(slot) = file.slots.get_mut(2) {
            *slot = Some(campaign(CampaignSlotId::One));
        }
        let store = CampaignStore {
            file: Some(file),
            unreadable: None,
            runtime_invalid: std::array::from_fn(|_| None),
            catalog_invalid: std::array::from_fn(|_| None),
        };
        let views = store.slot_views(None, None);
        assert_eq!(views.len(), 3);
        assert!(matches!(
            views.first().map(|view| &view.status),
            Some(CampaignSlotStatusView::Available { .. })
        ));
        assert!(matches!(
            views.get(1).map(|view| &view.status),
            Some(CampaignSlotStatusView::Empty)
        ));
        assert!(matches!(
            views.get(2).map(|view| &view.status),
            Some(CampaignSlotStatusView::Invalid { .. })
        ));

        let mut protected = store.clone();
        let before = protected.file.clone();
        assert!(protected
            .write_slot(
                &StoragePaths::under("unused-invalid-target"),
                CampaignSlotId::Three,
                campaign(CampaignSlotId::Three),
            )
            .is_err());
        assert_eq!(protected.file, before, "invalid data must not be replaced");
    }

    #[test]
    fn corrupt_or_incompatible_campaign_file_refuses_every_slot() {
        for store in [
            decode_campaigns("not ron"),
            decode_campaigns("(format_version:99,slots:(None,None,None))"),
        ] {
            assert!(store
                .slot_views(None, None)
                .iter()
                .all(|view| matches!(&view.status, CampaignSlotStatusView::Invalid { .. })));
        }
    }

    #[test]
    fn corrupt_pending_record_is_refused_before_world_restore() {
        let mut corrupt = campaign(CampaignSlotId::One);
        corrupt.units.push(CampaignUnitSave {
            id: UnitId(1),
            faction: Faction::Hostile,
            position: TilePos::ORIGIN,
            archetype: "raider".to_owned(),
            lattice: None,
            downed: false,
            display_name: "Raider".to_owned(),
        });

        let map: MapSettings = ron::from_str(include_str!("../../../assets/config/world.ron"))
            .expect("the shipped authored map should parse");
        let formations: FormationCatalog =
            ron::from_str(include_str!("../../../assets/config/formations.ron"))
                .expect("the shipped formations should parse");
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .insert_state(Screen::Gameplay)
            .insert_resource(map)
            .insert_resource(formations)
            .insert_resource(PartyFormation::default())
            .insert_resource(CampaignStore::default())
            .insert_resource(PendingCampaign(corrupt))
            .insert_resource(ActiveCampaign::new(CampaignSlotId::One, 0xC0DE_CAFE, 0))
            .add_systems(Update, restore_pending_campaign);
        let spell_file: SpellFile =
            ron::from_str(include_str!("../../../assets/config/spells.ron"))
                .expect("the shipped spells should parse");
        insert_coherent_content(&mut app, spell_file);

        app.update();
        app.update();

        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Title
        );
        assert!(!app.world().contains_resource::<PendingCampaign>());
        assert!(!app.world().contains_resource::<ActiveCampaign>());
        assert!(app
            .world()
            .resource::<GameplaySetupFailure>()
            .reason
            .contains("multiple units at one position"));
        assert!(matches!(
            app.world()
                .resource::<CampaignStore>()
                .slot_views(None, None)
                .first()
                .map(|view| &view.status),
            Some(CampaignSlotStatusView::Invalid { .. })
        ));
    }

    #[test]
    fn campaign_session_boundaries_clear_bound_state_and_save_notice() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .insert_state(Screen::Gameplay)
            .insert_resource(CampaignSaveNotice(Some(
                "Campaign slot 1 saved.".to_owned(),
            )))
            .insert_resource(PendingCampaign(campaign(CampaignSlotId::One)))
            .insert_resource(ActiveCampaign::new(CampaignSlotId::One, 0xC0DE_CAFE, 0))
            .insert_resource(GameplaySessionOrigin::Campaign(CampaignSlotId::One))
            .add_systems(OnEnter(Screen::Sandbox), clear_abandoned_campaign_session)
            .add_systems(OnEnter(Screen::Loading), clear_campaign_save_notice);

        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Sandbox);
        app.update();
        assert!(!app.world().contains_resource::<PendingCampaign>());
        assert!(!app.world().contains_resource::<ActiveCampaign>());
        assert!(!app.world().contains_resource::<GameplaySessionOrigin>());
        assert_eq!(app.world().resource::<CampaignSaveNotice>().0, None);

        app.world_mut().resource_mut::<CampaignSaveNotice>().0 =
            Some("stale prior-session notice".to_owned());
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Loading);
        app.update();
        assert_eq!(app.world().resource::<CampaignSaveNotice>().0, None);
    }

    #[test]
    fn active_time_requires_live_unpaused_unresolved_campaign_gameplay() {
        assert!(counts_as_active_play(
            Screen::Gameplay,
            Some(Pause(false)),
            GameplayPhase::Active,
            false,
            true,
        ));
        for (screen, pause, phase, resolved, bound) in [
            (Screen::Title, None, GameplayPhase::Active, false, true),
            (
                Screen::Gameplay,
                Some(Pause(true)),
                GameplayPhase::Active,
                false,
                true,
            ),
            (
                Screen::Gameplay,
                Some(Pause(false)),
                GameplayPhase::Deployment,
                false,
                true,
            ),
            (
                Screen::Gameplay,
                Some(Pause(false)),
                GameplayPhase::Active,
                true,
                true,
            ),
            (
                Screen::Gameplay,
                Some(Pause(false)),
                GameplayPhase::Active,
                false,
                false,
            ),
        ] {
            assert!(!counts_as_active_play(
                screen, pause, phase, resolved, bound
            ));
        }
        assert_eq!(format_active_time(3_723_456), "1:02:03");
    }

    #[test]
    fn real_state_transitions_charge_the_preceding_active_interval_only() {
        let step = Duration::from_millis(100);
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .insert_state(Screen::Loading)
            .add_sub_state::<Pause>()
            .insert_resource(TimeUpdateStrategy::ManualDuration(step))
            .insert_resource(GameplayPhase::Active)
            .insert_resource(EncounterResolution(None))
            .insert_resource(GameplaySessionOrigin::Campaign(CampaignSlotId::One))
            .insert_resource(ActiveCampaign::new(CampaignSlotId::One, 0xC0DE_CAFE, 1_000))
            .add_systems(Update, accumulate_active_play_time);

        app.update();
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        app.update();
        assert_eq!(*app.world().resource::<State<Pause>>().get(), Pause(false));
        assert_eq!(
            app.world()
                .resource::<ActiveCampaign>()
                .active_play_millis(),
            1_000,
            "Loading and the transition into Gameplay are excluded"
        );

        app.update();
        app.world_mut()
            .resource_mut::<NextState<Pause>>()
            .set(Pause(true));
        app.update();
        assert_eq!(
            app.world()
                .resource::<ActiveCampaign>()
                .active_play_millis(),
            1_200,
            "entering pause retains the complete active interval before it"
        );

        app.update();
        app.world_mut()
            .resource_mut::<NextState<Pause>>()
            .set(Pause(false));
        app.update();
        app.update();
        assert_eq!(
            app.world()
                .resource::<ActiveCampaign>()
                .active_play_millis(),
            1_300,
            "the paused intervals and transition out of pause are excluded"
        );

        app.world_mut().insert_resource(EncounterResolution(Some(
            hex_combat::EncounterOutcome::Victory,
        )));
        app.update();
        app.update();
        assert_eq!(
            app.world()
                .resource::<ActiveCampaign>()
                .active_play_millis(),
            1_400,
            "opening the outcome retains the preceding active interval and excludes the outcome"
        );

        app.world_mut().insert_resource(EncounterResolution(None));
        app.update();
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Loading);
        app.update();
        app.update();
        assert_eq!(
            app.world()
                .resource::<ActiveCampaign>()
                .active_play_millis(),
            1_500,
            "leaving Gameplay retains its preceding active interval and excludes Loading"
        );
    }

    #[test]
    fn manual_save_gate_and_provenance_fail_closed() {
        assert!(safe_for_manual_campaign_save(
            Screen::Gameplay,
            Some(Mode::Exploring),
            Some(Pause(true)),
            GameplayPhase::Active,
            true,
            false,
        ));
        for (screen, mode, pause, phase, commands_settled, resolved) in [
            (
                Screen::Title,
                Some(Mode::Exploring),
                Some(Pause(true)),
                GameplayPhase::Active,
                true,
                false,
            ),
            (
                Screen::Gameplay,
                Some(Mode::Combat),
                Some(Pause(true)),
                GameplayPhase::Active,
                true,
                false,
            ),
            (
                Screen::Gameplay,
                Some(Mode::Exploring),
                Some(Pause(false)),
                GameplayPhase::Active,
                true,
                false,
            ),
            (
                Screen::Gameplay,
                Some(Mode::Exploring),
                Some(Pause(true)),
                GameplayPhase::Deployment,
                true,
                false,
            ),
            (
                Screen::Gameplay,
                Some(Mode::Exploring),
                Some(Pause(true)),
                GameplayPhase::Active,
                false,
                false,
            ),
            (
                Screen::Gameplay,
                Some(Mode::Exploring),
                Some(Pause(true)),
                GameplayPhase::Active,
                true,
                true,
            ),
        ] {
            assert!(!safe_for_manual_campaign_save(
                screen,
                mode,
                pause,
                phase,
                commands_settled,
                resolved,
            ));
        }

        let matching = GameplaySessionOrigin::Campaign(CampaignSlotId::One);
        let mismatched = GameplaySessionOrigin::Campaign(CampaignSlotId::Two);
        let temporary = GameplaySessionOrigin::Sandbox;
        assert_eq!(
            campaign_origin_refusal(Some(&matching), CampaignSlotId::One),
            None
        );
        assert_eq!(
            campaign_origin_refusal(Some(&mismatched), CampaignSlotId::One),
            Some("Campaign not saved: the bound slot does not match this session.")
        );
        assert_eq!(
            campaign_origin_refusal(Some(&temporary), CampaignSlotId::One),
            Some("Campaign not saved: this session is temporary.")
        );
        assert_eq!(
            campaign_origin_refusal(None, CampaignSlotId::One),
            Some("Campaign not saved: this session is temporary.")
        );
    }

    #[test]
    fn replica_save_request_is_typed_not_authority_and_never_touches_disk() {
        let root = scratch_root("replica-save");
        let paths = StoragePaths::under(&root);
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .insert_state(Screen::Gameplay)
            .insert_resource(paths.clone())
            .insert_resource(InputBindings::default())
            .insert_resource(ButtonInput::<KeyCode>::default())
            .insert_resource(SimulationRole::Replica)
            .insert_resource(ActiveCampaign::new(CampaignSlotId::One, 0xC0DE_CAFE, 0))
            .insert_resource(CampaignSaveNotice::default())
            .insert_resource(CampaignSaveStatusProjection::default())
            .insert_resource(CampaignSaveRuntime::default())
            .add_systems(Update, save_exploration);
        let save_key = app
            .world()
            .resource::<InputBindings>()
            .chord(InputAction::Save)
            .key;
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(save_key);

        app.update();

        assert_eq!(
            app.world().resource::<CampaignSaveStatusProjection>().state,
            Some(CampaignSaveStateV2::Refused(
                CampaignSaveRefusalV2::NotAuthority
            ))
        );
        assert!(!paths.campaigns.exists());
        assert!(app
            .world()
            .resource::<CampaignSaveRuntime>()
            .pending
            .is_none());
        if root.exists() {
            std::fs::remove_dir_all(root).expect("scratch directory should clean up");
        }
    }

    #[test]
    fn scenario_changes_invalidate_the_digest() {
        let original = scenario();
        let mut changed = original.clone();
        changed.encounter = "config/encounters/other.ron".to_owned();
        assert_ne!(scenario_digest(&original), scenario_digest(&changed));
    }

    #[test]
    fn every_shipped_scenario_dependency_participates_in_campaign_invalidation() {
        let library: ScenarioLibrary =
            ron::from_str(include_str!("../../../assets/config/scenarios.ron"))
                .expect("the shipped scenario library should parse");
        let included: BTreeSet<&str> = SHIPPED_CAMPAIGN_INPUTS
            .iter()
            .map(|(path, _)| *path)
            .collect();
        assert_eq!(
            included.len(),
            SHIPPED_CAMPAIGN_INPUTS.len(),
            "campaign inputs must not repeat an asset path"
        );
        assert!(
            included.contains("config/terrain_damage.ron"),
            "terrain damage changes must invalidate resumable worlds"
        );

        for scenario in library.scenarios {
            for (kind, path) in [
                ("world", scenario.world.as_str()),
                ("lighting", scenario.lighting.as_str()),
                ("encounter", scenario.encounter.as_str()),
            ] {
                assert!(
                    included.contains(path),
                    "{kind} dependency {path:?} for scenario {:?} is absent from \
                     SHIPPED_CAMPAIGN_INPUTS",
                    scenario.name
                );
            }
        }

        let objects: hex_assets::ObjectCatalogFile =
            ron::from_str(include_str!("../../../assets/art/object_catalog.ron"))
                .expect("the shipped object catalog should parse");
        assert!(
            included.contains("art/object_catalog.ron"),
            "the object manifest itself must invalidate resumable worlds"
        );
        for id in objects.ids() {
            let path = format!("art/objects/{id}.ron");
            assert!(
                included.contains(path.as_str()),
                "object blueprint {path:?} can change generated blockers and must invalidate \
                 resumable worlds"
            );
        }
    }
}
