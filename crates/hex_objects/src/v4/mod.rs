//! Bounded stock-object art with exact global occupancy retained by the caller.
//!
//! This adapter reuses the existing object baker and authored styles, but installs
//! no legacy reconciliation, world snapshots, or screen lifecycle. Publication is
//! deliberately opt-in: the application must atomically suppress only the matching
//! object proxy faces and install authoritative object picking. Meshes start with
//! `Pickable::IGNORE`; [`ResidentObjectPart`] identifies them for that integration.
//! Unknown assets, mismatched footprints, and foreign-root occupancy without a
//! complete source record remain explicit proxies outside this adapter.

mod prepare;
mod publish;

pub use prepare::{ObjectPresentationLimits, PreparedObject};
pub use publish::{ObjectReceipt, ResidentObject, ResidentObjectPart, ResidentObjectPresenter};

/// A rejected product or placement; current presentation remains intact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectPresentationError(String);

impl std::fmt::Display for ObjectPresentationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ObjectPresentationError {}

impl From<hex_world_contracts::ContractError> for ObjectPresentationError {
    fn from(error: hex_world_contracts::ContractError) -> Self {
        Self(error.to_string())
    }
}

/// Install only authored-object transparency management, without legacy producers.
///
/// Optional until the application admits stock art atomically with proxy masks and
/// picking. This preserves authored Blend styles using the existing OIT lifecycle.
/// Do not also install the legacy object plugin in the same application.
pub fn transparency_plugin(app: &mut bevy::prelude::App) {
    use bevy::prelude::*;
    app.add_systems(
        PostUpdate,
        crate::manage_object_oit.in_set(hex_core::PresentationSystems::ApplyMaterials),
    );
}

#[cfg(test)]
mod tests;
