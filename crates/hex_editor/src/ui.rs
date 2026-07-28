//! Immediate-mode user interface for the Asset Workshop.
//!
//! This module deliberately owns no project persistence. It renders a cloned,
//! read-only [`WorkshopUiSnapshot`] and emits [`WorkshopUiAction`] messages for the
//! application session to validate and execute.

use std::collections::BTreeSet;

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};
use hex_assets::{
    ArtPalette, ConnectivityPolicy, EffectPart, ObjectAssetId, ObjectBlueprint, ObjectBounds,
    ObjectCategory, ObjectPart, PaletteSwatch, PlantPart, PropPart, SrgbColor, SwatchId,
    VoxelEmission, VoxelStyle, VoxelStyleCatalog, VoxelStyleId, VoxelSurfaceMode,
    DEFAULT_NEAR_COLOR_THRESHOLD,
};

use crate::model::{EditorModel, EditorTool, PreviewRig, WorkshopMode};
use crate::project::AssetProject;
use crate::viewport::ViewportInputEnabled;

const LEFT_PANEL_WIDTH: f32 = 258.0;
const RIGHT_PANEL_WIDTH: f32 = 310.0;
const STATUS_HEIGHT: f32 = 28.0;
const TOOLBAR_HEIGHT: f32 = 42.0;
const SEARCH_HEIGHT: f32 = 28.0;

const PANEL_FILL: egui::Color32 = egui::Color32::from_rgb(31, 34, 38);
const TOOLBAR_FILL: egui::Color32 = egui::Color32::from_rgb(25, 28, 31);
const FIELD_FILL: egui::Color32 = egui::Color32::from_rgb(42, 46, 50);
const BORDER: egui::Color32 = egui::Color32::from_rgb(66, 72, 77);
const MUTED: egui::Color32 = egui::Color32::from_rgb(162, 169, 174);
const ACCENT: egui::Color32 = egui::Color32::from_rgb(96, 183, 143);
const WARNING: egui::Color32 = egui::Color32::from_rgb(235, 180, 77);
const ERROR: egui::Color32 = egui::Color32::from_rgb(234, 105, 105);
const SUCCESS: egui::Color32 = egui::Color32::from_rgb(116, 201, 147);

/// Camera poses available from the object-authoring toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorCameraSnap {
    /// Three-quarter authoring view.
    Perspective,
    /// Straight down along the level axis.
    Top,
    /// Horizontal view aligned with the positive q axis.
    Front,
    /// Horizontal view aligned with the positive r axis.
    Side,
}

/// A compact status shown along the bottom edge of the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkshopStatus {
    /// Visual severity.
    pub kind: WorkshopStatusKind,
    /// Human-readable outcome or actionable failure.
    pub message: String,
}

impl WorkshopStatus {
    /// Creates an informational status.
    #[must_use]
    pub fn info(message: impl Into<String>) -> Self {
        Self {
            kind: WorkshopStatusKind::Info,
            message: message.into(),
        }
    }
}

/// Status-bar severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkshopStatusKind {
    /// Neutral context.
    Info,
    /// A completed operation.
    Success,
    /// A recoverable concern.
    Warning,
    /// An operation that failed.
    Error,
}

/// Read-only document values needed to render object controls.
#[derive(Debug, Clone)]
pub struct ObjectEditorSnapshot {
    /// Current draft blueprint.
    pub object: ObjectBlueprint,
    /// Active editing tool.
    pub tool: EditorTool,
    /// Active style used by paint operations.
    pub active_style: Option<VoxelStyleId>,
    /// Active category-safe semantic role.
    pub active_part: ObjectPart,
    /// Active integer slice.
    pub active_level: i32,
    /// Number of selected occupied cells.
    pub selection_count: usize,
    /// Number of copied occupied cells.
    pub clipboard_count: usize,
    /// Whether semantics differ from the last saved checkpoint.
    pub dirty: bool,
}

impl ObjectEditorSnapshot {
    /// Captures the public state of an editor model.
    #[must_use]
    pub fn from_model(model: &EditorModel) -> Self {
        Self {
            object: model.object().clone(),
            tool: model.tool(),
            active_style: model.active_style().cloned(),
            active_part: model.active_part(),
            active_level: model.active_level(),
            selection_count: model.selection().len(),
            clipboard_count: model.clipboard().len(),
            dirty: model.is_dirty(),
        }
    }
}

/// Session-facing read model consumed by the egui system.
///
/// The application should refresh this resource after handling UI actions. Keeping
/// project data here as clones avoids borrowing persistence state from an immediate
/// mode callback and makes the UI incapable of writing files.
#[derive(Resource, Debug, Clone, Default)]
pub struct WorkshopUiSnapshot {
    /// Loaded palette, or `None` while project loading failed.
    pub palette: Option<ArtPalette>,
    /// Loaded style catalog.
    pub styles: Option<VoxelStyleCatalog>,
    /// Saved object blueprints in deterministic id order.
    pub objects: Vec<ObjectBlueprint>,
    /// Current mode.
    pub mode: Option<WorkshopMode>,
    /// Current preview lighting rig.
    pub preview_rig: Option<PreviewRig>,
    /// Whether the palette draft differs from its saved checkpoint.
    pub palette_dirty: bool,
    /// Whether the voxel-style draft differs from its saved checkpoint.
    pub styles_dirty: bool,
    /// Label of the next globally undoable edit.
    pub undo_label: Option<String>,
    /// Label of the next globally redoable edit.
    pub redo_label: Option<String>,
    /// Open object document.
    pub editor: Option<ObjectEditorSnapshot>,
    /// Most recent operation status.
    pub status: Option<WorkshopStatus>,
}

impl WorkshopUiSnapshot {
    /// Creates a project-less snapshot that keeps the editor open with an
    /// actionable startup or load failure.
    #[must_use]
    pub fn failed(message: impl Into<String>) -> Self {
        let mut snapshot = Self::default();
        snapshot.set_load_failure(message);
        snapshot
    }

    /// Refreshes the UI snapshot from a valid project and open editor model.
    pub fn update_from(
        &mut self,
        project: &AssetProject,
        palette_draft: &ArtPalette,
        style_draft: &VoxelStyleCatalog,
        editor: &EditorModel,
        undo_label: Option<&str>,
        redo_label: Option<&str>,
        status: Option<WorkshopStatus>,
    ) {
        self.palette = Some(palette_draft.clone());
        self.styles = Some(style_draft.clone());
        self.objects = project.objects().values().cloned().collect();
        self.mode = Some(editor.mode());
        self.preview_rig = Some(editor.preview_rig());
        self.palette_dirty = palette_draft != project.palette();
        self.styles_dirty = style_draft != project.styles();
        self.undo_label = undo_label.map(str::to_owned);
        self.redo_label = redo_label.map(str::to_owned);
        self.editor = Some(ObjectEditorSnapshot::from_model(editor));
        self.status = status;
    }

    /// Clears project-dependent data while retaining an actionable load failure.
    pub fn set_load_failure(&mut self, message: impl Into<String>) {
        self.palette = None;
        self.styles = None;
        self.palette_dirty = false;
        self.styles_dirty = false;
        self.undo_label = None;
        self.redo_label = None;
        self.objects.clear();
        self.editor = None;
        self.status = Some(WorkshopStatus {
            kind: WorkshopStatusKind::Error,
            message: message.into(),
        });
    }
}

/// Commands emitted by the editor UI and executed by the application session.
#[derive(Message, Debug, Clone)]
pub enum WorkshopUiAction {
    /// Change the shared editor mode.
    SetMode(WorkshopMode),
    /// Undo one object operation.
    Undo,
    /// Redo one object operation.
    Redo,
    /// Save the current object under its existing id.
    SaveObject,
    /// Validate and explicitly save the palette and voxel-style drafts.
    SaveCatalogs,
    /// Save the current draft as a new object.
    SaveObjectAs {
        /// Immutable destination id.
        id: ObjectAssetId,
        /// Editable display name.
        display_name: String,
    },
    /// Duplicate a saved object under a new identity.
    DuplicateObject {
        /// Source asset.
        source: ObjectAssetId,
        /// New immutable id.
        id: ObjectAssetId,
        /// New display name.
        display_name: String,
    },
    /// Create a blank object document.
    NewObject {
        /// Initial immutable id.
        id: ObjectAssetId,
        /// Initial display name.
        display_name: String,
        /// Semantic category.
        category: ObjectCategory,
        /// Connectivity required by props. Plant/effect policy is derived.
        prop_connectivity: ConnectivityPolicy,
    },
    /// Open a saved object.
    OpenObject(ObjectAssetId),
    /// Preview one palette swatch on the floating style voxel.
    PreviewSwatch(SwatchId),
    /// Preview one reusable style on the floating style voxel.
    PreviewStyle(VoxelStyleId),
    /// Delete a saved object after explicit UI confirmation.
    DeleteObject(ObjectAssetId),
    /// Change the deterministic preview rig.
    SetPreviewRig(PreviewRig),
    /// Snap the viewport camera.
    SnapCamera(EditorCameraSnap),
    /// Frame all currently visible content.
    FrameCamera,
    /// Change the active object tool.
    SetTool(EditorTool),
    /// Choose the active style.
    SetActiveStyle(VoxelStyleId),
    /// Choose the active semantic role.
    SetActivePart(ObjectPart),
    /// Change the active level.
    SetActiveLevel(i32),
    /// Change the editable display name of the open draft.
    SetObjectDisplayName(String),
    /// Change the open prop's connectivity policy.
    SetObjectConnectivity(ConnectivityPolicy),
    /// Change the object authoring canvas.
    SetObjectBounds(ObjectBounds),
    /// Move the explicit object origin to the sole selected cell.
    SetOriginFromSelection,
    /// Move the exact selection.
    NudgeSelection {
        /// Axial q delta.
        q: i32,
        /// Axial r delta.
        r: i32,
        /// Vertical level delta.
        level: i32,
    },
    /// Rotate selected cells clockwise by 60 degrees around the object origin.
    RotateSelectionClockwise,
    /// Copy selected cells.
    CopySelection,
    /// Paste copied cells at the viewport cursor or active-level origin.
    PasteSelection,
    /// Delete selected cells.
    DeleteSelection,
    /// Clear selected cells.
    ClearSelection,
    /// Apply the active semantic role to all selected voxels.
    RepaintSelectionPart(ObjectPart),
    /// Add or remove selected plant foliage from the canopy mask.
    SetSelectionCanopy(bool),
    /// Add or remove selected prop columns from the blocker footprint.
    SetSelectionBlocker(bool),
    /// Toggle blocker-overlay presentation.
    ShowBlockerOverlay(bool),
    /// Toggle canopy-overlay presentation.
    ShowCanopyOverlay(bool),
    /// Toggle semantic-role overlay presentation.
    ShowSemanticOverlay(bool),
    /// Isolate the active level in the object viewport.
    IsolateActiveLevel(bool),
    /// Show or hide the empty-cell hex guide.
    ShowGrid(bool),
    /// Create or update a palette swatch.
    UpsertSwatch {
        /// Existing immutable id when editing, or the requested new id.
        id: SwatchId,
        /// Validated candidate value.
        swatch: PaletteSwatch,
        /// Whether the author explicitly accepted the near-colour warning.
        confirmed_near_color: bool,
    },
    /// Delete a swatch after explicit UI confirmation.
    DeleteSwatch(SwatchId),
    /// Create or update a reusable style.
    UpsertStyle {
        /// Existing immutable id when editing, or the requested new id.
        id: VoxelStyleId,
        /// Validated style candidate.
        style: VoxelStyle,
    },
    /// Delete a style after explicit UI confirmation.
    DeleteStyle(VoxelStyleId),
}

