use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use bevy::{light::NotShadowCaster, mesh::VertexAttributeValues, prelude::*};
use hex_assets::{ObjectAssetId, RuntimeArtCatalog, VoxelStyleId, VoxelSurfaceMode};
use hex_core::TilePos;
use hex_world_contracts::{ChunkId, ObjectInstance};

use super::prepare::{
    bake, exact_footprint, local_transform, select_fragment, AssetKey, BakedAsset,
};
use super::{ObjectPresentationError, ObjectPresentationLimits, PreparedObject};

/// Stable global identity on a disposable stock-art root.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct ResidentObject {
    /// World-unique source identity.
    pub id: String,
    /// Authoritative owner revision represented by this root.
    pub revision: u64,
    /// Checksum of the exact source record, including occupancy materials.
    pub fingerprint: u64,
    /// Owning visible chunk, or none for a whole-object root.
    pub clip: Option<ChunkId>,
}

/// Typed mesh-child identity for application-owned exact object picking.
///
/// All generated parts are initially `Pickable::IGNORE`. The application enables
/// picking only when matching proxy suppression and authoritative lookup are ready.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct ResidentObjectPart {
    /// Stable global identity, not an asset ID or disposable entity.
    pub id: String,
    /// Source revision, rechecked before resolving a hit.
    pub revision: u64,
    /// Full exact source checksum, rechecked before resolving a hit.
    pub fingerprint: u64,
    /// Only this chunk's authored voxels occur in a fragment mesh.
    pub clip: Option<ChunkId>,
}

/// A current publication receipt; art does not replace authoritative occupancy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectReceipt {
    /// Stable global object identity.
    pub id: String,
    /// Authored renderer asset ID.
    pub asset: String,
    /// Owning visible chunk, or none for a whole-object publication.
    pub clip: Option<ChunkId>,
    /// Source owner revision.
    pub revision: u64,
    /// Checksum of the exact source object record.
    pub fingerprint: u64,
    /// Accepted immutable art catalogue checksum.
    pub catalog_fingerprint: u64,
    /// Disposable root, stable through render-origin rebases.
    pub root: Entity,
    /// Bounded render-local origin supplied by the application.
    pub local_origin: TilePos,
    /// Number of exact voxels rendered by this whole object or selected fragment.
    pub voxels: usize,
    /// Number of exact source voxels validated before selecting a fragment.
    pub source_voxels: usize,
    /// Shared style/canopy mesh parts instantiated by this root.
    pub meshes: usize,
    /// Vertices used by this asset; shared among its resident instances.
    pub vertices: usize,
    /// Whether the authored asset contains Blend material requiring OIT.
    pub has_blend: bool,
}

struct PublishedObject {
    receipt: ObjectReceipt,
    object: Arc<ObjectInstance>,
    asset: ObjectAssetId,
    cache_key: AssetKey,
}

type ResidentKey = (String, Option<ChunkId>);

struct SharedAsset {
    baked: Arc<BakedAsset>,
    parts: Vec<crate::CachedChunk>,
    users: BTreeSet<ResidentKey>,
}

struct SharedMaterial {
    handle: Handle<StandardMaterial>,
    parts: usize,
}

/// Explicit owner of stock-art roots and bounded shared render assets.
///
/// Use one Bevy world for this presenter's entire lifecycle. The catalogue and
/// source primitive are immutable snapshots; replace the presenter to accept new
/// art. Preparation performs exact validation before any world mutation and reuses
/// the baked product of a live asset. Multiple queued first-use preparations may
/// duplicate temporary CPU geometry, so callers must bound that queue.
///
/// Removal/clear do not preserve historical authoritative revisions. Callers must
/// discard no-longer-desired jobs and recheck current host revisions before publish.
#[derive(Resource)]
pub struct ResidentObjectPresenter {
    catalog: Arc<RuntimeArtCatalog>,
    source: Mesh,
    level_height: f32,
    limits: ObjectPresentationLimits,
    generation: u64,
    context: Arc<()>,
    resident: BTreeMap<ResidentKey, PublishedObject>,
    assets: BTreeMap<AssetKey, SharedAsset>,
    materials: BTreeMap<VoxelStyleId, SharedMaterial>,
}

impl ResidentObjectPresenter {
    /// Create an opt-in presenter from accepted art and a loaded unit-height hex mesh.
    pub fn new(
        catalog: Arc<RuntimeArtCatalog>,
        source_hex: Mesh,
        level_height: f32,
    ) -> Result<Self, ObjectPresentationError> {
        Self::with_limits(
            catalog,
            source_hex,
            level_height,
            ObjectPresentationLimits::default(),
        )
    }

