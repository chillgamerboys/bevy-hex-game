//! Deterministic review-pack contracts, images, reports, and publication.
//!
//! Renderer orchestration deliberately lives outside this module. The core accepts
//! immutable, validated asset snapshots and captured RGBA frames, then produces a
//! byte-stable report and publishes a complete directory in one visibility step.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use bevy::prelude::{Image as BevyImage, Transform, Vec3};
use hex_assets::{
    ArtPalette, ConnectivityPolicy, LocalAxialCoord, LocalVoxelCoord, ObjectAssetId,
    ObjectBlueprint, ObjectBounds, ObjectCategory, ObjectPart, PaletteSwatch, SrgbColor, SwatchId,
    VoxelEmission, VoxelStyle, VoxelStyleCatalog, VoxelStyleId, VoxelSurfaceMode,
};
use image::codecs::png::{CompressionType, FilterType as PngFilterType, PngEncoder};
use image::imageops::{self, FilterType as ResizeFilter};
use image::{ExtendedColorType, GenericImage, ImageEncoder, ImageFormat, Rgba, RgbaImage};
use serde::{Deserialize, Serialize};

use crate::viewport::frame_object_positions;

/// Version of the review layout, camera, presentation, and report contract.
pub const REVIEW_FORMAT_VERSION: u16 = 1;
/// Width of every source review frame.
pub const REVIEW_FRAME_WIDTH: u32 = 1_024;
/// Height of every source review frame.
pub const REVIEW_FRAME_HEIGHT: u32 = 1_024;
/// Number of authored and diagnostic frames in one complete pack.
pub const REVIEW_FRAME_COUNT: usize = 10;
/// Filename of the deterministic report.
pub const REVIEW_REPORT_FILE: &str = "report.ron";
/// Filename of the derived contact sheet.
pub const REVIEW_CONTACT_SHEET_FILE: &str = "contact-sheet.png";
/// Opaque sRGB clear color expected from the neutral review camera target.
pub const REVIEW_CLEAR_RGBA: [u8; 4] = [9, 10, 11, 255];
/// Width and height of one contact-sheet thumbnail.
pub const CONTACT_THUMB_SIZE: u32 = 256;
/// Number of thumbnail columns in the contact sheet.
pub const CONTACT_SHEET_COLUMNS: u32 = 4;
/// Number of thumbnail rows in the contact sheet.
pub const CONTACT_SHEET_ROWS: u32 = 3;
/// Empty pixels around and between contact-sheet thumbnails.
pub const CONTACT_SHEET_GUTTER: u32 = 12;
/// Exact output width of the contact sheet.
pub const CONTACT_SHEET_WIDTH: u32 =
    CONTACT_THUMB_SIZE * CONTACT_SHEET_COLUMNS + CONTACT_SHEET_GUTTER * (CONTACT_SHEET_COLUMNS + 1);
/// Exact output height of the contact sheet.
pub const CONTACT_SHEET_HEIGHT: u32 =
    CONTACT_THUMB_SIZE * CONTACT_SHEET_ROWS + CONTACT_SHEET_GUTTER * (CONTACT_SHEET_ROWS + 1);

const REVIEW_OUTPUT_PATH: &str = ".context/asset-workshop/reviews";
const CONTACT_BACKGROUND_RGBA: Rgba<u8> = Rgba([24, 26, 30, 255]);
const LABEL_BACKGROUND_RGBA: Rgba<u8> = Rgba([4, 5, 6, 255]);
const LABEL_FOREGROUND_RGBA: Rgba<u8> = Rgba([244, 246, 248, 255]);
const FRAME_VARIATION_THRESHOLD: u8 = 8;
const MIN_VARIANT_PIXELS: u64 = 32;
const TURN_PITCH_RADIANS: f32 = 0.35;
const PERSPECTIVE_PITCH_RADIANS: f32 = 0.62;
const PERSPECTIVE_YAW_RADIANS: f32 = std::f32::consts::FRAC_PI_4;
const STAGING_ATTEMPTS: u64 = 128;
const REPORT_INDENT: &str = "    ";

static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// How viewport content is presented in one review frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewPresentation {
    /// Palette-authored materials under the neutral preview rig.
    Authored,
    /// Semantic-part rings drawn over palette-authored voxel bodies.
    SemanticParts,
    /// Exact blocker columns and canopy cells emphasized in a top view.
    BlockerCanopy,
}

/// One fixed review-camera orientation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ReviewCameraView {
    /// Orbit around the common focus using authored angles in radians.
    Orbit {
        /// Rotation around world Y.
        yaw_radians: f32,
        /// Angle above the horizontal plane.
        pitch_radians: f32,
    },
    /// Look straight down world Y with world negative Z as camera up.
    Top,
}

/// Immutable description of one required source frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReviewFrameSpec {
    /// One-based stable order in the review pack.
    pub ordinal: u8,
    /// Stable PNG filename.
    pub file_name: &'static str,
    /// Fixed camera orientation.
    pub camera: ReviewCameraView,
    /// Required viewport presentation.
    pub presentation: ReviewPresentation,
}

const fn orbit(yaw_radians: f32, pitch_radians: f32) -> ReviewCameraView {
    ReviewCameraView::Orbit {
        yaw_radians,
        pitch_radians,
    }
}

/// Ordered frame contract: perspective, top, six turns, and two overlays.
pub const REVIEW_FRAME_SPECS: [ReviewFrameSpec; REVIEW_FRAME_COUNT] = [
    ReviewFrameSpec {
        ordinal: 1,
        file_name: "01-perspective.png",
        camera: orbit(PERSPECTIVE_YAW_RADIANS, PERSPECTIVE_PITCH_RADIANS),
        presentation: ReviewPresentation::Authored,
    },
    ReviewFrameSpec {
        ordinal: 2,
        file_name: "02-top.png",
        camera: ReviewCameraView::Top,
        presentation: ReviewPresentation::Authored,
    },
    ReviewFrameSpec {
        ordinal: 3,
        file_name: "03-turn-000.png",
        camera: orbit(0.0, TURN_PITCH_RADIANS),
        presentation: ReviewPresentation::Authored,
    },
    ReviewFrameSpec {
        ordinal: 4,
        file_name: "04-turn-060.png",
        camera: orbit(std::f32::consts::FRAC_PI_3, TURN_PITCH_RADIANS),
        presentation: ReviewPresentation::Authored,
    },
    ReviewFrameSpec {
        ordinal: 5,
        file_name: "05-turn-120.png",
        camera: orbit(2.0 * std::f32::consts::FRAC_PI_3, TURN_PITCH_RADIANS),
        presentation: ReviewPresentation::Authored,
    },
    ReviewFrameSpec {
        ordinal: 6,
        file_name: "06-turn-180.png",
        camera: orbit(std::f32::consts::PI, TURN_PITCH_RADIANS),
        presentation: ReviewPresentation::Authored,
    },
    ReviewFrameSpec {
        ordinal: 7,
        file_name: "07-turn-240.png",
        camera: orbit(4.0 * std::f32::consts::FRAC_PI_3, TURN_PITCH_RADIANS),
        presentation: ReviewPresentation::Authored,
    },
    ReviewFrameSpec {
        ordinal: 8,
        file_name: "08-turn-300.png",
        camera: orbit(5.0 * std::f32::consts::FRAC_PI_3, TURN_PITCH_RADIANS),
        presentation: ReviewPresentation::Authored,
    },
    ReviewFrameSpec {
        ordinal: 9,
        file_name: "09-semantic.png",
        camera: orbit(PERSPECTIVE_YAW_RADIANS, PERSPECTIVE_PITCH_RADIANS),
        presentation: ReviewPresentation::SemanticParts,
    },
    ReviewFrameSpec {
        ordinal: 10,
        file_name: "10-blocker-canopy.png",
        camera: ReviewCameraView::Top,
        presentation: ReviewPresentation::BlockerCanopy,
    },
];

/// Common geometry framing shared by every frame in one pack.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ReviewFraming {
    /// World-space orbit focus.
    pub focus: [f32; 3],
    /// World-space camera distance.
    pub radius: f32,
}

impl ReviewFraming {
    /// Derives the same stable framing used by the interactive object viewport.
    pub fn from_object(object: &ObjectBlueprint) -> Result<Self, ReviewError> {
        object
            .validate_intrinsic()
            .map_err(|error| ReviewError::new("frame review object", None, error))?;
        let positions = object.placements.iter().map(|placement| placement.position);
        let Some((focus, radius)) = frame_object_positions(positions) else {
            return Err(ReviewError::new(
                "frame review object",
                None,
                "validated object has no occupied cells",
            ));
        };
        let framing = Self {
            focus: focus.to_array(),
            radius,
        };
        framing.validate()?;
        Ok(framing)
    }

    fn validate(self) -> Result<(), ReviewError> {
        let focus = Vec3::from_array(self.focus);
        if !focus.is_finite() || !self.radius.is_finite() || self.radius <= 0.0 {
            return Err(ReviewError::new(
                "validate review framing",
                None,
                "focus must be finite and radius must be finite and positive",
            ));
        }
        Ok(())
    }

