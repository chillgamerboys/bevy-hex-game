use std::sync::OnceLock;

use hex_assets::{HexObjectRotation, ObjectInstance};
use hex_core::arena::{
    ArenaDeploymentRegion, ArenaEncounterSite, ArenaExpeditionSites, ArenaFountainVolume,
};

use crate::procedural_v3::{FeatureId, FeatureKind, MapPresentationProjection, PlannedFeature};

use super::*;

fn content() -> &'static Content {
    static CONTENT: OnceLock<Content> = OnceLock::new();
    CONTENT.get_or_init(|| load_content().expect("accepted arena content"))
}

fn recipe() -> worlds::WorldRecipe {
    let content = content();
    let mut map = VoxelMap::new();
    for coord in HexCoord::ORIGIN.within_radius(12) {
        map.insert_column(coord, Column::filled(content.materials.grass, 9));
    }
    worlds::WorldRecipe {
        map,
        geometry: ArenaVoxelGeometry::default(),
        view: ArenaTerrainView {
            selection: ArenaSelection {
                map: ArenaMap::ForestMassif,
                ..default()
            },
            ..default()
        },
        presentation: MapPresentationProjection::default(),
        forest_source: None,
    }
}

fn image(recipe: &worlds::WorldRecipe) -> ArenaOverview {
    build(recipe, &content().substances, &content().art, 17)
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "test coordinates lie inside this 384-pixel image"
)]
fn pixel(image: &ArenaOverview, coord: HexCoord) -> (usize, [u8; 4]) {
    let at = coord.to_world(0.0);
    let uv = (Vec2::new(at.x, at.z) - image.min) / (image.max - image.min);
    let x = (uv.x * f32::from(SIDE)).floor() as usize;
    let row = (uv.y * f32::from(SIDE)).floor() as usize;
    let index = (row * usize::from(SIDE) + x) * 4;
    let bytes = image.rgba.get(index..index + 4).expect("pixel in image");
    (row, bytes.try_into().expect("RGBA pixel"))
}

#[test]
fn default_and_other_selections_are_empty() {
    let empty = ArenaOverview::default();
    assert!(empty.rgba.is_empty());
    assert_eq!((empty.width, empty.height, empty.generation), (0, 0, 0));
    let mut recipe = recipe();
    for map in [ArenaMap::Duel, ArenaMap::Fort, ArenaMap::SevenRegions] {
        recipe.view.selection.map = map;
        let empty = image(&recipe);
        assert!(empty.rgba.is_empty());
        assert_eq!((empty.width, empty.height, empty.generation), (0, 0, 17));
    }
}

#[test]
fn raster_covers_finite_hex_extent_and_north_is_first() {
    let recipe = recipe();
    let overview = image(&recipe);
    assert_eq!((overview.width, overview.height), (384, 384));
    assert_eq!(overview.rgba.len(), 384 * 384 * 4);
    assert_eq!(overview.rgba.first(), Some(&0));
    assert_eq!(overview.rgba.get(3), Some(&0));
    let margin = Vec2::new(HEX_SMALL_DIAMETER * 0.5, HEX_CIRCUMRADIUS);
    for coord in HexCoord::ORIGIN.within_radius(12) {
        let at = coord.to_world(0.0);
        let center = Vec2::new(at.x, at.z);
        assert!((center - margin)
            .cmpge(overview.min - Vec2::splat(0.0001))
            .all());
        assert!((center + margin)
            .cmple(overview.max + Vec2::splat(0.0001))
            .all());
        let (_, [_, _, _, alpha]) = pixel(&overview, coord);
        assert_eq!(alpha, 255);
    }
    assert!(
        pixel(&overview, HexCoord::from_axial(0, -8)).0
            < pixel(&overview, HexCoord::from_axial(0, 8)).0
    );
}

