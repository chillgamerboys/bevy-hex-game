//! Complete local illumination influence, derived from uniquely owned source lights.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    ChunkId, ChunkPackage, ContractError, WorldLight, WorldPackage, CHUNK_SIZE,
    MAX_SEMANTIC_RECORDS,
};

/// Maximum candidate chunk addresses examined when compiling one light's influence.
/// This bounds one operation, never the total number of lights or world chunks.
pub const MAX_LIGHT_INFLUENCE_CHUNK_PROBES: u64 = 4096;

pub(crate) fn influence_bounds(light: &WorldLight) -> Result<(ChunkId, ChunkId), ContractError> {
    let radius = i128::from(light.dim_radius);
    let axis = |center: i64| {
        let minimum = (i128::from(center) - radius).max(i128::from(i64::MIN));
        let maximum = (i128::from(center) + radius).min(i128::from(i64::MAX));
        (
            minimum.div_euclid(i128::from(CHUNK_SIZE)),
            maximum.div_euclid(i128::from(CHUNK_SIZE)),
        )
    };
    let (q_min, q_max) = axis(light.position.column.q);
    let (r_min, r_max) = axis(light.position.column.r);
    let probes = (q_max - q_min + 1) * (r_max - r_min + 1);
    if probes > i128::from(MAX_LIGHT_INFLUENCE_CHUNK_PROBES) {
        return Err(ContractError::new(
            "light.influence",
            "per-light chunk expansion budget exceeded",
        ));
    }
    let narrow = |value: i128| {
        i64::try_from(value)
            .map_err(|error| ContractError::new("light.influence", error.to_string()))
    };
    Ok((
        ChunkId {
            q: narrow(q_min)?,
            r: narrow(r_min)?,
        },
        ChunkId {
            q: narrow(q_max)?,
            r: narrow(r_max)?,
        },
    ))
}

pub(crate) fn affects_chunk(light: &WorldLight, chunk: &ChunkPackage) -> bool {
    chunk.columns.iter().any(|column| {
        column
            .position
            .checked_distance(light.position.column)
            .is_ok_and(|distance| distance <= u64::from(light.dim_radius))
    })
}

pub(crate) fn project_lights(
    package: &WorldPackage,
) -> Result<BTreeMap<ChunkId, Vec<WorldLight>>, ContractError> {
    let mut projected: BTreeMap<ChunkId, Vec<WorldLight>> = BTreeMap::new();
    let mut identities = BTreeSet::new();
    for source in package.chunks.values() {
        if source.semantics.lights.len() > MAX_SEMANTIC_RECORDS {
            return Err(ContractError::new(
                "light.influence",
                "source chunk light count exceeds package bound",
            ));
        }
        for light in &source.semantics.lights {
            light.validate()?;
            if !identities.insert(&light.id) {
                return Err(ContractError::new(
                    "world.lights",
                    "duplicate root light identity",
                ));
            }
            let (minimum, maximum) = influence_bounds(light)?;
            for q in minimum.q..=maximum.q {
                for r in minimum.r..=maximum.r {
                    let coordinate = ChunkId { q, r };
                    let Some(target) = package.chunks.get(&coordinate) else {
                        continue;
                    };
                    if target.columns.len() > 256 {
                        return Err(ContractError::new(
                            "light.influence",
                            "target chunk has too many columns",
                        ));
                    }
                    if !affects_chunk(light, target) {
                        continue;
                    }
                    let influences = projected.entry(coordinate).or_default();
                    if influences.len() >= MAX_SEMANTIC_RECORDS {
                        return Err(ContractError::new(
                            "light.influence",
                            "target influence count exceeds package bound",
                        ));
                    }
                    influences.push(light.clone());
                }
            }
        }
    }
    for influences in projected.values_mut() {
        influences.sort_by(|a, b| a.id.cmp(&b.id));
    }
    Ok(projected)
}
