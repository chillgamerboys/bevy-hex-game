//! Run-local finite edits over immutable, strictly validated source packages.
//!
//! Unlike persistent world editing this explicit mode permits broken supports,
//! boundaries and anchors after play starts. Liquid topology and the finite
//! vertical/column bounds remain immutable. A single cell removes every solid
//! contributor there, so overlapping objects cannot leave an invisible collider.
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use crate::{RuntimeError, RuntimeResult, WorldRuntime};
use hex_world_contracts::{
    hash_serializable, ChunkId, ChunkPackage, ColumnData, MaterialSpec, VoxelPosition, VoxelRun,
    WorldChange, WorldEditTransaction, WorldHex,
};

/// Explicit session authority for a finite destructible world, resident or streamed.
/// Source packages/blueprints remain unchanged; only edited cells consume storage.
/// Dropping this value restores the source. It intentionally has no save method.
#[derive(Debug)]
pub struct FiniteWorldSession {
    sources: BTreeMap<ChunkId, Arc<ChunkPackage>>,
    materials: BTreeMap<String, MaterialSpec>,
    revisions: BTreeMap<ChunkId, u64>,
    terrain_edits: BTreeMap<VoxelPosition, Option<String>>,
    object_removed: BTreeSet<VoxelPosition>,
    transactions: BTreeMap<String, (u64, WorldChange)>,
    bottom: i32,
    top: i32,
}

impl FiniteWorldSession {
    /// Create a run-local edit overlay whose immutable sources follow residency.
    /// Edits and partition revisions survive retirement; source bytes do not.
    pub fn streamed(runtime: &WorldRuntime, bottom: i32, top: i32) -> RuntimeResult<Self> {
        if bottom > top || top == i32::MAX {
            return Err(RuntimeError::invalid("invalid streamed edit height bounds"));
        }
        let mut result = Self {
            sources: BTreeMap::new(),
            materials: runtime
                .manifest()
                .materials
                .iter()
                .map(|m| (m.id.clone(), m.clone()))
                .collect(),
            revisions: BTreeMap::new(),
            terrain_edits: BTreeMap::new(),
            object_removed: BTreeSet::new(),
            transactions: BTreeMap::new(),
            bottom,
            top,
        };
        result.sync_residency(runtime);
        Ok(result)
    }

    /// Retire unloaded source payloads without discarding sparse edits or revisions.
    pub fn sync_residency(&mut self, runtime: &WorldRuntime) {
        let resident: BTreeSet<_> = runtime.resident_chunks().map(|p| p.coordinate).collect();
        self.sources.retain(|chunk, _| resident.contains(chunk));
        for product in runtime.resident_chunks() {
            self.sources
                .entry(product.coordinate)
                .or_insert_with(|| product.package.clone());
            self.revisions
                .entry(product.coordinate)
                .or_insert(product.revision);
        }
    }

    /// Number of immutable fine source payloads currently retained by this overlay.
    #[must_use]
    pub fn resident_source_count(&self) -> usize {
        self.sources.len()
    }

    /// Current clipped object intervals, with cut cells removed independently of terrain.
    #[must_use]
    pub fn object_column(&self, column: WorldHex) -> Option<ColumnData> {
        let source = self.source_column(column, true)?;
        let removed: BTreeSet<_> = self.removed_in_column(column).map(|p| p.level).collect();
        if removed.is_empty() {
            return Some(source.clone());
        }
        let mut runs = Vec::new();
        for run in &source.runs {
            let mut start = run.bottom;
            for level in removed.range(run.bottom..run.top) {
                if start < *level {
                    runs.push(VoxelRun {
                        bottom: start,
                        top: *level,
                        material: run.material.clone(),
                    });
                }
                start = level + 1;
            }
            if start < run.top {
                runs.push(VoxelRun {
                    bottom: start,
                    top: run.top,
                    material: run.material.clone(),
                });
            }
        }
        Some(ColumnData {
            position: column,
            runs,
        })
    }

