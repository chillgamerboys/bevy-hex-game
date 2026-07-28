//! Canonical colour and voxel-style contracts for authored art.
//!
//! These types deliberately use stable string keys rather than runtime numeric ids.
//! Authored objects must remain meaningful when a palette or style catalog is
//! reordered, and their references must be readable in review diffs.

use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

/// Current schema version for palette and voxel-style files.
pub const ART_SCHEMA_VERSION: u32 = 1;

/// OKLab distance at or below which the editor should warn about a near-duplicate.
pub const DEFAULT_NEAR_COLOR_THRESHOLD: f32 = 0.025;

const MAX_ID_LENGTH: usize = 128;
const MAX_DISPLAY_NAME_LENGTH: usize = 80;
const MAX_TAG_LENGTH: usize = 48;

/// A validation failure in an authored-art contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtContractError {
    message: String,
}

impl ArtContractError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Human-readable validation detail.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ArtContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ArtContractError {}

macro_rules! stable_id {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Validates and constructs a stable id.
            pub fn new(value: impl Into<String>) -> Result<Self, ArtContractError> {
                let value = value.into();
                validate_stable_id(&value)?;
                Ok(Self(value))
            }

            /// The string form stored in authored RON.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = ArtContractError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

stable_id!(
    SwatchId,
    "Stable, path-like key for one colour in the shared art palette."
);
stable_id!(
    VoxelStyleId,
    "Stable, path-like key for a reusable voxel rendering style."
);
stable_id!(
    ObjectAssetId,
    "Stable, path-like key for an authored object blueprint."
);

fn validate_stable_id(value: &str) -> Result<(), ArtContractError> {
    if value.is_empty() {
        return Err(ArtContractError::new("stable ids cannot be empty"));
    }
    if value.len() > MAX_ID_LENGTH {
        return Err(ArtContractError::new(format!(
            "stable id '{value}' exceeds {MAX_ID_LENGTH} bytes"
        )));
    }

    for segment in value.split('/') {
        if segment.is_empty() {
            return Err(ArtContractError::new(format!(
                "stable id '{value}' contains an empty path segment"
            )));
        }
        let mut characters = segment.chars();
        let Some(first) = characters.next() else {
            return Err(ArtContractError::new(format!(
                "stable id '{value}' contains an empty path segment"
            )));
        };
        if !first.is_ascii_lowercase() {
            return Err(ArtContractError::new(format!(
                "stable id '{value}' segments must begin with a lowercase ASCII letter"
            )));
        }
        if !characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        }) {
            return Err(ArtContractError::new(format!(
                "stable id '{value}' may contain only lowercase ASCII letters, digits, and '-' within '/'-separated segments"
            )));
        }
    }
    Ok(())
}

/// A finite sRGB colour with every component in the inclusive range `0..=1`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct SrgbColor {
    red: f32,
    green: f32,
    blue: f32,
}

impl SrgbColor {
    /// Constructs a validated sRGB colour.
    pub fn new(red: f32, green: f32, blue: f32) -> Result<Self, ArtContractError> {
        for (name, component) in [("red", red), ("green", green), ("blue", blue)] {
            if !component.is_finite() || !(0.0..=1.0).contains(&component) {
                return Err(ArtContractError::new(format!(
                    "sRGB {name} component must be finite and within 0..=1, received {component}"
                )));
            }
        }
        Ok(Self { red, green, blue })
    }

    /// Red sRGB component.
    #[must_use]
    pub const fn red(self) -> f32 {
        self.red
    }

    /// Green sRGB component.
    #[must_use]
    pub const fn green(self) -> f32 {
        self.green
    }

    /// Blue sRGB component.
    #[must_use]
    pub const fn blue(self) -> f32 {
        self.blue
    }

    /// Components in red, green, blue order.
    #[must_use]
    pub const fn to_array(self) -> [f32; 3] {
        [self.red, self.green, self.blue]
    }

