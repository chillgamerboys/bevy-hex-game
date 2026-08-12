//! Authority-only Campaign checkpoint vocabulary.
//!
//! These types are intentionally not registered as Replicon messages. A host persists
//! them locally; reconnecting clients receive the disclosure-limited live snapshot instead.

use std::{collections::BTreeSet, fmt};

use hex_core::{EffectId, Faction, PartyFormation, PersistentEffect, SimSeeds, TilePos, UnitId};
use hex_lattice::LatticeState;
use serde::{Deserialize, Serialize};

use crate::{
    BoundError, BoundedText, BoundedVec, BuildIdentityV1, ContentFingerprint, RulesManifestV1,
    WorldSnapshotV1, WorldSnapshotValidationError, MAX_IDENTITY_BYTES, MAX_SESSION_UNITS,
};

/// Current complete host-owned Campaign checkpoint version.
pub const CAMPAIGN_CHECKPOINT_VERSION_V2: u16 = 2;
/// Maximum persistent effects retained by one Campaign checkpoint.
pub const MAX_CAMPAIGN_EFFECTS: usize = 4_096;

/// One authority-private persistent effect with its stable ledger handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignEffectCheckpointV2 {
    /// Monotonic session-local effect handle.
    pub id: EffectId,
    /// Complete effect record.
    pub effect: PersistentEffect,
}

/// Exact authority effect ledger retained across a Campaign process restart.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignEffectLedgerV2 {
    /// Next never-reused effect handle.
    pub next_id: u64,
    /// Running effects in ascending handle order.
    pub effects: BoundedVec<CampaignEffectCheckpointV2, MAX_CAMPAIGN_EFFECTS>,
}

/// Complete authority-owned state of one Campaign actor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignUnitCheckpointV2 {
    /// Stable session unit identity.
    pub unit: UnitId,
    /// Exact faction.
    pub faction: Faction,
    /// Stable shipped archetype identity.
    pub archetype_identity: BoundedText<MAX_IDENTITY_BYTES>,
    /// Exact exposed world surface.
    pub position: TilePos,
    /// Complete battle-mutable lattice state, when the actor owns a lattice.
    pub lattice: Option<LatticeState>,
    /// Whether this actor is downed.
    pub downed: bool,
    /// Sanitized player-facing name.
    pub display_name: BoundedText<MAX_IDENTITY_BYTES>,
}

/// Complete host-owned Campaign checkpoint.
///
/// The shape deliberately excludes session instances, seats, credentials, online/store
/// identities, transport endpoints, cameras, and selections. Resuming this checkpoint
/// always creates a fresh multiplayer session and assignment lobby.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostCampaignCheckpointV2 {
    /// Exact checkpoint schema version.
    pub version: u16,
    /// Build that wrote the checkpoint.
    pub build: BuildIdentityV1,
    /// Accepted shipped-content graph.
    pub content_fingerprint: ContentFingerprint,
    /// Stable shipped scenario identity.
    pub scenario_identity: BoundedText<MAX_IDENTITY_BYTES>,
    /// Digest of the resolved scenario document.
    pub scenario_digest: u64,
    /// Stable shipped map catalog identity.
    pub map_catalog_identity: BoundedText<MAX_IDENTITY_BYTES>,
    /// Stable generator identity retained for compatibility diagnostics only.
    pub generator_identity: BoundedText<MAX_IDENTITY_BYTES>,
    /// Generator schema retained for compatibility diagnostics only.
    pub generator_version: u32,
    /// Resolved authored/procedural seed, when the scenario has one.
    pub resolved_seed: Option<u64>,
    /// Frozen gameplay rules.
    pub rules: RulesManifestV1,
    /// Deterministic simulation seed domains.
    pub simulation_seeds: SimSeeds,
    /// Complete generator-neutral current world.
    pub world: WorldSnapshotV1,
    /// Every authoritative actor in stable unit order.
    pub units: BoundedVec<CampaignUnitCheckpointV2, MAX_SESSION_UNITS>,
    /// Complete authority-private persistent-effect ledger.
    pub effects: CampaignEffectLedgerV2,
    /// Exact player-party formation.
    pub formation: PartyFormation,
    /// Accumulated active (unpaused, non-terminal) play time.
    pub active_play_millis: u64,
}

