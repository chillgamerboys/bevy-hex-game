//! Reusable application logic for the standalone Asset Workshop.

mod app;
/// Transactional undo and redo storage for editor documents.
pub mod history;
/// Editor launch argument parsing and project-root discovery.
pub mod launch;
/// Pure voxel-object editing state and commands.
pub mod model;
/// Project discovery, validation, and safe RON persistence.
pub mod project;
/// `bevy_egui` authoring panels and typed user intents.
pub mod ui;
/// Shared 3D authoring viewport.
pub mod viewport;
/// Session-wide drafts and global undo ordering.
pub mod workshop;

/// Runs the standalone Asset Workshop application.
pub use app::run;
