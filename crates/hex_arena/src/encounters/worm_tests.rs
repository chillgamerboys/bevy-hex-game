//! Real arena-step regressions for Worm phases and source-correlated earth admission.

use super::*;
use hex_core::arena::{ArenaBurrowResult, ArenaMap, ArenaSelection};
use hex_core::{ElementId, SubstanceId};

struct Fixture {
    session: ArenaSession,
    view: ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    materials: ArenaMaterials,
    tuning: ArenaTuning,
}

fn fixture() -> Fixture {
    let geometry = ArenaVoxelGeometry {
        radius: 16,
        ..Default::default()
    };
    let mut view = ArenaTerrainView {
        selection: ArenaSelection {
            map: ArenaMap::Fort,
            ..Default::default()
        },
        ..Default::default()
    };
    for coord in HexCoord::ORIGIN.within_radius(16) {
        for level in 0..=8 {
            view.voxels.insert(
                TilePos::new(coord, level),
                SubstanceId(if level == 8 { 2 } else { 1 }),
            );
        }
    }
    let materials = ArenaMaterials {
        stone: SubstanceId(1),
        grass: SubstanceId(2),
        dirt: SubstanceId(3),
        bedrock: SubstanceId(4),
        fire: ElementId(1),
    };
    let tuning = ArenaTuning::default();
    let mut worm = Actor::spawn(7, Vec3::new(-6.0, 3.2 + SKIN, 0.0), Vec3::X);
    worm.configure_species(Species::Worm, &tuning.encounters);
    worm.party = Some(2);
    let mut human = Actor::spawn(0, Vec3::new(10.0, 3.2 + SKIN, 0.0), Vec3::NEG_X);
    human.hp = 1000.0;
    human.max_hp = 1000.0;
    let home = worm.feet;
    let mut session = ArenaSession {
        actors: vec![human, worm],
        generation: Some(1),
        ..Default::default()
    };
    session.encounter.initialized = true;
    session.encounter.runtime.push(PartyRuntime {
        snapshot: PartySnapshot {
            id: 2,
            phase: PartyPhase::Active,
            home,
            living: 1,
        },
        knowledge: None,
        last_sight: 0,
        last_cue_id: None,
        leash: 100.0,
        search: 100.0,
        battle_search: None,
    });
    session
        .encounter
        .brains
        .insert(7, brain::Brain::new(7, home));
    session.install_burrow_policy(&ArenaBurrowMaterials {
        eligible: [materials.stone, materials.grass, materials.dirt].into(),
    });
    session.collision.refresh(&view, geometry);
    session.prepare_worms(&view, geometry);
    Fixture {
        session,
        view,
        geometry,
        materials,
        tuning,
    }
}

impl Fixture {
    fn advance(&mut self) -> CommandsOut {
        self.session.advance(
            ActorIntent::default(),
            &self.view,
            self.geometry,
            self.materials,
            &self.tuning,
        )
    }
    fn worm(&self) -> &Actor {
        self.session
            .actors
            .iter()
            .find(|a| a.id == 7)
            .expect("Worm")
    }
    fn apply(&mut self, requests: &[ArenaBurrowRequest]) {
        // This fixture models only material publication and correlated ack; the
        // actual world's HP ledger/conversion tests remain in the world lane.
        for request in requests {
            for pos in &request.volume {
                if self.view.voxels.contains_key(pos) {
                    self.view.voxels.insert(*pos, self.materials.dirt);
                }
            }
            self.view.revision += 1;
            self.view.full_rebuild = true;
            self.session.accept_burrow_outcome(&ArenaBurrowOutcome {
                generation: request.generation,
                actor: request.actor,
                sequence: request.sequence,
                result: ArenaBurrowResult::Accepted {
                    changed: Vec::new(),
                },
            });
        }
    }
}

