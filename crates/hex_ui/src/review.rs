//! Default-off authored presentation states for bounded visual review.

use bevy::prelude::*;

use crate::{
    CastingPanelView, GameplayHudView, GameplayLatticesView, LabStatisticsView, OutcomeReportView,
};

#[cfg(any(feature = "visual-review", feature = "test-support"))]
use crate::{
    ActionAffordance, ActionAvailability, ActionPriority, CastingAimView, CastingPanelContentView,
    CastingSpellView, CellInteraction, DecisionChoiceView, GameplayAction, LatticeCellView,
    OutcomeAction, OutcomeActionView, OutcomeCompareChoiceView, OwnLatticeView,
};

#[derive(Resource, Default)]
pub(crate) struct UiReviewPresentation {
    pub(crate) hud: Option<GameplayHudView>,
    pub(crate) casting: Option<CastingPanelView>,
    pub(crate) lattices: Option<GameplayLatticesView>,
    pub(crate) statistics: Option<LabStatisticsView>,
    pub(crate) outcome: Option<OutcomeReportView>,
}

#[cfg(any(feature = "visual-review", feature = "test-support"))]
/// Installs one authored presentation-only fixture without mutating gameplay state.
///
/// The visual-walk feature is the only caller. Logical behavior remains covered by
/// canonical state snapshots and deterministic simulation.
pub fn apply_ui_review_fixture(commands: &mut Commands, name: &str) -> Result<(), String> {
    let mut review = UiReviewPresentation::default();
    match name {
        "clear" => {}
        "normal-gameplay" => {
            review.hud = Some(GameplayHudView {
                phase: hex_core::GameplayPhase::Active,
                actor: Some(hex_core::UnitId(0)),
                actor_label: "Hedge Mage · Player".to_owned(),
                round: "Exploring".to_owned(),
                movement_remaining: 0,
                action_remaining: true,
                required_prompt: None,
                actions: vec![
                    ActionAffordance {
                        action: GameplayAction::Rest,
                        label: "Rest".to_owned(),
                        shortcut: Some("R".to_owned()),
                        availability: ActionAvailability::Enabled,
                        priority: ActionPriority::Primary,
                    },
                    ActionAffordance {
                        action: GameplayAction::Pause,
                        label: "Pause".to_owned(),
                        shortcut: Some("Escape".to_owned()),
                        availability: ActionAvailability::Enabled,
                        priority: ActionPriority::Primary,
                    },
                ],
            });
            review.statistics = Some(LabStatisticsView::default());
        }
        "player-turn-max" => {
            review.hud = Some(ordinary_hud());
            review.statistics = Some(LabStatisticsView::default());
        }
        "hostile-turn" => {
            review.hud = Some(GameplayHudView {
                phase: hex_core::GameplayPhase::Active,
                actor: Some(hex_core::UnitId(1)),
                actor_label: "Raider · Hostile".to_owned(),
                round: "Round 4 · enemy turn".to_owned(),
                movement_remaining: 4,
                action_remaining: true,
                required_prompt: Some("WAIT · Raider is choosing its action.".to_owned()),
                actions: Vec::new(),
            });
            review.statistics = Some(LabStatisticsView::default());
        }
        "casting-list" => {
            review.hud = Some(ordinary_hud());
            review.casting = Some(CastingPanelView {
                visible: true,
                content: CastingPanelContentView::Spells {
                    unavailable: None,
                    spells: production_spell_catalog(),
                    aiming: None,
                },
            });
            review.statistics = Some(LabStatisticsView {
                present: true,
                visible: true,
                expanded: false,
                text: "Round 4 · live Combat Lab totals".to_owned(),
            });
            review.lattices = Some(readout_lattices());
        }
        "required-decision" => {
            review.hud = Some(required_hud());
            review.casting = Some(CastingPanelView {
                visible: true,
                content: CastingPanelContentView::Decision {
                    prompt: "Required damage · choose exactly 3 live cells on Hedge Mage."
                        .to_owned(),
                    choice: DecisionChoiceView {
                        chosen: 2,
                        owed: 3,
                        restoring: false,
                    },
                },
            });
            review.lattices = Some(decision_lattices());
            review.statistics = Some(LabStatisticsView::default());
        }
        "restore-decision" => {
            review.hud = Some(required_hud());
            review.casting = Some(CastingPanelView {
                visible: true,
                content: CastingPanelContentView::Decision {
                    prompt: "Required restoration · choose exactly 3 disabled cells on Hedge Mage."
                        .to_owned(),
                    choice: DecisionChoiceView {
                        chosen: 2,
                        owed: 3,
                        restoring: true,
                    },
                },
            });
            review.lattices = Some(decision_lattices());
            review.statistics = Some(LabStatisticsView::default());
        }
        "aiming-disabled" => {
            review.hud = Some(ordinary_hud());
            review.casting = Some(CastingPanelView {
                visible: true,
                content: CastingPanelContentView::Spells {
                    unavailable: None,
                    spells: vec![CastingSpellView {
                        name: "Lightning Bolt".to_owned(),
                        cost: "2 Air · range 4 · single target".to_owned(),
                        blocked: None,
                        color: Color::srgb(0.55, 0.78, 0.98),
                    }],
                    aiming: Some(CastingAimView {
                        label: "AIMING · Lightning Bolt · Confirm unavailable: no legal target in range. Cancel or cycle target."
                            .to_owned(),
                        controls_enabled: false,
                    }),
                },
            });
            review.statistics = Some(LabStatisticsView::default());
        }
        "live-statistics" => {
            review.hud = Some(ordinary_hud());
            review.lattices = Some(readout_lattices());
            review.statistics = Some(LabStatisticsView {
                present: true,
                visible: true,
                expanded: true,
                text: "Round 4 · turns 11 · commands 27 accepted / 3 refused\nMovement 19 / budget 23 · Channel 4\nDisables raw 8 · prevented 2 · applied 6\nHedge Mage · turns 4 · casts 3 · move 7\nRaider · turns 4 · disables 2 · no-progress max 1"
                    .to_owned(),
            });
        }
        "dense-report-compare" => {
            review.statistics = Some(LabStatisticsView::default());
            review.outcome = Some(dense_report(hex_gameplay_model::ReportMode::Compare));
        }
        "ordinary-outcome" => {
            review.statistics = Some(LabStatisticsView::default());
            review.outcome = Some(OutcomeReportView {
                visible: true,
                title: "VICTORY".to_owned(),
                detail: "The hostile roster can no longer continue.".to_owned(),
                actions: vec![
                    OutcomeActionView {
                        action: OutcomeAction::Continue,
                        label: "Continue".to_owned(),
                    },
                    OutcomeActionView {
                        action: OutcomeAction::Retry,
                        label: "Retry".to_owned(),
                    },
                ],
                ..default()
            });
        }
        "report-overview" => {
            review.outcome = Some(dense_report(hex_gameplay_model::ReportMode::Overview));
        }
        "report-units" => {
            review.outcome = Some(dense_report(hex_gameplay_model::ReportMode::Units));
        }
        "report-spells-effects" => {
            review.outcome = Some(dense_report(hex_gameplay_model::ReportMode::SpellsEffects));
        }
        "report-timeline" => {
            review.outcome = Some(dense_report(hex_gameplay_model::ReportMode::Timeline));
        }
        "report-compare" => {
            review.outcome = Some(dense_report(hex_gameplay_model::ReportMode::Compare));
        }
        other => {
            return Err(format!(
                "unknown UI review fixture {other:?}; fixture must be registered by a UiTaskCase"
            ));
        }
    }
    commands.insert_resource(review);
    Ok(())
}

