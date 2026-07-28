//! Durable, editor-authored voxel object contracts.
//!
//! Object blueprints deliberately use stable textual style ids and object-local
//! coordinates. They never contain runtime substance ids or world
//! [`TilePos`](hex_core::TilePos) values: an authored object must survive palette
//! reordering and may be placed at any world position later.

use std::collections::{BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::art_palette::{ObjectAssetId, VoxelStyleCatalog, VoxelStyleId};

/// Current on-disk schema understood by [`ObjectBlueprint`].
pub const OBJECT_BLUEPRINT_SCHEMA_VERSION: u16 = 1;
/// Largest horizontal authoring radius accepted by the editor contract.
pub const MAX_OBJECT_RADIUS: u8 = 12;
/// Largest number of vertical levels in one authoring canvas.
pub const MAX_OBJECT_HEIGHT: u8 = 64;
/// Largest number of occupied cells in one object.
pub const MAX_OBJECT_VOXELS: usize = 8_192;

const FINGERPRINT_DOMAIN: &[u8] = b"bevy-hex-game/object-blueprint/v1";

/// One horizontal object-local hex, expressed in axial coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalAxialCoord {
    /// Axial q coordinate.
    pub q: i32,
    /// Axial r coordinate.
    pub r: i32,
}

impl LocalAxialCoord {
    /// Creates an object-local axial coordinate.
    #[must_use]
    pub const fn new(q: i32, r: i32) -> Self {
        Self { q, r }
    }

    /// Returns the hex distance from the canvas center.
    #[must_use]
    pub fn radius(self) -> i64 {
        let q = i64::from(self.q);
        let r = i64::from(self.r);
        q.abs().max(r.abs()).max((-q - r).abs())
    }

    /// Rotates this coordinate clockwise by exactly 60 degrees around `pivot`.
    ///
    /// Returns [`None`] only when maliciously large coordinates overflow `i32`.
    #[must_use]
    pub fn rotated_clockwise_60(self, pivot: Self) -> Option<Self> {
        let q = self.q.checked_sub(pivot.q)?;
        let r = self.r.checked_sub(pivot.r)?;
        let rotated_q = r.checked_neg()?;
        let rotated_r = q.checked_add(r)?;
        Some(Self {
            q: rotated_q.checked_add(pivot.q)?,
            r: rotated_r.checked_add(pivot.r)?,
        })
    }
}

/// One exact object-local cell: an axial hex plus a vertical voxel level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalVoxelCoord {
    /// Axial q coordinate.
    pub q: i32,
    /// Axial r coordinate.
    pub r: i32,
    /// Integer level relative to the object canvas.
    pub level: i32,
}

impl LocalVoxelCoord {
    /// Creates an exact object-local cell.
    #[must_use]
    pub const fn new(q: i32, r: i32, level: i32) -> Self {
        Self { q, r, level }
    }

    /// Returns this cell's horizontal coordinate.
    #[must_use]
    pub const fn axial(self) -> LocalAxialCoord {
        LocalAxialCoord::new(self.q, self.r)
    }

    /// Rotates this cell clockwise by exactly 60 degrees around `pivot`.
    ///
    /// Rotation is horizontal; the level and the pivot's level do not alter it.
    #[must_use]
    pub fn rotated_clockwise_60(self, pivot: Self) -> Option<Self> {
        let rotated = self.axial().rotated_clockwise_60(pivot.axial())?;
        Some(Self::new(rotated.q, rotated.r, self.level))
    }

    fn face_neighbours(self) -> [Option<Self>; 8] {
        [
            self.q
                .checked_add(1)
                .map(|q| Self::new(q, self.r, self.level)),
            self.q
                .checked_sub(1)
                .map(|q| Self::new(q, self.r, self.level)),
            self.r
                .checked_add(1)
                .map(|r| Self::new(self.q, r, self.level)),
            self.r
                .checked_sub(1)
                .map(|r| Self::new(self.q, r, self.level)),
            self.q
                .checked_add(1)
                .and_then(|q| self.r.checked_sub(1).map(|r| Self::new(q, r, self.level))),
            self.q
                .checked_sub(1)
                .and_then(|q| self.r.checked_add(1).map(|r| Self::new(q, r, self.level))),
            self.level
                .checked_add(1)
                .map(|level| Self::new(self.q, self.r, level)),
            self.level
                .checked_sub(1)
                .map(|level| Self::new(self.q, self.r, level)),
        ]
    }
}

/// The finite authoring canvas stored with an object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectBounds {
    /// Horizontal hex radius around `(q: 0, r: 0)`.
    pub radius: u8,
    /// Lowest permitted local level.
    pub min_level: i32,
    /// Number of permitted levels, beginning at [`Self::min_level`].
    pub height: u8,
}

