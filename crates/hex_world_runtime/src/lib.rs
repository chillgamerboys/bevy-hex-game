//! Renderer-independent resident world authority.
//!
//! Immutable packages, bounded asynchronous residency, and mutable terrain journals
//! have separate lifetimes. Queries never turn an unavailable chunk into air. The
//! caller pumps completed jobs and publishes the returned immutable products into
//! its own engine; this crate never owns a renderer, combat clock, or ECS schedule.

mod edits;
mod persistence;
mod runtime;
mod source;

pub use edits::{ChunkDelta, WorldDelta};
pub use runtime::{
    ChunkProduct, LoadFailure, RuntimeConfig, RuntimeCounts, RuntimeUpdate, WorldRuntime,
};
pub use source::{
    publish_package, CancellationToken, ChunkSource, FileChunkSource, IoLimits, MemoryChunkSource,
};

use std::fmt;

/// Category suitable for a caller's loading, recovery, or refusal presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// File access, synchronization, or publication failed.
    Io,
    /// Bytes, package identity, paths, or canonical data were invalid.
    InvalidData,
    /// A configured per-operation/residency budget would be exceeded.
    Limit,
    /// The operation requires terrain that is not resident.
    Unavailable,
    /// An expected revision, source identity, or idempotency key disagreed.
    Conflict,
    /// A canceled background operation did not publish anything.
    Cancelled,
    /// A pin protects a chunk against the requested operation.
    Pinned,
}

/// Typed error retaining a concrete diagnostic without panicking in engine code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeError {
    /// Stable error category.
    pub kind: ErrorKind,
    /// Exact operation context.
    pub message: String,
}

impl RuntimeError {
    pub(crate) fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub(crate) fn invalid(error: impl fmt::Display) -> Self {
        Self::new(ErrorKind::InvalidData, error.to_string())
    }

    pub(crate) fn io(error: impl fmt::Display) -> Self {
        Self::new(ErrorKind::Io, error.to_string())
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for RuntimeError {}

/// Runtime operation result.
pub type RuntimeResult<T> = Result<T, RuntimeError>;
