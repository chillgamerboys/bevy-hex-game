//! Focused headless contracts for unsupported-actor landing decisions.

use bevy::prelude::*;
use hex_core::{
    Busy, Headroom, HexCoord, HexSpan, HexTile, PerceptionSystems, RunBottom, TerrainSystems,
    TilePos, TraversalBlockers, TraversalProfile, UnitId, MAX_HEADROOM,
};
use hex_test_app::HeadlessAppBuilder;
use hex_test_support::{fixture_assets, STONE};
use hex_units::{
    plan_unsupported_actor_landing, Body, Footing, MovingTo, NoLanding, Standing, StandsOn,
    TerrainOccupancy, UnitOccupancy,
};

fn at(q: i32, r: i32, level: i32) -> TilePos {
    TilePos::new(HexCoord::from_axial(q, r), level)
}

fn span(level: i32) -> HexSpan {
    #[expect(
        clippy::cast_precision_loss,
        reason = "small exact fixture levels round-trip through f32"
    )]
    let bottom = level as f32;
    HexSpan::new(bottom, bottom + 1.0)
}

fn standing(pos: TilePos) -> Standing {
    Standing {
        pos,
        span: span(pos.level),
    }
}

#[derive(Clone, Copy)]
struct Surface {
    pos: TilePos,
    substance: hex_core::SubstanceId,
    headroom: Headroom,
}

impl Surface {
    fn open(pos: TilePos) -> Self {
        Self {
            pos,
            substance: STONE,
            headroom: Headroom(MAX_HEADROOM),
        }
    }
}

#[expect(
    clippy::expect_used,
    reason = "invalid dependency-limited fixture assets should fail the contract immediately"
)]
fn build_footing(body: Body, blockers: &TraversalBlockers, surfaces: &[Surface]) -> Footing {
    let (_, table) = fixture_assets().expect("fixture assets should be valid");
    let spans = surfaces
        .iter()
        .map(|surface| span(surface.pos.level))
        .collect::<Vec<_>>();
    Footing::from_tiles(
        surfaces
            .iter()
            .zip(&spans)
            .map(|(surface, span)| (&surface.pos, span, &surface.substance, &surface.headroom)),
        &table,
        body,
        Some(blockers),
    )
}

#[test]
fn highest_legal_unoccupied_support_below_wins_before_any_lateral_surface() {
    let actor = UnitId(10);
    let from = at(0, 0, 8);
    let highest = at(0, 0, 6);
    let occupied = at(0, 0, 7);
    let surfaces = [
        Surface::open(at(0, 0, 2)),
        Surface::open(highest),
        Surface::open(occupied),
        Surface::open(at(1, 0, 8)),
    ];
    let footing = build_footing(
        Body::new(TraversalProfile::WALKER),
        &Default::default(),
        &surfaces,
    );
    let occupancy = UnitOccupancy::from_positions([(UnitId(3), occupied)]);

    assert_eq!(
        plan_unsupported_actor_landing(actor, from, &footing, &occupancy),
        Ok(standing(highest))
    );
}

#[test]
fn body_headroom_solidity_blockers_and_reservations_all_fail_closed() {
    let actor = UnitId(10);
    let from = at(0, 0, 8);
    let cramped = at(0, 0, 7);
    let non_solid = at(0, 0, 6);
    let blocked = at(0, 0, 5);
    let reserved = at(0, 0, 4);
    let legal = at(0, 0, 3);
    let surfaces = [
        Surface {
            pos: cramped,
            substance: STONE,
            headroom: Headroom(1),
        },
        Surface {
            pos: non_solid,
            substance: hex_core::SubstanceId::AIR,
            headroom: Headroom(MAX_HEADROOM),
        },
        Surface::open(blocked),
        Surface::open(reserved),
        Surface::open(legal),
    ];
    let mut blockers = TraversalBlockers::new();
    blockers.insert(blocked);
    let footing = build_footing(Body::new(TraversalProfile::WALKER), &blockers, &surfaces);
    let occupancy = UnitOccupancy::from_positions([(UnitId(1), reserved)]);

    assert_eq!(
        plan_unsupported_actor_landing(actor, from, &footing, &occupancy),
        Ok(standing(legal))
    );
}

#[test]
fn lateral_order_is_distance_then_level_difference_then_lower_then_tile_position() {
    let actor = UnitId(10);
    let from = at(0, 0, 5);
    let farther_exact_level = at(2, 0, 5);
    let nearer_large_drop = at(1, 0, 1);
    let surfaces = [
        Surface::open(farther_exact_level),
        Surface::open(nearer_large_drop),
    ];
    let footing = build_footing(
        Body::new(TraversalProfile::WALKER),
        &Default::default(),
        &surfaces,
    );

    assert_eq!(
        plan_unsupported_actor_landing(actor, from, &footing, &Default::default()),
        Ok(standing(nearer_large_drop)),
        "hex distance is the first lateral key"
    );

    let smaller_level_difference = at(1, 0, 3);
    let larger_level_difference = at(0, 1, 1);
    let surfaces = [
        Surface::open(larger_level_difference),
        Surface::open(smaller_level_difference),
    ];
    let footing = build_footing(
        Body::new(TraversalProfile::WALKER),
        &Default::default(),
        &surfaces,
    );
    assert_eq!(
        plan_unsupported_actor_landing(actor, from, &footing, &Default::default()),
        Ok(standing(smaller_level_difference)),
        "absolute level difference is the second lateral key"
    );

    let lower = at(1, 0, 4);
    let higher = at(0, 1, 6);
    let surfaces = [Surface::open(higher), Surface::open(lower)];
    let footing = build_footing(
        Body::new(TraversalProfile::WALKER),
        &Default::default(),
        &surfaces,
    );
    assert_eq!(
        plan_unsupported_actor_landing(actor, from, &footing, &Default::default()),
        Ok(standing(lower)),
        "lower sorts before higher at equal distance and level difference"
    );

    let tile_first = at(-1, 1, 4);
    let tile_second = at(0, 1, 4);
    let surfaces = [Surface::open(tile_second), Surface::open(tile_first)];
    let footing = build_footing(
        Body::new(TraversalProfile::WALKER),
        &Default::default(),
        &surfaces,
    );
    assert_eq!(
        plan_unsupported_actor_landing(actor, from, &footing, &Default::default()),
        Ok(standing(tile_first.min(tile_second))),
        "exact TilePos is the final deterministic key"
    );
}

