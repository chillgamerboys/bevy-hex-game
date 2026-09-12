//! Strict, versioned, renderer-independent schematic contracts.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Current authoritative template and plan schema.
pub const SCHEMATIC_SCHEMA_VERSION: u16 = 1;
/// Fixed planning radius. This phase never chooses a runtime voxel radius.
pub const SCHEMATIC_RADIUS: u8 = 8;
/// Exact number of cells in a complete radius-eight hexagon.
pub const SCHEMATIC_CELL_COUNT: usize = 217;

const EXPECTED_CANDIDATE_ATTEMPTS: u8 = 32;
const MAX_STABLE_ID_BYTES: usize = 128;

/// A structural contract error, separate from generator policy failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelError {
    message: String,
}

impl ModelError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Human-readable failure detail.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ModelError {}

/// A durable lowercase path-like identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct StableId(String);

impl StableId {
    /// Validates and constructs an identifier.
    pub fn new(value: impl Into<String>) -> Result<Self, ModelError> {
        let value = value.into();
        validate_stable_id(&value)?;
        Ok(Self(value))
    }

    /// Exact wire value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StableId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl AsRef<str> for StableId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl FromStr for StableId {
    type Err = ModelError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for StableId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

fn validate_stable_id(value: &str) -> Result<(), ModelError> {
    if value.is_empty() || value.len() > MAX_STABLE_ID_BYTES {
        return Err(ModelError::new(format!(
            "stable identifier {value:?} must contain 1..={MAX_STABLE_ID_BYTES} bytes"
        )));
    }
    for segment in value.split('/') {
        let mut characters = segment.chars();
        let Some(first) = characters.next() else {
            return Err(ModelError::new(format!(
                "stable identifier {value:?} contains an empty segment"
            )));
        };
        if !first.is_ascii_lowercase()
            || !characters.all(|character| {
                character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
            })
        {
            return Err(ModelError::new(format!(
                "stable identifier {value:?} must use lowercase kebab-case '/'-separated segments"
            )));
        }
    }
    Ok(())
}

/// Strict cube coordinate serialized with all three axes.
///
/// Construction and deserialization reject any triple whose sum is not zero.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SchematicCoord {
    q: i32,
    r: i32,
    s: i32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSchematicCoord {
    q: i32,
    r: i32,
    s: i32,
}

impl SchematicCoord {
    /// Constructs a valid cube coordinate.
    pub fn new(q: i32, r: i32, s: i32) -> Result<Self, ModelError> {
        if i64::from(q) + i64::from(r) + i64::from(s) != 0 {
            return Err(ModelError::new(format!(
                "schematic cube coordinate ({q}, {r}, {s}) must sum to zero"
            )));
        }
        Ok(Self { q, r, s })
    }

    /// Constructs a coordinate from its axial pair.
    pub fn from_axial(q: i32, r: i32) -> Result<Self, ModelError> {
        let s = i64::from(q)
            .checked_add(i64::from(r))
            .and_then(i64::checked_neg)
            .and_then(|value| i32::try_from(value).ok())
            .ok_or_else(|| ModelError::new("schematic axial coordinate overflows cube space"))?;
        Ok(Self { q, r, s })
    }

    /// Cube q coordinate.
    #[must_use]
    pub const fn q(self) -> i32 {
        self.q
    }

    /// Cube r coordinate.
    #[must_use]
    pub const fn r(self) -> i32 {
        self.r
    }

    /// Cube s coordinate.
    #[must_use]
    pub const fn s(self) -> i32 {
        self.s
    }

    /// Hex distance to another coordinate, or `None` on malicious overflow.
    #[must_use]
    pub fn checked_distance(self, other: Self) -> Option<u32> {
        let q = i64::from(self.q).checked_sub(i64::from(other.q))?.abs();
        let r = i64::from(self.r).checked_sub(i64::from(other.r))?.abs();
        let s = i64::from(self.s).checked_sub(i64::from(other.s))?.abs();
        u32::try_from(q.max(r).max(s)).ok()
    }

    /// Six adjacent coordinates in clockwise order, starting east.
    #[must_use]
    pub fn neighbors(self) -> Option<[Self; 6]> {
        let deltas = [
            (1, 0, -1),
            (0, 1, -1),
            (-1, 1, 0),
            (-1, 0, 1),
            (0, -1, 1),
            (1, -1, 0),
        ];
        let mut result = [Self::default(); 6];
        for (slot, (dq, dr, ds)) in result.iter_mut().zip(deltas) {
            *slot = Self {
                q: self.q.checked_add(dq)?,
                r: self.r.checked_add(dr)?,
                s: self.s.checked_add(ds)?,
            };
        }
        Some(result)
    }
}

impl Serialize for SchematicCoord {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        RawSchematicCoord {
            q: self.q,
            r: self.r,
            s: self.s,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SchematicCoord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawSchematicCoord::deserialize(deserializer)?;
        Self::new(raw.q, raw.r, raw.s).map_err(serde::de::Error::custom)
    }
}

/// Stable canonical cell identity (`0..217`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct CellId(u16);

impl CellId {
    /// Constructs a valid cell identity.
    pub fn new(value: u16) -> Result<Self, ModelError> {
        if usize::from(value) >= SCHEMATIC_CELL_COUNT {
            return Err(ModelError::new(format!(
                "cell id {value} lies outside 0..{SCHEMATIC_CELL_COUNT}"
            )));
        }
        Ok(Self(value))
    }

