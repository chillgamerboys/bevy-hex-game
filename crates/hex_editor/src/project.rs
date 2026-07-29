//! Filesystem project loading, validation, impact reporting, and explicit saves.
//!
//! The editor model owns mutable drafts. [`AssetProject`] represents the last state
//! successfully loaded from or written to the tracked
//! `assets/art` tree, so a failed validation or filesystem operation never leaves its
//! in-memory view ahead of disk.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use atomicwrites::{AllowOverwrite, AtomicFile, DisallowOverwrite, OverwriteBehavior};
use hex_assets::{
    ArtPalette, ObjectAssetId, ObjectBlueprint, ObjectCategory, PaletteSwatch, SwatchId,
    VoxelStyle, VoxelStyleCatalog, VoxelStyleId,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use xxhash_rust::xxh3::xxh3_64;

const ART_PATH: &str = "assets/art";
const PALETTE_FILE: &str = "palette.ron";
const STYLE_FILE: &str = "voxel_styles.ron";
const OBJECT_DIRECTORY: &str = "objects";
const OBJECT_CATEGORIES: [ObjectCategory; 3] = [
    ObjectCategory::Plant,
    ObjectCategory::Effect,
    ObjectCategory::Prop,
];

/// One object affected by a shared style or swatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectUsage {
    /// Stable object id.
    pub object: ObjectAssetId,
    /// Number of placements whose style is affected.
    pub placements: usize,
}

/// Transitive impact of changing or removing one palette swatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwatchUsage {
    /// Styles that use the swatch as their base or emission colour.
    pub styles: Vec<VoxelStyleId>,
    /// Objects using one or more of those styles.
    pub objects: Vec<ObjectUsage>,
}

/// How one tracked art source differs from the bytes loaded by this editor session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalChangeKind {
    /// A new tracked RON file appeared.
    Added,
    /// A previously loaded tracked RON file disappeared.
    Removed,
    /// A tracked RON file now contains different bytes.
    Modified,
}

/// One external art-source change, reported with an `assets/art`-relative path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalAssetChange {
    /// Stable relative path below `assets/art`.
    pub path: PathBuf,
    /// Nature of the byte-level change.
    pub kind: ExternalChangeKind,
}

/// Stable byte identity of one tracked source at a recovery checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ByteRevision {
    /// Exact source byte count.
    pub byte_len: u64,
    /// Stable XXH3 digest of the complete source bytes.
    pub fingerprint: u64,
}

/// Complete tracked art-source identity stored with a recovery draft.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectRevisionSet {
    /// Revisions keyed by normalized `assets/art`-relative path.
    pub files: BTreeMap<String, ByteRevision>,
}

/// Actionable failure from loading or persisting an asset project.
#[derive(Debug)]
pub struct ProjectError {
    operation: &'static str,
    path: Option<PathBuf>,
    detail: String,
}

impl ProjectError {
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

    /// Short machine-stable description of the failed operation.
    #[must_use]
    pub const fn operation(&self) -> &'static str {
        self.operation
    }

    /// Relevant filesystem path, when the failure concerns one.
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

impl fmt::Display for ProjectError {
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

impl std::error::Error for ProjectError {}

/// The validated, last-saved contents of one repository's art asset tree.
#[derive(Debug, Clone)]
pub struct AssetProject {
    repository_root: PathBuf,
    art_root: PathBuf,
    palette: ArtPalette,
    styles: VoxelStyleCatalog,
    objects: BTreeMap<ObjectAssetId, ObjectBlueprint>,
    loaded_sources: BTreeMap<PathBuf, Vec<u8>>,
}

impl AssetProject {
    /// Loads and validates `assets/art` below `repository_root`.
    ///
    /// Object directories may be absent before the first object is saved. Existing
    /// `.ron` files are discovered in stable category and filename order.
    pub fn load(repository_root: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let repository_root = repository_root.as_ref().to_path_buf();
        let art_root = repository_root.join(ART_PATH);
        let palette_path = art_root.join(PALETTE_FILE);
        let style_path = art_root.join(STYLE_FILE);
        let palette = read_ron::<ArtPalette>(&palette_path)?;
        let styles = read_ron::<VoxelStyleCatalog>(&style_path)?;
        let objects = load_objects(&art_root)?;
        let loaded_sources = scan_art_sources(&art_root)?;

        let project = Self {
            repository_root,
            art_root,
            palette,
            styles,
            objects,
            loaded_sources,
        };
        project.validate_graph()?;
        Ok(project)
    }

    /// Repository root containing this project's `assets` directory.
    #[must_use]
    pub fn repository_root(&self) -> &Path {
        &self.repository_root
    }

    /// Exact `assets/art` root.
    #[must_use]
    pub fn art_root(&self) -> &Path {
        &self.art_root
    }

    /// Last successfully loaded or saved palette.
    #[must_use]
    pub const fn palette(&self) -> &ArtPalette {
        &self.palette
    }

    /// Last successfully loaded or saved voxel-style catalog.
    #[must_use]
    pub const fn styles(&self) -> &VoxelStyleCatalog {
        &self.styles
    }

    /// Saved objects in stable id order.
    #[must_use]
    pub const fn objects(&self) -> &BTreeMap<ObjectAssetId, ObjectBlueprint> {
        &self.objects
    }

    /// Looks up one saved object.
    #[must_use]
    pub fn object(&self, id: &ObjectAssetId) -> Option<&ObjectBlueprint> {
        self.objects.get(id)
    }

    /// Reports saved objects that use `style`, ordered by object id.
    #[must_use]
    pub fn style_usage(&self, style: &VoxelStyleId) -> Vec<ObjectUsage> {
        object_usage_for_styles(&self.objects, std::iter::once(style))
    }

    /// Reports styles and saved objects transitively affected by `swatch`.
    #[must_use]
    pub fn swatch_usage(&self, swatch: &SwatchId) -> SwatchUsage {
        let styles = self.styles.references_to(swatch);
        let objects = object_usage_for_styles(&self.objects, styles.iter());
        SwatchUsage { styles, objects }
    }

    /// Reports byte-level on-disk changes made since this project loaded or saved.
    ///
    /// Formatting-only edits count as modifications. The Workshop never silently
    /// merges or overwrites them.
    pub fn external_changes(&self) -> Result<Vec<ExternalAssetChange>, ProjectError> {
        let current = scan_art_sources(&self.art_root)?;
        Ok(compare_sources(&self.loaded_sources, &current))
    }

    /// Captures the exact loaded byte revisions for crash-recovery conflict checks.
    #[must_use]
    pub fn revision_snapshot(&self) -> ProjectRevisionSet {
        revision_set_from_sources(&self.loaded_sources)
    }

    /// Discards the loaded project snapshot and reloads the complete art graph.
    pub fn reload_from_disk(&mut self) -> Result<(), ProjectError> {
        *self = Self::load(&self.repository_root)?;
        Ok(())
    }

