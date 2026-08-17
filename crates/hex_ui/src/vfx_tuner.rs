//! Spell VFX tuner presentation.
//!
//! A Tools-menu authoring surface: the spell list on the left, the selected
//! spell's tunable parameters on the right, and Play/Save beneath them. The 3D
//! preview behind this UI belongs to `hex_game`; this module renders only the
//! panels and emits [`VfxTunerIntent`].
//!
//! The panel deliberately leaves the middle of the screen clear. Everything being
//! tuned here is something the designer has to *watch* — the two preview dummies
//! and the effect between them sit in that gap, so a parameter and its result are
//! visible at the same time rather than one behind the other.

use bevy::prelude::*;
use bevy::ui_widgets::ScrollArea;
use hex_core::Screen;

use crate::{
    blurb, button, display, divider, fine, heading, label, panel, small_button,
    transparent_screen_root, UiAssets, UiIntent, UiSystems, VfxTunerControl, VfxTunerField,
    VfxTunerIntent, VfxTunerView,
};

/// The parameter column, rebuilt whenever the view changes.
#[derive(Component)]
struct ParameterBody;

/// The spell list column, rebuilt whenever the view changes.
#[derive(Component)]
struct SpellBody;

/// The one-line save/notice readout beneath the parameters.
#[derive(Component)]
struct StatusLine;

#[derive(Component, Clone)]
struct Control(VfxTunerIntent);

#[derive(Component)]
struct BackControl;

/// Everything the two columns were last built from.
///
/// Every value is structural now that the rows are plain readouts: with nothing to
/// type into, a changed number is simply a rebuild, and the panel no longer needs
/// the in-place update path that existed only to avoid clobbering a caret.
#[derive(Component, PartialEq, Eq, Clone)]
struct RenderedShape {
    spells: Vec<(String, String, bool)>,
    rows: Vec<(VfxTunerField, String, VfxTunerControl, String)>,
}

impl RenderedShape {
    fn of(view: &VfxTunerView) -> Self {
        Self {
            spells: view
                .spells
                .iter()
                .map(|spell| (spell.name.clone(), spell.summary.clone(), spell.selected))
                .collect(),
            rows: view
                .rows
                .iter()
                .map(|row| (row.field, row.label.clone(), row.control, row.value.clone()))
                .collect(),
        }
    }
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Screen::VfxTuner), spawn)
        .add_systems(
            Update,
            (
                rebuild.in_set(UiSystems::Render),
                emit_intents.in_set(UiSystems::EmitIntents),
            )
                .run_if(in_state(Screen::VfxTuner)),
        );
}

