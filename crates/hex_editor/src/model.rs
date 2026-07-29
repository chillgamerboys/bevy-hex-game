//! Pure in-memory state and editing operations for the Asset Workshop.
//!
//! This module has no Bevy or UI dependency. Input layers translate pointer and
//! keyboard gestures into these commands, while project persistence validates the
//! resulting draft at the explicit save boundary.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use hex_assets::{
    ConnectivityPolicy, EffectPart, LocalAxialCoord, LocalVoxelCoord, ObjectAssetId,
    ObjectBlueprint, ObjectBounds, ObjectCategory, ObjectPart, ObjectPlacement, PlantPart,
    VoxelStyleCatalog, VoxelStyleId, MAX_OBJECT_HEIGHT, MAX_OBJECT_RADIUS, MAX_OBJECT_VOXELS,
    OBJECT_BLUEPRINT_SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};

use crate::history::{HistoryError, SnapshotHistory};
use crate::recovery::{
    sanitized_selection, EditorRecoveryDraft, RawObjectDraft, RecoveryError, RecoverySanitization,
};

/// The two authoring workspaces sharing one editor window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkshopMode {
    /// Edit reusable palette-backed voxel styles.
    VoxelStyles,
    /// Assemble styles into static voxel objects.
    Objects,
}

/// Active object-editing gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EditorTool {
    /// Add an occupied cell.
    Place,
    /// Remove an occupied cell.
    Erase,
    /// Change an occupied cell's style.
    Repaint,
    /// Read a style and semantic role from an occupied cell.
    Eyedropper,
    /// Build or transform a cell selection.
    Select,
}

/// Deterministic lighting rig used by the authoring viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreviewRig {
    /// Neutral key and fill lighting for ordinary authoring.
    Neutral,
    /// Dark environment for checking emission and silhouette.
    Dark,
    /// Unlit presentation for checking exact palette colour.
    Unlit,
}

/// A recoverable editor-model failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorModelError {
    message: String,
}

impl EditorModelError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Human-readable failure detail suitable for the editor status bar.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for EditorModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for EditorModelError {}

impl From<HistoryError> for EditorModelError {
    fn from(error: HistoryError) -> Self {
        Self::new(error.to_string())
    }
}

/// Exact occupied cells selected in the object viewport.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObjectSelection {
    cells: BTreeSet<LocalVoxelCoord>,
}

impl ObjectSelection {
    /// Whether no occupied cells are selected.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Number of selected cells.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Whether `position` is selected.
    #[must_use]
    pub fn contains(&self, position: LocalVoxelCoord) -> bool {
        self.cells.contains(&position)
    }

    /// Selected positions in deterministic coordinate order.
    #[must_use]
    pub fn cells(&self) -> &BTreeSet<LocalVoxelCoord> {
        &self.cells
    }

    fn replace_with(&mut self, positions: impl IntoIterator<Item = LocalVoxelCoord>) {
        self.cells.clear();
        self.cells.extend(positions);
    }
}

/// One copied voxel relative to the clipboard anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardVoxel {
    offset: LocalVoxelCoord,
    style: VoxelStyleId,
    part: ObjectPart,
    canopy_occluder: bool,
}

impl ClipboardVoxel {
    /// Offset from the clipboard anchor.
    #[must_use]
    pub const fn offset(&self) -> LocalVoxelCoord {
        self.offset
    }

    /// Reusable visual style.
    #[must_use]
    pub fn style(&self) -> &VoxelStyleId {
        &self.style
    }

    /// Category-safe semantic part.
    #[must_use]
    pub const fn part(&self) -> ObjectPart {
        self.part
    }

    /// Whether this exact copied cell participates in canopy cutaway.
    #[must_use]
    pub const fn is_canopy_occluder(&self) -> bool {
        self.canopy_occluder
    }
}

/// Clipboard payload produced from an object selection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObjectClipboard {
    category: Option<ObjectCategory>,
    voxels: Vec<ClipboardVoxel>,
    blocker_offsets: BTreeSet<LocalAxialCoord>,
}

impl ObjectClipboard {
    /// Whether no voxels are available to paste.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.voxels.is_empty()
    }

    /// Number of copied voxels.
    #[must_use]
    pub fn len(&self) -> usize {
        self.voxels.len()
    }

    /// Copied voxels in deterministic offset order.
    #[must_use]
    pub fn voxels(&self) -> &[ClipboardVoxel] {
        &self.voxels
    }

    /// Copied prop blocker columns relative to the clipboard anchor.
    #[must_use]
    pub fn blocker_offsets(&self) -> &BTreeSet<LocalAxialCoord> {
        &self.blocker_offsets
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EditorSnapshot {
    object: ObjectBlueprint,
    selection: ObjectSelection,
}

/// UI-independent state for one open Asset Workshop document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorModel {
    mode: WorkshopMode,
    tool: EditorTool,
    preview_rig: PreviewRig,
    active_level: i32,
    active_style: Option<VoxelStyleId>,
    active_part: ObjectPart,
    object: ObjectBlueprint,
    selection: ObjectSelection,
    clipboard: ObjectClipboard,
    saved_object: Option<ObjectBlueprint>,
    history: SnapshotHistory<EditorSnapshot>,
}

impl EditorModel {
    /// Opens a fully intrinsic-valid blueprint as a clean editor document.
    pub fn from_blueprint(object: ObjectBlueprint) -> Result<Self, EditorModelError> {
        object.validate_intrinsic().map_err(EditorModelError::new)?;
        let active_part = default_part(object.category);
        let active_level = object.origin.level;
        Ok(Self {
            mode: WorkshopMode::Objects,
            tool: EditorTool::Place,
            preview_rig: PreviewRig::Neutral,
            active_level,
            active_style: object
                .placements
                .first()
                .map(|placement| placement.style.clone()),
            active_part,
            saved_object: Some(object.clone()),
            object,
            selection: ObjectSelection::default(),
            clipboard: ObjectClipboard::default(),
            history: SnapshotHistory::default(),
        })
    }

    /// Captures all durable object-authoring state for crash recovery.
    ///
    /// Clipboard contents, undo/redo history, and an open transaction are
    /// intentionally session-only and are not included.
    #[must_use]
    pub fn recovery_snapshot(&self) -> EditorRecoveryDraft {
        EditorRecoveryDraft {
            object: RawObjectDraft::from_blueprint(&self.object),
            saved_object: RawObjectDraft::from_blueprint(&self.saved_object),
            mode: self.mode,
            tool: self.tool,
            preview_rig: self.preview_rig,
            active_level: self.active_level,
            active_style: self.active_style.clone(),
            active_part: self.active_part,
            selection: self.selection.cells.iter().copied().collect(),
        }
    }

    /// Restores a potentially incomplete object-authoring draft from recovery.
    ///
    /// Production blueprint validation is deliberately deferred until an explicit
    /// save. Selection cells that are no longer occupied are discarded, while
    /// clipboard and history state restart empty.
    pub fn from_recovery(
        mut recovery: EditorRecoveryDraft,
    ) -> Result<(Self, RecoverySanitization), RecoveryError> {
        recovery.normalize_and_validate()?;
        let EditorRecoveryDraft {
            object,
            saved_object,
            mode,
            tool,
            preview_rig,
            active_level,
            active_style,
            active_part,
            selection,
        } = recovery;
        let object = object.into_blueprint();
        let saved_object = saved_object.into_blueprint();
        let (selection, sanitization) = sanitized_selection(selection, &object);
        Ok((
            Self {
                mode,
                tool,
                preview_rig,
                active_level,
                active_style,
                active_part,
                object,
                selection: ObjectSelection { cells: selection },
                clipboard: ObjectClipboard::default(),
                saved_object,
                history: SnapshotHistory::default(),
            },
            sanitization,
        ))
    }

