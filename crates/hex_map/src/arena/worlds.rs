//! Accepted world recipes and exact arena publications. No tactical lifecycle.

use hex_assets::{
    ObjectBlueprint, ObjectCatalogFile, ObjectInstance, ObjectPart, PlantPart, RuntimeArtCatalog,
    VoxelStyleCatalog, VoxelSurfaceMode,
};
use hex_core::arena::{
    ArenaDeploymentRegion, ArenaMap, ArenaSelection, ArenaSolidSpan, ArenaStaticSpan,
};

use crate::procedural_v3::{FeatureKind, MapPresentationProjection};
use crate::settings::{MapSettings, ProceduralSettings, TerrainSettings};
use crate::terrain::TerrainPalette;

use super::*;

/// Pristine, map-owned reset snapshot. Damage and command ledgers are never cached.
#[derive(Clone)]
pub(super) struct WorldRecipe {
    pub map: VoxelMap,
    pub geometry: ArenaVoxelGeometry,
    pub view: ArenaTerrainView,
    pub presentation: MapPresentationProjection,
}

pub(super) fn load_art(palette: &ArtPalette) -> Result<RuntimeArtCatalog, String> {
    let styles: VoxelStyleCatalog =
        ron::from_str(include_str!("../../../../assets/art/voxel_styles.ron"))
            .map_err(|error| format!("Arena voxel styles: {error}"))?;
    let manifest: ObjectCatalogFile =
        ron::from_str(include_str!("../../../../assets/art/object_catalog.ron"))
            .map_err(|error| format!("Arena object manifest: {error}"))?;
    // The accepted catalog and every dependency are embedded together, just like
    // arena material content. No asynchronous cross-checkout content can leak in.
    let sources = [
        include_str!("../../../../assets/art/objects/plant/old-growth.ron"),
        include_str!("../../../../assets/art/objects/plant/small-broadleaf.ron"),
        include_str!("../../../../assets/art/objects/plant/snowy-old-growth.ron"),
        include_str!("../../../../assets/art/objects/plant/snowy-small-broadleaf.ron"),
        include_str!("../../../../assets/art/objects/plant/snowy-tall-narrow.ron"),
        include_str!("../../../../assets/art/objects/plant/tall-narrow.ron"),
        include_str!("../../../../assets/art/objects/prop/cave-lichen.ron"),
        include_str!("../../../../assets/art/objects/prop/cave-moss.ron"),
        include_str!("../../../../assets/art/objects/prop/crystal-branched.ron"),
        include_str!("../../../../assets/art/objects/prop/crystal-cathedral-heart.ron"),
        include_str!("../../../../assets/art/objects/prop/crystal-low-cluster.ron"),
        include_str!("../../../../assets/art/objects/prop/crystal-spire.ron"),
        include_str!("../../../../assets/art/objects/prop/grass-tuft.ron"),
        include_str!("../../../../assets/art/objects/prop/snowy-grass-tuft.ron"),
    ];
    let mut objects = BTreeMap::new();
    for source in sources {
        let object: ObjectBlueprint =
            ron::from_str(source).map_err(|error| format!("Arena object blueprint: {error}"))?;
        objects.insert(object.id.clone(), object);
    }
    RuntimeArtCatalog::from_sources(palette, &styles, &manifest, objects)
        .map_err(|error| format!("Arena accepted art graph: {error}"))
}

