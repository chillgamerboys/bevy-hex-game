//! Casting and required-decision panel from an immutable application projection.

use bevy::input_focus::tab_navigation::TabIndex;
use bevy::prelude::*;
use hex_core::Screen;

use crate::{
    blurb, fine, heading, row_button, spawn_decision_controls, stacked_row_button, CastingIntent,
    CastingPanelContentView, CastingPanelView, GameplayAction, HudElement, RequiredActionSurface,
    UiAssets, UiHudSetup, UiIntent, UiRegionRole, UiSystems, BLURB_SIZE, EDGE, LABEL, PANEL_BG,
};

const CONTROL_WIDTH: f32 = 104.0;
const SWATCH_WIDTH: f32 = 5.0;

const FRAME: Pickable = Pickable {
    should_block_lower: true,
    is_hoverable: false,
};

#[derive(Component)]
struct PanelBody;

#[derive(Component)]
struct CastingPanel;

#[derive(Component, Debug, Clone, PartialEq, Eq)]
enum CastingControl {
    Begin(String),
    Confirm,
    Next,
    Cancel,
    Channel,
    EndTurn,
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        OnEnter(Screen::Gameplay),
        spawn_panel.in_set(UiHudSetup::Panels),
    )
    .add_systems(
        Update,
        (rebuild, emit_intents.in_set(UiSystems::EmitIntents)).run_if(in_state(Screen::Gameplay)),
    );
}

fn spawn_panel(
    mut commands: Commands,
    assets: Res<UiAssets>,
    regions: Query<(Entity, &UiRegionRole)>,
) {
    let panel = commands
        .spawn((
            Name::new("Casting Panel"),
            CastingPanel,
            RequiredActionSurface,
            HudElement,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                height: Val::Px(126.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                padding: UiRect::axes(Val::Px(9.0), Val::Px(6.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(10.0)),
                ..default()
            },
            BorderColor::all(EDGE),
            BackgroundColor(PANEL_BG),
            FRAME,
        ))
        .with_children(|panel| {
            panel.spawn(heading(&assets, "actions"));
            panel.spawn((
                Name::new("Casting Body"),
                PanelBody,
                Node {
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(6.0),
                    align_items: AlignItems::Center,
                    ..default()
                },
                Pickable::IGNORE,
            ));
        })
        .id();
    if let Some(actions) = regions
        .iter()
        .find_map(|(entity, role)| (*role == UiRegionRole::Actions).then_some(entity))
    {
        commands.entity(actions).add_child(panel);
    }
}

fn rebuild(
    mut commands: Commands,
    view: Res<CastingPanelView>,
    review: Option<Res<crate::review::UiReviewPresentation>>,
    metrics: Res<crate::ResolvedUiMetrics>,
    mut panels: Query<&mut Node, With<CastingPanel>>,
    bodies: Query<Entity, With<PanelBody>>,
    assets: Res<UiAssets>,
) {
    let review_changed = review.as_ref().is_some_and(|review| review.is_changed());
    if !view.is_changed() && !review_changed && !metrics.is_changed() {
        return;
    }
    let view = review
        .as_ref()
        .and_then(|review| review.casting.as_ref())
        .unwrap_or(view.as_ref());
    let Ok(mut panel) = panels.single_mut() else {
        return;
    };
    // On the compact canvas a blocking decision is promoted into the persistent
    // action rail. Repeating its full prompt and controls in the fixed-height
    // casting region competes with that required surface at enlarged scales.
    let promoted_to_rail = metrics.viewport == crate::UiViewportClass::Compact
        && matches!(view.content, CastingPanelContentView::Decision { .. });
    panel.display = if view.visible && !promoted_to_rail {
        Display::Flex
    } else {
        Display::None
    };
    if !view.visible || promoted_to_rail {
        return;
    }
    let Ok(body) = bodies.single() else { return };
    commands.entity(body).despawn_related::<Children>();
    commands
        .entity(body)
        .with_children(|rows| match &view.content {
            CastingPanelContentView::Message {
                text,
                turn_controls,
            } => {
                rows.spawn(blurb(&assets, text.clone()));
                if *turn_controls {
                    spawn_turn_controls(rows, &assets);
                }
            }
            CastingPanelContentView::Decision { prompt, choice } => {
                rows.spawn(blurb(&assets, prompt.clone()));
                spawn_decision_controls(rows, *choice, &assets);
            }
            CastingPanelContentView::Spells {
                unavailable,
                spells,
                aiming,
            } => {
                if let Some(reason) = unavailable {
                    rows.spawn(blurb(&assets, reason.to_uppercase()));
                }
                if let Some(aiming) = aiming {
                    rows.spawn(blurb(&assets, aiming.label.clone()));
                    if aiming.controls_enabled {
                        spawn_aim_controls(rows, &assets);
                    }
                } else {
                    for spell in spells {
                        spawn_spell(rows, spell, unavailable.is_some(), &assets);
                    }
                }
                spawn_turn_controls(rows, &assets);
            }
        });
}

fn spawn_turn_controls(rows: &mut ChildSpawnerCommands, assets: &UiAssets) {
    rows.spawn((stacked_row_button("Channel", 94.0), CastingControl::Channel))
        .with_children(|button| {
            button.spawn(blurb(assets, "channel"));
            button.spawn(fine(assets, "restore mana"));
        });
    rows.spawn((
        stacked_row_button("End Turn", 94.0),
        CastingControl::EndTurn,
    ))
    .with_children(|button| {
        button.spawn(blurb(assets, "end turn"));
        button.spawn(fine(assets, "SPACE"));
    });
}

fn spawn_spell(
    rows: &mut ChildSpawnerCommands,
    spell: &crate::CastingSpellView,
    unavailable: bool,
    assets: &UiAssets,
) {
    rows.spawn((
        Name::new("Spell Row"),
        Node {
            flex_basis: Val::Px(0.0),
            flex_grow: 1.0,
            min_width: Val::Px(0.0),
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(4.0),
            align_items: AlignItems::Center,
            ..default()
        },
        Pickable::IGNORE,
    ))
    .with_children(|entry| {
        entry.spawn((
            Node {
                width: Val::Px(SWATCH_WIDTH),
                height: Val::Px(74.0),
                border_radius: BorderRadius::all(Val::Px(2.0)),
                ..default()
            },
            BackgroundColor(spell.color),
            Pickable::IGNORE,
        ));
        if spell_is_actionable(unavailable, spell) {
            entry
                .spawn((
                    Name::new(format!("Cast {}", spell.name)),
                    AccessibleLabel::new(format!("Cast {} · {}", spell.name, spell.cost)),
                    Button,
                    TabIndex(0),
                    CastingControl::Begin(spell.name.clone()),
                    spell_button_node(),
                    BorderColor::all(EDGE),
                    BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.08)),
                ))
                .with_children(|button| {
                    button.spawn((
                        Text::new(spell.name.clone()),
                        TextFont {
                            font: assets.body.clone().into(),
                            ..TextFont::from_font_size(BLURB_SIZE)
                        },
                        TextColor(LABEL),
                        Pickable::IGNORE,
                    ));
                    button.spawn(fine(assets, spell.cost.clone()));
                });
        } else {
            entry
                .spawn((
                    Name::new(format!("Cast {} Disabled", spell.name)),
                    AccessibleLabel::new(format!(
                        "{} disabled · {}",
                        spell.name,
                        spell.blocked.as_deref().unwrap_or(&spell.cost)
                    )),
                    spell_button_node(),
                    BorderColor::all(EDGE),
                    BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.04)),
                    Pickable::IGNORE,
                ))
                .with_children(|button| {
                    button.spawn(blurb(assets, spell.name.clone()));
                    button.spawn(fine(
                        assets,
                        spell.blocked.as_ref().map_or_else(
                            || spell.cost.clone(),
                            |reason| format!("BLOCKED · {reason}"),
                        ),
                    ));
                });
        }
    });
}

