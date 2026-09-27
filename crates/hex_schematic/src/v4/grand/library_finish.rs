//! Local library paving, applied only to already occupied floor-top voxels.
//!
//! Warm stone separates the floor from the cool walls and ceiling. One hex of
//! dark slate marks the perimeter and changes between chamber/gallery heights;
//! there is no raised trim, repeated checkerboard, or changed cave geometry.
use super::*;

fn material(p: WorldHex, floor: i32, ceiling: i32) -> &'static str {
    let border = DIRS.into_iter().any(|(q, r)| {
        library_cavity(WorldHex::new(p.q + q, p.r + r)).is_none_or(|(next_floor, next_ceiling)| {
            (next_floor - floor).abs() > 1 || (next_ceiling - ceiling).abs() >= 12
        })
    });
    if border {
        "slate"
    } else {
        "limestone"
    }
}

pub(super) fn floor(p: WorldHex, runs: &mut Vec<VoxelRun>) {
    let Some((floor, ceiling)) = library_cavity(p) else {
        return;
    };
    // The root temple already has its own authored floor treatment.
    if floor < terrain::LIBRARY_FLOOR {
        return;
    }
    let finish = material(p, floor, ceiling);
    let mut out = Vec::with_capacity(runs.len() + 2);
    for old in std::mem::take(runs) {
        // Both existing materials and both finishes resolve to ordinary stone.
        // Do not replace another substance, create missing support, or fill a void.
        if old.material == finish
            || !matches!(old.material.as_str(), "stone" | "slate")
            || !(old.bottom..old.top).contains(&floor)
        {
            out.push(old);
            continue;
        }
        if old.bottom < floor {
            out.push(run(old.bottom, floor, &old.material));
        }
        out.push(run(floor, floor + 1, finish));
        if old.top > floor + 1 {
            out.push(run(floor + 1, old.top, &old.material));
        }
    }
    *runs = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical(runs: &[VoxelRun]) -> Vec<(i32, i32, String)> {
        let mut out: Vec<(i32, i32, String)> = Vec::new();
        for run in runs {
            let policy = match run.material.as_str() {
                "limestone" | "slate" => "stone",
                other => other,
            };
            if let Some(previous) = out.last_mut() {
                if previous.1 == run.bottom && previous.2 == policy {
                    previous.1 = run.top;
                    continue;
                }
            }
            out.push((run.bottom, run.top, policy.to_owned()));
        }
        out
    }

    #[test]
    fn library_finish_preserves_every_column_interval_liquid_and_stone_policy() {
        let mut source: GrandSpec = ron::from_str(include_str!(
            "../../../../../assets/config/v4/grand-v4/world.ron"
        ))
        .expect("Grand source");
        source.full_dressing = false;
        source.geography = None; // This regression preserves the pre-r02 authoring path.
        let g = GrandCompiler::new(source).expect("terrain");
        let mut warm = 0;
        let mut border = 0;
        let mut floors = std::collections::BTreeSet::new();
        for r in -410..=-190 {
            for q in -220..=400 {
                let p = WorldHex::new(q, r);
                let Some((floor, ceiling)) = library_cavity(p) else {
                    continue;
                };
                let (before, before_liquid) = g.column_without_library_finish(p);
                let (after, after_liquid) = g.column(p);
                assert_eq!(
                    canonical(&before.runs),
                    canonical(&after.runs),
                    "changed geometry/policy at {p:?}"
                );
                assert_eq!(before_liquid, after_liquid, "changed liquid at {p:?}");
                for candidate in &after.runs {
                    if candidate.material == "limestone" {
                        assert_eq!((candidate.bottom, candidate.top), (floor, floor + 1));
                        warm += 1;
                    }
                }
                border += usize::from(material(p, floor, ceiling) == "slate");
                floors.insert(floor);
            }
        }
        assert!(warm > 10_000, "a coherent floor, not isolated tiles");
        assert!(border > 1_000, "continuous perimeter/threshold definition");
        assert!(floors.len() > 100, "include the whole ascending stair");
        let stone = g.materials.iter().find(|m| m.id == "stone").expect("stone");
        for name in ["limestone", "slate"] {
            let finish = g.materials.iter().find(|m| m.id == name).expect("finish");
            assert_eq!(
                (finish.solid, finish.diggable),
                (stone.solid, stone.diggable)
            );
        }
    }
}
