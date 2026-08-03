//! Player and disclosed-target lattice panels from immutable projections.

use bevy::{
    ecs::system::SystemParam,
    input_focus::{tab_navigation::TabGroup, InputFocus},
    prelude::*,
};
use hex_core::Screen;
use hex_gameplay_model::MainViewDestination;

use crate::{
    blurb, fine, hud_heading, panel, row_button, spawn_lattice_cells, GameplayLatticesView,
    HudElement, LatticeIntent, LatticeScale, RequiredActionSurface, TargetLatticeStateView,
    TargetPulseView, UiAssets, UiHudSetup, UiIntent, UiRegionRole, UiSystems, EDGE, PANEL_BG,
    READ_ONLY_HUD,
};

#[derive(Component)]
struct OwnBody;

#[derive(Component)]
struct OwnPanel;

#[derive(Component)]
struct OwnHeading;

#[derive(Component)]
struct TargetPanel;

#[derive(Component)]
struct TargetBody;

#[derive(Component)]
struct TargetHeading;

#[derive(Component)]
struct LatticeReadoutStack;

#[derive(Component)]
struct CompactDecisionPanel;

#[derive(Component)]
struct CompactDecisionBody;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct OwnCell(hex_core::LatticeCoord);

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
enum DecisionControl {
    Clear,
    Confirm,
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        OnEnter(Screen::Gameplay),
        spawn_panels.in_set(UiHudSetup::Panels),
    )
    .add_systems(
        Update,
        (
            (sync_required_decision_focus_scope, rebuild)
                .chain()
                .in_set(UiSystems::Render),
            emit_intents.in_set(UiSystems::EmitIntents),
        )
            .run_if(in_state(Screen::Gameplay)),
    );
}

fn spawn_panels(
    mut commands: Commands,
    assets: Res<UiAssets>,
    regions: Query<(Entity, &UiRegionRole)>,
) {
    let stack = commands
        .spawn((
            Name::new("Lattice Readout Stack"),
            LatticeReadoutStack,
            Node {
                width: Val::Percent(100.0),
                flex_shrink: 0.0,
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|stack| {
            stack
                .spawn((
                    Name::new("Own Lattice Panel"),
                    OwnPanel,
                    RequiredActionSurface,
                    HudElement,
                    panel(),
                    READ_ONLY_HUD,
                ))
                .insert(panel_node(Display::Flex))
                .with_children(|panel| {
                    panel.spawn((OwnHeading, hud_heading(&assets, "selected ally")));
                    panel.spawn((
                        Name::new("Own Lattice Body"),
                        OwnBody,
                        body_node(),
                        Pickable::IGNORE,
                    ));
                });
            stack
                .spawn((
                    Name::new("Target Lattice Panel"),
                    TargetPanel,
                    HudElement,
                    panel(),
                    READ_ONLY_HUD,
                ))
                .insert(panel_node(Display::None))
                .with_children(|panel| {
                    panel.spawn((TargetHeading, hud_heading(&assets, "aim target")));
                    panel.spawn((
                        Name::new("Target Lattice Body"),
                        TargetBody,
                        body_node(),
                        Pickable::IGNORE,
                    ));
                });
        })
        .id();
    if let Some(inspector) = regions
        .iter()
        .find_map(|(entity, role)| (*role == UiRegionRole::Inspector).then_some(entity))
    {
        commands.entity(inspector).add_child(stack);
    }
    if let Some(actions) = regions
        .iter()
        .find_map(|(entity, role)| (*role == UiRegionRole::Actions).then_some(entity))
    {
        let compact_decision = commands
            .spawn((
                Name::new("Compact Required Lattice Choice"),
                CompactDecisionPanel,
                RequiredActionSurface,
                HudElement,
                compact_decision_node(Display::None),
                BorderColor::all(EDGE),
                BackgroundColor(PANEL_BG),
            ))
            .with_child((
                Name::new("Compact Required Lattice Body"),
                CompactDecisionBody,
                Node {
                    width: Val::Percent(100.0),
                    min_height: Val::Px(0.0),
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(12.0),
                    ..default()
                },
                Pickable::IGNORE,
            ))
            .id();
        commands.entity(actions).add_child(compact_decision);
    }
}

fn panel_node(display: Display) -> Node {
    Node {
        display,
        width: Val::Percent(100.0),
        flex_direction: FlexDirection::Column,
        padding: UiRect::all(Val::Px(12.0)),
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(10.0)),
        row_gap: Val::Px(7.0),
        ..default()
    }
}

fn body_node() -> Node {
    Node {
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(5.0),
        ..default()
    }
}

fn compact_decision_node(display: Display) -> Node {
    Node {
        display,
        position_type: PositionType::Absolute,
        top: Val::Px(0.0),
        right: Val::Px(0.0),
        bottom: Val::Px(0.0),
        left: Val::Px(0.0),
        padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(10.0)),
        ..default()
    }
}

