//! Sparse, world-owned removals projected through bounded immutable sections.

use std::sync::Arc;

use super::*;

const SECTION_WIDTH: i32 = 16;
const SECTION_HEIGHT: i32 = 32;

/// Work from the most recent masked-object publication, for focused render checks.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ObjectCarveRenderStats {
    /// Accepted removal revision represented by the current meshes.
    pub revision: u64,
    /// Sections replaced on this publication; unchanged sections retain entities.
    pub rebuilt_sections: usize,
    /// Authored cells considered in those sections, excluding a bounded apron.
    pub considered_cells: usize,
    /// Immutable section count for this blueprint.
    pub total_sections: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Section(i32, i32, i32);

impl Section {
    fn for_cell(cell: LocalVoxelCoord) -> Self {
        // Center horizontal sections on the tree origin, avoiding four sections
        // for every small centered tree. Quotient/remainder cannot overflow.
        let horizontal = |value: i32| {
            value.div_euclid(SECTION_WIDTH)
                + i32::from(value.rem_euclid(SECTION_WIDTH) >= SECTION_WIDTH / 2)
        };
        Self(
            horizontal(cell.q),
            horizontal(cell.r),
            cell.level.div_euclid(SECTION_HEIGHT),
        )
    }
}

#[derive(Clone, Debug)]
pub(super) struct RenderedCarving {
    pub(super) mask: ObjectCarveMask,
    sections: BTreeMap<Section, Vec<Entity>>,
}

impl RenderedCarving {
    pub(super) fn children(&self) -> Vec<Entity> {
        self.sections.values().flatten().copied().collect()
    }
}

#[derive(Debug)]
struct PreparedObject {
    origin: LocalVoxelCoord,
    sections: BTreeMap<Section, Vec<LocalVoxelCoord>>,
    occupied: BTreeMap<LocalVoxelCoord, (OccupiedCell, bool)>,
    pristine: BTreeMap<Section, CachedObject>,
}

#[derive(Debug, Default)]
pub(super) struct CarveRenderCache {
    sources: BTreeMap<ObjectAssetId, Arc<PreparedObject>>,
    unique: BTreeMap<Entity, BTreeMap<Section, Vec<Handle<Mesh>>>>,
}

impl CarveRenderCache {
    pub(super) fn remove_entity(&mut self, entity: Entity, meshes: &mut Assets<Mesh>) {
        if let Some(sections) = self.unique.remove(&entity) {
            for handles in sections.into_values() {
                for handle in handles {
                    drop(meshes.remove(handle.id()));
                }
            }
        }
    }

    pub(super) fn clear(&mut self, meshes: &mut Assets<Mesh>) {
        for source in self.sources.values() {
            for section in source.pristine.values() {
                for chunk in &section.chunks {
                    drop(meshes.remove(chunk.mesh.id()));
                }
            }
        }
        self.sources.clear();
        for entity in self.unique.keys().copied().collect::<Vec<_>>() {
            self.remove_entity(entity, meshes);
        }
    }

    fn remove_section(&mut self, entity: Entity, section: Section, meshes: &mut Assets<Mesh>) {
        if let Some(handles) = self
            .unique
            .get_mut(&entity)
            .and_then(|sections| sections.remove(&section))
        {
            for handle in handles {
                drop(meshes.remove(handle.id()));
            }
        }
    }
}

fn neighbors(cell: LocalVoxelCoord) -> impl Iterator<Item = LocalVoxelCoord> {
    AXIAL_NEIGHBOURS
        .into_iter()
        .filter_map(move |(q, r)| {
            Some(LocalVoxelCoord::new(
                cell.q.checked_add(q)?,
                cell.r.checked_add(r)?,
                cell.level,
            ))
        })
        .chain([-1, 1].into_iter().filter_map(move |delta| {
            Some(LocalVoxelCoord::new(
                cell.q,
                cell.r,
                cell.level.checked_add(delta)?,
            ))
        }))
}