#[test]
fn relocating_the_earlier_actor_reserves_its_landing_for_the_next_plan() {
    let from = at(0, 0, 7);
    let first_choice = at(0, 0, 5);
    let second_choice = at(0, 0, 3);
    let surfaces = [Surface::open(second_choice), Surface::open(first_choice)];
    let footing = build_footing(
        Body::new(TraversalProfile::WALKER),
        &Default::default(),
        &surfaces,
    );
    let first = UnitId(1);
    let second = UnitId(2);
    let mut occupancy = UnitOccupancy::from_positions([(first, from), (second, from)]);

    let landing = plan_unsupported_actor_landing(first, from, &footing, &occupancy)
        .expect("the first actor should land");
    occupancy.relocate(first, landing.pos);

    assert_eq!(
        plan_unsupported_actor_landing(second, from, &footing, &occupancy),
        Ok(standing(second_choice))
    );
}

#[test]
fn no_surface_returns_typed_no_landing() {
    let from = at(0, 0, 7);
    let footing = build_footing(
        Body::new(TraversalProfile::WALKER),
        &Default::default(),
        &[],
    );

    assert_eq!(
        plan_unsupported_actor_landing(UnitId(4), from, &footing, &Default::default()),
        Err(NoLanding { from })
    );
}

#[derive(Resource, Debug, Default, PartialEq, Eq)]
struct RefreshObservation {
    terrain_was_current: bool,
    movement_was_current: bool,
    busy_was_cleared: bool,
    perception_ran_after: bool,
}

fn observe_reconcile_actor_phase(
    terrain: Res<TerrainOccupancy>,
    unit: Query<(&StandsOn, Has<Busy>)>,
    mut observation: ResMut<RefreshObservation>,
) {
    let Ok((standing, busy)) = unit.single() else {
        return;
    };
    observation.terrain_was_current = terrain.contains(at(1, 0, 2));
    observation.movement_was_current = standing.0.pos == at(1, 0, 2);
    observation.busy_was_cleared = !busy;
}

fn observe_perception_phase(mut observation: ResMut<RefreshObservation>) {
    observation.perception_ran_after = observation.terrain_was_current
        && observation.movement_was_current
        && observation.busy_was_cleared;
}

#[test]
fn refresh_phase_publishes_occupancy_then_reconciles_movement_before_actors() {
    let mut builder = HeadlessAppBuilder::new()
        .with_minimal_plugins()
        .with_update_sets();
    let app = builder.app_mut();
    app.configure_sets(
        Update,
        (
            TerrainSystems::ApplyWorld,
            TerrainSystems::RefreshProjections,
            TerrainSystems::ReconcileActors,
            TerrainSystems::ConsumeOutcomes,
            PerceptionSystems::ResolveIllumination,
        )
            .chain(),
    )
    .init_resource::<RefreshObservation>()
    .add_plugins(hex_units::plugin)
    .add_systems(
        Update,
        (
            observe_reconcile_actor_phase.in_set(TerrainSystems::ReconcileActors),
            observe_perception_phase.in_set(PerceptionSystems::ResolveIllumination),
        ),
    );

    let start = standing(at(0, 0, 2));
    let destination = standing(at(1, 0, 2));
    app.world_mut()
        .spawn((HexTile, destination.pos, RunBottom(destination.pos.level)));
    app.world_mut().spawn((
        UnitId(7),
        StandsOn(start),
        MovingTo::new(vec![start, destination], 0.0),
        Busy,
    ));

    let mut app = builder.build();
    app.update();

    assert_eq!(
        *app.world().resource::<RefreshObservation>(),
        RefreshObservation {
            terrain_was_current: true,
            movement_was_current: true,
            busy_was_cleared: true,
            perception_ran_after: true,
        }
    );
    let moving_count = {
        let world = app.world_mut();
        let mut moving = world.query::<&MovingTo>();
        moving.iter(world).count()
    };
    assert_eq!(
        moving_count, 0,
        "the completed stale route should be gone before actor reconciliation"
    );
    assert!(
        app.world()
            .resource::<hex_units::MovementCrossings>()
            .iter()
            .any(|(_, crossed)| crossed.pos == destination.pos),
        "the exact domain crossing remains published"
    );
}
