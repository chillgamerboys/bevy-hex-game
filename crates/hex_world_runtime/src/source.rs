//! Independently addressable, bounded package IO.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};

use atomicwrites::{AllowOverwrite, AtomicFile};
use hex_world_contracts::{
    ChunkDescriptor, ChunkId, ChunkPackage, ManifestIndex, WorldManifest, WorldPackage,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::{ErrorKind, RuntimeError, RuntimeResult};

static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Per-file limits, independent of the number of columns in the complete world.
#[derive(Debug, Clone, Copy)]
pub struct IoLimits {
    /// Maximum bytes in a manifest or save index.
    pub max_manifest_bytes: usize,
    /// Maximum bytes in one independently loaded chunk or saved partition.
    pub max_chunk_bytes: usize,
    /// Maximum bytes in one persisted transaction record.
    pub max_transaction_bytes: usize,
}

impl Default for IoLimits {
    fn default() -> Self {
        Self {
            max_manifest_bytes: 64 * 1024 * 1024,
            max_chunk_bytes: 8 * 1024 * 1024,
            max_transaction_bytes: 8 * 1024 * 1024,
        }
    }
}

/// Cooperative cancellation shared with exactly one load job.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    /// Requests cancellation. Cancellation never grants a stale job publication rights.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// Whether cancellation was requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    /// Returns a typed cancellation error when canceled.
    pub fn check(&self) -> RuntimeResult<()> {
        if self.is_cancelled() {
            Err(RuntimeError::new(ErrorKind::Cancelled, "load cancelled"))
        } else {
            Ok(())
        }
    }
}

/// Immutable source read by bounded background workers.
///
/// Implementations must avoid unbounded reads and must return only the requested
/// chunk. The runtime independently validates identity and integrity before admission.
pub trait ChunkSource: Send + Sync + 'static {
    /// Complete lightweight world catalogue, without loading terrain chunk payloads.
    fn manifest(&self) -> &WorldManifest;

    /// Optional shared validated catalogue index, reused by admission and queries.
    fn manifest_index(&self) -> Option<Arc<ManifestIndex>> {
        None
    }

    /// Loads exactly one chunk synchronously for tools or the background worker.
    fn load_chunk(&self, coordinate: ChunkId) -> RuntimeResult<ChunkPackage>;

    /// Cooperative load hook. The default checks cancellation before and after IO.
    fn load_chunk_cancelled(
        &self,
        coordinate: ChunkId,
        cancellation: &CancellationToken,
    ) -> RuntimeResult<ChunkPackage> {
        cancellation.check()?;
        let chunk = self.load_chunk(coordinate)?;
        cancellation.check()?;
        Ok(chunk)
    }
}

/// Filesystem source that reads `manifest.ron` once and chunk files on demand.
#[derive(Debug)]
pub struct FileChunkSource {
    root: PathBuf,
    manifest: WorldManifest,
    index: Arc<ManifestIndex>,
    descriptors: BTreeMap<ChunkId, ChunkDescriptor>,
    limits: IoLimits,
}

impl FileChunkSource {
    /// Opens a stable authoring workspace's current revision, or an immutable package directory.
    /// Workspace pointers are bounded, confined under `packages/`, and hash-checked.
    pub fn open_workspace(workspace: impl AsRef<Path>, limits: IoLimits) -> RuntimeResult<Self> {
        let root = workspace
            .as_ref()
            .canonicalize()
            .map_err(RuntimeError::io)?;
        if root.join("manifest.ron").is_file() {
            return Self::open(root.join("manifest.ron"), limits);
        }
        let pointer: WorkspacePointer = read_bounded(
            &checked_existing_path(&root, "current.ron")?,
            limits.max_manifest_bytes,
            &CancellationToken::default(),
        )?;
        pointer.validate()?;
        let manifest_path = checked_existing_path(&root, &pointer.manifest_path)?;
        let source = Self::open(manifest_path, limits)?;
        if source.manifest.fingerprint != pointer.manifest_fingerprint
            || source.manifest.world_id != pointer.world_id
        {
            return Err(RuntimeError::invalid(
                "workspace pointer and package manifest identity disagree",
            ));
        }
        Ok(source)
    }
    /// Opens and validates only the manifest; no chunk terrain is read here.
    pub fn open(manifest_path: impl AsRef<Path>, limits: IoLimits) -> RuntimeResult<Self> {
        let manifest_path = manifest_path
            .as_ref()
            .canonicalize()
            .map_err(RuntimeError::io)?;
        let root = manifest_path
            .parent()
            .ok_or_else(|| RuntimeError::invalid("manifest has no parent directory"))?
            .to_path_buf();
        let manifest: WorldManifest = read_bounded(
            &manifest_path,
            limits.max_manifest_bytes,
            &CancellationToken::default(),
        )?;
        let index = Arc::new(
            ManifestIndex::new(Arc::new(manifest.clone())).map_err(RuntimeError::invalid)?,
        );
        let descriptors = manifest
            .chunks
            .iter()
            .map(|descriptor| (descriptor.coordinate, descriptor.clone()))
            .collect();
        Ok(Self {
            root,
            manifest,
            index,
            descriptors,
            limits,
        })
    }