#[cfg(any(feature = "visual-review", feature = "test-support"))]
fn dense_report(mode: hex_gameplay_model::ReportMode) -> OutcomeReportView {
    let body = match mode {
        hex_gameplay_model::ReportMode::Overview => {
            "OUTCOME · player victory in round 9 after 47 turns\nCommands · 118 accepted / 11 refused · longest no-progress stretch 2\nMovement · 94 of 126 budget used · 17 strikes · 21 casts · 8 Channel actions\nEffects · 38 raw disables · 9 prevented · 29 applied · 11 restorations\nDownings / revivals · player 2 / 2 · hostile 6 / 1\n\nFrozen launch\nPlayer roster · Hedge Mage, Warden, Ember Adept, Tidecaller, Scryer, Stonebinder\nHostile roster · Raider A, Raider B, Hexer, Skirmisher, Bulwark, Seer\nDeployment · 12 exact TilePos entries retained in roster order\nRules · Tactical two-step; movement 2; engage 1; disengage margin 1"
        }
        hex_gameplay_model::ReportMode::Units => {
            "PLAYER UNITS\nHedge Mage · turns 8 · commands 17/1 · move 13/16 · casts 5 · Channel 1 · idle 0\nWarden · turns 8 · commands 16/2 · move 15/16 · strikes 4 · disables 6/1/5 · downed 0\nEmber Adept · turns 8 · commands 15/1 · move 14/16 · casts 6 · disables 9/2/7 · downed 1\nTidecaller · turns 8 · commands 14/0 · move 12/16 · casts 4 · restores 5 · revived 1\nScryer · turns 7 · commands 12/2 · move 10/14 · casts 3 · reveals 3 · idle 1\nStonebinder · turns 7 · commands 13/0 · move 11/14 · strikes 3 · prevents 4 · downed 1\n\nHOSTILE UNITS\nRaider A · turns 7 · commands 11/1 · move 9/14 · strikes 3 · downed 1\nRaider B · turns 7 · commands 10/1 · move 8/14 · strikes 3 · downed 1\nHexer · turns 6 · commands 9/1 · move 7/12 · casts 3 · disables 5/1/4 · downed 1\nSkirmisher · turns 6 · commands 8/1 · move 8/12 · strikes 2 · downed 1\nBulwark · turns 5 · commands 7/0 · move 5/10 · prevents 3 · downed 1\nSeer · turns 5 · commands 6/1 · move 4/10 · reveals 2 · downed 1"
        }
        hex_gameplay_model::ReportMode::SpellsEffects => {
            "SPELLS\nLightning Bolt · 7 casts · 7 delivered · 12 raw disables · 3 prevented · 9 applied\nRenewal · 5 casts · 5 delivered · 7 cells restored · 2 revivals\nEmber · 4 casts · 4 delivered · 8 raw disables · 1 prevented · 7 applied\nScrying Eye · 3 casts · 3 delivered · 3 Reveal applications\nStone Ward · 2 casts · 2 delivered · 4 disables prevented\nUndertow · 3 casts · 2 delivered · 1 refused (no legal target)\n\nCHANNEL BY ELEMENT\nAir 6 · Fire 4 · Water 5 · Earth 3 · Light 2 · Meta 0\n\nEFFECT TOTALS\nDisable · raw 38 · prevented 9 · applied 29\nRestore · 11 cells · 2 revivals\nReveal · 5 applications · longest duration 3 turns\nMovement displacement · 3 delivered · 1 blocked by occupancy"
        }
        hex_gameplay_model::ReportMode::Timeline => {
            "ROUND 7\nT39 · Hedge Mage · Channel · Air +2, Light +1 · accepted\nT40 · Raider A · Move (-1,0,2) → (0,0,2) · refused: occupied endpoint\nT41 · Warden · Strike Raider A · 2 raw / 1 prevented / 1 applied\nT42 · Hexer · Lightning Bolt Warden · accepted · decision opened\nT42 · Warden · disable choice · Fire, Air selected · confirmed\n\nROUND 8\nT43 · Tidecaller · Renewal Warden · 2 cells restored\nT44 · Raider B · route through (0,0,2) · refused: occupied route\nT45 · Ember Adept · Ember Raider B · 3 disables applied · downed\nT46 · Seer · Scrying Eye · Reveal applied for 3 turns\nT47 · Stonebinder · Move (1,-1,2) → (2,-1,2) · accepted\n\nROUND 9\nT48 · Hedge Mage · Lightning Bolt Hexer · 2 disables applied\nT49 · Hexer · Channel · refused: downed unit cannot act\nT50 · Warden · Strike Bulwark · 1 disable applied\nT51 · Tidecaller · Renewal Stonebinder · revival applied\nT52 · Ember Adept · Ember Seer · 2 disables applied · encounter resolved"
        }
        hex_gameplay_model::ReportMode::Compare => {
            "REPORT 17 → REPORT 23\nRounds -2 · turns -5 · commands accepted +3 · refused -1 · movement +7\nChannel +2 · applied disables +1 · restorations +2 · no-progress current/max -1/+0\n\nPLAYER DELTAS\nHedge Mage · turns +0 · commands +2 · move +4 · casts +1 · Channel +1\nWarden · turns -1 · commands +1 · move +3 · strikes +1 · disables +1\nEmber Adept · turns -1 · commands +0 · move +2 · casts +2 · disables +2\nTidecaller · turns +0 · commands +1 · move -1 · restorations +2\nScryer · turns -1 · commands -1 · move +0 · reveals +1\nStonebinder · turns +0 · commands +0 · move -1 · prevents +1\n\nHOSTILE DELTAS\nRaider A · turns -1 · commands -1 · move -2 · downed +0\nRaider B · turns -1 · commands +0 · move +1 · downed +0\nHexer · turns +0 · commands +1 · move +0 · casts -1\nSkirmisher · turns -1 · commands +0 · move +1 · strikes +0\nBulwark · turns +0 · commands +0 · move +0 · prevents -1\nSeer · turns -1 · commands +0 · move +0 · reveals +1\n\nSPELLS · Lightning Bolt +2 · Renewal -1 · Ember +1 · Scrying Eye +1\nEFFECTS · Disable +1 · Restore +2 · Reveal +2 · displacement +0"
        }
    };
    OutcomeReportView {
        visible: true,
        title: format!(
            "COMBAT LAB REPORT · {}",
            match mode {
                hex_gameplay_model::ReportMode::Overview => "OVERVIEW",
                hex_gameplay_model::ReportMode::Units => "UNITS",
                hex_gameplay_model::ReportMode::SpellsEffects => "SPELLS & EFFECTS",
                hex_gameplay_model::ReportMode::Timeline => "TIMELINE",
                hex_gameplay_model::ReportMode::Compare => "COMPARE",
            }
        ),
        detail: "Frozen launch identity stays fixed while one saved comparison axis changes."
            .to_owned(),
        metadata: Some(
            "TacticalTwoStep · Flat Arena · seed 8675309 · content 8A31D119 · fingerprint 04C1B2F7"
                .to_owned(),
        ),
        mode,
        body: Some(body.to_owned()),
        comparisons: vec![
            OutcomeCompareChoiceView {
                id: hex_gameplay_model::CombatLabReportId(17),
                label: "Report 17 · shipped".to_owned(),
                selected: false,
            },
            OutcomeCompareChoiceView {
                id: hex_gameplay_model::CombatLabReportId(23),
                label: "Report 23 · tactical · SELECTED".to_owned(),
                selected: true,
            },
        ],
        actions: vec![
            OutcomeActionView {
                action: OutcomeAction::RetryExact,
                label: "Retry Exact".to_owned(),
            },
            OutcomeActionView {
                action: OutcomeAction::TuneAgain,
                label: "Tune Again".to_owned(),
            },
            OutcomeActionView {
                action: OutcomeAction::Return,
                label: "Return to Lab".to_owned(),
            },
        ],
    }
}

