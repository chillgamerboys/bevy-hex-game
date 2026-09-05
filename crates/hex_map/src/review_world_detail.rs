//! Strict review-only contracts for disposable world-detail presentation.
//!
//! These values describe render projections only. They deliberately contain no
//! voxel edits, traversal policy, blockers, picking authority, save data, or
//! gameplay-lighting controls.

use std::collections::BTreeMap;

use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// Schema version accepted by [`ReviewWorldDetailProfileV1`].
pub const REVIEW_WORLD_DETAIL_PROFILE_VERSION_V1: u16 = 1;

/// Schema version emitted by [`ReviewWorldDetailReportV1`].
pub const REVIEW_WORLD_DETAIL_REPORT_VERSION_V1: u16 = 1;

/// Schema version emitted by [`ReviewRuntimeReceiptV1`].
pub const REVIEW_RUNTIME_RECEIPT_VERSION_V1: u16 = 1;

/// Runtime-authored binding between one automated launch and its exact inputs.
///
/// The receipt hash is SHA-256 over compact JSON for the first seven fields in
/// declaration order. The launch process constructs this only after validating
/// the harness inputs; callers cannot supply the receipt or its digest.
#[derive(Resource, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewRuntimeReceiptV1 {
    /// Exact receipt schema version. Only version 1 is accepted.
    pub version: u16,
    /// Fresh harness-provided lowercase hexadecimal launch nonce.
    pub launch_nonce: String,
    /// Nonzero operating-system process identifier of the game runtime.
    pub process_id: u64,
    /// SHA-256 of the exact bytes at the canonicalized current executable path.
    pub executable_sha256: String,
    /// Harness-computed SHA-256 of the frozen source-provenance object.
    pub source_provenance_sha256: String,
    /// SHA-256 of the exact UTF-8 `HEX_REVIEW_CAPTURE_PLAN` bytes.
    pub capture_plan_sha256: String,
    /// SHA-256 of the resolved canonical world-detail profile, including control.
    pub profile_sha256: String,
    /// SHA-256 of compact ordered JSON for all preceding fields.
    pub receipt_sha256: String,
}

#[derive(Serialize)]
struct ReviewRuntimeReceiptHashBodyV1<'a> {
    version: u16,
    launch_nonce: &'a str,
    process_id: u64,
    executable_sha256: &'a str,
    source_provenance_sha256: &'a str,
    capture_plan_sha256: &'a str,
    profile_sha256: &'a str,
}

impl ReviewRuntimeReceiptV1 {
    /// Constructs and hashes a strict receipt from runtime-observed launch data.
    pub fn new(
        launch_nonce: String,
        process_id: u64,
        executable_sha256: String,
        source_provenance_sha256: String,
        capture_plan_sha256: String,
        profile_sha256: String,
    ) -> Result<Self, ReviewWorldDetailError> {
        let mut receipt = Self {
            version: REVIEW_RUNTIME_RECEIPT_VERSION_V1,
            launch_nonce,
            process_id,
            executable_sha256,
            source_provenance_sha256,
            capture_plan_sha256,
            profile_sha256,
            receipt_sha256: String::new(),
        };
        receipt.validate_bindings()?;
        receipt.receipt_sha256 = receipt.expected_receipt_sha256()?;
        Ok(receipt)
    }

    /// Validates field spelling and recomputes the runtime-authored receipt hash.
    pub fn validate(&self) -> Result<(), ReviewWorldDetailError> {
        if self.version != REVIEW_RUNTIME_RECEIPT_VERSION_V1 {
            return Err(ReviewWorldDetailError::new(format!(
                "unsupported runtime receipt version {}; expected {}",
                self.version, REVIEW_RUNTIME_RECEIPT_VERSION_V1
            )));
        }
        self.validate_bindings()?;
        if !is_lower_hex(&self.receipt_sha256, 64) {
            return Err(ReviewWorldDetailError::new(
                "runtime receipt receipt_sha256 must be 64 lowercase hexadecimal characters",
            ));
        }
        if self.receipt_sha256 != self.expected_receipt_sha256()? {
            return Err(ReviewWorldDetailError::new(
                "runtime receipt receipt_sha256 does not match its canonical hash body",
            ));
        }
        Ok(())
    }

    fn validate_bindings(&self) -> Result<(), ReviewWorldDetailError> {
        if self.process_id == 0 {
            return Err(ReviewWorldDetailError::new(
                "runtime receipt process_id must be nonzero",
            ));
        }
        for (field, value) in [
            ("launch_nonce", self.launch_nonce.as_str()),
            ("executable_sha256", self.executable_sha256.as_str()),
            (
                "source_provenance_sha256",
                self.source_provenance_sha256.as_str(),
            ),
            ("capture_plan_sha256", self.capture_plan_sha256.as_str()),
            ("profile_sha256", self.profile_sha256.as_str()),
        ] {
            if !is_lower_hex(value, 64) {
                return Err(ReviewWorldDetailError::new(format!(
                    "runtime receipt {field} must be 64 lowercase hexadecimal characters"
                )));
            }
        }
        Ok(())
    }

    fn expected_receipt_sha256(&self) -> Result<String, ReviewWorldDetailError> {
        let body = ReviewRuntimeReceiptHashBodyV1 {
            version: self.version,
            launch_nonce: &self.launch_nonce,
            process_id: self.process_id,
            executable_sha256: &self.executable_sha256,
            source_provenance_sha256: &self.source_provenance_sha256,
            capture_plan_sha256: &self.capture_plan_sha256,
            profile_sha256: &self.profile_sha256,
        };
        let canonical = serde_json::to_vec(&body).map_err(|error| {
            ReviewWorldDetailError::new(format!(
                "could not serialize runtime receipt hash body: {error}"
            ))
        })?;
        Ok(hex_lower(&Sha256::digest(canonical)))
    }
}

/// Failure to parse, validate, canonicalize, or hash a review-detail contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewWorldDetailError {
    message: String,
}

impl ReviewWorldDetailError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn setting(family: &str, message: impl AsRef<str>) -> Self {
        Self::new(format!(
            "invalid {family} review setting: {}",
            message.as_ref()
        ))
    }
}

impl std::fmt::Display for ReviewWorldDetailError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ReviewWorldDetailError {}

/// One strict, deterministic review-only world-detail profile.
///
/// JSON input is versioned and denies unknown fields at this level and in every
/// nested section. [`Self::default`] is the shared control: all nine families are
/// `current`, so no review presentation is requested.
#[derive(Resource, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewWorldDetailProfileV1 {
    /// Exact schema version. Only version 1 is accepted.
    pub version: u16,
    /// Snow-line and snow-shell presentation.
    pub snow: SnowDetailV1,
    /// Water-surface optical presentation.
    pub water: WaterDetailV1,
    /// Collisionless world-space cloud presentation.
    pub physical_clouds: PhysicalCloudsDetailV1,
    /// Shore wetness, foam, and waterfall spray presentation.
    pub shore_and_falls: ShoreAndFallsDetailV1,
    /// Render-child scale and crown-snow presentation.
    pub alpine_vegetation: AlpineVegetationDetailV1,
    /// Exposed cliff-side value and strata presentation.
    pub cliff_strata: CliffStrataDetailV1,
    /// Collisionless terrain-prop presentation.
    pub terrain_props: TerrainPropsDetailV1,
    /// Collisionless water-edge ice presentation.
    pub ice_fringe: IceFringeDetailV1,
    /// Localized, presentation-only fog volumes.
    pub local_fog: LocalFogDetailV1,
}

impl Default for ReviewWorldDetailProfileV1 {
    fn default() -> Self {
        Self {
            version: REVIEW_WORLD_DETAIL_PROFILE_VERSION_V1,
            snow: SnowDetailV1::Current,
            water: WaterDetailV1::Current,
            physical_clouds: PhysicalCloudsDetailV1::Current,
            shore_and_falls: ShoreAndFallsDetailV1::Current,
            alpine_vegetation: AlpineVegetationDetailV1::Current,
            cliff_strata: CliffStrataDetailV1::Current,
            terrain_props: TerrainPropsDetailV1::Current,
            ice_fringe: IceFringeDetailV1::Current,
            local_fog: LocalFogDetailV1::Current,
        }
    }
}

impl ReviewWorldDetailProfileV1 {
    /// Parses strict JSON and validates the version and the complete fixed matrix.
    ///
    /// Whitespace and object-key order are accepted. Call
    /// [`Self::from_canonical_json`] when the environment contract must require
    /// the exact compact canonical spelling.
    pub fn from_json(source: &str) -> Result<Self, ReviewWorldDetailError> {
        reject_unknown_current_section_fields(source)?;
        let profile: Self = serde_json::from_str(source).map_err(|error| {
            ReviewWorldDetailError::new(format!("invalid review JSON: {error}"))
        })?;
        profile.validate()?;
        Ok(profile)
    }

    /// Parses and requires the input bytes to equal the canonical compact JSON.
    pub fn from_canonical_json(source: &str) -> Result<Self, ReviewWorldDetailError> {
        let profile = Self::from_json(source)?;
        let canonical = profile.canonical_json()?;
        if canonical != source {
            return Err(ReviewWorldDetailError::new(
                "review profile JSON is valid but not canonical compact JSON",
            ));
        }
        Ok(profile)
    }

    /// Validates the version and all nine family settings against the fixed study.
    pub fn validate(&self) -> Result<(), ReviewWorldDetailError> {
        if self.version != REVIEW_WORLD_DETAIL_PROFILE_VERSION_V1 {
            return Err(ReviewWorldDetailError::new(format!(
                "unsupported review profile version {}; expected {}",
                self.version, REVIEW_WORLD_DETAIL_PROFILE_VERSION_V1
            )));
        }
        self.snow.validate()?;
        self.water.validate()?;
        self.physical_clouds.validate()?;
        self.shore_and_falls.validate()?;
        self.alpine_vegetation.validate()?;
        self.cliff_strata.validate()?;
        self.terrain_props.validate()?;
        self.ice_fringe.validate()?;
        self.local_fog.validate()?;
        Ok(())
    }

    /// Serializes the validated profile to its sole canonical compact JSON form.
    pub fn canonical_json(&self) -> Result<String, ReviewWorldDetailError> {
        self.validate()?;
        serde_json::to_string(self).map_err(|error| {
            ReviewWorldDetailError::new(format!("could not serialize review profile: {error}"))
        })
    }

    /// Returns lowercase hexadecimal SHA-256 of [`Self::canonical_json`].
    pub fn profile_hash_sha256(&self) -> Result<String, ReviewWorldDetailError> {
        let canonical = self.canonical_json()?;
        Ok(hex_lower(&Sha256::digest(canonical.as_bytes())))
    }

    /// Returns whether all nine families use the shared control presentation.
    #[must_use]
    pub fn is_current(&self) -> bool {
        self.snow.is_current()
            && self.water.is_current()
            && self.physical_clouds.is_current()
            && self.shore_and_falls.is_current()
            && self.alpine_vegetation.is_current()
            && self.cliff_strata.is_current()
            && self.terrain_props.is_current()
            && self.ice_fringe.is_current()
            && self.local_fog.is_current()
    }

    /// Returns whether the profile needs order-independent transparency.
    #[must_use]
    pub fn requires_oit(&self) -> bool {
        self.water.requires_oit()
            || !self.physical_clouds.is_current()
            || self.shore_and_falls.requires_oit()
            || !self.ice_fringe.is_current()
    }

    /// Returns whether the profile needs screen-space transmission.
    #[must_use]
    pub fn requires_transmission(&self) -> bool {
        matches!(self.water, WaterDetailV1::Transmission { .. })
    }

    /// Returns whether the profile needs volumetric camera state.
    #[must_use]
    pub fn requires_volumetrics(&self) -> bool {
        !self.local_fog.is_current()
    }

