//! Canonical revision02 integration invariants. Legacy absolute-coordinate
//! geometry is retained only by explicit legacy fixtures in its owning modules.
use super::*;
use std::collections::BTreeSet;

pub(super) fn compiler(dressed: bool) -> &'static GrandCompiler {
    static PLAIN: std::sync::OnceLock<GrandCompiler> = std::sync::OnceLock::new();
    static DRESSED: std::sync::OnceLock<GrandCompiler> = std::sync::OnceLock::new();
    if dressed {
        DRESSED.get_or_init(|| dressing::test_compiler(true))
    } else {
        PLAIN.get_or_init(|| dressing::test_compiler(false))
    }
}

// Independent sampling of the public authoring polylines, in their declared
// model frame. Emitted column facts below remain the assertions' authority.
pub(super) fn line_columns(g: &GrandCompiler, points: &[[f64; 3]]) -> Vec<WorldHex> {
    let mut out = Vec::new();
    for pair in points.windows(2) {
        let [a, b] = pair else {
            continue;
        };
        let [ax, _, az] = *a;
        let [bx, _, bz] = *b;
        let [ax, az] = g.geography.world_xz([ax, az]);
        let [bx, bz] = g.geography.world_xz([bx, bz]);
        let steps = ((bx - ax).hypot(bz - az) / 0.75).ceil() as i32;
        for step in 0..=steps.max(1) {
            let t = f64::from(step) / f64::from(steps.max(1));
            let p = nearest_hex(ax + (bx - ax) * t, az + (bz - az) * t);
            if out.last() != Some(&p) {
                out.push(p);
            }
        }
    }
    out
}

#[test]
fn full_measured_world_has_independent_crystal_and_exact_cave_sites() {
    let g = compiler(true);
    let d = g.geography.document.as_ref().expect("canonical geography");
    if d.landform_coast {
        assert_eq!(g.mainland_tolerance_columns, 13_065);
    } else {
        assert_eq!(g.mainland_columns, 653_261);
        assert_eq!(g.mainland_tolerance_columns, 65);
    }
    assert!(g.mainland_columns.abs_diff(93_326 * 7) <= g.mainland_tolerance_columns);
    assert_eq!(g.crystal.len(), d.ascent.expected_columns);
    assert_eq!(g.crystal_columns, g.crystal.len());
    // A geometric enclosing polygon is approximately sevenfold; the old
    // disconnected exact-count disc was not the visible whole Crystal feature.
    assert!(g.crystal_columns.abs_diff(3_169 * 7) < 3_169 / 100);
    assert!(g.crystal.contains(&g.geography.world_hex(d.ascent.center)));
    assert_eq!(
        g.source
            .mainland_rows
            .iter()
            .map(|(_, a, b)| usize::try_from(b - a + 1).expect("ordered admitted row"))
            .sum::<usize>(),
        g.mainland_columns
    );
    let sites = g.sites(123).expect("actual canonical sites");
    assert_eq!(sites.encounters.len(), 14);
    for site in &sites.encounters {
        assert!(g.clear_support(site.preferred, 16), "{}", site.id);
        assert!(site.surfaces.contains(&site.preferred));
    }
    let cave = g
        .anchors
        .iter()
        .find(|a| a.id == "grand/anchor/shadow_tunnel")
        .expect("Shadow anchor")
        .position;
    let (column, _) = g.column(cave.column);
    assert!(
        column
            .runs
            .iter()
            .any(|run| run.material != "water" && run.bottom > cave.level + 16),
        "actual covered bore"
    );
    let chunks: BTreeSet<_> = sites
        .encounters
        .iter()
        .map(|s| s.preferred.column.chunk())
        .collect();
    for id in chunks {
        g.chunk(id)
            .expect("site chunk compiles")
            .expect("inside package")
            .validate()
            .expect("exact chunk");
    }
}