    /// Validates and atomically replaces the shared palette file.
    ///
    /// Every existing style and object is checked against the proposed palette before
    /// the previous file is touched.
    pub fn save_palette(&mut self, palette: ArtPalette) -> Result<(), ProjectError> {
        self.ensure_no_external_changes("save palette")?;
        validate_graph(&palette, &self.styles, &self.objects)?;
        let path = self.art_root.join(PALETTE_FILE);
        let source = write_ron_atomically(&path, &palette, AllowOverwrite)?;
        self.record_written_source(&path, source)?;
        self.palette = palette;
        Ok(())
    }

    /// Validates and atomically replaces the shared voxel-style file.
    ///
    /// Every existing object is checked against the proposed catalog before the
    /// previous file is touched.
    pub fn save_styles(&mut self, styles: VoxelStyleCatalog) -> Result<(), ProjectError> {
        self.ensure_no_external_changes("save voxel styles")?;
        validate_graph(&self.palette, &styles, &self.objects)?;
        let path = self.art_root.join(STYLE_FILE);
        let source = write_ron_atomically(&path, &styles, AllowOverwrite)?;
        self.record_written_source(&path, source)?;
        self.styles = styles;
        Ok(())
    }

    /// Validates and explicitly saves coherent palette and style drafts.
    ///
    /// When both files change, the method chooses an order whose intermediate graph
    /// remains valid. An I/O failure on the second file atomically restores the first
    /// file before returning and never advances the in-memory project.
    pub fn save_catalogs(
        &mut self,
        palette: ArtPalette,
        styles: VoxelStyleCatalog,
    ) -> Result<(), ProjectError> {
        self.ensure_no_external_changes("save art catalogs")?;
        validate_graph(&palette, &styles, &self.objects)?;
        let palette_changed = palette != self.palette;
        let styles_changed = styles != self.styles;
        match (palette_changed, styles_changed) {
            (false, false) => return Ok(()),
            (true, false) => return self.save_palette(palette),
            (false, true) => return self.save_styles(styles),
            (true, true) => {}
        }

        let palette_path = self.art_root.join(PALETTE_FILE);
        let style_path = self.art_root.join(STYLE_FILE);
        let old_palette = self.palette.clone();
        let old_styles = self.styles.clone();
        let old_palette_source = self.loaded_source(&palette_path)?.to_vec();
        let old_style_source = self.loaded_source(&style_path)?.to_vec();
        let palette_first = validate_graph(&palette, &old_styles, &self.objects).is_ok();
        let styles_first = validate_graph(&old_palette, &styles, &self.objects).is_ok();

        if palette_first {
            let palette_source = write_ron_atomically(&palette_path, &palette, AllowOverwrite)?;
            let style_source =
                write_ron_atomically(&style_path, &styles, AllowOverwrite).map_err(|error| {
                    with_rollback(
                        error,
                        write_bytes_atomically(&palette_path, &old_palette_source, AllowOverwrite),
                        "palette",
                    )
                })?;
            self.record_written_source(&palette_path, palette_source)?;
            self.record_written_source(&style_path, style_source)?;
        } else if styles_first {
            let style_source = write_ron_atomically(&style_path, &styles, AllowOverwrite)?;
            let palette_source = write_ron_atomically(&palette_path, &palette, AllowOverwrite)
                .map_err(|error| {
                    with_rollback(
                        error,
                        write_bytes_atomically(&style_path, &old_style_source, AllowOverwrite),
                        "voxel styles",
                    )
                })?;
            self.record_written_source(&style_path, style_source)?;
            self.record_written_source(&palette_path, palette_source)?;
        } else {
            return Err(ProjectError::new(
                "save art catalogs",
                None,
                "palette and style changes have no valid intermediate graph; save additions before migrating references, then remove obsolete entries",
            ));
        }

        self.palette = palette;
        self.styles = styles;
        Ok(())
    }

    /// Saves changes to an existing object without changing its stable id.
    ///
    /// Use [`Self::save_object_as`] for an unsaved draft or an identity change.
    pub fn save_object(
        &mut self,
        expected_id: &ObjectAssetId,
        blueprint: ObjectBlueprint,
    ) -> Result<(), ProjectError> {
        if &blueprint.id != expected_id {
            return Err(ProjectError::new(
                "save object",
                None,
                format!(
                    "open object identity is '{}' but the draft attempted to save as '{}'; use Save As for identity changes",
                    expected_id.as_str(),
                    blueprint.id.as_str()
                ),
            ));
        }
        if !self.objects.contains_key(expected_id) {
            return Err(ProjectError::new(
                "save object",
                None,
                format!(
                    "object '{}' is not saved; use Save As for a new id",
                    expected_id.as_str()
                ),
            ));
        }
        self.ensure_no_external_changes("save object")?;
        let path = object_path(&self.art_root, &blueprint)?;
        let mut objects = self.objects.clone();
        drop(objects.insert(blueprint.id.clone(), blueprint.clone()));
        validate_graph(&self.palette, &self.styles, &objects)?;
        let source = write_ron_atomically(&path, &blueprint, AllowOverwrite)?;
        self.record_written_source(&path, source)?;
        self.objects = objects;
        Ok(())
    }

    /// Saves a draft under a new stable id and path.
    ///
    /// The supplied `new_id` replaces the draft's transient id. Existing project
    /// objects and files are never overwritten by Save As.
    pub fn save_object_as(
        &mut self,
        mut blueprint: ObjectBlueprint,
        new_id: ObjectAssetId,
    ) -> Result<(), ProjectError> {
        self.ensure_catalog_sources_unchanged("save object as")?;
        let mut refreshed = Self::load(&self.repository_root)?;
        if refreshed.objects.contains_key(&new_id) {
            return Err(ProjectError::new(
                "save object as",
                None,
                format!("object '{}' already exists", new_id.as_str()),
            ));
        }
        blueprint.id = new_id.clone();
        let path = object_path(&refreshed.art_root, &blueprint)?;
        if path
            .try_exists()
            .map_err(|error| ProjectError::at("inspect Save As destination", &path, error))?
        {
            return Err(ProjectError::at(
                "save object as",
                &path,
                "destination already exists",
            ));
        }

        let mut objects = refreshed.objects.clone();
        drop(objects.insert(new_id, blueprint.clone()));
        validate_graph(&refreshed.palette, &refreshed.styles, &objects)?;
        let source = write_ron_atomically(&path, &blueprint, DisallowOverwrite)?;
        refreshed.record_written_source(&path, source)?;
        refreshed.objects = objects;
        *self = refreshed;
        Ok(())
    }

    /// Duplicates a saved object under a new id and display name.
    pub fn duplicate_object(
        &mut self,
        source: &ObjectAssetId,
        new_id: ObjectAssetId,
        display_name: impl Into<String>,
    ) -> Result<(), ProjectError> {
        let mut duplicate = self.objects.get(source).cloned().ok_or_else(|| {
            ProjectError::new(
                "duplicate object",
                None,
                format!("object '{}' does not exist", source.as_str()),
            )
        })?;
        duplicate.display_name = display_name.into();
        self.save_object_as(duplicate, new_id)
    }

