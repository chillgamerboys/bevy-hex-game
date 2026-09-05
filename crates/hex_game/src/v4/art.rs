//! Stock art is a disposable, chunk-owned projection of exact authored objects.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use bevy::prelude::*;
use hex_assets::{
    ArtPalette, ObjectAssetId, ObjectBlueprint, ObjectCatalogFile, RuntimeArtCatalog,
    VoxelStyleCatalog,
};
use hex_map::v4::RenderOrigin;
use hex_objects::v4::{
    ObjectPresentationLimits, PreparedObject, ResidentObjectPart, ResidentObjectPresenter,
};
use hex_world_contracts::{
    hash_serializable, ChunkId, ColumnData, ObjectInstance, VoxelRun, WorldHex,
};
use hex_world_runtime::WorldRuntime;
use serde::de::DeserializeOwned;

pub(super) struct ArtPlan {
    pub fingerprint: u64,
    pub suppression: Vec<ColumnData>,
    pub fragments: Vec<PreparedObject>,
    pub unresolved: Vec<String>,
}

pub(super) struct StockArt {
    catalog: Arc<RuntimeArtCatalog>,
    presenter: ResidentObjectPresenter,
    observed: BTreeMap<ChunkId, (u64, u64)>,
    objects: BTreeMap<String, ObjectInstance>,
    chunks: BTreeMap<ChunkId, Vec<String>>,
    signatures: BTreeMap<ChunkId, u64>,
    unresolved: BTreeMap<ChunkId, Vec<String>>,
    // A changed object cannot publish its new source beside another clip of its
    // old source. Retire old clips through exact terrain proxies first.
    replacing: BTreeSet<String>,
}

impl StockArt {
    pub fn load(mesh: Mesh) -> Result<Self, String> {
        let root = std::env::var_os("BEVY_ASSET_ROOT")
            .map(std::path::PathBuf::from)
            .ok_or("launch with Cargo so BEVY_ASSET_ROOT identifies the shipped asset tree")?
            .join("assets");
        let palette: ArtPalette = read(&root.join("art/palette.ron"))?;
        let styles: VoxelStyleCatalog = read(&root.join("art/voxel_styles.ron"))?;
        let manifest: ObjectCatalogFile = read(&root.join("art/object_catalog.ron"))?;
        let mut blueprints = BTreeMap::new();
        for id in manifest.ids() {
            let path = id.asset_path().map_err(|error| error.to_string())?;
            let blueprint: ObjectBlueprint = read(&root.join(path))?;
            blueprints.insert(id.clone(), blueprint);
        }
        let catalog = Arc::new(
            RuntimeArtCatalog::from_sources(&palette, &styles, &manifest, blueprints)
                .map_err(|error| error.to_string())?,
        );
        let presenter = ResidentObjectPresenter::with_limits(
            catalog.clone(),
            mesh,
            super::LEVEL_HEIGHT,
            ObjectPresentationLimits {
                max_asset_types: 512,
                ..default()
            },
        )
        .map_err(|error| error.to_string())?;
        Ok(Self {
            catalog,
            presenter,
            observed: BTreeMap::new(),
            objects: BTreeMap::new(),
            chunks: BTreeMap::new(),
            signatures: BTreeMap::new(),
            unresolved: BTreeMap::new(),
            replacing: BTreeSet::new(),
        })
    }

