//! Small native combat HUD and paused parameter controls.

use super::ViewState;
use bevy::prelude::*;
use hex_arena::{ArenaOutcome, ArenaSession, ArenaTuning, Spell};
use hex_core::arena::ArenaReset;

const INK: Color = Color::srgb(0.91, 0.94, 0.96);
const MUTED: Color = Color::srgb(0.57, 0.66, 0.73);
const PANEL: Color = Color::srgba(0.035, 0.055, 0.075, 0.94);

#[derive(Component)]
enum Label {
    Health,
    Status,
    Spell(usize),
    Parameter(usize),
}
#[derive(Component)]
struct PausePanel;
#[derive(Component)]
struct SpellCard(usize);
#[derive(Component, Clone, Copy)]
enum Action {
    Resume,
    Restart,
    Change(usize, f32),
}

fn text(value: impl Into<String>, size: f32, color: Color) -> (Text, TextFont, TextColor) {
    (
        Text::new(value),
        TextFont {
            font_size: size,
            ..default()
        },
        TextColor(color),
    )
}

pub(super) fn setup(mut commands: Commands) {
    commands.spawn((Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() }, GlobalZIndex(10)))
        .with_children(|root| {
            root.spawn((Node { position_type: PositionType::Absolute, top: px(24), left: px(30), flex_direction: FlexDirection::Column, row_gap: px(5), ..default() }))
                .with_children(|area| {
                    area.spawn(text("SPELL ARENA", 25.0, INK));
                    area.spawn(text("OFFLINE DUEL  /  COMBAT EXPERIMENT", 11.0, MUTED));
                    area.spawn((text("100 HP", 32.0, Color::srgb(0.36, 0.90, 0.78)), Label::Health));
                });
            root.spawn((Node { position_type: PositionType::Absolute, right: px(30), top: px(26), max_width: px(450), ..default() }, text("", 15.0, INK), Label::Status));
            root.spawn((Node { position_type: PositionType::Absolute, top: percent(50), left: percent(50), margin: UiRect { left: px(-7), top: px(-15), ..default() }, ..default() }, text("+", 24.0, INK)));
            root.spawn(Node { position_type: PositionType::Absolute, bottom: px(52), width: percent(100), justify_content: JustifyContent::Center, column_gap: px(10), ..default() })
                .with_children(|bar| {
                    for (index, name) in ["1  SHIELD", "2  FIREBALL", "3  AREA BLAST"].into_iter().enumerate() {
                        bar.spawn((Node { width: px(190), min_height: px(64), padding: UiRect::all(px(14)), border: UiRect::all(px(2)), ..default() },
                            BackgroundColor(PANEL), BorderColor::all(MUTED), BorderRadius::all(px(7)), SpellCard(index)))
                            .with_children(|card| { card.spawn((text(format!("{name}\nREADY"), 15.0, INK), Label::Spell(index))); });
                    }
                });
            root.spawn((Node { position_type: PositionType::Absolute, bottom: px(20), width: percent(100), justify_content: JustifyContent::Center, ..default() },
                text("WASD move   SHIFT sprint   SPACE jump   CLICK cast   C camera   T trajectory   ESC tune   R restart", 12.0, INK)));
        });
    commands.spawn((Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), align_items: AlignItems::Center, justify_content: JustifyContent::Center, display: Display::None, ..default() },
        BackgroundColor(Color::srgba(0.01, 0.02, 0.035, 0.72)), GlobalZIndex(20), PausePanel))
        .with_children(|overlay| {
            overlay.spawn((Node { width: px(600), max_width: percent(95), padding: UiRect::all(px(24)), flex_direction: FlexDirection::Column, row_gap: px(8), ..default() }, BackgroundColor(PANEL), BorderRadius::all(px(12))))
                .with_children(|panel| {
                    panel.spawn(text("PAUSED / COMBAT TUNING", 24.0, INK));
                    panel.spawn(text("Change one variable at a time. Sizes are independent.", 13.0, MUTED));
                    for index in 0..12 {
                        panel.spawn(Node { width: percent(100), align_items: AlignItems::Center, justify_content: JustifyContent::SpaceBetween, ..default() }).with_children(|row| {
                            row.spawn((Node { width: px(375), ..default() }, text("", 15.0, INK), Label::Parameter(index)));
                            for (label, amount) in [("-", -1.0), ("+", 1.0)] {
                                row.spawn((Button, Node { width: px(48), height: px(30), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() }, BackgroundColor(Color::srgb(0.14,0.21,0.26)), BorderRadius::all(px(4)), Action::Change(index, amount)))
                                    .with_children(|button| { button.spawn(text(label, 20.0, INK)); });
                            }
                        });
                    }
                    panel.spawn(text("Splash passes through walls. Your fireball can hurt you.\nShield walls remain until destroyed; restart restores all terrain.", 12.0, MUTED));
                    panel.spawn(Node { column_gap: px(12), margin: UiRect::top(px(10)), ..default() }).with_children(|row| {
                        for (label, action) in [("RESUME", Action::Resume), ("RESET ARENA", Action::Restart)] {
                            row.spawn((Button, Node { width: px(260), height: px(42), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() }, BackgroundColor(Color::srgb(0.16,0.37,0.41)), BorderRadius::all(px(5)), action))
                                .with_children(|button| { button.spawn(text(label, 15.0, INK)); });
                        }
                    });
                });
        });
}

