//! Isolated lattice-rules screen presentation.

use bevy::prelude::*;
use bevy::ui_widgets::ScrollArea;
use hex_core::Screen;

use crate::{
    blurb, button, display, divider, fine, heading, label, panel, screen_root, small_button,
    spawn_lattice_cells, LatticeDemoIntent, LatticeDemoView, LatticeScale, ResolvedUiMetrics,
    UiAssets, UiIntent, UiSystems, UiViewportClass, SMALL_BUTTON_WIDTH,
};

#[derive(Component)]
struct Body;

#[derive(Component)]
struct ControlsHeading;

#[derive(Component, Clone, Copy)]
struct Control(LatticeDemoIntent);

#[derive(Component)]
struct BackControl;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Screen::LatticeDemo), spawn)
        .add_systems(
            Update,
            (
                (rebuild, apply_layout).chain().in_set(UiSystems::Render),
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
            parent
                .spawn((
                    button("Back"),
                    BackControl,
                    crate::UiVisibilityRequirement::Immediate,
                ))
                .with_child(label(&assets, "Back"));
            parent.spawn((
                Name::new("Demo Body"),
                Body,
                ScrollArea,
                ScrollPosition::default(),
                Node {
                    width: Val::Percent(96.0),
                    min_height: Val::Px(0.0),
                    flex_basis: Val::Px(0.0),
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(32.0),
                    row_gap: Val::Px(18.0),
                    align_items: AlignItems::Stretch,
                    max_width: Val::Percent(96.0),
                    ..default()
                },
            ));
        });
}

fn rebuild(
    mut commands: Commands,
    view: Res<LatticeDemoView>,
    metrics: Res<ResolvedUiMetrics>,
    bodies: Query<Entity, With<Body>>,
    assets: Res<UiAssets>,
) {
    if !view.is_changed() && !metrics.is_changed() {
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
            if metrics.viewport == UiViewportClass::Compact {
                spawn_controls(panels, &assets, &view, true);
                spawn_lattice_panel(panels, &assets, &view, metrics.control_scale);
            } else {
                spawn_lattice_panel(panels, &assets, &view, metrics.control_scale);
                spawn_controls(panels, &assets, &view, false);
            }
            return;
        }
        spawn_controls(
            panels,
            &assets,
            &view,
            metrics.viewport == UiViewportClass::Compact,
        );
    });
}

fn spawn_lattice_panel(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    view: &LatticeDemoView,
    control_scale: f32,
) {
    parent
        .spawn((Name::new("Lattice Panel"), panel()))
        .with_children(|framed| {
            framed.spawn(heading(assets, "the inscription"));
            spawn_lattice_cells(
                framed,
                &view.cells,
                assets,
                LatticeScale::DEMO,
                control_scale,
                "Demo",
                |coord| {
                    (
                        Control(LatticeDemoIntent::ActivateCell(coord)),
                        crate::UiVisibilityRequirement::Scrollable,
                    )
                },
            );
        });
}

