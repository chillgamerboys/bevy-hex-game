//! Initiative application adapter. Rendering belongs to `hex_ui`.

use bevy::prelude::*;
use hex_combat::{CombatSystems, TurnOrder};
use hex_core::{AppSystems, GameplaySystems, Screen};
use hex_ui::{InitiativeEntryView, InitiativeSide, InitiativeView};
use hex_units::{Faction, UnitRegistry};

use crate::readouts::GameplayUiContext;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        publish_view
            .in_set(AppSystems::Update)
            .after(CombatSystems::Advance)
            .after(GameplaySystems::UiContext)
            .run_if(in_state(Screen::Gameplay)),
    );
}

fn publish_view(
    order: Res<TurnOrder>,
    registry: Res<UnitRegistry>,
    identities: Query<(&Name, &Faction)>,
    context: Option<Res<GameplayUiContext>>,
    mut view: ResMut<InitiativeView>,
) {
    let current = order.current();
    let entries = order
        .order()
        .iter()
        .filter_map(|unit| {
            let entity = registry.entity_of(*unit)?;
            let (name, faction) = identities.get(entity).ok()?;
            Some(InitiativeEntryView {
                unit: *unit,
                name: name.as_str().to_owned(),
                side: match faction {
                    Faction::Player => InitiativeSide::Ally,
                    Faction::Hostile => InitiativeSide::Hostile,
                },
                current: current == Some(*unit),
            })
        })
        .collect();
    let heading = context
        .as_deref()
        .and_then(|context| context.acting.as_ref())
        .map_or_else(
            || "turn order".to_owned(),
            |actor| match actor.faction {
                Faction::Player => "your turn".to_owned(),
                Faction::Hostile => "enemy turn".to_owned(),
            },
        );
    let next = InitiativeView { heading, entries };
    if *view != next {
        *view = next;
    }
}

#[cfg(test)]
mod tests {
    use hex_core::UnitId;

    use super::*;

    #[test]
    fn projected_order_keeps_canonical_identity_and_side_labels() {
        let view = InitiativeView {
            heading: "your turn".to_owned(),
            entries: vec![
                InitiativeEntryView {
                    unit: UnitId(4),
                    name: "mage #4".to_owned(),
                    side: InitiativeSide::Ally,
                    current: true,
                },
                InitiativeEntryView {
                    unit: UnitId(9),
                    name: "wolf #9".to_owned(),
                    side: InitiativeSide::Hostile,
                    current: false,
                },
            ],
        };
        assert_eq!(
            view.entries.first().map(|entry| entry.unit),
            Some(UnitId(4))
        );
        assert_eq!(
            view.entries.last().map(|entry| entry.side),
            Some(InitiativeSide::Hostile)
        );
    }
}
