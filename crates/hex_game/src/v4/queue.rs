//! Bounded asynchronous presentation transactions. Neighbor faces and root
//! lifecycle publish together, so unavailable presentation never opens a hole.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{mpsc, Arc, Mutex};

use bevy::prelude::*;
use hex_map::v4::{PreparedChunk, RenderNeighbor, TerrainPresenter};
use hex_world_contracts::ChunkId;
use hex_world_runtime::{ChunkProduct, WorldRuntime};

use super::art::{ArtPlan, StockArt};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SourceStamp {
    revision: u64,
    package: u64,
    art: u64,
}

impl SourceStamp {
    fn new(product: &ChunkProduct, art: &StockArt) -> Self {
        Self {
            revision: product.revision,
            package: product.package.fingerprint,
            art: art.signature(product.coordinate),
        }
    }
}

struct Completion {
    epoch: u64,
    coordinate: ChunkId,
    source: Option<SourceStamp>,
    prepared: Result<Vec<PreparedChunk>, String>,
    art: Option<ArtPlan>,
}

/// Exactly one preparation transaction can be in flight. Its target and at most
/// six published neighbors bound both prepared geometry and atomic uploads to
/// seven chunks. An obsolete worker keeps its slot until it actually finishes.
#[derive(Resource)]
pub(super) struct MeshQueue {
    epoch: u64,
    active: bool,
    pending: usize,
    accepted: BTreeMap<ChunkId, SourceStamp>,
    sender: mpsc::Sender<Completion>,
    receiver: Mutex<mpsc::Receiver<Completion>>,
    pub art: Option<StockArt>,
    pub published: u64,
    pub discarded: u64,
    pub peak_pending: usize,
}

impl Default for MeshQueue {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            epoch: 0,
            active: false,
            pending: 0,
            accepted: BTreeMap::new(),
            sender,
            receiver: Mutex::new(receiver),
            art: None,
            published: 0,
            discarded: 0,
            peak_pending: 0,
        }
    }
}

impl MeshQueue {
    pub fn cancel(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
    }

    pub fn is_idle(&self) -> bool {
        !self.active && self.pending == 0
    }

