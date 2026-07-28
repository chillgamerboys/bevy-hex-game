//! Turning a spell's shape into the exact voxels it reaches.
//!
//! Every effect resolves to a **3D voxel volume**, because a world with bridges, cave
//! floors and sky islands has no flat answer to "what did the blast touch". This
//! module is that resolution and nothing else: pure functions from a shape, an anchor
//! and a facing to a `Vec<TilePos>`. It reads no components, sends no messages, and
//! knows nothing about legality, mana or whose turn it is.
//!
//! # The metric is grid space
//!
//! Horizontal hex distance and vertical level distance count **equally**, and the
//! combining rule is the maximum of the two
//! ([`grid_distance`](crate::volumes::grid_distance)). A radius-3
//! [`sphere`](crate::volumes::sphere) therefore reaches three hexes out *and* three
//! levels up or down, and looks slightly squashed on screen. That is the correct
//! trade: a world-space ball would need `level_height`, which is a renderer fact the
//! world owner owns and gameplay is forbidden to know. There is no float here and no
//! distance to compare
//! against a threshold — only integers, which is also what makes a volume
//! bit-identical on every machine that replays the same command log.
//!
//! [`targeting`](crate::targeting) deliberately refuses this same symmetric metric,
//! and the two are not in conflict. *Reach* is directional — height is an advantage
//! that buys range in one direction only — so it cannot be a metric at all. A
//! *volume* has no asker and no direction, so the symmetric rule is the right one
//! there and the wrong one here.
//!
//! # A volume is geometric, not obstructed
//!
//! A sphere next to a cave wall fills the wall's voxels and the chamber beyond it.
//! This is wrong, it is documented as wrong in
//! [status.md](https://github.com/chillgamerboys/bevy-hex-game/blob/main/docs/planning/status.md),
//! and it is bounded: obstruction-aware clipping arrives with the line-of-sight work,
//! and nothing here should grow a second raycast in the meantime.
//!
//! # Canonical output
//!
//! Every resolver hands back a **sorted, deduplicated** vector — the form
//! `TerrainImpact::volume` contractually requires, so an announcement can be built
//! from one of these without a fixup pass. Overlapping volumes are combined with
//! [`canonical`](crate::volumes::canonical), which restores the property.
//!
//! # What is not here
//!
//! The legality ladder, the announcement types, and anything that needs to know
//! whether a voxel is rock or air. Volumes are named before the world is consulted;
//! that is the whole point of the split.

use hex_assets::{TargetShape, VoxelOffset};
use hex_core::{HexCoord, Sextant, TilePos};

/// Grid-space distance between two voxels: the greater of the hex distance and the
/// level distance.
///
/// The maximum rather than the sum, because the two axes count equally rather than
/// accumulating — "two hexes out and two levels up" is distance two, not four. A
/// stack is never collapsed: two surfaces sharing a [`HexCoord`] are as far apart as
/// their levels say.
#[must_use]
pub fn grid_distance(a: TilePos, b: TilePos) -> u32 {
    a.coord
        .distance(b.coord)
        .max(a.level_step_to(b).unsigned_abs())
}

/// Sorts and deduplicates a set of voxels into the canonical form every announcement
/// requires.
///
/// This is also how two volumes are combined: chain them and canonicalise, and a
/// voxel a sphere and a column both cover appears once.
#[must_use]
pub fn canonical<I>(voxels: I) -> Vec<TilePos>
where
    I: IntoIterator<Item = TilePos>,
{
    let mut volume: Vec<TilePos> = voxels.into_iter().collect();
    volume.sort_unstable();
    volume.dedup();
    volume
}

/// The caster's own voxel.
#[must_use]
pub fn self_cast(caster: TilePos) -> Vec<TilePos> {
    vec![caster]
}

/// One target voxel.
#[must_use]
pub fn single(anchor: TilePos) -> Vec<TilePos> {
    vec![anchor]
}

