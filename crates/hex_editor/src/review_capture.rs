//! Sequential renderer orchestration for deterministic object review packs.
//!
//! The capture runner deliberately uses a dedicated offscreen camera. The
//! interactive camera and window target remain untouched while the shared
//! viewport content is temporarily replaced with each validated review
//! snapshot.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bevy::camera::{ClearColorConfig, RenderTarget};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::tasks::{futures::check_ready, AsyncComputeTaskPool, Task};
use hex_assets::{
    EffectPart, ObjectBlueprint, ObjectPart, ObjectPlacement, PlantPart, PropPart, VoxelStyleId,
    OBJECT_BLUEPRINT_SCHEMA_VERSION,
};
use image::RgbaImage;

use crate::project::{
    current_file_revision, current_project_revisions, ByteRevision, ProjectRevisionSet,
};
use crate::review::{
    captured_rgba, publish_review_pack_with_pre_rename_check, ReviewPresentation,
    ReviewPublishOutcome, ReviewReport, REVIEW_CLEAR_RGBA, REVIEW_FRAME_COUNT, REVIEW_FRAME_HEIGHT,
    REVIEW_FRAME_SPECS, REVIEW_FRAME_WIDTH,
};
use crate::viewport::{
    HoveredFaceTarget, RenderedVoxel, ViewportContent, ViewportEmission, ViewportInputEnabled,
    ViewportMode, ViewportPreviewRig, ViewportStyle, HEX_MESH_ASSET_PATH,
};

const SETTLE_FRAMES: u8 = 6;
const MAX_READBACK_ATTEMPTS: u8 = 4;
const PREPARE_TIMEOUT: Duration = Duration::from_secs(30);
const READBACK_TIMEOUT: Duration = Duration::from_secs(30);

/// Installs deterministic offscreen capture and publication for object reviews.
#[derive(Debug, Default)]
pub struct ReviewCapturePlugin;

impl Plugin for ReviewCapturePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ReviewCaptureState>()
            .init_resource::<ReviewCaptureProgress>()
            .add_message::<ReviewCaptureRequest>()
            .add_message::<ReviewCaptureFinished>()
            .add_message::<ReviewCaptureRejected>()
            .add_systems(PostUpdate, drive_review_capture);
    }
}

/// Requests one complete, immutable review pack.
///
/// `contents` must contain exactly one prepared viewport snapshot for each item
/// in [`REVIEW_FRAME_SPECS`], in that order.
#[derive(Message, Debug, Clone)]
pub struct ReviewCaptureRequest {
    /// Repository root under which the untracked review pack is published.
    pub repository_root: PathBuf,
    /// Exact tracked art-source revisions from which this review was prepared.
    pub expected_revisions: ProjectRevisionSet,
    /// Deterministic semantic report for the exact saved asset snapshot.
    pub report: ReviewReport,
    /// Ordered authored and diagnostic viewport presentations.
    pub contents: Vec<ViewportContent>,
}

impl ReviewCaptureRequest {
    /// Creates and validates a complete capture request.
    pub fn new(
        repository_root: PathBuf,
        expected_revisions: ProjectRevisionSet,
        report: ReviewReport,
        contents: Vec<ViewportContent>,
    ) -> Result<Self, String> {
        let request = Self {
            repository_root,
            expected_revisions,
            report,
            contents,
        };
        request.validate()?;
        Ok(request)
    }

    /// Validates renderer-facing invariants before any viewport state changes.
    pub fn validate(&self) -> Result<(), String> {
        self.validate_payload()?;
        verify_project_revisions(
            &self.repository_root,
            &self.expected_revisions,
            self.report.renderer_mesh_revision(),
            RevisionCheckPoint::RequestCreation,
        )
    }

    fn validate_payload(&self) -> Result<(), String> {
        if self.repository_root.as_os_str().is_empty() {
            return Err("review repository root cannot be empty".to_owned());
        }
        if !self.repository_root.is_dir() {
            return Err(format!(
                "review repository root '{}' is not a directory",
                self.repository_root.display()
            ));
        }
        self.report
            .validate_for_publication()
            .map_err(|error| error.to_string())?;
        self.report
            .to_ron_bytes()
            .map_err(|error| error.to_string())?;
        validate_review_contents(&self.contents)?;
        validate_report_snapshot(&self.report, &self.contents)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RevisionCheckPoint {
    RequestCreation,
    CaptureStart,
    Publication,
}

fn verify_project_revisions(
    repository_root: &Path,
    expected: &ProjectRevisionSet,
    expected_renderer_mesh: ByteRevision,
    checkpoint: RevisionCheckPoint,
) -> Result<(), String> {
    let current = current_project_revisions(repository_root).map_err(|error| {
        let action = match checkpoint {
            RevisionCheckPoint::RequestCreation => "creating the review request",
            RevisionCheckPoint::CaptureStart => "starting the review capture",
            RevisionCheckPoint::Publication => "publishing the review capture",
        };
        format!(
            "could not verify tracked art sources while {action}: {error}; \
             resolve the filesystem error, reload the project, and retry the review export"
        )
    })?;
    let timing = match checkpoint {
        RevisionCheckPoint::RequestCreation => "after the saved asset snapshot was prepared",
        RevisionCheckPoint::CaptureStart => "before the review renderer started",
        RevisionCheckPoint::Publication => "while the review capture was running",
    };
    if &current != expected {
        let changes = expected
            .files
            .keys()
            .chain(current.files.keys())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter_map(|path| {
                let change = match (expected.files.get(path), current.files.get(path)) {
                    (None, Some(_)) => "added",
                    (Some(_), None) => "removed",
                    (Some(expected), Some(current)) if expected != current => "modified",
                    _ => return None,
                };
                Some(format!("{path} ({change})"))
            })
            .collect::<Vec<_>>();
        let examples = changes
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let remainder = changes.len().saturating_sub(5);
        let suffix = if remainder == 0 {
            String::new()
        } else {
            format!(", and {remainder} more")
        };
        return Err(format!(
            "tracked art sources changed {timing}: {examples}{suffix}; \
             no review pack was published, so reload the project and retry the review export"
        ));
    }

    let renderer_mesh_path = Path::new("assets").join(HEX_MESH_ASSET_PATH);
    let current_renderer_mesh = current_file_revision(repository_root, &renderer_mesh_path)
        .map_err(|error| {
            format!(
                "could not verify renderer mesh while {}: {error}; \
                 restore the renderer asset and retry the review export",
                match checkpoint {
                    RevisionCheckPoint::RequestCreation => "creating the review request",
                    RevisionCheckPoint::CaptureStart => "starting the review capture",
                    RevisionCheckPoint::Publication => "publishing the review capture",
                }
            )
        })?;
    if current_renderer_mesh != expected_renderer_mesh {
        return Err(format!(
            "renderer source changed {timing}: {} (modified); \
             no review pack was published, so retry the review export",
            renderer_mesh_path.display()
        ));
    }
    Ok(())
}

fn publish_review_pack_if_sources_current(
    repository_root: &Path,
    expected_revisions: &ProjectRevisionSet,
    report: &ReviewReport,
    frames: &[RgbaImage],
) -> Result<ReviewPublishOutcome, String> {
    publish_review_pack_if_sources_current_with_hook(
        repository_root,
        expected_revisions,
        report,
        frames,
        || Ok(()),
    )
}

fn publish_review_pack_if_sources_current_with_hook(
    repository_root: &Path,
    expected_revisions: &ProjectRevisionSet,
    report: &ReviewReport,
    frames: &[RgbaImage],
    mut before_final_revision_check: impl FnMut() -> Result<(), String>,
) -> Result<ReviewPublishOutcome, String> {
    verify_project_revisions(
        repository_root,
        expected_revisions,
        report.renderer_mesh_revision(),
        RevisionCheckPoint::Publication,
    )?;
    publish_review_pack_with_pre_rename_check(repository_root, report, frames, || {
        before_final_revision_check()?;
        verify_project_revisions(
            repository_root,
            expected_revisions,
            report.renderer_mesh_revision(),
            RevisionCheckPoint::Publication,
        )
    })
    .map_err(|error| error.to_string())
}

/// Completion result for one accepted capture request.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct ReviewCaptureFinished {
    /// Published directory, idempotent existing directory, or actionable error.
    pub result: Result<ReviewPublishOutcome, String>,
}

/// Rejection of a request that never became the active capture transaction.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct ReviewCaptureRejected {
    /// Actionable reason the request was not accepted.
    pub error: String,
}

