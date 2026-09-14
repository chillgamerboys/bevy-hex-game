//! Live broadphase for the existing, slice-ordered actor separation solver.

use super::{body_overlap, shapes, Actor, CollisionWorld, Species, SKIN};
use bevy_math::Vec3;
use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
mod tests;

/// Work performed by ordinary actor separation on the latest simulated tick.
/// Worms retain their separate movement/separation solver. Counts are deterministic;
/// CPU timing belongs to an external profiler, not simulation state.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ActorSeparationStats {
    /// Complete ordered passes, including the final unchanged pass.
    pub passes: usize,
    /// Eligible all-pairs comparisons for the same number of passes.
    pub possible_pairs: usize,
    /// Broadphase candidates sent to the unchanged body-overlap predicate.
    pub candidates: usize,
    /// Actual overlaps sent to the terrain-aware slide solver.
    pub overlaps: usize,
    /// Bucket queries, including re-queries when a push crosses cell boundaries.
    pub queries: usize,
}

const CELL_WIDTH: f32 = 2.0;

// Actor indices are slice order, so bits provide canonical deduplication and
// iteration without allocating a tree node for every candidate in a crowded camp.
struct Candidates(Vec<u64>);

impl Candidates {
    fn new(actor_count: usize) -> Self {
        Self(vec![0; actor_count.div_ceil(64)])
    }

    fn clear(&mut self) {
        self.0.fill(0);
    }

    fn insert(&mut self, index: usize) {
        if let Some(word) = self.0.get_mut(index / 64) {
            *word |= 1 << (index % 64);
        }
    }

