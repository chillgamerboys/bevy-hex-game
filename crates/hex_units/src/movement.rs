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
//! # Finding a way
//!
//! [`Reach`](crate::movement::Reach) floods outward from a surface and records how it
//! got to each one, so a single search answers both "where can this piece go" and "how
//! does it get to that particular tile". [`route`] is the one-destination convenience
//! on top.
//!
//! **`hexx::a_star` cannot be used here, despite being compiled in.** Its signature is
//! keyed on `Hex` alone, so it has nowhere to put a level: a bridge and the ground
//! beneath it are the same node to it, and pathing through one would let a piece
//! teleport to the other. `field_of_movement` has the same problem — it returns a
//! `HashSet<Hex>`. The search has to run over [`TilePos`](hex_core::TilePos), which is
//! why it is written out here rather than delegated. Earlier versions of this file, and
//! of the project documentation, recommended switching to `hexx`; that advice predates
//! the voxel map and following it would silently collapse every stack.
//!
//! Steps all cost one. Breadth-first order is therefore shortest-first, and no
//! priority queue is needed — the day terrain costs differ to cross, that stops being
//! true and [`Reach::from`](crate::movement::Reach::from) is where it changes.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use hex_assets::SubstanceTable;
use hex_core::{Headroom, HexCoord, HexSpan, Level, SubstanceId, TilePos};

/// Registers the movement types.
///
/// This module has no systems — it is rules, not behaviour. The plugin exists so
/// [`Body`] is registered beside where it is defined. It was previously registered by
/// the player's plugin, which meant a type declared in one file was announced by
/// another; moving either would have silently dropped it from the inspector.
pub fn plugin(app: &mut App) {
    app.register_type::<Body>();
}

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
    ///   but **water is** — the showcase map's river publishes ordinary tile entities
    ///   whose substance happens not to be solid. A tile's [`TilePos`] marks its
    ///   topmost *material* voxel, which is not the same as its topmost standable one,
    ///   so this check is the only thing between a piece and walking onto the river.
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
            // This run passed the solid-substance check, and its `TilePos` is already
            // its topmost material voxel, so the standable position is exactly that.
            // Gameplay never has to know how tall a level is, which keeps
            // `level_height` inside the map.
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

    /// Every standable surface at a coordinate, lowest first.
    ///
    /// The ordering is the surfaces' spawn order, which the map produces bottom-up.
    /// Nothing should *depend* on it — [`Self::steps_from`] sorts explicitly — but it
    /// is stated because it used to be the accidental tie-break for which surface a
    /// step landed on.
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

    /// Every surface reachable in one step from `from` at `coord`.
    ///
    /// **All** of them, not the nearest — a coordinate carrying both a floor and a
    /// bridge over it may offer two, and a search that only ever saw one would decide
    /// reachability by which happened to be listed first.
    ///
    /// Ordered closest in height, then lowest. The second key is what makes it
    /// deterministic: from level 4, a floor at 3 and a bridge at 5 are both one step
    /// away, and without a tiebreak the winner was whichever the map spawned first.
    #[must_use]
    pub fn steps_from(&self, from: Standing, coord: HexCoord) -> Vec<Standing> {
        let mut candidates: Vec<Standing> = self
            .at_coord(coord)
            .iter()
            .filter(|candidate| from.pos.is_within_step_of(candidate.pos, MAX_STEP))
            .copied()
            .collect();
        candidates.sort_by_key(|c| (from.pos.level_step_to(c.pos).abs(), c.pos.level));
        candidates
    }

    /// The single surface a piece would step onto at `coord`: the closest in height.
    #[must_use]
    pub fn step_from(&self, from: Standing, coord: HexCoord) -> Option<Standing> {
        self.steps_from(from, coord).first().copied()
    }
}

/// Where a search reached, and how it got there.
struct Step {
    standing: Standing,
    /// Steps taken from the start. Zero for the start itself.
    cost: u32,
    /// The surface stepped from, or [`None`] at the start.
    came_from: Option<TilePos>,
}

/// Everywhere a piece can walk from one surface, and the way to each.
///
/// A single breadth-first flood fill answers both questions the interface asks:
/// **which tiles can I reach** is the set of keys, and **how do I get to that one**
/// is a walk backwards down `came_from`. Computing a path per hovered tile would
/// redo the same search once per mouse move for no benefit.
///
/// Costs are uniform — one per step — so breadth-first order *is* shortest-first and
/// no priority queue is needed. That changes the day terrain costs differ to cross,
/// and this is where it changes.
#[derive(Debug, Default)]
pub struct Reach {
    // Keyed on `TilePos` rather than `Standing`, which holds an `f32` span and so
    // derives neither `Eq` nor `Hash`.
    steps: HashMap<TilePos, Step>,
}