fn sync_required_decision_focus_scope(
    mut commands: Commands,
    chrome: Res<crate::GameplayChromeView>,
    review: Option<Res<crate::review::UiReviewPresentation>>,
    metrics: Res<crate::ResolvedUiMetrics>,
    panels: Query<(Entity, Has<crate::focus::ModalFocusScope>), With<OwnPanel>>,
) {
    let chrome = review.as_ref().map_or(*chrome, |review| {
        review.effective_chrome(*chrome, metrics.viewport)
    });
    for (panel, owns_modal_focus) in &panels {
        if chrome.decision_required() && !owns_modal_focus {
            commands.entity(panel).insert((
                TabGroup {
                    order: 20,
                    modal: true,
                },
                crate::focus::ModalFocusScope,
            ));
        } else if !chrome.decision_required() && owns_modal_focus {
            commands
                .entity(panel)
                .remove::<TabGroup>()
                .remove::<crate::focus::ModalFocusScope>();
        }
    }
}

#[derive(SystemParam)]
struct DecisionFocus<'w, 's> {
    focus: ResMut<'w, InputFocus>,
    parents: Query<'w, 's, &'static ChildOf>,
    names: Query<'w, 's, &'static Name>,
    refreshes: ResMut<'w, crate::focus::FocusRefreshRequests>,
}

