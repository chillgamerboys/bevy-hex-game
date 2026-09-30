//! Default-off, four-chunk diagnostic. No source, picking, object or water ownership.
use super::*;
use hex_schematic::v4::northern::{terrain_surface::*, NorthernOverview};
use hex_world_contracts::{VoxelPosition, WorldHex};
use hex_world_runtime::FiniteWorldSession;

const MAX_VERTICES: usize = 100_000;
const MAX_BYTES: usize = 16 * 1024 * 1024;

/// Read-only facts about the active, published four-chunk diagnostic.
/// Counts describe retained mesh assets, not pixel coverage or total process memory.
#[derive(Clone, Debug, serde::Serialize)]
pub struct GrandSurfaceSampleSnapshot {
    /// Explicit diagnostic mode; absent snapshots never imply activation.
    pub mode: &'static str,
    /// Admitted immutable package identity.
    pub package_fingerprint: u64,
    /// Admitted canonical authoring identity.
    pub source_fingerprint: u64,
    /// Complete source/halo blocks, including blocks that never draw.
    pub source_chunks: usize,
    /// Storage coordinates converted by the diagnostic owner.
    pub selected_chunks: Vec<ChunkId>,
    /// Successfully converted products in the most recent atomic build.
    pub converted_chunks: usize,
    /// Products still attached to their original entity with a retained mesh asset.
    pub published_chunks: usize,
    /// Published products whose current Bevy view-visibility flag is true.
    pub visible_chunks: usize,
    /// Actual retained vertex count across published products.
    pub vertices: usize,
    /// Actual retained index count divided by three.
    pub triangles: usize,
    /// Packed position/normal/color/u32-index bytes, excluding allocator/GPU overhead.
    pub packed_bytes: usize,
    /// Aggregate packed-output ceiling; not a total or peak memory bound.
    pub packed_byte_limit: usize,
    /// Whether persistent source edits selected the exact interval-face rebuild.
    pub exact_edited_fallback: bool,
}

/// Inspect actual published diagnostic assets without granting residency or rebuilding.
/// Returns None before activation, in ordinary/headless runs, and after renderer clear.
#[must_use]
pub fn surface_sample_snapshot(world: &World) -> Option<GrandSurfaceSampleSnapshot> {
    let renderer = world.get_resource::<Renderer>()?;
    let sample = renderer.surface_sample.as_ref()?;
    let state = world.get_resource::<StreamedArena>()?;
    let source = state.overview.terrain_surface.as_ref()?;
    let assets = world.get_resource::<Assets<Mesh>>()?;
    let mut result = GrandSurfaceSampleSnapshot {
        mode: "crystal-four",
        package_fingerprint: state.runtime.manifest().fingerprint,
        source_fingerprint: state.runtime.manifest().source_fingerprint,
        source_chunks: source.source_chunks().len(),
        selected_chunks: sample.selected.iter().copied().collect(),
        converted_chunks: sample.selected.len(),
        published_chunks: 0,
        visible_chunks: 0,
        vertices: 0,
        triangles: 0,
        packed_bytes: 0,
        packed_byte_limit: MAX_BYTES,
        exact_edited_fallback: sample.exact_edited_fallback,
    };
    for chunk in &sample.selected {
        let Some((entity, handle)) = renderer.proxies.get(chunk) else {
            continue;
        };
        if world
            .get::<Mesh3d>(*entity)
            .is_none_or(|mesh| mesh.0 != *handle)
        {
            continue;
        }
        let Some(mesh) = assets.get(handle) else {
            continue;
        };
        result.published_chunks += 1;
        result.visible_chunks += usize::from(
            world
                .get::<ViewVisibility>(*entity)
                .is_some_and(|visibility| visibility.get()),
        );
        let vertices = mesh.count_vertices();
        let indices = mesh.indices().map_or(0, Indices::len);
        result.vertices += vertices;
        result.triangles += indices / 3;
        result.packed_bytes += vertices * 40 + indices * 4;
    }
    Some(result)
}

