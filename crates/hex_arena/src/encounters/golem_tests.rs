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
        let tuning = ArenaTuning::default();
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
fn laser_tracks_until_lock_then_keeps_direction_while_knockback_moves_its_mouth() {
    let mut f = Fixture::new();
    f.start(CreatureAbility::GolemLaser);
    f.ticks(180);
    assert!(f.owner().beam().is_some_and(|beam| !beam.locked));
    f.owner_mut().aim = Vec3::new(1.0, 0.0, 0.1).normalize();
    f.tick();
    assert!(
        f.owner()
            .beam()
            .expect("tracking")
            .direction
            .distance(f.owner().aim)
            < SKIN
    );
    f.ticks(20);
    let locked = f.owner().beam().expect("locked warning");
    assert!(locked.locked);
    f.owner_mut().aim = Vec3::NEG_Z;
    f.owner_mut().body.impulse_velocity = Vec3::Z * 4.0;
    let mut moved = f.owner().clone();
    motion::tick(
        &mut moved,
        Vec3::ZERO,
        false,
        false,
        false,
        &f.session.collision,
        &f.tuning.encounters,
    );
    *f.owner_mut() = moved;
    f.tick();
    let after = f.owner().beam().expect("locked moving mouth");
    assert!(after.direction.distance(locked.direction) < SKIN);
    assert!(after.origin.z > locked.origin.z + 0.01);
    assert!(
        after
            .origin
            .distance(shapes::golem_mouth(f.owner(), locked.direction))
            < SKIN
    );
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
fn lost_sight_cancels_unlocked_preparation_but_cannot_redirect_the_locked_line() {
    for (elapsed_ticks, must_cancel) in [(120, true), (205, false)] {
        let mut f = Fixture::new();
        f.start(CreatureAbility::GolemLaser);
        f.ticks(elapsed_ticks);
        let locked_direction = f.owner().beam().expect("warning").direction;
        for coord in HexCoord::ORIGIN.within_radius(20) {
            if coord.x() == 3 {
                for level in 1..=12 {
                    f.view
                        .voxels
                        .insert(TilePos::new(coord, level), SubstanceId(1));
                }
            }
        }
        f.refresh();
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
            f.session.tick + 1,
        );
        assert!(request.is_none());
        assert_eq!(
            f.brains.get(&1).expect("brain").active.is_none(),
            must_cancel
        );
        assert!(f
            .brains
            .get(&1)
            .expect("brain")
            .cooldowns
            .get(CreatureAbility::GolemLaser.index())
            .is_some_and(|cd| *cd > 0.0));
        if !must_cancel {
            assert!(motion.input.aim.distance(locked_direction) < SKIN);
        }
    }
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
