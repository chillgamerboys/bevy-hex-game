//! Frozen identities and deterministic launch inputs shared by every peer.

use std::{collections::BTreeSet, fmt};

use bevy_ecs::prelude::Message;
use hex_core::{Faction, SimSeeds, TilePos, UnitId};
use rand::{rngs::OsRng, RngCore as _};
use serde::{Deserialize, Serialize};

use crate::limits::{
    BoundedText, BoundedVec, MAX_BUILD_IDENTITY_BYTES, MAX_IDENTITY_BYTES, MAX_PARTY_MEMBERS,
};

/// Current serialized multiplayer protocol schema.
pub const SESSION_PROTOCOL_VERSION: u16 = 1;

/// Version of the transport-neutral game protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProtocolVersion(pub u16);

impl Default for ProtocolVersion {
    fn default() -> Self {
        Self(SESSION_PROTOCOL_VERSION)
    }
}

/// Exact build identity required for admission.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BuildIdentityV1 {
    /// Human-readable package version.
    pub version: BoundedText<MAX_BUILD_IDENTITY_BYTES>,
    /// Source revision or reproducible build identifier.
    pub revision: BoundedText<MAX_BUILD_IDENTITY_BYTES>,
}

impl BuildIdentityV1 {
    /// Creates a bounded exact build identity.
    pub fn new(
        version: impl Into<String>,
        revision: impl Into<String>,
    ) -> Result<Self, crate::BoundError> {
        Ok(Self {
            version: BoundedText::new(version)?,
            revision: BoundedText::new(revision)?,
        })
    }
}

/// Exact accepted shipped-content digest.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentFingerprint(pub u64);

/// Random identity of one concrete host session.
///
/// This is not a credential. It prevents reconnect state, live snapshots, and typed
/// closure messages from one run being applied to another run that happens to use the
/// same scenario, endpoint, or build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SessionInstanceId([u8; Self::BYTE_LENGTH]);

impl SessionInstanceId {
    /// Random identity size (128 bits).
    pub const BYTE_LENGTH: usize = 16;

    /// Generates a fresh identity from the operating system random source.
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0_u8; Self::BYTE_LENGTH];
        while bytes == [0; Self::BYTE_LENGTH] {
            OsRng.fill_bytes(&mut bytes);
        }
        Self(bytes)
    }

    /// Constructs an identity from exact bytes for decoding and deterministic tests.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; Self::BYTE_LENGTH]) -> Self {
        Self(bytes)
    }

    /// Returns the exact identity bytes.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; Self::BYTE_LENGTH] {
        self.0
    }

    /// Whether this value is a generated/assigned session identity.
    #[must_use]
    pub fn is_valid(self) -> bool {
        self.0 != [0; Self::BYTE_LENGTH]
    }
}

/// Version-1 complete public terrain/world digest.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PublicWorldFingerprintV1(pub u64);

/// Compatibility name retained for the existing launch-manifest API.
pub use PublicWorldFingerprintV1 as PublicWorldFingerprint;

/// Frozen map identity and deterministic generation inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapManifestV1 {
    /// Stable built-in catalog identity.
    pub catalog_identity: BoundedText<MAX_IDENTITY_BYTES>,
    /// Resolved map seed used by every peer.
    pub seed: u64,
    /// Stable generator identity rather than a Rust type name.
    pub generator_identity: BoundedText<MAX_IDENTITY_BYTES>,
    /// Exact generator schema version.
    pub generator_version: u32,
    /// Fingerprint expected from each peer before activation.
    pub expected_public_fingerprint: PublicWorldFingerprint,
}

/// Frozen gameplay-rule identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RulesManifestV1 {
    /// Stable named shipped rules profile.
    pub profile_identity: BoundedText<MAX_IDENTITY_BYTES>,
    /// Digest of all resolved rule values used by authority.
    pub fingerprint: u64,
}

/// One shipped party character named by stable session identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterEntryV1 {
    /// Stable session unit id.
    pub unit: UnitId,
    /// Built-in archetype identity, never custom Creator content.
    pub archetype_identity: BoundedText<MAX_IDENTITY_BYTES>,
    /// Shipped display identity used to verify the selected character definition.
    pub character_identity: BoundedText<MAX_IDENTITY_BYTES>,
    /// Party faction. Version 1 accepts only [`Faction::Player`].
    pub faction: Faction,
}