    /// Validated manifest, without loading chunk payloads.
    #[must_use]
    pub fn manifest(&self) -> &WorldManifest {
        &self.manifest
    }

    /// Loads and validates one descriptor's chunk.
    pub fn load_chunk(&self, coordinate: ChunkId) -> RuntimeResult<ChunkPackage> {
        self.load_chunk_cancelled(coordinate, &CancellationToken::default())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspacePointer {
    schema_version: u32,
    world_id: String,
    manifest_fingerprint: u64,
    manifest_path: String,
    fingerprint: u64,
}

impl WorkspacePointer {
    fn expected_fingerprint(&self) -> RuntimeResult<u64> {
        let mut value = self.clone();
        value.fingerprint = 0;
        hex_world_contracts::hash_serializable(&value).map_err(RuntimeError::invalid)
    }
    fn validate(&self) -> RuntimeResult<()> {
        if self.schema_version != hex_world_contracts::SCHEMA_VERSION
            || self.fingerprint != self.expected_fingerprint()?
            || self.manifest_path
                != format!("packages/{:016x}/manifest.ron", self.manifest_fingerprint)
        {
            return Err(RuntimeError::invalid(
                "invalid or escaping workspace revision pointer",
            ));
        }
        Ok(())
    }
}

/// Publishes an immutable revision, then atomically advances a stable workspace pointer.
/// Returns the immutable manifest path. Failed compilation/package publication never
/// advances `current.ron`; concurrent publishers are serialized by an OS file lock.
pub fn publish_revision(
    workspace: impl AsRef<Path>,
    package: &WorldPackage,
    limits: IoLimits,
) -> RuntimeResult<PathBuf> {
    package.validate().map_err(RuntimeError::invalid)?;
    fs::create_dir_all(workspace.as_ref()).map_err(RuntimeError::io)?;
    let root = workspace
        .as_ref()
        .canonicalize()
        .map_err(RuntimeError::io)?;
    if root.join("manifest.ron").exists() {
        return Err(RuntimeError::new(
            ErrorKind::Conflict,
            "immutable package directory cannot become an authoring workspace",
        ));
    }
    let writer = lock_directory(&root)?;
    if root.join("current.ron").exists() {
        let prior = FileChunkSource::open_workspace(&root, limits)?;
        if prior.manifest.world_id != package.manifest.world_id {
            return Err(RuntimeError::new(
                ErrorKind::Conflict,
                "workspace belongs to a different world",
            ));
        }
    }
    let packages = root.join("packages");
    fs::create_dir_all(&packages).map_err(RuntimeError::io)?;
    if packages.canonicalize().map_err(RuntimeError::io)? != packages {
        return Err(RuntimeError::invalid(
            "workspace packages directory must not be a symlink",
        ));
    }
    let revision = packages.join(format!("{:016x}", package.manifest.fingerprint));
    publish_package(&revision, package, limits)?;
    let mut pointer = WorkspacePointer {
        schema_version: hex_world_contracts::SCHEMA_VERSION,
        world_id: package.manifest.world_id.clone(),
        manifest_fingerprint: package.manifest.fingerprint,
        manifest_path: format!(
            "packages/{:016x}/manifest.ron",
            package.manifest.fingerprint
        ),
        fingerprint: 0,
    };
    pointer.fingerprint = pointer.expected_fingerprint()?;
    pointer.validate()?;
    atomic_write_head(&root, "current.ron", &pointer, limits.max_manifest_bytes)?;
    drop(writer);
    Ok(revision.join("manifest.ron"))
}

pub(crate) fn lock_directory(root: &Path) -> RuntimeResult<File> {
    let path = root.join("writer.lock");
    if path.exists() {
        let _safe = checked_existing_path(root, "writer.lock")?;
    }
    let writer = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(RuntimeError::io)?;
    writer.try_lock().map_err(|error| {
        RuntimeError::new(
            ErrorKind::Conflict,
            format!("directory already has a writer or cannot be locked: {error}"),
        )
    })?;
    Ok(writer)
}

pub(crate) fn atomic_write_head<T: Serialize>(
    root: &Path,
    name: &str,
    value: &T,
    maximum: usize,
) -> RuntimeResult<()> {
    let path = checked_relative_path(root, name)?;
    let bytes = encode_bounded(value, maximum)?;
    if let Some(parent) = root.parent() {
        sync_directory(parent)?;
    }
    AtomicFile::new(path, AllowOverwrite)
        .write(|file| {
            file.write_all(&bytes)?;
            file.sync_all()
        })
        .map_err(RuntimeError::io)?;
    sync_directory(root)
}

impl ChunkSource for FileChunkSource {
    fn manifest_index(&self) -> Option<Arc<ManifestIndex>> {
        Some(Arc::clone(&self.index))
    }

    fn manifest(&self) -> &WorldManifest {
        &self.manifest
    }

    fn load_chunk(&self, coordinate: ChunkId) -> RuntimeResult<ChunkPackage> {
        Self::load_chunk(self, coordinate)
    }

    fn load_chunk_cancelled(
        &self,
        coordinate: ChunkId,
        cancellation: &CancellationToken,
    ) -> RuntimeResult<ChunkPackage> {
        let descriptor = self.descriptors.get(&coordinate).ok_or_else(|| {
            RuntimeError::invalid(format!("chunk {coordinate:?} is outside the manifest"))
        })?;
        let path = checked_existing_path(&self.root, &descriptor.path)?;
        let chunk: ChunkPackage = read_bounded(&path, self.limits.max_chunk_bytes, cancellation)?;
        validate_source_chunk(&self.index, descriptor, &chunk)?;
        Ok(chunk)
    }
}

/// Already validated in-memory source, primarily for fixtures and embedded worlds.
#[derive(Debug)]
pub struct MemoryChunkSource {
    package: WorldPackage,
    index: Arc<ManifestIndex>,
}

impl MemoryChunkSource {
    /// Validates the complete caller-supplied package before retaining it.
    pub fn new(package: WorldPackage) -> RuntimeResult<Self> {
        package.validate().map_err(RuntimeError::invalid)?;
        let index = Arc::new(
            ManifestIndex::new(Arc::new(package.manifest.clone()))
                .map_err(RuntimeError::invalid)?,
        );
        Ok(Self { package, index })
    }
}

impl ChunkSource for MemoryChunkSource {
    fn manifest_index(&self) -> Option<Arc<ManifestIndex>> {
        Some(Arc::clone(&self.index))
    }

    fn manifest(&self) -> &WorldManifest {
        &self.package.manifest
    }
    fn load_chunk(&self, coordinate: ChunkId) -> RuntimeResult<ChunkPackage> {
        self.package
            .chunks
            .get(&coordinate)
            .cloned()
            .ok_or_else(|| RuntimeError::invalid(format!("unknown source chunk {coordinate:?}")))
    }
}

/// Publishes a new immutable package directory atomically.
///
/// Every chunk is flushed before `manifest.ron`, then the staging directory is
/// renamed into place. An existing destination is never overwritten. Identical
/// retries are accepted after validating the existing manifest identity.
pub fn publish_package(
    root: impl AsRef<Path>,
    package: &WorldPackage,
    limits: IoLimits,
) -> RuntimeResult<()> {
    package.validate().map_err(RuntimeError::invalid)?;
    let root = root.as_ref();
    if root.exists() {
        let existing = FileChunkSource::open(root.join("manifest.ron"), limits)?;
        if existing.manifest.fingerprint == package.manifest.fingerprint {
            for descriptor in &package.manifest.chunks {
                let _validated = existing.load_chunk(descriptor.coordinate)?;
            }
            return Ok(());
        }
        return Err(RuntimeError::new(
            ErrorKind::Conflict,
            "immutable package destination already exists",
        ));
    }
    let parent = root
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(RuntimeError::io)?;
    let stage = temporary_path(parent, "package");
    fs::create_dir(&stage).map_err(RuntimeError::io)?;
    let result = (|| {
        for descriptor in &package.manifest.chunks {
            let chunk = package
                .chunks
                .get(&descriptor.coordinate)
                .ok_or_else(|| RuntimeError::invalid("manifest chunk missing from package"))?;
            let path = checked_relative_path(&stage, &descriptor.path)?;
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(RuntimeError::io)?;
            }
            write_new(&path, &encode_bounded(chunk, limits.max_chunk_bytes)?)?;
        }
        write_new(
            &stage.join("manifest.ron"),
            &encode_bounded(&package.manifest, limits.max_manifest_bytes)?,
        )?;
        sync_directory_tree(&stage)?;
        fs::rename(&stage, root).map_err(RuntimeError::io)?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _cleanup = fs::remove_dir_all(&stage);
    }
    result
}

pub(crate) fn validate_source_chunk(
    index: &ManifestIndex,
    descriptor: &ChunkDescriptor,
    chunk: &ChunkPackage,
) -> RuntimeResult<()> {
    if chunk
        .semantics
        .objects
        .iter()
        .map(|object| object.id.as_str())
        .chain(
            chunk
                .semantics
                .object_influences
                .iter()
                .map(|influence| influence.id.as_str()),
        )
        .any(|id| id.starts_with(hex_world_contracts::RUNTIME_OBJECT_PREFIX))
    {
        return Err(RuntimeError::invalid(
            "compiled source uses reserved runtime object identity",
        ));
    }
    chunk
        .validate_with_index(index)
        .map_err(RuntimeError::invalid)?;
    if chunk.world_id != index.manifest().world_id
        || chunk.coordinate != descriptor.coordinate
        || chunk.fingerprint != descriptor.fingerprint
    {
        return Err(RuntimeError::invalid(
            "chunk world, coordinate, or fingerprint disagrees with manifest",
        ));
    }
    Ok(())
}

pub(crate) fn in_disk(
    position: hex_world_contracts::WorldHex,
    center: hex_world_contracts::WorldHex,
    radius: u32,
) -> bool {
    let q = i128::from(position.q) - i128::from(center.q);
    let r = i128::from(position.r) - i128::from(center.r);
    q.abs().max(r.abs()).max((q + r).abs()) <= i128::from(radius)
}

pub(crate) fn checked_relative_path(root: &Path, relative: &str) -> RuntimeResult<PathBuf> {
    let path = Path::new(relative);
    if relative.is_empty()
        || relative.contains('\\')
        || relative.contains(':')
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(RuntimeError::invalid(format!(
            "unsafe relative path {relative:?}"
        )));
    }
    Ok(root.join(path))
}

pub(crate) fn ensure_relative_directory(root: &Path, relative: &Path) -> RuntimeResult<PathBuf> {
    let value = relative
        .to_str()
        .ok_or_else(|| RuntimeError::invalid("non-UTF8 content directory"))?;
    let _checked = checked_relative_path(root, value)?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(RuntimeError::invalid("unsafe content directory"));
        };
        current.push(name);
        if current.exists() {
            let metadata = fs::symlink_metadata(&current).map_err(RuntimeError::io)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(RuntimeError::invalid(
                    "content directory is a symlink or non-directory",
                ));
            }
        } else {
            fs::create_dir(&current).map_err(RuntimeError::io)?;
            if let Some(parent) = current.parent() {
                sync_directory(parent)?;
            }
        }
    }
    Ok(current)
}

