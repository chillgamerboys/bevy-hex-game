#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "Tests use fixed fixture cells and require catalog, mesh, and entity preconditions"
)]

use super::*;
use crate::tests::{
    fixture_catalog, fixture_catalog_with_effect, instance, material_fixture_blueprint, settle,
    test_app,
};
use hex_assets::{EffectPart, ObjectPart, ObjectPlacement};

fn prism() -> Mesh {
    Cylinder::new(1.0, 1.0)
        .mesh()
        .resolution(6)
        .build()
        .try_transformed_by(Transform::from_rotation(Quat::from_rotation_y(TAU / 12.0)))
        .expect("hex prism")
}

fn source(cells: impl IntoIterator<Item = LocalVoxelCoord>) -> PreparedObject {
    let occupied_cell = OccupiedCell {
        style: VoxelStyleId::new("test/opaque").expect("style"),
        surface_mode: VoxelSurfaceMode::Opaque,
    };
    let mut source = PreparedObject {
        origin: LocalVoxelCoord::new(0, 0, 0),
        sections: BTreeMap::new(),
        occupied: BTreeMap::new(),
        pristine: BTreeMap::new(),
    };
    for cell in cells {
        source
            .sections
            .entry(Section::for_cell(cell))
            .or_default()
            .push(cell);
        source.occupied.insert(cell, (occupied_cell.clone(), false));
    }
    source
}

fn indices(chunks: &[(ChunkKey, Mesh)]) -> usize {
    chunks
        .iter()
        .map(|(_, mesh)| mesh.indices().map_or(0, Indices::len))
        .sum()
}

#[test]
fn carved_neighbor_exposes_new_faces_across_section_boundaries() {
    let left = LocalVoxelCoord::new(7, 0, 0);
    let right = LocalVoxelCoord::new(8, 0, 0);
    let source = source([left, right]);
    let mesh = prism();
    let before =
        bake_section(&mesh, &source, Section::for_cell(left), &BTreeSet::new()).expect("before");
    let after = bake_section(
        &mesh,
        &source,
        Section::for_cell(left),
        &BTreeSet::from([right]),
    )
    .expect("after");
    assert!(
        indices(&after) > indices(&before),
        "new cut face must be present"
    );
    let removed = bake_section(
        &mesh,
        &source,
        Section::for_cell(right),
        &BTreeSet::from([right]),
    )
    .expect("removed");
    assert!(removed.is_empty());
    let previous = RenderedCarving {
        mask: ObjectCarveMask::default(),
        sections: BTreeMap::new(),
    };
    let dirty = changed_sections(
        &source,
        &ObjectCarveMask {
            revision: 1,
            removed: BTreeSet::from([right]),
        },
        Some(&previous),
    );
    assert_eq!(
        dirty,
        BTreeSet::from([Section::for_cell(left), Section::for_cell(right)])
    );
}

#[test]
fn section_meshes_match_whole_blueprint_survivors_including_vertical_cuts() {
    let cells = [
        LocalVoxelCoord::new(7, 0, 31),
        LocalVoxelCoord::new(8, 0, 31),
        LocalVoxelCoord::new(8, 0, 32),
        LocalVoxelCoord::new(9, 0, 32),
    ];
    let source = source(cells);
    let mesh = prism();
    let catalog = fixture_catalog(0.24);
    let mut blueprint = material_fixture_blueprint();
    blueprint.origin = source.origin;
    blueprint.canopy_occluders.clear();
    for removed in [
        BTreeSet::new(),
        BTreeSet::from([cells[1]]),
        BTreeSet::from([cells[1], cells[2]]),
    ] {
        blueprint.placements = cells
            .iter()
            .filter(|cell| !removed.contains(cell))
            .map(|cell| ObjectPlacement {
                position: *cell,
                style: VoxelStyleId::new("test/opaque").expect("style"),
                part: ObjectPart::Effect(EffectPart::Core),
            })
            .collect();
        let whole = bake_blueprint(&mesh, &blueprint, &catalog).expect("whole survivors");
        let sections: Vec<_> = source
            .sections
            .keys()
            .flat_map(|section| {
                bake_section(&mesh, &source, *section, &removed).expect("section survivors")
            })
            .collect();
        assert_eq!(indices(&whole), indices(&sections));
        assert_eq!(
            whole
                .iter()
                .map(|(_, mesh)| mesh.count_vertices())
                .sum::<usize>(),
            sections
                .iter()
                .map(|(_, mesh)| mesh.count_vertices())
                .sum::<usize>()
        );
    }
}

