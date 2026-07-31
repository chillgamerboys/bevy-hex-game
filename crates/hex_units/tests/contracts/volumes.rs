//! Headless contract tests for the shape vocabulary.
//!
//! These matter more than most, because every spell forever after trusts these
//! functions and **nothing visual will ever check them**. A blast that quietly misses
//! the ground under a bridge, or a cone that is a different shape pointing north-east
//! than it is pointing west, produces no log line and no wrong-looking frame — it
//! produces a fight that feels off for reasons nobody can name.
//!
//! Four properties carry the weight:
//!
//! - **Congruence.** A directional shape rotated through every [`Sextant`] is the
//!   same shape, not merely the same size.
//! - **Stacked surfaces.** The metric is grid space, so a sphere on a bridge deck
//!   reaches the ground beneath it exactly when the level distance says so.
//! - **Canonical output.** Every resolver hands back the sorted, deduplicated form
//!   `TerrainImpact` requires, checked with `TerrainImpact`'s own predicate rather
//!   than a re-implementation of it.
//! - **Degenerate inputs.** Radius 0, height 0, an empty path, a zero-length cone.

use hex_assets::{TargetShape, VoxelOffset};
use hex_core::{ElementId, HexCoord, Level, Sextant, TerrainBatchId, TerrainImpact, TilePos};
use hex_units::volumes::{
    canonical, column, cone, grid_distance, line, needs_facing, path, resolve, rotated, self_cast,
    single, sphere,
};

fn at(x: i32, y: i32, z: i32, level: Level) -> TilePos {
    TilePos::new(HexCoord::new_cubic(x, y, z), level)
}

/// Asserts a volume is in the exact form an announcement requires, using the
/// contract's own predicate. A local "is it sorted" check would be a second opinion,
/// and two opinions about a canonical form is how they drift apart.
fn assert_canonical(volume: &[TilePos], what: &str) {
    let impact = TerrainImpact {
        batch: TerrainBatchId(0),
        volume: volume.to_vec(),
        element: ElementId(0),
        power: 1,
    };
    assert!(impact.is_canonical(), "{what} is not canonical: {volume:?}");
}

/// Re-reads a volume as offsets from its origin, so two volumes at the same origin
/// can be compared after one of them is rotated.
fn offsets_from(origin: TilePos, volume: &[TilePos]) -> Vec<VoxelOffset> {
    volume
        .iter()
        .map(|voxel| VoxelOffset {
            coord: HexCoord::from_axial(
                voxel.coord.x() - origin.coord.x(),
                voxel.coord.y() - origin.coord.y(),
            ),
            level: voxel.level - origin.level,
        })
        .collect()
}

/// Turns offsets back into voxels around an origin, canonicalised so the result can
/// be compared to a resolver's output directly.
fn volume_from(origin: TilePos, offsets: &[VoxelOffset]) -> Vec<TilePos> {
    canonical(offsets.iter().map(|offset| {
        TilePos::new(
            HexCoord::from_axial(
                origin.coord.x() + offset.coord.x(),
                origin.coord.y() + offset.coord.y(),
            ),
            origin.level + offset.level,
        )
    }))
}

/// The congruence check the whole rotation story rests on: take the shape in the
/// unrotated frame, turn every one of its voxels by `facing`, and demand the result
/// is *exactly* the shape the resolver builds when pointed that way.
///
/// Comparing voxel counts alone would pass for a shape that rotates into a different
/// arrangement of the same size, which is precisely the bug worth catching.
fn assert_congruent_in_every_direction(
    what: &str,
    origin: TilePos,
    build: impl Fn(Sextant) -> Vec<TilePos>,
) {
    let unrotated = build(Sextant::A);
    assert_canonical(&unrotated, what);
    let offsets = offsets_from(origin, &unrotated);

    for facing in Sextant::ALL {
        let built = build(facing);
        assert_canonical(&built, what);
        let turned: Vec<VoxelOffset> = offsets
            .iter()
            .map(|offset| rotated(*offset, facing))
            .collect();
        assert_eq!(
            built,
            volume_from(origin, &turned),
            "{what} pointing {facing:?} is not the unrotated shape turned {facing:?}"
        );
        assert_eq!(
            built.len(),
            unrotated.len(),
            "{what} pointing {facing:?} has a different voxel count"
        );
    }
}

// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
// The metric
// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

/// Hexes and levels count equally, and the rule combining them is the maximum.
///
/// The sum would be the other plausible reading, and it is the wrong one: it makes a
/// radius-2 sphere fail to reach a voxel two hexes out *and* two levels up, which is
/// exactly the reach the contract promises.
#[test]
fn grid_distance_counts_hexes_and_levels_equally() {
    let anchor = at(0, 0, 0, 10);
    assert_eq!(grid_distance(anchor, anchor), 0);
    assert_eq!(grid_distance(anchor, at(2, -2, 0, 10)), 2, "two hexes out");
    assert_eq!(grid_distance(anchor, at(0, 0, 0, 12)), 2, "two levels up");
    assert_eq!(grid_distance(anchor, at(0, 0, 0, 8)), 2, "two levels down");
    assert_eq!(
        grid_distance(anchor, at(2, -2, 0, 12)),
        2,
        "two out and two up is still two, not four"
    );
}

/// A stack is not collapsed. Two surfaces at one coordinate are as far apart as their
/// levels say, which is the difference between a metric that understands a bridge and
/// one that thinks the deck and the ground are the same place.
#[test]
fn grid_distance_separates_stacked_surfaces() {
    let deck = at(0, 0, 0, 16);
    let ground = at(0, 0, 0, 1);
    assert_eq!(grid_distance(deck, ground), 15);
    assert_eq!(grid_distance(ground, deck), 15, "and it is symmetric");
}

// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
// Congruence across all six rotations
// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

#[test]
fn a_line_is_congruent_in_every_direction() {
    let caster = at(2, -5, 3, 7);
    for width in 0..3 {
        assert_congruent_in_every_direction("a line", caster, |facing| {
            line(caster, facing, 4, width)
        });
    }
}

#[test]
fn a_cone_is_congruent_in_every_direction() {
    let caster = at(-3, 1, 2, 4);
    for spread in 0..4 {
        assert_congruent_in_every_direction("a cone", caster, |facing| {
            cone(caster, facing, 3, spread)
        });
    }
}

/// An authored path with vertical extent — the case a rotation about the wrong axis
/// would tip over — and with an asymmetric footprint, so a rotation that is really a
/// reflection cannot pass.
#[test]
fn a_path_is_congruent_in_every_direction() {
    let anchor = at(1, 1, -2, 9);
    let staircase = [
        VoxelOffset {
            coord: HexCoord::new_cubic(1, 0, -1),
            level: 0,
        },
        VoxelOffset {
            coord: HexCoord::new_cubic(2, 0, -2),
            level: 1,
        },
        VoxelOffset {
            coord: HexCoord::new_cubic(2, 1, -3),
            level: 2,
        },
    ];
    assert_congruent_in_every_direction("a path", anchor, |facing| {
        path(anchor, facing, &staircase)
    });
}

/// Rotation is about the vertical axis, so an authored level survives it untouched.
/// A staircase that rotates into a ramp lying on its side would still be "congruent"
/// by voxel count.
#[test]
fn rotation_never_moves_a_voxel_vertically() {
    let offset = VoxelOffset {
        coord: HexCoord::new_cubic(2, -1, -1),
        level: -3,
    };
    for facing in Sextant::ALL {
        assert_eq!(rotated(offset, facing).level, -3, "{facing:?} tipped it");
    }
}

/// Six turns is the identity, so a shape rotated all the way round is itself.
#[test]
fn a_full_turn_restores_an_offset() {
    let offset = VoxelOffset {
        coord: HexCoord::new_cubic(3, -5, 2),
        level: 4,
    };
    let mut turned = offset;
    for _ in 0..6 {
        turned = rotated(turned, Sextant::B);
    }
    assert_eq!(turned, offset);
}

// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
// Stacked-surface correctness
// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