fn spawn(mut commands: Commands, assets: Res<UiAssets>) {
    commands
        .spawn(transparent_screen_root(
            Screen::VfxTuner,
            "VFX Tuner Screen",
        ))
        .insert(Node {
            // The preview lives behind this screen, so the root lays its two
            // panels out along the edges instead of stacking them down the middle.
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::FlexStart,
            padding: UiRect::all(Val::Px(18.0)),
            column_gap: Val::Px(18.0),
            ..crate::screen_root_node()
        })
        .with_children(|parent| {
            parent
                .spawn((Name::new("VFX Tuner Spells"), panel()))
                .insert(Node {
                    width: Val::Px(260.0),
                    max_height: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(10.0),
                    padding: UiRect::all(Val::Px(16.0)),
                    ..crate::panel_node()
                })
                .with_children(|column| {
                    column.spawn(display(&assets, "VFX Tuner"));
                    column
                        .spawn((
                            button("Back"),
                            BackControl,
                            crate::UiVisibilityRequirement::Immediate,
                        ))
                        .with_child(label(&assets, "Back"));
                    column.spawn(divider(220.0));
                    column.spawn(heading(&assets, "spells"));
                    column.spawn((
                        Name::new("VFX Tuner Spell List"),
                        SpellBody,
                        ScrollArea,
                        ScrollPosition::default(),
                        Node {
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(8.0),
                            min_height: Val::Px(0.0),
                            flex_grow: 1.0,
                            overflow: Overflow::scroll_y(),
                            ..default()
                        },
                    ));
                });

            parent
                .spawn((Name::new("VFX Tuner Parameters"), panel()))
                .insert(Node {
                    width: Val::Px(420.0),
                    max_height: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(10.0),
                    padding: UiRect::all(Val::Px(16.0)),
                    ..crate::panel_node()
                })
                .with_children(|column| {
                    column.spawn(heading(&assets, "tuning"));
                    column.spawn((
                        Name::new("VFX Tuner Parameter Rows"),
                        ParameterBody,
                        ScrollArea,
                        ScrollPosition::default(),
                        Node {
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(6.0),
                            min_height: Val::Px(0.0),
                            flex_grow: 1.0,
                            overflow: Overflow::scroll_y(),
                            ..default()
                        },
                    ));
                    column.spawn(divider(380.0));
                    column
                        .spawn((
                            Name::new("VFX Tuner Actions"),
                            Node {
                                flex_direction: FlexDirection::Row,
                                flex_wrap: FlexWrap::Wrap,
                                column_gap: Val::Px(10.0),
                                row_gap: Val::Px(8.0),
                                ..default()
                            },
                        ))
                        .with_children(|actions| {
                            actions
                                .spawn((small_button("Play"), Control(VfxTunerIntent::Play)))
                                .with_children(|action| {
                                    action.spawn(blurb(&assets, "play"));
                                    action.spawn(fine(&assets, "or press Space"));
                                });
                            actions
                                .spawn((small_button("Save"), Control(VfxTunerIntent::Save)))
                                .with_children(|action| {
                                    action.spawn(blurb(&assets, "save"));
                                    action.spawn(fine(&assets, "to spell_animations.ron"));
                                });
                            actions
                                .spawn((small_button("Revert"), Control(VfxTunerIntent::Revert)))
                                .with_children(|action| {
                                    action.spawn(blurb(&assets, "revert"));
                                    action.spawn(fine(&assets, "discard edits"));
                                });
                        });
                    column.spawn((StatusLine, fine(&assets, "")));
                });
        });
}

fn rebuild(
    mut commands: Commands,
    view: Res<VfxTunerView>,
    assets: Res<UiAssets>,
    spell_bodies: Query<Entity, With<SpellBody>>,
    parameter_bodies: Query<(Entity, Option<&RenderedShape>), With<ParameterBody>>,
    mut status: Query<&mut Text, With<StatusLine>>,
) {
    if !view.is_changed() {
        return;
    }
    let (Ok(spell_body), Ok((parameter_body, shape))) =
        (spell_bodies.single(), parameter_bodies.single())
    else {
        return;
    };

    let wanted = RenderedShape::of(&view);
    if shape == Some(&wanted) {
        return;
    }
    commands.entity(parameter_body).insert(wanted);

    commands.entity(spell_body).despawn_related::<Children>();
    commands.entity(spell_body).with_children(|rows| {
        if !view.ready {
            rows.spawn(blurb(&assets, "waiting for content..."));
            return;
        }
        if view.spells.is_empty() {
            rows.spawn(blurb(&assets, "no authored animations"));
            return;
        }
        for spell in &view.spells {
            rows.spawn((
                small_button(format!("Tune {}", spell.name)),
                Control(VfxTunerIntent::Select(spell.name.clone())),
                crate::UiVisibilityRequirement::Scrollable,
                children![
                    blurb(
                        &assets,
                        if spell.selected {
                            format!("> {}", spell.name)
                        } else {
                            spell.name.clone()
                        }
                    ),
                    fine(&assets, spell.summary.clone()),
                ],
            ));
        }
    });

    commands
        .entity(parameter_body)
        .despawn_related::<Children>();
    commands.entity(parameter_body).with_children(|rows| {
        if view.rows.is_empty() {
            rows.spawn(blurb(&assets, "select a spell to tune"));
            return;
        }
        for row in &view.rows {
            spawn_parameter_row(rows, &assets, row);
        }
    });

    update_status(&view, &mut status);
}

