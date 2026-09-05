//! Exploration-only geometry from public terrain and authored object facts.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use hex_assets::{ObjectInstance, RuntimeArtCatalog, SubstanceTable};
use hex_core::{HexCoord, HexSpan, HexTile, SubstanceId, TilePos};

pub(super) const SKIN: f32 = 0.0001;
const FACE: f32 = 0.866_025_4;
const NORMALS: [Vec3; 4] = [
    Vec3::X,
    Vec3::new(0.5, 0.0, FACE),
    Vec3::new(-0.5, 0.0, FACE),
    Vec3::Y,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Material {
    Solid,
    Liquid,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Span {
    pub coord: HexCoord,
    pub bottom: f32,
    pub top: f32,
    pub material: Material,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Hit {
    pub fraction: f32,
    pub normal: Vec3,
}

#[derive(Resource, Default, Debug)]
pub(super) struct CollisionWorld {
    columns: BTreeMap<HexCoord, BTreeMap<Entity, Vec<Span>>>,
    entities: BTreeMap<Entity, BTreeSet<HexCoord>>,
    errors: BTreeMap<Entity, String>,
    pub initialized: bool,
    pub error: Option<String>,
    pub floor: f32,
    pub revision: u64,
}

impl CollisionWorld {
    pub fn replace(&mut self, entity: Entity, spans: Vec<Span>) {
        self.remove(entity);
        for span in spans {
            self.entities.entry(entity).or_default().insert(span.coord);
            self.columns
                .entry(span.coord)
                .or_default()
                .entry(entity)
                .or_default()
                .push(span);
        }
        self.revision = self.revision.saturating_add(1);
    }

    pub fn remove(&mut self, entity: Entity) {
        self.errors.remove(&entity);
        if let Some(coords) = self.entities.remove(&entity) {
            for coord in coords {
                if let Some(column) = self.columns.get_mut(&coord) {
                    column.remove(&entity);
                    if column.is_empty() {
                        self.columns.remove(&coord);
                    }
                }
            }
            self.revision = self.revision.saturating_add(1);
        }
    }

    fn candidates(&self, origin: Vec3, end: Vec3, radius: f32) -> impl Iterator<Item = Span> + '_ {
        let mut ring = 1;
        let mut covered = FACE;
        while radius >= covered {
            ring += 1;
            covered += FACE;
        }
        let coords = HexCoord::from_world(origin)
            .line_between(HexCoord::from_world(end))
            .into_iter()
            .flat_map(|coord| coord.within_radius(ring))
            .collect::<BTreeSet<_>>();
        coords
            .into_iter()
            .filter_map(|coord| self.columns.get(&coord))
            .flat_map(|column| column.values())
            .flatten()
            .copied()
    }

    pub fn clear(&self, feet: Vec3, height: f32, radius: f32) -> bool {
        feet.is_finite()
            && self.error.is_none()
            && !self
                .candidates(feet, feet, radius)
                .filter(|span| span.material == Material::Solid)
                .any(|span| contains(span, feet, height, radius))
    }

    pub fn sweep(&self, feet: Vec3, delta: Vec3, height: f32, radius: f32) -> Option<Hit> {
        self.candidates(feet, feet + delta, radius)
            .filter(|span| span.material == Material::Solid)
            .filter_map(|span| sweep_span(span, feet, delta, height, radius))
            .min_by(|a, b| a.fraction.total_cmp(&b.fraction))
    }

    /// Water depth at a supported body's feet; liquids in a lower cave do not count.
    pub fn water_depth(&self, feet: Vec3, radius: f32) -> f32 {
        self.candidates(feet, feet, radius)
            .filter(|span| {
                span.material == Material::Liquid
                    && span.bottom <= feet.y + SKIN * 4.0
                    && span.top > feet.y
                    && horizontal_contains(*span, feet, radius)
            })
            .map(|span| span.top - feet.y)
            .fold(0.0, f32::max)
    }

    pub fn ground(&self, feet: Vec3, height: f32, radius: f32, distance: f32) -> Option<Vec3> {
        let delta = Vec3::NEG_Y * distance;
        self.sweep(feet, delta, height, radius)
            .filter(|hit| hit.normal.y > 0.5)
            .map(|hit| feet + delta * hit.fraction + Vec3::Y * SKIN)
    }
}

fn horizontal_contains(span: Span, feet: Vec3, radius: f32) -> bool {
    let local = feet - span.coord.to_world(0.0);
    NORMALS
        .into_iter()
        .take(3)
        .all(|normal| local.dot(normal).abs() < FACE + radius - SKIN)
}

fn contains(span: Span, feet: Vec3, height: f32, radius: f32) -> bool {
    horizontal_contains(span, feet, radius)
        && feet.y > span.bottom - height + SKIN
        && feet.y < span.top - SKIN
}

/// Sweep a feet point through a body-expanded convex hex prism. Boundary contacts
/// block only movement into the surface, so adjacent floor spans cannot snag feet.
fn sweep_span(span: Span, feet: Vec3, delta: Vec3, height: f32, radius: f32) -> Option<Hit> {
    let local = feet - span.coord.to_world(0.0);
    let mut enter: f32 = -f32::INFINITY;
    let mut exit: f32 = f32::INFINITY;
    let mut normal = Vec3::ZERO;
    for axis in NORMALS {
        let (lower, upper) = if axis.y > 0.5 {
            (span.bottom - height, span.top)
        } else {
            (-FACE - radius, FACE + radius)
        };
        let position = local.dot(axis);
        let velocity = delta.dot(axis);
        if velocity.abs() <= f32::EPSILON {
            // Tangency belongs to the free side, including the supporting floor.
            if position <= lower + SKIN || position >= upper - SKIN {
                return None;
            }
            continue;
        }
        let a = (lower - position) / velocity;
        let b = (upper - position) / velocity;
        let near = a.min(b);
        if near > enter {
            enter = near;
            normal = if velocity > 0.0 { -axis } else { axis };
        }
        exit = exit.min(a.max(b));
        if enter > exit {
            return None;
        }
    }
    if exit < 0.0 || !(-SKIN..=1.0).contains(&enter) || normal.dot(delta) >= 0.0 {
        return None;
    }
    Some(Hit {
        fraction: enter.max(0.0),
        normal,
    })
}

#[expect(
    clippy::cast_precision_loss,
    reason = "validated finite world voxel coordinates use renderer f32 geometry"
)]
fn object_spans(
    instance: &ObjectInstance,
    catalog: &RuntimeArtCatalog,
) -> Result<Vec<Span>, String> {
    // These are soft dressing despite the authoring catalog's generic Structure tag.
    if matches!(
        instance.object_id().as_str(),
        "prop/grass-tuft" | "prop/snowy-grass-tuft" | "prop/cave-moss" | "prop/cave-lichen"
    ) {
        return Ok(Vec::new());
    }
    instance.validate().map_err(|error| error.to_string())?;
    let blueprint = catalog
        .object(instance.object_id())
        .ok_or_else(|| format!("Missing collision object {}", instance.object_id()))?;
    let mut runs = BTreeMap::<HexCoord, Vec<i32>>::new();
    for placement in &blueprint.placements {
        let rotated = instance
            .rotation()
            .rotate_voxel(placement.position, blueprint.origin)
            .ok_or_else(|| "Object collision rotation overflows".to_owned())?;
        let q = rotated
            .q
            .checked_sub(blueprint.origin.q)
            .and_then(|offset| instance.origin().coord.x().checked_add(offset));
        let r = rotated
            .r
            .checked_sub(blueprint.origin.r)
            .and_then(|offset| instance.origin().coord.y().checked_add(offset));
        let level = rotated
            .level
            .checked_sub(blueprint.origin.level)
            .and_then(|offset| instance.origin().level.checked_add(offset));
        let (Some(q), Some(r), Some(level)) = (q, r, level) else {
            return Err("Object collision coordinates overflow".into());
        };
        runs.entry(HexCoord::from_axial(q, r))
            .or_default()
            .push(level);
    }
    let mut spans: Vec<Span> = Vec::new();
    for (coord, mut levels) in runs {
        levels.sort_unstable();
        for level in levels {
            let bottom = level as f32 * instance.level_height();
            let top = (level as f32 + 1.0) * instance.level_height();
            if let Some(last) = spans
                .last_mut()
                .filter(|last| last.coord == coord && bottom <= last.top + SKIN)
            {
                last.top = last.top.max(top);
            } else {
                spans.push(Span {
                    coord,
                    bottom,
                    top,
                    material: Material::Solid,
                });
            }
        }
    }
    Ok(spans)
}

