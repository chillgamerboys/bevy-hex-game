//! Categorized gameplay history from disclosure-frozen presentation lines.

use bevy::input_focus::tab_navigation::TabIndex;
use bevy::prelude::*;
use hex_core::Screen;

use crate::{
    blurb, panel, supporting_text_role, ActivityIntent, ActivityLogView, ActivityTab, HudElement,
    UiAssets, UiHudSetup, UiIntent, UiRegionRole, UiSystems, ACCENT, BLURB_SIZE, DANGER, EDGE,
    LABEL, READ_ONLY_HUD,
};

#[derive(Component)]
struct CombatLogPanel;

#[derive(Component)]
struct CombatLogBody;

#[derive(Component)]
struct CombatLogHeading;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct ActivityTabControl(ActivityTab);

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct ActivityTabLabel(ActivityTab);

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
            Name::new("Activity Log Panel"),
            CombatLogPanel,
            HudElement,
            panel(),
            READ_ONLY_HUD,
        ))
        .insert(Node {
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(Val::Px(8.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(8.0)),
            row_gap: Val::Px(3.0),
            ..default()
        })
        .with_children(|panel| {
            panel.spawn((CombatLogHeading, blurb(&assets, "ACTIVITY")));
            panel
                .spawn((
                    Name::new("Activity Log Tabs"),
                    Node {
                        width: Val::Percent(100.0),
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(4.0),
                        ..default()
                    },
                    Pickable::IGNORE,
                ))
                .with_children(|tabs| {
                    for (tab, label) in [
                        (ActivityTab::All, "All"),
                        (ActivityTab::Combat, "Combat"),
                        (ActivityTab::Activity, "Activity"),
                    ] {
                        tabs.spawn((
                            Name::new(format!("Activity Tab {label}")),
                            AccessibleLabel::new(format!("Show {label} events")),
                            Button,
                            TabIndex(0),
                            crate::UiVisibilityRequirement::Immediate,
                            ActivityTabControl(tab),
                            Node {
                                min_width: Val::Px(44.0),
                                min_height: Val::Px(44.0),
                                padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)),
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::Center,
                                border: UiRect::all(Val::Px(1.0)),
                                border_radius: BorderRadius::all(Val::Px(5.0)),
                                ..default()
                            },
                            BorderColor::all(EDGE),
                            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.04)),
                        ))
                        .with_child((ActivityTabLabel(tab), blurb(&assets, label)));
                    }
                });
            panel.spawn((
                Name::new("Activity Log Body"),
                CombatLogBody,
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(3.0),
                    ..default()
                },
                Pickable::IGNORE,
            ));
        })
        .id();
    if let Some(events) = regions
        .iter()
        .find_map(|(entity, role)| (*role == UiRegionRole::Events).then_some(entity))
    {
        commands.entity(events).add_child(panel);
    }
}

fn rebuild(
    mut commands: Commands,
    view: Res<ActivityLogView>,
    review: Option<Res<crate::review::UiReviewPresentation>>,
    bodies: Query<Entity, With<CombatLogBody>>,
    mut headings: Query<&mut Text, (With<CombatLogHeading>, Without<ActivityTabLabel>)>,
    mut tabs: Query<(Entity, &ActivityTabControl, &mut BorderColor)>,
    mut tab_labels: Query<
        (&ActivityTabLabel, &mut Text),
        (Without<CombatLogHeading>, Without<ActivityTabControl>),
    >,
    assets: Res<UiAssets>,
) {
    let review_changed = review.as_ref().is_some_and(|review| review.is_changed());
    if !view.is_changed() && !review_changed {
        return;
    }
    let view = review
        .as_ref()
        .and_then(|review| review.activity.as_ref())
        .unwrap_or(view.as_ref());
    if let Ok(mut heading) = headings.single_mut() {
        heading.0.clone_from(&view.heading);
    }
    for (entity, tab, mut border) in &mut tabs {
        let selected = tab.0 == view.tab;
        *border = BorderColor::all(if selected { ACCENT } else { EDGE });
        let accessible = if selected {
            format!("{} events, selected", activity_tab_name(tab.0))
        } else {
            format!("Show {} events", activity_tab_name(tab.0))
        };
        commands
            .entity(entity)
            .insert(AccessibleLabel::new(accessible));
    }
    for (tab, mut label) in &mut tab_labels {
        let name = activity_tab_name(tab.0);
        label.0 = if tab.0 == view.tab {
            format!("✓ {name}")
        } else {
            name.to_owned()
        };
    }
    let Ok(body) = bodies.single() else { return };
    commands.entity(body).despawn_related::<Children>();
    commands.entity(body).with_children(|rows| {
        for line in &view.lines {
            rows.spawn((
                Text::new(line.text.clone()),
                supporting_text_role(),
                TextFont {
                    font: assets.body.clone().into(),
                    ..TextFont::from_font_size(BLURB_SIZE)
                },
                TextColor(if line.danger { DANGER } else { LABEL }),
                Pickable::IGNORE,
            ));
        }
    });
}