/// Egui ownership and central viewport bounds for input and camera systems.
#[derive(Resource, Debug, Clone, Copy)]
pub struct ViewportInputSuppression {
    /// Central viewport in egui logical points.
    pub viewport_rect: egui::Rect,
    /// True when pointer-driven viewport tools must not run.
    pub pointer: bool,
    /// True when keyboard-driven viewport tools must not run.
    pub keyboard: bool,
}

impl Default for ViewportInputSuppression {
    fn default() -> Self {
        Self {
            viewport_rect: egui::Rect::NOTHING,
            pointer: true,
            keyboard: true,
        }
    }
}

/// Run condition for pointer-driven viewport systems.
#[must_use]
pub fn viewport_accepts_pointer(input: Res<ViewportInputSuppression>) -> bool {
    !input.pointer
}

/// Run condition for keyboard-driven viewport systems.
#[must_use]
pub fn viewport_accepts_keyboard(input: Res<ViewportInputSuppression>) -> bool {
    !input.keyboard
}

/// Persistent UI-local forms, searches, selections, and overlay preferences.
#[derive(Resource, Debug, Clone)]
pub struct WorkshopUiState {
    palette_search: String,
    style_search: String,
    object_search: String,
    selected_swatch: Option<SwatchId>,
    selected_style: Option<VoxelStyleId>,
    selected_object: Option<ObjectAssetId>,
    swatch_form: SwatchForm,
    style_form: StyleForm,
    object_dialog: Option<ObjectDialog>,
    pending_delete: Option<DeleteTarget>,
    show_blockers: bool,
    show_canopy: bool,
    show_semantics: bool,
    isolate_active_level: bool,
    show_grid: bool,
    style_subject: StyleInspectorSubject,
    object_form_id: Option<ObjectAssetId>,
    object_name: String,
    object_bounds: ObjectBounds,
    object_connectivity: ConnectivityPolicy,
    initialized_theme: bool,
}

impl Default for WorkshopUiState {
    fn default() -> Self {
        Self {
            palette_search: String::new(),
            style_search: String::new(),
            object_search: String::new(),
            selected_swatch: None,
            selected_style: None,
            selected_object: None,
            swatch_form: SwatchForm::new(),
            style_form: StyleForm::new(),
            object_dialog: None,
            pending_delete: None,
            show_blockers: false,
            show_canopy: false,
            show_semantics: false,
            isolate_active_level: false,
            show_grid: true,
            style_subject: StyleInspectorSubject::Swatch,
            object_form_id: None,
            object_name: String::new(),
            object_bounds: ObjectBounds::DEFAULT,
            object_connectivity: ConnectivityPolicy::Grounded,
            initialized_theme: false,
        }
    }
}

#[derive(Debug, Clone)]
struct SwatchForm {
    is_new: bool,
    id: String,
    display_name: String,
    tags: String,
    rgb: [f32; 3],
    hsv: [f32; 3],
    hex: String,
    hex_error: Option<String>,
    confirmed_near_color: bool,
}

impl SwatchForm {
    fn new() -> Self {
        let rgb = [0.5, 0.5, 0.5];
        Self {
            is_new: true,
            id: String::new(),
            display_name: String::new(),
            tags: String::new(),
            rgb,
            hsv: rgb_to_hsv(rgb),
            hex: rgb_to_hex(rgb),
            hex_error: None,
            confirmed_near_color: false,
        }
    }

    fn from_existing(id: &SwatchId, swatch: &PaletteSwatch) -> Self {
        let rgb = swatch.color().to_array();
        Self {
            is_new: false,
            id: id.as_str().to_owned(),
            display_name: swatch.display_name().to_owned(),
            tags: swatch.tags().iter().cloned().collect::<Vec<_>>().join(", "),
            rgb,
            hsv: rgb_to_hsv(rgb),
            hex: rgb_to_hex(rgb),
            hex_error: None,
            confirmed_near_color: false,
        }
    }

    fn set_rgb(&mut self, rgb: [f32; 3]) {
        self.rgb = rgb.map(|channel| channel.clamp(0.0, 1.0));
        self.hsv = rgb_to_hsv(self.rgb);
        self.hex = rgb_to_hex(self.rgb);
        self.hex_error = None;
        self.confirmed_near_color = false;
    }

    fn set_hsv(&mut self, hsv: [f32; 3]) {
        let [hue, saturation, value] = hsv;
        self.hsv = [
            hue.rem_euclid(360.0),
            saturation.clamp(0.0, 100.0),
            value.clamp(0.0, 100.0),
        ];
        self.rgb = hsv_to_rgb(self.hsv);
        self.hex = rgb_to_hex(self.rgb);
        self.hex_error = None;
        self.confirmed_near_color = false;
    }

    fn apply_hex(&mut self) {
        match parse_hex_color(&self.hex) {
            Ok(rgb) => self.set_rgb(rgb),
            Err(error) => self.hex_error = Some(error),
        }
    }

    fn candidate(&self) -> Result<(SwatchId, PaletteSwatch), String> {
        let id = SwatchId::new(self.id.trim()).map_err(|error| error.to_string())?;
        let [red, green, blue] = self.rgb;
        let color = SrgbColor::new(red, green, blue).map_err(|error| error.to_string())?;
        let tags = parse_tags(&self.tags)?;
        let swatch = PaletteSwatch::new(self.display_name.trim(), color, tags)
            .map_err(|error| error.to_string())?;
        Ok((id, swatch))
    }
}

#[derive(Debug, Clone)]
struct StyleForm {
    is_new: bool,
    id: String,
    display_name: String,
    base_swatch: Option<SwatchId>,
    surface_mode: VoxelSurfaceMode,
    opacity: f32,
    emission_enabled: bool,
    emission_swatch: Option<SwatchId>,
    emission_strength: f32,
}

impl StyleForm {
    fn new() -> Self {
        Self {
            is_new: true,
            id: String::new(),
            display_name: String::new(),
            base_swatch: None,
            surface_mode: VoxelSurfaceMode::Opaque,
            opacity: 1.0,
            emission_enabled: false,
            emission_swatch: None,
            emission_strength: 1.0,
        }
    }

    fn from_existing(id: &VoxelStyleId, style: &VoxelStyle) -> Self {
        Self {
            is_new: false,
            id: id.as_str().to_owned(),
            display_name: style.display_name().to_owned(),
            base_swatch: Some(style.base_swatch().clone()),
            surface_mode: style.surface_mode(),
            opacity: style.opacity(),
            emission_enabled: style.emission().is_some(),
            emission_swatch: style.emission().map(|emission| emission.swatch().clone()),
            emission_strength: style.emission().map_or(1.0, VoxelEmission::strength),
        }
    }

    fn candidate(&self) -> Result<(VoxelStyleId, VoxelStyle), String> {
        let id = VoxelStyleId::new(self.id.trim()).map_err(|error| error.to_string())?;
        let base_swatch = self
            .base_swatch
            .clone()
            .ok_or_else(|| "choose a base swatch".to_owned())?;
        let emission = if self.emission_enabled {
            let swatch = self
                .emission_swatch
                .clone()
                .ok_or_else(|| "choose an emission swatch".to_owned())?;
            Some(
                VoxelEmission::new(swatch, self.emission_strength)
                    .map_err(|error| error.to_string())?,
            )
        } else {
            None
        };
        let style = VoxelStyle::new(
            self.display_name.trim(),
            base_swatch,
            self.surface_mode,
            self.opacity,
            emission,
        )
        .map_err(|error| error.to_string())?;
        Ok((id, style))
    }
}

#[derive(Debug, Clone)]
struct ObjectDialog {
    kind: ObjectDialogKind,
    id: String,
    display_name: String,
    category: ObjectCategory,
    prop_connectivity: ConnectivityPolicy,
    error: Option<String>,
}

#[derive(Debug, Clone)]
enum ObjectDialogKind {
    New,
    SaveAs,
    Duplicate { source: ObjectAssetId },
}