/// Observable phase of the one-click review operation.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ReviewCaptureProgress {
    /// No request is active.
    #[default]
    Idle,
    /// Renderer resources are being prepared for the first frame.
    Preparing,
    /// One frame is rebuilding and settling before readback.
    Settling {
        /// One-based frame ordinal.
        frame: u8,
        /// Total number of frames in the pack.
        total: u8,
    },
    /// One frame is being copied from the GPU.
    Reading {
        /// One-based frame ordinal.
        frame: u8,
        /// Total number of frames in the pack.
        total: u8,
    },
    /// Validated frames are being encoded and atomically published.
    Publishing,
}

#[derive(Resource, Default)]
struct ReviewCaptureState {
    next_token: u64,
    active: Option<ActiveCapture>,
}

struct ActiveCapture {
    token: u64,
    repository_root: PathBuf,
    expected_revisions: ProjectRevisionSet,
    report: ReviewReport,
    contents: Vec<ViewportContent>,
    frames: Vec<RgbaImage>,
    frame_index: usize,
    phase: CapturePhase,
    readback: Option<Result<RgbaImage, String>>,
    verification_frame: Option<RgbaImage>,
    readback_attempts: u8,
    viewport_snapshot: Option<ViewportSnapshot>,
    target: Option<Handle<Image>>,
    camera: Option<Entity>,
    screenshot: Option<Entity>,
}

enum CapturePhase {
    Settling {
        stable_frames: u8,
        started: Instant,
    },
    Reading {
        started: Instant,
    },
    Publishing {
        task: Task<Result<ReviewPublishOutcome, String>>,
    },
}

struct ViewportSnapshot {
    content: ViewportContent,
    mode: ViewportMode,
    rig: ViewportPreviewRig,
    input: ViewportInputEnabled,
}

#[derive(Component)]
struct ReviewCaptureCamera;