const fn activity_tab_name(tab: ActivityTab) -> &'static str {
    match tab {
        ActivityTab::All => "All",
        ActivityTab::Combat => "Combat",
        ActivityTab::Activity => "Activity",
    }
}

fn emit_intents(
    controls: Query<(&Interaction, &ActivityTabControl), Changed<Interaction>>,
    mut intents: MessageWriter<UiIntent>,
) {
    for (interaction, control) in &controls {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::Activity(ActivityIntent::SelectTab(control.0)));
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::MinimalPlugins;

    use super::*;

    fn rendered_tab_state(world: &mut World, wanted: ActivityTab) -> (String, String) {
        let accessible = {
            let mut query = world.query::<(&ActivityTabControl, &AccessibleLabel)>();
            query
                .iter(world)
                .find(|(tab, _)| tab.0 == wanted)
                .map(|(_, label)| label.0.clone())
                .expect("activity tab control must exist")
        };
        let visible = {
            let mut query = world.query::<(&ActivityTabLabel, &Text)>();
            query
                .iter(world)
                .find(|(tab, _)| tab.0 == wanted)
                .map(|(_, label)| label.0.clone())
                .expect("activity tab label must exist")
        };
        (visible, accessible)
    }

    #[test]
    fn danger_is_a_secondary_cue_beside_the_frozen_text() {
        let view = ActivityLogView {
            heading: "ACTIVITY".to_owned(),
            tab: ActivityTab::All,
            lines: vec![
                crate::ActivityLogLineView {
                    kind: crate::ActivityKind::Combat,
                    text: "ally cast Ember".to_owned(),
                    danger: false,
                },
                crate::ActivityLogLineView {
                    kind: crate::ActivityKind::Combat,
                    text: "ally lost fire gem".to_owned(),
                    danger: true,
                },
            ],
        };
        assert_eq!(view.lines.len(), 2);
        assert!(view.lines.last().is_some_and(|line| line.danger));
        assert!(view.lines.last().is_some_and(|line| !line.text.is_empty()));
    }

    #[test]
    fn selected_tab_is_visible_and_announced_without_relying_on_color() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(ActivityLogView {
                heading: "ACTIVITY".to_owned(),
                tab: ActivityTab::Combat,
                lines: Vec::new(),
            })
            .insert_resource(UiAssets {
                display: Handle::default(),
                body: Handle::default(),
                hex_cell: Handle::default(),
            })
            .add_systems(Startup, spawn_panel)
            .add_systems(Update, rebuild);
        app.world_mut().spawn(UiRegionRole::Events);

        app.update();

        assert_eq!(
            rendered_tab_state(app.world_mut(), ActivityTab::Combat),
            ("✓ Combat".to_owned(), "Combat events, selected".to_owned())
        );
        assert_eq!(
            rendered_tab_state(app.world_mut(), ActivityTab::All),
            ("All".to_owned(), "Show All events".to_owned())
        );

        app.world_mut().resource_mut::<ActivityLogView>().tab = ActivityTab::Activity;
        app.update();

        assert_eq!(
            rendered_tab_state(app.world_mut(), ActivityTab::Activity),
            (
                "✓ Activity".to_owned(),
                "Activity events, selected".to_owned()
            )
        );
        assert_eq!(
            rendered_tab_state(app.world_mut(), ActivityTab::Combat),
            ("Combat".to_owned(), "Show Combat events".to_owned())
        );
    }
}