    /// Resolves one fixed view into an exact eye/focus/up pose.
    pub fn camera_pose(self, view: ReviewCameraView) -> Result<ReviewCameraPose, ReviewError> {
        self.validate()?;
        let focus = Vec3::from_array(self.focus);
        let (eye, up) = match view {
            ReviewCameraView::Orbit {
                yaw_radians,
                pitch_radians,
            } => {
                if !yaw_radians.is_finite()
                    || !pitch_radians.is_finite()
                    || !(0.0..=std::f32::consts::FRAC_PI_2).contains(&pitch_radians)
                {
                    return Err(ReviewError::new(
                        "resolve review camera",
                        None,
                        "orbit angles must be finite and pitch must be within 0..=pi/2",
                    ));
                }
                let horizontal = pitch_radians.cos();
                let offset = Vec3::new(
                    yaw_radians.sin() * horizontal,
                    pitch_radians.sin(),
                    yaw_radians.cos() * horizontal,
                ) * self.radius;
                (focus + offset, Vec3::Y)
            }
            ReviewCameraView::Top => (focus + Vec3::Y * self.radius, Vec3::NEG_Z),
        };
        Ok(ReviewCameraPose {
            eye: eye.to_array(),
            focus: self.focus,
            up: up.to_array(),
        })
    }
}

/// Exact resolved camera pose for renderer integration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReviewCameraPose {
    /// World-space camera position.
    pub eye: [f32; 3],
    /// World-space point the camera observes.
    pub focus: [f32; 3],
    /// World-space camera up vector.
    pub up: [f32; 3],
}

impl ReviewCameraPose {
    /// Creates the Bevy transform represented by this reviewed pose.
    #[must_use]
    pub fn transform(self) -> Transform {
        Transform::from_translation(Vec3::from_array(self.eye))
            .looking_at(Vec3::from_array(self.focus), Vec3::from_array(self.up))
    }
}

/// Serializable entry for one frame in the deterministic report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewFrameReport {
    /// One-based stable order.
    pub ordinal: u8,
    /// Stable PNG filename.
    pub file_name: String,
    /// Exact camera orientation.
    pub camera: ReviewCameraView,
    /// Exact presentation mode.
    pub presentation: ReviewPresentation,
}

impl From<ReviewFrameSpec> for ReviewFrameReport {
    fn from(spec: ReviewFrameSpec) -> Self {
        Self {
            ordinal: spec.ordinal,
            file_name: spec.file_name.to_owned(),
            camera: spec.camera,
            presentation: spec.presentation,
        }
    }
}

/// One exact style dependency used by the reviewed object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewStyleDependency {
    /// Stable style id.
    pub id: VoxelStyleId,
    /// Human-facing style name.
    pub display_name: String,
    /// Number of reviewed placements using this style.
    pub placements: u32,
    /// Base palette reference.
    pub base_swatch: SwatchId,
    /// Renderer treatment used by every placement of this style.
    pub surface_mode: VoxelSurfaceMode,
    /// Exact authored surface opacity.
    pub opacity: f32,
    /// Optional emitted palette reference.
    pub emission_swatch: Option<SwatchId>,
    /// Optional finite nonnegative emission strength.
    pub emission_strength: Option<f32>,
}

/// One exact palette dependency used transitively by reviewed styles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewSwatchDependency {
    /// Stable swatch id.
    pub id: SwatchId,
    /// Human-facing swatch name.
    pub display_name: String,
    /// Exact authored sRGB color.
    pub color: SrgbColor,
    /// Sorted ownership and search tags.
    pub tags: Vec<String>,
}

/// Validation result embedded in every successfully built report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewValidation {
    /// The complete palette-style-object dependency graph was valid.
    Valid,
}

/// Deterministic semantic manifest for one complete object review.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewReport {
    /// Review contract version.
    review_format_version: u16,
    /// Immutable object identity.
    object_id: ObjectAssetId,
    /// Editable display name captured at review time.
    display_name: String,
    /// Broad object category.
    category: ObjectCategory,
    /// Authored connectivity contract.
    connectivity: ConnectivityPolicy,
    /// Authored object canvas.
    bounds: ObjectBounds,
    /// Exact authored object origin.
    origin: LocalVoxelCoord,
    /// Number of occupied cells.
    occupied_cells: u32,
    /// Stable semantic-part label to occupied-cell count.
    part_counts: BTreeMap<String, u32>,
    /// Exact blocker mask in sorted order.
    blocker_footprint: Vec<LocalAxialCoord>,
    /// Exact canopy mask in sorted order.
    canopy_occluders: Vec<LocalVoxelCoord>,
    /// Used style dependencies in stable-id order.
    style_dependencies: Vec<ReviewStyleDependency>,
    /// Used palette dependencies in stable-id order.
    swatch_dependencies: Vec<ReviewSwatchDependency>,
    /// Independent semantic object fingerprint.
    object_fingerprint: u64,
    /// Independent semantic style-catalog fingerprint.
    style_catalog_fingerprint: u64,
    /// Independent semantic palette fingerprint.
    palette_fingerprint: u64,
    /// Versioned composite directory identity.
    review_fingerprint: String,
    /// Common focus and radius used by every view.
    framing: ReviewFraming,
    /// Source frame dimensions.
    frame_size: [u32; 2],
    /// Ordered required captures.
    frames: Vec<ReviewFrameReport>,
    /// Result of complete dependency-graph validation.
    validation: ReviewValidation,
    /// Constructor provenance is deliberately absent from serialized reports.
    ///
    /// A parsed report remains useful as a read-only artifact, but only a report
    /// built from a validated live dependency graph may authorize capture or
    /// publication.
    #[serde(skip)]
    trusted_for_publication: bool,
}

impl ReviewReport {
    /// Validates an asset snapshot and derives its deterministic review manifest.
    pub fn new(
        object: &ObjectBlueprint,
        styles: &VoxelStyleCatalog,
        palette: &ArtPalette,
    ) -> Result<Self, ReviewError> {
        palette.validate().map_err(|error| {
            ReviewError::new("validate review palette", None, error.to_string())
        })?;
        styles
            .validate(palette)
            .map_err(|error| ReviewError::new("validate review styles", None, error.to_string()))?;
        object
            .validate(styles)
            .map_err(|error| ReviewError::new("validate review object", None, error))?;

        let mut style_counts = BTreeMap::<VoxelStyleId, u32>::new();
        let mut part_counts = BTreeMap::<String, u32>::new();
        for placement in &object.placements {
            increment_count(
                style_counts.entry(placement.style.clone()).or_default(),
                "style placement count",
            )?;
            increment_count(
                part_counts
                    .entry(part_label(placement.part).to_owned())
                    .or_default(),
                "part placement count",
            )?;
        }

        let mut swatch_ids = BTreeSet::new();
        let mut style_dependencies = Vec::with_capacity(style_counts.len());
        for (id, placements) in style_counts {
            let Some(style) = styles.get(&id) else {
                return Err(ReviewError::new(
                    "build review dependencies",
                    None,
                    format!("object references missing style '{id}'"),
                ));
            };
            swatch_ids.insert(style.base_swatch().clone());
            let emission_swatch = style.emission().map(|emission| emission.swatch().clone());
            let emission_strength = style.emission().map(VoxelEmission::strength);
            if let Some(emission) = &emission_swatch {
                swatch_ids.insert(emission.clone());
            }
            style_dependencies.push(ReviewStyleDependency {
                id,
                display_name: style.display_name().to_owned(),
                placements,
                base_swatch: style.base_swatch().clone(),
                surface_mode: style.surface_mode(),
                opacity: style.opacity(),
                emission_swatch,
                emission_strength,
            });
        }

        let mut swatch_dependencies = Vec::with_capacity(swatch_ids.len());
        for id in swatch_ids {
            let Some(swatch) = palette.get(&id) else {
                return Err(ReviewError::new(
                    "build review dependencies",
                    None,
                    format!("reviewed style references missing swatch '{id}'"),
                ));
            };
            swatch_dependencies.push(ReviewSwatchDependency {
                id,
                display_name: swatch.display_name().to_owned(),
                color: swatch.color(),
                tags: swatch.tags().iter().cloned().collect(),
            });
        }

        let object_fingerprint = object
            .semantic_fingerprint()
            .map_err(|error| ReviewError::new("fingerprint review object", None, error))?;
        let style_catalog_fingerprint = styles.semantic_fingerprint();
        let palette_fingerprint = palette.semantic_fingerprint();
        let mut blocker_footprint = object.blocker_footprint.clone();
        blocker_footprint.sort_unstable();
        let mut canopy_occluders = object.canopy_occluders.clone();
        canopy_occluders.sort_unstable();
        let occupied_cells = u32::try_from(object.placements.len()).map_err(|error| {
            ReviewError::new(
                "build review report",
                None,
                format!("occupied-cell count cannot be represented: {error}"),
            )
        })?;

        let report = Self {
            review_format_version: REVIEW_FORMAT_VERSION,
            object_id: object.id.clone(),
            display_name: object.display_name.clone(),
            category: object.category,
            connectivity: object.connectivity,
            bounds: object.bounds,
            origin: object.origin,
            occupied_cells,
            part_counts,
            blocker_footprint,
            canopy_occluders,
            style_dependencies,
            swatch_dependencies,
            object_fingerprint,
            style_catalog_fingerprint,
            palette_fingerprint,
            review_fingerprint: composite_fingerprint(
                object_fingerprint,
                style_catalog_fingerprint,
                palette_fingerprint,
            ),
            framing: ReviewFraming::from_object(object)?,
            frame_size: [REVIEW_FRAME_WIDTH, REVIEW_FRAME_HEIGHT],
            frames: REVIEW_FRAME_SPECS
                .into_iter()
                .map(ReviewFrameReport::from)
                .collect(),
            validation: ReviewValidation::Valid,
            trusted_for_publication: true,
        };
        report.validate_contract()?;
        Ok(report)
    }

