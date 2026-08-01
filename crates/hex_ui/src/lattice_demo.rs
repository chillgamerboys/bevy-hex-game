//! Isolated lattice-rules screen presentation.

use bevy::prelude::*;
use hex_core::Screen;

use crate::{
    blurb, display, divider, fine, heading, label, panel, screen_root, small_button,
    spawn_lattice_cells, LatticeDemoIntent, LatticeDemoView, LatticeScale, ResolvedUiMetrics,
    UiAssets, UiIntent, UiSystems, UiViewportClass, SMALL_BUTTON_WIDTH,
};

#[derive(Component)]
struct Body;

#[derive(Component, Clone, Copy)]
struct Control(LatticeDemoIntent);

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Screen::LatticeDemo), spawn)
        .add_systems(
            Update,
            (
                rebuild,
                apply_layout,
                emit_intents.in_set(UiSystems::EmitIntents),
            )
                .run_if(in_state(Screen::LatticeDemo)),
        );
}

fn spawn(mut commands: Commands, assets: Res<UiAssets>) {
    commands
        .spawn(screen_root(Screen::LatticeDemo, "Lattice Demo Screen"))
        .with_children(|parent| {
            parent.spawn(display(&assets, "The Lattice"));
            parent.spawn((
                Name::new("Demo Body"),
                Body,
                Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(32.0),
                    row_gap: Val::Px(18.0),
                    align_items: AlignItems::Stretch,
                    max_width: Val::Percent(96.0),
                    max_height: Val::Percent(82.0),
                    ..default()
                },
            ));
            parent.spawn(blurb(
                &assets,
                "cast from the right panel · click a gem to strike it · BACKSPACE to return",
            ));
        });
}

fn rebuild(
    mut commands: Commands,
    view: Res<LatticeDemoView>,
    bodies: Query<Entity, With<Body>>,
    assets: Res<UiAssets>,
) {
    if !view.is_changed() {
        return;
    }
    let Ok(body) = bodies.single() else { return };
    commands.entity(body).despawn_related::<Children>();
    commands.entity(body).with_children(|panels| {
        if !view.ready {
            panels.spawn(blurb(&assets, "waiting for content..."));
            return;
        }
        if view.cells.is_empty() {
            panels.spawn(blurb(&assets, "the content defined no demo lattice"));
        } else {
            panels
                .spawn((Name::new("Lattice Panel"), panel()))
                .with_children(|framed| {
                    framed.spawn(heading(&assets, "the inscription"));
                    spawn_lattice_cells(
                        framed,
                        &view.cells,
                        &assets,
                        LatticeScale::DEMO,
                        "Demo",
                        |coord| Control(LatticeDemoIntent::ActivateCell(coord)),
                    );
                });
        }
        spawn_controls(panels, &assets, &view);
    });
}

fn spawn_controls(parent: &mut ChildSpawnerCommands, assets: &UiAssets, view: &LatticeDemoView) {
    parent
        .spawn((Name::new("Demo Controls"), panel()))
        .insert(Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(10.0),
            width: Val::Px(470.0),
            max_width: Val::Percent(100.0),
            padding: UiRect::all(Val::Px(18.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(10.0)),
            ..default()
        })
        .with_children(|controls| {
            controls.spawn(heading(assets, "spells"));
            for spell in &view.spells {
                controls
                    .spawn((
                        Name::new(format!("Spell Row {}", spell.name)),
                        Node {
                            min_height: Val::Px(50.0),
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(14.0),
                            align_items: AlignItems::Center,
                            ..default()
                        },
                    ))
                    .with_children(|row| {
                        if let Some(cost) = spell.cost {
                            row.spawn((
                                small_button(format!("Cast {}", spell.name)),
                                Control(LatticeDemoIntent::Cast(spell.coord)),
                            ))
                            .with_children(|cast| {
                                cast.spawn(blurb(assets, "cast"));
                                cast.spawn(fine(assets, format!("{cost} mana")));
                            });
                        } else {
                            row.spawn((
                                Name::new(format!("{} Blocked Reason", spell.name)),
                                Node {
                                    width: Val::Px(SMALL_BUTTON_WIDTH),
                                    ..default()
                                },
                                children![fine(
                                    assets,
                                    format!(
                                        "blocked · {}",
                                        spell.blocked.as_deref().unwrap_or("unavailable")
                                    )
                                )],
                            ));
                        }
                        row.spawn((
                            Node {
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(2.0),
                                ..default()
                            },
                            children![
                                label(assets, spell.headline.clone()),
                                fine(assets, spell.kind.clone())
                            ],
                        ));
                    });
            }
            controls.spawn(divider(430.0));
            controls.spawn(blurb(assets, view.totals.clone()));
            controls
                .spawn((
                    Name::new("Demo Actions"),
                    Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(12.0),
                        ..default()
                    },
                ))
                .with_children(|actions| {
                    actions
                        .spawn((
                            small_button("End Turn"),
                            Control(LatticeDemoIntent::EndTurn),
                        ))
                        .with_children(|action| {
                            action.spawn(blurb(assets, "end turn"));
                            action.spawn(fine(assets, "channel mana"));
                        });
                    actions
                        .spawn((small_button("Reset"), Control(LatticeDemoIntent::Reset)))
                        .with_children(|action| {
                            action.spawn(blurb(assets, "reset"));
                            action.spawn(fine(assets, "fresh state"));
                        });
                });
            for line in &view.log {
                controls.spawn(fine(assets, format!("· {line}")));
            }
        });
}

fn apply_layout(
    metrics: Res<ResolvedUiMetrics>,
    added: Query<(), Added<Body>>,
    mut bodies: Query<&mut Node, With<Body>>,
) {
    if !metrics.is_changed() && added.is_empty() {
        return;
    }
    for mut node in &mut bodies {
        let compact = metrics.viewport == UiViewportClass::Compact;
        node.flex_direction = if compact {
            FlexDirection::Column
        } else {
            FlexDirection::Row
        };
        node.overflow = if compact {
            Overflow::scroll_y()
        } else {
            Overflow::default()
        };
    }
}

fn emit_intents(
    clicked: Query<(&Interaction, &Control), Changed<Interaction>>,
    mut intents: MessageWriter<UiIntent>,
) {
    for (interaction, control) in &clicked {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::LatticeDemo(control.0));
        }
    }
}
