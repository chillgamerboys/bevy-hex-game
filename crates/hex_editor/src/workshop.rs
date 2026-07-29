//! Session-wide drafts and undo ordering across catalogs and object edits.

use std::collections::VecDeque;
use std::fmt;

use bevy::prelude::Resource;
use hex_assets::{
    ArtPalette, ObjectAssetId, PaletteSwatch, SwatchId, VoxelStyle, VoxelStyleCatalog, VoxelStyleId,
};

use crate::model::{EditorModel, EditorModelError};

const GLOBAL_HISTORY_LIMIT: usize = 128;

/// A recoverable workshop draft operation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkshopDraftError {
    detail: String,
}

impl WorkshopDraftError {
    fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }

    /// Human-readable failure detail.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for WorkshopDraftError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for WorkshopDraftError {}

impl From<EditorModelError> for WorkshopDraftError {
    fn from(error: EditorModelError) -> Self {
        Self::new(error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq)]
struct CatalogSnapshot {
    palette: ArtPalette,
    styles: VoxelStyleCatalog,
}

#[derive(Debug, Clone, PartialEq)]
enum GlobalHistoryKind {
    Object,
    Catalog {
        before: CatalogSnapshot,
        after: CatalogSnapshot,
    },
}

#[derive(Debug, Clone, PartialEq)]
struct GlobalHistoryEntry {
    label: String,
    kind: GlobalHistoryKind,
}

/// Mutable session state shared by both authoring modes.
///
/// The inner object model keeps compact object snapshots. This coordinator records
/// the global order of object and catalog commands so Undo always addresses the
/// most recent edit regardless of which mode produced it.
#[derive(Resource, Debug, Clone)]
pub struct WorkshopDraft {
    palette: ArtPalette,
    styles: VoxelStyleCatalog,
    saved_palette: ArtPalette,
    saved_styles: VoxelStyleCatalog,
    editor: EditorModel,
    undo: VecDeque<GlobalHistoryEntry>,
    redo: VecDeque<GlobalHistoryEntry>,
    open_transaction_label: Option<String>,
}

impl WorkshopDraft {
    /// Creates a clean catalog draft around one open object document.
    #[must_use]
    pub fn new(palette: ArtPalette, styles: VoxelStyleCatalog, editor: EditorModel) -> Self {
        Self {
            saved_palette: palette.clone(),
            saved_styles: styles.clone(),
            palette,
            styles,
            editor,
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            open_transaction_label: None,
        }
    }

    /// Current palette draft.
    #[must_use]
    pub const fn palette(&self) -> &ArtPalette {
        &self.palette
    }

    /// Current voxel-style draft.
    #[must_use]
    pub const fn styles(&self) -> &VoxelStyleCatalog {
        &self.styles
    }

    /// Current object editor.
    #[must_use]
    pub const fn editor(&self) -> &EditorModel {
        &self.editor
    }

    /// Mutable object editor for selection and non-semantic active-tool changes.
    pub fn editor_mut_untracked(&mut self) -> &mut EditorModel {
        &mut self.editor
    }

    /// Whether the palette differs from its last explicit save.
    #[must_use]
    pub fn is_palette_dirty(&self) -> bool {
        self.palette != self.saved_palette
    }

    /// Whether the style catalog differs from its last explicit save.
    #[must_use]
    pub fn is_styles_dirty(&self) -> bool {
        self.styles != self.saved_styles
    }

    /// Whether either shared catalog has unsaved changes.
    #[must_use]
    pub fn are_catalogs_dirty(&self) -> bool {
        self.is_palette_dirty() || self.is_styles_dirty()
    }

    /// Whether any tracked workshop document has unsaved semantic changes.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.are_catalogs_dirty() || self.editor.is_dirty()
    }

