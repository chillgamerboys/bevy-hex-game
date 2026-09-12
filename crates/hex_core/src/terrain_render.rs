//! Disposable terrain-render batching and exact pointer-hit projection.
//!
//! Gameplay continues to address one logical [`HexTile`](crate::HexTile) entity per
//! material run. Rendering is free to combine many of those runs into one mesh, so a
//! pointer backend reports the batch entity rather than the logical run it hit. This
//! module is the narrow shared adapter between those two representations.

use bevy_ecs::{entity::Entity, prelude::Component};
use bevy_math::Vec3;

use crate::{HexCoord, HexSpan, SubstanceId, TerrainChunkRoot, TilePos};

/// Maximum number of exact logical runs represented by one combined terrain mesh.
///
/// Keeping the bound next to the shared lookup type prevents presentation builders
/// from silently creating an unbounded linear pointer-hit search.
pub const MAX_TERRAIN_PICK_RUNS_PER_BATCH: usize = 512;

/// One logical run represented by a combined terrain mesh.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainPickRun {
    entity: Entity,
    position: TilePos,
    span: HexSpan,
}

impl TerrainPickRun {
    /// Records the exact logical entity and world-space vertical span of one run.
    #[must_use]
    pub const fn new(entity: Entity, position: TilePos, span: HexSpan) -> Self {
        Self {
            entity,
            position,
            span,
        }
    }

    /// The lightweight [`crate::HexTile`] entity carrying gameplay's run tuple.
    #[must_use]
    pub const fn entity(self) -> Entity {
        self.entity
    }

    /// The run's topmost material voxel.
    #[must_use]
    pub const fn position(self) -> TilePos {
        self.position
    }

    /// The run's exact world-space vertical extent.
    #[must_use]
    pub const fn span(self) -> HexSpan {
        self.span
    }
}

/// A bounded combined solid-terrain mesh inside one resident chunk.
///
/// The component contains only disposable presentation lookup data. Chunk
/// coordinates and entity ids never enter saves, snapshots, map fingerprints, or
/// authoritative terrain addressing.
#[derive(Component, Debug, Clone)]
pub struct TerrainRenderBatch {
    chunk: TerrainChunkRoot,
    substance: SubstanceId,
    runs: Vec<TerrainPickRun>,
}

impl TerrainRenderBatch {
    /// Creates a batch from one deterministic, non-empty run partition.
    ///
    /// # Panics
    ///
    /// Panics when `runs` is empty or exceeds
    /// [`MAX_TERRAIN_PICK_RUNS_PER_BATCH`]. An empty mesh has no pick or
    /// presentation meaning, while an oversized partition breaks the bounded
    /// exact-hit contract.
    #[must_use]
    pub fn new(
        chunk: TerrainChunkRoot,
        substance: SubstanceId,
        mut runs: Vec<TerrainPickRun>,
    ) -> Self {
        assert!(!runs.is_empty(), "a terrain render batch cannot be empty");
        assert!(
            runs.len() <= MAX_TERRAIN_PICK_RUNS_PER_BATCH,
            "a terrain render batch cannot exceed {MAX_TERRAIN_PICK_RUNS_PER_BATCH} runs"
        );
        runs.sort_by_key(|run| (run.position, run.entity));
        Self {
            chunk,
            substance,
            runs,
        }
    }

    /// The resident presentation chunk that owns this mesh.
    #[must_use]
    pub const fn chunk(&self) -> TerrainChunkRoot {
        self.chunk
    }

    /// The single material shared by every prism in this mesh.
    #[must_use]
    pub const fn substance(&self) -> SubstanceId {
        self.substance
    }

