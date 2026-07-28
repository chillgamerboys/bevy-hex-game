//! Editor-only crash recovery for unsaved Asset Workshop drafts.
//!
//! Recovery data is deliberately separate from production [`ObjectBlueprint`]
//! deserialization. An interrupted authoring session may contain temporarily invalid
//! geometry, while tracked assets must continue to fail closed.

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use atomicwrites::{AllowOverwrite, AtomicFile};
use hex_assets::{
    ArtPalette, LocalAxialCoord, LocalVoxelCoord, ObjectAssetId, ObjectBlueprint, ObjectBounds,
    ObjectCategory, ObjectPart, ObjectPlacement, VoxelStyleCatalog, VoxelStyleId,
    MAX_OBJECT_HEIGHT, MAX_OBJECT_RADIUS, MAX_OBJECT_VOXELS, OBJECT_BLUEPRINT_SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};

use crate::model::{EditorTool, PreviewRig, WorkshopMode};
use crate::project::ProjectRevisionSet;

/// Current editor-only recovery schema.
pub const RECOVERY_SCHEMA_VERSION: u16 = 1;

/// Repository-relative location of the active recovery draft.
pub const RECOVERY_RELATIVE_PATH: &str = ".context/asset-workshop/recovery/workshop-v1.ron";

/// Maximum absolute authored level accepted from an untrusted recovery file.
///
/// Normal canvases are at most 64 levels high. The larger guard leaves room for
/// deliberately offset free effects while preventing extreme coordinates from
/// destabilizing viewport math.
pub const MAX_RECOVERY_LEVEL_ABS: i32 = 4_096;

const MAX_RECOVERY_DISPLAY_NAME_BYTES: usize = 1_024;

/// Whether the recovered object was a calibration scene, unsaved draft, or tracked
/// object at the time of the last recovery write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoveryDocument {
    /// The initial unsaved calibration scene.
    Calibration,
    /// A newly authored object that has not received its first tracked save.
    Unsaved(ObjectAssetId),
    /// An object previously loaded from or saved to the tracked asset tree.
    Saved(ObjectAssetId),
}

/// One object draft without production semantic validation.
///
/// Fields remain private so only the editor recovery bridge can construct a
/// production-shaped value without validation. Deserialization checks resource and
/// coordinate safety but intentionally permits empty occupancy, overlap, missing
/// origins, disconnected cells, and inconsistent masks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RawObjectDraft {
    schema_version: u16,
    id: ObjectAssetId,
    display_name: String,
    category: ObjectCategory,
    bounds: ObjectBounds,
    connectivity: hex_assets::ConnectivityPolicy,
    origin: LocalVoxelCoord,
    placements: Vec<ObjectPlacement>,
    blocker_footprint: Vec<LocalAxialCoord>,
    canopy_occluders: Vec<LocalVoxelCoord>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UncheckedRawObjectDraft {
    schema_version: u16,
    id: ObjectAssetId,
    display_name: String,
    category: ObjectCategory,
    bounds: ObjectBounds,
    connectivity: hex_assets::ConnectivityPolicy,
    origin: LocalVoxelCoord,
    placements: Vec<ObjectPlacement>,
    blocker_footprint: Vec<LocalAxialCoord>,
    canopy_occluders: Vec<LocalVoxelCoord>,
}

impl<'de> Deserialize<'de> for RawObjectDraft {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let unchecked = UncheckedRawObjectDraft::deserialize(deserializer)?;
        let mut draft = Self {
            schema_version: unchecked.schema_version,
            id: unchecked.id,
            display_name: unchecked.display_name,
            category: unchecked.category,
            bounds: unchecked.bounds,
            connectivity: unchecked.connectivity,
            origin: unchecked.origin,
            placements: unchecked.placements,
            blocker_footprint: unchecked.blocker_footprint,
            canopy_occluders: unchecked.canopy_occluders,
        };
        draft.normalize();
        draft
            .validate_recovery_safety()
            .map_err(serde::de::Error::custom)?;
        Ok(draft)
    }
}