    /// Deletes an object file and removes it from the saved project.
    ///
    /// Object-to-object references do not exist in schema version 1, so no downstream
    /// reference guard is needed yet.
    pub fn delete_object(&mut self, id: &ObjectAssetId) -> Result<ObjectBlueprint, ProjectError> {
        self.ensure_no_external_changes("delete object")?;
        let blueprint = self.objects.get(id).cloned().ok_or_else(|| {
            ProjectError::new(
                "delete object",
                None,
                format!("object '{}' does not exist", id.as_str()),
            )
        })?;
        let path = object_path(&self.art_root, &blueprint)?;
        fs::remove_file(&path).map_err(|error| ProjectError::at("delete object", &path, error))?;
        self.remove_written_source(&path)?;
        drop(self.objects.remove(id));
        Ok(blueprint)
    }

    /// Deletes an unreferenced style and atomically saves the catalog.
    ///
    /// Returns an error listing object usage when any placement still names `id`.
    pub fn delete_style(&mut self, id: &VoxelStyleId) -> Result<VoxelStyle, ProjectError> {
        let usage = self.style_usage(id);
        if !usage.is_empty() {
            return Err(ProjectError::new(
                "delete voxel style",
                None,
                format!(
                    "style '{}' is referenced by {}",
                    id.as_str(),
                    format_object_usage(&usage)
                ),
            ));
        }

        let mut styles = self.styles.clone();
        let removed = styles.remove(id).ok_or_else(|| {
            ProjectError::new(
                "delete voxel style",
                None,
                format!("style '{}' does not exist", id.as_str()),
            )
        })?;
        self.save_styles(styles)?;
        Ok(removed)
    }

    /// Deletes an unreferenced swatch and atomically saves the palette.
    ///
    /// Returns an error listing direct style references and their transitive object
    /// usage when the swatch is still in use.
    pub fn delete_swatch(&mut self, id: &SwatchId) -> Result<PaletteSwatch, ProjectError> {
        let usage = self.swatch_usage(id);
        if !usage.styles.is_empty() {
            let styles = usage
                .styles
                .iter()
                .map(VoxelStyleId::as_str)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(ProjectError::new(
                "delete palette swatch",
                None,
                format!(
                    "swatch '{}' is referenced by styles [{styles}] and {}",
                    id.as_str(),
                    format_object_usage(&usage.objects)
                ),
            ));
        }

        let mut palette = self.palette.clone();
        let removed = palette
            .remove(id)
            .map_err(|error| ProjectError::new("delete palette swatch", None, error.to_string()))?
            .ok_or_else(|| {
                ProjectError::new(
                    "delete palette swatch",
                    None,
                    format!("swatch '{}' does not exist", id.as_str()),
                )
            })?;
        self.save_palette(palette)?;
        Ok(removed)
    }

    fn validate_graph(&self) -> Result<(), ProjectError> {
        validate_graph(&self.palette, &self.styles, &self.objects)
    }

    fn ensure_no_external_changes(&self, operation: &'static str) -> Result<(), ProjectError> {
        let changes = self.external_changes()?;
        if changes.is_empty() {
            return Ok(());
        }
        Err(external_conflict(operation, &changes))
    }

    fn ensure_catalog_sources_unchanged(
        &self,
        operation: &'static str,
    ) -> Result<(), ProjectError> {
        let current = scan_art_sources(&self.art_root)?;
        let changes = [PathBuf::from(PALETTE_FILE), PathBuf::from(STYLE_FILE)]
            .into_iter()
            .filter_map(|path| {
                source_change(&path, self.loaded_sources.get(&path), current.get(&path))
            })
            .collect::<Vec<_>>();
        if changes.is_empty() {
            return Ok(());
        }
        Err(external_conflict(operation, &changes))
    }

    fn loaded_source(&self, path: &Path) -> Result<&[u8], ProjectError> {
        let key = relative_art_path(&self.art_root, path)?;
        self.loaded_sources
            .get(&key)
            .map(Vec::as_slice)
            .ok_or_else(|| {
                ProjectError::at(
                    "read loaded source snapshot",
                    path,
                    "the file was not present when the project loaded",
                )
            })
    }

    fn record_written_source(&mut self, path: &Path, source: Vec<u8>) -> Result<(), ProjectError> {
        let key = relative_art_path(&self.art_root, path)?;
        drop(self.loaded_sources.insert(key, source));
        Ok(())
    }

    fn remove_written_source(&mut self, path: &Path) -> Result<(), ProjectError> {
        let key = relative_art_path(&self.art_root, path)?;
        drop(self.loaded_sources.remove(&key));
        Ok(())
    }
}

fn load_objects(art_root: &Path) -> Result<BTreeMap<ObjectAssetId, ObjectBlueprint>, ProjectError> {
    let mut objects = BTreeMap::new();
    for category in OBJECT_CATEGORIES {
        let directory = art_root
            .join(OBJECT_DIRECTORY)
            .join(category_directory(category));
        if !directory
            .try_exists()
            .map_err(|error| ProjectError::at("inspect object directory", &directory, error))?
        {
            continue;
        }

        let entries = fs::read_dir(&directory)
            .map_err(|error| ProjectError::at("scan object directory", &directory, error))?;
        let mut paths = Vec::new();
        for entry in entries {
            let entry = entry
                .map_err(|error| ProjectError::at("scan object directory", &directory, error))?;
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "ron") {
                continue;
            }
            let file_type = entry
                .file_type()
                .map_err(|error| ProjectError::at("inspect object asset", &path, error))?;
            if !file_type.is_file() {
                return Err(ProjectError::at(
                    "scan object directory",
                    &path,
                    "object RON entry is not a regular file",
                ));
            }
            paths.push(path);
        }
        paths.sort();

        for path in paths {
            let blueprint = read_ron::<ObjectBlueprint>(&path)?;
            validate_loaded_object_path(art_root, category, &path, &blueprint)?;
            if let Some(previous) = objects.insert(blueprint.id.clone(), blueprint) {
                return Err(ProjectError::at(
                    "load object",
                    &path,
                    format!("duplicate object id '{}'", previous.id.as_str()),
                ));
            }
        }
    }
    Ok(objects)
}

fn read_ron<T: DeserializeOwned>(path: &Path) -> Result<T, ProjectError> {
    let source =
        fs::read_to_string(path).map_err(|error| ProjectError::at("read asset", path, error))?;
    ron::from_str(&source).map_err(|error| ProjectError::at("parse RON asset", path, error))
}

