//! Explicit test-support CPU attribution; never an input to simulation behavior.

use std::{cell::Cell, collections::BTreeMap, time::Instant};

use serde::Serialize;

use crate::ArenaSession;

/// Ordered CPU sections of an encounter tick, excluding rendering and GPU work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArenaCpuPhase {
    /// Adopting the current public terrain revision in the collision cache.
    CollisionRefresh,
    /// Effect expiry, encounter initialization, barriers and Worm preparation.
    EncounterSetup,
    /// Party sight, activation, memory and return-state updates.
    ObserveParties,
    /// Troll rally state and its bounded re-entry controller proofs.
    Rally,
    /// Creature intent generation, including steering and predictive probes.
    Brains,
    /// Live actor movement, casts, separation and physical confinement.
    MovementAndSeparation,
    /// Updating existing projectiles and resolving their impacts.
    Projectiles,
    /// Support, new releases, abilities, walls, progression and terminal handling.
    RemainingResolution,
}

/// Actual steering work during the measured brain loop; no per-step clocks.
/// Decision reasons overlap when both the deadline and terrain revision changed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SteeringCpuCounters {
    /// Fresh direction searches, including dormant Shadow travel.
    pub decisions: u32,
    /// Searches whose ordinary decision deadline expired.
    pub deadline_decisions: u32,
    /// Searches invalidated by a different collision-world revision.
    pub revision_decisions: u32,
    /// Shadow direction searches with a displaced body.
    pub displaced_decisions: u32,
    /// Blocked-body descent/jump/detour recovery passes.
    pub recovery_searches: u32,
    /// Sideways/backward detour searches after recovery did not find a landing.
    pub detour_searches: u32,
    /// Executed walking/flight look-ahead controller steps, including detours.
    pub walk_probe_steps: u32,
    /// Executed descending-landing controller steps.
    pub descent_probe_steps: u32,
    /// Executed jumping-landing controller steps.
    pub jump_probe_steps: u32,
}

/// One completed encounter tick from an explicitly enabled diagnostic session.
#[derive(Debug, Default, Clone, Serialize)]
pub struct ArenaCpuSnapshot {
    /// Actual simulation tick after the measured encounter finishes.
    pub tick: u64,
    /// Public terrain revision consumed by this tick.
    pub world_revision: u64,
    /// CPU wall milliseconds, including diagnostic boundary/counter overhead.
    pub phases_ms: BTreeMap<ArenaCpuPhase, f64>,
    /// Exact counted look-ahead work in this tick's creature brain loop.
    pub steering: SteeringCpuCounters,
    /// Exact small-movement candidate reuse within the same immutable brain loop.
    pub probe_cache: crate::ProbeCacheStats,
}

#[derive(Debug, Default)]
pub(crate) struct CpuDiagnostics {
    pub(crate) enabled: bool,
    boundary: Option<Instant>,
    snapshot: Option<ArenaCpuSnapshot>,
    completed: bool,
}

impl CpuDiagnostics {
    pub(crate) fn enabled(enabled: bool) -> Self {
        Self {
            enabled,
            ..Self::default()
        }
    }

    pub(crate) fn begin(&mut self, world_revision: u64) {
        self.completed = false;
        self.snapshot = self.enabled.then(|| ArenaCpuSnapshot {
            world_revision,
            ..ArenaCpuSnapshot::default()
        });
        self.boundary = self.enabled.then(Instant::now);
    }

    pub(crate) fn mark(&mut self, phase: ArenaCpuPhase) {
        if let (Some(previous), Some(snapshot)) = (self.boundary, self.snapshot.as_mut()) {
            let now = Instant::now();
            snapshot
                .phases_ms
                .insert(phase, now.duration_since(previous).as_secs_f64() * 1000.0);
            self.boundary = Some(now);
        }
    }

    pub(crate) fn begin_brains(&self) -> Option<SteeringScope> {
        self.enabled.then(SteeringScope::new)
    }

    pub(crate) fn finish_brains(&mut self, scope: Option<SteeringScope>) {
        if let Some(scope) = scope {
            let counters = scope.finish();
            if let Some(snapshot) = self.snapshot.as_mut() {
                snapshot.steering = counters;
            }
        }
        self.mark(ArenaCpuPhase::Brains);
    }

    pub(crate) fn record_probe_cache(&mut self, stats: crate::ProbeCacheStats) {
        if let Some(snapshot) = self.snapshot.as_mut() {
            snapshot.probe_cache = stats;
        }
    }

    pub(crate) fn finish(&mut self, tick: u64) {
        self.mark(ArenaCpuPhase::RemainingResolution);
        if let Some(snapshot) = self.snapshot.as_mut() {
            snapshot.tick = tick;
            self.completed = true;
        }
        self.boundary = None;
    }
}

