//! Immutable side-occlusion context; no neighboring logical or rendered entities.

use std::{collections::BTreeSet, sync::Arc};

use hex_world_contracts::{
    hash_serializable, ChunkId, ChunkPackage, ColumnData, VoxelRun, WorldHex,
};

use super::{
    prepare::{render_intervals, union_runs, validate_suppression},
    PresentationError, TerrainPreparer,
};

/// Number of outside columns adjacent to a 16-by-16 axial storage rectangle.
pub const MAX_RENDER_HALO_COLUMNS: usize = 66;

/// A source actually selected for neighboring presentation, with its exact art mask.
/// Availability and atomic presentation lifecycle remain the application's concern.
#[derive(Clone)]
pub struct RenderNeighbor {
    /// Immutable source used for this neighbor's presented geometry.
    pub package: Arc<ChunkPackage>,
    /// Presented authority revision; never substitute a newer unpresented revision.
    pub revision: u64,
    /// Exact occupancy suppressed by this neighbor's published stock-art fragments.
    pub suppression: Arc<Vec<ColumnData>>,
}

/// Nonrecursive source identity for one neighboring presentation dependency.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderHaloDependency {
    /// Neighboring storage coordinate.
    pub coordinate: ChunkId,
    /// Presented source revision.
    pub revision: u64,
    /// Presented payload checksum.
    pub package_fingerprint: u64,
    /// Presented stock-art occupancy mask checksum.
    pub suppression_fingerprint: u64,
}

/// Validated global one-hex context, reusable across render-origin rebases.
/// Construction is private; it contains only clipped effective render intervals.
#[derive(Clone)]
pub struct RenderHalo {
    owner: ChunkId,
    manifest_fingerprint: u64,
    columns: Arc<Vec<ColumnData>>,
    dependencies: Arc<Vec<RenderHaloDependency>>,
    fingerprint: u64,
    run_count: usize,
}

impl RenderHalo {
    /// Complete canonical context signature, including explicit neighbor presence.
    #[must_use]
    pub const fn fingerprint(&self) -> u64 {
        self.fingerprint
    }

    /// Sorted nonrecursive source dependencies for application freshness checks.
    #[must_use]
    pub fn dependencies(&self) -> &[RenderHaloDependency] {
        &self.dependencies
    }

    /// Number of outside border columns retained, at most 66.
    #[must_use]
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }

    pub(super) fn columns(&self) -> &[ColumnData] {
        &self.columns
    }

    pub(super) fn validate_context(
        &self,
        owner: ChunkId,
        manifest_fingerprint: u64,
        max_runs: usize,
    ) -> Result<(), PresentationError> {
        if self.owner != owner || self.manifest_fingerprint != manifest_fingerprint {
            return Err(PresentationError(
                "render halo belongs to another owner or world".into(),
            ));
        }
        if self.run_count > max_runs {
            return Err(PresentationError(
                "max_runs_per_halo presentation budget exceeded".into(),
            ));
        }
        Ok(())
    }
}

