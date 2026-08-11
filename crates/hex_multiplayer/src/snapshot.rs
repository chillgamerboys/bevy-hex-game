//! Versioned, generator-neutral world and reconnect snapshot contracts.
//!
//! The shared crate defines bounded stable-name data only. `hex_map` remains the sole
//! exporter/importer and resolves names, health, public projections, presentation
//! consequences, and transactional deltas against its private runtime state.

use std::{cmp::Ordering, fmt};

use bevy_ecs::prelude::Message;
use hex_core::{is_terrain_toughness, HexCoord, TilePos, MAX_HEADROOM};
use serde::{Deserialize, Serialize};

use crate::{
    limits::{
        BoundedText, BoundedVec, MAX_ABS_COMMAND_COORDINATE, MAX_ABS_COMMAND_LEVEL,
        MAX_IDENTITY_BYTES, MAX_OBJECT_BLOCKER_SURFACES, MAX_SESSION_UNITS, MAX_WORLD_COLUMNS,
        MAX_WORLD_DELTA_OPERATIONS, MAX_WORLD_PROJECTION_ENTRIES, MAX_WORLD_RUNS_PER_COLUMN,
    },
    AuthoritySequence, LiveSnapshotHeaderV1, ManifestValidationError, PublicWorldFingerprint,
    ReplicaValidationError, SessionManifestV1, SessionReplica, UnitReplica,
};

/// Serialized world snapshot schema.
pub const WORLD_SNAPSHOT_VERSION_V1: u16 = 1;
/// Serialized world delta schema.
pub const WORLD_DELTA_VERSION_V1: u16 = 1;
/// Serialized player-knowledge schema.
pub const PLAYER_KNOWLEDGE_SNAPSHOT_VERSION_V1: u16 = 1;
/// Serialized complete live-session schema.
pub const LIVE_SESSION_SNAPSHOT_VERSION_V1: u16 = 1;

/// One canonical rendered/material run in a stable-name voxel column.
///
/// `position` is the public `TilePos` at the top of the run. Together with
/// `run_bottom`, `span_*_bits`, `substance`, and `headroom`, this preserves the
/// complete published tile tuple without serializing transient entities or handles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldRunSnapshotV1 {
    /// Exact top material voxel.
    pub position: TilePos,
    /// Inclusive lowest voxel in the run.
    pub run_bottom: i32,
    /// Exact `f32::to_bits` of the published `HexSpan::bottom`.
    pub span_bottom_bits: u32,
    /// Exact `f32::to_bits` of the published `HexSpan::top`.
    pub span_top_bits: u32,
    /// Stable substance catalog name, never a runtime `SubstanceId`.
    pub substance: BoundedText<MAX_IDENTITY_BYTES>,
    /// Published quantized clearance above the exposed top face.
    pub headroom: i32,
}

impl WorldRunSnapshotV1 {
    /// Exact span bottom.
    #[must_use]
    pub const fn span_bottom(&self) -> f32 {
        f32::from_bits(self.span_bottom_bits)
    }

    /// Exact span top.
    #[must_use]
    pub const fn span_top(&self) -> f32 {
        f32::from_bits(self.span_top_bits)
    }
}

/// One non-empty voxel column in exact coordinate order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldColumnSnapshotV1 {
    /// Horizontal column coordinate.
    pub coord: HexCoord,
    /// Non-air material runs ordered from lowest to highest.
    pub runs: BoundedVec<WorldRunSnapshotV1, MAX_WORLD_RUNS_PER_COLUMN>,
}

/// Exact partial health for one extant voxel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldDamageSnapshotV1 {
    /// Damaged voxel.
    pub position: TilePos,
    /// Current positive health below `maximum`.
    pub remaining: u8,
    /// Authored maximum toughness.
    pub maximum: u8,
}

/// One stable scenario/map anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldAnchorSnapshotV1 {
    /// Stable anchor name.
    pub name: BoundedText<MAX_IDENTITY_BYTES>,
    /// Exact exposed surface.
    pub position: TilePos,
}

/// One exact interior floor membership.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteriorSurfaceSnapshotV1 {
    /// Exact floor surface.
    pub position: TilePos,
    /// Deterministic map-local region number.
    pub region: u32,
}

/// One exact authored cutaway-roof voxel membership.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteriorRoofSnapshotV1 {
    /// Exact roof voxel, not a transient run entity.
    pub position: TilePos,
    /// Deterministic map-local region number.
    pub region: u32,
}

/// One exact optional-movement region membership.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecialRegionSnapshotV1 {
    /// Exact exposed surface.
    pub position: TilePos,
    /// Deterministic map-local region number.
    pub region: u32,
}

/// One exact biome membership.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BiomeRegionSnapshotV1 {
    /// Exact exposed surface.
    pub position: TilePos,
    /// Deterministic map-local region number.
    pub region: u32,
}

/// Exact bit-pattern camera framing consequence published by the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldViewHintSnapshotV1 {
    /// `f32::to_bits` for eye x/y/z.
    pub eye_bits: [u32; 3],
    /// `f32::to_bits` for focus x/y/z.
    pub focus_bits: [u32; 3],
}

impl WorldViewHintSnapshotV1 {
    /// Exact eye vector.
    #[must_use]
    pub fn eye(self) -> [f32; 3] {
        self.eye_bits.map(f32::from_bits)
    }

    /// Exact focus vector.
    #[must_use]
    pub fn focus(self) -> [f32; 3] {
        self.focus_bits.map(f32::from_bits)
    }
}

/// Stable gameplay illumination tier.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum WorldIlluminationV1 {
    /// No ambient or local gameplay light.
    #[default]
    Dark,
    /// Weak illumination.
    Dim,
    /// Strong illumination.
    Bright,
}

/// One gameplay light and its generator-neutral exact consequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldLightSnapshotV1 {
    /// Stable map-local identity encoded as text.
    pub stable_id: BoundedText<MAX_IDENTITY_BYTES>,
    /// Exact source surface.
    pub origin: TilePos,
    /// Gameplay illumination tier.
    pub illumination: WorldIlluminationV1,
    /// Inclusive upper-dome radius.
    pub radius: u32,
}