impl RawObjectDraft {
    pub(crate) fn from_blueprint(blueprint: &ObjectBlueprint) -> Self {
        let mut draft = Self {
            schema_version: blueprint.schema_version,
            id: blueprint.id.clone(),
            display_name: blueprint.display_name.clone(),
            category: blueprint.category,
            bounds: blueprint.bounds,
            connectivity: blueprint.connectivity,
            origin: blueprint.origin,
            placements: blueprint.placements.clone(),
            blocker_footprint: blueprint.blocker_footprint.clone(),
            canopy_occluders: blueprint.canopy_occluders.clone(),
        };
        draft.normalize();
        draft
    }

    pub(crate) fn into_blueprint(self) -> ObjectBlueprint {
        ObjectBlueprint {
            schema_version: self.schema_version,
            id: self.id,
            display_name: self.display_name,
            category: self.category,
            bounds: self.bounds,
            connectivity: self.connectivity,
            origin: self.origin,
            placements: self.placements,
            blocker_footprint: self.blocker_footprint,
            canopy_occluders: self.canopy_occluders,
        }
    }

    fn normalize(&mut self) {
        self.placements.sort_by(|left, right| {
            left.position
                .cmp(&right.position)
                .then_with(|| left.style.as_str().cmp(right.style.as_str()))
                .then_with(|| left.part.cmp(&right.part))
        });
        self.blocker_footprint.sort_unstable();
        self.canopy_occluders.sort_unstable();
    }

    fn validate_recovery_safety(&self) -> Result<(), RecoveryError> {
        if self.schema_version != OBJECT_BLUEPRINT_SCHEMA_VERSION {
            return Err(RecoveryError::new(
                "validate recovery",
                None,
                format!(
                    "object recovery uses schema version {}; expected {OBJECT_BLUEPRINT_SCHEMA_VERSION}",
                    self.schema_version
                ),
            ));
        }
        if self.display_name.len() > MAX_RECOVERY_DISPLAY_NAME_BYTES {
            return Err(RecoveryError::new(
                "validate recovery",
                None,
                format!("object display name exceeds {MAX_RECOVERY_DISPLAY_NAME_BYTES} bytes"),
            ));
        }
        validate_recovery_bounds(self.bounds)?;
        validate_position_safety("object origin", self.origin)?;
        if self.placements.len() > MAX_OBJECT_VOXELS {
            return Err(RecoveryError::new(
                "validate recovery",
                None,
                format!(
                    "object recovery contains {} placements; maximum is {MAX_OBJECT_VOXELS}",
                    self.placements.len()
                ),
            ));
        }
        if self.blocker_footprint.len() > MAX_OBJECT_VOXELS
            || self.canopy_occluders.len() > MAX_OBJECT_VOXELS
        {
            return Err(RecoveryError::new(
                "validate recovery",
                None,
                format!("object recovery masks may contain at most {MAX_OBJECT_VOXELS} cells"),
            ));
        }
        for placement in &self.placements {
            validate_position_safety("object placement", placement.position)?;
        }
        for blocker in &self.blocker_footprint {
            validate_axial_safety("blocker mask", *blocker)?;
        }
        for canopy in &self.canopy_occluders {
            validate_position_safety("canopy mask", *canopy)?;
        }
        Ok(())
    }
}

/// Recoverable editor state for one open object document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditorRecoveryDraft {
    /// Current potentially invalid object draft.
    pub object: RawObjectDraft,
    /// Last explicit object-save checkpoint, which may be an intentionally invalid
    /// empty baseline for a never-saved document.
    pub saved_object: RawObjectDraft,
    /// Active Workshop mode.
    pub mode: WorkshopMode,
    /// Active object-editing tool.
    pub tool: EditorTool,
    /// Deterministic viewport lighting rig.
    pub preview_rig: PreviewRig,
    /// Active object level.
    pub active_level: i32,
    /// Active reusable style.
    pub active_style: Option<VoxelStyleId>,
    /// Active semantic part.
    pub active_part: ObjectPart,
    /// Selected occupied cells in deterministic order.
    pub selection: Vec<LocalVoxelCoord>,
}

