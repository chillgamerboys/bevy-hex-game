//! Frozen dry routes through the real world producer and continuous controller.
//! These are travel contracts, not evidence of autonomous pursuit decisions.

use bevy::prelude::*;
use hex_arena::{ActorIntent, ArenaInput, ArenaSession, ArenaTuning, Species};
use hex_core::arena::{
    ArenaEncounter, ArenaMap, ArenaMaterials, ArenaSelection, ArenaTerrainView, ArenaTick,
    ArenaVoxelGeometry,
};
use hex_core::{
    HexCoord, TerrainBatchId, TerrainDamageKind, TerrainEdit, TerrainImpact,
    TerrainImpactDisposition, TerrainImpactOutcome, TerrainImpactResult, TilePos,
};

type Waypoint = (i32, i32, i32);

// Accepted seeds: Fort 640367719 and Seven Regions 703700113. These explicit
// cell centers were discovered once and replayed in both directions. The tests
// contain no path search, runtime route repair, position assignment, or jump/fly.
const FORT: &[Waypoint] = &[
    (9, -4, 15),
    (8, -3, 15),
    (7, -3, 15),
    (6, -3, 15),
    (5, -3, 15),
    (4, -3, 15),
    (3, -3, 15),
    (2, -2, 15),
    (1, -1, 15),
    (0, 0, 15),
];
const HIGH_PASS: &[Waypoint] = &[
    (3, -11, 17),
    (4, -12, 16),
    (5, -13, 15),
    (6, -14, 15),
    (7, -15, 15),
    (8, -16, 16),
    (9, -17, 17),
    (10, -18, 18),
];
const COURTYARD: &[Waypoint] = &[
    (3, -11, 17),
    (3, -10, 17),
    (2, -9, 17),
    (1, -8, 17),
    (0, -7, 17),
    (-1, -6, 16),
    (-2, -5, 16),
    (-3, -4, 16),
    (-4, -3, 15),
    (-5, -2, 15),
    (-6, -1, 15),
    (-7, 0, 15),
    (-8, 1, 15),
    (-9, 2, 15),
    (-10, 3, 15),
    (-11, 4, 16),
    (-12, 5, 15),
    (-13, 6, 15),
    (-14, 7, 16),
    (-15, 8, 15),
    (-15, 9, 15),
    (-15, 10, 15),
    (-15, 11, 15),
    (-16, 12, 15),
    (-17, 13, 15),
    (-18, 14, 15),
    (-19, 15, 15),
    (-20, 16, 15),
    (-21, 17, 15),
    (-22, 18, 15),
    (-23, 19, 15),
    (-23, 20, 15),
];
const CAVE_ENTRANCE: &[Waypoint] = &[
    (3, -11, 17),
    (2, -11, 17),
    (1, -11, 16),
    (0, -11, 15),
    (-1, -11, 15),
    (-2, -11, 16),
    (-3, -11, 17),
    (-4, -11, 16),
    (-5, -11, 15),
    (-6, -11, 15),
    (-7, -11, 16),
    (-8, -11, 17),
    (-9, -11, 18),
    (-10, -11, 18),
];

fn app(map: ArenaMap, encounter: ArenaEncounter) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(ArenaSelection { map, encounter })
        .add_plugins((hex_map::arena::plugin, hex_arena::plugin));
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
    app.update();
    tick(&mut app);
    app
}

fn tick(app: &mut App) {
    app.world_mut().run_schedule(ArenaTick);
}

fn human_feet(app: &App) -> Vec3 {
    app.world()
        .resource::<ArenaSession>()
        .actors
        .iter()
        .find(|actor| actor.id == 0)
        .expect("human")
        .feet
}

fn point(app: &App, (q, r, level): Waypoint) -> Vec3 {
    let pos = TilePos::new(HexCoord::from_axial(q, r), level);
    pos.coord
        .to_world(app.world().resource::<ArenaVoxelGeometry>().top(pos))
}