/// Generator-neutral liquid flow consequence.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum WorldLiquidFlowV1 {
    /// Standing liquid.
    #[default]
    Still,
    /// Ordinary current.
    Current,
    /// Fast current.
    Rapid,
    /// Vertical fall.
    Fall,
}

/// One authored liquid voxel with exact flow topology.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldLiquidSnapshotV1 {
    /// Exact liquid voxel.
    pub position: TilePos,
    /// Stable substance catalog name.
    pub substance: BoundedText<MAX_IDENTITY_BYTES>,
    /// Rendering/flow consequence.
    pub flow: WorldLiquidFlowV1,
    /// Exact downstream neighbour when one exists.
    pub downstream: Option<TilePos>,
}

/// One current feature/crystal/object presentation consequence.
///
/// Recipe identities and private plans are intentionally absent. Stable asset identity,
/// placement, rotation, blockers, and edit protection are sufficient to restore the
/// current renderer and mutation guard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldObjectSnapshotV1 {
    /// Stable map-local object identity.
    pub stable_id: BoundedText<MAX_IDENTITY_BYTES>,
    /// Stable authored object catalog identity.
    pub asset_identity: BoundedText<MAX_IDENTITY_BYTES>,
    /// Exact supporting/root surface.
    pub root: TilePos,
    /// Clockwise sixth-turn rotation in `0..6`.
    pub rotation_sixths: u8,
    /// Exact contextual traversal blockers.
    pub blockers: BoundedVec<TilePos, MAX_OBJECT_BLOCKER_SURFACES>,
    /// Whether the current consequence protects supporting voxels from edits.
    pub protects_edits: bool,
}

/// Complete generator-neutral world-owned export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldSnapshotV1 {
    /// Exact snapshot schema.
    pub version: u16,
    /// Fingerprint of canonical stable-name state and every public projection below.
    pub public_fingerprint: PublicWorldFingerprint,
    /// Non-empty columns in exact coordinate order.
    pub columns: BoundedVec<WorldColumnSnapshotV1, MAX_WORLD_COLUMNS>,
    /// Partial voxel health in exact position order.
    pub damage: BoundedVec<WorldDamageSnapshotV1, MAX_WORLD_PROJECTION_ENTRIES>,
    /// Stable anchors in name order.
    pub anchors: BoundedVec<WorldAnchorSnapshotV1, MAX_WORLD_PROJECTION_ENTRIES>,
    /// Interior floors in exact position order.
    pub interior_surfaces: BoundedVec<InteriorSurfaceSnapshotV1, MAX_WORLD_PROJECTION_ENTRIES>,
    /// Interior roof voxels in exact position order.
    pub interior_roofs: BoundedVec<InteriorRoofSnapshotV1, MAX_WORLD_PROJECTION_ENTRIES>,
    /// Optional-movement memberships in exact position order.
    pub special_regions: BoundedVec<SpecialRegionSnapshotV1, MAX_WORLD_PROJECTION_ENTRIES>,
    /// Biome memberships in exact position order.
    pub biome_regions: BoundedVec<BiomeRegionSnapshotV1, MAX_WORLD_PROJECTION_ENTRIES>,
    /// Contextual traversal blockers in exact position order.
    pub blockers: BoundedVec<TilePos, MAX_WORLD_PROJECTION_ENTRIES>,
    /// Current map-owned camera framing consequence.
    pub view_hint: Option<WorldViewHintSnapshotV1>,
    /// Gameplay lights in stable-id order.
    pub lights: BoundedVec<WorldLightSnapshotV1, MAX_WORLD_PROJECTION_ENTRIES>,
    /// Liquid voxels in exact position order.
    pub liquids: BoundedVec<WorldLiquidSnapshotV1, MAX_WORLD_PROJECTION_ENTRIES>,
    /// Current feature/crystal presentation consequences in stable-id order.
    pub objects: BoundedVec<WorldObjectSnapshotV1, MAX_WORLD_PROJECTION_ENTRIES>,
}

