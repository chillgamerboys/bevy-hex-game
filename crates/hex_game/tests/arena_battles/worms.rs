//! Worm admission and world mutation through the actual authored map publishers.
//! These tests run after the coordinator removes the guarded readiness refusal.

use super::*;
use bevy::ecs::message::MessageCursor;
use hex_arena::{
    ArenaControl, AttackPhase, CreatureAbility, PartyPhase, ProjectileAppearance, Species,
    WormPhase,
};
use hex_core::arena::{ArenaBurrowOutcome, ArenaBurrowRequest, ArenaBurrowResult, ArenaMaterials};
use hex_core::{DamagedVoxels, HexCoord, TilePos};
use std::collections::{BTreeMap, BTreeSet};

const EPSILON: f32 = 0.001;

/// Initial deployment is above ground. Native-aligned centers establish one
/// exact hex footprint per component; ordinary clearance is valid only here.
fn assert_initial_worms(fixture: &App, segments: usize) {
    let session = fixture.world().resource::<ArenaSession>();
    let terrain = fixture.world().resource::<ArenaTerrainView>();
    let geometry = *fixture.world().resource::<ArenaVoxelGeometry>();
    let regions = terrain
        .elongated_deployment
        .as_ref()
        .expect("elongated regions");
    for actor in &session.actors {
        assert!(
            session.actor_pose_valid(actor.id, terrain, geometry),
            "initial above-ground actor{} {:?}",
            actor.id,
            actor.feet
        );
        if actor.species != Species::Worm {
            continue;
        }
        let setup = session.accepted_battle_setup();
        let side = if setup.control == ArenaControl::Spectator {
            setup
                .rosters
                .iter()
                .position(|roster| roster.team == actor.team)
                .expect("side comes from accepted roster order, not allegiance ID")
        } else {
            // The Fort player recipe explicitly uses the second elongated region.
            1
        };
        let region = regions.get(side).expect("authored side");
        let parts: Vec<_> = actor.body_hex_prisms().collect();
        assert_eq!(parts.len(), segments);
        assert_eq!(
            actor.body_rotation().to_array().map(f32::to_bits),
            Quat::IDENTITY.to_array().map(f32::to_bits)
        );
        let mut cells = BTreeSet::new();
        for part in parts {
            assert!((part.height - geometry.level_height).abs() < EPSILON);
            let base = actor.feet + part.offset;
            let coord = HexCoord::from_world(base);
            assert!(
                coord.to_world(base.y).distance(base) < EPSILON,
                "native component center"
            );
            let supporting = TilePos::new(coord, region.preferred.level);
            assert!(region.surfaces.contains(&supporting));
            assert!(terrain.voxels.contains_key(&supporting));
            assert!(base.y + EPSILON >= geometry.top(supporting));
            cells.insert(supporting);
        }
        assert_eq!(
            cells.len(),
            segments,
            "tail segments are not compressed into fewer cells"
        );
        assert_eq!(actor.worm().expect("physical state").head_index, 0);
    }
}

fn actor_snapshot(fixture: &App) -> serde_json::Value {
    serde_json::Value::Array(
        fixture
            .world()
            .resource::<ArenaSession>()
            .actors
            .iter()
            .map(|actor| {
                serde_json::json!({
                    "id":actor.id,"team":actor.team,"species":actor.species,
                    "feet":actor.feet.to_array(),"aim":actor.aim.to_array(),"hp":actor.hp,
                    "dimensions":actor.body_dimensions().to_array(),"worm":actor.worm(),
                    "prisms":actor.body_hex_prisms().map(|part|serde_json::json!({
                        "offset":part.offset.to_array(),"height":part.height
                    })).collect::<Vec<_>>()
                })
            })
            .collect(),
    )
}

#[test]
fn four_segment_worms_admit_on_both_real_maps_in_both_orders_and_against_each_other() {
    for map in [ArenaMap::Duel, ArenaMap::Fort] {
        for (left, right) in [
            (BattlePreset::Worm, BattlePreset::Shadow),
            (BattlePreset::Shadow, BattlePreset::Worm),
            (BattlePreset::Worm, BattlePreset::Worm),
        ] {
            let fixture = app(map, ArenaBattleSetup::spectator(left, right, 41));
            assert!(
                battle(&fixture).result.is_none(),
                "{map:?} {left:?}/{right:?}: {:?}",
                battle(&fixture).result
            );
            assert_eq!(fixture.world().resource::<ArenaSession>().actors.len(), 2);
            assert_eq!(
                fixture.world().resource::<ArenaSession>().human_actor_id(),
                None
            );
            assert_initial_worms(&fixture, 4);
        }
    }
}