#[expect(
    clippy::too_many_arguments,
    reason = "the capture runner owns one sequential transaction across renderer, viewport, \
              filesystem-task, and message resources"
)]
fn drive_review_capture(
    mut commands: Commands,
    mut requests: MessageReader<ReviewCaptureRequest>,
    mut finished: MessageWriter<ReviewCaptureFinished>,
    mut rejected: MessageWriter<ReviewCaptureRejected>,
    mut state: ResMut<ReviewCaptureState>,
    mut progress: ResMut<ReviewCaptureProgress>,
    mut content: ResMut<ViewportContent>,
    mut mode: ResMut<ViewportMode>,
    mut rig: ResMut<ViewportPreviewRig>,
    mut input: ResMut<ViewportInputEnabled>,
    mut hovered: ResMut<HoveredFaceTarget>,
    mut images: ResMut<Assets<Image>>,
    meshes: Res<Assets<Mesh>>,
    rendered: Query<(&RenderedVoxel, &Mesh3d)>,
    mut review_camera: Query<&mut Transform, With<ReviewCaptureCamera>>,
) {
    let mut accepted = None;
    for request in requests.read().cloned() {
        if state.active.is_some() || accepted.is_some() {
            rejected.write(ReviewCaptureRejected {
                error: "a review capture is already running; wait for it to finish".to_owned(),
            });
        } else {
            accepted = Some(request);
        }
    }

    if let Some(request) = accepted {
        match begin_capture(
            request,
            &mut commands,
            &mut state,
            &mut progress,
            &mut content,
            &mut mode,
            &mut rig,
            &mut input,
            &mut hovered,
            &mut images,
        ) {
            Ok(()) => return,
            Err(error) => {
                *progress = ReviewCaptureProgress::Idle;
                finished.write(ReviewCaptureFinished { result: Err(error) });
                return;
            }
        }
    }

    let Some(mut active) = state.active.take() else {
        return;
    };

    if *mode != ViewportMode::Object {
        *mode = ViewportMode::Object;
    }
    if *rig != ViewportPreviewRig::Neutral {
        *rig = ViewportPreviewRig::Neutral;
    }
    if input.0 {
        input.0 = false;
    }
    if hovered.0.is_some() {
        hovered.0 = None;
    }

    let result = match &mut active.phase {
        CapturePhase::Settling {
            stable_frames,
            started,
        } => {
            if started.elapsed() > PREPARE_TIMEOUT {
                Some(Err(format!(
                    "review frame {} did not become renderable within {} seconds",
                    active.frame_index.saturating_add(1),
                    PREPARE_TIMEOUT.as_secs()
                )))
            } else {
                let expected = active.contents.get(active.frame_index);
                if expected.is_some_and(|expected| expected == content.as_ref())
                    && expected.is_some_and(|expected| {
                        rendered_content_is_ready(expected, rendered.iter(), &meshes)
                    })
                {
                    *stable_frames = stable_frames.saturating_add(1);
                } else {
                    if let Some(expected) = expected {
                        *content = expected.clone();
                    }
                    *stable_frames = 0;
                }
                if *stable_frames >= SETTLE_FRAMES {
                    match request_readback(&mut commands, &mut active) {
                        Ok(()) => {
                            *progress = ReviewCaptureProgress::Reading {
                                frame: frame_ordinal(active.frame_index),
                                total: review_frame_total(),
                            };
                            None
                        }
                        Err(error) => Some(Err(error)),
                    }
                } else {
                    *progress = ReviewCaptureProgress::Settling {
                        frame: frame_ordinal(active.frame_index),
                        total: review_frame_total(),
                    };
                    None
                }
            }
        }
        CapturePhase::Reading { started } => {
            if let Some(readback) = active.readback.take() {
                active.screenshot = None;
                match assess_readback(
                    active.verification_frame.take(),
                    readback,
                    active.readback_attempts,
                ) {
                    ReadbackDecision::Accept(frame) => {
                        active.frames.push(frame);
                        active.readback_attempts = 0;
                        let next_index = active.frame_index.saturating_add(1);
                        if next_index < active.contents.len() {
                            match apply_frame(
                                &mut active,
                                next_index,
                                &mut content,
                                &mut review_camera,
                            ) {
                                Ok(()) => {
                                    *progress = ReviewCaptureProgress::Settling {
                                        frame: frame_ordinal(next_index),
                                        total: review_frame_total(),
                                    };
                                    None
                                }
                                Err(error) => Some(Err(error)),
                            }
                        } else {
                            let repository_root = active.repository_root.clone();
                            let expected_revisions = active.expected_revisions.clone();
                            let report = active.report.clone();
                            let frames = std::mem::take(&mut active.frames);
                            let task = AsyncComputeTaskPool::get().spawn(async move {
                                publish_review_pack_if_sources_current(
                                    &repository_root,
                                    &expected_revisions,
                                    &report,
                                    &frames,
                                )
                            });
                            active.phase = CapturePhase::Publishing { task };
                            *progress = ReviewCaptureProgress::Publishing;
                            None
                        }
                    }
                    ReadbackDecision::Retry(candidate) => {
                        active.verification_frame = candidate;
                        restart_settle(&mut active);
                        *progress = ReviewCaptureProgress::Settling {
                            frame: frame_ordinal(active.frame_index),
                            total: review_frame_total(),
                        };
                        None
                    }
                    ReadbackDecision::Fail(error) => Some(Err(error)),
                }
            } else if started.elapsed() > READBACK_TIMEOUT {
                Some(Err(format!(
                    "review frame {} GPU readback timed out after {} seconds",
                    active.frame_index.saturating_add(1),
                    READBACK_TIMEOUT.as_secs()
                )))
            } else {
                None
            }
        }
        CapturePhase::Publishing { task } => check_ready(task),
    };

    if let Some(result) = result {
        restore_viewport(
            &mut active,
            &mut commands,
            &mut content,
            &mut mode,
            &mut rig,
            &mut input,
            &mut hovered,
            &mut images,
        );
        *progress = ReviewCaptureProgress::Idle;
        finished.write(ReviewCaptureFinished { result });
    } else {
        state.active = Some(active);
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "beginning the transaction snapshots every viewport resource that capture changes"
)]
fn begin_capture(
    request: ReviewCaptureRequest,
    commands: &mut Commands,
    state: &mut ReviewCaptureState,
    progress: &mut ReviewCaptureProgress,
    content: &mut ViewportContent,
    mode: &mut ViewportMode,
    rig: &mut ViewportPreviewRig,
    input: &mut ViewportInputEnabled,
    hovered: &mut HoveredFaceTarget,
    images: &mut Assets<Image>,
) -> Result<(), String> {
    request.validate_payload()?;
    verify_project_revisions(
        &request.repository_root,
        &request.expected_revisions,
        request.report.renderer_mesh_revision(),
        RevisionCheckPoint::CaptureStart,
    )?;
    let first_content = request
        .contents
        .first()
        .cloned()
        .ok_or_else(|| "review request does not contain a first frame".to_owned())?;
    let first_spec = REVIEW_FRAME_SPECS
        .first()
        .ok_or_else(|| "review frame contract is empty".to_owned())?;
    let first_pose = request
        .report
        .framing()
        .camera_pose(first_spec.camera)
        .map_err(|error| error.to_string())?;

    let target = images.add(Image::new_target_texture(
        REVIEW_FRAME_WIDTH,
        REVIEW_FRAME_HEIGHT,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    let clear = REVIEW_CLEAR_RGBA;
    let camera = commands
        .spawn((
            Camera3d::default(),
            Camera {
                order: -1,
                clear_color: ClearColorConfig::Custom(Color::srgba_u8(
                    clear[0], clear[1], clear[2], clear[3],
                )),
                ..default()
            },
            RenderTarget::Image(target.clone().into()),
            first_pose.transform(),
            ReviewCaptureCamera,
            Name::new("Asset Workshop Review Camera"),
        ))
        .id();

    let snapshot = ViewportSnapshot {
        content: content.clone(),
        mode: *mode,
        rig: *rig,
        input: *input,
    };
    *content = first_content;
    *mode = ViewportMode::Object;
    *rig = ViewportPreviewRig::Neutral;
    input.0 = false;
    hovered.0 = None;

    let token = state.next_token;
    state.next_token = state.next_token.wrapping_add(1);
    state.active = Some(ActiveCapture {
        token,
        repository_root: request.repository_root,
        expected_revisions: request.expected_revisions,
        report: request.report,
        contents: request.contents,
        frames: Vec::with_capacity(REVIEW_FRAME_COUNT),
        frame_index: 0,
        phase: CapturePhase::Settling {
            stable_frames: 0,
            started: Instant::now(),
        },
        readback: None,
        verification_frame: None,
        readback_attempts: 0,
        viewport_snapshot: Some(snapshot),
        target: Some(target),
        camera: Some(camera),
        screenshot: None,
    });
    *progress = ReviewCaptureProgress::Preparing;
    Ok(())
}

fn request_readback(commands: &mut Commands, active: &mut ActiveCapture) -> Result<(), String> {
    let target = active
        .target
        .clone()
        .ok_or_else(|| "review render target disappeared before readback".to_owned())?;
    let token = active.token;
    let frame_index = active.frame_index;
    active.readback_attempts = active.readback_attempts.saturating_add(1);
    let screenshot = commands
        .spawn(Screenshot::image(target))
        .observe(
            move |captured: On<ScreenshotCaptured>, mut state: ResMut<ReviewCaptureState>| {
                let Some(active) = state.active.as_mut() else {
                    return;
                };
                if active.token != token || active.frame_index != frame_index {
                    return;
                }
                active.readback =
                    Some(captured_rgba(&captured.image).map_err(|error| error.to_string()));
            },
        )
        .id();
    active.screenshot = Some(screenshot);
    active.phase = CapturePhase::Reading {
        started: Instant::now(),
    };
    Ok(())
}

fn apply_frame(
    active: &mut ActiveCapture,
    frame_index: usize,
    content: &mut ResMut<ViewportContent>,
    camera: &mut Query<&mut Transform, With<ReviewCaptureCamera>>,
) -> Result<(), String> {
    let next_content = active
        .contents
        .get(frame_index)
        .cloned()
        .ok_or_else(|| format!("review frame {} is missing", frame_index.saturating_add(1)))?;
    let spec = REVIEW_FRAME_SPECS
        .get(frame_index)
        .ok_or_else(|| format!("review frame contract has no item at index {frame_index}"))?;
    let pose = active
        .report
        .framing()
        .camera_pose(spec.camera)
        .map_err(|error| error.to_string())?;
    let mut transform = camera
        .single_mut()
        .map_err(|error| format!("review camera is unavailable: {error}"))?;

    content.set_if_neq(next_content);
    *transform = pose.transform();
    active.frame_index = frame_index;
    active.phase = CapturePhase::Settling {
        stable_frames: 0,
        started: Instant::now(),
    };
    active.readback = None;
    active.verification_frame = None;
    active.readback_attempts = 0;
    Ok(())
}

enum ReadbackDecision {
    Retry(Option<RgbaImage>),
    Accept(RgbaImage),
    Fail(String),
}

fn assess_readback(
    previous: Option<RgbaImage>,
    readback: Result<RgbaImage, String>,
    attempts: u8,
) -> ReadbackDecision {
    match readback {
        Ok(frame) => match previous {
            Some(previous) if previous == frame => ReadbackDecision::Accept(frame),
            _ if attempts < MAX_READBACK_ATTEMPTS => ReadbackDecision::Retry(Some(frame)),
            Some(_) => ReadbackDecision::Fail(format!(
                "review frame did not produce two identical GPU readbacks after \
                 {MAX_READBACK_ATTEMPTS} attempts"
            )),
            None => ReadbackDecision::Fail(format!(
                "review frame could not be confirmed after {MAX_READBACK_ATTEMPTS} attempts"
            )),
        },
        Err(_error) if attempts < MAX_READBACK_ATTEMPTS => ReadbackDecision::Retry(previous),
        Err(error) => ReadbackDecision::Fail(format!(
            "review frame GPU readback remained invalid after {MAX_READBACK_ATTEMPTS} attempts: \
             {error}"
        )),
    }
}

fn restart_settle(active: &mut ActiveCapture) {
    active.phase = CapturePhase::Settling {
        stable_frames: 0,
        started: Instant::now(),
    };
    active.readback = None;
}

#[expect(
    clippy::too_many_arguments,
    reason = "restoration mirrors the complete viewport snapshot captured at transaction start"
)]
fn restore_viewport(
    active: &mut ActiveCapture,
    commands: &mut Commands,
    content: &mut ViewportContent,
    mode: &mut ViewportMode,
    rig: &mut ViewportPreviewRig,
    input: &mut ViewportInputEnabled,
    hovered: &mut HoveredFaceTarget,
    images: &mut Assets<Image>,
) {
    if let Some(snapshot) = active.viewport_snapshot.take() {
        *content = snapshot.content;
        *mode = snapshot.mode;
        *rig = snapshot.rig;
        *input = snapshot.input;
    }
    hovered.0 = None;
    if let Some(screenshot) = active.screenshot.take() {
        commands.entity(screenshot).try_despawn();
    }
    if let Some(camera) = active.camera.take() {
        commands.entity(camera).try_despawn();
    }
    if let Some(target) = active.target.take() {
        drop(images.remove(target.id()));
    }
}

