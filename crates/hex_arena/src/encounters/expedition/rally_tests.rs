use super::*;
use hex_core::arena::ArenaExpeditionRoute;

fn with_routes() -> (
    ArenaSession,
    ArenaTerrainView,
    ArenaVoxelGeometry,
    ArenaMaterials,
    ArenaTuning,
) {
    let (session, mut world, geometry, materials, tuning) = fixture();
    let sites = world.expedition.as_mut().expect("sites");
    let target = sites
        .encounters
        .get("forest_troll")
        .expect("Troll")
        .deployment
        .preferred;
    sites.route_nodes.insert("forest_troll".into(), target);
    sites
        .encounters
        .get_mut("forest_troll")
        .expect("Troll")
        .rally_entry = Some("forest_troll".into());
    for index in 1..=14 {
        let name = format!("forest_camp_{index:02}");
        let site = sites.encounters.get_mut(&name).expect("camp");
        let origin = site.deployment.preferred;
        site.rally_entry = Some(name.clone());
        sites.route_nodes.insert(name.clone(), origin);
        let supports: Vec<_> = origin
            .coord
            .line_between(target.coord)
            .into_iter()
            .map(|p| TilePos::new(p, 0))
            .collect();
        sites.routes.insert(
            name.clone(),
            ArenaExpeditionRoute {
                from: name,
                to: "forest_troll".into(),
                clearance_levels: 8,
                ribbon: supports
                    .iter()
                    .flat_map(|p| p.coord.within_radius(1))
                    .map(|p| TilePos::new(p, 0))
                    .collect(),
                supports,
            },
        );
    }
    for route in sites.routes.values() {
        for support in &route.ribbon {
            world.voxels.insert(*support, materials.stone);
        }
    }
    (session, world, geometry, materials, tuning)
}

fn id_for(session: &ArenaSession, role: ExpeditionRole) -> ActorId {
    session
        .actors
        .iter()
        .find(|a| a.expedition_role() == Some(role))
        .expect("role")
        .id
}

#[test]
fn troll_rally_requires_positive_player_damage_orders_survivors_once_and_dies_with_owner() {
    let (mut session, view, geometry, materials, tuning) = with_routes();
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    let troll = id_for(&session, ExpeditionRole::Troll);
    let dragon = id_for(&session, ExpeditionRole::Dragon);
    let baby = id_for(&session, ExpeditionRole::BabyGoblin);
    defeat(&mut session, baby, false);
    session.record_damage(dragon, troll, 1.0);
    session.record_damage(0, troll, 0.0);
    // Knockback attribution alone must neither issue the call nor fabricate XP.
    session.record_player_hit(0, troll);
    assert!(!session.expedition_rally_status().expect("status").triggered);
    session.record_damage(0, troll, 1.0);
    session.advance_rally(&view, geometry, &tuning);
    let status = session.expedition_rally_status().expect("status");
    assert!(status.triggered && status.active);
    assert_eq!((status.ordered_parties, status.ordered_actors), (14, 108));
    assert_eq!(status.living_ordered_actors, 108);
    assert_eq!(session.progress().expect("progress").total_xp, 0);
    assert!(session
        .encounter
        .runtime
        .iter()
        .filter(|p| p.snapshot.id < 14)
        .all(|p| p.knowledge.is_none() && p.snapshot.phase == PartyPhase::Dormant));
    let goals: Vec<_> = session
        .encounter
        .brains
        .iter()
        .map(|(id, b)| (*id, b.rally_goal))
        .collect();
    session.record_damage(0, troll, 7.0);
    assert_eq!(
        status,
        session.expedition_rally_status().expect("same orders")
    );
    assert_eq!(
        goals,
        session
            .encounter
            .brains
            .iter()
            .map(|(id, b)| (*id, b.rally_goal))
            .collect::<Vec<_>>()
    );
    // The call does not attribute later independent minion deaths to the player.
    let adult = id_for(&session, ExpeditionRole::Goblin);
    defeat(&mut session, adult, false);
    assert_eq!(session.progress().expect("progress").total_xp, 0);
    defeat(&mut session, troll, false);
    session.advance_rally(&view, geometry, &tuning);
    let ended = session.expedition_rally_status().expect("ended");
    assert!(ended.triggered && !ended.active);
    assert_eq!(ended.ordered_actors, 108);
    assert_eq!(ended.living_ordered_actors, 0);
    assert!(session
        .encounter
        .brains
        .values()
        .all(|b| b.rally_goal.is_none()));
    session.reset(1, &view, geometry);
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    let reset = session.expedition_rally_status().expect("reset");
    assert!(!reset.triggered && !reset.active);
    assert_eq!(reset.ordered_actors, 0);
}

#[test]
fn rally_travel_is_identical_when_an_unseen_player_moves_and_does_not_reveal_targets() {
    let (mut session, view, geometry, materials, tuning) = with_routes();
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    let troll = id_for(&session, ExpeditionRole::Troll);
    session.record_damage(0, troll, 1.0);
    session.advance_rally(&view, geometry, &tuning);
    for _ in 0..120 {
        session.tick += 1;
        session.advance_rally(&view, geometry, &tuning);
    }
    let actor = session
        .actors
        .iter()
        .find(|a| a.expedition_role() == Some(ExpeditionRole::BabyGoblin))
        .expect("baby")
        .clone();
    let party = session
        .encounter
        .runtime
        .iter()
        .find(|p| Some(p.snapshot.id) == actor.party)
        .expect("party");
    let goal = session
        .encounter
        .brains
        .get(&actor.id)
        .expect("brain")
        .rally_goal;
    assert!(goal.is_some_and(|point| point.distance(actor.feet) > 0.8));
    let mut a = brain::Brain::new(actor.id, actor.feet);
    let mut b = brain::Brain::new(actor.id, actor.feet);
    a.rally_goal = goal;
    b.rally_goal = goal;
    let mut moved = session.actors.clone();
    moved.first_mut().expect("player").feet += Vec3::X * 30.0;
    let (first, attack_a) = a.intent(
        &actor,
        party,
        &session.actors,
        &[],
        &[],
        &session.collision,
        &view,
        geometry,
        &tuning,
        10,
    );
    let (second, attack_b) = b.intent(
        &actor,
        party,
        &moved,
        &[],
        &[],
        &session.collision,
        &view,
        geometry,
        &tuning,
        10,
    );
    assert_eq!(first.direction, second.direction);
    assert_eq!(first.input.aim, second.input.aim);
    assert!(first.input.run && second.input.run);
    assert!(attack_a.is_none() && attack_b.is_none());
    assert!(party.knowledge.is_none());
    assert_eq!(session.progress().expect("progress").total_xp, 0);
}