impl HostCampaignCheckpointV2 {
    /// Validates the owner-neutral structural contract before any world or actor mutation.
    ///
    /// Lattice/catalog/footing validation remains gameplay-owned because it needs accepted
    /// shipped content and live public terrain contracts. This method proves canonical
    /// ordering, cross-record references, and the absence of dangling world positions.
    pub fn validate(&self) -> Result<(), CampaignValidationError> {
        if self.version != CAMPAIGN_CHECKPOINT_VERSION_V2 {
            return Err(CampaignValidationError::WrongVersion);
        }
        self.world
            .validate()
            .map_err(CampaignValidationError::World)?;
        if self.units.is_empty() {
            return Err(CampaignValidationError::EmptyRoster);
        }

        let surfaces = self
            .world
            .columns
            .as_slice()
            .iter()
            .flat_map(|column| column.runs.as_slice().iter().map(|run| run.position))
            .collect::<BTreeSet<_>>();
        let mut units = BTreeSet::new();
        let mut occupied = BTreeSet::new();
        let mut previous = None;
        let mut players = BTreeSet::new();
        for unit in self.units.as_slice() {
            if previous.is_some_and(|previous| previous >= unit.unit) {
                return Err(CampaignValidationError::NonCanonicalUnits);
            }
            previous = Some(unit.unit);
            if !units.insert(unit.unit) {
                return Err(CampaignValidationError::DuplicateUnit(unit.unit));
            }
            if !occupied.insert(unit.position) {
                return Err(CampaignValidationError::DuplicatePosition(unit.position));
            }
            if !surfaces.contains(&unit.position) {
                return Err(CampaignValidationError::DanglingPosition(unit.position));
            }
            if unit.faction == Faction::Player {
                players.insert(unit.unit);
            }
        }
        if players.is_empty() {
            return Err(CampaignValidationError::EmptyParty);
        }

        let _validated_preset =
            BoundedText::<MAX_IDENTITY_BYTES>::new(self.formation.preset.clone())
                .map_err(CampaignValidationError::FormationPreset)?;
        let assigned = self
            .formation
            .assignments
            .keys()
            .copied()
            .collect::<BTreeSet<_>>();
        if assigned != players {
            return Err(CampaignValidationError::FormationMembership);
        }
        let unique_slots = self
            .formation
            .assignments
            .values()
            .copied()
            .collect::<BTreeSet<_>>();
        if unique_slots.len() != self.formation.assignments.len() {
            return Err(CampaignValidationError::DuplicateFormationSlot);
        }

        let mut previous_effect = None;
        for entry in self.effects.effects.as_slice() {
            if previous_effect.is_some_and(|previous| previous >= entry.id) {
                return Err(CampaignValidationError::NonCanonicalEffects);
            }
            previous_effect = Some(entry.id);
            if entry.id.0 >= self.effects.next_id {
                return Err(CampaignValidationError::EffectSequence);
            }
            if !units.contains(&entry.effect.source) || !units.contains(&entry.effect.target) {
                return Err(CampaignValidationError::DanglingEffectUnit);
            }
        }
        Ok(())
    }
}

/// Why a complete host Campaign checkpoint is structurally invalid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CampaignValidationError {
    /// The checkpoint schema is unsupported.
    WrongVersion,
    /// The complete world snapshot is malformed.
    World(WorldSnapshotValidationError),
    /// No actors were retained.
    EmptyRoster,
    /// No player-faction party member exists.
    EmptyParty,
    /// Unit records are not in strict id order.
    NonCanonicalUnits,
    /// A stable unit id appears twice.
    DuplicateUnit(UnitId),
    /// Two actors occupy one exact surface.
    DuplicatePosition(TilePos),
    /// An actor position is absent from the retained public world.
    DanglingPosition(TilePos),
    /// The formation preset identity is malformed.
    FormationPreset(BoundError),
    /// Formation members differ from the complete player party.
    FormationMembership,
    /// Two party members occupy one formation slot.
    DuplicateFormationSlot,
    /// Effect records are not in strict handle order.
    NonCanonicalEffects,
    /// An effect handle is not below the next monotonic handle.
    EffectSequence,
    /// An effect source or target is absent from the actor roster.
    DanglingEffectUnit,
}

impl fmt::Display for CampaignValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::WrongVersion => "Campaign checkpoint version is unsupported",
            Self::World(_) => "Campaign world snapshot is invalid",
            Self::EmptyRoster => "Campaign checkpoint has no actors",
            Self::EmptyParty => "Campaign checkpoint has no player party",
            Self::NonCanonicalUnits => "Campaign units are not in canonical order",
            Self::DuplicateUnit(_) => "Campaign checkpoint repeats a unit",
            Self::DuplicatePosition(_) => "Campaign checkpoint repeats an actor position",
            Self::DanglingPosition(_) => "Campaign actor position is absent from the world",
            Self::FormationPreset(_) => "Campaign formation preset identity is invalid",
            Self::FormationMembership => "Campaign formation does not match the player party",
            Self::DuplicateFormationSlot => "Campaign formation repeats a slot",
            Self::NonCanonicalEffects => "Campaign effects are not in canonical order",
            Self::EffectSequence => "Campaign effect handle is outside its sequence",
            Self::DanglingEffectUnit => "Campaign effect references an absent actor",
        })
    }
}

