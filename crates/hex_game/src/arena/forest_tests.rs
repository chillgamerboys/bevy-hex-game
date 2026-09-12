//! Explicit full-world contracts; no native window, renderer, or FPS measurement.

use super::*;
use hex_arena::{ActorId, PartyPhase, Species};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

fn tick(app: &mut App) {
    app.world_mut().run_schedule(ArenaTick);
}

fn roster(app: &App) -> Vec<(ActorId, Species, Vec3)> {
    let view = app.world().resource::<ArenaTerrainView>();
    let geometry = *app.world().resource::<ArenaVoxelGeometry>();
    let session = app.world().resource::<ArenaSession>();
    assert_eq!(view.selection.map, ArenaMap::ForestMassif);
    assert_eq!(view.columns.len(), 105_469);
    assert_eq!(session.actors.len(), 26, "{}", session.notice);
    assert_eq!(session.parties().len(), 8);
    assert_eq!(
        session
            .parties()
            .iter()
            .map(|party| party.living)
            .collect::<Vec<_>>(),
        [2, 3, 4, 6, 7, 1, 1, 1]
    );
    let mut ids = BTreeSet::new();
    for actor in &session.actors {
        assert!(ids.insert(actor.id));
        assert!(actor.hp > 0.0);
        assert!(
            session.actor_pose_valid(actor.id, view, geometry),
            "{} {:?}",
            actor.id,
            actor.feet
        );
    }
    assert!(!session.is_finished());
    session
        .actors
        .iter()
        .map(|actor| (actor.id, actor.species, actor.feet))
        .collect()
}

fn visit(app: &mut App, representative: ActorId, home: Vec3, previous: Option<Vec3>) -> Vec3 {
    let (feet, aim) = {
        let session = app.world().resource::<ArenaSession>();
        let feet = encounter::stress_target_pose(
            session,
            0,
            representative,
            previous,
            home,
            app.world().resource::<ArenaTerrainView>(),
            *app.world().resource::<ArenaVoxelGeometry>(),
        )
        .expect("real camp must have a supported dry visible player visit pose");
        assert!(feet.distance(home) <= 10.0);
        let target = session
            .actors
            .iter()
            .find(|actor| actor.id == representative)
            .expect("representative");
        let human = session.actors.first().expect("human");
        (
            feet,
            (target.center() - (feet + human.eye() - human.feet)).normalize_or(Vec3::NEG_Z),
        )
    };
    {
        let mut session = app.world_mut().resource_mut::<ArenaSession>();
        let human = session.actors.first_mut().expect("human");
        human.feet = feet;
        human.previous_feet = feet;
        human.aim = aim;
    }
    app.world_mut().resource_mut::<ArenaInput>().human = ActorIntent { aim, ..default() };
    feet
}

fn distribution(mut samples: Vec<f64>) -> serde_json::Value {
    samples.sort_by(f64::total_cmp);
    let percentile = |percent: usize| {
        samples
            .get(samples.len().saturating_sub(1) * percent / 100)
            .copied()
    };
    serde_json::json!({
        "ticks": samples.len(), "p50_ms": percentile(50), "p95_ms": percentile(95),
        "p99_ms": percentile(99), "max_ms": samples.last().copied(),
    })
}