impl EditorRecoveryDraft {
    pub(crate) fn normalize_and_validate(&mut self) -> Result<(), RecoveryError> {
        self.object.normalize();
        self.saved_object.normalize();
        self.object.validate_recovery_safety()?;
        self.saved_object.validate_recovery_safety()?;
        validate_level("active recovery level", self.active_level)?;
        if self.selection.len() > MAX_OBJECT_VOXELS {
            return Err(RecoveryError::new(
                "validate recovery",
                None,
                format!(
                    "object recovery selects {} cells; maximum is {MAX_OBJECT_VOXELS}",
                    self.selection.len()
                ),
            ));
        }
        for position in &self.selection {
            validate_position_safety("object selection", *position)?;
        }
        self.selection.sort_unstable();
        self.selection.dedup();
        Ok(())
    }
}

/// Complete palette, style, and object draft plus their explicit-save checkpoints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryWorkshopDraft {
    /// Current palette draft.
    pub palette: ArtPalette,
    /// Current voxel-style draft.
    pub styles: VoxelStyleCatalog,
    /// Palette at the most recent explicit catalog save.
    pub saved_palette: ArtPalette,
    /// Voxel styles at the most recent explicit catalog save.
    pub saved_styles: VoxelStyleCatalog,
    /// Open object editor state and checkpoint.
    pub editor: EditorRecoveryDraft,
}

impl RecoveryWorkshopDraft {
    pub(crate) fn normalize_and_validate(&mut self) -> Result<(), RecoveryError> {
        self.palette
            .validate()
            .map_err(|error| RecoveryError::new("validate recovery", None, error.to_string()))?;
        self.styles
            .validate(&self.palette)
            .map_err(|error| RecoveryError::new("validate recovery", None, error.to_string()))?;
        self.saved_palette
            .validate()
            .map_err(|error| RecoveryError::new("validate recovery", None, error.to_string()))?;
        self.saved_styles
            .validate(&self.saved_palette)
            .map_err(|error| RecoveryError::new("validate recovery", None, error.to_string()))?;
        self.editor.normalize_and_validate()
    }
}

/// Versioned recovery file stored outside the tracked asset tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryEnvelope {
    /// Editor-only recovery schema version.
    pub schema_version: u16,
    /// Caller-supplied Unix timestamp in milliseconds.
    pub written_unix_ms: u64,
    /// Exact tracked art-source baseline when this recovery was written.
    pub base_revisions: ProjectRevisionSet,
    /// Persistence identity of the open object.
    pub document: RecoveryDocument,
    /// Complete Workshop draft and explicit-save checkpoints.
    pub workshop: RecoveryWorkshopDraft,
}

impl RecoveryEnvelope {
    /// Constructs and validates one deterministic recovery envelope.
    pub fn new(
        written_unix_ms: u64,
        base_revisions: ProjectRevisionSet,
        document: RecoveryDocument,
        mut workshop: RecoveryWorkshopDraft,
    ) -> Result<Self, RecoveryError> {
        workshop.normalize_and_validate()?;
        let envelope = Self {
            schema_version: RECOVERY_SCHEMA_VERSION,
            written_unix_ms,
            base_revisions,
            document,
            workshop,
        };
        envelope.validate()?;
        Ok(envelope)
    }

    /// Checks schema, catalog, resource, and coordinate safety without requiring the
    /// object draft to be production-valid.
    pub fn validate(&self) -> Result<(), RecoveryError> {
        if self.schema_version != RECOVERY_SCHEMA_VERSION {
            return Err(RecoveryError::new(
                "validate recovery",
                None,
                format!(
                    "recovery schema version {} is unsupported; expected {RECOVERY_SCHEMA_VERSION}",
                    self.schema_version
                ),
            ));
        }
        let mut workshop = self.workshop.clone();
        workshop.normalize_and_validate()?;
        let recovered_id = &workshop.editor.object.id;
        let identity_matches = match &self.document {
            RecoveryDocument::Calibration => recovered_id.as_str() == "calibration/scene",
            RecoveryDocument::Unsaved(id) | RecoveryDocument::Saved(id) => id == recovered_id,
        };
        if !identity_matches {
            return Err(RecoveryError::new(
                "validate recovery",
                None,
                format!(
                    "recovery document identity does not match object '{}'",
                    recovered_id.as_str()
                ),
            ));
        }
        Ok(())
    }
}

