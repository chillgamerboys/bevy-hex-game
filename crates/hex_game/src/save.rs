//! Three atomic, build-bound Campaign save slots.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;
use std::time::Duration;

use bevy::ecs::system::SystemParam;
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
    ResolvedMapSeed, Screen, SubstanceId, TilePos, TraversalBlockers, TraversalProfile, UnitId,
};
use hex_gameplay_model::CampaignSlotId;
use hex_lattice::{CellKind, LatticeState};
use hex_map::{MapSettings, TerrainSettings};
use hex_ui::{
    CampaignPartyMemberView, CampaignSlotStatusView, CampaignSlotView, MainMenuIntent,
    SandboxLatticeCellKind, SandboxLatticeCellView, UiIntent, UiSystems,
};
use hex_units::{
    Archetype as UnitArchetype, Body, Downed, Faction, Footing, MovingTo, Selected, StandsOn,
};
use serde::{Deserialize, Serialize};

use crate::scenarios::{ActiveScenario, ScenarioToLoad};
use crate::screens::sandbox::GameplaySessionOrigin;
use crate::storage::{read, write_atomic, StoragePaths};

const LEGACY_RESUME_VERSION: u32 = 1;
const CAMPAIGNS_VERSION: u32 = 1;

/// Exact digest translation table for resumes written by PR #174's `dev` head.
///
/// The cutover changed comments in two digest-bound assets without changing their
/// parsed meaning. An old digest is accepted only while the current digest still
/// equals the corresponding cutover digest, so any later semantic asset change keeps
/// invalidating the legacy resume as intended.
const LEGACY_RESUME_DIGESTS: &[(&str, u64, u64)] = &[
    ("The Crossing", 0x4FE3_7AF1_ED42_E275, 0x878B_132A_EE3C_56E9),
    (
        "Procedural Hills",
        0x0CA2_8E38_AE9D_0FBE,
        0xC6E6_E66F_AC88_1AA6,
    ),
    (
        "Rolling Hills",
        0xF0F5_9BD2_B2B4_F2FD,
        0x7CAC_73F3_AB85_64D1,
    ),
    ("Frozen Hills", 0x3AE0_F233_785C_9A54, 0x8F2B_1BC7_40CC_665C),
    (
        "Volcanic Hills",
        0x3DA8_7548_5A92_66CA,
        0x35A9_7668_EFE5_3722,
    ),
    ("Sky Islands", 0x839E_37A3_1C68_4BAF, 0x1678_D1D8_1708_4C8B),
    ("Mountains", 0xC8E5_F366_1693_22AC, 0xC8C7_F78A_02A3_D874),
    ("Caves", 0x4922_0268_B17F_DEBF, 0x6A34_BC6D_6EC5_FA9B),
    ("Waterfall", 0x5F92_730F_B230_D810, 0xCF04_4B44_AF02_AAC8),
    ("Forest", 0x99B8_85E9_0A0E_01E2, 0x7943_EBD8_2132_1A1A),
    ("Deep Forest", 0x5E2F_C204_F57E_7FF6, 0xD686_3A31_CFCB_F8DE),
    ("Prairie", 0x5566_A4CC_38B7_F533, 0xCF5F_0F51_1285_700F),
    ("Fort", 0xA3C0_0405_E270_1C2E, 0x8AF5_16FA_8DAC_4196),
    (
        "Seven Regions",
        0x7855_E22C_790E_292A,
        0x894C_CB40_04F1_E102,
    ),
    ("Two Rings", 0x1DB7_0E9A_A443_B8CC, 0x911B_3847_EEAB_1F14),
    ("Party Trial", 0xB392_E2D3_BC35_DBDB, 0x5847_F52E_E0E5_8697),
    ("Ability Lab", 0x3DCD_DEE6_C32B_7BF3, 0x5486_D856_33D8_B34F),
    (
        "Raider Mirror",
        0xE5EC_54E8_8BC8_EB5D,
        0xC9A1_CB56_3295_E6B1,
    ),
];

