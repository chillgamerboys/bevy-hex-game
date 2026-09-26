use super::*;
#[test]
fn full_measured_world_has_independent_crystal_and_exact_cave_sites() {
    let source = ron::from_str(include_str!(
        "../../../../../assets/config/v4/grand-v4/world.ron"
    ))
    .unwrap();
    let compiler = GrandCompiler::new(source).unwrap();
    assert_eq!(compiler.mainland_columns, 92849 * 7);
    assert_eq!(compiler.crystal.len(), 3169 * 7);
    let sites = compiler.sites(123).unwrap();
    assert_eq!(sites.encounters.len(), 14);
    for site in &sites.encounters {
        assert!(compiler.clear_support(site.preferred, 16));
    }
    let cave = compiler.support(-105., -295., true);
    assert_eq!(cave.level, 720);
    let (column, _) = compiler.column(cave.column);
    assert!(column.runs.iter().any(|r| r.bottom > cave.level + 16));
    let site_chunks: std::collections::BTreeSet<_> = sites
        .encounters
        .iter()
        .map(|s| s.preferred.column.chunk())
        .collect();
    for id in site_chunks {
        compiler.chunk(id).unwrap().unwrap().validate().unwrap();
    }
}