    /// Zero-based canonical ordinal.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

impl<'de> Deserialize<'de> for CellId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = u16::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Returns all cells centre-first, then each ring from its north-east corner clockwise.
#[must_use]
pub fn canonical_coordinates() -> Vec<SchematicCoord> {
    let mut cells = Vec::with_capacity(SCHEMATIC_CELL_COUNT);
    cells.push(SchematicCoord::default());
    let directions = [
        (0, 1, -1),
        (-1, 1, 0),
        (-1, 0, 1),
        (0, -1, 1),
        (1, -1, 0),
        (1, 0, -1),
    ];
    for radius in 1..=i32::from(SCHEMATIC_RADIUS) {
        let mut cell = SchematicCoord {
            q: radius,
            r: -radius,
            s: 0,
        };
        for (dq, dr, ds) in directions {
            for _ in 0..radius {
                cells.push(cell);
                cell = SchematicCoord {
                    q: cell.q + dq,
                    r: cell.r + dr,
                    s: cell.s + ds,
                };
            }
        }
    }
    cells
}

/// Constant-time canonical identity lookup for a radius-eight coordinate.
#[must_use]
pub fn canonical_cell_id(coord: SchematicCoord) -> Option<CellId> {
    let unsigned_radius = coord
        .q
        .unsigned_abs()
        .max(coord.r.unsigned_abs())
        .max(coord.s.unsigned_abs());
    if unsigned_radius > u32::from(SCHEMATIC_RADIUS) {
        return None;
    }
    let radius = i32::try_from(unsigned_radius).ok()?;
    if radius == 0 {
        return CellId::new(0).ok();
    }
    let (segment, offset) = if coord.q == radius && (-radius..0).contains(&coord.r) {
        (0, coord.r + radius)
    } else if coord.q > 0 && coord.r >= 0 && coord.q + coord.r == radius {
        (1, coord.r)
    } else if coord.r == radius && (-radius < coord.q && coord.q <= 0) {
        (2, -coord.q)
    } else if coord.q == -radius && (0 < coord.r && coord.r <= radius) {
        (3, radius - coord.r)
    } else if coord.q < 0 && coord.r <= 0 && coord.q + coord.r == -radius {
        (4, -coord.r)
    } else if coord.r == -radius && (0 <= coord.q && coord.q < radius) {
        (5, coord.q)
    } else {
        return None;
    };
    let index = 1_i32 + 3 * radius * (radius - 1) + segment * radius + offset;
    u16::try_from(index)
        .ok()
        .and_then(|value| CellId::new(value).ok())
}

/// Compatibility spelling for callers interested only in the ordinal.
#[must_use]
pub fn canonical_coordinate_index(coord: SchematicCoord) -> Option<usize> {
    canonical_cell_id(coord).map(|id| usize::from(id.get()))
}

/// Whether the cell is solid land or open surface water.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SurfaceKind {
    /// Solid land.
    Land,
    /// Open ocean, river, or lake surface.
    OpenWater,
}

/// Coarse shape intent without voxel dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LandformKind {
    /// No landform beneath open water.
    None,
    /// A disconnected scenic island.
    Island,
    /// Sandy coastal beach.
    Beach,
    /// Rocky or vegetated shoreline.
    Shore,
    /// Low valley floor.
    Valley,
    /// Broad raised plateau.
    Plateau,
    /// Rounded hill country.
    Hill,
    /// Mountain terrain outside the central mass.
    Mountain,
    /// Connected central mountain mass.
    Massif,
    /// One sharp summit cell.
    SharpPeak,
}

/// Broad climate intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ClimateKind {
    /// Mild ocean-influenced climate.
    Marine,
    /// Temperate inland climate.
    Temperate,
    /// Cold high-elevation climate.
    Alpine,
    /// Persistently frozen climate.
    Frozen,
}

/// Relative vegetation coverage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum VegetationDensity {
    /// No vegetation.
    None,
    /// Scattered plants.
    Sparse,
    /// Open cover.
    Light,
    /// Substantial cover with traversable gaps.
    Moderate,
    /// Dense woodland.
    Dense,
}

/// Designer intent for ordinary traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AccessIntent {
    /// Required ordinary-route terrain.
    Ordinary,
    /// Optional scenic terrain without a required ordinary connection.
    Scenic,
    /// Deliberately inaccessible terrain.
    Inaccessible,
}

