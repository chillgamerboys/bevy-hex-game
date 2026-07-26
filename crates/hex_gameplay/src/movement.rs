//! Which surfaces a piece may step between, and how it gets from one to another.
//!
//! # The rules
//!
//! > A body may **stand** on a surface when its substance is solid and its
//! > [`Headroom`](hex_core::Headroom) is at least the body's [`Body::levels_tall`].
//!
//! > A **step** is legal when the destination is an adjacent surface a body can stand
//! > on, and its surface is within [`MAX_STEP`] levels.
//!
//! Surfaces stacked in one column are never adjacent, so a piece on a bridge
//! cannot drop to the ground beneath it. Getting down means a ramp of adjacent
//! surfaces descending a level at a time — or an ability that explicitly ignores this,
//! which is not implemented and belongs here when it is.
//!
//! Headroom is what makes size matter: a two-level body cannot squeeze into the
//! one-voxel crawlspace under a bridge that a one-level body walks straight through.
//!
//! # What this is not
//!
//! [`route`] is a straight line, not a pathfinder. It gives up rather than going
//! around an obstacle, which is correct behaviour for "no route exists" but useless
//! for anything that needs to actually navigate. `hexx::a_star` is compiled in and is
//! the obvious replacement; it wants a movement-cost model first, which is design
//! work rather than plumbing.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use hex_assets::SubstanceTable;
use hex_core::{Headroom, HexCoord, HexSpan, Level, SubstanceId, TilePos};

/// How many levels a piece may climb or drop in one step.
///
/// One, by design: a step is a step. Anything steeper is a cliff and has to be walked
/// around, or bypassed with an ability.
pub const MAX_STEP: Level = 1;

/// How much room a thing takes up, and therefore where it fits.
///
/// A struct rather than a bare [`Level`] on purpose. Bodies come in configurations —
/// the obvious next one is a **footprint**, a set of coordinate offsets for something
/// wider than a single hex — and adding that field here changes [`Body::admits`] and
/// nothing else. No call site passes the parts separately, so none of them move.
///
/// A footprint is deliberately not built yet. It is pure gameplay, invisible to the
/// map, and it first needs a decision this codebase has not taken: whether a wide body
/// may straddle a one-level step, or must have every hex of its footprint level.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Eq)]
#[reflect(Component)]
pub struct Body {
    /// How many levels tall, and so how much clear space it needs overhead.
    pub levels_tall: Level,
}

impl Body {
    /// Whether this body fits in the space above a surface.
    ///
    /// The single place the size rule lives, and where a footprint check would join
    /// it. Everything else asks this rather than comparing levels itself.
    #[must_use]
    pub const fn admits(self, headroom: Headroom) -> bool {
        headroom.0 >= self.levels_tall
    }
}

/// A surface a piece can stand on: where it is and its rendered span.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Standing {
    /// Which voxel is being stood on.
    pub pos: TilePos,
    /// The rendered run's extent, for placing the piece in the world.
    pub span: HexSpan,
}

impl Standing {
    /// The world-space point a piece standing here occupies.
    #[must_use]
    pub fn world_position(self) -> Vec3 {
        self.pos.coord.to_world(self.span.top)
    }
}

/// Every surface **a particular body** could stand on, indexed by position.
///
/// Body-specific by construction rather than universal, because standability depends
/// on size: a crawlspace under a bridge is footing for a small creature and a wall for
/// a large one. Filtering once here is what lets [`Footing::at`], [`Footing::ground`],
/// [`Footing::step_from`] and [`route`] stay free of size arguments entirely.
///
/// Built from the tile entities rather than from a map resource, which is what keeps
/// gameplay independent of how terrain is stored. `hex_map` can be rewritten
/// wholesale as long as tiles carry a [`TilePos`], a [`HexSpan`], a [`SubstanceId`]
/// and a [`Headroom`].
#[derive(Debug, Default)]
pub struct Footing {
    by_pos: HashMap<TilePos, Standing>,
    /// Surfaces at each coordinate, so one can be found without knowing which
    /// level its top happens to be at.
    surfaces: HashMap<HexCoord, Vec<Standing>>,
}