#[test]
fn six_segment_real_map_attempts_are_complete_or_explicitly_refuse_finite_space() {
    for map in [ArenaMap::Duel, ArenaMap::Fort] {
        for (left, right) in [
            (BattlePreset::Worm, BattlePreset::Shadow),
            (BattlePreset::Shadow, BattlePreset::Worm),
            (BattlePreset::Worm, BattlePreset::Worm),
        ] {
            let mut tuning = authored_tuning();
            tuning.encounters.worm_segments = 6;
            tuning
                .validate()
                .expect("supported six-segment configuration");
            let fixture = configured_app(map, ArenaBattleSetup::spectator(left, right, 42), tuning);
            match battle(&fixture).result {
                None => assert_initial_worms(&fixture, 6),
                Some(BattleResult::InvalidSetup(reason)) => {
                    assert!(
                        reason.starts_with("No complete dry deployment"),
                        "explicit space refusal, not readiness or a schema failure: {reason}"
                    );
                    assert!(
                        fixture.world().resource::<ArenaSession>().actors.is_empty(),
                        "no partial roster"
                    );
                }
                result => panic!("unexpected setup outcome {map:?}/{left:?}/{right:?}: {result:?}"),
            }
        }
    }
}

#[test]
fn fort_player_worm_starts_outside_activation_and_restart_restores_body_and_original_earth() {
    let setup = ArenaBattleSetup {
        player_recipe: Some(BattlePreset::Worm),
        ..Default::default()
    };
    let mut fixture = app(ArenaMap::Fort, setup.clone());
    let session = fixture.world().resource::<ArenaSession>();
    assert!(!session.is_finished());
    assert_eq!(session.accepted_battle_setup(), &setup);
    assert_eq!(session.human_actor_id(), Some(0));
    assert_eq!(session.actors.len(), 2);
    assert_initial_worms(&fixture, 4);
    let human = session.actors.iter().find(|a| a.id == 0).expect("human");
    let worm = session
        .actors
        .iter()
        .find(|a| a.species == Species::Worm)
        .expect("Worm");
    assert!(
        human.eye().distance(worm.eye())
            > fixture
                .world()
                .resource::<ArenaTuning>()
                .encounters
                .activation_radius
    );
    assert!(session
        .parties()
        .iter()
        .all(|party| party.phase == PartyPhase::Dormant));
    let original_actors = actor_snapshot(&fixture);
    let original_earth = fixture
        .world()
        .resource::<ArenaTerrainView>()
        .voxels
        .clone();
    let original_health = fixture.world().resource::<DamagedVoxels>().clone();
    let mut requests = MessageCursor::<ArenaBurrowRequest>::default();
    let mut old_request = None;
    let mut changed = false;
    for _ in 0..1200 {
        fixture.world_mut().run_schedule(ArenaTick);
        for request in requests.read(fixture.world().resource::<Messages<ArenaBurrowRequest>>()) {
            old_request = Some(request.clone());
        }
        assert!(!fixture.world().resource::<ArenaSession>().is_finished());
        if fixture.world().resource::<ArenaTerrainView>().voxels != original_earth {
            changed = true;
            break;
        }
    }
    assert!(
        changed,
        "the normal dormant look/dive cycle must reach world conversion"
    );
    let old_request = old_request.expect("conversion came from an actual Worm request");
    fixture.world_mut().resource_mut::<ArenaReset>().generation += 1;
    // Replay the actual previous-generation request into the reset boundary.
    // It must neither recolor the restored map nor apply an old body endpoint.
    fixture.world_mut().write_message(old_request);
    fixture.world_mut().run_schedule(ArenaTick);
    assert_eq!(
        fixture.world().resource::<ArenaTerrainView>().voxels,
        original_earth
    );
    assert_eq!(
        *fixture.world().resource::<DamagedVoxels>(),
        original_health
    );
    assert_eq!(actor_snapshot(&fixture), original_actors);
    assert_eq!(fixture.world().resource::<ArenaSession>().tick, 1);
    assert_eq!(
        fixture
            .world()
            .resource::<ArenaSession>()
            .accepted_battle_setup(),
        &setup
    );
    assert!(fixture
        .world()
        .resource::<ArenaSession>()
        .projectiles
        .is_empty());
    *fixture.world_mut().resource_mut::<ArenaBattleSetup>() = ArenaBattleSetup::default();
    fixture.world_mut().resource_mut::<ArenaSelection>().map = ArenaMap::Duel;
    fixture.world_mut().resource_mut::<ArenaReset>().generation += 1;
    fixture.world_mut().run_schedule(ArenaTick);
    let session = fixture.world().resource::<ArenaSession>();
    assert_eq!(session.actors.len(), 2);
    assert!(session
        .actors
        .iter()
        .all(|a| matches!(a.species, Species::Human | Species::Shadow)));
    assert!(session.battle_summary().is_none());
}

