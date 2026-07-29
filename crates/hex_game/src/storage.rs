//! Small cross-platform storage boundary for disposable pre-alpha state.

use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use atomicwrites::{AllowOverwrite, AtomicFile};
use bevy::prelude::*;

/// Stable application identity used by artifacts and local data directories.
pub(crate) const APP_ID: &str = "com.chillgamerboys.hex-game";
/// Player-facing application name.
pub(crate) const APP_NAME: &str = "Hex Game";

/// Exact local files owned by the Wave 5 scaffolds.
#[derive(Resource, Debug, Clone)]
pub(crate) struct StoragePaths {
    pub(crate) preferences: PathBuf,
    pub(crate) resume: PathBuf,
    pub(crate) creations: PathBuf,
}

impl Default for StoragePaths {
    fn default() -> Self {
        let root = data_root();
        Self {
            preferences: root.join("preferences.ron"),
            resume: root.join("resume.ron"),
            creations: root.join("creations.ron"),
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

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub(crate) fn read(path: &Path) -> io::Result<String> {
    std::fs::read_to_string(path)
}

pub(crate) fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    AtomicFile::new(path, AllowOverwrite)
        .write(|file| file.write_all(contents.as_bytes()))
        .map_err(io::Error::from)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    #[test]
    fn explicit_data_directory_has_stable_filenames() {
        let root = PathBuf::from("chosen-root");
        let paths = StoragePaths {
            preferences: root.join("preferences.ron"),
            resume: root.join("resume.ron"),
            creations: root.join("creations.ron"),
        };
        assert_eq!(
            paths.preferences.file_name(),
            Some(OsStr::new("preferences.ron"))
        );
        assert_eq!(paths.resume.file_name(), Some(OsStr::new("resume.ron")));
        assert_eq!(
            paths.creations.file_name(),
            Some(OsStr::new("creations.ron"))
        );
    }
}
