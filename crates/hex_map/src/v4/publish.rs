use std::{collections::BTreeMap, sync::Arc};

use bevy::{picking::Pickable, prelude::*};
use hex_core::{
    Headroom, HexTile, RunBottom, SubstanceId, TerrainPickRun, TerrainRenderBatch, MAX_HEADROOM,
};
use hex_world_contracts::{ChunkId, ChunkPackage, ManifestIndex, WorldManifest};

use super::{
    PreparedChunk, PresentationError, PresentationLimits, RenderOrigin, RunSource, TerrainPreparer,
};

/// Global identity of a disposable resident root; never a gameplay authority.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResidentChunk {
    /// Exact global storage coordinate.
    pub coordinate: ChunkId,
    /// Exact runtime revision represented by all children.
    pub revision: u64,
    /// Exact revised payload checksum.
    pub fingerprint: u64,
}

/// Measured publication outcome for a single root and its owned assets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkReceipt {
    /// Global storage coordinate, unaffected by rendering origin.
    pub coordinate: ChunkId,
    /// Exact runtime revision published.
    pub revision: u64,
    /// Exact payload checksum, possibly different from the original manifest base.
    pub fingerprint: u64,
    /// Current disposable root; replacement/rebase returns a new entity.
    pub root: Entity,
    /// Number of exact logical interval entities.
    pub logical_runs: usize,
    /// Logical runs supplied by static-object occupancy.
    pub object_runs: usize,
    /// Exact liquid semantic rows in this resident package; rendered as prisms.
    pub liquid_intervals: usize,
    /// Number of picking batches and independently owned mesh assets.
    pub meshes: usize,
    /// Total mesh vertices published for this chunk.
    pub vertices: usize,
    /// Root-owned object assets represented only by occupancy geometry.
    pub unresolved_object_assets: usize,
}

struct PublishedChunk {
    receipt: ChunkReceipt,
    package: Arc<ChunkPackage>,
    meshes: Vec<Handle<Mesh>>,
}

/// Owns the disposable roots/assets for one bounded local presentation window.
///
/// The host selects nearby visible chunks independently of authoritative residency.
/// Distant parties can retain runtime chunks without allocating presentation here.
/// CPU preparation may run on workers using [`Self::preparer`]; publication runs
/// against one Bevy world, one bounded chunk at a time. No plugin is required.
#[derive(Resource)]
pub struct TerrainPresenter {
    context: TerrainPreparer,
    resident: BTreeMap<ChunkId, PublishedChunk>,
    materials: BTreeMap<SubstanceId, Handle<StandardMaterial>>,
}

impl TerrainPresenter {
    /// Create a presenter with default operational window limits and a typed palette.
    pub fn new(
        manifest: &WorldManifest,
        origin: RenderOrigin,
        level_height: f32,
    ) -> Result<Self, PresentationError> {
        Self::with_limits(
            manifest,
            origin,
            level_height,
            PresentationLimits::default(),
        )
    }

    /// Configure presentation bounds independently of world size or host residency.
    pub fn with_limits(
        manifest: &WorldManifest,
        origin: RenderOrigin,
        level_height: f32,
        limits: PresentationLimits,
    ) -> Result<Self, PresentationError> {
        let manifest = Arc::new(manifest.clone());
        let index = Arc::new(ManifestIndex::new(manifest.clone())?);
        limits.validate()?;
        if !level_height.is_finite() || !(0.01..=16.0).contains(&level_height) {
            return Err(PresentationError(
                "level height must be finite and within 0.01..=16".into(),
            ));
        }
        let palette = manifest
            .materials
            .iter()
            .enumerate()
            .map(|(index, material)| {
                let substance = u16::try_from(index + 1)
                    .map_err(|error| PresentationError(error.to_string()))?;
                Ok((
                    material.id.clone(),
                    (SubstanceId(substance), material.clone()),
                ))
            })
            .collect::<Result<BTreeMap<_, _>, PresentationError>>()?;
        Ok(Self {
            context: TerrainPreparer {
                manifest,
                index,
                palette: Arc::new(palette),
                origin,
                level_height,
                limits,
            },
            resident: BTreeMap::new(),
            materials: BTreeMap::new(),
        })
    }

