use super::*;
use std::collections::BTreeMap;

fn field(sample: impl Fn(i16, i16) -> (i16, u8)) -> TerrainSurfaceOverview {
    let mut overview = TerrainSurfaceOverview {
        halo: Vec::new(),
        version: 1,
        tolerance: 2.0,
        profiles: Vec::new(),
        chunks: Vec::new(),
    };
    let mut intern = BTreeMap::new();
    for q in -1..=2 {
        for r in -1..=2 {
            let mut profiles = Vec::new();
            let mut protection = Vec::new();
            for lr in 0..16 {
                for lq in 0..16 {
                    let (top, flags) = sample(q * 16 + lq, r * 16 + lr);
                    let id = *intern.entry(top).or_insert_with(|| {
                        let id =
                            u16::try_from(overview.profiles.len()).expect("fixture profile index");
                        overview.profiles.push(SolidProfile {
                            runs: vec![SolidRun {
                                bottom: 0,
                                top,
                                material: 0,
                            }],
                        });
                        id
                    });
                    profiles.push(id);
                    protection.push(flags);
                }
            }
            overview.chunks.push(SurfaceChunk {
                coordinate: ChunkId {
                    q: i64::from(q),
                    r: i64::from(r),
                },
                profiles,
                protection,
                surface: None,
            });
        }
    }
    overview
}
fn builder(
    overview: &TerrainSurfaceOverview,
) -> Result<SurfaceBuilder<'_>, hex_world_contracts::ContractError> {
    SurfaceBuilder::new(
        overview,
        128,
        [0, 3000],
        &[hex_world_contracts::MaterialSpec {
            id: "stone".into(),
            solid: true,
            diggable: true,
            color: [100, 100, 100, 255],
        }],
        0.35,
    )
}
fn build_patch(
    overview: &TerrainSurfaceOverview,
    coordinate: ChunkId,
    _height: f64,
) -> Result<MacroSurface, hex_world_contracts::ContractError> {
    builder(overview)?.build_patch(coordinate)
}
fn certify_patch(
    overview: &TerrainSurfaceOverview,
    coordinate: ChunkId,
    surface: &MacroSurface,
    _height: f64,
) -> Result<f64, hex_world_contracts::ContractError> {
    builder(overview)?.certify_patch(coordinate, surface)
}
fn build(overview: &TerrainSurfaceOverview, coordinate: ChunkId) -> MacroSurface {
    let surface = build_patch(overview, coordinate, 0.35).expect("certified constrained patch");
    assert!(
        certify_patch(overview, coordinate, &surface, 0.35).expect("independent certificate")
            <= 2.0 + 1e-8
    );
    surface
}

#[test]
fn native_constraints_preserve_holes_collinear_boundaries_and_chunk_corners() {
    let overview = field(|q, r| {
        (
            100 + q + r,
            if (7..=9).contains(&q) && (7..=9).contains(&r) {
                OBJECT_CONTACT
            } else {
                0
            },
        )
    });
    let coordinate = ChunkId { q: 0, r: 0 };
    let surface = build(&overview, coordinate);
    let domain = build::Domain::new(&build::Source::new(&overview), coordinate).expect("domain");
    assert_eq!(domain.cells.len(), 247);
    assert!(domain.boundary.len() > 120);
    let mut missing = surface.clone();
    missing.triangles.pop();
    assert!(certify_patch(&overview, coordinate, &missing, 0.35).is_err());
    let mut duplicate = surface.clone();
    duplicate
        .triangles
        .push(*surface.triangles.first().expect("triangle"));
    assert!(certify_patch(&overview, coordinate, &duplicate, 0.35).is_err());
    assert_eq!(
        surface,
        build(&overview, coordinate),
        "canonical ordering is stable"
    );
}