    /// Exact logical runs represented by this mesh, in canonical order.
    pub fn runs(&self) -> impl ExactSizeIterator<Item = TerrainPickRun> + '_ {
        self.runs.iter().copied()
    }

    /// Whether this batch represents the exact logical surface.
    ///
    /// Pointer `Out` events are allowed to omit hit coordinates. Retaining this
    /// bounded membership check lets hover teardown remain exact in that case.
    #[must_use]
    pub fn contains_position(&self, position: TilePos) -> bool {
        self.runs.iter().any(|run| run.position == position)
    }

    /// Resolves one world-space mesh hit back to the exact logical terrain run.
    ///
    /// Mesh picking supplies world coordinates. Horizontal ownership follows the
    /// canonical hex rounding rule; vertical ownership uses the reported face normal
    /// and exact published span. Without a finite, non-zero normal, only an exact cap
    /// plane can prove ownership; an apparent side hit fails closed rather than
    /// choosing whichever side of a hex boundary happens to win floating-point
    /// rounding. A tiny tolerance covers renderer interpolation at a face boundary
    /// without allowing a hit to jump across an air gap.
    #[must_use]
    pub fn resolve_hit(&self, world_position: Vec3, world_normal: Option<Vec3>) -> Option<Entity> {
        const FACE_EPSILON: f32 = 0.002;
        let normal = world_normal
            .filter(|normal| normal.is_finite() && normal.length_squared() > f32::EPSILON)
            .map(Vec3::normalize);
        let normal_y = normal.map_or(0.0, |normal| normal.y);
        // A side-face hit lies on the exact boundary between two hexes. Move the
        // sample imperceptibly into the prism that owned the picked triangle before
        // applying canonical hex rounding; otherwise the grid tie-break may select
        // the neighbouring column, which is not necessarily in this material batch.
        let horizontal_sample = match normal {
            Some(normal) if normal_y.abs() < 0.5 => world_position - normal * FACE_EPSILON,
            Some(_) | None => world_position,
        };
        let rounded = HexCoord::from_world(horizontal_sample);

        self.runs
            .iter()
            .filter(|run| run.position.coord == rounded)
            .filter_map(|run| {
                let vertical_error = match normal {
                    Some(_) if normal_y > 0.5 => (world_position.y - run.span.top).abs(),
                    Some(_) if normal_y < -0.5 => (world_position.y - run.span.bottom).abs(),
                    Some(_) if world_position.y < run.span.bottom => {
                        run.span.bottom - world_position.y
                    }
                    Some(_) if world_position.y > run.span.top => world_position.y - run.span.top,
                    Some(_) => 0.0,
                    None => (world_position.y - run.span.top)
                        .abs()
                        .min((world_position.y - run.span.bottom).abs()),
                };
                (vertical_error <= FACE_EPSILON).then_some((
                    vertical_error.to_bits(),
                    run.position,
                    run.entity,
                ))
            })
            .min()
            .map(|(_error, _position, entity)| entity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(entity: u32, coord: HexCoord, level: i32, bottom: f32, top: f32) -> TerrainPickRun {
        TerrainPickRun::new(
            Entity::from_raw_u32(entity).expect("fixture entity should be valid"),
            TilePos::new(coord, level),
            HexSpan::new(bottom, top),
        )
    }

    #[test]
    fn exact_hit_resolution_preserves_stacked_run_identity() {
        let coord = HexCoord::from_axial(3, -2);
        let lower = run(1, coord, 3, 0.0, 1.6);
        let upper = run(2, coord, 11, 3.2, 4.8);
        let batch = TerrainRenderBatch::new(
            TerrainChunkRoot { q: 0, r: -1 },
            SubstanceId(4),
            vec![upper, lower],
        );
        let centre = coord.to_world(0.0);

        assert_eq!(
            batch.resolve_hit(Vec3::new(centre.x, 1.6, centre.z), Some(Vec3::Y)),
            Some(lower.entity())
        );
        assert_eq!(
            batch.resolve_hit(Vec3::new(centre.x, 4.0, centre.z), Some(Vec3::X)),
            Some(upper.entity())
        );
        assert_eq!(
            batch.resolve_hit(Vec3::new(centre.x, 2.4, centre.z), Some(Vec3::X)),
            None,
            "an air gap must not resolve to either stacked run"
        );
    }

    #[test]
    fn hit_resolution_does_not_cross_to_an_adjacent_hex() {
        let origin = HexCoord::ORIGIN;
        let neighbour = HexCoord::from_axial(1, 0);
        let origin_run = run(3, origin, 0, 0.0, 0.4);
        let batch = TerrainRenderBatch::new(
            TerrainChunkRoot { q: 0, r: 0 },
            SubstanceId(1),
            vec![origin_run],
        );
        let neighbour_centre = neighbour.to_world(0.4);

        assert_eq!(batch.resolve_hit(neighbour_centre, Some(Vec3::Y)), None);
        assert!(batch.contains_position(origin_run.position()));
        assert!(!batch.contains_position(TilePos::new(neighbour, 0)));
    }

    #[test]
    fn side_face_boundary_is_nudged_into_the_triangle_owner() {
        let origin = HexCoord::ORIGIN;
        let origin_run = run(4, origin, 3, 0.0, 1.6);
        let batch = TerrainRenderBatch::new(
            TerrainChunkRoot { q: 0, r: 0 },
            SubstanceId(1),
            vec![origin_run],
        );
        // Mesh interpolation can place a nominal boundary hit a fraction outside.
        let hit = Vec3::new(0.5 * crate::config::HEX_SMALL_DIAMETER + 0.000_1, 0.8, 0.0);

        assert_eq!(
            batch.resolve_hit(hit, Some(Vec3::X)),
            Some(origin_run.entity())
        );
    }

    #[test]
    fn missing_normal_resolves_a_cap_but_rejects_an_unprovable_side() {
        let coord = HexCoord::ORIGIN;
        let exact = run(5, coord, 3, 0.0, 1.6);
        let batch =
            TerrainRenderBatch::new(TerrainChunkRoot { q: 0, r: 0 }, SubstanceId(1), vec![exact]);
        let centre = coord.to_world(0.0);

        assert_eq!(
            batch.resolve_hit(Vec3::new(centre.x, exact.span().top, centre.z), None),
            Some(exact.entity()),
            "the exact cap plane proves vertical ownership without a normal"
        );
        assert_eq!(
            batch.resolve_hit(
                Vec3::new(
                    centre.x + 0.5 * crate::config::HEX_SMALL_DIAMETER,
                    exact.span().centre(),
                    centre.z,
                ),
                None,
            ),
            None,
            "a normal-free side hit must not guess across a hex boundary"
        );
    }

    #[test]
    #[should_panic(expected = "a terrain render batch cannot exceed 512 runs")]
    fn oversized_batches_fail_at_the_shared_contract_boundary() {
        let runs = (0..=MAX_TERRAIN_PICK_RUNS_PER_BATCH)
            .map(|index| {
                run(
                    u32::try_from(index + 1).expect("fixture entity should fit u32"),
                    HexCoord::from_axial(i32::try_from(index).expect("fixture coordinate"), 0),
                    0,
                    0.0,
                    0.4,
                )
            })
            .collect();
        let _batch = TerrainRenderBatch::new(TerrainChunkRoot { q: 0, r: 0 }, SubstanceId(1), runs);
    }
}
