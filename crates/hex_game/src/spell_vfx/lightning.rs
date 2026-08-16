//! Procedural lightning geometry for [`hex_assets::MotionArchetype::Arc`].
//!
//! The path is built by **recursive midpoint displacement**: start with the straight
//! segment from caster to target, push its midpoint sideways by a random amount,
//! then recurse into each half with the displacement halved. A few short forked
//! branches spur off the result.
//!
//! Re-rolled on every cast rather than cached, which is the whole point. A painted
//! lightning texture reads as fake because real lightning never strikes the same
//! shape twice, and a bolt that replays an identical silhouette is immediately
//! recognisable as a decal. The mesh is cheap — a few hundred vertices — and dies
//! with the effect entity a fraction of a second later.
//!
//! The bolt is drawn as a **cross ribbon**: two flat strips sharing the path, at
//! right angles to each other. That keeps it readable from any camera angle without
//! billboarding, which matters because a single flat strip viewed edge-on vanishes —
//! the same failure the particles had before they were oriented.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use rand::Rng;

/// Builds one bolt's mesh, in world space, from `start` to `end`.
///
/// The returned mesh is positioned absolutely, so its entity carries an identity
/// transform: a jagged path has no meaningful local origin to scale or rotate about.
pub(super) fn build_arc_mesh(
    start: Vec3,
    end: Vec3,
    thickness: f32,
    displacement: f32,
    subdivisions: u32,
    branches: u32,
    rng: &mut impl Rng,
) -> Mesh {
    let path = jagged_path(start, end, displacement, subdivisions, rng);
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();

    append_ribbon(
        &path,
        thickness,
        &mut positions,
        &mut normals,
        &mut uvs,
        &mut indices,
    );
    for spur in fork_branches(&path, displacement, branches, rng) {
        // Branches are thinner than the trunk they leave, the way a real fork is.
        append_ribbon(
            &spur,
            thickness * 0.55,
            &mut positions,
            &mut normals,
            &mut uvs,
            &mut indices,
        );
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Recursive midpoint displacement between two points.
///
/// Each pass doubles the segment count and halves the displacement, so early passes
/// set the bolt's overall crookedness and later ones add fine crackle.
fn jagged_path(
    start: Vec3,
    end: Vec3,
    displacement: f32,
    subdivisions: u32,
    rng: &mut impl Rng,
) -> Vec<Vec3> {
    let mut points = vec![start, end];
    let mut amplitude = displacement;
    for _ in 0..subdivisions {
        let mut next = Vec::with_capacity(points.len() * 2);
        for pair in points.windows(2) {
            let [a, b] = pair else { continue };
            next.push(*a);
            next.push(displaced_midpoint(*a, *b, amplitude, rng));
        }
        if let Some(last) = points.last() {
            next.push(*last);
        }
        points = next;
        amplitude *= 0.5;
    }
    points
}

fn displaced_midpoint(a: Vec3, b: Vec3, amplitude: f32, rng: &mut impl Rng) -> Vec3 {
    let midpoint = a.midpoint(b);
    let Some(direction) = (b - a).try_normalize() else {
        return midpoint;
    };
    let (u, v) = perpendicular_basis(direction);
    // Displaced in a random direction within the plane perpendicular to the segment,
    // so the bolt wanders in 3D rather than staying inside one flat plane.
    let angle = rng.gen_range(0.0..std::f32::consts::TAU);
    let offset = (u * angle.cos() + v * angle.sin()) * rng.gen_range(-amplitude..amplitude);
    midpoint + offset
}

/// Short forks leaving the main path partway along, angled away from it.
fn fork_branches(
    path: &[Vec3],
    displacement: f32,
    branches: u32,
    rng: &mut impl Rng,
) -> Vec<Vec<Vec3>> {
    if path.len() < 3 || branches == 0 {
        return Vec::new();
    }
    let mut spurs = Vec::new();
    for _ in 0..branches {
        // Never the very first or last point: a fork off the caster's hand or
        // exactly at the impact reads as an error rather than a branch.
        let index = rng.gen_range(1..path.len() - 1);
        let (Some(&anchor), Some(&ahead)) = (path.get(index), path.get(index + 1)) else {
            continue;
        };
        let Some(direction) = (ahead - anchor).try_normalize() else {
            continue;
        };
        let (u, v) = perpendicular_basis(direction);
        let angle = rng.gen_range(0.0..std::f32::consts::TAU);
        let sideways = u * angle.cos() + v * angle.sin();
        // A fork keeps some of the parent's direction and veers off; a purely
        // perpendicular spur looks bolted on.
        let length = rng.gen_range(0.2..0.6) * displacement * 6.0;
        let tip = anchor + (direction * 0.4 + sideways).normalize_or_zero() * length;
        spurs.push(jagged_path(anchor, tip, displacement * 0.4, 2, rng));
    }
    spurs
}

/// Two unit vectors perpendicular to `direction` and to each other.
fn perpendicular_basis(direction: Vec3) -> (Vec3, Vec3) {
    // Cross with whichever axis the direction is least aligned to, so the cross
    // product never degenerates toward zero length.
    let seed = if direction.y.abs() < 0.9 {
        Vec3::Y
    } else {
        Vec3::X
    };
    let u = direction.cross(seed).normalize_or_zero();
    let v = direction.cross(u).normalize_or_zero();
    (u, v)
}

/// Appends a cross ribbon following `path` into the mesh buffers.
#[expect(
    clippy::cast_precision_loss,
    reason = "the index and segment count of one bolt's path; subdivisions are capped \
              at 10 (MAX_ARC_SUBDIVISIONS), so both stay under about 1024"
)]
fn append_ribbon(
    path: &[Vec3],
    thickness: f32,
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    uvs: &mut Vec<[f32; 2]>,
    indices: &mut Vec<u32>,
) {
    if path.len() < 2 {
        return;
    }
    let half = thickness * 0.5;
    let base = u32::try_from(positions.len()).unwrap_or(u32::MAX);

    for (index, point) in path.iter().enumerate() {
        let tangent = tangent_at(path, index);
        let (u, v) = perpendicular_basis(tangent);
        let along = index as f32 / (path.len() - 1) as f32;
        for (offset, normal) in [(u * half, v), (-u * half, v), (v * half, u), (-v * half, u)] {
            let position = *point + offset;
            positions.push(position.to_array());
            normals.push(normal.to_array());
            uvs.push([along, 0.5]);
        }
    }

    let segments = u32::try_from(path.len() - 1).unwrap_or(0);
    for segment in 0..segments {
        let here = base + segment * 4;
        let next = here + 4;
        // Two quads per segment: one spanning the `u` axis, one the `v` axis.
        for (a, b) in [(0, 1), (2, 3)] {
            indices.extend_from_slice(&[
                here + a,
                here + b,
                next + b,
                here + a,
                next + b,
                next + a,
            ]);
        }
    }
}

