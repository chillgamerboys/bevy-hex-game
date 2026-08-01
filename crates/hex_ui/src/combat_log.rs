//! Combat history rendering from disclosure-frozen presentation lines.

use bevy::prelude::*;
use hex_core::Screen;

use crate::{
    blurb, panel, supporting_text_role, CombatLogView, HudElement, UiAssets, UiHudSetup,
    UiRegionRole, BLURB_SIZE, DANGER, LABEL, READ_ONLY_HUD,
};

#[derive(Component)]
struct CombatLogPanel;

#[derive(Component)]
struct CombatLogBody;

#[derive(Component)]
struct CombatLogHeading;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        OnEnter(Screen::Gameplay),
        spawn_panel.in_set(UiHudSetup::Panels),
    )
    .add_systems(Update, rebuild.run_if(in_state(Screen::Gameplay)));
}

fn spawn_panel(
    mut commands: Commands,
    assets: Res<UiAssets>,
    regions: Query<(Entity, &UiRegionRole)>,
) {
    let panel = commands
        .spawn((
            Name::new("Combat Log Panel"),
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
            panel.spawn((
                CombatLogHeading,
                blurb(&assets, "RECENT EVENTS · L HISTORY"),
            ));
            panel.spawn((
                Name::new("Combat Log Body"),
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
    view: Res<CombatLogView>,
    bodies: Query<Entity, With<CombatLogBody>>,
    mut headings: Query<&mut Text, With<CombatLogHeading>>,
    assets: Res<UiAssets>,
) {
    if !view.is_changed() {
        return;
    }
    if let Ok(mut heading) = headings.single_mut() {
        heading.0.clone_from(&view.heading);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn danger_is_a_secondary_cue_beside_the_frozen_text() {
        let view = CombatLogView {
            heading: "COMBAT HISTORY · 2 EVENTS · L CLOSE".to_owned(),
            lines: vec![
                crate::CombatLogLineView {
                    text: "ally cast Ember".to_owned(),
                    danger: false,
                },
                crate::CombatLogLineView {
                    text: "ally lost fire gem".to_owned(),
                    danger: true,
                },
            ],
        };
        assert_eq!(view.lines.len(), 2);
        assert!(view.lines.last().is_some_and(|line| line.danger));
        assert!(view.lines.last().is_some_and(|line| !line.text.is_empty()));
    }
}
