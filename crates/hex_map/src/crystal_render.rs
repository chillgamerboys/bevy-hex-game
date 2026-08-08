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
    CaveCrystalAssetError, CaveCrystalKind, CaveCrystalObjectSet, CrystalAscentAssetError,
    CrystalAscentCrystalKind, CrystalAscentObjectSet, LightId, MapPresentationProjection,
    PlannedLightPresentation,
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

/// Private identity on one Crystal Ascent fixture root.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GeneratedCrystalAscent {
    pub(crate) id: LightId,
    pub(crate) kind: CrystalAscentCrystalKind,
}

/// Presentation-only physical light attached beneath a Crystal Ascent fixture.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GeneratedCrystalAscentPointLight;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreparedCrystalKind {
    Cave(CaveCrystalKind),
    Ascent(CrystalAscentCrystalKind),
}

/// Fully validated publication data. Creating this type queues no ECS commands.
#[derive(Debug)]
pub(crate) struct PreparedCrystal {
    id: LightId,
    kind: PreparedCrystalKind,
    instance: ObjectInstance,
    point_light_color: Color,
    point_light_offsets: Vec<Vec3>,
    occupancy: Option<hex_core::AuthoredObjectVoxelRuns>,
}

/// Failure to publish one cave-crystal presentation.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CrystalPresentationError {
    Asset(CaveCrystalAssetError),
    AscentAsset(CrystalAscentAssetError),
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
    OccupancyProjectionOverflow {
        id: LightId,
    },
}

impl fmt::Display for CrystalPresentationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Asset(error) => write!(formatter, "crystal asset preflight failed: {error}"),
            Self::AscentAsset(error) => {
                write!(formatter, "Crystal Ascent asset preflight failed: {error}")
            }
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
            Self::OccupancyProjectionOverflow { id } => {
                write!(formatter, "crystal {id:?} occupancy projection overflowed")
            }
        }
    }
}

impl std::error::Error for CrystalPresentationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Asset(error) => Some(error),
            Self::AscentAsset(error) => Some(error),
            Self::InvalidRotation { source, .. } | Self::InvalidObjectInstance { source, .. } => {
                Some(source)
            }
            Self::VisualOriginOverflow { .. } | Self::OccupancyProjectionOverflow { .. } => None,
        }
    }
}

/// Validates every crystal publication before any presentation entities are queued.
pub(crate) fn prepare_presentations(
    level_height: f32,
    projection: Option<&MapPresentationProjection>,
    catalog: Option<&RuntimeArtCatalog>,
) -> Result<Vec<PreparedCrystal>, CrystalPresentationError> {
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
    let ascent_crystal_count = projection
        .lights()
        .values()
        .filter(|light| {
            matches!(
                light.presentation,
                Some(PlannedLightPresentation::CrystalAscent(_))
            )
        })
        .count();
    if cave_crystal_count == 0 && ascent_crystal_count == 0 {
        return Ok(Vec::new());
    }
    let catalog = catalog.ok_or_else(|| {
        if ascent_crystal_count > 0 {
            CrystalPresentationError::AscentAsset(CrystalAscentAssetError::missing_catalog())
        } else {
            CrystalPresentationError::Asset(CaveCrystalAssetError::missing_catalog())
        }
    })?;
    let cave_objects = (cave_crystal_count > 0)
        .then(|| CaveCrystalObjectSet::resolve(catalog))
        .transpose()
        .map_err(CrystalPresentationError::Asset)?;
    let ascent_objects = (ascent_crystal_count > 0)
        .then(|| CrystalAscentObjectSet::resolve(catalog))
        .transpose()
        .map_err(CrystalPresentationError::AscentAsset)?;
    let mut prepared = Vec::with_capacity(cave_crystal_count + ascent_crystal_count);

    for (id, light) in projection.lights() {
        let Some(presentation) = light.presentation else {
            continue;
        };
        let (kind, rotation_steps, object_id, color, point_light_offsets) = match presentation {
            PlannedLightPresentation::CaveCrystal(crystal) => {
                let objects = cave_objects
                    .as_ref()
                    .expect("cave assets are preflighted when a cave crystal exists");
                let offset =
                    f32::from(crystal.kind.height_u8().saturating_sub(1)) * level_height * 0.5;
                (
                    PreparedCrystalKind::Cave(crystal.kind),
                    crystal.rotation,
                    objects.object_id(crystal.kind).clone(),
                    to_color(objects.glow_color()),
                    vec![Vec3::Y * offset],
                )
            }
            PlannedLightPresentation::CrystalAscent(crystal) => {
                let objects = ascent_objects
                    .as_ref()
                    .expect("ascent assets are preflighted when an ascent crystal exists");
                let (object_id, offsets) = match crystal.kind {
                    CrystalAscentCrystalKind::Landing(kind) => (
                        objects.landing_id(kind).clone(),
                        vec![
                            Vec3::Y
                                * (f32::from(kind.height_u8().saturating_sub(1))
                                    * level_height
                                    * 0.5),
                        ],
                    ),
                    CrystalAscentCrystalKind::Heart => (
                        objects.heart_id().clone(),
                        [2.0_f32, 8.0, 14.0, 20.0]
                            .into_iter()
                            .map(|level| Vec3::Y * (level * level_height))
                            .collect(),
                    ),
                };
                (
                    PreparedCrystalKind::Ascent(crystal.kind),
                    crystal.rotation,
                    object_id,
                    to_color(objects.glow_color()),
                    offsets,
                )
            }
        };
        let rotation = HexObjectRotation::new(rotation_steps)
            .map_err(|source| CrystalPresentationError::InvalidRotation { id: *id, source })?;
        let visual_level = light
            .origin
            .level
            .checked_add(1)
            .ok_or(CrystalPresentationError::VisualOriginOverflow { id: *id })?;
        let instance = ObjectInstance::new(
            object_id,
            hex_core::TilePos::new(light.origin.coord, visual_level),
            level_height,
            rotation,
        )
        .map_err(|source| CrystalPresentationError::InvalidObjectInstance { id: *id, source })?;
        let occupancy = match kind {
            PreparedCrystalKind::Ascent(CrystalAscentCrystalKind::Heart) => {
                let objects = ascent_objects
                    .as_ref()
                    .expect("ascent assets are preflighted when the heart exists");
                Some(
                    objects
                        .project_heart_runs(instance.origin(), rotation)
                        .ok_or(CrystalPresentationError::OccupancyProjectionOverflow { id: *id })?,
                )
            }
            PreparedCrystalKind::Cave(_)
            | PreparedCrystalKind::Ascent(CrystalAscentCrystalKind::Landing(_)) => None,
        };
        prepared.push(PreparedCrystal {
            id: *id,
            kind,
            instance,
            point_light_color: color,
            point_light_offsets,
            occupancy,
        });
    }
    Ok(prepared)
}