fn rendered_content_is_ready<'a>(
    expected: &ViewportContent,
    rendered: impl Iterator<Item = (&'a RenderedVoxel, &'a Mesh3d)>,
    meshes: &Assets<Mesh>,
) -> bool {
    let mut actual = Vec::new();
    for (voxel, mesh) in rendered {
        if meshes.get(mesh.0.id()).is_none() {
            return false;
        }
        actual.push(voxel.clone());
    }
    actual.sort_by(voxel_order);
    actual == expected.voxels
}

fn voxel_order(left: &RenderedVoxel, right: &RenderedVoxel) -> std::cmp::Ordering {
    left.position
        .cmp(&right.position)
        .then_with(|| left.style.cmp(&right.style))
}

fn validate_review_contents(contents: &[ViewportContent]) -> Result<(), String> {
    if contents.len() != REVIEW_FRAME_COUNT {
        return Err(format!(
            "review request contains {} viewport snapshots; expected {REVIEW_FRAME_COUNT}",
            contents.len()
        ));
    }

    for (content, spec) in contents.iter().zip(REVIEW_FRAME_SPECS) {
        if content.voxels.is_empty() {
            return Err(format!(
                "review frame {} ({}) has no occupied voxels",
                spec.ordinal, spec.file_name
            ));
        }
        if content.show_grid {
            return Err(format!(
                "review frame {} ({}) must hide the authoring grid",
                spec.ordinal, spec.file_name
            ));
        }
        if content.isolate_active_level {
            return Err(format!(
                "review frame {} ({}) cannot isolate one authoring level",
                spec.ordinal, spec.file_name
            ));
        }
        if !content.selected_cells.is_empty() {
            return Err(format!(
                "review frame {} ({}) cannot contain an editor selection",
                spec.ordinal, spec.file_name
            ));
        }
        for voxel in &content.voxels {
            if !content.styles.contains_key(&voxel.style) {
                return Err(format!(
                    "review frame {} ({}) references missing style '{}'",
                    spec.ordinal, spec.file_name, voxel.style
                ));
            }
        }
        let mut sorted = content.voxels.clone();
        sorted.sort_by(voxel_order);
        if sorted != content.voxels {
            return Err(format!(
                "review frame {} ({}) voxel placements are not deterministically ordered",
                spec.ordinal, spec.file_name
            ));
        }
        if sorted.windows(2).any(|pair| {
            pair.first()
                .zip(pair.last())
                .is_some_and(|(a, b)| a.position == b.position)
        }) {
            return Err(format!(
                "review frame {} ({}) contains overlapping voxel placements",
                spec.ordinal, spec.file_name
            ));
        }
        validate_presentation(content, spec.presentation, spec.ordinal, spec.file_name)?;
    }
    let reference = contents
        .first()
        .ok_or_else(|| "review request has no canonical viewport snapshot".to_owned())?;
    for (content, spec) in contents.iter().zip(REVIEW_FRAME_SPECS).skip(1) {
        if !same_authored_snapshot(reference, content) {
            return Err(format!(
                "review frame {} ({}) does not use the canonical object geometry, styles, or masks",
                spec.ordinal, spec.file_name
            ));
        }
    }
    Ok(())
}

