//! Runtime publication for authored V3 cave crystals.
//!
//! Gameplay illumination remains a separate exact [`GameplayLight`](hex_core::GameplayLight)
//! rooted on the cave floor. This adapter publishes only the authored visual object
//! and a deliberately restrained physical point light.

use std::fmt;

use bevy::prelude::*;
use hex_assets::{
    HexObjectRotation, ObjectInstance, ObjectInstanceError, RuntimeArtCatalog, SrgbColor,
};

use crate::procedural_v3::{
    CaveCrystalAssetError, CaveCrystalKind, CaveCrystalObjectSet, LightId,
    MapPresentationProjection, PlannedLightPresentation,
};

const POINT_LIGHT_INTENSITY_LUMENS: f32 = 4_500.0;
const POINT_LIGHT_RANGE: f32 = 4.5;
const POINT_LIGHT_RADIUS: f32 = 0.12;

/// Private identity on the transform root of one generated crystal.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GeneratedCaveCrystal {
    pub(crate) id: LightId,
    pub(crate) kind: CaveCrystalKind,
}

/// Presentation-only physical light attached beneath one generated crystal root.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GeneratedCaveCrystalPointLight;

/// Fully validated publication data. Creating this type queues no ECS commands.
#[derive(Debug)]
pub(crate) struct PreparedCaveCrystal {
    id: LightId,
    kind: CaveCrystalKind,
    instance: ObjectInstance,
    point_light: PointLight,
    point_light_offset: Vec3,
}

/// Failure to publish one cave-crystal presentation.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CrystalPresentationError {
    Asset(CaveCrystalAssetError),
    InvalidRotation {
        id: LightId,
        source: ObjectInstanceError,
    },
    VisualOriginOverflow {
        id: LightId,
    },
    InvalidObjectInstance {
        id: LightId,
        source: ObjectInstanceError,
    },
}

impl fmt::Display for CrystalPresentationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Asset(error) => write!(formatter, "crystal asset preflight failed: {error}"),
            Self::InvalidRotation { id, source } => {
                write!(
                    formatter,
                    "crystal {id:?} has an invalid rotation: {source}"
                )
            }
            Self::VisualOriginOverflow { id } => {
                write!(
                    formatter,
                    "crystal {id:?} visual origin exceeds the level range"
                )
            }
            Self::InvalidObjectInstance { id, source } => {
                write!(
                    formatter,
                    "crystal {id:?} has an invalid object instance: {source}"
                )
            }
        }
    }
}

impl std::error::Error for CrystalPresentationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Asset(error) => Some(error),
            Self::InvalidRotation { source, .. } | Self::InvalidObjectInstance { source, .. } => {
                Some(source)
            }
            Self::VisualOriginOverflow { .. } => None,
        }
    }
}

/// Validates every crystal publication before any presentation entities are queued.
pub(crate) fn prepare_presentations(
    level_height: f32,
    projection: Option<&MapPresentationProjection>,
    catalog: Option<&RuntimeArtCatalog>,
) -> Result<Vec<PreparedCaveCrystal>, CrystalPresentationError> {
    let Some(projection) = projection else {
        return Ok(Vec::new());
    };
    let cave_crystal_count = projection
        .lights()
        .values()
        .filter(|light| {
            matches!(
                light.presentation,
                Some(PlannedLightPresentation::CaveCrystal(_))
            )
        })
        .count();
    if cave_crystal_count == 0 {
        return Ok(Vec::new());
    }
    let catalog = catalog
        .ok_or_else(|| CrystalPresentationError::Asset(CaveCrystalAssetError::missing_catalog()))?;
    let objects =
        CaveCrystalObjectSet::resolve(catalog).map_err(CrystalPresentationError::Asset)?;
    let color = to_color(objects.glow_color());
    let mut prepared = Vec::with_capacity(cave_crystal_count);

    for (id, light) in projection.lights() {
        let Some(PlannedLightPresentation::CaveCrystal(crystal)) = light.presentation else {
            continue;
        };
        let rotation = HexObjectRotation::new(crystal.rotation)
            .map_err(|source| CrystalPresentationError::InvalidRotation { id: *id, source })?;
        let visual_level = light
            .origin
            .level
            .checked_add(1)
            .ok_or(CrystalPresentationError::VisualOriginOverflow { id: *id })?;
        let instance = ObjectInstance::new(
            objects.object_id(crystal.kind).clone(),
            hex_core::TilePos::new(light.origin.coord, visual_level),
            level_height,
            rotation,
        )
        .map_err(|source| CrystalPresentationError::InvalidObjectInstance { id: *id, source })?;
        let vertical_offset =
            f32::from(crystal.kind.height_u8().saturating_sub(1)) * level_height * 0.5;
        prepared.push(PreparedCaveCrystal {
            id: *id,
            kind: crystal.kind,
            instance,
            point_light: PointLight {
                color,
                intensity: POINT_LIGHT_INTENSITY_LUMENS,
                range: POINT_LIGHT_RANGE,
                radius: POINT_LIGHT_RADIUS,
                shadow_maps_enabled: false,
                contact_shadows_enabled: false,
                ..default()
            },
            point_light_offset: Vec3::Y * vertical_offset,
        });
    }
    Ok(prepared)
}

/// Publishes already-validated crystals as direct children of `HexGrid`.
pub(crate) fn spawn_prepared(
    commands: &mut Commands,
    prepared: Vec<PreparedCaveCrystal>,
) -> Vec<Entity> {
    let mut roots = Vec::with_capacity(prepared.len());
    for crystal in prepared {
        let point_light = commands
            .spawn((
                crystal.point_light,
                Transform::from_translation(crystal.point_light_offset),
                GeneratedCaveCrystalPointLight,
                Name::new("GeneratedCaveCrystalPointLight"),
            ))
            .id();
        let root = commands
            .spawn((
                crystal.instance,
                GeneratedCaveCrystal {
                    id: crystal.id,
                    kind: crystal.kind,
                },
                Transform::default(),
                Visibility::Inherited,
                Name::new("GeneratedCaveCrystal"),
            ))
            .add_child(point_light)
            .id();
        roots.push(root);
    }
    roots
}

fn to_color(color: SrgbColor) -> Color {
    Color::srgb(color.red(), color.green(), color.blue())
}
