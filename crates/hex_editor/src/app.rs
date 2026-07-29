//! Application composition and adapters between UI, draft state, and viewport.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use bevy::window::{
    PresentMode, PrimaryWindow, WindowCloseRequested, WindowResizeConstraints, WindowResolution,
};
use bevy_egui::EguiPlugin;
use hex_assets::{
    ArtPalette, ConnectivityPolicy, LocalVoxelCoord, ObjectAssetId, ObjectCategory, SwatchId,
    VoxelStyleCatalog, VoxelStyleId,
};

use crate::launch::resolve_repository_root;
use crate::model::{validate_position, EditorModel, EditorTool, PreviewRig, WorkshopMode};
use crate::project::{AssetProject, ExternalAssetChange, ProjectRevisionSet};
use crate::recovery::{RecoveryDocument, RecoveryEnvelope, RecoveryStore, RecoveryWorkshopDraft};
use crate::review::{ReviewPresentation, ReviewPublishOutcome, ReviewReport, REVIEW_FRAME_SPECS};
use crate::review_capture::{ReviewCaptureFinished, ReviewCaptureRejected, ReviewCaptureRequest};
use crate::ui::{
    EditorCameraSnap, RecoveryPrompt, WorkshopDocumentState, WorkshopStatus, WorkshopStatusKind,
    WorkshopUiAction, WorkshopUiSnapshot,
};
use crate::viewport::{
    CameraSnap, CameraSnapRequest, FrameViewportRequest, HoveredFaceTarget, RenderedVoxel,
    ViewportContent, ViewportContentUpdate, ViewportEmission, ViewportFaceTarget,
    ViewportInputEnabled, ViewportMode, ViewportPickSource, ViewportPreviewRig, ViewportStyle,
    ViewportSystems,
};
use crate::workshop::WorkshopDraft;

const RECOVERY_IDLE_SECONDS: f64 = 3.0;
const RECOVERY_MAX_INTERVAL_SECONDS: f64 = 30.0;
const POINTER_STROKE_DRAG_THRESHOLD: f32 = 4.0;

#[derive(Debug, Clone, PartialEq, Eq)]
enum OpenDocument {
    Calibration,
    Unsaved(ObjectAssetId),
    Saved(ObjectAssetId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PreviewSubject {
    ActiveStyle,
    Swatch(SwatchId),
    Style(VoxelStyleId),
}

#[derive(Debug)]
enum PendingRecovery {
    Available(Box<RecoveryEnvelope>),
    Invalid(String),
}

#[derive(Debug, Clone, PartialEq)]
struct RecoverableSession {
    document: RecoveryDocument,
    workshop: RecoveryWorkshopDraft,
}

#[derive(Debug, Default)]
struct RecoveryAutosave {
    last_observed: Option<RecoverableSession>,
    last_written: Option<RecoverableSession>,
    dirty_since_seconds: Option<f64>,
    last_change_seconds: Option<f64>,
    next_retry_seconds: f64,
}

#[derive(Debug, Clone, Copy, Default)]
struct OverlaySettings {
    blockers: bool,
    canopy: bool,
    semantics: bool,
    isolate_active_level: bool,
    grid: bool,
}

#[derive(Resource)]
struct WorkshopRuntime {
    project: Option<AssetProject>,
    draft: Option<WorkshopDraft>,
    document: OpenDocument,
    preview: PreviewSubject,
    overlays: OverlaySettings,
    status: Option<WorkshopStatus>,
    load_failure: Option<String>,
    external_changes: Vec<ExternalAssetChange>,
    recovery_store: Option<RecoveryStore>,
    pending_recovery: Option<PendingRecovery>,
    recovery_base_revisions: Option<ProjectRevisionSet>,
    recovery_conflict: bool,
    recovery_catalogs_reconciled: bool,
    recovery_object_requires_save_as: bool,
    recovery_autosave: RecoveryAutosave,
    review_in_progress: bool,
    close_confirmation: bool,
    exit_requested: bool,
    needs_sync: bool,
}

impl WorkshopRuntime {
    fn initialize() -> Self {
        let loaded = (|| -> Result<
            (
                AssetProject,
                WorkshopDraft,
                PreviewSubject,
                RecoveryStore,
                Option<PendingRecovery>,
                String,
            ),
            String,
        > {
                let current_directory = env::current_dir()
                    .map_err(|error| format!("cannot read working directory: {error}"))?;
                let root = resolve_repository_root(env::args_os(), &current_directory)
                    .map_err(|error| error.to_string())?;
                let project = AssetProject::load(&root).map_err(|error| error.to_string())?;
                let (editor, preview) = calibration_for_project(&project)?;
                let draft =
                    WorkshopDraft::new(project.palette().clone(), project.styles().clone(), editor);
                let recovery_store = RecoveryStore::new(&root);
                let pending_recovery = match recovery_store.load() {
                    Ok(Some(envelope)) => Some(PendingRecovery::Available(Box::new(envelope))),
                    Ok(None) => None,
                    Err(error) => Some(PendingRecovery::Invalid(error.to_string())),
                };
                Ok((
                    project,
                    draft,
                    preview,
                    recovery_store,
                    pending_recovery,
                    format!("Project loaded from {}", root.display()),
                ))
            })();

        match loaded {
            Ok((project, draft, preview, recovery_store, pending_recovery, message)) => Self {
                project: Some(project),
                draft: Some(draft),
                document: OpenDocument::Calibration,
                preview,
                overlays: OverlaySettings {
                    grid: true,
                    ..default()
                },
                status: Some(WorkshopStatus::info(message)),
                load_failure: None,
                external_changes: Vec::new(),
                recovery_store: Some(recovery_store),
                pending_recovery,
                recovery_base_revisions: None,
                recovery_conflict: false,
                recovery_catalogs_reconciled: false,
                recovery_object_requires_save_as: false,
                recovery_autosave: RecoveryAutosave::default(),
                review_in_progress: false,
                close_confirmation: false,
                exit_requested: false,
                needs_sync: true,
            },
            Err(error) => Self {
                project: None,
                draft: None,
                document: OpenDocument::Calibration,
                preview: PreviewSubject::ActiveStyle,
                overlays: OverlaySettings {
                    grid: true,
                    ..default()
                },
                status: None,
                load_failure: Some(error),
                external_changes: Vec::new(),
                recovery_store: None,
                pending_recovery: None,
                recovery_base_revisions: None,
                recovery_conflict: false,
                recovery_catalogs_reconciled: false,
                recovery_object_requires_save_as: false,
                recovery_autosave: RecoveryAutosave::default(),
                review_in_progress: false,
                close_confirmation: false,
                exit_requested: false,
                needs_sync: true,
            },
        }
    }

    fn set_status(&mut self, kind: WorkshopStatusKind, message: impl Into<String>) {
        self.status = Some(WorkshopStatus {
            kind,
            message: message.into(),
        });
        self.needs_sync = true;
    }

    fn draft_mut(&mut self) -> Result<&mut WorkshopDraft, String> {
        let error = self.load_error_message();
        self.draft.as_mut().ok_or(error)
    }

    fn project_mut(&mut self) -> Result<&mut AssetProject, String> {
        let error = self.load_error_message();
        self.project.as_mut().ok_or(error)
    }

    fn load_error_message(&self) -> String {
        self.load_failure
            .clone()
            .unwrap_or_else(|| "the Asset Workshop project is unavailable".to_owned())
    }
}

#[derive(Resource, Debug, Default)]
struct PointerStroke {
    active: bool,
    last_cell: Option<LocalVoxelCoord>,
    last_cursor_position: Option<Vec2>,
    skipped_boundary_cells: usize,
    first_boundary_warning: Option<String>,
}

impl PointerStroke {
    fn accepts_cell(&mut self, cell: LocalVoxelCoord, cursor_position: Option<Vec2>) -> bool {
        if self.last_cell == Some(cell) {
            return false;
        }
        if self.last_cell.is_some() {
            let (Some(previous), Some(current)) = (self.last_cursor_position, cursor_position)
            else {
                return false;
            };
            if previous.distance_squared(current) < POINTER_STROKE_DRAG_THRESHOLD.powi(2) {
                return false;
            }
        }
        self.last_cell = Some(cell);
        self.last_cursor_position = cursor_position;
        true
    }

    fn record_boundary_skip(&mut self, warning: String) {
        self.skipped_boundary_cells = self.skipped_boundary_cells.saturating_add(1);
        if self.first_boundary_warning.is_none() {
            self.first_boundary_warning = Some(warning);
        }
    }

    fn boundary_skip_summary(&self) -> Option<String> {
        let first = self.first_boundary_warning.as_deref()?;
        let count = self.skipped_boundary_cells;
        let noun = if count == 1 { "cell" } else { "cells" };
        Some(format!(
            "Skipped {count} out-of-bounds placement {noun}; valid cells in the stroke were kept. \
             First skipped cell: {first}"
        ))
    }
}

#[derive(Resource)]
struct ProjectChangePoll(Timer);

impl Default for ProjectChangePoll {
    fn default() -> Self {
        Self(Timer::from_seconds(2.0, TimerMode::Repeating))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowCloseDecision {
    Exit,
    WaitForReview,
    ConfirmDirty,
}

/// Starts the Asset Workshop.
pub fn run() {
    let runtime = WorkshopRuntime::initialize();
    let asset_root = runtime
        .project
        .as_ref()
        .map(|project| project.repository_root().join("assets"))
        .unwrap_or_else(|| PathBuf::from("assets"));
    App::new()
        .insert_resource(runtime)
        .insert_resource(ClearColor(Color::srgb(0.055, 0.06, 0.07)))
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: asset_root.to_string_lossy().into_owned(),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Bevy Hex Asset Workshop".to_owned(),
                        name: Some("hex-editor".to_owned()),
                        resolution: WindowResolution::new(1440, 900),
                        resize_constraints: WindowResizeConstraints {
                            min_width: 1024.0,
                            min_height: 640.0,
                            ..default()
                        },
                        present_mode: PresentMode::AutoVsync,
                        ..default()
                    }),
                    close_when_requested: false,
                    ..default()
                }),
        )
        .add_plugins(EguiPlugin::default())
        .add_plugins(crate::viewport::plugin)
        .add_plugins(crate::review_capture::plugin)
        .add_plugins(crate::ui::plugin)
        .init_resource::<PointerStroke>()
        .init_resource::<ProjectChangePoll>()
        .add_systems(
            Update,
            (
                handle_review_capture_outcomes,
                intercept_window_close_requests,
                handle_ui_actions,
                handle_pointer_editing,
                poll_external_changes,
                autosave_recovery,
                synchronize_views,
            )
                .chain()
                .before(ViewportSystems::Reconcile),
        )
        .run();
}

fn handle_ui_actions(
    mut actions: MessageReader<WorkshopUiAction>,
    hovered: Res<HoveredFaceTarget>,
    mut runtime: ResMut<WorkshopRuntime>,
    mut camera_snaps: MessageWriter<CameraSnapRequest>,
    mut frame_requests: MessageWriter<FrameViewportRequest>,
    mut review_requests: MessageWriter<ReviewCaptureRequest>,
    mut exit: MessageWriter<AppExit>,
) {
    for action in actions.read().cloned() {
        match action {
            WorkshopUiAction::SnapCamera(snap) => {
                camera_snaps.write(CameraSnapRequest(camera_snap(snap)));
                continue;
            }
            WorkshopUiAction::FrameCamera => {
                frame_requests.write(FrameViewportRequest);
                continue;
            }
            WorkshopUiAction::ExportReview => {
                match build_review_capture_request(&runtime) {
                    Ok(request) => {
                        review_requests.write(request);
                        runtime.review_in_progress = true;
                        runtime.set_status(
                            WorkshopStatusKind::Info,
                            "Rendering deterministic review pack",
                        );
                    }
                    Err(error) => runtime.set_status(WorkshopStatusKind::Error, error),
                }
                continue;
            }
            action => {
                let outcome = apply_ui_action(action, &mut runtime, hovered.0);
                match outcome {
                    Ok(message) => {
                        if let Some(message) = message {
                            runtime.set_status(WorkshopStatusKind::Success, message);
                        } else {
                            runtime.needs_sync = true;
                        }
                    }
                    Err(error) => runtime.set_status(WorkshopStatusKind::Error, error),
                }
            }
        }
    }
    if runtime.exit_requested {
        runtime.exit_requested = false;
        exit.write(AppExit::Success);
    }
}

fn handle_review_capture_outcomes(
    mut finished: MessageReader<ReviewCaptureFinished>,
    mut rejected: MessageReader<ReviewCaptureRejected>,
    mut runtime: ResMut<WorkshopRuntime>,
) {
    for finished in finished.read() {
        runtime.review_in_progress = false;
        match &finished.result {
            Ok(ReviewPublishOutcome::Published(path)) => runtime.set_status(
                WorkshopStatusKind::Success,
                format!("Published review pack to {}", path.display()),
            ),
            Ok(ReviewPublishOutcome::AlreadyPublished(path)) => runtime.set_status(
                WorkshopStatusKind::Info,
                format!("Review pack already exists at {}", path.display()),
            ),
            Err(error) => runtime.set_status(
                WorkshopStatusKind::Error,
                format!("Review export failed: {error}"),
            ),
        }
    }
    for rejected in rejected.read() {
        apply_review_capture_rejection(&mut runtime, &rejected.error);
    }
}