    pub(crate) fn object_id(&self) -> &ObjectAssetId {
        &self.object_id
    }

    pub(crate) fn display_name(&self) -> &str {
        &self.display_name
    }

    pub(crate) const fn category(&self) -> ObjectCategory {
        self.category
    }

    pub(crate) const fn connectivity(&self) -> ConnectivityPolicy {
        self.connectivity
    }

    pub(crate) const fn bounds(&self) -> ObjectBounds {
        self.bounds
    }

    pub(crate) const fn origin(&self) -> LocalVoxelCoord {
        self.origin
    }

    pub(crate) const fn occupied_cells(&self) -> u32 {
        self.occupied_cells
    }

    pub(crate) fn part_counts(&self) -> &BTreeMap<String, u32> {
        &self.part_counts
    }

    pub(crate) fn blocker_footprint(&self) -> &[LocalAxialCoord] {
        &self.blocker_footprint
    }

    pub(crate) fn canopy_occluders(&self) -> &[LocalVoxelCoord] {
        &self.canopy_occluders
    }

    pub(crate) fn style_dependencies(&self) -> &[ReviewStyleDependency] {
        &self.style_dependencies
    }

    pub(crate) fn swatch_dependencies(&self) -> &[ReviewSwatchDependency] {
        &self.swatch_dependencies
    }

    pub(crate) const fn object_fingerprint(&self) -> u64 {
        self.object_fingerprint
    }

    pub(crate) const fn framing(&self) -> ReviewFraming {
        self.framing
    }

    pub(crate) fn validate_for_publication(&self) -> Result<(), ReviewError> {
        self.validate_contract()?;
        if !self.trusted_for_publication {
            return Err(ReviewError::new(
                "validate review report",
                None,
                "parsed reports are read-only; rebuild the report from its validated asset graph",
            ));
        }
        Ok(())
    }

    /// Serializes this report as canonically formatted, newline-terminated RON.
    pub fn to_ron_bytes(&self) -> Result<Vec<u8>, ReviewError> {
        self.validate_contract()?;
        let config = ron::ser::PrettyConfig::default()
            .new_line("\n")
            .indentor(REPORT_INDENT);
        let mut source = ron::ser::to_string_pretty(self, config).map_err(|error| {
            ReviewError::new("serialize review report", None, error.to_string())
        })?;
        source.push('\n');
        Ok(source.into_bytes())
    }

    fn validate_contract(&self) -> Result<(), ReviewError> {
        if self.review_format_version != REVIEW_FORMAT_VERSION {
            return Err(ReviewError::new(
                "validate review report",
                None,
                format!(
                    "review format version {} is unsupported; expected {REVIEW_FORMAT_VERSION}",
                    self.review_format_version
                ),
            ));
        }
        if self.display_name.is_empty() || self.display_name.trim() != self.display_name {
            return Err(ReviewError::new(
                "validate review report",
                None,
                "display name must be non-empty and have no surrounding whitespace",
            ));
        }
        if self.occupied_cells == 0 {
            return Err(ReviewError::new(
                "validate review report",
                None,
                "occupied-cell count must be positive",
            ));
        }
        if !self.bounds.contains(self.origin) {
            return Err(ReviewError::new(
                "validate review report",
                None,
                "object origin lies outside the reported authoring bounds",
            ));
        }
        validate_report_counts(self)?;
        validate_report_masks(self)?;
        validate_report_dependencies(self)?;
        if self.frame_size != [REVIEW_FRAME_WIDTH, REVIEW_FRAME_HEIGHT] {
            return Err(ReviewError::new(
                "validate review report",
                None,
                format!(
                    "frame size {:?} does not match {REVIEW_FRAME_WIDTH}x{REVIEW_FRAME_HEIGHT}",
                    self.frame_size
                ),
            ));
        }
        self.framing.validate()?;
        let expected_frames: Vec<_> = REVIEW_FRAME_SPECS
            .into_iter()
            .map(ReviewFrameReport::from)
            .collect();
        if self.frames != expected_frames {
            return Err(ReviewError::new(
                "validate review report",
                None,
                "frame manifest does not match the versioned review contract",
            ));
        }
        let expected_fingerprint = composite_fingerprint(
            self.object_fingerprint,
            self.style_catalog_fingerprint,
            self.palette_fingerprint,
        );
        if self.review_fingerprint != expected_fingerprint {
            return Err(ReviewError::new(
                "validate review report",
                None,
                "composite review fingerprint does not match its semantic fingerprints",
            ));
        }
        Ok(())
    }
}

fn validate_report_counts(report: &ReviewReport) -> Result<(), ReviewError> {
    if report.part_counts.is_empty() || report.part_counts.values().any(|count| *count == 0) {
        return Err(ReviewError::new(
            "validate review report",
            None,
            "semantic-part counts must be non-empty and positive",
        ));
    }
    let part_total = report
        .part_counts
        .values()
        .try_fold(0_u32, |total, count| total.checked_add(*count))
        .ok_or_else(|| {
            ReviewError::new(
                "validate review report",
                None,
                "semantic-part counts overflow u32",
            )
        })?;
    if part_total != report.occupied_cells {
        return Err(ReviewError::new(
            "validate review report",
            None,
            "semantic-part counts do not sum to the occupied-cell count",
        ));
    }
    Ok(())
}

fn validate_report_masks(report: &ReviewReport) -> Result<(), ReviewError> {
    if !is_strictly_ordered(&report.blocker_footprint)
        || report
            .blocker_footprint
            .iter()
            .any(|position| !report.bounds.contains_axial(*position))
    {
        return Err(ReviewError::new(
            "validate review report",
            None,
            "blocker footprint must be strictly ordered and inside the authoring bounds",
        ));
    }
    if !is_strictly_ordered(&report.canopy_occluders)
        || report
            .canopy_occluders
            .iter()
            .any(|position| !report.bounds.contains(*position))
    {
        return Err(ReviewError::new(
            "validate review report",
            None,
            "canopy cells must be strictly ordered and inside the authoring bounds",
        ));
    }
    Ok(())
}

fn validate_report_dependencies(report: &ReviewReport) -> Result<(), ReviewError> {
    if report.swatch_dependencies.is_empty()
        || !is_strictly_ordered_by(&report.swatch_dependencies, |left, right| {
            left.id < right.id
        })
    {
        return Err(ReviewError::new(
            "validate review report",
            None,
            "swatch dependencies must be non-empty and strictly ordered",
        ));
    }

    let mut swatches = BTreeMap::new();
    for dependency in &report.swatch_dependencies {
        let tags = dependency.tags.iter().cloned().collect::<BTreeSet<_>>();
        if tags.len() != dependency.tags.len() || !dependency.tags.iter().eq(tags.iter()) {
            return Err(ReviewError::new(
                "validate review report",
                None,
                format!(
                    "swatch dependency '{}' tags are not strictly ordered",
                    dependency.id
                ),
            ));
        }
        let swatch = PaletteSwatch::new(dependency.display_name.clone(), dependency.color, tags)
            .map_err(|error| {
                ReviewError::new(
                    "validate review report",
                    None,
                    format!("invalid swatch dependency '{}': {error}", dependency.id),
                )
            })?;
        swatches.insert(dependency.id.clone(), swatch);
    }
    let palette = ArtPalette::new(swatches).map_err(|error| {
        ReviewError::new(
            "validate review report",
            None,
            format!("invalid review swatch dependencies: {error}"),
        )
    })?;

    if report.style_dependencies.is_empty()
        || !is_strictly_ordered_by(&report.style_dependencies, |left, right| left.id < right.id)
    {
        return Err(ReviewError::new(
            "validate review report",
            None,
            "style dependencies must be non-empty and strictly ordered",
        ));
    }

    let mut styles = BTreeMap::new();
    let mut referenced_swatches = BTreeSet::new();
    let mut placement_total = 0_u32;
    for dependency in &report.style_dependencies {
        if dependency.placements == 0 {
            return Err(ReviewError::new(
                "validate review report",
                None,
                format!("style dependency '{}' has no placements", dependency.id),
            ));
        }
        placement_total = placement_total
            .checked_add(dependency.placements)
            .ok_or_else(|| {
                ReviewError::new(
                    "validate review report",
                    None,
                    "style placement counts overflow u32",
                )
            })?;
        referenced_swatches.insert(dependency.base_swatch.clone());
        let emission = match (&dependency.emission_swatch, dependency.emission_strength) {
            (Some(swatch), Some(strength)) => {
                referenced_swatches.insert(swatch.clone());
                Some(
                    VoxelEmission::new(swatch.clone(), strength).map_err(|error| {
                        ReviewError::new(
                            "validate review report",
                            None,
                            format!("invalid style dependency '{}': {error}", dependency.id),
                        )
                    })?,
                )
            }
            (None, None) => None,
            _ => {
                return Err(ReviewError::new(
                    "validate review report",
                    None,
                    format!(
                        "style dependency '{}' must specify both emission swatch and strength",
                        dependency.id
                    ),
                ));
            }
        };
        let style = VoxelStyle::new(
            dependency.display_name.clone(),
            dependency.base_swatch.clone(),
            dependency.surface_mode,
            dependency.opacity,
            emission,
        )
        .map_err(|error| {
            ReviewError::new(
                "validate review report",
                None,
                format!("invalid style dependency '{}': {error}", dependency.id),
            )
        })?;
        styles.insert(dependency.id.clone(), style);
    }
    if placement_total != report.occupied_cells {
        return Err(ReviewError::new(
            "validate review report",
            None,
            "style placement counts do not sum to the occupied-cell count",
        ));
    }
    if referenced_swatches
        != report
            .swatch_dependencies
            .iter()
            .map(|dependency| dependency.id.clone())
            .collect()
    {
        return Err(ReviewError::new(
            "validate review report",
            None,
            "swatch dependencies differ from the exact set used by reviewed styles",
        ));
    }
    let styles = VoxelStyleCatalog::new(styles).map_err(|error| {
        ReviewError::new(
            "validate review report",
            None,
            format!("invalid review style dependencies: {error}"),
        )
    })?;
    styles.validate(&palette).map_err(|error| {
        ReviewError::new(
            "validate review report",
            None,
            format!("invalid review dependency graph: {error}"),
        )
    })
}

