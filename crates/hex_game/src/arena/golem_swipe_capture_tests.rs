//! Actual Fort capture replay: inputs create Shield, then ordinary Golem policy swipes.

use super::*;
use hex_core::arena::{ArenaEncounter, ArenaSelection, ArenaTick};
use hex_test_app::HeadlessAppBuilder;

#[test]
fn ordinary_fort_shield_inputs_elicit_live_stone_swipe_before_human_death() {
    for view in ["encounter-golem-swipe-windup", "encounter-golem-swipe"] {
        let mut builder = HeadlessAppBuilder::new()
            .with_minimal_plugins()
            .with_fixed_step(std::time::Duration::from_secs_f64(1.0 / 60.0));
        builder
            .app_mut()
            .insert_resource(ArenaSelection {
                map: ArenaMap::Fort,
                encounter: ArenaEncounter::Dragon,
            })
            .insert_resource(hex_arena::ArenaBattleSetup {
                player_recipe: Some(hex_arena::BattlePreset::Golem),
                ..default()
            })
            .insert_resource(ViewState {
                started: true,
                paused: false,
                capture: Some(std::path::PathBuf::from("unused-swipe-capture-test.png")),
                capture_view: view.into(),
                ..default()
            })
            .add_plugins((hex_map::arena::plugin, hex_arena::plugin))
            .add_systems(Update, super::super::drive_simulation);
        let mut app = builder.build();
        for _ in 0..1800 {
            app.update();
            if app
                .world()
                .resource::<ViewState>()
                .capture_event_frame
                .is_some()
                || app.world().resource::<ArenaSession>().is_finished()
            {
                break;
            }
        }
        let session = app.world().resource::<ArenaSession>();
        let state = app.world().resource::<ViewState>();
        assert!(state.capture_event_frame.is_some(),
            "ordinary {view} did not reach phase: tick={}, route={}, actors={:?}, decisions={:?}, last inputs={:?}",
            session.tick, state.capture_route_step,
            session.actors.iter().map(|a| (a.id, a.feet, a.hp, a.attack_state())).collect::<Vec<_>>(),
            session.creature_decisions(), state.capture_inputs.iter().rev().take(6).collect::<Vec<_>>());
        assert!(super::super::golem::phase_actor(session, view).is_some());
        assert!(session.actors.first().is_some_and(|a| a.hp > 0.0));
        assert!(fort_approach_complete(state.capture_route_step));
        assert!(
            state.capture_inputs.iter().any(|(_, input)| {
                input.selected == Some(Spell::Shield) && input.cast_pressed && input.cast_released
            }),
            "recovery obstacle must originate from an ordinary Shield release"
        );
        assert!(state
            .capture_inputs
            .iter()
            .all(|(_, input)| !input.cast_pressed
                || (input.selected == Some(Spell::Shield)
                    && input.cast_released
                    && !input.cast_held)));
        assert!(app.world().resource::<ArenaTerrainView>().revision > 1);
        // One further authoritative publication must preserve the real result;
        // the test never assigns HP, poses, terrain, abilities, cooldowns or AI.
        app.world_mut().run_schedule(ArenaTick);
    }
}