impl Footing {
    /// Collects the surfaces `body` can stand on from the tile entities.
    ///
    /// Two independent conditions, from two different places:
    ///
    /// - **Solid**, read from the substance table. Air is never spawned as a prism,
    ///   but a future non-solid substance such as water would be, and stepping onto it
    ///   should not silently work.
    /// - **Room enough**, from the [`Headroom`] the map reports. Zero headroom means
    ///   the tile is buried inside a column and is not a surface at all; too little
    ///   means the body does not fit.
    ///
    /// Both are checked here rather than in the caller's query, so there is exactly
    /// one place the rule lives. Getting this wrong is not subtle in its effects and
    /// is very subtle in its symptoms: treating buried runs as standable put the
    /// player inside the terrain and left every route walking through the bedrock,
    /// arriving nowhere.
    pub fn from_tiles<'a>(
        tiles: impl Iterator<Item = (&'a TilePos, &'a HexSpan, &'a SubstanceId, &'a Headroom)>,
        table: &SubstanceTable,
        body: Body,
    ) -> Self {
        let mut footing = Self::default();

        for (pos, span, substance, headroom) in tiles {
            if !table.is_solid(*substance) || !body.admits(*headroom) {
                continue;
            }
            // The tile's `TilePos` is already its topmost solid voxel, so the
            // standable position is exactly that — gameplay never has to know how
            // tall a level is, which is what keeps `level_height` inside the map.
            let standing = Standing {
                pos: *pos,
                span: *span,
            };
            footing.by_pos.insert(standing.pos, standing);
            footing
                .surfaces
                .entry(pos.coord)
                .or_default()
                .push(standing);
        }

        footing
    }

    /// The standable surface at `pos`, if one exists.
    #[must_use]
    pub fn at(&self, pos: TilePos) -> Option<Standing> {
        self.by_pos.get(&pos).copied()
    }

    /// Every standable surface at a coordinate, in no particular order.
    #[must_use]
    pub fn at_coord(&self, coord: HexCoord) -> &[Standing] {
        self.surfaces.get(&coord).map_or(&[], Vec::as_slice)
    }

    /// The lowest standable surface at a coordinate — the ground, rather than any
    /// bridge built over it.
    #[must_use]
    pub fn ground(&self, coord: HexCoord) -> Option<Standing> {
        self.at_coord(coord)
            .iter()
            .min_by_key(|standing| standing.pos.level)
            .copied()
    }

    /// The surface reachable in one step from `from` at `coord`, if any.
    ///
    /// Where several surfaces at that coordinate are within reach — a low bridge over
    /// a shallow ditch — the closest in height wins, because that is the one a piece
    /// walking in a straight line would naturally step onto.
    #[must_use]
    pub fn step_from(&self, from: Standing, coord: HexCoord) -> Option<Standing> {
        self.at_coord(coord)
            .iter()
            .filter(|candidate| from.pos.is_within_step_of(candidate.pos, MAX_STEP))
            .min_by_key(|candidate| from.pos.level_step_to(candidate.pos).abs())
            .copied()
    }
}