    /// Configure local presentation budgets independently from total world size.
    pub fn with_limits(
        catalog: Arc<RuntimeArtCatalog>,
        source_hex: Mesh,
        level_height: f32,
        limits: ObjectPresentationLimits,
    ) -> Result<Self, ObjectPresentationError> {
        limits.validate()?;
        if !level_height.is_finite() || !(0.01..=16.0).contains(&level_height) {
            return Err(ObjectPresentationError(
                "level height must be finite and within 0.01..=16".into(),
            ));
        }
        validate_source(&source_hex, limits)?;
        Ok(Self {
            catalog,
            source: source_hex,
            level_height,
            limits,
            generation: 0,
            context: Arc::new(()),
            resident: BTreeMap::new(),
            assets: BTreeMap::new(),
            materials: BTreeMap::new(),
        })
    }

    /// Validate exact rotated occupancy and a caller-checked local origin, then bake.
    ///
    /// Global-to-local origin subtraction belongs to the application and must occur
    /// in integers before floats. This method additionally bounds the entire exact
    /// footprint using widened integer offsets from that supplied local origin.
    pub fn prepare(
        &self,
        object: &ObjectInstance,
        revision: u64,
        local_origin: TilePos,
    ) -> Result<PreparedObject, ObjectPresentationError> {
        self.prepare_clipped(object, revision, local_origin, None)
    }

    /// Prepare only authored voxels owned by one global chunk, after full validation.
    ///
    /// Neighbor face culling still uses the complete blueprint. This keeps adjacent
    /// fragments consistent while allowing the application to replace each terrain
    /// chunk's exact proxy mask and art fragment atomically in one frame.
    pub fn prepare_fragment(
        &self,
        object: &ObjectInstance,
        revision: u64,
        local_origin: TilePos,
        clip: ChunkId,
    ) -> Result<PreparedObject, ObjectPresentationError> {
        self.prepare_clipped(object, revision, local_origin, Some(clip))
    }

    fn prepare_clipped(
        &self,
        object: &ObjectInstance,
        revision: u64,
        local_origin: TilePos,
        clip: Option<ChunkId>,
    ) -> Result<PreparedObject, ObjectPresentationError> {
        let asset = ObjectAssetId::new(&object.asset)
            .map_err(|error| ObjectPresentationError(error.to_string()))?;
        let blueprint = self.catalog.object(&asset).ok_or_else(|| {
            ObjectPresentationError(format!("stock catalogue has no asset '{}'", object.asset))
        })?;
        let source_voxels = exact_footprint(object, blueprint, self.limits)?;
        let transform =
            local_transform(object, &asset, local_origin, self.level_height, self.limits)?;
        let (cache_key, selected) = if let Some(clip) = clip {
            let (key, selected) = select_fragment(object, blueprint, asset.clone(), clip)?;
            (key, Some(selected))
        } else {
            (AssetKey::Whole(asset.clone()), None)
        };
        let voxels = selected.as_ref().map_or(source_voxels, BTreeSet::len);
        let baked = if let Some(cached) = self.assets.get(&cache_key) {
            cached.baked.clone()
        } else {
            bake(
                &self.source,
                blueprint,
                &self.catalog,
                self.limits,
                selected.as_ref(),
            )?
        };
        Ok(PreparedObject {
            object: Arc::new(object.clone()),
            asset,
            cache_key,
            clip,
            revision,
            fingerprint: hex_world_contracts::hash_serializable(object)?,
            local_origin,
            transform,
            generation: self.generation,
            context: self.context.clone(),
            baked,
            voxels,
            source_voxels,
        })
    }

    /// Current resident roots in stable global ID order.
    pub fn receipts(&self) -> impl ExactSizeIterator<Item = &ObjectReceipt> {
        self.resident.values().map(|object| &object.receipt)
    }

    /// Distinct shared asset meshes currently retained by live object users.
    pub fn cached_asset_count(&self) -> usize {
        self.assets.len()
    }

    /// Distinct authored styles currently retained by shared asset parts.
    pub fn cached_material_count(&self) -> usize {
        self.materials.len()
    }

    /// Exact source occupancy retained for the application's typed picking/mask lookup.
    pub fn object(&self, id: &str) -> Option<&ObjectInstance> {
        self.resident
            .range((id.to_owned(), None)..)
            .next()
            .filter(|((candidate, _), _)| candidate == id)
            .map(|(_, object)| object.object.as_ref())
    }