impl ObjectBounds {
    /// The standard editor canvas: radius 6 and levels `0..36`.
    pub const DEFAULT: Self = Self {
        radius: 6,
        min_level: 0,
        height: 36,
    };

    /// Returns whether an exact cell is inside these bounds.
    #[must_use]
    pub fn contains(self, position: LocalVoxelCoord) -> bool {
        if position.axial().radius() > i64::from(self.radius) {
            return false;
        }
        let level = i64::from(position.level);
        let min = i64::from(self.min_level);
        level >= min && level < min + i64::from(self.height)
    }

    /// Returns whether a horizontal mask cell is inside these bounds.
    #[must_use]
    pub fn contains_axial(self, position: LocalAxialCoord) -> bool {
        position.radius() <= i64::from(self.radius)
    }

    fn validate(self) -> Result<(), String> {
        if self.radius > MAX_OBJECT_RADIUS {
            return Err(format!(
                "bounds radius {} exceeds the maximum {MAX_OBJECT_RADIUS}",
                self.radius
            ));
        }
        if self.height == 0 {
            return Err("bounds height must be at least 1".to_owned());
        }
        if self.height > MAX_OBJECT_HEIGHT {
            return Err(format!(
                "bounds height {} exceeds the maximum {MAX_OBJECT_HEIGHT}",
                self.height
            ));
        }
        self.min_level
            .checked_add(i32::from(self.height))
            .ok_or_else(|| "bounds level range overflows i32".to_owned())?;
        Ok(())
    }
}

impl Default for ObjectBounds {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Broad authoring and runtime purpose of an object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectCategory {
    /// A rooted plant whose geometry is structurally connected.
    Plant,
    /// A static effect sculpture, which may contain floating cells.
    Effect,
    /// A general static prop.
    Prop,
}

/// Whether occupied cells must form one face-connected component at the origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectivityPolicy {
    /// Every occupied cell must connect to the grounded origin through shared faces.
    Grounded,
    /// Cells may form disconnected or floating components.
    Free,
}

/// Semantic role of one voxel in a plant object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PlantPart {
    /// Ground-level root, also used to derive the exact blocker footprint.
    Root,
    /// Main vertical support.
    Trunk,
    /// Secondary woody support.
    Branch,
    /// Leaf or needle volume.
    Foliage,
    /// Nonstructural visual accent such as a flower or fruit.
    Accent,
}

/// Semantic role of one voxel in a static effect sculpture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EffectPart {
    /// Primary origin of the effect.
    Core,
    /// Directional or residual trail.
    Trail,
    /// Secondary visual accent.
    Accent,
}

/// Semantic role of one voxel in a prop object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PropPart {
    /// Load-bearing or primary geometry.
    Structure,
    /// Nonstructural visual detail.
    Detail,
}

/// Category-safe semantic role attached to an occupied cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ObjectPart {
    /// A plant-specific part.
    Plant(PlantPart),
    /// An effect-specific part.
    Effect(EffectPart),
    /// A prop-specific part.
    Prop(PropPart),
}

impl ObjectPart {
    fn category(self) -> ObjectCategory {
        match self {
            Self::Plant(_) => ObjectCategory::Plant,
            Self::Effect(_) => ObjectCategory::Effect,
            Self::Prop(_) => ObjectCategory::Prop,
        }
    }
}

/// One occupied local cell and its durable visual and semantic references.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectPlacement {
    /// Exact object-local position.
    pub position: LocalVoxelCoord,
    /// Stable visual style key from the shared style catalog.
    pub style: VoxelStyleId,
    /// Category-safe semantic role.
    pub part: ObjectPart,
}

/// A complete, editor-authored static voxel object.
///
/// Intrinsic invariants are checked while deserializing. Call [`Self::validate`] as
/// well once the shared style catalog is available, so every style dependency is
/// proven to exist before the object is used or saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectBlueprint {
    /// On-disk schema version.
    pub schema_version: u16,
    /// Stable object identity, independent of its file path and display name.
    pub id: ObjectAssetId,
    /// Human-facing editable name.
    pub display_name: String,
    /// Object purpose and permitted part vocabulary.
    pub category: ObjectCategory,
    /// Stored authoring canvas.
    pub bounds: ObjectBounds,
    /// Required geometric connectivity.
    pub connectivity: ConnectivityPolicy,
    /// Pivot and semantic root of the object.
    pub origin: LocalVoxelCoord,
    /// Occupied cells. Positions must be unique.
    pub placements: Vec<ObjectPlacement>,
    /// Exact horizontal gameplay blocker footprint.
    pub blocker_footprint: Vec<LocalAxialCoord>,
    /// Exact occupied foliage cells eligible for canopy cutaway.
    pub canopy_occluders: Vec<LocalVoxelCoord>,
}