/// Canonically ordered semantic overlays on a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum FeatureKind {
    /// Reference coastline marker.
    Coastline,
    /// Surface river.
    River,
    /// Falling-water segment.
    Waterfall,
    /// Variable valley lake.
    ValleyLake,
    /// Fixed elevated mountain lake.
    MountainLake,
    /// Island inside the elevated lake.
    LakeIsland,
    /// Exact frozen woodland landmark.
    FrozenWoods,
    /// Exact authored peak-enclosure cells. The Grand V3 trace uses two
    /// six-cell chains rather than a generic generated ring.
    PeakRing,
    /// Crystal Ascent landmark.
    CrystalAscent,
    /// Complete underground tunnel route.
    Tunnel,
    /// Generated scenic sea-island membership.
    SeaIsland,
}

/// Independent provenance for one cell layer or overlay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum LayerProvenance {
    /// Immutable authored claim.
    Locked {
        /// Claim that owns the fact.
        claim: StableId,
    },
    /// Reference value governed by a bounded rule.
    Bounded {
        /// Rule that owns the allowed variation.
        rule: StableId,
    },
    /// Value selected by a named deterministic stream.
    Seeded {
        /// Independent stream identity.
        stream: StableId,
    },
    /// Value copied by the separately validated fallback.
    ReferenceFallback {
        /// Claim, bounded rule, or named stream from which the reference value
        /// was copied.
        source: StableId,
    },
}

/// Complete resolved cell facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellFacts {
    /// Land or open water.
    pub surface: SurfaceKind,
    /// Coarse landform.
    pub landform: LandformKind,
    /// Broad climate.
    pub climate: ClimateKind,
    /// Vegetation coverage.
    pub vegetation: VegetationDensity,
    /// Traversal intent.
    pub access: AccessIntent,
    /// Unique overlays in [`FeatureKind`] order.
    pub overlays: Vec<FeatureKind>,
}

/// Provenance for one overlay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayProvenance {
    /// Overlay whose provenance is described.
    pub feature: FeatureKind,
    /// Source of the overlay fact.
    pub source: LayerProvenance,
}

/// Per-layer provenance for one cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellProvenance {
    /// Surface source.
    pub surface: LayerProvenance,
    /// Landform source.
    pub landform: LayerProvenance,
    /// Climate source.
    pub climate: LayerProvenance,
    /// Vegetation source.
    pub vegetation: LayerProvenance,
    /// Access source.
    pub access: LayerProvenance,
    /// One source per overlay, in matching canonical order.
    pub overlays: Vec<OverlayProvenance>,
}

/// One canonical template or generated-plan cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellPlan {
    /// Stable ordinal identity.
    pub id: CellId,
    /// Exact cube coordinate corresponding to `id`.
    pub coord: SchematicCoord,
    /// Layered semantic facts.
    pub facts: CellFacts,
    /// Per-layer and per-overlay provenance.
    pub provenance: CellProvenance,
}

/// Exact layer value selected inside a bounded-region rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum BoundedTarget {
    /// Selects the given surface value.
    Surface(SurfaceKind),
    /// Selects the given landform value.
    Landform(LandformKind),
    /// Selects the given climate value.
    Climate(ClimateKind),
    /// Selects the given vegetation-density value.
    Vegetation(VegetationDensity),
    /// Selects any woodland-bearing vegetation density (`Light`, `Moderate`,
    /// or `Dense`) while allowing the generator to vary density inside one
    /// coherent bounded woodland region.
    Vegetated,
    /// Selects the given traversal intent.
    Access(AccessIntent),
    /// Selects membership in the given overlay.
    Overlay(FeatureKind),
}

/// Purpose of one bounded designer rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum BoundedRegionKind {
    /// Coast movement within its exact envelope.
    Coastline,
    /// Scenic open-sea islands.
    SeaIslands,
    /// Eligible woodland occupancy.
    Woodland,
    /// Variable lowland lake.
    ValleyLake,
    /// Connected central massif footprint.
    Massif,
    /// Another traced region whose count may vary by its declared range.
    TracedRegion,
}

/// Inclusive integer count range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CountRange {
    /// Inclusive minimum.
    pub min: u16,
    /// Inclusive maximum.
    pub max: u16,
}

impl CountRange {
    /// Constructs an ordered inclusive range.
    pub fn new(min: u16, max: u16) -> Result<Self, ModelError> {
        if min > max {
            return Err(ModelError::new(format!(
                "count range minimum {min} exceeds maximum {max}"
            )));
        }
        Ok(Self { min, max })
    }

    /// Membership test.
    #[must_use]
    pub const fn contains(self, value: u16) -> bool {
        self.min <= value && value <= self.max
    }
}

/// Inclusive integer percentage range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PercentRange {
    /// Inclusive minimum.
    pub min: u8,
    /// Inclusive maximum.
    pub max: u8,
}