    /// Returns stable ids for every non-control section, in family order.
    #[must_use]
    pub fn active_treatment_ids(&self) -> Vec<&'static str> {
        [
            self.snow.treatment_id(),
            self.water.treatment_id(),
            self.physical_clouds.treatment_id(),
            self.shore_and_falls.treatment_id(),
            self.alpine_vegetation.treatment_id(),
            self.cliff_strata.treatment_id(),
            self.terrain_props.treatment_id(),
            self.ice_fringe.treatment_id(),
            self.local_fog.treatment_id(),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    /// Builds the exact 60 one-factor non-control profiles in matrix order.
    #[must_use]
    pub fn atomic_matrix() -> Vec<Self> {
        let mut profiles = Vec::with_capacity(60);
        profiles.extend(SnowDetailV1::treatments().into_iter().map(|snow| Self {
            snow,
            ..Self::default()
        }));
        profiles.extend(WaterDetailV1::treatments().into_iter().map(|water| Self {
            water,
            ..Self::default()
        }));
        profiles.extend(
            PhysicalCloudsDetailV1::treatments()
                .into_iter()
                .map(|physical_clouds| Self {
                    physical_clouds,
                    ..Self::default()
                }),
        );
        profiles.extend(
            ShoreAndFallsDetailV1::treatments()
                .into_iter()
                .map(|shore_and_falls| Self {
                    shore_and_falls,
                    ..Self::default()
                }),
        );
        profiles.extend(AlpineVegetationDetailV1::treatments().into_iter().map(
            |alpine_vegetation| Self {
                alpine_vegetation,
                ..Self::default()
            },
        ));
        profiles.extend(
            CliffStrataDetailV1::treatments()
                .into_iter()
                .map(|cliff_strata| Self {
                    cliff_strata,
                    ..Self::default()
                }),
        );
        profiles.extend(
            TerrainPropsDetailV1::treatments()
                .into_iter()
                .map(|terrain_props| Self {
                    terrain_props,
                    ..Self::default()
                }),
        );
        profiles.extend(
            IceFringeDetailV1::treatments()
                .into_iter()
                .map(|ice_fringe| Self {
                    ice_fringe,
                    ..Self::default()
                }),
        );
        profiles.extend(
            LocalFogDetailV1::treatments()
                .into_iter()
                .map(|local_fog| Self {
                    local_fog,
                    ..Self::default()
                }),
        );
        profiles
    }
}

/// Serde's internally tagged unit variants accept surplus fields even when the
/// enum uses `deny_unknown_fields`. Close that narrow gap for the nine `current`
/// variants before typed deserialization; data-bearing variants remain guarded by
/// their ordinary serde field contracts.
fn reject_unknown_current_section_fields(source: &str) -> Result<(), ReviewWorldDetailError> {
    let value: serde_json::Value = serde_json::from_str(source)
        .map_err(|error| ReviewWorldDetailError::new(format!("invalid review JSON: {error}")))?;
    let Some(root) = value.as_object() else {
        return Ok(());
    };
    for family in [
        "snow",
        "water",
        "physical_clouds",
        "shore_and_falls",
        "alpine_vegetation",
        "cliff_strata",
        "terrain_props",
        "ice_fringe",
        "local_fog",
    ] {
        let Some(section) = root.get(family).and_then(serde_json::Value::as_object) else {
            continue;
        };
        if section.get("kind").and_then(serde_json::Value::as_str) == Some("current") {
            if let Some(unknown) = section.keys().find(|field| field.as_str() != "kind") {
                return Err(ReviewWorldDetailError::setting(
                    family,
                    format!("unknown field `{unknown}` for current setting"),
                ));
            }
        }
    }
    Ok(())
}

/// Review-only snow-line and vertical snow-shell presentation.
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SnowDetailV1 {
    /// Preserve the current rendered snow exactly.
    #[default]
    Current,
    /// Apply one straight integer snow-line threshold.
    StraightThreshold {
        /// First snow-covered voxel level.
        level: u16,
    },
    /// Vary the snow line coherently around a mean altitude.
    CoherentLine {
        /// Mean snow-line level.
        mean_level: u16,
        /// Maximum positive or negative coherent variation in levels.
        amplitude_levels: u8,
        /// Horizontal coherent-noise correlation length in hexes.
        correlation_hexes: u16,
    },
    /// Use the fixed terrain-aware line, optionally with vertical-only shells.
    TerrainAware {
        /// Collisionless vertical shell height in world units; zero means masking only.
        vertical_shell_height: f32,
    },
}

impl SnowDetailV1 {
    /// Returns whether this section preserves the current presentation.
    #[must_use]
    pub fn is_current(&self) -> bool {
        matches!(self, Self::Current)
    }

    /// Returns the stable matrix id, or `None` for the control.
    #[must_use]
    pub fn treatment_id(&self) -> Option<&'static str> {
        match self {
            Self::Current => None,
            Self::StraightThreshold { level: 128 } => Some("snow-01-straight-128"),
            Self::StraightThreshold { level: 140 } => Some("snow-02-straight-140"),
            Self::StraightThreshold { level: 152 } => Some("snow-03-straight-152"),
            // Focused visual choices span the full Grand V3 mountain height;
            // the original 128/140/152 matrix only probes the lower snowline.
            Self::StraightThreshold { level: 200 } => Some("snow-focused-straight-200"),
            Self::StraightThreshold { level: 260 } => Some("snow-focused-straight-260"),
            Self::CoherentLine {
                mean_level: 200,
                amplitude_levels: 16,
                correlation_hexes: 22,
            } => Some("snow-focused-coherent-200"),
            Self::CoherentLine {
                mean_level: 200,
                amplitude_levels: 32,
                correlation_hexes: 16,
            } => Some("snow-focused-coherent-200-strong"),
            Self::CoherentLine {
                mean_level: 200,
                amplitude_levels: 48,
                correlation_hexes: 12,
            } => Some("snow-focused-coherent-200-rugged"),
            Self::CoherentLine {
                mean_level: 136,
                amplitude_levels: 8,
                correlation_hexes: 22,
            } => Some("snow-04-coherent-136"),
            Self::CoherentLine {
                mean_level: 144,
                amplitude_levels: 8,
                correlation_hexes: 22,
            } => Some("snow-05-coherent-144"),
            Self::TerrainAware {
                vertical_shell_height,
            } if same(*vertical_shell_height, 0.0) => Some("snow-06-terrain-aware"),
            Self::TerrainAware {
                vertical_shell_height,
            } if same(*vertical_shell_height, 0.04) => Some("snow-07-terrain-aware-shell-004"),
            Self::TerrainAware {
                vertical_shell_height,
            } if same(*vertical_shell_height, 0.08) => Some("snow-08-terrain-aware-shell-008"),
            Self::TerrainAware {
                vertical_shell_height,
            } if same(*vertical_shell_height, 0.12) => Some("snow-09-terrain-aware-shell-012"),
            _ => None,
        }
    }

    /// Validates the original matrix and the focused Grand V3 snowline choices.
    pub fn validate(&self) -> Result<(), ReviewWorldDetailError> {
        if let Self::TerrainAware {
            vertical_shell_height,
        } = self
        {
            finite_range(
                "snow",
                "vertical_shell_height",
                *vertical_shell_height,
                0.0,
                0.12,
            )?;
        }
        valid_id("snow", self.is_current(), self.treatment_id())
    }

    fn treatments() -> [Self; 9] {
        [
            Self::StraightThreshold { level: 128 },
            Self::StraightThreshold { level: 140 },
            Self::StraightThreshold { level: 152 },
            Self::CoherentLine {
                mean_level: 136,
                amplitude_levels: 8,
                correlation_hexes: 22,
            },
            Self::CoherentLine {
                mean_level: 144,
                amplitude_levels: 8,
                correlation_hexes: 22,
            },
            Self::TerrainAware {
                vertical_shell_height: 0.0,
            },
            Self::TerrainAware {
                vertical_shell_height: 0.04,
            },
            Self::TerrainAware {
                vertical_shell_height: 0.08,
            },
            Self::TerrainAware {
                vertical_shell_height: 0.12,
            },
        ]
    }
}

/// Review-only water optics.
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WaterDetailV1 {
    /// Preserve the current opaque water geometry and material.
    #[default]
    Current,
    /// Use a transparent water surface with uniform alpha.
    UniformAlpha {
        /// Surface alpha.
        alpha: f32,
    },
    /// Combine alpha with depth-dependent absorption.
    DepthAbsorption {
        /// Surface alpha.
        alpha: f32,
        /// World-space depth that halves transmitted light.
        depth_half_distance: f32,
        /// Value multiplier approached by deep water.
        deep_value_multiplier: f32,
    },
    /// Use medium-quality screen-space transmission and restrained refraction.
    Transmission {
        /// Index of refraction.
        ior: f32,
        /// Effective material thickness in world units.
        thickness: f32,
        /// Maximum refraction in normalized screen UV.
        max_refraction_uv: f32,
    },
    /// Use a rough transparent surface without refraction.
    RoughSurface {
        /// Surface alpha.
        alpha: f32,
        /// Perceptual roughness.
        roughness: f32,
        /// Dielectric reflectance.
        reflectance: f32,
    },
}

impl WaterDetailV1 {
    /// Returns whether this section preserves the current presentation.
    #[must_use]
    pub fn is_current(&self) -> bool {
        matches!(self, Self::Current)
    }

    /// Returns whether this water treatment needs order-independent transparency.
    #[must_use]
    pub fn requires_oit(&self) -> bool {
        !matches!(self, Self::Current | Self::Transmission { .. })
    }

    /// Returns the stable matrix id, or `None` for the control.
    #[must_use]
    pub fn treatment_id(&self) -> Option<&'static str> {
        match self {
            Self::Current => None,
            Self::UniformAlpha { alpha } if same(*alpha, 0.85) => Some("water-01-alpha-085"),
            Self::UniformAlpha { alpha } if same(*alpha, 0.70) => Some("water-02-alpha-070"),
            Self::UniformAlpha { alpha } if same(*alpha, 0.55) => Some("water-03-alpha-055"),
            Self::DepthAbsorption {
                alpha,
                depth_half_distance,
                deep_value_multiplier,
            } if same(*alpha, 0.70)
                && same(*depth_half_distance, 0.70)
                && same(*deep_value_multiplier, 0.62) =>
            {
                Some("water-04-depth-short")
            }
            Self::DepthAbsorption {
                alpha,
                depth_half_distance,
                deep_value_multiplier,
            } if same(*alpha, 0.70)
                && same(*depth_half_distance, 1.40)
                && same(*deep_value_multiplier, 0.82) =>
            {
                Some("water-05-depth-long")
            }
            Self::Transmission {
                ior,
                thickness,
                max_refraction_uv,
            } if same(*ior, 1.333) && same(*thickness, 0.08) && same(*max_refraction_uv, 0.015) => {
                Some("water-06-transmission")
            }
            Self::RoughSurface {
                alpha,
                roughness,
                reflectance,
            } if same(*alpha, 0.70) && same(*roughness, 0.40) && same(*reflectance, 0.50) => {
                Some("water-07-rough-no-refraction")
            }
            _ => None,
        }
    }

    /// Validates this section against the seven fixed water treatments.
    pub fn validate(&self) -> Result<(), ReviewWorldDetailError> {
        match self {
            Self::UniformAlpha { alpha } => finite_range("water", "alpha", *alpha, 0.0, 1.0)?,
            Self::DepthAbsorption {
                alpha,
                depth_half_distance,
                deep_value_multiplier,
            } => {
                finite_range("water", "alpha", *alpha, 0.0, 1.0)?;
                finite_range(
                    "water",
                    "depth_half_distance",
                    *depth_half_distance,
                    0.01,
                    100.0,
                )?;
                finite_range(
                    "water",
                    "deep_value_multiplier",
                    *deep_value_multiplier,
                    0.0,
                    1.0,
                )?;
            }
            Self::Transmission {
                ior,
                thickness,
                max_refraction_uv,
            } => {
                finite_range("water", "ior", *ior, 1.0, 3.0)?;
                finite_range("water", "thickness", *thickness, 0.0, 10.0)?;
                finite_range("water", "max_refraction_uv", *max_refraction_uv, 0.0, 0.1)?;
            }
            Self::RoughSurface {
                alpha,
                roughness,
                reflectance,
            } => {
                finite_range("water", "alpha", *alpha, 0.0, 1.0)?;
                finite_range("water", "roughness", *roughness, 0.0, 1.0)?;
                finite_range("water", "reflectance", *reflectance, 0.0, 1.0)?;
            }
            Self::Current => {}
        }
        valid_id("water", self.is_current(), self.treatment_id())
    }

    fn treatments() -> [Self; 7] {
        [
            Self::UniformAlpha { alpha: 0.85 },
            Self::UniformAlpha { alpha: 0.70 },
            Self::UniformAlpha { alpha: 0.55 },
            Self::DepthAbsorption {
                alpha: 0.70,
                depth_half_distance: 0.70,
                deep_value_multiplier: 0.62,
            },
            Self::DepthAbsorption {
                alpha: 0.70,
                depth_half_distance: 1.40,
                deep_value_multiplier: 0.82,
            },
            Self::Transmission {
                ior: 1.333,
                thickness: 0.08,
                max_refraction_uv: 0.015,
            },
            Self::RoughSurface {
                alpha: 0.70,
                roughness: 0.40,
                reflectance: 0.50,
            },
        ]
    }
}

/// Height band for world-space cloud clusters, relative to maximum terrain height.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudAltitudeBandV1 {
    /// `H+4..H+12`, fully clear of the maximum terrain height.
    Clear,
    /// `H-4..H+4`, grazing the highest peaks.
    Grazing,
    /// `H-22..H-10`, crossing high terrain.
    Crossing,
}

/// Low-poly cloud-cluster shape family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudShapeV1 {
    /// Rounded batched puffs.
    Rounded,
    /// Flattened lenticular clusters.
    Lenticular,
}

/// Review-only, deterministic world-space cloud geometry.
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PhysicalCloudsDetailV1 {
    /// Add no world-space clouds; preserve the existing sky-dome clouds.
    #[default]
    Current,
    /// Add one faceted layer in a named altitude band.
    FacetedLayer {
        /// Altitude band relative to maximum exposed natural terrain height `H`.
        altitude_band: CloudAltitudeBandV1,
        /// Measured fraction of the deterministic circular massif field covered by
        /// the XZ projection of the rendered low-poly puff silhouettes.
        projected_coverage: f32,
        /// Minimum deterministic cluster diameter in world units.
        diameter_min: f32,
        /// Maximum deterministic cluster diameter in world units.
        diameter_max: f32,
    },
    /// Compare rounded and lenticular clusters at grazing height.
    GrazingShape {
        /// Cluster silhouette family.
        shape: CloudShapeV1,
        /// Measured fraction of the deterministic circular massif field covered by
        /// the XZ projection of the rendered low-poly puff silhouettes.
        projected_coverage: f32,
        /// Minimum deterministic cluster diameter in world units.
        diameter_min: f32,
        /// Maximum deterministic cluster diameter in world units.
        diameter_max: f32,
    },
    /// Compare rounded-cloud projected coverage at grazing height.
    RoundedCoverage {
        /// Measured fraction of the deterministic circular massif field covered by
        /// the XZ projection of the rendered low-poly puff silhouettes.
        projected_coverage: f32,
        /// Minimum deterministic cluster diameter in world units.
        diameter_min: f32,
        /// Maximum deterministic cluster diameter in world units.
        diameter_max: f32,
    },
    /// Add restrained maximum projected shadowing to rounded grazing clouds.
    RoundedShadow {
        /// Measured fraction of the deterministic circular massif field covered by
        /// the XZ projection of the rendered low-poly puff silhouettes.
        projected_coverage: f32,
        /// Minimum deterministic cluster diameter in world units.
        diameter_min: f32,
        /// Maximum deterministic cluster diameter in world units.
        diameter_max: f32,
        /// Maximum projected shadow fraction.
        max_projected_shadow: f32,
        /// World-space radial width of the shadow's outer feather transition.
        shadow_blur: f32,
    },
}

