//! Four named garden silhouettes, using the existing exact landmark lifecycle.
//! Root locations come from the public manifest feature index. Root chunks are
//! read once and released; this neither adds residency interests nor grants
//! gameplay collision at a distance.
use super::*;
use hex_world_contracts::{FeatureSummary, WorldManifest};

const OBJECTS: [(&str, &str); 4] = [
    ("grand/garden-arcades", "structure/grand-garden-court"),
    ("grand/garden-beds", "decor/grand-garden-beds"),
    ("grand/shrine/water", "structure/grand-shrine"),
    ("grand/fountain-rim", "structure/grand-fountain-rim"),
];
// Dressed01 has 216 columns / 307 runs across all four objects. Per-object
// bounds retain headroom for sparse edits without borrowing the huge tree cap.
// Total retained geometry is at most 512 columns, 2,048 runs and 65,536 vertices.
const LIMITS: SurfaceLimits = SurfaceLimits {
    columns: 128,
    runs: 512,
    vertices: 16_384,
};

fn features(manifest: &WorldManifest, required: bool) -> Result<Vec<&FeatureSummary>, String> {
    let mut selected = Vec::new();
    for (id, asset) in OBJECTS {
        if let Some(feature) = manifest.features.iter().find(|f| f.id == id) {
            if feature.asset.as_deref() != Some(asset) {
                return Err(format!("garden feature {id} has an unexpected asset"));
            }
            selected.push(feature);
        }
    }
    if selected.len() != OBJECTS.len() && (required || !selected.is_empty()) {
        return Err("garden feature set is incomplete".into());
    }
    Ok(selected)
}

pub(super) fn load(state: &StreamedArena) -> Result<Vec<Loaded>, String> {
    let source =
        FileChunkSource::open_workspace(package_path_for(ArenaMap::GrandV4), IoLimits::default())
            .map_err(|error| error.to_string())?;
    if source.manifest().fingerprint != state.runtime.manifest().fingerprint {
        return Err("garden source changed since arena initialization".into());
    }
    load_source(
        &source,
        state.overview.level_height,
        &state.edits,
        state.overview.building_count > 0,
    )
}

fn load_source(
    source: &FileChunkSource,
    level_height: f32,
    edits: &FiniteWorldSession,
    required: bool,
) -> Result<Vec<Loaded>, String> {
    let selected = features(source.manifest(), required)?;
    let palette: BTreeMap<_, _> = source
        .manifest()
        .materials
        .iter()
        .filter(|m| m.solid)
        .map(|m| {
            let [r, g, b, a] = m.color;
            let c = Color::srgba_u8(r, g, b, a).to_linear();
            (m.id.clone(), [c.red, c.green, c.blue, c.alpha])
        })
        .collect();
    let mut roots = BTreeMap::new();
    let mut loaded = Vec::new();
    for feature in selected {
        let chunk = feature.anchor.column.chunk();
        if let std::collections::btree_map::Entry::Vacant(entry) = roots.entry(chunk) {
            entry.insert(
                source
                    .load_chunk(chunk)
                    .map_err(|error| error.to_string())?,
            );
        }
        let object = roots
            .get(&chunk)
            .and_then(|p| p.semantics.objects.iter().find(|o| o.id == feature.id))
            .ok_or_else(|| format!("garden root missing object {}", feature.id))?;
        if object.origin != feature.anchor
            || Some(object.asset.as_str()) != feature.asset.as_deref()
        {
            return Err(format!("garden root does not match feature {}", feature.id));
        }
        let (geometry, mesh) =
            TreeGeometry::with_limits(object, level_height, palette.clone(), edits, LIMITS)?;
        loaded.push((object.id.clone(), geometry, mesh));
    }
    // All source validation and budget checks precede any entity/asset spawn.
    info!(
        objects = loaded.len(),
        root_chunks = roots.len(),
        vertices = loaded
            .iter()
            .map(|(_, _, mesh)| mesh.count_vertices())
            .sum::<usize>(),
        "Grand garden proxies prepared"
    );
    Ok(loaded)
}

#[cfg(test)]
#[path = "garden_tests.rs"]
mod tests;
