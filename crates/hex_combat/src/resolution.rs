//! Terminal encounter detection and the retained-world simulation gate.

use bevy::prelude::*;
use hex_core::{CommandQueue, Mode, PendingDecision, Screen};
use hex_units::{Downed, Faction};

use crate::{CombatEvent, EncounterOutcome};

/// The terminal result currently retaining and freezing the gameplay world.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EncounterResolution(pub Option<EncounterOutcome>);

impl EncounterResolution {
    /// The retained result, if the encounter has ended.
    #[must_use]
    pub fn outcome(&self) -> Option<EncounterOutcome> {
        self.0
    }

    /// Whether simulation is frozen behind an outcome modal.
    #[must_use]
    pub fn is_resolved(&self) -> bool {
        self.0.is_some()
    }
}

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<EncounterResolution>()
        .add_systems(OnEnter(Screen::Gameplay), reset_resolution)
        .add_systems(OnExit(Screen::Gameplay), reset_resolution)
        .add_systems(OnEnter(Mode::Exploring), reset_resolution);
}

/// Run condition shared by every pausable simulation system.
pub fn encounter_unresolved(resolution: Res<EncounterResolution>) -> bool {
    !resolution.is_resolved()
}

/// Opens a result once both the command and downing phases have settled.
pub(crate) fn detect_outcome(
    pending: Res<PendingDecision>,
    mut resolution: ResMut<EncounterResolution>,
    authority: Option<Res<crate::authority_host::CombatAuthority>>,
    units: Query<&Faction, Without<Downed>>,
    mut queue: ResMut<CommandQueue>,
    mut events: MessageWriter<CombatEvent>,
) {
    if let Some(authority) = authority {
        resolution.0 = authority.state.outcome;
        if resolution.is_resolved() {
            queue.clear();
        }
        return;
    }
    if resolution.is_resolved() || pending.is_open() {
        return;
    }
    let mut players = 0_u32;
    let mut hostiles = 0_u32;
    for faction in &units {
        match faction {
            Faction::Player => players += 1,
            Faction::Hostile => hostiles += 1,
        }
    }
    let outcome = if players == 0 {
        Some(EncounterOutcome::Defeat)
    } else if hostiles == 0 {
        Some(EncounterOutcome::Victory)
    } else {
        None
    };
    let Some(outcome) = outcome else {
        return;
    };

    resolution.0 = Some(outcome);
    queue.clear();
    events.write(CombatEvent::EncounterResolved { outcome });
    info!("encounter resolved: {outcome:?}");
}

fn reset_resolution(mut resolution: ResMut<EncounterResolution>) {
    resolution.0 = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::{PausableSystems, UnitId};

    fn app_with_detection() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<PendingDecision>()
            .init_resource::<CommandQueue>()
            .init_resource::<EncounterResolution>()
            .add_message::<CombatEvent>()
            .add_systems(Update, detect_outcome);
        app
    }

    #[test]
    fn terminal_outcome_emits_once() {
        let mut app = app_with_detection();
        app.world_mut().spawn((Faction::Player, UnitId(1)));
        app.world_mut().spawn((Faction::Hostile, UnitId(2), Downed));
        app.update();
        app.update();

        assert_eq!(
            app.world().resource::<EncounterResolution>().outcome(),
            Some(EncounterOutcome::Victory)
        );
        let events: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<CombatEvent>>()
            .drain()
            .collect();
        assert_eq!(
            events,
            vec![CombatEvent::EncounterResolved {
                outcome: EncounterOutcome::Victory
            }]
        );
    }

    #[test]
    fn simultaneous_elimination_is_defeat() {
        let mut app = app_with_detection();
        app.world_mut().spawn((Faction::Player, UnitId(1), Downed));
        app.world_mut().spawn((Faction::Hostile, UnitId(2), Downed));
        app.update();
        assert_eq!(
            app.world().resource::<EncounterResolution>().outcome(),
            Some(EncounterOutcome::Defeat)
        );
    }

    #[derive(Resource, Default)]
    struct Mutations(u32);

    fn mutate(mut mutations: ResMut<Mutations>) {
        mutations.0 += 1;
    }

    #[test]
    fn a_retained_outcome_blocks_pausable_simulation() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Mutations>()
            .init_resource::<EncounterResolution>()
            .configure_sets(Update, PausableSystems.run_if(encounter_unresolved))
            .add_systems(Update, mutate.in_set(PausableSystems));
        app.update();
        assert_eq!(app.world().resource::<Mutations>().0, 1);

        app.world_mut().resource_mut::<EncounterResolution>().0 = Some(EncounterOutcome::Defeat);
        app.update();
        assert_eq!(
            app.world().resource::<Mutations>().0,
            1,
            "simulation mutated behind the outcome"
        );
    }
}
