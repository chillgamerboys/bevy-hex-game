//! Authoring export of validated public package facts; never parses private files.

use hex_world_contracts::{
    ColumnData, LiquidColumn, MaterialSpec, ObjectInstance, RegionDescriptor, WorldAnchor,
};
use hex_world_runtime::{ChunkSource, FileChunkSource, IoLimits};
use serde::Serialize;
use std::{collections::BTreeMap, error::Error, path::Path};

#[derive(Serialize)]
struct Survey {
    version: u32,
    world_id: String,
    manifest_fingerprint: u64,
    materials: Vec<MaterialSpec>,
    regions: Vec<RegionDescriptor>,
    columns: Vec<ColumnData>,
    objects: Vec<ObjectInstance>,
    liquids: Vec<LiquidColumn>,
    anchors: Vec<WorldAnchor>,
}

fn collect(source: &impl ChunkSource) -> Result<Survey, Box<dyn Error>> {
    let manifest = source.manifest();
    let mut columns = BTreeMap::new();
    let mut objects = BTreeMap::new();
    let mut liquids = vec![];
    let mut anchors = BTreeMap::new();
    for descriptor in &manifest.chunks {
        let chunk = source.load_chunk(descriptor.coordinate)?;
        for column in chunk.columns {
            if columns.insert(column.position, column).is_some() {
                return Err("survey encountered a duplicate terrain column".into());
            }
        }
        for object in chunk.semantics.objects {
            if objects.insert(object.id.clone(), object).is_some() {
                return Err("survey encountered a duplicate root object".into());
            }
        }
        for anchor in chunk.semantics.anchors {
            if anchors.insert(anchor.id.clone(), anchor).is_some() {
                return Err("survey encountered a duplicate anchor".into());
            }
        }
        liquids.extend(chunk.semantics.liquids);
    }
    liquids.sort_by(|a, b| {
        (a.column, a.bottom, a.top, &a.body_id).cmp(&(b.column, b.bottom, b.top, &b.body_id))
    });
    Ok(Survey {
        version: 1,
        world_id: manifest.world_id.clone(),
        manifest_fingerprint: manifest.fingerprint,
        materials: manifest.materials.clone(),
        regions: manifest.regions.clone(),
        columns: columns.into_values().collect(),
        objects: objects.into_values().collect(),
        liquids,
        anchors: anchors.into_values().collect(),
    })
}

pub(super) fn run(package: &Path, output: &Path) -> Result<String, Box<dyn Error>> {
    let source = FileChunkSource::open_workspace(package, IoLimits::default())?;
    let survey = collect(&source)?;
    super::write_json(output, &survey)?;
    Ok(format!(
        "Surveyed {} exact terrain columns and {} root objects from {:016x} into {}",
        survey.columns.len(),
        survey.objects.len(),
        survey.manifest_fingerprint,
        output.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_schematic::v4::{compile_world, parse_world};
    use hex_world_runtime::MemoryChunkSource;

    #[test]
    fn survey_preserves_exact_public_terrain_objects_and_water() {
        let spec = parse_world(include_str!("../../../assets/config/v4/two-regions.ron"))
            .expect("fixture");
        let package = compile_world(&spec).expect("compile fixture");
        let expected_columns: usize = package.chunks.values().map(|c| c.columns.len()).sum();
        let expected_objects: usize = package
            .chunks
            .values()
            .map(|c| c.semantics.objects.len())
            .sum();
        let expected_liquids: usize = package
            .chunks
            .values()
            .map(|c| c.semantics.liquids.len())
            .sum();
        let source = MemoryChunkSource::new(package.clone()).expect("validated source");
        let actual = collect(&source).expect("survey");
        assert_eq!(actual.manifest_fingerprint, package.manifest.fingerprint);
        assert_eq!(actual.columns.len(), expected_columns);
        assert_eq!(actual.objects.len(), expected_objects);
        assert_eq!(actual.liquids.len(), expected_liquids);
        assert!(expected_columns > 0 && expected_liquids > 0);
        for column in &actual.columns {
            assert!(package.chunks.values().any(|c| c.columns.contains(column)));
        }
        assert_eq!(
            serde_json::to_vec(&actual).expect("encode"),
            serde_json::to_vec(&collect(&source).expect("repeat")).expect("encode")
        );
    }
}