    fn pop_first(&mut self) -> Option<usize> {
        for (index, word) in self.0.iter_mut().enumerate() {
            if *word != 0 {
                let bit = usize::try_from(word.trailing_zeros()).ok()?;
                *word &= *word - 1;
                return Some(index * 64 + bit);
            }
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cells {
    low: (i32, i32),
    high: (i32, i32),
}

impl Cells {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "floored spatial buckets; casts saturate"
    )]
    fn for_actor(actor: &Actor) -> Option<Self> {
        let (low, high) = match actor.species {
            Species::Golem | Species::Wisp | Species::Worm => {
                // Compound components have native radius one and world-axis offsets.
                // Include actual components rather than assuming the anchor is centered.
                actor.body_hex_prisms().fold(
                    (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
                    |(low, high), part| {
                        let center = actor.feet + part.offset;
                        (low.min(center - Vec3::ONE), high.max(center + Vec3::ONE))
                    },
                )
            }
            Species::Dragon => {
                let rotation = actor.body_rotation();
                let half = (rotation * Vec3::X).abs() * (actor.dimensions.x * 0.5)
                    + (rotation * Vec3::Z).abs() * (actor.dimensions.z * 0.5);
                (actor.feet - half, actor.feet + half)
            }
            Species::Human | Species::Shadow | Species::Goblin | Species::Shaman => {
                let half = Vec3::splat(actor.dimensions.x * 0.5);
                (actor.feet - half, actor.feet + half)
            }
        };
        if !low.is_finite() || !high.is_finite() {
            return None;
        }
        // Skin also covers the compound predicate's smaller face tolerance.
        let low = (low - Vec3::splat(SKIN)) / CELL_WIDTH;
        let high = (high + Vec3::splat(SKIN)) / CELL_WIDTH;
        let cells = Self {
            low: (low.x.floor() as i32, low.z.floor() as i32),
            high: (high.x.floor() as i32, high.z.floor() as i32),
        };
        let width = i64::from(cells.high.0) - i64::from(cells.low.0) + 1;
        let depth = i64::from(cells.high.1) - i64::from(cells.low.1) + 1;
        // Invalid or unusually huge bodies use the conservative global fallback.
        (width > 0 && depth > 0 && width <= 16 && depth <= 16).then_some(cells)
    }

    fn iter(self) -> impl Iterator<Item = (i32, i32)> {
        (self.low.0..=self.high.0)
            .flat_map(move |x| (self.low.1..=self.high.1).map(move |z| (x, z)))
    }
}

#[derive(Default)]
struct Index {
    buckets: BTreeMap<(i32, i32), Vec<usize>>,
    global: BTreeSet<usize>,
    active: BTreeMap<usize, Option<Cells>>,
}

impl Index {
    fn insert(&mut self, index: usize, cells: Option<Cells>) {
        self.active.insert(index, cells);
        if let Some(cells) = cells {
            for cell in cells.iter() {
                self.buckets.entry(cell).or_default().push(index);
            }
        } else {
            self.global.insert(index);
        }
    }

    /// Returns true only when the covered cells changed. The candidate set already
    /// contains every later actor in unchanged cells, even if the body moves inside.
    fn update(&mut self, index: usize, actor: &Actor) -> bool {
        let cells = Cells::for_actor(actor);
        let previous = self.active.get(&index).copied().flatten();
        if previous == cells {
            return false;
        }
        if let Some(previous) = previous {
            for cell in previous.iter() {
                if let Some(bucket) = self.buckets.get_mut(&cell) {
                    bucket.retain(|entry| *entry != index);
                    if bucket.is_empty() {
                        self.buckets.remove(&cell);
                    }
                }
            }
        } else {
            self.global.remove(&index);
        }
        self.insert(index, cells);
        true
    }

    fn query(&self, index: usize, after: usize, candidates: &mut Candidates) {
        if let Some(cells) = self.active.get(&index).copied().flatten() {
            for cell in cells.iter() {
                if let Some(bucket) = self.buckets.get(&cell) {
                    for other in bucket.iter().copied().filter(|other| *other > after) {
                        candidates.insert(other);
                    }
                }
            }
            for other in self.global.range((after + 1)..) {
                candidates.insert(*other);
            }
        } else {
            for (other, _) in self.active.range((after + 1)..) {
                candidates.insert(*other);
            }
        }
    }
}

pub(super) fn separate_many(actors: &mut [Actor], world: &CollisionWorld) -> ActorSeparationStats {
    let mut index = Index::default();
    for (i, actor) in actors.iter().enumerate() {
        if actor.hp > 0.0 && actor.species != Species::Worm {
            index.insert(i, Cells::for_actor(actor));
        }
    }
    let count = index.active.len();
    let pairs = count * count.saturating_sub(1) / 2;
    let mut stats = ActorSeparationStats::default();
    let mut candidates = Candidates::new(actors.len());
    for _ in 0..4 {
        stats.passes += 1;
        stats.possible_pairs += pairs;
        let mut changed = false;
        for i in 0..actors.len() {
            if !index.active.contains_key(&i) {
                continue;
            }
            candidates.clear();
            index.query(i, i, &mut candidates);
            stats.queries += 1;
            while let Some(j) = candidates.pop_first() {
                let (left, right) = actors.split_at_mut(j);
                let (Some(a), Some(b)) = (left.get_mut(i), right.first_mut()) else {
                    continue;
                };
                stats.candidates += 1;
                if let Some(push) = body_overlap(a, b) {
                    stats.overlaps += 1;
                    let fa = shapes::slide(world, a, a.feet, push).0;
                    let fb = shapes::slide(world, b, b.feet, -push).0;
                    changed |= fa.distance_squared(a.feet) > SKIN * SKIN
                        || fb.distance_squared(b.feet) > SKIN * SKIN;
                    a.feet = fa;
                    b.feet = fb;
                    let moved_cells = index.update(i, a);
                    index.update(j, b);
                    if moved_cells {
                        // Earlier pairs are deliberately not revisited. Later pairs
                        // newly reached by this push must run in original slice order.
                        index.query(i, j, &mut candidates);
                        stats.queries += 1;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    stats
}