impl WorldSnapshotV1 {
    /// Validates bounded canonical ordering and generator-neutral cross references.
    pub fn validate(&self) -> Result<(), WorldSnapshotValidationError> {
        if self.version != WORLD_SNAPSHOT_VERSION_V1 {
            return Err(WorldSnapshotValidationError::WrongVersion);
        }
        if self.columns.is_empty() {
            return Err(WorldSnapshotValidationError::EmptyWorld);
        }
        strictly_sorted(self.columns.as_slice(), |column| column.coord, "columns")?;
        for column in self.columns.as_slice() {
            validate_position(TilePos::new(column.coord, 0))?;
            if column.runs.is_empty() {
                return Err(WorldSnapshotValidationError::EmptyColumn(column.coord));
            }
            let mut previous: Option<&WorldRunSnapshotV1> = None;
            for run in column.runs.as_slice() {
                validate_run(column.coord, run)?;
                if let Some(previous) = previous {
                    if previous.position.level >= run.run_bottom {
                        return Err(WorldSnapshotValidationError::OverlappingRuns(column.coord));
                    }
                    if previous.position.level.saturating_add(1) == run.run_bottom
                        && previous.substance == run.substance
                    {
                        return Err(WorldSnapshotValidationError::UnmergedRuns(column.coord));
                    }
                }
                previous = Some(run);
            }
        }

        strictly_sorted(self.damage.as_slice(), |entry| entry.position, "damage")?;
        for entry in self.damage.as_slice() {
            validate_position(entry.position)?;
            if entry.remaining == 0
                || entry.remaining >= entry.maximum
                || !is_terrain_toughness(entry.maximum)
            {
                return Err(WorldSnapshotValidationError::InvalidDamage(entry.position));
            }
            if !self.contains_voxel(entry.position) {
                return Err(WorldSnapshotValidationError::DanglingVoxel(entry.position));
            }
        }

        strictly_sorted(
            self.anchors.as_slice(),
            |entry| entry.name.clone(),
            "anchors",
        )?;
        for entry in self.anchors.as_slice() {
            self.require_surface(entry.position)?;
        }
        validate_surface_memberships(self, self.interior_surfaces.as_slice(), |entry| {
            entry.position
        })?;
        strictly_sorted(
            self.interior_roofs.as_slice(),
            |entry| entry.position,
            "interior roofs",
        )?;
        for entry in self.interior_roofs.as_slice() {
            validate_position(entry.position)?;
            if !self.contains_voxel(entry.position) {
                return Err(WorldSnapshotValidationError::DanglingVoxel(entry.position));
            }
        }
        validate_surface_memberships(self, self.special_regions.as_slice(), |entry| {
            entry.position
        })?;
        validate_surface_memberships(self, self.biome_regions.as_slice(), |entry| entry.position)?;
        strictly_sorted(self.blockers.as_slice(), |position| *position, "blockers")?;
        for &position in self.blockers.as_slice() {
            self.require_surface(position)?;
        }

        if let Some(view_hint) = self.view_hint {
            validate_view_hint(view_hint)?;
        }
        strictly_sorted(
            self.lights.as_slice(),
            |entry| entry.stable_id.clone(),
            "lights",
        )?;
        for light in self.lights.as_slice() {
            self.require_surface(light.origin)?;
        }
        strictly_sorted(self.liquids.as_slice(), |entry| entry.position, "liquids")?;
        for liquid in self.liquids.as_slice() {
            validate_position(liquid.position)?;
            if !self.contains_voxel(liquid.position) {
                return Err(WorldSnapshotValidationError::DanglingVoxel(liquid.position));
            }
            if let Some(downstream) = liquid.downstream {
                validate_position(downstream)?;
                // Flow is authored per material run and copied onto every occupied
                // voxel in that run. A lower voxel may therefore name the adjacent
                // downstream run's top above its own level. `hex_map` validates the
                // run-level topology against live voxel state before mutation; the
                // shared untrusted-input boundary enforces adjacency and bounds only.
                if liquid.position.coord.distance(downstream.coord) != 1 {
                    return Err(WorldSnapshotValidationError::InvalidLiquidFlow(
                        liquid.position,
                    ));
                }
            }
        }
        strictly_sorted(
            self.objects.as_slice(),
            |entry| entry.stable_id.clone(),
            "objects",
        )?;
        for object in self.objects.as_slice() {
            self.require_surface(object.root)?;
            if object.rotation_sixths >= 6 {
                return Err(WorldSnapshotValidationError::InvalidObjectRotation);
            }
            strictly_sorted(
                object.blockers.as_slice(),
                |position| *position,
                "object blockers",
            )?;
            for &position in object.blockers.as_slice() {
                self.require_surface(position)?;
            }
        }
        Ok(())
    }

    /// Whether one exact voxel is occupied by a retained non-air run.
    #[must_use]
    pub fn contains_voxel(&self, position: TilePos) -> bool {
        self.columns
            .as_slice()
            .binary_search_by_key(&position.coord, |column| column.coord)
            .ok()
            .and_then(|index| self.columns.get(index))
            .is_some_and(|column| {
                column.runs.as_slice().iter().any(|run| {
                    run.run_bottom <= position.level && position.level <= run.position.level
                })
            })
    }

    /// Whether one exact public exposed surface exists.
    #[must_use]
    pub fn contains_surface(&self, position: TilePos) -> bool {
        self.columns
            .as_slice()
            .binary_search_by_key(&position.coord, |column| column.coord)
            .ok()
            .and_then(|index| self.columns.get(index))
            .is_some_and(|column| {
                column
                    .runs
                    .as_slice()
                    .binary_search_by_key(&position.level, |run| run.position.level)
                    .is_ok()
            })
    }

    fn require_surface(&self, position: TilePos) -> Result<(), WorldSnapshotValidationError> {
        validate_position(position)?;
        if self.contains_surface(position) {
            Ok(())
        } else {
            Err(WorldSnapshotValidationError::DanglingSurface(position))
        }
    }
}

fn validate_surface_memberships<T>(
    snapshot: &WorldSnapshotV1,
    entries: &[T],
    position: impl Fn(&T) -> TilePos + Copy,
) -> Result<(), WorldSnapshotValidationError> {
    strictly_sorted(entries, position, "region memberships")?;
    for entry in entries {
        snapshot.require_surface(position(entry))?;
    }
    Ok(())
}

fn validate_run(
    coord: HexCoord,
    run: &WorldRunSnapshotV1,
) -> Result<(), WorldSnapshotValidationError> {
    validate_position(run.position)?;
    if run.position.coord != coord
        || run.run_bottom < 0
        || run.run_bottom > run.position.level
        || run.run_bottom.unsigned_abs() > MAX_ABS_COMMAND_LEVEL
        || !(0..=MAX_HEADROOM).contains(&run.headroom)
    {
        return Err(WorldSnapshotValidationError::InvalidRun(run.position));
    }
    let bottom = run.span_bottom();
    let top = run.span_top();
    if !bottom.is_finite() || !top.is_finite() || top <= bottom {
        return Err(WorldSnapshotValidationError::InvalidSpan(run.position));
    }
    Ok(())
}

fn validate_view_hint(hint: WorldViewHintSnapshotV1) -> Result<(), WorldSnapshotValidationError> {
    let eye = hint.eye();
    let focus = hint.focus();
    if eye.into_iter().chain(focus).any(|value| !value.is_finite()) {
        return Err(WorldSnapshotValidationError::InvalidViewHint);
    }
    let delta = [eye[0] - focus[0], eye[1] - focus[1], eye[2] - focus[2]];
    if delta[0].mul_add(delta[0], delta[1].mul_add(delta[1], delta[2] * delta[2])) <= f32::EPSILON {
        return Err(WorldSnapshotValidationError::InvalidViewHint);
    }
    Ok(())
}

