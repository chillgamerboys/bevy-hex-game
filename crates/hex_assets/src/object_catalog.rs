//! Runtime contracts for loading and placing authored voxel objects.
//!
//! The tracked manifest is intentionally small: it gives packaged builds a
//! deterministic list of object files without relying on filesystem directory
//! enumeration. [`RuntimeArtCatalog`] is the accepted, cross-file snapshot used by
//! renderers. Runtime consumers never combine a newly loaded palette with stale
//! styles or objects.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use bevy::prelude::*;
use hex_core::TilePos;
use serde::{Deserialize, Deserializer, Serialize};

use crate::loader::RonAssetLoader;
use crate::{
    ArtPalette, LocalAxialCoord, LocalVoxelCoord, ObjectAssetId, ObjectBlueprint, ObjectCategory,
    SrgbColor, VoxelStyle, VoxelStyleCatalog, VoxelStyleId, CONFIG_EXTENSIONS,
};
use crate::{LoadSettings, SettingsRegistry};

/// Current on-disk schema understood by [`ObjectCatalogFile`].
pub const OBJECT_CATALOG_SCHEMA_VERSION: u16 = 1;

const OBJECT_PATH_PREFIX: &str = "art/objects";
const VOXEL_STYLE_PATH: &str = "art/voxel_styles.ron";
const OBJECT_CATALOG_PATH: &str = "art/object_catalog.ron";

/// Registers the authored-object dependency graph and its runtime resolver.
pub(crate) fn plugin(app: &mut App) {
    app.register_type::<HexObjectRotation>()
        .register_type::<ObjectInstance>();
    app.load_settings::<VoxelStyleCatalog>(VOXEL_STYLE_PATH, CONFIG_EXTENSIONS)
        .load_settings::<ObjectCatalogFile>(OBJECT_CATALOG_PATH, CONFIG_EXTENSIONS);

    if !app.world().contains_resource::<Assets<ObjectBlueprint>>() {
        app.init_asset::<ObjectBlueprint>();
        app.register_asset_loader(RonAssetLoader::<ObjectBlueprint>::new(CONFIG_EXTENSIONS));
    }

    app.init_resource::<RuntimeArtCatalogStatus>();
    app.world_mut()
        .resource_mut::<SettingsRegistry>()
        .mark_pending::<RuntimeArtCatalog>();
    app.add_systems(
        Update,
        (sync_object_blueprint_handles, resolve_runtime_art_catalog).chain(),
    );
}

/// A validation failure in the object manifest, its paths, or its resolved graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectCatalogError {
    message: String,
}

impl ObjectCatalogError {
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

impl fmt::Display for ObjectCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ObjectCatalogError {}

impl ObjectCategory {
    /// Stable directory segment used by authored object ids and paths.
    #[must_use]
    pub const fn directory(self) -> &'static str {
        match self {
            Self::Plant => "plant",
            Self::Effect => "effect",
            Self::Prop => "prop",
        }
    }
}

impl ObjectAssetId {
    /// Category encoded by this object's stable id.
    pub fn category(&self) -> Result<ObjectCategory, ObjectCatalogError> {
        let (category, filename) = self.parts()?;
        if filename.contains('/') {
            return Err(ObjectCatalogError::new(format!(
                "object id '{}' must contain exactly one filename after its category",
                self.as_str()
            )));
        }
        match category {
            "plant" => Ok(ObjectCategory::Plant),
            "effect" => Ok(ObjectCategory::Effect),
            "prop" => Ok(ObjectCategory::Prop),
            _ => Err(ObjectCatalogError::new(format!(
                "object id '{}' must begin with 'plant/', 'effect/', or 'prop/'",
                self.as_str()
            ))),
        }
    }

    /// Filename segment encoded by this object's stable id, without `.ron`.
    pub fn file_name(&self) -> Result<&str, ObjectCatalogError> {
        let (_, filename) = self.parts()?;
        if filename.contains('/') {
            return Err(ObjectCatalogError::new(format!(
                "object id '{}' must contain exactly one filename after its category",
                self.as_str()
            )));
        }
        Ok(filename)
    }

    /// Checks that the id's category agrees with the blueprint's authored category.
    pub fn validate_for_category(
        &self,
        expected: ObjectCategory,
    ) -> Result<(), ObjectCatalogError> {
        let actual = self.category()?;
        if actual != expected {
            return Err(ObjectCatalogError::new(format!(
                "{expected:?} object id '{}' must begin with '{}/'",
                self.as_str(),
                expected.directory()
            )));
        }
        Ok(())
    }

    /// Canonical Bevy asset path for this object blueprint.
    pub fn asset_path(&self) -> Result<String, ObjectCatalogError> {
        let category = self.category()?;
        let filename = self.file_name()?;
        Ok(format!(
            "{OBJECT_PATH_PREFIX}/{}/{filename}.ron",
            category.directory()
        ))
    }

    fn parts(&self) -> Result<(&str, &str), ObjectCatalogError> {
        let Some((category, filename)) = self.as_str().split_once('/') else {
            return Err(ObjectCatalogError::new(format!(
                "object id '{}' must be '<category>/<filename>'",
                self.as_str()
            )));
        };
        if filename.is_empty() {
            return Err(ObjectCatalogError::new(format!(
                "object id '{}' must end with a filename",
                self.as_str()
            )));
        }
        Ok((category, filename))
    }
}

/// Deterministically ordered manifest of tracked object blueprints.
#[derive(Asset, Resource, TypePath, Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ObjectCatalogFile {
    schema_version: u16,
    objects: Vec<ObjectAssetId>,
}

impl ObjectCatalogFile {
    /// Creates a schema-v1 manifest, sorting ids and rejecting duplicates.
    pub fn new(ids: impl IntoIterator<Item = ObjectAssetId>) -> Result<Self, ObjectCatalogError> {
        let mut seen = BTreeSet::new();
        for id in ids {
            id.category()?;
            if !seen.insert(id.clone()) {
                return Err(ObjectCatalogError::new(format!(
                    "object catalog repeats id '{}'",
                    id.as_str()
                )));
            }
        }
        Ok(Self {
            schema_version: OBJECT_CATALOG_SCHEMA_VERSION,
            objects: seen.into_iter().collect(),
        })
    }

    /// Schema version represented by this manifest.
    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    /// Object ids in stable path order.
    #[must_use]
    pub fn ids(&self) -> &[ObjectAssetId] {
        &self.objects
    }

    /// Whether this manifest tracks `id`.
    #[must_use]
    pub fn contains(&self, id: &ObjectAssetId) -> bool {
        self.objects.binary_search(id).is_ok()
    }

    /// Deterministic fingerprint of manifest semantics.
    #[must_use]
    pub fn semantic_fingerprint(&self) -> u64 {
        let mut encoder = SemanticFingerprint::new(b"bevy-hex-game/object-catalog/v1");
        encoder.u16(self.schema_version);
        encoder.usize(self.objects.len());
        for id in &self.objects {
            encoder.string(id.as_str());
        }
        encoder.finish()
    }