#[derive(Debug, Clone)]
enum DeleteTarget {
    Swatch(SwatchId),
    Style(VoxelStyleId),
    Object(ObjectAssetId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StyleInspectorSubject {
    Swatch,
    Style,
}

/// Installs UI state, command messages, and the primary egui pass.
pub fn plugin(app: &mut App) {
    app.init_resource::<WorkshopUiState>()
        .init_resource::<WorkshopUiSnapshot>()
        .init_resource::<ViewportInputSuppression>()
        .add_message::<WorkshopUiAction>()
        .add_systems(EguiPrimaryContextPass, draw_workshop_ui);
}

fn draw_workshop_ui(
    mut contexts: EguiContexts,
    snapshot: Res<WorkshopUiSnapshot>,
    mut state: ResMut<WorkshopUiState>,
    mut suppression: ResMut<ViewportInputSuppression>,
    viewport_input: Option<ResMut<ViewportInputEnabled>>,
    mut messages: MessageWriter<WorkshopUiAction>,
) -> Result {
    let context = contexts.ctx_mut()?;
    if !state.initialized_theme {
        install_theme(context);
        state.initialized_theme = true;
    }

    let mut actions = Vec::new();
    let mut root_ui = egui::Ui::new(
        context.clone(),
        egui::Id::new("workshop_root"),
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(context.viewport_rect()),
    );
    draw_top_toolbar(&mut root_ui, &snapshot, &mut state, &mut actions);
    draw_status_bar(&mut root_ui, &snapshot, &mut actions);
    draw_left_browser(&mut root_ui, &snapshot, &mut state, &mut actions);
    draw_right_inspector(&mut root_ui, &snapshot, &mut state, &mut actions);

    let central = egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(egui::Color32::TRANSPARENT))
        .show_inside(&mut root_ui, |ui| {
            ui.allocate_rect(ui.available_rect_before_wrap(), egui::Sense::hover())
        });
    draw_object_dialog(context, &snapshot, &mut state, &mut actions);
    draw_delete_confirmation(context, &mut state, &mut actions);

    suppression.viewport_rect = central.inner.rect;
    let pointer_position = context.input(|input| input.pointer.hover_pos());
    let overlay_open =
        state.object_dialog.is_some() || state.pending_delete.is_some() || context.any_popup_open();
    suppression.pointer = viewport_pointer_is_suppressed(
        suppression.viewport_rect,
        pointer_position,
        context.egui_is_using_pointer(),
        overlay_open,
    );
    suppression.keyboard = context.egui_wants_keyboard_input();
    if let Some(mut viewport_input) = viewport_input {
        viewport_input.0 = !suppression.pointer;
    }

    for action in actions {
        messages.write(action);
    }
    Ok(())
}

fn viewport_pointer_is_suppressed(
    viewport_rect: egui::Rect,
    pointer_position: Option<egui::Pos2>,
    egui_is_using_pointer: bool,
    overlay_open: bool,
) -> bool {
    overlay_open
        || egui_is_using_pointer
        || pointer_position.is_none_or(|position| !viewport_rect.contains(position))
}

fn install_theme(context: &egui::Context) {
    let mut style = (*context.global_style()).clone();
    style.spacing.item_spacing = egui::vec2(7.0, 6.0);
    style.spacing.button_padding = egui::vec2(9.0, 5.0);
    style.spacing.interact_size.y = 28.0;
    style.visuals.panel_fill = PANEL_FILL;
    style.visuals.window_fill = PANEL_FILL;
    style.visuals.extreme_bg_color = FIELD_FILL;
    style.visuals.faint_bg_color = egui::Color32::from_rgb(37, 40, 44);
    style.visuals.widgets.noninteractive.bg_fill = PANEL_FILL;
    style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, BORDER);
    style.visuals.widgets.inactive.bg_fill = FIELD_FILL;
    style.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, BORDER);
    style.visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(53, 59, 62);
    style.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, ACCENT);
    style.visuals.widgets.active.bg_fill = egui::Color32::from_rgb(60, 83, 72);
    style.visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, ACCENT);
    style.visuals.selection.bg_fill = egui::Color32::from_rgb(45, 100, 75);
    style.visuals.selection.stroke = egui::Stroke::new(1.0_f32, ACCENT);
    style.visuals.window_corner_radius = egui::CornerRadius::same(5);
    style.visuals.menu_corner_radius = egui::CornerRadius::same(4);
    context.set_global_style(style);
}

fn draw_top_toolbar(
    root_ui: &mut egui::Ui,
    snapshot: &WorkshopUiSnapshot,
    state: &mut WorkshopUiState,
    actions: &mut Vec<WorkshopUiAction>,
) {
    egui::Panel::top("workshop_toolbar")
        .exact_size(TOOLBAR_HEIGHT)
        .frame(
            egui::Frame::NONE
                .fill(TOOLBAR_FILL)
                .inner_margin(egui::Margin::symmetric(10, 6))
                .stroke(egui::Stroke::new(1.0_f32, BORDER)),
        )
        .show_inside(root_ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("ASSET WORKSHOP")
                        .strong()
                        .color(egui::Color32::WHITE),
                );
                ui.separator();

                let mode = snapshot.mode.unwrap_or(WorkshopMode::VoxelStyles);
                if ui
                    .selectable_label(mode == WorkshopMode::VoxelStyles, "Voxel Styles")
                    .on_hover_text("Author shared palette colours and reusable voxel materials")
                    .clicked()
                    && mode != WorkshopMode::VoxelStyles
                {
                    actions.push(WorkshopUiAction::SetMode(WorkshopMode::VoxelStyles));
                }
                if ui
                    .selectable_label(mode == WorkshopMode::Objects, "Objects")
                    .on_hover_text("Assemble palette-backed voxels into authored objects")
                    .clicked()
                    && mode != WorkshopMode::Objects
                {
                    actions.push(WorkshopUiAction::SetMode(WorkshopMode::Objects));
                }

                ui.separator();
                let document_ready = snapshot.editor.is_some();
                let project_ready = snapshot.palette.is_some() && snapshot.styles.is_some();
                if ui
                    .add_enabled(snapshot.undo_label.is_some(), egui::Button::new("Undo"))
                    .on_hover_text(
                        snapshot
                            .undo_label
                            .as_deref()
                            .map_or("Nothing to undo".to_owned(), |label| {
                                format!("Undo {label}")
                            }),
                    )
                    .clicked()
                {
                    actions.push(WorkshopUiAction::Undo);
                }
                if ui
                    .add_enabled(snapshot.redo_label.is_some(), egui::Button::new("Redo"))
                    .on_hover_text(
                        snapshot
                            .redo_label
                            .as_deref()
                            .map_or("Nothing to redo".to_owned(), |label| {
                                format!("Redo {label}")
                            }),
                    )
                    .clicked()
                {
                    actions.push(WorkshopUiAction::Redo);
                }

                let save_ready = match mode {
                    WorkshopMode::VoxelStyles => project_ready,
                    WorkshopMode::Objects => document_ready,
                };
                if ui
                    .add_enabled(save_ready, egui::Button::new("Save"))
                    .on_hover_text("Validate and explicitly save the active authoring data")
                    .clicked()
                {
                    actions.push(match mode {
                        WorkshopMode::VoxelStyles => WorkshopUiAction::SaveCatalogs,
                        WorkshopMode::Objects => WorkshopUiAction::SaveObject,
                    });
                }
                if mode == WorkshopMode::Objects {
                    if ui
                        .add_enabled(document_ready, egui::Button::new("Save As"))
                        .on_hover_text("Save the current draft under a new immutable id")
                        .clicked()
                    {
                        if let Some(editor) = &snapshot.editor {
                            state.object_dialog = Some(ObjectDialog {
                                kind: ObjectDialogKind::SaveAs,
                                id: String::new(),
                                display_name: editor.object.display_name.clone(),
                                category: editor.object.category,
                                prop_connectivity: editor.object.connectivity,
                                error: None,
                            });
                        }
                    }
                    if ui
                        .add_enabled(document_ready, egui::Button::new("Duplicate"))
                        .on_hover_text("Duplicate the saved object into a new asset")
                        .clicked()
                    {
                        if let Some(editor) = &snapshot.editor {
                            state.object_dialog = Some(ObjectDialog {
                                kind: ObjectDialogKind::Duplicate {
                                    source: editor.object.id.clone(),
                                },
                                id: String::new(),
                                display_name: format!("{} Copy", editor.object.display_name),
                                category: editor.object.category,
                                prop_connectivity: editor.object.connectivity,
                                error: None,
                            });
                        }
                    }
                }

                ui.separator();
                let active_rig = snapshot.preview_rig.unwrap_or(PreviewRig::Neutral);
                egui::ComboBox::from_id_salt("preview_rig")
                    .selected_text(rig_label(active_rig))
                    .width(78.0)
                    .show_ui(ui, |ui| {
                        for rig in [PreviewRig::Neutral, PreviewRig::Dark, PreviewRig::Unlit] {
                            if ui
                                .selectable_label(active_rig == rig, rig_label(rig))
                                .clicked()
                                && active_rig != rig
                            {
                                actions.push(WorkshopUiAction::SetPreviewRig(rig));
                            }
                        }
                    })
                    .response
                    .on_hover_text("Choose deterministic preview lighting");

                ui.separator();
                for (label, snap, tooltip) in [
                    (
                        "3D",
                        EditorCameraSnap::Perspective,
                        "Snap to perspective view",
                    ),
                    ("Top", EditorCameraSnap::Top, "Snap to top view"),
                    ("Front", EditorCameraSnap::Front, "Snap to front view"),
                    ("Side", EditorCameraSnap::Side, "Snap to side view"),
                ] {
                    if ui.button(label).on_hover_text(tooltip).clicked() {
                        actions.push(WorkshopUiAction::SnapCamera(snap));
                    }
                }
                if ui
                    .button("Frame")
                    .on_hover_text("Frame all visible authored voxels")
                    .clicked()
                {
                    actions.push(WorkshopUiAction::FrameCamera);
                }

                let dirty = match mode {
                    WorkshopMode::VoxelStyles => snapshot.palette_dirty || snapshot.styles_dirty,
                    WorkshopMode::Objects => {
                        snapshot.editor.as_ref().is_some_and(|editor| editor.dirty)
                    }
                };
                if dirty {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new("Unsaved").color(WARNING));
                    });
                }
            });
        });
}

fn draw_status_bar(
    root_ui: &mut egui::Ui,
    snapshot: &WorkshopUiSnapshot,
    actions: &mut Vec<WorkshopUiAction>,
) {
    egui::Panel::bottom("workshop_status")
        .exact_size(STATUS_HEIGHT)
        .frame(
            egui::Frame::NONE
                .fill(TOOLBAR_FILL)
                .inner_margin(egui::Margin::symmetric(10, 3))
                .stroke(egui::Stroke::new(1.0_f32, BORDER)),
        )
        .show_inside(root_ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(status) = &snapshot.status {
                    ui.colored_label(status_color(status.kind), &status.message);
                } else {
                    ui.label(egui::RichText::new("Ready").color(MUTED));
                }

                if snapshot.mode == Some(WorkshopMode::Objects) {
                    let Some(editor) = &snapshot.editor else {
                        return;
                    };
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} voxels  |  {} selected",
                                editor.object.placements.len(),
                                editor.selection_count
                            ))
                            .color(MUTED),
                        );
                        let upper = editor
                            .object
                            .bounds
                            .min_level
                            .saturating_add(i32::from(editor.object.bounds.height))
                            .saturating_sub(1);
                        let mut level = editor.active_level;
                        if ui
                            .add(
                                egui::DragValue::new(&mut level)
                                    .range(editor.object.bounds.min_level..=upper)
                                    .prefix("Level "),
                            )
                            .on_hover_text("Active placement and slice level")
                            .changed()
                        {
                            actions.push(WorkshopUiAction::SetActiveLevel(level));
                        }
                    });
                } else if let (Some(palette), Some(styles)) = (&snapshot.palette, &snapshot.styles)
                {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} swatches  |  {} styles",
                                palette.swatches().len(),
                                styles.styles().len()
                            ))
                            .color(MUTED),
                        );
                    });
                }
            });
        });
}

