//! V4 authority behind the finite forest arena. Legacy voxel storage is a staging
//! and collision projection; every material change commits to V4 before publication.

use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use hex_assets::{HexObjectRotation, ObjectAssetId};
use hex_world_contracts::{
    LiquidKind, ResidencyRequest, VoxelEdit, VoxelPosition, WorldEditTransaction, WorldHex,
};
use hex_world_runtime::{FileChunkSource, IoLimits, RuntimeConfig, WorldRuntime};

use super::*;
use crate::procedural_v3::{
    FeatureId, FeatureKind, FillMaterialRole, LiquidFlowState, MapPresentationProjection,
    MaterializedLiquidVoxel, PlannedFeature,
};

#[path = "expedition_file.rs"]
mod expedition_file;

pub(super) fn asset_root() -> PathBuf {
    std::env::var_os("BEVY_ASSET_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

pub(super) struct ForestRuntime {
    pub runtime: WorldRuntime,
    next_edit: u64,
}

impl ForestRuntime {
    pub fn new(source: Arc<FileChunkSource>) -> Result<Self, String> {
        let count = source.manifest().chunks.len();
        let mut runtime = WorldRuntime::new(
            source,
            RuntimeConfig {
                max_resident_chunks: count,
                max_unsaved_chunks: count,
                max_publications_per_pump: 32,
                max_unsaved_transactions: 16_384,
                max_unsaved_transaction_bytes: 256 * 1024 * 1024,
                ..default()
            },
        )
        .map_err(|error| error.to_string())?;
        runtime
            .set_interests(vec![ResidencyRequest {
                id: "forest-battle-session".into(),
                center: WorldHex::new(0, 0),
                radius: 187,
                retention_radius: 187,
                priority: 255,
            }])
            .map_err(|error| error.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(120);
        while runtime.resident_chunks().count() < count {
            let update = runtime.pump();
            if let Some(failure) = update.failures.first() {
                return Err(format!(
                    "Forest chunk {:?}: {}",
                    failure.coordinate, failure.error
                ));
            }
            if Instant::now() >= deadline {
                return Err("Forest V4 residency timed out".into());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(Self {
            runtime,
            next_edit: 0,
        })
    }

    pub fn commit_projection(
        &mut self,
        map: &VoxelMap,
        changed: &BTreeSet<HexCoord>,
        substances: &SubstanceTable,
    ) -> Result<(), String> {
        let mut edits = Vec::new();
        let mut revisions = BTreeMap::new();
        for coord in changed {
            let global = WorldHex::new(i64::from(coord.x()), i64::from(coord.y()));
            let product = self
                .runtime
                .resident_chunk(global.chunk())
                .ok_or("Unloaded forest edit")?;
            let column = product
                .package
                .columns
                .iter()
                .find(|column| column.position == global)
                .ok_or("Forest edit outside package")?;
            let old_top = column.runs.iter().map(|run| run.top).max().unwrap_or(0);
            let new_top = map.column(*coord).map_or(0, Column::top);
            for level in 0..old_top.max(new_top) {
                let old = column
                    .runs
                    .iter()
                    .find(|run| (run.bottom..run.top).contains(&level));
                let old_id = old
                    .map(|run| material_id(&run.material, substances))
                    .transpose()?
                    .unwrap_or(SubstanceId::AIR);
                let new = map.get(TilePos::new(*coord, level));
                if old_id == new {
                    continue;
                }
                let material = if new.is_air() {
                    None
                } else {
                    Some(
                        match substances.name(new).ok_or("Unknown staged material")? {
                            "dirt" => "soil",
                            other => other,
                        }
                        .to_owned(),
                    )
                };
                edits.push(VoxelEdit {
                    position: VoxelPosition {
                        column: global,
                        level,
                    },
                    material,
                });
                revisions.insert(global.chunk(), product.revision);
            }
        }
        if edits.is_empty() {
            return Ok(());
        }
        edits.sort_by_key(|edit| edit.position);
        self.next_edit = self.next_edit.saturating_add(1);
        self.runtime
            .apply_transaction(&WorldEditTransaction {
                id: format!("battle-edit-{}", self.next_edit),
                expected_revisions: revisions,
                edits,
            })
            .map_err(|error| error.to_string())?;
        self.runtime.pump();
        Ok(())
    }
}

pub(super) fn material_id(name: &str, substances: &SubstanceTable) -> Result<SubstanceId, String> {
    // Physics/durability vocabulary remains compatible with battle. Presentation
    // keeps the original V4 names and colors, including distinct forest floors.
    let name = match name {
        "soil" | "pine-floor" => "dirt",
        "moss" | "foliage" => "grass",
        "timber" | "limestone" => "stone",
        "spring-water" => "water",
        other => other,
    };
    substances
        .id(name)
        .ok_or_else(|| format!("Forest material {name} has no battle policy"))
}

fn local(column: WorldHex) -> Result<HexCoord, String> {
    Ok(HexCoord::from_axial(
        i32::try_from(column.q).map_err(|error| error.to_string())?,
        i32::try_from(column.r).map_err(|error| error.to_string())?,
    ))
}

fn position(voxel: VoxelPosition) -> Result<TilePos, String> {
    Ok(TilePos::new(local(voxel.column)?, voxel.level))
}

pub(super) fn build(
    selection: ArenaSelection,
    substances: &SubstanceTable,
    art: &RuntimeArtCatalog,
) -> Result<worlds::WorldRecipe, String> {
    let path = std::env::var_os("HEX_FOREST_WORLD")
        .map(PathBuf::from)
        .unwrap_or_else(|| asset_root().join("assets/config/v4/forest-massif/compiled"));
    let source = Arc::new(
        FileChunkSource::open_workspace(&path, IoLimits::default()).map_err(|error| {
            format!(
                "Forest V4 package {}: {error}. Run python3 tools/forest_world.py compile.",
                path.display()
            )
        })?,
    );
    let expedition = source.manifest().world_id == "forest-massif-expedition";
    if !expedition && source.manifest().world_id != "forest-massif-battle" {
        return Err("Forest selection requires a Forest battle or expedition package".into());
    }
    let backend = ForestRuntime::new(source.clone())?;
    let mut map = VoxelMap::new();
    let mut anchors = BTreeMap::new();
    let mut liquids = BTreeMap::new();
    let mut features = BTreeMap::new();
    let mut max_level = 128;
    for product in backend.runtime.resident_chunks() {
        for column in &product.package.columns {
            let coord = local(column.position)?;
            let mut projected = Column::new();
            for run in &column.runs {
                if run.bottom < 0 {
                    return Err("Forest recipe requires nonnegative terrain".into());
                }
                let material = material_id(&run.material, substances)?;
                for level in run.bottom..run.top {
                    projected.set(level, material);
                }
                max_level = max_level.max(run.top + 32);
            }
            map.insert_column(coord, projected);
        }
        for anchor in &product.package.semantics.anchors {
            let Some(name) = anchor.id.strip_prefix("forest/anchor/") else {
                continue;
            };
            anchors.insert(name.to_owned(), position(anchor.position)?);
        }
        for liquid in &product.package.semantics.liquids {
            let coord = local(liquid.column)?;
            let downstream = liquid
                .downstream
                .first()
                .copied()
                .map(position)
                .transpose()?;
            for level in liquid.bottom..liquid.top {
                liquids.insert(
                    TilePos::new(coord, level),
                    MaterializedLiquidVoxel {
                        material: FillMaterialRole::Water,
                        flow: match liquid.kind {
                            LiquidKind::Standing => LiquidFlowState::Still,
                            LiquidKind::Directed => LiquidFlowState::Current,
                            LiquidKind::Waterfall => LiquidFlowState::Fall,
                        },
                        downstream,
                    },
                );
            }
        }
        for object in &product.package.semantics.objects {
            let mut root = position(object.origin)?;
            root.level -= 1;
            let id = FeatureId(u32::try_from(features.len()).map_err(|error| error.to_string())?);
            features.insert(
                id,
                PlannedFeature {
                    root,
                    kind: if object.asset.starts_with("plant/") {
                        FeatureKind::Tree
                    } else {
                        FeatureKind::TallGrass
                    },
                    object_id: ObjectAssetId::new(object.asset.clone())
                        .map_err(|error| error.to_string())?,
                    rotation: HexObjectRotation::new(object.rotation)
                        .map_err(|error| error.to_string())?,
                    blocker_footprint: BTreeSet::from([root]),
                },
            );
            for column in &object.occupancy {
                for run in &column.runs {
                    max_level = max_level.max(run.top + 32);
                }
            }
        }
    }
    let geometry = ArenaVoxelGeometry {
        radius: 187,
        level_height: 0.35,
        vertical_offset: 0.35,
        max_level,
        ..default()
    };
    let anchors: BTreeMap<_, _> = anchors
        .into_iter()
        .map(|(name, pos)| (name, pos.coord.to_world(geometry.top(pos))))
        .collect();
    let sites = expedition_file::load(&path, source.manifest(), geometry)?;
    let required: &[&str] = if expedition {
        &["party_start", "bridge_center", "bridge_west", "bridge_east"]
    } else {
        &[
            "party_start",
            "hostile_start",
            "forest_outer_a",
            "forest_outer_b",
            "forest_middle",
            "forest_deep_a",
            "forest_deep_b",
            "dragon_lower",
            "dragon_middle",
            "dragon_upper",
            "ancient_tree",
            "bridge_west",
            "bridge_east",
        ]
    };
    for &name in required {
        if !anchors.contains_key(name) {
            return Err(format!("Forest lacks {name}"));
        }
    }
    let hostile_start = anchors
        .get("hostile_start")
        .copied()
        .or_else(|| {
            sites.as_ref()?.encounters.values().next().map(|site| {
                let at = site.deployment.preferred;
                at.coord.to_world(geometry.top(at))
            })
        })
        .ok_or("Forest lacks a hostile support candidate")?;
    let mut view = ArenaTerrainView {
        revision: 1,
        selection,
        spawns: [
            *anchors.get("party_start").ok_or("Missing start")?,
            hostile_start,
        ],
        anchors,
        expedition: sites,
        full_rebuild: true,
        ..default()
    };
    for (coord, column) in map.columns() {
        publish_column(&mut view, coord, column, substances);
        view.dirty_columns.insert(coord);
        for run in crate::runs(column)
            .into_iter()
            .filter(|run| !substances.is_solid(run.substance))
        {
            view.liquids.push(ArenaSolidSpan {
                bottom: TilePos::new(coord, run.bottom),
                top_level: run.top - 1,
                substance: run.substance,
            });
        }
    }
    view.liquids.sort_by_key(|span| span.bottom);
    let presentation =
        MapPresentationProjection::from_snapshot_parts(liquids, features, BTreeMap::new());
    worlds::project_static(&mut view, &presentation, geometry, art)?;
    // Preserve V4's material admission even where several names share one battle
    // durability class. The projection must never offer an edit V4 forbids.
    for product in backend.runtime.resident_chunks() {
        // Publish the same world-owned exclusions used by V4 edit admission.
        // Exact grounded crowns leave their understorey air available to Shield;
        // old packages retain whole-column exclusions until regenerated.
        for object in &product.package.semantics.object_influences {
            for (position, ranges) in object.terrain_edit_protection() {
                if position.chunk() != product.coordinate {
                    continue;
                }
                for (bottom, top) in ranges {
                    let clipped = (bottom.max(geometry.min_level), top.min(geometry.max_level));
                    if clipped.0 <= clipped.1 {
                        view.edit_protected
                            .entry(local(position)?)
                            .or_default()
                            .push(clipped);
                    }
                }
            }
        }
        for anchor in &product.package.semantics.anchors {
            if anchor.role != hex_world_contracts::AnchorRole::Observation {
                view.edit_protected
                    .entry(local(anchor.position.column)?)
                    .or_default()
                    .push((anchor.position.level, anchor.position.level + 2));
            }
        }
        for column in &product.package.columns {
            let coord = local(column.position)?;
            for run in &column.runs {
                let immutable = backend
                    .runtime
                    .manifest()
                    .materials
                    .iter()
                    .find(|material| material.id == run.material)
                    .is_some_and(|material| !material.diggable);
                if immutable {
                    view.edit_protected
                        .entry(coord)
                        .or_default()
                        .push((run.bottom, run.top - 1));
                }
            }
        }
    }
    // The one built crossing is an authored reservation: explosions cannot remove
    // its only support and strand the run. Ordinary bank/bed physics is unchanged.
    for (coord, column) in map.columns() {
        if coord.y().abs() <= 4 && coord.x().abs() <= 26 {
            view.edit_protected
                .entry(coord)
                .or_default()
                .push((0, column.top().saturating_sub(1)));
        }
    }
    // Forest FeatureVoxels declares opaque foliage solid in the V4 package.
    // Continuous collision must preserve that exact exported canopy occupancy.
    for span in &mut view.static_spans {
        span.blocks_movement |= span.blocks_sight;
    }
    compact_static(&mut view);
    super::expedition::validate(&view, geometry, substances)?;
    Ok(worlds::WorldRecipe {
        map,
        geometry,
        view,
        presentation,
        forest_source: Some(source),
    })
}

fn compact_static(view: &mut ArenaTerrainView) {
    view.static_spans.sort_by_key(|span| {
        (
            span.bottom.coord,
            span.blocks_movement,
            span.blocks_projectiles,
            span.blocks_sight,
            span.bottom.level,
        )
    });
    let mut compact: Vec<hex_core::arena::ArenaStaticSpan> = Vec::new();
    for span in std::mem::take(&mut view.static_spans) {
        if let Some(last) = compact.last_mut() {
            if last.bottom.coord == span.bottom.coord
                && last.blocks_movement == span.blocks_movement
                && last.blocks_projectiles == span.blocks_projectiles
                && last.blocks_sight == span.blocks_sight
                && span.bottom.level <= last.top_level + 1
            {
                last.top_level = last.top_level.max(span.top_level);
                continue;
            }
        }
        compact.push(span);
    }
    view.static_spans = compact;
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_world_contracts::{QueryResult, WorldQuery};

    /// Full authored fixture test is explicit so ordinary small arena checks do
    /// not quietly depend on an operator's generated world workspace.
    #[test]
    #[ignore = "requires python3 tools/forest_world.py compile; run explicitly for delivery"]
    fn authored_forest_publication_and_v4_edit_roundtrip() {
        let content = load_content().expect("accepted content");
        let selection = ArenaSelection {
            map: hex_core::arena::ArenaMap::ForestMassif,
            ..default()
        };
        let mut recipe =
            build(selection, &content.substances, &content.art).expect("complete V4 map");
        assert_eq!(recipe.map.columns().count(), 105_469);
        assert_eq!(recipe.view.anchors.len(), 13);
        assert!(recipe.geometry.max_level > 260);
        assert!(recipe
            .view
            .liquids
            .windows(2)
            .all(|pair| pair.first().expect("pair").bottom <= pair.last().expect("pair").bottom));
        for name in [
            "party_start",
            "forest_outer_a",
            "forest_outer_b",
            "forest_middle",
            "forest_deep_a",
            "forest_deep_b",
            "dragon_lower",
            "dragon_middle",
            "dragon_upper",
        ] {
            let feet = recipe.view.anchors.get(name).expect("named support");
            let support = recipe
                .geometry
                .voxel_at(*feet - Vec3::Y * 0.01)
                .expect("inside map");
            assert!(
                content.substances.is_solid(recipe.map.get(support)),
                "{name}"
            );
        }
        for pos in [
            TilePos::new(HexCoord::from_axial(-132, 5), 40),
            TilePos::new(HexCoord::from_axial(-109, 100), 40),
            TilePos::new(HexCoord::from_axial(-109, 100), 41),
        ] {
            assert!(
                recipe
                    .view
                    .edit_protected
                    .get(&pos.coord)
                    .is_some_and(|ranges| ranges
                        .iter()
                        .any(|(low, high)| (*low..=*high).contains(&pos.level))),
                "V4 semantic protection must be published before a spell: {pos:?}"
            );
        }
        let source = recipe.forest_source.clone().expect("V4 source");
        let mut backend = ForestRuntime::new(source).expect("resident authority");
        let pos = TilePos::new(HexCoord::from_axial(-5, 50), 100);
        assert!(recipe.map.get(pos).is_air());
        recipe.map.set(pos, content.materials.stone);
        backend
            .commit_projection(
                &recipe.map,
                &BTreeSet::from([pos.coord]),
                &content.substances,
            )
            .expect("V4 shield assignment");
        let query = VoxelPosition {
            column: WorldHex::new(-5, 50),
            level: 100,
        };
        assert_eq!(
            backend.runtime.voxel(query),
            QueryResult::Ready(Some("stone".into()))
        );
        recipe.map.set(pos, SubstanceId::AIR);
        backend
            .commit_projection(
                &recipe.map,
                &BTreeSet::from([pos.coord]),
                &content.substances,
            )
            .expect("V4 destruction");
        assert_eq!(backend.runtime.voxel(query), QueryResult::Ready(None));
    }
}
