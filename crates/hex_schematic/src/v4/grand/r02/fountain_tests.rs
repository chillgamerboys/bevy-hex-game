//! Emitted architectural geometry, not a substitute for ordinary controller proof.
use super::*;
use std::collections::{BTreeSet, VecDeque};

#[test]
fn fountain_south_steps_keep_the_pool_and_open_full_width_entry_and_exit() {
    let g = tests::compiler(true);
    let doc = g.geography.document.as_ref().expect("canonical geography");
    let entry = doc
        .fountain_entry
        .as_ref()
        .expect("authored southern entry");
    let frame = g.geography.frame("fountain").expect("fountain frame");
    let center = frame.hex([0., 0.]);

    // Load the previous document without the optional field through the normal
    // typed compiler. Existing content therefore retains its old basin profile.
    let mut old_json = serde_json::to_value(doc).expect("serializable geography");
    old_json
        .as_object_mut()
        .expect("document object")
        .remove("fountain_entry");
    let old_bytes = serde_json::to_vec(&old_json).expect("old geography bytes");
    let old_doc: GrandGeographyDocument = serde_json::from_slice(&old_bytes).expect("old schema");
    assert!(old_doc.fountain_entry.is_none());
    let old = GrandCompiler::with_geography(g.source.clone(), old_doc, &old_bytes)
        .expect("the unmodified basin remains valid content");
    assert_eq!(
        g.column(center),
        old.column(center),
        "central basin is unchanged"
    );
    assert_eq!(
        g.geography
            .frame("shrine_water")
            .expect("Water Shrine")
            .hex([0., 0.]),
        old.geography
            .frame("shrine_water")
            .expect("old Water Shrine")
            .hex([0., 0.])
    );
    let old_objects: BTreeMap<_, _> = old.objects.values().flatten().map(|o| (&o.id, o)).collect();
    let water_objects: Vec<_> = g
        .objects
        .values()
        .flatten()
        .filter(|o| o.id.contains("water"))
        .collect();
    assert!(
        !water_objects.is_empty(),
        "actual Water Shrine architecture"
    );
    for object in water_objects {
        assert_eq!(
            Some(&object),
            old_objects.get(&object.id),
            "Water Shrine architecture"
        );
    }

    let mut opening = BTreeMap::new();
    let mut court_domain = BTreeMap::new();
    let mut changed = 0;
    let mut rows = BTreeSet::new();
    let mut report = Vec::new();
    for q in -12_i64..=12 {
        for r in -12_i64..=12 {
            let p = WorldHex::new(center.q + q, center.r + r);
            let (column, liquid) = g.column(p);
            let (before, before_liquid) = old.column(p);
            let [east, north] = frame.local(p);
            let (distance, _, _) =
                geography::route_distance(g.geography.model_xz(p), &doc.fountain_rill.points);
            if g.fountain_entry_top(p).is_none() || distance < doc.fountain_rill.width * 0.5 {
                assert_eq!(column, before, "terrain changed outside the stair at {p:?}");
                assert_eq!(
                    liquid, before_liquid,
                    "water changed outside the stair at {p:?}"
                );
            }
            changed += usize::from(column != before);
            let top = column
                .runs
                .iter()
                .filter(|run| run.material != "water")
                .map(|run| run.top)
                .max()
                .expect("supported basin");
            let before_top = before
                .runs
                .iter()
                .filter(|run| run.material != "water")
                .map(|run| run.top)
                .max()
                .expect("old supported basin");
            if g.clear_support(
                VoxelPosition {
                    column: p,
                    level: top - 1,
                },
                8,
            ) {
                court_domain.insert(p, top);
            }
            if let Some(water) = &liquid {
                assert!(
                    column
                        .runs
                        .iter()
                        .filter(|run| run.material != "water")
                        .all(|run| run.top <= water.bottom || run.bottom >= water.top),
                    "solid/liquid overlap at {p:?}"
                );
                for influence in g.influences.get(&p.chunk()).into_iter().flatten() {
                    assert!(
                        influence
                            .occupancy
                            .iter()
                            .filter(|c| c.position == p)
                            .flat_map(|c| &c.runs)
                            .all(|run| run.top <= water.bottom || run.bottom >= water.top),
                        "architectural object overlaps emitted water at {p:?}"
                    );
                }
            }
            if east.abs() <= entry.half_width && (-entry.south_length..=0.).contains(&north) {
                let support = VoxelPosition {
                    column: p,
                    level: top - 1,
                };
                assert!(
                    g.clear_support(support, 8),
                    "full opening lacks headroom at {p:?}"
                );
                opening.insert(p, (top, liquid.as_ref().map(|w| (w.bottom, w.top))));
                rows.insert(p.r);
                report.push(serde_json::json!({"q":p.q,"r":p.r,"local":[east,north],
                    "old_exclusive_top":before_top,"exclusive_top":top,
                    "water":liquid.as_ref().map(|w| [w.bottom,w.top])}));
            }
        }
    }
    assert!(
        changed > 12 && changed < 64,
        "bounded whole entrance, changed {changed}"
    );
    assert!(
        opening.len() >= 24 && rows.len() >= 7,
        "complete broad stair, not one path"
    );
    let south: Vec<_> = opening
        .keys()
        .copied()
        .filter(|p| frame.local(*p)[1] < -entry.south_length + 2.)
        .collect();
    assert!(south.len() >= 3, "broad dry court connection");
    assert!(
        south
            .iter()
            .all(|p| opening.get(p).expect("known southern column").1.is_none()),
        "dry approach"
    );
    let mut reached: BTreeSet<_> = south.iter().copied().collect();
    let mut queue: VecDeque<_> = south.iter().copied().collect();
    let mut checked_edges = 0;
    while let Some(p) = queue.pop_front() {
        let &(top, _) = opening.get(&p).expect("known queued opening column");
        for n in p.neighbors().expect("bounded garden") {
            let Some(&(other, _)) = opening.get(&n) else {
                continue;
            };
            assert!(
                top.abs_diff(other) <= 1,
                "entry/exit riser {p:?}:{top} -> {n:?}:{other}"
            );
            // A human-radius capsule fits within a hex centre and its shared
            // edge midpoint. Eight clear levels at both ends exceed its height;
            // the one-voxel edge is admitted symmetrically, including the exit.
            for column in [p, n] {
                let floor = VoxelPosition {
                    column,
                    level: opening.get(&column).expect("known edge endpoint").0 - 1,
                };
                assert!(g.clear_support(floor, 8), "blocked stair edge {p:?}->{n:?}");
            }
            checked_edges += 1;
            if reached.insert(n) {
                queue.push_back(n);
            }
        }
    }
    assert_eq!(
        reached.len(),
        opening.len(),
        "every tread across the opening joins the court"
    );
    assert!(
        reached.contains(&center),
        "unchanged basin centre is reached and can exit"
    );
    // This is the unchanged endpoint of the production garden-ascent route.
    // Establish the real surrounding court connection as well as the stairs,
    // so an isolated but internally connected stair cannot pass this check.
    let court = g
        .layered
        .routes
        .iter()
        .find(|route| route.id == "garden_ascent")
        .and_then(|route| route.supports.last())
        .expect("existing court endpoint")
        .column;
    let mut court_reached = BTreeSet::from([court]);
    let mut court_queue = VecDeque::from([court]);
    while let Some(p) = court_queue.pop_front() {
        let top = court_domain.get(&p).expect("clear original court");
        for n in p.neighbors().expect("bounded court") {
            if court_domain
                .get(&n)
                .is_some_and(|other| top.abs_diff(*other) <= 1)
                && court_reached.insert(n)
            {
                court_queue.push_back(n);
            }
        }
    }
    assert!(
        opening.keys().all(|p| court_reached.contains(p)),
        "whole entrance must join the original court"
    );
    let water_top = g.geography.top_level(doc.fountain_basin.level);
    assert!(
        opening.values().any(|(top, water)| water
            .is_some_and(|(lo, hi)| lo == *top && hi == water_top && hi - lo == 1)),
        "shallow wet shelf"
    );
    let sites = g.sites(123).expect("new exact sites");
    let entrance = sites
        .route_nodes
        .iter()
        .find(|n| n.id == "fountain_entrance")
        .expect("published southern landing")
        .position;
    let basin = sites
        .route_nodes
        .iter()
        .find(|n| n.id == "fountain_basin")
        .expect("published stable basin centre")
        .position;
    assert_eq!(basin.column, center);
    assert_eq!(
        basin.level + 1,
        old.column(center)
            .0
            .runs
            .iter()
            .filter(|run| run.material != "water")
            .map(|run| run.top)
            .max()
            .expect("old basin floor")
    );
    assert!(south.contains(&entrance.column) && court_reached.contains(&entrance.column));
    assert!(
        g.column(entrance.column).1.is_none(),
        "ordinary dry landing"
    );
    for id in [
        "grand/anchor/fountain_entrance",
        "grand/anchor/fountain_basin",
    ] {
        assert_eq!(
            g.anchors
                .iter()
                .find(|a| a.id == id)
                .expect("ordinary anchor")
                .role,
            AnchorRole::Observation
        );
    }
    let fountain = sites
        .fountains
        .iter()
        .find(|f| f.id == "garden_fountain")
        .expect("stable fountain");
    let target = fountain
        .observation_target
        .expect("authored basin sight target");
    assert_eq!(target.column, basin.column);
    assert_eq!(target.level + 1, water_top);
    assert!(fountain.cells.contains(&target));
    assert!(!fountain
        .cells
        .iter()
        .any(|cell| cell.column == target.column && cell.level > target.level));

    for (&p, &(_, water)) in &opening {
        if let Some((lo, hi)) = water {
            for level in lo..hi {
                assert!(
                    fountain.cells.contains(&VoxelPosition { column: p, level }),
                    "new shelf liquid is discoverable/usable"
                );
            }
        }
    }
    for bad in [0., f64::NAN, 10.] {
        let mut invalid = doc.clone();
        invalid.fountain_entry.as_mut().expect("entry").tread_depth = bad;
        assert!(
            GrandGeography::new(invalid).is_err(),
            "invalid physical tread depth"
        );
    }
    if let Ok(path) = std::env::var("HEX_GRAND_FOUNTAIN_ENTRY_EXPORT") {
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "kind":"EXACT_EMITTED_SOURCE_STAIR_NOT_CONTROLLER_PROOF", "changed_columns":changed,
                "opening_columns":opening.len(),"bidirectional_edges":checked_edges,
                "central_column": [center.q,center.r],"water_top":water_top,"columns":report,
                "original_failed_controller_preserved":true
            }))
            .expect("bounded certificate"),
        )
        .expect("write bounded requested certificate");
    }
}
