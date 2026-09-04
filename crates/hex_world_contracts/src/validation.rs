use crate::*;
use serde::{
    de::{Error as _, MapAccess, Visitor},
    Deserialize, Deserializer,
};
use std::collections::{BTreeMap, BTreeSet};
use std::{fmt, marker::PhantomData};

pub(crate) fn deserialize_unique_map<'de, D, K, V>(
    deserializer: D,
) -> Result<BTreeMap<K, V>, D::Error>
where
    D: Deserializer<'de>,
    K: Deserialize<'de> + Ord,
    V: Deserialize<'de>,
{
    struct UniqueMap<K, V>(PhantomData<(K, V)>);
    impl<'de, K: Deserialize<'de> + Ord, V: Deserialize<'de>> Visitor<'de> for UniqueMap<K, V> {
        type Value = BTreeMap<K, V>;
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a map with unique keys")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
            let mut result = BTreeMap::new();
            while let Some((key, value)) = access.next_entry()? {
                if result.insert(key, value).is_some() {
                    return Err(A::Error::custom("duplicate map key"));
                }
            }
            Ok(result)
        }
    }
    deserializer.deserialize_map(UniqueMap(PhantomData))
}

/// Maximum material runs in one bounded column or object column.
pub const MAX_RUNS_PER_COLUMN: usize = 4_096;
/// Maximum semantic records in one package (not a total-world limit).
pub const MAX_SEMANTIC_RECORDS: usize = 65_536;
/// Maximum exact voxel edits in one atomic operation.
pub const MAX_EDITS_PER_TRANSACTION: usize = 65_536;

fn reject(context: &str, message: &str) -> ContractError {
    ContractError::new(context, message)
}

fn name(value: &str, context: &str) -> Result<(), ContractError> {
    if value.is_empty()
        || value.len() > 512
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(reject(context, "expected a nonempty stable name without surrounding whitespace or control characters (maximum 512 bytes)"));
    }
    Ok(())
}

fn material_name(value: &str) -> Result<(), ContractError> {
    name(value, "material")?;
    if value.eq_ignore_ascii_case("air") {
        return Err(reject(
            "material",
            "air must be absent, never stored as a run",
        ));
    }
    Ok(())
}

fn ordered<T: Ord>(
    values: impl IntoIterator<Item = T>,
    context: &str,
) -> Result<(), ContractError> {
    let mut previous = None;
    for value in values {
        if previous.as_ref().is_some_and(|prior| prior >= &value) {
            return Err(reject(
                context,
                "duplicate identity or noncanonical ordering",
            ));
        }
        previous = Some(value);
    }
    Ok(())
}

fn schema(version: u32) -> Result<(), ContractError> {
    if version != SCHEMA_VERSION {
        return Err(reject("schema_version", "unsupported world schema version"));
    }
    Ok(())
}

/// Validate a portable package-relative path without consulting the filesystem.
///
/// Filesystem adapters must additionally reject symlink escapes while opening it.
pub fn validate_package_path(path: &str) -> Result<(), ContractError> {
    if path.is_empty()
        || path.len() > 4_096
        || path.starts_with('/')
        || path.contains(['\\', ':'])
        || path.chars().any(char::is_control)
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == ".." || part.trim() != part)
    {
        return Err(reject(
            "package.path",
            "unsafe or nonportable relative package path",
        ));
    }
    Ok(())
}

impl RegionDescriptor {
    /// Whether the exact integer disk contains this column.
    pub fn contains(&self, position: WorldHex) -> Result<bool, ContractError> {
        Ok(self.origin.checked_distance(position)? <= u64::from(self.radius))
    }
}

impl ColumnData {
    /// Borrow the exact stored material at a voxel level; absence means actual air.
    pub fn material_at(&self, level: i32) -> Option<&str> {
        self.runs
            .iter()
            .find(|run| run.bottom <= level && level < run.top)
            .map(|run| run.material.as_str())
    }

    /// Derive every exposed solid support, retaining each stack's exact air clearance.
    pub fn surfaces(&self, materials: &[MaterialSpec]) -> Result<Vec<Surface>, ContractError> {
        self.validate()?;
        let mut surfaces = Vec::new();
        let mut runs = self.runs.iter().peekable();
        while let Some(run) = runs.next() {
            let material = materials
                .iter()
                .find(|entry| entry.id == run.material)
                .ok_or_else(|| reject("column.material", "unknown material"))?;
            if !material.solid {
                continue;
            }
            let clearance = runs
                .peek()
                .map(|next| i64::from(next.bottom) - i64::from(run.top));
            if clearance == Some(0) {
                continue;
            }
            let headroom = clearance
                .map(u32::try_from)
                .transpose()
                .map_err(|error| ContractError::new("surface.headroom", error.to_string()))?;
            surfaces.push(Surface {
                position: VoxelPosition {
                    column: self.position,
                    level: run.top - 1,
                },
                material: run.material.clone(),
                headroom,
            });
        }
        Ok(surfaces)
    }
}