pub(super) fn build(
    selection: ArenaSelection,
    materials: ArenaMaterials,
    substances: &SubstanceTable,
    art: &RuntimeArtCatalog,
) -> Result<WorldRecipe, String> {
    let (map, geometry, anchors, presentation) = match selection.map {
        ArenaMap::Duel => {
            let geometry = ArenaVoxelGeometry::default();
            let [human, enemy] = spawn_positions(geometry);
            (
                build_arena(geometry, materials),
                geometry,
                BTreeMap::from([
                    ("party_start".to_owned(), human),
                    ("hostile_start".to_owned(), enemy),
                ]),
                MapPresentationProjection::default(),
            )
        }
        ArenaMap::Fort | ArenaMap::SevenRegions => {
            let (source, seed) = match selection.map {
                ArenaMap::Fort => (
                    include_str!("../../../../assets/config/worlds/procedural-fort.ron"),
                    640_367_719,
                ),
                _ => (
                    include_str!("../../../../assets/config/worlds/procedural-ring7.ron"),
                    703_700_113,
                ),
            };
            let settings: MapSettings = ron::from_str(source)
                .map_err(|error| format!("Arena {:?} settings: {error}", selection.map))?;
            let TerrainSettings::Procedural(ProceduralSettings::V3(v3)) = &settings.terrain else {
                return Err("Arena real maps require the accepted V3 generator".to_owned());
            };
            let palette = TerrainPalette::for_terrain(substances, &settings.terrain)?;
            let generated = crate::procedural_v3::build(
                settings.grid_radius,
                settings.level_height,
                v3,
                seed,
                &palette,
                &|substance| substances.is_solid(substance),
                Some(art),
            )
            .map_err(|error| format!("Arena {:?} generation: {error}", selection.map))?;
            let geometry = ArenaVoxelGeometry {
                radius: settings.grid_radius,
                level_height: settings.level_height,
                // Preserve ordinary map and ObjectInstance coordinates. Storage
                // identities remain unchanged through all edits and damage batches.
                vertical_offset: settings.level_height,
                ..default()
            };
            if generated
                .map
                .columns()
                .any(|(_, column)| column.top() > geometry.max_level + 1)
            {
                return Err("Arena recipe exceeds its published editable level bounds".to_owned());
            }
            let anchors = generated
                .anchors
                .iter()
                .map(|(id, pos)| {
                    (
                        id.as_str().to_owned(),
                        pos.coord.to_world(geometry.top(pos)),
                    )
                })
                .collect();
            (generated.map, geometry, anchors, generated.presentation)
        }
    };
    let anchor = |name: &str| {
        anchors
            .get(name)
            .copied()
            .ok_or_else(|| format!("Arena {:?} lacks required anchor {name}", selection.map))
    };
    let spawns = [anchor("party_start")?, anchor("hostile_start")?];
    let mut view = ArenaTerrainView {
        revision: 1,
        selection,
        anchors,
        spawns,
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
    project_static(&mut view, &presentation, geometry, art)?;
    view.battle_deployment = battle_deployment(&view, geometry)?;
    view.elongated_deployment = elongated_deployment(&view, geometry)?;
    Ok(WorldRecipe {
        map,
        geometry,
        view,
        presentation,
    })
}

// These are supporting-voxel identities for the accepted recipes, not actor
// poses. Keep the adventure starts and authored terrain unchanged. Gameplay
// resolves complete bodies inside each finite set at the accepted reset.
fn battle_deployment(
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
) -> Result<Option<[ArenaDeploymentRegion; 2]>, String> {
    let (centers, level) = match view.selection.map {
        ArenaMap::Duel => ([(-6, 0), (6, 0)], GROUND_LEVEL),
        // Both sides share the open west courtyard. The adventure starts lie
        // outside/inside the curtain wall and would require gate/keep routing.
        ArenaMap::Fort => ([(-4, 2), (-2, -2)], 15),
        ArenaMap::SevenRegions => return Ok(None),
    };
    let regions = centers.map(|(q, r)| {
        let preferred = TilePos::new(HexCoord::from_axial(q, r), level);
        ArenaDeploymentRegion {
            preferred,
            surfaces: preferred
                .coord
                .within_radius(1)
                .into_iter()
                .map(|coord| TilePos::new(coord, level))
                .collect(),
        }
    });
    for surface in regions.iter().flat_map(|region| &region.surfaces) {
        let exposed_ground = view.voxels.contains_key(surface)
            && view.columns.get(&surface.coord).is_some_and(|runs| {
                runs.iter().map(|run| run.top_level).max() == Some(surface.level)
            });
        let static_or_liquid =
            view.static_spans
                .iter()
                .any(|span| span.bottom.coord == surface.coord && span.top_level > surface.level)
                || view.liquids.iter().any(|span| {
                    span.bottom.coord == surface.coord && span.top_level > surface.level
                });
        let protected = view
            .edit_protected
            .get(&surface.coord)
            .is_some_and(|intervals| intervals.iter().any(|(_, top)| *top >= surface.level));
        if !geometry.contains_column(surface.coord)
            || !(geometry.min_level..=geometry.max_level).contains(&surface.level)
            || !exposed_ground
            || static_or_liquid
            || protected
        {
            return Err(format!(
                "Arena {:?} deployment surface {surface:?} is not open dry unreserved ground",
                view.selection.map
            ));
        }
    }
    Ok(Some(regions))
}

/// Finite extra ground facts for the four-segment body. These do not enlarge
/// ordinary deployment and never create terrain or select a roof as a fallback.
fn elongated_deployment(
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
) -> Result<Option<[ArenaDeploymentRegion; 2]>, String> {
    let Some(ordinary) = &view.battle_deployment else {
        return Ok(None);
    };
    let regions = ordinary.each_ref().map(|region| {
        let preferred = region.preferred;
        let surfaces = preferred
            .coord
            .within_radius(2)
            .into_iter()
            // Fort's opposing regions stay in their own courtyard half. Radius
            // two alone would introduce shared ground across the center line.
            .filter(|coord| {
                view.selection.map != ArenaMap::Fort
                    || coord.y().signum() == preferred.coord.y().signum()
            })
            .map(|coord| TilePos::new(coord, preferred.level))
            .filter(|surface| deployment_surface_open(view, geometry, *surface))
            .collect();
        ArenaDeploymentRegion {
            preferred,
            surfaces,
        }
    });
    if regions.iter().any(|region| {
        !region.surfaces.contains(&region.preferred) || !contains_straight_run(region, 4)
    }) {
        return Err(format!(
            "Arena {:?} lacks a dry four-segment deployment pocket",
            view.selection.map
        ));
    }
    Ok(Some(regions))
}

fn deployment_surface_open(
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    surface: TilePos,
) -> bool {
    geometry.contains_column(surface.coord)
        && (geometry.min_level..=geometry.max_level).contains(&surface.level)
        && view.voxels.contains_key(&surface)
        && view
            .columns
            .get(&surface.coord)
            .is_some_and(|runs| runs.iter().map(|run| run.top_level).max() == Some(surface.level))
        && !view
            .static_spans
            .iter()
            .any(|span| span.bottom.coord == surface.coord && span.top_level >= surface.level)
        && !view
            .liquids
            .iter()
            .any(|span| span.bottom.coord == surface.coord && span.top_level >= surface.level)
        && !view
            .edit_protected
            .get(&surface.coord)
            .is_some_and(|intervals| intervals.iter().any(|(_, high)| *high >= surface.level))
}

/// A content diagnostic only: gameplay still checks the actual full body, yaw,
/// live support and other actors before accepting an initial roster.
fn contains_straight_run(region: &ArenaDeploymentRegion, count: i32) -> bool {
    region.surfaces.iter().any(|start| {
        [(1, 0), (0, 1), (-1, 1)].into_iter().any(|(q, r)| {
            (0..count).all(|step| {
                region.surfaces.contains(&TilePos::new(
                    HexCoord::from_axial(start.coord.x() + q * step, start.coord.y() + r * step),
                    start.level,
                ))
            })
        })
    })
}

#[cfg(test)]
#[path = "burrow_world_tests.rs"]
mod burrow_tests;

fn project_static(
    view: &mut ArenaTerrainView,
    presentation: &MapPresentationProjection,
    geometry: ArenaVoxelGeometry,
    art: &RuntimeArtCatalog,
) -> Result<(), String> {
    for feature in presentation.features().values() {
        if feature.kind != FeatureKind::Tree {
            continue;
        }
        let instance = ObjectInstance::new(
            feature.object_id.clone(),
            feature.root.above(),
            geometry.level_height,
            feature.rotation,
        )
        .map_err(|error| error.to_string())?;
        project_instance(view, art, &instance, true)?;
        for root in &feature.blocker_footprint {
            protect(view, root.coord, root.level, geometry.max_level);
        }
    }
    for crystal in crate::crystal_render::prepare_presentations(
        geometry.level_height,
        Some(presentation),
        Some(art),
    )
    .map_err(|error| error.to_string())?
    {
        project_instance(view, art, crystal.instance(), false)?;
    }
    // Retain existing world-owned protection: authored flow is not dynamically
    // regenerated; generated light sources and tree supports remain static.
    for liquid in presentation.liquids().keys() {
        protect(
            view,
            liquid.coord,
            geometry.min_level,
            liquid.level.saturating_add(1),
        );
    }
    for light in presentation.lights().values() {
        protect(
            view,
            light.origin.coord,
            geometry.min_level,
            geometry.max_level,
        );
    }
    for intervals in view.edit_protected.values_mut() {
        intervals.sort_unstable();
        let mut merged: Vec<(i32, i32)> = Vec::new();
        for (bottom, top) in std::mem::take(intervals) {
            if let Some(previous) = merged.last_mut() {
                if bottom <= previous.1.saturating_add(1) {
                    previous.1 = previous.1.max(top);
                    continue;
                }
            }
            merged.push((bottom, top));
        }
        *intervals = merged;
    }
    view.static_spans
        .sort_by_key(|span| (span.bottom, span.top_level));
    Ok(())
}

fn protect(view: &mut ArenaTerrainView, coord: HexCoord, bottom: i32, top: i32) {
    view.edit_protected
        .entry(coord)
        .or_default()
        .push((bottom, top));
}

/// Explicit arena object policy: woody plant parts collide; opaque/cutout canopy
/// blocks sight and projectiles. Grass and moss are excluded by their feature kind.
/// Crystal structural cells collide only where their authored style is opaque.
fn project_instance(
    view: &mut ArenaTerrainView,
    art: &RuntimeArtCatalog,
    instance: &ObjectInstance,
    tree: bool,
) -> Result<(), String> {
    let object = art
        .object(instance.object_id())
        .ok_or_else(|| format!("Missing arena object {}", instance.object_id()))?;
    for placement in &object.placements {
        let style = art
            .style(&placement.style)
            .ok_or_else(|| format!("Missing arena style {}", placement.style))?;
        let opaque = matches!(
            style.authored().surface_mode(),
            VoxelSurfaceMode::Opaque | VoxelSurfaceMode::Cutout
        );
        let woody = matches!(
            placement.part,
            ObjectPart::Plant(PlantPart::Root | PlantPart::Trunk | PlantPart::Branch)
        );
        let movement = if tree { woody } else { opaque };
        let rotated = instance
            .rotation()
            .rotate_voxel(placement.position, object.origin)
            .ok_or_else(|| "Arena object rotation overflow".to_owned())?;
        let coord = HexCoord::from_axial(
            instance.origin().coord.x() + rotated.q - object.origin.q,
            instance.origin().coord.y() + rotated.r - object.origin.r,
        );
        let level = instance.origin().level + rotated.level - object.origin.level;
        view.static_spans.push(ArenaStaticSpan {
            bottom: TilePos::new(coord, level),
            top_level: level,
            blocks_movement: movement,
            blocks_projectiles: movement || opaque,
            blocks_sight: opaque,
        });
        // New transparent occupancy remains non-blocking for ordinary queries
        // and does not expand their existing direct-edit protection policy.
        if movement || opaque {
            protect(view, coord, level, level);
        }
    }
    Ok(())
}
