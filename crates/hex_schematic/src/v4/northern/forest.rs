//! Bounded, residency-independent authored forest presentation data.
//!
//! These records describe disposable distant geometry, never occupancy, collision,
//! damage or disclosure authority. Exact local runs make conservative cut masks
//! possible without loading or pinning any semantic source chunk.
mod surface;
use hex_world_contracts::{
    ChunkId, ColumnData, ContractError, ObjectInstance, VoxelPosition, VoxelRun, WorldHex,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
pub use surface::{forest_mesh, ForestMesh};

/// Maximum different shared authored shapes, including snowy palette variants.
pub const MAX_FOREST_SHAPES: usize = 18;
/// Maximum ordinary tree instances; the World Tree has a separate representation.
pub const MAX_FOREST_INSTANCES: usize = 780;
/// Exact compact source intervals across the entire shared shape dictionary.
pub const MAX_FOREST_SOURCE_RUNS: usize = 8_000;
/// Maximum serialized forest section size, independent of the terrain overview.
pub const MAX_FOREST_BYTES: usize = 2 * 1024 * 1024;
/// Maximum opaque vertices in one uncut shared tree mesh.
pub const MAX_FOREST_SHAPE_VERTICES: usize = 6_144;
/// Maximum simultaneously submitted ordinary forest vertices, including roots.
pub const MAX_FOREST_VERTICES: usize = 1_600_000;
/// Canonical coarse foliage radius in units of the exact hex lattice.
pub const FOREST_FOLIAGE_RADIUS: i64 = 3;
/// Canonical coarse foliage band in exact logical levels.
pub const FOREST_FOLIAGE_LEVELS: i32 = 8;

/// Shared shapes and complete authored placements, independent of residency.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForestOverview {
    /// Presentation recipe version; currently one.
    pub version: u32,
    /// Shared shapes, sorted by stable shape identity.
    pub shapes: Vec<ForestShape>,
    /// Placements sorted by exact semantic object identity.
    pub instances: Vec<ForestInstance>,
}
/// One exact local source shape, shared by all matching instances.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForestShape {
    /// Stable shape identity, including any authored palette variant.
    pub id: String,
    /// Renderer asset identity from the semantic object catalog.
    pub asset: String,
    /// Exact compact columns relative to the local crown base and axial root.
    pub columns: Vec<ColumnData>,
}
/// A placement fully reconstructible without a generator or resident source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForestInstance {
    /// Exact globally unique semantic object identity.
    pub id: String,
    /// Index of the shared shape in this companion.
    pub shape: usize,
    /// Original semantic root, which may be below the crown's placement base.
    pub origin: VoxelPosition,
    /// Counterclockwise axial rotation in zero through five.
    pub rotation: u8,
    /// Actual world logical level of the shape's local zero.
    pub base_level: i32,
    /// Exact root extensions below local zero, in the shape's local coordinates.
    pub roots: Vec<ColumnData>,
    /// Original complete publication footprint, including the semantic root.
    pub footprint: BTreeSet<ChunkId>,
}
fn invalid(message: &str) -> ContractError {
    ContractError::new("overview/forest", message)
}
fn legal_material(material: &str) -> bool {
    matches!(
        material,
        "timber" | "foliage" | "foliage_dark" | "foliage_light" | "snow"
    )
}
/// Rotate a bounded local axial column by a six-way instance transform.
pub fn forest_rotate(mut point: WorldHex, rotation: u8) -> Result<WorldHex, ContractError> {
    if rotation >= 6 {
        return Err(invalid("invalid six-way forest rotation"));
    }
    for _ in 0..rotation {
        point = WorldHex::new(
            point
                .r
                .checked_neg()
                .ok_or_else(|| invalid("rotation overflow"))?,
            point
                .q
                .checked_add(point.r)
                .ok_or_else(|| invalid("rotation overflow"))?,
        );
    }
    Ok(point)
}
/// The deterministic nearest coarse hex; local source columns are bounded to radius16.
pub(super) fn forest_foliage_column(point: WorldHex) -> WorldHex {
    let q = point.q.div_euclid(FOREST_FOLIAGE_RADIUS);
    let r = point.r.div_euclid(FOREST_FOLIAGE_RADIUS);
    let mut best = (i64::MAX, q, r);
    for a in q..=q + 1 {
        for b in r..=r + 1 {
            let x = point.q - a * FOREST_FOLIAGE_RADIUS;
            let z = point.r - b * FOREST_FOLIAGE_RADIUS;
            best = best.min((x * x + x * z + z * z, a, b));
        }
    }
    WorldHex::new(best.1, best.2)
}
impl ForestOverview {
    /// Validate fixed shape/instance limits, exact footprints, and mesh budgets.
    pub fn validate(&self) -> Result<(), ContractError> {
        self.validate_in_bounds(900, [0, 1600])
    }
    /// Validate within a package's admitted finite envelope, keeping all mesh budgets.
    pub fn validate_in_bounds(&self, radius: u32, levels: [i32; 2]) -> Result<(), ContractError> {
        let [minimum, maximum] = levels;
        if radius == 0
            || radius > u32::from(i16::MAX.unsigned_abs())
            || minimum < 0
            || maximum > i32::from(i16::MAX)
            || minimum >= maximum
        {
            return Err(invalid("invalid forest finite envelope"));
        }
        if self.version != 1
            || self.shapes.len() > MAX_FOREST_SHAPES
            || self.instances.len() > MAX_FOREST_INSTANCES
            || !self
                .shapes
                .windows(2)
                .all(|w| w.first().zip(w.get(1)).is_some_and(|(a, b)| a.id < b.id))
            || !self
                .instances
                .windows(2)
                .all(|w| w.first().zip(w.get(1)).is_some_and(|(a, b)| a.id < b.id))
        {
            return Err(invalid("forest version, order or object budget is invalid"));
        }
        let runs = self
            .shapes
            .iter()
            .flat_map(|s| &s.columns)
            .map(|c| c.runs.len())
            .sum::<usize>();
        if runs > MAX_FOREST_SOURCE_RUNS {
            return Err(invalid("shared forest source-run budget exceeded"));
        }
        let mut vertices = Vec::new();
        for shape in &self.shapes {
            if shape.id.is_empty()
                || shape.id.len() > 160
                || shape.asset.len() > 128
                || !shape.asset.starts_with("plant/grand-")
                || shape.columns.len() > 512
                || shape.columns.is_empty()
            {
                return Err(invalid("invalid shared forest shape"));
            }
            validate_columns(&shape.columns, false)?;
            vertices.push(
                forest_mesh(&shape.columns, &BTreeSet::new())?
                    .positions
                    .len(),
            );
        }
        let mut total = 0_usize;
        for instance in &self.instances {
            let shape = self
                .shapes
                .get(instance.shape)
                .ok_or_else(|| invalid("missing forest shape"))?;
            if !instance.id.starts_with("grand/tree/")
                || instance.id.len() > 128
                || instance.rotation >= 6
                || instance
                    .origin
                    .column
                    .checked_distance(WorldHex::new(0, 0))?
                    > u64::from(radius)
                || !(minimum..=maximum).contains(&instance.origin.level)
                || !(minimum..=maximum).contains(&instance.base_level)
                || instance.roots.len() > 64
                || instance.footprint.len() > 16
                || instance.footprint.is_empty()
            {
                return Err(invalid("invalid forest placement or footprint budget"));
            }
            validate_columns(&instance.roots, true)?;
            let columns = instance.columns(shape)?;
            for column in &columns {
                if column.position.checked_distance(WorldHex::new(0, 0))? > u64::from(radius)
                    || column
                        .runs
                        .iter()
                        .any(|run| run.bottom < minimum || run.top > maximum)
                {
                    return Err(invalid("forest world source exceeds finite bounds"));
                }
            }
            let footprint: BTreeSet<_> = columns
                .iter()
                .map(|c| c.position.chunk())
                .chain(std::iter::once(instance.origin.column.chunk()))
                .collect();
            if instance.footprint != footprint {
                return Err(invalid(
                    "forest footprint differs from exact authored source",
                ));
            }
            total = total
                .checked_add(
                    *vertices
                        .get(instance.shape)
                        .ok_or_else(|| invalid("shape budget absent"))?,
                )
                .ok_or_else(|| invalid("forest vertex overflow"))?;
            total = total
                .checked_add(
                    forest_mesh(&instance.roots, &BTreeSet::new())?
                        .positions
                        .len(),
                )
                .ok_or_else(|| invalid("forest root vertex overflow"))?;
        }
        if total > MAX_FOREST_VERTICES {
            return Err(invalid(&format!(
                "whole forest vertex budget exceeded: {total} > {MAX_FOREST_VERTICES}"
            )));
        }
        let bytes = ron::to_string(self).map_err(|error| invalid(&error.to_string()))?;
        if bytes.len() > MAX_FOREST_BYTES {
            return Err(invalid("forest companion exceeds2MiB"));
        }
        Ok(())
    }
    /// Bind this presentation product to exact public object catalog facts.
    pub fn validate_catalog(
        &self,
        features: &[hex_world_contracts::FeatureSummary],
    ) -> Result<(), ContractError> {
        self.validate_catalog_in_bounds(features, 900, [0, 1600])
    }
    /// Bind exact public object facts within the same admitted finite envelope.
    pub fn validate_catalog_in_bounds(
        &self,
        features: &[hex_world_contracts::FeatureSummary],
        radius: u32,
        levels: [i32; 2],
    ) -> Result<(), ContractError> {
        self.validate_in_bounds(radius, levels)?;
        let catalog: BTreeMap<_, _> = features
            .iter()
            .filter(|f| f.id.starts_with("grand/tree/"))
            .map(|f| (f.id.as_str(), f))
            .collect();
        if catalog.len() != self.instances.len() {
            return Err(invalid("forest catalog is incomplete"));
        }
        for instance in &self.instances {
            let shape = self
                .shapes
                .get(instance.shape)
                .ok_or_else(|| invalid("shape missing"))?;
            let entry = catalog
                .get(instance.id.as_str())
                .ok_or_else(|| invalid("forest object missing from catalog"))?;
            if entry.anchor != instance.origin
                || entry.asset.as_deref() != Some(shape.asset.as_str())
                || entry.kind != "tree"
            {
                return Err(invalid("forest source identity differs from catalog"));
            }
        }
        Ok(())
    }
}
fn validate_columns(columns: &[ColumnData], roots: bool) -> Result<(), ContractError> {
    if !columns.windows(2).all(|w| {
        w.first()
            .zip(w.get(1))
            .is_some_and(|(a, b)| a.position < b.position)
    }) {
        return Err(invalid("forest columns are not canonical"));
    }
    for column in columns {
        column.validate()?;
        if column.runs.is_empty() || column.position.checked_distance(WorldHex::new(0, 0))? > 16 {
            return Err(invalid("forest local column outside authored bounds"));
        }
        for run in &column.runs {
            if !legal_material(&run.material)
                || (roots && (run.material != "timber" || run.bottom < -8 || run.top > 0))
                || (!roots && (run.bottom < 0 || run.top > 114))
            {
                return Err(invalid("forest local interval outside authored bounds"));
            }
        }
    }
    Ok(())
}
impl ForestInstance {
    /// Exact local shape plus authored ground-contact extensions.
    pub fn local_columns(&self, shape: &ForestShape) -> Result<Vec<ColumnData>, ContractError> {
        let mut columns: BTreeMap<_, Vec<VoxelRun>> = shape
            .columns
            .iter()
            .map(|c| (c.position, c.runs.clone()))
            .collect();
        for root in &self.roots {
            columns
                .entry(root.position)
                .or_default()
                .extend(root.runs.iter().cloned());
        }
        let mut result = Vec::new();
        for (position, mut runs) in columns {
            runs.sort_by_key(|r| r.bottom);
            let mut merged: Vec<VoxelRun> = Vec::new();
            for run in runs {
                if let Some(last) = merged
                    .last_mut()
                    .filter(|last| last.top == run.bottom && last.material == run.material)
                {
                    last.top = run.top;
                } else {
                    merged.push(run);
                }
            }
            let column = ColumnData {
                position,
                runs: merged,
            };
            column.validate()?;
            result.push(column);
        }
        Ok(result)
    }
    /// Reconstruct exact world-coordinate source columns for validation and masks.
    pub fn columns(&self, shape: &ForestShape) -> Result<Vec<ColumnData>, ContractError> {
        let mut result = self.local_columns(shape)?;
        for column in &mut result {
            let local = forest_rotate(column.position, self.rotation)?;
            column.position = WorldHex::new(
                self.origin
                    .column
                    .q
                    .checked_add(local.q)
                    .ok_or_else(|| invalid("forest q overflow"))?,
                self.origin
                    .column
                    .r
                    .checked_add(local.r)
                    .ok_or_else(|| invalid("forest r overflow"))?,
            );
            for run in &mut column.runs {
                run.bottom = run
                    .bottom
                    .checked_add(self.base_level)
                    .ok_or_else(|| invalid("forest level overflow"))?;
                run.top = run
                    .top
                    .checked_add(self.base_level)
                    .ok_or_else(|| invalid("forest level overflow"))?;
            }
        }
        result.sort_by_key(|c| c.position);
        Ok(result)
    }
    /// Exact local address of a world removal; callers retain only authored cells.
    pub fn local_position(&self, position: VoxelPosition) -> Result<VoxelPosition, ContractError> {
        if self.rotation >= 6 {
            return Err(invalid("invalid forest rotation"));
        }
        let relative = WorldHex::new(
            position
                .column
                .q
                .checked_sub(self.origin.column.q)
                .ok_or_else(|| invalid("forest q overflow"))?,
            position
                .column
                .r
                .checked_sub(self.origin.column.r)
                .ok_or_else(|| invalid("forest r overflow"))?,
        );
        Ok(VoxelPosition {
            column: forest_rotate(relative, (6 - self.rotation) % 6)?,
            level: position
                .level
                .checked_sub(self.base_level)
                .ok_or_else(|| invalid("forest level overflow"))?,
        })
    }
    /// Check the complete projection against its authoritative authored object.
    pub fn matches_object(
        &self,
        shape: &ForestShape,
        object: &ObjectInstance,
    ) -> Result<bool, ContractError> {
        Ok(self.id == object.id
            && self.origin == object.origin
            && shape.asset == object.asset
            && self.columns(shape)? == object.occupancy)
    }
}