#[test]
fn actual_duel_worms_naturally_convert_earth_expose_the_head_and_deal_boulder_damage() {
    let mut setup = ArenaBattleSetup::spectator(BattlePreset::Worm, BattlePreset::Worm, 43);
    setup.tick_limit = Some(2400);
    let mut fixture = app(ArenaMap::Duel, setup);
    assert!(
        battle(&fixture).result.is_none(),
        "Worm pair must admit before runtime proof"
    );
    let geometry = *fixture.world().resource::<ArenaVoxelGeometry>();
    let dirt = fixture.world().resource::<ArenaMaterials>().dirt;
    let mut request_cursor = MessageCursor::<ArenaBurrowRequest>::default();
    let mut outcome_cursor = MessageCursor::<ArenaBurrowOutcome>::default();
    let mut requests = BTreeMap::new();
    let mut converted = BTreeSet::new();
    let mut saw_travel = false;
    let mut saw_windup = false;
    let mut saw_boulder = false;
    let mut damaged = false;
    for _ in 0..2400 {
        if fixture.world().resource::<ArenaSession>().is_finished() {
            break;
        }
        fixture.world_mut().run_schedule(ArenaTick);
        for request in
            request_cursor.read(fixture.world().resource::<Messages<ArenaBurrowRequest>>())
        {
            assert!(request.structural_rejection().is_none());
            requests.insert(
                (request.generation, request.actor, request.sequence),
                request.volume.clone(),
            );
        }
        for outcome in
            outcome_cursor.read(fixture.world().resource::<Messages<ArenaBurrowOutcome>>())
        {
            let volume = requests
                .get(&(outcome.generation, outcome.actor, outcome.sequence))
                .expect("world acknowledgement matches real emitted request");
            if let ArenaBurrowResult::Accepted { changed } = &outcome.result {
                let world = fixture.world().resource::<ArenaTerrainView>();
                for cell in changed {
                    assert!(volume.contains(&cell.position));
                    assert!(cell.health_after.remaining <= cell.health_before.remaining);
                    assert_eq!(world.voxels.get(&cell.position), Some(&dirt));
                    let published = fixture
                        .world()
                        .resource::<DamagedVoxels>()
                        .get(cell.position);
                    if cell.health_after.remaining < cell.health_after.maximum {
                        assert_eq!(published, Some(cell.health_after));
                    } else {
                        assert!(published.is_none());
                    }
                    converted.insert(cell.position);
                }
            }
        }
        let session = fixture.world().resource::<ArenaSession>();
        for actor in session.actors.iter().filter(|actor| actor.hp > 0.0) {
            let state = actor.worm().expect("Worm physical projection");
            saw_travel |= state.phase == WormPhase::Travel;
            if let Some(attack) = actor
                .attack_state()
                .filter(|a| a.kind == CreatureAbility::WormBoulder)
            {
                assert!(attack.origin.distance(actor.eye()) < EPSILON);
                if attack.phase == AttackPhase::Windup {
                    assert!(
                        state.exposed && state.head_clearance + EPSILON >= geometry.level_height
                    );
                    saw_windup = true;
                }
            }
            // A buried pose deliberately occupies admitted solid dirt; ordinary
            // actor_pose_valid is not a truthful burrow-volume oracle.
        }
        for shot in session
            .projectiles
            .iter()
            .filter(|shot| shot.source_ability() == Some(CreatureAbility::WormBoulder))
        {
            assert_eq!(shot.appearance(), ProjectileAppearance::Boulder);
            if shot.age < EPSILON {
                let caster = session
                    .actors
                    .iter()
                    .find(|actor| actor.id == shot.owner)
                    .expect("living caster at release");
                let state = caster.worm().expect("Worm release geometry");
                assert!(state.exposed && state.head_clearance + EPSILON >= geometry.level_height);
            }
            saw_boulder = true;
        }
        damaged |= session
            .encounter_stats()
            .iter()
            .any(|stats| stats.species == Species::Worm && stats.combat.damage_dealt > 0.0);
        if !converted.is_empty() && saw_travel && saw_windup && saw_boulder && damaged {
            break;
        }
    }
    assert!(
        !requests.is_empty() && !converted.is_empty(),
        "normal movement reaches real correlated world conversion"
    );
    assert!(saw_travel, "normal phase controller reaches buried travel");
    assert!(
        saw_windup && saw_boulder,
        "physical exposure precedes a real boulder projectile"
    );
    assert!(damaged, "the actual released boulder removes hostile HP");
}
