//! Actual authored terrain must support every gameplay-owned Grand roster.
use super::*;
use hex_arena::{ExpeditionRole, Species};
use std::collections::BTreeSet;

const PARTIES: &[(&str, Species, usize)] = &[
    ("grand_goblin_01", Species::Goblin, 8),
    ("grand_goblin_02", Species::Goblin, 12),
    ("grand_goblin_03", Species::Goblin, 18),
    ("grand_goblin_04", Species::Goblin, 20),
    ("grand_goblin_05", Species::Goblin, 22),
    ("grand_goblin_06", Species::Goblin, 24),
    ("grand_shaman_01", Species::Shaman, 3),
    ("grand_dragon_01", Species::Dragon, 3),
    ("grand_golem_01", Species::Golem, 3),
    ("grand_wisp_01", Species::Wisp, 10),
    ("grand_worm_01", Species::Worm, 1),
    ("grand_worm_02", Species::Worm, 1),
    ("grand_worm_03", Species::Worm, 1),
    ("grand_shadow_tunnel", Species::Shadow, 1),
];

fn expected_profile(leader: Species, slot: usize) -> (Species, Option<ExpeditionRole>) {
    match leader {
        Species::Goblin => (leader, Some(ExpeditionRole::Goblin)),
        Species::Shaman if slot == 2 => (Species::Goblin, Some(ExpeditionRole::Troll)),
        Species::Shaman => (leader, Some(ExpeditionRole::Shaman)),
        Species::Dragon => (leader, Some(ExpeditionRole::Dragon)),
        Species::Golem => (leader, Some(ExpeditionRole::PlainGolem)),
        Species::Wisp => (leader, Some(ExpeditionRole::PlainWisp)),
        Species::Shadow => (leader, Some(ExpeditionRole::MountainShadow)),
        Species::Worm | Species::Human => (leader, None),
    }
}