fn validate_position(position: TilePos) -> Result<(), WorldSnapshotValidationError> {
    if position.coord.x().unsigned_abs() > MAX_ABS_COMMAND_COORDINATE
        || position.coord.y().unsigned_abs() > MAX_ABS_COMMAND_COORDINATE
        || position.coord.z().unsigned_abs() > MAX_ABS_COMMAND_COORDINATE
        || position.level < 0
        || position.level.unsigned_abs() > MAX_ABS_COMMAND_LEVEL
    {
        return Err(WorldSnapshotValidationError::PositionOutsideDomain(
            position,
        ));
    }
    Ok(())
}

fn strictly_sorted<T, K: Ord>(
    values: &[T],
    key: impl Fn(&T) -> K,
    collection: &'static str,
) -> Result<(), WorldSnapshotValidationError> {
    if values
        .windows(2)
        .any(|pair| matches!(pair, [left, right] if key(left) >= key(right)))
    {
        Err(WorldSnapshotValidationError::NonCanonicalCollection(
            collection,
        ))
    } else {
        Ok(())
    }
}

/// Why a world snapshot/delta failed before world-owned resolution or mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldSnapshotValidationError {
    /// Unsupported snapshot/delta version.
    WrongVersion,
    /// No terrain columns were supplied.
    EmptyWorld,
    /// A retained column contained no material run.
    EmptyColumn(HexCoord),
    /// A coordinate or level exceeded the accepted domain.
    PositionOutsideDomain(TilePos),
    /// A run disagreed with its column, level, or headroom contract.
    InvalidRun(TilePos),
    /// A run's exact floating span was non-finite or inverted.
    InvalidSpan(TilePos),
    /// Two runs overlap or are out of order.
    OverlappingRuns(HexCoord),
    /// Adjacent identical material runs were not canonicalized.
    UnmergedRuns(HexCoord),
    /// A collection was unsorted or duplicated.
    NonCanonicalCollection(&'static str),
    /// Partial health was zero, full, impossible, or outside the toughness scale.
    InvalidDamage(TilePos),
    /// Semantic state names a voxel absent from the snapshot.
    DanglingVoxel(TilePos),
    /// Semantic state names an exposed surface absent from the snapshot.
    DanglingSurface(TilePos),
    /// Camera framing was non-finite or degenerate.
    InvalidViewHint,
    /// Liquid flow did not name a horizontally adjacent coordinate.
    InvalidLiquidFlow(TilePos),
    /// Object rotation was outside the six hex orientations.
    InvalidObjectRotation,
}

impl fmt::Display for WorldSnapshotValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::WrongVersion => "world snapshot version is unsupported",
            Self::EmptyWorld => "world snapshot contains no columns",
            Self::EmptyColumn(_) => "world snapshot contains an empty column",
            Self::PositionOutsideDomain(_) => "world snapshot position is outside the domain",
            Self::InvalidRun(_) => "world snapshot contains an invalid material run",
            Self::InvalidSpan(_) => "world snapshot contains an invalid rendered span",
            Self::OverlappingRuns(_) => "world snapshot material runs overlap or are unordered",
            Self::UnmergedRuns(_) => "world snapshot contains adjacent identical runs",
            Self::NonCanonicalCollection(_) => {
                "world snapshot collection is unsorted or contains duplicate keys"
            }
            Self::InvalidDamage(_) => "world snapshot contains impossible partial damage",
            Self::DanglingVoxel(_) => "world snapshot semantic state names a missing voxel",
            Self::DanglingSurface(_) => "world snapshot semantic state names a missing surface",
            Self::InvalidViewHint => "world snapshot view hint is invalid",
            Self::InvalidLiquidFlow(_) => "world snapshot liquid topology is invalid",
            Self::InvalidObjectRotation => "world snapshot object rotation is invalid",
        })
    }
}

impl std::error::Error for WorldSnapshotValidationError {}

/// One ordered upsert/removal in a canonical authority-boundary delta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorldDeltaOperationV1 {
    /// Insert or replace one complete column.
    UpsertColumn(WorldColumnSnapshotV1),
    /// Remove one complete column.
    RemoveColumn(HexCoord),
    /// Insert or replace partial damage.
    UpsertDamage(WorldDamageSnapshotV1),
    /// Remove partial damage.
    RemoveDamage(TilePos),
    /// Insert or replace an anchor.
    UpsertAnchor(WorldAnchorSnapshotV1),
    /// Remove an anchor.
    RemoveAnchor(BoundedText<MAX_IDENTITY_BYTES>),
    /// Insert or replace an interior floor membership.
    UpsertInteriorSurface(InteriorSurfaceSnapshotV1),
    /// Remove an interior floor membership.
    RemoveInteriorSurface(TilePos),
    /// Insert or replace an interior roof membership.
    UpsertInteriorRoof(InteriorRoofSnapshotV1),
    /// Remove an interior roof membership.
    RemoveInteriorRoof(TilePos),
    /// Insert or replace a special region membership.
    UpsertSpecialRegion(SpecialRegionSnapshotV1),
    /// Remove a special region membership.
    RemoveSpecialRegion(TilePos),
    /// Insert or replace a biome membership.
    UpsertBiomeRegion(BiomeRegionSnapshotV1),
    /// Remove a biome membership.
    RemoveBiomeRegion(TilePos),
    /// Add one traversal blocker.
    UpsertBlocker(TilePos),
    /// Remove one traversal blocker.
    RemoveBlocker(TilePos),
    /// Replace the optional view hint.
    SetViewHint(WorldViewHintSnapshotV1),
    /// Clear the optional view hint.
    ClearViewHint,
    /// Insert or replace one gameplay light.
    UpsertLight(WorldLightSnapshotV1),
    /// Remove one gameplay light.
    RemoveLight(BoundedText<MAX_IDENTITY_BYTES>),
    /// Insert or replace one liquid voxel.
    UpsertLiquid(WorldLiquidSnapshotV1),
    /// Remove one liquid voxel.
    RemoveLiquid(TilePos),
    /// Insert or replace one object presentation consequence.
    UpsertObject(WorldObjectSnapshotV1),
    /// Remove one object presentation consequence.
    RemoveObject(BoundedText<MAX_IDENTITY_BYTES>),
}

