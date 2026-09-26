use super::*;
#[test]
fn full_measured_world_has_independent_crystal_and_exact_cave_sites() {
    let source = ron::from_str(include_str!(
        "../../../../../assets/config/v4/grand-v4/world.ron"
    ))
    .unwrap();
    let compiler = GrandCompiler::new(source).unwrap();
    assert_eq!(compiler.mainland_columns, 93326 * 7);
    assert_eq!(compiler.crystal.len(), 3169 * 7);
    let sites = compiler.sites(123).unwrap();
    assert_eq!(sites.encounters.len(), 14);
    for site in &sites.encounters {
        assert!(compiler.clear_support(site.preferred, 16));
    }
    let cave = compiler.support(-105., -295., true);
    assert_eq!(cave.level, 520);
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

#[test]
fn complete_stair_route_has_walkable_risers_and_tunnels_remain_separate() {
    let mut source: GrandSpec = ron::from_str(include_str!(
        "../../../../../assets/config/v4/grand-v4/world.ron"
    ))
    .unwrap();
    source.full_dressing = false;
    let g = GrandCompiler::new(source).unwrap();
    let positions = [
        (-119, -247),
        (-111, -263),
        (-42, -263),
        (-28, -290),
        (-106, -290),
        (-93, -317),
        (-26, -317),
        (-13, -343),
        (-68, -343),
        (-55, -370),
        (-46, -370),
        (-42, -377),
    ];
    let mut previous: Option<i32> = None;
    for pair in positions.windows(2) {
        let [a, b] = pair else {
            continue;
        };
        let a = WorldHex::new(a.0, a.1);
        let b = WorldHex::new(b.0, b.1);
        let steps = a.checked_distance(b).unwrap();
        let axz = world_xz(a);
        let bxz = world_xz(b);
        for i in 0..=steps {
            let t = i as f64 / steps as f64;
            let p = nearest_hex(
                axz[0] + (bxz[0] - axz[0]) * t,
                axz[1] + (bxz[1] - axz[1]) * t,
            );
            let floor = library_cavity(p).unwrap().0;
            let support = VoxelPosition {
                column: p,
                level: floor,
            };
            assert!(
                g.clear_support(support, 12),
                "blocked stair at {support:?}, column {:?}, surface {:?}",
                g.column(p),
                g.surface(p)
            );
            if let Some(last) = previous {
                assert!(
                    (floor - last).abs() <= 1,
                    "stair riser too tall: {last}->{floor}"
                );
            }
            previous = Some(floor);
        }
    }
    let p = nearest_hex(-105., -348.);
    let (column, _) = g.column(p);
    assert_eq!(column.material_at(522), None);
    assert_eq!(column.material_at(723), None);
    assert!(
        column.material_at(620).is_some(),
        "Shadow tunnel must not connect to library"
    );
    let sites = g.sites(9).unwrap();
    assert!(!sites.fountains.first().unwrap().cells.is_empty());
    let start = g
        .anchors
        .iter()
        .find(|a| a.id == "grand/anchor/party_start")
        .unwrap();
    assert!((400..=420).contains(&start.position.level));
    let biome = g.biomes(9);
    let [x, z] = world_xz(start.position.column);
    assert_ne!(
        biome.label_at([x, f64::from(start.position.level + 1) * LEVEL_HEIGHT, z]),
        Some("Open Sea")
    );
}

#[test]
fn watercourse_is_continuous_and_descends_from_garden_to_open_sea() {
    let mut source: GrandSpec = ron::from_str(include_str!(
        "../../../../../assets/config/v4/grand-v4/world.ron"
    ))
    .unwrap();
    source.full_dressing = false;
    let g = GrandCompiler::new(source).unwrap();
    let mut previous = 900;
    for z in -448..=-145 {
        let p = nearest_hex(headwater_center(f64::from(z)), f64::from(z));
        let surface = g.surface(p);
        let top = surface
            .water
            .expect("continuous headwater and waterfall center");
        // A waterfall's upper curtain overlaps the lower reach by two cells.
        assert!(
            top <= previous,
            "watercourse rises at {p:?}: {previous}->{top}"
        );
        previous = top;
        assert!(surface.level < top);
    }
    previous = 615;
    for z in -60..=600 {
        let p = nearest_hex(river_center(f64::from(z)), f64::from(z));
        let surface = g.surface(p);
        let top = surface.water.unwrap_or(SEA_TOP);
        assert!(top <= previous, "river rises at {p:?}: {previous}->{top}");
        assert!(
            surface.level < top,
            "river interrupted at {p:?}: {surface:?}"
        );
        previous = top;
    }
    for x in 276..=306 {
        let center =
            -474. + 4. * (f64::from(x - 276) / 30.) + 1.5 * (f64::from(x - 276) / 9.).sin();
        assert_eq!(
            g.surface(nearest_hex(f64::from(x), center)).water,
            Some(900)
        );
    }
}

#[test]
fn offshore_sailing_reference_has_clear_sea_between_launch_and_landing() {
    let mut source: GrandSpec = ron::from_str(include_str!(
        "../../../../../assets/config/v4/grand-v4/world.ron"
    ))
    .unwrap();
    source.full_dressing = false;
    let g = GrandCompiler::new(source).unwrap();
    let a = g
        .anchors
        .iter()
        .find(|a| a.id == "grand/anchor/sailing_start")
        .unwrap();
    let b = g
        .anchors
        .iter()
        .find(|a| a.id == "grand/anchor/volcano_landing")
        .unwrap();
    let a = world_xz(a.position.column);
    let b = world_xz(b.position.column);
    let distance = (a[0] - b[0]).hypot(a[1] - b[1]);
    // Controller calibration is 793.9485u/45s, while the route uses exact hex anchors.
    assert!((790. ..=801.).contains(&distance));
    for step in 0..=800 {
        let t = f64::from(step) / 800.;
        let p = nearest_hex(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
        assert!(!g.mainland(p), "sailing route crosses mainland at {p:?}");
        assert!(
            g.surface(p).level < SEA_TOP,
            "sailing route crosses terrain at {p:?}"
        );
    }
}