impl PhysicalCloudsDetailV1 {
    /// Returns whether this section adds no world-space cloud geometry.
    #[must_use]
    pub fn is_current(&self) -> bool {
        matches!(self, Self::Current)
    }

    /// Returns the stable matrix id, or `None` for the control.
    #[must_use]
    pub fn treatment_id(&self) -> Option<&'static str> {
        match self {
            Self::Current => None,
            Self::FacetedLayer {
                altitude_band,
                projected_coverage,
                diameter_min,
                diameter_max,
            } if same(*projected_coverage, 0.18)
                && same(*diameter_min, 16.0)
                && same(*diameter_max, 32.0) =>
            {
                Some(match altitude_band {
                    CloudAltitudeBandV1::Clear => "clouds-01-faceted-clear",
                    CloudAltitudeBandV1::Grazing => "clouds-02-faceted-grazing",
                    CloudAltitudeBandV1::Crossing => "clouds-03-faceted-crossing",
                })
            }
            Self::GrazingShape {
                shape,
                projected_coverage,
                diameter_min,
                diameter_max,
            } if same(*projected_coverage, 0.18)
                && same(*diameter_min, 16.0)
                && same(*diameter_max, 32.0) =>
            {
                Some(match shape {
                    CloudShapeV1::Rounded => "clouds-04-rounded-grazing",
                    CloudShapeV1::Lenticular => "clouds-05-lenticular-grazing",
                })
            }
            Self::RoundedCoverage {
                projected_coverage,
                diameter_min,
                diameter_max,
            } if same(*diameter_min, 16.0) && same(*diameter_max, 32.0) => {
                if same(*projected_coverage, 0.10) {
                    Some("clouds-06-rounded-coverage-010")
                } else if same(*projected_coverage, 0.28) {
                    Some("clouds-07-rounded-coverage-028")
                } else {
                    None
                }
            }
            Self::RoundedShadow {
                projected_coverage,
                diameter_min,
                diameter_max,
                max_projected_shadow,
                shadow_blur,
            } if same(*projected_coverage, 0.18)
                && same(*diameter_min, 16.0)
                && same(*diameter_max, 32.0)
                && same(*max_projected_shadow, 0.20)
                && same(*shadow_blur, 24.0) =>
            {
                Some("clouds-08-rounded-shadow")
            }
            _ => None,
        }
    }

    /// Validates this section against the eight fixed cloud treatments.
    pub fn validate(&self) -> Result<(), ReviewWorldDetailError> {
        match self {
            Self::FacetedLayer {
                projected_coverage,
                diameter_min,
                diameter_max,
                ..
            }
            | Self::GrazingShape {
                projected_coverage,
                diameter_min,
                diameter_max,
                ..
            }
            | Self::RoundedCoverage {
                projected_coverage,
                diameter_min,
                diameter_max,
            } => {
                validate_cloud_geometry(*projected_coverage, *diameter_min, *diameter_max)?;
            }
            Self::RoundedShadow {
                projected_coverage,
                diameter_min,
                diameter_max,
                max_projected_shadow,
                shadow_blur,
            } => {
                validate_cloud_geometry(*projected_coverage, *diameter_min, *diameter_max)?;
                finite_range(
                    "physical_clouds",
                    "max_projected_shadow",
                    *max_projected_shadow,
                    0.0,
                    1.0,
                )?;
                finite_range("physical_clouds", "shadow_blur", *shadow_blur, 0.0, 100.0)?;
            }
            Self::Current => {}
        }
        valid_id("physical_clouds", self.is_current(), self.treatment_id())
    }

    fn treatments() -> [Self; 8] {
        [
            Self::FacetedLayer {
                altitude_band: CloudAltitudeBandV1::Clear,
                projected_coverage: 0.18,
                diameter_min: 16.0,
                diameter_max: 32.0,
            },
            Self::FacetedLayer {
                altitude_band: CloudAltitudeBandV1::Grazing,
                projected_coverage: 0.18,
                diameter_min: 16.0,
                diameter_max: 32.0,
            },
            Self::FacetedLayer {
                altitude_band: CloudAltitudeBandV1::Crossing,
                projected_coverage: 0.18,
                diameter_min: 16.0,
                diameter_max: 32.0,
            },
            Self::GrazingShape {
                shape: CloudShapeV1::Rounded,
                projected_coverage: 0.18,
                diameter_min: 16.0,
                diameter_max: 32.0,
            },
            Self::GrazingShape {
                shape: CloudShapeV1::Lenticular,
                projected_coverage: 0.18,
                diameter_min: 16.0,
                diameter_max: 32.0,
            },
            Self::RoundedCoverage {
                projected_coverage: 0.10,
                diameter_min: 16.0,
                diameter_max: 32.0,
            },
            Self::RoundedCoverage {
                projected_coverage: 0.28,
                diameter_min: 16.0,
                diameter_max: 32.0,
            },
            Self::RoundedShadow {
                projected_coverage: 0.18,
                diameter_min: 16.0,
                diameter_max: 32.0,
                max_projected_shadow: 0.20,
                shadow_blur: 24.0,
            },
        ]
    }
}

/// Review-only shore wetness, foam, and waterfall spray.
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ShoreAndFallsDetailV1 {
    /// Preserve the current shore and waterfall presentation.
    #[default]
    Current,
    /// Add a dark, smoother collisionless rim at water contact.
    WetRim {
        /// Rim width in world units.
        width: f32,
        /// Relative value delta, such as `-0.12` for twelve percent darker.
        value_delta: f32,
        /// Additive roughness delta.
        roughness_delta: f32,
    },
    /// Add collisionless foam at water edges.
    Foam {
        /// Foam width in world units.
        width: f32,
        /// Foam alpha.
        opacity: f32,
    },
    /// Add plunge spray and pool foam at waterfall anchors.
    PlungeSpray {
        /// Spray footprint radius in hexes.
        radius_hexes: u8,
        /// Spray height in world units.
        height: f32,
        /// Spray alpha.
        opacity: f32,
        /// Pool-foam radius in hexes.
        pool_foam_radius_hexes: u8,
    },
    /// Combine the restrained wet rim, foam, and lower-opacity spray.
    RestrainedCombination {
        /// Rim width in world units.
        wet_rim_width: f32,
        /// Relative rim value delta.
        wet_rim_value_delta: f32,
        /// Additive rim roughness delta.
        wet_rim_roughness_delta: f32,
        /// Foam width in world units.
        foam_width: f32,
        /// Foam alpha.
        foam_opacity: f32,
        /// Spray footprint radius in hexes.
        spray_radius_hexes: u8,
        /// Spray height in world units.
        spray_height: f32,
        /// Spray alpha.
        spray_opacity: f32,
        /// Pool-foam radius in hexes.
        pool_foam_radius_hexes: u8,
    },
}

impl ShoreAndFallsDetailV1 {
    /// Returns whether this section preserves the current presentation.
    #[must_use]
    pub fn is_current(&self) -> bool {
        matches!(self, Self::Current)
    }

    /// Returns whether this treatment needs order-independent transparency.
    #[must_use]
    pub fn requires_oit(&self) -> bool {
        matches!(
            self,
            Self::Foam { .. } | Self::PlungeSpray { .. } | Self::RestrainedCombination { .. }
        )
    }

    /// Returns the stable matrix id, or `None` for the control.
    #[must_use]
    pub fn treatment_id(&self) -> Option<&'static str> {
        match self {
            Self::Current => None,
            Self::WetRim {
                width,
                value_delta,
                roughness_delta,
            } if same(*width, 0.12)
                && same(*value_delta, -0.12)
                && same(*roughness_delta, -0.15) =>
            {
                Some("shore-01-wet-rim-narrow")
            }
            Self::WetRim {
                width,
                value_delta,
                roughness_delta,
            } if same(*width, 0.25)
                && same(*value_delta, -0.18)
                && same(*roughness_delta, -0.15) =>
            {
                Some("shore-02-wet-rim-wide")
            }
            Self::Foam { width, opacity } if same(*width, 0.10) && same(*opacity, 0.35) => {
                Some("shore-03-foam-narrow")
            }
            Self::Foam { width, opacity } if same(*width, 0.20) && same(*opacity, 0.55) => {
                Some("shore-04-foam-wide")
            }
            Self::PlungeSpray {
                radius_hexes: 3,
                height,
                opacity,
                pool_foam_radius_hexes: 2,
            } if same(*height, 4.2) && same(*opacity, 0.08) => Some("shore-05-plunge-spray"),
            Self::RestrainedCombination {
                wet_rim_width,
                wet_rim_value_delta,
                wet_rim_roughness_delta,
                foam_width,
                foam_opacity,
                spray_radius_hexes: 3,
                spray_height,
                spray_opacity,
                pool_foam_radius_hexes: 2,
            } if same(*wet_rim_width, 0.12)
                && same(*wet_rim_value_delta, -0.12)
                && same(*wet_rim_roughness_delta, -0.15)
                && same(*foam_width, 0.10)
                && same(*foam_opacity, 0.35)
                && same(*spray_height, 4.2)
                && same(*spray_opacity, 0.06) =>
            {
                Some("shore-06-restrained-combination")
            }
            _ => None,
        }
    }

    /// Validates this section against the six fixed shore/falls treatments.
    pub fn validate(&self) -> Result<(), ReviewWorldDetailError> {
        match self {
            Self::WetRim {
                width,
                value_delta,
                roughness_delta,
            } => {
                finite_range("shore_and_falls", "width", *width, 0.0, 1.0)?;
                finite_range("shore_and_falls", "value_delta", *value_delta, -1.0, 0.0)?;
                finite_range(
                    "shore_and_falls",
                    "roughness_delta",
                    *roughness_delta,
                    -1.0,
                    1.0,
                )?;
            }
            Self::Foam { width, opacity } => {
                finite_range("shore_and_falls", "width", *width, 0.0, 1.0)?;
                finite_range("shore_and_falls", "opacity", *opacity, 0.0, 1.0)?;
            }
            Self::PlungeSpray {
                height, opacity, ..
            } => {
                finite_range("shore_and_falls", "height", *height, 0.0, 20.0)?;
                finite_range("shore_and_falls", "opacity", *opacity, 0.0, 1.0)?;
            }
            Self::RestrainedCombination {
                wet_rim_width,
                wet_rim_value_delta,
                wet_rim_roughness_delta,
                foam_width,
                foam_opacity,
                spray_height,
                spray_opacity,
                ..
            } => {
                finite_range("shore_and_falls", "wet_rim_width", *wet_rim_width, 0.0, 1.0)?;
                finite_range(
                    "shore_and_falls",
                    "wet_rim_value_delta",
                    *wet_rim_value_delta,
                    -1.0,
                    0.0,
                )?;
                finite_range(
                    "shore_and_falls",
                    "wet_rim_roughness_delta",
                    *wet_rim_roughness_delta,
                    -1.0,
                    1.0,
                )?;
                finite_range("shore_and_falls", "foam_width", *foam_width, 0.0, 1.0)?;
                finite_range("shore_and_falls", "foam_opacity", *foam_opacity, 0.0, 1.0)?;
                finite_range("shore_and_falls", "spray_height", *spray_height, 0.0, 20.0)?;
                finite_range("shore_and_falls", "spray_opacity", *spray_opacity, 0.0, 1.0)?;
            }
            Self::Current => {}
        }
        valid_id("shore_and_falls", self.is_current(), self.treatment_id())
    }

    fn treatments() -> [Self; 6] {
        [
            Self::WetRim {
                width: 0.12,
                value_delta: -0.12,
                roughness_delta: -0.15,
            },
            Self::WetRim {
                width: 0.25,
                value_delta: -0.18,
                roughness_delta: -0.15,
            },
            Self::Foam {
                width: 0.10,
                opacity: 0.35,
            },
            Self::Foam {
                width: 0.20,
                opacity: 0.55,
            },
            Self::PlungeSpray {
                radius_hexes: 3,
                height: 4.2,
                opacity: 0.08,
                pool_foam_radius_hexes: 2,
            },
            Self::RestrainedCombination {
                wet_rim_width: 0.12,
                wet_rim_value_delta: -0.12,
                wet_rim_roughness_delta: -0.15,
                foam_width: 0.10,
                foam_opacity: 0.35,
                spray_radius_hexes: 3,
                spray_height: 4.2,
                spray_opacity: 0.06,
                pool_foam_radius_hexes: 2,
            },
        ]
    }
}

/// Review-only alpine vegetation render-child variation.
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AlpineVegetationDetailV1 {
    /// Preserve current render-child scale and materials.
    #[default]
    Current,
    /// Apply deterministic horizontal and vertical render-child scale jitter.
    ScaleJitter {
        /// Minimum horizontal scale.
        horizontal_min: f32,
        /// Maximum horizontal scale.
        horizontal_max: f32,
        /// Minimum vertical scale.
        vertical_min: f32,
        /// Maximum vertical scale.
        vertical_max: f32,
    },
    /// Add collisionless snow shells to an upper fraction of eligible crowns.
    CrownSnowDust {
        /// Fraction of the crown eligible for the upper dust shell.
        upper_fraction: f32,
        /// Shell height in world units.
        shell_height: f32,
    },
    /// Combine deterministic render-child scale jitter with crown snow dust.
    ScaleJitterWithDust {
        /// Minimum horizontal scale.
        horizontal_min: f32,
        /// Maximum horizontal scale.
        horizontal_max: f32,
        /// Minimum vertical scale.
        vertical_min: f32,
        /// Maximum vertical scale.
        vertical_max: f32,
        /// Fraction of the crown eligible for the upper dust shell.
        upper_fraction: f32,
        /// Shell height in world units.
        shell_height: f32,
    },
}