#[test]
fn highest_actual_material_wins_over_water_in_a_bridge_stack() {
    let mut recipe = recipe();
    let water = content().substances.id("water").expect("water material");
    recipe.map.set(TilePos::new(HexCoord::ORIGIN, 10), water);
    let river = image(&recipe);
    recipe.map.set(
        TilePos::new(HexCoord::ORIGIN, 13),
        content().materials.stone,
    );
    let bridge = image(&recipe);
    assert_ne!(
        pixel(&river, HexCoord::ORIGIN).1,
        pixel(&bridge, HexCoord::ORIGIN).1
    );
    assert_eq!(image(&recipe).rgba, bridge.rgba);
    assert_eq!(recipe.map.get(TilePos::new(HexCoord::ORIGIN, 10)), water);
    assert!(recipe.map.get(TilePos::new(HexCoord::ORIGIN, 11)).is_air());
}

#[test]
fn terrain_relief_changes_shading_without_inventing_materials() {
    let mut recipe = recipe();
    let flat = image(&recipe);
    recipe.map.set(
        TilePos::new(HexCoord::from_axial(-1, 0), 30),
        content().materials.grass,
    );
    let hill = image(&recipe);
    assert_ne!(
        pixel(&flat, HexCoord::ORIGIN).1,
        pixel(&hill, HexCoord::ORIGIN).1
    );
    assert_eq!(
        pixel(&flat, HexCoord::from_axial(7, 0)).1,
        pixel(&hill, HexCoord::from_axial(7, 0)).1
    );
}

#[test]
fn accepted_rotated_tree_footprint_matches_authoritative_object_projection() {
    let mut recipe = recipe();
    let feature = PlannedFeature {
        root: TilePos::new(HexCoord::from_axial(3, -2), 8),
        kind: FeatureKind::Tree,
        object_id: ObjectAssetId::new("plant/forest-expedition-understory-broadleaf-1")
            .expect("id"),
        rotation: HexObjectRotation::new(2).expect("rotation"),
        blocker_footprint: BTreeSet::new(),
    };
    recipe.presentation = MapPresentationProjection::from_snapshot_parts(
        BTreeMap::new(),
        [(FeatureId(0), feature.clone())].into(),
        BTreeMap::new(),
    );
    let instance = ObjectInstance::new(
        feature.object_id,
        feature.root.above(),
        recipe.geometry.level_height,
        feature.rotation,
    )
    .expect("accepted instance");
    let mut physical = ArenaTerrainView::default();
    worlds::project_instance(&mut physical, &content().art, &instance, true)
        .expect("physical projection");
    let mut expected = BTreeMap::<HexCoord, i32>::new();
    for span in physical.static_spans {
        expected
            .entry(span.bottom.coord)
            .and_modify(|level| *level = (*level).max(span.top_level))
            .or_insert(span.top_level);
    }
    let actual: BTreeMap<_, _> = object_surfaces(&recipe, &content().art)
        .into_iter()
        .map(|(coord, surface)| (coord, surface.level))
        .collect();
    assert_eq!(actual, expected);
    let forest = image(&recipe);
    recipe.presentation = MapPresentationProjection::default();
    assert_ne!(forest.rgba, image(&recipe).rgba);
}

#[test]
fn encounter_and_fountain_metadata_do_not_reveal_markers() {
    let mut recipe = recipe();
    let pristine = image(&recipe);
    let at = TilePos::new(HexCoord::from_axial(4, 1), 8);
    recipe
        .view
        .anchors
        .insert("hidden_dragon".into(), Vec3::new(5.0, 3.0, 4.0));
    recipe.view.expedition = Some(ArenaExpeditionSites {
        encounters: [(
            "hidden_shadow".into(),
            ArenaEncounterSite {
                deployment: ArenaDeploymentRegion {
                    preferred: at,
                    surfaces: [at].into(),
                },
                rally_entry: None,
            },
        )]
        .into(),
        fountains: [(
            "hidden_fountain".into(),
            ArenaFountainVolume {
                cells: [at.above()].into(),
            },
        )]
        .into(),
        ..default()
    });
    assert_eq!(pristine.rgba, image(&recipe).rgba);
}

