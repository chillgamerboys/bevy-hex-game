use super::mesh::MeshData;
use bevy::prelude::*;
use hex_core::HexCoord;
use std::collections::{BTreeMap, BTreeSet};

/// Exact published liquid interval; coordinates and heights are world facts.
#[derive(Debug, Clone, Copy)]
pub struct OceanBoundaryColumn {
    /// The occupied water column.
    pub coordinate: HexCoord,
    /// Lower face in world units, inclusive.
    pub bottom: f32,
    /// Upper face in world units.
    pub top: f32,
}

/// Optional closed local water boundary behind the shared displaced surface.
/// The host updates this only when its published near-water window changes.
#[derive(Resource, Debug, Clone, Default)]
pub struct OceanNearBoundary {
    /// Changes when exact intervals or known-neighbor coverage change.
    pub revision: u64,
    /// Bounded visible/local water intervals, at most32,768.
    pub columns: Vec<OceanBoundaryColumn>,
    /// Columns whose complete liquid facts are known; unknown neighbors do not
    /// create a false vertical water wall at the streaming boundary.
    pub known_columns: BTreeSet<HexCoord>,
}

impl OceanNearBoundary {
    pub(super) fn build(&self) -> Option<Mesh> {
        if self.columns.len() > 32_768 {
            return None;
        }
        let mut lookup: BTreeMap<HexCoord, Vec<(f32, f32)>> = BTreeMap::new();
        for column in &self.columns {
            if !column.bottom.is_finite() || !column.top.is_finite() || column.top <= column.bottom
            {
                return None;
            }
            lookup
                .entry(column.coordinate)
                .or_default()
                .push((column.bottom, column.top));
        }
        let mut mesh = MeshData::default();
        for column in &self.columns {
            append_bottom(&mut mesh, *column);
            for neighbor in column
                .coordinate
                .within_radius(1)
                .into_iter()
                .filter(|neighbor| neighbor != &column.coordinate)
            {
                if !self.known_columns.contains(&neighbor) {
                    continue;
                }
                let mut remaining = vec![(column.bottom, column.top)];
                for (low, high) in lookup.get(&neighbor).into_iter().flatten() {
                    remaining = remaining
                        .into_iter()
                        .flat_map(|(a, b)| {
                            [(a, b.min(*low)), (a.max(*high), b)]
                                .into_iter()
                                .filter(|(start, end)| end > start)
                        })
                        .collect();
                }
                for (low, high) in remaining {
                    append_side(&mut mesh, column.coordinate, neighbor, low, high);
                }
            }
        }
        Some(mesh.into_mesh())
    }
}

fn append_bottom(mesh: &mut MeshData, column: OceanBoundaryColumn) {
    let center = column.coordinate.to_world(column.bottom);
    let base = mesh.len();
    mesh.vertex(center, Vec3::NEG_Y);
    for side in 0_u16..6 {
        let angle = (f32::from(side) * 60.0 + 30.0).to_radians();
        mesh.vertex(
            center + Vec3::new(angle.cos(), 0.0, angle.sin()),
            Vec3::NEG_Y,
        );
    }
    for side in 0..6 {
        mesh.indices
            .extend([base, base + 1 + side, base + 1 + (side + 1) % 6]);
    }
}

fn append_side(
    mesh: &mut MeshData,
    coordinate: HexCoord,
    neighbor: HexCoord,
    bottom: f32,
    top: f32,
) {
    let center = coordinate.to_world(0.0);
    let outward = (neighbor.to_world(0.0) - center).normalize();
    let edge = center + outward * (3.0_f32.sqrt() * 0.5);
    let tangent = Vec3::new(-outward.z, 0.0, outward.x) * 0.5;
    let start = mesh.len();
    for point in [
        edge - tangent + Vec3::Y * top,
        edge + tangent + Vec3::Y * top,
        edge + tangent + Vec3::Y * bottom,
        edge - tangent + Vec3::Y * bottom,
    ] {
        mesh.vertex(point, outward);
    }
    mesh.indices
        .extend([start, start + 1, start + 2, start, start + 2, start + 3]);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_neighbors_do_not_invent_streaming_walls_and_known_dry_edges_close() {
        let column = OceanBoundaryColumn {
            coordinate: HexCoord::ORIGIN,
            bottom: -3.0,
            top: 0.0,
        };
        let mut boundary = OceanNearBoundary {
            columns: vec![column],
            ..default()
        };
        assert_eq!(boundary.build().unwrap().count_vertices(), 7);
        boundary.known_columns = HexCoord::ORIGIN.within_radius(1).into_iter().collect();
        assert_eq!(boundary.build().unwrap().count_vertices(), 31);
        boundary.columns.push(OceanBoundaryColumn {
            coordinate: HexCoord::from_axial(1, 0),
            ..column
        });
        boundary
            .known_columns
            .extend(HexCoord::from_axial(1, 0).within_radius(1));
        let indices = boundary.build().unwrap().indices().unwrap().len();
        assert_eq!(
            indices, 96,
            "two bottoms and ten exterior sides; no shared water face"
        );
    }
}
