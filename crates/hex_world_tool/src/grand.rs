//! Streaming authoring output: at most one complete terrain chunk is staged.
use hex_schematic::v4::grand::{GrandCompiler, GrandSpec};
use hex_world_contracts::{ChunkDescriptor, MapSummaryCell};
use hex_world_runtime::{FileChunkSource, IoLimits};
use serde::Serialize;
use std::{error::Error, fs, path::Path, time::Instant};

#[derive(Serialize)]
struct Receipt {
    world_id: String,
    source_fingerprint: u64,
    package_fingerprint: u64,
    columns: usize,
    chunks: usize,
    liquids: usize,
    objects: usize,
    trees: usize,
    buildings: usize,
    solid_top_levels: [i32; 2],
    sea_top_level: i32,
    player_spawn: [f32; 3],
    elapsed_seconds: f64,
    mainland_columns: usize,
    canonical_mainland_columns: usize,
    crystal_columns: usize,
    canonical_crystal_columns: usize,
    sailing_reference_seconds: f64,
    sailing_reference_wind_speed: f64,
    sailing_reference_integrated_units: f64,
    sailing_verified_in_engine: bool,
    strict: bool,
    presentation_reviewed: bool,
}
/// Compile a runtime-loaded northern source into a new immutable V4 directory.
pub fn compile(source: &Path, output: &Path) -> Result<String, Box<dyn Error>> {
    if output.exists() {
        return Err("output exists; use a fresh immutable package directory".into());
    }
    let started = Instant::now();
    let spec: GrandSpec = ron::from_str(std::str::from_utf8(&super::read_bounded(source)?)?)?;
    let compiler = GrandCompiler::new(spec)?;
    let mut manifest = compiler.manifest();
    let stage = output.with_extension(format!("staging-{}", std::process::id()));
    fs::create_dir_all(stage.join("chunks"))?;
    let mut columns = 0;
    let mut liquids = 0;
    let mut objects = 0;
    let mut lowest = i32::MAX;
    let mut highest = i32::MIN;
    for id in compiler.chunk_ids() {
        let Some(chunk) = compiler.chunk(id)? else {
            continue;
        };
        columns += chunk.columns.len();
        liquids += chunk.semantics.liquids.len();
        objects += chunk.semantics.objects.len();
        for column in &chunk.columns {
            if let Some(top) = column
                .runs
                .iter()
                .filter(|r| r.material != "water")
                .map(|r| r.top)
                .max()
            {
                lowest = lowest.min(top);
                highest = highest.max(top);
            }
            if column.position.q.rem_euclid(8) == 0 && column.position.r.rem_euclid(8) == 0 {
                if let Some(run) = column.runs.last() {
                    manifest.summary.push(MapSummaryCell {
                        position: column.position,
                        level: run.top - 1,
                        material: run.material.clone(),
                        region_id: "grand".into(),
                    });
                }
            }
        }
        manifest.features.extend(chunk.features.iter().cloned());
        let path = format!("chunks/{}_{}.ron", id.q, id.r);
        fs::write(stage.join(&path), ron::to_string(&chunk)?)?;
        manifest.chunks.push(ChunkDescriptor {
            coordinate: id,
            fingerprint: chunk.fingerprint,
            path,
        });
    }
    manifest.seal()?;
    fs::write(stage.join("manifest.ron"), ron::to_string(&manifest)?)?;
    // Admission validates membership, materials and chunk hash through production IO.
    let admitted = FileChunkSource::open_workspace(&stage, IoLimits::default())?;
    for descriptor in &manifest.chunks {
        let _checked = admitted.load_chunk(descriptor.coordinate)?;
    }
    let mut overview = compiler.overview();
    overview.package_fingerprint = manifest.fingerprint;
    fs::write(stage.join("grand-overview.ron"), ron::to_string(&overview)?)?;
    let sites = compiler.sites(manifest.fingerprint)?;
    fs::write(stage.join("arena-sites.ron"), ron::to_string(&sites)?)?;
    fs::write(
        stage.join("grand-biomes.ron"),
        ron::to_string(&compiler.biomes(manifest.fingerprint))?,
    )?;
    let receipt = Receipt {
        world_id: manifest.world_id.clone(),
        source_fingerprint: manifest.source_fingerprint,
        package_fingerprint: manifest.fingerprint,
        columns,
        chunks: manifest.chunks.len(),
        liquids,
        objects,
        trees: compiler.tree_count,
        buildings: if compiler.source.full_dressing { 8 } else { 0 },
        solid_top_levels: [lowest, highest],
        sea_top_level: hex_schematic::v4::grand::SEA_TOP,
        player_spawn: overview.player_spawn,
        elapsed_seconds: started.elapsed().as_secs_f64(),
        mainland_columns: compiler.mainland_columns,
        canonical_mainland_columns: compiler.source.canonical_mainland_columns,
        crystal_columns: compiler.crystal_columns,
        canonical_crystal_columns: compiler.source.canonical_crystal_columns,
        sailing_reference_seconds: 45.0,
        sailing_reference_wind_speed: 9.0,
        sailing_reference_integrated_units: 793.9485,
        sailing_verified_in_engine: false,
        strict: true,
        presentation_reviewed: false,
    };
    fs::write(
        stage.join("compile-receipt.json"),
        serde_json::to_string_pretty(&receipt)?,
    )?;
    fs::rename(stage, output)?;
    Ok(serde_json::to_string_pretty(&receipt)?)
}