    /// Applies one atomic object command and records it in global undo order.
    pub fn edit_object(
        &mut self,
        label: impl Into<String>,
        operation: impl FnOnce(&mut EditorModel) -> Result<bool, EditorModelError>,
    ) -> Result<bool, WorkshopDraftError> {
        if self.open_transaction_label.is_some() {
            return Err(WorkshopDraftError::new(
                "use the active object transaction for grouped edits",
            ));
        }
        let label = validated_label(label)?;
        let changed = operation(&mut self.editor)?;
        if changed {
            self.push_undo(GlobalHistoryEntry {
                label,
                kind: GlobalHistoryKind::Object,
            });
            self.redo.clear();
        }
        Ok(changed)
    }

    /// Begins one grouped object command such as a paint stroke.
    pub fn begin_object_transaction(
        &mut self,
        label: impl Into<String>,
    ) -> Result<(), WorkshopDraftError> {
        if self.open_transaction_label.is_some() {
            return Err(WorkshopDraftError::new(
                "another workshop transaction is already open",
            ));
        }
        let label = validated_label(label)?;
        self.editor.begin_transaction(label.clone())?;
        self.open_transaction_label = Some(label);
        Ok(())
    }

    /// Commits the active grouped object command as one global undo entry.
    pub fn commit_object_transaction(&mut self) -> Result<bool, WorkshopDraftError> {
        let Some(label) = self.open_transaction_label.take() else {
            return Err(WorkshopDraftError::new("no workshop transaction is open"));
        };
        let changed = self.editor.commit_transaction()?;
        if changed {
            self.push_undo(GlobalHistoryEntry {
                label,
                kind: GlobalHistoryKind::Object,
            });
            self.redo.clear();
        }
        Ok(changed)
    }

    /// Cancels the active grouped object command and restores its baseline.
    pub fn cancel_object_transaction(&mut self) -> Result<(), WorkshopDraftError> {
        if self.open_transaction_label.take().is_none() {
            return Err(WorkshopDraftError::new("no workshop transaction is open"));
        }
        self.editor.cancel_transaction()?;
        Ok(())
    }

    /// Inserts or updates a palette swatch in the draft.
    ///
    /// A new or edited near-duplicate requires explicit confirmation. Updating the
    /// existing swatch at `id` does not compare that swatch against itself.
    pub fn upsert_swatch(
        &mut self,
        id: SwatchId,
        swatch: PaletteSwatch,
        confirmed_near_color: bool,
    ) -> Result<bool, WorkshopDraftError> {
        if !confirmed_near_color
            && !self
                .palette
                .near_duplicates(swatch.color(), Some(&id))
                .is_empty()
        {
            return Err(WorkshopDraftError::new(
                "the swatch is within the palette near-colour threshold; confirm it explicitly",
            ));
        }
        let before = self.catalog_snapshot();
        let mut palette = self.palette.clone();
        drop(
            palette
                .insert(id, swatch)
                .map_err(|error| WorkshopDraftError::new(error.to_string()))?,
        );
        self.styles
            .validate(&palette)
            .map_err(|error| WorkshopDraftError::new(error.to_string()))?;
        if palette == self.palette {
            return Ok(false);
        }
        self.palette = palette;
        self.record_catalog_edit("Change palette swatch", before);
        Ok(true)
    }

    /// Removes an unreferenced swatch from the draft.
    pub fn delete_swatch(&mut self, id: &SwatchId) -> Result<bool, WorkshopDraftError> {
        let references = self.styles.references_to(id);
        if !references.is_empty() {
            let ids = references
                .iter()
                .map(VoxelStyleId::as_str)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(WorkshopDraftError::new(format!(
                "swatch '{}' is still referenced by styles [{ids}]",
                id.as_str()
            )));
        }
        let before = self.catalog_snapshot();
        let removed = self
            .palette
            .remove(id)
            .map_err(|error| WorkshopDraftError::new(error.to_string()))?;
        if removed.is_none() {
            return Ok(false);
        }
        self.record_catalog_edit("Delete palette swatch", before);
        Ok(true)
    }

