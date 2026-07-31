//! Gameplay readouts: lattices, initiative, and the disclosed combat history.

use bevy::prelude::*;
use hex_combat::{CombatSystems, EncounterResolution};
use hex_core::{AppSystems, GameplayPhase, GameplaySetup, GameplaySystems, Screen};
pub(crate) use hex_ui::{
    HudElement, UiHudSetup as HudSetup, UiRegionRole as HudRegion, READ_ONLY_HUD,
};

mod badges;
mod context;
mod initiative;
mod lattice;
mod log;

pub(crate) use context::{GameplayUiContext, InspectorRole, TargetProvenance, UiUnitIdentity};
pub(crate) use lattice::{spawn_decision_controls, DecisionHud, DisableSelection};

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
    app.init_resource::<hex_core::InputBindings>();
    app.init_resource::<HudVisibility>()
        .configure_sets(
            Update,
            (
                GameplaySystems::Selection,
                GameplaySystems::Casting,
                GameplaySystems::UiContext,
            )
                .chain()
                .after(CombatSystems::Advance),
        )
        .configure_sets(
            OnEnter(Screen::Gameplay),
            (HudSetup::Frame, HudSetup::Panels)
                .chain()
                .in_set(GameplaySetup::View),
        )
        .add_systems(
            OnEnter(Screen::Gameplay),
            spawn_safe_frame.in_set(HudSetup::Frame),
        )
        .add_plugins(context::plugin)
        .add_plugins(badges::plugin)
        .add_plugins((lattice::plugin, initiative::plugin, log::plugin))
        .add_systems(OnEnter(Screen::Gameplay), reset_hud)
        // Deliberately outside `PausableSystems`: hiding chrome does not advance
        // the simulation and remains available while a decision is open.
        .add_systems(
            Update,
            (toggle_hud, apply_hud_visibility, apply_responsive_hud)
                .chain()
                .in_set(AppSystems::RecordInput)
                .run_if(in_state(Screen::Gameplay))
                .run_if(resource_equals(GameplayPhase::Active)),
        );
}

fn apply_responsive_hud(
    metrics: Res<hex_ui::ResolvedUiMetrics>,
    added_regions: Query<(), Added<HudRegion>>,
    mut regions: Query<(&HudRegion, &mut Node)>,
) {
    if !metrics.is_changed() && added_regions.is_empty() {
        return;
    }
    for (region, mut node) in &mut regions {
        let role = match region {
            HudRegion::Party => hex_ui::UiRegionRole::Party,
            HudRegion::Turn => hex_ui::UiRegionRole::Turn,
            HudRegion::Inspector => hex_ui::UiRegionRole::Inspector,
            HudRegion::Actions => hex_ui::UiRegionRole::Actions,
            HudRegion::Events => hex_ui::UiRegionRole::Events,
        };
        hex_ui::apply_region_layout(metrics.viewport, role, &mut node);
    }
}

fn spawn_safe_frame(mut commands: Commands) {
    commands
        .spawn((
            Name::new("Gameplay HUD Safe Frame"),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                right: Val::Px(0.0),
                bottom: Val::Px(0.0),
                left: Val::Px(0.0),
                ..default()
            },
            Pickable::IGNORE,
            crate::screens::DespawnOnExit(Screen::Gameplay),
        ))
        .with_children(|frame| {
            frame.spawn((
                Name::new("Party HUD Region"),
                HudRegion::Party,
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(12.0),
                    bottom: Val::Px(12.0),
                    left: Val::Px(12.0),
                    width: Val::Px(224.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(8.0),
                    ..default()
                },
                Pickable::IGNORE,
            ));
            frame.spawn((
                Name::new("Turn HUD Region"),
                HudRegion::Turn,
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(12.0),
                    left: Val::Px(244.0),
                    right: Val::Px(320.0),
                    height: Val::Px(72.0),
                    ..default()
                },
                Pickable::IGNORE,
            ));
            frame.spawn((
                Name::new("Inspector HUD Region"),
                HudRegion::Inspector,
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(12.0),
                    right: Val::Px(12.0),
                    bottom: Val::Px(12.0),
                    width: Val::Px(300.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(8.0),
                    ..default()
                },
                Pickable::IGNORE,
            ));
            frame.spawn((
                Name::new("Actions HUD Region"),
                HudRegion::Actions,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(244.0),
                    right: Val::Px(320.0),
                    bottom: Val::Px(12.0),
                    height: Val::Px(132.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(4.0),
                    ..default()
                },
                Pickable::IGNORE,
            ));
            frame.spawn((
                Name::new("Events HUD Region"),
                HudRegion::Events,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(244.0),
                    right: Val::Px(320.0),
                    bottom: Val::Px(152.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    ..default()
                },
                Pickable::IGNORE,
            ));
        });
}

/// Finds a region spawned by [`spawn_safe_frame`].
pub(crate) fn region(wanted: HudRegion, regions: &Query<(Entity, &HudRegion)>) -> Option<Entity> {
    regions
        .iter()
        .find_map(|(entity, region)| (*region == wanted).then_some(entity))
}

fn reset_hud(mut hud: ResMut<HudVisibility>) {
    hud.shown = true;
}

fn toggle_hud(
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<hex_core::InputBindings>,
    mut hud: ResMut<HudVisibility>,
) {
    if bindings.just_pressed(&keys, hex_core::InputAction::ToggleHud) {
        hud.shown = !hud.shown;
    }
}

fn apply_hud_visibility(
    hud: Res<HudVisibility>,
    selection: Res<lattice::DisableSelection>,
    resolution: Option<Res<EncounterResolution>>,
    mut roots: Query<(&mut Visibility, Has<DecisionHud>), With<HudElement>>,
) {
    for (mut visibility, decision_lattice) in &mut roots {
        let encounter_complete = resolution
            .as_deref()
            .is_some_and(|value| value.is_resolved());
        let wanted = if encounter_complete && decision_lattice {
            Visibility::Hidden
        } else if hud.shown || (decision_lattice && selection.is_active()) {
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
    fn read_only_hud_surfaces_pass_world_picks_through() {
        assert_eq!(READ_ONLY_HUD, Pickable::IGNORE);
    }

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
            .spawn((HudElement, DecisionHud, Visibility::Inherited))
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

    #[test]
    fn a_resolved_encounter_hides_stale_decision_surfaces() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<HudVisibility>()
            .init_resource::<lattice::DisableSelection>()
            .insert_resource(EncounterResolution(Some(
                hex_combat::EncounterOutcome::Victory,
            )))
            .add_systems(Update, apply_hud_visibility);
        let decision = app
            .world_mut()
            .spawn((HudElement, DecisionHud, Visibility::Inherited))
            .id();

        app.update();

        assert_eq!(
            app.world().get::<Visibility>(decision),
            Some(&Visibility::Hidden)
        );
    }
}
