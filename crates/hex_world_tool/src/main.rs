//! Supported prebuilt V4 authoring command. Map edits are runtime inputs.

mod preview;
mod replication_benchmark;
mod runtime_benchmark;

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use hex_schematic::v4::{compile_world, compile_world_cached, parse_world, WorldSpec};
use hex_world_contracts::{WorldManifest, WorldPackage};
use hex_world_runtime::{publish_revision, FileChunkSource, IoLimits};
use serde::{Deserialize, Serialize};

const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const USAGE: &str = "worldc validate --source WORLD.ron\nworldc compile --source WORLD.ron --output DIRECTORY\nworldc preview --package DIRECTORY --output REVIEW.html\nworldc inspect --package DIRECTORY\nworldc probe --package DIRECTORY --at q,r\nworldc benchmark --source WORLD.ron --output RECEIPT.json [--iterations 20]\nworldc edit-benchmark --series SERIES.ron --output RECEIPT.json\nworldc runtime-benchmark --series SERIES.ron --output RECEIPT.json\nworldc replication-benchmark --package DIRECTORY --output RECEIPT.json\n\nBuild worldc once. Authoring commands read source files at runtime and never invoke Cargo.\n";

fn main() -> ExitCode {
    match execute(std::env::args().skip(1)) {
        Ok(message) => match writeln!(io::stdout().lock(), "{message}") {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        },
        Err(error) => {
            let _written = writeln!(io::stderr().lock(), "worldc: {error}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Debug)]
struct Arguments {
    command: String,
    values: BTreeMap<String, String>,
}

impl Arguments {
    fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Self, Box<dyn Error>> {
        let mut arguments = arguments.into_iter();
        let command = arguments.next().unwrap_or_else(|| "help".to_owned());
        if command == "help" || command == "--help" || command == "-h" {
            return Ok(Self {
                command: "help".to_owned(),
                values: BTreeMap::new(),
            });
        }
        let allowed: &[&str] = match command.as_str() {
            "validate" => &["--source"],
            "compile" => &["--source", "--output"],
            "preview" | "replication-benchmark" => &["--package", "--output"],
            "replica-worker" => &["--package", "--save", "--connect"],
            "inspect" => &["--package"],
            "probe" => &["--package", "--at"],
            "benchmark" => &["--source", "--output", "--iterations"],
            "edit-benchmark" | "runtime-benchmark" => &["--series", "--output"],
            _ => return Err(format!("unknown command {command:?}\n{USAGE}").into()),
        };
        let mut values = BTreeMap::new();
        while let Some(flag) = arguments.next() {
            if !allowed.contains(&flag.as_str()) {
                return Err(format!("unknown argument {flag:?} for {command}").into());
            }
            let value = arguments
                .next()
                .ok_or_else(|| format!("missing value for {flag}"))?;
            if value.starts_with("--") || value.is_empty() {
                return Err(format!("missing value for {flag}").into());
            }
            if values.insert(flag.clone(), value).is_some() {
                return Err(format!("duplicate argument {flag}").into());
            }
        }
        for flag in allowed.iter().filter(|flag| **flag != "--iterations") {
            if !values.contains_key(*flag) {
                return Err(format!("{command} requires {flag}").into());
            }
        }
        Ok(Self { command, values })
    }

    fn path(&self, flag: &str) -> Result<PathBuf, Box<dyn Error>> {
        self.values
            .get(flag)
            .map(PathBuf::from)
            .ok_or_else(|| format!("missing {flag}").into())
    }
}

fn read_source(path: &Path) -> Result<WorldSpec, Box<dyn Error>> {
    let bytes = read_bounded(path)?;
    parse_world(std::str::from_utf8(&bytes)?)
        .map_err(|error| format!("{}: {error}", path.display()).into())
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_SOURCE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len())? > MAX_SOURCE_BYTES {
        return Err(format!("source {} exceeds 16 MiB", path.display()).into());
    }
    Ok(bytes)
}

fn read_manifest(directory: &Path) -> Result<WorldManifest, Box<dyn Error>> {
    let source = FileChunkSource::open_workspace(directory, IoLimits::default())?;
    Ok(source.manifest().clone())
}

fn execute(arguments: impl IntoIterator<Item = String>) -> Result<String, Box<dyn Error>> {
    let arguments = Arguments::parse(arguments)?;
    match arguments.command.as_str() {
        "help" => Ok(USAGE.to_owned()),
        "probe" => probe(
            &arguments.path("--package")?,
            arguments
                .values
                .get("--at")
                .ok_or("missing exact coordinate")?,
        ),
        "replication-benchmark" => {
            replication_benchmark::run(&arguments.path("--package")?, &arguments.path("--output")?)
        }
        "replica-worker" => replication_benchmark::worker(
            &arguments.path("--package")?,
            &arguments.path("--save")?,
            arguments
                .values
                .get("--connect")
                .ok_or("missing loopback endpoint")?,
        ),
        "validate" => {
            let source = read_source(&arguments.path("--source")?)?;
            let started = Instant::now();
            // Strict validation includes the actual compiled topology. It cannot
            // report schema-only success while a required route is invalid.
            let package = compile_world(&source)?;
            package.validate()?;
            Ok(format!(
                "Strict validation passed: {} regions, {} chunks, {:016x}, {:.3}s",
                package.manifest.regions.len(),
                package.chunks.len(),
                package.manifest.fingerprint,
                started.elapsed().as_secs_f64()
            ))
        }
        "compile" => {
            let path = arguments.path("--source")?;
            let source = read_source(&path)?;
            let started = Instant::now();
            let package = compile_world(&source)?;
            package.validate()?;
            let output = arguments.path("--output")?;
            let manifest_path = publish_revision(&output, &package, IoLimits::default())?;
            let package_directory = manifest_path
                .parent()
                .ok_or("published manifest has no parent")?;
            let receipt = CompileReceipt::new(&path, &package, started.elapsed().as_secs_f64());
            write_json(&package_directory.join("compile-receipt.json"), &receipt)?;
            write_json(&output.join("compile-receipt.json"), &receipt)?;
            preview::write(&package.manifest, &output.join("review.html"))?;
            Ok(format!(
                "Published {} regions / {} chunks at {} (fingerprint {:016x}, {:.3}s)",
                package.manifest.regions.len(),
                package.chunks.len(),
                output.display(),
                package.manifest.fingerprint,
                receipt.elapsed_seconds
            ))
        }
        "inspect" => {
            let manifest = read_manifest(&arguments.path("--package")?)?;
            Ok(serde_json::to_string_pretty(&InspectReceipt {
                world_id: &manifest.world_id,
                fingerprint: format!("{:016x}", manifest.fingerprint),
                regions: manifest.regions.len(),
                chunks: manifest.chunks.len(),
                summary_cells: manifest.summary.len(),
                boundaries: manifest.boundaries.len(),
                features: manifest.features.len(),
            })?)
        }
        "preview" => {
            let manifest = read_manifest(&arguments.path("--package")?)?;
            let output = arguments.path("--output")?;
            preview::write(&manifest, &output)?;
            Ok(format!("Geographic review written to {}", output.display()))
        }
        "benchmark" => benchmark(&arguments),
        "edit-benchmark" => edit_benchmark(&arguments),
        "runtime-benchmark" => {
            runtime_benchmark::run(&arguments.path("--series")?, &arguments.path("--output")?)
        }
        _ => Err("unrecognized command".into()),
    }
}

#[derive(Serialize)]
struct InspectReceipt<'a> {
    world_id: &'a str,
    fingerprint: String,
    regions: usize,
    chunks: usize,
    summary_cells: usize,
    boundaries: usize,
    features: usize,
}

#[derive(Serialize)]
struct CompileReceipt {
    tool: &'static str,
    tool_version: &'static str,
    source: String,
    source_fingerprint: String,
    package_fingerprint: String,
    regions: usize,
    chunks: usize,
    columns: usize,
    runs: usize,
    elapsed_seconds: f64,
    strict: bool,
    presentation_reviewed: bool,
}

impl CompileReceipt {
    fn new(source: &Path, package: &WorldPackage, elapsed_seconds: f64) -> Self {
        Self {
            tool: "worldc",
            tool_version: env!("CARGO_PKG_VERSION"),
            source: source.display().to_string(),
            source_fingerprint: format!("{:016x}", package.manifest.source_fingerprint),
            package_fingerprint: format!("{:016x}", package.manifest.fingerprint),
            regions: package.manifest.regions.len(),
            chunks: package.chunks.len(),
            columns: package
                .chunks
                .values()
                .map(|chunk| chunk.columns.len())
                .sum(),
            runs: package
                .chunks
                .values()
                .flat_map(|chunk| &chunk.columns)
                .map(|column| column.runs.len())
                .sum(),
            elapsed_seconds,
            strict: true,
            presentation_reviewed: false,
        }
    }
}

#[derive(Serialize)]
struct BenchmarkReceipt {
    kind: &'static str,
    iterations: usize,
    samples_seconds: Vec<f64>,
    p50_seconds: f64,
    p95_seconds: f64,
    maximum_seconds: f64,
    consistent_fingerprint: String,
    changing_source_edits: bool,
}

fn benchmark(arguments: &Arguments) -> Result<String, Box<dyn Error>> {
    let iterations: usize = arguments
        .values
        .get("--iterations")
        .map_or(Ok(20), |value| value.parse())?;
    if !(1..=1000).contains(&iterations) {
        return Err("iterations must be between 1 and 1000; zero work is invalid".into());
    }
    let source = read_source(&arguments.path("--source")?)?;
    let mut samples = Vec::with_capacity(iterations);
    let mut expected = None;
    for _ in 0..iterations {
        let started = Instant::now();
        let package = compile_world(&source)?;
        package.validate()?;
        samples.push(started.elapsed().as_secs_f64());
        if expected.is_some_and(|hash| hash != package.manifest.fingerprint) {
            return Err("unchanged source produced a different canonical package".into());
        }
        expected = Some(package.manifest.fingerprint);
    }
    let mut ordered = samples.clone();
    ordered.sort_by(f64::total_cmp);
    let percentile = |numerator: usize| {
        ordered
            .get((iterations * numerator).div_ceil(100).saturating_sub(1))
            .copied()
            .unwrap_or_default()
    };
    let receipt = BenchmarkReceipt {
        kind: "repeated strict compilation of unchanged source",
        iterations,
        samples_seconds: samples,
        p50_seconds: percentile(50),
        p95_seconds: percentile(95),
        maximum_seconds: ordered.last().copied().unwrap_or_default(),
        consistent_fingerprint: format!("{:016x}", expected.ok_or("benchmark ran no work")?),
        changing_source_edits: false,
    };
    write_json(&arguments.path("--output")?, &receipt)?;
    Ok(format!("{iterations} strict compiles: p50 {:.3}s, p95 {:.3}s. This is not a changing-edit authoring SLA measurement.", receipt.p50_seconds, receipt.p95_seconds))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EditSeries {
    version: u32,
    sources: Vec<PathBuf>,
}

#[derive(Serialize)]
struct EditSample {
    source: String,
    source_fingerprint: String,
    package_fingerprint: String,
    incremental_seconds: f64,
    clean_verification_seconds: f64,
    equivalent_to_clean: bool,
}

#[derive(Serialize)]
struct EditBenchmarkReceipt {
    kind: &'static str,
    samples: Vec<EditSample>,
    warm_edits: usize,
    p50_seconds: f64,
    p95_seconds: f64,
    target_sample_count_met: bool,
    elapsed_target_met: bool,
    active_authoring_hours: Option<f64>,
}

fn edit_benchmark(arguments: &Arguments) -> Result<String, Box<dyn Error>> {
    let series_path = arguments.path("--series")?;
    let series: EditSeries = ron::de::from_bytes(&read_bounded(&series_path)?)?;
    if series.version != 1 || !(2..=1001).contains(&series.sources.len()) {
        return Err("edit series requires version 1 and 2..=1001 source snapshots".into());
    }
    let mut cache = None;
    let mut previous_source = None;
    let mut samples = Vec::new();
    for source_path in series.sources {
        let path = series_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(source_path);
        let source = read_source(&path)?;
        let started = Instant::now();
        let compiled = compile_world_cached(&source, cache.as_ref())?;
        compiled.package.validate()?;
        let elapsed = started.elapsed().as_secs_f64();
        let fingerprint = compiled.package.manifest.source_fingerprint;
        if previous_source == Some(fingerprint) {
            return Err(format!("{} does not change the previous source; no-work edits cannot satisfy the benchmark", path.display()).into());
        }
        let verification_started = Instant::now();
        let clean = compile_world(&source)?;
        clean.validate()?;
        if compiled.package != clean {
            return Err(format!(
                "{} differs between incremental and clean compilation",
                path.display()
            )
            .into());
        }
        samples.push(EditSample {
            source: path.display().to_string(),
            source_fingerprint: format!("{fingerprint:016x}"),
            package_fingerprint: format!("{:016x}", clean.manifest.fingerprint),
            incremental_seconds: elapsed,
            clean_verification_seconds: verification_started.elapsed().as_secs_f64(),
            equivalent_to_clean: true,
        });
        previous_source = Some(fingerprint);
        cache = Some(compiled);
    }
    let mut warm = samples
        .iter()
        .skip(1)
        .map(|sample| sample.incremental_seconds)
        .collect::<Vec<_>>();
    warm.sort_by(f64::total_cmp);
    let percentile = |numerator: usize| {
        warm.get((warm.len() * numerator).div_ceil(100).saturating_sub(1))
            .copied()
            .unwrap_or_default()
    };
    let receipt = EditBenchmarkReceipt {
        kind: "changed source snapshots, incremental compilation verified against clean output",
        warm_edits: warm.len(),
        p50_seconds: percentile(50),
        p95_seconds: percentile(95),
        target_sample_count_met: warm.len() >= 20,
        elapsed_target_met: percentile(50) <= 30.0 && percentile(95) <= 90.0,
        active_authoring_hours: None,
        samples,
    };
    write_json(&arguments.path("--output")?, &receipt)?;
    Ok(format!("{} changed-source edits: p50 {:.3}s, p95 {:.3}s; every incremental result matches clean compilation. Active authoring time is unmeasured.", receipt.warm_edits, receipt.p50_seconds, receipt.p95_seconds))
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), Box<dyn Error>> {
    write_bytes(path, &serde_json::to_vec_pretty(value)?)
}

fn write_bytes(path: &Path, contents: &[u8]) -> Result<(), Box<dyn Error>> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    atomicwrites::AtomicFile::new(path, atomicwrites::AllowOverwrite).write(|file| {
        file.write_all(contents)?;
        file.sync_all()
    })?;
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

/// Resolve one exact column through the same residency/query authority as play.
fn probe(package: &Path, coordinate: &str) -> Result<String, Box<dyn Error>> {
    use hex_world_contracts::{QueryResult, ResidencyRequest, WorldHex, WorldQuery};
    use hex_world_runtime::{RuntimeConfig, WorldRuntime};
    let values = coordinate
        .split(',')
        .map(str::parse::<i64>)
        .collect::<Result<Vec<_>, _>>()?;
    let [q, r] = values.as_slice() else {
        return Err("probe --at must be q,r".into());
    };
    let column = WorldHex::new(*q, *r);
    let source = FileChunkSource::open_workspace(package, IoLimits::default())?;
    let mut runtime = WorldRuntime::new(std::sync::Arc::new(source), RuntimeConfig::default())?;
    runtime.set_interests(vec![ResidencyRequest {
        id: "authoring-probe".into(),
        center: column,
        radius: 0,
        retention_radius: 0,
        priority: 1,
    }])?;
    let start = Instant::now();
    loop {
        let update = runtime.pump();
        if let Some(failure) = update.failures.first() {
            return Err(failure.error.clone().into());
        }
        match runtime.surfaces(column) {
            QueryResult::Ready(surfaces) => {
                let product = runtime
                    .resident_chunk(column.chunk())
                    .ok_or("probe lost its resident source")?;
                return Ok(serde_json::to_string_pretty(&serde_json::json!({
                    "world_id": runtime.manifest().world_id,
                    "package_fingerprint": format!("{:016x}",runtime.manifest().fingerprint),
                    "column": column, "revision": product.revision, "surfaces": surfaces,
                    "terrain": product.package.columns.iter().find(|entry| entry.position == column),
                    "object_occupancy": product.package.semantics.occupancy.iter().find(|entry| entry.position == column),
                    "liquids": product.package.semantics.liquids.iter().filter(|entry| entry.column == column).collect::<Vec<_>>(),
                    "root_objects": product.package.semantics.objects.iter().filter(|object| object.origin.column == column).collect::<Vec<_>>()
                }))?);
            }
            QueryResult::OutsideWorld => {
                return Err("probe coordinate is outside the finite world".into())
            }
            QueryResult::Unloaded(_) => {}
        }
        if start.elapsed() > std::time::Duration::from_secs(10) {
            return Err("probe residency deadline exceeded".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(values: &[&str]) -> Result<Arguments, Box<dyn Error>> {
        Arguments::parse(values.iter().map(|value| (*value).to_owned()))
    }

    #[test]
    fn rejects_incomplete_unknown_and_duplicate_arguments() {
        for values in [
            vec!["compile", "--source", "a.ron"],
            vec!["inspect", "--source", "a.ron"],
            vec!["validate", "--source", "a.ron", "--source", "b.ron"],
            vec!["compile", "--source", "--output", "out"],
        ] {
            assert!(parse(&values).is_err(), "must reject {values:?}");
        }
    }

    #[test]
    fn zero_work_benchmark_is_rejected_before_accessing_source() {
        let arguments = parse(&[
            "benchmark",
            "--source",
            "missing.ron",
            "--output",
            "unused.json",
            "--iterations",
            "0",
        ])
        .expect("valid command shape");
        let error = benchmark(&arguments).expect_err("zero benchmark must fail");
        assert!(error.to_string().contains("zero work"));
    }
}