/// Derived-deserialization shape used to validate before exposing a blueprint.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnvalidatedObjectBlueprint {
    schema_version: u16,
    id: ObjectAssetId,
    display_name: String,
    category: ObjectCategory,
    bounds: ObjectBounds,
    connectivity: ConnectivityPolicy,
    origin: LocalVoxelCoord,
    placements: Vec<ObjectPlacement>,
    blocker_footprint: Vec<LocalAxialCoord>,
    canopy_occluders: Vec<LocalVoxelCoord>,
}

/// Borrowed, canonically ordered serialization shape.
#[derive(Serialize)]
struct CanonicalObjectBlueprint<'a> {
    schema_version: u16,
    id: &'a ObjectAssetId,
    display_name: &'a str,
    category: ObjectCategory,
    bounds: ObjectBounds,
    connectivity: ConnectivityPolicy,
    origin: LocalVoxelCoord,
    placements: Vec<&'a ObjectPlacement>,
    blocker_footprint: Vec<LocalAxialCoord>,
    canopy_occluders: Vec<LocalVoxelCoord>,
}

impl Serialize for ObjectBlueprint {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut placements: Vec<_> = self.placements.iter().collect();
        placements.sort_by(|left, right| {
            left.position
                .cmp(&right.position)
                .then_with(|| left.style.as_str().cmp(right.style.as_str()))
                .then_with(|| left.part.cmp(&right.part))
        });
        let mut blocker_footprint = self.blocker_footprint.clone();
        blocker_footprint.sort_unstable();
        let mut canopy_occluders = self.canopy_occluders.clone();
        canopy_occluders.sort_unstable();

        CanonicalObjectBlueprint {
            schema_version: self.schema_version,
            id: &self.id,
            display_name: &self.display_name,
            category: self.category,
            bounds: self.bounds,
            connectivity: self.connectivity,
            origin: self.origin,
            placements,
            blocker_footprint,
            canopy_occluders,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ObjectBlueprint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = UnvalidatedObjectBlueprint::deserialize(deserializer)?;
        let blueprint = Self {
            schema_version: raw.schema_version,
            id: raw.id,
            display_name: raw.display_name,
            category: raw.category,
            bounds: raw.bounds,
            connectivity: raw.connectivity,
            origin: raw.origin,
            placements: raw.placements,
            blocker_footprint: raw.blocker_footprint,
            canopy_occluders: raw.canopy_occluders,
        };
        blueprint
            .validate_intrinsic()
            .map_err(serde::de::Error::custom)?;
        Ok(blueprint)
    }
}

impl ObjectBlueprint {
    /// Validates geometry, category semantics, masks, and every style dependency.
    pub fn validate(&self, styles: &VoxelStyleCatalog) -> Result<(), String> {
        self.validate_intrinsic()?;
        self.validate_style_dependencies(|style| styles.contains(style))
    }

    /// Validates everything that does not require another asset file.
    ///
    /// Editors can call this before the style catalog has loaded. Production loading
    /// must still call [`Self::validate`] before accepting the object.
    pub fn validate_intrinsic(&self) -> Result<(), String> {
        if self.schema_version != OBJECT_BLUEPRINT_SCHEMA_VERSION {
            return Err(format!(
                "object '{}' uses schema version {}; expected {OBJECT_BLUEPRINT_SCHEMA_VERSION}",
                self.id.as_str(),
                self.schema_version
            ));
        }
        if self.display_name.trim().is_empty() {
            return Err(format!(
                "object '{}' must have a non-empty display name",
                self.id.as_str()
            ));
        }
        self.bounds.validate()?;
        if self.placements.is_empty() {
            return Err(format!(
                "object '{}' must contain at least one voxel",
                self.id.as_str()
            ));
        }
        if self.placements.len() > MAX_OBJECT_VOXELS {
            return Err(format!(
                "object '{}' contains {} voxels; the maximum is {MAX_OBJECT_VOXELS}",
                self.id.as_str(),
                self.placements.len()
            ));
        }
        if !self.bounds.contains(self.origin) {
            return Err(format!(
                "object '{}' origin {:?} lies outside its authoring bounds",
                self.id.as_str(),
                self.origin
            ));
        }

        let mut occupied = BTreeSet::new();
        for placement in &self.placements {
            if !self.bounds.contains(placement.position) {
                return Err(format!(
                    "object '{}' placement {:?} lies outside its authoring bounds",
                    self.id.as_str(),
                    placement.position
                ));
            }
            if placement.part.category() != self.category {
                return Err(format!(
                    "object '{}' is {:?} but placement {:?} uses {:?}",
                    self.id.as_str(),
                    self.category,
                    placement.position,
                    placement.part
                ));
            }
            if !occupied.insert(placement.position) {
                return Err(format!(
                    "object '{}' has overlapping placements at {:?}",
                    self.id.as_str(),
                    placement.position
                ));
            }
        }

        let origin_part = self
            .placements
            .iter()
            .find(|placement| placement.position == self.origin)
            .map(|placement| placement.part)
            .ok_or_else(|| {
                format!(
                    "object '{}' origin {:?} is not occupied",
                    self.id.as_str(),
                    self.origin
                )
            })?;

        let blockers = self.validate_blocker_mask(&occupied)?;
        self.validate_canopy_mask(&occupied)?;

        match self.category {
            ObjectCategory::Plant => {
                self.validate_plant(origin_part, &blockers)?;
                validate_connected(self.id.as_str(), self.origin, &occupied)?;
            }
            ObjectCategory::Effect => self.validate_effect(origin_part)?,
            ObjectCategory::Prop => {
                self.validate_prop(origin_part)?;
                if self.connectivity == ConnectivityPolicy::Grounded {
                    validate_connected(self.id.as_str(), self.origin, &occupied)?;
                }
            }
        }
        Ok(())
    }

