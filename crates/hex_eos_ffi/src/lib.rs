//! Minimal audited dynamic boundary to the official EOS C SDK.
//!
//! Ordinary source builds carry no EOS binary and never search the working directory.
//! Release/acceptance code supplies one explicit checksum-pinned runtime path. This crate
//! is the only workspace member allowed to contain `unsafe`; every pointer and symbol is
//! converted into owned safe Rust data before leaving this module.

use std::{
    ffi::{c_char, CStr},
    fmt,
    path::{Path, PathBuf},
};

use libloading::Library;

/// Official SDK header baseline selected for the integration.
///
/// Epic's May 2026 platform notice identifies 1.19.1 as the current desktop baseline.
/// Protected release CI must mount headers and runtimes from this exact SDK artifact until
/// an explicit audited upgrade changes this constant.
pub const EOS_SDK_HEADER_VERSION: &str = "1.19.1";

type EosGetVersion = unsafe extern "C" fn() -> *const c_char;

const _: () = {
    assert!(std::mem::size_of::<EosGetVersion>() == std::mem::size_of::<*const ()>());
    assert!(std::mem::align_of::<EosGetVersion>() == std::mem::align_of::<*const ()>());
};

/// Loaded official EOS runtime and its first audited function declaration.
///
/// The library handle outlives every copied function pointer. Additional declarations
/// are added only with protected header ABI evidence in the EOS feasibility foundation.
pub struct EosRuntimeLibrary {
    get_version: EosGetVersion,
    library: Library,
    path: PathBuf,
}

impl fmt::Debug for EosRuntimeLibrary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EosRuntimeLibrary")
            .field("path", &self.path)
            .field("version", &self.version().ok())
            .finish_non_exhaustive()
    }
}

impl EosRuntimeLibrary {
    /// Loads an explicitly selected EOS runtime and verifies its reported version.
    ///
    /// No implicit current-directory, environment, registry, or system-library search is
    /// performed. The release staging layer owns checksum verification before calling this
    /// function.
    pub fn load_explicit(path: impl AsRef<Path>) -> Result<Self, EosRuntimeLoadError> {
        let path = path.as_ref();
        if !path.is_absolute() {
            return Err(EosRuntimeLoadError::PathMustBeAbsolute);
        }
        if path.file_name().and_then(|name| name.to_str()) != Some(runtime_file_name()) {
            return Err(EosRuntimeLoadError::UnexpectedFileName);
        }

        // SAFETY: The caller supplies an explicit release-staged path. Loading a dynamic
        // library necessarily runs platform loader code; this unsafe capability is confined
        // to this crate and the handle is retained for every symbol's lifetime.
        let library = unsafe { Library::new(path) }
            .map_err(|error| EosRuntimeLoadError::Open(error.to_string()))?;
        // SAFETY: `EOS_GetVersion` is a no-argument function returning a borrowed C string
        // in every supported official EOS C SDK. The copied function pointer cannot outlive
        // `library` because both are fields of the same value.
        let get_version = unsafe {
            *library
                .get::<EosGetVersion>(b"EOS_GetVersion\0")
                .map_err(|error| EosRuntimeLoadError::MissingVersionSymbol(error.to_string()))?
        };
        let runtime = Self {
            get_version,
            library,
            path: path.to_path_buf(),
        };
        let version = runtime.version()?;
        if !version.is_compatible_with_headers() {
            return Err(EosRuntimeLoadError::IncompatibleVersion(version));
        }
        Ok(runtime)
    }

    /// Returns the official runtime version as owned validated text.
    pub fn version(&self) -> Result<EosRuntimeVersion, EosRuntimeLoadError> {
        // Read the handle so its intentional lifetime relationship remains visible to both
        // reviewers and dead-code analysis.
        let _keep_loaded = &self.library;
        // SAFETY: The function pointer was resolved from the retained official runtime.
        let pointer = unsafe { (self.get_version)() };
        if pointer.is_null() {
            return Err(EosRuntimeLoadError::NullVersion);
        }
        // SAFETY: The official EOS contract returns a process-lifetime NUL-terminated
        // string. It is copied immediately and no borrowed pointer escapes.
        let value = unsafe { CStr::from_ptr(pointer) }
            .to_str()
            .map_err(|error| EosRuntimeLoadError::InvalidVersionText(error.to_string()))?;
        EosRuntimeVersion::parse(value)
    }