    /// Builds the unsaved, in-memory scene shown when no authored object is open.
    pub fn calibration_scene() -> Result<Self, EditorModelError> {
        let id = ObjectAssetId::new("calibration/scene")
            .map_err(|error| EditorModelError::new(error.to_string()))?;
        let style = VoxelStyleId::new("calibration/neutral")
            .map_err(|error| EditorModelError::new(error.to_string()))?;
        let origin = LocalVoxelCoord::new(0, 0, 0);
        let object = ObjectBlueprint {
            schema_version: OBJECT_BLUEPRINT_SCHEMA_VERSION,
            id,
            display_name: "Calibration Scene".to_owned(),
            category: ObjectCategory::Plant,
            bounds: ObjectBounds::DEFAULT,
            connectivity: ConnectivityPolicy::Grounded,
            origin,
            placements: vec![ObjectPlacement {
                position: origin,
                style: style.clone(),
                part: ObjectPart::Plant(PlantPart::Root),
            }],
            blocker_footprint: vec![origin.axial()],
            canopy_occluders: Vec::new(),
        };
        let mut editor = Self::from_blueprint(object)?;
        editor.active_style = Some(style);
        Ok(editor)
    }

    /// Creates a validated unsaved document for one object category.
    ///
    /// Plants and effects require their fixed connectivity policy. Props may be
    /// grounded or free; free documents use a signed vertical canvas.
    pub fn blank(
        category: ObjectCategory,
        connectivity: ConnectivityPolicy,
        style: VoxelStyleId,
    ) -> Result<Self, EditorModelError> {
        match (category, connectivity) {
            (ObjectCategory::Plant, ConnectivityPolicy::Grounded)
            | (ObjectCategory::Effect, ConnectivityPolicy::Free)
            | (ObjectCategory::Prop, _) => {}
            (ObjectCategory::Plant, ConnectivityPolicy::Free) => {
                return Err(EditorModelError::new(
                    "new plant documents must be grounded",
                ));
            }
            (ObjectCategory::Effect, ConnectivityPolicy::Grounded) => {
                return Err(EditorModelError::new(
                    "new effect documents must use free connectivity",
                ));
            }
        }
        let (id, display_name, part, blockers) = match category {
            ObjectCategory::Plant => (
                "plant/untitled",
                "Untitled Plant",
                ObjectPart::Plant(PlantPart::Root),
                vec![LocalAxialCoord::new(0, 0)],
            ),
            ObjectCategory::Effect => (
                "effect/untitled",
                "Untitled Effect",
                ObjectPart::Effect(EffectPart::Core),
                Vec::new(),
            ),
            ObjectCategory::Prop => (
                "prop/untitled",
                "Untitled Prop",
                ObjectPart::Prop(hex_assets::PropPart::Structure),
                Vec::new(),
            ),
        };
        let id =
            ObjectAssetId::new(id).map_err(|error| EditorModelError::new(error.to_string()))?;
        let origin = LocalVoxelCoord::new(0, 0, 0);
        let bounds = if connectivity == ConnectivityPolicy::Free {
            ObjectBounds {
                radius: ObjectBounds::DEFAULT.radius,
                min_level: -18,
                height: ObjectBounds::DEFAULT.height,
            }
        } else {
            ObjectBounds::DEFAULT
        };
        let object = ObjectBlueprint {
            schema_version: OBJECT_BLUEPRINT_SCHEMA_VERSION,
            id,
            display_name: display_name.to_owned(),
            category,
            bounds,
            connectivity,
            origin,
            placements: vec![ObjectPlacement {
                position: origin,
                style: style.clone(),
                part,
            }],
            blocker_footprint: blockers,
            canopy_occluders: Vec::new(),
        };
        let mut editor = Self::from_blueprint(object)?;
        editor.active_style = Some(style);
        editor.saved_object = None;
        Ok(editor)
    }

    /// Current workshop mode.
    #[must_use]
    pub const fn mode(&self) -> WorkshopMode {
        self.mode
    }

    /// Changes the visible workshop mode without changing document history.
    pub fn set_mode(&mut self, mode: WorkshopMode) {
        self.mode = mode;
    }

    /// Active editing tool.
    #[must_use]
    pub const fn tool(&self) -> EditorTool {
        self.tool
    }

    /// Changes the active editing tool.
    pub fn set_tool(&mut self, tool: EditorTool) {
        self.tool = tool;
    }

    /// Current preview lighting rig.
    #[must_use]
    pub const fn preview_rig(&self) -> PreviewRig {
        self.preview_rig
    }

    /// Changes the preview lighting rig.
    pub fn set_preview_rig(&mut self, preview_rig: PreviewRig) {
        self.preview_rig = preview_rig;
    }

    /// Active integer editing level.
    #[must_use]
    pub const fn active_level(&self) -> i32 {
        self.active_level
    }

    /// Sets the active editing level when it lies inside the object's canvas.
    pub fn set_active_level(&mut self, level: i32) -> Result<(), EditorModelError> {
        let position = LocalVoxelCoord::new(0, 0, level);
        if !self.object.bounds.contains(position) {
            return Err(EditorModelError::new(format!(
                "active level {level} lies outside levels {}..={}; adjust Document > Authoring bounds",
                self.object.bounds.min_level,
                maximum_level(self.object.bounds)
            )));
        }
        self.active_level = level;
        Ok(())
    }

    /// Active reusable visual style, if one has been chosen.
    #[must_use]
    pub fn active_style(&self) -> Option<&VoxelStyleId> {
        self.active_style.as_ref()
    }

    /// Changes the active reusable visual style.
    pub fn set_active_style(&mut self, style: Option<VoxelStyleId>) {
        self.active_style = style;
    }

    /// Active semantic role used by placement and part painting.
    #[must_use]
    pub const fn active_part(&self) -> ObjectPart {
        self.active_part
    }

    /// Changes the active semantic role when it belongs to this object category.
    pub fn set_active_part(&mut self, part: ObjectPart) -> Result<(), EditorModelError> {
        validate_part_category(self.object.category, part)?;
        self.active_part = part;
        Ok(())
    }

    /// Current draft blueprint.
    #[must_use]
    pub const fn object(&self) -> &ObjectBlueprint {
        &self.object
    }

    /// Current exact selection.
    #[must_use]
    pub const fn selection(&self) -> &ObjectSelection {
        &self.selection
    }

    /// Current clipboard payload.
    #[must_use]
    pub const fn clipboard(&self) -> &ObjectClipboard {
        &self.clipboard
    }