    /// Produces a deterministic fingerprint of all serialized object semantics.
    ///
    /// Collection order and RON formatting do not affect the result. Invalid
    /// intrinsic data is rejected instead of being assigned an identity.
    pub fn semantic_fingerprint(&self) -> Result<u64, String> {
        self.validate_intrinsic()?;
        let mut encoder = FingerprintEncoder::new();
        encoder.bytes(FINGERPRINT_DOMAIN)?;
        encoder.u16(self.schema_version);
        encoder.string(self.id.as_str())?;
        encoder.string(&self.display_name)?;
        encoder.u8(category_tag(self.category));
        encoder.u8(self.bounds.radius);
        encoder.i32(self.bounds.min_level);
        encoder.u8(self.bounds.height);
        encoder.u8(connectivity_tag(self.connectivity));
        encoder.position(self.origin);

        let mut placements: Vec<&ObjectPlacement> = self.placements.iter().collect();
        placements.sort_by(|left, right| {
            left.position
                .cmp(&right.position)
                .then_with(|| left.style.as_str().cmp(right.style.as_str()))
                .then_with(|| left.part.cmp(&right.part))
        });
        encoder.length(placements.len(), "placements")?;
        for placement in placements {
            encoder.position(placement.position);
            encoder.string(placement.style.as_str())?;
            encoder.u8(part_tag(placement.part));
        }

        let blockers: BTreeSet<_> = self.blocker_footprint.iter().copied().collect();
        encoder.length(blockers.len(), "blocker footprint")?;
        for blocker in blockers {
            encoder.axial(blocker);
        }

        let canopy: BTreeSet<_> = self.canopy_occluders.iter().copied().collect();
        encoder.length(canopy.len(), "canopy occluders")?;
        for position in canopy {
            encoder.position(position);
        }
        Ok(encoder.finish())
    }

    fn validate_style_dependencies(
        &self,
        mut contains: impl FnMut(&VoxelStyleId) -> bool,
    ) -> Result<(), String> {
        for placement in &self.placements {
            if !contains(&placement.style) {
                return Err(format!(
                    "object '{}' placement {:?} references missing voxel style '{}'",
                    self.id.as_str(),
                    placement.position,
                    placement.style.as_str()
                ));
            }
        }
        Ok(())
    }

    fn validate_blocker_mask(
        &self,
        occupied: &BTreeSet<LocalVoxelCoord>,
    ) -> Result<BTreeSet<LocalAxialCoord>, String> {
        let mut blockers = BTreeSet::new();
        for blocker in &self.blocker_footprint {
            if !self.bounds.contains_axial(*blocker) {
                return Err(format!(
                    "object '{}' blocker {:?} lies outside its authoring bounds",
                    self.id.as_str(),
                    blocker
                ));
            }
            if !blockers.insert(*blocker) {
                return Err(format!(
                    "object '{}' repeats blocker {:?}",
                    self.id.as_str(),
                    blocker
                ));
            }
            if !occupied.iter().any(|position| position.axial() == *blocker) {
                return Err(format!(
                    "object '{}' blocker {:?} has no occupied voxel in its column",
                    self.id.as_str(),
                    blocker
                ));
            }
        }
        Ok(blockers)
    }

    fn validate_canopy_mask(&self, occupied: &BTreeSet<LocalVoxelCoord>) -> Result<(), String> {
        let mut canopy = BTreeSet::new();
        for position in &self.canopy_occluders {
            if !self.bounds.contains(*position) {
                return Err(format!(
                    "object '{}' canopy cell {:?} lies outside its authoring bounds",
                    self.id.as_str(),
                    position
                ));
            }
            if !canopy.insert(*position) {
                return Err(format!(
                    "object '{}' repeats canopy cell {:?}",
                    self.id.as_str(),
                    position
                ));
            }
            if !occupied.contains(position) {
                return Err(format!(
                    "object '{}' canopy cell {:?} is not occupied",
                    self.id.as_str(),
                    position
                ));
            }
            let is_foliage = self.placements.iter().any(|placement| {
                placement.position == *position
                    && placement.part == ObjectPart::Plant(PlantPart::Foliage)
            });
            if !is_foliage {
                return Err(format!(
                    "object '{}' canopy cell {:?} is not plant foliage",
                    self.id.as_str(),
                    position
                ));
            }
        }
        Ok(())
    }