    /// Inserts or updates a reusable style in the draft.
    pub fn upsert_style(
        &mut self,
        id: VoxelStyleId,
        style: VoxelStyle,
    ) -> Result<bool, WorkshopDraftError> {
        let before = self.catalog_snapshot();
        let mut styles = self.styles.clone();
        drop(
            styles
                .insert(id, style)
                .map_err(|error| WorkshopDraftError::new(error.to_string()))?,
        );
        styles
            .validate(&self.palette)
            .map_err(|error| WorkshopDraftError::new(error.to_string()))?;
        if styles == self.styles {
            return Ok(false);
        }
        self.styles = styles;
        self.record_catalog_edit("Change voxel style", before);
        Ok(true)
    }

    /// Removes a style when the open object does not reference it.
    ///
    /// The application must also check other saved objects through
    /// [`crate::project::AssetProject::style_usage`] before calling this method.
    pub fn delete_style(&mut self, id: &VoxelStyleId) -> Result<bool, WorkshopDraftError> {
        if self
            .editor
            .object()
            .placements
            .iter()
            .any(|placement| &placement.style == id)
        {
            return Err(WorkshopDraftError::new(format!(
                "style '{}' is used by the open object",
                id.as_str()
            )));
        }
        let before = self.catalog_snapshot();
        if self.styles.remove(id).is_none() {
            return Ok(false);
        }
        self.record_catalog_edit("Delete voxel style", before);
        Ok(true)
    }

    /// Undoes the latest object or catalog command, regardless of mode.
    pub fn undo(&mut self) -> Result<bool, WorkshopDraftError> {
        self.ensure_no_transaction("undo")?;
        let Some(entry) = self.undo.pop_back() else {
            return Ok(false);
        };
        match &entry.kind {
            GlobalHistoryKind::Object => {
                if !self.editor.undo()? {
                    self.undo.push_back(entry);
                    return Err(WorkshopDraftError::new(
                        "global object history diverged from the object document",
                    ));
                }
            }
            GlobalHistoryKind::Catalog { before, .. } => {
                self.restore_catalog_snapshot(before.clone());
            }
        }
        self.push_redo(entry);
        Ok(true)
    }

    /// Redoes the latest globally undone command.
    pub fn redo(&mut self) -> Result<bool, WorkshopDraftError> {
        self.ensure_no_transaction("redo")?;
        let Some(entry) = self.redo.pop_back() else {
            return Ok(false);
        };
        match &entry.kind {
            GlobalHistoryKind::Object => {
                if !self.editor.redo()? {
                    self.redo.push_back(entry);
                    return Err(WorkshopDraftError::new(
                        "global object history diverged from the object document",
                    ));
                }
            }
            GlobalHistoryKind::Catalog { after, .. } => {
                self.restore_catalog_snapshot(after.clone());
            }
        }
        self.push_undo(entry);
        Ok(true)
    }

