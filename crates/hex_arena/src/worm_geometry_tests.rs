//! Exact swept geometry and current-publication burrow admission.
use super::*;
use crate::BodyHexPrism;
use hex_core::arena::ArenaStaticSpan;

fn body(feet: Vec3, offsets: impl IntoIterator<Item = Vec3>) -> PrismPose {
    let parts: Vec<_> = offsets
        .into_iter()
        .map(|offset| BodyHexPrism {
            offset,
            height: 0.4,
        })
        .collect();
    PrismPose {
        feet,
        parts: BodyPrismSnapshot::try_from_parts(&parts).expect("bounded fixture"),
    }
}

fn voxel(q: i32, r: i32, level: i32) -> TilePos {
    TilePos::new(HexCoord::from_axial(q, r), level)
}

#[test]
fn native_stationary_prism_announces_only_its_actual_cell_at_both_vertical_offsets() {
    for offset in [0.0, 0.4] {
        let g = ArenaVoxelGeometry {
            vertical_offset: offset,
            ..Default::default()
        };
        for coord in [HexCoord::ORIGIN, HexCoord::from_axial(-6, 1)] {
            let pos = TilePos::new(coord, 8);
            let pose = body(coord.to_world(g.top(pos) - g.level_height), [Vec3::ZERO]);
            assert_eq!(
                swept_prism_cells(pose, pose, g).expect("native body"),
                vec![pos]
            );
        }
    }
}

#[test]
fn translation_announces_a_thin_voxel_between_clear_endpoints() {
    let g = ArenaVoxelGeometry::default();
    let before = body(Vec3::new(-3.0, 0.4, 0.0), [Vec3::ZERO]);
    let after = body(Vec3::new(3.0, 0.4, 0.0), [Vec3::ZERO]);
    let obstacle = voxel(0, 0, 2);
    for pose in [before, after] {
        assert!(!swept_prism_cells(pose, pose, g)
            .expect("end")
            .contains(&obstacle));
    }
    let swept = swept_prism_cells(before, after, g).expect("bounded full sweep");
    assert!(swept.contains(&obstacle));
    assert!(swept
        .windows(2)
        .all(|pair| pair.first().zip(pair.last()).is_some_and(|(a, b)| a < b)));
}

#[test]
fn a_stationary_head_does_not_hide_the_turning_tail_path() {
    let g = ArenaVoxelGeometry::default();
    let spacing = hex_core::config::HEX_SMALL_DIAMETER;
    for count in [4_u8, 6] {
        let before = body(
            Vec3::Y * 0.4,
            (0..count).map(|i| Vec3::NEG_X * f32::from(i) * spacing),
        );
        let after = body(
            Vec3::Y * 0.4,
            (0..count).map(|i| Vec3::NEG_Z * f32::from(i) * spacing),
        );
        let obstacle = voxel(-1, -2, 2);
        for pose in [before, after] {
            assert!(!swept_prism_cells(pose, pose, g)
                .expect("end")
                .contains(&obstacle));
        }
        assert!(swept_prism_cells(before, after, g)
            .expect("whole turn")
            .contains(&obstacle));
    }
}

#[test]
fn emergence_checks_the_roof_between_clear_head_endpoints() {
    let g = ArenaVoxelGeometry::default();
    let before = body(Vec3::Y * 0.4, [Vec3::ZERO]);
    let after = body(Vec3::Y * 0.4, [Vec3::Y * 2.0]);
    let roof = voxel(0, 0, 4);
    for pose in [before, after] {
        assert!(!swept_prism_cells(pose, pose, g)
            .expect("end")
            .contains(&roof));
    }
    assert!(swept_prism_cells(before, after, g)
        .expect("head rise")
        .contains(&roof));
}

