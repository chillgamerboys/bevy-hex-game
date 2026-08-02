//! One explicit projection of gameplay roles for every HUD surface.
//!
//! Acting, selected, casting, deciding, and targeted units are different roles. The
//! old lattice readout collapsed them through a precedence function and then called
//! the result "your lattice", making correct state look corrupt whenever two matching
//! archetypes fought. This resource retains every role and gives the inspector one
//! explicit, testable label.

use bevy::prelude::*;
use hex_combat::TurnOrder;
use hex_core::{AppSystems, GameplaySystems, Mode, PendingDecision, Screen, UnitId};
use hex_gameplay_model::{HudState, HudTransientSurface, MainViewDestination};
use hex_units::{Faction, Party, Player, Selected, StandsOn, UnitRegistry};

use crate::casting::{Aiming, CastReadout};

use super::{lattice::RetainedTarget, HudInspection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InspectorRole {
    SelectedAlly,
    ActiveAlly,
    DamageChoice,
    RestoreTarget,
}

impl InspectorRole {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::SelectedAlly => "SELECTED ALLY",
            Self::ActiveAlly => "ACTIVE ALLY",
            Self::DamageChoice => "DAMAGE CHOICE",
            Self::RestoreTarget => "RESTORE TARGET",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TargetProvenance {
    Aim,
    Pinned,
    /// Explicit disclosure-safe HUD inspection.
    Inspected,
}

impl TargetProvenance {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Aim => "AIM TARGET",
            Self::Pinned => "PINNED TARGET",
            Self::Inspected => "INSPECTED HOSTILE",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UiUnitIdentity {
    pub(crate) unit: UnitId,
    pub(crate) name: String,
    pub(crate) faction: Faction,
    pub(crate) party_slot: Option<usize>,
}

impl UiUnitIdentity {
    pub(crate) fn label(&self) -> String {
        match (self.faction, self.party_slot) {
            (Faction::Player, Some(slot)) => {
                format!("ALLY {} · {}", slot + 1, self.name.to_uppercase())
            }
            (Faction::Player, None) => format!("ALLY · {}", self.name.to_uppercase()),
            (Faction::Hostile, _) => format!("HOSTILE · {}", self.name.to_uppercase()),
        }
    }
}

#[derive(Resource, Default, Debug, Clone, PartialEq, Eq)]
pub(crate) struct GameplayUiContext {
    pub(crate) mode: Option<Mode>,
    pub(crate) acting: Option<UiUnitIdentity>,
    pub(crate) selected_ally: Option<UiUnitIdentity>,
    pub(crate) inspected: Option<UiUnitIdentity>,
    pub(crate) caster: Option<UiUnitIdentity>,
    pub(crate) decision_owner: Option<UiUnitIdentity>,
    pub(crate) decision_target: Option<UiUnitIdentity>,
    pub(crate) inspector: Option<(InspectorRole, UiUnitIdentity)>,
    pub(crate) target: Option<(TargetProvenance, UiUnitIdentity)>,
    pub(crate) invariant_error: Option<String>,
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<GameplayUiContext>().add_systems(
        Update,
        refresh
            .in_set(AppSystems::Update)
            .in_set(GameplaySystems::UiContext)
            .run_if(in_state(Screen::Gameplay)),
    );
}

#[expect(
    clippy::too_many_arguments,
    reason = "the context intentionally records every distinct gameplay role"
)]
pub(crate) fn refresh(
    mut context: ResMut<GameplayUiContext>,
    mode: Res<State<Mode>>,
    order: Res<TurnOrder>,
    pending: Res<PendingDecision>,
    casting: Res<CastReadout>,
    aiming: Res<Aiming>,
    retained: Res<RetainedTarget>,
    hud: Res<HudState>,
    inspection: Res<HudInspection>,
    party: Res<Party>,
    registry: Res<UnitRegistry>,
    selected: Query<&UnitId, (With<Player>, With<Selected>)>,
    identities: Query<(&UnitId, &Name, &Faction)>,
    positions: Query<(&UnitId, &Faction, &StandsOn)>,
) {
    let retained_ally = context
        .inspector
        .as_ref()
        .map(|(_, unit)| unit)
        .filter(|unit| unit.faction == Faction::Player)
        .cloned();
    let identity = |unit: UnitId| {
        let entity = registry.entity_of(unit)?;
        let (unit, name, faction) = identities.get(entity).ok()?;
        Some(UiUnitIdentity {
            unit: *unit,
            name: name.as_str().to_owned(),
            faction: *faction,
            party_slot: party.members.iter().position(|member| *member == *unit),
        })
    };

    let acting = order.current().and_then(identity);
    let selected_ally = selected.iter().next().and_then(|unit| identity(*unit));
    let inspected = inspection.subject.and_then(identity);
    let requested_character = match (hud.stored_main_view(), hud.raw_transient()) {
        (MainViewDestination::Character(unit), _) => Some(unit),
        (_, Some(HudTransientSurface::Character(unit))) => Some(unit),
        _ => None,
    };
    let inspected_for_main = inspected
        .as_ref()
        .filter(|identity| requested_character == Some(identity.unit));
    let caster = casting.caster.and_then(|caster| identity(caster.unit));
    let decision_owner = pending.decider().and_then(identity);
    let decision_target = match *pending {
        PendingDecision::ChooseDisables { decider, .. } => identity(decider),
        PendingDecision::ChooseRestores { target, .. } => identity(target),
        PendingDecision::None => None,
    };

    let first_ally = party.members.first().and_then(|unit| identity(*unit));
    let inspector = resolve_inspector(
        &pending,
        acting.as_ref(),
        selected_ally.as_ref(),
        retained_ally.as_ref(),
        inspected_for_main,
        first_ally.as_ref(),
        decision_owner.as_ref(),
        decision_target.as_ref(),
    );

    let aimed_unit = aiming.0.as_ref().and_then(|aim| {
        positions.iter().find_map(|(unit, faction, standing)| {
            (*faction == Faction::Hostile && standing.0.pos == aim.anchor).then_some(*unit)
        })
    });
    let inspected_target = inspected_for_main
        .filter(|identity| identity.faction == Faction::Hostile)
        .cloned()
        .map(|unit| (TargetProvenance::Inspected, unit));
    let target = inspected_target.or_else(|| {
        retained.unit.and_then(identity).map(|unit| {
            let provenance = if aimed_unit == Some(unit.unit) {
                TargetProvenance::Aim
            } else {
                TargetProvenance::Pinned
            };
            (provenance, unit)
        })
    });

    let invariant_error = invariant_error(
        *mode.get(),
        acting.as_ref(),
        selected_ally.as_ref(),
        caster.as_ref(),
        casting.unavailable.is_none(),
        &pending,
    );
    let next = GameplayUiContext {
        mode: Some(*mode.get()),
        acting,
        selected_ally,
        inspected,
        caster,
        decision_owner,
        decision_target,
        inspector,
        target,
        invariant_error,
    };
    if *context != next {
        if let Some(error) = next.invariant_error.as_deref() {
            error!("gameplay UI state invariant: {error}");
        }
        *context = next;
    }
}

fn resolve_inspector(
    pending: &PendingDecision,
    acting: Option<&UiUnitIdentity>,
    selected: Option<&UiUnitIdentity>,
    retained: Option<&UiUnitIdentity>,
    inspected: Option<&UiUnitIdentity>,
    first_ally: Option<&UiUnitIdentity>,
    decision_owner: Option<&UiUnitIdentity>,
    decision_target: Option<&UiUnitIdentity>,
) -> Option<(InspectorRole, UiUnitIdentity)> {
    let player_decision = decision_owner.is_some_and(|owner| owner.faction == Faction::Player);
    let decision = match pending {
        PendingDecision::ChooseDisables { .. } if player_decision => decision_target
            .cloned()
            .map(|target| (InspectorRole::DamageChoice, target)),
        PendingDecision::ChooseRestores { .. } if player_decision => decision_target
            .cloned()
            .map(|target| (InspectorRole::RestoreTarget, target)),
        _ => None,
    };
    decision
        .or_else(|| {
            inspected
                .cloned()
                .map(|unit| (InspectorRole::SelectedAlly, unit))
        })
        .or_else(|| {
            acting
                .filter(|unit| unit.faction == Faction::Player)
                .cloned()
                .map(|unit| (InspectorRole::ActiveAlly, unit))
                .or_else(|| {
                    selected
                        .filter(|unit| unit.faction == Faction::Player)
                        .cloned()
                        .map(|unit| (InspectorRole::SelectedAlly, unit))
                })
                .or_else(|| {
                    retained
                        .filter(|unit| unit.faction == Faction::Player)
                        .cloned()
                        .map(|unit| (InspectorRole::SelectedAlly, unit))
                })
                .or_else(|| {
                    first_ally
                        .filter(|unit| unit.faction == Faction::Player)
                        .cloned()
                        .map(|unit| (InspectorRole::SelectedAlly, unit))
                })
        })
}

fn invariant_error(
    mode: Mode,
    acting: Option<&UiUnitIdentity>,
    selected: Option<&UiUnitIdentity>,
    caster: Option<&UiUnitIdentity>,
    casting_enabled: bool,
    pending: &PendingDecision,
) -> Option<String> {
    if mode != Mode::Combat || pending.is_open() {
        return None;
    }
    match acting {
        Some(actor) if actor.faction == Faction::Player => {
            if selected.map(|unit| unit.unit) != Some(actor.unit) {
                return Some(format!(
                    "active ally {} is not the selected ally",
                    actor.unit.0
                ));
            }
            if caster.map(|unit| unit.unit) != Some(actor.unit) {
                return Some(format!(
                    "active ally {} is not the casting unit",
                    actor.unit.0
                ));
            }
        }
        Some(actor) if actor.faction == Faction::Hostile && caster.is_some() && casting_enabled => {
            return Some(format!(
                "hostile {} is acting while player casting controls remain bound",
                actor.unit.0
            ));
        }
        _ => {}
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(id: u64, faction: Faction) -> UiUnitIdentity {
        UiUnitIdentity {
            unit: UnitId(id),
            name: format!("unit #{id}"),
            faction,
            party_slot: (faction == Faction::Player).then_some(0),
        }
    }

    #[test]
    fn ally_identity_includes_party_slot_and_faction() {
        assert_eq!(unit(3, Faction::Player).label(), "ALLY 1 · UNIT #3");
        assert_eq!(unit(9, Faction::Hostile).label(), "HOSTILE · UNIT #9");
    }

    #[test]
    fn player_turn_requires_actor_selection_and_caster_to_agree() {
        let actor = unit(3, Faction::Player);
        let wrong = unit(4, Faction::Player);
        assert!(invariant_error(
            Mode::Combat,
            Some(&actor),
            Some(&wrong),
            Some(&actor),
            true,
            &PendingDecision::None
        )
        .is_some());
        assert!(invariant_error(
            Mode::Combat,
            Some(&actor),
            Some(&actor),
            Some(&actor),
            true,
            &PendingDecision::None
        )
        .is_none());
    }

    #[test]
    fn hostile_turn_never_keeps_player_casting_controls_bound() {
        let hostile = unit(9, Faction::Hostile);
        let ally = unit(3, Faction::Player);
        assert!(invariant_error(
            Mode::Combat,
            Some(&hostile),
            Some(&ally),
            Some(&ally),
            true,
            &PendingDecision::None
        )
        .is_some());
    }

    #[test]
    fn exploration_and_combat_choose_only_explicit_ally_inspector_roles() {
        let ally = unit(3, Faction::Player);
        let hostile = unit(9, Faction::Hostile);

        assert_eq!(
            resolve_inspector(
                &PendingDecision::None,
                None,
                Some(&ally),
                None,
                None,
                None,
                None,
            ),
            Some((InspectorRole::SelectedAlly, ally.clone()))
        );
        assert_eq!(
            resolve_inspector(
                &PendingDecision::None,
                Some(&ally),
                Some(&unit(4, Faction::Player)),
                None,
                None,
                None,
                None,
            ),
            Some((InspectorRole::ActiveAlly, ally.clone()))
        );
        assert_eq!(
            resolve_inspector(
                &PendingDecision::None,
                Some(&hostile),
                Some(&ally),
                None,
                None,
                None,
                None,
            ),
            Some((InspectorRole::SelectedAlly, ally))
        );
    }

    #[test]
    fn only_player_owned_decisions_replace_the_ally_inspector() {
        let ally = unit(3, Faction::Player);
        let restore_target = unit(4, Faction::Player);
        let hostile = unit(9, Faction::Hostile);

        assert_eq!(
            resolve_inspector(
                &PendingDecision::ChooseDisables {
                    decider: ally.unit,
                    count: 1,
                    source: hostile.unit,
                },
                Some(&hostile),
                Some(&restore_target),
                None,
                None,
                Some(&ally),
                Some(&ally),
            ),
            Some((InspectorRole::DamageChoice, ally.clone()))
        );
        assert_eq!(
            resolve_inspector(
                &PendingDecision::ChooseRestores {
                    decider: ally.unit,
                    target: restore_target.unit,
                    count: 1,
                },
                Some(&ally),
                Some(&ally),
                None,
                None,
                Some(&ally),
                Some(&restore_target),
            ),
            Some((InspectorRole::RestoreTarget, restore_target.clone()))
        );

        assert_eq!(
            resolve_inspector(
                &PendingDecision::ChooseDisables {
                    decider: hostile.unit,
                    count: 1,
                    source: ally.unit,
                },
                Some(&ally),
                Some(&restore_target),
                None,
                None,
                Some(&hostile),
                Some(&hostile),
            ),
            Some((InspectorRole::ActiveAlly, ally)),
            "an AI decision must never expose a hostile lattice as ally truth"
        );
    }
}