/// Earlier `dev` digests invalidated only by PR #172's scenario-browser comment edit.
///
/// The previous title/lane wording changed comments in `scenarios.ron`, which the
/// deliberately coarse resume digest includes byte-for-byte. Keep the exact known
/// predecessor guarded by the current cutover digest; an unknown saved digest or a
/// later change to any Campaign input remains incompatible. The predecessor values
/// are generated from the exact `260809f` `dev` tree.
const LEGACY_RESUME_DIGEST_ALIASES: &[(&str, u64, u64)] = &[
    ("The Crossing", 0xFBF2_7A92_7CD0_8F34, 0x878B_132A_EE3C_56E9),
    (
        "Procedural Hills",
        0x7642_0E89_9D06_AE43,
        0xC6E6_E66F_AC88_1AA6,
    ),
    (
        "Rolling Hills",
        0xD677_B8BD_04B3_F05C,
        0x7CAC_73F3_AB85_64D1,
    ),
    ("Frozen Hills", 0x6B60_05E5_0F13_8E55, 0x8F2B_1BC7_40CC_665C),
    (
        "Volcanic Hills",
        0x8D04_C7E9_FD70_94DF,
        0x35A9_7668_EFE5_3722,
    ),
    ("Sky Islands", 0x3604_C4A3_7559_A3C2, 0x1678_D1D8_1708_4C8B),
    ("Mountains", 0x8849_3BE2_DE07_386D, 0xC8C7_F78A_02A3_D874),
    ("Caves", 0x25F1_9D4B_CA4B_8212, 0x6A34_BC6D_6EC5_FA9B),
    ("Waterfall", 0x4680_0901_2C45_84F1, 0xCF04_4B44_AF02_AAC8),
    ("Forest", 0x4B24_C05A_9EBE_9897, 0x7943_EBD8_2132_1A1A),
    ("Deep Forest", 0xBCBC_87EB_C8F6_FC1B, 0xD686_3A31_CFCB_F8DE),
    ("Prairie", 0xCFA5_42D7_283F_6B56, 0xCF5F_0F51_1285_700F),
    ("Fort", 0x78D4_8B75_2CFA_8633, 0x8AF5_16FA_8DAC_4196),
    (
        "Seven Regions",
        0x4A2C_FDB2_4C1F_C13F,
        0x894C_CB40_04F1_E102,
    ),
    ("Two Rings", 0xBA55_FA38_C276_6D8D, 0x911B_3847_EEAB_1F14),
    ("Party Trial", 0x8737_950D_F612_A47E, 0x5847_F52E_E0E5_8697),
    ("Ability Lab", 0xEABC_1965_343D_D916, 0x5486_D856_33D8_B34F),
    (
        "Raider Mirror",
        0x17D5_770A_5EF5_5FBC,
        0xC9A1_CB56_3295_E6B1,
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
    slots: [Option<CampaignSave>; 3],
}

impl Default for CampaignsFile {
    fn default() -> Self {
        Self {
            format_version: CAMPAIGNS_VERSION,
            legacy_slot_one_refusal: None,
            slots: std::array::from_fn(|_| None),
        }
    }
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

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<StoragePaths>()
        .init_resource::<CampaignStore>()
        .init_resource::<CampaignSaveNotice>()
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
            (accumulate_active_play_time, save_exploration)
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

fn clear_campaign_save_notice(mut notice: ResMut<CampaignSaveNotice>) {
    notice.0 = None;
}

fn clear_abandoned_campaign_session(
    mut commands: Commands,
    mut notice: ResMut<CampaignSaveNotice>,
    origin: Option<Res<GameplaySessionOrigin>>,
) {
    commands.remove_resource::<PendingCampaign>();
    commands.remove_resource::<ActiveCampaign>();
    if matches!(origin.as_deref(), Some(GameplaySessionOrigin::Campaign(_))) {
        commands.remove_resource::<GameplaySessionOrigin>();
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
        Ok(file) if file.format_version == CAMPAIGNS_VERSION => CampaignStore {
            file: Some(file),
            unreadable: None,
            runtime_invalid: std::array::from_fn(|_| None),
            catalog_invalid: std::array::from_fn(|_| None),
        },
        Ok(file) => CampaignStore {
            file: None,
            unreadable: Some(format!(
                "Campaign format {} is incompatible with {}.",
                file.format_version, CAMPAIGNS_VERSION
            )),
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
                status: match self.slot(slot) {
                    Ok(None) => CampaignSlotStatusView::Empty,
                    Ok(Some(save)) => CampaignSlotStatusView::Available {
                        party: save
                            .units
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
                        active_time: format_active_time(save.active_play_millis),
                    },
                    Err(reason) => CampaignSlotStatusView::Invalid { reason },
                },
            })
            .collect()
    }

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
        let Some(save) = file.slots.get(slot.index()).and_then(Option::as_ref) else {
            return Ok(None);
        };
        validate_campaign_save(save, slot)?;
        Ok(Some(save))
    }

    fn available(&self, slot: CampaignSlotId) -> Option<CampaignSave> {
        self.slot(slot).ok().flatten().cloned()
    }

    fn is_empty(&self, slot: CampaignSlotId) -> bool {
        matches!(self.slot(slot), Ok(None))
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

    fn write_slot(
        &mut self,
        paths: &StoragePaths,
        slot: CampaignSlotId,
        save: CampaignSave,
    ) -> Result<(), String> {
        if let Err(reason) = self.slot(slot) {
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
        let Some(target) = next.slots.get_mut(slot.index()) else {
            return Err(format!(
                "Campaign slot {} is outside the fixed slot document.",
                slot.number()
            ));
        };
        *target = Some(save);
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
    mut store: ResMut<CampaignStore>,
) {
    let (Some(library), Some(accepted), Some(formations)) = (library, accepted, formations) else {
        return;
    };
    let refusals = std::array::from_fn(|index| {
        let slot = CampaignSlotId::ALL.get(index).copied()?;
        let save = store.file.as_ref()?.slots.get(index)?.as_ref()?;
        validate_campaign_save_against_catalog(save, slot, &formations)
            .err()
            .or_else(|| campaign_content_refusal(save, &library, accepted.fingerprint()))
    });
    if store.catalog_invalid != refusals {
        store.catalog_invalid = refusals;
    }
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

fn handle_campaign_intents(
    mut intents: MessageReader<UiIntent>,
    mut store: ResMut<CampaignStore>,
    library: Option<Res<ScenarioLibrary>>,
    accepted: Option<Res<AcceptedContentRevision>>,
    formations: Option<Res<FormationCatalog>>,
    mut commands: Commands,
    mut next: ResMut<NextState<Screen>>,
) {
    for intent in intents.read() {
        let UiIntent::MainMenu(intent) = intent else {
            continue;
        };
        match *intent {
            MainMenuIntent::NewCampaign(slot) => {
                if !store.is_empty(slot) {
                    continue;
                }
                let Some(library) = library.as_deref() else {
                    commands.insert_resource(GameplaySetupFailure::new(
                        "Campaign content is still loading.",
                    ));
                    continue;
                };
                let Some(accepted) = accepted.as_deref() else {
                    commands.insert_resource(GameplaySetupFailure::new(
                        "Campaign content is still loading.",
                    ));
                    continue;
                };
                let Some(_formations) = formations.as_deref() else {
                    commands.insert_resource(GameplaySetupFailure::new(
                        "Campaign content is still loading.",
                    ));
                    continue;
                };
                let Some(scenario) = library.default_scenario() else {
                    let reason = format!(
                        "The configured default game {:?} does not exist.",
                        library.default_game
                    );
                    commands.insert_resource(GameplaySetupFailure::new(reason));
                    continue;
                };
                commands.remove_resource::<GameplaySetupFailure>();
                commands.remove_resource::<PendingCampaign>();
                commands.insert_resource(ActiveCampaign::new(slot, accepted.fingerprint(), 0));
                commands.insert_resource(GameplaySessionOrigin::Campaign(slot));
                commands.insert_resource(ScenarioToLoad {
                    scenario: scenario.clone(),
                    resolved_seed: scenario.generation_seed.map(ResolvedMapSeed),
                    encounter_override: None,
                });
                next.set(Screen::Loading);
                return;
            }
            MainMenuIntent::ContinueCampaign(slot) => {
                let Some(save) = store.available(slot) else {
                    continue;
                };
                let Some(library) = library.as_deref() else {
                    commands.insert_resource(GameplaySetupFailure::new(
                        "Campaign content is still loading.",
                    ));
                    continue;
                };
                let Some(accepted) = accepted.as_deref() else {
                    commands.insert_resource(GameplaySetupFailure::new(
                        "Campaign content is still loading.",
                    ));
                    continue;
                };
                let Some(formations) = formations.as_deref() else {
                    commands.insert_resource(GameplaySetupFailure::new(
                        "Campaign content is still loading.",
                    ));
                    continue;
                };
                if let Err(reason) = validate_campaign_save_against_catalog(&save, slot, formations)
                {
                    refuse_continue(&mut store, &mut commands, slot, reason);
                    continue;
                }
                if let Some(reason) =
                    campaign_content_refusal(&save, library, accepted.fingerprint())
                {
                    refuse_continue(&mut store, &mut commands, slot, reason);
                    continue;
                }
                let Some(scenario) = library
                    .scenarios
                    .iter()
                    .find(|scenario| scenario.name == save.scenario_name)
                else {
                    refuse_continue(
                        &mut store,
                        &mut commands,
                        slot,
                        format!(
                            "The saved scenario {:?} is no longer available.",
                            save.scenario_name
                        ),
                    );
                    continue;
                };
                commands.remove_resource::<GameplaySetupFailure>();
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
                commands.insert_resource(GameplaySessionOrigin::Campaign(slot));
                next.set(Screen::Loading);
                return;
            }
            _ => {}
        }
    }
}

fn refuse_continue(
    store: &mut CampaignStore,
    commands: &mut Commands,
    slot: CampaignSlotId,
    reason: String,
) {
    store.mark_catalog_invalid(slot, reason.clone());
    commands.insert_resource(GameplaySetupFailure::new(reason));
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

#[derive(SystemParam)]
struct SaveWorld<'w, 's> {
    screen: Res<'w, State<Screen>>,
    mode: Option<Res<'w, State<Mode>>>,
    pause: Option<Res<'w, State<Pause>>>,
    phase: Res<'w, GameplayPhase>,
    queue: Res<'w, CommandQueue>,
    pending: Res<'w, PendingDecision>,
    resolution: Res<'w, EncounterResolution>,
    active_scenario: Option<Res<'w, ActiveScenario>>,
    active_campaign: Option<ResMut<'w, ActiveCampaign>>,
    origin: Option<Res<'w, GameplaySessionOrigin>>,
    accepted_content: Option<Res<'w, AcceptedContentRevision>>,
    map: Option<Res<'w, MapSettings>>,
    formation: Res<'w, PartyFormation>,
    moving: Query<'w, 's, (), Or<(With<MovingTo>, With<Busy>)>>,
    units: Query<
        'w,
        's,
        (
            &'static UnitId,
            &'static Faction,
            &'static UnitArchetype,
            &'static StandsOn,
            Option<&'static LatticeState>,
            Option<&'static Name>,
            Has<Downed>,
            Has<Selected>,
        ),
    >,
}

fn save_exploration(
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<InputBindings>,
    mut world: SaveWorld,
    paths: Res<StoragePaths>,
    mut store: ResMut<CampaignStore>,
    mut notice: ResMut<CampaignSaveNotice>,
) {
    if *world.screen.get() != Screen::Gameplay || !bindings.just_pressed(&keys, InputAction::Save) {
        return;
    }
    let Some(active_campaign) = world.active_campaign.as_deref_mut() else {
        notice.0 = Some("Campaign not saved: this session is temporary.".to_owned());
        return;
    };
    if let Some(refusal) = campaign_origin_refusal(world.origin.as_deref(), active_campaign.slot) {
        notice.0 = Some(refusal.to_owned());
        return;
    }
    let safe = safe_for_manual_campaign_save(
        *world.screen.get(),
        world.mode.as_deref().map(|mode| *mode.get()),
        world.pause.as_deref().map(|pause| *pause.get()),
        *world.phase,
        world.queue.is_empty() && !world.pending.is_open() && world.moving.is_empty(),
        world.resolution.is_resolved(),
    );
    if !safe {
        notice.0 = Some(
            "Campaign not saved: pause during safe exploration with no movement or decision pending."
                .to_owned(),
        );
        return;
    }
    let (Some(active_scenario), Some(map)) = (world.active_scenario, world.map) else {
        notice.0 = Some("Campaign not saved: scenario setup is incomplete.".to_owned());
        return;
    };
    let Some(accepted_content) = world.accepted_content else {
        notice.0 = Some("Campaign not saved: authored content is not accepted.".to_owned());
        return;
    };
    if accepted_content.fingerprint() != active_campaign.content_revision {
        notice.0 =
            Some("Campaign not saved: authored content changed during this session.".to_owned());
        return;
    }

    let mut snapshots: Vec<CampaignUnitSave> = world
        .units
        .iter()
        .map(
            |(id, faction, archetype, standing, lattice, name, downed, _)| CampaignUnitSave {
                id: *id,
                faction: *faction,
                position: standing.0.pos,
                archetype: archetype.0.clone(),
                lattice: lattice.cloned(),
                downed,
                display_name: name
                    .map(|name| name.as_str().to_owned())
                    .unwrap_or_else(|| format!("Unit {}", id.0)),
            },
        )
        .collect();
    snapshots.sort_by_key(|unit| unit.id);
    let selected = world
        .units
        .iter()
        .find_map(|(id, _, _, _, _, _, _, selected)| selected.then_some(*id));
    let slot = active_campaign.slot;
    let save = CampaignSave {
        slot,
        build_version: build_identity().to_owned(),
        scenario_name: active_scenario.0.scenario.name.clone(),
        scenario_digest: scenario_digest(&active_scenario.0.scenario),
        content_revision: Some(active_campaign.content_revision),
        resolved_seed: active_scenario.0.resolved_seed.map(|seed| seed.0),
        generator_version: generator_version(&map),
        formation: world.formation.as_ref().clone(),
        selected,
        active_play_millis: active_campaign.active_play_millis(),
        units: snapshots,
    };
    if let Err(reason) = validate_campaign_save(&save, slot) {
        notice.0 = Some(format!("Campaign not saved: {reason}"));
        return;
    }
    match store.write_slot(&paths, slot, save) {
        Ok(()) => {
            active_campaign.mark_persisted();
            notice.0 = Some(format!("Campaign slot {} saved.", slot.number()));
        }
        Err(reason) => notice.0 = Some(reason),
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
    LEGACY_RESUME_DIGESTS
        .iter()
        .chain(LEGACY_RESUME_DIGEST_ALIASES)
        .any(|(name, legacy, cutover)| {
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
        ArtPalette, ContentIndex, ElementFile, LatticeFile, ScenarioCategory, SpellBook, SpellFile,
        SubstanceFile, SubstanceTable,
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
        let palette: ArtPalette = ron::from_str(include_str!("../../../assets/art/palette.ron"))
            .expect("the shipped palette should parse");
        let elements = ElementCatalog::from_file(&element_file);
        let spells = SpellBook::from_file(&spell_file);
        let substances = SubstanceTable::from_file(&substance_file, &palette)
            .expect("the shipped substances should resolve");
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
    fn manual_save_serializes_the_bound_slot_with_exact_character_state_and_active_time() {
        let spell_file: SpellFile =
            ron::from_str(include_str!("../../../assets/config/spells.ron"))
                .expect("the shipped spells should parse");
        let accepted = {
            let mut content_app = App::new();
            content_app
                .add_plugins(MinimalPlugins)
                .add_plugins(hex_assets::content_index::plugin);
            insert_coherent_content(&mut content_app, spell_file);
            content_app.update();
            *content_app.world().resource::<AcceptedContentRevision>()
        };
        let map: MapSettings = ron::from_str(include_str!("../../../assets/config/world.ron"))
            .expect("the shipped authored map should parse");
        let (_, lattices) = shipped_lattice_tables();
        let archetype = lattices
            .get("hedge-mage")
            .expect("the shipped Hedge Mage should resolve");
        let lattice = LatticeState::new(&archetype.spec, &archetype.stats);
        let standing = Standing {
            pos: TilePos::ORIGIN,
            span: HexSpan::new(0.0, 1.0),
        };
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
        let mut active = ActiveCampaign::new(CampaignSlotId::Two, accepted.fingerprint(), 12_000);
        active.session_active_play = Duration::from_millis(345);
        let mut keys = ButtonInput::<KeyCode>::default();
        keys.press(KeyCode::F5);

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(State::new(Screen::Gameplay))
            .insert_resource(State::new(Mode::Exploring))
            .insert_resource(State::new(Pause(true)))
            .insert_resource(GameplayPhase::Active)
            .insert_resource(CommandQueue::default())
            .insert_resource(PendingDecision::None)
            .insert_resource(EncounterResolution(None))
            .insert_resource(ActiveScenario(ScenarioToLoad {
                scenario: scenario(),
                resolved_seed: None,
                encounter_override: None,
            }))
            .insert_resource(active)
            .insert_resource(GameplaySessionOrigin::Campaign(CampaignSlotId::Two))
            .insert_resource(accepted)
            .insert_resource(map)
            .insert_resource(compact_formation(&[UnitId(0)]))
            .insert_resource(keys)
            .insert_resource(InputBindings::default())
            .insert_resource(paths.clone())
            .insert_resource(store)
            .insert_resource(CampaignSaveNotice::default())
            .add_systems(Update, save_exploration);
        app.world_mut().spawn((
            UnitId(0),
            Faction::Player,
            UnitArchetype("hedge-mage".to_owned()),
            StandsOn(standing),
            lattice.clone(),
            Name::new("Saved Hedge Mage"),
            Selected,
        ));

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
        let saved = persisted.slots[1]
            .as_ref()
            .expect("the first normal save should occupy the bound slot two");
        assert_eq!(saved.slot, CampaignSlotId::Two);
        assert_eq!(saved.selected, Some(UnitId(0)));
        assert_eq!(saved.active_play_millis, 12_345);
        assert_eq!(saved.content_revision, Some(accepted.fingerprint()));
        assert_eq!(saved.units.len(), 1);
        let saved_unit = saved
            .units
            .first()
            .expect("the exact saved player should remain present");
        assert_eq!(saved_unit.archetype, "hedge-mage");
        assert_eq!(saved_unit.display_name, "Saved Hedge Mage");
        assert_eq!(saved_unit.lattice.as_ref(), Some(&lattice));
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
    fn exact_origin_dev_resume_digest_survives_comment_only_cutover_changes() {
        let legacy_text = include_str!("../testdata/legacy_resume_origin_dev.ron");
        let legacy: LegacyResumeFile =
            ron::from_str(legacy_text).expect("the origin/dev resume fixture should parse");
        assert_eq!(legacy.build_version, build_identity());
        assert_eq!(legacy.scenario_digest, 0xB392_E2D3_BC35_DBDB);
        validate_legacy_resume(&legacy).expect("the origin/dev record itself is valid");

        let library: ScenarioLibrary =
            ron::from_str(include_str!("../../../assets/config/scenarios.ron"))
                .expect("the cutover scenario library should parse");
        assert_eq!(LEGACY_RESUME_DIGESTS.len(), library.scenarios.len());
        for current in &library.scenarios {
            let (_, _, cutover_digest) = LEGACY_RESUME_DIGESTS
                .iter()
                .find(|(name, _, _)| *name == current.name)
                .expect("every PR #174 scenario has an explicit digest translation");
            assert_eq!(scenario_digest(current), *cutover_digest);
        }
        let current = library
            .scenarios
            .iter()
            .find(|candidate| candidate.name == legacy.scenario_name)
            .expect("Party Trial remains the canonical Campaign");
        assert_eq!(scenario_digest(current), 0x5847_F52E_E0E5_8697);
        assert!(legacy_resume_digest_is_compatible(
            &current.name,
            legacy.scenario_digest,
            scenario_digest(current),
        ));

        let root = scratch_root("origin-dev-legacy");
        let paths = StoragePaths::under(&root);
        write_atomic(&paths.resume, legacy_text).expect("legacy fixture should write");
        let store = migrate_legacy(&paths);
        let migrated = store
            .available(CampaignSlotId::One)
            .expect("the origin/dev record should migrate to slot one");
        assert_eq!(migrated.scenario_digest, legacy.scenario_digest);
        assert_eq!(migrated.active_play_millis, 0);
        assert_eq!(migrated.content_revision, None);
        assert_eq!(
            campaign_content_refusal(&migrated, &library, 0xC0DE_CAFE),
            None
        );
        let mut non_legacy = migrated.clone();
        non_legacy.content_revision = Some(0xC0DE_CAFE);
        assert!(campaign_content_refusal(&non_legacy, &library, 0xC0DE_CAFE).is_some());
        assert_eq!(read(&paths.resume).expect("legacy remains"), legacy_text);
        std::fs::remove_dir_all(root).expect("scratch directory should clean up");
    }

    #[test]
    fn pre_ui_foundation_resumes_survive_comment_only_copy_change() {
        let library: ScenarioLibrary =
            ron::from_str(include_str!("../../../assets/config/scenarios.ron"))
                .expect("the cutover scenario library should parse");
        assert_eq!(LEGACY_RESUME_DIGEST_ALIASES.len(), library.scenarios.len());
        for current in &library.scenarios {
            let (_, predecessor_digest, cutover_digest) = LEGACY_RESUME_DIGEST_ALIASES
                .iter()
                .find(|(name, _, _)| *name == current.name)
                .expect("every pre-UI-foundation scenario should have an explicit alias");
            assert_ne!(predecessor_digest, cutover_digest);
            assert_eq!(scenario_digest(current), *cutover_digest);
            assert!(legacy_resume_digest_is_compatible(
                &current.name,
                *predecessor_digest,
                *cutover_digest,
            ));
        }

        let (_, saved_digest, cutover_digest) = LEGACY_RESUME_DIGEST_ALIASES
            .iter()
            .find(|(name, _, _)| *name == "Party Trial")
            .expect("the known pre-UI-foundation Party Trial digest should remain explicit");
        assert_eq!(*saved_digest, 0x8737_950D_F612_A47E);
        let current = library
            .scenarios
            .iter()
            .find(|candidate| candidate.name == "Party Trial")
            .expect("Party Trial remains the canonical Campaign");
        assert_eq!(scenario_digest(current), *cutover_digest);

        let mut legacy = legacy_resume();
        legacy.scenario_digest = *saved_digest;
        let legacy_text = ron::ser::to_string_pretty(&legacy, ron::ser::PrettyConfig::new())
            .expect("the predecessor resume should encode");
        let root = scratch_root("pre-ui-foundation-legacy");
        let paths = StoragePaths::under(&root);
        write_atomic(&paths.resume, &legacy_text).expect("legacy fixture should write");

        let store = migrate_legacy(&paths);
        let migrated = store
            .available(CampaignSlotId::One)
            .expect("the known predecessor should migrate to slot one");
        assert_eq!(migrated.scenario_digest, *saved_digest);
        assert_eq!(migrated.content_revision, None);
        assert_eq!(
            campaign_content_refusal(&migrated, &library, 0xC0DE_CAFE),
            None
        );

        let campaigns_text = read(&paths.campaigns).expect("migration should persist campaigns");
        let reloaded = decode_campaigns(&campaigns_text);
        let persisted = reloaded
            .available(CampaignSlotId::One)
            .expect("the already-migrated predecessor should remain available");
        assert_eq!(persisted.scenario_digest, *saved_digest);
        assert_eq!(
            campaign_content_refusal(&persisted, &library, 0xC0DE_CAFE),
            None
        );

        let mut changed_library = library.clone();
        changed_library
            .scenarios
            .iter_mut()
            .find(|candidate| candidate.name == "Party Trial")
            .expect("Party Trial remains present")
            .encounter = "config/encounters/changed.ron".to_owned();
        assert!(campaign_content_refusal(&migrated, &changed_library, 0xC0DE_CAFE).is_some());
        let mut unknown_digest = persisted.clone();
        unknown_digest.scenario_digest = 0xDEAD_BEEF_DEAD_BEEF;
        assert_eq!(
            campaign_content_refusal(&unknown_digest, &library, 0xC0DE_CAFE),
            Some("The saved scenario \"Party Trial\" changed and cannot be resumed.".to_owned())
        );
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