impl PercentRange {
    /// Constructs an ordered range inside `0..=100`.
    pub fn new(min: u8, max: u8) -> Result<Self, ModelError> {
        if min > max || max > 100 {
            return Err(ModelError::new(format!(
                "percentage range {min}..={max} must be ordered inside 0..=100"
            )));
        }
        Ok(Self { min, max })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPercentRange {
    min: u8,
    max: u8,
}

impl<'de> Deserialize<'de> for PercentRange {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawPercentRange::deserialize(deserializer)?;
        Self::new(raw.min, raw.max).map_err(serde::de::Error::custom)
    }
}

/// One exact reference mask and the envelope in which it may vary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundedRegionRule {
    /// Stable rule identity.
    pub id: StableId,
    /// Rule purpose.
    pub kind: BoundedRegionKind,
    /// Unique target facts in canonical enum/value order.
    pub targets: Vec<BoundedTarget>,
    /// Exact hand-traced reference mask in canonical cell order.
    pub reference_mask: Vec<SchematicCoord>,
    /// Exact allowed cells, including the reference mask, in canonical order.
    pub envelope: Vec<SchematicCoord>,
    /// Maximum graph displacement from the reference mask; coastline uses two.
    pub max_displacement: u8,
    /// Hand-traced reference count.
    pub baseline_count: u16,
    /// Inclusive allowed total-cell count.
    pub count: CountRange,
    /// Inclusive connected-component count.
    pub components: CountRange,
    /// Inclusive component-size range.
    pub component_size: CountRange,
    /// Optional percentage selection range over the envelope.
    pub coverage_percent: Option<PercentRange>,
}

/// Exact cell-set feature claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureClaim {
    /// Stable claim identity.
    pub id: StableId,
    /// Exact overlay kind owned by the claim.
    pub kind: FeatureKind,
    /// Claim provenance.
    pub provenance: LayerProvenance,
    /// Unique cells in canonical order.
    pub cells: Vec<SchematicCoord>,
}

/// Semantic network purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum NetworkKind {
    /// Surface-water flow.
    Hydrology,
    /// Complete underground tunnel route.
    Tunnel,
}

/// Role of one exact network node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum NetworkNodeKind {
    /// Network source.
    Source,
    /// Intermediate junction or landmark.
    Junction,
    /// Network destination.
    Sink,
}

/// One exact named network node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkNode {
    /// Stable node identity.
    pub id: StableId,
    /// Node role.
    pub kind: NetworkNodeKind,
    /// Exact cell.
    pub coord: SchematicCoord,
}

/// One directed edge with its complete ordered path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkEdge {
    /// Stable edge identity.
    pub id: StableId,
    /// Existing source node.
    pub from: StableId,
    /// Existing destination node.
    pub to: StableId,
    /// Adjacent cells including both endpoints.
    pub path: Vec<SchematicCoord>,
}

/// One complete renderer-independent network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Network {
    /// Stable network identity.
    pub id: StableId,
    /// Network purpose.
    pub kind: NetworkKind,
    /// Nodes in stable-id order.
    pub nodes: Vec<NetworkNode>,
    /// Edges in stable-id order.
    pub edges: Vec<NetworkEdge>,
}

/// Candidate and independent-stream settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationSettings {
    /// Exactly 32 hard-valid candidate attempts.
    pub candidate_attempts: u8,
    /// Unique deterministic streams in stable-id order.
    pub named_streams: Vec<StableId>,
}

/// Whole-plan candidate, fallback, or reference-artifact provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanProvenance {
    /// Requested world seed.
    pub world_seed: u64,
    /// Exact number of candidate ordinals evaluated; zero for a reference artifact.
    pub candidates_evaluated: u8,
    /// Exact number of those candidates which passed hard validation.
    pub hard_valid_candidates: u8,
    /// Selected candidate, or none for a reference copy.
    pub selected_candidate: Option<u8>,
    /// Explicit fallback marker.
    pub used_reference_fallback: bool,
    /// Explicit marker for a directly requested, non-selected reference artifact.
    pub is_reference_artifact: bool,
}

impl PlanProvenance {
    /// Constructs checked normal-candidate provenance.
    pub fn candidate(
        world_seed: u64,
        selected_candidate: u8,
        hard_valid_candidates: u8,
    ) -> Result<Self, ModelError> {
        if selected_candidate >= EXPECTED_CANDIDATE_ATTEMPTS {
            return Err(ModelError::new(format!(
                "candidate {selected_candidate} must be below {EXPECTED_CANDIDATE_ATTEMPTS}"
            )));
        }
        if !(1..=EXPECTED_CANDIDATE_ATTEMPTS).contains(&hard_valid_candidates) {
            return Err(ModelError::new(format!(
                "hard_valid_candidates {hard_valid_candidates} must lie inside 1..={EXPECTED_CANDIDATE_ATTEMPTS}"
            )));
        }
        Ok(Self {
            world_seed,
            candidates_evaluated: EXPECTED_CANDIDATE_ATTEMPTS,
            hard_valid_candidates,
            selected_candidate: Some(selected_candidate),
            used_reference_fallback: false,
            is_reference_artifact: false,
        })
    }

    /// Constructs reference-fallback provenance.
    #[must_use]
    pub const fn reference_fallback(world_seed: u64) -> Self {
        Self {
            world_seed,
            candidates_evaluated: EXPECTED_CANDIDATE_ATTEMPTS,
            hard_valid_candidates: 0,
            selected_candidate: None,
            used_reference_fallback: true,
            is_reference_artifact: false,
        }
    }

