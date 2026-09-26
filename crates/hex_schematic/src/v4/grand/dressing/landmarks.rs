//! Small, supported architectural groups around Grand's existing playable voids.
use super::*;

fn garden(g: &GrandCompiler, occupied: &Occupied) -> Result<Vec<ObjectInstance>, ContractError> {
    let mut frame = Cells::new();
    // Short north/west arcades frame the court without enclosing its south and
    // east approaches. Their overhead beams are well above ordinary headroom.
    for (x, z, along_x) in [(248., -491., false), (274., -514., true)] {
        let root = nearest_hex(x, z);
        let base = g.surface(root).level + 1;
        for offset in -16_i32..=16 {
            if along_x && offset.abs() < 6 {
                continue;
            }
            let p = nearest_hex(
                x + if along_x { f64::from(offset) } else { 0. },
                z + if along_x { 0. } else { f64::from(offset) },
            );
            let ground = g.surface(p).level + 1;
            if offset.abs() == 16 || (along_x && offset.abs() == 6) || (!along_x && offset == 0) {
                add(&mut frame, p, ground, base + 20, "stone");
                add(&mut frame, p, base + 17, base + 20, "sand");
            }
            let arch = if along_x {
                18 + (5 - (offset.abs() - 11).abs()) / 2
            } else {
                18 + (8 - (offset.rem_euclid(16) - 8).abs()) / 3
            };
            add(&mut frame, p, base + arch, base + arch + 3, "sand");
            add(&mut frame, p, base + arch + 3, base + arch + 4, "moss");
        }
    }
    let mut plants = Cells::new();
    // Small raised beds and leafy centres, separated by generous walking gaps.
    // The existing shrine and the basin/rill axis remain wholly undecorated.
    for (x, z) in [(258., -504.), (290., -504.), (256., -462.), (291., -457.)] {
        let root = nearest_hex(x, z);
        for q in -3_i64..=3 {
            for r in -3_i64..=3 {
                let d = q.abs().max(r.abs()).max((q + r).abs());
                if d > 3 {
                    continue;
                }
                let p = WorldHex::new(root.q + q, root.r + r);
                let surface = g.surface(p);
                if surface.water.is_some() {
                    continue;
                }
                let floor = surface.level + 1;
                add(
                    &mut plants,
                    p,
                    floor,
                    floor + 2,
                    if d == 3 { "sand" } else { "soil" },
                );
                if d < 3 {
                    add(
                        &mut plants,
                        p,
                        floor + 2,
                        floor + 3 + (3 - d) as i32,
                        if (q - r).rem_euclid(3) == 0 {
                            "foliage_light"
                        } else {
                            "foliage"
                        },
                    );
                }
            }
        }
    }
    let root = nearest_hex(276., -474.);
    let mut basin = Cells::new();
    for q in -4_i64..=4 {
        for r in -4_i64..=4 {
            if q.abs().max(r.abs()).max((q + r).abs()) != 4 {
                continue;
            }
            let p = WorldHex::new(root.q + q, root.r + r);
            let [x, z] = world_xz(p);
            // The existing flowing outlet is untouched; the broad south break
            // permits an ordinary walk right to the original water cells.
            if g.surface(p).water.is_some() || ((x - 276.).abs() < 3.5 && z > -474.) {
                continue;
            }
            let floor = g.surface(p).level + 1;
            // The established Water Shrine posts retain these exact cells.
            // The low rim may meet a post, but cannot author another material
            // inside it; water and the open southern approach remain untouched.
            if occupied
                .get(&p)
                .is_some_and(|runs| runs.iter().any(|(lo, hi)| *lo <= floor && floor < *hi))
            {
                continue;
            }
            add(&mut basin, p, floor, floor + 1, "sand");
        }
    }
    Ok(vec![
        object(
            g,
            "grand/garden-arcades".into(),
            "structure/grand-garden-court",
            nearest_hex(260., -500.),
            frame,
        )?,
        object(
            g,
            "grand/garden-beds".into(),
            "decor/grand-garden-beds",
            nearest_hex(258., -504.),
            plants,
        )?,
        object(
            g,
            "grand/fountain-rim".into(),
            "structure/grand-fountain-rim",
            root,
            basin,
        )?,
    ])
}