impl WorldDeltaOperationV1 {
    fn family(&self) -> u8 {
        match self {
            Self::UpsertColumn(_) | Self::RemoveColumn(_) => 0,
            Self::UpsertDamage(_) | Self::RemoveDamage(_) => 1,
            Self::UpsertAnchor(_) | Self::RemoveAnchor(_) => 2,
            Self::UpsertInteriorSurface(_) | Self::RemoveInteriorSurface(_) => 3,
            Self::UpsertInteriorRoof(_) | Self::RemoveInteriorRoof(_) => 4,
            Self::UpsertSpecialRegion(_) | Self::RemoveSpecialRegion(_) => 5,
            Self::UpsertBiomeRegion(_) | Self::RemoveBiomeRegion(_) => 6,
            Self::UpsertBlocker(_) | Self::RemoveBlocker(_) => 7,
            Self::SetViewHint(_) | Self::ClearViewHint => 8,
            Self::UpsertLight(_) | Self::RemoveLight(_) => 9,
            Self::UpsertLiquid(_) | Self::RemoveLiquid(_) => 10,
            Self::UpsertObject(_) | Self::RemoveObject(_) => 11,
        }
    }

    fn key(&self) -> DeltaKey<'_> {
        match self {
            Self::UpsertColumn(value) => DeltaKey::Coord(value.coord),
            Self::RemoveColumn(value) => DeltaKey::Coord(*value),
            Self::UpsertDamage(value) => DeltaKey::Position(value.position),
            Self::RemoveDamage(value) => DeltaKey::Position(*value),
            Self::UpsertAnchor(value) => DeltaKey::Text(value.name.as_str()),
            Self::RemoveAnchor(value) => DeltaKey::Text(value.as_str()),
            Self::UpsertInteriorSurface(value) => DeltaKey::Position(value.position),
            Self::RemoveInteriorSurface(value) => DeltaKey::Position(*value),
            Self::UpsertInteriorRoof(value) => DeltaKey::Position(value.position),
            Self::RemoveInteriorRoof(value) => DeltaKey::Position(*value),
            Self::UpsertSpecialRegion(value) => DeltaKey::Position(value.position),
            Self::RemoveSpecialRegion(value) => DeltaKey::Position(*value),
            Self::UpsertBiomeRegion(value) => DeltaKey::Position(value.position),
            Self::RemoveBiomeRegion(value) => DeltaKey::Position(*value),
            Self::UpsertBlocker(value) | Self::RemoveBlocker(value) => DeltaKey::Position(*value),
            Self::SetViewHint(_) | Self::ClearViewHint => DeltaKey::Singleton,
            Self::UpsertLight(value) => DeltaKey::Text(value.stable_id.as_str()),
            Self::RemoveLight(value) => DeltaKey::Text(value.as_str()),
            Self::UpsertLiquid(value) => DeltaKey::Position(value.position),
            Self::RemoveLiquid(value) => DeltaKey::Position(*value),
            Self::UpsertObject(value) => DeltaKey::Text(value.stable_id.as_str()),
            Self::RemoveObject(value) => DeltaKey::Text(value.as_str()),
        }
    }

    fn validate_payload(&self) -> Result<(), WorldSnapshotValidationError> {
        match self {
            Self::UpsertColumn(column) => {
                validate_position(TilePos::new(column.coord, 0))?;
                if column.runs.is_empty() {
                    return Err(WorldSnapshotValidationError::EmptyColumn(column.coord));
                }
                let mut previous: Option<&WorldRunSnapshotV1> = None;
                for run in column.runs.as_slice() {
                    validate_run(column.coord, run)?;
                    if let Some(previous) = previous {
                        if previous.position.level >= run.run_bottom {
                            return Err(WorldSnapshotValidationError::OverlappingRuns(
                                column.coord,
                            ));
                        }
                        if previous.position.level.saturating_add(1) == run.run_bottom
                            && previous.substance == run.substance
                        {
                            return Err(WorldSnapshotValidationError::UnmergedRuns(column.coord));
                        }
                    }
                    previous = Some(run);
                }
                Ok(())
            }
            Self::RemoveColumn(coord) => validate_position(TilePos::new(*coord, 0)),
            Self::UpsertDamage(value) => {
                validate_position(value.position)?;
                if value.remaining == 0
                    || value.remaining >= value.maximum
                    || !is_terrain_toughness(value.maximum)
                {
                    Err(WorldSnapshotValidationError::InvalidDamage(value.position))
                } else {
                    Ok(())
                }
            }
            Self::RemoveDamage(position)
            | Self::RemoveInteriorSurface(position)
            | Self::RemoveInteriorRoof(position)
            | Self::RemoveSpecialRegion(position)
            | Self::RemoveBiomeRegion(position)
            | Self::UpsertBlocker(position)
            | Self::RemoveBlocker(position)
            | Self::RemoveLiquid(position) => validate_position(*position),
            Self::UpsertAnchor(value) => validate_position(value.position),
            Self::RemoveAnchor(_) | Self::RemoveLight(_) | Self::RemoveObject(_) => Ok(()),
            Self::UpsertInteriorSurface(value) => validate_position(value.position),
            Self::UpsertInteriorRoof(value) => validate_position(value.position),
            Self::UpsertSpecialRegion(value) => validate_position(value.position),
            Self::UpsertBiomeRegion(value) => validate_position(value.position),
            Self::SetViewHint(value) => validate_view_hint(*value),
            Self::ClearViewHint => Ok(()),
            Self::UpsertLight(value) => validate_position(value.origin),
            Self::UpsertLiquid(value) => {
                validate_position(value.position)?;
                if let Some(downstream) = value.downstream {
                    validate_position(downstream)?;
                    if value.position.coord.distance(downstream.coord) != 1 {
                        return Err(WorldSnapshotValidationError::InvalidLiquidFlow(
                            value.position,
                        ));
                    }
                }
                Ok(())
            }
            Self::UpsertObject(value) => {
                validate_position(value.root)?;
                if value.rotation_sixths >= 6 {
                    return Err(WorldSnapshotValidationError::InvalidObjectRotation);
                }
                strictly_sorted(
                    value.blockers.as_slice(),
                    |position| *position,
                    "object blockers",
                )?;
                for &position in value.blockers.as_slice() {
                    validate_position(position)?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeltaKey<'a> {
    Coord(HexCoord),
    Position(TilePos),
    Text(&'a str),
    Singleton,
}

impl Ord for DeltaKey<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Coord(left), Self::Coord(right)) => left.cmp(right),
            (Self::Position(left), Self::Position(right)) => left.cmp(right),
            (Self::Text(left), Self::Text(right)) => left.cmp(right),
            (Self::Singleton, Self::Singleton) => Ordering::Equal,
            _ => Ordering::Equal,
        }
    }
}