impl Validate for VoxelRun {
    fn validate(&self) -> Result<(), ContractError> {
        material_name(&self.material)?;
        if self.bottom >= self.top {
            return Err(reject("run", "bottom must be below exclusive top"));
        }
        Ok(())
    }
}

impl Validate for ColumnData {
    fn validate(&self) -> Result<(), ContractError> {
        if self.runs.len() > MAX_RUNS_PER_COLUMN {
            return Err(reject("column", "run allocation limit exceeded"));
        }
        let mut previous: Option<&VoxelRun> = None;
        for run in &self.runs {
            run.validate()?;
            if let Some(prior) = previous {
                if prior.top > run.bottom {
                    return Err(reject("column.runs", "overlapping or unsorted intervals"));
                }
                if prior.top == run.bottom && prior.material == run.material {
                    return Err(reject(
                        "column.runs",
                        "adjacent equal material intervals must be coalesced",
                    ));
                }
            }
            previous = Some(run);
        }
        Ok(())
    }
}

impl Seal for ColumnData {
    fn seal(&mut self) -> Result<(), ContractError> {
        let mut sorted = self.runs.clone();
        sorted.sort_by(|a, b| (a.bottom, a.top, &a.material).cmp(&(b.bottom, b.top, &b.material)));
        let mut result: Vec<VoxelRun> = Vec::with_capacity(sorted.len());
        for run in sorted {
            run.validate()?;
            if let Some(prior) = result.last_mut() {
                if prior.top > run.bottom {
                    return Err(reject("column.runs", "cannot seal overlapping intervals"));
                }
                if prior.top == run.bottom && prior.material == run.material {
                    prior.top = run.top;
                    continue;
                }
            }
            result.push(run);
        }
        let candidate = Self {
            position: self.position,
            runs: result,
        };
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
}

impl Validate for RegionDescriptor {
    fn validate(&self) -> Result<(), ContractError> {
        name(&self.id, "region.id")?;
        let radius = i64::from(self.radius);
        for offset in [
            WorldHex::new(radius, 0),
            WorldHex::new(-radius, 0),
            WorldHex::new(0, radius),
            WorldHex::new(0, -radius),
        ] {
            self.origin.checked_add(offset)?;
        }
        Ok(())
    }
}

impl Validate for FeatureSummary {
    fn validate(&self) -> Result<(), ContractError> {
        name(&self.id, "feature.id")?;
        name(&self.region_id, "feature.region_id")?;
        name(&self.kind, "feature.kind")?;
        if let Some(asset) = &self.asset {
            name(asset, "feature.asset")?;
        }
        Ok(())
    }
}

impl Validate for ChunkSemantics {
    fn validate(&self) -> Result<(), ContractError> {
        if self.occupancy.len() > 256 {
            return Err(reject(
                "semantics.occupancy",
                "more object columns than a chunk can contain",
            ));
        }
        ordered(
            self.occupancy.iter().map(|column| column.position),
            "semantics.occupancy",
        )?;
        for column in &self.occupancy {
            column.validate()?;
            if column.runs.is_empty() {
                return Err(reject(
                    "semantics.occupancy",
                    "empty object columns must be omitted",
                ));
            }
        }
        let count = self
            .liquids
            .len()
            .checked_add(self.anchors.len())
            .and_then(|value| value.checked_add(self.interiors.len()))
            .and_then(|value| value.checked_add(self.lights.len()))
            .and_then(|value| value.checked_add(self.objects.len()))
            .ok_or_else(|| reject("semantics", "record count overflow"))?;
        if count > MAX_SEMANTIC_RECORDS {
            return Err(reject("semantics", "per-package record limit exceeded"));
        }
        ordered(
            self.liquids
                .iter()
                .map(|row| (row.column, row.bottom, row.top, &row.body_id)),
            "liquids",
        )?;
        ordered(self.anchors.iter().map(|row| &row.id), "anchors")?;
        ordered(
            self.interiors
                .iter()
                .map(|row| (&row.id, row.column, row.floor_level)),
            "interiors",
        )?;
        ordered(self.lights.iter().map(|row| &row.id), "lights")?;
        ordered(self.objects.iter().map(|row| &row.id), "objects")?;
        for liquid in &self.liquids {
            name(&liquid.body_id, "liquid.body_id")?;
            if liquid.bottom >= liquid.top {
                return Err(reject("liquid", "empty or inverted liquid interval"));
            }
            ordered(liquid.downstream.iter(), "liquid.downstream")?;
            if liquid.kind == LiquidKind::Standing && !liquid.downstream.is_empty() {
                return Err(reject(
                    "liquid.downstream",
                    "standing interval cannot carry directed outflow",
                ));
            }
            for downstream in &liquid.downstream {
                if downstream.level > liquid.top - 1 {
                    return Err(reject("liquid.downstream", "uphill liquid edge"));
                }
                if downstream.column == liquid.column && downstream.level == liquid.top - 1 {
                    return Err(reject("liquid.downstream", "self-loop"));
                }
                if liquid.column.checked_distance(downstream.column)? > 1 {
                    return Err(reject(
                        "liquid.downstream",
                        "downstream interval must be in this or an adjacent column",
                    ));
                }
            }
        }
        for anchor in &self.anchors {
            name(&anchor.id, "anchor.id")?;
            name(&anchor.region_id, "anchor.region_id")?;
        }
        for interior in &self.interiors {
            name(&interior.id, "interior.id")?;
            name(&interior.light_domain, "interior.light_domain")?;
            if i64::from(interior.roof_bottom) - i64::from(interior.floor_level) < 2
                || interior.roof_bottom >= interior.roof_top
            {
                return Err(reject(
                    "interior",
                    "floor, clear interior, and exclusive roof interval must be ordered",
                ));
            }
        }
        for light in &self.lights {
            name(&light.id, "light.id")?;
            if let Some(domain) = &light.domain {
                name(domain, "light.domain")?;
            }
            if light.bright_radius > light.dim_radius {
                return Err(reject("light", "bright radius exceeds dim radius"));
            }
        }
        for object in &self.objects {
            name(&object.id, "object.id")?;
            name(&object.region_id, "object.region_id")?;
            name(&object.asset, "object.asset")?;
            if object.rotation >= 6 {
                return Err(reject(
                    "object.rotation",
                    "expected six-way rotation in 0..6",
                ));
            }
            if object.occupancy.len() > MAX_SEMANTIC_RECORDS {
                return Err(reject(
                    "object.occupancy",
                    "per-object column limit exceeded",
                ));
            }
            ordered(
                object.occupancy.iter().map(|column| column.position),
                "object.occupancy",
            )?;
            for column in &object.occupancy {
                column.validate()?;
            }
        }
        Ok(())
    }
}

fn canonicalize_semantics(semantics: &mut ChunkSemantics) -> Result<(), ContractError> {
    for column in &mut semantics.occupancy {
        column.seal()?;
    }
    semantics.occupancy.sort_by_key(|column| column.position);
    semantics.liquids.sort_by(|a, b| {
        (a.column, a.bottom, a.top, &a.body_id).cmp(&(b.column, b.bottom, b.top, &b.body_id))
    });
    for liquid in &mut semantics.liquids {
        liquid.downstream.sort();
    }
    semantics.anchors.sort_by(|a, b| a.id.cmp(&b.id));
    semantics
        .interiors
        .sort_by(|a, b| (&a.id, a.column, a.floor_level).cmp(&(&b.id, b.column, b.floor_level)));
    semantics.lights.sort_by(|a, b| a.id.cmp(&b.id));
    semantics.objects.sort_by(|a, b| a.id.cmp(&b.id));
    for object in &mut semantics.objects {
        for column in &mut object.occupancy {
            column.seal()?;
        }
        object.occupancy.sort_by_key(|column| column.position);
    }
    semantics.validate()
}

impl Validate for ChunkPackage {
    fn validate(&self) -> Result<(), ContractError> {
        schema(self.schema_version)?;
        name(&self.world_id, "chunk.world_id")?;
        self.coordinate.origin()?;
        if self.columns.len() > 256 {
            return Err(reject(
                "chunk.columns",
                "more columns than a 16 by 16 chunk",
            ));
        }
        ordered(
            self.columns.iter().map(|column| column.position),
            "chunk.columns",
        )?;
        for column in &self.columns {
            column.validate()?;
            if column.position.chunk() != self.coordinate {
                return Err(reject("chunk.columns", "column belongs to another chunk"));
            }
        }
        ordered(
            self.features.iter().map(|feature| &feature.id),
            "chunk.features",
        )?;
        for feature in &self.features {
            feature.validate()?;
            if feature.anchor.column.chunk() != self.coordinate {
                return Err(reject(
                    "feature.anchor",
                    "feature must be stored in its root chunk",
                ));
            }
        }
        self.semantics.validate()?;
        let positions = self
            .semantics
            .liquids
            .iter()
            .map(|row| row.column)
            .chain(self.semantics.interiors.iter().map(|row| row.column))
            .chain(self.semantics.anchors.iter().map(|row| row.position.column))
            .chain(self.semantics.lights.iter().map(|row| row.position.column))
            .chain(self.semantics.objects.iter().map(|row| row.origin.column));
        for position in positions.chain(
            self.semantics
                .occupancy
                .iter()
                .map(|column| column.position),
        ) {
            if position.chunk() != self.coordinate
                || self
                    .columns
                    .binary_search_by_key(&position, |column| column.position)
                    .is_err()
            {
                return Err(reject(
                    "chunk.semantics",
                    "semantic root/member requires its resident owner column",
                ));
            }
        }
        if self.fingerprint != fingerprint(self)? {
            return Err(reject("chunk.fingerprint", "content fingerprint mismatch"));
        }
        Ok(())
    }
}

impl CanonicalFingerprint for ChunkPackage {
    fn canonical_fingerprint(&self) -> Result<u64, ContractError> {
        let mut value = self.clone();
        value.fingerprint = 0;
        hash_serializable(&value)
    }
}

impl Seal for ChunkPackage {
    fn seal(&mut self) -> Result<(), ContractError> {
        let mut candidate = self.clone();
        for column in &mut candidate.columns {
            column.seal()?;
        }
        candidate.columns.sort_by_key(|column| column.position);
        candidate.features.sort_by(|a, b| a.id.cmp(&b.id));
        canonicalize_semantics(&mut candidate.semantics)?;
        candidate.fingerprint = fingerprint(&candidate)?;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
}

impl Validate for WorldManifest {
    fn validate(&self) -> Result<(), ContractError> {
        schema(self.schema_version)?;
        name(&self.world_id, "world_id")?;
        name(&self.compiler_version, "compiler_version")?;
        ordered(self.materials.iter().map(|entry| &entry.id), "materials")?;
        for material in &self.materials {
            material_name(&material.id)?;
        }
        ordered(self.regions.iter().map(|entry| &entry.id), "regions")?;
        if self.regions.is_empty() {
            return Err(reject("regions", "world must declare at least one region"));
        }
        for region in &self.regions {
            region.validate()?;
        }
        ordered(self.chunks.iter().map(|entry| entry.coordinate), "chunks")?;
        let mut paths = BTreeSet::new();
        for chunk in &self.chunks {
            chunk.coordinate.origin()?;
            validate_package_path(&chunk.path)?;
            if !paths.insert(&chunk.path) {
                return Err(reject(
                    "chunks.path",
                    "two chunks reference the same package path",
                ));
            }
        }
        ordered(self.boundaries.iter().map(|entry| &entry.id), "boundaries")?;
        let mut boundary_pairs = BTreeSet::new();
        for boundary in &self.boundaries {
            name(&boundary.id, "boundary.id")?;
            if boundary.region_a == boundary.region_b {
                return Err(reject("boundary", "boundary must join different regions"));
            }
            let a = self.region(&boundary.region_a)?;
            let b = self.region(&boundary.region_b)?;
            let pair = if a.id < b.id {
                (&a.id, &b.id)
            } else {
                (&b.id, &a.id)
            };
            if !boundary_pairs.insert(pair) {
                return Err(reject(
                    "boundary",
                    "region pair has multiple boundary authorities",
                ));
            }
            ordered(
                boundary.samples.iter().map(|sample| (sample.a, sample.b)),
                "boundary.samples",
            )?;
            if boundary.samples.is_empty() {
                return Err(reject(
                    "boundary.samples",
                    "boundary needs at least one sample",
                ));
            }
            for sample in &boundary.samples {
                if sample.a.checked_distance(sample.b)? != 1
                    || !a.contains(sample.a)?
                    || !b.contains(sample.b)?
                {
                    return Err(reject(
                        "boundary.samples",
                        "sample must join adjacent columns of the named regions",
                    ));
                }
                if sample.water_level == Some(sample.ground_level) {
                    return Err(reject(
                        "boundary.water_level",
                        "one voxel cannot be both solid ground and liquid",
                    ));
                }
            }
        }
        ordered(self.summary.iter().map(|cell| cell.position), "summary")?;
        for cell in &self.summary {
            self.material(&cell.material)?;
            if !self.region(&cell.region_id)?.contains(cell.position)? {
                return Err(reject("summary", "sample outside its source region"));
            }
        }
        ordered(self.features.iter().map(|entry| &entry.id), "features")?;
        for feature in &self.features {
            feature.validate()?;
            self.validate_source_position(&feature.region_id, feature.anchor.column)?;
        }
        if self.fingerprint != fingerprint(self)? {
            return Err(reject(
                "manifest.fingerprint",
                "content fingerprint mismatch",
            ));
        }
        Ok(())
    }
}

impl WorldManifest {
    /// Look up a stable material, rejecting unresolved source references.
    pub fn material(&self, id: &str) -> Result<&MaterialSpec, ContractError> {
        self.materials
            .iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| ContractError::new("material", format!("unknown material {id}")))
    }
    /// Look up a stable source region.
    pub fn region(&self, id: &str) -> Result<&RegionDescriptor, ContractError> {
        self.regions
            .iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| ContractError::new("region", format!("unknown region {id}")))
    }
    /// Test the union of declared finite world footprints without loading terrain.
    pub fn contains(&self, column: WorldHex) -> Result<bool, ContractError> {
        for region in &self.regions {
            if region.contains(column)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn validate_source_position(
        &self,
        region: &str,
        column: WorldHex,
    ) -> Result<(), ContractError> {
        if !self.region(region)?.contains(column)? {
            return Err(reject(
                "source.region",
                "root outside declared source region",
            ));
        }
        Ok(())
    }
}

impl CanonicalFingerprint for WorldManifest {
    fn canonical_fingerprint(&self) -> Result<u64, ContractError> {
        let mut value = self.clone();
        value.fingerprint = 0;
        hash_serializable(&value)
    }
}

impl Seal for WorldManifest {
    fn seal(&mut self) -> Result<(), ContractError> {
        let mut candidate = self.clone();
        candidate.materials.sort_by(|a, b| a.id.cmp(&b.id));
        candidate.regions.sort_by(|a, b| a.id.cmp(&b.id));
        candidate.chunks.sort_by_key(|entry| entry.coordinate);
        candidate.boundaries.sort_by(|a, b| a.id.cmp(&b.id));
        for boundary in &mut candidate.boundaries {
            boundary.samples.sort_by_key(|sample| (sample.a, sample.b));
        }
        candidate.summary.sort_by_key(|cell| cell.position);
        candidate.features.sort_by(|a, b| a.id.cmp(&b.id));
        candidate.fingerprint = fingerprint(&candidate)?;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
}

fn column_at(columns: &[ColumnData], position: WorldHex) -> Result<&ColumnData, ContractError> {
    columns
        .binary_search_by_key(&position, |column| column.position)
        .ok()
        .and_then(|index| columns.get(index))
        .ok_or_else(|| {
            ContractError::new(
                "column",
                format!("missing column ({}, {})", position.q, position.r),
            )
        })
}

fn solid_at(
    column: &ColumnData,
    level: i32,
    manifest: &WorldManifest,
) -> Result<bool, ContractError> {
    column
        .material_at(level)
        .map(|id| manifest.material(id).map(|material| material.solid))
        .transpose()
        .map(|solid| solid.unwrap_or(false))
}

fn supported(
    column: &ColumnData,
    position: VoxelPosition,
    manifest: &WorldManifest,
) -> Result<(), ContractError> {
    if !solid_at(column, position.level, manifest)? {
        return Err(reject("support", "required surface has no solid support"));
    }
    for offset in [1, 2] {
        let level = position
            .level
            .checked_add(offset)
            .ok_or_else(|| reject("support", "body clearance overflows level range"))?;
        if column.material_at(level).is_some() {
            return Err(reject(
                "support",
                "required walker has insufficient air clearance",
            ));
        }
    }
    Ok(())
}

fn interval_policy(
    column: &ColumnData,
    bottom: i32,
    top: i32,
    solid: bool,
    manifest: &WorldManifest,
) -> Result<(), ContractError> {
    let mut cursor = bottom;
    for run in &column.runs {
        if run.top <= cursor {
            continue;
        }
        if run.bottom > cursor || manifest.material(&run.material)?.solid != solid {
            return Err(reject(
                "semantic.interval",
                "terrain does not supply the declared material interval",
            ));
        }
        cursor = run.top.min(top);
        if cursor == top {
            return Ok(());
        }
    }
    Err(reject(
        "semantic.interval",
        "terrain interval ends before declared semantic interval",
    ))
}

impl ChunkPackage {
    /// Validate a sealed chunk's local content against a manifest's source and materials.
    ///
    /// Includes exact region-footprint coverage and local semantic consequences.
    /// This intentionally does not compare the original descriptor fingerprint, so
    /// a runtime can validate an edited revision before publishing it. Cross-chunk
    /// liquid/object/light dependencies are additionally checked by [`WorldPackage`].
    pub fn validate_against_manifest(&self, manifest: &WorldManifest) -> Result<(), ContractError> {
        self.validate()?;
        if self.world_id != manifest.world_id {
            return Err(reject("chunk.world_id", "chunk belongs to another world"));
        }
        let origin = self.coordinate.origin()?;
        for q in 0..CHUNK_SIZE {
            for r in 0..CHUNK_SIZE {
                let position = origin.checked_add(WorldHex::new(q, r))?;
                let present = self
                    .columns
                    .binary_search_by_key(&position, |column| column.position)
                    .is_ok();
                if manifest.contains(position)? != present {
                    return Err(reject(
                        "chunk.columns",
                        "columns must exactly cover world footprint within the chunk",
                    ));
                }
            }
        }
        for column in self.columns.iter().chain(&self.semantics.occupancy) {
            for run in &column.runs {
                manifest.material(&run.material)?;
            }
        }
        for feature in &self.features {
            manifest.validate_source_position(&feature.region_id, feature.anchor.column)?;
            if !manifest
                .features
                .iter()
                .any(|candidate| candidate == feature)
            {
                return Err(reject(
                    "chunk.feature",
                    "feature differs from world registry",
                ));
            }
        }
        for liquid in &self.semantics.liquids {
            interval_policy(
                column_at(&self.columns, liquid.column)?,
                liquid.bottom,
                liquid.top,
                false,
                manifest,
            )?;
        }
        for anchor in &self.semantics.anchors {
            manifest.validate_source_position(&anchor.region_id, anchor.position.column)?;
            if anchor.role != AnchorRole::Observation {
                supported(
                    column_at(&self.columns, anchor.position.column)?,
                    anchor.position,
                    manifest,
                )?;
            }
        }
        for interior in &self.semantics.interiors {
            let column = column_at(&self.columns, interior.column)?;
            if !solid_at(column, interior.floor_level, manifest)? {
                return Err(reject("interior.floor", "floor lacks solid support"));
            }
            let clear_start = i64::from(interior.floor_level) + 1;
            if column
                .runs
                .iter()
                .any(|run| i64::from(run.top) > clear_start && run.bottom < interior.roof_bottom)
            {
                return Err(reject(
                    "interior.clearance",
                    "terrain occupies the declared interior air interval",
                ));
            }
            interval_policy(
                column,
                interior.roof_bottom,
                interior.roof_top,
                true,
                manifest,
            )?;
        }
        for object in &self.semantics.objects {
            manifest.validate_source_position(&object.region_id, object.origin.column)?;
            for column in &object.occupancy {
                if !manifest.contains(column.position)? {
                    return Err(reject(
                        "object.occupancy",
                        "occupancy leaves declared world",
                    ));
                }
                for run in &column.runs {
                    manifest.material(&run.material)?;
                }
            }
        }
        Ok(())
    }
}

impl Validate for WorldPackage {
    fn validate(&self) -> Result<(), ContractError> {
        self.manifest.validate()?;
        if self.chunks.len() != self.manifest.chunks.len() {
            return Err(reject("world.chunks", "package and descriptor sets differ"));
        }
        let mut features = BTreeMap::new();
        let mut anchor_ids = BTreeSet::new();
        let mut object_ids = BTreeSet::new();
        let mut light_ids = BTreeSet::new();
        let mut columns = BTreeMap::new();
        let mut liquids = BTreeMap::new();
        let mut domains = BTreeMap::<&str, Vec<&InteriorSpan>>::new();
        for descriptor in &self.manifest.chunks {
            let chunk = self
                .chunks
                .get(&descriptor.coordinate)
                .ok_or_else(|| reject("world.chunks", "missing indexed chunk"))?;
            if chunk.coordinate != descriptor.coordinate
                || chunk.fingerprint != descriptor.fingerprint
            {
                return Err(reject(
                    "world.chunks",
                    "chunk key, address, or descriptor fingerprint mismatch",
                ));
            }
            chunk.validate_against_manifest(&self.manifest)?;
            for column in &chunk.columns {
                columns.insert(column.position, column);
            }
            for feature in &chunk.features {
                if features.insert(&feature.id, feature).is_some() {
                    return Err(reject("world.features", "duplicate world feature ID"));
                }
            }
            for anchor in &chunk.semantics.anchors {
                if !anchor_ids.insert(&anchor.id) {
                    return Err(reject("world.anchors", "duplicate world anchor ID"));
                }
            }
            for object in &chunk.semantics.objects {
                if !object_ids.insert(&object.id) {
                    return Err(reject("world.objects", "duplicate world object ID"));
                }
            }
            for light in &chunk.semantics.lights {
                if !light_ids.insert(&light.id) {
                    return Err(reject("world.lights", "duplicate world light ID"));
                }
            }
            for interior in &chunk.semantics.interiors {
                domains
                    .entry(&interior.light_domain)
                    .or_default()
                    .push(interior);
            }
            for liquid in &chunk.semantics.liquids {
                let position = VoxelPosition {
                    column: liquid.column,
                    level: liquid.top - 1,
                };
                if liquids.insert(position, liquid).is_some() {
                    return Err(reject("world.liquids", "duplicate exact liquid surface"));
                }
            }
        }
        if features.len() != self.manifest.features.len() {
            return Err(reject(
                "world.features",
                "world registry has unpublished features",
            ));
        }
        // Count existing columns instead of allocating each declared disk. A huge
        // missing region fails by arithmetic, without a total-world size cap.
        for region in &self.manifest.regions {
            let mut actual = 0_u128;
            for position in columns.keys() {
                if region.contains(*position)? {
                    actual += 1;
                }
            }
            let radius = u128::from(region.radius);
            let expected = 1 + 3 * radius * (radius + 1);
            if actual != expected {
                return Err(reject(
                    "world.regions",
                    "region footprint has missing chunks or columns",
                ));
            }
        }
        for chunk in self.chunks.values() {
            for object in &chunk.semantics.objects {
                for column in &object.occupancy {
                    if !columns.contains_key(&column.position) {
                        return Err(reject("world.object", "missing dependent occupancy chunk"));
                    }
                }
            }
            for light in &chunk.semantics.lights {
                if let Some(domain) = &light.domain {
                    let matching = domains.get(domain.as_str()).is_some_and(|spans| {
                        spans.iter().any(|span| {
                            span.column == light.position.column
                                && span.floor_level < light.position.level
                                && light.position.level < span.roof_bottom
                        })
                    });
                    if !matching {
                        return Err(reject(
                            "world.light",
                            "interior light has no matching domain at its source",
                        ));
                    }
                }
            }
        }
        for boundary in &self.manifest.boundaries {
            for sample in &boundary.samples {
                for position in [sample.a, sample.b] {
                    let column = columns
                        .get(&position)
                        .ok_or_else(|| reject("boundary", "missing sample column"))?;
                    if !solid_at(column, sample.ground_level, &self.manifest)?
                        || sample.ground_level.checked_add(1).is_some_and(|level| {
                            column.material_at(level).is_some_and(|id| {
                                self.manifest
                                    .materials
                                    .iter()
                                    .any(|material| material.id == id && material.solid)
                            })
                        })
                    {
                        return Err(reject(
                            "boundary.ground",
                            "ground datum does not match exposed solid terrain",
                        ));
                    }
                    if sample.required_access {
                        supported(
                            column,
                            VoxelPosition {
                                column: position,
                                level: sample.ground_level,
                            },
                            &self.manifest,
                        )?;
                    }
                    if let Some(level) = sample.water_level {
                        if !liquids.contains_key(&VoxelPosition {
                            column: position,
                            level,
                        }) {
                            return Err(reject(
                                "boundary.water",
                                "water datum lacks an exact liquid surface",
                            ));
                        }
                    }
                }
            }
        }
        let expected_occupancy = object_occupancy(self)?;
        let actual_occupancy: BTreeMap<_, _> = self
            .chunks
            .values()
            .flat_map(|chunk| &chunk.semantics.occupancy)
            .map(|column| (column.position, column.clone()))
            .collect();
        if expected_occupancy != actual_occupancy {
            return Err(reject(
                "world.occupancy",
                "resident object projection differs from complete root-object occupancy",
            ));
        }
        validate_liquid_graph(&liquids)?;
        Ok(())
    }
}

fn validate_liquid_graph(
    liquids: &BTreeMap<VoxelPosition, &LiquidColumn>,
) -> Result<(), ContractError> {
    let mut incoming: BTreeMap<VoxelPosition, usize> =
        liquids.keys().map(|position| (*position, 0)).collect();
    for liquid in liquids.values() {
        for target in &liquid.downstream {
            let degree = incoming.get_mut(target).ok_or_else(|| {
                reject("liquid.downstream", "missing exact downstream interval top")
            })?;
            *degree = degree
                .checked_add(1)
                .ok_or_else(|| reject("liquid.downstream", "degree overflow"))?;
        }
    }
    let mut ready: BTreeSet<_> = incoming
        .iter()
        .filter_map(|(position, degree)| (*degree == 0).then_some(*position))
        .collect();
    let mut visited = 0;
    while let Some(position) = ready.pop_first() {
        visited += 1;
        let liquid = liquids
            .get(&position)
            .ok_or_else(|| reject("liquid.graph", "inconsistent graph index"))?;
        for target in &liquid.downstream {
            let degree = incoming
                .get_mut(target)
                .ok_or_else(|| reject("liquid.graph", "missing indexed target"))?;
            *degree = degree
                .checked_sub(1)
                .ok_or_else(|| reject("liquid.graph", "invalid incoming degree"))?;
            if *degree == 0 {
                ready.insert(*target);
            }
        }
    }
    if visited != liquids.len() {
        return Err(reject("liquid.graph", "directed liquid cycle"));
    }
    Ok(())
}

impl CanonicalFingerprint for WorldPackage {
    fn canonical_fingerprint(&self) -> Result<u64, ContractError> {
        hash_serializable(self)
    }
}

impl Seal for WorldPackage {
    fn seal(&mut self) -> Result<(), ContractError> {
        let mut candidate = self.clone();
        let occupancy = object_occupancy(&candidate)?;
        for chunk in candidate.chunks.values_mut() {
            chunk.semantics.occupancy.clear();
        }
        for column in occupancy.into_values() {
            let chunk = candidate
                .chunks
                .get_mut(&column.position.chunk())
                .ok_or_else(|| reject("world.occupancy", "missing dependent object chunk"))?;
            chunk.semantics.occupancy.push(column);
        }
        for chunk in candidate.chunks.values_mut() {
            chunk.seal()?;
        }
        for descriptor in &mut candidate.manifest.chunks {
            let chunk = candidate
                .chunks
                .get(&descriptor.coordinate)
                .ok_or_else(|| reject("world.chunks", "missing indexed chunk"))?;
            descriptor.fingerprint = chunk.fingerprint;
        }
        candidate.manifest.seal()?;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
}

fn object_occupancy(
    package: &WorldPackage,
) -> Result<BTreeMap<WorldHex, ColumnData>, ContractError> {
    let mut grouped: BTreeMap<WorldHex, Vec<VoxelRun>> = BTreeMap::new();
    for object in package
        .chunks
        .values()
        .flat_map(|chunk| &chunk.semantics.objects)
    {
        for column in &object.occupancy {
            let mut checked = column.clone();
            checked.seal()?;
            grouped
                .entry(column.position)
                .or_default()
                .extend(checked.runs);
        }
    }
    let mut output = BTreeMap::new();
    for (position, mut runs) in grouped {
        runs.sort_by(|a, b| (a.bottom, a.top, &a.material).cmp(&(b.bottom, b.top, &b.material)));
        let mut union: Vec<VoxelRun> = Vec::new();
        for run in runs {
            if let Some(prior) = union.last_mut() {
                if prior.top > run.bottom && prior.material != run.material {
                    return Err(reject(
                        "world.occupancy",
                        "overlapping objects disagree on exact voxel material",
                    ));
                }
                if prior.top >= run.bottom && prior.material == run.material {
                    prior.top = prior.top.max(run.top);
                    continue;
                }
            }
            union.push(run);
        }
        if !union.is_empty() {
            output.insert(
                position,
                ColumnData {
                    position,
                    runs: union,
                },
            );
        }
    }
    Ok(output)
}

impl Validate for ResidencyRequest {
    fn validate(&self) -> Result<(), ContractError> {
        name(&self.id, "residency.id")?;
        if self.retention_radius < self.radius {
            return Err(reject(
                "residency",
                "retention radius is smaller than activation radius",
            ));
        }
        RegionDescriptor {
            id: self.id.clone(),
            origin: self.center,
            radius: self.retention_radius,
            source_fingerprint: 0,
        }
        .validate()
    }
}

impl Validate for WorldEditTransaction {
    fn validate(&self) -> Result<(), ContractError> {
        name(&self.id, "transaction.id")?;
        if self.edits.is_empty() || self.edits.len() > MAX_EDITS_PER_TRANSACTION {
            return Err(reject(
                "transaction.edits",
                "expected a bounded nonempty edit operation",
            ));
        }
        ordered(
            self.edits.iter().map(|edit| edit.position),
            "transaction.edits",
        )?;
        let affected: BTreeSet<_> = self
            .edits
            .iter()
            .map(|edit| edit.position.column.chunk())
            .collect();
        if affected != self.expected_revisions.keys().copied().collect() {
            return Err(reject(
                "transaction.revisions",
                "revision expectations must exactly cover affected chunks",
            ));
        }
        for edit in &self.edits {
            if let Some(material) = &edit.material {
                material_name(material)?;
            }
            if edit.material.is_some() && edit.position.level == i32::MAX {
                return Err(reject(
                    "transaction.level",
                    "exclusive run top cannot represent this assignment",
                ));
            }
        }
        Ok(())
    }
}

impl Validate for WorldChange {
    fn validate(&self) -> Result<(), ContractError> {
        name(&self.transaction_id, "change.transaction_id")?;
        if self.changed_columns.is_empty() || self.changed_columns.len() > MAX_EDITS_PER_TRANSACTION
        {
            return Err(reject(
                "change",
                "expected bounded nonempty changed columns",
            ));
        }
        ordered(self.changed_columns.iter(), "change.columns")?;
        let affected: BTreeSet<_> = self
            .changed_columns
            .iter()
            .map(|position| position.chunk())
            .collect();
        if affected != self.revisions.keys().copied().collect() {
            return Err(reject(
                "change.revisions",
                "revisions must exactly cover changed columns",
            ));
        }
        Ok(())
    }
}

macro_rules! inherent_validation {
    ($($ty:ty),+ $(,)?) => { $(impl $ty {
        /// Validate without normalizing or mutating this value.
        pub fn validate(&self) -> Result<(), ContractError> { <Self as Validate>::validate(self) }
    })+ };
}
inherent_validation!(
    VoxelRun,
    ColumnData,
    RegionDescriptor,
    FeatureSummary,
    ChunkSemantics,
    ChunkPackage,
    WorldManifest,
    WorldPackage,
    ResidencyRequest,
    WorldEditTransaction,
    WorldChange
);

macro_rules! inherent_sealing {
    ($($ty:ty),+ $(,)?) => { $(impl $ty {
        /// Canonicalize and seal trusted producer data, retaining the old value on error.
        pub fn seal(&mut self) -> Result<(), ContractError> { <Self as Seal>::seal(self) }
    })+ };
}
inherent_sealing!(ColumnData, ChunkPackage, WorldManifest, WorldPackage);