/// The surfaces a piece passes over walking from `from` to `to` in a straight line.
///
/// Returns [`None`] when the line is blocked — a cliff, a gap, or a coordinate with
/// nothing standable on it. **Terrain is not guaranteed connected**, so "no route
/// exists" is a real answer and callers have to handle it.
///
/// Straight-line only: this gives up where a pathfinder would go around.
#[must_use]
pub fn route(from: Standing, to: Standing, footing: &Footing) -> Option<Vec<Standing>> {
    if from.pos == to.pos {
        return Some(vec![from]);
    }

    let mut steps = vec![from];
    let mut current = from;

    for coord in from
        .pos
        .coord
        .line_between(to.pos.coord)
        .into_iter()
        .skip(1)
    {
        let next = footing.step_from(current, coord)?;
        steps.push(next);
        current = next;
    }

    // The line has to actually arrive. Landing on a different surface at the right
    // coordinate — the ground under the target bridge, say — is not arriving.
    (current.pos == to.pos).then_some(steps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_assets::{Substance, SubstanceFile};
    use hex_core::MAX_HEADROOM;

    const STONE: SubstanceId = SubstanceId(1);

    fn table() -> SubstanceTable {
        let mut substances = HashMap::default();
        substances.insert(
            "air".to_owned(),
            Substance {
                color: (0.0, 0.0, 0.0),
                solid: false,
                diggable: false,
            },
        );
        substances.insert(
            "stone".to_owned(),
            Substance {
                color: (0.5, 0.5, 0.5),
                solid: true,
                diggable: true,
            },
        );
        SubstanceTable::from_file(&SubstanceFile { substances })
    }

    /// A body of the size the game actually ships, for tests about stepping rather
    /// than about size.
    const NORMAL: Body = Body { levels_tall: 2 };

    /// A one-level column under open sky.
    ///
    /// The span uses a level height of 1 so world coordinates and levels line up,
    /// which keeps the tests readable. `hex_map` uses whatever `level_height` says;
    /// gameplay never sees it.
    fn tile(coord: HexCoord, level: Level) -> (TilePos, HexSpan, SubstanceId, Headroom) {
        roofed(coord, level, MAX_HEADROOM)
    }

    /// A one-level column with a specific amount of space above it — a ledge under an
    /// overhang, or a run buried in a column when `headroom` is zero.
    fn roofed(
        coord: HexCoord,
        level: Level,
        headroom: Level,
    ) -> (TilePos, HexSpan, SubstanceId, Headroom) {
        #[expect(clippy::cast_precision_loss, reason = "test levels are single digits")]
        let span = HexSpan::new(level as f32, (level + 1) as f32);
        (TilePos::new(coord, level), span, STONE, Headroom(headroom))
    }

    fn footing_from(tiles: &[(TilePos, HexSpan, SubstanceId, Headroom)]) -> Footing {
        footing_for(tiles, NORMAL)
    }

    fn footing_for(tiles: &[(TilePos, HexSpan, SubstanceId, Headroom)], body: Body) -> Footing {
        Footing::from_tiles(
            tiles
                .iter()
                .map(|(pos, span, substance, headroom)| (pos, span, substance, headroom)),
            &table(),
            body,
        )
    }

    #[test]
    fn a_flat_line_is_walkable() {
        let line: Vec<HexCoord> = HexCoord::ORIGIN.line_between(HexCoord::new_cubic(3, -3, 0));
        let tiles: Vec<_> = line.iter().map(|c| tile(*c, 4)).collect();
        let footing = footing_from(&tiles);

        let Some(from) = footing.ground(HexCoord::ORIGIN) else {
            unreachable!("the origin has ground")
        };
        let Some(to) = footing.ground(HexCoord::new_cubic(3, -3, 0)) else {
            unreachable!("the destination has ground")
        };

        let Some(steps) = route(from, to, &footing) else {
            panic!("flat ground should be walkable")
        };
        assert_eq!(steps.len(), line.len());
    }

    /// A ramp descending one level per hex is walkable however far it descends —
    /// this is the spiral staircase case.
    #[test]
    fn a_one_level_ramp_is_walkable() {
        let line: Vec<HexCoord> = HexCoord::ORIGIN.line_between(HexCoord::new_cubic(4, -4, 0));
        let tiles: Vec<_> = line
            .iter()
            .enumerate()
            .map(|(i, c)| tile(*c, 6 - Level::try_from(i).unwrap_or(0)))
            .collect();
        let footing = footing_from(&tiles);

        let Some(from) = footing.ground(HexCoord::ORIGIN) else {
            unreachable!("the origin has ground")
        };
        let Some(to) = footing.ground(HexCoord::new_cubic(4, -4, 0)) else {
            unreachable!("the destination has ground")
        };

        assert!(
            route(from, to, &footing).is_some(),
            "a ramp descending one level at a time should be walkable"
        );
    }

    /// A two-level step is a cliff. This is the whole point of `MAX_STEP`.
    #[test]
    fn a_cliff_blocks_the_route() {
        let a = HexCoord::ORIGIN;
        let [b, ..] = a.neighbors();
        let footing = footing_from(&[tile(a, 2), tile(b, 5)]);

        let Some(from) = footing.ground(a) else {
            unreachable!("a has ground")
        };
        let Some(to) = footing.ground(b) else {
            unreachable!("b has ground")
        };

        assert!(
            route(from, to, &footing).is_none(),
            "a three-level climb should be refused"
        );
    }

    /// Terrain is not guaranteed connected, so a missing column is a legitimate
    /// "no route exists" rather than something to route around.
    #[test]
    fn a_gap_blocks_the_route() {
        let line: Vec<HexCoord> = HexCoord::ORIGIN.line_between(HexCoord::new_cubic(3, -3, 0));
        // Everything except the middle of the line.
        let tiles: Vec<_> = line
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 2)
            .map(|(_, c)| tile(*c, 4))
            .collect();
        let footing = footing_from(&tiles);

        let Some(from) = footing.ground(HexCoord::ORIGIN) else {
            unreachable!("the origin has ground")
        };
        let Some(to) = footing.ground(HexCoord::new_cubic(3, -3, 0)) else {
            unreachable!("the destination has ground")
        };

        assert!(route(from, to, &footing).is_none(), "a gap has no route");
    }

    /// The rule that started all of this: a piece on a bridge cannot drop to the
    /// ground beneath it, however close they look on a map.
    #[test]
    fn a_bridge_does_not_connect_to_the_ground_below() {
        let coord = HexCoord::ORIGIN;
        let footing = footing_from(&[tile(coord, 1), tile(coord, 8)]);

        let Some(ground) = footing.at(TilePos::new(coord, 1)) else {
            unreachable!("the ground exists")
        };
        let Some(bridge) = footing.at(TilePos::new(coord, 8)) else {
            unreachable!("the bridge exists")
        };

        assert!(
            route(bridge, ground, &footing).is_none(),
            "stepping off a bridge onto the ground below is not a step"
        );
        assert!(
            route(ground, bridge, &footing).is_none(),
            "nor is climbing straight up it"
        );
    }

    /// Walking under a bridge must not snap the piece up onto it.
    #[test]
    fn walking_under_a_bridge_stays_on_the_ground() {
        let line: Vec<HexCoord> = HexCoord::ORIGIN.line_between(HexCoord::new_cubic(3, -3, 0));
        let mut tiles: Vec<_> = line.iter().map(|c| tile(*c, 1)).collect();
        // A bridge crossing the middle of the route, high above it.
        if let Some(middle) = line.get(2) {
            tiles.push(tile(*middle, 9));
        }
        let footing = footing_from(&tiles);

        let Some(from) = footing.ground(HexCoord::ORIGIN) else {
            unreachable!("the origin has ground")
        };
        let Some(to) = footing.ground(HexCoord::new_cubic(3, -3, 0)) else {
            unreachable!("the destination has ground")
        };

        let Some(steps) = route(from, to, &footing) else {
            panic!("the ground route should be walkable")
        };
        for step in steps {
            assert_eq!(
                step.pos.level, 1,
                "the route climbed onto the bridge instead of passing under it"
            );
        }
    }

    #[test]
    fn a_route_to_where_you_already_are_is_one_step() {
        let footing = footing_from(&[tile(HexCoord::ORIGIN, 3)]);
        let Some(here) = footing.ground(HexCoord::ORIGIN) else {
            unreachable!("the origin has ground")
        };

        let Some(steps) = route(here, here, &footing) else {
            panic!("standing still is always possible")
        };
        assert_eq!(steps.len(), 1);
    }

    /// Non-solid substances are not standable, so a future water or lava tile does
    /// not silently become walkable.
    #[test]
    fn non_solid_substances_are_not_standable() {
        let coord = HexCoord::ORIGIN;
        let span = HexSpan::new(0.0, 1.0);
        let footing = Footing::from_tiles(
            [(
                &TilePos::new(coord, 0),
                &span,
                &SubstanceId::AIR,
                &Headroom(MAX_HEADROOM),
            )]
            .into_iter(),
            &table(),
            NORMAL,
        );
        assert!(footing.ground(coord).is_none());
    }

    /// A run buried inside a column is not a surface, however solid it is.
    ///
    /// This is the shipped bug, in one assertion. Run-merging splits a column into
    /// stacked runs, and treating the buried ones as standable made the bedrock at
    /// the bottom look exactly as good as the grass on top — so the piece stood
    /// inside the terrain and routes walked through the rock.
    #[test]
    fn buried_runs_are_never_standable() {
        let coord = HexCoord::ORIGIN;
        let footing = footing_from(&[roofed(coord, 0, 0)]);
        assert!(
            footing.ground(coord).is_none(),
            "a run with no space above it is inside a column, not on top of one"
        );
    }

    /// The case headroom exists for: the same ledge is footing for a short body and a
    /// wall for a tall one.
    #[test]
    fn a_tall_body_does_not_fit_where_a_short_one_does() {
        let coord = HexCoord::ORIGIN;
        // One clear voxel — a crawlspace under a bridge.
        let tiles = [roofed(coord, 1, 1)];

        assert!(
            footing_for(&tiles, Body { levels_tall: 1 })
                .ground(coord)
                .is_some(),
            "one level of clearance is enough for a one-level body"
        );
        assert!(
            footing_for(&tiles, Body { levels_tall: 2 })
                .ground(coord)
                .is_none(),
            "a two-level body does not fit under a one-voxel ceiling"
        );
    }

    /// A ceiling partway along an otherwise flat route blocks a tall body and lets a
    /// short one through. Terrain that is walkable is a property of the walker.
    #[test]
    fn a_low_tunnel_blocks_only_the_bodies_that_do_not_fit() {
        let line: Vec<HexCoord> = HexCoord::ORIGIN.line_between(HexCoord::new_cubic(3, -3, 0));
        let tiles: Vec<_> = line
            .iter()
            .enumerate()
            // A low roof over the middle of the line, open sky either side.
            .map(|(i, c)| {
                if i == 2 {
                    roofed(*c, 4, 1)
                } else {
                    tile(*c, 4)
                }
            })
            .collect();

        let short = Body { levels_tall: 1 };
        let short_footing = footing_for(&tiles, short);
        let (Some(from), Some(to)) = (
            short_footing.ground(HexCoord::ORIGIN),
            short_footing.ground(HexCoord::new_cubic(3, -3, 0)),
        ) else {
            unreachable!("both ends have ground")
        };
        assert!(
            route(from, to, &short_footing).is_some(),
            "a one-level body fits through the tunnel"
        );

        let tall_footing = footing_for(&tiles, Body { levels_tall: 2 });
        let (Some(from), Some(to)) = (
            tall_footing.ground(HexCoord::ORIGIN),
            tall_footing.ground(HexCoord::new_cubic(3, -3, 0)),
        ) else {
            unreachable!("both ends are still open sky for a tall body")
        };
        assert!(
            route(from, to, &tall_footing).is_none(),
            "a two-level body cannot pass under the low roof"
        );
    }
}
