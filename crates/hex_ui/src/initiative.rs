//! Initiative rendering from an immutable, disclosure-safe view.

use bevy::input_focus::tab_navigation::TabIndex;
use bevy::prelude::*;
use hex_core::Screen;

use crate::{
    hud_heading, hud_text_role, panel, HudElement, InitiativeIntent, InitiativeSide,
    InitiativeView, UiAssets, UiHudSetup, UiIntent, UiRegionRole, UiSystems, ACCENT, BLURB_SIZE,
    EDGE, LABEL, READ_ONLY_HUD,
};

#[derive(Component)]
struct InitiativePanel;

#[derive(Component)]
struct InitiativeBody;

#[derive(Component)]
struct InitiativeHeading;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct InitiativeControl(hex_core::UnitId);

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
            panel.spawn((InitiativeHeading, hud_heading(&assets, "turn order")));
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
    if let Some(turn_region) = regions
        .iter()
        .find_map(|(entity, role)| (*role == UiRegionRole::Turn).then_some(entity))
    {
        commands.entity(turn_region).add_child(panel);
    }
}

fn rebuild(
    mut commands: Commands,
    view: Res<InitiativeView>,
    review: Option<Res<crate::review::UiReviewPresentation>>,
    bodies: Query<Entity, With<InitiativeBody>>,
    mut panels: Query<&mut Node, With<InitiativePanel>>,
    mut headings: Query<&mut Text, With<InitiativeHeading>>,
    assets: Res<UiAssets>,
) {
    let review_changed = review.as_ref().is_some_and(|review| review.is_changed());
    if !view.is_changed() && !review_changed {
        return;
    }
    let view = review
        .as_ref()
        .and_then(|review| review.initiative.as_ref())
        .unwrap_or(view.as_ref());
    if let Ok(mut heading) = headings.single_mut() {
        heading.0.clone_from(&view.heading);
    }
    if let Ok(mut node) = panels.single_mut() {
        node.display = if view.entries.is_empty() {
            Display::None
        } else {
            Display::Flex
        };
    }
    let Ok(body) = bodies.single() else { return };
    commands.entity(body).despawn_related::<Children>();
    commands.entity(body).with_children(|rows| {
        let dense = view.entries.len() > 8;
        for entry in &view.entries {
            let name = format!("Initiative Unit {}", entry.unit.0);
            let copy = (
                Text::new(entry_label(entry, dense)),
                hud_text_role(),
                TextFont {
                    font: assets.body.clone().into(),
                    ..TextFont::from_font_size(BLURB_SIZE)
                },
                TextColor(if entry.current { ACCENT } else { LABEL }),
                Pickable::IGNORE,
            );
            if entry.inspectable {
                rows.spawn((
                    Name::new(name),
                    AccessibleLabel::new(format!("Inspect {}", entry.name)),
                    Button,
                    TabIndex(0),
                    crate::UiVisibilityRequirement::Immediate,
                    InitiativeControl(entry.unit),
                    Node {
                        min_width: Val::Px(44.0),
                        min_height: Val::Px(44.0),
                        flex_shrink: 0.0,
                        padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(5.0)),
                        ..default()
                    },
                    BorderColor::all(if entry.current { ACCENT } else { EDGE }),
                    BackgroundColor(Color::srgba(0.02, 0.03, 0.045, 0.82)),
                ))
                .with_child(copy);
            } else {
                rows.spawn((
                    Name::new(format!("{name} Unavailable")),
                    AccessibleLabel::new(format!("{} is not currently observable", entry.name)),
                    Node {
                        min_height: Val::Px(44.0),
                        flex_shrink: 0.0,
                        padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)),
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    Pickable::IGNORE,
                ))
                .with_child(copy);
            }
        }
    });
}

fn emit_intents(
    controls: Query<(&Interaction, &InitiativeControl), Changed<Interaction>>,
    mut intents: MessageWriter<UiIntent>,
) {
    for (interaction, control) in &controls {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::Initiative(InitiativeIntent::ActivateUnit(
                control.0,
            )));
        }
    }
}

fn entry_label(entry: &crate::InitiativeEntryView, dense: bool) -> String {
    let side = match (entry.side, dense) {
        (InitiativeSide::Ally, true) => "P",
        (InitiativeSide::Hostile, true) => "H",
        (InitiativeSide::Ally, false) => "ALLY",
        (InitiativeSide::Hostile, false) => "HOSTILE",
    };
    let marker = if entry.current { "▶" } else { "·" };
    format!("{marker} {side} · {}", entry.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::UnitId;

    #[test]
    fn dense_labels_stay_non_color_and_keep_readable_type() {
        let view = InitiativeView {
            heading: "your turn".to_owned(),
            entries: (0..12)
                .map(|id| crate::InitiativeEntryView {
                    unit: UnitId(id),
                    name: format!("raider #{id}"),
                    side: if id < 6 {
                        InitiativeSide::Ally
                    } else {
                        InitiativeSide::Hostile
                    },
                    current: id == 0,
                    inspectable: id < 6,
                })
                .collect(),
        };
        assert_eq!(view.entries.len(), 12);
        assert_eq!(
            view.entries.first().map(|entry| entry_label(entry, true)),
            Some("▶ P · raider #0".to_owned())
        );
        assert_eq!(
            view.entries.last().map(|entry| entry_label(entry, true)),
            Some("· H · raider #11".to_owned())
        );
    }
}