pub(super) fn requested() -> bool {
    std::env::var("HEX_GRAND_SURFACE_SAMPLE").is_ok_and(|value| value == "1")
}

pub(super) struct State {
    selected: BTreeSet<ChunkId>,
    revisions: BTreeMap<ChunkId, u64>,
    exact_edited_fallback: bool,
}

impl State {
    pub(super) fn owns(&self, coordinate: ChunkId) -> bool {
        self.selected.contains(&coordinate)
    }

    pub(super) fn new(
        state: &StreamedArena,
    ) -> Result<(Option<Self>, BTreeMap<ChunkId, Mesh>), String> {
        if !requested() {
            return Ok((None, BTreeMap::new()));
        }
        if state.overview.world_id != "grand-v4" || std::env::var_os("HEX_ARENA_CAPTURE").is_none()
        {
            return Err(
                "Grand surface sample requires explicit windowless Grand diagnostic".into(),
            );
        }
        let source = state
            .overview
            .terrain_surface
            .as_ref()
            .ok_or("sample payload absent")?;
        let selected: BTreeSet<_> = source
            .chunks
            .iter()
            .filter(|c| c.surface.is_some())
            .map(|c| c.coordinate)
            .collect();
        if selected.len() != 4 || source.chunks.len() > 16 {
            return Err(
                "surface diagnostic requires exactly four patches and bounded complete halo".into(),
            );
        }
        for chunk in source
            .chunks
            .iter()
            .filter(|c| selected.contains(&c.coordinate))
        {
            for (index, flags) in chunk.protection.iter().enumerate() {
                if (index % 16 == 0 || index % 16 == 15 || index / 16 == 0 || index / 16 == 15)
                    && flags & PATCH_BOUNDARY == 0
                {
                    return Err("sample lacks exact native detailed-handoff ring".into());
                }
            }
        }
        let meshes = meshes(&state.overview, &state.edits)?;
        info!(package=state.overview.package_fingerprint, source=state.overview.source_fingerprint,
            patches=selected.len(), source_chunks=source.chunks.len(),
            "GRAND_SURFACE_SAMPLE DIAGNOSTIC active; omitted old regions retained; outer legacy interface unresolved");
        Ok((
            Some(Self {
                selected,
                revisions: revisions(source, &state.edits),
                exact_edited_fallback: has_edits(source, &state.edits, state.overview.level_bounds),
            }),
            meshes,
        ))
    }

    pub(super) fn refresh(
        &mut self,
        state: &StreamedArena,
    ) -> Result<Option<BTreeMap<ChunkId, Mesh>>, String> {
        let source = state
            .overview
            .terrain_surface
            .as_ref()
            .ok_or("sample payload disappeared")?;
        let current = revisions(source, &state.edits);
        if current == self.revisions {
            return Ok(None);
        }
        // Four products are built before any asset changes. This exclusive owner has
        // no asynchronous sample job that can complete against stale edit revisions.
        let products = meshes(&state.overview, &state.edits)?;
        self.revisions = current;
        self.exact_edited_fallback = has_edits(source, &state.edits, state.overview.level_bounds);
        Ok(Some(products))
    }
}

fn revisions(
    source: &TerrainSurfaceOverview,
    edits: &FiniteWorldSession,
) -> BTreeMap<ChunkId, u64> {
    source
        .source_chunks()
        .into_iter()
        .map(|c| (c, edits.revision(c).unwrap_or(0)))
        .collect()
}

fn has_edits(
    source: &TerrainSurfaceOverview,
    edits: &FiniteWorldSession,
    levels: [i32; 2],
) -> bool {
    source.chunks.iter().any(|c| {
        (0..16).any(|r| {
            (0..16).any(|q| {
                edits.terrain_edited_in_column(
                    WorldHex::new(c.coordinate.q * 16 + q, c.coordinate.r * 16 + r),
                    levels[0],
                    levels[1] + 1,
                )
            })
        })
    }) || source
        .halo
        .iter()
        .any(|c| edits.terrain_edited_in_column(c.column, levels[0], levels[1] + 1))
}

