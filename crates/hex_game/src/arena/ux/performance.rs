//! Opt-in native frame intervals and separately scoped UI wall-time measurements.
//!
//! This samples one fixed point after UI in each PostUpdate. Intervals include intervening
//! app work and pacing/waits; they are not GPU timestamps or presented-frame FPS.
//! Adapter timing excludes Bevy text/layout. The separately bracketed UI pipeline
//! is an elapsed wall-span upper bound, not exclusive CPU cost: unrelated parallel
//! systems, scheduling and deferred work can extend the measured span.
//! No resource or system is installed unless HEX_ARENA_UX_PERF is exactly `1`.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use bevy::prelude::*;
use bevy::ui::UiSystems;
use bevy::window::PrimaryWindow;
use hex_arena::ArenaSession;
use serde_json::{json, Value};

use super::{Page, UxState};
use crate::arena::{recording::Recorder, ViewState};

const CAPACITY: usize = 512;
const REPORT_INTERVAL: Duration = Duration::from_secs(5);

/// Install after UI setup. Disabled launches do no per-frame diagnostic work.
pub(super) fn install(app: &mut App) {
    if !opted_in(std::env::var("HEX_ARENA_UX_PERF").ok().as_deref()) {
        return;
    }
    app.init_resource::<Samples>();
    install_ui_span(app);
    app.add_systems(PostUpdate, observe.after(end_ui_span));
}

fn install_ui_span(app: &mut App) {
    // Bevy 0.19: Prepare -> Propagate -> Content (text measurement) -> Layout
    // -> PostLayout (text rebuilding/clipping). Stack is a separate public set;
    // font ingestion and rerender detection precede Content outside that chain.
    app.add_systems(
        PostUpdate,
        begin_ui_span
            .before(UiSystems::Prepare)
            .before(UiSystems::Stack)
            .before(bevy::text::load_font_assets_into_font_collection)
            .before(bevy::text::detect_text_needs_rerender),
    )
    .add_systems(
        PostUpdate,
        end_ui_span
            .after(begin_ui_span)
            .after(bevy::text::load_font_assets_into_font_collection)
            .after(bevy::text::detect_text_needs_rerender)
            .after(UiSystems::PostLayout)
            .after(UiSystems::Stack),
    );
}

fn begin_ui_span(mut samples: ResMut<Samples>) {
    samples.ui_pipeline_started = Some(Instant::now());
    samples.ui_pipeline_wall_micros = None;
}

fn end_ui_span(mut samples: ResMut<Samples>) {
    samples.ui_pipeline_wall_micros = samples
        .ui_pipeline_started
        .take()
        .map(|start| start.elapsed().as_secs_f64() * 1e6);
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
    physical_viewport: Option<UVec2>,
}

struct FrameSample {
    frame_ms: f64,
    combined_ui_micros: f64,
    hud_only_micros: f64,
    observation_micros: f64,
    pipeline_micros: Option<f64>,
}

