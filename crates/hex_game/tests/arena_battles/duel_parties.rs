//! Player-selected Duel parties through the real world publisher and 120 Hz loop.

use super::*;
use hex_arena::{ArenaOutcome, CreatureAbility, Species};
use std::collections::BTreeSet;

fn player_setup(preset: BattlePreset) -> ArenaBattleSetup {
    ArenaBattleSetup {
        player_recipe: Some(preset),
        ..Default::default()
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
                    "feet":actor.feet.to_array(),"hp":actor.hp,
                    "prisms":actor.body_hex_prisms().map(|part|serde_json::json!({
                        "offset":part.offset.to_array(),"height":part.height
                    })).collect::<Vec<_>>()
                })
            })
            .collect(),
    )
}

fn assert_roster(fixture: &App, preset: BattlePreset) {
    let session = fixture.world().resource::<ArenaSession>();
    let terrain = fixture.world().resource::<ArenaTerrainView>();
    let geometry = *fixture.world().resource::<ArenaVoxelGeometry>();
    assert_eq!(session.human_actor_id(), Some(0));
    assert_eq!(session.accepted_battle_setup(), &player_setup(preset));
    assert!(!session.is_finished(), "{preset:?}: {}", session.notice);
    assert!(
        session.battle_summary().is_none(),
        "player mode has no observer outcome"
    );
    let human = session.actors.first().expect("one physical human");
    assert_eq!((human.id, human.species), (0, Species::Human));
    assert_eq!(
        session
            .actors
            .iter()
            .skip(1)
            .map(|a| a.species)
            .collect::<Vec<_>>(),
        preset.members(),
        "{preset:?} must admit the complete requested roster"
    );
    assert_eq!(
        session
            .actors
            .iter()
            .map(|a| a.id)
            .collect::<BTreeSet<_>>()
            .len(),
        session.actors.len()
    );
    for actor in &session.actors {
        // Admission is above ground even for Worm. Its later burrowing pose has
        // a different volume contract and is not tested with this ordinary query.
        assert!(
            session.actor_pose_valid(actor.id, terrain, geometry),
            "{preset:?}: invalid initial actor{} at {:?}",
            actor.id,
            actor.feet
        );
        if actor.id != 0 {
            assert_ne!(actor.team, human.team);
        }
    }
    if preset == BattlePreset::Shadow {
        assert!(
            session.parties().is_empty(),
            "explicit Shadow retains the original bot loop"
        );
    } else {
        assert_eq!(session.parties().len(), 1);
        assert_eq!(
            session.encounter_summary().living_enemies,
            preset.members().len()
        );
    }
}