impl AlpineVegetationDetailV1 {
    /// Returns whether this section preserves current vegetation presentation.
    #[must_use]
    pub fn is_current(&self) -> bool {
        matches!(self, Self::Current)
    }

    /// Returns the stable matrix id, or `None` for the control.
    #[must_use]
    pub fn treatment_id(&self) -> Option<&'static str> {
        match self {
            Self::Current => None,
            Self::ScaleJitter {
                horizontal_min,
                horizontal_max,
                vertical_min,
                vertical_max,
            } if scale_values(
                *horizontal_min,
                *horizontal_max,
                *vertical_min,
                *vertical_max,
                0.90,
                1.10,
                0.95,
                1.05,
            ) =>
            {
                Some("vegetation-01-scale-light")
            }
            Self::ScaleJitter {
                horizontal_min,
                horizontal_max,
                vertical_min,
                vertical_max,
            } if scale_values(
                *horizontal_min,
                *horizontal_max,
                *vertical_min,
                *vertical_max,
                0.80,
                1.20,
                0.90,
                1.10,
            ) =>
            {
                Some("vegetation-02-scale-wide")
            }
            Self::CrownSnowDust {
                upper_fraction,
                shell_height,
            } if same(*upper_fraction, 0.25) && same(*shell_height, 0.02) => {
                Some("vegetation-03-dust-light")
            }
            Self::CrownSnowDust {
                upper_fraction,
                shell_height,
            } if same(*upper_fraction, 0.50) && same(*shell_height, 0.04) => {
                Some("vegetation-04-dust-heavy")
            }
            Self::ScaleJitterWithDust {
                horizontal_min,
                horizontal_max,
                vertical_min,
                vertical_max,
                upper_fraction,
                shell_height,
            } if scale_values(
                *horizontal_min,
                *horizontal_max,
                *vertical_min,
                *vertical_max,
                0.90,
                1.10,
                0.95,
                1.05,
            ) && same(*upper_fraction, 0.25)
                && same(*shell_height, 0.02) =>
            {
                Some("vegetation-05-scale-light-dust-light")
            }
            Self::ScaleJitterWithDust {
                horizontal_min,
                horizontal_max,
                vertical_min,
                vertical_max,
                upper_fraction,
                shell_height,
            } if scale_values(
                *horizontal_min,
                *horizontal_max,
                *vertical_min,
                *vertical_max,
                0.85,
                1.15,
                0.92,
                1.08,
            ) && same(*upper_fraction, 0.50)
                && same(*shell_height, 0.04) =>
            {
                Some("vegetation-06-scale-heavy-dust-heavy")
            }
            _ => None,
        }
    }

    /// Validates this section against the six fixed vegetation treatments.
    pub fn validate(&self) -> Result<(), ReviewWorldDetailError> {
        match self {
            Self::ScaleJitter {
                horizontal_min,
                horizontal_max,
                vertical_min,
                vertical_max,
            } => {
                validate_scale(
                    *horizontal_min,
                    *horizontal_max,
                    *vertical_min,
                    *vertical_max,
                )?;
            }
            Self::CrownSnowDust {
                upper_fraction,
                shell_height,
            } => {
                validate_dust(*upper_fraction, *shell_height)?;
            }
            Self::ScaleJitterWithDust {
                horizontal_min,
                horizontal_max,
                vertical_min,
                vertical_max,
                upper_fraction,
                shell_height,
            } => {
                validate_scale(
                    *horizontal_min,
                    *horizontal_max,
                    *vertical_min,
                    *vertical_max,
                )?;
                validate_dust(*upper_fraction, *shell_height)?;
            }
            Self::Current => {}
        }
        valid_id("alpine_vegetation", self.is_current(), self.treatment_id())
    }

    fn treatments() -> [Self; 6] {
        [
            Self::ScaleJitter {
                horizontal_min: 0.90,
                horizontal_max: 1.10,
                vertical_min: 0.95,
                vertical_max: 1.05,
            },
            Self::ScaleJitter {
                horizontal_min: 0.80,
                horizontal_max: 1.20,
                vertical_min: 0.90,
                vertical_max: 1.10,
            },
            Self::CrownSnowDust {
                upper_fraction: 0.25,
                shell_height: 0.02,
            },
            Self::CrownSnowDust {
                upper_fraction: 0.50,
                shell_height: 0.04,
            },
            Self::ScaleJitterWithDust {
                horizontal_min: 0.90,
                horizontal_max: 1.10,
                vertical_min: 0.95,
                vertical_max: 1.05,
                upper_fraction: 0.25,
                shell_height: 0.02,
            },
            Self::ScaleJitterWithDust {
                horizontal_min: 0.85,
                horizontal_max: 1.15,
                vertical_min: 0.92,
                vertical_max: 1.08,
                upper_fraction: 0.50,
                shell_height: 0.04,
            },
        ]
    }
}

/// Review-only exposed cliff-side value and strata presentation.
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CliffStrataDetailV1 {
    /// Preserve current cliff-side materials.
    #[default]
    Current,
    /// Darken eligible natural exposed sides without changing normals or silhouettes.
    SideValue {
        /// Relative side value delta.
        value_delta: f32,
    },
    /// Add presentation-only strata to eligible sides.
    Strata {
        /// Repeating period in voxel levels.
        period_levels: u16,
        /// Stripe width in voxel levels.
        width_levels: u16,
        /// Relative stripe contrast.
        contrast: f32,
        /// Maximum coherent phase variation in levels; zero means fixed phase.
        phase_variation_levels: u8,
        /// Horizontal phase correlation length in hexes; zero means fixed phase.
        correlation_hexes: u16,
    },
    /// Combine coherent strata with a restrained side-value reduction.
    StrataWithValue {
        /// Repeating period in voxel levels.
        period_levels: u16,
        /// Stripe width in voxel levels.
        width_levels: u16,
        /// Relative stripe contrast.
        contrast: f32,
        /// Maximum coherent phase variation in levels.
        phase_variation_levels: u8,
        /// Horizontal phase correlation length in hexes.
        correlation_hexes: u16,
        /// Relative side value delta.
        value_delta: f32,
    },
}

impl CliffStrataDetailV1 {
    /// Returns whether this section preserves current cliff presentation.
    #[must_use]
    pub fn is_current(&self) -> bool {
        matches!(self, Self::Current)
    }

    /// Returns the stable matrix id, or `None` for the control.
    #[must_use]
    pub fn treatment_id(&self) -> Option<&'static str> {
        match self {
            Self::Current => None,
            Self::SideValue { value_delta } if same(*value_delta, -0.06) => {
                Some("cliff-01-value-006")
            }
            Self::SideValue { value_delta } if same(*value_delta, -0.12) => {
                Some("cliff-02-value-012")
            }
            Self::Strata {
                period_levels: 24,
                width_levels: 2,
                contrast,
                phase_variation_levels: 0,
                correlation_hexes: 0,
            } if same(*contrast, 0.08) => Some("cliff-03-strata-24"),
            Self::Strata {
                period_levels: 40,
                width_levels: 4,
                contrast,
                phase_variation_levels: 0,
                correlation_hexes: 0,
            } if same(*contrast, 0.10) => Some("cliff-04-strata-40"),
            Self::Strata {
                period_levels: 32,
                width_levels: 3,
                contrast,
                phase_variation_levels: 4,
                correlation_hexes: 22,
            } if same(*contrast, 0.08) => Some("cliff-05-strata-coherent"),
            Self::StrataWithValue {
                period_levels: 32,
                width_levels: 3,
                contrast,
                phase_variation_levels: 4,
                correlation_hexes: 22,
                value_delta,
            } if same(*contrast, 0.08) && same(*value_delta, -0.08) => {
                Some("cliff-06-strata-coherent-value")
            }
            _ => None,
        }
    }

    /// Validates this section against the six fixed cliff treatments.
    pub fn validate(&self) -> Result<(), ReviewWorldDetailError> {
        match self {
            Self::SideValue { value_delta } => {
                finite_range("cliff_strata", "value_delta", *value_delta, -1.0, 0.0)?;
            }
            Self::Strata { contrast, .. } => {
                finite_range("cliff_strata", "contrast", *contrast, 0.0, 1.0)?;
            }
            Self::StrataWithValue {
                contrast,
                value_delta,
                ..
            } => {
                finite_range("cliff_strata", "contrast", *contrast, 0.0, 1.0)?;
                finite_range("cliff_strata", "value_delta", *value_delta, -1.0, 0.0)?;
            }
            Self::Current => {}
        }
        valid_id("cliff_strata", self.is_current(), self.treatment_id())
    }

    fn treatments() -> [Self; 6] {
        [
            Self::SideValue { value_delta: -0.06 },
            Self::SideValue { value_delta: -0.12 },
            Self::Strata {
                period_levels: 24,
                width_levels: 2,
                contrast: 0.08,
                phase_variation_levels: 0,
                correlation_hexes: 0,
            },
            Self::Strata {
                period_levels: 40,
                width_levels: 4,
                contrast: 0.10,
                phase_variation_levels: 0,
                correlation_hexes: 0,
            },
            Self::Strata {
                period_levels: 32,
                width_levels: 3,
                contrast: 0.08,
                phase_variation_levels: 4,
                correlation_hexes: 22,
            },
            Self::StrataWithValue {
                period_levels: 32,
                width_levels: 3,
                contrast: 0.08,
                phase_variation_levels: 4,
                correlation_hexes: 22,
                value_delta: -0.08,
            },
        ]
    }
}

/// Review-only collisionless terrain props.
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TerrainPropsDetailV1 {
    /// Add no presentation-only terrain props.
    #[default]
    Current,
    /// Scatter low-poly boulders.
    Boulders {
        /// Eligible-cell fraction.
        density: f32,
        /// Absolute piece cap.
        cap: u16,
    },
    /// Scatter grass and litter.
    GrassLitter {
        /// Eligible-cell fraction.
        density: f32,
        /// Absolute piece cap.
        cap: u16,
    },
    /// Scatter a fixed mix of boulders, tufts, and deadwood.
    Mixed {
        /// Eligible-cell boulder fraction.
        boulder_density: f32,
        /// Eligible-cell tuft fraction.
        tuft_density: f32,
        /// Eligible-cell deadwood fraction.
        deadwood_density: f32,
        /// Absolute combined piece cap.
        cap: u16,
    },
    /// Scatter deterministic three-to-five-piece clusters.
    Clustered {
        /// Eligible-cell cluster-center fraction.
        center_density: f32,
        /// Minimum pieces per cluster.
        pieces_min: u8,
        /// Maximum pieces per cluster.
        pieces_max: u8,
        /// Absolute combined piece cap.
        cap: u16,
    },
}

impl TerrainPropsDetailV1 {
    /// Returns whether this section adds no terrain props.
    #[must_use]
    pub fn is_current(&self) -> bool {
        matches!(self, Self::Current)
    }

    /// Returns the stable matrix id, or `None` for the control.
    #[must_use]
    pub fn treatment_id(&self) -> Option<&'static str> {
        match self {
            Self::Current => None,
            Self::Boulders { density, cap: 160 } if same(*density, 0.0015) => {
                Some("props-01-boulders-low")
            }
            Self::Boulders { density, cap: 370 } if same(*density, 0.0035) => {
                Some("props-02-boulders-high")
            }
            Self::GrassLitter { density, cap: 320 } if same(*density, 0.0030) => {
                Some("props-03-litter-low")
            }
            Self::GrassLitter { density, cap: 690 } if same(*density, 0.0065) => {
                Some("props-04-litter-high")
            }
            // Focused amount comparisons remain separate from the original matrix.
            Self::GrassLitter { density, cap: 4000 } if same(*density, 0.04) => {
                Some("props-focused-litter-4pct")
            }
            Self::GrassLitter {
                density,
                cap: 12000,
            } if same(*density, 0.12) => Some("props-focused-litter-12pct"),
            Self::Mixed {
                boulder_density,
                tuft_density,
                deadwood_density,
                cap: 500,
            } if same(*boulder_density, 0.0012)
                && same(*tuft_density, 0.0030)
                && same(*deadwood_density, 0.0005) =>
            {
                Some("props-05-mixed")
            }
            Self::Clustered {
                center_density,
                pieces_min: 3,
                pieces_max: 5,
                cap: 600,
            } if same(*center_density, 0.0005) => Some("props-06-clustered"),
            _ => None,
        }
    }

    /// Validates the original matrix and the focused plant-density choices.
    pub fn validate(&self) -> Result<(), ReviewWorldDetailError> {
        match self {
            Self::Boulders { density, .. } | Self::GrassLitter { density, .. } => {
                finite_range("terrain_props", "density", *density, 0.0, 1.0)?;
            }
            Self::Mixed {
                boulder_density,
                tuft_density,
                deadwood_density,
                ..
            } => {
                finite_range(
                    "terrain_props",
                    "boulder_density",
                    *boulder_density,
                    0.0,
                    1.0,
                )?;
                finite_range("terrain_props", "tuft_density", *tuft_density, 0.0, 1.0)?;
                finite_range(
                    "terrain_props",
                    "deadwood_density",
                    *deadwood_density,
                    0.0,
                    1.0,
                )?;
            }
            Self::Clustered { center_density, .. } => {
                finite_range("terrain_props", "center_density", *center_density, 0.0, 1.0)?;
            }
            Self::Current => {}
        }
        valid_id("terrain_props", self.is_current(), self.treatment_id())
    }

    fn treatments() -> [Self; 6] {
        [
            Self::Boulders {
                density: 0.0015,
                cap: 160,
            },
            Self::Boulders {
                density: 0.0035,
                cap: 370,
            },
            Self::GrassLitter {
                density: 0.0030,
                cap: 320,
            },
            Self::GrassLitter {
                density: 0.0065,
                cap: 690,
            },
            Self::Mixed {
                boulder_density: 0.0012,
                tuft_density: 0.0030,
                deadwood_density: 0.0005,
                cap: 500,
            },
            Self::Clustered {
                center_density: 0.0005,
                pieces_min: 3,
                pieces_max: 5,
                cap: 600,
            },
        ]
    }
}

