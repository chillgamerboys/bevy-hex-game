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

// Match the exact wet mask's sea-plane tolerance. Raised water retains its own
// flat renderer, while its intervals still occlude the ocean's neighboring faces.
fn at_sea_level(top: f32, sea: f32) -> bool {
    (top - sea).abs() < 0.01
}

impl OceanNearBoundary {
    /// A bounded axial wet/dry mask. Unknown columns retain decorative distant
    /// water; known dry columns never gain water after solid terrain is carved.
    pub(super) fn mask(&self, sea: f32) -> Option<(IVec2, UVec2, Vec<f32>)> {
        if self.known_columns.len() > 32_768 || self.columns.len() > 32_768 {
            return None;
        }
        let Some(first) = self.known_columns.first() else {
            return Some((IVec2::ZERO, UVec2::ONE, vec![0.0]));
        };
        let mut low = IVec2::new(first.x(), first.y());
        let mut high = low;
        for coord in &self.known_columns {
            let at = IVec2::new(coord.x(), coord.y());
            low = low.min(at);
            high = high.max(at);
        }
        let size = IVec2::new(
            high.x.checked_sub(low.x)?.checked_add(1)?,
            high.y.checked_sub(low.y)?.checked_add(1)?,
        );
        if size.min_element() <= 0 || size.max_element() > 512 {
            return None;
        }
        let width = usize::try_from(size.x).ok()?;
        let height = usize::try_from(size.y).ok()?;
        let mut values = vec![0.0; width.checked_mul(height)?];
        let index = |coord: HexCoord| -> Option<usize> {
            let at = IVec2::new(coord.x(), coord.y()) - low;
            Some(usize::try_from(at.y).ok()? * width + usize::try_from(at.x).ok()?)
        };
        for coord in &self.known_columns {
            *values.get_mut(index(*coord)?)? = -1.0;
        }
        for column in &self.columns {
            if self.known_columns.contains(&column.coordinate) && at_sea_level(column.top, sea) {
                *values.get_mut(index(column.coordinate)?)? = 1.0;
            }
        }
        Some((
            low,
            UVec2::new(u32::try_from(width).ok()?, u32::try_from(height).ok()?),
            values,
        ))
    }

    pub(super) fn build(&self, sea: f32) -> Option<Mesh> {
        if self.columns.len() > 32_768 || !sea.is_finite() {
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
        // All liquid intervals above participate in occlusion, including raised
        // rivers at a sea junction. Emit only sea boundaries: the ordinary
        // liquid mesh owns raised-water bottoms and sides as well as their tops.
        for column in self
            .columns
            .iter()
            .filter(|column| at_sea_level(column.top, sea))
        {
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
    fn bounded_mask_distinguishes_wet_dry_and_unknown_without_filling_carves() {
        let wet = HexCoord::from_axial(-2, 3);
        let dry = HexCoord::from_axial(0, 3);
        let mut boundary = OceanNearBoundary {
            columns: vec![OceanBoundaryColumn {
                coordinate: wet,
                bottom: -3.0,
                top: 0.0,
            }],
            known_columns: [wet, dry].into_iter().collect(),
            ..default()
        };
        let (origin, size, values) = boundary.mask(0.0).unwrap();
        assert_eq!(origin, IVec2::new(-2, 3));
        assert_eq!(size, UVec2::new(3, 1));
        assert!(values[0] > 0.5 && values[1].abs() < 0.01 && values[2] < -0.5);
        boundary.known_columns.insert(HexCoord::from_axial(600, 3));
        assert!(boundary.mask(0.0).is_none());
    }
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
        assert_eq!(boundary.build(0.0).unwrap().count_vertices(), 7);
        boundary.known_columns = HexCoord::ORIGIN.within_radius(1).into_iter().collect();
        assert_eq!(boundary.build(0.0).unwrap().count_vertices(), 31);
        boundary.columns.push(OceanBoundaryColumn {
            coordinate: HexCoord::from_axial(1, 0),
            ..column
        });
        boundary
            .known_columns
            .extend(HexCoord::from_axial(1, 0).within_radius(1));
        let indices = boundary.build(0.0).unwrap().indices().unwrap().len();
        assert_eq!(
            indices, 96,
            "two bottoms and ten exterior sides; no shared water face"
        );
    }

    #[test]
    fn raised_water_occludes_sea_faces_without_emitting_duplicate_boundaries(
    ) -> Result<(), &'static str> {
        let sea = OceanBoundaryColumn {
            coordinate: HexCoord::ORIGIN,
            bottom: 138.6,
            top: 140.0,
        };
        let raised = OceanBoundaryColumn {
            coordinate: HexCoord::from_axial(1, 0),
            // The first raised Grand level, 401 * 0.35, belongs to the flat mesh.
            top: 140.35,
            ..sea
        };
        let boundary = OceanNearBoundary {
            columns: vec![sea, raised],
            known_columns: HexCoord::ORIGIN.within_radius(1).into_iter().collect(),
            ..default()
        };
        let mesh = boundary.build(140.0).ok_or("valid mixed water boundary")?;
        assert_eq!(mesh.count_vertices(), 27, "one bottom and five sea sides");
        assert_eq!(mesh.indices().ok_or("boundary indices")?.len(), 48);
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            return Err("boundary positions");
        };
        assert!(positions
            .iter()
            .all(|&[x, y, z]| { x.hypot(z) <= 1.000_001 && (sea.bottom..=sea.top).contains(&y) }));
        let (_, _, mask) = boundary.mask(140.0).ok_or("valid mixed water mask")?;
        assert_eq!(mask.iter().filter(|value| **value > 0.5).count(), 1);

        let inland_only = OceanNearBoundary {
            columns: vec![raised],
            ..boundary
        };
        assert_eq!(
            inland_only
                .build(140.0)
                .ok_or("valid inland coverage")?
                .count_vertices(),
            0,
            "inland geometry has exactly one renderer"
        );
        assert!(inland_only.build(f32::NAN).is_none());
        Ok(())
    }
}