#[test]
fn duel_player_menu_rosters_finish_only_after_all_hostiles_and_reset_to_the_same_world() {
    for preset in BattlePreset::PLAYER {
        let mut fixture = app(ArenaMap::Duel, player_setup(preset));
        assert_roster(&fixture, preset);
        fixture
            .world_mut()
            .resource_mut::<ArenaSession>()
            .bot_enabled = false;
        // Establish the disabled-controller reset snapshot with the same ordinary
        // initialization tick used by the app's ready boundary.
        fixture.world_mut().resource_mut::<ArenaReset>().generation += 1;
        fixture.world_mut().run_schedule(ArenaTick);
        let initial_actors = actor_snapshot(&fixture);
        let original = fixture
            .world()
            .resource::<ArenaTerrainView>()
            .voxels
            .clone();
        let removable = original
            .keys()
            .find(|position| position.level > 0)
            .copied()
            .expect("destructible Duel terrain");
        fixture
            .world_mut()
            .write_message(TerrainEdit::Clear { pos: removable });
        fixture.world_mut().run_schedule(ArenaTick);
        assert!(!fixture
            .world()
            .resource::<ArenaTerrainView>()
            .voxels
            .contains_key(&removable));

        let last = fixture
            .world()
            .resource::<ArenaSession>()
            .actors
            .last()
            .expect("hostile roster")
            .id;
        if preset.members().len() > 1 {
            for actor in &mut fixture.world_mut().resource_mut::<ArenaSession>().actors {
                if actor.id != 0 && actor.id != last {
                    actor.hp = 0.0;
                }
            }
            fixture.world_mut().run_schedule(ArenaTick);
            let session = fixture.world().resource::<ArenaSession>();
            assert!(
                !session.is_finished(),
                "{preset:?}: one hostile is still alive"
            );
            assert_eq!(session.encounter_summary().living_enemies, 1);
        }
        fixture
            .world_mut()
            .resource_mut::<ArenaSession>()
            .actors
            .iter_mut()
            .find(|actor| actor.id == last)
            .expect("last hostile")
            .hp = 0.0;
        fixture.world_mut().run_schedule(ArenaTick);
        let session = fixture.world().resource::<ArenaSession>();
        assert_eq!(session.outcome, Some(ArenaOutcome::Winner(0)), "{preset:?}");
        let terminal_tick = session.tick;
        fixture.world_mut().run_schedule(ArenaTick);
        assert_eq!(
            fixture.world().resource::<ArenaSession>().tick,
            terminal_tick
        );

        fixture.world_mut().resource_mut::<ArenaReset>().generation += 1;
        fixture.world_mut().run_schedule(ArenaTick);
        assert_roster(&fixture, preset);
        assert_eq!(actor_snapshot(&fixture), initial_actors);
        assert_eq!(
            fixture.world().resource::<ArenaTerrainView>().voxels,
            original
        );
        assert!(fixture
            .world()
            .resource::<ArenaSession>()
            .projectiles
            .is_empty());

        *fixture.world_mut().resource_mut::<ArenaBattleSetup>() = ArenaBattleSetup::default();
        fixture.world_mut().run_schedule(ArenaTick);
        assert_eq!(
            fixture
                .world()
                .resource::<ArenaSession>()
                .accepted_battle_setup()
                .player_recipe,
            Some(preset)
        );
        fixture.world_mut().resource_mut::<ArenaReset>().generation += 1;
        fixture.world_mut().run_schedule(ArenaTick);
        let session = fixture.world().resource::<ArenaSession>();
        assert_eq!(session.accepted_battle_setup().player_recipe, None);
        assert_eq!(
            session.actors.iter().map(|a| a.species).collect::<Vec<_>>(),
            [Species::Human, Species::Shadow]
        );
        assert!(session.parties().is_empty());
        assert!(!session.is_finished());
    }
}