fn edited_source(
    map: &NorthernOverview,
    edits: &FiniteWorldSession,
) -> Result<Option<TerrainSurfaceOverview>, String> {
    let source = map
        .terrain_surface
        .as_ref()
        .ok_or("missing sample source")?;
    if !has_edits(source, edits, map.level_bounds) {
        return Ok(None);
    }
    let palette: BTreeMap<_, _> = map
        .materials
        .iter()
        .enumerate()
        .map(|(i, m)| {
            u16::try_from(i)
                .map(|i| (m.id.as_str(), (i, m.solid)))
                .map_err(|e| e.to_string())
        })
        .collect::<Result<_, _>>()?;
    let mut result = source.clone();
    result.profiles.clear();
    let mut intern = BTreeMap::new();
    let profiles = &mut result.profiles;
    let mut update = |p: WorldHex, id: &mut u16, flags: &mut u8| -> Result<(), String> {
        if *id == OUTSIDE_PROFILE {
            return Ok(());
        }
        let original = source
            .profiles
            .get(usize::from(*id))
            .ok_or("missing original profile")?;
        let profile =
            if edits.terrain_edited_in_column(p, map.level_bounds[0], map.level_bounds[1] + 1) {
                let mut runs: Vec<SolidRun> = Vec::new();
                for level in map.level_bounds[0]..=map.level_bounds[1] {
                    let at = VoxelPosition { column: p, level };
                    // None is a deletion only when the persistent sparse edit ledger
                    // says so. An unloaded unedited cell always comes from the companion.
                    let material = if edits.terrain_edited(at) {
                        edits
                            .terrain_at(at)
                            .map(|m| palette.get(m).copied().ok_or("unknown edited material"))
                            .transpose()?
                            .filter(|(_, solid)| *solid)
                            .map(|(i, _)| i)
                    } else {
                        original
                            .runs
                            .iter()
                            .find(|r| i32::from(r.bottom) <= level && level < i32::from(r.top))
                            .map(|r| r.material)
                    };
                    if let Some(material) = material {
                        let bottom = i16::try_from(level).map_err(|e| e.to_string())?;
                        let top = bottom.checked_add(1).ok_or("edited level overflow")?;
                        if let Some(last) = runs
                            .last_mut()
                            .filter(|r| r.top == bottom && r.material == material)
                        {
                            last.top = top;
                        } else {
                            runs.push(SolidRun {
                                bottom,
                                top,
                                material,
                            });
                        }
                    }
                }
                SolidProfile { runs }
            } else {
                original.clone()
            };
        *flags |= PATCH_BOUNDARY;
        if profile
            .runs
            .first()
            .is_some_and(|r| i32::from(r.bottom) > map.level_bounds[0])
            || profile.runs.windows(2).any(|p| {
                p.first()
                    .zip(p.get(1))
                    .is_some_and(|(a, b)| a.top < b.bottom)
            })
        {
            *flags |= STACKED;
        }
        *id = if let Some(id) = intern.get(&profile) {
            *id
        } else {
            let id = u16::try_from(profiles.len()).map_err(|e| e.to_string())?;
            intern.insert(profile.clone(), id);
            profiles.push(profile);
            id
        };
        Ok(())
    };
    for chunk in &mut result.chunks {
        for (index, (id, flags)) in chunk
            .profiles
            .iter_mut()
            .zip(&mut chunk.protection)
            .enumerate()
        {
            let p = WorldHex::new(
                chunk.coordinate.q * 16 + i64::try_from(index % 16).map_err(|e| e.to_string())?,
                chunk.coordinate.r * 16 + i64::try_from(index / 16).map_err(|e| e.to_string())?,
            );
            update(p, id, flags)?;
        }
        if chunk.surface.is_some() {
            chunk.surface = Some(MacroSurface {
                vertices: vec![],
                triangles: vec![],
                maximum_error: 0.0,
            });
        }
    }
    for fact in &mut result.halo {
        update(fact.column, &mut fact.profile, &mut fact.protection)?;
    }
    Ok(Some(result))
}

