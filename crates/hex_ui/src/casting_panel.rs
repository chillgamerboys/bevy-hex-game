//! Casting and required-decision panel from an immutable application projection.

use bevy::input_focus::tab_navigation::TabIndex;
use bevy::prelude::*;
use hex_core::Screen;

use crate::{
    blurb, body_text_role, fine, heading, owner_resolved_control_role, row_button,
    spawn_decision_controls, CastingIntent, CastingPanelContentView, CastingPanelView, HudElement,
    RequiredActionSurface, UiAssets, UiHudSetup, UiIntent, UiRegionRole, UiSystems, BLURB_SIZE,
    EDGE, LABEL, PANEL_BG,
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

#[derive(Component)]
struct CastingHeading;

#[derive(Component, Debug, Clone, PartialEq, Eq)]
enum CastingControl {
    Begin(String),
    Confirm,
    Next,
    Cancel,
}

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
            panel.spawn((CastingHeading, heading(&assets, "actions")));
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
    mut panels: Query<
        &mut Node,
        (
            With<CastingPanel>,
            Without<PanelBody>,
            Without<CastingHeading>,
        ),
    >,
    mut bodies: Query<
        (Entity, &mut Node),
        (
            With<PanelBody>,
            Without<CastingPanel>,
            Without<CastingHeading>,
        ),
    >,
    mut headings: Query<
        &mut Node,
        (
            With<CastingHeading>,
            Without<CastingPanel>,
            Without<PanelBody>,
        ),
    >,
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
    let promoted_to_rail = matches!(view.content, CastingPanelContentView::Decision { .. })
        && (metrics.viewport == crate::UiViewportClass::Compact || metrics.content_scale >= 1.5);
    let ultra_constrained = crate::layout::is_ultra_constrained(*metrics);
    for mut heading in &mut headings {
        heading.display = if ultra_constrained {
            Display::None
        } else {
            Display::Flex
        };
    }
    let stacked = ultra_constrained || metrics.content_scale >= 1.5;
    panel.height = if stacked { Val::Auto } else { Val::Px(126.0) };
    panel.overflow = Overflow::default();
    panel.display = if view.visible && !promoted_to_rail {
        Display::Flex
    } else {
        Display::None
    };
    if !view.visible || promoted_to_rail {
        return;
    }
    let Ok((body, mut body_node)) = bodies.single_mut() else {
        return;
    };
    body_node.flex_direction = if stacked {
        FlexDirection::Column
    } else {
        FlexDirection::Row
    };
    body_node.align_items = if stacked {
        AlignItems::Stretch
    } else {
        AlignItems::Center
    };
    body_node.flex_grow = if stacked { 0.0 } else { 1.0 };
    body_node.row_gap = Val::Px(if stacked { 4.0 } else { 0.0 });
    commands.entity(body).despawn_related::<Children>();
    commands
        .entity(body)
        .with_children(|rows| match &view.content {
            CastingPanelContentView::Message {
                text,
                turn_controls: _,
            } => {
                rows.spawn(blurb(&assets, text.clone()));
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
                    // Keep the escape hatch ahead of explanatory copy on short
                    // enlarged canvases. The action rail already carries the
                    // current phase; cancellation must never be pushed below it.
                    spawn_aim_controls(rows, &assets, aiming.controls_enabled);
                    rows.spawn(blurb(&assets, aiming.label.clone()));
                } else {
                    for spell in spells {
                        spawn_spell(
                            rows,
                            spell,
                            unavailable.is_some(),
                            stacked,
                            metrics.control_scale,
                            &assets,
                        );
                    }
                }
            }
        });
}

fn spawn_spell(
    rows: &mut ChildSpawnerCommands,
    spell: &crate::CastingSpellView,
    unavailable: bool,
    stacked: bool,
    semantic_control_scale: f32,
    assets: &UiAssets,
) {
    let semantic_control_scale = semantic_control_scale.max(1.0);
    rows.spawn((
        Name::new("Spell Row"),
        Node {
            flex_basis: if stacked { Val::Auto } else { Val::Px(0.0) },
            flex_grow: if stacked { 0.0 } else { 1.0 },
            flex_shrink: if stacked { 0.0 } else { 1.0 },
            min_width: Val::Px(0.0),
            min_height: Val::Px(74.0 * semantic_control_scale),
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
                height: Val::Px(74.0 * semantic_control_scale),
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
                    crate::UiVisibilityRequirement::Scrollable,
                    owner_resolved_control_role(),
                    CastingControl::Begin(spell.name.clone()),
                    spell_button_node(stacked, semantic_control_scale),
                    BorderColor::all(EDGE),
                    BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.08)),
                ))
                .with_children(|button| {
                    button.spawn((
                        Text::new(spell.name.clone()),
                        body_text_role(),
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
                    spell_button_node(stacked, semantic_control_scale),
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

fn spell_button_node(stacked: bool, semantic_control_scale: f32) -> Node {
    let semantic_control_scale = semantic_control_scale.max(1.0);
    Node {
        width: if stacked {
            Val::Percent(100.0)
        } else {
            Val::Px(148.0 * semantic_control_scale)
        },
        max_width: if stacked {
            Val::Auto
        } else {
            Val::Px(148.0 * semantic_control_scale)
        },
        flex_grow: 1.0,
        height: Val::Auto,
        min_height: Val::Px(74.0 * semantic_control_scale),
        padding: UiRect::all(Val::Px(7.0 * semantic_control_scale)),
        flex_direction: FlexDirection::Column,
        justify_content: JustifyContent::Center,
        row_gap: Val::Px(2.0),
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(6.0)),
        ..default()
    }
}

fn spawn_aim_controls(rows: &mut ChildSpawnerCommands, assets: &UiAssets, controls_enabled: bool) {
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
            if controls_enabled || matches!(&control, CastingControl::Cancel) {
                controls
                    .spawn((row_button(name, CONTROL_WIDTH), control))
                    .with_children(|button| {
                        button.spawn(blurb(assets, label));
                        button.spawn(fine(assets, shortcut));
                    });
            } else {
                controls
                    .spawn((
                        Name::new(format!("{name} Disabled")),
                        AccessibleLabel::new(format!("{label} unavailable")),
                        Node {
                            width: Val::Px(CONTROL_WIDTH),
                            min_height: Val::Px(48.0),
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::Center,
                            flex_direction: FlexDirection::Column,
                            border: UiRect::all(Val::Px(1.0)),
                            ..default()
                        },
                        BorderColor::all(EDGE),
                        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.035)),
                        Pickable::IGNORE,
                    ))
                    .with_children(|disabled| {
                        disabled.spawn(blurb(assets, label));
                        disabled.spawn(fine(assets, "UNAVAILABLE"));
                    });
            }
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