fn spawn_controls(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    view: &LatticeDemoView,
    compact: bool,
) {
    parent
        .spawn((Name::new("Demo Controls"), panel()))
        .insert(Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(10.0),
            // At high semantic scales the compact layout has ample horizontal
            // room but very little vertical room. Let the primary-action row
            // use that width so End Turn and Reset do not wrap and push the
            // second spell below the initial viewport.
            width: if compact {
                Val::Percent(100.0)
            } else {
                Val::Px(470.0)
            },
            max_width: if compact {
                Val::Px(760.0)
            } else {
                Val::Percent(100.0)
            },
            padding: UiRect::all(Val::Px(18.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(10.0)),
            ..default()
        })
        .with_children(|controls| {
            controls.spawn((ControlsHeading, heading(assets, "spells")));
            controls
                .spawn((
                    Name::new("Demo Actions"),
                    Node {
                        flex_direction: FlexDirection::Row,
                        flex_wrap: FlexWrap::Wrap,
                        column_gap: Val::Px(12.0),
                        row_gap: Val::Px(8.0),
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
            for line in &view.log {
                controls.spawn(fine(assets, format!("· {line}")));
            }
        });
}

fn apply_layout(
    metrics: Res<ResolvedUiMetrics>,
    added: Query<(), Added<Body>>,
    mut bodies: Query<&mut Node, (With<Body>, Without<ControlsHeading>)>,
    mut headings: Query<&mut Node, (With<ControlsHeading>, Without<Body>)>,
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
    for mut node in &mut headings {
        // "The Lattice" already identifies this compact screen. Keeping the
        // redundant subsection title ahead of four primary actions consumes a
        // complete text row at 200%, pushing the final cast out of the initial
        // viewport. Standard and Wide retain the desktop section hierarchy.
        node.display = if metrics.viewport == UiViewportClass::Compact {
            Display::None
        } else {
            Display::Flex
        };
    }
}

fn emit_intents(
    clicked: Query<(&Interaction, &Control), Changed<Interaction>>,
    back: Query<&Interaction, (Changed<Interaction>, With<BackControl>)>,
    mut intents: MessageWriter<UiIntent>,
) {
    for (interaction, control) in &clicked {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::LatticeDemo(control.0));
        }
    }
    if back
        .iter()
        .any(|interaction| *interaction == Interaction::Pressed)
    {
        intents.write(UiIntent::Back);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource, Default)]
    struct ReceivedBack(usize);

    fn receive(mut intents: MessageReader<UiIntent>, mut received: ResMut<ReceivedBack>) {
        for intent in intents.read() {
            if matches!(intent, UiIntent::Back) {
                received.0 += 1;
            }
        }
    }

    #[test]
    fn one_back_press_emits_one_shared_navigation_intent() {
        let mut app = App::new();
        app.add_message::<UiIntent>()
            .init_resource::<ReceivedBack>()
            .add_systems(Update, (emit_intents, receive).chain());
        let back = app.world_mut().spawn((Interaction::None, BackControl)).id();

        app.update();
        assert_eq!(app.world().resource::<ReceivedBack>().0, 0);
        *app.world_mut()
            .get_mut::<Interaction>(back)
            .expect("the test Back control must retain Interaction") = Interaction::Pressed;
        app.update();
        app.update();

        assert_eq!(app.world().resource::<ReceivedBack>().0, 1);
    }

    #[cfg(feature = "test-support")]
    fn populated_demo_snapshot(
        physical_width: u32,
        physical_height: u32,
        device_scale: f32,
        mode: crate::UiScaleMode,
    ) -> crate::test_support::UiTreeSnapshot {
        let mut app = App::new();
        app.add_plugins(crate::test_support::HeadlessUiPlugin::with_scale_factor(
            physical_width,
            physical_height,
            device_scale,
        ));
        app.world_mut()
            .insert_resource(crate::UiScalePreference(mode));
        app.world_mut().insert_resource(crate::LatticeDemoView {
            ready: true,
            cells: [
                (0, 0, "AIR"),
                (1, 0, "FIRE"),
                (0, 1, "WATER"),
                (-1, 1, "EARTH"),
            ]
            .into_iter()
            .map(|(q, r, label)| crate::LatticeCellView {
                coord: hex_core::LatticeCoord::new(q, r),
                label: label.to_owned(),
                detail: "LIVE · 1 MANA".to_owned(),
                color: Color::srgb(0.35, 0.62, 0.78),
                known_mana: Some(1),
                known_locked: Some(false),
                disabled: false,
                selected: false,
                interaction: crate::CellInteraction::Actionable,
            })
            .collect(),
            spells: vec![
                crate::LatticeDemoSpellView {
                    coord: hex_core::LatticeCoord::new(1, 0),
                    name: "Ember".to_owned(),
                    headline: "Ember · ready".to_owned(),
                    kind: "Evocation".to_owned(),
                    cost: Some(1),
                    blocked: None,
                },
                crate::LatticeDemoSpellView {
                    coord: hex_core::LatticeCoord::new(0, 1),
                    name: "Lightning Bolt".to_owned(),
                    headline: "Lightning Bolt · ready".to_owned(),
                    kind: "Evocation".to_owned(),
                    cost: Some(2),
                    blocked: None,
                },
            ],
            totals: "Mana 4 · disabled 0 · enchantments 0".to_owned(),
            log: (1..=8)
                .map(|index| format!("Bounded lattice event {index}"))
                .collect(),
        });
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::LatticeDemo);
        for _ in 0..8 {
            app.update();
        }
        crate::test_support::ui_tree_snapshot(app.world_mut())
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn compact_high_scale_keeps_every_primary_action_visible() {
        for (physical_width, physical_height, device_scale) in [(960, 540, 1.0), (1920, 1080, 2.0)]
        {
            let snapshot = populated_demo_snapshot(
                physical_width,
                physical_height,
                device_scale,
                crate::UiScaleMode::Percent200,
            );
            for name in [
                "Back",
                "End Turn",
                "Reset",
                "Cast Ember",
                "Cast Lightning Bolt",
            ] {
                let node = snapshot
                    .nodes
                    .iter()
                    .find(|node| node.name == name)
                    .unwrap_or_else(|| panic!("missing primary Lattice Demo action {name:?}"));
                assert_eq!(
                    node.visibility_requirement,
                    Some(crate::UiVisibilityRequirement::Immediate),
                    "{name} must remain an Immediate action"
                );
                assert!(
                    node.fully_visible,
                    "{name} must be visible at {physical_width}×{physical_height} / {device_scale}×: {node:?}"
                );
            }
        }
    }
}