/// Every voxel within `radius` in grid space of the anchor.
///
/// Because the metric is the maximum of the two axes, this is exactly the hexagonal
/// prism `radius` hexes wide and `2 * radius + 1` levels tall. A radius of `0` is the
/// anchor alone.
///
/// The caller bounds `radius`; a spell's comes from content, where `SpellFile`
/// validation caps it. Nothing here clamps, because a silently shrunken blast is
/// worse than a loud one.
#[must_use]
pub fn sphere(anchor: TilePos, radius: u32) -> Vec<TilePos> {
    let reach = i32::try_from(radius).unwrap_or(i32::MAX);
    canonical(
        anchor
            .coord
            .within_radius(radius)
            .into_iter()
            .flat_map(|coord| {
                (-reach..=reach)
                    .map(move |delta| TilePos::new(coord, anchor.level.saturating_add(delta)))
            }),
    )
}

/// The anchor voxel and the voxels stacked directly above it.
///
/// `height` counts voxels including the anchor, so a conjured wall two voxels tall —
/// the canonical walker's height — is `height: 2`. A `height` of `0` is an empty
/// volume rather than a silent single voxel: a wall with no voxels is what the
/// content asked for, and inventing one would make the count mean two things.
#[must_use]
pub fn column(anchor: TilePos, height: u32) -> Vec<TilePos> {
    canonical((0..height).map(|step| {
        let up = i32::try_from(step).unwrap_or(i32::MAX);
        TilePos::new(anchor.coord, anchor.level.saturating_add(up))
    }))
}

/// Out from the caster along `facing`, `length` hexes, at the caster's level.
///
/// **The caster's own voxel is never included** — a flamethrower does not burn the
/// hand holding it — so the steps run `1..=length` and a `length` of `0` is empty.
///
/// `width` is a **half-thickness in hexes**: every coordinate within `width` steps of
/// the line's spine, which makes a `width` of `0` the single file and keeps the shape
/// congruent under rotation for free, since hex distance is rotation-invariant. The
/// rounded ends this gives reach `length + width` hexes out; that is deliberate and
/// is the price of a thickening rule that cannot disagree with itself in six
/// directions.
///
/// The near end is where that rounding has to be overruled: at `width` 2 the first
/// spine hex's disc reaches back over the caster, so a designer who widened a line
/// would have quietly aimed it at themselves. The caster's voxel is excluded
/// explicitly. It is the origin of the rotation, so removing it costs no congruence.
///
/// The line is planar. Vertical extent belongs to [`column()`] and [`path`], which say
/// so in their parameters.
#[must_use]
pub fn line(caster: TilePos, facing: Sextant, length: u32, width: u32) -> Vec<TilePos> {
    let step = unit(facing);
    canonical(
        (1..=length)
            .flat_map(|out| translate(caster.coord, scale(step, out)).within_radius(width))
            .map(|coord| TilePos::new(coord, caster.level))
            .filter(|voxel| *voxel != caster),
    )
}

/// Widening out from the caster along `facing`, `length` hexes, at the caster's level.
///
/// `spread` counts the 60-degree sectors opened to **each** side of the facing, so
/// `0` is a bare ray identical to a `width`-0 [`line()`], `1` is the familiar
/// 120-degree cone (`2n + 1` hexes at range `n`), and `3` is a full disc. Values
/// above `3` say nothing more than `3` does and are treated as `3`; `SpellFile`
/// validation rejects them so no content can rely on the clamp.
///
/// Sectors are built from pairs of adjacent direction vectors rather than from an
/// angle, which is what keeps the shape exact — a 60-degree turn is a component
/// rotation on cube coordinates, with no rounding and no special case.
///
/// The cone is planar, for the same reason [`line()`] is.
#[must_use]
pub fn cone(caster: TilePos, facing: Sextant, length: u32, spread: u32) -> Vec<TilePos> {
    // Three sectors a side is the whole disc; a fourth would only re-cover it.
    let spread = spread.min(3);
    let mut voxels = Vec::new();
    for ring in 1..=length {
        // The facing ray itself, which is the shared edge of the first sector on
        // either side and so has to be emitted whether or not any sector is.
        voxels.push(TilePos::new(
            translate(caster.coord, scale(unit(facing), ring)),
            caster.level,
        ));
        for sector in 0..spread {
            // The sector clockwise of the facing, and its mirror. `turned` is
            // modular, so `6 - sector` is the anticlockwise turn of the same size.
            let arms = [
                (unit(facing.turned(sector)), unit(facing.turned(sector + 1))),
                (
                    unit(facing.turned(6 - sector)),
                    unit(facing.turned(5 - sector)),
                ),
            ];
            for (from, to) in arms {
                for along in 1..=ring {
                    let coord = translate(
                        caster.coord,
                        translate(scale(from, ring - along), scale(to, along)),
                    );
                    voxels.push(TilePos::new(coord, caster.level));
                }
            }
        }
    }
    canonical(voxels)
}