impl PartialOrd for DeltaKey<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Ordered idempotent mutations between two canonical world snapshots.
#[derive(Message, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldDeltaV1 {
    /// Exact delta schema.
    pub version: u16,
    /// Authority boundary that owns this delta.
    pub authority_sequence: AuthoritySequence,
    /// Fingerprint required before application.
    pub base_fingerprint: PublicWorldFingerprint,
    /// Fingerprint required after transactional application.
    pub target_fingerprint: PublicWorldFingerprint,
    /// Operations grouped by collection and strictly ordered by stable key.
    pub operations: BoundedVec<WorldDeltaOperationV1, MAX_WORLD_DELTA_OPERATIONS>,
}

impl WorldDeltaV1 {
    /// Validates operation payloads and rejects duplicate/out-of-order keys.
    pub fn validate(&self) -> Result<(), WorldSnapshotValidationError> {
        if self.version != WORLD_DELTA_VERSION_V1 {
            return Err(WorldSnapshotValidationError::WrongVersion);
        }
        for operation in self.operations.as_slice() {
            operation.validate_payload()?;
        }
        if self.operations.as_slice().windows(2).any(|pair| {
            matches!(pair, [left, right] if {
                let left_family = left.family();
                let right_family = right.family();
                left_family > right_family
                    || (left_family == right_family && left.key() >= right.key())
            })
        }) {
            return Err(WorldSnapshotValidationError::NonCanonicalCollection(
                "delta operations",
            ));
        }
        Ok(())
    }
}

/// Stable light-domain projection retained in remembered player terrain.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PlayerLightDomainV1 {
    /// Open-air domain.
    #[default]
    Exterior,
    /// One deterministic map-local interior.
    Interior(u32),
}

/// Known entries cannot represent Unknown; absence is Unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PlayerKnowledgeStateV1 {
    /// Last observed exact terrain facts.
    Remembered,
    /// Currently observed exact terrain facts.
    Observed,
}

/// One remembered/observed exact public surface projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerKnownSurfaceV1 {
    /// Exact exposed surface.
    pub position: TilePos,
    /// Remembered or currently observed.
    pub state: PlayerKnowledgeStateV1,
    /// Inclusive run bottom at the observation boundary.
    pub run_bottom: i32,
    /// Exact `HexSpan::bottom` bit pattern.
    pub span_bottom_bits: u32,
    /// Exact `HexSpan::top` bit pattern.
    pub span_top_bits: u32,
    /// Stable substance name.
    pub substance: BoundedText<MAX_IDENTITY_BYTES>,
    /// Published headroom.
    pub headroom: i32,
    /// Published material support fact.
    pub is_solid: bool,
    /// Published contextual blocker fact.
    pub blocked: bool,
    /// Exterior or exact interior domain.
    pub light_domain: PlayerLightDomainV1,
}

/// Complete shared player-faction remembered terrain view.
///
/// Hostile unit, lattice, and combat state are intentionally not representable here.
#[derive(Message, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerKnowledgeSnapshotV1 {
    /// Exact knowledge schema.
    pub version: u16,
    /// Known surfaces in exact position order; absence means Unknown.
    pub surfaces: BoundedVec<PlayerKnownSurfaceV1, MAX_WORLD_PROJECTION_ENTRIES>,
}

impl PlayerKnowledgeSnapshotV1 {
    /// Validates ordering and exact public projection values.
    pub fn validate(&self) -> Result<(), WorldSnapshotValidationError> {
        if self.version != PLAYER_KNOWLEDGE_SNAPSHOT_VERSION_V1 {
            return Err(WorldSnapshotValidationError::WrongVersion);
        }
        strictly_sorted(
            self.surfaces.as_slice(),
            |entry| entry.position,
            "knowledge",
        )?;
        for entry in self.surfaces.as_slice() {
            validate_position(entry.position)?;
            if entry.run_bottom < 0
                || entry.run_bottom > entry.position.level
                || !(0..=MAX_HEADROOM).contains(&entry.headroom)
            {
                return Err(WorldSnapshotValidationError::InvalidRun(entry.position));
            }
            let bottom = f32::from_bits(entry.span_bottom_bits);
            let top = f32::from_bits(entry.span_top_bits);
            if !bottom.is_finite() || !top.is_finite() || top <= bottom {
                return Err(WorldSnapshotValidationError::InvalidSpan(entry.position));
            }
        }
        Ok(())
    }
}

/// Restart-capable reconnect baseline followed by ordered `WorldDeltaV1` messages.
#[derive(Message, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveSessionSnapshotV1 {
    /// Exact live snapshot schema.
    pub version: u16,
    /// Frozen session and deterministic map identity.
    pub manifest: SessionManifestV1,
    /// Complete current world-owned state.
    pub world: WorldSnapshotV1,
    /// Client-authorized shared player-faction terrain knowledge.
    pub player_knowledge: PlayerKnowledgeSnapshotV1,
    /// Client-authorized unit replicas in stable unit-id order.
    pub units: BoundedVec<UnitReplica, MAX_SESSION_UNITS>,
    /// Client-authorized global session projection.
    pub session: SessionReplica,
    /// Exact authority baseline represented by all projections above.
    pub baseline_sequence: AuthoritySequence,
}

