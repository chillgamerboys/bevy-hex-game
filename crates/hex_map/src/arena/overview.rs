//! A disposable north-up atlas of the accepted, pristine world recipe.
//!
//! This is geography, not a live camera or a discovery authority. Ordinary terrain
//! edits intentionally leave it unchanged: rebuilding happens only at the world
//! selection/reset/package boundary, never once per destroyed voxel or frame.

use hex_assets::{LocalVoxelCoord, ObjectAssetId, ObjectPart, PlantPart};
use hex_core::arena::{ArenaMap, ArenaOverview, ArenaPackageIdentity};
use hex_core::config::{HEX_CIRCUMRADIUS, HEX_SMALL_DIAMETER};

use super::*;

const SIDE: u16 = 384;

#[derive(Debug, PartialEq, Eq)]
struct CacheKey {
    generation: u64,
    map: ArenaMap,
    package: Option<ArenaPackageIdentity>,
    art: u64,
}

#[derive(Resource, Default)]
struct OverviewCache {
    key: Option<CacheKey>,
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<ArenaOverview>()
        .init_resource::<OverviewCache>()
        .add_systems(
            Startup,
            publish
                .after(initialize)
                .in_set(ArenaSystems::PublishTerrain)
                .run_if(resource_exists::<RuntimeArtCatalog>),
        )
        .add_systems(
            ArenaTick,
            publish
                .after(publish_terrain)
                .in_set(ArenaSystems::PublishTerrain)
                .run_if(resource_exists::<RuntimeArtCatalog>),
        );
}

fn publish(
    state: Res<ArenaWorldState>,
    substances: Res<SubstanceTable>,
    art: Res<RuntimeArtCatalog>,
    mut cache: ResMut<OverviewCache>,
    mut overview: ResMut<ArenaOverview>,
) {
    let Some(recipe) = &state.original else {
        return;
    };
    if cache.key.as_ref().is_some_and(|key| {
        key.generation == state.generation
            && key.map == recipe.view.selection.map
            && key.package.as_ref() == recipe.view.package_identity.as_ref()
            && key.art == art.combined_fingerprint()
    }) {
        return;
    }
    *overview = build(recipe, &substances, &art, state.generation);
    cache.key = Some(CacheKey {
        generation: state.generation,
        map: recipe.view.selection.map,
        package: recipe.view.package_identity.clone(),
        art: art.combined_fingerprint(),
    });
}

#[derive(Clone, Copy)]
struct Surface {
    level: i32,
    color: Vec3,
    foliage: bool,
}

#[derive(Clone, Copy)]
struct LocalSurface {
    position: LocalVoxelCoord,
    color: Vec3,
    foliage: bool,
}

fn build(
    recipe: &worlds::WorldRecipe,
    substances: &SubstanceTable,
    art: &RuntimeArtCatalog,
    generation: u64,
) -> ArenaOverview {
    let empty = ArenaOverview {
        generation,
        ..default()
    };
    if recipe.view.selection.map != ArenaMap::ForestMassif {
        return empty;
    }
    // Collapsing the stack is confined to this overhead picture. Collision and
    // routes continue to use their exact, stack-safe supporting voxels.
    let terrain: BTreeMap<_, _> = recipe
        .map
        .columns()
        .filter_map(|(coord, column)| {
            let level = column.surface()?;
            let (r, g, b) = substances.get(column.get(level))?.color;
            Some((
                coord,
                Surface {
                    level,
                    color: Vec3::new(r, g, b),
                    foliage: false,
                },
            ))
        })
        .collect();
    let Some((min, max)) = bounds(&terrain) else {
        return empty;
    };
    let objects = object_surfaces(recipe, art);
    let paths = route_surfaces(recipe);
    let mut rgba = Vec::with_capacity(usize::from(SIDE) * usize::from(SIDE) * 4);
    let extent = max - min;
    for row in 0..SIDE {
        for column in 0..SIDE {
            let uv = Vec2::new(f32::from(column) + 0.5, f32::from(row) + 0.5) / f32::from(SIDE);
            let point = min + extent * uv;
            let coord = HexCoord::from_world(Vec3::new(point.x, 0.0, point.y));
            let Some(ground) = terrain.get(&coord) else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
                continue;
            };
            let shade = relief(coord, ground.level, &terrain, recipe.geometry);
            let mut color = ground.color * shade;
            if let Some(object) = objects
                .get(&coord)
                .filter(|object| object.level > ground.level)
            {
                // A little terrain shows through the canopy in the atlas so
                // hills and clearings remain readable beneath dense foliage.
                let opacity = if object.foliage { 0.82 } else { 1.0 };
                color = color.lerp(object.color * shade, opacity);
            }
            if paths
                .get(&coord)
                .is_some_and(|level| *level >= ground.level)
            {
                // Only actual validated walk ribbons receive the subdued trail
                // ink. No encounter anchors or discovery symbols are consulted.
                color = color.lerp(Vec3::new(0.66, 0.59, 0.42), 0.55);
            }
            rgba.extend_from_slice(&rgba8(color));
        }
    }
    ArenaOverview {
        generation,
        width: u32::from(SIDE),
        height: u32::from(SIDE),
        min,
        max,
        rgba,
    }
}

