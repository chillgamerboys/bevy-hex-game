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
            review.statistics = Some(LabStatisticsView::default());
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
                expanded: true,
                text: "Round 4 · turns 11 · commands 27 accepted / 3 refused\nMovement 19 / budget 23 · Channel 4\nDisables raw 8 · prevented 2 · applied 6\nHedge Mage · turns 4 · casts 3 · move 7\nRaider · turns 4 · disables 2 · no-progress max 1"
                    .to_owned(),
            });
        }
        "dense-report-compare" => {
            review.statistics = Some(LabStatisticsView::default());
            review.outcome = Some(OutcomeReportView {
                visible: true,
                title: "COMBAT LAB REPORT · COMPARE".to_owned(),
                detail: "Frozen launch identity stays fixed while one saved comparison axis changes."
                    .to_owned(),
                metadata: Some(
                    "TacticalTwoStep · Flat Arena · seed 8675309 · content 8A31D119 · fingerprint 04C1B2F7"
                        .to_owned(),
                ),
                mode: hex_gameplay_model::ReportMode::Compare,
                body: Some(
                    "REPORT 17 → REPORT 23\nRounds -2 · turns -5 · commands +3/-1 · movement +7\nChannel +2 · applied disables +1 · no-progress current/max -1/+0\n\nPer unit\nUnit 0: turns +0, commands +2, move +4, disables +1\nUnit 1: turns -1, commands +1, move +3, disables +0\n\nSpells: Lightning Bolt +2, Renewal -1\nEffects: Disable +1, Restore -1"
                        .to_owned(),
                ),
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
            });
        }
        other => {
            return Err(format!(
                "unknown UI review fixture {other:?}; expected clear, normal-gameplay, required-decision, aiming-disabled, live-statistics, or dense-report-compare"
            ));
        }
    }
    commands.insert_resource(review);
    Ok(())
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
