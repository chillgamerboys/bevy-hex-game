//! Casting-panel projection and canonical command adapters.

use bevy::prelude::*;
use hex_combat::TurnOrder;
use hex_core::{
    CommandQueue, ControlOwner, GameCommand, InputAction, InputBindings, IssuedCommand, Mode,
    PendingDecision,
};
use hex_ui::{CastingAimView, CastingPanelContentView, CastingPanelView, CastingSpellView};
use hex_units::{Faction, UnitRegistry};

use crate::readouts::{DisableSelection, GameplayUiContext, TargetProvenance};

use super::preview::AimVolume;
use super::{Aiming, CastReadout};

pub(super) fn publish_view(
    readout: Res<CastReadout>,
    aiming: Res<Aiming>,
    volume: Res<AimVolume>,
    mode: Res<State<Mode>>,
    context: Res<GameplayUiContext>,
    decision: Res<DisableSelection>,
    pending: Res<PendingDecision>,
    bindings: Res<InputBindings>,
    mut view: ResMut<CastingPanelView>,
) {
    let visible = *mode.get() == Mode::Combat;
    let confirm_shortcut = bindings.chord(InputAction::Confirm).label();
    let content = if let Some(choice) = decision.summary(confirm_shortcut.clone()) {
        let owner = context
            .decision_owner
            .as_ref()
            .map_or_else(|| "UNKNOWN ALLY".to_owned(), |unit| unit.label());
        let target = context
            .decision_target
            .as_ref()
            .map_or_else(|| "UNKNOWN TARGET".to_owned(), |unit| unit.label());
        CastingPanelContentView::Decision {
            prompt: if choice.restoring {
                format!("RESTORE TARGET · {owner} → {target}")
            } else {
                format!("DAMAGE CHOICE · {owner}")
            },
            choice,
        }
    } else if pending.is_open() {
        let owner = context
            .decision_owner
            .as_ref()
            .map_or_else(|| "UNKNOWN UNIT".to_owned(), |unit| unit.label());
        let task = match *pending {
            PendingDecision::ChooseDisables { .. } => "DAMAGE CHOICE",
            PendingDecision::ChooseRestores { .. } => "RESTORATION CHOICE",
            PendingDecision::None => unreachable!("the decision was checked as open"),
        };
        CastingPanelContentView::Message {
            text: format!("RESOLVING {task} · {owner} · PLAYER COMMANDS LOCKED"),
            turn_controls: false,
        }
    } else if context
        .acting
        .as_ref()
        .is_some_and(|unit| unit.faction == Faction::Hostile)
    {
        CastingPanelContentView::Message {
            text: "ENEMY TURN · PLAYER COMMANDS LOCKED".to_owned(),
            turn_controls: false,
        }
    } else if readout.caster.is_none() {
        CastingPanelContentView::Message {
            text: "no unit to cast from".to_owned(),
            turn_controls: true,
        }
    } else if readout.spells.is_empty() {
        CastingPanelContentView::Message {
            text: "this unit inscribes no spells".to_owned(),
            turn_controls: true,
        }
    } else {
        CastingPanelContentView::Spells {
            unavailable: readout.unavailable.map(str::to_owned),
            spells: readout
                .spells
                .iter()
                .map(|spell| CastingSpellView {
                    name: spell.name.clone(),
                    cost: spell.cost.clone(),
                    blocked: spell.blocked.map(str::to_owned),
                    color: spell.color,
                })
                .collect(),
            aiming: aiming.0.as_ref().map(|aim| CastingAimView {
                label: aim_label(&aim.spell, &volume, &context),
                controls_enabled: readout.unavailable.is_none(),
                confirm_shortcut: confirm_shortcut.clone(),
                next_target_shortcut: bindings.chord(InputAction::NextTarget).label(),
                cancel_shortcut: bindings.chord(InputAction::CancelCast).label(),
            }),
        }
    };
    let next = CastingPanelView { visible, content };
    if *view != next {
        *view = next;
    }
}

fn aim_label(spell: &str, volume: &AimVolume, context: &GameplayUiContext) -> String {
    let target = context
        .target
        .as_ref()
        .filter(|(provenance, _)| *provenance == TargetProvenance::Aim)
        .map(|(_, target)| {
            let identity = target.label();
            if context
                .caster
                .as_ref()
                .is_some_and(|caster| caster.unit == target.unit)
            {
                format!("SELF · {identity}")
            } else {
                identity
            }
        });
    target.map_or_else(
        || {
            format!(
                "AIMING {} · {} VOXELS / {} SURFACES",
                spell.to_uppercase(),
                volume.voxels,
                volume.painted
            )
        },
        |target| {
            format!(
                "AIMING {} · TARGET {target} · {} VOXELS / {} SURFACES",
                spell.to_uppercase(),
                volume.voxels,
                volume.painted
            )
        },
    )
}

pub(crate) fn queue_current_player_command(
    requested: bool,
    order: &TurnOrder,
    pending: &PendingDecision,
    registry: &UnitRegistry,
    owners: &Query<(Option<&ControlOwner>, &Faction)>,
    queue: &mut CommandQueue,
    command: impl FnOnce(hex_core::UnitId) -> GameCommand,
) {
    if pending.is_open() || !requested {
        return;
    }
    let Some(unit) = order.current() else {
        return;
    };
    let Some(entity) = registry.entity_of(unit) else {
        return;
    };
    let Ok((owner, faction)) = owners.get(entity) else {
        return;
    };
    if *faction != Faction::Player {
        return;
    }
    queue.push(IssuedCommand {
        seat: owner.copied().unwrap_or_default().0,
        command: command(unit),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::UnitId;

    fn identity(unit: u64, slot: usize, name: &str) -> crate::readouts::UiUnitIdentity {
        crate::readouts::UiUnitIdentity {
            unit: UnitId(unit),
            name: name.to_owned(),
            faction: Faction::Player,
            party_slot: Some(slot),
            disclosed: true,
        }
    }

    #[test]
    fn aim_summary_names_self_and_allied_targets_explicitly() {
        let caster = identity(1, 0, "healer");
        let ally = identity(2, 1, "scout");
        let volume = AimVolume {
            voxels: 1,
            painted: 1,
        };
        let mut context = GameplayUiContext {
            caster: Some(caster.clone()),
            target: Some((TargetProvenance::Aim, caster)),
            ..default()
        };

        assert_eq!(
            aim_label("Heal", &volume, &context),
            "AIMING HEAL · TARGET SELF · ALLY 1 · HEALER · 1 VOXELS / 1 SURFACES"
        );

        context.target = Some((TargetProvenance::Aim, ally));
        assert_eq!(
            aim_label("Heal", &volume, &context),
            "AIMING HEAL · TARGET ALLY 2 · SCOUT · 1 VOXELS / 1 SURFACES"
        );
    }
}
