//! Development-only cyclic time controls.

use bevy::prelude::*;
use hex_core::Screen;

use crate::{
    blurb, heading, panel, row_button, DevTimeIntent, DevTimeView, HudElement, UiAssets,
    UiHudSetup, UiIntent, UiRegionRole, UiSystems,
};

const CONTROLS: [(&str, &str, DevTimeIntent); 6] = [
    (
        "Dev Time Previous Half Hour",
        "−30m",
        DevTimeIntent::PreviousHalfHour,
    ),
    (
        "Dev Time Next Half Hour",
        "+30m",
        DevTimeIntent::NextHalfHour,
    ),
    ("Dev Time Midnight", "Midnight", DevTimeIntent::Midnight),
    ("Dev Time Dawn", "Dawn", DevTimeIntent::Dawn),
    ("Dev Time Noon", "Noon", DevTimeIntent::Noon),
    ("Dev Time Dusk", "Dusk", DevTimeIntent::Dusk),
];

#[derive(Component)]
struct DevTimePanel;

#[derive(Component)]
struct DevTimeStatus;

#[derive(Component, Default)]
struct DevTimeControls {
    available: bool,
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct DevTimeControl(DevTimeIntent);

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        OnEnter(Screen::Gameplay),
        spawn_panel.in_set(UiHudSetup::Panels),
    )
    .add_systems(
        Update,
        (
            rebuild.in_set(UiSystems::Render),
            emit_intents.in_set(UiSystems::EmitIntents),
        )
            .run_if(in_state(Screen::Gameplay)),
    );
}

fn spawn_panel(
    mut commands: Commands,
    assets: Res<UiAssets>,
    regions: Query<(Entity, &UiRegionRole)>,
) {
    let Some(inspector) = regions
        .iter()
        .find_map(|(entity, role)| (*role == UiRegionRole::Inspector).then_some(entity))
    else {
        return;
    };
    let panel = commands
        .spawn((
            Name::new("Dev Time Panel"),
            DevTimePanel,
            HudElement,
            panel(),
            Pickable::IGNORE,
        ))
        .insert(Node {
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(8.0),
            padding: UiRect::all(Val::Px(12.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(10.0)),
            ..default()
        })
        .with_children(|panel| {
            panel.spawn(heading(&assets, "DEV · TIME"));
            panel.spawn((DevTimeStatus, blurb(&assets, "Checking cyclic time…")));
            panel.spawn((
                Name::new("Dev Time Controls"),
                DevTimeControls::default(),
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    column_gap: Val::Px(6.0),
                    row_gap: Val::Px(6.0),
                    ..default()
                },
                Pickable::IGNORE,
            ));
        })
        .id();
    commands.entity(inspector).add_child(panel);
}

fn rebuild(
    mut commands: Commands,
    view: Res<DevTimeView>,
    mut statuses: Query<(&mut Text, Ref<DevTimeStatus>)>,
    mut controls: Query<(Entity, &mut DevTimeControls)>,
    assets: Res<UiAssets>,
) {
    let Ok((mut status, status_marker)) = statuses.single_mut() else {
        return;
    };
    let Ok((controls_entity, mut controls)) = controls.single_mut() else {
        return;
    };
    let controls_added = controls.is_added();
    if !view.is_changed() && !status_marker.is_added() && !controls_added {
        return;
    }

    let available = matches!(view.as_ref(), DevTimeView::Available { .. });
    if controls_added || controls.available != available {
        commands
            .entity(controls_entity)
            .despawn_related::<Children>();
        if available {
            commands.entity(controls_entity).with_children(|controls| {
                for (name, label, intent) in CONTROLS {
                    controls
                        .spawn((row_button(name, 96.0), DevTimeControl(intent)))
                        .with_child(blurb(&assets, label));
                }
            });
        }
        controls.available = available;
    }

    match view.as_ref() {
        DevTimeView::Available { hours } => {
            **status = format!("CURRENT · {hours:.1} h");
        }
        DevTimeView::Unavailable { reason } => {
            **status = format!("UNAVAILABLE · {reason}");
        }
    }
}

