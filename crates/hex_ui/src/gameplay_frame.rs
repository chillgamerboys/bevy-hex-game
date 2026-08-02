//! Responsive gameplay chrome owned entirely by the presentation crate.

use bevy::input_focus::tab_navigation::TabGroup;
use bevy::prelude::*;
use bevy::ui_widgets::ScrollArea;
use hex_core::{AppSystems, Screen};
use hex_gameplay_model::MainViewDestination;

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
    node.overflow = Overflow::scroll_y();
    constrain_region_to_canvas(
        ResolvedUiMetrics {
            viewport,
            ..ResolvedUiMetrics::default()
        },
        role,
        &mut node,
    );
    // Every visible surface may become the Compact layout's sole full-screen
    // scroll owner. Hidden regions are removed from picking by `apply_responsive_layout`.
    frame.spawn((
        Name::new(name),
        role,
        node,
        Pickable::default(),
        ScrollArea,
        ScrollPosition::default(),
    ));
}

fn apply_responsive_layout(
    metrics: Res<ResolvedUiMetrics>,
    chrome: Res<GameplayChromeView>,
    review: Option<Res<crate::review::UiReviewPresentation>>,
    added_regions: Query<(), Added<UiRegionRole>>,
    mut regions: Query<(&UiRegionRole, &mut Node, &mut Pickable)>,
) {
    let review_changed = review.as_ref().is_some_and(|review| review.is_changed());
    if !metrics.is_changed() && !chrome.is_changed() && !review_changed && added_regions.is_empty()
    {
        return;
    }
    let chrome = review
        .as_ref()
        .map_or(*chrome, |review| review.effective_chrome(*chrome));
    for (role, mut node, mut pickable) in &mut regions {
        constrain_region_to_canvas(*metrics, *role, &mut node);
        let responsive_display = if metrics.viewport == crate::UiViewportClass::Compact {
            Display::Flex
        } else {
            node.display
        };
        // Layout recomputation restores each region's canonical geometry. Apply
        // phase/user suppression after it so resizing cannot resurrect invisible
        // chrome as an interaction layer over the map.
        let shown = !chrome.encounter_complete
            && match *role {
                UiRegionRole::Party => chrome.party_shown,
                UiRegionRole::Turn => chrome.initiative_shown,
                UiRegionRole::Inspector => !matches!(chrome.main_view, MainViewDestination::Closed),
                UiRegionRole::Actions => chrome.action_bar_shown,
                UiRegionRole::Events => chrome.activity_shown,
            };
        if shown && metrics.viewport == crate::UiViewportClass::Compact {
            make_compact_task_surface(&mut node);
        }
        node.display = if shown {
            responsive_display
        } else {
            Display::None
        };
        let participates = node.display != Display::None;
        *pickable = if participates {
            Pickable::default()
        } else {
            Pickable::IGNORE
        };
    }
}

fn make_compact_task_surface(node: &mut Node) {
    node.position_type = PositionType::Absolute;
    node.top = Val::Px(8.0);
    node.right = Val::Px(8.0);
    node.bottom = Val::Px(8.0);
    node.left = Val::Px(8.0);
    node.width = Val::Auto;
    node.height = Val::Auto;
    node.overflow = Overflow::scroll_y();
    node.flex_direction = FlexDirection::Column;
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
        } else if view.any_ordinary_shown() || (required_action && view.decision_required()) {
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
            party_shown: false,
            initiative_shown: false,
            activity_shown: false,
            action_bar_shown: false,
            main_view: MainViewDestination::RequiredDecision,
            terrain_health_shown: false,
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
            party_shown: false,
            initiative_shown: false,
            activity_shown: false,
            action_bar_shown: false,
            main_view: MainViewDestination::Closed,
            terrain_health_shown: false,
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
