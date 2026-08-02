//! Responsive gameplay chrome owned entirely by the presentation crate.

use bevy::input_focus::tab_navigation::TabGroup;
use bevy::prelude::*;
use bevy::ui_widgets::ScrollArea;
use hex_core::{AppSystems, Screen};

use crate::{
    layout::constrain_region_to_canvas, DespawnOnExit, GameplayChromeView, HudElement,
    RequiredActionSurface, ResolvedUiMetrics, UiHudSetup, UiRegionRole,
};

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        OnEnter(Screen::Gameplay),
        spawn_safe_frame.in_set(UiHudSetup::Frame),
    )
    .add_systems(
        Update,
        (apply_responsive_layout, apply_visibility)
            .in_set(crate::UiSystems::Render)
            .after(AppSystems::Update)
            .run_if(in_state(Screen::Gameplay)),
    );
}

fn spawn_safe_frame(mut commands: Commands, metrics: Res<ResolvedUiMetrics>) {
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
            TabGroup::new(0),
            DespawnOnExit(Screen::Gameplay),
        ))
        .with_children(|frame| {
            spawn_region(
                frame,
                "Party HUD Region",
                UiRegionRole::Party,
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(8.0),
                    ..region_node()
                },
                metrics.viewport,
            );
            spawn_region(
                frame,
                "Turn HUD Region",
                UiRegionRole::Turn,
                region_node(),
                metrics.viewport,
            );
            spawn_region(
                frame,
                "Inspector HUD Region",
                UiRegionRole::Inspector,
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(8.0),
                    ..region_node()
                },
                metrics.viewport,
            );
            spawn_region(
                frame,
                "Actions HUD Region",
                UiRegionRole::Actions,
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(4.0),
                    ..region_node()
                },
                metrics.viewport,
            );
            spawn_region(
                frame,
                "Events HUD Region",
                UiRegionRole::Events,
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    ..region_node()
                },
                metrics.viewport,
            );
        });
}

fn region_node() -> Node {
    Node {
        position_type: PositionType::Absolute,
        ..default()
    }
}

fn spawn_region(
    frame: &mut ChildSpawnerCommands,
    name: &'static str,
    role: UiRegionRole,
    mut node: Node,
    viewport: crate::UiViewportClass,
) {
    if matches!(role, UiRegionRole::Inspector | UiRegionRole::Actions) {
        node.overflow = Overflow::scroll_y();
    }
    constrain_region_to_canvas(
        ResolvedUiMetrics {
            viewport,
            ..ResolvedUiMetrics::default()
        },
        role,
        &mut node,
    );
    let picking = if matches!(role, UiRegionRole::Inspector | UiRegionRole::Actions) {
        // A ScrollArea must itself participate in picking so wheel/trackpad input
        // over read-only descendants can target it and be consumed before the
        // world camera. Read-only, non-scrollable HUD regions remain transparent.
        Pickable::default()
    } else {
        Pickable::IGNORE
    };
    let mut region = frame.spawn((Name::new(name), role, node, picking));
    if matches!(role, UiRegionRole::Inspector | UiRegionRole::Actions) {
        region.insert((ScrollArea, ScrollPosition::default()));
    }
}

fn apply_responsive_layout(
    metrics: Res<ResolvedUiMetrics>,
    chrome: Res<GameplayChromeView>,
    lattices: Res<crate::GameplayLatticesView>,
    review: Option<Res<crate::review::UiReviewPresentation>>,
    added_regions: Query<(), Added<UiRegionRole>>,
    mut regions: Query<(&UiRegionRole, &mut Node, &mut Pickable)>,
) {
    let review_changed = review.as_ref().is_some_and(|review| review.is_changed());
    if !metrics.is_changed()
        && !chrome.is_changed()
        && !lattices.is_changed()
        && !review_changed
        && added_regions.is_empty()
    {
        return;
    }
    let lattices = review
        .as_ref()
        .and_then(|review| review.lattices.as_ref())
        .unwrap_or(lattices.as_ref());
    let chrome = review
        .as_ref()
        .map_or(*chrome, |review| review.effective_chrome(*chrome));
    let promoted_decision =
        crate::gameplay_lattices::compact_decision_visible(*metrics, &chrome, lattices);
    let ordinary_shown = chrome.shown && !chrome.encounter_complete;
    let decision_required = chrome.decision_required && !chrome.encounter_complete;
    for (role, mut node, mut pickable) in &mut regions {
        constrain_region_to_canvas(*metrics, *role, &mut node);
        let responsive_display = node.display;
        // Layout recomputation restores each region's canonical geometry. Apply
        // phase/user suppression after it so resizing cannot resurrect invisible
        // chrome as an interaction layer over the map.
        let shown = match *role {
            UiRegionRole::Party | UiRegionRole::Turn | UiRegionRole::Events => ordinary_shown,
            UiRegionRole::Inspector => (ordinary_shown || decision_required) && !promoted_decision,
            UiRegionRole::Actions => ordinary_shown || (decision_required && promoted_decision),
        };
        node.display = if shown {
            responsive_display
        } else {
            Display::None
        };
        let participates = node.display != Display::None;
        *pickable =
            if participates && matches!(*role, UiRegionRole::Inspector | UiRegionRole::Actions) {
                Pickable::default()
            } else {
                Pickable::IGNORE
            };
    }
}