    /// Publish one complete product after validation and budget checks.
    ///
    /// Call only in the same atomic application operation as corresponding proxy
    /// masks/picking. A failed admission leaves all old roots and assets intact.
    pub fn publish(
        &mut self,
        world: &mut World,
        prepared: PreparedObject,
    ) -> Result<ObjectReceipt, ObjectPresentationError> {
        if !Arc::ptr_eq(&self.context, &prepared.context) || self.generation != prepared.generation
        {
            return Err(ObjectPresentationError(
                "prepared object has a stale or foreign origin context".into(),
            ));
        }
        let key = (prepared.object.id.clone(), prepared.clip);
        // Whole and clipped products for one ID cannot overlap. Fragment revisions
        // may advance independently only when their exact source record is unchanged;
        // a changed object must first retire its old fragment set atomically.
        for ((id, clip), existing) in self.resident.range((prepared.object.id.clone(), None)..) {
            if id != &prepared.object.id {
                break;
            }
            if *clip != prepared.clip && (clip.is_none() || prepared.clip.is_none()) {
                return Err(ObjectPresentationError(
                    "whole-object and fragment publication cannot overlap".into(),
                ));
            }
            if *clip != prepared.clip
                && (existing.receipt.fingerprint != prepared.fingerprint
                    || existing.receipt.local_origin != prepared.local_origin)
            {
                return Err(ObjectPresentationError("resident fragments disagree on full source or local origin; retire them before changing the object".into()));
            }
        }
        let current = self.resident.get(&key);
        if let Some(current) = current {
            if prepared.revision < current.receipt.revision {
                return Err(ObjectPresentationError(
                    "object revision is older than current presentation".into(),
                ));
            }
            if prepared.revision == current.receipt.revision {
                if prepared.fingerprint != current.receipt.fingerprint
                    || prepared.local_origin != current.receipt.local_origin
                {
                    return Err(ObjectPresentationError("same object revision has conflicting source or local origin; use rebase for origin changes".into()));
                }
                if world.get_entity(current.receipt.root).is_err() {
                    return Err(ObjectPresentationError(
                        "resident root was removed outside its owner".into(),
                    ));
                }
                return Ok(current.receipt.clone());
            }
        } else if self.resident.len() >= self.limits.max_resident_objects {
            return Err(ObjectPresentationError(
                "max_resident_objects exceeded".into(),
            ));
        }
        let drops_old_asset = current
            .filter(|current| current.cache_key != prepared.cache_key)
            .and_then(|current| self.assets.get(&current.cache_key))
            .filter(|asset| asset.users.len() == 1);
        if !self.assets.contains_key(&prepared.cache_key) {
            let remaining_assets = self.assets.len() - usize::from(drops_old_asset.is_some());
            if remaining_assets >= self.limits.max_asset_types {
                return Err(ObjectPresentationError("max_asset_types exceeded".into()));
            }
            let current_vertices: usize =
                self.assets.values().map(|asset| asset.baked.vertices).sum();
            let released_vertices = drops_old_asset.map_or(0, |asset| asset.baked.vertices);
            if current_vertices
                .saturating_sub(released_vertices)
                .saturating_add(prepared.baked.vertices)
                > self.limits.max_cached_vertices
            {
                return Err(ObjectPresentationError(
                    "max_cached_vertices exceeded".into(),
                ));
            }
        }

        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        // Preserve a same-asset cache through replacement, even for its last user.
        if let Some(current) = self.resident.remove(&key) {
            world.despawn(current.receipt.root);
            if current.cache_key != prepared.cache_key {
                self.release_asset(world, &current.cache_key, &key);
            }
        }
        if !self.assets.contains_key(&prepared.cache_key) {
            self.allocate_asset(world, prepared.cache_key.clone(), prepared.baked.clone());
        }
        let cached = self.assets.get_mut(&prepared.cache_key).ok_or_else(|| {
            ObjectPresentationError("object asset allocation lost its cache entry".into())
        })?;
        cached.users.insert(key.clone());
        let root = world
            .spawn((
                ResidentObject {
                    id: prepared.object.id.clone(),
                    revision: prepared.revision,
                    fingerprint: prepared.fingerprint,
                    clip: prepared.clip,
                },
                prepared.transform,
                Visibility::default(),
                Name::new(format!("Resident object {}", prepared.object.id)),
            ))
            .id();
        let children = spawn_parts(world, root, &prepared, &cached.parts);
        world.entity_mut(root).add_children(&children);
        let receipt = ObjectReceipt {
            id: prepared.object.id.clone(),
            asset: prepared.object.asset.clone(),
            clip: prepared.clip,
            revision: prepared.revision,
            fingerprint: prepared.fingerprint,
            catalog_fingerprint: self.catalog.combined_fingerprint(),
            root,
            local_origin: prepared.local_origin,
            voxels: prepared.voxels,
            source_voxels: prepared.source_voxels,
            meshes: cached.parts.len(),
            vertices: cached.baked.vertices,
            has_blend: cached
                .parts
                .iter()
                .any(|part| part.surface_mode == VoxelSurfaceMode::Translucent),
        };
        self.resident.insert(
            key,
            PublishedObject {
                receipt: receipt.clone(),
                object: prepared.object,
                asset: prepared.asset,
                cache_key: prepared.cache_key,
            },
        );
        Ok(receipt)
    }