impl TerrainPreparer {
    /// Validate up to six distinct adjacent presentation sources and retain only
    /// their one-hex border. Missing neighbors have no occluding intervals, so the
    /// owner remains closed until the caller publishes a replacement with context.
    pub fn render_halo(
        &self,
        owner: ChunkId,
        neighbors: &[RenderNeighbor],
    ) -> Result<RenderHalo, PresentationError> {
        let origin = owner.origin()?;
        if !self.index.contains_chunk(owner) {
            return Err(PresentationError(
                "render halo owner is outside world".into(),
            ));
        }
        if neighbors.len() > 6 {
            return Err(PresentationError(
                "render halo exceeds six neighboring chunks".into(),
            ));
        }
        let mut seen = BTreeSet::new();
        let mut dependencies = Vec::new();
        let mut columns = Vec::new();
        let mut run_count = 0usize;
        for neighbor in neighbors {
            let package = &neighbor.package;
            let dq = i128::from(package.coordinate.q) - i128::from(owner.q);
            let dr = i128::from(package.coordinate.r) - i128::from(owner.r);
            if !matches!(
                (dq, dr),
                (1, 0) | (0, 1) | (-1, 1) | (-1, 0) | (0, -1) | (1, -1)
            ) || !seen.insert(package.coordinate)
            {
                return Err(PresentationError(
                    "render halo sources must be unique adjacent chunks".into(),
                ));
            }
            package.validate_with_index(&self.index)?;
            validate_suppression(
                package,
                &neighbor.suppression,
                self.limits.max_runs_per_chunk,
            )?;
            dependencies.push(RenderHaloDependency {
                coordinate: package.coordinate,
                revision: neighbor.revision,
                package_fingerprint: package.fingerprint,
                suppression_fingerprint: hash_serializable(neighbor.suppression.as_slice())?,
            });
            for column in &package.columns {
                if !adjacent_column(origin, column.position) {
                    continue;
                }
                let occupancy = package
                    .semantics
                    .occupancy
                    .binary_search_by_key(&column.position, |entry| entry.position)
                    .ok()
                    .and_then(|index| package.semantics.occupancy.get(index))
                    .map_or(&[][..], |entry| entry.runs.as_slice());
                let mut runs = Vec::new();
                for (run, source) in union_runs(&column.runs, occupancy)? {
                    for (bottom, top) in render_intervals(
                        column.position,
                        run.bottom,
                        run.top,
                        source,
                        &neighbor.suppression,
                    ) {
                        run_count = run_count.checked_add(1).ok_or_else(|| {
                            PresentationError("render halo interval count overflow".into())
                        })?;
                        if run_count > self.limits.max_runs_per_halo {
                            return Err(PresentationError(
                                "max_runs_per_halo presentation budget exceeded".into(),
                            ));
                        }
                        runs.push(VoxelRun {
                            bottom,
                            top,
                            material: run.material.clone(),
                        });
                    }
                }
                columns.push(ColumnData {
                    position: column.position,
                    runs,
                });
            }
        }
        columns.sort_by_key(|column| column.position);
        if columns.len() > MAX_RENDER_HALO_COLUMNS {
            return Err(PresentationError(
                "render halo exceeds its one-hex border".into(),
            ));
        }
        dependencies.sort_by_key(|dependency| dependency.coordinate);
        let fingerprint = hash_serializable(&(
            "render-halo-v1",
            self.manifest.fingerprint,
            owner,
            &columns,
            dependencies
                .iter()
                .map(|d| {
                    (
                        d.coordinate,
                        d.revision,
                        d.package_fingerprint,
                        d.suppression_fingerprint,
                    )
                })
                .collect::<Vec<_>>(),
        ))?;
        Ok(RenderHalo {
            owner,
            manifest_fingerprint: self.manifest.fingerprint,
            columns: Arc::new(columns),
            dependencies: Arc::new(dependencies),
            fingerprint,
            run_count,
        })
    }
}

fn adjacent_column(origin: WorldHex, position: WorldHex) -> bool {
    let q = i128::from(position.q) - i128::from(origin.q);
    let r = i128::from(position.r) - i128::from(origin.r);
    if (0..16).contains(&q) && (0..16).contains(&r) {
        return false;
    }
    [(1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1)]
        .into_iter()
        .any(|(dq, dr)| (0..16).contains(&(q + dq)) && (0..16).contains(&(r + dr)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_hex_halo_has_exactly_66_columns_and_excludes_diagonal_corners() {
        let origin = WorldHex::new(0, 0);
        let columns = (-2..18)
            .flat_map(|q| (-2..18).map(move |r| WorldHex::new(q, r)))
            .filter(|position| adjacent_column(origin, *position))
            .collect::<BTreeSet<_>>();
        assert_eq!(columns.len(), MAX_RENDER_HALO_COLUMNS);
        assert!(!columns.contains(&WorldHex::new(-1, -1)));
        assert!(!columns.contains(&WorldHex::new(16, 16)));
        assert!(columns.contains(&WorldHex::new(-1, 16)));
        assert!(columns.contains(&WorldHex::new(16, -1)));
        assert!(!columns.contains(&WorldHex::new(0, 0)));
    }
}
