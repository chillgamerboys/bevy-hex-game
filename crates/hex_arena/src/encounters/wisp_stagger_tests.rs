//! Independent opening-volley timing through ordinary battle advancement.

use super::*;
use hex_core::arena::ArenaDeploymentRegion;
use std::collections::BTreeSet;

struct VolleyFixture {
    session: ArenaSession,
    view: ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    materials: ArenaMaterials,
    tuning: ArenaTuning,
}

impl VolleyFixture {
    fn new(preset: BattlePreset, spread: f32) -> Self {
        let (mut session, mut view, geometry, materials, mut tuning) =
            fixture(ArenaEncounter::Dragon);
        view.battle_deployment = Some([-6, 6].map(|q| {
            let preferred = TilePos::new(HexCoord::from_axial(q, 0), 0);
            ArenaDeploymentRegion {
                preferred,
                surfaces: preferred
                    .coord
                    .within_radius(1)
                    .into_iter()
                    .map(|coord| TilePos::new(coord, 0))
                    .collect(),
            }
        }));
        // Keep a genuine visible moving opponent without making volley timing
        // depend on deaths, melee access, or changing the Wisp controller.
        tuning.encounters.goblin_hp = 1_000.0;
        tuning.encounters.goblin_walk = 0.01;
        tuning.encounters.goblin_run = 0.01;
        tuning.encounters.wisp_initial_volley_spread = spread;
        tuning.validate().expect("valid timing fixture");
        session.bot_enabled = true;
        session.reset_with_setup(
            10,
            &view,
            geometry,
            &ArenaBattleSetup::spectator(preset, BattlePreset::Goblin, 1),
        );
        Self {
            session,
            view,
            geometry,
            materials,
            tuning,
        }
    }

    fn step(&mut self) {
        self.session.advance(
            ActorIntent::default(),
            &self.view,
            self.geometry,
            self.materials,
            &self.tuning,
        );
        assert!(!self.session.is_finished(), "timing target remains alive");
    }

    fn opening_releases(&mut self, count: usize) -> BTreeMap<u8, u64> {
        let mut releases = BTreeMap::new();
        for _ in 0..150 {
            self.step();
            for shot in &self.session.projectiles {
                if shot.source_ability() == Some(CreatureAbility::WispEmber)
                    && shot.age.abs() < SKIN
                {
                    releases.entry(shot.owner).or_insert(self.session.tick);
                }
            }
            if releases.len() == count {
                return releases;
            }
        }
        panic!("missing opening releases: {releases:?}");
    }

    fn released(&self, id: u8) -> u32 {
        self.session
            .encounter
            .ability_counts
            .get(&id)
            .and_then(|counts| counts.get(CreatureAbility::WispEmber.index()))
            .copied()
            .unwrap_or(0)
    }
}

#[test]
fn twelve_wisp_openings_have_distinct_phases_across_point_six_seconds_and_reset_replays() {
    let mut f = VolleyFixture::new(BattlePreset::Wisps12, 0.6);
    let first = f.opening_releases(12);
    let ticks = first.values().copied().collect::<Vec<_>>();
    assert!(
        ticks
            .windows(2)
            .all(|pair| { pair.first().zip(pair.last()).is_some_and(|(a, b)| a < b) }),
        "stable roster slots receive distinct increasing release ticks"
    );
    assert_eq!(ticks.first().copied(), Some(42));
    assert_eq!(ticks.last().copied(), Some(114));
    assert_eq!(first.values().copied().collect::<BTreeSet<_>>().len(), 12);
    for id in first.keys() {
        assert_eq!(f.released(*id), 1);
    }
    let setup = f.session.accepted_battle_setup().clone();
    f.session.reset_with_setup(11, &f.view, f.geometry, &setup);
    assert_eq!(f.opening_releases(12), first);
}

#[test]
fn single_wisp_keeps_its_tick_forty_two_release_with_or_without_opening_spread() {
    for spread in [0.0, 0.6] {
        let mut f = VolleyFixture::new(BattlePreset::Wisp, spread);
        assert_eq!(
            f.tuning.encounters.wisp_ember_windup.to_bits(),
            0.35_f32.to_bits()
        );
        assert_eq!(
            f.tuning.encounters.wisp_ember_cooldown.to_bits(),
            2.0_f32.to_bits()
        );
        f.step();
        assert_eq!(f.session.tick, 1);
        assert!(f
            .session
            .actors
            .first()
            .and_then(Actor::attack_state)
            .is_some_and(|state| state.kind == CreatureAbility::WispEmber
                && state.phase == AttackPhase::Windup));
        for _ in 1..41 {
            f.step();
        }
        assert_eq!(f.released(0), 0);
        assert!(f.session.projectiles.is_empty());
        f.step();
        assert_eq!(f.session.tick, 42);
        assert_eq!(f.released(0), 1);
        assert!(f.session.projectiles.iter().any(|shot| shot.owner == 0
            && shot.source_ability() == Some(CreatureAbility::WispEmber)
            && shot.age.abs() < SKIN));
    }
}

#[test]
fn hidden_wait_does_not_spend_the_last_slot_phase_or_begin_its_windup_early() {
    let mut f = VolleyFixture::new(BattlePreset::Wisps2, 0.6);
    for q in -1..=1 {
        for r in -20..=20 {
            for level in 1..=24 {
                f.view.voxels.insert(
                    TilePos::new(HexCoord::from_axial(q, r), level),
                    f.materials.stone,
                );
            }
        }
    }
    f.view.revision += 1;
    for _ in 0..180 {
        f.step();
    }
    for id in [0, 1] {
        let actor = f.session.actors.iter().find(|a| a.id == id).expect("Wisp");
        assert!(actor.attack_state().is_none());
        assert_eq!(f.released(id), 0);
        assert!(f
            .session
            .encounter
            .brains
            .get(&id)
            .expect("brain")
            .ready(CreatureAbility::WispEmber));
    }
    f.view.voxels.retain(|pos, _| pos.level == 0);
    f.view.revision += 1;
    let mut first_start = None;
    let mut last_start = None;
    for _ in 0..100 {
        f.step();
        let active = |id| {
            f.session
                .actors
                .iter()
                .find(|a| a.id == id)
                .and_then(Actor::attack_state)
                .is_some_and(|a| {
                    a.kind == CreatureAbility::WispEmber && a.phase == AttackPhase::Windup
                })
        };
        if active(0) {
            first_start.get_or_insert(f.session.tick);
        }
        if active(1) {
            last_start = Some(f.session.tick);
            break;
        }
        assert_eq!(f.released(1), 0);
        assert!(f
            .session
            .encounter
            .brains
            .get(&1)
            .expect("last brain")
            .ready(CreatureAbility::WispEmber));
        if let Some(first) = first_start {
            assert!(
                f.session.tick < first + 72,
                "last phase must start at its deadline"
            );
        }
    }
    let first = first_start.expect("first own-visible slot begins");
    let last = last_start.expect("last own-visible slot begins");
    assert!(
        first > 180,
        "phase clock starts after actual sight admission"
    );
    assert_eq!(last - first, 72);
    assert_eq!(
        f.released(1),
        0,
        "opening phase starts the ordinary windup, not an instant shot"
    );
    for _ in 0..41 {
        f.step();
    }
    assert_eq!(f.session.tick, last + 41);
    assert_eq!(f.released(1), 1);
}