#[test]
fn conversion_ack_is_not_motion_permission_and_changed_intent_never_replays_old_pose() {
    let mut f = fixture();
    f.session.bot_enabled = false;
    f.session
        .encounter
        .worms
        .get_mut(&7)
        .expect("control")
        .phase(WormPhase::Diving);
    let start = f.worm().feet;
    let commands = f.advance();
    let [request] = commands.burrows.as_slice() else {
        panic!("one bounded proposal")
    };
    assert!(request.structural_rejection().is_none());
    assert_eq!(
        f.worm().feet.to_array().map(f32::to_bits),
        start.to_array().map(f32::to_bits)
    );
    for _ in 0..24 {
        assert!(f.advance().burrows.is_empty());
    }
    f.session.accept_burrow_outcome(&ArenaBurrowOutcome {
        generation: 1,
        actor: 7,
        sequence: request.sequence,
        result: ArenaBurrowResult::Accepted {
            changed: Vec::new(),
        },
    });
    assert!(f.session.pending_burrows.is_empty());
    let next = f.advance();
    let [retry] = next.burrows.as_slice() else {
        panic!("current non-dirt still requires conversion")
    };
    assert!(retry.sequence > request.sequence);
    assert_eq!(
        f.worm().feet.to_array().map(f32::to_bits),
        start.to_array().map(f32::to_bits)
    );
    f.session
        .encounter
        .worms
        .get_mut(&7)
        .expect("control")
        .phase(WormPhase::Emerging);
    f.apply(&next.burrows);
    f.advance();
    assert!((f.worm().feet.y - start.y).abs() < SKIN);
    assert!(f.worm().body_hex_prisms().next().expect("head").offset.y > 0.0);
    assert!(f.session.pending_burrows.is_empty());
}

#[test]
fn death_and_reset_discard_pending_identity_and_cannot_resume_a_stale_dive() {
    let mut f = fixture();
    f.session.bot_enabled = false;
    f.session
        .encounter
        .worms
        .get_mut(&7)
        .expect("control")
        .phase(WormPhase::Diving);
    let out = f.advance();
    let request = out.burrows.first().expect("request").clone();
    f.session
        .actors
        .iter_mut()
        .find(|a| a.id == 7)
        .expect("Worm")
        .hp = 0.0;
    let feet = f.worm().feet;
    f.apply(&out.burrows);
    f.advance();
    assert!(f.session.pending_burrows.is_empty());
    assert_eq!(
        f.worm().feet.to_array().map(f32::to_bits),
        feet.to_array().map(f32::to_bits)
    );
    f.session.reset(2, &f.view, f.geometry);
    f.session.accept_burrow_outcome(&ArenaBurrowOutcome {
        generation: 1,
        actor: 7,
        sequence: request.sequence,
        result: ArenaBurrowResult::Accepted {
            changed: Vec::new(),
        },
    });
    assert!(
        f.session.pending_burrows.is_empty()
            && f.session.next_burrow.is_empty()
            && f.session.encounter.worms.is_empty()
    );
    assert_eq!(f.session.tick, 0);
}

#[test]
fn natural_head_rise_and_boulder_use_physical_exposure_frozen_payload_and_real_damage() {
    let mut f = fixture();
    let mut saw_windup = false;
    let mut saw_rock = false;
    let mut saw_travel = false;
    let mut saw_request = false;
    let mut saw_physical_impact = false;
    for _ in 0..700 {
        let out = f.advance();
        saw_request |= !out.burrows.is_empty();
        saw_physical_impact |= out
            .impacts
            .iter()
            .any(|i| i.kind == TerrainDamageKind::Physical);
        let actor = f.worm();
        let body = actor.worm().expect("physical state");
        saw_travel |= body.phase == WormPhase::Travel;
        let (low, high) = actor.body_prism_snapshot().expect("body").bounds();
        assert!(actor.body_dimensions().distance(high - low) < SKIN);
        if let Some(attack) = actor
            .attack_state()
            .filter(|a| a.kind == CreatureAbility::WormBoulder)
        {
            assert!(body.exposed && body.head_clearance + SKIN >= f.geometry.level_height);
            assert!(attack.origin.distance(actor.eye()) < SKIN);
            saw_windup |= attack.phase == AttackPhase::Windup;
        }
        for shot in f
            .session
            .projectiles
            .iter()
            .filter(|s| s.source_ability() == Some(CreatureAbility::WormBoulder))
        {
            assert_eq!(shot.appearance(), ProjectileAppearance::Boulder);
            saw_rock = true;
        }
        f.apply(&out.burrows);
    }
    assert!(saw_windup && saw_rock && saw_travel && saw_request && saw_physical_impact);
    assert!(
        f.session
            .actors
            .iter()
            .find(|a| a.id == 0)
            .expect("target")
            .hp
            < 1000.0
    );
}