#[test]
fn dirt_phasing_never_admits_an_unconverted_cell_or_false_mask_object() {
    let g = ArenaVoxelGeometry::default();
    let pos = voxel(0, 0, 2);
    let pose = body(Vec3::Y * 0.4, [Vec3::ZERO]);
    let dirt = SubstanceId(3);
    let stone = SubstanceId(1);
    let policy = ArenaBurrowMaterials {
        eligible: [dirt, stone].into(),
    };
    let mut view = ArenaTerrainView {
        voxels: [(pos, dirt)].into(),
        ..Default::default()
    };
    let mut query = BurrowQuery::default();
    query.refresh(&view);
    assert!(matches!(
        query.admit(
            pose,
            pose,
            &BurrowContext {
                world: &view,
                policy: &policy,
                dirt,
                bodies: &[],
                owner: 0,
                geometry: g
            }
        ),
        Admission::Clear
    ));
    let mut normal = crate::collision::CollisionWorld::default();
    normal.refresh(&view, g);
    assert!(!normal.clear(pose.feet, 0.4, hex_core::config::HEX_SMALL_DIAMETER * 0.5));
    view.voxels.insert(pos, stone);
    view.revision += 1;
    query.refresh(&view);
    match query.admit(
        pose,
        pose,
        &BurrowContext {
            world: &view,
            policy: &policy,
            dirt,
            bodies: &[],
            owner: 0,
            geometry: g,
        },
    ) {
        Admission::NeedsConversion(volume) => assert_eq!(volume, vec![pos]),
        state => panic!("unconverted material cannot phase: {state:?}"),
    }
    view.static_spans.push(ArenaStaticSpan {
        bottom: pos,
        top_level: pos.level,
        blocks_movement: false,
        blocks_projectiles: false,
        blocks_sight: false,
    });
    view.full_rebuild = true;
    view.revision += 1;
    query.refresh(&view);
    assert!(matches!(
        query.admit(
            pose,
            pose,
            &BurrowContext {
                world: &view,
                policy: &policy,
                dirt,
                bodies: &[],
                owner: 0,
                geometry: g
            }
        ),
        Admission::Blocked(StepRejection::Static)
    ));
}

#[test]
fn protected_air_and_liquid_block_the_entire_interior_sweep() {
    let g = ArenaVoxelGeometry::default();
    let before = body(Vec3::new(-3.0, 0.4, 0.0), [Vec3::ZERO]);
    let after = body(Vec3::new(3.0, 0.4, 0.0), [Vec3::ZERO]);
    let obstacle = voxel(0, 0, 2);
    let dirt = SubstanceId(3);
    let policy = ArenaBurrowMaterials {
        eligible: [dirt].into(),
    };
    let mut view = ArenaTerrainView::default();
    view.edit_protected.insert(obstacle.coord, vec![(2, 2)]);
    let mut query = BurrowQuery::default();
    query.refresh(&view);
    assert!(matches!(
        query.admit(
            before,
            after,
            &BurrowContext {
                world: &view,
                policy: &policy,
                dirt,
                bodies: &[],
                owner: 0,
                geometry: g
            }
        ),
        Admission::Blocked(StepRejection::Protected)
    ));
    view.edit_protected.clear();
    view.liquids.push(hex_core::arena::ArenaSolidSpan {
        bottom: obstacle,
        top_level: 2,
        substance: SubstanceId(7),
    });
    view.revision += 2;
    // A missed publication must rebuild all nonterrain indexes.
    query.refresh(&view);
    assert!(matches!(
        query.admit(
            before,
            after,
            &BurrowContext {
                world: &view,
                policy: &policy,
                dirt,
                bodies: &[],
                owner: 0,
                geometry: g
            }
        ),
        Admission::Blocked(StepRejection::Liquid)
    ));
}

#[test]
fn a_live_capsule_between_clear_endpoints_blocks_without_a_centerline_shortcut() {
    let g = ArenaVoxelGeometry::default();
    let before = body(Vec3::new(-3.0, 0.4, 0.0), [Vec3::ZERO]);
    let after = body(Vec3::new(3.0, 0.4, 0.0), [Vec3::ZERO]);
    let actor = Actor::spawn(1, Vec3::Y * 0.4, Vec3::Z);
    assert!(!sweep_hits_body(before, before, &actor));
    assert!(!sweep_hits_body(after, after, &actor));
    assert!(sweep_hits_body(before, after, &actor));
    assert!(swept_prism_cells(before, after, g).is_ok());
}

#[test]
fn invalid_identity_or_unbounded_proposal_is_rejected_without_volume_truncation() {
    let g = ArenaVoxelGeometry {
        radius: 100,
        ..Default::default()
    };
    let before = body(Vec3::Y * 0.4, [Vec3::ZERO]);
    let bad_count = body(Vec3::Y * 0.4, [Vec3::ZERO, Vec3::X * 1.7]);
    assert_eq!(
        swept_prism_cells(before, bad_count, g),
        Err(StepRejection::InvalidGeometry)
    );
    let after = body(Vec3::new(50.0, 20.0, 50.0), [Vec3::ZERO]);
    assert!(matches!(
        swept_prism_cells(before, after, g),
        Err(StepRejection::CandidateBudget | StepRejection::TooManyCells)
    ));
    let below = body(Vec3::new(0.0, -1.0, 0.0), [Vec3::ZERO]);
    assert_eq!(
        swept_prism_cells(before, below, g),
        Err(StepRejection::OutsideWorld)
    );
}