fn emit_intents(
    view: Res<DevTimeView>,
    controls: Query<(&Interaction, &DevTimeControl), Changed<Interaction>>,
    mut intents: MessageWriter<UiIntent>,
) {
    if !matches!(view.as_ref(), DevTimeView::Available { .. }) {
        return;
    }
    for (interaction, control) in &controls {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::DevTime(control.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::input_focus::tab_navigation::{TabGroup, TabIndex};
    use bevy::input_focus::InputFocus;

    use super::*;

    #[derive(Resource, Default)]
    struct Received(Vec<DevTimeIntent>);

    fn receive(mut intents: MessageReader<UiIntent>, mut received: ResMut<Received>) {
        for intent in intents.read() {
            if let UiIntent::DevTime(intent) = intent {
                received.0.push(*intent);
            }
        }
    }

    #[test]
    fn controls_cover_the_exact_six_intents_with_stable_names() {
        assert_eq!(
            CONTROLS,
            [
                (
                    "Dev Time Previous Half Hour",
                    "−30m",
                    DevTimeIntent::PreviousHalfHour,
                ),
                (
                    "Dev Time Next Half Hour",
                    "+30m",
                    DevTimeIntent::NextHalfHour,
                ),
                ("Dev Time Midnight", "Midnight", DevTimeIntent::Midnight),
                ("Dev Time Dawn", "Dawn", DevTimeIntent::Dawn),
                ("Dev Time Noon", "Noon", DevTimeIntent::Noon),
                ("Dev Time Dusk", "Dusk", DevTimeIntent::Dusk),
            ]
        );
    }

    #[test]
    fn one_press_emits_one_typed_intent() {
        let mut app = App::new();
        app.add_message::<UiIntent>()
            .init_resource::<Received>()
            .insert_resource(DevTimeView::Available { hours: 12.0 })
            .add_systems(Update, (emit_intents, receive).chain());
        let button = app
            .world_mut()
            .spawn((
                Interaction::None,
                DevTimeControl(DevTimeIntent::NextHalfHour),
            ))
            .id();

        app.update();
        assert!(app.world().resource::<Received>().0.is_empty());
        *app.world_mut().get_mut::<Interaction>(button).unwrap() = Interaction::Pressed;
        app.update();
        app.update();

        assert_eq!(
            app.world().resource::<Received>().0,
            [DevTimeIntent::NextHalfHour]
        );
    }

    #[test]
    fn unavailable_removes_controls_and_blocks_a_stale_press() {
        let mut app = App::new();
        app.add_message::<UiIntent>()
            .init_resource::<Received>()
            .insert_resource(DevTimeView::Available { hours: 12.0 })
            .insert_resource(UiAssets {
                display: Handle::default(),
                body: Handle::default(),
                hex_cell: Handle::default(),
            })
            .add_systems(Update, (rebuild, emit_intents, receive).chain());
        app.world_mut()
            .spawn((DevTimeStatus, Text::new("Waiting…")));
        app.world_mut()
            .spawn((DevTimeControls::default(), Node::default()));

        app.update();
        let controls = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<Entity, With<DevTimeControl>>();
            query.iter(world).collect::<Vec<_>>()
        };
        assert_eq!(controls.len(), 6);
        let stale = controls
            .first()
            .copied()
            .expect("available time must create controls");
        *app.world_mut().get_mut::<Interaction>(stale).unwrap() = Interaction::Pressed;
        *app.world_mut().resource_mut::<DevTimeView>() = DevTimeView::Unavailable {
            reason: "Static lighting profile".to_owned(),
        };

        app.update();

        let control_count = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<Entity, With<DevTimeControl>>();
            query.iter(world).count()
        };
        assert_eq!(control_count, 0);
        assert!(app.world().resource::<Received>().0.is_empty());
        let view = DevTimeView::default();
        assert!(matches!(
            view,
            DevTimeView::Unavailable { ref reason } if !reason.is_empty()
        ));
    }

    #[test]
    fn controls_are_focusable_accessible_inspector_descendants() {
        let mut app = App::new();
        app.insert_resource(DevTimeView::Available { hours: 12.0 })
            .insert_resource(UiAssets {
                display: Handle::default(),
                body: Handle::default(),
                hex_cell: Handle::default(),
            })
            .add_systems(Update, (spawn_panel, rebuild).chain());
        let gameplay_group = app
            .world_mut()
            .spawn((Name::new("Gameplay HUD Safe Frame"), TabGroup::new(0)))
            .id();
        let inspector = app
            .world_mut()
            .spawn((Name::new("Inspector HUD Region"), UiRegionRole::Inspector))
            .id();
        app.world_mut()
            .entity_mut(gameplay_group)
            .add_child(inspector);

        app.update();

        let controls = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<
                (Entity, &Name, &TabIndex, &AccessibleLabel),
                With<DevTimeControl>,
            >();
            query
                .iter(world)
                .map(|(entity, name, index, label)| {
                    (entity, name.as_str().to_owned(), index.0, label.0.clone())
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(controls.len(), 6);
        for (entity, name, index, label) in controls {
            assert!(name.starts_with("Dev Time "));
            assert_eq!(index, 0);
            assert!(!label.is_empty());
            assert!(has_ancestor(app.world(), entity, inspector));
            assert!(has_ancestor(app.world(), entity, gameplay_group));
        }
    }

    #[test]
    fn unchanged_available_view_rebuilds_recreated_gameplay_panel() {
        let mut app = App::new();
        app.insert_resource(DevTimeView::Available { hours: 6.0 })
            .insert_resource(UiAssets {
                display: Handle::default(),
                body: Handle::default(),
                hex_cell: Handle::default(),
            })
            .add_systems(Update, rebuild);
        let first_status = app
            .world_mut()
            .spawn((DevTimeStatus, Text::new("Checking cyclic time…")))
            .id();
        let first_controls = app
            .world_mut()
            .spawn((DevTimeControls::default(), Node::default()))
            .id();

        app.update();
        assert_eq!(control_entities(app.world_mut()).len(), 6);

        for entity in control_entities(app.world_mut()) {
            assert!(app.world_mut().despawn(entity));
        }
        assert!(app.world_mut().despawn(first_controls));
        assert!(app.world_mut().despawn(first_status));
        let second_status = app
            .world_mut()
            .spawn((DevTimeStatus, Text::new("Checking cyclic time…")))
            .id();
        app.world_mut()
            .spawn((DevTimeControls::default(), Node::default()));

        app.update();

        assert_eq!(control_entities(app.world_mut()).len(), 6);
        assert_eq!(
            app.world()
                .get::<Text>(second_status)
                .map(|text| text.as_str()),
            Some("CURRENT · 6.0 h")
        );
    }

    #[test]
    fn available_hour_updates_preserve_control_entities_and_focus() {
        let mut app = App::new();
        app.insert_resource(DevTimeView::Available { hours: 12.0 })
            .insert_resource(UiAssets {
                display: Handle::default(),
                body: Handle::default(),
                hex_cell: Handle::default(),
            })
            .add_systems(Update, rebuild);
        app.world_mut()
            .spawn((DevTimeStatus, Text::new("Checking cyclic time…")));
        app.world_mut()
            .spawn((DevTimeControls::default(), Node::default()));
        app.update();
        let mut before = control_entities(app.world_mut());
        before.sort_by_key(|entity| entity.to_bits());
        let focused = before
            .first()
            .copied()
            .expect("available time must create controls");
        app.insert_resource(InputFocus::from_entity(focused));

        *app.world_mut().resource_mut::<DevTimeView>() = DevTimeView::Available { hours: 12.5 };
        app.update();

        let mut after = control_entities(app.world_mut());
        after.sort_by_key(|entity| entity.to_bits());
        assert_eq!(after, before);
        assert!(app.world().get_entity(focused).is_ok());
        assert_eq!(
            app.world().resource::<InputFocus>().get(),
            Some(focused),
            "updating the time must preserve keyboard focus"
        );
    }

    fn control_entities(world: &mut World) -> Vec<Entity> {
        let mut query = world.query_filtered::<Entity, With<DevTimeControl>>();
        query.iter(world).collect()
    }

    fn has_ancestor(world: &World, mut entity: Entity, wanted: Entity) -> bool {
        while let Some(parent) = world.get::<ChildOf>(entity) {
            entity = parent.parent();
            if entity == wanted {
                return true;
            }
        }
        false
    }
}
