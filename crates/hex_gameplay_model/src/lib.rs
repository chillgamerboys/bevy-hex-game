//! Renderer-free state and transition models for gameplay-owned screens.
//!
//! This crate owns decisions that can be tested without a Bevy [`App`]. The game
//! package adapts button interactions, persistence, loaded catalogs, and navigation
//! into these typed transitions; it does not reconstruct their results.

mod combat_lab;
mod creator;

pub use combat_lab::{
    resolve_lab_run, CombatLabEdit, CombatLabModel, CombatLabReportDeployment, CombatLabReportId,
    LabRunAction, LabRunFailure, LabRunTransition, LabTab, ReportMode, ReportViewModel,
    RosterChoice, SandboxRestore, SandboxStep, MAX_COMBAT_LAB_ROSTER,
};
pub use creator::{
    CreatorDestination, CreatorEntry, CreatorNavigation, CreatorSurface, EditHistory,
};