fn bounds(terrain: &BTreeMap<HexCoord, Surface>) -> Option<(Vec2, Vec2)> {
    let mut min = Vec2::splat(f32::INFINITY);
    let mut max = Vec2::splat(f32::NEG_INFINITY);
    let half_hex = Vec2::new(HEX_SMALL_DIAMETER * 0.5, HEX_CIRCUMRADIUS);
    for coord in terrain.keys() {
        let at = coord.to_world(0.0);
        let center = Vec2::new(at.x, at.z);
        min = min.min(center - half_hex);
        max = max.max(center + half_hex);
    }
    (!terrain.is_empty()).then_some((min, max))
}

fn object_surfaces(
    recipe: &worlds::WorldRecipe,
    art: &RuntimeArtCatalog,
) -> BTreeMap<HexCoord, Surface> {
    let mut local = BTreeMap::<ObjectAssetId, Vec<LocalSurface>>::new();
    let mut result = BTreeMap::new();
    for feature in recipe.presentation.features().values() {
        let Some(object) = art.object(&feature.object_id) else {
            continue;
        };
        let footprint = local.entry(feature.object_id.clone()).or_insert_with(|| {
            let mut columns = BTreeMap::<(i32, i32), LocalSurface>::new();
            for placement in &object.placements {
                let Some(style) = art.style(&placement.style) else {
                    continue;
                };
                let surface = LocalSurface {
                    position: placement.position,
                    color: Vec3::from_array(style.base_color().to_array()),
                    foliage: matches!(placement.part, ObjectPart::Plant(PlantPart::Foliage)),
                };
                let key = (placement.position.q, placement.position.r);
                if columns
                    .get(&key)
                    .is_none_or(|old| old.position.level < surface.position.level)
                {
                    columns.insert(key, surface);
                }
            }
            columns.into_values().collect()
        });
        for surface in footprint {
            let Some(rotated) = feature
                .rotation
                .rotate_voxel(surface.position, object.origin)
            else {
                continue;
            };
            let coord = HexCoord::from_axial(
                feature.root.coord.x() + rotated.q - object.origin.q,
                feature.root.coord.y() + rotated.r - object.origin.r,
            );
            let world_surface = Surface {
                level: feature.root.level + 1 + rotated.level - object.origin.level,
                color: surface.color,
                foliage: surface.foliage,
            };
            if result
                .get(&coord)
                .is_none_or(|old: &Surface| old.level < world_surface.level)
            {
                result.insert(coord, world_surface);
            }
        }
    }
    result
}

fn route_surfaces(recipe: &worlds::WorldRecipe) -> BTreeMap<HexCoord, i32> {
    let mut paths = BTreeMap::<HexCoord, i32>::new();
    if let Some(sites) = &recipe.view.expedition {
        for surface in sites.routes.values().flat_map(|route| &route.ribbon) {
            paths
                .entry(surface.coord)
                .and_modify(|level| *level = (*level).max(surface.level))
                .or_insert(surface.level);
        }
    }
    paths
}

fn relief(
    coord: HexCoord,
    level: i32,
    terrain: &BTreeMap<HexCoord, Surface>,
    geometry: ArenaVoxelGeometry,
) -> f32 {
    let height = |q, r| {
        let at = HexCoord::from_axial(coord.x() + q, coord.y() + r);
        let level = terrain.get(&at).map_or(level, |surface| surface.level);
        geometry.top(TilePos::new(at, level))
    };
    let normal = Vec3::new(
        (height(-1, 0) - height(1, 0)) * 0.22,
        1.0,
        (height(0, -1) - height(0, 1)) * 0.22,
    )
    .normalize();
    0.85 + 0.20 * normal.dot(Vec3::new(-0.45, 0.80, -0.40).normalize())
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "finite accepted sRGB colors are explicitly clamped and rounded to bytes"
)]
fn rgba8(color: Vec3) -> [u8; 4] {
    let [r, g, b] = color
        .to_array()
        .map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8);
    [r, g, b, 255]
}

#[cfg(test)]
mod tests;
