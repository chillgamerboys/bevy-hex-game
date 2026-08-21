//! Shared exact strata for dry V3 surface recipes.
//!
//! This module owns material columns only. Desert Transition, Desert Plain,
//! Dunes, and Oasis retain separate landform, feature, liquid, and validation
//! authority while sharing one bedrock/stone/topsoil contract.

use hex_core::Level;

use super::volume::{LevelInterval, SolidMass, SolidMaterialRole, VolumeColumn, VolumeElement};

/// Builds one fully supported dry column with an exact one-voxel surface cap.
///
/// Callers select `Grass`, `Dirt`, `Gravel`, or `Sand` according to their
/// semantic surface classification. Settings validation guarantees at least
/// five levels, so the saturating bounds are a final fail-closed guard rather
/// than a way to admit shallow strata. A Dirt cap is coalesced with the three
/// Dirt levels beneath it because semantic volumes reject adjacent identical
/// intervals.
pub(super) fn arid_column(surface: Level, cap: SolidMaterialRole) -> VolumeColumn {
    debug_assert!(matches!(
        cap,
        SolidMaterialRole::Grass
            | SolidMaterialRole::Dirt
            | SolidMaterialRole::Gravel
            | SolidMaterialRole::Sand
    ));
    let dirt_top = if cap == SolidMaterialRole::Dirt {
        surface.saturating_add(1)
    } else {
        surface
    };
    let mut elements = vec![
        VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(0, 1),
            material: SolidMaterialRole::Bedrock,
            cutaway_for: None,
        }),
        VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(1, surface.saturating_sub(3)),
            material: SolidMaterialRole::Stone,
            cutaway_for: None,
        }),
        VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(surface.saturating_sub(3), dirt_top),
            material: SolidMaterialRole::Dirt,
            cutaway_for: None,
        }),
    ];
    if cap != SolidMaterialRole::Dirt {
        elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(surface, surface.saturating_add(1)),
            material: cap,
            cutaway_for: None,
        }));
    }
    VolumeColumn { elements }
}

/// Convenience wrapper for recipes whose complete exposed surface is sand.
pub(super) fn sand_column(surface: Level) -> VolumeColumn {
    arid_column(surface, SolidMaterialRole::Sand)
}

/// Grass-capped shore strata used by the green ring around an Oasis pool.
pub(super) fn oasis_grass_column(surface: Level) -> VolumeColumn {
    arid_column(surface, SolidMaterialRole::Grass)
}