#[derive(Resource, Default)]
struct Samples {
    phase: Option<Phase>,
    previous: Option<Instant>,
    phase_started: Option<Instant>,
    bucket_started: Option<Instant>,
    observed_frames: u64,
    ui_pipeline_started: Option<Instant>,
    ui_pipeline_wall_micros: Option<f64>,
    ui_observation_micros: f64,
    retained: VecDeque<FrameSample>,
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
    let window = windows.single().ok();
    let phase = (!transitioning).then_some(Phase {
        recording: recorder.as_ref().is_some_and(|r| r.is_recording()),
        map_visible: ux.map_visible,
        paused: view.paused,
        started: view.started,
        capture_mode: view.capture.is_some(),
        page: ux.page,
        scale_bits: scale.0.to_bits(),
        actors: session.actors.len(),
        focused: window.map(|window| window.focused),
        physical_viewport: window.map(|window| {
            UVec2::new(
                window.resolution.physical_width(),
                window.resolution.physical_height(),
            )
        }),
    });
    samples.ui_observation_micros = ux.observation_micros;
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
        let observation_micros = self.ui_observation_micros;
        if !elapsed.is_zero()
            && ui_micros.is_finite()
            && observation_micros.is_finite()
            && observation_micros >= 0.0
            && ui_micros >= observation_micros
        {
            self.observed_frames = self.observed_frames.saturating_add(1);
            if self.retained.len() == CAPACITY {
                self.retained.pop_front();
            }
            self.retained.push_back(FrameSample {
                frame_ms: elapsed.as_secs_f64() * 1000.0,
                combined_ui_micros: ui_micros,
                // Derive before computing quantiles: subtracting the two p95s
                // would mix costs from different frames.
                hud_only_micros: ui_micros - observation_micros,
                observation_micros,
                pipeline_micros: self
                    .ui_pipeline_wall_micros
                    .filter(|value| value.is_finite() && *value >= 0.0),
            });
        }
        let bucket_duration = now.saturating_duration_since(self.bucket_started?);
        let stable_duration = now.saturating_duration_since(self.phase_started?);
        if bucket_duration < REPORT_INTERVAL || stable_duration < REPORT_INTERVAL {
            return None;
        }
        let mut frame_ms: Vec<_> = self.retained.iter().map(|sample| sample.frame_ms).collect();
        let mut ui_us: Vec<_> = self
            .retained
            .iter()
            .map(|sample| sample.combined_ui_micros)
            .collect();
        let mut hud_us: Vec<_> = self
            .retained
            .iter()
            .map(|sample| sample.hud_only_micros)
            .collect();
        let mut observation_us: Vec<_> = self
            .retained
            .iter()
            .map(|sample| sample.observation_micros)
            .collect();
        let mut pipeline_us: Vec<_> = self
            .retained
            .iter()
            .filter_map(|sample| sample.pipeline_micros)
            .collect();
        let retained_interval_seconds = frame_ms.iter().sum::<f64>() / 1000.0;
        frame_ms.sort_by(f64::total_cmp);
        ui_us.sort_by(f64::total_cmp);
        hud_us.sort_by(f64::total_cmp);
        observation_us.sort_by(f64::total_cmp);
        pipeline_us.sort_by(f64::total_cmp);
        let retained_samples = u64::try_from(self.retained.len()).unwrap_or(u64::MAX);
        let report = json!({
            "schema_version": 3,
            "measurement": "wall_clock_between_post_UI_PostUpdate_observations",
            "evidence_boundary": "app_frame_intervals_include_intervening_work_and_waits_not_GPU_timestamps_or_presented_FPS",
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
            "physical_viewport_width": phase.physical_viewport.map(|size| size.x),
            "physical_viewport_height": phase.physical_viewport.map(|size| size.y),
            "physical_viewport_scope": "primary_window_render_extent_full_window_arena_camera",
            "observed_frames": self.observed_frames,
            "retained_samples": retained_samples,
            "dropped_old_samples": self.observed_frames.saturating_sub(retained_samples),
            "sampling": "most_recent_512_valid_intervals_in_this_bucket",
            "retained_interval_seconds": retained_interval_seconds,
            "quantile": "nearest_rank",
            "frame_interval_ms": { "p50": percentile(&frame_ms, 50), "p95": percentile(&frame_ms, 95) },
            "ui_adapter_wall_micros_p95": percentile(&ui_us, 95),
            "ui_adapter_scope": "combined_corrected_Update_HUD_UX_wall_span_plus_player_observation_excludes_preceding_3D_presentation_and_Bevy_PostUpdate_text_layout",
            "ui_hud_only_wall_micros_p95": percentile(&hud_us, 95),
            "ui_hud_only_scope": "per_frame_combined_minus_observation_before_quantiles_begin_after_expedition_present_before_hud_update_end_after_UX_scroll_hints_may_include_scheduler_wait",
            "ui_observation_wall_micros_p95": percentile(&observation_us, 95),
            "ui_observation_scope": "player_camera_observation_adapter_including_gameplay_10Hz_visibility_queries_and_intervening_frames_early_returns",
            "bevy_ui_postupdate_wall_micros_p95": percentile(&pipeline_us, 95),
            "bevy_ui_postupdate_samples": pipeline_us.len(),
            "bevy_ui_postupdate_scope": "before_Prepare_font_ingestion_rerender_detection_and_Stack_through_after_PostLayout_and_Stack_includes_Content_text_measurement_Layout_text_rebuild_and_clipping",
            "ui_timing_boundary": "elapsed_wall_span_upper_bounds_may_include_unrelated_parallel_work_and_scheduling_not_exclusive_CPU_excludes_render_extraction_and_GPU_do_not_sum_separate_p95_values",
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
            physical_viewport: Some(UVec2::new(1600, 900)),
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
        let mut samples = Samples {
            ui_pipeline_wall_micros: Some(450.0),
            ..Default::default()
        };
        assert!(samples.observe(now, Some(phase()), 10.0).is_none());
        for frame in 1..50_u32 {
            assert!(samples
                .observe(
                    now + Duration::from_millis(u64::from(frame) * 100),
                    Some(phase()),
                    100.0
                )
                .is_none());
        }
        let report = samples
            .observe(now + Duration::from_secs(5), Some(phase()), 100.0)
            .expect("five-second report");
        assert_eq!(report.get("actor_count"), Some(&json!(115)));
        assert_eq!(report.get("schema_version"), Some(&json!(3)));
        assert_eq!(report.get("physical_viewport_width"), Some(&json!(1600)));
        assert_eq!(report.get("physical_viewport_height"), Some(&json!(900)));
        assert_eq!(report.get("retained_samples"), Some(&json!(50)));
        assert_eq!(
            report.pointer("/frame_interval_ms/p50"),
            Some(&json!(100.0))
        );
        assert_eq!(
            report.pointer("/frame_interval_ms/p95"),
            Some(&json!(100.0))
        );
        assert_eq!(
            report.get("ui_adapter_wall_micros_p95"),
            Some(&json!(100.0))
        );
        assert_eq!(
            report.get("bevy_ui_postupdate_wall_micros_p95"),
            Some(&json!(450.0))
        );
        assert_eq!(report.get("bevy_ui_postupdate_samples"), Some(&json!(50)));
        assert!(samples.retained.is_empty());
        assert!(samples
            .observe(now + Duration::from_secs(6), Some(phase()), 100.0)
            .is_none());
    }

    #[test]
    fn postupdate_markers_wrap_every_public_ui_stage_and_clear_stale_measurements() {
        #[derive(Resource, Default)]
        struct Visited(usize);
        fn within_span(samples: Res<Samples>, mut visited: ResMut<Visited>) {
            assert!(samples.ui_pipeline_started.is_some());
            assert!(samples.ui_pipeline_wall_micros.is_none());
            visited.0 += 1;
        }
        let mut app = App::new();
        app.init_resource::<Samples>().init_resource::<Visited>();
        app.configure_sets(
            PostUpdate,
            (
                UiSystems::Prepare,
                UiSystems::Propagate,
                UiSystems::Content,
                UiSystems::Layout,
                UiSystems::PostLayout,
            )
                .chain(),
        );
        install_ui_span(&mut app);
        for set in [
            UiSystems::Prepare,
            UiSystems::Propagate,
            UiSystems::Content,
            UiSystems::Layout,
            UiSystems::PostLayout,
            UiSystems::Stack,
        ] {
            app.add_systems(PostUpdate, within_span.in_set(set));
        }
        for expected in [6, 12] {
            app.world_mut().run_schedule(PostUpdate);
            let samples = app.world().resource::<Samples>();
            assert!(samples.ui_pipeline_started.is_none());
            assert!(samples.ui_pipeline_wall_micros.is_some());
            assert_eq!(app.world().resource::<Visited>().0, expected);
        }
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
            Phase {
                physical_viewport: Some(UVec2::new(1600, 950)),
                ..phase()
            },
            Phase {
                physical_viewport: Some(UVec2::new(1700, 900)),
                ..phase()
            },
        ] {
            let mut samples = Samples::default();
            samples.observe(start, Some(phase()), 1.0);
            samples.observe(start + Duration::from_secs(4), Some(phase()), 1.0);
            assert!(samples
                .observe(start + Duration::from_secs(5), Some(changed), 1.0)
                .is_none());
            assert!(samples.retained.is_empty());
            assert!(samples
                .observe(start + Duration::from_secs(9), Some(changed), 2.0)
                .is_none());
            assert!(samples
                .observe(start + Duration::from_secs(10), Some(changed), 2.0)
                .is_some());
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
    fn hud_only_cost_is_derived_per_frame_before_separate_quantiles() {
        let now = Instant::now();
        let mut samples = Samples::default();
        samples.observe(now, Some(phase()), 0.0);
        let mut report = None;
        for frame in 1..=50_u64 {
            let (hud_micros, observation_micros) = match frame {
                1..=3 => (90.0, 0.0),
                4..=6 => (0.0, 90.0),
                _ => (10.0, 10.0),
            };
            samples.ui_observation_micros = observation_micros;
            report = samples.observe(
                now + Duration::from_millis(frame * 100),
                Some(phase()),
                hud_micros + observation_micros,
            );
        }
        let report = report.expect("five-second report");
        // HUD and observation peaks occur on different frames. Subtracting
        // combined-p95 minus observation-p95 would incorrectly report zero HUD.
        for metric in [
            "ui_adapter_wall_micros_p95",
            "ui_hud_only_wall_micros_p95",
            "ui_observation_wall_micros_p95",
        ] {
            assert_eq!(report.get(metric), Some(&json!(90.0)), "{metric}");
        }
    }

    #[test]
    fn fast_frames_keep_only_512_samples_and_report_the_truncation() {
        let start = Instant::now();
        let mut samples = Samples::default();
        samples.observe(start, Some(phase()), 1.0);
        for frame in 1..5000_u64 {
            assert!(samples
                .observe(start + Duration::from_millis(frame), Some(phase()), 250.0)
                .is_none());
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