fn is_strictly_ordered<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| {
        pair.first()
            .zip(pair.last())
            .is_some_and(|(left, right)| left < right)
    })
}

fn is_strictly_ordered_by<T>(values: &[T], less: impl Fn(&T, &T) -> bool) -> bool {
    values.windows(2).all(|pair| {
        pair.first()
            .zip(pair.last())
            .is_some_and(|(left, right)| less(left, right))
    })
}

fn increment_count(count: &mut u32, kind: &str) -> Result<(), ReviewError> {
    *count = count.checked_add(1).ok_or_else(|| {
        ReviewError::new(
            "build review report",
            None,
            format!("{kind} overflowed u32"),
        )
    })?;
    Ok(())
}

const fn part_label(part: ObjectPart) -> &'static str {
    use hex_assets::{EffectPart, PlantPart, PropPart};
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

fn composite_fingerprint(object: u64, styles: u64, palette: u64) -> String {
    format!("v{REVIEW_FORMAT_VERSION}-{object:016x}-{styles:016x}-{palette:016x}")
}

/// Actionable failure while constructing or publishing a review pack.
#[derive(Debug)]
pub struct ReviewError {
    operation: &'static str,
    path: Option<PathBuf>,
    detail: String,
}

impl ReviewError {
    fn new(operation: &'static str, path: Option<PathBuf>, detail: impl Into<String>) -> Self {
        Self {
            operation,
            path,
            detail: detail.into(),
        }
    }

    fn at(operation: &'static str, path: &Path, error: impl fmt::Display) -> Self {
        Self::new(operation, Some(path.to_path_buf()), error.to_string())
    }

    /// Stable description of the failed operation.
    #[must_use]
    pub const fn operation(&self) -> &'static str {
        self.operation
    }

    /// Relevant filesystem path, when this is an I/O failure.
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

impl fmt::Display for ReviewError {
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

impl std::error::Error for ReviewError {}

/// Minimal coverage evidence from one accepted frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReviewFrameStats {
    /// Brightest RGB channel present.
    pub brightest: u8,
    /// Pixels visibly different from the top-left clear-color sample.
    pub variant_pixels: u64,
}

/// Converts Bevy's fixed review target into an owned RGBA frame.
pub fn captured_rgba(image: &BevyImage) -> Result<RgbaImage, ReviewError> {
    if image.width() != REVIEW_FRAME_WIDTH || image.height() != REVIEW_FRAME_HEIGHT {
        return Err(ReviewError::new(
            "convert review frame",
            None,
            format!(
                "renderer output is {}x{}; expected {REVIEW_FRAME_WIDTH}x{REVIEW_FRAME_HEIGHT}",
                image.width(),
                image.height()
            ),
        ));
    }
    let rgba = image
        .clone()
        .try_into_dynamic()
        .map_err(|error| {
            ReviewError::new(
                "convert review frame",
                None,
                format!("unsupported or uninitialized renderer output: {error}"),
            )
        })?
        .into_rgba8();
    validate_review_frame(&rgba)?;
    Ok(rgba)
}

/// Rejects wrong-sized, black, or effectively background-only review frames.
pub fn validate_review_frame(frame: &RgbaImage) -> Result<ReviewFrameStats, ReviewError> {
    if frame.width() != REVIEW_FRAME_WIDTH || frame.height() != REVIEW_FRAME_HEIGHT {
        return Err(ReviewError::new(
            "validate review frame",
            None,
            format!(
                "frame is {}x{}; expected {REVIEW_FRAME_WIDTH}x{REVIEW_FRAME_HEIGHT}",
                frame.width(),
                frame.height()
            ),
        ));
    }

    let reference = *frame.get_pixel(0, 0);
    let mut brightest = u8::MIN;
    let mut variant_pixels = 0_u64;
    for pixel in frame.pixels() {
        let [red, green, blue, _alpha] = pixel.0;
        brightest = brightest.max(red).max(green).max(blue);
        let [reference_red, reference_green, reference_blue, _reference_alpha] = reference.0;
        if red.abs_diff(reference_red) > FRAME_VARIATION_THRESHOLD
            || green.abs_diff(reference_green) > FRAME_VARIATION_THRESHOLD
            || blue.abs_diff(reference_blue) > FRAME_VARIATION_THRESHOLD
        {
            variant_pixels = variant_pixels.saturating_add(1);
        }
    }

    if variant_pixels < MIN_VARIANT_PIXELS {
        return Err(ReviewError::new(
            "validate review frame",
            None,
            "frame is blank or effectively background-only",
        ));
    }
    Ok(ReviewFrameStats {
        brightest,
        variant_pixels,
    })
}

/// Composes the ordered source frames into a fixed four-by-three contact sheet.
///
/// `frames` must follow [`REVIEW_FRAME_SPECS`] exactly.
pub fn build_contact_sheet(frames: &[RgbaImage]) -> Result<RgbaImage, ReviewError> {
    if frames.len() != REVIEW_FRAME_COUNT {
        return Err(ReviewError::new(
            "build review contact sheet",
            None,
            format!(
                "received {} frames; expected {REVIEW_FRAME_COUNT}",
                frames.len()
            ),
        ));
    }
    let mut sheet = RgbaImage::from_pixel(
        CONTACT_SHEET_WIDTH,
        CONTACT_SHEET_HEIGHT,
        CONTACT_BACKGROUND_RGBA,
    );
    for (index, frame) in frames.iter().enumerate() {
        validate_review_frame(frame)?;
        let index_u32 = u32::try_from(index).map_err(|error| {
            ReviewError::new(
                "build review contact sheet",
                None,
                format!("frame index cannot be represented: {error}"),
            )
        })?;
        let column = index_u32 % CONTACT_SHEET_COLUMNS;
        let row = index_u32 / CONTACT_SHEET_COLUMNS;
        let x = CONTACT_SHEET_GUTTER + column * (CONTACT_THUMB_SIZE + CONTACT_SHEET_GUTTER);
        let y = CONTACT_SHEET_GUTTER + row * (CONTACT_THUMB_SIZE + CONTACT_SHEET_GUTTER);
        let thumbnail = imageops::resize(
            frame,
            CONTACT_THUMB_SIZE,
            CONTACT_THUMB_SIZE,
            ResizeFilter::Triangle,
        );
        sheet.copy_from(&thumbnail, x, y).map_err(|error| {
            ReviewError::new(
                "build review contact sheet",
                None,
                format!("cannot place thumbnail {}: {error}", index + 1),
            )
        })?;
        let ordinal = u8::try_from(index + 1).map_err(|error| {
            ReviewError::new(
                "build review contact sheet",
                None,
                format!("frame ordinal cannot be represented: {error}"),
            )
        })?;
        draw_ordinal(&mut sheet, x + 6, y + 6, ordinal);
    }
    Ok(sheet)
}

fn draw_ordinal(sheet: &mut RgbaImage, x: u32, y: u32, ordinal: u8) {
    const SCALE: u32 = 3;
    const GLYPH_WIDTH: u32 = 3;
    const GLYPH_HEIGHT: u32 = 5;
    const GLYPH_GAP: u32 = 2;
    const PADDING: u32 = 3;
    let label_width = PADDING * 2 + GLYPH_WIDTH * SCALE * 2 + GLYPH_GAP;
    let label_height = PADDING * 2 + GLYPH_HEIGHT * SCALE;
    for local_y in 0..label_height {
        for local_x in 0..label_width {
            sheet.put_pixel(x + local_x, y + local_y, LABEL_BACKGROUND_RGBA);
        }
    }

    let tens = ordinal / 10;
    let ones = ordinal % 10;
    draw_digit(sheet, x + PADDING, y + PADDING, tens, SCALE);
    draw_digit(
        sheet,
        x + PADDING + GLYPH_WIDTH * SCALE + GLYPH_GAP,
        y + PADDING,
        ones,
        SCALE,
    );
}

