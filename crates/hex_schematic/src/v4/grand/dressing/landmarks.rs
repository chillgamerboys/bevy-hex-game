//! Small, supported architectural groups around Grand's existing playable voids.
use super::*;

fn garden(g: &GrandCompiler, occupied: &Occupied) -> Result<Vec<ObjectInstance>, ContractError> {
    let garden_frame = g.geography.frame("garden")?;
    let mut frame = Cells::new();
    // Short north/west arcades frame the court without enclosing its south and
    // east approaches. Their overhead beams are well above ordinary headroom.
    for (x, z, along_x) in [(-27., 1., false), (-1., 24., true)] {
        let root = garden_frame.hex([x, z]);
        let base = g.surface(root).level + 1;
        for offset in -16_i32..=16 {
            if along_x && offset.abs() < 6 {
                continue;
            }
            let p = garden_frame.hex([
                x + if along_x { f64::from(offset) } else { 0. },
                z + if along_x { 0. } else { f64::from(offset) },
            ]);
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
    for (x, z) in [(-17., 14.), (15., 14.), (-19., -28.), (16., -33.)] {
        let root = garden_frame.hex([x, z]);
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
    let fountain = g.geography.frame("fountain")?;
    let root = fountain.hex([0., 0.]);
    let mut basin = Cells::new();
    for q in -4_i64..=4 {
        for r in -4_i64..=4 {
            if q.abs().max(r.abs()).max((q + r).abs()) != 4 {
                continue;
            }
            let p = WorldHex::new(root.q + q, root.r + r);
            let [_, north] = fountain.local(p);
            // The existing flowing outlet is untouched; the broad south break
            // permits an ordinary walk right to the original water cells.
            if g.surface(p).water.is_some()
                || (fountain
                    .local(p)
                    .first()
                    .is_some_and(|east| east.abs() < 3.5)
                    && north < 0.)
            {
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
            garden_frame.hex([-15., 10.]),
            frame,
        )?,
        object(
            g,
            "grand/garden-beds".into(),
            "decor/grand-garden-beds",
            garden_frame.hex([-17., 14.]),
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
    frame: &'static str,
}
const BAYS: &[Bay] = &[
    Bay {
        x: -52.,
        z: -39.,
        along_x: true,
        frame: "library_lower",
    },
    Bay {
        x: -24.,
        z: -39.,
        along_x: true,
        frame: "library_lower",
    },
    Bay {
        x: 4.,
        z: -39.,
        along_x: true,
        frame: "library_lower",
    },
    // The south-east diagonal is the real staircase entrance. This bay
    // belongs on the free north-west wall so both piers retain their feet.
    Bay {
        x: -42.,
        z: 33.,
        along_x: true,
        frame: "library_lower",
    },
    Bay {
        x: 3.,
        z: 33.,
        along_x: true,
        frame: "library_lower",
    },
    Bay {
        x: 33.,
        z: 33.,
        along_x: true,
        frame: "library_lower",
    },
    Bay {
        x: -65.,
        z: 27.,
        along_x: false,
        frame: "library_lower",
    },
    Bay {
        x: -65.,
        z: -25.,
        along_x: false,
        frame: "library_lower",
    },
    Bay {
        x: 70.,
        z: -25.,
        along_x: false,
        frame: "library_lower",
    },
    Bay {
        x: 55.,
        z: 30.,
        along_x: false,
        frame: "library_lower",
    },
    Bay {
        x: -32.,
        z: -4.,
        along_x: false,
        frame: "library_upper",
    },
    Bay {
        x: -32.,
        z: 16.,
        along_x: false,
        frame: "library_upper",
    },
    Bay {
        x: 30.,
        z: -4.,
        along_x: false,
        frame: "library_upper",
    },
];

fn bay(
    g: &GrandCompiler,
    index: usize,
    b: Bay,
    encounter_columns: &std::collections::BTreeSet<WorldHex>,
) -> Result<ObjectInstance, ContractError> {
    let mut object = bay_without_finish(g, index, b, encounter_columns)?;
    for column in &mut object.occupancy {
        for run in &mut column.runs {
            if run.material == "stone" {
                run.material = "limestone".into();
            }
        }
    }
    Ok(object)
}

fn bay_without_finish(
    g: &GrandCompiler,
    index: usize,
    b: Bay,
    encounter_columns: &std::collections::BTreeSet<WorldHex>,
) -> Result<ObjectInstance, ContractError> {
    let mut cells = Cells::new();
    let frame = g.geography.frame(b.frame)?;
    let base = g.support_at(&frame, [b.x, b.z])?.level + 1;
    // A wall bay has visible depth: shelves sit one hex behind the piers.
    // Every authored voxel is checked against the actual cave interval below,
    // so a stair crossing takes precedence over an ornament, never the reverse.
    for t in -8_i32..=8 {
        let p = frame.hex([
            b.x + if b.along_x { f64::from(t) } else { 0. },
            b.z + if b.along_x { 0. } else { f64::from(t) },
        ]);
        if t.abs() >= 7 {
            add(&mut cells, p, base, base + 34, "stone");
            add(&mut cells, p, base, base + 3, "slate");
            add(&mut cells, p, base + 29, base + 33, "sand");
        }
        let soffit = 27 + (8 - t.abs()) / 2;
        add(&mut cells, p, base + soffit, base + soffit + 3, "sand");
        if t.abs() <= 6 {
            // Recess along the inward/outward normal chosen for each side wall.
            let inward = if b.along_x || b.x > 0. { -1.5 } else { 1.5 };
            let shelf = frame.hex([
                b.x + if b.along_x { f64::from(t) } else { -inward },
                b.z + if b.along_x { -inward } else { f64::from(t) },
            ]);
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
        let Ok(support) = g.support_at(&frame, frame.local(*p)) else {
            return false;
        };
        if encounter_columns.contains(p) {
            return false;
        }
        let (column, _) = g.column(*p);
        ys.retain(|y, _| {
            *y > support.level
                && column.material_at(*y).is_none()
                && !g.reserved_interval(*p, *y, *y + 1)
        });
        !ys.is_empty()
    });
    object(
        g,
        format!("grand/library-bay/{index:02}"),
        "structure/grand-library-arcade",
        frame.hex([b.x, b.z]),
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
    fn library_pier_finish_preserves_complete_arcade_occupancy_and_grounding() {
        let compiler = test_compiler(false);
        let encounter_columns = std::collections::BTreeSet::new();
        let mut recolored = 0;
        for (index, spec) in BAYS.iter().copied().enumerate() {
            let before = bay_without_finish(&compiler, index, spec, &encounter_columns)
                .expect("original arcade");
            let mut finished =
                bay(&compiler, index, spec, &encounter_columns).expect("finished arcade");
            for column in &mut finished.occupancy {
                for run in &mut column.runs {
                    if run.material == "limestone" {
                        run.material = "stone".into();
                        recolored += 1;
                    }
                }
            }
            assert_eq!(finished, before, "arcade {index} changed beyond pier color");
        }
        assert!(recolored > 0);
    }

    #[test]
    fn complete_dressing_has_compatible_materials_in_every_shared_chunk() {
        let compiler = test_compiler(true);
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
        for name in ["garden", "fountain"] {
            let root = compiler
                .geography
                .frame(name)
                .expect("named garden frame")
                .hex([0., 0.]);
            let chunk = compiler
                .chunk(root.chunk())
                .expect("Garden composition must pass strict chunk sealing")
                .expect("Garden chunk exists");
            assert!(!chunk.semantics.object_influences.is_empty());
        }
        let objects: Vec<_> = compiler.objects.values().flatten().collect();
        assert!(!objects.iter().any(|object| object.id.contains("fort")));
        let camps: Vec<_> = objects
            .iter()
            .filter(|object| object.id.starts_with("grand/forest-camp/"))
            .collect();
        assert_eq!(camps.len(), 6);
        for camp in camps {
            assert!(camp
                .occupancy
                .iter()
                .flat_map(|c| &c.runs)
                .any(|run| run.material == "timber"));
            assert!(!camp
                .grounding
                .as_ref()
                .expect("hut and clearing contacts")
                .is_empty());
        }
        let rim = objects
            .iter()
            .find(|object| object.id == "grand/fountain-rim")
            .expect("the low fountain rim remains authored");
        assert!(!rim.occupancy.is_empty());
        let shrine = objects
            .iter()
            .find(|object| object.id == "grand/shrine/water")
            .expect("the Water Shrine remains authored");
        let mut occupied = Occupied::new();
        reserve(&mut occupied, shrine);
        assert!(
            !overlaps(&occupied, &rim.occupancy),
            "rim cannot replace a shrine post"
        );
    }

    #[test]
    fn courtyard_and_library_are_supported_and_preserve_exact_water_and_passages() {
        let plain = test_compiler(false);
        let water_shrine =
            shrine(&plain, "water", &Occupied::new()).expect("existing Water Shrine fixture");
        let additions = compose(&plain, &[water_shrine]).expect("bounded architecture");
        assert_eq!(additions.len(), 16);
        let mut occupied = Occupied::new();
        for object in &additions {
            assert!(!object.occupancy.is_empty(), "{}", object.id);
            let grounding = object.grounding.as_ref().expect("contacts");
            assert!(!grounding.is_empty(), "{}", object.id);
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
            for contact in grounding {
                let (terrain, _) = plain.column(contact.column);
                assert!(
                    terrain.material_at(contact.level).is_some(),
                    "{}",
                    object.id
                );
                assert!(
                    object.occupancy.iter().any(|column| {
                        column.position == contact.column
                            && column
                                .runs
                                .iter()
                                .any(|run| run.bottom == contact.level + 1)
                    }),
                    "contact must touch the actual architecture: {}",
                    object.id
                );
            }
            for column in &object.occupancy {
                let (terrain, _) = plain.column(column.position);
                for run in &column.runs {
                    assert!(
                        !terrain
                            .runs
                            .iter()
                            .any(|ground| ground.bottom < run.top && run.bottom < ground.top),
                        "buried architecture: {}",
                        object.id
                    );
                    assert!(
                        !plain.reserved_interval(column.position, run.bottom, run.top),
                        "reserved passage filled: {}",
                        object.id
                    );
                }
            }
            if object.id.starts_with("grand/library-") {
                assert!(
                    object
                        .occupancy
                        .iter()
                        .flat_map(|c| &c.runs)
                        .any(|run| run.material == "crystal"),
                    "filled shelves: {}",
                    object.id
                );
            }
        }
        let before = plain.sites(1).expect("plain sites");
        let dressed = test_compiler(true);
        let after = dressed
            .sites(1)
            .expect("all shrines and encounters retain headroom");
        assert!(
            !before.routes.is_empty(),
            "the producer must publish actual traversable routes"
        );
        for route in &before.routes {
            let new = after
                .routes
                .iter()
                .find(|new| new.id == route.id)
                .expect("same public route");
            assert_eq!(
                route.ribbon, new.ribbon,
                "dressing cannot move the supported route: {}",
                route.id
            );
            assert_eq!(
                route.supports, new.supports,
                "dressing cannot move route endpoints: {}",
                route.id
            );
            let clearance = i32::try_from(route.clearance_levels).expect("bounded body clearance");
            for support in &route.ribbon {
                assert!(
                    dressed.clear_support(*support, clearance),
                    "route obstructed: {} at {support:?}",
                    route.id
                );
            }
        }
        assert_eq!(before.fountains.len(), after.fountains.len());
        for fountain in &before.fountains {
            let new = after
                .fountains
                .iter()
                .find(|f| f.id == fountain.id)
                .expect("same fountain");
            assert_eq!(fountain.cells, new.cells);
            for cell in &fountain.cells {
                assert!(
                    occupied.get(&cell.column).is_none_or(|runs| runs
                        .iter()
                        .all(|(lo, hi)| *lo > cell.level || *hi <= cell.level)),
                    "fountain occupied"
                );
            }
        }
        for old in &before.encounters {
            let new = after
                .encounters
                .iter()
                .find(|e| e.id == old.id)
                .expect("same encounter");
            assert_eq!(
                old.surfaces, new.surfaces,
                "exact deployment remains clear: {}",
                old.id
            );
        }
        for id in [
            "shrine_water",
            "shrine_plant",
            "shrine_earth",
            "library_entrance",
            "root_temple_entrance",
        ] {
            let frame = dressed.geography.frame(id).expect("shared access frame");
            let centre = dressed
                .support_at(&frame, [0., 0.])
                .expect("named layer support");
            assert!(dressed.clear_support(centre, 8), "access blocked: {id}");
        }
    }
}