/// The case the naive implementation gets wrong. A blast on a bridge deck reaches
/// down the column exactly as far as its radius, and stops.
///
/// An implementation that resolved the shape over `HexCoord` and then attached the
/// anchor's level would put every one of these voxels on the deck; one that ignored
/// the vertical axis entirely would reach the ground at any depth. Both pass a test
/// that only looks sideways.
#[test]
fn a_sphere_on_a_bridge_reaches_exactly_as_far_down_as_its_radius() {
    let deck = at(0, 0, 0, 12);
    let volume = sphere(deck, 2);

    for depth in 0..=2 {
        let below = at(0, 0, 0, 12 - depth);
        assert!(
            volume.contains(&below),
            "level {} is {depth} down and should be inside",
            below.level
        );
    }
    assert!(
        !volume.contains(&at(0, 0, 0, 9)),
        "the ground three levels down is outside a radius-2 blast"
    );
    assert!(
        !volume.contains(&at(0, 0, 0, 15)),
        "and so is the sky three levels up"
    );
}

/// The corner of the metric: as far out sideways *and* as far up as the radius allows
/// is still inside, because the two axes do not accumulate.
#[test]
fn a_sphere_reaches_its_corners() {
    let anchor = at(0, 0, 0, 10);
    let volume = sphere(anchor, 2);
    assert!(volume.contains(&at(2, -2, 0, 12)), "two out and two up");
    assert!(volume.contains(&at(2, -2, 0, 8)), "two out and two down");
    assert!(!volume.contains(&at(3, -3, 0, 10)), "three out is outside");
}

/// Membership agrees with the metric everywhere, not only at the points a hand-picked
/// example happens to name.
#[test]
fn sphere_membership_is_exactly_the_metric() {
    let anchor = at(1, -1, 0, 6);
    let radius = 3;
    let volume = sphere(anchor, radius);
    for x in -6..=6 {
        for y in -6..=6 {
            for level in 0..=12 {
                let voxel = TilePos::new(HexCoord::from_axial(x, y), level);
                assert_eq!(
                    volume.contains(&voxel),
                    grid_distance(anchor, voxel) <= radius,
                    "{voxel:?} disagrees with the metric"
                );
            }
        }
    }
}

/// The prism the metric implies: a hexagon of `3r² + 3r + 1` coordinates, each
/// carrying `2r + 1` levels.
#[test]
fn a_sphere_is_a_prism_of_the_expected_size() {
    let anchor = at(0, 0, 0, 20);
    for radius in 0..=4usize {
        let hexes = 3 * radius * radius + 3 * radius + 1;
        let levels = 2 * radius + 1;
        let built = sphere(anchor, u32::try_from(radius).expect("four fits in u32"));
        assert_eq!(
            built.len(),
            hexes * levels,
            "wrong voxel count at radius {radius}"
        );
    }
}

/// A column climbs its own coordinate and touches nothing beside it — the shape a
/// conjured wall is, and the one that must not spread sideways.
#[test]
fn a_column_stacks_upward_from_the_anchor() {
    let anchor = at(4, -1, -3, 5);
    let wall = column(anchor, 2);
    assert_eq!(wall, vec![anchor, anchor.above()]);
    for neighbour in anchor.neighbours() {
        assert!(!wall.contains(&neighbour), "a wall is one coordinate wide");
    }
}

// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
// Canonical output
// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

/// Every resolver, on an anchor chosen so the shapes are not accidentally built in
/// sorted order.
#[test]
fn every_resolver_returns_a_canonical_volume() {
    let caster = at(-2, 3, -1, 7);
    let anchor = at(5, -2, -3, 11);
    let offsets = [
        VoxelOffset {
            coord: HexCoord::new_cubic(2, -2, 0),
            level: 1,
        },
        VoxelOffset {
            coord: HexCoord::new_cubic(-1, 0, 1),
            level: -2,
        },
        VoxelOffset {
            coord: HexCoord::ORIGIN,
            level: 0,
        },
    ];

    assert_canonical(&self_cast(caster), "a self-cast");
    assert_canonical(&single(anchor), "a single target");
    assert_canonical(&sphere(anchor, 3), "a sphere");
    assert_canonical(&column(anchor, 4), "a column");
    for facing in Sextant::ALL {
        assert_canonical(&line(caster, facing, 4, 1), "a line");
        assert_canonical(&cone(caster, facing, 3, 1), "a cone");
        assert_canonical(&path(anchor, facing, &offsets), "a path");
    }
}