    fn oklab(self) -> [f32; 3] {
        let [red, green, blue] = self.to_array().map(srgb_to_linear);
        let long =
            0.412_221_46f32.mul_add(red, 0.536_332_55f32.mul_add(green, 0.051_445_995 * blue));
        let medium =
            0.211_903_5f32.mul_add(red, 0.680_699_5f32.mul_add(green, 0.107_396_96 * blue));
        let short =
            0.088_302_46f32.mul_add(red, 0.281_718_85f32.mul_add(green, 0.629_978_7 * blue));
        let long_root = long.cbrt();
        let medium_root = medium.cbrt();
        let short_root = short.cbrt();

        [
            0.210_454_26f32.mul_add(
                long_root,
                0.793_617_8f32.mul_add(medium_root, -0.004_072_047 * short_root),
            ),
            1.977_998_5f32.mul_add(
                long_root,
                (-2.428_592_2f32).mul_add(medium_root, 0.450_593_7 * short_root),
            ),
            0.025_904_037f32.mul_add(
                long_root,
                0.782_771_77f32.mul_add(medium_root, -0.808_675_77 * short_root),
            ),
        ]
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnvalidatedSrgbColor {
    red: f32,
    green: f32,
    blue: f32,
}

impl<'de> Deserialize<'de> for SrgbColor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = UnvalidatedSrgbColor::deserialize(deserializer)?;
        Self::new(raw.red, raw.green, raw.blue).map_err(serde::de::Error::custom)
    }
}

fn srgb_to_linear(component: f32) -> f32 {
    if component <= 0.040_45 {
        component / 12.92
    } else {
        ((component + 0.055) / 1.055).powf(2.4)
    }
}

/// One named colour in the shared palette.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PaletteSwatch {
    display_name: String,
    color: SrgbColor,
    tags: BTreeSet<String>,
}

impl PaletteSwatch {
    /// Constructs a validated palette entry.
    pub fn new(
        display_name: impl Into<String>,
        color: SrgbColor,
        tags: BTreeSet<String>,
    ) -> Result<Self, ArtContractError> {
        let display_name = display_name.into();
        validate_display_name(&display_name)?;
        if tags.is_empty() {
            return Err(ArtContractError::new(
                "palette swatch must have at least one ownership or search tag",
            ));
        }
        for tag in &tags {
            validate_tag(tag)?;
        }
        Ok(Self {
            display_name,
            color,
            tags,
        })
    }

    /// Human-readable colour name.
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Exact sRGB value.
    #[must_use]
    pub const fn color(&self) -> SrgbColor {
        self.color
    }

    /// Sorted search and ownership tags.
    #[must_use]
    pub fn tags(&self) -> &BTreeSet<String> {
        &self.tags
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnvalidatedPaletteSwatch {
    display_name: String,
    color: SrgbColor,
    tags: Vec<String>,
}

impl<'de> Deserialize<'de> for PaletteSwatch {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = UnvalidatedPaletteSwatch::deserialize(deserializer)?;
        let tags: BTreeSet<String> = raw.tags.iter().cloned().collect();
        if tags.len() != raw.tags.len() {
            return Err(serde::de::Error::custom(
                "palette swatch tags cannot contain duplicates",
            ));
        }
        Self::new(raw.display_name, raw.color, tags).map_err(serde::de::Error::custom)
    }
}

/// A nearest-colour query result, ordered by OKLab distance then stable id.
#[derive(Debug, Clone, PartialEq)]
pub struct SwatchMatch {
    /// Matching palette entry.
    pub id: SwatchId,
    /// Euclidean distance in OKLab.
    pub distance: f32,
}

/// Versioned shared art palette.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ArtPalette {
    schema_version: u32,
    swatches: BTreeMap<SwatchId, PaletteSwatch>,
}

impl ArtPalette {
    /// Constructs a schema-v1 palette.
    pub fn new(swatches: BTreeMap<SwatchId, PaletteSwatch>) -> Result<Self, ArtContractError> {
        let palette = Self {
            schema_version: ART_SCHEMA_VERSION,
            swatches,
        };
        palette.validate()?;
        Ok(palette)
    }

    /// Schema version represented by this palette.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Palette entries in stable-id order.
    #[must_use]
    pub fn swatches(&self) -> &BTreeMap<SwatchId, PaletteSwatch> {
        &self.swatches
    }

    /// Looks up one swatch by stable id.
    #[must_use]
    pub fn get(&self, id: &SwatchId) -> Option<&PaletteSwatch> {
        self.swatches.get(id)
    }

    /// Whether a swatch exists.
    #[must_use]
    pub fn contains(&self, id: &SwatchId) -> bool {
        self.swatches.contains_key(id)
    }