    /// Clone the current immutable worker preparation context without copying the manifest.
    #[must_use]
    pub fn preparer(&self) -> TerrainPreparer {
        self.context.clone()
    }

    /// Validate and build one CPU mesh product synchronously.
    pub fn prepare(
        &self,
        package: &ChunkPackage,
        revision: u64,
    ) -> Result<PreparedChunk, PresentationError> {
        self.context.prepare(package, revision)
    }

    /// Current integer anchor of this presentation window.
    #[must_use]
    pub fn origin(&self) -> RenderOrigin {
        self.context.origin
    }

    /// Iterate the current root/revision mapping in global chunk order.
    pub fn receipts(&self) -> impl ExactSizeIterator<Item = &ChunkReceipt> {
        self.resident.values().map(|entry| &entry.receipt)
    }

    /// Borrow the exact accepted package, including all unrendered semantic metadata.
    #[must_use]
    pub fn package(&self, chunk: ChunkId) -> Option<&ChunkPackage> {
        self.resident
            .get(&chunk)
            .map(|entry| entry.package.as_ref())
    }

    /// Publish or replace exactly one chunk; old revisions and stale contexts fail closed.
    ///
    /// Equal revision/checksum is idempotent. Meshes and logical entities of other
    /// roots remain untouched. Materials are shared and retained until [`Self::clear`].
    pub fn publish(
        &mut self,
        world: &mut World,
        prepared: PreparedChunk,
    ) -> Result<ChunkReceipt, PresentationError> {
        if prepared.context.origin != self.context.origin
            || prepared.context.level_height.to_bits() != self.context.level_height.to_bits()
            || prepared.context.limits != self.context.limits
            || prepared.context.manifest.fingerprint != self.context.manifest.fingerprint
        {
            return Err(PresentationError(
                "prepared product belongs to a stale origin or palette".into(),
            ));
        }
        let chunk = prepared.coordinate();
        if let Some(old) = self.resident.get(&chunk) {
            if prepared.revision < old.receipt.revision
                || (prepared.revision == old.receipt.revision
                    && prepared.package.fingerprint != old.receipt.fingerprint)
            {
                return Err(PresentationError(
                    "stale or conflicting resident revision".into(),
                ));
            }
            if prepared.revision == old.receipt.revision {
                return Ok(old.receipt.clone());
            }
        } else if self.resident.len() >= self.context.limits.max_resident_chunks {
            return Err(PresentationError(
                "active presentation root budget exhausted".into(),
            ));
        }
        Ok(self.install(world, prepared))
    }

    /// Remove one root and all owned meshes, retaining shared palette materials.
    pub fn remove(&mut self, world: &mut World, chunk: ChunkId) -> Option<ChunkReceipt> {
        let old = self.resident.remove(&chunk)?;
        cleanup(world, &old);
        Some(old.receipt)
    }

    /// Atomically prepare then replace the bounded resident set at a new origin.
    ///
    /// Any preparation failure leaves origin, roots and assets untouched. This
    /// initial implementation temporarily holds both old and new meshes, bounded
    /// by `max_resident_chunks`; callers should unload out-of-view roots before
    /// moving to a distant window. Incremental rebasing is a future optimization.
    pub fn rebase(
        &mut self,
        world: &mut World,
        origin: RenderOrigin,
    ) -> Result<Vec<ChunkReceipt>, PresentationError> {
        if origin == self.context.origin {
            return Ok(self.receipts().cloned().collect());
        }
        let mut next = self.context.clone();
        next.origin = origin;
        let prepared = self
            .resident
            .values()
            .map(|old| next.prepare(&old.package, old.receipt.revision))
            .collect::<Result<Vec<_>, _>>()?;
        self.context = next;
        Ok(prepared
            .into_iter()
            .map(|chunk| self.install(world, chunk))
            .collect())
    }