    /// Only resident root revisions are inspected. Known foreign-root records are
    /// retained while any of their exact footprint remains resident.
    pub fn refresh(&mut self, runtime: &WorldRuntime) -> Result<(), String> {
        let resident = runtime
            .resident_chunks()
            .map(|p| p.coordinate)
            .collect::<BTreeSet<_>>();
        let previously_resident = self.observed.len();
        self.observed
            .retain(|coordinate, _| resident.contains(coordinate));
        let mut changed = previously_resident != self.observed.len();
        for product in runtime.resident_chunks() {
            let signature = (product.revision, product.package.fingerprint);
            if self.observed.get(&product.coordinate) == Some(&signature) {
                continue;
            }
            self.observed.insert(product.coordinate, signature);
            self.objects
                .retain(|_, object| object.origin.column.chunk() != product.coordinate);
            for object in &product.package.semantics.objects {
                if ObjectAssetId::new(&object.asset)
                    .ok()
                    .is_some_and(|id| self.catalog.object(&id).is_some())
                {
                    self.objects.insert(object.id.clone(), object.clone());
                }
            }
            changed = true;
        }
        let before = self.objects.len();
        self.objects.retain(|_, object| {
            object
                .occupancy
                .iter()
                .any(|column| resident.contains(&column.position.chunk()))
        });
        changed |= before != self.objects.len();
        if changed {
            // An unloaded root can no longer keep an obsolete source alive after
            // an edit to its resident footprint. Identity-tagged influences, not
            // the union occupancy, prove that the cached complete source applies.
            let mut stale = Vec::new();
            for (id, object) in &self.objects {
                let fingerprint = hash_serializable(object).map_err(|e| e.to_string())?;
                if object.occupancy.iter().any(|column| {
                    runtime
                        .resident_chunk(column.position.chunk())
                        .is_some_and(|product| {
                            !product
                                .package
                                .semantics
                                .object_influences
                                .iter()
                                .any(|influence| {
                                    influence.id == *id
                                        && influence.source_fingerprint == fingerprint
                                })
                        })
                }) {
                    stale.push(id.clone());
                }
            }
            for id in stale {
                self.objects.remove(&id);
            }
        }
        let replacing = replacing_sources(
            self.presenter.receipts().filter_map(|receipt| {
                self.presenter
                    .object(&receipt.id)
                    .map(|object| (receipt.id.as_str(), object))
            }),
            &self.objects,
        );
        changed |= self.replacing != replacing;
        self.replacing = replacing;
        if self.objects.len() > 8192 {
            return Err("stock art resident source budget exceeded".into());
        }
        if changed {
            self.chunks.clear();
            self.signatures.clear();
            for object in self.objects.values() {
                let footprint = object
                    .occupancy
                    .iter()
                    .map(|column| column.position.chunk())
                    .collect::<BTreeSet<_>>();
                for chunk in footprint {
                    self.chunks
                        .entry(chunk)
                        .or_default()
                        .push(object.id.clone());
                }
            }
            for (chunk, ids) in &self.chunks {
                let sources = ids
                    .iter()
                    .filter_map(|id| self.objects.get(id))
                    .collect::<Vec<_>>();
                self.signatures.insert(
                    *chunk,
                    hash_serializable(&(
                        sources,
                        ids.iter()
                            .filter(|id| self.replacing.contains(*id))
                            .collect::<Vec<_>>(),
                    ))
                    .map_err(|error| error.to_string())?,
                );
            }
        }
        Ok(())
    }

    pub fn signature(&self, chunk: ChunkId) -> u64 {
        self.signatures.get(&chunk).copied().unwrap_or(0)
    }

    pub fn prepare(
        &self,
        chunk: ChunkId,
        revision: u64,
        origin: RenderOrigin,
    ) -> Result<ArtPlan, String> {
        let mut columns = BTreeMap::<WorldHex, Vec<VoxelRun>>::new();
        let mut fragments = Vec::new();
        let mut unresolved = Vec::new();
        for id in self.chunks.get(&chunk).into_iter().flatten() {
            if self.replacing.contains(id) {
                continue;
            }
            let object = self
                .objects
                .get(id)
                .ok_or("stock object index lost its source")?;
            let local = match origin.local_voxel(object.origin) {
                Ok(local) => local,
                Err(error) => {
                    unresolved.push(format!("{id}: {error}"));
                    continue;
                }
            };
            // Full exact-footprint validation precedes any proxy suppression.
            match self
                .presenter
                .prepare_fragment(object, revision, local, chunk)
            {
                Ok(fragment) => fragments.push(fragment),
                Err(error) => {
                    // Stock-art compatibility is not a world admission rule. Keep
                    // exact proxy geometry when this catalogue cannot represent it.
                    unresolved.push(format!("{id}: {error}"));
                    continue;
                }
            }
            for column in object
                .occupancy
                .iter()
                .filter(|column| column.position.chunk() == chunk)
            {
                columns
                    .entry(column.position)
                    .or_default()
                    .extend(column.runs.clone());
            }
        }
        Ok(ArtPlan {
            fingerprint: self.signature(chunk),
            suppression: canonical_union(columns)?,
            fragments,
            unresolved,
        })
    }

    pub fn publish(
        &mut self,
        world: &mut World,
        chunk: ChunkId,
        plan: ArtPlan,
    ) -> Result<(), String> {
        let receipts = self
            .presenter
            .replace_fragments(world, chunk, plan.fragments)
            .map_err(|error| error.to_string())?;
        self.unresolved.insert(chunk, plan.unresolved);
        let parts = receipts
            .iter()
            .filter_map(|receipt| world.get::<Children>(receipt.root))
            .flat_map(|children| children.iter())
            .filter(|entity| world.get::<ResidentObjectPart>(*entity).is_some())
            .collect::<Vec<_>>();
        for entity in parts {
            world.entity_mut(entity).insert(Pickable::default());
        }
        Ok(())
    }

    pub fn remove(&mut self, world: &mut World, chunk: ChunkId) -> Result<(), String> {
        self.presenter
            .replace_fragments(world, chunk, Vec::new())
            .map_err(|error| error.to_string())?;
        self.unresolved.remove(&chunk);
        Ok(())
    }