/// Selection changes made while safely restoring a recovery draft.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecoverySanitization {
    /// Selection entries removed because they no longer name an occupied cell.
    pub discarded_selection_cells: usize,
}

/// Actionable recovery encoding, validation, or filesystem failure.
#[derive(Debug)]
pub struct RecoveryError {
    operation: &'static str,
    path: Option<PathBuf>,
    detail: String,
}

impl RecoveryError {
    pub(crate) fn new(
        operation: &'static str,
        path: Option<PathBuf>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            operation,
            path,
            detail: detail.into(),
        }
    }

    fn at(operation: &'static str, path: &Path, error: impl fmt::Display) -> Self {
        Self::new(operation, Some(path.to_path_buf()), error.to_string())
    }

    /// Stable description of the failed recovery operation.
    #[must_use]
    pub const fn operation(&self) -> &'static str {
        self.operation
    }

    /// Relevant recovery path, when the failure concerns the filesystem.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Human-readable failure detail.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for RecoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.path {
            Some(path) => write!(
                formatter,
                "{} '{}': {}",
                self.operation,
                path.display(),
                self.detail
            ),
            None => write!(formatter, "{}: {}", self.operation, self.detail),
        }
    }
}

impl std::error::Error for RecoveryError {}

/// Repository-scoped storage for the single active Workshop recovery draft.
#[derive(Debug, Clone)]
pub struct RecoveryStore {
    path: PathBuf,
}

impl RecoveryStore {
    /// Resolves the untracked recovery path below a repository root.
    #[must_use]
    pub fn new(repository_root: impl AsRef<Path>) -> Self {
        Self {
            path: repository_root.as_ref().join(RECOVERY_RELATIVE_PATH),
        }
    }

    /// Exact recovery file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads the recovery draft when it exists.
    ///
    /// Invalid or incompatible content returns an error and remains untouched.
    pub fn load(&self) -> Result<Option<RecoveryEnvelope>, RecoveryError> {
        if !self
            .path
            .try_exists()
            .map_err(|error| RecoveryError::at("inspect recovery", &self.path, error))?
        {
            return Ok(None);
        }
        let bytes = fs::read(&self.path)
            .map_err(|error| RecoveryError::at("read recovery", &self.path, error))?;
        decode_recovery(&bytes)
            .map(Some)
            .map_err(|error| error.with_path(&self.path))
    }

    /// Atomically writes a validated recovery envelope.
    pub fn write(&self, envelope: &RecoveryEnvelope) -> Result<(), RecoveryError> {
        let bytes = encode_recovery(envelope)?;
        let parent = self.path.parent().ok_or_else(|| {
            RecoveryError::at(
                "prepare recovery",
                &self.path,
                "recovery destination has no parent directory",
            )
        })?;
        fs::create_dir_all(parent)
            .map_err(|error| RecoveryError::at("create recovery directory", parent, error))?;
        AtomicFile::new(&self.path, AllowOverwrite)
            .write(|file| {
                file.write_all(&bytes)?;
                file.sync_all()
            })
            .map_err(|error| RecoveryError::at("replace recovery draft", &self.path, error))
    }

    /// Explicitly discards the current recovery file. Missing files are a no-op.
    pub fn discard(&self) -> Result<bool, RecoveryError> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(RecoveryError::at("discard recovery", &self.path, error)),
        }
    }
}

impl RecoveryError {
    fn with_path(mut self, path: &Path) -> Self {
        self.path = Some(path.to_path_buf());
        self
    }
}

/// Encodes a recovery envelope into canonical, newline-terminated RON.
pub fn encode_recovery(envelope: &RecoveryEnvelope) -> Result<Vec<u8>, RecoveryError> {
    let mut canonical = envelope.clone();
    canonical.workshop.normalize_and_validate()?;
    canonical.validate()?;
    let config = ron::ser::PrettyConfig::default()
        .new_line("\n")
        .indentor("    ");
    let mut encoded = ron::ser::to_string_pretty(&canonical, config)
        .map_err(|error| RecoveryError::new("encode recovery", None, error.to_string()))?;
    encoded.push('\n');
    let decoded = decode_recovery(encoded.as_bytes())?;
    if decoded != canonical {
        return Err(RecoveryError::new(
            "verify recovery",
            None,
            "encoded recovery did not preserve its normalized draft",
        ));
    }
    Ok(encoded.into_bytes())
}

