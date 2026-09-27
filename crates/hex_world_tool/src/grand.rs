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
    let compiler = if let Some(name) = &spec.geography {
        if !name.ends_with(".json") || name.contains(['/', '\\']) || name.contains("..") {
            return Err("geography must be a single sibling JSON filename".into());
        }
        let path = source
            .parent()
            .ok_or("source lacks parent directory")?
            .join(name);
        let bytes = super::read_bounded(&path)?;
        let document = serde_json::from_slice(&bytes)?;
        GrandCompiler::with_geography(spec, document, &bytes)?
    } else {
        GrandCompiler::new(spec)?
    };
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
        let _checked = admitted
            .load_chunk(descriptor.coordinate)
            .map_err(|error| format!("Grand chunk {:?}: {error}", descriptor.coordinate))?;
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
        buildings: objects.saturating_sub(compiler.tree_count),
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn approved_r02_plain_geography_has_measured_area_and_exact_routes(
    ) -> Result<(), Box<dyn Error>> {
        let mut spec: GrandSpec =
            ron::from_str(include_str!("../../../assets/config/v4/grand-v4/world.ron"))?;
        spec.full_dressing = false;
        spec.geography = Some("geography-r02.json".into());
        let bytes = include_bytes!("../../../assets/config/v4/grand-v4/geography-r02.json");
        let compiler = GrandCompiler::with_geography(spec, serde_json::from_slice(bytes)?, bytes)?;
        assert_eq!(compiler.mainland_columns, 653282);
        assert_eq!(compiler.crystal_columns, 22183);
        let sites = serde_json::to_value(compiler.sites(0)?)?;
        let overview = compiler.overview();
        let inland = overview
            .inland_water
            .as_ref()
            .ok_or("missing inland facts")?;
        let liquid_columns: usize = inland.chunks.iter().map(|c| c.columns.len()).sum();
        let terrain_columns: usize = inland
            .chunks
            .iter()
            .map(|c| c.terrain.len() + c.halo.len())
            .sum();
        let terrain_runs: usize = inland
            .chunks
            .iter()
            .flat_map(|c| c.terrain.iter().chain(&c.halo))
            .map(|c| c.runs.len())
            .sum();
        for id in [
            "grand-overview",
            "grand-mainland",
            "grand-mainland-south",
            "grand-mainland-east",
            "grand-mainland-northwest",
            "grand-crystal-frozen",
            "grand-valley-tree-bank",
            "grand-valley-lake-bank",
            "grand-valley-waterfall-approach",
            "grand-garden",
            "grand-garden-ground",
            "grand-waterfall",
            "grand-valley-lake",
            "grand-world-tree",
            "grand-roots-entrance",
            "grand-shrine-plant",
            "grand-forest",
            "grand-forest-ground",
            "grand-forest-ground-reverse",
            "grand-river-exit",
            "grand-island-landing",
            "grand-summit",
            "grand-shrine-air",
            "grand-crystal",
            "grand-shrine-earth",
            "grand-frozen-woods",
            "grand-volcano",
            "grand-shrine-fire",
            "grand-bay",
            "grand-bay-baseline",
            "grand-bay-reverse",
            "grand-waterline",
            "grand-underwater",
            "grand-waterfall-cave",
            "grand-library",
            "grand-library-reverse",
            "grand-library-upper",
            "grand-shadow-tunnel",
            "grand-shadow-reverse",
            "grand-shadow-exit",
            "grand-motion-forest-forward",
            "grand-motion-river-forward",
        ] {
            assert!(
                overview.review_cameras.contains_key(id),
                "missing canonical camera {id}"
            );
        }
        println!("r02 producer: {} routes, {} encounters; inland {} chunks, {} water columns, {} solid+halo columns / {} runs, {} serialized overview bytes", sites.get("routes").and_then(serde_json::Value::as_array).ok_or("missing routes")?.len(), sites.get("encounters").and_then(serde_json::Value::as_array).ok_or("missing encounters")?.len(), inland.chunks.len(), liquid_columns, terrain_columns, terrain_runs, ron::to_string(&overview)?.len());
        Ok(())
    }
}