    /// Whether object semantics differ from the last saved checkpoint.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.saved_object.as_ref() != Some(&self.object)
    }

    /// Marks the current draft as the saved checkpoint.
    pub fn mark_saved(&mut self) {
        self.saved_object = Some(self.object.clone());
    }

    /// Adopts the immutable identity assigned by a successful Save As operation.
    ///
    /// This is a persistence checkpoint rather than an undoable content edit.
    pub fn mark_saved_as(&mut self, id: ObjectAssetId) {
        self.object.id = id;
        self.mark_saved();
        self.history.clear();
    }

    /// Assigns a proposed identity and name to a document that has not been saved.
    ///
    /// The identity does not become immutable until persistence succeeds.
    pub fn set_unsaved_identity(
        &mut self,
        id: ObjectAssetId,
        display_name: impl Into<String>,
    ) -> Result<(), EditorModelError> {
        let display_name = display_name.into();
        if display_name.trim().is_empty() {
            return Err(EditorModelError::new(
                "object display name must contain visible text",
            ));
        }
        self.object.id = id;
        self.object.display_name = display_name;
        self.saved_object = None;
        self.history.clear();
        Ok(())
    }

    /// Changes the editable display name as one undoable operation.
    pub fn set_display_name(
        &mut self,
        display_name: impl Into<String>,
    ) -> Result<bool, EditorModelError> {
        let display_name = display_name.into();
        if display_name.trim().is_empty() {
            return Err(EditorModelError::new(
                "object display name must contain visible text",
            ));
        }
        self.edit("Rename object", move |object, _selection| {
            object.display_name = display_name;
            Ok(())
        })
    }

    /// Changes the authoring canvas when every existing cell and mask still fits.
    pub fn set_bounds(&mut self, bounds: ObjectBounds) -> Result<bool, EditorModelError> {
        validate_bounds(bounds)?;
        let changed = self.edit("Change object bounds", move |object, _selection| {
            if !bounds.contains(object.origin) {
                return Err(EditorModelError::new(format!(
                    "origin {:?} lies outside the requested bounds",
                    object.origin
                )));
            }
            if let Some(placement) = object
                .placements
                .iter()
                .find(|placement| !bounds.contains(placement.position))
            {
                return Err(EditorModelError::new(format!(
                    "occupied cell {:?} lies outside the requested bounds",
                    placement.position
                )));
            }
            if let Some(blocker) = object
                .blocker_footprint
                .iter()
                .find(|blocker| !bounds.contains_axial(**blocker))
            {
                return Err(EditorModelError::new(format!(
                    "blocker cell {blocker:?} lies outside the requested bounds"
                )));
            }
            object.bounds = bounds;
            Ok(())
        })?;
        let maximum = bounds
            .min_level
            .saturating_add(i32::from(bounds.height))
            .saturating_sub(1);
        self.active_level = self.active_level.clamp(bounds.min_level, maximum);
        Ok(changed)
    }

    /// Moves the object pivot/root to an occupied category-valid cell.
    pub fn set_origin(&mut self, origin: LocalVoxelCoord) -> Result<bool, EditorModelError> {
        self.edit("Move object origin", move |object, _selection| {
            validate_position(object, origin)?;
            let part = placement_at(object, origin)
                .map(|placement| placement.part)
                .ok_or_else(|| {
                    EditorModelError::new(format!("origin {origin:?} must name an occupied cell"))
                })?;
            validate_origin_part(object.category, object.connectivity, origin, part)?;
            object.origin = origin;
            Ok(())
        })
    }

    /// Changes a prop between grounded and free connectivity.
    ///
    /// Plant and effect connectivity is fixed by their category contract.
    pub fn set_connectivity(
        &mut self,
        connectivity: ConnectivityPolicy,
    ) -> Result<bool, EditorModelError> {
        self.edit("Change connectivity", move |object, _selection| {
            match (object.category, connectivity) {
                (ObjectCategory::Plant, ConnectivityPolicy::Grounded)
                | (ObjectCategory::Effect, ConnectivityPolicy::Free)
                | (ObjectCategory::Prop, _) => {}
                (ObjectCategory::Plant, ConnectivityPolicy::Free) => {
                    return Err(EditorModelError::new("plant objects must remain grounded"));
                }
                (ObjectCategory::Effect, ConnectivityPolicy::Grounded) => {
                    return Err(EditorModelError::new("effect objects must remain free"));
                }
            }
            validate_origin_part(
                object.category,
                connectivity,
                object.origin,
                placement_at(object, object.origin)
                    .map(|placement| placement.part)
                    .ok_or_else(|| EditorModelError::new("object origin is not occupied"))?,
            )?;
            object.connectivity = connectivity;
            Ok(())
        })
    }

    /// Checks whether the current draft is intrinsically complete.
    pub fn validate_draft(&self) -> Result<(), EditorModelError> {
        self.object
            .validate_intrinsic()
            .map_err(EditorModelError::new)
    }

    /// Produces a sorted, fully validated blueprint for persistence.
    pub fn blueprint_for_save(
        &self,
        styles: &VoxelStyleCatalog,
    ) -> Result<ObjectBlueprint, EditorModelError> {
        let mut object = self.object.clone();
        normalize_collections(&mut object);
        object.validate(styles).map_err(EditorModelError::new)?;
        Ok(object)
    }

    /// Begins a grouped editing transaction such as one pointer drag.
    pub fn begin_transaction(&mut self, label: impl Into<String>) -> Result<(), EditorModelError> {
        let snapshot = self.snapshot();
        self.history.begin(label, &snapshot)?;
        Ok(())
    }

    /// Commits all commands since [`Self::begin_transaction`] as one undo step.
    pub fn commit_transaction(&mut self) -> Result<bool, EditorModelError> {
        normalize_collections(&mut self.object);
        let snapshot = self.snapshot();
        self.history.commit(&snapshot).map_err(Into::into)
    }

    /// Cancels an open transaction and restores its baseline.
    pub fn cancel_transaction(&mut self) -> Result<(), EditorModelError> {
        let snapshot = self.history.cancel()?;
        self.restore(snapshot);
        Ok(())
    }

    /// Undoes one atomic command or grouped transaction.
    pub fn undo(&mut self) -> Result<bool, EditorModelError> {
        let current = self.snapshot();
        let Some(snapshot) = self.history.undo(&current)? else {
            return Ok(false);
        };
        self.restore(snapshot);
        Ok(true)
    }

    /// Redoes one atomic command or grouped transaction.
    pub fn redo(&mut self) -> Result<bool, EditorModelError> {
        let current = self.snapshot();
        let Some(snapshot) = self.history.redo(&current)? else {
            return Ok(false);
        };
        self.restore(snapshot);
        Ok(true)
    }

    /// Label of the next object edit that Undo would restore.
    #[must_use]
    pub fn undo_label(&self) -> Option<&str> {
        self.history.undo_label()
    }

    /// Label of the next object edit that Redo would reapply.
    #[must_use]
    pub fn redo_label(&self) -> Option<&str> {
        self.history.redo_label()
    }

    /// Whether an object paint/transform transaction is currently open.
    #[must_use]
    pub const fn is_transaction_open(&self) -> bool {
        self.history.is_transaction_open()
    }

    /// Places an occupied cell with the active style and semantic part.
    pub fn place_active(&mut self, position: LocalVoxelCoord) -> Result<bool, EditorModelError> {
        let Some(style) = self.active_style.clone() else {
            return Err(EditorModelError::new(
                "choose a voxel style before placing a cell",
            ));
        };
        self.place(position, style, self.active_part)
    }

    /// Places one occupied cell.
    pub fn place(
        &mut self,
        position: LocalVoxelCoord,
        style: VoxelStyleId,
        part: ObjectPart,
    ) -> Result<bool, EditorModelError> {
        self.edit("Place voxel", move |object, _selection| {
            validate_position(object, position)?;
            validate_part_for_position(object.category, part, position)?;
            if placement_at(object, position).is_some() {
                return Err(EditorModelError::new(format!(
                    "cannot place at {position:?}: the cell is already occupied"
                )));
            }
            if object.placements.len() >= MAX_OBJECT_VOXELS {
                return Err(EditorModelError::new(format!(
                    "cannot place more than {MAX_OBJECT_VOXELS} voxels"
                )));
            }
            object.placements.push(ObjectPlacement {
                position,
                style,
                part,
            });
            repair_masks_after_content_change(object);
            Ok(())
        })
    }

    /// Erases one occupied cell. Erasing empty space is a no-op.
    pub fn erase(&mut self, position: LocalVoxelCoord) -> Result<bool, EditorModelError> {
        self.edit("Erase voxel", move |object, selection| {
            let before = object.placements.len();
            object
                .placements
                .retain(|placement| placement.position != position);
            if object.placements.len() == before {
                return Ok(());
            }
            object
                .canopy_occluders
                .retain(|candidate| *candidate != position);
            selection.cells.remove(&position);
            repair_masks_after_content_change(object);
            Ok(())
        })
    }

    /// Repaints one occupied cell.
    pub fn repaint(
        &mut self,
        position: LocalVoxelCoord,
        style: VoxelStyleId,
    ) -> Result<bool, EditorModelError> {
        self.edit("Repaint voxel", move |object, _selection| {
            let Some(placement) = placement_at_mut(object, position) else {
                return Err(EditorModelError::new(format!(
                    "cannot repaint {position:?}: the cell is empty"
                )));
            };
            placement.style = style;
            Ok(())
        })
    }

    /// Samples an occupied cell into the active style and semantic part.
    pub fn pick_from(&mut self, position: LocalVoxelCoord) -> Result<(), EditorModelError> {
        let Some(placement) = placement_at(&self.object, position) else {
            return Err(EditorModelError::new(format!(
                "cannot sample {position:?}: the cell is empty"
            )));
        };
        self.active_style = Some(placement.style.clone());
        self.active_part = placement.part;
        Ok(())
    }

    /// Changes the semantic role of one occupied cell.
    pub fn change_part(
        &mut self,
        position: LocalVoxelCoord,
        part: ObjectPart,
    ) -> Result<bool, EditorModelError> {
        self.edit("Change voxel part", move |object, _selection| {
            validate_part_for_position(object.category, part, position)?;
            let Some(placement) = placement_at_mut(object, position) else {
                return Err(EditorModelError::new(format!(
                    "cannot change {position:?}: the cell is empty"
                )));
            };
            placement.part = part;
            if part != ObjectPart::Plant(PlantPart::Foliage) {
                object
                    .canopy_occluders
                    .retain(|candidate| *candidate != position);
            }
            repair_masks_after_content_change(object);
            Ok(())
        })
    }

    /// Adds or removes one exact foliage cell from the canopy cutaway mask.
    pub fn set_canopy_occluder(
        &mut self,
        position: LocalVoxelCoord,
        enabled: bool,
    ) -> Result<bool, EditorModelError> {
        self.edit("Change canopy mask", move |object, _selection| {
            if object.category != ObjectCategory::Plant {
                return Err(EditorModelError::new(
                    "only plant objects can define canopy occluders",
                ));
            }
            let is_foliage = placement_at(object, position)
                .is_some_and(|placement| placement.part == ObjectPart::Plant(PlantPart::Foliage));
            if enabled && !is_foliage {
                return Err(EditorModelError::new(format!(
                    "canopy cell {position:?} must be occupied plant foliage"
                )));
            }
            set_membership(&mut object.canopy_occluders, position, enabled);
            Ok(())
        })
    }

    /// Adds or removes one prop blocker column.
    ///
    /// Plant blockers are derived exactly from root cells and effects never block.
    pub fn set_prop_blocker(
        &mut self,
        position: LocalAxialCoord,
        enabled: bool,
    ) -> Result<bool, EditorModelError> {
        self.edit("Change blocker mask", move |object, _selection| {
            if object.category != ObjectCategory::Prop {
                return Err(EditorModelError::new(
                    "only prop blocker masks are edited directly",
                ));
            }
            if !object.bounds.contains_axial(position) {
                return Err(EditorModelError::new(format!(
                    "blocker {position:?} lies outside the object canvas"
                )));
            }
            if enabled
                && !object
                    .placements
                    .iter()
                    .any(|placement| placement.position.axial() == position)
            {
                return Err(EditorModelError::new(format!(
                    "blocker {position:?} has no occupied voxel in its column"
                )));
            }
            set_membership(&mut object.blocker_footprint, position, enabled);
            Ok(())
        })
    }

    /// Selects one occupied cell, replacing the selection unless `additive`.
    ///
    /// Selecting empty space clears a non-additive selection.
    pub fn select(&mut self, position: LocalVoxelCoord, additive: bool) -> bool {
        let before = self.selection.clone();
        if !additive {
            self.selection.cells.clear();
        }
        if placement_at(&self.object, position).is_some() {
            self.selection.cells.insert(position);
        }
        self.selection != before
    }

    /// Adds all occupied positions from an iterator to the selection.
    pub fn select_additive(
        &mut self,
        positions: impl IntoIterator<Item = LocalVoxelCoord>,
    ) -> usize {
        let before = self.selection.len();
        for position in positions {
            if placement_at(&self.object, position).is_some() {
                self.selection.cells.insert(position);
            }
        }
        self.selection.len().saturating_sub(before)
    }

    /// Clears the current selection.
    pub fn clear_selection(&mut self) {
        self.selection.cells.clear();
    }

    /// Moves the selection by an exact axial and vertical delta.
    pub fn nudge_selection(
        &mut self,
        q: i32,
        r: i32,
        level: i32,
    ) -> Result<bool, EditorModelError> {
        self.transform_selection("Nudge selection", move |position| {
            checked_translate(position, q, r, level)
        })
    }

    /// Rotates the selection clockwise by exactly 60 degrees around `pivot`.
    pub fn rotate_selection_clockwise(
        &mut self,
        pivot: LocalVoxelCoord,
    ) -> Result<bool, EditorModelError> {
        self.transform_selection("Rotate selection", move |position| {
            position.rotated_clockwise_60(pivot).ok_or_else(|| {
                EditorModelError::new("selection rotation overflowed object coordinates")
            })
        })
    }

    /// Copies selected voxels and their selected semantics into the clipboard.
    pub fn copy_selection(&mut self) -> Result<usize, EditorModelError> {
        let Some(anchor) = self.selection.cells.iter().next().copied() else {
            return Err(EditorModelError::new("cannot copy an empty selection"));
        };
        let canopy: BTreeSet<_> = self.object.canopy_occluders.iter().copied().collect();
        let blockers: BTreeSet<_> = self.object.blocker_footprint.iter().copied().collect();
        let mut voxels = Vec::with_capacity(self.selection.len());
        for position in &self.selection.cells {
            let Some(placement) = placement_at(&self.object, *position) else {
                return Err(EditorModelError::new(format!(
                    "selection contains empty cell {position:?}"
                )));
            };
            voxels.push(ClipboardVoxel {
                offset: checked_difference(*position, anchor)?,
                style: placement.style.clone(),
                part: placement.part,
                canopy_occluder: canopy.contains(position),
            });
        }
        voxels.sort_by_key(ClipboardVoxel::offset);

        let mut blocker_offsets = BTreeSet::new();
        for blocker in blockers {
            let occupied_column: Vec<_> = self
                .object
                .placements
                .iter()
                .map(|placement| placement.position)
                .filter(|position| position.axial() == blocker)
                .collect();
            if !occupied_column.is_empty()
                && occupied_column
                    .iter()
                    .all(|position| self.selection.cells.contains(position))
            {
                blocker_offsets.insert(checked_axial_difference(blocker, anchor.axial())?);
            }
        }
        self.clipboard = ObjectClipboard {
            category: Some(self.object.category),
            voxels,
            blocker_offsets,
        };
        Ok(self.clipboard.len())
    }

    /// Pastes the clipboard so its deterministic anchor lands at `target`.
    pub fn paste_at(&mut self, target: LocalVoxelCoord) -> Result<bool, EditorModelError> {
        let clipboard = self.clipboard.clone();
        if clipboard.is_empty() {
            return Err(EditorModelError::new("cannot paste an empty clipboard"));
        }
        if clipboard.category != Some(self.object.category) {
            return Err(EditorModelError::new(
                "clipboard semantic parts belong to a different object category",
            ));
        }
        self.edit("Paste voxels", move |object, selection| {
            if object
                .placements
                .len()
                .saturating_add(clipboard.voxels.len())
                > MAX_OBJECT_VOXELS
            {
                return Err(EditorModelError::new(format!(
                    "paste would exceed the {MAX_OBJECT_VOXELS}-voxel limit"
                )));
            }
            let mut positions = BTreeSet::new();
            let mut staged = Vec::with_capacity(clipboard.voxels.len());
            for voxel in &clipboard.voxels {
                let position =
                    checked_translate(target, voxel.offset.q, voxel.offset.r, voxel.offset.level)?;
                validate_position(object, position)?;
                validate_part_for_position(object.category, voxel.part, position)?;
                if placement_at(object, position).is_some() || !positions.insert(position) {
                    return Err(EditorModelError::new(format!(
                        "cannot paste at {position:?}: the cell is already occupied"
                    )));
                }
                staged.push((position, voxel.clone()));
            }
            let staged_blockers = if object.category == ObjectCategory::Prop {
                clipboard
                    .blocker_offsets
                    .iter()
                    .map(|offset| checked_translate_axial(target.axial(), offset.q, offset.r))
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                Vec::new()
            };

            for (position, voxel) in &staged {
                object.placements.push(ObjectPlacement {
                    position: *position,
                    style: voxel.style.clone(),
                    part: voxel.part,
                });
                if voxel.canopy_occluder {
                    object.canopy_occluders.push(*position);
                }
            }
            for blocker in staged_blockers {
                set_membership(&mut object.blocker_footprint, blocker, true);
            }
            repair_masks_after_content_change(object);
            selection.replace_with(staged.iter().map(|(position, _)| *position));
            Ok(())
        })
    }

    /// Deletes every selected voxel as one atomic command.
    pub fn delete_selection(&mut self) -> Result<bool, EditorModelError> {
        let selected = self.selection.cells.clone();
        self.edit("Delete selection", move |object, selection| {
            if selected.is_empty() {
                return Ok(());
            }
            object
                .placements
                .retain(|placement| !selected.contains(&placement.position));
            object
                .canopy_occluders
                .retain(|position| !selected.contains(position));
            selection.cells.clear();
            repair_masks_after_content_change(object);
            Ok(())
        })
    }

    fn transform_selection(
        &mut self,
        label: &'static str,
        transform: impl Fn(LocalVoxelCoord) -> Result<LocalVoxelCoord, EditorModelError>,
    ) -> Result<bool, EditorModelError> {
        let selected = self.selection.cells.clone();
        if selected.is_empty() {
            return Err(EditorModelError::new("cannot transform an empty selection"));
        }
        let mut mapping = BTreeMap::new();
        for position in &selected {
            let target = transform(*position)?;
            mapping.insert(*position, target);
        }
        self.edit(label, move |object, selection| {
            validate_transform(object, &selected, &mapping)?;
            let transformed_blockers = transformed_prop_blockers(object, &selected, &mapping)?;

            for placement in &mut object.placements {
                if let Some(target) = mapping.get(&placement.position) {
                    placement.position = *target;
                }
            }
            if let Some(target) = mapping.get(&object.origin) {
                object.origin = *target;
            }
            for canopy in &mut object.canopy_occluders {
                if let Some(target) = mapping.get(canopy) {
                    *canopy = *target;
                }
            }
            if let Some(blockers) = transformed_blockers {
                object.blocker_footprint = blockers;
            }
            repair_masks_after_content_change(object);
            selection.replace_with(mapping.values().copied());
            Ok(())
        })
    }

    fn edit(
        &mut self,
        label: &'static str,
        operation: impl FnOnce(
            &mut ObjectBlueprint,
            &mut ObjectSelection,
        ) -> Result<(), EditorModelError>,
    ) -> Result<bool, EditorModelError> {
        if self.history.is_transaction_open() {
            #[cfg(debug_assertions)]
            let before = self.snapshot();
            let result = operation(&mut self.object, &mut self.selection);
            #[cfg(debug_assertions)]
            if result.is_err() {
                debug_assert_eq!(
                    self.snapshot(),
                    before,
                    "transaction command mutated editor state before returning an error"
                );
            }
            result?;
            return Ok(true);
        }
        let before = self.snapshot();
        if let Err(error) = operation(&mut self.object, &mut self.selection) {
            self.restore(before);
            return Err(error);
        }
        normalize_collections(&mut self.object);
        let current = self.snapshot();
        self.history
            .record_atomic(label, before, &current)
            .map_err(Into::into)
    }

    fn snapshot(&self) -> EditorSnapshot {
        EditorSnapshot {
            object: self.object.clone(),
            selection: self.selection.clone(),
        }
    }

    fn restore(&mut self, snapshot: EditorSnapshot) {
        self.object = snapshot.object;
        self.selection = snapshot.selection;
    }
}