#[derive(Clone, Copy)]
struct Bay {
    x: f64,
    z: f64,
    along_x: bool,
    floor: i32,
}
const BAYS: &[Bay] = &[
    Bay {
        x: -422.,
        z: -309.,
        along_x: true,
        floor: 600,
    },
    Bay {
        x: -394.,
        z: -309.,
        along_x: true,
        floor: 600,
    },
    Bay {
        x: -366.,
        z: -309.,
        along_x: true,
        floor: 600,
    },
    Bay {
        x: -337.,
        z: -309.,
        along_x: true,
        floor: 600,
    },
    Bay {
        x: -367.,
        z: -381.,
        along_x: true,
        floor: 600,
    },
    Bay {
        x: -337.,
        z: -381.,
        along_x: true,
        floor: 600,
    },
    Bay {
        x: -435.,
        z: -375.,
        along_x: false,
        floor: 600,
    },
    Bay {
        x: -435.,
        z: -323.,
        along_x: false,
        floor: 600,
    },
    Bay {
        x: -315.,
        z: -323.,
        along_x: false,
        floor: 600,
    },
    Bay {
        x: -315.,
        z: -378.,
        along_x: false,
        floor: 600,
    },
    Bay {
        x: -432.,
        z: -526.,
        along_x: false,
        floor: 824,
    },
    Bay {
        x: -432.,
        z: -546.,
        along_x: false,
        floor: 824,
    },
    Bay {
        x: -370.,
        z: -526.,
        along_x: false,
        floor: 824,
    },
];

fn bay(
    g: &GrandCompiler,
    index: usize,
    b: Bay,
    encounter_columns: &std::collections::BTreeSet<WorldHex>,
) -> Result<ObjectInstance, ContractError> {
    let mut cells = Cells::new();
    let base = b.floor + 1;
    // A wall bay has visible depth: shelves sit one hex behind the piers.
    // Every authored voxel is checked against the actual cave interval below,
    // so a stair crossing takes precedence over an ornament, never the reverse.
    for t in -8_i32..=8 {
        let p = nearest_hex(
            b.x + if b.along_x { f64::from(t) } else { 0. },
            b.z + if b.along_x { 0. } else { f64::from(t) },
        );
        if t.abs() >= 7 {
            add(&mut cells, p, base, base + 34, "stone");
            add(&mut cells, p, base, base + 3, "slate");
            add(&mut cells, p, base + 29, base + 33, "sand");
        }
        let soffit = 27 + (8 - t.abs()) / 2;
        add(&mut cells, p, base + soffit, base + soffit + 3, "sand");
        if t.abs() <= 6 {
            // Recess along the inward/outward normal chosen for each side wall.
            let inward = if b.along_x || b.x > -400. { -1.5 } else { 1.5 };
            let shelf = nearest_hex(
                b.x + if b.along_x { f64::from(t) } else { -inward },
                b.z + if b.along_x { -inward } else { f64::from(t) },
            );
            for tier in 0_i32..3 {
                let lo = base + tier * 8;
                add(&mut cells, shelf, lo, lo + 2, "timber");
                if t.abs() == 6 {
                    add(&mut cells, shelf, lo + 2, lo + 8, "timber");
                } else {
                    let material = match (t + tier * 3).rem_euclid(5) {
                        0 => "worked_stone",
                        1 => "crystal",
                        2 => "sand",
                        3 => "moss",
                        _ => "slate",
                    };
                    add(
                        &mut cells,
                        shelf,
                        lo + 2,
                        lo + 5 + (t + tier).rem_euclid(3),
                        material,
                    );
                }
            }
            add(&mut cells, shelf, base + 24, base + 26, "timber");
        }
    }
    // The exact cave and encounter semantics remain authoritative. Decoration
    // may not occupy any step of the continuous ascending stair or deployment.
    cells.retain(|p, ys| {
        let Some((floor, ceiling)) = library_cavity(*p) else {
            return false;
        };
        if floor != b.floor || encounter_columns.contains(p) {
            return false;
        }
        ys.retain(|y, _| *y > floor && *y < ceiling);
        !ys.is_empty()
    });
    object(
        g,
        format!("grand/library-bay/{index:02}"),
        "structure/grand-library-arcade",
        nearest_hex(b.x, b.z),
        cells,
    )
}

