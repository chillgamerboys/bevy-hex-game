//! Renderer-free state and transition models for gameplay-owned screens.
//!
//! This crate owns decisions that can be tested without a Bevy `App`. The game
//! package adapts button interactions, persistence, loaded catalogs, and navigation
//! into these typed transitions; it does not reconstruct their results.

mod creator;
mod main_menu;
mod sandbox;

pub use creator::{
    CreatorDestination, CreatorEntry, CreatorNavigation, CreatorOrigin, CreatorSurface, EditHistory,
};
pub use main_menu::{CampaignSlotId, MainMenuModel, MainMenuRoute};
pub use sandbox::{
    SandboxBackResult, SandboxCharacter, SandboxDestination, SandboxDraft, SandboxEntryOrigin,
    SandboxMapSelection, SandboxModel, SandboxRoster, SandboxRoute, SandboxSide, SandboxSlotIndex,
    SandboxStartBlocker, SANDBOX_ROSTER_SIZE,
};