/// An authored voxel list, rotated into `facing` and hung on the anchor.
///
/// The escape hatch for a shape the parameterised vocabulary cannot say — an
/// L-shaped wall, a staircase, a bridge — and the only shape whose vertical extent an
/// author controls voxel by voxel.
///
/// **Anchored on the target, not the caster.** A wall is authored where it is built,
/// and the anchor is the one thing a cast always names.
///
/// An empty offset list is an empty volume. Offsets are written in the unrotated
/// frame, which is [`Sextant::A`]; see [`rotated`].
#[must_use]
pub fn path(anchor: TilePos, facing: Sextant, offsets: &[VoxelOffset]) -> Vec<TilePos> {
    canonical(offsets.iter().map(|offset| {
        let turned = rotated(*offset, facing);
        TilePos::new(
            translate(anchor.coord, turned.coord),
            anchor.level.saturating_add(turned.level),
        )
    }))
}

/// An authored offset turned from the unrotated frame into `facing`.
///
/// Rotation is 60 degrees per sextant, which is exact on cube coordinates, so an
/// authored pattern keeps its shape in all six directions. **The level is untouched**
/// — the rotation is about the vertical axis, so a staircase rotates into a staircase
/// rather than tipping over.
#[must_use]
pub fn rotated(offset: VoxelOffset, facing: Sextant) -> VoxelOffset {
    VoxelOffset {
        coord: rotate(offset.coord, facing),
        level: offset.level,
    }
}

/// Whether a shape has to be told which way it points.
///
/// [`TargetShape::Line`], [`TargetShape::Cone`] and [`TargetShape::Path`] do; the
/// rest are the same in every direction. The legality ladder uses this to know
/// whether a cast is missing its `facing` before it resolves anything.
#[must_use]
pub fn needs_facing(shape: &TargetShape) -> bool {
    matches!(
        shape,
        TargetShape::Line { .. } | TargetShape::Cone { .. } | TargetShape::Path { .. }
    )
}

/// The exact voxels a spell's shape reaches, or [`None`] when the shape needs a
/// facing and the cast did not name one.
///
/// The `None` is deliberate rather than an empty volume: a directional spell cast
/// with no direction is a malformed command, and returning nothing at all would make
/// it indistinguishable from a legal cast that happened to reach nothing. See
/// [`needs_facing`] for the same question asked ahead of time.
#[must_use]
pub fn resolve(
    shape: &TargetShape,
    caster: TilePos,
    anchor: TilePos,
    facing: Option<Sextant>,
) -> Option<Vec<TilePos>> {
    let volume = match shape {
        TargetShape::SelfCast => self_cast(caster),
        TargetShape::Single => single(anchor),
        TargetShape::Sphere { radius } => sphere(anchor, u32::from(*radius)),
        TargetShape::Column { height } => column(anchor, u32::from(*height)),
        TargetShape::Line { length, width } => {
            line(caster, facing?, u32::from(*length), u32::from(*width))
        }
        TargetShape::Cone { length, spread } => {
            cone(caster, facing?, u32::from(*length), u32::from(*spread))
        }
        TargetShape::Path { offsets } => path(anchor, facing?, offsets),
    };
    Some(volume)
}

/// The one-hex step in a direction.
fn unit(facing: Sextant) -> HexCoord {
    HexCoord::ORIGIN.neighbor(facing)
}

