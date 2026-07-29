//! Command-line parsing and repository-root discovery for the standalone editor.

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

const PROJECT_ROOT_FLAG: &str = "--project-root";
const PALETTE_PATH: &str = "assets/art/palette.ron";
const STYLES_PATH: &str = "assets/art/voxel_styles.ron";

/// A startup failure that can be shown in the editor window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchError {
    detail: String,
}

impl LaunchError {
    fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }

    /// Human-readable failure detail.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for LaunchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for LaunchError {}

/// Resolves the repository that owns the tracked art catalogs.
///
/// `arguments` includes the executable name. Without `--project-root`, discovery
/// walks upward from `current_directory`.
pub fn resolve_repository_root(
    arguments: impl IntoIterator<Item = OsString>,
    current_directory: &Path,
) -> Result<PathBuf, LaunchError> {
    let mut arguments = arguments.into_iter();
    drop(arguments.next());
    let mut explicit_root = None;
    while let Some(argument) = arguments.next() {
        let argument_text = argument.to_string_lossy();
        if argument_text == PROJECT_ROOT_FLAG {
            if explicit_root.is_some() {
                return Err(LaunchError::new(
                    "--project-root may be supplied at most once",
                ));
            }
            let Some(path) = arguments.next() else {
                return Err(LaunchError::new("--project-root requires a path"));
            };
            explicit_root = Some(PathBuf::from(path));
            continue;
        }
        if let Some(path) = argument_text.strip_prefix("--project-root=") {
            if explicit_root.is_some() {
                return Err(LaunchError::new(
                    "--project-root may be supplied at most once",
                ));
            }
            if path.is_empty() {
                return Err(LaunchError::new("--project-root requires a path"));
            }
            explicit_root = Some(PathBuf::from(path));
            continue;
        }
        return Err(LaunchError::new(format!(
            "unrecognized editor argument '{argument_text}'"
        )));
    }

    let candidate = match explicit_root {
        Some(path) if path.is_absolute() => path,
        Some(path) => current_directory.join(path),
        None => discover_upward(current_directory).ok_or_else(|| {
            LaunchError::new(format!(
                "could not find {PALETTE_PATH} and {STYLES_PATH} above '{}'; pass --project-root",
                current_directory.display()
            ))
        })?,
    };
    validate_repository_root(&candidate)?;
    fs::canonicalize(&candidate).map_err(|error| {
        LaunchError::new(format!(
            "cannot resolve repository root '{}': {error}",
            candidate.display()
        ))
    })
}

fn discover_upward(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|candidate| is_repository_root(candidate))
        .map(Path::to_path_buf)
}

fn validate_repository_root(root: &Path) -> Result<(), LaunchError> {
    if is_repository_root(root) {
        return Ok(());
    }
    Err(LaunchError::new(format!(
        "'{}' is not an Asset Workshop project: expected {PALETTE_PATH} and {STYLES_PATH}",
        root.display()
    )))
}

fn is_repository_root(root: &Path) -> bool {
    root.join(PALETTE_PATH).is_file() && root.join(STYLES_PATH).is_file()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestTree {
        root: PathBuf,
    }

    impl TestTree {
        fn new() -> Self {
            let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "hex-editor-launch-test-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(root.join("assets/art"))
                .expect("test art directory should be created");
            fs::write(root.join(PALETTE_PATH), "()")
                .expect("test palette marker should be written");
            fs::write(root.join(STYLES_PATH), "()").expect("test style marker should be written");
            Self { root }
        }
    }

    impl Drop for TestTree {
        fn drop(&mut self) {
            drop(fs::remove_dir_all(&self.root));
        }
    }

    #[test]
    fn discovers_project_from_a_nested_working_directory() {
        let tree = TestTree::new();
        let nested = tree.root.join("crates/hex_editor");
        fs::create_dir_all(&nested).expect("nested test directory should be created");
        let resolved = resolve_repository_root([OsString::from("hex_editor")], &nested)
            .expect("repository should be discovered");
        assert_eq!(
            resolved,
            fs::canonicalize(&tree.root).expect("test root should canonicalize")
        );
    }

    #[test]
    fn explicit_relative_root_and_equals_form_are_supported() {
        let tree = TestTree::new();
        let parent = tree.root.parent().expect("test root should have a parent");
        let name = tree
            .root
            .file_name()
            .expect("test root should have a filename")
            .to_string_lossy();
        let argument = OsString::from(format!("--project-root={name}"));
        let resolved = resolve_repository_root([OsString::from("hex_editor"), argument], parent)
            .expect("explicit repository should resolve");
        assert_eq!(
            resolved,
            fs::canonicalize(&tree.root).expect("test root should canonicalize")
        );
    }

    #[test]
    fn malformed_arguments_and_non_projects_are_actionable() {
        let tree = TestTree::new();
        let missing = resolve_repository_root(
            [
                OsString::from("hex_editor"),
                OsString::from("--project-root"),
            ],
            &tree.root,
        )
        .expect_err("missing flag value must fail");
        assert!(missing.detail().contains("requires a path"));

        let unknown = resolve_repository_root(
            [OsString::from("hex_editor"), OsString::from("--wat")],
            &tree.root,
        )
        .expect_err("unknown flag must fail");
        assert!(unknown.detail().contains("unrecognized"));

        let non_project = tree.root.join("empty");
        fs::create_dir_all(&non_project).expect("empty directory should be created");
        let invalid = resolve_repository_root(
            [
                OsString::from("hex_editor"),
                OsString::from("--project-root"),
                non_project.into_os_string(),
            ],
            &tree.root,
        )
        .expect_err("non-project must fail");
        assert!(invalid.detail().contains("not an Asset Workshop project"));
    }
}