    fn validate_ordered(&self) -> Result<(), ObjectCatalogError> {
        if self.schema_version != OBJECT_CATALOG_SCHEMA_VERSION {
            return Err(ObjectCatalogError::new(format!(
                "object catalog schema version {} is unsupported; expected \
                 {OBJECT_CATALOG_SCHEMA_VERSION}",
                self.schema_version
            )));
        }
        for id in &self.objects {
            id.category()?;
        }
        for pair in self.objects.windows(2) {
            let [left, right] = pair else {
                continue;
            };
            if left >= right {
                return Err(ObjectCatalogError::new(format!(
                    "object catalog ids must be strictly sorted and unique; '{}' appears before '{}'",
                    left.as_str(),
                    right.as_str()
                )));
            }
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnvalidatedObjectCatalogFile {
    schema_version: u16,
    objects: Vec<ObjectAssetId>,
}

impl<'de> Deserialize<'de> for ObjectCatalogFile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = UnvalidatedObjectCatalogFile::deserialize(deserializer)?;
        let catalog = Self {
            schema_version: raw.schema_version,
            objects: raw.objects,
        };
        catalog
            .validate_ordered()
            .map_err(serde::de::Error::custom)?;
        Ok(catalog)
    }
}

/// One authored voxel style with all palette references resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedVoxelStyle {
    authored: VoxelStyle,
    base_color: SrgbColor,
    emission_color: Option<SrgbColor>,
}

impl ResolvedVoxelStyle {
    /// Authored style semantics, including surface mode, opacity, and emission strength.
    #[must_use]
    pub const fn authored(&self) -> &VoxelStyle {
        &self.authored
    }

    /// Resolved base colour from the accepted palette snapshot.
    #[must_use]
    pub const fn base_color(&self) -> SrgbColor {
        self.base_color
    }

    /// Resolved emission colour, when the authored style emits light.
    #[must_use]
    pub const fn emission_color(&self) -> Option<SrgbColor> {
        self.emission_color
    }
}

/// One coherent accepted palette → style → object dependency graph.
#[derive(Resource, Debug, Clone)]
pub struct RuntimeArtCatalog {
    palette: ArtPalette,
    styles: VoxelStyleCatalog,
    manifest: ObjectCatalogFile,
    objects: BTreeMap<ObjectAssetId, ObjectBlueprint>,
    resolved_styles: BTreeMap<VoxelStyleId, ResolvedVoxelStyle>,
    palette_fingerprint: u64,
    style_fingerprint: u64,
    object_fingerprint: u64,
    combined_fingerprint: u64,
}

impl RuntimeArtCatalog {
    /// Resolves and validates one complete candidate graph.
    pub fn from_sources(
        palette: &ArtPalette,
        styles: &VoxelStyleCatalog,
        manifest: &ObjectCatalogFile,
        objects: BTreeMap<ObjectAssetId, ObjectBlueprint>,
    ) -> Result<Self, ObjectCatalogError> {
        palette
            .validate()
            .map_err(|error| ObjectCatalogError::new(error.to_string()))?;
        styles
            .validate(palette)
            .map_err(|error| ObjectCatalogError::new(error.to_string()))?;
        manifest.validate_ordered()?;

        if objects.len() != manifest.ids().len() {
            return Err(ObjectCatalogError::new(format!(
                "object catalog lists {} objects but {} blueprints were supplied",
                manifest.ids().len(),
                objects.len()
            )));
        }

        for id in manifest.ids() {
            let blueprint = objects.get(id).ok_or_else(|| {
                ObjectCatalogError::new(format!(
                    "object catalog references missing blueprint '{}'",
                    id.as_str()
                ))
            })?;
            if &blueprint.id != id {
                return Err(ObjectCatalogError::new(format!(
                    "object catalog id '{}' resolved to blueprint '{}'",
                    id.as_str(),
                    blueprint.id.as_str()
                )));
            }
            id.validate_for_category(blueprint.category)?;
            blueprint.validate(styles).map_err(|error| {
                ObjectCatalogError::new(format!("object '{}': {error}", id.as_str()))
            })?;
        }
        if let Some(extra) = objects.keys().find(|id| !manifest.contains(id)) {
            return Err(ObjectCatalogError::new(format!(
                "blueprint '{}' is not listed by the object catalog",
                extra.as_str()
            )));
        }

        let mut resolved_styles = BTreeMap::new();
        for (id, style) in styles.styles() {
            let base_color = palette
                .get(style.base_swatch())
                .ok_or_else(|| {
                    ObjectCatalogError::new(format!(
                        "voxel style '{}' references missing base swatch '{}'",
                        id.as_str(),
                        style.base_swatch()
                    ))
                })?
                .color();
            let emission_color = style
                .emission()
                .map(|emission| {
                    palette
                        .get(emission.swatch())
                        .map(|swatch| swatch.color())
                        .ok_or_else(|| {
                            ObjectCatalogError::new(format!(
                                "voxel style '{}' references missing emission swatch '{}'",
                                id.as_str(),
                                emission.swatch()
                            ))
                        })
                })
                .transpose()?;
            resolved_styles.insert(
                id.clone(),
                ResolvedVoxelStyle {
                    authored: style.clone(),
                    base_color,
                    emission_color,
                },
            );
        }

        let palette_fingerprint = palette.semantic_fingerprint();
        let style_fingerprint = styles.semantic_fingerprint();
        let mut object_encoder = SemanticFingerprint::new(b"bevy-hex-game/runtime-objects/v1");
        object_encoder.u64(manifest.semantic_fingerprint());
        object_encoder.usize(objects.len());
        for (id, object) in &objects {
            object_encoder.string(id.as_str());
            object_encoder.u64(
                object
                    .semantic_fingerprint()
                    .map_err(ObjectCatalogError::new)?,
            );
        }
        let object_fingerprint = object_encoder.finish();
        let mut combined = SemanticFingerprint::new(b"bevy-hex-game/runtime-art-catalog/v1");
        combined.u64(palette_fingerprint);
        combined.u64(style_fingerprint);
        combined.u64(object_fingerprint);
        let combined_fingerprint = combined.finish();

        Ok(Self {
            palette: palette.clone(),
            styles: styles.clone(),
            manifest: manifest.clone(),
            objects,
            resolved_styles,
            palette_fingerprint,
            style_fingerprint,
            object_fingerprint,
            combined_fingerprint,
        })
    }

    /// Accepted palette snapshot.
    #[must_use]
    pub const fn palette(&self) -> &ArtPalette {
        &self.palette
    }

    /// Accepted authored style catalog.
    #[must_use]
    pub const fn styles(&self) -> &VoxelStyleCatalog {
        &self.styles
    }

    /// Accepted object manifest.
    #[must_use]
    pub const fn manifest(&self) -> &ObjectCatalogFile {
        &self.manifest
    }

    /// Accepted objects in stable-id order.
    #[must_use]
    pub const fn objects(&self) -> &BTreeMap<ObjectAssetId, ObjectBlueprint> {
        &self.objects
    }

    /// Looks up one accepted object blueprint.
    #[must_use]
    pub fn object(&self, id: &ObjectAssetId) -> Option<&ObjectBlueprint> {
        self.objects.get(id)
    }

    /// Looks up one accepted and palette-resolved style.
    #[must_use]
    pub fn style(&self, id: &VoxelStyleId) -> Option<&ResolvedVoxelStyle> {
        self.resolved_styles.get(id)
    }

    /// Fingerprint of the accepted palette semantics.
    #[must_use]
    pub const fn palette_fingerprint(&self) -> u64 {
        self.palette_fingerprint
    }

    /// Fingerprint of the accepted authored style semantics.
    #[must_use]
    pub const fn style_fingerprint(&self) -> u64 {
        self.style_fingerprint
    }

