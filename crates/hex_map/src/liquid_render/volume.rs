//! Closed exterior boundary of the actual liquid union, including buried faces.
//! Internal faces are removed against the complete map, across render partitions.
use super::*;

pub(super) fn water_boundary(
    map: &VoxelMap,
    water: SubstanceId,
    height: f32,
    fountains: &FountainWater,
) -> Result<BTreeMap<LiquidCapBatchKey, RawMesh>, LiquidPresentationError> {
    let mut batches = BTreeMap::<LiquidCapBatchKey, RawMesh>::new();
    for (coord, column) in map.columns() {
        for run in runs(column)
            .into_iter()
            .filter(|run| run.substance == water)
        {
            let surface = LiquidSurface {
                position: TilePos::new(coord, run.top - 1),
                role: FillMaterialRole::Water,
                flow: LiquidFlowState::Still,
                downstream: None,
            };
            let key = LiquidCapBatchKey {
                chunk: terrain_chunk_coord(coord),
                role: surface.role,
                fountain: fountains.group(surface),
            };
            let mesh = batches.entry(key).or_default();
            if column.get(run.top) != water {
                append_cap(mesh, coord, run.top, height, false)?;
            }
            if column.get(run.bottom - 1) != water {
                append_cap(mesh, coord, run.bottom, height, true)?;
            }
            for side in HexSide::ALL {
                let neighbor = side.neighbor(coord);
                let mut start = None;
                for level in run.bottom..=run.top {
                    let exposed =
                        level < run.top && map.get(TilePos::new(neighbor, level)) != water;
                    if exposed && start.is_none() {
                        start = Some(level);
                    }
                    if !exposed {
                        if let Some(bottom) = start.take() {
                            append_side(mesh, coord, side, bottom, level, height)?;
                        }
                    }
                }
            }
        }
    }
    for mesh in batches.values() {
        mesh.validate_finite()?;
    }
    Ok(batches)
}

#[expect(
    clippy::cast_precision_loss,
    reason = "finite voxel levels are exactly representable"
)]
fn append_cap(
    mesh: &mut RawMesh,
    coord: HexCoord,
    level: i32,
    height: f32,
    bottom: bool,
) -> Result<(), LiquidPresentationError> {
    let cap = cap_geometry();
    let base = u32::try_from(mesh.positions.len())
        .map_err(|_| LiquidPresentationError::MeshIndexOverflow)?;
    let center = coord.to_world(level as f32 * height);
    for position in &cap.positions {
        let p = center + Vec3::from_array(*position);
        mesh.positions.push(p.to_array());
        mesh.normals.push(if bottom {
            [0.0, -1.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        });
        mesh.uvs.push(continuous_cap_uv(p));
    }
    for triangle in cap.indices.chunks_exact(3) {
        let Some((&a, rest)) = triangle.split_first() else {
            continue;
        };
        let (Some(&b), Some(&c)) = (rest.first(), rest.get(1)) else {
            continue;
        };
        mesh.indices.extend(if bottom {
            [base + a, base + c, base + b]
        } else {
            [base + a, base + b, base + c]
        });
    }
    Ok(())
}

#[expect(
    clippy::cast_precision_loss,
    reason = "finite voxel levels are exactly representable"
)]
fn append_side(
    mesh: &mut RawMesh,
    coord: HexCoord,
    side: HexSide,
    bottom: i32,
    top: i32,
    height: f32,
) -> Result<(), LiquidPresentationError> {
    let base = u32::try_from(mesh.positions.len())
        .map_err(|_| LiquidPresentationError::MeshIndexOverflow)?;
    let rotation = side_rotation(side);
    let center = coord.to_world(0.0);
    let normal = rotation * Vec3::X;
    for (z, y) in [(-0.5, top), (0.5, top), (0.5, bottom), (-0.5, bottom)] {
        let p =
            center + rotation * Vec3::new(HEX_INRADIUS, y as f32 * height, z * HEX_CIRCUMRADIUS);
        mesh.positions.push(p.to_array());
        mesh.normals.push(normal.to_array());
        mesh.uvs.push([z, y as f32 * height]);
    }
    mesh.indices
        .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adjacent_water_culls_shared_faces_across_render_chunks() {
        let mut map = VoxelMap::new();
        let water = super::super::tests::liquid_table()
            .id("water")
            .expect("water");
        for q in [15, 16] {
            for level in 3..6 {
                map.set(TilePos::new(HexCoord::from_axial(q, 0), level), water);
            }
        }
        let mesh = water_boundary(&map, water, 0.35, &FountainWater::default()).expect("volume");
        assert_eq!(mesh.values().map(|m| m.indices.len()).sum::<usize>(), 132);
    }

    #[test]
    fn stacked_water_has_only_two_caps_and_six_outer_walls() {
        let mut map = VoxelMap::new();
        let water = super::super::tests::liquid_table()
            .id("water")
            .expect("water");
        for level in 3..6 {
            map.set(TilePos::new(HexCoord::ORIGIN, level), water);
        }
        let mesh = water_boundary(&map, water, 0.35, &FountainWater::default()).expect("volume");
        assert_eq!(mesh.values().map(|m| m.indices.len()).sum::<usize>(), 72);
        let low = mesh
            .values()
            .flat_map(|m| &m.positions)
            .map(|[_, y, _]| *y)
            .fold(f32::INFINITY, f32::min);
        assert!((low - 1.05).abs() < 0.001);
    }
}