fn same_authored_snapshot(left: &ViewportContent, right: &ViewportContent) -> bool {
    left.voxels == right.voxels
        && left.styles == right.styles
        && left.grid_radius == right.grid_radius
        && left.active_level == right.active_level
        && left.isolate_active_level == right.isolate_active_level
        && left.show_grid == right.show_grid
        && left.selected_cells == right.selected_cells
        && left.semantic_parts == right.semantic_parts
        && left.blocker_columns == right.blocker_columns
        && left.canopy_cells == right.canopy_cells
}

fn validate_report_snapshot(
    report: &ReviewReport,
    contents: &[ViewportContent],
) -> Result<(), String> {
    let content = contents
        .first()
        .ok_or_else(|| "review request has no canonical viewport snapshot".to_owned())?;
    if content.grid_radius != report.bounds().radius {
        return Err(format!(
            "review viewport radius {} differs from report radius {}",
            content.grid_radius,
            report.bounds().radius
        ));
    }
    if usize::try_from(report.occupied_cells()).ok() != Some(content.voxels.len()) {
        return Err(format!(
            "review viewport contains {} voxels but report declares {}",
            content.voxels.len(),
            report.occupied_cells()
        ));
    }
    if content.semantic_parts.len() != content.voxels.len() {
        return Err(
            "review semantic map must contain exactly one role for every occupied voxel".to_owned(),
        );
    }

    let mut placements = Vec::with_capacity(content.voxels.len());
    let mut style_counts = BTreeMap::<VoxelStyleId, u32>::new();
    let mut part_counts = BTreeMap::<String, u32>::new();
    for voxel in &content.voxels {
        let part = content.semantic_parts.get(&voxel.position).ok_or_else(|| {
            format!(
                "review voxel {:?} has no semantic role in the canonical snapshot",
                voxel.position
            )
        })?;
        let style_count = style_counts.entry(voxel.style.clone()).or_default();
        *style_count = style_count
            .checked_add(1)
            .ok_or_else(|| "review style placement count overflowed u32".to_owned())?;
        let part_count = part_counts
            .entry(review_part_label(*part).to_owned())
            .or_default();
        *part_count = part_count
            .checked_add(1)
            .ok_or_else(|| "review semantic-part count overflowed u32".to_owned())?;
        placements.push(ObjectPlacement {
            position: voxel.position,
            style: voxel.style.clone(),
            part: *part,
        });
    }

    let expected_style_counts = report
        .style_dependencies()
        .iter()
        .map(|dependency| (dependency.id.clone(), dependency.placements))
        .collect::<BTreeMap<_, _>>();
    if expected_style_counts.len() != report.style_dependencies().len()
        || style_counts != expected_style_counts
    {
        return Err("review viewport style counts differ from the report".to_owned());
    }
    validate_resolved_styles(report, content)?;
    if &part_counts != report.part_counts() {
        return Err("review viewport semantic-part counts differ from the report".to_owned());
    }
    if content.blocker_columns.iter().copied().collect::<Vec<_>>() != report.blocker_footprint() {
        return Err("review viewport blocker mask differs from the report".to_owned());
    }
    if content.canopy_cells.iter().copied().collect::<Vec<_>>() != report.canopy_occluders() {
        return Err("review viewport canopy mask differs from the report".to_owned());
    }

    let object = ObjectBlueprint {
        schema_version: OBJECT_BLUEPRINT_SCHEMA_VERSION,
        id: report.object_id().clone(),
        display_name: report.display_name().to_owned(),
        category: report.category(),
        bounds: report.bounds(),
        connectivity: report.connectivity(),
        origin: report.origin(),
        placements,
        blocker_footprint: report.blocker_footprint().to_vec(),
        canopy_occluders: report.canopy_occluders().to_vec(),
    };
    let object_fingerprint = object.semantic_fingerprint().map_err(|error| {
        format!("review viewport cannot reconstruct the reported object: {error}")
    })?;
    if object_fingerprint != report.object_fingerprint() {
        return Err("review viewport object fingerprint differs from the report".to_owned());
    }
    let framing = crate::review::ReviewFraming::from_object(&object).map_err(|error| {
        format!("review viewport cannot reconstruct the reported framing: {error}")
    })?;
    if framing != report.framing() {
        return Err("review viewport framing differs from the report".to_owned());
    }
    Ok(())
}

fn validate_resolved_styles(
    report: &ReviewReport,
    content: &ViewportContent,
) -> Result<(), String> {
    let swatches = report
        .swatch_dependencies()
        .iter()
        .map(|dependency| (dependency.id.clone(), dependency.color))
        .collect::<BTreeMap<_, _>>();
    for dependency in report.style_dependencies() {
        let color = swatches
            .get(&dependency.base_swatch)
            .copied()
            .ok_or_else(|| {
                format!(
                    "review style '{}' references missing report swatch '{}'",
                    dependency.id, dependency.base_swatch
                )
            })?;
        let emission = match (&dependency.emission_swatch, dependency.emission_strength) {
            (Some(swatch), Some(strength)) => {
                let color = swatches.get(swatch).copied().ok_or_else(|| {
                    format!(
                        "review style '{}' references missing report emission swatch '{}'",
                        dependency.id, swatch
                    )
                })?;
                Some(ViewportEmission { color, strength })
            }
            (None, None) => None,
            _ => {
                return Err(format!(
                    "review style '{}' has inconsistent emission semantics",
                    dependency.id
                ));
            }
        };
        let expected = ViewportStyle {
            color,
            surface_mode: dependency.surface_mode,
            opacity: dependency.opacity,
            emission,
        };
        let actual = content.styles.get(&dependency.id).ok_or_else(|| {
            format!(
                "review viewport does not resolve reported style '{}'",
                dependency.id
            )
        })?;
        if *actual != expected {
            return Err(format!(
                "review viewport resolved style '{}' differs from the report",
                dependency.id
            ));
        }
    }
    Ok(())
}

