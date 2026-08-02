//! Headless behavior contracts for the game-layer Combat Lab surfaces.
//!
//! This is deliberately one integration binary: linking Bevy once is expensive,
//! while these assertions share the same immutable game-layer observation API.

use bevy::prelude::World;
use hex_assets::{CombatRulesProfile, CombatSettings};
use hex_combat::{
    CombatSummary, DeliveredEffectKind, EncounterOutcome, EncounterResolution, UnitCombatSummary,
};
use hex_core::{GameplayPhase, HexCoord, HexSpan, TilePos, Turn, UnitId};
use hex_game::combat_reports::{
    CombatLabReport, CombatLabReportController, CombatLabReportDeployment, CombatLabReportId,
    CombatLabReportMap, CombatLabReportOrigin, CombatLabReportRosterEntry, CombatLabReportRosters,
    CombatLabReportTermination, SavedCombatLabReport,
};
use hex_game::test_support::{
    fixture_filter_snapshot, gameplay_state_snapshot, live_statistics_snapshot,
    live_statistics_visible, production_lab_ui_transition_snapshot, report_transition_snapshot,
    sandbox_reentry_snapshot, tempo_movement_matrix, wave_seven_fixtures, ReportMode,
};
use hex_units::{Standing, StandsOn};

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
fn report_modes_and_comparison_identity_follow_real_typed_application_transitions() {
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

    let transition = report_transition_snapshot(&current, &saved, CombatLabReportId(17))
        .expect("completed reports must drive the production report adapter");
    assert_eq!(
        transition
            .modes
            .iter()
            .map(|projection| projection.mode)
            .collect::<Vec<_>>(),
        vec![
            ReportMode::Overview,
            ReportMode::Units,
            ReportMode::SpellsEffects,
            ReportMode::Timeline,
            ReportMode::Compare,
        ],
        "each typed intent must settle through the model and game adapter"
    );
    let current_fingerprint = current
        .fingerprint()
        .expect("current fixture must have a canonical fingerprint");
    assert!(transition
        .modes
        .iter()
        .all(|projection| projection.current_fingerprint == current_fingerprint));
    assert!(transition
        .modes
        .iter()
        .all(|projection| projection.units == [UnitId(0), UnitId(1)]));
    let Some(comparison) = transition
        .modes
        .last()
        .and_then(|projection| projection.comparison)
    else {
        panic!("Compare must preserve the independently selected report");
    };
    assert_eq!(comparison.0, CombatLabReportId(17));
    assert_eq!(
        comparison.1,
        saved
            .first()
            .expect("comparison fixture exists")
            .report
            .fingerprint()
            .expect("comparison fixture must have a canonical fingerprint")
    );
    assert_eq!(transition.after_compare.mode, ReportMode::Overview);
    assert_eq!(
        transition.after_compare.comparison,
        Some(comparison),
        "leaving Compare must preserve the independently selected canonical report"
    );
    assert_eq!(
        transition.after_compare.current_fingerprint, current_fingerprint,
        "presentation transitions must not replace the retained current report"
    );
}

