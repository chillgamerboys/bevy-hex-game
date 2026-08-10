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
//!
//! The crate imports only Bevy's application, ECS, state, logging, and reflection
//! subcrates directly. `hex_assets` and `hex_units` still depend transitively on the
//! Bevy facade for their own runtime behavior, so this boundary promises
//! renderer-independent perception rules, not a renderer-free dependency graph.

mod illumination;
mod knowledge;
mod runtime;
mod sight;
mod snapshots;

pub use illumination::{
    resolve_illumination_at, LightSourceSnapshot, ResolvedIllumination, ResolvedLight,
};
pub use knowledge::{apply_observations, FactionKnowledge, FactionMapKnowledge, KnownSurface};
pub use runtime::{plugin, PerceptionRuntimeStats};
pub use sight::{
    can_observe, can_observe_with_authored_objects, resolve_observations,
    resolve_observations_with_authored_objects, FactionObservation, FactionObservations,
};
pub use snapshots::{ObservedUnit, PerceptionError, SurfaceSnapshot, SurfaceSnapshots};