fn default_part(category: ObjectCategory) -> ObjectPart {
    match category {
        ObjectCategory::Plant => ObjectPart::Plant(PlantPart::Root),
        ObjectCategory::Effect => ObjectPart::Effect(EffectPart::Core),
        ObjectCategory::Prop => ObjectPart::Prop(hex_assets::PropPart::Structure),
    }
}

fn part_category(part: ObjectPart) -> ObjectCategory {
    match part {
        ObjectPart::Plant(_) => ObjectCategory::Plant,
        ObjectPart::Effect(_) => ObjectCategory::Effect,
        ObjectPart::Prop(_) => ObjectCategory::Prop,
    }
}

fn validate_part_category(
    category: ObjectCategory,
    part: ObjectPart,
) -> Result<(), EditorModelError> {
    if part_category(part) != category {
        return Err(EditorModelError::new(format!(
            "{part:?} cannot be used in a {category:?} object"
        )));
    }
    Ok(())
}

fn validate_part_for_position(
    category: ObjectCategory,
    part: ObjectPart,
    position: LocalVoxelCoord,
) -> Result<(), EditorModelError> {
    validate_part_category(category, part)?;
    if part == ObjectPart::Plant(PlantPart::Root) && position.level != 0 {
        return Err(EditorModelError::new(format!(
            "plant root {position:?} must remain at level 0"
        )));
    }
    Ok(())
}