    fn placements(
        &self,
        origin: RenderOrigin,
    ) -> Result<BTreeMap<String, hex_core::TilePos>, String> {
        let mut placements = BTreeMap::new();
        for receipt in self.presenter.receipts() {
            let object = self
                .presenter
                .object(&receipt.id)
                .ok_or("resident art lost its source")?;
            placements.insert(
                receipt.id.clone(),
                origin
                    .local_voxel(object.origin)
                    .map_err(|error| error.to_string())?,
            );
        }
        Ok(placements)
    }

    pub fn validate_rebase(&self, world: &World, origin: RenderOrigin) -> Result<(), String> {
        self.presenter
            .validate_rebase(world, &self.placements(origin)?)
            .map_err(|error| error.to_string())
    }

    pub fn rebase(&mut self, world: &mut World, origin: RenderOrigin) -> Result<(), String> {
        self.presenter
            .rebase(world, &self.placements(origin)?)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub fn object(&self, id: &str) -> Option<&ObjectInstance> {
        self.presenter.object(id)
    }
    pub fn unresolved(&self) -> Vec<String> {
        self.unresolved
            .values()
            .flatten()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn counts(&self) -> (usize, usize, usize) {
        (
            self.presenter.receipts().len(),
            self.presenter.cached_asset_count(),
            self.presenter
                .receipts()
                .map(|receipt| receipt.vertices)
                .sum(),
        )
    }
}

fn replacing_sources<'a>(
    published: impl Iterator<Item = (&'a str, &'a ObjectInstance)>,
    current: &BTreeMap<String, ObjectInstance>,
) -> BTreeSet<String> {
    published
        .filter(|(id, object)| current.get(*id) != Some(*object))
        .map(|(id, _)| id.to_owned())
        .collect()
}

fn read<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|error| format!("{}: {error}", path.display()))?
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err(format!("{} exceeds asset input budget", path.display()));
    }
    ron::de::from_bytes(&bytes).map_err(|error| format!("{}: {error}", path.display()))
}

fn canonical_union(columns: BTreeMap<WorldHex, Vec<VoxelRun>>) -> Result<Vec<ColumnData>, String> {
    columns
        .into_iter()
        .map(|(position, mut runs)| {
            runs.sort_by(|a, b| {
                (a.bottom, a.top, &a.material).cmp(&(b.bottom, b.top, &b.material))
            });
            let mut union: Vec<VoxelRun> = Vec::new();
            for run in runs {
                if let Some(previous) = union.last_mut() {
                    if run.bottom < previous.top && run.material != previous.material {
                        return Err(
                            "stock proxy masks have conflicting material constraints".into()
                        );
                    }
                    if run.bottom <= previous.top && run.material == previous.material {
                        previous.top = previous.top.max(run.top);
                        continue;
                    }
                }
                union.push(run);
            }
            let column = ColumnData {
                position,
                runs: union,
            };
            column.validate().map_err(|error| error.to_string())?;
            Ok(column)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_sources_wait_until_every_old_fragment_is_retired() {
        let old = ObjectInstance {
            id: "region/tree".into(),
            region_id: "region".into(),
            asset: "plant/tall-narrow".into(),
            origin: hex_world_contracts::VoxelPosition {
                column: WorldHex::new(15, 0),
                level: 1,
            },
            rotation: 0,
            occupancy: Vec::new(),
        };
        let mut replacement = old.clone();
        replacement.origin.column = WorldHex::new(16, 0);
        let current = BTreeMap::from([(old.id.clone(), replacement.clone())]);
        let blocked = BTreeSet::from([old.id.clone()]);
        assert_eq!(
            replacing_sources(
                [(old.id.as_str(), &old), (old.id.as_str(), &old)].into_iter(),
                &current
            ),
            blocked
        );
        assert_eq!(
            replacing_sources([(old.id.as_str(), &old)].into_iter(), &current),
            blocked
        );
        assert!(replacing_sources(std::iter::empty(), &current).is_empty());
        assert!(
            replacing_sources([(old.id.as_str(), &replacement)].into_iter(), &current).is_empty()
        );
        assert_eq!(
            replacing_sources([(old.id.as_str(), &old)].into_iter(), &BTreeMap::new()),
            blocked
        );
    }

    #[test]
    fn overlapping_same_material_fragments_coalesce_but_conflicts_never_hide_geometry() {
        let run = |bottom, top, material: &str| VoxelRun {
            bottom,
            top,
            material: material.into(),
        };
        let input = BTreeMap::from([(
            WorldHex::new(-1, 16),
            vec![run(2, 5, "wood"), run(0, 3, "wood"), run(5, 7, "wood")],
        )]);
        assert_eq!(
            canonical_union(input)
                .expect("union")
                .first()
                .expect("column")
                .runs,
            vec![run(0, 7, "wood")]
        );
        let conflict = BTreeMap::from([(
            WorldHex::new(-1, 16),
            vec![run(2, 5, "wood"), run(0, 3, "stone")],
        )]);
        assert!(canonical_union(conflict).is_err());
    }
}