fn assert_clear_and_dry(app: &App, context: &str) {
    assert!(
        app.world().resource::<ArenaSession>().actor_volume_valid(
            0,
            app.world().resource::<ArenaTerrainView>(),
            *app.world().resource::<ArenaVoxelGeometry>()
        ),
        "{context}: human entered liquid or solid geometry at {:?}",
        human_feet(app)
    );
}

fn aim_and_run(app: &mut App, target: Vec3) {
    let direction = (target - human_feet(app)).with_y(0.0).normalize_or_zero();
    app.world_mut().resource_mut::<ArenaInput>().human = ActorIntent {
        aim: direction,
        movement: Vec2::Y,
        run: true,
        ..Default::default()
    };
}

fn move_to(app: &mut App, waypoint: Vec3, name: &str, segment: usize, ticks: &mut u32) {
    let initial = human_feet(app);
    for _ in 0..360 {
        let feet = human_feet(app);
        assert_clear_and_dry(app, name);
        if feet.with_y(0.0).distance(waypoint.with_y(0.0)) < 0.16
            && (feet.y - waypoint.y).abs() < 0.05
        {
            return;
        }
        assert!(feet.y>=initial.y.min(waypoint.y)-0.45,
            "{name} segment {segment} fell below the authored step: {initial:?} -> {waypoint:?}, feet {feet:?}");
        *ticks += 1;
        assert!(
            *ticks <= 3_600,
            "{name} exceeded the 30-second continuous route budget"
        );
        aim_and_run(app, waypoint);
        tick(app);
    }
    panic!(
        "{name} segment {segment} stalled: {initial:?} -> {waypoint:?}, feet {:?}",
        human_feet(app)
    );
}

fn settle(app: &mut App) {
    app.world_mut().resource_mut::<ArenaInput>().human = ActorIntent::default();
    for _ in 0..24 {
        tick(app);
        assert_clear_and_dry(app, "settle");
    }
    assert!(
        app.world().resource::<ArenaSession>().actor_pose_valid(
            0,
            app.world().resource::<ArenaTerrainView>(),
            *app.world().resource::<ArenaVoxelGeometry>()
        ),
        "the returned human must be supported"
    );
}

fn roundtrip(app: &mut App, route: &[Waypoint], anchor: &str) {
    assert!(route.len() <= 32);
    let initial = human_feet(app);
    let destination = *app
        .world()
        .resource::<ArenaTerrainView>()
        .anchors
        .get(anchor)
        .expect("encounter anchor");
    let points: Vec<_> = route.iter().map(|waypoint| point(app, *waypoint)).collect();
    assert!(initial.distance(*points.first().expect("route start")) < 0.18);
    let mut ticks = 0;
    for (segment, waypoint) in points.iter().enumerate() {
        move_to(app, *waypoint, anchor, segment, &mut ticks);
    }
    assert!(
        human_feet(app).distance(destination) < 3.8,
        "the route must reach the encounter approach"
    );
    settle(app);
    for (segment, waypoint) in points.iter().rev().enumerate() {
        move_to(app, *waypoint, anchor, segment, &mut ticks);
    }
    move_to(app, initial, anchor, points.len(), &mut ticks);
    settle(app);
    assert!(human_feet(app).distance(initial) < 0.18);
    assert!(app
        .world()
        .resource::<ArenaSession>()
        .projectiles
        .is_empty());
}