#[test]
fn complete_stair_route_has_walkable_risers_and_tunnels_remain_separate() {
    let g = compiler(false);
    let sites = g.sites(9).expect("published canonical routes");
    assert!(sites.routes.len() >= 8);
    for route in &sites.routes {
        assert!(!route.supports.is_empty(), "{}", route.id);
        let clearance = i32::try_from(route.clearance_levels).expect("bounded body");
        for support in &route.supports {
            assert!(
                g.clear_support(*support, clearance),
                "blocked {} at {support:?}",
                route.id
            );
        }
        for pair in route.supports.windows(2) {
            let [a, b] = pair else {
                continue;
            };
            assert!(
                a.column.checked_distance(b.column).expect("bounded route") <= 1,
                "disconnected {}",
                route.id
            );
            assert!(
                (a.level - b.level).abs() <= 1,
                "unwalkable {}: {a:?} → {b:?}",
                route.id
            );
        }
    }
    // At any actual shared horizontal column the separate tunnel/library voids
    // need solid terrain between them. Their old fixed crossing is superseded.
    let mut shared = 0;
    for (p, layers) in &g.layered.columns {
        for shadow in layers
            .iter()
            .filter(|l| l.layer == SupportLayer::Shadow && !l.open)
        {
            for library in layers.iter().filter(|l| {
                matches!(
                    l.layer,
                    SupportLayer::LibraryLower | SupportLayer::LibraryUpper
                ) && !l.open
            }) {
                let (low, high) = if shadow.top < library.top {
                    (shadow, library)
                } else {
                    (library, shadow)
                };
                assert!(low.ceiling < high.top, "unintended cave join at {p:?}");
                let (column, _) = g.column(*p);
                assert!(
                    column.runs.iter().any(|run| run.material != "water"
                        && run.bottom <= low.ceiling
                        && run.top >= high.top - 2),
                    "missing cave separator at {p:?}"
                );
                shared += 1;
            }
        }
    }
    println!("R02_CAVE_SEPARATION shared_columns={shared}");
    assert!(!sites.fountains.first().expect("fountain").cells.is_empty());
    let start = g
        .anchors
        .iter()
        .find(|a| a.id == "grand/anchor/party_start")
        .expect("start")
        .position;
    assert!(g.clear_support(start, 8));
    assert!(g.column(start.column).1.is_none(), "the start is dry");
    let [x, z] = world_xz(start.column);
    assert_ne!(
        g.biomes(9)
            .label_at([x, f64::from(start.level + 1) * LEVEL_HEIGHT, z]),
        Some("Open Sea")
    );
}

#[test]
fn shadow_route_walks_from_south_mouth_to_open_crystal_landing() {
    let g = compiler(true);
    let sites = g.sites(1).expect("dressed routes");
    let shadow = sites
        .routes
        .iter()
        .find(|r| r.id == "shadow_tunnel")
        .expect("Shadow route");
    let crystal = sites
        .routes
        .iter()
        .find(|r| r.id == "crystal_ascent")
        .expect("Crystal route");
    let frozen = sites
        .routes
        .iter()
        .find(|r| r.id == "frozen_shore")
        .expect("Frozen route");
    let exit = shadow.supports.last().expect("Shadow exit");
    let first_tread = crystal.supports.first().expect("bottom ascent landing");
    // The two routes enter different sides of the same open well. Their full
    // floor connection is checked by the composed well-floor graph fixture;
    // neither route is required to extend across the other one's endpoint.
    assert!(
        g.clear_support(*exit, 8) && g.clear_support(*first_tread, 8),
        "both routes meet supported open-well floor"
    );
    assert_eq!(
        crystal.supports.last(),
        frozen.supports.first(),
        "ascent top meets the continuous woods route"
    );
    assert!(g.crystal.contains(&exit.column));
    assert!(
        g.column(exit.column)
            .0
            .runs
            .iter()
            .filter(|run| run.material != "water")
            .all(|run| run.top <= exit.level + 1),
        "open Crystal landing"
    );
    let entrance = g
        .anchors
        .iter()
        .find(|a| a.id == "grand/anchor/shadow_entrance")
        .expect("mouth");
    assert_eq!(shadow.supports.first(), Some(&entrance.position));
    for support in &shadow.ribbon {
        assert!(
            g.clear_support(*support, 8),
            "full tunnel body width at {support:?}"
        );
    }
    let earth = g.geography.frame("shrine_earth").expect("Earth at base");
    let floor = g.support_at(&earth, [0., 0.]).expect("Earth floor");
    let source = g.geography.document.as_ref().expect("canonical geography");
    assert_eq!(floor.level + 1, g.geography.top_level(source.ascent.base));
}

#[test]
fn watercourse_is_continuous_and_descends_from_garden_to_open_sea() {
    let g = compiler(false);
    let source = g.geography.document.as_ref().expect("canonical geography");
    for (name, channel) in [
        ("fountain", &source.fountain_rill),
        ("falls", &source.falls),
        ("river", &source.river),
    ] {
        let mut previous = None;
        for p in line_columns(g, &channel.points) {
            let (column, liquid) = g.column(p);
            let water = liquid.expect("actual continuous wet centerline");
            assert_eq!(
                column.material_at(water.top - 1),
                Some("water"),
                "{name} at {p:?}"
            );
            assert!(water.bottom < water.top);
            if let Some(top) = previous {
                assert!(
                    water.top <= top + 1,
                    "uphill {name} at {p:?}: {top} → {}",
                    water.top
                );
            }
            previous = Some(water.top);
        }
    }
    let end = line_columns(g, &source.river.points)
        .last()
        .copied()
        .expect("river end");
    assert_eq!(g.column(end).1.expect("sea receiver").top, SEA_TOP);
}