#[expect(
    clippy::too_many_arguments,
    reason = "the renderer updates two independently scoped panels from one atomic view"
)]
fn rebuild(
    mut commands: Commands,
    view: Res<GameplayLatticesView>,
    review: Option<Res<crate::review::UiReviewPresentation>>,
    pulse: Res<TargetPulseView>,
    own_bodies: Query<Entity, With<OwnBody>>,
    target_bodies: Query<Entity, With<TargetBody>>,
    compact_bodies: Query<Entity, With<CompactDecisionBody>>,
    mut stacks: Query<
        &mut Node,
        (
            With<LatticeReadoutStack>,
            Without<CompactDecisionPanel>,
            Without<TargetPanel>,
        ),
    >,
    mut compact_panels: Query<&mut Node, (With<CompactDecisionPanel>, Without<TargetPanel>)>,
    mut target_panels: Query<
        (&mut Node, &mut BackgroundColor),
        (With<TargetPanel>, Without<CompactDecisionPanel>),
    >,
    mut own_headings: Query<&mut Text, (With<OwnHeading>, Without<TargetHeading>)>,
    mut target_headings: Query<&mut Text, (With<TargetHeading>, Without<OwnHeading>)>,
    assets: Res<UiAssets>,
    metrics: Res<crate::ResolvedUiMetrics>,
    chrome: Res<crate::GameplayChromeView>,
    mut decision_focus: DecisionFocus,
) {
    let view_changed = view.is_changed();
    let review_changed = review.as_ref().is_some_and(|review| review.is_changed());
    let chrome_changed = chrome.is_changed();
    if !view_changed
        && !review_changed
        && !pulse.is_changed()
        && !metrics.is_changed()
        && !chrome_changed
    {
        return;
    }
    let view = review
        .as_ref()
        .and_then(|review| review.lattices.as_ref())
        .unwrap_or(view.as_ref());
    let chrome = review.as_ref().map_or(*chrome, |review| {
        review.effective_chrome(*chrome, metrics.viewport)
    });
    if let Ok(mut stack) = stacks.single_mut() {
        stack.display = if matches!(
            chrome.main_view,
            MainViewDestination::Character(_) | MainViewDestination::RequiredDecision
        ) {
            Display::Flex
        } else {
            Display::None
        };
    }
    let compact_decision = compact_decision_visible(*metrics, &chrome, view);
    if let Ok(mut panel) = compact_panels.single_mut() {
        panel.display = if compact_decision {
            Display::Flex
        } else {
            Display::None
        };
    }
    if let Ok((mut node, mut background)) = target_panels.single_mut() {
        node.display = if view.target.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        background.0 = if pulse.0 {
            Color::srgba(0.25, 0.10, 0.06, 0.9)
        } else {
            PANEL_BG
        };
    }
    if !view_changed && !review_changed && !metrics.is_changed() && !chrome_changed {
        return;
    }
    if let Ok(body) = compact_bodies.single() {
        commands.entity(body).despawn_related::<Children>();
        if compact_decision {
            let Some((own, decision)) = view
                .own
                .as_ref()
                .and_then(|own| own.decision.as_ref().map(|decision| (own, decision)))
            else {
                return;
            };
            commands.entity(body).with_children(|body| {
                let ultra_constrained = crate::layout::is_ultra_constrained(*metrics);
                body.spawn((
                    Name::new("Compact Required Lattice Summary"),
                    Node {
                        width: Val::Px(if ultra_constrained { 160.0 } else { 188.0 }),
                        min_width: Val::Px(if ultra_constrained { 160.0 } else { 188.0 }),
                        min_height: Val::Px(72.0),
                        height: Val::Auto,
                        flex_shrink: 0.0,
                        flex_direction: FlexDirection::Column,
                        justify_content: JustifyContent::Center,
                        row_gap: Val::Px(3.0),
                        ..default()
                    },
                    Pickable::IGNORE,
                ))
                .with_children(|summary| {
                    summary.spawn(blurb(
                        &assets,
                        if decision.restoring {
                            "RESTORE CELL"
                        } else if ultra_constrained {
                            "SELECT CELL"
                        } else {
                            "SELECT LIVE CELL"
                        },
                    ));
                    summary.spawn(fine(
                        &assets,
                        format!("{} / {} selected", decision.chosen, decision.owed,),
                    ));
                });
                spawn_lattice_cells(
                    body,
                    &own.cells,
                    &assets,
                    if ultra_constrained {
                        LatticeScale::TIGHT
                    } else {
                        LatticeScale::PANEL
                    },
                    metrics.control_scale,
                    "Compact Required",
                    OwnCell,
                );
            });
        }
    }
    if let Ok(body) = own_bodies.single() {
        crate::focus::begin_route_refresh(
            body,
            &mut decision_focus.focus,
            &decision_focus.parents,
            &decision_focus.names,
            &mut decision_focus.refreshes,
        );
        commands.entity(body).despawn_related::<Children>();
        commands.entity(body).with_children(|body| {
            let Some(own) = view.own.as_ref() else {
                body.spawn(blurb(&assets, "no player lattice"));
                return;
            };
            body.spawn(blurb(&assets, own.identity.clone()));
            spawn_lattice_cells(
                body,
                &own.cells,
                &assets,
                LatticeScale::PANEL,
                metrics.control_scale,
                "Own",
                OwnCell,
            );
            if let Some(decision) = own.decision.as_ref() {
                spawn_decision_controls(body, decision, &assets);
            }
        });
    }
    if let (Ok(mut heading), Some(own)) = (own_headings.single_mut(), view.own.as_ref()) {
        heading.0.clone_from(&own.heading);
    }
    if let Ok(body) = target_bodies.single() {
        commands.entity(body).despawn_related::<Children>();
        commands.entity(body).with_children(|body| {
            let Some(target) = view.target.as_ref() else {
                return;
            };
            body.spawn(blurb(&assets, target.identity.clone()));
            match &target.state {
                TargetLatticeStateView::Opaque => {
                    body.spawn(blurb(&assets, "lattice unknown"));
                }
                TargetLatticeStateView::Known { cells, unknown } => {
                    spawn_lattice_cells(
                        body,
                        cells,
                        &assets,
                        LatticeScale::PANEL,
                        metrics.control_scale,
                        "Target",
                        |_| (),
                    );
                    if let Some(unknown) = unknown.filter(|unknown| *unknown > 0) {
                        body.spawn(fine(&assets, format!("{unknown} cells unknown")));
                    }
                }
            }
        });
    }
    if let (Ok(mut heading), Some(target)) = (target_headings.single_mut(), view.target.as_ref()) {
        heading.0.clone_from(&target.heading);
    }
}