#[test]
fn independently_built_adjacent_chunks_agree_at_every_shared_lattice_point() {
    let overview = field(|q, r| (100 + q + r, 0));
    let a = build(&overview, ChunkId { q: 0, r: 0 });
    let b = build(&overview, ChunkId { q: 1, r: 0 });
    let points: BTreeMap<_, _> = a
        .vertices
        .iter()
        .map(|v| (v.lattice, v.half_level))
        .collect();
    let mut shared = 0;
    for vertex in &b.vertices {
        let [q, r] = vertex.lattice;
        if let Some(height) = points.get(&[q + 48, r]) {
            assert_eq!(*height, vertex.half_level);
            shared += 1;
        }
    }
    assert!(shared >= 30, "all native interface constraints survive");
    let mut incomplete = overview.clone();
    incomplete.chunks.retain(|c| c.coordinate.q >= 0);
    assert!(
        build_patch(&incomplete, ChunkId { q: 0, r: 0 }, 0.35).is_err(),
        "missing halo never invents an edge height"
    );
}

#[test]
fn sharp_well_breakline_remains_a_hole_without_corner_max_lifting() {
    // The measured Crystal source jump is 285.95 -> 185.50 (817 -> 530).
    let overview = field(|q, _| {
        (
            if q < 8 { 817 } else { 530 },
            if q == 7 || q == 8 { CLIFF } else { 0 },
        )
    });
    let surface = build(&overview, ChunkId { q: 0, r: 0 });
    assert!(surface
        .vertices
        .iter()
        .all(|v| v.half_level == 1634 || v.half_level == 1060));
    let domain =
        build::Domain::new(&build::Source::new(&overview), ChunkId { q: 0, r: 0 }).expect("domain");
    assert_eq!(domain.cells.len(), 224);
    // The exact side owner retains both heights and full profiles separately;
    // the approximate cap cannot triangulate across this deliberate gap.
    let incompatible = field(|q, _| (if q < 8 { 817 } else { 530 }, 0));
    assert!(build_patch(&incompatible, ChunkId { q: 0, r: 0 }, 0.35).is_err());
}

#[test]
fn certificate_refines_interior_relief_and_rejects_modified_export() {
    let overview = field(|q, r| {
        let d = (q - 7).abs().max((r - 7).abs());
        (130 - 2 * d.min(15), 0)
    });
    let coordinate = ChunkId { q: 0, r: 0 };
    let surface = build(&overview, coordinate);
    assert!(surface.maximum_error <= 2.0 + 1e-8);
    let mut modified = surface.clone();
    modified.vertices.first_mut().expect("vertex").half_level += 50;
    assert!(certify_patch(&overview, coordinate, &modified, 0.35).is_err());
    let mut reversed = surface;
    reversed.triangles.first_mut().expect("triangle").swap(1, 2);
    assert!(certify_patch(&overview, coordinate, &reversed, 0.35).is_err());
}

#[test]
fn recertifier_refuses_nonfinite_metadata_and_high_edge_incidence() {
    let overview = field(|_, _| (100, 0));
    let coordinate = ChunkId { q: 0, r: 0 };
    let surface = build(&overview, coordinate);
    let mut bad = surface.clone();
    bad.maximum_error = f64::NAN;
    assert!(certify_patch(&overview, coordinate, &bad, 0.35).is_err());
    let mut bad_overview = overview.clone();
    bad_overview.tolerance = f64::NAN;
    assert!(certify_patch(&bad_overview, coordinate, &surface, 0.35).is_err());
    let mut repeated = surface.clone();
    repeated.triangles = vec![*surface.triangles.first().expect("triangle"); 256];
    assert!(certify_patch(&overview, coordinate, &repeated, 0.35).is_err());
}

#[test]
fn recertification_rejects_within_tolerance_boundary_shift_and_duplicate_xy() {
    let overview = field(|_, _| (100, 0));
    let coordinate = ChunkId { q: 0, r: 0 };
    let surface = build(&overview, coordinate);
    let mut shifted = surface.clone();
    for vertex in &mut shifted.vertices {
        vertex.half_level += 1;
    }
    shifted.maximum_error = 0.175;
    assert!(certify_patch(&overview, coordinate, &shifted, 0.35).is_err());
    let mut duplicate = surface;
    duplicate
        .vertices
        .push(*duplicate.vertices.first().expect("vertex"));
    assert!(certify_patch(&overview, coordinate, &duplicate, 0.35).is_err());
}

