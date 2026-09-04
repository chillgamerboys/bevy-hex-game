//! Compiler orchestration, declared dependencies, shared boundary authority and diagnostics.
use super::{
    geometry,
    model::*,
    operators::{self, RegionBuild},
};
use hex_world_contracts::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
    time::Instant,
};

const COMPILER_VERSION: &str = "hex-authoring/1";

/// One contextual, actionable authoring/compiler rejection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompileDiagnostic {
    /// Source region, connection, recipe or world identity.
    pub context: String,
    /// Compiler stage that rejected its input.
    pub stage: String,
    /// Precise reason; no partial package is published.
    pub message: String,
}

/// A failed compilation. All contained diagnostics refer to the submitted source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompileDiagnostics {
    /// Collected source validation errors or contextual generation failure.
    pub diagnostics: Vec<CompileDiagnostic>,
}
impl CompileDiagnostics {
    pub(super) fn one(
        context: impl Into<String>,
        stage: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            diagnostics: vec![CompileDiagnostic {
                context: context.into(),
                stage: stage.into(),
                message: message.into(),
            }],
        }
    }
}
impl fmt::Display for CompileDiagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for diagnostic in &self.diagnostics {
            writeln!(
                f,
                "{} [{}]: {}",
                diagnostic.context, diagnostic.stage, diagnostic.message
            )?;
        }
        Ok(())
    }
}
impl std::error::Error for CompileDiagnostics {}

/// Measured execution or cache reuse of one declared deterministic stage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageTiming {
    /// Stable instance identity, or `world` for shared work.
    pub region_id: String,
    /// Boundary, geometry, features, or package stage.
    pub stage: String,
    /// Measured wall time; explicitly excluded from content fingerprints.
    pub elapsed_micros: u64,
    /// True when a matching immutable artifact was reused.
    pub reused: bool,
    /// Complete declared input fingerprint for this stage.
    pub input_fingerprint: u64,
}

/// Structural counts and timings, separate from authoritative package content.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompileReport {
    /// Per-stage timing and dependency evidence.
    pub stages: Vec<StageTiming>,
    /// Number of complete region outputs reused.
    pub regions_reused: usize,
    /// Number of region outputs actually rebuilt.
    pub regions_compiled: usize,
    /// Exact occupied or explicitly empty horizontal columns.
    pub columns: usize,
    /// Canonical interval count, retaining caves, decks and liquid.
    pub runs: usize,
    /// Exact liquid surface count.
    pub liquid_columns: usize,
    /// Exact roofed interior memberships.
    pub interior_columns: usize,
    /// Stable authored object instances.
    pub objects: usize,
}

#[derive(Clone, Debug)]
struct CachedRegion {
    geometry_key: u64,
    output_key: u64,
    geometry: Arc<RegionBuild>,
    output: Arc<RegionBuild>,
}

/// A complete immutable compile result and reusable in-memory stage artifacts.
///
/// Cache entries never represent runtime edits. Reuse compares all declared
/// dependencies and compiler version; there is no implicit global mutable cache.
#[derive(Clone, Debug)]
pub struct CompileArtifacts {
    /// Validated, sealed in-memory package ready for a caller-owned filesystem adapter.
    pub package: WorldPackage,
    /// Non-authoritative structural counts and timing report.
    pub report: CompileReport,
    regions: BTreeMap<String, CachedRegion>,
}

