//! First-launch preparation of the implicit expedition package, before Bevy starts.

use std::{ffi::OsStr, path::PathBuf, process::Command};

use hex_core::arena::ArenaMap;

fn requires_preparation(map: ArenaMap, package_override: Option<&OsStr>) -> bool {
    map == ArenaMap::ForestMassif && package_override.is_none()
}

fn authoring_target(root: &std::path::Path, app_target: Option<&OsStr>) -> PathBuf {
    let preferred = root.join("target/v4-authoring");
    let app_target = app_target.map_or_else(
        || root.join("target"),
        |value| {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        },
    );
    let canonical_app = app_target.canonicalize().unwrap_or(app_target);
    let canonical_preferred = preferred
        .canonicalize()
        .unwrap_or_else(|_| preferred.clone());
    if canonical_app == canonical_preferred {
        preferred.join("forest-bootstrap")
    } else {
        preferred
    }
}

pub(super) fn prepare_default(map: ArenaMap) -> Result<(), String> {
    let package_override = std::env::var_os("HEX_FOREST_WORLD");
    if !requires_preparation(map, package_override.as_deref()) {
        return Ok(());
    }
    let root = std::env::var_os("BEVY_ASSET_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let root = root
        .canonicalize()
        .map_err(|error| format!("Forest asset root: {error}"))?;
    // The child may build worldc; never inherit the currently running app target.
    let target = authoring_target(&root, std::env::var_os("CARGO_TARGET_DIR").as_deref());
    let status = Command::new("python3")
        .arg(root.join("tools/forest_package.py"))
        .arg("ensure")
        .arg("--target-dir")
        .arg(&target)
        .current_dir(&root)
        .env("CARGO_TARGET_DIR", &target)
        .env("CARGO_INCREMENTAL", "0")
        .env("CARGO_BUILD_JOBS", "2")
        .status()
        .map_err(|error| format!("Cannot prepare Forest Expedition: {error}. Install Python 3 and run python3 tools/forest_package.py ensure."))?;
    if !status.success() {
        return Err(format!(
            "Forest Expedition preparation failed ({status}). Run python3 tools/forest_package.py ensure; launch stopped."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_implicit_forest_launches_prepare_a_package() {
        assert!(requires_preparation(ArenaMap::ForestMassif, None));
        for map in [ArenaMap::Duel, ArenaMap::Fort, ArenaMap::SevenRegions] {
            assert!(!requires_preparation(map, None));
        }
        assert!(!requires_preparation(
            ArenaMap::ForestMassif,
            Some(OsStr::new("legacy-package"))
        ));
        assert!(
            !requires_preparation(ArenaMap::ForestMassif, Some(OsStr::new(""))),
            "an invalid explicit override must fail through normal admission"
        );
    }

    #[test]
    fn authoring_uses_a_different_target_even_if_app_uses_the_preferred_cache() {
        let root = std::env::temp_dir().join("hex-bootstrap-target-test");
        let preferred = root.join("target/v4-authoring");
        assert_eq!(authoring_target(&root, None), preferred);
        assert_eq!(
            authoring_target(&root, Some(preferred.as_os_str())),
            preferred.join("forest-bootstrap")
        );
    }
}