    /// Remove a whole-object root; fragment roots use [`Self::remove_fragment`].
    pub fn remove(&mut self, world: &mut World, id: &str) -> Option<ObjectReceipt> {
        self.remove_key(world, &(id.to_owned(), None))
    }

    /// Remove only one chunk's fragment and release its final shared asset users.
    pub fn remove_fragment(
        &mut self,
        world: &mut World,
        id: &str,
        clip: ChunkId,
    ) -> Option<ObjectReceipt> {
        self.remove_key(world, &(id.to_owned(), Some(clip)))
    }

    fn remove_key(&mut self, world: &mut World, key: &ResidentKey) -> Option<ObjectReceipt> {
        let object = self.resident.remove(key)?;
        world.despawn(object.receipt.root);
        self.release_asset(world, &object.cache_key, key);
        Some(object.receipt)
    }

    /// Atomically validate and move the complete current local resident set.
    ///
    /// The mapping must contain every distinct resident ID exactly once and no extra
    /// IDs. All fragments of one object share that object's checked local origin.
    /// Global records, authoritative revisions, roots and shared assets remain
    /// unchanged. Any failure preserves all old transforms and prepared generations.
    /// Unload distant roots before rebasing to another bounded view neighborhood.
    pub fn rebase(
        &mut self,
        world: &mut World,
        placements: &BTreeMap<String, TilePos>,
    ) -> Result<Vec<ObjectReceipt>, ObjectPresentationError> {
        let ids: BTreeSet<_> = self.resident.keys().map(|(id, _)| id).collect();
        if !ids.iter().copied().eq(placements.keys()) {
            return Err(ObjectPresentationError(
                "rebase placements must exactly match resident object IDs".into(),
            ));
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| ObjectPresentationError("object origin generation exhausted".into()))?;
        let transforms = self
            .resident
            .iter()
            .map(|(key, object)| {
                let origin = placements.get(&key.0).copied().ok_or_else(|| {
                    ObjectPresentationError("rebase lost a checked placement".into())
                })?;
                if world.get::<Transform>(object.receipt.root).is_none() {
                    return Err(ObjectPresentationError(
                        "resident transform was removed outside its owner".into(),
                    ));
                }
                let transform = local_transform(
                    &object.object,
                    &object.asset,
                    origin,
                    self.level_height,
                    self.limits,
                )?;
                Ok((key.clone(), origin, transform))
            })
            .collect::<Result<Vec<_>, ObjectPresentationError>>()?;
        for (id, origin, transform) in transforms {
            if let Some(object) = self.resident.get_mut(&id) {
                world.entity_mut(object.receipt.root).insert(transform);
                object.receipt.local_origin = origin;
            }
        }
        self.generation = generation;
        Ok(self.receipts().cloned().collect())
    }

    /// Release every owned root/mesh/material and invalidate all queued products.
    pub fn clear(&mut self, world: &mut World) {
        for (_, object) in std::mem::take(&mut self.resident) {
            world.despawn(object.receipt.root);
        }
        for (_, asset) in std::mem::take(&mut self.assets) {
            for part in asset.parts {
                drop(world.resource_mut::<Assets<Mesh>>().remove(part.mesh.id()));
            }
        }
        for (_, material) in std::mem::take(&mut self.materials) {
            drop(
                world
                    .resource_mut::<Assets<StandardMaterial>>()
                    .remove(material.handle.id()),
            );
        }
        self.context = Arc::new(());
        self.generation = 0;
    }

