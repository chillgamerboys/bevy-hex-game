#![expect(
    clippy::expect_used,
    reason = "test fixture failures require specific diagnostics"
)]

use std::{collections::BTreeMap, sync::Arc};

use bevy::{core_pipeline::oit::OrderIndependentTransparencySettings, prelude::*};
use hex_assets::ObjectBlueprint;
use hex_core::{HexCoord, TilePos};
use hex_world_contracts::{ColumnData, ObjectInstance, VoxelPosition, VoxelRun, WorldHex};

use super::*;

fn origin(q: i32, r: i32, level: i32) -> TilePos {
    TilePos {
        coord: HexCoord::from_axial(q, r),
        level,
    }
}

fn record(blueprint: &ObjectBlueprint, id: &str, rotation: u8) -> ObjectInstance {
    let root = VoxelPosition {
        column: WorldHex::new(9_000_000_000_000, -9_000_000_000_000),
        level: 1_000_000,
    };
    let mut columns = BTreeMap::<WorldHex, Vec<VoxelRun>>::new();
    for placement in &blueprint.placements {
        let offset = WorldHex::new(
            i64::from(placement.position.q) - i64::from(blueprint.origin.q),
            i64::from(placement.position.r) - i64::from(blueprint.origin.r),
        )
        .rotate_60(rotation)
        .expect("fixture rotation");
        let level = root.level + placement.position.level - blueprint.origin.level;
        columns
            .entry(
                root.column
                    .checked_add(offset)
                    .expect("fixture translation"),
            )
            .or_default()
            .push(VoxelRun {
                bottom: level,
                top: level + 1,
                material: "stone".into(),
            });
    }
    ObjectInstance {
        id: id.into(),
        region_id: "region-a".into(),
        asset: blueprint.id.to_string(),
        origin: root,
        rotation,
        occupancy: columns
            .into_iter()
            .map(|(position, runs)| {
                let mut column = ColumnData { position, runs };
                column.seal().expect("fixture canonical column");
                column
            })
            .collect(),
    }
}

fn plant(id: &str, rotation: u8) -> ObjectInstance {
    record(&crate::tests::fixture_blueprint(), id, rotation)
}

fn presenter(limits: ObjectPresentationLimits) -> ResidentObjectPresenter {
    ResidentObjectPresenter::with_limits(
        Arc::new(crate::tests::fixture_catalog(0.24)),
        Mesh::from(Cuboid::new(1.0, 1.0, 1.0)),
        0.5,
        limits,
    )
    .expect("valid presenter")
}

#[test]
fn exact_rotated_footprints_admit_all_six_turns_and_reject_stale_shapes() {
    let presenter = presenter(ObjectPresentationLimits::default());
    for rotation in 0..6 {
        let object = plant("tree", rotation);
        let prepared = presenter
            .prepare(&object, 4, origin(1, -2, 3))
            .expect("exact rotation");
        assert_eq!(prepared.voxels(), 4);
        assert_eq!(prepared.object(), &object);
        let mut wrong_rotation = object.clone();
        wrong_rotation.rotation = (rotation + 1) % 6;
        assert!(presenter
            .prepare(&wrong_rotation, 4, origin(1, -2, 3))
            .is_err());
        let mut wrong_height = object;
        wrong_height
            .occupancy
            .first_mut()
            .expect("column")
            .runs
            .first_mut()
            .expect("run")
            .top += 1;
        assert!(presenter
            .prepare(&wrong_height, 4, origin(1, -2, 3))
            .is_err());
    }
}

#[test]
fn independent_rotation_oracle_checks_the_expected_global_axial_offset() {
    // Plant leaf (+2,0) rotated twice is (-2,+2), independently of the helper.
    let object = plant("tree", 2);
    let column = object
        .origin
        .column
        .checked_add(WorldHex::new(-2, 2))
        .expect("offset");
    assert!(object
        .occupancy
        .iter()
        .any(|occupied| occupied.position == column));
    let presenter = presenter(ObjectPresentationLimits::default());
    let prepared = presenter
        .prepare(&object, 1, origin(-5, 6, 0))
        .expect("matched footprint");
    assert_eq!(prepared.object().origin, object.origin);
}