    pub fn tick(
        &mut self,
        world: &mut World,
        runtime: &WorldRuntime,
        presenter: &mut TerrainPresenter,
        desired: &BTreeSet<ChunkId>,
        allow_admission: bool,
    ) -> Result<(), String> {
        let result = self
            .receiver
            .lock()
            .map_err(|error| error.to_string())?
            .try_recv()
            .ok();
        if let Some(result) = result {
            self.active = false;
            let current = runtime.resident_chunk(result.coordinate);
            let valid = result.epoch == self.epoch
                && match result.source {
                    Some(source) => {
                        allow_admission
                            && desired.contains(&result.coordinate)
                            && current.as_ref().zip(self.art.as_ref()).is_some_and(
                                |(current, art)| SourceStamp::new(current, art) == source,
                            )
                    }
                    None => !desired.contains(&result.coordinate),
                };
            if valid {
                let prepared = result.prepared?;
                // No roots or assets change until every neighbor and target has
                // passed admission. Only this serial queue mutates presentation.
                for chunk in &prepared {
                    presenter
                        .validate_publication(chunk)
                        .map_err(|error| error.to_string())?;
                }
                let art = self.art.as_mut().ok_or("stock art is not ready")?;
                if let Some(plan) = result.art {
                    art.publish(world, result.coordinate, plan)?;
                } else {
                    art.remove(world, result.coordinate)?;
                }
                for chunk in prepared {
                    presenter
                        .publish(world, chunk)
                        .map_err(|error| error.to_string())?;
                    self.published += 1;
                }
                if let Some(source) = result.source {
                    self.accepted.insert(result.coordinate, source);
                } else {
                    presenter.remove(world, result.coordinate);
                    self.accepted.remove(&result.coordinate);
                }
            } else {
                self.discarded += 1;
            }
        }
        let retired = presenter
            .receipts()
            .filter(|receipt| !desired.contains(&receipt.coordinate))
            .map(|receipt| receipt.coordinate)
            .collect::<Vec<_>>();
        let changed = runtime
            .resident_chunks()
            .filter(|product| desired.contains(&product.coordinate))
            .filter(|product| {
                self.art.as_ref().is_none_or(|art| {
                    self.accepted.get(&product.coordinate) != Some(&SourceStamp::new(product, art))
                })
            })
            .collect::<Vec<_>>();
        self.pending = retired.len() + changed.len();
        self.peak_pending = self.peak_pending.max(self.pending);
        if self.active || self.pending == 0 {
            return Ok(());
        }
        let Some(art) = &self.art else {
            return Ok(());
        };
        // Restore the surviving boundary before removing an old root. New roots
        // consume only bounded preparation capacity after retirement progresses.
        let (coordinate, source, art_plan, replacement) = if let Some(coordinate) = retired.first()
        {
            (*coordinate, None, None, None)
        } else if let Some(product) = changed.first().filter(|_| allow_admission) {
            let plan = art.prepare(product.coordinate, product.revision, presenter.origin())?;
            let replacement = RenderNeighbor {
                package: product.package.clone(),
                revision: product.revision,
                suppression: Arc::new(plan.suppression.clone()),
            };
            (
                product.coordinate,
                Some(SourceStamp::new(product, art)),
                Some(plan),
                Some(replacement),
            )
        } else {
            return Ok(());
        };
        let affected = affected_chunks(coordinate, replacement.is_some(), |chunk| {
            presenter.package(chunk).is_some()
        });
        // A target affects its immediate neighbors. Preparing those neighbors
        // needs their own one-hex halos, so snapshot at most two chunk rings.
        // Snapshots describe published geometry, never merely resident terrain.
        let mut snapshots = BTreeMap::new();
        for chunk in &affected {
            for candidate in std::iter::once(*chunk).chain(neighbors(*chunk)) {
                if let Some(snapshot) = presenter.render_neighbor(candidate) {
                    snapshots.entry(candidate).or_insert(snapshot);
                }
            }
        }
        snapshots.remove(&coordinate);
        if let Some(replacement) = replacement {
            snapshots.insert(coordinate, replacement);
        }
        let context = presenter.preparer();
        let sender = self.sender.clone();
        let epoch = self.epoch;
        std::thread::Builder::new()
            .name(format!("hex-mesh-{}-{}", coordinate.q, coordinate.r))
            .spawn(move || {
                let prepared = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    affected
                        .into_iter()
                        .map(|chunk| {
                            let own = snapshots
                                .get(&chunk)
                                .ok_or("presentation snapshot is absent")?;
                            let neighbors = neighbors(chunk)
                                .filter_map(|neighbor| snapshots.get(&neighbor).cloned())
                                .collect::<Vec<_>>();
                            let halo = context
                                .render_halo(chunk, &neighbors)
                                .map_err(|error| error.to_string())?;
                            context
                                .prepare_with_render_halo(
                                    &own.package,
                                    own.revision,
                                    &own.suppression,
                                    &halo,
                                )
                                .map_err(|error| error.to_string())
                        })
                        .collect::<Result<Vec<_>, String>>()
                }))
                .unwrap_or_else(|_| Err("mesh preparation worker panicked".to_owned()));
                let _sent = sender.send(Completion {
                    epoch,
                    coordinate,
                    source,
                    prepared,
                    art: art_plan,
                });
            })
            .map_err(|error| error.to_string())?;
        self.active = true;
        Ok(())
    }
}

fn neighbors(chunk: ChunkId) -> impl Iterator<Item = ChunkId> {
    [(1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1)]
        .into_iter()
        .filter_map(move |(q, r)| {
            Some(ChunkId {
                q: chunk.q.checked_add(q)?,
                r: chunk.r.checked_add(r)?,
            })
        })
}

fn affected_chunks(
    target: ChunkId,
    present_after: bool,
    presented: impl Fn(ChunkId) -> bool,
) -> BTreeSet<ChunkId> {
    neighbors(target)
        .filter(|chunk| presented(*chunk))
        .chain(present_after.then_some(target))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admission_and_retirement_repair_only_existing_neighbor_faces() {
        let target = ChunkId { q: -1, r: 0 };
        let west = ChunkId { q: -2, r: 0 };
        let east = ChunkId { q: 0, r: 0 };
        let resident_but_unpresented = ChunkId { q: -1, r: 1 };
        let presented = BTreeSet::from([west, east]);
        let admitted = affected_chunks(target, true, |chunk| presented.contains(&chunk));
        assert_eq!(admitted, BTreeSet::from([west, target, east]));
        assert!(!admitted.contains(&resident_but_unpresented));
        assert_eq!(
            affected_chunks(target, false, |chunk| presented.contains(&chunk)),
            presented
        );
    }

    #[test]
    fn fixed_transaction_bound_does_not_depend_on_catalogue_or_coordinate_size() {
        let target = ChunkId {
            q: 4_000_000_000,
            r: -4_000_000_000,
        };
        assert_eq!(affected_chunks(target, true, |_| true).len(), 7);
        assert_eq!(affected_chunks(target, false, |_| true).len(), 6);
        assert_eq!(
            neighbors(ChunkId {
                q: i64::MAX,
                r: i64::MIN
            })
            .count(),
            3
        );
    }
}

#[cfg(test)]
#[path = "queue_lifecycle_tests.rs"]
mod lifecycle_tests;
