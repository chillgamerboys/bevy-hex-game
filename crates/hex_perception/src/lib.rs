//! Headless gameplay perception.
//!
//! This crate separates three deterministic simulation facts:
//!
//! - [`ResolvedIllumination`] describes objective light at exact surfaces.
//! - [`FactionObservations`] describes what each faction sees now.
//! - [`FactionMapKnowledge`] retains only what each faction is allowed to know.
//!
//! Rendering lights, shadows, fog meshes, and picking are consumers or presentation
//! inputs elsewhere. They never establish the facts stored here.

mod illumination;
mod knowledge;
mod runtime;
mod sight;
mod snapshots;

pub use illumination::{
    resolve_illumination_at, LightSourceSnapshot, ResolvedIllumination, ResolvedLight,
};
pub use knowledge::{apply_observations, FactionKnowledge, FactionMapKnowledge, KnownSurface};
pub use runtime::plugin;
pub use sight::{can_observe, resolve_observations, FactionObservation, FactionObservations};
pub use snapshots::{ObservedUnit, PerceptionError, SurfaceSnapshot, SurfaceSnapshots};
