//! Small native combat HUD and paused parameter controls.

use super::ux::set_display;
use super::{spectator, ViewState};
use bevy::prelude::*;
use bevy::window::{MonitorSelection, PrimaryWindow, WindowMode};
use hex_arena::{
    ActorIntent, ArenaBattleSetup, ArenaControl, ArenaInput, ArenaOutcome, ArenaSession,
    ArenaTuning, BattlePreset, ExpeditionReward, ExpeditionSnapshot, UpgradeStat,
};
use hex_core::arena::{ArenaEncounter, ArenaMap, ArenaReset, ArenaSelection};

const INK: Color = Color::srgb(0.91, 0.94, 0.96);
const MUTED: Color = Color::srgb(0.57, 0.66, 0.73);
const PANEL: Color = Color::srgba(0.035, 0.055, 0.075, 0.94);

fn milestone_status(expedition: &ExpeditionSnapshot, reward: ExpeditionReward) -> &'static str {
    expedition
        .milestones
        .iter()
        .find(|milestone| milestone.reward == reward)
        .map_or("locked", |milestone| {
            if milestone.collected {
                "collected"
            } else if milestone.available_position.is_some() {
                "orb ready to collect"
            } else if milestone.defeated {
                "defeated"
            } else {
                "locked"
            }
        })
}