/// The union case the contract calls out: a sphere and a column that overlap must
/// yield each shared voxel once.
#[test]
fn overlapping_volumes_combine_without_repeats() {
    let anchor = at(0, 0, 0, 10);
    let ball = sphere(anchor, 2);
    let stack = column(anchor, 5);
    let both = canonical(ball.iter().copied().chain(stack.iter().copied()));

    assert_canonical(&both, "a sphere combined with a column");
    for voxel in ball.iter().chain(stack.iter()) {
        assert!(both.contains(voxel), "{voxel:?} was lost in the union");
    }
    // The column's lower three voxels are inside the ball; only its top two are new.
    assert_eq!(both.len(), ball.len() + 2);
}

/// An authored path that names the same voxel twice — an easy thing to do in a hand
/// written offset list — resolves to one voxel, not two applications of the effect.
#[test]
fn a_path_that_repeats_itself_still_resolves_canonically() {
    let anchor = at(0, 0, 0, 4);
    let repeated = VoxelOffset {
        coord: HexCoord::new_cubic(1, -1, 0),
        level: 0,
    };
    let volume = path(anchor, Sextant::A, &[repeated, repeated, repeated]);
    assert_canonical(&volume, "a repeated path");
    assert_eq!(volume.len(), 1);
}

// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
// Degenerate inputs
// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

/// A radius-0 ball is the anchor. It is not empty, because "everything within zero of
/// here" includes here.
#[test]
fn a_zero_radius_sphere_is_the_anchor_alone() {
    let anchor = at(3, -3, 0, 6);
    assert_eq!(sphere(anchor, 0), vec![anchor]);
    assert_eq!(sphere(anchor, 0), single(anchor));
}

/// A zero-height column is empty rather than a silent single voxel, because `height`
/// counts voxels and a wall of no voxels is what was asked for. Content validation
/// refuses to author one; the resolver still has to be total.
#[test]
fn a_zero_height_column_is_empty() {
    assert!(column(at(0, 0, 0, 3), 0).is_empty());
}

/// A line and a cone both start one hex out, so a zero-length one is empty — and, in
/// particular, does not silently include the caster.
#[test]
fn zero_length_lines_and_cones_are_empty() {
    let caster = at(0, 0, 0, 5);
    for facing in Sextant::ALL {
        assert!(line(caster, facing, 0, 3).is_empty(), "a line of no length");
        assert!(cone(caster, facing, 0, 2).is_empty(), "a cone of no length");
    }
}

#[test]
fn an_empty_path_is_an_empty_volume() {
    assert!(path(at(0, 0, 0, 2), Sextant::D, &[]).is_empty());
}

/// The caster is never in its own line or cone. A flamethrower that burns the hand
/// holding it would be a memorable bug to ship.
#[test]
fn a_line_never_includes_the_caster() {
    let caster = at(0, 0, 0, 5);
    for facing in Sextant::ALL {
        assert!(!line(caster, facing, 5, 2).contains(&caster));
        assert!(!cone(caster, facing, 5, 1).contains(&caster));
    }
}

/// A `SelfCast` is the one shape that is the caster's own voxel, which is what makes
/// the exclusion above safe to state so bluntly.
#[test]
fn a_self_cast_is_exactly_the_caster() {
    let caster = at(7, -4, -3, 2);
    assert_eq!(self_cast(caster), vec![caster]);
}

// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
// Line and cone shape
// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

/// A single-file line is one voxel per step, all at the caster's level.
#[test]
fn a_thin_line_is_one_voxel_per_step() {
    let caster = at(0, 0, 0, 8);
    for facing in Sextant::ALL {
        let volume = line(caster, facing, 4, 0);
        assert_eq!(volume.len(), 4);
        for voxel in &volume {
            assert_eq!(voxel.level, 8, "a line is planar");
        }
        assert!(volume.contains(&TilePos::new(HexCoord::ORIGIN.neighbor(facing), 8)));
    }
}