    fn validate_plant(
        &self,
        origin_part: ObjectPart,
        blockers: &BTreeSet<LocalAxialCoord>,
    ) -> Result<(), String> {
        if self.connectivity != ConnectivityPolicy::Grounded {
            return Err(format!(
                "plant '{}' must use Grounded connectivity",
                self.id.as_str()
            ));
        }
        if self.bounds.min_level != 0 {
            return Err(format!(
                "plant '{}' bounds must begin at level 0",
                self.id.as_str()
            ));
        }
        if self.origin.level != 0 || origin_part != ObjectPart::Plant(PlantPart::Root) {
            return Err(format!(
                "plant '{}' origin must be an occupied Root at level 0",
                self.id.as_str()
            ));
        }

        let mut roots = BTreeSet::new();
        for placement in &self.placements {
            if placement.part == ObjectPart::Plant(PlantPart::Root) {
                if placement.position.level != 0 {
                    return Err(format!(
                        "plant '{}' root {:?} must be at level 0",
                        self.id.as_str(),
                        placement.position
                    ));
                }
                roots.insert(placement.position.axial());
            }
        }
        if roots != *blockers {
            return Err(format!(
                "plant '{}' blocker footprint must exactly match its level-0 roots",
                self.id.as_str()
            ));
        }
        Ok(())
    }

    fn validate_effect(&self, origin_part: ObjectPart) -> Result<(), String> {
        if self.connectivity != ConnectivityPolicy::Free {
            return Err(format!(
                "effect '{}' must use Free connectivity",
                self.id.as_str()
            ));
        }
        if origin_part != ObjectPart::Effect(EffectPart::Core) {
            return Err(format!(
                "effect '{}' origin must be an occupied Core",
                self.id.as_str()
            ));
        }
        if !self.blocker_footprint.is_empty() || !self.canopy_occluders.is_empty() {
            return Err(format!(
                "effect '{}' cannot define blocker or canopy masks",
                self.id.as_str()
            ));
        }
        Ok(())
    }

    fn validate_prop(&self, origin_part: ObjectPart) -> Result<(), String> {
        if origin_part != ObjectPart::Prop(PropPart::Structure) {
            return Err(format!(
                "prop '{}' origin must be occupied Structure",
                self.id.as_str()
            ));
        }
        if !self.canopy_occluders.is_empty() {
            return Err(format!(
                "prop '{}' cannot define canopy cells",
                self.id.as_str()
            ));
        }
        if self.connectivity == ConnectivityPolicy::Grounded
            && (self.bounds.min_level != 0 || self.origin.level != 0)
        {
            return Err(format!(
                "grounded prop '{}' must begin and have its origin at level 0",
                self.id.as_str()
            ));
        }
        Ok(())
    }
}

fn validate_connected(
    id: &str,
    origin: LocalVoxelCoord,
    occupied: &BTreeSet<LocalVoxelCoord>,
) -> Result<(), String> {
    let mut reached = BTreeSet::new();
    let mut frontier = VecDeque::from([origin]);
    while let Some(position) = frontier.pop_front() {
        if !reached.insert(position) {
            continue;
        }
        for neighbour in position.face_neighbours().into_iter().flatten() {
            if occupied.contains(&neighbour) && !reached.contains(&neighbour) {
                frontier.push_back(neighbour);
            }
        }
    }
    if reached.len() != occupied.len() {
        return Err(format!(
            "object '{id}' has {} occupied cells disconnected from its origin",
            occupied.len().saturating_sub(reached.len())
        ));
    }
    Ok(())
}

const fn category_tag(category: ObjectCategory) -> u8 {
    match category {
        ObjectCategory::Plant => 0,
        ObjectCategory::Effect => 1,
        ObjectCategory::Prop => 2,
    }
}

const fn connectivity_tag(connectivity: ConnectivityPolicy) -> u8 {
    match connectivity {
        ConnectivityPolicy::Grounded => 0,
        ConnectivityPolicy::Free => 1,
    }
}