pub(super) fn buttons(
    interactions: Query<(&Interaction, &Action), Changed<Interaction>>,
    mut state: ResMut<ViewState>,
    mut tuning: ResMut<ArenaTuning>,
    mut reset: ResMut<ArenaReset>,
) {
    for (interaction, action) in &interactions {
        if *interaction != Interaction::Pressed || !state.paused {
            continue;
        }
        match *action {
            Action::Resume => {
                state.paused = false;
                state.suppress_click = true;
            }
            Action::Restart => {
                reset.generation = reset.generation.saturating_add(1);
                state.paused = false;
                state.initialized = false;
                state.suppress_click = true;
            }
            Action::Change(index, direction) => change(&mut tuning, index, direction),
        }
    }
}

pub(super) fn change(t: &mut ArenaTuning, field: usize, direction: f32) {
    let size = |v: &mut usize| {
        *v = if direction > 0.0 {
            v.saturating_add(1).min(2)
        } else {
            v.saturating_sub(1)
        };
    };
    match field {
        0 => size(&mut t.shield_size),
        1 => size(&mut t.fireball_size),
        2 => size(&mut t.blast_size),
        3 => t.projectile_speed = (t.projectile_speed + direction * 2.0).clamp(8.0, 64.0),
        4 => t.projectile_gravity = (t.projectile_gravity + direction * 2.0).clamp(2.0, 40.0),
        5 => t.shield_cooldown = (t.shield_cooldown + direction * 0.5).clamp(0.5, 20.0),
        6 => t.fireball_cooldown = (t.fireball_cooldown + direction * 0.25).clamp(0.25, 10.0),
        7 => t.blast_cooldown = (t.blast_cooldown + direction * 0.5).clamp(0.5, 20.0),
        8 => t.fireball_damage = (t.fireball_damage + direction * 5.0).clamp(5.0, 100.0),
        9 => t.blast_damage = (t.blast_damage + direction * 5.0).clamp(5.0, 100.0),
        10 => t.fireball_knockback = (t.fireball_knockback + direction).clamp(0.0, 25.0),
        11 => t.blast_knockback = (t.blast_knockback + direction).clamp(0.0, 25.0),
        _ => {}
    }
}

pub(super) fn update(
    session: Res<ArenaSession>,
    tuning: Res<ArenaTuning>,
    state: Res<ViewState>,
    mut labels: Query<(&Label, &mut Text)>,
    mut cards: Query<(&SpellCard, &mut BorderColor)>,
    mut panels: Query<&mut Node, With<PausePanel>>,
) {
    let actor = session.actors.first();
    for mut node in &mut panels {
        node.display = if state.paused {
            Display::Flex
        } else {
            Display::None
        };
    }
    let size_name = |index| {
        ["Compact", "Standard", "Large"]
            .get(index)
            .copied()
            .unwrap_or("Standard")
    };
    for (label, mut text) in &mut labels {
        text.0 = match label {
            Label::Health => format!("{:03.0} HP", actor.map_or(100.0, |a| a.hp)),
            Label::Status => {
                if let Some(outcome) = &session.outcome {
                    format!(
                        "{}\nR to restart",
                        match outcome {
                            ArenaOutcome::Winner(0) => "YOU WIN",
                            ArenaOutcome::Winner(_) => "YOU WERE KNOCKED OUT",
                            ArenaOutcome::Draw => "DOUBLE KNOCKOUT",
                        }
                    )
                } else {
                    format!(
                        "{}  /  {}\n{}",
                        if state.third_person {
                            "CLOSE THIRD PERSON"
                        } else {
                            "FIRST PERSON"
                        },
                        if state.paused { "PAUSED" } else { "LIVE" },
                        session.notice
                    )
                }
            }
            Label::Spell(index) => {
                let spell = [Spell::Shield, Spell::Fireball, Spell::AreaBlast]
                    .get(*index)
                    .copied()
                    .unwrap_or(Spell::Shield);
                let cooldown = actor
                    .and_then(|a| a.cooldowns.get(*index))
                    .copied()
                    .unwrap_or(0.0);
                format!(
                    "{}  {}\n{}",
                    index + 1,
                    spell.name().to_uppercase(),
                    if cooldown > 0.0 {
                        format!("{cooldown:.1}s")
                    } else {
                        "READY".into()
                    }
                )
            }
            Label::Parameter(index) => match index {
                0 => format!(
                    "Shield size                 {}",
                    size_name(tuning.shield_size)
                ),
                1 => format!(
                    "Fireball size                {}",
                    size_name(tuning.fireball_size)
                ),
                2 => format!("Area Blast size           {}", size_name(tuning.blast_size)),
                3 => format!(
                    "Launch speed              {:.0} units/s",
                    tuning.projectile_speed
                ),
                4 => format!(
                    "Projectile gravity       {:.0} units/s²",
                    tuning.projectile_gravity
                ),
                5 => format!("Shield cooldown          {:.1}s", tuning.shield_cooldown),
                6 => format!("Fireball cooldown        {:.2}s", tuning.fireball_cooldown),
                7 => format!("Area Blast cooldown    {:.1}s", tuning.blast_cooldown),
                8 => format!("Fireball damage           {:.0} HP", tuning.fireball_damage),
                9 => format!("Area Blast damage      {:.0} HP", tuning.blast_damage),
                10 => format!("Fireball knockback       {:.0}", tuning.fireball_knockback),
                11 => format!("Area Blast knockback  {:.0}", tuning.blast_knockback),
                _ => String::new(),
            },
        };
    }
    for (card, mut border) in &mut cards {
        let selected = actor.is_some_and(|a| a.selected.index() == card.0);
        *border = BorderColor::all(if selected {
            Color::srgb(0.35, 0.94, 0.79)
        } else {
            Color::srgb(0.19, 0.27, 0.32)
        });
    }
}
