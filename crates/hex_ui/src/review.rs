//! Default-off authored presentation states for bounded visual review.

use bevy::prelude::*;

use crate::{
    ActivityLogView, CastingPanelView, GameplayChromeView, GameplayHudView, GameplayLatticesView,
    InitiativeView, MultiplayerView, OutcomeView, PartyView,
};

#[cfg(any(feature = "visual-review", feature = "test-support"))]
use crate::{
    ActionAffordance, ActionAvailability, ActionPriority, CastingAimView, CastingPanelContentView,
    CastingSpellView, CellInteraction, DecisionChoiceView, GameplayAction, InitiativeEntryView,
    InitiativeSide, LatticeCellView, MultiplayerAssignmentView, MultiplayerSeatConnectionView,
    MultiplayerSeatView, OutcomeAction, OutcomeActionView, OwnLatticeView, PartyMemberView,
    SandboxLatticeCellKind, SandboxLatticeCellView, SensitiveText, TargetLatticeStateView,
    TargetLatticeView,
};

#[derive(Resource, Default)]
pub(crate) struct UiReviewPresentation {
    pub(crate) chrome: Option<GameplayChromeOverride>,
    pub(crate) hud: Option<GameplayHudView>,
    pub(crate) party: Option<PartyView>,
    pub(crate) initiative: Option<InitiativeView>,
    pub(crate) activity: Option<ActivityLogView>,
    pub(crate) casting: Option<CastingPanelView>,
    pub(crate) lattices: Option<GameplayLatticesView>,
    pub(crate) outcome: Option<OutcomeView>,
    pub(crate) multiplayer: Option<MultiplayerView>,
}

/// Narrow presentation overrides used only by authored review fixtures.
///
/// Keeping each field optional prevents a fixture that needs one presentation
/// fact (for example, a required decision) from shadowing unrelated canonical
/// chrome such as the player's HUD visibility preference.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GameplayChromeOverride {
    party_shown: Option<bool>,
    initiative_shown: Option<bool>,
    activity_shown: Option<bool>,
    action_bar_shown: Option<bool>,
    main_view: Option<hex_gameplay_model::MainViewDestination>,
    encounter_complete: Option<bool>,
}

impl GameplayChromeOverride {
    #[cfg(any(feature = "visual-review", feature = "test-support"))]
    const fn required_decision() -> Self {
        Self {
            main_view: Some(hex_gameplay_model::MainViewDestination::RequiredDecision),
            ..Self::none()
        }
    }

    #[cfg(any(feature = "visual-review", feature = "test-support"))]
    const fn player_turn() -> Self {
        Self {
            // Preserve the viewport-aware Party projection: visible by default on
            // Standard/Wide, suppressed on Compact when another task surface owns it.
            party_shown: None,
            initiative_shown: Some(true),
            activity_shown: Some(false),
            action_bar_shown: Some(true),
            main_view: Some(hex_gameplay_model::MainViewDestination::Closed),
            encounter_complete: Some(false),
        }
    }

    #[cfg(any(feature = "visual-review", feature = "test-support"))]
    const fn hostile_turn() -> Self {
        Self {
            action_bar_shown: Some(false),
            ..Self::player_turn()
        }
    }

    #[cfg(any(feature = "visual-review", feature = "test-support"))]
    const fn encounter_complete() -> Self {
        Self {
            encounter_complete: Some(true),
            ..Self::none()
        }
    }

    #[cfg(any(feature = "visual-review", feature = "test-support"))]
    const fn none() -> Self {
        Self {
            party_shown: None,
            initiative_shown: None,
            activity_shown: None,
            action_bar_shown: None,
            main_view: None,
            encounter_complete: None,
        }
    }