    /// Constructs provenance for a directly requested reference artifact.
    ///
    /// Unlike a fallback selected by normal generation, this artifact evaluates
    /// no candidate ordinals and makes no claim that normal generation failed.
    #[must_use]
    pub const fn reference_artifact(world_seed: u64) -> Self {
        Self {
            world_seed,
            candidates_evaluated: 0,
            hard_valid_candidates: 0,
            selected_candidate: None,
            used_reference_fallback: false,
            is_reference_artifact: true,
        }
    }

    fn validate(self) -> Result<(), ModelError> {
        match (
            self.candidates_evaluated,
            self.selected_candidate,
            self.used_reference_fallback,
            self.is_reference_artifact,
            self.hard_valid_candidates,
        ) {
            (
                EXPECTED_CANDIDATE_ATTEMPTS,
                Some(candidate),
                false,
                false,
                1..=EXPECTED_CANDIDATE_ATTEMPTS,
            )
                if candidate < EXPECTED_CANDIDATE_ATTEMPTS =>
            {
                Ok(())
            }
            (EXPECTED_CANDIDATE_ATTEMPTS, None, true, false, 0)
            | (0, None, false, true, 0) => Ok(()),
            _ => Err(ModelError::new(
                "candidate selection, fallback/reference marker, evaluated count, and hard-valid count disagree",
            )),
        }
    }
}

/// Strict version-one designer template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SchematicTemplateV1 {
    /// Must equal one.
    pub schema_version: u16,
    /// Stable template identity.
    pub id: StableId,
    /// Positive authored revision.
    pub revision: u32,
    /// Must equal eight.
    pub radius: u8,
    /// Exactly 217 reference cells in canonical order.
    pub reference_cells: Vec<CellPlan>,
    /// Exact immutable feature claims in stable-id order.
    pub fixed_claims: Vec<FeatureClaim>,
    /// Bounded generator rules in stable-id order.
    pub bounded_regions: Vec<BoundedRegionRule>,
    /// Exact hydrology and tunnel declarations in stable-id order.
    pub networks: Vec<Network>,
    /// Candidate and named-stream settings.
    pub generation: GenerationSettings,
}

/// Compatibility alias for callers that do not need to spell the wire version.
pub type SchematicTemplate = SchematicTemplateV1;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSchematicTemplateV1 {
    schema_version: u16,
    id: StableId,
    revision: u32,
    radius: u8,
    reference_cells: Vec<CellPlan>,
    fixed_claims: Vec<FeatureClaim>,
    bounded_regions: Vec<BoundedRegionRule>,
    networks: Vec<Network>,
    generation: GenerationSettings,
}

impl<'de> Deserialize<'de> for SchematicTemplateV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawSchematicTemplateV1::deserialize(deserializer)?;
        let value = Self {
            schema_version: raw.schema_version,
            id: raw.id,
            revision: raw.revision,
            radius: raw.radius,
            reference_cells: raw.reference_cells,
            fixed_claims: raw.fixed_claims,
            bounded_regions: raw.bounded_regions,
            networks: raw.networks,
            generation: raw.generation,
        };
        value
            .validate_structure()
            .map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}

impl SchematicTemplateV1 {
    /// Validates version, canonical coverage, and duplicate stable identities.
    pub fn validate_structure(&self) -> Result<(), ModelError> {
        validate_header(self.schema_version, self.radius, self.revision)?;
        validate_cells(&self.reference_cells)?;
        validate_named_order(self.fixed_claims.iter().map(|claim| &claim.id), "claim")?;
        validate_feature_claim_order(&self.fixed_claims)?;
        validate_named_order(self.bounded_regions.iter().map(|rule| &rule.id), "rule")?;
        validate_bounded_region_order(&self.bounded_regions)?;
        validate_named_order(self.networks.iter().map(|network| &network.id), "network")?;
        validate_network_identifiers(&self.networks)?;
        validate_named_order(self.generation.named_streams.iter(), "stream")?;
        if self.generation.candidate_attempts != EXPECTED_CANDIDATE_ATTEMPTS {
            return Err(ModelError::new("candidate_attempts must equal 32"));
        }
        Ok(())
    }

    /// Constant-time reference-cell lookup.
    #[must_use]
    pub fn cell(&self, coord: SchematicCoord) -> Option<&CellPlan> {
        canonical_coordinate_index(coord).and_then(|index| self.reference_cells.get(index))
    }
}

/// Checked constructor payload for an authoritative plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchematicPlanParts {
    /// Source template identity.
    pub template_id: StableId,
    /// Source template revision.
    pub template_revision: u32,
    /// Candidate, fallback, or reference-artifact provenance.
    pub provenance: PlanProvenance,
    /// Exactly 217 canonical cells.
    pub cells: Vec<CellPlan>,
    /// Resolved claims in stable-id order.
    pub features: Vec<FeatureClaim>,
    /// Resolved networks in stable-id order.
    pub networks: Vec<Network>,
    /// Required deterministic semantic fingerprint.
    pub semantic_fingerprint: u64,
}

