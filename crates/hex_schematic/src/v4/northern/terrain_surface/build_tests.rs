use super::*;
use std::collections::BTreeMap;

fn field(sample: impl Fn(i16, i16) -> (i16, u8)) -> TerrainSurfaceOverview {
    let mut overview = TerrainSurfaceOverview {
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
