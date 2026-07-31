//! Headless behavior contracts for the game-layer Combat Lab surfaces.
//!
//! This is deliberately one integration binary: linking Bevy once is expensive,
//! while these assertions share the same immutable game-layer observation API.

use std::collections::BTreeSet;

use hex_assets::{CombatRulesProfile, CombatSettings};
use hex_combat::{
    CombatSummary, DeliveredEffectKind, EncounterOutcome, EncounterResolution, UnitCombatSummary,
};
use hex_core::{GameplayPhase, HexCoord, TilePos, UnitId};
use hex_game::combat_reports::{
    CombatLabReport, CombatLabReportController, CombatLabReportDeployment, CombatLabReportId,
    CombatLabReportMap, CombatLabReportOrigin, CombatLabReportRosterEntry, CombatLabReportRosters,
    CombatLabReportTermination, SavedCombatLabReport,
};
use hex_game::test_support::{
    fixture_filter_snapshot, live_statistics_text, live_statistics_visible, report_text,
    sandbox_reentry_snapshot, tempo_movement_matrix, wave_seven_fixtures, ReportMode,
};

fn position(q: i32, r: i32, level: i32) -> TilePos {
    TilePos::new(HexCoord::from_axial(q, r), level)
}

fn sample_report(rounds: u32, player_id: u64, hostile_id: u64) -> CombatLabReport {
    let shipped = CombatSettings::default();
    let mut summary = CombatSummary::default();
    summary.rounds = rounds;
    summary.turns = rounds.saturating_mul(3);
    summary.successful_commands = 11;
    summary.refused_commands = 2;
    summary.movement_distance = 7;
    summary.movement_budget_used = 7;
    summary.channels = 1;
    summary.raw_disables = 4;
    summary.prevented_disables = 1;
    summary.applied_disables = 3;
    summary.no_progress_max = 2;
    summary.outcome = Some(EncounterOutcome::Victory);
    summary.casts_by_spell.insert("Spark".to_owned(), 1);
    summary
        .delivered_effects
        .insert(DeliveredEffectKind::Disable, 3);
    summary.channelled_mana.insert("Ember".to_owned(), 2);
    summary.units.insert(
        UnitId(player_id),
        UnitCombatSummary {
            turns: rounds,
            movement_distance: 7,
            channels: 1,
            ..Default::default()
        },
    );
    let result = CombatLabReport::new(
        CombatRulesProfile::shipped(&shipped),
        CombatLabReportOrigin::Sandbox,
        CombatLabReportMap {
            catalog_id: "flat-arena".to_owned(),
            scenario: "Ability Lab".to_owned(),
            resolved_seed: None,
        },
        77,
        CombatLabReportRosters {
            players: vec![CombatLabReportRosterEntry {
                unit_id: player_id,
                archetype: "hedge-mage".to_owned(),
                display_name: "Hedge Mage".to_owned(),
                controller: CombatLabReportController::Human,
            }],
            hostiles: vec![CombatLabReportRosterEntry {
                unit_id: hostile_id,
                archetype: "raider".to_owned(),
                display_name: "Raider".to_owned(),
                controller: CombatLabReportController::BaselineAi,
            }],
        },
        CombatLabReportDeployment {
            players: vec![position(-2, 1, 1)],
            hostiles: vec![position(3, -1, 2)],
        },
        CombatLabReportTermination::Outcome(EncounterOutcome::Victory),
        summary,
    );
    match result {
        Ok(report) => report,
        Err(_) => std::process::abort(),
    }
}

#[test]
fn fixture_search_filters_in_place_and_clear_restores_every_card() {
    let snapshot = fixture_filter_snapshot("tempo");
    assert_eq!(snapshot.visible, vec!["tempo-matrix"]);
    assert_eq!(snapshot.visible_after_clear.len(), 7);
    assert!(
        snapshot.input_survived,
        "filtering must not rebuild and replace the focused editable entity"
    );
}