/// A cone with no spread is a bare ray, which is the same set a single-file line is.
/// Two shapes that must agree at their shared degenerate case, so a change to one
/// that forgets the other shows up here.
#[test]
fn a_cone_with_no_spread_is_a_line() {
    let caster = at(1, -1, 0, 3);
    for facing in Sextant::ALL {
        assert_eq!(cone(caster, facing, 4, 0), line(caster, facing, 4, 0));
    }
}

/// The classic 120-degree cone: `2n + 1` hexes at range `n`, so lengths 1, 2 and 3
/// give 3, 8 and 15 voxels in total.
#[test]
fn a_cone_of_spread_one_widens_by_two_hexes_a_step() {
    let caster = at(0, 0, 0, 4);
    for facing in Sextant::ALL {
        for (length, expected) in [(1, 3), (2, 8), (3, 15)] {
            assert_eq!(
                cone(caster, facing, length, 1).len(),
                expected,
                "length {length} pointing {facing:?}"
            );
        }
    }
}

/// Three sectors a side is the whole disc, so the cone becomes the ring set around
/// the caster — every hex within `length`, minus the caster's own.
#[test]
fn a_cone_of_full_spread_is_a_disc() {
    let caster = at(0, 0, 0, 4);
    for facing in Sextant::ALL {
        let disc = cone(caster, facing, 3, 3);
        assert_eq!(disc.len(), 3 * 3 * 3 + 3 * 3 + 1 - 1);
        assert!(!disc.contains(&caster));
    }
}

/// Spread beyond a full disc says nothing more, and saying it does not change the
/// answer. Content validation rejects such a value, so this pins the resolver's
/// totality rather than a behaviour anyone should author.
#[test]
fn spread_beyond_a_full_disc_is_still_a_disc() {
    let caster = at(0, 0, 0, 4);
    assert_eq!(
        cone(caster, Sextant::C, 3, 9),
        cone(caster, Sextant::C, 3, 3)
    );
}

/// A thickened line is the set within `width` of its spine — minus the caster, whose
/// exclusion is the one carve-out. Checking membership against that rule rather than
/// against a count pins the definition rather than one of its consequences.
///
/// `width` 2 is the case that matters: the first spine hex is one out, so its disc
/// reaches back over the caster and the rule alone would aim the spell at whoever
/// cast it.
#[test]
fn a_thick_line_is_everything_within_width_of_its_spine() {
    let caster = at(0, 0, 0, 6);
    let facing = Sextant::B;
    let width = 2;
    let volume = line(caster, facing, 3, width);
    let spine: Vec<HexCoord> = (1..=3)
        .map(|step| {
            let unit = HexCoord::ORIGIN.neighbor(facing);
            HexCoord::from_axial(unit.x() * step, unit.y() * step)
        })
        .collect();

    for x in -6..=6 {
        for y in -6..=6 {
            let coord = HexCoord::from_axial(x, y);
            let voxel = TilePos::new(coord, 6);
            let near = spine.iter().any(|point| point.distance(coord) <= width);
            assert_eq!(
                volume.contains(&voxel),
                near && voxel != caster,
                "{coord:?} disagrees with the within-width rule"
            );
        }
    }
}

// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
// Resolving a content shape
// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

/// A directional shape with no facing is a malformed cast, and `resolve` says so
/// rather than handing back an empty volume — which would be indistinguishable from a
/// legal cast that reached nothing.
#[test]
fn a_directional_shape_without_a_facing_does_not_resolve() {
    let caster = at(0, 0, 0, 5);
    let anchor = at(2, -2, 0, 5);
    let directional = [
        TargetShape::Line {
            length: 3,
            width: 0,
        },
        TargetShape::Cone {
            length: 3,
            spread: 1,
        },
        TargetShape::Path {
            offsets: vec![VoxelOffset::default()],
        },
    ];
    for shape in &directional {
        assert!(needs_facing(shape), "{shape:?} should need a facing");
        assert!(
            resolve(shape, caster, anchor, None).is_none(),
            "{shape:?} resolved without a facing"
        );
        assert!(resolve(shape, caster, anchor, Some(Sextant::A)).is_some());
    }
}

