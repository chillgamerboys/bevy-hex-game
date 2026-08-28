//! Which surfaces a piece may step between, and how it gets from one to another.
//!
//! # The rules
//!
//! > A body may **stand** on a surface when its shared
//! > [`TraversalProfile`](hex_core::TraversalProfile) admits the substance and
//! > [`Headroom`](hex_core::Headroom).
//!
//! > A **step** is legal when both endpoints are adjacent standable surfaces, the
//! > profile admits the climb or drop, and their clear volumes share a body-high
//! > lateral aperture.
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
//! [`Reach`](crate::movement::Reach) floods outward when a caller needs a complete
//! movement field. [`route`] and [`route_with_occupancy`] instead use deterministic
//! destination-specific A*, avoiding a whole-world flood for long exploration clicks.
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
//! Steps all cost one. A* uses horizontal hex distance, which never overestimates even
//! when stacked surfaces force a detour, plus exact positional tie-breaking.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::Arc;

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use hex_assets::SubstanceTable;
use hex_core::{
    AppSystems, AuthoritativeSystems, Headroom, HexCoord, HexSpan, Mode, PausableSystems, Screen,
    SimulationRole, SubstanceId, TerrainSystems, TilePos, TraversalBlockers, TraversalEndpoint,
    TraversalProfile, UnitId,
};

use crate::{
    AuthoredObjectOccupancy, AuthoredObjectOccupancySystems, TerrainOccupancySystems, UnitOccupancy,
};

/// Ordering for systems that consume a unit's logical position.
#[derive(SystemSet, Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum MovementSystems {
    /// Advance domain routes and reconcile [`StandsOn`](crate::StandsOn).
    Reconcile,
    /// Settle exploration movement before combat freezes its opening facts.
    HaltOnCombat,
}

/// Every whole waypoint crossed during the latest domain movement tick.
///
/// A single fixed tick may finish several route legs. Consumers that care whether a
/// route passed through an intermediate position must inspect this resource after
/// [`MovementSystems::Reconcile`] instead of sampling only the final
/// [`StandsOn`](crate::StandsOn).
///
/// Iteration order is a sim input — "the first crossing within hostile reach"
/// decides which unit combat freezes — so it must not inherit query iteration
/// order. Entries sort by the crossing unit's stable [`UnitId`] (unregistered
/// units last), and the stable sort keeps each unit's waypoints in route order.
#[derive(Resource, Debug, Default)]
pub struct MovementCrossings(Vec<(Option<UnitId>, Entity, Standing)>);

impl MovementCrossings {
    /// Crossed waypoints, deterministically ordered across units and in route
    /// order within one.
    pub fn iter(&self) -> impl Iterator<Item = (Entity, Standing)> + '_ {
        self.0
            .iter()
            .map(|&(_, entity, standing)| (entity, standing))
    }

    pub(crate) fn clear(&mut self) {
        self.0.clear();
    }

    pub(crate) fn push(&mut self, unit: Option<UnitId>, entity: Entity, standing: Standing) {
        self.0.push((unit, entity, standing));
    }

    /// Orders entries by stable id; the stable sort preserves route order.
    pub(crate) fn sort_deterministic(&mut self) {
        // `is_none` first so unregistered units genuinely sort last —
        // `Option`'s own ordering would put `None` first.
        self.0.sort_by_key(|&(unit, _, _)| (unit.is_none(), unit));
    }
}

/// Registers the movement types.
///
/// [`Body`] is registered beside where it is defined, and route reconciliation lives
/// here because every kind of unit needs its logical position kept aligned with the
/// domain route.
pub fn plugin(app: &mut App) {
    app.register_type::<Body>()
        .init_resource::<SimulationRole>()
        .init_resource::<MovementCrossings>()
        .init_resource::<FootingCache>()
        .add_systems(OnExit(Screen::Gameplay), clear_footing_cache);
    app.configure_sets(
        Update,
        AuthoritativeSystems.run_if(resource_equals(SimulationRole::Authority)),
    );

    // Where a unit *is*, kept true as it walks. Separated from `units::plugin`, which
    // also reads the active scenario placements and spawns pieces: anything that needs
    // positions to stay honest — `hex_combat`, and its tests — wants this half without
    // that one. The route advances from the pausable virtual clock directly;
    // generic transform animation mirrors it as presentation.
    app.add_systems(
        Update,
        crate::units::reconcile_movement
            .in_set(MovementSystems::Reconcile)
            .in_set(TerrainSystems::RefreshProjections)
            .in_set(AppSystems::Update)
            .in_set(AuthoritativeSystems)
            .in_set(PausableSystems)
            .after(TerrainOccupancySystems::Publish)
            .after(AuthoredObjectOccupancySystems::Publish)
            .before(hex_anim::AnimationSystems::Drive),
    );
    // Committing to a long walk and then being ambushed halfway should leave the piece
    // where the ambush happened.
    app.add_systems(
        OnEnter(Mode::Combat),
        crate::units::halt_on_combat
            .in_set(MovementSystems::HaltOnCombat)
            .run_if(resource_equals(SimulationRole::Authority)),
    );
}