#[test]
fn unknown_assets_and_noncanonical_occupancy_remain_unresolved() {
    let presenter = presenter(ObjectPresentationLimits::default());
    let mut unknown = plant("tree", 0);
    unknown.asset = "procedural/limestone-tower".into();
    assert!(presenter.prepare(&unknown, 0, origin(0, 0, 0)).is_err());
    let mut noncanonical = plant("tree", 0);
    noncanonical.occupancy.reverse();
    assert!(presenter
        .prepare(&noncanonical, 0, origin(0, 0, 0))
        .is_err());
}

#[test]
fn shared_mesh_users_survive_local_replacement_and_free_on_last_removal() {
    let mut presenter = presenter(ObjectPresentationLimits::default());
    let mut world = World::new();
    let first = plant("first", 0);
    let second = plant("second", 3);
    let a = presenter
        .prepare(&first, 1, origin(0, 0, 0))
        .expect("first prepare");
    let a = presenter.publish(&mut world, a).expect("first publish");
    let b = presenter
        .prepare(&second, 1, origin(10, 0, 0))
        .expect("second prepare");
    let b = presenter.publish(&mut world, b).expect("second publish");
    assert_eq!(world.resource::<Assets<Mesh>>().len(), a.meshes);
    assert_eq!(a.meshes, b.meshes);
    assert_eq!(presenter.cached_asset_count(), 1);
    assert_eq!(presenter.cached_material_count(), 2);
    let handles: Vec<_> = world.resource::<Assets<Mesh>>().ids().collect();
    let replaced = presenter
        .prepare(&first, 2, origin(0, 0, 0))
        .expect("replacement prepare");
    let replaced = presenter
        .publish(&mut world, replaced)
        .expect("replacement publish");
    assert_ne!(replaced.root, a.root);
    assert!(world.get_entity(a.root).is_err());
    assert!(world.get_entity(b.root).is_ok());
    assert_eq!(
        world.resource::<Assets<Mesh>>().ids().collect::<Vec<_>>(),
        handles
    );
    presenter.remove(&mut world, "first").expect("remove first");
    assert_eq!(world.resource::<Assets<Mesh>>().len(), b.meshes);
    presenter
        .remove(&mut world, "second")
        .expect("remove second");
    assert_eq!(world.resource::<Assets<Mesh>>().len(), 0);
    assert_eq!(world.resource::<Assets<StandardMaterial>>().len(), 0);
    assert_eq!(world.query::<&ResidentObject>().iter(&world).count(), 0);
    assert_eq!(world.query::<&ResidentObjectPart>().iter(&world).count(), 0);
}

#[test]
fn revision_conflicts_and_foreign_products_leave_current_roots_intact() {
    let mut presenter = presenter(ObjectPresentationLimits::default());
    let mut world = World::new();
    let object = plant("tree", 0);
    let ready = presenter
        .prepare(&object, 5, origin(0, 0, 0))
        .expect("prepare");
    let receipt = presenter.publish(&mut world, ready).expect("publish");
    let ready = presenter
        .prepare(&object, 5, origin(0, 0, 0))
        .expect("idempotent prepare");
    assert_eq!(
        presenter
            .publish(&mut world, ready)
            .expect("idempotent publish"),
        receipt
    );
    let ready = presenter
        .prepare(&object, 4, origin(0, 0, 0))
        .expect("older prepare");
    assert!(presenter.publish(&mut world, ready).is_err());
    let mut changed = object.clone();
    changed
        .occupancy
        .first_mut()
        .expect("column")
        .runs
        .first_mut()
        .expect("run")
        .material = "timber".into();
    let ready = presenter
        .prepare(&changed, 5, origin(0, 0, 0))
        .expect("material prepare");
    assert!(presenter.publish(&mut world, ready).is_err());
    let foreign = super::tests::presenter(ObjectPresentationLimits::default());
    let ready = foreign
        .prepare(&object, 6, origin(0, 0, 0))
        .expect("foreign prepare");
    assert!(presenter.publish(&mut world, ready).is_err());
    assert_eq!(presenter.receipts().next(), Some(&receipt));
}