/// Strict version-one authoritative generated plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SchematicPlanV1 {
    /// Must equal one.
    pub schema_version: u16,
    /// Source template identity.
    pub template_id: StableId,
    /// Positive source template revision.
    pub template_revision: u32,
    /// Must equal eight.
    pub radius: u8,
    /// Candidate, fallback, or reference-artifact provenance.
    pub provenance: PlanProvenance,
    /// Exactly 217 canonical cells.
    pub cells: Vec<CellPlan>,
    /// Resolved claims in stable-id order.
    pub features: Vec<FeatureClaim>,
    /// Resolved networks in stable-id order.
    pub networks: Vec<Network>,
    /// Required deterministic semantic fingerprint.
    pub semantic_fingerprint: u64,
}

/// Compatibility alias for callers that do not need to spell the wire version.
pub type SchematicPlan = SchematicPlanV1;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSchematicPlanV1 {
    schema_version: u16,
    template_id: StableId,
    template_revision: u32,
    radius: u8,
    provenance: PlanProvenance,
    cells: Vec<CellPlan>,
    features: Vec<FeatureClaim>,
    networks: Vec<Network>,
    semantic_fingerprint: u64,
}

impl<'de> Deserialize<'de> for SchematicPlanV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawSchematicPlanV1::deserialize(deserializer)?;
        let value = Self {
            schema_version: raw.schema_version,
            template_id: raw.template_id,
            template_revision: raw.template_revision,
            radius: raw.radius,
            provenance: raw.provenance,
            cells: raw.cells,
            features: raw.features,
            networks: raw.networks,
            semantic_fingerprint: raw.semantic_fingerprint,
        };
        value
            .validate_structure()
            .map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}

impl SchematicPlanV1 {
    /// Constructs a structurally checked current-version plan.
    pub fn new(parts: SchematicPlanParts) -> Result<Self, ModelError> {
        let value = Self {
            schema_version: SCHEMATIC_SCHEMA_VERSION,
            template_id: parts.template_id,
            template_revision: parts.template_revision,
            radius: SCHEMATIC_RADIUS,
            provenance: parts.provenance,
            cells: parts.cells,
            features: parts.features,
            networks: parts.networks,
            semantic_fingerprint: parts.semantic_fingerprint,
        };
        value.validate_structure()?;
        Ok(value)
    }

    /// Validates strict wire structure without duplicating generator policy.
    pub fn validate_structure(&self) -> Result<(), ModelError> {
        validate_header(self.schema_version, self.radius, self.template_revision)?;
        self.provenance.validate()?;
        validate_cells(&self.cells)?;
        validate_named_order(self.features.iter().map(|claim| &claim.id), "claim")?;
        validate_feature_claim_order(&self.features)?;
        validate_named_order(self.networks.iter().map(|network| &network.id), "network")?;
        validate_network_identifiers(&self.networks)?;
        Ok(())
    }

    /// Constant-time plan-cell lookup.
    #[must_use]
    pub fn cell(&self, coord: SchematicCoord) -> Option<&CellPlan> {
        canonical_coordinate_index(coord).and_then(|index| self.cells.get(index))
    }

    /// Recovers the checked-constructor payload.
    #[must_use]
    pub fn into_parts(self) -> SchematicPlanParts {
        SchematicPlanParts {
            template_id: self.template_id,
            template_revision: self.template_revision,
            provenance: self.provenance,
            cells: self.cells,
            features: self.features,
            networks: self.networks,
            semantic_fingerprint: self.semantic_fingerprint,
        }
    }
}

fn validate_header(version: u16, radius: u8, revision: u32) -> Result<(), ModelError> {
    if version != SCHEMATIC_SCHEMA_VERSION || radius != SCHEMATIC_RADIUS || revision == 0 {
        return Err(ModelError::new(format!(
            "schematic header must use schema {SCHEMATIC_SCHEMA_VERSION}, radius {SCHEMATIC_RADIUS}, and a positive revision"
        )));
    }
    Ok(())
}

fn validate_cells(cells: &[CellPlan]) -> Result<(), ModelError> {
    if cells.len() != SCHEMATIC_CELL_COUNT {
        return Err(ModelError::new(format!(
            "schematic contains {} cells; expected {SCHEMATIC_CELL_COUNT}",
            cells.len()
        )));
    }
    for (index, (cell, coord)) in cells.iter().zip(canonical_coordinates()).enumerate() {
        if usize::from(cell.id.get()) != index || cell.coord != coord {
            return Err(ModelError::new(format!(
                "cell {index} does not match its canonical id and coordinate"
            )));
        }
        validate_order(&cell.facts.overlays, "cell overlays")?;
        let overlay_kinds = cell
            .provenance
            .overlays
            .iter()
            .map(|overlay| overlay.feature)
            .collect::<Vec<_>>();
        if overlay_kinds != cell.facts.overlays {
            return Err(ModelError::new(format!(
                "cell {index} overlay provenance does not match its overlays"
            )));
        }
    }
    Ok(())
}