    /// Produce exact current occupancy for disposable voxel rendering.
    /// Broken authored supports deliberately cease to be live semantic promises.
    /// Immutable source packages were fully validated before this session began.
    pub fn presentation_package(&self, chunk: ChunkId) -> RuntimeResult<Arc<ChunkPackage>> {
        let source = self
            .sources
            .get(&chunk)
            .ok_or_else(|| RuntimeError::invalid("render source is not resident"))?;
        if self.revision(chunk) == Some(0) {
            return Ok(source.clone());
        }
        let mut package = (**source).clone();
        package.columns = source
            .columns
            .iter()
            .filter_map(|column| self.terrain_column(column.position))
            .collect();
        // Rendering damaged objects as exact colored intervals preserves all surviving cells.
        // The original object descriptors remain owned by the immutable source.
        for column in &mut package.columns {
            let object = self.object_column(column.position);
            if let Some(object) = object {
                let mut bounds: BTreeSet<_> = column
                    .runs
                    .iter()
                    .chain(&object.runs)
                    .flat_map(|r| [r.bottom, r.top])
                    .collect();
                let sorted: Vec<_> = std::mem::take(&mut bounds).into_iter().collect();
                let mut merged: Vec<VoxelRun> = Vec::new();
                for pair in sorted.windows(2) {
                    let [bottom, top] = *pair else {
                        continue;
                    };
                    let material = column
                        .material_at(bottom)
                        .or_else(|| object.material_at(bottom));
                    let Some(material) = material else {
                        continue;
                    };
                    if let Some(last) = merged
                        .last_mut()
                        .filter(|last| last.top == bottom && last.material == material)
                    {
                        last.top = top;
                    } else {
                        merged.push(VoxelRun {
                            bottom,
                            top,
                            material: material.to_owned(),
                        });
                    }
                }
                column.runs = merged;
            }
        }
        package.semantics.occupancy.clear();
        package.semantics.objects.clear();
        package.semantics.object_influences.clear();
        package.semantics.anchors.clear();
        package.semantics.interiors.clear();
        package.semantics.lights.clear();
        package.semantics.light_influences.clear();
        package.seal().map_err(RuntimeError::invalid)?;
        Ok(Arc::new(package))
    }

    /// Pin immutable products from a fully resident runtime, with inclusive bounds.
    /// Strict source validation has already run before any relaxed live edit exists.
    pub fn new(runtime: &WorldRuntime, bottom: i32, top: i32) -> RuntimeResult<Self> {
        if bottom > top || top == i32::MAX {
            return Err(RuntimeError::invalid("invalid finite edit height bounds"));
        }
        let products: Vec<_> = runtime.resident_chunks().collect();
        if products.len() != runtime.manifest().chunks.len() {
            return Err(RuntimeError::invalid(
                "finite edit mode requires the entire source resident",
            ));
        }
        Ok(Self {
            sources: products
                .iter()
                .map(|p| (p.coordinate, p.package.clone()))
                .collect(),
            revisions: products
                .iter()
                .map(|p| (p.coordinate, p.revision))
                .collect(),
            materials: runtime
                .manifest()
                .materials
                .iter()
                .map(|m| (m.id.clone(), m.clone()))
                .collect(),
            terrain_edits: BTreeMap::new(),
            object_removed: BTreeSet::new(),
            transactions: BTreeMap::new(),
            bottom,
            top,
        })
    }

    /// Current revision for this finite session's partition.
    #[must_use]
    pub fn revision(&self, chunk: ChunkId) -> Option<u64> {
        self.revisions.get(&chunk).copied()
    }

    fn source_column(&self, at: WorldHex, objects: bool) -> Option<&ColumnData> {
        let package = self.sources.get(&at.chunk())?;
        let columns = if objects {
            &package.semantics.occupancy
        } else {
            &package.columns
        };
        columns
            .binary_search_by_key(&at, |column| column.position)
            .ok()
            .and_then(|index| columns.get(index))
    }

    /// Current terrain material, without object occupancy.
    #[must_use]
    pub fn terrain_at(&self, at: VoxelPosition) -> Option<&str> {
        self.terrain_edits.get(&at).map_or_else(
            || self.source_column(at.column, false)?.material_at(at.level),
            |material| material.as_deref(),
        )
    }

    /// Current exact material with terrain taking precedence over object overlap.
    #[must_use]
    pub fn material_at(&self, at: VoxelPosition) -> Option<&str> {
        self.terrain_at(at).or_else(|| self.object_at(at))
    }

    /// Current object material, separate from mutable terrain assignments.
    #[must_use]
    pub fn object_at(&self, at: VoxelPosition) -> Option<&str> {
        (!self.object_removed.contains(&at))
            .then(|| self.source_column(at.column, true)?.material_at(at.level))
            .flatten()
    }