/// Review-only collisionless water-edge ice fringes.
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum IceFringeDetailV1 {
    /// Add no ice-fringe presentation.
    #[default]
    Current,
    /// Add fringes beside shore at or above one voxel level.
    LevelFringe {
        /// Minimum shoreline voxel level.
        minimum_level: u16,
        /// Fringe width in world units.
        width: f32,
        /// Eligible-edge coverage fraction.
        coverage: f32,
        /// Ice alpha.
        alpha: f32,
        /// Ice roughness.
        roughness: f32,
        /// Ice reflectance.
        reflectance: f32,
        /// Vertical presentation bias in world units.
        y_bias: f32,
    },
    /// Add fringes beside snow, optionally including Frozen biome shores.
    SnowAdjacent {
        /// Whether Frozen biome membership also makes a shore eligible.
        include_frozen: bool,
        /// Fringe width in world units.
        width: f32,
        /// Eligible-edge coverage fraction.
        coverage: f32,
        /// Ice alpha.
        alpha: f32,
        /// Ice roughness.
        roughness: f32,
        /// Ice reflectance.
        reflectance: f32,
        /// Vertical presentation bias in world units.
        y_bias: f32,
    },
    /// Add a wider Frozen-or-snow fringe with an inward alpha feather.
    Feathered {
        /// Whether Frozen biome membership also makes a shore eligible.
        include_frozen: bool,
        /// Fringe width in world units.
        width: f32,
        /// Eligible-edge coverage fraction.
        coverage: f32,
        /// Inward feather width in world units.
        inward_feather: f32,
        /// Ice alpha.
        alpha: f32,
        /// Ice roughness.
        roughness: f32,
        /// Ice reflectance.
        reflectance: f32,
        /// Vertical presentation bias in world units.
        y_bias: f32,
    },
}

impl IceFringeDetailV1 {
    /// Returns whether this section adds no ice fringes.
    #[must_use]
    pub fn is_current(&self) -> bool {
        matches!(self, Self::Current)
    }

    /// Returns the stable matrix id, or `None` for the control.
    #[must_use]
    pub fn treatment_id(&self) -> Option<&'static str> {
        match self {
            Self::Current => None,
            Self::LevelFringe {
                minimum_level: 140,
                width,
                coverage,
                alpha,
                roughness,
                reflectance,
                y_bias,
            } if ice_material(*alpha, *roughness, *reflectance, *y_bias) => {
                if same(*width, 0.15) && same(*coverage, 0.40) {
                    Some("ice-01-level-narrow")
                } else if same(*width, 0.30) && same(*coverage, 0.65) {
                    Some("ice-02-level-medium")
                } else if same(*width, 0.45) && same(*coverage, 0.85) {
                    Some("ice-03-level-wide")
                } else {
                    None
                }
            }
            Self::SnowAdjacent {
                include_frozen: false,
                width,
                coverage,
                alpha,
                roughness,
                reflectance,
                y_bias,
            } if same(*width, 0.25)
                && same(*coverage, 0.65)
                && ice_material(*alpha, *roughness, *reflectance, *y_bias) =>
            {
                Some("ice-04-snow-adjacent")
            }
            Self::SnowAdjacent {
                include_frozen: true,
                width,
                coverage,
                alpha,
                roughness,
                reflectance,
                y_bias,
            } if same(*width, 0.25)
                && same(*coverage, 0.65)
                && ice_material(*alpha, *roughness, *reflectance, *y_bias) =>
            {
                Some("ice-05-frozen-or-snow")
            }
            Self::Feathered {
                include_frozen: true,
                width,
                coverage,
                inward_feather,
                alpha,
                roughness,
                reflectance,
                y_bias,
            } if same(*width, 0.35)
                && same(*coverage, 0.75)
                && same(*inward_feather, 0.10)
                && ice_material(*alpha, *roughness, *reflectance, *y_bias) =>
            {
                Some("ice-06-frozen-or-snow-feathered")
            }
            _ => None,
        }
    }

    /// Validates this section against the six fixed ice treatments.
    pub fn validate(&self) -> Result<(), ReviewWorldDetailError> {
        match self {
            Self::LevelFringe {
                width,
                coverage,
                alpha,
                roughness,
                reflectance,
                y_bias,
                ..
            }
            | Self::SnowAdjacent {
                width,
                coverage,
                alpha,
                roughness,
                reflectance,
                y_bias,
                ..
            } => {
                validate_ice(*width, *coverage, *alpha, *roughness, *reflectance, *y_bias)?;
            }
            Self::Feathered {
                width,
                coverage,
                inward_feather,
                alpha,
                roughness,
                reflectance,
                y_bias,
                ..
            } => {
                validate_ice(*width, *coverage, *alpha, *roughness, *reflectance, *y_bias)?;
                finite_range("ice_fringe", "inward_feather", *inward_feather, 0.0, 1.0)?;
            }
            Self::Current => {}
        }
        valid_id("ice_fringe", self.is_current(), self.treatment_id())
    }

    fn treatments() -> [Self; 6] {
        [
            Self::LevelFringe {
                minimum_level: 140,
                width: 0.15,
                coverage: 0.40,
                alpha: 0.82,
                roughness: 0.32,
                reflectance: 0.30,
                y_bias: 0.006,
            },
            Self::LevelFringe {
                minimum_level: 140,
                width: 0.30,
                coverage: 0.65,
                alpha: 0.82,
                roughness: 0.32,
                reflectance: 0.30,
                y_bias: 0.006,
            },
            Self::LevelFringe {
                minimum_level: 140,
                width: 0.45,
                coverage: 0.85,
                alpha: 0.82,
                roughness: 0.32,
                reflectance: 0.30,
                y_bias: 0.006,
            },
            Self::SnowAdjacent {
                include_frozen: false,
                width: 0.25,
                coverage: 0.65,
                alpha: 0.82,
                roughness: 0.32,
                reflectance: 0.30,
                y_bias: 0.006,
            },
            Self::SnowAdjacent {
                include_frozen: true,
                width: 0.25,
                coverage: 0.65,
                alpha: 0.82,
                roughness: 0.32,
                reflectance: 0.30,
                y_bias: 0.006,
            },
            Self::Feathered {
                include_frozen: true,
                width: 0.35,
                coverage: 0.75,
                inward_feather: 0.10,
                alpha: 0.82,
                roughness: 0.32,
                reflectance: 0.30,
                y_bias: 0.006,
            },
        ]
    }
}

/// Named anchor class used by deterministic local fog placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalFogPlacementV1 {
    /// Named water and waterfall anchors.
    WaterHugging,
    /// Named valley-floor anchors.
    ValleyFloor,
    /// Deterministic mix of named water and valley anchors.
    Mixed,
}

/// Review-only localized fog volumes.
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LocalFogDetailV1 {
    /// Add no local fog volumes.
    #[default]
    Current,
    /// Place deterministic fog volumes at a named anchor class.
    Layer {
        /// Anchor class used for placement.
        placement: LocalFogPlacementV1,
        /// Minimum deterministic volume radius in world units.
        radius_min: f32,
        /// Maximum deterministic volume radius in world units.
        radius_max: f32,
        /// Volume height in world units.
        height: f32,
        /// Fraction of eligible named anchors selected as exact `ceil(count * coverage)`.
        coverage: f32,
        /// Fog density/opacity scalar.
        opacity: f32,
        /// Bottom offset above the local surface in world units.
        bottom_offset: f32,
    },
}

impl LocalFogDetailV1 {
    /// Returns whether this section adds no local fog volumes.
    #[must_use]
    pub fn is_current(&self) -> bool {
        matches!(self, Self::Current)
    }

    /// Returns the stable matrix id, or `None` for the control.
    #[must_use]
    pub fn treatment_id(&self) -> Option<&'static str> {
        match self {
            Self::Current => None,
            Self::Layer {
                placement,
                radius_min,
                radius_max,
                height,
                coverage,
                opacity,
                bottom_offset,
            } if same(*bottom_offset, 0.15) => {
                let values = (*radius_min, *radius_max, *height, *coverage, *opacity);
                match placement {
                    LocalFogPlacementV1::WaterHugging
                        if fog_values(values, (12.0, 18.0, 1.4, 0.10, 0.06)) =>
                    {
                        Some("fog-01-water-light")
                    }
                    LocalFogPlacementV1::WaterHugging
                        if fog_values(values, (20.0, 30.0, 2.8, 0.20, 0.10)) =>
                    {
                        Some("fog-02-water-heavy")
                    }
                    LocalFogPlacementV1::ValleyFloor
                        if fog_values(values, (14.0, 22.0, 1.8, 0.12, 0.06)) =>
                    {
                        Some("fog-03-valley-light")
                    }
                    LocalFogPlacementV1::ValleyFloor
                        if fog_values(values, (24.0, 36.0, 3.5, 0.24, 0.10)) =>
                    {
                        Some("fog-04-valley-heavy")
                    }
                    LocalFogPlacementV1::Mixed
                        if fog_values(values, (16.0, 26.0, 2.4, 0.16, 0.07)) =>
                    {
                        Some("fog-05-mixed")
                    }
                    LocalFogPlacementV1::Mixed
                        if fog_values(values, (28.0, 42.0, 4.5, 0.28, 0.12)) =>
                    {
                        Some("fog-06-mixed-cinematic")
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Validates this section against the six fixed fog treatments.
    pub fn validate(&self) -> Result<(), ReviewWorldDetailError> {
        if let Self::Layer {
            radius_min,
            radius_max,
            height,
            coverage,
            opacity,
            bottom_offset,
            ..
        } = self
        {
            finite_range("local_fog", "radius_min", *radius_min, 0.0, 100.0)?;
            finite_range("local_fog", "radius_max", *radius_max, 0.0, 100.0)?;
            finite_range("local_fog", "height", *height, 0.0, 20.0)?;
            finite_range("local_fog", "coverage", *coverage, 0.0, 1.0)?;
            finite_range("local_fog", "opacity", *opacity, 0.0, 1.0)?;
            finite_range("local_fog", "bottom_offset", *bottom_offset, 0.0, 2.0)?;
            if radius_min > radius_max {
                return Err(ReviewWorldDetailError::setting(
                    "local_fog",
                    "radius_min must not exceed radius_max",
                ));
            }
        }
        valid_id("local_fog", self.is_current(), self.treatment_id())
    }

    fn treatments() -> [Self; 6] {
        [
            fog(
                LocalFogPlacementV1::WaterHugging,
                12.0,
                18.0,
                1.4,
                0.10,
                0.06,
            ),
            fog(
                LocalFogPlacementV1::WaterHugging,
                20.0,
                30.0,
                2.8,
                0.20,
                0.10,
            ),
            fog(
                LocalFogPlacementV1::ValleyFloor,
                14.0,
                22.0,
                1.8,
                0.12,
                0.06,
            ),
            fog(
                LocalFogPlacementV1::ValleyFloor,
                24.0,
                36.0,
                3.5,
                0.24,
                0.10,
            ),
            fog(LocalFogPlacementV1::Mixed, 16.0, 26.0, 2.4, 0.16, 0.07),
            fog(LocalFogPlacementV1::Mixed, 28.0, 42.0, 4.5, 0.28, 0.12),
        ]
    }
}

const fn fog(
    placement: LocalFogPlacementV1,
    radius_min: f32,
    radius_max: f32,
    height: f32,
    coverage: f32,
    opacity: f32,
) -> LocalFogDetailV1 {
    LocalFogDetailV1::Layer {
        placement,
        radius_min,
        radius_max,
        height,
        coverage,
        opacity,
        bottom_offset: 0.15,
    }
}

/// Authority identities that must remain unchanged by every review treatment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewAuthorityFingerprintsV1 {
    /// Exact `VoxelMap` fingerprint.
    pub voxel_map: String,
    /// Structural-plan fingerprint.
    pub structural: String,
    /// Materialized-world fingerprint.
    pub materialized: String,
    /// Logical liquid graph fingerprint.
    pub liquid_graph: String,
    /// World topology fingerprint.
    pub topology: String,
    /// Traversal projection fingerprint.
    pub traversal: String,
    /// Traversal-blocker fingerprint.
    pub blockers: String,
    /// Gameplay-anchor fingerprint.
    pub anchors: String,
    /// Biome projection fingerprint.
    pub biomes: String,
    /// Authoritative feature-root fingerprint.
    pub feature_roots: String,
    /// Logical terrain and picking-tuple fingerprint.
    pub logical_terrain_picking: String,
    /// Gameplay time, illumination, fog-of-war, save, and replication fingerprint.
    pub gameplay_state: String,
}

impl ReviewAuthorityFingerprintsV1 {
    fn validate(&self) -> Result<(), ReviewWorldDetailError> {
        for (field, value) in [
            ("voxel_map", self.voxel_map.as_str()),
            ("structural", self.structural.as_str()),
            ("materialized", self.materialized.as_str()),
            ("liquid_graph", self.liquid_graph.as_str()),
            ("topology", self.topology.as_str()),
            ("traversal", self.traversal.as_str()),
            ("blockers", self.blockers.as_str()),
            ("anchors", self.anchors.as_str()),
            ("biomes", self.biomes.as_str()),
            ("feature_roots", self.feature_roots.as_str()),
            (
                "logical_terrain_picking",
                self.logical_terrain_picking.as_str(),
            ),
            ("gameplay_state", self.gameplay_state.as_str()),
        ] {
            if !is_lower_hex(value, 16) {
                return Err(ReviewWorldDetailError::new(format!(
                    "authority.{field} must be 16 lowercase hexadecimal characters"
                )));
            }
        }
        Ok(())
    }
}

/// Geometry and allocation counts for one disposable presentation layer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewPresentationCountsV1 {
    /// Spawned disposable ECS entities.
    pub entities: u64,
    /// Distinct shared material assets.
    pub materials: u64,
    /// Mesh vertices.
    pub vertices: u64,
    /// Mesh triangles.
    pub triangles: u64,
}

impl std::ops::Add for ReviewPresentationCountsV1 {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            entities: self.entities.saturating_add(other.entities),
            materials: self.materials.saturating_add(other.materials),
            vertices: self.vertices.saturating_add(other.vertices),
            triangles: self.triangles.saturating_add(other.triangles),
        }
    }
}