/// Decodes and validates editor recovery RON.
pub fn decode_recovery(bytes: &[u8]) -> Result<RecoveryEnvelope, RecoveryError> {
    let source = std::str::from_utf8(bytes)
        .map_err(|error| RecoveryError::new("decode recovery", None, error.to_string()))?;
    let mut envelope: RecoveryEnvelope = ron::from_str(source)
        .map_err(|error| RecoveryError::new("decode recovery", None, error.to_string()))?;
    envelope.workshop.normalize_and_validate()?;
    envelope.validate()?;
    Ok(envelope)
}

fn validate_recovery_bounds(bounds: ObjectBounds) -> Result<(), RecoveryError> {
    if bounds.radius > MAX_OBJECT_RADIUS {
        return Err(RecoveryError::new(
            "validate recovery",
            None,
            format!(
                "recovery radius {} exceeds maximum {MAX_OBJECT_RADIUS}",
                bounds.radius
            ),
        ));
    }
    if bounds.height == 0 || bounds.height > MAX_OBJECT_HEIGHT {
        return Err(RecoveryError::new(
            "validate recovery",
            None,
            format!(
                "recovery height {} must be within 1..={MAX_OBJECT_HEIGHT}",
                bounds.height
            ),
        ));
    }
    validate_level("recovery minimum level", bounds.min_level)?;
    let maximum = bounds
        .min_level
        .checked_add(i32::from(bounds.height))
        .and_then(|exclusive| exclusive.checked_sub(1))
        .ok_or_else(|| {
            RecoveryError::new(
                "validate recovery",
                None,
                "recovery level range overflows i32",
            )
        })?;
    validate_level("recovery maximum level", maximum)
}

fn validate_position_safety(
    description: &'static str,
    position: LocalVoxelCoord,
) -> Result<(), RecoveryError> {
    validate_axial_safety(description, position.axial())?;
    validate_level(description, position.level)
}

fn validate_axial_safety(
    description: &'static str,
    position: LocalAxialCoord,
) -> Result<(), RecoveryError> {
    if position.radius() > i64::from(MAX_OBJECT_RADIUS) {
        return Err(RecoveryError::new(
            "validate recovery",
            None,
            format!(
                "{description} ({}, {}) exceeds maximum axial radius {MAX_OBJECT_RADIUS}",
                position.q, position.r
            ),
        ));
    }
    Ok(())
}

fn validate_level(description: &'static str, level: i32) -> Result<(), RecoveryError> {
    if !(-MAX_RECOVERY_LEVEL_ABS..=MAX_RECOVERY_LEVEL_ABS).contains(&level) {
        return Err(RecoveryError::new(
            "validate recovery",
            None,
            format!(
                "{description} level {level} exceeds recovery safety range -{MAX_RECOVERY_LEVEL_ABS}..={MAX_RECOVERY_LEVEL_ABS}"
            ),
        ));
    }
    Ok(())
}

