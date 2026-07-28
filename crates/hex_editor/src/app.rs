//! Application composition and adapters between UI, draft state, and viewport.

use std::collections::{BTreeMap, BTreeSet};
use std::env;

use bevy::prelude::*;
use bevy::window::{PresentMode, WindowResolution};
use bevy_egui::EguiPlugin;
use hex_assets::{
    ConnectivityPolicy, LocalVoxelCoord, ObjectAssetId, ObjectCategory, SwatchId,
    VoxelStyleCatalog, VoxelStyleId,
};

use crate::launch::resolve_repository_root;
use crate::model::{EditorModel, EditorTool, PreviewRig, WorkshopMode};
use crate::project::AssetProject;
use crate::ui::{
    EditorCameraSnap, WorkshopStatus, WorkshopStatusKind, WorkshopUiAction, WorkshopUiPlugin,
    WorkshopUiSnapshot,
};
use crate::viewport::{
    CameraSnap, CameraSnapRequest, FrameViewportRequest, HoveredFaceTarget, RenderedVoxel,
    ViewportContent, ViewportContentUpdate, ViewportEmission, ViewportFaceTarget,
    ViewportInputEnabled, ViewportMode, ViewportPickSource, ViewportPreviewRig, ViewportStyle,
    ViewportSystems,
};
use crate::workshop::WorkshopDraft;

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
    needs_sync: bool,
}