impl ArenaSession {
    /// Enable test-support CPU diagnostics without changing simulation decisions.
    /// Disabled by default; reset retains this choice and clears the old sample.
    pub fn set_cpu_profiling(&mut self, enabled: bool) {
        self.cpu = CpuDiagnostics::enabled(enabled);
    }

    /// Borrow the last completed encounter tick; none when disabled or incomplete.
    /// These CPU timings are neither native frame timings nor GPU/FPS evidence.
    #[must_use]
    pub fn cpu_profile(&self) -> Option<&ArenaCpuSnapshot> {
        self.cpu.snapshot.as_ref().filter(|_| self.cpu.completed)
    }
}

thread_local! {
    // The scope exists only while one explicitly profiled session runs brains.
    // Thread-local storage keeps concurrent test worlds separate; Drop restores
    // any outer scope even if intent generation unwinds.
    static STEERING: Cell<Option<SteeringCpuCounters>> = const { Cell::new(None) };
}

pub(crate) struct SteeringScope {
    previous: Option<SteeringCpuCounters>,
    active: bool,
}

impl SteeringScope {
    fn new() -> Self {
        Self {
            previous: STEERING.replace(Some(SteeringCpuCounters::default())),
            active: true,
        }
    }

    fn finish(mut self) -> SteeringCpuCounters {
        self.active = false;
        STEERING.replace(self.previous).unwrap_or_default()
    }
}

impl Drop for SteeringScope {
    fn drop(&mut self) {
        if self.active {
            STEERING.set(self.previous);
        }
    }
}

fn count(update: impl FnOnce(&mut SteeringCpuCounters)) {
    if let Some(mut counters) = STEERING.get() {
        update(&mut counters);
        STEERING.set(Some(counters));
    }
}

pub(crate) fn decision(deadline: bool, revision: bool, displaced: bool) {
    count(|c| {
        c.decisions += 1;
        c.deadline_decisions += u32::from(deadline);
        c.revision_decisions += u32::from(revision);
        c.displaced_decisions += u32::from(displaced);
    });
}

pub(crate) fn recovery() {
    count(|c| c.recovery_searches += 1);
}

pub(crate) fn detour() {
    count(|c| c.detour_searches += 1);
}

pub(crate) enum ProbeKind {
    Walk,
    Descent,
    Jump,
}

/// Counts locally and publishes once per search, including early-return paths.
/// Inactive diagnostics perform no thread-local updates or clock reads per step.
pub(crate) struct ProbeCounter {
    kind: ProbeKind,
    enabled: bool,
    steps: u32,
}

impl ProbeCounter {
    pub(crate) fn new(kind: ProbeKind) -> Self {
        Self {
            kind,
            enabled: STEERING.get().is_some(),
            steps: 0,
        }
    }

    pub(crate) fn step(&mut self) {
        self.steps += u32::from(self.enabled);
    }
}

impl Drop for ProbeCounter {
    fn drop(&mut self) {
        if self.enabled {
            count(|c| match self.kind {
                ProbeKind::Walk => c.walk_probe_steps += self.steps,
                ProbeKind::Descent => c.descent_probe_steps += self.steps,
                ProbeKind::Jump => c.jump_probe_steps += self.steps,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_are_opt_in_and_reset_clears_the_sample() {
        let mut session = ArenaSession::default();
        session.cpu.begin(7);
        session.cpu.finish(1);
        assert!(session.cpu_profile().is_none());
        assert!(session.cpu.boundary.is_none());
        session.set_cpu_profiling(true);
        session.cpu.begin(7);
        session.cpu.mark(ArenaCpuPhase::CollisionRefresh);
        session.cpu.finish(2);
        assert_eq!(
            session.cpu_profile().map(|s| (s.tick, s.world_revision)),
            Some((2, 7))
        );
        session.reset(1, &Default::default(), Default::default());
        assert!(session.cpu.enabled);
        assert!(session.cpu_profile().is_none());
        session.set_cpu_profiling(false);
        assert!(!session.cpu.enabled);
    }

    #[test]
    fn steering_scope_counts_actual_steps_and_restores_nested_or_dropped_scopes() {
        assert!(STEERING.get().is_none());
        decision(true, true, false);
        assert!(STEERING.get().is_none());
        let outer = SteeringScope::new();
        decision(true, true, false);
        {
            let nested = SteeringScope::new();
            let mut probe = ProbeCounter::new(ProbeKind::Walk);
            probe.step();
            probe.step();
            drop(probe);
            assert_eq!(nested.finish().walk_probe_steps, 2);
        }
        {
            let _dropped = SteeringScope::new();
            recovery();
        }
        assert_eq!(
            outer.finish(),
            SteeringCpuCounters {
                decisions: 1,
                deadline_decisions: 1,
                revision_decisions: 1,
                ..SteeringCpuCounters::default()
            }
        );
        assert!(STEERING.get().is_none());
    }
}
