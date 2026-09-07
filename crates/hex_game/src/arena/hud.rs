//! Small native combat HUD and paused parameter controls.

use super::ViewState;
use bevy::prelude::*;
use bevy::window::{MonitorSelection, PrimaryWindow, WindowMode};
use hex_arena::{ActorIntent, ArenaInput, ArenaOutcome, ArenaSession, ArenaTuning, Spell};
use hex_core::arena::ArenaReset;

const INK: Color = Color::srgb(0.91, 0.94, 0.96);
const MUTED: Color = Color::srgb(0.57, 0.66, 0.73);
const PANEL: Color = Color::srgba(0.035, 0.055, 0.075, 0.94);

#[derive(Component)]
pub(super) enum Label {
    Health,
    Status,
    Spell(usize),
    Parameter(usize),
    WindowMode,
    Charge,
}
#[derive(Component)]
pub(super) struct PausePanel;
#[derive(Component)]
pub(super) struct StartPanel;
#[derive(Component)]
pub(super) struct CombatHud;
#[derive(Component)]
pub(super) struct SpellCard(usize);
#[derive(Component)]
pub(super) enum ChargeNode {
    Panel,
    Track,
    Fill,
}
#[derive(Component, Clone, Copy)]
pub(super) enum Action {
    Start,
    Resume,
    Restart,
    Fullscreen,
    Quit,
    Change(usize, f32),
}