/// Geometry/roster proxy checkpoint only. Rewards, populated forest performance
/// and final visual/native acceptance have separate, still-pending gates.
#[test]
#[ignore = "requires HEX_FOREST_WORLD pointing at the compiled expedition and companion"]
fn authored_expedition_proxy_has_115_supported_actors_and_resets() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(ArenaSelection {
            map: ArenaMap::ForestMassif,
            ..default()
        })
        .add_plugins((hex_map::arena::plugin, hex_arena::plugin));
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
    let setup = Instant::now();
    app.update();
    assert!(
        app.world().contains_resource::<ArenaTerrainView>(),
        "expedition world/companion rejected"
    );
    tick(&mut app);
    let setup_ms = setup.elapsed().as_secs_f64() * 1000.0;
    let inspect = |app: &App| {
        let world = app.world().resource::<ArenaTerrainView>();
        let geometry = *app.world().resource::<ArenaVoxelGeometry>();
        let session = app.world().resource::<ArenaSession>();
        assert!(
            world.expedition.is_some(),
            "fixture needs the expedition package"
        );
        assert_eq!(world.columns.len(), 105_469);
        assert_eq!(session.actors.len(), 115, "{}", session.notice);
        assert_eq!(session.parties().len(), 19);
        assert_eq!(
            session
                .parties()
                .iter()
                .map(|party| party.living)
                .collect::<Vec<_>>(),
            [3, 3, 3, 3, 3, 5, 5, 5, 9, 9, 11, 14, 16, 20, 1, 1, 1, 1, 1]
        );
        let mut roles = BTreeMap::<String, usize>::new();
        for actor in &session.actors {
            assert!(
                session.actor_pose_valid(actor.id, world, geometry),
                "unsupported {} at {:?}",
                actor.id,
                actor.feet
            );
            if let Some(role) = actor.expedition_role() {
                *roles.entry(format!("{role:?}")).or_default() += 1;
            }
        }
        assert_eq!(roles.get("BabyGoblin"), Some(&15));
        assert_eq!(roles.get("Goblin"), Some(&92));
        assert_eq!(roles.get("Shaman"), Some(&2));
        assert_eq!(roles.get("Troll"), Some(&1));
        assert_eq!(roles.get("Dragon"), Some(&3));
        assert_eq!(roles.get("MountainShadow"), Some(&1));
        assert!(!session.is_finished());
        session
            .actors
            .iter()
            .map(|actor| (actor.id, actor.species, actor.feet))
            .collect::<Vec<_>>()
    };
    let initial = inspect(&app);
    let mut samples = Vec::new();
    for _ in 0..120 {
        let start = Instant::now();
        tick(&mut app);
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let reset = Instant::now();
    app.world_mut().resource_mut::<ArenaReset>().generation += 1;
    tick(&mut app);
    assert_eq!(inspect(&app), initial);
    println!(
        "EXPEDITION_PROXY_RECEIPT {}",
        serde_json::json!({
            "actors": 115, "parties": 19, "setup_ms": setup_ms,
            "reset_ms": reset.elapsed().as_secs_f64() * 1000.0,
            "bridge_start_ticks": distribution(samples),
            "scope": "terrain-only proxy spawn/reset and idle simulation CPU; no forest/rally/renderer/FPS claim"
        })
    );
}