const fn review_part_label(part: ObjectPart) -> &'static str {
    match part {
        ObjectPart::Plant(PlantPart::Root) => "plant/root",
        ObjectPart::Plant(PlantPart::Trunk) => "plant/trunk",
        ObjectPart::Plant(PlantPart::Branch) => "plant/branch",
        ObjectPart::Plant(PlantPart::Foliage) => "plant/foliage",
        ObjectPart::Plant(PlantPart::Accent) => "plant/accent",
        ObjectPart::Effect(EffectPart::Core) => "effect/core",
        ObjectPart::Effect(EffectPart::Trail) => "effect/trail",
        ObjectPart::Effect(EffectPart::Accent) => "effect/accent",
        ObjectPart::Prop(PropPart::Structure) => "prop/structure",
        ObjectPart::Prop(PropPart::Detail) => "prop/detail",
    }
}

fn validate_presentation(
    content: &ViewportContent,
    presentation: ReviewPresentation,
    ordinal: u8,
    file_name: &str,
) -> Result<(), String> {
    let expected = match presentation {
        ReviewPresentation::Authored => (false, false, false),
        ReviewPresentation::SemanticParts => (true, false, false),
        ReviewPresentation::BlockerCanopy => (false, true, true),
    };
    let actual = (
        content.show_semantic_overlay,
        content.show_blocker_overlay,
        content.show_canopy_overlay,
    );
    if actual != expected {
        return Err(format!(
            "review frame {ordinal} ({file_name}) overlay flags {actual:?} do not match \
             {presentation:?} presentation {expected:?}"
        ));
    }
    Ok(())
}

fn frame_ordinal(index: usize) -> u8 {
    u8::try_from(index.saturating_add(1)).unwrap_or(u8::MAX)
}

