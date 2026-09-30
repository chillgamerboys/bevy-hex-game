//! Exact residency measurements. These do not claim renderer or movement acceptance.

use hex_world_contracts::{QueryResult, ResidencyRequest, WorldHex, WorldQuery};
use hex_world_runtime::{FileChunkSource, IoLimits, RuntimeConfig, WorldRuntime};
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Series {
    version: u32,
    radius: u32,
    retention_radius: u32,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    name: String,
    package: String,
    /// Successive interest sets; each may contain several separated sources.
    footprints: Vec<Vec<WorldHex>>,
}

#[derive(Serialize)]
struct Receipt {
    kind: &'static str,
    radius: u32,
    retention_radius: u32,
    cases: Vec<CaseReceipt>,
}

#[derive(Serialize)]
struct CaseReceipt {
    name: String,
    package_fingerprint: String,
    catalogue_regions: usize,
    catalogue_chunks: usize,
    open_seconds: f64,
    peak_resident_chunks: usize,
    peak_jobs: usize,
    steps: Vec<StepReceipt>,
}

#[derive(Serialize)]
struct StepReceipt {
    interests: Vec<WorldHex>,
    settlement_seconds: f64,
    resident_chunks: usize,
    resident_columns: usize,
    loaded_chunks: usize,
    removed_chunks: usize,
    pump_p95_milliseconds: f64,
    pump_max_milliseconds: f64,
    surface_queries: usize,
    query_microseconds_per_call: f64,
}

pub(super) fn run(series_path: &Path, output: &Path) -> Result<String, Box<dyn Error>> {
    let series: Series = ron::de::from_bytes(&super::read_bounded(series_path)?)?;
    if series.version != 1
        || series.cases.is_empty()
        || series.cases.len() > 64
        || series.radius == 0
        || series.radius > 96
        || series.retention_radius < series.radius
        || series.retention_radius > 128
    {
        return Err("invalid residency benchmark version, cases or local radius".into());
    }
    let parent = series_path.parent().unwrap_or_else(|| Path::new("."));
    let mut names = std::collections::BTreeSet::new();
    let mut receipts = Vec::new();
    for case in series.cases {
        if case.name.is_empty()
            || !names.insert(case.name.clone())
            || case.footprints.len() < 2
            || case.footprints.len() > 100
            || case
                .footprints
                .iter()
                .any(|step| step.is_empty() || step.len() > 8)
            || case
                .footprints
                .windows(2)
                .all(|pair| matches!(pair, [first, second] if first == second))
        {
            return Err(
                "each uniquely named residency case needs changing, bounded interest sets".into(),
            );
        }
        let started = Instant::now();
        let directory = parent.join(&case.package);
        let source = FileChunkSource::open_workspace(&directory, IoLimits::default())?;
        let mut runtime = WorldRuntime::new(
            Arc::new(source),
            RuntimeConfig {
                max_resident_chunks: 768,
                max_in_flight_jobs: 2,
                max_publications_per_pump: 2,
                ..RuntimeConfig::default()
            },
        )?;
        let mut receipt = CaseReceipt {
            name: case.name,
            package_fingerprint: format!("{:016x}", runtime.manifest().fingerprint),
            catalogue_regions: runtime.manifest().regions.len(),
            catalogue_chunks: runtime.manifest().chunks.len(),
            open_seconds: started.elapsed().as_secs_f64(),
            peak_resident_chunks: 0,
            peak_jobs: 0,
            steps: Vec::new(),
        };
        for centers in case.footprints {
            let started = Instant::now();
            runtime.set_interests(
                centers
                    .iter()
                    .enumerate()
                    .map(|(index, center)| ResidencyRequest {
                        id: format!("benchmark/{index}"),
                        center: *center,
                        radius: series.radius,
                        retention_radius: series.retention_radius,
                        priority: 10,
                    })
                    .collect(),
            )?;
            let mut pump_samples = Vec::new();
            let mut loaded_chunks = 0;
            let mut removed_chunks = 0;
            loop {
                let pump_started = Instant::now();
                let update = runtime.pump();
                pump_samples.push(pump_started.elapsed().as_secs_f64() * 1000.0);
                if let Some(failure) = update.failures.first() {
                    return Err(failure.error.clone().into());
                }
                loaded_chunks += update.loaded.len();
                removed_chunks += update.removed.len();
                let counts = runtime.counts();
                receipt.peak_resident_chunks =
                    receipt.peak_resident_chunks.max(counts.resident_chunks);
                receipt.peak_jobs = receipt.peak_jobs.max(counts.in_flight_jobs);
                if counts.queued_chunks == 0 && counts.in_flight_jobs == 0 {
                    break;
                }
                if started.elapsed() > Duration::from_secs(120) {
                    return Err("residency settlement deadline exceeded".into());
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            let settlement_seconds = started.elapsed().as_secs_f64();
            for center in &centers {
                if !matches!(runtime.surfaces(*center), QueryResult::Ready(_)) {
                    return Err(
                        format!("settled benchmark center {center:?} is not available").into(),
                    );
                }
            }
            let query_started = Instant::now();
            let surface_queries = 10_000;
            for center in centers.iter().cycle().take(surface_queries) {
                std::hint::black_box(runtime.surfaces(*center));
            }
            let query_microseconds_per_call = query_started.elapsed().as_secs_f64() * 100.0;
            pump_samples.sort_by(f64::total_cmp);
            let p95 = pump_samples
                .get((pump_samples.len() * 95).div_ceil(100).saturating_sub(1))
                .copied()
                .ok_or("residency benchmark performed zero pump work")?;
            receipt.steps.push(StepReceipt {
                interests: centers,
                settlement_seconds,
                resident_chunks: runtime.counts().resident_chunks,
                resident_columns: runtime
                    .resident_chunks()
                    .map(|chunk| chunk.package.columns.len())
                    .sum(),
                loaded_chunks,
                removed_chunks,
                pump_p95_milliseconds: p95,
                pump_max_milliseconds: pump_samples.last().copied().unwrap_or(0.0),
                surface_queries,
                query_microseconds_per_call,
            });
        }
        receipts.push(receipt);
    }
    let count = receipts.len();
    super::write_json(
        output,
        &Receipt {
            kind: "exact terrain residency only; renderer and actor movement are separate gates",
            radius: series.radius,
            retention_radius: series.retention_radius,
            cases: receipts,
        },
    )?;
    Ok(format!(
        "Measured {count} changing residency cases; receipt {}",
        output.display()
    ))
}