fn draw_left_browser(
    root_ui: &mut egui::Ui,
    snapshot: &WorkshopUiSnapshot,
    state: &mut WorkshopUiState,
    actions: &mut Vec<WorkshopUiAction>,
) {
    egui::Panel::left("workshop_browser")
        .default_size(LEFT_PANEL_WIDTH)
        .min_size(220.0)
        .max_size(360.0)
        .resizable(true)
        .frame(
            egui::Frame::NONE
                .fill(PANEL_FILL)
                .inner_margin(egui::Margin::symmetric(10, 10))
                .stroke(egui::Stroke::new(1.0_f32, BORDER)),
        )
        .show_inside(root_ui, |ui| {
            let mode = snapshot.mode.unwrap_or(WorkshopMode::VoxelStyles);
            match mode {
                WorkshopMode::VoxelStyles => {
                    draw_palette_browser(ui, snapshot, state, actions);
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(5.0);
                    draw_style_browser(ui, snapshot, state, false, actions);
                }
                WorkshopMode::Objects => {
                    draw_object_browser(ui, snapshot, state, actions);
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(5.0);
                    draw_style_browser(ui, snapshot, state, true, actions);
                }
            }
        });
}

fn draw_palette_browser(
    ui: &mut egui::Ui,
    snapshot: &WorkshopUiSnapshot,
    state: &mut WorkshopUiState,
    actions: &mut Vec<WorkshopUiAction>,
) {
    ui.horizontal(|ui| {
        ui.heading("Palette");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button("New")
                .on_hover_text("Create a named palette swatch")
                .clicked()
            {
                state.selected_swatch = None;
                state.style_subject = StyleInspectorSubject::Swatch;
                state.swatch_form = SwatchForm::new();
            }
        });
    });
    search_field(ui, "palette_search", &mut state.palette_search);

    let Some(palette) = &snapshot.palette else {
        unavailable_label(ui);
        return;
    };
    egui::ScrollArea::vertical()
        .id_salt("palette_entries")
        .max_height(250.0)
        .show(ui, |ui| {
            for (id, swatch) in palette.swatches() {
                if !matches_search(
                    &state.palette_search,
                    [id.as_str(), swatch.display_name()]
                        .into_iter()
                        .chain(swatch.tags().iter().map(String::as_str)),
                ) {
                    continue;
                }
                let selected = state.selected_swatch.as_ref() == Some(id)
                    && state.style_subject == StyleInspectorSubject::Swatch;
                let response = ui
                    .horizontal(|ui| {
                        color_chip(ui, swatch.color(), egui::vec2(22.0, 22.0));
                        ui.selectable_label(selected, swatch.display_name())
                    })
                    .inner;
                if response
                    .on_hover_text(format!("{}\n{}", id, tags_label(swatch.tags())))
                    .clicked()
                {
                    state.selected_swatch = Some(id.clone());
                    state.style_subject = StyleInspectorSubject::Swatch;
                    state.swatch_form = SwatchForm::from_existing(id, swatch);
                    actions.push(WorkshopUiAction::PreviewSwatch(id.clone()));
                }
            }
        });
}

fn draw_style_browser(
    ui: &mut egui::Ui,
    snapshot: &WorkshopUiSnapshot,
    state: &mut WorkshopUiState,
    object_palette: bool,
    actions: &mut Vec<WorkshopUiAction>,
) {
    ui.horizontal(|ui| {
        ui.heading(if object_palette {
            "Voxel Styles"
        } else {
            "Styles"
        });
        if !object_palette {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("New")
                    .on_hover_text("Create a reusable voxel style")
                    .clicked()
                {
                    state.selected_style = None;
                    state.style_subject = StyleInspectorSubject::Style;
                    state.style_form = StyleForm::new();
                    if let Some(first) = snapshot
                        .palette
                        .as_ref()
                        .and_then(|palette| palette.swatches().keys().next())
                    {
                        state.style_form.base_swatch = Some(first.clone());
                    }
                }
            });
        }
    });
    search_field(ui, "style_search", &mut state.style_search);

    let (Some(styles), Some(palette)) = (&snapshot.styles, &snapshot.palette) else {
        unavailable_label(ui);
        return;
    };
    egui::ScrollArea::vertical()
        .id_salt(if object_palette {
            "object_style_entries"
        } else {
            "style_entries"
        })
        .max_height(if object_palette { 330.0 } else { 250.0 })
        .show(ui, |ui| {
            if styles.styles().is_empty() {
                ui.label(egui::RichText::new("No voxel styles yet").color(MUTED));
            }
            for (id, style) in styles.styles() {
                if !matches_search(
                    &state.style_search,
                    [
                        id.as_str(),
                        style.display_name(),
                        surface_mode_label(style.surface_mode()),
                    ],
                ) {
                    continue;
                }
                let active = snapshot
                    .editor
                    .as_ref()
                    .and_then(|editor| editor.active_style.as_ref())
                    == Some(id);
                let selected = if object_palette {
                    active
                } else {
                    state.selected_style.as_ref() == Some(id)
                        && state.style_subject == StyleInspectorSubject::Style
                };
                let response = ui
                    .horizontal(|ui| {
                        if let Some(swatch) = palette.get(style.base_swatch()) {
                            color_chip(ui, swatch.color(), egui::vec2(22.0, 22.0));
                        }
                        let mut name = style.display_name().to_owned();
                        if style.emission().is_some() {
                            name.push_str("  E");
                        }
                        ui.selectable_label(selected, name)
                    })
                    .inner;
                if response
                    .on_hover_text(format!(
                        "{}\n{} | opacity {:.2}",
                        id,
                        surface_mode_label(style.surface_mode()),
                        style.opacity()
                    ))
                    .clicked()
                {
                    if object_palette {
                        actions.push(WorkshopUiAction::SetActiveStyle(id.clone()));
                    } else {
                        state.selected_style = Some(id.clone());
                        state.style_subject = StyleInspectorSubject::Style;
                        state.style_form = StyleForm::from_existing(id, style);
                        actions.push(WorkshopUiAction::PreviewStyle(id.clone()));
                    }
                }
            }
        });
}

fn draw_object_browser(
    ui: &mut egui::Ui,
    snapshot: &WorkshopUiSnapshot,
    state: &mut WorkshopUiState,
    actions: &mut Vec<WorkshopUiAction>,
) {
    ui.horizontal(|ui| {
        ui.heading("Objects");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button("New")
                .on_hover_text("Create an unsaved object document")
                .clicked()
            {
                state.object_dialog = Some(ObjectDialog {
                    kind: ObjectDialogKind::New,
                    id: String::new(),
                    display_name: String::new(),
                    category: ObjectCategory::Plant,
                    prop_connectivity: ConnectivityPolicy::Grounded,
                    error: None,
                });
            }
        });
    });
    search_field(ui, "object_search", &mut state.object_search);
    egui::ScrollArea::vertical()
        .id_salt("object_entries")
        .max_height(330.0)
        .show(ui, |ui| {
            if snapshot.objects.is_empty() {
                ui.label(egui::RichText::new("No saved objects").color(MUTED));
            }
            for category in [
                ObjectCategory::Plant,
                ObjectCategory::Effect,
                ObjectCategory::Prop,
            ] {
                let matching: Vec<_> = snapshot
                    .objects
                    .iter()
                    .filter(|object| object.category == category)
                    .filter(|object| {
                        matches_search(
                            &state.object_search,
                            [object.id.as_str(), object.display_name.as_str()],
                        )
                    })
                    .collect();
                if matching.is_empty() {
                    continue;
                }
                ui.label(
                    egui::RichText::new(category_label(category).to_uppercase())
                        .small()
                        .color(MUTED),
                );
                for object in matching {
                    let selected = snapshot
                        .editor
                        .as_ref()
                        .is_some_and(|editor| editor.object.id == object.id);
                    if ui
                        .selectable_label(selected, &object.display_name)
                        .on_hover_text(format!("{}\n{} voxels", object.id, object.placements.len()))
                        .clicked()
                    {
                        state.selected_object = Some(object.id.clone());
                        actions.push(WorkshopUiAction::OpenObject(object.id.clone()));
                    }
                }
                ui.add_space(4.0);
            }
        });
}

fn draw_right_inspector(
    root_ui: &mut egui::Ui,
    snapshot: &WorkshopUiSnapshot,
    state: &mut WorkshopUiState,
    actions: &mut Vec<WorkshopUiAction>,
) {
    egui::Panel::right("workshop_inspector")
        .default_size(RIGHT_PANEL_WIDTH)
        .min_size(275.0)
        .max_size(420.0)
        .resizable(true)
        .frame(
            egui::Frame::NONE
                .fill(PANEL_FILL)
                .inner_margin(egui::Margin::symmetric(12, 10))
                .stroke(egui::Stroke::new(1.0_f32, BORDER)),
        )
        .show_inside(root_ui, |ui| {
            let mode = snapshot.mode.unwrap_or(WorkshopMode::VoxelStyles);
            egui::ScrollArea::vertical()
                .id_salt("workshop_inspector_scroll")
                .show(ui, |ui| match mode {
                    WorkshopMode::VoxelStyles => {
                        draw_style_mode_inspector(ui, snapshot, state, actions);
                    }
                    WorkshopMode::Objects => {
                        draw_object_inspector(ui, snapshot, state, actions);
                    }
                });
        });
}