const fn part_tag(part: ObjectPart) -> u8 {
    match part {
        ObjectPart::Plant(PlantPart::Root) => 0,
        ObjectPart::Plant(PlantPart::Trunk) => 1,
        ObjectPart::Plant(PlantPart::Branch) => 2,
        ObjectPart::Plant(PlantPart::Foliage) => 3,
        ObjectPart::Plant(PlantPart::Accent) => 4,
        ObjectPart::Effect(EffectPart::Core) => 5,
        ObjectPart::Effect(EffectPart::Trail) => 6,
        ObjectPart::Effect(EffectPart::Accent) => 7,
        ObjectPart::Prop(PropPart::Structure) => 8,
        ObjectPart::Prop(PropPart::Detail) => 9,
    }
}

/// Small canonical encoder followed by domain-separated FNV-1a.
///
/// This module cannot borrow generator-private hashing, and its contract must not
/// depend on platform `Hash` implementations.
struct FingerprintEncoder {
    bytes: Vec<u8>,
}

impl FingerprintEncoder {
    const fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn i32(&mut self, value: i32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn length(&mut self, value: usize, kind: &str) -> Result<(), String> {
        let length = u64::try_from(value)
            .map_err(|error| format!("{kind} length {value} cannot be fingerprinted: {error}"))?;
        self.bytes.extend_from_slice(&length.to_le_bytes());
        Ok(())
    }

    fn bytes(&mut self, value: &[u8]) -> Result<(), String> {
        self.length(value.len(), "byte sequence")?;
        self.bytes.extend_from_slice(value);
        Ok(())
    }

    fn string(&mut self, value: &str) -> Result<(), String> {
        self.bytes(value.as_bytes())
    }

    fn axial(&mut self, position: LocalAxialCoord) {
        self.i32(position.q);
        self.i32(position.r);
    }

    fn position(&mut self, position: LocalVoxelCoord) {
        self.axial(position.axial());
        self.i32(position.level);
    }

    fn finish(self) -> u64 {
        const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
        const PRIME: u64 = 0x0000_0100_0000_01b3;
        self.bytes.into_iter().fold(OFFSET_BASIS, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(PRIME)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::art_palette::{SwatchId, VoxelStyle, VoxelSurfaceMode};

    fn object_id(value: &str) -> ObjectAssetId {
        ron::from_str(&format!("\"{value}\"")).expect("test object ids are valid")
    }

    fn style_id(value: &str) -> VoxelStyleId {
        ron::from_str(&format!("\"{value}\"")).expect("test style ids are valid")
    }

    fn swatch_id(value: &str) -> SwatchId {
        ron::from_str(&format!("\"{value}\"")).expect("test swatch ids are valid")
    }

    fn style_catalog(ids: &[&str]) -> VoxelStyleCatalog {
        let styles = ids
            .iter()
            .map(|id| {
                let style = VoxelStyle::new(
                    format!("Style {id}"),
                    swatch_id("plant/test"),
                    VoxelSurfaceMode::Opaque,
                    1.0,
                    None,
                )
                .expect("test style values are valid");
                (style_id(id), style)
            })
            .collect::<BTreeMap<_, _>>();
        VoxelStyleCatalog::new(styles).expect("test catalog values are valid")
    }

    fn placement(q: i32, r: i32, level: i32, part: PlantPart) -> ObjectPlacement {
        ObjectPlacement {
            position: LocalVoxelCoord::new(q, r, level),
            style: style_id(match part {
                PlantPart::Root | PlantPart::Trunk | PlantPart::Branch => "plant/wood",
                PlantPart::Foliage | PlantPart::Accent => "plant/leaf",
            }),
            part: ObjectPart::Plant(part),
        }
    }

    fn plant() -> ObjectBlueprint {
        ObjectBlueprint {
            schema_version: OBJECT_BLUEPRINT_SCHEMA_VERSION,
            id: object_id("plants/test-tree"),
            display_name: "Test Tree".to_owned(),
            category: ObjectCategory::Plant,
            bounds: ObjectBounds {
                radius: 2,
                min_level: 0,
                height: 8,
            },
            connectivity: ConnectivityPolicy::Grounded,
            origin: LocalVoxelCoord::new(0, 0, 0),
            placements: vec![
                placement(0, 0, 0, PlantPart::Root),
                placement(0, 0, 1, PlantPart::Trunk),
                placement(0, 0, 2, PlantPart::Branch),
                placement(1, 0, 2, PlantPart::Foliage),
            ],
            blocker_footprint: vec![LocalAxialCoord::new(0, 0)],
            canopy_occluders: vec![LocalVoxelCoord::new(1, 0, 2)],
        }
    }

    fn effect() -> ObjectBlueprint {
        ObjectBlueprint {
            schema_version: OBJECT_BLUEPRINT_SCHEMA_VERSION,
            id: object_id("effects/test-burst"),
            display_name: "Test Burst".to_owned(),
            category: ObjectCategory::Effect,
            bounds: ObjectBounds {
                radius: 2,
                min_level: -3,
                height: 7,
            },
            connectivity: ConnectivityPolicy::Free,
            origin: LocalVoxelCoord::new(0, 0, 0),
            placements: vec![
                ObjectPlacement {
                    position: LocalVoxelCoord::new(0, 0, 0),
                    style: style_id("effect/core"),
                    part: ObjectPart::Effect(EffectPart::Core),
                },
                ObjectPlacement {
                    position: LocalVoxelCoord::new(2, -1, 3),
                    style: style_id("effect/spark"),
                    part: ObjectPart::Effect(EffectPart::Accent),
                },
            ],
            blocker_footprint: Vec::new(),
            canopy_occluders: Vec::new(),
        }
    }

    #[test]
    fn valid_grounded_plant_passes_intrinsic_validation() {
        assert_eq!(plant().validate_intrinsic(), Ok(()));
    }

    #[test]
    fn valid_effect_may_float_and_be_disconnected() {
        assert_eq!(effect().validate_intrinsic(), Ok(()));
    }

    #[test]
    fn bounds_enforce_radius_height_and_signed_levels() {
        let bounds = ObjectBounds {
            radius: MAX_OBJECT_RADIUS,
            min_level: -32,
            height: MAX_OBJECT_HEIGHT,
        };
        assert!(bounds.validate().is_ok());
        assert!(bounds.contains(LocalVoxelCoord::new(12, -12, -32)));
        assert!(bounds.contains(LocalVoxelCoord::new(0, 0, 31)));
        assert!(!bounds.contains(LocalVoxelCoord::new(13, -13, 0)));
        assert!(!bounds.contains(LocalVoxelCoord::new(0, 0, 32)));

        let mut invalid = plant();
        invalid.bounds.radius = MAX_OBJECT_RADIUS.saturating_add(1);
        assert!(invalid.validate_intrinsic().is_err());
        invalid = plant();
        invalid.bounds.height = 0;
        assert!(invalid.validate_intrinsic().is_err());
    }

    #[test]
    fn overlap_and_out_of_bounds_are_rejected() {
        let mut overlapping = plant();
        overlapping
            .placements
            .push(placement(0, 0, 0, PlantPart::Root));
        assert!(overlapping
            .validate_intrinsic()
            .is_err_and(|error| error.contains("overlapping")));

        let mut outside = plant();
        outside
            .placements
            .push(placement(3, 0, 2, PlantPart::Foliage));
        assert!(outside
            .validate_intrinsic()
            .is_err_and(|error| error.contains("outside")));
    }

    #[test]
    fn category_mismatches_and_malformed_origins_are_rejected() {
        let mut wrong_part = plant();
        if let Some(first) = wrong_part.placements.first_mut() {
            first.part = ObjectPart::Prop(PropPart::Structure);
        }
        assert!(wrong_part
            .validate_intrinsic()
            .is_err_and(|error| error.contains("uses")));

        let mut missing_origin = plant();
        missing_origin.origin = LocalVoxelCoord::new(1, -1, 0);
        assert!(missing_origin
            .validate_intrinsic()
            .is_err_and(|error| error.contains("not occupied")));

        let mut wrong_origin_part = plant();
        if let Some(first) = wrong_origin_part.placements.first_mut() {
            first.part = ObjectPart::Plant(PlantPart::Trunk);
        }
        assert!(wrong_origin_part
            .validate_intrinsic()
            .is_err_and(|error| error.contains("Root")));
    }

    #[test]
    fn plant_roots_exactly_define_blockers() {
        let mut missing = plant();
        missing.blocker_footprint.clear();
        assert!(missing
            .validate_intrinsic()
            .is_err_and(|error| error.contains("exactly match")));

        let mut elevated = plant();
        if let Some(root) = elevated.placements.first_mut() {
            root.position.level = 3;
        }
        elevated.origin.level = 3;
        assert!(elevated
            .validate_intrinsic()
            .is_err_and(|error| error.contains("level 0")));
    }

    #[test]
    fn grounded_geometry_must_be_face_connected() {
        let mut disconnected = plant();
        disconnected
            .placements
            .push(placement(2, -2, 7, PlantPart::Foliage));
        assert!(disconnected
            .validate_intrinsic()
            .is_err_and(|error| error.contains("disconnected")));
    }

    #[test]
    fn canopy_cells_must_be_exact_occupied_foliage() {
        let mut unoccupied = plant();
        unoccupied
            .canopy_occluders
            .push(LocalVoxelCoord::new(-1, 0, 2));
        assert!(unoccupied
            .validate_intrinsic()
            .is_err_and(|error| error.contains("not occupied")));

        let mut trunk = plant();
        trunk.canopy_occluders = vec![LocalVoxelCoord::new(0, 0, 1)];
        assert!(trunk
            .validate_intrinsic()
            .is_err_and(|error| error.contains("not plant foliage")));
    }

    #[test]
    fn effect_contract_requires_free_core_and_empty_masks() {
        let mut grounded = effect();
        grounded.connectivity = ConnectivityPolicy::Grounded;
        assert!(grounded.validate_intrinsic().is_err());

        let mut masked = effect();
        masked.blocker_footprint = vec![LocalAxialCoord::new(0, 0)];
        assert!(masked
            .validate_intrinsic()
            .is_err_and(|error| error.contains("cannot define")));
    }

    #[test]
    fn props_choose_grounded_or_free_connectivity() {
        let base = ObjectPlacement {
            position: LocalVoxelCoord::new(0, 0, 0),
            style: style_id("prop/stone"),
            part: ObjectPart::Prop(PropPart::Structure),
        };
        let floating = ObjectPlacement {
            position: LocalVoxelCoord::new(2, -2, 3),
            style: style_id("prop/stone"),
            part: ObjectPart::Prop(PropPart::Detail),
        };
        let mut prop = ObjectBlueprint {
            schema_version: OBJECT_BLUEPRINT_SCHEMA_VERSION,
            id: object_id("props/test"),
            display_name: "Test Prop".to_owned(),
            category: ObjectCategory::Prop,
            bounds: ObjectBounds {
                radius: 2,
                min_level: 0,
                height: 4,
            },
            connectivity: ConnectivityPolicy::Grounded,
            origin: LocalVoxelCoord::new(0, 0, 0),
            placements: vec![base, floating],
            blocker_footprint: vec![LocalAxialCoord::new(0, 0)],
            canopy_occluders: Vec::new(),
        };
        assert!(prop.validate_intrinsic().is_err());
        prop.connectivity = ConnectivityPolicy::Free;
        assert_eq!(prop.validate_intrinsic(), Ok(()));
    }

    #[test]
    fn dependency_validation_reports_missing_styles() {
        let blueprint = plant();
        assert!(blueprint
            .validate_style_dependencies(|style| style.as_str() == "plant/wood")
            .is_err_and(|error| error.contains("plant/leaf")));
        assert_eq!(blueprint.validate_style_dependencies(|_| true), Ok(()));

        let incomplete = style_catalog(&["plant/wood"]);
        assert!(blueprint
            .validate(&incomplete)
            .is_err_and(|error| error.contains("plant/leaf")));
        let complete = style_catalog(&["plant/wood", "plant/leaf"]);
        assert_eq!(blueprint.validate(&complete), Ok(()));
    }

    #[test]
    fn voxel_cap_is_enforced() {
        let mut oversized = effect();
        oversized.placements = (0..=MAX_OBJECT_VOXELS)
            .map(|index| {
                let level = i32::try_from(index).unwrap_or(i32::MAX);
                ObjectPlacement {
                    position: LocalVoxelCoord::new(0, 0, level),
                    style: style_id("effect/core"),
                    part: ObjectPart::Effect(EffectPart::Core),
                }
            })
            .collect();
        assert!(oversized
            .validate_intrinsic()
            .is_err_and(|error| error.contains("maximum")));
    }

    #[test]
    fn fingerprint_ignores_collection_order_but_covers_semantics() {
        let first = plant();
        assert_eq!(first.semantic_fingerprint(), Ok(1_424_557_119_057_464_755));
        let mut reordered = first.clone();
        reordered.placements.reverse();
        reordered.blocker_footprint.reverse();
        reordered.canopy_occluders.reverse();
        assert_eq!(
            first.semantic_fingerprint(),
            reordered.semantic_fingerprint()
        );
        assert_eq!(
            ron::to_string(&first),
            ron::to_string(&reordered),
            "serialization must use canonical collection order"
        );

        let mut changed = first.clone();
        changed.display_name = "Changed".to_owned();
        assert_ne!(first.semantic_fingerprint(), changed.semantic_fingerprint());
    }

    #[test]
    fn serialization_round_trip_and_parse_time_validation() {
        let original = plant();
        let encoded = ron::to_string(&original).expect("valid blueprint serializes");
        let decoded: ObjectBlueprint =
            ron::from_str(&encoded).expect("serialized blueprint parses and validates");
        assert_eq!(decoded, original);

        let invalid = encoded.replace("schema_version:1", "schema_version:99");
        assert!(ron::from_str::<ObjectBlueprint>(&invalid).is_err());
    }

    #[test]
    fn six_rotations_restore_exact_axial_coordinates() {
        let original = LocalVoxelCoord::new(4, -2, 7);
        let pivot = LocalVoxelCoord::new(1, 1, -20);
        let mut rotated = original;
        for _ in 0..6 {
            rotated = rotated
                .rotated_clockwise_60(pivot)
                .expect("small editor coordinates cannot overflow");
        }
        assert_eq!(rotated, original);
    }
}
