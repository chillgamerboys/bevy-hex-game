//! Gameplay readouts: lattices, initiative, and the disclosed combat history.

use bevy::prelude::*;
use hex_core::{AppSystems, Screen};

mod initiative;
mod lattice;
mod log;

/// An ordinary gameplay UI root controlled by the `H` toggle.
#[derive(Component)]
pub(crate) struct HudElement;

/// Whether ordinary gameplay chrome is currently shown.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HudVisibility {
    pub(crate) shown: bool,
}

impl Default for HudVisibility {
    fn default() -> Self {
        Self { shown: true }
    }
}

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<HudVisibility>()
        .add_plugins((lattice::plugin, initiative::plugin, log::plugin))
        .add_systems(OnEnter(Screen::Gameplay), reset_hud)
        // Deliberately outside `PausableSystems`: hiding chrome does not advance
        // the simulation and remains available while a decision is open.
        .add_systems(
            Update,
            (toggle_hud, apply_hud_visibility)
                .chain()
                .in_set(AppSystems::RecordInput)
                .run_if(in_state(Screen::Gameplay)),
        );
}

fn reset_hud(mut hud: ResMut<HudVisibility>) {
    hud.shown = true;
}

fn toggle_hud(keys: Res<ButtonInput<KeyCode>>, mut hud: ResMut<HudVisibility>) {
    if keys.just_pressed(KeyCode::KeyH) {
        hud.shown = !hud.shown;
    }
}

fn apply_hud_visibility(
    hud: Res<HudVisibility>,
    selection: Res<lattice::DisableSelection>,
    mut roots: Query<(&mut Visibility, Has<lattice::DecisionLattice>), With<HudElement>>,
) {
    for (mut visibility, decision_lattice) in &mut roots {
        let wanted = if hud.shown || (decision_lattice && selection.is_active()) {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::MinimalPlugins;

    use super::*;

    #[test]
    fn hiding_the_hud_changes_roots_without_despawning_them() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<HudVisibility>()
            .init_resource::<lattice::DisableSelection>()
            .add_systems(Update, apply_hud_visibility);
        let root = app
            .world_mut()
            .spawn((HudElement, Visibility::Inherited))
            .id();

        app.world_mut().resource_mut::<HudVisibility>().shown = false;
        app.update();

        assert_eq!(
            app.world().get::<Visibility>(root),
            Some(&Visibility::Hidden)
        );
        assert!(app.world().get_entity(root).is_ok(), "the root stays alive");
    }

    #[test]
    fn an_active_decision_lattice_stays_visible_when_the_hud_is_hidden() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<HudVisibility>()
            .init_resource::<lattice::DisableSelection>()
            .add_systems(Update, apply_hud_visibility);
        let ordinary = app
            .world_mut()
            .spawn((HudElement, Visibility::Inherited))
            .id();
        let decision = app
            .world_mut()
            .spawn((HudElement, lattice::DecisionLattice, Visibility::Inherited))
            .id();
        app.world_mut().resource_mut::<HudVisibility>().shown = false;
        app.world_mut()
            .resource_mut::<lattice::DisableSelection>()
            .decision = Some(lattice::DisableDecision {
            decider: hex_core::UnitId(3),
            target: hex_core::UnitId(3),
            owed: 1,
            restoring: false,
            live: vec![hex_core::LatticeCoord::ORIGIN],
        });
        app.update();

        assert_eq!(
            app.world().get::<Visibility>(ordinary),
            Some(&Visibility::Hidden)
        );
        assert_eq!(
            app.world().get::<Visibility>(decision),
            Some(&Visibility::Inherited)
        );
    }
}
