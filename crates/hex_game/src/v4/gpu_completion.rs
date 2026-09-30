//! Finite GPU-completion pacing for the windowless review renderer.
//!
//! The render app completes its submitted batch before starting another frame.
//! This bounds GPU backlog independently of the main/render-app channel capacity.
//! Native window presentation retains its existing runner and pacing.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bevy::prelude::*;
use bevy::render::render_resource::PollType;
use bevy::render::renderer::RenderDevice;
use bevy::render::{Render, RenderApp, RenderSystems};
use serde::Serialize;

const WAIT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_SAMPLES: usize = 10_000;

/// Shared progress uses short locks; GPU waiting never holds the mutex.
#[derive(Clone, Resource, Default)]
pub(super) struct GpuCompletion(Arc<Mutex<CompletionState>>);

#[derive(Default)]
struct CompletionState {
    attempted_batches: u64,
    completed_batches: u64,
    last_completion: Option<Instant>,
    wait_samples_ms: VecDeque<f64>,
    completion_intervals_ms: VecDeque<f64>,
    error: Option<String>,
}

/// Actual successful GPU wait evidence, sampled after screenshot readback.
#[derive(Serialize)]
pub(super) struct CompletionReceipt {
    mechanism: &'static str,
    wait_timeout_seconds: u64,
    attempted_batches: u64,
    completed_batches: u64,
    sample_capacity: usize,
    wait_samples_ms: Vec<f64>,
    completion_intervals_ms: Vec<f64>,
}

impl CompletionState {
    fn completed(&self) -> Result<u64, String> {
        match &self.error {
            Some(error) => Err(error.clone()),
            None => Ok(self.completed_batches),
        }
    }

    fn record_completion(&mut self, wait: Duration, finished: Instant) {
        if self.error.is_some() {
            return;
        }
        let Some(completed) = self.completed_batches.checked_add(1) else {
            self.error = Some("windowless GPU completion counter overflow".into());
            return;
        };
        self.completed_batches = completed;
        push_sample(&mut self.wait_samples_ms, wait.as_secs_f64() * 1000.0);
        if let Some(previous) = self.last_completion {
            push_sample(
                &mut self.completion_intervals_ms,
                finished.duration_since(previous).as_secs_f64() * 1000.0,
            );
        }
        self.last_completion = Some(finished);
    }
}

fn push_sample(samples: &mut VecDeque<f64>, value: f64) {
    if samples.len() == MAX_SAMPLES {
        samples.pop_front();
    }
    samples.push_back(value);
}

impl GpuCompletion {
    /// No timing-vector copy is needed for the per-update capture gate.
    pub fn completed_batches(&self) -> Result<u64, String> {
        self.0
            .lock()
            .map_err(|error| format!("windowless GPU completion state is poisoned: {error}"))?
            .completed()
    }

    /// A failed or absent wait can never create successful completion evidence.
    pub fn receipt(&self) -> Result<CompletionReceipt, String> {
        let state = self
            .0
            .lock()
            .map_err(|error| format!("windowless GPU completion state is poisoned: {error}"))?;
        if state.completed()? == 0 {
            return Err("windowless capture has no completed GPU batch".into());
        }
        Ok(CompletionReceipt {
            mechanism: "finite GPU wait after RenderSystems::Render; latest completed batch before next render frame",
            wait_timeout_seconds: WAIT_TIMEOUT.as_secs(),
            attempted_batches: state.attempted_batches,
            completed_batches: state.completed_batches,
            sample_capacity: MAX_SAMPLES,
            wait_samples_ms: state.wait_samples_ms.iter().copied().collect(),
            completion_intervals_ms: state.completion_intervals_ms.iter().copied().collect(),
        })
    }
}

/// Install only when the application uses its image-only capture path.
pub(super) fn install(app: &mut App) -> Result<(), String> {
    let completion = GpuCompletion::default();
    let render = app
        .get_sub_app_mut(RenderApp)
        .ok_or("windowless GPU pacing requires the render app")?;
    render.insert_resource(completion.clone()).add_systems(
        Render,
        wait_for_gpu
            .after(RenderSystems::Render)
            .before(RenderSystems::Cleanup),
    );
    app.insert_resource(completion);
    Ok(())
}

fn wait_for_gpu(device: Res<RenderDevice>, completion: Res<GpuCompletion>) {
    {
        let Ok(mut state) = completion.0.lock() else {
            return; // The main capture gate reports the poisoned state as failure.
        };
        if state.error.is_some() {
            return;
        }
        let Some(attempted) = state.attempted_batches.checked_add(1) else {
            state.error = Some("windowless GPU wait counter overflow".into());
            return;
        };
        state.attempted_batches = attempted;
    }
    let started = Instant::now();
    let result = device.poll(PollType::Wait {
        submission_index: None,
        timeout: Some(WAIT_TIMEOUT),
    });
    let finished = Instant::now();
    let Ok(mut state) = completion.0.lock() else {
        return;
    };
    match result {
        Ok(status) if status.wait_finished() => {
            state.record_completion(finished.duration_since(started), finished);
        }
        Ok(_) => {
            state.error = Some("windowless GPU poll did not confirm completion".into());
        }
        Err(error) => {
            state.error = Some(format!("windowless GPU completion wait failed: {error}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_completion_stays_failed_even_if_a_later_callback_succeeds() {
        let completion = GpuCompletion::default();
        assert!(completion.receipt().is_err());
        let now = Instant::now();
        {
            let mut state = completion.0.lock().expect("test state");
            state.record_completion(Duration::from_millis(1), now);
            state.error = Some("GPU wait timed out".into());
            state.record_completion(Duration::from_millis(2), now);
            assert_eq!(state.completed_batches, 1);
        }
        assert_eq!(
            completion.completed_batches(),
            Err("GPU wait timed out".into())
        );
        assert!(completion.receipt().is_err());
    }

    #[test]
    fn timing_history_retains_only_the_bounded_latest_completed_window() {
        let mut state = CompletionState::default();
        let start = Instant::now();
        for milliseconds in 0_u64..10_005 {
            let duration = Duration::from_millis(milliseconds);
            state.record_completion(duration, start + duration);
        }
        assert_eq!(state.completed_batches, 10_005);
        assert_eq!(state.wait_samples_ms.len(), MAX_SAMPLES);
        assert_eq!(state.completion_intervals_ms.len(), MAX_SAMPLES);
        assert!(state
            .wait_samples_ms
            .front()
            .is_some_and(|value| (*value - 5.0).abs() < f64::EPSILON));
        assert!(state
            .completion_intervals_ms
            .iter()
            .all(|value| (*value - 1.0).abs() < f64::EPSILON));
    }
}