#[test]
fn checked_batch_refuses_truncated_duplicate_or_unprotected_stacked_profiles() {
    let overview = field(|_, _| (100, 0));
    let mut short = overview.clone();
    short.chunks.first_mut().expect("chunk").protection.pop();
    assert!(builder(&short).is_err());
    let mut duplicate = overview.clone();
    duplicate
        .chunks
        .push(duplicate.chunks.first().expect("chunk").clone());
    assert!(builder(&duplicate).is_err());
    let mut stacked = overview;
    stacked.profiles.first_mut().expect("profile").runs = vec![
        SolidRun {
            bottom: 0,
            top: 50,
            material: 0,
        },
        SolidRun {
            bottom: 80,
            top: 100,
            material: 0,
        },
    ];
    assert!(builder(&stacked).is_err());
}

#[test]
fn interval_faces_keep_well_cliff_and_stacked_floors_roofs_and_voids() {
    let mut overview = field(|q, _| (if q < 8 { 817 } else { 530 }, CLIFF));
    let coordinate = ChunkId { q: 0, r: 0 };
    let sample = builder(&overview).expect("checked source");
    let surface = sample.build_patch(coordinate).expect("empty macro domain");
    let faces = sample
        .build_faces(coordinate, &surface, 30_000)
        .expect("exact cliff faces");
    assert!(
        faces
            .faces
            .iter()
            .filter(|f| f.kind == FaceKind::Side)
            .any(|f| {
                let lo = f
                    .points
                    .iter()
                    .map(|p| p.level)
                    .fold(f64::INFINITY, f64::min);
                let hi = f
                    .points
                    .iter()
                    .map(|p| p.level)
                    .fold(f64::NEG_INFINITY, f64::max);
                (lo - 530.0).abs() < 1e-9 && (hi - 817.0).abs() < 1e-9
            }),
        "the real 100.45-unit well face is preserved, never welded upward"
    );
    assert!(
        sample.build_faces(coordinate, &surface, 3).is_err(),
        "budget never truncates"
    );
    let cavity = SolidProfile {
        runs: vec![
            SolidRun {
                bottom: 0,
                top: 80,
                material: 0,
            },
            SolidRun {
                bottom: 100,
                top: 110,
                material: 0,
            },
        ],
    };
    overview.profiles = vec![cavity];
    for chunk in &mut overview.chunks {
        chunk.profiles.fill(0);
        chunk.protection.fill(STACKED);
    }
    let sample = builder(&overview).expect("stacked source");
    let surface = sample.build_patch(coordinate).expect("protected only");
    let faces = sample
        .build_faces(coordinate, &surface, 30_000)
        .expect("cavity faces");
    assert_eq!(faces.faces.len(), 256 * 3);
    assert_eq!(
        faces
            .faces
            .iter()
            .filter(|f| f.kind == FaceKind::Ceiling)
            .count(),
        256
    );
    for face in faces.faces {
        let expected = if face.kind == FaceKind::Ceiling {
            100.0
        } else {
            let level = face.points.first().expect("face point").level;
            assert!((level - 80.0).abs() < 1e-9 || (level - 110.0).abs() < 1e-9);
            level
        };
        assert!(face
            .points
            .iter()
            .all(|p| (p.level - expected).abs() < 1e-9));
    }
}

