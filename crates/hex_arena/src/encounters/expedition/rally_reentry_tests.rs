//! Controller-backed re-entry checks; no live actors move during route proofs.

use super::*;
use hex_core::SubstanceId;

fn floor() -> (ArenaTerrainView, ArenaVoxelGeometry, CollisionWorld) {
    let geometry = ArenaVoxelGeometry {
        radius: 187,
        ..Default::default()
    };
    let mut view = ArenaTerrainView::default();
    for coord in HexCoord::ORIGIN.within_radius(24) {
        view.voxels.insert(TilePos::new(coord, 0), SubstanceId(1));
    }
    let mut collision = CollisionWorld::default();
    collision.refresh(&view, geometry);
    (view, geometry, collision)
}

fn point(x: f32, z: f32) -> Vec3 {
    Vec3::new(x, SKIN, z)
}

fn minion(id: ActorId, feet: Vec3, role: ExpeditionRole) -> Actor {
    let mut actor = Actor::spawn(id, feet, Vec3::X);
    actor.configure_expedition(role, &ArenaTuning::default().encounters);
    actor.party = Some(0);
    actor.body.grounded = true;
    actor.grounded = true;
    actor
}

fn session(path: Vec<Vec3>, starts: &[Vec3]) -> ArenaSession {
    let end = *path.last().expect("route endpoint");
    let mut result = ArenaSession::default();
    result.actors = starts
        .iter()
        .enumerate()
        .map(|(i, feet)| {
            minion(
                ActorId::try_from(i + 1).expect("bounded test roster"),
                *feet,
                ExpeditionRole::Goblin,
            )
        })
        .collect();
    let mut control = Control {
        troll: Some(200),
        started: true,
        active: true,
        paths: [(0, path)].into(),
        cursors: BTreeMap::new(),
        next_reentry_actor: 0,
        gathering: vec![end],
        ordered_parties: 1,
        ordered_actors: starts.len(),
    };
    for actor in &result.actors {
        control.cursors.insert(actor.id, Order::default());
        result
            .encounter
            .brains
            .insert(actor.id, brain::Brain::new(actor.id, actor.feet));
    }
    let mut troll = Actor::spawn(200, end + Vec3::Z * 5.0, Vec3::X);
    troll.configure_expedition(ExpeditionRole::Troll, &ArenaTuning::default().encounters);
    result.actors.push(troll);
    result.encounter.expedition = Some(control);
    result
}

fn order(session: &ArenaSession, id: ActorId) -> &Order {
    session
        .encounter
        .expedition
        .as_ref()
        .expect("rally")
        .cursors
        .get(&id)
        .expect("order")
}

fn join(session: &mut ArenaSession, view: &ArenaTerrainView, geometry: ArenaVoxelGeometry) {
    for _ in 0..600 {
        session.tick += 1;
        session.advance_rally(view, geometry, &ArenaTuning::default());
        if order(session, 1).joined {
            return;
        }
    }
    panic!("reachable re-entry did not finish its bounded proof");
}

#[test]
fn kited_survivor_joins_near_troll_and_keeps_every_later_bend() {
    let (view, geometry, collision) = floor();
    let path = vec![
        point(-20.0, 0.0),
        point(-12.0, 0.0),
        point(-4.0, 0.0),
        point(4.0, 0.0),
        point(4.0, 4.0),
        point(8.0, 4.0),
        point(8.0, 8.0),
        point(12.0, 8.0),
    ];
    let start = point(4.0, -2.0);
    let mut session = session(path.clone(), &[start]);
    session.collision = collision;
    join(&mut session, &view, geometry);
    assert_eq!(order(&session, 1).cursor, 3, "must not return to camp");
    assert_eq!(session.actors.first().expect("survivor").feet, start);
    for (index, support) in path.iter().enumerate().skip(3) {
        let actor = session.actors.first_mut().expect("survivor");
        actor.feet = *support;
        actor.previous_feet = *support;
        session.tick += 1;
        session.advance_rally(&view, geometry, &ArenaTuning::default());
        assert_eq!(order(&session, 1).cursor, index + 1);
        assert!(order(&session, 1).joined);
        assert_eq!(
            session.encounter.brains.get(&1).expect("brain").rally_goal,
            path.get(index + 1).or(path.last()).copied(),
            "each subsequent bend remains the next ordered support"
        );
    }
}