#[test]
fn offshore_sailing_reference_has_clear_sea_between_launch_and_landing() {
    let g = compiler(false);
    let d = g.geography.document.as_ref().expect("canonical geography");
    let a = g
        .anchors
        .iter()
        .find(|a| a.id == "grand/anchor/sailing_start")
        .expect("launch")
        .position;
    let b = g
        .anchors
        .iter()
        .find(|a| a.id == "grand/anchor/volcano_landing")
        .expect("landing")
        .position;
    let [ax, az] = world_xz(a.column);
    let [bx, bz] = world_xz(b.column);
    let distance = (ax - bx).hypot(az - bz);
    assert!(
        g.column(a.column).1.is_some_and(|l| l.top == SEA_TOP),
        "launch in actual sea"
    );
    assert!(g.clear_support(b, 8), "dry supported destination");
    let mut sea_samples = 0;
    let mut reached_island = false;
    let steps = distance.ceil() as i32;
    for step in 0..=steps {
        let t = f64::from(step) / f64::from(steps.max(1));
        let p = nearest_hex(ax + (bx - ax) * t, az + (bz - az) * t);
        assert!(!g.mainland(p), "sailing line crosses mainland at {p:?}");
        let wet = g.column(p).1.is_some_and(|l| l.top == SEA_TOP);
        if wet {
            assert!(!reached_island, "unexpected dry barrier before destination");
            sea_samples += 1;
        } else {
            assert!(
                geography::irregular(
                    g.geography.model_xz(p),
                    d.volcano.center,
                    d.volcano.radii,
                    d.volcano.phase,
                ) < 1.2,
                "unexpected offshore obstacle"
            );
            reached_island = true;
        }
    }
    assert!(sea_samples > 0 && reached_island);
    // The previous 793.9485u/45s calibration belongs to the old route. Actual
    // favorable-wind controller timing is measured by the explicit-package probe.
    println!(
        "R02_SAILING_GEOMETRY anchor_distance={distance:.3} sea_samples={sea_samples}/{steps}; timing_not_verified_here"
    );
}