#[cfg(any(feature = "visual-review", feature = "test-support"))]
fn ordinary_hud() -> GameplayHudView {
    GameplayHudView {
        phase: hex_core::GameplayPhase::Active,
        actor: Some(hex_core::UnitId(0)),
        actor_label: "Hedge Mage · Player".to_owned(),
        round: "Round 4".to_owned(),
        movement_remaining: 2,
        action_remaining: true,
        required_prompt: None,
        actions: vec![
            ActionAffordance {
                action: GameplayAction::Channel,
                label: "Channel".to_owned(),
                shortcut: None,
                availability: ActionAvailability::Enabled,
                priority: ActionPriority::Primary,
            },
            ActionAffordance {
                action: GameplayAction::EndTurn,
                label: "End Turn".to_owned(),
                shortcut: Some("Space".to_owned()),
                availability: ActionAvailability::Enabled,
                priority: ActionPriority::Primary,
            },
            ActionAffordance {
                action: GameplayAction::Rest,
                label: "Rest".to_owned(),
                shortcut: Some("R".to_owned()),
                availability: ActionAvailability::Disabled {
                    reason: "Unavailable while enemies are nearby".to_owned(),
                },
                priority: ActionPriority::Primary,
            },
            ActionAffordance {
                action: GameplayAction::Pause,
                label: "Pause".to_owned(),
                shortcut: Some("Escape".to_owned()),
                availability: ActionAvailability::Enabled,
                priority: ActionPriority::Primary,
            },
        ],
    }
}