#[test]
fn rebase_retains_identity_and_assets_invalidates_jobs_and_is_failure_atomic() {
    let mut presenter = presenter(ObjectPresentationLimits::default());
    let mut world = World::new();
    let object = plant("tree", 1);
    let ready = presenter
        .prepare(&object, 1, origin(0, 0, 0))
        .expect("prepare");
    let receipt = presenter.publish(&mut world, ready).expect("publish");
    let stale = presenter
        .prepare(&object, 2, origin(0, 0, 0))
        .expect("queued product");
    let before = *world
        .get::<Transform>(receipt.root)
        .expect("root transform");
    let bad = BTreeMap::from([("tree".into(), origin(1024, 1024, 0))]);
    assert!(presenter.rebase(&mut world, &bad).is_err());
    assert_eq!(
        *world
            .get::<Transform>(receipt.root)
            .expect("root transform"),
        before
    );
    assert!(presenter.rebase(&mut world, &BTreeMap::new()).is_err());
    let after = presenter
        .rebase(
            &mut world,
            &BTreeMap::from([("tree".into(), origin(8, -4, -12))]),
        )
        .expect("rebase");
    let after = after.first().expect("receipt");
    assert_eq!(after.root, receipt.root);
    assert_eq!(after.revision, receipt.revision);
    assert_eq!(after.fingerprint, receipt.fingerprint);
    assert_eq!(after.local_origin, origin(8, -4, -12));
    assert_eq!(presenter.object("tree"), Some(&object));
    assert_eq!(world.resource::<Assets<Mesh>>().len(), receipt.meshes);
    assert!(presenter.publish(&mut world, stale).is_err());
    assert_ne!(
        *world
            .get::<Transform>(receipt.root)
            .expect("root transform"),
        before
    );
}

#[test]
fn clear_releases_only_owned_assets_and_invalidates_prepared_products() {
    let mut presenter = presenter(ObjectPresentationLimits::default());
    let mut world = World::new();
    world.init_resource::<Assets<Mesh>>();
    world.init_resource::<Assets<StandardMaterial>>();
    let external_mesh = world
        .resource_mut::<Assets<Mesh>>()
        .add(Mesh::from(Cuboid::default()));
    let external_material = world
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());
    let external_entity = world.spawn(Name::new("unrelated")).id();
    let object = plant("tree", 0);
    let queued = presenter
        .prepare(&object, 1, origin(0, 0, 0))
        .expect("queued");
    let ready = presenter
        .prepare(&object, 1, origin(0, 0, 0))
        .expect("prepare");
    presenter.publish(&mut world, ready).expect("publish");
    presenter.clear(&mut world);
    assert!(presenter.publish(&mut world, queued).is_err());
    assert!(world
        .resource::<Assets<Mesh>>()
        .get(&external_mesh)
        .is_some());
    assert!(world
        .resource::<Assets<StandardMaterial>>()
        .get(&external_material)
        .is_some());
    assert!(world.get_entity(external_entity).is_ok());
    assert_eq!(world.resource::<Assets<Mesh>>().len(), 1);
    assert_eq!(world.resource::<Assets<StandardMaterial>>().len(), 1);
    presenter.clear(&mut world);
}

#[test]
fn full_local_footprint_is_bounded_before_any_float_transform_or_mesh_work() {
    let presenter = presenter(ObjectPresentationLimits {
        max_local_hex: 8,
        ..default()
    });
    let object = plant("tree", 0);
    assert!(presenter.prepare(&object, 1, origin(7, 0, 0)).is_err());
    assert!(presenter.prepare(&object, 1, origin(0, 0, 4096)).is_err());
    let ready = presenter
        .prepare(&object, 1, origin(-7, 0, -100))
        .expect("bounded local placement");
    assert!(ready.transform.translation.is_finite());
    assert!(ready.transform.translation.length() < 100.0);
    let tiny = super::tests::presenter(ObjectPresentationLimits {
        max_voxels_per_object: 3,
        ..default()
    });
    assert!(tiny.prepare(&object, 1, origin(0, 0, 0)).is_err());
    let tiny = super::tests::presenter(ObjectPresentationLimits {
        max_vertices_per_asset: 25,
        ..default()
    });
    assert!(tiny.prepare(&object, 1, origin(0, 0, 0)).is_err());
}

