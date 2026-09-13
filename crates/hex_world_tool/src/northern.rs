//! Streaming authoring output: at most one complete terrain chunk is staged.
use hex_schematic::v4::northern::{NorthernCompiler, NorthernSpec};
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
    elapsed_seconds: f64,
    strict: bool,
    presentation_reviewed: bool,
}
/// Compile a runtime-loaded northern source into a new immutable V4 directory.
pub fn compile(source: &Path, output: &Path) -> Result<String, Box<dyn Error>> {
    if output.exists() {
        return Err("output exists; use a fresh immutable package directory".into());
    }
    let started = Instant::now();
    let spec: NorthernSpec = ron::from_str(std::str::from_utf8(&super::read_bounded(source)?)?)?;
    let compiler = NorthernCompiler::new(spec)?;
    let mut manifest = compiler.manifest();
    let stage = output.with_extension(format!("staging-{}", std::process::id()));
    fs::create_dir_all(stage.join("chunks"))?;
    let mut columns = 0;
    let mut liquids = 0;
    let mut objects = 0;
    for id in compiler.chunk_ids() {
        let Some(chunk) = compiler.chunk(id)? else {
            continue;
        };
        columns += chunk.columns.len();
        liquids += chunk.semantics.liquids.len();
        objects += chunk.semantics.objects.len();
        for column in &chunk.columns {
            if column.position.q.rem_euclid(8) == 0 && column.position.r.rem_euclid(8) == 0 {
                if let Some(run) = column.runs.last() {
                    manifest.summary.push(MapSummaryCell {
                        position: column.position,
                        level: run.top - 1,
                        material: run.material.clone(),
                        region_id: "northern".into(),
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
    fs::write(
        stage.join("northern-overview.ron"),
        ron::to_string(&overview)?,
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
        buildings: 4,
        elapsed_seconds: started.elapsed().as_secs_f64(),
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