#[cfg(any(feature = "visual-review", feature = "test-support"))]
fn production_spell_catalog() -> Vec<CastingSpellView> {
    [
        ("Ember", "1 Fire · range 3", Color::srgb(0.91, 0.36, 0.25)),
        (
            "Lightning Bolt",
            "2 Air · range 4",
            Color::srgb(0.55, 0.78, 0.98),
        ),
        (
            "Renewal",
            "2 Water · restore 3",
            Color::srgb(0.35, 0.76, 0.58),
        ),
        (
            "Scrying Eye",
            "1 Air · reveal 4",
            Color::srgb(0.66, 0.62, 0.93),
        ),
    ]
    .into_iter()
    .map(|(name, cost, color)| CastingSpellView {
        name: name.to_owned(),
        cost: cost.to_owned(),
        blocked: None,
        color,
    })
    .collect()
}

#[cfg(any(feature = "visual-review", feature = "test-support"))]
fn required_hud() -> GameplayHudView {
    GameplayHudView {
        required_prompt: Some(
            "CONFIRM · choose 1 more live cell (2 of 3 selected). Other actions are blocked."
                .to_owned(),
        ),
        actions: vec![ActionAffordance {
            action: GameplayAction::ConfirmDecision,
            label: "Confirm 2 / 3".to_owned(),
            shortcut: Some("Enter".to_owned()),
            availability: ActionAvailability::Disabled {
                reason: "Choose exactly 1 more live cell".to_owned(),
            },
            priority: ActionPriority::Required,
        }],
        ..ordinary_hud()
    }
}