fn draw_style_mode_inspector(
    ui: &mut egui::Ui,
    snapshot: &WorkshopUiSnapshot,
    state: &mut WorkshopUiState,
    actions: &mut Vec<WorkshopUiAction>,
) {
    ui.horizontal(|ui| {
        if ui
            .selectable_label(
                state.style_subject == StyleInspectorSubject::Swatch,
                "Swatch",
            )
            .clicked()
        {
            state.style_subject = StyleInspectorSubject::Swatch;
        }
        if ui
            .selectable_label(
                state.style_subject == StyleInspectorSubject::Style,
                "Voxel Style",
            )
            .clicked()
        {
            state.style_subject = StyleInspectorSubject::Style;
        }
    });
    ui.separator();

    match state.style_subject {
        StyleInspectorSubject::Swatch => {
            draw_swatch_inspector(ui, snapshot, state, actions);
        }
        StyleInspectorSubject::Style => {
            draw_voxel_style_inspector(ui, snapshot, state, actions);
        }
    }
}

fn draw_swatch_inspector(
    ui: &mut egui::Ui,
    snapshot: &WorkshopUiSnapshot,
    state: &mut WorkshopUiState,
    actions: &mut Vec<WorkshopUiAction>,
) {
    ui.heading(if state.swatch_form.is_new {
        "New Swatch"
    } else {
        "Edit Swatch"
    });
    ui.add_space(4.0);

    labeled_text(
        ui,
        "Stable key",
        &mut state.swatch_form.id,
        !state.swatch_form.is_new,
    );
    labeled_text(
        ui,
        "Display name",
        &mut state.swatch_form.display_name,
        false,
    );
    labeled_text(ui, "Tags", &mut state.swatch_form.tags, false);
    ui.add_space(8.0);

    ui.label(egui::RichText::new("Color").strong());
    ui.horizontal(|ui| {
        let mut bytes = state.swatch_form.rgb.map(float_to_byte);
        if ui
            .color_edit_button_srgb(&mut bytes)
            .on_hover_text("Open the sRGB colour picker")
            .changed()
        {
            state
                .swatch_form
                .set_rgb(bytes.map(|channel| f32::from(channel) / 255.0));
        }
        let [red, green, blue] = state.swatch_form.rgb;
        if let Ok(color) = SrgbColor::new(red, green, blue) {
            color_chip(ui, color, egui::vec2(96.0, 28.0));
        }
    });

    let mut rgb = state.swatch_form.rgb;
    let rgb_changed = ui
        .horizontal(|ui| {
            let [red, green, blue] = &mut rgb;
            let red = ui.add(
                egui::DragValue::new(red)
                    .range(0.0..=1.0)
                    .speed(0.005)
                    .prefix("R "),
            );
            let green = ui.add(
                egui::DragValue::new(green)
                    .range(0.0..=1.0)
                    .speed(0.005)
                    .prefix("G "),
            );
            let blue = ui.add(
                egui::DragValue::new(blue)
                    .range(0.0..=1.0)
                    .speed(0.005)
                    .prefix("B "),
            );
            red.changed() || green.changed() || blue.changed()
        })
        .inner;
    if rgb_changed {
        state.swatch_form.set_rgb(rgb);
    }

    let mut hsv = state.swatch_form.hsv;
    let hsv_changed = ui
        .horizontal(|ui| {
            let [hue, saturation, value] = &mut hsv;
            let hue = ui.add(
                egui::DragValue::new(hue)
                    .range(0.0..=360.0)
                    .speed(1.0)
                    .prefix("H "),
            );
            let saturation = ui.add(
                egui::DragValue::new(saturation)
                    .range(0.0..=100.0)
                    .speed(0.5)
                    .prefix("S "),
            );
            let value = ui.add(
                egui::DragValue::new(value)
                    .range(0.0..=100.0)
                    .speed(0.5)
                    .prefix("V "),
            );
            hue.changed() || saturation.changed() || value.changed()
        })
        .inner;
    if hsv_changed {
        state.swatch_form.set_hsv(hsv);
    }

    ui.horizontal(|ui| {
        ui.label("#");
        let response = ui.add(
            egui::TextEdit::singleline(&mut state.swatch_form.hex)
                .desired_width(92.0)
                .char_limit(6),
        );
        let submit = response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        if submit || ui.small_button("Apply").clicked() {
            state.swatch_form.apply_hex();
        }
    });
    if let Some(error) = &state.swatch_form.hex_error {
        ui.colored_label(ERROR, error);
    }

    let [red, green, blue] = state.swatch_form.rgb;
    let candidate_color = SrgbColor::new(red, green, blue).ok();
    let matches = match (&snapshot.palette, candidate_color) {
        (Some(palette), Some(color)) => nearest_distinct_swatches(
            palette,
            color,
            if state.swatch_form.is_new {
                None
            } else {
                state.selected_swatch.as_ref()
            },
            5,
        ),
        _ => Vec::new(),
    };
    ui.add_space(8.0);
    ui.label(egui::RichText::new("Nearest palette colours").strong());
    if matches.is_empty() {
        ui.label(egui::RichText::new("No comparison swatches").color(MUTED));
    } else if let Some(palette) = &snapshot.palette {
        for (id, distance) in &matches {
            if let Some(swatch) = palette.get(id) {
                ui.horizontal(|ui| {
                    color_chip(ui, swatch.color(), egui::vec2(16.0, 16.0));
                    ui.label(swatch.display_name());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("{distance:.4}"))
                                .monospace()
                                .color(if *distance <= DEFAULT_NEAR_COLOR_THRESHOLD {
                                    WARNING
                                } else {
                                    MUTED
                                }),
                        );
                    });
                });
            }
        }
    }

    let has_near_match = matches
        .first()
        .is_some_and(|(_, distance)| *distance <= DEFAULT_NEAR_COLOR_THRESHOLD);
    if has_near_match {
        ui.add_space(5.0);
        ui.colored_label(
            WARNING,
            format!(
                "A palette colour is within OKLab distance {:.3}.",
                DEFAULT_NEAR_COLOR_THRESHOLD
            ),
        );
        ui.checkbox(
            &mut state.swatch_form.confirmed_near_color,
            "Create or update this distinct colour anyway",
        );
    }

    ui.add_space(10.0);
    let candidate = state.swatch_form.candidate();
    if let Err(error) = &candidate {
        ui.colored_label(ERROR, error);
    }
    let confirmation_ready = !has_near_match || state.swatch_form.confirmed_near_color;
    ui.horizontal(|ui| {
        let save_label = if state.swatch_form.is_new {
            "Create Swatch"
        } else {
            "Update Swatch"
        };
        let save_clicked = ui
            .add_enabled(
                candidate.is_ok() && confirmation_ready && snapshot.palette.is_some(),
                egui::Button::new(save_label),
            )
            .clicked();
        if save_clicked {
            if let Ok((id, swatch)) = candidate {
                actions.push(WorkshopUiAction::UpsertSwatch {
                    id,
                    swatch,
                    confirmed_near_color: state.swatch_form.confirmed_near_color,
                });
            }
        }
        if !state.swatch_form.is_new {
            let delete_clicked = ui
                .button("Delete")
                .on_hover_text("Delete only when no style references this swatch")
                .clicked();
            if delete_clicked {
                if let Some(id) = &state.selected_swatch {
                    state.pending_delete = Some(DeleteTarget::Swatch(id.clone()));
                }
            }
        }
    });

    if let Some(id) = &state.selected_swatch {
        draw_swatch_impact(ui, snapshot, id);
    }
}

fn draw_voxel_style_inspector(
    ui: &mut egui::Ui,
    snapshot: &WorkshopUiSnapshot,
    state: &mut WorkshopUiState,
    actions: &mut Vec<WorkshopUiAction>,
) {
    ui.heading(if state.style_form.is_new {
        "New Voxel Style"
    } else {
        "Edit Voxel Style"
    });
    ui.add_space(4.0);
    labeled_text(
        ui,
        "Stable key",
        &mut state.style_form.id,
        !state.style_form.is_new,
    );
    labeled_text(
        ui,
        "Display name",
        &mut state.style_form.display_name,
        false,
    );

    let palette = snapshot.palette.as_ref();
    swatch_combo(
        ui,
        "Base swatch",
        "style_base_swatch",
        palette,
        &mut state.style_form.base_swatch,
    );

    ui.horizontal(|ui| {
        ui.label("Surface");
        egui::ComboBox::from_id_salt("style_surface_mode")
            .selected_text(surface_mode_label(state.style_form.surface_mode))
            .show_ui(ui, |ui| {
                for mode in [
                    VoxelSurfaceMode::Opaque,
                    VoxelSurfaceMode::Cutout,
                    VoxelSurfaceMode::Translucent,
                    VoxelSurfaceMode::Additive,
                ] {
                    ui.selectable_value(
                        &mut state.style_form.surface_mode,
                        mode,
                        surface_mode_label(mode),
                    );
                }
            });
    });
    if state.style_form.surface_mode == VoxelSurfaceMode::Opaque {
        state.style_form.opacity = 1.0;
        ui.add_enabled(
            false,
            egui::Slider::new(&mut state.style_form.opacity, 0.01..=1.0).text("Opacity"),
        );
    } else {
        ui.add(
            egui::Slider::new(&mut state.style_form.opacity, 0.01..=1.0)
                .text("Opacity")
                .clamping(egui::SliderClamping::Always),
        );
    }

    ui.separator();
    ui.checkbox(
        &mut state.style_form.emission_enabled,
        "Independent emission",
    );
    ui.add_enabled_ui(state.style_form.emission_enabled, |ui| {
        swatch_combo(
            ui,
            "Emission swatch",
            "style_emission_swatch",
            palette,
            &mut state.style_form.emission_swatch,
        );
        ui.add(
            egui::DragValue::new(&mut state.style_form.emission_strength)
                .range(0.0..=10_000.0)
                .speed(0.1)
                .prefix("Strength "),
        );
    });

    ui.add_space(10.0);
    let candidate = state.style_form.candidate();
    if let Err(error) = &candidate {
        ui.colored_label(ERROR, error);
    }
    ui.horizontal(|ui| {
        let save_label = if state.style_form.is_new {
            "Create Style"
        } else {
            "Update Style"
        };
        let save_clicked = ui
            .add_enabled(
                candidate.is_ok() && snapshot.styles.is_some() && snapshot.palette.is_some(),
                egui::Button::new(save_label),
            )
            .clicked();
        if save_clicked {
            if let Ok((id, style)) = candidate {
                actions.push(WorkshopUiAction::UpsertStyle { id, style });
            }
        }
        if !state.style_form.is_new {
            let delete_clicked = ui
                .button("Delete")
                .on_hover_text("Delete only when no object references this style")
                .clicked();
            if delete_clicked {
                if let Some(id) = &state.selected_style {
                    state.pending_delete = Some(DeleteTarget::Style(id.clone()));
                }
            }
        }
    });

    if let Some(id) = &state.selected_style {
        draw_style_impact(ui, snapshot, id);
    }
}