/// Exact total and per-family counts published for one capture.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewWorldDetailCountsV1 {
    /// Sum of the nine family counts.
    pub total: ReviewPresentationCountsV1,
    /// Snow presentation counts.
    pub snow: ReviewPresentationCountsV1,
    /// Water presentation counts.
    pub water: ReviewPresentationCountsV1,
    /// World-space cloud presentation counts.
    pub physical_clouds: ReviewPresentationCountsV1,
    /// Shore and waterfall presentation counts.
    pub shore_and_falls: ReviewPresentationCountsV1,
    /// Alpine vegetation presentation counts.
    pub alpine_vegetation: ReviewPresentationCountsV1,
    /// Cliff/strata presentation counts.
    pub cliff_strata: ReviewPresentationCountsV1,
    /// Terrain-prop presentation counts.
    pub terrain_props: ReviewPresentationCountsV1,
    /// Ice-fringe presentation counts.
    pub ice_fringe: ReviewPresentationCountsV1,
    /// Local-fog presentation counts.
    pub local_fog: ReviewPresentationCountsV1,
}

impl ReviewWorldDetailCountsV1 {
    /// Computes the saturating sum of all nine family rows.
    #[must_use]
    pub fn computed_total(&self) -> ReviewPresentationCountsV1 {
        self.snow
            + self.water
            + self.physical_clouds
            + self.shore_and_falls
            + self.alpine_vegetation
            + self.cliff_strata
            + self.terrain_props
            + self.ice_fringe
            + self.local_fog
    }
}

/// Camera renderer features resolved for one review capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewCameraFeaturesV1 {
    /// Whether order-independent transparency is configured on the camera and
    /// supported by the live render adapter/device capability preflight.
    pub oit: bool,
    /// Whether medium-quality screen-space transmission is active.
    pub medium_transmission: bool,
    /// Whether a depth texture is available to review materials.
    pub depth_texture: bool,
    /// Whether volumetric camera processing is active.
    pub volumetrics: bool,
}

/// Runtime cost sample attached to each genuine rendered capture.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewPerformanceSampleV1 {
    /// Duration of the immediately preceding rendered frame in milliseconds.
    pub frame_time_ms: f32,
    /// Total resident presentation asset bytes for the matched live scene.
    pub resident_presentation_bytes: u64,
    /// Whether the capture was emitted only after the configured warm-up window.
    pub warmup_complete: bool,
}

impl ReviewPerformanceSampleV1 {
    fn validate(&self) -> Result<(), ReviewWorldDetailError> {
        if !self.frame_time_ms.is_finite() || self.frame_time_ms < 0.0 {
            return Err(ReviewWorldDetailError::new(
                "performance.frame_time_ms must be finite and nonnegative",
            ));
        }
        if self.warmup_complete
            && (self.frame_time_ms <= 0.0 || self.resident_presentation_bytes == 0)
        {
            return Err(ReviewWorldDetailError::new(
                "a warm performance sample requires positive frame time and resident bytes",
            ));
        }
        Ok(())
    }
}

/// Teardown and state-restoration evidence for one review capture lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewCleanupStateV1 {
    /// Number of completed review enter/exit cycles represented by the report.
    pub completed_cycles: u16,
    /// Disposable review entities remaining after teardown.
    pub entities_remaining: u64,
    /// Disposable review materials remaining after teardown.
    pub materials_remaining: u64,
    /// Disposable review meshes remaining after teardown.
    pub meshes_remaining: u64,
    /// Temporary review render-target images remaining after teardown.
    pub target_images_remaining: u64,
    /// Whether camera feature state was restored exactly.
    pub camera_state_restored: bool,
    /// Whether OIT state was restored exactly.
    pub oit_state_restored: bool,
    /// Whether transmission state was restored exactly.
    pub transmission_state_restored: bool,
    /// Whether depth state was restored exactly.
    pub depth_state_restored: bool,
    /// Whether volumetric state was restored exactly.
    pub volumetric_state_restored: bool,
}

impl ReviewCleanupStateV1 {
    /// Returns whether teardown left no disposable allocations and restored all state.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.entities_remaining == 0
            && self.materials_remaining == 0
            && self.meshes_remaining == 0
            && self.target_images_remaining == 0
            && self.camera_state_restored
            && self.oit_state_restored
            && self.transmission_state_restored
            && self.depth_state_restored
            && self.volumetric_state_restored
    }
}

/// Authoritative namespace from which one named review anchor was resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewAnchorClassV1 {
    /// A standable anchor from the gameplay-owned placement namespace.
    Gameplay,
    /// A scenic anchor from the non-walkability-bearing observation namespace.
    Observation,
}

/// Stable identities of the two pure planners and their renderer-facing mesh streams.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewWorldDetailProjectionHashesV1 {
    /// Stable hash of the terrain-detail planner output.
    pub terrain_plan: String,
    /// Stable hash of the liquid and atmosphere planner output.
    pub liquid_atmosphere_plan: String,
    /// Stable hash of the renderer-facing canonical mesh streams.
    pub mesh_projection: String,
}

/// Read-only evidence for renderer-owned asset types that external review
/// samplers cannot name directly.
///
/// This resource is live only while a disposable world-detail projection is
/// committed. It does not participate in profile, plan, or authority hashes.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReviewWorldDetailRuntimeAssetEvidenceV1 {
    /// Number of distinct live ordinary liquid material assets.
    pub liquid_material_count: u64,
    /// Exact inline allocation bytes of those ordinary liquid materials.
    pub liquid_material_bytes: u64,
    /// Number of distinct live extended review-water material assets.
    pub review_water_material_count: u64,
    /// Exact inline allocation bytes of those review-water materials.
    pub review_water_material_bytes: u64,
    /// Number of distinct live review-owned fog density images.
    pub fog_density_image_count: u64,
    /// Exact owned pixel-payload bytes of those fog density images.
    pub fog_density_image_bytes: u64,
}

/// Exact post-teardown receipt for renderer allocations owned by one disposable
/// review projection.
///
/// The renderer publishes this only after its deferred entity cleanup has
/// completed and clears it before constructing the next projection.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReviewWorldDetailTeardownReceiptV1 {
    /// Review-marker entities that still exist after cleanup.
    pub review_entities_remaining: u64,
    /// Owned standard-material assets that remain live after removal.
    pub standard_materials_remaining: u64,
    /// Owned mesh assets that remain live after removal.
    pub meshes_remaining: u64,
    /// Owned extended review-water material assets that remain live after removal.
    pub review_water_materials_remaining: u64,
    /// Owned fog-density image assets that remain live after removal.
    pub fog_density_images_remaining: u64,
    /// Ordinary terrain-material bindings that are missing or differ from their
    /// exact pre-review handles after restoration.
    pub terrain_material_overrides_remaining: u64,
    /// Ordinary liquid material bindings that are missing or differ from their
    /// exact pre-review handles. The V1 field spelling is retained for report
    /// compatibility; review suppression leaves mesh visibility and picking intact.
    pub liquid_visibility_overrides_remaining: u64,
    /// Existing vegetation render children that are missing or retain a
    /// review-time scale after restoration.
    pub vegetation_scale_overrides_remaining: u64,
}

/// One-shot request to tear down and verify the disposable projection while
/// retaining the loaded authoritative map.
///
/// The renderer removes this resource when servicing it. If the profile remains
/// present, the ordinary update path recreates the projection on the next frame.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReviewWorldDetailTeardownRequestV1;

impl ReviewWorldDetailProjectionHashesV1 {
    fn validate(&self) -> Result<(), ReviewWorldDetailError> {
        for (field, value) in [
            ("terrain_plan", self.terrain_plan.as_str()),
            (
                "liquid_atmosphere_plan",
                self.liquid_atmosphere_plan.as_str(),
            ),
            ("mesh_projection", self.mesh_projection.as_str()),
        ] {
            if !is_lower_hex(value, 16) {
                return Err(ReviewWorldDetailError::new(format!(
                    "projection_hashes.{field} must be 16 lowercase hexadecimal characters"
                )));
            }
        }
        Ok(())
    }
}

/// Deterministic sampling evidence for one physical-cloud treatment.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewCloudCoverageEvidenceV1 {
    /// Radius of the circular massif cloud field in world units.
    pub field_radius: f32,
    /// Requested projected rendered-puff silhouette-union coverage fraction.
    pub target_fraction: f32,
    /// Measured projected rendered-puff silhouette-union coverage fraction.
    pub measured_fraction: f32,
    /// Exact accepted absolute error bound (`0.01`) from the target fraction.
    pub tolerance: f32,
    /// Number of deterministic in-field samples used by the measurement.
    pub sample_count: u64,
    /// Number of cloud clusters whose rendered puff silhouettes form the measured union.
    pub cloud_clusters: u32,
    /// Whether this altitude treatment is required to intersect the massif peak.
    pub peak_intersection_required: bool,
    /// Number of emitted puffs whose XZ silhouette and vertical extent overlap
    /// the exact solid peak column selected by the renderer-neutral input.
    pub peak_intersecting_puffs: u32,
}

impl ReviewCloudCoverageEvidenceV1 {
    fn validate(&self) -> Result<(), ReviewWorldDetailError> {
        for (field, value) in [
            ("field_radius", self.field_radius),
            ("target_fraction", self.target_fraction),
            ("measured_fraction", self.measured_fraction),
            ("tolerance", self.tolerance),
        ] {
            if !value.is_finite() {
                return Err(ReviewWorldDetailError::new(format!(
                    "effect_validation.cloud_coverage.{field} must be finite"
                )));
            }
        }
        if self.field_radius <= 0.0
            || !(0.0..=1.0).contains(&self.target_fraction)
            || !(0.0..=1.0).contains(&self.measured_fraction)
            || !same(self.tolerance, 0.01)
            || self.sample_count == 0
            || self.cloud_clusters == 0
            || (self.peak_intersection_required && self.peak_intersecting_puffs == 0)
            || (self.measured_fraction - self.target_fraction).abs() > self.tolerance
        {
            return Err(ReviewWorldDetailError::new(
                "effect_validation.cloud_coverage is outside its deterministic contract",
            ));
        }
        Ok(())
    }
}

/// Measured spatial-occupancy evidence for one local-fog treatment.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewFogCoverageEvidenceV1 {
    /// Requested fraction of the deterministic XZ density footprint.
    pub target_fraction: f32,
    /// Exact measured fraction of admitted footprint samples.
    pub measured_fraction: f32,
    /// Number of in-footprint samples in the shared density texture.
    pub sample_count: u32,
    /// Exact deterministic ceiling-sized sample subset carrying fog density.
    pub active_samples: u32,
    /// Number of named-anchor volumes using the measured density footprint.
    pub fog_volumes: u32,
}

impl ReviewFogCoverageEvidenceV1 {
    fn validate(&self) -> Result<(), ReviewWorldDetailError> {
        if !self.target_fraction.is_finite()
            || !self.measured_fraction.is_finite()
            || !(0.0..=1.0).contains(&self.target_fraction)
            || !(0.0..=1.0).contains(&self.measured_fraction)
            || self.sample_count == 0
            || self.fog_volumes == 0
        {
            return Err(ReviewWorldDetailError::new(
                "effect_validation.fog_coverage is outside its deterministic contract",
            ));
        }
        let expected = (f64::from(self.sample_count) * f64::from(self.target_fraction))
            .ceil()
            .min(f64::from(self.sample_count));
        #[expect(
            clippy::cast_precision_loss,
            reason = "Evidence compares the renderer's f32 sample fraction with the same intentional u32-to-f32 rounding"
        )]
        let measured_fraction = self.active_samples as f32 / self.sample_count as f32;
        if f64::from(self.active_samples).to_bits() != expected.to_bits()
            || self.measured_fraction.to_bits() != measured_fraction.to_bits()
        {
            return Err(ReviewWorldDetailError::new(
                "effect_validation.fog_coverage must report its exact ceiling-sized density subset",
            ));
        }
        Ok(())
    }
}

