//! Generator-neutral live-world export, validation, fingerprinting, and deltas.
//!
//! `hex_multiplayer` owns the bounded wire vocabulary. This module is the only
//! adapter allowed to inspect map-private voxel and presentation state, resolve
//! stable names against shipped content, and turn an accepted snapshot back into
//! resources suitable for the ordinary `TerrainReady` publication path.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use bevy::prelude::{Message, Resource, World};
use hex_assets::{HexObjectRotation, ObjectAssetId, RuntimeArtCatalog, SubstanceTable};
use hex_core::{
    BiomeRegionId, BiomeRegions, DamagedVoxels, HexCoord, IlluminationLevel, InteriorRegionId,
    InteriorRegions, MapAnchorId, MapAnchors, MapViewHint, SpecialMovementRegion,
    SpecialMovementRegions, TerrainVoxelHealth, TilePos, TraversalBlockers, MAX_HEADROOM,
};
use hex_multiplayer::{
    AuthoritySequence, BiomeRegionSnapshotV1, BoundError, BoundedText, BoundedVec,
    InteriorRoofSnapshotV1, InteriorSurfaceSnapshotV1, PublicWorldFingerprint,
    SpecialRegionSnapshotV1, WorldAnchorSnapshotV1, WorldColumnSnapshotV1, WorldDamageSnapshotV1,
    WorldDeltaOperationV1, WorldDeltaV1, WorldIlluminationV1, WorldLightSnapshotV1,
    WorldLiquidFlowV1, WorldLiquidSnapshotV1, WorldObjectSnapshotV1, WorldRunSnapshotV1,
    WorldSnapshotV1, WorldSnapshotValidationError, WorldViewHintSnapshotV1,
    MAX_ABS_COMMAND_COORDINATE, MAX_ABS_COMMAND_LEVEL, MAX_IDENTITY_BYTES,
    MAX_OBJECT_BLOCKER_SURFACES, MAX_WORLD_COLUMNS, MAX_WORLD_DELTA_OPERATIONS,
    WORLD_DELTA_VERSION_V1, WORLD_SNAPSHOT_VERSION_V1,
};
use xxhash_rust::xxh3::xxh3_64;

use crate::procedural_v3::{
    CaveCrystalKind, CaveCrystalObjectSet, CaveCrystalPresentation, CaveCrystalSiteKind,
    CrystalAscentCrystalKind, CrystalAscentCrystalPresentation, CrystalAscentObjectSet, FeatureId,
    FeatureKind, FillMaterialRole, LightId, LiquidFlowState, MapPresentationProjection,
    MaterializedLiquidVoxel, PlannedFeature, PlannedGameplayLight, PlannedLightPresentation,
};
use crate::settings::MapSettings;
use crate::voxel::{runs, Column, VoxelMap};

const FINGERPRINT_DOMAIN: &[u8] = b"hex-public-world-fingerprint-v1";
const FEATURE_ID_PREFIX: &str = "feature:";
const LIGHT_ID_PREFIX: &str = "light:";

/// Exact current world snapshot retained for launch verification and reconnect.
///
/// This resource is a cache of map-owned truth, not an independent authority. The
/// map replaces it after generation, mutation, damage, or snapshot import.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct CurrentWorldSnapshotV1(WorldSnapshotV1);

impl CurrentWorldSnapshotV1 {
    /// Borrows the complete canonical snapshot.
    #[must_use]
    pub const fn snapshot(&self) -> &WorldSnapshotV1 {
        &self.0
    }

    /// Current complete public-world fingerprint.
    #[must_use]
    pub const fn fingerprint(&self) -> PublicWorldFingerprint {
        self.0.public_fingerprint
    }

    /// Consumes the cache wrapper.
    #[must_use]
    pub fn into_snapshot(self) -> WorldSnapshotV1 {
        self.0
    }

    pub(crate) fn new(snapshot: WorldSnapshotV1) -> Self {
        Self(snapshot)
    }

    pub(crate) fn refresh_changed_coordinates(
        &mut self,
        source: WorldExportParts<'_>,
        changed_coords: &BTreeSet<HexCoord>,
    ) -> Result<(), WorldSnapshotError> {
        refresh_snapshot_from_parts(&mut self.0, source, changed_coords)
    }
}

/// Complete world waiting to replace generator output during Campaign bootstrap.
///
/// The shared save/session layer inserts this resource before entering gameplay. Map
/// authority consumes it exactly once in [`hex_core::GameplaySetup::Resources`], validates
/// it against the accepted shipped content, and publishes it through the ordinary
/// `TerrainReady` path. The option is private so callers cannot construct an empty request.
#[derive(Resource, Debug)]
pub struct PendingCampaignWorldSnapshotV2 {
    snapshot: Option<Box<WorldSnapshotV1>>,
}

impl PendingCampaignWorldSnapshotV2 {
    /// Wraps one complete generator-neutral Campaign world.
    #[must_use]
    pub fn new(snapshot: WorldSnapshotV1) -> Self {
        Self {
            snapshot: Some(Box::new(snapshot)),
        }
    }

    /// Borrows the pending snapshot without exposing map-private prepared state.
    #[must_use]
    pub fn snapshot(&self) -> Option<&WorldSnapshotV1> {
        self.snapshot.as_deref()
    }

    pub(crate) fn take(&mut self) -> Option<WorldSnapshotV1> {
        self.snapshot.take().map(|snapshot| *snapshot)
    }
}

/// Typed result retained after a Campaign world bootstrap attempt.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct CampaignWorldRestoreResultV2 {
    /// Applied or fail-closed outcome.
    pub outcome: CampaignWorldRestoreOutcomeV2,
}

/// Result of restoring the world portion of a host Campaign checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CampaignWorldRestoreOutcomeV2 {
    /// The exact snapshot was validated and published as ordinary terrain.
    Applied {
        /// Complete resulting public-world identity.
        public_fingerprint: PublicWorldFingerprint,
    },
    /// The candidate did not activate any world.
    Refused(CampaignWorldRestoreRefusalV2),
}

/// Why a pending Campaign world was rejected before actor restoration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CampaignWorldRestoreRefusalV2 {
    /// The pending wrapper had already been consumed.
    MissingSnapshot,
    /// Structural, content, or semantic validation failed before ECS mutation.
    InvalidSnapshot(WorldSnapshotError),
    /// Validated map truth could not be published through the presentation path.
    PresentationFailed(String),
}

/// Ordered world-owned state application requested by a transport/session adapter.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum WorldReplicationRequestV1 {
    /// Replace the current world with a restart-capable baseline.
    Restore {
        /// Authority boundary represented by the snapshot.
        baseline_sequence: AuthoritySequence,
        /// Complete generator-neutral world state.
        snapshot: Box<WorldSnapshotV1>,
    },
    /// Apply one later authority-boundary delta.
    ApplyDelta(WorldDeltaV1),
}

impl WorldReplicationRequestV1 {
    /// Sequence owned by this request.
    #[must_use]
    pub const fn authority_sequence(&self) -> AuthoritySequence {
        match self {
            Self::Restore {
                baseline_sequence, ..
            } => *baseline_sequence,
            Self::ApplyDelta(delta) => delta.authority_sequence,
        }
    }
}

/// Typed local result after map validation and presentation publication.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct WorldReplicationResultV1 {
    /// Correlated authority sequence.
    pub authority_sequence: AuthoritySequence,
    /// Accepted, duplicate, or fail-closed outcome.
    pub outcome: WorldReplicationOutcomeV1,
}

/// Result of one ordered world replication request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorldReplicationOutcomeV1 {
    /// Candidate validated, committed, and republished as `TerrainReady`.
    Applied {
        /// Exact resulting public world.
        public_fingerprint: PublicWorldFingerprint,
    },
    /// This exact sequence/target was already committed.
    Duplicate {
        /// Exact already-current public world.
        public_fingerprint: PublicWorldFingerprint,
    },
    /// The request changed nothing.
    Refused(WorldReplicationRefusalV1),
}

/// Why a live snapshot or delta was not applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorldReplicationRefusalV1 {
    /// Commands, decisions, or movement still own the authority boundary.
    BoundaryBusy,
    /// More ordered world mutations arrived in one update than the bounded adapter accepts.
    RequestBurstExceeded,
    /// A delta arrived before a complete baseline/current world existed.
    MissingCurrentWorld,
    /// A sequence older than the last committed boundary arrived.
    StaleSequence,
    /// The same sequence named a different target fingerprint.
    SequenceConflict,
    /// Bounded structural/content/presentation validation failed.
    InvalidSnapshot(WorldSnapshotError),
    /// The candidate could not enter the ordinary presentation path.
    PresentationFailed(String),
}

/// Last authority boundary committed by the map-owned replica adapter.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WorldReplicationStateV1 {
    last_applied_sequence: Option<AuthoritySequence>,
}

impl WorldReplicationStateV1 {
    /// Last imported baseline or delta sequence, if any.
    #[must_use]
    pub const fn last_applied_sequence(&self) -> Option<AuthoritySequence> {
        self.last_applied_sequence
    }

    pub(crate) fn set_last_applied_sequence(&mut self, value: Option<AuthoritySequence>) {
        self.last_applied_sequence = value;
    }
}

/// Fully resolved candidate. Construction performs no ECS mutation.
pub(crate) struct PreparedWorldSnapshotV1 {
    pub(crate) snapshot: WorldSnapshotV1,
    pub(crate) map: VoxelMap,
    pub(crate) damage: Vec<(TilePos, TerrainVoxelHealth)>,
    pub(crate) anchors: MapAnchors,
    pub(crate) interiors: InteriorRegions,
    pub(crate) special_regions: SpecialMovementRegions,
    pub(crate) biome_regions: BiomeRegions,
    pub(crate) blockers: TraversalBlockers,
    pub(crate) view_hint: Option<MapViewHint>,
    pub(crate) presentation: Option<MapPresentationProjection>,
}

/// Why map-owned snapshot work failed before mutating the live world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorldSnapshotError {
    /// A required live resource did not exist.
    WorldUnavailable(&'static str),
    /// Shared structural/canonical validation failed.
    Structural(WorldSnapshotValidationError),
    /// One bounded shared collection could not be constructed.
    Bound(BoundError),
    /// One named top-level world projection exceeded its shared collection bound.
    CollectionBound {
        /// Stable snapshot field name.
        collection: &'static str,
        /// Exact shared bound failure.
        source: BoundError,
    },
    /// A stable substance name is absent from accepted shipped content.
    UnknownSubstance(String),
    /// Air was supplied where an occupied voxel/material is required.
    AirAsMaterial(String),
    /// A stable authored-object id is invalid or absent from accepted content.
    UnknownObject(String),
    /// A map-local presentation identity is malformed or has the wrong family.
    InvalidPresentationIdentity(String),
    /// Stable state does not reproduce the exact public tuple supplied on the wire.
    ProjectionMismatch(TilePos),
    /// Partial health disagrees with the current substance toughness.
    DamageMismatch(TilePos),
    /// A presentation consequence disagrees with voxel/public world state.
    PresentationMismatch(String),
    /// The supplied or computed public fingerprint does not match canonical state.
    FingerprintMismatch {
        /// Fingerprint named by the input.
        expected: PublicWorldFingerprint,
        /// Fingerprint computed by this adapter.
        actual: PublicWorldFingerprint,
    },
    /// A delta names a different base world.
    DeltaBaseMismatch {
        /// Fingerprint required by the delta.
        expected: PublicWorldFingerprint,
        /// Fingerprint of the supplied base snapshot.
        actual: PublicWorldFingerprint,
    },
}