fn review_frame_total() -> u8 {
    u8::try_from(REVIEW_FRAME_COUNT).unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use hex_assets::{
        ArtPalette, ConnectivityPolicy, LocalAxialCoord, LocalVoxelCoord, ObjectAssetId,
        ObjectBlueprint, ObjectBounds, ObjectCategory, ObjectPart, ObjectPlacement, PaletteSwatch,
        PlantPart, SrgbColor, SwatchId, VoxelStyle, VoxelStyleCatalog, VoxelStyleId,
        VoxelSurfaceMode, OBJECT_BLUEPRINT_SCHEMA_VERSION,
    };

    use super::*;
    use crate::review::ReviewReport;
    use crate::viewport::ViewportStyle;

    static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn fixture() -> (PathBuf, ReviewReport, Vec<ViewportContent>, VoxelStyleId) {
        let swatch_id = SwatchId::new("plant/leaf").expect("fixture swatch id should be valid");
        let swatch = PaletteSwatch::new(
            "Leaf".to_owned(),
            SrgbColor::new(0.2, 0.7, 0.3).expect("fixture colour should be valid"),
            BTreeSet::from(["plant".to_owned()]),
        )
        .expect("fixture swatch should be valid");
        let palette = ArtPalette::new(BTreeMap::from([(swatch_id.clone(), swatch.clone())]))
            .expect("fixture palette should be valid");
        let style_id = VoxelStyleId::new("plant/leaf").expect("fixture style id should be valid");
        let style = VoxelStyle::new(
            "Leaf".to_owned(),
            swatch_id,
            VoxelSurfaceMode::Opaque,
            1.0,
            None,
        )
        .expect("fixture style should be valid");
        let styles = VoxelStyleCatalog::new(BTreeMap::from([(style_id.clone(), style)]))
            .expect("fixture styles should be valid");
        let root = LocalVoxelCoord::new(0, 0, 0);
        let object = ObjectBlueprint {
            schema_version: OBJECT_BLUEPRINT_SCHEMA_VERSION,
            id: ObjectAssetId::new("plant/review-tree").expect("fixture object id should be valid"),
            display_name: "Review Tree".to_owned(),
            category: ObjectCategory::Plant,
            origin: root,
            bounds: ObjectBounds {
                radius: 2,
                min_level: 0,
                height: 4,
            },
            connectivity: ConnectivityPolicy::Grounded,
            blocker_footprint: vec![LocalAxialCoord::new(0, 0)],
            canopy_occluders: vec![LocalVoxelCoord::new(0, 0, 1)],
            placements: vec![
                ObjectPlacement {
                    position: root,
                    style: style_id.clone(),
                    part: ObjectPart::Plant(PlantPart::Root),
                },
                ObjectPlacement {
                    position: LocalVoxelCoord::new(0, 0, 1),
                    style: style_id.clone(),
                    part: ObjectPart::Plant(PlantPart::Foliage),
                },
            ],
        };
        object
            .validate(&styles)
            .expect("fixture object should be valid");
        let sequence = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test clock should follow the Unix epoch")
            .as_nanos();
        let local_sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "hex-editor-review-capture-{}-{sequence}-{local_sequence}",
            std::process::id(),
        ));
        let mesh_path = root.join("assets").join(HEX_MESH_ASSET_PATH);
        fs::create_dir_all(
            mesh_path
                .parent()
                .expect("fixture renderer mesh should have a parent"),
        )
        .expect("fixture renderer directory should be created");
        fs::write(&mesh_path, b"fixture-hex-mesh")
            .expect("fixture renderer mesh should be written");
        let mesh_revision =
            current_file_revision(&root, &Path::new("assets").join(HEX_MESH_ASSET_PATH))
                .expect("fixture renderer mesh should be readable");
        let report = ReviewReport::new(&object, &styles, &palette, mesh_revision)
            .expect("fixture report should build");
        let resolved_style = ViewportStyle {
            color: swatch.color(),
            surface_mode: VoxelSurfaceMode::Opaque,
            opacity: 1.0,
            emission: None,
        };
        let voxels = object
            .placements
            .iter()
            .map(|placement| RenderedVoxel {
                position: placement.position,
                style: placement.style.clone(),
            })
            .collect::<Vec<_>>();
        let semantic_parts: BTreeMap<LocalVoxelCoord, ObjectPart> = object
            .placements
            .iter()
            .map(|placement| (placement.position, placement.part))
            .collect();
        let mut contents = Vec::with_capacity(REVIEW_FRAME_COUNT);
        for spec in REVIEW_FRAME_SPECS {
            let (semantics, blockers, canopy) = match spec.presentation {
                ReviewPresentation::Authored => (false, false, false),
                ReviewPresentation::SemanticParts => (true, false, false),
                ReviewPresentation::BlockerCanopy => (false, true, true),
            };
            contents.push(ViewportContent {
                voxels: voxels.clone(),
                styles: BTreeMap::from([(style_id.clone(), resolved_style)]),
                grid_radius: object.bounds.radius,
                active_level: 0,
                isolate_active_level: false,
                show_grid: false,
                selected_cells: BTreeSet::new(),
                semantic_parts: semantic_parts.clone(),
                blocker_columns: object.blocker_footprint.iter().copied().collect(),
                canopy_cells: object.canopy_occluders.iter().copied().collect(),
                show_semantic_overlay: semantics,
                show_blocker_overlay: blockers,
                show_canopy_overlay: canopy,
            });
        }
        (root, report, contents, style_id)
    }

    #[test]
    fn validated_request_accepts_exact_ordered_presentations() {
        let (root, report, contents, _style_id) = fixture();
        let request = ReviewCaptureRequest::new(
            root.clone(),
            ProjectRevisionSet::default(),
            report,
            contents,
        );
        assert!(request.is_ok());
        fs::remove_dir_all(root).expect("fixture repository should be removable");
    }

    #[test]
    fn request_rejects_wrong_frame_count_without_touching_viewport() {
        let (root, report, mut contents, _style_id) = fixture();
        drop(contents.pop());
        let error = ReviewCaptureRequest::new(
            root.clone(),
            ProjectRevisionSet::default(),
            report,
            contents,
        )
        .expect_err("missing review frame must be rejected");
        assert!(error.contains("9 viewport snapshots"));
        fs::remove_dir_all(root).expect("fixture repository should be removable");
    }

    #[test]
    fn request_rejects_editor_state_and_mismatched_overlay() {
        let (root, report, mut contents, _style_id) = fixture();
        contents
            .first_mut()
            .expect("fixture should include the first frame")
            .show_grid = true;
        let error = ReviewCaptureRequest::new(
            root.clone(),
            ProjectRevisionSet::default(),
            report.clone(),
            contents.clone(),
        )
        .expect_err("authoring grid must be rejected");
        assert!(error.contains("must hide the authoring grid"));

        let first = contents
            .first_mut()
            .expect("fixture should include the first frame");
        first.show_grid = false;
        first.show_semantic_overlay = true;
        let error = ReviewCaptureRequest::new(
            root.clone(),
            ProjectRevisionSet::default(),
            report,
            contents,
        )
        .expect_err("authored frame cannot carry a semantic overlay");
        assert!(error.contains("do not match Authored"));
        fs::remove_dir_all(root).expect("fixture repository should be removable");
    }

    #[test]
    fn request_rejects_missing_style_and_overlapping_cells() {
        let (root, report, mut contents, style_id) = fixture();
        let first = contents
            .first_mut()
            .expect("fixture should include the first frame");
        assert!(first.styles.remove(&style_id).is_some());
        let error = ReviewCaptureRequest::new(
            root.clone(),
            ProjectRevisionSet::default(),
            report.clone(),
            contents.clone(),
        )
        .expect_err("missing style must be rejected");
        assert!(error.contains("references missing style"));

        let (other_root, _other_report, mut valid_contents, _other_style_id) = fixture();
        let duplicate = valid_contents
            .first()
            .and_then(|content| content.voxels.first())
            .cloned()
            .expect("fixture should contain a voxel");
        for content in &mut valid_contents {
            content.voxels.push(duplicate.clone());
            content.voxels.sort_by(voxel_order);
        }
        let error = ReviewCaptureRequest::new(
            root.clone(),
            ProjectRevisionSet::default(),
            report,
            valid_contents,
        )
        .expect_err("overlapping placements must be rejected");
        assert!(error.contains("overlapping voxel placements"));
        fs::remove_dir_all(root).expect("fixture repository should be removable");
        fs::remove_dir_all(other_root).expect("second fixture repository should be removable");
    }

    #[test]
    fn request_rejects_cross_frame_and_report_snapshot_mismatches() {
        let (root, report, mut contents, _style_id) = fixture();
        contents
            .get_mut(1)
            .expect("fixture should include the second frame")
            .voxels
            .pop();
        let error = ReviewCaptureRequest::new(
            root.clone(),
            ProjectRevisionSet::default(),
            report.clone(),
            contents,
        )
        .expect_err("cross-frame geometry mismatch must be rejected");
        assert!(error.contains("canonical object geometry"));

        let (other_root, _other_report, mut contents, _other_style_id) = fixture();
        for content in &mut contents {
            content.blocker_columns.clear();
        }
        let error = ReviewCaptureRequest::new(
            root.clone(),
            ProjectRevisionSet::default(),
            report,
            contents,
        )
        .expect_err("report-mask mismatch must be rejected");
        assert!(error.contains("blocker mask differs"));
        fs::remove_dir_all(root).expect("fixture repository should be removable");
        fs::remove_dir_all(other_root).expect("second fixture repository should be removable");
    }

    #[test]
    fn request_rejects_resolved_style_semantics_that_differ_from_report() {
        let (root, report, mut contents, style_id) = fixture();
        let conflicting_color =
            SrgbColor::new(0.95, 0.1, 0.2).expect("conflicting color should be valid");
        for content in &mut contents {
            let style = content
                .styles
                .get_mut(&style_id)
                .expect("fixture should resolve its used style");
            style.color = conflicting_color;
        }
        let error = ReviewCaptureRequest::new(
            root.clone(),
            ProjectRevisionSet::default(),
            report,
            contents,
        )
        .expect_err("rendering semantics must match the report dependencies");
        assert!(error.contains("resolved style"));
        fs::remove_dir_all(root).expect("fixture repository should be removable");
    }

    #[test]
    fn request_creation_rejects_a_stale_tracked_source_snapshot() {
        let (root, report, contents, _style_id) = fixture();
        let art_root = root.join("assets/art");
        fs::create_dir_all(&art_root).expect("fixture art root should be created");
        let palette_path = art_root.join("palette.ron");
        fs::write(&palette_path, b"version-a").expect("fixture source should be written");
        let expected =
            current_project_revisions(&root).expect("initial tracked source revisions should scan");
        fs::write(&palette_path, b"version-b").expect("fixture source should be modified");

        let error = ReviewCaptureRequest::new(root.clone(), expected, report, contents)
            .expect_err("a stale source snapshot must reject request creation");
        assert!(error.contains("palette.ron (modified)"));
        assert!(error.contains("reload the project"));
        fs::remove_dir_all(root).expect("fixture repository should be removable");
    }

    #[test]
    fn request_creation_rejects_a_stale_renderer_mesh() {
        let (root, report, contents, _style_id) = fixture();
        let mesh_path = root.join("assets").join(HEX_MESH_ASSET_PATH);
        fs::write(&mesh_path, b"changed-hex-mesh")
            .expect("fixture renderer mesh should be modified");

        let error = ReviewCaptureRequest::new(
            root.clone(),
            ProjectRevisionSet::default(),
            report,
            contents,
        )
        .expect_err("a stale renderer mesh must reject request creation");
        assert!(error.contains("assets/meshes/hex.glb (modified)"));
        assert!(error.contains("retry the review export"));
        fs::remove_dir_all(root).expect("fixture repository should be removable");
    }

    #[test]
    fn tracked_source_change_during_capture_blocks_publication() {
        let (root, report, contents, _style_id) = fixture();
        let art_root = root.join("assets/art");
        fs::create_dir_all(&art_root).expect("fixture art root should be created");
        let palette_path = art_root.join("palette.ron");
        fs::write(&palette_path, b"version-a").expect("fixture source should be written");
        let expected =
            current_project_revisions(&root).expect("initial tracked source revisions should scan");
        let request = ReviewCaptureRequest::new(root.clone(), expected, report, contents)
            .expect("current source revisions should allow capture");

        fs::write(&palette_path, b"version-b").expect("fixture source should change mid-capture");
        let error = publish_review_pack_if_sources_current(
            &root,
            &request.expected_revisions,
            &request.report,
            &[],
        )
        .expect_err("a source change during capture must block publication");

        assert!(error.contains("while the review capture was running"));
        assert!(error.contains("palette.ron (modified)"));
        assert!(error.contains("no review pack was published"));
        assert!(!root.join(".context/asset-workshop/reviews").exists());
        fs::remove_dir_all(root).expect("fixture repository should be removable");
    }

    #[test]
    fn tracked_source_change_during_staging_blocks_atomic_publication() {
        let (root, report, contents, _style_id) = fixture();
        let art_root = root.join("assets/art");
        fs::create_dir_all(&art_root).expect("fixture art root should be created");
        let palette_path = art_root.join("palette.ron");
        fs::write(&palette_path, b"version-a").expect("fixture source should be written");
        let expected =
            current_project_revisions(&root).expect("initial tracked source revisions should scan");
        let request = ReviewCaptureRequest::new(root.clone(), expected, report, contents)
            .expect("current source revisions should allow capture");
        let frames = review_frame_set();

        let error = publish_review_pack_if_sources_current_with_hook(
            &root,
            &request.expected_revisions,
            &request.report,
            &frames,
            || {
                fs::write(&palette_path, b"version-b").map_err(|error| {
                    format!("fixture source should change during staging: {error}")
                })
            },
        )
        .expect_err("a source change during staging must block the final rename");

        assert!(error.contains("while the review capture was running"));
        assert!(error.contains("palette.ron (modified)"));
        assert!(error.contains("no review pack was published"));
        let final_path = crate::review::review_pack_path(&root, &request.report)
            .expect("fixture review path should resolve");
        assert!(!final_path.exists());
        let review_parent = final_path
            .parent()
            .expect("fixture review path should have a parent");
        assert_eq!(
            fs::read_dir(review_parent)
                .expect("review parent should remain readable")
                .count(),
            0,
            "failed publication must remove its staging directory"
        );
        fs::remove_dir_all(root).expect("fixture repository should be removable");
    }

    fn review_frame_set() -> Vec<RgbaImage> {
        (0..REVIEW_FRAME_COUNT)
            .map(|index| {
                let index =
                    u8::try_from(index).expect("review frame index should fit in a single byte");
                let mut frame = RgbaImage::from_pixel(
                    REVIEW_FRAME_WIDTH,
                    REVIEW_FRAME_HEIGHT,
                    image::Rgba(REVIEW_CLEAR_RGBA),
                );
                let accent = image::Rgba([
                    56_u8.saturating_add(index.saturating_mul(12)),
                    110_u8.saturating_add(index.saturating_mul(7)),
                    170_u8.saturating_sub(index.saturating_mul(6)),
                    255,
                ]);
                let center_x = REVIEW_FRAME_WIDTH / 2;
                let center_y = REVIEW_FRAME_HEIGHT / 2;
                for y in center_y - 48..center_y + 48 {
                    for x in center_x - 48..center_x + 48 {
                        frame.put_pixel(x, y, accent);
                    }
                }
                frame
            })
            .collect()
    }

    #[test]
    fn gpu_readbacks_require_two_identical_valid_frames() {
        let first = RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]));
        let second = RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]));
        let candidate = match assess_readback(None, Ok(first), 1) {
            ReadbackDecision::Retry(Some(candidate)) => candidate,
            _ => panic!("the first valid readback should become a verification candidate"),
        };
        assert!(matches!(
            assess_readback(Some(candidate), Ok(second), 2),
            ReadbackDecision::Accept(_)
        ));

        let unstable = RgbaImage::from_pixel(2, 2, image::Rgba([40, 50, 60, 255]));
        assert!(matches!(
            assess_readback(
                Some(RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]))),
                Ok(unstable),
                2
            ),
            ReadbackDecision::Retry(Some(_))
        ));
        assert!(matches!(
            assess_readback(None, Err("blank".to_owned()), MAX_READBACK_ATTEMPTS),
            ReadbackDecision::Fail(_)
        ));
    }

    #[test]
    fn frame_progress_contract_is_one_based_and_fixed() {
        assert_eq!(frame_ordinal(0), 1);
        assert_eq!(frame_ordinal(REVIEW_FRAME_COUNT - 1), 10);
        assert_eq!(review_frame_total(), 10);
    }
}