fn validate_graph(
    palette: &ArtPalette,
    styles: &VoxelStyleCatalog,
    objects: &BTreeMap<ObjectAssetId, ObjectBlueprint>,
) -> Result<(), ProjectError> {
    palette
        .validate()
        .map_err(|error| ProjectError::new("validate art graph", None, error.to_string()))?;
    styles
        .validate(palette)
        .map_err(|error| ProjectError::new("validate art graph", None, error.to_string()))?;
    for (id, blueprint) in objects {
        if id != &blueprint.id {
            return Err(ProjectError::new(
                "validate art graph",
                None,
                format!(
                    "object map key '{}' does not match blueprint id '{}'",
                    id.as_str(),
                    blueprint.id.as_str()
                ),
            ));
        }
        validate_object_identity(blueprint)?;
        blueprint.validate(styles).map_err(|error| {
            ProjectError::new(
                "validate art graph",
                None,
                format!("object '{}': {error}", id.as_str()),
            )
        })?;
    }
    Ok(())
}

fn validate_object_identity(blueprint: &ObjectBlueprint) -> Result<&str, ProjectError> {
    let expected_category = category_directory(blueprint.category);
    let id = blueprint.id.as_str();
    let Some((category, filename)) = id.split_once('/') else {
        return Err(ProjectError::new(
            "validate object identity",
            None,
            format!("object id '{id}' must be '<category>/<filename>' for its tracked path"),
        ));
    };
    if category != expected_category || filename.is_empty() || filename.contains('/') {
        return Err(ProjectError::new(
            "validate object identity",
            None,
            format!(
                "object id '{id}' must be '{expected_category}/<filename>' with one filename segment"
            ),
        ));
    }
    Ok(filename)
}

fn validate_loaded_object_path(
    art_root: &Path,
    scanned_category: ObjectCategory,
    actual: &Path,
    blueprint: &ObjectBlueprint,
) -> Result<(), ProjectError> {
    if blueprint.category != scanned_category {
        return Err(ProjectError::at(
            "validate object path",
            actual,
            format!(
                "file is in '{}' but blueprint category is '{}'",
                category_directory(scanned_category),
                category_directory(blueprint.category)
            ),
        ));
    }
    let expected = object_path(art_root, blueprint)?;
    if actual != expected {
        return Err(ProjectError::at(
            "validate object path",
            actual,
            format!("object id requires path '{}'", expected.display()),
        ));
    }
    Ok(())
}

fn object_path(art_root: &Path, blueprint: &ObjectBlueprint) -> Result<PathBuf, ProjectError> {
    let filename = validate_object_identity(blueprint)?;
    Ok(art_root
        .join(OBJECT_DIRECTORY)
        .join(category_directory(blueprint.category))
        .join(format!("{filename}.ron")))
}

const fn category_directory(category: ObjectCategory) -> &'static str {
    match category {
        ObjectCategory::Plant => "plant",
        ObjectCategory::Effect => "effect",
        ObjectCategory::Prop => "prop",
    }
}

fn object_usage_for_styles<'a>(
    objects: &BTreeMap<ObjectAssetId, ObjectBlueprint>,
    styles: impl IntoIterator<Item = &'a VoxelStyleId>,
) -> Vec<ObjectUsage> {
    let styles: BTreeSet<&VoxelStyleId> = styles.into_iter().collect();
    objects
        .iter()
        .filter_map(|(id, object)| {
            let placements = object
                .placements
                .iter()
                .filter(|placement| styles.contains(&placement.style))
                .count();
            (placements > 0).then(|| ObjectUsage {
                object: id.clone(),
                placements,
            })
        })
        .collect()
}

fn format_object_usage(usage: &[ObjectUsage]) -> String {
    if usage.is_empty() {
        return "no saved objects".to_owned();
    }
    usage
        .iter()
        .map(|entry| format!("{} ({} voxels)", entry.object.as_str(), entry.placements))
        .collect::<Vec<_>>()
        .join(", ")
}

fn with_rollback(
    original: ProjectError,
    rollback: Result<Vec<u8>, ProjectError>,
    restored_asset: &str,
) -> ProjectError {
    match rollback {
        Ok(_) => ProjectError::new(
            original.operation(),
            original.path().map(Path::to_path_buf),
            format!(
                "{}; the previous {restored_asset} file was restored",
                original.detail()
            ),
        ),
        Err(rollback_error) => ProjectError::new(
            "restore art catalogs",
            rollback_error.path().map(Path::to_path_buf),
            format!(
                "{}; restoring the previous {restored_asset} file also failed: {}",
                original, rollback_error
            ),
        ),
    }
}

fn write_ron_atomically<T>(
    path: &Path,
    value: &T,
    overwrite: OverwriteBehavior,
) -> Result<Vec<u8>, ProjectError>
where
    T: Serialize + DeserializeOwned,
{
    let encoded = pretty_ron(value)?;
    ron::from_str::<T>(&encoded)
        .map_err(|error| ProjectError::at("verify serialized RON", path, error))?;
    write_bytes_atomically(path, encoded.as_bytes(), overwrite)
}

