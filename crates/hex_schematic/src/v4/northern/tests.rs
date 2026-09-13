use super::*;
fn spec() -> NorthernSpec {
    ron::from_str(include_str!(
        "../../../../../assets/config/v4/northern-archipelago/world.ron"
    ))
    .expect("source")
}
#[test]
fn exact_fullscale_roster_and_surface_contract() {
    let source = spec();
    source.validate().expect("valid");
    assert_eq!(source.islands.len(), 11);
    assert_eq!(RADIUS, 700);
    let mut peak = f64::MIN;
    let mut low = f64::MAX;
    for q in (-700..=700).step_by(3) {
        for r in (-700..=700).step_by(3) {
            let p = WorldHex::new(q, r);
            if p.checked_distance(WorldHex::new(0, 0)).expect("distance") > 700 {
                continue;
            }
            let height = (source.surface(p).level + 1) as f64 * LEVEL_HEIGHT - 140.0;
            peak = peak.max(height);
            low = low.min(height);
        }
    }
    assert!((285.0..=320.0).contains(&peak), "peak {peak}");
    assert!((-141.0..=-80.0).contains(&low), "deep seafloor {low}");
}
#[test]
fn chunk_is_deterministic_and_water_reaches_its_exact_surface() {
    let compiler = NorthernCompiler::new(spec()).expect("compiler");
    let id = WorldHex::new(0, 0).chunk();
    let a = compiler.chunk(id).expect("compile").expect("chunk");
    let b = compiler.chunk(id).expect("compile").expect("chunk");
    assert_eq!(a, b);
    for liquid in &a.semantics.liquids {
        assert_eq!(liquid.top, SEA_TOP);
        let column = a
            .columns
            .iter()
            .find(|c| c.position == liquid.column)
            .expect("column");
        let bed = column
            .runs
            .iter()
            .filter(|r| r.material != "water")
            .map(|r| r.top)
            .max()
            .expect("bed");
        assert_eq!(liquid.bottom, bed);
    }
}
#[test]
fn spawn_buildings_and_exact_tree_occupancy_are_supported() {
    let compiler = NorthernCompiler::new(spec()).expect("compiler");
    assert!(compiler.tree_count > 10);
    let overview = compiler.overview();
    let [_, height, _] = overview.player_spawn;
    assert!(height > 142.0, "dryspawn {height}");
    let (lowest, _) = overview
        .bed_heights
        .iter()
        .enumerate()
        .filter(|(index, _)| {
            let x = f64::from(overview.origin_xz[0])
                + (*index % overview.width as usize) as f64 * f64::from(overview.spacing);
            let z = f64::from(overview.origin_xz[1])
                + (*index / overview.width as usize) as f64 * f64::from(overview.spacing);
            nearest_hex(x, z)
                .checked_distance(WorldHex::new(0, 0))
                .expect("distance")
                <= RADIUS as u64
        })
        .min_by(|a, b| a.1.total_cmp(b.1))
        .expect("seabed samples");
    let low_x = f64::from(overview.origin_xz[0])
        + (lowest % overview.width as usize) as f64 * f64::from(overview.spacing);
    let low_z = f64::from(overview.origin_xz[1])
        + (lowest / overview.width as usize) as f64 * f64::from(overview.spacing);
    compiler
        .chunk(nearest_hex(low_x, low_z).chunk())
        .expect("deepest seabed chunk has no empty substrate run")
        .expect("in world");
    assert_eq!(
        overview.bed_heights.len(),
        overview.width as usize * overview.height as usize
    );
    for influences in compiler.influences.values() {
        union_object_occupancy(influences)
            .expect("reserved structures and tree footprints never conflict");
    }
    for object in compiler.objects.values().flatten() {
        object.validate().expect("blueprint");
        for contact in object.grounding.as_deref().unwrap_or_default() {
            assert_eq!(compiler.source.surface(contact.column).level, contact.level);
        }
        for column in &object.occupancy {
            let ground = compiler.source.surface(column.position).level;
            assert!(
                column.runs.iter().all(|r| r.bottom > ground),
                "object {} intersects terrain at {:?}",
                object.id,
                column.position
            );
        }
    }
}
#[test]
fn eleven_separate_land_components_and_cluster_shore_gaps() {
    let source = spec();
    let mut land = BTreeMap::new();
    for q in (-690..=690).step_by(2) {
        for r in (-690..=690).step_by(2) {
            let p = WorldHex::new(q, r);
            if source.surface(p).level >= SEA_TOP {
                let [x, z] = world_xz(p);
                let island = source
                    .islands
                    .iter()
                    .min_by(|a, b| a.normalized(x, z).0.total_cmp(&b.normalized(x, z).0))
                    .expect("island");
                land.insert(p, island.cluster);
            }
        }
    }
    let neighbors = [(2, 0), (0, 2), (-2, 2), (-2, 0), (0, -2), (2, -2)];
    let mut shores: [Vec<[f64; 2]>; 3] = Default::default();
    for (p, cluster) in &land {
        if neighbors
            .iter()
            .any(|(q, r)| !land.contains_key(&WorldHex::new(p.q + q, p.r + r)))
        {
            shores
                .get_mut(usize::from(*cluster))
                .expect("cluster")
                .push(world_xz(*p));
        }
    }
    let mut components = 0;
    while let Some((&start, _)) = land.first_key_value() {
        components += 1;
        let mut pending = vec![start];
        land.remove(&start);
        while let Some(p) = pending.pop() {
            for (q, r) in neighbors {
                let n = WorldHex::new(p.q + q, p.r + r);
                if land.remove(&n).is_some() {
                    pending.push(n);
                }
            }
        }
    }
    assert_eq!(components, 11);
    for (a, b) in [(0, 1), (0, 2), (1, 2)] {
        let mut distance = f64::MAX;
        for [x, z] in shores.get(a).expect("shore") {
            for [xx, zz] in shores.get(b).expect("shore") {
                distance = distance.min((x - xx).hypot(z - zz));
            }
        }
        assert!(
            (300.0..=500.0).contains(&distance),
            "cluster {a}/{b}: {distance}"
        );
    }
}