fn validate_named_order<'a>(
    ids: impl IntoIterator<Item = &'a StableId>,
    kind: &str,
) -> Result<(), ModelError> {
    let mut previous: Option<&str> = None;
    for id in ids {
        if previous.is_some_and(|prior| prior >= id.as_str()) {
            return Err(ModelError::new(format!(
                "{kind} identifiers must be unique and lexicographically ordered"
            )));
        }
        previous = Some(id.as_str());
    }
    Ok(())
}

fn validate_network_identifiers(networks: &[Network]) -> Result<(), ModelError> {
    for network in networks {
        validate_named_order(network.nodes.iter().map(|node| &node.id), "network node")?;
        validate_named_order(network.edges.iter().map(|edge| &edge.id), "network edge")?;
    }
    Ok(())
}

fn validate_feature_claim_order(claims: &[FeatureClaim]) -> Result<(), ModelError> {
    for claim in claims {
        validate_coordinate_order(&claim.cells, "claim cells")?;
    }
    Ok(())
}

fn validate_bounded_region_order(rules: &[BoundedRegionRule]) -> Result<(), ModelError> {
    for rule in rules {
        validate_order(&rule.targets, "bounded targets")?;
        validate_coordinate_order(&rule.reference_mask, "bounded reference mask")?;
        validate_coordinate_order(&rule.envelope, "bounded envelope")?;
        if let Some(range) = rule.coverage_percent {
            PercentRange::new(range.min, range.max)?;
            if rule.kind != BoundedRegionKind::Woodland {
                return Err(ModelError::new(format!(
                    "bounded rule {} may declare coverage_percent only for Woodland",
                    rule.id
                )));
            }
        }
    }
    Ok(())
}

fn validate_coordinate_order(values: &[SchematicCoord], kind: &str) -> Result<(), ModelError> {
    let mut previous = None;
    for coord in values {
        let index = canonical_coordinate_index(*coord)
            .ok_or_else(|| ModelError::new(format!("{kind} contains an out-of-grid cell")))?;
        if previous.is_some_and(|prior| prior >= index) {
            return Err(ModelError::new(format!(
                "{kind} must be unique and canonically ordered"
            )));
        }
        previous = Some(index);
    }
    Ok(())
}

fn validate_order<T: Ord>(values: &[T], kind: &str) -> Result<(), ModelError> {
    if values
        .windows(2)
        .any(|pair| pair.first().zip(pair.get(1)).is_some_and(|(a, b)| a >= b))
    {
        return Err(ModelError::new(format!(
            "{kind} must be unique and canonically ordered"
        )));
    }
    Ok(())
}

/// Returns the cells in `reference_mask` plus every radius-eight cell within
/// `max_displacement`, in canonical order.
pub fn bounded_envelope(
    reference_mask: &[SchematicCoord],
    max_displacement: u8,
) -> Result<Vec<SchematicCoord>, ModelError> {
    if reference_mask.is_empty() {
        return Err(ModelError::new("bounded reference mask cannot be empty"));
    }
    let result = canonical_coordinates()
        .into_iter()
        .filter(|candidate| {
            reference_mask.iter().any(|reference| {
                candidate
                    .checked_distance(*reference)
                    .is_some_and(|distance| distance <= u32::from(max_displacement))
            })
        })
        .collect();
    Ok(result)
}