    /// Inserts or replaces one validated swatch.
    pub fn insert(
        &mut self,
        id: SwatchId,
        swatch: PaletteSwatch,
    ) -> Result<Option<PaletteSwatch>, ArtContractError> {
        validate_display_name(swatch.display_name())?;
        for tag in swatch.tags() {
            validate_tag(tag)?;
        }
        Ok(self.swatches.insert(id, swatch))
    }

    /// Removes a swatch while preserving the non-empty palette invariant.
    pub fn remove(&mut self, id: &SwatchId) -> Result<Option<PaletteSwatch>, ArtContractError> {
        if self.swatches.len() == 1 && self.swatches.contains_key(id) {
            return Err(ArtContractError::new(
                "the last palette swatch cannot be removed",
            ));
        }
        Ok(self.swatches.remove(id))
    }

    /// Checks schema and entry invariants.
    pub fn validate(&self) -> Result<(), ArtContractError> {
        validate_schema(self.schema_version, "art palette")?;
        if self.swatches.is_empty() {
            return Err(ArtContractError::new(
                "art palette must contain at least one swatch",
            ));
        }
        for swatch in self.swatches.values() {
            validate_display_name(swatch.display_name())?;
            if swatch.tags().is_empty() {
                return Err(ArtContractError::new(
                    "palette swatch must have at least one ownership or search tag",
                ));
            }
            for tag in swatch.tags() {
                validate_tag(tag)?;
            }
        }
        Ok(())
    }

    /// Finds at most `limit` palette colours nearest to `color` in OKLab.
    #[must_use]
    pub fn nearest_swatches(&self, color: SrgbColor, limit: usize) -> Vec<SwatchMatch> {
        let target = color.oklab();
        let mut matches: Vec<SwatchMatch> = self
            .swatches
            .iter()
            .map(|(id, swatch)| SwatchMatch {
                id: id.clone(),
                distance: oklab_distance(target, swatch.color().oklab()),
            })
            .collect();
        matches.sort_by(|left, right| {
            left.distance
                .total_cmp(&right.distance)
                .then_with(|| left.id.cmp(&right.id))
        });
        matches.truncate(limit);
        matches
    }

    /// Finds all entries close enough to trigger the editor's duplicate warning.
    #[must_use]
    pub fn near_duplicates(&self, color: SrgbColor) -> Vec<SwatchMatch> {
        self.nearest_swatches(color, self.swatches.len())
            .into_iter()
            .take_while(|candidate| candidate.distance <= DEFAULT_NEAR_COLOR_THRESHOLD)
            .collect()
    }

    /// Deterministic fingerprint of semantic palette content.
    #[must_use]
    pub fn semantic_fingerprint(&self) -> u64 {
        let mut encoder = FingerprintEncoder::new(b"hex-art-palette-v1");
        encoder.u32(self.schema_version);
        encoder.usize(self.swatches.len());
        for (id, swatch) in &self.swatches {
            encoder.string(id.as_str());
            encoder.string(swatch.display_name());
            encoder.color(swatch.color());
            encoder.usize(swatch.tags().len());
            for tag in swatch.tags() {
                encoder.string(tag);
            }
        }
        encoder.finish()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnvalidatedArtPalette {
    schema_version: u32,
    swatches: BTreeMap<SwatchId, PaletteSwatch>,
}

impl<'de> Deserialize<'de> for ArtPalette {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = UnvalidatedArtPalette::deserialize(deserializer)?;
        let palette = Self {
            schema_version: raw.schema_version,
            swatches: raw.swatches,
        };
        palette.validate().map_err(serde::de::Error::custom)?;
        Ok(palette)
    }
}

fn oklab_distance(left: [f32; 3], right: [f32; 3]) -> f32 {
    let [left_lightness, left_green_red, left_blue_yellow] = left;
    let [right_lightness, right_green_red, right_blue_yellow] = right;
    let lightness = left_lightness - right_lightness;
    let green_red = left_green_red - right_green_red;
    let blue_yellow = left_blue_yellow - right_blue_yellow;
    lightness
        .mul_add(
            lightness,
            green_red.mul_add(green_red, blue_yellow * blue_yellow),
        )
        .sqrt()
}

/// Renderer treatment for a voxel style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum VoxelSurfaceMode {
    /// Fully opaque material.
    Opaque,
    /// Alpha-to-coverage material suited to foliage edges.
    Cutout,
    /// Conventional alpha-blended material.
    Translucent,
    /// Additive material suited to energy and spell accents.
    Additive,
}