fn draw_digit(sheet: &mut RgbaImage, x: u32, y: u32, digit: u8, scale: u32) {
    for (index, enabled) in digit_pattern(digit).iter().copied().enumerate() {
        if !enabled {
            continue;
        }
        let Ok(index) = u32::try_from(index) else {
            continue;
        };
        let glyph_x = index % 3;
        let glyph_y = index / 3;
        for pixel_y in 0..scale {
            for pixel_x in 0..scale {
                sheet.put_pixel(
                    x + glyph_x * scale + pixel_x,
                    y + glyph_y * scale + pixel_y,
                    LABEL_FOREGROUND_RGBA,
                );
            }
        }
    }
}

const fn digit_pattern(digit: u8) -> &'static [bool; 15] {
    match digit {
        0 => &[
            true, true, true, true, false, true, true, false, true, true, false, true, true, true,
            true,
        ],
        1 => &[
            false, true, false, true, true, false, false, true, false, false, true, false, true,
            true, true,
        ],
        2 => &[
            true, true, true, false, false, true, true, true, true, true, false, false, true, true,
            true,
        ],
        3 => &[
            true, true, true, false, false, true, true, true, true, false, false, true, true, true,
            true,
        ],
        4 => &[
            true, false, true, true, false, true, true, true, true, false, false, true, false,
            false, true,
        ],
        5 => &[
            true, true, true, true, false, false, true, true, true, false, false, true, true, true,
            true,
        ],
        6 => &[
            true, true, true, true, false, false, true, true, true, true, false, true, true, true,
            true,
        ],
        7 => &[
            true, true, true, false, false, true, false, true, false, true, false, false, true,
            false, false,
        ],
        8 => &[
            true, true, true, true, false, true, true, true, true, true, false, true, true, true,
            true,
        ],
        9 => &[
            true, true, true, true, false, true, true, true, true, false, false, true, true, true,
            true,
        ],
        _ => &[
            false, false, false, false, false, false, false, false, false, false, false, false,
            false, false, false,
        ],
    }
}

/// Result of attempting to publish an immutable review directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewPublishOutcome {
    /// A newly complete pack became visible at this path.
    Published(PathBuf),
    /// The same complete report was already present and remained untouched.
    AlreadyPublished(PathBuf),
}

impl ReviewPublishOutcome {
    /// Final immutable review directory.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Published(path) | Self::AlreadyPublished(path) => path,
        }
    }
}

/// Returns the immutable destination directory for a validated report.
pub fn review_pack_path(
    repository_root: &Path,
    report: &ReviewReport,
) -> Result<PathBuf, ReviewError> {
    report.validate_contract()?;
    Ok(repository_root
        .join(REVIEW_OUTPUT_PATH)
        .join(report.object_id.as_str())
        .join(&report.review_fingerprint))
}

/// Writes and atomically publishes a complete deterministic review pack.
///
/// Every frame is validated before filesystem work starts. Files are written with
/// create-new semantics inside a unique hidden sibling directory, verified there,
/// and exposed by renaming that directory to its previously absent fingerprint path.
/// Existing complete packs are immutable and produce [`ReviewPublishOutcome::AlreadyPublished`].
pub fn publish_review_pack(
    repository_root: &Path,
    report: &ReviewReport,
    frames: &[RgbaImage],
) -> Result<ReviewPublishOutcome, ReviewError> {
    publish_review_pack_with_hook(repository_root, report, frames, |_| Ok(()))
}

pub(crate) fn publish_review_pack_with_pre_rename_check(
    repository_root: &Path,
    report: &ReviewReport,
    frames: &[RgbaImage],
    mut check: impl FnMut() -> Result<(), String>,
) -> Result<ReviewPublishOutcome, ReviewError> {
    publish_review_pack_with_hook(repository_root, report, frames, |checkpoint| {
        if checkpoint == PublishCheckpoint::BeforeRename {
            check().map_err(|detail| {
                ReviewError::new("verify review sources before publication", None, detail)
            })?;
        }
        Ok(())
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublishCheckpoint {
    Frame(u8),
    ContactSheet,
    Report,
    BeforeRename,
}

fn publish_review_pack_with_hook(
    repository_root: &Path,
    report: &ReviewReport,
    frames: &[RgbaImage],
    mut hook: impl FnMut(PublishCheckpoint) -> Result<(), ReviewError>,
) -> Result<ReviewPublishOutcome, ReviewError> {
    report.validate_for_publication()?;
    let report_bytes = report.to_ron_bytes()?;
    if frames.len() != REVIEW_FRAME_COUNT {
        return Err(ReviewError::new(
            "publish review pack",
            None,
            format!(
                "received {} frames; expected {REVIEW_FRAME_COUNT}",
                frames.len()
            ),
        ));
    }
    for frame in frames {
        validate_review_frame(frame)?;
    }
    let contact_sheet = build_contact_sheet(frames)?;
    let final_path = review_pack_path(repository_root, report)?;
    let parent = final_path.parent().ok_or_else(|| {
        ReviewError::at(
            "prepare review directory",
            &final_path,
            "destination has no parent directory",
        )
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| ReviewError::at("create review directory", parent, error))?;

    if path_exists(&final_path, "inspect review destination")? {
        validate_pack_directory(&final_path, &report_bytes, frames, &contact_sheet)?;
        return Ok(ReviewPublishOutcome::AlreadyPublished(final_path));
    }

    let staging_path = create_staging_directory(parent, &report.review_fingerprint)?;
    let mut staging = StagingDirectory::new(staging_path);
    for (index, spec) in REVIEW_FRAME_SPECS.into_iter().enumerate() {
        let Some(frame) = frames.get(index) else {
            return Err(ReviewError::new(
                "publish review pack",
                None,
                format!("missing frame {}", spec.ordinal),
            ));
        };
        write_png_create_new(&staging.path().join(spec.file_name), frame)?;
        hook(PublishCheckpoint::Frame(spec.ordinal))?;
    }
    write_png_create_new(
        &staging.path().join(REVIEW_CONTACT_SHEET_FILE),
        &contact_sheet,
    )?;
    hook(PublishCheckpoint::ContactSheet)?;
    write_bytes_create_new(&staging.path().join(REVIEW_REPORT_FILE), &report_bytes)?;
    hook(PublishCheckpoint::Report)?;
    validate_pack_directory(staging.path(), &report_bytes, frames, &contact_sheet)?;
    hook(PublishCheckpoint::BeforeRename)?;

    match fs::rename(staging.path(), &final_path) {
        Ok(()) => {
            staging.disarm();
            Ok(ReviewPublishOutcome::Published(final_path))
        }
        Err(rename_error) if path_exists(&final_path, "inspect concurrent review publication")? => {
            validate_pack_directory(&final_path, &report_bytes, frames, &contact_sheet).map_err(
                |existing_error| {
                    ReviewError::at(
                        "publish review pack",
                        &final_path,
                        format!(
                            "destination appeared during publication and conflicts with this pack \
                             ({rename_error}); existing pack is invalid: {existing_error}"
                        ),
                    )
                },
            )?;
            Ok(ReviewPublishOutcome::AlreadyPublished(final_path))
        }
        Err(error) => Err(ReviewError::at("publish review pack", &final_path, error)),
    }
}

fn create_staging_directory(parent: &Path, fingerprint: &str) -> Result<PathBuf, ReviewError> {
    for _attempt in 0..STAGING_ATTEMPTS {
        let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let name = format!(".{fingerprint}.{}.{}.staging", std::process::id(), sequence);
        let path = parent.join(name);
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(ReviewError::at(
                    "create review staging directory",
                    &path,
                    error,
                ));
            }
        }
    }
    Err(ReviewError::at(
        "create review staging directory",
        parent,
        format!("could not find a unique name after {STAGING_ATTEMPTS} attempts"),
    ))
}

struct StagingDirectory {
    path: PathBuf,
    armed: bool,
}

impl StagingDirectory {
    const fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        if self.armed {
            drop(fs::remove_dir_all(&self.path));
        }
    }
}

fn write_png_create_new(path: &Path, image: &RgbaImage) -> Result<(), ReviewError> {
    let bytes = encode_png(image)?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| ReviewError::at("create review PNG", path, error))?;
    let mut writer = BufWriter::new(file);
    writer
        .write_all(&bytes)
        .map_err(|error| ReviewError::at("write review PNG", path, error))?;
    writer
        .flush()
        .map_err(|error| ReviewError::at("flush review PNG", path, error))?;
    writer
        .get_ref()
        .sync_all()
        .map_err(|error| ReviewError::at("sync review PNG", path, error))
}

fn encode_png(image: &RgbaImage) -> Result<Vec<u8>, ReviewError> {
    let mut bytes = Vec::new();
    let mut writer = BufWriter::new(&mut bytes);
    PngEncoder::new_with_quality(&mut writer, CompressionType::Fast, PngFilterType::Paeth)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            ExtendedColorType::Rgba8,
        )
        .map_err(|error| ReviewError::new("encode review PNG", None, error.to_string()))?;
    writer
        .flush()
        .map_err(|error| ReviewError::new("flush encoded review PNG", None, error.to_string()))?;
    drop(writer);
    Ok(bytes)
}

fn write_bytes_create_new(path: &Path, bytes: &[u8]) -> Result<(), ReviewError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| ReviewError::at("create review report", path, error))?;
    file.write_all(bytes)
        .map_err(|error| ReviewError::at("write review report", path, error))?;
    file.sync_all()
        .map_err(|error| ReviewError::at("sync review report", path, error))
}

