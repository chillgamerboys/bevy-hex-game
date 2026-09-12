use super::{geometry, model::*, operators, volume};
use hex_world_contracts::{VoxelRun, WorldHex};

fn ground() -> operators::RegionBuild {
    operators::RegionBuild {
        columns: geometry::disk(WorldHex::new(0, 0), 32)
            .expect("bounded ground")
            .into_iter()
            .map(|p| {
                (
                    p,
                    vec![VoxelRun {
                        bottom: 0,
                        top: 41,
                        material: "stone".into(),
                    }],
                )
            })
            .collect(),
        ..Default::default()
    }
}

fn arch() -> BridgeSpec {
    BridgeSpec {
        id: "arched-crossing".into(),
        points: [(-24, 48), (0, 60), (24, 48)]
            .into_iter()
            .map(|(q, level)| GradePoint {
                column: WorldHex::new(q, 0),
                level,
            })
            .collect(),
        half_width: 3,
        thickness: 3,
        material: "limestone".into(),
    }
}

#[test]
fn broad_arch_has_walkable_full_width_preserved_air_and_pinned_apex() {
    let mut world = ground();
    operators::bridge(&mut world, &arch()).expect("graded broad crossing");
    for (p, expected) in [(-24, 48), (0, 60), (24, 48)] {
        let at = WorldHex::new(p, 0);
        assert_eq!(operators::terrain(&world, at).expect("deck").0, expected);
    }
    for p in &world.reserved {
        let (top, material) = operators::terrain(&world, *p).expect("deck");
        assert_eq!(material, "limestone");
        let column = world.columns.get(p).expect("deck column");
        assert_eq!(volume::material_at(column, 40), Some("stone"));
        assert_eq!(volume::material_at(column, top - 3), None);
        assert_eq!(volume::material_at(column, top - 2), Some("limestone"));
        for n in geometry::neighbors(*p).filter(|n| world.reserved.contains(n)) {
            assert!(top.abs_diff(operators::terrain(&world, n).expect("neighbor").0) <= 1);
        }
    }
    for r in -3..=3 {
        assert!(world.reserved.contains(&WorldHex::new(0, r)));
    }
}

#[test]
fn bridge_cross_sections_are_invariant_under_reverse_traversal() {
    let mut forward = ground();
    let mut reverse = ground();
    let mut bridge = arch();
    operators::bridge(&mut forward, &bridge).expect("forward");
    bridge.points.reverse();
    operators::bridge(&mut reverse, &bridge).expect("reverse");
    for (p, expected) in &forward.columns {
        assert_eq!(reverse.columns.get(p), Some(expected), "column {p:?}");
    }
    assert_eq!(forward.reserved, reverse.reserved);
}

#[test]
fn bridge_cross_sections_rotate_with_the_authored_path() {
    let mut canonical = ground();
    operators::bridge(&mut canonical, &arch()).expect("canonical");
    for turns in 1..6 {
        let mut rotated = ground();
        let mut bridge = arch();
        for point in &mut bridge.points {
            point.column = point.column.rotate_60(turns).expect("rotation");
        }
        operators::bridge(&mut rotated, &bridge).expect("rotated arch");
        for p in &canonical.reserved {
            let rotated_p = p.rotate_60(turns).expect("rotation");
            assert_eq!(
                rotated.columns.get(&rotated_p),
                canonical.columns.get(p),
                "turn {turns} at {p:?}"
            );
        }
    }
}

#[test]
fn bridge_cannot_silently_flatten_conflicting_retraced_controls() {
    let mut world = ground();
    let mut bridge = arch();
    bridge.points = [(-12, 48), (12, 60), (-12, 50)]
        .into_iter()
        .map(|(q, level)| GradePoint {
            column: WorldHex::new(q, 0),
            level,
        })
        .collect();
    let error = operators::bridge(&mut world, &bridge).expect_err("contradictory crossing");
    assert!(error.contains("conflicting centerline"), "{error}");
    assert!(world.reserved.is_empty());
}

#[test]
fn rejected_arch_does_not_publish_a_partial_deck() {
    let mut world = ground();
    world.columns.remove(&WorldHex::new(25, 0));
    let original = world.columns.clone();
    assert!(operators::bridge(&mut world, &arch()).is_err());
    assert_eq!(world.columns, original);
    assert!(world.reserved.is_empty());
}

#[test]
fn flat_bridge_keeps_its_existing_exact_ribbon() {
    let mut world = ground();
    let mut bridge = arch();
    for point in &mut bridge.points {
        point.level = 48;
    }
    operators::bridge(&mut world, &bridge).expect("flat crossing");
    let line = geometry::line(WorldHex::new(-24, 0), WorldHex::new(24, 0)).expect("centerline");
    assert_eq!(world.reserved, geometry::ribbon(&line, 3).expect("ribbon"));
    for p in &world.reserved {
        assert_eq!(operators::terrain(&world, *p).expect("deck").0, 48);
    }
}
