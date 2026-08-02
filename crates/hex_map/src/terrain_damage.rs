//! Private voxel-health state and deterministic terrain-impact resolution.
//!
//! The public messages and read-only projection live in `hex_core`; this module keeps
//! the authoritative sparse ledger behind the map ownership boundary. Missing ledger
//! entries mean that the voxel is at its current material's full authored toughness.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use hex_assets::{SubstanceTable, TerrainDamageTable};
use hex_core::{
    DamagedVoxels, TerrainBatchId, TerrainImpact, TerrainImpactDisposition, TerrainImpactOutcome,
    TerrainImpactResult, TerrainVoxelHealth, TerrainVoxelOutcome, TilePos,
};

use crate::voxel::VoxelMap;

/// Session-local terrain damage state owned exclusively by `hex_map`.
#[derive(Resource, Debug, Default)]
pub(crate) struct TerrainDamageState {
    remaining: BTreeMap<TilePos, u8>,
    consumed_batches: BTreeSet<TerrainBatchId>,
}

impl TerrainDamageState {
    /// Claims the first processed use of a batch id, including rejected batches.
    pub(crate) fn consume_batch(&mut self, batch: TerrainBatchId) -> bool {
        self.consumed_batches.insert(batch)
    }

    /// Drops partial health after any accepted material-changing direct edit.
    pub(crate) fn forget_voxel(&mut self, position: TilePos, damaged: &mut DamagedVoxels) {
        self.remaining.remove(&position);
        damaged.remove(position);
    }

    /// Clears all per-world state while retaining the resource allocation.
    pub(crate) fn reset(&mut self, damaged: &mut DamagedVoxels) {
        self.remaining.clear();
        self.consumed_batches.clear();
        damaged.clear();
    }

    /// Applies one already-admitted impact in its exact announced order.
    pub(crate) fn apply(
        &mut self,
        impact: TerrainImpact,
        map: &mut VoxelMap,
        substances: &SubstanceTable,
        damage_table: &TerrainDamageTable,
        damaged: &mut DamagedVoxels,
        mut is_protected: impl FnMut(TilePos) -> bool,
    ) -> AppliedTerrainImpact {
        let mut destroyed = Vec::new();
        let voxels = impact
            .volume
            .iter()
            .copied()
            .map(|position| {
                let substance = map.get(position);
                if substance.is_air() {
                    self.forget_voxel(position, damaged);
                    return TerrainVoxelOutcome {
                        pos: position,
                        disposition: TerrainImpactDisposition::NoMaterial,
                        before: None,
                        after: None,
                        health_before: None,
                        health_after: None,
                    };
                }

                let maximum = substances.toughness(substance);
                let health_before = maximum.and_then(|maximum| {
                    let remaining = self.remaining.get(&position).copied().unwrap_or(maximum);
                    TerrainVoxelHealth::new(remaining.min(maximum), maximum)
                        .or_else(|| TerrainVoxelHealth::new(maximum, maximum))
                });
                let admitted = substances.is_diggable(substance)
                    && maximum.is_some()
                    && damage_table.damages(impact.element, substance)
                    && !is_protected(position);

                if !admitted {
                    return TerrainVoxelOutcome {
                        pos: position,
                        disposition: TerrainImpactDisposition::Resisted,
                        before: Some(substance),
                        after: Some(substance),
                        health_before,
                        health_after: health_before,
                    };
                }

                let Some(health_before) = health_before else {
                    return TerrainVoxelOutcome {
                        pos: position,
                        disposition: TerrainImpactDisposition::Resisted,
                        before: Some(substance),
                        after: Some(substance),
                        health_before: None,
                        health_after: None,
                    };
                };
                if impact.power >= health_before.remaining {
                    map.set(position, hex_core::SubstanceId::AIR);
                    self.forget_voxel(position, damaged);
                    destroyed.push(position);
                    TerrainVoxelOutcome {
                        pos: position,
                        disposition: TerrainImpactDisposition::Destroyed,
                        before: Some(substance),
                        after: None,
                        health_before: Some(health_before),
                        health_after: None,
                    }
                } else {
                    let health_after = TerrainVoxelHealth {
                        remaining: health_before.remaining - impact.power,
                        maximum: health_before.maximum,
                    };
                    self.remaining.insert(position, health_after.remaining);
                    damaged.publish(position, health_after);
                    TerrainVoxelOutcome {
                        pos: position,
                        disposition: TerrainImpactDisposition::Damaged,
                        before: Some(substance),
                        after: Some(substance),
                        health_before: Some(health_before),
                        health_after: Some(health_after),
                    }
                }
            })
            .collect();

        AppliedTerrainImpact {
            outcome: TerrainImpactOutcome {
                batch: impact.batch,
                result: TerrainImpactResult::Applied(voxels),
            },
            destroyed,
        }
    }
}

/// Applied outcome plus exact material changes that need ordinary map consequences.
#[derive(Debug)]
pub(crate) struct AppliedTerrainImpact {
    pub(crate) outcome: TerrainImpactOutcome,
    pub(crate) destroyed: Vec<TilePos>,
}