/// Load the operator's compiled package (HEX_FOREST_WORLD or the default compiled
/// directory), prove real spawn/reset composition, then measure a bounded synthetic
/// workload. Only the human receives extra HP and moves among validated camp poses;
/// enemy bodies, stats, leashes, terrain and normal decision rules stay authored.
#[test]
#[ignore = "requires compiled Forest V4 package; explicit full-map CPU timing fixture"]
fn authored_forest_spawn_reset_and_three_second_active_tick_profile() {
    let setup_start = Instant::now();
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(ArenaSelection {
            map: ArenaMap::ForestMassif,
            ..default()
        })
        .add_plugins((hex_map::arena::plugin, hex_arena::plugin));
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
    app.update();
    assert!(
        app.world().contains_resource::<ArenaTerrainView>(),
        "compile the Forest package before running this explicit test"
    );
    tick(&mut app);
    let setup_ms = setup_start.elapsed().as_secs_f64() * 1000.0;
    let initial = roster(&app);
    let original_progress = app
        .world()
        .resource::<ArenaSession>()
        .progress()
        .expect("forest progress");
    let (goblin, home) = {
        let session = app.world().resource::<ArenaSession>();
        let goblin = session
            .actors
            .iter()
            .find(|actor| actor.species == Species::Goblin)
            .expect("goblin");
        let home = session
            .parties()
            .iter()
            .find(|party| Some(party.id) == goblin.party)
            .expect("party")
            .home;
        (goblin.id, home)
    };
    visit(&mut app, goblin, home, None);
    // The reduced victim HP is an explicit kill-credit fixture. XP must still be
    // caused by the ordinary input -> contact projectile -> damage/death path.
    app.world_mut()
        .resource_mut::<ArenaSession>()
        .actors
        .iter_mut()
        .find(|actor| actor.id == goblin)
        .expect("goblin")
        .hp = 1.0;
    {
        let mut input = app.world_mut().resource_mut::<ArenaInput>();
        input.human.selected = Some(Spell::Fireball);
        input.human.cast_pressed = true;
        input.human.cast_released = true;
    }
    for _ in 0..120 {
        tick(&mut app);
        if app
            .world()
            .resource::<ArenaSession>()
            .progress()
            .expect("progress")
            .total_xp
            > 0
        {
            break;
        }
    }
    assert_eq!(
        app.world()
            .resource::<ArenaSession>()
            .progress()
            .expect("progress")
            .total_xp,
        1,
        "real Fireball must grant kill credit before reset"
    );
    app.world_mut().resource_mut::<ArenaInput>().human = ActorIntent::default();
    app.world_mut().resource_mut::<ArenaReset>().generation += 1;
    let reset_start = Instant::now();
    tick(&mut app);
    let reset_ms = reset_start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(roster(&app), initial);
    assert_eq!(
        app.world().resource::<ArenaSession>().progress(),
        Some(original_progress)
    );

    let representatives = {
        let mut session = app.world_mut().resource_mut::<ArenaSession>();
        session.bot_enabled = true;
        let human = session.actors.first_mut().expect("human");
        human.max_hp = 100_000.0;
        human.hp = human.max_hp;
        session
            .parties()
            .iter()
            .map(|party| {
                let actor = session
                    .actors
                    .iter()
                    .find(|actor| actor.party == Some(party.id))
                    .expect("representative");
                (actor.id, party.home)
            })
            .collect::<Vec<_>>()
    };
    let mut anchors = BTreeMap::new();
    let mut samples = Vec::new();
    let mut all_active = Vec::new();
    let mut peak_active = 0;
    let mut peak_projectiles = 0;
    let mut publication_ticks = 0;
    // Twelve ticks guarantee one complete staggered party sight cycle per visit.
    // The first complete round of eight visits is the explicit 0.8s warm-up.
    for step in 0..456 {
        let slot = (step / 12) % representatives.len();
        let (actor, home) = *representatives.get(slot).expect("bounded party index");
        let feet = visit(&mut app, actor, home, anchors.get(&actor).copied());
        anchors.insert(actor, feet);
        let revision = app.world().resource::<ArenaTerrainView>().revision;
        let started = Instant::now();
        tick(&mut app);
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        let session = app.world().resource::<ArenaSession>();
        assert!(!session.is_finished());
        assert_eq!(session.encounter_summary().living_enemies, 25);
        let active = session
            .parties()
            .iter()
            .filter(|party| party.phase == PartyPhase::Active)
            .count();
        peak_active = peak_active.max(active);
        peak_projectiles = peak_projectiles.max(session.projectiles.len());
        if step >= 96 {
            samples.push(elapsed_ms);
            if active == 8 {
                all_active.push(elapsed_ms);
            }
            publication_ticks +=
                usize::from(app.world().resource::<ArenaTerrainView>().revision != revision);
        }
    }
    let all_active_ticks = all_active.len();
    let receipt = serde_json::json!({
        "fixture": "authored-forest-26-actors-eight-real-camps",
        "measurement": "ArenaTick CPU wall time only; no renderer, GPU or FPS claim",
        "synthetic_changes": "one-HP Goblin for reset XP check; reset restores it; extra player HP and validated camp visits during timing; enemy placements and tuning remain authored",
        "setup_ms": setup_ms, "reset_ms": reset_ms,
        "warmup_ticks": 96, "measured_simulation_seconds": 3,
        "all_ticks": distribution(samples), "all_eight_parties_active": distribution(all_active),
        "peak_active_parties": peak_active, "peak_projectiles": peak_projectiles,
        "terrain_publication_ticks": publication_ticks,
        "final_parties": app.world().resource::<ArenaSession>().parties().iter().map(|party| (party.id, format!("{:?}", party.phase), party.living)).collect::<Vec<_>>(),
    });
    println!("FOREST_COMBINED_RECEIPT {receipt}");
    assert_eq!(peak_active, 8, "all real camps must activate");
    assert!(all_active_ticks >= 240, "at least two of the three measured seconds need all eight parties active; got {all_active_ticks}/360 ticks");
}