fn validate_origin_part(
    category: ObjectCategory,
    connectivity: ConnectivityPolicy,
    origin: LocalVoxelCoord,
    part: ObjectPart,
) -> Result<(), EditorModelError> {
    let valid_part = match category {
        ObjectCategory::Plant => part == ObjectPart::Plant(PlantPart::Root),
        ObjectCategory::Effect => part == ObjectPart::Effect(EffectPart::Core),
        ObjectCategory::Prop => part == ObjectPart::Prop(hex_assets::PropPart::Structure),
    };
    if !valid_part {
        return Err(EditorModelError::new(format!(
            "{category:?} origin must use its root/core/structure part"
        )));
    }
    let grounded = matches!(category, ObjectCategory::Plant)
        || matches!(category, ObjectCategory::Prop) && connectivity == ConnectivityPolicy::Grounded;
    if grounded && origin.level != 0 {
        return Err(EditorModelError::new(
            "grounded object origin must remain at level 0",
        ));
    }
    Ok(())
}

fn validate_bounds(bounds: ObjectBounds) -> Result<(), EditorModelError> {
    if bounds.radius > MAX_OBJECT_RADIUS {
        return Err(EditorModelError::new(format!(
            "bounds radius {} exceeds the maximum {MAX_OBJECT_RADIUS}",
            bounds.radius
        )));
    }
    if bounds.height == 0 || bounds.height > MAX_OBJECT_HEIGHT {
        return Err(EditorModelError::new(format!(
            "bounds height must be within 1..={MAX_OBJECT_HEIGHT}"
        )));
    }
    bounds
        .min_level
        .checked_add(i32::from(bounds.height))
        .ok_or_else(|| EditorModelError::new("bounds level range overflows i32"))?;
    Ok(())
}

pub(crate) fn validate_position(
    object: &ObjectBlueprint,
    position: LocalVoxelCoord,
) -> Result<(), EditorModelError> {
    let axial_radius = position.axial().radius();
    if axial_radius > i64::from(object.bounds.radius) {
        return Err(EditorModelError::new(format!(
            "cell {position:?} is outside authoring radius {}: axial radius {axial_radius} is above the maximum; increase Radius under Document > Authoring bounds",
            object.bounds.radius,
        )));
    }

    let level = i64::from(position.level);
    let minimum = i64::from(object.bounds.min_level);
    let maximum = maximum_level(object.bounds);
    if level < minimum {
        return Err(EditorModelError::new(format!(
            "cell {position:?} is outside levels {minimum}..={maximum}: level {level} is below authoring minimum {minimum}; adjust Minimum level under Document > Authoring bounds"
        )));
    }
    if level > maximum {
        return Err(EditorModelError::new(format!(
            "cell {position:?} is outside levels {minimum}..={maximum}: level {level} is above authoring maximum {maximum}; increase Height under Document > Authoring bounds"
        )));
    }

    Ok(())
}

fn maximum_level(bounds: ObjectBounds) -> i64 {
    i64::from(bounds.min_level) + i64::from(bounds.height) - 1
}

fn placement_at(object: &ObjectBlueprint, position: LocalVoxelCoord) -> Option<&ObjectPlacement> {
    object
        .placements
        .iter()
        .find(|placement| placement.position == position)
}

fn placement_at_mut(
    object: &mut ObjectBlueprint,
    position: LocalVoxelCoord,
) -> Option<&mut ObjectPlacement> {
    object
        .placements
        .iter_mut()
        .find(|placement| placement.position == position)
}

fn set_membership<T>(values: &mut Vec<T>, value: T, enabled: bool)
where
    T: Copy + PartialEq,
{
    if enabled {
        if !values.contains(&value) {
            values.push(value);
        }
    } else {
        values.retain(|candidate| *candidate != value);
    }
}

fn repair_masks_after_content_change(object: &mut ObjectBlueprint) {
    let occupied: BTreeSet<_> = object
        .placements
        .iter()
        .map(|placement| placement.position)
        .collect();
    object
        .canopy_occluders
        .retain(|position| occupied.contains(position));

    match object.category {
        ObjectCategory::Plant => {
            object.blocker_footprint = object
                .placements
                .iter()
                .filter(|placement| {
                    placement.part == ObjectPart::Plant(PlantPart::Root)
                        && placement.position.level == 0
                })
                .map(|placement| placement.position.axial())
                .collect();
        }
        ObjectCategory::Effect => {
            object.blocker_footprint.clear();
            object.canopy_occluders.clear();
        }
        ObjectCategory::Prop => {
            object
                .blocker_footprint
                .retain(|blocker| occupied.iter().any(|position| position.axial() == *blocker));
            object.canopy_occluders.clear();
        }
    }
}