#[test]
fn fort_human_walks_through_gate_to_encounter_and_returns_dry() {
    roundtrip(
        &mut app(ArenaMap::Fort, ArenaEncounter::Goblins),
        FORT,
        "hostile_start",
    );
}
#[test]
fn seven_regions_human_reaches_mountain_high_pass_and_returns_dry() {
    roundtrip(
        &mut app(ArenaMap::SevenRegions, ArenaEncounter::Dragon),
        HIGH_PASS,
        "mountains_high_pass",
    );
}
#[test]
fn seven_regions_human_reaches_fort_courtyard_and_returns_dry() {
    roundtrip(
        &mut app(ArenaMap::SevenRegions, ArenaEncounter::Dragon),
        COURTYARD,
        "fort_fort_courtyard",
    );
}
#[test]
fn seven_regions_human_reaches_cave_entrance_and_returns_dry() {
    roundtrip(
        &mut app(ArenaMap::SevenRegions, ArenaEncounter::Dragon),
        CAVE_ENTRANCE,
        "caves_cave_entrance",
    );
}

fn assert_local_roundtrip(app: &App, id: u8, offset: HexCoord) {
    let session = app.world().resource::<ArenaSession>();
    let actor = session
        .actors
        .iter()
        .find(|actor| actor.id == id)
        .expect("local enemy");
    assert!(
        !actor.flying,
        "this contract exercises supported ground travel"
    );
    let view = app.world().resource::<ArenaTerrainView>();
    let geometry = *app.world().resource::<ArenaVoxelGeometry>();
    assert!(session.actor_pose_valid(id, view, geometry));
    let destination = actor.feet + offset.to_world(0.0);
    assert!(destination.distance(actor.feet) > 2.5);
    assert!(session.probe_dry_route(id,&[destination,actor.feet],view,geometry,app.world().resource::<ArenaTuning>()),
        "{:?} {id} cannot make the frozen local ground excursion {:?} -> {destination:?} and return",actor.species,actor.feet);
}

#[test]
fn every_fort_ground_profile_can_approach_and_return_inside_courtyard() {
    for encounter in [
        ArenaEncounter::Dragon,
        ArenaEncounter::Goblins,
        ArenaEncounter::ShamanParty,
        ArenaEncounter::Shadow,
    ] {
        let fixture = app(ArenaMap::Fort, encounter);
        let session = fixture.world().resource::<ArenaSession>();
        assert!(
            session.actors.len() > 1,
            "{encounter:?} must have an admitted enemy"
        );
        for actor in session.actors.iter().filter(|actor| actor.id != 0) {
            assert_local_roundtrip(&fixture, actor.id, HexCoord::from_axial(-2, 0));
        }
    }
}

#[test]
fn all_three_seven_region_parties_have_supported_local_ground_excursions() {
    let fixture = app(ArenaMap::SevenRegions, ArenaEncounter::Dragon);
    let session = fixture.world().resource::<ArenaSession>();
    assert_eq!(session.parties().len(), 3);
    assert_eq!(session.actors.len(), 11);
    for (id, species, q, r) in [
        (1, Species::Dragon, -2, 0),
        (2, Species::Shaman, -2, 0),
        (3, Species::Goblin, -2, 0),
        (4, Species::Goblin, -2, 0),
        (5, Species::Goblin, -2, 0),
        (6, Species::Goblin, -2, 2),
        (7, Species::Goblin, -2, 2),
        (8, Species::Goblin, -2, 1),
        (9, Species::Goblin, -1, -1),
        (10, Species::Goblin, -2, 0),
    ] {
        assert_eq!(
            session
                .actors
                .iter()
                .find(|a| a.id == id)
                .expect("stable roster")
                .species,
            species
        );
        assert_local_roundtrip(&fixture, id, HexCoord::from_axial(q, r));
    }
}