#[test]
fn macro_interfaces_clip_material_strata_without_inverting_or_filling_voids() {
    let mut overview = field(|q, r| (100 + q + r, if q == 8 { CLIFF } else { 0 }));
    // Two opaque material strata without a gap. The upper one can end below a
    // macro corner, or be partly clipped by it; neither exposes a buried cap.
    for profile in &mut overview.profiles {
        let top = profile.runs.first().expect("run").top;
        profile.runs = vec![
            SolidRun {
                bottom: 0,
                top: top - 1,
                material: 0,
            },
            SolidRun {
                bottom: top - 1,
                top,
                material: 1,
            },
        ];
    }
    let materials = vec![
        hex_world_contracts::MaterialSpec {
            id: "rock".into(),
            solid: true,
            diggable: true,
            color: [100, 100, 100, 255],
        },
        hex_world_contracts::MaterialSpec {
            id: "soil".into(),
            solid: true,
            diggable: true,
            color: [110, 80, 60, 255],
        },
    ];
    let source =
        SurfaceBuilder::new(&overview, 128, [0, 3000], &materials, 0.35).expect("material source");
    let c = ChunkId { q: 0, r: 0 };
    let patch = source.build_patch(c).expect("constrained cap");
    let faces = source
        .build_faces(c, &patch, 30_000)
        .expect("clipped faces");
    assert!(faces.faces.iter().any(|f| f.kind == FaceKind::Side));
    assert_eq!(
        faces
            .faces
            .iter()
            .filter(|f| f.kind == FaceKind::Cap)
            .count(),
        16
    );
    assert!(!faces.faces.iter().any(|f| f.kind == FaceKind::Ceiling));
    assert!(faces
        .faces
        .iter()
        .all(|f| f.points.iter().all(|p| p.level.is_finite())));
    // Sample every side triangle's interior: exactly one rendered solid owner
    // must cover the point. Shared boundary coordinates alone are insufficient.
    for face in faces.faces.iter().filter(|f| f.kind == FaceKind::Side) {
        let a = face.points.first().expect("polygon start");
        for pair in face.points.windows(2).skip(1) {
            let [b, c] = pair else { unreachable!() };
            let point = [
                (a.lattice[0] + b.lattice[0] + c.lattice[0]) / 3.0,
                (a.lattice[1] + b.lattice[1] + c.lattice[1]) / 3.0,
            ];
            let level = (a.level + b.level + c.level) / 3.0;
            let mut covered = 0;
            for r in -1..=16_i16 {
                for q in -1..=16_i16 {
                    if !certificate::inside_hex(point, [q, r]) {
                        continue;
                    }
                    let top = 100.0 + f64::from(q + r);
                    let cap = if q == 8 {
                        top
                    } else {
                        let domain = build::Domain::new(&source.source, ChunkId { q: 0, r: 0 })
                            .expect("domain");
                        let corners = build::CORNERS.map(|[a, b]| [3 * q + a, 3 * r + b]);
                        corners
                            .iter()
                            .copied()
                            .zip(corners.iter().copied().cycle().skip(1))
                            .take(6)
                            .find_map(|(a, b)| {
                                let [ax, az] = a.map(f64::from);
                                let [bx, bz] = b.map(f64::from);
                                let dx = bx - ax;
                                let dz = bz - az;
                                let cross = dx * (point[1] - az) - dz * (point[0] - ax);
                                let t = ((point[0] - ax) * dx + (point[1] - az) * dz)
                                    / (dx * dx + dz * dz);
                                if cross.abs() > 1e-7 || !(-1e-7..=1.0 + 1e-7).contains(&t) {
                                    return None;
                                }
                                let ha = f64::from(
                                    domain.vertex(&source.source, a).expect("corner").half_level,
                                ) * 0.5;
                                let hb = f64::from(
                                    domain.vertex(&source.source, b).expect("corner").half_level,
                                ) * 0.5;
                                Some(ha + (hb - ha) * t)
                            })
                            .expect("side sample lies on the actual hex boundary")
                    };
                    if level > 0.0 && level < cap - 1e-8 {
                        covered += 1;
                    }
                }
            }
            assert_eq!(
                covered, 1,
                "exact/macro side must separate solid from air at {point:?}/{level}"
            );
        }
    }
}

