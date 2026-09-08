//! Independent creature sight regressions; child of encounters/tests.rs.

use super::*;

const OBSERVER: u8 = 7;
const NEAR: u8 = 8;

struct SightFixture {
    actors: Vec<Actor>,
    party: PartyRuntime,
    world: ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    collision: CollisionWorld,
    tuning: ArenaTuning,
}

impl SightFixture {
    fn head_only(species: Species) -> Self {
        let (_, mut world, geometry, _, tuning) = fixture(ArenaEncounter::Dragon);
        let mut observer = Actor::spawn(OBSERVER, Vec3::new(-3.0, 0.8 + SKIN, 0.0), Vec3::X);
        observer.species = species;
        observer.team = 7;
        observer.party = Some(1);
        observer.body_yaw = -std::f32::consts::FRAC_PI_2;
        observer.previous_yaw = observer.body_yaw;
        let support = if species == Species::Dragon {
            observer.dimensions = Vec3::new(
                tuning.encounters.dragon_width,
                tuning.encounters.dragon_height,
                tuning.encounters.dragon_length,
            );
            // Its physical mouth is near the upright observer's eye position.
            observer.feet = Vec3::new(-3.0 - (observer.dimensions.z * 0.5 - 0.05), 1.2 + SKIN, 0.0);
            3
        } else {
            2
        };
        observer.previous_feet = observer.feet;
        observer.grounded = true;
        observer.body.grounded = true;
        for coord in HexCoord::ORIGIN.within_radius(geometry.radius) {
            if coord.to_world(0.0).x < -2.5 {
                for level in 1..=support {
                    world
                        .voxels
                        .insert(TilePos::new(coord, level), SubstanceId(1));
                }
            }
        }
        for level in 1..=2 {
            world
                .voxels
                .insert(TilePos::new(HexCoord::ORIGIN, level), SubstanceId(1));
        }
        let mut target = Actor::spawn(NEAR, Vec3::new(3.0, SKIN, 0.0), Vec3::NEG_X);
        target.species = Species::Goblin;
        target.team = 42;
        let party = PartyRuntime {
            snapshot: PartySnapshot {
                id: 1,
                phase: PartyPhase::Active,
                home: observer.feet,
                living: 1,
            },
            knowledge: None,
            last_sight: 0,
            last_cue_id: None,
            leash: 30.0,
            search: 6.0,
            battle_search: Some(target.feet),
        };
        world.revision += 1;
        let mut collision = CollisionWorld::default();
        collision.refresh(&world, geometry);
        let result = Self {
            actors: vec![observer, target],
            party,
            world,
            geometry,
            collision,
            tuning,
        };
        result.assert_head_only();
        for actor in &result.actors {
            assert!(shapes::clear(
                &result.collision,
                actor,
                actor.feet,
                actor.body_yaw
            ));
            assert!(shapes::ground(&result.collision, actor, actor.feet, 0.001).is_some());
        }
        result
    }

    fn observer(&self) -> &Actor {
        self.actors
            .iter()
            .find(|a| a.id == OBSERVER)
            .expect("observer")
    }

    fn actor(&self, id: ActorId) -> &Actor {
        self.actors
            .iter()
            .find(|a| a.id == id)
            .expect("fixture actor")
    }

    fn assert_head_only(&self) {
        let eye = self.observer().eye();
        assert!(
            !self.collision.sight_clear(eye, self.actor(NEAR).center()),
            "low wall must cover center"
        );
        assert!(
            self.collision.sight_clear(eye, self.actor(NEAR).eye()),
            "the actual head must remain visible"
        );
        let observations =
            targeting::observe(self.observer(), &self.actors, &[], &self.collision, 1, 0.5);
        assert!(
            observations.iter().any(|seen| seen.body.id == NEAR),
            "the production facade admits head-only sight"
        );
    }

    fn decide(
        &self,
        brain: &mut brain::Brain,
        tick: u64,
    ) -> (brain::MotionIntent, Option<brain::Request>) {
        brain.intent(
            self.observer(),
            &self.party,
            &self.actors,
            &[],
            &[],
            &self.collision,
            &self.world,
            self.geometry,
            &self.tuning,
            tick,
        )
    }

    fn observer_mut(&mut self) -> &mut Actor {
        self.actors
            .iter_mut()
            .find(|a| a.id == OBSERVER)
            .expect("observer")
    }

    fn apply_cast_sample(&mut self, input: ActorIntent) -> Option<(Spell, f32)> {
        let tuning = self.tuning.clone();
        let observer = self.observer_mut();
        observer.aim = input.aim;
        if let Some(spell) = input.selected {
            observer.selected = spell;
        }
        observer.casting(input, &tuning)
    }
}

#[test]
fn head_only_admission_keeps_dragon_grounded_and_shaman_preparing_a_shot() {
    let dragon = SightFixture::head_only(Species::Dragon);
    let mut brain = brain::Brain::for_battle(OBSERVER, dragon.observer().feet, 0x5eed);
    let (intent, _) = dragon.decide(&mut brain, 1);
    assert!(
        !intent.flight,
        "a visible head must not be classified as a lost-target flight search"
    );

    let shaman = SightFixture::head_only(Species::Shaman);
    let mut brain = brain::Brain::for_battle(OBSERVER, shaman.observer().feet, 0x5eed);
    shaman.decide(&mut brain, 1);
    let (intent, request) = shaman.decide(&mut brain, 61);
    assert!(request.is_none(), "one Shaman has no ally needing an aura");
    assert!(
        intent.input.cast_pressed && intent.input.cast_held,
        "after reaction time, own head-only sight permits ordinary precharge"
    );
    assert_eq!(intent.input.selected, Some(Spell::Fireball));
}