#[test]
fn cache_and_instance_limits_reject_before_replacing_valid_presentation() {
    let mut presenter = presenter(ObjectPresentationLimits {
        max_resident_objects: 1,
        max_asset_types: 1,
        ..default()
    });
    let mut world = World::new();
    let ready = presenter
        .prepare(&plant("first", 0), 1, origin(0, 0, 0))
        .expect("prepare");
    let original = presenter.publish(&mut world, ready).expect("publish");
    let ready = presenter
        .prepare(&plant("second", 0), 1, origin(1, 0, 0))
        .expect("prepare");
    assert!(presenter.publish(&mut world, ready).is_err());
    assert!(world.get_entity(original.root).is_ok());
    let effect = record(&crate::tests::material_fixture_blueprint(), "first", 0);
    let ready = presenter
        .prepare(&effect, 2, origin(0, 0, 0))
        .expect("effect prepare");
    let replacement = presenter
        .publish(&mut world, ready)
        .expect("replacement can release old sole asset at capacity");
    assert!(replacement.has_blend);
    assert_eq!(presenter.cached_asset_count(), 1);
    assert_eq!(world.resource::<Assets<Mesh>>().len(), replacement.meshes);
    assert!(world.get_entity(original.root).is_err());
    presenter.clear(&mut world);
    assert_eq!(world.resource::<Assets<StandardMaterial>>().len(), 0);
}

#[test]
fn style_modes_emission_and_typed_pick_identity_are_preserved_without_legacy_plugin() {
    let mut presenter = presenter(ObjectPresentationLimits::default());
    let mut world = World::new();
    let effect = record(&crate::tests::material_fixture_blueprint(), "glow", 0);
    let ready = presenter
        .prepare(&effect, 3, origin(0, 0, 0))
        .expect("prepare");
    let receipt = presenter.publish(&mut world, ready).expect("publish");
    let mut modes = Vec::new();
    let mut emissive = false;
    for (part, pick, material) in world
        .query::<(
            &ResidentObjectPart,
            &Pickable,
            &MeshMaterial3d<StandardMaterial>,
        )>()
        .iter(&world)
    {
        assert_eq!(part.id, "glow");
        assert_eq!(part.revision, 3);
        assert_eq!(part.fingerprint, receipt.fingerprint);
        assert!(!pick.is_hoverable && !pick.should_block_lower);
        let material = world
            .resource::<Assets<StandardMaterial>>()
            .get(&material.0)
            .expect("style material");
        modes.push(material.alpha_mode);
        emissive |= material.emissive.red > 0.0;
    }
    assert!(modes.contains(&AlphaMode::Opaque));
    assert!(modes.contains(&AlphaMode::AlphaToCoverage));
    assert!(modes.contains(&AlphaMode::Blend));
    assert!(modes.contains(&AlphaMode::Add));
    assert!(emissive);
    assert!(!world.contains_resource::<crate::ObjectRenderCache>());
}

#[test]
fn opt_in_transparency_restores_camera_msaa_after_final_blend_is_removed() {
    let mut app = App::new();
    transparency_plugin(&mut app);
    let camera = app
        .world_mut()
        .spawn((Camera3d::default(), Msaa::Sample4))
        .id();
    let mut presenter = presenter(ObjectPresentationLimits::default());
    let effect = record(&crate::tests::material_fixture_blueprint(), "glow", 0);
    let ready = presenter
        .prepare(&effect, 1, origin(0, 0, 0))
        .expect("prepare");
    presenter.publish(app.world_mut(), ready).expect("publish");
    app.update();
    assert_eq!(app.world().get::<Msaa>(camera), Some(&Msaa::Off));
    assert!(app
        .world()
        .get::<OrderIndependentTransparencySettings>(camera)
        .is_some());
    presenter.clear(app.world_mut());
    app.update();
    assert_eq!(app.world().get::<Msaa>(camera), Some(&Msaa::Sample4));
    assert!(app
        .world()
        .get::<OrderIndependentTransparencySettings>(camera)
        .is_none());
}

#[test]
fn malformed_source_mesh_is_rejected_before_any_assets_are_published() {
    let mut mesh = Mesh::from(Cuboid::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[f32::NAN, 0.0, 0.0]; 24]);
    assert!(
        ResidentObjectPresenter::new(Arc::new(crate::tests::fixture_catalog(0.24)), mesh, 0.5)
            .is_err()
    );
}