fn draw_object_inspector(
    ui: &mut egui::Ui,
    snapshot: &WorkshopUiSnapshot,
    state: &mut WorkshopUiState,
    actions: &mut Vec<WorkshopUiAction>,
) {
    let Some(editor) = &snapshot.editor else {
        ui.heading("Object");
        unavailable_label(ui);
        return;
    };
    sync_object_form(state, editor);

    ui.horizontal(|ui| {
        ui.heading(&editor.object.display_name);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button("Delete")
                .on_hover_text("Delete this saved object after confirmation")
                .clicked()
            {
                state.pending_delete = Some(DeleteTarget::Object(editor.object.id.clone()));
            }
        });
    });
    ui.label(
        egui::RichText::new(editor.object.id.as_str())
            .monospace()
            .small()
            .color(MUTED),
    );
    ui.add_space(5.0);

    egui::CollapsingHeader::new("Document")
        .default_open(false)
        .show(ui, |ui| {
            ui.label("Display name");
            let response = ui.text_edit_singleline(&mut state.object_name);
            let apply = (response.lost_focus()
                && ui.input(|input| input.key_pressed(egui::Key::Enter))
                || ui
                    .add_enabled(
                        state.object_name.trim() != editor.object.display_name,
                        egui::Button::new("Apply Name"),
                    )
                    .clicked())
                && state.object_name.trim() != editor.object.display_name;
            if apply {
                actions.push(WorkshopUiAction::SetObjectDisplayName(
                    state.object_name.trim().to_owned(),
                ));
            }

            ui.horizontal(|ui| {
                ui.label("Category");
                ui.label(category_label(editor.object.category));
            });

            if editor.object.category == ObjectCategory::Prop {
                ui.horizontal(|ui| {
                    ui.label("Connectivity");
                    let before = state.object_connectivity;
                    egui::ComboBox::from_id_salt("object_connectivity")
                        .selected_text(connectivity_label(state.object_connectivity))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut state.object_connectivity,
                                ConnectivityPolicy::Grounded,
                                "Grounded",
                            );
                            ui.selectable_value(
                                &mut state.object_connectivity,
                                ConnectivityPolicy::Free,
                                "Free",
                            );
                        });
                    if before != state.object_connectivity {
                        actions.push(WorkshopUiAction::SetObjectConnectivity(
                            state.object_connectivity,
                        ));
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    ui.label("Connectivity");
                    ui.label(connectivity_label(editor.object.connectivity));
                });
            }

            let mut bounds_changed = false;
            ui.label("Authoring bounds");
            ui.horizontal(|ui| {
                bounds_changed |= ui
                    .add(
                        egui::DragValue::new(&mut state.object_bounds.radius)
                            .range(0..=12)
                            .prefix("Radius "),
                    )
                    .changed();
                bounds_changed |= ui
                    .add(
                        egui::DragValue::new(&mut state.object_bounds.height)
                            .range(1..=64)
                            .prefix("Height "),
                    )
                    .changed();
            });
            bounds_changed |= ui
                .add(
                    egui::DragValue::new(&mut state.object_bounds.min_level)
                        .prefix("Minimum level "),
                )
                .changed();
            if bounds_changed {
                actions.push(WorkshopUiAction::SetObjectBounds(state.object_bounds));
            }

            ui.horizontal(|ui| {
                ui.label("Origin");
                ui.monospace(format_coord(editor.object.origin));
                if ui
                    .add_enabled(
                        editor.selection_count == 1,
                        egui::Button::new("Use Selection"),
                    )
                    .on_hover_text("Move the explicit object origin to the selected voxel")
                    .clicked()
                {
                    actions.push(WorkshopUiAction::SetOriginFromSelection);
                }
            });
        });

    ui.add_space(6.0);
    ui.label(egui::RichText::new("Tools").strong());
    ui.horizontal_wrapped(|ui| {
        for tool in [
            EditorTool::Place,
            EditorTool::Erase,
            EditorTool::Repaint,
            EditorTool::Eyedropper,
            EditorTool::Select,
        ] {
            if ui
                .selectable_label(editor.tool == tool, tool_label(tool))
                .on_hover_text(tool_tooltip(tool))
                .clicked()
                && editor.tool != tool
            {
                actions.push(WorkshopUiAction::SetTool(tool));
            }
        }
    });

    ui.add_space(6.0);
    part_combo(ui, editor.object.category, editor.active_part, actions);

    ui.separator();
    ui.label(egui::RichText::new("View").strong());
    let blocker_changed = ui
        .checkbox(&mut state.show_blockers, "Blocker footprint")
        .on_hover_text("Overlay exact horizontal gameplay blockers")
        .changed();
    if blocker_changed {
        actions.push(WorkshopUiAction::ShowBlockerOverlay(state.show_blockers));
    }
    let canopy_changed = ui
        .checkbox(&mut state.show_canopy, "Canopy occluders")
        .on_hover_text("Overlay exact cells eligible for canopy cutaway")
        .changed();
    if canopy_changed {
        actions.push(WorkshopUiAction::ShowCanopyOverlay(state.show_canopy));
    }
    let semantic_changed = ui
        .checkbox(&mut state.show_semantics, "Semantic roles")
        .on_hover_text("Colour-code voxels by their authored role")
        .changed();
    if semantic_changed {
        actions.push(WorkshopUiAction::ShowSemanticOverlay(state.show_semantics));
    }
    let slice_changed = ui
        .checkbox(&mut state.isolate_active_level, "Isolate active level")
        .changed();
    if slice_changed {
        actions.push(WorkshopUiAction::IsolateActiveLevel(
            state.isolate_active_level,
        ));
    }
    let grid_changed = ui.checkbox(&mut state.show_grid, "Hex guide").changed();
    if grid_changed {
        actions.push(WorkshopUiAction::ShowGrid(state.show_grid));
    }

    ui.separator();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Selection").strong());
        ui.label(
            egui::RichText::new(editor.selection_count.to_string())
                .monospace()
                .color(MUTED),
        );
    });
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(editor.selection_count > 0, egui::Button::new("Copy"))
            .on_hover_text("Copy selected voxels and masks")
            .clicked()
        {
            actions.push(WorkshopUiAction::CopySelection);
        }
        if ui
            .add_enabled(editor.clipboard_count > 0, egui::Button::new("Paste"))
            .on_hover_text("Paste at the viewport cursor or active-level origin")
            .clicked()
        {
            actions.push(WorkshopUiAction::PasteSelection);
        }
        if ui
            .add_enabled(editor.selection_count > 0, egui::Button::new("Delete"))
            .on_hover_text("Delete selected voxels as one command")
            .clicked()
        {
            actions.push(WorkshopUiAction::DeleteSelection);
        }
        if ui
            .add_enabled(editor.selection_count > 0, egui::Button::new("Clear"))
            .clicked()
        {
            actions.push(WorkshopUiAction::ClearSelection);
        }
    });

    ui.label(egui::RichText::new("Nudge").small().color(MUTED));
    ui.horizontal_wrapped(|ui| {
        for (label, q, r, level) in [
            ("Q-", -1, 0, 0),
            ("Q+", 1, 0, 0),
            ("R-", 0, -1, 0),
            ("R+", 0, 1, 0),
            ("Down", 0, 0, -1),
            ("Up", 0, 0, 1),
        ] {
            if ui
                .add_enabled(editor.selection_count > 0, egui::Button::new(label))
                .clicked()
            {
                actions.push(WorkshopUiAction::NudgeSelection { q, r, level });
            }
        }
    });
    if ui
        .add_enabled(
            editor.selection_count > 0,
            egui::Button::new("Rotate 60 CW"),
        )
        .on_hover_text("Rotate exactly around the object origin")
        .clicked()
    {
        actions.push(WorkshopUiAction::RotateSelectionClockwise);
    }

    if editor.selection_count > 0 {
        ui.add_space(5.0);
        if ui.button("Apply Active Role").clicked() {
            actions.push(WorkshopUiAction::RepaintSelectionPart(editor.active_part));
        }
        match editor.object.category {
            ObjectCategory::Plant => {
                ui.horizontal(|ui| {
                    if ui.small_button("Add to Canopy").clicked() {
                        actions.push(WorkshopUiAction::SetSelectionCanopy(true));
                    }
                    if ui.small_button("Remove from Canopy").clicked() {
                        actions.push(WorkshopUiAction::SetSelectionCanopy(false));
                    }
                });
            }
            ObjectCategory::Prop => {
                ui.horizontal(|ui| {
                    if ui.small_button("Mark Blocking").clicked() {
                        actions.push(WorkshopUiAction::SetSelectionBlocker(true));
                    }
                    if ui.small_button("Clear Blocking").clicked() {
                        actions.push(WorkshopUiAction::SetSelectionBlocker(false));
                    }
                });
            }
            ObjectCategory::Effect => {}
        }
    }

    ui.separator();
    let validation = editor.object.validate_intrinsic();
    match validation {
        Ok(()) => {
            ui.colored_label(SUCCESS, "Intrinsic object contract valid");
        }
        Err(error) => {
            ui.colored_label(ERROR, error);
        }
    }
}

fn sync_object_form(state: &mut WorkshopUiState, editor: &ObjectEditorSnapshot) {
    if state.object_form_id.as_ref() == Some(&editor.object.id) {
        return;
    }
    state.object_form_id = Some(editor.object.id.clone());
    state.object_name = editor.object.display_name.clone();
    state.object_bounds = editor.object.bounds;
    state.object_connectivity = editor.object.connectivity;
}

fn part_combo(
    ui: &mut egui::Ui,
    category: ObjectCategory,
    active: ObjectPart,
    actions: &mut Vec<WorkshopUiAction>,
) {
    ui.horizontal(|ui| {
        ui.label("Role");
        egui::ComboBox::from_id_salt("active_object_part")
            .selected_text(part_label(active))
            .show_ui(ui, |ui| {
                for part in parts_for_category(category) {
                    if ui
                        .selectable_label(active == part, part_label(part))
                        .clicked()
                        && active != part
                    {
                        actions.push(WorkshopUiAction::SetActivePart(part));
                    }
                }
            });
    });
}

