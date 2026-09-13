use bevy::{
    asset::RenderAssetUsages, mesh::Indices, prelude::*, render::render_resource::PrimitiveTopology,
};

const SEGMENTS: u32 = 512;
pub(super) const HORIZON_RADIUS: f32 = 12_288.0;

/// One connected radial topology avoids independent chunk edges and LOD cracks.
/// Four-unit radial spacing covers the first256 units, then grows toward the
/// decorative horizon. Only the owning entity's X/Z translation changes.
pub(super) fn surface_mesh() -> Mesh {
    let radii = (1_u16..=64)
        .map(|n| f32::from(n) * 4.0)
        .chain((1_u16..=48).map(|n| 256.0 + f32::from(n) * 16.0))
        .chain((1_u16..=48).map(|n| 1024.0 + f32::from(n) * 64.0))
        .chain((1_u16..=32).map(|n| 4096.0 + f32::from(n) * 256.0));
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
    data.into_mesh()
}

#[derive(Default)]
pub(super) struct MeshData {
    pub positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
}
impl MeshData {
    pub fn len(&self) -> u32 {
        u32::try_from(self.positions.len()).unwrap_or(u32::MAX)
    }
    pub fn vertex(&mut self, position: Vec3, normal: Vec3) {
        self.positions.push(position.to_array());
        self.normals.push(normal.to_array());
    }
    pub fn into_mesh(self) -> Mesh {
        let count = self.positions.len();
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; count])
        .with_inserted_indices(Indices::U32(self.indices))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