#[cfg(any(feature = "visual-review", feature = "test-support"))]
fn decision_lattices() -> GameplayLatticesView {
    let cell = |q, r, label: &str, selected| LatticeCellView {
        coord: hex_core::LatticeCoord::new(q, r),
        label: label.to_owned(),
        detail: if selected { "CHOSEN" } else { "LIVE" }.to_owned(),
        color: if selected {
            Color::srgb(0.95, 0.72, 0.28)
        } else {
            Color::srgb(0.28, 0.58, 0.70)
        },
        known_mana: Some(1),
        known_locked: Some(false),
        disabled: false,
        selected,
        interaction: CellInteraction::Actionable,
    };
    GameplayLatticesView {
        own: Some(OwnLatticeView {
            heading: "required damage choice".to_owned(),
            identity: "Hedge Mage · Player".to_owned(),
            cells: vec![
                cell(0, 0, "AIR", true),
                cell(1, 0, "FIRE", true),
                cell(0, 1, "SPELL", false),
                cell(-1, 1, "WATER", false),
            ],
            decision: Some(DecisionChoiceView {
                chosen: 2,
                owed: 3,
                restoring: false,
            }),
        }),
        target: None,
    }
}

#[cfg(any(feature = "visual-review", feature = "test-support"))]
fn readout_lattices() -> GameplayLatticesView {
    let cell = |q, r, label: &str, detail: &str, color| LatticeCellView {
        coord: hex_core::LatticeCoord::new(q, r),
        label: label.to_owned(),
        detail: detail.to_owned(),
        color,
        known_mana: Some(3),
        known_locked: Some(false),
        disabled: false,
        selected: false,
        interaction: CellInteraction::ReadOnly,
    };
    GameplayLatticesView {
        own: Some(OwnLatticeView {
            heading: "selected ally".to_owned(),
            identity: "Hedge Mage · Player".to_owned(),
            cells: vec![
                cell(0, 0, "FIRE", "3 / 3", Color::srgb(0.58, 0.15, 0.45)),
                cell(1, 0, "LIGHT", "3 / 3", Color::srgb(0.12, 0.44, 0.49)),
                cell(0, 1, "WATER", "2 / 2", Color::srgb(0.12, 0.48, 0.24)),
                cell(-1, 1, "EMBER", "tier 1", Color::srgb(0.28, 0.30, 0.35)),
            ],
            decision: None,
        }),
        target: None,
    }
}
