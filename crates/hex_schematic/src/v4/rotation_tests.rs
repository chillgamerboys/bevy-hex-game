use super::*;
use hex_world_contracts::{hash_serializable, WorldHex};

const SOURCE: &str = r#"(version:1,id:"rotation-fixture",seed:91,
materials:[(id:"stone",solid:true,diggable:true,color:(100,100,100,255)),
(id:"bedrock",solid:true,diggable:false,color:(20,20,20,255))],
recipes:{"flat":(base_level:10,
strata:(bedrock:"bedrock",rock:"stone",soil:"stone",soil_depth:1,surface:"stone"),
features:[(id:"asymmetric",kind:"tree",asset:"procedural/test",mask:(center:(q:2,r:0),radius:0),
density:0,roots:[(q:2,r:0)],voxels:[(offset:(q:0,r:0),bottom:0,top:3,material:"stone"),
(offset:(q:1,r:0),bottom:2,top:3,material:"stone")])],hub:(column:(q:-6,r:0),level:10))},
regions:[(id:"flat",recipe:"flat",origin:(q:0,r:0),radius:8,rotation:0)],connections:[])"#;

#[test]
fn absent_orientation_preserves_legacy_hash_and_serialization() {
    let source = parse_world(SOURCE).expect("fixture");
    let feature = source
        .recipes
        .get("flat")
        .expect("recipe")
        .features
        .first()
        .expect("feature");
    assert_eq!(feature.rotation, None);
    assert!(!ron::ser::to_string(feature)
        .expect("serialize")
        .contains("rotation"));
    let package = compile_world(&source).expect("compile");
    let object = package
        .chunks
        .values()
        .flat_map(|c| &c.semantics.objects)
        .next()
        .expect("object");
    let expected =
        (hash_serializable(&(91_u64, "asymmetric", WorldHex::new(2, 0))).expect("hash") % 6) as u8;
    assert_eq!(object.rotation, expected);
    assert_eq!(package, compile_world(&source).expect("repeat compile"));
}

#[test]
fn explicit_orientations_transform_occupancy_and_grounding_exactly() {
    for turn in 0..6 {
        let mut source = parse_world(SOURCE).expect("fixture");
        source
            .recipes
            .get_mut("flat")
            .expect("recipe")
            .features
            .first_mut()
            .expect("feature")
            .rotation = Some(turn);
        let package = compile_world(&source).expect("compile");
        let object = package
            .chunks
            .values()
            .flat_map(|c| &c.semantics.objects)
            .next()
            .expect("object");
        let branch = WorldHex::new(2, 0)
            .checked_add(WorldHex::new(1, 0).rotate_60(turn).expect("turn"))
            .expect("offset");
        assert_eq!(object.rotation, turn);
        assert_eq!(object.occupancy.len(), 2);
        let support = object
            .occupancy
            .iter()
            .find(|c| c.position == branch)
            .expect("rotated branch");
        assert_eq!(support.runs.first().expect("run").bottom, 13);
        assert_eq!(support.runs.first().expect("run").top, 14);
        let grounding = object.grounding.as_ref().expect("grounding");
        assert_eq!(grounding.len(), 1);
        assert_eq!(
            grounding.first().expect("ground contact").column,
            WorldHex::new(2, 0)
        );
        assert_eq!(grounding.first().expect("ground contact").level, 10);
    }
}

#[test]
fn invalid_fixed_orientation_is_rejected_before_compilation() {
    let invalid = SOURCE.replace("density:0", "rotation:Some(6),density:0");
    assert!(parse_world(&invalid)
        .expect_err("invalid rotation")
        .to_string()
        .contains("rotation must be 0..5"));
}