    /// Whether an original blueprint cell was carved, including a later terrain refill.
    #[must_use]
    pub fn object_removed(&self, at: VoxelPosition) -> bool {
        self.object_removed.contains(&at)
    }

    /// Sparse original-object removals in one column, for instance mask publication.
    pub fn removed_in_column(&self, column: WorldHex) -> impl Iterator<Item = VoxelPosition> + '_ {
        self.object_removed
            .range(
                VoxelPosition {
                    column,
                    level: i32::MIN,
                }..=VoxelPosition {
                    column,
                    level: i32::MAX,
                },
            )
            .copied()
    }

    /// Current terrain runs for one edited or untouched column, preserving source colors.
    #[must_use]
    pub fn terrain_column(&self, column: WorldHex) -> Option<ColumnData> {
        let source = self.source_column(column, false)?;
        let edits: Vec<_> = self
            .terrain_edits
            .range(
                VoxelPosition {
                    column,
                    level: i32::MIN,
                }..=VoxelPosition {
                    column,
                    level: i32::MAX,
                },
            )
            .collect();
        if edits.is_empty() {
            return Some(source.clone());
        }
        let last = source.runs.last().map_or(self.bottom, |run| run.top).max(
            edits
                .iter()
                .map(|(at, _)| at.level.saturating_add(1))
                .max()
                .unwrap_or(self.bottom),
        );
        let mut runs: Vec<VoxelRun> = Vec::new();
        for level in self.bottom..last {
            let Some(material) = self.terrain_at(VoxelPosition { column, level }) else {
                continue;
            };
            if let Some(run) = runs.last_mut() {
                if run.top == level && run.material == material {
                    run.top += 1;
                    continue;
                }
            }
            runs.push(VoxelRun {
                bottom: level,
                top: level + 1,
                material: material.to_owned(),
            });
        }
        Some(ColumnData {
            position: column,
            runs,
        })
    }

    /// Atomically apply exact terrain additions and mixed terrain/object carving.
    /// Existing liquids cannot be replaced, and new liquids cannot be constructed.
    /// Duplicate identities replay once; changed payloads or stale revisions reject.
    pub fn apply_transaction(
        &mut self,
        request: &WorldEditTransaction,
    ) -> RuntimeResult<WorldChange> {
        request.validate().map_err(RuntimeError::invalid)?;
        let fingerprint = hash_serializable(request).map_err(RuntimeError::invalid)?;
        if let Some((prior, result)) = self.transactions.get(&request.id) {
            return if *prior == fingerprint {
                Ok(result.clone())
            } else {
                Err(RuntimeError::invalid(
                    "finite transaction identity reused with different edits",
                ))
            };
        }
        for (chunk, expected) in &request.expected_revisions {
            if self.revision(*chunk) != Some(*expected) {
                return Err(RuntimeError::invalid("stale finite chunk revision"));
            }
        }
        for edit in &request.edits {
            if !(self.bottom..=self.top).contains(&edit.position.level)
                || self.source_column(edit.position.column, false).is_none()
            {
                return Err(RuntimeError::invalid("finite edit outside source bounds"));
            }
            if self
                .terrain_at(edit.position)
                .is_some_and(|name| self.materials.get(name).is_some_and(|m| !m.solid))
            {
                return Err(RuntimeError::invalid("finite liquid cells are immutable"));
            }
            if edit
                .material
                .as_ref()
                .is_some_and(|name| !self.materials.get(name).is_some_and(|m| m.solid))
            {
                return Err(RuntimeError::invalid(
                    "finite edits construct known solids only",
                ));
            }
        }
        let mut result = WorldChange {
            transaction_id: request.id.clone(),
            revisions: BTreeMap::new(),
            changed_columns: request
                .edits
                .iter()
                .map(|edit| edit.position.column)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
        };
        for edit in &request.edits {
            if self
                .source_column(edit.position.column, true)
                .and_then(|col| col.material_at(edit.position.level))
                .is_some()
            {
                self.object_removed.insert(edit.position);
            }
            self.terrain_edits
                .insert(edit.position, edit.material.clone());
        }
        for chunk in request.expected_revisions.keys() {
            let next = self.revision(*chunk).unwrap_or(0).saturating_add(1);
            self.revisions.insert(*chunk, next);
            result.revisions.insert(*chunk, next);
        }
        self.transactions
            .insert(request.id.clone(), (fingerprint, result.clone()));
        Ok(result)
    }
}