#[test]
#[ignore = "explicit immutable package, profile fixture and fresh output directory required"]
fn actual_crystal_profiles_export_certified_caps_and_exact_interval_faces() {
    use hex_world_contracts::{ChunkPackage, WorldManifest};
    use std::path::PathBuf;
    let root =
        PathBuf::from(std::env::var("HEX_SURFACE_PACKAGE").expect("explicit immutable package"));
    let input =
        PathBuf::from(std::env::var("HEX_SURFACE_FIXTURE").expect("explicit profile fixture"));
    let output =
        PathBuf::from(std::env::var("HEX_SURFACE_OUTPUT").expect("fresh output directory"));
    let expected: u64 = std::env::var("HEX_SURFACE_EXPECTED_PACKAGE")
        .expect("expected package identity")
        .parse()
        .expect("u64 package identity");
    assert!(
        !output.exists(),
        "never replace an earlier prototype receipt"
    );
    let mut source: TerrainSurfaceOverview =
        ron::from_str(&std::fs::read_to_string(&input).expect("fixture read"))
            .expect("fixture parse");
    let overview: super::super::NorthernOverview = ron::from_str(
        &std::fs::read_to_string(root.join("grand-overview.ron")).expect("overview read"),
    )
    .expect("overview parse");
    let manifest: WorldManifest =
        ron::from_str(&std::fs::read_to_string(root.join("manifest.ron")).expect("manifest read"))
            .expect("manifest parse");
    manifest.validate().expect("sealed manifest");
    assert_eq!(manifest.fingerprint, expected);
    assert_eq!(overview.package_fingerprint, expected);
    assert_eq!(overview.source_fingerprint, manifest.source_fingerprint);
    let materials: BTreeMap<_, _> = overview
        .materials
        .iter()
        .enumerate()
        .map(|(i, m)| {
            (
                m.id.as_str(),
                (u16::try_from(i).expect("material index"), m.solid),
            )
        })
        .collect();
    // The external selection/protection fixture must reproduce every actual
    // solid interval in its entire source + halo, not only the four roof tops.
    for chunk in &source.chunks {
        let id = chunk.coordinate;
        let descriptor = manifest
            .chunks
            .iter()
            .find(|d| d.coordinate == id)
            .expect("manifest chunk");
        let package: ChunkPackage = ron::from_str(
            &std::fs::read_to_string(root.join(&descriptor.path)).expect("source chunk read"),
        )
        .expect("source chunk parse");
        package
            .validate_against_manifest(&manifest)
            .expect("sealed source chunk");
        let actual: BTreeMap<_, _> = package.columns.iter().map(|c| (c.position, c)).collect();
        let origin = id.origin().expect("origin");
        for (i, &profile) in chunk.profiles.iter().enumerate() {
            let i = i64::try_from(i).expect("local index");
            let p = origin
                .checked_add(hex_world_contracts::WorldHex::new(i % 16, i / 16))
                .expect("column");
            let Some(column) = actual.get(&p) else {
                assert_eq!(profile, OUTSIDE_PROFILE);
                continue;
            };
            let expected: Vec<_> = column
                .runs
                .iter()
                .filter_map(|run| {
                    let &(material, solid) = materials
                        .get(run.material.as_str())
                        .expect("known material");
                    solid.then(|| SolidRun {
                        bottom: i16::try_from(run.bottom).expect("level"),
                        top: i16::try_from(run.top).expect("level"),
                        material,
                    })
                })
                .collect();
            assert_eq!(
                source
                    .profiles
                    .get(usize::from(profile))
                    .expect("profile")
                    .runs,
                expected,
                "complete source profile at {p:?}"
            );
        }
    }
    let builder = SurfaceBuilder::new(
        &source,
        overview.radius,
        overview.level_bounds,
        &overview.materials,
        f64::from(overview.level_height),
    )
    .expect("checked source");
    let mut receipts = Vec::new();
    let mut patches = Vec::new();
    let mut exported = Vec::new();
    for q in 28..=29 {
        for r in -20..=-19 {
            let coordinate = ChunkId { q, r };
            let surface = builder
                .build_patch(coordinate)
                .expect("actual constrained surface");
            let faces = builder
                .build_faces(coordinate, &surface, 65_536)
                .expect("actual exact interval faces");
            receipts.push(serde_json::json!({"chunk":[q,r],"cap_vertices":surface.vertices.len(),"cap_triangles":surface.triangles.len(),"maximum_error":surface.maximum_error,"unshared_face_vertices":faces.vertices,"face_triangles":faces.triangles}));
            exported.extend(faces.faces.iter().map(|face| serde_json::json!({"chunk":[q,r],"kind":format!("{:?}",face.kind),"material":face.material,"points":face.points.iter().map(|p| [p.lattice[0],p.level,p.lattice[1]]).collect::<Vec<_>>()})));
            patches.push((coordinate, surface));
        }
    }
    for (coordinate, surface) in patches {
        source
            .chunks
            .iter_mut()
            .find(|c| c.coordinate == coordinate)
            .expect("owned source block")
            .surface = Some(surface);
    }
    std::fs::create_dir(&output).expect("fresh output");
    std::fs::write(
        output.join("surface.ron"),
        ron::ser::to_string(&source).expect("surface serialize"),
    )
    .expect("surface write");
    std::fs::write(
        output.join("faces.json"),
        serde_json::to_vec(&exported).expect("faces serialize"),
    )
    .expect("faces write");
    std::fs::write(output.join("receipt.json"), serde_json::to_vec_pretty(&serde_json::json!({"kind":"ACTUAL_SOURCE_CONSTRAINED_CRYSTAL_FACE_EXPORT","package_fingerprint":expected,"source_fingerprint":manifest.source_fingerprint,"source_chunks":source.chunks.len(),"profiles":source.profiles.len(),"patches":receipts,"limits":["No renderer/edit/picking acceptance.","Outer legacy interface remains explicitly unresolved."]})).expect("receipt serialize")).expect("receipt write");
}