fn validate_transform(
    object: &ObjectBlueprint,
    selected: &BTreeSet<LocalVoxelCoord>,
    mapping: &BTreeMap<LocalVoxelCoord, LocalVoxelCoord>,
) -> Result<(), EditorModelError> {
    let occupied: BTreeSet<_> = object
        .placements
        .iter()
        .map(|placement| placement.position)
        .collect();
    let mut targets = BTreeSet::new();
    for (source, target) in mapping {
        validate_position(object, *target)?;
        if !targets.insert(*target) {
            return Err(EditorModelError::new(format!(
                "selection transform overlaps at {target:?}"
            )));
        }
        if occupied.contains(target) && !selected.contains(target) {
            return Err(EditorModelError::new(format!(
                "selection transform collides with occupied cell {target:?}"
            )));
        }
        let Some(placement) = placement_at(object, *source) else {
            return Err(EditorModelError::new(format!(
                "selection contains empty cell {source:?}"
            )));
        };
        validate_part_for_position(object.category, placement.part, *target)?;
    }
    Ok(())
}

fn transformed_prop_blockers(
    object: &ObjectBlueprint,
    selected: &BTreeSet<LocalVoxelCoord>,
    mapping: &BTreeMap<LocalVoxelCoord, LocalVoxelCoord>,
) -> Result<Option<Vec<LocalAxialCoord>>, EditorModelError> {
    if object.category != ObjectCategory::Prop {
        return Ok(None);
    }
    let mut blockers = BTreeSet::new();
    for blocker in &object.blocker_footprint {
        let selected_in_column: Vec<_> = selected
            .iter()
            .filter(|position| position.axial() == *blocker)
            .copied()
            .collect();
        let has_unselected = object.placements.iter().any(|placement| {
            placement.position.axial() == *blocker && !selected.contains(&placement.position)
        });
        if selected_in_column.is_empty() || has_unselected {
            blockers.insert(*blocker);
            continue;
        }
        let Some(target) = selected_in_column
            .iter()
            .find_map(|position| mapping.get(position))
        else {
            return Err(EditorModelError::new(format!(
                "cannot transform blocker {blocker:?} without its selected voxel"
            )));
        };
        blockers.insert(target.axial());
    }
    Ok(Some(blockers.into_iter().collect()))
}

fn checked_translate(
    position: LocalVoxelCoord,
    q: i32,
    r: i32,
    level: i32,
) -> Result<LocalVoxelCoord, EditorModelError> {
    let Some(q) = position.q.checked_add(q) else {
        return Err(EditorModelError::new("q coordinate overflowed"));
    };
    let Some(r) = position.r.checked_add(r) else {
        return Err(EditorModelError::new("r coordinate overflowed"));
    };
    let Some(level) = position.level.checked_add(level) else {
        return Err(EditorModelError::new("level coordinate overflowed"));
    };
    Ok(LocalVoxelCoord::new(q, r, level))
}

fn checked_translate_axial(
    position: LocalAxialCoord,
    q: i32,
    r: i32,
) -> Result<LocalAxialCoord, EditorModelError> {
    let Some(q) = position.q.checked_add(q) else {
        return Err(EditorModelError::new("q coordinate overflowed"));
    };
    let Some(r) = position.r.checked_add(r) else {
        return Err(EditorModelError::new("r coordinate overflowed"));
    };
    Ok(LocalAxialCoord::new(q, r))
}

fn checked_difference(
    position: LocalVoxelCoord,
    anchor: LocalVoxelCoord,
) -> Result<LocalVoxelCoord, EditorModelError> {
    let Some(q) = position.q.checked_sub(anchor.q) else {
        return Err(EditorModelError::new("clipboard q offset overflowed"));
    };
    let Some(r) = position.r.checked_sub(anchor.r) else {
        return Err(EditorModelError::new("clipboard r offset overflowed"));
    };
    let Some(level) = position.level.checked_sub(anchor.level) else {
        return Err(EditorModelError::new("clipboard level offset overflowed"));
    };
    Ok(LocalVoxelCoord::new(q, r, level))
}

fn checked_axial_difference(
    position: LocalAxialCoord,
    anchor: LocalAxialCoord,
) -> Result<LocalAxialCoord, EditorModelError> {
    let Some(q) = position.q.checked_sub(anchor.q) else {
        return Err(EditorModelError::new(
            "clipboard blocker q offset overflowed",
        ));
    };
    let Some(r) = position.r.checked_sub(anchor.r) else {
        return Err(EditorModelError::new(
            "clipboard blocker r offset overflowed",
        ));
    };
    Ok(LocalAxialCoord::new(q, r))
}