/// Explicit CPU workload on the complete expedition. Validated player visits and
/// extra player HP keep the measurement bounded; every enemy keeps its authored
/// body, stats, position, perception and ordinary combat behavior.
#[test]
#[ignore = "requires HEX_FOREST_WORLD pointing at the final populated expedition"]
fn authored_expedition_largest_camp_and_full_rally_tick_profile() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(ArenaSelection {
            map: ArenaMap::ForestMassif,
            ..default()
        })
        .add_plugins((hex_map::arena::plugin, hex_arena::plugin));
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
    app.update();
    tick(&mut app);
    let inspect = |app: &App| {
        let session = app.world().resource::<ArenaSession>();
        let world = app.world().resource::<ArenaTerrainView>();
        let geometry = *app.world().resource::<ArenaVoxelGeometry>();
        assert!(world.expedition.is_some());
        assert_eq!(session.actors.len(), 115, "{}", session.notice);
        assert_eq!(session.parties().len(), 19);
        for actor in &session.actors {
            assert!(session.actor_pose_valid(actor.id, world, geometry));
        }
        session
            .actors
            .iter()
            .map(|actor| (actor.id, actor.feet))
            .collect::<BTreeMap<_, _>>()
    };
    let initial = inspect(&app);
    let representative = |app: &App, largest: bool| {
        let session = app.world().resource::<ArenaSession>();
        let party = if largest {
            session
                .parties()
                .iter()
                .find(|party| party.living == 20)
                .expect("twenty-Goblin camp")
        } else {
            let troll = session
                .actors
                .iter()
                .find(|a| a.expedition_role() == Some(hex_arena::ExpeditionRole::Troll))
                .expect("Troll");
            session
                .parties()
                .iter()
                .find(|party| Some(party.id) == troll.party)
                .expect("Troll party")
        };
        let actor = session
            .actors
            .iter()
            .find(|actor| actor.party == Some(party.id))
            .expect("party representative");
        (actor.id, party.id, party.home)
    };
    let protect_player = |app: &mut App| {
        let mut session = app.world_mut().resource_mut::<ArenaSession>();
        let human = session.actors.first_mut().expect("player");
        human.max_hp = 100_000.0;
        human.hp = human.max_hp;
    };
    let (goblin, party_id, home) = representative(&app, true);
    protect_player(&mut app);
    visit(&mut app, goblin, home, None);
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = true;
    let mut camp_samples = Vec::new();
    let mut camp_activated = false;
    let mut peak_projectiles = 0;
    for step in 0..720 {
        let start = Instant::now();
        tick(&mut app);
        if step >= 120 {
            camp_samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        let session = app.world().resource::<ArenaSession>();
        assert!(!session.is_finished());
        camp_activated |= session
            .parties()
            .iter()
            .any(|party| party.id == party_id && party.phase == PartyPhase::Active);
        peak_projectiles = peak_projectiles.max(session.projectiles.len());
    }
    assert!(camp_activated, "largest authored camp must see the visitor");

    app.world_mut().resource_mut::<ArenaReset>().generation += 1;
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
    tick(&mut app);
    assert_eq!(inspect(&app), initial);
    let (troll, _, home) = representative(&app, false);
    protect_player(&mut app);
    visit(&mut app, troll, home, None);
    {
        let mut input = app.world_mut().resource_mut::<ArenaInput>();
        input.human.selected = Some(Spell::Fireball);
        input.human.cast_pressed = true;
        input.human.cast_released = true;
    }
    for _ in 0..120 {
        tick(&mut app);
        if app
            .world()
            .resource::<ArenaSession>()
            .expedition_rally_status()
            .expect("rally snapshot")
            .triggered
        {
            break;
        }
    }
    let issued = app
        .world()
        .resource::<ArenaSession>()
        .expedition_rally_status()
        .expect("issued orders");
    assert!(
        issued.triggered && issued.active,
        "ordinary contact shot must damage Troll"
    );
    assert_eq!((issued.ordered_parties, issued.ordered_actors), (14, 109));
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = true;
    let mut rally_samples = Vec::new();
    let mut publication_ticks = 0;
    let mut peak_active = 0;
    let mut moved_parties = BTreeSet::new();
    for step in 0..2520 {
        let revision = app.world().resource::<ArenaTerrainView>().revision;
        let start = Instant::now();
        tick(&mut app);
        if step >= 120 {
            rally_samples.push(start.elapsed().as_secs_f64() * 1000.0);
            publication_ticks +=
                usize::from(app.world().resource::<ArenaTerrainView>().revision != revision);
        }
        let session = app.world().resource::<ArenaSession>();
        assert!(!session.is_finished());
        peak_projectiles = peak_projectiles.max(session.projectiles.len());
        peak_active = peak_active.max(
            session
                .parties()
                .iter()
                .filter(|party| party.phase == PartyPhase::Active)
                .count(),
        );
        for actor in session.actors.iter().filter(|actor| {
            matches!(
                actor.expedition_role(),
                Some(
                    hex_arena::ExpeditionRole::BabyGoblin
                        | hex_arena::ExpeditionRole::Goblin
                        | hex_arena::ExpeditionRole::Shaman
                )
            )
        }) {
            if initial
                .get(&actor.id)
                .is_some_and(|feet| actor.feet.distance(*feet) > 1.0)
            {
                if let Some(party) = actor.party {
                    moved_parties.insert(party);
                }
            }
        }
    }
    let session = app.world().resource::<ArenaSession>();
    println!(
        "EXPEDITION_ACTIVE_RECEIPT {}",
        serde_json::json!({
            "actors_at_reset":115,"parties":19,
            "scope":"ArenaTick CPU wall time; no renderer, GPU, FPS or native traversal claim",
            "synthetic_changes":"extra player HP and two validated player visits; ordinary single contact shot triggers Troll rally; authored enemy stats and placements",
            "largest_camp":distribution(camp_samples),"rally":distribution(rally_samples),
            "issued_rally":issued,"final_rally":session.expedition_rally_status(),
            "moved_forest_parties":moved_parties.len(),"peak_active_parties":peak_active,
            "peak_projectiles":peak_projectiles,"terrain_publication_ticks":publication_ticks,
            "remaining_enemies":session.encounter_summary().living_enemies,
        })
    );
    assert_eq!(
        moved_parties.len(),
        14,
        "every ordered forest party must begin travel"
    );
}