impl fmt::Display for WorldSnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WorldUnavailable(resource) => {
                write!(formatter, "world snapshot requires live {resource}")
            }
            Self::Structural(error) => write!(formatter, "invalid world snapshot: {error}"),
            Self::Bound(error) => write!(formatter, "world snapshot exceeds a bound: {error}"),
            Self::CollectionBound { collection, source } => write!(
                formatter,
                "world snapshot collection '{collection}' exceeds its bound: {source}"
            ),
            Self::UnknownSubstance(name) => {
                write!(formatter, "world snapshot names unknown substance '{name}'")
            }
            Self::AirAsMaterial(name) => {
                write!(
                    formatter,
                    "world snapshot uses air as occupied material '{name}'"
                )
            }
            Self::UnknownObject(name) => {
                write!(formatter, "world snapshot names unknown object '{name}'")
            }
            Self::InvalidPresentationIdentity(identity) => write!(
                formatter,
                "world snapshot contains invalid presentation identity '{identity}'"
            ),
            Self::ProjectionMismatch(position) => write!(
                formatter,
                "world snapshot public run tuple disagrees at {position:?}"
            ),
            Self::DamageMismatch(position) => {
                write!(formatter, "world snapshot damage disagrees at {position:?}")
            }
            Self::PresentationMismatch(reason) => {
                write!(formatter, "world snapshot presentation disagrees: {reason}")
            }
            Self::FingerprintMismatch { expected, actual } => write!(
                formatter,
                "world snapshot fingerprint mismatch: expected {}, computed {}",
                expected.0, actual.0
            ),
            Self::DeltaBaseMismatch { expected, actual } => write!(
                formatter,
                "world delta base mismatch: expected {}, received {}",
                expected.0, actual.0
            ),
        }
    }
}

impl std::error::Error for WorldSnapshotError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Structural(error) => Some(error),
            Self::Bound(error) | Self::CollectionBound { source: error, .. } => Some(error),
            Self::WorldUnavailable(_)
            | Self::UnknownSubstance(_)
            | Self::AirAsMaterial(_)
            | Self::UnknownObject(_)
            | Self::InvalidPresentationIdentity(_)
            | Self::ProjectionMismatch(_)
            | Self::DamageMismatch(_)
            | Self::PresentationMismatch(_)
            | Self::FingerprintMismatch { .. }
            | Self::DeltaBaseMismatch { .. } => None,
        }
    }
}

impl From<WorldSnapshotValidationError> for WorldSnapshotError {
    fn from(value: WorldSnapshotValidationError) -> Self {
        Self::Structural(value)
    }
}

impl From<BoundError> for WorldSnapshotError {
    fn from(value: BoundError) -> Self {
        Self::Bound(value)
    }
}

/// Exports current map-owned truth from an ECS world without mutating it.
pub fn export_world_snapshot_v1(world: &World) -> Result<WorldSnapshotV1, WorldSnapshotError> {
    let map = required::<VoxelMap>(world, "VoxelMap")?;
    let table = required::<SubstanceTable>(world, "SubstanceTable")?;
    let settings = required::<MapSettings>(world, "MapSettings")?;
    let anchors = required::<MapAnchors>(world, "MapAnchors")?;
    let interiors = required::<InteriorRegions>(world, "InteriorRegions")?;
    let special_regions = required::<SpecialMovementRegions>(world, "SpecialMovementRegions")?;
    let empty_damage = DamagedVoxels::new();
    export_from_parts(WorldExportParts {
        map,
        table,
        settings,
        damage: world
            .get_resource::<DamagedVoxels>()
            .unwrap_or(&empty_damage),
        anchors,
        interiors,
        special_regions,
        biome_regions: world.get_resource::<BiomeRegions>(),
        blockers: world.get_resource::<TraversalBlockers>(),
        view_hint: world.get_resource::<MapViewHint>(),
        presentation: world.get_resource::<MapPresentationProjection>(),
        art_catalog: world.get_resource::<RuntimeArtCatalog>(),
    })
}

fn required<'a, T: Resource>(
    world: &'a World,
    name: &'static str,
) -> Result<&'a T, WorldSnapshotError> {
    world
        .get_resource::<T>()
        .ok_or(WorldSnapshotError::WorldUnavailable(name))
}

pub(crate) struct WorldExportParts<'a> {
    pub(crate) map: &'a VoxelMap,
    pub(crate) table: &'a SubstanceTable,
    pub(crate) settings: &'a MapSettings,
    pub(crate) damage: &'a DamagedVoxels,
    pub(crate) anchors: &'a MapAnchors,
    pub(crate) interiors: &'a InteriorRegions,
    pub(crate) special_regions: &'a SpecialMovementRegions,
    pub(crate) biome_regions: Option<&'a BiomeRegions>,
    pub(crate) blockers: Option<&'a TraversalBlockers>,
    pub(crate) view_hint: Option<&'a MapViewHint>,
    pub(crate) presentation: Option<&'a MapPresentationProjection>,
    pub(crate) art_catalog: Option<&'a RuntimeArtCatalog>,
}

