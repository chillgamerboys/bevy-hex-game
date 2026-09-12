//! Opt-in native wall-clock frame intervals and scoped UI CPU measurements.
//!
//! This samples one fixed point in each Update. Intervals include intervening
//! app work and pacing/waits; they are not GPU timestamps or presented-frame FPS.
//! No resource or system is installed unless HEX_ARENA_UX_PERF is exactly `1`.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use hex_arena::ArenaSession;
use serde_json::{Value, json};

use super::{Page, UxState};
use crate::arena::{ArenaFrame, ViewState, recording::Recorder};

const CAPACITY: usize = 512;
const REPORT_INTERVAL: Duration = Duration::from_secs(5);

/// Install after UI setup. Disabled launches do no per-frame diagnostic work.
pub(super) fn install(app: &mut App) {
    if !opted_in(std::env::var("HEX_ARENA_UX_PERF").ok().as_deref()) {
        return;
    }
    app.init_resource::<Samples>()
        .add_systems(Update, observe.after(ArenaFrame::Present));
}

fn opted_in(value: Option<&str>) -> bool {
    value == Some("1")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Phase {
    recording: bool,
    map_visible: bool,
    paused: bool,
    started: bool,
    capture_mode: bool,
    page: Page,
    scale_bits: u32,
    actors: usize,
    focused: Option<bool>,
}

#[derive(Resource, Default)]
struct Samples {
    phase: Option<Phase>,
    previous: Option<Instant>,
    phase_started: Option<Instant>,
    bucket_started: Option<Instant>,
    observed_frames: u64,
    // Frame interval milliseconds and the same frame's measured UI microseconds.
    retained: VecDeque<(f64, f64)>,
}

fn observe(
    mut samples: ResMut<Samples>,
    session: Res<ArenaSession>,
    ux: Res<UxState>,
    view: Res<ViewState>,
    recorder: Option<Res<Recorder>>,
    scale: Res<UiScale>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    let now = Instant::now();
    let transitioning = recorder
        .as_ref()
        .is_some_and(|r| r.is_starting() || r.is_finalizing());
    let phase = (!transitioning).then_some(Phase {
        recording: recorder.as_ref().is_some_and(|r| r.is_recording()),
        map_visible: ux.map_visible,
        paused: view.paused,
        started: view.started,
        capture_mode: view.capture.is_some(),
        page: ux.page,
        scale_bits: scale.0.to_bits(),
        actors: session.actors.len(),
        focused: windows.single().ok().map(|window| window.focused),
    });
    if let Some(report) = samples.observe(now, phase, ux.present_micros) {
        info!("ARENA_UX_NATIVE_PERF {report}");
    }
}

impl Samples {
    fn clear_bucket(&mut self, now: Instant) {
        self.bucket_started = Some(now);
        self.observed_frames = 0;
        self.retained.clear();
    }

    fn observe(&mut self, now: Instant, phase: Option<Phase>, ui_micros: f64) -> Option<Value> {
        let Some(phase) = phase else {
            self.phase = None;
            self.previous = None;
            self.phase_started = None;
            self.clear_bucket(now);
            return None;
        };
        if self.phase != Some(phase) {
            self.phase = Some(phase);
            self.previous = Some(now);
            self.phase_started = Some(now);
            self.clear_bucket(now);
            // Never attribute a transition interval to either stable state.
            return None;
        }
        let previous = self.previous.replace(now)?;
        let elapsed = now.saturating_duration_since(previous);
        if !elapsed.is_zero() && ui_micros.is_finite() && ui_micros >= 0.0 {
            self.observed_frames = self.observed_frames.saturating_add(1);
            if self.retained.len() == CAPACITY {
                self.retained.pop_front();
            }
            self.retained
                .push_back((elapsed.as_secs_f64() * 1000.0, ui_micros));
        }
        let bucket_duration = now.saturating_duration_since(self.bucket_started?);
        let stable_duration = now.saturating_duration_since(self.phase_started?);
        if bucket_duration < REPORT_INTERVAL || stable_duration < REPORT_INTERVAL {
            return None;
        }
        let mut frame_ms: Vec<_> = self.retained.iter().map(|(frame, _)| *frame).collect();
        let mut ui_us: Vec<_> = self.retained.iter().map(|(_, ui)| *ui).collect();
        let retained_interval_seconds = frame_ms.iter().sum::<f64>() / 1000.0;
        frame_ms.sort_by(f64::total_cmp);
        ui_us.sort_by(f64::total_cmp);
        let retained_samples = u64::try_from(self.retained.len()).unwrap_or(u64::MAX);
        let report = json!({
            "schema_version": 1,
            "measurement": "wall_clock_between_post_presentation_Update_observations",
            "evidence_boundary": "app_update_intervals_include_intervening_work_and_waits_not_GPU_timestamps_or_presented_FPS",
            "stable_phase_seconds": stable_duration.as_secs_f64(),
            "bucket_seconds": bucket_duration.as_secs_f64(),
            "actor_count": phase.actors,
            "map_visible": phase.map_visible,
            "paused": phase.paused,
            "started": phase.started,
            "recording": phase.recording,
            "capture_mode": phase.capture_mode,
            "menu_page": phase.page.name(),
            "ui_scale": f32::from_bits(phase.scale_bits),
            "window_focused": phase.focused,
            "observed_frames": self.observed_frames,
            "retained_samples": retained_samples,
            "dropped_old_samples": self.observed_frames.saturating_sub(retained_samples),
            "sampling": "most_recent_512_valid_intervals_in_this_bucket",
            "retained_interval_seconds": retained_interval_seconds,
            "quantile": "nearest_rank",
            "frame_interval_ms": { "p50": percentile(&frame_ms, 50), "p95": percentile(&frame_ms, 95) },
            "ui_present_micros_p95": percentile(&ui_us, 95),
        });
        self.clear_bucket(now);
        Some(report)
    }
}

fn percentile(sorted: &[f64], percent: usize) -> Option<f64> {
    let index = (sorted.len() * percent).div_ceil(100).saturating_sub(1);
    sorted.get(index).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phase() -> Phase {
        Phase {
            recording: false,
            map_visible: false,
            paused: false,
            started: true,
            capture_mode: false,
            page: Page::Overview,
            scale_bits: 1.0_f32.to_bits(),
            actors: 115,
            focused: Some(true),
        }
    }

    #[test]
    fn default_is_off_and_only_explicit_one_enables_the_sampler() {
        for value in [None, Some("0"), Some("true"), Some(""), Some("01")] {
            assert!(!opted_in(value));
        }
        assert!(opted_in(Some("1")));
    }

    #[test]
    fn stable_phase_emits_after_five_seconds_with_real_interval_units() {
        let now = Instant::now();
        let mut samples = Samples::default();
        assert!(samples.observe(now, Some(phase()), 10.0).is_none());
        for frame in 1..50_u32 {
            assert!(
                samples
                    .observe(
                        now + Duration::from_millis(u64::from(frame) * 100),
                        Some(phase()),
                        100.0
                    )
                    .is_none()
            );
        }
        let report = samples
            .observe(now + Duration::from_secs(5), Some(phase()), 100.0)
            .expect("five-second report");
        assert_eq!(report.get("actor_count"), Some(&json!(115)));
        assert_eq!(report.get("retained_samples"), Some(&json!(50)));
        assert_eq!(
            report.pointer("/frame_interval_ms/p50"),
            Some(&json!(100.0))
        );
        assert_eq!(
            report.pointer("/frame_interval_ms/p95"),
            Some(&json!(100.0))
        );
        assert_eq!(report.get("ui_present_micros_p95"), Some(&json!(100.0)));
        assert!(samples.retained.is_empty());
        assert!(
            samples
                .observe(now + Duration::from_secs(6), Some(phase()), 100.0)
                .is_none()
        );
    }

    #[test]
    fn each_state_transition_discards_prior_intervals_and_waits_for_stability() {
        let start = Instant::now();
        for changed in [
            Phase {
                recording: true,
                ..phase()
            },
            Phase {
                map_visible: true,
                ..phase()
            },
            Phase {
                paused: true,
                ..phase()
            },
            Phase {
                scale_bits: 2.0_f32.to_bits(),
                ..phase()
            },
            Phase {
                page: Page::Map,
                ..phase()
            },
            Phase {
                focused: Some(false),
                ..phase()
            },
        ] {
            let mut samples = Samples::default();
            samples.observe(start, Some(phase()), 1.0);
            samples.observe(start + Duration::from_secs(4), Some(phase()), 1.0);
            assert!(
                samples
                    .observe(start + Duration::from_secs(5), Some(changed), 1.0)
                    .is_none()
            );
            assert!(samples.retained.is_empty());
            assert!(
                samples
                    .observe(start + Duration::from_secs(9), Some(changed), 2.0)
                    .is_none()
            );
            assert!(
                samples
                    .observe(start + Duration::from_secs(10), Some(changed), 2.0)
                    .is_some()
            );
            samples.observe(start + Duration::from_secs(11), None, 2.0);
            assert!(
                samples
                    .observe(start + Duration::from_secs(20), Some(changed), 2.0)
                    .is_none(),
                "recorder start/finalization is not a stable recording-off phase"
            );
        }
    }

    #[test]
    fn fast_frames_keep_only_512_samples_and_report_the_truncation() {
        let start = Instant::now();
        let mut samples = Samples::default();
        samples.observe(start, Some(phase()), 1.0);
        for frame in 1..5000_u64 {
            assert!(
                samples
                    .observe(start + Duration::from_millis(frame), Some(phase()), 250.0)
                    .is_none()
            );
            assert!(samples.retained.len() <= CAPACITY);
        }
        let report = samples
            .observe(start + Duration::from_secs(5), Some(phase()), 250.0)
            .expect("report");
        assert_eq!(report.get("observed_frames"), Some(&json!(5000)));
        assert_eq!(report.get("retained_samples"), Some(&json!(512)));
        assert_eq!(report.get("dropped_old_samples"), Some(&json!(4488)));
        assert_eq!(report.pointer("/frame_interval_ms/p95"), Some(&json!(1.0)));
        assert!(percentile(&[], 95).is_none());
        assert_eq!(
            percentile(&[1.0, 2.0, 3.0, 4.0], 50).map(f64::to_bits),
            Some(2.0_f64.to_bits())
        );
    }
}