impl VoxelSurfaceMode {
    const fn fingerprint_tag(self) -> u8 {
        match self {
            Self::Opaque => 0,
            Self::Cutout => 1,
            Self::Translucent => 2,
            Self::Additive => 3,
        }
    }
}

/// Optional emissive treatment for a voxel style.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VoxelEmission {
    swatch: SwatchId,
    strength: f32,
}

impl VoxelEmission {
    /// Constructs a validated emission reference.
    pub fn new(swatch: SwatchId, strength: f32) -> Result<Self, ArtContractError> {
        validate_emission_strength(strength)?;
        Ok(Self { swatch, strength })
    }

    /// Palette colour emitted by the style.
    #[must_use]
    pub fn swatch(&self) -> &SwatchId {
        &self.swatch
    }

    /// Nonnegative emission strength.
    #[must_use]
    pub const fn strength(&self) -> f32 {
        self.strength
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnvalidatedVoxelEmission {
    swatch: SwatchId,
    strength: f32,
}

impl<'de> Deserialize<'de> for VoxelEmission {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = UnvalidatedVoxelEmission::deserialize(deserializer)?;
        Self::new(raw.swatch, raw.strength).map_err(serde::de::Error::custom)
    }
}

/// A reusable rendering style applied to authored object voxels.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VoxelStyle {
    display_name: String,
    base_swatch: SwatchId,
    surface_mode: VoxelSurfaceMode,
    opacity: f32,
    emission: Option<VoxelEmission>,
}

impl VoxelStyle {
    /// Constructs a style and validates its local values.
    pub fn new(
        display_name: impl Into<String>,
        base_swatch: SwatchId,
        surface_mode: VoxelSurfaceMode,
        opacity: f32,
        emission: Option<VoxelEmission>,
    ) -> Result<Self, ArtContractError> {
        let display_name = display_name.into();
        validate_display_name(&display_name)?;
        validate_opacity(surface_mode, opacity)?;
        if let Some(emission) = &emission {
            validate_emission_strength(emission.strength())?;
        }
        Ok(Self {
            display_name,
            base_swatch,
            surface_mode,
            opacity,
            emission,
        })
    }

    /// Human-readable style name.
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Base colour in the shared palette.
    #[must_use]
    pub fn base_swatch(&self) -> &SwatchId {
        &self.base_swatch
    }

    /// Renderer treatment.
    #[must_use]
    pub const fn surface_mode(&self) -> VoxelSurfaceMode {
        self.surface_mode
    }

    /// Surface opacity.
    #[must_use]
    pub const fn opacity(&self) -> f32 {
        self.opacity
    }

    /// Optional emission settings.
    #[must_use]
    pub fn emission(&self) -> Option<&VoxelEmission> {
        self.emission.as_ref()
    }

