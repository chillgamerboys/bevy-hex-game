//! Strict command-line parsing and orchestration for the schematic tool.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use atomicwrites::{AllowOverwrite, AtomicFile};
use hex_schematic::{
    canonical_cell_id, canonical_coordinates, CellId, SchematicCoord, SchematicMetricsV1,
    SchematicPlanV1, SchematicTemplateV1, SCHEMATIC_RADIUS, SCHEMATIC_SCHEMA_VERSION,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::render;

const STAGING_ATTEMPTS: u8 = 64;
static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Complete command-line usage text.
pub const USAGE: &str = "Usage:\n\
    hex_schematic grid --output <directory>\n\
    hex_schematic generate --template <file.ron> --seed <u64> --output <directory>\n\
    hex_schematic gallery --template <file.ron> --first-seed <u64> --output <directory>\n\
    hex_schematic validate --template <file.ron> [--plan <file.ron> [--metrics <file.ron>]]\n\
\n\
The gallery always contains exactly twelve consecutive seeds.\n";

/// One successfully parsed command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Emit the canonical radius-eight grid and its labelled projection.
    Grid {
        /// Destination directory.
        output: PathBuf,
    },
    /// Generate and render one deterministic plan.
    Generate {
        /// Strict designer template input.
        template: PathBuf,
        /// World seed.
        seed: u64,
        /// Destination directory.
        output: PathBuf,
    },
    /// Generate and atomically publish the twelve-seed gallery.
    Gallery {
        /// Strict designer template input.
        template: PathBuf,
        /// First of twelve consecutive world seeds.
        first_seed: u64,
        /// Destination directory.
        output: PathBuf,
    },
    /// Parse and validate a template and, optionally, one authoritative plan.
    Validate {
        /// Strict template input and validation authority.
        template: PathBuf,
        /// Optional strict plan input.
        plan: Option<PathBuf>,
        /// Optional plan metrics input. When absent, the sibling `metrics.ron` is
        /// checked only when it already exists.
        metrics: Option<PathBuf>,
    },
}

/// The result of parsing a command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseOutcome {
    /// A command is ready to execute.
    Command(Command),
    /// The caller should print usage and exit successfully.
    Help,
}

/// A strict command-line syntax error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    detail: String,
}

impl CliError {
    fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for CliError {}

/// A command execution or publication error.
#[derive(Debug)]
pub struct RunError {
    operation: &'static str,
    path: Option<PathBuf>,
    detail: String,
}

impl RunError {
    fn new(operation: &'static str, detail: impl Into<String>) -> Self {
        Self {
            operation,
            path: None,
            detail: detail.into(),
        }
    }

    fn at(operation: &'static str, path: &Path, detail: impl fmt::Display) -> Self {
        Self {
            operation,
            path: Some(path.to_path_buf()),
            detail: detail.to_string(),
        }
    }
}

impl fmt::Display for RunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(path) = &self.path {
            write!(
                formatter,
                "{} at {}: {}",
                self.operation,
                path.display(),
                self.detail
            )
        } else {
            write!(formatter, "{}: {}", self.operation, self.detail)
        }
    }
}