/// Exact subset-size evidence for one ice-fringe treatment.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewIceCoverageEvidenceV1 {
    /// Requested fraction of eligible shoreline edges.
    pub target_fraction: f32,
    /// Number of edges admitted by the treatment's level/biome rule.
    pub eligible_edges: u32,
    /// Exact deterministic ceiling-sized subset that was meshed.
    pub selected_edges: u32,
}

/// Exact authored-to-liquid binding used by one plunge-spray treatment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewWaterfallAnchorEvidenceV1 {
    /// Stable authored anchor name.
    pub anchor_name: String,
    /// Authored dry review footing as `[q, r, level]`.
    pub anchor_position: [i32; 3],
    /// Resolved published Fall downstream as `[q, r, level]`.
    pub landing_position: [i32; 3],
    /// Exact axial distance from authored footing to liquid landing.
    pub distance_hexes: u32,
}

fn axial_distance(first: [i32; 3], second: [i32; 3]) -> u64 {
    // This is `HexCoord::distance` in axial q/r space, widened before subtraction so
    // malformed evidence at the i32 limits is rejected instead of overflowing.
    let delta_q = i64::from(second[0]) - i64::from(first[0]);
    let delta_r = i64::from(second[1]) - i64::from(first[1]);
    delta_q
        .unsigned_abs()
        .max(delta_r.unsigned_abs())
        .max((delta_q + delta_r).unsigned_abs())
}

impl ReviewIceCoverageEvidenceV1 {
    fn validate(&self) -> Result<(), ReviewWorldDetailError> {
        if !self.target_fraction.is_finite()
            || !(0.0..=1.0).contains(&self.target_fraction)
            || self.eligible_edges == 0
        {
            return Err(ReviewWorldDetailError::new(
                "effect_validation.ice_coverage is outside its deterministic contract",
            ));
        }
        let expected = (f64::from(self.eligible_edges) * f64::from(self.target_fraction)).ceil();
        if f64::from(self.selected_edges).to_bits()
            != expected.min(f64::from(self.eligible_edges)).to_bits()
        {
            return Err(ReviewWorldDetailError::new(
                "effect_validation.ice_coverage selected_edges must equal ceil(eligible_edges * target_fraction)",
            ));
        }
        Ok(())
    }
}

/// Pure-planner evidence for semantic coverage and named effect ownership.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewWorldDetailEffectValidationV1 {
    /// Measured rendered-puff XZ silhouette coverage, absent for the cloud control.
    pub cloud_coverage: Option<ReviewCloudCoverageEvidenceV1>,
    /// Exact selected/eligible edge cardinality, absent for the ice control.
    pub ice_coverage: Option<ReviewIceCoverageEvidenceV1>,
    /// Exact density-footprint occupancy, absent for the local-fog control.
    pub fog_coverage: Option<ReviewFogCoverageEvidenceV1>,
    /// Sorted unique authored waterfall-anchor bindings used for plunge spray.
    pub waterfall_anchors: Vec<ReviewWaterfallAnchorEvidenceV1>,
}

impl ReviewWorldDetailEffectValidationV1 {
    fn validate(&self) -> Result<(), ReviewWorldDetailError> {
        if let Some(cloud) = self.cloud_coverage {
            cloud.validate()?;
        }
        if let Some(ice) = self.ice_coverage {
            ice.validate()?;
        }
        if let Some(fog) = self.fog_coverage {
            fog.validate()?;
        }
        if self.waterfall_anchors.iter().any(|binding| {
            binding.anchor_name.is_empty()
                || binding.distance_hexes > 13
                || u64::from(binding.distance_hexes)
                    != axial_distance(binding.anchor_position, binding.landing_position)
                || binding.anchor_position == binding.landing_position
        }) || self.waterfall_anchors.windows(2).any(|bindings| {
            let [first, second] = bindings else {
                return true;
            };
            first.anchor_name >= second.anchor_name
        }) {
            return Err(ReviewWorldDetailError::new(
                "effect_validation.waterfall_anchors must be sorted, unique, report exact axial distance, and remain within the authored displacement bound",
            ));
        }
        Ok(())
    }
}

/// Deterministic review-only evidence published for every capture.
#[derive(Resource, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewWorldDetailReportV1 {
    /// Exact report schema version. Only version 1 is accepted.
    pub version: u16,
    /// Runtime-authored binding to this automated capture launch and its inputs.
    pub runtime_receipt: ReviewRuntimeReceiptV1,
    /// Lowercase hexadecimal SHA-256 of the resolved canonical profile JSON.
    pub profile_hash_sha256: String,
    /// Unchanged gameplay/world-authority identities.
    pub authority: ReviewAuthorityFingerprintsV1,
    /// Pure-plan and renderer-facing projection identities for reproduction checks.
    pub projection_hashes: ReviewWorldDetailProjectionHashesV1,
    /// Pure-planner coverage measurements and exact named-effect ownership.
    pub effect_validation: ReviewWorldDetailEffectValidationV1,
    /// Total and per-family render allocation/geometry counts.
    pub counts: ReviewWorldDetailCountsV1,
    /// Named anchor world heights, ordered lexicographically by anchor name.
    pub anchor_heights: BTreeMap<String, f32>,
    /// Namespace/class of every name in [`Self::anchor_heights`].
    pub anchor_classes: BTreeMap<String, ReviewAnchorClassV1>,
    /// Camera renderer features resolved for the capture.
    pub camera_features: ReviewCameraFeaturesV1,
    /// Post-warm-up runtime performance and resident presentation memory sample.
    pub performance: ReviewPerformanceSampleV1,
    /// Teardown and renderer-state restoration evidence.
    pub cleanup: ReviewCleanupStateV1,
}

impl ReviewWorldDetailReportV1 {
    /// Validates the report schema, hash spelling, finite anchors, and count sum.
    pub fn validate(&self) -> Result<(), ReviewWorldDetailError> {
        if self.version != REVIEW_WORLD_DETAIL_REPORT_VERSION_V1 {
            return Err(ReviewWorldDetailError::new(format!(
                "unsupported review report version {}; expected {}",
                self.version, REVIEW_WORLD_DETAIL_REPORT_VERSION_V1
            )));
        }
        self.runtime_receipt.validate()?;
        if self.profile_hash_sha256.len() != 64
            || !self
                .profile_hash_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ReviewWorldDetailError::new(
                "profile_hash_sha256 must be 64 lowercase hexadecimal characters",
            ));
        }
        if self.runtime_receipt.profile_sha256 != self.profile_hash_sha256 {
            return Err(ReviewWorldDetailError::new(
                "runtime receipt profile_sha256 does not match report profile_hash_sha256",
            ));
        }
        self.authority.validate()?;
        self.projection_hashes.validate()?;
        self.effect_validation.validate()?;
        self.performance.validate()?;
        for (anchor, height) in &self.anchor_heights {
            if anchor.is_empty() {
                return Err(ReviewWorldDetailError::new("anchor name must not be empty"));
            }
            if !height.is_finite() {
                return Err(ReviewWorldDetailError::new(format!(
                    "anchor height for {anchor} must be finite"
                )));
            }
        }
        if self.anchor_heights.keys().ne(self.anchor_classes.keys()) {
            return Err(ReviewWorldDetailError::new(
                "anchor_heights and anchor_classes must contain the same ordered key set",
            ));
        }
        if self.counts.total != self.counts.computed_total() {
            return Err(ReviewWorldDetailError::new(
                "review presentation total does not equal the nine family rows",
            ));
        }
        Ok(())
    }

    /// Serializes a validated report to compact deterministic JSON.
    pub fn canonical_json(&self) -> Result<String, ReviewWorldDetailError> {
        self.validate()?;
        serde_json::to_string(self).map_err(|error| {
            ReviewWorldDetailError::new(format!("could not serialize review report: {error}"))
        })
    }
}

fn valid_id(
    family: &str,
    is_current: bool,
    treatment_id: Option<&str>,
) -> Result<(), ReviewWorldDetailError> {
    if is_current || treatment_id.is_some() {
        Ok(())
    } else {
        Err(ReviewWorldDetailError::setting(
            family,
            "parameters do not name one of the fixed experiment treatments",
        ))
    }
}