    /// Label of the next globally undoable command.
    #[must_use]
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.back().map(|entry| entry.label.as_str())
    }

    /// Label of the next globally redoable command.
    #[must_use]
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.back().map(|entry| entry.label.as_str())
    }

    /// Marks both catalog drafts as explicitly saved.
    pub fn mark_catalogs_saved(&mut self) {
        self.saved_palette = self.palette.clone();
        self.saved_styles = self.styles.clone();
    }

    /// Marks the object as explicitly saved under its current identity.
    pub fn mark_object_saved(&mut self) {
        self.editor.mark_saved();
    }

    /// Adopts a successful Save As identity and clears incompatible history.
    pub fn mark_object_saved_as(&mut self, id: ObjectAssetId) {
        self.editor.mark_saved_as(id);
        self.clear_history();
    }

    /// Replaces the open object and starts a new document history.
    pub fn open_object(&mut self, editor: EditorModel) {
        self.editor = editor;
        self.clear_history();
    }

    fn record_catalog_edit(&mut self, label: &str, before: CatalogSnapshot) {
        let after = self.catalog_snapshot();
        self.push_undo(GlobalHistoryEntry {
            label: label.to_owned(),
            kind: GlobalHistoryKind::Catalog { before, after },
        });
        self.redo.clear();
    }

    fn catalog_snapshot(&self) -> CatalogSnapshot {
        CatalogSnapshot {
            palette: self.palette.clone(),
            styles: self.styles.clone(),
        }
    }

    fn restore_catalog_snapshot(&mut self, snapshot: CatalogSnapshot) {
        self.palette = snapshot.palette;
        self.styles = snapshot.styles;
    }

    fn ensure_no_transaction(&self, operation: &str) -> Result<(), WorkshopDraftError> {
        if self.open_transaction_label.is_some() {
            return Err(WorkshopDraftError::new(format!(
                "cannot {operation} while an object transaction is open"
            )));
        }
        Ok(())
    }

    fn clear_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.open_transaction_label = None;
    }

    fn push_undo(&mut self, entry: GlobalHistoryEntry) {
        self.undo.push_back(entry);
        trim_history(&mut self.undo);
    }

    fn push_redo(&mut self, entry: GlobalHistoryEntry) {
        self.redo.push_back(entry);
        trim_history(&mut self.redo);
    }
}

fn validated_label(label: impl Into<String>) -> Result<String, WorkshopDraftError> {
    let label = label.into();
    if label.trim().is_empty() {
        return Err(WorkshopDraftError::new(
            "workshop history labels cannot be empty",
        ));
    }
    Ok(label)
}

