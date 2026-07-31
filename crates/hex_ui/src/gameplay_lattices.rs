//! Player and disclosed-target lattice panels from immutable projections.

use bevy::prelude::*;
use hex_core::Screen;

use crate::{
    blurb, fine, heading, panel, row_button, spawn_lattice_cells, GameplayLatticesView, HudElement,
    LatticeIntent, LatticeScale, RequiredActionSurface, TargetLatticeStateView, TargetPulseView,
    UiAssets, UiHudSetup, UiIntent, UiRegionRole, UiSystems, EDGE, PANEL_BG, READ_ONLY_HUD,
};

#[derive(Component)]
struct OwnBody;

#[derive(Component)]
struct OwnHeading;

#[derive(Component)]
struct TargetPanel;

#[derive(Component)]
struct TargetBody;

#[derive(Component)]
struct TargetHeading;

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
        (rebuild, emit_intents.in_set(UiSystems::EmitIntents)).run_if(in_state(Screen::Gameplay)),
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
            Node {
                width: Val::Percent(100.0),
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
                    RequiredActionSurface,
                    HudElement,
                    panel(),
                    READ_ONLY_HUD,
                ))
                .insert(panel_node(Display::Flex))
                .with_children(|panel| {
                    panel.spawn((OwnHeading, heading(&assets, "selected ally")));
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
                    panel.spawn((TargetHeading, heading(&assets, "aim target")));
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

#[expect(
    clippy::too_many_arguments,
    reason = "the renderer updates two independently scoped panels from one atomic view"
)]
fn rebuild(
    mut commands: Commands,
    view: Res<GameplayLatticesView>,
    pulse: Res<TargetPulseView>,
    own_bodies: Query<Entity, With<OwnBody>>,
    target_bodies: Query<Entity, With<TargetBody>>,
    mut target_panels: Query<(&mut Node, &mut BackgroundColor), With<TargetPanel>>,
    mut own_headings: Query<&mut Text, (With<OwnHeading>, Without<TargetHeading>)>,
    mut target_headings: Query<&mut Text, (With<TargetHeading>, Without<OwnHeading>)>,
    assets: Res<UiAssets>,
) {
    if !view.is_changed() && !pulse.is_changed() {
        return;
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
    if !view.is_changed() {
        return;
    }
    if let Ok(body) = own_bodies.single() {
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
                "Own",
                OwnCell,
            );
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

/// Adds the shared clear/confirm affordances to any required-decision surface.
pub fn spawn_decision_controls(
    body: &mut ChildSpawnerCommands,
    decision: crate::DecisionChoiceView,
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
                    button.spawn(fine(assets, "ENTER"));
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
