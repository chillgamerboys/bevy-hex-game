//! Bounded asynchronous mesh preparation. Only the main thread publishes assets.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{mpsc, Mutex};

use bevy::prelude::*;
use hex_map::v4::{PreparedChunk, TerrainPresenter};
use hex_world_contracts::ChunkId;
use hex_world_runtime::{ChunkProduct, WorldRuntime};

struct Completion {
    epoch: u64,
    product: ChunkProduct,
    prepared: Result<PreparedChunk, String>,
}

/// One view's queue; canceled work still occupies a real worker slot until it ends.
#[derive(Resource)]
pub(super) struct MeshQueue {
    epoch: u64,
    pending: BTreeMap<ChunkId, ChunkProduct>,
    jobs: BTreeSet<(u64, ChunkId)>,
    completed: VecDeque<Completion>,
    sender: mpsc::Sender<Completion>,
    receiver: Mutex<mpsc::Receiver<Completion>>,
    pub published: u64,
    pub discarded: u64,
    pub peak_pending: usize,
}

impl Default for MeshQueue {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            epoch: 0,
            pending: BTreeMap::new(),
            jobs: BTreeSet::new(),
            completed: VecDeque::new(),
            sender,
            receiver: Mutex::new(receiver),
            published: 0,
            discarded: 0,
            peak_pending: 0,
        }
    }
}

impl MeshQueue {
    pub fn enqueue(&mut self, product: ChunkProduct) {
        self.pending.insert(product.coordinate, product);
        self.peak_pending = self.peak_pending.max(self.pending.len());
    }

    pub fn cancel(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.pending.clear();
        self.completed.clear();
    }

    pub fn forget(&mut self, coordinate: ChunkId) {
        self.pending.remove(&coordinate);
    }

    pub fn is_idle(&self) -> bool {
        self.pending.is_empty() && self.jobs.is_empty() && self.completed.is_empty()
    }

    pub fn tick(
        &mut self,
        world: &mut World,
        runtime: &WorldRuntime,
        presenter: &mut TerrainPresenter,
        desired: &BTreeSet<ChunkId>,
    ) -> Result<(), String> {
        let receiver = self.receiver.lock().map_err(|error| error.to_string())?;
        while let Ok(result) = receiver.try_recv() {
            self.jobs.remove(&(result.epoch, result.product.coordinate));
            self.completed.push_back(result);
        }
        drop(receiver);
        let mut published = 0;
        while published < 2 {
            let Some(result) = self.completed.pop_front() else {
                break;
            };
            let coordinate = result.product.coordinate;
            let current = runtime.resident_chunk(coordinate);
            if result.epoch != self.epoch
                || !desired.contains(&coordinate)
                || current.is_none_or(|current| {
                    current.revision != result.product.revision
                        || current.package.fingerprint != result.product.package.fingerprint
                })
            {
                self.discarded += 1;
                continue;
            }
            presenter
                .publish(world, result.prepared?)
                .map_err(|error| error.to_string())?;
            self.published += 1;
            published += 1;
        }
        // Completed geometry counts against the queue budget too. A slow upload
        // stage cannot accumulate an unbounded mesh backlog behind two live workers.
        while self.jobs.len() + self.completed.len() < 2 {
            let candidate = self
                .pending
                .keys()
                .find(|coordinate| !self.jobs.contains(&(self.epoch, **coordinate)))
                .copied();
            let Some(coordinate) = candidate else {
                break;
            };
            let Some(product) = self.pending.remove(&coordinate) else {
                break;
            };
            if !desired.contains(&coordinate) {
                continue;
            }
            let context = presenter.preparer();
            let sender = self.sender.clone();
            let epoch = self.epoch;
            std::thread::Builder::new()
                .name(format!("hex-mesh-{}-{}", coordinate.q, coordinate.r))
                .spawn(move || {
                    let prepared = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        context
                            .prepare(&product.package, product.revision)
                            .map_err(|error| error.to_string())
                    }))
                    .unwrap_or_else(|_| Err("mesh preparation worker panicked".to_owned()));
                    let _sent = sender.send(Completion {
                        epoch,
                        product,
                        prepared,
                    });
                })
                .map_err(|error| error.to_string())?;
            self.jobs.insert((epoch, coordinate));
        }
        Ok(())
    }
}