fn write_bytes_atomically(
    path: &Path,
    source: &[u8],
    overwrite: OverwriteBehavior,
) -> Result<Vec<u8>, ProjectError> {
    let parent = path.parent().ok_or_else(|| {
        ProjectError::at(
            "prepare atomic save",
            path,
            "destination has no parent directory",
        )
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| ProjectError::at("create asset directory", parent, error))?;
    AtomicFile::new(path, overwrite)
        .write(|file| file.write_all(source))
        .map_err(|error| ProjectError::at("atomically replace asset", path, error))?;
    Ok(source.to_vec())
}

fn pretty_ron<T: Serialize>(value: &T) -> Result<String, ProjectError> {
    let config = ron::ser::PrettyConfig::default()
        .new_line("\n")
        .indentor("    ");
    let mut encoded = ron::ser::to_string_pretty(value, config)
        .map_err(|error| ProjectError::new("serialize RON asset", None, error.to_string()))?;
    encoded.push('\n');
    Ok(encoded)
}

fn scan_art_sources(art_root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>, ProjectError> {
    let mut sources = BTreeMap::new();
    for filename in [PALETTE_FILE, STYLE_FILE] {
        let path = art_root.join(filename);
        if path
            .try_exists()
            .map_err(|error| ProjectError::at("inspect art source", &path, error))?
        {
            let bytes = fs::read(&path)
                .map_err(|error| ProjectError::at("read art source", &path, error))?;
            drop(sources.insert(PathBuf::from(filename), bytes));
        }
    }

    for category in OBJECT_CATEGORIES {
        let relative_directory = PathBuf::from(OBJECT_DIRECTORY).join(category_directory(category));
        let directory = art_root.join(&relative_directory);
        if !directory
            .try_exists()
            .map_err(|error| ProjectError::at("inspect object directory", &directory, error))?
        {
            continue;
        }
        let entries = fs::read_dir(&directory)
            .map_err(|error| ProjectError::at("scan object directory", &directory, error))?;
        for entry in entries {
            let entry = entry
                .map_err(|error| ProjectError::at("scan object directory", &directory, error))?;
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "ron") {
                continue;
            }
            let file_type = entry
                .file_type()
                .map_err(|error| ProjectError::at("inspect object asset", &path, error))?;
            if !file_type.is_file() {
                return Err(ProjectError::at(
                    "scan object directory",
                    &path,
                    "object RON entry is not a regular file",
                ));
            }
            let bytes = fs::read(&path)
                .map_err(|error| ProjectError::at("read art source", &path, error))?;
            let filename = path.file_name().ok_or_else(|| {
                ProjectError::at(
                    "scan object directory",
                    &path,
                    "object path has no filename",
                )
            })?;
            drop(sources.insert(relative_directory.join(filename), bytes));
        }
    }
    Ok(sources)
}

/// Reads the exact current byte revisions below a repository's tracked art tree.
///
/// This intentionally shares the same source discovery as project loading and
/// external-change detection so capture transactions cannot omit a source that
/// either of those paths considers authoritative.
pub(crate) fn current_project_revisions(
    repository_root: &Path,
) -> Result<ProjectRevisionSet, ProjectError> {
    let sources = scan_art_sources(&repository_root.join(ART_PATH))?;
    Ok(revision_set_from_sources(&sources))
}

/// Reads the exact byte revision of one repository-relative renderer source.
pub(crate) fn current_file_revision(
    repository_root: &Path,
    relative_path: &Path,
) -> Result<ByteRevision, ProjectError> {
    let path = repository_root.join(relative_path);
    let source =
        fs::read(&path).map_err(|error| ProjectError::at("read renderer source", &path, error))?;
    Ok(byte_revision(&source))
}

fn byte_revision(source: &[u8]) -> ByteRevision {
    ByteRevision {
        byte_len: u64::try_from(source.len()).unwrap_or(u64::MAX),
        fingerprint: xxh3_64(source),
    }
}

fn revision_set_from_sources(sources: &BTreeMap<PathBuf, Vec<u8>>) -> ProjectRevisionSet {
    ProjectRevisionSet {
        files: sources
            .iter()
            .map(|(path, source)| (normalized_relative_path(path), byte_revision(source)))
            .collect(),
    }
}

fn compare_sources(
    loaded: &BTreeMap<PathBuf, Vec<u8>>,
    current: &BTreeMap<PathBuf, Vec<u8>>,
) -> Vec<ExternalAssetChange> {
    loaded
        .keys()
        .chain(current.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|path| source_change(path, loaded.get(path), current.get(path)))
        .collect()
}

fn source_change(
    path: &Path,
    loaded: Option<&Vec<u8>>,
    current: Option<&Vec<u8>>,
) -> Option<ExternalAssetChange> {
    let kind = match (loaded, current) {
        (None, Some(_)) => ExternalChangeKind::Added,
        (Some(_), None) => ExternalChangeKind::Removed,
        (Some(loaded), Some(current)) if loaded != current => ExternalChangeKind::Modified,
        _ => return None,
    };
    Some(ExternalAssetChange {
        path: path.to_path_buf(),
        kind,
    })
}

fn external_conflict(operation: &'static str, changes: &[ExternalAssetChange]) -> ProjectError {
    let examples = changes
        .iter()
        .take(5)
        .map(|change| {
            let action = match change.kind {
                ExternalChangeKind::Added => "added",
                ExternalChangeKind::Removed => "removed",
                ExternalChangeKind::Modified => "modified",
            };
            format!("{} ({action})", change.path.display())
        })
        .collect::<Vec<_>>()
        .join(", ");
    let remainder = changes.len().saturating_sub(5);
    let suffix = if remainder > 0 {
        format!(", and {remainder} more")
    } else {
        String::new()
    };
    ProjectError::new(
        operation,
        None,
        format!(
            "tracked art files changed outside this editor: {examples}{suffix}; reload the project or use Save As for the object draft"
        ),
    )
}

fn relative_art_path(art_root: &Path, path: &Path) -> Result<PathBuf, ProjectError> {
    path.strip_prefix(art_root)
        .map(Path::to_path_buf)
        .map_err(|error| ProjectError::at("resolve art source path", path, error))
}

fn normalized_relative_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use hex_assets::{
        ConnectivityPolicy, EffectPart, LocalAxialCoord, LocalVoxelCoord, ObjectBounds, ObjectPart,
        ObjectPlacement, PlantPart, PropPart, VoxelStyle, VoxelSurfaceMode,
        OBJECT_BLUEPRINT_SCHEMA_VERSION,
    };

    use super::*;
    use crate::model::EditorModel;

    static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    pub(crate) struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        pub(crate) fn new() -> Self {
            let sequence = TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "hex-editor-project-test-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("test directory should be created");
            Self { path }
        }

        pub(crate) fn repository_root(&self) -> &Path {
            &self.path
        }

        pub(crate) fn art_root(&self) -> PathBuf {
            self.path.join(ART_PATH)
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

    pub(crate) fn fixture_catalog() -> VoxelStyleCatalog {
        let mut styles = BTreeMap::new();
        styles.insert(
            style_id("plant/trunk"),
            VoxelStyle::new(
                "Plant Trunk",
                swatch_id("plant/trunk"),
                VoxelSurfaceMode::Opaque,
                1.0,
                None,
            )
            .expect("fixture style should be valid"),
        );
        VoxelStyleCatalog::new(styles).expect("fixture catalog should be valid")
    }

    fn tree(id: &str, display_name: &str) -> ObjectBlueprint {
        ObjectBlueprint {
            schema_version: OBJECT_BLUEPRINT_SCHEMA_VERSION,
            id: object_id(id),
            display_name: display_name.to_owned(),
            category: ObjectCategory::Plant,
            bounds: ObjectBounds {
                radius: 1,
                min_level: 0,
                height: 4,
            },
            connectivity: ConnectivityPolicy::Grounded,
            origin: LocalVoxelCoord::new(0, 0, 0),
            placements: vec![
                ObjectPlacement {
                    position: LocalVoxelCoord::new(0, 0, 0),
                    style: style_id("plant/trunk"),
                    part: ObjectPart::Plant(PlantPart::Root),
                },
                ObjectPlacement {
                    position: LocalVoxelCoord::new(0, 0, 1),
                    style: style_id("plant/trunk"),
                    part: ObjectPart::Plant(PlantPart::Trunk),
                },
            ],
            blocker_footprint: vec![LocalAxialCoord::new(0, 0)],
            canopy_occluders: Vec::new(),
        }
    }

    fn prop(id: &str, display_name: &str) -> ObjectBlueprint {
        ObjectBlueprint {
            schema_version: OBJECT_BLUEPRINT_SCHEMA_VERSION,
            id: object_id(id),
            display_name: display_name.to_owned(),
            category: ObjectCategory::Prop,
            bounds: ObjectBounds {
                radius: 1,
                min_level: 0,
                height: 2,
            },
            connectivity: ConnectivityPolicy::Grounded,
            origin: LocalVoxelCoord::new(0, 0, 0),
            placements: vec![ObjectPlacement {
                position: LocalVoxelCoord::new(0, 0, 0),
                style: style_id("plant/trunk"),
                part: ObjectPart::Prop(PropPart::Structure),
            }],
            blocker_footprint: vec![LocalAxialCoord::new(0, 0)],
            canopy_occluders: Vec::new(),
        }
    }

    fn floating_effect(id: &str, display_name: &str) -> ObjectBlueprint {
        let origin = LocalVoxelCoord::new(0, 0, -2);
        ObjectBlueprint {
            schema_version: OBJECT_BLUEPRINT_SCHEMA_VERSION,
            id: object_id(id),
            display_name: display_name.to_owned(),
            category: ObjectCategory::Effect,
            bounds: ObjectBounds {
                radius: 2,
                min_level: -4,
                height: 8,
            },
            connectivity: ConnectivityPolicy::Free,
            origin,
            placements: vec![
                ObjectPlacement {
                    position: origin,
                    style: style_id("plant/trunk"),
                    part: ObjectPart::Effect(EffectPart::Core),
                },
                ObjectPlacement {
                    position: LocalVoxelCoord::new(1, 0, 1),
                    style: style_id("plant/trunk"),
                    part: ObjectPart::Effect(EffectPart::Trail),
                },
            ],
            blocker_footprint: Vec::new(),
            canopy_occluders: Vec::new(),
        }
    }

    pub(crate) fn prepare_project() -> TestDirectory {
        let directory = TestDirectory::new();
        let art_root = directory.art_root();
        fs::create_dir_all(&art_root).expect("fixture art directory should be created");
        fs::write(
            art_root.join(PALETTE_FILE),
            include_str!("../../../assets/art/palette.ron"),
        )
        .expect("fixture palette should be written");
        fs::write(
            art_root.join(STYLE_FILE),
            pretty_ron(&fixture_catalog()).expect("fixture catalog should serialize"),
        )
        .expect("fixture style catalog should be written");
        directory
    }

    fn write_object_fixture(directory: &TestDirectory, blueprint: &ObjectBlueprint) -> PathBuf {
        let path =
            object_path(&directory.art_root(), blueprint).expect("fixture id should map to a path");
        let parent = path.parent().expect("fixture object path has a parent");
        fs::create_dir_all(parent).expect("fixture object directory should be created");
        fs::write(
            &path,
            pretty_ron(blueprint).expect("fixture object should serialize"),
        )
        .expect("fixture object should be written");
        path
    }

    #[test]
    fn loads_missing_object_directories_and_sorts_discovered_objects() {
        let directory = prepare_project();
        let empty = AssetProject::load(&directory.path).expect("empty project should load");
        assert!(empty.objects().is_empty());

        write_object_fixture(&directory, &tree("plant/zinnia", "Zinnia"));
        write_object_fixture(&directory, &tree("plant/ash", "Ash"));
        let loaded = AssetProject::load(&directory.path).expect("project should load");
        assert_eq!(
            loaded
                .objects()
                .keys()
                .map(ObjectAssetId::as_str)
                .collect::<Vec<_>>(),
            ["plant/ash", "plant/zinnia"]
        );
    }

    #[test]
    fn load_rejects_category_path_and_style_reference_mismatches() {
        let directory = prepare_project();
        let mut wrong_path = tree("plant/oak", "Oak");
        let path = write_object_fixture(&directory, &wrong_path);
        let renamed = path.with_file_name("elm.ron");
        fs::rename(&path, &renamed).expect("fixture should move");
        let error = AssetProject::load(&directory.path).expect_err("wrong path must fail");
        assert_eq!(error.operation(), "validate object path");

        fs::rename(&renamed, &path).expect("fixture should move back");
        wrong_path = prop("prop/oak", "Oak Prop");
        fs::write(
            &path,
            pretty_ron(&wrong_path).expect("mismatched fixture should serialize"),
        )
        .expect("mismatched fixture should be written");
        let error = AssetProject::load(&directory.path).expect_err("wrong category must fail");
        assert_eq!(error.operation(), "validate object path");

        let missing_style = tree("plant/oak", "Oak");
        fs::write(
            &path,
            pretty_ron(&missing_style).expect("fixture should serialize"),
        )
        .expect("fixture should be restored");
        fs::write(
            directory.art_root().join(STYLE_FILE),
            pretty_ron(
                &VoxelStyleCatalog::new(BTreeMap::new())
                    .expect("an empty style catalog is intrinsically valid"),
            )
            .expect("empty catalog should serialize"),
        )
        .expect("empty catalog should be written");
        let error = AssetProject::load(&directory.path).expect_err("missing style must fail");
        assert_eq!(error.operation(), "validate art graph");
    }

    #[test]
    fn save_save_as_and_duplicate_are_explicit_and_reloadable() {
        let directory = prepare_project();
        let mut project = AssetProject::load(&directory.path).expect("project should load");
        project
            .save_object_as(tree("plant/draft", "Oak"), object_id("plant/oak"))
            .expect("Save As should create an object");

        let oak_path = directory
            .art_root()
            .join("objects")
            .join("plant")
            .join("oak.ron");
        let saved = fs::read_to_string(&oak_path).expect("saved object should be readable");
        assert!(saved.ends_with('\n'));
        assert!(!saved.contains(".tmp"));

        let mut oak = project
            .object(&object_id("plant/oak"))
            .cloned()
            .expect("saved oak should be indexed");
        oak.display_name = "Old Oak".to_owned();
        project
            .save_object(&object_id("plant/oak"), oak)
            .expect("Save should replace oak");
        project
            .duplicate_object(
                &object_id("plant/oak"),
                object_id("plant/young-oak"),
                "Young Oak",
            )
            .expect("Duplicate should create a new object");

        let reloaded = AssetProject::load(&directory.path).expect("saved project should reload");
        assert_eq!(reloaded.objects().len(), 2);
        assert_eq!(
            reloaded
                .object(&object_id("plant/oak"))
                .map(|object| object.display_name.as_str()),
            Some("Old Oak")
        );
        assert_eq!(
            reloaded
                .object(&object_id("plant/young-oak"))
                .map(|object| object.display_name.as_str()),
            Some("Young Oak")
        );

        let before = fs::read(&oak_path).expect("oak should be readable");
        let error = project
            .save_object_as(tree("plant/other", "Other"), object_id("plant/oak"))
            .expect_err("Save As cannot replace an existing id");
        assert_eq!(error.operation(), "save object as");
        assert_eq!(
            fs::read(&oak_path).expect("oak should remain readable"),
            before
        );
    }

    #[test]
    fn invalid_saves_preserve_previous_files_and_memory() {
        let directory = prepare_project();
        write_object_fixture(&directory, &tree("plant/oak", "Oak"));
        let mut project = AssetProject::load(&directory.path).expect("project should load");

        let palette_path = directory.art_root().join(PALETTE_FILE);
        let palette_before = fs::read(&palette_path).expect("palette should be readable");
        let mut invalid_palette = project.palette().clone();
        let removed = invalid_palette
            .remove(&swatch_id("plant/trunk"))
            .expect("palette removal operation should be valid");
        assert!(removed.is_some());
        assert!(project.save_palette(invalid_palette).is_err());
        assert_eq!(
            fs::read(&palette_path).expect("palette should remain readable"),
            palette_before
        );
        assert!(project.palette().contains(&swatch_id("plant/trunk")));

        let oak_path = directory
            .art_root()
            .join("objects")
            .join("plant")
            .join("oak.ron");
        let oak_before = fs::read(&oak_path).expect("oak should be readable");
        let mut invalid_oak = project
            .object(&object_id("plant/oak"))
            .cloned()
            .expect("oak should be indexed");
        invalid_oak.placements.clear();
        assert!(project
            .save_object(&object_id("plant/oak"), invalid_oak)
            .is_err());
        assert_eq!(
            fs::read(&oak_path).expect("oak should remain readable"),
            oak_before
        );
        assert_eq!(
            project
                .object(&object_id("plant/oak"))
                .map(|object| object.placements.len()),
            Some(2)
        );
    }

    #[test]
    fn signed_effect_pivot_rotates_six_times_and_survives_save_reload() {
        let directory = prepare_project();
        let mut project = AssetProject::load(&directory.path).expect("project should load");
        let styles = project.styles().clone();
        let mut editor =
            EditorModel::from_blueprint(floating_effect("effect/draft", "Signed Burst"))
                .expect("floating effect should open");
        let origin = editor.object().origin;
        let trail = LocalVoxelCoord::new(1, 0, 1);
        assert!(editor.select(trail, false));
        for _ in 0..6 {
            assert_eq!(editor.rotate_selection_clockwise(origin), Ok(true));
        }
        assert!(editor.selection().contains(trail));
        let blueprint = editor
            .blueprint_for_save(&styles)
            .expect("rotated effect should validate");
        project
            .save_object_as(blueprint, object_id("effect/signed-burst"))
            .expect("effect should save");

        let reloaded = AssetProject::load(&directory.path).expect("project should reload");
        let saved = reloaded
            .object(&object_id("effect/signed-burst"))
            .expect("saved effect should exist");
        assert_eq!(saved.origin, LocalVoxelCoord::new(0, 0, -2));
        assert!(saved
            .placements
            .iter()
            .any(|placement| placement.position == trail));
        assert_eq!(saved.connectivity, ConnectivityPolicy::Free);
    }

    #[test]
    fn ordinary_save_cannot_overwrite_another_objects_identity() {
        let directory = prepare_project();
        write_object_fixture(&directory, &tree("plant/oak", "Oak"));
        write_object_fixture(&directory, &tree("plant/ash", "Ash"));
        let mut project = AssetProject::load(&directory.path).expect("project should load");
        let ash_path = directory
            .art_root()
            .join("objects")
            .join("plant")
            .join("ash.ron");
        let ash_before = fs::read(&ash_path).expect("ash should be readable");
        let mut disguised = project
            .object(&object_id("plant/ash"))
            .cloned()
            .expect("ash should be indexed");
        disguised.display_name = "Not Oak".to_owned();

        let error = project
            .save_object(&object_id("plant/oak"), disguised)
            .expect_err("ordinary Save cannot change the open identity");
        assert!(error.detail().contains("use Save As"));
        assert_eq!(
            fs::read(&ash_path).expect("ash should remain readable"),
            ash_before
        );
    }

    #[test]
    fn catalog_save_orders_additions_and_removals_without_invalid_intermediates() {
        let directory = prepare_project();
        let mut project = AssetProject::load(&directory.path).expect("project should load");
        let accent_id = swatch_id("plant/accent");
        let accent = PaletteSwatch::new(
            "Plant Accent",
            hex_assets::SrgbColor::new(0.88, 0.12, 0.25).expect("fixture colour should be valid"),
            BTreeSet::from(["plant".to_owned()]),
        )
        .expect("fixture swatch should be valid");
        let mut palette = project.palette().clone();
        drop(
            palette
                .insert(accent_id.clone(), accent)
                .expect("fixture palette edit should be valid"),
        );
        let mut styles = project.styles().clone();
        drop(
            styles
                .insert(
                    style_id("plant/accent"),
                    VoxelStyle::new(
                        "Plant Accent",
                        accent_id,
                        VoxelSurfaceMode::Opaque,
                        1.0,
                        None,
                    )
                    .expect("fixture style should be valid"),
                )
                .expect("fixture catalog edit should be valid"),
        );
        project
            .save_catalogs(palette, styles)
            .expect("additions should save palette first");

        let mut palette = project.palette().clone();
        assert!(palette
            .remove(&swatch_id("plant/trunk"))
            .expect("fixture removal should be valid")
            .is_some());
        let mut styles = project.styles().clone();
        assert!(styles.remove(&style_id("plant/trunk")).is_some());
        project
            .save_catalogs(palette, styles)
            .expect("removals should save styles first");
        let reloaded = AssetProject::load(&directory.path).expect("catalogs should reload");
        assert!(reloaded.palette().contains(&swatch_id("plant/accent")));
        assert!(reloaded.styles().contains(&style_id("plant/accent")));
        assert!(!reloaded.palette().contains(&swatch_id("plant/trunk")));
    }

    #[test]
    fn usage_reports_are_sorted_transitive_and_count_placements() {
        let directory = prepare_project();
        write_object_fixture(&directory, &tree("plant/oak", "Oak"));
        write_object_fixture(&directory, &tree("plant/ash", "Ash"));
        let project = AssetProject::load(&directory.path).expect("project should load");

        let style_usage = project.style_usage(&style_id("plant/trunk"));
        assert_eq!(
            style_usage,
            vec![
                ObjectUsage {
                    object: object_id("plant/ash"),
                    placements: 2,
                },
                ObjectUsage {
                    object: object_id("plant/oak"),
                    placements: 2,
                },
            ]
        );
        let swatch_usage = project.swatch_usage(&swatch_id("plant/trunk"));
        assert_eq!(swatch_usage.styles, vec![style_id("plant/trunk")]);
        assert_eq!(swatch_usage.objects, style_usage);
    }

    #[test]
    fn delete_guards_preserve_referenced_catalog_entries() {
        let directory = prepare_project();
        write_object_fixture(&directory, &tree("plant/oak", "Oak"));
        let mut project = AssetProject::load(&directory.path).expect("project should load");
        let style_path = directory.art_root().join(STYLE_FILE);
        let palette_path = directory.art_root().join(PALETTE_FILE);
        let styles_before = fs::read(&style_path).expect("styles should be readable");
        let palette_before = fs::read(&palette_path).expect("palette should be readable");

        let style_error = project
            .delete_style(&style_id("plant/trunk"))
            .expect_err("referenced style deletion must fail");
        assert!(style_error.detail().contains("plant/oak"));
        assert_eq!(
            fs::read(&style_path).expect("styles should remain readable"),
            styles_before
        );

        let swatch_error = project
            .delete_swatch(&swatch_id("plant/trunk"))
            .expect_err("referenced swatch deletion must fail");
        assert!(swatch_error.detail().contains("plant/trunk"));
        assert_eq!(
            fs::read(&palette_path).expect("palette should remain readable"),
            palette_before
        );

        project
            .delete_object(&object_id("plant/oak"))
            .expect("object deletion should succeed");
        project
            .delete_style(&style_id("plant/trunk"))
            .expect("unreferenced style deletion should succeed");
        project
            .delete_swatch(&swatch_id("plant/trunk"))
            .expect("unreferenced swatch deletion should succeed");
        assert!(!project.styles().contains(&style_id("plant/trunk")));
        assert!(!project.palette().contains(&swatch_id("plant/trunk")));
    }

    #[test]
    fn canonical_saves_leave_no_sibling_temp_files() {
        let directory = prepare_project();
        let mut project = AssetProject::load(&directory.path).expect("project should load");
        project
            .save_object_as(tree("plant/draft", "Oak"), object_id("plant/oak"))
            .expect("Save As should succeed");

        let object_directory = directory.art_root().join(OBJECT_DIRECTORY).join("plant");
        let names = fs::read_dir(object_directory)
            .expect("object directory should be readable")
            .map(|entry| {
                entry
                    .expect("directory entry should be readable")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(names, ["oak.ron"]);
    }

    #[test]
    fn formatting_only_external_changes_block_overwrite_until_reload() {
        let directory = prepare_project();
        let mut project = AssetProject::load(&directory.path).expect("project should load");
        let palette_path = directory.art_root().join(PALETTE_FILE);
        let mut external = fs::read(&palette_path).expect("palette should be readable");
        external.extend_from_slice(b"\n// external formatting edit\n");
        fs::write(&palette_path, &external).expect("external edit should be written");

        assert_eq!(
            project
                .external_changes()
                .expect("external changes should scan"),
            vec![ExternalAssetChange {
                path: PathBuf::from(PALETTE_FILE),
                kind: ExternalChangeKind::Modified,
            }]
        );
        let error = project
            .save_palette(project.palette().clone())
            .expect_err("an external byte change must block overwrite");
        assert!(error.detail().contains("palette.ron (modified)"));
        assert_eq!(
            fs::read(&palette_path).expect("external palette should remain readable"),
            external
        );

        project
            .reload_from_disk()
            .expect("a valid external formatting edit should reload");
        assert!(project
            .external_changes()
            .expect("reloaded project should scan")
            .is_empty());
    }

    #[test]
    fn current_project_revisions_track_exact_on_disk_bytes() {
        let directory = prepare_project();
        let project = AssetProject::load(&directory.path).expect("project should load");
        let loaded = project.revision_snapshot();
        assert_eq!(
            current_project_revisions(&directory.path)
                .expect("unchanged tracked sources should scan"),
            loaded
        );

        let palette_path = directory.art_root().join(PALETTE_FILE);
        let mut modified = fs::read(&palette_path).expect("palette should be readable");
        let byte = modified
            .first_mut()
            .expect("fixture palette should contain source bytes");
        *byte = byte.wrapping_add(1);
        fs::write(&palette_path, modified).expect("equal-length edit should be written");

        let current = current_project_revisions(&directory.path)
            .expect("modified tracked sources should scan");
        let loaded_palette = loaded
            .files
            .get(PALETTE_FILE)
            .expect("loaded revisions should include the palette");
        let current_palette = current
            .files
            .get(PALETTE_FILE)
            .expect("current revisions should include the palette");
        assert_eq!(current_palette.byte_len, loaded_palette.byte_len);
        assert_ne!(current_palette.fingerprint, loaded_palette.fingerprint);
    }

    #[test]
    fn same_length_external_object_edit_blocks_save_but_save_as_preserves_both() {
        let directory = prepare_project();
        let oak_path = write_object_fixture(&directory, &tree("plant/oak", "Oak"));
        let mut project = AssetProject::load(&directory.path).expect("project should load");
        let mut local = project
            .object(&object_id("plant/oak"))
            .cloned()
            .expect("oak should be indexed");
        local.display_name = "Local".to_owned();

        let external = pretty_ron(&tree("plant/oak", "Elm"))
            .expect("external fixture should serialize")
            .into_bytes();
        let loaded_len = fs::metadata(&oak_path)
            .expect("oak metadata should load")
            .len();
        assert_eq!(
            u64::try_from(external.len()).expect("fixture length should fit"),
            loaded_len
        );
        fs::write(&oak_path, &external).expect("external object edit should be written");

        let error = project
            .save_object(&object_id("plant/oak"), local.clone())
            .expect_err("ordinary save must preserve an external object edit");
        assert!(error.detail().contains("objects/plant/oak.ron (modified)"));
        project
            .save_object_as(local, object_id("plant/local-oak"))
            .expect("Save As should preserve a conflicted draft under a new key");

        assert_eq!(
            fs::read(&oak_path).expect("external oak should remain readable"),
            external
        );
        assert!(directory
            .art_root()
            .join("objects/plant/local-oak.ron")
            .is_file());
        assert_eq!(
            project
                .object(&object_id("plant/oak"))
                .expect("refreshed external object should be indexed")
                .display_name,
            "Elm"
        );
        assert!(project.object(&object_id("plant/local-oak")).is_some());
        assert!(project
            .external_changes()
            .expect("successful Save As should refresh object sources")
            .is_empty());
    }

    #[test]
    fn added_and_removed_sources_are_reported_and_block_graph_writes() {
        let directory = prepare_project();
        let mut project = AssetProject::load(&directory.path).expect("project should load");
        let ash_path = write_object_fixture(&directory, &tree("plant/ash", "Ash"));
        let changes = project
            .external_changes()
            .expect("added file should be detected");
        assert_eq!(
            changes,
            vec![ExternalAssetChange {
                path: PathBuf::from("objects/plant/ash.ron"),
                kind: ExternalChangeKind::Added,
            }]
        );
        let error = project
            .save_catalogs(project.palette().clone(), project.styles().clone())
            .expect_err("catalog writes must account for every saved object");
        assert!(error.detail().contains("objects/plant/ash.ron (added)"));

        project
            .reload_from_disk()
            .expect("valid added object should reload");
        fs::remove_file(&ash_path).expect("external object deletion should succeed");
        assert_eq!(
            project
                .external_changes()
                .expect("removed file should be detected"),
            vec![ExternalAssetChange {
                path: PathBuf::from("objects/plant/ash.ron"),
                kind: ExternalChangeKind::Removed,
            }]
        );
    }
}
