//! Default-off authored presentation states for bounded visual review.

use bevy::prelude::*;

use crate::{
    CastingPanelView, GameplayChromeView, GameplayHudView, GameplayLatticesView, OutcomeView,
};

#[cfg(any(feature = "visual-review", feature = "test-support"))]
use crate::{
    ActionAffordance, ActionAvailability, ActionPriority, CastingAimView, CastingPanelContentView,
    CastingSpellView, CellInteraction, DecisionChoiceView, GameplayAction, LatticeCellView,
    OutcomeAction, OutcomeActionView, OwnLatticeView, TargetLatticeStateView, TargetLatticeView,
};

#[derive(Resource, Default)]
pub(crate) struct UiReviewPresentation {
    pub(crate) chrome: Option<GameplayChromeOverride>,
    pub(crate) hud: Option<GameplayHudView>,
    pub(crate) casting: Option<CastingPanelView>,
    pub(crate) lattices: Option<GameplayLatticesView>,
    pub(crate) outcome: Option<OutcomeView>,
}

/// Narrow presentation overrides used only by authored review fixtures.
///
/// Keeping each field optional prevents a fixture that needs one presentation
/// fact (for example, a required decision) from shadowing unrelated canonical
/// chrome such as the player's HUD visibility preference.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GameplayChromeOverride {
    shown: Option<bool>,
    decision_required: Option<bool>,
    encounter_complete: Option<bool>,
}

impl GameplayChromeOverride {
    #[cfg(any(feature = "visual-review", feature = "test-support"))]
    const fn required_decision() -> Self {
        Self {
            decision_required: Some(true),
            shown: None,
            encounter_complete: None,
        }
    }

    #[cfg(any(feature = "visual-review", feature = "test-support"))]
    const fn encounter_complete() -> Self {
        Self {
            encounter_complete: Some(true),
            shown: None,
            decision_required: None,
        }
    }

    pub(crate) fn apply(self, base: GameplayChromeView) -> GameplayChromeView {
        GameplayChromeView {
            shown: self.shown.unwrap_or(base.shown),
            decision_required: self.decision_required.unwrap_or(base.decision_required),
            encounter_complete: self.encounter_complete.unwrap_or(base.encounter_complete),
        }
    }
}

impl UiReviewPresentation {
    pub(crate) fn effective_chrome(&self, base: GameplayChromeView) -> GameplayChromeView {
        self.chrome.map_or(base, |override_| override_.apply(base))
    }
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
            review.lattices = Some(populated_own_lattice());
        }
        "player-turn-max" => {
            review.hud = Some(ordinary_hud());
            review.lattices = Some(populated_own_lattice());
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
            review.lattices = Some(populated_own_lattice());
        }
        "casting-list" => {
            review.hud = Some(ordinary_hud());
            review.casting = Some(populated_casting());
            review.lattices = Some(populated_lattices());
        }
        "required-decision" => {
            review.chrome = Some(GameplayChromeOverride::required_decision());
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
        }
        "restore-decision" => {
            review.chrome = Some(GameplayChromeOverride::required_decision());
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
            review.lattices = Some(populated_own_lattice());
        }
        "sandbox-outcome" => {
            review.chrome = Some(GameplayChromeOverride::encounter_complete());
            review.outcome = Some(OutcomeView {
                visible: true,
                title: "VICTORY".to_owned(),
                detail: "The hostile roster can no longer continue.".to_owned(),
                actions: vec![
                    OutcomeActionView {
                        action: OutcomeAction::RetryExact,
                        label: "Retry Exact".to_owned(),
                    },
                    OutcomeActionView {
                        action: OutcomeAction::Return,
                        label: "Return to Sandbox".to_owned(),
                    },
                ],
            });
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
pub(crate) fn populated_casting() -> CastingPanelView {
    CastingPanelView {
        visible: true,
        content: CastingPanelContentView::Spells {
            unavailable: None,
            spells: production_spell_catalog(),
            aiming: None,
        },
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
pub(crate) fn populated_lattices() -> GameplayLatticesView {
    let mut lattices = decision_lattices();
    if let Some(own) = &mut lattices.own {
        own.heading = "selected ally".to_owned();
        own.decision = None;
        let templates = own.cells.clone();
        for (index, (template, (q, r))) in templates
            .iter()
            .cycle()
            .zip([
                (1, -1),
                (0, -1),
                (-1, 0),
                (2, 0),
                (2, -1),
                (2, -2),
                (1, -2),
                (0, -2),
                (-1, -1),
            ])
            .enumerate()
        {
            let mut cell = template.clone();
            cell.coord = hex_core::LatticeCoord::new(q, r);
            cell.label = format!("CELL {}", index + templates.len() + 1);
            cell.selected = false;
            cell.interaction = CellInteraction::ReadOnly;
            own.cells.push(cell);
        }
        for cell in &mut own.cells {
            cell.detail = "LIVE".to_owned();
            cell.selected = false;
            cell.interaction = CellInteraction::ReadOnly;
        }
    }
    let target_cells = lattices
        .own
        .as_ref()
        .map_or_else(Vec::new, |own| own.cells.iter().take(8).cloned().collect());
    lattices.target = Some(TargetLatticeView {
        heading: "aim target".to_owned(),
        identity: "Raider · Hostile".to_owned(),
        state: TargetLatticeStateView::Known {
            cells: target_cells,
            unknown: Some(4),
        },
    });
    lattices
}

#[cfg(any(feature = "visual-review", feature = "test-support"))]
fn populated_own_lattice() -> GameplayLatticesView {
    let mut lattices = populated_lattices();
    lattices.target = None;
    lattices
}