pub(crate) fn compact_decision_visible(
    _metrics: crate::ResolvedUiMetrics,
    _chrome: &crate::GameplayChromeView,
    _view: &GameplayLatticesView,
) -> bool {
    // Required decisions now own the Compact full-screen Main View. Keeping a
    // second promoted copy in the Action Bar would create two competing focus scopes.
    false
}

/// Adds the shared clear/confirm affordances to any required-decision surface.
pub fn spawn_decision_controls(
    body: &mut ChildSpawnerCommands,
    decision: &crate::DecisionChoiceView,
    assets: &UiAssets,
) {
    body.spawn(fine(
        assets,
        format!(
            "{}/{} {} cells chosen",
            decision.chosen,
            decision.owed,
            if decision.restoring {
                "disabled"
            } else {
                "live"
            }
        ),
    ));
    body.spawn((
        Name::new("Disable Decision Controls"),
        Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(8.0),
            ..default()
        },
        Pickable::IGNORE,
    ))
    .with_children(|controls| {
        controls
            .spawn((
                row_button("Clear Disable Selection", 119.0),
                DecisionControl::Clear,
            ))
            .with_children(|button| {
                button.spawn(blurb(assets, "clear"));
            });
        if decision.chosen == decision.owed {
            controls
                .spawn((
                    row_button("Confirm Disable Selection", 119.0),
                    DecisionControl::Confirm,
                ))
                .with_children(|button| {
                    button.spawn(blurb(assets, "confirm"));
                    button.spawn(fine(assets, decision.confirm_shortcut.clone()));
                });
        } else {
            controls
                .spawn((
                    Name::new("Confirm Disable Selection Disabled"),
                    Node {
                        width: Val::Px(119.0),
                        height: Val::Px(48.0),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(1.0),
                        padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(6.0)),
                        ..default()
                    },
                    BorderColor::all(EDGE),
                    BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.03)),
                    Pickable::IGNORE,
                ))
                .with_children(|button| {
                    button.spawn(blurb(assets, "confirm"));
                    button.spawn(fine(assets, "choose more"));
                });
        }
    });
}

fn emit_intents(
    cells: Query<(&Interaction, &OwnCell), Changed<Interaction>>,
    controls: Query<(&Interaction, &DecisionControl), Changed<Interaction>>,
    mut intents: MessageWriter<UiIntent>,
) {
    for (interaction, cell) in &cells {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::Lattice(LatticeIntent::ToggleCell(cell.0)));
        }
    }
    for (interaction, control) in &controls {
        if *interaction != Interaction::Pressed {
            continue;
        }
        intents.write(UiIntent::Lattice(match control {
            DecisionControl::Clear => LatticeIntent::ClearDecision,
            DecisionControl::Confirm => LatticeIntent::ConfirmDecision,
        }));
    }
}

#[cfg(test)]
mod tests {
    use bevy::input_focus::tab_navigation::TabIndex;
    use bevy::MinimalPlugins;

    use super::*;
    use crate::CellInteraction;

    fn actionable_cell(coord: hex_core::LatticeCoord) -> crate::LatticeCellView {
        crate::LatticeCellView {
            coord,
            label: "Fire".to_owned(),
            detail: "2 mana".to_owned(),
            color: Color::srgb(0.3, 0.2, 0.1),
            known_mana: Some(2),
            known_locked: Some(false),
            disabled: false,
            selected: false,
            interaction: CellInteraction::Actionable,
        }
    }

