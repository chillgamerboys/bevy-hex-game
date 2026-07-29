//! Stable combat order with names, factions, and the current actor.

use bevy::picking::Pickable;
use bevy::prelude::*;
use hex_combat::{CombatSystems, TurnOrder};
use hex_core::{AppSystems, Screen, UnitId};
use hex_units::{Faction, UnitRegistry};

use crate::menus::widgets::{heading, panel, UiAssets, ACCENT, LABEL};
use crate::readouts::HudElement;
use crate::screens::DespawnOnExit;

#[derive(Resource, Default, Debug, PartialEq, Eq)]
struct InitiativeReadout(Vec<InitiativeEntry>);

#[derive(Debug, PartialEq, Eq)]
struct InitiativeEntry {
    unit: UnitId,
    name: String,
    faction: Faction,
    current: bool,
}

#[derive(Component)]
struct InitiativePanel;

#[derive(Component)]
struct InitiativeBody;

const FRAME: Pickable = Pickable {
    should_block_lower: true,
    is_hoverable: false,
};

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<InitiativeReadout>()
        .add_systems(OnEnter(Screen::Gameplay), spawn_panel)
        .add_systems(
            Update,
            refresh
                .in_set(AppSystems::Update)
                .after(CombatSystems::Advance)
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(
            Update,
            rebuild.after(refresh).run_if(in_state(Screen::Gameplay)),
        );
}

fn spawn_panel(
    mut commands: Commands,
    mut readout: ResMut<InitiativeReadout>,
    assets: Res<UiAssets>,
) {
    *readout = InitiativeReadout::default();
    commands
        .spawn((
            Name::new("Initiative Panel"),
            InitiativePanel,
            HudElement,
            panel(),
            FRAME,
            DespawnOnExit(Screen::Gameplay),
        ))
        .insert(Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            top: Val::Px(12.0),
            left: Val::Percent(50.0),
            width: Val::Px(286.0),
            margin: UiRect::left(Val::Px(-143.0)),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(Val::Px(12.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(10.0)),
            row_gap: Val::Px(5.0),
            ..default()
        })
        .with_children(|panel| {
            panel.spawn(heading(&assets, "initiative"));
            panel.spawn((
                Name::new("Initiative Body"),
                InitiativeBody,
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(3.0),
                    ..default()
                },
                Pickable::IGNORE,
            ));
        });
}

fn refresh(
    mut readout: ResMut<InitiativeReadout>,
    order: Res<TurnOrder>,
    registry: Res<UnitRegistry>,
    identities: Query<(&Name, &Faction)>,
) {
    let current = order.current();
    let entries = order
        .order()
        .iter()
        .filter_map(|unit| {
            let entity = registry.entity_of(*unit)?;
            let (name, faction) = identities.get(entity).ok()?;
            Some(InitiativeEntry {
                unit: *unit,
                name: name.as_str().to_owned(),
                faction: *faction,
                current: current == Some(*unit),
            })
        })
        .collect();
    let next = InitiativeReadout(entries);
    if *readout != next {
        *readout = next;
    }
}

fn rebuild(
    mut commands: Commands,
    readout: Res<InitiativeReadout>,
    bodies: Query<Entity, With<InitiativeBody>>,
    mut panels: Query<&mut Node, With<InitiativePanel>>,
    assets: Res<UiAssets>,
) {
    if !readout.is_changed() {
        return;
    }
    if let Ok(mut node) = panels.single_mut() {
        node.display = if readout.0.is_empty() {
            Display::None
        } else {
            Display::Flex
        };
    }
    let Ok(body) = bodies.single() else { return };
    commands.entity(body).despawn_related::<Children>();
    commands.entity(body).with_children(|rows| {
        for entry in &readout.0 {
            let side = match entry.faction {
                Faction::Player => "player",
                Faction::Hostile => "hostile",
            };
            let marker = if entry.current { "▶" } else { "·" };
            rows.spawn((
                Name::new(format!("Initiative Unit {}", entry.unit.0)),
                Text::new(format!("{marker} {} · {side}", entry.name)),
                TextFont {
                    font: assets.body.clone().into(),
                    ..TextFont::from_font_size(12.0)
                },
                TextColor(if entry.current { ACCENT } else { LABEL }),
                Pickable::IGNORE,
            ));
        }
    });
}

#[cfg(test)]
mod tests {
    use bevy::MinimalPlugins;

    use super::*;

    #[test]
    fn initiative_renders_in_stable_order_and_marks_the_current_unit() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(UiAssets {
            display: Handle::default(),
            body: Handle::default(),
            hex_cell: Handle::default(),
        });
        app.init_resource::<InitiativeReadout>()
            .add_systems(Startup, spawn_panel)
            .add_systems(Update, rebuild);
        app.update();
        app.insert_resource(InitiativeReadout(vec![
            InitiativeEntry {
                unit: UnitId(4),
                name: "mage #4".to_owned(),
                faction: Faction::Player,
                current: true,
            },
            InitiativeEntry {
                unit: UnitId(9),
                name: "wolf #9".to_owned(),
                faction: Faction::Hostile,
                current: false,
            },
        ]));
        app.update();

        let mut rows = app
            .world_mut()
            .query_filtered::<(&Name, &Text, &TextColor), Without<InitiativeBody>>();
        let rendered: Vec<_> = rows
            .iter(app.world())
            .filter(|(name, _, _)| name.as_str().starts_with("Initiative Unit"))
            .map(|(_, text, color)| (text.0.clone(), color.0))
            .collect();
        assert_eq!(
            rendered,
            vec![
                ("▶ mage #4 · player".to_owned(), ACCENT),
                ("· wolf #9 · hostile".to_owned(), LABEL),
            ]
        );
    }
}