#[test]
fn re_covering_the_head_cancels_unreleased_boulder_and_buried_sensing_has_no_hidden_truth() {
    let mut f = fixture();
    for _ in 0..180 {
        let out = f.advance();
        f.apply(&out.burrows);
        if f.worm().attack_state().is_some_and(|a| {
            a.kind == CreatureAbility::WormBoulder && a.phase == AttackPhase::Windup
        }) {
            break;
        }
    }
    assert!(f
        .worm()
        .attack_state()
        .is_some_and(|a| a.phase == AttackPhase::Windup));
    let blocker = f.geometry.voxel_at(f.worm().eye()).expect("head voxel");
    f.view.voxels.insert(blocker, f.materials.stone);
    f.view.revision += 1;
    f.advance();
    assert!(!f.worm().worm().expect("head").exposed && f.worm().attack_state().is_none());
    assert!(!f
        .session
        .projectiles
        .iter()
        .any(|s| s.source_ability() == Some(CreatureAbility::WormBoulder)));
    let mut other = fixture();
    other
        .session
        .actors
        .iter_mut()
        .find(|a| a.id == 0)
        .expect("target")
        .feet += Vec3::X * 100.0;
    // Both untouched fixture Worms start below the required one-level exposure.
    let original = fixture();
    let a = targeting::observe(
        original.worm(),
        &original.session.actors,
        &[],
        &original.session.collision,
        12,
        0.5,
    );
    let b = targeting::observe(
        other.worm(),
        &other.session.actors,
        &[],
        &other.session.collision,
        12,
        0.5,
    );
    assert!(a.is_empty() && b.is_empty());
}

#[test]
fn pending_conversion_keeps_separate_knockback_and_exposed_forecasts_freeze_owner_history() {
    let mut f = fixture();
    f.session.bot_enabled = false;
    f.session
        .encounter
        .worms
        .get_mut(&7)
        .expect("control")
        .phase(WormPhase::Diving);
    let first = f.advance();
    assert_eq!(first.burrows.len(), 1);
    let before = f.worm().feet;
    f.session
        .actors
        .iter_mut()
        .find(|a| a.id == 7)
        .expect("Worm")
        .body
        .impulse_velocity = Vec3::Y * 2.0;
    f.advance();
    assert!(f.worm().feet.y > before.y);
    assert!(f.worm().body.impulse_velocity.y > 0.0 && f.session.pending_burrows.len() == 1);
    let mut caster = f.worm().clone();
    let parts = WormBodyState::parts_at(4, caster.body_yaw, 1.2).expect("raised body");
    let old = caster.body_prism_snapshot().expect("old");
    caster.set_observed_prisms(parts).expect("copy");
    caster.worm.as_mut().expect("body").previous = old;
    let mut frozen = caster.clone();
    frozen.set_observed_prisms(parts).expect("freeze");
    let spec = boulder_spec(&f.tuning.encounters);
    let a = forecast_creature_projectile(
        &caster,
        Vec3::X,
        spec,
        &[],
        &f.session.collision,
        &f.view,
        f.geometry,
    );
    let b = forecast_creature_projectile(
        &frozen,
        Vec3::X,
        spec,
        &[],
        &f.session.collision,
        &f.view,
        f.geometry,
    );
    assert_eq!(
        a.impact.map(|i| (
            i.point.to_array().map(f32::to_bits),
            i.time.to_bits(),
            i.actor
        )),
        b.impact.map(|i| (
            i.point.to_array().map(f32::to_bits),
            i.time.to_bits(),
            i.actor
        ))
    );
}

#[test]
fn destroyed_support_settles_the_clear_body_without_conversion_or_stale_depth() {
    let mut f = fixture();
    f.session.bot_enabled = false;
    let start = f.worm().feet;
    // Remove four complete support levels, beyond the old shallow-band lookup.
    // The lower published floor remains real, and the Worm starts wholly in air.
    f.view.voxels.retain(|pos, _| pos.level <= 4);
    f.view.revision += 1;
    f.view.full_rebuild = true;
    for _ in 0..180 {
        let out = f.advance();
        assert!(
            out.burrows.is_empty(),
            "falling is never material conversion"
        );
        let body = pose(f.worm()).expect("complete body");
        assert!(f
            .session
            .burrow_query
            .above_ground_clear(body, &f.view, f.geometry));
        assert!(
            f.worm().feet.y >= 1.6 - SKIN,
            "fall cannot tunnel through floor"
        );
    }
    assert!(f.worm().feet.y < start.y - 1.5);
    assert!((f.worm().feet.y - 1.6).abs() < SKIN * 8.0);
    assert!(
        (f.session.encounter.worms.get(&7).expect("control").surface - 1.6).abs() < SKIN,
        "feet {:?}, controller {:?}",
        f.worm().feet,
        f.session.encounter.worms.get(&7)
    );
    assert!(f.worm().body.vertical_velocity.abs() < SKIN);
}