fn changed_sections(
    source: &PreparedObject,
    mask: &ObjectCarveMask,
    previous: Option<&RenderedCarving>,
) -> BTreeSet<Section> {
    let Some(previous) = previous else {
        return source.sections.keys().copied().collect();
    };
    mask.removed
        .symmetric_difference(&previous.mask.removed)
        .flat_map(|cell| std::iter::once(*cell).chain(neighbors(*cell)))
        .map(Section::for_cell)
        .filter(|section| source.sections.contains_key(section))
        .collect()
}

fn prepare(
    object_id: &ObjectAssetId,
    catalog: &RuntimeArtCatalog,
    source_mesh: &Mesh,
    edge: ReviewEdgeTreatment,
    cache: &mut ObjectRenderCache,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> Result<Arc<PreparedObject>, String> {
    if let Some(source) = cache.carved.sources.get(object_id) {
        return Ok(Arc::clone(source));
    }
    let blueprint = catalog
        .object(object_id)
        .ok_or_else(|| format!("missing carved blueprint '{object_id}'"))?;
    let canopy: BTreeSet<_> = blueprint.canopy_occluders.iter().copied().collect();
    let mut source = PreparedObject {
        origin: blueprint.origin,
        sections: BTreeMap::new(),
        occupied: BTreeMap::new(),
        pristine: BTreeMap::new(),
    };
    for placement in &blueprint.placements {
        let style = catalog
            .style(&placement.style)
            .ok_or_else(|| format!("missing carved style '{}'", placement.style))?;
        source
            .sections
            .entry(Section::for_cell(placement.position))
            .or_default()
            .push(placement.position);
        source.occupied.insert(
            placement.position,
            (
                OccupiedCell {
                    style: placement.style.clone(),
                    surface_mode: style.authored().surface_mode(),
                },
                canopy.contains(&placement.position),
            ),
        );
    }
    let treated = mesh_with_micro_bevel_normals(source_mesh, edge)?;
    for section in source.sections.keys().copied().collect::<Vec<_>>() {
        let baked = bake_section(&treated, &source, section, &BTreeSet::new())?;
        let cached = publish_meshes(baked, catalog, cache, meshes, materials)?;
        source.pristine.insert(section, cached);
    }
    let source = Arc::new(source);
    cache
        .carved
        .sources
        .insert(object_id.clone(), Arc::clone(&source));
    Ok(source)
}

fn publish_meshes(
    baked: Vec<(ChunkKey, Mesh)>,
    catalog: &RuntimeArtCatalog,
    cache: &mut ObjectRenderCache,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> Result<CachedObject, String> {
    let mut chunks = Vec::with_capacity(baked.len());
    for (key, mesh) in baked {
        let style = catalog
            .style(&key.style)
            .ok_or_else(|| format!("missing carved style '{}'", key.style))?;
        let material = cached_material(cache, &key.style, style, materials);
        let surface_mode = style.authored().surface_mode();
        chunks.push(CachedChunk {
            key,
            mesh: meshes.add(mesh),
            material,
            surface_mode,
            casts_shadows: matches!(
                surface_mode,
                VoxelSurfaceMode::Opaque | VoxelSurfaceMode::Cutout
            ),
        });
    }
    Ok(CachedObject { chunks })
}

/// Only selected section cells and their one-cell neighbors enter a rebuild.
/// Removed neighbors never participate in culling, exposing real new cut faces.
fn bake_section(
    treated: &Mesh,
    source: &PreparedObject,
    section: Section,
    removed: &BTreeSet<LocalVoxelCoord>,
) -> Result<Vec<(ChunkKey, Mesh)>, String> {
    let Some(cells) = source.sections.get(&section) else {
        return Ok(Vec::new());
    };
    let mut groups: BTreeMap<ChunkKey, Vec<LocalVoxelCoord>> = BTreeMap::new();
    let mut occupancy: BTreeMap<bool, BTreeMap<LocalVoxelCoord, OccupiedCell>> = BTreeMap::new();
    for cell in cells.iter().filter(|cell| !removed.contains(cell)) {
        let Some((occupied, canopy)) = source.occupied.get(cell) else {
            continue;
        };
        groups
            .entry(ChunkKey {
                style: occupied.style.clone(),
                canopy: *canopy,
            })
            .or_default()
            .push(*cell);
        for position in std::iter::once(*cell).chain(neighbors(*cell)) {
            if removed.contains(&position) {
                continue;
            }
            if let Some((occupied, canopy)) = source.occupied.get(&position) {
                occupancy
                    .entry(*canopy)
                    .or_default()
                    .insert(position, occupied.clone());
            }
        }
    }
    let enclosed = cull_internal_faces(
        treated,
        FaceCullMask {
            sides: [true; 6],
            top: true,
            bottom: true,
        },
    )?;
    let enclosed_invisible = enclosed.indices().is_some_and(Indices::is_empty);
    let mut baked = Vec::with_capacity(groups.len());
    for (key, mut cells) in groups {
        cells.sort_unstable();
        let occupied = occupancy
            .get(&key.canopy)
            .ok_or_else(|| "carved visibility partition missing".to_owned())?;
        if enclosed_invisible {
            cells.retain(|cell| !face_cull_mask(*cell, occupied).is_full());
        }
        if !cells.is_empty() {
            baked.push((key, merge_cells(treated, source.origin, &cells, occupied)?));
        }
    }
    Ok(baked)
}

pub(super) fn reconcile(
    entity: Entity,
    instance: &ObjectInstance,
    tree: Option<TreeOccluder>,
    mask: &ObjectCarveMask,
    previous: Option<&RenderedCarving>,
    catalog: &RuntimeArtCatalog,
    source_mesh: &Mesh,
    edge: ReviewEdgeTreatment,
    cache: &mut ObjectRenderCache,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> Result<RenderedCarving, String> {
    let source = prepare(
        instance.object_id(),
        catalog,
        source_mesh,
        edge,
        cache,
        meshes,
        materials,
    )?;
    let changed = changed_sections(&source, mask, previous);
    let considered_cells = changed
        .iter()
        .filter_map(|section| source.sections.get(section))
        .map(Vec::len)
        .sum();
    let mut sections = previous.map_or_else(BTreeMap::new, |previous| previous.sections.clone());
    let treated = mesh_with_micro_bevel_normals(source_mesh, edge)?;
    // Build the replacement set before retiring any live section. One failed
    // mesh must not leave a partially replaced object in this render frame.
    let mut replacements = Vec::with_capacity(changed.len());
    for section in &changed {
        let affected = !mask.removed.is_empty()
            && source.sections.get(section).is_some_and(|cells| {
                cells.iter().any(|cell| {
                    std::iter::once(*cell)
                        .chain(neighbors(*cell))
                        .any(|cell| mask.removed.contains(&cell))
                })
            });
        let result = if affected {
            bake_section(&treated, &source, *section, &mask.removed)
                .and_then(|baked| publish_meshes(baked, catalog, cache, meshes, materials))
        } else {
            source
                .pristine
                .get(section)
                .cloned()
                .ok_or_else(|| "pristine carved section missing".to_owned())
        };
        match result {
            Ok(cached) => replacements.push((*section, cached, affected)),
            Err(error) => {
                for (_, cached, unique) in replacements {
                    if unique {
                        for chunk in cached.chunks {
                            drop(meshes.remove(chunk.mesh.id()));
                        }
                    }
                }
                return Err(error);
            }
        }
    }
    for (section, cached, unique) in replacements {
        if let Some(children) = sections.remove(&section) {
            for child in children {
                commands.entity(child).despawn();
            }
        }
        cache.carved.remove_section(entity, section, meshes);
        if unique {
            cache.carved.unique.entry(entity).or_default().insert(
                section,
                cached
                    .chunks
                    .iter()
                    .map(|chunk| chunk.mesh.clone())
                    .collect(),
            );
        }
        sections.insert(
            section,
            spawn_chunks(commands, entity, instance.object_id(), &cached, tree),
        );
    }
    commands.entity(entity).insert(ObjectCarveRenderStats {
        revision: mask.revision,
        rebuilt_sections: changed.len(),
        considered_cells,
        total_sections: source.sections.len(),
    });
    Ok(RenderedCarving {
        mask: mask.clone(),
        sections,
    })
}

#[cfg(test)]
mod tests;