    /// Exact staged runtime path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Parsed official EOS runtime version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EosRuntimeVersion {
    raw: String,
    /// Major SDK component.
    pub major: u16,
    /// Minor SDK component.
    pub minor: u16,
    /// Patch SDK component.
    pub patch: u16,
}

impl EosRuntimeVersion {
    /// Parses the leading `major.minor.patch` components reported by `EOS_GetVersion`.
    pub fn parse(value: &str) -> Result<Self, EosRuntimeLoadError> {
        let numeric = value
            .split(|character: char| !(character.is_ascii_digit() || character == '.'))
            .find(|segment| segment.matches('.').count() >= 2)
            .ok_or_else(|| EosRuntimeLoadError::InvalidVersionText(value.to_owned()))?;
        let mut parts = numeric.split('.');
        let major = parse_version_part(parts.next(), value)?;
        let minor = parse_version_part(parts.next(), value)?;
        let patch = parse_version_part(parts.next(), value)?;
        Ok(Self {
            raw: value.to_owned(),
            major,
            minor,
            patch,
        })
    }

    /// Whether this runtime exactly matches the pinned 1.19.1 declarations.
    #[must_use]
    pub fn is_compatible_with_headers(&self) -> bool {
        (self.major, self.minor, self.patch) == (1, 19, 1)
    }

    /// Borrows the complete official version text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.raw
    }
}

/// Platform-specific official EOS shipping-runtime filename.
#[must_use]
pub const fn runtime_file_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "EOSSDK-Win64-Shipping.dll"
    } else if cfg!(target_os = "macos") {
        "libEOSSDK-Mac-Shipping.dylib"
    } else {
        "libEOSSDK-Linux-Shipping.so"
    }
}

/// Why the official EOS runtime could not be loaded safely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EosRuntimeLoadError {
    /// Only an explicit absolute staged path is accepted.
    PathMustBeAbsolute,
    /// The selected filename is not the official target shipping-runtime name.
    UnexpectedFileName,
    /// Platform loader rejected the library.
    Open(String),
    /// The required version symbol is absent.
    MissingVersionSymbol(String),
    /// The version function returned null.
    NullVersion,
    /// Runtime version text is malformed or not UTF-8.
    InvalidVersionText(String),
    /// Runtime is older or from a different major ABI than the pinned headers.
    IncompatibleVersion(EosRuntimeVersion),
}

impl fmt::Display for EosRuntimeLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PathMustBeAbsolute => "EOS runtime path must be absolute",
            Self::UnexpectedFileName => "EOS runtime filename does not match this target",
            Self::Open(_) => "EOS runtime could not be opened",
            Self::MissingVersionSymbol(_) => "EOS runtime lacks EOS_GetVersion",
            Self::NullVersion => "EOS runtime returned a null version",
            Self::InvalidVersionText(_) => "EOS runtime version text is invalid",
            Self::IncompatibleVersion(_) => "EOS runtime is incompatible with pinned headers",
        })
    }
}

impl std::error::Error for EosRuntimeLoadError {}

fn parse_version_part(value: Option<&str>, raw: &str) -> Result<u16, EosRuntimeLoadError> {
    value
        .and_then(|part| part.parse::<u16>().ok())
        .ok_or_else(|| EosRuntimeLoadError::InvalidVersionText(raw.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_runtime_version_requires_an_exact_audited_sdk() {
        let exact = EosRuntimeVersion::parse("1.19.1").expect("version parses");
        let decorated = EosRuntimeVersion::parse("EOS SDK 1.19.1-CL-123").expect("version parses");
        assert!(exact.is_compatible_with_headers());
        assert!(decorated.is_compatible_with_headers());
        assert!(!EosRuntimeVersion::parse("1.19.2")
            .expect("version parses")
            .is_compatible_with_headers());
        assert!(!EosRuntimeVersion::parse("1.18.0")
            .expect("version parses")
            .is_compatible_with_headers());
        assert!(!EosRuntimeVersion::parse("2.0.0")
            .expect("version parses")
            .is_compatible_with_headers());
    }

    #[test]
    fn runtime_loader_refuses_implicit_or_wrong_named_paths_before_loading() {
        assert!(matches!(
            EosRuntimeLibrary::load_explicit(runtime_file_name()),
            Err(EosRuntimeLoadError::PathMustBeAbsolute)
        ));
        let wrong = std::env::temp_dir().join("untrusted-eos-runtime.bin");
        assert!(matches!(
            EosRuntimeLibrary::load_explicit(wrong),
            Err(EosRuntimeLoadError::UnexpectedFileName)
        ));
    }
}