fn update_status(view: &VfxTunerView, status: &mut Query<&mut Text, With<StatusLine>>) {
    if let Ok(mut text) = status.single_mut() {
        let notice = match (&view.status, view.dirty) {
            (Some(status), _) => status.clone(),
            (None, true) => "unsaved edits".to_owned(),
            (None, false) => "saved".to_owned(),
        };
        if text.0 != notice {
            *text = Text::new(notice);
        }
    }
}

fn spawn_parameter_row(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    row: &crate::VfxTunerRowView,
) {
    parent
        .spawn((
            Name::new(format!("VFX Tuner Row {}", row.label)),
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                min_height: Val::Px(34.0),
                ..default()
            },
        ))
        .with_children(|line| {
            line.spawn((
                Node {
                    width: Val::Px(132.0),
                    ..default()
                },
                children![fine(assets, row.label.clone())],
            ));
            match row.control {
                VfxTunerControl::Nudge => {
                    spawn_step_button(
                        line,
                        assets,
                        "-",
                        format!("{} Decrement", row.label),
                        VfxTunerIntent::Decrement(row.field),
                    );
                    // A readout, not an input. An editable box here never took
                    // keyboard focus, and a control that looks typeable but
                    // silently swallows every keystroke is worse than one that
                    // plainly shows the value the steppers change.
                    line.spawn((
                        Name::new(format!("{} Value", row.label)),
                        Node {
                            width: Val::Px(96.0),
                            min_height: Val::Px(30.0),
                            flex_shrink: 0.0,
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        children![label(assets, row.value.clone())],
                    ));
                    spawn_step_button(
                        line,
                        assets,
                        "+",
                        format!("{} Increment", row.label),
                        VfxTunerIntent::Increment(row.field),
                    );
                }
                VfxTunerControl::Cycle => {
                    line.spawn((
                        small_button(format!("{} Cycle", row.label)),
                        Control(VfxTunerIntent::Cycle(row.field)),
                        crate::UiVisibilityRequirement::Scrollable,
                        children![label(assets, row.value.clone())],
                    ));
                }
            }
        });
}

fn spawn_step_button(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    glyph: &'static str,
    name: String,
    intent: VfxTunerIntent,
) {
    parent.spawn((
        crate::row_button(name, 40.0),
        Control(intent),
        crate::UiVisibilityRequirement::Scrollable,
        children![label(assets, glyph)],
    ));
}

/// Space replays the selected spell without a trip to the Play button — the tuner's
/// whole point is edit-play-edit, and a hand already on the keyboard should not have
/// to travel back to the same button between every tweak.
fn emit_intents(
    clicked: Query<(&Interaction, &Control), Changed<Interaction>>,
    back: Query<&Interaction, (Changed<Interaction>, With<BackControl>)>,
    keys: Res<ButtonInput<KeyCode>>,
    mut intents: MessageWriter<UiIntent>,
) {
    for (interaction, control) in &clicked {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::VfxTuner(control.0.clone()));
        }
    }
    if keys.just_pressed(KeyCode::Space) {
        intents.write(UiIntent::VfxTuner(VfxTunerIntent::Play));
    }
    if back
        .iter()
        .any(|interaction| *interaction == Interaction::Pressed)
    {
        intents.write(UiIntent::Back);
    }
}

/// Field ordering is data, not layout: the adapter in `hex_game` builds rows in the
/// order a designer reads them, and this module renders whatever it is handed.
#[cfg(test)]
mod tests {
    use crate::VfxTunerField;

    #[test]
    fn every_field_has_a_distinct_identity() {
        let fields = [
            VfxTunerField::Motion,
            VfxTunerField::Style,
            VfxTunerField::TimingPrimary,
            VfxTunerField::TimingImpact,
            VfxTunerField::Trail,
            VfxTunerField::ParticleCount,
            VfxTunerField::ParticleSpeed,
            VfxTunerField::ParticleLifetime,
            VfxTunerField::Scale,
            VfxTunerField::Spread,
            VfxTunerField::ColorOverride,
            VfxTunerField::ColorRed,
            VfxTunerField::ColorGreen,
            VfxTunerField::ColorBlue,
        ];
        for (index, field) in fields.iter().enumerate() {
            assert_eq!(
                fields.iter().filter(|other| *other == field).count(),
                1,
                "field at {index} is duplicated"
            );
        }
    }
}