    #[test]
    fn required_main_view_never_creates_a_duplicate_compact_lattice_choice() {
        let mut view = GameplayLatticesView::default();
        let mut chrome = crate::GameplayChromeView::default();
        let ultra =
            crate::resolve_ui_metrics(Vec2::new(1280.0, 720.0), crate::UiScaleMode::Percent200);
        assert!(!compact_decision_visible(ultra, &chrome, &view));

        view.own = Some(crate::OwnLatticeView {
            heading: "required choice".to_owned(),
            identity: "player".to_owned(),
            cells: Vec::new(),
            decision: Some(crate::DecisionChoiceView {
                chosen: 1,
                owed: 2,
                restoring: false,
                confirm_shortcut: "Enter".to_owned(),
            }),
        });
        assert!(
            !compact_decision_visible(ultra, &chrome, &view),
            "a stale lattice projection cannot promote before the application names a blocking decision"
        );
        chrome.main_view = MainViewDestination::RequiredDecision;
        assert!(
            !compact_decision_visible(ultra, &chrome, &view),
            "the forced Main View is the single Compact decision surface"
        );

        let ordinary_compact =
            crate::resolve_ui_metrics(Vec2::new(1280.0, 720.0), crate::UiScaleMode::Auto);
        assert_eq!(ordinary_compact.viewport, crate::UiViewportClass::Compact);
        assert!(!compact_decision_visible(ordinary_compact, &chrome, &view));
    }

    #[test]
    fn required_decision_owns_an_accessibly_named_modal_focus_scope() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<InputFocus>()
            .init_resource::<crate::focus::FocusRefreshRequests>()
            .init_resource::<TargetPulseView>()
            .insert_resource(crate::ResolvedUiMetrics::default())
            .insert_resource(UiAssets {
                display: Handle::default(),
                body: Handle::default(),
                hex_cell: Handle::default(),
            })
            .insert_resource(crate::GameplayChromeView {
                main_view: MainViewDestination::RequiredDecision,
                ..default()
            })
            .insert_resource(GameplayLatticesView {
                own: Some(crate::OwnLatticeView {
                    heading: "required choice".to_owned(),
                    identity: "Hedge Mage".to_owned(),
                    cells: vec![
                        actionable_cell(hex_core::LatticeCoord::ORIGIN),
                        actionable_cell(hex_core::LatticeCoord::new(1, 0)),
                    ],
                    decision: Some(crate::DecisionChoiceView {
                        chosen: 0,
                        owed: 2,
                        restoring: false,
                        confirm_shortcut: "Enter".to_owned(),
                    }),
                }),
                target: None,
            })
            .add_systems(Startup, spawn_panels)
            .add_systems(
                Update,
                (sync_required_decision_focus_scope, rebuild).chain(),
            );
        app.world_mut().spawn(UiRegionRole::Inspector);
        app.world_mut().spawn(UiRegionRole::Actions);

        app.update();

        let panel = app
            .world_mut()
            .query_filtered::<Entity, With<OwnPanel>>()
            .single(app.world())
            .unwrap();
        assert!(app
            .world()
            .get::<crate::focus::ModalFocusScope>(panel)
            .is_some());
        assert!(app
            .world()
            .get::<TabGroup>(panel)
            .is_some_and(|group| group.modal));
        let cells = {
            let world = app.world_mut();
            let mut query =
                world.query_filtered::<(&Name, &AccessibleLabel, &TabIndex), With<OwnCell>>();
            query
                .iter(world)
                .map(|(name, label, index)| (name.as_str().to_owned(), label.0.clone(), index.0))
                .collect::<Vec<_>>()
        };
        assert_eq!(cells.len(), 2);
        assert!(cells.iter().all(|(name, label, index)| {
            name.starts_with("Own Cell (")
                && label.contains("lattice cell")
                && !label.is_empty()
                && *index == 0
        }));

        app.world_mut()
            .resource_mut::<crate::GameplayChromeView>()
            .main_view = MainViewDestination::Closed;
        app.update();

        assert!(app
            .world()
            .get::<crate::focus::ModalFocusScope>(panel)
            .is_none());
        assert!(app.world().get::<TabGroup>(panel).is_none());
    }
}
