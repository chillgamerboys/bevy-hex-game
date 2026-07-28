//! Reusable application logic for the standalone Asset Workshop.

mod app;
/// Transactional undo and redo storage for editor documents.
pub mod history;
/// Pure voxel-object editing state and commands.
pub mod model;
/// Project discovery, validation, and safe RON persistence.
pub mod project;

/// Runs the standalone Asset Workshop application.
pub use app::run;