/// How much room a thing takes up, and therefore where it fits.
///
/// A struct rather than a bare [`hex_core::Level`] on purpose. Bodies can eventually gain a
/// footprint or a non-walking movement profile without changing every system that
/// carries one.
///
/// A footprint is deliberately not built yet. It is pure gameplay, invisible to the
/// map, and it first needs a decision this codebase has not taken: whether a wide body
/// may straddle a one-level step, or must have every hex of its footprint level.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[reflect(Component)]
pub struct Body {
    /// Exact movement and occupancy rules used by this body.
    pub profile: TraversalProfile,
}

impl Body {
    /// Creates a body governed by one traversal profile.
    #[must_use]
    pub const fn new(profile: TraversalProfile) -> Self {
        Self { profile }
    }

    /// The traversal rules this body uses.
    #[must_use]
    pub const fn traversal_profile(self) -> TraversalProfile {
        self.profile
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
#[derive(Debug)]
pub struct Footing {
    profile: TraversalProfile,
    by_pos: HashMap<TilePos, Standing>,
    headroom_by_pos: HashMap<TilePos, Headroom>,
    /// Surfaces at each coordinate, so one can be found without knowing which
    /// level its top happens to be at.
    surfaces: HashMap<HexCoord, Vec<Standing>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FootingSourceKey {
    terrain_revision: u64,
    substances: u64,
    blockers: u64,
    authored_objects: u64,
}

/// Session cache of body-specific standable terrain projections.
///
/// This cache owns no map authority. Its source key covers the public terrain
/// revision plus every non-tile input to [`Footing`], and it is cleared on gameplay
/// exit. Consumers share immutable [`Arc`] projections across hover and click paths.
#[derive(Resource, Debug, Default)]
pub struct FootingCache {
    source: Option<FootingSourceKey>,
    by_body: Vec<(Body, Arc<Footing>)>,
    builds: u64,
}

impl FootingCache {
    /// Returns a cached body projection or builds it exactly once for this source.
    pub fn get_or_build<'a>(
        &mut self,
        terrain_revision: u64,
        tiles: impl Iterator<Item = (&'a TilePos, &'a HexSpan, &'a SubstanceId, &'a Headroom)>,
        table: &SubstanceTable,
        body: Body,
        blockers: Option<&TraversalBlockers>,
        authored_objects: &AuthoredObjectOccupancy,
    ) -> Arc<Footing> {
        let source = FootingSourceKey {
            terrain_revision,
            substances: substance_solidity_fingerprint(table),
            blockers: blocker_fingerprint(blockers),
            authored_objects: authored_objects.fingerprint(),
        };
        if self.source != Some(source) {
            self.source = Some(source);
            self.by_body.clear();
        }
        if let Some((_, footing)) = self
            .by_body
            .iter()
            .find(|(cached_body, _)| *cached_body == body)
        {
            return Arc::clone(footing);
        }
        let footing = Arc::new(Footing::from_tiles_with_object_occupancy(
            tiles,
            table,
            body,
            blockers,
            authored_objects,
        ));
        self.builds = self.builds.wrapping_add(1);
        self.by_body.push((body, Arc::clone(&footing)));
        footing
    }

    /// Number of full tile projections built during this gameplay session.
    #[must_use]
    pub const fn builds(&self) -> u64 {
        self.builds
    }

    fn clear(&mut self) {
        self.source = None;
        self.by_body.clear();
        self.builds = 0;
    }
}

fn clear_footing_cache(mut cache: ResMut<FootingCache>) {
    cache.clear();
}

fn fnv_bytes(mut state: u64, bytes: impl IntoIterator<Item = u8>) -> u64 {
    for byte in bytes {
        state ^= u64::from(byte);
        state = state.wrapping_mul(1_099_511_628_211);
    }
    state
}

fn substance_solidity_fingerprint(table: &SubstanceTable) -> u64 {
    let mut fingerprint = 14_695_981_039_346_656_037_u64;
    for index in 0..table.len() {
        let Ok(index) = u16::try_from(index) else {
            return 0;
        };
        fingerprint = fnv_bytes(fingerprint, index.to_le_bytes());
        fingerprint = fnv_bytes(fingerprint, [u8::from(table.is_solid(SubstanceId(index)))]);
    }
    fingerprint
}

fn blocker_fingerprint(blockers: Option<&TraversalBlockers>) -> u64 {
    let mut fingerprint = 14_695_981_039_346_656_037_u64;
    for position in blockers.into_iter().flat_map(TraversalBlockers::iter) {
        fingerprint = fnv_bytes(fingerprint, position.coord.x().to_le_bytes());
        fingerprint = fnv_bytes(fingerprint, position.coord.y().to_le_bytes());
        fingerprint = fnv_bytes(fingerprint, position.level.to_le_bytes());
    }
    fingerprint
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
    /// - **Unblocked**, from the optional exact [`TraversalBlockers`] projection.
    ///   Generated features such as tree trunks occupy otherwise-solid surfaces;
    ///   omitting those roots would make presentation and pathfinding disagree.
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
        blockers: Option<&TraversalBlockers>,
    ) -> Self {
        Self::from_tiles_with_optional_object_occupancy(tiles, table, body, blockers, None)
    }

    /// Collects standable surfaces while enforcing exact authored-object volume.
    ///
    /// Production movement and pathfinding use this constructor after the session's
    /// [`AuthoredObjectOccupancy`] has been published. The older [`Self::from_tiles`]
    /// remains available to generator validation and synthetic fixtures that have no
    /// authored-object authority.
    pub fn from_tiles_with_object_occupancy<'a>(
        tiles: impl Iterator<Item = (&'a TilePos, &'a HexSpan, &'a SubstanceId, &'a Headroom)>,
        table: &SubstanceTable,
        body: Body,
        blockers: Option<&TraversalBlockers>,
        authored_objects: &AuthoredObjectOccupancy,
    ) -> Self {
        Self::from_tiles_with_optional_object_occupancy(
            tiles,
            table,
            body,
            blockers,
            Some(authored_objects),
        )
    }