pub(crate) fn checked_existing_path(root: &Path, relative: &str) -> RuntimeResult<PathBuf> {
    let candidate = checked_relative_path(root, relative)?
        .canonicalize()
        .map_err(RuntimeError::io)?;
    let canonical_root = root.canonicalize().map_err(RuntimeError::io)?;
    if !candidate.starts_with(&canonical_root) {
        return Err(RuntimeError::invalid(
            "package path escapes its root through a symlink",
        ));
    }
    if !candidate.is_file() {
        return Err(RuntimeError::invalid("package path is not a regular file"));
    }
    Ok(candidate)
}

pub(crate) fn read_bounded<T: DeserializeOwned>(
    path: &Path,
    maximum: usize,
    cancellation: &CancellationToken,
) -> RuntimeResult<T> {
    let bytes = read_bytes_bounded(path, maximum, cancellation)?;
    ron::de::from_bytes(&bytes).map_err(RuntimeError::invalid)
}

pub(crate) fn read_bytes_bounded(
    path: &Path,
    maximum: usize,
    cancellation: &CancellationToken,
) -> RuntimeResult<Vec<u8>> {
    cancellation.check()?;
    let mut file = File::open(path).map_err(RuntimeError::io)?;
    if file.metadata().map_err(RuntimeError::io)?.len() > u64::try_from(maximum).unwrap_or(u64::MAX)
    {
        return Err(RuntimeError::new(
            ErrorKind::Limit,
            format!("{} exceeds {maximum} bytes", path.display()),
        ));
    }
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        cancellation.check()?;
        let count = file.read(&mut buffer).map_err(RuntimeError::io)?;
        if count == 0 {
            break;
        }
        if bytes.len().saturating_add(count) > maximum {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "file grew beyond its read budget",
            ));
        }
        if let Some(part) = buffer.get(..count) {
            bytes.extend_from_slice(part);
        }
    }
    cancellation.check()?;
    Ok(bytes)
}