#[test]
fn trail_ink_uses_published_ribbons_but_cannot_replace_an_overhead_surface() {
    let mut recipe = recipe();
    let before = image(&recipe);
    let at = TilePos::new(HexCoord::ORIGIN, 8);
    recipe.view.expedition = Some(ArenaExpeditionSites {
        routes: [(
            "path".into(),
            hex_core::arena::ArenaExpeditionRoute {
                from: "a".into(),
                to: "b".into(),
                clearance_levels: 4,
                supports: vec![at],
                ribbon: [at].into(),
            },
        )]
        .into(),
        ..default()
    });
    let path = image(&recipe);
    assert_ne!(pixel(&path, at.coord).1, pixel(&before, at.coord).1);
    assert_eq!(
        pixel(&path, HexCoord::from_axial(4, 4)).1,
        pixel(&before, HexCoord::from_axial(4, 4)).1
    );
    recipe
        .map
        .set(TilePos::new(at.coord, 20), content().materials.stone);
    let covered_path = image(&recipe);
    recipe.view.expedition = None;
    assert_eq!(covered_path.rgba, image(&recipe).rgba);
}

#[test]
fn cache_ignores_dirty_terrain_and_refreshes_on_reset_package_or_selection() {
    let mut app = App::new();
    app.insert_resource(content().substances.clone())
        .insert_resource(content().art.clone())
        .insert_resource(ArenaWorldState {
            original: Some(recipe()),
            ..default()
        })
        .init_resource::<OverviewCache>()
        .init_resource::<ArenaOverview>()
        .add_systems(Update, publish);
    app.update();
    let old_ptr = app.world().resource::<ArenaOverview>().rgba.as_ptr();
    {
        let mut state = app.world_mut().resource_mut::<ArenaWorldState>();
        state.changed.insert(HexCoord::ORIGIN);
        let recipe = state.original.as_mut().expect("recipe");
        recipe.map.set(
            TilePos::new(HexCoord::ORIGIN, 40),
            content().materials.stone,
        );
        recipe.view.revision += 1;
    }
    app.update();
    assert_eq!(
        old_ptr,
        app.world().resource::<ArenaOverview>().rgba.as_ptr()
    );
    app.world_mut().resource_mut::<ArenaWorldState>().generation += 1;
    app.update();
    assert_ne!(
        old_ptr,
        app.world().resource::<ArenaOverview>().rgba.as_ptr()
    );
    assert_eq!(app.world().resource::<ArenaOverview>().generation, 1);
    let reset_ptr = app.world().resource::<ArenaOverview>().rgba.as_ptr();
    app.world_mut()
        .resource_mut::<ArenaWorldState>()
        .original
        .as_mut()
        .expect("recipe")
        .view
        .package_identity = Some(ArenaPackageIdentity {
        world_id: "different-package".into(),
        manifest_fingerprint: 42,
        sites_fingerprint: Some(43),
    });
    app.update();
    assert_ne!(
        reset_ptr,
        app.world().resource::<ArenaOverview>().rgba.as_ptr()
    );
    app.world_mut()
        .resource_mut::<ArenaWorldState>()
        .original
        .as_mut()
        .expect("recipe")
        .view
        .selection
        .map = ArenaMap::Duel;
    app.update();
    assert!(app.world().resource::<ArenaOverview>().rgba.is_empty());
}