    fn validate(&self, palette: Option<&ArtPalette>) -> Result<(), ArtContractError> {
        validate_display_name(self.display_name())?;
        validate_opacity(self.surface_mode, self.opacity)?;
        if let Some(palette) = palette {
            if !palette.contains(self.base_swatch()) {
                return Err(ArtContractError::new(format!(
                    "voxel style '{}' references missing base swatch '{}'",
                    self.display_name(),
                    self.base_swatch()
                )));
            }
            if let Some(emission) = self.emission() {
                if !palette.contains(emission.swatch()) {
                    return Err(ArtContractError::new(format!(
                        "voxel style '{}' references missing emission swatch '{}'",
                        self.display_name(),
                        emission.swatch()
                    )));
                }
            }
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnvalidatedVoxelStyle {
    display_name: String,
    base_swatch: SwatchId,
    surface_mode: VoxelSurfaceMode,
    opacity: f32,
    emission: Option<VoxelEmission>,
}

impl<'de> Deserialize<'de> for VoxelStyle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = UnvalidatedVoxelStyle::deserialize(deserializer)?;
        Self::new(
            raw.display_name,
            raw.base_swatch,
            raw.surface_mode,
            raw.opacity,
            raw.emission,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Versioned catalog of reusable voxel styles.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VoxelStyleCatalog {
    schema_version: u32,
    styles: BTreeMap<VoxelStyleId, VoxelStyle>,
}

impl VoxelStyleCatalog {
    /// Constructs a schema-v1 style catalog.
    pub fn new(styles: BTreeMap<VoxelStyleId, VoxelStyle>) -> Result<Self, ArtContractError> {
        let catalog = Self {
            schema_version: ART_SCHEMA_VERSION,
            styles,
        };
        catalog.validate_local()?;
        Ok(catalog)
    }

    /// Schema version represented by this catalog.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Styles in stable-id order.
    #[must_use]
    pub fn styles(&self) -> &BTreeMap<VoxelStyleId, VoxelStyle> {
        &self.styles
    }

    /// Looks up one style by stable id.
    #[must_use]
    pub fn get(&self, id: &VoxelStyleId) -> Option<&VoxelStyle> {
        self.styles.get(id)
    }

    /// Whether a style exists.
    #[must_use]
    pub fn contains(&self, id: &VoxelStyleId) -> bool {
        self.styles.contains_key(id)
    }

    /// Inserts or replaces one locally valid style.
    pub fn insert(
        &mut self,
        id: VoxelStyleId,
        style: VoxelStyle,
    ) -> Result<Option<VoxelStyle>, ArtContractError> {
        style.validate(None)?;
        Ok(self.styles.insert(id, style))
    }

    /// Removes one style.
    pub fn remove(&mut self, id: &VoxelStyleId) -> Option<VoxelStyle> {
        self.styles.remove(id)
    }

    /// Validates local values and every reference into `palette`.
    pub fn validate(&self, palette: &ArtPalette) -> Result<(), ArtContractError> {
        self.validate_local()?;
        for style in self.styles.values() {
            style.validate(Some(palette))?;
        }
        Ok(())
    }

    /// Returns style ids that refer to `swatch` as a base or emission colour.
    #[must_use]
    pub fn references_to(&self, swatch: &SwatchId) -> Vec<VoxelStyleId> {
        self.styles
            .iter()
            .filter(|(_, style)| {
                style.base_swatch() == swatch
                    || style
                        .emission()
                        .is_some_and(|emission| emission.swatch() == swatch)
            })
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Deterministic fingerprint of semantic style content.
    #[must_use]
    pub fn semantic_fingerprint(&self) -> u64 {
        let mut encoder = FingerprintEncoder::new(b"hex-voxel-style-catalog-v1");
        encoder.u32(self.schema_version);
        encoder.usize(self.styles.len());
        for (id, style) in &self.styles {
            encoder.string(id.as_str());
            encoder.string(style.display_name());
            encoder.string(style.base_swatch().as_str());
            encoder.u8(style.surface_mode().fingerprint_tag());
            encoder.f32(style.opacity());
            if let Some(emission) = style.emission() {
                encoder.u8(1);
                encoder.string(emission.swatch().as_str());
                encoder.f32(emission.strength());
            } else {
                encoder.u8(0);
            }
        }
        encoder.finish()
    }

    fn validate_local(&self) -> Result<(), ArtContractError> {
        validate_schema(self.schema_version, "voxel style catalog")?;
        for style in self.styles.values() {
            style.validate(None)?;
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnvalidatedVoxelStyleCatalog {
    schema_version: u32,
    styles: BTreeMap<VoxelStyleId, VoxelStyle>,
}

impl<'de> Deserialize<'de> for VoxelStyleCatalog {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = UnvalidatedVoxelStyleCatalog::deserialize(deserializer)?;
        let catalog = Self {
            schema_version: raw.schema_version,
            styles: raw.styles,
        };
        catalog.validate_local().map_err(serde::de::Error::custom)?;
        Ok(catalog)
    }
}

fn validate_schema(schema_version: u32, kind: &str) -> Result<(), ArtContractError> {
    if schema_version != ART_SCHEMA_VERSION {
        return Err(ArtContractError::new(format!(
            "{kind} schema version {schema_version} is unsupported; expected {ART_SCHEMA_VERSION}"
        )));
    }
    Ok(())
}

fn validate_display_name(display_name: &str) -> Result<(), ArtContractError> {
    if display_name.is_empty() || display_name.trim() != display_name {
        return Err(ArtContractError::new(
            "display names must be non-empty and have no surrounding whitespace",
        ));
    }
    if display_name.chars().count() > MAX_DISPLAY_NAME_LENGTH {
        return Err(ArtContractError::new(format!(
            "display name exceeds {MAX_DISPLAY_NAME_LENGTH} characters"
        )));
    }
    if display_name.chars().any(char::is_control) {
        return Err(ArtContractError::new(
            "display names cannot contain control characters",
        ));
    }
    Ok(())
}

fn validate_tag(tag: &str) -> Result<(), ArtContractError> {
    if tag.is_empty() || tag.len() > MAX_TAG_LENGTH {
        return Err(ArtContractError::new(format!(
            "palette tags must contain 1..={MAX_TAG_LENGTH} bytes"
        )));
    }
    if !tag.chars().all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '-' | '_')
    }) {
        return Err(ArtContractError::new(format!(
            "palette tag '{tag}' may contain only lowercase ASCII letters, digits, '-' and '_'"
        )));
    }
    Ok(())
}

fn validate_opacity(surface_mode: VoxelSurfaceMode, opacity: f32) -> Result<(), ArtContractError> {
    if !opacity.is_finite() || opacity <= 0.0 || opacity > 1.0 {
        return Err(ArtContractError::new(format!(
            "voxel style opacity must be finite and within 0 < opacity <= 1, received {opacity}"
        )));
    }
    if surface_mode == VoxelSurfaceMode::Opaque && opacity.to_bits() != 1.0f32.to_bits() {
        return Err(ArtContractError::new(
            "opaque voxel styles must use opacity 1",
        ));
    }
    Ok(())
}

fn validate_emission_strength(strength: f32) -> Result<(), ArtContractError> {
    if !strength.is_finite() || strength < 0.0 {
        return Err(ArtContractError::new(format!(
            "emission strength must be finite and nonnegative, received {strength}"
        )));
    }
    Ok(())
}

struct FingerprintEncoder {
    hash: u64,
}

impl FingerprintEncoder {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    fn new(domain: &[u8]) -> Self {
        let mut encoder = Self {
            hash: Self::OFFSET_BASIS,
        };
        encoder.bytes(domain);
        encoder
    }

    fn bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.hash ^= u64::from(*byte);
            self.hash = self.hash.wrapping_mul(Self::PRIME);
        }
    }

    fn u8(&mut self, value: u8) {
        self.bytes(&[value]);
    }

    fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }

    fn usize(&mut self, value: usize) {
        self.bytes(&u64::try_from(value).unwrap_or(u64::MAX).to_le_bytes());
    }

    fn string(&mut self, value: &str) {
        self.usize(value.len());
        self.bytes(value.as_bytes());
    }

    fn f32(&mut self, value: f32) {
        self.u32(value.to_bits());
    }

    fn color(&mut self, color: SrgbColor) {
        for component in color.to_array() {
            self.f32(component);
        }
    }

    const fn finish(self) -> u64 {
        self.hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id<T>(value: &str) -> T
    where
        T: FromStr<Err = ArtContractError>,
    {
        let Ok(id) = value.parse() else {
            unreachable!("test stable id should be valid")
        };
        id
    }

    fn color(red: f32, green: f32, blue: f32) -> SrgbColor {
        let Ok(color) = SrgbColor::new(red, green, blue) else {
            unreachable!("test colour should be valid")
        };
        color
    }

    fn swatch(name: &str, color: SrgbColor) -> PaletteSwatch {
        let Ok(swatch) = PaletteSwatch::new(name, color, BTreeSet::from(["test".to_owned()]))
        else {
            unreachable!("test swatch should be valid")
        };
        swatch
    }

    fn palette() -> ArtPalette {
        let mut swatches = BTreeMap::new();
        swatches.insert(id("plant/leaf"), swatch("Leaf", color(0.12, 0.34, 0.12)));
        swatches.insert(id("plant/trunk"), swatch("Trunk", color(0.28, 0.15, 0.07)));
        let Ok(palette) = ArtPalette::new(swatches) else {
            unreachable!("test palette should be valid")
        };
        palette
    }

    fn shipped_palette() -> ArtPalette {
        let parsed = ron::from_str(include_str!("../../../assets/art/palette.ron"));
        let Ok(palette) = parsed else {
            unreachable!("the shipped art palette should parse")
        };
        palette
    }

    fn shipped_style_catalog() -> VoxelStyleCatalog {
        let parsed = ron::from_str(include_str!("../../../assets/art/voxel_styles.ron"));
        let Ok(catalog) = parsed else {
            unreachable!("the shipped voxel style catalog should parse")
        };
        catalog
    }

    fn style(name: &str, swatch: &str, mode: VoxelSurfaceMode, opacity: f32) -> VoxelStyle {
        let Ok(style) = VoxelStyle::new(name, id(swatch), mode, opacity, None) else {
            unreachable!("test style should be valid")
        };
        style
    }

    #[test]
    fn stable_ids_are_path_like_and_validate_during_deserialization() {
        assert!(SwatchId::new("plant/broad-leaf-2").is_ok());
        for invalid in [
            "",
            "/plant",
            "plant/",
            "plant//leaf",
            "Plant/leaf",
            "plant/2leaf",
            "plant/leaf.color",
            "plant/broad_leaf",
        ] {
            assert!(
                SwatchId::new(invalid).is_err(),
                "'{invalid}' should not be a stable id"
            );
        }
        assert!(ron::from_str::<SwatchId>("\"Plant/leaf\"").is_err());
    }

    #[test]
    fn srgb_rejects_non_finite_and_out_of_range_components() {
        assert!(SrgbColor::new(0.0, 0.5, 1.0).is_ok());
        assert!(SrgbColor::new(-0.01, 0.5, 1.0).is_err());
        assert!(SrgbColor::new(0.0, 1.01, 1.0).is_err());
        assert!(SrgbColor::new(0.0, f32::NAN, 1.0).is_err());
        assert!(SrgbColor::new(0.0, 0.5, f32::INFINITY).is_err());
    }

    #[test]
    fn palette_ron_is_strict_and_validated() {
        let valid = r#"(
            schema_version: 1,
            swatches: {
                "plant/leaf": (
                    display_name: "Leaf",
                    color: (red: 0.12, green: 0.34, blue: 0.12),
                    tags: ["plant", "foliage"],
                ),
            },
        )"#;
        assert!(ron::from_str::<ArtPalette>(valid).is_ok());
        assert!(ron::from_str::<ArtPalette>(
            &valid.replace("schema_version: 1", "schema_version: 2")
        )
        .is_err());
        assert!(ron::from_str::<ArtPalette>(&valid.replace(
            "tags: [\"plant\", \"foliage\"],",
            "tags: [\"plant\", \"plant\"],"
        ))
        .is_err());
        assert!(ron::from_str::<ArtPalette>(
            &valid.replace("tags: [\"plant\", \"foliage\"],", "tags: [],")
        )
        .is_err());
        assert!(PaletteSwatch::new("Leaf", color(0.12, 0.34, 0.12), BTreeSet::new()).is_err());
        assert!(ron::from_str::<ArtPalette>(&valid.replace(
            "display_name: \"Leaf\",",
            "display_name: \"Leaf\", stale: true,"
        ))
        .is_err());
    }

    #[test]
    fn shipped_art_catalogs_parse_and_resolve_together() {
        let palette = shipped_palette();
        let styles = shipped_style_catalog();

        assert_eq!(palette.swatches().len(), 19);
        assert!(styles.styles().is_empty());
        assert!(styles.validate(&palette).is_ok());
    }

    #[test]
    fn nearest_swatches_use_oklab_and_stable_tie_breaking() {
        let mut swatches = BTreeMap::new();
        swatches.insert(id("test/a"), swatch("A", color(0.2, 0.4, 0.2)));
        swatches.insert(id("test/b"), swatch("B", color(0.2, 0.4, 0.2)));
        swatches.insert(id("test/far"), swatch("Far", color(0.9, 0.1, 0.8)));
        let Ok(palette) = ArtPalette::new(swatches) else {
            unreachable!("test palette should be valid")
        };

        let matches = palette.nearest_swatches(color(0.2, 0.4, 0.2), 2);
        let [first, second] = matches.as_slice() else {
            unreachable!("the query requested two entries from a three-entry palette")
        };
        assert_eq!(first.id.as_str(), "test/a");
        assert_eq!(second.id.as_str(), "test/b");
        assert!(first.distance <= DEFAULT_NEAR_COLOR_THRESHOLD);
        assert_eq!(palette.nearest_swatches(color(0.2, 0.4, 0.2), 0), []);
    }

    #[test]
    fn palette_fingerprint_is_order_and_format_independent() {
        let first = palette();
        let ron = ron::ser::to_string_pretty(&first, ron::ser::PrettyConfig::default());
        let Ok(ron) = ron else {
            unreachable!("valid palette should serialize")
        };
        let Ok(round_trip): Result<ArtPalette, _> = ron::from_str(&ron) else {
            unreachable!("serialized palette should parse")
        };
        assert_eq!(
            first.semantic_fingerprint(),
            round_trip.semantic_fingerprint()
        );

        let mut changed = round_trip.clone();
        let replaced = changed.insert(id("plant/leaf"), swatch("Leaf", color(0.13, 0.34, 0.12)));
        assert!(replaced.is_ok());
        assert_ne!(first.semantic_fingerprint(), changed.semantic_fingerprint());
    }

    #[test]
    fn style_catalog_validates_palette_references() {
        let palette = palette();
        let mut styles = BTreeMap::new();
        styles.insert(
            id("plant/leaf"),
            style("Leaf", "plant/leaf", VoxelSurfaceMode::Cutout, 0.8),
        );
        let Ok(mut catalog) = VoxelStyleCatalog::new(styles) else {
            unreachable!("test style catalog should be locally valid")
        };
        assert!(catalog.validate(&palette).is_ok());
        assert!(catalog.contains(&id("plant/leaf")));

        let inserted = catalog.insert(
            id("effect/missing"),
            style("Missing", "effect/missing", VoxelSurfaceMode::Additive, 0.5),
        );
        assert!(inserted.is_ok());
        assert!(catalog.validate(&palette).is_err());
    }

    #[test]
    fn style_values_and_ron_are_strict() {
        assert!(VoxelStyle::new(
            "Opaque",
            id("plant/leaf"),
            VoxelSurfaceMode::Opaque,
            0.5,
            None,
        )
        .is_err());
        assert!(VoxelEmission::new(id("plant/leaf"), -0.1).is_err());
        assert!(VoxelEmission::new(id("plant/leaf"), f32::NAN).is_err());

        let invalid = r#"(
            schema_version: 1,
            styles: {
                "plant/leaf": (
                    display_name: "Leaf",
                    base_swatch: "plant/leaf",
                    surface_mode: Opaque,
                    opacity: 1.0,
                    emission: None,
                    stale: true,
                ),
            },
        )"#;
        assert!(ron::from_str::<VoxelStyleCatalog>(invalid).is_err());
    }