#[test]
fn bounded_dressing_keeps_temple_and_encounter_approaches_open() {
    let g = compiler(true);
    let objects: Vec<_> = g.objects.values().flatten().collect();
    assert!(objects.len() < 900, "bounded authored objects");
    assert_eq!(
        objects
            .iter()
            .filter(|o| o.id.starts_with("grand/forest-camp/"))
            .count(),
        6
    );
    assert_eq!(
        objects
            .iter()
            .filter(|o| o.id.starts_with("grand/coastal-rock/"))
            .count(),
        4
    );
    assert!(
        !objects.iter().any(|o| o.id == "grand/goblin-fort"),
        "the fort was superseded by open camps"
    );
    assert!(objects.iter().any(|o| o.id == "grand/root-temple-plant"));
    for id in [
        "grand/fire-flame-marker",
        "grand/air-spiral-marker",
        "grand/earth-heart-marker",
    ] {
        let marker = objects
            .iter()
            .find(|object| object.id == id)
            .expect("temple marker");
        assert!(marker.occupancy.len() <= 19, "small marker: {id}");
        assert!(
            !marker.grounding.as_ref().expect("contacts").is_empty(),
            "{id}"
        );
    }
    assert_eq!(
        g.overview().building_count,
        objects
            .iter()
            .filter(|o| !o.asset.starts_with("plant/"))
            .count()
    );
    let tree = objects
        .iter()
        .find(|o| o.id == "grand/world-tree")
        .expect("World Tree");
    let tree_chunks: BTreeSet<_> = tree.occupancy.iter().map(|c| c.position.chunk()).collect();
    assert!(tree_chunks.len() <= 128);
    assert!(tree.occupancy.len() <= 20_000);
    assert!(
        tree.occupancy.iter().any(|c| c
            .position
            .checked_distance(tree.origin.column)
            .expect("bounded tree")
            > 20
            && c.runs
                .iter()
                .any(|r| r.material == "timber" && r.bottom > tree.origin.level + 100)),
        "outer canopy has supporting branches"
    );
    assert!(
        tree.occupancy.iter().any(|c| c
            .position
            .checked_distance(tree.origin.column)
            .expect("bounded tree")
            > 40
            && c.runs
                .iter()
                .any(|r| r.material == "timber" && r.bottom == g.surface(c.position).level + 1)),
        "roots spread beyond the trunk"
    );
    let sites = g.sites(1).expect("dressed sites");
    for site in &sites.encounters {
        assert!(g.clear_support(site.preferred, 16), "{}", site.id);
        assert!(
            site.surfaces.len() >= 50,
            "deployment footprint: {}",
            site.id
        );
    }
    for anchor in &g.anchors {
        assert!(
            anchor
                .position
                .column
                .checked_distance(WorldHex::new(0, 0))
                .expect("bounded anchor")
                <= RADIUS as u64,
            "anchor outside admitted world: {}",
            anchor.id
        );
        assert!(
            (0..=MAX_LEVEL).contains(&anchor.position.level),
            "anchor outside vertical bounds: {}",
            anchor.id
        );
        if matches!(anchor.role, AnchorRole::Gameplay | AnchorRole::Transit) {
            assert!(
                g.clear_support(anchor.position, 8),
                "gameplay anchor blocked: {}",
                anchor.id
            );
        }
    }
    let temple = sites
        .routes
        .iter()
        .find(|r| r.id == "root_temple")
        .expect("temple approach");
    for support in &temple.ribbon {
        assert!(
            g.clear_support(*support, 8),
            "temple approach at {support:?}"
        );
    }
    // Exact material/grounding admission for all special architecture and tree
    // chunks. Undecorated finite descriptors remain synthetic, never payloads.
    let chunks: BTreeSet<_> = objects
        .iter()
        .filter(|o| !o.id.starts_with("grand/tree/") && !o.id.starts_with("grand/forest-detail/"))
        .flat_map(|o| {
            o.occupancy
                .iter()
                .map(|c| c.position.chunk())
                .chain(std::iter::once(o.origin.column.chunk()))
        })
        .collect();
    let mut manifest = g.manifest();
    // Manifest admission requires the complete finite coordinate catalogue,
    // even when this focused test materializes only decorated chunks. Checksums
    // of the unvisited chunks are synthetic and are never admitted as payloads.
    for id in g.chunk_ids() {
        let origin = id.origin().unwrap();
        if (0..CHUNK_SIZE).any(|q| {
            (0..CHUNK_SIZE).any(|r| {
                WorldHex::new(origin.q + q, origin.r + r)
                    .checked_distance(WorldHex::new(0, 0))
                    .unwrap()
                    <= RADIUS as u64
            })
        }) {
            manifest.chunks.push(ChunkDescriptor {
                coordinate: id,
                fingerprint: 1,
                path: format!("chunks/{}_{}.ron", id.q, id.r),
            });
        }
    }
    let mut packages = vec![];
    for id in chunks {
        let package = g.chunk(id).unwrap().unwrap();
        manifest.features.extend(package.features.iter().cloned());
        manifest
            .chunks
            .iter_mut()
            .find(|d| d.coordinate == id)
            .unwrap()
            .fingerprint = package.fingerprint;
        packages.push(package);
    }
    manifest.seal().unwrap();
    let index = ManifestIndex::new(std::sync::Arc::new(manifest)).unwrap();
    for package in packages {
        package.validate_with_index(&index).unwrap();
    }
}

#[test]
fn volcanic_caldera_stays_below_its_rim_and_preserves_exact_sites() {
    let g = compiler(false);
    let source = g.geography.document.as_ref().expect("canonical geography");
    let [cx, cz] = source.volcano.center;
    let center = g.column(g.geography.world_hex([cx, cz])).0;
    let bottom = center
        .runs
        .iter()
        .filter(|r| r.material != "water")
        .map(|r| r.top)
        .max()
        .expect("caldera floor");
    let [rx, rz] = source.caldera.radii;
    let mut highest = bottom;
    for sample in 0..24 {
        let angle = f64::from(sample) * std::f64::consts::TAU / 24.;
        let p = g
            .geography
            .world_hex([cx + rx * angle.cos(), cz + rz * angle.sin()]);
        let height = g
            .column(p)
            .0
            .runs
            .iter()
            .filter(|r| r.material != "water")
            .map(|r| r.top)
            .max()
            .expect("rim support");
        highest = highest.max(height);
    }
    assert!(
        highest - bottom >= 80,
        "visible caldera depth survives ordinary access cuts"
    );
    let frame = g.geography.frame("shrine_fire").expect("Fire frame");
    let fire = g.support_at(&frame, [0., 0.]).expect("exact shrine floor");
    assert!(g.clear_support(fire, 8));
    let sites = g.sites(1).expect("actual routes");
    let route = sites
        .routes
        .iter()
        .find(|r| r.id == "volcano_ascent")
        .expect("landing to crater");
    assert_eq!(route.supports.last(), Some(&fire));
}