fn validate_pack_directory(
    directory: &Path,
    expected_report: &[u8],
    expected_frames: &[RgbaImage],
    expected_contact_sheet: &RgbaImage,
) -> Result<(), ReviewError> {
    let expected_names = expected_pack_names();
    let mut actual_names = BTreeSet::new();
    let entries = fs::read_dir(directory)
        .map_err(|error| ReviewError::at("inspect review pack", directory, error))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| ReviewError::at("inspect review pack", directory, error))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| ReviewError::at("inspect review artifact", &path, error))?;
        if !file_type.is_file() {
            return Err(ReviewError::at(
                "validate review pack",
                &path,
                "review artifacts must be regular files",
            ));
        }
        let name = entry.file_name().into_string().map_err(|name| {
            ReviewError::at(
                "validate review pack",
                &path,
                format!(
                    "artifact name is not valid Unicode: {}",
                    display_os_string(name)
                ),
            )
        })?;
        actual_names.insert(name);
    }
    if actual_names != expected_names {
        return Err(ReviewError::at(
            "validate review pack",
            directory,
            format!(
                "artifact set differs from contract; expected {expected_names:?}, \
                 found {actual_names:?}"
            ),
        ));
    }

    let report_path = directory.join(REVIEW_REPORT_FILE);
    let report = fs::read(&report_path)
        .map_err(|error| ReviewError::at("read review report", &report_path, error))?;
    if report != expected_report {
        return Err(ReviewError::at(
            "validate review pack",
            &report_path,
            "existing report differs for the same review fingerprint",
        ));
    }

    for (spec, expected_frame) in REVIEW_FRAME_SPECS.into_iter().zip(expected_frames) {
        let path = directory.join(spec.file_name);
        let bytes =
            fs::read(&path).map_err(|error| ReviewError::at("read review PNG", &path, error))?;
        let expected_bytes = encode_png(expected_frame)?;
        if bytes != expected_bytes {
            return Err(ReviewError::at(
                "validate review pack",
                &path,
                "existing frame bytes differ for the same review fingerprint",
            ));
        }
        let image = decode_png(&path)?;
        validate_review_frame(&image)
            .map_err(|error| ReviewError::at("validate review pack", &path, error.detail()))?;
    }
    let contact_path = directory.join(REVIEW_CONTACT_SHEET_FILE);
    let contact_bytes = fs::read(&contact_path)
        .map_err(|error| ReviewError::at("read review PNG", &contact_path, error))?;
    let expected_contact_bytes = encode_png(expected_contact_sheet)?;
    if contact_bytes != expected_contact_bytes {
        return Err(ReviewError::at(
            "validate review pack",
            &contact_path,
            "existing contact-sheet bytes differ from the source frames",
        ));
    }
    let contact = decode_png(&contact_path)?;
    if contact.width() != CONTACT_SHEET_WIDTH || contact.height() != CONTACT_SHEET_HEIGHT {
        return Err(ReviewError::at(
            "validate review pack",
            &contact_path,
            format!(
                "contact sheet is {}x{}; expected {CONTACT_SHEET_WIDTH}x{CONTACT_SHEET_HEIGHT}",
                contact.width(),
                contact.height()
            ),
        ));
    }
    Ok(())
}

fn decode_png(path: &Path) -> Result<RgbaImage, ReviewError> {
    let bytes = fs::read(path).map_err(|error| ReviewError::at("read review PNG", path, error))?;
    image::load_from_memory_with_format(&bytes, ImageFormat::Png)
        .map_err(|error| ReviewError::at("decode review PNG", path, error))
        .map(image::DynamicImage::into_rgba8)
}

fn expected_pack_names() -> BTreeSet<String> {
    REVIEW_FRAME_SPECS
        .into_iter()
        .map(|spec| spec.file_name.to_owned())
        .chain([
            REVIEW_CONTACT_SHEET_FILE.to_owned(),
            REVIEW_REPORT_FILE.to_owned(),
        ])
        .collect()
}

fn path_exists(path: &Path, operation: &'static str) -> Result<bool, ReviewError> {
    path.try_exists()
        .map_err(|error| ReviewError::at(operation, path, error))
}