fn draw_object_dialog(
    context: &egui::Context,
    snapshot: &WorkshopUiSnapshot,
    state: &mut WorkshopUiState,
    actions: &mut Vec<WorkshopUiAction>,
) {
    let Some(mut dialog) = state.object_dialog.take() else {
        return;
    };
    let mut close = false;
    egui::Modal::new(egui::Id::new("object_identity_dialog")).show(context, |ui| {
        ui.set_min_width(340.0);
        ui.heading(match dialog.kind {
            ObjectDialogKind::New => "New Object",
            ObjectDialogKind::SaveAs => "Save Object As",
            ObjectDialogKind::Duplicate { .. } => "Duplicate Object",
        });
        ui.add_space(5.0);
        labeled_text(ui, "Stable key", &mut dialog.id, false);
        labeled_text(ui, "Display name", &mut dialog.display_name, false);

        if matches!(dialog.kind, ObjectDialogKind::New) {
            ui.horizontal(|ui| {
                ui.label("Category");
                egui::ComboBox::from_id_salt("new_object_category")
                    .selected_text(category_label(dialog.category))
                    .show_ui(ui, |ui| {
                        for category in [
                            ObjectCategory::Plant,
                            ObjectCategory::Effect,
                            ObjectCategory::Prop,
                        ] {
                            ui.selectable_value(
                                &mut dialog.category,
                                category,
                                category_label(category),
                            );
                        }
                    });
            });
            if dialog.category == ObjectCategory::Prop {
                ui.horizontal(|ui| {
                    ui.label("Connectivity");
                    egui::ComboBox::from_id_salt("new_prop_connectivity")
                        .selected_text(connectivity_label(dialog.prop_connectivity))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut dialog.prop_connectivity,
                                ConnectivityPolicy::Grounded,
                                "Grounded",
                            );
                            ui.selectable_value(
                                &mut dialog.prop_connectivity,
                                ConnectivityPolicy::Free,
                                "Free",
                            );
                        });
                });
            }
        }

        let parsed_id = ObjectAssetId::new(dialog.id.trim()).map_err(|error| error.to_string());
        let duplicate_id = parsed_id
            .as_ref()
            .is_ok_and(|id| snapshot.objects.iter().any(|object| object.id == *id));
        let name_valid = !dialog.display_name.trim().is_empty();
        if duplicate_id {
            dialog.error = Some("that object id already exists".to_owned());
        } else if !name_valid {
            dialog.error = Some("display name cannot be empty".to_owned());
        } else if let Err(error) = &parsed_id {
            dialog.error = Some(error.clone());
        } else {
            dialog.error = None;
        }
        if let Some(error) = &dialog.error {
            ui.colored_label(ERROR, error);
        }

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let submit_label = match dialog.kind {
                ObjectDialogKind::New => "Create",
                ObjectDialogKind::SaveAs => "Save As",
                ObjectDialogKind::Duplicate { .. } => "Duplicate",
            };
            let submit_clicked = ui
                .add_enabled(dialog.error.is_none(), egui::Button::new(submit_label))
                .clicked();
            if submit_clicked {
                if let Ok(id) = parsed_id {
                    let display_name = dialog.display_name.trim().to_owned();
                    match &dialog.kind {
                        ObjectDialogKind::New => {
                            actions.push(WorkshopUiAction::NewObject {
                                id,
                                display_name,
                                category: dialog.category,
                                prop_connectivity: dialog.prop_connectivity,
                            });
                        }
                        ObjectDialogKind::SaveAs => {
                            actions.push(WorkshopUiAction::SaveObjectAs { id, display_name });
                        }
                        ObjectDialogKind::Duplicate { source } => {
                            actions.push(WorkshopUiAction::DuplicateObject {
                                source: source.clone(),
                                id,
                                display_name,
                            });
                        }
                    }
                    close = true;
                }
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    });
    if !close {
        state.object_dialog = Some(dialog);
    }
}

fn draw_delete_confirmation(
    context: &egui::Context,
    state: &mut WorkshopUiState,
    actions: &mut Vec<WorkshopUiAction>,
) {
    let Some(target) = state.pending_delete.take() else {
        return;
    };
    let mut close = false;
    egui::Modal::new(egui::Id::new("delete_asset_dialog")).show(context, |ui| {
        ui.set_min_width(330.0);
        ui.heading("Delete Asset");
        ui.label(format!(
            "Delete {}? References are checked before any file changes.",
            delete_target_label(&target)
        ));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Delete").clicked() {
                actions.push(match &target {
                    DeleteTarget::Swatch(id) => WorkshopUiAction::DeleteSwatch(id.clone()),
                    DeleteTarget::Style(id) => WorkshopUiAction::DeleteStyle(id.clone()),
                    DeleteTarget::Object(id) => WorkshopUiAction::DeleteObject(id.clone()),
                });
                close = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    });
    if !close {
        state.pending_delete = Some(target);
    }
}

fn draw_swatch_impact(ui: &mut egui::Ui, snapshot: &WorkshopUiSnapshot, id: &SwatchId) {
    let Some(styles) = &snapshot.styles else {
        return;
    };
    let referenced_styles = styles.references_to(id);
    let mut object_ids = BTreeSet::new();
    for object in &snapshot.objects {
        if object
            .placements
            .iter()
            .any(|placement| referenced_styles.contains(&placement.style))
        {
            object_ids.insert(object.id.clone());
        }
    }
    ui.add_space(10.0);
    egui::CollapsingHeader::new("Global impact")
        .default_open(false)
        .show(ui, |ui| {
            ui.label(format!(
                "{} styles, {} objects",
                referenced_styles.len(),
                object_ids.len()
            ));
            for style in &referenced_styles {
                ui.monospace(style.as_str());
            }
            for object in object_ids {
                ui.monospace(object.as_str());
            }
        });
}

fn draw_style_impact(ui: &mut egui::Ui, snapshot: &WorkshopUiSnapshot, id: &VoxelStyleId) {
    let objects: Vec<_> = snapshot
        .objects
        .iter()
        .filter(|object| {
            object
                .placements
                .iter()
                .any(|placement| placement.style == *id)
        })
        .collect();
    ui.add_space(10.0);
    egui::CollapsingHeader::new("Object usage")
        .default_open(false)
        .show(ui, |ui| {
            if objects.is_empty() {
                ui.label(egui::RichText::new("No saved object references").color(MUTED));
            }
            for object in objects {
                let count = object
                    .placements
                    .iter()
                    .filter(|placement| placement.style == *id)
                    .count();
                ui.label(format!("{} ({count})", object.id));
            }
        });
}

fn search_field(ui: &mut egui::Ui, id: &'static str, search: &mut String) {
    ui.add_sized(
        [ui.available_width(), SEARCH_HEIGHT],
        egui::TextEdit::singleline(search)
            .id_salt(id)
            .hint_text("Search"),
    );
}

fn labeled_text(ui: &mut egui::Ui, label: &str, value: &mut String, locked: bool) {
    ui.label(label);
    ui.add_enabled(
        !locked,
        egui::TextEdit::singleline(value).desired_width(f32::INFINITY),
    );
}

fn swatch_combo(
    ui: &mut egui::Ui,
    label: &str,
    id: &'static str,
    palette: Option<&ArtPalette>,
    selected: &mut Option<SwatchId>,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        let selected_text = selected
            .as_ref()
            .and_then(|selected_id| {
                palette
                    .and_then(|palette| palette.get(selected_id))
                    .map(PaletteSwatch::display_name)
            })
            .unwrap_or("Choose...");
        egui::ComboBox::from_id_salt(id)
            .selected_text(selected_text)
            .width(150.0)
            .show_ui(ui, |ui| {
                let Some(palette) = palette else {
                    ui.label("Palette unavailable");
                    return;
                };
                for (swatch_id, swatch) in palette.swatches() {
                    ui.horizontal(|ui| {
                        color_chip(ui, swatch.color(), egui::vec2(15.0, 15.0));
                        ui.selectable_value(
                            selected,
                            Some(swatch_id.clone()),
                            swatch.display_name(),
                        );
                    });
                }
            });
    });
}

fn color_chip(ui: &mut egui::Ui, color: SrgbColor, size: egui::Vec2) {
    let [red, green, blue] = color.to_array();
    let color = egui::Color32::from_rgb(
        float_to_byte(red),
        float_to_byte(green),
        float_to_byte(blue),
    );
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().rect(
        rect,
        3.0,
        color,
        egui::Stroke::new(1.0_f32, BORDER),
        egui::StrokeKind::Inside,
    );
}

fn unavailable_label(ui: &mut egui::Ui) {
    ui.label(
        egui::RichText::new("Project assets unavailable")
            .color(ERROR)
            .small(),
    );
}

fn nearest_distinct_swatches(
    palette: &ArtPalette,
    color: SrgbColor,
    excluded: Option<&SwatchId>,
    limit: usize,
) -> Vec<(SwatchId, f32)> {
    palette
        .nearest_swatches(color, palette.swatches().len())
        .into_iter()
        .filter(|candidate| excluded != Some(&candidate.id))
        .take(limit)
        .map(|candidate| (candidate.id, candidate.distance))
        .collect()
}

fn matches_search<'a>(query: &str, values: impl IntoIterator<Item = &'a str>) -> bool {
    let query = query.trim().to_ascii_lowercase();
    query.is_empty()
        || values
            .into_iter()
            .any(|value| value.to_ascii_lowercase().contains(&query))
}

fn parse_tags(value: &str) -> Result<BTreeSet<String>, String> {
    let mut tags = BTreeSet::new();
    for raw in value.split(',') {
        let tag = raw.trim();
        if tag.is_empty() {
            continue;
        }
        if !tags.insert(tag.to_owned()) {
            return Err(format!("tag '{tag}' is repeated"));
        }
    }
    Ok(tags)
}

fn parse_hex_color(value: &str) -> Result<[f32; 3], String> {
    let value = value.trim().strip_prefix('#').unwrap_or(value.trim());
    if value.chars().count() != 6 || !value.chars().all(|character| character.is_ascii_hexdigit()) {
        return Err("hex colour must contain exactly six hexadecimal digits".to_owned());
    }
    let mut characters = value.chars();
    let red = parse_hex_pair(characters.next(), characters.next())?;
    let green = parse_hex_pair(characters.next(), characters.next())?;
    let blue = parse_hex_pair(characters.next(), characters.next())?;
    Ok([
        f32::from(red) / 255.0,
        f32::from(green) / 255.0,
        f32::from(blue) / 255.0,
    ])
}