pub(super) fn compose(
    g: &GrandCompiler,
    existing: &[ObjectInstance],
) -> Result<Vec<ObjectInstance>, ContractError> {
    // Exact manifest-bound deployment columns plus one neighbouring hex retain
    // body clearance without treating an entire interior hall as a forest glade.
    let mut encounter_columns = std::collections::BTreeSet::new();
    for encounter in g.sites(0)?.encounters {
        for support in encounter.surfaces {
            encounter_columns.insert(support.column);
            for (q, r) in DIRS {
                encounter_columns.insert(WorldHex::new(support.column.q + q, support.column.r + r));
            }
        }
    }
    let mut occupied = Occupied::new();
    for object in existing {
        reserve(&mut occupied, object);
    }
    let mut out = garden(g, &occupied)?;
    for (index, b) in BAYS.iter().copied().enumerate() {
        out.push(bay(g, index, b, &encounter_columns)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_dressing_has_compatible_materials_in_every_shared_chunk() {
        let source: GrandSpec = ron::from_str(include_str!(
            "../../../../../../assets/config/v4/grand-v4/world.ron"
        ))
        .expect("Grand source");
        assert!(
            source.full_dressing,
            "validate the actual complete composition"
        );
        let compiler = GrandCompiler::new(source).expect("complete composition");
        let mut conflicts = Vec::new();
        for (chunk, influences) in &compiler.influences {
            let mut occupied: BTreeMap<WorldHex, Vec<(&str, &VoxelRun)>> = BTreeMap::new();
            for influence in influences {
                for column in &influence.occupancy {
                    let previous = occupied.entry(column.position).or_default();
                    for run in &column.runs {
                        for (id, old) in previous.iter() {
                            if old.bottom < run.top
                                && run.bottom < old.top
                                && old.material != run.material
                            {
                                conflicts.push(format!(
                                    "chunk {chunk:?}, column {:?}, levels {}..{}: {id} ({}) vs {} ({})",
                                    column.position,
                                    old.bottom.max(run.bottom),
                                    old.top.min(run.top),
                                    old.material,
                                    influence.id,
                                    run.material,
                                ));
                            }
                        }
                        previous.push((&influence.id, run));
                    }
                }
            }
        }
        assert!(conflicts.is_empty(), "{}", conflicts.join("\n"));
        for q in [19, 20] {
            let garden = compiler
                .chunk(ChunkId { q, r: -20 })
                .expect("Garden composition must pass strict chunk sealing")
                .expect("Garden chunk exists");
            assert!(!garden.semantics.object_influences.is_empty());
        }
        let rim = compiler
            .objects
            .values()
            .flatten()
            .find(|object| object.id == "grand/fountain-rim")
            .expect("the low fountain rim remains authored");
        assert!(!rim.occupancy.is_empty());
        for q in [318, 321] {
            let p = WorldHex::new(q, -320);
            assert!(
                rim.occupancy.iter().all(|column| column.position != p),
                "the existing Water Shrine owns the rim/post intersection at {p:?}"
            );
            let shrine = compiler
                .influences
                .get(&p.chunk())
                .expect("shrine chunk")
                .iter()
                .find(|object| object.id == "grand/shrine/water")
                .expect("the existing Water Shrine remains authored");
            let post = shrine
                .occupancy
                .iter()
                .find(|column| column.position == p)
                .expect("the established shrine post is preserved");
            assert!(post.runs.iter().any(|run| run.material == "stone"));
        }
    }

    #[test]
    fn courtyard_and_library_are_supported_and_preserve_exact_water_and_passages() {
        let mut source: GrandSpec = ron::from_str(include_str!(
            "../../../../../../assets/config/v4/grand-v4/world.ron"
        ))
        .expect("Grand source");
        source.full_dressing = false;
        let plain = GrandCompiler::new(source.clone()).expect("terrain");
        let water_shrine =
            shrine(&plain, "water", 275., -490., false).expect("existing Water Shrine fixture");
        let additions = compose(&plain, &[water_shrine]).expect("bounded architecture");
        assert_eq!(additions.len(), 16);
        let mut occupied = Occupied::new();
        for object in &additions {
            assert!(!object.occupancy.is_empty(), "{}", object.id);
            assert!(
                !object.grounding.as_ref().expect("contacts").is_empty(),
                "{}",
                object.id
            );
            assert!(
                object.occupancy.len() < 600,
                "bounded footprint: {}",
                object.id
            );
            assert!(
                !overlaps(&occupied, &object.occupancy),
                "overlap: {}",
                object.id
            );
            reserve(&mut occupied, object);
            if object.id.starts_with("grand/library-") {
                for column in &object.occupancy {
                    let (floor, ceiling) = library_cavity(column.position).expect("inside library");
                    assert!(column
                        .runs
                        .iter()
                        .all(|r| r.bottom > floor && r.top <= ceiling));
                }
                assert!(
                    object
                        .occupancy
                        .iter()
                        .flat_map(|c| &c.runs)
                        .any(|r| r.material == "crystal"),
                    "filled shelves: {}",
                    object.id
                );
            }
        }
        // Preserve a broad ribbon around the original, independently recorded
        // stair centreline, including its lower/upper room connections.
        let stair = [
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
        for pair in stair.windows(2) {
            let [a, b] = pair else {
                continue;
            };
            let a = WorldHex::new(a.0, a.1);
            let b = WorldHex::new(b.0, b.1);
            let steps = a.checked_distance(b).expect("bounded route");
            for step in 0..=steps {
                let t = step as f64 / steps as f64;
                let [ax, az] = world_xz(a);
                let [bx, bz] = world_xz(b);
                let centre = nearest_hex(ax + (bx - ax) * t, az + (bz - az) * t);
                for q in -2_i64..=2 {
                    for r in -2_i64..=2 {
                        if q.abs().max(r.abs()).max((q + r).abs()) > 2 {
                            continue;
                        }
                        let p = WorldHex::new(centre.q + q, centre.r + r);
                        let (floor, _) = library_cavity(p).expect("stair ribbon");
                        assert!(
                            occupied.get(&p).is_none_or(|runs| runs
                                .iter()
                                .all(|(a, b)| *a >= floor + 9 || *b <= floor + 1)),
                            "stair obstructed at {p:?}"
                        );
                    }
                }
            }
        }
        let before = plain.sites(1).expect("plain sites");
        source.full_dressing = true;
        let dressed = GrandCompiler::new(source).expect("complete composition");
        let after = dressed
            .sites(1)
            .expect("all shrines and encounters retain headroom");
        assert_eq!(
            before.fountains.first().expect("fountain").cells,
            after.fountains.first().expect("fountain").cells
        );
        for old in &before.encounters {
            if matches!(old.id.as_str(), "grand_golem_01" | "grand_wisp_01") {
                let new = after
                    .encounters
                    .iter()
                    .find(|e| e.id == old.id)
                    .expect("same encounter");
                assert_eq!(
                    old.surfaces, new.surfaces,
                    "exact interior deployment remains clear: {}",
                    old.id
                );
            }
        }
        let fountain = nearest_hex(276., -474.);
        for object in &additions {
            for column in &object.occupancy {
                assert!(
                    !column
                        .position
                        .checked_distance(fountain)
                        .is_ok_and(|d| d <= 3),
                    "discovery basin occupied by {}",
                    object.id
                );
            }
        }
        // Three wide entrances, the shrine centre, basin's southern access, and
        // the entire waterfall-gallery axis remain ordinary clear walking space.
        for (x, z, inside) in [
            (275., -512., false),
            (275., -457., false),
            (302., -490., false),
            (275., -490., false),
            (276., -465., false),
            (-370., -348., true),
            (-400., -530., true),
        ] {
            let centre = dressed.support(x, z, inside);
            for q in -1_i64..=1 {
                for r in -1_i64..=1 {
                    let p = WorldHex::new(centre.column.q + q, centre.column.r + r);
                    let [px, pz] = world_xz(p);
                    assert!(
                        dressed.clear_support(dressed.support(px, pz, inside), 8),
                        "access at {px},{pz}"
                    );
                }
            }
        }
    }
}