#[test]
fn sparse_halo_preserves_selected_caps_exact_faces_and_refuses_missing_facts() {
    let mut original = field(|q, r| {
        if q.rem_euclid(16) == 0
            || q.rem_euclid(16) == 15
            || r.rem_euclid(16) == 0
            || r.rem_euclid(16) == 15
        {
            (80, PATCH_BOUNDARY)
        } else {
            (80 + (q + r).rem_euclid(3), 0)
        }
    });
    for coordinate in [ChunkId { q: 0, r: 0 }, ChunkId { q: 1, r: 0 }] {
        let patch = build(&original, coordinate);
        original
            .chunks
            .iter_mut()
            .find(|c| c.coordinate == coordinate)
            .unwrap()
            .surface = Some(patch);
    }
    let compact = original.compact_halo().unwrap();
    assert_eq!(compact.chunks.len(), 2);
    assert!(compact.halo.len() < (original.chunks.len() - 2) * 256);
    assert_eq!(compact.compact_halo().unwrap(), compact);
    let old = builder(&original).unwrap();
    let new = builder(&compact).unwrap();
    for chunk in &compact.chunks {
        let cap = chunk.surface.as_ref().unwrap();
        assert_eq!(new.build_patch(chunk.coordinate).unwrap(), *cap);
        assert_eq!(
            old.build_faces(chunk.coordinate, cap, 100_000).unwrap(),
            new.build_faces(chunk.coordinate, cap, 100_000).unwrap()
        );
    }
    let mut missing = compact.clone();
    missing.halo.remove(0);
    let checked = builder(&missing).unwrap();
    assert!(missing.chunks.iter().any(|c| checked
        .build_faces(c.coordinate, c.surface.as_ref().unwrap(), 100_000)
        .is_err()));
    let mut bad = compact.clone();
    bad.halo.push(*bad.halo.first().unwrap());
    assert!(builder(&bad).is_err());
    let mut bad = compact.clone();
    bad.halo.first_mut().unwrap().column = WorldHex::new(0, 0);
    assert!(builder(&bad).is_err());
    let mut bad = compact.clone();
    bad.halo.first_mut().unwrap().profile = OUTSIDE_PROFILE;
    assert!(builder(&bad).is_err());
    let mut bad = compact.clone();
    bad.halo.first_mut().unwrap().protection = 128;
    assert!(builder(&bad).is_err());
    let before = compact.fingerprint().unwrap();
    let mut changed = compact;
    changed.halo.first_mut().unwrap().protection |= OBJECT_CONTACT;
    assert_ne!(before, changed.fingerprint().unwrap());
}

#[test]
fn absent_sparse_halo_preserves_original_v1_wire_and_fingerprint() {
    #[derive(Serialize)]
    struct Legacy<'a> {
        version: u32,
        tolerance: f64,
        profiles: &'a [SolidProfile],
        chunks: &'a [SurfaceChunk],
    }
    let original = field(|_, _| (80, PATCH_BOUNDARY));
    let legacy = Legacy {
        version: original.version,
        tolerance: original.tolerance,
        profiles: &original.profiles,
        chunks: &original.chunks,
    };
    let wire = ron::to_string(&legacy).unwrap();
    assert_eq!(wire, ron::to_string(&original).unwrap());
    assert_eq!(
        hex_world_contracts::hash_serializable(&legacy).unwrap(),
        original.fingerprint().unwrap()
    );
    assert_eq!(
        ron::from_str::<TerrainSurfaceOverview>(&wire).unwrap(),
        original
    );
}
