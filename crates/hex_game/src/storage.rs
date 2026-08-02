//! Small cross-platform storage boundary for local pre-alpha state.

use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use atomicwrites::{AllowOverwrite, AtomicFile};
use bevy::prelude::*;

#[cfg(test)]
std::thread_local! {
    static STORAGE_ACCESS_LOG: std::cell::RefCell<Vec<StorageAccess>> = const {
        std::cell::RefCell::new(Vec::new())
    };
}

/// Test-only operation recorded at the local persistence boundary.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StorageAccessKind {
    Read,
    Write,
}

/// One test-only access to an exact local persistence path.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StorageAccess {
    pub(crate) kind: StorageAccessKind,
    pub(crate) path: PathBuf,
}

/// Stable application identity used by artifacts and local data directories.
pub(crate) const APP_ID: &str = "com.chillgamerboys.hex-game";
/// Player-facing application name.
pub(crate) const APP_NAME: &str = "Hex Game";

/// Exact local files owned by runtime persistence and legacy compatibility.
#[derive(Resource, Debug, Clone)]
pub(crate) struct StoragePaths {
    pub(crate) preferences: PathBuf,
    /// Canonical three-slot Campaign save file.
    pub(crate) campaigns: PathBuf,
    /// Read-only compatibility source for the former single resume slot.
    pub(crate) resume: PathBuf,
    pub(crate) creations: PathBuf,
    /// Retired report path retained only to prove existing bytes stay untouched.
    #[cfg(test)]
    pub(crate) combat_reports: PathBuf,
}

impl Default for StoragePaths {
    fn default() -> Self {
        Self::under(data_root())
    }
}

impl StoragePaths {
    /// Builds the complete storage projection beneath one explicit root.
    ///
    /// Runtime defaults and disposable tooling sessions share the filenames;
    /// only the owning root differs.
    pub(crate) fn under(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref();
        Self {
            preferences: root.join("preferences.ron"),
            campaigns: root.join("campaigns.ron"),
            resume: root.join("resume.ron"),
            creations: root.join("creations.ron"),
            #[cfg(test)]
            combat_reports: root.join("combat-reports.ron"),
        }
    }
}

fn data_root() -> PathBuf {
    if let Some(path) = std::env::var_os("HEX_GAME_DATA_DIR") {
        return PathBuf::from(path);
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(path) = std::env::var_os("APPDATA") {
            return PathBuf::from(path).join(APP_NAME);
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(path) = home_dir() {
            return path.join("Library/Application Support").join(APP_NAME);
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
            return PathBuf::from(path).join("hex-game");
        }
        if let Some(path) = home_dir() {
            return path.join(".local/share/hex-game");
        }
    }

    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".hex-game")
}

#[cfg(unix)]
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub(crate) fn read(path: &Path) -> io::Result<String> {
    #[cfg(test)]
    record_access(StorageAccessKind::Read, path);
    std::fs::read_to_string(path)
}

pub(crate) fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    #[cfg(test)]
    record_access(StorageAccessKind::Write, path);
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    AtomicFile::new(path, AllowOverwrite)
        .write(|file| file.write_all(contents.as_bytes()))
        .map_err(io::Error::from)
}

#[cfg(test)]
fn record_access(kind: StorageAccessKind, path: &Path) {
    STORAGE_ACCESS_LOG.with(|log| {
        log.borrow_mut().push(StorageAccess {
            kind,
            path: path.to_path_buf(),
        });
    });
}

/// Runs one synchronous persistence action and returns its exact boundary accesses.
#[cfg(test)]
pub(crate) fn record_storage_accesses<T>(action: impl FnOnce() -> T) -> (T, Vec<StorageAccess>) {
    STORAGE_ACCESS_LOG.with(|log| log.borrow_mut().clear());
    let result = action();
    let accesses = STORAGE_ACCESS_LOG.with(|log| std::mem::take(&mut *log.borrow_mut()));
    (result, accesses)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    #[test]
    fn explicit_data_directory_has_stable_filenames() {
        let root = PathBuf::from("chosen-root");
        let paths = StoragePaths::under(&root);
        assert_eq!(paths.preferences.parent(), Some(root.as_path()));
        assert_eq!(
            paths.preferences.file_name(),
            Some(OsStr::new("preferences.ron"))
        );
        assert_eq!(paths.resume.file_name(), Some(OsStr::new("resume.ron")));
        assert_eq!(
            paths.campaigns.file_name(),
            Some(OsStr::new("campaigns.ron"))
        );
        assert_eq!(
            paths.creations.file_name(),
            Some(OsStr::new("creations.ron"))
        );
        assert_eq!(
            paths.combat_reports.file_name(),
            Some(OsStr::new("combat-reports.ron"))
        );
    }
}