fn meshes(
    map: &NorthernOverview,
    edits: &FiniteWorldSession,
) -> Result<BTreeMap<ChunkId, Mesh>, String> {
    let edited = edited_source(map, edits)?;
    let source = edited
        .as_ref()
        .or(map.terrain_surface.as_ref())
        .ok_or("missing source")?;
    let builder = SurfaceBuilder::new(
        source,
        map.radius,
        map.level_bounds,
        &map.materials,
        f64::from(map.level_height),
    )
    .map_err(|e| e.to_string())?;
    let mut products = BTreeMap::new();
    let mut bytes = 0usize;
    let mut triangles = 0usize;
    for chunk in &source.chunks {
        let Some(cap) = &chunk.surface else {
            continue;
        };
        let faces = builder
            .build_faces(chunk.coordinate, cap, MAX_VERTICES)
            .map_err(|e| e.to_string())?;
        bytes += faces.vertices * 40 + faces.triangles * 12;
        triangles += faces.triangles;
        if bytes > MAX_BYTES {
            return Err("diagnostic surface exceeds 16 MiB aggregate packed buffer budget".into());
        }
        products.insert(chunk.coordinate, mesh(map, &faces)?);
    }
    info!(
        bytes,
        triangles,
        exact_edited_fallback = edited.is_some(),
        "GRAND_SURFACE_SAMPLE bounded buffers; no runtime triangulation"
    );
    Ok(products)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "admitted bounded lattice geometry is converted to Bevy f32 only for disposable presentation"
)]
fn mesh(map: &NorthernOverview, faces: &SurfaceFaces) -> Result<Mesh, String> {
    let palette: Vec<_> = map
        .materials
        .iter()
        .map(|m| {
            let [r, g, b, a] = m.color;
            Color::srgba_u8(r, g, b, a).to_linear().to_f32_array()
        })
        .collect();
    let mut positions = Vec::with_capacity(faces.vertices);
    let mut normals = Vec::with_capacity(faces.vertices);
    let mut colors = Vec::with_capacity(faces.vertices);
    let mut indices = Vec::with_capacity(faces.triangles * 3);
    for face in &faces.faces {
        let points: Vec<Vec3> = face
            .points
            .iter()
            .map(|p| {
                Vec3::new(
                    (3.0_f64.sqrt() * (p.lattice[0] + p.lattice[1] * 0.5) / 3.) as f32,
                    (p.level * f64::from(map.level_height)) as f32,
                    (p.lattice[1] * 0.5) as f32,
                )
            })
            .collect();
        let normal = points
            .windows(3)
            .filter_map(|p| {
                let [a, b, c] = p else {
                    return None;
                };
                Some((*b - *a).cross(*c - *a))
            })
            .find(|n| n.length_squared() > 1e-12)
            .ok_or("degenerate surface face")?
            .normalize()
            .to_array();
        let first = u32::try_from(positions.len()).map_err(|e| e.to_string())?;
        for point in points {
            let color = if let Some(material) = face.material {
                *palette
                    .get(usize::from(material))
                    .ok_or("face material missing")?
            } else {
                super::proxy_sample(map, point.x, point.z)
                    .ok_or("macro palette sample missing")?
                    .color
            };
            positions.push(point.to_array());
            normals.push(normal);
            colors.push(color);
        }
        for i in 1..face.points.len() - 1 {
            let offset = u32::try_from(i).map_err(|e| e.to_string())?;
            indices.extend([first, first + offset, first + offset + 1]);
        }
    }
    Ok(Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(indices)))
}

#[cfg(test)]
#[path = "grand_surface_sample_tests.rs"]
mod tests;