/// Exact initial placement for one shipped party character.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitDeploymentV1 {
    /// Stable session unit id.
    pub unit: UnitId,
    /// Exact voxel surface selected during deployment.
    pub position: TilePos,
}

/// How peers obtain the immutable world and actor baseline for this session.
///
/// A Sandbox launch is reproducible from the frozen manifest. A Campaign launch must
/// import the host's complete disclosure-safe live snapshot; regenerating its original
/// map would discard authoritative mutations retained by the Campaign checkpoint.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionLaunchKindV1 {
    /// Generate the shipped Sandbox world from [`SessionManifestV1::map`].
    #[default]
    Sandbox,
    /// Wait for and transactionally import the host's complete live baseline.
    Campaign,
}

/// Immutable launch contract frozen before a direct lobby opens.
///
/// This type intentionally contains no transport ids, credentials, cameras, or local
/// selection state. It is suitable for Direct Connect today and Steam later.
#[derive(Message, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionManifestV1 {
    /// Random identity of this concrete host session.
    pub session_instance_id: SessionInstanceId,
    /// Protocol schema version.
    pub protocol: ProtocolVersion,
    /// Exact executable build identity.
    pub build: BuildIdentityV1,
    /// Exact accepted shipped-content revision.
    pub content_fingerprint: ContentFingerprint,
    /// Stable built-in scenario identity.
    pub scenario_identity: BoundedText<MAX_IDENTITY_BYTES>,
    /// Whether peers generate a fresh Sandbox or import a Campaign baseline.
    pub launch_kind: SessionLaunchKindV1,
    /// Frozen deterministic map contract.
    pub map: MapManifestV1,
    /// Frozen gameplay rules contract.
    pub rules: RulesManifestV1,
    /// Existing shipped six-character party in stable slot order.
    pub shipped_roster: BoundedVec<RosterEntryV1, MAX_PARTY_MEMBERS>,
    /// Exact initial deployment in stable unit-id order.
    pub deployment: BoundedVec<UnitDeploymentV1, MAX_PARTY_MEMBERS>,
    /// All deterministic simulation seed domains.
    pub simulation_seeds: SimSeeds,
}

impl SessionManifestV1 {
    /// Validates protocol version, shipped party shape, stable identity uniqueness, and
    /// complete deployment membership.
    pub fn validate(&self) -> Result<(), ManifestValidationError> {
        if !self.session_instance_id.is_valid() {
            return Err(ManifestValidationError::InvalidSessionInstanceId);
        }
        if self.protocol != ProtocolVersion::default() {
            return Err(ManifestValidationError::UnsupportedProtocol);
        }
        if self.shipped_roster.is_empty() {
            return Err(ManifestValidationError::EmptyRoster);
        }

        let mut roster_ids = BTreeSet::new();
        for entry in self.shipped_roster.as_slice() {
            if entry.faction != Faction::Player {
                return Err(ManifestValidationError::NonPlayerRosterEntry(entry.unit));
            }
            if !roster_ids.insert(entry.unit) {
                return Err(ManifestValidationError::DuplicateRosterUnit(entry.unit));
            }
        }

        let mut deployment_ids = BTreeSet::new();
        for placement in self.deployment.as_slice() {
            if !roster_ids.contains(&placement.unit) {
                return Err(ManifestValidationError::UnknownDeploymentUnit(
                    placement.unit,
                ));
            }
            if !deployment_ids.insert(placement.unit) {
                return Err(ManifestValidationError::DuplicateDeploymentUnit(
                    placement.unit,
                ));
            }
        }
        if deployment_ids != roster_ids {
            return Err(ManifestValidationError::IncompleteDeployment);
        }
        Ok(())
    }
}