pub(crate) fn export_from_parts(
    source: WorldExportParts<'_>,
) -> Result<WorldSnapshotV1, WorldSnapshotError> {
    let mut coordinates = source.map.columns().collect::<Vec<_>>();
    coordinates.sort_by_key(|(coord, _column)| *coord);
    let mut columns = Vec::with_capacity(coordinates.len());
    let mut all_surfaces = Vec::new();
    for (coord, column) in coordinates {
        let Some(projected) =
            export_column(coord, column, source.table, source.settings.level_height)?
        else {
            continue;
        };
        all_surfaces.extend(projected.runs.iter().map(|run| run.position));
        columns.push(projected);
    }

    let damage = source
        .damage
        .iter()
        .map(|(position, health)| {
            let substance = source.map.get(position);
            if substance.is_air() || source.table.toughness(substance) != Some(health.maximum) {
                return Err(WorldSnapshotError::DamageMismatch(position));
            }
            Ok(WorldDamageSnapshotV1 {
                position,
                remaining: health.remaining,
                maximum: health.maximum,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut anchors = source
        .anchors
        .iter()
        .map(|(name, position)| {
            Ok(WorldAnchorSnapshotV1 {
                name: bounded_text(name.as_str())?,
                position,
            })
        })
        .collect::<Result<Vec<_>, WorldSnapshotError>>()?;
    anchors.sort_by(|left, right| left.name.cmp(&right.name));

    let mut interior_surfaces = source
        .interiors
        .surfaces()
        .map(|(position, region)| InteriorSurfaceSnapshotV1 {
            position,
            region: region.0,
        })
        .collect::<Vec<_>>();
    interior_surfaces.sort_by_key(|entry| entry.position);
    let mut interior_roofs = source
        .interiors
        .roof_voxels()
        .map(|(position, region)| InteriorRoofSnapshotV1 {
            position,
            region: region.0,
        })
        .collect::<Vec<_>>();
    interior_roofs.sort_by_key(|entry| entry.position);
    let mut special_regions = source
        .special_regions
        .iter()
        .map(|(position, region)| SpecialRegionSnapshotV1 {
            position,
            region: region.0,
        })
        .collect::<Vec<_>>();
    special_regions.sort_by_key(|entry| entry.position);
    let biome_regions = source.biome_regions.map_or_else(Vec::new, |regions| {
        regions
            .iter()
            .map(|(position, region)| BiomeRegionSnapshotV1 {
                position,
                region: region.0,
            })
            .collect()
    });
    let blockers = source
        .blockers
        .map_or_else(Vec::new, |blockers| blockers.iter().collect());
    let view_hint = source.view_hint.map(|hint| WorldViewHintSnapshotV1 {
        eye_bits: [
            hint.eye.0.to_bits(),
            hint.eye.1.to_bits(),
            hint.eye.2.to_bits(),
        ],
        focus_bits: [
            hint.focus.0.to_bits(),
            hint.focus.1.to_bits(),
            hint.focus.2.to_bits(),
        ],
    });

    let (lights, liquids, objects) = export_presentation(
        source.presentation,
        source.art_catalog,
        source.map,
        source.table,
        &all_surfaces,
    )?;

    let mut snapshot = WorldSnapshotV1 {
        version: WORLD_SNAPSHOT_VERSION_V1,
        public_fingerprint: PublicWorldFingerprint(0),
        columns: bounded_collection("columns", columns)?,
        damage: bounded_collection("damage", damage)?,
        anchors: bounded_collection("anchors", anchors)?,
        interior_surfaces: bounded_collection("interior_surfaces", interior_surfaces)?,
        interior_roofs: bounded_collection("interior_roofs", interior_roofs)?,
        special_regions: bounded_collection("special_regions", special_regions)?,
        biome_regions: bounded_collection("biome_regions", biome_regions)?,
        blockers: bounded_collection("blockers", blockers)?,
        view_hint,
        lights: bounded_collection("lights", lights)?,
        liquids: bounded_collection("liquids", liquids)?,
        objects: bounded_collection("objects", objects)?,
    };
    snapshot.validate()?;
    snapshot.public_fingerprint = fingerprint_world_snapshot_v1(&snapshot)?;
    Ok(snapshot)
}

fn export_column(
    coord: HexCoord,
    column: &Column,
    table: &SubstanceTable,
    level_height: f32,
) -> Result<Option<WorldColumnSnapshotV1>, WorldSnapshotError> {
    let projected = runs(column);
    if projected.is_empty() {
        return Ok(None);
    }
    let mut wire_runs = Vec::with_capacity(projected.len());
    for run in projected {
        let position = TilePos::new(coord, run.top.saturating_sub(1));
        let name = table
            .name(run.substance)
            .ok_or_else(|| WorldSnapshotError::UnknownSubstance(format!("{:?}", run.substance)))?;
        if run.substance.is_air() || name == "air" {
            return Err(WorldSnapshotError::AirAsMaterial(name.to_owned()));
        }
        let (span_bottom_bits, span_top_bits) = span_bits(run.bottom, run.top, level_height);
        wire_runs.push(WorldRunSnapshotV1 {
            position,
            run_bottom: run.bottom,
            span_bottom_bits,
            span_top_bits,
            substance: bounded_text(name)?,
            headroom: column.headroom_above(run.top).0,
        });
    }
    Ok(Some(WorldColumnSnapshotV1 {
        coord,
        runs: BoundedVec::new(wire_runs)?,
    }))
}

/// Reprojects only consequences owned by an accepted terrain-edit coordinate.
///
/// Terrain edits cannot mutate anchors, framing, authored liquids, or gameplay
/// lights, and their conservative guards keep every light/liquid object intact.
/// Keeping those bounded collections in place is both cheaper and stronger than
/// recreating their stable identities. Surface features are the exception: grass
/// and cave vegetation may retire when their support changes, so their object
/// consequences are reconciled against the surviving private feature IDs.
fn refresh_snapshot_from_parts(
    snapshot: &mut WorldSnapshotV1,
    source: WorldExportParts<'_>,
    changed_coords: &BTreeSet<HexCoord>,
) -> Result<(), WorldSnapshotError> {
    if changed_coords.is_empty() {
        return Err(WorldSnapshotError::PresentationMismatch(
            "incremental snapshot refresh named no changed coordinates".to_owned(),
        ));
    }

    let mut changed_columns = BTreeMap::new();
    let mut changed_surfaces = Vec::new();
    for coord in changed_coords {
        let projected = source
            .map
            .column(*coord)
            .map(|column| export_column(*coord, column, source.table, source.settings.level_height))
            .transpose()?
            .flatten();
        if let Some(column) = &projected {
            changed_surfaces.extend(column.runs.iter().map(|run| run.position));
        }
        changed_columns.insert(*coord, projected);
    }

    let mut damage = Vec::new();
    for coord in changed_coords {
        let Some(column) = source.map.column(*coord) else {
            continue;
        };
        for level in 0..column.top() {
            let position = TilePos::new(*coord, level);
            let Some(health) = source.damage.get(position) else {
                continue;
            };
            let substance = source.map.get(position);
            if substance.is_air() || source.table.toughness(substance) != Some(health.maximum) {
                return Err(WorldSnapshotError::DamageMismatch(position));
            }
            damage.push(WorldDamageSnapshotV1 {
                position,
                remaining: health.remaining,
                maximum: health.maximum,
            });
        }
    }

    let interior_surfaces = changed_surfaces
        .iter()
        .filter_map(|position| {
            source
                .interiors
                .get(*position)
                .map(|region| InteriorSurfaceSnapshotV1 {
                    position: *position,
                    region: region.0,
                })
        })
        .collect();
    let special_regions = changed_surfaces
        .iter()
        .filter_map(|position| {
            source
                .special_regions
                .get(*position)
                .map(|region| SpecialRegionSnapshotV1 {
                    position: *position,
                    region: region.0,
                })
        })
        .collect();
    let biome_regions = source.biome_regions.map_or_else(Vec::new, |regions| {
        changed_surfaces
            .iter()
            .filter_map(|position| {
                regions.get(*position).map(|region| BiomeRegionSnapshotV1 {
                    position: *position,
                    region: region.0,
                })
            })
            .collect()
    });
    let blockers = source.blockers.map_or_else(Vec::new, |blockers| {
        changed_surfaces
            .iter()
            .copied()
            .filter(|position| blockers.contains(*position))
            .collect()
    });

    // Accepted edits only remove authored roof membership. Re-read the prior
    // coordinate keys through the live resource so removal and region replacement
    // remain exact without scanning every roof voxel in a cathedral-sized world.
    let interior_roofs = snapshot
        .interior_roofs
        .iter()
        .filter(|entry| changed_coords.contains(&entry.position.coord))
        .filter_map(|entry| {
            source
                .interiors
                .roof_region(entry.position)
                .map(|region| InteriorRoofSnapshotV1 {
                    position: entry.position,
                    region: region.0,
                })
        })
        .collect();

    replace_changed_columns(&mut snapshot.columns, changed_columns)?;
    replace_changed_positions(
        "damage",
        &mut snapshot.damage,
        changed_coords,
        damage,
        |entry| entry.position,
    )?;
    replace_changed_positions(
        "interior_surfaces",
        &mut snapshot.interior_surfaces,
        changed_coords,
        interior_surfaces,
        |entry| entry.position,
    )?;
    replace_changed_positions(
        "interior_roofs",
        &mut snapshot.interior_roofs,
        changed_coords,
        interior_roofs,
        |entry| entry.position,
    )?;
    replace_changed_positions(
        "special_regions",
        &mut snapshot.special_regions,
        changed_coords,
        special_regions,
        |entry| entry.position,
    )?;
    replace_changed_positions(
        "biome_regions",
        &mut snapshot.biome_regions,
        changed_coords,
        biome_regions,
        |entry| entry.position,
    )?;
    replace_changed_positions(
        "blockers",
        &mut snapshot.blockers,
        changed_coords,
        blockers,
        |position| *position,
    )?;
    reconcile_presentation_objects(
        snapshot,
        source.presentation,
        source.map,
        source.art_catalog,
        changed_coords,
    )?;

    validate_incremental_snapshot_changes(snapshot, changed_coords)?;
    snapshot.public_fingerprint = fingerprint_canonical_world_snapshot_v1(snapshot);
    Ok(())
}

fn validate_incremental_snapshot_changes(
    snapshot: &WorldSnapshotV1,
    changed_coords: &BTreeSet<HexCoord>,
) -> Result<(), WorldSnapshotError> {
    if snapshot.columns.is_empty() {
        return Err(WorldSnapshotValidationError::EmptyWorld.into());
    }
    for column in snapshot
        .columns
        .iter()
        .filter(|column| changed_coords.contains(&column.coord))
    {
        let coordinate_probe = TilePos::new(column.coord, 0);
        if column.coord.x().unsigned_abs() > MAX_ABS_COMMAND_COORDINATE
            || column.coord.y().unsigned_abs() > MAX_ABS_COMMAND_COORDINATE
            || column.coord.z().unsigned_abs() > MAX_ABS_COMMAND_COORDINATE
        {
            return Err(
                WorldSnapshotValidationError::PositionOutsideDomain(coordinate_probe).into(),
            );
        }
        for run in column.runs.iter() {
            if run.position.coord != column.coord
                || run.run_bottom < 0
                || run.run_bottom > run.position.level
                || run.position.level.unsigned_abs() > MAX_ABS_COMMAND_LEVEL
                || !(0..=MAX_HEADROOM).contains(&run.headroom)
            {
                return Err(WorldSnapshotValidationError::InvalidRun(run.position).into());
            }
            let bottom = run.span_bottom();
            let top = run.span_top();
            if !bottom.is_finite() || !top.is_finite() || top <= bottom {
                return Err(WorldSnapshotValidationError::InvalidSpan(run.position).into());
            }
        }
    }
    for entry in snapshot
        .damage
        .iter()
        .filter(|entry| changed_coords.contains(&entry.position.coord))
    {
        if !snapshot.contains_voxel(entry.position) {
            return Err(WorldSnapshotValidationError::DanglingVoxel(entry.position).into());
        }
    }
    for position in snapshot
        .anchors
        .iter()
        .map(|entry| entry.position)
        .chain(
            snapshot
                .interior_surfaces
                .iter()
                .map(|entry| entry.position),
        )
        .chain(snapshot.special_regions.iter().map(|entry| entry.position))
        .chain(snapshot.biome_regions.iter().map(|entry| entry.position))
        .chain(snapshot.blockers.iter().copied())
        .chain(snapshot.lights.iter().map(|entry| entry.origin))
        .chain(snapshot.objects.iter().map(|entry| entry.root))
        .filter(|position| changed_coords.contains(&position.coord))
    {
        if !snapshot.contains_surface(position) {
            return Err(WorldSnapshotValidationError::DanglingSurface(position).into());
        }
    }
    for position in snapshot
        .interior_roofs
        .iter()
        .map(|entry| entry.position)
        .chain(snapshot.liquids.iter().map(|entry| entry.position))
        .filter(|position| changed_coords.contains(&position.coord))
    {
        if !snapshot.contains_voxel(position) {
            return Err(WorldSnapshotValidationError::DanglingVoxel(position).into());
        }
    }
    for position in snapshot
        .objects
        .iter()
        .flat_map(|entry| entry.blockers.iter().copied())
        .filter(|position| changed_coords.contains(&position.coord))
    {
        if !snapshot.contains_surface(position) {
            return Err(WorldSnapshotValidationError::DanglingSurface(position).into());
        }
    }
    Ok(())
}

fn replace_changed_columns(
    columns: &mut BoundedVec<WorldColumnSnapshotV1, MAX_WORLD_COLUMNS>,
    replacements: BTreeMap<HexCoord, Option<WorldColumnSnapshotV1>>,
) -> Result<(), WorldSnapshotError> {
    let mut values = std::mem::take(columns).into_vec();
    for (coord, replacement) in replacements {
        match values.binary_search_by_key(&coord, |column| column.coord) {
            Ok(index) => match replacement {
                Some(replacement) => {
                    if let Some(column) = values.get_mut(index) {
                        *column = replacement;
                    }
                }
                None => {
                    values.remove(index);
                }
            },
            Err(index) => {
                if let Some(replacement) = replacement {
                    values.insert(index, replacement);
                }
            }
        }
    }
    *columns = bounded_collection("columns", values)?;
    Ok(())
}

fn replace_changed_positions<T, const MAX: usize>(
    collection: &'static str,
    bounded: &mut BoundedVec<T, MAX>,
    changed_coords: &BTreeSet<HexCoord>,
    replacements: Vec<T>,
    position: impl Fn(&T) -> TilePos + Copy,
) -> Result<(), WorldSnapshotError> {
    let mut values = std::mem::take(bounded).into_vec();
    values.retain(|entry| !changed_coords.contains(&position(entry).coord));
    values.extend(replacements);
    values.sort_by_key(position);
    *bounded = bounded_collection(collection, values)?;
    Ok(())
}

fn reconcile_presentation_objects(
    snapshot: &mut WorldSnapshotV1,
    presentation: Option<&MapPresentationProjection>,
    map: &VoxelMap,
    catalog: Option<&RuntimeArtCatalog>,
    changed_coords: &BTreeSet<HexCoord>,
) -> Result<(), WorldSnapshotError> {
    let Some(presentation) = presentation else {
        if !snapshot.lights.is_empty()
            || !snapshot.liquids.is_empty()
            || !snapshot.objects.is_empty()
        {
            return Err(WorldSnapshotError::WorldUnavailable(
                "MapPresentationProjection",
            ));
        }
        return Ok(());
    };
    let live_features = presentation
        .features()
        .keys()
        .copied()
        .collect::<BTreeSet<_>>();
    let live_presented_lights = presentation
        .lights()
        .iter()
        .filter_map(|(id, light)| light.presentation.map(|_| *id))
        .collect::<BTreeSet<_>>();
    let mut seen_features = BTreeSet::new();
    let mut seen_lights = BTreeSet::new();
    for object in snapshot.objects.iter() {
        let identity = object.stable_id.as_str();
        if identity.starts_with(FEATURE_ID_PREFIX) {
            seen_features.insert(FeatureId(parse_local_id(identity, FEATURE_ID_PREFIX)?));
        } else {
            seen_lights.insert(LightId(parse_local_id(identity, LIGHT_ID_PREFIX)?));
        }
    }
    if live_features.iter().any(|id| !seen_features.contains(id)) {
        return Err(WorldSnapshotError::PresentationMismatch(
            "live feature projection has no current snapshot object".to_owned(),
        ));
    }
    if live_presented_lights
        .iter()
        .any(|id| !seen_lights.contains(id))
    {
        return Err(WorldSnapshotError::PresentationMismatch(
            "live presented light has no current snapshot object".to_owned(),
        ));
    }

    let mut objects = std::mem::take(&mut snapshot.objects).into_vec();
    objects.retain(|object| {
        let identity = object.stable_id.as_str();
        identity
            .strip_prefix(FEATURE_ID_PREFIX)
            .and_then(|suffix| suffix.parse::<u32>().ok())
            .is_none_or(|id| live_features.contains(&FeatureId(id)))
    });

    for (id, light) in presentation.lights() {
        let Some(light_presentation) = light.presentation else {
            continue;
        };
        let PlannedLightPresentation::CrystalAscent(CrystalAscentCrystalPresentation {
            kind: CrystalAscentCrystalKind::Heart,
            rotation: rotation_steps,
        }) = light_presentation
        else {
            continue;
        };
        let catalog = catalog.ok_or(WorldSnapshotError::WorldUnavailable("RuntimeArtCatalog"))?;
        let object_set = CrystalAscentObjectSet::resolve(catalog)
            .map_err(|error| WorldSnapshotError::UnknownObject(error.to_string()))?;
        let rotation = HexObjectRotation::new(rotation_steps)
            .map_err(|error| WorldSnapshotError::PresentationMismatch(error.to_string()))?;
        let visual_level = light.origin.level.checked_add(1).ok_or_else(|| {
            WorldSnapshotError::PresentationMismatch(
                "crystal heart visual origin overflowed".to_owned(),
            )
        })?;
        let visual_origin = TilePos::new(light.origin.coord, visual_level);
        let heart_runs = object_set
            .project_heart_runs(visual_origin, rotation)
            .ok_or_else(|| {
                WorldSnapshotError::PresentationMismatch(
                    "crystal heart occupied-volume projection overflowed".to_owned(),
                )
            })?;
        let heart_coords = heart_runs
            .iter()
            .map(|run| run.top.coord)
            .collect::<BTreeSet<_>>();
        if heart_coords.is_disjoint(changed_coords) {
            continue;
        }
        let supports = heart_coords
            .iter()
            .filter_map(|coord| map.column(*coord).map(|column| (*coord, column)))
            .flat_map(|(coord, column)| {
                runs(column)
                    .into_iter()
                    .map(move |run| TilePos::new(coord, run.top.saturating_sub(1)))
            })
            .collect::<Vec<_>>();
        let (asset, rotation, blockers) =
            crystal_object_consequence(light_presentation, light.origin, &supports, catalog)?;
        let replacement = WorldObjectSnapshotV1 {
            stable_id: bounded_text(stable_local_id(LIGHT_ID_PREFIX, id.0))?,
            asset_identity: bounded_text(asset.as_str())?,
            root: light.origin,
            rotation_sixths: rotation.steps(),
            blockers: BoundedVec::<_, MAX_OBJECT_BLOCKER_SURFACES>::new(blockers)?,
            protects_edits: true,
        };
        upsert(&mut objects, replacement, |object| object.stable_id.clone());
    }
    snapshot.objects = bounded_collection("objects", objects)?;
    Ok(())
}

fn export_presentation(
    projection: Option<&MapPresentationProjection>,
    catalog: Option<&RuntimeArtCatalog>,
    map: &VoxelMap,
    table: &SubstanceTable,
    all_surfaces: &[TilePos],
) -> Result<
    (
        Vec<WorldLightSnapshotV1>,
        Vec<WorldLiquidSnapshotV1>,
        Vec<WorldObjectSnapshotV1>,
    ),
    WorldSnapshotError,
> {
    let Some(projection) = projection else {
        return Ok((Vec::new(), Vec::new(), Vec::new()));
    };

    let liquids = projection
        .liquids()
        .iter()
        .map(|(position, liquid)| {
            let substance = map.get(*position);
            let name = table
                .name(substance)
                .ok_or_else(|| WorldSnapshotError::UnknownSubstance(format!("{:?}", substance)))?;
            let expected = liquid_role_for_name(name)?;
            if expected != liquid.material {
                return Err(WorldSnapshotError::PresentationMismatch(format!(
                    "liquid material at {position:?} does not match '{name}'"
                )));
            }
            Ok(WorldLiquidSnapshotV1 {
                position: *position,
                substance: bounded_text(name)?,
                flow: encode_flow(liquid.flow),
                downstream: liquid.downstream,
            })
        })
        .collect::<Result<Vec<_>, WorldSnapshotError>>()?;

    let mut objects = projection
        .features()
        .iter()
        .map(|(id, feature)| {
            Ok(WorldObjectSnapshotV1 {
                stable_id: bounded_text(stable_local_id(FEATURE_ID_PREFIX, id.0))?,
                asset_identity: bounded_text(feature.object_id.as_str())?,
                root: feature.root,
                rotation_sixths: feature.rotation.steps(),
                blockers: BoundedVec::<_, MAX_OBJECT_BLOCKER_SURFACES>::new(
                    feature.blocker_footprint.iter().copied().collect(),
                )?,
                protects_edits: feature.kind == FeatureKind::Tree,
            })
        })
        .collect::<Result<Vec<_>, WorldSnapshotError>>()?;

    let mut lights = Vec::with_capacity(projection.lights().len());
    for (id, light) in projection.lights() {
        let stable_id = stable_local_id(LIGHT_ID_PREFIX, id.0);
        lights.push(WorldLightSnapshotV1 {
            stable_id: bounded_text(&stable_id)?,
            origin: light.origin,
            illumination: encode_illumination(light.level),
            radius: light.radius,
        });
        let Some(presentation) = light.presentation else {
            continue;
        };
        let catalog = catalog.ok_or(WorldSnapshotError::WorldUnavailable("RuntimeArtCatalog"))?;
        let (asset, rotation, blockers) =
            crystal_object_consequence(presentation, light.origin, all_surfaces, catalog)?;
        objects.push(WorldObjectSnapshotV1 {
            stable_id: bounded_text(stable_id)?,
            asset_identity: bounded_text(asset.as_str())?,
            root: light.origin,
            rotation_sixths: rotation.steps(),
            blockers: BoundedVec::<_, MAX_OBJECT_BLOCKER_SURFACES>::new(blockers)?,
            protects_edits: true,
        });
    }
    objects.sort_by(|left, right| left.stable_id.cmp(&right.stable_id));
    Ok((lights, liquids, objects))
}

fn crystal_object_consequence(
    presentation: PlannedLightPresentation,
    root: TilePos,
    all_surfaces: &[TilePos],
    catalog: &RuntimeArtCatalog,
) -> Result<(ObjectAssetId, HexObjectRotation, Vec<TilePos>), WorldSnapshotError> {
    match presentation {
        PlannedLightPresentation::CaveCrystal(crystal) => {
            let objects = CaveCrystalObjectSet::resolve(catalog)
                .map_err(|error| WorldSnapshotError::UnknownObject(error.to_string()))?;
            let rotation = HexObjectRotation::new(crystal.rotation)
                .map_err(|error| WorldSnapshotError::PresentationMismatch(error.to_string()))?;
            Ok((
                objects.object_id(crystal.kind).clone(),
                rotation,
                Vec::new(),
            ))
        }
        PlannedLightPresentation::CrystalAscent(crystal) => {
            let objects = CrystalAscentObjectSet::resolve(catalog)
                .map_err(|error| WorldSnapshotError::UnknownObject(error.to_string()))?;
            let rotation = HexObjectRotation::new(crystal.rotation)
                .map_err(|error| WorldSnapshotError::PresentationMismatch(error.to_string()))?;
            match crystal.kind {
                CrystalAscentCrystalKind::Landing(kind) => {
                    Ok((objects.landing_id(kind).clone(), rotation, Vec::new()))
                }
                CrystalAscentCrystalKind::Heart => {
                    let visual_level = root.level.checked_add(1).ok_or_else(|| {
                        WorldSnapshotError::PresentationMismatch(
                            "crystal heart visual origin overflowed".to_owned(),
                        )
                    })?;
                    let blockers = objects
                        .project_heart_traversal_blockers(
                            all_surfaces.iter().copied(),
                            TilePos::new(root.coord, visual_level),
                            rotation,
                        )
                        .ok_or_else(|| {
                            WorldSnapshotError::PresentationMismatch(
                                "crystal heart blocker projection overflowed".to_owned(),
                            )
                        })?
                        .into_iter()
                        .collect();
                    Ok((objects.heart_id().clone(), rotation, blockers))
                }
            }
        }
    }
}

/// Validates and resolves a complete snapshot into private map resources.
///
/// Returning the candidate performs no ECS changes, which is the transactional
/// boundary used by the runtime import system.
pub(crate) fn prepare_world_snapshot_v1(
    snapshot: WorldSnapshotV1,
    table: &SubstanceTable,
    settings: &MapSettings,
    catalog: Option<&RuntimeArtCatalog>,
) -> Result<PreparedWorldSnapshotV1, WorldSnapshotError> {
    snapshot.validate()?;
    verify_fingerprint(&snapshot)?;

    let mut map = VoxelMap::new();
    let mut all_surfaces = Vec::new();
    for column in snapshot.columns.as_slice() {
        let mut resolved = Column::new();
        for run in column.runs.as_slice() {
            let substance = resolve_material(table, run.substance.as_str())?;
            for level in run.run_bottom..=run.position.level {
                resolved.set(level, substance);
            }
        }
        verify_published_column(column, &resolved, settings.level_height)?;
        all_surfaces.extend(column.runs.as_slice().iter().map(|run| run.position));
        map.insert_column(column.coord, resolved);
    }

    let mut damage = Vec::with_capacity(snapshot.damage.len());
    for entry in snapshot.damage.as_slice() {
        let substance = map.get(entry.position);
        let health = TerrainVoxelHealth::new(entry.remaining, entry.maximum)
            .filter(|health| health.is_damaged())
            .ok_or(WorldSnapshotError::DamageMismatch(entry.position))?;
        if substance.is_air() || table.toughness(substance) != Some(entry.maximum) {
            return Err(WorldSnapshotError::DamageMismatch(entry.position));
        }
        damage.push((entry.position, health));
    }

    let anchors = snapshot
        .anchors
        .as_slice()
        .iter()
        .map(|entry| (MapAnchorId::from(entry.name.as_str()), entry.position))
        .collect();
    let mut interiors = InteriorRegions::new();
    for entry in snapshot.interior_surfaces.as_slice() {
        interiors.insert_surface(entry.position, InteriorRegionId(entry.region));
    }
    for entry in snapshot.interior_roofs.as_slice() {
        interiors.insert_roof_voxel(entry.position, InteriorRegionId(entry.region));
    }
    let special_regions = snapshot
        .special_regions
        .as_slice()
        .iter()
        .map(|entry| (entry.position, SpecialMovementRegion(entry.region)))
        .collect();
    let mut biome_regions = BiomeRegions::new();
    for entry in snapshot.biome_regions.as_slice() {
        biome_regions.insert(entry.position, BiomeRegionId(entry.region));
    }
    let mut blockers = TraversalBlockers::new();
    for position in snapshot.blockers.as_slice() {
        blockers.insert(*position);
    }
    let view_hint = snapshot.view_hint.map(decode_view_hint);
    let presentation =
        prepare_presentation(&snapshot, &map, table, catalog, &all_surfaces, &blockers)?;

    Ok(PreparedWorldSnapshotV1 {
        snapshot,
        map,
        damage,
        anchors,
        interiors,
        special_regions,
        biome_regions,
        blockers,
        view_hint,
        presentation,
    })
}

/// Validates a snapshot against current shipped content without changing the world.
pub fn validate_world_snapshot_v1_against_content(
    snapshot: &WorldSnapshotV1,
    table: &SubstanceTable,
    settings: &MapSettings,
    catalog: Option<&RuntimeArtCatalog>,
) -> Result<(), WorldSnapshotError> {
    prepare_world_snapshot_v1(snapshot.clone(), table, settings, catalog).map(|_| ())
}

fn verify_published_column(
    wire: &WorldColumnSnapshotV1,
    column: &Column,
    level_height: f32,
) -> Result<(), WorldSnapshotError> {
    let projected = runs(column);
    if projected.len() != wire.runs.len() {
        let position = wire
            .runs
            .as_slice()
            .first()
            .map_or(TilePos::new(wire.coord, 0), |run| run.position);
        return Err(WorldSnapshotError::ProjectionMismatch(position));
    }
    for (run, expected) in projected.into_iter().zip(wire.runs.as_slice()) {
        let position = TilePos::new(wire.coord, run.top.saturating_sub(1));
        let span = span_bits(run.bottom, run.top, level_height);
        if position != expected.position
            || run.bottom != expected.run_bottom
            || span != (expected.span_bottom_bits, expected.span_top_bits)
            || column.headroom_above(run.top).0 != expected.headroom
        {
            return Err(WorldSnapshotError::ProjectionMismatch(expected.position));
        }
    }
    Ok(())
}

fn prepare_presentation(
    snapshot: &WorldSnapshotV1,
    map: &VoxelMap,
    table: &SubstanceTable,
    catalog: Option<&RuntimeArtCatalog>,
    all_surfaces: &[TilePos],
    blockers: &TraversalBlockers,
) -> Result<Option<MapPresentationProjection>, WorldSnapshotError> {
    let liquids = prepare_liquid_presentation(snapshot, map, table)?;

    let mut lights = BTreeMap::new();
    for entry in snapshot.lights.as_slice() {
        let id = parse_local_id(entry.stable_id.as_str(), LIGHT_ID_PREFIX)?;
        lights.insert(
            LightId(id),
            PlannedGameplayLight {
                origin: entry.origin,
                level: decode_illumination(entry.illumination),
                radius: entry.radius,
                presentation: None,
            },
        );
    }

    let mut features = BTreeMap::new();
    let mut presented_lights = BTreeSet::new();
    for object in snapshot.objects.as_slice() {
        let catalog = catalog.ok_or(WorldSnapshotError::WorldUnavailable("RuntimeArtCatalog"))?;
        let object_id = ObjectAssetId::new(object.asset_identity.as_str()).map_err(|_error| {
            WorldSnapshotError::UnknownObject(object.asset_identity.as_str().to_owned())
        })?;
        if catalog.object(&object_id).is_none() {
            return Err(WorldSnapshotError::UnknownObject(
                object.asset_identity.as_str().to_owned(),
            ));
        }
        let rotation = HexObjectRotation::new(object.rotation_sixths)
            .map_err(|error| WorldSnapshotError::PresentationMismatch(error.to_string()))?;
        if let Ok(id) = parse_local_id(object.stable_id.as_str(), FEATURE_ID_PREFIX) {
            if object
                .blockers
                .as_slice()
                .iter()
                .any(|position| !blockers.contains(*position))
            {
                return Err(WorldSnapshotError::PresentationMismatch(format!(
                    "feature '{}' names an unpublished traversal blocker",
                    object.stable_id.as_str()
                )));
            }
            if object.protects_edits
                && (object.blockers.is_empty()
                    || !object.blockers.as_slice().contains(&object.root))
            {
                return Err(WorldSnapshotError::PresentationMismatch(format!(
                    "protected feature '{}' must block a footprint containing its root",
                    object.stable_id.as_str()
                )));
            }
            if !object.protects_edits && !object.blockers.is_empty() {
                return Err(WorldSnapshotError::PresentationMismatch(format!(
                    "presentation-only feature '{}' cannot carry blockers",
                    object.stable_id.as_str()
                )));
            }
            let kind = feature_kind_from_consequence(object);
            features.insert(
                FeatureId(id),
                PlannedFeature {
                    root: object.root,
                    kind,
                    object_id,
                    rotation,
                    blocker_footprint: object.blockers.as_slice().iter().copied().collect(),
                },
            );
            continue;
        }

        let id = parse_local_id(object.stable_id.as_str(), LIGHT_ID_PREFIX)?;
        let light = lights.get_mut(&LightId(id)).ok_or_else(|| {
            WorldSnapshotError::PresentationMismatch(format!(
                "object '{}' has no matching gameplay light",
                object.stable_id.as_str()
            ))
        })?;
        if light.origin != object.root || !object.protects_edits || !presented_lights.insert(id) {
            return Err(WorldSnapshotError::PresentationMismatch(format!(
                "light object '{}' has inconsistent root/protection/identity",
                object.stable_id.as_str()
            )));
        }
        light.presentation = Some(resolve_crystal_presentation(
            &object_id,
            rotation,
            object.blockers.as_slice(),
            object.root,
            all_surfaces,
            catalog,
        )?);
    }

    if liquids.is_empty() && features.is_empty() && lights.is_empty() {
        Ok(None)
    } else {
        Ok(Some(MapPresentationProjection::from_snapshot_parts(
            liquids, features, lights,
        )))
    }
}

fn prepare_liquid_presentation(
    snapshot: &WorldSnapshotV1,
    map: &VoxelMap,
    table: &SubstanceTable,
) -> Result<BTreeMap<TilePos, MaterializedLiquidVoxel>, WorldSnapshotError> {
    let mut materialized = BTreeMap::new();
    let entries = snapshot
        .liquids
        .as_slice()
        .iter()
        .map(|entry| (entry.position, entry))
        .collect::<BTreeMap<_, _>>();
    let mut liquid_run_tops = BTreeSet::new();
    for entry in snapshot.liquids.as_slice() {
        let substance = resolve_material(table, entry.substance.as_str())?;
        if map.get(entry.position) != substance {
            return Err(WorldSnapshotError::PresentationMismatch(format!(
                "liquid voxel {:?} has a different material",
                entry.position
            )));
        }
        materialized.insert(
            entry.position,
            MaterializedLiquidVoxel {
                material: liquid_role_for_name(entry.substance.as_str())?,
                flow: decode_flow(entry.flow),
                downstream: entry.downstream,
            },
        );
        let run_top = snapshot
            .columns
            .as_slice()
            .binary_search_by_key(&entry.position.coord, |column| column.coord)
            .ok()
            .and_then(|index| snapshot.columns.get(index))
            .and_then(|column| {
                column.runs.as_slice().iter().find(|run| {
                    run.run_bottom <= entry.position.level
                        && entry.position.level <= run.position.level
                })
            })
            .map(|run| run.position)
            .ok_or_else(|| {
                WorldSnapshotError::PresentationMismatch(format!(
                    "liquid voxel {:?} has no material run",
                    entry.position
                ))
            })?;
        liquid_run_tops.insert(run_top);
    }

    let mut nodes = BTreeMap::<TilePos, (WorldLiquidSnapshotV1, i32)>::new();
    for column in snapshot.columns.as_slice() {
        for run in column.runs.as_slice() {
            if !liquid_run_tops.contains(&run.position) {
                continue;
            }
            let mut descriptor: Option<WorldLiquidSnapshotV1> = None;
            for level in run.run_bottom..=run.position.level {
                let position = TilePos::new(column.coord, level);
                let entry = entries.get(&position).ok_or_else(|| {
                    WorldSnapshotError::PresentationMismatch(format!(
                        "liquid run {:?} is missing voxel {position:?}",
                        run.position
                    ))
                })?;
                if entry.substance != run.substance {
                    return Err(WorldSnapshotError::PresentationMismatch(format!(
                        "liquid voxel {position:?} disagrees with its material run"
                    )));
                }
                if descriptor.as_ref().is_some_and(|current| {
                    current.substance != entry.substance
                        || current.flow != entry.flow
                        || current.downstream != entry.downstream
                }) {
                    return Err(WorldSnapshotError::PresentationMismatch(format!(
                        "liquid run {:?} contains inconsistent flow descriptors",
                        run.position
                    )));
                }
                descriptor.get_or_insert_with(|| (*entry).clone());
            }
            let descriptor = descriptor.ok_or_else(|| {
                WorldSnapshotError::PresentationMismatch(format!(
                    "liquid run {:?} has no descriptor",
                    run.position
                ))
            })?;
            nodes.insert(run.position, (descriptor, run.run_bottom));
        }
    }

    for (position, (node, run_bottom)) in &nodes {
        let Some(downstream) = node.downstream else {
            if node.flow != WorldLiquidFlowV1::Still {
                return Err(WorldSnapshotError::PresentationMismatch(format!(
                    "moving liquid run {position:?} has no downstream node"
                )));
            }
            continue;
        };
        let Some((target, _target_bottom)) = nodes.get(&downstream) else {
            return Err(WorldSnapshotError::PresentationMismatch(format!(
                "liquid run {position:?} names missing downstream node {downstream:?}"
            )));
        };
        if target.substance != node.substance || downstream.level > position.level {
            return Err(WorldSnapshotError::PresentationMismatch(format!(
                "liquid run {position:?} has incompatible downstream node {downstream:?}"
            )));
        }
        let drop = position.level.saturating_sub(downstream.level);
        match node.flow {
            WorldLiquidFlowV1::Still | WorldLiquidFlowV1::Current | WorldLiquidFlowV1::Rapid
                if drop > 1 =>
            {
                return Err(WorldSnapshotError::PresentationMismatch(format!(
                    "non-falling liquid run {position:?} drops too far to {downstream:?}"
                )));
            }
            WorldLiquidFlowV1::Fall
                if drop < 2 || *run_bottom > downstream.level.saturating_add(1) =>
            {
                return Err(WorldSnapshotError::PresentationMismatch(format!(
                    "falling liquid run {position:?} is discontinuous from {downstream:?}"
                )));
            }
            WorldLiquidFlowV1::Still
            | WorldLiquidFlowV1::Current
            | WorldLiquidFlowV1::Rapid
            | WorldLiquidFlowV1::Fall => {}
        }
    }

    let mut complete = BTreeSet::new();
    for start in nodes.keys().copied() {
        let mut path = BTreeSet::new();
        let mut current = start;
        loop {
            if complete.contains(&current) {
                break;
            }
            if !path.insert(current) {
                return Err(WorldSnapshotError::PresentationMismatch(format!(
                    "liquid flow contains a cycle through {current:?}"
                )));
            }
            let Some((node, _run_bottom)) = nodes.get(&current) else {
                break;
            };
            let Some(downstream) = node.downstream else {
                break;
            };
            current = downstream;
        }
        complete.extend(path);
    }

    Ok(materialized)
}

fn resolve_crystal_presentation(
    object_id: &ObjectAssetId,
    rotation: HexObjectRotation,
    blockers: &[TilePos],
    root: TilePos,
    all_surfaces: &[TilePos],
    catalog: &RuntimeArtCatalog,
) -> Result<PlannedLightPresentation, WorldSnapshotError> {
    let cave = CaveCrystalObjectSet::resolve(catalog)
        .map_err(|error| WorldSnapshotError::UnknownObject(error.to_string()))?;
    for kind in [
        CaveCrystalKind::LowCluster,
        CaveCrystalKind::Branched,
        CaveCrystalKind::Spire,
    ] {
        if cave.object_id(kind) == object_id {
            if !blockers.is_empty() {
                return Err(WorldSnapshotError::PresentationMismatch(format!(
                    "nonblocking crystal '{}' carries blockers",
                    object_id.as_str()
                )));
            }
            return Ok(PlannedLightPresentation::CaveCrystal(
                CaveCrystalPresentation {
                    kind,
                    site: CaveCrystalSiteKind::InteriorAlcove,
                    rotation: rotation.steps(),
                },
            ));
        }
    }

    let ascent = CrystalAscentObjectSet::resolve(catalog)
        .map_err(|error| WorldSnapshotError::UnknownObject(error.to_string()))?;
    if ascent.heart_id() != object_id {
        return Err(WorldSnapshotError::UnknownObject(
            object_id.as_str().to_owned(),
        ));
    }
    let visual_level = root.level.checked_add(1).ok_or_else(|| {
        WorldSnapshotError::PresentationMismatch("crystal heart origin overflowed".to_owned())
    })?;
    let expected = ascent
        .project_heart_traversal_blockers(
            all_surfaces.iter().copied(),
            TilePos::new(root.coord, visual_level),
            rotation,
        )
        .ok_or_else(|| {
            WorldSnapshotError::PresentationMismatch(
                "crystal heart blocker projection overflowed".to_owned(),
            )
        })?;
    if expected.iter().copied().collect::<Vec<_>>() != blockers {
        return Err(WorldSnapshotError::PresentationMismatch(
            "crystal heart blockers do not match shipped asset occupancy".to_owned(),
        ));
    }
    Ok(PlannedLightPresentation::CrystalAscent(
        CrystalAscentCrystalPresentation {
            kind: CrystalAscentCrystalKind::Heart,
            rotation: rotation.steps(),
        },
    ))
}

fn feature_kind_from_consequence(object: &WorldObjectSnapshotV1) -> FeatureKind {
    if object.protects_edits {
        FeatureKind::Tree
    } else if object.asset_identity.as_str().contains("cave") {
        FeatureKind::CaveVegetation
    } else {
        FeatureKind::TallGrass
    }
}

/// Computes the current public-world fingerprint over every canonical collection.
pub fn fingerprint_world_snapshot_v1(
    snapshot: &WorldSnapshotV1,
) -> Result<PublicWorldFingerprint, WorldSnapshotError> {
    snapshot.validate()?;
    Ok(fingerprint_canonical_world_snapshot_v1(snapshot))
}

fn fingerprint_canonical_world_snapshot_v1(snapshot: &WorldSnapshotV1) -> PublicWorldFingerprint {
    let mut encoder = CanonicalEncoder::default();
    encoder.bytes(FINGERPRINT_DOMAIN);
    encoder.u16(snapshot.version);
    encode_columns(&mut encoder, snapshot.columns.as_slice());
    encode_damage(&mut encoder, snapshot.damage.as_slice());
    encode_anchors(&mut encoder, snapshot.anchors.as_slice());
    encode_interior_surfaces(&mut encoder, snapshot.interior_surfaces.as_slice());
    encode_interior_roofs(&mut encoder, snapshot.interior_roofs.as_slice());
    encode_special_regions(&mut encoder, snapshot.special_regions.as_slice());
    encode_biome_regions(&mut encoder, snapshot.biome_regions.as_slice());
    encoder.len(snapshot.blockers.len());
    for position in snapshot.blockers.as_slice() {
        encoder.position(*position);
    }
    match snapshot.view_hint {
        Some(hint) => {
            encoder.u8(1);
            for bits in hint.eye_bits.into_iter().chain(hint.focus_bits) {
                encoder.u32(bits);
            }
        }
        None => encoder.u8(0),
    }
    encode_lights(&mut encoder, snapshot.lights.as_slice());
    encode_liquids(&mut encoder, snapshot.liquids.as_slice());
    encode_objects(&mut encoder, snapshot.objects.as_slice());
    PublicWorldFingerprint(xxh3_64(&encoder.bytes))
}

fn verify_fingerprint(snapshot: &WorldSnapshotV1) -> Result<(), WorldSnapshotError> {
    let actual = fingerprint_world_snapshot_v1(snapshot)?;
    if actual == snapshot.public_fingerprint {
        Ok(())
    } else {
        Err(WorldSnapshotError::FingerprintMismatch {
            expected: snapshot.public_fingerprint,
            actual,
        })
    }
}

/// Diffs two canonical snapshots into family/key-ordered authority operations.
pub fn diff_world_snapshots_v1(
    base: &WorldSnapshotV1,
    target: &WorldSnapshotV1,
    authority_sequence: AuthoritySequence,
) -> Result<WorldDeltaV1, WorldSnapshotError> {
    base.validate()?;
    target.validate()?;
    verify_fingerprint(base)?;
    verify_fingerprint(target)?;
    let mut operations = Vec::new();
    diff_collection(
        base.columns.as_slice(),
        target.columns.as_slice(),
        |value| value.coord,
        WorldDeltaOperationV1::UpsertColumn,
        WorldDeltaOperationV1::RemoveColumn,
        &mut operations,
    );
    diff_collection(
        base.damage.as_slice(),
        target.damage.as_slice(),
        |value| value.position,
        WorldDeltaOperationV1::UpsertDamage,
        WorldDeltaOperationV1::RemoveDamage,
        &mut operations,
    );
    diff_collection(
        base.anchors.as_slice(),
        target.anchors.as_slice(),
        |value| value.name.clone(),
        WorldDeltaOperationV1::UpsertAnchor,
        WorldDeltaOperationV1::RemoveAnchor,
        &mut operations,
    );
    diff_collection(
        base.interior_surfaces.as_slice(),
        target.interior_surfaces.as_slice(),
        |value| value.position,
        WorldDeltaOperationV1::UpsertInteriorSurface,
        WorldDeltaOperationV1::RemoveInteriorSurface,
        &mut operations,
    );
    diff_collection(
        base.interior_roofs.as_slice(),
        target.interior_roofs.as_slice(),
        |value| value.position,
        WorldDeltaOperationV1::UpsertInteriorRoof,
        WorldDeltaOperationV1::RemoveInteriorRoof,
        &mut operations,
    );
    diff_collection(
        base.special_regions.as_slice(),
        target.special_regions.as_slice(),
        |value| value.position,
        WorldDeltaOperationV1::UpsertSpecialRegion,
        WorldDeltaOperationV1::RemoveSpecialRegion,
        &mut operations,
    );
    diff_collection(
        base.biome_regions.as_slice(),
        target.biome_regions.as_slice(),
        |value| value.position,
        WorldDeltaOperationV1::UpsertBiomeRegion,
        WorldDeltaOperationV1::RemoveBiomeRegion,
        &mut operations,
    );
    diff_set(
        base.blockers.as_slice(),
        target.blockers.as_slice(),
        WorldDeltaOperationV1::UpsertBlocker,
        WorldDeltaOperationV1::RemoveBlocker,
        &mut operations,
    );
    if base.view_hint != target.view_hint {
        operations.push(target.view_hint.map_or(
            WorldDeltaOperationV1::ClearViewHint,
            WorldDeltaOperationV1::SetViewHint,
        ));
    }
    diff_collection(
        base.lights.as_slice(),
        target.lights.as_slice(),
        |value| value.stable_id.clone(),
        WorldDeltaOperationV1::UpsertLight,
        WorldDeltaOperationV1::RemoveLight,
        &mut operations,
    );
    diff_collection(
        base.liquids.as_slice(),
        target.liquids.as_slice(),
        |value| value.position,
        WorldDeltaOperationV1::UpsertLiquid,
        WorldDeltaOperationV1::RemoveLiquid,
        &mut operations,
    );
    diff_collection(
        base.objects.as_slice(),
        target.objects.as_slice(),
        |value| value.stable_id.clone(),
        WorldDeltaOperationV1::UpsertObject,
        WorldDeltaOperationV1::RemoveObject,
        &mut operations,
    );
    let delta = WorldDeltaV1 {
        version: WORLD_DELTA_VERSION_V1,
        authority_sequence,
        base_fingerprint: base.public_fingerprint,
        target_fingerprint: target.public_fingerprint,
        operations: BoundedVec::<_, MAX_WORLD_DELTA_OPERATIONS>::new(operations)?,
    };
    delta.validate()?;
    Ok(delta)
}

fn diff_collection<T, K>(
    base: &[T],
    target: &[T],
    key: impl Fn(&T) -> K,
    upsert: impl Fn(T) -> WorldDeltaOperationV1,
    remove: impl Fn(K) -> WorldDeltaOperationV1,
    operations: &mut Vec<WorldDeltaOperationV1>,
) where
    T: Clone + PartialEq,
    K: Clone + Ord,
{
    let start = operations.len();
    let base = base
        .iter()
        .map(|value| (key(value), value))
        .collect::<BTreeMap<_, _>>();
    let target = target
        .iter()
        .map(|value| (key(value), value))
        .collect::<BTreeMap<_, _>>();
    for (entry_key, value) in &target {
        if base.get(entry_key).is_none_or(|current| *current != *value) {
            operations.push(upsert((*value).clone()));
        }
    }
    for entry_key in base.keys() {
        if !target.contains_key(entry_key) {
            operations.push(remove(entry_key.clone()));
        }
    }
    if let Some(added) = operations.get_mut(start..) {
        added.sort_by(compare_operations);
    }
}

fn diff_set<T: Copy + Ord>(
    base: &[T],
    target: &[T],
    upsert: impl Fn(T) -> WorldDeltaOperationV1,
    remove: impl Fn(T) -> WorldDeltaOperationV1,
    operations: &mut Vec<WorldDeltaOperationV1>,
) {
    let start = operations.len();
    let base = base.iter().copied().collect::<BTreeSet<_>>();
    let target = target.iter().copied().collect::<BTreeSet<_>>();
    operations.extend(target.difference(&base).copied().map(upsert));
    operations.extend(base.difference(&target).copied().map(remove));
    if let Some(added) = operations.get_mut(start..) {
        added.sort_by(compare_operations);
    }
}

/// Applies a structurally valid delta to a cloned candidate and verifies its target.
pub fn apply_world_delta_v1(
    base: &WorldSnapshotV1,
    delta: &WorldDeltaV1,
) -> Result<WorldSnapshotV1, WorldSnapshotError> {
    base.validate()?;
    verify_fingerprint(base)?;
    delta.validate()?;
    if base.public_fingerprint != delta.base_fingerprint {
        return Err(WorldSnapshotError::DeltaBaseMismatch {
            expected: delta.base_fingerprint,
            actual: base.public_fingerprint,
        });
    }

    let mut candidate = SnapshotCollections::from(base);
    for operation in delta.operations.as_slice() {
        candidate.apply(operation);
    }
    let snapshot = candidate.finish(delta.target_fingerprint)?;
    let actual = fingerprint_world_snapshot_v1(&snapshot)?;
    if actual != delta.target_fingerprint {
        return Err(WorldSnapshotError::FingerprintMismatch {
            expected: delta.target_fingerprint,
            actual,
        });
    }
    Ok(snapshot)
}

struct SnapshotCollections {
    columns: Vec<WorldColumnSnapshotV1>,
    damage: Vec<WorldDamageSnapshotV1>,
    anchors: Vec<WorldAnchorSnapshotV1>,
    interior_surfaces: Vec<InteriorSurfaceSnapshotV1>,
    interior_roofs: Vec<InteriorRoofSnapshotV1>,
    special_regions: Vec<SpecialRegionSnapshotV1>,
    biome_regions: Vec<BiomeRegionSnapshotV1>,
    blockers: Vec<TilePos>,
    view_hint: Option<WorldViewHintSnapshotV1>,
    lights: Vec<WorldLightSnapshotV1>,
    liquids: Vec<WorldLiquidSnapshotV1>,
    objects: Vec<WorldObjectSnapshotV1>,
}

impl From<&WorldSnapshotV1> for SnapshotCollections {
    fn from(value: &WorldSnapshotV1) -> Self {
        Self {
            columns: value.columns.as_slice().to_vec(),
            damage: value.damage.as_slice().to_vec(),
            anchors: value.anchors.as_slice().to_vec(),
            interior_surfaces: value.interior_surfaces.as_slice().to_vec(),
            interior_roofs: value.interior_roofs.as_slice().to_vec(),
            special_regions: value.special_regions.as_slice().to_vec(),
            biome_regions: value.biome_regions.as_slice().to_vec(),
            blockers: value.blockers.as_slice().to_vec(),
            view_hint: value.view_hint,
            lights: value.lights.as_slice().to_vec(),
            liquids: value.liquids.as_slice().to_vec(),
            objects: value.objects.as_slice().to_vec(),
        }
    }
}

impl SnapshotCollections {
    fn apply(&mut self, operation: &WorldDeltaOperationV1) {
        match operation {
            WorldDeltaOperationV1::UpsertColumn(value) => {
                upsert(&mut self.columns, value.clone(), |entry| entry.coord)
            }
            WorldDeltaOperationV1::RemoveColumn(key) => {
                remove(&mut self.columns, key, |entry| entry.coord)
            }
            WorldDeltaOperationV1::UpsertDamage(value) => {
                upsert(&mut self.damage, *value, |entry| entry.position)
            }
            WorldDeltaOperationV1::RemoveDamage(key) => {
                remove(&mut self.damage, key, |entry| entry.position)
            }
            WorldDeltaOperationV1::UpsertAnchor(value) => {
                upsert(&mut self.anchors, value.clone(), |entry| entry.name.clone())
            }
            WorldDeltaOperationV1::RemoveAnchor(key) => {
                remove(&mut self.anchors, key, |entry| entry.name.clone())
            }
            WorldDeltaOperationV1::UpsertInteriorSurface(value) => {
                upsert(&mut self.interior_surfaces, *value, |entry| entry.position)
            }
            WorldDeltaOperationV1::RemoveInteriorSurface(key) => {
                remove(&mut self.interior_surfaces, key, |entry| entry.position)
            }
            WorldDeltaOperationV1::UpsertInteriorRoof(value) => {
                upsert(&mut self.interior_roofs, *value, |entry| entry.position)
            }
            WorldDeltaOperationV1::RemoveInteriorRoof(key) => {
                remove(&mut self.interior_roofs, key, |entry| entry.position)
            }
            WorldDeltaOperationV1::UpsertSpecialRegion(value) => {
                upsert(&mut self.special_regions, *value, |entry| entry.position)
            }
            WorldDeltaOperationV1::RemoveSpecialRegion(key) => {
                remove(&mut self.special_regions, key, |entry| entry.position)
            }
            WorldDeltaOperationV1::UpsertBiomeRegion(value) => {
                upsert(&mut self.biome_regions, *value, |entry| entry.position)
            }
            WorldDeltaOperationV1::RemoveBiomeRegion(key) => {
                remove(&mut self.biome_regions, key, |entry| entry.position)
            }
            WorldDeltaOperationV1::UpsertBlocker(value) => {
                upsert(&mut self.blockers, *value, |entry| *entry)
            }
            WorldDeltaOperationV1::RemoveBlocker(key) => {
                remove(&mut self.blockers, key, |entry| *entry)
            }
            WorldDeltaOperationV1::SetViewHint(value) => self.view_hint = Some(*value),
            WorldDeltaOperationV1::ClearViewHint => self.view_hint = None,
            WorldDeltaOperationV1::UpsertLight(value) => {
                upsert(&mut self.lights, value.clone(), |entry| {
                    entry.stable_id.clone()
                })
            }
            WorldDeltaOperationV1::RemoveLight(key) => {
                remove(&mut self.lights, key, |entry| entry.stable_id.clone())
            }
            WorldDeltaOperationV1::UpsertLiquid(value) => {
                upsert(&mut self.liquids, value.clone(), |entry| entry.position)
            }
            WorldDeltaOperationV1::RemoveLiquid(key) => {
                remove(&mut self.liquids, key, |entry| entry.position)
            }
            WorldDeltaOperationV1::UpsertObject(value) => {
                upsert(&mut self.objects, value.clone(), |entry| {
                    entry.stable_id.clone()
                })
            }
            WorldDeltaOperationV1::RemoveObject(key) => {
                remove(&mut self.objects, key, |entry| entry.stable_id.clone())
            }
        }
    }

    fn finish(
        self,
        public_fingerprint: PublicWorldFingerprint,
    ) -> Result<WorldSnapshotV1, WorldSnapshotError> {
        let snapshot = WorldSnapshotV1 {
            version: WORLD_SNAPSHOT_VERSION_V1,
            public_fingerprint,
            columns: BoundedVec::new(self.columns)?,
            damage: BoundedVec::new(self.damage)?,
            anchors: BoundedVec::new(self.anchors)?,
            interior_surfaces: BoundedVec::new(self.interior_surfaces)?,
            interior_roofs: BoundedVec::new(self.interior_roofs)?,
            special_regions: BoundedVec::new(self.special_regions)?,
            biome_regions: BoundedVec::new(self.biome_regions)?,
            blockers: BoundedVec::new(self.blockers)?,
            view_hint: self.view_hint,
            lights: BoundedVec::new(self.lights)?,
            liquids: BoundedVec::new(self.liquids)?,
            objects: BoundedVec::new(self.objects)?,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }
}

fn upsert<T, K: Ord>(values: &mut Vec<T>, value: T, key: impl Fn(&T) -> K) {
    let target = key(&value);
    match values.binary_search_by(|entry| key(entry).cmp(&target)) {
        Ok(index) => {
            if let Some(entry) = values.get_mut(index) {
                *entry = value;
            }
        }
        Err(index) => values.insert(index, value),
    }
}

fn remove<T, K: Ord>(values: &mut Vec<T>, target: &K, key: impl Fn(&T) -> K) {
    if let Ok(index) = values.binary_search_by(|entry| key(entry).cmp(target)) {
        values.remove(index);
    }
}

fn compare_operations(
    left: &WorldDeltaOperationV1,
    right: &WorldDeltaOperationV1,
) -> std::cmp::Ordering {
    operation_family(left)
        .cmp(&operation_family(right))
        .then_with(|| operation_key(left).cmp(&operation_key(right)))
}

fn operation_family(operation: &WorldDeltaOperationV1) -> u8 {
    match operation {
        WorldDeltaOperationV1::UpsertColumn(_) | WorldDeltaOperationV1::RemoveColumn(_) => 0,
        WorldDeltaOperationV1::UpsertDamage(_) | WorldDeltaOperationV1::RemoveDamage(_) => 1,
        WorldDeltaOperationV1::UpsertAnchor(_) | WorldDeltaOperationV1::RemoveAnchor(_) => 2,
        WorldDeltaOperationV1::UpsertInteriorSurface(_)
        | WorldDeltaOperationV1::RemoveInteriorSurface(_) => 3,
        WorldDeltaOperationV1::UpsertInteriorRoof(_)
        | WorldDeltaOperationV1::RemoveInteriorRoof(_) => 4,
        WorldDeltaOperationV1::UpsertSpecialRegion(_)
        | WorldDeltaOperationV1::RemoveSpecialRegion(_) => 5,
        WorldDeltaOperationV1::UpsertBiomeRegion(_)
        | WorldDeltaOperationV1::RemoveBiomeRegion(_) => 6,
        WorldDeltaOperationV1::UpsertBlocker(_) | WorldDeltaOperationV1::RemoveBlocker(_) => 7,
        WorldDeltaOperationV1::SetViewHint(_) | WorldDeltaOperationV1::ClearViewHint => 8,
        WorldDeltaOperationV1::UpsertLight(_) | WorldDeltaOperationV1::RemoveLight(_) => 9,
        WorldDeltaOperationV1::UpsertLiquid(_) | WorldDeltaOperationV1::RemoveLiquid(_) => 10,
        WorldDeltaOperationV1::UpsertObject(_) | WorldDeltaOperationV1::RemoveObject(_) => 11,
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum OperationKey<'a> {
    Coord(hex_core::HexCoord),
    Position(TilePos),
    Text(&'a str),
    Singleton,
}

fn operation_key(operation: &WorldDeltaOperationV1) -> OperationKey<'_> {
    match operation {
        WorldDeltaOperationV1::UpsertColumn(value) => OperationKey::Coord(value.coord),
        WorldDeltaOperationV1::RemoveColumn(value) => OperationKey::Coord(*value),
        WorldDeltaOperationV1::UpsertDamage(value) => OperationKey::Position(value.position),
        WorldDeltaOperationV1::RemoveDamage(value) => OperationKey::Position(*value),
        WorldDeltaOperationV1::UpsertAnchor(value) => OperationKey::Text(value.name.as_str()),
        WorldDeltaOperationV1::RemoveAnchor(value) => OperationKey::Text(value.as_str()),
        WorldDeltaOperationV1::UpsertInteriorSurface(value) => {
            OperationKey::Position(value.position)
        }
        WorldDeltaOperationV1::RemoveInteriorSurface(value) => OperationKey::Position(*value),
        WorldDeltaOperationV1::UpsertInteriorRoof(value) => OperationKey::Position(value.position),
        WorldDeltaOperationV1::RemoveInteriorRoof(value) => OperationKey::Position(*value),
        WorldDeltaOperationV1::UpsertSpecialRegion(value) => OperationKey::Position(value.position),
        WorldDeltaOperationV1::RemoveSpecialRegion(value) => OperationKey::Position(*value),
        WorldDeltaOperationV1::UpsertBiomeRegion(value) => OperationKey::Position(value.position),
        WorldDeltaOperationV1::RemoveBiomeRegion(value) => OperationKey::Position(*value),
        WorldDeltaOperationV1::UpsertBlocker(value)
        | WorldDeltaOperationV1::RemoveBlocker(value) => OperationKey::Position(*value),
        WorldDeltaOperationV1::SetViewHint(_) | WorldDeltaOperationV1::ClearViewHint => {
            OperationKey::Singleton
        }
        WorldDeltaOperationV1::UpsertLight(value) => OperationKey::Text(value.stable_id.as_str()),
        WorldDeltaOperationV1::RemoveLight(value) => OperationKey::Text(value.as_str()),
        WorldDeltaOperationV1::UpsertLiquid(value) => OperationKey::Position(value.position),
        WorldDeltaOperationV1::RemoveLiquid(value) => OperationKey::Position(*value),
        WorldDeltaOperationV1::UpsertObject(value) => OperationKey::Text(value.stable_id.as_str()),
        WorldDeltaOperationV1::RemoveObject(value) => OperationKey::Text(value.as_str()),
    }
}

#[derive(Default)]
struct CanonicalEncoder {
    bytes: Vec<u8>,
}

impl CanonicalEncoder {
    fn bytes(&mut self, value: &[u8]) {
        self.len(value.len());
        self.bytes.extend_from_slice(value);
    }

    fn text(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }

    fn len(&mut self, value: usize) {
        self.u64(u64::try_from(value).unwrap_or(u64::MAX));
    }

    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn i32(&mut self, value: i32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn position(&mut self, value: TilePos) {
        self.i32(value.coord.x());
        self.i32(value.coord.y());
        self.i32(value.coord.z());
        self.i32(value.level);
    }
}

fn encode_columns(encoder: &mut CanonicalEncoder, values: &[WorldColumnSnapshotV1]) {
    encoder.len(values.len());
    for column in values {
        encoder.i32(column.coord.x());
        encoder.i32(column.coord.y());
        encoder.i32(column.coord.z());
        encoder.len(column.runs.len());
        for run in column.runs.as_slice() {
            encoder.position(run.position);
            encoder.i32(run.run_bottom);
            encoder.u32(run.span_bottom_bits);
            encoder.u32(run.span_top_bits);
            encoder.text(run.substance.as_str());
            encoder.i32(run.headroom);
        }
    }
}

fn encode_damage(encoder: &mut CanonicalEncoder, values: &[WorldDamageSnapshotV1]) {
    encoder.len(values.len());
    for value in values {
        encoder.position(value.position);
        encoder.u8(value.remaining);
        encoder.u8(value.maximum);
    }
}

fn encode_anchors(encoder: &mut CanonicalEncoder, values: &[WorldAnchorSnapshotV1]) {
    encoder.len(values.len());
    for value in values {
        encoder.text(value.name.as_str());
        encoder.position(value.position);
    }
}

fn encode_interior_surfaces(encoder: &mut CanonicalEncoder, values: &[InteriorSurfaceSnapshotV1]) {
    encoder.len(values.len());
    for value in values {
        encoder.position(value.position);
        encoder.u32(value.region);
    }
}

fn encode_interior_roofs(encoder: &mut CanonicalEncoder, values: &[InteriorRoofSnapshotV1]) {
    encoder.len(values.len());
    for value in values {
        encoder.position(value.position);
        encoder.u32(value.region);
    }
}

fn encode_special_regions(encoder: &mut CanonicalEncoder, values: &[SpecialRegionSnapshotV1]) {
    encoder.len(values.len());
    for value in values {
        encoder.position(value.position);
        encoder.u32(value.region);
    }
}

fn encode_biome_regions(encoder: &mut CanonicalEncoder, values: &[BiomeRegionSnapshotV1]) {
    encoder.len(values.len());
    for value in values {
        encoder.position(value.position);
        encoder.u32(value.region);
    }
}

fn encode_lights(encoder: &mut CanonicalEncoder, values: &[WorldLightSnapshotV1]) {
    encoder.len(values.len());
    for value in values {
        encoder.text(value.stable_id.as_str());
        encoder.position(value.origin);
        encoder.u8(match value.illumination {
            WorldIlluminationV1::Dark => 0,
            WorldIlluminationV1::Dim => 1,
            WorldIlluminationV1::Bright => 2,
        });
        encoder.u32(value.radius);
    }
}

fn encode_liquids(encoder: &mut CanonicalEncoder, values: &[WorldLiquidSnapshotV1]) {
    encoder.len(values.len());
    for value in values {
        encoder.position(value.position);
        encoder.text(value.substance.as_str());
        encoder.u8(match value.flow {
            WorldLiquidFlowV1::Still => 0,
            WorldLiquidFlowV1::Current => 1,
            WorldLiquidFlowV1::Rapid => 2,
            WorldLiquidFlowV1::Fall => 3,
        });
        if let Some(downstream) = value.downstream {
            encoder.u8(1);
            encoder.position(downstream);
        } else {
            encoder.u8(0);
        }
    }
}

fn encode_objects(encoder: &mut CanonicalEncoder, values: &[WorldObjectSnapshotV1]) {
    encoder.len(values.len());
    for value in values {
        encoder.text(value.stable_id.as_str());
        encoder.text(value.asset_identity.as_str());
        encoder.position(value.root);
        encoder.u8(value.rotation_sixths);
        encoder.len(value.blockers.len());
        for position in value.blockers.as_slice() {
            encoder.position(*position);
        }
        encoder.u8(u8::from(value.protects_edits));
    }
}

fn resolve_material(
    table: &SubstanceTable,
    name: &str,
) -> Result<hex_core::SubstanceId, WorldSnapshotError> {
    let substance = table
        .id(name)
        .ok_or_else(|| WorldSnapshotError::UnknownSubstance(name.to_owned()))?;
    if substance.is_air() || name == "air" {
        Err(WorldSnapshotError::AirAsMaterial(name.to_owned()))
    } else {
        Ok(substance)
    }
}

fn liquid_role_for_name(name: &str) -> Result<FillMaterialRole, WorldSnapshotError> {
    match name {
        "water" => Ok(FillMaterialRole::Water),
        "lava" => Ok(FillMaterialRole::Lava),
        _ => Err(WorldSnapshotError::PresentationMismatch(format!(
            "substance '{name}' is not a shipped liquid role"
        ))),
    }
}

fn bounded_text(value: impl Into<String>) -> Result<BoundedText<MAX_IDENTITY_BYTES>, BoundError> {
    BoundedText::new(value)
}

fn bounded_collection<T, const MAX: usize>(
    collection: &'static str,
    values: Vec<T>,
) -> Result<BoundedVec<T, MAX>, WorldSnapshotError> {
    BoundedVec::new(values)
        .map_err(|source| WorldSnapshotError::CollectionBound { collection, source })
}

fn stable_local_id(prefix: &str, value: u32) -> String {
    format!("{prefix}{value:010}")
}

fn parse_local_id(value: &str, prefix: &str) -> Result<u32, WorldSnapshotError> {
    let Some(suffix) = value.strip_prefix(prefix) else {
        return Err(WorldSnapshotError::InvalidPresentationIdentity(
            value.to_owned(),
        ));
    };
    if suffix.len() != 10 || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(WorldSnapshotError::InvalidPresentationIdentity(
            value.to_owned(),
        ));
    }
    suffix
        .parse()
        .map_err(|_error| WorldSnapshotError::InvalidPresentationIdentity(value.to_owned()))
}

#[expect(
    clippy::cast_precision_loss,
    reason = "snapshot levels use the same exact small integer domain as terrain publication"
)]
fn span_bits(bottom: i32, top: i32, level_height: f32) -> (u32, u32) {
    (
        (bottom as f32 * level_height).to_bits(),
        (top as f32 * level_height).to_bits(),
    )
}

fn encode_illumination(value: IlluminationLevel) -> WorldIlluminationV1 {
    match value {
        IlluminationLevel::Dark => WorldIlluminationV1::Dark,
        IlluminationLevel::Dim => WorldIlluminationV1::Dim,
        IlluminationLevel::Bright => WorldIlluminationV1::Bright,
    }
}

fn decode_illumination(value: WorldIlluminationV1) -> IlluminationLevel {
    match value {
        WorldIlluminationV1::Dark => IlluminationLevel::Dark,
        WorldIlluminationV1::Dim => IlluminationLevel::Dim,
        WorldIlluminationV1::Bright => IlluminationLevel::Bright,
    }
}

fn encode_flow(value: LiquidFlowState) -> WorldLiquidFlowV1 {
    match value {
        LiquidFlowState::Still => WorldLiquidFlowV1::Still,
        LiquidFlowState::Current => WorldLiquidFlowV1::Current,
        LiquidFlowState::Rapid => WorldLiquidFlowV1::Rapid,
        LiquidFlowState::Fall => WorldLiquidFlowV1::Fall,
    }
}

fn decode_flow(value: WorldLiquidFlowV1) -> LiquidFlowState {
    match value {
        WorldLiquidFlowV1::Still => LiquidFlowState::Still,
        WorldLiquidFlowV1::Current => LiquidFlowState::Current,
        WorldLiquidFlowV1::Rapid => LiquidFlowState::Rapid,
        WorldLiquidFlowV1::Fall => LiquidFlowState::Fall,
    }
}

fn decode_view_hint(value: WorldViewHintSnapshotV1) -> MapViewHint {
    let [eye_x, eye_y, eye_z] = value.eye();
    let [focus_x, focus_y, focus_z] = value.focus();
    MapViewHint::new((eye_x, eye_y, eye_z), (focus_x, focus_y, focus_z))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::{HexCoord, MAX_HEADROOM};

    fn empty_snapshot() -> WorldSnapshotV1 {
        let position = TilePos::new(HexCoord::ORIGIN, 0);
        let mut snapshot = WorldSnapshotV1 {
            version: WORLD_SNAPSHOT_VERSION_V1,
            public_fingerprint: PublicWorldFingerprint(0),
            columns: BoundedVec::new(vec![WorldColumnSnapshotV1 {
                coord: HexCoord::ORIGIN,
                runs: BoundedVec::new(vec![WorldRunSnapshotV1 {
                    position,
                    run_bottom: 0,
                    span_bottom_bits: 0.0_f32.to_bits(),
                    span_top_bits: 1.0_f32.to_bits(),
                    substance: bounded_text("stone").expect("fixture name"),
                    headroom: MAX_HEADROOM,
                }])
                .expect("fixture runs"),
            }])
            .expect("fixture columns"),
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
        snapshot.public_fingerprint =
            fingerprint_world_snapshot_v1(&snapshot).expect("fixture fingerprints");
        snapshot
    }

    fn only_run(snapshot: &WorldSnapshotV1) -> &WorldRunSnapshotV1 {
        let Some(column) = snapshot.columns.first() else {
            panic!("fixture should contain one column");
        };
        let Some(run) = column.runs.first() else {
            panic!("fixture should contain one run");
        };
        run
    }

    #[test]
    fn fingerprint_covers_public_run_tuple() {
        let base = empty_snapshot();
        let mut changed = base.clone();
        changed.columns = BoundedVec::new(vec![WorldColumnSnapshotV1 {
            coord: HexCoord::ORIGIN,
            runs: BoundedVec::new(vec![WorldRunSnapshotV1 {
                headroom: MAX_HEADROOM.saturating_sub(1),
                ..only_run(&changed).clone()
            }])
            .expect("changed runs"),
        }])
        .expect("changed columns");
        changed.public_fingerprint =
            fingerprint_world_snapshot_v1(&changed).expect("changed fingerprints");

        assert_ne!(base.public_fingerprint, changed.public_fingerprint);
    }

    #[test]
    fn canonical_delta_round_trip_is_exact_and_idempotent() {
        let base = empty_snapshot();
        let mut target = base.clone();
        target.anchors = BoundedVec::new(vec![WorldAnchorSnapshotV1 {
            name: bounded_text("party_start").expect("fixture anchor"),
            position: only_run(&target).position,
        }])
        .expect("fixture anchors");
        target.public_fingerprint =
            fingerprint_world_snapshot_v1(&target).expect("target fingerprints");

        let delta =
            diff_world_snapshots_v1(&base, &target, AuthoritySequence(7)).expect("canonical diff");
        assert_eq!(apply_world_delta_v1(&base, &delta), Ok(target.clone()));
        assert!(matches!(
            apply_world_delta_v1(&target, &delta),
            Err(WorldSnapshotError::DeltaBaseMismatch { .. })
        ));
    }

    #[test]
    fn wrong_target_fingerprint_rejects_transactional_candidate() {
        let base = empty_snapshot();
        let mut target = base.clone();
        target.blockers =
            BoundedVec::new(vec![only_run(&target).position]).expect("fixture blockers");
        target.public_fingerprint =
            fingerprint_world_snapshot_v1(&target).expect("target fingerprint");
        let mut delta =
            diff_world_snapshots_v1(&base, &target, AuthoritySequence(9)).expect("canonical diff");
        delta.target_fingerprint = PublicWorldFingerprint(delta.target_fingerprint.0 ^ 1);

        assert!(matches!(
            apply_world_delta_v1(&base, &delta),
            Err(WorldSnapshotError::FingerprintMismatch { .. })
        ));
    }
}
