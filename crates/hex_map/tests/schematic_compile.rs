//! External contract for compiling one exact Grand V3 schematic artifact.

use bevy::prelude::World;
use hex_assets::{ArtPalette, SubstanceFile, SubstanceTable};
use hex_map::{
    compile_schematic, export_world_snapshot_v1, GenerationReport, MapSettings, VoxelMap,
};

#[test]
fn public_exact_schematic_boundary_compiles_publishes_and_exports() {
    let template = hex_schematic::grand_v3_reference_template().expect("template should parse");
    let generated =
        hex_schematic::reference_plan(&template, 1_592_598_566).expect("reference should validate");
    let settings: MapSettings = ron::de::from_str(include_str!(
        "../../../assets/config/worlds/procedural-grand-v3-baseline.ron"
    ))
    .expect("Grand V3 proxy settings should parse");
    let palette: ArtPalette = ron::de::from_str(include_str!("../../../assets/art/palette.ron"))
        .expect("art palette should parse");
    let substances: SubstanceFile =
        ron::de::from_str(include_str!("../../../assets/config/substances.ron"))
            .expect("substances should parse");
    let table = SubstanceTable::from_file(&substances, &palette)
        .expect("accepted content should resolve substances");

    let compiled = compile_schematic(&generated.plan, &settings, &table)
        .expect("the exact public plan should compile");
    assert_eq!(compiled.map.len(), 105_469);
    let presentation = compiled.presentation_counts();
    assert!(presentation.liquids > 30_000);
    assert_eq!(presentation.features, 0);
    assert_eq!(presentation.structures, 0);
    assert_eq!(presentation.lights, 0);

    let mut world = World::new();
    world.insert_resource(settings);
    world.insert_resource(table);
    compiled.publish(&mut world);
    assert!(
        !world.contains_resource::<hex_core::TerrainReady>(),
        "resource publication cannot claim readiness before chunk roots exist"
    );
    assert_eq!(world.resource::<VoxelMap>().len(), 105_469);
    assert_eq!(world.resource::<GenerationReport>().seed, 1_592_598_566);

    let snapshot = export_world_snapshot_v1(&world).expect("published exact plan should export");
    assert_eq!(snapshot.columns.len(), 105_469);
    assert_eq!(snapshot.liquids.len(), presentation.liquids);
    assert_eq!(snapshot.version, 1);
}