#[test]
fn report_modes_are_functional_canonical_and_independently_selectable() {
    let current = sample_report(8, 0, 1);
    let older = sample_report(2, 2, 3);
    let selected = sample_report(6, 4, 5);
    let saved = [
        SavedCombatLabReport {
            id: CombatLabReportId(17),
            label: "older".to_owned(),
            notes: String::new(),
            report: older,
        },
        SavedCombatLabReport {
            id: CombatLabReportId(23),
            label: "selected".to_owned(),
            notes: String::new(),
            report: selected,
        },
    ];

    let projections = [
        report_text(&current, ReportMode::Overview, &saved, None),
        report_text(&current, ReportMode::Units, &saved, None),
        report_text(&current, ReportMode::SpellsEffects, &saved, None),
        report_text(&current, ReportMode::Timeline, &saved, None),
        report_text(
            &current,
            ReportMode::Compare,
            &saved,
            Some(CombatLabReportId(17)),
        ),
    ];
    assert_eq!(
        projections.iter().collect::<BTreeSet<_>>().len(),
        projections.len(),
        "every tab must project a distinct canonical concern"
    );
    assert!(projections.first().is_some_and(|text| {
        text.contains("Rounds 8") && text.contains("11 successful / 2 refused")
    }));
    assert!(projections.get(1).is_some_and(|text| {
        text.contains("UNITS") && text.contains("Hedge Mage") && text.contains("#0")
    }));
    assert!(projections
        .get(2)
        .is_some_and(|text| text.contains("Spark 1") && text.contains("Ember 2")));
    assert!(projections
        .get(3)
        .is_some_and(|text| text.contains("TIMELINE")));
    assert!(projections.get(4).is_some_and(|text| {
        text.contains("REPORT 17") && text.contains("rounds +6") && !text.contains("REPORT 23")
    }));
}

#[test]
fn live_drawer_uses_the_canonical_summary_and_has_a_bounded_lifecycle() {
    let report = sample_report(4, 0, 1);
    let label = live_statistics_text(&report.summary);
    for expected in [
        "Round 4",
        "Turns 12",
        "11 successful / 2 refused",
        "7 distance / 7 budget used",
        "Channel 1",
        "4 raw / 1 prevented / 3 applied",
        "Ember 2",
        "#0",
    ] {
        assert!(
            label.contains(expected),
            "missing canonical fact {expected:?}"
        );
    }

    assert!(live_statistics_visible(
        GameplayPhase::Active,
        Some(EncounterResolution(None))
    ));
    assert!(!live_statistics_visible(
        GameplayPhase::Deployment,
        Some(EncounterResolution(None))
    ));
    assert!(!live_statistics_visible(
        GameplayPhase::Active,
        Some(EncounterResolution(Some(EncounterOutcome::Victory)))
    ));
}

#[test]
fn retry_copy_and_tune_restore_exact_frozen_launch_identity_once() {
    let report = sample_report(8, 0, 1);
    let expected_profile = report.profile.preset;
    let expected_players = report.deployment.players.clone();
    let expected_hostiles = report.deployment.hostiles.clone();

    let restored = sandbox_reentry_snapshot(report);
    assert_eq!(restored.map, "flat-arena");
    assert_eq!(restored.preset, expected_profile);
    assert_eq!(restored.players, ["hedge-mage"]);
    assert_eq!(restored.hostiles, ["raider"]);
    assert_eq!(restored.player_positions, expected_players);
    assert_eq!(restored.hostile_positions, expected_hostiles);
    assert!(
        restored.request_consumed,
        "re-entry handoffs must be one-shot so cold launch cannot inherit stale identity"
    );
}

#[test]
fn canonical_zero_unit_id_is_valid_while_duplicate_nonzero_ids_are_rejected() {
    let shipped = CombatSettings::default();
    let zero = sample_report(1, 0, 1);
    assert!(zero.validate(&shipped).is_ok(), "UnitId(0) is canonical");

    let mut duplicate = sample_report(1, 7, 8);
    let Some(hostile) = duplicate.rosters.hostiles.first_mut() else {
        panic!("sample report has one hostile");
    };
    hostile.unit_id = 7;
    assert!(duplicate
        .validate(&shipped)
        .is_err_and(|error| error.contains("unique")));
}

#[test]
fn wave_seven_fixtures_own_three_by_three_rosters_and_all_tempo_profiles() {
    let fixtures = wave_seven_fixtures();
    assert_eq!(
        fixtures
            .iter()
            .map(|fixture| fixture.id)
            .collect::<Vec<_>>(),
        ["occupancy-matrix", "channel-attrition", "tempo-matrix"]
    );
    assert!(fixtures
        .iter()
        .all(|fixture| fixture.players == 3 && fixture.hostiles == 3));
    assert_eq!(tempo_movement_matrix(&CombatSettings::default()), [4, 2, 3]);
}
