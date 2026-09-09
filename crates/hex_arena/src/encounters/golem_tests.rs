//! Authority regressions: real finite casts, observed decisions and prism motion.

use super::*;
use hex_core::{ElementId, SubstanceId};

struct Fixture {
    session: ArenaSession,
    view: ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    materials: ArenaMaterials,
    tuning: ArenaTuning,
    brains: BTreeMap<ActorId, brain::Brain>,
}

impl Fixture {
    fn new() -> Self {
        let geometry = ArenaVoxelGeometry {
            radius: 20,
            ..Default::default()
        };
        let view = ArenaTerrainView {
            voxels: HexCoord::ORIGIN
                .within_radius(20)
                .into_iter()
                .map(|coord| (TilePos::new(coord, 0), SubstanceId(1)))
                .collect(),
            ..Default::default()
        };
        // Keep the original short budget in the geometric contact fixtures.
        // Pressure tests below explicitly use the current authored defaults.
        let tuning = ArenaTuning {
            encounters: EncounterTuning {
                golem_laser_seconds: 1.0,
                golem_laser_damage: 45.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut golem = Actor::spawn(1, Vec3::Y * SKIN, Vec3::X);
        golem.configure_species(Species::Golem, &tuning.encounters);
        golem.body.grounded = true;
        golem.grounded = true;
        let mut target = Actor::spawn(0, Vec3::new(15.0, 1.0 + SKIN, 0.0), Vec3::NEG_X);
        target.hp = 200.0;
        target.max_hp = 200.0;
        let mut session = ArenaSession {
            actors: vec![target, golem],
            ..Default::default()
        };
        session.collision.refresh(&view, geometry);
        Self {
            session,
            view,
            geometry,
            materials: ArenaMaterials {
                stone: SubstanceId(1),
                grass: SubstanceId(2),
                dirt: SubstanceId(3),
                bedrock: SubstanceId(4),
                fire: ElementId(1),
            },
            tuning,
            brains: BTreeMap::from([(1, brain::Brain::new(1, Vec3::ZERO))]),
        }
    }

    fn start(&mut self, kind: CreatureAbility) {
        let aim = self.session.actors.get(1).expect("golem").aim;
        self.session.begin_ability(
            &mut self.brains,
            1,
            brain::Request { kind, aim },
            &self.tuning,
        );
        assert!(self.brains.get(&1).expect("brain").active.is_some());
    }

    fn tick(&mut self) -> CommandsOut {
        self.session.tick += 1;
        self.ability_tick()
    }

    fn ability_tick(&mut self) -> CommandsOut {
        let mut out = CommandsOut::default();
        self.session.advance_abilities(
            &mut self.brains,
            &self.view,
            self.geometry,
            self.materials,
            &self.tuning,
            &mut out,
        );
        out
    }

    fn ticks(&mut self, count: usize) -> Vec<TerrainImpact> {
        (0..count).flat_map(|_| self.tick().impacts).collect()
    }

    fn owner(&self) -> &Actor {
        self.session.actors.get(1).expect("golem")
    }
    fn owner_mut(&mut self) -> &mut Actor {
        self.session.actors.get_mut(1).expect("golem")
    }
    fn refresh(&mut self) {
        self.view.revision += 1;
        self.view.full_rebuild = true;
        self.session.collision.refresh(&self.view, self.geometry);
    }

    fn observed_tick(&mut self) -> CommandsOut {
        self.session.tick += 1;
        let party = PartyRuntime {
            snapshot: PartySnapshot {
                id: 0,
                phase: PartyPhase::Active,
                home: Vec3::ZERO,
                living: 1,
            },
            knowledge: None,
            last_sight: self.session.tick,
            last_cue_id: None,
            leash: 30.0,
            search: 6.0,
            battle_search: Some(Vec3::X * 20.0),
        };
        let (intent, request) = self.brains.get_mut(&1).expect("brain").intent(
            self.session.actors.get(1).expect("golem"),
            &party,
            &self.session.actors,
            &[],
            &[],
            &self.session.collision,
            &self.view,
            self.geometry,
            &self.tuning,
            self.session.tick,
        );
        let mut actor = self.owner().clone();
        actor.aim = intent.input.aim;
        motion::tick(
            &mut actor,
            intent.direction,
            intent.input.run,
            intent.input.jump,
            intent.flight,
            &self.session.collision,
            &self.tuning.encounters,
        );
        *self.owner_mut() = actor;
        if let Some(request) = request {
            self.session
                .begin_ability(&mut self.brains, 1, request, &self.tuning);
        }
        self.ability_tick()
    }

    fn divider(&mut self) {
        for coord in HexCoord::ORIGIN.within_radius(20) {
            if coord.x() == 3 {
                for level in 1..=20 {
                    self.view
                        .voxels
                        .insert(TilePos::new(coord, level), self.materials.stone);
                }
            }
        }
        self.refresh();
    }
}

#[test]
fn observed_golem_chooses_close_sphere_or_long_beam_and_keeps_the_medium_gap() {
    for (distance, expected) in [
        (6.0, Some(CreatureAbility::GolemSlam)),
        (10.0, None),
        (16.0, Some(CreatureAbility::GolemLaser)),
    ] {
        let mut f = Fixture::new();
        f.session.actors.first_mut().expect("human").feet = Vec3::new(distance, SKIN, 0.0);
        let party = PartyRuntime {
            snapshot: PartySnapshot {
                id: 0,
                phase: PartyPhase::Active,
                home: Vec3::ZERO,
                living: 1,
            },
            knowledge: None,
            last_sight: 0,
            last_cue_id: None,
            leash: 30.0,
            search: 6.0,
            battle_search: None,
        };
        let (_, request) = f.brains.get_mut(&1).expect("brain").intent(
            f.session.actors.get(1).expect("golem"),
            &party,
            &f.session.actors,
            &[],
            &[],
            &f.session.collision,
            &f.view,
            f.geometry,
            &f.tuning,
            1,
        );
        assert_eq!(request.map(|r| r.kind), expected, "distance {distance}");
    }
}

#[test]
fn continuous_laser_has_no_windup_damage_and_one_cast_cap_with_allies_passed() {
    let mut f = Fixture::new();
    let mut ally = Actor::spawn(2, Vec3::new(8.0, 1.0, 0.0), Vec3::X);
    ally.team = f.owner().team;
    let mut farther = Actor::spawn(3, Vec3::new(20.0, 1.0, 0.0), Vec3::NEG_X);
    farther.team = 0;
    f.session.actors.extend([ally, farther]);
    f.start(CreatureAbility::GolemLaser);
    f.ticks(239);
    assert!((f.session.actors.first().expect("target").hp - 200.0).abs() < SKIN);
    f.ticks(150);
    assert!((f.session.actors.first().expect("target").hp - 155.0).abs() < 0.01);
    assert!((f.session.actors.get(2).expect("ally").hp - 100.0).abs() < SKIN);
    assert!((f.session.actors.get(3).expect("farther hostile").hp - 100.0).abs() < SKIN);
    assert_eq!(
        f.session
            .encounter
            .ability_counts
            .get(&1)
            .expect("counts")
            .get(CreatureAbility::GolemLaser.index())
            .copied(),
        Some(1)
    );
    assert!(f.owner().beam().is_none());
}

#[test]
fn laser_nearest_terrain_blocks_actor_and_barrier_with_once_per_voxel_fire_power() {
    let mut f = Fixture::new();
    let coord = HexCoord::from_axial(3, 0);
    for level in 1..=5 {
        f.view
            .voxels
            .insert(TilePos::new(coord, level), SubstanceId(1));
    }
    f.refresh();
    f.session.encounter.barriers.push(BarrierSnapshot {
        id: 7,
        owner: 0,
        center: Vec3::new(8.0, 1.4, 0.0),
        normal: Vec3::X,
        width: 3.0,
        height: 3.0,
        hp: 60.0,
        max_hp: 60.0,
        remaining: 10.0,
        lifetime: 10.0,
    });
    f.session
        .collision
        .sync_barriers(&f.session.encounter.barriers);
    f.start(CreatureAbility::GolemLaser);
    let impacts = f.ticks(380);
    assert_eq!(impacts.len(), 1);
    let impact = impacts.first().expect("one terrain admission");
    assert_eq!(impact.volume, vec![TilePos::new(coord, 4)]);
    assert_eq!(impact.kind, TerrainDamageKind::Elemental(f.materials.fire));
    assert_eq!(impact.power, 2);
    assert!(
        (f.session
            .barriers()
            .first()
            .expect("barrier behind terrain")
            .hp
            - 60.0)
            .abs()
            < SKIN
    );
    assert!((f.session.actors.first().expect("target").hp - 200.0).abs() < SKIN);
}

#[test]
fn laser_barrier_absorbs_only_its_cast_budget_and_blocks_a_hostile_behind_it() {
    let mut f = Fixture::new();
    f.session.encounter.barriers.push(BarrierSnapshot {
        id: 7,
        owner: 1,
        center: Vec3::new(8.0, 1.4, 0.0),
        normal: Vec3::X,
        width: 3.0,
        height: 3.0,
        hp: 60.0,
        max_hp: 60.0,
        remaining: 10.0,
        lifetime: 10.0,
    });
    f.session
        .collision
        .sync_barriers(&f.session.encounter.barriers);
    f.start(CreatureAbility::GolemLaser);
    f.ticks(380);
    assert!(
        (f.session
            .barriers()
            .first()
            .expect("own barrier blocks both ways")
            .hp
            - 15.0)
            .abs()
            < 0.01
    );
    assert!((f.session.actors.first().expect("target").hp - 200.0).abs() < SKIN);
}

#[test]
fn slam_is_a_true_unoccluded_sphere_with_team_immunity_and_one_physical_terrain_pulse() {
    let mut f = Fixture::new();
    f.session.actors.first_mut().expect("target").feet = Vec3::new(5.0, SKIN, 0.0);
    let mut high = Actor::spawn(2, Vec3::new(0.0, 8.5, 0.0), Vec3::X);
    high.team = 0;
    let mut ally = Actor::spawn(3, Vec3::new(0.0, SKIN, 4.0), Vec3::X);
    ally.team = 1;
    f.session.actors.extend([high, ally]);
    for level in 1..=6 {
        f.view.voxels.insert(
            TilePos::new(HexCoord::from_axial(2, 0), level),
            SubstanceId(1),
        );
    }
    f.refresh();
    f.start(CreatureAbility::GolemSlam);
    f.ticks(95);
    assert!(f.session.effects.is_empty());
    let impacts = f.ticks(40);
    assert_eq!(impacts.len(), 1);
    assert_eq!(
        impacts.first().expect("slam terrain").kind,
        TerrainDamageKind::Physical
    );
    assert_eq!(impacts.first().expect("slam terrain").power, 2);
    assert!((f.session.actors.first().expect("covered enemy").hp - 165.0).abs() < SKIN);
    assert!(
        f.session
            .actors
            .first()
            .expect("enemy impulse")
            .impulse_velocity()
            .length()
            > 4.9
    );
    assert!(
        (f.session
            .actors
            .get(2)
            .expect("outside spherical height")
            .hp
            - 100.0)
            .abs()
            < SKIN
    );
    assert!((f.session.actors.get(3).expect("ally").hp - 100.0).abs() < SKIN);
    assert!((f.owner().hp - f.tuning.encounters.golem_hp).abs() < SKIN);
    assert_eq!(f.session.effects.len(), 1);
    let effect = f.session.effects.first().expect("ordinary sphere VFX");
    assert_eq!(effect.kind, Spell::AreaBlast);
    assert!(effect.center.distance(f.owner().center()) < SKIN);
    assert!(
        impacts
            .first()
            .expect("terrain")
            .volume
            .contains(&TilePos::new(HexCoord::ORIGIN, 0)),
        "own support is part of the true sphere, without special protection"
    );
}

#[test]
fn slam_support_publication_leaves_golem_subject_to_gravity_and_knockback_without_jump() {
    let mut f = Fixture::new();
    f.start(CreatureAbility::GolemSlam);
    let impacts = f.ticks(100);
    let original = f.owner().feet;
    // Simulate the next authoritative world publication after a destructible
    // support layer is removed. Geometry and body physics see the same revision.
    for pos in &impacts.first().expect("slam volume").volume {
        f.view.voxels.remove(pos);
    }
    f.refresh();
    f.owner_mut().body.impulse_velocity = Vec3::X * 2.0;
    let mut actor = f.owner().clone();
    for _ in 0..30 {
        motion::tick(
            &mut actor,
            Vec3::ZERO,
            true,
            true,
            true,
            &f.session.collision,
            &f.tuning.encounters,
        );
    }
    assert!(actor.feet.y < original.y - 0.4 && actor.feet.x > original.x + 0.2);
    assert!(
        !actor.flying && !actor.grounded,
        "jump/flight inputs cannot rescue a ground-only Golem"
    );
}

#[test]
fn death_cancels_remaining_beam_damage_and_published_attack() {
    let mut f = Fixture::new();
    f.start(CreatureAbility::GolemLaser);
    f.ticks(260);
    let hp = f.session.actors.first().expect("target").hp;
    assert!(hp < 200.0 && hp > 155.0);
    f.owner_mut().hp = 0.0;
    f.ticks(150);
    assert!((f.session.actors.first().expect("target").hp - hp).abs() < SKIN);
    assert!(f.owner().beam().is_none() && f.owner().attack_state().is_none());
    assert!(f.brains.get(&1).expect("brain").active.is_none());
}

#[test]
fn world_endpoint_uses_native_hex_boundary_and_vertical_limits_without_a_range_cap() {
    let geometry = ArenaVoxelGeometry {
        radius: 100,
        ..Default::default()
    };
    let start = Vec3::Y * 1.0;
    for direction in [Vec3::X, Vec3::Z, Vec3::new(1.0, 0.0, 0.7).normalize()] {
        let end = world_end(start, direction, geometry);
        assert!(end.distance(start) > 150.0, "{direction:?}: {end:?}");
        assert!(geometry.contains_column(HexCoord::from_world(end - direction * 0.01)));
        assert!(!geometry.contains_column(HexCoord::from_world(end + direction * 0.01)));
    }
    let up = world_end(start, Vec3::Y, geometry);
    assert!((up.y - geometry.top(TilePos::new(HexCoord::ORIGIN, geometry.max_level))).abs() < SKIN);
    let down = world_end(start, Vec3::NEG_Y, geometry);
    assert!((down.y + geometry.level_height).abs() < SKIN);
}

#[test]
fn observed_laser_aim_uses_the_surface_mouth_and_hits_a_low_dragon_at_elevation() {
    let mut f = Fixture::new();
    let dragon = f.session.actors.first_mut().expect("target");
    dragon.configure_species(Species::Dragon, &f.tuning.encounters);
    dragon.feet = Vec3::new(18.0, SKIN, 0.0);
    dragon.previous_feet = dragon.feet;
    let hp = dragon.hp;
    let party = PartyRuntime {
        snapshot: PartySnapshot {
            id: 0,
            phase: PartyPhase::Active,
            home: Vec3::ZERO,
            living: 1,
        },
        knowledge: None,
        last_sight: 0,
        last_cue_id: None,
        leash: 30.0,
        search: 6.0,
        battle_search: Some(Vec3::X * 18.0),
    };
    let (motion, request) = f.brains.get_mut(&1).expect("brain").intent(
        f.session.actors.get(1).expect("golem"),
        &party,
        &f.session.actors,
        &[],
        &[],
        &f.session.collision,
        &f.view,
        f.geometry,
        &f.tuning,
        1,
    );
    assert_eq!(
        request.expect("long low target").kind,
        CreatureAbility::GolemLaser
    );
    f.owner_mut().aim = motion.input.aim;
    f.start(CreatureAbility::GolemLaser);
    f.ticks(380);
    assert!((f.session.actors.first().expect("low target").hp - (hp - 45.0)).abs() < 0.01);
}

#[test]
fn laser_starting_sphere_overlap_damages_the_tangent_wall_but_not_static_only_cover() {
    for solid_terrain in [true, false] {
        let mut f = Fixture::new();
        let coord = HexCoord::from_axial(2, 0);
        if solid_terrain {
            for level in 1..=5 {
                f.view
                    .voxels
                    .insert(TilePos::new(coord, level), SubstanceId(1));
            }
        } else {
            f.view.static_spans.push(hex_core::arena::ArenaStaticSpan {
                bottom: TilePos::new(coord, 1),
                top_level: 5,
                blocks_movement: true,
                blocks_projectiles: true,
                blocks_sight: true,
            });
        }
        f.refresh();
        assert!(
            shapes::clear(&f.session.collision, f.owner(), f.owner().feet, 0.0),
            "a tangent wall leaves the complete Golem body clear"
        );
        f.start(CreatureAbility::GolemLaser);
        let impacts = f.ticks(380);
        assert!((f.session.actors.first().expect("blocked target").hp - 200.0).abs() < SKIN);
        if solid_terrain {
            assert_eq!(impacts.len(), 1);
            assert_eq!(
                impacts.first().expect("tangent overlap admission").volume,
                vec![TilePos::new(coord, 4)]
            );
        } else {
            assert!(
                impacts.is_empty(),
                "immutable static geometry is not invented voxel HP"
            );
        }
    }
}

#[test]
fn observed_pressure_laser_runs_four_seconds_with_180_total_damage_and_bounded_turns() {
    let mut f = Fixture::new();
    f.tuning = ArenaTuning::default();
    let mut active_ticks = 0_u16;
    let mut previous = None::<Vec3>;
    for _ in 0..800 {
        f.observed_tick();
        if let Some(beam) = f.owner().beam() {
            if let Some(old) = previous {
                let angle = old.dot(beam.direction).clamp(-1.0, 1.0).acos();
                assert!(angle <= f.tuning.encounters.golem_laser_turn_speed * STEP + SKIN);
            }
            previous = Some(beam.direction);
            if f.owner()
                .attack_state()
                .is_some_and(|a| a.phase == AttackPhase::Active)
            {
                active_ticks += 1;
                assert!(beam.tracking);
            }
        }
        if f.session.tick < 239 {
            assert!((f.session.actors.first().expect("target").hp - 200.0).abs() < SKIN);
        }
    }
    assert!(
        (479..=481).contains(&active_ticks),
        "four-second active window: {active_ticks}"
    );
    assert!((f.session.actors.first().expect("target").hp - 20.0).abs() < 0.03);
    assert!(f.owner().beam().is_none());
    assert_eq!(
        f.session
            .encounter
            .ability_counts
            .get(&1)
            .expect("one beam")
            .get(CreatureAbility::GolemLaser.index()),
        Some(&1)
    );
}

#[test]
fn active_laser_tracks_lateral_sight_at_10hz_with_a_per_step_turn_limit() {
    let mut f = Fixture::new();
    f.tuning = ArenaTuning::default();
    for _ in 0..265 {
        f.observed_tick();
    }
    let initial = f.owner().beam().expect("active beam").direction;
    f.session.actors.first_mut().expect("visible target").feet.z = 7.0;
    let mut previous = initial;
    for _ in 0..120 {
        f.observed_tick();
        let beam = f.owner().beam().expect("sustained beam");
        let change = previous.dot(beam.direction).clamp(-1.0, 1.0).acos();
        assert!(change <= 1.2 * STEP + SKIN, "one-step turn {change}");
        assert!(beam.tracking);
        previous = beam.direction;
    }
    assert!(
        previous.z > initial.z + 0.2,
        "the live shot must turn toward its observation"
    );
    let old = f.owner().beam().expect("before impulse");
    f.owner_mut().body.impulse_velocity = Vec3::Z * 4.0;
    f.observed_tick();
    let moved = f.owner().beam().expect("after impulse");
    assert!(moved.origin.z > old.origin.z);
    assert!(
        moved
            .origin
            .distance(shapes::golem_mouth(f.owner(), moved.direction))
            < SKIN
    );
}

#[test]
fn active_laser_coasts_from_its_own_velocity_without_hidden_pose_or_target_switch_leaks() {
    let mut fixtures = [Fixture::new(), Fixture::new()];
    for f in &mut fixtures {
        f.tuning = ArenaTuning::default();
        for frame in 0..265_u16 {
            f.session
                .actors
                .first_mut()
                .expect("moving observed target")
                .feet
                .z = f32::from(frame) * STEP;
            f.observed_tick();
        }
        f.divider();
        let mut distractor = Actor::spawn(3, Vec3::new(-6.0, SKIN, 3.0), Vec3::X);
        distractor.team = 0;
        f.session.actors.push(distractor);
    }
    let initial = fixtures
        .first()
        .expect("first run")
        .owner()
        .beam()
        .expect("active beam")
        .direction;
    for frame in 0..360_u16 {
        for (index, f) in fixtures.iter_mut().enumerate() {
            let human = f.session.actors.first_mut().expect("hidden target");
            human.feet = if index == 0 {
                Vec3::new(20.0, 1.0, -4.0)
            } else {
                Vec3::new(24.0, 1.0, 8.0 + f32::from(frame) * STEP)
            };
            f.observed_tick();
            assert!(!f.owner().beam().expect("coasting beam").tracking);
        }
        let a = fixtures
            .first()
            .expect("first run")
            .owner()
            .beam()
            .expect("first beam");
        let b = fixtures
            .get(1)
            .expect("second run")
            .owner()
            .beam()
            .expect("second beam");
        assert_eq!(
            a.direction.to_array().map(f32::to_bits),
            b.direction.to_array().map(f32::to_bits)
        );
    }
    let f = fixtures.first().expect("first run");
    assert!(
        f.owner()
            .beam()
            .expect("coast beyond forecast horizon")
            .direction
            .z
            > initial.z + 0.05
    );
    assert!(
        f.brains
            .get(&1)
            .expect("brain")
            .active
            .as_ref()
            .expect("cast")
            .laser_target()
            == Some(0)
    );
}

#[test]
fn full_laser_charge_requires_sight_and_reacquisition_refreshes_an_active_shot() {
    let mut canceled = Fixture::new();
    canceled.tuning = ArenaTuning::default();
    for _ in 0..210 {
        canceled.observed_tick();
    }
    canceled.divider();
    canceled.observed_tick();
    assert!(canceled.brains.get(&1).expect("brain").active.is_none());
    assert!(!canceled.session.encounter.ability_counts.contains_key(&1));
    assert!(canceled
        .brains
        .get(&1)
        .expect("brain")
        .cooldowns
        .get(CreatureAbility::GolemLaser.index())
        .is_some_and(|cd| *cd > 0.0));

    let mut f = Fixture::new();
    f.tuning = ArenaTuning::default();
    for _ in 0..260 {
        f.observed_tick();
    }
    f.divider();
    for _ in 0..30 {
        f.observed_tick();
    }
    assert!(!f.owner().beam().expect("hidden target coast").tracking);
    f.view.voxels.retain(|pos, _| pos.level == 0);
    f.refresh();
    f.session
        .actors
        .first_mut()
        .expect("reacquired target")
        .feet
        .z = -5.0;
    for _ in 0..120 {
        f.observed_tick();
    }
    let beam = f.owner().beam().expect("same active cast");
    assert!(beam.tracking && beam.direction.z < -0.1);
    assert_eq!(
        f.session
            .encounter
            .ability_counts
            .get(&1)
            .expect("count")
            .get(CreatureAbility::GolemLaser.index()),
        Some(&1)
    );
}

#[test]
fn stone_swipe_is_distinct_short_frontal_damage_without_ally_hit_or_impulse() {
    let mut f = Fixture::new();
    f.session.actors.first_mut().expect("front target").feet = Vec3::new(4.0, SKIN, 0.0);
    let mut rear = Actor::spawn(2, Vec3::new(-4.0, SKIN, 0.0), Vec3::X);
    rear.team = 0;
    let mut high = Actor::spawn(3, Vec3::new(4.0, 3.0, 0.0), Vec3::X);
    high.team = 0;
    let mut far = Actor::spawn(4, Vec3::new(7.0, SKIN, 0.0), Vec3::X);
    far.team = 0;
    let mut ally = Actor::spawn(5, Vec3::new(4.0, SKIN, 1.0), Vec3::X);
    ally.team = f.owner().team;
    f.session.actors.extend([rear, high, far, ally]);
    f.start(CreatureAbility::GolemSwipe);
    let early = f.ticks(40);
    assert!(early.is_empty());
    assert!((f.session.actors.first().expect("windup").hp - 200.0).abs() < SKIN);
    let impacts = f.ticks(80);
    assert!(
        impacts.is_empty(),
        "a frontal strike must not dig its own floor"
    );
    assert!((f.session.actors.first().expect("struck target").hp - 175.0).abs() < SKIN);
    assert!(
        f.session
            .actors
            .first()
            .expect("no added impulse")
            .impulse_velocity()
            .length()
            < SKIN
    );
    for actor in f.session.actors.iter().skip(2) {
        assert!((actor.hp - 100.0).abs() < SKIN);
    }
    assert_eq!(CreatureAbility::GolemSwipe.index(), 11);
    assert_eq!(
        f.session
            .encounter
            .ability_counts
            .get(&1)
            .expect("distinct swipe count")
            .get(CreatureAbility::GolemSwipe.index()),
        Some(&1)
    );
    assert!(!f
        .brains
        .get(&1)
        .expect("cooldown")
        .ready(CreatureAbility::GolemSwipe));
}

#[test]
fn stone_swipe_clears_front_stone_but_preserves_floor_protection_and_cover_ordering() {
    let mut f = Fixture::new();
    let front = HexCoord::from_axial(2, 0);
    let rear = HexCoord::from_axial(-2, 0);
    for coord in [front, rear] {
        for level in 1..=5 {
            f.view
                .voxels
                .insert(TilePos::new(coord, level), f.materials.stone);
        }
    }
    f.view.edit_protected.insert(front, vec![(5, 5)]);
    f.refresh();
    f.session
        .actors
        .first_mut()
        .expect("target behind wall")
        .feet = Vec3::new(4.5, SKIN, 0.0);
    f.start(CreatureAbility::GolemSwipe);
    let impacts = f.ticks(100);
    assert_eq!(impacts.len(), 1);
    let impact = impacts.first().expect("front stone impact");
    assert_eq!(impact.kind, TerrainDamageKind::Physical);
    assert_eq!(impact.power, 8);
    assert_eq!(
        impact.volume,
        (1..=4)
            .map(|level| TilePos::new(front, level))
            .collect::<Vec<_>>()
    );
    assert!((f.session.actors.first().expect("covered target").hp - 200.0).abs() < SKIN);
    assert!(impact
        .volume
        .iter()
        .all(|pos| pos.level > 0 && pos.coord != rear));
}

#[test]
fn blocked_golem_uses_swipe_then_follows_the_published_opening_without_teleport() {
    let mut f = Fixture::new();
    f.tuning = ArenaTuning::default();
    // After the wall opens, this target is in the deliberate medium gap, so
    // choosing an ordinary long laser cannot legitimately stop the route probe.
    f.session.actors.first_mut().expect("covered target").feet.x = 10.0;
    for coord in HexCoord::ORIGIN.within_radius(2) {
        if coord.distance(HexCoord::ORIGIN) == 2 {
            for level in 1..=5 {
                f.view
                    .voxels
                    .insert(TilePos::new(coord, level), f.materials.stone);
            }
        }
    }
    f.refresh();
    let start = f.owner().feet;
    let mut impact = None;
    for _ in 0..180 {
        let out = f.observed_tick();
        if let Some(value) = out
            .impacts
            .into_iter()
            .find(|value| value.kind == TerrainDamageKind::Physical)
        {
            impact = Some(value);
            break;
        }
    }
    let impact = impact.expect("actual no-progress recovery swipe");
    assert_eq!(impact.power, 8);
    assert!(!impact.volume.is_empty() && impact.volume.iter().all(|pos| pos.level > 0));
    // Replay the world's next published cleared surface; the controller itself
    // must traverse it. Actual material HP admission is owned by world tests.
    for pos in impact.volume {
        f.view.voxels.remove(&pos);
    }
    f.refresh();
    for _ in 0..120 {
        let previous = f.owner().feet;
        f.observed_tick();
        assert!(f.owner().feet.distance(previous) < 0.03);
        assert!(shapes::clear(
            &f.session.collision,
            f.owner(),
            f.owner().feet,
            0.0
        ));
    }
    assert!(
        f.owner().feet.x > start.x + 0.5,
        "the real body should leave the cleared pocket"
    );
    assert!(f.owner().feet.y >= start.y - SKIN);
}

#[test]
fn immutable_static_cover_and_missing_floor_do_not_admit_a_stone_swipe() {
    let mut f = Fixture::new();
    f.view.static_spans.push(hex_core::arena::ArenaStaticSpan {
        bottom: TilePos::new(HexCoord::from_axial(2, 0), 1),
        top_level: 5,
        blocks_movement: true,
        blocks_projectiles: true,
        blocks_sight: true,
    });
    f.refresh();
    assert!(!golem_swipe_blocked(
        f.owner(),
        Vec3::X,
        &f.session.collision,
        &f.view,
        f.geometry
    ));
    f.view.static_spans.clear();
    f.view.voxels.clear();
    f.refresh();
    assert!(!golem_swipe_blocked(
        f.owner(),
        Vec3::X,
        &f.session.collision,
        &f.view,
        f.geometry
    ));
}