#[test]
fn distant_sections_are_not_rebuilt_and_duplicate_mask_revisions_do_no_work() {
    let cells = [
        LocalVoxelCoord::new(0, 0, 0),
        LocalVoxelCoord::new(16, 0, 0),
        LocalVoxelCoord::new(32, 0, 0),
    ];
    let source = source(cells);
    let mask = ObjectCarveMask {
        revision: 4,
        removed: BTreeSet::from([cells[0]]),
    };
    let previous = RenderedCarving {
        mask: ObjectCarveMask::default(),
        sections: BTreeMap::new(),
    };
    assert_eq!(
        changed_sections(&source, &mask, Some(&previous)),
        BTreeSet::from([Section::for_cell(cells[0])])
    );
    let previous = RenderedCarving {
        mask: mask.clone(),
        sections: BTreeMap::new(),
    };
    assert!(changed_sections(&source, &mask, Some(&previous)).is_empty());
    let mut republished = mask;
    republished.revision += 1;
    assert!(changed_sections(&source, &republished, Some(&previous)).is_empty());
}

fn app_fixture() -> (App, Entity, Entity, Vec<LocalVoxelCoord>) {
    let cells = vec![
        LocalVoxelCoord::new(0, 0, 0),
        LocalVoxelCoord::new(1, 0, 0),
        LocalVoxelCoord::new(16, 0, 0),
        LocalVoxelCoord::new(32, 0, 0),
    ];
    let mut blueprint = material_fixture_blueprint();
    blueprint.origin = LocalVoxelCoord::new(0, 0, 0);
    blueprint.bounds.radius = 32;
    blueprint.bounds.min_level = 0;
    blueprint.bounds.height = 2;
    blueprint.canopy_occluders.clear();
    blueprint.placements = cells
        .iter()
        .map(|cell| ObjectPlacement {
            position: *cell,
            style: VoxelStyleId::new("test/opaque").expect("style"),
            part: ObjectPart::Effect(EffectPart::Core),
        })
        .collect();
    let mut app = test_app(fixture_catalog_with_effect(0.24, Some(blueprint)));
    let first = app
        .world_mut()
        .spawn((
            instance("effect/material-test", HexCoord::ORIGIN, 0, 1.0, 0),
            ObjectCarveMask::default(),
        ))
        .id();
    let second = app
        .world_mut()
        .spawn((
            instance(
                "effect/material-test",
                HexCoord::from_axial(0, 50),
                0,
                1.0,
                2,
            ),
            ObjectCarveMask::default(),
        ))
        .id();
    settle(&mut app);
    (app, first, second, cells)
}

fn handles(
    app: &App,
    entity: Entity,
) -> BTreeMap<Section, Vec<(Entity, AssetId<Mesh>, AssetId<StandardMaterial>)>> {
    app.world()
        .get::<RenderedObject>(entity)
        .expect("rendered")
        .carved
        .as_ref()
        .expect("carving")
        .sections
        .iter()
        .map(|(section, children)| {
            (
                *section,
                children
                    .iter()
                    .map(|child| {
                        (
                            *child,
                            app.world().get::<Mesh3d>(*child).expect("mesh").0.id(),
                            app.world()
                                .get::<MeshMaterial3d<StandardMaterial>>(*child)
                                .expect("material")
                                .0
                                .id(),
                        )
                    })
                    .collect(),
            )
        })
        .collect()
}

