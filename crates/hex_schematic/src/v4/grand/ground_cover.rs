//! Irregular, grounded grove-floor patches; never solid authored objects.
use super::*;
use crate::v4::northern::ground_cover::{
    GroundCover, GroundCoverChunk, GroundTuft, MAX_CHUNK_TUFTS,
};

pub(super) fn compile(
    g: &GrandCompiler,
    objects: &[ObjectInstance],
) -> Result<GroundCover, ContractError> {
    let mut occupied: BTreeMap<WorldHex, Vec<(i32, i32)>> = BTreeMap::new();
    for object in objects {
        for column in &object.occupancy {
            occupied
                .entry(column.position)
                .or_default()
                .extend(column.runs.iter().map(|r| (r.bottom, r.top)));
        }
    }
    let mut batches: BTreeMap<ChunkId, Vec<(u64, GroundTuft)>> = BTreeMap::new();
    for &(r, first, last) in &g.source.mainland_rows {
        for q in first..=last {
            let p = WorldHex::new(q, r);
            let [x, z] = world_xz(p);
            let density = dressing::forest_density(g, p);
            if density < 0.12 || dressing::reserved_growth(g, p) {
                continue;
            }
            // Several overlapping wavelengths produce patches, feathered edges,
            // and open ground. There are no review-camera-specific plantings.
            let patches = 0.50
                + 0.24 * (x * 0.19 + (z * 0.08).sin()).sin()
                + 0.18 * (z * 0.23 - x * 0.07).cos();
            let key = dressing::forest_hash(g.source.seed ^ 0x0047_5241_5353, p);
            if patches < 0.39 || (key % 10_000) as f64 / 10_000. > density * patches * 0.56 {
                continue;
            }
            let surface = g.surface(p);
            if surface.water.is_some() {
                continue;
            }
            let (column, liquid) = g.column(p);
            let Some(top) = column.runs.last() else {
                continue;
            };
            if liquid.is_some()
                || !matches!(top.material.as_str(), "moss" | "soil")
                || top.top != surface.level + 1
                || occupied.get(&p).is_some_and(|runs| {
                    runs.iter()
                        .any(|(lo, hi)| *lo < top.top + 2 && *hi > top.top)
                })
                || DIRS.iter().any(|(a, b)| {
                    (g.surface(WorldHex::new(q + a, r + b)).level - surface.level).abs() > 2
                })
            {
                continue;
            }
            batches.entry(p.chunk()).or_default().push((
                key,
                GroundTuft {
                    support: VoxelPosition {
                        column: p,
                        level: top.top - 1,
                    },
                    material: top.material.clone(),
                    variant: (key % 6) as u8,
                },
            ));
        }
    }
    let chunks = batches
        .into_iter()
        .map(|(coordinate, mut candidates)| {
            candidates.sort_by_key(|(key, tuft)| (*key, tuft.support.column));
            candidates.truncate(MAX_CHUNK_TUFTS);
            let mut tufts: Vec<_> = candidates.into_iter().map(|(_, tuft)| tuft).collect();
            tufts.sort_by_key(|t| t.support.column);
            GroundCoverChunk { coordinate, tufts }
        })
        .collect();
    let cover = GroundCover { version: 1, chunks };
    cover.validate_in_bounds(RADIUS as u32, [0, MAX_LEVEL])?;
    Ok(cover)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ground_cover_is_bounded_grounded_reserved_and_has_no_occupancy() {
        let g = dressing::test_compiler(true);
        let cover = g.ground_cover.as_ref().expect("dressed companion");
        cover
            .validate_in_bounds(RADIUS as u32, [0, MAX_LEVEL])
            .expect("bounded source");
        let mut count = 0;
        let mut covered_chunks = std::collections::BTreeSet::new();
        for batch in &cover.chunks {
            for tuft in &batch.tufts {
                count += 1;
                let p = tuft.support.column;
                assert!(!dressing::reserved_growth(&g, p));
                assert!(dressing::forest_density(&g, p) >= 0.12);
                let (column, liquid) = g.column(p);
                assert!(liquid.is_none());
                assert_eq!(
                    column.material_at(tuft.support.level),
                    Some(tuft.material.as_str())
                );
                for level in tuft.support.level + 1..=tuft.support.level + 2 {
                    assert!(
                        column.material_at(level).is_none(),
                        "cover never becomes terrain occupancy"
                    );
                    assert!(
                        !g.influences
                            .get(&p.chunk())
                            .into_iter()
                            .flatten()
                            .flat_map(|i| &i.occupancy)
                            .any(|c| c.position == p && c.material_at(level).is_some()),
                        "cover never becomes object occupancy"
                    );
                }
                covered_chunks.insert(p.chunk());
            }
        }
        assert!(
            count > 10_000,
            "a whole forest needs useful ground coverage, got {count}"
        );
        assert!(
            covered_chunks.len() > 100,
            "ground patches must cover the shared woodland rather than a camera-local planting"
        );
        let mut old_overview = g.overview();
        old_overview.ground_cover = None;
        let old_bytes = ron::to_string(&old_overview).expect("legacy-compatible overview");
        assert!(!old_bytes.contains("ground_cover:"));
        let decoded: NorthernOverview =
            ron::from_str(&old_bytes).expect("old overview remains loadable");
        assert!(decoded.ground_cover.is_none());
        let mut invalid = cover.clone();
        invalid
            .chunks
            .first_mut()
            .expect("chunks")
            .tufts
            .first_mut()
            .expect("tufts")
            .material = "stone".into();
        assert!(
            invalid
                .validate_in_bounds(RADIUS as u32, [0, MAX_LEVEL])
                .is_err()
        );
    }
}