impl std::fmt::Debug for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Step")
            .field("pos", &self.standing.pos)
            .field("cost", &self.cost)
            .finish()
    }
}

impl Reach {
    /// Floods outward from `start`, stopping after `budget` steps if there is one.
    ///
    /// `None` means unlimited, which is what exploring uses — there is no turn and so
    /// no movement budget. The whole standable graph is around 1,300 surfaces with at
    /// most six edges each, so even the unbounded search is trivial.
    #[must_use]
    pub fn from(start: Standing, footing: &Footing, budget: Option<u32>) -> Self {
        let mut reach = Self::default();
        reach.steps.insert(
            start.pos,
            Step {
                standing: start,
                cost: 0,
                came_from: None,
            },
        );

        let mut frontier = std::collections::VecDeque::from([start]);
        while let Some(current) = frontier.pop_front() {
            let Some(cost) = reach.cost(current.pos) else {
                continue;
            };
            if budget.is_some_and(|limit| cost >= limit) {
                continue;
            }

            // Neighbour order comes from `hexx`, a fixed constant array, so it is the
            // same every run. It is also what decides between two paths of equal
            // length — the first one found wins and is never replaced.
            for coord in current.pos.coord.neighbors() {
                for next in footing.steps_from(current, coord) {
                    if reach.steps.contains_key(&next.pos) {
                        continue;
                    }
                    reach.steps.insert(
                        next.pos,
                        Step {
                            standing: next,
                            cost: cost + 1,
                            came_from: Some(current.pos),
                        },
                    );
                    frontier.push_back(next);
                }
            }
        }

        reach
    }

    /// How many steps away a surface is, or [`None`] if it cannot be reached.
    #[must_use]
    pub fn cost(&self, pos: TilePos) -> Option<u32> {
        self.steps.get(&pos).map(|step| step.cost)
    }

    /// The surfaces walked over to get there, starting with the surface stood on now.
    ///
    /// [`None`] when the destination is out of reach — terrain is not guaranteed
    /// connected, and a budget makes far-away places unreachable this turn.
    #[must_use]
    pub fn path_to(&self, pos: TilePos) -> Option<Vec<Standing>> {
        let mut path = Vec::new();
        let mut cursor = Some(pos);
        while let Some(at) = cursor {
            let step = self.steps.get(&at)?;
            path.push(step.standing);
            cursor = step.came_from;
        }
        path.reverse();
        Some(path)
    }

    /// Every surface that can be reached, including the one started from.
    pub fn surfaces(&self) -> impl Iterator<Item = Standing> + '_ {
        self.steps.values().map(|step| step.standing)
    }
}