impl LiveSessionSnapshotV1 {
    /// Validates nested contracts and requires all three baseline declarations to agree.
    pub fn validate_with_header(
        &self,
        header: LiveSnapshotHeaderV1,
    ) -> Result<(), LiveSessionSnapshotValidationError> {
        if self.version != LIVE_SESSION_SNAPSHOT_VERSION_V1 {
            return Err(LiveSessionSnapshotValidationError::WrongVersion);
        }
        self.manifest
            .validate()
            .map_err(LiveSessionSnapshotValidationError::Manifest)?;
        self.world
            .validate()
            .map_err(LiveSessionSnapshotValidationError::World)?;
        self.player_knowledge
            .validate()
            .map_err(LiveSessionSnapshotValidationError::World)?;
        self.session
            .validate()
            .map_err(LiveSessionSnapshotValidationError::Replica)?;
        strictly_sorted(self.units.as_slice(), |unit| unit.unit, "unit replicas")
            .map_err(LiveSessionSnapshotValidationError::World)?;
        for unit in self.units.as_slice() {
            unit.validate()
                .map_err(LiveSessionSnapshotValidationError::Replica)?;
        }
        if self.baseline_sequence != self.session.authority_sequence
            || self.baseline_sequence != header.baseline_sequence
        {
            return Err(LiveSessionSnapshotValidationError::BaselineMismatch);
        }
        Ok(())
    }
}

/// Why a complete reconnect snapshot failed closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveSessionSnapshotValidationError {
    /// Unsupported complete-snapshot schema.
    WrongVersion,
    /// Frozen session manifest was invalid.
    Manifest(ManifestValidationError),
    /// World or player knowledge was structurally invalid.
    World(WorldSnapshotValidationError),
    /// Unit/session projection was internally inconsistent.
    Replica(ReplicaValidationError),
    /// Fixed header, payload baseline, and session projection disagreed.
    BaselineMismatch,
}

impl fmt::Display for LiveSessionSnapshotValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::WrongVersion => "live session snapshot version is unsupported",
            Self::Manifest(_) => "live session manifest is invalid",
            Self::World(_) => "live session world projection is invalid",
            Self::Replica(_) => "live session replica projection is invalid",
            Self::BaselineMismatch => "live session authority baselines disagree",
        })
    }
}

impl std::error::Error for LiveSessionSnapshotValidationError {}

#[cfg(test)]
mod tests {
    use hex_core::{
        ControlOwner, Faction, HexCoord, Mode, Pause, PendingDecision, PlayerSeat, SimSeeds,
        TilePos, UnitId,
    };

    use super::*;
    use crate::{
        BuildIdentityV1, ContentFingerprint, MapManifestV1, ProtocolVersion, RosterEntryV1,
        RulesManifestV1, SessionInstanceId, UnitDeploymentV1,
    };

    fn text(value: &str) -> BoundedText<MAX_IDENTITY_BYTES> {
        BoundedText::new(value).expect("fixture text should fit")
    }

    fn run(coord: HexCoord, bottom: i16, top: i16, material: &str) -> WorldRunSnapshotV1 {
        WorldRunSnapshotV1 {
            position: TilePos::new(coord, i32::from(top)),
            run_bottom: i32::from(bottom),
            span_bottom_bits: f32::from(bottom).to_bits(),
            span_top_bits: f32::from(top.saturating_add(1)).to_bits(),
            substance: text(material),
            headroom: MAX_HEADROOM,
        }
    }

    fn world() -> WorldSnapshotV1 {
        let coord = HexCoord::ORIGIN;
        WorldSnapshotV1 {
            version: WORLD_SNAPSHOT_VERSION_V1,
            public_fingerprint: PublicWorldFingerprint(77),
            columns: BoundedVec::new(vec![WorldColumnSnapshotV1 {
                coord,
                runs: BoundedVec::new(vec![run(coord, 0, 2, "stone")]).expect("runs fit"),
            }])
            .expect("columns fit"),
            damage: BoundedVec::new(vec![WorldDamageSnapshotV1 {
                position: TilePos::new(coord, 1),
                remaining: 1,
                maximum: 2,
            }])
            .expect("damage fits"),
            anchors: BoundedVec::new(vec![WorldAnchorSnapshotV1 {
                name: text("party_start"),
                position: TilePos::new(coord, 2),
            }])
            .expect("anchors fit"),
            interior_surfaces: BoundedVec::default(),
            interior_roofs: BoundedVec::default(),
            special_regions: BoundedVec::default(),
            biome_regions: BoundedVec::new(vec![BiomeRegionSnapshotV1 {
                position: TilePos::new(coord, 2),
                region: 1,
            }])
            .expect("biomes fit"),
            blockers: BoundedVec::default(),
            view_hint: Some(WorldViewHintSnapshotV1 {
                eye_bits: [0.0_f32.to_bits(), 10.0_f32.to_bits(), 10.0_f32.to_bits()],
                focus_bits: [0.0_f32.to_bits(); 3],
            }),
            lights: BoundedVec::new(vec![WorldLightSnapshotV1 {
                stable_id: text("light.0"),
                origin: TilePos::new(coord, 2),
                illumination: WorldIlluminationV1::Bright,
                radius: 4,
            }])
            .expect("lights fit"),
            liquids: BoundedVec::default(),
            objects: BoundedVec::default(),
        }
    }

    fn manifest() -> SessionManifestV1 {
        SessionManifestV1 {
            session_instance_id: SessionInstanceId::from_bytes([3; 16]),
            protocol: ProtocolVersion::default(),
            build: BuildIdentityV1::new("0.4.0", "fixture").expect("build fits"),
            content_fingerprint: ContentFingerprint(1),
            scenario_identity: text("sandbox"),
            map: MapManifestV1 {
                catalog_identity: text("fixture-map"),
                seed: 1,
                generator_identity: text("fixture-generator"),
                generator_version: 1,
                expected_public_fingerprint: PublicWorldFingerprint(77),
            },
            rules: RulesManifestV1 {
                profile_identity: text("default"),
                fingerprint: 2,
            },
            shipped_roster: BoundedVec::new(vec![RosterEntryV1 {
                unit: UnitId(1),
                archetype_identity: text("warrior"),
                character_identity: text("shipped-warrior"),
                faction: Faction::Player,
            }])
            .expect("roster fits"),
            deployment: BoundedVec::new(vec![UnitDeploymentV1 {
                unit: UnitId(1),
                position: TilePos::new(HexCoord::ORIGIN, 2),
            }])
            .expect("deployment fits"),
            simulation_seeds: SimSeeds {
                world: 1,
                ai_flavor: 2,
                cosmetic: 3,
            },
        }
    }