    #[test]
    fn style_fingerprint_is_deterministic_and_covers_emission() {
        let mut styles = BTreeMap::new();
        styles.insert(
            id("plant/leaf"),
            style("Leaf", "plant/leaf", VoxelSurfaceMode::Cutout, 0.8),
        );
        let Ok(first) = VoxelStyleCatalog::new(styles.clone()) else {
            unreachable!("test catalog should be valid")
        };
        let Ok(second) = VoxelStyleCatalog::new(styles) else {
            unreachable!("test catalog should be valid")
        };
        assert_eq!(first.semantic_fingerprint(), second.semantic_fingerprint());

        let Ok(emission) = VoxelEmission::new(id("plant/leaf"), 1.5) else {
            unreachable!("test emission should be valid")
        };
        let Ok(emissive_style) = VoxelStyle::new(
            "Leaf",
            id("plant/leaf"),
            VoxelSurfaceMode::Cutout,
            0.8,
            Some(emission),
        ) else {
            unreachable!("test emissive style should be valid")
        };
        let mut emissive_styles = BTreeMap::new();
        emissive_styles.insert(id("plant/leaf"), emissive_style);
        let Ok(emissive) = VoxelStyleCatalog::new(emissive_styles) else {
            unreachable!("test catalog should be valid")
        };
        assert_ne!(
            first.semantic_fingerprint(),
            emissive.semantic_fingerprint()
        );
    }

    #[test]
    fn reference_reports_are_sorted_and_cover_base_and_emission() {
        let Ok(emission) = VoxelEmission::new(id("plant/leaf"), 0.0) else {
            unreachable!("test emission should be valid")
        };
        let Ok(trunk) = VoxelStyle::new(
            "Trunk",
            id("plant/trunk"),
            VoxelSurfaceMode::Opaque,
            1.0,
            Some(emission),
        ) else {
            unreachable!("test style should be valid")
        };
        let mut styles = BTreeMap::new();
        styles.insert(
            id("plant/a-leaf"),
            style("Leaf", "plant/leaf", VoxelSurfaceMode::Cutout, 0.8),
        );
        styles.insert(id("plant/b-trunk"), trunk);
        let Ok(catalog) = VoxelStyleCatalog::new(styles) else {
            unreachable!("test catalog should be valid")
        };

        let references = catalog.references_to(&id("plant/leaf"));
        assert_eq!(
            references
                .iter()
                .map(VoxelStyleId::as_str)
                .collect::<Vec<_>>(),
            ["plant/a-leaf", "plant/b-trunk"]
        );
    }
}