    pub(crate) fn apply(
        self,
        base: GameplayChromeView,
        viewport: crate::UiViewportClass,
    ) -> GameplayChromeView {
        let mut next = base;
        // Review fixtures may populate canonical combat content while a live walk
        // remains in Exploration. They may not bypass Compact's one-surface policy:
        // the renderer-free model (or the headless case) still owns that choice.
        if viewport != crate::UiViewportClass::Compact {
            next.party_shown = self.party_shown.unwrap_or(base.party_shown);
            next.initiative_shown = self.initiative_shown.unwrap_or(base.initiative_shown);
            next.activity_shown = self.activity_shown.unwrap_or(base.activity_shown);
            next.action_bar_shown = self.action_bar_shown.unwrap_or(base.action_bar_shown);
        }
        if let Some(main_view) = self.main_view {
            next.main_view = main_view;
        }
        next.encounter_complete = self.encounter_complete.unwrap_or(base.encounter_complete);
        next
    }
}

impl UiReviewPresentation {
    pub(crate) fn effective_chrome(
        &self,
        base: GameplayChromeView,
        viewport: crate::UiViewportClass,
    ) -> GameplayChromeView {
        self.chrome
            .map_or(base, |override_| override_.apply(base, viewport))
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
            review.party = Some(review_party());
            review.activity = Some(review_activity());
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
            review.chrome = Some(GameplayChromeOverride::player_turn());
            review.party = Some(review_party());
            review.initiative = Some(review_initiative(false));
            review.hud = Some(ordinary_hud());
            review.lattices = Some(populated_own_lattice());
        }
        "hostile-turn" => {
            review.chrome = Some(GameplayChromeOverride::hostile_turn());
            review.party = Some(review_party());
            review.initiative = Some(review_initiative(true));
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
                        confirm_shortcut: "Enter".to_owned(),
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
                        confirm_shortcut: "Enter".to_owned(),
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
                        confirm_shortcut: "Enter".to_owned(),
                        next_target_shortcut: "Tab".to_owned(),
                        cancel_shortcut: "Q".to_owned(),
                    }),
                },
            });
            review.lattices = Some(populated_own_lattice());
        }
        "multiplayer-lobby" => {
            review.multiplayer = Some(multiplayer_lobby_fixture(false));
        }
        "multiplayer-mismatch" => {
            let mut view = MultiplayerView::default();
            view.route = hex_gameplay_model::MultiplayerRoute::Ended;
            view.notice = Some(
                "Build mismatch: host and client must use the exact same shipped build.".to_owned(),
            );
            review.multiplayer = Some(view);
        }
        "multiplayer-reconnect" => {
            review.multiplayer = Some(multiplayer_lobby_fixture(true));
        }
        "multiplayer-host" => {
            let mut view = multiplayer_lobby_fixture(false);
            view.role = Some(hex_gameplay_model::MultiplayerRole::Host);
            view.local_seat = Some(hex_core::PlayerSeat::HOST);
            view.local_menu_open = false;
            for seat in &mut view.seats {
                seat.local = seat.seat == hex_core::PlayerSeat::HOST;
            }
            view.share_code = Some(SensitiveText::new("HEX1.REDACTED_REVIEW_CODE"));
            review.multiplayer = Some(view);
        }
        "multiplayer-client-menu" => {
            let mut view = multiplayer_lobby_fixture(false);
            view.role = Some(hex_gameplay_model::MultiplayerRole::Client);
            view.local_seat = Some(hex_core::PlayerSeat(1));
            view.local_menu_open = true;
            review.multiplayer = Some(view);
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
fn multiplayer_lobby_fixture(reconnecting: bool) -> MultiplayerView {
    let assignment = |unit, label: &str| MultiplayerAssignmentView {
        unit: hex_core::UnitId(unit),
        label: label.to_owned(),
    };
    let local_seat = if reconnecting {
        hex_core::PlayerSeat(4)
    } else {
        hex_core::PlayerSeat(1)
    };
    let mut view = MultiplayerView::default();
    view.route = hex_gameplay_model::MultiplayerRoute::Lobby;
    view.role = Some(hex_gameplay_model::MultiplayerRole::Client);
    view.local_seat = Some(local_seat);
    view.seats = vec![
        MultiplayerSeatView {
            seat: hex_core::PlayerSeat::HOST,
            connection: MultiplayerSeatConnectionView::Connected,
            player_label: Some("Host · Aria".to_owned()),
            assignments: vec![assignment(0, "Hedge Mage"), assignment(5, "Stone Warden")],
            ready: true,
            local: local_seat == hex_core::PlayerSeat::HOST,
        },
        MultiplayerSeatView {
            seat: hex_core::PlayerSeat(1),
            connection: MultiplayerSeatConnectionView::Connected,
            player_label: Some("Milo".to_owned()),
            assignments: vec![assignment(1, "Ember Knight")],
            ready: true,
            local: local_seat == hex_core::PlayerSeat(1),
        },
        MultiplayerSeatView {
            seat: hex_core::PlayerSeat(2),
            connection: MultiplayerSeatConnectionView::Connected,
            player_label: Some("Nia".to_owned()),
            assignments: vec![assignment(2, "Tidecaller")],
            ready: true,
            local: local_seat == hex_core::PlayerSeat(2),
        },
        MultiplayerSeatView {
            seat: hex_core::PlayerSeat(3),
            connection: if reconnecting {
                MultiplayerSeatConnectionView::Delegated
            } else {
                MultiplayerSeatConnectionView::Reserved { seconds: 17 }
            },
            player_label: Some("Oren".to_owned()),
            assignments: vec![assignment(3, "Gale Adept")],
            ready: false,
            local: local_seat == hex_core::PlayerSeat(3),
        },
        MultiplayerSeatView {
            seat: hex_core::PlayerSeat(4),
            connection: if reconnecting {
                MultiplayerSeatConnectionView::ReclaimPending
            } else {
                MultiplayerSeatConnectionView::Delegated
            },
            player_label: Some("Pia".to_owned()),
            assignments: vec![assignment(4, "Bloom Sage")],
            ready: false,
            local: local_seat == hex_core::PlayerSeat(4),
        },
        MultiplayerSeatView::vacant(hex_core::PlayerSeat(5)),
    ];
    view.launch_summary = Some("Shipped Sandbox · Party Trial · seed 77".to_owned());
    view.notice = reconnecting.then(|| {
        "Seat 5 reconnected. Control returns after the current command, decision, and movement finish."
            .to_owned()
    });
    view.can_launch = false;
    view.launch_blocker = Some(if reconnecting {
        "Seat 5 is waiting for a safe authority boundary.".to_owned()
    } else {
        "Seat 4 is disconnected; wait for reconnection or delegation.".to_owned()
    });
    view
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
                confirm_shortcut: "Enter".to_owned(),
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

#[cfg(any(feature = "visual-review", feature = "test-support"))]
fn review_party() -> PartyView {
    let cells = vec![
        SandboxLatticeCellView {
            q: 0,
            r: 0,
            label: "G".to_owned(),
            kind: SandboxLatticeCellKind::Gem,
        },
        SandboxLatticeCellView {
            q: 1,
            r: 0,
            label: "F".to_owned(),
            kind: SandboxLatticeCellKind::Fusion,
        },
        SandboxLatticeCellView {
            q: 0,
            r: 1,
            label: "S".to_owned(),
            kind: SandboxLatticeCellKind::Spell,
        },
    ];
    PartyView {
        members: ["Hedge Mage", "Raider", "Wolf"]
            .into_iter()
            .enumerate()
            .map(|(slot, name)| PartyMemberView {
                slot,
                label: format!("{name} · {}", if slot == 0 { "active" } else { "ready" }),
                cells: cells.clone(),
                active: slot == 0,
                selected: slot == 0,
            })
            .collect(),
        formation_visible: true,
        movement_mode: "GROUP · formation follows the selected anchor".to_owned(),
        presets: vec!["Column".to_owned(), "Wedge".to_owned()],
        slots: Vec::new(),
    }
}

#[cfg(any(feature = "visual-review", feature = "test-support"))]
fn review_initiative(hostile_turn: bool) -> InitiativeView {
    InitiativeView {
        heading: if hostile_turn {
            "enemy turn"
        } else {
            "your turn"
        }
        .to_owned(),
        entries: vec![
            InitiativeEntryView {
                unit: hex_core::UnitId(0),
                name: "Hedge Mage".to_owned(),
                side: InitiativeSide::Ally,
                current: !hostile_turn,
                inspectable: true,
            },
            InitiativeEntryView {
                unit: hex_core::UnitId(1),
                name: "Observed Raider".to_owned(),
                side: InitiativeSide::Hostile,
                current: hostile_turn,
                inspectable: true,
            },
            InitiativeEntryView {
                unit: hex_core::UnitId(2),
                name: "Unobserved hostile".to_owned(),
                side: InitiativeSide::Hostile,
                current: false,
                inspectable: false,
            },
        ],
    }
}

#[cfg(any(feature = "visual-review", feature = "test-support"))]
fn review_activity() -> ActivityLogView {
    ActivityLogView {
        heading: "ACTIVITY · L".to_owned(),
        tab: crate::ActivityTab::All,
        lines: vec![
            crate::ActivityLogLineView {
                kind: crate::ActivityKind::Combat,
                text: "Hedge Mage cast Lightning Bolt".to_owned(),
                danger: false,
            },
            crate::ActivityLogLineView {
                kind: crate::ActivityKind::Activity,
                text: "Party formation changed to Wedge".to_owned(),
                danger: false,
            },
        ],
    }
}

#[cfg(all(test, any(feature = "visual-review", feature = "test-support")))]
mod tests {
    use super::*;

    #[test]
    fn combat_turn_fixtures_override_only_the_authored_component_combination() {
        let base = GameplayChromeView {
            party_shown: true,
            initiative_shown: false,
            activity_shown: true,
            action_bar_shown: false,
            main_view: hex_gameplay_model::MainViewDestination::RequiredDecision,
            terrain_health_shown: false,
            encounter_complete: true,
        };

        assert_eq!(
            GameplayChromeOverride::player_turn().apply(base, crate::UiViewportClass::Standard),
            GameplayChromeView {
                party_shown: true,
                initiative_shown: true,
                activity_shown: false,
                action_bar_shown: true,
                main_view: hex_gameplay_model::MainViewDestination::Closed,
                terrain_health_shown: false,
                encounter_complete: false,
            }
        );
        assert_eq!(
            GameplayChromeOverride::hostile_turn().apply(base, crate::UiViewportClass::Standard),
            GameplayChromeView {
                party_shown: true,
                initiative_shown: true,
                activity_shown: false,
                action_bar_shown: false,
                main_view: hex_gameplay_model::MainViewDestination::Closed,
                terrain_health_shown: false,
                encounter_complete: false,
            }
        );
    }

    #[test]
    fn required_fixture_preserves_ordinary_and_phase_chrome() {
        let base = GameplayChromeView {
            party_shown: false,
            initiative_shown: true,
            activity_shown: true,
            action_bar_shown: false,
            main_view: hex_gameplay_model::MainViewDestination::Closed,
            terrain_health_shown: false,
            encounter_complete: true,
        };
        let expected = GameplayChromeView {
            main_view: hex_gameplay_model::MainViewDestination::RequiredDecision,
            ..base
        };

        assert_eq!(
            GameplayChromeOverride::required_decision()
                .apply(base, crate::UiViewportClass::Standard),
            expected
        );
    }

    #[test]
    fn combat_fixture_cannot_bypass_compact_single_surface_policy() {
        let base = GameplayChromeView {
            action_bar_shown: true,
            ..GameplayChromeView::default()
        };
        assert_eq!(
            GameplayChromeOverride::player_turn().apply(base, crate::UiViewportClass::Compact),
            base
        );
    }

    #[test]
    fn turn_fixture_content_is_populated_and_disclosure_safe() {
        let party = review_party();
        let initiative = review_initiative(true);
        let activity = review_activity();

        assert_eq!(party.members.len(), 3);
        assert!(party.members.iter().all(|member| !member.cells.is_empty()));
        assert_eq!(initiative.entries.len(), 3);
        assert!(initiative.entries.iter().any(|entry| entry.current));
        assert!(initiative.entries.iter().any(|entry| !entry.inspectable));
        assert!(activity.lines.len() >= 2);
    }
}