/// Returns floor/ceiling bounds at minus/plus twenty percent of a traced count.
#[must_use]
pub fn traced_twenty_percent_range(baseline: u16) -> CountRange {
    let baseline = u32::from(baseline);
    let min = baseline.saturating_mul(80) / 100;
    let max = baseline.saturating_mul(120).saturating_add(99) / 100;
    CountRange {
        min: u16::try_from(min).unwrap_or(u16::MAX),
        max: u16::try_from(max).unwrap_or(u16::MAX),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_grid_round_trips_ids() {
        let cells = canonical_coordinates();
        assert_eq!(cells.len(), SCHEMATIC_CELL_COUNT);
        assert_eq!(cells.first(), Some(&SchematicCoord::default()));
        assert_eq!(cells.get(1), SchematicCoord::new(1, -1, 0).ok().as_ref());
        for (index, coord) in cells.into_iter().enumerate() {
            assert_eq!(canonical_coordinate_index(coord), Some(index));
        }
    }

    #[test]
    fn malformed_cube_coordinates_and_ids_fail_closed() {
        assert!(ron::from_str::<SchematicCoord>("(q:1,r:1,s:1)").is_err());
        assert!(StableId::new("Bad Id").is_err());
        assert!(CellId::new(217).is_err());

        let extreme = SchematicCoord::new(i32::MIN, i32::MAX, 1)
            .expect("the extreme cube coordinate is mathematically valid");
        assert_eq!(canonical_cell_id(extreme), None);
    }

    #[test]
    fn layer_provenance_rejects_unknown_variant_fields() {
        assert!(ron::from_str::<LayerProvenance>(
            r#"Seeded(stream:"stream/vegetation",unexpected:true)"#,
        )
        .is_err());
    }

    #[test]
    fn plan_provenance_distinguishes_reference_artifacts_from_fallbacks() {
        let candidate = PlanProvenance::candidate(7, 3, 12)
            .expect("a selected hard-valid candidate must construct");
        assert_eq!(candidate.candidates_evaluated, EXPECTED_CANDIDATE_ATTEMPTS);
        assert!(!candidate.used_reference_fallback);
        assert!(!candidate.is_reference_artifact);

        let fallback = PlanProvenance::reference_fallback(7);
        assert_eq!(fallback.candidates_evaluated, EXPECTED_CANDIDATE_ATTEMPTS);
        assert!(fallback.used_reference_fallback);
        assert!(!fallback.is_reference_artifact);

        let artifact = PlanProvenance::reference_artifact(7);
        assert_eq!(artifact.candidates_evaluated, 0);
        assert_eq!(artifact.hard_valid_candidates, 0);
        assert_eq!(artifact.selected_candidate, None);
        assert!(!artifact.used_reference_fallback);
        assert!(artifact.is_reference_artifact);
        assert!(artifact.validate().is_ok());

        let mut contradictory = artifact;
        contradictory.candidates_evaluated = EXPECTED_CANDIDATE_ATTEMPTS;
        assert!(contradictory.validate().is_err());
    }

    #[test]
    fn template_wire_rejects_malformed_or_misplaced_coverage_ranges() {
        assert!(ron::from_str::<PercentRange>("(min:80,max:30)").is_err());
        assert!(ron::from_str::<PercentRange>("(min:30,max:101)").is_err());
        assert!(matches!(
            ron::from_str::<PercentRange>("(min:30,max:80)"),
            Ok(PercentRange { min: 30, max: 80 })
        ));

        let mut reversed = crate::template::grand_v3_reference_template()
            .expect("the packaged template must parse");
        reversed
            .bounded_regions
            .iter_mut()
            .find(|rule| rule.kind == BoundedRegionKind::Woodland)
            .expect("the packaged template must contain a Woodland rule")
            .coverage_percent = Some(PercentRange { min: 80, max: 30 });
        let wire = ron::ser::to_string(&reversed).expect("the malformed value still serializes");
        assert!(ron::from_str::<SchematicTemplateV1>(&wire).is_err());

        let mut overflow = crate::template::grand_v3_reference_template()
            .expect("the packaged template must parse");
        overflow
            .bounded_regions
            .iter_mut()
            .find(|rule| rule.kind == BoundedRegionKind::Woodland)
            .expect("the packaged template must contain a Woodland rule")
            .coverage_percent = Some(PercentRange { min: 30, max: 101 });
        let wire = ron::ser::to_string(&overflow).expect("the malformed value still serializes");
        assert!(ron::from_str::<SchematicTemplateV1>(&wire).is_err());

        let mut misplaced = crate::template::grand_v3_reference_template()
            .expect("the packaged template must parse");
        misplaced
            .bounded_regions
            .iter_mut()
            .find(|rule| rule.kind != BoundedRegionKind::Woodland)
            .expect("the packaged template must contain a non-Woodland rule")
            .coverage_percent = Some(PercentRange { min: 30, max: 80 });
        let wire = ron::ser::to_string(&misplaced).expect("the malformed value still serializes");
        assert!(ron::from_str::<SchematicTemplateV1>(&wire).is_err());
    }

    #[test]
    fn template_wire_rejects_duplicate_nested_identifiers() {
        let mut template = crate::template::grand_v3_reference_template()
            .expect("the packaged template must parse");
        let duplicate = template
            .networks
            .first()
            .and_then(|network| network.nodes.first())
            .expect("the packaged template must contain a network node")
            .id
            .clone();
        template
            .networks
            .first_mut()
            .and_then(|network| network.nodes.get_mut(1))
            .expect("the packaged template must contain a second network node")
            .id = duplicate;

        let wire = ron::ser::to_string(&template).expect("the malformed value still serializes");
        assert!(ron::from_str::<SchematicTemplateV1>(&wire).is_err());
    }

    #[test]
    fn template_wire_rejects_noncanonical_nested_collections() {
        let mut template = crate::template::grand_v3_reference_template()
            .expect("the packaged template must parse");
        let claim = template
            .fixed_claims
            .iter_mut()
            .find(|claim| claim.cells.len() > 1)
            .expect("the packaged template must contain a multi-cell claim");
        claim.cells.swap(0, 1);
        let wire = ron::ser::to_string(&template).expect("the malformed value still serializes");
        assert!(ron::from_str::<SchematicTemplateV1>(&wire).is_err());

        let mut template = crate::template::grand_v3_reference_template()
            .expect("the packaged template must parse");
        let rule = template
            .bounded_regions
            .iter_mut()
            .find(|rule| rule.targets.len() > 1)
            .expect("the packaged template must contain a multi-target rule");
        rule.targets.swap(0, 1);
        let wire = ron::ser::to_string(&template).expect("the malformed value still serializes");
        assert!(ron::from_str::<SchematicTemplateV1>(&wire).is_err());
    }
}