fn apply_review_capture_rejection(runtime: &mut WorkshopRuntime, error: &str) {
    runtime.review_in_progress = false;
    runtime.set_status(
        WorkshopStatusKind::Error,
        format!("Review export rejected: {error}"),
    );
}

fn apply_ui_action(
    action: WorkshopUiAction,
    runtime: &mut WorkshopRuntime,
    hovered: Option<ViewportFaceTarget>,
) -> Result<Option<String>, String> {
    match action {
        WorkshopUiAction::SetMode(mode) => {
            runtime.draft_mut()?.editor_mut_untracked().set_mode(mode);
            Ok(None)
        }
        WorkshopUiAction::Undo => {
            let label = runtime
                .draft
                .as_ref()
                .and_then(WorkshopDraft::undo_label)
                .map(str::to_owned);
            let changed = runtime
                .draft_mut()?
                .undo()
                .map_err(|error| error.to_string())?;
            Ok(changed.then(|| format!("Undid {}", label.as_deref().unwrap_or("edit"))))
        }
        WorkshopUiAction::Redo => {
            let label = runtime
                .draft
                .as_ref()
                .and_then(WorkshopDraft::redo_label)
                .map(str::to_owned);
            let changed = runtime
                .draft_mut()?
                .redo()
                .map_err(|error| error.to_string())?;
            Ok(changed.then(|| format!("Redid {}", label.as_deref().unwrap_or("edit"))))
        }
        WorkshopUiAction::SaveCatalogs => {
            ensure_catalog_save_allowed(runtime)?;
            let (palette, styles) = {
                let draft = runtime
                    .draft
                    .as_ref()
                    .ok_or_else(|| runtime.load_error_message())?;
                (draft.palette().clone(), draft.styles().clone())
            };
            runtime
                .project_mut()?
                .save_catalogs(palette, styles)
                .map_err(|error| error.to_string())?;
            runtime.draft_mut()?.mark_catalogs_saved();
            refresh_recovery_baseline_after_tracked_write(runtime);
            Ok(Some("Saved palette and voxel styles".to_owned()))
        }
        WorkshopUiAction::ReloadProject => reload_project(runtime),
        WorkshopUiAction::ExportReview => {
            Err("review export must be handled by the capture adapter".to_owned())
        }
        WorkshopUiAction::RestoreRecovery => restore_pending_recovery(runtime),
        WorkshopUiAction::DiscardRecovery => discard_pending_recovery(runtime),
        WorkshopUiAction::ReconcileRecoveryCatalogs => reconcile_recovered_catalogs(runtime),
        WorkshopUiAction::SaveAllAndClose => {
            save_all_for_close(runtime)?;
            discard_recovery_file(runtime)?;
            runtime.close_confirmation = false;
            runtime.exit_requested = true;
            Ok(Some("Saved all Workshop changes".to_owned()))
        }
        WorkshopUiAction::DiscardAndClose => {
            discard_recovery_file(runtime)?;
            runtime.close_confirmation = false;
            runtime.exit_requested = true;
            Ok(Some("Discarded local Workshop changes".to_owned()))
        }
        WorkshopUiAction::CancelClose => {
            runtime.close_confirmation = false;
            Ok(Some("Close cancelled".to_owned()))
        }
        WorkshopUiAction::SaveObject => save_current_object(runtime),
        WorkshopUiAction::SaveObjectAs { id, display_name } => {
            save_current_object_as(runtime, id, display_name)
        }
        WorkshopUiAction::DuplicateObject {
            source,
            id,
            display_name,
        } => {
            ensure_document_can_change(runtime)?;
            runtime
                .project_mut()?
                .duplicate_object(&source, id.clone(), display_name)
                .map_err(|error| error.to_string())?;
            refresh_recovery_baseline_after_tracked_write(runtime);
            let blueprint = runtime
                .project
                .as_ref()
                .and_then(|project| project.object(&id))
                .cloned()
                .ok_or_else(|| "saved duplicate was not indexed".to_owned())?;
            let editor =
                EditorModel::from_blueprint(blueprint).map_err(|error| error.to_string())?;
            runtime.draft_mut()?.open_object(editor);
            runtime.document = OpenDocument::Saved(id.clone());
            Ok(Some(format!("Duplicated {}", id.as_str())))
        }
        WorkshopUiAction::NewObject {
            id,
            display_name,
            category,
            prop_connectivity,
        } => {
            ensure_document_can_change(runtime)?;
            validate_object_id_category(&id, category)?;
            let style = {
                let draft = runtime
                    .draft
                    .as_ref()
                    .ok_or_else(|| runtime.load_error_message())?;
                draft
                    .editor()
                    .active_style()
                    .filter(|style| draft.styles().contains(style))
                    .cloned()
                    .or_else(|| draft.styles().styles().keys().next().cloned())
                    .ok_or_else(|| "create a voxel style before creating an object".to_owned())?
            };
            let connectivity = match category {
                ObjectCategory::Plant => ConnectivityPolicy::Grounded,
                ObjectCategory::Effect => ConnectivityPolicy::Free,
                ObjectCategory::Prop => prop_connectivity,
            };
            let mut editor = EditorModel::blank(category, connectivity, style)
                .map_err(|error| error.to_string())?;
            editor
                .set_unsaved_identity(id.clone(), display_name)
                .map_err(|error| error.to_string())?;
            runtime.draft_mut()?.open_object(editor);
            runtime.document = OpenDocument::Unsaved(id.clone());
            Ok(Some(format!("Created unsaved {}", id.as_str())))
        }
        WorkshopUiAction::OpenObject(id) => {
            ensure_document_can_change(runtime)?;
            let blueprint = runtime
                .project
                .as_ref()
                .and_then(|project| project.object(&id))
                .cloned()
                .ok_or_else(|| format!("object '{}' does not exist", id.as_str()))?;
            let editor =
                EditorModel::from_blueprint(blueprint).map_err(|error| error.to_string())?;
            runtime.draft_mut()?.open_object(editor);
            runtime.document = OpenDocument::Saved(id.clone());
            Ok(Some(format!("Opened {}", id.as_str())))
        }
        WorkshopUiAction::DeleteObject(id) => {
            ensure_tracked_overwrite_allowed(runtime)?;
            if runtime.document == OpenDocument::Saved(id.clone()) {
                ensure_document_can_change(runtime)?;
            }
            runtime
                .project_mut()?
                .delete_object(&id)
                .map_err(|error| error.to_string())?;
            refresh_recovery_baseline_after_tracked_write(runtime);
            if runtime.document == OpenDocument::Saved(id.clone()) {
                reset_to_calibration(runtime)?;
            }
            Ok(Some(format!("Deleted {}", id.as_str())))
        }
        WorkshopUiAction::PreviewSwatch(id) => {
            runtime.preview = PreviewSubject::Swatch(id);
            runtime
                .draft_mut()?
                .editor_mut_untracked()
                .set_mode(WorkshopMode::VoxelStyles);
            Ok(None)
        }
        WorkshopUiAction::PreviewStyle(id) => {
            runtime.preview = PreviewSubject::Style(id.clone());
            let editor = runtime.draft_mut()?.editor_mut_untracked();
            editor.set_active_style(Some(id));
            editor.set_mode(WorkshopMode::VoxelStyles);
            Ok(None)
        }
        WorkshopUiAction::DeleteSwatch(id) => {
            if let Some(project) = runtime.project.as_ref() {
                let usage = project.swatch_usage(&id);
                if !usage.styles.is_empty() {
                    return Err(format!(
                        "save or migrate styles [{}] before deleting '{}'",
                        usage
                            .styles
                            .iter()
                            .map(VoxelStyleId::as_str)
                            .collect::<Vec<_>>()
                            .join(", "),
                        id.as_str()
                    ));
                }
            }
            let changed = runtime
                .draft_mut()?
                .delete_swatch(&id)
                .map_err(|error| error.to_string())?;
            Ok(changed.then(|| format!("Removed {} from the palette draft", id.as_str())))
        }
        WorkshopUiAction::SetPreviewRig(rig) => {
            runtime
                .draft_mut()?
                .editor_mut_untracked()
                .set_preview_rig(rig);
            Ok(None)
        }
        WorkshopUiAction::SetTool(tool) => {
            runtime.draft_mut()?.editor_mut_untracked().set_tool(tool);
            Ok(None)
        }
        WorkshopUiAction::SetActiveStyle(id) => {
            if !runtime
                .draft
                .as_ref()
                .is_some_and(|draft| draft.styles().contains(&id))
            {
                return Err(format!("voxel style '{}' does not exist", id.as_str()));
            }
            runtime.preview = PreviewSubject::Style(id.clone());
            runtime
                .draft_mut()?
                .editor_mut_untracked()
                .set_active_style(Some(id));
            Ok(None)
        }
        WorkshopUiAction::SetActivePart(part) => {
            runtime
                .draft_mut()?
                .editor_mut_untracked()
                .set_active_part(part)
                .map_err(|error| error.to_string())?;
            Ok(None)
        }
        WorkshopUiAction::SetActiveLevel(level) => {
            runtime
                .draft_mut()?
                .editor_mut_untracked()
                .set_active_level(level)
                .map_err(|error| error.to_string())?;
            Ok(None)
        }
        WorkshopUiAction::SetObjectDisplayName(display_name) => {
            let changed = runtime
                .draft_mut()?
                .edit_object("Rename object", |editor| {
                    editor.set_display_name(display_name)
                })
                .map_err(|error| error.to_string())?;
            Ok(changed.then(|| "Renamed object draft".to_owned()))
        }
        WorkshopUiAction::SetObjectConnectivity(connectivity) => {
            let changed = runtime
                .draft_mut()?
                .edit_object("Change connectivity", |editor| {
                    editor.set_connectivity(connectivity)
                })
                .map_err(|error| error.to_string())?;
            Ok(changed.then(|| "Changed object connectivity".to_owned()))
        }
        WorkshopUiAction::SetObjectBounds(bounds) => {
            let changed = runtime
                .draft_mut()?
                .edit_object("Change object bounds", |editor| editor.set_bounds(bounds))
                .map_err(|error| error.to_string())?;
            Ok(changed.then(|| "Changed authoring bounds".to_owned()))
        }
        WorkshopUiAction::SetOriginFromSelection => {
            let origin = {
                let editor = runtime
                    .draft
                    .as_ref()
                    .ok_or_else(|| runtime.load_error_message())?
                    .editor();
                if editor.selection().len() != 1 {
                    return Err("select exactly one occupied cell for the origin".to_owned());
                }
                editor
                    .selection()
                    .cells()
                    .iter()
                    .next()
                    .copied()
                    .ok_or_else(|| "origin selection is empty".to_owned())?
            };
            let changed = runtime
                .draft_mut()?
                .edit_object("Move object origin", |editor| editor.set_origin(origin))
                .map_err(|error| error.to_string())?;
            Ok(changed.then(|| "Moved object origin".to_owned()))
        }
        WorkshopUiAction::NudgeSelection { q, r, level } => {
            let changed = runtime
                .draft_mut()?
                .edit_object("Nudge selection", |editor| {
                    editor.nudge_selection(q, r, level)
                })
                .map_err(|error| error.to_string())?;
            Ok(changed.then(|| "Moved selection".to_owned()))
        }
        WorkshopUiAction::RotateSelectionClockwise => {
            let pivot = runtime
                .draft
                .as_ref()
                .ok_or_else(|| runtime.load_error_message())?
                .editor()
                .object()
                .origin;
            let changed = runtime
                .draft_mut()?
                .edit_object("Rotate selection", |editor| {
                    editor.rotate_selection_clockwise(pivot)
                })
                .map_err(|error| error.to_string())?;
            Ok(changed.then(|| "Rotated selection by 60 degrees".to_owned()))
        }
        WorkshopUiAction::CopySelection => {
            let count = runtime
                .draft_mut()?
                .editor_mut_untracked()
                .copy_selection()
                .map_err(|error| error.to_string())?;
            Ok(Some(format!("Copied {count} voxels")))
        }
        WorkshopUiAction::PasteSelection => {
            let target = hovered.map(|target| target.cell).unwrap_or_else(|| {
                let level = runtime
                    .draft
                    .as_ref()
                    .map_or(0, |draft| draft.editor().active_level());
                LocalVoxelCoord::new(0, 0, level)
            });
            let changed = runtime
                .draft_mut()?
                .edit_object("Paste selection", |editor| editor.paste_at(target))
                .map_err(|error| error.to_string())?;
            Ok(changed.then(|| "Pasted selection".to_owned()))
        }
        WorkshopUiAction::DeleteSelection => {
            let changed = runtime
                .draft_mut()?
                .edit_object("Delete selection", EditorModel::delete_selection)
                .map_err(|error| error.to_string())?;
            Ok(changed.then(|| "Deleted selection".to_owned()))
        }
        WorkshopUiAction::ClearSelection => {
            runtime
                .draft_mut()?
                .editor_mut_untracked()
                .clear_selection();
            Ok(None)
        }
        WorkshopUiAction::RepaintSelectionPart(part) => {
            edit_selected(runtime, "Change selected roles", |editor, position| {
                editor.change_part(position, part)
            })?;
            Ok(Some("Changed selected voxel roles".to_owned()))
        }
        WorkshopUiAction::SetSelectionCanopy(enabled) => {
            edit_selected(
                runtime,
                "Change selected canopy mask",
                |editor, position| editor.set_canopy_occluder(position, enabled),
            )?;
            Ok(Some("Changed canopy mask".to_owned()))
        }
        WorkshopUiAction::SetSelectionBlocker(enabled) => {
            edit_selected_columns(runtime, "Change selected blocker mask", enabled)?;
            Ok(Some("Changed blocker footprint".to_owned()))
        }
        WorkshopUiAction::ShowBlockerOverlay(show) => {
            runtime.overlays.blockers = show;
            Ok(None)
        }
        WorkshopUiAction::ShowCanopyOverlay(show) => {
            runtime.overlays.canopy = show;
            Ok(None)
        }
        WorkshopUiAction::ShowSemanticOverlay(show) => {
            runtime.overlays.semantics = show;
            Ok(None)
        }
        WorkshopUiAction::IsolateActiveLevel(isolate) => {
            runtime.overlays.isolate_active_level = isolate;
            Ok(None)
        }
        WorkshopUiAction::ShowGrid(show) => {
            runtime.overlays.grid = show;
            Ok(None)
        }
        WorkshopUiAction::UpsertSwatch {
            id,
            swatch,
            confirmed_near_color,
        } => {
            let changed = runtime
                .draft_mut()?
                .upsert_swatch(id.clone(), swatch, confirmed_near_color)
                .map_err(|error| error.to_string())?;
            runtime.preview = PreviewSubject::Swatch(id.clone());
            Ok(changed.then(|| format!("Updated palette draft '{}'", id.as_str())))
        }
        WorkshopUiAction::UpsertStyle { id, style } => {
            let changed = runtime
                .draft_mut()?
                .upsert_style(id.clone(), style)
                .map_err(|error| error.to_string())?;
            runtime.preview = PreviewSubject::Style(id.clone());
            runtime
                .draft_mut()?
                .editor_mut_untracked()
                .set_active_style(Some(id.clone()));
            Ok(changed.then(|| format!("Updated style draft '{}'", id.as_str())))
        }
        WorkshopUiAction::DeleteStyle(id) => {
            if let Some(usage) = runtime
                .project
                .as_ref()
                .map(|project| project.style_usage(&id))
                .filter(|usage| !usage.is_empty())
            {
                return Err(format!(
                    "style '{}' is used by {}",
                    id.as_str(),
                    usage
                        .iter()
                        .map(|entry| entry.object.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            let changed = runtime
                .draft_mut()?
                .delete_style(&id)
                .map_err(|error| error.to_string())?;
            Ok(changed.then(|| format!("Removed style '{}' from the draft", id.as_str())))
        }
        WorkshopUiAction::SnapCamera(_) | WorkshopUiAction::FrameCamera => Ok(None),
    }
}

fn save_current_object(runtime: &mut WorkshopRuntime) -> Result<Option<String>, String> {
    let blueprint = {
        let draft = runtime
            .draft
            .as_ref()
            .ok_or_else(|| runtime.load_error_message())?;
        draft
            .editor()
            .blueprint_for_save(draft.styles())
            .map_err(|error| error.to_string())?
    };
    match runtime.document.clone() {
        OpenDocument::Calibration => Err("use New or Save As for the calibration scene".to_owned()),
        OpenDocument::Unsaved(proposed_id) => {
            runtime
                .project_mut()?
                .save_object_as(blueprint, proposed_id.clone())
                .map_err(|error| error.to_string())?;
            runtime
                .draft_mut()?
                .mark_object_saved_as(proposed_id.clone());
            runtime.document = OpenDocument::Saved(proposed_id.clone());
            refresh_recovery_baseline_after_tracked_write(runtime);
            Ok(Some(format!("Saved {}", proposed_id.as_str())))
        }
        OpenDocument::Saved(id) => {
            ensure_tracked_overwrite_allowed(runtime)?;
            runtime
                .project_mut()?
                .save_object(&id, blueprint)
                .map_err(|error| error.to_string())?;
            runtime.draft_mut()?.mark_object_saved();
            refresh_recovery_baseline_after_tracked_write(runtime);
            Ok(Some(format!("Saved {}", id.as_str())))
        }
    }
}

fn save_current_object_as(
    runtime: &mut WorkshopRuntime,
    id: ObjectAssetId,
    display_name: String,
) -> Result<Option<String>, String> {
    let (mut editor, styles) = {
        let draft = runtime
            .draft
            .as_ref()
            .ok_or_else(|| runtime.load_error_message())?;
        validate_object_id_category(&id, draft.editor().object().category)?;
        (draft.editor().clone(), draft.styles().clone())
    };
    editor
        .set_display_name(display_name)
        .map_err(|error| error.to_string())?;
    let blueprint = editor
        .blueprint_for_save(&styles)
        .map_err(|error| error.to_string())?;
    runtime
        .project_mut()?
        .save_object_as(blueprint, id.clone())
        .map_err(|error| error.to_string())?;
    editor.mark_saved_as(id.clone());
    runtime.draft_mut()?.open_object(editor);
    runtime.document = OpenDocument::Saved(id.clone());
    runtime.recovery_object_requires_save_as = false;
    resolve_recovery_conflict_after_save_as(runtime);
    refresh_recovery_baseline_after_tracked_write(runtime);
    Ok(Some(format!("Saved as {}", id.as_str())))
}

fn resolve_recovery_conflict_after_save_as(runtime: &mut WorkshopRuntime) {
    if !runtime.recovery_conflict {
        return;
    }
    let catalogs_match = runtime
        .project
        .as_ref()
        .zip(runtime.draft.as_ref())
        .is_some_and(|(project, draft)| {
            draft.palette() == project.palette() && draft.styles() == project.styles()
        });
    if catalogs_match || runtime.recovery_catalogs_reconciled {
        clear_recovery_conflict(runtime);
    }
}

fn reconcile_recovered_catalogs(runtime: &mut WorkshopRuntime) -> Result<Option<String>, String> {
    if !runtime.recovery_conflict {
        return Ok(Some(
            "Recovered work already uses the current tracked baseline".to_owned(),
        ));
    }

    let root = runtime
        .project
        .as_ref()
        .ok_or_else(|| runtime.load_error_message())?
        .repository_root()
        .to_path_buf();
    let current_project = AssetProject::load(&root).map_err(|error| error.to_string())?;
    let (base_palette, base_styles, local_palette, local_styles) = {
        let draft = runtime
            .draft
            .as_ref()
            .ok_or_else(|| runtime.load_error_message())?;
        (
            draft.saved_palette().clone(),
            draft.saved_styles().clone(),
            draft.palette().clone(),
            draft.styles().clone(),
        )
    };

    let (palette_entries, palette_conflicts) = three_way_merge_entries(
        base_palette.swatches(),
        local_palette.swatches(),
        current_project.palette().swatches(),
    );
    let (style_entries, style_conflicts) = three_way_merge_entries(
        base_styles.styles(),
        local_styles.styles(),
        current_project.styles().styles(),
    );
    if !palette_conflicts.is_empty() || !style_conflicts.is_empty() {
        let mut conflicts = palette_conflicts
            .into_iter()
            .map(|id| format!("swatch '{}'", id.as_str()))
            .collect::<Vec<_>>();
        conflicts.extend(
            style_conflicts
                .into_iter()
                .map(|id| format!("style '{}'", id.as_str())),
        );
        conflicts.sort();
        return Err(format!(
            "recovery and tracked catalogs both changed {}; choose the desired values in the \
             recovered draft or reload before reconciling",
            conflicts.join(", ")
        ));
    }

    let merged_palette = ArtPalette::new(palette_entries).map_err(|error| error.to_string())?;
    let merged_styles = VoxelStyleCatalog::new(style_entries).map_err(|error| error.to_string())?;
    current_project
        .validate_catalogs(&merged_palette, &merged_styles)
        .map_err(|error| {
            format!("recovered catalogs cannot be safely rebased onto tracked objects: {error}")
        })?;
    let current_palette = current_project.palette().clone();
    let current_styles = current_project.styles().clone();

    runtime
        .draft_mut()?
        .adopt_rebased_catalogs(
            current_palette,
            current_styles,
            merged_palette,
            merged_styles,
        )
        .map_err(|error| error.to_string())?;
    runtime.project = Some(current_project);
    runtime.external_changes.clear();
    runtime.recovery_catalogs_reconciled = true;
    if !runtime.recovery_object_requires_save_as {
        clear_recovery_conflict(runtime);
    }
    runtime.needs_sync = true;

    Ok(Some(if runtime.recovery_conflict {
        "Reconciled recovered catalogs; save them, then use Save As to preserve the recovered object"
            .to_owned()
    } else {
        "Reconciled recovered catalogs onto the current tracked baseline".to_owned()
    }))
}

fn three_way_merge_entries<K, V>(
    base: &BTreeMap<K, V>,
    local: &BTreeMap<K, V>,
    current: &BTreeMap<K, V>,
) -> (BTreeMap<K, V>, Vec<K>)
where
    K: Ord + Clone,
    V: Clone + PartialEq,
{
    let keys = base
        .keys()
        .chain(local.keys())
        .chain(current.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut merged = BTreeMap::new();
    let mut conflicts = Vec::new();

    for key in keys {
        let base_value = base.get(&key);
        let local_value = local.get(&key);
        let current_value = current.get(&key);
        let chosen = if local_value == base_value {
            current_value
        } else if current_value == base_value || local_value == current_value {
            local_value
        } else {
            conflicts.push(key);
            continue;
        };
        if let Some(value) = chosen {
            drop(merged.insert(key, value.clone()));
        }
    }

    (merged, conflicts)
}

fn clear_recovery_conflict(runtime: &mut WorkshopRuntime) {
    runtime.recovery_conflict = false;
    runtime.recovery_catalogs_reconciled = false;
    runtime.recovery_object_requires_save_as = false;
    runtime.recovery_base_revisions = runtime
        .project
        .as_ref()
        .map(AssetProject::revision_snapshot);
}

fn refresh_recovery_baseline_after_tracked_write(runtime: &mut WorkshopRuntime) {
    if runtime.recovery_conflict {
        // Keep an unresolved object-rescue requirement crash-durable. Clearing this
        // mismatch before Save As would make the next launch permit an overwrite.
        return;
    }
    runtime.recovery_base_revisions = runtime
        .project
        .as_ref()
        .map(AssetProject::revision_snapshot);
}

fn ensure_document_can_change(runtime: &WorkshopRuntime) -> Result<(), String> {
    let Some(draft) = runtime.draft.as_ref() else {
        return Err(runtime.load_error_message());
    };
    if runtime.document != OpenDocument::Calibration && draft.editor().is_dirty() {
        return Err("save or undo the current object changes first".to_owned());
    }
    Ok(())
}

fn reset_to_calibration(runtime: &mut WorkshopRuntime) -> Result<(), String> {
    let (editor, preview) = calibration_for_project(
        runtime
            .project
            .as_ref()
            .ok_or_else(|| runtime.load_error_message())?,
    )?;
    runtime.draft_mut()?.open_object(editor);
    runtime.document = OpenDocument::Calibration;
    runtime.preview = preview;
    Ok(())
}

fn reload_project(runtime: &mut WorkshopRuntime) -> Result<Option<String>, String> {
    let root = runtime
        .project
        .as_ref()
        .ok_or_else(|| runtime.load_error_message())?
        .repository_root()
        .to_path_buf();
    let project = AssetProject::load(&root).map_err(|error| error.to_string())?;
    let previous_mode = runtime
        .draft
        .as_ref()
        .map(|draft| draft.editor().mode())
        .unwrap_or(WorkshopMode::VoxelStyles);
    let saved_id = match &runtime.document {
        OpenDocument::Saved(id) => Some(id.clone()),
        OpenDocument::Calibration | OpenDocument::Unsaved(_) => None,
    };

    let restored = match saved_id {
        Some(id) => match project.object(&id).cloned() {
            Some(blueprint) => Some((
                EditorModel::from_blueprint(blueprint).map_err(|error| {
                    format!(
                        "cannot reopen saved object '{}' after reload: {error}",
                        id.as_str()
                    )
                })?,
                OpenDocument::Saved(id),
                PreviewSubject::ActiveStyle,
            )),
            None => None,
        },
        None => None,
    };
    let (mut editor, document, preview) = if let Some(restored) = restored {
        restored
    } else {
        let (editor, preview) = calibration_for_project(&project)?;
        (editor, OpenDocument::Calibration, preview)
    };
    if previous_mode == WorkshopMode::VoxelStyles {
        editor.set_mode(WorkshopMode::VoxelStyles);
    }
    let draft = WorkshopDraft::new(project.palette().clone(), project.styles().clone(), editor);
    discard_recovery_file(runtime)?;
    runtime.project = Some(project);
    runtime.draft = Some(draft);
    runtime.document = document;
    runtime.preview = preview;
    runtime.external_changes.clear();
    runtime.pending_recovery = None;
    runtime.recovery_base_revisions = None;
    runtime.recovery_conflict = false;
    runtime.recovery_catalogs_reconciled = false;
    runtime.recovery_object_requires_save_as = false;
    runtime.recovery_autosave = RecoveryAutosave::default();
    runtime.review_in_progress = false;
    runtime.close_confirmation = false;
    runtime.load_failure = None;
    runtime.needs_sync = true;
    Ok(Some(format!("Reloaded project from {}", root.display())))
}

fn restore_pending_recovery(runtime: &mut WorkshopRuntime) -> Result<Option<String>, String> {
    let envelope = match runtime.pending_recovery.as_ref() {
        Some(PendingRecovery::Available(envelope)) => envelope.as_ref().clone(),
        Some(PendingRecovery::Invalid(_)) => {
            return Err("discard the invalid recovery file before authoring".to_owned());
        }
        None => return Err("no recovery draft is available".to_owned()),
    };
    let current_revisions = runtime
        .project
        .as_ref()
        .ok_or_else(|| runtime.load_error_message())?
        .revision_snapshot();
    let base_conflict = envelope.base_revisions != current_revisions;
    let recovered_object_source_changed = match &envelope.document {
        RecoveryDocument::Saved(id) => {
            saved_object_revision_changed(id, &envelope.base_revisions, &current_revisions)
        }
        RecoveryDocument::Calibration | RecoveryDocument::Unsaved(_) => false,
    };
    let document = open_document_from_recovery(&envelope.document);
    let session = RecoverableSession {
        document: envelope.document.clone(),
        workshop: envelope.workshop.clone(),
    };
    let (draft, sanitization) =
        WorkshopDraft::from_recovery(envelope.workshop).map_err(|error| error.to_string())?;
    let preview = preview_for_draft(&draft);

    runtime.draft = Some(draft);
    runtime.document = document;
    runtime.preview = preview;
    runtime.pending_recovery = None;
    runtime.recovery_base_revisions = Some(envelope.base_revisions);
    runtime.recovery_conflict = base_conflict;
    runtime.recovery_catalogs_reconciled = false;
    runtime.recovery_object_requires_save_as = base_conflict
        && matches!(runtime.document, OpenDocument::Saved(_))
        && (recovered_object_source_changed
            || runtime
                .draft
                .as_ref()
                .is_some_and(|draft| draft.editor().is_dirty()));
    runtime.recovery_autosave = RecoveryAutosave {
        last_observed: Some(session.clone()),
        last_written: Some(session),
        dirty_since_seconds: None,
        last_change_seconds: None,
        next_retry_seconds: 0.0,
    };
    runtime.needs_sync = true;

    let selection_note = (sanitization.discarded_selection_cells > 0).then(|| {
        format!(
            "; discarded {} stale selection cells",
            sanitization.discarded_selection_cells
        )
    });
    let conflict_note = base_conflict
        .then_some("; tracked files changed since recovery, so overwrites remain blocked");
    Ok(Some(format!(
        "Restored recovery draft{}{}",
        selection_note.as_deref().unwrap_or(""),
        conflict_note.unwrap_or("")
    )))
}

fn saved_object_revision_changed(
    id: &ObjectAssetId,
    recovered: &ProjectRevisionSet,
    current: &ProjectRevisionSet,
) -> bool {
    let path = format!("objects/{}.ron", id.as_str());
    recovered.files.get(&path) != current.files.get(&path)
}

fn discard_pending_recovery(runtime: &mut WorkshopRuntime) -> Result<Option<String>, String> {
    if runtime.pending_recovery.is_none() {
        return Err("no pending recovery file is available".to_owned());
    }
    let discarded = discard_recovery_file(runtime)?;
    runtime.pending_recovery = None;
    runtime.recovery_base_revisions = None;
    runtime.recovery_conflict = false;
    runtime.recovery_catalogs_reconciled = false;
    runtime.recovery_object_requires_save_as = false;
    runtime.recovery_autosave = RecoveryAutosave::default();
    runtime.needs_sync = true;
    Ok(Some(if discarded {
        "Discarded the recovery draft".to_owned()
    } else {
        "Recovery draft was already absent".to_owned()
    }))
}

fn discard_recovery_file(runtime: &mut WorkshopRuntime) -> Result<bool, String> {
    let discarded = runtime
        .recovery_store
        .as_ref()
        .ok_or_else(|| "recovery storage is unavailable".to_owned())?
        .discard()
        .map_err(|error| error.to_string())?;
    runtime.recovery_autosave.last_written = None;
    Ok(discarded)
}

fn save_all_for_close(runtime: &mut WorkshopRuntime) -> Result<(), String> {
    ensure_tracked_overwrite_allowed(runtime)?;
    let object_needs_save = runtime
        .draft
        .as_ref()
        .is_some_and(|draft| draft.editor().is_dirty());
    if object_needs_save && !matches!(runtime.document, OpenDocument::Saved(_)) {
        return Err("use Save As for the current object before closing".to_owned());
    }

    if object_needs_save {
        let draft = runtime
            .draft
            .as_ref()
            .ok_or_else(|| runtime.load_error_message())?;
        draft
            .editor()
            .blueprint_for_save(draft.styles())
            .map_err(|error| error.to_string())?;
    }

    let catalogs_need_save = {
        let project = runtime
            .project
            .as_ref()
            .ok_or_else(|| runtime.load_error_message())?;
        let draft = runtime
            .draft
            .as_ref()
            .ok_or_else(|| runtime.load_error_message())?;
        draft.palette() != project.palette() || draft.styles() != project.styles()
    };
    if catalogs_need_save {
        let (palette, styles) = {
            let draft = runtime
                .draft
                .as_ref()
                .ok_or_else(|| runtime.load_error_message())?;
            (draft.palette().clone(), draft.styles().clone())
        };
        runtime
            .project_mut()?
            .save_catalogs(palette, styles)
            .map_err(|error| error.to_string())?;
        runtime.draft_mut()?.mark_catalogs_saved();
    }
    if object_needs_save {
        drop(save_current_object(runtime)?);
    }
    runtime.recovery_base_revisions = runtime
        .project
        .as_ref()
        .map(AssetProject::revision_snapshot);
    Ok(())
}

fn ensure_tracked_overwrite_allowed(runtime: &WorkshopRuntime) -> Result<(), String> {
    if runtime.recovery_conflict {
        return Err(
            "recovered work has an older tracked baseline; reconcile catalogs and use Save As, \
             or reload first"
                .to_owned(),
        );
    }
    if !runtime.external_changes.is_empty() {
        return Err("tracked art files changed outside this editor; reload first".to_owned());
    }
    Ok(())
}

fn ensure_catalog_save_allowed(runtime: &WorkshopRuntime) -> Result<(), String> {
    if runtime.recovery_conflict && !runtime.recovery_catalogs_reconciled {
        return Err(
            "reconcile recovered catalogs with the current tracked baseline before saving"
                .to_owned(),
        );
    }
    if !runtime.external_changes.is_empty() {
        return Err("tracked art files changed outside this editor; reload first".to_owned());
    }
    Ok(())
}

fn open_document_from_recovery(document: &RecoveryDocument) -> OpenDocument {
    match document {
        RecoveryDocument::Calibration => OpenDocument::Calibration,
        RecoveryDocument::Unsaved(id) => OpenDocument::Unsaved(id.clone()),
        RecoveryDocument::Saved(id) => OpenDocument::Saved(id.clone()),
    }
}

fn recovery_document(document: &OpenDocument) -> RecoveryDocument {
    match document {
        OpenDocument::Calibration => RecoveryDocument::Calibration,
        OpenDocument::Unsaved(id) => RecoveryDocument::Unsaved(id.clone()),
        OpenDocument::Saved(id) => RecoveryDocument::Saved(id.clone()),
    }
}

fn recovery_document_label(document: &RecoveryDocument) -> String {
    match document {
        RecoveryDocument::Calibration => "the calibration scene".to_owned(),
        RecoveryDocument::Unsaved(id) => format!("new object '{}'", id.as_str()),
        RecoveryDocument::Saved(id) => format!("saved object '{}'", id.as_str()),
    }
}

fn preview_for_draft(draft: &WorkshopDraft) -> PreviewSubject {
    if let Some(style) = draft
        .editor()
        .active_style()
        .filter(|style| draft.styles().contains(style))
    {
        return PreviewSubject::Style(style.clone());
    }
    draft
        .palette()
        .swatches()
        .keys()
        .next()
        .cloned()
        .map_or(PreviewSubject::ActiveStyle, PreviewSubject::Swatch)
}

fn calibration_for_project(
    project: &AssetProject,
) -> Result<(EditorModel, PreviewSubject), String> {
    let mut editor = EditorModel::calibration_scene().map_err(|error| error.to_string())?;
    if let Some(style) = project.styles().styles().keys().next() {
        apply_calibration_style(&mut editor, style)?;
        editor.set_active_style(Some(style.clone()));
        editor.mark_saved();
        return Ok((editor, PreviewSubject::ActiveStyle));
    }
    editor.set_mode(WorkshopMode::VoxelStyles);
    let preview = project
        .palette()
        .swatches()
        .keys()
        .next()
        .cloned()
        .map_or(PreviewSubject::ActiveStyle, PreviewSubject::Swatch);
    Ok((editor, preview))
}

fn apply_calibration_style(editor: &mut EditorModel, style: &VoxelStyleId) -> Result<(), String> {
    let occupied = editor
        .object()
        .placements
        .iter()
        .map(|placement| placement.position)
        .collect::<Vec<_>>();
    for position in occupied {
        editor
            .repaint(position, style.clone())
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn validate_object_id_category(id: &ObjectAssetId, category: ObjectCategory) -> Result<(), String> {
    let prefix = match category {
        ObjectCategory::Plant => "plant/",
        ObjectCategory::Effect => "effect/",
        ObjectCategory::Prop => "prop/",
    };
    let Some(filename) = id.as_str().strip_prefix(prefix) else {
        return Err(format!(
            "{category:?} object ids must begin with '{prefix}'"
        ));
    };
    if filename.is_empty() || filename.contains('/') {
        return Err(format!(
            "object id '{}' must contain exactly one filename after '{prefix}'",
            id.as_str()
        ));
    }
    Ok(())
}

fn edit_selected(
    runtime: &mut WorkshopRuntime,
    label: &str,
    mut operation: impl FnMut(
        &mut EditorModel,
        LocalVoxelCoord,
    ) -> Result<bool, crate::model::EditorModelError>,
) -> Result<bool, String> {
    let positions: Vec<_> = runtime
        .draft
        .as_ref()
        .ok_or_else(|| runtime.load_error_message())?
        .editor()
        .selection()
        .cells()
        .iter()
        .copied()
        .collect();
    if positions.is_empty() {
        return Err("select at least one occupied cell".to_owned());
    }
    let draft = runtime.draft_mut()?;
    draft
        .begin_object_transaction(label)
        .map_err(|error| error.to_string())?;
    for position in positions {
        if let Err(error) = operation(draft.editor_mut_untracked(), position) {
            draft
                .cancel_object_transaction()
                .map_err(|cancel| format!("{error}; rollback failed: {cancel}"))?;
            return Err(error.to_string());
        }
    }
    draft
        .commit_object_transaction()
        .map_err(|error| error.to_string())
}

fn edit_selected_columns(
    runtime: &mut WorkshopRuntime,
    label: &str,
    enabled: bool,
) -> Result<bool, String> {
    let columns: BTreeSet<_> = runtime
        .draft
        .as_ref()
        .ok_or_else(|| runtime.load_error_message())?
        .editor()
        .selection()
        .cells()
        .iter()
        .map(|position| position.axial())
        .collect();
    if columns.is_empty() {
        return Err("select at least one occupied prop cell".to_owned());
    }
    let draft = runtime.draft_mut()?;
    draft
        .begin_object_transaction(label)
        .map_err(|error| error.to_string())?;
    for column in columns {
        if let Err(error) = draft
            .editor_mut_untracked()
            .set_prop_blocker(column, enabled)
        {
            draft
                .cancel_object_transaction()
                .map_err(|cancel| format!("{error}; rollback failed: {cancel}"))?;
            return Err(error.to_string());
        }
    }
    draft
        .commit_object_transaction()
        .map_err(|error| error.to_string())
}

fn handle_pointer_editing(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    input_enabled: Res<ViewportInputEnabled>,
    hovered: Res<HoveredFaceTarget>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut stroke: ResMut<PointerStroke>,
    mut runtime: ResMut<WorkshopRuntime>,
) {
    if stroke.active && buttons.just_released(MouseButton::Left) {
        let boundary_warning = stroke.boundary_skip_summary();
        *stroke = PointerStroke::default();
        let result = runtime.draft_mut().and_then(|draft| {
            draft
                .commit_object_transaction()
                .map_err(|error| error.to_string())
        });
        match result {
            Ok(_) => {
                runtime.needs_sync = true;
                if let Some(warning) = boundary_warning {
                    runtime.set_status(WorkshopStatusKind::Warning, warning);
                }
            }
            Err(error) => runtime.set_status(WorkshopStatusKind::Error, error),
        }
    }
    if !input_enabled.0 {
        return;
    }
    if keys.pressed(KeyCode::Space) {
        return;
    }
    let Some(draft) = runtime.draft.as_ref() else {
        return;
    };
    if draft.editor().mode() != WorkshopMode::Objects {
        return;
    }
    let tool = draft.editor().tool();
    let Some(target) = hovered.0 else {
        return;
    };

    if buttons.just_pressed(MouseButton::Left) {
        match tool {
            EditorTool::Place | EditorTool::Erase | EditorTool::Repaint => {
                let begin = runtime.draft_mut().and_then(|draft| {
                    draft
                        .begin_object_transaction(tool_label(tool))
                        .map_err(|error| error.to_string())
                });
                match begin {
                    Ok(()) => {
                        *stroke = PointerStroke {
                            active: true,
                            ..default()
                        };
                    }
                    Err(error) => {
                        runtime.set_status(WorkshopStatusKind::Error, error);
                        return;
                    }
                }
            }
            EditorTool::Eyedropper => {
                if target.source == ViewportPickSource::Voxel {
                    let result = runtime.draft_mut().and_then(|draft| {
                        draft
                            .editor_mut_untracked()
                            .pick_from(target.cell)
                            .map_err(|error| error.to_string())
                    });
                    match result {
                        Ok(()) => {
                            runtime.preview = PreviewSubject::ActiveStyle;
                            runtime.needs_sync = true;
                        }
                        Err(error) => runtime.set_status(WorkshopStatusKind::Error, error),
                    }
                }
                return;
            }
            EditorTool::Select => {
                let additive =
                    keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
                if target.source == ViewportPickSource::Voxel {
                    if let Ok(draft) = runtime.draft_mut() {
                        draft.editor_mut_untracked().select(target.cell, additive);
                        runtime.needs_sync = true;
                    }
                } else if !additive {
                    if let Ok(draft) = runtime.draft_mut() {
                        draft.editor_mut_untracked().clear_selection();
                        runtime.needs_sync = true;
                    }
                }
                return;
            }
        }
    }

    if !stroke.active || !buttons.pressed(MouseButton::Left) {
        return;
    }
    let Some(cell) = editing_cell(tool, target) else {
        return;
    };
    let cursor_position = windows.single().ok().and_then(Window::cursor_position);
    if !stroke.accepts_cell(cell, cursor_position) {
        return;
    }
    let Some(editor) = runtime.draft.as_ref().map(WorkshopDraft::editor) else {
        return;
    };
    if let Err(warning) = preflight_pointer_stroke_cell(editor, tool, cell) {
        stroke.record_boundary_skip(warning);
        return;
    }
    let result = apply_stroke_cell(runtime.draft_mut(), tool, cell);
    match result {
        Ok(_) => runtime.needs_sync = true,
        Err(error) => {
            let rollback = runtime.draft_mut().and_then(|draft| {
                draft
                    .cancel_object_transaction()
                    .map_err(|cancel| cancel.to_string())
            });
            *stroke = PointerStroke::default();
            runtime.needs_sync = true;
            let error = match rollback {
                Ok(()) => format!("{error}; the complete stroke was rolled back"),
                Err(cancel) => format!("{error}; stroke rollback also failed: {cancel}"),
            };
            runtime.set_status(WorkshopStatusKind::Error, error);
        }
    }
}

fn preflight_pointer_stroke_cell(
    editor: &EditorModel,
    tool: EditorTool,
    cell: LocalVoxelCoord,
) -> Result<(), String> {
    if tool != EditorTool::Place {
        return Ok(());
    }
    validate_position(editor.object(), cell).map_err(|error| error.to_string())
}

fn apply_stroke_cell(
    draft: Result<&mut WorkshopDraft, String>,
    tool: EditorTool,
    cell: LocalVoxelCoord,
) -> Result<bool, String> {
    let editor = draft?.editor_mut_untracked();
    match tool {
        EditorTool::Place => editor.place_active(cell).map_err(|error| error.to_string()),
        EditorTool::Erase => editor.erase(cell).map_err(|error| error.to_string()),
        EditorTool::Repaint => {
            let style = editor
                .active_style()
                .cloned()
                .ok_or_else(|| "choose a voxel style before repainting".to_owned())?;
            let part = editor.active_part();
            let style_changed = editor
                .repaint(cell, style)
                .map_err(|error| error.to_string())?;
            let part_changed = editor
                .change_part(cell, part)
                .map_err(|error| error.to_string())?;
            Ok(style_changed || part_changed)
        }
        EditorTool::Eyedropper | EditorTool::Select => Ok(false),
    }
}

fn editing_cell(tool: EditorTool, target: ViewportFaceTarget) -> Option<LocalVoxelCoord> {
    match tool {
        EditorTool::Place => placement_cell(target),
        EditorTool::Erase | EditorTool::Repaint => {
            (target.source == ViewportPickSource::Voxel).then_some(target.cell)
        }
        EditorTool::Eyedropper | EditorTool::Select => None,
    }
}

fn placement_cell(target: ViewportFaceTarget) -> Option<LocalVoxelCoord> {
    if target.source == ViewportPickSource::Grid {
        return Some(target.cell);
    }
    if target.normal.y > 0.5 {
        return checked_offset(target.cell, 0, 0, 1);
    }
    if target.normal.y < -0.5 {
        return checked_offset(target.cell, 0, 0, -1);
    }
    let normal = Vec2::new(target.normal.x, target.normal.z).normalize_or_zero();
    let directions = [
        (1, 0, Vec2::new(1.0, 0.0)),
        (0, 1, Vec2::new(0.5, 0.866_025_4)),
        (-1, 1, Vec2::new(-0.5, 0.866_025_4)),
        (-1, 0, Vec2::new(-1.0, 0.0)),
        (0, -1, Vec2::new(-0.5, -0.866_025_4)),
        (1, -1, Vec2::new(0.5, -0.866_025_4)),
    ];
    directions
        .into_iter()
        .max_by(|left, right| left.2.dot(normal).total_cmp(&right.2.dot(normal)))
        .and_then(|(q, r, _)| checked_offset(target.cell, q, r, 0))
}

fn checked_offset(cell: LocalVoxelCoord, q: i32, r: i32, level: i32) -> Option<LocalVoxelCoord> {
    Some(LocalVoxelCoord::new(
        cell.q.checked_add(q)?,
        cell.r.checked_add(r)?,
        cell.level.checked_add(level)?,
    ))
}

fn tool_label(tool: EditorTool) -> &'static str {
    match tool {
        EditorTool::Place => "Place stroke",
        EditorTool::Erase => "Erase stroke",
        EditorTool::Repaint => "Repaint stroke",
        EditorTool::Eyedropper => "Eyedropper",
        EditorTool::Select => "Select",
    }
}

fn intercept_window_close_requests(
    mut requests: MessageReader<WindowCloseRequested>,
    mut runtime: ResMut<WorkshopRuntime>,
    mut pointer_stroke: ResMut<PointerStroke>,
    mut exit: MessageWriter<AppExit>,
) {
    if requests.read().next().is_none() {
        return;
    }
    match window_close_decision(
        runtime.review_in_progress,
        runtime.pending_recovery.is_some(),
        has_unsaved_work(&runtime),
    ) {
        WindowCloseDecision::WaitForReview => {
            runtime.set_status(
                WorkshopStatusKind::Warning,
                "Wait for the review export to finish before closing the Workshop",
            );
            return;
        }
        WindowCloseDecision::Exit => {
            exit.write(AppExit::Success);
            return;
        }
        WindowCloseDecision::ConfirmDirty => {}
    }

    let open_transaction = runtime
        .draft
        .as_ref()
        .is_some_and(|draft| draft.editor().is_transaction_open());
    if open_transaction {
        match runtime.draft_mut().and_then(|draft| {
            draft
                .commit_object_transaction()
                .map_err(|error| error.to_string())
        }) {
            Ok(_) => *pointer_stroke = PointerStroke::default(),
            Err(error) => {
                runtime.set_status(WorkshopStatusKind::Error, error);
                return;
            }
        }
    }
    if let Err(error) = write_recovery_now(&mut runtime) {
        runtime.set_status(WorkshopStatusKind::Error, error);
    }
    runtime.close_confirmation = true;
    runtime.needs_sync = true;
}

fn window_close_decision(
    review_in_progress: bool,
    pending_recovery: bool,
    has_unsaved_work: bool,
) -> WindowCloseDecision {
    if review_in_progress {
        WindowCloseDecision::WaitForReview
    } else if pending_recovery || !has_unsaved_work {
        WindowCloseDecision::Exit
    } else {
        WindowCloseDecision::ConfirmDirty
    }
}

fn autosave_recovery(time: Res<Time>, mut runtime: ResMut<WorkshopRuntime>) {
    if runtime.pending_recovery.is_some() || runtime.close_confirmation {
        return;
    }
    let now = time.elapsed_secs_f64();
    if !has_unsaved_work(&runtime) {
        if runtime.recovery_autosave.last_written.is_some()
            && !runtime.recovery_conflict
            && now >= runtime.recovery_autosave.next_retry_seconds
        {
            match discard_recovery_file(&mut runtime) {
                Ok(_) => {
                    runtime.recovery_base_revisions = None;
                    runtime.recovery_autosave = RecoveryAutosave::default();
                }
                Err(error) => {
                    runtime.recovery_autosave.next_retry_seconds = now + RECOVERY_IDLE_SECONDS;
                    runtime.set_status(WorkshopStatusKind::Error, error);
                }
            }
        } else if !runtime.recovery_conflict {
            runtime.recovery_autosave.dirty_since_seconds = None;
            runtime.recovery_autosave.last_change_seconds = None;
        }
        return;
    }

    let Ok(session) = recoverable_session(&runtime) else {
        return;
    };
    if runtime.recovery_autosave.last_observed.as_ref() != Some(&session) {
        runtime.recovery_autosave.last_observed = Some(session.clone());
        runtime
            .recovery_autosave
            .dirty_since_seconds
            .get_or_insert(now);
        runtime.recovery_autosave.last_change_seconds = Some(now);
    }
    if runtime.recovery_autosave.last_written.as_ref() == Some(&session)
        || now < runtime.recovery_autosave.next_retry_seconds
        || runtime
            .draft
            .as_ref()
            .is_some_and(|draft| draft.editor().is_transaction_open())
    {
        return;
    }

    if !recovery_write_due(&runtime.recovery_autosave, now) {
        return;
    }
    match write_recovery_now(&mut runtime) {
        Ok(_) => {
            runtime.recovery_autosave.dirty_since_seconds = None;
            runtime.recovery_autosave.last_change_seconds = None;
            runtime.recovery_autosave.next_retry_seconds = 0.0;
        }
        Err(error) => {
            runtime.recovery_autosave.next_retry_seconds = now + RECOVERY_IDLE_SECONDS;
            runtime.set_status(WorkshopStatusKind::Error, error);
        }
    }
}

fn recovery_write_due(autosave: &RecoveryAutosave, now_seconds: f64) -> bool {
    autosave
        .last_change_seconds
        .is_some_and(|changed| now_seconds - changed >= RECOVERY_IDLE_SECONDS)
        || autosave
            .dirty_since_seconds
            .is_some_and(|started| now_seconds - started >= RECOVERY_MAX_INTERVAL_SECONDS)
}

fn has_unsaved_work(runtime: &WorkshopRuntime) -> bool {
    let Some(project) = runtime.project.as_ref() else {
        return false;
    };
    let Some(draft) = runtime.draft.as_ref() else {
        return false;
    };
    runtime.recovery_conflict
        || matches!(runtime.document, OpenDocument::Unsaved(_))
        || draft.palette() != project.palette()
        || draft.styles() != project.styles()
        || draft.editor().is_dirty()
}

fn recoverable_session(runtime: &WorkshopRuntime) -> Result<RecoverableSession, String> {
    let draft = runtime
        .draft
        .as_ref()
        .ok_or_else(|| runtime.load_error_message())?;
    Ok(RecoverableSession {
        document: recovery_document(&runtime.document),
        workshop: draft.recovery_snapshot(),
    })
}

fn write_recovery_now(runtime: &mut WorkshopRuntime) -> Result<bool, String> {
    if !has_unsaved_work(runtime) {
        return Ok(false);
    }
    let session = recoverable_session(runtime)?;
    let store = runtime
        .recovery_store
        .as_ref()
        .cloned()
        .ok_or_else(|| "recovery storage is unavailable".to_owned())?;
    let base_revisions = runtime
        .recovery_base_revisions
        .clone()
        .or_else(|| {
            runtime
                .project
                .as_ref()
                .map(AssetProject::revision_snapshot)
        })
        .ok_or_else(|| "tracked art revisions are unavailable".to_owned())?;
    let envelope = RecoveryEnvelope::new(
        unix_timestamp_millis()?,
        base_revisions.clone(),
        session.document.clone(),
        session.workshop.clone(),
    )
    .map_err(|error| error.to_string())?;
    store.write(&envelope).map_err(|error| error.to_string())?;
    let written_session = RecoverableSession {
        document: envelope.document,
        workshop: envelope.workshop,
    };
    runtime.recovery_base_revisions = Some(base_revisions);
    runtime.recovery_autosave.last_observed = Some(written_session.clone());
    runtime.recovery_autosave.last_written = Some(written_session);
    Ok(true)
}

fn unix_timestamp_millis() -> Result<u64, String> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock precedes Unix epoch: {error}"))?;
    Ok(u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
}

fn poll_external_changes(
    time: Res<Time>,
    mut poll: ResMut<ProjectChangePoll>,
    mut runtime: ResMut<WorkshopRuntime>,
) {
    poll.0.tick(time.delta());
    if !poll.0.just_finished() {
        return;
    }
    let Some(project) = runtime.project.as_ref() else {
        return;
    };
    match project.external_changes() {
        Ok(changes) if changes != runtime.external_changes => {
            let newly_conflicted = runtime.external_changes.is_empty() && !changes.is_empty();
            runtime.external_changes = changes;
            if newly_conflicted {
                runtime.set_status(
                    WorkshopStatusKind::Warning,
                    "Tracked art files changed outside this editor; reload or use Save As",
                );
            } else {
                runtime.needs_sync = true;
            }
        }
        Ok(_) => {}
        Err(error) => runtime.set_status(WorkshopStatusKind::Error, error.to_string()),
    }
}

fn synchronize_views(
    mut runtime: ResMut<WorkshopRuntime>,
    mut ui_snapshot: ResMut<WorkshopUiSnapshot>,
    mut viewport_updates: MessageWriter<ViewportContentUpdate>,
    mut viewport_mode: ResMut<ViewportMode>,
    mut preview_rig: ResMut<ViewportPreviewRig>,
) {
    if !runtime.needs_sync {
        return;
    }
    let Some(project) = runtime.project.as_ref() else {
        ui_snapshot.set_load_failure(runtime.load_error_message());
        runtime.needs_sync = false;
        return;
    };
    let Some(draft) = runtime.draft.as_ref() else {
        ui_snapshot.set_load_failure(runtime.load_error_message());
        runtime.needs_sync = false;
        return;
    };
    let recovery_prompt = pending_recovery_prompt(&runtime);

    ui_snapshot.update_from(
        project,
        draft.palette(),
        draft.styles(),
        draft.editor(),
        draft.undo_label(),
        draft.redo_label(),
        match runtime.document {
            OpenDocument::Calibration => WorkshopDocumentState::Calibration,
            OpenDocument::Unsaved(_) => WorkshopDocumentState::Unsaved,
            OpenDocument::Saved(_) => WorkshopDocumentState::Saved,
        },
        &runtime.external_changes,
        recovery_prompt,
        runtime.recovery_conflict,
        runtime.recovery_catalogs_reconciled,
        review_is_ready(&runtime),
        runtime.review_in_progress,
        runtime.close_confirmation,
        runtime.status.clone(),
    );
    *viewport_mode = match draft.editor().mode() {
        WorkshopMode::VoxelStyles => ViewportMode::StylePreview,
        WorkshopMode::Objects => ViewportMode::Object,
    };
    *preview_rig = match draft.editor().preview_rig() {
        PreviewRig::Neutral => ViewportPreviewRig::Neutral,
        PreviewRig::Dark => ViewportPreviewRig::Dark,
        PreviewRig::Unlit => ViewportPreviewRig::Unlit,
    };
    viewport_updates.write(ViewportContentUpdate::new(build_viewport_content(
        draft,
        &runtime.preview,
        runtime.overlays,
    )));
    runtime.needs_sync = false;
}

fn pending_recovery_prompt(runtime: &WorkshopRuntime) -> Option<RecoveryPrompt> {
    match runtime.pending_recovery.as_ref()? {
        PendingRecovery::Available(envelope) => {
            let baseline_conflict = runtime
                .project
                .as_ref()
                .is_some_and(|project| envelope.base_revisions != project.revision_snapshot());
            Some(RecoveryPrompt::Available {
                written_unix_ms: envelope.written_unix_ms,
                document: recovery_document_label(&envelope.document),
                baseline_conflict,
            })
        }
        PendingRecovery::Invalid(message) => Some(RecoveryPrompt::Invalid {
            message: message.clone(),
        }),
    }
}

fn review_is_ready(runtime: &WorkshopRuntime) -> bool {
    if runtime.review_in_progress
        || runtime.pending_recovery.is_some()
        || !runtime.external_changes.is_empty()
        || !matches!(runtime.document, OpenDocument::Saved(_))
        || has_unsaved_work(runtime)
    {
        return false;
    }
    runtime
        .draft
        .as_ref()
        .is_some_and(|draft| draft.editor().blueprint_for_save(draft.styles()).is_ok())
}

fn build_review_capture_request(runtime: &WorkshopRuntime) -> Result<ReviewCaptureRequest, String> {
    if !review_is_ready(runtime) {
        return Err(
            "review export requires a clean saved object with no recovery or disk conflicts"
                .to_owned(),
        );
    }
    let project = runtime
        .project
        .as_ref()
        .ok_or_else(|| runtime.load_error_message())?;
    let draft = runtime
        .draft
        .as_ref()
        .ok_or_else(|| runtime.load_error_message())?;
    let object = draft
        .editor()
        .blueprint_for_save(draft.styles())
        .map_err(|error| error.to_string())?;
    let report = ReviewReport::new(&object, draft.styles(), draft.palette())
        .map_err(|error| error.to_string())?;

    let contents = build_review_viewport_contents(draft);

    ReviewCaptureRequest::new(
        project.repository_root().to_path_buf(),
        project.revision_snapshot(),
        report,
        contents,
    )
}

fn build_review_viewport_contents(draft: &WorkshopDraft) -> Vec<ViewportContent> {
    let mut authored = build_object_viewport_content(draft, OverlaySettings::default());
    authored.selected_cells.clear();
    let mut semantic = authored.clone();
    semantic.show_semantic_overlay = true;
    let mut blocker_canopy = authored.clone();
    blocker_canopy.show_blocker_overlay = true;
    blocker_canopy.show_canopy_overlay = true;
    REVIEW_FRAME_SPECS
        .iter()
        .map(|spec| match spec.presentation {
            ReviewPresentation::Authored => authored.clone(),
            ReviewPresentation::SemanticParts => semantic.clone(),
            ReviewPresentation::BlockerCanopy => blocker_canopy.clone(),
        })
        .collect()
}

fn build_viewport_content(
    draft: &WorkshopDraft,
    preview: &PreviewSubject,
    overlays: OverlaySettings,
) -> ViewportContent {
    let mut content = build_object_viewport_content(draft, overlays);

    if draft.editor().mode() == WorkshopMode::Objects {
        return content;
    }

    content.voxels.clear();
    match preview {
        PreviewSubject::Swatch(id) => {
            if let Some(swatch) = draft.palette().get(id) {
                if let Ok(preview_id) = VoxelStyleId::new("editor/swatch-preview") {
                    content.styles.insert(
                        preview_id.clone(),
                        ViewportStyle {
                            color: swatch.color(),
                            surface_mode: hex_assets::VoxelSurfaceMode::Opaque,
                            opacity: 1.0,
                            emission: None,
                        },
                    );
                    content.set_voxels(vec![RenderedVoxel {
                        position: LocalVoxelCoord::new(0, 0, 0),
                        style: preview_id,
                    }]);
                }
            }
        }
        PreviewSubject::Style(id) => {
            if content.styles.contains_key(id) {
                content.set_voxels(vec![RenderedVoxel {
                    position: LocalVoxelCoord::new(0, 0, 0),
                    style: id.clone(),
                }]);
            }
        }
        PreviewSubject::ActiveStyle => {
            if let Some(id) = draft
                .editor()
                .active_style()
                .filter(|id| content.styles.contains_key(*id))
            {
                content.set_voxels(vec![RenderedVoxel {
                    position: LocalVoxelCoord::new(0, 0, 0),
                    style: id.clone(),
                }]);
            }
        }
    }
    content
}

fn build_object_viewport_content(
    draft: &WorkshopDraft,
    overlays: OverlaySettings,
) -> ViewportContent {
    let mut content = ViewportContent {
        grid_radius: draft.editor().object().bounds.radius,
        active_level: draft.editor().active_level(),
        isolate_active_level: overlays.isolate_active_level,
        show_grid: overlays.grid,
        selected_cells: draft.editor().selection().cells().clone(),
        semantic_parts: draft
            .editor()
            .object()
            .placements
            .iter()
            .map(|placement| (placement.position, placement.part))
            .collect(),
        blocker_columns: draft
            .editor()
            .object()
            .blocker_footprint
            .iter()
            .copied()
            .collect(),
        canopy_cells: draft
            .editor()
            .object()
            .canopy_occluders
            .iter()
            .copied()
            .collect(),
        show_semantic_overlay: overlays.semantics,
        show_blocker_overlay: overlays.blockers,
        show_canopy_overlay: overlays.canopy,
        ..default()
    };
    let resolved = resolve_styles(draft.palette(), draft.styles());
    content.set_styles(resolved);
    content.set_voxels(
        draft
            .editor()
            .object()
            .placements
            .iter()
            .map(|placement| RenderedVoxel {
                position: placement.position,
                style: placement.style.clone(),
            })
            .collect(),
    );
    content
}

fn resolve_styles(
    palette: &hex_assets::ArtPalette,
    styles: &VoxelStyleCatalog,
) -> BTreeMap<VoxelStyleId, ViewportStyle> {
    styles
        .styles()
        .iter()
        .filter_map(|(id, style)| {
            let base = palette.get(style.base_swatch())?;
            let emission = style.emission().and_then(|emission| {
                palette
                    .get(emission.swatch())
                    .map(|swatch| ViewportEmission {
                        color: swatch.color(),
                        strength: emission.strength(),
                    })
            });
            Some((
                id.clone(),
                ViewportStyle {
                    color: base.color(),
                    surface_mode: style.surface_mode(),
                    opacity: style.opacity(),
                    emission,
                },
            ))
        })
        .collect()
}

const fn camera_snap(snap: EditorCameraSnap) -> CameraSnap {
    match snap {
        EditorCameraSnap::Perspective => CameraSnap::Perspective,
        EditorCameraSnap::Top => CameraSnap::Top,
        EditorCameraSnap::Front => CameraSnap::Front,
        EditorCameraSnap::Side => CameraSnap::Side,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    use hex_assets::{
        LocalAxialCoord, ObjectBlueprint, ObjectBounds, ObjectPart, ObjectPlacement, PaletteSwatch,
        PlantPart, SrgbColor, VoxelStyle, VoxelSurfaceMode, OBJECT_BLUEPRINT_SCHEMA_VERSION,
    };
    use serde::Serialize;

    use super::*;

    static TEST_PROJECT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestProject {
        root: PathBuf,
    }

    impl TestProject {
        fn new(palette: &ArtPalette, styles: &VoxelStyleCatalog) -> Self {
            let sequence = TEST_PROJECT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "hex-editor-app-test-{}-{sequence}",
                std::process::id()
            ));
            let art_root = root.join("assets/art");
            fs::create_dir_all(&art_root).expect("test art directory should be created");
            write_ron(&art_root.join("palette.ron"), palette);
            write_ron(&art_root.join("voxel_styles.ron"), styles);
            Self { root }
        }
    }

    impl Drop for TestProject {
        fn drop(&mut self) {
            drop(fs::remove_dir_all(&self.root));
        }
    }

    fn write_ron(path: &Path, value: &impl Serialize) {
        let source = ron::ser::to_string_pretty(value, ron::ser::PrettyConfig::default())
            .expect("fixture RON should encode");
        fs::write(path, format!("{source}\n")).expect("fixture RON should be written");
    }

    fn fixture_palette() -> ArtPalette {
        ArtPalette::new(BTreeMap::from([
            (
                SwatchId::new("plant/base").expect("fixture swatch id should be valid"),
                PaletteSwatch::new(
                    "Plant Base",
                    SrgbColor::new(0.2, 0.3, 0.2).expect("fixture colour should be valid"),
                    BTreeSet::from(["plant".to_owned()]),
                )
                .expect("fixture swatch should be valid"),
            ),
            (
                SwatchId::new("plant/accent").expect("fixture swatch id should be valid"),
                PaletteSwatch::new(
                    "Plant Accent",
                    SrgbColor::new(0.4, 0.6, 0.3).expect("fixture colour should be valid"),
                    BTreeSet::from(["plant".to_owned()]),
                )
                .expect("fixture swatch should be valid"),
            ),
        ]))
        .expect("fixture palette should be valid")
    }

    fn fixture_styles() -> VoxelStyleCatalog {
        VoxelStyleCatalog::new(BTreeMap::from([(
            VoxelStyleId::new("plant/base").expect("fixture style id should be valid"),
            VoxelStyle::new(
                "Plant Base",
                SwatchId::new("plant/base").expect("fixture swatch id should be valid"),
                VoxelSurfaceMode::Opaque,
                1.0,
                None,
            )
            .expect("fixture style should be valid"),
        )]))
        .expect("fixture styles should be valid")
    }

    fn fixture_editor() -> EditorModel {
        let style = VoxelStyleId::new("plant/base").expect("fixture style id should be valid");
        EditorModel::blank(ObjectCategory::Plant, ConnectivityPolicy::Grounded, style)
            .expect("fixture editor should be valid")
    }

    fn fixture_runtime(
        project: AssetProject,
        draft: WorkshopDraft,
        document: OpenDocument,
        recovery_store: RecoveryStore,
    ) -> WorkshopRuntime {
        WorkshopRuntime {
            project: Some(project),
            draft: Some(draft),
            document,
            preview: PreviewSubject::ActiveStyle,
            overlays: OverlaySettings::default(),
            status: None,
            load_failure: None,
            external_changes: Vec::new(),
            recovery_store: Some(recovery_store),
            pending_recovery: None,
            recovery_base_revisions: None,
            recovery_conflict: false,
            recovery_catalogs_reconciled: false,
            recovery_object_requires_save_as: false,
            recovery_autosave: RecoveryAutosave::default(),
            review_in_progress: false,
            close_confirmation: false,
            exit_requested: false,
            needs_sync: false,
        }
    }

    fn face_target(
        cell: LocalVoxelCoord,
        source: ViewportPickSource,
        normal: Vec3,
    ) -> ViewportFaceTarget {
        ViewportFaceTarget {
            entity: Entity::PLACEHOLDER,
            cell,
            source,
            world_position: Vec3::ZERO,
            normal,
        }
    }

    #[test]
    fn placement_uses_grid_cells_and_every_clicked_voxel_face() {
        let cell = LocalVoxelCoord::new(2, -1, 4);
        assert_eq!(
            placement_cell(face_target(cell, ViewportPickSource::Grid, Vec3::Y)),
            Some(cell)
        );
        assert_eq!(
            placement_cell(face_target(cell, ViewportPickSource::Voxel, Vec3::Y)),
            Some(LocalVoxelCoord::new(2, -1, 5))
        );
        assert_eq!(
            placement_cell(face_target(cell, ViewportPickSource::Voxel, Vec3::NEG_Y)),
            Some(LocalVoxelCoord::new(2, -1, 3))
        );
        for (normal, expected) in [
            (Vec3::X, LocalVoxelCoord::new(3, -1, 4)),
            (
                Vec3::new(0.5, 0.0, 0.866_025_4),
                LocalVoxelCoord::new(2, 0, 4),
            ),
            (
                Vec3::new(-0.5, 0.0, 0.866_025_4),
                LocalVoxelCoord::new(1, 0, 4),
            ),
            (Vec3::NEG_X, LocalVoxelCoord::new(1, -1, 4)),
        ] {
            assert_eq!(
                placement_cell(face_target(cell, ViewportPickSource::Voxel, normal)),
                Some(expected)
            );
        }
    }

    #[test]
    fn stationary_pointer_cannot_cascade_across_new_hover_targets() {
        let mut stroke = PointerStroke::default();
        let cursor = Some(Vec2::new(400.0, 300.0));

        assert!(stroke.accepts_cell(LocalVoxelCoord::new(0, 0, 0), cursor));
        assert!(!stroke.accepts_cell(LocalVoxelCoord::new(0, 0, 1), cursor));
        assert!(!stroke.accepts_cell(LocalVoxelCoord::new(0, 0, 1), Some(Vec2::new(403.0, 300.0))));
        assert!(stroke.accepts_cell(LocalVoxelCoord::new(0, 0, 1), Some(Vec2::new(404.0, 300.0))));
    }

    #[test]
    fn pointer_stroke_without_cursor_data_places_only_its_first_cell() {
        let mut stroke = PointerStroke::default();

        assert!(stroke.accepts_cell(LocalVoxelCoord::new(0, 0, 0), None));
        assert!(!stroke.accepts_cell(LocalVoxelCoord::new(1, 0, 0), None));
    }

    fn boundary_stroke_editor() -> EditorModel {
        let mut object = EditorModel::calibration_scene()
            .expect("calibration scene should be valid")
            .object()
            .clone();
        object.bounds = ObjectBounds {
            radius: 3,
            min_level: 0,
            height: 6,
        };
        let mut editor =
            EditorModel::from_blueprint(object).expect("boundary fixture should be valid");
        editor
            .set_active_part(ObjectPart::Plant(PlantPart::Trunk))
            .expect("trunk should be valid for a plant");
        editor
    }

    #[test]
    fn boundary_contact_skips_only_that_cell_and_preserves_the_valid_stroke() {
        let mut editor = boundary_stroke_editor();
        let valid = LocalVoxelCoord::new(0, 3, 5);
        let beyond_top = LocalVoxelCoord::new(0, 3, 6);

        editor
            .begin_transaction("Place stroke")
            .expect("stroke should begin");
        assert_eq!(
            preflight_pointer_stroke_cell(&editor, EditorTool::Place, valid),
            Ok(())
        );
        assert_eq!(editor.place_active(valid), Ok(true));
        let warning = preflight_pointer_stroke_cell(&editor, EditorTool::Place, beyond_top)
            .expect_err("boundary contact should be skipped");
        assert!(warning.contains("above authoring maximum 5"));
        assert_eq!(editor.commit_transaction(), Ok(true));

        assert!(editor
            .object()
            .placements
            .iter()
            .any(|placement| placement.position == valid));
        assert!(!editor
            .object()
            .placements
            .iter()
            .any(|placement| placement.position == beyond_top));
    }

    #[test]
    fn boundary_skips_are_coalesced_into_one_stroke_summary() {
        let mut stroke = PointerStroke::default();
        assert!(stroke.boundary_skip_summary().is_none());

        stroke.record_boundary_skip("first boundary detail".to_owned());
        stroke.record_boundary_skip("second boundary detail".to_owned());

        let summary = stroke
            .boundary_skip_summary()
            .expect("recorded skips should produce one summary");
        assert!(summary.contains("Skipped 2 out-of-bounds placement cells"));
        assert!(summary.contains("valid cells in the stroke were kept"));
        assert!(summary.contains("First skipped cell: first boundary detail"));
        assert!(!summary.contains("second boundary detail"));
    }

    #[test]
    fn entirely_out_of_bounds_pointer_stroke_is_a_no_op() {
        let mut editor = boundary_stroke_editor();
        let original = editor.object().clone();
        let beyond_radius = LocalVoxelCoord::new(0, 4, 0);

        editor
            .begin_transaction("Place stroke")
            .expect("stroke should begin");
        let warning = preflight_pointer_stroke_cell(&editor, EditorTool::Place, beyond_radius)
            .expect_err("boundary contact should be skipped");
        assert!(warning.contains("outside authoring radius 3"));
        assert_eq!(editor.commit_transaction(), Ok(false));
        assert_eq!(editor.object(), &original);
    }

    #[test]
    fn object_ids_match_their_singular_category_directory() {
        let plant = ObjectAssetId::new("plant/oak").expect("fixture id should be valid");
        assert!(validate_object_id_category(&plant, ObjectCategory::Plant).is_ok());
        assert!(validate_object_id_category(&plant, ObjectCategory::Effect).is_err());
        let nested =
            ObjectAssetId::new("plant/trees/oak").expect("path-like fixture id should be valid");
        assert!(validate_object_id_category(&nested, ObjectCategory::Plant).is_err());
    }

    #[test]
    fn rejected_save_as_does_not_mutate_the_live_draft() {
        let editor = EditorModel::blank(
            ObjectCategory::Plant,
            ConnectivityPolicy::Grounded,
            VoxelStyleId::new("plant/base").expect("fixture style id should be valid"),
        )
        .expect("fixture editor should be valid");
        let draft = WorkshopDraft::new(fixture_palette(), fixture_styles(), editor);
        let document = OpenDocument::Unsaved(
            ObjectAssetId::new("plant/untitled").expect("fixture id should be valid"),
        );
        let mut runtime = WorkshopRuntime {
            project: None,
            draft: Some(draft),
            document: document.clone(),
            preview: PreviewSubject::ActiveStyle,
            overlays: OverlaySettings::default(),
            status: None,
            load_failure: None,
            external_changes: Vec::new(),
            recovery_store: None,
            pending_recovery: None,
            recovery_base_revisions: None,
            recovery_conflict: false,
            recovery_catalogs_reconciled: false,
            recovery_object_requires_save_as: false,
            recovery_autosave: RecoveryAutosave::default(),
            review_in_progress: false,
            close_confirmation: false,
            exit_requested: false,
            needs_sync: false,
        };
        let before = runtime
            .draft
            .as_ref()
            .expect("fixture draft should exist")
            .editor()
            .clone();

        let result = save_current_object_as(
            &mut runtime,
            ObjectAssetId::new("effect/wrong-category").expect("fixture id should be valid"),
            "Rejected rename".to_owned(),
        );

        assert!(result.is_err());
        assert_eq!(runtime.document, document);
        let draft = runtime.draft.as_ref().expect("fixture draft should remain");
        assert_eq!(draft.editor(), &before);
        assert_eq!(draft.undo_label(), None);
        assert_eq!(draft.redo_label(), None);
    }

    #[test]
    fn calibration_scene_uses_a_real_catalog_style_when_available() {
        let mut editor =
            EditorModel::calibration_scene().expect("calibration scene should be valid");
        let style =
            VoxelStyleId::new("plant/calibration").expect("fixture style id should be valid");

        apply_calibration_style(&mut editor, &style)
            .expect("calibration style replacement should succeed");

        assert!(!editor.object().placements.is_empty());
        assert!(editor
            .object()
            .placements
            .iter()
            .all(|placement| placement.style == style));
    }

    #[test]
    fn recovery_waits_for_idle_but_caps_continuous_editing() {
        let mut autosave = RecoveryAutosave {
            dirty_since_seconds: Some(10.0),
            last_change_seconds: Some(20.0),
            ..default()
        };
        assert!(!recovery_write_due(&autosave, 22.9));
        assert!(recovery_write_due(&autosave, 23.0));

        autosave.last_change_seconds = Some(39.9);
        assert!(!recovery_write_due(&autosave, 39.9));
        assert!(recovery_write_due(&autosave, 40.0));
    }

    #[test]
    fn successful_catalog_save_refreshes_the_recovery_baseline() {
        let palette = fixture_palette();
        let styles = fixture_styles();
        let directory = TestProject::new(&palette, &styles);
        let project = AssetProject::load(&directory.root).expect("fixture project should load");
        let original_revisions = project.revision_snapshot();
        let mut draft = WorkshopDraft::new(palette, styles, fixture_editor());
        let changed = PaletteSwatch::new(
            "Plant Base",
            SrgbColor::new(0.28, 0.36, 0.22).expect("fixture colour should be valid"),
            BTreeSet::from(["plant".to_owned()]),
        )
        .expect("fixture swatch should be valid");
        assert_eq!(
            draft.upsert_swatch(
                SwatchId::new("plant/base").expect("fixture swatch id should be valid"),
                changed,
                true,
            ),
            Ok(true)
        );
        let mut runtime = fixture_runtime(
            project,
            draft,
            OpenDocument::Calibration,
            RecoveryStore::new(&directory.root),
        );
        runtime.recovery_base_revisions = Some(original_revisions.clone());

        apply_ui_action(WorkshopUiAction::SaveCatalogs, &mut runtime, None)
            .expect("catalog save should succeed");

        let saved_revisions = runtime
            .project
            .as_ref()
            .expect("project should remain loaded")
            .revision_snapshot();
        assert_ne!(saved_revisions, original_revisions);
        assert_eq!(
            runtime.recovery_base_revisions.as_ref(),
            Some(&saved_revisions)
        );
    }

    #[test]
    fn successful_object_save_refreshes_the_recovery_baseline() {
        let palette = fixture_palette();
        let styles = fixture_styles();
        let directory = TestProject::new(&palette, &styles);
        let mut project = AssetProject::load(&directory.root).expect("fixture project should load");
        let saved_id =
            ObjectAssetId::new("plant/saved").expect("fixture object id should be valid");
        let blueprint = fixture_editor()
            .blueprint_for_save(&styles)
            .expect("fixture object should be valid");
        project
            .save_object_as(blueprint, saved_id.clone())
            .expect("fixture object should save");
        let original_revisions = project.revision_snapshot();
        let saved_blueprint = project
            .object(&saved_id)
            .cloned()
            .expect("saved fixture should be indexed");
        let mut editor =
            EditorModel::from_blueprint(saved_blueprint).expect("saved fixture should open");
        assert_eq!(
            editor.set_display_name("Edited Saved Object".to_owned()),
            Ok(true)
        );
        let draft = WorkshopDraft::new(palette, styles, editor);
        let mut runtime = fixture_runtime(
            project,
            draft,
            OpenDocument::Saved(saved_id),
            RecoveryStore::new(&directory.root),
        );
        runtime.recovery_base_revisions = Some(original_revisions.clone());

        save_current_object(&mut runtime).expect("existing object should save");

        let saved_revisions = runtime
            .project
            .as_ref()
            .expect("project should remain loaded")
            .revision_snapshot();
        assert_ne!(saved_revisions, original_revisions);
        assert_eq!(
            runtime.recovery_base_revisions.as_ref(),
            Some(&saved_revisions)
        );
    }

    #[test]
    fn recovered_catalogs_reconcile_local_and_tracked_changes_without_loss() {
        let palette = fixture_palette();
        let styles = fixture_styles();
        let directory = TestProject::new(&palette, &styles);
        let base_project =
            AssetProject::load(&directory.root).expect("fixture project should load");
        let base_revisions = base_project.revision_snapshot();
        let mut recovered = WorkshopDraft::new(palette.clone(), styles.clone(), fixture_editor());
        let local_base = PaletteSwatch::new(
            "Recovered Base",
            SrgbColor::new(0.26, 0.38, 0.24).expect("fixture colour should be valid"),
            BTreeSet::from(["plant".to_owned()]),
        )
        .expect("fixture swatch should be valid");
        assert_eq!(
            recovered.upsert_swatch(
                SwatchId::new("plant/base").expect("fixture swatch id should be valid"),
                local_base.clone(),
                true,
            ),
            Ok(true)
        );
        let envelope = RecoveryEnvelope::new(
            1,
            base_revisions,
            RecoveryDocument::Unsaved(
                ObjectAssetId::new("plant/untitled").expect("fixture object id should be valid"),
            ),
            recovered.recovery_snapshot(),
        )
        .expect("fixture recovery should be valid");

        let mut current_project =
            AssetProject::load(&directory.root).expect("fixture project should reload");
        let mut current_palette = palette;
        let tracked_accent = PaletteSwatch::new(
            "Tracked Accent",
            SrgbColor::new(0.5, 0.7, 0.34).expect("fixture colour should be valid"),
            BTreeSet::from(["plant".to_owned()]),
        )
        .expect("fixture swatch should be valid");
        current_palette
            .insert(
                SwatchId::new("plant/accent").expect("fixture swatch id should be valid"),
                tracked_accent.clone(),
            )
            .expect("fixture palette edit should be valid");
        current_project
            .save_palette(current_palette)
            .expect("tracked fixture edit should save");
        let clean_draft = WorkshopDraft::new(
            current_project.palette().clone(),
            current_project.styles().clone(),
            fixture_editor(),
        );
        let mut runtime = fixture_runtime(
            current_project,
            clean_draft,
            OpenDocument::Calibration,
            RecoveryStore::new(&directory.root),
        );
        runtime.pending_recovery = Some(PendingRecovery::Available(Box::new(envelope)));

        restore_pending_recovery(&mut runtime).expect("recovery should restore");
        assert!(runtime.recovery_conflict);
        reconcile_recovered_catalogs(&mut runtime)
            .expect("independent catalog edits should reconcile");

        assert!(!runtime.recovery_conflict);
        let draft = runtime.draft.as_ref().expect("draft should remain loaded");
        assert_eq!(
            draft
                .palette()
                .get(&SwatchId::new("plant/base").expect("fixture id should be valid")),
            Some(&local_base)
        );
        assert_eq!(
            draft
                .palette()
                .get(&SwatchId::new("plant/accent").expect("fixture id should be valid")),
            Some(&tracked_accent)
        );
        apply_ui_action(WorkshopUiAction::SaveCatalogs, &mut runtime, None)
            .expect("rebased catalogs should save");
        assert_eq!(
            runtime.recovery_base_revisions,
            runtime
                .project
                .as_ref()
                .map(AssetProject::revision_snapshot)
        );
    }

    #[test]
    fn conflicted_saved_object_can_reconcile_catalogs_then_exit_through_save_as() {
        let palette = fixture_palette();
        let styles = fixture_styles();
        let directory = TestProject::new(&palette, &styles);
        let project = AssetProject::load(&directory.root).expect("fixture project should load");
        let mut editor = fixture_editor();
        let original_id =
            ObjectAssetId::new("plant/recovered-original").expect("fixture id should be valid");
        editor
            .set_unsaved_identity(original_id.clone(), "Recovered Original".to_owned())
            .expect("fixture identity should be valid");
        editor.mark_saved();
        assert_eq!(
            editor.set_display_name("Recovered Local Edit".to_owned()),
            Ok(true)
        );
        let recovered = WorkshopDraft::new(palette.clone(), styles.clone(), editor);
        let envelope = RecoveryEnvelope::new(
            2,
            ProjectRevisionSet::default(),
            RecoveryDocument::Saved(original_id),
            recovered.recovery_snapshot(),
        )
        .expect("fixture recovery should be valid");
        let clean_draft = WorkshopDraft::new(palette, styles, fixture_editor());
        let mut runtime = fixture_runtime(
            project,
            clean_draft,
            OpenDocument::Calibration,
            RecoveryStore::new(&directory.root),
        );
        runtime.pending_recovery = Some(PendingRecovery::Available(Box::new(envelope)));

        restore_pending_recovery(&mut runtime).expect("recovery should restore");
        assert!(runtime.recovery_object_requires_save_as);
        let conflicted_baseline = runtime.recovery_base_revisions.clone();
        reconcile_recovered_catalogs(&mut runtime).expect("unchanged catalogs should reconcile");
        assert!(runtime.recovery_conflict);
        assert!(runtime.recovery_catalogs_reconciled);
        assert_eq!(runtime.recovery_base_revisions, conflicted_baseline);
        assert!(ensure_catalog_save_allowed(&runtime).is_ok());

        let rescued_id =
            ObjectAssetId::new("plant/recovered-copy").expect("fixture id should be valid");
        save_current_object_as(
            &mut runtime,
            rescued_id.clone(),
            "Recovered Copy".to_owned(),
        )
        .expect("Save As should preserve the recovered object");

        assert!(!runtime.recovery_conflict);
        assert!(!runtime.recovery_object_requires_save_as);
        assert_eq!(
            runtime.recovery_base_revisions,
            runtime
                .project
                .as_ref()
                .map(AssetProject::revision_snapshot)
        );
        assert!(runtime
            .project
            .as_ref()
            .and_then(|project| project.object(&rescued_id))
            .is_some());
    }

    #[test]
    fn clean_session_discards_obsolete_recovery_state() {
        let palette = fixture_palette();
        let styles = fixture_styles();
        let directory = TestProject::new(&palette, &styles);
        let project = AssetProject::load(&directory.root).expect("fixture project should load");
        let (editor, _) =
            calibration_for_project(&project).expect("calibration fixture should be valid");
        let draft = WorkshopDraft::new(palette, styles, editor);
        let store = RecoveryStore::new(&directory.root);
        let mut runtime = fixture_runtime(project, draft, OpenDocument::Calibration, store.clone());
        let session = recoverable_session(&runtime).expect("fixture should be recoverable");
        let envelope = RecoveryEnvelope::new(
            3,
            runtime
                .project
                .as_ref()
                .expect("project should exist")
                .revision_snapshot(),
            session.document.clone(),
            session.workshop.clone(),
        )
        .expect("fixture recovery should be valid");
        store
            .write(&envelope)
            .expect("fixture recovery should write");
        runtime.recovery_base_revisions = Some(envelope.base_revisions);
        runtime.recovery_autosave.last_observed = Some(session.clone());
        runtime.recovery_autosave.last_written = Some(session);

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(runtime)
            .add_systems(Update, autosave_recovery);
        app.update();

        let runtime = app.world().resource::<WorkshopRuntime>();
        assert!(runtime.recovery_base_revisions.is_none());
        assert!(runtime.recovery_autosave.last_written.is_none());
        assert!(!store.path().exists());
    }

    #[test]
    fn review_rejection_clears_the_modal_and_reports_the_reason() {
        let palette = fixture_palette();
        let styles = fixture_styles();
        let directory = TestProject::new(&palette, &styles);
        let project = AssetProject::load(&directory.root).expect("fixture project should load");
        let draft = WorkshopDraft::new(palette, styles, fixture_editor());
        let mut runtime = fixture_runtime(
            project,
            draft,
            OpenDocument::Calibration,
            RecoveryStore::new(&directory.root),
        );
        runtime.review_in_progress = true;

        apply_review_capture_rejection(&mut runtime, "capture already running");

        assert!(!runtime.review_in_progress);
        assert_eq!(
            runtime.status,
            Some(WorkshopStatus {
                kind: WorkshopStatusKind::Error,
                message: "Review export rejected: capture already running".to_owned(),
            })
        );
    }

    #[test]
    fn three_way_catalog_merge_names_same_entry_conflicts() {
        let id = SwatchId::new("plant/base").expect("fixture id should be valid");
        let base = BTreeMap::from([(id.clone(), 1_u8)]);
        let local = BTreeMap::from([(id.clone(), 2_u8)]);
        let current = BTreeMap::from([(id.clone(), 3_u8)]);

        let (merged, conflicts) = three_way_merge_entries(&base, &local, &current);

        assert!(merged.is_empty());
        assert_eq!(conflicts, vec![id]);
    }

    #[test]
    fn window_close_decision_covers_clean_dirty_recovery_and_review_states() {
        assert_eq!(
            window_close_decision(false, false, false),
            WindowCloseDecision::Exit
        );
        assert_eq!(
            window_close_decision(false, false, true),
            WindowCloseDecision::ConfirmDirty
        );
        assert_eq!(
            window_close_decision(false, true, true),
            WindowCloseDecision::Exit
        );
        assert_eq!(
            window_close_decision(true, false, false),
            WindowCloseDecision::WaitForReview
        );
    }

    #[test]
    fn repaint_stroke_applies_the_active_style_and_semantic_role() {
        let base_swatch = SwatchId::new("plant/base").expect("fixture swatch id should be valid");
        let accent_swatch =
            SwatchId::new("plant/accent").expect("fixture swatch id should be valid");
        let palette = hex_assets::ArtPalette::new(BTreeMap::from([
            (
                base_swatch.clone(),
                PaletteSwatch::new(
                    "Base",
                    SrgbColor::new(0.2, 0.3, 0.2).expect("fixture colour should be valid"),
                    BTreeSet::from(["plant".to_owned()]),
                )
                .expect("fixture swatch should be valid"),
            ),
            (
                accent_swatch.clone(),
                PaletteSwatch::new(
                    "Accent",
                    SrgbColor::new(0.4, 0.7, 0.2).expect("fixture colour should be valid"),
                    BTreeSet::from(["plant".to_owned()]),
                )
                .expect("fixture swatch should be valid"),
            ),
        ]))
        .expect("fixture palette should be valid");
        let base_style = VoxelStyleId::new("plant/base").expect("fixture style id should be valid");
        let accent_style =
            VoxelStyleId::new("plant/accent").expect("fixture style id should be valid");
        let styles = VoxelStyleCatalog::new(BTreeMap::from([
            (
                base_style.clone(),
                VoxelStyle::new("Base", base_swatch, VoxelSurfaceMode::Opaque, 1.0, None)
                    .expect("fixture style should be valid"),
            ),
            (
                accent_style.clone(),
                VoxelStyle::new("Accent", accent_swatch, VoxelSurfaceMode::Cutout, 0.9, None)
                    .expect("fixture style should be valid"),
            ),
        ]))
        .expect("fixture style catalog should be valid");
        let mut editor = EditorModel::blank(
            ObjectCategory::Plant,
            ConnectivityPolicy::Grounded,
            base_style.clone(),
        )
        .expect("fixture editor should be valid");
        let target = LocalVoxelCoord::new(0, 0, 1);
        assert_eq!(
            editor.place(target, base_style, ObjectPart::Plant(PlantPart::Trunk),),
            Ok(true)
        );
        editor.set_active_style(Some(accent_style.clone()));
        assert_eq!(
            editor.set_active_part(ObjectPart::Plant(PlantPart::Foliage)),
            Ok(())
        );
        let mut draft = WorkshopDraft::new(palette, styles, editor);
        assert!(draft.begin_object_transaction("Repaint stroke").is_ok());

        assert_eq!(
            apply_stroke_cell(Ok(&mut draft), EditorTool::Repaint, target),
            Ok(true)
        );
        assert_eq!(draft.commit_object_transaction(), Ok(true));
        let placement = draft
            .editor()
            .object()
            .placements
            .iter()
            .find(|placement| placement.position == target)
            .expect("repainted placement should remain");
        assert_eq!(placement.style, accent_style);
        assert_eq!(placement.part, ObjectPart::Plant(PlantPart::Foliage));
    }

    #[test]
    fn review_contents_ignore_transient_editor_presentation() {
        let swatch_id =
            SwatchId::new("plant/review-green").expect("fixture swatch id should be valid");
        let palette = hex_assets::ArtPalette::new(BTreeMap::from([(
            swatch_id.clone(),
            PaletteSwatch::new(
                "Review Green".to_owned(),
                SrgbColor::new(0.2, 0.7, 0.3).expect("fixture colour should be valid"),
                BTreeSet::from(["plant".to_owned()]),
            )
            .expect("fixture swatch should be valid"),
        )]))
        .expect("fixture palette should be valid");
        let style_id =
            VoxelStyleId::new("plant/review-leaf").expect("fixture style id should be valid");
        let styles = VoxelStyleCatalog::new(BTreeMap::from([(
            style_id.clone(),
            VoxelStyle::new(
                "Review Leaf".to_owned(),
                swatch_id,
                VoxelSurfaceMode::Opaque,
                1.0,
                None,
            )
            .expect("fixture style should be valid"),
        )]))
        .expect("fixture style catalog should be valid");
        let root = LocalVoxelCoord::new(0, 0, 0);
        let canopy = LocalVoxelCoord::new(0, 0, 1);
        let object = ObjectBlueprint {
            schema_version: OBJECT_BLUEPRINT_SCHEMA_VERSION,
            id: ObjectAssetId::new("plant/review-sprout")
                .expect("fixture object id should be valid"),
            display_name: "Review Sprout".to_owned(),
            category: ObjectCategory::Plant,
            origin: root,
            bounds: ObjectBounds {
                radius: 2,
                min_level: 0,
                height: 4,
            },
            connectivity: ConnectivityPolicy::Grounded,
            blocker_footprint: vec![LocalAxialCoord::new(0, 0)],
            canopy_occluders: vec![canopy],
            placements: vec![
                ObjectPlacement {
                    position: root,
                    style: style_id.clone(),
                    part: ObjectPart::Plant(PlantPart::Root),
                },
                ObjectPlacement {
                    position: canopy,
                    style: style_id,
                    part: ObjectPart::Plant(PlantPart::Foliage),
                },
            ],
        };
        let mut editor =
            EditorModel::from_blueprint(object).expect("fixture editor should be valid");
        editor.set_mode(WorkshopMode::VoxelStyles);
        assert!(editor.select(canopy, false));
        let draft = WorkshopDraft::new(palette, styles, editor);

        let contents = build_review_viewport_contents(&draft);

        assert_eq!(contents.len(), REVIEW_FRAME_SPECS.len());
        for (content, spec) in contents.iter().zip(REVIEW_FRAME_SPECS) {
            assert_eq!(content.voxels.len(), 2);
            assert!(!content.show_grid);
            assert!(!content.isolate_active_level);
            assert!(content.selected_cells.is_empty());
            let expected = match spec.presentation {
                ReviewPresentation::Authored => (false, false, false),
                ReviewPresentation::SemanticParts => (true, false, false),
                ReviewPresentation::BlockerCanopy => (false, true, true),
            };
            assert_eq!(
                (
                    content.show_semantic_overlay,
                    content.show_blocker_overlay,
                    content.show_canopy_overlay,
                ),
                expected
            );
        }
    }
}