    /// Fingerprint of the accepted manifest and object semantics.
    #[must_use]
    pub const fn object_fingerprint(&self) -> u64 {
        self.object_fingerprint
    }

    /// Fingerprint of the complete accepted dependency graph.
    #[must_use]
    pub const fn combined_fingerprint(&self) -> u64 {
        self.combined_fingerprint
    }

    /// Whether this accepted snapshot exactly matches a complete candidate graph.
    #[must_use]
    pub fn matches_sources(
        &self,
        palette: &ArtPalette,
        styles: &VoxelStyleCatalog,
        manifest: &ObjectCatalogFile,
        objects: &BTreeMap<ObjectAssetId, ObjectBlueprint>,
    ) -> bool {
        self.palette == *palette
            && self.styles == *styles
            && self.manifest == *manifest
            && self.objects == *objects
    }
}

/// Readiness and rejection detail for the authored-object dependency graph.
///
/// Loading may proceed as soon as one coherent graph has been accepted. A later
/// invalid hot reload leaves that graph available and records the rejection here.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeArtCatalogStatus {
    accepted_fingerprint: Option<u64>,
    resolving_update: bool,
    retained_error: Option<String>,
}

impl RuntimeArtCatalogStatus {
    /// Whether at least one coherent runtime art graph has been accepted.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.accepted_fingerprint.is_some()
    }

    /// Accepted graph fingerprint, or [`None`] before initial resolution succeeds.
    #[must_use]
    pub const fn accepted_fingerprint(&self) -> Option<u64> {
        self.accepted_fingerprint
    }

    /// Whether referenced object files for a newer manifest are still arriving.
    #[must_use]
    pub const fn is_resolving_update(&self) -> bool {
        self.resolving_update
    }

    /// Most recent rejected update, retained until sources change or resolve.
    #[must_use]
    pub fn retained_error(&self) -> Option<&str> {
        self.retained_error.as_deref()
    }

    fn awaiting(&mut self) {
        self.resolving_update = true;
        self.retained_error = None;
    }

    fn accepted(&mut self, fingerprint: u64) {
        self.accepted_fingerprint = Some(fingerprint);
        self.resolving_update = false;
        self.retained_error = None;
    }

    fn rejected(&mut self, error: String) -> bool {
        self.resolving_update = false;
        if self.retained_error.as_deref() == Some(error.as_str()) {
            return false;
        }
        self.retained_error = Some(error);
        true
    }
}

#[derive(Resource)]
struct ObjectBlueprintHandles {
    manifest: ObjectCatalogFile,
    handles: BTreeMap<ObjectAssetId, Handle<ObjectBlueprint>>,
}

fn sync_object_blueprint_handles(
    mut commands: Commands,
    manifest: Option<Res<ObjectCatalogFile>>,
    current: Option<Res<ObjectBlueprintHandles>>,
    asset_server: Res<AssetServer>,
    mut status: ResMut<RuntimeArtCatalogStatus>,
) {
    let Some(manifest) = manifest else {
        return;
    };
    if current
        .as_deref()
        .is_some_and(|current| current.manifest == *manifest)
    {
        return;
    }

    let mut handles = BTreeMap::new();
    for id in manifest.ids() {
        let Ok(path) = id.asset_path() else {
            status.rejected(format!(
                "object catalog contains non-canonical id '{}'",
                id.as_str()
            ));
            return;
        };
        handles.insert(id.clone(), asset_server.load::<ObjectBlueprint>(path));
    }
    status.awaiting();
    commands.insert_resource(ObjectBlueprintHandles {
        manifest: manifest.as_ref().clone(),
        handles,
    });
}

fn resolve_runtime_art_catalog(
    mut commands: Commands,
    palette: Option<Res<ArtPalette>>,
    styles: Option<Res<VoxelStyleCatalog>>,
    manifest: Option<Res<ObjectCatalogFile>>,
    handles: Option<Res<ObjectBlueprintHandles>>,
    blueprints: Res<Assets<ObjectBlueprint>>,
    accepted: Option<Res<RuntimeArtCatalog>>,
    mut changes: MessageReader<AssetEvent<ObjectBlueprint>>,
    mut failures: MessageReader<bevy::asset::AssetLoadFailedEvent<ObjectBlueprint>>,
    mut status: ResMut<RuntimeArtCatalogStatus>,
    mut registry: ResMut<SettingsRegistry>,
) {
    let mut changed_assets = BTreeSet::new();
    let mut successfully_loaded_assets = BTreeSet::new();
    for event in changes.read() {
        match event {
            AssetEvent::Added { id } | AssetEvent::Modified { id } => {
                changed_assets.extend([*id]);
                successfully_loaded_assets.extend([*id]);
            }
            AssetEvent::Removed { id } | AssetEvent::LoadedWithDependencies { id } => {
                changed_assets.extend([*id]);
            }
            AssetEvent::Unused { .. } => {}
        }
    }
    let failed_loads: Vec<_> = failures
        .read()
        .map(|failure| (failure.id, failure.error.to_string()))
        .collect();
    let (Some(palette), Some(styles), Some(manifest), Some(handles)) =
        (palette, styles, manifest, handles)
    else {
        return;
    };
    if handles.manifest != *manifest {
        return;
    }

    let referenced_asset_changed = handles
        .handles
        .values()
        .any(|handle| changed_assets.contains(&handle.id()));
    let candidate_changed = palette.is_changed()
        || styles.is_changed()
        || manifest.is_changed()
        || handles.is_changed()
        || referenced_asset_changed;
    let referenced_failure = handles.handles.iter().find_map(|(id, handle)| {
        failed_loads
            .iter()
            .find(|(failed_id, _)| *failed_id == handle.id())
            .map(|(_, error)| (id, error))
    });
    let unpaired_failure = handles.handles.iter().find_map(|(id, handle)| {
        failed_loads
            .iter()
            .find(|(failed_id, _)| *failed_id == handle.id())
            .filter(|(failed_id, _)| !successfully_loaded_assets.contains(failed_id))
            .map(|(_, error)| (id, error))
    });
    if !candidate_changed && unpaired_failure.is_none() {
        return;
    }

    let mut objects = BTreeMap::new();
    for (id, handle) in &handles.handles {
        let Some(blueprint) = blueprints.get(handle) else {
            if let Some((failed_id, error)) = referenced_failure {
                reject_candidate(
                    &mut status,
                    accepted.as_deref(),
                    &mut registry,
                    format!("could not load object '{}': {error}", failed_id.as_str()),
                );
            } else {
                status.resolving_update = true;
            }
            return;
        };
        objects.insert(id.clone(), blueprint.clone());
    }

    if let Some(accepted) = accepted
        .as_deref()
        .filter(|accepted| accepted.matches_sources(&palette, &styles, &manifest, &objects))
    {
        if let Some((id, error)) = unpaired_failure {
            reject_candidate(
                &mut status,
                Some(accepted),
                &mut registry,
                format!("could not load object '{}': {error}", id.as_str()),
            );
            return;
        }
        let fingerprint = accepted.combined_fingerprint();
        status.accepted(fingerprint);
        registry.clear_pending::<RuntimeArtCatalog>();
        return;
    }

    match RuntimeArtCatalog::from_sources(&palette, &styles, &manifest, objects) {
        Ok(resolved) => {
            let fingerprint = resolved.combined_fingerprint();
            commands.insert_resource(resolved);
            status.accepted(fingerprint);
            registry.clear_pending::<RuntimeArtCatalog>();
        }
        Err(error) => {
            reject_candidate(
                &mut status,
                accepted.as_deref(),
                &mut registry,
                error.to_string(),
            );
        }
    }
}

