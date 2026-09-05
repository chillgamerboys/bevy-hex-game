use std::{collections::BTreeSet, sync::Arc};

use bevy::prelude::*;
use hex_assets::{
    HexObjectRotation, LocalVoxelCoord, ObjectAssetId, ObjectBlueprint, RuntimeArtCatalog,
};
use hex_core::TilePos;
use hex_world_contracts::{ChunkId, ChunkSemantics, ObjectInstance, VoxelPosition, WorldHex};

use super::ObjectPresentationError;

/// Operational bounds for one local presentation window, never world admission caps.
#[derive(Clone, Copy, Debug)]
pub struct ObjectPresentationLimits {
    /// Maximum simultaneously published roots, counting each fragment separately.
    pub max_resident_objects: usize,
    /// Maximum distinct baked whole assets or fragment variants with live users.
    pub max_asset_types: usize,
    /// Maximum exact voxels in one admitted object.
    pub max_voxels_per_object: usize,
    /// Conservative pre-bake vertex budget for one blueprint.
    pub max_vertices_per_asset: usize,
    /// Maximum total vertices retained across shared baked assets.
    pub max_cached_vertices: usize,
    /// Maximum absolute local axial/cube coordinate of every occupied voxel.
    pub max_local_hex: i32,
    /// Maximum absolute local occupied voxel level.
    pub max_local_level: i32,
    /// Maximum absolute height of occupied voxel boundaries in rendering units.
    pub max_render_height: f32,
}

impl Default for ObjectPresentationLimits {
    fn default() -> Self {
        Self {
            max_resident_objects: 2048,
            max_asset_types: 64,
            max_voxels_per_object: 8192,
            max_vertices_per_asset: 1_000_000,
            max_cached_vertices: 4_000_000,
            max_local_hex: 1024,
            max_local_level: 4096,
            max_render_height: 4096.0,
        }
    }
}

impl ObjectPresentationLimits {
    pub(super) fn validate(self) -> Result<(), ObjectPresentationError> {
        if self.max_resident_objects == 0
            || self.max_asset_types == 0
            || self.max_voxels_per_object == 0
            || self.max_vertices_per_asset == 0
            || self.max_cached_vertices == 0
            || !(1..=4096).contains(&self.max_local_hex)
            || !(1..=1_048_576).contains(&self.max_local_level)
            || !self.max_render_height.is_finite()
            || !(1.0..=4096.0).contains(&self.max_render_height)
        {
            return Err(ObjectPresentationError(
                "invalid object presentation limits".into(),
            ));
        }
        Ok(())
    }
}

pub(super) struct BakedPart {
    pub key: crate::ChunkKey,
    pub mesh: Mesh,
    pub material: StandardMaterial,
    pub surface_mode: hex_assets::VoxelSurfaceMode,
    pub casts_shadows: bool,
}

pub(super) struct BakedAsset {
    pub parts: Vec<BakedPart>,
    pub vertices: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum AssetKey {
    Whole(ObjectAssetId),
    Fragment {
        asset: ObjectAssetId,
        rotation: u8,
        chunk_offset: WorldHex,
    },
}

/// Validated disposable art product, bound to one presenter's origin generation.
///
/// The exact record is copied from authority; it is not reconstructed from art.
/// Callers must bound queued products and recheck residency/revision before publish.
pub struct PreparedObject {
    pub(super) object: Arc<ObjectInstance>,
    pub(super) asset: ObjectAssetId,
    pub(super) cache_key: AssetKey,
    pub(super) clip: Option<ChunkId>,
    pub(super) revision: u64,
    pub(super) fingerprint: u64,
    pub(super) local_origin: TilePos,
    pub(super) transform: Transform,
    pub(super) generation: u64,
    pub(super) context: Arc<()>,
    pub(super) baked: Arc<BakedAsset>,
    pub(super) voxels: usize,
    pub(super) source_voxels: usize,
}

impl PreparedObject {
    /// Stable global object identity.
    pub fn id(&self) -> &str {
        &self.object.id
    }

    /// Exact source record, including the mask the application must reconcile.
    pub fn object(&self) -> &ObjectInstance {
        &self.object
    }