impl std::error::Error for CampaignValidationError {}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use hex_core::{HexCoord, PartyMovementMode, Sextant};

    use super::*;
    use crate::{
        PublicWorldFingerprint, WorldColumnSnapshotV1, WorldRunSnapshotV1,
        WORLD_SNAPSHOT_VERSION_V1,
    };

    fn text(value: &str) -> BoundedText<MAX_IDENTITY_BYTES> {
        BoundedText::new(value).expect("fixture text fits")
    }

    fn checkpoint() -> HostCampaignCheckpointV2 {
        let position = TilePos::new(HexCoord::ORIGIN, 0);
        HostCampaignCheckpointV2 {
            version: CAMPAIGN_CHECKPOINT_VERSION_V2,
            build: BuildIdentityV1::new("0.4.0", "fixture").expect("build fits"),
            content_fingerprint: ContentFingerprint(7),
            scenario_identity: text("Party Trial"),
            scenario_digest: 9,
            map_catalog_identity: text("party-trial"),
            generator_identity: text("authored"),
            generator_version: 1,
            resolved_seed: None,
            rules: RulesManifestV1 {
                profile_identity: text("default"),
                fingerprint: 11,
            },
            simulation_seeds: SimSeeds {
                world: 1,
                ai_flavor: 2,
                cosmetic: 3,
            },
            world: WorldSnapshotV1 {
                version: WORLD_SNAPSHOT_VERSION_V1,
                public_fingerprint: PublicWorldFingerprint(1),
                columns: BoundedVec::new(vec![WorldColumnSnapshotV1 {
                    coord: HexCoord::ORIGIN,
                    runs: BoundedVec::new(vec![WorldRunSnapshotV1 {
                        position,
                        run_bottom: 0,
                        span_bottom_bits: 0.0_f32.to_bits(),
                        span_top_bits: 1.0_f32.to_bits(),
                        substance: text("stone"),
                        headroom: hex_core::MAX_HEADROOM,
                    }])
                    .expect("run fits"),
                }])
                .expect("column fits"),
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
                unit: UnitId(1),
                faction: Faction::Player,
                archetype_identity: text("warrior"),
                position,
                lattice: None,
                downed: false,
                display_name: text("Warrior"),
            }])
            .expect("unit fits"),
            effects: CampaignEffectLedgerV2::default(),
            formation: PartyFormation {
                preset: "Line".to_owned(),
                assignments: BTreeMap::from([(UnitId(1), HexCoord::ORIGIN)]),
                facing: Sextant::default(),
                mode: PartyMovementMode::Group,
            },
            active_play_millis: 12_000,
        }
    }

    #[test]
    fn complete_checkpoint_round_trips_without_transport_or_selection_fields() {
        let checkpoint = checkpoint();
        assert_eq!(checkpoint.validate(), Ok(()));
        let encoded = serde_json::to_string(&checkpoint).expect("checkpoint serializes");
        assert!(!encoded.contains("credential"));
        assert!(!encoded.contains("principal"));
        assert!(!encoded.contains("selection"));
        assert!(!encoded.contains("camera"));
        let decoded: HostCampaignCheckpointV2 =
            serde_json::from_str(&encoded).expect("checkpoint deserializes");
        assert_eq!(decoded, checkpoint);
    }

    #[test]
    fn structural_validation_rejects_dangling_positions_and_effects() {
        let mut dangling_position = checkpoint();
        let unit = dangling_position
            .units
            .into_vec()
            .into_iter()
            .map(|mut unit| {
                unit.position = TilePos::new(HexCoord::from_axial(1, 0), 0);
                unit
            })
            .collect();
        dangling_position.units = BoundedVec::new(unit).expect("unit fits");
        assert!(matches!(
            dangling_position.validate(),
            Err(CampaignValidationError::DanglingPosition(_))
        ));

        let mut dangling_effect = checkpoint();
        dangling_effect.effects = CampaignEffectLedgerV2 {
            next_id: 1,
            effects: BoundedVec::new(vec![CampaignEffectCheckpointV2 {
                id: EffectId(0),
                effect: PersistentEffect {
                    source: UnitId(1),
                    target: UnitId(2),
                    payload: hex_core::EffectPayload::Burn,
                    start: 0,
                    end: hex_core::EffectEnd::AfterTurns(1),
                    ticks: 0,
                },
            }])
            .expect("effect fits"),
        };
        assert_eq!(
            dangling_effect.validate(),
            Err(CampaignValidationError::DanglingEffectUnit)
        );
    }
}