#[test]
#[ignore = "requires explicit actual Grand package and isolated HEX_GAME_DATA_DIR with grand-verification-only marker"]
fn actual_grand_all_authored_parties_admit_and_checkpoint() {
    let data =
        PathBuf::from(std::env::var_os("HEX_GAME_DATA_DIR").expect("isolated data directory"));
    assert!(
        data.join("grand-verification-only").is_file(),
        "refuse real user data"
    );
    let package =
        PathBuf::from(std::env::var_os("HEX_GRAND_WORLD").expect("explicit actual package"));
    assert!(!data.join("admission.json").exists(), "use fresh output");
    let mut app = fixture();
    let beach = app
        .world()
        .resource::<ArenaTerrainView>()
        .spawns
        .first()
        .copied()
        .expect("authored beach");
    relocate(&mut app, beach);
    step(&mut app, ActorIntent::default());
    assert!(
        app.world().resource::<ArenaSession>().tick > 0,
        "the real beach must admit the initial player before encounter visits"
    );
    let geometry = *app.world().resource::<ArenaVoxelGeometry>();
    let sites = app
        .world()
        .resource::<ArenaTerrainView>()
        .expedition
        .clone()
        .expect("authored expedition sites");
    assert_eq!(sites.encounters.len(), PARTIES.len());
    let mut observed_ids = BTreeSet::new();
    let mut samples = Vec::new();
    for (ordinal, &(name, leader, count)) in PARTIES.iter().enumerate() {
        let deployment = &sites
            .encounters
            .get(name)
            .expect("exact authored encounter")
            .deployment;
        let home = deployment.preferred;
        // Only the real human and its streaming interest are relocated. Enemy
        // bodies, HP, IDs, terrain and admission rules remain production-owned.
        relocate(&mut app, home.coord.to_world(geometry.top(home) + 0.02));
        pump_until(&mut app, name, |world| {
            let terrain = world.resource::<ArenaTerrainView>();
            let residency = terrain.residency.as_ref().expect("finite residency");
            deployment
                .surfaces
                .iter()
                .all(|pos| residency.at(pos.coord, geometry) == ArenaAvailability::Ready)
        });
        let party = u16::try_from(ordinal).expect("fourteen parties");
        let expected_ids: BTreeSet<_> = (0..count)
            .map(|slot| u32::try_from(ordinal * 32 + slot + 1).expect("bounded authored ID"))
            .collect();
        let initial_tick = app.world().resource::<ArenaSession>().tick;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            step(&mut app, ActorIntent::default());
            let session = app.world().resource::<ArenaSession>();
            let actual_ids: BTreeSet<_> = session
                .actors
                .iter()
                .filter(|actor| actor.party == Some(party))
                .map(|actor| actor.id)
                .collect();
            if actual_ids == expected_ids && session.tick > initial_tick {
                break;
            }
            assert!(
                session.tick.saturating_sub(initial_tick) < 12 && Instant::now() < deadline,
                "{name} failed ordinary admission: expected={expected_ids:?}, actual={actual_ids:?}, tick={}, notice={}, preferred_solid={:?}",
                session.tick,
                session.notice,
                app.world().resource::<ArenaTerrainView>().solid_at(home),
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        let session = app.world().resource::<ArenaSession>();
        assert!(session
            .actors
            .iter()
            .find(|actor| actor.id == 0)
            .is_some_and(|player| player.hp > 0.0));
        let members: Vec<_> = session
            .actors
            .iter()
            .filter(|actor| actor.party == Some(party))
            .collect();
        assert_eq!(members.len(), count, "{name}: exact roster size");
        for (slot, &id) in expected_ids.iter().enumerate() {
            let actor = members
                .iter()
                .find(|actor| actor.id == id)
                .expect("complete stable roster");
            assert_eq!(
                (actor.species, actor.expedition_role()),
                expected_profile(leader, slot),
                "{name}: actor {id} profile"
            );
            assert!(
                actor.hp > 0.0 && actor.feet.is_finite(),
                "{name}: actor {id}"
            );
            assert!(observed_ids.insert(id), "duplicate authored actor {id}");
        }
        samples.push(serde_json::json!({
            "site": name, "party": party, "actor_ids": expected_ids,
            "members": count, "simulation_ticks": session.tick - initial_tick,
            "active_actors": session.actors.len(),
            "registered_enemies": session.expedition_progress().expect("Grand XP roster").enemies_total,
        }));
        println!("GRAND_ADMISSION_SITE {name} members={count}");
    }
    assert_eq!(observed_ids.len(), 127);
    let session = app.world().resource::<ArenaSession>();
    assert_eq!(
        session
            .expedition_progress()
            .expect("Grand XP roster")
            .enemies_total,
        127,
        "departing a site must retain its dormant logical roster"
    );
    assert!(
        session.actors.len() < 128,
        "distant parties must become dormant"
    );
    settle(app.world_mut()).expect("completed terrain boundary");
    let identity = gameplay_identity(app.world().resource::<State>()).expect("content identity");
    let bytes = app
        .world()
        .resource::<ArenaSession>()
        .encode_grand_checkpoint(&identity)
        .expect("all admitted active and dormant records validate");
    let restored = ArenaSession::decode_grand_checkpoint(
        &bytes,
        &identity,
        app.world().resource::<ArenaTerrainView>(),
        geometry,
        app.world().resource::<ArenaReset>().generation,
    )
    .expect("all-party checkpoint restores");
    assert_eq!(
        restored
            .encode_grand_checkpoint(&identity)
            .expect("restored codec"),
        bytes,
        "complete active/dormant gameplay state survives the codec"
    );
    let map = app.world().resource::<StreamedArena>();
    let receipt = serde_json::json!({
        "kind": "grand-authored-admission-v1", "status": "PASS",
        "scope": "Actual package; real human relocations and ordinary full-party admission with AI decisions disabled. No enemy/HP/terrain injection. Exact actor profiles, stable identities, dormancy and gameplay checkpoint round trip; no combat difficulty or native-control claim.",
        "package": package, "manifest_fingerprint": map.runtime.manifest().fingerprint,
        "source_fingerprint": map.runtime.manifest().source_fingerprint,
        "parties": samples, "unique_enemies": observed_ids.len(),
        "checkpoint_bytes": bytes.len(), "active_actors_at_checkpoint": restored.actors.len(),
    });
    std::fs::write(
        data.join("admission.json"),
        serde_json::to_vec_pretty(&receipt).expect("admission receipt"),
    )
    .expect("write admission receipt");
    println!(
        "GRAND_ADMISSION_PASS {}",
        data.join("admission.json").display()
    );
}