#[test]
fn live_drawer_uses_the_canonical_summary_and_has_a_bounded_lifecycle() {
    let report = sample_report(4, 0, 1);
    let snapshot = live_statistics_snapshot(&report.summary);
    assert_eq!(snapshot.rounds, 4);
    assert_eq!(snapshot.turns, 12);
    assert_eq!(snapshot.successful_commands, 11);
    assert_eq!(snapshot.refused_commands, 2);
    assert_eq!(snapshot.movement_distance, 7);
    assert_eq!(snapshot.movement_budget_used, 7);
    assert_eq!(snapshot.channels, 1);
    assert_eq!(snapshot.disables, (4, 1, 3));
    assert_eq!(snapshot.channelled_mana.get("Ember"), Some(&2));
    assert_eq!(snapshot.units, [UnitId(0)]);

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
fn production_lab_toggle_keeps_statistics_below_the_lattice_at_retina_size() {
    let transition = production_lab_ui_transition_snapshot();
    assert_eq!(
        transition.published_own_lattice_cells, 7,
        "the production lattice publisher must project the canonical player lattice"
    );
    assert!(
        transition.expanded_state,
        "the typed production intent must toggle application state"
    );
    assert_eq!(
        transition.expanded.metrics.logical_size,
        bevy::prelude::Vec2::new(1291.0, 721.0),
        "the contract must use Bevy client pixels, excluding native title-bar chrome"
    );
    assert_eq!(
        transition.expanded.metrics.viewport,
        hex_ui::UiViewportClass::Compact
    );
    assert!(
        transition.collapsed.layout_issues().is_empty(),
        "the collapsed production adapter tree must satisfy the structural oracle: {:?}",
        transition.collapsed.layout_issues()
    );
    assert!(
        transition.expanded.layout_issues().is_empty(),
        "the expanded production adapter tree must satisfy the structural oracle: {:?}",
        transition.expanded.layout_issues()
    );

    fn node<'a>(
        snapshot: &'a hex_ui::test_support::UiTreeSnapshot,
        name: &str,
    ) -> &'a hex_ui::test_support::UiNodeObservation {
        snapshot
            .nodes
            .iter()
            .find(|node| node.name == name)
            .unwrap_or_else(|| panic!("production tree is missing {name:?}"))
    }
    let collapsed_lattice = node(&transition.collapsed, "Lattice Readout Stack");
    let expanded_lattice = node(&transition.expanded, "Lattice Readout Stack");
    let drawer = node(&transition.expanded, "Combat Lab Live Statistics Drawer");
    assert_eq!(
        expanded_lattice.parent_name.as_deref(),
        Some("Inspector HUD Region")
    );
    assert_eq!(drawer.parent_name.as_deref(), Some("Inspector HUD Region"));
    assert!(
        expanded_lattice.center.y + expanded_lattice.size.y * 0.5
            <= drawer.center.y - drawer.size.y * 0.5 + 0.5,
        "statistics must follow the persistent lattice in the same scroll flow: lattice={expanded_lattice:?}, drawer={drawer:?}"
    );
    assert!(
        (expanded_lattice.center - collapsed_lattice.center)
            .abs()
            .max_element()
            <= 0.5
            && (expanded_lattice.size - collapsed_lattice.size)
                .abs()
                .max_element()
                <= 0.5,
        "expanding through the production intent must not move or resize the lattice: collapsed={collapsed_lattice:?}, expanded={expanded_lattice:?}"
    );
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

#[test]
fn gameplay_snapshot_reads_exact_canonical_position_and_budget_without_rendering() {
    let mut world = World::new();
    world.insert_resource(GameplayPhase::Active);
    world.spawn((
        UnitId(0),
        StandsOn(Standing {
            pos: position(-3, 2, 4),
            span: HexSpan::new(3.0, 4.0),
        }),
        Turn {
            movement_left: 2,
            acted: true,
        },
    ));

    let snapshot = gameplay_state_snapshot(&mut world);
    assert_eq!(snapshot.phase, Some(GameplayPhase::Active));
    assert_eq!(snapshot.units.len(), 1);
    let Some(unit) = snapshot.units.first() else {
        panic!("the canonical unit must be observable");
    };
    assert_eq!(unit.id, UnitId(0));
    assert_eq!(unit.position, position(-3, 2, 4));
    assert_eq!(unit.turn.map(|turn| turn.movement_left), Some(2));
    assert_eq!(unit.turn.map(|turn| turn.acted), Some(true));
    assert!(snapshot.presented_actions.is_empty());
}

#[test]
fn gameplay_snapshot_names_hud_actions_as_a_presentation_projection() {
    let mut world = World::new();
    let presented = hex_ui::ActionAffordance {
        action: hex_ui::GameplayAction::EndTurn,
        label: "End turn".to_owned(),
        shortcut: Some("Space".to_owned()),
        availability: hex_ui::ActionAvailability::Enabled,
        priority: hex_ui::ActionPriority::Primary,
    };
    world.insert_resource(hex_ui::GameplayHudView {
        phase: GameplayPhase::Active,
        actor: Some(UnitId(0)),
        actor_label: "Hedge Mage".to_owned(),
        round: "Round 1".to_owned(),
        movement_remaining: 2,
        action_remaining: true,
        required_prompt: None,
        actions: vec![presented.clone()],
    });

    let snapshot = gameplay_state_snapshot(&mut world);
    assert_eq!(snapshot.presented_actions, vec![presented]);
}