#[test]
fn physical_damage_lowers_a_traversed_fort_surface_and_the_human_still_returns() {
    let mut fixture = app(ArenaMap::Fort, ArenaEncounter::Goblins);
    let pos = TilePos::new(HexCoord::from_axial(8, -3), 15);
    let view = fixture.world().resource::<ArenaTerrainView>();
    assert!(view.voxels.contains_key(&pos));
    assert!(!view
        .edit_protected
        .get(&pos.coord)
        .is_some_and(|spans| spans
            .iter()
            .any(|(low, high)| (*low..=*high).contains(&pos.level))));
    let revision = view.revision;
    let impact = TerrainImpact {
        batch: TerrainBatchId(90_001),
        volume: vec![pos],
        kind: TerrainDamageKind::Physical,
        power: 8,
    };
    fixture.world_mut().write_message(impact.clone());
    tick(&mut fixture);
    let outcome = fixture
        .world_mut()
        .resource_mut::<Messages<TerrainImpactOutcome>>()
        .drain()
        .find(|outcome| outcome.batch == impact.batch)
        .expect("physical outcome");
    assert!(outcome.is_consistent_with(&impact));
    let TerrainImpactResult::Applied(outcomes) = outcome.result else {
        panic!("physical damage must be admitted");
    };
    assert_eq!(
        outcomes.first().expect("one voxel outcome").disposition,
        TerrainImpactDisposition::Destroyed
    );
    let view = fixture.world().resource::<ArenaTerrainView>();
    assert_eq!(view.revision, revision + 1);
    assert!(view.dirty_columns.contains(&pos.coord) && !view.full_rebuild);
    assert!(!view.voxels.contains_key(&pos));
    let lower = pos.below();
    assert!(
        view.voxels.contains_key(&lower),
        "one-level lower physical support"
    );
    let adjusted: Vec<_> = FORT
        .iter()
        .map(|&(q, r, level)| {
            if HexCoord::from_axial(q, r) == pos.coord {
                (q, r, lower.level)
            } else {
                (q, r, level)
            }
        })
        .collect();
    roundtrip(&mut fixture, &adjusted, "hostile_start");
}

#[test]
fn tall_fort_route_wall_stops_motion_then_clear_and_retry_succeeds_without_teleport() {
    let mut fixture = app(ArenaMap::Fort, ArenaEncounter::Goblins);
    let initial = human_feet(&fixture);
    let destination = point(&fixture, (8, -3, 15));
    let stone = fixture.world().resource::<ArenaMaterials>().stone;
    let cells: Vec<_> = (16..=20)
        .map(|level| TilePos::new(HexCoord::from_axial(8, -3), level))
        .collect();
    for pos in &cells {
        assert!(!fixture
            .world()
            .resource::<ArenaTerrainView>()
            .voxels
            .contains_key(pos));
        fixture.world_mut().write_message(TerrainEdit::Set {
            pos: *pos,
            substance: stone,
        });
    }
    tick(&mut fixture);
    assert!(cells.iter().all(|pos| fixture
        .world()
        .resource::<ArenaTerrainView>()
        .voxels
        .contains_key(pos)));
    for _ in 0..240 {
        aim_and_run(&mut fixture, destination);
        tick(&mut fixture);
        assert_clear_and_dry(&fixture, "blocked Fort route");
        assert!(
            (human_feet(&fixture).y - initial.y).abs() < 0.05,
            "a wall taller than one level cannot be stepped over"
        );
    }
    let stopped = human_feet(&fixture);
    assert!(
        stopped.distance(initial) > 0.15,
        "direct input must reach the obstacle"
    );
    assert!(
        stopped.distance(destination) > 1.0,
        "the body cannot enter the tall wall"
    );
    fixture.world_mut().resource_mut::<ArenaInput>().human = ActorIntent::default();
    for pos in &cells {
        fixture
            .world_mut()
            .write_message(TerrainEdit::Clear { pos: *pos });
    }
    tick(&mut fixture);
    assert!(cells.iter().all(|pos| !fixture
        .world()
        .resource::<ArenaTerrainView>()
        .voxels
        .contains_key(pos)));
    let mut ticks = 0;
    move_to(
        &mut fixture,
        destination,
        "cleared Fort route",
        0,
        &mut ticks,
    );
    move_to(&mut fixture, initial, "cleared Fort return", 1, &mut ticks);
    settle(&mut fixture);
    assert!(human_feet(&fixture).distance(initial) < 0.18);
}