#[test]
fn an_incoming_lethal_hit_discards_the_same_ticks_unpublished_burrow_request() {
    for last_enemy in [false, true] {
        let mut f = fixture();
        f.session.bot_enabled = false;
        f.session
            .encounter
            .worms
            .get_mut(&7)
            .expect("control")
            .phase(WormPhase::Diving);
        f.session
            .actors
            .iter_mut()
            .find(|a| a.id == 7)
            .expect("Worm")
            .hp = 1.0;
        if !last_enemy {
            let mut survivor = Actor::spawn(8, Vec3::new(20.0, 3.2 + SKIN, 0.0), Vec3::NEG_X);
            survivor.configure_species(Species::Goblin, &f.tuning.encounters);
            f.session.actors.push(survivor);
        }
        let mut launch = CommandsOut::default();
        f.session.release(
            0,
            Spell::Fireball,
            &f.tuning,
            f.tuning.projectile_speed,
            &f.view,
            f.geometry,
            f.materials,
            &mut launch,
        );
        let contact = f.worm().eye();
        let incoming = f
            .session
            .projectiles
            .first_mut()
            .expect("real human projectile");
        incoming.position = contact;
        incoming.previous_position = contact;
        let out = f.advance();
        assert!(f.worm().hp <= 0.0);
        assert_eq!(f.session.is_finished(), last_enemy);
        assert_eq!(
            f.session.next_burrow.get(&7),
            Some(&1),
            "motion did propose before damage"
        );
        assert!(f.session.pending_burrows.is_empty());
        assert!(
            out.burrows.is_empty(),
            "dead owner cannot publish a new conversion"
        );
        assert!(
            !out.impacts.is_empty(),
            "the real incoming fireball resolved"
        );
    }
}

#[test]
fn an_airborne_worm_keeps_falling_while_sideways_impulse_pressures_a_wall() {
    let mut f = fixture();
    f.session.bot_enabled = false;
    let anchor = HexCoord::from_axial(-4, 0);
    let feet = anchor.to_world(5.0);
    let actor = f
        .session
        .actors
        .iter_mut()
        .find(|a| a.id == 7)
        .expect("Worm");
    actor.feet = feet;
    actor.previous_feet = feet;
    // Every segment lies along this native row. The adjacent row's shared hex
    // faces are tangent, so an inward lateral impulse immediately meets stone.
    for q in -8..=0 {
        for level in 9..=20 {
            f.view.voxels.insert(
                TilePos::new(HexCoord::from_axial(q, 1), level),
                f.materials.stone,
            );
        }
    }
    f.view.revision += 1;
    f.view.full_rebuild = true;
    f.session.collision.refresh(&f.view, f.geometry);
    assert!(shapes::clear(
        &f.session.collision,
        f.worm(),
        feet,
        f.worm().body_yaw
    ));
    let mut last_y = feet.y;
    for _ in 0..30 {
        // Sustained external pressure makes this stronger than merely waiting
        // for one impulse to decay; its vertical component is always zero.
        f.session
            .actors
            .iter_mut()
            .find(|a| a.id == 7)
            .expect("Worm")
            .body
            .impulse_velocity = Vec3::Z * 8.0;
        let out = f.advance();
        assert!(out.burrows.is_empty());
        assert!(
            f.worm().feet.y < last_y,
            "side contact cannot suspend the body"
        );
        assert!(f.session.burrow_query.above_ground_clear(
            pose(f.worm()).expect("pose"),
            &f.view,
            f.geometry
        ));
        last_y = f.worm().feet.y;
    }
    assert!(f.worm().feet.y < feet.y - 0.5);
    assert!(f.worm().body.vertical_velocity < -4.0);
}