/// Why a frozen launch manifest is structurally invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestValidationError {
    /// A zero/unassigned session identity cannot bind reconnect state.
    InvalidSessionInstanceId,
    /// The manifest uses a protocol schema this build cannot interpret.
    UnsupportedProtocol,
    /// At least one shipped party member is required.
    EmptyRoster,
    /// Version 1 party rosters may not carry hostile/custom actors.
    NonPlayerRosterEntry(UnitId),
    /// Two roster slots name the same stable unit.
    DuplicateRosterUnit(UnitId),
    /// A deployment names a unit absent from the shipped party.
    UnknownDeploymentUnit(UnitId),
    /// Two deployment entries name the same stable unit.
    DuplicateDeploymentUnit(UnitId),
    /// Every shipped party character must have one initial placement.
    IncompleteDeployment,
}

impl fmt::Display for ManifestValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidSessionInstanceId => "manifest session instance id is invalid",
            Self::UnsupportedProtocol => "manifest protocol version is unsupported",
            Self::EmptyRoster => "manifest shipped roster is empty",
            Self::NonPlayerRosterEntry(_) => "manifest roster contains a non-player unit",
            Self::DuplicateRosterUnit(_) => "manifest roster repeats a unit id",
            Self::UnknownDeploymentUnit(_) => "manifest deployment names an unknown unit",
            Self::DuplicateDeploymentUnit(_) => "manifest deployment repeats a unit id",
            Self::IncompleteDeployment => "manifest deployment does not cover the shipped roster",
        })
    }
}

impl std::error::Error for ManifestValidationError {}

#[cfg(test)]
mod tests {
    use hex_core::{HexCoord, TilePos};

    use super::*;

    fn text(value: &str) -> BoundedText<MAX_IDENTITY_BYTES> {
        BoundedText::new(value).expect("fixture identity should fit")
    }

    fn fixture() -> SessionManifestV1 {
        SessionManifestV1 {
            session_instance_id: SessionInstanceId::from_bytes([7; 16]),
            protocol: ProtocolVersion::default(),
            build: BuildIdentityV1::new("0.4.0", "fixture-revision")
                .expect("fixture build identity should fit"),
            content_fingerprint: ContentFingerprint(11),
            scenario_identity: text("sandbox"),
            launch_kind: SessionLaunchKindV1::Sandbox,
            map: MapManifestV1 {
                catalog_identity: text("small-island"),
                seed: 42,
                generator_identity: text("procedural-v3"),
                generator_version: 3,
                expected_public_fingerprint: PublicWorldFingerprint(22),
            },
            rules: RulesManifestV1 {
                profile_identity: text("default"),
                fingerprint: 33,
            },
            shipped_roster: BoundedVec::new(vec![RosterEntryV1 {
                unit: UnitId(0),
                archetype_identity: text("warrior"),
                character_identity: text("shipped-warrior"),
                faction: Faction::Player,
            }])
            .expect("one roster entry fits"),
            deployment: BoundedVec::new(vec![UnitDeploymentV1 {
                unit: UnitId(0),
                position: TilePos::new(HexCoord::ORIGIN, 1),
            }])
            .expect("one deployment entry fits"),
            simulation_seeds: SimSeeds {
                world: 1,
                ai_flavor: 2,
                cosmetic: 3,
            },
        }
    }

    #[test]
    fn manifest_round_trip_retains_every_launch_identity() {
        let manifest = fixture();
        assert_eq!(manifest.validate(), Ok(()));
        let json = serde_json::to_string(&manifest).expect("manifest should serialize");
        let restored: SessionManifestV1 =
            serde_json::from_str(&json).expect("manifest should deserialize");
        assert_eq!(restored, manifest);
        assert_eq!(restored.validate(), Ok(()));
    }

    #[test]
    fn manifest_rejects_incomplete_or_foreign_deployment() {
        let mut unbound = fixture();
        unbound.session_instance_id = SessionInstanceId::from_bytes([0; 16]);
        assert_eq!(
            unbound.validate(),
            Err(ManifestValidationError::InvalidSessionInstanceId)
        );

        let mut manifest = fixture();
        manifest.deployment = BoundedVec::default();
        assert_eq!(
            manifest.validate(),
            Err(ManifestValidationError::IncompleteDeployment)
        );

        manifest.deployment = BoundedVec::new(vec![UnitDeploymentV1 {
            unit: UnitId(99),
            position: TilePos::ORIGIN,
        }])
        .expect("one deployment entry fits");
        assert_eq!(
            manifest.validate(),
            Err(ManifestValidationError::UnknownDeploymentUnit(UnitId(99)))
        );
    }
}
