//! Runtime publication for authored V3 surface features.
//!
//! The procedural plan owns exact placement, rotation, blockers, and stable object
//! identity. This adapter publishes renderer-neutral [`ObjectInstance`] components;
//! `hex_objects` owns mesh baking and materials.

use std::fmt;

use bevy::prelude::*;
use hex_assets::{ObjectInstance, ObjectInstanceError};
use hex_core::CanopyOccluder;

use crate::procedural_v3::{FeatureId, FeatureKind, MapPresentationProjection};

/// Private identity on the transform root of one generated feature.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GeneratedFeatureRoot {
    pub(crate) id: FeatureId,
    pub(crate) kind: FeatureKind,
}

/// Failure to publish one validated authored-object placement.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FeaturePresentationError {
    InvalidObjectInstance {
        id: FeatureId,
        source: ObjectInstanceError,
    },
    VisualOriginOverflow {
        id: FeatureId,
    },
}

impl fmt::Display for FeaturePresentationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidObjectInstance { id, source } => {
                write!(
                    formatter,
                    "feature {id:?} has an invalid object instance: {source}"
                )
            }
            Self::VisualOriginOverflow { id } => {
                write!(
                    formatter,
                    "feature {id:?} visual origin exceeds the level range"
                )
            }
        }
    }
}

impl std::error::Error for FeaturePresentationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidObjectInstance { source, .. } => Some(source),
            Self::VisualOriginOverflow { .. } => None,
        }
    }
}

/// Publishes generated authored-object roots as direct children of `HexGrid`.
///
/// A feature root identifies its exact solid footing. The blueprint's level-zero
/// voxel belongs one level above that surface, so the runtime object origin is
/// shifted upward exactly once.
pub(crate) fn spawn_presentations(
    commands: &mut Commands,
    level_height: f32,
    projection: Option<&MapPresentationProjection>,
) -> Result<Vec<Entity>, FeaturePresentationError> {
    let Some(projection) = projection else {
        return Ok(Vec::new());
    };

    let mut publications = Vec::with_capacity(projection.features().len());
    for (id, feature) in projection.features() {
        let visual_level = feature
            .root
            .level
            .checked_add(1)
            .ok_or(FeaturePresentationError::VisualOriginOverflow { id: *id })?;
        let instance = ObjectInstance::new(
            feature.object_id.clone(),
            hex_core::TilePos::new(feature.root.coord, visual_level),
            level_height,
            feature.rotation,
        )
        .map_err(|source| FeaturePresentationError::InvalidObjectInstance { id: *id, source })?;
        publications.push((*id, feature, instance));
    }

    let mut roots = Vec::with_capacity(publications.len());
    for (id, feature, instance) in publications {
        let mut root = commands.spawn((
            instance,
            GeneratedFeatureRoot {
                id,
                kind: feature.kind,
            },
            Name::new(match feature.kind {
                FeatureKind::Tree => "GeneratedTree",
                FeatureKind::TallGrass => "GeneratedTallGrass",
            }),
        ));
        if feature.kind == FeatureKind::Tree {
            root.insert(CanopyOccluder(feature.root));
        }
        roots.push(root.id());
    }
    Ok(roots)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use bevy::ecs::world::CommandQueue;
    use hex_assets::{HexObjectRotation, ObjectAssetId};
    use hex_core::{HexCoord, TilePos};

    use super::*;
    use crate::procedural_v3::PlannedFeature;

    fn object_id(value: &str) -> ObjectAssetId {
        ObjectAssetId::new(value).expect("fixture object id should be valid")
    }

    fn feature(kind: FeatureKind) -> PlannedFeature {
        let root = TilePos::new(HexCoord::from_axial(2, -1), 15);
        PlannedFeature {
            root,
            kind,
            object_id: object_id(match kind {
                FeatureKind::Tree => "plant/small-broadleaf",
                FeatureKind::TallGrass => "prop/grass-tuft",
            }),
            rotation: HexObjectRotation::new(4).expect("fixture rotation should be valid"),
            blocker_footprint: match kind {
                FeatureKind::Tree => BTreeSet::from([root]),
                FeatureKind::TallGrass => BTreeSet::new(),
            },
        }
    }

    #[test]
    fn publisher_preserves_object_identity_rotation_and_exact_visual_origin() {
        let planned = feature(FeatureKind::Tree);
        let projection =
            MapPresentationProjection::with_test_features([(FeatureId(7), planned.clone())]);
        let mut world = World::new();
        let mut queue = CommandQueue::default();
        let roots = {
            let mut commands = Commands::new(&mut queue, &world);
            spawn_presentations(&mut commands, 0.4, Some(&projection))
                .expect("valid feature should publish")
        };
        queue.apply(&mut world);

        assert_eq!(roots.len(), 1);
        let root = *roots.first().expect("publisher should return one root");
        let entity = world.entity(root);
        let instance = entity
            .get::<ObjectInstance>()
            .expect("publisher should attach a renderer-neutral instance");
        assert_eq!(instance.object_id(), &planned.object_id);
        assert_eq!(instance.rotation(), planned.rotation);
        assert_eq!(
            instance.origin(),
            TilePos::new(planned.root.coord, planned.root.level + 1)
        );
        assert!((instance.level_height() - 0.4).abs() < f32::EPSILON);
        assert_eq!(
            entity.get::<CanopyOccluder>(),
            Some(&CanopyOccluder(planned.root))
        );
        assert!(entity.get::<Transform>().is_none());
    }

    #[test]
    fn visual_only_grass_does_not_publish_canopy_metadata() {
        let projection = MapPresentationProjection::with_test_features([(
            FeatureId(2),
            feature(FeatureKind::TallGrass),
        )]);
        let mut world = World::new();
        let mut queue = CommandQueue::default();
        let roots = {
            let mut commands = Commands::new(&mut queue, &world);
            spawn_presentations(&mut commands, 0.4, Some(&projection))
                .expect("valid grass should publish")
        };
        queue.apply(&mut world);

        let root = *roots.first().expect("publisher should return one root");
        assert!(world.entity(root).get::<CanopyOccluder>().is_none());
    }

    #[test]
    fn invalid_level_height_rolls_back_before_commands_are_applied() {
        let projection = MapPresentationProjection::with_test_features(BTreeMap::from([(
            FeatureId(0),
            feature(FeatureKind::Tree),
        )]));
        let world = World::new();
        let mut queue = CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);
        assert!(matches!(
            spawn_presentations(&mut commands, f32::NAN, Some(&projection)),
            Err(FeaturePresentationError::InvalidObjectInstance {
                id: FeatureId(0),
                ..
            })
        ));
    }
}