fn parse_hex_pair(high: Option<char>, low: Option<char>) -> Result<u8, String> {
    let high = high
        .and_then(|character| character.to_digit(16))
        .ok_or_else(|| "hex colour contains an invalid digit".to_owned())?;
    let low = low
        .and_then(|character| character.to_digit(16))
        .ok_or_else(|| "hex colour contains an invalid digit".to_owned())?;
    let value = high
        .checked_mul(16)
        .and_then(|value| value.checked_add(low))
        .ok_or_else(|| "hex colour component overflowed".to_owned())?;
    u8::try_from(value).map_err(|_conversion_error| "hex colour component overflowed".to_owned())
}

fn rgb_to_hex(rgb: [f32; 3]) -> String {
    let [red, green, blue] = rgb.map(float_to_byte);
    format!("{red:02X}{green:02X}{blue:02X}")
}

fn float_to_byte(value: f32) -> u8 {
    let target = (value.clamp(0.0, 1.0) * 255.0).round();
    for candidate in 0..=u8::MAX {
        if f32::from(candidate) >= target {
            return candidate;
        }
    }
    u8::MAX
}

fn rgb_to_hsv(rgb: [f32; 3]) -> [f32; 3] {
    let [red, green, blue] = rgb;
    let maximum = red.max(green).max(blue);
    let minimum = red.min(green).min(blue);
    let delta = maximum - minimum;
    let hue = if delta <= f32::EPSILON {
        0.0
    } else if red >= green && red >= blue {
        60.0 * ((green - blue) / delta).rem_euclid(6.0)
    } else if green >= blue {
        60.0 * ((blue - red) / delta + 2.0)
    } else {
        60.0 * ((red - green) / delta + 4.0)
    };
    let saturation = if maximum <= f32::EPSILON {
        0.0
    } else {
        delta / maximum
    };
    [hue, saturation * 100.0, maximum * 100.0]
}

fn hsv_to_rgb(hsv: [f32; 3]) -> [f32; 3] {
    let [hue, saturation_percent, value_percent] = hsv;
    let saturation = (saturation_percent / 100.0).clamp(0.0, 1.0);
    let value = (value_percent / 100.0).clamp(0.0, 1.0);
    let chroma = value * saturation;
    let sector = hue.rem_euclid(360.0) / 60.0;
    let intermediate = chroma * (1.0 - (sector.rem_euclid(2.0) - 1.0).abs());
    let (red, green, blue) = if sector < 1.0 {
        (chroma, intermediate, 0.0)
    } else if sector < 2.0 {
        (intermediate, chroma, 0.0)
    } else if sector < 3.0 {
        (0.0, chroma, intermediate)
    } else if sector < 4.0 {
        (0.0, intermediate, chroma)
    } else if sector < 5.0 {
        (intermediate, 0.0, chroma)
    } else {
        (chroma, 0.0, intermediate)
    };
    let match_value = value - chroma;
    [red + match_value, green + match_value, blue + match_value]
}

fn tags_label(tags: &BTreeSet<String>) -> String {
    if tags.is_empty() {
        "No tags".to_owned()
    } else {
        tags.iter().cloned().collect::<Vec<_>>().join(", ")
    }
}

fn status_color(kind: WorkshopStatusKind) -> egui::Color32 {
    match kind {
        WorkshopStatusKind::Info => MUTED,
        WorkshopStatusKind::Success => SUCCESS,
        WorkshopStatusKind::Warning => WARNING,
        WorkshopStatusKind::Error => ERROR,
    }
}

fn rig_label(rig: PreviewRig) -> &'static str {
    match rig {
        PreviewRig::Neutral => "Neutral",
        PreviewRig::Dark => "Dark",
        PreviewRig::Unlit => "Unlit",
    }
}

fn surface_mode_label(mode: VoxelSurfaceMode) -> &'static str {
    match mode {
        VoxelSurfaceMode::Opaque => "Opaque",
        VoxelSurfaceMode::Cutout => "Cutout",
        VoxelSurfaceMode::Translucent => "Translucent",
        VoxelSurfaceMode::Additive => "Additive",
    }
}

fn category_label(category: ObjectCategory) -> &'static str {
    match category {
        ObjectCategory::Plant => "Plant",
        ObjectCategory::Effect => "Effect",
        ObjectCategory::Prop => "Prop",
    }
}

fn connectivity_label(connectivity: ConnectivityPolicy) -> &'static str {
    match connectivity {
        ConnectivityPolicy::Grounded => "Grounded",
        ConnectivityPolicy::Free => "Free",
    }
}

fn tool_label(tool: EditorTool) -> &'static str {
    match tool {
        EditorTool::Place => "Place",
        EditorTool::Erase => "Erase",
        EditorTool::Repaint => "Repaint",
        EditorTool::Eyedropper => "Pick",
        EditorTool::Select => "Select",
    }
}

fn tool_tooltip(tool: EditorTool) -> &'static str {
    match tool {
        EditorTool::Place => "Place from a clicked face or the active level",
        EditorTool::Erase => "Erase an occupied voxel",
        EditorTool::Repaint => "Apply the active style and semantic role to an occupied voxel",
        EditorTool::Eyedropper => "Sample a voxel's style and role",
        EditorTool::Select => "Select occupied voxels for exact transforms",
    }
}

fn parts_for_category(category: ObjectCategory) -> Vec<ObjectPart> {
    match category {
        ObjectCategory::Plant => vec![
            ObjectPart::Plant(PlantPart::Root),
            ObjectPart::Plant(PlantPart::Trunk),
            ObjectPart::Plant(PlantPart::Branch),
            ObjectPart::Plant(PlantPart::Foliage),
            ObjectPart::Plant(PlantPart::Accent),
        ],
        ObjectCategory::Effect => vec![
            ObjectPart::Effect(EffectPart::Core),
            ObjectPart::Effect(EffectPart::Trail),
            ObjectPart::Effect(EffectPart::Accent),
        ],
        ObjectCategory::Prop => vec![
            ObjectPart::Prop(PropPart::Structure),
            ObjectPart::Prop(PropPart::Detail),
        ],
    }
}

fn part_label(part: ObjectPart) -> &'static str {
    match part {
        ObjectPart::Plant(PlantPart::Root) => "Root",
        ObjectPart::Plant(PlantPart::Trunk) => "Trunk",
        ObjectPart::Plant(PlantPart::Branch) => "Branch",
        ObjectPart::Plant(PlantPart::Foliage) => "Foliage",
        ObjectPart::Plant(PlantPart::Accent) => "Accent",
        ObjectPart::Effect(EffectPart::Core) => "Core",
        ObjectPart::Effect(EffectPart::Trail) => "Trail",
        ObjectPart::Effect(EffectPart::Accent) => "Accent",
        ObjectPart::Prop(PropPart::Structure) => "Structure",
        ObjectPart::Prop(PropPart::Detail) => "Detail",
    }
}

fn delete_target_label(target: &DeleteTarget) -> String {
    match target {
        DeleteTarget::Swatch(id) => format!("swatch '{id}'"),
        DeleteTarget::Style(id) => format!("style '{id}'"),
        DeleteTarget::Object(id) => format!("object '{id}'"),
    }
}

fn format_coord(position: hex_assets::LocalVoxelCoord) -> String {
    format!("({}, {}, {})", position.q, position.r, position.level)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(left: f32, right: f32) {
        assert!((left - right).abs() < 0.000_1, "{left} != {right}");
    }

    #[test]
    fn hex_parser_accepts_hash_and_round_trips() {
        assert_eq!(
            parse_hex_color("#0A80FF").map(rgb_to_hex),
            Ok("0A80FF".to_owned())
        );
    }

    #[test]
    fn hex_parser_rejects_ambiguous_lengths() {
        assert!(parse_hex_color("fff").is_err());
        assert!(parse_hex_color("1234567").is_err());
        assert!(parse_hex_color("00ZZ00").is_err());
    }

    #[test]
    fn rgb_hsv_conversion_round_trips_primary_and_neutral_colours() {
        for rgb in [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.42, 0.42, 0.42],
            [0.13, 0.72, 0.38],
        ] {
            let [actual_red, actual_green, actual_blue] = hsv_to_rgb(rgb_to_hsv(rgb));
            let [red, green, blue] = rgb;
            close(actual_red, red);
            close(actual_green, green);
            close(actual_blue, blue);
        }
    }

    #[test]
    fn search_matches_ids_names_and_tags_case_insensitively() {
        assert!(matches_search("FoLi", ["plant/oak", "Oak", "foliage"]));
        assert!(!matches_search("metal", ["plant/oak", "Oak", "foliage"]));
        assert!(matches_search("", ["anything"]));
    }

    #[test]
    fn duplicate_tags_are_rejected_before_contract_creation() {
        assert!(parse_tags("plant, foliage, plant").is_err());
        assert_eq!(parse_tags("plant, foliage").map(|tags| tags.len()), Ok(2));
    }

    #[test]
    fn central_viewport_accepts_pointer_without_ui_capture() {
        let viewport = egui::Rect::from_min_max(egui::pos2(100.0, 80.0), egui::pos2(900.0, 700.0));
        assert!(!viewport_pointer_is_suppressed(
            viewport,
            Some(egui::pos2(450.0, 320.0)),
            false,
            false,
        ));
    }

    #[test]
    fn panels_active_widgets_and_overlays_suppress_viewport_pointer() {
        let viewport = egui::Rect::from_min_max(egui::pos2(100.0, 80.0), egui::pos2(900.0, 700.0));
        assert!(viewport_pointer_is_suppressed(
            viewport,
            Some(egui::pos2(40.0, 320.0)),
            false,
            false,
        ));
        assert!(viewport_pointer_is_suppressed(
            viewport,
            Some(egui::pos2(450.0, 320.0)),
            true,
            false,
        ));
        assert!(viewport_pointer_is_suppressed(
            viewport,
            Some(egui::pos2(450.0, 320.0)),
            false,
            true,
        ));
        assert!(viewport_pointer_is_suppressed(viewport, None, false, false));
    }
}