/// The shortest walk from `from` to `to`, going around whatever is in the way.
///
/// Returns [`None`] when there is genuinely no way there — a moat, an island, a ledge
/// too high on every approach. **Terrain is not guaranteed connected**, so "no route
/// exists" is a real answer and callers have to handle it.
///
/// This walked a straight line until recently and gave up at the first obstacle, which
/// made ordinary terraced ground look broken: a click two hexes away would silently do
/// nothing because the line clipped a corner. Anything relying on the old behaviour —
/// a test asserting a route fails — needs re-reading, because most of those failures
/// were the router, not the map.
///
/// For a single query this builds a whole flood fill and then reads one path out of
/// it. That is deliberate: the caller that runs every frame wants the flood fill
/// anyway, for the movement range, so [`Reach`] is the real interface and this is the
/// convenience on top.
#[must_use]
pub fn route(from: Standing, to: Standing, footing: &Footing) -> Option<Vec<Standing>> {
    // Standing still is a valid answer, and a one-element path is what callers expect
    // for it — `HexPathingLine` turns that into an animation that finishes at once.
    if from.pos == to.pos {
        return Some(vec![from]);
    }
    Reach::from(from, footing, None).path_to(to.pos)
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

    /// A budget is a hard edge: everything within it is reachable, nothing beyond is.
    #[test]
    fn a_budget_bounds_what_can_be_reached() {
        let tiles: Vec<_> = HexCoord::ORIGIN
            .within_radius(6)
            .into_iter()
            .map(|coord| tile(coord, 4))
            .collect();
        let footing = footing_from(&tiles);
        let Some(start) = footing.ground(HexCoord::ORIGIN) else {
            unreachable!("the origin has ground")
        };

        let reach = Reach::from(start, &footing, Some(2));

        assert_eq!(reach.cost(start.pos), Some(0), "you are where you are");
        for standing in reach.surfaces() {
            let distance = HexCoord::ORIGIN.distance(standing.pos.coord);
            assert!(
                distance <= 2,
                "{:?} is {distance} away but the budget was 2",
                standing.pos.coord
            );
        }
        // On open ground, cost is exactly hex distance, so a budget of 2 reaches the
        // centre plus two full rings: 1 + 6 + 12.
        assert_eq!(reach.surfaces().count(), 19);
    }

    /// The path is as long as the cost says, and starts where the piece stands.
    #[test]
    fn a_path_is_as_long_as_its_cost() {
        let tiles: Vec<_> = HexCoord::ORIGIN
            .within_radius(4)
            .into_iter()
            .map(|coord| tile(coord, 4))
            .collect();
        let footing = footing_from(&tiles);
        let (Some(start), Some(target)) = (
            footing.ground(HexCoord::ORIGIN),
            footing.ground(HexCoord::new_cubic(3, -3, 0)),
        ) else {
            unreachable!("both ends are ground")
        };

        let reach = Reach::from(start, &footing, None);
        let Some(cost) = reach.cost(target.pos) else {
            panic!("open ground three hexes away is reachable")
        };
        let Some(path) = reach.path_to(target.pos) else {
            panic!("a reachable surface has a path")
        };

        assert_eq!(cost, 3, "three hexes across open ground is three steps");
        assert_eq!(
            path.len(),
            cost as usize + 1,
            "a path includes the surface stood on, so it is one longer than the cost"
        );
    }

    /// Somewhere unreachable has neither a cost nor a path — the two must agree, or a
    /// tile could be highlighted as reachable and then refuse to be walked to.
    #[test]
    fn an_unreachable_surface_has_no_cost_and_no_path() {
        let here = HexCoord::ORIGIN;
        let island = HexCoord::new_cubic(5, -5, 0);
        let footing = footing_from(&[tile(here, 4), tile(island, 4)]);
        let Some(start) = footing.ground(here) else {
            unreachable!("the origin has ground")
        };

        let reach = Reach::from(start, &footing, None);
        let island_pos = TilePos::new(island, 4);

        assert_eq!(reach.cost(island_pos), None);
        assert!(reach.path_to(island_pos).is_none());
    }

    /// **The point of having a pathfinder.** A wall across the direct line, with a way
    /// round it, must be walked around rather than reported as unreachable.
    ///
    /// This fails on a straight-line router, which is what this replaced: the first
    /// coordinate on the line has nothing standable at a reachable height, so it gave
    /// up and a perfectly ordinary click did nothing at all.
    #[test]
    fn a_route_goes_around_an_obstacle() {
        // A patch of flat ground, minus a wall of raised tiles that the straight line
        // from origin to (0, 3, -3) runs into. The wall stops short of the edge, so
        // there is a way round it.
        let mut tiles = Vec::new();
        for coord in HexCoord::ORIGIN.within_radius(4) {
            // The obstructing row sits three levels up: too tall to step onto from
            // level 4, so it is impassable rather than merely uphill.
            let blocked = coord.y() == 1 && coord.x() >= -1;
            tiles.push(tile(coord, if blocked { 8 } else { 4 }));
        }
        let footing = footing_from(&tiles);

        let (Some(from), Some(to)) = (
            footing.ground(HexCoord::ORIGIN),
            footing.ground(HexCoord::new_cubic(0, 3, -3)),
        ) else {
            unreachable!("both ends are ordinary ground")
        };

        let Some(steps) = route(from, to, &footing) else {
            panic!("a wall with a way round it is not 'no route exists'")
        };
        assert_eq!(
            steps.first().map(|s| s.pos),
            Some(from.pos),
            "a path starts where the piece stands"
        );
        assert_eq!(
            steps.last().map(|s| s.pos),
            Some(to.pos),
            "and ends where it was asked to go"
        );
        assert!(
            steps.iter().all(|s| s.pos.level == 4),
            "the detour should stay on the low ground, never on the wall"
        );
        assert!(
            steps.len() > 4,
            "going around is longer than the four-hex straight line; got {}",
            steps.len()
        );
    }

    /// Terrain is not guaranteed connected, so a missing column is a legitimate
    /// "no route exists" — and with a pathfinder that claim is only true because this
    /// fixture is a bare line with nothing either side of it to detour through.
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

    /// Non-solid substances are not standable, which stopped being hypothetical when
    /// the showcase map added a river. Water is drawn as a prism like any other run.
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
