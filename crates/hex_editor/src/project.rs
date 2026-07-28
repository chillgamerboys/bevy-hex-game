//! Filesystem project loading, validation, impact reporting, and explicit saves.
//!
//! The editor model owns mutable drafts. [`AssetProject`] represents the last state
//! successfully loaded from or written to the tracked `assets/art` tree, so a failed
//! validation or filesystem operation never leaves its in-memory view ahead of disk.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use hex_assets::{
    ArtPalette, ObjectAssetId, ObjectBlueprint, ObjectCategory, PaletteSwatch, SwatchId,
    VoxelStyle, VoxelStyleCatalog, VoxelStyleId,
};
use serde::de::DeserializeOwned;
use serde::Serialize;

const ART_PATH: &str = "assets/art";
const PALETTE_FILE: &str = "palette.ron";
const STYLE_FILE: &str = "voxel_styles.ron";
const OBJECT_DIRECTORY: &str = "objects";
const TEMP_WRITE_ATTEMPTS: usize = 32;

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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

        let project = Self {
            repository_root,
            art_root,
            palette,
            styles,
            objects,
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

    /// Validates and atomically replaces the shared palette file.
    ///
    /// Every existing style and object is checked against the proposed palette before
    /// the previous file is touched.
    pub fn save_palette(&mut self, palette: ArtPalette) -> Result<(), ProjectError> {
        validate_graph(&palette, &self.styles, &self.objects)?;
        let path = self.art_root.join(PALETTE_FILE);
        write_ron_atomically(&path, &palette)?;
        self.palette = palette;
        Ok(())
    }

    /// Validates and atomically replaces the shared voxel-style file.
    ///
    /// Every existing object is checked against the proposed catalog before the
    /// previous file is touched.
    pub fn save_styles(&mut self, styles: VoxelStyleCatalog) -> Result<(), ProjectError> {
        validate_graph(&self.palette, &styles, &self.objects)?;
        let path = self.art_root.join(STYLE_FILE);
        write_ron_atomically(&path, &styles)?;
        self.styles = styles;
        Ok(())
    }

    /// Saves changes to an existing object without changing its stable id.
    ///
    /// Use [`Self::save_object_as`] for an unsaved draft or an identity change.
    pub fn save_object(&mut self, blueprint: ObjectBlueprint) -> Result<(), ProjectError> {
        if !self.objects.contains_key(&blueprint.id) {
            return Err(ProjectError::new(
                "save object",
                None,
                format!(
                    "object '{}' is not saved; use Save As for a new id",
                    blueprint.id.as_str()
                ),
            ));
        }
        let path = object_path(&self.art_root, &blueprint)?;
        let mut objects = self.objects.clone();
        drop(objects.insert(blueprint.id.clone(), blueprint.clone()));
        validate_graph(&self.palette, &self.styles, &objects)?;
        write_ron_atomically(&path, &blueprint)?;
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
        if self.objects.contains_key(&new_id) {
            return Err(ProjectError::new(
                "save object as",
                None,
                format!("object '{}' already exists", new_id.as_str()),
            ));
        }
        blueprint.id = new_id.clone();
        let path = object_path(&self.art_root, &blueprint)?;
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

        let mut objects = self.objects.clone();
        drop(objects.insert(new_id, blueprint.clone()));
        validate_graph(&self.palette, &self.styles, &objects)?;
        write_ron_atomically(&path, &blueprint)?;
        self.objects = objects;
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
        let blueprint = self.objects.get(id).cloned().ok_or_else(|| {
            ProjectError::new(
                "delete object",
                None,
                format!("object '{}' does not exist", id.as_str()),
            )
        })?;
        let path = object_path(&self.art_root, &blueprint)?;
        fs::remove_file(&path).map_err(|error| ProjectError::at("delete object", &path, error))?;
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
}

fn load_objects(art_root: &Path) -> Result<BTreeMap<ObjectAssetId, ObjectBlueprint>, ProjectError> {
    let mut objects = BTreeMap::new();
    for category in [
        ObjectCategory::Plant,
        ObjectCategory::Effect,
        ObjectCategory::Prop,
    ] {
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

fn write_ron_atomically<T>(path: &Path, value: &T) -> Result<(), ProjectError>
where
    T: Serialize + DeserializeOwned,
{
    let encoded = pretty_ron(value)?;
    ron::from_str::<T>(&encoded)
        .map_err(|error| ProjectError::at("verify serialized RON", path, error))?;

    let parent = path.parent().ok_or_else(|| {
        ProjectError::at(
            "prepare atomic save",
            path,
            "destination has no parent directory",
        )
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| ProjectError::at("create asset directory", parent, error))?;
    let (temporary_path, mut temporary_file) = create_sibling_temp(path)?;
    let mut guard = TemporaryFileGuard::new(temporary_path.clone());

    let write_result = (|| -> io::Result<()> {
        temporary_file.write_all(encoded.as_bytes())?;
        temporary_file.sync_all()
    })();
    drop(temporary_file);
    write_result
        .map_err(|error| ProjectError::at("write temporary asset", &temporary_path, error))?;

    if let Ok(metadata) = fs::metadata(path) {
        fs::set_permissions(&temporary_path, metadata.permissions()).map_err(|error| {
            ProjectError::at("preserve asset permissions", &temporary_path, error)
        })?;
    }

    install_temporary(&temporary_path, path)?;
    guard.disarm();
    Ok(())
}

#[cfg(windows)]
fn install_temporary(temporary: &Path, destination: &Path) -> Result<(), ProjectError> {
    install_with_backup(temporary, destination)
}

#[cfg(not(windows))]
fn install_temporary(temporary: &Path, destination: &Path) -> Result<(), ProjectError> {
    fs::rename(temporary, destination)
        .map_err(|error| ProjectError::at("atomically replace asset", destination, error))
}

/// Windows cannot replace an existing file with `std::fs::rename`. Move the last
/// valid file aside, install the fully written temporary file, and restore the old
/// file if installation fails.
#[cfg(any(windows, test))]
fn install_with_backup(temporary: &Path, destination: &Path) -> Result<(), ProjectError> {
    if !destination
        .try_exists()
        .map_err(|error| ProjectError::at("inspect atomic destination", destination, error))?
    {
        return fs::rename(temporary, destination)
            .map_err(|error| ProjectError::at("install new asset", destination, error));
    }

    let backup = move_destination_to_backup(destination)?;
    match fs::rename(temporary, destination) {
        Ok(()) => {
            // The new file is installed. A stale hidden backup is preferable to
            // reporting failure after disk and memory have already diverged.
            drop(fs::remove_file(backup));
            Ok(())
        }
        Err(install_error) => match fs::rename(&backup, destination) {
            Ok(()) => Err(ProjectError::at(
                "install asset",
                destination,
                format!(
                    "installation failed and the previous file was restored: {install_error}"
                ),
            )),
            Err(restore_error) => Err(ProjectError::at(
                "restore asset",
                destination,
                format!(
                    "installation failed ({install_error}); previous file remains at '{}' because restoration also failed ({restore_error})",
                    backup.display()
                ),
            )),
        },
    }
}

#[cfg(any(windows, test))]
fn move_destination_to_backup(destination: &Path) -> Result<PathBuf, ProjectError> {
    let parent = destination.parent().ok_or_else(|| {
        ProjectError::at(
            "prepare asset backup",
            destination,
            "destination has no parent directory",
        )
    })?;
    let filename = destination.file_name().ok_or_else(|| {
        ProjectError::at(
            "prepare asset backup",
            destination,
            "destination has no filename",
        )
    })?;
    let filename = filename.to_string_lossy();

    for _ in 0..TEMP_WRITE_ATTEMPTS {
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let backup = parent.join(format!(
            ".{filename}.{}.{}.backup",
            std::process::id(),
            sequence
        ));
        if backup
            .try_exists()
            .map_err(|error| ProjectError::at("inspect asset backup", &backup, error))?
        {
            continue;
        }
        match fs::rename(destination, &backup) {
            Ok(()) => return Ok(backup),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(ProjectError::at(
                    "preserve previous asset",
                    destination,
                    error,
                ));
            }
        }
    }

    Err(ProjectError::at(
        "preserve previous asset",
        destination,
        format!("could not reserve a sibling backup after {TEMP_WRITE_ATTEMPTS} attempts"),
    ))
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

fn create_sibling_temp(destination: &Path) -> Result<(PathBuf, File), ProjectError> {
    let parent = destination.parent().ok_or_else(|| {
        ProjectError::at(
            "prepare atomic save",
            destination,
            "destination has no parent directory",
        )
    })?;
    let filename = destination.file_name().ok_or_else(|| {
        ProjectError::at(
            "prepare atomic save",
            destination,
            "destination has no filename",
        )
    })?;
    let filename = filename.to_string_lossy();

    for _ in 0..TEMP_WRITE_ATTEMPTS {
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary_path = parent.join(format!(
            ".{filename}.{}.{}.tmp",
            std::process::id(),
            sequence
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
        {
            Ok(file) => return Ok((temporary_path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(ProjectError::at(
                    "create temporary asset",
                    &temporary_path,
                    error,
                ));
            }
        }
    }

    Err(ProjectError::at(
        "create temporary asset",
        destination,
        format!("could not reserve a unique sibling after {TEMP_WRITE_ATTEMPTS} attempts"),
    ))
}

struct TemporaryFileGuard {
    path: Option<PathBuf>,
}

impl TemporaryFileGuard {
    const fn new(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    fn disarm(&mut self) {
        self.path = None;
    }
}

impl Drop for TemporaryFileGuard {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            drop(fs::remove_file(path));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use hex_assets::{
        ConnectivityPolicy, LocalAxialCoord, LocalVoxelCoord, ObjectBounds, ObjectPart,
        ObjectPlacement, PlantPart, PropPart, VoxelStyle, VoxelSurfaceMode,
        OBJECT_BLUEPRINT_SCHEMA_VERSION,
    };

    use super::*;

    static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new() -> Self {
            let sequence = TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "hex-editor-project-test-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("test directory should be created");
            Self { path }
        }

        fn art_root(&self) -> PathBuf {
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

    fn fixture_catalog() -> VoxelStyleCatalog {
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

    fn prepare_project() -> TestDirectory {
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
        project.save_object(oak).expect("Save should replace oak");
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
        assert!(project.save_object(invalid_oak).is_err());
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
    fn backup_installer_replaces_and_restores_portably() {
        let directory = TestDirectory::new();
        let destination = directory.path.join("asset.ron");
        let temporary = directory.path.join("asset.tmp");
        fs::write(&destination, "old").expect("old asset should be written");
        fs::write(&temporary, "new").expect("temporary asset should be written");

        install_with_backup(&temporary, &destination).expect("replacement should succeed");
        assert_eq!(
            fs::read_to_string(&destination).expect("replacement should be readable"),
            "new"
        );
        assert!(!temporary.exists());

        fs::write(&destination, "last-valid").expect("last-valid asset should be written");
        let missing_temporary = directory.path.join("missing.tmp");
        assert!(install_with_backup(&missing_temporary, &destination).is_err());
        assert_eq!(
            fs::read_to_string(&destination).expect("old asset should be restored"),
            "last-valid"
        );
    }
}