fn is_lower_hex(value: &str, expected_length: usize) -> bool {
    value.len() == expected_length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn finite_range(
    family: &str,
    field: &str,
    value: f32,
    minimum: f32,
    maximum: f32,
) -> Result<(), ReviewWorldDetailError> {
    if !value.is_finite() {
        return Err(ReviewWorldDetailError::setting(
            family,
            format!("{field} must be finite"),
        ));
    }
    if value < minimum || value > maximum {
        return Err(ReviewWorldDetailError::setting(
            family,
            format!("{field} must be in [{minimum}, {maximum}]"),
        ));
    }
    Ok(())
}

fn same(left: f32, right: f32) -> bool {
    left.to_bits() == right.to_bits()
}

fn scale_values(
    h_min: f32,
    h_max: f32,
    v_min: f32,
    v_max: f32,
    expected_h_min: f32,
    expected_h_max: f32,
    expected_v_min: f32,
    expected_v_max: f32,
) -> bool {
    same(h_min, expected_h_min)
        && same(h_max, expected_h_max)
        && same(v_min, expected_v_min)
        && same(v_max, expected_v_max)
}

fn validate_scale(
    horizontal_min: f32,
    horizontal_max: f32,
    vertical_min: f32,
    vertical_max: f32,
) -> Result<(), ReviewWorldDetailError> {
    finite_range(
        "alpine_vegetation",
        "horizontal_min",
        horizontal_min,
        0.1,
        4.0,
    )?;
    finite_range(
        "alpine_vegetation",
        "horizontal_max",
        horizontal_max,
        0.1,
        4.0,
    )?;
    finite_range("alpine_vegetation", "vertical_min", vertical_min, 0.1, 4.0)?;
    finite_range("alpine_vegetation", "vertical_max", vertical_max, 0.1, 4.0)?;
    if horizontal_min > horizontal_max || vertical_min > vertical_max {
        return Err(ReviewWorldDetailError::setting(
            "alpine_vegetation",
            "minimum scale must not exceed maximum scale",
        ));
    }
    Ok(())
}

fn validate_dust(upper_fraction: f32, shell_height: f32) -> Result<(), ReviewWorldDetailError> {
    finite_range(
        "alpine_vegetation",
        "upper_fraction",
        upper_fraction,
        0.0,
        1.0,
    )?;
    finite_range("alpine_vegetation", "shell_height", shell_height, 0.0, 1.0)
}

fn validate_cloud_geometry(
    projected_coverage: f32,
    diameter_min: f32,
    diameter_max: f32,
) -> Result<(), ReviewWorldDetailError> {
    finite_range(
        "physical_clouds",
        "projected_coverage",
        projected_coverage,
        0.0,
        1.0,
    )?;
    finite_range(
        "physical_clouds",
        "diameter_min",
        diameter_min,
        0.1,
        1_000.0,
    )?;
    finite_range(
        "physical_clouds",
        "diameter_max",
        diameter_max,
        0.1,
        1_000.0,
    )?;
    if diameter_min > diameter_max {
        return Err(ReviewWorldDetailError::setting(
            "physical_clouds",
            "diameter_min must not exceed diameter_max",
        ));
    }
    Ok(())
}

fn ice_material(alpha: f32, roughness: f32, reflectance: f32, y_bias: f32) -> bool {
    same(alpha, 0.82) && same(roughness, 0.32) && same(reflectance, 0.30) && same(y_bias, 0.006)
}

fn validate_ice(
    width: f32,
    coverage: f32,
    alpha: f32,
    roughness: f32,
    reflectance: f32,
    y_bias: f32,
) -> Result<(), ReviewWorldDetailError> {
    finite_range("ice_fringe", "width", width, 0.0, 2.0)?;
    finite_range("ice_fringe", "coverage", coverage, 0.0, 1.0)?;
    finite_range("ice_fringe", "alpha", alpha, 0.0, 1.0)?;
    finite_range("ice_fringe", "roughness", roughness, 0.0, 1.0)?;
    finite_range("ice_fringe", "reflectance", reflectance, 0.0, 1.0)?;
    finite_range("ice_fringe", "y_bias", y_bias, -1.0, 1.0)
}

fn fog_values(values: (f32, f32, f32, f32, f32), expected: (f32, f32, f32, f32, f32)) -> bool {
    same(values.0, expected.0)
        && same(values.1, expected.1)
        && same(values.2, expected.2)
        && same(values.3, expected.3)
        && same(values.4, expected.4)
}

fn hex_lower(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        if write!(&mut output, "{byte:02x}").is_err() {
            return String::new();
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_runtime_receipt(profile_sha256: &str) -> ReviewRuntimeReceiptV1 {
        ReviewRuntimeReceiptV1::new(
            "a".repeat(64),
            42,
            "b".repeat(64),
            "c".repeat(64),
            "d".repeat(64),
            profile_sha256.to_owned(),
        )
        .expect("valid fixture receipt must hash")
    }

    #[test]
    fn default_is_all_current_and_canonical() {
        let profile = ReviewWorldDetailProfileV1::default();
        assert!(profile.is_current());
        assert_eq!(profile.active_treatment_ids(), Vec::<&str>::new());
        let json = profile.canonical_json().expect("control must serialize");
        assert_eq!(
            json,
            r#"{"version":1,"snow":{"kind":"current"},"water":{"kind":"current"},"physical_clouds":{"kind":"current"},"shore_and_falls":{"kind":"current"},"alpine_vegetation":{"kind":"current"},"cliff_strata":{"kind":"current"},"terrain_props":{"kind":"current"},"ice_fringe":{"kind":"current"},"local_fog":{"kind":"current"}}"#
        );
        assert_eq!(
            profile.profile_hash_sha256().expect("control must hash"),
            "c962b67a10570c64e4515780f4c9704ad41099851c8f928e27886e4eb8a7db8b"
        );
    }

    #[test]
    fn runtime_receipt_hashes_exact_ordered_json_and_rejects_tampering() {
        let mut receipt = fixture_runtime_receipt(&"e".repeat(64));
        let body = format!(
            r#"{{"version":1,"launch_nonce":"{}","process_id":42,"executable_sha256":"{}","source_provenance_sha256":"{}","capture_plan_sha256":"{}","profile_sha256":"{}"}}"#,
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64),
            "d".repeat(64),
            "e".repeat(64),
        );
        assert_eq!(
            receipt.receipt_sha256,
            hex_lower(&Sha256::digest(body.as_bytes()))
        );
        assert_eq!(
            receipt.receipt_sha256,
            "a17bb0640de9b3d7d9bde193df47eb1513da23b8cd598df4c5bcf831fff85359"
        );
        assert_eq!(
            serde_json::to_string(&receipt).expect("receipt must serialize"),
            format!(
                r#"{},"receipt_sha256":"{}"}}"#,
                body.strip_suffix('}').expect("body closes once"),
                receipt.receipt_sha256
            )
        );
        assert!(receipt.validate().is_ok());
        receipt.process_id = 0;
        assert!(receipt.validate().is_err());
        receipt.process_id = 42;
        receipt.launch_nonce.replace_range(..1, "A");
        assert!(receipt.validate().is_err());
        receipt.launch_nonce.replace_range(..1, "a");
        receipt.receipt_sha256.replace_range(..1, "0");
        assert!(receipt.validate().is_err());
    }

    #[test]
    fn canonical_round_trip_covers_all_sixty_atomic_profiles() {
        let profiles = ReviewWorldDetailProfileV1::atomic_matrix();
        assert_eq!(profiles.len(), 60);
        let mut ids = std::collections::BTreeSet::new();
        for profile in profiles {
            profile.validate().expect("matrix entry must validate");
            let active = profile.active_treatment_ids();
            assert_eq!(active.len(), 1);
            let id = active.first().expect("one active treatment");
            assert!(ids.insert(*id), "duplicate treatment id {id}");
            let json = profile
                .canonical_json()
                .expect("matrix entry must serialize");
            let reparsed = ReviewWorldDetailProfileV1::from_canonical_json(&json)
                .expect("canonical matrix JSON must parse");
            assert_eq!(reparsed, profile);
            assert_eq!(
                reparsed
                    .profile_hash_sha256()
                    .expect("reparsed profile must hash"),
                profile
                    .profile_hash_sha256()
                    .expect("source profile must hash")
            );
        }
        assert_eq!(ids.len(), 60);
    }

    #[test]
    fn focused_snowline_profiles_round_trip_with_distinct_identities() {
        let mut hashes = std::collections::BTreeSet::new();
        for snow in [
            SnowDetailV1::StraightThreshold { level: 200 },
            SnowDetailV1::StraightThreshold { level: 260 },
            SnowDetailV1::CoherentLine {
                mean_level: 200,
                amplitude_levels: 16,
                correlation_hexes: 22,
            },
            SnowDetailV1::CoherentLine {
                mean_level: 200,
                amplitude_levels: 32,
                correlation_hexes: 16,
            },
            SnowDetailV1::CoherentLine {
                mean_level: 200,
                amplitude_levels: 48,
                correlation_hexes: 12,
            },
        ] {
            let profile = ReviewWorldDetailProfileV1 {
                snow,
                ..ReviewWorldDetailProfileV1::default()
            };
            let json = profile.canonical_json().expect("focused profile is valid");
            assert_eq!(
                ReviewWorldDetailProfileV1::from_canonical_json(&json)
                    .expect("focused profile round trips"),
                profile
            );
            assert!(hashes.insert(profile.profile_hash_sha256().expect("profile hashes")));
        }
    }

    #[test]
    fn focused_plant_density_profiles_round_trip_with_distinct_identities() {
        let mut hashes = std::collections::BTreeSet::new();
        for (density, cap, id) in [
            (0.04, 4000, "props-focused-litter-4pct"),
            (0.12, 12000, "props-focused-litter-12pct"),
        ] {
            let profile = ReviewWorldDetailProfileV1 {
                terrain_props: TerrainPropsDetailV1::GrassLitter { density, cap },
                ..ReviewWorldDetailProfileV1::default()
            };
            assert_eq!(profile.active_treatment_ids(), vec![id]);
            let json = profile.canonical_json().expect("focused profile is valid");
            assert_eq!(
                ReviewWorldDetailProfileV1::from_canonical_json(&json)
                    .expect("focused profile round trips"),
                profile
            );
            assert!(hashes.insert(profile.profile_hash_sha256().expect("profile hashes")));
        }
        let matrix = ReviewWorldDetailProfileV1::atomic_matrix();
        assert_eq!(matrix.len(), 60);
        assert_eq!(
            matrix
                .iter()
                .filter(|profile| !profile.terrain_props.is_current())
                .count(),
            6
        );
    }

    #[test]
    fn unknown_fields_are_rejected_at_root_and_section_levels() {
        let root = r#"{"version":1,"snow":{"kind":"current"},"water":{"kind":"current"},"physical_clouds":{"kind":"current"},"shore_and_falls":{"kind":"current"},"alpine_vegetation":{"kind":"current"},"cliff_strata":{"kind":"current"},"terrain_props":{"kind":"current"},"ice_fringe":{"kind":"current"},"local_fog":{"kind":"current"},"surprise":true}"#;
        assert!(ReviewWorldDetailProfileV1::from_json(root).is_err());

        let nested = ReviewWorldDetailProfileV1::default()
            .canonical_json()
            .expect("control must serialize")
            .replace(
                r#""snow":{"kind":"current"}"#,
                r#""snow":{"kind":"current","surprise":true}"#,
            );
        assert!(ReviewWorldDetailProfileV1::from_json(&nested).is_err());
    }

    #[test]
    fn unsupported_version_is_rejected() {
        let profile = ReviewWorldDetailProfileV1 {
            version: 2,
            ..ReviewWorldDetailProfileV1::default()
        };
        assert!(profile.validate().is_err());
        let json = serde_json::to_string(&profile).expect("invalid version still serializes");
        assert!(ReviewWorldDetailProfileV1::from_json(&json).is_err());
    }

    #[test]
    fn nonfinite_and_out_of_range_values_are_rejected() {
        let nonfinite = ReviewWorldDetailProfileV1 {
            water: WaterDetailV1::UniformAlpha { alpha: f32::NAN },
            ..ReviewWorldDetailProfileV1::default()
        };
        assert!(nonfinite.validate().is_err());

        let out_of_range = ReviewWorldDetailProfileV1 {
            water: WaterDetailV1::UniformAlpha { alpha: 1.1 },
            ..ReviewWorldDetailProfileV1::default()
        };
        assert!(out_of_range.validate().is_err());

        let unsupported_in_range = ReviewWorldDetailProfileV1 {
            snow: SnowDetailV1::StraightThreshold { level: 141 },
            ..ReviewWorldDetailProfileV1::default()
        };
        assert!(unsupported_in_range.validate().is_err());
    }

    #[test]
    fn canonical_parser_rejects_whitespace_and_reordered_input() {
        let canonical = ReviewWorldDetailProfileV1::default()
            .canonical_json()
            .expect("control must serialize");
        let spaced = canonical.replace("{", "{ ");
        assert!(ReviewWorldDetailProfileV1::from_json(&spaced).is_ok());
        assert!(ReviewWorldDetailProfileV1::from_canonical_json(&spaced).is_err());
    }

    #[test]
    fn capability_helpers_follow_active_sections() {
        let transmission = ReviewWorldDetailProfileV1 {
            water: WaterDetailV1::Transmission {
                ior: 1.333,
                thickness: 0.08,
                max_refraction_uv: 0.015,
            },
            ..ReviewWorldDetailProfileV1::default()
        };
        assert!(!transmission.requires_oit());
        assert!(transmission.requires_transmission());
        assert!(!transmission.requires_volumetrics());

        let wet_rim = ReviewWorldDetailProfileV1 {
            shore_and_falls: ShoreAndFallsDetailV1::WetRim {
                width: 0.12,
                value_delta: -0.12,
                roughness_delta: -0.15,
            },
            ..ReviewWorldDetailProfileV1::default()
        };
        assert!(!wet_rim.requires_oit());

        let fog = ReviewWorldDetailProfileV1 {
            local_fog: fog(LocalFogPlacementV1::Mixed, 16.0, 26.0, 2.4, 0.16, 0.07),
            ..ReviewWorldDetailProfileV1::default()
        };
        assert!(fog.requires_volumetrics());

        let cliff_shell = ReviewWorldDetailProfileV1 {
            cliff_strata: CliffStrataDetailV1::SideValue { value_delta: -0.06 },
            ..ReviewWorldDetailProfileV1::default()
        };
        assert!(!cliff_shell.requires_oit());
        assert!(!cliff_shell.requires_transmission());
        assert!(!cliff_shell.requires_volumetrics());
    }

    #[test]
    fn waterfall_evidence_requires_exact_axial_distance() {
        let mut evidence = ReviewWorldDetailEffectValidationV1 {
            waterfall_anchors: vec![ReviewWaterfallAnchorEvidenceV1 {
                anchor_name: "grand_v3.waterfall_base".to_owned(),
                anchor_position: [1, 2, 3],
                landing_position: [3, -1, 1],
                distance_hexes: 3,
            }],
            ..ReviewWorldDetailEffectValidationV1::default()
        };
        assert!(evidence.validate().is_ok());

        evidence
            .waterfall_anchors
            .first_mut()
            .expect("the waterfall evidence fixture retains its one anchor")
            .distance_hexes = 2;
        assert!(evidence.validate().is_err());

        let anchor = evidence
            .waterfall_anchors
            .first_mut()
            .expect("the waterfall evidence fixture retains its one anchor");
        anchor.anchor_position = [i32::MIN, i32::MIN, 3];
        anchor.landing_position = [i32::MAX, i32::MAX, 1];
        assert!(evidence.validate().is_err());
    }

    #[test]
    fn report_rejects_bad_hash_nonfinite_anchor_and_mismatched_total() {
        let authority = ReviewAuthorityFingerprintsV1 {
            voxel_map: "0".repeat(16),
            structural: "0".repeat(16),
            materialized: "0".repeat(16),
            liquid_graph: "0".repeat(16),
            topology: "0".repeat(16),
            traversal: "0".repeat(16),
            blockers: "0".repeat(16),
            anchors: "0".repeat(16),
            biomes: "0".repeat(16),
            feature_roots: "0".repeat(16),
            logical_terrain_picking: "0".repeat(16),
            gameplay_state: "0".repeat(16),
        };
        let mut report = ReviewWorldDetailReportV1 {
            version: REVIEW_WORLD_DETAIL_REPORT_VERSION_V1,
            runtime_receipt: fixture_runtime_receipt(&"0".repeat(64)),
            profile_hash_sha256: "0".repeat(64),
            authority,
            projection_hashes: ReviewWorldDetailProjectionHashesV1 {
                terrain_plan: "0".repeat(16),
                liquid_atmosphere_plan: "0".repeat(16),
                mesh_projection: "0".repeat(16),
            },
            effect_validation: ReviewWorldDetailEffectValidationV1::default(),
            counts: ReviewWorldDetailCountsV1::default(),
            anchor_heights: BTreeMap::new(),
            anchor_classes: BTreeMap::new(),
            camera_features: ReviewCameraFeaturesV1 {
                oit: false,
                medium_transmission: false,
                depth_texture: false,
                volumetrics: false,
            },
            performance: ReviewPerformanceSampleV1 {
                frame_time_ms: 16.0,
                resident_presentation_bytes: 1,
                warmup_complete: true,
            },
            cleanup: ReviewCleanupStateV1 {
                completed_cycles: 100,
                entities_remaining: 0,
                materials_remaining: 0,
                meshes_remaining: 0,
                target_images_remaining: 0,
                camera_state_restored: true,
                oit_state_restored: true,
                transmission_state_restored: true,
                depth_state_restored: true,
                volumetric_state_restored: true,
            },
        };
        assert!(report.validate().is_ok());
        assert!(report.cleanup.is_complete());

        report.authority.voxel_map = String::new();
        assert!(report.validate().is_err());
        report.authority.voxel_map = "0".repeat(16);
        report.profile_hash_sha256 = "ABC".into();
        assert!(report.validate().is_err());
        report.profile_hash_sha256 = "0".repeat(64);
        report.runtime_receipt = fixture_runtime_receipt(&"1".repeat(64));
        assert!(report.validate().is_err());
        report.runtime_receipt = fixture_runtime_receipt(&"0".repeat(64));
        report.projection_hashes.mesh_projection = "ABC".into();
        assert!(report.validate().is_err());
        report.projection_hashes.mesh_projection = "0".repeat(16);
        report.performance.frame_time_ms = f32::NAN;
        assert!(report.validate().is_err());
        report.performance.frame_time_ms = 16.0;
        report.anchor_heights.insert("peak".into(), f32::INFINITY);
        assert!(report.validate().is_err());
        report.anchor_heights.clear();
        report.anchor_heights.insert("peak".into(), 42.0);
        assert!(report.validate().is_err());
        report
            .anchor_classes
            .insert("peak".into(), ReviewAnchorClassV1::Observation);
        assert!(report.validate().is_ok());
        report.anchor_heights.clear();
        report.anchor_classes.clear();
        report.counts.total.entities = 1;
        assert!(report.validate().is_err());
    }
}