fn apply_visibility(
    view: Res<GameplayChromeView>,
    review: Option<Res<crate::review::UiReviewPresentation>>,
    added_roots: Query<(), Added<HudElement>>,
    mut roots: Query<(&mut Visibility, Has<RequiredActionSurface>), With<HudElement>>,
) {
    let review_changed = review.as_ref().is_some_and(|review| review.is_changed());
    if !view.is_changed() && !review_changed && added_roots.is_empty() {
        return;
    }
    let view = review
        .as_ref()
        .map_or(*view, |review| review.effective_chrome(*view));
    for (mut visibility, required_action) in &mut roots {
        let wanted = if view.encounter_complete {
            Visibility::Hidden
        } else if view.shown || (required_action && view.decision_required) {
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
    use bevy::input_focus::tab_navigation::TabGroup;
    use bevy::MinimalPlugins;

    use super::*;

    #[test]
    fn required_actions_survive_hud_hiding_but_not_encounter_completion() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<GameplayChromeView>()
            .add_systems(Update, apply_visibility);
        let ordinary = app
            .world_mut()
            .spawn((HudElement, Visibility::Inherited))
            .id();
        let required = app
            .world_mut()
            .spawn((HudElement, RequiredActionSurface, Visibility::Inherited))
            .id();

        app.world_mut().insert_resource(GameplayChromeView {
            shown: false,
            decision_required: true,
            encounter_complete: false,
        });
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(ordinary),
            Some(&Visibility::Hidden)
        );
        assert_eq!(
            app.world().get::<Visibility>(required),
            Some(&Visibility::Inherited)
        );

        app.world_mut()
            .resource_mut::<GameplayChromeView>()
            .encounter_complete = true;
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(ordinary),
            Some(&Visibility::Hidden)
        );
        assert_eq!(
            app.world().get::<Visibility>(required),
            Some(&Visibility::Hidden)
        );
    }

    #[test]
    fn gameplay_controls_share_a_real_tab_navigation_group() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<ResolvedUiMetrics>()
            .add_systems(Update, spawn_safe_frame);
        app.update();

        let mut groups = app.world_mut().query::<(&Name, &TabGroup)>();
        assert!(groups
            .iter(app.world())
            .any(|(name, group)| name.as_str() == "Gameplay HUD Safe Frame" && !group.modal));
    }

    #[test]
    fn hidden_chrome_releases_every_region_from_layout_and_picking_after_resize() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<ResolvedUiMetrics>()
            .init_resource::<GameplayChromeView>()
            .init_resource::<crate::GameplayLatticesView>()
            .add_systems(Update, apply_responsive_layout);
        for role in [
            UiRegionRole::Party,
            UiRegionRole::Turn,
            UiRegionRole::Inspector,
            UiRegionRole::Actions,
            UiRegionRole::Events,
        ] {
            app.world_mut()
                .spawn((role, Node::default(), Pickable::default()));
        }

        app.world_mut().insert_resource(GameplayChromeView {
            shown: false,
            decision_required: false,
            encounter_complete: false,
        });
        app.update();
        app.world_mut()
            .resource_mut::<ResolvedUiMetrics>()
            .logical_size = Vec2::new(960.0, 540.0);
        app.update();

        let mut regions = app.world_mut().query::<(&Node, &Pickable)>();
        for (node, pickable) in regions.iter(app.world()) {
            assert_eq!(node.display, Display::None);
            assert_eq!(*pickable, Pickable::IGNORE);
        }
    }
}
