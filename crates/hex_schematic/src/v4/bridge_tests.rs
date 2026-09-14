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
        walkway_half_width: None,
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

#[test]
fn authored_walkway_reserves_travel_and_releases_supported_outer_water_ledge() {
    use hex_world_contracts::{LiquidColumn, LiquidKind};
    let mut world = ground();
    let mut bridge = arch();
    bridge.half_width = 5;
    bridge.walkway_half_width = Some(4);
    let outer = WorldHex::new(0, 5);
    world.columns.insert(
        outer,
        vec![
            VoxelRun {
                bottom: 0,
                top: 31,
                material: "stone".into(),
            },
            VoxelRun {
                bottom: 31,
                top: 36,
                material: "water".into(),
            },
        ],
    );
    world.liquids.insert(
        outer,
        LiquidColumn {
            column: outer,
            bottom: 31,
            top: 36,
            kind: LiquidKind::Standing,
            body_id: "river".into(),
            downstream: vec![],
        },
    );
    world.reserved.insert(outer);
    let water_before = world.liquids.clone();
    operators::bridge(&mut world, &bridge).expect("separate dry ledge");
    let centerline = geometry::line(WorldHex::new(-24, 0), WorldHex::new(24, 0)).expect("line");
    assert_eq!(
        world.reserved,
        geometry::ribbon(&centerline, 4).expect("walkway")
    );
    assert!(!world.reserved.contains(&outer));
    assert_eq!(
        operators::terrain(&world, outer).expect("ledge").1,
        "limestone"
    );
    assert_eq!(world.liquids, water_before);
    assert_eq!(
        volume::material_at(world.columns.get(&outer).expect("column"), 33),
        Some("water")
    );
}

#[test]
fn bridge_outer_ledge_preserves_an_existing_dry_reservation() {
    let mut world = ground();
    let mut bridge = arch();
    bridge.half_width = 5;
    bridge.walkway_half_width = Some(4);
    let prior = WorldHex::new(0, 5);
    world.reserved.insert(prior);
    operators::bridge(&mut world, &bridge).expect("bridge");
    assert!(world.reserved.contains(&prior));
}

#[test]
fn invalid_walkway_width_is_atomic_and_legacy_serialization_omits_it() {
    let mut bridge = arch();
    assert!(!ron::ser::to_string(&bridge)
        .expect("serialize")
        .contains("walkway_half_width"));
    bridge.walkway_half_width = Some(bridge.half_width + 1);
    let mut world = ground();
    let before = world.columns.clone();
    assert!(operators::bridge(&mut world, &bridge)
        .expect_err("oversize walkway")
        .contains("walkway"));
    assert_eq!(world.columns, before);
    assert!(world.reserved.is_empty());

    let mut source =
        super::parse_world(include_str!("../../../../assets/config/v4/rich-region.ron"))
            .expect("source");
    let recipe = source.recipes.values_mut().next().expect("recipe");
    let bridge = recipe.bridges.first_mut().expect("bridge");
    bridge.walkway_half_width = Some(bridge.half_width + 1);
    assert!(super::validate_world(&source).is_err());
}