/// `coord` displaced by `offset`.
///
/// Saturating rather than wrapping: a coordinate near [`i32::MAX`] is already a grid
/// eighteen billion hexes across, and a shape that bends at the edge of one is a less
/// alarming failure than a shape that reappears on the far side.
fn translate(coord: HexCoord, offset: HexCoord) -> HexCoord {
    HexCoord::from_axial(
        coord.x().saturating_add(offset.x()),
        coord.y().saturating_add(offset.y()),
    )
}

/// `offset` multiplied by a non-negative `factor`.
fn scale(offset: HexCoord, factor: u32) -> HexCoord {
    let factor = i32::try_from(factor).unwrap_or(i32::MAX);
    HexCoord::from_axial(
        offset.x().saturating_mul(factor),
        offset.y().saturating_mul(factor),
    )
}

/// A cube-coordinate rotation of `facing`'s size, clockwise from the unrotated frame.
///
/// One sextant clockwise is `(x, y, z) -> (-y, -z, -x)`; the six closed forms are
/// written out rather than iterated, so the rotation is total by construction and
/// carries no panic path — the same reason
/// [`Sextant::turned`](hex_core::Sextant::turned) is a match rather than an index.
fn rotate(offset: HexCoord, facing: Sextant) -> HexCoord {
    let [x, y, z] = offset.to_cubic_array();
    let (x, y) = match facing {
        Sextant::A => (x, y),
        Sextant::B => (-y, -z),
        Sextant::C => (z, x),
        Sextant::D => (-x, -y),
        Sextant::E => (y, z),
        Sextant::F => (-z, -x),
    };
    HexCoord::from_axial(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rotation this module applies and the rotation [`Sextant`] names have to be
    /// the same one. If they drift, an authored path points somewhere its author did
    /// not mean and nothing else in the codebase would notice.
    #[test]
    fn rotating_the_unrotated_step_gives_that_direction() {
        for facing in Sextant::ALL {
            assert_eq!(
                rotate(unit(Sextant::A), facing),
                unit(facing),
                "rotating A's step by {facing:?} should give {facing:?}'s step"
            );
        }
    }

    /// Rotation must be a group action: turning every direction by the same facing
    /// permutes them the way `turned` says, for all six starting directions rather
    /// than just the one the previous test pins.
    #[test]
    fn rotation_agrees_with_turned_from_every_direction() {
        for (steps, facing) in Sextant::ALL.into_iter().enumerate() {
            let steps = u32::try_from(steps).expect("six fits in u32");
            for start in Sextant::ALL {
                assert_eq!(
                    rotate(unit(start), facing),
                    unit(start.turned(steps)),
                    "{start:?} rotated by {facing:?} should be {start:?} turned {steps}"
                );
            }
        }
    }

    /// A full turn is the identity, which is what makes "congruent in all six
    /// directions" a property rather than six hand-checked cases.
    #[test]
    fn six_rotations_return_to_the_start() {
        let offset = HexCoord::new_cubic(3, -5, 2);
        let mut turned = offset;
        for _ in 0..6 {
            turned = rotate(turned, Sextant::B);
        }
        assert_eq!(turned, offset);
    }

    /// Rotation is an isometry: it moves a coordinate without changing how far out it
    /// is. Everything congruence depends on follows from this.
    #[test]
    fn rotation_preserves_distance_from_the_origin() {
        for x in -6..=6 {
            for y in -6..=6 {
                let offset = HexCoord::from_axial(x, y);
                let far = HexCoord::ORIGIN.distance(offset);
                for facing in Sextant::ALL {
                    assert_eq!(HexCoord::ORIGIN.distance(rotate(offset, facing)), far);
                }
            }
        }
    }

    #[test]
    fn scaling_a_step_walks_that_far() {
        for facing in Sextant::ALL {
            for factor in 0..8 {
                assert_eq!(
                    HexCoord::ORIGIN.distance(scale(unit(facing), factor)),
                    factor
                );
            }
        }
    }
}