/// The shapes that look the same in every direction resolve whether or not one was
/// given, and ignore it when it was.
#[test]
fn an_omnidirectional_shape_resolves_without_a_facing() {
    let caster = at(0, 0, 0, 5);
    let anchor = at(2, -2, 0, 9);
    let shapes = [
        TargetShape::SelfCast,
        TargetShape::Single,
        TargetShape::Sphere { radius: 2 },
        TargetShape::Column { height: 3 },
    ];
    for shape in &shapes {
        assert!(!needs_facing(shape), "{shape:?} should not need a facing");
        let without = resolve(shape, caster, anchor, None);
        assert!(without.is_some(), "{shape:?} needed a facing after all");
        assert_eq!(
            without,
            resolve(shape, caster, anchor, Some(Sextant::E)),
            "{shape:?} changed when handed a facing it does not use"
        );
    }
}

/// Every shape is planted on the origin its resolver expects.
///
/// `resolve` is the one place `caster` and `anchor` are dispatched, they are the same
/// type, and the six arms make three different choices between them — so a swap
/// compiles silently. Nothing else here would catch it: the `Line` and `Cone` tests
/// all call the resolvers directly, so `Line { .. } => line(anchor, ..)` would fire a
/// flamethrower from the clicked tile and leave the suite green. Pinning both origins
/// against a `caster` that is nowhere near the `anchor` is what makes the swap fail.
#[test]
fn every_shape_resolves_against_the_right_origin() {
    let caster = at(0, 0, 0, 5);
    let anchor = at(4, -4, 0, 9);
    let facing = Sextant::C;

    // Directed shapes fire from the caster; the anchor is not theirs to read.
    assert_eq!(
        resolve(
            &TargetShape::Line {
                length: 3,
                width: 1
            },
            caster,
            anchor,
            Some(facing)
        ),
        Some(line(caster, facing, 3, 1)),
    );
    assert_eq!(
        resolve(
            &TargetShape::Cone {
                length: 2,
                spread: 1
            },
            caster,
            anchor,
            Some(facing)
        ),
        Some(cone(caster, facing, 2, 1)),
    );

    // Anchored shapes land where the cast pointed, whatever the caster's own voxel.
    assert_eq!(
        resolve(&TargetShape::Sphere { radius: 2 }, caster, anchor, None),
        Some(sphere(anchor, 2)),
    );
    assert_eq!(
        resolve(&TargetShape::Column { height: 3 }, caster, anchor, None),
        Some(column(anchor, 3)),
    );
    let offsets = vec![VoxelOffset {
        coord: HexCoord::from_axial(1, 0),
        level: 1,
    }];
    assert_eq!(
        resolve(
            &TargetShape::Path {
                offsets: offsets.clone()
            },
            caster,
            anchor,
            Some(facing)
        ),
        Some(path(anchor, facing, &offsets)),
    );
}

/// `SelfCast` is the one shape resolved against the caster rather than the anchor —
/// the distinction that makes passing both worth it.
#[test]
fn a_self_cast_ignores_the_anchor() {
    let caster = at(0, 0, 0, 5);
    let anchor = at(4, -4, 0, 9);
    assert_eq!(
        resolve(&TargetShape::SelfCast, caster, anchor, None),
        Some(vec![caster])
    );
    assert_eq!(
        resolve(&TargetShape::Single, caster, anchor, None),
        Some(vec![anchor])
    );
}

/// Resolving the same cast twice gives the identical vector, byte for byte. These
/// volumes may feed a future replay log, so "the same shape in a different order" is
/// a defect.
#[test]
fn resolving_is_deterministic() {
    let caster = at(-1, 4, -3, 6);
    let anchor = at(3, -1, -2, 9);
    let shapes = [
        TargetShape::Sphere { radius: 3 },
        TargetShape::Column { height: 4 },
        TargetShape::Line {
            length: 4,
            width: 2,
        },
        TargetShape::Cone {
            length: 3,
            spread: 2,
        },
    ];
    for shape in &shapes {
        let first = resolve(shape, caster, anchor, Some(Sextant::F));
        let second = resolve(shape, caster, anchor, Some(Sextant::F));
        assert_eq!(first, second, "{shape:?} is not deterministic");
    }
}