fn text(value: impl Into<String>, size: f32, color: Color) -> (Text, TextFont, TextColor) {
    (
        Text::new(value),
        TextFont {
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
    )
}

pub(super) fn setup(mut commands: Commands) {
    commands.spawn((Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() }, GlobalZIndex(10), CombatHud))
        .with_children(|root| {
            root.spawn((Node { position_type: PositionType::Absolute, top: px(14), left: px(18), padding: UiRect::all(px(12)), border_radius: BorderRadius::all(px(6)), flex_direction: FlexDirection::Column, row_gap: px(5), ..default() }, BackgroundColor(PANEL)))
                .with_children(|area| {
                    area.spawn(text("SPELL ARENA", 25.0, INK));
                    area.spawn(text("OFFLINE DUEL  /  COMBAT EXPERIMENT", 11.0, MUTED));
                    area.spawn((text("100 HP", 32.0, Color::srgb(0.36, 0.90, 0.78)), Label::Health));
                });
            root.spawn((Node { position_type: PositionType::Absolute, right: px(18), top: px(14), max_width: px(474), padding: UiRect::all(px(12)), border_radius: BorderRadius::all(px(6)), ..default() }, BackgroundColor(PANEL), text("", 15.0, INK), Label::Status));
            root.spawn((Node { position_type: PositionType::Absolute, top: percent(50), left: percent(50), margin: UiRect { left: px(-7), top: px(-15), ..default() }, ..default() }, text("+", 24.0, INK), TextShadow { offset: Vec2::splat(1.5), color: Color::BLACK }));
            root.spawn((Node { position_type: PositionType::Absolute, top: percent(50), left: percent(50), width: px(236), margin: UiRect { left: px(-118), top: px(28), ..default() }, padding: UiRect::all(px(8)), flex_direction: FlexDirection::Column, align_items: AlignItems::Center, row_gap: px(6), border_radius: BorderRadius::all(px(5)), display: Display::None, ..default() }, BackgroundColor(PANEL), ChargeNode::Panel, Name::new("Charge panel")))
                .with_children(|charge| {
                    charge.spawn((Node { width: percent(100), height: px(8), flex_shrink: 0.0, border_radius: BorderRadius::all(px(4)), overflow: Overflow::clip(), ..default() }, BackgroundColor(Color::srgb(0.18, 0.26, 0.31)), ChargeNode::Track, Name::new("Charge track")))
                        .with_children(|track| { track.spawn((Node { width: percent(0), height: percent(100), ..default() }, BackgroundColor(Color::srgb(0.35, 0.94, 0.79)), ChargeNode::Fill, Name::new("Charge fill"))); });
                    charge.spawn((text("", 14.0, INK), Label::Charge, Name::new("Charge guidance")));
                });
            root.spawn(Node { position_type: PositionType::Absolute, bottom: px(52), width: percent(100), justify_content: JustifyContent::Center, column_gap: px(10), ..default() })
                .with_children(|bar| {
                    for (index, name) in ["1  SHIELD", "2  FIREBALL", "3  AREA BLAST"].into_iter().enumerate() {
                        bar.spawn((Node { width: px(190), min_height: px(64), padding: UiRect::all(px(14)), border: UiRect::all(px(2)), border_radius: BorderRadius::all(px(7)), ..default() },
                            BackgroundColor(PANEL), BorderColor::all(MUTED), SpellCard(index)))
                            .with_children(|card| { card.spawn((text(format!("{name}\nREADY"), 15.0, INK), Label::Spell(index))); });
                    }
                });
            root.spawn(Node { position_type: PositionType::Absolute, bottom: px(14), width: percent(100), height: px(28), justify_content: JustifyContent::Center, ..default() })
                .with_children(|footer| { footer.spawn((Node { padding: UiRect::axes(px(12), px(6)), border_radius: BorderRadius::all(px(4)), ..default() }, BackgroundColor(PANEL),
                    text("WASD move   SHIFT sprint   SPACE jump   HOLD charge / RELEASE cast   C camera   T trajectory   ESC / TAB pause   R reset", 12.0, INK))); });
        });
    commands.spawn((Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), align_items: AlignItems::Center, justify_content: JustifyContent::Center, ..default() },
        BackgroundColor(Color::srgba(0.01, 0.02, 0.035, 0.78)), GlobalZIndex(20), StartPanel))
        .with_children(|overlay| {
            overlay.spawn((Node { width: px(600), max_width: percent(95), padding: UiRect::all(px(28)), flex_direction: FlexDirection::Column, row_gap: px(18), border_radius: BorderRadius::all(px(12)), ..default() }, BackgroundColor(PANEL)))
                .with_children(|panel| {
                    panel.spawn(text("SPELL ARENA", 36.0, INK));
                    panel.spawn(text("One player. One opponent. Three spells.", 18.0, INK));
                    panel.spawn(text("WASD move / mouse look / Space jump / Shift sprint\n1 Shield / 2 Fireball / 3 Area Blast\nHold mouse to charge Shield or Fireball. Release to cast.\nArea Blast casts on release with fixed power.", 15.0, INK));
                    panel.spawn(text("ESC or TAB pauses combat and frees the mouse.\nUse the paused menu for fullscreen, tuning, or quitting.", 16.0, Color::srgb(0.36, 0.90, 0.78)));
                    panel.spawn(text("Combat waits until you start.", 15.0, INK));
                    panel.spawn((Button, Node { width: percent(100), height: px(46), justify_content: JustifyContent::Center, align_items: AlignItems::Center, border_radius: BorderRadius::all(px(5)), ..default() }, BackgroundColor(Color::srgb(0.16,0.37,0.41)), Action::Start))
                        .with_children(|button| { button.spawn(text("START DUEL  /  ENTER", 17.0, INK)); });
                    panel.spawn(Node { height: px(42), column_gap: px(12), ..default() }).with_children(|row| {
                        for (label, action) in [("FULLSCREEN", Action::Fullscreen), ("QUIT GAME", Action::Quit)] {
                            row.spawn((Button, Node { flex_grow: 1.0, flex_basis: px(0), height: px(42), justify_content: JustifyContent::Center, align_items: AlignItems::Center, border_radius: BorderRadius::all(px(5)), ..default() }, BackgroundColor(Color::srgb(0.14,0.21,0.26)), action))
                                .with_children(|button| {
                                    let mut label_entity = button.spawn(text(label, 15.0, INK));
                                    if matches!(action, Action::Fullscreen) { label_entity.insert(Label::WindowMode); }
                                });
                        }
                    });
                });
        });
    commands.spawn((Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), align_items: AlignItems::Center, justify_content: JustifyContent::Center, display: Display::None, ..default() },
        BackgroundColor(Color::srgba(0.01, 0.02, 0.035, 0.72)), GlobalZIndex(20), PausePanel))
        .with_children(|overlay| {
            overlay.spawn((Node { width: px(600), height: px(710), max_width: percent(95), padding: UiRect::all(px(20)), flex_direction: FlexDirection::Column, row_gap: px(5), flex_shrink: 0.0, border_radius: BorderRadius::all(px(12)), ..default() }, BackgroundColor(PANEL)))
                .with_children(|panel| {
                    panel.spawn((Node { height: px(30), flex_shrink: 0.0, ..default() }, text("PAUSED / COMBAT MENU", 24.0, INK)));
                    panel.spawn((Node { height: px(18), flex_shrink: 0.0, ..default() }, text("Mouse is free. ESC / TAB resumes. Sizes are independent.", 13.0, INK)));
                    for index in 0..12 {
                        panel.spawn(Node { width: percent(100), height: px(30), flex_shrink: 0.0, align_items: AlignItems::Center, justify_content: JustifyContent::SpaceBetween, ..default() }).with_children(|row| {
                            row.spawn((Node { width: px(375), height: px(20), flex_shrink: 0.0, ..default() }, text("", 15.0, INK), Label::Parameter(index)));
                            for (label, amount) in [("-", -1.0), ("+", 1.0)] {
                                row.spawn((Button, Node { width: px(48), height: px(30), border_radius: BorderRadius::all(px(4)), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() }, BackgroundColor(Color::srgb(0.14,0.21,0.26)), Action::Change(index, amount)))
                                    .with_children(|button| { button.spawn(text(label, 20.0, INK)); });
                            }
                        });
                    }
                    panel.spawn((Node { height: px(32), flex_shrink: 0.0, ..default() }, text("Splash passes through walls. Your fireball can hurt you.\nShield walls remain until destroyed; restart restores all terrain.", 12.0, MUTED)));
                    panel.spawn(Node { height: px(42), flex_shrink: 0.0, column_gap: px(12), margin: UiRect::top(px(8)), ..default() }).with_children(|row| {
                        for (label, action) in [("RESUME", Action::Resume), ("RESET ARENA", Action::Restart)] {
                            row.spawn((Button, Node { width: px(260), height: px(42), border_radius: BorderRadius::all(px(5)), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() }, BackgroundColor(Color::srgb(0.16,0.37,0.41)), action))
                                .with_children(|button| { button.spawn(text(label, 15.0, INK)); });
                        }
                    });
                    panel.spawn(Node { height: px(42), flex_shrink: 0.0, column_gap: px(12), ..default() }).with_children(|row| {
                        for (label, action) in [("FULLSCREEN", Action::Fullscreen), ("QUIT GAME", Action::Quit)] {
                            row.spawn((Button, Node { width: px(260), height: px(42), border_radius: BorderRadius::all(px(5)), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() }, BackgroundColor(Color::srgb(0.14,0.21,0.26)), action))
                                .with_children(|button| {
                                    let mut label_entity = button.spawn(text(label, 15.0, INK));
                                    if matches!(action, Action::Fullscreen) { label_entity.insert(Label::WindowMode); }
                                });
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
    mut input: ResMut<ArenaInput>,
    mut session: ResMut<ArenaSession>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut exit: MessageWriter<AppExit>,
) {
    for (interaction, action) in &interactions {
        if *interaction != Interaction::Pressed
            || !state.paused
            || windows.iter().any(|window| !window.focused)
        {
            continue;
        }
        match *action {
            Action::Start if !state.started => state.begin_play(),
            Action::Resume if state.started => state.begin_play(),
            Action::Restart if state.started => {
                reset.generation = reset.generation.saturating_add(1);
                state.prepare_round();
            }
            Action::Fullscreen => {
                for mut window in &mut windows {
                    window.mode = if window.mode == WindowMode::Windowed {
                        WindowMode::BorderlessFullscreen(MonitorSelection::Current)
                    } else {
                        WindowMode::Windowed
                    };
                }
            }
            Action::Quit => {
                exit.write(AppExit::Success);
            }
            Action::Change(index, direction) if state.started => {
                change(&mut tuning, index, direction)
            }
            _ => continue,
        }
        // UI clicks cannot leak into casting when the simulation resumes.
        session.cancel_charges();
        input.human = ActorIntent {
            aim: super::aim(&state),
            ..default()
        };
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
    mut panels: Query<
        (
            &mut Node,
            Has<PausePanel>,
            Has<StartPanel>,
            Has<CombatHud>,
            Option<&ChargeNode>,
        ),
        Or<(
            With<PausePanel>,
            With<StartPanel>,
            With<CombatHud>,
            With<ChargeNode>,
        )>,
    >,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    let actor = session.actors.first();
    let charge = actor.and_then(|actor| actor.charge());
    let progress = charge.map_or(0.0, |charge| {
        (charge.elapsed / tuning.charge_seconds).clamp(0.0, 1.0)
    });
    for (mut node, pause, start, combat, charge_node) in &mut panels {
        if let Some(kind) = charge_node {
            let visible =
                state.started && !state.paused && session.outcome.is_none() && charge.is_some();
            let has_bar = charge.is_some_and(|charge| charge.spell != Spell::AreaBlast);
            node.display = if visible && (matches!(kind, ChargeNode::Panel) || has_bar) {
                Display::Flex
            } else {
                Display::None
            };
            if matches!(kind, ChargeNode::Fill) {
                node.width = percent(progress * 100.0);
            }
            continue;
        }
        node.display = if (pause && state.paused && state.started)
            || (start && !state.started)
            || (combat && state.started)
        {
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
            Label::Charge => match charge {
                Some(charge) if charge.spell == Spell::AreaBlast => "Release to cast".into(),
                Some(_) => format!("{:.0}% / Release to cast", progress * 100.0),
                None => String::new(),
            },
            Label::WindowMode => {
                if windows
                    .iter()
                    .any(|window| window.mode != WindowMode::Windowed)
                {
                    "WINDOWED".into()
                } else {
                    "FULLSCREEN".into()
                }
            }
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
                        if state.external_camera() {
                            "REVIEW CAMERA"
                        } else if state.third_person {
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
                    "Base launch speed         {:.0} units/s",
                    tuning.projectile_speed
                ),
                4 => format!(
                    "Projectile gravity       {:.0} units/s^2",
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