fn contextual<T>(
    context: &str,
    stage: &str,
    value: Result<T, String>,
) -> Result<T, CompileDiagnostics> {
    value.map_err(|message| CompileDiagnostics::one(context, stage, message))
}
fn hashed<T: Serialize>(value: &T) -> Result<u64, CompileDiagnostics> {
    hash_serializable(value)
        .map_err(|error| CompileDiagnostics::one("world", "fingerprint", error.to_string()))
}
fn elapsed(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX)
}
fn stage(
    report: &mut CompileReport,
    region: &str,
    name: &str,
    start: Instant,
    reused: bool,
    key: u64,
) {
    report.stages.push(StageTiming {
        region_id: region.into(),
        stage: name.into(),
        elapsed_micros: elapsed(start),
        reused,
        input_fingerprint: key,
    });
}
fn canonical(source: &WorldSpec) -> WorldSpec {
    let mut result = source.clone();
    result.materials.sort_by(|a, b| a.id.cmp(&b.id));
    result.regions.sort_by(|a, b| a.id.cmp(&b.id));
    result.connections.sort_by(|a, b| a.id.cmp(&b.id));
    for recipe in result.recipes.values_mut() {
        recipe.landforms.sort_by(|a, b| a.id.cmp(&b.id));
        recipe.biomes.sort_by(|a, b| a.id.cmp(&b.id));
        recipe.basins.sort_by(|a, b| a.id.cmp(&b.id));
        recipe.channels.sort_by(|a, b| a.id.cmp(&b.id));
        recipe.routes.sort_by(|a, b| a.id.cmp(&b.id));
        recipe.bridges.sort_by(|a, b| a.id.cmp(&b.id));
        recipe.caves.sort_by(|a, b| a.id.cmp(&b.id));
        recipe.features.sort_by(|a, b| a.id.cmp(&b.id));
        recipe.overrides.sort_by(|a, b| a.id.cmp(&b.id));
        recipe.anchors.sort_by(|a, b| a.id.cmp(&b.id));
        for field in &mut recipe.landforms {
            field.centers.sort();
        }
        for feature in &mut recipe.features {
            feature.roots.sort();
            feature.voxels.sort_by_key(|voxel| {
                (
                    voxel.offset,
                    voxel.bottom,
                    voxel.top,
                    voxel.material.clone(),
                )
            });
        }
        for cave in &mut recipe.caves {
            cave.entrances.sort();
            cave.rooms.sort_by_key(|room| (room.center, room.radius));
        }
        for channel in &mut recipe.channels {
            channel.falls_after.sort();
        }
    }
    result
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

/// Validate runtime-loaded source before executing generation or allocating region columns.
///
/// Bounds are per operator/region, not a world-size or total-region-count limit.
/// Final geometric consequences are additionally validated on the compiled package.
pub fn validate_source(source: &WorldSpec) -> Result<(), CompileDiagnostics> {
    let mut errors = Vec::new();
    let mut issue = |context: &str, message: String| {
        errors.push(CompileDiagnostic {
            context: context.into(),
            stage: "source".into(),
            message,
        })
    };
    if source.version != SOURCE_VERSION {
        issue(
            &source.id,
            format!("unsupported source version {}", source.version),
        );
    }
    if !valid_name(&source.id) || source.regions.is_empty() {
        issue("world", "world ID and regions must be nonempty".into());
    }
    let mut materials = BTreeMap::new();
    for material in &source.materials {
        if !valid_name(&material.id) || material.id.eq_ignore_ascii_case("air") {
            issue(&material.id, "material needs non-air identity".into());
        }
        if materials.insert(material.id.as_str(), material).is_some() {
            issue(&material.id, "duplicate material".into());
        }
    }
    let require_material = |name: &str, solid: bool| -> Option<String> {
        match materials.get(name) {
            None => Some(format!("unknown material {name}")),
            Some(material) if material.solid != solid => {
                Some(format!("material {name} must have solid={solid}"))
            }
            _ => None,
        }
    };
    for (key, recipe) in &source.recipes {
        if !valid_name(key) {
            issue(key, "invalid recipe identity".into());
        }
        let mut ids = BTreeSet::new();
        let mut register = |id: &str| {
            if !valid_name(id) || !ids.insert(id.to_string()) {
                issue(key, format!("empty/duplicate operator ID {id}"));
            }
        };
        for id in recipe
            .landforms
            .iter()
            .map(|v| &v.id)
            .chain(recipe.biomes.iter().map(|v| &v.id))
            .chain(recipe.basins.iter().map(|v| &v.id))
            .chain(recipe.channels.iter().map(|v| &v.id))
            .chain(recipe.routes.iter().map(|v| &v.id))
            .chain(recipe.bridges.iter().map(|v| &v.id))
            .chain(recipe.caves.iter().map(|v| &v.id))
            .chain(recipe.features.iter().map(|v| &v.id))
            .chain(recipe.overrides.iter().map(|v| &v.id))
            .chain(recipe.anchors.iter().map(|v| &v.id))
        {
            register(id);
        }
        if !(1..=60_000).contains(&recipe.base_level) || !(1..=60_000).contains(&recipe.hub.level) {
            issue(key, "base/hub level outside 1..=60000".into());
        }
        if recipe.strata.soil_depth > 60_000 {
            issue(key, "soil depth exceeds vertical operator bound".into());
        }
        for name in [
            &recipe.strata.bedrock,
            &recipe.strata.rock,
            &recipe.strata.soil,
            &recipe.strata.surface,
        ] {
            if let Some(error) = require_material(name, true) {
                issue(key, error);
            }
        }
        if materials
            .get(recipe.strata.bedrock.as_str())
            .is_some_and(|material| material.diggable)
        {
            issue(key, "bedrock material must be non-diggable".into());
        }
        for field in &recipe.landforms {
            if field.centers.is_empty()
                || field.radius == 0
                || field.radius > 2048
                || field.plateau_radius >= field.radius
                || field.relief > 10_000
                || field.rise.unsigned_abs() > 60_000
            {
                issue(&field.id,"landform requires centers, 0 <= plateau < radius <= 2048, and bounded relief/rise".into());
            }
        }
        for biome in &recipe.biomes {
            if let Some(error) = require_material(&biome.material, true) {
                issue(&biome.id, error);
            }
        }
        for basin in &recipe.basins {
            if basin.depth == 0
                || basin.depth > 60_000
                || basin.water_level <= basin.depth as i32
                || basin.water_level > 60_000
                || basin.bank_width > 128
            {
                issue(&basin.id, "invalid liquid depth/level or bank width".into());
            }
            for (name, solid) in [(&basin.material, false), (&basin.bed_material, true)] {
                if let Some(error) = require_material(name, solid) {
                    issue(&basin.id, error);
                }
            }
        }
        for channel in &recipe.channels {
            if channel.points.len() < 2
                || channel.half_width > 32
                || channel.bank_width > 128
                || channel.depth == 0
                || channel.depth > 60_000
            {
                issue(
                    &channel.id,
                    "channel requires >=2 controls and bounded positive depth/width".into(),
                );
            }
            if channel.points.windows(2).any(|pair| match pair {
                [a, b] => a.level < b.level || a.column == b.column,
                _ => false,
            }) {
                issue(&channel.id,"channel controls must descend or remain level, with distinct consecutive coordinates".into());
            }
            if channel
                .points
                .iter()
                .any(|point| point.level <= channel.depth as i32 || point.level > 60_000)
            {
                issue(
                    &channel.id,
                    "channel level must clear bedrock and respect vertical bound".into(),
                );
            }
            if channel
                .falls_after
                .iter()
                .any(|index| *index >= channel.points.len().saturating_sub(1))
            {
                issue(
                    &channel.id,
                    "waterfall references absent control segment".into(),
                );
            }
            for (name, solid) in [(&channel.material, false), (&channel.bed_material, true)] {
                if let Some(error) = require_material(name, solid) {
                    issue(&channel.id, error);
                }
            }
        }
        for route in &recipe.routes {
            if route.points.len() < 2 || route.half_width > 32 || route.shoulder_width > 128 {
                issue(
                    &route.id,
                    "route requires >=2 controls and bounded ribbon/shoulder width".into(),
                );
            }
            if route
                .points
                .iter()
                .any(|p| !(1..=60_000).contains(&p.level))
            {
                issue(&route.id, "route pin level outside vertical bound".into());
            }
            if let Some(error) = require_material(&route.material, true) {
                issue(&route.id, error);
            }
        }
        for bridge in &recipe.bridges {
            if bridge.points.len() < 2
                || bridge.half_width > 32
                || bridge.thickness == 0
                || bridge.thickness > 1024
                || bridge
                    .points
                    .iter()
                    .any(|point| point.level <= bridge.thickness as i32 || point.level > 60_000)
            {
                issue(&bridge.id, "invalid bridge controls/width/thickness".into());
            }
            if let Some(error) = require_material(&bridge.material, true) {
                issue(&bridge.id, error);
            }
            if bridge.points.windows(2).any(|pair| match pair {
                [a, b] => !geometry::distance(a.column, b.column)
                    .is_ok_and(|distance| distance >= u64::from(a.level.abs_diff(b.level))),
                _ => false,
            }) {
                issue(
                    &bridge.id,
                    "bridge controls exceed ordinary one-level walking grade".into(),
                );
            }
        }
        for cave in &recipe.caves {
            if cave.path.len() < 2
                || cave.half_width > 32
                || cave.clearance < 2
                || cave.clearance > 1024
                || cave.roof_thickness == 0
                || cave.roof_thickness > 1024
                || !(1..=60_000).contains(&cave.floor_level)
            {
                issue(
                    &cave.id,
                    "invalid cave path, floor, headroom or roof".into(),
                );
            }
            if let Some(error) = require_material(&cave.material, true) {
                issue(&cave.id, error);
            }
        }
        for feature in &recipe.features {
            if feature.voxels.is_empty()
                || feature.density > 10_000
                || feature.kind.is_empty()
                || feature.asset.is_empty()
            {
                issue(
                    &feature.id,
                    "feature needs kind, asset, occupancy, density <=10000".into(),
                );
            }
            for voxel in &feature.voxels {
                if voxel.bottom < 0
                    || voxel.top <= voxel.bottom
                    || voxel.top > 1024
                    || !geometry::distance(voxel.offset, WorldHex::new(0, 0)).is_ok_and(|d| d <= 64)
                {
                    issue(&feature.id, "invalid feature offset/interval".into());
                }
                if let Some(error) = require_material(&voxel.material, true) {
                    issue(&feature.id, error);
                }
            }
            let mut prototype = BTreeMap::<WorldHex, Vec<VoxelRun>>::new();
            for voxel in &feature.voxels {
                prototype.entry(voxel.offset).or_default().push(VoxelRun {
                    bottom: voxel.bottom,
                    top: voxel.top,
                    material: voxel.material.clone(),
                });
            }
            for runs in prototype.into_values() {
                if let Err(error) = super::volume::canonicalize(runs) {
                    issue(&feature.id, format!("invalid prefab prototype: {error}"));
                }
            }
            if let Some(provenance) = &feature.provenance {
                if provenance.source_path.is_empty()
                    || provenance.source_revision.len() != 40
                    || provenance.style_materials.is_empty()
                {
                    issue(&feature.id,"stock export needs source path, exact 40-character git revision and explicit style policy".into());
                }
                for material in provenance.style_materials.values() {
                    if let Some(error) = require_material(material, true) {
                        issue(&feature.id, error);
                    }
                }
            }
        }
        for patch in &recipe.overrides {
            if patch
                .surface_level
                .is_some_and(|level| !(1..=60_000).contains(&level))
            {
                issue(&patch.id, "override level outside vertical bound".into());
            }
            if let Some(material) = &patch.material {
                if let Some(error) = require_material(material, true) {
                    issue(&patch.id, error);
                }
            }
        }
    }
    let mut regions = BTreeMap::new();
    for region in &source.regions {
        if !valid_name(&region.id) || regions.insert(region.id.as_str(), region).is_some() {
            issue(&region.id, "empty/duplicate region ID".into());
        }
        if region.rotation >= 6 || region.radius == 0 || region.radius > 1024 {
            issue(
                &region.id,
                "rotation must be 0..6; radius must be 1..=1024 per compile unit".into(),
            );
        }
        if let Err(error) = (RegionDescriptor {
            id: region.id.clone(),
            origin: region.origin,
            radius: region.radius,
            source_fingerprint: 0,
        })
        .validate()
        {
            issue(&region.id, error.to_string());
        }
        let Some(recipe) = source.recipes.get(&region.recipe) else {
            issue(&region.id, format!("unknown recipe {}", region.recipe));
            continue;
        };
        let fits = |p: WorldHex, radius: u32| {
            geometry::distance(p, WorldHex::new(0, 0)).is_ok_and(|distance| {
                distance
                    .checked_add(u64::from(radius))
                    .is_some_and(|extent| extent <= u64::from(region.radius))
            })
        };
        if !fits(recipe.hub.column, 0) {
            issue(&region.id, "hub is outside footprint".into());
        }
        for anchor in &recipe.anchors {
            if !fits(anchor.column, 0)
                || anchor
                    .level
                    .is_some_and(|level| !(0..=65_535).contains(&level))
            {
                issue(
                    &anchor.id,
                    "anchor position leaves the source geometry bounds".into(),
                );
            }
        }
        for feature in &recipe.features {
            if feature.roots.iter().any(|root| !fits(*root, 0)) {
                issue(
                    &feature.id,
                    "explicit feature root leaves region footprint".into(),
                );
            }
        }
        for mask in recipe
            .biomes
            .iter()
            .map(|v| &v.mask)
            .chain(recipe.basins.iter().map(|v| &v.mask))
            .chain(recipe.features.iter().map(|v| &v.mask))
            .chain(recipe.overrides.iter().map(|v| &v.mask))
            .chain(recipe.caves.iter().flat_map(|v| v.rooms.iter()))
        {
            if !fits(mask.center, mask.radius) {
                issue(
                    &region.id,
                    format!("operator disk {mask:?} leaves footprint"),
                );
            }
        }
        for (id, points, width) in recipe
            .routes
            .iter()
            .map(|v| (&v.id, &v.points, v.half_width))
            .chain(
                recipe
                    .channels
                    .iter()
                    .map(|v| (&v.id, &v.points, v.half_width)),
            )
            .chain(
                recipe
                    .bridges
                    .iter()
                    .map(|v| (&v.id, &v.points, v.half_width)),
            )
        {
            if points.iter().any(|p| !fits(p.column, width)) {
                issue(id, "control ribbon leaves region footprint".into());
            }
        }
        for cave in &recipe.caves {
            if cave.path.iter().any(|p| !fits(*p, cave.half_width))
                || cave.entrances.iter().any(|p| !fits(*p, 0))
            {
                issue(&cave.id, "cave path/entrance leaves footprint".into());
            }
        }
    }
    let mut connections = BTreeSet::new();
    let mut connection_ids = BTreeSet::new();
    for connection in &source.connections {
        if !connection_ids.insert(&connection.id) || !valid_name(&connection.id) {
            issue(&connection.id, "empty/duplicate connection ID".into());
        }
        let pair = if connection.region_a < connection.region_b {
            (&connection.region_a, &connection.region_b)
        } else {
            (&connection.region_b, &connection.region_a)
        };
        if !connections.insert(pair) || pair.0 == pair.1 {
            issue(
                &connection.id,
                "duplicate/reflexive region connection".into(),
            );
        }
        if !regions.contains_key(connection.region_a.as_str())
            || !regions.contains_key(connection.region_b.as_str())
        {
            issue(&connection.id, "connection references absent region".into());
        }
        if !(1..=60_000).contains(&connection.ground_level)
            || !(1..=128).contains(&connection.transition_width)
        {
            issue(
                &connection.id,
                "invalid boundary datum/transition width".into(),
            );
        }
        if let Some(water) = &connection.water {
            if water.depth == 0
                || water.depth > 60_000
                || water.level > 60_000
                || i64::from(water.level) - i64::from(water.depth)
                    != i64::from(connection.ground_level)
                || water.half_width > 32
            {
                issue(&connection.id,"shared water bed must equal ground datum; bounded positive depth/width required".into());
            }
            for (name, solid) in [(&water.material, false), (&water.bed_material, true)] {
                if let Some(error) = require_material(name, solid) {
                    issue(&connection.id, error);
                }
            }
            if let Some(flow) = &water.flow {
                if let (Some(a), Some(b)) = (
                    regions.get(connection.region_a.as_str()),
                    regions.get(connection.region_b.as_str()),
                ) {
                    let inside = |region: &&RegionSpec, p| {
                        geometry::distance(region.origin, p)
                            .is_ok_and(|distance| distance <= u64::from(region.radius))
                    };
                    if !((inside(a, flow.upstream) && inside(b, flow.downstream))
                        || (inside(b, flow.upstream) && inside(a, flow.downstream)))
                    {
                        issue(
                            &connection.id,
                            "directed water endpoints must lie in opposite participating regions"
                                .into(),
                        );
                    }
                }
            }
        }
    }
    for (index, a) in source.regions.iter().enumerate() {
        for b in source.regions.iter().skip(index + 1) {
            match geometry::distance(a.origin, b.origin) {
                Ok(distance) if distance <= u64::from(a.radius) + u64::from(b.radius) => {
                    issue(&a.id, format!("region overlaps {}", b.id))
                }
                Ok(distance) if distance == u64::from(a.radius) + u64::from(b.radius) + 1 => {
                    let pair = if a.id < b.id {
                        (&a.id, &b.id)
                    } else {
                        (&b.id, &a.id)
                    };
                    if !connections.contains(&pair) {
                        issue(
                            &a.id,
                            format!("touching region {} needs one shared connection", b.id),
                        );
                    }
                }
                Err(error) => issue(&a.id, error),
                _ => {}
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(CompileDiagnostics {
            diagnostics: errors,
        })
    }
}

fn world(region: &RegionSpec, p: WorldHex) -> Result<WorldHex, String> {
    p.rotate_60(region.rotation)
        .and_then(|p| region.origin.checked_add(p))
        .map_err(|error| error.to_string())
}
fn local(region: &RegionSpec, p: WorldHex) -> Result<WorldHex, String> {
    let q =
        p.q.checked_sub(region.origin.q)
            .ok_or_else(|| "local q overflow".to_string())?;
    let r =
        p.r.checked_sub(region.origin.r)
            .ok_or_else(|| "local r overflow".to_string())?;
    WorldHex::new(q, r)
        .rotate_60((6 - region.rotation) % 6)
        .map_err(|error| error.to_string())
}

#[derive(Clone, Debug, Serialize)]
struct ResolvedBoundary {
    source: ConnectionSpec,
    contract: BoundaryContract,
    wet: BTreeSet<WorldHex>,
    downstream: BTreeMap<WorldHex, WorldHex>,
}
fn boundaries(source: &WorldSpec) -> Result<Vec<ResolvedBoundary>, CompileDiagnostics> {
    let regions: BTreeMap<_, _> = source
        .regions
        .iter()
        .map(|region| (&region.id, region))
        .collect();
    let mut output = Vec::new();
    for connection in &source.connections {
        let a = regions.get(&connection.region_a).ok_or_else(|| {
            CompileDiagnostics::one(&connection.id, "boundary", "missing first region")
        })?;
        let b = regions.get(&connection.region_b).ok_or_else(|| {
            CompileDiagnostics::one(&connection.id, "boundary", "missing second region")
        })?;
        let mut pairs = BTreeSet::new();
        for p in contextual(
            &connection.id,
            "boundary",
            geometry::ring(a.origin, a.radius),
        )? {
            for n in geometry::neighbors(p) {
                if contextual(&connection.id, "boundary", geometry::distance(n, b.origin))?
                    <= u64::from(b.radius)
                {
                    pairs.insert((p, n));
                }
            }
        }
        if pairs.is_empty() {
            return Err(CompileDiagnostics::one(
                &connection.id,
                "boundary",
                "declared regions are not adjacent",
            ));
        }
        let midpoint = pairs.iter().nth(pairs.len() / 2).copied().ok_or_else(|| {
            CompileDiagnostics::one(&connection.id, "boundary", "missing midpoint")
        })?;
        let mut wet = BTreeSet::new();
        let mut downstream = BTreeMap::new();
        if let Some(water) = &connection.water {
            // One global mask creates the same shoreline on both sides. Fringe
            // edges claim no paired water datum; their solid ground remains shared.
            if let Some(flow) = &water.flow {
                let path = contextual(
                    &connection.id,
                    "boundary flow",
                    geometry::line(flow.upstream, flow.downstream),
                )?;
                wet = contextual(
                    &connection.id,
                    "boundary flow",
                    geometry::ribbon(&path, water.half_width),
                )?;
                for p in &wet {
                    if contextual(
                        &connection.id,
                        "boundary flow",
                        geometry::distance(*p, a.origin),
                    )? > u64::from(a.radius)
                        && contextual(
                            &connection.id,
                            "boundary flow",
                            geometry::distance(*p, b.origin),
                        )? > u64::from(b.radius)
                    {
                        return Err(CompileDiagnostics::one(
                            &connection.id,
                            "boundary flow",
                            "flow ribbon leaves the two declared region footprints",
                        ));
                    }
                }
                let distances = geometry::distances(&wet, flow.downstream);
                if distances.len() != wet.len() {
                    return Err(CompileDiagnostics::one(
                        &connection.id,
                        "boundary flow",
                        "disconnected flow ribbon",
                    ));
                }
                for (p, rank) in &distances {
                    if let Some((_, next)) = geometry::neighbors(*p)
                        .filter_map(|next| {
                            distances
                                .get(&next)
                                .filter(|distance| *distance < rank)
                                .map(|distance| (*distance, next))
                        })
                        .min()
                    {
                        downstream.insert(*p, next);
                    }
                }
            } else {
                for center in [midpoint.0, midpoint.1] {
                    wet.extend(contextual(
                        &connection.id,
                        "boundary",
                        geometry::disk(center, water.half_width.max(1)),
                    )?);
                }
            }
        }
        let samples = pairs
            .into_iter()
            .map(|(p, n)| BoundarySample {
                a: p,
                b: n,
                ground_level: connection.ground_level,
                water_level: connection
                    .water
                    .as_ref()
                    .filter(|_| wet.contains(&p) && wet.contains(&n))
                    .map(|water| water.level),
                required_access: connection.required_access
                    && !wet.contains(&p)
                    && !wet.contains(&n),
            })
            .collect();
        output.push(ResolvedBoundary {
            source: connection.clone(),
            contract: BoundaryContract {
                id: connection.id.clone(),
                region_a: connection.region_a.clone(),
                region_b: connection.region_b.clone(),
                samples,
            },
            wet,
            downstream,
        });
    }
    Ok(output)
}

fn compile_geometry(
    source: &WorldSpec,
    region: &RegionSpec,
    recipe: &RegionRecipe,
    seams: &[&ResolvedBoundary],
) -> Result<RegionBuild, CompileDiagnostics> {
    let mut build = contextual(
        &region.id,
        "landforms",
        operators::base(region, recipe, source.seed),
    )?;
    let mut all_levels = BTreeMap::new();
    for seam in seams {
        let mut levels = BTreeMap::new();
        for sample in &seam.contract.samples {
            let p = if seam.contract.region_a == region.id {
                sample.a
            } else {
                sample.b
            };
            let p = contextual(&region.id, "boundary", local(region, p))?;
            if all_levels
                .insert(p, sample.ground_level)
                .is_some_and(|old| old != sample.ground_level)
            {
                return Err(CompileDiagnostics::one(
                    &region.id,
                    "boundary",
                    "conflicting corner datums",
                ));
            }
            levels.insert(p, sample.ground_level);
        }
        contextual(
            &region.id,
            "boundary terrain",
            operators::seam_terrain(&mut build, recipe, &levels, seam.source.transition_width),
        )?;
        contextual(
            &region.id,
            "boundary terrain",
            operators::check_overrides(&build, recipe, &seam.source.id),
        )?;
    }
    for basin in &recipe.basins {
        contextual(
            &region.id,
            &format!("basin/{}", basin.id),
            operators::basin(
                &mut build,
                recipe,
                basin,
                &format!("{}/{}", region.id, basin.id),
            ),
        )?;
        contextual(
            &region.id,
            "basin",
            operators::check_overrides(&build, recipe, &basin.id),
        )?;
    }
    for channel in &recipe.channels {
        contextual(
            &region.id,
            &format!("channel/{}", channel.id),
            operators::channel(
                &mut build,
                recipe,
                channel,
                &format!("{}/{}", region.id, channel.id),
            ),
        )?;
        contextual(
            &region.id,
            "channel",
            operators::check_overrides(&build, recipe, &channel.id),
        )?;
    }
    for seam in seams {
        if let Some(water) = &seam.source.water {
            let mut wet = BTreeSet::new();
            for p in &seam.wet {
                let p = contextual(&region.id, "boundary water", local(region, *p))?;
                if build.columns.contains_key(&p) {
                    wet.insert(p);
                }
            }
            contextual(
                &region.id,
                "boundary water",
                operators::seam_water(
                    &mut build,
                    recipe,
                    &wet,
                    water,
                    &format!("boundary/{}", seam.source.id),
                ),
            )?;
            for (from, to) in &seam.downstream {
                let from = contextual(&region.id, "boundary flow", local(region, *from))?;
                if let Some(liquid) = build.liquids.get_mut(&from) {
                    liquid.kind = LiquidKind::Directed;
                    liquid.downstream = vec![VoxelPosition {
                        column: contextual(&region.id, "boundary flow", local(region, *to))?,
                        level: water.level,
                    }];
                }
            }
            contextual(
                &region.id,
                "boundary water",
                operators::check_overrides(&build, recipe, &seam.source.id),
            )?;
        }
    }
    // Boundary approaches are generated before authored routes/structures, which
    // can refine them. Final route/seam/semantic validators reject interference.
    for seam in seams {
        if seam.source.required_access {
            let candidates: Vec<_> = seam
                .contract
                .samples
                .iter()
                .filter(|sample| sample.required_access)
                .collect();
            let sample = candidates.get(candidates.len() / 2).ok_or_else(|| {
                CompileDiagnostics::one(&region.id, "boundary route", "no dry walking port")
            })?;
            let p = if seam.contract.region_a == region.id {
                sample.a
            } else {
                sample.b
            };
            let p = contextual(&region.id, "boundary route", local(region, p))?;
            contextual(
                &region.id,
                "boundary route",
                operators::auto_route(
                    &mut build,
                    recipe,
                    GradePoint {
                        column: p,
                        level: sample.ground_level,
                    },
                    &seam.source.id,
                ),
            )?;
            contextual(
                &region.id,
                "boundary route",
                operators::check_overrides(&build, recipe, &seam.source.id),
            )?;
            build.semantics.anchors.push(WorldAnchor {
                id: format!("{}/boundary/{}", region.id, seam.source.id),
                region_id: region.id.clone(),
                position: VoxelPosition {
                    column: p,
                    level: sample.ground_level,
                },
                role: AnchorRole::Transit,
            });
        }
    }
    for route in &recipe.routes {
        contextual(
            &region.id,
            &format!("route/{}", route.id),
            operators::route(&mut build, recipe, route),
        )?;
        contextual(
            &region.id,
            "route",
            operators::check_overrides(&build, recipe, &route.id),
        )?;
    }
    for bridge in &recipe.bridges {
        contextual(
            &region.id,
            &format!("bridge/{}", bridge.id),
            operators::bridge(&mut build, bridge),
        )?;
        contextual(
            &region.id,
            "bridge",
            operators::check_overrides(&build, recipe, &bridge.id),
        )?;
    }
    for cave in &recipe.caves {
        contextual(
            &region.id,
            &format!("cave/{}", cave.id),
            operators::cave(&mut build, recipe, &region.id, cave),
        )?;
        contextual(
            &region.id,
            "cave",
            operators::check_overrides(&build, recipe, &cave.id),
        )?;
    }
    build.reserved.insert(recipe.hub.column);
    for anchor in &recipe.anchors {
        let level = match anchor.level {
            Some(level) => level,
            None => {
                contextual(
                    &region.id,
                    "anchors",
                    operators::terrain(&build, anchor.column),
                )?
                .0
            }
        };
        if anchor.role != AnchorRole::Observation {
            build.reserved.insert(anchor.column);
        }
        build.semantics.anchors.push(WorldAnchor {
            id: format!("{}/anchor/{}", region.id, anchor.id),
            region_id: region.id.clone(),
            position: VoxelPosition {
                column: anchor.column,
                level,
            },
            role: anchor.role,
        });
    }
    for bridge in &recipe.bridges {
        for (index, pin) in bridge.points.iter().enumerate() {
            build.semantics.anchors.push(WorldAnchor {
                id: format!("{}/bridge/{}/pin-{index}", region.id, bridge.id),
                region_id: region.id.clone(),
                position: VoxelPosition {
                    column: pin.column,
                    level: pin.level,
                },
                role: AnchorRole::Transit,
            });
        }
    }
    build.semantics.anchors.push(WorldAnchor {
        id: format!("{}/hub", region.id),
        region_id: region.id.clone(),
        position: VoxelPosition {
            column: recipe.hub.column,
            level: recipe.hub.level,
        },
        role: AnchorRole::Gameplay,
    });
    for anchor in &build.semantics.anchors {
        let kind = if anchor.id == format!("{}/hub", region.id) {
            "entry"
        } else {
            match anchor.role {
                AnchorRole::Gameplay => "gameplay-anchor",
                AnchorRole::Transit => "transit",
                AnchorRole::Observation => "observation",
            }
        };
        build.features.push(FeatureSummary {
            id: anchor.id.clone(),
            region_id: region.id.clone(),
            kind: kind.into(),
            anchor: anchor.position,
            asset: None,
        });
    }
    Ok(build)
}

/// Compile a complete deterministic world without any ambient filesystem or cache state.
pub fn compile_world(source: &WorldSpec) -> Result<WorldPackage, CompileDiagnostics> {
    Ok(compile_world_cached(source, None)?.package)
}

/// Compile with optional immutable stage artifacts from an earlier successful compile.
///
/// Geometry dependencies include compiler/schema version, world seed, materials,
/// placement, recipe excluding features, and every resolved touching seam. Feature
/// edits reuse geometry; geometry/seam edits invalidate the affected region only.
/// Package assembly and global consistency validation always execute.
pub fn compile_world_cached(
    source: &WorldSpec,
    previous: Option<&CompileArtifacts>,
) -> Result<CompileArtifacts, CompileDiagnostics> {
    validate_source(source)?;
    let source = canonical(source);
    let source_key = hashed(&(COMPILER_VERSION, &source))?;
    let mut report = CompileReport::default();
    let start = Instant::now();
    let seams = boundaries(&source)?;
    stage(
        &mut report,
        "world",
        "boundaries",
        start,
        false,
        hashed(&source.connections)?,
    );
    let mut cache = BTreeMap::new();
    for region in &source.regions {
        let recipe = source
            .recipes
            .get(&region.recipe)
            .ok_or_else(|| CompileDiagnostics::one(&region.id, "source", "unknown recipe"))?;
        let local_seams: Vec<_> = seams
            .iter()
            .filter(|seam| {
                seam.contract.region_a == region.id || seam.contract.region_b == region.id
            })
            .collect();
        let output_key = hashed(&(
            COMPILER_VERSION,
            &source.id,
            source.seed,
            &source.materials,
            region,
            recipe,
            &local_seams,
        ))?;
        let mut geometry_recipe = recipe.clone();
        geometry_recipe.features.clear();
        let geometry_key = hashed(&(
            COMPILER_VERSION,
            &source.id,
            source.seed,
            &source.materials,
            region,
            &geometry_recipe,
            &local_seams,
        ))?;
        let old = previous.and_then(|old| old.regions.get(&region.id));
        let start = Instant::now();
        if let Some(old) = old.filter(|old| old.output_key == output_key) {
            cache.insert(region.id.clone(), old.clone());
            report.regions_reused += 1;
            stage(
                &mut report,
                &region.id,
                "geometry",
                start,
                true,
                geometry_key,
            );
            stage(
                &mut report,
                &region.id,
                "features",
                Instant::now(),
                true,
                output_key,
            );
            continue;
        }
        let reused = old.is_some_and(|old| old.geometry_key == geometry_key);
        let geometry = if let Some(old) = old.filter(|old| old.geometry_key == geometry_key) {
            Arc::clone(&old.geometry)
        } else {
            Arc::new(compile_geometry(
                &source,
                region,
                &geometry_recipe,
                &local_seams,
            )?)
        };
        stage(
            &mut report,
            &region.id,
            "geometry",
            start,
            reused,
            geometry_key,
        );
        let start = Instant::now();
        let mut output = (*geometry).clone();
        contextual(
            &region.id,
            "features",
            operators::decorate(&mut output, recipe, &region.id, source.seed),
        )?;
        contextual(
            &region.id,
            "walking validation",
            operators::validate_access(&output, recipe, &source.materials),
        )?;
        stage(
            &mut report,
            &region.id,
            "features",
            start,
            false,
            output_key,
        );
        report.regions_compiled += 1;
        cache.insert(
            region.id.clone(),
            CachedRegion {
                geometry_key,
                output_key,
                geometry,
                output: Arc::new(output),
            },
        );
    }
    let start = Instant::now();
    let package = assemble(&source, source_key, &seams, &cache)?;
    stage(&mut report, "world", "package", start, false, source_key);
    for chunk in package.chunks.values() {
        report.columns += chunk.columns.len();
        report.runs += chunk
            .columns
            .iter()
            .map(|column| column.runs.len())
            .sum::<usize>();
        report.liquid_columns += chunk.semantics.liquids.len();
        report.interior_columns += chunk.semantics.interiors.len();
        report.objects += chunk.semantics.objects.len();
    }
    Ok(CompileArtifacts {
        package,
        report,
        regions: cache,
    })
}

fn assemble(
    source: &WorldSpec,
    source_key: u64,
    seams: &[ResolvedBoundary],
    cache: &BTreeMap<String, CachedRegion>,
) -> Result<WorldPackage, CompileDiagnostics> {
    let mut chunks: BTreeMap<ChunkId, ChunkPackage> = BTreeMap::new();
    let mut contributors: BTreeMap<ChunkId, BTreeMap<String, u64>> = BTreeMap::new();
    let mut summary = Vec::new();
    let mut features = Vec::new();
    let mut regions = Vec::new();
    for region in &source.regions {
        let cached = cache.get(&region.id).ok_or_else(|| {
            CompileDiagnostics::one(&region.id, "package", "missing compiled region")
        })?;
        let build = &cached.output;
        regions.push(RegionDescriptor {
            id: region.id.clone(),
            origin: region.origin,
            radius: region.radius,
            source_fingerprint: cached.output_key,
        });
        for (p, runs) in &build.columns {
            let position = contextual(&region.id, "transform", world(region, *p))?;
            let coordinate = position.chunk();
            let chunk = chunks.entry(coordinate).or_insert_with(|| ChunkPackage {
                schema_version: SCHEMA_VERSION,
                world_id: source.id.clone(),
                coordinate,
                source_fingerprint: 0,
                columns: vec![],
                features: vec![],
                semantics: ChunkSemantics::default(),
                fingerprint: 0,
            });
            chunk.columns.push(ColumnData {
                position,
                runs: runs.clone(),
            });
            contributors
                .entry(coordinate)
                .or_default()
                .insert(region.id.clone(), cached.output_key);
            if p.q.rem_euclid(12) == 0 && p.r.rem_euclid(12) == 0 {
                if let Some(top) = runs.last() {
                    summary.push(MapSummaryCell {
                        position,
                        level: top.top - 1,
                        material: top.material.clone(),
                        region_id: region.id.clone(),
                    });
                }
            }
        }
        let transform = |p: VoxelPosition| -> Result<VoxelPosition, CompileDiagnostics> {
            Ok(VoxelPosition {
                column: contextual(&region.id, "transform", world(region, p.column))?,
                level: p.level,
            })
        };
        for feature in &build.features {
            let mut feature = feature.clone();
            feature.anchor = transform(feature.anchor)?;
            chunks
                .get_mut(&feature.anchor.column.chunk())
                .ok_or_else(|| {
                    CompileDiagnostics::one(&region.id, "package", "missing feature chunk")
                })?
                .features
                .push(feature.clone());
            features.push(feature);
        }
        for liquid in build.liquids.values() {
            let mut liquid = liquid.clone();
            liquid.column = contextual(&region.id, "transform", world(region, liquid.column))?;
            liquid.downstream = liquid
                .downstream
                .into_iter()
                .map(transform)
                .collect::<Result<_, _>>()?;
            chunks
                .get_mut(&liquid.column.chunk())
                .ok_or_else(|| {
                    CompileDiagnostics::one(&region.id, "package", "missing liquid chunk")
                })?
                .semantics
                .liquids
                .push(liquid);
        }
        for anchor in &build.semantics.anchors {
            let mut anchor = anchor.clone();
            anchor.position = transform(anchor.position)?;
            chunks
                .get_mut(&anchor.position.column.chunk())
                .ok_or_else(|| {
                    CompileDiagnostics::one(&region.id, "package", "missing anchor chunk")
                })?
                .semantics
                .anchors
                .push(anchor);
        }
        for interior in &build.semantics.interiors {
            let mut interior = interior.clone();
            interior.column = contextual(&region.id, "transform", world(region, interior.column))?;
            chunks
                .get_mut(&interior.column.chunk())
                .ok_or_else(|| {
                    CompileDiagnostics::one(&region.id, "package", "missing interior chunk")
                })?
                .semantics
                .interiors
                .push(interior);
        }
        for light in &build.semantics.lights {
            let mut light = light.clone();
            light.position = transform(light.position)?;
            chunks
                .get_mut(&light.position.column.chunk())
                .ok_or_else(|| {
                    CompileDiagnostics::one(&region.id, "package", "missing light chunk")
                })?
                .semantics
                .lights
                .push(light);
        }
        for object in &build.semantics.objects {
            let mut object = object.clone();
            object.origin = transform(object.origin)?;
            object.rotation = (object.rotation + region.rotation) % 6;
            for column in &mut object.occupancy {
                column.position =
                    contextual(&region.id, "transform", world(region, column.position))?;
            }
            chunks
                .get_mut(&object.origin.column.chunk())
                .ok_or_else(|| {
                    CompileDiagnostics::one(&region.id, "package", "missing object chunk")
                })?
                .semantics
                .objects
                .push(object);
        }
    }
    for (coordinate, chunk) in &mut chunks {
        chunk.source_fingerprint = hashed(&(COMPILER_VERSION, contributors.get(coordinate)))?;
    }
    let descriptors = chunks
        .keys()
        .map(|coordinate| ChunkDescriptor {
            coordinate: *coordinate,
            fingerprint: 0,
            path: format!("chunks/{}_{}.ron", coordinate.q, coordinate.r),
        })
        .collect();
    let mut package = WorldPackage {
        manifest: WorldManifest {
            schema_version: SCHEMA_VERSION,
            world_id: source.id.clone(),
            compiler_version: COMPILER_VERSION.into(),
            source_fingerprint: source_key,
            materials: source.materials.clone(),
            regions,
            chunks: descriptors,
            boundaries: seams.iter().map(|seam| seam.contract.clone()).collect(),
            summary,
            features,
            fingerprint: 0,
        },
        chunks,
    };
    package.seal().map_err(|error| {
        CompileDiagnostics::one(&source.id, "package validation", error.to_string())
    })?;
    Ok(package)
}
