use bevy::{
    asset::RenderAssetUsages, mesh::Indices, prelude::*, render::render_resource::PrimitiveTopology,
};

const SEGMENTS: u32 = 512;
pub(super) const HORIZON_RADIUS: f32 = 12_288.0;

/// One connected radial topology avoids independent chunk edges and LOD cracks.
/// Four-unit radial spacing covers the first256 units, then grows toward the
/// decorative horizon. Only the owning entity's X/Z translation changes.
pub(super) fn surface_mesh() -> Mesh {
    surface_data().into_mesh()
}

fn surface_data() -> MeshData {
    let radii = (1_u16..=64)
        .map(|n| f32::from(n) * 4.0)
        .chain((1_u16..=48).map(|n| 256.0 + f32::from(n) * 16.0))
        .chain((1_u16..=48).map(|n| 1024.0 + f32::from(n) * 64.0))
        .chain((1_u16..=32).map(|n| 4096.0 + f32::from(n) * ((HORIZON_RADIUS - 4096.0) / 32.0)));
    let mut data = MeshData::default();
    data.vertex(Vec3::ZERO, Vec3::Y);
    let mut previous = None;
    for radius in radii {
        let start = data.len();
        for segment in 0_u16..512 {
            let angle = f32::from(segment) * std::f32::consts::TAU / 512.0;
            data.vertex(
                Vec3::new(angle.cos() * radius, 0.0, angle.sin() * radius),
                Vec3::Y,
            );
        }
        for segment in 0..SEGMENTS {
            let next = (segment + 1) % SEGMENTS;
            if let Some(prior) = previous {
                data.indices.extend([
                    prior + segment,
                    start + next,
                    start + segment,
                    prior + segment,
                    prior + next,
                    start + next,
                ]);
            } else {
                data.indices.extend([0, start + next, start + segment]);
            }
        }
        previous = Some(start);
    }
    data
}

/// Fixed local hex columns, backed by the existing coarser distant surface.
pub(super) fn voxel_surface_mesh() -> Mesh {
    let mut data = surface_data();
    let corners = [
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(0.866_025_4, 0.0, 0.5),
        Vec3::new(0.866_025_4, 0.0, -0.5),
        Vec3::new(0.0, 0.0, -1.0),
        Vec3::new(-0.866_025_4, 0.0, -0.5),
        Vec3::new(-0.866_025_4, 0.0, 0.5),
    ];
    for coord in hex_core::HexCoord::ORIGIN.within_radius(64) {
        let center = coord.to_world(0.0);
        let uv = Vec2::new(center.x, center.z);
        let start = data.len();
        data.vertex_with_uv(center, Vec3::Y, uv);
        for corner in corners {
            data.vertex_with_uv(center + corner, Vec3::Y, uv);
        }
        for edge in 0..6_u32 {
            data.indices
                .extend([start, start + 1 + edge, start + 1 + (edge + 1) % 6]);
        }
        for (a, b) in corners.into_iter().zip(corners.into_iter().cycle().skip(1)) {
            let start = data.len();
            let normal = (a + b).normalize();
            for point in [
                center + a - Vec3::Y,
                center + b - Vec3::Y,
                center + b,
                center + a,
            ] {
                data.vertex_with_uv(point, normal, uv);
            }
            data.indices
                .extend([start, start + 1, start + 2, start, start + 2, start + 3]);
        }
    }
    data.into_mesh()
}

#[derive(Default)]
pub(super) struct MeshData {
    pub positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}
impl MeshData {
    pub fn len(&self) -> u32 {
        u32::try_from(self.positions.len()).unwrap_or(u32::MAX)
    }
    pub fn vertex(&mut self, position: Vec3, normal: Vec3) {
        self.vertex_with_uv(position, normal, Vec2::splat(100_000.0));
    }
    fn vertex_with_uv(&mut self, position: Vec3, normal: Vec3, uv: Vec2) {
        self.positions.push(position.to_array());
        self.normals.push(normal.to_array());
        self.uvs.push(uv.to_array());
    }
    pub fn into_mesh(self) -> Mesh {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs)
        .with_inserted_indices(Indices::U32(self.indices))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn voxel_patch_is_bounded_and_every_vertex_has_cell_metadata() {
        let mesh = voxel_surface_mesh();
        assert_eq!(mesh.count_vertices(), 98_305 + 12_481 * 31);
        assert_eq!(
            mesh.attribute(Mesh::ATTRIBUTE_UV_0).unwrap().len(),
            mesh.count_vertices()
        );
        assert_eq!(
            mesh.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap().len(),
            mesh.count_vertices()
        );
    }

    #[test]
    fn horizon_geometry_has_fixed_bounded_cost_and_no_nonfinite_vertices() {
        let mesh = surface_mesh();
        assert_eq!(mesh.count_vertices(), 98_305);
        assert_eq!(mesh.indices().unwrap().len(), 588_288);
        let bevy::mesh::VertexAttributeValues::Float32x3(points) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
        else {
            panic!("positions");
        };
        assert!(points
            .iter()
            .all(|point| Vec3::from_array(*point).is_finite()));
        assert!(points
            .iter()
            .any(|point| (Vec3::from_array(*point).length() - HORIZON_RADIUS).abs() < 0.01));
    }
}
