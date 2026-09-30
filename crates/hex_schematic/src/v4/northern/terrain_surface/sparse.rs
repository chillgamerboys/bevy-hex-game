//! Exact source-only halo storage; no topology or residency policy.
use super::{build::Source, *};

const DIRECTIONS: [[i64; 2]; 6] = [[1, 0], [1, -1], [0, -1], [-1, 0], [-1, 1], [0, 1]];

pub(super) fn validate(
    overview: &TerrainSurfaceOverview,
    radius: u32,
    stacked: &[bool],
) -> Result<(), ContractError> {
    let chunks: BTreeSet<_> = overview.chunks.iter().map(|c| c.coordinate).collect();
    let mut previous = None;
    for fact in &overview.halo {
        if previous.is_some_and(|p| p >= fact.column)
            || chunks.contains(&ChunkId::from_world_hex(fact.column))
            || fact.protection & !KNOWN_PROTECTION != 0
        {
            return Err(invalid("unordered, duplicate or overlapping sparse halo"));
        }
        previous = Some(fact.column);
        let inside = fact.column.checked_distance(WorldHex::new(0, 0))? <= u64::from(radius);
        if (fact.profile == OUTSIDE_PROFILE) == inside
            || (fact.profile == OUTSIDE_PROFILE && fact.protection != 0)
            || (fact.profile != OUTSIDE_PROFILE
                && overview.profiles.get(usize::from(fact.profile)).is_none())
            || (stacked
                .get(usize::from(fact.profile))
                .copied()
                .unwrap_or(false)
                && fact.protection & STACKED == 0)
        {
            return Err(invalid(
                "sparse halo profile differs from the finite source",
            ));
        }
    }
    Ok(())
}

pub(super) fn compact(
    overview: &TerrainSurfaceOverview,
) -> Result<TerrainSurfaceOverview, ContractError> {
    // Structural guards must precede indexing even for callers preparing a payload
    // that has not yet supplied its physical/material validation inputs.
    if overview.chunks.len() > MAX_SURFACE_CHUNKS
        || overview.halo.len() > MAX_SURFACE_COLUMNS.saturating_sub(overview.chunks.len() * 256)
        || overview
            .chunks
            .windows(2)
            .any(|p| matches!(p, [a,b] if a.coordinate >= b.coordinate))
        || overview
            .chunks
            .iter()
            .any(|c| c.profiles.len() != 256 || c.protection.len() != 256)
        || overview
            .halo
            .windows(2)
            .any(|p| matches!(p, [a,b] if a.column >= b.column))
        || overview.halo.iter().any(|h| {
            overview
                .chunks
                .binary_search_by_key(&ChunkId::from_world_hex(h.column), |c| c.coordinate)
                .is_ok()
        })
    {
        return Err(invalid("cannot compact malformed source storage"));
    }
    let source = Source::new(overview);
    let selected: BTreeSet<_> = overview
        .chunks
        .iter()
        .filter(|c| c.surface.is_some())
        .map(|c| c.coordinate)
        .collect();
    if selected.is_empty() {
        return Err(invalid(
            "halo compaction requires explicitly selected patches",
        ));
    }
    let mut required = BTreeSet::new();
    for coordinate in &selected {
        let origin = coordinate.origin()?;
        for r in 0..16_i64 {
            for q in 0..16_i64 {
                let column = origin.checked_add(WorldHex::new(q, r))?;
                for [dq, dr] in DIRECTIONS {
                    let neighbor = column.checked_add(WorldHex::new(dq, dr))?;
                    if !selected.contains(&ChunkId::from_world_hex(neighbor)) {
                        required.insert(neighbor);
                    }
                }
            }
        }
    }
    let halo = required
        .into_iter()
        .map(|column| {
            let (profile, protection) = source.fact(column)?;
            Ok(SurfaceHaloColumn {
                column,
                profile,
                protection,
            })
        })
        .collect::<Result<Vec<_>, ContractError>>()?;
    let mut result = overview.clone();
    result.chunks.retain(|c| selected.contains(&c.coordinate));
    result.halo = halo;
    Ok(result)
}