pub(crate) fn sanitized_selection(
    positions: Vec<LocalVoxelCoord>,
    object: &ObjectBlueprint,
) -> (BTreeSet<LocalVoxelCoord>, RecoverySanitization) {
    let occupied: BTreeSet<_> = object
        .placements
        .iter()
        .map(|placement| placement.position)
        .collect();
    let original = positions.len();
    let selection: BTreeSet<_> = positions
        .into_iter()
        .filter(|position| occupied.contains(position))
        .collect();
    let retained = selection.len();
    (
        selection,
        RecoverySanitization {
            discarded_selection_cells: original.saturating_sub(retained),
        },
    )
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::atomic::{AtomicU64, Ordering};

    use hex_assets::{
        ConnectivityPolicy, ObjectCategory, PaletteSwatch, PlantPart, SrgbColor, SwatchId,
        VoxelStyle, VoxelSurfaceMode,
    };

    use super::*;
    use crate::model::{EditorModel, EditorTool, PreviewRig, WorkshopMode};
    use crate::workshop::WorkshopDraft;

    static TEMP_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn swatch_id(value: &str) -> SwatchId {
        SwatchId::new(value).expect("test swatch id should be valid")
    }

    fn style_id(value: &str) -> VoxelStyleId {
        VoxelStyleId::new(value).expect("test style id should be valid")
    }

    fn object_id(value: &str) -> ObjectAssetId {
        ObjectAssetId::new(value).expect("test object id should be valid")
    }

    fn fixture() -> WorkshopDraft {
        let base_swatch_id = swatch_id("editor/neutral");
        let base_swatch = PaletteSwatch::new(
            "Editor Neutral",
            SrgbColor::new(0.45, 0.5, 0.55).expect("test colour should be valid"),
            BTreeSet::from(["editor".to_owned()]),
        )
        .expect("test swatch should be valid");
        let palette = ArtPalette::new(BTreeMap::from([(base_swatch_id.clone(), base_swatch)]))
            .expect("test palette should be valid");
        let base_style_id = style_id("editor/neutral");
        let base_style = VoxelStyle::new(
            "Editor Neutral",
            base_swatch_id,
            VoxelSurfaceMode::Opaque,
            1.0,
            None,
        )
        .expect("test style should be valid");
        let styles = VoxelStyleCatalog::new(BTreeMap::from([(base_style_id.clone(), base_style)]))
            .expect("test styles should be valid");
        let editor = EditorModel::blank(
            ObjectCategory::Plant,
            ConnectivityPolicy::Grounded,
            base_style_id,
        )
        .expect("test editor should be valid");
        WorkshopDraft::new(palette, styles, editor)
    }

    fn recovery(draft: &WorkshopDraft) -> RecoveryEnvelope {
        RecoveryEnvelope::new(
            1_735_689_600_123,
            ProjectRevisionSet::default(),
            RecoveryDocument::Unsaved(object_id("plant/untitled")),
            draft.recovery_snapshot(),
        )
        .expect("test recovery should be valid")
    }

    fn temporary_root() -> PathBuf {
        let sequence = TEMP_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "hex-editor-recovery-{}-{sequence}",
            std::process::id()
        ))
    }

    #[test]
    fn recovery_round_trips_an_intrinsically_invalid_empty_object() {
        let mut draft = fixture();
        let origin = draft.editor().object().origin;
        assert_eq!(
            draft.edit_object("Erase root", |editor| editor.erase(origin)),
            Ok(true)
        );
        assert!(draft.editor().validate_draft().is_err());

        let envelope = recovery(&draft);
        let encoded = encode_recovery(&envelope).expect("invalid draft should encode safely");
        let decoded = decode_recovery(&encoded).expect("invalid draft should decode safely");
        let raw_object =
            ron::to_string(&decoded.workshop.editor.object).expect("raw object should serialize");
        assert!(
            ron::from_str::<ObjectBlueprint>(&raw_object).is_err(),
            "production deserialization must continue rejecting the invalid object"
        );

        let (restored, sanitization) = WorkshopDraft::from_recovery(decoded.workshop)
            .expect("recovery restore should admit the incomplete draft");
        assert_eq!(sanitization, RecoverySanitization::default());
        assert!(restored.editor().object().placements.is_empty());
        assert!(restored.editor().validate_draft().is_err());
    }

    #[test]
    fn recovery_document_identity_must_match_the_open_object() {
        let draft = fixture();
        let result = RecoveryEnvelope::new(
            1,
            ProjectRevisionSet::default(),
            RecoveryDocument::Saved(object_id("plant/different")),
            draft.recovery_snapshot(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn recovery_preserves_drafts_checkpoints_and_authoring_controls() {
        let mut draft = fixture();
        let accent_swatch_id = swatch_id("plant/accent");
        let accent_swatch = PaletteSwatch::new(
            "Plant Accent",
            SrgbColor::new(0.9, 0.18, 0.12).expect("test colour should be valid"),
            BTreeSet::from(["plant".to_owned()]),
        )
        .expect("test swatch should be valid");
        assert_eq!(
            draft.upsert_swatch(accent_swatch_id.clone(), accent_swatch, false),
            Ok(true)
        );
        let accent_style_id = style_id("plant/accent");
        let accent_style = VoxelStyle::new(
            "Plant Accent",
            accent_swatch_id,
            VoxelSurfaceMode::Cutout,
            0.8,
            None,
        )
        .expect("test style should be valid");
        assert_eq!(
            draft.upsert_style(accent_style_id.clone(), accent_style),
            Ok(true)
        );
        assert_eq!(
            draft.edit_object("Rename object", |editor| {
                editor.set_display_name("Recovered Plant")
            }),
            Ok(true)
        );

        let origin = draft.editor().object().origin;
        {
            let editor = draft.editor_mut_untracked();
            editor.set_mode(WorkshopMode::VoxelStyles);
            editor.set_tool(EditorTool::Select);
            editor.set_preview_rig(PreviewRig::Dark);
            editor
                .set_active_level(5)
                .expect("active test level should fit");
            editor.set_active_style(Some(accent_style_id.clone()));
            editor
                .set_active_part(ObjectPart::Plant(PlantPart::Trunk))
                .expect("test part should match the object");
            assert!(editor.select(origin, false));
            assert_eq!(
                editor.copy_selection(),
                Ok(1),
                "precondition: clipboard should contain transient data"
            );
        }
        draft
            .begin_object_transaction("Interrupted paint stroke")
            .expect("test transaction should begin");
        assert_eq!(
            draft.editor_mut_untracked().place(
                LocalVoxelCoord::new(0, 0, 1),
                accent_style_id.clone(),
                ObjectPart::Plant(PlantPart::Trunk),
            ),
            Ok(true)
        );

        let mut snapshot = draft.recovery_snapshot();
        snapshot
            .editor
            .selection
            .push(LocalVoxelCoord::new(1, 0, 0));
        let envelope = RecoveryEnvelope::new(
            42,
            ProjectRevisionSet::default(),
            RecoveryDocument::Unsaved(object_id("plant/untitled")),
            snapshot,
        )
        .expect("recovery snapshot should be valid");
        let decoded = decode_recovery(&encode_recovery(&envelope).expect("recovery should encode"))
            .expect("recovery should decode");
        let (restored, sanitization) =
            WorkshopDraft::from_recovery(decoded.workshop).expect("recovery should restore");

        assert_eq!(sanitization.discarded_selection_cells, 1);
        assert!(restored.is_palette_dirty());
        assert!(restored.is_styles_dirty());
        assert!(restored.editor().is_dirty());
        assert_eq!(restored.editor().mode(), WorkshopMode::VoxelStyles);
        assert_eq!(restored.editor().tool(), EditorTool::Select);
        assert_eq!(restored.editor().preview_rig(), PreviewRig::Dark);
        assert_eq!(restored.editor().active_level(), 5);
        assert_eq!(restored.editor().active_style(), Some(&accent_style_id));
        assert_eq!(
            restored.editor().active_part(),
            ObjectPart::Plant(PlantPart::Trunk)
        );
        assert_eq!(restored.editor().selection().len(), 1);
        assert!(restored.editor().selection().contains(origin));
        assert!(restored.editor().clipboard().is_empty());
        assert!(restored.undo_label().is_none());
        assert!(restored.redo_label().is_none());
        assert!(restored.editor().undo_label().is_none());
        assert!(restored.editor().redo_label().is_none());
        assert!(!restored.editor().is_transaction_open());
        assert_eq!(restored.editor().object().display_name, "Recovered Plant");
        assert!(
            restored
                .editor()
                .object()
                .placements
                .iter()
                .any(|placement| placement.position == LocalVoxelCoord::new(0, 0, 1)),
            "the in-progress semantic draft itself must survive recovery"
        );
    }

    #[test]
    fn canonical_encoding_is_byte_stable_and_normalizes_order() {
        let mut envelope = recovery(&fixture());
        envelope.workshop.editor.object.placements.reverse();
        envelope.workshop.editor.selection =
            vec![LocalVoxelCoord::new(0, 0, 0), LocalVoxelCoord::new(0, 0, 0)];

        let first = encode_recovery(&envelope).expect("recovery should encode");
        let decoded = decode_recovery(&first).expect("recovery should decode");
        let second = encode_recovery(&decoded).expect("recovery should re-encode");

        assert_eq!(first, second);
        assert!(first.ends_with(b"\n"));
        assert_eq!(decoded.workshop.editor.selection.len(), 1);
    }

    #[test]
    fn incompatible_or_malformed_recovery_is_rejected() {
        let envelope = recovery(&fixture());
        let encoded =
            String::from_utf8(encode_recovery(&envelope).expect("recovery should encode as UTF-8"))
                .expect("RON should be UTF-8");

        let incompatible = encoded.replacen("schema_version: 1", "schema_version: 99", 1);
        let error =
            decode_recovery(incompatible.as_bytes()).expect_err("unknown schema should fail");
        assert_eq!(error.operation(), "validate recovery");
        assert!(error.detail().contains("unsupported"));

        let unknown_field = encoded.replacen(
            "written_unix_ms:",
            "unexpected_field: true,\n    written_unix_ms:",
            1,
        );
        let error =
            decode_recovery(unknown_field.as_bytes()).expect_err("unknown fields should fail");
        assert_eq!(error.operation(), "decode recovery");

        let error = decode_recovery(b"not valid RON").expect_err("malformed RON should fail");
        assert_eq!(error.operation(), "decode recovery");
    }

    #[test]
    fn recovery_enforces_resource_and_coordinate_safety_caps() {
        let base = recovery(&fixture());

        let mut excessive_radius = base.clone();
        excessive_radius.workshop.editor.object.bounds.radius = MAX_OBJECT_RADIUS + 1;
        assert!(encode_recovery(&excessive_radius).is_err());

        let mut empty_height = base.clone();
        empty_height.workshop.editor.object.bounds.height = 0;
        assert!(encode_recovery(&empty_height).is_err());

        let mut excessive_level = base.clone();
        excessive_level.workshop.editor.active_level = MAX_RECOVERY_LEVEL_ABS + 1;
        assert!(encode_recovery(&excessive_level).is_err());

        let mut excessive_coordinate = base.clone();
        excessive_coordinate.workshop.editor.object.origin.q = i32::from(MAX_OBJECT_RADIUS) + 1;
        assert!(encode_recovery(&excessive_coordinate).is_err());

        let mut excessive_placements = base;
        let Some(placement) = excessive_placements
            .workshop
            .editor
            .object
            .placements
            .first()
            .cloned()
        else {
            unreachable!("recovery fixture must contain one placement")
        };
        excessive_placements
            .workshop
            .editor
            .object
            .placements
            .resize(MAX_OBJECT_VOXELS + 1, placement);
        assert!(encode_recovery(&excessive_placements).is_err());
    }

    #[test]
    fn atomic_store_preserves_last_good_file_after_a_rejected_write() {
        let root = temporary_root();
        let store = RecoveryStore::new(&root);
        let envelope = recovery(&fixture());
        store
            .write(&envelope)
            .expect("valid recovery should be written");
        let before = fs::read(store.path()).expect("recovery file should exist");
        assert_eq!(
            store.load().expect("recovery should load"),
            Some(envelope.clone())
        );

        let mut rejected = envelope;
        rejected.workshop.editor.object.bounds.height = 0;
        assert!(store.write(&rejected).is_err());
        assert_eq!(
            fs::read(store.path()).expect("previous recovery should remain"),
            before
        );

        fs::write(store.path(), b"corrupt").expect("test should corrupt recovery");
        assert!(store.load().is_err());
        assert_eq!(
            fs::read(store.path()).expect("corrupt source should remain"),
            b"corrupt"
        );
        assert!(store.discard().expect("recovery should be discarded"));
        assert!(!store.discard().expect("missing recovery should be a no-op"));
        fs::remove_dir_all(root).expect("temporary recovery tree should be removable");
    }
}