fn reject_candidate(
    status: &mut RuntimeArtCatalogStatus,
    accepted: Option<&RuntimeArtCatalog>,
    registry: &mut SettingsRegistry,
    error: String,
) {
    let newly_rejected = status.rejected(error.clone());
    if let Some(accepted) = accepted {
        status.accepted_fingerprint = Some(accepted.combined_fingerprint());
        registry.clear_pending::<RuntimeArtCatalog>();
        if newly_rejected {
            error!(
                "could not resolve authored object assets: {error}; retaining the previous valid \
                 runtime art catalog"
            );
        }
    } else if newly_rejected {
        error!(
            "could not resolve initial authored object assets: {error}; Loading remains blocked"
        );
    }
}

/// One of the six exact clockwise axial rotations of a voxel object.
///
/// This is a relative turn count: zero preserves the blueprint's authored
/// orientation. [`hex_core::Sextant`] instead names an absolute grid direction,
/// so the two six-way vocabularies are deliberately not interchangeable.
#[derive(Reflect, Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[reflect(opaque)]
pub struct HexObjectRotation(u8);

impl HexObjectRotation {
    /// No horizontal rotation.
    pub const ZERO: Self = Self(0);

    /// Creates a rotation from clockwise 60-degree steps.
    pub fn new(steps: u8) -> Result<Self, ObjectInstanceError> {
        let rotation = Self(steps);
        rotation.validate()?;
        Ok(rotation)
    }

    /// Revalidates this rotation before it crosses into presentation code.
    ///
    /// Constructors and RON deserialization already enforce the invariant. This
    /// remains public so consumers can defend against values injected by tooling or
    /// future migration code.
    pub fn validate(self) -> Result<(), ObjectInstanceError> {
        if self.0 >= 6 {
            return Err(ObjectInstanceError::InvalidRotation { steps: self.0 });
        }
        Ok(())
    }

    /// Clockwise 60-degree steps in `0..6`.
    #[must_use]
    pub const fn steps(self) -> u8 {
        self.0
    }

    /// Rotates one local axial coordinate around the object's origin.
    #[must_use]
    pub fn rotate_axial(
        self,
        mut position: LocalAxialCoord,
        pivot: LocalAxialCoord,
    ) -> Option<LocalAxialCoord> {
        for _ in 0..self.0 {
            position = position.rotated_clockwise_60(pivot)?;
        }
        Some(position)
    }

    /// Rotates one local voxel coordinate around the object's origin.
    #[must_use]
    pub fn rotate_voxel(
        self,
        mut position: LocalVoxelCoord,
        pivot: LocalVoxelCoord,
    ) -> Option<LocalVoxelCoord> {
        for _ in 0..self.0 {
            position = position.rotated_clockwise_60(pivot)?;
        }
        Some(position)
    }
}

impl<'de> Deserialize<'de> for HexObjectRotation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let steps = u8::deserialize(deserializer)?;
        Self::new(steps).map_err(serde::de::Error::custom)
    }
}

/// Invalid runtime placement metadata for an authored object.
#[derive(Debug, Clone, PartialEq)]
pub enum ObjectInstanceError {
    /// The stable id did not identify one canonical object asset path.
    InvalidObjectId {
        /// Invalid stable id.
        object_id: ObjectAssetId,
        /// Path-contract failure detail.
        reason: String,
    },
    /// The exact six-way rotation was outside `0..6`.
    InvalidRotation {
        /// Invalid clockwise step count.
        steps: u8,
    },
    /// Vertical scale was non-finite or non-positive.
    InvalidLevelHeight {
        /// Invalid world-unit height.
        level_height: f32,
    },
}

impl fmt::Display for ObjectInstanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidObjectId { object_id, reason } => {
                write!(formatter, "invalid object id '{object_id}': {reason}")
            }
            Self::InvalidRotation { steps } => {
                write!(
                    formatter,
                    "object rotation steps must be within 0..6, received {steps}"
                )
            }
            Self::InvalidLevelHeight { level_height } => write!(
                formatter,
                "object level height must be positive and finite, received {level_height}"
            ),
        }
    }
}

impl std::error::Error for ObjectInstanceError {}

/// Renderer-neutral request to place one authored object at an exact world voxel.
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
#[reflect(opaque)]
#[reflect(Component)]
pub struct ObjectInstance {
    object_id: ObjectAssetId,
    origin: TilePos,
    level_height: f32,
    rotation: HexObjectRotation,
}

impl ObjectInstance {
    /// Creates a validated runtime object placement.
    pub fn new(
        object_id: ObjectAssetId,
        origin: TilePos,
        level_height: f32,
        rotation: HexObjectRotation,
    ) -> Result<Self, ObjectInstanceError> {
        let instance = Self {
            object_id,
            origin,
            level_height,
            rotation,
        };
        instance.validate()?;
        Ok(instance)
    }

    /// Revalidates all runtime placement invariants before rendering.
    ///
    /// This is intentionally separate from [`Self::new`]: presentation adapters can
    /// call it defensively on ECS values supplied by inspectors or migration code.
    pub fn validate(&self) -> Result<(), ObjectInstanceError> {
        if let Err(error) = self.object_id.category() {
            return Err(ObjectInstanceError::InvalidObjectId {
                object_id: self.object_id.clone(),
                reason: error.to_string(),
            });
        }
        if !self.level_height.is_finite() || self.level_height <= 0.0 {
            return Err(ObjectInstanceError::InvalidLevelHeight {
                level_height: self.level_height,
            });
        }
        self.rotation.validate()
    }

    /// Stable authored object id.
    #[must_use]
    pub const fn object_id(&self) -> &ObjectAssetId {
        &self.object_id
    }

    /// Exact world voxel occupied by the blueprint's local origin.
    #[must_use]
    pub const fn origin(&self) -> TilePos {
        self.origin
    }

    /// World-unit height of one authored voxel level.
    #[must_use]
    pub const fn level_height(&self) -> f32 {
        self.level_height
    }

    /// Exact horizontal orientation.
    #[must_use]
    pub const fn rotation(&self) -> HexObjectRotation {
        self.rotation
    }
}

struct SemanticFingerprint {
    state: u64,
}

impl SemanticFingerprint {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    fn new(domain: &[u8]) -> Self {
        let mut encoder = Self {
            state: Self::OFFSET,
        };
        encoder.bytes(domain);
        encoder
    }

