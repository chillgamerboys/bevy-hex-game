//! Data-authored, deterministic V4 region compiler and immutable package builder.
//!
//! Source parsing is independent from generation. A caller can load changed RON
//! bytes at runtime without relinking this library. Generation accepts only exact
//! integer contracts; Bevy publication belongs to the map adapter.

mod compiler;
mod geometry;
mod model;
mod operators;
mod volume;

pub use compiler::{
    compile_world, compile_world_cached, validate_source, CompileArtifacts, CompileDiagnostic,
    CompileDiagnostics, CompileReport, StageTiming,
};
pub use model::*;

/// Parses strict, runtime-loaded RON source and checks its authoring contract.
pub fn parse_world(source: &str) -> Result<WorldSpec, CompileDiagnostics> {
    let world: WorldSpec = ron::from_str(source)
        .map_err(|error| CompileDiagnostics::one("source", "parse", error.to_string()))?;
    validate_world(&world)?;
    Ok(world)
}

/// Validates authored identities, geometry, materials and declared boundaries.
pub fn validate_world(source: &WorldSpec) -> Result<(), CompileDiagnostics> {
    compiler::validate_source(source)
}

#[cfg(test)]
mod tests;
