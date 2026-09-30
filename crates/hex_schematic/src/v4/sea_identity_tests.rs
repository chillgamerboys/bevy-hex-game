//! Compatibility witnesses for extending recipes without invalidating reviewed maps.
use super::*;
use hex_world_contracts::{hash_serializable, WorldHex};

#[test]
fn reviewed_forest_package_identity_survives_optional_sea_support() {
    let source = parse_world(include_str!(
        "../../../../assets/config/v4/forest-massif/expedition/world.ron"
    ))
    .expect("reviewed Forest source");
    assert!(source.recipes.values().all(|recipe| recipe.seas.is_empty()));
    let package = compile_world(&source).expect("complete Forest reproduction");
    assert_eq!(package.manifest.source_fingerprint, 0x6642_c37e_7d0d_71a1);
    assert_eq!(package.manifest.fingerprint, 0x8f56_a997_4aaa_c54b);
    assert_eq!(package.chunks.len(), 444);
}

#[test]
fn authored_sea_fills_remain_serialized_and_affect_recipe_identity() {
    let mut recipe: RegionRecipe = ron::from_str(
        r#"(base_level:10,strata:(bedrock:"bedrock",rock:"rock",soil:"soil",soil_depth:2,surface:"grass"),hub:(column:(q:0,r:0),level:10))"#,
    )
    .expect("legacy recipe without a sea field");
    let legacy = hash_serializable(&recipe).expect("legacy identity");
    recipe.seas.push(SeaFillSpec {
        id: "ocean".into(),
        mask: DiskMask {
            center: WorldHex::new(0, 0),
            radius: 12,
        },
        water_level: 15,
        material: "water".into(),
    });
    let encoded = ron::to_string(&recipe).expect("sea serialization");
    let decoded: RegionRecipe = ron::from_str(&encoded).expect("sea round trip");
    assert_eq!(decoded, recipe);
    assert_ne!(hash_serializable(&decoded).expect("sea identity"), legacy);
    recipe.seas.clear();
    assert_eq!(
        hash_serializable(&recipe).expect("restored identity"),
        legacy
    );
}