    /// Revision supplied by the authoritative resident owner.
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Deterministic checksum of the entire exact source object record.
    pub const fn fingerprint(&self) -> u64 {
        self.fingerprint
    }

    /// Number of exact occupied voxels validated against the blueprint.
    pub const fn voxels(&self) -> usize {
        self.voxels
    }

    /// Owning global chunk for a fragment, or none for a whole-object product.
    pub const fn clip(&self) -> Option<ChunkId> {
        self.clip
    }

    /// Complete exact source voxel count validated before any fragment selection.
    pub const fn source_voxels(&self) -> usize {
        self.source_voxels
    }
}

pub(super) fn select_fragment(
    object: &ObjectInstance,
    blueprint: &ObjectBlueprint,
    asset: ObjectAssetId,
    clip: ChunkId,
) -> Result<(AssetKey, BTreeSet<LocalVoxelCoord>), ObjectPresentationError> {
    let mut selected = BTreeSet::new();
    for placement in &blueprint.placements {
        let offset = WorldHex::new(
            i64::from(placement.position.q) - i64::from(blueprint.origin.q),
            i64::from(placement.position.r) - i64::from(blueprint.origin.r),
        )
        .rotate_60(object.rotation)?;
        if object.origin.column.checked_add(offset)?.chunk() == clip {
            selected.insert(placement.position);
        }
    }
    if selected.is_empty() {
        return Err(ObjectPresentationError(
            "object has no authored voxels in requested fragment".into(),
        ));
    }
    let chunk_origin = clip.origin()?;
    let chunk_offset = WorldHex::new(
        i64::try_from(i128::from(chunk_origin.q) - i128::from(object.origin.column.q))
            .map_err(|error| ObjectPresentationError(error.to_string()))?,
        i64::try_from(i128::from(chunk_origin.r) - i128::from(object.origin.column.r))
            .map_err(|error| ObjectPresentationError(error.to_string()))?,
    );
    Ok((
        AssetKey::Fragment {
            asset,
            rotation: object.rotation,
            chunk_offset,
        },
        selected,
    ))
}

pub(super) fn exact_footprint(
    object: &ObjectInstance,
    blueprint: &ObjectBlueprint,
    limits: ObjectPresentationLimits,
) -> Result<usize, ObjectPresentationError> {
    ChunkSemantics {
        objects: vec![object.clone()],
        ..default()
    }
    .validate()?;
    if blueprint.placements.len() > limits.max_voxels_per_object {
        return Err(ObjectPresentationError(
            "max_voxels_per_object exceeded".into(),
        ));
    }
    let mut expected = BTreeSet::new();
    for placement in &blueprint.placements {
        let offset = WorldHex::new(
            i64::from(placement.position.q) - i64::from(blueprint.origin.q),
            i64::from(placement.position.r) - i64::from(blueprint.origin.r),
        )
        .rotate_60(object.rotation)?;
        let level = i64::from(object.origin.level) + i64::from(placement.position.level)
            - i64::from(blueprint.origin.level);
        expected.insert(VoxelPosition {
            column: object.origin.column.checked_add(offset)?,
            level: i32::try_from(level)
                .map_err(|error| ObjectPresentationError(error.to_string()))?,
        });
    }
    let mut actual = BTreeSet::new();
    let mut count = 0usize;
    for column in &object.occupancy {
        for run in &column.runs {
            let height = usize::try_from(i64::from(run.top) - i64::from(run.bottom))
                .map_err(|error| ObjectPresentationError(error.to_string()))?;
            count = count
                .checked_add(height)
                .ok_or_else(|| ObjectPresentationError("object voxel count overflow".into()))?;
            if count > limits.max_voxels_per_object {
                return Err(ObjectPresentationError(
                    "max_voxels_per_object exceeded".into(),
                ));
            }
            actual.extend((run.bottom..run.top).map(|level| VoxelPosition {
                column: column.position,
                level,
            }));
        }
    }
    if actual != expected {
        return Err(ObjectPresentationError(
            "stock blueprint footprint differs from exact rotated object occupancy".into(),
        ));
    }
    Ok(actual.len())
}

pub(super) fn local_transform(
    object: &ObjectInstance,
    asset: &ObjectAssetId,
    local_origin: TilePos,
    level_height: f32,
    limits: ObjectPresentationLimits,
) -> Result<Transform, ObjectPresentationError> {
    let local_q = i128::from(local_origin.coord.x());
    let local_r = i128::from(local_origin.coord.y());
    check_local(
        local_q,
        local_r,
        i128::from(local_origin.level),
        level_height,
        limits,
    )?;
    for column in &object.occupancy {
        let q = local_q + i128::from(column.position.q) - i128::from(object.origin.column.q);
        let r = local_r + i128::from(column.position.r) - i128::from(object.origin.column.r);
        for run in &column.runs {
            for level in [run.bottom, run.top - 1] {
                let level = i128::from(local_origin.level) + i128::from(level)
                    - i128::from(object.origin.level);
                check_local(q, r, level, level_height, limits)?;
            }
        }
    }
    let rotation = HexObjectRotation::new(object.rotation)
        .map_err(|error| ObjectPresentationError(error.to_string()))?;
    let local =
        hex_assets::ObjectInstance::new(asset.clone(), local_origin, level_height, rotation)
            .map_err(|error| ObjectPresentationError(error.to_string()))?;
    Ok(crate::object_root_transform(&local))
}

fn check_local(
    q: i128,
    r: i128,
    level: i128,
    level_height: f32,
    limits: ObjectPresentationLimits,
) -> Result<(), ObjectPresentationError> {
    if q.abs().max(r.abs()).max((q + r).abs()) > i128::from(limits.max_local_hex) {
        return Err(ObjectPresentationError("max_local_hex exceeded".into()));
    }
    if level.abs() > i128::from(limits.max_local_level) {
        return Err(ObjectPresentationError("max_local_level exceeded".into()));
    }
    let bounded_level =
        i32::try_from(level).map_err(|error| ObjectPresentationError(error.to_string()))?;
    if f64::from(bounded_level)
        .abs()
        .max((f64::from(bounded_level) + 1.0).abs())
        * f64::from(level_height)
        > f64::from(limits.max_render_height)
    {
        return Err(ObjectPresentationError("max_render_height exceeded".into()));
    }
    Ok(())
}

pub(super) fn bake(
    source: &Mesh,
    blueprint: &ObjectBlueprint,
    catalog: &RuntimeArtCatalog,
    limits: ObjectPresentationLimits,
    selected: Option<&BTreeSet<LocalVoxelCoord>>,
) -> Result<Arc<BakedAsset>, ObjectPresentationError> {
    if source
        .count_vertices()
        .saturating_mul(selected.map_or(blueprint.placements.len(), BTreeSet::len))
        > limits.max_vertices_per_asset
    {
        return Err(ObjectPresentationError(
            "max_vertices_per_asset exceeded before baking".into(),
        ));
    }
    let raw = if selected.is_some() {
        crate::bake_blueprint_selected(
            source,
            blueprint,
            catalog,
            hex_core::ReviewEdgeTreatment::Current,
            selected,
        )
    } else {
        crate::bake_blueprint(source, blueprint, catalog)
    }
    .map_err(ObjectPresentationError)?;
    let mut parts = Vec::with_capacity(raw.len());
    let mut vertices = 0usize;
    for (key, mesh) in raw {
        let style = catalog.style(&key.style).ok_or_else(|| {
            ObjectPresentationError("validated object references missing style".into())
        })?;
        vertices += mesh.count_vertices();
        parts.push(BakedPart {
            key,
            mesh,
            material: crate::material_for(style, hex_core::ReviewMaterialTreatment::Current),
            surface_mode: style.authored().surface_mode(),
            casts_shadows: matches!(
                style.authored().surface_mode(),
                hex_assets::VoxelSurfaceMode::Opaque | hex_assets::VoxelSurfaceMode::Cutout
            ),
        });
    }
    Ok(Arc::new(BakedAsset { parts, vertices }))
}
