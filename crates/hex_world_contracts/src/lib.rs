//! Renderer-free V4 world contracts.
//!
//! Deserialization establishes wire shape, not trust. Call [`Validate::validate`]
//! (or [`parse_ron`]) before admitting external data. [`Seal::seal`] is the separate
//! producer operation that canonicalizes ordering and computes fingerprints.
//! Neither operation reads files, schedules work, or owns gameplay authority.

mod geometry;
mod index;
mod model;
mod traversal;
mod validation;

pub use geometry::*;
pub use index::ManifestIndex;
pub use model::*;
pub use traversal::*;
pub use validation::*;

use serde::{de::DeserializeOwned, Serialize};
use std::fmt;

/// Version of the initial streamed-world wire format.
pub const SCHEMA_VERSION: u32 = 1;

/// Nominal axial q/r pitch of the regular schema-v1 map-summary lattice.
///
/// Producers and atlas consumers share this spacing instead of hardcoding it.
/// Additional landmark observations may lie off the lattice. Summary spacing is
/// a coarse presentation convention, never exact terrain coverage or disclosure.
pub const SUMMARY_SAMPLE_PITCH: i64 = 12;

/// A precise rejection at a public contract boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractError {
    /// Field or object that failed validation.
    pub context: String,
    /// Human-readable reason, without implementation-private state.
    pub message: String,
}

impl ContractError {
    /// Create a contextual contract rejection.
    pub fn new(context: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            context: context.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.context, self.message)
    }
}

impl std::error::Error for ContractError {}

/// Validate existing data without changing, sorting, or repairing it.
pub trait Validate {
    /// Reject malformed, noncanonical, or inconsistent data.
    fn validate(&self) -> Result<(), ContractError>;
}

/// Canonicalize trusted producer data, then fingerprint and validate it.
pub trait Seal {
    /// Seal the producer value. This does not admit untrusted input.
    fn seal(&mut self) -> Result<(), ContractError>;
}

/// Values with a defined fingerprint-field exclusion policy.
pub trait CanonicalFingerprint {
    /// Compute the expected fingerprint without trusting the stored value.
    fn canonical_fingerprint(&self) -> Result<u64, ContractError>;
}

/// Compute a package fingerprint while excluding its own stored fingerprint.
pub fn fingerprint<T: CanonicalFingerprint + ?Sized>(value: &T) -> Result<u64, ContractError> {
    value.canonical_fingerprint()
}

/// Hash a deterministic serialization of an auxiliary source or persistence DTO.
///
/// Callers must use ordered maps and integer authority fields. This helper does
/// not normalize collections or validate a DTO. Package hashes instead use
/// [`fingerprint`] to exclude their stored checksum. xxh3 provides accidental
/// corruption detection, not authentication against an adversarial writer.
pub fn hash_serializable<T: Serialize + ?Sized>(value: &T) -> Result<u64, ContractError> {
    let bytes = ron::ser::to_string(value)
        .map_err(|error| ContractError::new("serialization", error.to_string()))?;
    Ok(xxhash_rust::xxh3::xxh3_64(bytes.as_bytes()))
}

/// Parse strict RON wire data and validate it before returning it.
///
/// The filesystem adapter must bound bytes before this call; limits here are per
/// package/operation and never impose a total-world size cap.
pub fn parse_ron<T: DeserializeOwned + Validate>(source: &str) -> Result<T, ContractError> {
    let value: T =
        ron::from_str(source).map_err(|error| ContractError::new("wire", error.to_string()))?;
    value.validate()?;
    Ok(value)
}
