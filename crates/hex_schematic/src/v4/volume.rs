//! Canonical occupied-interval algebra. Air is implicit and stacks remain exact.

use hex_world_contracts::VoxelRun;

pub(super) fn canonicalize(mut runs: Vec<VoxelRun>) -> Result<Vec<VoxelRun>, String> {
    runs.sort_by_key(|run| (run.bottom, run.top));
    let mut result: Vec<VoxelRun> = Vec::new();
    for run in runs {
        if run.bottom < 0 || run.top <= run.bottom || run.material.is_empty() {
            return Err("invalid occupied interval".to_owned());
        }
        if let Some(previous) = result.last_mut() {
            if previous.top > run.bottom {
                return Err("overlapping occupied intervals".to_owned());
            }
            if previous.top == run.bottom && previous.material == run.material {
                previous.top = run.top;
                continue;
            }
        }
        result.push(run);
    }
    Ok(result)
}

pub(super) fn replace(
    runs: &mut Vec<VoxelRun>,
    bottom: i32,
    top: i32,
    material: Option<&str>,
) -> Result<(), String> {
    if bottom < 1 || top <= bottom {
        return Err("volume operation has invalid bounds or would modify bedrock".to_owned());
    }
    let mut result = Vec::new();
    for run in runs.iter() {
        if run.top <= bottom || run.bottom >= top {
            result.push(run.clone());
            continue;
        }
        if run.bottom < bottom {
            result.push(VoxelRun {
                bottom: run.bottom,
                top: bottom,
                material: run.material.clone(),
            });
        }
        if run.top > top {
            result.push(VoxelRun {
                bottom: top,
                top: run.top,
                material: run.material.clone(),
            });
        }
    }
    if let Some(material) = material {
        result.push(VoxelRun {
            bottom,
            top,
            material: material.to_owned(),
        });
    }
    *runs = canonicalize(result)?;
    Ok(())
}

pub(super) fn insert(runs: &mut Vec<VoxelRun>, run: VoxelRun) -> Result<(), String> {
    if runs
        .iter()
        .any(|old| old.bottom < run.top && run.bottom < old.top)
    {
        return Err("additive structure intersects an existing occupied interval".to_owned());
    }
    let mut result = runs.clone();
    result.push(run);
    *runs = canonicalize(result)?;
    Ok(())
}

pub(super) fn material_at(runs: &[VoxelRun], level: i32) -> Option<&str> {
    runs.iter()
        .find(|run| run.bottom <= level && level < run.top)
        .map(|run| run.material.as_str())
}

pub(super) fn clear_above(runs: &[VoxelRun], level: i32) -> Option<u32> {
    let from = level.checked_add(1)?;
    if runs.iter().any(|run| run.bottom <= from && from < run.top) {
        return Some(0);
    }
    runs.iter()
        .filter(|run| run.bottom > from)
        .map(|run| run.bottom.abs_diff(from))
        .min()
}