    fn allocate_asset(&mut self, world: &mut World, id: AssetKey, baked: Arc<BakedAsset>) {
        let parts = baked
            .parts
            .iter()
            .map(|part| {
                let material = self
                    .materials
                    .entry(part.key.style.clone())
                    .or_insert_with(|| SharedMaterial {
                        handle: world
                            .resource_mut::<Assets<StandardMaterial>>()
                            .add(part.material.clone()),
                        parts: 0,
                    });
                material.parts += 1;
                crate::CachedChunk {
                    key: part.key.clone(),
                    mesh: world.resource_mut::<Assets<Mesh>>().add(part.mesh.clone()),
                    material: material.handle.clone(),
                    surface_mode: part.surface_mode,
                    casts_shadows: part.casts_shadows,
                }
            })
            .collect();
        self.assets.insert(
            id,
            SharedAsset {
                baked,
                parts,
                users: BTreeSet::new(),
            },
        );
    }

    fn release_asset(&mut self, world: &mut World, id: &AssetKey, user: &ResidentKey) {
        let unused = self.assets.get_mut(id).is_some_and(|asset| {
            asset.users.remove(user);
            asset.users.is_empty()
        });
        if unused {
            if let Some(asset) = self.assets.remove(id) {
                for part in asset.parts {
                    drop(world.resource_mut::<Assets<Mesh>>().remove(part.mesh.id()));
                    let unused_style =
                        self.materials
                            .get_mut(&part.key.style)
                            .is_some_and(|material| {
                                material.parts = material.parts.saturating_sub(1);
                                material.parts == 0
                            });
                    if unused_style {
                        if let Some(material) = self.materials.remove(&part.key.style) {
                            drop(
                                world
                                    .resource_mut::<Assets<StandardMaterial>>()
                                    .remove(material.handle.id()),
                            );
                        }
                    }
                }
            }
        }
    }
}

fn spawn_parts(
    world: &mut World,
    root: Entity,
    prepared: &PreparedObject,
    parts: &[crate::CachedChunk],
) -> Vec<Entity> {
    parts
        .iter()
        .map(|part| {
            let mut child = world.spawn((
                Mesh3d(part.mesh.clone()),
                MeshMaterial3d(part.material.clone()),
                Transform::IDENTITY,
                Pickable::IGNORE,
                ResidentObjectPart {
                    id: prepared.object.id.clone(),
                    revision: prepared.revision,
                    fingerprint: prepared.fingerprint,
                    clip: prepared.clip,
                },
                crate::ObjectRenderChunk {
                    style: part.key.style.clone(),
                    canopy: part.key.canopy,
                },
                Name::new(format!("Resident object part {root} / {}", part.key.style)),
            ));
            if part.key.canopy {
                child.insert(crate::ObjectCanopyChunk);
            }
            if !part.casts_shadows {
                child.insert(NotShadowCaster);
            }
            if part.surface_mode == VoxelSurfaceMode::Translucent {
                child.insert(crate::ObjectTranslucentChunk);
            }
            child.id()
        })
        .collect()
}

fn validate_source(
    source: &Mesh,
    limits: ObjectPresentationLimits,
) -> Result<(), ObjectPresentationError> {
    if source.primitive_topology() != bevy::mesh::PrimitiveTopology::TriangleList {
        return Err(ObjectPresentationError(
            "source hex must use TriangleList topology".into(),
        ));
    }
    let Some(VertexAttributeValues::Float32x3(positions)) =
        source.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return Err(ObjectPresentationError(
            "source hex needs Float32x3 positions".into(),
        ));
    };
    if positions.is_empty() || positions.len() > limits.max_vertices_per_asset {
        return Err(ObjectPresentationError(
            "source hex vertex budget exceeded or empty".into(),
        ));
    }
    if positions
        .iter()
        .flatten()
        .any(|value| !value.is_finite() || value.abs() > 2.0)
    {
        return Err(ObjectPresentationError(
            "source hex vertices must be finite and within the unit primitive envelope".into(),
        ));
    }
    if let Some(indices) = source.indices() {
        if indices.len() % 3 != 0 || indices.iter().any(|index| index >= positions.len()) {
            return Err(ObjectPresentationError(
                "source hex has malformed triangle indices".into(),
            ));
        }
    } else if positions.len() % 3 != 0 {
        return Err(ObjectPresentationError(
            "source hex has incomplete triangles".into(),
        ));
    }
    Ok(())
}
