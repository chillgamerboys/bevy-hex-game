//! Shared relief-preserving sea fill; all interval tops remain exclusive.
use hex_world_contracts::{
    ColumnData, ContractError, LiquidColumn, LiquidKind, VoxelRun, WorldHex,
};

/// Fill the open interval above the last composed run up to `water_level + 1`.
/// Existing terrain, including its materials and buried relief, is never rewritten.
/// Call before adding above-ground objects; the caller owns liquid material admission.
pub fn fill_sea_column(
    column: WorldHex,
    runs: &mut Vec<VoxelRun>,
    water_level: i32,
    material: &str,
    body_id: &str,
) -> Result<Option<LiquidColumn>, ContractError> {
    let top = water_level
        .checked_add(1)
        .ok_or_else(|| ContractError::new("sea", "surface level overflow"))?;
    let bottom = runs.last().map_or(0, |run| run.top);
    if bottom >= top {
        return Ok(None);
    }
    let mut filled = ColumnData {
        position: column,
        runs: runs.clone(),
    };
    filled.runs.push(VoxelRun {
        bottom,
        top,
        material: material.into(),
    });
    filled.seal()?;
    *runs = filled.runs;
    Ok(Some(LiquidColumn {
        column,
        bottom,
        top,
        kind: LiquidKind::Standing,
        body_id: body_id.into(),
        downstream: vec![],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fill_preserves_uneven_bed_and_all_existing_strata() {
        for top in [6, 18, 33] {
            let mut runs = vec![
                VoxelRun {
                    bottom: 0,
                    top: 3,
                    material: "bedrock".into(),
                },
                VoxelRun {
                    bottom: 3,
                    top,
                    material: "rock".into(),
                },
            ];
            let old = runs.clone();
            let sea = fill_sea_column(WorldHex::new(1, 2), &mut runs, 39, "water", "sea")
                .expect("fill")
                .expect("wet");
            assert_eq!(runs.get(..2), Some(old.as_slice()));
            assert_eq!((sea.bottom, sea.top), (top, 40));
        }
    }
    #[test]
    fn dry_column_and_exact_surface_are_unchanged() {
        for top in [40, 50] {
            let mut runs = vec![VoxelRun {
                bottom: 0,
                top,
                material: "rock".into(),
            }];
            let old = runs.clone();
            assert!(
                fill_sea_column(WorldHex::new(0, 0), &mut runs, 39, "water", "sea")
                    .expect("fill")
                    .is_none()
            );
            assert_eq!(runs, old);
        }
    }
}