/// Publishes already-validated crystals as direct children of `HexGrid`.
pub(crate) fn spawn_prepared(
    commands: &mut Commands,
    prepared: Vec<PreparedCrystal>,
) -> Vec<Entity> {
    let mut roots = Vec::with_capacity(prepared.len());
    for crystal in prepared {
        let point_lights = crystal
            .point_light_offsets
            .into_iter()
            .map(|offset| {
                let point_light = PointLight {
                    color: crystal.point_light_color,
                    intensity: POINT_LIGHT_INTENSITY_LUMENS,
                    range: POINT_LIGHT_RANGE,
                    radius: POINT_LIGHT_RADIUS,
                    shadow_maps_enabled: false,
                    contact_shadows_enabled: false,
                    ..default()
                };
                let mut entity = commands.spawn((point_light, Transform::from_translation(offset)));
                match crystal.kind {
                    PreparedCrystalKind::Cave(_) => {
                        entity.insert((
                            GeneratedCaveCrystalPointLight,
                            Name::new("GeneratedCaveCrystalPointLight"),
                        ));
                    }
                    PreparedCrystalKind::Ascent(_) => {
                        entity.insert((
                            GeneratedCrystalAscentPointLight,
                            Name::new("GeneratedCrystalAscentPointLight"),
                        ));
                    }
                }
                entity.id()
            })
            .collect::<Vec<_>>();
        let mut root = commands.spawn((
            crystal.instance,
            Transform::default(),
            Visibility::Inherited,
        ));
        if let Some(occupancy) = crystal.occupancy {
            root.insert(occupancy);
        }
        match crystal.kind {
            PreparedCrystalKind::Cave(kind) => {
                root.insert((
                    GeneratedCaveCrystal {
                        id: crystal.id,
                        kind,
                    },
                    Name::new("GeneratedCaveCrystal"),
                ));
            }
            PreparedCrystalKind::Ascent(kind) => {
                root.insert((
                    GeneratedCrystalAscent {
                        id: crystal.id,
                        kind,
                    },
                    Name::new("GeneratedCrystalAscent"),
                ));
            }
        }
        let root = root.add_children(&point_lights).id();
        roots.push(root);
    }
    roots
}

fn to_color(color: SrgbColor) -> Color {
    Color::srgb(color.red(), color.green(), color.blue())
}