fn normalize_collections(object: &mut ObjectBlueprint) {
    object.placements.sort_by(|left, right| {
        left.position
            .cmp(&right.position)
            .then_with(|| left.style.as_str().cmp(right.style.as_str()))
            .then_with(|| left.part.cmp(&right.part))
    });
    object.blocker_footprint.sort_unstable();
    object.blocker_footprint.dedup();
    object.canopy_occluders.sort_unstable();
    object.canopy_occluders.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_assets::{
        ArtPalette, PaletteSwatch, PropPart, SrgbColor, SwatchId, VoxelStyle, VoxelSurfaceMode,
    };

    fn style_id(value: &str) -> VoxelStyleId {
        let Ok(id) = VoxelStyleId::new(value) else {
            unreachable!("test style id should be valid")
        };
        id
    }

    fn editor() -> EditorModel {
        let Ok(editor) = EditorModel::calibration_scene() else {
            unreachable!("calibration scene should be valid")
        };
        editor
    }

    fn style_catalog() -> VoxelStyleCatalog {
        let Ok(color) = SrgbColor::new(0.4, 0.6, 0.2) else {
            unreachable!("test colour should be valid")
        };
        let Ok(swatch_id) = SwatchId::new("calibration/green") else {
            unreachable!("test swatch id should be valid")
        };
        let Ok(swatch) = PaletteSwatch::new(
            "Calibration Green",
            color,
            BTreeSet::from(["editor".to_owned()]),
        ) else {
            unreachable!("test swatch should be valid")
        };
        let mut swatches = BTreeMap::new();
        swatches.insert(swatch_id.clone(), swatch);
        let Ok(palette) = ArtPalette::new(swatches) else {
            unreachable!("test palette should be valid")
        };
        let Ok(style) = VoxelStyle::new("Neutral", swatch_id, VoxelSurfaceMode::Opaque, 1.0, None)
        else {
            unreachable!("test style should be valid")
        };
        let mut styles = BTreeMap::new();
        styles.insert(style_id("calibration/neutral"), style);
        let Ok(catalog) = VoxelStyleCatalog::new(styles) else {
            unreachable!("test style catalog should be valid")
        };
        assert!(catalog.validate(&palette).is_ok());
        catalog
    }

    fn placement(editor: &EditorModel, position: LocalVoxelCoord) -> Option<&ObjectPlacement> {
        placement_at(editor.object(), position)
    }

    #[test]
    fn calibration_scene_is_intrinsically_valid_and_clean() {
        let editor = editor();
        assert!(editor.validate_draft().is_ok());
        assert!(!editor.is_dirty());
        assert_eq!(editor.object().bounds, ObjectBounds::DEFAULT);
        assert_eq!(editor.object().placements.len(), 1);
    }

    #[test]
    fn blank_documents_cover_every_supported_category_policy() {
        for (category, connectivity) in [
            (ObjectCategory::Plant, ConnectivityPolicy::Grounded),
            (ObjectCategory::Effect, ConnectivityPolicy::Free),
            (ObjectCategory::Prop, ConnectivityPolicy::Grounded),
            (ObjectCategory::Prop, ConnectivityPolicy::Free),
        ] {
            let created =
                EditorModel::blank(category, connectivity, style_id("calibration/neutral"));
            let Ok(created) = created else {
                unreachable!("supported blank document should be valid")
            };
            assert_eq!(created.object().category, category);
            assert_eq!(created.object().connectivity, connectivity);
            assert!(created.validate_draft().is_ok());
            assert!(created.is_dirty());
            if connectivity == ConnectivityPolicy::Free {
                assert!(created.object().bounds.min_level < 0);
            }
        }
        assert!(EditorModel::blank(
            ObjectCategory::Plant,
            ConnectivityPolicy::Free,
            style_id("calibration/neutral"),
        )
        .is_err());
        assert!(EditorModel::blank(
            ObjectCategory::Effect,
            ConnectivityPolicy::Grounded,
            style_id("calibration/neutral"),
        )
        .is_err());
    }

    #[test]
    fn empty_unsaved_documents_cannot_match_a_synthetic_saved_checkpoint() {
        for (category, connectivity) in [
            (ObjectCategory::Effect, ConnectivityPolicy::Free),
            (ObjectCategory::Prop, ConnectivityPolicy::Grounded),
            (ObjectCategory::Prop, ConnectivityPolicy::Free),
        ] {
            let mut editor =
                EditorModel::blank(category, connectivity, style_id("calibration/neutral"))
                    .expect("supported blank document should be valid");
            let origin = editor.object().origin;

            assert_eq!(editor.erase(origin), Ok(true));
            assert!(editor.object().placements.is_empty());
            assert!(editor.is_dirty());
        }
    }

    #[test]
    fn metadata_edits_are_validated_and_undoable() {
        let mut editor = editor();
        assert_eq!(editor.set_display_name("Young Oak"), Ok(true));
        assert_eq!(editor.object().display_name, "Young Oak");
        assert_eq!(
            editor.set_bounds(ObjectBounds {
                radius: 4,
                min_level: 0,
                height: 20,
            }),
            Ok(true)
        );
        assert_eq!(editor.object().bounds.radius, 4);
        assert_eq!(editor.undo(), Ok(true));
        assert_eq!(editor.object().bounds, ObjectBounds::DEFAULT);
        assert_eq!(editor.undo(), Ok(true));
        assert_eq!(editor.object().display_name, "Calibration Scene");

        assert!(editor.set_active_level(35).is_ok());
        assert_eq!(
            editor.set_bounds(ObjectBounds {
                radius: 6,
                min_level: 0,
                height: 1,
            }),
            Ok(true)
        );
        assert_eq!(editor.active_level(), 0);

        assert!(editor.set_display_name("  ").is_err());
        assert!(editor
            .set_bounds(ObjectBounds {
                radius: MAX_OBJECT_RADIUS + 1,
                min_level: 0,
                height: 1,
            })
            .is_err());
        assert_eq!(editor.object().display_name, "Calibration Scene");
    }

    #[test]
    fn authoring_bounds_accept_exact_edges_and_report_each_exceeded_dimension() {
        let mut editor = editor();
        let bounds = ObjectBounds {
            radius: 3,
            min_level: -2,
            height: 8,
        };
        assert_eq!(editor.set_bounds(bounds), Ok(true));

        for position in [
            LocalVoxelCoord::new(0, 3, -2),
            LocalVoxelCoord::new(0, 3, 5),
        ] {
            assert_eq!(validate_position(editor.object(), position), Ok(()));
        }

        let below = validate_position(editor.object(), LocalVoxelCoord::new(0, 3, -3))
            .expect_err("the level below the inclusive minimum must be rejected");
        assert!(below.message().contains("level -3"));
        assert!(below.message().contains("below authoring minimum -2"));
        assert!(below.message().contains("levels -2..=5"));
        assert!(below.message().contains("Document > Authoring bounds"));

        let above = validate_position(editor.object(), LocalVoxelCoord::new(0, 3, 6))
            .expect_err("the level above the inclusive maximum must be rejected");
        assert!(above.message().contains("level 6"));
        assert!(above.message().contains("above authoring maximum 5"));
        assert!(above.message().contains("levels -2..=5"));
        assert!(above.message().contains("Document > Authoring bounds"));

        let outside_radius = validate_position(editor.object(), LocalVoxelCoord::new(0, 4, 0))
            .expect_err("the cell beyond the inclusive axial radius must be rejected");
        assert!(outside_radius
            .message()
            .contains("outside authoring radius 3"));
        assert!(outside_radius.message().contains("axial radius 4"));
        assert!(outside_radius
            .message()
            .contains("Document > Authoring bounds"));
    }

    #[test]
    fn save_as_adopts_the_persistent_identity_without_an_undo_step() {
        let mut editor = editor();
        assert_eq!(editor.set_display_name("Young Oak"), Ok(true));
        let Ok(saved_id) = ObjectAssetId::new("plant/young-oak") else {
            unreachable!("test object id should be valid")
        };
        editor.mark_saved_as(saved_id.clone());
        assert_eq!(&editor.object().id, &saved_id);
        assert!(!editor.is_dirty());
        assert_eq!(editor.undo(), Ok(false));
    }

    #[test]
    fn origin_and_connectivity_follow_category_contracts() {
        let mut editor = editor();
        let trunk = LocalVoxelCoord::new(0, 0, 1);
        assert_eq!(
            editor.place(
                trunk,
                style_id("calibration/neutral"),
                ObjectPart::Plant(PlantPart::Trunk),
            ),
            Ok(true)
        );
        assert!(editor.set_origin(trunk).is_err());
        assert!(editor.set_connectivity(ConnectivityPolicy::Free).is_err());
        assert_eq!(editor.object().origin, LocalVoxelCoord::new(0, 0, 0));
        assert_eq!(editor.undo(), Ok(true));
        assert!(placement(&editor, trunk).is_none());
    }

    #[test]
    fn place_erase_repaint_and_part_changes_are_undoable() {
        let mut editor = editor();
        let position = LocalVoxelCoord::new(0, 0, 1);
        assert_eq!(
            editor.place(
                position,
                style_id("calibration/neutral"),
                ObjectPart::Plant(PlantPart::Trunk),
            ),
            Ok(true)
        );
        assert_eq!(
            editor.repaint(position, style_id("calibration/alternate")),
            Ok(true)
        );
        assert_eq!(
            editor.change_part(position, ObjectPart::Plant(PlantPart::Foliage)),
            Ok(true)
        );
        assert_eq!(editor.set_canopy_occluder(position, true), Ok(true));
        let Some(painted) = placement(&editor, position) else {
            unreachable!("placed cell should exist")
        };
        assert_eq!(painted.style.as_str(), "calibration/alternate");
        assert!(editor.object().canopy_occluders.contains(&position));

        assert_eq!(editor.erase(position), Ok(true));
        assert!(placement(&editor, position).is_none());
        assert!(!editor.object().canopy_occluders.contains(&position));
        assert_eq!(editor.undo(), Ok(true));
        assert!(placement(&editor, position).is_some());
    }

    #[test]
    fn a_drag_stroke_is_one_history_entry() {
        let mut editor = editor();
        assert!(editor.begin_transaction("Paint stroke").is_ok());
        for level in 1..=3 {
            assert_eq!(
                editor.place(
                    LocalVoxelCoord::new(0, 0, level),
                    style_id("calibration/neutral"),
                    ObjectPart::Plant(PlantPart::Trunk),
                ),
                Ok(true)
            );
        }
        assert_eq!(editor.commit_transaction(), Ok(true));
        assert_eq!(editor.object().placements.len(), 4);
        assert_eq!(editor.undo(), Ok(true));
        assert_eq!(editor.object().placements.len(), 1);
        assert_eq!(editor.redo(), Ok(true));
        assert_eq!(editor.object().placements.len(), 4);
    }

    #[test]
    fn cancelling_a_failed_drag_restores_the_complete_baseline() {
        let mut editor = editor();
        let original = editor.object().clone();
        assert!(editor.begin_transaction("Paint stroke").is_ok());
        assert_eq!(
            editor.place(
                LocalVoxelCoord::new(0, 0, 1),
                style_id("calibration/neutral"),
                ObjectPart::Plant(PlantPart::Trunk),
            ),
            Ok(true)
        );
        assert!(editor
            .place(
                LocalVoxelCoord::new(7, 0, 1),
                style_id("calibration/neutral"),
                ObjectPart::Plant(PlantPart::Trunk),
            )
            .is_err());
        assert_eq!(editor.cancel_transaction(), Ok(()));
        assert_eq!(editor.object(), &original);
        assert!(!editor.is_transaction_open());
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "transaction command mutated editor state before returning an error")]
    fn transaction_command_that_mutates_before_failure_violates_debug_invariant() {
        let mut editor = editor();
        assert!(editor.begin_transaction("Paint stroke").is_ok());

        let _result = editor.edit("Deliberately failing edit", |object, selection| {
            object.display_name = "Leaked partial mutation".to_owned();
            object.placements.clear();
            selection.cells.clear();
            Err(EditorModelError::new("deliberate test failure"))
        });
    }

    #[test]
    fn invalid_operations_are_atomic_and_explain_the_failure() {
        let mut editor = editor();
        let original = editor.object().clone();
        let result = editor.place(
            LocalVoxelCoord::new(7, 0, 0),
            style_id("calibration/neutral"),
            ObjectPart::Plant(PlantPart::Root),
        );
        let Err(error) = result else {
            unreachable!("out-of-bounds placement should fail")
        };
        assert!(error.message().contains("outside"));
        assert_eq!(editor.object(), &original);

        let wrong_part = editor.change_part(
            LocalVoxelCoord::new(0, 0, 0),
            ObjectPart::Effect(EffectPart::Core),
        );
        assert!(wrong_part.is_err());
        assert_eq!(editor.object(), &original);
    }

    #[test]
    fn selection_supports_replace_and_additive_modes() {
        let mut editor = editor();
        let upper = LocalVoxelCoord::new(0, 0, 1);
        assert!(editor
            .place(
                upper,
                style_id("calibration/neutral"),
                ObjectPart::Plant(PlantPart::Trunk),
            )
            .is_ok());

        assert!(editor.select(LocalVoxelCoord::new(0, 0, 0), false));
        assert_eq!(editor.selection().len(), 1);
        assert!(editor.select(upper, true));
        assert_eq!(editor.selection().len(), 2);
        assert!(editor.select(LocalVoxelCoord::new(3, 3, 3), false));
        assert!(editor.selection().is_empty());
    }

    #[test]
    fn nudge_rejects_collision_without_partially_moving_selection() {
        let mut editor = editor();
        let lower = LocalVoxelCoord::new(0, 0, 0);
        let upper = LocalVoxelCoord::new(0, 0, 1);
        assert!(editor
            .place(
                upper,
                style_id("calibration/neutral"),
                ObjectPart::Plant(PlantPart::Trunk),
            )
            .is_ok());
        assert!(editor.select(lower, false));
        let before = editor.object().clone();

        let result = editor.nudge_selection(0, 0, 1);
        assert!(result.is_err());
        assert_eq!(editor.object(), &before);
        assert!(editor.selection().contains(lower));
    }

    #[test]
    fn copy_paste_preserves_styles_parts_and_canopy() {
        let mut editor = editor();
        let foliage = LocalVoxelCoord::new(0, 0, 1);
        assert!(editor
            .place(
                foliage,
                style_id("calibration/green"),
                ObjectPart::Plant(PlantPart::Foliage),
            )
            .is_ok());
        assert!(editor.set_canopy_occluder(foliage, true).is_ok());
        assert!(editor.select(foliage, false));
        assert_eq!(editor.copy_selection(), Ok(1));

        let target = LocalVoxelCoord::new(1, 0, 1);
        assert_eq!(editor.paste_at(target), Ok(true));
        let Some(pasted) = placement(&editor, target) else {
            unreachable!("pasted voxel should exist")
        };
        assert_eq!(pasted.style.as_str(), "calibration/green");
        assert_eq!(pasted.part, ObjectPart::Plant(PlantPart::Foliage));
        assert!(editor.object().canopy_occluders.contains(&target));
        assert!(editor.selection().contains(target));
    }

    #[test]
    fn delete_can_temporarily_invalidate_draft_but_save_rejects_it() {
        let mut editor = editor();
        let origin = editor.object().origin;
        assert!(editor.select(origin, false));
        assert_eq!(editor.delete_selection(), Ok(true));
        assert!(editor.object().placements.is_empty());
        assert!(editor.validate_draft().is_err());
        assert!(editor.blueprint_for_save(&style_catalog()).is_err());
        assert_eq!(editor.undo(), Ok(true));
        assert!(editor.blueprint_for_save(&style_catalog()).is_ok());
    }

    #[test]
    fn six_clockwise_rotations_return_exact_coordinates() {
        let mut editor = editor();
        let branch = LocalVoxelCoord::new(1, -1, 1);
        assert!(editor
            .place(
                branch,
                style_id("calibration/neutral"),
                ObjectPart::Plant(PlantPart::Branch),
            )
            .is_ok());
        assert!(editor.select(branch, false));
        let original = editor.object().clone();
        let pivot = editor.object().origin;
        for _ in 0..6 {
            assert_eq!(editor.rotate_selection_clockwise(pivot), Ok(true));
        }
        assert_eq!(editor.object(), &original);
        assert!(editor.selection().contains(branch));
    }

    #[test]
    fn prop_blocker_moves_with_a_fully_selected_column() {
        let id = ObjectAssetId::new("calibration/prop");
        let Ok(id) = id else {
            unreachable!("test object id should be valid")
        };
        let origin = LocalVoxelCoord::new(0, 0, 0);
        let object = ObjectBlueprint {
            schema_version: OBJECT_BLUEPRINT_SCHEMA_VERSION,
            id,
            display_name: "Calibration Prop".to_owned(),
            category: ObjectCategory::Prop,
            bounds: ObjectBounds::DEFAULT,
            connectivity: ConnectivityPolicy::Grounded,
            origin,
            placements: vec![ObjectPlacement {
                position: origin,
                style: style_id("calibration/neutral"),
                part: ObjectPart::Prop(PropPart::Structure),
            }],
            blocker_footprint: vec![origin.axial()],
            canopy_occluders: Vec::new(),
        };
        let Ok(mut editor) = EditorModel::from_blueprint(object) else {
            unreachable!("test prop should be valid")
        };
        assert!(editor.select(origin, false));
        assert_eq!(editor.nudge_selection(1, 0, 0), Ok(true));
        assert_eq!(
            editor.object().blocker_footprint,
            [LocalAxialCoord::new(1, 0)]
        );
        assert_eq!(editor.object().origin, LocalVoxelCoord::new(1, 0, 0));
    }

    #[test]
    fn prop_clipboard_carries_a_blocker_only_for_a_fully_selected_column() {
        let Ok(id) = ObjectAssetId::new("calibration/prop") else {
            unreachable!("test object id should be valid")
        };
        let origin = LocalVoxelCoord::new(0, 0, 0);
        let detail = LocalVoxelCoord::new(0, 0, 1);
        let object = ObjectBlueprint {
            schema_version: OBJECT_BLUEPRINT_SCHEMA_VERSION,
            id,
            display_name: "Calibration Prop".to_owned(),
            category: ObjectCategory::Prop,
            bounds: ObjectBounds::DEFAULT,
            connectivity: ConnectivityPolicy::Grounded,
            origin,
            placements: vec![
                ObjectPlacement {
                    position: origin,
                    style: style_id("calibration/neutral"),
                    part: ObjectPart::Prop(PropPart::Structure),
                },
                ObjectPlacement {
                    position: detail,
                    style: style_id("calibration/neutral"),
                    part: ObjectPart::Prop(PropPart::Detail),
                },
            ],
            blocker_footprint: vec![origin.axial()],
            canopy_occluders: Vec::new(),
        };
        let Ok(mut editor) = EditorModel::from_blueprint(object) else {
            unreachable!("test prop should be valid")
        };

        assert!(editor.select(detail, false));
        assert_eq!(editor.copy_selection(), Ok(1));
        assert_eq!(editor.paste_at(LocalVoxelCoord::new(1, 0, 1)), Ok(true));
        assert!(!editor
            .object()
            .blocker_footprint
            .contains(&LocalAxialCoord::new(1, 0)));

        assert!(editor.select(origin, false));
        assert!(editor.select(detail, true));
        assert_eq!(editor.copy_selection(), Ok(2));
        assert_eq!(editor.paste_at(LocalVoxelCoord::new(2, 0, 0)), Ok(true));
        assert!(editor
            .object()
            .blocker_footprint
            .contains(&LocalAxialCoord::new(2, 0)));
    }
}
