//! Stable combat order with names, factions, and the current actor.

use bevy::picking::Pickable;
use bevy::prelude::*;
use hex_combat::{CombatSystems, TurnOrder};
use hex_core::{AppSystems, GameplaySystems, Screen, UnitId};
use hex_units::{Faction, UnitRegistry};

use crate::menus::widgets::{heading, panel, UiAssets, ACCENT, LABEL};
use crate::readouts::{region, GameplayUiContext, HudElement, HudRegion, HudSetup, READ_ONLY_HUD};

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

#[derive(Component)]
struct InitiativeHeading;

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<InitiativeReadout>()
        .add_systems(
            OnEnter(Screen::Gameplay),
            spawn_panel.in_set(HudSetup::Panels),
        )
        .add_systems(
            Update,
            refresh
                .in_set(AppSystems::Update)
                .after(CombatSystems::Advance)
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(
            Update,
            rebuild
                .after(refresh)
                .after(GameplaySystems::UiContext)
                .run_if(in_state(Screen::Gameplay)),
        );
}

fn spawn_panel(
    mut commands: Commands,
    mut readout: ResMut<InitiativeReadout>,
    assets: Res<UiAssets>,
    regions: Query<(Entity, &HudRegion)>,
) {
    *readout = InitiativeReadout::default();
    let turn_region = region(HudRegion::Turn, &regions);
    let panel = commands
        .spawn((
            Name::new("Initiative Panel"),
            InitiativePanel,
            HudElement,
            panel(),
            READ_ONLY_HUD,
        ))
        .insert(Node {
            display: Display::None,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(10.0)),
            column_gap: Val::Px(12.0),
            ..default()
        })
        .with_children(|panel| {
            panel.spawn((InitiativeHeading, heading(&assets, "turn order")));
            panel.spawn((
                Name::new("Initiative Body"),
                InitiativeBody,
                Node {
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    column_gap: Val::Px(8.0),
                    row_gap: Val::Px(2.0),
                    align_items: AlignItems::Center,
                    align_content: AlignContent::Center,
                    ..default()
                },
                Pickable::IGNORE,
            ));
        })
        .id();
    if let Some(turn_region) = turn_region {
        commands.entity(turn_region).add_child(panel);
    }
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
    mut headings: Query<&mut Text, With<InitiativeHeading>>,
    context: Option<Res<GameplayUiContext>>,
    assets: Res<UiAssets>,
) {
    if !readout.is_changed() && !context.as_ref().is_some_and(|context| context.is_changed()) {
        return;
    }
    if let Ok(mut heading) = headings.single_mut() {
        heading.0 = context
            .as_deref()
            .and_then(|context| context.acting.as_ref())
            .map_or_else(
                || "turn order".to_owned(),
                |actor| match actor.faction {
                    Faction::Player => "your turn".to_owned(),
                    Faction::Hostile => "enemy turn".to_owned(),
                },
            );
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
        let dense = readout.0.len() > 8;
        for entry in &readout.0 {
            let side = match entry.faction {
                Faction::Player if dense => "P",
                Faction::Hostile if dense => "H",
                Faction::Player => "ALLY",
                Faction::Hostile => "HOSTILE",
            };
            let marker = if entry.current { "▶" } else { "·" };
            rows.spawn((
                Name::new(format!("Initiative Unit {}", entry.unit.0)),
                Text::new(format!("{marker} {side} · {}", entry.name)),
                TextFont {
                    font: assets.body.clone().into(),
                    ..TextFont::from_font_size(if dense { 11.0 } else { 13.0 })
                },
                TextColor(if entry.current { ACCENT } else { LABEL }),
                Node {
                    flex_shrink: 0.0,
                    ..default()
                },
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
                ("▶ ALLY · mage #4".to_owned(), ACCENT),
                ("· HOSTILE · wolf #9".to_owned(), LABEL),
            ]
        );
    }

    #[test]
    fn dense_initiative_uses_compact_nonshrinking_entries() {
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
        app.insert_resource(InitiativeReadout(
            (0..12)
                .map(|id| InitiativeEntry {
                    unit: UnitId(id),
                    name: format!("raider #{id}"),
                    faction: if id < 6 {
                        Faction::Player
                    } else {
                        Faction::Hostile
                    },
                    current: id == 0,
                })
                .collect(),
        ));
        app.update();

        let mut rows = app
            .world_mut()
            .query_filtered::<(&Text, &TextFont, &Node), (With<Name>, Without<InitiativeBody>)>();
        let rendered: Vec<_> = rows
            .iter(app.world())
            .filter(|(text, _, _)| text.0.contains("raider #"))
            .map(|(text, font, node)| (text.0.clone(), font.font_size, node.flex_shrink))
            .collect();
        assert_eq!(rendered.len(), 12);
        assert_eq!(
            rendered.first().map(|entry| entry.0.as_str()),
            Some("▶ P · raider #0")
        );
        assert_eq!(
            rendered.last().map(|entry| entry.0.as_str()),
            Some("· H · raider #11")
        );
        assert!(rendered.iter().all(|(_, font_size, flex_shrink)| {
            *font_size == FontSize::Px(11.0) && *flex_shrink == 0.0
        }));
    }
}
