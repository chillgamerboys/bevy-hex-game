use std::{collections::BTreeMap, sync::Arc};

use crate::{
    BoundarySample, ChunkId, ContractError, FeatureSummary, MaterialSpec, RegionDescriptor,
    WorldHex, WorldManifest,
};

/// Validated immutable catalogue indexes shared by chunk producers and consumers.
///
/// Construction validates the complete manifest once. Footprint queries examine
/// only regions intersecting the queried storage chunk; canonical sorted ID tables
/// provide logarithmic material, region and feature lookup. This index contains no
/// terrain, filesystem state, residency policy or renderer types. Retain it in an
/// `Arc` across chunk loads, edits and mesh preparation instead of rebuilding it.
#[derive(Debug)]
pub struct ManifestIndex {
    manifest: Arc<WorldManifest>,
    regions_by_chunk: BTreeMap<ChunkId, Vec<usize>>,
    boundaries_by_column: BTreeMap<WorldHex, Vec<(usize, usize)>>,
}

impl ManifestIndex {
    /// Validate a complete catalogue and build its immutable spatial lookup once.
    pub fn new(manifest: Arc<WorldManifest>) -> Result<Self, ContractError> {
        manifest.validate()?;
        let regions_by_chunk = crate::validation::validate_chunk_catalogue(&manifest)?;
        let mut boundaries_by_column: BTreeMap<WorldHex, Vec<(usize, usize)>> = BTreeMap::new();
        for (boundary_index, boundary) in manifest.boundaries.iter().enumerate() {
            for (sample_index, sample) in boundary.samples.iter().enumerate() {
                for column in [sample.a, sample.b] {
                    boundaries_by_column
                        .entry(column)
                        .or_default()
                        .push((boundary_index, sample_index));
                }
            }
        }
        Ok(Self {
            manifest,
            regions_by_chunk,
            boundaries_by_column,
        })
    }

    /// Borrow the exact validated manifest, including its unchanged fingerprint.
    #[must_use]
    pub fn manifest(&self) -> &WorldManifest {
        &self.manifest
    }

    /// Whether the complete validated catalogue contains this storage coordinate.
    #[must_use]
    pub fn contains_chunk(&self, chunk: ChunkId) -> bool {
        self.regions_by_chunk.contains_key(&chunk)
    }

    /// Number of candidate region rows a footprint lookup in this chunk can inspect.
    ///
    /// This diagnostic measures locality directly without timing-dependent tests.
    #[must_use]
    pub fn candidate_region_count(&self, chunk: ChunkId) -> usize {
        self.regions_by_chunk.get(&chunk).map_or(0, Vec::len)
    }

    /// Query exact finite-world membership using only this chunk's candidate regions.
    pub fn contains(&self, column: WorldHex) -> Result<bool, ContractError> {
        if let Some(candidates) = self.regions_by_chunk.get(&column.chunk()) {
            for candidate in candidates {
                let region = self.manifest.regions.get(*candidate).ok_or_else(|| {
                    ContractError::new("manifest.index", "invalid private region reference")
                })?;
                if region.contains(column)? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Resolve a material from the validated canonical ID index.
    pub fn material(&self, id: &str) -> Result<&MaterialSpec, ContractError> {
        self.manifest.material(id)
    }

    /// Resolve a source region from the validated canonical ID index.
    pub fn region(&self, id: &str) -> Result<&RegionDescriptor, ContractError> {
        self.manifest.region(id)
    }

    /// Resolve a feature from the validated canonical ID index.
    #[must_use]
    pub fn feature(&self, id: &str) -> Option<&FeatureSummary> {
        self.manifest
            .features
            .binary_search_by(|entry| entry.id.as_str().cmp(id))
            .ok()
            .and_then(|index| self.manifest.features.get(index))
    }

    /// Exact authored boundary contracts touching one changed column, without a world scan.
    pub fn boundary_samples_at(&self, column: WorldHex) -> impl Iterator<Item = &BoundarySample> {
        self.boundaries_by_column
            .get(&column)
            .into_iter()
            .flatten()
            .filter_map(|(boundary, sample)| {
                self.manifest
                    .boundaries
                    .get(*boundary)
                    .and_then(|boundary| boundary.samples.get(*sample))
            })
    }

    pub(crate) fn validate_source_position(
        &self,
        region: &str,
        column: WorldHex,
    ) -> Result<(), ContractError> {
        if !self.region(region)?.contains(column)? {
            return Err(ContractError::new(
                "source.region",
                "root outside declared source region",
            ));
        }
        Ok(())
    }
}