    fn from_tiles_with_optional_object_occupancy<'a>(
        tiles: impl Iterator<Item = (&'a TilePos, &'a HexSpan, &'a SubstanceId, &'a Headroom)>,
        table: &SubstanceTable,
        body: Body,
        blockers: Option<&TraversalBlockers>,
        authored_objects: Option<&AuthoredObjectOccupancy>,
    ) -> Self {
        let profile = body.traversal_profile();
        let mut footing = Self {
            profile,
            by_pos: HashMap::default(),
            headroom_by_pos: HashMap::default(),
            surfaces: HashMap::default(),
        };

        for (pos, span, substance, headroom) in tiles {
            // Logical terrain publishes every material run, including buried strata
            // and non-solid liquids. Reject those with the cheap local predicate
            // before probing the ordered blocker and authored-volume indexes; neither
            // can make an otherwise inadmissible surface standable.
            if !profile.admits_surface(table.is_solid(*substance), *headroom) {
                continue;
            }
            if blockers.is_some_and(|blockers| blockers.contains(*pos)) {
                continue;
            }
            if authored_objects
                .is_some_and(|occupancy| occupancy.blocks_standing_body(*pos, profile))
            {
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
            footing.headroom_by_pos.insert(standing.pos, *headroom);
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

    /// Whether this footing's exact traversal profile admits one positional step.
    ///
    /// Combat uses this for adjacent melee reach as well as movement using it for
    /// routes, so a profile cannot validate one elevation rule and attack with
    /// another.
    #[must_use]
    pub fn admits_step(&self, from: TilePos, to: TilePos) -> bool {
        let (Some(from_headroom), Some(to_headroom)) = (
            self.headroom_by_pos.get(&from),
            self.headroom_by_pos.get(&to),
        ) else {
            return false;
        };
        self.profile.admits_transition(
            TraversalEndpoint::new(from, true, *from_headroom),
            TraversalEndpoint::new(to, true, *to_headroom),
        )
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

    /// Every standable surface in exact positional order.
    ///
    /// The backing maps are hash maps for fast pathfinding. Formation compression
    /// needs to compare the whole candidate set, so it receives an explicitly sorted
    /// projection rather than inheriting hash iteration order.
    #[must_use]
    pub fn standings(&self) -> Vec<Standing> {
        let mut standings: Vec<_> = self.by_pos.values().copied().collect();
        standings.sort_by_key(|standing| standing.pos);
        standings
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
        let mut candidates = Vec::new();
        self.steps_from_into(from, coord, &mut candidates);
        candidates
    }

    /// Reuses caller-owned scratch storage while preserving [`Self::steps_from`]'s
    /// exact candidate order.
    fn steps_from_into(&self, from: Standing, coord: HexCoord, candidates: &mut Vec<Standing>) {
        candidates.clear();
        candidates.extend(
            self.at_coord(coord)
                .iter()
                .filter(|candidate| self.admits_step(from.pos, candidate.pos))
                .copied(),
        );
        candidates.sort_by_key(|candidate| {
            (
                from.pos.level_step_to(candidate.pos).abs(),
                candidate.pos.level,
            )
        });
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
    /// no movement budget. The search is linear in the standable graph (at most six
    /// outgoing edges per surface); callers that use a shipped large map cache the
    /// unbounded result rather than recomputing it per interaction.
    #[must_use]
    pub fn from(start: Standing, footing: &Footing, budget: Option<u32>) -> Self {
        Self::flood(start, footing, budget, None, None)
    }

    /// Floods through terrain while treating every other occupied exact surface as a
    /// closed node.
    #[must_use]
    pub fn with_occupancy(
        start: Standing,
        footing: &Footing,
        budget: Option<u32>,
        occupancy: &UnitOccupancy,
        mover: UnitId,
    ) -> Self {
        Self::flood(start, footing, budget, None, Some((occupancy, mover)))
    }

    /// Floods only until `target` is discovered, or the connected component ends.
    ///
    /// This is the route-preserving projection used by formation planning. Its
    /// predecessor for `target` is identical to [`Self::from`]: both searches use
    /// the same breadth-first frontier and deterministic neighbor ordering. Stopping
    /// at discovery avoids traversing the rest of a large map when the highest
    /// priority formation slot is only one step away.
    pub(crate) fn until_with_occupancy(
        start: Standing,
        footing: &Footing,
        target: TilePos,
        occupancy: &UnitOccupancy,
        mover: UnitId,
    ) -> Self {
        Self::flood(start, footing, None, Some(target), Some((occupancy, mover)))
    }

    fn flood(
        start: Standing,
        footing: &Footing,
        budget: Option<u32>,
        target: Option<TilePos>,
        occupancy: Option<(&UnitOccupancy, UnitId)>,
    ) -> Self {
        let mut reach = Self::default();
        reach.steps.insert(
            start.pos,
            Step {
                standing: start,
                cost: 0,
                came_from: None,
            },
        );
        if target == Some(start.pos) {
            return reach;
        }
        // Formation members normally advance into an adjacent slot. The general
        // flood would discover that same one-edge predecessor, but only after
        // allocating a frontier and probing neighbor coordinates that precede it
        // in the deterministic order. Record the identical result directly.
        let direct = target.and_then(|target| {
            footing
                .at(target)
                .filter(|_| {
                    footing.admits_step(start.pos, target)
                        && occupancy.is_none_or(|(occupancy, mover)| {
                            !occupancy.is_occupied(target, Some(mover))
                        })
                })
                .map(|standing| (target, standing))
        });
        if let Some((target, standing)) = direct {
            reach.steps.insert(
                target,
                Step {
                    standing,
                    cost: 1,
                    came_from: Some(start.pos),
                },
            );
            return reach;
        }

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
                    if occupancy.is_some_and(|(occupancy, mover)| {
                        occupancy.is_occupied(next.pos, Some(mover))
                    }) {
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
                    if target == Some(next.pos) {
                        return reach;
                    }
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

    /// The exact standing surface reached at `pos`, if this search discovered it.
    ///
    /// Presentation uses this to mark a distant destination without allocating and
    /// reversing the complete route merely to recover its last element.
    #[must_use]
    pub fn standing(&self, pos: TilePos) -> Option<Standing> {
        self.steps.get(&pos).map(|step| step.standing)
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
/// This destination-specific search leaves whole-field movement previews to [`Reach`].
#[must_use]
pub fn route(from: Standing, to: Standing, footing: &Footing) -> Option<Vec<Standing>> {
    a_star_route(from, to, footing, None)
}

/// The shortest walk that never enters another body's exact surface.
#[must_use]
pub fn route_with_occupancy(
    from: Standing,
    to: Standing,
    footing: &Footing,
    occupancy: &UnitOccupancy,
    mover: UnitId,
) -> Option<Vec<Standing>> {
    a_star_route(from, to, footing, Some((occupancy, mover)))
}

fn a_star_route(
    from: Standing,
    to: Standing,
    footing: &Footing,
    occupancy: Option<(&UnitOccupancy, UnitId)>,
) -> Option<Vec<Standing>> {
    if from.pos == to.pos {
        return Some(vec![from]);
    }
    if footing.at(from.pos).is_none() || footing.at(to.pos).is_none() {
        return None;
    }

    // `Reverse` turns BinaryHeap's maximum order into the canonical minimum key.
    // Insertion order preserves the same deterministic neighbour preference as the
    // breadth-first movement preview when several equally short paths exist. Exact
    // position remains a final tie-break if the monotonic counter ever wraps.
    let mut frontier = BinaryHeap::new();
    let mut insertion_order = 0_u64;
    frontier.push(Reverse((
        from.pos.coord.distance(to.pos.coord),
        0_u32,
        insertion_order,
        from.pos,
    )));
    let mut costs: HashMap<TilePos, u32> = HashMap::default();
    costs.insert(from.pos, 0_u32);
    let mut came_from: HashMap<TilePos, TilePos> = HashMap::default();
    let mut candidates = Vec::new();

    while let Some(Reverse((_estimate, current_cost, _order, current_pos))) = frontier.pop() {
        if costs.get(&current_pos).copied() != Some(current_cost) {
            continue;
        }
        if current_pos == to.pos {
            let mut positions = vec![current_pos];
            let mut cursor = current_pos;
            while cursor != from.pos {
                cursor = *came_from.get(&cursor)?;
                positions.push(cursor);
            }
            positions.reverse();
            return positions
                .into_iter()
                .map(|position| footing.at(position))
                .collect();
        }

        let current = footing.at(current_pos)?;
        for neighbor_coord in current_pos.coord.neighbors() {
            footing.steps_from_into(current, neighbor_coord, &mut candidates);
            for next in candidates.iter().copied() {
                if occupancy
                    .is_some_and(|(occupied, mover)| occupied.is_occupied(next.pos, Some(mover)))
                {
                    continue;
                }
                let next_cost = current_cost.saturating_add(1);
                if costs
                    .get(&next.pos)
                    .is_some_and(|known| *known <= next_cost)
                {
                    continue;
                }
                costs.insert(next.pos, next_cost);
                came_from.insert(next.pos, current_pos);
                let estimate = next_cost.saturating_add(next.pos.coord.distance(to.pos.coord));
                insertion_order = insertion_order.wrapping_add(1);
                frontier.push(Reverse((estimate, next_cost, insertion_order, next.pos)));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;
    use hex_assets::{ArtPalette, PaletteSwatch, SrgbColor, Substance, SubstanceFile, SwatchId};
    use hex_core::{AuthoredObjectVoxelRun, Level, MAX_HEADROOM};

    const STONE: SubstanceId = SubstanceId(10);

    fn table() -> SubstanceTable {
        let stone_id =
            SwatchId::new("terrain/stone").expect("the fixture swatch id should be valid");
        let stone = PaletteSwatch::new(
            "Stone",
            SrgbColor::new(0.5, 0.5, 0.5).expect("the fixture color should be valid"),
            BTreeSet::from(["test".to_owned()]),
        )
        .expect("the fixture swatch should be valid");
        let palette = ArtPalette::new(BTreeMap::from([(stone_id.clone(), stone)]))
            .expect("the fixture palette should be valid");
        let mut substances = HashMap::default();
        substances.insert("air".to_owned(), Substance::invisible(false, false));
        substances.insert(
            "stone".to_owned(),
            Substance::from_swatch(stone_id, true, true),
        );
        SubstanceTable::from_file(&SubstanceFile { substances }, &palette)
            .expect("the fixture substance should resolve through its palette")
    }

    /// A body of the size the game actually ships, for tests about stepping rather
    /// than about size.
    const NORMAL: Body = Body::new(TraversalProfile::WALKER);

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
            None,
        )
    }

    #[test]
    fn footing_cache_reuses_one_projection_until_an_authoritative_input_changes() {
        let coord = HexCoord::ORIGIN;
        let tiles = [tile(coord, 4)];
        let table = table();
        let authored = AuthoredObjectOccupancy::default();
        let mut blockers = TraversalBlockers::new();
        let mut cache = FootingCache::default();

        let first = cache.get_or_build(
            7,
            tiles
                .iter()
                .map(|(pos, span, substance, headroom)| (pos, span, substance, headroom)),
            &table,
            NORMAL,
            Some(&blockers),
            &authored,
        );
        let reused = cache.get_or_build(
            7,
            tiles
                .iter()
                .map(|(pos, span, substance, headroom)| (pos, span, substance, headroom)),
            &table,
            NORMAL,
            Some(&blockers),
            &authored,
        );
        assert!(Arc::ptr_eq(&first, &reused));
        assert_eq!(cache.builds(), 1);

        let revised = cache.get_or_build(
            8,
            tiles
                .iter()
                .map(|(pos, span, substance, headroom)| (pos, span, substance, headroom)),
            &table,
            NORMAL,
            Some(&blockers),
            &authored,
        );
        assert!(!Arc::ptr_eq(&first, &revised));
        assert_eq!(cache.builds(), 2, "terrain revision invalidates the cache");

        blockers.insert(tiles[0].0);
        let blocked = cache.get_or_build(
            8,
            tiles
                .iter()
                .map(|(pos, span, substance, headroom)| (pos, span, substance, headroom)),
            &table,
            NORMAL,
            Some(&blockers),
            &authored,
        );
        assert!(blocked.at(tiles[0].0).is_none());
        assert_eq!(cache.builds(), 3, "blocker changes invalidate the cache");

        blockers.remove(tiles[0].0);
        let occupied = AuthoredObjectOccupancy::from_runs([AuthoredObjectVoxelRun {
            top: TilePos::new(coord, 6),
            bottom: 5,
        }])
        .expect("the authored-object fixture is a valid run");
        let object_blocked = cache.get_or_build(
            8,
            tiles
                .iter()
                .map(|(pos, span, substance, headroom)| (pos, span, substance, headroom)),
            &table,
            NORMAL,
            Some(&blockers),
            &occupied,
        );
        assert!(object_blocked.at(tiles[0].0).is_none());
        assert_eq!(
            cache.builds(),
            4,
            "authored occupancy invalidates the cache"
        );

        cache.clear();
        assert_eq!(cache.builds(), 0);
        let after_clear = cache.get_or_build(
            8,
            tiles
                .iter()
                .map(|(pos, span, substance, headroom)| (pos, span, substance, headroom)),
            &table,
            NORMAL,
            Some(&blockers),
            &authored,
        );
        assert!(after_clear.at(tiles[0].0).is_some());
        assert_eq!(cache.builds(), 1);
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

    #[test]
    fn occupied_chokepoints_and_destinations_are_not_reachable() {
        let line: Vec<HexCoord> = HexCoord::ORIGIN.line_between(HexCoord::new_cubic(2, -2, 0));
        let tiles: Vec<_> = line.iter().map(|coord| tile(*coord, 4)).collect();
        let footing = footing_from(&tiles);
        let from = footing
            .ground(HexCoord::ORIGIN)
            .expect("the fixture start exists");
        let middle = footing
            .ground(HexCoord::new_cubic(1, -1, 0))
            .expect("the fixture chokepoint exists");
        let to = footing
            .ground(HexCoord::new_cubic(2, -2, 0))
            .expect("the fixture destination exists");
        let mover = UnitId(1);

        let chokepoint =
            UnitOccupancy::from_positions([(mover, from.pos), (UnitId(2), middle.pos)]);
        assert!(
            route_with_occupancy(from, to, &footing, &chokepoint, mover).is_none(),
            "a one-surface route cannot pass through another body"
        );

        let destination = UnitOccupancy::from_positions([(mover, from.pos), (UnitId(2), to.pos)]);
        let reach = Reach::with_occupancy(from, &footing, None, &destination, mover);
        assert_eq!(reach.cost(to.pos), None);
        assert_eq!(reach.path_to(to.pos), None);
        assert_eq!(
            destination.validate_route(&[from.pos, middle.pos, to.pos], mover),
            Err(crate::OccupancyBlock::Destination {
                position: to.pos,
                occupant: UnitId(2),
            }),
            "preview and authoritative validation consume the same projection"
        );
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

    /// A two-level step exceeds the ordinary walker's climb limit.
    #[test]
    fn a_cliff_blocks_the_route() {
        let a = HexCoord::ORIGIN;
        let [b, ..] = a.neighbors();
        let footing = footing_from(&[tile(a, 2), tile(b, 4)]);

        let Some(from) = footing.ground(a) else {
            unreachable!("a has ground")
        };
        let Some(to) = footing.ground(b) else {
            unreachable!("b has ground")
        };

        assert!(
            route(from, to, &footing).is_none(),
            "a two-level climb should be refused"
        );
    }

    /// Procedural validation calls the core predicate directly while live movement
    /// reaches it through `Body` and `Footing`. The boundary answers must be identical
    /// or a generated map can validate and still refuse the player.
    #[test]
    fn live_headroom_matches_the_shared_profile() {
        let low = HexCoord::ORIGIN;
        let [exact, ..] = low.neighbors();
        let tiles = [roofed(low, 4, 1), roofed(exact, 4, 2)];
        let footing = footing_from(&tiles);
        let profile = NORMAL.traversal_profile();

        assert_eq!(
            footing.ground(low).is_some(),
            profile.admits_surface(true, Headroom(1))
        );
        assert_eq!(
            footing.ground(exact).is_some(),
            profile.admits_surface(true, Headroom(2))
        );
        assert!(footing.ground(low).is_none());
        assert!(footing.ground(exact).is_some());
    }

    /// The same parity check for complete transitions: one level with shared clearance
    /// belongs to the live graph and two levels does not, exactly as the shared
    /// predicate reports.
    #[test]
    fn live_steps_match_the_shared_profile() {
        let from_coord = HexCoord::ORIGIN;
        let [one_coord, two_coord, ..] = from_coord.neighbors();
        let tiles = [tile(from_coord, 4), tile(one_coord, 5), tile(two_coord, 6)];
        let footing = footing_from(&tiles);
        let from = footing
            .ground(from_coord)
            .expect("the start is ordinary ground");
        let profile = NORMAL.traversal_profile();
        let one = TilePos::new(one_coord, 5);
        let two = TilePos::new(two_coord, 6);

        assert_eq!(
            footing.step_from(from, one_coord).is_some(),
            profile.admits_transition(
                TraversalEndpoint::new(from.pos, true, Headroom(MAX_HEADROOM)),
                TraversalEndpoint::new(one, true, Headroom(MAX_HEADROOM)),
            )
        );
        assert_eq!(
            footing.step_from(from, two_coord).is_some(),
            profile.admits_transition(
                TraversalEndpoint::new(from.pos, true, Headroom(MAX_HEADROOM)),
                TraversalEndpoint::new(two, true, Headroom(MAX_HEADROOM)),
            )
        );
        assert!(footing.step_from(from, one_coord).is_some());
        assert!(footing.step_from(from, two_coord).is_none());
    }

    /// Two rooms can each fit the walker while the boundary between them cannot. On a
    /// one-level ramp, two clear levels above each endpoint overlap by only one level;
    /// the lower room needs one extra clear voxel to provide a full lateral aperture.
    #[test]
    fn a_low_lintel_blocks_an_individually_standable_ramp() {
        let low_coord = HexCoord::ORIGIN;
        let [high_coord, ..] = low_coord.neighbors();
        let low = roofed(low_coord, 4, 2);
        let high = roofed(high_coord, 5, 2);
        let footing = footing_from(&[low, high]);
        let from = footing.ground(low_coord).expect("the lower room fits");
        let to = footing.ground(high_coord).expect("the higher room fits");

        assert!(route(from, to, &footing).is_none());
        assert!(route(to, from, &footing).is_none());
    }

    #[test]
    fn a_one_level_underground_ramp_with_shared_aperture_is_walkable() {
        let low_coord = HexCoord::ORIGIN;
        let [high_coord, ..] = low_coord.neighbors();
        let low = roofed(low_coord, 4, 3);
        let high = roofed(high_coord, 5, 2);
        let footing = footing_from(&[low, high]);
        let from = footing.ground(low_coord).expect("the lower room fits");
        let to = footing.ground(high_coord).expect("the higher room fits");

        assert!(route(from, to, &footing).is_some());
        assert!(route(to, from, &footing).is_some());
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

    /// Manual release-mode guard for the largest current flat selection graph.
    ///
    /// Radius 77 contains 18,019 surfaces. The movement preview builds this search
    /// once per stable selection, so its p95 must remain inside one 50 ms frame even
    /// when every surface belongs to the connected component.
    #[test]
    #[ignore = "manual release-mode radius-77 movement-preview benchmark"]
    fn radius_77_disclosed_preview_release_p95_stays_under_fifty_ms() {
        let tiles: Vec<_> = HexCoord::ORIGIN
            .within_radius(77)
            .into_iter()
            .map(|coord| tile(coord, 4))
            .collect();
        assert_eq!(tiles.len(), 18_019, "the benchmark radius drifted");

        let occupancy = UnitOccupancy::default();
        let mut samples = Vec::with_capacity(20);
        for _ in 0..20 {
            let started = std::time::Instant::now();
            let footing = footing_from(&tiles);
            let origin = footing
                .ground(HexCoord::ORIGIN)
                .expect("the benchmark origin should be standable");
            let reach = Reach::with_occupancy(origin, &footing, None, &occupancy, UnitId(1));
            assert_eq!(reach.surfaces().count(), tiles.len());
            samples.push(started.elapsed());
        }
        samples.sort_unstable();
        let p95 = samples
            .get(18)
            .copied()
            .expect("twenty samples should have a p95");
        assert!(
            p95 < std::time::Duration::from_millis(50),
            "radius-77 movement-preview p95 was {p95:?}"
        );
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
        let occupancy = UnitOccupancy::default();
        assert_eq!(
            Reach::until_with_occupancy(from, &footing, to.pos, &occupancy, UnitId(1))
                .path_to(to.pos),
            Some(steps.clone()),
            "a target-bounded projection must retain the full router's exact tie-breaks"
        );
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
            None,
        );
        assert!(footing.ground(coord).is_none());
    }

    #[test]
    fn exact_feature_blockers_are_not_footing() {
        let start = tile(HexCoord::ORIGIN, 4);
        let blocked = tile(HexCoord::from_axial(1, 0), 4);
        let tiles = [start, blocked];
        let mut blockers = TraversalBlockers::new();
        assert!(blockers.insert(blocked.0));
        let footing = Footing::from_tiles(
            tiles
                .iter()
                .map(|(pos, span, substance, headroom)| (pos, span, substance, headroom)),
            &table(),
            NORMAL,
            Some(&blockers),
        );

        assert!(footing.at(start.0).is_some());
        assert!(footing.at(blocked.0).is_none());
    }

    #[test]
    fn exact_authored_object_volume_removes_only_overlapping_body_footing() {
        let start = tile(HexCoord::ORIGIN, 4);
        let blocked = tile(HexCoord::from_axial(1, 0), 4);
        let clear_above = tile(HexCoord::from_axial(2, 0), 4);
        let tiles = [start, blocked, clear_above];
        let authored = AuthoredObjectOccupancy::from_runs([
            hex_core::AuthoredObjectVoxelRun::new(blocked.0.above(), 5),
            hex_core::AuthoredObjectVoxelRun::new(clear_above.0.above().above().above(), 7),
        ])
        .expect("authored-object fixture");
        let footing = Footing::from_tiles_with_object_occupancy(
            tiles
                .iter()
                .map(|(pos, span, substance, headroom)| (pos, span, substance, headroom)),
            &table(),
            NORMAL,
            None,
            &authored,
        );

        assert!(footing.at(start.0).is_some());
        assert!(footing.at(blocked.0).is_none());
        assert!(footing.at(clear_above.0).is_some());
    }

    #[test]
    fn authored_object_overlap_uses_the_movers_exact_body_height() {
        let surface_level = 10;
        let first = tile(HexCoord::from_axial(0, 0), surface_level);
        let second = tile(HexCoord::from_axial(1, 0), surface_level);
        let third = tile(HexCoord::from_axial(2, 0), surface_level);
        let support_only = tile(HexCoord::from_axial(3, 0), surface_level);
        let surfaces = [first, second, third, support_only];
        let authored = AuthoredObjectOccupancy::from_runs([
            hex_core::AuthoredObjectVoxelRun::new(first.0.above(), surface_level + 1),
            hex_core::AuthoredObjectVoxelRun::new(second.0.above().above(), surface_level + 2),
            hex_core::AuthoredObjectVoxelRun::new(
                third.0.above().above().above(),
                surface_level + 3,
            ),
            hex_core::AuthoredObjectVoxelRun::new(support_only.0, surface_level),
        ])
        .expect("height-specific authored-object fixture");

        let footing_for_height = |levels_tall| {
            Footing::from_tiles_with_object_occupancy(
                surfaces
                    .iter()
                    .map(|(pos, span, substance, headroom)| (pos, span, substance, headroom)),
                &table(),
                Body::new(TraversalProfile {
                    levels_tall,
                    max_climb: 1,
                    max_drop: 1,
                }),
                None,
                &authored,
            )
        };

        let walker = footing_for_height(2);
        assert!(walker.at(first.0).is_none());
        assert!(walker.at(second.0).is_none());
        assert!(walker.at(third.0).is_some());
        assert!(walker.at(support_only.0).is_some());

        let tall = footing_for_height(3);
        assert!(tall.at(first.0).is_none());
        assert!(tall.at(second.0).is_none());
        assert!(tall.at(third.0).is_none());
        assert!(tall.at(support_only.0).is_some());
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
            footing_for(
                &tiles,
                Body::new(TraversalProfile {
                    levels_tall: 1,
                    ..TraversalProfile::WALKER
                }),
            )
            .ground(coord)
            .is_some(),
            "one level of clearance is enough for a one-level body"
        );
        assert!(
            footing_for(&tiles, Body::new(TraversalProfile::WALKER))
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

        let short = Body::new(TraversalProfile {
            levels_tall: 1,
            ..TraversalProfile::WALKER
        });
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

        let tall_footing = footing_for(&tiles, Body::new(TraversalProfile::WALKER));
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