#[test]
fn one_instance_carves_only_affected_sections_and_reuses_original_materials() {
    let (mut app, first, second, cells) = app_fixture();
    let before = handles(&app, first);
    let sibling = handles(&app, second);
    for (section, chunks) in &before {
        let other = sibling.get(section).expect("shared section");
        assert_eq!(
            chunks
                .iter()
                .map(|(_, mesh, material)| (*mesh, *material))
                .collect::<Vec<_>>(),
            other
                .iter()
                .map(|(_, mesh, material)| (*mesh, *material))
                .collect::<Vec<_>>()
        );
    }
    app.world_mut().entity_mut(first).insert(ObjectCarveMask {
        revision: 1,
        removed: BTreeSet::from([cells[0]]),
    });
    settle(&mut app);
    let after = handles(&app, first);
    assert_eq!(handles(&app, second), sibling);
    for cell in cells.iter().skip(2) {
        assert_eq!(
            after.get(&Section::for_cell(*cell)),
            before.get(&Section::for_cell(*cell))
        );
    }
    let dirty = Section::for_cell(cells[0]);
    assert_ne!(after.get(&dirty), before.get(&dirty));
    assert_eq!(
        after
            .get(&dirty)
            .expect("new")
            .iter()
            .map(|(_, _, material)| *material)
            .collect::<Vec<_>>(),
        before
            .get(&dirty)
            .expect("old")
            .iter()
            .map(|(_, _, material)| *material)
            .collect::<Vec<_>>()
    );
    let stats = app
        .world()
        .get::<ObjectCarveRenderStats>(first)
        .expect("stats");
    assert_eq!(stats.rebuilt_sections, 1);
    assert_eq!(stats.considered_cells, 2);
    assert_eq!(stats.total_sections, 3);
    settle(&mut app);
    assert_eq!(
        handles(&app, first),
        after,
        "idle update preserves every child and handle"
    );
}

#[test]
fn restoring_mask_reuses_shared_meshes_and_removal_retires_unique_assets() {
    let (mut app, first, second, cells) = app_fixture();
    app.world_mut().entity_mut(first).insert(ObjectCarveMask {
        revision: 1,
        removed: BTreeSet::from([cells[0]]),
    });
    settle(&mut app);
    let carved = handles(&app, first);
    let dirty = Section::for_cell(cells[0]);
    let unique = carved
        .get(&dirty)
        .expect("dirty")
        .iter()
        .map(|(_, mesh, _)| *mesh)
        .collect::<Vec<_>>();
    app.world_mut().entity_mut(first).insert(ObjectCarveMask {
        revision: 2,
        removed: BTreeSet::new(),
    });
    settle(&mut app);
    let restored = handles(&app, first);
    let sibling = handles(&app, second);
    assert_eq!(
        restored
            .get(&dirty)
            .expect("restored")
            .iter()
            .map(|(_, mesh, _)| *mesh)
            .collect::<Vec<_>>(),
        sibling
            .get(&dirty)
            .expect("pristine")
            .iter()
            .map(|(_, mesh, _)| *mesh)
            .collect::<Vec<_>>()
    );
    assert!(unique
        .iter()
        .all(|id| app.world().resource::<Assets<Mesh>>().get(*id).is_none()));
    app.world_mut().entity_mut(first).insert(ObjectCarveMask {
        revision: 3,
        removed: BTreeSet::from([cells[1]]),
    });
    settle(&mut app);
    assert!(app
        .world()
        .resource::<ObjectRenderCache>()
        .carved
        .unique
        .contains_key(&first));
    app.world_mut().despawn(first);
    settle(&mut app);
    assert!(!app
        .world()
        .resource::<ObjectRenderCache>()
        .carved
        .unique
        .contains_key(&first));
    assert_eq!(handles(&app, second), sibling);
}

#[test]
fn removing_every_cell_keeps_the_empty_object_root() {
    let (mut app, first, _, cells) = app_fixture();
    app.world_mut().entity_mut(first).insert(ObjectCarveMask {
        revision: 1,
        removed: cells.into_iter().collect(),
    });
    settle(&mut app);
    assert!(app.world().get::<ObjectInstance>(first).is_some());
    assert!(app
        .world()
        .get::<RenderedObject>(first)
        .expect("root")
        .children
        .is_empty());
    assert_eq!(
        app.world()
            .get::<ObjectCarveRenderStats>(first)
            .expect("stats")
            .rebuilt_sections,
        3
    );
}