#[derive(Component)]
pub(super) enum Label {
    Health,
    Rewards,
    Status,
    Spell(usize),
    Parameter(usize),
    Encounter,
    Selection,
    Help,
    Team(usize),
    ObserverTeams,
    ObserverStatus,
    MenuTitle,
    MenuHelp,
    Resume,
    MenuRules,
    WindowMode,
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

pub(super) fn text(
    value: impl Into<String>,
    size: f32,
    color: Color,
) -> (Text, TextFont, TextColor) {
    (
        Text::new(value),
        TextFont {
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
    )
}

mod layout;
pub(super) use layout::setup;

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
    recorder: Option<ResMut<super::recording::Recorder>>,
    render: Option<Res<hex_core::arena::ArenaRenderStatus>>,
) {
    let mut recorder = recorder;
    let terrain_ready = render.is_none_or(|status| status.pending_chunks == 0);
    let chooses_something_else = state.paused
        && windows.iter().all(|window| window.focused)
        && interactions.iter().any(|(interaction, action)| {
            *interaction == Interaction::Pressed
                && !matches!(
                    action,
                    Action::Map(ArenaMap::ForestMassif) | Action::Fullscreen
                )
        });
    if state.started || battle.control == ArenaControl::Spectator || chooses_something_else {
        state.forest_preparation.cancel_selection();
    }
    if state.forest_preparation.poll() {
        apply_map_selection(
            ArenaMap::ForestMassif,
            &mut state,
            &mut reset,
            &mut selection,
            &mut battle,
        );
        // A new map needs its own publication before any Start event can apply.
        return;
    }
    let cancels_northern = state.paused
        && windows.iter().all(|window| window.focused)
        && interactions.iter().any(|(interaction, action)| {
            *interaction == Interaction::Pressed
                && !matches!(
                    action,
                    Action::Map(ArenaMap::NorthernArchipelago) | Action::Fullscreen
                )
        });
    if state.started || battle.control == ArenaControl::Spectator || cancels_northern {
        state.northern_preparation.cancel_selection();
    }
    if state.northern_preparation.poll() {
        apply_map_selection(
            ArenaMap::NorthernArchipelago,
            &mut state,
            &mut reset,
            &mut selection,
            &mut battle,
        );
        return;
    }
    for (interaction, action) in &interactions {
        if *interaction != Interaction::Pressed
            || !state.paused
            || windows.iter().any(|window| !window.focused)
        {
            continue;
        }
        if !matches!(
            action,
            Action::Map(ArenaMap::ForestMassif) | Action::Fullscreen
        ) {
            state.forest_preparation.cancel_selection();
        }
        if !matches!(
            action,
            Action::Map(ArenaMap::NorthernArchipelago) | Action::Fullscreen
        ) {
            state.northern_preparation.cancel_selection();
        }
        match *action {
            Action::Map(map)
                if !state.started
                    && selection.map != map
                    && !(battle.control == ArenaControl::Spectator
                        && matches!(
                            map,
                            ArenaMap::SevenRegions
                                | ArenaMap::ForestMassif
                                | ArenaMap::NorthernArchipelago
                        )) =>
            {
                if map == ArenaMap::ForestMassif && !state.forest_preparation.request() {
                    continue;
                }
                if map == ArenaMap::NorthernArchipelago
                    && !state.northern_preparation.request_for(map)
                {
                    continue;
                }
                apply_map_selection(map, &mut state, &mut reset, &mut selection, &mut battle);
            }
            Action::Control(control)
                if !state.started
                    && battle.control != control
                    && !(matches!(
                        selection.map,
                        ArenaMap::ForestMassif | ArenaMap::NorthernArchipelago
                    ) && control == ArenaControl::Spectator) =>
            {
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
                    && matches!(selection.map, ArenaMap::Fort | ArenaMap::Duel)
                    && super::player_preset(*selection, &battle)
                        != super::encounter_preset(encounter) =>
            {
                super::choose_player_preset(
                    &mut selection,
                    &mut battle,
                    super::encounter_preset(encounter),
                );
                reset.generation = reset.generation.saturating_add(1);
                state.prepare_round();
            }
            Action::PlayerRecipe(recipe)
                if !state.started
                    && battle.control == ArenaControl::Player
                    && matches!(selection.map, ArenaMap::Fort | ArenaMap::Duel)
                    && super::player_preset(*selection, &battle) != recipe =>
            {
                super::choose_player_preset(&mut selection, &mut battle, recipe);
                reset.generation = reset.generation.saturating_add(1);
                state.prepare_round();
            }
            Action::Start if !state.started && terrain_ready => state.begin_play(),
            Action::Resume if state.started && !session.is_finished() && terrain_ready => {
                state.begin_play()
            }
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
                if let Some(recorder) = recorder.as_mut() {
                    recorder.request_quit();
                } else {
                    exit.write(AppExit::Success);
                }
            }
            Action::Change(index, direction) if state.started => {
                if session.is_forest_run() {
                    if direction > 0.0 {
                        if let Some(stat) = upgrade_stat(index) {
                            session.spend_upgrade(stat);
                        }
                    }
                } else {
                    change(&mut tuning, index, direction);
                }
            }
            _ => continue,
        }
        // UI clicks cannot leak into casting when the simulation resumes.
        session.cancel_charges();
        input.human = ActorIntent {
            aim: super::aim(&state),
            glider_look: super::aim(&state),
            ..default()
        };
    }
}

fn apply_map_selection(
    map: ArenaMap,
    state: &mut ViewState,
    reset: &mut ArenaReset,
    selection: &mut ArenaSelection,
    battle: &mut ArenaBattleSetup,
) {
    let previous = (!matches!(
        selection.map,
        ArenaMap::SevenRegions | ArenaMap::ForestMassif | ArenaMap::NorthernArchipelago
    ))
    .then(|| super::player_preset(*selection, battle));
    selection.map = map;
    if matches!(
        map,
        ArenaMap::SevenRegions | ArenaMap::ForestMassif | ArenaMap::NorthernArchipelago
    ) || battle.control == ArenaControl::Spectator
    {
        battle.player_recipe = None;
    } else {
        let preset = previous.unwrap_or(if map == ArenaMap::Duel {
            BattlePreset::Shadow
        } else {
            BattlePreset::Dragon
        });
        super::choose_player_preset(selection, battle, preset);
    }
    reset.generation = reset.generation.saturating_add(1);
    state.prepare_round();
}

fn upgrade_stat(index: usize) -> Option<UpgradeStat> {
    match index {
        0 => Some(UpgradeStat::ShieldSize),
        1 => Some(UpgradeStat::FireballSize),
        2 => Some(UpgradeStat::HighJumpHeight),
        3 => Some(UpgradeStat::ProjectileSpeed),
        5 => Some(UpgradeStat::ShieldCooldown),
        6 => Some(UpgradeStat::FireballCooldown),
        7 => Some(UpgradeStat::HighJumpCooldown),
        8 => Some(UpgradeStat::FireballDamage),
        9 => Some(UpgradeStat::FireballKnockback),
        10 => Some(UpgradeStat::ShieldProjectileSpeed),
        11 => Some(UpgradeStat::WalkingSpeed),
        _ => None,
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
        2 => t.high_jump_height = (t.high_jump_height + direction * 0.5).clamp(2.0, 8.0),
        3 => t.projectile_speed = (t.projectile_speed + direction * 2.0).clamp(8.0, 64.0),
        4 => t.projectile_gravity = (t.projectile_gravity + direction * 2.0).clamp(2.0, 40.0),
        5 => t.shield_cooldown = (t.shield_cooldown + direction * 0.5).clamp(0.5, 20.0),
        6 => t.fireball_cooldown = (t.fireball_cooldown + direction * 0.25).clamp(0.25, 10.0),
        7 => t.high_jump_cooldown = (t.high_jump_cooldown + direction * 0.5).clamp(0.5, 20.0),
        8 => t.fireball_damage = (t.fireball_damage + direction * 5.0).clamp(5.0, 100.0),
        9 => t.fireball_knockback = (t.fireball_knockback + direction).clamp(0.0, 25.0),
        10 => {
            let steps = (t.bot.acquisition_seconds * 20.0).round() + direction;
            t.bot.acquisition_seconds = steps.clamp(0.0, 10.0) * 0.05;
        }
        11 => t.bot.escape.enabled = direction > 0.0,
        _ => {}
    }
}

pub(super) fn update(
    session: Res<ArenaSession>,
    tuning: Res<ArenaTuning>,
    state: Res<ViewState>,
    selection: Option<Res<ArenaSelection>>,
    battle: Option<Res<ArenaBattleSetup>>,
    mut mode_contents: Query<(&ModeContent, &mut Node), (Without<CombatHud>, Without<Action>)>,
    mut labels: Query<(&Label, &mut Text)>,
    mut cards: Query<(&SpellCard, &mut BorderColor)>,
    mut choices: Query<(&Action, &mut BackgroundColor, &mut Node)>,
    mut panels: Query<
        (
            &mut Node,
            Has<PausePanel>,
            Has<StartPanel>,
            Has<CombatHud>,
            Has<ObserverHud>,
        ),
        (
            Without<ModeContent>,
            Without<Action>,
            Or<(
                With<PausePanel>,
                With<StartPanel>,
                With<CombatHud>,
                With<ObserverHud>,
            )>,
        ),
    >,
    render: Option<Res<hex_core::arena::ArenaRenderStatus>>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    let pending = render.map_or(0, |status| status.pending_chunks);
    let tuning = session.player_tuning(&tuning);
    let run = session.progress();
    let expedition = session.expedition_progress();
    let selection = selection.as_deref().copied().unwrap_or_default();
    let battle = battle.as_deref().cloned().unwrap_or_default();
    let observing = spectator::active(&session);
    for (content, mut node) in &mut mode_contents {
        set_display(
            &mut node,
            if content.0 == battle.control && !selection.map.capabilities().expedition_player {
                Display::Flex
            } else {
                Display::None
            },
        );
    }
    for (action, mut color, mut node) in &mut choices {
        if let Action::Change(index, direction) = action {
            if run.is_some() {
                let stat = upgrade_stat(*index);
                set_display(
                    &mut node,
                    if *direction > 0.0 && stat.is_some() {
                        Display::Flex
                    } else {
                        Display::None
                    },
                );
                color.set_if_neq(BackgroundColor(
                    if stat.is_some_and(|stat| session.can_upgrade(stat)) {
                        Color::srgb(0.16, 0.40, 0.43)
                    } else {
                        Color::srgb(0.12, 0.15, 0.18)
                    },
                ));
            } else {
                set_display(&mut node, Display::Flex);
                color.set_if_neq(BackgroundColor(Color::srgb(0.14, 0.21, 0.26)));
            }
            continue;
        }
        if matches!(action, Action::Control(ArenaControl::Spectator)) {
            set_display(
                &mut node,
                if selection.map.capabilities().expedition_player {
                    Display::None
                } else {
                    Display::Flex
                },
            );
        }
        if matches!(action, Action::Start) {
            color.set_if_neq(BackgroundColor(if pending > 0 {
                Color::srgb(0.12, 0.15, 0.18)
            } else {
                Color::srgb(0.16, 0.37, 0.41)
            }));
            continue;
        }
        if matches!(action, Action::Resume) {
            color.set_if_neq(BackgroundColor(if session.is_finished() || pending > 0 {
                Color::srgb(0.12, 0.15, 0.18)
            } else {
                Color::srgb(0.16, 0.37, 0.41)
            }));
            continue;
        }
        let selected = match action {
            Action::Control(control) => *control == battle.control,
            Action::Map(map) => *map == selection.map,
            Action::Encounter(encounter) => {
                matches!(selection.map, ArenaMap::Fort | ArenaMap::Duel)
                    && super::player_preset(selection, &battle)
                        == super::encounter_preset(*encounter)
            }
            Action::PlayerRecipe(recipe) => {
                matches!(selection.map, ArenaMap::Fort | ArenaMap::Duel)
                    && super::player_preset(selection, &battle) == *recipe
            }
            _ => continue,
        };
        color.set_if_neq(BackgroundColor(if selected {
            Color::srgb(0.16, 0.40, 0.43)
        } else {
            Color::srgb(0.11, 0.16, 0.20)
        }));
    }
    let actor = session
        .human_actor_id()
        .and_then(|id| session.actors.iter().find(|actor| actor.id == id));
    let forest_knocked_out =
        run.is_some() && session.is_finished() && actor.is_some_and(|actor| actor.hp <= 0.0);
    // External landscape cameras never grant player observations. Their spell
    // readiness is intentionally inactive, so keep that combat strip out of the
    // composition evidence instead of presenting it as unavailable gameplay.
    let composition_capture =
        state.capture.is_some() && super::northern::fixture_view(&state.capture_view);
    for (mut node, pause, start, combat, observer) in &mut panels {
        set_display(
            &mut node,
            if (pause && state.paused && state.started)
                || (start && !state.started)
                || (combat && state.started && !state.paused && !observing && !composition_capture)
                || (observer && state.started && observing)
            {
                Display::Flex
            } else {
                Display::None
            },
        );
    }
    let size_name = |index| {
        ["Compact", "Standard", "Large"]
            .get(index)
            .copied()
            .unwrap_or("Standard")
    };
    for (label, mut text) in &mut labels {
        let next = match label {
            Label::WindowMode => if windows.iter().next().is_some_and(|w| w.mode != WindowMode::Windowed) {"Windowed".into()} else {"Fullscreen".into()},
            Label::MenuTitle if forest_knocked_out => "RUN ENDED / KNOCKED OUT".into(),
            Label::MenuTitle if session.completed_run() => "VICTORY / EXPLORE THE MAP".into(),
            Label::MenuTitle if run.is_some() && !session.is_finished() => "PAUSED / LEVEL UPGRADES".into(),
            Label::MenuTitle if session.is_finished() => {
                if observing { "BATTLE COMPLETE".into() } else {
                    match session.outcome {
                        Some(ArenaOutcome::Winner(0)) => "YOU WIN / COMBAT MENU",
                        Some(ArenaOutcome::Winner(_)) => "DEFEATED / COMBAT MENU",
                        _ => "ROUND OVER / COMBAT MENU",
                    }.into()
                }
            },
            Label::MenuTitle => "PAUSED / COMBAT MENU".into(),
            Label::MenuHelp if forest_knocked_out => "Restart begins a fresh run. Your discoveries remain available on the Map until then.".into(),
            Label::MenuHelp if pending > 0 => format!("Preparing terrain: {pending} chunks remaining"),
            Label::MenuHelp if run.is_some() => run.map_or_else(String::new, |p| format!("Level {}  /  XP {} of {}  /  {} upgrade points", p.level, p.xp, p.xp_to_next, p.available_upgrades)),
            Label::MenuHelp if session.is_finished() => "Mouse is free. Reset Arena returns to the start screen.".into(),
            Label::MenuHelp => "Mouse is free. ESC / TAB resumes. Sizes are independent.".into(),
            Label::Resume if forest_knocked_out => "RESTART TO PLAY AGAIN".into(),
            Label::Resume => if session.is_finished() { "ROUND COMPLETE" } else { "RESUME" }.into(),
            Label::Team(slot) => format!("TEAM {}  /  {}", slot + 1, spectator::preset_for(&battle, *slot).map_or("Custom", BattlePreset::label)),
            Label::ObserverTeams => session.battle_summary().map_or_else(String::new, |summary| spectator::team_status(&summary)),
            Label::ObserverStatus => session.battle_summary().map_or_else(String::new, |summary| spectator::battle_status(&summary, state.observer.mode, state.paused)),
            Label::Help if state.northern_preparation.status().is_some() => state.northern_preparation.status().unwrap_or_default().into(),
            Label::Help if state.forest_preparation.status().is_some() => state.forest_preparation.status().unwrap_or_default().into(),
            Label::Help if pending > 0 => format!("Preparing terrain: {pending} chunks remaining. Start unlocks when ready."),
            Label::Help if battle.control == ArenaControl::Spectator => "WASD pan / move / Q and E down and up / Shift fast
Mouse look / Wheel orbit zoom / C orbit or free camera
Camera movement never controls a creature.".into(),
            Label::Help => format!("WASD move / mouse look / Space jump / E High Jump
Hold LMB for Fireball or RMB for Shield; release to cast.
G opens or folds your glider; diving gains speed, climbing loses it.
Casting folds the glider. High Jump keeps your charge. Movement speed is {} units/s.", if expedition.is_some() { "4.725" } else { "4.5" }),
            Label::Selection if battle.control == ArenaControl::Spectator => format!("{} / Seed {} / Two independent teams
Seven Regions is available in Play mode.", super::map_name(selection.map), battle.seed),
            Label::Selection if expedition.is_some() => "Forest Expedition: 107 Goblins, 2 Shamans and the Troll.\nThree Dragons and a Shadow guard the mountains; 3 Golems and 10 Wisps inhabit the lowlands.\nStart on the bridge. Hidden fountains are your only healing.".into(),
            Label::Selection => match selection.map {
                ArenaMap::Duel | ArenaMap::Fort => format!("{}: {}. Restart keeps this enemy party.", super::map_name(selection.map), super::player_preset(selection, &battle).label()),
                ArenaMap::NorthernArchipelago => "Northern Archipelago: an open exploration map. F toggles free flight; Shift accelerates; Space/Ctrl rise/descend. No encounters or victory objective.".into(),
                ArenaMap::ForestMassif => "Forest Massif: 20 Goblins + 2 Shamans in the forest.\nThree Dragons guard the massif beyond the central bridge.".into(),
                ArenaMap::SevenRegions => "Seven Regions: Dragon, Shaman party and Goblins.\nThis map has three fixed enemy parties.".into(),
            },
            Label::Encounter if expedition.is_some() => expedition.as_ref().zip(run).map_or_else(String::new, |(e, p)| format!("FOREST {}/{}  /  DRAGONS {}/3  /  ALL {}/{}\nLEVEL {}  /  XP {} of {}  /  {} upgrade points", e.forest_defeated, e.forest_total, e.dragons_defeated, e.enemies_defeated, e.enemies_total, p.level, p.xp, p.xp_to_next, p.available_upgrades)),
            Label::Encounter if run.is_some() => run.map_or_else(String::new, |p| format!("FOREST {}/22  /  DRAGONS {}/3\nLEVEL {}  /  XP {} of {}  /  {} upgrade points", p.forest_defeated, p.dragons_defeated, p.level, p.xp, p.xp_to_next, p.available_upgrades)),
            Label::Encounter => {
                let summary = session.encounter_summary();
                if selection.map == ArenaMap::Duel {
                    if super::player_preset(selection, &battle) == BattlePreset::Shadow {
                        "DUEL  /  SHADOW CHALLENGE".into()
                    } else {
                        format!("DUEL  /  {}", super::player_preset(selection, &battle).label().to_uppercase())
                    }
                } else {
                    format!("{}  /  {} / {} parties cleared", super::map_name(selection.map), summary.defeated_parties, session.parties().len())
                }
            },
            Label::Rewards => expedition.as_ref().map_or(String::new(), |e| format!(
                "Troll: +25 base damage / {}\nDragons {}/3: explosions / {}\nWisps {}/10: +15 Fireball speed and aim guide / {}\nGolems {}/3: +20 Shield speed, +2 × +2 dimensions / {}",
                milestone_status(e, ExpeditionReward::TrollDamage), e.dragons_defeated,
                milestone_status(e, ExpeditionReward::DragonExplosions), e.wisps_defeated,
                milestone_status(e, ExpeditionReward::WispBallistics), e.golems_defeated,
                milestone_status(e, ExpeditionReward::GolemShield))),
            Label::Health if expedition.is_some() => actor.map_or_else(String::new, |a| format!("{:.0} / {:.0} HP", a.hp, a.max_hp)),
            Label::Health => format!("{:03.0} HP", actor.map_or(100.0, |a| a.hp)),
            Label::Status if forest_knocked_out => "RUN ENDED / YOU WERE KNOCKED OUT\nShift+R to restart from level 1.".into(),
            Label::Status if session.completed_run() => "VICTORY - THE MAP IS CLEAR\nKeep exploring and casting. Shift+R restarts your run.".into(),
            Label::Status => {
                if state.capture.is_some() && super::encounter::stress_view(&state.capture_view) {
                    "SYNTHETIC PERFORMANCE FIXTURE\nExtra HP / scripted party visits".into()
                } else if let Some(outcome) = &session.outcome {
                    format!(
                        "{}\nShift+R to restart",
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
                let feedback = session.combat_feedback(&tuning);
                let Some(spell) = feedback.spells.get(*index) else { continue; };
                match spell.state {
                    hex_arena::SpellAvailabilityState::Ready => "✓ READY".into(),
                    hex_arena::SpellAvailabilityState::CoolingDown => format!("{:.1}s", (spell.cooldown_remaining * 10.0).ceil() / 10.0),
                    hex_arena::SpellAvailabilityState::Charging => format!("CHARGE {:.0}%", spell.charge_fraction * 100.0),
                    hex_arena::SpellAvailabilityState::Unavailable => "UNAVAILABLE".into(),
                }
            }
            Label::MenuRules if expedition.is_some() => expedition.as_ref().map_or_else(String::new, |e| format!("Shadow: +25 maximum HP, no healing / {}\nHidden fountains heal up to 40 HP once. Enemies give XP, never HP.\nEach level grants one + upgrade; cooldown + makes it faster.", milestone_status(e, ExpeditionReward::ShadowVitality))),
            Label::MenuRules if run.is_some() => "Each level grants one + upgrade; cooldown + makes it faster.\nClear forest: +25 damage. Slay 3 Dragons: explosions. Reset clears upgrades.".into(),
            Label::MenuRules => "Splash passes through walls. Fireballs can hurt their caster.\nShield walls remain until destroyed; restart restores all terrain.".into(),
            Label::Parameter(1) if expedition.is_some() && run.is_some_and(|p| !p.explosions_unlocked) => "Contact only / collect the Dragon orb".into(),
            Label::Parameter(1) if run.is_some_and(|p| !p.explosions_unlocked) => "Fireball impact only - slay 3 Dragons".into(),
            Label::Parameter(4) if run.is_some() => "Gravity (fixed)           12 units/s^2".into(),
            Label::Parameter(index) if run.is_some() => upgrade_stat(*index)
                .and_then(|stat| session.upgrade_preview(stat).map(|preview| upgrade_description(*index, preview)))
                .unwrap_or_default(),
            Label::Parameter(index) => match index {
                0 => format!(
                    "Shield size                 {}",
                    size_name(tuning.shield_size)
                ),
                1 => format!(
                    "Fireball size                {}",
                    size_name(tuning.fireball_size)
                ),
                2 => format!("High Jump height          {:.1} units", tuning.high_jump_height),
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
                7 => format!("High Jump cooldown     {:.1}s", tuning.high_jump_cooldown),
                8 => format!("Fireball damage           {:.0} HP", tuning.fireball_damage),
                9 => format!("Fireball knockback       {:.0}", tuning.fireball_knockback),
                10 => if tuning.bot.acquisition_seconds <= 0.0 {
                    "Shadow reaction             Off".into()
                } else {
                    format!("Shadow reaction             {:.0} ms", tuning.bot.acquisition_seconds * 1000.0)
                },
                11 => format!("Shadow escape                 {}", if tuning.bot.escape.enabled { "On" } else { "Off" }),
                _ => String::new(),
            },
        };
        if text.0 != next {
            text.0 = next;
        }
    }
    let feedback = session.combat_feedback(&tuning);
    for (card, mut border) in &mut cards {
        let Some(spell) = feedback.spells.get(card.0) else {
            continue;
        };
        border.set_if_neq(BorderColor::all(match spell.state {
            hex_arena::SpellAvailabilityState::Ready => Color::srgb(0.4, 0.9, 0.8),
            hex_arena::SpellAvailabilityState::Charging => Color::srgb(1.0, 0.75, 0.32),
            _ => MUTED,
        }));
    }
}

fn upgrade_description(index: usize, preview: hex_arena::UpgradePreview) -> String {
    let name = match index {
        0 => "Shield dimensions",
        1 => "Explosion radius",
        2 => "High Jump height",
        3 => "Fireball launch speed",
        5 => "Shield cooldown",
        6 => "Fireball cooldown",
        7 => "High Jump cooldown",
        8 => "Fireball damage",
        9 => "Fireball knockback",
        10 => "Shield launch speed",
        11 => "Walking speed",
        _ => "Upgrade",
    };
    let value = |value: hex_arena::UpgradeValue| match value {
        hex_arena::UpgradeValue::Scalar(value) => format!("{value:.2}"),
        hex_arena::UpgradeValue::Dimensions(width, height) => format!("{width} × {height}"),
    };
    let next = preview.after.map_or_else(|| "MAX".into(), value);
    format!(
        "{name}  {} → {next}  [{}/{}]",
        value(preview.before),
        preview.rank,
        preview.max_ranks
    )
}