#[test]
fn blocked_nearest_cached_endpoint_falls_back_to_farther_admitted_target() {
    let mut fixture = SightFixture::head_only(Species::Shaman);
    let mut far = Actor::spawn(9, Vec3::new(5.0, SKIN, 4.0), Vec3::NEG_X);
    far.species = Species::Goblin;
    far.team = 42;
    fixture.actors.push(far);
    let observed = targeting::observe(
        fixture.observer(),
        &fixture.actors,
        &[],
        &fixture.collision,
        1,
        0.5,
    );
    assert_eq!(
        observed.iter().map(|seen| seen.body.id).collect::<Vec<_>>(),
        vec![NEAR, 9]
    );
    let mut brain = brain::Brain::for_battle(OBSERVER, fixture.observer().feet, 0x5eed);
    fixture.decide(&mut brain, 1);

    // Before the next 12-tick sense, changed terrain hides the recorded head.
    // Actor positions remain unchanged; fallback must use another cached fact.
    fixture
        .world
        .voxels
        .insert(TilePos::new(HexCoord::ORIGIN, 3), SubstanceId(1));
    fixture.world.revision += 1;
    fixture.collision.refresh(&fixture.world, fixture.geometry);
    assert!(!fixture
        .collision
        .sight_clear(fixture.observer().eye(), fixture.actor(NEAR).eye()));
    assert!(!fixture
        .collision
        .sight_clear(fixture.observer().eye(), fixture.actor(NEAR).center()));
    assert!(fixture
        .collision
        .sight_clear(fixture.observer().eye(), fixture.actor(9).center()));
    let (intent, _) = fixture.decide(&mut brain, 2);
    let expected = (fixture.actor(9).center() - fixture.observer().eye()).normalize();
    assert!(
        intent.input.aim.dot(expected) > 0.999,
        "the cached farther visible target must replace the newly covered nearest body"
    );
}

#[test]
fn fully_hidden_live_pose_changes_do_not_change_creature_sight_decisions_or_rng() {
    let mut left = SightFixture::head_only(Species::Shaman);
    let mut right = SightFixture::head_only(Species::Shaman);
    for fixture in [&mut left, &mut right] {
        let mut far = Actor::spawn(9, Vec3::new(5.0, SKIN, 4.0), Vec3::NEG_X);
        far.species = Species::Goblin;
        far.team = 42;
        fixture.actors.push(far);
    }
    let mut a = brain::Brain::for_battle(OBSERVER, left.observer().feet, 0x5eed);
    let mut b = brain::Brain::for_battle(OBSERVER, right.observer().feet, 0x5eed);
    left.decide(&mut a, 1);
    right.decide(&mut b, 1);
    assert_eq!(format!("{a:?}"), format!("{b:?}"));
    // Identical disclosed histories first; then terrain hides the same remembered
    // target. Its different silent live poses must never replace the saved fact.
    for fixture in [&mut left, &mut right] {
        for level in 3..=8 {
            fixture
                .world
                .voxels
                .insert(TilePos::new(HexCoord::ORIGIN, level), SubstanceId(1));
        }
        fixture.world.revision += 1;
        fixture.collision.refresh(&fixture.world, fixture.geometry);
        assert!(fixture
            .collision
            .sight_clear(fixture.observer().eye(), fixture.actor(9).center()));
    }
    let mut precharged = false;
    for tick in 2..=180 {
        let offset = if tick % 2 == 0 { 0.15 } else { -0.15 };
        for (fixture, x, z) in [(&mut left, 3.0, 0.0), (&mut right, 4.0, 0.0)] {
            let hidden = fixture
                .actors
                .iter_mut()
                .find(|a| a.id == NEAR)
                .expect("hidden target");
            hidden.previous_feet = Vec3::new(x, SKIN, z - offset);
            hidden.feet = Vec3::new(x, SKIN, z + offset);
            hidden.body_yaw = offset * 3.0;
            let eye = fixture.observer().eye();
            assert!(!fixture
                .collision
                .sight_clear(eye, fixture.actor(NEAR).eye()));
            assert!(!fixture
                .collision
                .sight_clear(eye, fixture.actor(NEAR).center()));
        }
        let (ia, ra) = left.decide(&mut a, tick);
        let (ib, rb) = right.decide(&mut b, tick);
        assert_eq!(format!("{ia:?}/{ra:?}"), format!("{ib:?}/{rb:?}"));
        // Brain's existing Debug covers its private RNG, retained observations,
        // reaction clocks, and decisions without adding a production getter.
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
        precharged |= ia.input.cast_pressed && ia.input.cast_held;
        let released_a = left
            .apply_cast_sample(ia.input)
            .map(|(spell, speed)| (spell, speed.to_bits()));
        let released_b = right
            .apply_cast_sample(ib.input)
            .map(|(spell, speed)| (spell, speed.to_bits()));
        assert_eq!(released_a, released_b);
    }
    assert!(
        precharged,
        "the comparison must exercise real Shaman charge decisions"
    );
}