#[test]
fn custom_duel_parties_take_real_offensive_actions_after_observing_the_player() {
    let offense = [
        CreatureAbility::Fireball,
        CreatureAbility::FireCone,
        CreatureAbility::Bite,
        CreatureAbility::Swipe,
        CreatureAbility::GolemSlam,
        CreatureAbility::GolemLaser,
        CreatureAbility::WispEmber,
        CreatureAbility::WormBoulder,
    ];
    for preset in BattlePreset::PLAYER
        .into_iter()
        .filter(|p| *p != BattlePreset::Shadow)
    {
        let mut fixture = app(ArenaMap::Duel, player_setup(preset));
        assert_roster(&fixture, preset);
        let feet = {
            let session = fixture.world().resource::<ArenaSession>();
            let terrain = fixture.world().resource::<ArenaTerrainView>();
            let geometry = *fixture.world().resource::<ArenaVoxelGeometry>();
            let target = session.actors.get(1).expect("party representative");
            if preset == BattlePreset::Golem {
                let human = session.actors.first().expect("human");
                let bearing = (human.feet - target.feet)
                    .with_y(0.0)
                    .normalize_or(Vec3::NEG_X);
                // Keep the deliberate medium-range gap intact: use an observed,
                // supported point inside activation and the existing slam range.
                session.visible_supported_actor_pose(
                    0,
                    target.id,
                    target.feet + bearing * 5.0,
                    terrain,
                    geometry,
                )
            } else {
                session.synthetic_combat_target_pose(0, target.id, None, terrain, geometry)
            }
            .expect("actual dry, body-clear, visible approach on Duel")
        };
        {
            let mut session = fixture.world_mut().resource_mut::<ArenaSession>();
            session.actors.first_mut().expect("human").feet = feet;
            session.bot_enabled = true;
        }
        let mut observed = BTreeSet::new();
        let mut acted = false;
        for _ in 0..1800 {
            fixture.world_mut().run_schedule(ArenaTick);
            let session = fixture.world().resource::<ArenaSession>();
            for decision in session.creature_decisions() {
                // Player-mode own sight is specifically the living human. The
                // optional target ID is populated only for copied-body forecasts;
                // ordinary Dragon/Goblin/Shaman/Golem observations omit that ID.
                if decision.own_sight
                    && decision.target.is_none_or(|id| id == 0)
                    && decision
                        .observation_tick
                        .is_some_and(|tick| tick <= session.tick)
                {
                    observed.insert(decision.id);
                }
            }
            if preset == BattlePreset::Worm {
                // This preset has exactly one member (asserted above), so its
                // party's direct sight can only come from this exposed Worm.
                // Worm uses its own controller rather than creature_decisions.
                let worm = session.actors.get(1).expect("the single Worm");
                if session.party_knowledge().iter().any(|knowledge| {
                    Some(knowledge.id) == worm.party
                        && knowledge.source == "sight"
                        && knowledge.tick.is_some_and(|tick| tick <= session.tick)
                }) {
                    observed.insert(worm.id);
                }
            }
            acted = session.encounter_stats().iter().any(|actor| {
                observed.contains(&actor.id)
                    && offense.iter().any(|ability| {
                        actor
                            .abilities
                            .get(ability.index())
                            .is_some_and(|count| *count > 0)
                    })
            });
            if acted || session.is_finished() {
                break;
            }
        }
        assert!(
            acted,
            "{preset:?} must observe the player and perform a real offensive activation: {}",
            serde_json::json!({
                "approach":feet.to_array(),"observed_actor_ids":observed,
                "actors":actor_snapshot(&fixture),
                "tick":fixture.world().resource::<ArenaSession>().tick,
                "notice":fixture.world().resource::<ArenaSession>().notice,
                "decisions":fixture.world().resource::<ArenaSession>().creature_decisions(),
                "knowledge":fixture.world().resource::<ArenaSession>().party_knowledge(),
                "stats":fixture.world().resource::<ArenaSession>().encounter_stats()
            })
        );
    }
}

#[test]
fn explicit_shadow_keeps_the_original_loop_and_duel_player_never_regenerates() {
    let mut original = app(ArenaMap::Duel, ArenaBattleSetup::default());
    let mut explicit = app(ArenaMap::Duel, player_setup(BattlePreset::Shadow));
    for _ in 0..240 {
        original.world_mut().run_schedule(ArenaTick);
        explicit.world_mut().run_schedule(ArenaTick);
        assert_eq!(actor_snapshot(&original), actor_snapshot(&explicit));
        assert_eq!(
            serde_json::to_value(original.world().resource::<ArenaSession>().round_summary())
                .expect("classic summary"),
            serde_json::to_value(explicit.world().resource::<ArenaSession>().round_summary())
                .expect("explicit summary")
        );
    }
    for preset in [
        None,
        Some(BattlePreset::Shadow),
        Some(BattlePreset::Goblins),
    ] {
        let mut fixture = app(
            ArenaMap::Duel,
            ArenaBattleSetup {
                player_recipe: preset,
                ..Default::default()
            },
        );
        {
            let mut session = fixture.world_mut().resource_mut::<ArenaSession>();
            session.bot_enabled = false;
            session.actors.first_mut().expect("human").hp = 50.0;
        }
        // Ten quiet seconds exceed the authored eight-second regeneration delay.
        for _ in 0..1200 {
            fixture.world_mut().run_schedule(ArenaTick);
        }
        let session = fixture.world().resource::<ArenaSession>();
        assert!(!session.is_finished(), "quiet Duel {preset:?}");
        assert_eq!(
            session.actors.first().expect("human").hp.to_bits(),
            50.0_f32.to_bits(),
            "Duel {preset:?} must retain its no-regeneration rule"
        );
    }
}