fn display_os_string(value: OsString) -> String {
    value.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::atomic::{AtomicU64, Ordering};

    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    use hex_assets::{
        PaletteSwatch, PlantPart, VoxelEmission, VoxelStyle, VoxelSurfaceMode,
        OBJECT_BLUEPRINT_SCHEMA_VERSION,
    };

    use super::*;

    static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let sequence = TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "hex-editor-review-{label}-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("test directory should be created");
            Self { path }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            drop(fs::remove_dir_all(&self.path));
        }
    }

    fn swatch_id(value: &str) -> SwatchId {
        SwatchId::new(value).expect("test swatch id should be valid")
    }

    fn style_id(value: &str) -> VoxelStyleId {
        VoxelStyleId::new(value).expect("test style id should be valid")
    }

    fn object_id(value: &str) -> ObjectAssetId {
        ObjectAssetId::new(value).expect("test object id should be valid")
    }

    fn art_fixture() -> (ObjectBlueprint, VoxelStyleCatalog, ArtPalette) {
        let bark = swatch_id("vegetation/bark");
        let leaf = swatch_id("vegetation/leaf");
        let glow = swatch_id("effect/glow");
        let palette = ArtPalette::new(BTreeMap::from([
            (
                bark.clone(),
                PaletteSwatch::new(
                    "Bark",
                    SrgbColor::new(0.31, 0.19, 0.10).expect("test color should be valid"),
                    BTreeSet::from(["vegetation".to_owned()]),
                )
                .expect("test swatch should be valid"),
            ),
            (
                glow.clone(),
                PaletteSwatch::new(
                    "Glow",
                    SrgbColor::new(0.45, 0.82, 0.91).expect("test color should be valid"),
                    BTreeSet::from(["effect".to_owned()]),
                )
                .expect("test swatch should be valid"),
            ),
            (
                leaf.clone(),
                PaletteSwatch::new(
                    "Leaf",
                    SrgbColor::new(0.18, 0.48, 0.22).expect("test color should be valid"),
                    BTreeSet::from(["vegetation".to_owned()]),
                )
                .expect("test swatch should be valid"),
            ),
        ]))
        .expect("test palette should be valid");
        let wood_style = style_id("plant/wood");
        let leaf_style = style_id("plant/leaf");
        let styles = VoxelStyleCatalog::new(BTreeMap::from([
            (
                leaf_style.clone(),
                VoxelStyle::new(
                    "Leaves",
                    leaf,
                    VoxelSurfaceMode::Cutout,
                    0.82,
                    Some(VoxelEmission::new(glow, 0.25).expect("test emission should be valid")),
                )
                .expect("test style should be valid"),
            ),
            (
                wood_style.clone(),
                VoxelStyle::new("Wood", bark, VoxelSurfaceMode::Opaque, 1.0, None)
                    .expect("test style should be valid"),
            ),
        ]))
        .expect("test style catalog should be valid");
        let object = ObjectBlueprint {
            schema_version: OBJECT_BLUEPRINT_SCHEMA_VERSION,
            id: object_id("plant/review-tree"),
            display_name: "Review Tree".to_owned(),
            category: ObjectCategory::Plant,
            bounds: ObjectBounds::DEFAULT,
            connectivity: ConnectivityPolicy::Grounded,
            origin: LocalVoxelCoord::new(0, 0, 0),
            placements: vec![
                hex_assets::ObjectPlacement {
                    position: LocalVoxelCoord::new(0, 0, 0),
                    style: wood_style.clone(),
                    part: ObjectPart::Plant(PlantPart::Root),
                },
                hex_assets::ObjectPlacement {
                    position: LocalVoxelCoord::new(0, 0, 1),
                    style: wood_style,
                    part: ObjectPart::Plant(PlantPart::Trunk),
                },
                hex_assets::ObjectPlacement {
                    position: LocalVoxelCoord::new(0, 0, 2),
                    style: leaf_style,
                    part: ObjectPart::Plant(PlantPart::Foliage),
                },
            ],
            blocker_footprint: vec![LocalAxialCoord::new(0, 0)],
            canopy_occluders: vec![LocalVoxelCoord::new(0, 0, 2)],
        };
        object
            .validate(&styles)
            .expect("test object should be valid");
        (object, styles, palette)
    }

    fn valid_frame(accent: Rgba<u8>) -> RgbaImage {
        let mut frame = RgbaImage::from_pixel(
            REVIEW_FRAME_WIDTH,
            REVIEW_FRAME_HEIGHT,
            Rgba(REVIEW_CLEAR_RGBA),
        );
        let center_x = REVIEW_FRAME_WIDTH / 2;
        let center_y = REVIEW_FRAME_HEIGHT / 2;
        for y in center_y - 48..center_y + 48 {
            for x in center_x - 48..center_x + 48 {
                frame.put_pixel(x, y, accent);
            }
        }
        frame
    }

    fn frame_set() -> Vec<RgbaImage> {
        (0..REVIEW_FRAME_COUNT)
            .map(|index| {
                let index = u8::try_from(index).expect("review frame index should fit in u8");
                valid_frame(Rgba([
                    56_u8.saturating_add(index.saturating_mul(12)),
                    110_u8.saturating_add(index.saturating_mul(7)),
                    170_u8.saturating_sub(index.saturating_mul(6)),
                    255,
                ]))
            })
            .collect()
    }

    #[test]
    fn frame_manifest_has_the_required_stable_order() {
        let names: Vec<_> = REVIEW_FRAME_SPECS
            .into_iter()
            .map(|spec| spec.file_name)
            .collect();
        assert_eq!(
            names,
            vec![
                "01-perspective.png",
                "02-top.png",
                "03-turn-000.png",
                "04-turn-060.png",
                "05-turn-120.png",
                "06-turn-180.png",
                "07-turn-240.png",
                "08-turn-300.png",
                "09-semantic.png",
                "10-blocker-canopy.png",
            ]
        );
        for (index, spec) in REVIEW_FRAME_SPECS.into_iter().enumerate() {
            assert_eq!(usize::from(spec.ordinal), index + 1);
        }
    }

    #[test]
    fn turn_views_advance_by_exact_sixty_degree_azimuths() {
        let yaws: Vec<_> = REVIEW_FRAME_SPECS
            .into_iter()
            .filter(|spec| spec.file_name.contains("turn-"))
            .filter_map(|spec| match spec.camera {
                ReviewCameraView::Orbit { yaw_radians, .. } => Some(yaw_radians),
                ReviewCameraView::Top => None,
            })
            .collect();
        assert_eq!(yaws.len(), 6);
        for pair in yaws.windows(2) {
            let Some((left, right)) = pair.first().zip(pair.get(1)) else {
                continue;
            };
            assert!((*right - *left - std::f32::consts::FRAC_PI_3).abs() < 1e-6);
        }
    }

    #[test]
    fn camera_poses_share_framing_and_look_at_the_focus() {
        let (object, _, _) = art_fixture();
        let framing = ReviewFraming::from_object(&object).expect("framing should succeed");
        for spec in REVIEW_FRAME_SPECS {
            let pose = framing
                .camera_pose(spec.camera)
                .expect("fixed pose should be valid");
            let eye = Vec3::from_array(pose.eye);
            let focus = Vec3::from_array(pose.focus);
            assert!((eye.distance(focus) - framing.radius).abs() < 1e-4);
            let transform = pose.transform();
            let direction = (focus - eye).normalize();
            assert!(transform.forward().as_vec3().dot(direction) > 0.9999);
        }

        let top = framing
            .camera_pose(ReviewCameraView::Top)
            .expect("top pose should be valid");
        assert!(Vec3::from_array(top.up).distance(Vec3::NEG_Z) < 1e-6);
        let top_offset = Vec3::from_array(top.eye) - Vec3::from_array(top.focus);
        assert!(top_offset.cross(Vec3::Y).length() < 1e-5);
    }

    #[test]
    fn report_is_order_independent_and_byte_stable() {
        let (object, styles, palette) = art_fixture();
        let first = ReviewReport::new(&object, &styles, &palette).expect("report should build");
        let first_bytes = first.to_ron_bytes().expect("report should serialize");

        let mut reordered = object.clone();
        reordered.placements.reverse();
        reordered.blocker_footprint.reverse();
        reordered.canopy_occluders.reverse();
        let second = ReviewReport::new(&reordered, &styles, &palette)
            .expect("reordered report should build");
        let second_bytes = second
            .to_ron_bytes()
            .expect("reordered report should serialize");
        assert_eq!(first, second);
        assert_eq!(first_bytes, second_bytes);
        assert!(first_bytes.ends_with(b"\n"));
        let source = std::str::from_utf8(&first_bytes).expect("report should be UTF-8");
        let round_trip: ReviewReport = ron::from_str(source).expect("report should parse");
        assert_eq!(
            round_trip
                .to_ron_bytes()
                .expect("parsed report should remain readable"),
            first_bytes
        );
        assert!(round_trip.validate_for_publication().is_err());
        assert_eq!(first.occupied_cells, 3);
        assert_eq!(
            first.part_counts,
            BTreeMap::from([
                ("plant/foliage".to_owned(), 1),
                ("plant/root".to_owned(), 1),
                ("plant/trunk".to_owned(), 1),
            ])
        );
        assert_eq!(first.style_dependencies.len(), 2);
        assert_eq!(first.swatch_dependencies.len(), 3);
        assert_eq!(
            first.review_fingerprint,
            "v1-cf2ac38befa36349-b449adbbc6e91f77-e0e35a51a766f8e2"
        );
    }

    #[test]
    fn every_semantic_fingerprint_contributes_to_review_identity() {
        let (object, styles, palette) = art_fixture();
        let baseline =
            ReviewReport::new(&object, &styles, &palette).expect("baseline report should build");

        let mut changed_object = object.clone();
        changed_object.display_name = "Changed Tree".to_owned();
        let object_report = ReviewReport::new(&changed_object, &styles, &palette)
            .expect("changed object report should build");
        assert_ne!(
            baseline.review_fingerprint,
            object_report.review_fingerprint
        );

        let mut changed_styles = styles.clone();
        changed_styles
            .insert(
                style_id("unused/style"),
                VoxelStyle::new(
                    "Unused",
                    swatch_id("vegetation/bark"),
                    VoxelSurfaceMode::Opaque,
                    1.0,
                    None,
                )
                .expect("changed style should be valid"),
            )
            .expect("style insert should succeed");
        let style_report = ReviewReport::new(&object, &changed_styles, &palette)
            .expect("changed style report should build");
        assert_ne!(baseline.review_fingerprint, style_report.review_fingerprint);

        let mut changed_palette = palette.clone();
        changed_palette
            .insert(
                swatch_id("unused/color"),
                PaletteSwatch::new(
                    "Unused",
                    SrgbColor::new(0.8, 0.1, 0.2).expect("changed color should be valid"),
                    BTreeSet::from(["test".to_owned()]),
                )
                .expect("changed swatch should be valid"),
            )
            .expect("swatch insert should succeed");
        let palette_report = ReviewReport::new(&object, &styles, &changed_palette)
            .expect("changed palette report should build");
        assert_ne!(
            baseline.review_fingerprint,
            palette_report.review_fingerprint
        );
    }

    #[test]
    fn report_validation_rejects_mutated_rendering_dependencies() {
        let (object, styles, palette) = art_fixture();
        let mut report =
            ReviewReport::new(&object, &styles, &palette).expect("report should build");
        let dependency = report
            .style_dependencies
            .first_mut()
            .expect("fixture report should contain a style dependency");
        dependency.opacity = 0.0;
        let error = report
            .to_ron_bytes()
            .expect_err("invalid mutated style semantics must not serialize");
        assert!(error.to_string().contains("opacity"));
    }

    #[test]
    fn frame_validation_rejects_wrong_size_black_and_uniform_frames() {
        let wrong = RgbaImage::from_pixel(
            REVIEW_FRAME_WIDTH - 1,
            REVIEW_FRAME_HEIGHT,
            Rgba([220, 220, 220, 255]),
        );
        assert!(validate_review_frame(&wrong).is_err());

        let black = RgbaImage::from_pixel(
            REVIEW_FRAME_WIDTH,
            REVIEW_FRAME_HEIGHT,
            Rgba([0, 0, 0, 255]),
        );
        assert!(validate_review_frame(&black).is_err());

        let uniform = RgbaImage::from_pixel(
            REVIEW_FRAME_WIDTH,
            REVIEW_FRAME_HEIGHT,
            Rgba([180, 180, 180, 255]),
        );
        assert!(validate_review_frame(&uniform).is_err());

        let single_color_object = valid_frame(Rgba([60, 170, 80, 255]));
        let stats = validate_review_frame(&single_color_object)
            .expect("one-color object should remain reviewable");
        assert!(stats.variant_pixels >= 96 * 96);

        let black_object = valid_frame(Rgba([0, 0, 0, 255]));
        let stats = validate_review_frame(&black_object)
            .expect("a black silhouette on the review clear should remain reviewable");
        assert_eq!(stats.brightest, REVIEW_CLEAR_RGBA[2]);
        assert!(stats.variant_pixels >= 96 * 96);
    }

    #[test]
    fn bevy_capture_conversion_preserves_rgba_pixels() {
        let frame = valid_frame(Rgba([70, 130, 210, 255]));
        let image = BevyImage::new(
            Extent3d {
                width: REVIEW_FRAME_WIDTH,
                height: REVIEW_FRAME_HEIGHT,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            frame.clone().into_raw(),
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        let converted = captured_rgba(&image).expect("supported capture should convert");
        assert_eq!(converted, frame);

        let wrong = BevyImage::new_target_texture(10, 10, TextureFormat::Rgba8UnormSrgb, None);
        assert!(captured_rgba(&wrong).is_err());
    }

    #[test]
    fn contact_sheet_preserves_frame_order_and_empty_slots() {
        let frames = frame_set();
        let sheet = build_contact_sheet(&frames).expect("contact sheet should build");
        assert_eq!(
            sheet.dimensions(),
            (CONTACT_SHEET_WIDTH, CONTACT_SHEET_HEIGHT)
        );
        for (index, frame) in frames.iter().enumerate() {
            let index_u32 = u32::try_from(index).expect("test index should fit u32");
            let column = index_u32 % CONTACT_SHEET_COLUMNS;
            let row = index_u32 / CONTACT_SHEET_COLUMNS;
            let sample_x = CONTACT_SHEET_GUTTER
                + column * (CONTACT_THUMB_SIZE + CONTACT_SHEET_GUTTER)
                + CONTACT_THUMB_SIZE / 2;
            let sample_y = CONTACT_SHEET_GUTTER
                + row * (CONTACT_THUMB_SIZE + CONTACT_SHEET_GUTTER)
                + CONTACT_THUMB_SIZE / 2;
            assert_eq!(
                sheet.get_pixel(sample_x, sample_y),
                frame.get_pixel(REVIEW_FRAME_WIDTH / 2, REVIEW_FRAME_HEIGHT / 2)
            );
        }

        for index in [10_u32, 11_u32] {
            let column = index % CONTACT_SHEET_COLUMNS;
            let row = index / CONTACT_SHEET_COLUMNS;
            let sample_x = CONTACT_SHEET_GUTTER
                + column * (CONTACT_THUMB_SIZE + CONTACT_SHEET_GUTTER)
                + CONTACT_THUMB_SIZE / 2;
            let sample_y = CONTACT_SHEET_GUTTER
                + row * (CONTACT_THUMB_SIZE + CONTACT_SHEET_GUTTER)
                + CONTACT_THUMB_SIZE / 2;
            assert_eq!(
                *sheet.get_pixel(sample_x, sample_y),
                CONTACT_BACKGROUND_RGBA
            );
        }
    }

    #[test]
    fn locked_png_encoder_is_repeatable_and_lossless() {
        let directory = TestDirectory::new("png");
        let frame = valid_frame(Rgba([190, 90, 130, 255]));
        let first = directory.path.join("first.png");
        let second = directory.path.join("second.png");
        write_png_create_new(&first, &frame).expect("first PNG should write");
        write_png_create_new(&second, &frame).expect("second PNG should write");
        assert_eq!(
            fs::read(&first).expect("first PNG should read"),
            fs::read(&second).expect("second PNG should read")
        );
        assert_eq!(decode_png(&first).expect("PNG should decode"), frame);
    }

    #[test]
    fn publication_is_complete_and_idempotent() {
        let directory = TestDirectory::new("publish");
        let (object, styles, palette) = art_fixture();
        let report = ReviewReport::new(&object, &styles, &palette).expect("report should build");
        let frames = frame_set();
        let first =
            publish_review_pack(&directory.path, &report, &frames).expect("pack should publish");
        assert!(matches!(first, ReviewPublishOutcome::Published(_)));
        let final_path =
            review_pack_path(&directory.path, &report).expect("pack path should resolve");
        assert_eq!(first.path(), final_path);
        assert_eq!(actual_file_names(&final_path), expected_pack_names());

        let second = publish_review_pack(&directory.path, &report, &frames)
            .expect("identical pack should be idempotent");
        assert!(matches!(second, ReviewPublishOutcome::AlreadyPublished(_)));
        assert_eq!(second.path(), final_path);
    }

    #[test]
    fn failed_staging_steps_never_publish_or_leave_own_stage() {
        let directory = TestDirectory::new("failure");
        let (object, styles, palette) = art_fixture();
        let report = ReviewReport::new(&object, &styles, &palette).expect("report should build");
        let frames = frame_set();
        let checkpoints = [
            PublishCheckpoint::Frame(1),
            PublishCheckpoint::Frame(10),
            PublishCheckpoint::ContactSheet,
            PublishCheckpoint::Report,
            PublishCheckpoint::BeforeRename,
        ];

        for checkpoint in checkpoints {
            let result =
                publish_review_pack_with_hook(&directory.path, &report, &frames, |actual| {
                    if actual == checkpoint {
                        Err(ReviewError::new(
                            "inject review failure",
                            None,
                            format!("{actual:?}"),
                        ))
                    } else {
                        Ok(())
                    }
                });
            assert!(result.is_err());
            let final_path =
                review_pack_path(&directory.path, &report).expect("pack path should resolve");
            assert!(!final_path.exists());
            let parent = final_path
                .parent()
                .expect("review path should have a parent");
            assert_eq!(actual_file_names(parent), BTreeSet::new());
        }
    }

    #[test]
    fn conflicting_existing_pack_is_preserved_and_rejected() {
        let directory = TestDirectory::new("collision");
        let (object, styles, palette) = art_fixture();
        let report = ReviewReport::new(&object, &styles, &palette).expect("report should build");
        let frames = frame_set();
        let outcome =
            publish_review_pack(&directory.path, &report, &frames).expect("pack should publish");
        let report_path = outcome.path().join(REVIEW_REPORT_FILE);
        fs::write(&report_path, b"corrupt").expect("test should corrupt report");

        let result = publish_review_pack(&directory.path, &report, &frames);
        assert!(result.is_err());
        assert_eq!(
            fs::read(&report_path).expect("corrupt report should remain"),
            b"corrupt"
        );
    }

    #[test]
    fn valid_but_different_existing_images_are_preserved_and_rejected() {
        let directory = TestDirectory::new("image-collision");
        let (object, styles, palette) = art_fixture();
        let report = ReviewReport::new(&object, &styles, &palette).expect("report should build");
        let frames = frame_set();
        let outcome =
            publish_review_pack(&directory.path, &report, &frames).expect("pack should publish");
        let frame_path = outcome.path().join(REVIEW_FRAME_SPECS[0].file_name);
        fs::remove_file(&frame_path).expect("test frame should be removable");
        let conflicting = valid_frame(Rgba([220, 80, 40, 255]));
        write_png_create_new(&frame_path, &conflicting).expect("conflicting frame should write");

        let result = publish_review_pack(&directory.path, &report, &frames);
        assert!(result.is_err());
        assert_eq!(
            decode_png(&frame_path).expect("conflicting frame should remain"),
            conflicting
        );

        let contact_directory = TestDirectory::new("contact-collision");
        let outcome = publish_review_pack(&contact_directory.path, &report, &frames)
            .expect("second pack should publish");
        let contact_path = outcome.path().join(REVIEW_CONTACT_SHEET_FILE);
        let mut conflicting_contact =
            build_contact_sheet(&frames).expect("contact sheet should build");
        conflicting_contact.put_pixel(0, 0, Rgba([255, 0, 255, 255]));
        fs::remove_file(&contact_path).expect("test contact sheet should be removable");
        write_png_create_new(&contact_path, &conflicting_contact)
            .expect("conflicting contact sheet should write");

        let result = publish_review_pack(&contact_directory.path, &report, &frames);
        assert!(result.is_err());
        assert_eq!(
            decode_png(&contact_path).expect("conflicting contact sheet should remain"),
            conflicting_contact
        );
    }

    #[test]
    fn concurrent_identical_publication_resolves_idempotently() {
        let directory = TestDirectory::new("identical-race");
        let (object, styles, palette) = art_fixture();
        let report = ReviewReport::new(&object, &styles, &palette).expect("report should build");
        let frames = frame_set();
        let mut nested_published = false;
        let outcome =
            publish_review_pack_with_hook(&directory.path, &report, &frames, |checkpoint| {
                if checkpoint == PublishCheckpoint::BeforeRename && !nested_published {
                    let nested = publish_review_pack(&directory.path, &report, &frames)?;
                    assert!(matches!(nested, ReviewPublishOutcome::Published(_)));
                    nested_published = true;
                }
                Ok(())
            })
            .expect("outer publication should accept the identical winner");
        assert!(matches!(outcome, ReviewPublishOutcome::AlreadyPublished(_)));
        assert!(nested_published);
    }

    #[test]
    fn concurrent_conflicting_publication_is_preserved_and_rejected() {
        let directory = TestDirectory::new("conflicting-race");
        let (object, styles, palette) = art_fixture();
        let report = ReviewReport::new(&object, &styles, &palette).expect("report should build");
        let frames = frame_set();
        let final_path =
            review_pack_path(&directory.path, &report).expect("pack path should resolve");
        let conflicting_file = final_path.join("foreign.txt");
        let result =
            publish_review_pack_with_hook(&directory.path, &report, &frames, |checkpoint| {
                if checkpoint == PublishCheckpoint::BeforeRename {
                    fs::create_dir(&final_path).map_err(|error| {
                        ReviewError::at("create conflicting test pack", &final_path, error)
                    })?;
                    fs::write(&conflicting_file, b"foreign").map_err(|error| {
                        ReviewError::at("write conflicting test pack", &conflicting_file, error)
                    })?;
                }
                Ok(())
            });
        assert!(result.is_err());
        assert_eq!(
            fs::read(&conflicting_file).expect("conflicting artifact should remain"),
            b"foreign"
        );
    }

    #[test]
    fn missing_or_extra_existing_artifacts_are_rejected() {
        let directory = TestDirectory::new("artifact-set");
        let (object, styles, palette) = art_fixture();
        let report = ReviewReport::new(&object, &styles, &palette).expect("report should build");
        let frames = frame_set();
        let outcome =
            publish_review_pack(&directory.path, &report, &frames).expect("pack should publish");
        let missing = outcome.path().join("03-turn-000.png");
        fs::remove_file(&missing).expect("test should remove one frame");
        assert!(publish_review_pack(&directory.path, &report, &frames).is_err());

        write_png_create_new(&missing, frames.get(2).expect("frame should exist"))
            .expect("missing frame should be restored");
        let extra = outcome.path().join("extra.png");
        write_png_create_new(&extra, frames.first().expect("frame should exist"))
            .expect("extra frame should write");
        assert!(publish_review_pack(&directory.path, &report, &frames).is_err());
    }

    fn actual_file_names(directory: &Path) -> BTreeSet<String> {
        let Ok(entries) = fs::read_dir(directory) else {
            return BTreeSet::new();
        };
        entries
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect()
    }
}