#[test]
fn head_exposure_requires_one_current_surface_for_every_real_footprint_column() {
    let g = ArenaVoxelGeometry::default();
    let pose = body(Vec3::new(0.7, 1.2, 0.0), [Vec3::ZERO]);
    let columns = head_columns(pose).expect("head footprint");
    assert!(
        columns.len() > 1,
        "offset head spans multiple actual columns"
    );
    let mut supports: Vec<_> = columns
        .into_iter()
        .map(|coord| TilePos::new(coord, 2))
        .collect();
    let mut view = ArenaTerrainView {
        voxels: supports.iter().map(|pos| (*pos, SubstanceId(3))).collect(),
        ..Default::default()
    };
    let clearance = head_clearance(pose, &supports, &view, g).expect("clear raised head");
    assert!((clearance - g.level_height).abs() < SKIN);
    assert!(head_clearance(pose, supports.get(1..).expect("subset"), &view, g).is_none());
    let edge = supports.last_mut().expect("edge support");
    view.voxels.remove(edge);
    edge.level = 3;
    view.voxels.insert(*edge, SubstanceId(3));
    assert!(
        head_clearance(pose, &supports, &view, g).expect("tangent higher support")
            < g.level_height - SKIN
    );
}

#[test]
fn head_exposure_rechecks_cover_and_rejects_a_stale_lower_support_band() {
    let g = ArenaVoxelGeometry::default();
    let pose = body(Vec3::Y * 1.6, [Vec3::ZERO]);
    let old = voxel(0, 0, 2);
    let new = voxel(0, 0, 3);
    let mut view = ArenaTerrainView {
        voxels: [(old, SubstanceId(3))].into(),
        ..Default::default()
    };
    assert!((head_clearance(pose, &[old], &view, g).expect("clear") - 0.8).abs() < SKIN);
    view.voxels.insert(new, SubstanceId(1));
    assert!(head_clearance(pose, &[old], &view, g).is_none());
    assert!((head_clearance(pose, &[new], &view, g).expect("current band") - 0.4).abs() < SKIN);
    view.voxels.insert(voxel(0, 0, 5), SubstanceId(1));
    assert!(
        head_clearance(pose, &[new], &view, g).is_none(),
        "new wall covers head"
    );
}

#[test]
fn head_clearance_honors_transparent_static_occupancy_and_a_separate_high_roof() {
    let g = ArenaVoxelGeometry::default();
    let pose = body(Vec3::Y * 1.2, [Vec3::ZERO]);
    let support = voxel(0, 0, 2);
    let mut view = ArenaTerrainView {
        voxels: [(support, SubstanceId(3)), (voxel(0, 0, 7), SubstanceId(1))].into(),
        ..Default::default()
    };
    assert!(head_clearance(pose, &[support], &view, g).is_some());
    view.static_spans.push(ArenaStaticSpan {
        bottom: voxel(0, 0, 4),
        top_level: 4,
        blocks_movement: false,
        blocks_projectiles: false,
        blocks_sight: false,
    });
    assert!(head_clearance(pose, &[support], &view, g).is_none());
}

#[test]
fn initial_air_volume_requires_all_worm_masks_without_any_material_policy() {
    let geometry = ArenaVoxelGeometry::default();
    let pose = body(Vec3::Y * 0.4, [Vec3::ZERO]);
    let pos = voxel(0, 0, 2);
    let mut view = ArenaTerrainView::default();
    let mut query = BurrowQuery::default();
    query.refresh(&view);
    assert!(query.above_ground_clear(pose, &view, geometry));
    view.static_spans.push(ArenaStaticSpan {
        bottom: pos,
        top_level: 2,
        blocks_movement: false,
        blocks_projectiles: false,
        blocks_sight: false,
    });
    view.full_rebuild = true;
    view.revision += 1;
    query.refresh(&view);
    let mut ordinary = crate::collision::CollisionWorld::default();
    ordinary.refresh(&view, geometry);
    assert!(ordinary.clear(pose.feet, 0.4, hex_core::config::HEX_SMALL_DIAMETER * 0.5));
    assert!(!query.above_ground_clear(pose, &view, geometry));
    view.static_spans.clear();
    view.edit_protected.insert(pos.coord, vec![(2, 2)]);
    view.revision += 1;
    query.refresh(&view);
    assert!(!query.above_ground_clear(pose, &view, geometry));
    view.edit_protected.clear();
    view.voxels.insert(pos, SubstanceId(3));
    view.revision += 1;
    query.refresh(&view);
    assert!(
        !query.above_ground_clear(pose, &view, geometry),
        "even dirt is solid at deployment"
    );
    view.voxels.clear();
    view.liquids.push(hex_core::arena::ArenaSolidSpan {
        bottom: pos,
        top_level: 2,
        substance: SubstanceId(7),
    });
    view.revision += 1;
    query.refresh(&view);
    assert!(!query.above_ground_clear(pose, &view, geometry));
}