impl WorkshopRuntime {
    fn initialize() -> Self {
        let loaded =
            (|| -> Result<(AssetProject, WorkshopDraft, PreviewSubject, String), String> {
                let current_directory = env::current_dir()
                    .map_err(|error| format!("cannot read working directory: {error}"))?;
                let root = resolve_repository_root(env::args_os(), &current_directory)
                    .map_err(|error| error.to_string())?;
                let project = AssetProject::load(&root).map_err(|error| error.to_string())?;
                let mut editor =
                    EditorModel::calibration_scene().map_err(|error| error.to_string())?;
                let preview = if let Some(style) = project.styles().styles().keys().next() {
                    editor.set_active_style(Some(style.clone()));
                    PreviewSubject::ActiveStyle
                } else {
                    editor.set_mode(WorkshopMode::VoxelStyles);
                    project
                        .palette()
                        .swatches()
                        .keys()
                        .next()
                        .cloned()
                        .map_or(PreviewSubject::ActiveStyle, PreviewSubject::Swatch)
                };
                let draft =
                    WorkshopDraft::new(project.palette().clone(), project.styles().clone(), editor);
                Ok((
                    project,
                    draft,
                    preview,
                    format!("Project loaded from {}", root.display()),
                ))
            })();

        match loaded {
            Ok((project, draft, preview, message)) => Self {
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
}

/// Starts the Asset Workshop.
pub fn run() {
    let runtime = WorkshopRuntime::initialize();
    App::new()
        .insert_resource(runtime)
        .insert_resource(ClearColor(Color::srgb(0.055, 0.06, 0.07)))
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Bevy Hex Asset Workshop".to_owned(),
                name: Some("hex-editor".to_owned()),
                resolution: WindowResolution::new(1440, 900),
                present_mode: PresentMode::AutoVsync,
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .add_plugins(crate::viewport::plugin)
        .add_plugins(WorkshopUiPlugin)
        .init_resource::<PointerStroke>()
        .add_systems(
            Update,
            (handle_ui_actions, handle_pointer_editing, synchronize_views)
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
            Ok(Some("Saved palette and voxel styles".to_owned()))
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
            if runtime.document == OpenDocument::Saved(id.clone()) {
                ensure_document_can_change(runtime)?;
            }
            runtime
                .project_mut()?
                .delete_object(&id)
                .map_err(|error| error.to_string())?;
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
            Ok(Some(format!("Saved {}", proposed_id.as_str())))
        }
        OpenDocument::Saved(id) => {
            runtime
                .project_mut()?
                .save_object(&id, blueprint)
                .map_err(|error| error.to_string())?;
            runtime.draft_mut()?.mark_object_saved();
            Ok(Some(format!("Saved {}", id.as_str())))
        }
    }
}

fn save_current_object_as(
    runtime: &mut WorkshopRuntime,
    id: ObjectAssetId,
    display_name: String,
) -> Result<Option<String>, String> {
    {
        let draft = runtime.draft_mut()?;
        draft
            .edit_object("Rename object", |editor| {
                editor.set_display_name(display_name)
            })
            .map_err(|error| error.to_string())?;
    }
    let blueprint = {
        let draft = runtime
            .draft
            .as_ref()
            .ok_or_else(|| runtime.load_error_message())?;
        validate_object_id_category(&id, draft.editor().object().category)?;
        draft
            .editor()
            .blueprint_for_save(draft.styles())
            .map_err(|error| error.to_string())?
    };
    runtime
        .project_mut()?
        .save_object_as(blueprint, id.clone())
        .map_err(|error| error.to_string())?;
    runtime.draft_mut()?.mark_object_saved_as(id.clone());
    runtime.document = OpenDocument::Saved(id.clone());
    Ok(Some(format!("Saved as {}", id.as_str())))
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
    let mut editor = EditorModel::calibration_scene().map_err(|error| error.to_string())?;
    let preview = if let Some(style) = runtime
        .draft
        .as_ref()
        .and_then(|draft| draft.styles().styles().keys().next())
    {
        editor.set_active_style(Some(style.clone()));
        PreviewSubject::ActiveStyle
    } else {
        editor.set_mode(WorkshopMode::VoxelStyles);
        runtime
            .draft
            .as_ref()
            .and_then(|draft| draft.palette().swatches().keys().next())
            .cloned()
            .map_or(PreviewSubject::ActiveStyle, PreviewSubject::Swatch)
    };
    runtime.draft_mut()?.open_object(editor);
    runtime.document = OpenDocument::Calibration;
    runtime.preview = preview;
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
    mut stroke: ResMut<PointerStroke>,
    mut runtime: ResMut<WorkshopRuntime>,
) {
    if stroke.active && buttons.just_released(MouseButton::Left) {
        stroke.active = false;
        stroke.last_cell = None;
        let result = runtime.draft_mut().and_then(|draft| {
            draft
                .commit_object_transaction()
                .map_err(|error| error.to_string())
        });
        match result {
            Ok(_) => runtime.needs_sync = true,
            Err(error) => runtime.set_status(WorkshopStatusKind::Error, error),
        }
    }
    if !input_enabled.0 {
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
                        stroke.active = true;
                        stroke.last_cell = None;
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
    if stroke.last_cell == Some(cell) {
        return;
    }
    stroke.last_cell = Some(cell);
    let result = apply_stroke_cell(runtime.draft_mut(), tool, cell);
    match result {
        Ok(_) => runtime.needs_sync = true,
        Err(error) => runtime.set_status(WorkshopStatusKind::Error, error),
    }
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
            editor
                .repaint(cell, style)
                .map_err(|error| error.to_string())
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

    ui_snapshot.update_from(
        project,
        draft.palette(),
        draft.styles(),
        draft.editor(),
        draft.undo_label(),
        draft.redo_label(),
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

fn build_viewport_content(
    draft: &WorkshopDraft,
    preview: &PreviewSubject,
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

    match draft.editor().mode() {
        WorkshopMode::Objects => {
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
        }
        WorkshopMode::VoxelStyles => match preview {
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
        },
    }
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
    use super::*;

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
    fn object_ids_match_their_singular_category_directory() {
        let plant = ObjectAssetId::new("plant/oak").expect("fixture id should be valid");
        assert!(validate_object_id_category(&plant, ObjectCategory::Plant).is_ok());
        assert!(validate_object_id_category(&plant, ObjectCategory::Effect).is_err());
        let nested =
            ObjectAssetId::new("plant/trees/oak").expect("path-like fixture id should be valid");
        assert!(validate_object_id_category(&nested, ObjectCategory::Plant).is_err());
    }
}