fn trim_history(history: &mut VecDeque<GlobalHistoryEntry>) {
    while history.len() > GLOBAL_HISTORY_LIMIT {
        drop(history.pop_front());
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use hex_assets::{ObjectCategory, SrgbColor, VoxelSurfaceMode};

    use super::*;
    use crate::model::EditorModel;

    fn swatch_id(value: &str) -> SwatchId {
        SwatchId::new(value).expect("test swatch id should be valid")
    }

    fn style_id(value: &str) -> VoxelStyleId {
        VoxelStyleId::new(value).expect("test style id should be valid")
    }

    fn fixture() -> WorkshopDraft {
        let base_id = swatch_id("editor/neutral");
        let swatch = PaletteSwatch::new(
            "Editor Neutral",
            SrgbColor::new(0.5, 0.5, 0.5).expect("test colour should be valid"),
            BTreeSet::from(["editor".to_owned()]),
        )
        .expect("test swatch should be valid");
        let palette = ArtPalette::new(BTreeMap::from([(base_id.clone(), swatch)]))
            .expect("test palette should be valid");
        let style = VoxelStyle::new(
            "Editor Neutral",
            base_id,
            VoxelSurfaceMode::Opaque,
            1.0,
            None,
        )
        .expect("test style should be valid");
        let style_id = style_id("editor/neutral");
        let styles = VoxelStyleCatalog::new(BTreeMap::from([(style_id.clone(), style)]))
            .expect("test catalog should be valid");
        let editor = EditorModel::blank(
            ObjectCategory::Plant,
            hex_assets::ConnectivityPolicy::Grounded,
            style_id,
        )
        .expect("blank test document should be valid");
        WorkshopDraft::new(palette, styles, editor)
    }

    #[test]
    fn global_history_orders_catalog_and_object_commands() {
        let mut draft = fixture();
        let accent_id = swatch_id("plant/accent");
        let accent = PaletteSwatch::new(
            "Accent",
            SrgbColor::new(0.9, 0.1, 0.2).expect("test colour should be valid"),
            BTreeSet::from(["plant".to_owned()]),
        )
        .expect("test swatch should be valid");
        assert_eq!(
            draft.upsert_swatch(accent_id.clone(), accent, false),
            Ok(true)
        );
        assert_eq!(
            draft.edit_object("Rename object", |editor| {
                editor.set_display_name("History Plant")
            }),
            Ok(true)
        );
        assert_eq!(draft.undo_label(), Some("Rename object"));

        assert_eq!(draft.undo(), Ok(true));
        assert_eq!(draft.editor().object().display_name, "Untitled Plant");
        assert_eq!(draft.undo(), Ok(true));
        assert!(!draft.palette().contains(&accent_id));
        assert_eq!(draft.redo(), Ok(true));
        assert!(draft.palette().contains(&accent_id));
        assert_eq!(draft.redo(), Ok(true));
        assert_eq!(draft.editor().object().display_name, "History Plant");
    }

    #[test]
    fn invalid_object_history_labels_are_rejected_before_mutation() {
        let mut draft = fixture();
        let before = draft.editor().clone();

        let result = draft.edit_object("  ", |editor| {
            editor.set_display_name("Must not be applied")
        });

        assert!(result.is_err());
        assert_eq!(draft.editor(), &before);
        assert_eq!(draft.undo_label(), None);
    }

    #[test]
    fn catalog_dirty_state_changes_only_at_explicit_checkpoint() {
        let mut draft = fixture();
        assert!(!draft.are_catalogs_dirty());
        let id = swatch_id("plant/accent");
        let swatch = PaletteSwatch::new(
            "Accent",
            SrgbColor::new(0.9, 0.1, 0.2).expect("test colour should be valid"),
            BTreeSet::from(["plant".to_owned()]),
        )
        .expect("test swatch should be valid");
        assert_eq!(draft.upsert_swatch(id, swatch, false), Ok(true));
        assert!(draft.are_catalogs_dirty());
        draft.mark_catalogs_saved();
        assert!(!draft.are_catalogs_dirty());
        assert_eq!(draft.undo(), Ok(true));
        assert!(draft.are_catalogs_dirty());
    }

    #[test]
    fn near_colour_confirmation_excludes_self_but_covers_edits() {
        let mut draft = fixture();
        let changed = PaletteSwatch::new(
            "Editor Neutral Renamed",
            SrgbColor::new(0.5, 0.5, 0.5).expect("test colour should be valid"),
            BTreeSet::from(["editor".to_owned()]),
        )
        .expect("test swatch should be valid");
        assert_eq!(
            draft.upsert_swatch(swatch_id("editor/neutral"), changed, false),
            Ok(true)
        );

        let close = PaletteSwatch::new(
            "Very Close",
            SrgbColor::new(0.5, 0.5, 0.5).expect("test colour should be valid"),
            BTreeSet::from(["editor".to_owned()]),
        )
        .expect("test swatch should be valid");
        let error = draft
            .upsert_swatch(swatch_id("editor/near"), close.clone(), false)
            .expect_err("near-colour insertion should require confirmation");
        assert_eq!(
            error.detail(),
            "the swatch is within the palette near-colour threshold; confirm it explicitly"
        );
        assert_eq!(
            draft.upsert_swatch(swatch_id("editor/near"), close, true),
            Ok(true)
        );

        let second_id = swatch_id("plant/second");
        let second = PaletteSwatch::new(
            "Second",
            SrgbColor::new(0.8, 0.2, 0.2).expect("test colour should be valid"),
            BTreeSet::from(["plant".to_owned()]),
        )
        .expect("test swatch should be valid");
        assert_eq!(
            draft.upsert_swatch(second_id.clone(), second, false),
            Ok(true)
        );
        let close_to_neutral = PaletteSwatch::new(
            "Second Near Neutral",
            SrgbColor::new(0.5, 0.5, 0.5).expect("test colour should be valid"),
            BTreeSet::from(["plant".to_owned()]),
        )
        .expect("test swatch should be valid");
        let error = draft
            .upsert_swatch(second_id.clone(), close_to_neutral.clone(), false)
            .expect_err("near-colour edit should require confirmation");
        assert_eq!(
            error.detail(),
            "the swatch is within the palette near-colour threshold; confirm it explicitly"
        );
        assert_eq!(
            draft.upsert_swatch(second_id, close_to_neutral, true),
            Ok(true)
        );
    }
}