#[test]
fn returning_after_combat_rejoins_forward_from_displaced_pose() {
    let (view, geometry, collision) = floor();
    let path: Vec<_> = (-10_i16..=10)
        .map(|x| point(f32::from(x) * 2.0, 0.0))
        .collect();
    let mut session = session(path, &[point(-20.0, 0.0)]);
    session.collision = collision;
    join(&mut session, &view, geometry);
    let original_cursor = order(&session, 1).cursor;
    session.encounter.runtime.push(PartyRuntime {
        snapshot: PartySnapshot {
            id: 0,
            phase: PartyPhase::Active,
            home: point(-20.0, 0.0),
            living: 1,
        },
        knowledge: Some(Knowledge {
            point: point(14.0, 0.0),
            velocity: Vec3::ZERO,
            tick: 1,
            direct: true,
            cue_kind: None,
            observed: None,
        }),
        last_sight: 1,
        last_cue_id: None,
        leash: 100.0,
        search: 1.0,
        battle_search: None,
    });
    session.advance_rally(&view, geometry, &ArenaTuning::default());
    session.actors.first_mut().expect("survivor").feet = point(16.0, 2.0);
    session.advance_rally(&view, geometry, &ArenaTuning::default());
    assert!(!order(&session, 1).joined, "combat defers re-entry proofs");
    assert_eq!(order(&session, 1).cursor, original_cursor);
    let party = session.encounter.runtime.first_mut().expect("party");
    party.snapshot.phase = PartyPhase::Returning;
    party.knowledge = None;
    join(&mut session, &view, geometry);
    assert_eq!(order(&session, 1).cursor, 18);
    assert_eq!(
        session.encounter.brains.get(&1).expect("brain").rally_goal,
        Some(point(16.0, 0.0))
    );
    assert!(session
        .encounter
        .runtime
        .first()
        .expect("party")
        .knowledge
        .is_none());
}

#[test]
fn blocked_entry_waits_then_retries_after_wall_removal_for_adult_and_shaman() {
    for role in [ExpeditionRole::Goblin, ExpeditionRole::Shaman] {
        let (mut view, geometry, mut collision) = floor();
        let wall: Vec<_> = (-20..=20)
            .flat_map(|r| {
                (1..=10).map(move |level| TilePos::new(HexCoord::from_axial(0, r), level))
            })
            .collect();
        for cell in &wall {
            view.voxels.insert(*cell, SubstanceId(1));
        }
        view.revision += 1;
        collision.refresh(&view, geometry);
        let start = HexCoord::from_axial(-3, 0).to_world(SKIN);
        let end = HexCoord::from_axial(3, 0).to_world(SKIN);
        let actor = minion(1, start, role);
        let path = [end, end + Vec3::X * 2.0];
        let mut order = Order::default();
        let mut retried = false;
        for tick in 0..1200 {
            if tick >= order.retry_tick {
                let spent = reenter(
                    &mut order,
                    &actor,
                    &path,
                    None,
                    &collision,
                    &view,
                    geometry,
                    &ArenaTuning::default(),
                    tick,
                    REENTRY_TICKS_PER_ACTOR,
                );
                assert!(spent <= REENTRY_TICKS_PER_ACTOR);
                retried |= order.retry_tick > tick;
            }
            assert!(
                !order.joined,
                "physical wall must prevent re-entry for {role:?}"
            );
        }
        assert!(retried, "blocked proof must wait between candidate passes");
        for cell in &wall {
            view.voxels.remove(cell);
        }
        view.revision += 1;
        collision.refresh(&view, geometry);
        for tick in 1200..1800 {
            if tick >= order.retry_tick {
                reenter(
                    &mut order,
                    &actor,
                    &path,
                    None,
                    &collision,
                    &view,
                    geometry,
                    &ArenaTuning::default(),
                    tick,
                    REENTRY_TICKS_PER_ACTOR,
                );
            }
            if order.joined {
                break;
            }
        }
        assert!(order.joined, "opening the wall must permit delayed entry");
        assert_eq!(actor.feet, start, "proof must not mutate the live actor");
    }
}

#[test]
fn full_rally_reentry_service_is_deterministic_bounded_and_round_robin() {
    let (view, geometry, collision) = floor();
    let starts = vec![point(-20.0, 0.0); 109];
    let mut first = session(vec![point(20.0, 0.0)], &starts);
    let mut second = session(vec![point(20.0, 0.0)], &starts);
    first.collision = collision.clone();
    second.collision = collision;
    for frame in 1..=2 {
        for session in [&mut first, &mut second] {
            session.tick += 1;
            session.advance_rally(&view, geometry, &ArenaTuning::default());
        }
        let progress = |session: &ArenaSession| {
            session
                .encounter
                .expedition
                .as_ref()
                .expect("rally")
                .cursors
                .iter()
                .filter_map(|(id, order)| order.probe.as_ref().map(|probe| (*id, probe.ticks)))
                .collect::<Vec<_>>()
        };
        let probes = progress(&first);
        assert_eq!(probes, progress(&second));
        assert_eq!(
            probes.len(),
            frame * 4,
            "service moves to new pending actors"
        );
        assert!(probes
            .iter()
            .all(|(_, ticks)| *ticks < REENTRY_TICKS_PER_ACTOR));
        assert!(
            probes.iter().map(|(_, ticks)| ticks).sum::<usize>() <= frame * REENTRY_TICKS_PER_FRAME
        );
        assert!(first
            .actors
            .iter()
            .filter(|actor| actor.id != 200)
            .all(|actor| actor.feet == point(-20.0, 0.0)));
    }
    // A completed long entry remains valid while approaching its distant goal.
    join(&mut first, &view, geometry);
    first.advance_rally(&view, geometry, &ArenaTuning::default());
    assert!(order(&first, 1).joined);
}
