//! Bounded memoization of exact ordered candidates during an immutable probe scope.
//! No occupancy, hit, clearance, decision, or dynamic barrier result is cached.

use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use hex_core::HexCoord;

use super::Span;

const MAX_ENTRIES: usize = 128;
const MAX_SPANS: usize = 8192;
pub(super) const MAX_ENTRY_SPANS: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Key {
    pub start: HexCoord,
    pub end: HexCoord,
    pub ring: u32,
}

/// Candidate-cache work in one immutable brain loop, available only to tests.
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Default, Clone, Copy, serde::Serialize)]
pub struct ProbeCacheStats {
    /// Eligible small movement queries served by an existing exact entry.
    pub hits: u64,
    /// Eligible small movement queries which required ordinary enumeration.
    pub misses: u64,
    /// Oldest entries removed to keep the fixed entry/span limits.
    pub evictions: u64,
    /// Candidate lists too large to cache; the original iterator is continued.
    pub oversized: u64,
    /// Maximum cached entries during this scope (limit 128).
    pub peak_entries: usize,
    /// Maximum cached spans during this scope (limit 8192).
    pub peak_spans: usize,
}

#[derive(Debug, Default)]
struct State {
    depth: usize,
    entries: HashMap<Key, Arc<[Span]>>,
    oldest: VecDeque<Key>,
    spans: usize,
    #[cfg(any(test, feature = "test-support"))]
    stats: ProbeCacheStats,
}

#[derive(Debug, Default)]
pub(super) struct ProbeCache {
    active: AtomicBool,
    state: Mutex<State>,
}

// A cloned collision world must never share cached candidates or scope state.
impl Clone for ProbeCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

pub(super) enum Lookup {
    Inactive,
    Miss,
    Hit(Arc<[Span]>),
}

impl ProbeCache {
    pub(super) fn scope(&self) -> ProbeScope<'_> {
        let entered = if let Ok(mut state) = self.state.lock() {
            if state.depth == 0 {
                state.entries.clear();
                state.oldest.clear();
                state.spans = 0;
                #[cfg(any(test, feature = "test-support"))]
                {
                    state.stats = ProbeCacheStats::default();
                }
            }
            state.depth += 1;
            self.active.store(true, Ordering::Relaxed);
            true
        } else {
            // Poisoning can disable an optimization, never suppress a world query.
            false
        };
        ProbeScope {
            cache: self,
            entered,
        }
    }

    pub(super) fn lookup(&self, key: Key) -> Lookup {
        if !self.active.load(Ordering::Relaxed) {
            return Lookup::Inactive;
        }
        let Ok(mut state) = self.state.lock() else {
            return Lookup::Inactive;
        };
        if state.depth == 0 {
            return Lookup::Inactive;
        }
        if let Some(spans) = state.entries.get(&key).cloned() {
            #[cfg(any(test, feature = "test-support"))]
            {
                state.stats.hits += 1;
            }
            Lookup::Hit(spans)
        } else {
            #[cfg(any(test, feature = "test-support"))]
            {
                state.stats.misses += 1;
            }
            Lookup::Miss
        }
    }

    pub(super) fn insert(&self, key: Key, spans: Arc<[Span]>) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.depth == 0 || state.entries.contains_key(&key) {
            return;
        }
        while state.entries.len() >= MAX_ENTRIES || state.spans + spans.len() > MAX_SPANS {
            let Some(oldest) = state.oldest.pop_front() else {
                return;
            };
            if let Some(removed) = state.entries.remove(&oldest) {
                state.spans -= removed.len();
                #[cfg(any(test, feature = "test-support"))]
                {
                    state.stats.evictions += 1;
                }
            }
        }
        state.spans += spans.len();
        state.entries.insert(key, spans);
        state.oldest.push_back(key);
        #[cfg(any(test, feature = "test-support"))]
        {
            state.stats.peak_entries = state.stats.peak_entries.max(state.entries.len());
            state.stats.peak_spans = state.stats.peak_spans.max(state.spans);
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn oversized(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.stats.oversized += 1;
        }
    }
}

/// Holds the collision world's shared borrow until all same-snapshot queries end.
pub(crate) struct ProbeScope<'a> {
    cache: &'a ProbeCache,
    entered: bool,
}

impl ProbeScope<'_> {
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn stats(&self) -> ProbeCacheStats {
        self.cache
            .state
            .lock()
            .map_or_else(|_| ProbeCacheStats::default(), |state| state.stats)
    }
}

impl Drop for ProbeScope<'_> {
    fn drop(&mut self) {
        if !self.entered {
            return;
        }
        if let Ok(mut state) = self.cache.state.lock() {
            state.depth -= 1;
            if state.depth == 0 {
                self.cache.active.store(false, Ordering::Relaxed);
                state.entries.clear();
                state.oldest.clear();
                state.spans = 0;
            }
        }
    }
}

pub(super) enum Candidates<I> {
    Live(I),
    Cached {
        spans: Arc<[Span]>,
        next: usize,
    },
    Overflow {
        prefix: std::vec::IntoIter<Span>,
        rest: I,
    },
}

impl<I: Iterator<Item = Span>> Iterator for Candidates<I> {
    type Item = Span;

    fn next(&mut self) -> Option<Span> {
        match self {
            Self::Live(iter) => iter.next(),
            Self::Cached { spans, next } => {
                let span = spans.get(*next).copied();
                *next += usize::from(span.is_some());
                span
            }
            Self::Overflow { prefix, rest } => prefix.next().or_else(|| rest.next()),
        }
    }
}