    /// Remove all owned roots, meshes and shared materials before retiring this presenter.
    pub fn clear(&mut self, world: &mut World) {
        for old in std::mem::take(&mut self.resident).into_values() {
            cleanup(world, &old);
        }
        if let Some(mut assets) = world.get_resource_mut::<Assets<StandardMaterial>>() {
            for handle in std::mem::take(&mut self.materials).into_values() {
                assets.remove(handle.id());
            }
        } else {
            self.materials.clear();
        }
    }

    fn install(&mut self, world: &mut World, prepared: PreparedChunk) -> ChunkReceipt {
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        let coordinate = prepared.coordinate();
        let root = world
            .spawn((
                ResidentChunk {
                    coordinate,
                    revision: prepared.revision,
                    fingerprint: prepared.package.fingerprint,
                },
                prepared.marker,
                Transform::default(),
                Visibility::default(),
                Name::new(format!("V4Terrain[{},{}]", coordinate.q, coordinate.r)),
            ))
            .id();
        let mut children = Vec::new();
        let mut meshes = Vec::new();
        let mut receipt = ChunkReceipt {
            coordinate,
            revision: prepared.revision,
            fingerprint: prepared.package.fingerprint,
            root,
            logical_runs: 0,
            object_runs: 0,
            liquid_intervals: prepared.package.semantics.liquids.len(),
            meshes: prepared.batches.len(),
            vertices: 0,
            unresolved_object_assets: prepared.package.semantics.objects.len(),
        };
        for batch in prepared.batches {
            let material = self
                .materials
                .entry(batch.substance)
                .or_insert_with(|| {
                    let [r, g, b, a] = batch.material.color;
                    let color = Color::srgba_u8(r, g, b, a);
                    let mut material = StandardMaterial::from(color);
                    material.perceptual_roughness = 0.9;
                    material.alpha_mode = if a < 255 {
                        AlphaMode::Blend
                    } else {
                        AlphaMode::Opaque
                    };
                    world
                        .resource_mut::<Assets<StandardMaterial>>()
                        .add(material)
                })
                .clone();
            let mut lookup = Vec::new();
            for run in batch.runs {
                receipt.logical_runs += 1;
                receipt.object_runs += usize::from(run.exact.source == RunSource::StaticObject);
                let headroom = run.exact.headroom.map_or(MAX_HEADROOM, |value| {
                    i32::try_from(value)
                        .unwrap_or(MAX_HEADROOM)
                        .min(MAX_HEADROOM)
                });
                let geometry = run.geometry;
                let entity = world
                    .spawn((
                        HexTile,
                        geometry.position.coord,
                        geometry.position,
                        geometry.span,
                        batch.substance,
                        RunBottom(geometry.bottom),
                        Headroom(headroom),
                        run.exact,
                    ))
                    .id();
                lookup.push(TerrainPickRun::new(
                    entity,
                    geometry.position,
                    geometry.span,
                ));
                children.push(entity);
            }
            receipt.vertices += batch.mesh.count_vertices();
            let mesh = world.resource_mut::<Assets<Mesh>>().add(batch.mesh);
            let entity = world
                .spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material),
                    Transform::default(),
                    Visibility::Inherited,
                    Pickable::default(),
                    TerrainRenderBatch::new(prepared.marker, batch.substance, lookup),
                ))
                .id();
            children.push(entity);
            meshes.push(mesh);
        }
        world.entity_mut(root).add_children(&children);
        let entry = PublishedChunk {
            receipt: receipt.clone(),
            package: prepared.package,
            meshes,
        };
        if let Some(old) = self.resident.insert(coordinate, entry) {
            cleanup(world, &old);
        }
        receipt
    }
}

fn cleanup(world: &mut World, old: &PublishedChunk) {
    world.despawn(old.receipt.root);
    if let Some(mut assets) = world.get_resource_mut::<Assets<Mesh>>() {
        for handle in &old.meshes {
            assets.remove(handle.id());
        }
    }
}
