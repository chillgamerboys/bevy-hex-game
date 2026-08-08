//! Runtime publication for authored V3 cave and Crystal Ascent crystals.
//!
//! Gameplay illumination remains a separate exact [`GameplayLight`](hex_core::GameplayLight)
//! rooted on the authored floor. This adapter publishes only the authored visual
//! object, exact opt-in occupancy for the cathedral heart, and deliberately restrained
//! physical point lights.

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CrystalPresentationRequest {
    id: LightId,
    origin: hex_core::TilePos,
    kind: PreparedCrystalKind,
    rotation_steps: u8,
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

/// Failure to publish one authored crystal presentation.
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
    PreflightInvariant {
        id: LightId,
        asset_set: &'static str,
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
            Self::PreflightInvariant { id, asset_set } => {
                write!(
                    formatter,
                    "crystal {id:?} lost its preflighted {asset_set} asset set"
                )
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
            Self::VisualOriginOverflow { .. }
            | Self::OccupancyProjectionOverflow { .. }
            | Self::PreflightInvariant { .. } => None,
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
    let requests = projection
        .lights()
        .iter()
        .filter_map(|(id, light)| match light.presentation {
            Some(PlannedLightPresentation::CaveCrystal(crystal)) => {
                Some(CrystalPresentationRequest {
                    id: *id,
                    origin: light.origin,
                    kind: PreparedCrystalKind::Cave(crystal.kind),
                    rotation_steps: crystal.rotation,
                })
            }
            Some(PlannedLightPresentation::CrystalAscent(crystal)) => {
                Some(CrystalPresentationRequest {
                    id: *id,
                    origin: light.origin,
                    kind: PreparedCrystalKind::Ascent(crystal.kind),
                    rotation_steps: crystal.rotation,
                })
            }
            None => None,
        })
        .collect::<Vec<_>>();
    prepare_requests(level_height, &requests, catalog)
}

fn prepare_requests(
    level_height: f32,
    requests: &[CrystalPresentationRequest],
    catalog: Option<&RuntimeArtCatalog>,
) -> Result<Vec<PreparedCrystal>, CrystalPresentationError> {
    let cave_crystal_count = requests
        .iter()
        .filter(|request| matches!(request.kind, PreparedCrystalKind::Cave(_)))
        .count();
    let ascent_crystal_count = requests
        .iter()
        .filter(|request| matches!(request.kind, PreparedCrystalKind::Ascent(_)))
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

    for request in requests {
        let (object_id, color, point_light_offsets) = match request.kind {
            PreparedCrystalKind::Cave(kind) => {
                let objects =
                    cave_objects
                        .as_ref()
                        .ok_or(CrystalPresentationError::PreflightInvariant {
                            id: request.id,
                            asset_set: "cave crystal",
                        })?;
                // `hex_objects` scales the complete object root by `level_height`.
                // Child translations are therefore authored in local voxel levels;
                // multiplying here would apply the vertical scale twice.
                let offset = f32::from(kind.height_u8().saturating_sub(1)) * 0.5;
                (
                    objects.object_id(kind).clone(),
                    to_color(objects.glow_color()),
                    vec![Vec3::Y * offset],
                )
            }
            PreparedCrystalKind::Ascent(kind) => {
                let objects = ascent_objects.as_ref().ok_or(
                    CrystalPresentationError::PreflightInvariant {
                        id: request.id,
                        asset_set: "Crystal Ascent",
                    },
                )?;
                let (object_id, offsets) = match kind {
                    CrystalAscentCrystalKind::Landing(kind) => (
                        objects.landing_id(kind).clone(),
                        vec![Vec3::Y * (f32::from(kind.height_u8().saturating_sub(1)) * 0.5)],
                    ),
                    CrystalAscentCrystalKind::Heart => (
                        objects.heart_id().clone(),
                        [2.0_f32, 8.0, 14.0, 20.0]
                            .into_iter()
                            .map(|level| Vec3::Y * level)
                            .collect(),
                    ),
                };
                (object_id, to_color(objects.glow_color()), offsets)
            }
        };
        let rotation = HexObjectRotation::new(request.rotation_steps).map_err(|source| {
            CrystalPresentationError::InvalidRotation {
                id: request.id,
                source,
            }
        })?;
        let visual_level = request
            .origin
            .level
            .checked_add(1)
            .ok_or(CrystalPresentationError::VisualOriginOverflow { id: request.id })?;
        let instance = ObjectInstance::new(
            object_id,
            hex_core::TilePos::new(request.origin.coord, visual_level),
            level_height,
            rotation,
        )
        .map_err(|source| CrystalPresentationError::InvalidObjectInstance {
            id: request.id,
            source,
        })?;
        let occupancy = match request.kind {
            PreparedCrystalKind::Ascent(CrystalAscentCrystalKind::Heart) => {
                let objects = ascent_objects.as_ref().ok_or(
                    CrystalPresentationError::PreflightInvariant {
                        id: request.id,
                        asset_set: "Crystal Ascent",
                    },
                )?;
                Some(
                    objects
                        .project_heart_runs(instance.origin(), rotation)
                        .ok_or(CrystalPresentationError::OccupancyProjectionOverflow {
                            id: request.id,
                        })?,
                )
            }
            PreparedCrystalKind::Cave(_)
            | PreparedCrystalKind::Ascent(CrystalAscentCrystalKind::Landing(_)) => None,
        };
        prepared.push(PreparedCrystal {
            id: request.id,
            kind: request.kind,
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::OnceLock;

    use bevy::ecs::world::CommandQueue;
    use bevy::transform::TransformPlugin;
    use hex_assets::{ArtPalette, ObjectBlueprint, ObjectCatalogFile, VoxelStyleCatalog};
    use hex_core::{AuthoredObjectVoxelRuns, HexCoord, TilePos};

    use super::*;

    const LEVEL_HEIGHT: f32 = 0.4;
    const TOLERANCE: f32 = 0.000_01;

    fn runtime_art_catalog() -> &'static RuntimeArtCatalog {
        static CATALOG: OnceLock<RuntimeArtCatalog> = OnceLock::new();
        CATALOG.get_or_init(|| {
            let palette: ArtPalette = ron::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/art/palette.ron"
            )))
            .expect("tracked art palette should parse");
            let styles: VoxelStyleCatalog = ron::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/art/voxel_styles.ron"
            )))
            .expect("tracked voxel styles should parse");
            let mut objects = BTreeMap::new();
            for source in [
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/prop/crystal-low-cluster.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/prop/crystal-branched.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/prop/crystal-spire.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/prop/crystal-cathedral-heart.ron"
                )),
            ] {
                let blueprint: ObjectBlueprint =
                    ron::from_str(source).expect("tracked crystal blueprint should parse");
                objects.insert(blueprint.id.clone(), blueprint);
            }
            let manifest = ObjectCatalogFile::new(objects.keys().cloned())
                .expect("crystal fixture ids should form a valid manifest");
            RuntimeArtCatalog::from_sources(&palette, &styles, &manifest, objects)
                .expect("crystal runtime art graph should resolve")
        })
    }

    fn request(
        id: u32,
        coord: HexCoord,
        level: i32,
        kind: PreparedCrystalKind,
        rotation_steps: u8,
    ) -> CrystalPresentationRequest {
        CrystalPresentationRequest {
            id: LightId(id),
            origin: TilePos::new(coord, level),
            kind,
            rotation_steps,
        }
    }

    fn prepared_by_id(prepared: &[PreparedCrystal], id: LightId) -> &PreparedCrystal {
        prepared
            .iter()
            .find(|crystal| crystal.id == id)
            .expect("prepared fixture should retain its light identity")
    }

    fn assert_near(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= TOLERANCE,
            "expected {expected}, got {actual}"
        );
    }

    fn assert_local_y_offsets(actual: &[Vec3], expected: &[f32]) {
        assert_eq!(actual.len(), expected.len());
        for (offset, expected_y) in actual.iter().zip(expected) {
            assert_near(offset.x, 0.0);
            assert_near(offset.y, *expected_y);
            assert_near(offset.z, 0.0);
        }
    }

    fn spawn_with_level_scale(prepared: Vec<PreparedCrystal>) -> (App, Vec<Entity>) {
        let mut app = App::new();
        app.add_plugins(TransformPlugin);
        let mut queue = CommandQueue::default();
        let roots = {
            let mut commands = Commands::new(&mut queue, app.world());
            spawn_prepared(&mut commands, prepared)
        };
        queue.apply(app.world_mut());

        for root in &roots {
            app.world_mut().entity_mut(*root).insert(
                Transform::from_translation(Vec3::new(7.0, 11.0, -3.0)).with_scale(Vec3::new(
                    1.0,
                    LEVEL_HEIGHT,
                    1.0,
                )),
            );
        }
        app.update();
        (app, roots)
    }

    fn ascent_root(world: &World, roots: &[Entity], id: LightId) -> Entity {
        roots
            .iter()
            .copied()
            .find(|root| {
                world
                    .entity(*root)
                    .get::<GeneratedCrystalAscent>()
                    .is_some_and(|marker| marker.id == id)
            })
            .expect("spawned ascent fixture should retain its light identity")
    }

    fn cave_root(world: &World, roots: &[Entity], id: LightId) -> Entity {
        roots
            .iter()
            .copied()
            .find(|root| {
                world
                    .entity(*root)
                    .get::<GeneratedCaveCrystal>()
                    .is_some_and(|marker| marker.id == id)
            })
            .expect("spawned cave fixture should retain its light identity")
    }

    fn child_entities(world: &World, root: Entity) -> Vec<Entity> {
        world
            .entity(root)
            .get::<Children>()
            .expect("crystal root should own its physical light children")
            .iter()
            .collect()
    }

    fn global_y_offsets(world: &World, root: Entity, children: &[Entity]) -> Vec<f32> {
        let root_y = world
            .entity(root)
            .get::<GlobalTransform>()
            .expect("crystal root should have a propagated transform")
            .translation()
            .y;
        let mut offsets = children
            .iter()
            .map(|child| {
                world
                    .entity(*child)
                    .get::<GlobalTransform>()
                    .expect("point-light child should have a propagated transform")
                    .translation()
                    .y
                    - root_y
            })
            .collect::<Vec<_>>();
        offsets.sort_by(f32::total_cmp);
        offsets
    }

    fn assert_physical_light(light: &PointLight) {
        assert_near(light.intensity, POINT_LIGHT_INTENSITY_LUMENS);
        assert_near(light.range, POINT_LIGHT_RANGE);
        assert_near(light.radius, POINT_LIGHT_RADIUS);
        assert!(!light.shadow_maps_enabled);
        assert!(!light.contact_shadows_enabled);
    }

    #[test]
    fn ascent_preparation_keeps_fixture_identity_offsets_and_heart_only_occupancy_exact() {
        let landing_id = LightId(17);
        let heart_id = LightId(23);
        let landing_origin = TilePos::new(HexCoord::from_axial(-3, 2), 7);
        let heart_origin = TilePos::new(HexCoord::from_axial(8, -5), 10);
        let requests = [
            CrystalPresentationRequest {
                id: landing_id,
                origin: landing_origin,
                kind: PreparedCrystalKind::Ascent(CrystalAscentCrystalKind::Landing(
                    CaveCrystalKind::Spire,
                )),
                rotation_steps: 4,
            },
            CrystalPresentationRequest {
                id: heart_id,
                origin: heart_origin,
                kind: PreparedCrystalKind::Ascent(CrystalAscentCrystalKind::Heart),
                rotation_steps: 2,
            },
        ];
        let prepared = prepare_requests(LEVEL_HEIGHT, &requests, Some(runtime_art_catalog()))
            .expect("tracked ascent fixtures should preflight");
        let objects = CrystalAscentObjectSet::resolve(runtime_art_catalog())
            .expect("tracked ascent fixture assets should resolve");

        let landing = prepared_by_id(&prepared, landing_id);
        assert_eq!(
            landing.instance.object_id(),
            objects.landing_id(CaveCrystalKind::Spire)
        );
        assert_eq!(
            landing.instance.origin(),
            TilePos::new(landing_origin.coord, landing_origin.level + 1)
        );
        assert_eq!(
            landing.instance.rotation(),
            HexObjectRotation::new(4).expect("fixture rotation should be valid")
        );
        assert_near(landing.instance.level_height(), LEVEL_HEIGHT);
        assert_local_y_offsets(&landing.point_light_offsets, &[1.5]);
        assert!(
            landing.occupancy.is_none(),
            "small landing crystals must remain visual-only and nonblocking"
        );

        let heart = prepared_by_id(&prepared, heart_id);
        assert_eq!(heart.instance.object_id(), objects.heart_id());
        assert_eq!(
            heart.instance.origin(),
            TilePos::new(heart_origin.coord, heart_origin.level + 1)
        );
        assert_eq!(
            heart.instance.rotation(),
            HexObjectRotation::new(2).expect("fixture rotation should be valid")
        );
        assert_local_y_offsets(&heart.point_light_offsets, &[2.0, 8.0, 14.0, 20.0]);
        let expected_occupancy = objects
            .project_heart_runs(heart.instance.origin(), heart.instance.rotation())
            .expect("fixture heart occupancy should project");
        assert!(!expected_occupancy.is_empty());
        assert_eq!(heart.occupancy.as_ref(), Some(&expected_occupancy));
    }

    #[test]
    fn ascent_lights_spawn_once_or_four_times_with_voxel_local_scaled_offsets() {
        let landing_id = LightId(5);
        let heart_id = LightId(9);
        let requests = [
            request(
                landing_id.0,
                HexCoord::ORIGIN,
                3,
                PreparedCrystalKind::Ascent(CrystalAscentCrystalKind::Landing(
                    CaveCrystalKind::Spire,
                )),
                0,
            ),
            request(
                heart_id.0,
                HexCoord::from_axial(5, -2),
                12,
                PreparedCrystalKind::Ascent(CrystalAscentCrystalKind::Heart),
                1,
            ),
        ];
        let prepared = prepare_requests(LEVEL_HEIGHT, &requests, Some(runtime_art_catalog()))
            .expect("tracked ascent fixtures should preflight");
        let (app, roots) = spawn_with_level_scale(prepared);

        let landing_root = ascent_root(app.world(), &roots, landing_id);
        assert!(
            app.world()
                .entity(landing_root)
                .get::<AuthoredObjectVoxelRuns>()
                .is_none(),
            "small landing crystals must not publish gameplay occupancy"
        );
        let landing_children = child_entities(app.world(), landing_root);
        assert_eq!(landing_children.len(), 1);
        let landing_offsets = global_y_offsets(app.world(), landing_root, &landing_children);
        let landing_offset = landing_offsets
            .first()
            .copied()
            .expect("landing crystal should have one physical light offset");
        assert_near(landing_offset, 0.6);

        let heart_root = ascent_root(app.world(), &roots, heart_id);
        assert!(
            app.world()
                .entity(heart_root)
                .get::<AuthoredObjectVoxelRuns>()
                .is_some(),
            "the cathedral heart must publish its exact gameplay occupancy"
        );
        let heart_children = child_entities(app.world(), heart_root);
        assert_eq!(heart_children.len(), 4);
        let heart_offsets = global_y_offsets(app.world(), heart_root, &heart_children);
        for (actual, expected) in heart_offsets.iter().zip([0.8, 3.2, 5.6, 8.0]) {
            assert_near(*actual, expected);
        }

        for child in landing_children.iter().chain(&heart_children) {
            let entity = app.world().entity(*child);
            assert!(entity.get::<GeneratedCrystalAscentPointLight>().is_some());
            assert!(entity.get::<GeneratedCaveCrystalPointLight>().is_none());
            assert_eq!(
                entity.get::<Name>().map(Name::as_str),
                Some("GeneratedCrystalAscentPointLight")
            );
            assert_physical_light(
                entity
                    .get::<PointLight>()
                    .expect("ascent child should carry a physical point light"),
            );
        }
    }

    #[test]
    fn cave_crystal_publication_keeps_one_light_and_remains_nonblocking() {
        let cave_id = LightId(31);
        let requests = [request(
            cave_id.0,
            HexCoord::from_axial(-4, 7),
            -2,
            PreparedCrystalKind::Cave(CaveCrystalKind::Branched),
            5,
        )];
        let prepared = prepare_requests(LEVEL_HEIGHT, &requests, Some(runtime_art_catalog()))
            .expect("tracked cave fixture should preflight");
        let cave = prepared_by_id(&prepared, cave_id);
        assert_local_y_offsets(&cave.point_light_offsets, &[1.0]);
        assert!(cave.occupancy.is_none());

        let (app, roots) = spawn_with_level_scale(prepared);
        let root = cave_root(app.world(), &roots, cave_id);
        let root_entity = app.world().entity(root);
        assert!(root_entity.get::<GeneratedCrystalAscent>().is_none());
        assert!(root_entity.get::<AuthoredObjectVoxelRuns>().is_none());
        assert_eq!(
            root_entity.get::<Name>().map(Name::as_str),
            Some("GeneratedCaveCrystal")
        );

        let children = child_entities(app.world(), root);
        assert_eq!(children.len(), 1);
        let offsets = global_y_offsets(app.world(), root, &children);
        let offset = offsets
            .first()
            .copied()
            .expect("cave crystal should have one physical light offset");
        assert_near(offset, 0.4);
        let child = app.world().entity(
            *children
                .first()
                .expect("cave crystal should have one physical light child"),
        );
        assert!(child.get::<GeneratedCaveCrystalPointLight>().is_some());
        assert!(child.get::<GeneratedCrystalAscentPointLight>().is_none());
        assert_eq!(
            child.get::<Name>().map(Name::as_str),
            Some("GeneratedCaveCrystalPointLight")
        );
        assert_physical_light(
            child
                .get::<PointLight>()
                .expect("cave child should carry a physical point light"),
        );
    }

    #[test]
    fn later_preflight_failure_queues_no_partial_crystal_entities() {
        let requests = [
            request(
                41,
                HexCoord::ORIGIN,
                2,
                PreparedCrystalKind::Ascent(CrystalAscentCrystalKind::Landing(
                    CaveCrystalKind::LowCluster,
                )),
                0,
            ),
            request(
                42,
                HexCoord::from_axial(1, -1),
                2,
                PreparedCrystalKind::Ascent(CrystalAscentCrystalKind::Heart),
                6,
            ),
        ];
        let mut world = World::new();
        let mut queue = CommandQueue::default();
        let baseline_entities = world.iter_entities().count();
        let result = prepare_requests(LEVEL_HEIGHT, &requests, Some(runtime_art_catalog())).map(
            |prepared| {
                let mut commands = Commands::new(&mut queue, &world);
                spawn_prepared(&mut commands, prepared)
            },
        );

        assert!(matches!(
            result,
            Err(CrystalPresentationError::InvalidRotation {
                id: LightId(42),
                ..
            })
        ));
        queue.apply(&mut world);
        assert_eq!(world.iter_entities().count(), baseline_entities);
    }
}
