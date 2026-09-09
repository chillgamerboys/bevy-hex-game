//! Duel uses the same ready-screen party choices as Fort.
use super::*;
use hex_arena::{ArenaBattleSetup, BattlePreset, Species};

#[test]
fn duel_launch_defaults_to_shadow_and_accepts_all_player_recipes() {
    let selection = launch_selection(Some("duel"), None).expect("default Duel");
    assert_eq!(selection.encounter, ArenaEncounter::Shadow);
    assert_eq!(
        player_preset(selection, &ArenaBattleSetup::default()),
        BattlePreset::Shadow
    );
    for preset in BattlePreset::ALL {
        let selection = launch_selection(Some("duel"), Some(preset.slug())).expect("Duel party");
        let mut setup = ArenaBattleSetup::default();
        apply_player_recipe(&mut setup, selection, Some(preset.slug())).expect("accepted recipe");
        assert_eq!(player_preset(selection, &setup), preset);
        assert_eq!(
            setup.player_recipe,
            (preset != BattlePreset::Shadow).then_some(preset)
        );
    }
}

#[test]
fn duel_menu_switches_actual_parties_and_preserves_selection_across_restart_and_fort() {
    let (mut fixture, _) = menu_app();
    for preset in BattlePreset::PLAYER {
        let action = match preset {
            BattlePreset::Shadow => hud::Action::Encounter(ArenaEncounter::Shadow),
            BattlePreset::Dragon => hud::Action::Encounter(ArenaEncounter::Dragon),
            BattlePreset::Goblins => hud::Action::Encounter(ArenaEncounter::Goblins),
            BattlePreset::ShamanParty => hud::Action::Encounter(ArenaEncounter::ShamanParty),
            _ => hud::Action::PlayerRecipe(preset),
        };
        press_action(&mut fixture, action);
        let assert_party = |app: &App| {
            let session = app.world().resource::<ArenaSession>();
            assert!(!session.is_finished(), "{preset:?}: {}", session.notice);
            assert_eq!(
                session.actors.first().expect("human body").species,
                Species::Human
            );
            assert_eq!(
                session
                    .actors
                    .iter()
                    .skip(1)
                    .map(|a| a.species)
                    .collect::<Vec<_>>(),
                preset.members()
            );
            assert_eq!(
                player_preset(
                    *app.world().resource::<ArenaSelection>(),
                    app.world().resource::<ArenaBattleSetup>()
                ),
                preset
            );
        };
        assert_party(&fixture);
        let generation = fixture.world().resource::<ArenaReset>().generation;
        press_action(&mut fixture, action);
        assert_eq!(
            fixture.world().resource::<ArenaReset>().generation,
            generation,
            "reselecting is a no-op"
        );
        press_action(&mut fixture, hud::Action::Start);
        tap_key(&mut fixture, KeyCode::Escape);
        let rejected = if preset == BattlePreset::Dragon {
            ArenaEncounter::Shadow
        } else {
            ArenaEncounter::Dragon
        };
        press_action(&mut fixture, hud::Action::Encounter(rejected));
        assert_party(&fixture);
        tap_key(&mut fixture, KeyCode::KeyR);
        assert_party(&fixture);
        assert!(!fixture.world().resource::<ViewState>().started);
        let input = &fixture.world().resource::<ArenaInput>().human;
        assert!(!input.cast_pressed && !input.cast_held && !input.cast_released);
        press_action(&mut fixture, hud::Action::Map(ArenaMap::Fort));
        assert_party(&fixture);
        press_action(&mut fixture, hud::Action::Map(ArenaMap::Duel));
        assert_party(&fixture);
    }
}