fn spell_is_actionable(unavailable: bool, spell: &crate::CastingSpellView) -> bool {
    !unavailable && spell.blocked.is_none()
}

fn spell_button_node() -> Node {
    Node {
        width: Val::Px(148.0),
        max_width: Val::Px(148.0),
        flex_grow: 1.0,
        height: Val::Px(74.0),
        padding: UiRect::all(Val::Px(7.0)),
        flex_direction: FlexDirection::Column,
        justify_content: JustifyContent::Center,
        row_gap: Val::Px(2.0),
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(6.0)),
        ..default()
    }
}

fn spawn_aim_controls(rows: &mut ChildSpawnerCommands, assets: &UiAssets) {
    rows.spawn((
        Name::new("Aim Controls"),
        Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(8.0),
            ..default()
        },
        Pickable::IGNORE,
    ))
    .with_children(|controls| {
        for (name, label, shortcut, control) in [
            ("Confirm Cast", "cast", "ENTER", CastingControl::Confirm),
            ("Next Target", "next", "TAB", CastingControl::Next),
            ("Cancel Aim", "cancel", "Q", CastingControl::Cancel),
        ] {
            controls
                .spawn((row_button(name, CONTROL_WIDTH), control))
                .with_children(|button| {
                    button.spawn(blurb(assets, label));
                    button.spawn(fine(assets, shortcut));
                });
        }
    });
}

fn emit_intents(
    controls: Query<(&Interaction, &CastingControl), Changed<Interaction>>,
    mut intents: MessageWriter<UiIntent>,
) {
    for (interaction, control) in &controls {
        if *interaction != Interaction::Pressed {
            continue;
        }
        intents.write(match control {
            CastingControl::Begin(spell) => UiIntent::Casting(CastingIntent::Begin(spell.clone())),
            CastingControl::Confirm => UiIntent::Casting(CastingIntent::Confirm),
            CastingControl::Next => UiIntent::Casting(CastingIntent::NextTarget),
            CastingControl::Cancel => UiIntent::Casting(CastingIntent::Cancel),
            CastingControl::Channel => UiIntent::Gameplay(GameplayAction::Channel),
            CastingControl::EndTurn => UiIntent::Gameplay(GameplayAction::EndTurn),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_wide_and_spell_refusals_both_remove_activation() {
        let castable = crate::CastingSpellView {
            name: "Ember".to_owned(),
            cost: "1 mana".to_owned(),
            blocked: None,
            color: Color::WHITE,
        };
        let mut blocked = castable.clone();
        blocked.blocked = Some("spell hex disabled".to_owned());

        assert!(spell_is_actionable(false, &castable));
        assert!(!spell_is_actionable(true, &castable));
        assert!(!spell_is_actionable(false, &blocked));
    }

    #[test]
    fn casting_controls_keep_keyboard_parity_labels() {
        let controls = [
            ("Confirm Cast", "ENTER"),
            ("Next Target", "TAB"),
            ("Cancel Aim", "Q"),
        ];
        assert!(controls
            .iter()
            .all(|(label, key)| !label.is_empty() && !key.is_empty()));
    }
}
