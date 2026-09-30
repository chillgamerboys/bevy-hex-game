//! Explicit small diagnostic export; omitted regions retain their ordinary presentation.
use super::*;
use crate::v4::northern::terrain_surface::{
    SolidProfile, SolidRun, SurfaceBuilder, SurfaceChunk, TerrainSurfaceOverview, CLIFF,
    GROUND_COVER, OBJECT_CONTACT, OUTSIDE_PROFILE, PATCH_BOUNDARY, STACKED, WET,
};
use std::collections::BTreeSet;

// These are diagnostic storage addresses, not a second geographical feature definition.
const SAMPLE: [ChunkId; 4] = [
    ChunkId { q: 28, r: -20 },
    ChunkId { q: 28, r: -19 },
    ChunkId { q: 29, r: -20 },
    ChunkId { q: 29, r: -19 },
];

fn invalid(message: &str) -> ContractError {
    ContractError::new("grand.surface_sample", message)
}

pub(super) fn compile(g: &GrandCompiler) -> Result<TerrainSurfaceOverview, ContractError> {
    let selected: BTreeSet<_> = SAMPLE.into_iter().collect();
    let coordinates: BTreeSet<_> = SAMPLE
        .into_iter()
        .flat_map(|c| {
            std::iter::once(c).chain(DIRS.map(|(q, r)| ChunkId {
                q: c.q + q,
                r: c.r + r,
            }))
        })
        .collect();
    let palette: BTreeMap<_, _> = g
        .materials
        .iter()
        .enumerate()
        .map(|(i, m)| {
            Ok((
                m.id.as_str(),
                (
                    u16::try_from(i)
                        .map_err(|error| invalid(&format!("material index: {error}")))?,
                    m.solid,
                ),
            ))
        })
        .collect::<Result<_, ContractError>>()?;
    let mut facts = BTreeMap::new();
    // One native neighbor beyond every stored column suffices for protection decisions;
    // only complete selected/halo chunks are serialized, in canonical order.
    for c in &coordinates {
        for r in -1..=16 {
            for q in -1..=16 {
                let p = WorldHex::new(c.q * 16 + q, c.r * 16 + r);
                if p.checked_distance(WorldHex::new(0, 0))? > RADIUS as u64
                    || facts.contains_key(&p)
                {
                    continue;
                }
                let (column, liquid) = g.column(p);
                let mut runs: Vec<SolidRun> = Vec::new();
                for run in column.runs {
                    let &(material, solid) = palette
                        .get(run.material.as_str())
                        .ok_or_else(|| invalid("unknown material"))?;
                    if !solid {
                        continue;
                    }
                    let bottom = i16::try_from(run.bottom)
                        .map_err(|error| invalid(&format!("bottom range: {error}")))?;
                    let top = i16::try_from(run.top)
                        .map_err(|error| invalid(&format!("top range: {error}")))?;
                    if let Some(last) = runs
                        .last_mut()
                        .filter(|last| last.top == bottom && last.material == material)
                    {
                        last.top = top;
                    } else {
                        runs.push(SolidRun {
                            bottom,
                            top,
                            material,
                        });
                    }
                }
                facts.insert(p, (SolidProfile { runs }, liquid.is_some()));
            }
        }
    }
    let cover: BTreeSet<_> = g
        .ground_cover
        .iter()
        .flat_map(|c| &c.chunks)
        .flat_map(|c| &c.tufts)
        .map(|t| t.support.column)
        .collect();
    let mut object_columns: BTreeMap<WorldHex, Vec<(i32, i32)>> = BTreeMap::new();
    for c in &coordinates {
        for object in g.influences.get(c).into_iter().flatten() {
            for column in &object.occupancy {
                object_columns
                    .entry(column.position)
                    .or_default()
                    .extend(column.runs.iter().map(|run| (run.bottom, run.top)));
            }
        }
    }
    let mut result = TerrainSurfaceOverview {
        version: 1,
        tolerance: 2.0,
        profiles: vec![],
        chunks: vec![],
    };
    let mut dictionary = BTreeMap::new();
    for coordinate in coordinates {
        let mut profiles = Vec::with_capacity(256);
        let mut protection = Vec::with_capacity(256);
        for r in 0..16 {
            for q in 0..16 {
                let p = WorldHex::new(coordinate.q * 16 + q, coordinate.r * 16 + r);
                let Some((profile, wet)) = facts.get(&p) else {
                    profiles.push(OUTSIDE_PROFILE);
                    protection.push(0);
                    continue;
                };
                let id = if let Some(id) = dictionary.get(profile) {
                    *id
                } else {
                    let id = u16::try_from(result.profiles.len())
                        .map_err(|error| invalid(&format!("profile budget: {error}")))?;
                    dictionary.insert(profile.clone(), id);
                    result.profiles.push(profile.clone());
                    id
                };
                let mut flags = if *wet { WET } else { 0 };
                if q == 0 || q == 15 || r == 0 || r == 15 {
                    flags |= PATCH_BOUNDARY;
                }
                if profile.runs.first().is_some_and(|run| run.bottom > 0)
                    || profile.runs.windows(2).any(|pair| {
                        pair.first()
                            .zip(pair.get(1))
                            .is_some_and(|(a, b)| a.top < b.bottom)
                    })
                {
                    flags |= STACKED;
                }
                if cover.contains(&p) {
                    flags |= GROUND_COVER;
                }
                if let Some(top) = profile.runs.last().map(|run| i32::from(run.top)) {
                    // Any shared corner of remaining eligible columns spans at most
                    // eleven levels: half its range is strictly below the 2u ceiling.
                    if DIRS.iter().any(|(dq, dr)| {
                        facts
                            .get(&WorldHex::new(p.q + dq, p.r + dr))
                            .and_then(|(other, _)| other.runs.last())
                            .is_none_or(|run| (top - i32::from(run.top)).abs() > 11)
                    }) {
                        flags |= CLIFF;
                    }
                    if object_columns.get(&p).is_some_and(|runs| {
                        runs.iter().any(|(lo, hi)| *lo < top + 6 && *hi > top - 6)
                    }) {
                        flags |= OBJECT_CONTACT;
                    }
                }
                profiles.push(id);
                protection.push(flags);
            }
        }
        result.chunks.push(SurfaceChunk {
            coordinate,
            profiles,
            protection,
            surface: None,
        });
    }
    let builder = SurfaceBuilder::new(
        &result,
        RADIUS as u32,
        [0, MAX_LEVEL],
        &g.materials,
        f64::from(LEVEL_HEIGHT as f32),
    )?;
    let patches = selected
        .into_iter()
        .map(|c| builder.build_patch(c).map(|patch| (c, patch)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    for chunk in &mut result.chunks {
        chunk.surface = patches.get(&chunk.coordinate).cloned();
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Explicit bounded compiler diagnostic; run once with the current canonical source"]
    fn canonical_crystal_sample_has_four_certified_patches_and_exact_handoffs() {
        let g = dressing::test_compiler(true);
        let sample = compile(&g).expect("bounded diagnostic source");
        let builder = SurfaceBuilder::new(
            &sample,
            RADIUS as u32,
            [0, MAX_LEVEL],
            &g.materials,
            f64::from(LEVEL_HEIGHT as f32),
        )
        .unwrap();
        let mut vertices = 0;
        let mut triangles = 0;
        for chunk in sample.chunks.iter().filter(|c| c.surface.is_some()) {
            for (i, flags) in chunk.protection.iter().enumerate() {
                if i % 16 == 0 || i % 16 == 15 || i / 16 == 0 || i / 16 == 15 {
                    assert_ne!(flags & PATCH_BOUNDARY, 0);
                }
            }
            let faces = builder
                .build_faces(chunk.coordinate, chunk.surface.as_ref().unwrap(), 100_000)
                .unwrap();
            vertices += faces.vertices;
            triangles += faces.triangles;
        }
        assert_eq!(
            sample.chunks.iter().filter(|c| c.surface.is_some()).count(),
            4
        );
        println!("CRYSTAL_SAMPLE profiles={} chunks={} vertices={vertices} triangles={triangles} packed_bytes={}", sample.profiles.len(), sample.chunks.len(), vertices*40+triangles*12);
    }
}