    fn bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.state ^= u64::from(*byte);
            self.state = self.state.wrapping_mul(Self::PRIME);
        }
    }

    fn u16(&mut self, value: u16) {
        self.bytes(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
    }

    fn usize(&mut self, value: usize) {
        self.u64(u64::try_from(value).unwrap_or(u64::MAX));
    }

    fn string(&mut self, value: &str) {
        self.usize(value.len());
        self.bytes(value.as_bytes());
    }

    const fn finish(self) -> u64 {
        self.state
    }
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use super::*;
    use crate::{
        ConnectivityPolicy, EffectPart, ObjectBounds, ObjectPart, ObjectPlacement, PaletteSwatch,
        PropPart, VoxelSurfaceMode, OBJECT_BLUEPRINT_SCHEMA_VERSION,
    };
    use bevy::asset::{AssetLoadError, AssetLoadFailedEvent, AssetPath};
    use bevy::reflect::ReflectRef;
    use hex_test_app::HeadlessAppBuilder;

    fn id(value: &str) -> ObjectAssetId {
        ObjectAssetId::new(value).expect("fixture id should use stable-id syntax")
    }

    fn swatch_id(value: &str) -> crate::SwatchId {
        crate::SwatchId::new(value).expect("fixture swatch id should be valid")
    }

    fn style_id(value: &str) -> VoxelStyleId {
        VoxelStyleId::new(value).expect("fixture style id should be valid")
    }

    fn palette(color: [f32; 3]) -> ArtPalette {
        ArtPalette::new(BTreeMap::from([(
            swatch_id("effect/core"),
            PaletteSwatch::new(
                "Core",
                SrgbColor::new(color[0], color[1], color[2])
                    .expect("fixture color should be valid"),
                BTreeSet::from(["effect".to_owned()]),
            )
            .expect("fixture swatch should be valid"),
        )]))
        .expect("fixture palette should be valid")
    }

    fn styles(swatch: &str) -> VoxelStyleCatalog {
        VoxelStyleCatalog::new(BTreeMap::from([(
            style_id("effect/core"),
            VoxelStyle::new(
                "Core",
                swatch_id(swatch),
                VoxelSurfaceMode::Opaque,
                1.0,
                None,
            )
            .expect("fixture style should be locally valid"),
        )]))
        .expect("fixture catalog should be locally valid")
    }

    fn blueprint() -> ObjectBlueprint {
        ObjectBlueprint {
            schema_version: OBJECT_BLUEPRINT_SCHEMA_VERSION,
            id: id("effect/glow"),
            display_name: "Glow".to_owned(),
            category: ObjectCategory::Effect,
            bounds: ObjectBounds {
                radius: 0,
                min_level: 0,
                height: 1,
            },
            connectivity: ConnectivityPolicy::Free,
            origin: LocalVoxelCoord::new(0, 0, 0),
            placements: vec![ObjectPlacement {
                position: LocalVoxelCoord::new(0, 0, 0),
                style: style_id("effect/core"),
                part: ObjectPart::Effect(EffectPart::Core),
            }],
            blocker_footprint: Vec::new(),
            canopy_occluders: Vec::new(),
        }
    }

    fn manifest() -> ObjectCatalogFile {
        ObjectCatalogFile::new([id("effect/glow")]).expect("fixture manifest should be valid")
    }

    fn objects() -> BTreeMap<ObjectAssetId, ObjectBlueprint> {
        BTreeMap::from([(id("effect/glow"), blueprint())])
    }

    #[test]
    fn object_paths_are_category_safe_and_canonical() {
        let plant = id("plant/small-broadleaf");
        assert_eq!(plant.category(), Ok(ObjectCategory::Plant));
        assert_eq!(plant.file_name(), Ok("small-broadleaf"));
        assert_eq!(
            plant.asset_path().as_deref(),
            Ok("art/objects/plant/small-broadleaf.ron")
        );
        assert_eq!(plant.validate_for_category(ObjectCategory::Plant), Ok(()));
        assert!(plant.validate_for_category(ObjectCategory::Effect).is_err());
        assert!(id("plant/trees/oak").asset_path().is_err());
        assert!(id("misc/oak").asset_path().is_err());
    }

    #[test]
    fn manifest_constructor_sorts_and_rejects_duplicates() {
        let catalog =
            ObjectCatalogFile::new([id("prop/stone"), id("plant/oak"), id("effect/glow")])
                .expect("fixture manifest should validate");
        assert_eq!(
            catalog
                .ids()
                .iter()
                .map(ObjectAssetId::as_str)
                .collect::<Vec<_>>(),
            ["effect/glow", "plant/oak", "prop/stone"]
        );
        assert!(ObjectCatalogFile::new([id("plant/oak"), id("plant/oak")]).is_err());
    }

    #[test]
    fn manifest_ron_requires_canonical_order() {
        let valid = "(schema_version:1,objects:[\"effect/glow\",\"plant/oak\",\"prop/stone\"])";
        assert!(ron::from_str::<ObjectCatalogFile>(valid).is_ok());
        let reordered = "(schema_version:1,objects:[\"plant/oak\",\"effect/glow\",\"prop/stone\"])";
        assert!(ron::from_str::<ObjectCatalogFile>(reordered).is_err());
        let duplicate = "(schema_version:1,objects:[\"plant/oak\",\"plant/oak\"])";
        assert!(ron::from_str::<ObjectCatalogFile>(duplicate).is_err());
    }

    #[test]
    fn rotations_and_instances_reject_invalid_runtime_values() {
        assert_eq!(
            HexObjectRotation::new(5).map(HexObjectRotation::steps),
            Ok(5)
        );
        assert!(HexObjectRotation::new(6).is_err());
        assert!(ron::from_str::<HexObjectRotation>("5").is_ok());
        assert!(ron::from_str::<HexObjectRotation>("6").is_err());
        let origin = TilePos::ORIGIN;
        assert!(ObjectInstance::new(id("plant/oak"), origin, 0.4, HexObjectRotation::ZERO).is_ok());
        assert!(
            ObjectInstance::new(id("plant/oak"), origin, f32::NAN, HexObjectRotation::ZERO)
                .is_err()
        );
        assert!(ObjectInstance::new(id("misc/oak"), origin, 0.4, HexObjectRotation::ZERO).is_err());

        let mut malformed =
            ObjectInstance::new(id("plant/oak"), origin, 0.4, HexObjectRotation::ZERO)
                .expect("fixture instance should be valid");
        malformed.level_height = f32::NAN;
        assert!(matches!(
            malformed.validate(),
            Err(ObjectInstanceError::InvalidLevelHeight { .. })
        ));
        malformed.level_height = 0.4;
        malformed.rotation = HexObjectRotation(6);
        assert!(matches!(
            malformed.validate(),
            Err(ObjectInstanceError::InvalidRotation { steps: 6 })
        ));
        assert!(matches!(malformed.reflect_ref(), ReflectRef::Opaque(_)));
        assert!(matches!(
            HexObjectRotation::ZERO.reflect_ref(),
            ReflectRef::Opaque(_)
        ));
    }

    #[test]
    fn resolved_catalog_validates_and_resolves_the_complete_graph() {
        let palette = palette([0.2, 0.4, 0.6]);
        let style_catalog = styles("effect/core");
        let manifest = manifest();
        let resolved =
            RuntimeArtCatalog::from_sources(&palette, &style_catalog, &manifest, objects())
                .expect("coherent fixture graph should resolve");

        assert_eq!(
            resolved
                .style(&style_id("effect/core"))
                .expect("resolved style should exist")
                .base_color()
                .to_array()
                .map(f32::to_bits),
            [0.2, 0.4, 0.6].map(f32::to_bits)
        );
        assert!(resolved.object(&id("effect/glow")).is_some());
        assert!(resolved.matches_sources(&palette, &style_catalog, &manifest, &objects()));

        let missing_style = styles("effect/missing");
        assert!(
            RuntimeArtCatalog::from_sources(&palette, &missing_style, &manifest, objects())
                .is_err()
        );
        assert!(RuntimeArtCatalog::from_sources(
            &palette,
            &style_catalog,
            &manifest,
            BTreeMap::new()
        )
        .is_err());
    }

    #[test]
    fn resolved_fingerprints_are_order_independent_and_cover_each_layer() {
        let original = RuntimeArtCatalog::from_sources(
            &palette([0.2, 0.4, 0.6]),
            &styles("effect/core"),
            &manifest(),
            objects(),
        )
        .expect("fixture graph should resolve");
        let same = RuntimeArtCatalog::from_sources(
            &palette([0.2, 0.4, 0.6]),
            &styles("effect/core"),
            &manifest(),
            objects(),
        )
        .expect("equivalent graph should resolve");
        assert_eq!(original.combined_fingerprint(), same.combined_fingerprint());

        let recolored = RuntimeArtCatalog::from_sources(
            &palette([0.8, 0.1, 0.3]),
            &styles("effect/core"),
            &manifest(),
            objects(),
        )
        .expect("recolored graph should resolve");
        assert_ne!(
            original.palette_fingerprint(),
            recolored.palette_fingerprint()
        );
        assert_eq!(original.style_fingerprint(), recolored.style_fingerprint());
        assert_eq!(
            original.object_fingerprint(),
            recolored.object_fingerprint()
        );
        assert_ne!(
            original.combined_fingerprint(),
            recolored.combined_fingerprint()
        );
    }

    fn resolver_app() -> (App, Handle<ObjectBlueprint>) {
        let mut builder = HeadlessAppBuilder::new()
            .with_minimal_plugins()
            .with_asset_plugin();
        builder
            .app_mut()
            .init_asset::<ObjectBlueprint>()
            .init_resource::<SettingsRegistry>()
            .init_resource::<RuntimeArtCatalogStatus>()
            .add_systems(Update, resolve_runtime_art_catalog);
        builder
            .app_mut()
            .world_mut()
            .resource_mut::<SettingsRegistry>()
            .mark_pending::<RuntimeArtCatalog>();
        builder.app_mut().insert_resource(palette([0.2, 0.4, 0.6]));
        builder.app_mut().insert_resource(styles("effect/core"));
        let manifest = manifest();
        builder.app_mut().insert_resource(manifest.clone());
        let handle = builder
            .app_mut()
            .world_mut()
            .resource_mut::<Assets<ObjectBlueprint>>()
            .add(blueprint());
        builder.app_mut().insert_resource(ObjectBlueprintHandles {
            manifest,
            handles: BTreeMap::from([(id("effect/glow"), handle.clone())]),
        });
        (builder.build(), handle)
    }

    fn report_object_load_failure(app: &mut App, handle: &Handle<ObjectBlueprint>) {
        let path = "art/objects/effect/glow.ron";
        app.world_mut()
            .resource_mut::<Messages<AssetLoadFailedEvent<ObjectBlueprint>>>()
            .write(AssetLoadFailedEvent {
                id: handle.id(),
                path: AssetPath::from(path),
                error: AssetLoadError::MissingAssetLoader {
                    asset_type_id: Some(TypeId::of::<ObjectBlueprint>()),
                    asset_path: path.to_owned(),
                },
            });
    }

    #[test]
    fn resolver_blocks_initial_loading_then_retains_the_last_valid_graph() {
        let (mut app, _) = resolver_app();
        assert!(!app.world().resource::<SettingsRegistry>().all_loaded());

        app.update();
        let accepted = app
            .world()
            .resource::<RuntimeArtCatalog>()
            .combined_fingerprint();
        assert!(app.world().resource::<RuntimeArtCatalogStatus>().is_ready());
        assert!(app.world().resource::<SettingsRegistry>().all_loaded());

        app.insert_resource(styles("effect/missing"));
        app.update();
        assert_eq!(
            app.world()
                .resource::<RuntimeArtCatalog>()
                .combined_fingerprint(),
            accepted,
            "an invalid update must not replace the accepted catalog"
        );
        let status = app.world().resource::<RuntimeArtCatalogStatus>();
        assert!(status.is_ready());
        assert!(status
            .retained_error()
            .is_some_and(|error| error.contains("missing base swatch")));
    }

    #[test]
    fn same_frame_reversion_and_opposite_edit_use_the_current_sources() {
        let (mut app, _) = resolver_app();
        app.update();
        let original = app
            .world()
            .resource::<RuntimeArtCatalog>()
            .combined_fingerprint();

        app.insert_resource(styles("effect/missing"));
        app.update();
        assert!(app
            .world()
            .resource::<RuntimeArtCatalogStatus>()
            .retained_error()
            .is_some());

        app.insert_resource(styles("effect/core"));
        app.insert_resource(palette([0.8, 0.1, 0.3]));
        app.update();
        let catalog = app.world().resource::<RuntimeArtCatalog>();
        assert_ne!(catalog.combined_fingerprint(), original);
        assert_eq!(
            catalog
                .style(&style_id("effect/core"))
                .expect("resolved style should exist")
                .base_color()
                .to_array()
                .map(f32::to_bits),
            [0.8, 0.1, 0.3].map(f32::to_bits)
        );
        assert!(app
            .world()
            .resource::<RuntimeArtCatalogStatus>()
            .retained_error()
            .is_none());
    }

    #[test]
    fn no_change_frame_does_not_republish_the_resolved_catalog_or_status() {
        let (mut app, _) = resolver_app();
        app.update();
        app.world_mut()
            .resource_mut::<Messages<AssetEvent<ObjectBlueprint>>>()
            .clear();
        app.world_mut().clear_trackers();

        app.update();

        assert!(
            !app.world().resource_ref::<RuntimeArtCatalog>().is_changed(),
            "an idle frame should not clone and republish the object graph"
        );
        assert!(
            !app.world()
                .resource_ref::<RuntimeArtCatalogStatus>()
                .is_changed(),
            "an idle frame should not rewrite readiness state"
        );
    }

    #[test]
    fn object_asset_events_revalidate_without_replacing_on_failure() {
        let (mut app, handle) = resolver_app();
        app.update();
        let accepted = app
            .world()
            .resource::<RuntimeArtCatalog>()
            .combined_fingerprint();
        let mut invalid = blueprint();
        let Some(first) = invalid.placements.first_mut() else {
            unreachable!("the fixture blueprint always contains its core placement");
        };
        first.style = style_id("effect/missing");
        app.world_mut()
            .resource_mut::<Assets<ObjectBlueprint>>()
            .insert(handle.id(), invalid)
            .expect("fixture object handle should remain allocated");
        app.world_mut()
            .resource_mut::<Messages<AssetEvent<ObjectBlueprint>>>()
            .write(AssetEvent::Modified { id: handle.id() });

        app.update();

        assert_eq!(
            app.world()
                .resource::<RuntimeArtCatalog>()
                .combined_fingerprint(),
            accepted
        );
        assert!(app
            .world()
            .resource::<RuntimeArtCatalogStatus>()
            .retained_error()
            .is_some_and(|error| error.contains("missing voxel style")));
    }

    #[test]
    fn same_frame_object_failure_and_success_use_the_current_coherent_blueprint() {
        let (mut app, handle) = resolver_app();
        app.update();
        let original = app
            .world()
            .resource::<RuntimeArtCatalog>()
            .combined_fingerprint();

        report_object_load_failure(&mut app, &handle);
        let mut updated = blueprint();
        updated.display_name = "Revised Glow".to_owned();
        app.world_mut()
            .resource_mut::<Assets<ObjectBlueprint>>()
            .insert(handle.id(), updated)
            .expect("fixture object handle should remain allocated");
        app.world_mut()
            .resource_mut::<Messages<AssetEvent<ObjectBlueprint>>>()
            .write(AssetEvent::Modified { id: handle.id() });

        app.update();

        let catalog = app.world().resource::<RuntimeArtCatalog>();
        assert_ne!(catalog.combined_fingerprint(), original);
        assert_eq!(
            catalog
                .object(&id("effect/glow"))
                .expect("updated object should remain resolved")
                .display_name,
            "Revised Glow"
        );
        assert!(
            app.world()
                .resource::<RuntimeArtCatalogStatus>()
                .retained_error()
                .is_none(),
            "the stale failure must not overwrite a successful same-frame reload"
        );
    }

    #[test]
    fn object_failure_without_a_matching_success_retains_the_previous_graph() {
        let (mut app, handle) = resolver_app();
        app.update();
        app.world_mut()
            .resource_mut::<Messages<AssetEvent<ObjectBlueprint>>>()
            .clear();
        let original = app
            .world()
            .resource::<RuntimeArtCatalog>()
            .combined_fingerprint();

        report_object_load_failure(&mut app, &handle);
        app.update();

        assert_eq!(
            app.world()
                .resource::<RuntimeArtCatalog>()
                .combined_fingerprint(),
            original
        );
        assert!(
            app.world()
                .resource::<RuntimeArtCatalogStatus>()
                .retained_error()
                .is_some_and(|error| error.contains("could not load object")),
            "a lone load failure should remain visible while the accepted graph stays usable"
        );
    }

    #[test]
    fn shipped_manifest_objects_and_art_catalog_resolve_together() {
        let palette: ArtPalette = ron::from_str(include_str!("../../../assets/art/palette.ron"))
            .expect("shipped palette should parse");
        let styles: VoxelStyleCatalog =
            ron::from_str(include_str!("../../../assets/art/voxel_styles.ron"))
                .expect("shipped styles should parse");
        let manifest: ObjectCatalogFile =
            ron::from_str(include_str!("../../../assets/art/object_catalog.ron"))
                .expect("shipped object manifest should parse");
        let objects = [
            include_str!("../../../assets/art/objects/plant/old-growth.ron"),
            include_str!("../../../assets/art/objects/plant/small-broadleaf.ron"),
            include_str!("../../../assets/art/objects/plant/snowy-old-growth.ron"),
            include_str!("../../../assets/art/objects/plant/snowy-small-broadleaf.ron"),
            include_str!("../../../assets/art/objects/plant/snowy-tall-narrow.ron"),
            include_str!("../../../assets/art/objects/plant/tall-narrow.ron"),
            include_str!("../../../assets/art/objects/prop/cave-lichen.ron"),
            include_str!("../../../assets/art/objects/prop/cave-moss.ron"),
            include_str!("../../../assets/art/objects/prop/crystal-branched.ron"),
            include_str!("../../../assets/art/objects/prop/crystal-cathedral-heart.ron"),
            include_str!("../../../assets/art/objects/prop/crystal-low-cluster.ron"),
            include_str!("../../../assets/art/objects/prop/crystal-spire.ron"),
            include_str!("../../../assets/art/objects/prop/grass-tuft.ron"),
            include_str!("../../../assets/art/objects/prop/snowy-grass-tuft.ron"),
        ]
        .into_iter()
        .map(|source| {
            let object: ObjectBlueprint =
                ron::from_str(source).expect("shipped object should parse");
            (object.id.clone(), object)
        })
        .collect::<BTreeMap<_, _>>();
        let expected_ids = [
            "plant/old-growth",
            "plant/small-broadleaf",
            "plant/snowy-old-growth",
            "plant/snowy-small-broadleaf",
            "plant/snowy-tall-narrow",
            "plant/tall-narrow",
            "prop/cave-lichen",
            "prop/cave-moss",
            "prop/crystal-branched",
            "prop/crystal-cathedral-heart",
            "prop/crystal-low-cluster",
            "prop/crystal-spire",
            "prop/grass-tuft",
            "prop/snowy-grass-tuft",
        ]
        .map(id);
        assert_eq!(manifest.ids(), expected_ids.as_slice());

        let resolved = RuntimeArtCatalog::from_sources(&palette, &styles, &manifest, objects)
            .expect("shipped authored object graph should resolve");
        assert_eq!(resolved.objects().len(), 14);
        assert_eq!(resolved.styles().styles().len(), 8);

        let small = resolved
            .object(&id("plant/small-broadleaf"))
            .expect("small tree should resolve");
        assert_eq!(small.bounds.height, 9);
        assert_eq!(small.blocker_footprint, [LocalAxialCoord::new(0, 0)]);
        assert_eq!(
            small
                .placements
                .iter()
                .map(|placement| placement.position.level)
                .max(),
            Some(8)
        );

        let tall = resolved
            .object(&id("plant/tall-narrow"))
            .expect("tall tree should resolve");
        assert_eq!(tall.bounds.height, 16);
        assert_eq!(tall.blocker_footprint, [LocalAxialCoord::new(0, 0)]);
        assert_eq!(
            tall.placements
                .iter()
                .map(|placement| placement.position.level)
                .max(),
            Some(15)
        );

        let old_growth = resolved
            .object(&id("plant/old-growth"))
            .expect("old-growth tree should resolve");
        assert_eq!(old_growth.bounds.height, 21);
        assert_eq!(old_growth.blocker_footprint.len(), 7);
        assert_eq!(
            old_growth
                .blocker_footprint
                .iter()
                .copied()
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                LocalAxialCoord::new(-1, 0),
                LocalAxialCoord::new(-1, 1),
                LocalAxialCoord::new(0, -1),
                LocalAxialCoord::new(0, 0),
                LocalAxialCoord::new(0, 1),
                LocalAxialCoord::new(1, -1),
                LocalAxialCoord::new(1, 0),
            ])
        );
        assert_eq!(
            old_growth
                .placements
                .iter()
                .map(|placement| placement.position.level)
                .max(),
            Some(20)
        );

        let grass = resolved
            .object(&id("prop/grass-tuft"))
            .expect("grass tuft should resolve");
        assert_eq!(grass.category, ObjectCategory::Prop);
        assert!(grass.blocker_footprint.is_empty());
        assert_eq!(grass.bounds.height, 1);

        for (snowy_id, expected_height, expected_blockers) in [
            ("plant/snowy-small-broadleaf", 9, 1),
            ("plant/snowy-tall-narrow", 16, 1),
            ("plant/snowy-old-growth", 21, 7),
        ] {
            let snowy = resolved
                .object(&id(snowy_id))
                .expect("snowy tree should resolve");
            assert_eq!(snowy.category, ObjectCategory::Plant);
            assert_eq!(snowy.bounds.height, expected_height);
            assert_eq!(snowy.blocker_footprint.len(), expected_blockers);
            assert!(!snowy.canopy_occluders.is_empty());
        }

        for nonblocking_id in [
            "prop/cave-lichen",
            "prop/cave-moss",
            "prop/snowy-grass-tuft",
        ] {
            let vegetation = resolved
                .object(&id(nonblocking_id))
                .expect("nonblocking vegetation should resolve");
            assert_eq!(vegetation.category, ObjectCategory::Prop);
            assert!(vegetation.blocker_footprint.is_empty());
            assert!(vegetation.canopy_occluders.is_empty());
        }

        for vegetation_id in [
            "plant/old-growth",
            "plant/small-broadleaf",
            "plant/snowy-old-growth",
            "plant/snowy-small-broadleaf",
            "plant/snowy-tall-narrow",
            "plant/tall-narrow",
            "prop/cave-lichen",
            "prop/cave-moss",
            "prop/grass-tuft",
            "prop/snowy-grass-tuft",
        ] {
            let vegetation = resolved
                .object(&id(vegetation_id))
                .expect("vegetation asset should resolve");
            let authored_positions = vegetation
                .placements
                .iter()
                .map(|placement| placement.position)
                .collect::<BTreeSet<_>>();
            for steps in 0..6 {
                let rotation =
                    HexObjectRotation::new(steps).expect("all six authored rotations are valid");
                let rotated_positions = vegetation
                    .placements
                    .iter()
                    .map(|placement| {
                        rotation
                            .rotate_voxel(placement.position, vegetation.origin)
                            .expect("shipped vegetation coordinates should rotate without overflow")
                    })
                    .collect::<BTreeSet<_>>();
                assert_eq!(
                    rotated_positions.len(),
                    authored_positions.len(),
                    "rotation {steps} overlaps '{vegetation_id}' placements"
                );
                assert!(
                    rotated_positions
                        .iter()
                        .all(|position| vegetation.bounds.contains(*position)),
                    "rotation {steps} moves '{vegetation_id}' outside its authored bounds"
                );

                let rotated_blockers = vegetation
                    .blocker_footprint
                    .iter()
                    .map(|blocker| {
                        rotation
                            .rotate_axial(*blocker, vegetation.origin.axial())
                            .expect("shipped blocker coordinates should rotate without overflow")
                    })
                    .collect::<BTreeSet<_>>();
                assert_eq!(
                    rotated_blockers.len(),
                    vegetation.blocker_footprint.len(),
                    "rotation {steps} overlaps '{vegetation_id}' blockers"
                );

                let rotated_canopy = vegetation
                    .canopy_occluders
                    .iter()
                    .map(|position| {
                        rotation
                            .rotate_voxel(*position, vegetation.origin)
                            .expect("shipped canopy coordinates should rotate without overflow")
                    })
                    .collect::<BTreeSet<_>>();
                assert_eq!(
                    rotated_canopy.len(),
                    vegetation.canopy_occluders.len(),
                    "rotation {steps} overlaps '{vegetation_id}' canopy cells"
                );
            }

            let full_turn = authored_positions
                .iter()
                .copied()
                .map(|mut position| {
                    for _ in 0..6 {
                        position = position.rotated_clockwise_60(vegetation.origin).expect(
                            "shipped vegetation coordinates should rotate without overflow",
                        );
                    }
                    position
                })
                .collect::<BTreeSet<_>>();
            assert_eq!(
                full_turn, authored_positions,
                "six turns should restore '{vegetation_id}' exactly"
            );
        }

        for crystal_id in [
            "prop/crystal-low-cluster",
            "prop/crystal-branched",
            "prop/crystal-spire",
        ] {
            let crystal = resolved
                .object(&id(crystal_id))
                .expect("crystal should resolve");
            assert_eq!(crystal.category, ObjectCategory::Prop);
            assert!(crystal.blocker_footprint.is_empty());
            assert!(crystal.canopy_occluders.is_empty());
        }
        let heart = resolved
            .object(&id("prop/crystal-cathedral-heart"))
            .expect("cathedral heart should resolve");
        assert_eq!(heart.category, ObjectCategory::Prop);
        assert_eq!(heart.connectivity, ConnectivityPolicy::Free);
        assert_eq!(heart.bounds.radius, 4);
        assert_eq!(heart.bounds.min_level, 0);
        assert_eq!(heart.bounds.height, 24);
        assert_eq!(heart.origin, LocalVoxelCoord::new(0, 0, 0));
        assert!(heart.canopy_occluders.is_empty());
        assert!(heart
            .placements
            .iter()
            .all(|placement| placement.part == ObjectPart::Prop(PropPart::Structure)));
        assert_eq!(
            heart
                .placements
                .iter()
                .map(|placement| placement.style.clone())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([style_id("crystal/cyan-body"), style_id("crystal/cyan-glow"),])
        );
        assert_eq!(
            heart
                .placements
                .iter()
                .map(|placement| placement.position.level)
                .collect::<BTreeSet<_>>(),
            (0..24).collect::<BTreeSet<_>>()
        );
        let expected_heart_footprint = (-4_i32..=4)
            .flat_map(|q| (-4_i32..=4).map(move |r| LocalAxialCoord::new(q, r)))
            .filter(|coord| {
                coord
                    .q
                    .abs()
                    .max(coord.r.abs())
                    .max((-coord.q - coord.r).abs())
                    <= 4
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            heart
                .blocker_footprint
                .iter()
                .copied()
                .collect::<BTreeSet<_>>(),
            expected_heart_footprint
        );
        let body_style = resolved
            .styles()
            .get(&style_id("crystal/cyan-body"))
            .expect("crystal body style should resolve");
        assert_eq!(body_style.surface_mode(), VoxelSurfaceMode::Opaque);
        assert!(body_style.emission().is_some());
        let glow_style = resolved
            .styles()
            .get(&style_id("crystal/cyan-glow"))
            .expect("crystal glow style should resolve");
        assert_eq!(glow_style.surface_mode(), VoxelSurfaceMode::Additive);
        assert!(glow_style.emission().is_some());

        assert_eq!(
            (
                manifest.semantic_fingerprint(),
                resolved.object_fingerprint(),
                resolved.combined_fingerprint(),
            ),
            (
                5_183_140_313_222_150_403,
                9_652_748_088_792_271_647,
                2_286_574_576_222_903_349,
            )
        );
        let expected_object_fingerprints = BTreeMap::from([
            (id("plant/old-growth"), 18_215_252_645_504_955_369),
            (id("plant/small-broadleaf"), 692_655_780_260_542_668),
            (id("plant/snowy-old-growth"), 16_803_730_044_443_536_229),
            (id("plant/snowy-small-broadleaf"), 7_195_704_118_276_503_348),
            (id("plant/snowy-tall-narrow"), 10_256_897_596_986_011_740),
            (id("plant/tall-narrow"), 6_591_765_473_067_103_716),
            (id("prop/cave-lichen"), 14_754_322_871_995_823_724),
            (id("prop/cave-moss"), 11_746_802_235_239_197_086),
            (id("prop/crystal-branched"), 632_179_240_403_471_067),
            (
                id("prop/crystal-cathedral-heart"),
                7_289_663_172_659_263_250,
            ),
            (id("prop/crystal-low-cluster"), 1_307_286_824_627_267_907),
            (id("prop/crystal-spire"), 1_248_030_652_803_885_799),
            (id("prop/grass-tuft"), 8_128_471_665_006_116_358),
            (id("prop/snowy-grass-tuft"), 10_601_105_736_077_673_696),
        ]);
        let actual_object_fingerprints = resolved
            .objects()
            .iter()
            .map(|(id, object)| {
                (
                    id.clone(),
                    object
                        .semantic_fingerprint()
                        .expect("resolved object should have a semantic fingerprint"),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(actual_object_fingerprints, expected_object_fingerprints);
    }
}