pub(crate) fn encode_bounded<T: Serialize>(value: &T, maximum: usize) -> RuntimeResult<Vec<u8>> {
    struct BoundedWriter {
        bytes: Vec<u8>,
        maximum: usize,
        exceeded: bool,
    }
    impl std::fmt::Write for BoundedWriter {
        fn write_str(&mut self, value: &str) -> std::fmt::Result {
            if self.bytes.len().saturating_add(value.len()) > self.maximum {
                self.exceeded = true;
                return Err(std::fmt::Error);
            }
            self.bytes.extend_from_slice(value.as_bytes());
            Ok(())
        }
    }
    let mut writer = BoundedWriter {
        bytes: Vec::new(),
        maximum,
        exceeded: false,
    };
    let result = ron::ser::to_writer(&mut writer, value);
    if writer.exceeded {
        return Err(RuntimeError::new(
            ErrorKind::Limit,
            format!("encoded file exceeds {maximum} bytes"),
        ));
    }
    result.map_err(RuntimeError::invalid)?;
    Ok(writer.bytes)
}

pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> RuntimeResult<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(RuntimeError::io)?;
    file.write_all(bytes).map_err(RuntimeError::io)?;
    file.sync_all().map_err(RuntimeError::io)
}

pub(crate) fn temporary_path(parent: &Path, kind: &str) -> PathBuf {
    parent.join(format!(
        ".hex-v4-{kind}-{}-{}",
        std::process::id(),
        TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}

pub(crate) fn sync_directory(path: &Path) -> RuntimeResult<()> {
    #[cfg(unix)]
    {
        File::open(path)
            .and_then(|directory| directory.sync_all())
            .map_err(RuntimeError::io)
    }
    #[cfg(not(unix))]
    {
        let _path = path;
        Ok(())
    }
}

fn sync_directory_tree(root: &Path) -> RuntimeResult<()> {
    for entry in fs::read_dir(root).map_err(RuntimeError::io)? {
        let entry = entry.map_err(RuntimeError::io)?;
        if entry.file_type().map_err(RuntimeError::io)?.is_dir() {
            sync_directory_tree(&entry.path())?;
        }
    }
    sync_directory(root)
}

/// Reuses a source's validated index only when its complete manifest agrees.
pub(crate) fn source_index(source: &dyn ChunkSource) -> RuntimeResult<Arc<ManifestIndex>> {
    if let Some(index) = source.manifest_index() {
        if index.manifest() != source.manifest() {
            return Err(RuntimeError::invalid("source index and manifest disagree"));
        }
        return Ok(index);
    }
    ManifestIndex::new(Arc::new(source.manifest().clone()))
        .map(Arc::new)
        .map_err(RuntimeError::invalid)
}