/// The path direction at `index`, averaged across the joint so corners do not pinch.
fn tangent_at(path: &[Vec3], index: usize) -> Vec3 {
    let previous = index.saturating_sub(1);
    let next = (index + 1).min(path.len() - 1);
    let (Some(&before), Some(&after)) = (path.get(previous), path.get(next)) else {
        return Vec3::Z;
    };
    (after - before).try_normalize().unwrap_or(Vec3::Z)
}

#[cfg(test)]
mod tests {
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    use super::*;

    fn rng() -> StdRng {
        StdRng::seed_from_u64(7)
    }

    #[test]
    fn each_subdivision_doubles_the_segment_count() {
        let mut rng = rng();
        for subdivisions in 0..6 {
            let path = jagged_path(Vec3::ZERO, Vec3::X * 6.0, 0.4, subdivisions, &mut rng);
            assert_eq!(path.len(), (1 << subdivisions) + 1);
        }
    }

    /// The bolt has to start in the caster's hand and land on the target; only the
    /// middle is free to wander.
    #[test]
    fn the_path_still_connects_its_endpoints() {
        let mut rng = rng();
        let start = Vec3::new(-3.0, 0.85, 0.0);
        let end = Vec3::new(3.0, 0.85, 0.0);
        let path = jagged_path(start, end, 0.5, 5, &mut rng);
        assert_eq!(path.first(), Some(&start));
        assert_eq!(path.last(), Some(&end));
    }

    /// The reason this is generated rather than cached: a bolt that replayed one
    /// silhouette would read as a decal.
    #[test]
    fn two_casts_produce_different_paths() {
        let mut rng = rng();
        let first = jagged_path(Vec3::ZERO, Vec3::X * 6.0, 0.5, 4, &mut rng);
        let second = jagged_path(Vec3::ZERO, Vec3::X * 6.0, 0.5, 4, &mut rng);
        assert_ne!(first, second);
    }

    #[test]
    fn a_degenerate_zero_length_bolt_does_not_produce_nan() {
        let mut rng = rng();
        let path = jagged_path(Vec3::ZERO, Vec3::ZERO, 0.5, 4, &mut rng);
        assert!(path.iter().all(|point| point.is_finite()));
    }

    #[test]
    fn the_mesh_is_a_closed_indexed_triangle_list() {
        let mut rng = rng();
        let mesh = build_arc_mesh(Vec3::ZERO, Vec3::X * 6.0, 0.1, 0.4, 4, 3, &mut rng);
        let positions = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .expect("the bolt mesh has positions")
            .len();
        let indices = mesh.indices().expect("the bolt mesh is indexed");
        assert_eq!(indices.len() % 3, 0, "triangles come in threes");
        assert!(
            indices.iter().all(|index| index < positions),
            "every index must address a vertex that exists"
        );
    }

    #[test]
    fn every_generated_vertex_is_finite() {
        let mut rng = rng();
        let mesh = build_arc_mesh(Vec3::ZERO, Vec3::Y * 4.0, 0.1, 0.4, 5, 4, &mut rng);
        let bevy::mesh::VertexAttributeValues::Float32x3(positions) = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .expect("the bolt mesh has positions")
        else {
            panic!("positions are 3-component floats");
        };
        assert!(positions
            .iter()
            .all(|position| position.iter().all(|value| value.is_finite())));
    }
}
