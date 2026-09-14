//! One bounded opaque batch; only disposable mesh data changes with the waves.
use super::*;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;
use bevy::transform::TransformSystems;
use hex_core::arena::ArenaTerrainView;
use hex_core::ocean::OceanWaterColumn;
use hex_core::water_lab::LabStyle;

#[derive(Resource, Default)]
struct Cache {
    entity: Option<Entity>,
    mesh: Option<Handle<Mesh>>,
    last: Option<(u64, WaterLabSettings, u32)>,
}

pub(super) fn install(app: &mut App) {
    app.init_resource::<Cache>()
        .add_systems(PostUpdate, render.before(TransformSystems::Propagate));
}

const CORNERS: [Vec3; 6] = [
    Vec3::new(0.0, 0.0, 1.0),
    Vec3::new(0.866_025_4, 0.0, 0.5),
    Vec3::new(0.866_025_4, 0.0, -0.5),
    Vec3::new(0.0, 0.0, -1.0),
    Vec3::new(-0.866_025_4, 0.0, -0.5),
    Vec3::new(-0.866_025_4, 0.0, 0.5),
];

fn color(
    surface: LabSurface,
    coord: HexCoord,
    column: OceanWaterColumn,
    height: f32,
    time: f32,
) -> [f32; 4] {
    let point = coord.to_world(0.0);
    let depth = ((column.mean_height - column.bed_height) / 6.0).clamp(0.0, 1.0);
    let mut rgb = Vec3::new(0.045, 0.36, 0.38).lerp(Vec3::new(0.012, 0.095, 0.20), depth);
    if surface.settings.style != LabStyle::Depth {
        let crest = ((height - column.mean_height) / 1.2).clamp(0.0, 1.0);
        rgb = rgb.lerp(Vec3::new(0.46, 0.72, 0.73), crest * crest);
    }
    if surface.settings.style == LabStyle::Patterns {
        let variation =
            ((point.x * 0.34 + point.z * 0.19).sin() * (point.z * 0.27).cos()).signum() * 0.025;
        rgb += Vec3::splat(variation);
        if (point.z - 3.0).abs() < 2.2 && point.x.abs() < 17.0 {
            let stripe = (point.x * 0.7 - surface.settings.phase(time) * 2.0)
                .sin()
                .max(0.0)
                .powi(6);
            rgb = rgb.lerp(Vec3::new(0.23, 0.60, 0.58), stripe * 0.6);
        }
    }
    [rgb.x.max(0.0), rgb.y.max(0.0), rgb.z.max(0.0), 1.0]
}

fn mesh(
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    surface: LabSurface,
    time: f32,
) -> Mesh {
    let columns: BTreeMap<_, _> = view
        .liquids
        .iter()
        .map(|span| {
            let col = OceanWaterColumn {
                mean_height: geometry.top(TilePos::new(span.bottom.coord, span.top_level)),
                bed_height: geometry.top(span.bottom) - geometry.level_height,
                water_id: span.substance,
            };
            (
                span.bottom.coord,
                (col, surface.height(span.bottom.coord, time, col)),
            )
        })
        .collect();
    let mut positions = Vec::with_capacity(columns.len() * 30);
    let mut normals = Vec::with_capacity(columns.len() * 30);
    let mut colors = Vec::with_capacity(columns.len() * 30);
    let mut triangle = |a: Vec3, b: Vec3, c: Vec3, tint: [f32; 4]| {
        let normal = (b - a).cross(c - a).normalize_or(Vec3::Y);
        positions.extend([a.to_array(), b.to_array(), c.to_array()]);
        normals.extend([normal.to_array(); 3]);
        colors.extend([tint; 3]);
    };
    for (coord, (column, height)) in &columns {
        let center = coord.to_world(*height);
        let tint = color(surface, *coord, *column, *height, time);
        for (a, b) in CORNERS.into_iter().zip(CORNERS.into_iter().cycle().skip(1)) {
            triangle(center, center + a, center + b, tint);
            let neighbour = HexCoord::from_world(center + a + b);
            let lower = columns
                .get(&neighbour)
                .map_or(column.bed_height, |(_, top)| *top)
                .min(*height);
            if *height - lower > 0.001 {
                let mut side = tint;
                for channel in side.iter_mut().take(3) {
                    *channel *= 0.72;
                }
                triangle(
                    (center + a).with_y(lower),
                    (center + b).with_y(lower),
                    center + b,
                    side,
                );
                triangle((center + a).with_y(lower), center + b, center + a, side);
            }
        }
    }
    let uvs = vec![[0.0, 0.0]; positions.len()];
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
}

#[expect(
    clippy::too_many_arguments,
    reason = "Disposable lab batch joins exact geometry, comparison controls, clock and render assets."
)]
fn render(
    mut commands: Commands,
    frame: Res<WaterLabFrame>,
    settings: Res<WaterLabSettings>,
    view: Option<Res<ArenaTerrainView>>,
    geometry: Option<Res<ArenaVoxelGeometry>>,
    mut cache: ResMut<Cache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !frame.enabled {
        if let Some(entity) = cache.entity.take() {
            commands.entity(entity).despawn();
        }
        if let Some(handle) = cache.mesh.take() {
            meshes.remove(handle.id());
        }
        cache.last = None;
        return;
    }
    let (Some(view), Some(geometry)) = (view, geometry) else {
        return;
    };
    let key = (view.revision, *settings, frame.seconds.to_bits());
    if cache.last == Some(key) {
        return;
    }
    let surface = LabSurface {
        settings: *settings,
        level_height: geometry.level_height,
    };
    let batch = mesh(&view, *geometry, surface, frame.seconds);
    if let Some(handle) = &cache.mesh {
        if let Some(mut existing) = meshes.get_mut(handle) {
            *existing = batch;
        }
    } else {
        let handle = meshes.add(batch);
        let material = materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.85,
            reflectance: 0.12,
            ..default()
        });
        cache.entity = Some(
            commands
                .spawn((
                    Name::new("Water lab opaque columns"),
                    Mesh3d(handle.clone()),
                    MeshMaterial3d(material),
                    Transform::default(),
                    bevy::camera::visibility::NoFrustumCulling,
                ))
                .id(),
        );
        cache.mesh = Some(handle);
    }
    cache.last = Some(key);
}