impl std::error::Error for RunError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CanonicalGridV1 {
    schema_version: u16,
    radius: u8,
    cells: Vec<CanonicalGridCell>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CanonicalGridCell {
    id: CellId,
    coord: SchematicCoord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BundleRecord {
    seed: u64,
    fingerprint: String,
    summary: String,
    projection: render::RenderPlan,
}

/// Executes one parsed command without weakening generator or validator policy.
pub fn execute(command: Command) -> Result<String, RunError> {
    match command {
        Command::Grid { output } => execute_grid(&output),
        Command::Generate {
            template,
            seed,
            output,
        } => execute_generate(&template, seed, &output),
        Command::Gallery {
            template,
            first_seed,
            output,
        } => execute_gallery(&template, first_seed, &output),
        Command::Validate {
            template,
            plan,
            metrics,
        } => execute_validate(&template, plan.as_deref(), metrics.as_deref()),
    }
}

fn execute_generate(template_path: &Path, seed: u64, output: &Path) -> Result<String, RunError> {
    let template = load_valid_template(template_path)?;
    let record = publish_new_directory(output, |staging| {
        let generated = hex_schematic::generator::generate(&template, seed)
            .map_err(|error| RunError::new("generate schematic", error.to_string()))?;
        write_plan_bundle(staging, &template, &generated.plan, &generated.metrics)
    })?;
    Ok(format!(
        "published seed {} fingerprint {} to {}",
        record.seed,
        record.fingerprint,
        output.display()
    ))
}

fn execute_gallery(
    template_path: &Path,
    first_seed: u64,
    output: &Path,
) -> Result<String, RunError> {
    let template = load_valid_template(template_path)?;
    let last_seed = first_seed.checked_add(11).ok_or_else(|| {
        RunError::new(
            "publish gallery",
            "first seed cannot describe twelve consecutive seeds",
        )
    })?;
    let published_records = publish_new_directory(output, |staging| {
        let reference_generated = hex_schematic::generator::reference_plan(&template, first_seed)
            .map_err(|error| {
            RunError::new("generate reference schematic", error.to_string())
        })?;
        let reference_record = write_plan_bundle(
            &staging.join("reference"),
            &template,
            &reference_generated.plan,
            &reference_generated.metrics,
        )?;
        let reference_entry = render::GalleryEntry {
            heading: "Canonical reference artifact".to_owned(),
            seed: None,
            fingerprint: reference_record.fingerprint,
            summary: reference_record.summary,
            composite_href: "reference/composite.svg".to_owned(),
            diagnostic_href: "reference/diagnostics.svg".to_owned(),
            plan_href: "reference/plan.ron".to_owned(),
            metrics_href: "reference/metrics.ron".to_owned(),
        };
        let mut entries = Vec::with_capacity(12);
        let mut records = Vec::with_capacity(12);
        let mut contact_plans = Vec::with_capacity(12);
        for seed in first_seed..=last_seed {
            let generated = hex_schematic::generator::generate(&template, seed)
                .map_err(|error| RunError::new("generate gallery seed", error.to_string()))?;
            let directory_name = seed_directory_name(seed);
            let record = write_plan_bundle(
                &staging.join(&directory_name),
                &template,
                &generated.plan,
                &generated.metrics,
            )?;
            entries.push(render::GalleryEntry {
                heading: format!("Seed {seed}"),
                seed: Some(seed),
                fingerprint: record.fingerprint.clone(),
                summary: record.summary.clone(),
                composite_href: format!("{directory_name}/composite.svg"),
                diagnostic_href: format!("{directory_name}/diagnostics.svg"),
                plan_href: format!("{directory_name}/plan.ron"),
                metrics_href: format!("{directory_name}/metrics.ron"),
            });
            contact_plans.push(record.projection.clone());
            records.push(record);
        }
        let contact = render::contact_sheet_svg(&entries, &contact_plans)
            .map_err(|error| RunError::new("render gallery contact sheet", error.to_string()))?;
        let html = render::complete_gallery_html(&entries, &reference_entry, "contact-sheet.svg")
            .map_err(|error| RunError::new("render gallery HTML", error.to_string()))?;
        write_atomic(
            &staging.join("contact-sheet.svg"),
            contact.as_bytes(),
            "write gallery contact sheet",
        )?;
        write_atomic(
            &staging.join("index.html"),
            html.as_bytes(),
            "write complete gallery HTML",
        )?;
        verify_gallery_stage(staging, &entries, &reference_entry, &contact, &html)?;
        Ok(records)
    })?;
    let fingerprint_span = published_records
        .first()
        .zip(published_records.last())
        .map_or_else(
            || "no fingerprints".to_owned(),
            |(first, last)| format!("fingerprints {}..{}", first.fingerprint, last.fingerprint),
        );
    Ok(format!(
        "published 12 validated seeds {}..={} ({}) to {}",
        first_seed,
        last_seed,
        fingerprint_span,
        output.display()
    ))
}

fn load_valid_template(path: &Path) -> Result<SchematicTemplateV1, RunError> {
    let template: SchematicTemplateV1 = read_ron(path, "read strict template RON")?;
    hex_schematic::validate::validate_template(&template)
        .map_err(|error| RunError::at("validate template", path, error))?;
    Ok(template)
}

fn seed_directory_name(seed: u64) -> String {
    format!("seed-{seed:020}")
}

fn verify_gallery_stage(
    staging: &Path,
    entries: &[render::GalleryEntry],
    reference: &render::GalleryEntry,
    contact: &str,
    html: &str,
) -> Result<(), RunError> {
    if entries.len() != 12 {
        return Err(RunError::at(
            "verify gallery",
            staging,
            format!("gallery contains {} entries; expected 12", entries.len()),
        ));
    }
    if reference.seed.is_some() {
        return Err(RunError::at(
            "verify gallery",
            staging,
            "reference artifact was not separately marked",
        ));
    }
    for (path, expected) in [
        (staging.join("contact-sheet.svg"), contact),
        (staging.join("index.html"), html),
    ] {
        if read_utf8(&path, "reload gallery artifact")? != expected {
            return Err(RunError::at(
                "verify gallery artifact",
                &path,
                "persisted bytes differ from deterministic renderer output",
            ));
        }
    }
    let root_entry_count = fs::read_dir(staging)
        .map_err(|error| RunError::at("inspect staged gallery", staging, error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| RunError::at("inspect staged gallery", staging, error))?
        .len();
    if root_entry_count != 15 {
        return Err(RunError::at(
            "verify staged gallery",
            staging,
            format!("gallery root has {root_entry_count} entries; expected 15"),
        ));
    }
    Ok(())
}

fn execute_grid(output: &Path) -> Result<String, RunError> {
    let grid = canonical_grid()?;
    let grid_ron = canonical_ron(&grid, "serialize canonical grid")?;
    let grid_svg = render::composite_svg(&render::grid_projection())
        .map_err(|error| RunError::new("render canonical grid", error.to_string()))?;
    publish_new_directory(output, |staging| {
        write_atomic(
            &staging.join("grid.ron"),
            &grid_ron,
            "write canonical grid RON",
        )?;
        write_atomic(
            &staging.join("grid.svg"),
            grid_svg.as_bytes(),
            "write canonical grid SVG",
        )?;
        let persisted: CanonicalGridV1 =
            read_ron(&staging.join("grid.ron"), "verify canonical grid RON")?;
        if persisted != grid {
            return Err(RunError::at(
                "verify canonical grid RON",
                &staging.join("grid.ron"),
                "round trip changed canonical grid facts",
            ));
        }
        Ok(())
    })?;
    Ok(format!("published canonical grid to {}", output.display()))
}

fn execute_validate(
    template_path: &Path,
    plan_path: Option<&Path>,
    metrics_path: Option<&Path>,
) -> Result<String, RunError> {
    let template: SchematicTemplateV1 = read_ron(template_path, "read strict template RON")?;
    hex_schematic::validate::validate_template(&template)
        .map_err(|error| RunError::at("validate template", template_path, error))?;
    let Some(plan_path) = plan_path else {
        return Ok(format!(
            "validated template {} revision {}",
            template.id, template.revision
        ));
    };
    let plan: SchematicPlanV1 = read_ron(plan_path, "read strict plan RON")?;
    let actual_metrics = hex_schematic::validate::validate_plan(&template, &plan)
        .map_err(|error| RunError::at("validate plan", plan_path, error))?;
    let metrics_path = if let Some(path) = metrics_path {
        Some(path.to_path_buf())
    } else {
        let sibling = usable_parent(plan_path).join("metrics.ron");
        if path_exists(&sibling, "inspect sibling metrics")? {
            Some(sibling)
        } else {
            None
        }
    };
    if let Some(metrics_path) = metrics_path {
        let persisted: SchematicMetricsV1 = read_ron(&metrics_path, "read strict metrics RON")?;
        if persisted != actual_metrics {
            return Err(RunError::at(
                "validate metrics",
                &metrics_path,
                "persisted metrics differ from freshly recomputed plan metrics",
            ));
        }
    }
    Ok(format!(
        "validated plan seed {} fingerprint {:016x}",
        plan.provenance.world_seed, plan.semantic_fingerprint
    ))
}

fn canonical_grid() -> Result<CanonicalGridV1, RunError> {
    let cells = canonical_coordinates()
        .into_iter()
        .map(|coord| {
            canonical_cell_id(coord)
                .map(|id| CanonicalGridCell { id, coord })
                .ok_or_else(|| {
                    RunError::new(
                        "construct canonical grid",
                        format!(
                            "canonical coordinate ({}, {}, {}) has no canonical cell identity",
                            coord.q(),
                            coord.r(),
                            coord.s()
                        ),
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CanonicalGridV1 {
        schema_version: SCHEMATIC_SCHEMA_VERSION,
        radius: SCHEMATIC_RADIUS,
        cells,
    })
}

fn metric_summary(metrics: &SchematicMetricsV1) -> Vec<(String, String)> {
    vec![
        ("cells".to_owned(), metrics.cell_count.to_string()),
        (
            "surface-land/water".to_owned(),
            format!("{}/{}", metrics.surfaces.land, metrics.surfaces.open_water),
        ),
        (
            "mountain/massif/peak".to_owned(),
            format!(
                "{}/{}/{}",
                metrics.landforms.mountain, metrics.landforms.massif, metrics.landforms.sharp_peak
            ),
        ),
        (
            "coast-moved/max".to_owned(),
            format!(
                "{}/{}",
                metrics.moved_coast_cells, metrics.maximum_coast_displacement
            ),
        ),
        (
            "valley-lake".to_owned(),
            metrics.valley_lake_cells.to_string(),
        ),
        (
            "sea-islands groups/cells".to_owned(),
            format!("{}/{}", metrics.sea_island_groups, metrics.sea_island_cells),
        ),
        (
            "woodland selected/eligible/%".to_owned(),
            format!(
                "{}/{}/{}",
                metrics.woodland_cells, metrics.eligible_woodland_cells, metrics.woodland_percent
            ),
        ),
        (
            "features/networks/edges".to_owned(),
            format!(
                "{}/{}/{}",
                metrics.feature_claims, metrics.networks, metrics.network_edges
            ),
        ),
    ]
}

fn write_plan_bundle(
    destination: &Path,
    template: &SchematicTemplateV1,
    plan: &SchematicPlanV1,
    metrics: &SchematicMetricsV1,
) -> Result<BundleRecord, RunError> {
    let recomputed = hex_schematic::validate::validate_plan(template, plan)
        .map_err(|error| RunError::at("validate generated plan", destination, error))?;
    if &recomputed != metrics {
        return Err(RunError::at(
            "verify generated metrics",
            destination,
            "generator metrics differ from validator recomputation",
        ));
    }
    prepare_empty_bundle_directory(destination)?;
    let plan_ron = canonical_ron(plan, "serialize canonical plan RON")?;
    let metrics_ron = canonical_ron(metrics, "serialize canonical metrics RON")?;
    let projection = render::plan_projection(template, plan, metric_summary(metrics))
        .map_err(|error| RunError::new("project generated plan", error.to_string()))?;
    let composite = render::composite_svg(&projection)
        .map_err(|error| RunError::new("render composite SVG", error.to_string()))?;
    let diagnostics = render::diagnostic_svg(&projection)
        .map_err(|error| RunError::new("render diagnostic SVG", error.to_string()))?;

    for (name, bytes, operation) in [
        ("plan.ron", plan_ron.as_slice(), "write canonical plan RON"),
        (
            "metrics.ron",
            metrics_ron.as_slice(),
            "write canonical metrics RON",
        ),
        ("composite.svg", composite.as_bytes(), "write composite SVG"),
        (
            "diagnostics.svg",
            diagnostics.as_bytes(),
            "write diagnostic SVG",
        ),
    ] {
        write_atomic(&destination.join(name), bytes, operation)?;
    }

    let persisted_plan: SchematicPlanV1 =
        read_ron(&destination.join("plan.ron"), "reload generated plan RON")?;
    let persisted_metrics: SchematicMetricsV1 = read_ron(
        &destination.join("metrics.ron"),
        "reload generated metrics RON",
    )?;
    let persisted_recomputed = hex_schematic::validate::validate_plan(template, &persisted_plan)
        .map_err(|error| {
            RunError::at(
                "validate reloaded generated plan",
                &destination.join("plan.ron"),
                error,
            )
        })?;
    if &persisted_plan != plan || &persisted_metrics != metrics || &persisted_recomputed != metrics
    {
        return Err(RunError::at(
            "verify reloaded plan bundle",
            destination,
            "persisted typed artifacts differ from validated generated values",
        ));
    }
    for (name, expected) in [
        ("composite.svg", composite.as_str()),
        ("diagnostics.svg", diagnostics.as_str()),
    ] {
        let path = destination.join(name);
        if read_utf8(&path, "reload rendered SVG")? != expected {
            return Err(RunError::at(
                "verify rendered SVG",
                &path,
                "persisted SVG differs from deterministic renderer output",
            ));
        }
    }

    Ok(BundleRecord {
        seed: plan.provenance.world_seed,
        fingerprint: format!("{:016x}", plan.semantic_fingerprint),
        summary: format!(
            "{} land · {} water · {} island groups · {}% woodland",
            metrics.surfaces.land,
            metrics.surfaces.open_water,
            metrics.sea_island_groups,
            metrics.woodland_percent
        ),
        projection,
    })
}

fn prepare_empty_bundle_directory(destination: &Path) -> Result<(), RunError> {
    match fs::create_dir(destination) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let mut entries = fs::read_dir(destination)
                .map_err(|cause| RunError::at("inspect plan bundle", destination, cause))?;
            match entries.next() {
                None => Ok(()),
                Some(Ok(_)) => Err(RunError::at(
                    "prepare plan bundle",
                    destination,
                    "bundle directory is not empty",
                )),
                Some(Err(cause)) => Err(RunError::at("inspect plan bundle", destination, cause)),
            }
        }
        Err(error) => Err(RunError::at("create plan bundle", destination, error)),
    }
}

fn canonical_ron<T>(value: &T, operation: &'static str) -> Result<Vec<u8>, RunError>
where
    T: Serialize + DeserializeOwned + PartialEq,
{
    let config = ron::ser::PrettyConfig::default()
        .new_line("\n")
        .indentor("    ");
    let mut source = ron::ser::to_string_pretty(value, config)
        .map_err(|error| RunError::new(operation, error.to_string()))?;
    source.push('\n');
    let decoded: T =
        ron::from_str(&source).map_err(|error| RunError::new(operation, error.to_string()))?;
    if &decoded != value {
        return Err(RunError::new(
            operation,
            "canonical RON round trip changed typed content",
        ));
    }
    Ok(source.into_bytes())
}

fn read_ron<T: DeserializeOwned>(path: &Path, operation: &'static str) -> Result<T, RunError> {
    let source = read_utf8(path, operation)?;
    ron::from_str(&source).map_err(|error| RunError::at(operation, path, error))
}

/// Parses arguments after the executable name.
///
/// Subcommands and option names must be UTF-8 and exact. Path values remain native
/// [`OsString`] values, so non-UTF-8 filesystem paths continue to work.
pub fn parse_args(arguments: impl IntoIterator<Item = OsString>) -> Result<ParseOutcome, CliError> {
    let mut arguments = arguments.into_iter();
    let Some(subcommand) = arguments.next() else {
        return Err(CliError::new("missing subcommand"));
    };
    if subcommand == OsStr::new("--help") || subcommand == OsStr::new("-h") {
        ensure_no_more(arguments)?;
        return Ok(ParseOutcome::Help);
    }
    let subcommand = subcommand
        .to_str()
        .ok_or_else(|| CliError::new("subcommand must be valid UTF-8"))?;
    let command = match subcommand {
        "grid" => parse_grid(arguments)?,
        "generate" => parse_generate(arguments)?,
        "gallery" => parse_gallery(arguments)?,
        "validate" => parse_validate(arguments)?,
        unknown => {
            return Err(CliError::new(format!(
                "unknown subcommand {unknown:?}; expected grid, generate, gallery, or validate"
            )));
        }
    };
    Ok(ParseOutcome::Command(command))
}

fn parse_grid(arguments: impl Iterator<Item = OsString>) -> Result<Command, CliError> {
    let mut output = None;
    parse_options(arguments, |option, value| match option {
        "--output" => set_once(&mut output, "--output", PathBuf::from(value)),
        _ => Err(unknown_option("grid", option)),
    })?;
    Ok(Command::Grid {
        output: required(output, "grid", "--output")?,
    })
}

fn parse_generate(arguments: impl Iterator<Item = OsString>) -> Result<Command, CliError> {
    let mut template = None;
    let mut seed = None;
    let mut output = None;
    parse_options(arguments, |option, value| match option {
        "--template" => set_once(&mut template, "--template", PathBuf::from(value)),
        "--seed" => set_once(&mut seed, "--seed", parse_seed(&value, "--seed")?),
        "--output" => set_once(&mut output, "--output", PathBuf::from(value)),
        _ => Err(unknown_option("generate", option)),
    })?;
    Ok(Command::Generate {
        template: required(template, "generate", "--template")?,
        seed: required(seed, "generate", "--seed")?,
        output: required(output, "generate", "--output")?,
    })
}

fn parse_gallery(arguments: impl Iterator<Item = OsString>) -> Result<Command, CliError> {
    let mut template = None;
    let mut first_seed = None;
    let mut output = None;
    parse_options(arguments, |option, value| match option {
        "--template" => set_once(&mut template, "--template", PathBuf::from(value)),
        "--first-seed" => set_once(
            &mut first_seed,
            "--first-seed",
            parse_seed(&value, "--first-seed")?,
        ),
        "--output" => set_once(&mut output, "--output", PathBuf::from(value)),
        _ => Err(unknown_option("gallery", option)),
    })?;
    let first_seed = required(first_seed, "gallery", "--first-seed")?;
    first_seed
        .checked_add(11)
        .ok_or_else(|| CliError::new("--first-seed cannot describe twelve consecutive seeds"))?;
    Ok(Command::Gallery {
        template: required(template, "gallery", "--template")?,
        first_seed,
        output: required(output, "gallery", "--output")?,
    })
}

fn parse_validate(arguments: impl Iterator<Item = OsString>) -> Result<Command, CliError> {
    let mut template = None;
    let mut plan = None;
    let mut metrics = None;
    parse_options(arguments, |option, value| match option {
        "--template" => set_once(&mut template, "--template", PathBuf::from(value)),
        "--plan" => set_once(&mut plan, "--plan", PathBuf::from(value)),
        "--metrics" => set_once(&mut metrics, "--metrics", PathBuf::from(value)),
        _ => Err(unknown_option("validate", option)),
    })?;
    if plan.is_none() && metrics.is_some() {
        return Err(CliError::new(
            "validate accepts --metrics only when --plan is supplied",
        ));
    }
    Ok(Command::Validate {
        template: required(template, "validate", "--template")?,
        plan,
        metrics,
    })
}

fn parse_options(
    mut arguments: impl Iterator<Item = OsString>,
    mut accept: impl FnMut(&str, OsString) -> Result<(), CliError>,
) -> Result<(), CliError> {
    while let Some(option) = arguments.next() {
        if option == OsStr::new("--help") || option == OsStr::new("-h") {
            return Err(CliError::new(
                "--help is accepted only in place of a subcommand",
            ));
        }
        let option = option
            .to_str()
            .ok_or_else(|| CliError::new("option names must be valid UTF-8"))?;
        if !option.starts_with("--") {
            return Err(CliError::new(format!(
                "unexpected positional argument {option:?}"
            )));
        }
        let value = arguments
            .next()
            .ok_or_else(|| CliError::new(format!("option {option} requires a value")))?;
        if value.is_empty() {
            return Err(CliError::new(format!(
                "option {option} requires a non-empty value"
            )));
        }
        accept(option, value)?;
    }
    Ok(())
}

fn set_once<T>(slot: &mut Option<T>, option: &str, value: T) -> Result<(), CliError> {
    if slot.replace(value).is_some() {
        Err(CliError::new(format!(
            "option {option} may be supplied only once"
        )))
    } else {
        Ok(())
    }
}

fn required<T>(value: Option<T>, subcommand: &str, option: &str) -> Result<T, CliError> {
    value.ok_or_else(|| CliError::new(format!("{subcommand} requires {option}")))
}

fn unknown_option(subcommand: &str, option: &str) -> CliError {
    CliError::new(format!("unknown {subcommand} option {option:?}"))
}

fn parse_seed(value: &OsStr, option: &str) -> Result<u64, CliError> {
    let value = value
        .to_str()
        .ok_or_else(|| CliError::new(format!("{option} must be valid UTF-8 decimal digits")))?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(CliError::new(format!(
            "{option} must contain only decimal digits"
        )));
    }
    value
        .parse()
        .map_err(|error| CliError::new(format!("invalid {option} value {value:?}: {error}")))
}

fn ensure_no_more(mut arguments: impl Iterator<Item = OsString>) -> Result<(), CliError> {
    if let Some(argument) = arguments.next() {
        Err(CliError::new(format!(
            "unexpected argument after --help: {}",
            argument.to_string_lossy()
        )))
    } else {
        Ok(())
    }
}

fn read_utf8(path: &Path, operation: &'static str) -> Result<String, RunError> {
    let bytes = fs::read(path).map_err(|error| RunError::at(operation, path, error))?;
    String::from_utf8(bytes).map_err(|error| RunError::at(operation, path, error))
}

fn write_atomic(path: &Path, bytes: &[u8], operation: &'static str) -> Result<(), RunError> {
    let parent = usable_parent(path);
    fs::create_dir_all(parent)
        .map_err(|error| RunError::at("create output parent", parent, error))?;
    AtomicFile::new(path, AllowOverwrite)
        .write(|file| file.write_all(bytes))
        .map_err(|error| RunError::at(operation, path, error))
}

fn publish_new_directory<T>(
    destination: &Path,
    build: impl FnOnce(&Path) -> Result<T, RunError>,
) -> Result<T, RunError> {
    if path_exists(destination, "inspect publication destination")? {
        return Err(RunError::at(
            "publish directory",
            destination,
            "destination already exists; refusing to replace or invalidate it",
        ));
    }
    let parent = usable_parent(destination);
    fs::create_dir_all(parent)
        .map_err(|error| RunError::at("create publication parent", parent, error))?;
    let staging = create_staging_directory(parent, destination)?;
    let mut staging = StagingDirectory::new(staging);
    let publication = build(staging.path())?;
    if path_exists(destination, "recheck publication destination")? {
        return Err(RunError::at(
            "publish directory",
            destination,
            "destination appeared while output was staged; refusing to overwrite it",
        ));
    }
    rename_directory_no_replace(staging.path(), destination)
        .map_err(|error| RunError::at("atomically publish directory", destination, error))?;
    staging.disarm();
    Ok(publication)
}

#[cfg(any(target_vendor = "apple", target_os = "linux", target_os = "redox"))]
fn rename_directory_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        source,
        rustix::fs::CWD,
        destination,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(io::Error::from)
}

#[cfg(target_os = "windows")]
fn rename_directory_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    // Windows directory rename already fails when the destination exists.
    fs::rename(source, destination)
}

#[cfg(not(any(
    target_vendor = "apple",
    target_os = "linux",
    target_os = "redox",
    target_os = "windows"
)))]
fn rename_directory_no_replace(_source: &Path, _destination: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "this platform has no supported atomic no-replace directory rename",
    ))
}