#[test]
fn v4_overview_preserves_package_colors_after_collision_material_aliasing() {
    use hex_world_contracts::{
        ChunkDescriptor, ChunkPackage, ChunkSemantics, ColumnData, MaterialSpec, RegionDescriptor,
        Seal, VoxelRun, WorldHex, WorldManifest, WorldPackage, SCHEMA_VERSION,
    };
    use hex_world_runtime::MemoryChunkSource;

    // One validated chunk, with the accepted expedition colors. Moss/grass and
    // pine-floor/soil deliberately share collision policies but not appearance.
    let palette = [
        ("basalt", [76, 88, 101, 255]),
        ("grass", [105, 141, 72, 255]),
        ("moss", [49, 91, 57, 255]),
        ("pine-floor", [78, 98, 60, 255]),
        ("soil", [98, 76, 49, 255]),
        ("stone", [105, 112, 116, 255]),
        ("water", [42, 115, 144, 190]),
    ];
    let origin = WorldHex::new(8, 8);
    let expected: BTreeMap<_, _> = HexCoord::from_axial(8, 8)
        .within_radius(1)
        .zip(palette)
        .collect();
    let mut package = WorldPackage {
        manifest: WorldManifest {
            schema_version: SCHEMA_VERSION,
            world_id: "overview-palette".into(),
            compiler_version: "test".into(),
            source_fingerprint: 1,
            materials: palette
                .into_iter()
                .map(|(name, color)| MaterialSpec {
                    id: name.into(),
                    solid: name != "water",
                    diggable: name != "water",
                    color,
                })
                .collect(),
            regions: vec![RegionDescriptor {
                id: "palette".into(),
                origin,
                radius: 1,
                source_fingerprint: 1,
            }],
            chunks: vec![ChunkDescriptor {
                coordinate: origin.chunk(),
                fingerprint: 0,
                path: "chunks/0_0.ron".into(),
            }],
            boundaries: Vec::new(),
            summary: Vec::new(),
            features: Vec::new(),
            fingerprint: 0,
        },
        chunks: [(
            origin.chunk(),
            ChunkPackage {
                schema_version: SCHEMA_VERSION,
                world_id: "overview-palette".into(),
                coordinate: origin.chunk(),
                source_fingerprint: 1,
                columns: expected
                    .iter()
                    .map(|(coord, (name, _))| ColumnData {
                        position: WorldHex::new(i64::from(coord.x()), i64::from(coord.y())),
                        runs: vec![
                            VoxelRun {
                                bottom: 0,
                                top: 4,
                                material: "stone".into(),
                            },
                            VoxelRun {
                                bottom: 7,
                                top: 9,
                                material: (*name).into(),
                            },
                        ],
                    })
                    .collect(),
                features: Vec::new(),
                semantics: ChunkSemantics::default(),
                fingerprint: 0,
            },
        )]
        .into(),
    };
    package.seal().expect("small V4 fixture seals");
    let source = MemoryChunkSource::new(package).expect("validated V4 fixture");
    let mut surfaces: BTreeMap<_, _> = expected
        .iter()
        .map(|(coord, (name, _))| {
            let policy =
                forest::material_id(name, &content().substances).expect("collision policy");
            let (r, g, b) = content().substances.get(policy).expect("substance").color;
            (
                *coord,
                Surface {
                    level: 8,
                    color: Vec3::new(r, g, b),
                    foliage: false,
                },
            )
        })
        .collect();
    let old_bounds = bounds(&surfaces);
    assert_eq!(
        forest::material_id("grass", &content().substances).expect("grass"),
        forest::material_id("moss", &content().substances).expect("moss"),
    );
    let basalt = expected
        .iter()
        .find(|(_, (name, _))| *name == "basalt")
        .map(|(coord, _)| *coord)
        .expect("basalt column");
    assert_ne!(
        rgba8(surfaces.get(&basalt).expect("basalt").color),
        [76, 88, 101, 255],
        "the old generic palette demonstrably differs"
    );
    apply_package_palette(&mut surfaces, &source).expect("package display colors");
    assert_eq!(surfaces.len(), 7);
    assert_eq!(bounds(&surfaces), old_bounds);
    for (coord, (_, [r, g, b, _])) in expected {
        let surface = surfaces.get(&coord).expect("same surface coordinate");
        assert_eq!(surface.level, 8);
        assert!(!surface.foliage);
        assert_eq!(rgba8(surface.color), [r, g, b, 255]);
    }
    // Resolve the run containing the chosen surface; neither a buried material
    // nor a guessed upper run may replace that exact stack-safe height.
    surfaces.get_mut(&basalt).expect("basalt").level = 3;
    apply_package_palette(&mut surfaces, &source).expect("lower selected surface");
    assert_eq!(
        rgba8(surfaces.get(&basalt).expect("lower stone").color),
        [105, 112, 116, 255]
    );
    surfaces.get_mut(&basalt).expect("basalt").level = 6;
    assert!(apply_package_palette(&mut surfaces, &source).is_err());
}