    #[test]
    fn world_snapshot_round_trip_retains_complete_public_tuple() {
        let snapshot = world();
        assert_eq!(snapshot.validate(), Ok(()));
        let json = serde_json::to_string(&snapshot).expect("snapshot should serialize");
        let restored: WorldSnapshotV1 =
            serde_json::from_str(&json).expect("snapshot should deserialize");
        assert_eq!(restored, snapshot);
        assert_eq!(restored.validate(), Ok(()));
    }

    #[test]
    fn world_snapshot_rejects_unsorted_and_dangling_semantics() {
        let mut snapshot = world();
        snapshot.anchors = BoundedVec::new(vec![
            WorldAnchorSnapshotV1 {
                name: text("z"),
                position: TilePos::new(HexCoord::ORIGIN, 2),
            },
            WorldAnchorSnapshotV1 {
                name: text("a"),
                position: TilePos::new(HexCoord::ORIGIN, 2),
            },
        ])
        .expect("anchors fit");
        assert!(matches!(
            snapshot.validate(),
            Err(WorldSnapshotValidationError::NonCanonicalCollection(
                "anchors"
            ))
        ));

        let mut snapshot = world();
        snapshot.blockers =
            BoundedVec::new(vec![TilePos::new(HexCoord::ORIGIN, 1)]).expect("blocker fits");
        assert!(matches!(
            snapshot.validate(),
            Err(WorldSnapshotValidationError::DanglingSurface(_))
        ));
    }

    #[test]
    fn liquid_voxels_accept_an_adjacent_run_level_downstream() {
        let mut snapshot = world();
        let source = TilePos::new(HexCoord::ORIGIN, 0);
        let downstream = TilePos::new(HexCoord::from_axial(1, 0), 2);
        let liquid = WorldLiquidSnapshotV1 {
            position: source,
            substance: text("water"),
            flow: WorldLiquidFlowV1::Current,
            downstream: Some(downstream),
        };
        snapshot.liquids = BoundedVec::new(vec![liquid.clone()]).expect("liquid fits");

        assert_eq!(snapshot.validate(), Ok(()));

        let delta = WorldDeltaV1 {
            version: WORLD_DELTA_VERSION_V1,
            authority_sequence: AuthoritySequence(3),
            base_fingerprint: PublicWorldFingerprint(1),
            target_fingerprint: PublicWorldFingerprint(2),
            operations: BoundedVec::new(vec![WorldDeltaOperationV1::UpsertLiquid(liquid)])
                .expect("operation fits"),
        };
        assert_eq!(delta.validate(), Ok(()));
    }

    #[test]
    fn delta_rejects_duplicate_keys_even_across_remove_and_upsert() {
        let delta = WorldDeltaV1 {
            version: WORLD_DELTA_VERSION_V1,
            authority_sequence: AuthoritySequence(2),
            base_fingerprint: PublicWorldFingerprint(1),
            target_fingerprint: PublicWorldFingerprint(2),
            operations: BoundedVec::new(vec![
                WorldDeltaOperationV1::RemoveDamage(TilePos::ORIGIN),
                WorldDeltaOperationV1::UpsertDamage(WorldDamageSnapshotV1 {
                    position: TilePos::ORIGIN,
                    remaining: 1,
                    maximum: 2,
                }),
            ])
            .expect("operations fit"),
        };
        assert!(matches!(
            delta.validate(),
            Err(WorldSnapshotValidationError::NonCanonicalCollection(
                "delta operations"
            ))
        ));
    }

    #[test]
    fn live_snapshot_requires_header_payload_and_session_baselines_to_match() {
        let session = SessionReplica {
            authority_sequence: AuthoritySequence(9),
            mode: Mode::Exploring,
            pause: Pause::default(),
            initiative: BoundedVec::default(),
            active_turn: None,
            round: 0,
            pending_decision: PendingDecision::default(),
            outcome: None,
        };
        let unit = UnitReplica {
            unit: UnitId(1),
            archetype: crate::ArchetypeIdentityV1::new("warrior").expect("fixture archetype fits"),
            faction: Faction::Player,
            position: TilePos::new(HexCoord::ORIGIN, 2),
            motion: None,
            owner: ControlOwner(PlayerSeat::HOST),
            lattice: None,
            downed: false,
            turn: None,
            effects: BoundedVec::default(),
        };
        let snapshot = LiveSessionSnapshotV1 {
            version: LIVE_SESSION_SNAPSHOT_VERSION_V1,
            manifest: manifest(),
            world: world(),
            player_knowledge: PlayerKnowledgeSnapshotV1 {
                version: PLAYER_KNOWLEDGE_SNAPSHOT_VERSION_V1,
                surfaces: BoundedVec::default(),
            },
            units: BoundedVec::new(vec![unit]).expect("unit fits"),
            session,
            baseline_sequence: AuthoritySequence(9),
        };
        let header = LiveSnapshotHeaderV1::new(AuthoritySequence(9), 0).expect("header fits");
        assert_eq!(snapshot.validate_with_header(header), Ok(()));

        let mut mutated_world = snapshot.clone();
        mutated_world.world.public_fingerprint = PublicWorldFingerprint(78);
        assert_eq!(mutated_world.validate_with_header(header), Ok(()));

        let wrong = LiveSnapshotHeaderV1::new(AuthoritySequence(8), 0).expect("header fits");
        assert_eq!(
            snapshot.validate_with_header(wrong),
            Err(LiveSessionSnapshotValidationError::BaselineMismatch)
        );
    }
}