fn usable_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn path_exists(path: &Path, operation: &'static str) -> Result<bool, RunError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(RunError::at(operation, path, error)),
    }
}

fn create_staging_directory(parent: &Path, destination: &Path) -> Result<PathBuf, RunError> {
    let base = destination
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            RunError::at(
                "prepare publication",
                destination,
                "destination must have a non-empty final path component",
            )
        })?;
    for _attempt in 0..STAGING_ATTEMPTS {
        let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut name = OsString::from(".");
        name.push(base);
        name.push(format!(".{}.{}.staging", std::process::id(), sequence));
        let path = parent.join(name);
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(RunError::at(
                    "create publication staging directory",
                    &path,
                    error,
                ));
            }
        }
    }
    Err(RunError::at(
        "create publication staging directory",
        parent,
        format!("no unique staging name after {STAGING_ATTEMPTS} attempts"),
    ))
}

#[derive(Debug)]
struct StagingDirectory {
    path: PathBuf,
    armed: bool,
}

impl StagingDirectory {
    const fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    const fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        if self.armed {
            let _cleanup = fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "hex-schematic-cli-{label}-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("unique test directory should be created");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _cleanup = fs::remove_dir_all(&self.0);
        }
    }

    fn parse(arguments: &[&str]) -> Result<ParseOutcome, CliError> {
        parse_args(arguments.iter().map(OsString::from))
    }

    #[test]
    fn parses_each_exact_command_shape() {
        assert_eq!(
            parse(&["grid", "--output", "out"]),
            Ok(ParseOutcome::Command(Command::Grid {
                output: PathBuf::from("out"),
            }))
        );
        assert_eq!(
            parse(&[
                "generate",
                "--seed",
                "42",
                "--output",
                "out",
                "--template",
                "template.ron",
            ]),
            Ok(ParseOutcome::Command(Command::Generate {
                template: PathBuf::from("template.ron"),
                seed: 42,
                output: PathBuf::from("out"),
            }))
        );
        assert_eq!(
            parse(&[
                "gallery",
                "--template",
                "template.ron",
                "--first-seed",
                "7",
                "--output",
                "gallery",
            ]),
            Ok(ParseOutcome::Command(Command::Gallery {
                template: PathBuf::from("template.ron"),
                first_seed: 7,
                output: PathBuf::from("gallery"),
            }))
        );
        assert_eq!(
            parse(&[
                "validate",
                "--template",
                "template.ron",
                "--plan",
                "plan.ron",
            ]),
            Ok(ParseOutcome::Command(Command::Validate {
                template: PathBuf::from("template.ron"),
                plan: Some(PathBuf::from("plan.ron")),
                metrics: None,
            }))
        );
        assert_eq!(
            parse(&["validate", "--template", "template.ron"]),
            Ok(ParseOutcome::Command(Command::Validate {
                template: PathBuf::from("template.ron"),
                plan: None,
                metrics: None,
            }))
        );
    }

    #[test]
    fn rejects_unknown_duplicate_missing_and_positional_inputs() {
        for arguments in [
            &["unknown"][..],
            &["grid"][..],
            &["grid", "out"][..],
            &["grid", "--where", "out"][..],
            &["grid", "--output"][..],
            &["grid", "--output", "one", "--output", "two"][..],
            &[
                "generate",
                "--template",
                "t",
                "--seed",
                "-1",
                "--output",
                "o",
            ][..],
            &["validate", "--plan", "plan.ron"][..],
            &[
                "validate",
                "--template",
                "template.ron",
                "--metrics",
                "metrics.ron",
            ][..],
        ] {
            assert!(parse(arguments).is_err(), "accepted {arguments:?}");
        }
    }

    #[test]
    fn rejects_gallery_seed_range_overflow() {
        assert!(parse(&[
            "gallery",
            "--template",
            "template.ron",
            "--first-seed",
            &u64::MAX.to_string(),
            "--output",
            "gallery",
        ])
        .is_err());
    }

    #[test]
    fn directory_publication_is_all_or_nothing_and_never_replaces() {
        let root = TestDirectory::new("publication");
        let failed = root.path().join("failed");
        let error = publish_new_directory(&failed, |staging| -> Result<(), RunError> {
            fs::write(staging.join("partial.txt"), "partial")
                .map_err(|cause| RunError::at("write fixture", staging, cause))?;
            Err(RunError::new("fixture", "abort"))
        })
        .expect_err("failed builder must fail publication");
        assert!(error.to_string().contains("abort"));
        assert!(!failed.exists());

        let published = root.path().join("published");
        publish_new_directory(&published, |staging| {
            fs::write(staging.join("complete.txt"), "complete")
                .map_err(|cause| RunError::at("write fixture", staging, cause))
        })
        .expect("complete builder should publish");
        assert_eq!(
            fs::read_to_string(published.join("complete.txt"))
                .expect("published fixture should be readable"),
            "complete"
        );

        let second = publish_new_directory(&published, |_staging| Ok(()))
            .expect_err("existing destination must be rejected");
        assert!(second.to_string().contains("already exists"));
        assert_eq!(
            fs::read_to_string(published.join("complete.txt"))
                .expect("existing publication should remain untouched"),
            "complete"
        );
    }

    #[test]
    fn atomic_directory_publish_refuses_a_destination_created_at_rename_time() {
        let root = TestDirectory::new("publication-race");
        let staging = root.path().join("staging");
        let destination = root.path().join("destination");
        fs::create_dir(&staging).expect("staging directory should be created");
        fs::write(staging.join("generated.txt"), "generated")
            .expect("staged fixture should be written");

        // This destination represents another publisher winning after the
        // caller's last existence check and immediately before the rename.
        fs::create_dir(&destination).expect("late destination should be created");
        rename_directory_no_replace(&staging, &destination)
            .expect_err("atomic no-replace rename must reject the late destination");
        assert!(destination.is_dir());
        assert_eq!(
            fs::read_dir(&destination)
                .expect("late destination must remain readable")
                .count(),
            0,
            "late-created empty destination must remain untouched"
        );
        assert_eq!(
            fs::read_to_string(staging.join("generated.txt"))
                .expect("rejected staging directory must remain intact"),
            "generated"
        );
    }

    #[test]
    fn atomic_file_write_replaces_complete_bytes() {
        let root = TestDirectory::new("atomic-file");
        let path = root.path().join("output.txt");
        write_atomic(&path, b"first", "write fixture").expect("first write succeeds");
        write_atomic(&path, b"second", "write fixture").expect("replacement succeeds");
        assert_eq!(
            read_utf8(&path, "read fixture").expect("fixture should be UTF-8"),
            "second"
        );
    }

    #[test]
    fn grid_command_publishes_one_complete_checked_directory() {
        let root = TestDirectory::new("grid-command");
        let destination = root.path().join("grid");
        let summary = execute_grid(&destination).expect("grid command should publish");
        assert!(summary.contains("canonical grid"));
        let grid: CanonicalGridV1 =
            read_ron(&destination.join("grid.ron"), "read grid test artifact")
                .expect("grid RON should reload");
        assert_eq!(grid.cells.len(), hex_schematic::SCHEMATIC_CELL_COUNT);
        let svg = read_utf8(&destination.join("grid.svg"), "read grid test SVG")
            .expect("grid SVG should reload");
        assert!(svg.contains("role=\"img\""));
        assert_eq!(
            svg.matches("class=\"authorship-outline authorship-grid\"")
                .count(),
            hex_schematic::SCHEMATIC_CELL_COUNT,
        );
        assert!(!svg.contains("authorship-locked\" points="));

        let first_ron =
            fs::read(destination.join("grid.ron")).expect("published grid RON should be readable");
        assert!(execute_grid(&destination).is_err());
        assert_eq!(
            fs::read(destination.join("grid.ron"))
                .expect("rejected rerun must retain first publication"),
            first_ron
        );
    }
}