/// Changes are applied once before the controller; hidden rendering still collides.
pub(super) fn refresh(
    mut index: ResMut<CollisionWorld>,
    substances: Res<SubstanceTable>,
    catalog: Res<RuntimeArtCatalog>,
    tiles: Query<(Entity, &TilePos, &HexSpan, &SubstanceId), With<HexTile>>,
    changed_tiles: Query<
        (Entity, &TilePos, &HexSpan, &SubstanceId),
        (
            With<HexTile>,
            Or<(
                Added<HexTile>,
                Changed<TilePos>,
                Changed<HexSpan>,
                Changed<SubstanceId>,
            )>,
        ),
    >,
    objects: Query<(Entity, &ObjectInstance)>,
    changed_objects: Query<(Entity, &ObjectInstance), Changed<ObjectInstance>>,
    mut removed_tiles: RemovedComponents<HexTile>,
    mut removed_positions: RemovedComponents<TilePos>,
    mut removed_spans: RemovedComponents<HexSpan>,
    mut removed_substances: RemovedComponents<SubstanceId>,
    mut removed_objects: RemovedComponents<ObjectInstance>,
) {
    let all = !index.initialized || substances.is_changed() || catalog.is_changed();
    let revision = index.revision;
    if all {
        *index = CollisionWorld {
            revision,
            ..default()
        };
    }
    for entity in removed_tiles
        .read()
        .chain(removed_positions.read())
        .chain(removed_spans.read())
        .chain(removed_substances.read())
        .chain(removed_objects.read())
    {
        index.remove(entity);
    }
    let tile_updates: Vec<_> = if all {
        tiles.iter().collect()
    } else {
        changed_tiles.iter().collect()
    };
    for (entity, position, span, substance) in tile_updates {
        let material = if substances.is_solid(*substance) {
            Some(Material::Solid)
        } else if matches!(substances.name(*substance), Some("water" | "lava")) {
            Some(Material::Liquid)
        } else {
            None
        };
        index.replace(
            entity,
            material
                .map(|material| Span {
                    coord: position.coord,
                    bottom: span.bottom,
                    top: span.top,
                    material,
                })
                .into_iter()
                .collect(),
        );
    }
    let object_updates: Vec<_> = if all {
        objects.iter().collect()
    } else {
        changed_objects.iter().collect()
    };
    for (entity, instance) in object_updates {
        match object_spans(instance, &catalog) {
            Ok(spans) => index.replace(entity, spans),
            Err(error) => {
                index.remove(entity);
                index.errors.insert(entity, error);
            }
        }
    }
    if all || index.revision != revision {
        index.floor = index
            .columns
            .values()
            .flat_map(|column| column.values())
            .flatten()
            .filter(|span| span.material == Material::Solid)
            .map(|span| span.bottom)
            .fold(0.0, f32::min)
            - 20.0;
    }
    index.error = index.errors.values().next().cloned();
    index.initialized = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(coord: HexCoord, bottom: f32, top: f32) -> Span {
        Span {
            coord,
            bottom,
            top,
            material: Material::Solid,
        }
    }
    fn index(spans: Vec<Span>) -> CollisionWorld {
        let mut world = CollisionWorld {
            initialized: true,
            ..default()
        };
        world.replace(Entity::from_bits(1), spans);
        world
    }

    #[test]
    fn all_hex_faces_stop_a_swept_body_without_tunnelling() {
        let world = index(vec![solid(HexCoord::default(), 0.0, 4.0)]);
        for axis in NORMALS.into_iter().take(3).flat_map(|axis| [axis, -axis]) {
            let start = axis * 10.0 + Vec3::Y;
            let delta = -axis * 20.0;
            let hit = world
                .sweep(start, delta, 0.8, 0.25)
                .expect("thin prism must block a long sweep");
            assert!((hit.fraction - (10.0 - FACE - 0.25) / 20.0).abs() < 0.00001);
            assert!(hit.normal.dot(axis) > 0.99);
            assert!(world.clear(start + delta * hit.fraction, 0.8, 0.25));
        }
    }

    #[test]
    fn floor_tangency_does_not_block_horizontal_motion_and_stacked_ceiling_remains() {
        let world = index(vec![
            solid(HexCoord::default(), -1.0, 0.0),
            solid(HexCoord::default(), 2.0, 3.0),
        ]);
        let feet = Vec3::ZERO;
        assert!(world.sweep(feet, Vec3::X * 0.5, 0.8, 0.25).is_none());
        assert!(world.clear(feet, 0.8, 0.25));
        let roof = world
            .sweep(feet, Vec3::Y * 5.0, 0.8, 0.25)
            .expect("ceiling");
        assert!((roof.fraction - 1.2 / 5.0).abs() < 0.0001);
        assert!(roof.normal.y < -0.9);
        let landing = world
            .ground(Vec3::Y * 4.0, 0.8, 0.25, 8.0)
            .expect("upper bridge");
        assert!((landing.y - 3.0).abs() < 0.001);
        let lower = world.ground(Vec3::Y, 0.8, 0.25, 8.0).expect("lower floor");
        assert!(lower.y.abs() < 0.001);
    }

    #[test]
    fn removing_one_object_preserves_shared_column_and_water_is_not_support() {
        let mut world = index(vec![solid(HexCoord::default(), -1.0, 0.0)]);
        world.replace(
            Entity::from_bits(2),
            vec![solid(HexCoord::default(), 2.0, 3.0)],
        );
        world.replace(
            Entity::from_bits(3),
            vec![Span {
                coord: HexCoord::default(),
                bottom: 0.0,
                top: 0.8,
                material: Material::Liquid,
            }],
        );
        assert!((world.water_depth(Vec3::ZERO, 0.25) - 0.8).abs() < 0.001);
        world.remove(Entity::from_bits(2));
        assert!(world.clear(Vec3::Y * 2.0, 0.8, 0.25));
        assert!(
            world
                .ground(Vec3::Y * 5.0, 0.8, 0.25, 10.0)
                .expect("floor")
                .y
                < 0.001
        );
    }
    fn catalog() -> RuntimeArtCatalog {
        use hex_assets::{ArtPalette, ObjectBlueprint, ObjectCatalogFile, VoxelStyleCatalog};
        let palette: ArtPalette =
            ron::from_str(include_str!("../../../../assets/art/palette.ron")).expect("palette");
        let styles: VoxelStyleCatalog =
            ron::from_str(include_str!("../../../../assets/art/voxel_styles.ron")).expect("styles");
        let blueprint: ObjectBlueprint = ron::from_str(
            r#"(
            schema_version:1, id:"prop/test-collider", display_name:"Collision fixture",
            category:Prop, bounds:(radius:3,min_level:-2,height:5), connectivity:Free,
            origin:(q:1,r:-1,level:-2), placements:[
                (position:(q:1,r:-1,level:-2),style:"crystal/cyan-body",part:Prop(Structure)),
                (position:(q:2,r:-1,level:-2),style:"crystal/cyan-body",part:Prop(Structure)),
                (position:(q:2,r:-1,level:-1),style:"crystal/cyan-body",part:Prop(Structure)),
                (position:(q:2,r:-1,level:2),style:"crystal/cyan-body",part:Prop(Structure)),
            ],blocker_footprint:[],canopy_occluders:[]
        )"#,
        )
        .expect("fixture blueprint");
        let manifest = ObjectCatalogFile::new([blueprint.id.clone()]).expect("manifest");
        RuntimeArtCatalog::from_sources(
            &palette,
            &styles,
            &manifest,
            BTreeMap::from([(blueprint.id.clone(), blueprint)]),
        )
        .expect("catalog")
    }

    fn instance(steps: u8) -> ObjectInstance {
        ObjectInstance::new(
            hex_assets::ObjectAssetId::new("prop/test-collider").expect("id"),
            TilePos {
                coord: HexCoord::from_axial(5, -3),
                level: 10,
            },
            0.35,
            hex_assets::HexObjectRotation::new(steps).expect("rotation"),
        )
        .expect("instance")
    }

    #[test]
    fn object_projection_preserves_pivot_rotation_scale_and_stacked_gaps() {
        let catalog = catalog();
        for (rotation, offset) in (0..6).zip([(1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1)]) {
            let spans = object_spans(&instance(rotation), &catalog).expect("projection");
            assert_eq!(spans.len(), 3);
            let side = HexCoord::from_axial(5 + offset.0, -3 + offset.1);
            let lower = spans
                .iter()
                .find(|span| span.coord == side && span.bottom < 4.0)
                .expect("lower run");
            assert!((lower.bottom - 3.5).abs() < 0.0001 && (lower.top - 4.2).abs() < 0.0001);
            let upper = spans
                .iter()
                .find(|span| span.coord == side && span.bottom > 4.0)
                .expect("upper run");
            assert!((upper.bottom - 4.9).abs() < 0.0001 && (upper.top - 5.25).abs() < 0.0001);
        }
    }

    fn refresh_app() -> App {
        let palette =
            ron::from_str(include_str!("../../../../assets/art/palette.ron")).expect("palette");
        let file = ron::from_str(include_str!("../../../../assets/config/substances.ron"))
            .expect("substances");
        let mut app = App::new();
        app.insert_resource(SubstanceTable::from_file(&file, &palette).expect("substance table"))
            .insert_resource(catalog())
            .init_resource::<CollisionWorld>()
            .add_systems(Update, refresh);
        app
    }

    #[test]
    fn unchanged_frames_reuse_index_and_component_marker_addition_and_removal_refresh_it() {
        let mut app = refresh_app();
        let stone = app
            .world()
            .resource::<SubstanceTable>()
            .id("stone")
            .expect("stone");
        let tile = app
            .world_mut()
            .spawn((TilePos::ORIGIN, HexSpan::new(0.0, 1.0), stone))
            .id();
        app.update();
        let initial = app.world().resource::<CollisionWorld>().revision;
        app.update();
        assert_eq!(app.world().resource::<CollisionWorld>().revision, initial);
        app.world_mut().entity_mut(tile).insert(HexTile);
        app.update();
        assert!(!app
            .world()
            .resource::<CollisionWorld>()
            .clear(Vec3::Y * 0.1, 0.8, 0.25));
        let changed = app.world().resource::<CollisionWorld>().revision;
        app.update();
        assert_eq!(app.world().resource::<CollisionWorld>().revision, changed);
        app.world_mut().entity_mut(tile).remove::<HexSpan>();
        app.update();
        assert!(app
            .world()
            .resource::<CollisionWorld>()
            .clear(Vec3::Y * 0.1, 0.8, 0.25));
    }

    #[test]
    fn missing_object_error_clears_after_correction_or_removal() {
        let mut app = refresh_app();
        let broken = ObjectInstance::new(
            hex_assets::ObjectAssetId::new("prop/missing").expect("id"),
            TilePos::ORIGIN,
            0.35,
            hex_assets::HexObjectRotation::new(0).expect("rotation"),
        )
        .expect("instance");
        let entity = app.world_mut().spawn(broken.clone()).id();
        app.update();
        assert!(app.world().resource::<CollisionWorld>().error.is_some());
        app.world_mut().entity_mut(entity).insert(instance(0));
        app.update();
        assert!(app.world().resource::<CollisionWorld>().error.is_none());
        app.world_mut().entity_mut(entity).insert(broken);
        app.update();
        assert!(app.world().resource::<CollisionWorld>().error.is_some());
        app.world_mut()
            .entity_mut(entity)
            .remove::<ObjectInstance>();
        app.update();
        assert!(app.world().resource::<CollisionWorld>().error.is_none());
    }
}
