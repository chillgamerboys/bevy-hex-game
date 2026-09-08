//! Small native combat HUD and paused parameter controls.

use super::{spectator, ViewState};
use bevy::prelude::*;
use bevy::window::{MonitorSelection, PrimaryWindow, WindowMode};
use hex_arena::{
    ActorIntent, ArenaBattleSetup, ArenaControl, ArenaInput, ArenaOutcome, ArenaSession,
    ArenaTuning, BattlePreset, Spell,
};
use hex_core::arena::{ArenaEncounter, ArenaMap, ArenaReset, ArenaSelection};

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
    Encounter,
    Selection,
    Help,
    Team(usize),
    ObserverTeams,
    ObserverStatus,
}
#[derive(Component)]
pub(super) struct PausePanel;
#[derive(Component)]
pub(super) struct StartPanel;
#[derive(Component)]
pub(super) struct CombatHud;
#[derive(Component)]
pub(super) struct ObserverHud;
#[derive(Component)]
pub(super) struct ModeContent(ArenaControl);
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
    Map(ArenaMap),
    Control(ArenaControl),
    Roster(usize, i8),
    Encounter(ArenaEncounter),
    PlayerRecipe(BattlePreset),
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
                    area.spawn((text("", 12.0, MUTED), Label::Encounter));
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
    commands.spawn((Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), display: Display::None, ..default() }, GlobalZIndex(10), ObserverHud))
        .with_children(|root| {
            root.spawn((Node { position_type: PositionType::Absolute, top: px(14), left: px(18), padding: UiRect::all(px(12)), border_radius: BorderRadius::all(px(6)), ..default() }, BackgroundColor(PANEL), text("", 17.0, INK), Label::ObserverTeams));
            root.spawn((Node { position_type: PositionType::Absolute, top: px(14), right: px(18), max_width: px(450), padding: UiRect::all(px(12)), border_radius: BorderRadius::all(px(6)), ..default() }, BackgroundColor(PANEL), text("", 17.0, INK), Label::ObserverStatus));
            root.spawn((Node { position_type: PositionType::Absolute, bottom: px(14), width: percent(100), padding: UiRect::horizontal(px(18)), justify_content: JustifyContent::Center, ..default() }, Name::new("Observer footer container"))).with_children(|footer| {
                footer.spawn((text("WASD pan / move   Q / E down / up   SHIFT fast   MOUSE look   WHEEL orbit zoom   C orbit / free   ESC / TAB pause   R reset", 12.0, INK), TextShadow { offset: Vec2::splat(1.5), color: Color::BLACK }, Name::new("Observer footer text")));
            });
        });
    commands.spawn((Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), align_items: AlignItems::Center, justify_content: JustifyContent::Center, ..default() },
        BackgroundColor(Color::srgba(0.01, 0.02, 0.035, 0.78)), GlobalZIndex(20), StartPanel))
        .with_children(|overlay| {
            overlay.spawn((Node { width: px(600), max_width: percent(95), padding: UiRect::all(px(22)), flex_direction: FlexDirection::Column, row_gap: px(10), border_radius: BorderRadius::all(px(12)), ..default() }, BackgroundColor(PANEL)))
                .with_children(|panel| {
                    panel.spawn(text("SPELL ARENA", 30.0, INK));
                    panel.spawn(Node { height: px(34), column_gap: px(8), ..default() }).with_children(|row| {
                        for (label, control) in [("PLAY", ArenaControl::Player), ("SPECTATE BATTLE", ArenaControl::Spectator)] {
                            row.spawn((Button, Node { flex_grow: 1.0, flex_basis: px(0), height: px(34), align_items: AlignItems::Center, justify_content: JustifyContent::Center, border_radius: BorderRadius::all(px(4)), ..default() }, BackgroundColor(PANEL), Action::Control(control))).with_children(|button| { button.spawn(text(label, 14.0, INK)); });
                        }
                    });
                    panel.spawn(text("MAP", 12.0, MUTED));
                    panel.spawn(Node { height: px(38), column_gap: px(8), ..default() }).with_children(|row| {
                        for map in [ArenaMap::Duel, ArenaMap::Fort, ArenaMap::SevenRegions] {
                            row.spawn((Button, Node { flex_grow: 1.0, flex_basis: px(0), height: px(38), align_items: AlignItems::Center, justify_content: JustifyContent::Center, border_radius: BorderRadius::all(px(4)), ..default() }, BackgroundColor(PANEL), Action::Map(map)))
                                .with_children(|button| { button.spawn(text(super::map_name(map), 14.0, INK)); });
                        }
                    });
                    panel.spawn((Node { flex_direction: FlexDirection::Column, row_gap: px(6), ..default() }, ModeContent(ArenaControl::Player))).with_children(|panel| {
                    panel.spawn(text("FORT ENCOUNTER", 12.0, MUTED));
                    panel.spawn(Node { height: px(38), column_gap: px(6), ..default() }).with_children(|row| {
                        for encounter in [ArenaEncounter::Dragon, ArenaEncounter::Goblins, ArenaEncounter::ShamanParty, ArenaEncounter::Shadow] {
                            row.spawn((Button, Node { flex_grow: 1.0, flex_basis: px(0), height: px(38), align_items: AlignItems::Center, justify_content: JustifyContent::Center, border_radius: BorderRadius::all(px(4)), ..default() }, BackgroundColor(PANEL), Action::Encounter(encounter)))
                                .with_children(|button| { button.spawn(text(super::encounter_name(encounter), 13.0, INK)); });
                        }
                        row.spawn((Button, Node { flex_grow: 1.0, flex_basis: px(0), height: px(38), align_items: AlignItems::Center, justify_content: JustifyContent::Center, border_radius: BorderRadius::all(px(4)), ..default() }, BackgroundColor(PANEL), Action::PlayerRecipe(BattlePreset::Golem)))
                            .with_children(|button| { button.spawn(text("Golem", 13.0, INK)); });
                    });
                    });
                    panel.spawn((Node { flex_direction: FlexDirection::Column, row_gap: px(6), display: Display::None, ..default() }, ModeContent(ArenaControl::Spectator))).with_children(|panel| {
                        for slot in 0..2 {
                            panel.spawn(Node { height: px(34), align_items: AlignItems::Center, column_gap: px(8), ..default() }).with_children(|row| {
                                row.spawn((Node { flex_grow: 1.0, ..default() }, text("", 15.0, if slot == 0 { Color::srgb(0.24,0.82,1.0) } else { Color::srgb(1.0,0.62,0.20) }), Label::Team(slot)));
                                for (label, step) in [("<", -1), (">", 1)] {
                                    row.spawn((Button, Node { width: px(46), height: px(34), align_items: AlignItems::Center, justify_content: JustifyContent::Center, ..default() }, BackgroundColor(Color::srgb(0.14,0.21,0.26)), Action::Roster(slot, step))).with_children(|button| { button.spawn(text(label, 18.0, INK)); });
                                }
                            });
                        }
                    });
                    panel.spawn((text("", 13.0, INK), Label::Selection));
                    panel.spawn((text("", 14.0, INK), Label::Help));
                    panel.spawn(text("ESC or TAB pauses combat and frees the mouse.\nUse the paused menu for fullscreen, tuning, or quitting.", 16.0, Color::srgb(0.36, 0.90, 0.78)));
                    panel.spawn(text("Combat waits until you start.", 15.0, INK));
                    panel.spawn((Button, Node { width: percent(100), height: px(46), justify_content: JustifyContent::Center, align_items: AlignItems::Center, border_radius: BorderRadius::all(px(5)), ..default() }, BackgroundColor(Color::srgb(0.16,0.37,0.41)), Action::Start))
                        .with_children(|button| { button.spawn(text("START  /  ENTER", 17.0, INK)); });
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
                    panel.spawn((Node { height: px(32), flex_shrink: 0.0, ..default() }, text("Splash passes through walls. Fireballs can hurt their caster.\nShield walls remain until destroyed; restart restores all terrain.", 12.0, MUTED)));
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
    mut selection: ResMut<ArenaSelection>,
    mut battle: ResMut<ArenaBattleSetup>,
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
            Action::Map(map)
                if !state.started
                    && selection.map != map
                    && !(battle.control == ArenaControl::Spectator
                        && map == ArenaMap::SevenRegions) =>
            {
                selection.map = map;
                if map != ArenaMap::Fort {
                    battle.player_recipe = None;
                }
                reset.generation = reset.generation.saturating_add(1);
                state.prepare_round();
            }
            Action::Control(control) if !state.started && battle.control != control => {
                battle.control = control;
                if control == ArenaControl::Spectator {
                    battle.player_recipe = None;
                }
                if control == ArenaControl::Spectator && selection.map == ArenaMap::SevenRegions {
                    selection.map = ArenaMap::Fort;
                }
                reset.generation = reset.generation.saturating_add(1);
                state.prepare_round();
            }
            Action::Roster(slot, step)
                if !state.started && battle.control == ArenaControl::Spectator =>
            {
                let current = spectator::preset_for(&battle, slot).unwrap_or(BattlePreset::Shadow);
                let index = BattlePreset::ALL
                    .iter()
                    .position(|preset| *preset == current)
                    .unwrap_or(0);
                let next = if step > 0 {
                    (index + 1) % BattlePreset::ALL.len()
                } else {
                    (index + BattlePreset::ALL.len() - 1) % BattlePreset::ALL.len()
                };
                let preset = BattlePreset::ALL
                    .get(next)
                    .copied()
                    .unwrap_or(BattlePreset::Shadow);
                if !spectator::choose_preset(&mut battle, slot, preset) {
                    continue;
                }
                reset.generation = reset.generation.saturating_add(1);
                state.prepare_round();
            }
            Action::Encounter(encounter)
                if !state.started
                    && battle.control == ArenaControl::Player
                    && selection.map == ArenaMap::Fort
                    && (selection.encounter != encounter || battle.player_recipe.is_some()) =>
            {
                selection.encounter = encounter;
                battle.player_recipe = None;
                reset.generation = reset.generation.saturating_add(1);
                state.prepare_round();
            }
            Action::PlayerRecipe(recipe)
                if !state.started
                    && battle.control == ArenaControl::Player
                    && selection.map == ArenaMap::Fort
                    && battle.player_recipe != Some(recipe) =>
            {
                battle.player_recipe = Some(recipe);
                reset.generation = reset.generation.saturating_add(1);
                state.prepare_round();
            }
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
    selection: Option<Res<ArenaSelection>>,
    battle: Option<Res<ArenaBattleSetup>>,
    mut mode_contents: Query<(&ModeContent, &mut Node), Without<CombatHud>>,
    mut labels: Query<(&Label, &mut Text)>,
    mut cards: Query<(&SpellCard, &mut BorderColor)>,
    mut choices: Query<(&Action, &mut BackgroundColor)>,
    mut panels: Query<
        (
            &mut Node,
            Has<PausePanel>,
            Has<StartPanel>,
            Has<CombatHud>,
            Has<ObserverHud>,
            Option<&ChargeNode>,
        ),
        (
            Without<ModeContent>,
            Or<(
                With<PausePanel>,
                With<StartPanel>,
                With<CombatHud>,
                With<ObserverHud>,
                With<ChargeNode>,
            )>,
        ),
    >,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    let selection = selection.as_deref().copied().unwrap_or_default();
    let battle = battle.as_deref().cloned().unwrap_or_default();
    let observing = spectator::active(&session);
    for (content, mut node) in &mut mode_contents {
        node.display = if content.0 == battle.control {
            Display::Flex
        } else {
            Display::None
        };
    }
    for (action, mut color) in &mut choices {
        let selected = match action {
            Action::Control(control) => *control == battle.control,
            Action::Map(map) => *map == selection.map,
            Action::Encounter(encounter) => {
                selection.map == ArenaMap::Fort
                    && battle.player_recipe.is_none()
                    && *encounter == selection.encounter
            }
            Action::PlayerRecipe(recipe) => {
                selection.map == ArenaMap::Fort && battle.player_recipe == Some(*recipe)
            }
            _ => continue,
        };
        *color = BackgroundColor(if selected {
            Color::srgb(0.16, 0.40, 0.43)
        } else {
            Color::srgb(0.11, 0.16, 0.20)
        });
    }
    let actor = session
        .human_actor_id()
        .and_then(|id| session.actors.iter().find(|actor| actor.id == id));
    let charge = actor.and_then(|actor| actor.charge());
    let progress = charge.map_or(0.0, |charge| {
        (charge.elapsed / tuning.charge_seconds).clamp(0.0, 1.0)
    });
    for (mut node, pause, start, combat, observer, charge_node) in &mut panels {
        if let Some(kind) = charge_node {
            let visible =
                state.started && !state.paused && !session.is_finished() && charge.is_some();
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
            || (combat && state.started && !observing)
            || (observer && state.started && observing)
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
            Label::Team(slot) => format!("TEAM {}  /  {}", slot + 1, spectator::preset_for(&battle, *slot).map_or("Custom", BattlePreset::label)),
            Label::ObserverTeams => session.battle_summary().map_or_else(String::new, |summary| spectator::team_status(&summary)),
            Label::ObserverStatus => session.battle_summary().map_or_else(String::new, |summary| spectator::battle_status(&summary, state.observer.mode, state.paused)),
            Label::Help if battle.control == ArenaControl::Spectator => "WASD pan / move / Q and E down and up / Shift fast
Mouse look / Wheel orbit zoom / C orbit or free camera
Camera movement never controls a creature.".into(),
            Label::Help => "WASD move / mouse look / Space jump / Shift sprint
1 Shield / 2 Fireball / 3 Area Blast
Hold mouse to charge Shield or Fireball. Release to cast.
Area Blast casts on release with fixed power.".into(),
            Label::Selection if battle.control == ArenaControl::Spectator => format!("{} / Seed {} / Two independent teams
Seven Regions is available in Play mode.", super::map_name(selection.map), battle.seed),
            Label::Selection => match selection.map {
                ArenaMap::Duel => "Duel: the original Shadow challenge.".into(),
                ArenaMap::Fort => format!("Fort: {}. Restart keeps this encounter.", battle.player_recipe.map_or(super::encounter_name(selection.encounter), BattlePreset::label)),
                ArenaMap::SevenRegions => "Seven Regions: Dragon, Shaman party and Goblins.\nFort encounter buttons apply only to Fort.".into(),
            },
            Label::Encounter => {
                let summary = session.encounter_summary();
                if selection.map == ArenaMap::Duel {
                    "DUEL  /  SHADOW CHALLENGE".into()
                } else {
                    format!("{}  /  {} / {} parties cleared", super::map_name(selection.map), summary.defeated_parties, session.parties().len())
                }
            },
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
                if state.capture.is_some() && super::encounter::stress_view(&state.capture_view) {
                    "SYNTHETIC PERFORMANCE FIXTURE\nExtra HP / scripted party visits".into()
                } else if let Some(outcome) = &session.outcome {
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
