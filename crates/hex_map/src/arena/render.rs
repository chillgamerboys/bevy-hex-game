//! Disposable column presentation sharing the world's exact physical voxel spans.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;
use bevy::transform::TransformSystems;

use super::*;

#[derive(Resource, Default)]
struct RenderCache {
    mesh: Option<Handle<Mesh>>,
    materials: BTreeMap<SubstanceId, Handle<StandardMaterial>>,
    columns: BTreeMap<HexCoord, Entity>,
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<RenderCache>().add_systems(
        PostUpdate,
        refresh
            .before(TransformSystems::Propagate)
            .run_if(resource_exists::<VoxelMap>),
    );
}

fn refresh(
    mut commands: Commands,
    mut cache: ResMut<RenderCache>,
    mut state: ResMut<ArenaWorldState>,
    map: Res<VoxelMap>,
    geometry: Res<ArenaVoxelGeometry>,
    substances: Res<SubstanceTable>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
) {
    let (Some(mut meshes), Some(mut materials)) = (meshes, materials) else {
        // Logical tests intentionally do not install a renderer or asset storage.
        return;
    };
    if state.render_dirty.is_empty() {
        return;
    }
    let mesh = cache
        .mesh
        .get_or_insert_with(|| meshes.add(hex_prism()))
        .clone();
    for coord in std::mem::take(&mut state.render_dirty) {
        if let Some(previous) = cache.columns.remove(&coord) {
            commands.entity(previous).despawn();
        }
        let Some(column) = map.column(coord) else {
            continue;
        };
        let mut children = Vec::new();
        for run in crate::runs(column) {
            let Some(substance) = substances.get(run.substance) else {
                continue;
            };
            let (red, green, blue) = substance.color;
            let material = cache
                .materials
                .entry(run.substance)
                .or_insert_with(|| {
                    materials.add(StandardMaterial {
                        base_color: Color::srgb(red, green, blue),
                        perceptual_roughness: 0.95,
                        reflectance: 0.08,
                        ..default()
                    })
                })
                .clone();
            let lower = TilePos::new(coord, run.bottom);
            let upper = TilePos::new(coord, run.top - 1);
            let bottom = geometry.top(lower) - geometry.level_height;
            let top = geometry.top(upper);
            let child = commands
                .spawn((
                    Name::new("Arena terrain run"),
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material),
                    Transform::from_translation(coord.to_world((bottom + top) * 0.5))
                        .with_scale(Vec3::new(1.0, top - bottom, 1.0)),
                ))
                .id();
            children.push(child);
        }
        let root = commands
            .spawn((
                Name::new(format!("Arena column {},{}", coord.x(), coord.y())),
                Transform::default(),
                Visibility::default(),
            ))
            .add_children(&children)
            .id();
        cache.columns.insert(coord, root);
    }
}

/// Circumradius-one, Y-up pointy hex with exactly the same horizontal half-spaces
/// as `HexCoord`. Separate triangle normals keep the six sides visibly faceted.
fn hex_prism() -> Mesh {
    let corners = [
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(0.866_025_4, 0.0, 0.5),
        Vec3::new(0.866_025_4, 0.0, -0.5),
        Vec3::new(0.0, 0.0, -1.0),
        Vec3::new(-0.866_025_4, 0.0, -0.5),
        Vec3::new(-0.866_025_4, 0.0, 0.5),
    ];
    let mut positions = Vec::<[f32; 3]>::new();
    let mut normals = Vec::<[f32; 3]>::new();
    let mut triangle = |a: Vec3, b: Vec3, c: Vec3| {
        let normal = (b - a).cross(c - a).normalize();
        positions.extend([a.to_array(), b.to_array(), c.to_array()]);
        normals.extend([normal.to_array(); 3]);
    };
    let up = Vec3::Y * 0.5;
    for (a, b) in corners.into_iter().zip(corners.into_iter().cycle().skip(1)) {
        triangle(a - up, b - up, b + up);
        triangle(a - up, b + up, a + up);
        triangle(up, a + up, b + up);
        triangle(-up, b - up, a - up);
    }
    let uvs = vec![[0.0_f32, 0.0]; positions.len()];
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
}

#[cfg(test)]
mod tests {
    use bevy::mesh::VertexAttributeValues;

    use super::*;

    #[test]
    fn prism_faces_have_outward_normals_and_exact_hex_extents() {
        let mesh = hex_prism();
        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("prism positions missing");
        };
        let Some(VertexAttributeValues::Float32x3(normals)) =
            mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
        else {
            panic!("prism normals missing");
        };
        assert_eq!(positions.len(), 72);
        for (position, normal) in positions.iter().zip(normals) {
            let point = Vec3::from_array(*position);
            let normal = Vec3::from_array(*normal);
            assert!(point.dot(normal) > 0.0);
            assert!(point.y.abs() <= 0.5);
            assert!(point.x.abs() <= 0.866_025_4);
            assert!(point.z.abs() <= 1.0);
        }
    }

    #[test]
    fn presentation_replaces_only_changed_columns_and_reuses_meshes() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(Assets::<Mesh>::default())
            .insert_resource(Assets::<StandardMaterial>::default())
            .add_plugins(super::super::plugin);
        app.update();
        let untouched = HexCoord::from_axial(5, 0);
        let before = app.world().resource::<RenderCache>().columns.clone();
        app.world_mut().write_message(TerrainEdit::Clear {
            pos: TilePos::new(HexCoord::ORIGIN, 3),
        });
        app.world_mut().run_schedule(ArenaTick);
        app.update();
        let after = &app.world().resource::<RenderCache>().columns;
        assert_eq!(after.len(), 469);
        assert_eq!(after.get(&untouched), before.get(&untouched));
        assert_ne!(after.get(&HexCoord::ORIGIN), before.get(&HexCoord::ORIGIN));
        assert_eq!(app.world().resource::<Assets<Mesh>>().len(), 1);
        assert_eq!(app.world().resource::<Assets<StandardMaterial>>().len(), 4);
    }
}
